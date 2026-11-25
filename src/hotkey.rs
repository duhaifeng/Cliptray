//! 快捷键描述的解析。与平台无关；抓取在 `platform`（X11 或 macOS Carbon）。
//!
//! 写法：`shift+super+c`。`cmd` / `command` / `meta` / `win` 都是 Super 的别名，
//! 这样把 iCopy 的 ⇧⌘C 直接写成 `shift+cmd+c` 也能用。

use std::fmt;
use std::str::FromStr;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub super_key: bool,
}

impl Mods {
    pub fn is_empty(self) -> bool {
        !(self.shift || self.ctrl || self.alt || self.super_key)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HotkeySpec {
    pub mods: Mods,
    /// 规范化后的按键名：单个小写字符，或 `space` / `enter` / `f1` 这类名字。
    pub key: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum HotkeyError {
    Empty,
    NoKey,
    MultipleKeys(String, String),
    UnknownKey(String),
}

impl fmt::Display for HotkeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "快捷键为空"),
            Self::NoKey => write!(f, "快捷键只有修饰键，缺少主键"),
            Self::MultipleKeys(a, b) => write!(f, "快捷键里有两个主键: {a}, {b}"),
            Self::UnknownKey(k) => write!(f, "不认识的按键: {k}"),
        }
    }
}

impl std::error::Error for HotkeyError {}

impl FromStr for HotkeySpec {
    type Err = HotkeyError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.trim().is_empty() {
            return Err(HotkeyError::Empty);
        }
        let mut mods = Mods::default();
        let mut key: Option<String> = None;
        for raw in s.split('+') {
            let part = raw.trim().to_ascii_lowercase();
            match part.as_str() {
                "shift" => mods.shift = true,
                "ctrl" | "control" => mods.ctrl = true,
                "alt" | "option" | "opt" => mods.alt = true,
                "super" | "cmd" | "command" | "meta" | "win" | "windows" => mods.super_key = true,
                "" => return Err(HotkeyError::UnknownKey(raw.to_owned())),
                other => {
                    let norm = normalize_key(other)
                        .ok_or_else(|| HotkeyError::UnknownKey(other.to_owned()))?;
                    if let Some(prev) = key.replace(norm.clone()) {
                        return Err(HotkeyError::MultipleKeys(prev, norm));
                    }
                }
            }
        }
        let key = key.ok_or(HotkeyError::NoKey)?;
        Ok(HotkeySpec { mods, key })
    }
}

impl fmt::Display for HotkeySpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts: Vec<&str> = Vec::new();
        if self.mods.ctrl {
            parts.push("ctrl");
        }
        if self.mods.alt {
            parts.push("alt");
        }
        if self.mods.shift {
            parts.push("shift");
        }
        if self.mods.super_key {
            parts.push("super");
        }
        parts.push(&self.key);
        write!(f, "{}", parts.join("+"))
    }
}

impl HotkeySpec {
    /// 给人看的写法。`mac_symbols` 为真时是 macOS 习惯的 ⌃⌥⇧⌘C，否则是 Ctrl+Shift+C。
    pub fn label(&self, mac_symbols: bool) -> String {
        let key = display_key(&self.key);
        if mac_symbols {
            let m = self.mods;
            let mut out = String::new();
            for (on, sym) in [
                (m.ctrl, '⌃'),
                (m.alt, '⌥'),
                (m.shift, '⇧'),
                (m.super_key, '⌘'),
            ] {
                if on {
                    out.push(sym);
                }
            }
            out.push_str(&key);
            out
        } else {
            let m = self.mods;
            let mut parts: Vec<&str> = Vec::new();
            for (on, name) in [
                (m.ctrl, "Ctrl"),
                (m.alt, "Alt"),
                (m.shift, "Shift"),
                (m.super_key, "Super"),
            ] {
                if on {
                    parts.push(name);
                }
            }
            parts.push(&key);
            parts.join("+")
        }
    }

    /// Mutter 接受的 `<Shift><Super>c` 写法（GNOME Shell 扩展抓热键用）。
    pub fn gnome_accelerator(&self) -> String {
        let mut out = String::new();
        for (on, name) in [
            (self.mods.shift, "<Shift>"),
            (self.mods.ctrl, "<Control>"),
            (self.mods.alt, "<Alt>"),
            (self.mods.super_key, "<Super>"),
        ] {
            if on {
                out.push_str(name);
            }
        }
        let key = match self.key.as_str() {
            "space" => "space",
            "enter" => "Return",
            "esc" => "Escape",
            "tab" => "Tab",
            "insert" => "Insert",
            "delete" => "Delete",
            "backspace" => "BackSpace",
            "home" => "Home",
            "left" => "Left",
            "up" => "Up",
            "right" => "Right",
            "down" => "Down",
            "pageup" => "Page_Up",
            "pagedown" => "Page_Down",
            "end" => "End",
            "`" => "grave",
            "-" => "minus",
            "=" => "equal",
            "[" => "bracketleft",
            "]" => "bracketright",
            "\\" => "backslash",
            ";" => "semicolon",
            "'" => "apostrophe",
            "," => "comma",
            "." => "period",
            "/" => "slash",
            other => {
                if let Some(n) = other.strip_prefix('f').and_then(|n| n.parse::<u32>().ok()) {
                    out.push_str(&format!("F{n}"));
                    return out;
                }
                other
            }
        };
        out.push_str(key);
        out
    }

