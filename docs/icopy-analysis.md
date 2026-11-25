# iCopy 分析记录

目标：`/Applications/iCopy.app`，cn.better365.iCopy，1.3.4（build 2026050102），Mach-O universal（x86_64 + arm64），Mac App Store 版。

证据分三类：**观察**（工具直接读到的事实）、**推断**（由观察合理推出，未经运行验证）、**未知**。
没有配置反编译引擎（Ghidra / Hopper / IDA 均未配置），所以没有函数体级别的证据。
面板外观与交互另外对照了运行中的 iCopy 截图（见“运行观察”）。

## 使用的工具

| 工具 | 用途 |
| --- | --- |
| REA `inspect-plist` | Info.plist |
| REA `inspect-macho`、`trace-dylib-resolution` | 链接的框架、dyld 解析 |
| REA `inspect-native-dispatch-metadata` | Objective-C 类、方法、实例变量 |
| REA `decode-interface-builder` | 解码 nib（已产出，未逐个分析界面细节） |
| REA `inspect-with-objdump` | 尝试反汇编 `+[ConfigModel defaultConfig]`，参数与 Apple objdump 不兼容，未取得结果 |
| macOS `codesign`、`strings`、`plutil` | 授权、二进制字符串、本地化资源（非 REA） |

`inspect-native-dispatch-metadata` 的上限是 20000 条记录，结果标为 partial：已解码 599 个类，**不保证完整**。

## 观察

**应用形态**
- `LSUIElement = true`：菜单栏应用，无 Dock 图标。登录项 `Library/LoginItems/iCopyHelper.app`。
- URL scheme `iCopy`。沙盒、网络客户端、iCloud 文档容器 `iCloud.cn.better365.iCopy`、用户选择文件读写。
- 链接 AppKit、Carbon、Security、ServiceManagement、StoreKit、libsqlite3。

**类（按职责）**
- 菜单栏与全局：`StatusManager`（`registerHotKey`、`monitorMouseAndKeyboard`、`maskFlagsChanged:`、`showBoard`、`hideBoardAfterPaste`、`preActiveApp`）。
- 数据：`PasteModel`（字段 `_type _text _richText _filePath _data _sourceAppName _createTime _favorite`）、`YLCacheTools`（FMDB 封装）、`AppManager`（`beginMonitorPasteBoard`、`parserPasteboardItem:`、`deletePasteDatasMoreThanDiskLimit`、`autoClearNeedlessPaste`、黑名单读写）。
- 面板：`ListWindowController` / `ListViewController`（纵向表格 + `PastePreviewController`）、`RowWindowController` / `RowViewController`（横向 collection）、`StackViewController`（剪切板堆栈）、`TagsView`（分类标签）、`SearchField`。
- 配置：`ConfigModel`、`HotkeyModel`、`DoubleHotkeyModel`、`FnHotkeyModel`、`PasteHistorySettingModel`、`BlackListAppModel`；快捷键库为 `MASShortcut`。
- 写回：`PasteModel` 的 `writeToSystemPasteBoard…`、`…AndPasteWithCompletion:`、`writeToPasteBoardAndPasteAsPlainText`、`copyTxtWithoutNewline`、`copyTxtWithoutNewlineAndWhitespace`。
- 其它（本项目未复刻）：`Reply*`（快捷回复）、`BackupViewController`（iCloud 备份）、`YLExcelTools`（Excel 导入导出）、`FreeApps*`（自家应用推广）、`YLUpdateManager`。

**配置字段（`ConfigModel` 实例变量）**
`autoBoot`、`showStatusItem`、`confirmationOnDeletion`、`plainTextPreview`、`pasteboardFollowCursor`、`pasteboardAutoHidden`、`soundable`、`pasteboardHoverSwitchGroup`、`pasteboardDefaultFocus`、`pasteboardHeightScale`、`pasteboardDiskLimit`、`historyType`、`positionType`、`pasteboardNumHotkeyFlags`、`pasteboardNumHotkeyAction`、`pasteboardDoubleClickAction`、`boardHotKey`、`boardDoubleFlags`、`boardFnHotkey`、`stackHotKey`、`stackPasteHotKey`、`theme`、`languageType` 等。

