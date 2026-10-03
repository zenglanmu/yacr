# 命令、输入与选择系统功能需求清单

本文档反推自 `src/command.rs`、`src/app/command_driver/`、`src/input/`（含 `trackpad.rs`、`spacemouse/`）、`src/app/navigation.rs`、`src/app/shortcuts.rs`、`src/app/alias.rs`、`src/app/drafting_settings.rs`、`src/modules/draw/select.rs`、`src/modules/draw/fence.rs`、`src/ui/overlay.rs`、`src/ui/command_line.rs`、`src/ui/popup/`、`src/ui/window/{drafting_settings,alias_editor,shortcuts}.rs`、`src/ui/statusbar/`、`src/snap.rs`、`src/scene/pick/{grip,selection_state}.rs` 以及 `assets/ocad.pgp`。覆盖命令行、命令提示与选项、透明命令、动态输入、对象捕捉/追踪/极轴/正交/栅格、选择方式、夹点、鼠标与触控板手势、键盘快捷键、命令别名等面向用户的交互。

---

## 一、命令行与命令执行

### 命令输入框
- **功能简介**：位于窗口底部的命令行输入框，输入命令名、关键字、数值、坐标或文件路径并回车执行。
- **UI 入口**：`主窗口 → 底部命令行 → 输入框（cmd_input）`。
- **样式**：`text_input` 尺寸 11，内边距 `[4, 30, 4, 6]`；输入行容器高度固定 30px（与标签栏/状态栏对齐），背景为主题 `background.weakest`。
- **触发命令**：任意（`Message::CommandInput` / `Message::CommandSubmit`）。
- **实现位置**：`src/ui/command_line.rs::CommandLine::view`、`submit`；处理 `src/app/update/command.rs::on_command_submit`。
- **备注**：提交时仅把首个 token（命令动词）大写，其后的参数保留原始大小写（保护大小写敏感路径/标识符）；输入被记录进历史。

### 命令提示行（Pin 住的当前步骤提示）
- **功能简介**：命令运行中，当前步骤的提示会固定在输入框上方显示且不淡出；步骤变化后旧提示恢复淡出。
- **UI 入口**：`主窗口 → 命令行 → 输入框上方提示行`。
- **样式**：`INFO_PREFIX = "ⓘ "` 前缀；尺寸 11；当前提示为唯一 `pinned` 行并被推到最近历史之上；提示颜色由 `accessible_accent_threshold` 保证对比度。
- **触发命令**：命令运行期间自动（`CommandLine::set_step_prompt`）。
- **实现位置**：`src/ui/command_line.rs::set_step_prompt`、`entry_visible`、`HistoryEntry.pinned`。
- **备注**：`CLIPROMPTLINES`（0–50，默认 3）控制输入框上方临时提示行数量；`COMMANDLINEFADETIME`（0–60000ms，默认 3000）控制淡出时长；0 表示不绘制临时行。

### 命令选项按钮（Clickable Options）
- **功能简介**：命令当前步骤的中括号关键字选项（如 `[Close/Undo]`）渲染为提示旁的按钮，点击等同于键入该关键字，无需手打。
- **UI 入口**：`主窗口 → 命令行 → 当前提示行右侧的选项按钮`。
- **样式**：`button(text(opt.label.to_uppercase()).size(11))`，内边距 `[1, 6]`，圆角 3；默认背景 `background.weakest`，悬停/按下为 `primary.weak`；提示行上会剥离原 `[A/B]` 列举文本（`strip_option_listing`）。
- **触发命令**：命令运行期间自动（`Message::CommandOptionPick(keyword)`）。
- **实现位置**：`src/ui/command_line.rs::view`（`step_options` 分支）、`set_step_options`；数据结构 `src/command.rs::CmdOption`（`new` / `enter`）；各命令 `options()`。
- **备注**：`CmdOption.keyword` 为空表示"等同回车/默认动作"的完成按钮（`CmdOption::enter`）；点击走与键入完全相同的 `on_text_input` 路径。

### 命令历史浮层与完整历史下拉
- **功能简介**：命令行会短暂显示最近的历史行（命令、输出、错误、信息），并可点开完整档案下拉查看/复制/清空全部历史。
- **UI 入口**：`主窗口 → 命令行 → 输入框右侧 ▸/▾ 按钮（CommandHistoryToggle）`。
- **样式**：历史浮层行尺寸 11；颜色区分类型——命令为 `background.base.text`、输出 `scale_alpha(0.72)`、错误 `danger`（加粗）、信息 `primary`；下拉为单个只读 `text_editor`（支持跨行拖选、Ctrl+C 拷贝），可调高度（`HISTORY_HEIGHT_MIN=72`、默认 180、最大 560，且 ≤ 窗口高 60%），头部有 Copy / Clear 按钮，右侧 `RESIZE` 句柄可拖动/双击复位。
- **触发命令**：`COMMANDHISTORY`（F2）；下拉触发 `Message::CommandHistoryToggle`。
- **实现位置**：`src/ui/command_line.rs::view`、`toggle_history`、`history_plain_text`、`clear_history`；快捷键 `src/app/shortcuts.rs::run_action`（`COMMANDHISTORY`/`HISTORYPREV`/`HISTORYNEXT`）；处理 `src/app/update/command.rs`。
- **备注**：仅保留最多 `MAX_HISTORY = 64` 条；错误有 `push_error_once` 去重（#498）；警告 `push_warning` 以错误样式显示但不影响 `last_error`。

### 命令历史上下导航（↑/↓）
- **功能简介**：在输入框中按 ↑/↓ 回调此前已成功执行的命令，并暂存当前草稿。
- **UI 入口**：`命令行输入框 → ↑ / ↓`。
- **样式**：无额外控件；输入框文本替换。
- **触发命令**：↑ = `HISTORYPREV`，↓ = `HISTORYNEXT`。
- **实现位置**：`src/ui/command_line.rs::history_prev`、`history_next`、`cmd_recall`；键 `src/app/shortcuts.rs::default_bindings`。
- **备注**：`cmd_recall` 最多 50 条，连续重复跳过；开始导航时保存草稿，向下越过最新项后恢复草稿。

### 字面空格模式（`>` 开关）
- **功能简介**：开启后每一行都按以 `>` 开头处理——空格保留在输入中而不提交，便于输入含空格的文本、路径等。
- **UI 入口**：`主窗口 → 命令行 → 提示 "❯" 左侧的 ">" 按钮（CommandLiteralToggle）`。
- **样式**：`button(text(">").size(11))`，内边距 `[2, 6]`，圆角 3；激活时背景 `primary.weak`；按钮带 tooltip（工具提示文本："Literal spaces: Space stays in the line instead of running the command (same as typing a leading '>'). Stays on until toggled off."）。
- **触发命令**：`Message::CommandLiteralToggle`；持久保存在用户配置。
- **实现位置**：`src/ui/command_line.rs::literal_spaces`、`view`（`literal_active`）；locale `literal-spaces-space-stays-in-the-line-instead`。
- **备注**：手工在输入首部键入 `>` 也会点亮该按钮（仅该行生效）。

### 自动补全下拉（Autocomplete）
- **功能简介**：输入命令名前缀时，在输入框上方弹出匹配命令列表，最多 8 条，前缀匹配优先，可用方向键选择、Enter/点击执行。
- **UI 入口**：`主窗口 → 命令行 → 输入框上方自动补全面板`。
- **样式**：每行为 `button`（尺寸 11，图标 14px + 命令名），选中行背景 `primary.weak`，悬停 `background.weak`，面板 `bordered_box` 全宽。
- **触发命令**：键入即触发（`Message::CommandSuggestionPick`）。
- **实现位置**：`src/ui/command_line.rs::autocomplete_matches`、`ranked_matches`、`autocomplete_prev/next`、`selected_suggestion`、`AUTOCOMPLETE_LIMIT=8`。
- **备注**：命中来源 = 编译期 `inventory` 命令注册表 + 插件动态命令（#272）；别名本身被隐藏，但其目标命令仍显示（#288）；当整个输入恰为别名时，其目标被置顶（如 `AA`→`AREA`、`L`→`LINE`）；大小写不敏感子串匹配。

### 命令补全/校验错误提示
- **功能简介**：命令被拒绝或输入非法时，命令行以错误样式打印消息并重新显示当前提示。
- **UI 入口**：`命令行历史行`。
- **样式**：`ERROR_PREFIX = "✕ "` + "Invalid" 大写前缀；错误色为 `danger`（加粗）。错误前缀常量化于 `format_error`。
- **触发命令**：`CmdResult::ReportError`、`CancelWithMessage`、`reprompt_active_command`。
- **实现位置**：`src/ui/command_line.rs::push_error`、`push_error_once`、`format_error`；`src/app/command_driver/mod.rs::reprompt_active_command`。
- **备注**：历史条目 `EntryKind::{Command, Output, Error, Info}`。

### 命令行信息/输出前缀
- **功能简介**：信息与输出行使用固定前缀区分于命令与错误。
- **UI 入口**：`命令行历史行`。
- **样式**：命令前缀 `COMMAND_PREFIX = "❯ "`（"Command: " + 命令）；信息前缀 `INFO_PREFIX = "ⓘ "`。
- **实现位置**：`src/ui/command_line.rs::push_command`、`push_info`、`push_output`。

### 命令重复（Idle Enter 重复上一命令）
- **功能简介**：无活动命令时按 Enter 会重新执行上一次命令。
- **UI 入口**：`视口 / 命令行`。
- **触发命令**：`ENTER`（`FINALIZE`）。
- **实现位置**：`src/app/update/command.rs::on_command_finalize`（`last_cmd` 分支）；默认绑定 `("ENTER","FINALIZE")`。
- **备注**：有非空输入先提交输入；有活动命令则走 `StepInput::Enter`；夹点热时 Enter 落位。

### 空格作为回车
- **功能简介**：空格在命令提示处等同于回车提交当前步骤（单 token 输入时）。
- **UI 入口**：`命令提示处`。
- **触发命令**：`SPACE`（`COMMANDSPACE`）。
- **实现位置**：`src/app/shortcuts.rs::default_bindings`（`("SPACE","COMMANDSPACE")`）、`run_action`；`InputKind::SingleToken` 语义见 `src/command.rs::InputKind`。
- **备注**：自由文本步骤中空格是字面字符；点步接受关键字（`point_step_accepts_keywords`）时数字进输入、字母作关键字。

---

## 二、透明命令与命令中断/恢复

### 透明绘图辅助命令（Ortho/Grid/Snap/Polar/Osnap/DSettings）
- **功能简介**：这些开关类命令在另一命令运行中执行时不打断当前命令，只翻转标志（如 MOVE 中按 F8 只约束后续）。
- **UI 入口**：命令运行中键入 `ORTHO`、`GRID`、`SNAP`、`POLAR`、`OSNAP`、`DSETTINGS` 或对应功能键。
- **触发命令**：`ORTHO`、`GRID`、`SNAP`、`POLAR`、`OSNAP`、`DSETTINGS`。
- **实现位置**：`src/app/commands/mod.rs::is_transparent`、`dispatch_command_inner`（`transparent` 分支）。
- **备注**：透明命令绕过命令拆除逻辑（#677）。

