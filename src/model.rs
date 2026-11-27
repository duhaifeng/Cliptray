//! 剪切板条目的领域模型。
//!
//! 类型分组仿照 iCopy 的查询形态：带 `text like` 的查询匹配 2 种类型（这里是纯文本与富文本），
//! 带 `filepath like` 的匹配 3 种（文件 / 文件夹 / 多文件）。`pastetype` 的具体数值是本项目自定的，
//! 没有从 iCopy 读到。

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ItemKind {
    Text = 0,
    RichText = 1,
    Image = 2,
    File = 3,
    Folder = 4,
    Files = 5,
}

impl ItemKind {
    pub fn as_i64(self) -> i64 {
        self as i64
    }

    pub fn from_i64(v: i64) -> Option<Self> {
        Some(match v {
            0 => Self::Text,
            1 => Self::RichText,
            2 => Self::Image,
            3 => Self::File,
            4 => Self::Folder,
            5 => Self::Files,
            _ => return None,
        })
    }

    pub fn is_file_like(self) -> bool {
        matches!(self, Self::File | Self::Folder | Self::Files)
    }
}

/// 面板顶部的分类标签。`Favorite` 不是类型，而是收藏标记。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Tag {
    #[default]
    All,
    Text,
    Image,
    File,
    Favorite,
}

impl Tag {
    pub const ORDER: [Tag; 5] = [Tag::All, Tag::Text, Tag::Image, Tag::File, Tag::Favorite];

    pub fn kinds(self) -> Option<&'static [ItemKind]> {
        match self {
            Tag::All | Tag::Favorite => None,
            Tag::Text => Some(&[ItemKind::Text, ItemKind::RichText]),
            Tag::Image => Some(&[ItemKind::Image]),
            Tag::File => Some(&[ItemKind::File, ItemKind::Folder, ItemKind::Files]),
        }
    }

    pub fn next(self) -> Tag {
        let i = Self::ORDER.iter().position(|t| *t == self).unwrap_or(0);
        Self::ORDER[(i + 1) % Self::ORDER.len()]
    }

    pub fn prev(self) -> Tag {
        let i = Self::ORDER.iter().position(|t| *t == self).unwrap_or(0);
        Self::ORDER[(i + Self::ORDER.len() - 1) % Self::ORDER.len()]
    }

    pub fn parse(s: &str) -> Option<Tag> {
        Some(match s.to_ascii_lowercase().as_str() {
            "all" => Tag::All,
            "text" => Tag::Text,
            "image" => Tag::Image,
            "file" | "files" => Tag::File,
            "favorite" | "fav" | "star" => Tag::Favorite,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Tag::All => "all",
            Tag::Text => "text",
            Tag::Image => "image",
            Tag::File => "file",
            Tag::Favorite => "favorite",
        }
    }
}

/// 写回剪切板前对文本的改写，对应 iCopy 的“拷贝为 → 去除换行 / 去除换行和空格”。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Transform {
    #[default]
    None,
    StripNewlines,
    StripWhitespace,
}

impl Transform {
    pub fn apply(self, s: &str) -> String {
        match self {
            Transform::None => s.to_owned(),
            Transform::StripNewlines => s.chars().filter(|c| !matches!(c, '\n' | '\r')).collect(),
            Transform::StripWhitespace => s.chars().filter(|c| !c.is_whitespace()).collect(),
        }
    }
}

/// 一条剪切板历史。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Item {
    #[serde(default)]
    pub id: i64,
    pub kind: ItemKind,
    /// 来源应用（X11 下为活动窗口的 WM_CLASS）；无法判断时为 `None`。
    #[serde(default)]
    pub source_app: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub html: Option<String>,
    #[serde(default)]
    pub files: Vec<String>,
    /// 相对数据目录的图片文件名（位于 `tempFiles/`）。
    #[serde(default)]
    pub image_file: Option<String>,
    #[serde(default)]
    pub image_size: Option<(u32, u32)>,
    /// Unix 秒，带小数。
    pub created_at: f64,
    #[serde(default)]
    pub favorite: bool,
    /// 内容摘要，用于去重。
    pub digest: String,
}