**存储（二进制字符串 + 本地化资源）**
- `data.sqlite`：`t_data(id, pastetype, sourcetype, text, filepath, createtime, data blob, favorite)`。
- `reply.sqlite`：`t_reply`、`t_group`。
- 目录：`icons/`（来源应用图标）、`tempFiles/`（图片）、`config.data`、`hotkeyBlackList.data`、`pasteBlackList.data`、`backup_date`。
- 查询形态：按 `pastetype` 过滤（带 `text like` 的查询匹配 2 种类型，带 `filepath like` 的匹配 3 种，另有单类型查询；各数值对应哪种内容未读到）；`(createtime > ? or favorite = ?)` 保留收藏；`text like ? or filepath like ?` 搜索；`order by createtime desc limit ?, ?` 分页。

**本地化文案（zh-Hans）给出的行为**
面板快捷键“唤出/关闭剪切板面板”；数字快捷键；双击/回车；单条粘贴；纯文本粘贴；拷贝（去除换行 / 去除换行和空格）；收藏/置顶；分类；黑名单（“黑名单中的 App，其剪切板内容不会被记录”；“黑名单 App 激活时快捷键被忽略”）；存储时间限制（日 / 周 / 月 / 季度 / 半年 / 年 / 无限制）；磁盘占用限制；跟随光标 / 上次关闭位置；纵向 / 横向样式；悬浮切换分组；删除二次确认。

**运行观察（截图与手动操作，本机系统语言为 en-CN）**
- 面板贴在屏幕底部、全宽、高约 350pt、圆角，位于 Dock 之上；界面为英文。
- 顶部工具栏：搜索框、分类标签 All / Text / Image / File / Collect、右侧 T（纯文本）与设置按钮。
- 横向卡片：头部按类型着色，显示类型（Plain text / Image / File / Folder…）、相对时间（“N minutes ago”）、
  来源应用名与图标；底部为序号、字符数或文件大小（十进制单位，两位小数）、发送与拷贝按钮。
- 右键卡片弹出菜单（粘贴、纯文本 / 富文本粘贴、拷贝及去除换行变体、收藏、删除）；删除弹出“温馨提示”确认。

面板尺寸、颜色和间距是按截图量出的近似值，不是从 nib 或代码读到的。

## 菜单栏（2026-11-24）

**观察**
- REA `inspect-native-dispatch-metadata`（Evidence `ev_6e338b2f9da332c7ba5582017e3817442eebfa2fad2ff766e330a0a9d91fde63`，覆盖 partial，599 个类后触到记录上限）里 `StatusManager` 有 `loadStatusItem`、`resetStatusItemMenu`，以及 `menuShowSetting`、`menuShowBoard`、`menuShowStack`、`menuShowShortcut`、`showStudy`、`showApps`、`contactUs`、`menuQuit`。
- 资源目录 `Assets.car` 里有名为 `status_menu_icon` 的 rendition（Evidence `ev_041f75f0e62cc2df51bf2a8bacf13a8899e8c8232ab986333009410885fc4908`）。REA 不解码图像字节。
- 本机正在运行的 iCopy，用系统辅助功能读到状态栏菜单（menu bar 2），顺序是：Setting、Clipboard、Clipboard Stack、Quick Reply、分隔线、Tutorial、Free Apps、Contact Us、分隔线、Quit。
- 点 Setting 后，辅助功能读到的第一页是剪切板设置：唤出面板快捷键、数字快捷键、按下后、双击/回车、操作后关闭、悬停切换分组、粘贴为纯文本、音效、显示设置、磁盘限制当前值 1GB、存储时间。侧栏按钮是自绘的，辅助功能没有给出标题；本地化表开头的顺序是 Clipboard、Quick Reply、General、Black list、Backup、Study tutorial、Free Apps、Contact us、Buy。

