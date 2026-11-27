//! 界面文案（中 / 英）与相对时间格式。
//! 文案取自 iCopy 简体中文本地化里对应功能的叫法。

use chrono::{DateTime, Datelike, Local, TimeZone};

use crate::config::Language;
use crate::model::{ItemKind, Tag};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    Zh,
    En,
}

impl Lang {
    /// `Auto` 跟随系统界面语言（`zh_CN.UTF-8`、`zh-Hans-CN` 之类）；没有 CJK 字体时中文无法显示，退回英文。
    pub fn resolve(setting: Language, system: Option<&str>, have_cjk_font: bool) -> Lang {
        let want_zh = match setting {
            Language::Zh => true,
            Language::En => false,
            Language::Auto => system.is_some_and(|v| v.to_ascii_lowercase().starts_with("zh")),
        };
        if want_zh && have_cjk_font {
            Lang::Zh
        } else {
            Lang::En
        }
    }

    /// 分类标签上的文字。
    pub fn tag(self, t: Tag) -> &'static str {
        match (self, t) {
            (Lang::Zh, Tag::All) => "全部",
            (Lang::Zh, Tag::Text) => "文本",
            (Lang::Zh, Tag::Image) => "图片",
            (Lang::Zh, Tag::File) => "文件",
            (Lang::Zh, Tag::Favorite) => "收藏",
            (Lang::En, Tag::All) => "All",
            (Lang::En, Tag::Text) => "Text",
            (Lang::En, Tag::Image) => "Image",
            (Lang::En, Tag::File) => "File",
            (Lang::En, Tag::Favorite) => "Collect",
        }
    }

    /// 卡片头部的类型名。
    pub fn kind(self, k: ItemKind) -> &'static str {
        match (self, k) {
            (Lang::Zh, ItemKind::Text) => "纯文本",
            (Lang::Zh, ItemKind::RichText) => "富文本",
            (Lang::Zh, ItemKind::Image) => "图片",
            (Lang::Zh, ItemKind::File) => "文件",
            (Lang::Zh, ItemKind::Folder) => "文件夹",
            (Lang::Zh, ItemKind::Files) => "多个文件(夹)",
            (Lang::En, ItemKind::Text) => "Plain text",
            (Lang::En, ItemKind::RichText) => "Rich text",
            (Lang::En, ItemKind::Image) => "Image",
            (Lang::En, ItemKind::File) => "File",
            (Lang::En, ItemKind::Folder) => "Folder",
            (Lang::En, ItemKind::Files) => "Multiple files",
        }
    }

    pub fn chars(self, n: usize) -> String {
        match self {
            Lang::Zh => format!("{n}个字符"),
            Lang::En => format!("{n} characters"),
        }
    }

    pub fn text(self, t: Text) -> &'static str {
        let (zh, en) = match t {
            Text::Search => ("搜索", "Search"),
            Text::Paste => ("粘贴", "Paste"),
            Text::PastePlain => ("粘贴为纯文本", "Paste as plain text"),
            Text::PasteRich => ("粘贴为富文本", "Paste as rich text"),
            Text::Copy => ("拷贝", "Copy"),
            Text::CopyPlain => ("拷贝为纯文本", "Copy as plain text"),
            Text::CopyNoNewlines => ("拷贝(去除换行)", "Copy (remove line breaks)"),
            Text::CopyNoWhitespace => (
                "拷贝(去除换行和空格)",
                "Copy (remove line breaks and spaces)",
            ),
            Text::Collect => ("收藏", "Collect"),
            Text::Uncollect => ("取消收藏", "Cancel collect"),
            Text::Delete => ("删除", "Delete"),
            Text::KindTips => ("温馨提示", "Kind tips"),
            Text::ConfirmDelete => ("是否确定删除？", "Confirm deletion?"),
            Text::Cancel => ("取消", "Cancel"),
            Text::Sure => ("确定", "Sure"),
            Text::PlainOn => (
                "粘贴为纯文本（点击切换为保留格式）",
                "Paste as plain text (click to keep formatting)",
            ),
            Text::PlainOff => (
                "保留格式粘贴（点击切换为纯文本）",
                "Keep formatting (click for plain text)",
            ),
            Text::Settings => ("设置", "Setting"),
            Text::Error => ("出错了", "Error"),
        };
        if self == Lang::Zh { zh } else { en }
    }

    /// 菜单栏菜单和设置窗口的文案，取自 iCopy 英文界面实测和简体中文本地化。
    pub fn menu(self, word: MenuWord) -> &'static str {
        let (zh, en) = match word {
            MenuWord::Setting => ("设置", "Setting"),
            MenuWord::Clipboard => ("剪切板", "Clipboard"),
            MenuWord::Quit => ("退出", "Quit"),
            MenuWord::General => ("通用设置", "General"),
            MenuWord::Blacklist => ("黑名单", "Black list"),
            MenuWord::ShowHidePanel => ("唤出/关闭剪切板面板", "Show/Hide Clipboard Panel"),
            MenuWord::ShortcutKey => ("快捷键", "Shortcut Key"),
            MenuWord::ShortcutRestart => (
                "更改快捷键后，重新启动守护进程才会生效",
                "The shortcut takes effect after restarting the daemon",
            ),
            MenuWord::NumberShortcut => ("数字快捷键", "Number Shortcut"),
            MenuWord::OnPress => ("按下后", "On Press"),
            MenuWord::DoubleClickEnter => ("双击/回车", "Double Click/Enter"),
            MenuWord::HoverSwitch => ("光标悬浮切换分组", "Hover to Switch Group"),
            MenuWord::PastePlain => ("粘贴为纯文本", "Paste as Plain Text"),
            MenuWord::TrimWhitespace => ("粘贴时清理首尾空白", "Trim whitespace when pasting"),
            MenuWord::StripStyles => (
                "粘贴时清理样式，只保留纯文本",
                "Strip styles when pasting, keep plain text",
            ),
            MenuWord::AutoClose => (
                "操作完成后自动关闭窗口",
                "Auto close window after operation",
            ),
            MenuWord::DefaultGroup => ("默认打开分组", "Default Open Group"),
            MenuWord::LastGroup => ("上次退出时分组", "Group at Last Exit"),
            MenuWord::DiskLimit => ("占用磁盘空间大小限制", "Disk Space Limit"),
            MenuWord::TimeLimit => ("存储时间限制", "Storage Time Limit"),
            MenuWord::Unlimited => ("无限制", "Unlimited"),
            MenuWord::Day => ("天", "Day"),
            MenuWord::Week => ("周", "Week"),
            MenuWord::Month => ("月", "Month"),
            MenuWord::Quarter => ("季度", "Quarter"),
            MenuWord::HalfYear => ("半年", "Half a year"),
            MenuWord::Year => ("年", "Year"),
            MenuWord::StartupDisplay => ("启动与显示", "Startup & Display"),
            MenuWord::LaunchAtStartup => ("开机自启动", "Launch at Startup"),
            MenuWord::LaunchHint => ("下次登录时生效", "Takes effect at next login"),
            MenuWord::ShowMenuIcon => ("显示菜单栏图标", "Show Menu Bar Icon"),
            MenuWord::ConfirmDelete => ("删除内容时二次确认", "Confirm Before Deleting Content"),
            MenuWord::Language => ("语言:", "Language:"),
            MenuWord::FollowSystem => ("跟随系统", "Follow System"),
            MenuWord::RestoreDefault => ("恢复默认配置", "Restore Default Settings"),
            MenuWord::Initialize => ("初始化", "Initialization"),
            MenuWord::SureRestore => (
                "是否确定将所有设置恢复到初始值？",
                "Sure to restore default config?",
            ),
            MenuWord::SureInitialize => (
                "初始化将清空所有数据，并将所有设置恢复到初始值，是否确定继续？",
                "Initializing will clear all data and restore all settings to default. Are you sure to continue?",
            ),
            MenuWord::BlacklistHelp => (
                "黑名单中的App，其剪切板内容不会被记录，以保护隐私内容。",
                "Apps on the blacklist will not have their clipboard content recorded to protect privacy.",
            ),
            MenuWord::BlacklistTitle => {
                ("剪切板监听黑名单", "Blacklist of system clipboard monitor")
            }
            MenuWord::PressShortcut => ("请按下快捷键…", "Press shortcut…"),
            MenuWord::AppNameHint => ("输入应用名，按 + 添加", "App name, then press +"),
            MenuWord::InteractionSettings => ("交互设置", "Interaction Settings"),
            MenuWord::HistoryLimit => ("剪切板历史记录限制", "Clipboard History Limit"),
            MenuWord::Nothing => ("无", "Nothing"),
            MenuWord::InputContent => ("输入剪切板内容", "Input Clipboard Content"),
            MenuWord::CopyContent => ("拷贝剪切板内容", "Copy Clipboard Content"),
            MenuWord::SelectOnly => ("只选中", "Select"),
        };
        if self == Lang::Zh { zh } else { en }
    }

    pub fn clear_tag(self, tag: Tag, keep_favorites: bool) -> String {
        let name = self.tag(tag);
        match (self, keep_favorites) {
            (Lang::Zh, false) => format!("清空\"{name}\"条目"),
            (Lang::Zh, true) => format!("清空\"{name}\"条目(不包含收藏)"),
            (Lang::En, false) => format!("Clear \"{name}\" items"),
            (Lang::En, true) => format!("Clear \"{name}\" items (without favorites)"),
        }
    }
}

