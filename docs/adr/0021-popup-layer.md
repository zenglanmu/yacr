# ADR 0021：弹窗层——居中模态窗口与锚定菜单

- 状态：**Accepted**（2026-10-20：D1/D2 已实施并经真实显示验证；**D3 右键菜单**用户决定"后面再考虑实现形式"，钩子保留、形式待定）。
- 日期：2026-10-20。
- 相关：[ADR-0010](0010-react-linux-host-replaces-slint.md)（DOM chrome + 原生 CAD 分区的组合方式，`:26-27`）、
  [ADR-0011](0011-native-wayland-subsurface.md)（原生 Wayland 子表面）、
  [ADR-0019](0019-multileader-family.md)、[ADR-0020](0020-in-canvas-text-editing.md)（文本编辑入口）。
- 影响：宿主 UI（`apps/app-linux`）、输入漏斗、i18n 读取路径；**不**影响 `cad-*` 业务层。

---

## 问题

CAD 绘图区是**不可被 DOM 覆盖**的原生表面。那么后续必然出现的弹窗类 UI——**选项/设置**、**右键随手菜单**、
**文字图元的弹窗编辑**——该用什么机制？

## 约束（事实，附出处）

1. **遮挡是像素级的，不是事件级的。** CAD 分区被重新 `add_overlay` 到**最后**，是最上层 overlay child，
   且是**不透明**原生表面（X11 独立 X child / Wayland app-owned `wl_subsurface`）。WebView 在其**下面**铺满整窗。
   输入穿透（空 input shape/region）只让**输入**过去；**像素不过去**。
   ⇒ CAD 矩形内的 DOM **看不见，但存在、且收得到指针事件**。这是最容易误判的一点。
2. **Wayland 协议禁止 toplevel 定位。** `xdg_toplevel` 没有任何位置请求；只有 `xdg_popup` + positioner 可以锚定，
   且必须给出 parent 表面、非零 anchor rect，并需要 grab serial；"子表面必须与其 parent 相交或至少部分相邻"。
   `gtk_window_move()` / `GTK_WIN_POS_MOUSE` / `CENTER_ON_PARENT` 在 Wayland 上**被丢弃**（compositor 决定）。
3. **GTK3 `GtkPopover` 在 Wayland 上不是 `xdg_popup`。** `popover_realize()` 创建的是 `GDK_WINDOW_SUBSURFACE`，
   即 toplevel 的 `wl_subsurface`，用 `wl_subsurface_set_position` 定位（窗口相对坐标，**从不**是屏幕绝对坐标）。
   更致命：`gtk_window_restack_popovers()` 调 `gdk_window_raise()`，而 **`gdk_window_wayland_raise()` 是空函数**；
   Wayland 的 z 序**只由创建顺序**决定（"A new sub-surface is initially added as the top-most"）。
   ⇒ 若 CAD 子表面在 popover 之后创建（本项目的真实顺序），**CAD 会永远盖住 popover**，且 GTK 没有 API 可修。
   X11 上它是 toplevel 的 `GDK_WINDOW_CHILD`（被窗口裁剪、与兄弟 child 竞争 z 序），同样不可靠。
4. **popover 的 modal 是进程内 GTK grab**（`gtk_grab_add`），不是 X/Wayland seat grab。
   ⇒ 不经过 GTK 路由的原生表面**既不关闭它、也不被它拦住**。
5. **`GtkMenu` 才是真的 popup 表面**：Wayland 上 `xdg_popup` + `xdg_positioner`，X11 上 override-redirect，
   并做 `gdk_seat_grab`（获得外部点击 `popup_done`、键盘导航与 a11y）。
6. **GTK 模态不覆盖应用自己的原生表面。** `gtk_window_set_modal(true)` 只约束 GTK 路由的输入。
   对本项目：CAD 表面**从不接收真实 OS 输入**（输入透明），全部输入经 DOM `.native-input` → `probe_input`
   ⇒ **模态必须由我们在那个唯一漏斗里强制**（见 D2）。

