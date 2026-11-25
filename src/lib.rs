//! splay：iCopy 的 Linux 复刻（Rust）。
//!
//! 依赖方向：`model` / `config` / `hotkey` 是纯数据；`store`、`clipboard`、`platform`、`appicon` 是适配层；
//! `monitor`、`daemon`、`picker` 是应用逻辑；`main` 只做命令行装配。

pub mod appicon;
pub mod autostart;
pub mod clipboard;
pub mod config;
pub mod daemon;
pub mod hotkey;
pub mod i18n;
pub mod ipc;
pub mod model;
pub mod monitor;
pub mod picker;
pub mod platform;
pub mod settings;
mod settings_ui;
pub mod statusicon;
pub mod store;
