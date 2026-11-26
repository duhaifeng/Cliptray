#!/usr/bin/env bash
# 在 macOS 和 Linux 上重新编译并启动 Cliptray 守护进程。
# 用法：
#   ./restart.sh                   调试构建并启动 daemon
#   ./restart.sh --release         发布构建后再启动
#   ./restart.sh -- list --limit 5 其余参数原样传给程序（此时不会停止旧的 daemon）
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
cd "$root"

os=$(uname -s)
case "$os" in
  Darwin|Linux) ;;
  *)
    printf '只支持 macOS 和 Linux，当前系统是 %s\n' "$os" >&2
    exit 1
    ;;
esac

if ! command -v cargo >/dev/null 2>&1; then
  printf '未找到 cargo。请先安装 Rust：https://rustup.rs\n' >&2
  exit 1
fi

profile=debug
cargo_args=
if [ "${1:-}" = "--release" ]; then
  profile=release
  cargo_args=--release
  shift
fi
# 允许用 -- 分隔脚本参数和程序参数。
if [ "${1:-}" = "--" ]; then
  shift
fi
# 没有参数就启动常驻进程。
if [ "$#" -eq 0 ]; then
  set -- daemon
fi

pattern="$root/target/(debug|release)/cliptray( |$)"

stop_running() {
  if ! command -v pgrep >/dev/null 2>&1; then
    return 0
  fi
  # 只结束本仓库编译出来的进程（含面板子进程），避免误杀其他同名程序。
  pids=$(pgrep -f "$pattern" || true)
  if [ -z "$pids" ]; then
    return 0
  fi
  printf '停止已在运行的 Cliptray：%s\n' "$pids"
  # SIGTERM 会让 daemon 清理 socket 并退出。
  # shellcheck disable=SC2086
  kill $pids 2>/dev/null || true
  sleep 0.4
  pids=$(pgrep -f "$pattern" || true)
  if [ -n "$pids" ]; then
    # shellcheck disable=SC2086
    kill -9 $pids 2>/dev/null || true
  fi
}

linux_build_hint() {
  if [ "$os" != "Linux" ]; then
    return 0
  fi
  if command -v cc >/dev/null 2>&1; then
    return 0
  fi
  cat >&2 <<'EOF'
Linux 上编译需要 C 编译器（内置 SQLite 要用）。Debian/Ubuntu 可以先安装：
  sudo apt-get install build-essential pkg-config
EOF
}

# 只有启动 daemon 才先停掉旧进程，避免新旧两个 daemon 抢同一个热键和 socket；
# list/status 等一次性命令不该打断正在运行的 daemon。
if [ "$1" = "daemon" ]; then
  stop_running
fi
linux_build_hint

printf '正在编译（%s）…\n' "$profile"
# shellcheck disable=SC2086
cargo build $cargo_args

bin="$root/target/$profile/cliptray"
printf '启动 %s %s\n' "$bin" "$*"
exec "$bin" "$@"