## 决策

### D1 居中模态/对话框 = **第二个 Tauri webview 窗口**

- `WebviewWindowBuilder::new(app, label, WebviewUrl::App(...))`
  + `.parent(&main)`（Linux 上落到 `gtk_window_set_transient_for`）
  + `.decorations(false)` + `.transparent(true)` + `.resizable(false)` + `.focused(true)`，
  并在 GTK 层 `gtk_window().set_modal(true)`（Tauri 2.11.6 **没有** `modal()` 方法）。
- 与主窗**同一 `tauri://` origin、同一 bundle** ⇒ 复用 React chrome 与 i18n 目录；路由经
  `initialization_script` / `emit_to(label, …)`（**不**依赖 `WebviewUrl::App` 的 query——那里只承诺 path 部分）。
- **capabilities 必须显式加入该窗口 label**（默认只列 `main`，否则新窗口**没有 IPC 权限**）。
- **定位不承诺像素**：Wayland 下 `position()`/`gtk_window_move` 对 toplevel 无效，居中由 `CENTER_ON_PARENT`
  与合成器策略决定（Mutter/KWin 通常居中瞬态对话框，但那是**策略不是 API**）。
- **必须由用户手势打开**：Wayland `xdg_activation` 禁止自激活，无合法 serial 时可能只被标记 urgent；
  优先 `present_with_time(event_time)` 而非 `GDK_CURRENT_TIME`。
- **不采用**：`gtk_dialog_new()` + 手写 `WebKitWebView`（无 Tauri IPC，需自建 `script-message` 桥、双 web 进程、
  主题/字体对齐靠手工）。
- **不采用**：`GtkPopover` 承载 WebView（D3 的理由，且 "WebView in popover" 无任何官方支持声明）。

### D2 模态由**我们在输入漏斗里**强制（不靠 GTK）

- 打开模态即置 `input_gate = Suspended`：`.native-input` 的 `probe_input` 转发、以及除"关闭/Esc"外的全局快捷键
  **全部停**；同时 **host 侧二次拒绝** CAD 命令（不依赖 UI 自觉——UI 与被覆盖的区域可能不同步）。
- 关闭/取消后恢复，并**重新校验当前工具相位**（模态期间宿主状态可能变化，恢复不能假设还是同一相位）。
- 这一条在本项目比在通用 GTK 应用里**便宜得多**：CAD 输入本来就只有**一个漏斗**。
- **与既有 `suspendPointer` 的关系（must）**：命令行的 `onBlur` **无条件**解除挂起，而外部点击本身会先触发 `blur`
  （`releaseFocusOnOutsidePress`）⇒ **模态期间 `input_gate` 必须自己重新断言**，不能把"挂起"交给单一所有者，
  否则**一次外部点击就把模态保护解除**。两者是**独立开关**，`input_gate = Suspended` 时 `accepting` 恒为 false。

### D3 锚定菜单（右键）= **原生 `GtkMenu`**

- 用 `gtk_menu_popup_at_pointer`（或 `popup_at_rect`），它以**当前指针**为准 ⇒ **不需要屏幕绝对坐标**，
  因此在 Wayland 也可用；并获得真实 seat grab（外部点击关闭）、键盘导航与 a11y。
- 菜单项文本**只能**来自 `crates/cad-ui-core/i18n`（Rust 侧读目录构建标签），禁止硬编码。
- 外观用 `GtkCssProvider` 近似 chrome；**声明偏离：原生菜单观感 ≠ React chrome**。
- 触发链：DOM `contextmenu`（已有 `cadContextMenuRequest({point, selection, tool})` 钩子）
  → **新 IPC `cad_open_context_menu(point)`** → host 在主线程 popup。
- **不采用**：无边框 Tauri 窗口定位到光标（Wayland 不可能；X11-only 会造成**后端观感分叉**）。
- **不采用**：`GtkPopover`（约束 3/4）。
- **不采用**：把菜单画成 DOM 后"盖在"CAD 上（约束 1）。

