# 右键上下文菜单、弹出/浮动层、模态框与宽菜单功能需求清单

本文档反推自 `src/ui/popup/`（`context_menu.rs`、`cycle_popup.rs`、`isolate_popup.rs`、`polar_popup.rs`、`scale_popup.rs`、`selection_filter_popup.rs`、`snap_popup.rs`、`units_popup.rs`）、`src/app/update/context_menu.rs`、`src/app/view/overlay.rs`、`src/app/view/mod.rs`、`src/ui/modal.rs`、`src/ui/overlay.rs`、`src/ui/wide_menu.rs`、`src/ui/read_only.rs`、`src/ui/node_graph.rs`、`src/ui/statusbar/mod.rs`、`src/ui/statusbar/status_menu.rs`、`src/ui/command_line.rs` 与 `src/app/mod.rs`（`ModalKind`）。覆盖各类右键菜单的全部菜单项、每个弹出层/浮层的触发与关闭、通用模态框架、画布浮层种类、宽菜单、只读字段与节点图浮层。

---

## 一、视口右键上下文菜单（`src/ui/popup/context_menu.rs`）

视口右键菜单是「模型/视图」分离的纯函数实现：`build_context_menu` 依据 `MenuContext` 生成行（`ContextMenu`），渲染器与键盘处理器走同一批行，因此「所见」与「按键所选」不会分叉。三种上下文：`MenuContext::Command`（命令运行中）、`MenuContext::Idle`（空闲）、`MenuContext::Grip`（夹点编辑中）。

### 菜单通用外观与度量
- **功能简介**：决定右键菜单的宽度、行高、默认行位置、快捷键提示列与禁用态。
- **UI 入口**：`视口 → 右键 → 上下文菜单`（面板以光标处为锚点，默认行正压在光标下）。
- **样式**：面板行高 `MENU_ROW_H = 22.0`，分隔行 `MENU_SEP_H = 7.0`，顶部内边距 `MENU_PAD_TOP = 4.0`；无提示列宽 `MENU_WIDTH = 200.0`，任一行带右侧关键字提示时加宽到 `MENU_WIDTH_WITH_HINTS = 236.0`；行有图标槽（`MENU_GUTTER_W`、`MENU_ICON_SIZE`）、`✓` 勾选前缀、默认行加粗、禁用行文字淡化（`context_menu_row`、`context_menu_row_style`）。
- **触发命令**：右键（受 `SHORTCUTMENU`/`RightClickMode` 控制，见第十一节）。
- **实现位置**：`src/ui/popup/context_menu.rs::build_context_menu`、`ContextMenu::selectable/default_index/find_mnemonic/default_row_y/height`；渲染 `src/app/view/overlay.rs::viewport_context_menu_overlay`、`context_menu_row`、`context_menu_row_style`、`context_menu_gutter`。
- **备注**：Recent Input / Repeat 列最多显示 `RECENT_LIMIT = 10` 条；子菜单为内联手风琴（`SubmenuId`：`RecentInput`、`Clipboard`、`DrawOrder`、`Isolate`、`SnapOverrides`），同一时刻只展开一个。行的键盘助记符取选项关键字首字母（`mnemonic_for`）。

### 1.1 命令运行中菜单（`MenuContext::Command` / `command_rows`）
- **功能简介**：命令执行期间右键，菜单以「Enter/Cancel + 该步骤自己的关键字选项 + 最近输入 + 捕捉覆盖 + 透明平移缩放」组织，让用户既能点选关键字又能看到其键盘简写。
- **UI 入口**：`视口 → 右键（有命令运行）→ 命令上下文菜单`。
- **样式**：行列表，关键字行右侧显示关键字提示；默认行为 `Enter` 行（`default()`，加粗）。
- **菜单项（全部，`src/ui/popup/context_menu.rs::command_rows`）**：
  - `Enter` — 完成/接受当前步骤（`MenuAction::Enter`）；提示 `⏎`；若该命令提供完成选项且标签为 `<...>`（如 OFFSET 的 `<0.5000>`）则提示显示该默认值（`enter_hint`）。
  - `Cancel` — 取消命令（`MenuAction::Cancel`，提示 `Esc`）。
  - 【分隔】
  - 该步骤的每一个非空关键字选项（`CmdOption`，按顺序）：标签行 + 右侧关键字提示 + 助记符（`MenuAction::Option(keyword)`）。
  - 【分隔】
  - `Recent Input ▸` — 最近输入子菜单（`SubmenuId::RecentInput`），内容为最近键入的点/距离/关键字，最多 10 条；选中后回喂给命令（`MenuAction::FeedInput`）。无内容时该子菜单头禁用。
  - `Snap Overrides ▸` — 仅当步骤可接受点拾取（`has_point_step`）时出现（`SubmenuId::SnapOverrides`，见 1.4）。
  - 【分隔】
  - `Pan` — 透明执行 `'PAN`，命令稍后恢复（`navigation_rows(true)`，图标 `pan_icon`）。
  - `Zoom` — 透明执行 `'ZOOM DYNAMIC`（图标 `zoom_icon`）。
- **实现位置**：`context_menu.rs::command_rows`、`recent_input_submenu`、`navigation_rows`；`CmdOption` 来源为活动命令的 `options()`；上下文组装 `src/app/update/context_menu.rs::context_menu_context`。
- **备注**：若命令自己命名完成动作（`CmdOption::enter(...)`），该行即 `Enter`，不重复列出（`src/ui/popup/context_menu.rs` 头注与 `enter_option_becomes_default_row` 测试）。

