# 状态栏（Status Bar）与命令行（Command Line）功能需求清单

本文档反推自 `src/ui/statusbar/`（`mod.rs`、`status_menu.rs`、`statusbar_config.rs`、`statusbar_menu.rs`、`spacemouse.rs`）、`src/ui/command_line.rs`、状态栏弹出层 `src/ui/popup/`（`snap_popup.rs`、`polar_popup.rs`、`units_popup.rs`、`scale_popup.rs`、`selection_filter_popup.rs`、`isolate_popup.rs`、`cycle_popup.rs`）、`src/app/command_driver/`（`mod.rs`、`utilities.rs`）、`src/app/drafting_settings.rs`、`src/app/config.rs`、`src/app/options_session.rs`、`src/app/alias.rs`，以及相关 `src/app/commands/`（`display.rs`、`styleprops.rs`、`mod.rs`、`fileops.rs`、`view.rs`）与 `src/app/update/`（`mod.rs`、`command.rs`、`context_menu.rs`）、`src/app/shortcuts.rs`。状态栏权威视图入口是 `src/ui/statusbar/mod.rs::StatusBar::view`，由 `src/app/view/mod.rs`（约 2232 行调用）驱动；命令行权威视图入口是 `src/ui/command_line.rs::CommandLine::view`，由 `src/app/view/mod.rs:2087` 调用。行高基准 `ROW_HEIGHT = 30.0`。

---

## 一、状态栏整体与左侧布局区

### 文档/布局标签条整体（Status Bar）
- **功能简介**：屏幕最底部的横向条，承载左侧汉堡菜单 + Model/图纸标签 + “+”新建按钮，右侧为一排可自定义的状态药丸（pill），并在窄窗口自动换行到多行。
- **UI 入口**：`主窗口 → 底部状态栏（Command Line 之上的灰色横条）`。
- **样式**：整条 `container` 背景取主题 `palette.background.base.color`，下边框 `palette.background.neutral.color`、宽 1、圆角 0；`padding([0, 4])`；内部用 `WrapBar::new(left_area, right_status).min_row_h(ROW_HEIGHT).justify_end(true)` 做左右分块换行（左侧块左对齐、右侧药丸块右对齐且 `spacing(2.0)`、`vertical_spacing(0.0)`）。每个药丸垂直居中（issue #216）。
- **触发命令**：无（始终显示）。
- **实现位置**：`src/ui/statusbar/mod.rs::StatusBar::view`、常量 `ROW_HEIGHT`；`src/app/view/mod.rs`（`self.status_bar.view(...)`）。

### 布局列表汉堡菜单（Hamburger / Model and layout list）
- **功能简介**：打开一个下拉菜单，列出 Model 与每个图纸布局，可直接切换；在标签条被滚动/窄屏时仍能选到布局。
- **UI 入口**：`状态栏 → 最左侧汉堡图标（☰）`。
- **样式**：`button(menu_icon).style(button::subtle).padding([4, 8])`，图标 16px（`themed_secondary(MENU)`；开始页为 `themed_disabled`）；用 `status_menu::menu_bar` 挂载下拉，菜单宽 200，工具提示 “Model and layout list”。开始页（无图纸）时不可点击，只显示提示 “Open or create a drawing to manage layouts.”。
- **触发命令**：`Message::LayoutSwitch(name)`（选择行时）。
- **实现位置**：`src/ui/statusbar/mod.rs::StatusBar::view`（menu_btn 构造）、`src/ui/statusbar/statusbar_menu.rs::layout_entries/layout_row`。
- **备注**：菜单每行 `Entry::close(layout_row(...))`，当前布局行加主色底高亮（`palette.primary.weak`）。

### 新建布局按钮（+）
- **功能简介**：新建一个图纸布局。
- **UI 入口**：`状态栏 → 标签条末尾的 “+” 按钮`。
- **样式**：`button(text("+").size(12)).style(button::subtle).padding([4, 8])`。
- **触发命令**：`Message::LayoutCreate`（处理器 `OpenCADStudio::on_layout_create`）。
- **实现位置**：`src/ui/statusbar/mod.rs::StatusBar::view`（add_btn）；`src/app/update/mod.rs::Message::LayoutCreate => self.on_layout_create()`。
- **备注**：开始页禁用，提示 “Open or create a drawing to add a layout.”。

### 布局标签（Model / Layout 标签）
- **功能简介**：切换模型空间与各图纸布局，并支持块编辑标签。
- **UI 入口**：`状态栏 → 汉堡菜单右侧的一排标签（Model、Layout1、…、以及打开的块编辑标签）`。
- **样式**：`space_tab`：`container(text(...).size(12)).padding([4, 10])`，活动标签底色 `palette.primary.weak.color`、主色边框 1、圆角 2、文字色 `primary.weak.text`；非活动文字 `base.text` 72% 透明；禁用 42% 透明。`space_tab` 依据 `show_layout_tabs`（LAYOUTTAB）显示。
- **触发命令**：`Message::LayoutSwitch(name)`（左键）；`Message::BlockEditSwitch(name)`（块标签）。
- **实现位置**：`src/ui/statusbar/mod.rs::space_tab`、`StatusBar::view`；标签顺序来自 `layouts`（`layouts[0]` 恒为 "Model"）。
- **备注**：布局标签受 `show_layout_tabs`（`LAYOUTTAB` 命令）控制；标签条现为 flex 换行（常量 `LAYOUT_TABS_SCROLL_ID` 仅为旧滚动消息保留）。

### 布局标签右键上下文菜单（Rename / Delete）
- **功能简介**：在图纸布局标签上右键弹出菜单，可重命名或删除该布局（Model 标签除外）。
- **UI 入口**：`状态栏 → 布局标签（图纸标签）→ 右键`。
- **样式**：`layout_tab_context_menu`：`column![rename, delete].width(160).style(container::bordered_box).padding([4, 0])`，每行 `text(..).size(12).padding([4, 12])`，指针光标。
- **触发命令/项**：
  - `Rename` — `Message::LayoutRenameStart(name)`（行内改名）。
  - `Delete` — `Message::LayoutDelete(name)`。
- **实现位置**：`src/ui/statusbar/mod.rs::layout_tab_context_menu`；由 `space_tab` 用 `iced_aw::ContextMenu` 包裹（仅可重排的图纸标签有该菜单）；处理 `src/app/update/mod.rs::Message::LayoutDelete/LayoutRenameStart`。
- **备注**：删除当前布局时会先取消活动命令并切回 Model（`LayoutDelete` 分支）；删除成功输出 “Layout "<name>" silindi”（源码原文，疑似未本地化）。

### 布局标签拖拽重排（ReorderTab）
- **功能简介**：拖动图纸标签改变其排列顺序。
- **UI 入口**：`状态栏 → 图纸标签 → 按住拖动`。
- **样式**：`crate::ui::wrap_bar::ReorderTab::layout(...)` 包装标签。
- **触发命令**：`Message::LayoutReorder { from, to, after }`（处理器入栈撤销并 `set_layout_tab_order`）。
- **实现位置**：`src/ui/statusbar/mod.rs::space_tab`（`reorderable_layouts.contains(&label)` 分支）；`src/app/update/mod.rs::Message::LayoutReorder`。

### 布局标签行内改名（Inline Rename）
- **功能简介**：标签就地变为文本输入框，输入新名并提交；带取消按钮。
- **UI 入口**：`状态栏 → 布局标签右键 → Rename`。
- **样式**：`text_input("", edit_val).id(LAYOUT_RENAME_INPUT_ID).size(12).padding([3, 6]).width(Fixed(90))` + `✕` 取消按钮（`CLOSE` 图标 10px，`button::subtle`）。
- **触发命令**：`Message::LayoutRenameEdit`（输入）、`LayoutRenameCommit`（提交/回车）、`LayoutRenameCancel`（✕）。
- **实现位置**：`src/ui/statusbar/mod.rs::space_tab`（`rename_edit` 分支）、常量 `LAYOUT_RENAME_INPUT_ID`；`src/app/update/mod.rs::Message::LayoutRenameStart/Edit/Commit/Cancel`、`on_layout_rename_commit`。
- **备注**：打开时用 `operation::focus(LAYOUT_RENAME_INPUT_ID)` 直接抓焦点（issue #86）；不可对 Model 改名。

### 布局标签禁用态（开始页）
- **功能简介**：无图纸时标签只读并给提示。
- **UI 入口**：`状态栏 → 开始/欢迎页下的标签`。
- **样式**：`space_tab` 走 `!enabled` 分支，文字 42% 透明，提示 “Open or create a drawing to switch layouts.”。
- **触发命令**：无。
- **实现位置**：`src/ui/statusbar/mod.rs::space_tab`（`!enabled` 分支）。

---

## 二、右侧状态药丸（可自定义）

药丸顺序由 `StatusBarConfig::ALL` 决定；`config.is_visible(pill)` 决定是否渲染。默认隐藏：Coords、Lwt、Dyn、Space、Units、Transparency、SelCycle、Vp（见 `statusbar_config.rs::Default`）。

### 坐标显示药丸（Coordinates）
- **功能简介**：显示当前光标坐标，可点击在 static / live absolute / polar 三种 `$COORDS` 模式间循环。
- **UI 入口**：`状态栏 → 坐标药丸`。
- **样式**：`action_pill`（`button(text.size(12)).style(button::subtle).padding([4,7])`），提示 “Cursor coordinates ($COORDS)\nClick to cycle: static / live / polar”。
- **触发命令**：`Message::CycleCoordsMode`（`$COORDS` 0→1→2→0）；显示格式 `format_coords`，使用 `entities::common::format_length`，polar 模式用 `format_direction` 输出 `距离 < 角度`。
- **实现位置**：`src/ui/statusbar/mod.rs::format_coords/StatusBar::view`；处理器 `src/app/update/mod.rs::Message::CycleCoordsMode`（输出 “COORDS = {mode} ({label})”）；`$COORDS` SETVAR 见 `src/app/commands/styleprops.rs`（约 2373 行）。
- **备注**：static（0）显示上次拾取点 `last_point`；polar（2）仅在 `picking` 且有 `last_point` 时给 `distance < angle`，否则绝对坐标；默认隐藏。

### GPU 状态药丸（Warning Pill，不可隐藏）
- **功能简介**：当场景运行在软件光栅器或无渲染器时，常驻显示降级警告，点击重新打开说明弹窗。
- **UI 入口**：`状态栏 → 警告色药丸（仅降级时出现）`。
- **样式**：`button(...).style(button::warning).padding([4, 8])`，颜色取主题 warning；提示体含适配器名 + `gpu/pill-tip`。
- **触发命令**：`Message::GpuWarningOpen`。
- **实现位置**：`src/ui/statusbar/mod.rs::gpu_pill_label/warning_pill`、`StatusBar::view`。
- **备注**：不属于 `StatusPill`，不参与自定义；`GpuStatus::Hardware/Unknown` 时不显示。

### 参数自由度药丸（DOF）
- **功能简介**：显示当前参数约束作用域剩余自由度；为 0 时变成功色。
- **UI 入口**：`状态栏 → “DOF: n” 药丸（仅存在 ParametricConstraintSet 时）`。
- **样式**：`status_pill`（`container(text.size(12)).style(container::bordered_box).padding([4,8])`）；`dof == 0` 时 `success_pill`（成功色底/边框，圆角 4）。
- **触发命令**：无（信息展示，带提示）。
- **实现位置**：`src/ui/statusbar/mod.rs::StatusBar::view`、`success_pill`。
- **备注**：作用域无 `ParametricConstraintSet` 时不渲染（`parametric_dof` 为 `None`）。

