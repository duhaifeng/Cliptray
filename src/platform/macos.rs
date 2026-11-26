//! macOS 平台能力。
//!
//! - 全局热键：Carbon `RegisterEventHotKey`，不需要「辅助功能」权限。
//! - 模拟粘贴：CGEvent，需要「辅助功能」授权（授权给启动守护进程的程序）。
//! - 前台应用与图标：系统自带的 `lsappinfo` / `PlistBuddy` / `sips`，只在剪切板变化时调用。
//!
//! 键码是 ANSI 美式键盘的物理键位：字母键和键帽上的字母一致，中文输入法不改变这些键位。
//! 其他键位排列（例如 AZERTY）上，字母键可能对不上。

use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::Sender;

use anyhow::{Context, Result, anyhow};

use crate::hotkey::{HotkeySpec, Mods};
use crate::monitor::ActiveApp;

const CMD: u32 = 1 << 8;
const SHIFT: u32 = 1 << 9;
const OPTION: u32 = 1 << 11;
const CONTROL: u32 = 1 << 12;

/// `kEventClassKeyboard`
const EVENT_CLASS_KEYBOARD: u32 = u32::from_be_bytes(*b"keyb");
/// `kEventHotKeyPressed`
const EVENT_HOTKEY_PRESSED: u32 = 5;
/// `kEventHotKeyExclusive`：同一组合已由其他进程独占时，注册会失败而不是两边一起响应。
const HOTKEY_EXCLUSIVE: u32 = 1;
/// `eventHotKeyExistsErr`
const HOTKEY_EXISTS: i32 = -9878;
const SIGNATURE: u32 = u32::from_be_bytes(*b"cpbd");

pub(crate) fn carbon_modifiers(mods: Mods) -> u32 {
    (if mods.super_key { CMD } else { 0 })
        | (if mods.shift { SHIFT } else { 0 })
        | (if mods.alt { OPTION } else { 0 })
        | (if mods.ctrl { CONTROL } else { 0 })
}

/// 快捷键主键对应的 macOS 虚拟键码。`None` 表示这个名字在 macOS 上没有键位。
pub(crate) fn carbon_keycode(key: &str) -> Option<u32> {
    Some(match key {
        "a" => 0x00,
        "s" => 0x01,
        "d" => 0x02,
        "f" => 0x03,
        "h" => 0x04,
        "g" => 0x05,
        "z" => 0x06,
        "x" => 0x07,
        "c" => 0x08,
        "v" => 0x09,
        "b" => 0x0b,
        "q" => 0x0c,
        "w" => 0x0d,
        "e" => 0x0e,
        "r" => 0x0f,
        "y" => 0x10,
        "t" => 0x11,
        "1" => 0x12,
        "2" => 0x13,
        "3" => 0x14,
        "4" => 0x15,
        "6" => 0x16,
        "5" => 0x17,
        "=" => 0x18,
        "9" => 0x19,
        "7" => 0x1a,
        "-" => 0x1b,
        "8" => 0x1c,
        "0" => 0x1d,
        "]" => 0x1e,
        "o" => 0x1f,
        "u" => 0x20,
        "[" => 0x21,
        "i" => 0x22,
        "p" => 0x23,
        "l" => 0x25,
        "j" => 0x26,
        "'" => 0x27,
        "k" => 0x28,
        ";" => 0x29,
        "\\" => 0x2a,
        "," => 0x2b,
        "/" => 0x2c,
        "n" => 0x2d,
        "m" => 0x2e,
        "." => 0x2f,
        "`" => 0x32,
        "enter" => 0x24,
        "tab" => 0x30,
        "space" => 0x31,
        "backspace" => 0x33,
        "esc" => 0x35,
        "insert" => 0x72,
        "home" => 0x73,
        "pageup" => 0x74,
        "delete" => 0x75,
        "end" => 0x77,
        "pagedown" => 0x79,
        "left" => 0x7b,
        "right" => 0x7c,
        "down" => 0x7d,
        "up" => 0x7e,
        "f1" => 0x7a,
        "f2" => 0x78,
        "f3" => 0x63,
        "f4" => 0x76,
        "f5" => 0x60,
        "f6" => 0x61,
        "f7" => 0x62,
        "f8" => 0x64,
        "f9" => 0x65,
        "f10" => 0x6d,
        "f11" => 0x67,
        "f12" => 0x6f,
        _ => return None,
    })
}

