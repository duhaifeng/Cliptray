//! 设置窗口的外观：配色、徽标图标和自绘控件。
//!
//! 版式和颜色取自本机 iCopy 设置窗口的截图（2x 截图取色），结构对应 REA 读到的类：
//! `SettingViewController`（背景 + 分栏）、`MenuViewController`（毛玻璃侧栏、版本号、`MenuButton`）、
//! `ShortcutViewController` 的 `box1View`…`box4View`（分区卡片）、`PasteHistorySlider`（存储时间滑块）。
//! 图标是自画的几何图形，没有使用 iCopy `Assets.car` 里的像素。

use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, FontId, Layout, Painter, Rect, Response, RichText,
    Sense, Shape, Stroke, StrokeKind, Ui, Vec2, pos2, vec2,
};

pub const SIDEBAR_BG: Color32 = Color32::from_rgb(0x20, 0x1F, 0x1F);
pub const SIDEBAR_SELECTED: Color32 = Color32::from_rgb(0x36, 0x35, 0x35);
pub const CONTENT_BG: Color32 = Color32::from_rgb(0x27, 0x22, 0x23);
pub const CARD_BG: Color32 = Color32::from_rgb(0x21, 0x1D, 0x1E);
pub const CONTROL_BG: Color32 = Color32::from_rgb(0x32, 0x2E, 0x2F);
pub const CONTROL_HOVER: Color32 = Color32::from_rgb(0x3F, 0x3A, 0x3B);
pub const CHECK_BLUE: Color32 = Color32::from_rgb(0x06, 0x80, 0xFA);
pub const TICK_TEAL: Color32 = Color32::from_rgb(0x6C, 0xB5, 0xB0);
pub const PIN_BLUE: Color32 = Color32::from_rgb(0x1E, 0x9B, 0xD7);
pub const TEXT: Color32 = Color32::from_rgb(0xDE, 0xDD, 0xDD);
pub const TEXT_WEAK: Color32 = Color32::from_rgb(0x8E, 0x8A, 0x8B);
pub const ERROR: Color32 = Color32::from_rgb(0xFF, 0x6B, 0x6B);

/// 表单行里标签列的宽度。标签右对齐，控件从标签列右边接着排。
pub const LABEL_W: f32 = 170.0;
pub const SIDEBAR_W: f32 = 200.0;

/// 全局视觉：深色底、圆角控件、没有描边。
pub fn apply_visuals(ctx: &egui::Context) {
    let mut v = egui::Visuals::dark();
    v.panel_fill = CONTENT_BG;
    v.window_fill = Color32::from_rgb(0x2C, 0x28, 0x29);
    v.extreme_bg_color = CONTROL_BG;
    v.faint_bg_color = CARD_BG;
    v.window_corner_radius = CornerRadius::same(12);
    v.menu_corner_radius = CornerRadius::same(8);
    v.selection.bg_fill = CHECK_BLUE;
    v.selection.stroke = Stroke::new(1.0, Color32::WHITE);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
    let radius = CornerRadius::same(6);
    for (w, fill) in [
        (&mut v.widgets.inactive, CONTROL_BG),
        (&mut v.widgets.hovered, CONTROL_HOVER),
        (&mut v.widgets.active, CONTROL_HOVER),
        (&mut v.widgets.open, CONTROL_HOVER),
    ] {
        w.weak_bg_fill = fill;
        w.bg_fill = fill;
        w.bg_stroke = Stroke::NONE;
        w.corner_radius = radius;
        w.fg_stroke = Stroke::new(1.0, TEXT);
    }
    ctx.set_theme(egui::ThemePreference::Dark);
    ctx.set_visuals_of(egui::Theme::Dark, v);
    ctx.all_styles_mut(|s| {
        s.spacing.item_spacing = vec2(8.0, 8.0);
        s.spacing.button_padding = vec2(12.0, 5.0);
        s.spacing.interact_size.y = 28.0;
        s.spacing.combo_width = 0.0;
        s.text_styles
            .insert(egui::TextStyle::Body, FontId::proportional(14.0));
        s.text_styles
            .insert(egui::TextStyle::Button, FontId::proportional(14.0));
    });
}

