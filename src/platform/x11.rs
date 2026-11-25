//! X11 实现：全局热键（XGrabKey）、前台应用（_NET_ACTIVE_WINDOW + WM_CLASS）、
//! 鼠标位置、XTest 合成按键。

use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use x11rb::connection::Connection;
use x11rb::protocol::Event;
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as _, GrabMode, KeyButMask};
use x11rb::protocol::xtest::ConnectionExt as _;
use x11rb::rust_connection::RustConnection;

use crate::hotkey::{HotkeySpec, Mods};
use crate::monitor::ActiveApp;

const MOD_SHIFT: u16 = 1 << 0;
const MOD_LOCK: u16 = 1 << 1; // CapsLock
const MOD_CTRL: u16 = 1 << 2;
const MOD_ALT: u16 = 1 << 3; // Mod1
const MOD_NUM: u16 = 1 << 4; // Mod2 (NumLock)
const MOD_SUPER: u16 = 1 << 6; // Mod4

const KEYSYM_SHIFT_L: u32 = 0xffe1;
const KEYSYM_CTRL_L: u32 = 0xffe3;
const KEYSYM_ALT_L: u32 = 0xffe9;
const KEYSYM_SUPER_L: u32 = 0xffeb;

fn mod_mask(m: Mods) -> u16 {
    (if m.shift { MOD_SHIFT } else { 0 })
        | (if m.ctrl { MOD_CTRL } else { 0 })
        | (if m.alt { MOD_ALT } else { 0 })
        | (if m.super_key { MOD_SUPER } else { 0 })
}

fn connect() -> Result<(RustConnection, u32)> {
    let (conn, screen) = x11rb::connect(None)
        .context("无法连接 X11 显示（DISPLAY 未设置，或当前是纯 Wayland 会话）")?;
    let root = conn.setup().roots[screen].root;
    Ok((conn, root))
}

/// 把 keysym 映射到 keycode。优先匹配未按 Shift 的列，找不到再看 Shift 列。
fn keycode_for(conn: &RustConnection, keysym: u32) -> Result<u8> {
    let setup = conn.setup();
    let (min, max) = (setup.min_keycode, setup.max_keycode);
    let map = conn.get_keyboard_mapping(min, max - min + 1)?.reply()?;
    let per = map.keysyms_per_keycode as usize;
    let mut shifted: Option<u8> = None;
    for (i, syms) in map.keysyms.chunks(per).enumerate() {
        let code = min + i as u8;
        if syms.first() == Some(&keysym) {
            return Ok(code);
        }
        if shifted.is_none() && syms.contains(&keysym) {
            shifted = Some(code);
        }
    }
    shifted.ok_or_else(|| anyhow!("当前键盘布局里找不到 keysym 0x{keysym:x}"))
}

/// 抓取热键（CapsLock / NumLock 开启时修饰键状态会变化，四种组合都要抓），返回主键的 keycode。
fn grab_hotkey(conn: &RustConnection, root: u32, spec: &HotkeySpec) -> Result<u8> {
    let keycode = keycode_for(conn, spec.keysym())?;
    let base = mod_mask(spec.mods);
    for extra in [0, MOD_LOCK, MOD_NUM, MOD_LOCK | MOD_NUM] {
        let grabbed = conn
            .grab_key(
                true,
                root,
                (base | extra).into(),
                keycode,
                GrabMode::ASYNC,
                GrabMode::ASYNC,
            )
            .map_err(anyhow::Error::from)
            .and_then(|c| c.check().map_err(anyhow::Error::from));
        if let Err(e) = grabbed {
            // 前面已经抓到的组合要放掉，不然换不回去。
            ungrab_hotkey(conn, root, keycode, spec);
            return Err(anyhow!(
                "抓取全局热键 {spec} 失败（可能已被桌面环境或其他程序占用）: {e}"
            ));
        }
    }
    conn.flush()?;
    Ok(keycode)
}

fn ungrab_hotkey(conn: &RustConnection, root: u32, keycode: u8, spec: &HotkeySpec) {
    let base = mod_mask(spec.mods);
    for extra in [0, MOD_LOCK, MOD_NUM, MOD_LOCK | MOD_NUM] {
        let _ = conn.ungrab_key(keycode, root, (base | extra).into());
    }
    let _ = conn.flush();
}

