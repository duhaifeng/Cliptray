#!/usr/bin/env bash
# 在 macOS 上把 splay 打成 .app，并压缩成 zip。
# 图标使用 assets/splay.svg，和 deb 包是同一张图。
# macOS 上注册全局热键（默认 ⇧⌘C）。自动粘贴还没实现，选中条目后请手动 ⌘V。
# 用法：./package-mac.sh
# 产物：dist/splay.app
#       dist/splay_<版本>_macos_<架构>.zip
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
cd "$root"

if [ "$(uname -s)" != "Darwin" ]; then
  printf 'package-mac.sh 只能在 macOS 上运行。\n' >&2
  exit 1
fi

for cmd in cargo qlmanage sips iconutil ditto python3; do
  if ! command -v "$cmd" >/dev/null 2>&1; then
    printf '缺少工具：%s\n' "$cmd" >&2
    exit 1
  fi
done

version=$(awk -F '"' '/^version = / { print $2; exit }' "$root/Cargo.toml")
if [ -z "$version" ]; then
  printf '无法从 Cargo.toml 读取版本号\n' >&2
  exit 1
fi

arch=$(uname -m)
svg="$root/assets/splay.svg"
if [ ! -f "$svg" ]; then
  printf '找不到图标：%s\n' "$svg" >&2
  exit 1
fi

printf '正在编译发布版本…\n'
cargo build --release --locked
bin="$root/target/release/splay"

app="$root/dist/splay.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"

install -m 0755 "$bin" "$app/Contents/MacOS/splay"
strip -x "$app/Contents/MacOS/splay" 2>/dev/null || true

# 双击 .app 时没有命令行参数，用一个启动脚本转成 `splay daemon`。
# 直接执行 Contents/MacOS/splay 仍然可以当命令行工具用。
cat > "$app/Contents/MacOS/splay-launcher" <<'EOF'
#!/bin/sh
dir=$(dirname "$0")
exec "$dir/splay" daemon
EOF
chmod 0755 "$app/Contents/MacOS/splay-launcher"

# 先栅格化最大尺寸，再缩小，避免每个尺寸都重新渲染。
icon_tmp=$(mktemp -d)
trap 'rm -rf "$icon_tmp"' EXIT
qlmanage -t -s 1024 -o "$icon_tmp" "$svg" >/dev/null
rendered="$icon_tmp/splay.svg.png"
master="$icon_tmp/icon.png"
if [ ! -f "$rendered" ]; then
  printf '无法从 SVG 生成图标\n' >&2
  exit 1
fi
python3 "$root/assets/mac_icon.py" "$rendered" "$master"

iconset="$icon_tmp/AppIcon.iconset"
mkdir -p "$iconset"
# 名称和像素尺寸是 iconutil 要求的对应关系。
render_size() {
  name=$1
  size=$2
  sips -z "$size" "$size" "$master" --out "$iconset/$name" >/dev/null
}
render_size icon_16x16.png 16
render_size icon_16x16@2x.png 32
render_size icon_32x32.png 32
render_size icon_32x32@2x.png 64
render_size icon_128x128.png 128
render_size icon_128x128@2x.png 256
render_size icon_256x256.png 256
render_size icon_256x256@2x.png 512
render_size icon_512x512.png 512
render_size icon_512x512@2x.png 1024
iconutil -c icns "$iconset" -o "$app/Contents/Resources/AppIcon.icns"

# LSUIElement：常驻后台，不在 Dock 显示图标，和 iCopy 一样只有面板。
cat > "$app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key>
  <string>zh_CN</string>
  <key>CFBundleDisplayName</key>
  <string>splay</string>
  <key>CFBundleExecutable</key>
  <string>splay-launcher</string>
  <key>CFBundleIconFile</key>
  <string>AppIcon</string>
  <key>CFBundleIdentifier</key>
  <string>com.splay.app</string>
  <key>CFBundleInfoDictionaryVersion</key>
  <string>6.0</string>
  <key>CFBundleName</key>
  <string>splay</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleShortVersionString</key>
  <string>$version</string>
  <key>CFBundleVersion</key>
  <string>$version</string>
  <key>LSApplicationCategoryType</key>
  <string>public.app-category.utilities</string>
  <key>LSMinimumSystemVersion</key>
  <string>11.0</string>
  <key>LSUIElement</key>
  <true/>
  <key>NSHighResolutionCapable</key>
  <true/>
</dict>
</plist>
EOF
printf 'APPL????' > "$app/Contents/PkgInfo"

if command -v codesign >/dev/null 2>&1; then
  codesign --force --deep --sign - "$app"
fi

zip="$root/dist/splay_${version}_macos_${arch}.zip"
rm -f "$zip"
ditto -c -k --keepParent "$app" "$zip"
printf '已生成 %s\n' "$app"
printf '已生成 %s\n' "$zip"