### 参数冲突药丸（⚠ n conflicting）
- **功能简介**：显示当前作用域冗余/冲突约束数量，点击移除一个并重新求解。
- **UI 入口**：`状态栏 → “⚠ {n} conflicting” 药丸（仅冲突数 > 0）`。
- **样式**：`action_pill(...label...).style(button::subtle)`。
- **触发命令**：`Message::ResolveOneParametricConflict`。
- **实现位置**：`src/ui/statusbar/mod.rs::StatusBar::view`；处理器 `src/app/update/mod.rs::Message::ResolveOneParametricConflict => self.resolve_one_parametric_conflict()`。

### 正交模式药丸（Ortho Mode）
- **功能简介**：开关正交约束（绘制沿正交方向）。
- **UI 入口**：`状态栏 → 正交按钮（图标）`。
- **样式**：`toggle_pill(ST_ORTHO, ortho_mode, Message::ToggleOrtho)`，图标 17px；关闭取主题 secondary，开启取 `themed_primary_weak_text`（主色弱色），激活时背景 `primary.weak.color`、边框 `primary.base`、`padding([4,7])`。提示 “Orthogonal Mode\nF8”。
- **触发命令**：`Message::ToggleOrtho`（快捷键 F8，命令 `ORTHO`）。
- **实现位置**：`src/ui/statusbar/mod.rs::toggle_pill/StatusBar::view`；处理器 `src/app/update/mod.rs::Message::ToggleOrtho`（开启时清 polar）；快捷键 `src/app/shortcuts.rs` F8→ORTHO，`src/app/commands/fileops.rs` `"ORTHO"`。

### 显示线宽药丸（Show Lineweight / LWT）
- **功能简介**：开关视口线宽显示。
- **UI 入口**：`状态栏 → LWT 按钮`。
- **样式**：`toggle_pill(ST_LWT, lineweight_display, Message::ToggleLineweightDisplay)`，提示 “Show Lineweight\nLWDISPLAY”。
- **触发命令**：`Message::ToggleLineweightDisplay`；命令 `LWDISPLAY [ON|OFF]`。
- **实现位置**：`src/ui/statusbar/mod.rs::StatusBar::view`；处理器 `src/app/update/mod.rs::Message::ToggleLineweightDisplay`（翻转 header `lineweight_display`，标脏，无需重剖分）；命令 `src/app/commands/styleprops.rs::"LWDISPLAY"`。
- **备注**：默认隐藏。

### 极轴追踪药丸（Polar Tracking）
- **功能简介**：左键开关极轴追踪；右键在预设增量间快速循环；右侧 ▾ 打开角度选择器。
- **UI 入口**：`状态栏 → 极轴药丸（主区）+ ▾（角度选择器）`。
- **样式**：`split_pill`（外层 `container(row![main, caret].spacing(3))`，激活底 `primary.weak.color` / 边框 `primary.base`，否则 `background.weakest` / `background.neutral`，圆角 2、`padding([4,6])`）；主区为极轴图标 + 角度文本（size 11），`padding([4,7])`。提示见下。
- **触发命令**：
  - 左键：`Message::TogglePolar`（开启时清 ortho）。
  - 右键：`Message::SetPolarAngle(next_angle)`，循环序列常量 `CYCLE = [90,45,30,22.5,18,15,10,5,1]`，非匹配时落到 45°。
  - ▾：`status_menu::menu_bar` 挂 `polar_popup::menu_entries`，宽 120。
- **实现位置**：`src/ui/statusbar/mod.rs::polar_pill/split_pill`；处理器 `src/app/update/mod.rs::Message::TogglePolar/SetPolarAngle`；快捷键 F10→POLAR（`shortcuts.rs`），命令 `POLAR`（`src/app/commands/display.rs`）。
- **备注**：提示 “Polar Tracking (%{angle})\nF10 — left-click on/off\nRight-click cycles · ▾ picks angle”。

### 动态输入药丸（Dynamic Input）
- **功能简介**：开关光标处动态输入字段（尺寸/角度框）。
- **UI 入口**：`状态栏 → DYN 按钮`。
- **样式**：`toggle_pill(ST_DYN, dyn_input, Message::ToggleDynInput)`，提示 “Dynamic Input\nF12”。
- **触发命令**：`Message::ToggleDynInput`（快捷键 F12，命令无关；短表 `shortcuts.rs` DYNINPUT）。
- **实现位置**：`src/ui/statusbar/mod.rs::StatusBar::view`；处理器 `src/app/update/mod.rs::Message::ToggleDynInput`。
- **备注**：默认隐藏。动态输入捕获键盘时命令行输入框会移除 `on_input`（`command_line.rs::view` 的 `dyn_capturing`）。

### 对象捕捉追踪药丸（Object Snap Tracking / Otrack）
- **功能简介**：开关对象捕捉追踪。
- **UI 入口**：`状态栏 → OTRACK 按钮`。
- **样式**：`toggle_pill(ST_OTRACK, otrack, Message::ToggleOTrack)`，提示 “Object Snap Tracking\nF11”。
- **触发命令**：`Message::ToggleOTrack`（F11→OTRACK，`shortcuts.rs`）。
- **实现位置**：`src/ui/statusbar/mod.rs::StatusBar::view`；处理器 `src/app/update/mod.rs::Message::ToggleOTrack`（关闭时清追踪状态）。

### 对象捕捉药丸（Object Snap / OSNAP，拆分药丸）
- **功能简介**：左键全局开关对象捕捉；右侧 ▾ 打开捕捉类型列表。
- **UI 入口**：`状态栏 → OSNAP 主区（左键）+ ▾（捕捉列表）`。
- **样式**：`split_pill`；主区 `mouse_area(snap_icon).on_press(...)`，`on = snapper.is_active() || snap_enabled`，图标 `ST_OSNAP` 17px；提示 “Object Snap: toggle on/off\nF3”；▾ 提示 “Object Snap list\nClick to choose snap types”，列表宽 210。
- **触发命令**：`Message::ToggleSnapEnabled`（F3→TOGGLEOSNAP）；弹出层项见「三、捕捉弹出层」。
- **实现位置**：`src/ui/statusbar/mod.rs::osnap_btn`；处理器 `src/app/update/mod.rs::Message::ToggleSnapEnabled`（`snapper.toggle_global()`）。

### 模型/图纸空间模式按钮（Model/Paper Space）
- **功能简介**：显示当前是模型空间还是图纸空间；在图纸中双击视口进入 MSPACE，点击可切换。
- **UI 入口**：`状态栏 → “MODEL”/“PAPER” 按钮`。
- **样式**：`space_mode_btn`：`button(text.size(12))`，`button::subtle`；激活（MSPACE 显示 MODEL）时底 `primary.weak.color`（悬停且可点时 `primary.base`）、文字 `primary.weak.text`、边框 `primary.base`。Model 标签下显示 MODEL 且不可点。
- **触发命令**：
  - 图纸 PSPACE 显示 “PAPER”，点击 → `Message::MspaceCommand`。
  - 图纸 MSPACE 显示 “MODEL”，点击 → `Message::ExitViewport`。
- **实现位置**：`src/ui/statusbar/mod.rs::space_mode_btn`；处理器 `src/app/update/mod.rs::Message::MspaceCommand/ExitViewport`、`src/app/commands/view.rs` `MspaceCommand`。
- **备注**：提示 “PAPER: double-click viewport to enter MSPACE\nMODEL: click to switch to Model Space”；默认隐藏。

### 注释/视口比例药丸（Annotation / Viewport Scale）
- **功能简介**：显示当前注释比例（模型）或视口比例（图纸），点击/▾ 打开比例选择器。
- **UI 入口**：`状态栏 → 比例药丸（文本）+ ▾`。
- **样式**：`status_menu::menu_bar` 包裹 `popup_pill(scale_label)`，面板宽：模型 150、图纸 120；不可交互时用静态 `status_pill`。标签来自 `active_scale_label`（按文件比例表匹配）或 `format_scale` 回退（如 `1:50`、`2:1`、`1:1`）。
- **触发命令**：弹出层子项 `Message::SetAnnotationScale`（模型）/`Message::SetViewportScale`（图纸）；`ScaleManagerOpen`（Manage…）。
- **实现位置**：`src/ui/statusbar/mod.rs::active_scale_label/format_scale/StatusBar::view`；`src/ui/popup/scale_popup.rs::menu_entries`；处理器 `src/app/update/mod.rs::Message::SetAnnotationScale/SetViewportScale/ScaleManagerOpen`。
- **备注**：模型空间恒可交互，图纸空间仅在有活动视口（`scale_pill_enabled`）时可交互。

### 显示注释对象药丸（Show Annotation Objects）
- **功能简介**：开关是否显示注释对象。
- **UI 入口**：`状态栏 → “注释可见” 按钮`。
- **样式**：`toggle_pill(ST_ANNO_VISIBILITY, annotation_all_visible, Message::ToggleAnnotationVisibility)`，提示 “Show Annotation Objects”。
- **触发命令**：`Message::ToggleAnnotationVisibility`；命令 `ANNOALLVISIBLE [0/1]`。
- **实现位置**：`src/ui/statusbar/mod.rs::StatusBar::view`；处理器 `src/app/update/mod.rs::Message::ToggleAnnotationVisibility`；命令 `src/app/commands/display.rs::"ANNOALLVISIBLE"`。

### 自动添加比例药丸（Automatically Add Scales）
- **功能简介**：开关在切换注释比例时自动把新比例加到现有注释对象。
- **UI 入口**：`状态栏 → “自动加比例” 按钮`。
- **样式**：`toggle_pill(ST_ANNO_AUTO_ADD, annotation_auto_add, Message::ToggleAnnotationAutoAdd)`，提示 “Automatically Add Scales”。
- **触发命令**：`Message::ToggleAnnotationAutoAdd`；命令 `ANNOAUTOSCALE [-4..4]`。
- **实现位置**：`src/ui/statusbar/mod.rs::StatusBar::view`；处理器 `src/app/update/mod.rs::Message::ToggleAnnotationAutoAdd`（0→4，否则取负）；命令 `src/app/commands/display.rs::"ANNOAUTOSCALE"`。

### 视口/注释比例同步药丸（Viewport / Annotation Scale Sync）
- **功能简介**：开关视口比例与注释比例同步。
- **UI 入口**：`状态栏 → 同步按钮（仅视口已同步状态存在时）`。
- **样式**：`toggle_pill(ST_VP_SCALE_SYNC, synced, Message::SyncViewportAnnotationScale)`，提示 “Viewport / Annotation Scale Sync”。
- **触发命令**：`Message::SyncViewportAnnotationScale`。
- **实现位置**：`src/ui/statusbar/mod.rs::StatusBar::view`；处理器 `src/app/update/mod.rs::Message::SyncViewportAnnotationScale`（`scene.sync_viewport_annotation_scale()`）。