### 1.2 空闲菜单（`MenuContext::Idle` / `idle_rows`）
- **功能简介**：无命令运行时右键，提供重复上一命令、最近命令、剪贴板、撤销/重做、选择编辑、隔离、导航、选择工具与面板开关。
- **UI 入口**：`视口 → 右键（空闲）→ 空闲上下文菜单`。
- **样式**：同通用外观；有选择时默认行为 `Repeat <上一命令>`（加粗）。
- **菜单项（全部，分选择集有无，`src/ui/popup/context_menu.rs::idle_rows`）**：
  - `Repeat %<最近命令>` — 重复最近命令（`MenuAction::Command(大写)`，带命令图标，默认行）；仅在存在最近命令时出现。
  - `Recent Input ▸` — 最近命令子菜单（`SubmenuId::RecentInput`），最多 10 条，选中即运行（`MenuAction::Command`）。
  - 【分隔】
  - `Clipboard ▸`（`SubmenuId::Clipboard`，全部子项）：
    - `Cut`（`CUTCLIP`）— 有选择才可用。
    - `Copy`（`COPYCLIP`）— 有选择才可用。
    - `Copy with Base Point`（`COPYBASE`）— 有选择才可用。
    - `Paste`（`PASTECLIP`）— 剪贴板非空才可用。
    - `Paste as Block`（`PASTEBLOCK`）— 剪贴板非空才可用。
    - `Paste to Original Coordinates`（`PASTEORIG`）— 剪贴板非空才可用。
  - 【分隔】
  - `Undo %<标签>` — 撤销（`MenuAction::Undo`，图标 `undo_icon`，提示 `Ctrl+Z`）；无历史时禁用。
  - `Redo %<标签>` — 重做（`MenuAction::Redo`，图标 `redo_icon`，提示 `Ctrl+Y`）；无历史时禁用。
  - 【分隔】
  - 选择编辑块（仅当有选择，`has_selection`）：
    - `Erase`（`MenuAction::DeleteSelected`，命令图标 `ERASE`）。
    - `Move`（`MOVE`）。
    - `Copy Selection`（`COPY`）。
    - `Scale`（`SCALE`）。
    - `Rotate`（`ROTATE`）。
    - `Mirror`（`MIRROR`）。
    - `Draw Order ▸`（`SubmenuId::DrawOrder`）：
      - `Bring to Front`（`DRAWORDER F`）
      - `Send to Back`（`DRAWORDER B`）
      - `Bring Above Object`（`MenuAction::DrawOrderPickRef(true)`，先选参照对象）
      - `Send Under Object`（`MenuAction::DrawOrderPickRef(false)`）
    - 【分隔】
  - 若未选择但选中了约束字形（`selected_constraint`）：`Delete`（`MenuAction::ConstraintDelete(id)`），然后分隔。
  - `Isolate ▸`（`SubmenuId::Isolate`，图标 `ST_ISOLATE`）：
    - `Isolate Objects`（`ISOLATEOBJECTS`）— 有选择才可用。
    - `Hide Objects`（`HIDEOBJECTS`）— 有选择才可用。
    - `End Object Isolation`（`UNISOLATEOBJECTS`）— 处于隔离状态才可用。
  - 【分隔】
  - `Pan`（`PAN`，图标 `pan_icon`）、`Zoom`（`ZOOM DYNAMIC`，图标 `zoom_icon`）— 非透明（`navigation_rows(false)`）。
  - `Zoom Extents`（`ZOOM EXTENTS`）— 仅当无选择时出现。
  - 【分隔】
  - 选择工具：有选择时 `Select Similar`、`Invert Selection`、`Deselect All`；无选择时 `Select All`（`SELECTALL`）。始终有 `Quick Select...`（`MenuAction::QuickSelect`）。
  - 【分隔】
  - `Properties` — 打开/切换特性面板（`MenuAction::Properties`，命令图标 `PROPERTIES`，面板已开时显示勾选 `checked(props_open)`）。
  - `Options...` — 打开选项（`MenuAction::Options`，命令图标 `OPTIONS`）。
- **实现位置**：`context_menu.rs::idle_rows`。
- **备注**：`Properties` 行勾选随 `props_open` 变化；`Quick Select...` 触发 `Message::QSelectOpen`（见第六节）。

### 1.3 夹点菜单（`MenuContext::Grip` / `grip_rows`）
- **功能简介**：夹点热/拖动时右键，提供夹点模式快捷菜单（拉伸/移动/旋转/缩放/镜像、基点、复制、撤销、退出）。
- **UI 入口**：`视口 → 夹点热时右键 → 夹点上下文菜单`。
- **样式**：同通用外观；当前模式行显示勾选；`Undo` 在未拖动时禁用。
- **菜单项（全部，`src/ui/popup/context_menu.rs::grip_rows`）**：
  - `Enter` — 提交夹点编辑（默认行，提示 `⏎`）。
  - 【分隔】
  - `Stretch` — 切到拉伸模式（`GripMenuCmd::Stretch`，当前 `GripEditMode::Stretch` 时勾选）。
  - `Move`（`GripMenuCmd::Move`，命令图标 `MOVE`）。
  - `Rotate`（`ROTATE`）。
  - `Scale`（`SCALE`）。
  - `Mirror`（`MIRROR`）。
  - 【分隔】
  - `Base Point`（`GripMenuCmd::BasePoint`）。
  - `Copy`（`GripMenuCmd::CopyToggle`，`grip.copy_on` 时勾选）。
  - `Undo`（`GripMenuCmd::Undo`，夹点已移动 `grip.moved` 才可用）。
  - 【分隔】
  - `Exit`（`GripMenuCmd::Exit`，提示 `Esc`）。
- **实现位置**：`context_menu.rs::grip_rows`；行为 `src/app/update/context_menu.rs::on_grip_context_pick`。
- **备注**：`Move/Rotate/Scale/Mirror` 会把选择交给对应命令并以夹点为基点（直接开到第二个提示）；`Base Point` 重设基点不放置；`Copy` 切换复制；`Undo` 把几何复位且夹点保持热（`on_grip_context_pick`）。

### 1.4 捕捉覆盖子菜单（Snap Overrides）
- **功能简介**：为下一次点拾取选择一次性对象捕捉（不改变全局 OSNAP 开关），并提供 M2P 与捕捉设置入口。
- **UI 入口**：`视口 → 右键（命令点步骤）→ 命令上下文菜单 → Snap Overrides ▸`。
- **样式**：子菜单行，每行带对应捕捉 SVG 图标（`MenuIcon::Snap`）或 M2P 图标。
- **菜单项（全部，`src/ui/popup/context_menu.rs::snap_override_items`）**：`Mid Between 2 Points`（M2P，`MenuAction::Mtp`）、`Endpoint`、`Midpoint`、`Intersection`、`Apparent Intersection`、`Extension`、`Center`、`Quadrant`、`Tangent`、`Perpendicular`、`Parallel`、`Node`、`Insertion`、`Nearest`（以上各 `MenuAction::SnapOverride(SnapType)`）、`None`（`MenuAction::SnapOverrideNone`）、`Osnap Settings...`（`MenuAction::Command("OSNAP")`）。
- **触发命令**：右键菜单；参考命令 `OSNAP`。
- **实现位置**：`context_menu.rs::snap_override_items`；`ALL_SNAP_MODES`（`src/snap`）。
- **备注**：仅点拾取步骤出现；一次性覆盖在下一次拾取后失效（`snap_override_none_is_one_shot` 测试）。

### 1.5 键盘导航与拾取路由
- **功能简介**：菜单打开时支持 ↑/↓ 移动高亮、Enter 选择、字母助记符选择、←/→ 展开/收起子菜单；无法识别的可打印键会关闭菜单并落入命令行。
- **UI 入口**：`视口 → 右键打开菜单 → 键盘`。
- **样式**：高亮行使用主题主色（`context_menu_row_style`，与夹点弹窗一致），提示文字在高亮时提亮。
- **触发命令/按键**：`↑`/`↓`（`ContextMenuNav::Up/Down`）、`Enter`（`ContextMenuNav::Enter`）、字母（`ContextMenuNav::Mnemonic`）、`←`/`→` 子菜单展开/收起、`Esc` 取消。
- **实现位置**：`src/app/update/context_menu.rs::on_context_menu_navigate`、`intercept_context_menu_key`、`context_menu_typed`、`on_context_menu_submenu_toggle`、`on_context_menu_pick`；字母循环 `ContextMenu::find_mnemonic`。
- **备注**：重复字母（如 PLINE 的 `CEnter`/`CLose`）会循环高亮，唯一字母立即选中；键入未匹配到助记符的字符时菜单让路给命令行（`menu_unmatched_char_closes_menu_and_types` 测试）。

