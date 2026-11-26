//! 与桌面环境相关的能力。
//! Linux 提供 X11 热键、前台应用、屏幕尺寸和模拟粘贴，Wayland 下降级。
//! macOS 提供 Carbon 全局热键、前台应用、应用图标和 CGEvent 模拟粘贴。
//! 其他平台只提供桩，让核心逻辑能编译和测试。

use std::sync::mpsc::{Receiver, Sender};

use anyhow::Result;

use crate::hotkey::HotkeySpec;
use crate::monitor::ActiveApp;
#[cfg(not(target_os = "macos"))]
use crate::monitor::NoActiveApp;

#[cfg(target_os = "linux")]
mod gnome;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "linux")]
mod status_linux;
#[cfg(target_os = "macos")]
mod status_macos;
#[cfg(target_os = "linux")]
mod x11;

/// 会话类型，用于给出准确的降级提示。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Session {
    X11,
    Wayland,
    Macos,
    Unknown,
}

pub fn session() -> Session {
    #[cfg(target_os = "macos")]
    {
        Session::Macos
    }
    #[cfg(not(target_os = "macos"))]
    {
        let has = |k: &str| std::env::var_os(k).is_some_and(|v| !v.is_empty());
        if has("WAYLAND_DISPLAY") {
            Session::Wayland
        } else if has("DISPLAY") {
            Session::X11
        } else {
            Session::Unknown
        }
    }
}

/// 注册全局热键并运行 `body`，`body` 收到注册结果。注册成功后每次触发向 `tx` 发送一个 `()`。
/// 应在主线程调用：macOS 的 Carbon 事件必须由主线程分发，那里 `body` 会被放到工作线程上运行。
/// X11 和 macOS 由应用自己注册；纯 Wayland 不允许，需要把 `cliptray toggle` 绑定到桌面环境的快捷键。
pub fn run_with_hotkey<R: Send + 'static>(
    spec: &HotkeySpec,
    tx: Sender<()>,
    body: impl FnOnce(Result<()>) -> R + Send + 'static,
) -> R {
    run_session(spec, tx, None, body)
}

/// 和 [`run_with_hotkey`] 一样，另外在主线程上维护菜单栏图标。
pub fn run_session<R: Send + 'static>(
    spec: &HotkeySpec,
    tx: Sender<()>,
    status: Option<StatusIcon>,
    body: impl FnOnce(Result<()>) -> R + Send + 'static,
) -> R {
    #[cfg(target_os = "linux")]
    {
        *HOTKEY_TX.lock().unwrap() = Some(tx.clone());
        let registered = linux_hotkey(spec, tx);
        if let Some(status) = status {
            status_linux::start(status);
        }
        body(registered)
    }
    #[cfg(target_os = "macos")]
    {
        macos::run_session(spec, tx, status, body)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (spec, tx, status);
        body(Err(anyhow::anyhow!(
            "全局热键目前只支持 Linux/X11 和 macOS"
        )))
    }
}

/// 让子进程里的窗口走 X11（XWayland）。Wayland 不允许应用自己决定窗口位置，
/// 历史面板要贴在屏幕底边，就只能用 X11 窗口；有 `DISPLAY` 时才这样做。
/// 其他平台什么也不做。
pub fn prefer_x11_window(cmd: &mut std::process::Command) {
    #[cfg(target_os = "linux")]
    {
        let has_display = std::env::var_os("DISPLAY").is_some_and(|v| !v.is_empty());
        if session() == Session::Wayland && has_display {
            cmd.env_remove("WAYLAND_DISPLAY")
                .env("XDG_SESSION_TYPE", "x11")
                .env("WINIT_UNIX_BACKEND", "x11");
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = cmd;
    }
}

/// 换热键的函数；注册成功的后端把它放在这里。
type Rebinder = Box<dyn Fn(&HotkeySpec) -> Result<()> + Send>;
static REBINDER: std::sync::Mutex<Option<Rebinder>> = std::sync::Mutex::new(None);
/// 触发热键时要通知的通道，初次注册失败后再换热键时要用它重新注册。
#[cfg(target_os = "linux")]
static HOTKEY_TX: std::sync::Mutex<Option<Sender<()>>> = std::sync::Mutex::new(None);

/// 不重启守护进程就换全局热键。新热键注册不上时旧热键保持有效并返回错误；
/// 启动时注册就失败的，这里会重新尝试注册。macOS 目前需要重启。
pub fn rebind_hotkey(spec: &HotkeySpec) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        if let Some(rebind) = REBINDER.lock().unwrap().as_ref() {
            return rebind(spec);
        }
        let tx = HOTKEY_TX
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| anyhow::anyhow!("全局热键还没有初始化"))?;
        linux_hotkey(spec, tx)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = spec;
        anyhow::bail!("macOS 上更换快捷键需要重新启动守护进程")
    }
}