    /// X11 keysym。ASCII 字符的 keysym 等于其 Latin-1 码点（小写）。
    pub fn keysym(&self) -> u32 {
        key_to_keysym(&self.key).expect("key 在解析时已校验")
    }
}

/// 主键的显示名：单字符大写，`f5` 变 `F5`，`space` 变 `Space`。
fn display_key(k: &str) -> String {
    let mut chars = k.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

fn normalize_key(k: &str) -> Option<String> {
    let named = match k {
        "esc" | "escape" => "esc",
        "return" | "enter" => "enter",
        "spacebar" | "space" => "space",
        "tab" => "tab",
        "ins" | "insert" => "insert",
        "del" | "delete" => "delete",
        "backspace" => "backspace",
        "up" => "up",
        "down" => "down",
        "left" => "left",
        "right" => "right",
        "home" => "home",
        "end" => "end",
        "pageup" | "pgup" => "pageup",
        "pagedown" | "pgdn" => "pagedown",
        _ => "",
    };
    if !named.is_empty() {
        return Some(named.to_owned());
    }
    if let Some(n) = k.strip_prefix('f').and_then(|n| n.parse::<u32>().ok()) {
        return (1..=12).contains(&n).then(|| format!("f{n}"));
    }
    let mut chars = k.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if c.is_ascii_alphanumeric() || "`-=[]\\;',./".contains(c) => {
            Some(c.to_string())
        }
        _ => None,
    }
}

fn key_to_keysym(k: &str) -> Option<u32> {
    Some(match k {
        "space" => 0x0020,
        "enter" => 0xff0d,
        "esc" => 0xff1b,
        "tab" => 0xff09,
        "insert" => 0xff63,
        "delete" => 0xffff,
        "backspace" => 0xff08,
        "home" => 0xff50,
        "left" => 0xff51,
        "up" => 0xff52,
        "right" => 0xff53,
        "down" => 0xff54,
        "pageup" => 0xff55,
        "pagedown" => 0xff56,
        "end" => 0xff57,
        _ => {
            if let Some(n) = k.strip_prefix('f').and_then(|n| n.parse::<u32>().ok()) {
                return (1..=12).contains(&n).then_some(0xffbe + n - 1);
            }
            let mut chars = k.chars();
            return match (chars.next(), chars.next()) {
                (Some(c), None) if c.is_ascii() => Some(c as u32),
                _ => None,
            };
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icopy_default_shift_cmd_c_parses_as_shift_super_c() {
        let h: HotkeySpec = "shift+cmd+c".parse().unwrap();
        assert!(h.mods.shift && h.mods.super_key && !h.mods.ctrl && !h.mods.alt);
        assert_eq!(h.key, "c");
        assert_eq!(h.keysym(), b'c' as u32);
        assert_eq!(h.to_string(), "shift+super+c");
    }

    #[test]
    fn aliases_and_case_and_spaces() {
        let a: HotkeySpec = " Control + Option + V ".parse().unwrap();
        assert!(a.mods.ctrl && a.mods.alt);
        assert_eq!(a.key, "v");
        let b: HotkeySpec = "win+Return".parse().unwrap();
        assert_eq!(b.key, "enter");
        assert_eq!(b.keysym(), 0xff0d);
    }

    #[test]
    fn function_keys() {
        assert_eq!("f1".parse::<HotkeySpec>().unwrap().keysym(), 0xffbe);
        assert_eq!("ctrl+f12".parse::<HotkeySpec>().unwrap().keysym(), 0xffc9);
        assert!("f13".parse::<HotkeySpec>().is_err());
    }

    #[test]
    fn rejects_bad_specs() {
        assert_eq!("".parse::<HotkeySpec>(), Err(HotkeyError::Empty));
        assert_eq!("shift+cmd".parse::<HotkeySpec>(), Err(HotkeyError::NoKey));
        assert!(matches!(
            "a+b".parse::<HotkeySpec>(),
            Err(HotkeyError::MultipleKeys(..))
        ));
        assert!(matches!(
            "shift+foo".parse::<HotkeySpec>(),
            Err(HotkeyError::UnknownKey(_))
        ));
        assert!(matches!(
            "shift++c".parse::<HotkeySpec>(),
            Err(HotkeyError::UnknownKey(_))
        ));
    }

    #[test]
    fn label_uses_mac_symbols_or_words() {
        let spec: HotkeySpec = "shift+cmd+c".parse().unwrap();
        assert_eq!(spec.label(true), "⇧⌘C");
        assert_eq!(spec.label(false), "Shift+Super+C");
        let spec: HotkeySpec = "ctrl+alt+f12".parse().unwrap();
        assert_eq!(spec.label(true), "⌃⌥F12");
        assert_eq!(spec.label(false), "Ctrl+Alt+F12");
    }

    #[test]
    fn gnome_accelerators() {
        let accel = |s: &str| s.parse::<HotkeySpec>().unwrap().gnome_accelerator();
        assert_eq!(accel("shift+super+c"), "<Shift><Super>c");
        assert_eq!(accel("ctrl+alt+enter"), "<Control><Alt>Return");
        assert_eq!(accel("alt+pageup"), "<Alt>Page_Up");
        assert_eq!(accel("ctrl+f5"), "<Control>F5");
        assert_eq!(accel("super+."), "<Super>period");
    }

    #[test]
    fn display_roundtrips() {
        let h: HotkeySpec = "super+shift+alt+ctrl+space".parse().unwrap();
        let again: HotkeySpec = h.to_string().parse().unwrap();
        assert_eq!(h, again);
    }
}
