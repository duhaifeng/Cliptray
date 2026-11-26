//! GNOME Shell + Wayland 下的全局热键。
//!
//! Wayland 客户端收不到全局按键，XWayland 的 `XGrabKey` 也只在焦点是 X11 窗口时有效，
//! 所以由 `gnome/cliptray@cliptray.local` 这个 Shell 扩展替我们抓，
//! 本模块通过 session bus 上的 `local.cliptray.Shell` 驱动它：
//! 调 `SetShortcuts` 注册，收 `Shortcut` 信号触发，扩展（重新）启动时收 `Ready` 重新注册。

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use zbus::blocking::{Connection, Proxy};

use crate::hotkey::HotkeySpec;

const BUS_NAME: &str = "local.cliptray.Shell";
const OBJECT_PATH: &str = "/local/cliptray/Shell";
const INTERFACE: &str = "local.cliptray.Shell";
/// 守护进程持有的名字；扩展靠它知道什么时候该释放热键。
const DAEMON_NAME: &str = "local.cliptray.Daemon";

const EXTENSION_UUID: &str = "cliptray@cliptray.local";
const SYSTEM_EXTENSION_DIR: &str = "/usr/share/gnome-shell/extensions";
/// 与 `gnome/cliptray@cliptray.local/extension.js` 的 EXTENSION_VERSION 一致。
const EXTENSION_VERSION: u32 = 1;
const EXTENSION_METADATA: &str = include_str!("../../gnome/cliptray@cliptray.local/metadata.json");
const EXTENSION_SCRIPT: &str = include_str!("../../gnome/cliptray@cliptray.local/extension.js");

/// 启动时 Shell 可能还在加载扩展（登录后自启动），最多等这么久。
const LOGIN_PATIENCE: Duration = Duration::from_secs(8);

/// 当前是 GNOME 的 Wayland 会话。`CLIPTRAY_BACKEND=x11` 可以强制走 X11。
pub fn is_gnome_wayland() -> bool {
    if std::env::var("CLIPTRAY_BACKEND").is_ok_and(|v| v.eq_ignore_ascii_case("x11")) {
        return false;
    }
    let wayland = std::env::var("XDG_SESSION_TYPE")
        .is_ok_and(|t| t.eq_ignore_ascii_case("wayland"))
        || std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty());
    let gnome = std::env::var("XDG_CURRENT_DESKTOP")
        .is_ok_and(|d| d.to_ascii_lowercase().contains("gnome"));
    wayland && gnome
}

/// 换热键的函数：不重启就换掉扩展里抓的热键。
pub type Rebinder = Box<dyn Fn(&HotkeySpec) -> Result<()> + Send>;

/// 通过 GNOME Shell 扩展注册全局热键。失败（扩展没装好、热键被桌面占用）会同步返回错误。
pub fn spawn_hotkey_listener(spec: &HotkeySpec, tx: Sender<()>) -> Result<Rebinder> {
    let connection = Connection::session().context("无法连接 session bus")?;
    let proxy = connect(&connection)?;

    check_version(&proxy);
    // 先占名字再抓热键：扩展看到名字消失就会释放热键。
    connection
        .request_name(DAEMON_NAME)
        .with_context(|| format!("无法占用 D-Bus 名字 {DAEMON_NAME}（已有 Cliptray 在运行？）"))?;
    // (accelerator, 给人看的写法)；信号线程和换热键的调用共用。
    let current = Arc::new(Mutex::new((spec.gnome_accelerator(), spec.to_string())));
    {
        let (accelerator, label) = current.lock().unwrap().clone();
        grab(&proxy, &accelerator, &label)?;
    }

    let signals = proxy.receive_all_signals()?;
    let thread_proxy = proxy.clone();
    let thread_current = Arc::clone(&current);
    std::thread::Builder::new()
        .name("gnome-hotkey".into())
        .spawn(move || {
            // 持有连接，守护进程退出前名字不能释放。
            let _connection = connection;
            let mut last = Instant::now() - Duration::from_secs(1);
            for message in signals {
                let header = message.header();
                match header.member().map(|m| m.as_str()) {
                    Some("Ready") => {
                        let (accelerator, label) = thread_current.lock().unwrap().clone();
                        if let Err(e) = grab(&thread_proxy, &accelerator, &label) {
                            eprintln!("GNOME 扩展重启后重新注册热键失败: {e:#}");
                        }
                    }
                    Some("Shortcut") => {
                        let Ok(got) = message.body().deserialize::<String>() else {
                            continue;
                        };
                        let wanted = thread_current.lock().unwrap().0.clone();
                        if got != wanted || last.elapsed() < Duration::from_millis(300) {
                            continue;
                        }
                        last = Instant::now();
                        if tx.send(()).is_err() {
                            break;
                        }
                    }
                    _ => {}
                }
            }
        })?;

    Ok(Box::new(move |new: &HotkeySpec| {
        let (new_accel, new_label) = (new.gnome_accelerator(), new.to_string());
        let (old_accel, old_label) = current.lock().unwrap().clone();
        if new_accel == old_accel {
            return Ok(());
        }
        // SetShortcuts 会先放掉旧的；新的被拒绝时要把旧的抓回来。
        match grab(&proxy, &new_accel, &new_label) {
            Ok(()) => {
                *current.lock().unwrap() = (new_accel, new_label);
                Ok(())
            }
            Err(e) => {
                let _ = grab(&proxy, &old_accel, &old_label);
                Err(e)
            }
        }
    }))
}

