use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};

use splay::config::{Config, Paths};
use splay::ipc::{self, Command};
use splay::model::Tag;
use splay::store::{Query, Store};
use splay::{daemon, i18n, picker, platform};

#[derive(Parser)]
#[command(
    name = "splay",
    version,
    about = "剪切板历史工具（iCopy 的 Linux 复刻）"
)]
struct Cli {
    /// 配置文件路径，默认 ~/.config/splay/config.toml
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// 启动常驻守护进程：监控剪切板、注册全局热键
    Daemon,
    /// 直接打开历史面板，把选择以 JSON 打印到标准输出（守护进程内部使用）
    Pick,
    /// 唤出 / 关闭面板（Wayland 下请把桌面环境的快捷键绑定到它）
    Toggle,
    /// 唤出面板
    Show,
    /// 关闭面板
    Hide,
    /// 查看守护进程状态
    Status,
    /// 让守护进程退出
    Quit,
    /// 打开设置窗口（守护进程负责只开一个）
    Settings,
    /// 在终端列出历史
    List {
        /// all / text / image / file / favorite
        #[arg(long, default_value = "all")]
        tag: String,
        #[arg(long)]
        search: Option<String>,
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// 清空历史
    Clear {
        #[arg(long, default_value = "all")]
        tag: String,
        /// 保留收藏
        #[arg(long)]
        keep_favorites: bool,
        /// 确认执行；不加则只提示将删除多少条
        #[arg(long)]
        yes: bool,
    },
    /// 配置文件操作
    Config {
        #[command(subcommand)]
        action: ConfigCmd,
    },
    /// 检查运行环境（会话类型、热键、剪切板、字体……）
    Doctor,
}

#[derive(Subcommand)]
enum ConfigCmd {
    /// 打印配置文件路径
    Path,
    /// 打印当前生效的配置
    Show,
    /// 写出默认配置；文件已存在则不动，加 --force 会先备份再覆盖
    Init {
        #[arg(long)]
        force: bool,
    },
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("错误: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let probe = Config::default();
    let config_path = Paths::resolve(&probe, cli.config.as_deref())?.config_file;
    let cfg = Config::load(&config_path)?;
    let paths = Paths::resolve(&cfg, cli.config.as_deref())?;

    match cli.command {
        Cmd::Daemon => daemon::run(cfg, paths, cli.config),
        Cmd::Pick => {
            let store = Store::open(&paths.data_dir)?;
            if let Some(pick) = picker::run(cfg, store, &paths.data_dir, &config_path)? {
                println!("{}", serde_json::to_string(&pick)?);
            }
            Ok(())
        }
        Cmd::Toggle => send(&paths, Command::Toggle),
        Cmd::Show => send(&paths, Command::Show),
        Cmd::Hide => send(&paths, Command::Hide),
        Cmd::Status => send(&paths, Command::Status),
        Cmd::Quit => send(&paths, Command::Quit),
        Cmd::Settings => splay::settings::run(cfg, paths),
        Cmd::List { tag, search, limit } => list(&paths, &cfg, &tag, search, limit),
        Cmd::Clear {
            tag,
            keep_favorites,
            yes,
        } => clear(&paths, &tag, keep_favorites, yes),
        Cmd::Config { action } => config_cmd(action, &config_path, &cfg),
        Cmd::Doctor => doctor(&cfg, &paths, &config_path),
    }
}

fn send(paths: &Paths, cmd: Command) -> Result<()> {
    let reply = ipc::send(&paths.socket, cmd)?;
    if let Some(msg) = reply.strip_prefix("error: ") {
        bail!("{msg}");
    }
    println!("{reply}");
    Ok(())
}

fn parse_tag(s: &str) -> Result<Tag> {
    Tag::parse(s)
        .with_context(|| format!("未知分类 {s:?}，可选: all / text / image / file / favorite"))
}

fn list(
    paths: &Paths,
    cfg: &Config,
    tag: &str,
    search: Option<String>,
    limit: usize,
) -> Result<()> {
    let store = Store::open(&paths.data_dir)?;
    let q = Query {
        tag: parse_tag(tag)?,
        text: search,
        limit,
        offset: 0,
        cutoff: cfg.cutoff(splay::store::now_secs()),
    };
    let items = store.query(&q)?;
    let now = chrono::Local::now();
    for it in items {
        println!(
            "{:>5}  {}{}  {:<10}  {}",
            it.id,
            if it.favorite { "★" } else { " " },
            i18n::relative_time(i18n::Lang::En, now, i18n::from_unix(it.created_at)),
            format!("{:?}", it.kind),
            it.preview_line(80)
        );
    }
    Ok(())
}

fn clear(paths: &Paths, tag: &str, keep_favorites: bool, yes: bool) -> Result<()> {
    let tag = parse_tag(tag)?;
    let mut store = Store::open(&paths.data_dir)?;
    if !yes {
        let n = store
            .query(&Query {
                tag,
                limit: usize::MAX >> 1,
                ..Query::default()
            })?
            .iter()
            .filter(|i| !(keep_favorites && i.favorite))
            .count();
        println!("将删除 {n} 条历史。确认请加 --yes。");
        return Ok(());
    }
    let n = store.clear(tag, keep_favorites)?;
    println!("已删除 {n} 条");
    Ok(())
}

fn config_cmd(action: ConfigCmd, path: &std::path::Path, cfg: &Config) -> Result<()> {
    match action {
        ConfigCmd::Path => println!("{}", path.display()),
        ConfigCmd::Show => print!("{}", cfg.to_toml()?),
        ConfigCmd::Init { force } => {
            if path.exists() {
                if !force {
                    println!(
                        "配置已存在，未改动: {}（要覆盖请加 --force，会先备份）",
                        path.display()
                    );
                    return Ok(());
                }
                let backup = path.with_extension(format!(
                    "toml.bak-{}",
                    chrono::Local::now().format("%Y%m%d%H%M%S")
                ));
                std::fs::copy(path, &backup)
                    .with_context(|| format!("备份失败: {}", backup.display()))?;
                println!("已备份到 {}", backup.display());
            }
            Config::write_default(path)?;
            println!("已写入默认配置: {}", path.display());
        }
    }
    Ok(())
}

fn doctor(cfg: &Config, paths: &Paths, config_path: &std::path::Path) -> Result<()> {
    let ok = |b: bool| if b { "ok  " } else { "FAIL" };
    println!(
        "配置文件   {} ({})",
        config_path.display(),
        if config_path.exists() {
            "存在"
        } else {
            "不存在，使用默认值"
        }
    );
    println!("数据目录   {}", paths.data_dir.display());
    let session = platform::session();
    println!("会话类型   {session:?}");
    println!(
        "[{}] 热键 {} 的内置抓取：{}",
        ok(matches!(
            session,
            platform::Session::X11 | platform::Session::Macos
        )),
        cfg.hotkey,
        match session {
            platform::Session::X11 => "X11 下由 splay 直接抓取",
            platform::Session::Macos => "macOS 使用 Carbon 全局热键",
            platform::Session::Wayland =>
                "Wayland 不允许；请在桌面环境里把快捷键绑定到 `splay toggle`",
            platform::Session::Unknown => "未检测到图形会话（DISPLAY / WAYLAND_DISPLAY 均未设置）",
        }
    );
    let clip = splay::clipboard::SystemClipboard::new();
    println!(
        "[{}] 剪切板访问{}",
        ok(clip.is_ok()),
        clip.err().map(|e| format!("：{e}")).unwrap_or_default()
    );
    let paste_hint = match session {
        platform::Session::Wayland => {
            let has = std::process::Command::new("wtype")
                .arg("--help")
                .output()
                .is_ok();
            (
                has,
                if has {
                    "wtype 可用"
                } else {
                    "未安装 wtype，Wayland 下无法自动粘贴（仍可只拷贝）"
                },
            )
        }
        platform::Session::X11 => (true, "X11 使用 XTest 模拟按键"),
        platform::Session::Macos if platform::accessibility_trusted() => {
            (true, "macOS 使用 CGEvent 发送粘贴键")
        }
        platform::Session::Macos => (
            false,
            "需要在「系统设置 → 隐私与安全性 → 辅助功能」里允许启动守护进程的程序（终端 / Cursor / splay.app）",
        ),
        platform::Session::Unknown => (false, "未检测到图形会话，无法自动粘贴"),
    };
    println!("[{}] 自动粘贴：{}", ok(paste_hint.0), paste_hint.1);
    let font = picker_font();
    println!(
        "[{}] CJK 字体：{}",
        ok(font.is_some()),
        font.unwrap_or_else(|| "未找到，面板将使用英文界面且中文内容无法显示".into())
    );
    let daemon = ipc::send(&paths.socket, Command::Status);
    println!(
        "[{}] 守护进程：{}",
        ok(daemon.is_ok()),
        daemon.unwrap_or_else(|_| "未运行".into())
    );
    println!(
        "菜单栏图标  {}",
        match session {
            platform::Session::Macos => "macOS 状态栏（NSStatusItem）；可在设置里关闭",
            platform::Session::X11 | platform::Session::Wayland =>
                "Linux 使用 StatusNotifierItem（GNOME 需要 AppIndicator 扩展）",
            platform::Session::Unknown => "没有图形会话，不显示",
        }
    );
    Ok(())
}

fn picker_font() -> Option<String> {
    picker::find_cjk_font().map(|f| f.path.display().to_string())
}