#[derive(Clone, Copy, Debug)]
pub enum Glyph {
    Clipboard,
    Gear,
    Stop,
    Command,
    Sliders,
    Clock,
    Display,
}

/// 圆角方块底 + 白色线稿图标，对应 iCopy 侧栏和分区标题前的彩色图标。
pub fn badge(p: &Painter, rect: Rect, glyph: Glyph, color: Color32) {
    let r = (rect.width() * 0.26).round().clamp(2.0, 255.0) as u8;
    p.rect_filled(rect, CornerRadius::same(r), color);
    // 上半部提亮一点，做出渐变的感觉。
    let top = Rect::from_min_max(rect.min, pos2(rect.max.x, rect.center().y));
    p.rect_filled(
        top,
        CornerRadius {
            nw: r,
            ne: r,
            sw: 0,
            se: 0,
        },
        Color32::from_white_alpha(26),
    );
    draw_glyph(p, rect.shrink(rect.width() * 0.22), glyph);
}

fn draw_glyph(p: &Painter, g: Rect, glyph: Glyph) {
    let w = g.width();
    let line = Stroke::new((w * 0.12).max(1.3), Color32::WHITE);
    let c = g.center();
    match glyph {
        Glyph::Clipboard => {
            let body = Rect::from_min_max(pos2(g.left(), g.top() + w * 0.14), g.max);
            p.rect_stroke(body, CornerRadius::same(2), line, StrokeKind::Inside);
            let clip =
                Rect::from_center_size(pos2(c.x, g.top() + w * 0.14), vec2(w * 0.46, w * 0.26));
            p.rect_filled(clip, CornerRadius::same(2), Color32::WHITE);
        }
        Glyph::Gear => {
            p.circle_stroke(c, w * 0.26, line);
            for i in 0..8 {
                let a = i as f32 * std::f32::consts::TAU / 8.0;
                let dir = vec2(a.cos(), a.sin());
                p.line_segment([c + dir * (w * 0.34), c + dir * (w * 0.5)], line);
            }
            p.circle_filled(c, w * 0.08, Color32::WHITE);
        }
        Glyph::Stop => {
            p.circle_stroke(c, w * 0.46, line);
            let d = w * 0.32;
            p.line_segment([c + vec2(-d, d), c + vec2(d, -d)], line);
        }
        Glyph::Command => {
            let s = w * 0.17;
            let sq = Rect::from_center_size(c, vec2(s * 2.0, s * 2.0));
            p.rect_stroke(sq, CornerRadius::ZERO, line, StrokeKind::Middle);
            for (sx, sy) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                p.circle_stroke(
                    c + vec2(sx * s, sy * s) * 1.0 + vec2(sx, sy) * (w * 0.09),
                    w * 0.1,
                    line,
                );
            }
        }
        Glyph::Sliders => {
            for (y, x) in [(0.28, 0.34), (0.72, 0.68)] {
                let yy = g.top() + w * y;
                p.line_segment([pos2(g.left(), yy), pos2(g.right(), yy)], line);
                p.circle_filled(pos2(g.left() + w * x, yy), w * 0.15, Color32::WHITE);
            }
        }
        Glyph::Clock => {
            p.circle_stroke(c, w * 0.46, line);
            p.line_segment([c, c + vec2(0.0, -w * 0.28)], line);
            p.line_segment([c, c + vec2(w * 0.2, w * 0.1)], line);
        }
        Glyph::Display => {
            let screen = Rect::from_min_size(g.min, vec2(w, w * 0.7));
            p.rect_stroke(screen, CornerRadius::same(2), line, StrokeKind::Inside);
            let y = g.top() + w * 0.92;
            p.line_segment(
                [pos2(g.left() + w * 0.28, y), pos2(g.right() - w * 0.28, y)],
                line,
            );
        }
    }
}