### 设置窗口的外观（2026-11-24 复查）

**观察**
- 本机运行的 iCopy 1.3.4，用辅助功能打开 Setting，再用 `screencapture` 截下 Clipboard、General、Black list 三页（2x，窗口 900×750 pt）。
- 窗口内容延伸到标题栏下，只有红、黄两个按钮；左侧栏约 200 pt 宽，深灰底（取色 `#201f1f`），顶部是应用图标和 `iCopy: 1.3.4`，下面是带彩色圆角方块图标的导航项，选中项是 `#363535` 的圆角底。
- 右侧底色 `#272223`，内容分成圆角卡片（`#211d1e`），每张卡片上方有「彩色小图标 + 标题」。表单行是标签右对齐、控件紧随其后；下拉、快捷键框是 `#322e2f` 的圆角块；勾选框是蓝色（`#0680fa`）圆角方块。
- 存储时间是等距刻度的滑块：Day、Week、Month、Quarter、Half a year、Year、Unlimited，当前档位上方有蓝色图钉，刻度文字是青绿色。
- General 页的下方有「Permissions」卡片（授权说明和三条链接）以及右对齐的「Restore Default Settings」「Initialization」。Black list 页是两张表（剪切板监听、快捷键操作），各带 `+` / `-`。
- REA `inspect-native-dispatch-metadata`（同一份 Evidence，重跑 `--max-records 20000` 仍是 599 个类）读到的结构：`SettingViewController` 有 `bgView`、`splitView`（`SettingSplitView`）、`menuVc`；`MenuViewController` 有 `effectView`、`iconView`、`versionView`、`payBtn`、`currentBtn`；`MenuButton` 有 `imgView`、`titleLabel`；`ShortcutViewController` 有 `box1View`…`box4View`；`GeneralViewController` 有 `box1View`、`box2View`；`BlackListViewController` 有 `pasteTableView`、`operateTableView` 和各自的删除按钮；`PasteHistorySlider` 有 `indicatorView`、`modelArr`。
- `Assets.car` 里有 `paste_icon`、`reply_icon`、`general_icon`、`blacklist_icon`、`iCloud_icon`、`study_icon`、`apps_icon`、`contact_icon`、`shortcut_icon`、`stack_icon`、`display_icon`、`history_icon`、`style_icon` 等 rendition，对应侧栏和分区标题的图标。REA 不解码图像字节，所以 splay 的图标是自画的。
- 本地化表里有 `Interaction Settings`（交互设置）、`Clipboard History Limit`（剪切板历史记录限制）、`Permissions`、`Restore Default Settings`、`Initialization`；「剪切板监听黑名单」出现在 `pasteBlackList.data` 的说明里。

**推断**
- 毛玻璃侧栏来自 `effectView`（`NSVisualEffectView` 的常见命名），没有读到它的 material。splay 用纯色近似。
- 窗口内的光晕背景应是 `bgView` 上的图片（`window_bg_*`），没有复刻。

**未复刻**
- Clipboard Stack、Show Settings（横向 / 纵向样式、文本预览、默认焦点）、Double Click / Fn 修饰键热键、音效、Permissions 卡片、Theme、Preference Panel Shortcut、Toggle Image Button Tooltip；splay 没有对应功能。
- Black list 的第二张表（快捷键操作黑名单）；splay 的快捷键不按来源应用过滤。

**推断**
- `showApps` 对应 Free Apps，`MenuViewController` 的 `pay` 对应 Buy。这两项是推广。
- 设置侧栏的前几项和本地化键的书写顺序一致，但没有逐个点开侧栏核对。

**未知**
- 状态栏菜单各项的快捷键修饰（辅助功能读到的名字没有快捷键）。
- 磁盘限制下拉的完整选项、存储时间滑块的刻度。设置窗口里用的档位是按文案取的，不是从控件读到的。


## 推断（未经运行验证）