### 单位格式药丸（Units / LUNITS）
- **功能简介**：显示并切换长度书写格式（LUNITS），并可跳转完整 UNITS 与 DWGUNITS 对话框。
- **UI 入口**：`状态栏 → 单位药丸（如 “Dec”）+ ▾`。
- **样式**：`status_menu::menu_bar` 包裹 `popup_pill(t!(units::linear_format_short(linear_format)))`，面板宽 140；提示 “Units (LUNITS)\nHow lengths are written\nClick to change”。
- **触发命令**：弹出层子项 `Message::SetLinearFormat(code)`；`Units…` → `Message::Command("UNITS")`；`Convert drawing…` → `Message::Command("DWGUNITS")`。
- **实现位置**：`src/ui/statusbar/mod.rs::StatusBar::view`；`src/ui/popup/units_popup.rs::menu_entries`；`src/modules/draw/units.rs::linear_formats/linear_format_short`。
- **备注**：默认隐藏。

### 透明度显示药丸（Show Transparency）
- **功能简介**：开关实体透明度显示（关闭时强制不透明）。
- **UI 入口**：`状态栏 → 透明度按钮`。
- **样式**：`toggle_pill(ST_TRANSPARENCY, transparency_display, Message::ToggleTransparencyDisplay)`，提示 “Show Transparency\nForce opaque when off”。
- **触发命令**：`Message::ToggleTransparencyDisplay`。
- **实现位置**：`src/ui/statusbar/mod.rs::StatusBar::view`；处理器 `src/app/update/mod.rs::Message::ToggleTransparencyDisplay`（翻转 `scene.transparency_display`，无需重剖分）。
- **备注**：默认隐藏。

### 隔离对象药丸（Isolate Objects）
- **功能简介**：打开折叠药丸，执行隔离/隐藏/结束隔离。
- **UI 入口**：`状态栏 → 隔离按钮 + ▾（弹出三项）`。
- **样式**：`status_menu::menu_bar` 包裹 `toggle_pill(ST_ISOLATE, isolation_active, Message::StatusMenuTooltipHidden(true))`，面板宽 160；提示 “Isolate Objects\nClick for Isolate / Hide / End”。
- **触发命令**：弹出层项见「七、隔离弹出层」。
- **实现位置**：`src/ui/statusbar/mod.rs::StatusBar::view`；`src/ui/popup/isolate_popup.rs::menu_entries`。

### 快速属性药丸（Quick Properties）
- **功能简介**：开关选择时出现的浮动 Quick Properties 面板。
- **UI 入口**：`状态栏 → QP 按钮`。
- **样式**：`toggle_pill(ST_QUICKPROPS, quick_properties, Message::ToggleQuickProperties)`，提示 “Quick Properties\nFloating panel on selection”。
- **触发命令**：`Message::ToggleQuickProperties`；命令 `QUICKPROPERTIES`。
- **实现位置**：`src/ui/statusbar/mod.rs::StatusBar::view`；处理器 `src/app/update/mod.rs::Message::ToggleQuickProperties`（开启时把锚点设为当前光标屏位）；命令 `src/app/commands/display.rs::"QUICKPROPERTIES"`，配置持久化 `AppConfig.quick_properties`。

### 选择过滤药丸（Selection Filtering）
- **功能简介**：打开折叠药丸，勾选哪些实体类型可被拾取。
- **UI 入口**：`状态栏 → 过滤按钮 + ▾`。
- **样式**：`status_menu::menu_bar` 包裹 `toggle_pill(ST_FILTER, selection_filter_active, ...)`，面板宽 180；提示 “Selection Filtering\nLimit which object types can be picked”。
- **触发命令**：弹出层项见「六、选择过滤弹出层」。
- **实现位置**：`src/ui/statusbar/mod.rs::StatusBar::view`；`src/ui/popup/selection_filter_popup.rs::menu_entries`。

### 选择循环药丸（Selection Cycling）
- **功能简介**：开关重复点击以在重叠对象间循环选择。
- **UI 入口**：`状态栏 → 选择循环按钮`。
- **样式**：`toggle_pill(ST_SELCYCLE, selection_cycling, Message::ToggleSelectionCycling)`，提示 “Selection Cycling\nRepeat-click to step through overlapping objects”。
- **触发命令**：`Message::ToggleSelectionCycling`。
- **实现位置**：`src/ui/statusbar/mod.rs::StatusBar::view`；处理器 `src/app/update/mod.rs::Message::ToggleSelectionCycling`（清候选与高亮）。
- **备注**：默认隐藏。循环候选列表见「八、选择循环弹出层」。

### 视口数量药丸（Viewport Count）
- **功能简介**：显示当前图纸的用户视口数量（`%{n} VP`）。
- **UI 入口**：`状态栏 → “%n VP” 药丸（仅图纸内视口数 > 0）`。
- **样式**：`status_pill(vp_label)`，提示 “Viewport count in active layout”。
- **触发命令**：无（信息）。
- **实现位置**：`src/ui/statusbar/mod.rs::StatusBar::view`。
- **备注**：默认隐藏。

### 干净屏幕药丸（Clean Screen）
- **功能简介**：隐藏 Ribbon 与面板，仅留画布与状态栏。
- **UI 入口**：`状态栏 → CLEANSCREEN 按钮`。
- **样式**：`toggle_pill(ST_CLEANSCREEN, clean_screen, Message::ToggleCleanScreen)`，提示 “Clean Screen\nHide ribbon and panels”。
- **触发命令**：`Message::ToggleCleanScreen`；命令 `CLEANSCREEN`；快捷键 `Ctrl+0`（macOS `Cmd+0`）。
- **实现位置**：`src/ui/statusbar/mod.rs::StatusBar::view`；处理器 `src/app/update/mod.rs::Message::ToggleCleanScreen`；命令 `src/app/commands/display.rs::"CLEANSCREEN"`；快捷键 `src/app/shortcuts.rs`。

### 状态栏自定义句柄（Customization ⚙/☰）
- **功能简介**：状态栏最右端的句柄，打开药丸显示/隐藏菜单。
- **UI 入口**：`状态栏 → 最右侧菜单句柄（图标）`。
- **样式**：`customize_btn()` = `button(themed_secondary(MENU,16)).style(button::subtle).padding([4,8])`；用 `status_menu::menu_bar` 挂 `statusbar_menu::customization_entries`，面板宽 200；提示 “Customization\nShow or hide status-bar items”。
- **触发命令**：`Message::ToggleStatusPill(pill)`（每行）；`Message::StatusMenuTooltipHidden(true)`（点击时隐藏工具提示）。
- **实现位置**：`src/ui/statusbar/mod.rs::customize_btn/StatusBar::view`；`src/ui/statusbar/statusbar_menu.rs::customization_entries`。
- **备注**：该句柄不可隐藏（不属于 `StatusPill`）。

### 状态栏自定义菜单项（显示/隐藏各药丸）
- **功能简介**：列出全部 21 个药丸，带勾选标记；点击切换其可见性，菜单保持打开以便连续切换，选择持久化。
- **UI 入口**：`状态栏 → 自定义句柄 → 菜单`。
- **样式**：`menu_row` = `button(row![check, text(label).size(11)].spacing(6)).style(button::subtle).width(Fill).padding([4,10])`；勾选用 `themed_check_cell(checked)`；每行 `Entry::stay`（不自动关闭）。
- **触发命令/项**（`StatusPill::ALL` 顺序与标签，全部）：
  - `SpaceMouse`（"SpaceMouse"）、`Coords`（"Coordinates"）、`Ortho`（"Ortho Mode"）、`Lwt`（"Show Lineweight"）、`Polar`（"Polar Tracking"）、`Dyn`（"Dynamic Input"）、`Otrack`（"Object Snap Tracking"）、`Osnap`（"Object Snap"）、`Space`（"Model/Paper Space"）、`Scale`（"Annotation Scale"）、`AnnoVisibility`（"Show Annotation Objects"）、`AnnoAutoAdd`（"Automatically Add Scales"）、`VpScaleSync`（"Viewport / Annotation Scale Sync"）、`Units`（"Drawing Units"）、`Transparency`（"Show Transparency"）、`Isolate`（"Isolate Objects"）、`QuickProps`（"Quick Properties"）、`SelFilter`（"Selection Filtering"）、`SelCycle`（"Selection Cycling"）、`Vp`（"Viewport Count"）、`CleanScreen`（"Clean Screen"）。
- **实现位置**：`src/ui/statusbar/statusbar_menu.rs::customization_entries`、`statusbar_config.rs::StatusPill::{ALL,id,label}`、`StatusBarConfig::{is_visible,toggle}`；处理器 `src/app/update/mod.rs::Message::ToggleStatusPill`（`statusbar_config.toggle` + `save_config`，菜单保持打开）。
- **备注**：持久化字段为 `AppConfig.statusbar`（JSON 的 "statusbar" 段，`hidden` 集合），见 `src/app/config.rs`。

---

## 三、捕捉弹出层（Snap Popup / OSNAP 下拉）

`src/ui/popup/snap_popup.rs::menu_entries(snapper, isometric, iso_plane)`。每行 `Entry::stay`（勾选后菜单不关）。

### Select All / Clear All（全选/清空捕捉）
- **功能简介**：一键开启全部 2D 对象捕捉模式，或全部关闭。
- **UI 入口**：`状态栏 → OSNAP ▾ → 顶部 “Select All” / “Clear All”`。
- **样式**：`header_btn`（`button(text.size(10)).style(button::secondary).padding([3,8])`），当无意义时禁用（`Select All` 在全开时禁用，`Clear All` 在全关时禁用）。
- **触发命令**：`Message::SnapSelectAll` / `Message::SnapClearAll`。
- **实现位置**：`src/ui/popup/snap_popup.rs::menu_entries/header_btn`；处理器 `src/app/update/mod.rs::Message::SnapSelectAll/SnapClearAll`（`snapper.enable_all/disable_all`）。

### 等轴测草图设置（Isometric drafting）
- **功能简介**：开关等轴测草图，并选择当前等轴测平面（左/上/右）。
- **UI 入口**：`状态栏 → OSNAP ▾ → “Isometric drafting” 复选行 + 三个平面按钮`。
- **样式**：`checkbox(isometric).size(14)` + `text("Isometric drafting").size(11)`；平面按钮 `button(text(plane.label()).size(10)).style(button::primary/secondary).padding([3,8])`；提示文字 “F5 cycles the active plane.”（size 10）。
- **触发命令**：`Message::ToggleIsometricDrafting`；`Message::SetIsoPlane(plane)`（`IsoPlane::ALL`）。
- **实现位置**：`src/ui/popup/snap_popup.rs::menu_entries`；处理器 `src/app/update/mod.rs::Message::ToggleIsometricDrafting/SetIsoPlane`；快捷键 F5→ISOPLANE（`shortcuts.rs`）；命令 `ISOPLANE/ISODRAFT`（`src/app/commands/display.rs`）。

### 单个捕捉模式行（全部 2D 模式）
- **功能简介**：逐项开关某类对象捕捉。
- **UI 入口**：`状态栏 → OSNAP ▾ → 各捕捉模式行`。
- **样式**：`snap_row` = `row![checkmark, icon_el, label_el]`，`checkmark` 为 `themed_check_cell(active)`，`icon_el` 为 `themed_success(icons::osnap(snap_type), 13)` 固定宽 16 居中（SVG 标记，避免 web 字体 tofu），`label_el` size 11；`button(...).style(button::subtle).width(Fill).padding([3,8])`。
- **触发命令/项**（`ALL_SNAP_MODES`，全部，顺序）：
  - `Endpoint`、`Midpoint`、`Center`、`Node`、`Quadrant`、`Intersection`、`Extension`、`Insertion`、`Perpendicular`、`Tangent`、`Nearest`、`Apparent Intersection`、`Parallel`。
