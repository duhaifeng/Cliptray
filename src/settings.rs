//! 设置窗口。对应 iCopy 点菜单栏图标后的 Setting。
//!
//! 侧边栏只放 splay 已经有行为的页：剪切板、通用设置、黑名单。
//! 免费 Apps、购买是推广；快捷回复、备份、剪切板堆栈这三项功能本身还没有复刻。
//! 外观见 `settings_ui`。

use anyhow::{Result, anyhow};
use eframe::egui::{
    self, Color32, CornerRadius, RichText, ScrollArea, Sense, Stroke, StrokeKind, TextEdit, vec2,
};

use crate::autostart;
use crate::config::{Action, ClickAction, Config, Language, Paths};
use crate::hotkey::HotkeySpec;
use crate::i18n::{Lang, MenuWord, Text};
use crate::ipc::{self, Command};
use crate::model::Tag;
use crate::picker;
use crate::settings_ui::{self as ui_kit, Glyph, Recorded};
use crate::store::Store;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Page {
    Clipboard,
    General,
    Blacklist,
}

impl Page {
    const ALL: [Page; 3] = [Page::Clipboard, Page::General, Page::Blacklist];

    fn word(self) -> MenuWord {
        match self {
            Page::Clipboard => MenuWord::Clipboard,
            Page::General => MenuWord::General,
            Page::Blacklist => MenuWord::Blacklist,
        }
    }