/// 侧栏顶部的应用图标。
pub fn app_logo(p: &Painter, rect: Rect) {
    badge(
        p,
        rect.shrink(0.0),
        Glyph::Clipboard,
        Color32::from_rgb(0x2B, 0x9F, 0xE6),
    );
}

/// 分区标题：彩色小图标 + 标题。
pub fn section_header(ui: &mut Ui, glyph: Glyph, color: Color32, title: &str) {
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(24.0, 24.0), Sense::hover());
        badge(ui.painter(), r, glyph, color);
        ui.add_space(2.0);
        ui.label(RichText::new(title).size(15.0).strong().color(TEXT));
    });
    ui.add_space(2.0);
}

/// 分区卡片：比窗口底更深一点的圆角块，占满可用宽度。
pub fn card<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Frame::new()
        .fill(CARD_BG)
        .corner_radius(CornerRadius::same(12))
        .inner_margin(egui::Margin::symmetric(24, 16))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 10.0;
            add(ui)
        })
        .inner
}

/// 一行表单：标签右对齐占一列，控件紧接在右边。
pub fn form_row<R>(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(
            vec2(LABEL_W, 28.0),
            Layout::right_to_left(Align::Center),
            |ui| {
                ui.label(RichText::new(label).color(TEXT));
            },
        );
        ui.add_space(6.0);
        add(ui)
    })
    .inner
}

pub fn weak(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).size(12.0).color(TEXT_WEAK));
}

/// 蓝底白勾的勾选框。返回值表示这一帧被点击切换了。
pub fn check_box(ui: &mut Ui, value: &mut bool, label: &str) -> bool {
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), FontId::proportional(14.0), TEXT);
    let size = vec2(26.0 + galley.size().x + 4.0, 24.0);
    let (rect, mut resp) = ui.allocate_exact_size(size, Sense::click());
    if resp.clicked() {
        *value = !*value;
        resp.mark_changed();
    }
    let b = Rect::from_min_size(pos2(rect.left(), rect.center().y - 9.0), vec2(18.0, 18.0));
    let p = ui.painter();
    if *value {
        p.rect_filled(b, CornerRadius::same(5), CHECK_BLUE);
        let check = Stroke::new(1.8, Color32::WHITE);
        let pts = [
            b.min + vec2(4.5, 9.5),
            b.min + vec2(7.8, 12.8),
            b.min + vec2(13.5, 5.5),
        ];
        p.line_segment([pts[0], pts[1]], check);
        p.line_segment([pts[1], pts[2]], check);
    } else {
        p.rect_filled(b, CornerRadius::same(5), CONTROL_BG);
        p.rect_stroke(
            b,
            CornerRadius::same(5),
            Stroke::new(1.0, Color32::from_gray(0x70)),
            StrokeKind::Inside,
        );
    }
    p.galley(
        pos2(rect.left() + 26.0, rect.center().y - galley.size().y / 2.0),
        galley,
        TEXT,
    );
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp.changed()
}

/// 下拉框。选项是 (值, 显示文案)；选了新值时返回它。
pub fn dropdown<T: Clone + PartialEq>(
    ui: &mut Ui,
    id: &str,
    current: &T,
    options: &[(T, String)],
    width: f32,
) -> Option<T> {
    let mut value = current.clone();
    let text = options
        .iter()
        .find(|(v, _)| v == current)
        .map(|(_, l)| l.as_str())
        .unwrap_or("…");
    egui::ComboBox::from_id_salt(id)
        .selected_text(text)
        .width(width)
        .show_ui(ui, |ui| {
            for (v, label) in options {
                ui.selectable_value(&mut value, v.clone(), label);
            }
        });
    (value != *current).then_some(value)
}

