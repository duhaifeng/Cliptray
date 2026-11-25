//! 历史面板的 egui 界面，复刻 iCopy 的横向剪切板（`RowViewController` / `RowItem` / `TagsView`）。
//!
//! 键盘：↑↓ 切换条目，←→ / Tab 切换分类，Enter 或数字键粘贴，⌘F（Linux 为 Ctrl+F）搜索，Esc 关闭。
//! 鼠标：悬停选中卡片、悬停切换分类，单击粘贴，右键菜单，滚轮横向滚动。

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;
use std::time::{Duration, Instant};

use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, FontId, Id, Key, Modifiers, Painter, Pos2, Rect,
    RichText, Sense, Shape, Stroke, StrokeKind, TextEdit, TextureHandle, TextureOptions, Vec2,
    pos2, text::LayoutJob, vec2,
};

use super::state::PickerState;
use super::theme as t;
use super::{Locations, fonts, save_last_tag};
use crate::appicon;
use crate::config::{Action, ClickAction, Config};
use crate::i18n::{self, Lang, Text};
use crate::ipc::Pick;
use crate::model::{Item, ItemKind, Tag, Transform};
use crate::store::{Store, now_secs};

/// 缩略图最长边（像素）。卡片 278 点，Retina 下约 560 像素。
const THUMB_MAX_SIDE: u32 = 560;
/// 每帧最多解码的图片数，避免一打开面板就在首帧里解码一屏截图。
const DECODES_PER_FRAME: usize = 2;
/// 卡片最多排版的字符数与行数；一张卡片只显示得下十几行，更长的文本交给 egui 排版会卡住界面。
const PREVIEW_CHARS: usize = 1200;
const PREVIEW_LINES: usize = 24;
const APP_NAME_CHARS: usize = 22;
const UV: Rect = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Confirm {
    Delete(i64),
    Clear { keep_favorites: bool },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MenuAction {
    Pick {
        action: Action,
        plain: bool,
        transform: Transform,
    },
    ToggleFavorite,
    Delete,
    Clear {
        keep_favorites: bool,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CardEvent {
    Click,
    DoubleClick,
    Send,
    Copy,
}

/// 卡片上不随帧变化、计算又不便宜的内容（字符数要扫全文，文件要 stat）。
struct Meta {
    preview: String,
    size: String,
    missing: bool,
}

pub struct PickerApp {
    st: PickerState,
    cfg: Config,
    lang: Lang,
    loc: Locations,
    result: Rc<RefCell<Option<Pick>>>,
    /// 工具栏 T 按钮：文本以纯文本写回。
    plain: bool,
    thumbs: HashMap<i64, Option<TextureHandle>>,
    icons: HashMap<String, Option<TextureHandle>>,
    meta: HashMap<i64, Meta>,
    scroll: f32,
    reveal_selected: bool,
    focus_search: bool,
    confirm: Option<Confirm>,
    error: Option<String>,
    opened: Instant,
    was_focused: bool,
    placed: bool,
    done: bool,
}

fn search_id() -> Id {
    Id::unique("search")
}

impl PickerApp {
    pub fn new(
        ctx: &egui::Context,
        st: PickerState,
        cfg: Config,
        loc: Locations,
        result: Rc<RefCell<Option<Pick>>>,
    ) -> Self {
        let have_cjk = fonts::install_system_fonts(ctx);
        let lang = Lang::resolve(
            cfg.language,
            crate::platform::ui_language().as_deref(),
            have_cjk,
        );
        install_style(ctx);
        PickerApp {
            st,
            plain: cfg.paste_plain_text,
            cfg,
            lang,
            loc,
            result,
            thumbs: HashMap::new(),
            icons: HashMap::new(),
            meta: HashMap::new(),
            scroll: 0.0,
            reveal_selected: true,
            focus_search: false,
            confirm: None,
            error: None,
            opened: Instant::now(),
            was_focused: false,
            placed: false,
            done: false,
        }
    }

    fn finish(&mut self, ctx: &egui::Context, pick: Option<Pick>) {
        if std::mem::replace(&mut self.done, true) {
            return;
        }
        save_last_tag(&self.loc.data_dir, self.st.tag);
        *self.result.borrow_mut() = pick;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    fn pick(&mut self, ctx: &egui::Context, idx: usize, action: Action) {
        self.pick_with(ctx, idx, action, self.plain, Transform::None);
    }

    fn pick_with(
        &mut self,
        ctx: &egui::Context,
        idx: usize,
        action: Action,
        plain: bool,
        transform: Transform,
    ) {
        if let Some(item) = self.st.item_at(idx) {
            let pick = Pick {
                id: item.id,
                action,
                plain,
                transform,
            };
            self.finish(ctx, Some(pick));
        }
    }

    fn guard<T>(&mut self, r: anyhow::Result<T>) {
        if let Err(e) = r {
            self.error = Some(format!("{e:#}"));
        }
    }

    fn switch_tag(&mut self, tag: Tag) {
        if tag != self.st.tag {
            let r = self.st.set_tag(tag);
            self.guard(r);
            self.scroll = 0.0;
        }
    }

    fn move_by(&mut self, delta: isize) {
        let r = self.st.move_by(delta);
        self.guard(r);
        self.reveal_selected = true;
    }

    fn request_delete(&mut self) {
        let Some(id) = self.st.selected_item().map(|i| i.id) else {
            return;
        };
        if self.cfg.confirm_delete {
            self.confirm = Some(Confirm::Delete(id));
        } else {
            self.delete(id);
        }
    }

    fn delete(&mut self, id: i64) {
        if let Some(idx) = self.st.position_of(id) {
            self.st.select(idx);
            let r = self.st.delete_selected();
            self.guard(r);
        }
    }

    fn run_confirm(&mut self) {
        match self.confirm.take() {
            Some(Confirm::Delete(id)) => self.delete(id),
            Some(Confirm::Clear { keep_favorites }) => {
                let r = self.st.clear_tag(keep_favorites);
                self.guard(r);
                self.scroll = 0.0;
            }
            None => {}
        }
    }

    // ---- 输入 ----

    fn handle_keys(&mut self, ctx: &egui::Context) {
        let consume = |m: Modifiers, k: Key| ctx.input_mut(|i| i.consume_key(m, k));

        if self.confirm.is_some() {
            if consume(Modifiers::NONE, Key::Escape) {
                self.confirm = None;
            } else if consume(Modifiers::NONE, Key::Enter) {
                self.run_confirm();
            }
            return;
        }
        // Esc 和 Enter 在搜索框里也生效，iCopy 同样如此。
        if consume(Modifiers::NONE, Key::Escape) {
            return self.finish(ctx, None);
        }
        if consume(Modifiers::NONE, Key::Enter) {
            return self.pick(ctx, self.st.selected, self.cfg.enter_action);
        }
        if consume(Modifiers::COMMAND, Key::F) {
            self.focus_search = true;
        }
        if consume(Modifiers::SHIFT, Key::Tab) {
            self.switch_tag(self.st.tag.prev());
        } else if consume(Modifiers::NONE, Key::Tab) {
            self.switch_tag(self.st.tag.next());
        }

        // 搜索框有焦点时方向键、删除键和数字都属于输入。
        let searching = ctx.memory(|m| m.has_focus(search_id()));
        if !searching {
            if consume(Modifiers::NONE, Key::ArrowLeft) {
                self.switch_tag(self.st.tag.prev());
            }
            if consume(Modifiers::NONE, Key::ArrowRight) {
                self.switch_tag(self.st.tag.next());
            }
            if consume(Modifiers::NONE, Key::ArrowUp) {
                self.move_by(-1);
            }
            if consume(Modifiers::NONE, Key::ArrowDown) {
                self.move_by(1);
            }
            if consume(Modifiers::NONE, Key::Delete) || consume(Modifiers::NONE, Key::Backspace) {
                self.request_delete();
            }
        }

        let num_mods = match self.cfg.number_modifier.as_str() {
            "alt" => Some(Modifiers::ALT),
            "ctrl" => Some(Modifiers::CTRL),
            _ => None,
        };
        if num_mods.is_some() || !searching {
            let mods = num_mods.unwrap_or(Modifiers::NONE);
            for (n, key) in NUM_KEYS.iter().enumerate() {
                if consume(mods, *key) {
                    return self.pick(ctx, n, self.cfg.enter_action);
                }
            }
        }
    }

    /// 失去焦点就关闭（iCopy“操作后自动关闭”）。只有“拿到过焦点”之后才生效：
    /// 有的窗口管理器会拒绝新窗口抢焦点，那种情况下直接关闭会让面板一闪而过。
    fn close_on_focus_loss(&mut self, ctx: &egui::Context) {
        match ctx.input(|i| i.viewport().focused) {
            Some(true) => self.was_focused = true,
            Some(false)
                if self.cfg.auto_hide
                    && self.was_focused
                    && self.opened.elapsed() > Duration::from_millis(250) =>
            {
                self.finish(ctx, None)
            }
            _ => {}
        }
    }

    /// 启动时按主屏幕摆放；egui 报告的显示器尺寸与之不同（X11 缩放、Wayland、多显示器）时纠正一次。
    fn place_window(&mut self, ctx: &egui::Context) {
        if self.placed {
            return;
        }
        let Some(monitor) = ctx.input(|i| i.viewport().monitor_size) else {
            return;
        };
        self.placed = true;
        // 工作区内缩是物理像素，换算成 egui 的逻辑点。
        let ppp = ctx.pixels_per_point().max(0.1);
        let insets = crate::platform::work_area_insets()
            .map_or((0.0, 0.0, 0.0), |(l, r, _, b)| (l / ppp, r / ppp, b / ppp));
        let (pos, size) = super::board_frame_inset((monitor.x, monitor.y), insets);
        let current = ctx.input(|i| i.viewport().inner_rect);
        if current.is_none_or(|r| (r.size() - size).length() > 1.0 || (r.min - pos).length() > 1.0)
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(pos));
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
    }

    fn open_settings(&mut self, ctx: &egui::Context) {
        let sent = crate::config::Paths::resolve(&self.cfg, Some(&self.loc.config_file))
            .ok()
            .and_then(|paths| crate::ipc::send(&paths.socket, crate::ipc::Command::Settings).ok());
        if sent.is_none()
            && let Ok(exe) = std::env::current_exe()
        {
            let _ = std::process::Command::new(exe)
                .arg("--config")
                .arg(&self.loc.config_file)
                .arg("settings")
                .spawn();
        }
        self.finish(ctx, None);
    }

    // ---- 工具栏 ----

    fn draw_toolbar(&mut self, ui: &mut egui::Ui, bar: Rect) {
        let ctx = ui.ctx().clone();
        let painter = ui.painter().clone();
        let cy = bar.center().y;
        let widths: Vec<f32> = Tag::ORDER
            .iter()
            .map(|tag| text_width(&painter, self.lang.tag(*tag), t::PILL_FONT) + 2.0 * t::PILL_PAD)
            .collect();
        let total = t::SEARCH_W
            + t::SEARCH_GAP
            + widths.iter().sum::<f32>()
            + t::PILL_GAP * (widths.len().saturating_sub(1)) as f32;
        let mut x = bar.center().x - total / 2.0;

        self.draw_search(
            ui,
            Rect::from_min_size(
                pos2(x, cy - t::SEARCH_H / 2.0),
                vec2(t::SEARCH_W, t::SEARCH_H),
            ),
        );
        x += t::SEARCH_W + t::SEARCH_GAP;

        let moved = pointer_moved(ui);
        for (tag, w) in Tag::ORDER.into_iter().zip(widths) {
            let r = Rect::from_min_size(pos2(x, cy - t::PILL_H / 2.0), vec2(w, t::PILL_H));
            let resp = ui.interact(r, Id::unique(("tag", tag.as_str())), Sense::click());
            if resp.clicked() || (self.cfg.hover_switch_group && moved && resp.hovered()) {
                self.switch_tag(tag);
            }
            painter.rect_filled(r, t::PILL_RADIUS, t::tag_color(tag, tag == self.st.tag));
            painter.text(
                r.center(),
                Align2::CENTER_CENTER,
                self.lang.tag(tag),
                FontId::proportional(t::PILL_FONT),
                Color32::WHITE,
            );
            x += w + t::PILL_GAP;
        }

        let icon = Vec2::splat(t::TOOL_ICON);
        let settings = Rect::from_center_size(
            pos2(bar.right() - t::TOOL_RIGHT - t::TOOL_ICON / 2.0, cy),
            icon,
        );
        let plain = settings.translate(vec2(-(t::TOOL_ICON + t::TOOL_GAP), 0.0));

        let hint = self.lang.text(if self.plain {
            Text::PlainOn
        } else {
            Text::PlainOff
        });
        if ui
            .interact(plain.expand(4.0), Id::unique("plain"), Sense::click())
            .on_hover_text(hint)
            .clicked()
        {
            self.plain = !self.plain;
        }
        paint_plain_icon(&painter, plain, self.plain);

        let resp = ui.interact(settings.expand(4.0), Id::unique("settings"), Sense::click());
        if resp.on_hover_text(self.lang.text(Text::Settings)).clicked() {
            self.open_settings(&ctx);
        }
        paint_settings_icon(&painter, settings);

        if let Some(e) = &self.error {
            let msg = format!(
                "{}: {}",
                self.lang.text(Text::Error),
                crate::model::truncate_chars(e, 80)
            );
            painter.text(
                pos2(bar.left() + 20.0, cy),
                Align2::LEFT_CENTER,
                msg,
                FontId::proportional(11.0),
                t::MISSING_RED,
            );
        }
    }

    fn draw_search(&mut self, ui: &mut egui::Ui, r: Rect) {
        let painter = ui.painter();
        painter.rect_filled(
            r,
            CornerRadius::same((t::SEARCH_H / 2.0) as u8),
            t::SEARCH_BG,
        );
        paint_magnifier(painter, pos2(r.left() + 12.0, r.center().y), t::SEARCH_HINT);
        let field = Rect::from_min_max(
            pos2(r.left() + 22.0, r.top()),
            pos2(r.right() - 8.0, r.bottom()),
        );
        let mut text = self.st.search().to_owned();
        let edit = TextEdit::singleline(&mut text)
            .id(search_id())
            .frame(egui::Frame::NONE)
            .margin(egui::Margin::ZERO)
            .font(FontId::proportional(12.0))
            .text_color(Color32::WHITE)
            .hint_text(
                RichText::new(self.lang.text(Text::Search))
                    .color(t::SEARCH_HINT)
                    .size(12.0),
            )
            .desired_width(field.width())
            .align(Align2::LEFT_CENTER);
        let resp = ui.put(field, edit);
        if std::mem::take(&mut self.focus_search) {
            resp.request_focus();
        }
        match self.st.set_search(&text) {
            Ok(true) => self.scroll = 0.0,
            Ok(false) => {}
            Err(e) => self.error = Some(format!("{e:#}")),
        }
    }

    // ---- 卡片 ----

    fn draw_cards(&mut self, ui: &mut egui::Ui, area: Rect) {
        let ctx = ui.ctx().clone();
        let view_w = area.width();
        if ui.rect_contains_pointer(area) {
            // iCopy 的 RowClipView 把纵向滚轮转成横向滚动。
            let d = ui.input(|i| i.smooth_scroll_delta);
            self.scroll -= d.x + d.y;
        }
        // 先补页再算可见范围：滚到末尾附近时新的一页要在同一帧里出现。
        if self.st.has_more
            && t::visible_cards(self.scroll, view_w, usize::MAX).end + 3 >= self.st.items.len()
        {
            let r = self.st.load_more();
            self.guard(r);
        }
        let n = self.st.items.len();
        if std::mem::take(&mut self.reveal_selected) && n > 0 {
            self.scroll = t::scroll_to_reveal(self.scroll, view_w, self.st.selected);
        }
        let max_scroll = (t::strip_width(n) - view_w).max(0.0);
        self.scroll = self.scroll.clamp(0.0, max_scroll);

        let painter = ui.painter_at(area);
        let moved = pointer_moved(ui);
        let now = i18n::from_unix(now_secs());
        let mut budget = DECODES_PER_FRAME;
        let mut event: Option<(usize, CardEvent)> = None;
        let mut menu: Option<(usize, MenuAction)> = None;
        let mut hovered: Option<usize> = None;
        let lang = self.lang;
        let (plain, tag) = (self.plain, self.st.tag);

        for idx in t::visible_cards(self.scroll, view_w, n) {
            let x = area.left() + t::card_x(idx) - self.scroll;
            let rect = Rect::from_min_size(
                pos2(x, area.top() + t::CARD_TOP - t::TOOLBAR_H),
                Vec2::splat(t::CARD),
            );
            let item = &self.st.items[idx];
            let thumb = (item.kind == ItemKind::Image)
                .then(|| thumb(&mut self.thumbs, self.st.store(), &ctx, &mut budget, item))
                .flatten();
            let icon = item
                .source_app
                .as_deref()
                .and_then(|app| app_icon(&mut self.icons, &self.loc.data_dir, &ctx, app));
            let meta = self
                .meta
                .entry(item.id)
                .or_insert_with(|| meta_for(item, self.st.store(), lang));
            let time = i18n::relative_time(lang, now, i18n::from_unix(item.created_at));
            let buttons = paint_card(
                &painter,
                &CardView {
                    rect,
                    item,
                    idx,
                    selected: idx == self.st.selected,
                    kind: lang.kind(item.kind),
                    time: &time,
                    meta,
                    thumb: thumb.as_ref(),
                    icon: icon.as_ref(),
                },
            );

            let resp = ui.interact(rect, Id::unique(("card", item.id)), Sense::click());
            let send = ui.interact(buttons.send, Id::unique(("send", item.id)), Sense::click());
            let copy = ui.interact(buttons.copy, Id::unique(("copy", item.id)), Sense::click());
            if moved && resp.contains_pointer() {
                hovered = Some(idx);
            }
            if send.clicked() {
                event = Some((idx, CardEvent::Send));
            } else if copy.clicked() {
                event = Some((idx, CardEvent::Copy));
            } else if resp.double_clicked() {
                event = Some((idx, CardEvent::DoubleClick));
            } else if resp.clicked() {
                event = Some((idx, CardEvent::Click));
            }
            if resp.secondary_clicked() {
                hovered = Some(idx);
            }
            let (kind, favorite) = (item.kind, item.favorite);
            let mut chosen = None;
            resp.context_menu(|ui| menu_ui(ui, lang, kind, favorite, plain, tag, &mut chosen));
            if let Some(a) = chosen {
                menu = Some((idx, a));
            }
        }

        paint_scroll_indicator(&painter, area, self.scroll, t::strip_width(n));

        if let Some(idx) = hovered {
            self.st.select(idx);
        }
        if let Some((idx, ev)) = event {
            match ev {
                CardEvent::Send => self.pick(&ctx, idx, Action::Paste),
                CardEvent::Copy => self.pick(&ctx, idx, Action::Copy),
                CardEvent::Click => match self.cfg.click_action {
                    ClickAction::Paste => self.pick(&ctx, idx, Action::Paste),
                    ClickAction::Copy => self.pick(&ctx, idx, Action::Copy),
                    ClickAction::Select => self.st.select(idx),
                },
                CardEvent::DoubleClick if self.cfg.click_action == ClickAction::Select => {
                    self.pick(&ctx, idx, self.cfg.double_click_action)
                }
                CardEvent::DoubleClick => {}
            }
        }
        if let Some((idx, action)) = menu {
            self.apply_menu(&ctx, idx, action);
        }
    }

    fn apply_menu(&mut self, ctx: &egui::Context, idx: usize, action: MenuAction) {
        self.st.select(idx);
        match action {
            MenuAction::Pick {
                action,
                plain,
                transform,
            } => self.pick_with(ctx, idx, action, plain, transform),
            MenuAction::ToggleFavorite => {
                let r = self.st.toggle_favorite_selected();
                self.guard(r);
            }
            MenuAction::Delete => self.request_delete(),
            MenuAction::Clear { keep_favorites } => {
                self.confirm = Some(Confirm::Clear { keep_favorites })
            }
        }
    }

    fn draw_confirm(&mut self, ctx: &egui::Context) {
        let Some(confirm) = self.confirm else { return };
        let lang = self.lang;
        let message = match confirm {
            Confirm::Delete(_) => lang.text(Text::ConfirmDelete).to_owned(),
            Confirm::Clear { keep_favorites } => {
                format!(
                    "{}\n{}",
                    lang.clear_tag(self.st.tag, keep_favorites),
                    lang.text(Text::ConfirmDelete)
                )
            }
        };
        let mut answer = None;
        let frame = egui::Frame::popup(&ctx.global_style())
            .inner_margin(16)
            .corner_radius(10);
        egui::Modal::new(Id::unique("confirm"))
            .frame(frame)
            .backdrop_color(Color32::from_black_alpha(90))
            .show(ctx, |ui| {
                ui.set_width(240.0);
                ui.vertical_centered(|ui| {
                    ui.label(
                        RichText::new(lang.text(Text::KindTips))
                            .size(14.0)
                            .strong()
                            .color(Color32::WHITE),
                    );
                    ui.add_space(6.0);
                    ui.label(RichText::new(message).size(12.0).color(t::BODY_TEXT));
                    ui.add_space(14.0);
                    ui.horizontal(|ui| {
                        let w = 80.0;
                        ui.add_space(
                            (ui.available_width() - 2.0 * w - ui.spacing().item_spacing.x) / 2.0,
                        );
                        if ui
                            .add_sized([w, 24.0], egui::Button::new(lang.text(Text::Cancel)))
                            .clicked()
                        {
                            answer = Some(false);
                        }
                        let sure = egui::Button::new(
                            RichText::new(lang.text(Text::Sure)).color(Color32::WHITE),
                        )
                        .fill(t::SEND_BLUE);
                        if ui.add_sized([w, 24.0], sure).clicked() {
                            answer = Some(true);
                        }
                    });
                });
            });
        match answer {
            Some(true) => self.run_confirm(),
            Some(false) => self.confirm = None,
            None => {}
        }
    }

    /// 一帧的完整界面与输入处理。独立于 `eframe::Frame`，测试可以直接驱动它。
    fn show(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.place_window(&ctx);
        self.close_on_focus_loss(&ctx);
        self.handle_keys(&ctx);
        if self.done {
            return;
        }

        let full = ui.max_rect();
        let bar = Rect::from_min_size(full.min, vec2(full.width(), t::TOOLBAR_H));
        let area = Rect::from_min_max(pos2(full.left(), bar.bottom()), full.max);
        let painter = ui.painter();
        painter.rect_filled(full, t::WINDOW_RADIUS, t::CONTENT_BG);
        let r = t::WINDOW_RADIUS;
        painter.rect_filled(
            bar,
            CornerRadius {
                nw: r,
                ne: r,
                sw: 0,
                se: 0,
            },
            t::TOOLBAR_BG,
        );

        self.draw_toolbar(ui, bar);
        if !self.done {
            self.draw_cards(ui, area);
        }
        self.draw_confirm(&ctx);
    }
}

impl eframe::App for PickerApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.show(ui);
    }

    /// 窗口透明，圆角外露出桌面。
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0; 4]
    }
}