    /// 侧栏图标和颜色，取自 iCopy 侧栏截图里各项的主色。
    fn badge(self) -> (Glyph, Color32) {
        match self {
            Page::Clipboard => (Glyph::Clipboard, Color32::from_rgb(0xE8, 0x5D, 0x96)),
            Page::General => (Glyph::Gear, Color32::from_rgb(0x1F, 0x8F, 0xF2)),
            Page::Blacklist => (Glyph::Stop, Color32::from_rgb(0xF0, 0x44, 0x4F)),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Confirm {
    Restore,
    Initialize,
}

/// 存储时间的档位。天数是按 iCopy 文案（天/周/月/季度/半年/年/无限制）取的整档，
/// 滑块上的顺序和 iCopy 一致：无限制在最右。刻度的精确含义没有从界面里读到。
const HISTORY: &[u32] = &[1, 7, 30, 90, 180, 365, 0];
/// 磁盘上限（MiB）。1GB 是 iCopy 设置页上看到的当前值，其余档位是常用步进。
const DISK_MB: &[u64] = &[0, 100, 512, 1024, 2048, 5120];

pub fn run(cfg: Config, paths: Paths) -> Result<()> {
    let viewport = egui::ViewportBuilder::default()
        .with_title("splay")
        .with_app_id("splay-settings")
        .with_inner_size([920.0, 780.0])
        .with_min_inner_size([760.0, 560.0])
        .with_resizable(true)
        .with_maximize_button(false)
        // macOS：内容延伸到标题栏下面，红黄绿按钮浮在侧栏上，和 iCopy 一样。其他系统忽略这几项。
        .with_fullsize_content_view(true)
        .with_titlebar_shown(false)
        .with_title_shown(false)
        // 其他系统：不用系统标题栏（Wayland 下 GNOME 没有服务端装饰，补画的又小又看不清），
        // 在界面里自己画拖动条和关闭按钮，见 `ui_kit::window_controls`。
        .with_decorations(cfg!(target_os = "macos"));
    let options = eframe::NativeOptions {
        viewport,
        event_loop_builder: Some(Box::new(|_builder| {
            #[cfg(target_os = "macos")]
            {
                use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS};
                _builder.with_activation_policy(ActivationPolicy::Accessory);
            }
        })),
        ..Default::default()
    };
    eframe::run_native(
        "splay",
        options,
        Box::new(move |cc| {
            let have_cjk = picker::install_system_fonts(&cc.egui_ctx);
            let lang = Lang::resolve(
                cfg.language,
                crate::platform::ui_language().as_deref(),
                have_cjk,
            );
            ui_kit::apply_visuals(&cc.egui_ctx);
            Ok(Box::new(SettingsApp::new(cfg, paths, lang, have_cjk)))
        }),
    )
    .map_err(|e| anyhow!("无法打开设置窗口: {e}"))?;
    Ok(())
}

struct SettingsApp {
    cfg: Config,
    paths: Paths,
    lang: Lang,
    have_cjk: bool,
    page: Page,
    /// 快捷键按钮是否在等按键。
    recording: bool,
    /// 黑名单里选中的行和待添加的应用名。
    blacklist_selected: Option<usize>,
    blacklist_input: String,
    confirm: Option<Confirm>,
    error: Option<String>,
    focused_once: bool,
}

impl SettingsApp {
    fn new(cfg: Config, paths: Paths, lang: Lang, have_cjk: bool) -> Self {
        SettingsApp {
            cfg,
            paths,
            lang,
            have_cjk,
            page: Page::Clipboard,
            recording: false,
            blacklist_selected: None,
            blacklist_input: String::new(),
            confirm: None,
            error: None,
            focused_once: false,
        }
    }

    fn word(&self, word: MenuWord) -> &'static str {
        self.lang.menu(word)
    }

    fn persist(&mut self, launch_changed: bool) {
        if let Err(e) = self.cfg.save(&self.paths.config_file) {
            self.error = Some(format!("{e:#}"));
            return;
        }
        if launch_changed
            && let Err(e) = std::env::current_exe()
                .map_err(anyhow::Error::from)
                .and_then(|p| autostart::apply(self.cfg.launch_at_startup, &p))
        {
            self.error = Some(format!("{e:#}"));
            return;
        }
        self.error = None;
        // 守护进程没在运行不算错误；它在运行却拒绝了（例如热键被占用），要让用户看到。
        if let Ok(reply) = ipc::send(&self.paths.socket, Command::Reload)
            && let Some(msg) = reply.strip_prefix("error: ")
        {
            self.error = Some(msg.to_owned());
        }
    }

    fn set_language(&mut self, language: Language) {
        self.cfg.language = language;
        self.lang = Lang::resolve(
            language,
            crate::platform::ui_language().as_deref(),
            self.have_cjk,
        );
        self.persist(false);
    }

    fn set_hotkey(&mut self, spec: &str) {
        if spec == self.cfg.hotkey {
            return;
        }
        if let Err(e) = spec.parse::<HotkeySpec>() {
            self.error = Some(format!("{e:#}"));
            return;
        }
        let previous = std::mem::replace(&mut self.cfg.hotkey, spec.to_owned());
        self.persist(false);
        if self.error.is_some() {
            // 新热键没能生效，守护进程还在用旧的：配置文件和界面也退回去，下次启动才不会再失败。
            self.cfg.hotkey = previous;
            let _ = self.cfg.save(&self.paths.config_file);
        }
    }

    fn hotkey_label(&self) -> String {
        match self.cfg.hotkey.parse::<HotkeySpec>() {
            Ok(spec) => spec.label(cfg!(target_os = "macos")),
            Err(_) => self.cfg.hotkey.clone(),
        }
    }

    fn add_blacklist_entry(&mut self) {
        let name = std::mem::take(&mut self.blacklist_input);
        if push_blacklist(&mut self.cfg.blacklist, &name) {
            self.blacklist_selected = None;
            self.persist(false);
        }
    }

    fn remove_blacklist_entry(&mut self) {
        if let Some(i) = self.blacklist_selected.take()
            && i < self.cfg.blacklist.len()
        {
            self.cfg.blacklist.remove(i);
            self.persist(false);
        }
    }

    fn restore_default(&mut self) {
        self.cfg = Config::default();
        self.blacklist_selected = None;
        self.lang = Lang::resolve(
            self.cfg.language,
            crate::platform::ui_language().as_deref(),
            self.have_cjk,
        );
        self.persist(true);
    }

    fn initialize(&mut self) {
        self.restore_default();
        if let Ok(mut store) = Store::open(&self.paths.data_dir)
            && let Err(e) = store.clear(Tag::All, false)
        {
            self.error = Some(format!("{e:#}"));
        }
    }

    fn sidebar(&mut self, ui: &mut egui::Ui) {
        // macOS 的红黄绿按钮浮在这里，留出位置。
        ui.add_space(if cfg!(target_os = "macos") {
            44.0
        } else {
            18.0
        });
        ui.vertical_centered(|ui| {
            let (rect, _) = ui.allocate_exact_size(vec2(66.0, 66.0), Sense::hover());
            ui_kit::app_logo(ui.painter(), rect);
            ui.add_space(6.0);
            ui.label(
                RichText::new(format!("splay: {}", env!("CARGO_PKG_VERSION")))
                    .size(13.0)
                    .color(ui_kit::TEXT),
            );
        });
        ui.add_space(46.0);
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            ui.add_space(0.0);
            egui::Frame::new()
                .inner_margin(egui::Margin::symmetric(10, 0))
                .show(ui, |ui| {
                    for page in Page::ALL {
                        let (glyph, color) = page.badge();
                        let title = self.lang.menu(page.word());
                        if ui_kit::nav_item(ui, self.page == page, glyph, color, title).clicked() {
                            self.page = page;
                        }
                    }
                });
        });
    }
}

