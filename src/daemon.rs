//! 常驻守护进程。
//!
//! 单线程拥有剪切板、历史库和监控状态；热键线程、IPC 线程只把事件送进同一个通道。
//! 选择面板是独立的短命子进程：Linux 的剪切板内容归写入它的进程所有，进程退出内容就没了，
//! 所以“把选中的条目写进剪切板”必须由常驻的守护进程来做，面板只负责返回选择。

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command as Process, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

use crate::appicon::IconCache;
use crate::clipboard::{ClipboardBackend, SystemClipboard};
use crate::config::{Action, Config, Paths};
use crate::hotkey::HotkeySpec;
use crate::i18n::{Lang, MenuWord};
use crate::ipc::{self, Command, Pick, Request};
use crate::monitor::{ActiveApp, Captured, Monitor, SkipReason, item_to_content};
use crate::platform::{self, MenuAction, StatusIcon, TrayControl, TrayIo, TrayLabels};
use crate::store::{Store, now_secs};

static SHUTDOWN: AtomicBool = AtomicBool::new(false);

extern "C" fn on_signal(_: libc::c_int) {
    SHUTDOWN.store(true, Ordering::SeqCst);
}

type PasteFn = Box<dyn FnMut(&HotkeySpec) -> Result<()>>;

/// 守护进程的状态与对剪切板 / 历史库的操作，不含线程与子进程，便于单测。
pub struct Core<B: ClipboardBackend> {
    pub cfg: Config,
    pub store: Store,
    pub backend: B,
    pub monitor: Monitor,
    pub app: Box<dyn ActiveApp>,
    pub icons: IconCache,
    paste: PasteFn,
}

impl<B: ClipboardBackend> Core<B> {
    pub fn new(
        cfg: Config,
        store: Store,
        backend: B,
        app: Box<dyn ActiveApp>,
        paste: PasteFn,
    ) -> Self {
        Core {
            cfg,
            store,
            backend,
            monitor: Monitor::new(),
            app,
            icons: IconCache::disabled(),
            paste,
        }
    }

    pub fn tick(&mut self, now: f64) {
        match self.monitor.poll(
            &mut self.backend,
            &mut self.app,
            &mut self.store,
            &self.cfg,
            now,
        ) {
            Ok(Captured::Stored(o)) => {
                eprintln!("已记录 #{}", o.id());
                if let Ok(Some(app)) = self.store.get(o.id()).map(|i| i.and_then(|i| i.source_app))
                {
                    self.icons.ensure(&app);
                }
            }
            Ok(Captured::Skipped(SkipReason::Blacklisted)) => {
                eprintln!("来源应用在黑名单中，已忽略")
            }
            Ok(_) => {}
            // 剪切板偶尔被其他程序独占，下个周期会重试，不值得退出。
            Err(e) => eprintln!("读取剪切板失败: {e:#}"),
        }
    }

    pub fn maintenance(&mut self, now: f64) {
        if let Some(cutoff) = self.cfg.cutoff(now) {
            match self.store.purge_before(cutoff) {
                Ok(n) if n > 0 => eprintln!("清理了 {n} 条过期历史"),
                Ok(_) => {}
                Err(e) => eprintln!("清理过期历史失败: {e:#}"),
            }
        }
        if self.cfg.disk_limit_mb > 0 {
            match self
                .store
                .enforce_disk_limit(self.cfg.disk_limit_mb * 1024 * 1024)
            {
                Ok(n) if n > 0 => eprintln!("磁盘占用超限，删除了 {n} 张旧图片"),
                Ok(_) => {}
                Err(e) => eprintln!("执行磁盘限制失败: {e:#}"),
            }
        }
    }

    /// 把用户选中的条目写进系统剪切板；动作为粘贴时再发送粘贴按键。
    pub fn apply_pick(&mut self, pick: &Pick, now: f64) -> Result<()> {
        let item = self
            .store
            .get(pick.id)?
            .with_context(|| format!("条目 #{} 已不存在", pick.id))?;
        let content = item_to_content(&item, &self.store, pick.plain, pick.transform)?;
        // 先登记再写入：写入后的第一次轮询要认出它是自己的内容。
        self.monitor.note_self_write(&content);
        self.backend.write(&content).context("写入剪切板失败")?;
        self.store.touch(item.id, now)?;
        if pick.action == Action::Paste {
            std::thread::sleep(Duration::from_millis(self.cfg.paste_delay_ms));
            let key = self.cfg.paste_key_spec()?;
            (self.paste)(&key).context("模拟粘贴失败，内容已在剪切板中，可手动粘贴")?;
        }
        Ok(())
    }
}