const NUM_KEYS: [Key; 9] = [
    Key::Num1,
    Key::Num2,
    Key::Num3,
    Key::Num4,
    Key::Num5,
    Key::Num6,
    Key::Num7,
    Key::Num8,
    Key::Num9,
];

fn install_style(ctx: &egui::Context) {
    let mut v = egui::Visuals::dark();
    v.window_fill = Color32::from_rgb(0x3C, 0x3A, 0x3B);
    v.panel_fill = Color32::TRANSPARENT;
    v.window_stroke = Stroke::new(1.0, Color32::from_gray(90));
    v.selection.bg_fill = t::SEND_BLUE;
    v.selection.stroke = Stroke::new(1.0, Color32::WHITE);
    v.text_cursor.stroke = Stroke::new(1.5, Color32::WHITE);
    ctx.set_visuals(v);
}

fn pointer_moved(ui: &egui::Ui) -> bool {
    ui.input(|i| i.pointer.delta() != Vec2::ZERO)
}

fn text_width(painter: &Painter, text: &str, size: f32) -> f32 {
    painter
        .layout_no_wrap(text.to_owned(), FontId::proportional(size), Color32::WHITE)
        .size()
        .x
}

fn menu_ui(
    ui: &mut egui::Ui,
    lang: Lang,
    kind: ItemKind,
    favorite: bool,
    plain: bool,
    tag: Tag,
    out: &mut Option<MenuAction>,
) {
    ui.set_min_width(200.0);
    let mut entry = |ui: &mut egui::Ui, label: &str, action: MenuAction| {
        if ui.button(label).clicked() {
            *out = Some(action);
            ui.close();
        }
    };
    let pick = |action, plain, transform| MenuAction::Pick {
        action,
        plain,
        transform,
    };
    let is_text = matches!(kind, ItemKind::Text | ItemKind::RichText);
    entry(
        ui,
        lang.text(Text::Paste),
        pick(Action::Paste, plain, Transform::None),
    );
    if kind == ItemKind::RichText {
        let label = lang.text(if plain {
            Text::PasteRich
        } else {
            Text::PastePlain
        });
        entry(ui, label, pick(Action::Paste, !plain, Transform::None));
    }
    entry(
        ui,
        lang.text(Text::Copy),
        pick(Action::Copy, plain, Transform::None),
    );
    if kind == ItemKind::RichText && !plain {
        entry(
            ui,
            lang.text(Text::CopyPlain),
            pick(Action::Copy, true, Transform::None),
        );
    }
    if is_text {
        entry(
            ui,
            lang.text(Text::CopyNoNewlines),
            pick(Action::Copy, true, Transform::StripNewlines),
        );
        entry(
            ui,
            lang.text(Text::CopyNoWhitespace),
            pick(Action::Copy, true, Transform::StripWhitespace),
        );
    }
    ui.separator();
    entry(
        ui,
        lang.text(if favorite {
            Text::Uncollect
        } else {
            Text::Collect
        }),
        MenuAction::ToggleFavorite,
    );
    entry(ui, lang.text(Text::Delete), MenuAction::Delete);
    ui.separator();
    entry(
        ui,
        &lang.clear_tag(tag, false),
        MenuAction::Clear {
            keep_favorites: false,
        },
    );
    entry(
        ui,
        &lang.clear_tag(tag, true),
        MenuAction::Clear {
            keep_favorites: true,
        },
    );
}