/// 药丸形按钮（恢复默认、初始化、+、−）。
pub fn pill_button(ui: &mut Ui, text: &str, enabled: bool) -> Response {
    ui.add_enabled(
        enabled,
        egui::Button::new(RichText::new(text).color(TEXT))
            .fill(CONTROL_BG)
            .corner_radius(CornerRadius::same(6))
            .min_size(vec2(0.0, 28.0)),
    )
}

/// 无边框窗口顶部的拖动条高度。
pub const TITLE_BAR_H: f32 = 40.0;

/// 自绘窗口控制：非 macOS 上设置窗口没有系统标题栏（Wayland 下 GNOME 不提供服务端装饰，
/// winit 补画的标题栏又小又和深色界面混在一起，关闭按钮几乎看不见）。
/// 顶部整条可拖动，右上角是最小化和关闭，四边四角可以拉伸，外围有一圈细边框。
pub fn window_controls(ctx: &egui::Context) {
    use egui::{Id, Order, ResizeDirection as Dir, ViewportCommand as Cmd};
    let screen = ctx.content_rect();
    let fullscreen_like = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
    egui::Area::new(Id::unique("window-chrome"))
        .order(Order::Foreground)
        .fixed_pos(screen.min)
        .interactable(true)
        .show(ctx, |ui| {
            let bar = Rect::from_min_size(screen.min, vec2(screen.width(), TITLE_BAR_H));
            let drag = ui.interact(bar, Id::unique("chrome-drag"), Sense::click_and_drag());
            if drag.drag_started() {
                ctx.send_viewport_cmd(Cmd::StartDrag);
            }

            if !fullscreen_like {
                let e = 5.0;
                let (l, r, t, b) = (screen.left(), screen.right(), screen.top(), screen.bottom());
                let zones = [
                    (
                        Rect::from_min_max(pos2(l + e, t), pos2(r - e, t + e)),
                        Dir::North,
                        egui::CursorIcon::ResizeVertical,
                    ),
                    (
                        Rect::from_min_max(pos2(l + e, b - e), pos2(r - e, b)),
                        Dir::South,
                        egui::CursorIcon::ResizeVertical,
                    ),
                    (
                        Rect::from_min_max(pos2(l, t + e), pos2(l + e, b - e)),
                        Dir::West,
                        egui::CursorIcon::ResizeHorizontal,
                    ),
                    (
                        Rect::from_min_max(pos2(r - e, t + e), pos2(r, b - e)),
                        Dir::East,
                        egui::CursorIcon::ResizeHorizontal,
                    ),
                    (
                        Rect::from_min_size(pos2(l, t), vec2(e, e)),
                        Dir::NorthWest,
                        egui::CursorIcon::ResizeNwSe,
                    ),
                    (
                        Rect::from_min_size(pos2(r - e, t), vec2(e, e)),
                        Dir::NorthEast,
                        egui::CursorIcon::ResizeNeSw,
                    ),
                    (
                        Rect::from_min_size(pos2(l, b - e), vec2(e, e)),
                        Dir::SouthWest,
                        egui::CursorIcon::ResizeNeSw,
                    ),
                    (
                        Rect::from_min_size(pos2(r - e, b - e), vec2(e, e)),
                        Dir::SouthEast,
                        egui::CursorIcon::ResizeNwSe,
                    ),
                ];
                for (i, (rect, dir, cursor)) in zones.into_iter().enumerate() {
                    let resp = ui.interact(rect, Id::unique(("chrome-resize", i)), Sense::drag());
                    if resp.hovered() {
                        ctx.set_cursor_icon(cursor);
                    }
                    if resp.drag_started() {
                        ctx.send_viewport_cmd(Cmd::BeginResize(dir));
                    }
                }
            }

            let size = 28.0;
            let y = screen.top() + (TITLE_BAR_H - size) / 2.0;
            let close =
                Rect::from_min_size(pos2(screen.right() - 12.0 - size, y), vec2(size, size));
            let mini = Rect::from_min_size(pos2(close.left() - 6.0 - size, y), vec2(size, size));
            for (rect, id, is_close) in [(mini, "chrome-min", false), (close, "chrome-close", true)]
            {
                let resp = ui.interact(rect, Id::unique(id), Sense::click());
                let hovered = resp.hovered();
                let fill = match (is_close, hovered) {
                    (true, true) => Color32::from_rgb(0xE5, 0x48, 0x4D),
                    (false, true) => CONTROL_HOVER,
                    _ => CONTROL_BG,
                };
                let fg = if hovered { Color32::WHITE } else { TEXT };
                let p = ui.painter();
                p.rect_filled(rect, CornerRadius::same(7), fill);
                let c = rect.center();
                let stroke = Stroke::new(1.6, fg);
                if is_close {
                    let d = 5.0;
                    p.line_segment([c + vec2(-d, -d), c + vec2(d, d)], stroke);
                    p.line_segment([c + vec2(-d, d), c + vec2(d, -d)], stroke);
                } else {
                    p.line_segment([c + vec2(-5.0, 4.0), c + vec2(5.0, 4.0)], stroke);
                }
                if hovered {
                    ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                if resp.clicked() {
                    ctx.send_viewport_cmd(if is_close {
                        Cmd::Close
                    } else {
                        Cmd::Minimized(true)
                    });
                }
            }

            ui.painter().rect_stroke(
                screen,
                CornerRadius::ZERO,
                Stroke::new(1.0, Color32::from_gray(0x4A)),
                StrokeKind::Inside,
            );
        });
}

/// 快捷键录制的结果。
pub enum Recorded {
    /// 还在等按键，或没有变化。
    Nothing,
    /// 按下了带修饰键的组合，写成 `shift+super+c` 这样的形式。
    Spec(String),
}

/// 快捷键按钮：点一下进入录制，按下带修饰键的组合就记下来，Esc 取消。
pub fn shortcut_button(ui: &mut Ui, label: &str, hint: &str, recording: &mut bool) -> Recorded {
    let (rect, resp) = ui.allocate_exact_size(vec2(160.0, 28.0), Sense::click());
    if resp.clicked() {
        *recording = !*recording;
        ui.data_mut(|d| d.insert_temp(resp.id.with("held_keys"), 0u8));
    }
    let fill = if *recording {
        Color32::from_rgb(0x1C, 0x3A, 0x5E)
    } else if resp.hovered() {
        CONTROL_HOVER
    } else {
        CONTROL_BG
    };
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(6), fill);
    if *recording {
        p.rect_stroke(
            rect,
            CornerRadius::same(6),
            Stroke::new(1.5, CHECK_BLUE),
            StrokeKind::Inside,
        );
    }
    p.text(
        rect.center(),
        Align2::CENTER_CENTER,
        if *recording { hint } else { label },
        FontId::proportional(if *recording { 12.5 } else { 14.0 }),
        if *recording { TEXT_WEAK } else { TEXT },
    );
    if !*recording {
        return Recorded::Nothing;
    }
    // egui 的 Modifiers 在 macOS 之外不记录 Super（Windows / Meta）键，所以得自己推断。
    // 不能只看 SuperLeft 事件：egui 的按键事件取的是物理键位，键盘布局把 Ctrl 和 Win 对调时
    // （xkb 的 altwin:ctrl_win，很多 Mac 风格配置），按下「Super」来的是 ControlLeft 事件，
    // 而 modifiers 里 ctrl 却是假的。所以记下物理上按着的 Ctrl / Alt / Super 键，
    // 再和 modifiers 对照，见 `super_from`。跨帧保存。
    let held_id = resp.id.with("held_keys");
    let mut held: u8 = ui.data(|d| d.get_temp::<u8>(held_id)).unwrap_or(0);
    // egui-winit 会把 Cmd+C / Cmd+X / Cmd+V 变成 Copy / Cut / Paste 事件，不再发按键事件，
    // 所以这三个键要从这些事件里还原，不然默认的 ⇧⌘C 录不进来。
    let found = ui.input(|i| {
        // 一帧里可能有好几个事件（按得快、或帧率低时），修饰键要取事件发生那一刻的，
        // 而不是这一帧结束时的 `i.modifiers`，所以边遍历边跟着事件更新。
        let mut mods = i.modifiers;
        for ev in &i.events {
            if let egui::Event::ModifiersChanged(m) = ev {
                mods = *m;
            }
            if let egui::Event::Key { modifiers, .. } = ev {
                mods = *modifiers;
            }
            let key = match ev {
                egui::Event::Key { key, pressed, .. } if physical_modifier_bit(*key) != 0 => {
                    let bit = physical_modifier_bit(*key);
                    if *pressed {
                        held |= bit;
                    } else {
                        held &= !bit;
                    }
                    None
                }
                egui::Event::Key {
                    key,
                    pressed: true,
                    repeat: false,
                    ..
                } => {
                    if *key == egui::Key::Escape && !mods.any() && !super_from(mods, held) {
                        return Some(None);
                    }
                    key_name(*key)
                }
                egui::Event::Copy => Some("c".to_owned()),
                egui::Event::Cut => Some("x".to_owned()),
                egui::Event::Paste(_) => Some("v".to_owned()),
                _ => None,
            };
            if let Some(key) = key {
                let super_now = super_from(mods, held);
                if mods.any() || super_now {
                    return Some(Some(spec_string(mods, super_now, &key)));
                }
            }
        }
        None
    });
    ui.data_mut(|d| d.insert_temp(held_id, if found.is_none() { held } else { 0 }));
    match found {
        Some(Some(spec)) => {
            *recording = false;
            Recorded::Spec(spec)
        }
        Some(None) => {
            *recording = false;
            Recorded::Nothing
        }
        None => Recorded::Nothing,
    }
}

