// Cliptray 在 GNOME Wayland 下的全局热键后端。
// Wayland 不允许普通程序接收全局按键（XWayland 的 XGrabKey 只在焦点是 X11 窗口时有效），
// 所以由 Shell 来抓：本扩展在 session bus 上导出 local.cliptray.Shell，
// `cliptray daemon` 调用 SetShortcuts 并监听 Shortcut 信号。扩展里没有任何业务逻辑。

import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Meta from 'gi://Meta';
import Shell from 'gi://Shell';

import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';

const BUS_NAME = 'local.cliptray.Shell';
const OBJECT_PATH = '/local/cliptray/Shell';
// 由运行中的 Cliptray 守护进程持有；它退出后扩展必须释放热键，不能一直吞掉按键。
const DAEMON_NAME = 'local.cliptray.Daemon';
// Version() 的返回值。守护进程拿它和自己期望的版本比较，Shell 里还是旧扩展时提示重新登录。
// 与 src/platform/gnome.rs 的 EXTENSION_VERSION 保持一致，改这个文件时两处一起加。
const EXTENSION_VERSION = 1;

const INTERFACE = `
<node>
  <interface name="local.cliptray.Shell">
    <method name="Version">
      <arg type="u" direction="out" name="version"/>
    </method>
    <method name="SetShortcuts">
      <arg type="as" direction="in" name="accelerators"/>
      <arg type="as" direction="out" name="failed"/>
    </method>
    <!-- 扩展（重新）启动时发出；守护进程要重新抓热键。 -->
    <signal name="Ready"/>
    <signal name="Shortcut">
      <arg type="s" name="accelerator"/>
    </signal>
  </interface>
</node>`;

class CliptrayService {
    constructor() {
        this._actions = new Map();
        this._dbus = null;
        this._ownerId = 0;
        this._watchId = 0;
        this._signals = [];
    }

    start() {
        this._dbus = Gio.DBusExportedObject.wrapJSObject(INTERFACE, this);
        this._dbus.export(Gio.DBus.session, OBJECT_PATH);
        this._ownerId = Gio.bus_own_name_on_connection(
            Gio.DBus.session, BUS_NAME, Gio.BusNameOwnerFlags.NONE, null, null);
        this._watchId = Gio.bus_watch_name(
            Gio.BusType.SESSION, DAEMON_NAME, Gio.BusNameWatcherFlags.NONE,
            null, () => this._ungrabAll());
        this._signals.push(
            global.display.connect('accelerator-activated',
                (_display, action) => this._onAccelerator(action)));
        this._emit('Ready', null);
    }

    stop() {
        for (const id of this._signals)
            global.display.disconnect(id);
        this._signals = [];
        if (this._watchId) {
            Gio.bus_unwatch_name(this._watchId);
            this._watchId = 0;
        }
        this._ungrabAll();
        if (this._ownerId) {
            Gio.bus_unown_name(this._ownerId);
            this._ownerId = 0;
        }
        if (this._dbus) {
            this._dbus.unexport();
            this._dbus = null;
        }
    }

    _emit(name, signature, ...args) {
        this._dbus?.emit_signal(name, signature ? new GLib.Variant(signature, args) : null);
    }

    Version() {
        return new GLib.Variant('(u)', [EXTENSION_VERSION]);
    }

    SetShortcuts(accelerators) {
        this._ungrabAll();
        const failed = [];
        for (const accelerator of accelerators) {
            const action = global.display.grab_accelerator(accelerator, Meta.KeyBindingFlags.NONE);
            if (action === Meta.KeyBindingAction.NONE) {
                failed.push(accelerator);
                continue;
            }
            // 和 Shell 自己的 GrabAccelerator 一样：不放行的话，窗口管理器会在所有模式下过滤掉这个绑定，
            // 热键永远不会触发。
            const binding = Meta.external_binding_name_for_action(action);
            Main.wm.allowKeybinding(binding, Shell.ActionMode.NORMAL);
            this._actions.set(action, {accelerator, binding});
        }
        return new GLib.Variant('(as)', [failed]);
    }

    _ungrabAll() {
        for (const [action, {binding}] of this._actions) {
            global.display.ungrab_accelerator(action);
            Main.wm.allowKeybinding(binding, Shell.ActionMode.NONE);
        }
        this._actions.clear();
    }

    _onAccelerator(action) {
        const grab = this._actions.get(action);
        if (grab === undefined || Main.sessionMode.isLocked)
            return;
        this._emit('Shortcut', '(s)', grab.accelerator);
    }
}

export default class CliptrayExtension extends Extension {
    enable() {
        this._service = new CliptrayService();
        this._service.start();
    }

    disable() {
        this._service?.stop();
        this._service = null;
    }
}