impl eframe::App for SettingsApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if !self.focused_once {
            self.focused_once = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(
                self.word(MenuWord::Setting).to_owned(),
            ));
        }
        egui::Panel::left("nav")
            .exact_size(ui_kit::SIDEBAR_W)
            .resizable(false)
            .show_separator_line(false)
            .frame(egui::Frame::new().fill(ui_kit::SIDEBAR_BG))
            .show(ui, |ui| self.sidebar(ui));

        let mut confirm_choice: Option<Option<Confirm>> = None;
        if !cfg!(target_os = "macos") {
            ui_kit::window_controls(&ctx);
        }
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(ui_kit::CONTENT_BG)
                    .inner_margin(egui::Margin {
                        left: 26,
                        right: 26,
                        top: if cfg!(target_os = "macos") {
                            30
                        } else {
                            ui_kit::TITLE_BAR_H as i8
                        },
                        bottom: 20,
                    }),
            )
            .show(ui, |ui| {
                if let Some(err) = &self.error {
                    ui.colored_label(ui_kit::ERROR, err);
                }
                ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 10.0;
                        match self.page {
                            Page::Clipboard => clipboard_page(self, ui),
                            Page::General => general_page(self, ui),
                            Page::Blacklist => blacklist_page(self, ui),
                        }
                    });
            });

        if let Some(kind) = self.confirm {
            let message = self.lang.menu(match kind {
                Confirm::Restore => MenuWord::SureRestore,
                Confirm::Initialize => MenuWord::SureInitialize,
            });
            let sure = self.lang.text(Text::Sure).to_owned();
            let cancel = self.lang.text(Text::Cancel).to_owned();
            let title = self.lang.text(Text::KindTips);
            egui::Modal::new(egui::Id::unique("settings-confirm")).show(&ctx, |ui| {
                ui.set_min_width(360.0);
                ui.label(RichText::new(title).strong());
                ui.add_space(6.0);
                ui.label(message);
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui_kit::pill_button(ui, &sure, true).clicked() {
                        confirm_choice = Some(Some(kind));
                    }
                    if ui_kit::pill_button(ui, &cancel, true).clicked() {
                        confirm_choice = Some(None);
                    }
                });
            });
        }
        if let Some(choice) = confirm_choice {
            self.confirm = None;
            match choice {
                Some(Confirm::Restore) => self.restore_default(),
                Some(Confirm::Initialize) => self.initialize(),
                None => {}
            }
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(200));
    }
}