- 剪切板变化靠定时器轮询：`AppManager` 有 `_timer` 和 `beginMonitorPasteBoard`，`StatusManager` 有 `_monitorArr`。轮询对象（`changeCount`）是常见做法，但没有读到函数体。
- 粘贴 = 写入系统剪切板后模拟 ⌘V：方法名 `…AndPasteWithCompletion:` 加上“需要辅助功能权限”文案。
- 重复内容上移而不是新增：存在 `update t_data set data = ?, createtime = ? where id = ?`，触发条件未读到。
- 全局热键用 Carbon 热键（链接了 Carbon + MASShortcut），双击修饰键和 Fn 走事件监听（`maskFlagsChanged:`）。

## 未知

- **默认快捷键的具体取值**：`+[ConfigModel defaultConfig]` 的函数体没有取得。本项目按用户给出的 ⇧⌘C 作为默认值。
- 各项默认值（保留时长、磁盘上限、数字键修饰键、面板尺寸）。
- 堆栈（`StackViewController`）的精确语义：从方法名看是“添加到堆栈，逐条 / 全部发送”，发送机制未验证。
- iCloud 备份格式与合并策略。
- 精确的像素尺寸、颜色与动效（只有截图近似值）。

## 复刻对照

| iCopy | splay | 说明 |
| --- | --- | --- |
| ⇧⌘C 唤出面板 | `shift+super+c`（可配置） | X11 内置抓取；Wayland 需绑定 `splay toggle` |
| 文本 / 富文本 / 图片 / 文件 / 多文件 | 同 | 分组仿照 iCopy 的查询；类型数值与库内 `data` 格式自定，不能与 iCopy 的库互读 |
| 搜索、分类标签、分页 | 同 | |
| 收藏 | 同 | 收藏不受保留期与磁盘上限清理 |
| 数字键快速粘贴 | 搜索框外 `1..9`（`number_modifier` 可改 alt / ctrl） | |
| 单击 / 双击 / 回车 | `click_action`、`double_click_action`、`enter_action` | |
| 纯文本粘贴、拷贝（去除换行 / 去除换行和空格） | 右键菜单、工具栏 T 按钮、`paste_plain_text` | 只改写文本，文件仍按文件写回 |
| 悬停切换分组 | `hover_switch_group` | |
| 删除二次确认、清空分类 | 同 | |
| 来源应用黑名单 | 按 WM_CLASS 精确匹配 | 仅 X11 能识别来源 |
| 存储时间 / 磁盘占用限制 | `history_days` / `disk_limit_mb` | |
| 横向卡片条（贴底） | 同 | 纵向列表样式、跟随光标位置未复刻 |
| 来源应用图标（`icons/`） | 同 | macOS 从应用包导出；Linux 取 `.desktop` 的 PNG 图标 |
| 菜单栏图标 | macOS `NSStatusItem`，Linux StatusNotifierItem | 图标是自画的剪贴板剪影，不是 iCopy 的 `status_menu_icon` 像素 |
| 状态栏菜单：设置 / 剪切板 / 退出 | 同 | 实测菜单里还有剪切板堆栈、快捷回复、教程、免费 Apps、联系我们。堆栈和快捷回复的功能未复刻；免费 Apps 是推广；教程和联系我们打开外部页面 |
| 设置：剪切板交互、通用（开机启动、菜单栏图标、删除确认、语言）、黑名单 | 同；外观仿照 iCopy 的深色侧栏 + 分区卡片，快捷键点一下直接录制，存储时间是带图钉的刻度滑块，黑名单是列表加 `+` / `−` | 磁盘档位里除了实测到的 1GB，其余步进是按文案取的；存储时间的天/周/月等是文案档，不是滑块刻度 |
| 去重并上移 | 同 | 以内容摘要判断 |
| 快捷回复、剪切板堆栈、iCloud 备份、Excel 导入导出、主题、音效、双击修饰键 / Fn 热键、纵向列表、导出为图片 / RTF / TXT、免费 Apps / 购买 | **未实现** | 免费 Apps 和购买是推广，不复刻 |