const HELD_CTRL: u8 = 1;
const HELD_ALT: u8 = 2;
const HELD_SUPER: u8 = 4;

/// 物理修饰键对应的位；不是这几个键返回 0。
fn physical_modifier_bit(key: egui::Key) -> u8 {
    use egui::Key::*;
    match key {
        ControlLeft | ControlRight => HELD_CTRL,
        AltLeft | AltRight => HELD_ALT,
        SuperLeft | SuperRight => HELD_SUPER,
        _ => 0,
    }
}

/// 此刻按着的是不是 Super。`held` 是物理上按着的 Ctrl / Alt / Super 键（`HELD_*` 的位或），
/// `mods` 是系统按键盘布局解释后的修饰键状态：
/// - 物理 Ctrl（或 Alt）按着，`mods` 里却没有 ctrl（alt）：这个键在布局里其实是 Super；
/// - 物理 Super 按着，而 `mods` 里的 ctrl（alt）没有物理键能解释：它在布局里是 Ctrl（Alt），不是 Super；
/// - 否则物理 Super 按着就是 Super。
fn super_from(mods: egui::Modifiers, held: u8) -> bool {
    let ctrl_key = held & HELD_CTRL != 0;
    let alt_key = held & HELD_ALT != 0;
    let super_key = held & HELD_SUPER != 0;
    if mods.mac_cmd {
        return true;
    }
    if (ctrl_key && !mods.ctrl) || (alt_key && !mods.alt) {
        return true;
    }
    let ctrl_from_super = mods.ctrl && !ctrl_key;
    let alt_from_super = mods.alt && !alt_key;
    super_key && !ctrl_from_super && !alt_from_super
}