enum Event {
    Hotkey,
    Menu(MenuAction),
    Ipc(Request),
    PickerDone { generation: u64, output: String },
}

struct PickerProc {
    child: Child,
    generation: u64,
}

struct SettingsProc {
    child: Child,
}

/// 必须在主线程调用：macOS 的全局热键要由主线程分发事件，守护进程的工作循环这时跑在工作线程上。
pub fn run(cfg: Config, paths: Paths, config_override: Option<PathBuf>) -> Result<()> {
    let listener = ipc::bind(&paths.socket)?;
    let socket_path = paths.socket.clone();
    let hotkey = cfg.hotkey_spec()?;
    let show_status = cfg.show_status_item;
    let labels = tray_labels(&cfg);
    let (hk_tx, hk_rx) = channel::<()>();
    let (menu_tx, menu_rx) = channel::<MenuAction>();
    let (ctl_tx, ctl_rx) = channel::<TrayControl>();
    let status = StatusIcon {
        io: TrayIo {
            actions: menu_tx,
            control: ctl_rx,
        },
        labels,
        visible: show_status,
    };
    let result = platform::run_session(&hotkey, hk_tx, Some(status), move |registered| {
        run_inner(
            cfg,
            &paths,
            config_override,
            listener,
            registered,
            hk_rx,
            menu_rx,
            ctl_tx,
        )
    });
    let _ = std::fs::remove_file(&socket_path);
    result
}

