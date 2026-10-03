# Ribbon「View」选项卡功能需求清单

本文档反推自 `src/modules/view/`（全部 `.rs`）、`src/app/commands/view.rs`、`src/app/commands/display.rs`、`src/app/commands/fileops.rs`、`src/app/commands/dim.rs`、`src/app/commands/layerprops.rs`、`src/ui/ribbon/mod.rs`、`src/ui/ribbon/widgets.rs`，以及视图相关对话框 `src/ui/window/plot.rs`、`src/ui/window/print_all.rs`、`src/ui/window/layout_manager.rs`、`src/ui/window/options.rs`。Ribbon 面板与工具的权威布局入口是 `src/modules/view/mod.rs::ViewModule::ribbon_groups()`。

面板组权威顺序（`src/modules/view/mod.rs`）：Viewport Tools、Navigate、Model Viewports、Visual Style、Projection、Preset、Palettes、Interface、Plot。

> 说明：View 选项卡中的多数工具为**一次性动作**（点击后立即执行、无交互命令驻留），其按钮高亮由 `src/app/update/dialog.rs::on_ribbon_tool_click` 在命令结束且无对话框/平移/环绕/动态缩放状态时立即清除（源码注释 #355）。少数为**状态开关**（toggle），高亮由 `src/ui/ribbon/widgets.rs::is_active_tool` 依据实时状态决定。下文逐条标注。

---

## 一、Viewport Tools 面板（Ribbon → View 选项卡 → Viewport Tools 面板）

面板布局（`src/modules/view/mod.rs`）：`LargeTool(UCS Icon)`、`LargeTool(ViewCube)`，两个大按钮。

### UCS 图标（UCS Icon）
- **功能简介**：切换模型空间 UCS 图标的显示/隐藏，并控制图标是否绘制在 UCS 原点。
- **UI 入口**：`Ribbon → View 选项卡 → Viewport Tools 面板 → UCS Icon（LargeTool）`。
- **样式**：`LargeTool`（大按钮，图标 `ucs_icon.svg`，标签 `UCS\nIcon`）。
- **触发命令**：`UCSICON`。子选项 `ON` / `OFF` / `NOORIGIN` / `ORIGIN`（`src/app/commands/display.rs:344`）。
- **实现位置**：`src/modules/view/ucs_icon.rs::tool()`（`id "UCSICON"`，`ModuleEvent::Command("UCSICON")`）；分发与状态写回 `src/app/commands/display.rs` 的 `"UCSICON"` 与 `"UCSICON "` 分支（`self.show_ucs_icon`、`self.ucs_icon_at_origin`、`ribbon.set_ucs_icon`，并遍历所有 `Viewport` 写回 `vp.status.ucs_icon_visible` / `ucs_icon_at_origin`）。
- **高亮规则**：**状态开关**。`is_active_tool` 中 `"UCSICON" => state.show_ucs_icon`（`src/ui/ribbon/widgets.rs:380`），状态由 `Ribbon::show_ucs_icon` 提供（`src/ui/ribbon/mod.rs:57`、`toggle_state`）。裸 `UCSICON` 为切换可见性。
- **备注**：命令还提供 `NOORIGIN`/`ORIGIN` 决定图标画在角落还是原点（据源码）。相关选项页 `src/ui/window/options.rs::AppPrefs.show_ucs_icon / ucs_icon_at_origin`（`UCSICON`、`UCSICON ORigin`）。

### ViewCube（View Cube）
- **功能简介**：切换屏幕导航立方体（ViewCube）的显示/隐藏。
- **UI 入口**：`Ribbon → View 选项卡 → Viewport Tools 面板 → View Cube（LargeTool）`。
- **样式**：`LargeTool`（图标 `viewcube.svg`，标签 `View\nCube`）。
- **触发命令**：`NAVVCUBE`。
- **实现位置**：`src/modules/view/viewcube.rs::tool()`（`id "NAVVCUBE"`）；`src/app/commands/display.rs:410` 分发 `Message::ToggleViewCube`；处理 `src/app/update/mod.rs:4196`（`self.show_viewcube ^= true; ribbon.set_viewcube`）。
- **高亮规则**：**状态开关**。`"NAVVCUBE" => state.show_viewcube`（`src/ui/ribbon/widgets.rs:379`、`src/ui/ribbon/mod.rs:55`）。
- **备注**：选项页“3D Modeling → Display Tools”含 `NAVVCUBE: show the navigation cube` 复选框（`src/ui/window/options.rs::AppPrefs.show_viewcube`，`Message::ShowViewCubeChanged`）。

---

## 二、Navigate 面板（Ribbon → View 选项卡 → Navigate 面板）

面板布局（`src/modules/view/mod.rs`）：`LargeTool(Zoom Extents)`，随后五个小 `Tool`：Zoom Window、Zoom In、Zoom Out、Pan、3D Orbit。

### 范围缩放（Zoom Extents）
- **功能简介**：将当前视图缩放至显示全部图形范围。
- **UI 入口**：`Ribbon → View 选项卡 → Navigate 面板 → Zoom Extents（LargeTool）`。
- **样式**：`LargeTool`（图标 `zoom_ext.svg`，标签 `Zoom\nExtents`）。
- **触发命令**：`ZOOM EXTENTS`（别名 `ZOOM E`、`ZE`、`ZOOMEXTENTS`）。
- **实现位置**：`src/modules/view/zoom_ext.rs::tool()`（`id "ZOOM_EXT"`）；`src/app/commands/dim.rs:835` 分支（`remember_current_view()` + `fit_all()`）。
- **备注**：另有关联命令 `ZOOM EXTENTS ALL`（`ZEA`，缩放所有模型视口，`dim.rs:829`）；右键上下文菜单也有 “Zoom Extents”（`src/ui/popup/context_menu.rs:699`）。