---

## 二、Shift+右键一次性捕捉覆盖网格（Viewport）

### 2.1 捕捉覆盖弹出网格
- **功能简介**：按住 Shift 右键，在光标处弹出仅含图标的捕捉网格；点一个即对下一次点拾取生效，MTP 单元则挂起提示改取两点中点。
- **UI 入口**：`视口 → Shift + 右键 → 捕捉覆盖网格`。
- **样式**：4 列图标网格（`COLS = 4`），单元 26×26、图标 16px、悬停底色为主题弱主色；每个单元悬停显示名称 tooltip（`Position::Bottom`），无文字标签。
- **触发命令**：Shift+右键（`Message::ViewportRightPress` 中 `shift_down` 分支设置 `snap_override_popup`）。
- **实现位置**：`src/app/view/overlay.rs::snap_override_overlay`；触发 `src/app/update/mod.rs::ViewportRightPress`；渲染挂载 `src/app/view/mod.rs`（`snap_override_layer`）。
- **备注**：单元来自 `crate::snap::ALL_SNAP_MODES` 再加末尾 MTP（`_M2P`）；选择后 `snap_override_popup` 清空（`src/app/update/mod.rs` 多处清空）。这与 1.4 的 `Snap Overrides ▸` 子菜单是同一功能的两种入口。

---

## 三、其他 iced_aw 上下文菜单（标签与列表行）

这些菜单由 `iced_aw::ContextMenu` 托管打开、定位、边界钳制与关闭。

### 3.1 文档标签右键菜单
- **功能简介**：右键绘图文档标签，批量保存/关闭，或定位文件。
- **UI 入口**：`顶部文档标签栏 → 右键某个文档标签 → 菜单`。
- **样式**：面板宽 `MENU_W = 210.0`，`container::bordered_box`，行 12px 文字、内边距 `[4,12]`；不可用项文字淡化。
- **菜单项（全部，`src/app/view/mod.rs::doc_tab_context_menu`）**：
  - `Save All`（`Message::DocTabSaveAll`）。
  - `Close All`（`Message::DocTabCloseAll`）。
  - `Close All Other Drawings`（`Message::DocTabCloseOthers(tab_idx)`，仅当还有其它文档时可用）。
  - `Copy Full File Path`（`Message::DocTabCopyFullPath(tab_idx)`，仅非 wasm 且当前有路径时可用）。
  - `Open File Location`（`Message::DocTabOpenFileLocation(tab_idx)`，同上）。
- **实现位置**：`src/app/view/mod.rs::doc_tab_context_menu`、`doc_tab_bar`。
- **备注**：`Copy Full File Path` / `Open File Location` 在 `cfg!(not(target_arch = "wasm32"))` 下才加入。

### 3.2 布局（Layout）标签右键菜单
- **功能简介**：右键图纸布局标签，重命名或删除该布局。
- **UI 入口**：`状态栏 → 布局标签（如 Layout1）→ 右键 → 菜单`。
- **样式**：`container::bordered_box`，面板宽 160，行 12px、内边距 `[4,12]`。
- **菜单项（全部，`src/ui/statusbar/mod.rs::layout_tab_context_menu`）**：
  - `Rename`（`Message::LayoutRenameStart(name)`）— 启动内联重命名输入框（`LAYOUT_RENAME_INPUT_ID`，`✕` 取消按钮）。
  - `Delete`（`Message::LayoutDelete(name)`）。
- **实现位置**：`src/ui/statusbar/mod.rs::layout_tab_context_menu`、`space_tab`。
- **备注**：仅「可重排布局」（`reorderable_layouts` 含该标签）才包 `ContextMenu`；`Model` 标签无右键菜单、不可重命名；点击标签左键切换布局。

### 3.3 外部参照管理器行右键菜单
- **功能简介**：外部参照管理器列表行右键的上下文操作（引用/路径等）。
- **UI 入口**：`外部参照管理器窗口 → 列表行 → 右键 → 菜单`。
- **样式**：`iced_aw::ContextMenu` 包裹行，`RightClickArea` 捕获右键（`Message::XrefRowRightClick(index)`）。
- **实现位置**：`src/ui/window/xref_manager.rs::row_menu_for`、`RightClickArea`、`Row`（`06-...` 或外部参照文档详列子项）。
- **备注**：其子项内容属于外部参照功能，本文档不逐项展开，符号见上。

---

## 四、状态栏弹出层（`src/ui/popup/` 与 `src/ui/statusbar/status_menu.rs`）

状态栏各下拉由 `status_menu::menu_bar` 用 `iced_aw::MenuBar` 承载；`Entry::stay`（点击后保持打开）与 `Entry::close`（点击后关闭）决定行为。菜单项定义在 `src/ui/popup/` 各文件。

### 4.1 对象捕捉（OSNAP）状态菜单
- **功能简介**：开关全局对象捕捉、逐项开关捕捉类型、设置等轴测草图平面，并提供全选/全清。
- **UI 入口**：`状态栏 → 对象捕捉按钮 ▾ → 捕捉列表`。
- **样式**：面板宽 210；顶部 `Select All`/`Clear All` 小按钮（`button::secondary`，字号 10），分隔线，等轴测区块（复选框 + 平面按钮 + “F5 cycles the active plane.” 提示），随后每个捕捉一行：勾选格 + SVG 图标（16px 槽）+ 名称（11px）。
- **触发命令**：点击捕捉按钮下拉箭头；`F3` 切换开关。
- **菜单项（全部，`src/ui/popup/snap_popup.rs::menu_entries`）**：`Select All`（`SnapSelectAll`）、`Clear All`（`SnapClearAll`）、`Isometric drafting` 复选框（`ToggleIsometricDrafting`）、等轴测平面按钮（`IsoPlane::ALL`，`SetIsoPlane`）、随后 `ALL_SNAP_MODES` 每一项（`ToggleSnap(SnapType)`）。
- **实现位置**：`src/ui/popup/snap_popup.rs::menu_entries`、`snap_row`、`header_btn`；状态栏挂载 `src/ui/statusbar/mod.rs::osnap_btn`。
- **备注**：条目行的勾选状态来自 `Snapper::is_on`；全选/全清按钮按当前状态禁用。

