# ADR 0020：画布内文本编辑——本相**不交付**（DOM 与原生 GTK 两条路均不采用）

- 状态：**Accepted**（2026-10-20，用户决定：**不采用原生 GTK 编辑器**；DOM 路线的否决由证据给出）。
- 范围：宿主 UI 的文本录入形态；**不**改变文本通道本身（`cad_set_tool_text` / `awaiting_text` 仍按
  [ADR-0019](0019-multileader-family.md) 决策 A 实施，用于**停靠式**命令行文本输入）。
- 取代/澄清：[ADR-0010](0010-react-linux-host-replaces-slint.md) `:26-27` 的"不允许跨 CAD 分区的 DOM 浮层"
  **继续有效**，且本文给出其**几何依据**；[ADR-0002](0002-gpu-ui-composition.md) 与该问题**无关**
  （它已被 ADR-0010 取代，是旧的 Slint/Android 合成决策；此前 ADR-0017/0018/0019 与 handoff 曾**误引**它，
  已于 `c2db182` 更正）。

---

## 问题

AutoCAD 的 `TEXT`/`MLEADER` 在**画布内**打开就地文本编辑器。本项目需要一个等价能力吗？

## 证据（勘察结论，逐条带出处）

### DOM 路线：**不可行**（几何前置条件不成立）

1. **CAD 绘图区是 `GtkOverlay` 的最上层 overlay child**：WebView 被 `add_overlay`，随后绘图区被移除并
   **重新 `add_overlay` 到最后**，代码注释明确写着 "keep the CAD drawing area as the topmost overlay child so the
   webview cannot cover it"（`apps/app-linux/src/gtk_bridge/native.rs:33-49`）。
2. **它的原生呈现面不透明**：X11 上绘图区 `set_has_window(true)` 后是**父窗口的子窗口**
   （`owner.rs:45-47`，`native.rs:242-341`）；Wayland 上是**应用自有的 `wl_subsurface`**
   （`owner.rs:431-513`，`apps/app-linux/native/wayland_shim.c:398-416`）。两者在各自矩形内**盖住 WebView**。
3. **只有输入被穿透**：X11 空 input shape（`native.rs:58-79`）、Wayland 子表面空 input region
   （`wayland_shim.c:350-367,437`）。⇒ **能点穿，但不能画上去**。
4. **DOM 节点契约是"无子节点"**：`apps/app-linux/ui/src/components/Workspace.jsx:35-42` 的 `.native-input`
   注明 "Keep this node childless"。
5. **规范禁止**：ADR-0010 `:26-27`。

> ⇒ 要让 DOM 浮在 CAD 之上，必须**改变合成方式**（例如把 CAD 改为经 WebView 呈现），这属于另一份 ADR 的量级，
> 且会牵动 present/latency 与三后端（X11/Wayland/占位平台）。

### 原生 GTK 路线：**不采用**（用户决定），且技术上还有四个未解问题

1. **没有焦点桥**：`InputPolicy::text_focus` / `set_text_focus` / `EscapeAction::DismissTextFocus` 已存在
   （`crates/cad-app/src/input/mod.rs:102-162,299-313`）但**全仓库无任何宿主调用者**；没有任何状态能告诉 CAD
   "文本框有焦点"。
2. **夺焦会取消工具**：把 GTK 焦点从 WebView 移走会触发 DOM `window` `blur`，而 `blur` 是**终态** ProbeInput
   （`apps/app-linux/ui/src/input.mjs:154,188`；`crates/cad-ui-core/src/protocol.rs:99-101`）⇒ 正在进行中的
   工具交互会被取消。
3. **绘图区是 overlay child**：GTK 忽略其位置边距，宿主靠**同步** `queue_resize`+`check_resize` 仪式补偿
   （`owner.rs:106-145`）；瞬时 popup 会扰动该仪式，并与**每帧"绘图区必须匹配 DOM 矩形"守卫**冲突
   （`apps/app-linux/src/host/tick.rs:541-559`，不匹配即**不呈现并报错**）。
4. **Wayland 差异未验证**：popup 是 `xdg_popup`，父必须是 toplevel 表面、位置由合成器调整、无
   override-redirect；而 CAD 是父表面 sub-surface tree 内的子表面，**无法用 `place_above` 相互定序**，
   只能依赖合成器的 popup 堆叠——本宿主**无运行证据**（仅 `scale=1` 的有界 Mutter 运行）。

## 决定

1. **不交付画布内文本编辑**。`TEXT`/`MLEADER` 的文本录入使用**停靠式命令行文本模式**
   （ADR-0019 决策 A.3），文本经 `cad_set_tool_text` 进入会话。
2. **不允许原生 GTK 浮层编辑器**（用户决定）。理由：用户可见 UI 保持**单一来源**（React + i18n 目录）；
   避免把 X11/Wayland 的定位/抓取/堆叠差异带进产品行为；保住 ADR-0010 的组合不变量。
3. **不允许跨 CAD 分区的 DOM 浮层**（ADR-0010 继续有效，且新增**几何依据**见上）。

## 影响与如实声明的能力差距

- 交互形态与 AutoCAD **不同**：用户先拾取点、再在命令行输入文本，而**不是**就地编辑。
  能力表与 handoff 必须**显式声明**该差距，不得暗示"已支持就地编辑"。
- 无实时排版预览（与 P5/P6 的"无真正预览"同源）。
- **不**因此降低诚实性要求：文本通道的拒绝面（空/超长/非待文本/点未齐/待文本期间 Confirm）与
  终端语义**照 ADR-0019 执行**。

## 重新评估的条件（写清，避免"永久封闭"）

本决定只对**当前合成方式**成立。若将来采用以下任一路线，必须**另起 ADR**：
① 把 CAD 改为经 WebView 呈现（DOM 层可行）；② 建立**焦点桥**并证明 popup 不破坏布局仪式与 `blur` 语义，
且在 **X11 与原生 Wayland 上都有运行证据**。