fn run_inner(
    cfg: Config,
    paths: &Paths,
    config_override: Option<PathBuf>,
    listener: std::os::unix::net::UnixListener,
    hotkey_registered: Result<()>,
    hotkey_rx: Receiver<()>,
    menu_rx: Receiver<MenuAction>,
    tray_tx: Sender<TrayControl>,
) -> Result<()> {
    let store = Store::open(&paths.data_dir)?;
    let backend = SystemClipboard::new().context("无法访问系统剪切板")?;
    let poll = Duration::from_millis(cfg.poll_interval_ms);
    let hotkey = cfg.hotkey.clone();

    let mut core = Core::new(
        cfg,
        store,
        backend,
        platform::active_app(),
        Box::new(platform::send_paste),
    );
    core.icons = IconCache::new(&paths.data_dir);
    core.monitor.prime(&mut core.backend);
    core.maintenance(now_secs());

    let (tx, rx) = channel::<Event>();

    let (ipc_tx, ipc_rx) = channel::<Request>();
    ipc::serve(listener, ipc_tx);
    bridge(ipc_rx, tx.clone(), Event::Ipc);
    bridge(menu_rx, tx.clone(), Event::Menu);

    // 不管启动时有没有注册成功都接上：之后在设置里换热键可能才注册成功。
    bridge(hotkey_rx, tx.clone(), |()| Event::Hotkey);
    match hotkey_registered {
        Ok(()) => {
            eprintln!("全局热键已注册: {hotkey}");
        }
        Err(e) => {
            eprintln!("未能注册全局热键 {hotkey}: {e:#}\n  仍可用 `cliptray toggle` 唤出面板。")
        }
    }

    install_signal_handlers();
    eprintln!(
        "Cliptray 守护进程已启动，数据目录: {}",
        paths.data_dir.display()
    );

    let exe = std::env::current_exe().context("无法确定自身路径")?;
    let mut picker: Option<PickerProc> = None;
    let mut settings: Option<SettingsProc> = None;
    let mut generation = 0u64;
    let mut last_tick = Instant::now();
    let mut last_maintenance = Instant::now();

    while !SHUTDOWN.load(Ordering::SeqCst) {
        let wait = poll.saturating_sub(last_tick.elapsed());
        match rx.recv_timeout(wait) {
            Ok(Event::Hotkey) => toggle(
                &mut picker,
                &mut generation,
                &exe,
                config_override.as_deref(),
                &tx,
            ),
            Ok(Event::Menu(action)) => match action {
                MenuAction::Toggle => toggle(
                    &mut picker,
                    &mut generation,
                    &exe,
                    config_override.as_deref(),
                    &tx,
                ),
                MenuAction::Settings => {
                    open_settings(&mut settings, &exe, config_override.as_deref())
                }
                MenuAction::Quit => SHUTDOWN.store(true, Ordering::SeqCst),
            },
            Ok(Event::Ipc((cmd, reply))) => {
                let text = match cmd {
                    Command::Toggle => {
                        toggle(
                            &mut picker,
                            &mut generation,
                            &exe,
                            config_override.as_deref(),
                            &tx,
                        );
                        "ok".to_owned()
                    }
                    Command::Show => {
                        if picker.is_none() {
                            open(
                                &mut picker,
                                &mut generation,
                                &exe,
                                config_override.as_deref(),
                                &tx,
                            );
                        }
                        "ok".to_owned()
                    }
                    Command::Hide => {
                        close(&mut picker);
                        "ok".to_owned()
                    }
                    Command::Status => format!(
                        "running hotkey={} items={} picker={}",
                        core.cfg.hotkey,
                        core.store.count().unwrap_or(-1),
                        if picker.is_some() { "open" } else { "closed" }
                    ),
                    Command::Quit => {
                        SHUTDOWN.store(true, Ordering::SeqCst);
                        "ok".to_owned()
                    }
                    Command::Settings => {
                        open_settings(&mut settings, &exe, config_override.as_deref());
                        "ok".to_owned()
                    }
                    Command::Reload => reload(&mut core, paths, &tray_tx),
                };
                let _ = reply.send(text);
            }
            Ok(Event::PickerDone {
                generation: g,
                output,
            }) => {
                // 只处理当前这一代面板；被 toggle 关掉的旧面板的输出直接丢弃。
                if picker.as_ref().is_some_and(|p| p.generation == g) {
                    if let Some(mut p) = picker.take() {
                        let _ = p.child.wait();
                    }
                    match serde_json::from_str::<Pick>(output.trim()) {
                        Ok(pick) => {
                            if let Err(e) = core.apply_pick(&pick, now_secs()) {
                                eprintln!("{e:#}");
                            }
                        }
                        Err(_) if output.trim().is_empty() => {}
                        Err(e) => eprintln!("面板返回了无法解析的内容: {e}"),
                    }
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        if last_tick.elapsed() >= poll {
            last_tick = Instant::now();
            core.tick(now_secs());
        }
        reap_settings(&mut settings);
        if last_maintenance.elapsed() >= Duration::from_secs(600) {
            last_maintenance = Instant::now();
            core.maintenance(now_secs());
        }
    }

    close(&mut picker);
    close_settings(&mut settings);
    eprintln!("Cliptray 守护进程已退出");
    Ok(())
}

fn bridge<T: Send + 'static>(
    rx: Receiver<T>,
    tx: Sender<Event>,
    wrap: impl Fn(T) -> Event + Send + 'static,
) {
    std::thread::spawn(move || {
        for v in rx {
            if tx.send(wrap(v)).is_err() {
                break;
            }
        }
    });
}

fn toggle(
    picker: &mut Option<PickerProc>,
    generation: &mut u64,
    exe: &Path,
    cfg: Option<&Path>,
    tx: &Sender<Event>,
) {
    if picker.is_some() {
        close(picker);
    } else {
        open(picker, generation, exe, cfg, tx);
    }
}

fn open(
    picker: &mut Option<PickerProc>,
    generation: &mut u64,
    exe: &Path,
    cfg: Option<&Path>,
    tx: &Sender<Event>,
) {
    let mut cmd = Process::new(exe);
    if let Some(c) = cfg {
        cmd.arg("--config").arg(c);
    }
    cmd.arg("pick").stdin(Stdio::null()).stdout(Stdio::piped());
    platform::prefer_x11_window(&mut cmd);
    match cmd.spawn() {
        Ok(mut child) => {
            *generation += 1;
            let g = *generation;
            if let Some(mut out) = child.stdout.take() {
                let tx = tx.clone();
                std::thread::spawn(move || {
                    let mut s = String::new();
                    let _ = out.read_to_string(&mut s);
                    let _ = tx.send(Event::PickerDone {
                        generation: g,
                        output: s,
                    });
                });
            }
            *picker = Some(PickerProc {
                child,
                generation: g,
            });
        }
        Err(e) => eprintln!("启动选择面板失败: {e}"),
    }
}

fn close(picker: &mut Option<PickerProc>) {
    if let Some(mut p) = picker.take() {
        let _ = p.child.kill();
        let _ = p.child.wait();
    }
}

fn open_settings(slot: &mut Option<SettingsProc>, exe: &Path, cfg: Option<&Path>) {
    if let Some(proc) = slot
        && proc.child.try_wait().ok().flatten().is_none()
    {
        return;
    }
    let mut cmd = Process::new(exe);
    if let Some(c) = cfg {
        cmd.arg("--config").arg(c);
    }
    cmd.arg("settings")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit());
    match cmd.spawn() {
        Ok(child) => *slot = Some(SettingsProc { child }),
        Err(e) => eprintln!("打开设置窗口失败: {e}"),
    }
}

fn reap_settings(slot: &mut Option<SettingsProc>) {
    if let Some(proc) = slot
        && proc.child.try_wait().ok().flatten().is_some()
    {
        *slot = None;
    }
}

fn close_settings(slot: &mut Option<SettingsProc>) {
    if let Some(mut proc) = slot.take() {
        let _ = proc.child.kill();
        let _ = proc.child.wait();
    }
}

fn reload(
    core: &mut Core<impl ClipboardBackend>,
    paths: &Paths,
    tray_tx: &Sender<TrayControl>,
) -> String {
    match Config::load(&paths.config_file) {
        Ok(mut new) => {
            // 热键有变就当场换。新热键注册不上时留着旧的，并把原因回复给设置窗口。
            let mut reply = "ok".to_owned();
            if new.hotkey != core.cfg.hotkey {
                let rebound = new
                    .hotkey_spec()
                    .and_then(|spec| platform::rebind_hotkey(&spec));
                match rebound {
                    Ok(()) => eprintln!("快捷键已改为 {}", new.hotkey),
                    Err(e) => {
                        eprintln!(
                            "快捷键 {} 没能生效，仍使用 {}: {e:#}",
                            new.hotkey, core.cfg.hotkey
                        );
                        reply = format!("error: {e:#}");
                        new.hotkey = core.cfg.hotkey.clone();
                    }
                }
            }
            let visible = new.show_status_item;
            let labels = tray_labels(&new);
            core.cfg = new;
            let _ = tray_tx.send(TrayControl::Relabel(labels));
            let _ = tray_tx.send(TrayControl::Visible(visible));
            reply
        }
        Err(e) => format!("error: {e:#}"),
    }
}

fn tray_labels(cfg: &Config) -> TrayLabels {
    let have_cjk = cfg!(target_os = "macos") || crate::picker::find_cjk_font().is_some();
    let lang = Lang::resolve(cfg.language, platform::ui_language().as_deref(), have_cjk);
    TrayLabels {
        setting: lang.menu(MenuWord::Setting).to_owned(),
        clipboard: lang.menu(MenuWord::Clipboard).to_owned(),
        quit: lang.menu(MenuWord::Quit).to_owned(),
    }
}

fn install_signal_handlers() {
    // SAFETY: handler 只写一个原子变量，是 async-signal-safe 的。
    unsafe {
        libc::signal(libc::SIGINT, on_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGTERM, on_signal as *const () as libc::sighandler_t);
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::*;
    use crate::clipboard::ClipContent;
    use crate::clipboard::testing::MemoryClipboard;
    use crate::model::{Item, Transform};
    use crate::monitor::NoActiveApp;

    fn core(paste_log: Rc<RefCell<Vec<String>>>) -> (tempfile::TempDir, Core<MemoryClipboard>) {
        let d = tempfile::tempdir().unwrap();
        let store = Store::open(d.path()).unwrap();
        let cfg = Config {
            paste_delay_ms: 0,
            paste_key: "ctrl+v".into(),
            ..Config::default()
        };
        let c = Core::new(
            cfg,
            store,
            MemoryClipboard::default(),
            Box::new(NoActiveApp),
            Box::new(move |k| {
                paste_log.borrow_mut().push(k.to_string());
                Ok(())
            }),
        );
        (d, c)
    }

    #[test]
    fn paste_pick_writes_clipboard_bumps_item_and_sends_paste_key() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let (_d, mut c) = core(log.clone());
        let a = c
            .store
            .insert(&Item::text("a".into(), None, None, 1.0))
            .unwrap()
            .id();
        c.store
            .insert(&Item::text("b".into(), None, None, 2.0))
            .unwrap();

        c.apply_pick(
            &Pick {
                id: a,
                action: Action::Paste,
                plain: false,
                transform: Transform::None,
            },
            10.0,
        )
        .unwrap();

        assert_eq!(
            c.backend.content,
            Some(ClipContent::Text {
                text: "a".into(),
                html: None
            })
        );
        assert_eq!(c.store.latest().unwrap().unwrap().id, a);
        assert_eq!(*log.borrow(), ["ctrl+v"]);
    }

    #[test]
    fn copy_pick_does_not_send_paste_key() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let (_d, mut c) = core(log.clone());
        let a = c
            .store
            .insert(&Item::text("a".into(), None, None, 1.0))
            .unwrap()
            .id();
        c.apply_pick(
            &Pick {
                id: a,
                action: Action::Copy,
                plain: false,
                transform: Transform::None,
            },
            2.0,
        )
        .unwrap();
        assert!(log.borrow().is_empty());
        assert!(c.backend.content.is_some());
    }

    #[test]
    fn picked_content_is_not_recorded_again_by_the_monitor() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let (_d, mut c) = core(log);
        let a = c
            .store
            .insert(&Item::text("a".into(), Some("<b>a</b>".into()), None, 1.0))
            .unwrap()
            .id();
        c.apply_pick(
            &Pick {
                id: a,
                action: Action::Copy,
                plain: true,
                transform: Transform::None,
            },
            2.0,
        )
        .unwrap();
        c.tick(3.0);
        // 纯文本写回后没有新增“a（纯文本）”这一条。
        assert_eq!(c.store.count().unwrap(), 1);
    }

    #[test]
    fn plain_pick_drops_html() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let (_d, mut c) = core(log);
        let a = c
            .store
            .insert(&Item::text("a".into(), Some("<b>a</b>".into()), None, 1.0))
            .unwrap()
            .id();
        c.apply_pick(
            &Pick {
                id: a,
                action: Action::Copy,
                plain: true,
                transform: Transform::None,
            },
            2.0,
        )
        .unwrap();
        assert_eq!(
            c.backend.content,
            Some(ClipContent::Text {
                text: "a".into(),
                html: None
            })
        );
    }

    #[test]
    fn transform_rewrites_text_and_is_not_recorded_again() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let (_d, mut c) = core(log);
        let a = c
            .store
            .insert(&Item::text(
                "a b\nc".into(),
                Some("<p>a b</p>".into()),
                None,
                1.0,
            ))
            .unwrap()
            .id();
        let pick = Pick {
            id: a,
            action: Action::Copy,
            plain: false,
            transform: Transform::StripWhitespace,
        };
        c.apply_pick(&pick, 2.0).unwrap();
        assert_eq!(
            c.backend.content,
            Some(ClipContent::Text {
                text: "abc".into(),
                html: None
            })
        );
        c.tick(3.0);
        assert_eq!(c.store.count().unwrap(), 1);
    }

    #[test]
    fn missing_item_is_an_error_and_leaves_clipboard_alone() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let (_d, mut c) = core(log.clone());
        let err = c
            .apply_pick(
                &Pick {
                    id: 999,
                    action: Action::Paste,
                    plain: false,
                    transform: Transform::None,
                },
                1.0,
            )
            .unwrap_err();
        assert!(format!("{err:#}").contains("#999"));
        assert!(c.backend.writes.is_empty());
        assert!(log.borrow().is_empty());
    }

    #[test]
    fn paste_failure_is_reported_but_content_stays_on_clipboard() {
        let d = tempfile::tempdir().unwrap();
        let mut store = Store::open(d.path()).unwrap();
        let id = store
            .insert(&Item::text("a".into(), None, None, 1.0))
            .unwrap()
            .id();
        let cfg = Config {
            paste_delay_ms: 0,
            ..Config::default()
        };
        let mut c = Core::new(
            cfg,
            store,
            MemoryClipboard::default(),
            Box::new(NoActiveApp),
            Box::new(|_| anyhow::bail!("no xtest")),
        );
        let err = c
            .apply_pick(
                &Pick {
                    id,
                    action: Action::Paste,
                    plain: false,
                    transform: Transform::None,
                },
                2.0,
            )
            .unwrap_err();
        assert!(format!("{err:#}").contains("no xtest"));
        assert!(c.backend.content.is_some());
    }

    #[test]
    fn maintenance_applies_retention() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let (_d, mut c) = core(log);
        c.cfg.history_days = 1;
        c.store
            .insert(&Item::text("old".into(), None, None, 0.0))
            .unwrap();
        c.store
            .insert(&Item::text("new".into(), None, None, 200_000.0))
            .unwrap();
        c.maintenance(200_000.0);
        assert_eq!(c.store.count().unwrap(), 1);
    }
}