/// 换成新热键；新的抓不到就把旧的抓回来，保证出错时旧热键仍然可用。
fn regrab(
    conn: &RustConnection,
    root: u32,
    keycode: &mut u8,
    current: &mut HotkeySpec,
    new: HotkeySpec,
) -> Result<()> {
    ungrab_hotkey(conn, root, *keycode, current);
    match grab_hotkey(conn, root, &new) {
        Ok(code) => {
            *keycode = code;
            *current = new;
            Ok(())
        }
        Err(e) => {
            if let Ok(code) = grab_hotkey(conn, root, current) {
                *keycode = code;
            }
            Err(e)
        }
    }
}

/// 换热键的请求：新热键，加一个回复通道。
pub type Rebind = (HotkeySpec, Sender<Result<()>>);

/// 在独立线程里抓取全局热键，触发时向 `tx` 发送一个 `()`。
/// 抓取失败（例如该组合已被桌面环境占用）会同步返回错误，而不是让线程悄悄退出。
/// 返回的通道用来在不重启的情况下换热键。
pub fn spawn_hotkey_listener(spec: &HotkeySpec, tx: Sender<()>) -> Result<Sender<Rebind>> {
    use std::os::fd::AsRawFd;

    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<()>>();
    let (cmd_tx, cmd_rx) = std::sync::mpsc::channel::<Rebind>();
    let mut current = spec.clone();
    std::thread::Builder::new()
        .name("hotkey".into())
        .spawn(move || {
            let setup = || -> Result<(RustConnection, u32, u8)> {
                let (conn, root) = connect()?;
                let keycode = grab_hotkey(&conn, root, &current)?;
                Ok((conn, root, keycode))
            };
            let (conn, root, mut keycode) = match setup() {
                Ok(v) => {
                    let _ = ready_tx.send(Ok(()));
                    v
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                    return;
                }
            };
            let fd = conn.stream().as_raw_fd();
            let mut last = Instant::now() - Duration::from_secs(1);
            loop {
                while let Ok((new, reply)) = cmd_rx.try_recv() {
                    let _ = reply.send(regrab(&conn, root, &mut keycode, &mut current, new));
                }
                loop {
                    match conn.poll_for_event() {
                        Ok(Some(Event::KeyPress(ev))) if ev.detail == keycode => {
                            // 按住不放会产生自动重复的 KeyPress，做个去抖。
                            if last.elapsed() > Duration::from_millis(300) {
                                last = Instant::now();
                                if tx.send(()).is_err() {
                                    return;
                                }
                            }
                        }
                        Ok(Some(_)) => {}
                        Ok(None) => break,
                        Err(e) => {
                            eprintln!("X11 热键连接断开: {e}");
                            return;
                        }
                    }
                }
                // 最多等 100ms：有 X 事件立刻醒，没有就回头看看有没有换热键的请求。
                let mut pfd = libc::pollfd {
                    fd,
                    events: libc::POLLIN,
                    revents: 0,
                };
                // SAFETY: pfd 是有效的单个 pollfd，fd 在 conn 存活期间有效。
                unsafe { libc::poll(&mut pfd, 1, 100) };
            }
        })?;
    ready_rx.recv().context("热键线程意外退出")??;
    Ok(cmd_tx)
}

pub struct X11ActiveApp {
    conn: RustConnection,
    root: u32,
    net_active_window: u32,
}

impl X11ActiveApp {
    pub fn new() -> Result<Self> {
        let (conn, root) = connect()?;
        let net_active_window = conn
            .intern_atom(false, b"_NET_ACTIVE_WINDOW")?
            .reply()?
            .atom;
        Ok(X11ActiveApp {
            conn,
            root,
            net_active_window,
        })
    }

    fn query(&self) -> Result<Option<String>> {
        let r = self
            .conn
            .get_property(
                false,
                self.root,
                self.net_active_window,
                AtomEnum::WINDOW,
                0,
                1,
            )?
            .reply()?;
        let Some(win) = r.value32().and_then(|mut v| v.next()).filter(|w| *w != 0) else {
            return Ok(None);
        };
        let class = self
            .conn
            .get_property(false, win, AtomEnum::WM_CLASS, AtomEnum::STRING, 0, 256)?
            .reply()?;
        // WM_CLASS 是 "instance\0class\0"。取 class 部分，与桌面环境显示的应用名一致。
        let mut parts = class.value.split(|b| *b == 0).filter(|p| !p.is_empty());
        let (instance, class) = (parts.next(), parts.next());
        Ok(class
            .or(instance)
            .map(|b| String::from_utf8_lossy(b).into_owned()))
    }
}