### 透明导航命令（`'PAN` / `'ZOOM`）
- **功能简介**：以单引号前缀（或右键菜单里的导航行）运行 PAN/ZOOM，在另一命令中间平移/缩放，结束后自动恢复原命令。
- **UI 入口**：`命令行键入 'PAN / 'ZOOM …`；`视口 → 右键 → 命令菜单 Pan / Zoom`。
- **样式**：右键菜单导航行带图标（chrome pan/zoom 图标），快捷键提示见右栏。
- **触发命令**：`'PAN`、`'ZOOM`、`'ZOOM DYNAMIC`、`'ZW`/`'ZE`/`'ZA`/`'ZP`/`'ZI`/`'ZO`/`'ZD`/`'ZEA`/`'ZOBJ`。
- **实现位置**：`src/app/commands/mod.rs::is_transparent_capable`、`zoom_prompts`；`src/app/command_driver/mod.rs::resume_transparent_parent`。
- **备注**：交互式 ZOOM（窗口/对象）会 park 当前命令（`suspended_cmd` + `transparent_resume`），结束后恢复；一次性缩放立即恢复。

### MTP / M2P 两点间中点（命令暂停修饰符）
- **功能简介**：在任意需要点输入的命令中调用"两点间中点"，暂停当前命令，拾取两点后把其中点回填给父命令。
- **UI 入口**：`命令行 → MTP / M2P`；`视口 → Shift+右键 → 捕捉替代网格 → Mid Between 2 Points`。
- **样式**：预览为青色橡皮筋线 + 青色三角形中点标记（线宽随距离缩放 `(dist*0.02).clamp(0.5,10)`）。
- **触发命令**：`MTP`（`M2P`）。
- **实现位置**：`src/command.rs::Mid2PointCommand`、`src/app/command_driver/mod.rs::start_mtp_modifier`；右键 `MenuAction::Mtp`。
- **备注**：提示 `_mtp Specify first point of mid:` / `_mtp Specify second point of mid:`。

---

## 三、动态输入（Dynamic Input）

