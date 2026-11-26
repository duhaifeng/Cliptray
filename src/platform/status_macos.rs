//! macOS 菜单栏图标。NSStatusItem 必须在主线程创建，菜单点击经 AppKit 跑起来之后才能收到。

use std::sync::mpsc::{Receiver, Sender, TryRecvError};

use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

use super::{MenuAction, StatusIcon, TrayControl, TrayLabels};

pub struct Session {
    actions: Sender<MenuAction>,
    control: Receiver<TrayControl>,
    labels: TrayLabels,
    icon: Option<TrayIcon>,
    ids: MenuIds,
}

struct MenuIds {
    setting: MenuId,
    clipboard: MenuId,
    quit: MenuId,
}

impl Session {
    pub fn new(status: StatusIcon) -> Self {
        accessory_policy();
        let mut session = Session {
            actions: status.io.actions,
            control: status.io.control,
            labels: status.labels,
            icon: None,
            ids: MenuIds {
                setting: MenuId::new("setting"),
                clipboard: MenuId::new("clipboard"),
                quit: MenuId::new("quit"),
            },
        };
        if status.visible {
            session.show();
        }
        session
    }

    pub fn poll(&mut self) {
        drain_app_events();
        loop {
            match self.control.try_recv() {
                Ok(TrayControl::Visible(true)) => {
                    if self.icon.is_none() {
                        self.show();
                    }
                }
                Ok(TrayControl::Visible(false)) => {
                    self.icon = None;
                }
                Ok(TrayControl::Relabel(labels)) => {
                    self.labels = labels;
                    if self.icon.is_some() {
                        self.show();
                    }
                }
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
            }
        }
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            let action = if event.id == self.ids.setting {
                Some(MenuAction::Settings)
            } else if event.id == self.ids.clipboard {
                Some(MenuAction::Toggle)
            } else if event.id == self.ids.quit {
                Some(MenuAction::Quit)
            } else {
                None
            };
            if let Some(action) = action {
                let _ = self.actions.send(action);
            }
        }
    }

    fn show(&mut self) {
        match build(&self.labels) {
            Ok((icon, ids)) => {
                self.ids = ids;
                self.icon = Some(icon);
                eprintln!("菜单栏图标已显示");
            }
            Err(e) => eprintln!("菜单栏图标创建失败: {e}"),
        }
    }
}

fn build(labels: &TrayLabels) -> Result<(TrayIcon, MenuIds), String> {
    let (w, h, rgba) = crate::statusicon::clipboard_template_rgba();
    let icon = Icon::from_rgba(rgba, w, h).map_err(|e| e.to_string())?;
    let menu = Menu::new();
    let setting = MenuItem::with_id("setting", labels.setting.clone(), true, None);
    let clipboard = MenuItem::with_id("clipboard", labels.clipboard.clone(), true, None);
    let quit = MenuItem::with_id("quit", labels.quit.clone(), true, None);
    let ids = MenuIds {
        setting: setting.id().clone(),
        clipboard: clipboard.id().clone(),
        quit: quit.id().clone(),
    };
    menu.append(&setting).map_err(|e| e.to_string())?;
    menu.append(&clipboard).map_err(|e| e.to_string())?;
    menu.append(&PredefinedMenuItem::separator())
        .map_err(|e| e.to_string())?;
    menu.append(&quit).map_err(|e| e.to_string())?;
    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip("Cliptray")
        .with_icon(icon)
        .with_icon_as_template(true)
        .with_menu_on_left_click(true)
        .build()
        .map_err(|e| e.to_string())?;
    Ok((tray, ids))
}

fn drain_app_events() {
    #[link(name = "AppKit", kind = "framework")]
    unsafe extern "C" {
        static NSDefaultRunLoopMode: *mut std::ffi::c_void;
    }
    #[link(name = "objc")]
    unsafe extern "C" {
        fn objc_getClass(name: *const std::ffi::c_char) -> *mut std::ffi::c_void;
        fn sel_registerName(name: *const std::ffi::c_char) -> *mut std::ffi::c_void;
        fn objc_msgSend();
    }
    unsafe {
        let cls = objc_getClass(c"NSApplication".as_ptr());
        let date_cls = objc_getClass(c"NSDate".as_ptr());
        if cls.is_null() || date_cls.is_null() {
            return;
        }
        type Msg0 = unsafe extern "C" fn(
            *mut std::ffi::c_void,
            *mut std::ffi::c_void,
        ) -> *mut std::ffi::c_void;
        type Next = unsafe extern "C" fn(
            *mut std::ffi::c_void,
            *mut std::ffi::c_void,
            u64,
            *mut std::ffi::c_void,
            *mut std::ffi::c_void,
            u8,
        ) -> *mut std::ffi::c_void;
        type Send = unsafe extern "C" fn(
            *mut std::ffi::c_void,
            *mut std::ffi::c_void,
            *mut std::ffi::c_void,
        );
        let msg0: Msg0 = std::mem::transmute(objc_msgSend as *const ());
        let next: Next = std::mem::transmute(objc_msgSend as *const ());
        let send: Send = std::mem::transmute(objc_msgSend as *const ());
        let sel = |name| sel_registerName(name);
        let app = msg0(cls, sel(c"sharedApplication".as_ptr()));
        let past = msg0(date_cls, sel(c"distantPast".as_ptr()));
        // NSEventMaskAny。只取已经排队的事件，不阻塞。
        for _ in 0..32 {
            let event = next(
                app,
                sel(c"nextEventMatchingMask:untilDate:inMode:dequeue:".as_ptr()),
                u64::MAX,
                past,
                NSDefaultRunLoopMode,
                1,
            );
            if event.is_null() {
                break;
            }
            send(app, sel(c"sendEvent:".as_ptr()), event);
        }
    }
}

fn accessory_policy() {
    #[link(name = "objc")]
    unsafe extern "C" {
        fn objc_getClass(name: *const std::ffi::c_char) -> *mut std::ffi::c_void;
        fn sel_registerName(name: *const std::ffi::c_char) -> *mut std::ffi::c_void;
        fn objc_msgSend();
    }
    unsafe {
        let cls = objc_getClass(c"NSApplication".as_ptr());
        if cls.is_null() {
            return;
        }
        type Msg0 = unsafe extern "C" fn(
            *mut std::ffi::c_void,
            *mut std::ffi::c_void,
        ) -> *mut std::ffi::c_void;
        type SetPolicy = unsafe extern "C" fn(*mut std::ffi::c_void, *mut std::ffi::c_void, isize);
        let shared: Msg0 = std::mem::transmute(objc_msgSend as *const ());
        let set: SetPolicy = std::mem::transmute(objc_msgSend as *const ());
        let app = shared(cls, sel_registerName(c"sharedApplication".as_ptr()));
        // NSApplicationActivationPolicyAccessory：有菜单栏图标，但不进 Dock。
        set(app, sel_registerName(c"setActivationPolicy:".as_ptr()), 1);
        let finish: Msg0 = std::mem::transmute(objc_msgSend as *const ());
        let _ = finish(app, sel_registerName(c"finishLaunching".as_ptr()));
    }
}
