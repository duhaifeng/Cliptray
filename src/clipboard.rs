//! 系统剪切板读写。后端用 trait 隔离，监控逻辑因此可以不碰真实剪切板做测试。

use std::path::PathBuf;

use anyhow::Result;

use crate::model::{ItemKind, digest_parts};
use crate::store::image_digest;

#[derive(Clone, Debug, PartialEq)]
pub enum ClipContent {
    Empty,
    Text {
        text: String,
        html: Option<String>,
    },
    Image {
        rgba: Vec<u8>,
        width: u32,
        height: u32,
    },
    Files(Vec<PathBuf>),
}

impl ClipContent {
    /// 内容摘要。必须与 `Item` 里存的摘要同源，监控才能识别“自己刚写入的内容”。
    pub fn digest(&self) -> Option<String> {
        match self {
            ClipContent::Empty => None,
            ClipContent::Text { text, html } => {
                let kind = if html.is_some() {
                    ItemKind::RichText
                } else {
                    ItemKind::Text
                };
                Some(digest_parts(
                    kind,
                    &[text.as_bytes(), html.as_deref().unwrap_or("").as_bytes()],
                ))
            }
            ClipContent::Image {
                rgba,
                width,
                height,
            } => Some(image_digest(rgba, *width, *height)),
            ClipContent::Files(paths) => {
                let strs: Vec<String> = paths
                    .iter()
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect();
                let joined = strs.join("\n");
                let kind = match (paths.len(), paths.first().is_some_and(|p| p.is_dir())) {
                    (1, false) => ItemKind::File,
                    (1, true) => ItemKind::Folder,
                    _ => ItemKind::Files,
                };
                Some(digest_parts(kind, &[joined.as_bytes()]))
            }
        }
    }
}

pub trait ClipboardBackend {
    fn read(&mut self) -> Result<ClipContent>;
    fn write(&mut self, content: &ClipContent) -> Result<()>;
}

pub struct SystemClipboard {
    inner: arboard::Clipboard,
}

impl SystemClipboard {
    pub fn new() -> Result<Self> {
        Ok(SystemClipboard {
            inner: arboard::Clipboard::new()?,
        })
    }
}

fn available<T>(r: Result<T, arboard::Error>) -> Result<Option<T>> {
    match r {
        Ok(v) => Ok(Some(v)),
        // 该格式当前没有内容是常态，不是错误。
        Err(arboard::Error::ContentNotAvailable) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

impl ClipboardBackend for SystemClipboard {
    fn read(&mut self) -> Result<ClipContent> {
        // 文件 > 图片 > 文本：文件管理器复制文件时常同时带路径文本，应按文件记录。
        if let Some(files) = available(self.inner.get().file_list())?
            && !files.is_empty()
        {
            return Ok(ClipContent::Files(files));
        }
        if let Some(img) = available(self.inner.get().image())? {
            return Ok(ClipContent::Image {
                width: img.width as u32,
                height: img.height as u32,
                rgba: img.bytes.into_owned(),
            });
        }
        let html = available(self.inner.get().html())?.filter(|h| !h.trim().is_empty());
        match available(self.inner.get().text())? {
            Some(text) => Ok(ClipContent::Text { text, html }),
            None => Ok(ClipContent::Empty),
        }
    }

    fn write(&mut self, content: &ClipContent) -> Result<()> {
        match content {
            ClipContent::Empty => self.inner.clear()?,
            ClipContent::Text { text, html: None } => self.inner.set().text(text.as_str())?,
            ClipContent::Text {
                text,
                html: Some(html),
            } => self.inner.set().html(html.as_str(), Some(text.as_str()))?,
            ClipContent::Image {
                rgba,
                width,
                height,
            } => self.inner.set().image(arboard::ImageData {
                width: *width as usize,
                height: *height as usize,
                bytes: rgba.as_slice().into(),
            })?,
            ClipContent::Files(paths) => self.inner.set().file_list(paths)?,
        }
        Ok(())
    }
}

#[cfg(test)]
pub mod testing {
    use super::*;

    /// 内存剪切板：`read` 返回最近一次设置的内容。
    #[derive(Default)]
    pub struct MemoryClipboard {
        pub content: Option<ClipContent>,
        pub fail_reads: bool,
        pub writes: Vec<ClipContent>,
    }

    impl ClipboardBackend for MemoryClipboard {
        fn read(&mut self) -> Result<ClipContent> {
            if self.fail_reads {
                anyhow::bail!("occupied");
            }
            Ok(self.content.clone().unwrap_or(ClipContent::Empty))
        }

        fn write(&mut self, content: &ClipContent) -> Result<()> {
            self.writes.push(content.clone());
            self.content = Some(content.clone());
            Ok(())
        }
    }
}