### 4.2 极轴追踪（POLAR）状态菜单
- **功能简介**：选择极轴增量角度或自定义角度。
- **UI 入口**：`状态栏 → 极轴按钮 ▾ → 角度列表`。
- **样式**：每行勾选格 + 角度标签（11px）；底部自定义行内嵌 `text_input`（宽 58）加 `°`。
- **菜单项（全部，`src/ui/popup/polar_popup.rs::menu_entries`，`PRESETS`）**：`90°`、`45°`、`30°`、`22.5°`、`18°`、`15°`、`10°`、`5°`、`1°`（各 `Message::SetPolarAngle(deg)`），及 `Custom…` 自由输入（`PolarCustomInput` 输入、`SubmitPolarCustom` 提交）。
- **实现位置**：`src/ui/popup/polar_popup.rs::menu_entries`、`angle_label`、`angle_row`；状态栏 `polar_pill`。
- **备注**：右键极轴按钮可直接跳到下一个角度（`src/ui/statusbar/mod.rs` 中 `on_right_press(Message::SetPolarAngle(next_angle))`）。

### 4.3 注释/视口比例（Scale）状态菜单
- **功能简介**：选择模型空间的注释比例或图纸空间视口比例；模型空间另有 `Manage...` 打开比例管理器。
- **UI 入口**：`状态栏 → 比例按钮 ▾ → 比例列表`。
- **样式**：每行勾选格 + 比例名（11px）；`Manage...` 行用 `button::primary`。
- **菜单项（全部，`src/ui/popup/scale_popup.rs::menu_entries`）**：来自图纸 `ACAD_SCALELIST` 的每个比例（模型空间发 `Message::SetAnnotationScale(label)`，图纸空间发 `SetViewportScale(label)`）；模型空间末尾追加 `Manage...`（`Message::ScaleManagerOpen`）。
- **实现位置**：`src/ui/popup/scale_popup.rs::menu_entries`、`scale_row`、`manage_row`；状态栏 `scale_element`。
- **备注**：只显示文件中实际存在的比例，不注入额外比例；高亮由当前比例名/视口比例决定。

### 4.4 图形线性格式（Units）状态菜单
- **功能简介**：选择长度书写格式（LUNITS），并提供单位对话框与图形转换入口。
- **UI 入口**：`状态栏 → 单位按钮 ▾ → 线性格式列表`。
- **样式**：每行勾选格 + 格式名（11px）+ 右侧其自身的样例（10px、淡化）；分隔线后是两条链接行。
- **菜单项（全部，`src/ui/popup/units_popup.rs::menu_entries`）**：`units::linear_formats()` 的每一种（`Message::SetLinearFormat(code)`）；`Units…`（`Message::Command("UNITS")`）；`Convert drawing…`（`Message::Command("DWGUNITS")`）。
- **实现位置**：`src/ui/popup/units_popup.rs::menu_entries`、`format_row`、`divider`、`link`；来源 `src/modules/draw/units.rs::linear_formats`；状态栏挂载。
- **备注**：此菜单提供 LUNITS 而非 INSUNITS；插入单位在 UNITS 对话框里（源码头注）。

### 4.5 隔离对象（Isolate）状态菜单
- **功能简介**：隔离/隐藏选中对象，或结束隔离。
- **UI 入口**：`状态栏 → 隔离按钮 ▾ → 菜单`。
- **样式**：行内按钮 `button::subtle`、内边距 `[4,12]`、11px；可用项发消息并关闭，禁用项保持打开且无按下处理器。
- **菜单项（全部，`src/ui/popup/isolate_popup.rs::menu_entries`）**：`Isolate Objects`（`ISOLATEOBJECTS`，有选择可用）、`Hide Objects`（`HIDEOBJECTS`，有选择可用）、`End Isolation`（`UNISOLATEOBJECTS`，隔离中可用）。
- **实现位置**：`src/ui/popup/isolate_popup.rs::menu_entries`、`action_entry`、`action_row`；状态栏挂载（宽 160）。
- **备注**：与 1.2 空闲右键菜单的 `Isolate ▸` 子菜单功能一致。

### 4.6 选择过滤（Selection Filter）状态菜单
- **功能简介**：选择哪些实体类型可被拾取；勾选=可选，取消=排除。
- **UI 入口**：`状态栏 → 过滤按钮 ▾ → 类型列表`。
- **样式**：顶部 `Select All`/`Clear All` 小按钮 + 分隔线；每行勾选格 + 类型名（11px）；无类型时显示淡化 `No objects`。
- **菜单项（全部，`src/ui/popup/selection_filter_popup.rs::menu_entries`）**：`Select All`（`SelectionFilterSelectAll`）、`Clear All`（`SelectionFilterClearAll`），以及当前布局中出现的每个实体类型（`ToggleSelectionFilterType(name)`）。
- **实现位置**：`src/ui/popup/selection_filter_popup.rs::menu_entries`、`type_row`、`empty_row`、`header_btn`；状态栏挂载（宽 180）。
- **备注**：`Select All` 清除所有排除，`Clear All` 排除所有当前存在类型；按钮按状态禁用。

### 4.7 状态菜单框架（通用）
- **功能简介**：所有状态栏菜单共用的 MenuBar 管道与外观（宽度、圆角、阴影、关闭行为）。
- **UI 入口**：`状态栏 → 各 ▾ 按钮`。
- **样式**：菜单背景 `background.weakest`、边框 `background.neutral` 1px、圆角 3；阴影主色偏移 `(0,-2)`、模糊 6；`DrawPath::Backdrop`；`safe_bounds_margin(ROW_HEIGHT)`；`close_on_background_click(true)` 与 `close_on_background_click_global(true)`。
- **触发命令**：无（内嵌框架）。
- **实现位置**：`src/ui/statusbar/status_menu.rs::menu_bar`、`Entry::stay`、`Entry::close`。
- **备注**：每行包 `mouse_area`（`Interaction::Idle`）以保证悬停禁用行时光标不消失（#684）；菜单可在光标离开自身矩形最多一个状态栏行高后仍不关闭（#682）。

---

## 五、选择循环浮层（`src/ui/popup/cycle_popup.rs`）

### 5.1 重叠对象选择列表框
- **功能简介**：当一次点击落在两个及以上重叠对象上时，在光标处弹出列表，逐条列出对象，点击某条把它加入当前选择；点击外部取消。
- **UI 入口**：`视口 → 在重叠对象上左键 → 选择循环列表框`。
- **样式**：全画布透明点击捕获层 + 光标锚定的面板（`container::bordered_box`），宽度 = `type_w + layer_w + 42`；每行：9×9 颜色块（描边 `#6B6B6B`、圆角 2）、11px 类型名、竖分隔线、右对齐 10px 图层名（淡化）、`button::subtle` 行（`padding [3,8]`）；类型/图层列宽按最宽项钳制（`clamp_col_width`，类型 40~96、图层 28~84）。
- **触发条件**：状态栏 `SelCycle`（`selection_cycling`）开启且命中 `>= 2` 个对象（`click_hits_all`）。
- **实现位置**：`src/ui/popup/cycle_popup.rs::cycle_popup_overlay`、`item_row`、`CycleCandidate`；触发 `src/app/update/viewport.rs`（设置 `cycle_candidates`）；渲染 `src/app/view/mod.rs`。
- **备注**：悬停行高亮底层对象（`CycleHover`/`CycleHoverExit`）；点击行发 `Message::CycleSelect(handle)`；点击面板外发 `Message::CycleCancel`。