- **触发命令**：`Message::ToggleSnap(SnapType)`。
- **实现位置**：`src/ui/popup/snap_popup.rs::snap_row`；`src/snap.rs::ALL_SNAP_MODES/ALL_3D_SNAP_MODES`；处理器 `src/app/update/mod.rs::Message::ToggleSnap`。
- **备注**：Grid 故意不作为对象捕捉模式（`snap.rs` 注释）；3D 捕捉（Vertex、Midpoint on edge、Center of face、Knot、Perpendicular to face、Nearest to face）属独立系统（F4 + `enabled3d`），不在本弹层，而在「制图设置」对话框的 3D 页。

---

## 四、极轴弹出层（Polar Popup）

`src/ui/popup/polar_popup.rs::menu_entries(current, custom)`。

### 角度预设行（Presets）
- **功能简介**：从常用增量中选一个作为极轴角度并启用极轴。
- **UI 入口**：`状态栏 → 极轴 ▾ → 角度列表`。
- **样式**：`angle_row` = `button(row![check, text(angle_label(deg)).size(11)].spacing(6)).style(button::subtle).width(Fill).padding([4,10])`，当前值打勾。
- **触发命令**：`Message::SetPolarAngle(deg)`。
- **实现位置**：`src/ui/popup/polar_popup.rs::PRESETS/menu_entries/angle_row`；处理器 `src/app/update/mod.rs::Message::SetPolarAngle`（设角度、开极轴、关正交）。
- **备注**：预设 `[90, 45, 30, 22.5, 18, 15, 10, 5, 1]`（`PRESETS`）；`angle_label` 去掉多余的 `.0`（如 `22.5°`、`15°`）。

### 自定义角度（Custom…）
- **功能简介**：自由输入角度值并回车应用。
- **UI 入口**：`状态栏 → 极轴 ▾ → “Custom…” 输入框`。
- **样式**：`text_input("Custom…", custom).size(11).padding([2,6]).width(Fixed(58))` + `text("°").size(11)`，行 `padding([5,10])`，`Entry::stay`。
- **触发命令**：`Message::PolarCustomInput`（输入）、`Message::SubmitPolarCustom`（回车）。
- **实现位置**：`src/ui/popup/polar_popup.rs::menu_entries`；处理器 `src/app/update/mod.rs::Message::PolarCustomInput/SubmitPolarCustom`（接受 0<v≤360，否则忽略并清空）。

---

## 五、单位弹出层（Units Popup / LUNITS）

`src/ui/popup/units_popup.rs::menu_entries(current)`。

### 长度格式行（Linear formats）
- **功能简介**：选择长度书写格式；每行附带该格式的样例，便于识别。
- **UI 入口**：`状态栏 → 单位药丸 ▾ → 格式行`。
- **样式**：`format_row` = `row![check, text(label).size(11), Space(Fill), text(sample).size(10) 55%透明].spacing(6)`，`button(...).style(button::subtle).width(Fill).padding([4,10])`；当前格式打勾。
- **触发命令/项**（`units::linear_formats()`，全部）：
  - `Architectural`（样例 `2'9 1/2"`）、`Decimal`（`33.5000`）、`Engineering`（`2'-9.5000"`）、`Fractional`（`33 1/2`）、`Scientific`（`3.3500E+01`）。
- **触发命令**：`Message::SetLinearFormat(code)`（LUNITS 码 4/2/3/5/1）。
- **实现位置**：`src/ui/popup/units_popup.rs::format_row/menu_entries`；`src/modules/draw/units.rs::LINEAR_FORMATS/linear_formats`。
- **备注**：该弹层只提供 LUNITS（如何书写长度），不提供 INSUNITS（单位标签）。

### 分隔线与 UNITS…/Convert drawing…
- **功能简介**：提供通往完整单位对话框与单位换算对话框的入口。
- **UI 入口**：`状态栏 → 单位药丸 ▾ → 分隔线下方两项`。
- **样式**：`divider()`（高 1，色 `background.weak`，左右 padding 4）；`link` = `button(text(label).size(11)).style(button::subtle).width(Fill).padding([4,10])`。
- **触发命令/项**：
  - `Units…` → `Message::Command("UNITS")`（打开绘图单位对话框）。
  - `Convert drawing…` → `Message::Command("DWGUNITS")`（打开单位换算对话框）。
- **实现位置**：`src/ui/popup/units_popup.rs::divider/link`；命令 `src/app/commands/display.rs::"UNITS"/"DDUNITS"`（`Message::OpenDrawingUnits`）、`src/app/commands/layers.rs::"DWGUNITS"`；窗口 `src/ui/window/drawing_units.rs`、`src/ui/window/layout_settling`（DWGUNITS）。
- **备注**：精度、角度、插入单位都在这两个对话框里（`units.rs` 头注）。

---

## 六、比例弹出层（Scale Popup）

`src/ui/popup/scale_popup.rs::menu_entries(is_model, current_scale_name, viewport_scale, file_scales)`。

### 比例行（Scale list）
- **功能简介**：从图纸文件（ACAD_SCALELIST）中列出的比例里选一个，设为注释比例（模型）或视口比例（图纸）。
- **UI 入口**：`状态栏 → 比例药丸 ▾ → 比例行`。
- **样式**：`scale_row` = `row![check, text(label).size(11)].spacing(6)`，`button(...).style(button::subtle).width(Fill).padding([4,10])`；当前比例打勾（图纸按名称或数值匹配）。
- **触发命令**：模型 `Message::SetAnnotationScale(label)`；图纸 `Message::SetViewportScale(label)`。
- **实现位置**：`src/ui/popup/scale_popup.rs::scale_row/menu_entries`；处理器 `src/app/update/mod.rs::Message::SetAnnotationScale/SetViewportScale`（自动加比例时调用 `add_annotation_scale_to_objects`）。
- **备注**：只显示文件中实际存在的比例，选择器不自行注入。

### 管理比例（Manage…）
- **功能简介**：打开比例管理器窗口。
- **UI 入口**：`状态栏 → 比例药丸 ▾ → “Manage…”（仅模型空间出现）`。
- **样式**：`manage_row` = `button(text("Manage…").size(11)).style(button::primary).width(Fill).padding([5,10])`。
- **触发命令**：`Message::ScaleManagerOpen`。
- **实现位置**：`src/ui/popup/scale_popup.rs::manage_row/menu_entries`；处理器 `src/app/update/mod.rs::Message::ScaleManagerOpen`（`scale_stage_begin`、`ensure_real_scale_list`）。

---

## 七、隔离弹出层（Isolate Popup）

`src/ui/popup/isolate_popup.rs::menu_entries(has_selection, isolation_active)`。

### Isolate Objects（隔离对象）
- **功能简介**：隐藏未选中对象，仅显示选择集（需有选择）。
- **UI 入口**：`状态栏 → 隔离药丸 ▾ → “Isolate Objects”`。
- **样式**：`action_row` = `button(row![text(label).size(11)]).style(button::subtle).width(Fill).padding([4,12])`；无选择时禁用（不绑 `on_press`，`Entry::stay`），可用时点击关闭菜单（`Entry::close`）。
- **触发命令**：`Message::Command("ISOLATEOBJECTS")`。
- **实现位置**：`src/ui/popup/isolate_popup.rs::menu_entries/action_row`。

### Hide Objects（隐藏对象）
- **功能简介**：隐藏选中对象（需有选择）。
- **UI 入口**：`状态栏 → 隔离药丸 ▾ → “Hide Objects”`。
- **样式**：同 `action_row`。
- **触发命令**：`Message::Command("HIDEOBJECTS")`。
- **实现位置**：`src/ui/popup/isolate_popup.rs::menu_entries`。

### End Isolation（结束隔离）
- **功能简介**：恢复被隔离/隐藏隐藏的对象（仅隔离生效时可用）。
- **UI 入口**：`状态栏 → 隔离药丸 ▾ → “End Isolation”`。
- **样式**：同 `action_row`。
- **触发命令**：`Message::Command("UNISOLATEOBJECTS")`。
- **实现位置**：`src/ui/popup/isolate_popup.rs::menu_entries`。

---

## 八、选择过滤弹出层（Selection Filter Popup）

`src/ui/popup/selection_filter_popup.rs::menu_entries(types, excluded)`。每行 `Entry::stay`。

### Select All / Clear All（全选/清空类型）
- **功能简介**：允许所有当前布局内出现的类型被选择（清空排除），或排除全部类型。
- **UI 入口**：`状态栏 → 选择过滤 ▾ → 顶部 “Select All” / “Clear All”`。
- **样式**：`header_btn`（`button(text.size(10)).style(button::secondary).padding([3,8])`），无意义时禁用（全包含时 `Select All` 禁用、全排除时 `Clear All` 禁用）。
- **触发命令**：`Message::SelectionFilterSelectAll` / `Message::SelectionFilterClearAll`。
- **实现位置**：`src/ui/popup/selection_filter_popup.rs::menu_entries/header_btn`；处理器 `src/app/update/mod.rs::Message::SelectionFilterSelectAll`（清 `selection_filter`）/`SelectionFilterClearAll`（把当前布局类型全部插入）。

### 类型行（Type rows）
- **功能简介**：逐类勾选是否可被拾取；取消勾选即排除该类型。
- **UI 入口**：`状态栏 → 选择过滤 ▾ → 类型行列表`。
- **样式**：`type_row` = `button(row![check, text(name).size(11)].spacing(6)).style(button::subtle).width(Fill).padding([4,10])`。
- **触发命令**：`Message::ToggleSelectionFilterType(name)`。
- **实现位置**：`src/ui/popup/selection_filter_popup.rs::type_row/menu_entries`；处理器 `src/app/update/mod.rs::Message::ToggleSelectionFilterType`（在 `selection_filter` 集合里增删）。
- **备注**：`types` = 当前布局内实体类型名（`scene.entity_type_names_in_layout()`）。

### 空状态行（No objects）
- **功能简介**：当前布局内无对象时的占位提示。
- **UI 入口**：`状态栏 → 选择过滤 ▾ → “No objects”`。
- **样式**：`empty_row` = `text("No objects").size(11)`，色 `background.base.text` 42% 透明，`padding([4,10])`，`Entry::stay`。
- **触发命令**：无。
- **实现位置**：`src/ui/popup/selection_filter_popup.rs::empty_row`。

---

## 九、选择循环弹出层（Selection Cycling / Cycle Popup）

`src/ui/popup/cycle_popup.rs::cycle_popup_overlay(anchor, items)`。当一个点击落在两个及以上重叠对象时，在光标处弹出。

### 循环候选列表整体
- **功能简介**：列出重叠候选对象（颜色块、类型、图层），点击某行把该对象加入当前选择；点击列表外取消。
- **UI 入口**：`视口 → 重叠对象处点击（选择循环开启时）→ 光标旁列表`。
- **样式**：`container(column(rows)).style(container::bordered_box).width(Fixed(type_w + layer_w + 42))`；用 `iced::widget::pin(opaque(panel)).position(anchor)` 定位在画布坐标；外层 `mouse_area(positioned).on_press(Message::CycleCancel)` 做取消捕获。类型列宽 `clamp_col_width(..., 11.0, 40.0, 96.0)`，图层列宽 `clamp_col_width(..., 10.0, 28.0, 84.0)`。
- **触发命令**：点击行 `Message::CycleSelect(handle)`；点击外部 `Message::CycleCancel`。
- **实现位置**：`src/ui/popup/cycle_popup.rs::cycle_popup_overlay`；处理器 `src/app/update/mod.rs::Message::CycleSelect/CycleCancel`。