fn grab(proxy: &Proxy<'_>, accelerator: &str, label: &str) -> Result<()> {
    let failed: Vec<String> = proxy.call("SetShortcuts", &(vec![accelerator.to_owned()],))?;
    if !failed.is_empty() {
        bail!(
            "GNOME 拒绝了热键 {label} ({accelerator})：可能已被桌面或其他程序占用，请在设置里换一个"
        );
    }
    Ok(())
}

fn check_version(proxy: &Proxy<'_>) {
    match proxy.call::<_, _, u32>("Version", &()) {
        Ok(EXTENSION_VERSION) => {}
        found => {
            eprintln!(
                "GNOME Shell 里运行的 Cliptray 扩展版本是 {found:?}，期望 {EXTENSION_VERSION}；\
                 注销再登录一次才会加载新版本"
            );
            notify("Cliptray 扩展已更新，请注销再登录一次让 GNOME 加载新版本。");
        }
    }
}

/// 连上扩展。连不上时尝试安装并启用扩展；刚装好需要重新登录才会被 Shell 发现。
fn connect(connection: &Connection) -> Result<Proxy<'static>> {
    let proxy = Proxy::new(connection, BUS_NAME, OBJECT_PATH, INTERFACE)?;
    let probe = |p: &Proxy<'_>| p.call::<_, _, u32>("Version", &());
    let first = match probe(&proxy) {
        Ok(_) => return Ok(proxy),
        Err(e) => e,
    };

    let setup = prepare_extension();
    // 扩展早就装好并启用，多半只是 Shell 还在加载它（登录后立刻自启动）。
    if matches!(setup, Ok(false)) {
        let deadline = Instant::now() + LOGIN_PATIENCE;
        while Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(500));
            if probe(&proxy).is_ok() {
                return Ok(proxy);
            }
        }
    }
    let hint = match setup {
        Ok(true) => {
            notify("已安装 Cliptray 的 GNOME 扩展，请注销再登录一次以启用全局热键。");
            "已安装并启用 GNOME 扩展，但 Wayland 下 Shell 不能热重启：注销再登录一次后生效"
                .to_owned()
        }
        Ok(false) => format!(
            "扩展已安装并启用，但 Shell 没有加载它；查看 `gnome-extensions info {EXTENSION_UUID}`（扩展只声明了 GNOME Shell 50）"
        ),
        Err(e) => format!("扩展没有运行，自动安装也失败了（{e:#}）"),
    };
    Err(anyhow!(first)).context(hint)
}

/// 桌面通知；失败不算错误。
fn notify(body: &str) {
    let _ = Command::new("notify-send")
        .args(["-a", "Cliptray", "Cliptray", body])
        .status();
}

fn user_extension_dir() -> Result<PathBuf> {
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .context("XDG_DATA_HOME 和 HOME 都没有设置")?;
    Ok(data_home
        .join("gnome-shell/extensions")
        .join(EXTENSION_UUID))
}

fn system_extension_installed() -> bool {
    Path::new(SYSTEM_EXTENSION_DIR)
        .join(EXTENSION_UUID)
        .join("metadata.json")
        .is_file()
}

/// 把内置的扩展写进用户目录，返回是否有文件变化。
fn write_user_extension(dir: &Path) -> Result<bool> {
    std::fs::create_dir_all(dir).with_context(|| format!("创建 {}", dir.display()))?;
    let mut changed = false;
    for (name, contents) in [
        ("metadata.json", EXTENSION_METADATA),
        ("extension.js", EXTENSION_SCRIPT),
    ] {
        let path = dir.join(name);
        if std::fs::read_to_string(&path).is_ok_and(|c| c == contents) {
            continue;
        }
        std::fs::write(&path, contents).with_context(|| format!("写入 {}", path.display()))?;
        changed = true;
    }
    Ok(changed)
}