/// Linux 热键：GNOME Wayland 走 Shell 扩展（XGrabKey 在那里只对 X11 窗口有效），
/// 扩展不可用时退回 X11 并说明限制；其他会话走 X11；纯 Wayland 不允许抓取。
#[cfg(target_os = "linux")]
fn linux_hotkey(spec: &HotkeySpec, tx: Sender<()>) -> Result<()> {
    let x11_rebinder = |tx: Sender<()>| -> Result<()> {
        let cmd = x11::spawn_hotkey_listener(spec, tx)?;
        *REBINDER.lock().unwrap() = Some(Box::new(move |new: &HotkeySpec| {
            let (reply_tx, reply_rx) = std::sync::mpsc::channel();
            cmd.send((new.clone(), reply_tx))
                .map_err(|_| anyhow::anyhow!("热键线程已退出，需要重启守护进程"))?;
            reply_rx
                .recv_timeout(std::time::Duration::from_secs(3))
                .map_err(|_| anyhow::anyhow!("热键线程没有响应"))?
        }));
        Ok(())
    };
    if gnome::is_gnome_wayland() {
        let gnome_err = match gnome::spawn_hotkey_listener(spec, tx.clone()) {
            Ok(rebind) => {
                *REBINDER.lock().unwrap() = Some(rebind);
                return Ok(());
            }
            Err(e) => e,
        };
        return match x11_rebinder(tx) {
            Ok(()) => {
                eprintln!(
                    "GNOME 扩展不可用（{gnome_err:#}）；已退回 XWayland 抓取，\
                     只有焦点在 X11 窗口时热键才有效。"
                );
                Ok(())
            }
            Err(_) => Err(gnome_err),
        };
    }
    if session() == Session::Wayland && std::env::var_os("DISPLAY").is_none() {
        anyhow::bail!("纯 Wayland 会话不允许应用抓取全局热键");
    }
    x11_rebinder(tx)
}

/// 菜单栏菜单点下去的动作。文案跟 iCopy 状态栏菜单的实测顺序，广告项和未复刻的功能不在这里。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuAction {
    Toggle,
    Settings,
    Quit,
}

#[derive(Clone, Debug)]
pub struct TrayLabels {
    pub setting: String,
    pub clipboard: String,
    pub quit: String,
}

pub enum TrayControl {
    Visible(bool),
    Relabel(TrayLabels),
}

pub struct TrayIo {
    pub actions: Sender<MenuAction>,
    pub control: Receiver<TrayControl>,
}

pub struct StatusIcon {
    pub io: TrayIo,
    pub labels: TrayLabels,
    pub visible: bool,
}

pub fn active_app() -> Box<dyn ActiveApp> {
    #[cfg(target_os = "linux")]
    {
        match x11::X11ActiveApp::new() {
            Ok(a) => Box::new(a),
            Err(_) => Box::new(NoActiveApp),
        }
    }
    #[cfg(target_os = "macos")]
    {
        Box::new(macos::FrontApp)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        Box::new(NoActiveApp)
    }
}

/// 主屏幕尺寸，用来在面板第一帧之前摆好窗口；拿不到时返回 `None`（例如 Wayland）。
/// macOS 是逻辑点；X11 是物理像素，缩放不为 1 时由面板第一帧按 egui 报告的显示器尺寸纠正。
pub fn main_screen_size() -> Option<(f32, f32)> {
    #[cfg(target_os = "linux")]
    {
        x11::screen_size().map(|(w, h)| (w as f32, h as f32))
    }
    #[cfg(target_os = "macos")]
    {
        Some(macos::main_screen_size())
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        None
    }
}

/// 工作区相对屏幕四边的内缩 (左, 右, 上, 下)，单位与 [`main_screen_size`] 相同。
/// Dock、顶栏占的位置在里面；拿不到或不需要避开时返回 `None`（macOS 的面板有意盖在 Dock 上）。
pub fn work_area_insets() -> Option<(f32, f32, f32, f32)> {
    #[cfg(target_os = "linux")]
    {
        x11::work_area_insets().map(|(l, r, t, b)| (l as f32, r as f32, t as f32, b as f32))
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

/// 向当前焦点窗口发送粘贴按键。
pub fn send_paste(spec: &HotkeySpec) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        if session() == Session::Wayland {
            if gnome::is_gnome_wayland() {
                return gnome::send_key(spec);
            }
            return wtype(spec);
        }
        x11::send_key(spec)
    }
    #[cfg(target_os = "macos")]
    {
        macos::send_key(spec)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = spec;
        anyhow::bail!("模拟粘贴目前只支持 Linux 和 macOS")
    }
}