/// 修饰键 + 主键写成 `shift+super+c`；`super_held` 是 egui 不记录的 Super 键状态。
fn spec_string(mods: egui::Modifiers, super_held: bool, key: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    if mods.ctrl {
        parts.push("ctrl");
    }
    if mods.alt {
        parts.push("alt");
    }
    if mods.shift {
        parts.push("shift");
    }
    if mods.mac_cmd || super_held {
        parts.push("super");
    }
    parts.push(key);
    parts.join("+")
}

/// egui 按键转成 `HotkeySpec` 认识的名字；认不出的原样小写，让解析器报错。
fn key_name(key: egui::Key) -> Option<String> {
    use egui::Key::*;
    // 修饰键和锁定键本身也会作为按键事件送来（先按下 Ctrl 就是一个 ControlLeft），
    // 它们不是主键：返回 None，继续等真正的主键。
    if matches!(
        key,
        ShiftLeft
            | ShiftRight
            | ControlLeft
            | ControlRight
            | AltLeft
            | AltRight
            | SuperLeft
            | SuperRight
            | CapsLock
            | NumLock
            | ScrollLock
            | Fn
    ) {
        return None;
    }
    let special = match key {
        Minus => "-",
        Equals => "=",
        Comma => ",",
        Period => ".",
        Slash => "/",
        Semicolon => ";",
        Backslash => "\\",
        OpenBracket => "[",
        CloseBracket => "]",
        Quote => "'",
        Backtick => "`",
        _ => "",
    };
    if !special.is_empty() {
        return Some(special.to_owned());
    }
    let name = key.name().to_lowercase();
    Some(
        name.strip_prefix("arrow")
            .map(str::to_owned)
            .unwrap_or(name),
    )
}