### 窗口缩放（Zoom Window）
- **功能简介**：用两个角点框选一个区域，将其放大到填满视口。
- **UI 入口**：`Ribbon → View 选项卡 → Navigate 面板 → Zoom Window（Tool）`。
- **样式**：`Tool`（1 行小图标 `zoom_window.svg`，标签 “Zoom Window”）。
- **触发命令**：`ZOOM WINDOW`（`ZOOM`、`ZW`），命令名 `ZOOM WINDOW`。
- **实现位置**：`src/modules/view/zoom_window.rs::tool()`（`id "ZOOM_WINDOW"`）与 `ZoomWindowCommand`；`src/app/commands/dim.rs:938` 分支启动交互命令；悬停时用青色矩形预览（`zoom_window.rs::on_mouse_move`）。
- **备注**：命令提示与选项 `options()`：`Window(W)`、`Extents(E)`、`Previous(P)`、`Object(O)`、`All(A)`、`Dynamic(D)`、`Extents All(EA)`、`In(I)`、`Out(OUT)`、`Scale(S)`（`zoom_window.rs:52`）。两角点使用自由点拾取（`window_corner_pick = true`，避开 Ortho/Polar，#363/#291）。

### 放大（Zoom In）
- **功能简介**：以固定倍率放大当前视图。
- **UI 入口**：`Ribbon → View 选项卡 → Navigate 面板 → Zoom In（Tool）`。
- **样式**：`Tool`（图标 `zoom_in.svg`，标签 “Zoom In”）。
- **触发命令**：`ZOOM IN`（`ZOOM I`、`ZI`）。
- **实现位置**：`src/modules/view/zoom_in.rs::tool()`（`id "ZOOM_IN"`）；`src/app/commands/dim.rs:841` 分支（`zoom_camera(1.0/1.5)`）。

### 缩小（Zoom Out）
- **功能简介**：以固定倍率缩小当前视图。
- **UI 入口**：`Ribbon → View 选项卡 → Navigate 面板 → Zoom Out（Tool）`。
- **样式**：`Tool`（图标 `zoom_out.svg`，标签 “Zoom Out”）。
- **触发命令**：`ZOOM OUT`（`ZO`）。
- **实现位置**：`src/modules/view/zoom_out.rs::tool()`（`id "ZOOM_OUT"`）；`src/app/commands/dim.rs:847` 分支（`zoom_camera(1.5)`）。

### 平移（Pan）
- **功能简介**：进入交互式平移模式，按住左键拖动视图，Esc 退出。
- **UI 入口**：`Ribbon → View 选项卡 → Navigate 面板 → Pan（Tool）`。
- **样式**：`Tool`（图标 `pan.svg`，标签 “Pan”）。
- **触发命令**：`PAN`。
- **实现位置**：`src/modules/view/pan.rs::tool()`（`id "PAN"`）；`src/app/commands/display.rs:8` 分支（`pan_mode = true`）。
- **备注**：`pan_mode` 有效期间 Ribbon 按钮高亮由 `on_ribbon_tool_click` 保留（`active_cmd.is_none()` 但 `pan_mode == true` 时不清除，`src/app/update/dialog.rs:133-140`）。

### 三维环绕（3D Orbit）
- **功能简介**：进入三维动态观察模式，可绕场景自由旋转视角。
- **UI 入口**：`Ribbon → View 选项卡 → Navigate 面板 → 3D Orbit（Tool）`。
- **样式**：`Tool`（图标 `orbit.svg`，标签 “3D Orbit”）。
- **触发命令**：`3DORBIT`。
- **实现位置**：`src/modules/view/orbit.rs::tool()`（`id "3DORBIT"`）；`src/app/commands/inquiry.rs:82` 分支（`orbit_mode = true`）。
- **备注**：`orbit_mode` 有效期间按钮高亮保留（`src/app/update/dialog.rs:136`）。

---

## 三、Model Viewports 面板（Ribbon → View 选项卡 → Model Viewports 面板）

面板布局（`src/modules/view/mod.rs`）：`LargeTool(VPORTS)`，随后三个小 `Tool`：Named、Join、Restore。所有项最终都走 `VPORTS` 命令；Model 空间使用 `iced::widget::pane_grid` 拆分平铺视口，图纸空间创建 `Viewport` 实体（`src/app/commands/view.rs:561`）。

### 视口配置（Viewport Configuration）
- **功能简介**：在模型空间把视图拆分为预设布局（单视口 / 2 横 / 2 竖 / 4 格），或在图纸空间创建对应的视口实体。
- **UI 入口**：`Ribbon → View 选项卡 → Model Viewports 面板 → Viewport Configuration（LargeTool）`。
- **样式**：`LargeTool`（图标 `vports_config.svg`，标签 `Viewport\nConfiguration`）。
- **触发命令**：`VPORTS`。子项 `SINGLE`/`SI`/`1`、`2H`/`2`、`2V`、`4`。
- **实现位置**：`src/modules/view/vports_config.rs::tool()`（`id "VPORTS"`）；分发 `src/app/commands/view.rs:561` 的 `"VPORTS" | "VPORTS "` 分支（模型空间 `scene.set_model_panes`；图纸空间删除旧用户视口并创建 2H/2V/4/SINGLE 预设矩形，`auto_fit_viewport`，重排 `vp.id`）。
- **备注**：模型空间裸 `VPORTS` 设置 `awaiting_vports = true` 并在命令行提示 `Configuration [SIngle/2H/2V/4]:`（`view.rs:567`）。图纸空间裸 `VPORTS` 列出当前布局所有视口（编号、尺寸、位置、比例、On/Locked 状态，`view.rs:607`）。图层锁定时报错 “unlock existing viewport layers first”。