/// 界面语言：macOS 取系统偏好语言（iCopy 跟随它），其他平台取区域设置环境变量。
pub fn ui_language() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        macos::preferred_language()
    }
    #[cfg(not(target_os = "macos"))]
    {
        ["LC_ALL", "LC_MESSAGES", "LANG"]
            .iter()
            .filter_map(|k| std::env::var(k).ok())
            .find(|v| !v.is_empty())
    }
}

/// 把面板窗口提到 Dock 之上（iCopy 的面板层级）。
///
/// # Safety
/// `ns_view` 必须是主线程上仍然有效的 `NSView`。
#[cfg(target_os = "macos")]
pub unsafe fn raise_above_dock(ns_view: *mut std::ffi::c_void) {
    unsafe { macos::raise_above_dock(ns_view) }
}

/// macOS 合成按键需要「辅助功能」授权；其他平台不需要，恒为真。
pub fn accessibility_trusted() -> bool {
    #[cfg(target_os = "macos")]
    {
        macos::accessibility_trusted()
    }
    #[cfg(not(target_os = "macos"))]
    {
        true
    }
}

/// 把来源应用的图标导出成 PNG 写到 `out`。找不到图标时返回 `Ok(false)`。
pub fn export_app_icon(app: &str, out: &std::path::Path) -> Result<bool> {
    #[cfg(target_os = "macos")]
    {
        macos::export_app_icon(app, out)
    }
    #[cfg(not(target_os = "macos"))]
    {
        match crate::appicon::find_desktop_icon(app, &crate::appicon::xdg_data_dirs()) {
            Some(src) => {
                std::fs::copy(&src, out)?;
                Ok(true)
            }
            None => Ok(false),
        }
    }
}

/// Wayland 没有通用的按键注入协议，借助外部的 `wtype`（需要合成器支持 virtual-keyboard）。
#[cfg(target_os = "linux")]
fn wtype(spec: &HotkeySpec) -> Result<()> {
    let args = wtype_args(spec);
    let status = std::process::Command::new("wtype")
        .args(&args)
        .status()
        .map_err(|e| {
            anyhow::anyhow!(
                "Wayland 下模拟粘贴需要安装 wtype（{e}）；也可把 auto_paste 相关行为改为只复制"
            )
        })?;
    anyhow::ensure!(status.success(), "wtype 执行失败: {status}");
    Ok(())
}

#[cfg(any(target_os = "linux", test))]
pub(crate) fn wtype_args(spec: &HotkeySpec) -> Vec<String> {
    let mut args = Vec::new();
    let mods = [
        (spec.mods.ctrl, "ctrl"),
        (spec.mods.alt, "alt"),
        (spec.mods.shift, "shift"),
        (spec.mods.super_key, "logo"),
    ];
    for (on, name) in mods {
        if on {
            args.extend(["-M".to_owned(), name.to_owned()]);
        }
    }
    let key = match spec.key.as_str() {
        "enter" => "Return".to_owned(),
        "esc" => "Escape".to_owned(),
        "insert" => "Insert".to_owned(),
        "space" => "space".to_owned(),
        "tab" => "Tab".to_owned(),
        other => other.to_owned(),
    };
    args.extend(["-k".to_owned(), key]);
    for (on, name) in mods.iter().rev() {
        if *on {
            args.extend(["-m".to_owned(), (*name).to_owned()]);
        }
    }
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wtype_presses_then_releases_modifiers_in_reverse() {
        let spec: HotkeySpec = "ctrl+shift+v".parse().unwrap();
        assert_eq!(
            wtype_args(&spec),
            [
                "-M", "ctrl", "-M", "shift", "-k", "v", "-m", "shift", "-m", "ctrl"
            ]
        );
    }

    #[test]
    fn wtype_named_key() {
        let spec: HotkeySpec = "shift+insert".parse().unwrap();
        assert_eq!(
            wtype_args(&spec),
            ["-M", "shift", "-k", "Insert", "-m", "shift"]
        );
    }
}