fn clipboard_page(app: &mut SettingsApp, ui: &mut egui::Ui) {
    ui_kit::section_header(
        ui,
        Glyph::Command,
        Color32::from_rgb(0xF2, 0x8C, 0x28),
        app.word(MenuWord::ShowHidePanel),
    );
    ui_kit::card(ui, |ui| {
        let label = app.hotkey_label();
        let hint = app.word(MenuWord::PressShortcut);
        ui_kit::form_row(ui, app.word(MenuWord::ShortcutKey), |ui| {
            if let Recorded::Spec(spec) =
                ui_kit::shortcut_button(ui, &label, hint, &mut app.recording)
            {
                app.set_hotkey(&spec);
            }
        });
        // Linux 上换热键当场生效；macOS 还要重启守护进程。
        if cfg!(target_os = "macos") {
            ui.horizontal(|ui| {
                ui.add_space(ui_kit::LABEL_W + 14.0);
                ui_kit::weak(ui, app.word(MenuWord::ShortcutRestart));
            });
        }
    });

    ui_kit::section_header(
        ui,
        Glyph::Sliders,
        Color32::from_rgb(0x8E, 0x5B, 0xE0),
        app.word(MenuWord::InteractionSettings),
    );
    ui_kit::card(ui, |ui| {
        let modifiers = [
            ("none".to_owned(), app.word(MenuWord::Nothing).to_owned()),
            ("alt".to_owned(), "Alt".to_owned()),
            ("ctrl".to_owned(), "Ctrl".to_owned()),
        ];
        ui_kit::form_row(ui, app.word(MenuWord::NumberShortcut), |ui| {
            if let Some(v) =
                ui_kit::dropdown(ui, "num", &app.cfg.number_modifier, &modifiers, 150.0)
            {
                app.cfg.number_modifier = v;
                app.persist(false);
            }
            ui_kit::weak(ui, "+ 1~9");
        });
        let clicks = [
            (
                ClickAction::Paste,
                app.word(MenuWord::InputContent).to_owned(),
            ),
            (
                ClickAction::Copy,
                app.word(MenuWord::CopyContent).to_owned(),
            ),
            (
                ClickAction::Select,
                app.word(MenuWord::SelectOnly).to_owned(),
            ),
        ];
        ui_kit::form_row(ui, app.word(MenuWord::OnPress), |ui| {
            if let Some(v) = ui_kit::dropdown(ui, "press", &app.cfg.click_action, &clicks, 190.0) {
                app.cfg.click_action = v;
                app.persist(false);
            }
        });
        let actions = [
            (Action::Paste, app.word(MenuWord::InputContent).to_owned()),
            (Action::Copy, app.word(MenuWord::CopyContent).to_owned()),
        ];
        ui_kit::form_row(ui, app.word(MenuWord::DoubleClickEnter), |ui| {
            if let Some(v) = ui_kit::dropdown(ui, "enter", &app.cfg.enter_action, &actions, 190.0) {
                app.cfg.enter_action = v;
                app.cfg.double_click_action = v;
                app.persist(false);
            }
        });
        let groups = group_options(app.lang);
        ui_kit::form_row(ui, app.word(MenuWord::DefaultGroup), |ui| {
            if let Some(v) = ui_kit::dropdown(ui, "group", &app.cfg.default_tag, &groups, 190.0) {
                app.cfg.default_tag = v;
                app.persist(false);
            }
        });
        check(ui, app, MenuWord::AutoClose, |c| &mut c.auto_hide, false);
        check(
            ui,
            app,
            MenuWord::HoverSwitch,
            |c| &mut c.hover_switch_group,
            false,
        );
        check(
            ui,
            app,
            MenuWord::PastePlain,
            |c| &mut c.paste_plain_text,
            false,
        );
    });

    ui_kit::section_header(
        ui,
        Glyph::Clock,
        Color32::from_rgb(0x2F, 0xB5, 0xC8),
        app.word(MenuWord::HistoryLimit),
    );
    ui_kit::card(ui, |ui| {
        let disks = disk_options(app.cfg.disk_limit_mb, app.lang);
        ui_kit::form_row(ui, app.word(MenuWord::DiskLimit), |ui| {
            if let Some(v) = ui_kit::dropdown(ui, "disk", &app.cfg.disk_limit_mb, &disks, 130.0) {
                app.cfg.disk_limit_mb = v;
                app.persist(false);
            }
        });
        let stops = history_options(app.cfg.history_days, app.lang);
        ui_kit::form_row(ui, app.word(MenuWord::TimeLimit), |ui| {
            if let Some(v) = ui_kit::history_slider(ui, &stops, app.cfg.history_days) {
                app.cfg.history_days = v;
                app.persist(false);
            }
        });
    });
}