---

## 六、视口内其他弹出/浮动层（`src/app/view/mod.rs` 的 `viewport_stack`）

这些浮层随视口栈叠加，锚定在画布坐标。

### 6.1 夹点多功能弹出菜单
- **功能简介**：夹点悬停时出现的多函数菜单，选择要执行的夹点操作。
- **UI 入口**：`视口 → 悬停夹点 → 多功能弹窗`。
- **样式**：面板 `background.weak` + 边框 `background.neutral` 圆角 3，行 12px、内边距 `[3,10]`；选中行主色强底，悬停 `background.strong`。
- **触发命令/条件**：夹点悬停（`grip_popup`）。
- **实现位置**：`src/app/view/mod.rs`（夹点弹窗渲染块，`Message::GripMenuPick(idx)`）；锚点偏移 `+12,+12`。
- **备注**：与右键夹点菜单（1.3）互为不同入口，行为语义相近。

### 6.2 动态块可见性状态下拉
- **功能简介**：动态块可见性状态的弹出选择列表。
- **UI 入口**：`视口 → 动态块可见性夹点 → 下拉列表`。
- **样式**：面板宽按最长项估算（`(max_len+2)*7+24`），行含勾选列 + 名称（12px），悬停主题弱主色；`height = Fit.max(360.0)` 可滚动。
- **触发条件**：`visibility_popup`（动态块可见性夹点）。
- **实现位置**：`src/app/view/mod.rs`（`visibility_popup` 渲染块，`Message::VisibilityPick(idx)`）。
- **备注**：当前状态行显示 `✓` 标记。

### 6.3 就地单行文字编辑器（TEXT）
- **功能简介**：在插入点点位就地编辑单行文字的输入框。
- **UI 入口**：`视口 → 双击文字 / TEXT 编辑 → 就地输入框`。
- **样式**：`text_input`（宽 240、字号 13、内边距 6）外包 `background.weak` 面板（圆角 5、边框 `background.neutral`）。
- **触发命令**：`TEXT`（编辑态）；Enter 提交（`TextInlineOk`），Esc 取消。
- **实现位置**：`src/app/view/overlay.rs::text_inline_overlay`（控件 id `TEXT_INLINE_ID`）。
- **备注**：仅单行、无格式工具栏。

### 6.4 就地多行文字编辑器（MTEXT）
- **功能简介**：在插入点位置就地编辑多行文字，带格式工具栏与文本区。
- **UI 入口**：`视口 → 双击多行文字 / MTEXT 编辑 → 就地编辑器`。
- **样式**：工具栏 + 文本区叠加于插入点。
- **触发命令**：`MTEXT`（编辑态）；控件 id `MTEXT_TEXT_ID`。
- **实现位置**：`src/app/view/overlay.rs::mtext_editor_overlay` 区域（`MTEXT_TEXT_ID`）。
- **备注**：见 06 号/文字相关文档的完整子项；此处仅作浮层登记。

### 6.5 快速选择（QSelect）浮层
- **功能简介**：按类型/特性条件批量过滤选择的浮层。
- **UI 入口**：`右键 → Quick Select...` 或 `QSELECT` 命令。
- **样式**：`qselect_overlay`，含可用类型、可用特性、候选计数。
- **触发命令**：`QSELECT`；右键 `Quick Select...`（`MenuAction::QuickSelect` → `Message::QSelectOpen`）。
- **实现位置**：`src/app/view/mod.rs`（`self.qselect` 渲染，`qselect_overlay`）；上下文见 `state.available_types/properties/candidate_count`。
- **备注**：叠加在 `main_ui` 之上、模态之下（`stack![main_ui, dropdown_layer, qselect_layer, ...]`）。

### 6.6 图形打开进度浮层（Open Progress）
- **功能简介**：打开大图时显示进度指示；有恢复错误时改为错误路径。
- **UI 入口**：`打开文件时自动显示`。
- **样式**：`src/ui/window/open_progress::view`（按 `Instant` 驱动动画）。
- **触发条件**：`self.opening` 存在且无 `recovery_error`。
- **实现位置**：`src/app/view/mod.rs`（`open_progress_layer`）；`src/ui/window/open_progress.rs::view`。
- **备注**：仅在有动画需求时请求逐帧订阅（`needs_frames`）。

### 6.7 性能/诊断浮层（可选）
- **功能简介**：显示渲染性能统计的小面板（位置约 `(12,40)`）。
- **UI 入口**：`视口 → 顶部左侧性能面板（开启时）`。
- **样式**：`perf_w` 宽、内边距 6、无边框。
- **触发条件**：性能面板开启（`perf` 显示条件）。
- **实现位置**：`src/app/view/mod.rs`（`perf_w` 渲染块，`position_canvas_overlay(Point::new(12.0,40.0), ...)`）。
- **备注**：（据源码推断）仅在开发者/诊断开关下出现。

### 6.8 浮层定位辅助
- **功能简介**：把面板锚定到画布坐标或光标附近并做边界钳制。
- **UI 入口**：无（框架）。
- **样式**：`pin(opaque(panel)).position(...)`。
- **实现位置**：`src/app/view/overlay.rs::position_canvas_overlay`、`position_canvas_overlay_clamped`、`position_canvas_overlay_near_cursor`。
- **备注**：右键菜单用 `position_canvas_overlay_clamped` 使默认行中心压在指针下（`MENU_CURSOR_INSET_X`）。

---

## 七、画布浮层种类（`src/ui/overlay.rs`）

`src/ui/overlay.rs` 是「视口浮层」的集合，均以 `canvas` 全填充绘制。

### 7.1 网格浮层（Grid Overlay）
- **功能简介**：绘制主/次网格线、轴线与地平线，带几何缓存复用。
- **UI 入口**：`视口 → 网格（GRID）`。
- **样式**：`GridStyle { opacity, bg_luminance }`（默认 `opacity=18`、`bg_luminance=0.15`）；次网格 `MIN_GRID_PX=20.0`、地平线网格 `MIN_HORIZON_GRID_PX=5.0`。
- **触发条件**：网格开关开启。
- **实现位置**：`src/ui/overlay.rs::grid_overlay`、`GridCanvas`、`draw_grid`、`draw_axes`、`GridParams`、`GridGeometry`、`GridKey`、`should_reuse`、`compute_grid_step(s)`。
- **备注**：缓存键包含参数、边界与样式，任一变化即失效重绘。