### 动态输入坐标框（像素级提示）
- **功能简介**：在光标处显示可编辑数值框——点步显示 X/Y（或无基点时）、有基点时显示"距离 < 角度"（极坐标）、距离/角度/半径/直径/宽/高/标量按命令步骤而变化；也可显示命令提示与追踪提示。
- **UI 入口**：`视口 → 光标附近浮层`（动态输入开启时）。
- **样式**：`dynamic_input_overlay`；框高 `DYN_BOX_H`、字体 `DYN_FONT`、内边距 `DYN_PAD`、字符宽 `DYN_CHAR_W`；活动框背景 `primary.weak`、边框 `primary.base`（宽 1.6），锁定（已键入）框背景 `warning.weak`、边框 `warning.base`，普通框背景 `background.weak`/边框 `background.neutral`；空内容框不显示标签，角度框直接显示已格式化含 `°` 的值。
- **触发命令**：`DYNINPUT`（F12）开启/关闭。
- **实现位置**：`src/ui/overlay.rs::dynamic_input_overlay`、`DynInputCanvas::{draw_box,box_colors,draw_prompt,draw_tracking_hint}`；宿主 `src/app/update/dynamic.rs::sync_dyn_fields`、`apply_dyn_spec`、`dyn_resolve_point`；字段模型 `src/app/document.rs::DynFieldEntry`；命令侧 `src/command.rs::{DynField,DynRole,DynSpec,DynGuide,DynAnchor,DynFieldSpec}`。
- **备注**：字段角色 `DynRole` 含 X/Y/Z/Distance/Angle/Radius/`⌀`/Width(W)/Height(H)/Factor/Count(#)；引导线 `DynGuide` 含 Polar（+X 参考线与角度弧）、AxisDelta（虚线投影到锚点 X/Y 轴）、Radius（锚点→光标线）、RectSides（宽×高两边）、Perp / PerpDim（椭圆短半轴/矩形高，尺寸样式）；直径框显示/接受两倍半径（`value_scale=2.0`）。

### 动态输入键盘与逗号/坐标模式
- **功能简介**：动态输入字段聚焦时，数字与表达式字符（`0-9 . - + * / ^ % ( )`）进入字段缓冲；字母仍走命令行关键字；`,` 锁定当前字段并推进到下一坐标（可在极坐标→笛卡尔、2D→3D 间重塑）；`@`/`#` 切换相对/绝对坐标模式。
- **UI 入口**：`视口光标处动态框`（输入框未聚焦时）。
- **样式**：见上条。
- **触发命令**：无独立命令；键入驱动。
- **实现位置**：`src/app/update/command.rs::on_command_append_char`（`,`、`@`/`#`、`dyn_field_char` 分支）、`on_command_backspace`；`src/app/update/dynamic.rs::dyn_comma_advance`、`dyn_set_coordinate_mode`、`dyn_has_coordinate_fields`。
- **备注**：`Backspace` 先编辑活动动态字段，清空即解锁并回到光标跟踪；`DynFieldEntry.buffer==None` 表示跟随光标。

### 动态输入 Tab 切换字段
- **功能简介**：在动态输入多个字段间切换焦点。
- **UI 入口**：`视口动态框`。
- **触发命令**：`TAB`（`DYNTAB`）。
- **实现位置**：`src/app/shortcuts.rs`（`("TAB","DYNTAB")`）、`run_action`（`DynTabNext`）。
- **备注**：另有逗号 `,` 推进字段（见上条）。

### 动态输入/指示提示框（命令提示 + 追踪提示）
- **功能简介**：在光标附近显示命令当前提示与对齐参考标签（追踪提示）。
- **UI 入口**：`视口光标附近`。
- **样式**：提示框填充 `background.strong`，边框 `primary.base.scale_alpha(0.9)`，文字 `background.strong.text`。
- **实现位置**：`src/ui/overlay.rs::DynInputCanvas::draw_prompt`、`draw_tracking_hint`。

### 夹点动态输入（单字段）
- **功能简介**：夹点编辑时动态框显示与编辑模式匹配的标量：拉伸/矩形缩放显示距离+角度/宽+高；拉长、半径、弧长、矩形宽高、平行移动显示单个对应字段（拉长等还可把键入值同步进命令行）。
- **UI 入口**：`选中对象 → 光标悬停夹点 → 拖动时`。
- **样式**：同动态框；矩形框用 `DynGuide::RectSides`，标量拉伸用 `DynGuide::Radius`，普通拉伸用 `DynGuide::Polar`。
- **实现位置**：`src/app/update/dynamic.rs::sync_dyn_fields`（`grip_input` 分支）、`src/scene/pick/grip.rs::GripEditMode::uses_scalar_dynamic_input`。
- **备注**：`GripEditMode` 含 Stretch、Lengthen、Radius、ArcLength、RectangleWidth、RectangleHeight、RectangleResize、MoveParallel。

---

## 四、对象捕捉（OSNAP）

### 对象捕捉总开关（Status → OSNAP）
- **功能简介**：开启/关闭全部对象捕捉；图标左侧主按钮切换全局开关，右侧 ▾ 打开捕捉模式列表。
- **UI 入口**：`状态栏 → 对象捕捉按钮（主按钮切换 / ▾ 展开列表）`。
- **样式**：`split_pill` 包裹的图标 + 下拉箭头；激活时背景 `primary.weak`、边框 `primary.base`。
- **触发命令**：`OSNAP`（F3，`TOGGLEOSNAP`）。
- **实现位置**：`src/ui/statusbar/mod.rs::osnap_btn`；`src/snap.rs::Snapper::{toggle_global,is_active}`；快捷键 `src/app/shortcuts.rs`。
- **备注**：主按钮 tooltip："Object Snap: toggle on/off\nF3"。

### 二维对象捕捉模式列表（Snap Popup）
- **功能简介**：逐一勾选/取消对象捕捉模式；顶部 Select All / Clear All；底部含等轴测与 F5 说明。
- **UI 入口**：`状态栏 → 对象捕捉 ▾ → 模式列表`。
- **样式**：每行 = 勾选标记 + 捕捉 SVG 图标（`themed_success`，13px）+ 标签（尺寸 11）；`button::subtle`，内边距 `[3, 8]`；头部 Select All/Clear All 为 `button::secondary`（尺寸 10）。
- **触发命令**：`Message::ToggleSnap(SnapType)`、`SnapSelectAll`、`SnapClearAll`。
- **实现位置**：`src/ui/popup/snap_popup.rs::menu_entries`、`snap_row`；模式清单 `src/snap.rs::ALL_SNAP_MODES`。
- **全部二维模式**（`ALL_SNAP_MODES`，标签原文 + 中文）：
  - `Endpoint`（端点 `◻`）
  - `Midpoint`（中点 `△`）
  - `Center`（圆心 `◯`）
  - `Node`（节点 `◆`）
  - `Quadrant`（象限点 `◇`）
  - `Intersection`（交点 `✕`）
  - `Extension`（延伸 `—`）
  - `Insertion`（插入点 `⊾`）
  - `Perpendicular`（垂足 `⊥`）
  - `Tangent`（切点 `⌒`）
  - `Nearest`（最近点 `✧`）
  - `Apparent Intersection`（外观交点 `✗`，X 外套方框）
  - `Parallel`（平行 `∥`）

### 三维对象捕捉（F4 + 3D 模式）
- **功能简介**：独立于二维捕捉的实体 B-rep 特征捕捉系统，F4 主开关，模式在草图设置"3D Object Snap"选项卡配置。
- **UI 入口**：`草图设置 → 3D Object Snap 选项卡`；`F4`。
- **样式**：复选框行（尺寸 14 复选框 + 标签尺寸 11），与二维相同。
- **触发命令**：`TOGGLE3DOSNAP`（F4）。
- **实现位置**：`src/snap.rs::{SnapType::is_3d, ALL_3D_SNAP_MODES, Snapper::snap3d_enabled/enabled3d}`；`src/ui/window/drafting_settings.rs::object_snap_3d_view`。
- **全部三维模式**（`ALL_3D_SNAP_MODES`）：
  - `Vertex`（实体角点 `◈`）
  - `Midpoint on edge`（边中点 `▽`）
  - `Center of face`（面中心 `◉`）
  - `Knot`（样条节点 `⬥`）
  - `Perpendicular to face`（到面垂足 `⟂`）
  - `Nearest to face`（面上最近点 `✦`，默认关闭）

### 捕捉标记绘制（各模式图形）
- **功能简介**：捕捉命中时在光标处以不同几何图形显示中/端点等类型；延伸/外观交点/交点还绘制虚线引导线。
- **UI 入口**：`视口光标处`。
- **样式**：线条用主题 marker 色；交点/外观交点的虚线 `LineDash [4,4]`；延伸虚线 `[4,4]` 且带三点串；平行参考在小 ∥ 字形上。
- **实现位置**：`src/ui/overlay.rs::SelectionCanvas::draw`（捕捉分支）、`SnapType::{Intersection,ApparentIntersection,Insertion,Perpendicular,Tangent,Nearest,Extension,Quadrant}` 绘制；标记数据 `src/snap.rs::SnapResult`。

### 捕捉孔径与优先级
- **功能简介**：捕捉判定半径（`osnap_radius_px`，默认 15px）内，按"优先级优先、距离次之"选取；连续型捕捉（Nearest/Perp）不会遮蔽离散型（Endpoint/Midpoint/Center）。
- **UI 入口**：`视口光标`。
- **实现位置**：`src/snap.rs::Snapper::snap`（`DEFAULT_OSNAP_RADIUS_PX=15.0`，`best_rank/best_sub/best_d2`）。
- **备注**：候选投影超出 pane 矩形者被拒（防止捕捉到被裁剪几何）。

### 捕捉替代（Snap Overrides，Shift+右键）
- **功能简介**：为"下一次拾取"临时指定唯一捕捉模式（或 None/MTP），用后自动还原；即使运行中捕捉关闭也生效。
- **UI 入口**：`视口 → Shift+右键 → 图标网格`；命令运行中右键菜单 `Snap Overrides ▸`。
- **样式**：4 列图标网格（单元格 26×26，间距 2），仅图标、悬停显示名称 tooltip；面板背景 `background.weak`、边框 `background.neutral`、圆角 4；点击外部/右键关闭。
- **触发命令**：`Message::SnapOverridePick(SnapType)`、`SnapOverrideMtp`、`SnapOverrideNone`；`src/snap.rs::Snapper::{set_override,set_override_none,clear_override}`。
- **实现位置**：`src/app/view/overlay.rs::snap_override_overlay`；菜单项 `src/ui/popup/context_menu.rs::snap_override_items`。
- **备注**：右键菜单对应项"M2P、Endpoint、Midpoint、Intersection、Apparent Intersection、Extension、Center、Quadrant、Tangent、Perpendicular、Parallel、Node、Insertion、Nearest、None、Osnap Settings..."；Esc 丢弃未消费的替代并关闭菜单（#337）。

### Osnap 设置入口
- **功能简介**：从替代菜单打开图形设置对话框。
- **UI 入口**：`Shift+右键 → Osnap Settings...`。
- **触发命令**：`OSNAP`。

---

## 五、对象追踪 / 极轴 / 正交 / 栅格 / 等轴测

### 栅格显示（GRID）
- **功能简介**：在活动 UCS 平面上绘制自适应网格，可含主网格线，可限制在图纸范围内。
- **UI 入口**：`状态栏 / F7`；设置见草图设置"Snap and Grid"。
- **样式**：网格不透明度由 `GridStyle.opacity`（默认 18）；主轴 `major_every`（<2 禁用）；网格缓存键 `GridKey`。
- **触发命令**：`GRID`（F7）。
- **实现位置**：`src/ui/overlay.rs::{grid_overlay,GridCanvas,draw_grid,compute_grid_steps,GridParams,GridStyle}`；宿主 `src/app/drafting_settings.rs`。
- **备注**：`grid_adaptive` 按缩放以 5 的幂放大步长（仅增长，不低于用户基数）；`grid_beyond_limits` 控制是否超出 LIMITS 绘制。

### 栅格捕捉（SNAP）
- **功能简介**：把每个点吸附到栅格角；独立于对象捕捉总开关，采用锁定语义（不受孔径门控）。
- **UI 入口**：`状态栏 / F9`；草图设置 Snap X/Y 间距与"Snap On (F9)"。
- **样式**：开关 pill。
- **触发命令**：`SNAP`（F9）。
- **实现位置**：`src/snap.rs::Snapper::{grid_snap,grid_snap_on,toggle_grid_snap}`、`snap`（Grid 分支）；`src/ui/window/drafting_settings.rs::snap_and_grid_view`。
- **备注**：默认间距 X=Y=10；对象捕捉永不吃到栅格点。

### 正交模式（ORTHO）
- **功能简介**：把光标移动约束到相对上一点的 0°/90° 轴。
- **UI 入口**：`状态栏 Ortho pill / F8`；草图设置 Polar Tracking 选项卡。
- **样式**：`toggle_pill(ST_ORTHO)`，激活背景 `primary.weak`；tooltip "Orthogonal Mode\nF8"。
- **触发命令**：`ORTHO`（F8）、`Message::ToggleOrtho`。
- **实现位置**：`src/ui/statusbar/mod.rs`（Ortho pill）；`src/snap.rs::otrack_snap`（ortho 硬锁语义）；`src/app/drafting_settings.rs`。

### 极轴追踪（POLAR）
- **功能简介**：按预设增量角（可自定义）引导光标移动；主按钮左键开关，右键循环增量，▾ 打开角度选择器。
- **UI 入口**：`状态栏 Polar pill（图标+当前角度）/ F10`。
- **样式**：`split_pill`；显示角度文本；下拉 `polar_popup` 列表含勾选标记与自定义输入框（"Custom…" + `°`）。
- **触发命令**：`POLAR`（F10）、`TogglePolar`、`SetPolarAngle`、`PolarCustomInput`、`SubmitPolarCustom`。
- **实现位置**：`src/ui/statusbar/mod.rs::polar_pill`；`src/ui/popup/polar_popup.rs::{menu_entries,PRESETS,angle_label}`。
- **备注**：预设角度 `[90,45,30,22.5,18,15,10,5,1]°`（#264）；tooltip 说明"F10 — left-click on/off\nRight-click cycles · ▾ picks angle"。

### 对象捕捉追踪（OTRACK）
- **功能简介**：在已捕捉点上停留后获得临时追踪点，光标可对齐到通过这些点的水平/垂直/边延伸/垂直/与基点连线方向，并支持两向量交点锁定。
- **UI 入口**：`状态栏 Otrack pill / F11`。
- **样式**：对齐线主题主色半透明（`primary.base.scale_alpha(0.7)`），虚线 `[6,4]`，穿过获取点延伸很长（L=5000）再裁剪；交点锁绘制两条交叉向量。
- **触发命令**：`OTRACK`（F11）。
- **实现位置**：`src/snap.rs::{Snapper::otrack_snap,tracking_active,update_otrack_dwell,clear_tracking,OtrackHit}`；绘制 `src/ui/overlay.rs::SelectionCanvas`（`otrack_lines`、`ost_points`）；`src/ui/statusbar/mod.rs`。
- **备注**：停留阈值 `DWELL_MS=250`、`DWELL_PX=8`、`MIN_OBSERVATIONS=3`，最多 4 个追踪点（超限淘汰最旧）；正交开启时对齐成硬锁（#218）；交点锁定显示两向量（#1313）。

### 平行捕捉（Parallel）
- **功能简介**：悬停一条线/多段线一段时间获取平行参考，随后从基点画出的方向与之平行时锁定；再次悬停同线可取消。
- **UI 入口**：`对象捕捉模式列表 → Parallel`。
- **样式**：参考线位置绘制 ∥ 字形（主题主色，线宽 1.5）；对齐虚线与追踪同色。
- **实现位置**：`src/snap.rs::{Snapper::update_parallel,parallel_snap,parallel_ref}`；绘制 `src/ui/overlay.rs::parallel_ref_marker`。
- **备注**：`PAR_DWELL_MS=150`；忽略曲线（平行于曲线无定义）；独立于 OTRACK（#277）。

### 等轴测草图与平面切换
- **功能简介**：开启等轴测草图后，栅格/捕捉吸附到等轴测平面；F5 轮流切换 Left/Top/Right 平面。
- **UI 入口**：`状态栏 → 捕捉 ▾ → Isometric drafting + Left/Top/Right 按钮`；`草图设置 → Snap and Grid → Snap type`。
- **样式**：三个平面按钮，当前平面用 `button::primary`，其余 `button::secondary`；说明文字 "F5 cycles Left, Top, and Right." / "F5 cycles the active plane."。
- **触发命令**：`ISOPLANE`（F5）、`SetIsoPlane`、`ToggleIsometricDrafting`。
- **实现位置**：`src/ui/popup/snap_popup.rs`；`src/ui/window/drafting_settings.rs`；`src/app/drafting_settings.rs::iso_plane/isometric_drafting`。
- **备注**：等轴测对为斜交，栅格捕捉用 2×2 Gram 逆矩阵回算坐标（`src/snap.rs::snap`）。

### 捕捉旋转角
- **功能简介**：显示并可重置捕捉/栅格整体旋转角。
- **UI 入口**：`草图设置 → Snap and Grid → Snap type → Rotation: {angle}° + Reset rotation`。
- **触发命令**：`Message::DraftingSettingsResetRotation`。
- **实现位置**：`src/ui/window/drafting_settings.rs::snap_and_grid_view`。

---

## 六、草图设置对话框（DSETTINGS / OSNAP）

### 对话框整体
- **功能简介**：集中配置捕捉/栅格、极轴、对象捕捉、3D 捕捉、动态输入、快速特性、选择循环。
- **UI 入口**：`状态栏各 pill / F 键 → 草图设置`；命令 `DSETTINGS`。
- **样式**：单行不复制的选项卡条（尺寸 11、选中 `primary.strong`，底部分隔线），内容 `group`（尺寸 11 标题 + 浅边框），底部 OK / Apply（脏时可用，`button::secondary`；否则 `button::text`）/ Close；关闭有未保存确认 `discard_guard`。
- **实现位置**：`src/ui/window/drafting_settings.rs::{view_window,DraftingSettingsState,DraftingSettingsTab}`；`src/app/drafting_settings.rs`。
- **选项卡**：`Snap and Grid`、`Polar Tracking`、`Object Snap`、`3D Object Snap`、`Dynamic Input`、`Quick Properties`、`Selection Cycling`。

### Snap and Grid 选项卡
- **功能简介**：开关捕捉/栅格，设置 X/Y 间距、是否等间距、主网格间隔、自适应、超出范围显示、等轴测、旋转角。
- **UI 入口**：`草图设置 → Snap and Grid`。
- **控件与标签**：`Snap On (F9)`；`Snap spacing`（`Snap X spacing:`、`Snap Y spacing:` 输入默认 10，`Equal X and Y spacing`）；`Snap type`（`Enable isometric drafting`、Left/Top/Right 按钮、`F5 cycles Left, Top, and Right.`、`Rotation: %{angle}°` + `Reset rotation`）；`Grid On (F7)`；`Grid spacing`（`Grid X spacing:`、`Grid Y spacing:`、`Major line every:` 默认 5）；`Grid behavior`（`Adaptive grid`、`Display grid beyond Limits`）。
- **实现位置**：`src/ui/window/drafting_settings.rs::snap_and_grid_view`。
- **备注**：间距须为正有限数（≤1e9）；主网格间隔须为 2–100 整数；错误提示见 `apply_drafting_settings`。

### Polar Tracking 选项卡
- **功能简介**：开关极轴与正交，显示极轴增量角。
- **控件与标签**：`Polar Tracking On (F10)`、`Ortho mode (F8)`、`Polar Angle Settings`（`Increment angle:` + `{:.1}°` 只读显示）、说明文字 "Polar Tracking guides cursor movement along specified angles. Ortho mode constrains movement to orthogonal axes."。
- **实现位置**：`src/ui/window/drafting_settings.rs::polar_tracking_view`。

### Object Snap 选项卡
- **功能简介**：开关对象捕捉与追踪，勾选全部二维模式，Select All / Clear All。
- **控件与标签**：`Object Snap On (F3)`、`Object Snap Tracking On (F11)`、`Select All`、`Clear All`、`Object Snap modes`（两列复选框）。模式来自 `ALL_SNAP_MODES`。
- **实现位置**：`src/ui/window/drafting_settings.rs::object_snap_view`。

### 3D Object Snap 选项卡
- **控件与标签**：`3D Object Snap On (F4)`、`3D Object Snap modes`（单列复选框）。模式来自 `ALL_3D_SNAP_MODES`。
- **实现位置**：`src/ui/window/drafting_settings.rs::object_snap_3d_view`。

### Dynamic Input 选项卡
- **功能简介**：开关指针输入并展示相关说明复选项。
- **控件与标签**：`Enable Pointer Input (F12)`；`Pointer Input`（`Display coordinate input near crosshairs`、`Enable dimension input fields`，恒勾选）；`Dynamic Prompts`（`Show command prompting and command input near crosshairs`，恒勾选）。
- **实现位置**：`src/ui/window/drafting_settings.rs::dynamic_input_view`。

### Quick Properties 选项卡
- **控件与标签**：`Display Quick Properties palette on selection`；`Palette Location`（`Cursor-dependent position` 默认勾选、`Static quadrant location`）。
- **实现位置**：`src/ui/window/drafting_settings.rs::quick_properties_view`。

### Selection Cycling 选项卡
- **控件与标签**：`Allow selection cycling`；`Display Selection Cycling List Box`（`Show cycling badge when objects overlap`、`Display selection candidate list box on click`，恒勾选）。
- **实现位置**：`src/ui/window/drafting_settings.rs::selection_cycling_view`。

---

## 七、选择方式

### 点选（单选/累加）
- **功能简介**：单击拾取鼠标下对象；连续点击累加选择；Shift+点击移除；`PICKADD 0` 时普通点击替换、Shift+点击切换。
- **UI 入口**：`视口 → 左键点击对象`。
- **样式**：命中对象以高亮色显示（`highlight_color`）；锁定图层对象旁绘制小锁徽标（`hover_locked`）。
- **实现位置**：`src/app/update/viewport.rs`（点击命中与累加分支）；`src/scene/pick/selection_state.rs::SelectionState`；`src/scene/pick/hit_test.rs::click_hit`；锁定徽标 `src/ui/overlay.rs`。
- **备注**：选择受选择过滤器约束（`passes_selection_filter`）。

### 窗选 / 交叉选（矩形框）
- **功能简介**：空白处按下并拖出矩形框——从左往右为窗选（蓝，全包含），从右往左为交叉（绿，接触即选）；框的语义也可由 Window/Crossing 关键字固定。
- **UI 入口**：`视口 → 空白处按住左键拖动`。
- **样式**：交叉色 `DEFAULT_CROSSING_COLOR`（emerald `#33B870`）、窗选色 `DEFAULT_WINDOW_COLOR`（cobalt `#3370B8`）；按主题/画布明暗取更适配的成对颜色（`theme_selection_colors` / `light_canvas_color`）；填充透明度 `selection_fill_alpha`（用户不透明度/100，浅色画布 ×1.35 上限 0.45）。
- **实现位置**：`src/app/update/viewport.rs`（box 分支 `box_crossing`/`box_crossing_locked`）；`src/scene/pick/selection_state.rs`；颜色 `src/ui/overlay.rs::{resolve_selection_base_color,selection_fill_alpha}`。
- **备注**：`PICKADD 0` 与 Shift 语义同上。

### 套索（多边形选择 / lasso）
- **功能简介**：拖动时按 Lasso 手势形成自由多边形选择路径，闭合后按交叉/窗选判定。
- **UI 入口**：`视口 → 空白处拖动（极简拖动）`。
- **样式**：`poly_active`/`poly_points` 绘制多边形轮廓，交叉/窗选色同上。
- **实现位置**：`src/app/update/viewport.rs`（`poly_active`、`poly_crossing`）、`src/scene/pick/selection_state.rs::{poly_active,poly_points,poly_crossing}`。
- **备注**：Shift+套索移除；空套索不改变选择。

### 围栏（Fence）
- **功能简介**：逐点拾取一条开放折线，选择它切割到的所有对象；预览为青色虚线橡皮筋。
- **UI 入口**：`命令 "Select objects:" → 选项按钮 Fence 或键入 F`；TRIM/EXTEND 的 Fence 手势。
- **样式**：虚线 `DASH_LENGTH=0.8`、`DASH_PATTERN=[0.5,-0.3,...]`，青色 `WireModel::CYAN`。
- **触发命令**：选择关键字 `F`（`FENCE`）。
- **实现位置**：`src/modules/draw/fence.rs::{FencePick::fence,preview,dashed}`；`src/modules/draw/select.rs::on_text_input`。
- **备注**：至少 2 点可用；提示 "Fence: pick points (N placed, Enter to apply):"。

### 窗口多边形（WPolygon）
- **功能简介**：逐点拾取闭合多边形，仅选择完全位于其中的对象。
- **UI 入口**：`"Select objects:" → WPolygon 按钮 / 键入 WP`。
- **样式**：闭合虚线青色预览；关闭后按窗选语义。
- **触发命令**：`WP`（`WPOLYGON`）。
- **实现位置**：`src/modules/draw/fence.rs::{FencePick::polygon}`；`src/modules/draw/select.rs::on_text_input`（`pick_crossing=false`）。
- **备注**：至少 3 点；提示 "Window polygon: pick points (N placed, Enter to apply):"。

### 交叉多边形（CPolygon）
- **功能简介**：逐点拾取闭合多边形，选择位于其中或与其相交的对象。
- **UI 入口**：`"Select objects:" → CPolygon 按钮 / 键入 CP`。
- **样式**：闭合虚线青色预览；按交叉语义。
- **触发命令**：`CP`（`CPOLYGON`）。
- **实现位置**：`src/modules/draw/select.rs::on_text_input`（`pick_crossing=true`）、`on_preview_wires`。
- **备注**：至少 3 点；提示 "Crossing polygon: pick points (N placed, Enter to apply):"。

### 选择关键字集合（Select objects 提示与按钮）
- **功能简介**：修改命令未预选时进入 "Select objects:" 收集阶段，提供 Window/Crossing/Fence/WPolygon/CPolygon/All/Add/Remove/Previous/Last 按钮与关键字。
- **UI 入口**：`命令行提示按钮 / 右键菜单`。
- **样式**：选项按钮见"命令选项按钮"；提示随已选数量变化——"`<CMD>  Select objects (%N selected, Enter to apply):`"。
- **触发命令**：`W`、`C`、`F`、`WP`、`CP`、`ALL`、`A`、`R`、`P`、`L`（自动约束命令另有 `S`=Settings）。
- **实现位置**：`src/modules/draw/select.rs::{SelectObjectsCommand::options,prompt,on_text_input,on_enter}`、变体 `new/plain/routed/instant/with_prompt/associative_dimensions/auto_constrain`。
- **备注**：Enter/右键提交；无选择取消；Fence/WPolygon/CPolygon 在命令内累积点，其余关键字集中消费（`try_selection_keyword`，#426/#596）；`instant` 首次完成选择立即生效（如 LAYMCUR）；`Previous` 复用上一命令选择集（`prev_selection`）。

### 快速选择（Quick Select / QSELECT）
- **功能简介**：按对象类型 + 特性 + 运算符 + 值构建过滤器批量选择，可追加到当前选择；候选数实时显示；无效时禁用 Apply。
- **UI 入口**：`视口 → 右键 → Quick Select...`；命令 `QSELECT`（`QS`）。
- **样式**：共享可移动模态框，标题 "Quick Select"；两段 `Scope` / `Filter`；标签宽 112、尺寸 12，段落标题尺寸 11（0.65 透明）；底部 Cancel / Apply（`button::primary`）。
- **触发命令**：`QSELECT`（别名 `QS`）、`MenuAction::QuickSelect`。
- **实现位置**：`src/app/view/overlay.rs::{qselect_overlay,qselect_content}`；状态 `src/app/mod.rs::{QSelectState,QSelectOp,QSelectScope,QSelectValueEditor}`；右键 `src/ui/popup/context_menu.rs`。
- **备注**：`(Any type)` / `(Any property)` 为通配项；数值特性才出现 `>`/`<` 运算符；错误如 "No objects are available in this scope." / "Enter a valid number." / "Choose a value." / "This operator requires a numeric property."；`{} candidate object(s)` 与 `Apply to:` 范围（CurrentSpace / CurrentSelection）与 `Append` 复选框。

### 选择过滤器（Selection Filtering）
- **功能简介**：状态栏拦截，勾选哪些实体类型可被交互选择；未勾选类型从点选候选中剔除。
- **UI 入口**：`状态栏 → 选择过滤 pill → 类型列表`。
- **样式**：列表每行 = 勾选标记 + 类型名（尺寸 11），`button::subtle`；空时显示 "No objects"（0.42 透明）；头部 Select All / Clear All。
- **触发命令**：`Message::ToggleSelectionFilterType(name)`、`SelectionFilterSelectAll`、`SelectionFilterClearAll`。
- **实现位置**：`src/ui/popup/selection_filter_popup.rs::menu_entries`；状态栏 `src/ui/statusbar/mod.rs`（`StatusPill::SelFilter`）；应用 `passes_selection_filter`（`src/app/update/viewport.rs`）。
- **备注**：类型清单来自当前布局中实际存在的实体类型。

### 选择循环（Selection Cycling / 重叠对象列表）
- **功能简介**：点击落在两个及以上重叠对象上时，在光标处弹出候选列表，点击某行选择该对象；悬停行高亮其对象；点击外部取消。
- **UI 入口**：`视口 → 点击重叠对象（选择循环开启）`。
- **样式**：列表每行 = 颜色块（9×9，边框 `#6b6b6b`，圆角 2）+ 类型名（尺寸 11）+ 竖分隔（高 12）+ 图层（尺寸 10，右对齐、0.72 透明）；面板 `bordered_box`，宽度 = 类型列 + 图层列 + 42。
- **触发命令**：`SELCYCLE` / 状态栏 SelCycle pill；`Message::{CycleSelect,CycleHover,CycleHoverExit,CycleCancel}`。
- **实现位置**：`src/ui/popup/cycle_popup.rs::{cycle_popup_overlay,item_row,CycleCandidate}`；触发 `src/app/update/viewport.rs`（`selection_cycling`、`cycle_candidates`）；开关 `src/ui/window/drafting_settings.rs::selection_cycling_view`。
- **备注**：候选需通过选择过滤器；≥2 个才弹出。

### 选择预览与高亮（rollover）
- **功能简介**：光标悬停在可选对象上时以高亮色预览；静止可触发延迟 rollover 拾取。
- **UI 入口**：`视口 → 光标悬停对象`。
- **样式**：高亮色 `SelectionVisualOptions.highlight_color`（0 表示用主题）；锁定图层对象显示小锁徽标而非可编辑高亮。
- **实现位置**：`src/ui/overlay.rs::SelectionVisualOptions`、`hover_locked`；`src/app/mod.rs::{HOVER_DWELL_MS,HOVER_DWELL_DENSE_WIRES}`。

### 选择可视化设置（SelectionVisualOptions）
- **功能简介**：控制选择框/交叉底色、不透明度、高亮色、夹点大小与颜色（普通/热/悬停），可被系统变量覆盖。
- **UI 入口**：选项/系统变量（`GRIPCOLOR`/`GRIPHOT`/`GRIPHOVER`/窗口交叉色等）。
- **样式**：默认 `grip_size=5.0`、`opacity=12`；ACI>0 时按 ACI 换算 RGB，否则用主题色。
- **实现位置**：`src/ui/overlay.rs::{SelectionVisualOptions,resolve_selection_base_color,DEFAULT_CROSSING_COLOR,DEFAULT_WINDOW_COLOR}`。

---

## 八、夹点（Grips）与夹点菜单

### 夹点显示
- **功能简介**：选中对象后在其特征点（端点、中点、圆心、象限点、顶点等）显示小方块/菱形等夹点标记，可悬停、点亮、拖动编辑。
- **UI 入口**：`视口 → 选中对象后`。
- **样式**：默认半尺寸 `GRIP_HALF_PX=5.0`（`grip_size` 可调 1–25）；形状 `GripShape::{Square,Rectangle,Triangle,Circle,Dropdown,DropdownAdjacent,GizmoAxis,GizmoPlane}`；普通填充为画布底色 0.7 + 主色描边，悬停填充 `primary.strong`，热态填充 `danger`（或 ACI 覆盖色）；命中半径 `GRIP_THRESHOLD_PX=8.0`。
- **实现位置**：`src/ui/overlay.rs::{draw_grip_marker,GripMarker}`；`src/scene/pick/grip.rs::{grips_to_screen,find_hit_grip,GRIP_HALF_PX,GRIP_THRESHOLD_PX}`。
- **备注**：下拉型夹点偏移 `GRIP_DROPDOWN_OFFSET_X_PX=24`、`Y=27`；相邻下拉偏移 X=12；夹点投影用相对眼点路径避免 UTM 尺度量化误差。

### 夹点拖动编辑
- **功能简介**：拖动夹点实现拉伸、拉长、改半径、改弧长、矩形宽高/缩放、平行移动等；支持多夹点同时编辑、方向轴约束。
- **UI 入口**：`视口 → 按下夹点拖动`。
- **样式**：拖动中的夹点为热色（红）；移动带 gizmo 时显示移动操纵器。
- **实现位置**：`src/scene/pick/grip.rs::{GripEdit,GripEditMode,GripTarget}`；`src/app/update/viewport.rs`（`active_grip`、多夹点选择）；动态框见"夹点动态输入"。

### 移动操纵器（Move Gizmo）
- **功能简介**：拖动移动类夹点时显示三轴箭头（X 红、Y 绿、Z 蓝）与三对面方块，用于 3D 轴向/平面移动；悬停/拖动部分变黄。
- **UI 入口**：`视口 → 拖动移动夹点`。
- **样式**：轴长 `GIZMO_AXIS_PX=72`；轴色 `[红(0.90,0.22,0.20), 绿(0.27,0.70,0.29), 蓝(0.18,0.52,0.93)]`，激活黄 `(1.0,0.84,0.0)`；平面方块填充灰/黄，透明度 0.25/0.55。
- **实现位置**：`src/ui/overlay.rs::draw_move_gizmo`；`src/scene/pick/grip.rs::{gizmo_axis,gizmo_plane_axes,gizmo_screen_axes,GIZMO_AXIS_PX}`。

### 多夹点悬停弹出菜单（Grip Popup）
- **功能简介**：光标在支持的夹点上停留超过阈值后弹出多功能夹点菜单，可选取操作（含需要数值的项如 Lengthen/Radius/Arc Length/Rotate Text）。
- **UI 入口**：`视口 → 光标在夹点上停留`。
- **样式**：`GripPopup { items, selected, pinned }`，居中项高亮；可见性夹点（`VIS_GRIP_ID`）有独立点击下拉，无悬停菜单。
- **实现位置**：`src/app/mod.rs::GripPopup`、`GripPendingValue`；`src/app/update/viewport.rs::update_grip_hover`（`HOVER_OPEN_MS`）。
- **备注**：`pinned` 表示点击打开的菜单会停留在原夹点之外；Enter 提交当前高亮项（`GripMenuPick`）。

### 夹点右键菜单（Grip shortcut menu）
- **功能简介**：夹点编辑时右键弹出经典夹点快捷菜单。
- **UI 入口**：`视口 → 夹点热/拖动时右键`。
- **样式**：行高 `MENU_ROW_H=22`；当前模式行有勾选标记。
- **菜单项**：`Enter`（默认）、`Stretch`、`Move`、`Rotate`、`Scale`、`Mirror`、`Base Point`、`Copy`、`Undo`（仅移动过后可用）、`Exit`。
- **实现位置**：`src/ui/popup/context_menu.rs::{grip_rows,GripMenuCmd,GripMenuContext}`；动作 `src/app/update/context_menu.rs`。
- **备注**：`Stretch` 项按当前模式勾选；`Copy` 按 `copy_on` 勾选；Undo 在 `moved=false` 时禁用。

---

## 九、右键上下文菜单（非夹点）

### 命令运行中菜单
- **功能简介**：命令进行时右键菜单列出 Enter / Cancel 与该步骤关键字选项、Recent Input、Snap Overrides、Pan/Zoom（透明）。
- **UI 入口**：`视口 → 右键（命令活动时）`。
- **样式**：行高 22、分隔高 7、面板顶内边距 4；无提示时宽 `MENU_WIDTH=200`，有右栏关键字提示时 `MENU_WIDTH_WITH_HINTS=236`；默认行加粗并锚定光标下方；关键字在右栏以灰字显示并作助记键。
- **实现位置**：`src/ui/popup/context_menu.rs::{command_rows,build_context_menu,MENU_*}`；模型 `MenuContext::Command`。
- **备注**：完成项（`keyword` 为空）即 Enter 行，不重复；默认值显示为 Enter 行提示（如 OFFSET 的 `<0.5000>`）；Recent Input 最多 10 条（`RECENT_LIMIT`）。

### 空闲菜单（Idle）
- **功能简介**：无命令时右键菜单提供 Repeat（默认）、Recent Input、Clipboard、Undo/Redo、选择编辑块、Isolate、Pan/Zoom、选择工具、Properties/Options。
- **UI 入口**：`视口 → 右键（无命令）`。
- **样式**：同上；勾选状态行（Properties）显示勾；禁用行变灰。
- **菜单项（有选择时）**：`Erase`、`Move`、`Copy Selection`、`Scale`、`Rotate`、`Mirror`、`Draw Order ▸`（`Bring to Front`、`Send to Back`、`Bring Above Object`、`Send Under Object`）；`Select Similar`、`Invert Selection`、`Deselect All`；`Isolate ▸`（`Isolate Objects`、`Hide Objects`、`End Object Isolation`）；`Quick Select...`。
- **菜单项（无选择时）**：`Repeat <cmd>`、`Recent Input ▸`、`Clipboard ▸`（`Cut`、`Copy`、`Copy with Base Point`、`Paste`、`Paste as Block`、`Paste to Original Coordinates`）、`Undo [label]`（Ctrl+Z）、`Redo [label]`（Ctrl+Y）、`Isolate ▸`、`Pan`、`Zoom`、`Zoom Extents`、`Select All`、`Quick Select...`、`Properties`、`Options...`。
- **实现位置**：`src/ui/popup/context_menu.rs::{idle_rows,recent_input_submenu,navigation_rows}`。
- **备注**：Clipboard 粘贴项在剪贴板为空时禁用；Undo/Redo 无历史时禁用并按历史标签显示命令名；选中的约束字形显示最小 `Delete` 块。

### 菜单键盘导航与助记键
- **功能简介**：菜单打开后可用 ↑/↓ 高亮、Enter 选定默认/高亮行、按助记字母跳转（重复字母循环），Esc 关闭。
- **UI 入口**：`右键菜单`。
- **样式**：高亮行由 `ContextMenuUi.highlighted` 驱动（鼠标用户初始不显示高亮）。
- **实现位置**：`src/ui/popup/context_menu.rs::{selectable,default_index,find_mnemonic,default_row_y}`；`src/app/mod.rs::ContextMenuNav`；`src/scene/pick/selection_state.rs::ContextMenuUi`。
- **备注**：助记键取自关键字/标签首个 ASCII 字母数字，重复时循环（PLINE 的 CEnter/CLose 都答 `C`）。

### 右键"回车 vs 打开菜单"循环
- **功能简介**：命令运行中第一次右键等同 Enter（提交当前步骤），连续第二次右键才打开上下文菜单；任何其它交互重置该循环。
- **UI 入口**：`视口 → 右键（命令中）`。
- **实现位置**：`src/scene/pick/selection_state.rs::right_click_entered`；`src/app/update/viewport.rs`。

### 布局标签右键菜单
- **功能简介**：图纸布局标签右键提供重命名与删除。
- **UI 入口**：`状态栏 → 布局标签 → 右键`。
- **菜单项**：`Rename`、`Delete`。
- **实现位置**：`src/ui/statusbar/mod.rs::layout_tab_context_menu`。

---

## 十、鼠标、触控板与 SpaceMouse 手势

### 左键点击拾取 / 命令点输入
- **功能简介**：命令无活动时点击选择/编辑；命令活动时点击作为点输入或对象拾取。
- **UI 入口**：`视口 → 左键`。
- **实现位置**：`src/app/update/viewport.rs`（左键按下/释放分支）；`src/scene/pick/selection_state.rs::{left_down,left_press_pos,left_dragging}`。

### 中键拖动平移
- **功能简介**：按住中键拖动平移视图；中键双击（类）触发缩放适配/其它中键动作。
- **UI 入口**：`视口 → 中键拖动`。
- **触发命令**：`Message::ViewportMiddlePress`/中间拖动。
- **实现位置**：`src/app/update/viewport.rs::{on_viewport_middle_press}`、`selection_state.rs::{middle_down,middle_last_pos,middle_last_press_time}`。
- **备注**：交互式导航模式（orbit/pan/zoom-dynamic）复用中键移动路径（`orbit_mode`/`pan_mode`/`zoom_dynamic_mode`）。

### Shift+中键旋转（Orbit）
- **功能简介**：Shift+中键拖动绕枢轴旋转视图（3D orbit），枢轴在拖动开始时确定。
- **UI 入口**：`视口 → Shift+中键拖动`。
- **实现位置**：`src/app/update/viewport.rs`（orbit 分支）；`selection_state.rs::orbit_pivot`（#229）。
- **备注**：旋转枢轴取选择/模型中心并整个手势保持固定。

### 滚轮缩放
- **功能简介**：滚轮每格缩放视图；方向可反转（`zoom_wheel_reversed`）。
- **UI 入口**：`视口 → 滚轮`。
- **实现位置**：`src/app/update/viewport.rs::{scroll_intent,on_viewport_scroll}`。
- **备注**：`ScrollDelta::Lines` 始终缩放；`Pixels` 在 macOS 表触控板（→平移），其它平台按格缩放（浏览器也报告像素）。

### 双指滚动平移（触控板）
- **功能简介**：触控板双指滚动平移视图。
- **UI 入口**：`视口 → 双指滚动`。
- **实现位置**：`src/app/update/viewport.rs::scroll_intent`（macOS `Pixels` → `ScrollIntent::Pan`）。
- **备注**：平移方向与中键拖动一致。

### 捏合缩放（Pinch，macOS）
- **功能简介**：触控板捏合以 1:1 与手指同步缩放，枢轴为光标。
- **UI 入口**：`视口 → 捏合`。
- **实现位置**：`src/input/trackpad.rs::subscription`；宿主 `src/app/update/viewport.rs::pinch_zoom_steps`、`on_pinch`。
- **备注**：`pinch_zoom_steps(m) = 10*m/(1+m)`；macOS 用 AppKit 本地事件监视器读取，非 macOS 无此手势；等价于 `Camera::zoom: distance *= 1 - s/10`。

### 双击（Model 空间：编辑文本/属性/块）
- **功能简介**：Model 空间双击文本/MText 打开就地编辑器；双击表格进入单元格编辑；双击带属性块打开属性编辑器；无属性块按设置 BEDIT 或就地 REFEDIT；双击约束尺寸从命令行改参数。
- **UI 入口**：`视口 → 双击对象`（Model 空间）。
- **样式**：锁定图层对象双击不编辑，命令行提示 `Object is on locked layer "{layer}" — unlock the layer to edit it.`。
- **实现位置**：`src/app/update/viewport.rs`（双击分支，`dt<400 && d<8.0` 判定）；`begin_table_cell_edit`、`begin_text_edit`、`dynamic_dimension_value_command`。
- **备注**：双击容差 400ms、8px。

### 双击（布局/视口：MSPACE/PSPACE）
- **功能简介**：图纸布局中双击视口进入 MSPACE；MSPACE 中双击空白/外部退出到 PSPACE。
- **UI 入口**：`图纸布局 → 双击视口`。
- **触发命令**：`MSPACE`（别名 `MS`）、`PSPACE`（`PS`）、`Message::MspaceCommand`/`Message::ExitViewport`。
- **实现位置**：`src/app/update/viewport.rs`（布局双击分支）；`src/ui/statusbar/mod.rs::space_mode_btn`。

### SpaceMouse 3D 导航（六自由度）
- **功能简介**：用 3Dconnexion SpaceMouse 平移/缩放/环绕；四种模式——Follow context / Pan only / Pan and zoom / 3D navigation。
- **UI 入口**：`状态栏 → SpaceMouse pill → 模式列表`。
- **样式**：pill = 图标 + 当前模式文本 + ▾；列表显示 `●/○` 标记当前模式、状态文本 `SpaceMouse · <状态>`，另有 "Paper sheets keep rotation locked."、"Pause/Resume SpaceMouse"、"SpaceMouse preferences…"、"3Dconnexion settings…"。
- **触发命令**：`SPACEMOUSE`、`SPACEMOUSEPAUSE`、`SPACEMOUSEPAN`、`SPACEMOUSEPANZOOM`、`SPACEMOUSEAUTO`、`SPACEMOUSE3D`、`SPACEMOUSEFIT`（Fit view）、`SPACEMOUSETOP`（Top view）。
- **实现位置**：`src/input/spacemouse/mod.rs::{NavigationMode,Preferences,Status}`；`src/ui/statusbar/spacemouse.rs::view`；`src/app/navigation.rs::{sync_spacemouse,spacemouse_label,spacemouse_pivot_overlay}`。
- **备注**：模式标签 = "Follow context"/"Pan only"/"Pan and zoom"/"3D navigation"；状态标签含 "Connecting…"、"Ready"、"No SpaceMouse connected"、"3DxWare unavailable"、"Available in the Windows desktop app"；枢轴标记为 10×10 圆环（主色）；平移速度 `pan_speed`（10–300，默认 100）与 `pan_reversed`；纸张布局旋转锁定。

### SpaceMouse 枢轴标记
- **功能简介**：导航中显示当前旋转枢轴点的小圆环标记。
- **样式**：10×10、边框 2px、圆角 5、颜色 `primary.base`。
- **实现位置**：`src/app/navigation.rs::spacemouse_pivot_overlay`。

### 视图立方体（ViewCube）
- **功能简介**：视口右上角视图立方体，用于切换标准视图与主视图。
- **UI 入口**：`视口 → 右上角 ViewCube 区域`。
- **样式**：区域 `VIEWCUBE_REGION_PX`、内边距 `VIEWCUBE_PAD`；悬停时鼠标交互设为 None。
- **实现位置**：`src/ui/overlay.rs::SelectionCanvas::mouse_interaction`（viewcube 分支）；`SPACEMOUSETOP` → `Message::ViewCubeHome`。

### PAN / Orbit / Zoom 交互模式与抓手光标
- **功能简介**：进入交互 PAN/Orbit/Zoom 模式后，视口整体成为可拖拽面，十字光标隐藏、鼠标变为抓手。
- **UI 入口**：命令 `PAN`/`ZOOM DYNAMIC`/`3DORBIT`。
- **样式**：PAN 悬停 `Grab`、拖动 `Grabbing`；`pan_mode` 下十字光标隐藏。
- **实现位置**：`src/ui/overlay.rs::SelectionCanvas::mouse_interaction`（`pan_mode` 分支）；`src/app/update/command.rs::on_command_escape`（退出模式，PAN 打印 "PAN ended."）。

### 十字光标与拾取框（Crosshair / Pickbox）
- **功能简介**：视口内隐藏系统光标、绘制 CAD 十字光标与拾取框；尺寸由 `CURSORSIZE`/`PICKBOX` 控制。
- **UI 入口**：`视口`。
- **样式**：`CROSSHAIR_SQ=7.5`、`CROSSHAIR_ARM=60`（默认 `CURSORSIZE=5`）；`PICKBOX` 默认 3 → 拾取孔径 `DEFAULT_PICK_APERTURE=8`；0 隐藏框但保留 1px 最小命中；`CursorType::Crosshair` 时 `Interaction::Hidden`。
- **实现位置**：`src/ui/overlay.rs::{crosshair_arm_px,pick_box_half_px,pick_box_aperture_px,CrosshairOptions}`。
- **备注**：客户端 `user_select`/`getpoint` 等待人工时，十字臂消失只留蓝色拾取框；下拉浮层打开时（`suppressed`）显示普通光标。

### 平移/缩放时框锚重投影
- **功能简介**：拖框选择过程中若视图缩放/平移，框锚点按世界点重投影而非冻结在像素上。
- **实现位置**：`src/scene/pick/selection_state.rs::box_anchor_world`（#234）；`src/app/navigation.rs::reproject_box_anchor`。

### 导航后悬停延迟重新武装
- **功能简介**：导航（平移/缩放/SpaceMouse）结束或开始时会清除/重新武装悬停高亮，避免即时误高亮。
- **实现位置**：`src/app/navigation.rs::{clear_navigation_hover,arm_hover_after_navigation}`。

---

## 十一、键盘输入与快捷键

### 全局快捷键清单（默认绑定）
- **功能简介**：内置全局键盘绑定表，用户可在 CUI 编辑器中修改；下表为出厂默认。
- **UI 入口**：`主窗口 → 任意处按键`；编辑见 `CUI` / `SHORTCUTS`。
- **实现位置**：`src/app/shortcuts.rs::default_bindings`。
- **完整清单（Ctrl 在 macOS 显示为 CMD）**：
  - `F1` → `HELP`
  - `F2` → `COMMANDHISTORY`
  - `F3` → `TOGGLEOSNAP`
  - `F4` → `TOGGLE3DOSNAP`
  - `F5` → `ISOPLANE`
  - `F7` → `GRID`
  - `F8` → `ORTHO`
  - `F9` → `SNAP`
  - `F10` → `POLAR`
  - `F11` → `OTRACK`
  - `F12` → `DYNINPUT`
  - `CTRL+0` → `CLEANSCREEN`
  - `CTRL+1` → `PROPERTIES`
  - `CTRL+N` → `NEW`
  - `CTRL+O` → `OPEN`
  - `CTRL+P` → `PLOT`
  - `CTRL+Q` → `QUIT`
  - `CTRL+S` → `SAVE`
  - `CTRL+SHIFT+S` → `SAVEAS`
  - `CTRL+Z` → `UNDO`
  - `CTRL+SHIFT+Z` → `REDO`
  - `CTRL+Y` → `REDO`
  - `CTRL+F` → `FIND`
  - `CTRL+H` → `FIND`
  - `CTRL+A` → `SELECTALL`
  - `CTRL+C` → `COPYCLIP`
  - `CTRL+SHIFT+C` → `COPYBASE`
  - `CTRL+X` → `CUTCLIP`
  - `CTRL+V` → `PASTECLIP`
  - `CTRL+SHIFT+V` → `PASTEBLOCK`
  - `ENTER` → `FINALIZE`
  - `SPACE` → `COMMANDSPACE`
  - `ESCAPE` → `CANCEL`
  - `DELETE` → `DELETESELECTED`
  - `BACKSPACE` → `BACKSPACE`
  - `TAB` → `DYNTAB`
  - `UP` → `HISTORYPREV`
  - `DOWN` → `HISTORYNEXT`
  - `LEFT` → `CARETLEFT`
  - `RIGHT` → `CARETRIGHT`

### 全局键 vs 焦点键判定
- **功能简介**：功能键（F1–Fn）、Escape、以及除 Ctrl+A/C/V/X 外的加速键被视为全局键，即使焦点在文本控件也拦截；剪贴板与选择命令保持文本控件原生行为。
- **实现位置**：`src/app/shortcuts.rs::{is_global_key,is_named_key}`。

### 输入动作（不进入命令注册表）
- **功能简介**：由 `run_shortcut` 直接解析的内建输入动作，不出现在命令补全注册表中，但可作快捷键命令。
- **清单**：`SPACEMOUSEFIT`、`SPACEMOUSETOP`、`FINALIZE`、`COMMANDSPACE`、`CANCEL`、`DELETESELECTED`、`BACKSPACE`、`DYNTAB`、`HISTORYPREV`、`HISTORYNEXT`、`CARETLEFT`、`CARETRIGHT`、`COMMANDHISTORY`、`TOGGLEOSNAP`、`TOGGLE3DOSNAP`、`OTRACK`、`DYNINPUT`、`SELECTALL`、`PASTECLIP`。
- **实现位置**：`src/app/shortcuts.rs::INPUT_ACTIONS`、`run_action`。

### 键名规范化
- **功能简介**：快捷键编辑器统一修饰键顺序（CTRL、CMD、ALT、SHIFT + 键名），等价输入归并为一行；`ESC`→`ESCAPE`、`RETURN`→`ENTER`、`ARROWUP`→`UP` 等别名归一。
- **实现位置**：`src/app/shortcuts.rs::normalize_key`。

### 快捷键编辑器（CUI / SHORTCUTS）
- **功能简介**：增删改快捷键；键列点击后按键捕获；命令行校验（重复键、未知命令标红）；重置默认需确认；Apply / Apply & Exit 与未保存关闭确认。
- **UI 入口**：`命令 CUI / SHORTCUTS`（`Message::` 打开快捷键窗口）。
- **样式**：标题 "Keyboard Shortcuts"（15），提示尺寸 11；列头 `Key`（宽 180）/`Command`；捕获中键列显示 "Press a key combination..."（`button::primary`），空为 "Click, then press keys..."，重复键整格红（`button::danger`）；行尾草稿有 ✓（`button::success`，仅有效时可按）/ ✕（`button::danger`）或垃圾桶；底部 `+ Add`、`Reset to default`、`Number of shortcuts: N`、Apply / Apply && Exit；冲突横幅逐条显示 "Shortcut already used for command: {key} → {command}" 与 "Unknown command: {command}"；重置确认 "Are you sure you want to reset? You will lose all your current shortcuts!" + "Yes, reset"/"No"；关闭确认 "Unsaved changes will be discarded." + "Discard && close"/"Keep editing"。
- **实现位置**：`src/ui/window/shortcuts.rs::view_window`；逻辑 `src/app/shortcuts.rs::{finish_shortcut_editor,reset_shortcuts_to_defaults,apply_shortcut_editor_rows,finish_pending_add}`。

### Esc 键分级行为
- **功能简介**：Esc 依次优先处理：客户端 getpoint/user_select、ribbon 扩展、一次性捕捉替代与菜单、MText 编辑器、TEXT 编辑器、面板移动、待定 SETVAR、UCS 图标编辑、交互导航模式（PAN 打印 "PAN ended."）、夹点菜单、可见性弹窗、活动夹点编辑、待定夹点值、活动命令取消。
- **UI 入口**：`任意处 Esc`。
- **触发命令**：`ESCAPE`（`CANCEL`）。
- **实现位置**：`src/app/update/command.rs::on_command_escape`。

### Delete 键删除选中
- **功能简介**：删除当前选择集（等同 ERASE）。
- **触发命令**：`DELETE`（`DELETESELECTED`）。
- **实现位置**：`src/app/shortcuts.rs`（`("DELETE","DELETESELECTED")`）；`Message::DeleteSelected`。

### Ctrl+Z 命令内撤销
- **功能简介**：多步命令（PLINE/SPLINE 等）中 Ctrl+Z 由命令自身消费（仅撤最后一点/一步），否则走文档撤销。
- **触发命令**：`UNDO`（Ctrl+Z）。
- **实现位置**：`src/app/shortcuts.rs::run_action`（`UNDO` 分支，`on_undo_step`）；`src/command.rs::on_undo_step`。

### Shift+Enter（自由文本换行）
- **功能简介**：自由文本步骤中 Shift+Enter 插入换行，Enter 完成编辑。
- **触发命令**：`SHIFT+ENTER`。
- **实现位置**：`src/app/update/command.rs::text_entry_mode`、`on_command_finalize`；`src/command.rs::InputKind::FreeText`。

### 文本输入三种模式
- **功能简介**：命令步骤分 Point（点/视图交互）、SingleToken（单 token：自动大写、空格提交、支持表达式如 `5*2`）、FreeText（保留大小写、空格字面、Shift+Enter 换行、Enter 结束）。
- **实现位置**：`src/command.rs::InputKind`、各命令 `input_kind()`；`src/app/update/command.rs::{text_entry_mode,is_free_text_active}`。

---

## 十二、命令别名（Aliases）

### 别名解析（键入短写执行完整命令）
- **功能简介**：命令输入首 token 经别名表展开为完整命令（如 `L`→`LINE`、`CC`→`COPYCLIP`），参数保留不变。
- **UI 入口**：`命令行键入别名`。
- **触发命令**：任意别名。
- **实现位置**：`src/app/alias.rs::{resolve_alias,parse_pgp,load_aliases,command_aliases}`；分发 `src/app/commands/mod.rs::dispatch_command_inner`。
- **备注**：大小写不敏感；别名表来源 `assets/ocad.pgp`（首次运行复制至用户配置 `<config>/ocad.pgp`，web 版存 `localStorage`）。

### 出厂默认别名清单
- **功能简介**：内置别名文件，按类别给出默认短写与目标命令。
- **UI 入口**：`命令行`。
- **实现位置**：`assets/ocad.pgp`（完整清单）。节选类别：
  - Draw：`L`→LINE、`ML`→MLINE、`PL`→PLINE、`A`→ARC、`C`→CIRCLE、`B`→BLOCK、`I`→INSERT、`DO`→DONUT、`EL`→ELLIPSE、`PO`→POINT、`SPL`→SPLINE、`H`/`BH`→HATCH、`HE`→HATCHEDIT、`HB`→HATCHTOBACK、`BO`→BOUNDARY、`GD`→GRADIENT、`XL`→XLINE、`POL`→POLYGON、`REC`→RECT、`SO`→SOLID、`WO`→WIPEOUT。
  - Modify：`M`→MOVE、`CO`/`CP`→COPY、`RO`→ROTATE、`SC`→SCALE、`MI`→MIRROR、`E`→ERASE、`X`→EXPLODE、`SS`/`S`→STRETCH、`O`→OFFSET、`TR`→TRIM、`ER`→EXTERNALREFERENCES、`EX`→EXTEND、`F`→FILLET、`CHA`→CHAMFER、`J`→JOIN、`BR`→BREAK、`BAP`→BREAKATPOINT、`PE`→PEDIT、`SPE`→SPLINEDIT、`LEN`→LENGTHEN、`AL`→ALIGN、`AR`→ARRAY、`MA`→MATCHPROP、`ATE`→ATTEDIT。
  - Clipboard：`CC`→COPYCLIP、`CX`→CUTCLIP、`PC`→PASTECLIP。
  - Blocks/refs：`ATT`→ATTDEF、`BE`→BEDIT、`IAT`→IMAGEATTACH、`XA`→XATTACH、`XR`→XREF、`ADC`→ADCENTER、`W`→WBLOCK。
  - Layers/groups：`LA`→LAYERS、`G`→GROUP、`UG`→UNGROUP。
  - Dimensions/annotation：`D`→DIMSTYLE、`DAL`→DIMALIGNED、`DAN`→DIMANGULAR、`DDI`→DIMDIAMETER、`DLI`→DIMLINEAR、`DRA`→DIMRADIUS、`DOR`→DIMORDINATE、`DBA`→DIMBASELINE、`DCO`→DIMCONTINUE、`DED`→DIMEDIT、`DBR`→DIMBREAK、`DJL`→DIMJOGLINE、`DJO`/`JOG`→DIMJOGGED、`DCE`→DIMCENTER、`LE`→LEADER、`QL`→QLEADER、`MLD`→MLEADER、`MLA`→MLEADERADD、`MLR`→MLEADERREMOVE、`MLAL`→MLEADERALIGN、`MLC`→MLEADERCOLLECT、`MLS`→MLEADERSTYLE、`MT`→MTEXT、`DT`/`T`→TEXT、`ED`→DDEDIT、`TOL`→TOLERANCE、`TB`→TABLE、`ST`→STYLE。
  - Inquiry/selection：`AA`→AREA、`DI`→DIST、`DIV`→DIVIDE、`ME`→MEASURE、`MEA`→MEASUREGEOM、`LI`→LIST、`SA`→SELECTALL、`DE`→DESELECT、`QS`→QSELECT、`FI`→FILTER。
  - View/display：`Z`→ZOOM、`P`→PAN、`U`→UNDO、`PR`/`CH`/`MO`→PROPERTIES、`UN`→UNITS、`TP`→TOOLPALETTES、`SSM`→SHEETSET、`HI`→HIDE、`R`→REDRAW、`RA`→REDRAWALL、`RE`→REGEN、`REA`→REGENALL、`3O`/`ORBIT`→3DORBIT、`MV`→MVIEW、`MS`→MSPACE、`PS`→PSPACE、`PW`→PLOTWINDOW、`QP`→QUICKPRINT、`V`→VIEW、`VSM`→VISUALSTYLES、`CUBE`→NAVVCUBE、`FSHOT`→FLATSHOT、`SHA`→SHADEMODE。
  - Settings/properties：`COL`→COLOR、`DR`→DRAWORDER、`DS`→DSETTINGS、`LT`→LINETYPE、`LTS`→LTSCALE、`OS`→OSNAP、`PU`→PURGE、`REN`→RENAME、`SET`→SETVAR、`SN`→SNAP、`TH`→THICKNESS、`UNHIDE`→UNISOLATEOBJECTS。
  - File/data：`DL`→DATALINK、`DX`→DATAEXTRACTION、`EPDF`→EXPORTPDF、`EXP`→EXPORT、`ZIP`→ETRANSMIT。
  - 3D solids：`EXT`→EXTRUDE、`REV`→REVOLVE、`INF`→INTERFERE、`CYL`→CYLINDER、`IN`→INTERSECT、`PSOLID`→POLYSOLID、`SEC`→SECTION、`SU`→SUBTRACT、`TOR`→TORUS、`UNI`→UNION、`WE`→WEDGE。

### 别名编辑器（ALIASEDIT）
- **功能简介**：增删改命令别名；两列输入（Alias / Command）；重复别名或未知命令标红并显示持久横幅；重置默认需确认；Apply / Apply & Exit 与未保存关闭确认。
- **UI 入口**：`命令 ALIASEDIT`。
- **样式**：标题 "Command Aliases"（15）；提示 "Click + Add alias, type the alias and command, accept the row, then Apply. Esc cancels the pending row."；列头 `Alias`（宽 120）/`Command`；草稿行有 ✓/✕，正常行垃圾桶；底部 `+ Add alias` / `Cancel add (Esc)`、`Reset to default`、`Number of aliases: N`、Apply / Apply && Exit；冲突横幅 "Alias already used for command: {a} → {c}" / "Unknown command: {c}"；重置确认 "Are you sure you want to reset? You will lose all your current aliases!"；关闭确认同上。
- **实现位置**：`src/ui/window/alias_editor.rs::view_window`；逻辑 `src/app/alias.rs::{apply_alias_editor_rows,finish_alias_editor,reset_aliases_to_defaults,finish_pending_alias_add}`。

### 别名默认迁移
- **功能简介**：升级时为老配置补发新增的默认别名（不覆盖用户已有映射、不复活用户删除项），当前版本 4（v2 加 R/RA/RE/REA，v3 加 HB，v4 加 ER）。
- **实现位置**：`src/app/alias.rs::{introduced_at,migrate_aliases,DEFAULT_ALIASES_VERSION}`。

---

## 十三、状态栏交互控件

### 状态栏 pills 与定制菜单
- **功能简介**：状态栏右侧由若干 pill 组成（可换行），最右定制手柄打开显示/隐藏菜单；选择持久保存。
- **UI 入口**：`状态栏右侧 → 定制手柄（MENU 图标）`。
- **样式**：行高 `ROW_HEIGHT=30`；pill 背景 `background.base`、边框 `background.neutral`；隐藏项默认包含 Coordinates、Show Lineweight、Dynamic Input、Model/Paper Space、Drawing Units、Show Transparency、Selection Cycling、Viewport Count。
- **实现位置**：`src/ui/statusbar/mod.rs::view`；`src/ui/statusbar/statusbar_config.rs::{StatusPill,StatusBarConfig}`。
- **全部 pill（`StatusPill::ALL`，标签）**：SpaceMouse、Coordinates、Ortho Mode、Show Lineweight、Polar Tracking、Dynamic Input、Object Snap Tracking、Object Snap、Model/Paper Space、Annotation Scale、Show Annotation Objects、Automatically Add Scales、Viewport / Annotation Scale Sync、Drawing Units、Show Transparency、Isolate Objects、Quick Properties、Selection Filtering、Selection Cycling、Viewport Count、Clean Screen。
- **备注**：`Model/Paper Space` pill 在 Model 显示 MODEL、PSPACE 显示 PAPER、MSPACE 显示高亮 MODEL；坐标 pill 显示 `$COORDS` 三种读数；`DOF`/冲突/GPU 软件渲染等 pill 非用户可隐藏。

### 坐标读数与 `$COORDS` 模式
- **功能简介**：状态栏坐标 pill 显示光标/上一点坐标，点击循环三种模式：静态（0，仅拾取更新）、实时绝对（1）、极坐标（2，拾取中显示 `距离 < 方向`）。
- **UI 入口**：`状态栏 → 坐标 pill（点击）`。
- **触发命令**：`Message::CycleCoordsMode`。
- **实现位置**：`src/ui/statusbar/mod.rs::{format_coords}`。
- **备注**：长度按图形 `LUNITS` 格式化（`format_length`），方向按图形零度（`format_direction`）。

### 单位 pill（LUNITS）
- **功能简介**：显示/切换线性格式，打开单位弹窗。
- **UI 入口**：`状态栏 → 单位 pill → units_popup`；命令 `UNITS`/`DWGUNITS`。
- **实现位置**：`src/ui/statusbar/mod.rs`（Units pill）；`src/ui/popup/units_popup.rs`。

### 比例 pill（注释/视口比例）
- **功能简介**：显示当前注释/视口比例名称，点击打开比例选择器；Model 恒可交互，图纸布局仅视口激活/选中时可交互。
- **UI 入口**：`状态栏 → 比例 pill → scale_popup`。
- **实现位置**：`src/ui/statusbar/mod.rs`（scale_element）；`src/ui/popup/scale_popup.rs`。

### Isolate 菜单 pill
- **功能简介**：隔离/隐藏/结束隔离。
- **UI 入口**：`状态栏 → Isolate pill → isolate_popup`。
- **菜单项**：`Isolate Objects`、`Hide Objects`、`End Object Isolation`。
- **实现位置**：`src/ui/statusbar/mod.rs`；`src/ui/popup/isolate_popup.rs`；命令 `ISOLATEOBJECTS`/`HIDEOBJECTS`/`UNISOLATEOBJECTS`。

### 布局标签与汉堡菜单
- **功能简介**：Model/布局标签切换与新建；汉堡按钮列出全部布局；标签可右键重命名/删除；图纸标签可拖动重排。
- **UI 入口**：`状态栏左侧 → 布局标签 / 汉堡菜单 / "+" 按钮`。
- **样式**：`space_tab`（尺寸 12，激活背景 `primary.weak`、边框 `primary.base`）；重命名时内联 `text_input`（宽 90）+ ✕。
- **实现位置**：`src/ui/statusbar/mod.rs::{space_tab,layout_tab_context_menu}`；`src/ui/statusbar/statusbar_menu.rs::layout_entries`。

### MCP 控制 pill
- **功能简介**：显示自动化通道状态并可开关控制；四种状态文本与配色。
- **UI 入口**：`命令行 → MCP 按钮`。
- **状态文本**：`MCP control is off`（红）、`MCP is waiting for you to pick — Enter confirms, Esc cancels`（蓝）、`MCP is handling a request`（黄）、`MCP control is ready`（绿）。
- **实现位置**：`src/ui/command_line.rs::{mcp_status,mcp_btn}`。

### 节点图按钮
- **功能简介**：命令行右侧切换节点图（Node Graph）面板。
- **UI 入口**：`命令行 → 节点图图标按钮`。
- **样式**：打开时 `button::primary`，否则 `button::subtle`；tooltip "Node graph"。
- **实现位置**：`src/ui/command_line.rs`（`graph_btn`）；`Message::Graph(GraphMsg::Toggle)`。

---

## 十四、预览、测量与命令结果反馈

### 橡皮筋/预览线
- **功能简介**：命令等待下一步时以青色绘制预览线（rubber-band），已提交段以普通色绘制。
- **实现位置**：`src/command.rs::{CmdResult::Preview,InterimWire,on_mouse_move,on_preview_wires,on_preview_wires_with_tangent}`；绘制由各命令返回 `WireModel`。

### 测量/查询结果行
- **功能简介**：命令通过 `Measurement`/`ReportMeasurement`/`ReportMeasurementAndDeselect` 等把结果打印到命令行并（按情况）结束或保持。
- **实现位置**：`src/command.rs::CmdResult::{Measurement,ReportMeasurement,ReportError,CancelWithMessage,...}`；`src/app/command_driver/mod.rs`（`push_output`/`push_error`）。

### 选择完成数量提示
- **功能简介**：选择收集阶段完成后报告找到的对象数。
- **实现位置**：`src/app/command_driver/mod.rs::push_info("{count} object(s) found.")`。

### 组创建/解散反馈
- **实现位置**：`src/app/command_driver/mod.rs::push_info("Group \"{}\" created.")`、`"{} group(s) dissolved."`、`"No groups found for selected objects."`。

### 单元格锁定反馈
- **实现位置**：`src/app/command_driver/mod.rs::push_error("The cell is content locked.")`、`"No editable table cell picked."`。

### 命令结束清选择与 Previous 记忆
- **功能简介**：命令结束（非保留选择的结果）时清空并记忆选择集，供 `Previous` 关键字复用；同时清除悬停高亮与步骤选项。
- **实现位置**：`src/app/command_driver/mod.rs::apply_cmd_result`（`prev_selection`、`deselect_all`）。

---

## 十五、命令提示与选项（命令侧原型）

本节列出 `src/command.rs` 中面向用户的提示/选项原型命令（除各绘图命令外，可在命令行直接触发）。

### VALUE 单值提示命令（PDMODE/PDSIZE/LTSCALE/CELTSCALE 等）
- **功能简介**：输入 `PDMODE` 等进入，提示一个值；空回车委托 `<name> ` 报告当前值。
- **触发命令**：`PDMODE`、`PDSIZE`、`LTSCALE`、`CELTSCALE` 等。
- **实现位置**：`src/command.rs::ValuePromptCommand`。

### RENAME 重命名命令
- **功能简介**：分步选择对象类型（按钮：Layer/Block/Text style/Dim style/Linetype/UCS/View）→ 当前名 → 新名；委托 `RENAME <type> <old> <new>`。
- **触发命令**：`RENAME`（`REN`）。
- **实现位置**：`src/command.rs::RenameCommand::{TYPES,on_text_input}`；提示 "RENAME  Select the object type to rename:"、"RENAME %{type}  Enter the current name:"、"RENAME %{type}  Rename \"%{old}\" to:"。
- **备注**：类型别名接受 `LA`/`B`/`ST`/`D`/`LT`/`V` 等；未知类型重提示。

### USERI / USERR 用户寄存器命令
- **功能简介**：先选寄存器 1–5（按钮），再输入值，委托 `USERI/USERR <n> <value>`。
- **触发命令**：`USERI`、`USERR`。
- **实现位置**：`src/command.rs::UserRegCommand`。

### Keyword 子动词命令（ATTDISP/LAYERSTATE/SCALELISTEDIT/VIEW…）
- **功能简介**：把子动词显示为按钮；动作动词直接派发，需要参数者再提示一次；`with_default` 可为回车设置默认动词。
- **实现位置**：`src/command.rs::KeywordCommand::{new,with_default}`、`match_cmd_option`（精确/唯一前缀匹配）。
- **备注**：如 PLAN 的 `<Current>` 默认动词。

### TwoValuePrompt 双参命令（LAYMRG/SETVAR…）
- **功能简介**：依次提示两个值，委托 `<name> <first> <second>`；第二个值空回车委托 `<name> <first>`（读取型）。
- **实现位置**：`src/command.rs::TwoValuePromptCommand`。

### SelectThenKeyword 选择后关键字命令（CHPROP/ADJUST/XDATA/UNDERLAY/DRAWORDER…）
- **功能简介**：无选择时先收集选择（回车确认），随后显示子动词按钮并派发 `<name> <verb> [value]`。
- **实现位置**：`src/command.rs::SelectThenKeywordCommand`。
- **备注**：收集阶段提示 "`%{name}  select objects, then press Enter:`"；有选择时用 `Relaunch` 保留选择。

### SelectThenValue 选择后单值命令（HYPERLINK/ARCTEXT/TEXTFIT/TCASE…）
- **功能简介**：先选择，再提示一个值，派发 `<name> <value>`；空回车派发 `<name>`（可选值命令如 TCOUNT/TEXTMASK）。
- **实现位置**：`src/command.rs::SelectThenValueCommand`。

### TCOUNT 重新编号命令
- **功能简介**：选择文字后依次提示起始号、增量、放置方式（Overwrite/Prefix/Suffix 按钮），派发 `TCOUNT <start> <inc> <placement>`。
- **触发命令**：`TCOUNT`。
- **样式**：放置步选项按钮 `Overwrite`(O)/`Prefix`(P)/`Suffix`(S)。
- **实现位置**：`src/command.rs::TCountCommand`。

### UCS FACE / UCS OBJECT 交互式拾取
- **功能简介**：拾取实体面或平面对象以定义 UCS；派发内联 `UCS FACE <handle> x,y,z` / `UCS OBJECT <handle>`。
- **触发命令**：`UCS`（`FACE`/`OBJECT` 关键字）。
- **实现位置**：`src/command.rs::UcsPickCommand`。

---

## 十六、SpaceMouse 命令面板（navigation.rs）

### SpaceMouse 命令注册与动作目录
- **功能简介**：把 SpaceMouse 相关命令注册进补全表，并把它们作为可绑定动作（含图标/说明），供驱动按钮映射与快捷键编辑使用。
- **触发命令**：`SPACEMOUSE`、`SPACEMOUSEPAUSE`、`SPACEMOUSEPAN`、`SPACEMOUSEPANZOOM`、`SPACEMOUSEAUTO`、`SPACEMOUSE3D`；另有 `SPACEMOUSEFIT`（Fit view）、`SPACEMOUSETOP`（Top view）。
- **实现位置**：`src/app/navigation.rs::actions`、`inventory::submit!`；动作标签 "SpaceMouse preferences"、"Pause / resume SpaceMouse"、"SpaceMouse: pan only"、"SpaceMouse: pan and zoom"、"SpaceMouse: follow context"、"SpaceMouse: 3D navigation"、"Fit view"、"Top view"。

---

## 十七、其他用户可见行为

### 节点图覆盖层的抑制光标
- **功能简介**：节点图等浮层打开时视口十字光标被抑制、显示普通光标。
- **实现位置**：`src/ui/overlay.rs::SelectionCanvas::{suppressed,mouse_interaction}`（#227）。

### 多窗格分隔条与窗格拖动
- **功能简介**：Model 平铺布局显示分隔条；可拖动手柄移动/交换窗格（源窗格变暗、目标高亮、拖动半透明幽灵卡）。
- **实现位置**：`src/ui/overlay.rs::SelectionCanvas::draw`（dividers / pane_move / pane_drop）。

### UCS 图标（含 Origin 跟随与拖动）
- **功能简介**：视口角落绘制 UCS 三轴图标；`UCSICON ORigin` 时跟随原点；悬停高亮；选中时在原点与轴端显示夹点可拖动；Esc 结束拖动并清除选择。
- **样式**：`UcsIconParams { axes, origin_screen, hover, selected }`。
- **实现位置**：`src/ui/overlay.rs::{draw_ucs_icon,UcsIconParams}`；`src/app/update/command.rs::on_command_escape`（UCS 分支）。

### 几何约束字形与悬停提示
- **功能简介**：约束以字形气泡显示在几何旁；冲突/冗余用危险色；选中加环；悬停显示类型 tooltip；同点重合约束用蓝色紧凑徽标；游标旁显示已参与约束的符号徽标。
- **实现位置**：`src/ui/overlay.rs::SelectionCanvas::draw`（`constraint_glyphs`、`constraint_glyph_tooltip`、`constraint_cursor_badge`）；`GlyphEntry`/`glyph_hit_test_entries`（`src/scene/parametric_constraints.rs`）。

### 锁定图层对象提示徽标
- **功能简介**：光标悬停到锁定图层对象时，光标旁绘制小锁徽标。
- **实现位置**：`src/ui/overlay.rs::{hover_locked,draw}`。

### 窗格/浮动视口夹点裁剪
- **功能简介**：夹点标记被裁剪到活动 3D 窗格/浮动视口矩形内，避免环绕时泄漏到图纸空间或相邻窗格。
- **实现位置**：`src/ui/overlay.rs::SelectionCanvas::grip_clip`。

### 命令空间切换取消
- **功能简介**：活动图纸空间改变时取消命令并明确报告上下文变化（不同于普通 Escape 的"用已有点完成"）。
- **实现位置**：`src/command.rs::on_space_change`、`CmdResult::CancelForSpaceChange`；`src/app/command_driver/mod.rs`。

---

（文档结束）