/// 菜单栏和设置窗口用到的 iCopy 文案。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuWord {
    Setting,
    Clipboard,
    Quit,
    General,
    Blacklist,
    ShowHidePanel,
    ShortcutKey,
    ShortcutRestart,
    NumberShortcut,
    OnPress,
    DoubleClickEnter,
    HoverSwitch,
    PastePlain,
    TrimWhitespace,
    StripStyles,
    AutoClose,
    DefaultGroup,
    LastGroup,
    DiskLimit,
    TimeLimit,
    Unlimited,
    Day,
    Week,
    Month,
    Quarter,
    HalfYear,
    Year,
    StartupDisplay,
    LaunchAtStartup,
    LaunchHint,
    ShowMenuIcon,
    ConfirmDelete,
    Language,
    FollowSystem,
    RestoreDefault,
    Initialize,
    SureRestore,
    SureInitialize,
    BlacklistHelp,
    BlacklistTitle,
    AppNameHint,
    PressShortcut,
    InteractionSettings,
    HistoryLimit,
    Nothing,
    InputContent,
    CopyContent,
    SelectOnly,
}

/// 固定文案，取自 iCopy 本地化表里对应功能的叫法。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Text {
    Search,
    Paste,
    PastePlain,
    PasteRich,
    Copy,
    CopyPlain,
    CopyNoNewlines,
    CopyNoWhitespace,
    Collect,
    Uncollect,
    Delete,
    KindTips,
    ConfirmDelete,
    Cancel,
    Sure,
    PlainOn,
    PlainOff,
    Settings,
    Error,
}