### D4 文字图元的**弹窗编辑**（原 ADR-0020 的 C）重新纳入

- ADR-0020 否决的是"独立窗口弹窗编辑"；用户 2026-10-20 明确：**text/mtext/mleader 图元类接受弹窗编辑，
  除右键菜单外居中即可**。⇒ **C 以 D1 的模态窗口形式重新纳入**。
- **A（`TEXTEDIT`）与 B（属性面板字段）保留**为快速路径；**C** 用于**多行 / MTEXT / 长文本 / MLEADER（P7b 之后）**，
  以及"需要看见原文格式"的场景。
- C **不改** A/B 的任何实现，只增加一条入口；编辑语义仍是**整串替换 + 单事务 + 一步撤销 + 有效值往返**
  （ADR-0020 增补已定）。

### D5 画布内**就地**编辑（冻结帧）= **不采用**，保留为未来选项

- 机制：wgpu 读回一帧 → `<img>` 铺满 CAD 矩形 → 隐藏原生子表面 → DOM 在其上编辑。
- 代价：4K 一帧 ≈ 33 MB（base64 ≈ 44 MB）过 IPC；**编辑期间渲染冻结**（不能平移/缩放）；
  需新建 GPU 回读路径；`map/unmap` 与逐帧"绘图区必须匹配 DOM 矩形"的守卫互相牵制。
- 只有"必须**在图纸原位**编辑"时（而非"以图纸为背景"）才值得；当前瓶颈不在这里。

### D6 chrome 内浮层：**已审计，规则已在代码里**

- **审计结论（2026-10-20，纠正本文初稿的猜测）**：唯一的绝对定位浮层是 ribbon 的**变体下拉**
  （`.ribbon-panel__menu`），它**已经**是 **in-band popover**：定位在 `.ribbon-panel__body`（`position: relative`）
  之内，`max-height: 100%` + 内部滚动 ⇒ **不会画到 ribbon 带之外**，因此**不会**被 CAD 原生表面遮挡。
  该约束**已有源码注释**（`apps/app-linux/ui/src/style.css:243-247`：「nothing paints below the ribbon (which would be
  hidden behind the native CAD surface) … it cannot extend past the body rectangle」）——即**本项目早就知道并处理了**，
  本文初稿把它列为"现存隐患/首要嫌疑"是**误判**。
- 其余候选（命令行的 `completions`、`history-panel`、palette）**不是绝对定位**，在文档流内 ⇒ 它们改变的是
  chrome 布局（CAD 矩形随之重算并触发原生表面 resize），**不是遮住**。
- **规则（对未来的浮层）**：chrome 内浮层**必须**约束在自身 chrome 矩形内（in-band / 内部滚动 / 向上翻转）；
  **需要更多空间或需要锚定到 CAD 区**的，走 D1 的窗口或 D3 的原生菜单。

## 后果

- 新增：窗口生命周期管理、input gate 一个布尔相位、`cad_open_context_menu` IPC。
- **唯一明确牺牲**：右键菜单是**原生外观**（无法与 React chrome 像素一致）。
- 不再假设"DOM 可以画在 CAD 上"：凡跨 CAD 矩形的 UI，要么是**独立表面**（窗口），要么在 **chrome 内**。

## 验证计划（真实显示，按后端分别记录）

1. **模态窗口**：新 label 的 `invoke` 权限；Mutter/KWin 下 transient 置顶与居中；`transparent(true)` 圆角；
   **模态期间点/滚轮/快捷键确实不进入 CAD**（gate 生效）；Esc 关闭后恢复且**相位重校验**。
2. **`GtkMenu`**：X11 与 Wayland（Mutter）下 `popup_at_pointer` 的出现位置、外部点击关闭、键盘导航；
   `WAYLAND_DEBUG=1` 确认走 `xdg_popup`（**不是** `wl_subsurface`）。