### 单行候选（swatch + 类型 + 图层）
- **功能简介**：一行候选对象，悬停时高亮底层对象。
- **UI 入口**：`选择循环列表 → 每一行`。
- **样式**：`row![swatch, text(type).size(11) (定宽), rule::vertical(1) (高12), text(layer).size(10) 右对齐 72%透明 (定宽)]`，`spacing(6)`；swatch 为 9×9 颜色块（对象绘制色，边框 `#6B6B6B`、圆角 2）；`button(...).style(button::subtle).width(Fill).padding([3,8])`。
- **触发命令**：行按钮 `Message::CycleSelect(handle)`；悬停 `Message::CycleHover(Some(handle))` / `Message::CycleHoverExit(handle)`。
- **实现位置**：`src/ui/popup/cycle_popup.rs::item_row`；处理器 `src/app/update/mod.rs::Message::CycleHover/CycleHoverExit`。

---

## 十、SpaceMouse 状态指示与菜单

`src/ui/statusbar/spacemouse.rs::view(preferences, status, paused, label, sheet)`。仅在 `spacemouse.visible()` 或模式 ≠ Auto 时出现（`src/app/view/mod.rs`），并受 `StatusPill::SpaceMouse` 可见性控制。

### SpaceMouse 状态药丸（根按钮）
- **功能简介**：显示 SpaceMouse 连接状态与当前模式标签，点击展开菜单。
- **UI 入口**：`状态栏 → SpaceMouse 药丸`。
- **样式**：`button(row![icon(15px), text(label).size(11), themed_arrow_toggle(false, 8)].spacing(5)).style(button::subtle).padding([3,6])`；图标为 `window::options::spacemouse::ICON`。
- **触发命令**：菜单由 `status_menu::menu_bar` 挂载，面板宽 260。
- **实现位置**：`src/ui/statusbar/spacemouse.rs::view`；调用点 `src/app/view/mod.rs`（`spacemouse::view(...)`）。

### 连接状态行（Status）
- **功能简介**：显示 “SpaceMouse · <状态>”。
- **UI 入口**：`状态栏 → SpaceMouse 药丸 → 菜单首行`。
- **样式**：`Entry::stay`，`text("SpaceMouse · {}", status.label()).size(11)`。
- **触发命令**：无（信息）。
- **实现位置**：`src/ui/statusbar/spacemouse.rs::view`；`src/input/spacemouse/mod.rs::Status::label`（Connecting… / Ready / No SpaceMouse connected / 3DxWare unavailable / Available in the Windows desktop app）。

### 导航模式行（Navigation modes）
- **功能简介**：在当前模式前以 ●/○ 标记，选择新的导航模式。
- **UI 入口**：`状态栏 → SpaceMouse 药丸 → 模式行`。
- **样式**：`button(text("●  Follow context").size(12)).width(Fill).padding([7,10]).style(button::text)`（选中 ●，未选 ○），`Entry::close`。
- **触发命令/项**（`NavigationMode::ALL`）：
  - `Follow context`（Auto）、`Pan only`、`Pan and zoom`、`3D navigation`（Full3D）。
- **触发命令**：`Message::SpaceMouseMode(mode)`。
- **实现位置**：`src/ui/statusbar/spacemouse.rs::view`；`src/input/spacemouse/mod.rs::NavigationMode::{ALL,label}`；处理器 `src/app/update/mod.rs::Message::SpaceMouseMode`。

### 图纸旋转锁定提示（Paper sheets note）
- **功能简介**：当处于图纸（sheet）时提示旋转被锁定。
- **UI 入口**：`状态栏 → SpaceMouse 药丸 → 提示行（仅 sheet）`。
- **样式**：`Entry::stay`，`text("Paper sheets keep rotation locked.").size(11)`。
- **触发命令**：无。
- **实现位置**：`src/ui/statusbar/spacemouse.rs::view`；`spacemouse::view` 的 `sheet` 参数来自 `OpenCADStudio::spacemouse_sheet()`（`src/app/navigation.rs`）。

### 暂停/恢复（Pause / Resume SpaceMouse）
- **功能简介**：暂停或恢复 SpaceMouse 输入。
- **UI 入口**：`状态栏 → SpaceMouse 药丸 → “Pause SpaceMouse” / “Resume SpaceMouse”`。
- **样式**：`button(text(..).size(12)).on_press(..).style(button::text).width(Fill).padding([7,10])`，`Entry::close`。
- **触发命令**：`Message::SpaceMousePause`。
- **实现位置**：`src/ui/statusbar/spacemouse.rs::view`；处理器 `src/app/update/mod.rs::Message::SpaceMousePause`。

### SpaceMouse 偏好设置…（Preferences）
- **功能简介**：打开 SpaceMouse 偏好设置对话框。
- **UI 入口**：`状态栏 → SpaceMouse 药丸 → “SpaceMouse preferences…”`。
- **样式**：同菜单按钮行，`Entry::close`。
- **触发命令**：`Message::SpaceMousePreferences`（命令 `SPACEMOUSE`）。
- **实现位置**：`src/ui/statusbar/spacemouse.rs::view`；处理器 `src/app/update/mod.rs::Message::SpaceMousePreferences`；快捷键表 `src/app/shortcuts.rs` `"SPACEMOUSE"`。

### 3Dconnexion 设置…（Driver settings）
- **功能简介**：打开 3Dconnexion 厂商设置。
- **UI 入口**：`状态栏 → SpaceMouse 药丸 → “3Dconnexion settings…”`。
- **样式**：同菜单按钮行，`Entry::close`。
- **触发命令**：`Message::SpaceMouseDriverSettings`。
- **实现位置**：`src/ui/statusbar/spacemouse.rs::view`；处理器 `src/app/update/mod.rs::Message::SpaceMouseDriverSettings => self.open_spacemouse_driver_settings()`。

---

## 十一、状态栏右键菜单系统（status_menu / statusbar_menu）

### 状态栏菜单通用底座（menu_bar）
- **功能简介**：所有状态栏下拉/折叠菜单共用的 `iced_aw::MenuBar` 构造与样式。
- **UI 入口**：各药丸的 ▾/主区点击后弹出的菜单。
- **样式**：`Menu::new(items).width(Fixed(width)).padding(0).spacing(0).offset(1.0).close_on_background_click(true)`；`MenuBar` 的 `safe_bounds_margin(ROW_HEIGHT)`（允许光标越过一行仍不关闭，issue #682）、`close_on_background_click_global(true)`、`draw_path(Backdrop)`；菜单背板 `background.weakest`、边框 `background.neutral` 1px 圆角 3、阴影向下偏移、路径高亮 `primary.weak`。
- **触发命令**：每行 `Item::close_on_click(entry.close_on_click)`；行内容用 `mouse_area(...).interaction(Idle)`，使灰化选项悬停不隐藏光标（issue #684）。
- **实现位置**：`src/ui/statusbar/status_menu.rs::menu_bar/Entry::{stay,close}`。

### 工具提示显隐机制（statusbar tooltip）
- **功能简介**：菜单根控件在点击前显示工具提示，点击菜单后抑制，移出后重置；避免覆盖已展开的菜单。
- **UI 入口**：所有状态栏药丸/菜单根。
- **样式**：`tip` 用 `tooltip(...).position(Top)`，提示体 `container(...).style(container::bordered_box).padding([4,8])`，文本 size 11；`menu_tip` 用 `mouse_area(...).on_exit(Message::StatusMenuTooltipHidden(false))` 包裹。
- **触发命令**：`Message::StatusMenuTooltipHidden(bool)`。
- **实现位置**：`src/ui/statusbar/mod.rs::tip/menu_tip/tip_node`；处理器 `src/app/update/mod.rs::Message::StatusMenuTooltipHidden`。

### 布局列表菜单项（layout_entries）
- **功能简介**：汉堡菜单与状态栏布局列表共用；列出全部布局并可切换。
- **UI 入口**：`状态栏 → 汉堡菜单`。
- **样式**：`layout_row` = `button(row![text(name).size(11)]).style(button::subtle).width(Fill).padding([4,12])`，当前项在 Active 态用主色弱底高亮；`Entry::close`。
- **触发命令**：`Message::LayoutSwitch(name)`。
- **实现位置**：`src/ui/statusbar/statusbar_menu.rs::layout_entries/layout_row`。

### 状态栏旧式主菜单字段（ToggleStatusBarMenu / CloseStatusBarMenu）
- **功能简介**：状态栏菜单的显式开关状态处理（保留的历史路径）。
- **UI 入口**：`状态栏菜单`。
- **样式**：无。
- **触发命令**：`Message::ToggleStatusBarMenu` / `Message::CloseStatusBarMenu`（翻转/清除 `statusbar_menu_open`）。
- **实现位置**：`src/app/update/mod.rs::Message::ToggleStatusBarMenu/CloseStatusBarMenu`。

### 布局列表开合（ToggleLayoutList / CloseLayoutList）
- **功能简介**：布局列表的显式开关处理。
- **UI 入口**：`状态栏 → 布局菜单`。
- **样式**：无。
- **触发命令**：`Message::ToggleLayoutList` / `Message::CloseLayoutList`。
- **实现位置**：`src/app/update/mod.rs::Message::ToggleLayoutList/CloseLayoutList`。

---

## 十二、命令行整体布局（Command Line）

`src/ui/command_line.rs::CommandLine::view`。行高与状态栏/标签条对齐（输入行 `center_y(Fixed(30.0))`）。

### 命令行容器与停靠位置
- **功能简介**：屏幕底部的命令区浮层，含历史行、自动补全、下拉历史与输入行；在开始页贴底显示，绘图时叠加在画布底部。
- **UI 入口**：`主窗口 → 底部中央的命令行浮层`。
- **样式**：根 `container` 背景 `background.base`，宽 `Fill.max(720.0)`；历史下拉打开时有 `background.neutral` 1px 圆角 4 边框，否则无边框；`src/app/view/mod.rs` 用 `container(...).align(X::Center).align(Y::Bottom).padding(bottom 2)` 定位（开始页贴底、非开始页 stack 叠加）。
- **触发命令**：无。
- **实现位置**：`src/ui/command_line.rs::CommandLine::view`；`src/app/view/mod.rs`（2087-2131）。

### 命令行提示标签（Label）
- **功能简介**：输入行最左的 “命令行” 文字标签（国际化 `command-line/label`）。
- **UI 入口**：`命令行 → 输入行左侧提示`。
- **样式**：`container(text(label).size(11)).padding([5,8])`，颜色经 `accessible_accent_threshold(success.base, 背景, 文字, 4.5)` 保证对比度。
- **触发命令**：无。
- **实现位置**：`src/ui/command_line.rs::CommandLine::view`（prompt 构造）。

### 命令行输入框（Input）
- **功能简介**：输入命令名/关键字/坐标/参数；空格视为提交分隔符，命令动词自动大写。
- **UI 入口**：`命令行 → 文本输入框`（控件 id `cmd_input`）。
- **样式**：`text_input("", &self.input).id(CMD_INPUT_ID).size(11).padding({top:4,right:30,bottom:4,left:6})`；动态输入捕获时不绑定 `on_input/on_submit`（`dyn_capturing`）。
- **触发命令**：`Message::CommandInput(s)`（输入）、`Message::CommandSubmit`（回车/空格）。
- **实现位置**：`src/ui/command_line.rs::CommandLine::view/cmd_input_id`、常量 `CMD_INPUT_ID`；处理器 `src/app/update/mod.rs::Message::CommandInput/CommandSubmit`。
- **备注**：`CommandInput` 里：命令动词大写（除非自由文本或字面空格模式）；含空格且非自由文本/非字面模式时经 `CommandSubmit` 按行拆分成多步；连续空格触发提交。`submit()` 会记录历史并清空输入。

