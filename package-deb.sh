#!/usr/bin/env bash
# 把 Cliptray 打成 linux/amd64 的 .deb。
# 在 Linux x86_64 上直接编译；在 macOS 或其他架构上通过 Docker 编译。
# 用法：./package-deb.sh
# 产物：dist/cliptray_<版本>_amd64.deb
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
cd "$root"

version=$(awk -F '"' '/^version = / { print $2; exit }' "$root/Cargo.toml")
if [ -z "$version" ]; then
  printf '无法从 Cargo.toml 读取版本号\n' >&2
  exit 1
fi

if [ -z "${DEB_MAINTAINER:-}" ]; then
  name=$(git -C "$root" config user.name 2>/dev/null || true)
  email=$(git -C "$root" config user.email 2>/dev/null || true)
  if [ -n "$name" ] && [ -n "$email" ]; then
    DEB_MAINTAINER="$name <$email>"
  else
    DEB_MAINTAINER="Cliptray maintainers <cliptray@localhost>"
  fi
fi

# 内置 SQLite 要用 C 编译器；图形库（X11、Wayland、GL）都是运行时动态加载，不需要开发包。
build_deps='build-essential ca-certificates pkg-config dpkg-dev file binutils'

install_build_deps() {
  if [ "$(id -u)" -ne 0 ]; then
    printf '缺少编译依赖。请用 root 安装：\n  sudo apt-get install %s\n' "$build_deps" >&2
    exit 1
  fi
  export DEBIAN_FRONTEND=noninteractive
  apt-get update
  # shellcheck disable=SC2086
  apt-get install -y --no-install-recommends $build_deps
}

ensure_native_tools() {
  missing=
  for cmd in cargo cc dpkg-deb strip file; do
    if ! command -v "$cmd" >/dev/null 2>&1; then
      missing="$missing $cmd"
    fi
  done
  if [ -n "$missing" ]; then
    printf '缺少工具：%s\nDebian/Ubuntu 可安装：\n  sudo apt-get install %s\n并安装 Rust：https://rustup.rs\n' "$missing" "$build_deps" >&2
    exit 1
  fi
}

pack_deb() {
  if [ -z "${CARGO_TARGET_DIR:-}" ]; then
    export CARGO_TARGET_DIR="$root/target"
  fi
  printf '正在为 x86_64-unknown-linux-gnu 编译发布版本…\n'
  cargo build --release --locked --target x86_64-unknown-linux-gnu

  bin="$CARGO_TARGET_DIR/x86_64-unknown-linux-gnu/release/cliptray"
  if [ ! -f "$bin" ]; then
    printf '没有找到编译产物：%s\n' "$bin" >&2
    exit 1
  fi
  if command -v file >/dev/null 2>&1 && ! file "$bin" | grep -q 'x86-64'; then
    printf '产物不是 amd64 可执行文件：%s\n' "$(file "$bin")" >&2
    exit 1
  fi

  stage=$(mktemp -d)
  trap 'rm -rf "$stage"' EXIT
  install -d -m 0755 \
    "$stage/DEBIAN" \
    "$stage/usr/bin" \
    "$stage/usr/share/applications" \
    "$stage/usr/share/icons/hicolor/scalable/apps" \
    "$stage/usr/share/doc/cliptray" \
    "$stage/usr/lib/systemd/user" \
    "$stage/etc/xdg/autostart"
  install -m 0755 "$bin" "$stage/usr/bin/cliptray"
  strip --strip-unneeded "$stage/usr/bin/cliptray"

  # 应用菜单入口：点一下就是启动守护进程。
  cat > "$stage/usr/share/applications/cliptray.desktop" <<'EOF'
[Desktop Entry]
Type=Application
Name=Cliptray
GenericName=Clipboard History
Comment=剪切板历史，Shift+Super+C 唤出面板
Exec=cliptray daemon
Icon=cliptray
Terminal=false
Categories=Utility;
Keywords=clipboard;history;paste;剪切板;
EOF

  # 登录桌面时自动启动守护进程。
  cat > "$stage/etc/xdg/autostart/cliptray.desktop" <<'EOF'
[Desktop Entry]
Type=Application
Name=Cliptray
Comment=剪切板历史守护进程
Exec=cliptray daemon
Icon=cliptray
Terminal=false
NoDisplay=true
X-GNOME-Autostart-enabled=true
EOF

  # 只安装 systemd 用户单元，不自动启用，避免和上面的自动启动重复。
  # 想用 systemd 管理时：systemctl --user enable --now cliptray
  cat > "$stage/usr/lib/systemd/user/cliptray.service" <<'EOF'
[Unit]
Description=Cliptray clipboard history daemon
After=graphical-session.target
PartOf=graphical-session.target

[Service]
ExecStart=/usr/bin/cliptray daemon
Restart=on-failure
RestartSec=2

[Install]
WantedBy=graphical-session.target
EOF

  install -m 0644 "$root/assets/cliptray.svg" \
    "$stage/usr/share/icons/hicolor/scalable/apps/cliptray.svg"
  install -m 0644 "$root/README.md" "$stage/usr/share/doc/cliptray/README.md"

  shlibs=
  if command -v dpkg-shlibdeps >/dev/null 2>&1; then
    # 错误输出放到 stage 之外，避免 shlibdeps.err 被打进 deb（会与其它包冲突）。
    shlibs_err=$(mktemp)
    # dpkg-shlibdeps 要求当前目录下有 debian/control，在临时目录里放一个最小的。
    shlibs_work=$(mktemp -d)
    mkdir -p "$shlibs_work/debian"
    printf 'Source: cliptray\n\nPackage: cliptray\nArchitecture: amd64\n' > "$shlibs_work/debian/control"
    set +e
    shlibs_out=$(cd "$shlibs_work" && dpkg-shlibdeps --ignore-missing-info -O "$stage/usr/bin/cliptray" 2>"$shlibs_err")
    shlibs_status=$?
    set -e
    rm -rf "$shlibs_work"
    if [ "$shlibs_status" -ne 0 ] && [ -s "$shlibs_err" ]; then
      printf 'dpkg-shlibdeps 提示：\n' >&2
      cat "$shlibs_err" >&2
    fi
    rm -f "$shlibs_err"
    shlibs=$(printf '%s\n' "$shlibs_out" | sed -n 's/^shlibs:Depends=//p')
  fi
  if [ -n "$shlibs" ]; then
    depends="$shlibs"
  else
    depends="libc6 (>= 2.34), libgcc-s1"
  fi

  installed_size=$(du -sk "$stage/usr" "$stage/etc" | awk '{ sum += $1 } END { print sum }')
  # wtype：Wayland 下自动粘贴；字体：面板里显示中文；GL/X11 库：面板窗口运行时加载。
  cat > "$stage/DEBIAN/control" <<EOF
Package: cliptray
Version: $version
Section: utils
Priority: optional
Architecture: amd64
Maintainer: $DEB_MAINTAINER
Installed-Size: $installed_size
Depends: $depends
Recommends: libgl1, libegl1, libx11-6, libxkbcommon0, libxkbcommon-x11-0, libwayland-client0, fonts-noto-cjk | fonts-wqy-microhei
Suggests: wtype
Homepage: https://github.com/duhaifeng/Cliptray
Description: Cliptray clipboard history manager with a popup picker
 记录文本、图片、文件的剪切板历史，按 Shift+Super+C 弹出面板选择并粘贴。
 X11 下注册全局热键并自动粘贴；Wayland 下请把桌面快捷键绑定到
 "cliptray toggle"，自动粘贴需要安装 wtype。
EOF

  # 每次打包前彻底清空 dist，避免残留旧版本产物。
  # 用 find 删内容而不是删目录本身：Docker 里 dist 是挂载点，不能被删除。
  mkdir -p "$root/dist"
  find "$root/dist" -mindepth 1 -delete
  out="$root/dist/cliptray_${version}_amd64.deb"
  deb_args=
  if dpkg-deb --help 2>&1 | grep -q root-owner-group; then
    deb_args=--root-owner-group
  fi
  # shellcheck disable=SC2086
  dpkg-deb $deb_args --build "$stage" "$out"
  if [ -n "${HOST_UID:-}" ] && [ -n "${HOST_GID:-}" ]; then
    chown "$HOST_UID:$HOST_GID" "$out" "$root/dist" 2>/dev/null || true
  fi
  printf '已生成 %s\n' "$out"
}

