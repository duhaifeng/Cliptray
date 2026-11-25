//! 来源应用图标缓存：`<数据目录>/icons/<应用名摘要>.png`。
//!
//! 守护进程在记录到某个应用的复制时导出一次图标；面板只读这些文件，不在弹出时查询系统。

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::model::hex;
use crate::platform;

pub fn icon_path(data_dir: &Path, app: &str) -> PathBuf {
    let h = hex(&Sha256::digest(app.as_bytes()));
    data_dir.join("icons").join(format!("{}.png", &h[..16]))
}

pub struct IconCache {
    data_dir: Option<PathBuf>,
    /// 本次运行已尝试过的应用：找不到图标的应用不会在每次复制时重新查询。
    tried: HashSet<String>,
}

impl IconCache {
    pub fn new(data_dir: &Path) -> Self {
        IconCache {
            data_dir: Some(data_dir.to_owned()),
            tried: HashSet::new(),
        }
    }

    pub fn disabled() -> Self {
        IconCache {
            data_dir: None,
            tried: HashSet::new(),
        }
    }

    pub fn ensure(&mut self, app: &str) {
        let Some(dir) = &self.data_dir else { return };
        if !self.tried.insert(app.to_owned()) {
            return;
        }
        let dest = icon_path(dir, app);
        if dest.is_file() {
            return;
        }
        if let Err(e) = export(app, &dest) {
            eprintln!("导出 {app} 的图标失败: {e:#}");
        }
    }
}

/// 先写临时文件再改名，面板不会读到写了一半的 PNG。
fn export(app: &str, dest: &Path) -> anyhow::Result<()> {
    let dir = dest.parent().expect("图标路径带目录");
    std::fs::create_dir_all(dir)?;
    let tmp = dest.with_extension("png.tmp");
    match platform::export_app_icon(app, &tmp) {
        Ok(true) => Ok(std::fs::rename(&tmp, dest)?),
        other => {
            let _ = std::fs::remove_file(&tmp);
            other.map(|_| ())
        }
    }
}

/// `$XDG_DATA_HOME` 与 `$XDG_DATA_DIRS`（含 Flatpak 导出目录），按优先级排列。
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub fn xdg_data_dirs() -> Vec<PathBuf> {
    let env_dir = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty());
    let home = env_dir("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")));
    let system = env_dir("XDG_DATA_DIRS")
        .map(|v| std::env::split_paths(&v).collect::<Vec<_>>())
        .unwrap_or_else(|| vec!["/usr/local/share".into(), "/usr/share".into()]);
    home.into_iter()
        .chain(system)
        .chain([PathBuf::from("/var/lib/flatpak/exports/share")])
        .collect()
}

const ICON_SIZES: &[&str] = &["64x64", "48x48", "96x96", "128x128", "256x256", "32x32"];

/// 按 WM_CLASS 找 `.desktop` 文件里的 `Icon=`，再到 hicolor / pixmaps 里找 PNG。
/// SVG 图标不处理（面板只解码 PNG）。
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub fn find_desktop_icon(wm_class: &str, data_dirs: &[PathBuf]) -> Option<PathBuf> {
    let icon = data_dirs
        .iter()
        .find_map(|d| desktop_icon_name(&d.join("applications"), wm_class))?;
    let p = Path::new(&icon);
    if p.is_absolute() {
        return (p.extension().is_some_and(|e| e == "png") && p.is_file()).then(|| p.to_owned());
    }
    data_dirs.iter().find_map(|d| {
        ICON_SIZES
            .iter()
            .map(|s| {
                d.join("icons/hicolor")
                    .join(s)
                    .join("apps")
                    .join(format!("{icon}.png"))
            })
            .chain([d.join("pixmaps").join(format!("{icon}.png"))])
            .find(|p| p.is_file())
    })
}