### 字面空格切换按钮（> Literal spaces）
- **功能简介**：持久切换“字面空格”模式；开启时整行如同以 `>` 开头，空格保留在输入中而不提交，便于输入含空格的参数（文本、路径、`UCS Z 90`）。
- **UI 入口**：`命令行 → 输入框左侧 “>” 按钮`。
- **样式**：`button(text(">").size(11)).padding([2,6])`，激活时底 `primary.weak`、文字 `primary.weak.text`，悬停底 `background.weak`，否则 `background.weakest`，边框 `background.neutral` 1px 圆角 3；带工具提示 `command-line/literal-spaces`（`tooltip` Top，gap 4）。
- **触发命令**：`Message::CommandLiteralToggle`（持久化到配置）。
- **实现位置**：`src/ui/command_line.rs::CommandLine::view`（literal_btn）、字段 `literal_spaces`；处理器 `src/app/update/mod.rs::Message::CommandLiteralToggle`（翻转并 `save_config`）。
- **备注**：手打的前导 `>` 只对当前行生效，也会点亮按钮（`literal_active = literal_spaces || input.starts_with('>')`）。

### 历史下拉按钮（History dropdown ▸/▾）
- **功能简介**：打开/关闭完整历史存档面板；关闭时图标为右箭头，打开时为下箭头。
- **UI 入口**：`命令行 → 输入框右侧的箭头按钮`（叠加在输入框右端，`right:30` 内边距留位）。
- **样式**：`button(dropdown_icon).style(button::text).padding([4,8])`，图标 `themed_arrow_down/right(11)`；用 `stack![input, container(dropdown_btn).align(X::Right).align(Y::Center)]` 叠加。
- **触发命令**：`Message::CommandHistoryToggle`；快捷键 F2→COMMANDHISTORY。
- **实现位置**：`src/ui/command_line.rs::CommandLine::view`；处理器 `src/app/update/mod.rs::Message::CommandHistoryToggle`（`toggle_history` + `sync_open_command_history`）。
- **备注**：`toggle_history`/`close_history`/`history_open` 见 `command_line.rs`。

### 节点图按钮（Node Graph）
- **功能简介**：打开/关闭节点图面板。
- **UI 入口**：`命令行 → 输入行右侧的节点图图标按钮`。
- **样式**：`button(themed(NODE_GRAPH, 13)).padding([2,6])`；打开时 `button::primary`，否则 `button::subtle`；工具提示 “Node graph”（Top，gap 4）。
- **触发命令**：`Message::Graph(GraphMsg::Toggle)`。
- **实现位置**：`src/ui/command_line.rs::CommandLine::view`；`src/app/view/mod.rs` 传入 `show_node_graph`。

### MCP 控制按钮（MCP）
- **功能简介**：显示自动化/MCP 通道状态并可切换启用；状态区分关闭/就绪/忙碌/等待用户拾取。
- **UI 入口**：`命令行 → 输入行右侧 “MCP” 按钮`。
- **样式**：`button(text("MCP").size(11)).padding([2,6])`，背景随状态着色（关闭红 `(0.90,0.35,0.35)`、等待蓝 `(0.30,0.55,0.98)`、忙碌黄 `(0.95,0.72,0.25)`、就绪绿 `(0.35,0.85,0.55)`），文字深色 `(0.08,0.08,0.10)`，边框取同色；工具提示（Top，gap 4），`container(...).padding(right 6)`。
- **触发命令**：`Message::ControlToggle`（翻转 `control.enabled`）。
- **实现位置**：`src/ui/command_line.rs::mcp_status/CommandLine::view`；处理器 `src/app/update/mod.rs::Message::ControlToggle`；`control_busy()` 见 `src/app/control/mod.rs`。
- **备注**：`mcp_status` 文案：“MCP control is off” / “MCP is waiting for you to pick — Enter confirms, Esc cancels” / “MCP is handling a request” / “MCP control is ready”；等待状态优先级高于忙碌。

### 输入行整体（Input Row）
- **功能简介**：由提示标签、字面空格按钮、输入框+历史下拉、节点图按钮、MCP 按钮组成的行。
- **UI 入口**：`命令行 → 输入行`。
- **样式**：`row![prompt, literal_btn, input_with_history, graph_btn, mcp_btn].spacing(4).align_y(Center)`，外层 `container(input_row)` 背景 `background.weakest`、`center_y(Fixed(30.0))`。
- **触发命令**：无。
- **实现位置**：`src/ui/command_line.rs::CommandLine::view`。

---

## 十三、命令行历史输出区

### 临时历史覆盖行（Transient overlay history）
- **功能简介**：在输入行上方显示最近的命令回显、输出、错误与信息行，超时后淡出；当前步骤提示被固定不淡出。
- **UI 入口**：`命令行 → 输入行上方叠层`。
- **样式**：`container(history_rows).background(background.base).width(Fill).padding([2,0])`；每条 `container(text(...).size(11)).padding([1,8])`；颜色见 `history_color`；错误加粗。仅显示 `CLIPROMPTLINES` 条（0 表示不显示），且处于 `COMMANDLINEFADETIME` 可见窗口内（`pinned` 行始终显示）。固定行若带选项，则把提示中的 “[A / B]” 列表剥离并用按钮替代（`strip_option_listing`）。
- **触发命令**：无（由 `push_command/push_output/push_error/push_info` 追加）。
- **实现位置**：`src/ui/command_line.rs::CommandLine::view/entry_visible/history_color`、常量 `DEFAULT_COMMANDLINE_FADE_MS/COMMANDLINE_FADE_MIN_MS/COMMANDLINE_FADE_MAX_MS`、字段 `cliprompt_lines/fade_ms/history`。
- **备注**：历史上限 `MAX_HISTORY = 64`（超出移除最旧）；`overlay_lines_height()` 供光标锚定面板避让命令行区。

### 命令回显（Command echo）
- **功能简介**：提交的命令以 “❯ 命令: <cmd>” 记入历史。
- **UI 入口**：`命令行 → 历史覆盖行 / 历史下拉`。
- **样式**：前缀常量 `COMMAND_PREFIX = "❯ "`；颜色 `background.base.text`。
- **触发命令**：`CommandLine::push_command`（由 `submit` 调用）。
- **实现位置**：`src/ui/command_line.rs::push_command/submit/history_color`（`EntryKind::Command`）。

### 普通输出（Output）
- **功能简介**：报告命令结果与提示（如坐标、布局删除、隔离等）。
- **UI 入口**：`命令行 → 历史区`。
- **样式**：文字色 `background.base.text` 72% 透明（`EntryKind::Output`）。
- **触发命令**：`CommandLine::push_output`。
- **实现位置**：`src/ui/command_line.rs::push_output/history_color`。

### 错误信息（Error）
- **功能简介**：以 “✕ 无效: <msg>” 记录错误，加粗红色显示。
- **UI 入口**：`命令行 → 历史区`。
- **样式**：前缀常量 `ERROR_PREFIX = "✕ "`，`format_error` 拼接 `Invalid` 大写；颜色 `danger.base.color`，粗体。
- **触发命令**：`CommandLine::push_error`（并更新 `error_revision`/`last_error`）；`push_error_once` 去重连续相同错误（刷新时间戳，issue #498）。
- **实现位置**：`src/ui/command_line.rs::format_error/push_error/push_error_once/history_color`。

### 警告（Warning）
- **功能简介**：记录非命令引发的会话警告（如渲染退回 CPU），外观似错误但不计入命令失败。
- **UI 入口**：`命令行 → 历史区`。
- **样式**：`*Warning*  <msg>`，`EntryKind::Error`（红色），不改 `last_error`/`error_revision`。
- **触发命令**：`CommandLine::push_warning`。
- **实现位置**：`src/ui/command_line.rs::push_warning`。

### 信息（Info）
- **功能简介**：以 “ⓘ <msg>” 记录一般提示与当前步骤提示。
- **UI 入口**：`命令行 → 历史区`。
- **样式**：前缀常量 `INFO_PREFIX = "ⓘ "`（`format_info`），颜色经 `accessible_accent_threshold(primary.base, 背景, 文字, 4.5)`。
- **触发命令**：`CommandLine::push_info`；`set_step_prompt` 会把当前步骤提示行固定（`pinned=true`）并避免重复。
- **实现位置**：`src/ui/command_line.rs::format_info/push_info/set_step_prompt/history_color`。
- **备注**：`CommandLine::new` 启动时推入两条信息行（`command-line/ready`、`command-line/hint`）。

### 当前步骤提示固定（Pinned step prompt）
- **功能简介**：当前命令步骤的提示固定在覆盖区，不随时间淡出；步骤切换或命令结束才解除固定。
- **UI 入口**：`命令行 → 输入行上方最末行`。
- **样式**：与其他信息行一致，但 `pinned` 行在可见列表末尾置顶；步骤切换时旧固定行重置 `created_at` 开始冷却。
- **触发命令**：`CommandLine::set_step_prompt(Option<String>)`（由 `handle_need_point`、`handle_preview` 等调用）。
- **实现位置**：`src/ui/command_line.rs::set_step_prompt`；`src/app/command_driver/mod.rs::handle_need_point/handle_preview/handle_interim_wire`。

### 历史下拉面板（Full backlog dropdown）
- **功能简介**：打开后显示自启动以来的完整历史，单个只读文本编辑器，可跨行拖选、Ctrl+C 复制；带 Copy / Clear 与可调高度手柄。
- **UI 入口**：`命令行 → 历史下拉按钮（▸/▾）打开 → 输入行上方面板`。
- **样式**：`text_editor(history_content).size(11).padding([2,8]).height(Shrink).highlight_with::<HistoryHighlighter>(...)`，背景 `background.base`，选区 `primary.base` 50% 透明；外层 `scrollable(...).id(HISTORY_SCROLL_ID).height(Fixed(history_height)).direction(Vertical(Scrollbar width 8, scroller 6)).anchor_bottom()`；面板顶部 header 右侧为 `Copy`（COPY 图标成功色）与 `Clear`（TRASH 图标警告色）按钮（`header_btn_style`）；左上角为 `RESIZE` 图标拖拽手柄（15px，`Interaction::ResizingVertically`）。
- **触发命令**：`Message::CommandHistoryCopy`、`CommandHistoryClear`、`CommandHistoryEdit`、`CommandHistoryResizeGrab/ResizeMove/ResizeRelease`、`CommandHistoryHeightReset`。
- **实现位置**：`src/ui/command_line.rs::CommandLine::view`、`HistoryHighlighter`、`history_highlight_settings/history_highlight_format/history_plain_text`；常量 `HISTORY_HEIGHT_MIN/DEFAULT/MAX`、`HISTORY_SCROLL_ID`、`history_max_height`；处理器 `src/app/update/mod.rs`（2360-2436）。
- **备注**：高度范围 `[72, min(560, 窗口高*0.60)]`；拖动上限随窗口高度（`history_max_height`）；双击手柄重置为默认 180；复制内容为 `history_plain_text()`（每行一条）；面板用 `opaque` 阻止滚轮透传到画布；历史打开时隐藏临时覆盖行（`recent_history` 为空，issue #555）。