run_in_docker() {
  if ! command -v docker >/dev/null 2>&1; then
    printf '当前系统不是 Linux amd64，需要 Docker 才能打 amd64 deb。\n' >&2
    exit 1
  fi
  if ! docker info >/dev/null 2>&1; then
    printf 'Docker 没有在运行。请先启动 Docker 后再执行 ./package-deb.sh\n' >&2
    exit 1
  fi
  printf '使用 Docker（linux/amd64）打包…\n'
  mkdir -p "$root/dist"
  # 只挂载源码和产物目录，避免把本机 target 送进容器。
  docker run --rm --platform linux/amd64 \
    -e IN_DOCKER=1 \
    -e DEB_MAINTAINER="$DEB_MAINTAINER" \
    -e HOST_UID="$(id -u)" \
    -e HOST_GID="$(id -g)" \
    -e CARGO_TARGET_DIR=/tmp/cliptray-target \
    -v "$root/Cargo.toml:/work/Cargo.toml:ro" \
    -v "$root/Cargo.lock:/work/Cargo.lock:ro" \
    -v "$root/README.md:/work/README.md:ro" \
    -v "$root/src:/work/src:ro" \
    -v "$root/assets:/work/assets:ro" \
    -v "$root/package-deb.sh:/work/package-deb.sh:ro" \
    -v "$root/dist:/work/dist" \
    -v cliptray-cargo-registry:/usr/local/cargo/registry \
    -v cliptray-cargo-git:/usr/local/cargo/git \
    -w /work \
    rust:1-bookworm \
    bash /work/package-deb.sh --inside
}

if [ "${1:-}" = "--inside" ] || [ "${IN_DOCKER:-}" = "1" ]; then
  if [ "$(uname -s)" != "Linux" ] || [ "$(uname -m)" != "x86_64" ]; then
    printf '打包步骤必须在 Linux x86_64 环境中运行。\n' >&2
    exit 1
  fi
  if [ "${IN_DOCKER:-}" = "1" ]; then
    install_build_deps
  else
    ensure_native_tools
  fi
  if command -v rustup >/dev/null 2>&1; then
    rustup target add x86_64-unknown-linux-gnu
  fi
  pack_deb
  exit 0
fi

if [ "$(uname -s)" = "Linux" ] && [ "$(uname -m)" = "x86_64" ]; then
  ensure_native_tools
  if command -v rustup >/dev/null 2>&1; then
    rustup target add x86_64-unknown-linux-gnu
  fi
  pack_deb
else
  run_in_docker
fi