/// 在一个 applications 目录里找与 WM_CLASS 对应的条目：`StartupWMClass` 相同优先，
/// 其次文件名（或反向域名的最后一段，如 `org.gnome.Nautilus`）相同。
fn desktop_icon_name(dir: &Path, wm_class: &str) -> Option<String> {
    let mut by_name = None;
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "desktop") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Some(entry) = parse_desktop_entry(&text) else {
            continue;
        };
        let Some(icon) = entry.icon else { continue };
        if entry
            .wm_class
            .is_some_and(|c| c.eq_ignore_ascii_case(wm_class))
        {
            return Some(icon);
        }
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let last = stem.rsplit('.').next().unwrap_or(&stem);
        if by_name.is_none()
            && (stem.eq_ignore_ascii_case(wm_class) || last.eq_ignore_ascii_case(wm_class))
        {
            by_name = Some(icon);
        }
    }
    by_name
}

struct DesktopEntry {
    icon: Option<String>,
    wm_class: Option<String>,
}

/// 只读 `[Desktop Entry]` 组；`[Desktop Action …]` 组里的 `Icon=` 不算。
fn parse_desktop_entry(text: &str) -> Option<DesktopEntry> {
    let mut in_main = false;
    let mut seen_main = false;
    let mut e = DesktopEntry {
        icon: None,
        wm_class: None,
    };
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            in_main = line == "[Desktop Entry]";
            seen_main |= in_main;
            continue;
        }
        if !in_main {
            continue;
        }
        if let Some(v) = line.strip_prefix("Icon=") {
            e.icon = Some(v.trim().to_owned()).filter(|v| !v.is_empty());
        } else if let Some(v) = line.strip_prefix("StartupWMClass=") {
            e.wm_class = Some(v.trim().to_owned()).filter(|v| !v.is_empty());
        }
    }
    seen_main.then_some(e)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(p: &Path, s: &str) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, s).unwrap();
    }

    #[test]
    fn icon_path_is_stable_and_filesystem_safe() {
        let a = icon_path(Path::new("/d"), "Google Chrome");
        assert_eq!(a, icon_path(Path::new("/d"), "Google Chrome"));
        assert_ne!(a, icon_path(Path::new("/d"), "Cursor"));
        assert_eq!(a.parent(), Some(Path::new("/d/icons")));
        assert!(icon_path(Path::new("/d"), "../x/y").starts_with("/d/icons"));
    }

    #[test]
    fn desktop_lookup_prefers_wm_class_then_file_name() {
        let d = tempfile::tempdir().unwrap();
        let share = d.path().join("share");
        write(
            &share.join("applications/code.desktop"),
            "[Desktop Entry]\nName=VS Code\nIcon=vscode\nStartupWMClass=Code\n[Desktop Action new]\nIcon=other\n",
        );
        write(
            &share.join("applications/org.gnome.Nautilus.desktop"),
            "[Desktop Entry]\nIcon=org.gnome.Nautilus\n",
        );
        write(&share.join("icons/hicolor/48x48/apps/vscode.png"), "png");
        write(&share.join("pixmaps/org.gnome.Nautilus.png"), "png");
        let dirs = [share.clone()];
        assert_eq!(
            find_desktop_icon("code", &dirs),
            Some(share.join("icons/hicolor/48x48/apps/vscode.png"))
        );
        assert_eq!(
            find_desktop_icon("Nautilus", &dirs),
            Some(share.join("pixmaps/org.gnome.Nautilus.png"))
        );
        assert_eq!(find_desktop_icon("firefox", &dirs), None);
    }

    #[test]
    fn svg_only_and_action_group_icons_are_ignored() {
        let d = tempfile::tempdir().unwrap();
        let share = d.path().to_owned();
        write(
            &share.join("applications/a.desktop"),
            "[Desktop Action x]\nIcon=a\n",
        );
        write(
            &share.join("applications/b.desktop"),
            "[Desktop Entry]\nIcon=b\n",
        );
        write(&share.join("icons/hicolor/scalable/apps/b.svg"), "svg");
        write(&share.join("pixmaps/a.png"), "png");
        let dirs = [share];
        assert_eq!(find_desktop_icon("a", &dirs), None);
        assert_eq!(find_desktop_icon("b", &dirs), None);
    }
}