### 历史读写与编辑器（History content）
- **功能简介**：历史面板所依托的只读文本内容；编辑动作被丢弃，保留选择/光标并让滚轮走外层滚动条。
- **UI 入口**：`命令行历史面板`。
- **样式**：只读。
- **触发命令**：`Message::CommandHistoryEdit(action)`。
- **实现位置**：`src/app/update/mod.rs::Message::CommandHistoryEdit`（`Action::Scroll` 转 `scroll_by`，非编辑动作 `perform`）。

---

## 十四、命令行交互：自动补全 / 历史导航 / 输入解析

### 自动补全建议面板（Autocomplete suggestions）
- **功能简介**：输入前缀匹配到命令时，在输入行上方显示最多 8 条建议，点击直接派发；高亮项与回车执行项一致。
- **UI 入口**：`命令行 → 输入框上方建议列表`（仅 `allow_autocomplete` 时，即无活动命令、无 grip 待定值）。
- **样式**：`container(col).style(container::bordered_box).width(Fill)`；每行 `button(row![icon(14px), text(cmd).size(11)].spacing(6)).width(Fill).padding([2,8])`，高亮行底 `primary.weak`、悬停 `background.weak`、否则 `background.base`；图标来自 `command_icon(cmd)`，无图标时留 14px 空位对齐。
- **触发命令**：`Message::CommandSuggestionPick(cmd)`。
- **实现位置**：`src/ui/command_line.rs::CommandLine::view/autocomplete_matches/selected_suggestion/autocomplete_prev/autocomplete_next`、`ranked_matches`；`allow_autocomplete` 见 `src/app/view/mod.rs`（1900）；处理器 `src/app/update/mod.rs::Message::CommandSuggestionPick`。
- **备注**：匹配规则：大小写不敏感子串、前缀优先、字母序、上限 `AUTOCOMPLETE_LIMIT = 8`；别名本身隐藏，仅显示其目标命令；当整个输入是某别名时，其目标命令强制置顶（`ranked_matches`，issue #288）。命令池 = 编译期注册表 `all_registered_command_names()` + 插件动态命令 `dynamic_commands`（issue #272）。

### 上下键历史导航（↑/↓ Recall）
- **功能简介**：↑ 回溯已派发命令，↓ 前进并最终回到编辑草稿；导航时保存草稿。
- **UI 入口**：`命令行 → 输入框内按 ↑/↓`。
- **样式**：输入框内容被替换，光标移至末尾。
- **触发命令**：`Message::CommandHistoryPrev` / `Message::CommandHistoryNext`（默认绑定 ↑/↓）；无活动命令且未在导航时优先走自动补全上下移动。
- **实现位置**：`src/ui/command_line.rs::history_prev/history_next/history_navigation_active/cancel_history_navigation`、字段 `cmd_recall/recall_cursor/recall_draft`；处理器 `src/app/update/mod.rs::Message::CommandHistoryPrev/CommandHistoryNext/CommandLineArrowProbe/CommandLineArrowResolved`。
- **备注**：`cmd_recall`/`recent_commands` 各上限 50，连续重复命令跳过（`record_recent`）；grip 弹窗打开时方向键走弹窗项；MText 编辑器打开时方向键走光标移动。

### 命令提交与令牌解析（run_command_line / feed_active_cmd）
- **功能简介**：把一行输入解析为：单命令派发、内联参数命令、或“首词启动交互工具 + 后续令牌作为点/关键字喂入”，最后按回车结束。
- **UI 入口**：`命令行 → 回车/空格提交`。
- **样式**：无（逻辑）。
- **触发命令**：`Message::CommandSubmit` → `on_command_submit` → `run_command_line`；`Message::CommandFinalize`（未聚焦输入的回车）→ `on_command_finalize`。
- **实现位置**：`src/app/update/command.rs::on_command_submit/on_command_finalize`；`src/app/command_driver/utilities.rs::run_command_line/run_command_line_streaming/finish_active_command/feed_active_cmd/feed_command/feed_command_consumed/try_selection_keyword`。
- **备注**：`>` 前导在提交时剥除（除非自由文本）；坐标按活动 UCS 解释（相对坐标由 UCS 轴旋转）；对象拾取步里关键字优先于十六进制句柄（如 TRIM 的 F/C/E，issue #336）；MTP/M2P 在点步进入两点中点修饰；未被任何步骤消费的令牌记入 `CommandLine::unconsumed`；`PAUSE`/`\` 支持挂起与后续令牌排空（`drain_pending_pause_tokens`）。

### 直距输入与表达式求值（Direct distance / expr）
- **功能简介**：在某点提示下输入裸数字时，先交给步骤自身解释（角度、因子、半径等），未被消费才作为沿光标方向的直距；数值输入支持表达式求值。
- **UI 入口**：`命令行 → 数字输入`。
- **样式**：无。
- **触发命令**：`feed_active_cmd` → `try_direct_distance_entry`；`on_command_submit` 中 `expr_eval::eval_to_string`。
- **实现位置**：`src/app/command_driver/utilities.rs::feed_active_cmd`；`src/app/update/command.rs::on_command_submit`。
- **备注**：测试示例 `5*2`、`Foo Bar`、`>Note`、`LINE 0,0 10,10`（`src/app/update/mod.rs` 测试段）。

### 选择关键字（Select objects: P/L/ALL/W/C/A/R）
- **功能简介**：在 “Select objects:” 提示下支持 Previous/Last/All 以及 Window/Crossing/Add/Remove 模式关键字。
- **UI 入口**：`命令行 → 选择阶段输入关键字`。
- **样式**：无。
- **触发命令**：`try_selection_keyword`（`P`/`PREVIOUS`、`L`/`LAST`、`ALL`、`W`/`WINDOW`、`C`/`CROSSING`、`A`/`ADD`、`R`/`REMOVE`）。
- **实现位置**：`src/app/command_driver/utilities.rs::try_selection_keyword`。
- **备注**：Previous 用上次工作集（`prev_selection`，issue #426）；Last 取当前空间最大句柄；W/C 锁定下一次框选的窗口/交叉模式（issue #596）；A/R 设置加/减选择模式并给提示。

### 命令步骤选项按钮（CmdOption buttons）
- **功能简介**：当前步骤提供的选项渲染为提示行内的小按钮，点击即喂入对应关键字，无需键入。
- **UI 入口**：`命令行 → 固定提示行内的选项按钮`。
- **样式**：`button(text(opt.label.to_uppercase()).size(11)).padding([1,6])`，悬停/按下底 `primary.weak`、否则 `background.weakest`，边框 `background.neutral` 1px 圆角 3；按钮行 `row![提示文本, 各按钮].spacing(6)`，`container(...).padding([1,8])`。
- **触发命令**：`Message::CommandOptionPick(kw)`（空关键字等价回车）。
- **实现位置**：`src/ui/command_line.rs::CommandLine::view/set_step_options/strip_option_listing`；字段 `step_options`；处理器 `src/app/update/mod.rs::Message::CommandOptionPick`（空 → `StepInput::Enter`，否则 `feed_active_cmd`）。
- **备注**：`set_step_options` 每帧由 `handle_need_point`/`resume_transparent_parent` 等刷新；固定提示里的 “[A / B]” 列表会被剥离，改由按钮呈现（`strip_option_listing`，issue #304）。

### 右键上下文菜单与命令行（Repeat / Recent Input / Snap Overrides）
- **功能简介**：视口右键菜单在命令运行时列出当前步骤选项（`CmdOption`）、Enter/Cancel、Recent Input、Snap Overrides 与透明 Pan/Zoom；空闲时列出 Repeat、Recent Input 等。
- **UI 入口**：`视口 → 右键 → 上下文菜单`（`MenuContext::Command` / `Idle`）。
- **样式**：`build_context_menu` 生成 `ContextMenu`，宽 `MENU_WIDTH`，有提示时用 `MENU_WIDTH_WITH_HINTS`。
- **触发命令/项**：
  - Command：`Enter`(`MenuAction::Enter`) + `Cancel`(`Cancel`)，随后为各 `CmdOption` 行（`MenuAction::Option`），`Recent Input ▸`（`MenuAction::FeedInput`），`Snap Overrides ▸`（`MenuAction::SnapOverride`/`Mtp`/`SnapOverrideNone`/`Osnap Settings...`），`Pan`/`Zoom`（透明 `'PAN`/`'ZOOM DYNAMIC`）。
  - Idle：`Repeat %{last}`、`Recent Input ▸` 等。
- **实现位置**：`src/ui/popup/context_menu.rs::build_context_menu/command_rows/idle_rows/snap_override_items/navigation_rows`；`MenuContext`；处理器 `src/app/update/context_menu.rs`（`MenuAction::Cancel → CommandEscape`、`Option(kw) → CommandOptionPick`）。
- **备注**：`RECENT_LIMIT` 限制 Recent Input/Repeat 条数；Recent Input 子菜单总是列出（无内容时禁用）；`CmdOption` 见 `src/command.rs`。

### 最近输入记录（Recent Input）
- **功能简介**：记录喂给运行中命令的令牌（点、距离、关键字），供右键菜单一键复用。
- **UI 入口**：`命令行 → 右键 → Recent Input ▸`。
- **样式**：菜单子项。
- **触发命令**：`CommandLine::record_recent_input`（追加）、`MenuAction::FeedInput`（复用）。
- **实现位置**：`src/ui/command_line.rs::record_recent_input`、常量 `RECENT_INPUT_CAP = 10`；字段 `recent_inputs`。
- **备注**：最新在前、大小写不敏感去重置顶；空、超 40 字符、实体句柄令牌跳过。

### 最近命令记录（Recent Commands / Repeat）
- **功能简介**：记录实际派发的命令（来自任意来源），驱动右键菜单的 Repeat 与 Recall。
- **UI 入口**：`视口 → 右键 → Repeat %{last}`。
- **样式**：菜单项。
- **触发命令**：`CommandLine::record_recent`（在派发咽喉点调用）。
- **实现位置**：`src/ui/command_line.rs::record_recent`；字段 `recent_commands`（上限 50）。

### 命令行的 MCP 等待提示（Pick pending）
- **功能简介**：当 MCP 客户端挂起 `user_select`/`getpoint` 等待用户拾取时，命令行状态按钮显示蓝色“等待拾取”提示。
- **UI 入口**：`命令行 → MCP 按钮`。
- **样式**：见「MCP 控制按钮」。
- **触发命令**：`Message::ControlToggle`；`pending_pick_label()` 决定 `pick_pending`。
- **实现位置**：`src/ui/command_line.rs::mcp_status`；`src/app/view/mod.rs`（`self.pending_pick_label().is_some()`）。

---

## 十五、命令行相关命令与系统变量

### 命令行历史显示行数（CLIPROMPTLINES）
- **功能简介**：设置命令行窗口上方临时提示行数量（0–50，默认 3）。
- **UI 入口**：`命令行输入 → CLIPROMPTLINES`。
- **样式**：无。
- **触发命令**：`CLIPROMPTLINES`（裸命令进入值提示；“CLIPROMPTLINES <n>” 直接设值）。
- **实现位置**：`src/ui/command_line.rs::set_cliprompt_lines`、字段 `cliprompt_lines`；`src/app/commands/styleprops.rs::"CLIPROMPTLINES"`（约 1994 行，含 SETVAR 支持）。

