//! 配置。字段对应 iCopy `ConfigModel` 中适用于 Linux 的部分。
//! 配置文件是 TOML；缺省字段取默认值，因此旧配置在新版本里仍然可用。

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

use crate::hotkey::HotkeySpec;
use crate::model::Tag;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    /// 写入剪切板并向前一个窗口发送粘贴键。
    #[default]
    Paste,
    /// 只写入剪切板。
    Copy,
}

/// 单击卡片的行为（iCopy“按下时”）。`select` 只选中，这时双击才有意义。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ClickAction {
    #[default]
    Paste,
    Copy,
    Select,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    #[default]
    Auto,
    Zh,
    En,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// 唤出 / 关闭历史面板。iCopy 默认 ⇧⌘C，Linux 上 ⌘ 对应 Super。
    pub hotkey: String,
    /// 自动粘贴时发送的按键；Linux 终端里常需要 `ctrl+shift+v`。
    pub paste_key: String,
    pub click_action: ClickAction,
    pub enter_action: Action,
    pub double_click_action: Action,
    /// 文本以纯文本写回（iCopy“粘贴为纯文本”，面板工具栏的 T 按钮可临时切换）。
    pub paste_plain_text: bool,
    /// 鼠标悬停在分类标签上即切换分类（iCopy“悬停切换分组”）。
    pub hover_switch_group: bool,
    /// 自动粘贴前等待焦点回到原窗口的毫秒数。
    pub paste_delay_ms: u64,
    /// 面板打开时的分类：`last`（上次退出时的分类）/ `all` / `text` / `image` / `file` / `favorite`。
    pub default_tag: String,
    /// 剪切板轮询间隔（毫秒）。Linux 没有 changeCount，只能轮询比对内容。
    pub poll_interval_ms: u64,
    /// 历史保留天数，0 为不限（iCopy 的“存储时间限制”）。
    pub history_days: u32,
    /// 图片占用磁盘上限（MiB），0 为不限（iCopy 的“占用磁盘空间大小限制”）。
    pub disk_limit_mb: u64,
    /// 单条文本最大字节数，超出则不记录，避免超大剪切板拖垮内存。
    pub max_text_bytes: usize,
    /// 来源应用黑名单：按应用名（X11 为 WM_CLASS，macOS 为应用显示名）大小写不敏感地精确匹配。
    pub blacklist: Vec<String>,
    /// 数字键快速粘贴的修饰键：`none` / `alt` / `ctrl`。`none` 时数字键在搜索框外直接粘贴。
    pub number_modifier: String,
    pub confirm_delete: bool,
    /// 操作完成后自动关闭面板（iCopy “Auto close window after operation”）。
    pub auto_hide: bool,
    /// 在菜单栏 / 状态栏显示图标。关掉之后仍可用热键唤出面板，再从面板里的设置按钮打开。
    pub show_status_item: bool,
    /// 登录时启动守护进程。macOS 写 LaunchAgent，Linux 写 XDG autostart；下次登录生效。
    pub launch_at_startup: bool,
    pub page_size: usize,
    pub language: Language,
    /// 数据目录；为空时用 XDG 数据目录。
    pub data_dir: Option<PathBuf>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            hotkey: "shift+super+c".into(),
            paste_key: if cfg!(target_os = "macos") {
                "super+v"
            } else {
                "ctrl+v"
            }
            .into(),
            click_action: ClickAction::Paste,
            enter_action: Action::Paste,
            double_click_action: Action::Paste,
            paste_plain_text: true,
            hover_switch_group: true,
            paste_delay_ms: 150,
            default_tag: "last".into(),
            poll_interval_ms: 300,
            history_days: 0,
            disk_limit_mb: 1024,
            max_text_bytes: 4 * 1024 * 1024,
            blacklist: Vec::new(),
            number_modifier: "none".into(),
            confirm_delete: true,
            auto_hide: true,
            show_status_item: true,
            launch_at_startup: false,
            page_size: 50,
            language: Language::Auto,
            data_dir: None,
        }
    }
}

impl Config {
    pub fn parse(s: &str) -> Result<Self> {
        let cfg: Config = toml::from_str(s).context("配置文件格式错误")?;
        cfg.validate()?;
        Ok(cfg)
    }