### 7.2 选择/十字光标浮层（Selection Overlay）
- **功能简介**：单一浮层绘制十字光标、拾取框、选择高亮、窗口/交叉窗口、夹点、移动夹点、捕捉标记、对象捕捉追踪线、约束字形、UCS 图标、视图立方体、窗格分隔与拖放矩形等。
- **UI 入口**：`视口`（始终作为画布层）。
- **样式**：十字光标 `CROSSHAIR_SQ=7.5`、`CROSSHAIR_ARM=60.0`（`crosshair_arm_px` 依 `CURSORSIZE` 换算），拾取框依 `PICKBOX`（`pick_box_half_px`、`pick_box_aperture_px`）；窗口选择默认钴蓝 `#3370B8`、交叉选择默认翠绿 `#33B870`（`DEFAULT_WINDOW_COLOR`/`DEFAULT_CROSSING_COLOR`，`theme_selection_colors` 按主题调整）；选择填充透明度 `selection_fill_alpha` 依 `SELECTIONAREAOPACITY`。
- **触发条件**：始终存在；`suppressed` 可抑制。
- **实现位置**：`src/ui/overlay.rs::selection_overlay`、`SelectionCanvas`、`SelectionVisualOptions`、`CrosshairOptions`、`draw_grip_marker`、`draw_move_gizmo`、`draw_smooth_constraint_glyph`、`draw_tangent_constraint_glyph`、`draw_concentric_constraint_glyph`、`draw_fixed_constraint_glyph`、`draw_vertical_constraint_glyph`。
- **关键入参种类（逐类）**：`snap: Option<(Point, SnapType)>`（捕捉标记）、`snap_ext_base`/`snap_ext_base2`（延伸引导线基点）、`grips: Vec<GripMarker>`（夹点）、`control_polygon`（控制多边形预览）、`grip_clip`、`ucs_icons: Vec<UcsIconParams>`（UCS 图标）、`ost_points`/`otrack_lines`（对象捕捉追踪点/线）、`parallel_ref_marker`（平行参照标记）、`show_viewcube`（视图立方体）、`dividers: Vec<Rectangle>`（窗格分隔）、`pane_move_rect`/`pane_drop_rect`（窗格拖放预览）、`pan_mode`、`hover_locked`、`crosshair_bg`、`constraint_glyphs`/`constraint_glyph_selected`/`constraint_glyph_tooltip`/`constraint_cursor_badge`（约束字形、选择、提示与光标徽标）。
- **备注**：`is_dynamic_dimension_glyph` 识别动态尺寸锁标；`OstTrackPoint` 为捕捉追踪点屏幕坐标。

### 7.3 UCS 图标（UCS Icon）
- **功能简介**：显示可拖动的坐标系统三轴图标，可跟随原点或被选中。
- **UI 入口**：`视口 → 角落/UCS 原点`。
- **样式**：`UCS_ICON_MARGIN=50.0`、`UCS_ICON_LEN=38.0`、`UCS_ICON_TIP=7.0`、`UCS_GRIP_BOX=7.0`；悬停增亮，选中时在原点与轴端绘制可拖拽方块。
- **触发条件**：UCS 图标开启；`origin_screen` 决定跟随原点或钉在角落。
- **实现位置**：`src/ui/overlay.rs::UcsIconParams`、`ucs_icon_geometry`、`ucs_icon_hit`、`draw_ucs_icon`、`UcsIconHit`。
- **备注**：`origin_screen=None` 时钉在角落（`UCSICON ORigin` 反转）。

### 7.4 动态输入浮层（Dynamic Input）
- **功能简介**：在光标/锚点处显示当前命令提示、值输入框、跟踪提示与引导几何（极轴弧、半径线、轴增量、矩形边、垂直测量等）。
- **UI 入口**：`视口 → 动态输入（DYN，F12 开关）`。
- **样式**：`DYN_OFFSET_X=14.0`、`DYN_PAD=4.0`、`DYN_GAP=6.0`、`DYN_FONT=11.0`、`DYN_BOX_H=DYN_FONT+8`；值框颜色：活动=主色弱底/主色边、锁定=警告色、普通=`background.weak`；提示胶囊用 `background.strong` + 主色边；框可被钳制在视口内。
- **触发条件**：`dyn_input` 开启且命令产生 `DynSpec`（或旧式 `dyn_field`）。
- **实现位置**：`src/ui/overlay.rs::dynamic_input_overlay`、`DynInputCanvas`、`DynBox`、`draw_box`、`box_colors`、`draw_prompt`、`draw_tracking_hint`、`draw_guided`、`draw_row`；角色 `DynRole`（`src/command.rs`）、引导 `DynGuide`（同上）、锚点 `DynAnchor`。
- **值框角色（`DynRole`，全部）**：`X`、`Y`、`Z`、`Distance`、`Angle`、`Radius`（标签 `R`）、`Diameter`（标签 `⌀`，显示/输入为半径两倍）、`Width`（`W`）、`Height`（`H`）、`Factor`、`Count`（`#`，保留）。标签由 `DynRole::label` 决定，`value_scale` 处理直径。
- **引导几何（`DynGuide`，全部）**：`None`（无引导）、`Polar`（+X/参照线 + 角度弧）、`AxisDelta`（轴向投影腿）、`Radius`（锚点→光标线）、`RectSides`（矩形两边）、`Perp`（垂直于参照线的半轴）、`PerpDim`（带尺寸偏移与延伸线）。
- **备注**：TAB 焦点框（`active`）用更粗描边；已键入值的框 `locked` 不再跟随光标；远处的引导线会按视口裁剪以防 #406 的巨量四边形（`clip_seg`）。

---

## 八、通用模态框框架（`src/ui/modal.rs`）

### 8.1 模态框主体（`modal`）
- **功能简介**：把内容叠在主视图之上并加变暗遮罩，带居中标题栏与右上角 ✕ 关闭按钮；可拖动、可调整大小。
- **UI 入口**：`任意窗口/对话框`（如关于、图层管理器、样式编辑器、选项、绘图等）。
- **样式**：面板背景 `background.base`、边框 `background.neutral` 1px、圆角 6；标题栏高 24，标题 15px 居中；关闭按钮 ✕ 13px（危险样式 `close_style` 或中性样式 `neutral_close_style`）；内边距 10；遮罩为 `background.strongest` 55% 透明；`ModalOptions::STANDARD`（可移动、可缩放、危险关闭）与 `NOTICE`（可移动、不可缩放、中性关闭）。
- **触发命令**：由 `active_modal`（`ModalKind`）驱动；`Message::CloseModal` 关闭。
- **实现位置**：`src/ui/modal.rs::modal`、`ModalOptions::STANDARD/NOTICE`、`close_style`、`neutral_close_style`、`intrinsic`（固有尺寸测量）。
- **备注**：遮罩只挡点击、不关闭；关闭仅靠 ✕。拖动通过标题栏 `Message::ModalGrab`/`ModalDragMove`/`ModalDragRelease`；缩放通过 `Message::ModalResizeGrab` 与 `Message::ModalContentResized`；偏移用非对称内边距实现（`offset`）。