impl Item {
    pub fn text(text: String, html: Option<String>, source_app: Option<String>, now: f64) -> Self {
        let kind = if html.is_some() {
            ItemKind::RichText
        } else {
            ItemKind::Text
        };
        let digest = digest_parts(
            kind,
            &[text.as_bytes(), html.as_deref().unwrap_or("").as_bytes()],
        );
        Item {
            id: 0,
            kind,
            source_app,
            text: Some(text),
            html,
            files: Vec::new(),
            image_file: None,
            image_size: None,
            created_at: now,
            favorite: false,
            digest,
        }
    }

    pub fn files(paths: Vec<String>, is_dir: bool, source_app: Option<String>, now: f64) -> Self {
        let kind = match (paths.len(), is_dir) {
            (1, false) => ItemKind::File,
            (1, true) => ItemKind::Folder,
            _ => ItemKind::Files,
        };
        let joined = paths.join("\n");
        let digest = digest_parts(kind, &[joined.as_bytes()]);
        Item {
            id: 0,
            kind,
            source_app,
            text: None,
            html: None,
            files: paths,
            image_file: None,
            image_size: None,
            created_at: now,
            favorite: false,
            digest,
        }
    }

    pub fn image(
        file: String,
        size: (u32, u32),
        pixel_digest: String,
        source_app: Option<String>,
        now: f64,
    ) -> Self {
        Item {
            id: 0,
            kind: ItemKind::Image,
            source_app,
            text: None,
            html: None,
            files: Vec::new(),
            image_file: Some(file),
            image_size: Some(size),
            created_at: now,
            favorite: false,
            digest: pixel_digest,
        }
    }

    /// `t_data.filepath` 列：图片为文件名，文件类为换行拼接的路径，其余为空。
    pub fn filepath_column(&self) -> String {
        match self.kind {
            ItemKind::Image => self.image_file.clone().unwrap_or_default(),
            k if k.is_file_like() => self.files.join("\n"),
            _ => String::new(),
        }
    }

    /// 列表里展示的单行预览：折叠空白，截断到 `max_chars` 个字符。
    pub fn preview_line(&self, max_chars: usize) -> String {
        let raw = match self.kind {
            ItemKind::Image => match self.image_size {
                Some((w, h)) => format!("{w} × {h}"),
                None => String::new(),
            },
            k if k.is_file_like() => self
                .files
                .iter()
                .map(|p| p.rsplit('/').next().unwrap_or(p).to_owned())
                .collect::<Vec<_>>()
                .join(", "),
            _ => self.text.clone().unwrap_or_default(),
        };
        let collapsed = raw.split_whitespace().collect::<Vec<_>>().join(" ");
        truncate_chars(&collapsed, max_chars)
    }
}

pub fn truncate_chars(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_owned();
    }
    let mut out: String = s.chars().take(max_chars.saturating_sub(1)).collect();
    out.push('…');
    out
}

pub fn digest_parts(kind: ItemKind, parts: &[&[u8]]) -> String {
    let mut h = Sha256::new();
    h.update([kind as u8]);
    for p in parts {
        h.update((p.len() as u64).to_le_bytes());
        h.update(p);
    }
    hex(&h.finalize())
}

/// 去掉首尾空白。包括空行、制表符、半角空格、全角空格（U+3000）以及其它 Unicode 空白。
/// 中间的空白原样保留。
pub fn trim_edge_whitespace(s: &str) -> String {
    s.trim().to_owned()
}

/// 去掉终端样式转义（ANSI / OSC）和 HTML 样式标记，留下可读的纯文本。
/// 代码里的 `Vec<String>`、`a < b` 这类比较不会被当成标签。
pub fn strip_style_marks(s: &str) -> String {
    strip_markup(&strip_ansi(s))
}