type OSStatus = i32;
type EventRef = *mut c_void;
type EventHandlerCallRef = *mut c_void;
type EventHandlerRef = *mut c_void;
type EventHotKeyRef = *mut c_void;
type EventTargetRef = *mut c_void;
type EventHandlerUPP =
    Option<extern "C" fn(EventHandlerCallRef, EventRef, *mut c_void) -> OSStatus>;

#[repr(C)]
struct EventHotKeyID {
    signature: u32,
    id: u32,
}

#[repr(C)]
struct EventTypeSpec {
    event_class: u32,
    event_kind: u32,
}

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn RegisterEventHotKey(
        key_code: u32,
        modifiers: u32,
        hotkey_id: EventHotKeyID,
        target: EventTargetRef,
        options: u32,
        out_ref: *mut EventHotKeyRef,
    ) -> OSStatus;
    fn GetEventDispatcherTarget() -> EventTargetRef;
    /// `inNumTypes` 是 `unsigned long`，在 64 位 macOS 上是 8 字节。
    fn ReceiveNextEvent(
        num_types: u64,
        types: *const EventTypeSpec,
        timeout: f64,
        pull: u8,
        out_event: *mut EventRef,
    ) -> OSStatus;
    fn SendEventToEventTarget(event: EventRef, target: EventTargetRef) -> OSStatus;
    fn ReleaseEvent(event: EventRef);
    fn InstallEventHandler(
        target: EventTargetRef,
        handler: EventHandlerUPP,
        num_types: u64,
        types: *const EventTypeSpec,
        user_data: *mut c_void,
        out_ref: *mut EventHandlerRef,
    ) -> OSStatus;
}

/// `user` 是泄漏的 `Box<Sender<()>>`，和热键线程同寿命。
extern "C" fn on_hotkey(
    _call: EventHandlerCallRef,
    _event: EventRef,
    user: *mut c_void,
) -> OSStatus {
    let tx = unsafe { &*(user.cast::<Sender<()>>()) };
    let _ = tx.send(());
    0
}

/// 在调用线程上注册，并由该线程一直取事件。
/// `ReceiveNextEvent` 会跑当前线程的事件循环；只跑 `CFRunLoop` 收不到 Carbon 热键。
///
/// # Safety
/// 必须在即将调用 `ReceiveNextEvent` 的那个线程上调用。同一进程不要注册同一个组合两次。
unsafe fn register(keycode: u32, modifiers: u32, tx: Sender<()>) -> Result<()> {
    let target = unsafe { GetEventDispatcherTarget() };
    if target.is_null() {
        anyhow::bail!("GetEventDispatcherTarget 返回空");
    }
    let types = EventTypeSpec {
        event_class: EVENT_CLASS_KEYBOARD,
        event_kind: EVENT_HOTKEY_PRESSED,
    };
    // 先装回调再注册，避免两者之间的按键被丢掉。回调数据活到进程结束。
    let leaked = Box::into_raw(Box::new(tx));
    let status = unsafe {
        InstallEventHandler(
            target,
            Some(on_hotkey),
            1,
            &types,
            leaked.cast(),
            std::ptr::null_mut(),
        )
    };
    if status != 0 {
        unsafe { drop(Box::from_raw(leaked)) };
        anyhow::bail!("InstallEventHandler 返回 {status}");
    }
    let mut hotkey: EventHotKeyRef = std::ptr::null_mut();
    let status = unsafe {
        RegisterEventHotKey(
            keycode,
            modifiers,
            EventHotKeyID {
                signature: SIGNATURE,
                id: 1,
            },
            target,
            HOTKEY_EXCLUSIVE,
            &mut hotkey,
        )
    };
    if status == HOTKEY_EXISTS {
        anyhow::bail!(
            "该组合已被占用（本进程重复注册，或别的程序以独占方式注册了同一个快捷键，例如正在运行的 iCopy）"
        );
    }
    if status != 0 {
        anyhow::bail!("RegisterEventHotKey 返回 {status}");
    }
    if hotkey.is_null() {
        anyhow::bail!("RegisterEventHotKey 没有返回热键引用");
    }
    Ok(())
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kCFRunLoopDefaultMode: *const c_void;
    fn CFRunLoopRunInMode(mode: *const c_void, seconds: f64, return_after_source: u8) -> i32;
}