/// 对应 iCopy 的“刚刚 / N分钟前 / 今天 HH:mm / MM-dd / yyyy-MM-dd”。
pub fn relative_time(lang: Lang, now: DateTime<Local>, then: DateTime<Local>) -> String {
    let secs = (now - then).num_seconds();
    if secs < 60 {
        return match lang {
            Lang::Zh => "刚刚".into(),
            Lang::En => "just".into(),
        };
    }
    if secs < 3600 {
        let m = secs / 60;
        return match lang {
            Lang::Zh => format!("{m}分钟前"),
            Lang::En => format!("{m} minutes ago"),
        };
    }
    if now.date_naive() == then.date_naive() {
        let hm = then.format("%H:%M");
        return match lang {
            Lang::Zh => format!("今天 {hm}"),
            Lang::En => format!("Today {hm}"),
        };
    }
    match (lang, now.year() == then.year()) {
        (Lang::Zh, true) => format!("{:02}月{:02}日", then.month(), then.day()),
        (Lang::Zh, false) => format!("{}年{:02}月{:02}日", then.year(), then.month(), then.day()),
        (Lang::En, true) => then.format("%m-%d").to_string(),
        (Lang::En, false) => then.format("%Y-%m-%d").to_string(),
    }
}

pub fn from_unix(ts: f64) -> DateTime<Local> {
    Local
        .timestamp_opt(ts as i64, 0)
        .single()
        .unwrap_or_else(Local::now)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> DateTime<Local> {
        Local.with_ymd_and_hms(y, mo, d, h, mi, s).single().unwrap()
    }

    #[test]
    fn relative_time_buckets() {
        let now = at(2026, 11, 24, 15, 0, 0);
        assert_eq!(
            relative_time(Lang::Zh, now, at(2026, 11, 24, 14, 59, 30)),
            "刚刚"
        );
        assert_eq!(
            relative_time(Lang::En, now, at(2026, 11, 24, 14, 59, 30)),
            "just"
        );
        assert_eq!(
            relative_time(Lang::Zh, now, at(2026, 11, 24, 14, 55, 0)),
            "5分钟前"
        );
        assert_eq!(
            relative_time(Lang::En, now, at(2026, 11, 24, 9, 5, 0)),
            "Today 09:05"
        );
        assert_eq!(
            relative_time(Lang::Zh, now, at(2026, 11, 20, 9, 5, 0)),
            "11月20日"
        );
        assert_eq!(
            relative_time(Lang::En, now, at(2026, 11, 20, 9, 5, 0)),
            "11-20"
        );
        assert_eq!(
            relative_time(Lang::Zh, now, at(2025, 3, 2, 9, 5, 0)),
            "2025年03月02日"
        );
    }

    #[test]
    fn clear_tag_uses_icopy_wording() {
        assert_eq!(
            Lang::Zh.clear_tag(Tag::Text, true),
            "清空\"文本\"条目(不包含收藏)"
        );
        assert_eq!(Lang::En.clear_tag(Tag::All, false), "Clear \"All\" items");
    }

    #[test]
    fn language_follows_system_and_needs_cjk_font_for_zh() {
        assert_eq!(Lang::resolve(Language::Zh, None, false), Lang::En);
        assert_eq!(Lang::resolve(Language::Zh, None, true), Lang::Zh);
        assert_eq!(
            Lang::resolve(Language::En, Some("zh_CN.UTF-8"), true),
            Lang::En
        );
        assert_eq!(
            Lang::resolve(Language::Auto, Some("zh-Hans-CN"), true),
            Lang::Zh
        );
        assert_eq!(Lang::resolve(Language::Auto, Some("en-CN"), true), Lang::En);
        assert_eq!(Lang::resolve(Language::Auto, None, true), Lang::En);
    }

    #[test]
    fn menu_uses_icopy_status_item_wording() {
        assert_eq!(Lang::En.menu(MenuWord::Setting), "Setting");
        assert_eq!(Lang::Zh.menu(MenuWord::Clipboard), "剪切板");
        assert_eq!(Lang::En.menu(MenuWord::Quit), "Quit");
    }
}