fn general_page(app: &mut SettingsApp, ui: &mut egui::Ui) {
    ui_kit::section_header(
        ui,
        Glyph::Display,
        Color32::from_rgb(0xF0, 0x48, 0x5A),
        app.word(MenuWord::StartupDisplay),
    );
    ui_kit::card(ui, |ui| {
        ui.horizontal(|ui| {
            check(
                ui,
                app,
                MenuWord::LaunchAtStartup,
                |c| &mut c.launch_at_startup,
                true,
            );
            ui_kit::weak(ui, app.word(MenuWord::LaunchHint));
        });
        check(
            ui,
            app,
            MenuWord::ShowMenuIcon,
            |c| &mut c.show_status_item,
            false,
        );
        check(
            ui,
            app,
            MenuWord::ConfirmDelete,
            |c| &mut c.confirm_delete,
            false,
        );
        let languages = [
            (Language::Auto, app.word(MenuWord::FollowSystem).to_owned()),
            (Language::Zh, "中文".to_owned()),
            (Language::En, "English".to_owned()),
        ];
        ui_kit::form_row(ui, app.word(MenuWord::Language), |ui| {
            if let Some(v) = ui_kit::dropdown(ui, "lang", &app.cfg.language, &languages, 160.0) {
                app.set_language(v);
            }
        });
    });
    ui.allocate_ui_with_layout(
        vec2(ui.available_width(), 30.0),
        egui::Layout::right_to_left(egui::Align::Center),
        |ui| {
            if ui_kit::pill_button(ui, app.word(MenuWord::Initialize), true).clicked() {
                app.confirm = Some(Confirm::Initialize);
            }
            if ui_kit::pill_button(ui, app.word(MenuWord::RestoreDefault), true).clicked() {
                app.confirm = Some(Confirm::Restore);
            }
        },
    );
}

fn blacklist_page(app: &mut SettingsApp, ui: &mut egui::Ui) {
    ui_kit::card(ui, |ui| {
        ui.label(
            RichText::new(app.word(MenuWord::BlacklistTitle))
                .size(14.0)
                .strong()
                .color(ui_kit::TEXT),
        );
        ui_kit::weak(ui, app.word(MenuWord::BlacklistHelp));

        // 列表区：圆角描边框，选中行用蓝色高亮。
        let height = 280.0;
        let (outer, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
        ui.painter().rect_stroke(
            outer,
            CornerRadius::same(10),
            Stroke::new(1.0, Color32::from_gray(0x48)),
            StrokeKind::Inside,
        );
        let mut list_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(outer.shrink(4.0))
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        ScrollArea::vertical()
            .id_salt("blacklist-rows")
            .auto_shrink([false, false])
            .show(&mut list_ui, |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                for (i, name) in app.cfg.blacklist.iter().enumerate() {
                    let (row, resp) =
                        ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::click());
                    if app.blacklist_selected == Some(i) {
                        ui.painter()
                            .rect_filled(row, CornerRadius::same(6), ui_kit::CHECK_BLUE);
                    } else if resp.hovered() {
                        ui.painter().rect_filled(
                            row,
                            CornerRadius::same(6),
                            Color32::from_white_alpha(12),
                        );
                    }
                    ui.painter().text(
                        row.left_center() + vec2(10.0, 0.0),
                        egui::Align2::LEFT_CENTER,
                        name,
                        egui::FontId::proportional(14.0),
                        ui_kit::TEXT,
                    );
                    if resp.clicked() {
                        app.blacklist_selected = Some(i);
                    }
                }
            });

        ui.horizontal(|ui| {
            let input = ui.add(
                TextEdit::singleline(&mut app.blacklist_input)
                    .hint_text(app.lang.menu(MenuWord::AppNameHint))
                    .desired_width(240.0)
                    .margin(vec2(8.0, 6.0)),
            );
            let enter = input.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let can_add = !app.blacklist_input.trim().is_empty();
            let add_clicked = ui_kit::pill_button(ui, "+", can_add).clicked();
            if can_add && (add_clicked || enter) {
                app.add_blacklist_entry();
            }
            if ui_kit::pill_button(ui, "−", app.blacklist_selected.is_some()).clicked() {
                app.remove_blacklist_entry();
            }
        });
    });
}