### 命名视口（Named）
- **功能简介**：打开/进入视口配置的命名入口（当前与 Viewport Configuration 共用 `VPORTS` 命令）。
- **UI 入口**：`Ribbon → View 选项卡 → Model Viewports 面板 → Named（Tool）`。
- **样式**：`Tool`（图标 `vports_named.svg`，标签 “Named”）。
- **触发命令**：`VPORTS`（与 Viewport Configuration 同一命令）。
- **实现位置**：`src/modules/view/vports_named.rs::tool()`（`id "VPORTS_NAMED"`，`ModuleEvent::Command("VPORTS")`）。
- **备注**：源码未见独立的命名保存/恢复实现，此项与 `VPORTS` 共用入口（据源码推断为占位/别名）。

### 合并视口（Join）
- **功能简介**：把平铺视口合并为单视口。
- **UI 入口**：`Ribbon → View 选项卡 → Model Viewports 面板 → Join（Tool）`。
- **样式**：`Tool`（图标 `vports_join.svg`，标签 “Join”）。
- **触发命令**：`VPJOIN`（内部委托 `VPORTS SINGLE`，`src/app/commands/view.rs:557`）。
- **实现位置**：`src/modules/view/vports_join.rs::tool()`（`id "VPJOIN"`）；分发 `src/app/commands/view.rs:557`（`"VPJOIN" => dispatch_view("VPORTS SINGLE")`）。
- **备注**：在模型空间即 `pane_grid` 单窗格；图纸空间即删除旧用户视口并创建一个整页视口。

### 恢复视口（Restore）
- **功能简介**：恢复视口配置的入口（当前与 Viewport Configuration 共用 `VPORTS` 命令）。
- **UI 入口**：`Ribbon → View 选项卡 → Model Viewports 面板 → Restore（Tool）`。
- **样式**：`Tool`（图标 `vports_restore.svg`，标签 “Restore”）。
- **触发命令**：`VPORTS`。
- **实现位置**：`src/modules/view/vports_restore.rs::tool()`（`id "VPORTS_RESTORE"`，`ModuleEvent::Command("VPORTS")`）。
- **备注**：源码未见独立恢复实现，与 `VPORTS` 共用入口（据源码推断为占位/别名）。

---

## 四、Visual Style 面板（Ribbon → View 选项卡 → Visual Style 面板）

面板布局（`src/modules/view/mod.rs`）：单个 `LargeDropdown { id: "VISUAL_STYLE", label: "Visual\nStyle", icon: VISUAL_STYLES[0].icon, items: VISUAL_STYLES, default: "VISUALSTYLES WIREFRAME2D" }`。

### 视觉样式下拉（Visual Style）
- **功能简介**：把活动视口切换到七种视觉样式之一（线框、消隐、着色，各含带边线变体）。
- **UI 入口**：`Ribbon → View 选项卡 → Visual Style 面板 → Visual Style（LargeDropdown，默认 VISUALSTYLES WIREFRAME2D）`。
- **样式**：`LargeDropdown`（大按钮带 ▾；默认图标 `wireframe.svg`，标签 `Visual\nStyle`；下拉行含勾选格、20px 图标、11px 标签，`src/ui/ribbon/mod.rs::dropdown_overlay`）。
- **触发命令**：默认 `VISUALSTYLES WIREFRAME2D`。下拉子项（`src/modules/view/visual_style.rs::VISUAL_STYLES`，全部）：
  - `VISUALSTYLES WIREFRAME2D` — Wireframe 2D（图标 `wireframe.svg`）
  - `VISUALSTYLES WIREFRAME3D` — Wireframe 3D（图标 `wireframe.svg`）
  - `VISUALSTYLES HIDDENLINE` — Hidden Line（图标 `hidden.svg`）
  - `VISUALSTYLES FLATSHADED` — Flat Shaded（图标 `solid.svg`）
  - `VISUALSTYLES GOURAUDSHADED` — Gouraud Shaded（图标 `solid.svg`）
  - `VISUALSTYLES FLATSHADEDWITHEDGES` — Flat Shaded + Edges（图标 `solid.svg`）
  - `VISUALSTYLES GOURAUDSHADEDWITHEDGES` — Gouraud Shaded + Edges（图标 `solid.svg`）