// ---- 资源 ----

fn thumb(
    cache: &mut HashMap<i64, Option<TextureHandle>>,
    store: &Store,
    ctx: &egui::Context,
    budget: &mut usize,
    item: &Item,
) -> Option<TextureHandle> {
    if let Some(t) = cache.get(&item.id) {
        return t.clone();
    }
    if *budget == 0 {
        ctx.request_repaint();
        return None;
    }
    *budget -= 1;
    let tex = item.image_file.as_deref().and_then(|name| {
        let img = image::open(store.image_path(name)).ok()?;
        // `thumbnail` 也会放大小图；只在超过上限时缩小。
        let img = if img.width().max(img.height()) > THUMB_MAX_SIDE {
            img.thumbnail(THUMB_MAX_SIDE, THUMB_MAX_SIDE)
        } else {
            img
        }
        .into_rgba8();
        let size = [img.width() as usize, img.height() as usize];
        let color = egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw());
        Some(ctx.load_texture(format!("img{}", item.id), color, TextureOptions::LINEAR))
    });
    cache.insert(item.id, tex.clone());
    tex
}

fn app_icon(
    cache: &mut HashMap<String, Option<TextureHandle>>,
    data_dir: &Path,
    ctx: &egui::Context,
    app: &str,
) -> Option<TextureHandle> {
    if let Some(t) = cache.get(app) {
        return t.clone();
    }
    let tex = image::open(appicon::icon_path(data_dir, app))
        .ok()
        .map(|img| {
            let img = if img.width() > 64 {
                img.thumbnail(64, 64)
            } else {
                img
            }
            .into_rgba8();
            let color = egui::ColorImage::from_rgba_unmultiplied(
                [img.width() as usize, img.height() as usize],
                img.as_raw(),
            );
            ctx.load_texture(format!("icon:{app}"), color, TextureOptions::LINEAR)
        });
    cache.insert(app.to_owned(), tex.clone());
    tex
}