### 8.2 模态遮罩（`backdrop`）
- **功能简介**：给第三方覆盖层（如 iced_aw 取色器）加变暗且吞掉指针事件的背景，使其表现为模态。
- **UI 入口**：`选色器等第三方覆盖层下`。
- **样式**：`background.strongest` 55% 透明遮罩，`opaque` 包裹，交互 `Idle`；点击遮罩发 `on_close`。
- **实现位置**：`src/ui/modal.rs::backdrop`；用于 `src/app/view/mod.rs` 的 True Color 取色器（`Message::CloseColorPicker`）。
- **备注**：与 `modal` 不同，`backdrop` 点击会关闭。

### 8.3 未保存防护面板（`discard_guard`）
- **功能简介**：关闭有未保存改动的对话框时，主内容变暗并弹出小面板，让用户选择「丢弃并关闭」或「继续编辑」。
- **UI 入口**：`对话框 → 关闭（有未保存改动）→ 防护面板`。
- **样式**：遮罩同 8.2；面板 `container::rounded_box`，内边距 `[18,22]`，文案 “Unsaved changes will be discarded.” 13.5px；按钮 `Discard && close`（`button::danger`）与 `Keep editing`（`button::secondary`），12px、内边距 `[5,14]`。
- **触发条件**：用户在有改动时请求关闭。
- **实现位置**：`src/ui/modal.rs::discard_guard`。
- **备注**：点击变暗区域等同「继续编辑」（`on_keep`）。

### 8.4 模态种类清单（`ModalKind`）
- **功能简介**：全部以就地模态呈现的窗口/对话框。
- **UI 入口**：`菜单/命令/自动触发`。
- **实现位置**：`src/app/mod.rs::ModalKind`、标题映射 `src/app/view/modal.rs::modal_title`、内容分发 `src/app/view/modal.rs::modal_content`；窗口实现 `src/ui/window/`。
- **清单（全部，`ModalKind`）**：`About`（关于）、`Shortcuts`（键盘快捷键编辑器）、`Aliases`（命令别名）、`NamedParameters`（命名参数）、`Options`（选项，含右键模式等）、`FindReplace`（查找替换）、`PluginManager`（插件管理器；wasm 下用 `NOTICE`）、`UpdateNotice`（有更新）、`DonationPrompt`（捐赠提示）、`Layers`（图层管理器）、`LayerStateManager`（图层状态管理器）、`LayerStateEditor`（编辑图层状态）、`LayerTranslator`（图层转换器）、`DrawingUnits`（图形单位）、`BlockDefinition`（块定义）、`PdfAttach`（附着 PDF/DWF/DGN 底图）、`PointCloudAttach`（附着点云）、`PointCloudColorMap`（点云颜色映射）、`PcSection`（点云提取剖切）、`UnderlayLayers`（底图图层）、`PdfImportSettings`/`PdfImportFile`（PDF 导入设置/文件）、`XrefAttach`（附着外部参照）、`XrefHelp`（参照管理器帮助）、`WriteBlock`（写块）、`GeometricTolerance`（形位公差）、`DraftingSettings`（草图设置）、`AutoConstrainSettings`（约束设置）、`Plot`（打印）、`PrintAll`（打印全部）、`LayoutManager`（布局管理器）、`Plotstyle`（打印样式编辑器，以 Plot 为被挡父级）、`TextStyle`（文字样式管理器）、`TableStyle`（表格样式管理器）、`MlStyle`（多线样式管理器）、`MLeaderStyle`（多重引线样式管理器）、`DimStyle`（标注样式管理器）、`AssocPrompt`（默认程序选择）、`AecDropWarning`（保存警告）、`FileInUse`（无法保存，非 wasm）、`ExternalChange`（图形已更改，非 wasm）、`LayerDeleteWarning`（删除图层警告）、`Unsaved`（未保存更改）、`PointStyle`（点样式）、`AttributeEditor`（属性编辑器）、`SaveDialog`（图形另存为）、`Recovery`（恢复报告）、`MissingFonts`（缺失字体）、`RecoveryPrompt`（恢复提示）、`GpuWarning`（GPU 警告）、`ScaleManager`（比例管理器）、`AnnoObjectScale`（注释对象比例）、`Hyperlink`（超链接编辑器）、`InsertTable`（插入表格）、`DataLinkManager`（数据链接管理器）、`DataExtraction`（数据提取向导）。
- **备注**：`Plotstyle` 通过 `plotstyle_parent_plot_geometry` 把 Plot 对话框作为被挡的父层先渲染；`Intrinsic` 使对话框按内容固有尺寸起始，超出部分滚动。

---

## 九、宽菜单（`src/ui/wide_menu.rs`）

### 9.1 宽菜单容器
- **功能简介**：让组合框（combo box）的下拉弹层比输入框本身更宽，避免线型名/ASCII 预览被输入框宽度压挤。
- **UI 入口**：`特性面板 → 线型下拉`；`图层管理器 → 线型列`。
- **样式**：不改内容视觉，仅在布局阶段把弹层宽度取 `menu_width.max(bounds.width)`，高度沿用内容高度。
- **触发条件**：组合框展开。
- **实现位置**：`src/ui/wide_menu.rs::wide_menu`、`WideMenu`（`overlay` 中设置 `State::menu_layout`）。
- **备注**：调用点：`src/ui/properties.rs`（`wide_menu(combo, LINETYPE_MENU_W)`）、`src/ui/window/layers.rs`（线型列，返回 `LayerLinetypeSet`）。

---

## 十、只读字段与只读会话（`src/ui/read_only.rs`）

### 10.1 只读值字段
- **功能简介**：不可编辑但可选中复制的值输入框，用于展示只读特性（不画光标、键入无效，仍支持点击/拖动选择与 Ctrl+C/Ctrl+A）。
- **UI 入口**：`特性面板只读行`、`标注样式/文字样式/多重引线样式/块定义等窗口的只读字段`。
- **样式**：`text_input` 无 `on_input`；背景 `background.weakest`、边框 `background.neutral` 1px、圆角 2；值文字 `background.base.text` 72% 透明，占位 48%，选区主色 50%。
- **触发条件**：字段被标记只读（`PropValue::ReadOnly` 等）。
- **实现位置**：`src/ui/read_only.rs::field`、`read_only_style`；调用点如 `src/ui/properties.rs`、`src/ui/style/dimstyle.rs`、`src/ui/style/textstyle.rs`、`src/ui/style/mleaderstyle.rs`、`src/ui/window/block_definition.rs` 等。
- **备注**：字段不借用传入的 `value`，调用方可传临时 `String`。

