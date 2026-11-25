//! 守护进程的本地控制通道（Unix socket，一行一条命令）以及选择面板的返回协议。
//!
//! Wayland 不允许应用抓取全局热键，用户在桌面环境里把快捷键绑定到 `splay toggle`，
//! 它经由这里唤起面板；X11 下的内置热键走的是同一个 `Toggle`。

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::str::FromStr;
use std::sync::mpsc::{Sender, channel};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::config::Action;
use crate::model::Transform;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Toggle,
    Show,
    Hide,
    Status,
    /// 打开设置窗口
    Settings,
    /// 重新读取配置文件
    Reload,
    Quit,
}

impl Command {
    pub fn as_str(self) -> &'static str {
        match self {
            Command::Toggle => "toggle",
            Command::Show => "show",
            Command::Hide => "hide",
            Command::Status => "status",
            Command::Quit => "quit",
            Command::Settings => "settings",
            Command::Reload => "reload",
        }
    }
}

impl FromStr for Command {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        Ok(match s.trim() {
            "toggle" => Command::Toggle,
            "show" => Command::Show,
            "hide" => Command::Hide,
            "status" => Command::Status,
            "quit" => Command::Quit,
            "settings" => Command::Settings,
            "reload" => Command::Reload,
            other => bail!("未知命令: {other:?}"),
        })
    }
}

/// 选择面板告诉守护进程用户选了什么。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pick {
    pub id: i64,
    pub action: Action,
    /// 以纯文本写入（丢弃富文本格式；文件条目写成路径文本）。
    pub plain: bool,
    /// 写入前改写文本；改写后必然是纯文本。
    #[serde(default)]
    pub transform: Transform,
}

pub type Request = (Command, Sender<String>);

/// 绑定控制 socket。已有存活的守护进程时报错；遗留的死 socket 文件会被清理。
pub fn bind(path: &Path) -> Result<UnixListener> {
    if let Some(parent) = path.parent() {
        // recursive 模式下目录已存在不算错误；权限只作用于新建的目录。
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(parent)?;
    }
    if path.exists() {
        if UnixStream::connect(path).is_ok() {
            bail!("splay 守护进程已在运行（{}）", path.display());
        }
        fs::remove_file(path)
            .with_context(|| format!("清理遗留 socket 失败: {}", path.display()))?;
    }
    let listener = UnixListener::bind(path)
        .with_context(|| format!("绑定 socket 失败: {}", path.display()))?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

/// 在后台线程里接受连接，把命令转给守护进程主循环，并把主循环的回复写回客户端。
pub fn serve(listener: UnixListener, tx: Sender<Request>) {
    std::thread::Builder::new()
        .name("ipc".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let tx = tx.clone();
                // 每个连接单独线程：一个卡住的客户端不应阻塞后续命令。
                let _ = std::thread::Builder::new()
                    .name("ipc-conn".into())
                    .spawn(move || {
                        let _ = handle(stream, &tx);
                    });
            }
        })
        .expect("启动 ipc 线程");
}

fn handle(stream: UnixStream, tx: &Sender<Request>) -> Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    let mut line = String::new();
    BufReader::new(&stream).read_line(&mut line)?;
    let reply = match line.parse::<Command>() {
        Ok(cmd) => {
            let (rtx, rrx) = channel();
            if tx.send((cmd, rtx)).is_err() {
                "error: 守护进程正在退出".to_owned()
            } else {
                rrx.recv_timeout(Duration::from_secs(10))
                    .unwrap_or_else(|_| "error: 守护进程无响应".to_owned())
            }
        }
        Err(e) => format!("error: {e}"),
    };
    (&stream).write_all(format!("{reply}\n").as_bytes())?;
    Ok(())
}

/// 客户端：发送命令并返回守护进程的回复。
pub fn send(path: &Path, cmd: Command) -> Result<String> {
    let stream = UnixStream::connect(path).with_context(|| {
        format!(
            "连接不上 splay 守护进程（{}）。先运行 `splay daemon`",
            path.display()
        )
    })?;
    stream.set_read_timeout(Some(Duration::from_secs(12)))?;
    (&stream).write_all(format!("{}\n", cmd.as_str()).as_bytes())?;
    let mut reply = String::new();
    BufReader::new(&stream).read_line(&mut reply)?;
    Ok(reply.trim_end().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_commands() {
        assert_eq!("toggle\n".parse::<Command>().unwrap(), Command::Toggle);
        assert_eq!(" quit ".parse::<Command>().unwrap(), Command::Quit);
        assert!("nope".parse::<Command>().is_err());
    }

    #[test]
    fn pick_json_roundtrip() {
        let p = Pick {
            id: 7,
            action: Action::Paste,
            plain: true,
            transform: Transform::StripNewlines,
        };
        let s = serde_json::to_string(&p).unwrap();
        assert_eq!(serde_json::from_str::<Pick>(&s).unwrap(), p);
        assert_eq!(
            s,
            r#"{"id":7,"action":"paste","plain":true,"transform":"strip_newlines"}"#
        );
        // 面板与守护进程版本不一致时，缺省字段按不改写处理。
        let old: Pick = serde_json::from_str(r#"{"id":7,"action":"copy","plain":false}"#).unwrap();
        assert_eq!(old.transform, Transform::None);
    }

    #[test]
    fn server_roundtrip_and_stale_socket_cleanup() {
        let d = tempfile::tempdir().unwrap();
        let sock = d.path().join("s.sock");

        // 遗留的死 socket：文件存在但没人监听。
        drop(UnixListener::bind(&sock).unwrap());
        assert!(sock.exists());

        let listener = bind(&sock).unwrap();
        let (tx, rx) = channel::<Request>();
        serve(listener, tx);
        let responder = std::thread::spawn(move || {
            let (cmd, reply) = rx.recv().unwrap();
            reply.send(format!("got {}", cmd.as_str())).unwrap();
        });
        assert_eq!(send(&sock, Command::Toggle).unwrap(), "got toggle");
        responder.join().unwrap();

        // 已有存活实例时第二次 bind 必须失败，而不是抢走 socket。
        assert!(bind(&sock).is_err());
    }

    #[test]
    fn unknown_command_gets_error_reply() {
        let d = tempfile::tempdir().unwrap();
        let sock = d.path().join("s.sock");
        let listener = bind(&sock).unwrap();
        let (tx, _rx) = channel::<Request>();
        serve(listener, tx);
        let mut s = UnixStream::connect(&sock).unwrap();
        s.write_all(b"bogus\n").unwrap();
        let mut reply = String::new();
        BufReader::new(&s).read_line(&mut reply).unwrap();
        assert!(reply.starts_with("error:"));
    }

    #[test]
    fn client_without_daemon_gives_actionable_error() {
        let d = tempfile::tempdir().unwrap();
        let err = send(&d.path().join("none.sock"), Command::Status).unwrap_err();
        assert!(format!("{err:#}").contains("splay daemon"));
    }
}