- **实现位置**：`src/modules/view/visual_style.rs`（`VisualStyle`、`VISUAL_STYLES`、`keyword()`、`mode_for_keyword()`、`label_for()`、`keyword_prompt()`）；`src/app/commands/view.rs:1250` 的 `"VISUALSTYLES "` 分支（`Message::SetRenderMode(mode)`）；处理 `src/app/update/mod.rs:1762`。
- **备注**：下拉选择走 `Message::DropdownSelectItem` → `ribbon.select_dropdown_item` → `dispatch_command(cmd)`（`src/app/update/mod.rs:5801`），并记住 `last_cmd`，故按钮面显示上次样式图标/标签。相关命令别名：`VS`、`VSCURRENT`、`SHADEMODE`、`VISUALSTYLES`（打开交互选单，`src/app/commands/fileops.rs:113`），`HIDE` 直接切到 Hidden Line（`src/app/commands/view.rs:1242`）。选项页渲染模式选择器也共用同一列表（`src/app/view/controls.rs:182`、`src/app/update/dialog.rs:156`）。源码注释指出曾把 “Shaded” 错挂到 2D `SOLID` 绘图命令、`HIDDEN` 无对应命令，现已统一为七种（`visual_style.rs` 头注 #621）。

---

## 五、Projection 面板（Ribbon → View 选项卡 → Projection 面板）

面板布局（`src/modules/view/mod.rs`）：`LargeTool(Ortho)`、`LargeTool(Persp)`，互为状态开关。

### 正交投影（Ortho）
- **功能简介**：把相机设为正交（平行）投影。
- **UI 入口**：`Ribbon → View 选项卡 → Projection 面板 → Ortho（LargeTool）`。
- **样式**：`LargeTool`（图标 `ortho.svg`，标签 “Ortho”）。
- **触发命令**：`PARALLEL`（按钮事件为 `PARALLEL`，以便把键入的 `ORTHO` 留给正交光标约束绘图辅助，`ortho.rs` 头注）。
- **实现位置**：`src/modules/view/ortho.rs::tool()`（`id "ORTHO"`，`ModuleEvent::Command("PARALLEL")`）；`src/app/commands/fileops.rs:337` 分发 `Message::SetProjection(true)`；处理 `src/app/update/mod.rs:1787`（`Projection::Orthographic`、`ribbon.set_ortho(true)`）。
- **高亮规则**：**状态开关**。`"ORTHO" => state.ortho_mode`（`src/ui/ribbon/widgets.rs:377`、`src/ui/ribbon/mod.rs:53`）。
- **备注**：`ORTHO` 命令本身是正交光标约束（`Message::ToggleOrtho`，`fileops.rs:336`），与投影无关；两者刻意分离（源码注释）。

### 透视投影（Persp）
- **功能简介**：把相机设为透视投影。
- **UI 入口**：`Ribbon → View 选项卡 → Projection 面板 → Persp（LargeTool）`。
- **样式**：`LargeTool`（图标 `persp.svg`，标签 “Persp”）。
- **触发命令**：`PERSP`。
- **实现位置**：`src/modules/view/persp.rs::tool()`（`id "PERSP"`）；`src/app/commands/fileops.rs:338` 分发 `Message::SetProjection(false)`；处理同 `SetProjection` 分支（`Projection::Perspective`）。
- **高亮规则**：**状态开关**。`"PERSP" => !state.ortho_mode`（`src/ui/ribbon/widgets.rs:378`）；即 Ortho 与 Persp 互斥高亮。

---

## 六、Preset 面板（Ribbon → View 选项卡 → Preset 面板）

面板布局（`src/modules/view/mod.rs`）：四个小 `Tool`：Top、Front、Right、Iso。

### 俯视图（Top）
- **功能简介**：将当前视图切换到顶视图（俯视）。
- **UI 入口**：`Ribbon → View 选项卡 → Preset 面板 → Top（Tool）`。
- **样式**：`Tool`（图标 `view_top.svg`，标签 “Top”）。
- **触发命令**：`VIEW TOP`。
- **实现位置**：`src/modules/view/view_top.rs::tool()`（`id "VIEW_TOP"`，`ModuleEvent::Command("VIEW TOP")`）。
- **备注**：`VIEW ` 前缀分支主要处理命名视图的 `LIST/SAVE/RESTORE/DELETE`（`src/app/commands/layerprops.rs:781`），预设方向由相机命令路径实现。

### 前视图（Front）
- **功能简介**：将当前视图切换到前视图。
- **UI 入口**：`Ribbon → View 选项卡 → Preset 面板 → Front（Tool）`。
- **样式**：`Tool`（图标 `view_front.svg`，标签 “Front”）。
- **触发命令**：`VIEW FRONT`。
- **实现位置**：`src/modules/view/view_front.rs::tool()`（`id "VIEW_FRONT"`）。

### 右视图（Right）
- **功能简介**：将当前视图切换到右视图。
- **UI 入口**：`Ribbon → View 选项卡 → Preset 面板 → Right（Tool）`。
- **样式**：`Tool`（图标 `view_right.svg`，标签 “Right”）。
- **触发命令**：`VIEW RIGHT`。
- **实现位置**：`src/modules/view/view_right.rs::tool()`（`id "VIEW_RIGHT"`）。

### 等轴测视图（Iso）
- **功能简介**：将当前视图切换到西南等轴测（Isometric）视图。
- **UI 入口**：`Ribbon → View 选项卡 → Preset 面板 → Iso（Tool）`。
- **样式**：`Tool`（图标 `view_iso.svg`，标签 “Iso”）。
- **触发命令**：`VIEW ISO`。
- **实现位置**：`src/modules/view/view_iso.rs::tool()`（`id "VIEW_ISO"`）。

---

## 七、Palettes 面板（Ribbon → View 选项卡 → Palettes 面板）

面板布局（`src/modules/view/mod.rs`）：三个 `LargeTool`：Tool Palettes、Properties、Sheet Set Manager。