fn meta_for(item: &Item, store: &Store, lang: Lang) -> Meta {
    let file_size = |p: &Path| {
        std::fs::metadata(p)
            .ok()
            .filter(|m| m.is_file())
            .map(|m| format_bytes(m.len()))
    };
    match item.kind {
        ItemKind::Text | ItemKind::RichText => {
            let text = item.text.as_deref().unwrap_or("");
            Meta {
                preview: preview_text(text),
                size: lang.chars(text.chars().count()),
                missing: false,
            }
        }
        ItemKind::Image => {
            let size = item
                .image_file
                .as_deref()
                .and_then(|n| file_size(&store.image_path(n)));
            Meta {
                preview: String::new(),
                size: size.unwrap_or_default(),
                missing: false,
            }
        }
        ItemKind::File | ItemKind::Folder | ItemKind::Files => {
            let missing = item.files.iter().any(|f| !Path::new(f).exists());
            let size = match (item.kind, item.files.first()) {
                (ItemKind::File, Some(f)) => file_size(Path::new(f)).unwrap_or_default(),
                _ => String::new(),
            };
            let names: Vec<&str> = item.files.iter().map(|f| file_name(f)).collect();
            Meta {
                preview: names.join(", "),
                size,
                missing,
            }
        }
    }
}

fn file_name(path: &str) -> &str {
    path.trim_end_matches('/')
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(path)
}