### 命令行淡出时间（COMMANDLINEFADETIME）
- **功能简介**：设置历史覆盖行的可见时长（毫秒，0–60000，默认 3000；0 完全跳过临时行，固定提示仍显示）。
- **UI 入口**：`命令行输入 → COMMANDLINEFADETIME`。
- **样式**：无。
- **触发命令**：`COMMANDLINEFADETIME`（裸命令进入值提示；“COMMANDLINEFADETIME <ms>” 设值）。
- **实现位置**：`src/ui/command_line.rs::set_commandline_fade_ms/commandline_fade_ms/fade_secs/entry_visible`；`src/app/commands/styleprops.rs::"COMMANDLINEFADETIME"`（约 939、2008 行）。

### 坐标读out模式（COORDS）
- **功能简介**：设置坐标显示模式 0/1/2（static/live/polar），与状态栏坐标药丸循环同源。
- **UI 入口**：`命令行输入 → COORDS`（或状态栏坐标药丸点击）。
- **样式**：无。
- **触发命令**：`COORDS`；SETVAR 支持。
- **实现位置**：`src/ui/statusbar/mod.rs::format_coords`；`src/app/commands/styleprops.rs::"COORDS"`（约 2373 行）；处理器 `src/app/update/mod.rs::Message::CycleCoordsMode`。

### 线宽显示（LWDISPLAY）
- **功能简介**：ON/OFF 视口线宽显示，与状态栏 LWT 药丸同源。
- **UI 入口**：`命令行输入 → LWDISPLAY [ON|OFF]`。
- **样式**：无。
- **触发命令**：`LWDISPLAY`。
- **实现位置**：`src/app/commands/styleprops.rs::"LWDISPLAY"`（3200-3228 行）。

### 注释可见性/自动加比例（ANNOALLVISIBLE / ANNOAUTOSCALE）
- **功能简介**：前者设注释对象是否全可见（0/1），后者设自动加比例模式（-4..4）。
- **UI 入口**：`命令行输入 → ANNOALLVISIBLE / ANNOAUTOSCALE`（或状态栏对应药丸）。
- **样式**：无。
- **触发命令**：`ANNOALLVISIBLE`、`ANNOAUTOSCALE`；SETVAR 支持。
- **实现位置**：`src/app/commands/display.rs`（1373-1414 行）；处理器 `src/app/update/mod.rs::Message::ToggleAnnotationVisibility/ToggleAnnotationAutoAdd`。

### 其他制图系统变量（通过 SETVAR 与状态栏相关）
- **功能简介**：SETVAR 报告/设置一批制图变量，其中 `CLIPROMPTLINES`、`COMMANDLINEFADETIME`、`ORTHOMODE`、`SNAPANG` 等与状态栏/命令行相关。
- **UI 入口**：`命令行输入 → SETVAR`。
- **样式**：无。
- **触发命令**：`SETVAR`。
- **实现位置**：`src/app/commands/styleprops.rs::"SETVAR"`（报告串含 `CLIPROMPTLINES COMMANDLINEFADETIME` 等，1111 行附近）。

---

## 十六、草稿设置与状态栏开关的联动（Drafting Settings）

### 草稿设置对话框状态与状态栏同步（DraftingSettingsState::from_app）
- **功能简介**：从应用当前状态构建草稿设置对话框，并在应用时把网格/捕捉/等轴测/极轴/正交/对象捕捉/追踪/DYN/快速属性/选择循环等写回状态栏所依赖的字段。
- **UI 入口**：`状态栏 → OSNAP ▾（或命令 DSETTINGS / OSNAP） → 草稿设置对话框`。
- **样式**：对话框（见 `src/ui/window/drafting_settings.rs`）；本文件只负责状态映射。
- **触发命令**：`DSETTINGS`/`OSNAP` → `Message::ToggleSnapPopup`；应用 `apply_drafting_settings`。
- **实现位置**：`src/app/drafting_settings.rs::DraftingSettingsState::from_app/OpenCADStudio::apply_drafting_settings`；处理器 `src/app/update/mod.rs::Message::ToggleSnapPopup/CloseSnapPopup`。
- **备注**：`apply_drafting_settings` 校验捕捉/网格间距为正、Major every 2-100；`snap_equal` 锁定时 Y 跟随 X；写回字段包括 `show_grid`、`grid_spacing_*`、`grid_major_every`、`grid_adaptive`、`grid_beyond_limits`、`snapper.grid_snap_on`、`snap_spacing_*`、`isometric_drafting`、`iso_plane`、`snap_angle_deg`、`polar_mode`、`ortho_mode`、`polar_increment_deg`、`snapper.snap_enabled`、`snapper.otrack_enabled`、`snapper.enabled`、`snapper.snap3d_enabled`、`snapper.enabled3d`、`dyn_input`、`quick_properties`、`selection_cycling`。脏检查 `drafting_settings_dirty/is_dirty`。

### 3D 对象捕捉开关（ToggleSnap3dEnabled）
- **功能简介**：独立于 2D 对象捕捉的 3D 捕捉总开关（F4）。
- **UI 入口**：`状态栏 OSNAP ▾ → 草稿设置 → 3D 对象捕捉页`（不在 OSNAP 弹出层）。
- **样式**：对话框内。
- **触发命令**：`Message::ToggleSnap3dEnabled`（F4→TOGGLE3DOSNAP）。
- **实现位置**：`src/app/update/mod.rs::Message::ToggleSnap3dEnabled`；`src/snap.rs::ALL_3D_SNAP_MODES`；`src/app/shortcuts.rs` F4。

---

## 十七、别名 / 快捷键与命令行

### 命令别名（Aliases，ocad.pgp）
- **功能简介**：以 `.pgp` 文本文件定义短别名（如 `L` → `LINE`、`CC` → `COPYCLIP`），在派发前改写命令动词，保留后续参数。
- **UI 入口**：`命令行 → 输入别名`；别名文件 `ocad.pgp` 可手编，或经 `ALIASEDIT` 命令编辑。
- **样式**：无（文本文件）；头注含 `;` 注释与 `ALIAS,*COMMAND` 格式。
- **触发命令**：`resolve_alias` 在派发前解析；`ALIASEDIT` 打开编辑器。
- **实现位置**：`src/app/alias.rs::resolve_alias/parse_pgp/to_pgp/load_aliases/save_map/set_command_aliases/apply_alias_editor_rows/finish_alias_editor/reset_aliases_to_defaults/alias_editor_dirty`；默认别名嵌入自 `assets/ocad.pgp`。
- **备注**：别名大小写不敏感、统一大写存储；版本化默认别名迁移（`DEFAULT_ALIASES_VERSION = 4`，v2 引入 `R/RA/RE/REA`，v3 引入 `HB`，v4 引入 `ER`；`introduced_at`/`migrate_aliases` 只增不删，保留用户覆盖与删除）；别名在自动补全中被隐藏，但其目标命令仍显示（`ranked_matches`，issue #288）。

### 快捷键与命令行（default_bindings）
- **功能简介**：功能键直接驱动状态栏开关与命令行动作。
- **UI 入口**：`键盘`。
- **样式**：无。
- **触发命令/键**（`default_bindings`，全部相关项）：
  - `F1`→`HELP`、`F2`→`COMMANDHISTORY`（开合历史面板）、`F3`→`TOGGLEOSNAP`、`F4`→`TOGGLE3DOSNAP`、`F5`→`ISOPLANE`、`F7`→`GRID`、`F8`→`ORTHO`、`F9`→`SNAP`、`F10`→`POLAR`、`F11`→`OTRACK`、`F12`→`DYNINPUT`、`Ctrl+0`（macOS `Cmd+0`）→`CLEANSCREEN`、`ENTER`→`FINALIZE`、`SPACE`→`COMMANDSPACE`、`ESCAPE`→`CANCEL`、`BACKSPACE`→`BACKSPACE`、`TAB`→`DYNTAB`、`UP`→`HISTORYPREV`、`DOWN`→`HISTORYNEXT`。
- **实现位置**：`src/app/shortcuts.rs::default_bindings/run_shortcut`（`COMMANDHISTORY`→`CommandHistoryToggle` 等映射）；`OPEN` 等命令名映射见同文件。

### 命令输入框焦点（focus_cmd_input）
- **功能简介**：在命令结果处理后、切换空间、选项点击等场景把焦点交回命令输入框；动态输入/文本编辑器占用时让出焦点。
- **UI 入口**：`命令行输入框`。
- **样式**：无。
- **触发命令**：`OpenCADStudio::focus_cmd_input`（内部）。
- **实现位置**：`src/app/view/mod.rs::focus_cmd_input`；调用点 `src/app/command_driver/mod.rs::apply_cmd_result_inner`（按 `mtext_editor`/`text_inline` 分流）。

### 空间改变时的命令行清理（cancel_active_command_for_space_change）
- **功能简介**：切换图纸/模型空间时取消活动命令、清空输入、关闭历史、清自动补全与交互状态。
- **UI 入口**：`状态栏标签切换 / MSPACE-PSPACE 切换`。
- **样式**：无。
- **触发命令**：`OpenCADStudio::cancel_active_command_for_space_change`。
- **实现位置**：`src/app/command_driver/utilities.rs::cancel_active_command_for_space_change`。

---

## 十八、Options 会话与状态栏配置持久化

### 状态栏配置持久化（StatusBarConfig in AppConfig）
- **功能简介**：隐藏药丸集合作为 `AppConfig.statusbar` 段随用户配置一起持久化，跨会话保留。
- **UI 入口**：`状态栏 → 自定义句柄 → 勾选切换`。
- **样式**：无。
- **触发命令**：`Message::ToggleStatusPill` → `statusbar_config.toggle` + `save_config`。
- **实现位置**：`src/ui/statusbar/statusbar_config.rs::StatusBarConfig`（`hidden: HashSet<StatusPill>`，serde `default`）；`src/app/config.rs::AppConfig.statusbar`（JSON 文件 `<config>/OpenCADStudio/settings.json`，web 为 `localStorage` 键 `opencadstudio.settings`）。

### Options 窗口的提交/回退（Options Session）
- **功能简介**：Options 窗口改动即时可见，但 Apply/OK 前不落盘；Close/×/Esc 回退到上次提交快照，含被编辑的绘图头变量与 textfill。
- **UI 入口**：`菜单/命令 → Options…`。
- **样式**：无（对话框逻辑）。
- **触发命令**：`Message::OptionsOpen/Apply/Ok/Close/CloseDiscard/CloseKeep`。
- **实现位置**：`src/app/options_session.rs::options_open/options_apply/options_discard/options_forget/options_dirty/options_revert_to`、`OptionsSnapshot/HeaderVars`。
- **备注**：快照含 `AppConfig`、活动标签页的 `HeaderVars`（isolines、display_silhouette、surface_u/v_density、surface_type、record_solid_history、show_solid_history、tab_dirty）与 `textfill`；`apply_config` 分发全部持久化偏好（主题、语言等）。

### 配置项（AppConfig 与状态栏/命令行相关字段）
- **功能简介**：所有界面偏好集中在一个 JSON。
- **UI 入口**：无独立按钮。
- **样式**：无。
- **触发命令**：`save_config` / `apply_config`。
- **实现位置**：`src/app/config.rs::AppConfig`（含 `statusbar`、`annotation_auto_scale`、`ribbon`、`theme`、`shortcuts`、`model_space` 等）、`AppConfig::load/save`。
- **备注**：`annotation_auto_scale` 默认 `-4`；`statusbar` 默认隐藏 Coords/Lwt/Dyn/Space/Units/Transparency/SelCycle/Vp。

---

（文档结束）