### 工具选项板（Tool Palettes）
- **功能简介**：工具选项板入口。
- **UI 入口**：`Ribbon → View 选项卡 → Palettes 面板 → Tool Palettes（LargeTool）`。
- **样式**：`LargeTool`（图标 `tool_palettes.svg`，标签 `Tool\nPalettes`）。
- **触发命令**：`TOOLPALETTES`。
- **实现位置**：`src/modules/view/tool_palettes.rs::tool()`（`id "TOOLPALETTES"`）；分发 `src/app/commands/display.rs:585`。
- **备注**：**尚未实现** —— 分发仅向命令行输出 “TOOLPALETTES: Tool Palettes not yet implemented.”（`display.rs:587`）。

### 特性面板（Properties）
- **功能简介**：切换特性（Properties）停靠面板的显示/隐藏。
- **UI 入口**：`Ribbon → View 选项卡 → Palettes 面板 → Properties（LargeTool）`。
- **样式**：`LargeTool`（图标 `properties.svg`，标签 “Properties”）。
- **触发命令**：`PROPERTIES`（别名 `PROPS`）。
- **实现位置**：`src/modules/view/properties_palette.rs::tool()`（`id "PROPERTIES"`）；`src/app/commands/display.rs:471` 分发 `Message::ToggleProperties`；处理 `src/app/update/mod.rs:4201`（`show_properties ^= true`、`ribbon.set_properties`）。右键菜单 `MenuAction::Properties` 亦触发（`src/app/update/context_menu.rs:154`）。
- **高亮规则**：**状态开关**。`"PROPERTIES" => state.show_properties`（`src/ui/ribbon/widgets.rs:381`、`src/ui/ribbon/mod.rs:59`）。

### 图纸集管理器（Sheet Set Manager）
- **功能简介**：图纸集管理器（Sheet Set Manager）入口。
- **UI 入口**：`Ribbon → View 选项卡 → Palettes 面板 → Sheet Set Manager（LargeTool）`。
- **样式**：`LargeTool`（图标 `sheetset.svg`，标签 `Sheet Set\nManager`）。
- **触发命令**：`SHEETSET`。
- **实现位置**：`src/modules/view/sheetset.rs::tool()`（`id "SHEETSET"`）；分发 `src/app/commands/display.rs:591`。
- **备注**：**尚未实现** —— 分发仅输出 “SHEETSET: Sheet Set Manager not yet implemented.”（`display.rs:593`）。

---

## 八、Interface 面板（Ribbon → View 选项卡 → Interface 面板）

面板布局（`src/modules/view/mod.rs`）：`LargeTool(File Tabs)`、`LargeTool(Layout Tabs)`，随后三个小 `Tool`：Tile Horizontally、Tile Vertically、Cascade。Tile 系列委托给 `VPORTS` 预设（`src/app/commands/view.rs:555-558`）。

### 文件选项卡（File Tabs）
- **功能简介**：切换顶部文件（图形）选项卡栏的显示/隐藏。
- **UI 入口**：`Ribbon → View 选项卡 → Interface 面板 → File Tabs（LargeTool）`。
- **样式**：`LargeTool`（图标 `file_tabs.svg`，标签 `File\nTabs`）。
- **触发命令**：`FILETAB`。
- **实现位置**：`src/modules/view/file_tabs.rs::tool()`（`id "FILETAB"`）；`src/app/commands/display.rs:476` 分发 `Message::ToggleFileTabs`；处理 `src/app/update/mod.rs:4206`（`show_file_tabs ^= true`、`ribbon.set_file_tabs`）。
- **高亮规则**：**状态开关**。`"FILETAB" => state.show_file_tabs`（`src/ui/ribbon/widgets.rs:383`、`src/ui/ribbon/mod.rs:61`）。

### 布局选项卡（Layout Tabs）
- **功能简介**：切换底部布局（图纸空间）选项卡栏的显示/隐藏。
- **UI 入口**：`Ribbon → View 选项卡 → Interface 面板 → Layout Tabs（LargeTool）`。
- **样式**：`LargeTool`（图标 `layout_tabs.svg`，标签 `Layout\nTabs`）。
- **触发命令**：`LAYOUTTAB`。
- **实现位置**：`src/modules/view/layout_tabs.rs::tool()`（`id "LAYOUTTAB"`）；`src/app/commands/display.rs:481` 分发 `Message::ToggleLayoutTabs`；处理 `src/app/update/mod.rs:4211`（`show_layout_tabs ^= true`、`ribbon.set_layout_tabs`）。
- **高亮规则**：**状态开关**。`"LAYOUTTAB" => state.show_layout_tabs`（`src/ui/ribbon/widgets.rs:384`、`src/ui/ribbon/mod.rs:63`）。

### 水平平铺（Tile Horizontally）
- **功能简介**：把模型视口横向平铺为左右两个（2H）。
- **UI 入口**：`Ribbon → View 选项卡 → Interface 面板 → Tile Horiz.（Tool）`。
- **样式**：`Tool`（图标 `tile_horiz.svg`，标签 `Tile\nHoriz.`）。
- **触发命令**：`HORIZONTAL`（委托 `VPORTS 2H`）。
- **实现位置**：`src/modules/view/tile_horiz.rs::tool()`（`id "HORIZONTAL"`）；`src/app/commands/view.rs:555`（`"HORIZONTAL" => dispatch_view("VPORTS 2H")`）。