fn preview_text(s: &str) -> String {
    let mut out = String::new();
    let mut lines = 0;
    for c in s.chars().take(PREVIEW_CHARS) {
        match c {
            '\r' => continue,
            '\n' => {
                lines += 1;
                if lines >= PREVIEW_LINES {
                    break;
                }
                out.push('\n');
            }
            '\t' => out.push_str("    "),
            c => out.push(c),
        }
    }
    out
}

/// 文件大小，十进制单位、两位小数（iCopy 卡片上显示为 “40.01 KB” / “1.33 MB”）。
pub(crate) fn format_bytes(n: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if n < 1000 {
        return format!("{n} B");
    }
    let mut v = n as f64 / 1000.0;
    let mut unit = 0;
    while v >= 1000.0 && unit + 1 < UNITS.len() {
        v /= 1000.0;
        unit += 1;
    }
    format!("{v:.2} {}", UNITS[unit])
}

// ---- 绘制 ----

struct CardView<'a> {
    rect: Rect,
    item: &'a Item,
    idx: usize,
    selected: bool,
    kind: &'a str,
    time: &'a str,
    meta: &'a Meta,
    thumb: Option<&'a TextureHandle>,
    icon: Option<&'a TextureHandle>,
}

struct CardButtons {
    send: Rect,
    copy: Rect,
}

fn paint_card(p: &Painter, c: &CardView) -> CardButtons {
    let rect = c.rect;
    let color = t::kind_color(c.item.kind);
    let r = t::CARD_RADIUS;
    let head = Rect::from_min_size(rect.min, vec2(rect.width(), t::HEAD_H));
    let body = Rect::from_min_max(pos2(rect.left(), head.bottom()), rect.max);
    p.rect_filled(
        head,
        CornerRadius {
            nw: r,
            ne: r,
            sw: 0,
            se: 0,
        },
        color,
    );
    p.rect_filled(
        body,
        CornerRadius {
            nw: 0,
            ne: 0,
            sw: r,
            se: r,
        },
        t::BODY_BG,
    );

    // 头部第一行：类型 + 来源应用图标；第二行：时间 + 来源应用名。
    p.text(
        head.left_top() + vec2(10.0, 9.0),
        Align2::LEFT_TOP,
        c.kind,
        FontId::proportional(15.0),
        Color32::WHITE,
    );
    let icon_rect = Rect::from_min_size(
        pos2(head.right() - 10.0 - t::APP_ICON, head.top() + 7.0),
        Vec2::splat(t::APP_ICON),
    );
    match (c.icon, c.item.source_app.as_deref()) {
        (Some(tex), _) => {
            p.image(tex.id(), icon_rect, UV, Color32::WHITE);
        }
        (None, Some(app)) => paint_letter_avatar(p, icon_rect, app),
        (None, None) => {}
    }
    let row2 = head.bottom() - 8.0;
    let small = FontId::proportional(11.0);
    p.text(
        pos2(head.left() + 10.0, row2),
        Align2::LEFT_BOTTOM,
        c.time,
        small.clone(),
        Color32::WHITE,
    );
    if let Some(app) = &c.item.source_app {
        let name = crate::model::truncate_chars(app, APP_NAME_CHARS);
        p.text(
            pos2(head.right() - 10.0, row2),
            Align2::RIGHT_BOTTOM,
            name,
            small.clone(),
            Color32::WHITE,
        );
    }

    let content = Rect::from_min_max(
        body.min + Vec2::splat(t::BODY_PAD),
        pos2(body.right() - t::BODY_PAD, body.bottom() - t::FOOT_H),
    );
    let clip = p.with_clip_rect(content.intersect(p.clip_rect()));
    match c.item.kind {
        ItemKind::Text | ItemKind::RichText => {
            let galley = clip.layout(
                c.meta.preview.clone(),
                FontId::proportional(13.0),
                t::BODY_TEXT,
                content.width(),
            );
            clip.galley(content.min, galley, t::BODY_TEXT);
        }
        ItemKind::Image => {
            if let Some(tex) = c.thumb {
                clip.image(
                    tex.id(),
                    fit_top_left(content, tex.size_vec2()),
                    UV,
                    Color32::WHITE,
                );
            }
        }
        kind => paint_file_body(&clip, content, kind, c.item, c.meta),
    }

    let cy = rect.bottom() - t::FOOT_H / 2.0;
    let foot = FontId::proportional(t::FOOT_FONT);
    p.text(
        pos2(rect.left() + 10.0, cy),
        Align2::LEFT_CENTER,
        (c.idx + 1).to_string(),
        foot.clone(),
        color,
    );
    p.text(
        pos2(rect.center().x, cy),
        Align2::CENTER_CENTER,
        &c.meta.size,
        foot,
        t::BODY_TEXT,
    );
    let copy = Rect::from_center_size(pos2(rect.right() - 16.0, cy), Vec2::splat(16.0));
    let send = copy.translate(vec2(-20.0, 0.0));
    paint_send_icon(p, send.center());
    paint_copy_icon(p, copy.center());

    if c.selected {
        p.rect_stroke(
            rect,
            r,
            Stroke::new(t::SELECTED_BORDER, color),
            StrokeKind::Inside,
        );
    }
    CardButtons { send, copy }
}

/// 按比例缩小放进 `bounds`，贴左上角（iCopy 图片卡片的排法）；不放大小图。
fn fit_top_left(bounds: Rect, img: Vec2) -> Rect {
    if img.x <= 0.0 || img.y <= 0.0 {
        return bounds;
    }
    let scale = (bounds.width() / img.x)
        .min(bounds.height() / img.y)
        .min(1.0);
    Rect::from_min_size(bounds.min, img * scale)
}

fn paint_file_body(p: &Painter, content: Rect, kind: ItemKind, item: &Item, meta: &Meta) {
    let icon_c = pos2(content.center().x, content.top() + 54.0);
    let missing = meta.missing;
    match kind {
        ItemKind::Folder => {
            paint_folder(p, icon_c);
            if missing {
                paint_missing_badge(p, icon_c + vec2(30.0, 26.0));
            }
        }
        ItemKind::Files => {
            paint_document(p, icon_c + vec2(10.0, -6.0), "", false);
            paint_document(p, icon_c + vec2(-10.0, 6.0), "", missing);
        }
        _ => {
            let ext = item
                .files
                .first()
                .and_then(|f| Path::new(f).extension())
                .map(|e| e.to_string_lossy().to_uppercase());
            paint_document(
                p,
                icon_c,
                &crate::model::truncate_chars(ext.as_deref().unwrap_or(""), 5),
                missing,
            );
        }
    }
    let color = if missing {
        t::MISSING_RED
    } else {
        t::BODY_TEXT
    };
    let mut job = LayoutJob::simple(
        meta.preview.clone(),
        FontId::proportional(12.5),
        color,
        content.width(),
    );
    job.halign = Align::Center;
    job.wrap.max_rows = 2;
    job.wrap.break_anywhere = true;
    let galley = p.layout_job(job);
    p.galley(pos2(content.center().x, icon_c.y + 54.0), galley, color);
}