/// 存储时间滑块：等距刻度，当前档位上方有一个蓝色图钉，点击或拖动选择最近的刻度。
pub fn history_slider(ui: &mut Ui, stops: &[(u32, String)], current: u32) -> Option<u32> {
    let width = ui.available_width().max(200.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 62.0), Sense::click_and_drag());
    let inset = 28.0_f32.min(width / 8.0);
    let n = stops.len();
    let x_of = |i: usize| rect.left() + inset + (width - 2.0 * inset) * i as f32 / (n - 1) as f32;
    let y = rect.top() + 32.0;
    let p = ui.painter();
    let track = Color32::from_gray(0xD8);
    p.line_segment(
        [pos2(x_of(0), y), pos2(x_of(n - 1), y)],
        Stroke::new(2.0, track),
    );
    for (i, (_, label)) in stops.iter().enumerate() {
        let x = x_of(i);
        p.line_segment(
            [pos2(x, y - 6.0), pos2(x, y + 6.0)],
            Stroke::new(2.0, track),
        );
        p.text(
            pos2(x, y + 14.0),
            Align2::CENTER_TOP,
            label,
            FontId::proportional(11.5),
            TICK_TEAL,
        );
    }
    if let Some(i) = stops.iter().position(|(v, _)| *v == current) {
        let x = x_of(i);
        let head = pos2(x, y - 20.0);
        p.circle_filled(head, 7.5, PIN_BLUE);
        p.add(Shape::convex_polygon(
            vec![
                pos2(x - 5.5, y - 16.0),
                pos2(x + 5.5, y - 16.0),
                pos2(x, y - 6.5),
            ],
            PIN_BLUE,
            Stroke::NONE,
        ));
        p.circle_filled(head, 2.8, Color32::WHITE);
    }
    if (resp.clicked() || resp.dragged())
        && let Some(pos) = resp.interact_pointer_pos()
    {
        let nearest = nearest_stop(pos.x, rect.left() + inset, width - 2.0 * inset, n);
        if stops[nearest].0 != current {
            return Some(stops[nearest].0);
        }
    }
    None
}

/// 横坐标 `x` 离哪个刻度最近（刻度在 `left` 起、总长 `span` 上等距）。
pub fn nearest_stop(x: f32, left: f32, span: f32, n: usize) -> usize {
    if n <= 1 {
        return 0;
    }
    let t = ((x - left) / span).clamp(0.0, 1.0);
    (t * (n - 1) as f32).round() as usize
}

