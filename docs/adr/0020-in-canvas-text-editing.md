# ADR 0020：画布内文本编辑——本相**不交付**（DOM 与原生 GTK 两条路均不采用）

- 状态：**Accepted**（2026-10-20，用户决定：**不采用原生 GTK 编辑器**；DOM 路线的否决由证据给出）。
  **2026-10-20 局部修正**：本文否决的是"独立**窗口**弹窗"这一**机制不解**；用户随后明确
  **text/mtext/mleader 图元类接受弹窗编辑（居中即可）**，且 [ADR-0021](0021-popup-layer.md) D1 给出了可行机制
  ⇒ **入口 C（弹窗编辑）以模态窗口形式重新纳入**；**A（`TEXTEDIT`）与 B（面板可编辑字段）不变**。
  **"画布内就地编辑"与"原生 GTK 就地编辑器"仍不交付**（ADR-0021 D5 保留为未来选项）。
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

---

## 增补（2026-10-20）：**编辑现有** TEXT/MTEXT 的入口形态 —— 采用 **A + B**

"创建"与"编辑"是两件事：创建已在 ADR-0019 决策 A/F.1 定案；本节定**编辑**。

### 已具备的前提（勘察核实）

- **就地写入路径已存在**：`cad-db` 的 `update_entity`（`drawing/transaction.rs:76`），
  TRIM/EXTEND/STRETCH/CHAMFER 均在用（`trim_extend.rs:203,348`、`stretch.rs:357`、`chamfer_fillet.rs:212`），
  每次都在**一条事务**里并**重新校验**。
- **缺**的是两样：① 把"当前文本"以**可回填**形式送给 UI（属性面板的 `PropertyRow`
  （`cad-app/src/selection.rs:22`）是**展示字符串**，不可回填）；② 一个**编辑入口**。

### 决定

1. **A：`TEXTEDIT` 命令**（选中图元 → 当前文本回填到文本通道 → 输入替换 → `update_entity` → 一条事务/一步撤销）。
   复用 ADR-0019 决策 A 的 `SetToolText` 通道与**同一套拒绝面**（空/超长/非待文本/点未齐/待文本期间 Confirm）。
2. **B：停靠属性面板里的可编辑文本字段**。面板位于 CAD 矩形**之外**（属 chrome）⇒ **不越墙**。
   需要一个新的状态字段，携带**实体身份 + 可回填的当前文本**（**不得**复用 `PropertyRow` 的展示字符串）。
3. **C（独立窗口弹窗）不采用**：它技术上可行，但夺焦会让主窗口收到**终态** `blur`（`protocol.rs:99-101`）
   从而取消在制工具，再加上多窗口生命周期，**收益相对 B 很小**。记录为**未来可选**。
4. **D（主窗口内盖在 CAD 矩形上的 DOM 浮层）仍不可行**（见上文几何证据）。

### 明确声明

- 编辑是**整串替换**（无光标级编辑、不解析/重排 MTEXT 内联标记）。
- 导入的 **MTEXT** 编辑的是**已折叠的 `Text`**（列/背景/行距在上游已丢，P7a 起报 `Partial`）。
- **MLEADER 的文本编辑推迟到 P7b**：今天其文字是 `Compound` 的**子项、无可寻址身份**，
  "编辑它"会退化为"改某个子项"——必须等一等表示落地后再做（届时 A/B 两种入口都可复用）。

### 实施（A+B，2026-10-20）

- **A：`TEXTEDIT`**——`DrawToolKind::TextEdit`：**先实体拾取 1 次**（`picked_step_count = 1`）→ 文本步**预填**
  当前字符串（`text_prefill`）→ `SetToolText` **终端完成** → `update_entity`：**只替换字符串**，
  样式/高度/旋转/位置/font/对齐**原样保留**，**一条事务/一步撤销**（撤销标签 `edit text`）。
- **B：面板编辑**——`InteractionRequest::SetSelectedText { text }` + Tauri `cad_set_selected_text { text }`
  （有界准入，返回 `CadCommandAck { accepted: true }`）。**payload 不含实体 id**：宿主解析**当前单选**并在
  应用时**重新校验**；`CadSelection.text` 携带**可往返**的当前字符串（与展示用 `PropertyRow` 分离）。
- **UI**：属性面板（chrome，**在 CAD 矩形之外**）内用 **`textarea`**（`input` 会吞换行、破坏逐字往返），
  逐字显示/回传；选择变化时草稿**保留为 detached**（Apply 停用 + 说明如何恢复，**绝不**应用到别的对象）；
  空文本**可点但具名拒绝**；超限显示 `{used} / {max}` 且**不截断**；`Ctrl/Cmd+Enter` 提交、**普通 Enter 是换行**；
  IME 双信号不提交；异步失败**回滚到图纸实际值**（宿主权威）。
- **契约核对**：`text_prefill` 用 **snake_case**（`CadTool` 是 camelCase 重命名，漏写会被静默忽略）；
  `cad_set_tool_text` 与 `cad_set_selected_text` **都**返回 `CadCommandAck { accepted: true }`（与前端判定一致）。
- **拒绝面（具名、不开事务）**：`textedit.target_not_text` / `target_missing`（含块内成员与子元素）、
  `tool.text_empty`、`tool.text_too_long`（1024 `chars()`）、`tool.text_not_awaiting`、`tool.text_points_incomplete`、
  `textedit.selection_empty` / `selection_multiple`。
- **声明**：`TEXTEDIT` **无 ribbon 入口**（打字命令，且有选择时面板编辑器即主入口）；拾取提示键
  `tool.prompt.textedit.select_entity` 已补；`status.tool_parameter.text_height` 的 UI 步进器**仍未接线**（默认 2.5）。