/// 让 Shell 在下次登录时加载当前版本的扩展：有系统级副本就用它（用户目录里的旧副本会盖住它，要删），
/// 否则写用户目录；两种情况都要在 `enabled-extensions` 里登记。返回是否有改动，也就是是否需要重新登录。
fn prepare_extension() -> Result<bool> {
    let user_dir = user_extension_dir()?;
    let mut changed = false;
    if system_extension_installed() {
        if user_dir.exists() {
            std::fs::remove_dir_all(&user_dir)
                .with_context(|| format!("删除 {}", user_dir.display()))?;
            changed = true;
        }
    } else {
        changed |= write_user_extension(&user_dir)?;
    }
    changed |= enable_in_settings()?;
    Ok(changed)
}

/// 把扩展加进 `enabled-extensions`，返回是否原本没有。
fn enable_in_settings() -> Result<bool> {
    let gsettings = |args: &[&str]| -> Result<String> {
        let out = Command::new("gsettings")
            .args(args)
            .output()
            .context("运行 gsettings")?;
        anyhow::ensure!(
            out.status.success(),
            "gsettings {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    };
    let current = gsettings(&["get", "org.gnome.shell", "enabled-extensions"])?;
    let mut ids: Vec<&str> = current.split('\'').skip(1).step_by(2).collect();
    if ids.contains(&EXTENSION_UUID) {
        return Ok(false);
    }
    ids.push(EXTENSION_UUID);
    let value = format!(
        "[{}]",
        ids.iter()
            .map(|id| format!("'{id}'"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    gsettings(&["set", "org.gnome.shell", "enabled-extensions", &value])?;
    Ok(true)
}

/// 用 Mutter 的 RemoteDesktop 接口向当前焦点窗口发送一次按键组合。
///
/// GNOME 的合成器没有 virtual-keyboard 协议，`wtype` 在这里不能用；RemoteDesktop 是
/// 桌面自己提供的输入注入接口（会话内的普通程序即可调用），按键用 keysym 给出，
/// 由合成器按当前键盘布局翻译，所以对调了 Ctrl 和 Win 之类的布局也不会按错键。
pub fn send_key(spec: &HotkeySpec) -> Result<()> {
    const DEST: &str = "org.gnome.Mutter.RemoteDesktop";
    const SESSION_IFACE: &str = "org.gnome.Mutter.RemoteDesktop.Session";
    const KEYSYM_SHIFT_L: u32 = 0xffe1;
    const KEYSYM_CTRL_L: u32 = 0xffe3;
    const KEYSYM_ALT_L: u32 = 0xffe9;
    const KEYSYM_SUPER_L: u32 = 0xffeb;

    let connection = Connection::session().context("无法连接 session bus")?;
    let session: zbus::zvariant::OwnedObjectPath = connection
        .call_method(
            Some(DEST),
            "/org/gnome/Mutter/RemoteDesktop",
            Some(DEST),
            "CreateSession",
            &(),
        )
        .context("GNOME 的 RemoteDesktop 接口不可用，无法模拟粘贴")?
        .body()
        .deserialize()?;
    fn call<B>(connection: &Connection, session: &str, method: &str, args: &B) -> Result<()>
    where
        B: serde::Serialize + zbus::zvariant::DynamicType,
    {
        connection
            .call_method(Some(DEST), session, Some(SESSION_IFACE), method, args)
            .with_context(|| format!("RemoteDesktop.{method} 失败"))?;
        Ok(())
    }
    let session = session.as_str();
    call(&connection, session, "Start", &())?;
    let result = (|| -> Result<()> {
        let mut mods: Vec<u32> = Vec::new();
        for (on, sym) in [
            (spec.mods.ctrl, KEYSYM_CTRL_L),
            (spec.mods.alt, KEYSYM_ALT_L),
            (spec.mods.shift, KEYSYM_SHIFT_L),
            (spec.mods.super_key, KEYSYM_SUPER_L),
        ] {
            if on {
                mods.push(sym);
            }
        }
        let key =
            |sym: u32, down: bool| call(&connection, session, "NotifyKeyboardKeysym", &(sym, down));
        for m in &mods {
            key(*m, true)?;
        }
        key(spec.keysym(), true)?;
        key(spec.keysym(), false)?;
        for m in mods.iter().rev() {
            key(*m, false)?;
        }
        Ok(())
    })();
    let _ = call(&connection, session, "Stop", &());
    result
}