/// 文档图标；文件已不存在时画成灰色带问号（iCopy 的样子）。
fn paint_document(p: &Painter, c: Pos2, ext: &str, missing: bool) {
    let (w, h, fold) = (64.0, 82.0, 16.0);
    let r = Rect::from_center_size(c, vec2(w, h));
    let page = vec![
        r.left_top(),
        pos2(r.right() - fold, r.top()),
        pos2(r.right(), r.top() + fold),
        r.right_bottom(),
        r.left_bottom(),
    ];
    let fill = if missing {
        Color32::from_gray(222)
    } else {
        Color32::from_rgb(0xF4, 0xF4, 0xF4)
    };
    p.add(Shape::convex_polygon(
        page,
        fill,
        Stroke::new(1.0, Color32::from_gray(200)),
    ));
    let corner = vec![
        pos2(r.right() - fold, r.top()),
        pos2(r.right() - fold, r.top() + fold),
        pos2(r.right(), r.top() + fold),
    ];
    p.add(Shape::convex_polygon(
        corner,
        Color32::from_gray(200),
        Stroke::NONE,
    ));
    if missing {
        p.text(
            c,
            Align2::CENTER_CENTER,
            "?",
            FontId::proportional(30.0),
            Color32::from_gray(160),
        );
    } else if !ext.is_empty() {
        p.text(
            pos2(c.x, r.bottom() - 18.0),
            Align2::CENTER_CENTER,
            ext,
            FontId::proportional(12.0),
            Color32::from_gray(130),
        );
    }
}

fn paint_folder(p: &Painter, c: Pos2) {
    let body = Rect::from_center_size(c + vec2(0.0, 4.0), vec2(82.0, 60.0));
    let tab = Rect::from_min_size(body.left_top() - vec2(0.0, 7.0), vec2(32.0, 12.0));
    p.rect_filled(tab, 3, Color32::from_rgb(0x4A, 0x9F, 0xE0));
    p.rect_filled(body, 5, Color32::from_rgb(0x5B, 0xAE, 0xEC));
    let front = Rect::from_min_max(body.left_top() + vec2(0.0, 8.0), body.right_bottom());
    p.rect_filled(front, 5, Color32::from_rgb(0x74, 0xC0, 0xF6));
}

fn paint_missing_badge(p: &Painter, c: Pos2) {
    p.circle_filled(c, 11.0, Color32::from_rgb(0xE5, 0x48, 0x3F));
    p.line_segment(
        [c - vec2(5.5, 0.0), c + vec2(5.5, 0.0)],
        Stroke::new(2.5, Color32::WHITE),
    );
}

fn paint_letter_avatar(p: &Painter, r: Rect, app: &str) {
    p.rect_filled(r, 6, Color32::from_black_alpha(55));
    let letter: String = app
        .chars()
        .next()
        .map(|c| c.to_uppercase().collect())
        .unwrap_or_default();
    p.text(
        r.center(),
        Align2::CENTER_CENTER,
        letter,
        FontId::proportional(14.0),
        Color32::WHITE,
    );
}

fn paint_send_icon(p: &Painter, c: Pos2) {
    p.circle_filled(c, 7.5, t::SEND_BLUE);
    let s = Stroke::new(1.4, Color32::WHITE);
    // ↩ 形箭头：从右上折向左，箭头朝左。
    p.line(
        vec![
            pos2(c.x + 3.0, c.y - 3.5),
            pos2(c.x + 3.0, c.y + 1.0),
            pos2(c.x - 3.0, c.y + 1.0),
        ],
        s,
    );
    p.line(
        vec![
            pos2(c.x - 0.8, c.y - 1.4),
            pos2(c.x - 3.2, c.y + 1.0),
            pos2(c.x - 0.8, c.y + 3.4),
        ],
        s,
    );
}

fn paint_copy_icon(p: &Painter, c: Pos2) {
    let s = Stroke::new(1.2, t::COPY_GRAY);
    let back = Rect::from_center_size(c - vec2(1.8, 1.8), Vec2::splat(9.0));
    let front = back.translate(Vec2::splat(3.6));
    p.rect_stroke(back, 2, s, StrokeKind::Middle);
    p.rect_filled(front, 2, t::BODY_BG);
    p.rect_stroke(front, 2, s, StrokeKind::Middle);
}

fn paint_magnifier(p: &Painter, c: Pos2, color: Color32) {
    let s = Stroke::new(1.3, color);
    p.circle_stroke(c - vec2(0.8, 0.8), 4.0, s);
    p.line_segment([c + vec2(2.2, 2.2), c + vec2(5.0, 5.0)], s);
}

/// 工具栏 “T”：描边表示粘贴为纯文本，实心表示保留格式。
fn paint_plain_icon(p: &Painter, r: Rect, plain: bool) {
    let r = r.shrink(1.0);
    let font = FontId::proportional(11.0);
    if plain {
        p.rect_stroke(
            r,
            3,
            Stroke::new(1.2, t::TOOL_ICON_COLOR),
            StrokeKind::Middle,
        );
        p.text(
            r.center(),
            Align2::CENTER_CENTER,
            "T",
            font,
            t::TOOL_ICON_COLOR,
        );
    } else {
        p.rect_filled(r, 3, t::TOOL_ICON_COLOR);
        p.text(r.center(), Align2::CENTER_CENTER, "T", font, t::TOOLBAR_BG);
    }
}

fn paint_settings_icon(p: &Painter, r: Rect) {
    let c = r.center();
    let s = Stroke::new(1.3, t::TOOL_ICON_COLOR);
    let hex: Vec<Pos2> = (0..6)
        .map(|i| {
            let a = std::f32::consts::FRAC_PI_3 * i as f32 + std::f32::consts::FRAC_PI_6;
            c + 7.0 * vec2(a.cos(), a.sin())
        })
        .collect();
    p.add(Shape::closed_line(hex, s));
    p.circle_stroke(c, 2.4, s);
}