impl ActiveApp for X11ActiveApp {
    fn current(&mut self) -> Option<String> {
        self.query().ok().flatten()
    }
}

/// 根窗口尺寸（物理像素；多显示器时是所有显示器拼起来的范围）。
pub fn screen_size() -> Option<(u32, u32)> {
    let (conn, root) = connect().ok()?;
    let screen = conn.setup().roots.iter().find(|s| s.root == root)?;
    Some((
        screen.width_in_pixels as u32,
        screen.height_in_pixels as u32,
    ))
}

/// 工作区（去掉顶栏、Dock 等占位）相对根窗口四边的内缩，物理像素：(左, 右, 上, 下)。
/// 读 `_NET_WORKAREA` 的第一个桌面；窗口管理器没有提供时返回 `None`。
pub fn work_area_insets() -> Option<(u32, u32, u32, u32)> {
    let (conn, root) = connect().ok()?;
    let atom = conn
        .intern_atom(false, b"_NET_WORKAREA")
        .ok()?
        .reply()
        .ok()?
        .atom;
    let reply = conn
        .get_property(false, root, atom, AtomEnum::CARDINAL, 0, 4)
        .ok()?
        .reply()
        .ok()?;
    let v: Vec<u32> = reply.value32()?.collect();
    let [x, y, w, h] = v.get(..4)?.try_into().ok()?;
    let (sw, sh) = screen_size()?;
    Some((
        x.min(sw),
        sw.saturating_sub(x + w),
        y.min(sh),
        sh.saturating_sub(y + h),
    ))
}

/// 等待物理修饰键全部松开，最多 `timeout`。
/// 否则用户还按着 Alt（数字键快速粘贴）时，合成的 Ctrl+V 会变成 Alt+Ctrl+V。
fn wait_modifiers_released(conn: &RustConnection, root: u32, timeout: Duration) {
    let held = KeyButMask::SHIFT | KeyButMask::CONTROL | KeyButMask::MOD1 | KeyButMask::MOD4;
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        match conn.query_pointer(root).ok().and_then(|c| c.reply().ok()) {
            Some(p) if p.mask & held != KeyButMask::from(0u16) => {
                std::thread::sleep(Duration::from_millis(15))
            }
            _ => return,
        }
    }
}

/// 用 XTest 向当前焦点窗口发送一次按键组合。
pub fn send_key(spec: &HotkeySpec) -> Result<()> {
    let (conn, root) = connect()?;
    if !conn.query_extension(b"XTEST")?.reply()?.present {
        bail!("X 服务器不支持 XTEST 扩展，无法模拟粘贴");
    }
    wait_modifiers_released(&conn, root, Duration::from_millis(1500));

    let mut mods: Vec<u8> = Vec::new();
    for (on, sym) in [
        (spec.mods.ctrl, KEYSYM_CTRL_L),
        (spec.mods.alt, KEYSYM_ALT_L),
        (spec.mods.shift, KEYSYM_SHIFT_L),
        (spec.mods.super_key, KEYSYM_SUPER_L),
    ] {
        if on {
            mods.push(keycode_for(&conn, sym)?);
        }
    }
    let key = keycode_for(&conn, spec.keysym())?;

    const KEY_PRESS: u8 = 2;
    const KEY_RELEASE: u8 = 3;
    let fake = |ty: u8, code: u8| -> Result<()> {
        conn.xtest_fake_input(ty, code, 0, root, 0, 0, 0)?;
        Ok(())
    };
    for m in &mods {
        fake(KEY_PRESS, *m)?;
    }
    fake(KEY_PRESS, key)?;
    fake(KEY_RELEASE, key)?;
    for m in mods.iter().rev() {
        fake(KEY_RELEASE, *m)?;
    }
    conn.flush()?;
    // 往返一次，确保事件在连接关闭前已被服务器处理。
    conn.get_input_focus()?.reply()?;
    Ok(())
}