### 垂直平铺（Tile Vertically）
- **功能简介**：把模型视口纵向平铺为上下两个（2V）。
- **UI 入口**：`Ribbon → View 选项卡 → Interface 面板 → Tile Vert.（Tool）`。
- **样式**：`Tool`（图标 `tile_vert.svg`，标签 `Tile\nVert.`）。
- **触发命令**：`VERTICAL`（委托 `VPORTS 2V`）。
- **实现位置**：`src/modules/view/tile_vert.rs::tool()`（`id "VERTICAL"`）；`src/app/commands/view.rs:556`（`"VERTICAL" => dispatch_view("VPORTS 2V")`）。

### 层叠（Cascade）
- **功能简介**：把模型视口排成 2×2 四格（4）。
- **UI 入口**：`Ribbon → View 选项卡 → Interface 面板 → Cascade（Tool）`。
- **样式**：`Tool`（图标 `cascade.svg`，标签 “Cascade”）。
- **触发命令**：`CASCADE`（委托 `VPORTS 4`）。
- **实现位置**：`src/modules/view/cascade.rs::tool()`（`id "CASCADE"`）；`src/app/commands/view.rs:558`（`"CASCADE" => dispatch_view("VPORTS 4")`）。
- **备注**：尽管名为 Cascade（层叠），源码将其映射为四分格 `VPORTS 4`（`view.rs:558`，据源码推断）。

---

## 九、Plot 面板（Ribbon → View 选项卡 → Plot 面板）

面板布局（`src/modules/view/mod.rs`）：单个小 `Tool` “Page Setup”。源码注释说明：模型空间没有图纸空间侧边工具栏，故 Page Setup（为 `PLOTWINDOW` 选择格式/方向/窗口）需在此提供入口。

### 页面设置（Page Setup）
- **功能简介**：打开统一的打印/页面设置对话框（Plot / Page Setup），配置打印机、纸张、比例、打印样式与输出选项。
- **UI 入口**：`Ribbon → View 选项卡 → Plot 面板 → Page Setup（Tool）`。
- **样式**：`Tool`（图标 `pagesetup.svg`，标签 “Page Setup”）。
- **触发命令**：`PAGESETUP`（内联 `ToolDef`，`ModuleEvent::Command("PAGESETUP")`）。
- **实现位置**：`src/modules/view/mod.rs`（内联 `ToolDef`）；`src/app/commands/display.rs:1107` 分发 `Message::PlotDialogOpen`（注释：PAGESETUP 折叠进统一打印对话框）；对话框 `src/ui/window/plot.rs::view_window`；打开处理 `src/app/update/mod.rs:9565`（`on_plot_dialog_open`）。
- **备注**：`PLOT` / `PRINT` 打开同一对话框（`display.rs:866`）。对话框内容见下文“视图相关对话框”。

---

## 十、视图相关对话框

以下为 View 选项卡相关工具的对话框/窗口入口与其内部功能。

### Plot / Page Setup 对话框（打印与页面设置）
- **功能简介**：一个完整的打印设置界面（在画布内渲染的模态 Plan B），打包打印机、纸张、比例、偏移、打印样式、质量与输出选项；提交时把当前布局送到系统打印机或写成 PDF。
- **UI 入口**：`Ribbon → View 选项卡 → Plot 面板 → Page Setup`；或 `命令输入框 → PLOT / PRINT / PAGESETUP`；或 `选项 → Plotting → “Plot and Page Setup…”`（`src/ui/window/options.rs:385`，`Message::PlotDialogOpen`）。
- **样式**：`src/ui/window/plot.rs::view_window`。左侧 160px 宽的命名页面设置侧栏（“Page setups” 列表 + New/Copy/Delete/Import… 工具栏），中部两列 `left`/`right` 设置面板，底部工具栏含 Set current / Preview / 主操作按钮（`Print` 或 `Export PDF` 或 `Apply`），`btn(true)` 为主色按钮。字段/下拉复用 `src/ui/style/form.rs`（`labeled_pick_list`、`labeled_field`、`section_label`、`button_style`）。
- **触发命令**：`PLOT` / `PRINT` / `PAGESETUP` / `EXPORT` / `EXPORTPDF`（`src/app/commands/display.rs:866`、`:1107`）。
- **实现位置**：`src/ui/window/plot.rs`（`PlotDialogState`、`PlotDlgMsg`、`PlotFlag`、`plot.rs::view_window`、`custom_paper_editor`、`printer_options_editor`、`page_setup_import_chooser`、`custom_scale_row`、`choice` 系列辅助）。
- **对话框内分区与子项**（全部来自 `plot.rs::view_window`）：
  - **Page setups 侧栏**：命名页面设置列表、`New`/`Copy`/`Delete`/`Import…`、`Set current`。
  - **Printer / plotter**：打印机下拉（`OUT_DEFAULT = "System default printer"`、发现的打印机、`OUT_PDF = "Save to PDF file…"`）、`Copies`、`Properties…`（驱动选项内联编辑器：每选项一行，`Reset`/`Cancel`/`Apply`）、`Print in background` 相关。
  - **Paper size**：打印机报告的纸张（或目录纸张）、用户自定义纸张（`Custom…`、`Remove`；内联编辑器含 Name / Units / Width / Height / 可打印边距 Left/Bottom/Right/Top，`Add`/`Cancel`）。
  - **Orientation**：`Portrait`/`Landscape`、`Plot upside-down`。
  - **Plot area**：`What to plot` = `Extents`/`Limits`/`Display`/`Window`（图纸空间含 `Layout`，另可含 `View: <name>` 命名视图）；`Window` 时显示 `Pick…`（`PlotDlgMsg::PickWindow`）；`Plot offset` X/Y；`Center the plot`。
  - **Plot scale**：`Fit to paper`、`Scale` 下拉、`Units`（Millimeters/Inches）、`Custom`（`paper = drawing` 两输入框）、`Scale lineweights`。
  - **Plot style table (pen assignments)**：`Table` 下拉（`STYLE_NONE = "<none>"` + CTB 文件）、`Plot with plot styles`、`Display plot styles`、`Load…`、`Edit…`（打开 `Message::PlotStylePanelOpen`）。
  - **Shaded viewport options**：`Shade plot` = `As displayed`/`2D Wireframe`/`3D Wireframe`/`Hidden Line`/`Flat Shaded`/`Gouraud Shaded`/`Flat Shaded + Edges`/`Gouraud Shaded + Edges`；`Quality` = `Low`/`Normal`/`High`。
  - **Plot options**：`Plot in background`（`BACKGROUNDPLOT` bit 1）、`Object lineweights`、`Plot transparency`、`Hide paperspace objects`（仅图纸空间）、`Paper space last`（仅图纸空间）、`Merge overlapping lines`、`Plot stamp`、`Save changes to layout`（仅图纸空间）。
  - **底部工具栏**：`Set current`、`Preview`、主操作（`Print`/`Export PDF`/`Apply`）。