fn strip_ansi(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\u{1b}' {
            i += 1;
            if i >= chars.len() {
                break;
            }
            match chars[i] {
                '[' => {
                    i += 1;
                    while i < chars.len() && !is_csi_final(chars[i]) {
                        i += 1;
                    }
                    if i < chars.len() {
                        i += 1;
                    }
                }
                ']' => {
                    i += 1;
                    while i < chars.len() {
                        if chars[i] == '\u{7}' {
                            i += 1;
                            break;
                        }
                        if chars[i] == '\u{1b}' && i + 1 < chars.len() && chars[i + 1] == '\\' {
                            i += 2;
                            break;
                        }
                        i += 1;
                    }
                }
                '(' | ')' | '*' | '+' => i += 2,
                _ => i += 1,
            }
        } else if chars[i] == '\u{9b}' {
            i += 1;
            while i < chars.len() && !is_csi_final(chars[i]) {
                i += 1;
            }
            if i < chars.len() {
                i += 1;
            }
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

fn is_csi_final(c: char) -> bool {
    matches!(c, '\u{40}'..='\u{7e}')
}

fn strip_markup(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut extra = std::collections::HashSet::new();
    let mut i = 0;
    let mut stripped_tags = false;
    while i < bytes.len() {
        if bytes[i] != b'<' {
            let ch = s[i..].chars().next().unwrap();
            if !is_style_format(ch) {
                out.push(ch);
            }
            i += ch.len_utf8();
            continue;
        }
        if s[i..].starts_with("<!--") {
            if let Some(end) = s[i + 4..].find("-->") {
                i += 4 + end + 3;
                stripped_tags = true;
                continue;
            }
        }
        if s[i..].starts_with("<!") || s[i..].starts_with("<?") {
            if let Some(end) = s[i..].find('>') {
                i += end + 1;
                stripped_tags = true;
                continue;
            }
        }
        if let Some(tag) = parse_tag(s, i) {
            let lower = tag.name.to_ascii_lowercase();
            let known = is_html_tag(&lower);
            let strip = known || tag.has_attr || extra.contains(&lower);
            if strip {
                if !tag.closing && tag.has_attr {
                    extra.insert(lower.clone());
                }
                if (lower == "script" || lower == "style") && !tag.closing {
                    let close = format!("</{lower}");
                    let rest = s[tag.end..].to_ascii_lowercase();
                    if let Some(rel) = rest.find(&close)
                        && let Some(gt) = s[tag.end + rel..].find('>')
                    {
                        i = tag.end + rel + gt + 1;
                        stripped_tags = true;
                        continue;
                    }
                }
                if is_break_tag(&lower) && (tag.closing || lower == "br" || lower == "hr") {
                    if !out.ends_with('\n') {
                        out.push('\n');
                    }
                }
                i = tag.end;
                stripped_tags = true;
                continue;
            }
        }
        out.push('<');
        i += 1;
    }
    if stripped_tags {
        decode_entities(&out)
            .trim_end_matches(['\n', '\r'])
            .to_owned()
    } else {
        out
    }
}

struct HtmlTag<'a> {
    end: usize,
    name: &'a str,
    closing: bool,
    has_attr: bool,
}

fn parse_tag<'a>(s: &'a str, start: usize) -> Option<HtmlTag<'a>> {
    let bytes = s.as_bytes();
    let mut i = start + 1;
    let mut closing = false;
    if i < bytes.len() && bytes[i] == b'/' {
        closing = true;
        i += 1;
    }
    let name_start = i;
    if i >= bytes.len() || !bytes[i].is_ascii_alphabetic() {
        return None;
    }
    i += 1;
    while i < bytes.len()
        && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'-' || bytes[i] == b':')
    {
        i += 1;
    }
    let name = &s[name_start..i];
    let attr_start = i;
    let mut quote: Option<u8> = None;
    while i < bytes.len() {
        let c = bytes[i];
        if let Some(q) = quote {
            if c == q {
                quote = None;
            }
            i += 1;
            continue;
        }
        if c == b'"' || c == b'\'' {
            quote = Some(c);
            i += 1;
            continue;
        }
        if c == b'>' {
            let raw_attr = &s[attr_start..i];
            return Some(HtmlTag {
                end: i + 1,
                name,
                closing,
                has_attr: raw_attr.contains('='),
            });
        }
        i += 1;
    }
    None
}