### 10.2 只读会话（`--read-only`）
- **功能简介**：会话级只读：允许编辑但禁止所有保存，保存时在命令行报告。
- **UI 入口**：`命令行 → 保存类命令`（只读会话下）。
- **样式**：无专门浮层；以命令行错误形式呈现：“Read-only session (--read-only): saving is disabled.”。
- **触发命令**：启动参数 `--read-only`；任保存命令。
- **实现位置**：`src/app/mod.rs::read_only`；`src/app/commands/fileops.rs`（`if self.read_only { push_error(...) }`）。
- **备注**：这是提示而非浮层；任务所指「只读提示浮层」在源码中对应这两处机制（只读字段 + 只读会话错误）。

---

## 十一、节点图浮层（`src/ui/node_graph.rs`，简要）

### 11.1 节点图书签/画布浮层（NODEGRAPH）
- **功能简介**：在视口上叠加参数化节点图画布：节点库面板（对象节点 + 运算节点分类）、节点卡片、连线、拖拽、缩放平移，以及 `.ocg` 存/取；对象节点的行即该实体在特性面板的行。
- **UI 入口**：`节点图开关/命令 → 视口覆盖层（画布） + 停靠面板「Node Graph」`。
- **样式**：节点宽 `NODE_W=280.0`，头高 `HEADER_H=26.0`，段头 `SECTION_H=22.0`，行高 `ROW_H=22.0`，标签列 `LABEL_W=96.0`，端口 `PORT_W=14.0`、命中半径 `PORT_HIT=10.0`；连线为贝塞尔（`wire_path`，控制点偏移 `max(|dx|*0.5, 40)`），待连线段用主色弱色；节点头用主色弱底/弱文字，节点体 `background.base` 圆角 4；端口圆点已连=主色填充，未连=主色描边。
- **触发命令/入口**：节点图开关；`GraphMsg::Toggle`/`New`/`Save`/`Open`；停靠面板 `PanelId::NodeGraph`（标题“Node Graph”，`dock::frame`/`dock::title_bar`）。
- **实现位置**：`src/ui/node_graph.rs::Graph::view`、`panel`、`node_view`、`row_view`、`port_dot`、`Wires`、`Zoomed`、`GraphMsg`、`PaletteItem`、`ObjectKind`、`NodeSection`、`NodeRow`；缩放 `ZOOM_STEP=1.1`、步数 `ZOOM_STEPS=-15..=12`。
- **备注**：面板含搜索框与 New/Open/Save 工具栏（图标 `DOC_NEW`/`FOLDER_OPEN`/`SAVE`），分类折叠；对象节点可从节点库拖到画布或点击在左上角落位（`drop_position`）；`.ocg` 文件保存图、画布与所属图形指纹（`to_file`/`from_file`）；删除对象后节点显示 `Object deleted`。

---

## 十二、命令输入框 / 图层下拉 / 视图标签右键（据源码核查）

### 12.1 命令输入框右键
- **功能简介**：**当前源码中未发现命令输入框专用的右键上下文菜单**。命令行的历史/回显以只读文本编辑区与淡出叠层实现，而非右键菜单。
- **UI 入口**：`命令行 → 历史区`。
- **样式**：整段日志渲染为单个只读 `text_editor`，编辑被丢弃；叠层历史条目按 `COMMANDLINEFADETIME`（默认 `DEFAULT_COMMANDLINE_FADE_MS=3000`，范围 0~60000）淡出。
- **实现位置**：`src/ui/command_line.rs::CommandLine::view`、`overlay_lines_height`、`has_visible_history`、`HistoryEntry`、`EntryKind`、`HISTORY_HEIGHT_*`。
- **备注**：无法选中/右键功能在此实现中不存在；如后续需要，应作为新需求（据源码推断）。

### 12.2 图层下拉右键
- **功能简介**：**当前源码中未发现图层名称下拉的右键菜单**。图层下拉（`LayerComboGroup`）为左键展开，行内图标左键切换可见/冻结/锁定（见 02 号文档「图层下拉本身」）。
- **UI 入口**：`Ribbon → Draw 选项卡 → Layers 面板 → 图层名称条（左键展开）`。
- **实现位置**：`src/ui/ribbon/mod.rs::layer_combo_overlay`、`src/ui/ribbon/widgets.rs::render_large`。
- **备注**：应为左键交互；无右键菜单。

### 12.3 视图标签（View Tab）/ 图纸标签右键
- **功能简介**：视图标签（顶部文档标签）与状态栏布局标签的右键菜单已在 3.1 / 3.2 覆盖；除此之外没有独立的「视图标签」右键菜单。
- **UI 入口**：见 3.1、3.2。
- **实现位置**：`src/app/view/mod.rs::doc_tab_context_menu`、`src/ui/statusbar/mod.rs::layout_tab_context_menu`。
- **备注**：`Model` 标签无右键菜单。

---

## 十三、右键触发模式与打开/关闭（通用行为）

### 13.1 右键模式（SHORTCUTMENU）
- **功能简介**：决定视口右键是开菜单还是当 Enter/重复：`ShortcutMenu`（总是开菜单）、`TimeSensitive`（快速点击=Enter，按住超过阈值=开菜单）、`EnterFirst`（命令中第一次=Enter，第二次=开菜单；空闲总是开菜单）。
- **UI 入口**：`菜单 → 选项 → 用户偏好 → 右键模式`（Options → User Preferences）。
- **样式**：在 Options 模态框内以标签选择呈现。
- **触发命令**：`SHORTCUTMENU`；阈值 `right_click_hold_ms`（默认 250，钳制 100~1000）。
- **实现位置**：`src/app/settings.rs::RightClickMode`、`clamp_right_click_hold_ms`；逻辑 `src/app/update/mod.rs::ViewportRightRelease`；设置界面 `src/ui/window/options.rs`。
- **备注**：命令行为空时，右键优先按 Enter 运行键入内容（`CommandFinalize`），其余情况才按模式决定开菜单；打开菜单后会把键盘从命令行移走；`Esc`、键入命令名或坐标、左键拾取等都会让菜单让路（`intercept_context_menu_key`）。

### 13.2 打开/关闭与状态存储
- **功能简介**：右键菜单锚点与高亮状态存于每个标签页的选择状态，随标签切换独立。
- **UI 入口**：`视口右键`。
- **实现位置**：`src/scene/pick/selection_state.rs::SelectionState { context_menu, context_menu_ui }`、`ContextMenuUi { open_submenu, highlighted }`、`open_context_menu`、`close_context_menu`；打开 `src/app/update/mod.rs::ViewportRightRelease`；关闭 `src/app/update/context_menu.rs::close_context_menu`。
- **备注**：拾取任一行会先关闭菜单、恢复命令行焦点，再走与键入等价的消息路径，确保行为一致（`on_context_menu_pick`）。

---

（文档结束）
