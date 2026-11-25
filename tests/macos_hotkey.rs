//! macOS 全局热键的真实系统检查。Carbon 事件只能在主线程上分发，所以这个测试不走 libtest：
//! `harness = false` 让 `main` 就是进程主线程，和守护进程的调用方式一致。

#[cfg(target_os = "macos")]
fn main() {
    use std::process::Command;
    use std::sync::mpsc;
    use std::time::Duration;

    use splay::hotkey::HotkeySpec;
    use splay::platform::run_with_hotkey;

    let (tx, _rx) = mpsc::channel();
    let spec: HotkeySpec = "shift+cmd+c".parse().unwrap();
    run_with_hotkey(&spec, tx, |registered| {
        registered.expect("⇧⌘C 注册失败，可能被 iCopy 或其他程序独占");
    });
    println!("ok: shift+cmd+c 可以注册");

    let (tx, rx) = mpsc::channel();
    // 用一个几乎不会和系统快捷键冲突的组合。F12 的虚拟键码是 0x6F，System Events 里的 key code 也是 111。
    let spec: HotkeySpec = "ctrl+alt+shift+f12".parse().unwrap();
    let fired = run_with_hotkey(&spec, tx, move |registered| {
        registered.expect("注册热键");
        std::thread::sleep(Duration::from_millis(200));
        // CGEventPost 合成的按键不会进入 Carbon 热键匹配；System Events 的 key code 会。
        let output = Command::new("osascript")
            .args(["-e", "tell application \"System Events\" to key code 111 using {control down, option down, shift down}"])
            .output()
            .expect("运行 osascript");
        assert!(
            output.status.success(),
            "osascript 失败: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        rx.recv_timeout(Duration::from_secs(2)).is_ok()
    });
    assert!(fired, "热键没有触发");
    println!("ok: System Events 的按键触发了已注册的热键");
}

#[cfg(not(target_os = "macos"))]
fn main() {}