/// 侧栏一行：彩色徽标 + 标题，选中时底色高亮。
pub fn nav_item(
    ui: &mut Ui,
    selected: bool,
    glyph: Glyph,
    color: Color32,
    title: &str,
) -> Response {
    let width = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 38.0), Sense::click());
    let p = ui.painter();
    if selected {
        p.rect_filled(rect, CornerRadius::same(9), SIDEBAR_SELECTED);
    } else if resp.hovered() {
        p.rect_filled(rect, CornerRadius::same(9), Color32::from_white_alpha(10));
    }
    let b = Rect::from_min_size(
        pos2(rect.left() + 11.0, rect.center().y - 12.0),
        Vec2::splat(24.0),
    );
    badge(p, b, glyph, color);
    p.text(
        pos2(rect.left() + 48.0, rect.center().y),
        Align2::LEFT_CENTER,
        title,
        FontId::proportional(14.5),
        TEXT,
    );
    resp
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearest_stop_snaps_to_equal_ticks() {
        // 7 个刻度，跨度 600：间隔 100。
        assert_eq!(nearest_stop(0.0, 0.0, 600.0, 7), 0);
        assert_eq!(nearest_stop(49.0, 0.0, 600.0, 7), 0);
        assert_eq!(nearest_stop(51.0, 0.0, 600.0, 7), 1);
        assert_eq!(nearest_stop(310.0, 0.0, 600.0, 7), 3);
        assert_eq!(nearest_stop(999.0, 0.0, 600.0, 7), 6);
        assert_eq!(nearest_stop(-20.0, 0.0, 600.0, 7), 0);
    }

    #[test]
    fn modifier_keys_are_not_main_keys() {
        for key in [
            egui::Key::ControlLeft,
            egui::Key::AltRight,
            egui::Key::ShiftLeft,
            egui::Key::SuperLeft,
            egui::Key::CapsLock,
            egui::Key::Fn,
        ] {
            assert_eq!(key_name(key), None, "{key:?}");
        }
    }

    #[test]
    fn super_key_is_recorded_even_though_egui_does_not_track_it() {
        use crate::hotkey::HotkeySpec;
        let shift = egui::Modifiers {
            shift: true,
            ..Default::default()
        };
        let spec = spec_string(shift, true, "c");
        assert_eq!(spec, "shift+super+c");
        assert!(spec.parse::<HotkeySpec>().is_ok());
        assert_eq!(spec_string(shift, false, "c"), "shift+c");
    }

    #[test]
    fn super_is_inferred_from_physical_keys_and_layout_modifiers() {
        let none = egui::Modifiers::default();
        let ctrl = egui::Modifiers {
            ctrl: true,
            ..Default::default()
        };
        // 普通布局：Win 键是 Super。
        assert!(super_from(none, HELD_SUPER));
        // 普通 Ctrl。
        assert!(!super_from(ctrl, HELD_CTRL));
        // Ctrl 和 Win 对调：物理 Ctrl 键按下，布局里是 Super，modifiers 没有 ctrl。
        assert!(super_from(none, HELD_CTRL));
        // 对调后物理 Win 键按下，布局里是 Ctrl。
        assert!(!super_from(ctrl, HELD_SUPER));
        // 对调后两个键一起按：Ctrl 和 Super 都在。
        assert!(super_from(ctrl, HELD_SUPER | HELD_CTRL));
        // 什么都没按。
        assert!(!super_from(none, 0));
    }

    #[test]
    fn key_names_match_the_hotkey_parser() {
        use crate::hotkey::HotkeySpec;
        for (key, expect) in [
            (egui::Key::C, "c"),
            (egui::Key::Num1, "1"),
            (egui::Key::F5, "f5"),
            (egui::Key::Space, "space"),
            (egui::Key::ArrowUp, "up"),
            (egui::Key::Minus, "-"),
            (egui::Key::Enter, "enter"),
        ] {
            let name = key_name(key).unwrap();
            assert_eq!(name, expect);
            assert!(
                format!("ctrl+{name}").parse::<HotkeySpec>().is_ok(),
                "{name}"
            );
        }
    }
}