fn is_html_tag(name: &str) -> bool {
    matches!(
        name,
        "a" | "abbr"
            | "article"
            | "aside"
            | "b"
            | "bdi"
            | "bdo"
            | "blockquote"
            | "body"
            | "br"
            | "button"
            | "caption"
            | "center"
            | "cite"
            | "code"
            | "col"
            | "colgroup"
            | "dd"
            | "del"
            | "div"
            | "dl"
            | "dt"
            | "em"
            | "figcaption"
            | "figure"
            | "font"
            | "footer"
            | "form"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "head"
            | "header"
            | "hr"
            | "html"
            | "i"
            | "img"
            | "input"
            | "ins"
            | "kbd"
            | "label"
            | "li"
            | "link"
            | "main"
            | "mark"
            | "meta"
            | "nav"
            | "nobr"
            | "ol"
            | "p"
            | "pre"
            | "q"
            | "s"
            | "samp"
            | "script"
            | "section"
            | "small"
            | "span"
            | "strike"
            | "strong"
            | "style"
            | "sub"
            | "sup"
            | "table"
            | "tbody"
            | "td"
            | "tfoot"
            | "th"
            | "thead"
            | "title"
            | "tr"
            | "u"
            | "ul"
            | "var"
            | "wbr"
    )
}

fn is_break_tag(name: &str) -> bool {
    matches!(
        name,
        "br" | "hr"
            | "p"
            | "div"
            | "li"
            | "tr"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "blockquote"
            | "pre"
            | "section"
            | "article"
            | "header"
            | "footer"
            | "nav"
            | "table"
            | "ul"
            | "ol"
            | "dd"
    )
}

fn is_style_format(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}'
            | '\u{200B}'
            | '\u{200C}'
            | '\u{200D}'
            | '\u{200E}'
            | '\u{200F}'
            | '\u{202A}'
            | '\u{202B}'
            | '\u{202C}'
            | '\u{202D}'
            | '\u{202E}'
            | '\u{2060}'
            | '\u{2066}'
            | '\u{2067}'
            | '\u{2068}'
            | '\u{2069}'
            | '\u{FEFF}'
    )
}

