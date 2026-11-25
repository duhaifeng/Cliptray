# splay

[中文](README.zh-CN.md)

splay keeps a history of what you copy and shows it as a row of cards along the bottom of the screen. Press **Shift+Super+C** (⇧⌘C on a Mac) to open the strip, pick an item, and paste it back into the window you were using.

It is a Rust clipboard history tool in the spirit of [iCopy](https://icopy.better365.cn), for Linux and macOS.

## Install

**Debian / Ubuntu (amd64).** Download `splay_0.1.0_amd64.deb` from the [latest release](https://github.com/duhaifeng/splay/releases/latest):

```bash
sudo dpkg -i splay_0.1.0_amd64.deb
```

The package installs `/usr/bin/splay`, an application menu entry, and an XDG autostart file. A systemd user unit is installed but not enabled, so it does not start a second daemon next to the autostart entry.

**From source.** You need Rust and a C compiler (SQLite is bundled).

```bash
cargo install --path .
splay doctor    # session, hotkey, clipboard, CJK font
splay daemon    # run in the foreground
```

At runtime the panel needs libxkbcommon, an X11 or Wayland client library, OpenGL or EGL, and a CJK font such as `fonts-noto-cjk`. Without that font the interface falls back to English and Chinese text is drawn as boxes.

Log in again after the first install on GNOME Wayland so the Shell extension that grabs the global hotkey can load.

## What it records

Text, rich text (HTML), images, and files, including folders and several files at once. Copying the same thing again moves that entry to the front instead of adding a duplicate.

Favorites are kept when old history or large images are cleaned up. You can ignore copies from named apps, limit how many days are kept, and cap how much disk images use.

## The panel

The strip sits on the bottom edge of the screen: a full-width, rounded, dark bar. The top row is a search field, category tabs (All / Text / Image / File / Collect), a plain-text toggle (T), and settings. Each card is colored by type and shows the type, a relative time, and the source app. The footer shows the index, the character count or file size, and buttons to paste or copy. A missing file is a gray question mark and a red name.

The menu bar item (macOS status item, Linux StatusNotifierItem) opens Settings, the clipboard, or Quit. Settings has three pages: Clipboard, General, and Block list.

| Key or pointer | Action |
| --- | --- |
| ⌘F (Ctrl+F on Linux) or click the search field | Search |
| ↑ ↓, scroll wheel, or hover a card | Select a card. The wheel scrolls sideways |
| ← →, Tab, Shift+Tab, or hover a tab | Change category |
| Enter or click a card | Paste. `enter_action` and `click_action` can be set to copy only |
| 1…9 | Paste item N, when the search field is not focused. `number_modifier` can require Alt or Ctrl |
| Blue or gray button on a card | Paste, or copy only |
| Delete or Backspace | Delete, after a confirmation |
| Right click | Paste, paste as plain or rich text, copy, copy without line breaks, copy without line breaks or spaces, favorite, delete. Right-click a category tab to clear that category. Favorites are kept |
| Esc or a click outside | Close |

The language follows the system. On macOS that is the preferred language list. The panel reopens on the category you used last time.

## Commands

```
splay daemon                 resident process, with a menu bar icon
splay toggle | show | hide   show or hide the panel
splay settings               open settings
splay status | quit
splay list [--tag text] [--search WORD] [--limit 20]
splay clear [--tag all] [--keep-favorites] --yes
splay config path | show | init [--force]   # --force backs up the old file first
splay doctor
```

## Configuration

`~/.config/splay/config.toml`. Missing fields use the defaults. `splay config show` prints the values in effect.

```toml
hotkey = "shift+super+c"      # cmd, command, meta, and win are aliases of super
paste_key = "ctrl+v"          # macOS default is "super+v"; many Linux terminals want "ctrl+shift+v"
click_action = "paste"        # paste | copy | select (select pastes on double click)
enter_action = "paste"        # paste | copy
double_click_action = "paste"
paste_plain_text = true       # write text back as plain text; the T button toggles this for the open panel
hover_switch_group = true     # hovering a category tab switches to it
default_tag = "last"          # last | all | text | image | file | favorite
history_days = 0              # 0 = keep everything
disk_limit_mb = 1024          # image storage cap, 0 = no cap
blacklist = ["keepassxc"]     # exact app name, case-insensitive. X11 uses WM_CLASS; macOS uses the app name
number_modifier = "none"      # none | alt | ctrl
confirm_delete = true
auto_hide = true              # close the panel after an action
show_status_item = true
launch_at_startup = false     # next login. macOS writes a LaunchAgent; Linux writes an XDG autostart file
```

History lives in `~/.local/share/splay/`: `data.sqlite`, `tempFiles/` for images, `icons/` for source-app icons, and `last_tag` for the last category.

## Linux and macOS

| | X11 | Wayland | macOS |
| --- | --- | --- | --- |
| Global hotkey | Grabbed by splay | GNOME uses the bundled Shell extension `splay@splay.local`. Other compositors cannot be grabbed; bind Shift+Super+C to `splay toggle` | Carbon hotkey. The process that starts the daemon needs Accessibility to paste |
| Paste | XTest | GNOME uses Mutter RemoteDesktop. Other compositors need `wtype` | CGEvent, after Accessibility is granted. Otherwise the item is only copied |
| Source app and block list | `WM_CLASS` | Not available, so the block list does not apply | App name |
| Clipboard watch | Yes | Needs the compositor's data-control protocol | Yes |

`splay doctor` reports what the current session can actually do. On GNOME Wayland the extension is copied into `~/.local/share/gnome-shell/extensions/` the first time the daemon starts. Shell loads a new extension at the next login, so the hotkey stays dead until then. `SPLAY_BACKEND=x11` forces the X11 path.

macOS builds are produced with `./package-mac.sh` (`dist/splay.app` and a zip). Linux amd64 packages are produced with `./package-deb.sh`.

## Build

```bash
cargo test
./package-deb.sh    # dist/splay_<version>_amd64.deb
./package-mac.sh    # on macOS
```

`./restart.sh [--release]` stops a daemon built from this tree, rebuilds, and starts it again.

The daemon owns the clipboard and the history database. The panel is a short-lived child process: on Linux, clipboard contents belong to the process that wrote them and disappear when that process exits, so the daemon writes the chosen item and pastes it. The panel only returns the selection.

Not carried over from iCopy: quick replies, the clipboard stack, iCloud backup, Excel import and export, themes, sounds, double-modifier or Fn hotkeys, a vertical list, and export as an image, RTF, or TXT.

## License

[MIT](LICENSE)