fn check(
    ui: &mut egui::Ui,
    app: &mut SettingsApp,
    word: MenuWord,
    field: impl Fn(&mut Config) -> &mut bool,
    launch: bool,
) {
    let label = app.lang.menu(word);
    let mut value = *field(&mut app.cfg);
    if ui_kit::check_box(ui, &mut value, label) {
        *field(&mut app.cfg) = value;
        app.persist(launch);
    }
}

fn group_options(lang: Lang) -> Vec<(String, String)> {
    let mut out = vec![("last".to_owned(), lang.menu(MenuWord::LastGroup).to_owned())];
    for tag in Tag::ORDER {
        out.push((tag.as_str().to_owned(), lang.tag(tag).to_owned()));
    }
    out
}

fn disk_options(current: u64, lang: Lang) -> Vec<(u64, String)> {
    let mut values: Vec<u64> = DISK_MB.to_vec();
    if !values.contains(&current) {
        values.push(current);
        values.sort_unstable();
    }
    values
        .into_iter()
        .map(|mb| (mb, disk_label(mb, lang)))
        .collect()
}

/// 滑块的刻度。当前值不在预设档位里时（手改配置文件）追加一档，免得滑块上看不到当前值。
fn history_options(current: u32, lang: Lang) -> Vec<(u32, String)> {
    let mut values: Vec<u32> = HISTORY.to_vec();
    if !values.contains(&current) {
        let at = values.iter().position(|&d| d == 0).unwrap_or(values.len());
        values.insert(at, current);
    }
    values
        .into_iter()
        .map(|days| (days, history_label(days, lang)))
        .collect()
}

/// 添加一个应用名：去掉首尾空白，忽略空串和不分大小写的重复。返回是否真的加入了。
pub fn push_blacklist(list: &mut Vec<String>, name: &str) -> bool {
    let name = name.trim();
    if name.is_empty() || list.iter().any(|n| n.eq_ignore_ascii_case(name)) {
        return false;
    }
    list.push(name.to_owned());
    true
}

pub fn history_label(days: u32, lang: Lang) -> String {
    let word = match days {
        0 => MenuWord::Unlimited,
        1 => MenuWord::Day,
        7 => MenuWord::Week,
        30 => MenuWord::Month,
        90 => MenuWord::Quarter,
        180 => MenuWord::HalfYear,
        365 => MenuWord::Year,
        _ => return format!("{days}"),
    };
    lang.menu(word).to_owned()
}

pub fn disk_label(mb: u64, lang: Lang) -> String {
    match mb {
        0 => lang.menu(MenuWord::Unlimited).to_owned(),
        100 => "100MB".into(),
        512 => "512MB".into(),
        1024 => "1GB".into(),
        2048 => "2GB".into(),
        5120 => "5GB".into(),
        other => format!("{other}MB"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blacklist_trims_and_ignores_blanks_and_duplicates() {
        let mut list = vec!["KeePassXC".to_owned()];
        assert!(push_blacklist(&mut list, "  1Password "));
        assert!(!push_blacklist(&mut list, "   "));
        assert!(!push_blacklist(&mut list, "keepassxc"));
        assert_eq!(list, vec!["KeePassXC".to_owned(), "1Password".to_owned()]);
    }

    #[test]
    fn disk_label_matches_the_observed_one_gigabyte() {
        assert_eq!(disk_label(1024, Lang::En), "1GB");
        assert_eq!(disk_label(0, Lang::Zh), "无限制");
    }

    #[test]
    fn history_stops_end_with_unlimited_like_icopy() {
        assert_eq!(HISTORY, &[1, 7, 30, 90, 180, 365, 0]);
        assert_eq!(DISK_MB[3], 1024);
        let stops = history_options(7, Lang::En);
        assert_eq!(stops.first().unwrap().1, "Day");
        assert_eq!(stops.last().unwrap().1, "Unlimited");
        assert_eq!(stops[1].1, "Week");
    }

    #[test]
    fn unusual_history_value_gets_its_own_stop_before_unlimited() {
        let stops = history_options(14, Lang::En);
        assert_eq!(stops.len(), HISTORY.len() + 1);
        assert_eq!(stops[stops.len() - 2].0, 14);
        assert_eq!(stops.last().unwrap().0, 0);
    }
}
