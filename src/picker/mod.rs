//! 历史选择面板：一个短命的 egui 窗口，选完把结果交还给调用方（守护进程）。
//!
//! 面板仿照 iCopy 的横向剪切板：屏幕底部、与屏幕同宽、高 350 点的圆角条。

mod app;
mod fonts;
pub mod state;
mod theme;

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use anyhow::{Result, anyhow};
use eframe::egui;

use crate::config::Config;
use crate::ipc::Pick;
use crate::model::Tag;
use crate::platform;
use crate::store::Store;

use self::state::PickerState;

pub use self::fonts::{find_cjk_font, install_system_fonts};

/// 面板外框（逻辑点）：贴屏幕底边、与屏幕同宽。
pub fn board_frame(screen: (f32, f32)) -> (egui::Pos2, egui::Vec2) {
    board_frame_inset(screen, (0.0, 0.0, 0.0))
}

/// 和 [`board_frame`] 一样，但避开左、右、下三边被 Dock 占掉的位置，免得第一张卡片被盖住。
pub fn board_frame_inset(
    screen: (f32, f32),
    (left, right, bottom): (f32, f32, f32),
) -> (egui::Pos2, egui::Vec2) {
    let w = (screen.0 - left - right).max(1.0);
    let avail_h = (screen.1 - bottom).max(1.0);
    let h = theme::BOARD_H.min(avail_h);
    (egui::pos2(left, avail_h - h), egui::vec2(w, h))
}

fn last_tag_file(data_dir: &Path) -> PathBuf {
    data_dir.join("last_tag")
}

pub fn load_last_tag(data_dir: &Path) -> Option<Tag> {
    Tag::parse(
        std::fs::read_to_string(last_tag_file(data_dir))
            .ok()?
            .trim(),
    )
}

pub fn save_last_tag(data_dir: &Path, tag: Tag) {
    let _ = std::fs::write(last_tag_file(data_dir), tag.as_str());
}

/// 面板需要的外部位置：数据目录（图标、上次分类）与配置文件（设置按钮）。
pub struct Locations {
    pub data_dir: PathBuf,
    pub config_file: PathBuf,
}

/// 打开面板并等待用户选择。`None` 表示取消（Esc / 失去焦点 / 关闭）。
pub fn run(cfg: Config, store: Store, data_dir: &Path, config_file: &Path) -> Result<Option<Pick>> {
    // 拿不到屏幕尺寸（Wayland）时先给一个常见尺寸，第一帧再按 egui 报告的显示器尺寸调整。
    let screen = platform::main_screen_size().unwrap_or((1440.0, 900.0));
    let insets = platform::work_area_insets().map_or((0.0, 0.0, 0.0), |(l, r, _, b)| (l, r, b));
    let (pos, size) = board_frame_inset(screen, insets);
    let viewport = egui::ViewportBuilder::default()
        .with_title("splay")
        .with_app_id("splay")
        .with_position(pos)
        .with_inner_size(size)
        .with_decorations(false)
        .with_transparent(true)
        .with_resizable(false)
        .with_always_on_top()
        .with_taskbar(false);
    let options = eframe::NativeOptions {
        viewport,
        event_loop_builder: Some(Box::new(|_builder| {
            // 不出现在 Dock 和 ⌘Tab 里，与 iCopy（LSUIElement）一致。
            #[cfg(target_os = "macos")]
            {
                use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS};
                _builder.with_activation_policy(ActivationPolicy::Accessory);
            }
        })),
        ..Default::default()
    };

    let tag = cfg.opening_tag(load_last_tag(data_dir));
    let st = PickerState::new(store, cfg.clone(), tag)?;
    let result: Rc<RefCell<Option<Pick>>> = Rc::new(RefCell::new(None));
    let out = result.clone();
    let locations = Locations {
        data_dir: data_dir.to_owned(),
        config_file: config_file.to_owned(),
    };
    eframe::run_native(
        "splay",
        options,
        Box::new(move |cc| {
            #[cfg(target_os = "macos")]
            raise_above_dock(cc);
            Ok(Box::new(app::PickerApp::new(
                &cc.egui_ctx,
                st,
                cfg,
                locations,
                out,
            )))
        }),
    )
    .map_err(|e| anyhow!("无法打开选择面板: {e}"))?;

    let pick = result.borrow_mut().take();
    Ok(pick)
}

/// winit 的“置顶”只是浮动层级，低于 Dock；iCopy 的面板在 Dock 之上。
#[cfg(target_os = "macos")]
fn raise_above_dock(cc: &eframe::CreationContext) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    if let Ok(handle) = cc.window_handle()
        && let RawWindowHandle::AppKit(h) = handle.as_raw()
    {
        // SAFETY: 创建回调在主线程上运行，句柄指向 eframe 刚创建的窗口视图。
        unsafe { platform::raise_above_dock(h.ns_view.as_ptr()) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn board_sits_on_the_bottom_edge_at_full_width() {
        let (pos, size) = board_frame((1728.0, 1117.0));
        assert_eq!(size, egui::vec2(1728.0, 350.0));
        assert_eq!(
            pos,
            egui::pos2(0.0, 767.0),
            "与 iCopy 在 1728×1117 屏幕上的实测位置一致"
        );
    }

    #[test]
    fn board_keeps_clear_of_a_side_dock() {
        let (pos, size) = board_frame_inset((2560.0, 1440.0), (67.0, 0.0, 0.0));
        assert_eq!(pos, egui::pos2(67.0, 1090.0));
        assert_eq!(size, egui::vec2(2493.0, 350.0));
        let (pos, _) = board_frame_inset((2560.0, 1440.0), (0.0, 0.0, 40.0));
        assert_eq!(pos.y, 1050.0, "底部有 Dock 时面板坐在它上面");
    }

    #[test]
    fn board_never_exceeds_a_short_screen() {
        let (pos, size) = board_frame((800.0, 300.0));
        assert_eq!((pos.y, size.y), (0.0, 300.0));
    }

    #[test]
    fn last_tag_roundtrip_and_garbage() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(load_last_tag(d.path()), None);
        save_last_tag(d.path(), Tag::Image);
        assert_eq!(load_last_tag(d.path()), Some(Tag::Image));
        std::fs::write(last_tag_file(d.path()), "nonsense").unwrap();
        assert_eq!(load_last_tag(d.path()), None);
    }
}