3. **反证（保留为"为什么不选 popover"的证据）**：`GtkPopover` + WebView 在 Wayland 下被 CAD 子表面遮挡。
4. **证据边界**：以上均为**有界本地**（GNOME/Mutter + Quadro P620 + `scale=1`）；**不**推广为其它合成器、
   其它 DPI、vendor 兼容或真机结论。

## 未决（需用户定）

- **右键菜单是否接受原生外观**（D3）。若不接受，唯一"观感一致"的方案是 X11 用 DOM 窗口 + Wayland 用原生菜单
  ⇒ **后端分叉**；本 ADR 建议**接受原生**（一致性 > 像素一致）。
- **C（文字弹窗编辑）的排期**：紧跟 A/B，还是等 P7b（MultiLeader）之后一次做全。

## 实施状态（2026-10-20，第 1 步：管线骨架 + `about`）

- **窗口**：`apps/app-linux/src/host/popup.rs`；`WindowBuilder` + `add_child(WebviewBuilder)`（与主窗同构，**不用** `WebviewWindowBuilder`），
  `popup-1` / `popup-1-webview`、520×360、无边框、不可缩放、`parent(main)` + GTK `set_modal(true)` + `CenterOnParent`（Wayland 忽略）。
- **透明**：`Screen::is_composited()` **实测**决定，日志写 `composited=true transparent=true`；不按后端名推断。
- **载荷**：`initialization_script` 注入 JSON 字面量 `window.__YACR_POPUP__ = {kind,modal,label,payload:{version,build}}` ⇒ 无竞态；
  UI 侧缺省/畸形/未知 kind **一律回落普通外壳**。
- **IPC**：`cad_open_popup{kind}`（**允许清单**，未知 ⇒ `popup.unknown_kind`）、`cad_close_popup`（**幂等**）。
- **capability：不需要**。仓库无 app ACL manifest ⇒ 本地 origin 的应用命令对所有窗口可用（核实过，不是猜）。
- **gate（D2 落地）**：`ShellState.popup`（UI 工具包无关）为权威；**只在窗口 `Destroyed` 清除** ⇒ IPC/Esc/窗管关闭**同一条路径**；
  开窗前对账陈旧记录，**漏事件不会把 CAD 输入卡死**；`probe_input`（除 `resize`）与 `cad_command` 拒绝 `popup.modal_open`。
- **UI 侧独立开关（N5 落地）**：挂起有两个独立理由 `textFocus` / `popupOpen`，命令行 `onBlur` **只能清自己的**；
  有回归测试证明"弹窗开着 + 命令行失焦 ⇒ 仍然挂起"。
- **状态**：`cad_status.popup`（`null` | `{kind,label,modal}`），始终存在。
- **证据（真实显示，有界）**：`popup_pass=1`；`status_before=-`、`status_open=about:popup-1:true`、`status_after=-`；
  `input_refused=popup.modal_open`、`command_refused=popup.modal_open`；关后 `input_admitted=1`、`command_admitted=1`。
  弹窗自身窗口被 `x11-window-grab <xid>` 抓到（520×360），像素判读**接受**：无模板占位符泄漏、无缺字、tokens 正确、圆角/边框正确；
  **四处布局缺陷（中部 195px 空洞、内边距不一致、关闭图标不对称、标头字重偏弱）已修并复判为 fixed**。
- **已知小瑕疵（未修，不阻塞）**：版本行 `版本 0.1.2（release）` 中全角括号左侧字面留白偏大，**读起来像多了一个空格**；
  未知 kind 的文案复用 `status.toggle_unsupported`（**当前不可达**，宿主允许清单挡在前面）。
- **未做**：D3 右键菜单（用户暂缓）；D4 的"文字弹窗编辑"（后端 `SetSelectedText` 已就绪，只差把对话框接上）；
  D5 冻结帧（明确不采用）。
