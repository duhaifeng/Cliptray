//! Linux 状态栏图标（StatusNotifierItem）。GNOME 需要 AppIndicator 扩展，KDE 和 wlroots 直接支持。

use std::sync::mpsc::Sender;

use ksni::blocking::TrayMethods;
use ksni::menu::StandardItem;

use super::{MenuAction, StatusIcon, TrayControl, TrayLabels};

pub fn start(status: StatusIcon) {
    let _ = std::thread::Builder::new()
        .name("status-item".into())
        .spawn(move || {
            if let Err(e) = drive(status) {
                eprintln!("状态栏图标不可用: {e:#}");
            }
        });
}

fn drive(status: StatusIcon) -> anyhow::Result<()> {
    let mut labels = status.labels;
    let actions = status.io.actions;
    let mut handle = if status.visible {
        Some(spawn(&actions, &labels)?)
    } else {
        None
    };
    while let Ok(ctrl) = status.io.control.recv() {
        match ctrl {
            TrayControl::Visible(true) if handle.is_none() => match spawn(&actions, &labels) {
                Ok(next) => handle = Some(next),
                Err(e) => eprintln!("状态栏图标创建失败: {e:#}"),
            },
            TrayControl::Visible(false) => {
                if let Some(current) = handle.take() {
                    current.shutdown().wait();
                }
            }
            TrayControl::Relabel(next) => {
                labels = next;
                if let Some(current) = &handle {
                    let labels = labels.clone();
                    current.update(|tray| tray.labels = labels);
                }
            }
            TrayControl::Visible(true) => {}
        }
    }
    Ok(())
}

/// 状态栏图标的颜色：GNOME 顶栏是深色，自带图标都是浅灰白色，这样和它们一致。
const PANEL_ICON_RGB: [u8; 3] = [0xEE, 0xEE, 0xEE];

struct BoardTray {
    actions: Sender<MenuAction>,
    labels: TrayLabels,
    icon: ksni::Icon,
}

impl BoardTray {
    fn new(actions: Sender<MenuAction>, labels: TrayLabels) -> Self {
        let (w, h, rgba) = crate::statusicon::clipboard_template_rgba();
        let colored = crate::statusicon::colorize(&rgba, PANEL_ICON_RGB);
        let icon = ksni::Icon {
            width: w as i32,
            height: h as i32,
            data: crate::statusicon::rgba_to_argb(&colored),
        };
        BoardTray {
            actions,
            labels,
            icon,
        }
    }
}

impl ksni::Tray for BoardTray {
    fn id(&self) -> String {
        "cliptray".into()
    }

    fn title(&self) -> String {
        "Cliptray".into()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        vec![self.icon.clone()]
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        let item = |label: String, action: MenuAction| {
            StandardItem {
                label,
                activate: Box::new(move |this: &mut Self| {
                    let _ = this.actions.send(action);
                }),
                ..Default::default()
            }
            .into()
        };
        vec![
            item(self.labels.setting.clone(), MenuAction::Settings),
            item(self.labels.clipboard.clone(), MenuAction::Toggle),
            ksni::MenuItem::Separator,
            item(self.labels.quit.clone(), MenuAction::Quit),
        ]
    }
}

fn spawn(
    actions: &Sender<MenuAction>,
    labels: &TrayLabels,
) -> anyhow::Result<ksni::blocking::Handle<BoardTray>> {
    BoardTray::new(actions.clone(), labels.clone())
        .spawn()
        .map_err(|e| anyhow::anyhow!("{e}"))
}