fn paint_scroll_indicator(p: &Painter, area: Rect, scroll: f32, content_w: f32) {
    let view_w = area.width();
    if content_w <= view_w + 0.5 {
        return;
    }
    let track = Rect::from_min_max(
        pos2(area.left() + 16.0, area.bottom() - 7.0),
        pos2(area.right() - 16.0, area.bottom() - 4.0),
    );
    let thumb_w = (track.width() * view_w / content_w).max(24.0);
    let max_scroll = content_w - view_w;
    let x = track.left() + (track.width() - thumb_w) * (scroll / max_scroll).clamp(0.0, 1.0);
    let thumb = Rect::from_min_size(pos2(x, track.top()), vec2(thumb_w, track.height()));
    p.rect_filled(thumb, 2, Color32::from_white_alpha(70));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;

    const SCREEN: Vec2 = vec2(1728.0, 350.0);

    /// 无头驱动：每帧注入真实的输入事件，走与实机相同的 `show` 路径。
    struct Harness {
        dir: tempfile::TempDir,
        ctx: egui::Context,
        app: PickerApp,
        result: Rc<RefCell<Option<Pick>>>,
        pointer: Pos2,
    }

    impl Harness {
        fn new(cfg: Config, texts: &[&str]) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let mut store = Store::open(dir.path()).unwrap();
            for (i, text) in texts.iter().enumerate() {
                store
                    .insert(&Item::text(
                        (*text).into(),
                        None,
                        None,
                        now_secs() + i as f64,
                    ))
                    .unwrap();
            }
            let st = PickerState::new(store, cfg.clone(), Tag::All).unwrap();
            let ctx = egui::Context::default();
            let result = Rc::new(RefCell::new(None));
            let loc = Locations {
                data_dir: dir.path().to_owned(),
                config_file: dir.path().join("config.toml"),
            };
            let app = PickerApp::new(&ctx, st, cfg, loc, result.clone());
            let mut h = Harness {
                dir,
                ctx,
                app,
                result,
                pointer: pos2(-10.0, -10.0),
            };
            h.frame(vec![]);
            h.frame(vec![]);
            h
        }

        /// 跑一帧，返回这一帧是否请求了关闭窗口。
        fn frame(&mut self, events: Vec<egui::Event>) -> bool {
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, SCREEN)),
                events,
                ..Default::default()
            };
            let app = &mut self.app;
            let mut out = self.ctx.run_ui(input, |ui| app.show(ui));
            let closed = out
                .viewport_output
                .get(&egui::ViewportId::ROOT)
                .is_some_and(|v| {
                    v.commands
                        .iter()
                        .any(|c| matches!(c, egui::ViewportCommand::Close))
                });
            // 无头测试不上传纹理；egui 要求显式丢弃每帧的纹理增量。
            out.textures_delta.clear();
            closed
        }

        fn press(&mut self, key: Key, modifiers: Modifiers) -> bool {
            self.frame(vec![egui::Event::Key {
                key,
                physical_key: Some(key),
                pressed: true,
                repeat: false,
                modifiers,
            }])
        }

        fn type_text(&mut self, s: &str) {
            self.frame(vec![egui::Event::Text(s.into())]);
        }

        /// 指针刚进入窗口的第一次移动没有位移量，真实鼠标随后的移动才有，所以分两步走。
        fn move_to(&mut self, pos: Pos2) {
            self.pointer = pos;
            self.frame(vec![egui::Event::PointerMoved(pos - vec2(2.0, 2.0))]);
            self.frame(vec![egui::Event::PointerMoved(pos)]);
            self.frame(vec![]);
        }

        fn click(&mut self, button: egui::PointerButton) -> bool {
            let pos = self.pointer;
            let ev = |pressed| egui::Event::PointerButton {
                pos,
                button,
                pressed,
                modifiers: Modifiers::NONE,
            };
            let a = self.frame(vec![ev(true)]);
            let b = self.frame(vec![ev(false)]);
            a || b
        }

        fn picked(&self) -> Option<Pick> {
            self.result.borrow().clone()
        }

        fn selected_text(&self) -> Option<String> {
            self.app.st.selected_item().and_then(|i| i.text.clone())
        }
    }

    fn cfg() -> Config {
        Config {
            page_size: 20,
            ..Config::default()
        }
    }

    fn card_center(idx: usize) -> Pos2 {
        pos2(t::card_x(idx) + t::CARD / 2.0, t::CARD_TOP + 100.0)
    }

    #[test]
    fn up_down_move_selection_and_left_right_switch_groups() {
        let mut h = Harness::new(cfg(), &["one", "two", "three"]);
        assert_eq!(h.selected_text().as_deref(), Some("three"));
        h.press(Key::ArrowDown, Modifiers::NONE);
        assert_eq!(h.selected_text().as_deref(), Some("two"));
        h.press(Key::ArrowUp, Modifiers::NONE);
        assert_eq!(h.selected_text().as_deref(), Some("three"));
        h.press(Key::ArrowRight, Modifiers::NONE);
        assert_eq!(h.app.st.tag, Tag::Text);
        h.press(Key::ArrowLeft, Modifiers::NONE);
        h.press(Key::ArrowLeft, Modifiers::NONE);
        assert_eq!(h.app.st.tag, Tag::Favorite, "分类首尾相接");
    }

    #[test]
    fn enter_pastes_selected_item_as_plain_text_by_default() {
        let mut h = Harness::new(cfg(), &["one", "two"]);
        h.press(Key::ArrowDown, Modifiers::NONE);
        let id = h.app.st.selected_item().unwrap().id;
        assert!(h.press(Key::Enter, Modifiers::NONE), "选择后应关闭面板");
        assert_eq!(
            h.picked(),
            Some(Pick {
                id,
                action: Action::Paste,
                plain: true,
                transform: Transform::None
            })
        );
    }

    #[test]
    fn escape_cancels_and_remembers_the_group() {
        let mut h = Harness::new(cfg(), &["one"]);
        h.press(Key::Tab, Modifiers::NONE);
        assert!(h.press(Key::Escape, Modifiers::NONE));
        assert_eq!(h.picked(), None);
        assert_eq!(super::super::load_last_tag(h.dir.path()), Some(Tag::Text));
    }

    #[test]
    fn bare_digit_pastes_nth_item_and_typing_does_not_search() {
        let mut h = Harness::new(cfg(), &["one", "two", "three"]);
        h.type_text("t");
        assert_eq!(h.app.st.search(), "", "搜索框没有焦点时输入不进入搜索");
        let expect = h.app.st.item_at(1).unwrap().id;
        assert!(h.press(Key::Num2, Modifiers::NONE));
        assert_eq!(h.picked().unwrap().id, expect);
    }

    #[test]
    fn digit_beyond_list_does_nothing() {
        let mut h = Harness::new(cfg(), &["one"]);
        assert!(!h.press(Key::Num5, Modifiers::NONE));
        assert_eq!(h.picked(), None);
    }

    #[test]
    fn modifier_digits_when_configured() {
        let mut h = Harness::new(
            Config {
                number_modifier: "alt".into(),
                ..cfg()
            },
            &["one", "two"],
        );
        assert!(!h.press(Key::Num1, Modifiers::NONE));
        assert!(h.press(Key::Num1, Modifiers::ALT));
    }

    #[test]
    fn command_f_focuses_search_and_enter_or_escape_still_work() {
        let mut h = Harness::new(cfg(), &["apple", "banana", "apricot"]);
        h.press(Key::F, Modifiers::COMMAND);
        h.frame(vec![]);
        h.type_text("ap");
        assert_eq!(h.app.st.search(), "ap");
        assert_eq!(h.app.st.items.len(), 2);
        // 搜索框里数字和方向键属于输入。
        h.press(Key::ArrowDown, Modifiers::NONE);
        assert_eq!(h.app.st.selected, 0);
        assert!(!h.press(Key::Num1, Modifiers::NONE));
        let id = h.app.st.selected_item().unwrap().id;
        assert!(h.press(Key::Enter, Modifiers::NONE));
        assert_eq!(h.picked().unwrap().id, id);

        let mut h = Harness::new(cfg(), &["apple"]);
        h.press(Key::F, Modifiers::COMMAND);
        h.frame(vec![]);
        assert!(
            h.press(Key::Escape, Modifiers::NONE),
            "搜索框里 Esc 直接关闭面板"
        );
    }

    #[test]
    fn tab_cycles_groups_even_from_the_search_field() {
        let mut h = Harness::new(cfg(), &["one"]);
        h.press(Key::F, Modifiers::COMMAND);
        h.press(Key::Tab, Modifiers::NONE);
        assert_eq!(h.app.st.tag, Tag::Text);
        h.press(Key::Tab, Modifiers::SHIFT);
        assert_eq!(h.app.st.tag, Tag::All);
    }

    #[test]
    fn delete_key_asks_for_confirmation() {
        let mut h = Harness::new(cfg(), &["one", "two"]);
        h.press(Key::Delete, Modifiers::NONE);
        assert_eq!(h.app.st.items.len(), 2, "先弹确认");
        assert!(!h.press(Key::Escape, Modifiers::NONE), "Esc 只关确认框");
        assert_eq!(h.app.st.items.len(), 2);
        h.press(Key::Backspace, Modifiers::NONE);
        h.press(Key::Enter, Modifiers::NONE);
        assert_eq!(h.app.st.items.len(), 1);
        assert_eq!(h.picked(), None, "确认删除用的 Enter 不能顺带粘贴");
    }

    #[test]
    fn delete_without_confirmation_when_disabled() {
        let mut h = Harness::new(
            Config {
                confirm_delete: false,
                ..cfg()
            },
            &["one", "two"],
        );
        h.press(Key::Delete, Modifiers::NONE);
        assert_eq!(h.app.st.items.len(), 1);
    }

    #[test]
    fn hovering_a_card_selects_it_and_click_pastes_it() {
        let mut h = Harness::new(cfg(), &["one", "two", "three"]);
        h.move_to(card_center(2));
        assert_eq!(h.app.st.selected, 2);
        let id = h.app.st.items[2].id;
        assert!(h.click(egui::PointerButton::Primary));
        assert_eq!(
            h.picked().unwrap(),
            Pick {
                id,
                action: Action::Paste,
                plain: true,
                transform: Transform::None
            }
        );
    }

    #[test]
    fn select_click_action_needs_double_click() {
        let c = Config {
            click_action: ClickAction::Select,
            double_click_action: Action::Copy,
            ..cfg()
        };
        let mut h = Harness::new(c, &["one", "two"]);
        h.move_to(card_center(1));
        assert!(!h.click(egui::PointerButton::Primary));
        assert!(h.click(egui::PointerButton::Primary), "第二次点击构成双击");
        assert_eq!(h.picked().unwrap().action, Action::Copy);
    }

    #[test]
    fn copy_button_copies_without_paste() {
        let mut h = Harness::new(cfg(), &["one"]);
        let card = Rect::from_min_size(pos2(t::card_x(0), t::CARD_TOP), Vec2::splat(t::CARD));
        h.move_to(pos2(card.right() - 16.0, card.bottom() - t::FOOT_H / 2.0));
        assert!(h.click(egui::PointerButton::Primary));
        assert_eq!(h.picked().unwrap().action, Action::Copy);
    }

    #[test]
    fn hovering_a_group_pill_switches_group() {
        let mut h = Harness::new(cfg(), &["one"]);
        let pill = pill_rect(&h, Tag::Image);
        h.move_to(pill.center());
        assert_eq!(h.app.st.tag, Tag::Image);

        let mut h = Harness::new(
            Config {
                hover_switch_group: false,
                ..cfg()
            },
            &["one"],
        );
        h.move_to(pill.center());
        assert_eq!(h.app.st.tag, Tag::All);
        h.click(egui::PointerButton::Primary);
        assert_eq!(h.app.st.tag, Tag::Image);
    }

    fn pill_rect(h: &Harness, tag: Tag) -> Rect {
        let painter = Painter::new(h.ctx.clone(), egui::LayerId::background(), Rect::EVERYTHING);
        let widths: Vec<f32> = Tag::ORDER
            .iter()
            .map(|t| text_width(&painter, h.app.lang.tag(*t), t::PILL_FONT) + 2.0 * t::PILL_PAD)
            .collect();
        let total = t::SEARCH_W + t::SEARCH_GAP + widths.iter().sum::<f32>() + t::PILL_GAP * 4.0;
        let mut x = SCREEN.x / 2.0 - total / 2.0 + t::SEARCH_W + t::SEARCH_GAP;
        for (t, w) in Tag::ORDER.into_iter().zip(widths) {
            if t == tag {
                return Rect::from_min_size(pos2(x, 25.0 - t::PILL_H / 2.0), vec2(w, t::PILL_H));
            }
            x += w + t::PILL_GAP;
        }
        unreachable!()
    }

    #[test]
    fn plain_toggle_changes_the_pick() {
        let mut h = Harness::new(cfg(), &["one"]);
        let settings_x = SCREEN.x - t::TOOL_RIGHT - t::TOOL_ICON / 2.0;
        h.move_to(pos2(settings_x - t::TOOL_ICON - t::TOOL_GAP, 25.0));
        h.click(egui::PointerButton::Primary);
        assert!(!h.app.plain);
        h.press(Key::Enter, Modifiers::NONE);
        assert!(!h.picked().unwrap().plain);
    }

    #[test]
    fn menu_actions() {
        let mut h = Harness::new(cfg(), &["a\nb", "two"]);
        let ctx = h.ctx.clone();
        h.app.apply_menu(&ctx, 1, MenuAction::ToggleFavorite);
        assert!(h.app.st.items[1].favorite);
        h.app.apply_menu(
            &ctx,
            0,
            MenuAction::Clear {
                keep_favorites: true,
            },
        );
        h.press(Key::Enter, Modifiers::NONE);
        assert_eq!(h.app.st.items.len(), 1, "清空后只剩收藏");
        assert_eq!(h.picked(), None);
        let pick = MenuAction::Pick {
            action: Action::Copy,
            plain: true,
            transform: Transform::StripNewlines,
        };
        h.app.apply_menu(&ctx, 0, pick);
        assert_eq!(h.picked().unwrap().transform, Transform::StripNewlines);
    }

    #[test]
    fn wheel_scrolls_horizontally_and_pages_in_more_items() {
        let texts: Vec<String> = (0..30).map(|i| format!("item {i}")).collect();
        let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
        let mut h = Harness::new(
            Config {
                page_size: 10,
                ..cfg()
            },
            &refs,
        );
        assert_eq!(h.app.st.items.len(), 10);
        h.move_to(card_center(1));
        for _ in 0..30 {
            h.frame(vec![egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: vec2(0.0, -400.0),
                phase: egui::TouchPhase::Move,
                modifiers: Modifiers::NONE,
                source: egui::MouseWheelSource::Wheel,
            }]);
        }
        assert!(h.app.scroll > 0.0);
        assert!(h.app.st.items.len() > 10, "滚到末尾附近时加载下一页");
    }

    #[test]
    fn keyboard_selection_keeps_card_in_view() {
        let texts: Vec<String> = (0..12).map(|i| format!("item {i}")).collect();
        let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
        let mut h = Harness::new(cfg(), &refs);
        for _ in 0..10 {
            h.press(Key::ArrowDown, Modifiers::NONE);
        }
        let right = t::card_x(10) + t::CARD;
        assert!(right - h.app.scroll <= SCREEN.x, "选中卡片完整可见");
        assert!(h.app.scroll > 0.0);
    }

    #[test]
    fn enter_on_empty_history_is_harmless() {
        let mut h = Harness::new(cfg(), &[]);
        assert!(!h.press(Key::Enter, Modifiers::NONE));
        assert_eq!(h.picked(), None);
    }

    #[test]
    fn previews_are_bounded() {
        let long = "x\n".repeat(1000);
        let p = preview_text(&long);
        assert_eq!(p.lines().count(), PREVIEW_LINES);
        assert_eq!(preview_text("a\tb\r\n"), "a    b\n");
    }

    #[test]
    fn byte_sizes_use_two_decimals() {
        assert_eq!(format_bytes(999), "999 B");
        assert_eq!(format_bytes(40_010), "40.01 KB");
        assert_eq!(format_bytes(1_330_000), "1.33 MB");
    }

    #[test]
    fn image_fits_top_left_without_upscaling() {
        let bounds = Rect::from_min_size(pos2(10.0, 20.0), vec2(100.0, 100.0));
        assert_eq!(
            fit_top_left(bounds, vec2(200.0, 100.0)),
            Rect::from_min_size(pos2(10.0, 20.0), vec2(100.0, 50.0))
        );
        assert_eq!(
            fit_top_left(bounds, vec2(20.0, 10.0)).size(),
            vec2(20.0, 10.0)
        );
        assert_eq!(fit_top_left(bounds, Vec2::ZERO), bounds);
    }

    #[test]
    fn file_names_from_paths() {
        assert_eq!(file_name("/a/b/report.pdf"), "report.pdf");
        assert_eq!(file_name("/a/dir/"), "dir");
        assert_eq!(file_name("/"), "/");
    }
}