    pub fn load(path: &Path) -> Result<Self> {
        match fs::read_to_string(path) {
            Ok(s) => Self::parse(&s).with_context(|| format!("读取配置失败: {}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
            Err(e) => Err(e).with_context(|| format!("读取配置失败: {}", path.display())),
        }
    }

    pub fn validate(&self) -> Result<()> {
        self.hotkey
            .parse::<HotkeySpec>()
            .with_context(|| format!("hotkey = {:?}", self.hotkey))?;
        self.paste_key
            .parse::<HotkeySpec>()
            .with_context(|| format!("paste_key = {:?}", self.paste_key))?;
        anyhow::ensure!(
            self.default_tag == "last" || Tag::parse(&self.default_tag).is_some(),
            "default_tag 无效: {}",
            self.default_tag
        );
        anyhow::ensure!(
            matches!(self.number_modifier.as_str(), "alt" | "ctrl" | "none"),
            "number_modifier 只能是 alt / ctrl / none"
        );
        anyhow::ensure!(self.poll_interval_ms >= 50, "poll_interval_ms 不能小于 50");
        anyhow::ensure!(self.page_size >= 1, "page_size 至少为 1");
        Ok(())
    }

    pub fn hotkey_spec(&self) -> Result<HotkeySpec> {
        Ok(self.hotkey.parse()?)
    }

    pub fn paste_key_spec(&self) -> Result<HotkeySpec> {
        Ok(self.paste_key.parse()?)
    }

    /// 面板打开时的分类；`last` 时用 `last` 参数（上次退出时的分类，读不到为 `None`）。
    pub fn opening_tag(&self, last: Option<Tag>) -> Tag {
        match self.default_tag.as_str() {
            "last" => last.unwrap_or(Tag::All),
            s => Tag::parse(s).unwrap_or(Tag::All),
        }
    }

    /// 历史保留的起始时间；不限时为 `None`。
    pub fn cutoff(&self, now: f64) -> Option<f64> {
        (self.history_days > 0).then_some(now - self.history_days as f64 * 86_400.0)
    }

    pub fn is_blacklisted(&self, app: Option<&str>) -> bool {
        match app {
            Some(app) => self.blacklist.iter().any(|b| b.eq_ignore_ascii_case(app)),
            None => false,
        }
    }

    pub fn to_toml(&self) -> Result<String> {
        Ok(toml::to_string_pretty(self)?)
    }

    /// 校验后把当前配置写到 `path`，覆盖已有文件。
    pub fn save(&self, path: &Path) -> Result<()> {
        self.validate()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("创建目录失败: {}", parent.display()))?;
        }
        fs::write(path, self.to_toml()?)
            .with_context(|| format!("写入配置失败: {}", path.display()))
    }

    /// 把默认配置写到 `path`，覆盖已有文件；需要备份由调用方负责。
    pub fn write_default(path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("创建目录失败: {}", parent.display()))?;
        }
        fs::write(path, Config::default().to_toml()?)
            .with_context(|| format!("写入配置失败: {}", path.display()))
    }
}

pub struct Paths {
    pub config_file: PathBuf,
    pub data_dir: PathBuf,
    pub socket: PathBuf,
}

impl Paths {
    pub fn resolve(cfg: &Config, config_override: Option<&Path>) -> Result<Self> {
        let dirs = ProjectDirs::from("", "", "splay").context("无法确定用户主目录")?;
        let config_file = config_override
            .map(Path::to_owned)
            .unwrap_or_else(|| dirs.config_dir().join("config.toml"));
        let data_dir = cfg
            .data_dir
            .clone()
            .unwrap_or_else(|| dirs.data_dir().to_owned());
        let runtime = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                std::env::temp_dir().join(format!("splay-{}", unsafe { libc::getuid() }))
            });
        Ok(Paths {
            config_file,
            data_dir,
            socket: runtime.join("splay.sock"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid_and_match_icopy_hotkey() {
        let c = Config::default();
        c.validate().unwrap();
        let h = c.hotkey_spec().unwrap();
        assert!(h.mods.shift && h.mods.super_key);
        assert_eq!(h.key, "c");
    }

    #[test]
    fn partial_file_falls_back_to_defaults() {
        let c = Config::parse("click_action = \"select\"\nblacklist = [\"keepassxc\"]").unwrap();
        assert_eq!(c.click_action, ClickAction::Select);
        assert_eq!(c.hotkey, "shift+super+c");
        assert_eq!(c.blacklist, ["keepassxc"]);
        assert!(c.show_status_item);
        assert!(c.auto_hide);
        assert!(!c.launch_at_startup);
    }

    #[test]
    fn keys_from_older_versions_are_ignored() {
        let c = Config::parse("layout = \"vertical\"\nposition = \"cursor\"").unwrap();
        assert_eq!(c, Config::default());
    }

    #[test]
    fn opening_tag_follows_last_or_fixed_setting() {
        let c = Config::default();
        assert_eq!(c.opening_tag(Some(Tag::Image)), Tag::Image);
        assert_eq!(c.opening_tag(None), Tag::All);
        let c = Config {
            default_tag: "text".into(),
            ..Config::default()
        };
        assert_eq!(c.opening_tag(Some(Tag::Image)), Tag::Text);
    }

    #[test]
    fn invalid_values_are_rejected() {
        assert!(Config::parse("click_action = \"diagonal\"").is_err());
        assert!(Config::parse("default_tag = \"recent\"").is_err());
        assert!(Config::parse("hotkey = \"shift+nope\"").is_err());
        assert!(Config::parse("number_modifier = \"hyper\"").is_err());
        assert!(Config::parse("poll_interval_ms = 5").is_err());
    }

    #[test]
    fn roundtrip_through_toml() {
        let mut c = Config::default();
        c.blacklist.push("x".into());
        c.history_days = 30;
        let back = Config::parse(&c.to_toml().unwrap()).unwrap();
        assert_eq!(c, back);
    }

    #[test]
    fn blacklist_is_case_insensitive_exact() {
        let c = Config {
            blacklist: vec!["KeePassXC".into()],
            ..Config::default()
        };
        assert!(c.is_blacklisted(Some("keepassxc")));
        assert!(!c.is_blacklisted(Some("keepassxc2")));
        assert!(!c.is_blacklisted(None));
    }

    #[test]
    fn cutoff_zero_means_unlimited() {
        assert_eq!(Config::default().cutoff(1_000_000.0), None);
        let c = Config {
            history_days: 1,
            ..Config::default()
        };
        assert_eq!(c.cutoff(100_000.0), Some(100_000.0 - 86_400.0));
    }

    #[test]
    fn missing_file_yields_defaults() {
        let d = tempfile::tempdir().unwrap();
        let c = Config::load(&d.path().join("nope.toml")).unwrap();
        assert_eq!(c, Config::default());
    }
}