fn decode_entities(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'&'
            && let Some(semi) = s[i + 1..].find(';')
            && semi > 0
            && semi <= 16
            && let Some(ch) = decode_entity(&s[i + 1..i + 1 + semi])
        {
            out.push(ch);
            i += 1 + semi + 1;
            continue;
        }
        let ch = s[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn decode_entity(body: &str) -> Option<char> {
    match body {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "nbsp" | "ensp" | "emsp" | "thinsp" => Some(' '),
        _ => {
            let hex = body.strip_prefix("#x").or_else(|| body.strip_prefix("#X"));
            let code = if let Some(hex) = hex {
                u32::from_str_radix(hex, 16).ok()?
            } else {
                let dec = body.strip_prefix('#')?;
                dec.parse().ok()?
            };
            char::from_u32(code).filter(|c| *c != '\0' && !c.is_control())
        }
    }
}

pub fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_roundtrip() {
        for k in [
            ItemKind::Text,
            ItemKind::RichText,
            ItemKind::Image,
            ItemKind::File,
            ItemKind::Folder,
            ItemKind::Files,
        ] {
            assert_eq!(ItemKind::from_i64(k.as_i64()), Some(k));
        }
        assert_eq!(ItemKind::from_i64(99), None);
    }

    #[test]
    fn tag_cycles_both_ways() {
        assert_eq!(Tag::All.next(), Tag::Text);
        assert_eq!(Tag::Favorite.next(), Tag::All);
        assert_eq!(Tag::All.prev(), Tag::Favorite);
    }

    #[test]
    fn tag_names_roundtrip() {
        for t in Tag::ORDER {
            assert_eq!(Tag::parse(t.as_str()), Some(t));
        }
    }

    #[test]
    fn trim_drops_edge_blank_lines_and_fullwidth_spaces_only() {
        assert_eq!(trim_edge_whitespace("\n\n  hello\u{3000}\n"), "hello");
        assert_eq!(trim_edge_whitespace("\u{00A0}a\tb  "), "a\tb");
        assert_eq!(trim_edge_whitespace("a \n\n b"), "a \n\n b");
    }

    #[test]
    fn strip_styles_keeps_plain_text_from_console_and_html() {
        assert_eq!(strip_style_marks("\u{1b}[1;32mOK\u{1b}[0m"), "OK");
        assert_eq!(
            strip_style_marks("\u{1b}]8;;https://example.com\u{7}link\u{1b}]8;;\u{7}"),
            "link"
        );
        assert_eq!(strip_style_marks("\u{9b}31mred\u{9b}0m"), "red");
        assert_eq!(
            strip_style_marks("<span style=\"color:red\">hi</span>"),
            "hi"
        );
        assert_eq!(
            strip_style_marks("<p>a&amp;b&nbsp;c</p><p>d</p>"),
            "a&b c\nd"
        );
        assert_eq!(
            strip_style_marks("<script>alert(1)</script><b>ok</b>"),
            "ok"
        );
        assert_eq!(strip_style_marks("<!--x--><i>t</i>"), "t");
        assert_eq!(strip_style_marks("<span title=\"a>b\">x</span>"), "x");
        assert_eq!(
            strip_style_marks("if a < b && c > d { Vec<String> }"),
            "if a < b && c > d { Vec<String> }"
        );
        assert_eq!(strip_style_marks("a\u{200b}b\u{feff}"), "ab");
    }

    #[test]
    fn transforms_strip_line_breaks_or_all_whitespace() {
        let s = "a b\r\nc\td\n";
        assert_eq!(Transform::None.apply(s), s);
        assert_eq!(Transform::StripNewlines.apply(s), "a bc\td");
        assert_eq!(Transform::StripWhitespace.apply(s), "abcd");
    }

    #[test]
    fn text_digest_distinguishes_rich_from_plain() {
        let a = Item::text("hi".into(), None, None, 1.0);
        let b = Item::text("hi".into(), Some("<b>hi</b>".into()), None, 1.0);
        assert_ne!(a.digest, b.digest);
        assert_eq!(a.kind, ItemKind::Text);
        assert_eq!(b.kind, ItemKind::RichText);
    }

    #[test]
    fn digest_length_prefix_prevents_concat_collision() {
        let a = digest_parts(ItemKind::Text, &[b"ab", b"c"]);
        let b = digest_parts(ItemKind::Text, &[b"a", b"bc"]);
        assert_ne!(a, b);
    }

    #[test]
    fn preview_collapses_whitespace_and_truncates() {
        let it = Item::text("a \n\n  b\tc".into(), None, None, 0.0);
        assert_eq!(it.preview_line(10), "a b c");
        let long = Item::text("x".repeat(50), None, None, 0.0);
        let p = long.preview_line(10);
        assert_eq!(p.chars().count(), 10);
        assert!(p.ends_with('…'));
    }

    #[test]
    fn file_kinds_by_count_and_dir() {
        assert_eq!(
            Item::files(vec!["/a".into()], false, None, 0.0).kind,
            ItemKind::File
        );
        assert_eq!(
            Item::files(vec!["/a".into()], true, None, 0.0).kind,
            ItemKind::Folder
        );
        assert_eq!(
            Item::files(vec!["/a".into(), "/b".into()], false, None, 0.0).kind,
            ItemKind::Files
        );
    }
}
