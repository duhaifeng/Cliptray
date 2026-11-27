//! 剪切板监控：轮询 -> 去重 / 过滤 -> 入库。以及把历史条目写回剪切板。
//!
//! iCopy 用 `NSPasteboard.changeCount` 的定时轮询探测变化；Linux 没有等价计数，
//! 这里改为比对内容摘要。

use std::path::PathBuf;

use anyhow::Result;

use crate::clipboard::{ClipContent, ClipboardBackend};
use crate::config::Config;
use crate::model::{Item, ItemKind, Transform, strip_style_marks, trim_edge_whitespace};
use crate::store::{InsertOutcome, Store};

/// 当前前台应用，用于黑名单和来源记录。
pub trait ActiveApp {
    fn current(&mut self) -> Option<String>;
}

impl ActiveApp for Box<dyn ActiveApp> {
    fn current(&mut self) -> Option<String> {
        (**self).current()
    }
}

pub struct NoActiveApp;

impl ActiveApp for NoActiveApp {
    fn current(&mut self) -> Option<String> {
        None
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum SkipReason {
    Blank,
    TooLarge,
    Blacklisted,
    /// 内容是本程序刚写入的，不再作为新历史记录。
    SelfWritten,
}

#[derive(Debug, PartialEq)]
pub enum Captured {
    /// 没有变化或剪切板为空。
    Nothing,
    Skipped(SkipReason),
    Stored(InsertOutcome),
}

#[derive(Default)]
pub struct Monitor {
    last_digest: Option<String>,
    self_written: Option<String>,
}

impl Monitor {
    pub fn new() -> Self {
        Self::default()
    }

    /// 启动时把当前剪切板内容当作已处理，避免重启守护进程就把旧内容当新内容重复入库。
    pub fn prime(&mut self, backend: &mut impl ClipboardBackend) {
        if let Ok(c) = backend.read() {
            self.last_digest = c.digest();
        }
    }

    /// 记录“这是我们自己写入的内容”，下一次轮询看到它时跳过。
    pub fn note_self_write(&mut self, content: &ClipContent) {
        self.self_written = content.digest();
    }

    pub fn poll(
        &mut self,
        backend: &mut impl ClipboardBackend,
        app: &mut impl ActiveApp,
        store: &mut Store,
        cfg: &Config,
        now: f64,
    ) -> Result<Captured> {
        let content = backend.read()?;
        let Some(digest) = content.digest() else {
            // 剪切板被清空：之后再次复制同样的内容应当被视作新的复制。
            self.last_digest = None;
            return Ok(Captured::Nothing);
        };
        if self.last_digest.as_deref() == Some(digest.as_str()) {
            return Ok(Captured::Nothing);
        }
        // 先记下摘要：被跳过的内容也不应在每个轮询周期被重新评估。
        self.last_digest = Some(digest.clone());

        if self.self_written.as_deref() == Some(digest.as_str()) {
            return Ok(Captured::Skipped(SkipReason::SelfWritten));
        }
        if let ClipContent::Text { text, .. } = &content {
            if text.trim().is_empty() {
                return Ok(Captured::Skipped(SkipReason::Blank));
            }
            if text.len() > cfg.max_text_bytes {
                return Ok(Captured::Skipped(SkipReason::TooLarge));
            }
        }
        let source = app.current();
        if cfg.is_blacklisted(source.as_deref()) {
            return Ok(Captured::Skipped(SkipReason::Blacklisted));
        }

        let item = match content {
            ClipContent::Empty => return Ok(Captured::Nothing),
            ClipContent::Text { text, html } => Item::text(text, html, source, now),
            ClipContent::Files(paths) => {
                let is_dir = paths.len() == 1 && paths[0].is_dir();
                let strs = paths
                    .iter()
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect();
                Item::files(strs, is_dir, source, now)
            }
            ClipContent::Image {
                rgba,
                width,
                height,
            } => {
                let (file, digest) = store.save_png(&rgba, width, height)?;
                Item::image(file, (width, height), digest, source, now)
            }
        };
        Ok(Captured::Stored(store.insert(&item)?))
    }
}

/// 把历史条目还原成可写入剪切板的内容。`plain` 为真或有改写时丢弃富文本格式；
/// 两者都只作用于文本条目。
pub fn item_to_content(
    item: &Item,
    store: &Store,
    plain: bool,
    transform: Transform,
) -> Result<ClipContent> {
    Ok(match item.kind {
        ItemKind::Text | ItemKind::RichText => {
            let raw = item.text.as_deref().unwrap_or_default();
            let text = transform.apply(raw);
            let keep_html = !plain && transform == Transform::None;
            ClipContent::Text {
                text,
                html: if keep_html { item.html.clone() } else { None },
            }
        }
        ItemKind::Image => {
            let name = item
                .image_file
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("图片条目缺少文件名"))?;
            let (rgba, width, height) = store.read_image(name)?;
            ClipContent::Image {
                rgba,
                width,
                height,
            }
        }
        ItemKind::File | ItemKind::Folder | ItemKind::Files => {
            ClipContent::Files(item.files.iter().map(PathBuf::from).collect())
        }
    })
}

/// 按设置整理即将写回剪切板的文本。历史里仍是原文。
/// `trim_whitespace` 去掉首尾空行和全半角空格；`strip_styles` 去掉终端转义与 HTML 样式，并丢弃富文本格式。
pub fn apply_outgoing_filters(cfg: &Config, content: ClipContent) -> ClipContent {
    match content {
        ClipContent::Text { mut text, mut html } => {
            if cfg.strip_styles {
                text = strip_style_marks(&text);
                html = None;
            }
            if cfg.trim_whitespace {
                text = trim_edge_whitespace(&text);
            }
            ClipContent::Text { text, html }
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clipboard::testing::MemoryClipboard;

    struct FixedApp(Option<&'static str>);
    impl ActiveApp for FixedApp {
        fn current(&mut self) -> Option<String> {
            self.0.map(str::to_owned)
        }
    }

    fn setup() -> (tempfile::TempDir, Store, MemoryClipboard, Monitor, Config) {
        let d = tempfile::tempdir().unwrap();
        let s = Store::open(d.path()).unwrap();
        (
            d,
            s,
            MemoryClipboard::default(),
            Monitor::new(),
            Config::default(),
        )
    }

    fn text(s: &str) -> ClipContent {
        ClipContent::Text {
            text: s.into(),
            html: None,
        }
    }

    #[test]
    fn new_text_is_stored_once_with_source() {
        let (_d, mut s, mut cb, mut m, cfg) = setup();
        cb.content = Some(text("hello"));
        let r = m
            .poll(&mut cb, &mut FixedApp(Some("firefox")), &mut s, &cfg, 1.0)
            .unwrap();
        assert!(matches!(r, Captured::Stored(InsertOutcome::Inserted(_))));
        assert_eq!(
            m.poll(&mut cb, &mut FixedApp(None), &mut s, &cfg, 2.0)
                .unwrap(),
            Captured::Nothing
        );
        let it = s.latest().unwrap().unwrap();
        assert_eq!(it.source_app.as_deref(), Some("firefox"));
        assert_eq!(s.count().unwrap(), 1);
    }

    #[test]
    fn copying_old_content_again_moves_it_to_top() {
        let (_d, mut s, mut cb, mut m, cfg) = setup();
        for (c, t) in [("a", 1.0), ("b", 2.0), ("a", 3.0)] {
            cb.content = Some(text(c));
            m.poll(&mut cb, &mut NoActiveApp, &mut s, &cfg, t).unwrap();
        }
        assert_eq!(s.count().unwrap(), 2);
        assert_eq!(s.latest().unwrap().unwrap().text.as_deref(), Some("a"));
    }

    #[test]
    fn clearing_clipboard_then_recopying_same_text_counts_again() {
        let (_d, mut s, mut cb, mut m, cfg) = setup();
        cb.content = Some(text("x"));
        m.poll(&mut cb, &mut NoActiveApp, &mut s, &cfg, 1.0)
            .unwrap();
        s.set_favorite(s.latest().unwrap().unwrap().id, false)
            .unwrap();
        cb.content = None;
        m.poll(&mut cb, &mut NoActiveApp, &mut s, &cfg, 2.0)
            .unwrap();
        cb.content = Some(text("x"));
        let r = m
            .poll(&mut cb, &mut NoActiveApp, &mut s, &cfg, 3.0)
            .unwrap();
        assert!(matches!(r, Captured::Stored(InsertOutcome::Bumped(_))));
    }

    #[test]
    fn blank_and_oversized_text_skipped() {
        let (_d, mut s, mut cb, mut m, mut cfg) = setup();
        cfg.max_text_bytes = 5;
        cb.content = Some(text("  \n\t"));
        assert_eq!(
            m.poll(&mut cb, &mut NoActiveApp, &mut s, &cfg, 1.0)
                .unwrap(),
            Captured::Skipped(SkipReason::Blank)
        );
        cb.content = Some(text("123456"));
        assert_eq!(
            m.poll(&mut cb, &mut NoActiveApp, &mut s, &cfg, 2.0)
                .unwrap(),
            Captured::Skipped(SkipReason::TooLarge)
        );
        assert_eq!(s.count().unwrap(), 0);
    }

    #[test]
    fn blacklisted_source_is_not_recorded_and_not_reevaluated() {
        let (_d, mut s, mut cb, mut m, mut cfg) = setup();
        cfg.blacklist = vec!["KeePassXC".into()];
        cb.content = Some(text("secret"));
        let r = m
            .poll(&mut cb, &mut FixedApp(Some("keepassxc")), &mut s, &cfg, 1.0)
            .unwrap();
        assert_eq!(r, Captured::Skipped(SkipReason::Blacklisted));
        // 之后即便前台换成普通应用，同一份内容也不能被补记。
        assert_eq!(
            m.poll(&mut cb, &mut FixedApp(Some("firefox")), &mut s, &cfg, 2.0)
                .unwrap(),
            Captured::Nothing
        );
        assert_eq!(s.count().unwrap(), 0);
    }

    #[test]
    fn own_write_is_not_recorded() {
        let (_d, mut s, mut cb, mut m, cfg) = setup();
        let c = text("pasted by us");
        cb.write(&c).unwrap();
        m.note_self_write(&c);
        assert_eq!(
            m.poll(&mut cb, &mut NoActiveApp, &mut s, &cfg, 1.0)
                .unwrap(),
            Captured::Skipped(SkipReason::SelfWritten)
        );
        assert_eq!(s.count().unwrap(), 0);
    }

    #[test]
    fn prime_ignores_preexisting_clipboard() {
        let (_d, mut s, mut cb, mut m, cfg) = setup();
        cb.content = Some(text("before start"));
        m.prime(&mut cb);
        assert_eq!(
            m.poll(&mut cb, &mut NoActiveApp, &mut s, &cfg, 1.0)
                .unwrap(),
            Captured::Nothing
        );
        assert_eq!(s.count().unwrap(), 0);
    }

    #[test]
    fn read_error_propagates_without_losing_state() {
        let (_d, mut s, mut cb, mut m, cfg) = setup();
        cb.fail_reads = true;
        assert!(
            m.poll(&mut cb, &mut NoActiveApp, &mut s, &cfg, 1.0)
                .is_err()
        );
        cb.fail_reads = false;
        cb.content = Some(text("recovered"));
        assert!(matches!(
            m.poll(&mut cb, &mut NoActiveApp, &mut s, &cfg, 2.0)
                .unwrap(),
            Captured::Stored(_)
        ));
    }

    #[test]
    fn image_is_saved_to_disk_and_restorable() {
        let (_d, mut s, mut cb, mut m, cfg) = setup();
        let px = vec![7u8; 8 * 8 * 4];
        cb.content = Some(ClipContent::Image {
            rgba: px.clone(),
            width: 8,
            height: 8,
        });
        assert!(matches!(
            m.poll(&mut cb, &mut NoActiveApp, &mut s, &cfg, 1.0)
                .unwrap(),
            Captured::Stored(_)
        ));
        let it = s.latest().unwrap().unwrap();
        assert_eq!(it.kind, ItemKind::Image);
        match item_to_content(&it, &s, false, Transform::None).unwrap() {
            ClipContent::Image {
                rgba,
                width,
                height,
            } => assert_eq!((rgba, width, height), (px, 8, 8)),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn outgoing_filters_trim_edges_and_drop_styles() {
        let rich = ClipContent::Text {
            text: "\n\u{1b}[32m  <b>hi</b>\u{3000}\n".into(),
            html: Some("<b>hi</b>".into()),
        };
        let cfg = Config {
            trim_whitespace: true,
            strip_styles: true,
            ..Config::default()
        };
        assert_eq!(
            apply_outgoing_filters(&cfg, rich),
            ClipContent::Text {
                text: "hi".into(),
                html: None,
            }
        );
        let keep = ClipContent::Text {
            text: "\n  x\u{3000}".into(),
            html: Some("<i>x</i>".into()),
        };
        assert_eq!(
            apply_outgoing_filters(&Config::default(), keep.clone()),
            keep
        );
    }

    #[test]
    fn plain_and_transforms_only_rewrite_text() {
        let (_d, s, _cb, _m, _cfg) = setup();
        let rich = Item::text("a\nb".into(), Some("<b>a</b>".into()), None, 0.0);
        assert_eq!(
            item_to_content(&rich, &s, true, Transform::None).unwrap(),
            text("a\nb")
        );
        assert!(matches!(
            item_to_content(&rich, &s, false, Transform::None).unwrap(),
            ClipContent::Text { html: Some(_), .. }
        ));
        assert_eq!(
            item_to_content(&rich, &s, false, Transform::StripNewlines).unwrap(),
            text("ab")
        );
        let files = Item::files(vec!["/a/x".into(), "/b/y".into()], false, None, 0.0);
        for plain in [false, true] {
            let c = item_to_content(&files, &s, plain, Transform::StripWhitespace).unwrap();
            assert!(matches!(c, ClipContent::Files(_)), "文件条目始终按文件写回");
        }
    }

    #[test]
    fn content_digest_matches_item_digest() {
        let it = Item::text("same".into(), Some("<i>same</i>".into()), None, 0.0);
        let c = ClipContent::Text {
            text: "same".into(),
            html: Some("<i>same</i>".into()),
        };
        assert_eq!(c.digest().unwrap(), it.digest);
        let f = Item::files(vec!["/nonexistent/a".into()], false, None, 0.0);
        assert_eq!(
            ClipContent::Files(vec!["/nonexistent/a".into()])
                .digest()
                .unwrap(),
            f.digest
        );
    }
}