- **备注**：`PlotDialogState` 持久化到配置的 “plot” 段（`#[serde(skip)]` 标记运行时字段）；`<none>`/`<previous>` 为列表顶部哨兵项（`SETUP_NONE`/`SETUP_PREV`）。`PSETUPIN` 语义由 `PageSetupImportDraft` + `Import(PageSetupImportMsg)` 承担。选项页开关 “Show the page setup for new layouts” 对应 `page_setup_on_new_layout`（`options.rs:394`，`Message::PageSetupOnNewLayoutChanged`）。

### 全部打印对话框（Print All / 批量打印）
- **功能简介**：选择要输出的图纸布局，每页按 Layout 模式打印，或输出为 PDF。
- **UI 入口**：`命令输入框 → PRINTALL`（`Message::PrintAllOpen`，`src/app/commands/display.rs:869`）；由 Print All 流程打开。
- **样式**：`src/ui/window/print_all.rs::view_window`。复选框列表（`checkbox` size 16、文字 12）、`Select all`/`Select none` 按钮、`Options…`、`PDF`（secondary）/`Print`（primary）按钮；无可用布局时显示 “No paper layouts are available.”。
- **触发命令**：`PRINTALL`（`BACKGROUNDPLOT` bit 2 影响后台批量打印，见 `plot.rs` 的 `background_publish`）。
- **实现位置**：`src/ui/window/print_all.rs::view_window`（`Message::PrintAllToggle`、`PrintAllSelectAll`、`PrintAllSelectNone`、`PrintAllOptions`、`PrintAllPdf`、`PrintAllPrint`）。
- **备注**：`Options…` 打开 `plot.rs` 对话框的 `print_all_options` 模式（顶部操作按钮变 `Apply`，且禁用 New/Copy/Delete/Import 与命名区域选择）。

### 布局管理器（Layout Manager）
- **功能简介**：以整窗界面管理布局：新建、删除、左右移动、设为当前、选择与重命名布局，显示模型/图纸空间详情。
- **UI 入口**：`命令输入框 → LAYOUTMANAGER / LAYOUTPANEL`（`Message::LayoutManagerOpen`，`src/app/commands/view.rs:519`）；另有布局标签栏相关入口。
- **样式**：`src/ui/window/layout_manager.rs::view_window`（整窗）。顶部工具栏（`New Layout`、危险色的 `Delete`、`Move Left`/`Move Right`、主色 `Set Current`）；左 220px 布局列表（当前布局带左箭头，选中项主色）；右侧详情（`Model Space`/`Paper Space Layout`、`Name:`、`Status:` Active/Inactive、`Rename` 输入框 + `OK`）；`btn_s`/`list_item` 样式。
- **触发命令**：`LAYOUTMANAGER`、`LAYOUTPANEL`。
- **实现位置**：`src/app/commands/view.rs:519`；`src/ui/window/layout_manager.rs`；消息 `Message::LayoutManagerNew/Delete/MoveLeft/MoveRight/SetCurrent/Select/RenameBuf/RenameCommit`（打开处理 `src/app/update/mod.rs:7368`）。
- **备注**：无图形打开时提示 “Open or create a drawing to manage layouts.”（`update/mod.rs:7371`）。

