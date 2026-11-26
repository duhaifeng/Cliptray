//! “开机自启动”。
//!
//! macOS 写 `~/Library/LaunchAgents/com.cliptray.app.plist`，Linux 写
//! `~/.config/autostart/cliptray.desktop`。只写文件，不 `launchctl bootstrap`：
//! bootstrap 会立刻再拉起一个进程，和正在运行的守护进程抢 socket。下次登录才生效。

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

const MAC_LABEL: &str = "com.cliptray.app";
const LINUX_NAME: &str = "cliptray.desktop";

pub fn apply(enabled: bool, program: &Path) -> Result<()> {
    let home = std::env::var_os("HOME").context("没有 HOME，无法写入开机启动项")?;
    apply_in(&PathBuf::from(home), enabled, program)
}

pub fn apply_in(home: &Path, enabled: bool, program: &Path) -> Result<()> {
    let path = autostart_path(home);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("创建目录失败: {}", parent.display()))?;
    }
    if enabled {
        let body = if cfg!(target_os = "macos") {
            launch_agent_plist(program)
        } else {
            desktop_entry(program)
        };
        std::fs::write(&path, body)
            .with_context(|| format!("写入开机启动项失败: {}", path.display()))?;
    } else if path.exists() {
        std::fs::remove_file(&path)
            .with_context(|| format!("移除开机启动项失败: {}", path.display()))?;
    }
    Ok(())
}

fn autostart_path(home: &Path) -> PathBuf {
    if cfg!(target_os = "macos") {
        home.join("Library/LaunchAgents")
            .join(format!("{MAC_LABEL}.plist"))
    } else {
        home.join(".config/autostart").join(LINUX_NAME)
    }
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

pub fn launch_agent_plist(program: &Path) -> String {
    let program = xml_escape(&program.display().to_string());
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{MAC_LABEL}</string>
  <key>ProgramArguments</key>
  <array>
    <string>{program}</string>
    <string>daemon</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
</dict>
</plist>
"#
    )
}

pub fn desktop_entry(program: &Path) -> String {
    let program = program.display().to_string().replace('"', "\\\"");
    format!(
        "[Desktop Entry]\nType=Application\nName=Cliptray\nComment=Clipboard history\nExec=\"{program}\" daemon\nTerminal=false\nX-GNOME-Autostart-enabled=true\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enable_writes_and_disable_removes() {
        let home = tempfile::tempdir().unwrap();
        let program = Path::new("/opt/cliptray");
        apply_in(home.path(), true, program).unwrap();
        let path = autostart_path(home.path());
        let body = std::fs::read_to_string(&path).unwrap();
        if cfg!(target_os = "macos") {
            assert!(body.contains("/opt/cliptray"));
            assert!(body.contains("<string>daemon</string>"));
            assert!(body.contains(MAC_LABEL));
        } else {
            assert!(body.contains("Exec=\"/opt/cliptray\" daemon"));
        }
        apply_in(home.path(), false, program).unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn plist_escapes_xml() {
        let body = launch_agent_plist(Path::new("/tmp/a&b"));
        assert!(body.contains("/tmp/a&amp;b"));
        assert!(!body.contains("/tmp/a&b<"));
    }
}