/// `eventLoopTimedOutErr`
const EVENT_LOOP_TIMED_OUT: i32 = -9875;
/// `eventLoopQuitErr`
const EVENT_LOOP_QUIT: i32 = -9876;

/// 让 AppKit 处理状态栏图标的鼠标事件。Carbon 的 `ReceiveNextEvent` 不会把菜单点开。
fn pump_appkit() {
    unsafe {
        // 跑满这一小段，而不是处理完一个源就返回，否则状态栏图标注册不完。
        CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.05, 0);
    }
}

/// 在调用线程上注册热键并分发 Carbon 事件，`body` 在工作线程上运行；`body` 返回后结束分发并返回其结果。
///
/// 调用线程必须是主线程。没有菜单栏图标且热键注册失败时，`body` 直接在调用线程上运行。
/// 有菜单栏图标时即使热键注册失败也留在主线程上泵 AppKit，否则状态栏菜单点不开。
pub fn run_session<R: Send + 'static>(
    spec: &HotkeySpec,
    tx: Sender<()>,
    status: Option<super::StatusIcon>,
    body: impl FnOnce(Result<()>) -> R + Send + 'static,
) -> R {
    let registered = carbon_keycode(&spec.key)
        .ok_or_else(|| anyhow!("macOS 热键没有对应的键位: {}", spec.key))
        .and_then(|keycode| unsafe { register(keycode, carbon_modifiers(spec.mods), tx) })
        .with_context(|| format!("注册全局热键 {spec} 失败"));
    let hotkey_ok = registered.is_ok();
    if registered.is_err() && status.is_none() {
        return body(registered);
    }
    let worker = std::thread::Builder::new()
        .name("daemon".into())
        .spawn(move || body(registered))
        .expect("创建工作线程");
    let mut status = status.map(super::status_macos::Session::new);
    let types = EventTypeSpec {
        event_class: EVENT_CLASS_KEYBOARD,
        event_kind: EVENT_HOTKEY_PRESSED,
    };
    let timeout = if status.is_some() { 0.05 } else { 0.25 };
    while !worker.is_finished() {
        if hotkey_ok {
            let mut event: EventRef = std::ptr::null_mut();
            let status_code = unsafe { ReceiveNextEvent(1, &types, timeout, 1, &mut event) };
            if status_code == 0 && !event.is_null() {
                unsafe {
                    let _ = SendEventToEventTarget(event, GetEventDispatcherTarget());
                    ReleaseEvent(event);
                }
            } else if status_code != EVENT_LOOP_TIMED_OUT && status_code != 0 {
                // AppKit 自己的事件循环会发 eventLoopQuitErr（-9876）唤醒这里，它不代表要退出。
                // 以前遇到非零就 break，结果热键和状态栏菜单都没人处理了。只在工作线程结束时才退出。
                if status_code != EVENT_LOOP_QUIT {
                    eprintln!("热键事件循环返回 {status_code}，继续运行");
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        } else {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        if let Some(session) = &mut status {
            pump_appkit();
            session.poll();
        }
    }
    worker
        .join()
        .unwrap_or_else(|p| std::panic::resume_unwind(p))
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CGPoint {
    x: f64,
    y: f64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CGSize {
    width: f64,
    height: f64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CGRect {
    origin: CGPoint,
    size: CGSize,
}

const EVENT_FLAG_SHIFT: u64 = 0x0002_0000;
const EVENT_FLAG_CONTROL: u64 = 0x0004_0000;
const EVENT_FLAG_OPTION: u64 = 0x0008_0000;
const EVENT_FLAG_COMMAND: u64 = 0x0010_0000;
/// `kCGSessionEventTap`
const SESSION_EVENT_TAP: u32 = 1;

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGMainDisplayID() -> u32;
    fn CGDisplayBounds(display: u32) -> CGRect;
    fn CGEventCreateKeyboardEvent(
        source: *const c_void,
        keycode: u16,
        key_down: bool,
    ) -> *mut c_void;
    fn CGEventSetFlags(event: *mut c_void, flags: u64);
    fn CGEventPost(tap: u32, event: *mut c_void);
}

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> bool;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(cf: *const c_void);
}

#[link(name = "objc")]
unsafe extern "C" {
    fn sel_registerName(name: *const std::ffi::c_char) -> *mut c_void;
    fn objc_msgSend();
}

/// `NSStatusWindowLevel`。iCopy 面板实测在这一层，高于 Dock（20），所以 Dock 不会盖住卡片。
const STATUS_WINDOW_LEVEL: isize = 25;

/// 把 `ns_view` 所在窗口提到状态栏层级。
///
/// # Safety
/// `ns_view` 必须是主线程上仍然有效的 `NSView`。
pub unsafe fn raise_above_dock(ns_view: *mut c_void) {
    type GetWindow = unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void;
    type SetLevel = unsafe extern "C" fn(*mut c_void, *mut c_void, isize);
    unsafe {
        let get: GetWindow = std::mem::transmute(objc_msgSend as *const ());
        let set: SetLevel = std::mem::transmute(objc_msgSend as *const ());
        let window = get(ns_view, sel_registerName(c"window".as_ptr()));
        if !window.is_null() {
            set(
                window,
                sel_registerName(c"setLevel:".as_ptr()),
                STATUS_WINDOW_LEVEL,
            );
        }
    }
}

type CFTypeRef = *const c_void;

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFLocaleCopyPreferredLanguages() -> CFTypeRef;
    fn CFArrayGetCount(array: CFTypeRef) -> isize;
    fn CFArrayGetValueAtIndex(array: CFTypeRef, idx: isize) -> CFTypeRef;
    fn CFStringGetCString(
        s: CFTypeRef,
        buf: *mut std::ffi::c_char,
        size: isize,
        encoding: u32,
    ) -> bool;
}

const UTF8: u32 = 0x0800_0100;

/// 系统偏好语言列表的第一项，例如 `en-CN`、`zh-Hans-CN`（与 `defaults read -g AppleLanguages` 一致）。
pub fn preferred_language() -> Option<String> {
    unsafe {
        let langs = CFLocaleCopyPreferredLanguages();
        if langs.is_null() {
            return None;
        }
        let mut buf = [0 as std::ffi::c_char; 64];
        let first = (CFArrayGetCount(langs) > 0).then(|| CFArrayGetValueAtIndex(langs, 0));
        let ok = first
            .is_some_and(|s| CFStringGetCString(s, buf.as_mut_ptr(), buf.len() as isize, UTF8));
        CFRelease(langs);
        ok.then(|| {
            std::ffi::CStr::from_ptr(buf.as_ptr())
                .to_string_lossy()
                .into_owned()
        })
    }
}

/// 主显示器尺寸（逻辑点）。
pub fn main_screen_size() -> (f32, f32) {
    let r = unsafe { CGDisplayBounds(CGMainDisplayID()) };
    (r.size.width as f32, r.size.height as f32)
}

pub fn accessibility_trusted() -> bool {
    unsafe { AXIsProcessTrusted() }
}

pub(crate) fn event_flags(mods: Mods) -> u64 {
    (if mods.super_key {
        EVENT_FLAG_COMMAND
    } else {
        0
    }) | (if mods.shift { EVENT_FLAG_SHIFT } else { 0 })
        | (if mods.alt { EVENT_FLAG_OPTION } else { 0 })
        | (if mods.ctrl { EVENT_FLAG_CONTROL } else { 0 })
}

/// 向前台应用发送一次按键组合。修饰键直接写进事件标志，不受用户此刻按着的物理修饰键影响。
pub fn send_key(spec: &HotkeySpec) -> Result<()> {
    // 未授权时 CGEventPost 不报错，只是静默丢弃事件，所以要先检查。
    anyhow::ensure!(
        accessibility_trusted(),
        "没有「辅助功能」授权：请在「系统设置 → 隐私与安全性 → 辅助功能」里允许启动 Cliptray 守护进程的程序"
    );
    let keycode = carbon_keycode(&spec.key)
        .ok_or_else(|| anyhow!("macOS 没有对应的键位: {}", spec.key))? as u16;
    let flags = event_flags(spec.mods);
    for down in [true, false] {
        let event = unsafe { CGEventCreateKeyboardEvent(std::ptr::null(), keycode, down) };
        anyhow::ensure!(!event.is_null(), "CGEventCreateKeyboardEvent 失败");
        unsafe {
            CGEventSetFlags(event, flags);
            CGEventPost(SESSION_EVENT_TAP, event);
            CFRelease(event);
        }
    }
    Ok(())
}

/// 前台应用的显示名，例如 “Google Chrome”。
pub struct FrontApp;

impl ActiveApp for FrontApp {
    fn current(&mut self) -> Option<String> {
        let asn = run("lsappinfo", &["front"])?;
        let info = run("lsappinfo", &["info", "-only", "name", asn.trim()])?;
        parse_lsappinfo_name(&info)
    }
}

fn run(program: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(program).args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// `lsappinfo info` 的第一行形如 `"Finder" ASN:0x0-0x23023: (in front)`。
pub(crate) fn parse_lsappinfo_name(info: &str) -> Option<String> {
    let rest = info.trim_start().strip_prefix('"')?;
    let name = &rest[..rest.find('"')?];
    (!name.is_empty()).then(|| name.to_owned())
}

/// 从 `lsappinfo info -only bundlepath` 的输出里取 `bundle path="..."`。
pub(crate) fn parse_bundle_path(info: &str) -> Option<PathBuf> {
    let rest = &info[info.find("bundle path=\"")? + "bundle path=\"".len()..];
    Some(PathBuf::from(&rest[..rest.find('"')?]))
}

/// 从 `.app` 包里导出 64 点的 PNG 图标。`lsappinfo` 按名字查只对正在运行的应用有效，
/// 守护进程在刚记录到该应用的复制时调用，此时它就在前台。
pub fn export_app_icon(app: &str, out: &Path) -> Result<bool> {
    let Some(bundle) =
        run("lsappinfo", &["info", "-only", "bundlepath", app]).and_then(|s| parse_bundle_path(&s))
    else {
        return Ok(false);
    };
    let plist = bundle.join("Contents/Info.plist");
    let Some(name) = run(
        "/usr/libexec/PlistBuddy",
        &["-c", "Print :CFBundleIconFile", &plist.to_string_lossy()],
    ) else {
        // 只用 Assets.car 提供图标的应用没有 CFBundleIconFile，这里不解析资源目录。
        return Ok(false);
    };
    let name = name.trim();
    let file = if Path::new(name).extension().is_some() {
        name.to_owned()
    } else {
        format!("{name}.icns")
    };
    let icns = bundle.join("Contents/Resources").join(file);
    if !icns.is_file() {
        return Ok(false);
    }
    let status = Command::new("sips")
        .args(["-s", "format", "png", "-Z", "64"])
        .arg(&icns)
        .arg("--out")
        .arg(out)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .context("运行 sips 失败")?;
    anyhow::ensure!(status.success(), "sips 转换图标失败: {}", icns.display());
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lsappinfo_output_parsing() {
        let info = "\"Google Chrome\" ASN:0x0-0x1d01d: (in front) \n    bundleID=[ NULL ] \n";
        assert_eq!(parse_lsappinfo_name(info).as_deref(), Some("Google Chrome"));
        assert_eq!(parse_lsappinfo_name("[ NULL ]  ASN:0x0-0x1: "), None);
        let info = "[ NULL ]  ASN:0x0-0x23023: (in front) \n    bundle path=\"/System/Library/CoreServices/Finder.app\"\n";
        assert_eq!(
            parse_bundle_path(info),
            Some(PathBuf::from("/System/Library/CoreServices/Finder.app"))
        );
        assert_eq!(parse_bundle_path("bundle path=[ NULL ]"), None);
    }

    #[test]
    fn front_app_and_icon_export_work_on_this_machine() {
        let name = FrontApp.current().expect("lsappinfo 应能返回前台应用");
        let d = tempfile::tempdir().unwrap();
        let out = d.path().join("finder.png");
        // Finder 总在运行，而且用 icns 提供图标。
        assert!(
            export_app_icon("Finder", &out).unwrap(),
            "前台应用是 {name}"
        );
        let img = image::open(&out).unwrap();
        assert!(img.width() <= 64 && img.width() >= 32);
    }

    #[test]
    fn preferred_language_matches_defaults() {
        let out = Command::new("defaults")
            .args(["read", "-g", "AppleLanguages"])
            .output()
            .unwrap();
        let listed = String::from_utf8_lossy(&out.stdout);
        let first = listed.split('"').nth(1).expect("AppleLanguages 至少有一项");
        assert_eq!(preferred_language().as_deref(), Some(first));
    }

    #[test]
    fn paste_flags_match_modifiers() {
        assert_eq!(
            event_flags("super+v".parse::<HotkeySpec>().unwrap().mods),
            EVENT_FLAG_COMMAND
        );
        assert_eq!(
            event_flags("ctrl+shift+v".parse::<HotkeySpec>().unwrap().mods),
            EVENT_FLAG_CONTROL | EVENT_FLAG_SHIFT
        );
    }

    #[test]
    fn shift_cmd_c_is_the_c_key_with_shift_and_command() {
        let spec: HotkeySpec = "shift+cmd+c".parse().unwrap();
        assert_eq!(carbon_keycode(&spec.key), Some(0x08));
        assert_eq!(carbon_modifiers(spec.mods), SHIFT | CMD);
    }

    #[test]
    fn every_parsable_key_has_a_keycode() {
        let keys = [
            "a",
            "b",
            "c",
            "d",
            "e",
            "f",
            "g",
            "h",
            "i",
            "j",
            "k",
            "l",
            "m",
            "n",
            "o",
            "p",
            "q",
            "r",
            "s",
            "t",
            "u",
            "v",
            "w",
            "x",
            "y",
            "z",
            "0",
            "1",
            "2",
            "3",
            "4",
            "5",
            "6",
            "7",
            "8",
            "9",
            "space",
            "enter",
            "esc",
            "tab",
            "insert",
            "delete",
            "backspace",
            "up",
            "down",
            "left",
            "right",
            "home",
            "end",
            "pageup",
            "pagedown",
            "f1",
            "f2",
            "f3",
            "f4",
            "f5",
            "f6",
            "f7",
            "f8",
            "f9",
            "f10",
            "f11",
            "f12",
            "`",
            "-",
            "=",
            "[",
            "]",
            "\\",
            ";",
            "'",
            ",",
            ".",
            "/",
        ];
        for key in keys {
            let spec: HotkeySpec = format!("super+{key}")
                .parse()
                .unwrap_or_else(|e| panic!("{key}: {e}"));
            assert_eq!(spec.key, key);
            assert!(carbon_keycode(key).is_some(), "{key}");
        }
    }
}