### 选项对话框（Options）中的视图相关项
- **功能简介**：应用偏好设置，其中多页含视图/显示相关控件。
- **UI 入口**：`命令输入框 → OPTIONS / OP`（`Message::OptionsOpen`，`src/app/commands/view.rs:312`）；或 `右键 → Options...`。
- **样式**：`src/ui/window/options.rs::view_window`。左侧固定 `TAB_RAIL_WIDTH = 178.0` 的竖直标签栏，对话框 880×620（`DIALOG_WIDTH`/`DIALOG_HEIGHT`）；页签枚举 `OptionsTab`（General、Files、OpenAndSave、Display、Drafting、Modeling、Selection、UserPreferences）。
- **视图相关子项**：
  - **Display 页**：`Model Space Appearance`（`Canvas mode`、`Model background`、`Paper background`、`Desk surround`、`Grid opacity`）、`Crosshair`（`Crosshair size`、`Cursor type`、`Crosshair color`）、`Lineweight`（`Model display scale`、`Fill TrueType glyphs (TEXTFILL)`）、`Command Line`（`Prompt lines` CLIPROMPTLINES、`History fade time` COMMANDLINEFADETIME）。
  - **User Preferences 页**：`Zoom`（`Reverse mouse wheel zoom (ZOOMWHEEL)`、`Zoom factor` ZOOMFACTOR）、`Text and Dimensions`、`Annotation`、`Right-click Customization`、`Block Edit`、`Drawing Units…`。
  - **Modeling 页**：`3D Modeling → Display Tools → NAVVCUBE: show the navigation cube`（`prefs.show_viewcube`，`Message::ShowViewCubeChanged`）。
  - **General 页**：`Plotting` 区含 `Plot and Page Setup…`（`Message::PlotDialogOpen`）与 “Show the page setup for new layouts”（`prefs.page_setup_on_new_layout`）。
- **触发命令**：`OPTIONS` / `OP`。
- **实现位置**：`src/ui/window/options.rs`（`OptionsTab`、`AppPrefs`、`SelectionPrefs`、`DrawingPrefs`、`view_window`）；处理分散于 `src/app/update/mod.rs`（如 `ShowViewCubeChanged`、`PageSetupOnNewLayoutChanged`）。

---

## 十一、命令行可直接触发但未在 View 选项卡固定面板出现的视图/显示命令

以下命令经命令行（`CommandRegistration` 自动补全）或右键菜单可达，属视图/显示范畴但不在 View 面板固定按钮中。

- `ZOOM ALL`（`ZA`）：缩放至绘图界限（`src/app/commands/dim.rs:854`，`fit_all_with_limits`）。
- `ZOOM PREVIOUS`（`ZP`）：恢复上一视图（`dim.rs:860`，`restore_previous_view`）。
- `ZOOM OBJECT`（`ZOBJ`/`ZOOM O`）：缩放至选中对象包围盒（`dim.rs:869`，`zoom_to_entities`）。
- `ZOOM DYNAMIC`（`ZD`）：动态平移/缩放（`dim.rs:893`，`zoom_dynamic_mode`）。
- `ZOOM SCALE <f>`（`ZS`）：按倍率缩放（`dim.rs:905`，`zoom_camera(1.0/factor)`）。
- `ZOOM EXTENTS ALL`（`ZEA`）：缩放所有模型视口（`dim.rs:829`）。
- `MVIEW`：在图纸空间创建视口（`src/app/commands/view.rs:524`；须先切换到图纸布局）。
- `MSPACE` / `PSPACE`：进出模型空间/图纸空间视口（`view.rs:544/547`，`Message::MspaceCommand`/`PspaceCommand`）。
- `VPLAYER`：逐视口图层冻结/解冻（`view.rs:798`，`VplayerCommand`）。
- `SYNCPVIEWPORTS` / `VPSYNC`：把首个选中视口的显示设置同步到其余视口（`view.rs:1194`）。
- `HIDE`：将活动视口切到隐藏线视觉样式（`view.rs:1242`，`SetRenderMode(HiddenLine)`）。
- `VIEW`（`V`）：命名视图的管理 —— 交互选项 `List`/`Save`/`Restore`/`Delete`（`src/app/commands/layerprops.rs:766`、`:781`）。
- `UCS`：交互式 UCS 命令（选项 Face/OBject/World/View/3Point/Z/X/Y/Origin/Save/Delete 或名称，`src/modules/view/ucs_cmd.rs::UcsCommand`）。
- `LIMITS`：设置/开关绘图界限（`src/modules/view/limits.rs::LimitsCommand`；`ON`/`OFF`/`SET` 分支见 `src/app/commands/display.rs:414`）。
- `PLOTWINDOW`：交互拾取两角点定义打印窗口矩形（青色预览，`src/modules/view/plot_window.rs::PlotWindowCommand`；分发 `src/app/commands/dim.rs:924`，结果 `CmdResult::SetPlotWindow`）。
- `QUICKPRINT`（`QP`）：运行命令、选择对象、Enter 即把选区包围盒打印为 PDF（无对话框，`src/modules/view/quick_print.rs::QuickPrintCommand`；分发 `src/app/commands/dim.rs:931`，结果 `CmdResult::QuickPrint`）。
- `CLEANSCREEN`：折叠周围面板以获得全画布（`src/app/commands/display.rs:576`，`Message::ToggleCleanScreen`）。
- `QUICKPROPERTIES`：切换浮动快速特性读数（`display.rs:580`，`Message::ToggleQuickProperties`）。
- `REDRAW` / `REDRAWALL` / `REGEN` / `REGENALL`：重绘/重生成（`display.rs:492`）。
- `TILEMODE`：控制模型/图纸空间切换变量（`src/app/commands/plotvars.rs:164`）。
- 菜单/右键：`右键 → Zoom Extents`（`ZOOM EXTENTS`，`src/ui/popup/context_menu.rs:699`）、`右键 → Properties`（`ToggleProperties`）。

### 状态栏 / 选项页提供的相关开关
- `GRID`（`ToggleGrid`）、`SNAP`（`ToggleGridSnap`）、`POLAR`（`TogglePolar`）、`OSNAP`/`DSETTINGS`（`ToggleSnapPopup`）、`ISOPLANE`/`ISODRAFT`（`display.rs:515-558`）等绘图辅助开关经命令/状态栏可达。

---

（文档结束）
