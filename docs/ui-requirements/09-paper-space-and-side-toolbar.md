# 图纸空间（Layout）、右侧竖直工具栏与选中上下文工具功能需求清单

本文档反推自 `src/modules/layout/`（`mod.rs`、`mview.rs`、`vplayer.rs`）、`src/modules/view/`（`plot_window.rs`、`quick_print.rs`、`mod.rs`）、`src/ui/side_toolbar.rs`、`src/ui/ribbon/context_tools.rs`、`src/ui/ribbon/draw_panel.rs`、`src/ui/ribbon/modify_panel.rs`、`src/ui/window/`（`plot.rs`、`pdf_dialogs.rs`、`print_all.rs`、`layout_manager.rs`）、`src/ui/dock.rs`、`src/ui/popup/scale_popup.rs` 以及它们的宿主接线（`src/app/view/mod.rs`、`src/app/commands/pdf_underlay.rs`、`src/app/commands/view.rs`、`src/app/commands/display.rs`、`src/ui/statusbar/mod.rs`）。覆盖图纸空间上下文工具、右侧竖直工具栏、XREF/PDF 底图/点云的选中上下文工具、选中对象的浮动修改面板、模型/图纸空间切换以及打印对话框。

右侧竖直工具栏的权威入口是 `src/app/view/mod.rs` 中 `selection_tools` 的选择（2049-1653 行附近）：其优先级为「XREF 上下文 → 点云上下文 → 底图上下文 → 图纸空间通用工具」，且「同一时刻只显示一个右缘工具栏」（源码注释 ponytail）。

---

## 一、图纸空间（Layout）上下文工具（右侧竖直工具栏）

只有当活动布局不是 `Model` 时（`is_paper = tab.scene.current_layout != "Model"`，`src/app/view/mod.rs::view`）且没有选中 XREF/点云/底图时才显示。定义于 `src/modules/layout/mod.rs::paper_space_tools()`，由 `paper_space_tools()` 返回的扁平 `Vec<ToolDef>` 驱动 `src/ui/side_toolbar.rs::view`。该 Context 选项卡已不再显示为 Ribbon 选项卡（源码注释：「the contextual ribbon tab is no longer shown」）。

### 视口（MVIEW，Viewport）
- **功能简介**：在图纸空间交互式创建视口（矩形/多边形/对象/适应/插入命名视图），也可作为 VPCLIP 裁剪既有视口。
- **UI 入口**：`图纸空间布局 → 视口右侧竖直工具栏 → Viewport（IconKind::Svg，icon viewport.svg）`。
- **样式**：`src/ui/side_toolbar.rs` 的 `view` 渲染 —— 单列居中图标按钮，按钮 `BTN_SIZE = 38.0`，图标 `ICON_SIZE = 22.0`，列间距 4，文字提示（tooltip）在按钮左侧（`tooltip::Position::Left`，字号 11）。
- **触发命令**：`MVIEW`（同一命令在裁剪模式时 `name()` 返回 `VPCLIP`）。
- **实现位置**：`src/modules/layout/mview.rs::tool()`、`MviewCommand`；入口 `src/modules/layout/mod.rs::paper_space_tools`；分发 `src/app/commands/view.rs::"MVIEW"`。
- **备注**：仅在非 Model 布局可用；在 Model 布局执行会报错 “MVIEW: switch to a paper space layout first.”。矩形/多边形/对象/Fit/Insert view 选项；命名视图「New」会临时切到 `Model` 定义模型范围（`CmdResult::MviewSwitchLayout("Model")`）。MVIEW 通过 `MviewCreate` / `MviewCreateClipped` 落盘（`src/app/command_driver/viewport.rs::handle_mview_create`/`handle_mview_clipped`）。

### 页面设置（PAGESETUP，Page Setup）
- **功能简介**：打开统一打印对话框，配置并保存页面设置。
- **UI 入口**：`图纸空间布局 → 视口右侧竖直工具栏 → Page Setup（icon pagesetup.svg）`。
- **样式**：与 `Viewport` 相同的竖直工具栏按钮。
- **触发命令**：`PAGESETUP`（`PAGESETUP` 折叠进统一 Plot 对话框，`src/app/commands/display.rs::"PAGESETUP"`）。
- **实现位置**：`src/modules/layout/mod.rs`（内联 `ToolDef { id:"PAGESETUP", event: ModuleEvent::Command("PAGESETUP") }`）；窗口 `src/ui/window/plot.rs::view_window`。
- **备注**：模型空间无图纸空间侧栏，因此 `src/modules/view/mod.rs` 的 View 选项卡 Plot 面板另设一个 `PAGESETUP` 小 `Tool`。

### 全部打印（PRINTALL，Print All）
- **功能简介**：批量输出选中的图纸布局（逐页 Layout 模式）。 
- **UI 入口**：`图纸空间布局 → 视口右侧竖直工具栏 → Print All（icon plot.svg）`。
- **样式**：竖直工具栏按钮（同上）。
- **触发命令**：`PRINTALL`。
- **实现位置**：`src/modules/layout/mod.rs`（内联 `ToolDef { id:"PRINTALL" }`）；分发 `src/app/commands/display.rs::"PRINTALL"` → `Message::PrintAllOpen`；窗口 `src/ui/window/print_all.rs::view_window`。
- **备注**：`Print All` 只列出非 Model 布局（`on_print_all_open` 过滤 `name != "Model"`，`src/app/update/file.rs`）。

### Layout Ribbon 分组（备用/历史入口）
- **功能简介**：`LayoutModule::ribbon_groups()` 定义 `Viewport` / `Plot` 两个分组（`Viewport` 含 `MVIEW`；`Plot` 含 `PAGESETUP`、`PRINTALL`）。
- **UI 入口**：`Ribbon → Layout 选项卡`（仅在非 Model 布局显示，据模块注释「This tab is only shown when the active layout is not "Model"」）。
- **样式**：`RibbonGroup` 标准面板（`Viewport` 一个 `Tool`，`Plot` 两个 `Tool`）。
- **实现位置**：`src/modules/layout/mod.rs::LayoutModule::ribbon_groups`。
- **备注**：源码注释说明该 Context 选项卡已不再显示，仅作为右侧竖直工具栏工具的同一数据源保留。

---

## 二、右侧竖直工具栏（Side Toolbar）框架与逐按钮

### 工具栏容器（`side_toolbar::view` / `view_with_active`）
- **功能简介**：在画布右缘浮出一列上下文图标按钮，随选择/空间变化自动出现或消失。
- **UI 入口**：`视口 → 画布右缘竖直工具栏`（任意触发该工具栏的上下文场景）。
- **样式**：`src/ui/side_toolbar.rs`：`column![].spacing(4)`，每个按钮 `BTN_SIZE = 38.0`、图标 `ICON_SIZE = 22.0`。容器 `padding(4)`，背景 `palette.background.weak.color`，描边 `background.neutral.color`、圆角 5。整条工具栏 `align_x(Right)`、`align_y(Center)`，右边距 `EDGE_MARGIN = 8.0`，顶部为视图立方体/UCS 列表预留 `TOP_RESERVE = 200.0`。按钮悬停/按下时背景 `background.strong.color`；`active` 命中的按钮背景 `primary.weak.color`、描边 `primary.base.color` 宽 1。工具提示在左侧。
- **触发命令**：按钮点击统一发 `Message::RibbonToolClick { tool_id, event }`，复用 Ribbon 工具分发路径。
- **实现位置**：`src/ui/side_toolbar.rs::view`、`view_with_active`、`icon_el`、`tip_panel`。
- **备注**：`view` 等价于 `active` 恒为 false 的 `view_with_active`；`tools` 为空返回 `None`，调用方跳过叠加。同一时刻只显示一个右缘工具栏（`src/app/view/mod.rs::view`）。

### 工具栏显示优先级（上下文选择）
- **功能简介**：根据当前选择决定显示哪种工具集；「XREF → 点云 → 底图」优先，其次才是图纸空间通用工具；REFEDIT/BEDIT 会话再叠加一组保存/放弃按钮。
- **UI 入口**：`图纸空间/模型空间视口右缘`。
- **样式**：不适用（逻辑选择）。
- **触发命令**：不适用。
- **实现位置**：`src/app/view/mod.rs::view`（`selection_tools` 判断与 `is_paper` 分支；REFEDIT/BEDIT 分支）。
- **备注**：源码注释 ponytail：「one right-edge toolbar at a time; stack them if both are ever needed together」。起始页（`tab.is_start`）不显示。REFEDIT 与 BEDIT 会话工具栏另见 `src/modules/draw/modify/refedit.rs::refedit_tools`、`block_edit.rs::block_edit_tools`（本文件不展开）。

---

## 三、选中外部参照（XREF）时的上下文工具

当且仅当选中集「全部是外部参照块引用」时（`selected_xrefs`，要求每个选中实体都是 `Insert` 且其块记录 `flags.is_xref || flags.is_xref_overlay`，`src/app/commands/pdf_underlay.rs::selected_xrefs`），且非起始页，右缘工具栏显示 `src/ui/ribbon/context_tools.rs::xref_tools()`（`src/app/view/mod.rs::view`，用 `side_toolbar::view`，无 active 高亮）。

### 编辑参照（_XREFEDIT，Edit Reference In-Place）
- **功能简介**：就地（REFEDIT）编辑选中的外部参照。
- **UI 入口**：`选中 XREF → 视口右缘竖直工具栏 → Edit Reference In-Place（icon edit_block.svg）`。
- **样式**：竖直工具栏按钮（tooltip 标签「Edit Reference\nIn-Place」）。
- **触发命令**：内部命令 `_XREFEDIT`（宿主转 `REFEDIT`）。
- **实现位置**：`src/ui/ribbon/context_tools.rs::xref_tools`；分发 `src/app/commands/pdf_underlay.rs::dispatch_pdf_underlay` 的 `"_XREFEDIT" | "_XREFOPEN"` 分支（反选后选该 xref 并 `dispatch_command("REFEDIT")`）。
- **备注**：命令名带前导下划线，表示宿主专用、不在命令行补全中直接出现。

### 打开参照（_XREFOPEN，Open Reference）
- **功能简介**：在新文档中打开选中的外部参照源文件。
- **UI 入口**：`选中 XREF → 视口右缘竖直工具栏 → Open Reference（icon FOLDER_OPEN）`。
- **样式**：竖直工具栏按钮。
- **触发命令**：内部命令 `_XREFOPEN`（宿主转 `XOPEN`）。
- **实现位置**：`src/ui/ribbon/context_tools.rs::xref_tools`；分发 `src/app/commands/pdf_underlay.rs`（同上分支，`command = "XOPEN"`）。

### 创建裁剪边界（_XREFCLIP，Create Clipping Boundary）
- **功能简介**：为选中的外部参照创建 XCLIP 裁剪边界。
- **UI 入口**：`选中 XREF → 视口右缘竖直工具栏 → Create Clipping Boundary（icon xclip.svg）`。
- **样式**：竖直工具栏按钮。
- **触发命令**：内部命令 `_XREFCLIP`（转 `XclipCommand`，等价 `XCLIP`）。
- **实现位置**：`src/ui/ribbon/context_tools.rs::xref_tools`；分发 `src/app/commands/pdf_underlay.rs::"_XREFCLIP"`（`XclipCommand::start_new_boundary`，检测是否已有裁剪以决定默认「新建/删除」）。
- **备注**：`XCLIP` / `CLIP` 注册于 `src/modules/insert/xclip.rs`。

### 移除裁剪（_XREFUNCLIP，Remove Clipping）
- **功能简介**：删除选中外部参照的裁剪边界。
- **UI 入口**：`选中 XREF → 视口右缘竖直工具栏 → Remove Clipping（icon xclip_remove.svg，带红色小叉）`。
- **样式**：竖直工具栏按钮（`UNCLIP` 图标为 `CLIP` 图标右下角加红叉）。
- **触发命令**：内部命令 `_XREFUNCLIP`。
- **实现位置**：`src/ui/ribbon/context_tools.rs::xref_tools`；分发 `src/app/commands/pdf_underlay.rs::"_XREFUNCLIP"`（`apply_xclip(..., XclipAction::Delete)`）。

### 外部参照选项板（EXTERNALREFERENCES，External References）
- **功能简介**：打开「外部参照」选项板（停靠面板）。
- **UI 入口**：`选中 XREF → 视口右缘竖直工具栏 → External References（icon FOLDER_OPEN）`。
- **样式**：竖直工具栏按钮（tooltip 标签「External\nReferences」）。
- **触发命令**：`EXTERNALREFERENCES`（也是普通注册命令名，非下划线）。
- **实现位置**：`src/ui/ribbon/context_tools.rs::xref_tools`；停靠面板 `PanelId::ExternalReferences`（`src/ui/dock.rs::PanelId::title/default_width`）。
- **备注**：该工具同时出现在 Reference 面板角落启动器与 PDF 底图工具栏中。

### XREF 淡显（XDWGFADECTL 滑条 + 开关）
- **功能简介**：控制外部参照整体淡显的开关与淡显量（0~90），并把结果写入 `XDWGFADECTL`。
- **UI 入口**：`Ribbon → 任意选项卡 → Reference 面板标题 “Reference ▾” → 弹出层 “Xref fading” 开关 + 滑条`（注意：此为一个 Ribbon 面板飞出，不在右缘竖直工具栏中）。
- **样式**：Reference 飞出面板宽 `260.0`（`draw_panel.rs::reference_overlay`），顶部 “Edit Reference” 行（`popup_row_style`），下方为开关按钮 + “Xref fading” 文本 + `slider(0..=90u8)` + 数值（字号 11）。面板描边为主题主色（`primary.base.color`）。
- **触发命令**：`XDWGFADECTL`（开关发 `Message::XrefFadeToggle`；拖动发 `Message::XrefFadeSlide`；松开发 `Message::XrefFadeCommit`）。
- **实现位置**：`src/ui/ribbon/draw_panel.rs::reference_overlay`、`REFERENCE_TOOLS`、`REFERENCE_PANEL_ID`；`reference_overlay` 中 `fade = ribbon.xref_fade`，`amount = fade.unsigned_abs().min(90)`。
- **备注**：开关会保留淡显量（以负值表示关闭）。这是任务中提到的「XDWGFADECTL 淡显滑条」入口；对比度/淡显仍在 Properties 面板中（`context_tools.rs` 头注）。

### XREF 绑定/插入/卸载/重载等（命令行 XREF）
- **功能简介**：外部参照的绑定、拆离、路径、卸载、重载、覆盖、附着、显示等操作。
- **UI 入口**：`命令输入框 → XREF`（右侧竖直工具栏未为这些操作单独设按钮，仅 XCLIP/打开/编辑；据源码推断）。
- **样式**：命令行提示。
- **触发命令**：`XREF`（选项提示 `Enter an option [?/Bind/Detach/Path/pathType/Unload/Reload/Overlay/Attach/Show] <Attach>:`）。
- **实现位置**：`src/modules/insert/xref_cmd.rs`（`prompt`；步骤 `Option`/`List`/`Names(...)`/`File`/`NewPath`/`PathTypeNames`/`PathTypeKind`/`ShowNames`）。
- **备注**：Bind/Detach/Unload/Reload/Overlay/Attach/Show 均在此命令中以关键字提供。XREF 淡显由 `XDWGFADECTL` 控制。

---

## 四、选中 PDF/图片底图时的上下文工具

当且仅当选中集「全部是同一类型的底图」时（`selected_pdf_underlays`，要求每个选中实体都是 `Underlay` 且 `underlay_type` 一致，`src/app/commands/pdf_underlay.rs`），右缘工具栏显示 `src/ui/ribbon/context_tools.rs::pdf_underlay_tools(kind)`。工具按钮的「开/关」高亮来自 `UnderlayContext`（`monochrome`/`shown`/`snap`），由 `src/app/view/mod.rs` 的 `view_with_active` 传入。

### 单色显示（_PDFULMONO，Display in Monochrome）
- **功能简介**：把选中底图切换为单色（灰度）显示或恢复。
- **UI 入口**：`选中底图 → 视口右缘竖直工具栏 → Display in Monochrome（icon underlay_frames.svg）`。
- **样式**：竖直工具栏按钮；当底图 `MONOCHROME` 标志为真时按钮呈 active（`primary.weak` 背景 + 主色描边）。
- **触发命令**：内部命令 `_PDFULMONO`。
- **实现位置**：`src/ui/ribbon/context_tools.rs::pdf_underlay_tools`；分发 `src/app/commands/pdf_underlay.rs::"_PDFULMONO"`（`u.set_monochrome(on)`，撤销标签 `PDFADJUST`）。

### 创建裁剪边界（_PDFULCLIP，Create Clipping Boundary）
- **功能简介**：为选中的底图创建裁剪边界（`PdfClipCommand`，等价 `PDFCLIP`/`DWFCLIP`/`DGNCLIP`/`IMAGECLIP`）。
- **UI 入口**：`选中底图 → 视口右缘竖直工具栏 → Create Clipping Boundary（icon xclip.svg）`。
- **样式**：竖直工具栏按钮。
- **触发命令**：内部命令 `_PDFULCLIP`。
- **实现位置**：`src/ui/ribbon/context_tools.rs::pdf_underlay_tools`；分发 `src/app/commands/pdf_underlay.rs::"_PDFULCLIP"`；命令 `src/modules/insert/pdf_clip.rs::PdfClipCommand`。

### 移除裁剪（_PDFULUNCLIP，Remove Clipping）
- **功能简介**：移除选中底图的裁剪。
- **UI 入口**：`选中底图 → 视口右缘竖直工具栏 → Remove Clipping（icon xclip_remove.svg）`。
- **样式**：竖直工具栏按钮。
- **触发命令**：内部命令 `_PDFULUNCLIP`。
- **实现位置**：`src/ui/ribbon/context_tools.rs::pdf_underlay_tools`；分发 `src/app/commands/pdf_underlay.rs::"_PDFULUNCLIP"`（清空裁剪顶点、去掉 `CLIPPING` 标志，撤销标签 `PDFCLIP`）。

### 显示底图（_PDFULSHOW，Show Underlay）
- **功能简介**：切换选中底图的显示/隐藏（ON/OFF）。
- **UI 入口**：`选中底图 → 视口右缘竖直工具栏 → Show Underlay（icon underlay_frames.svg）`。
- **样式**：竖直工具栏按钮；当底图 `ON` 标志为真时呈 active。
- **触发命令**：内部命令 `_PDFULSHOW`。
- **实现位置**：`src/ui/ribbon/context_tools.rs::pdf_underlay_tools`；分发 `src/app/commands/pdf_underlay.rs::"_PDFULSHOW"`（`u.set_on(on)`，撤销标签 `PDFUNDERLAY`）。

### 启用捕捉（_PDFULSNAP，Enable Snap）
- **功能简介**：切换对该类底图（PDF/DWF/DGN）几何的对象捕捉。
- **UI 入口**：`选中底图 → 视口右缘竖直工具栏 → Enable Snap（icon snap_underlays.svg）`。
- **样式**：竖直工具栏按钮；当 `underlay_osnap(kind)` 为真时呈 active。
- **触发命令**：内部命令 `_PDFULSNAP`（实际切换 `PDFOSNAP` / `DWFOSNAP` / `DGNOSNAP`）。
- **实现位置**：`src/ui/ribbon/context_tools.rs::pdf_underlay_tools`；分发 `src/app/commands/pdf_underlay.rs::"_PDFULSNAP"`（`set_underlay_osnap`）。

### 编辑图层（ULAYERS，Edit Layers）
- **功能简介**：打开「底图图层」对话框，逐层开关底图文件内部的图层。
- **UI 入口**：`选中底图 → 视口右缘竖直工具栏 → Edit Layers（icon underlay_layers.svg）`。
- **样式**：竖直工具栏按钮。
- **触发命令**：`ULAYERS`（别名 `UNDERLAYLAYERS`）。
- **实现位置**：`src/ui/ribbon/context_tools.rs::pdf_underlay_tools`；分发 `src/app/commands/display.rs::"UNDERLAYLAYERS" | "ULAYERS"` → `open_underlay_layers_dialog`（`src/app/commands/pdf_dialogs.rs`）；窗口 `src/ui/window/pdf_dialogs.rs::view_layers`。

### 导入为对象（_PDFULIMPORT，Import As Objects；仅 PDF）
- **功能简介**：把选中 PDF 底图的矢量内容导入为图形对象（PDFIMPORT）。
- **UI 入口**：`选中 PDF 底图 → 视口右缘竖直工具栏 → Import As Objects（icon cui_import.svg）`。
- **样式**：竖直工具栏按钮；**仅当底图种类为 `UnderlayType::Pdf` 时追加该按钮**（`pdf_underlay_tools` 末尾 `if kind == Pdf`）。
- **触发命令**：内部命令 `_PDFULIMPORT`。
- **实现位置**：`src/ui/ribbon/context_tools.rs::pdf_underlay_tools`；分发 `src/app/commands/pdf_underlay.rs::"_PDFULIMPORT"`（`PdfImportCommand::for_underlay`）。

### 外部参照选项板（EXTERNALREFERENCES）
- **功能简介**：打开外部参照选项板（与 XREF 上下文相同按钮，底图工具栏中也提供）。
- **UI 入口**：`选中底图 → 视口右缘竖直工具栏 → External References`。
- **样式**：竖直工具栏按钮。
- **触发命令**：`EXTERNALREFERENCES`。
- **实现位置**：`src/ui/ribbon/context_tools.rs::pdf_underlay_tools`。

### 底图的淡显/对比度（UNDERLAY，命令行）
- **功能简介**：编辑选中底图的淡显、对比度、开关、裁剪开关与单色。
- **UI 入口**：`命令输入框 → UNDERLAY`（右侧竖直工具栏未设淡显/对比度按钮，源码注释说明「Contrast and fade stay in the Properties panel」）。
- **样式**：命令行提示。
- **触发命令**：`UNDERLAY`（选项 `[Fade / Contrast / On / Off / Mono / Clip]`；子选项 `UNDERLAY FADE <0-100>`、`UNDERLAY CONTRAST <0-100>`、`UNDERLAY ON|OFF`、`UNDERLAY MONO ON|OFF`、`UNDERLAY CLIP ON|OFF`）。
- **实现位置**：`src/app/commands/display.rs::"UNDERLAY"` 与 `cmd if cmd.starts_with("UNDERLAY ")`。
- **备注**：无子命令时打印 `fade/contrast/on/clip/mono` 状态。

### 图片底图的透明度（TRANSPARENCY，命令行）
- **功能简介**：切换光栅图片的透明像素显示。
- **UI 入口**：`命令输入框 → TRANSPARENCY`（据源码为图片底图相关功能）。
- **样式**：命令行提示。
- **触发命令**：`TRANSPARENCY` / `TRANSPARENCY MODE` / `TRANSPARENCY ON` / `TRANSPARENCY OFF`。
- **实现位置**：`src/app/commands/display.rs::"TRANSPARENCY"`、`TransparencyCommand`（`src/modules/insert/image_transparency.rs`）。

---

## 五、选中点云时的上下文工具

当且仅当选中集「全部是点云」时（`selected_point_clouds`，要求每个选中实体都是 `ExtendedEntityData::PointCloudEx`，`src/app/commands/pdf_underlay.rs`），右缘工具栏显示 `src/ui/ribbon/context_tools.rs::point_cloud_tools()`。`_PCCROPSHOW` 按钮的 active 高亮来自点云 `show_cropping`（`set_point_cloud_context`，`src/app/view/mod.rs`）。

### 矩形裁剪（_PCCROPRECT，Rectangular Crop）
- **功能简介**：为选中的第一个点云创建矩形裁剪边界。
- **UI 入口**：`选中点云 → 视口右缘竖直工具栏 → Rectangular Crop（icon xclip.svg）`。
- **样式**：竖直工具栏按钮。
- **触发命令**：内部命令 `_PCCROPRECT`（`PointCloudCropCommand::for_cloud(handle, cloud, "")`）。
- **实现位置**：`src/ui/ribbon/context_tools.rs::point_cloud_tools`；分发 `src/app/commands/pdf_underlay.rs::"_PCCROPRECT" | "_PCCROPPOLY" | "_PCCROPCIRC"`；命令 `src/modules/insert/pc_crop.rs::PointCloudCropCommand`。
- **备注**：只作用于「第一个」选中的点云；图层锁定时不发命令。

### 多边形裁剪（_PCCROPPOLY，Polygonal Crop）
- **功能简介**：为选中的第一个点云创建多边形裁剪边界。
- **UI 入口**：`选中点云 → 视口右缘竖直工具栏 → Polygonal Crop（icon xclip.svg）`。
- **样式**：竖直工具栏按钮。
- **触发命令**：内部命令 `_PCCROPPOLY`（`option = "P"`）。
- **实现位置**：`src/ui/ribbon/context_tools.rs::point_cloud_tools`；分发 `src/app/commands/pdf_underlay.rs`（同分支）。

### 圆形裁剪（_PCCROPCIRC，Circular Crop）
- **功能简介**：为选中的第一个点云创建圆形裁剪边界。
- **UI 入口**：`选中点云 → 视口右缘竖直工具栏 → Circular Crop（icon xclip.svg）`。
- **样式**：竖直工具栏按钮。
- **触发命令**：内部命令 `_PCCROPCIRC`（`option = "C"`）。
- **实现位置**：`src/ui/ribbon/context_tools.rs::point_cloud_tools`；分发 `src/app/commands/pdf_underlay.rs`（同分支）。

### 显示裁剪（_PCCROPSHOW，Show Cropping）
- **功能简介**：切换点云裁剪边界的显示。
- **UI 入口**：`选中点云 → 视口右缘竖直工具栏 → Show Cropping（icon underlay_frames.svg）`。
- **样式**：竖直工具栏按钮；当点云 `show_cropping` 为真时呈 active（`view_with_active` 传入 `id == "_PCCROPSHOW" && shown`）。
- **触发命令**：内部命令 `_PCCROPSHOW`。
- **实现位置**：`src/ui/ribbon/context_tools.rs::point_cloud_tools`；分发 `src/app/commands/pdf_underlay.rs::"_PCCROPSHOW"`（`data.show_cropping = on`，撤销标签 `POINTCLOUDCROP`）。

### 反转裁剪（_PCCROPINVERT，Invert Cropping）
- **功能简介**：反转选中的每个点云的所有裁剪的「内/外」。
- **UI 入口**：`选中点云 → 视口右缘竖直工具栏 → Invert Cropping（icon xclip.svg）`。
- **样式**：竖直工具栏按钮。
- **触发命令**：内部命令 `_PCCROPINVERT`。
- **实现位置**：`src/ui/ribbon/context_tools.rs::point_cloud_tools`；分发 `src/app/commands/pdf_underlay.rs::"_PCCROPINVERT"`（对每个 `crop.inverted` 取反）。

### 移除全部裁剪（_PCUNCROP，Remove All Cropping）
- **功能简介**：清空选中点云的所有裁剪边界。
- **UI 入口**：`选中点云 → 视口右缘竖直工具栏 → Remove All Cropping（icon xclip_remove.svg）`。
- **样式**：竖直工具栏按钮。
- **触发命令**：内部命令 `_PCUNCROP`。
- **实现位置**：`src/ui/ribbon/context_tools.rs::point_cloud_tools`；分发 `src/app/commands/pdf_underlay.rs::"_PCUNCROP"`（`data.croppings.clear()`，撤销标签 `POINTCLOUDUNCROP`）。

### 外部参照选项板（EXTERNALREFERENCES）
- **功能简介**：打开外部参照选项板（点云工具栏也提供）。
- **UI 入口**：`选中点云 → 视口右缘竖直工具栏 → External References`。
- **样式**：竖直工具栏按钮。
- **触发命令**：`EXTERNALREFERENCES`。
- **实现位置**：`src/ui/ribbon/context_tools.rs::point_cloud_tools`。

### 点云样式（POINTCLOUDSTYLIZE，命令行 / 内部）
- **功能简介**：按一种着色方式给选中的点云着色（RGB / 对象色 / 强度 / 高程 / 法线 / 分类）。
- **UI 入口**：`命令输入框 → POINTCLOUDSTYLIZE`（右侧点云工具栏未直接给出；由命令行或内部 `_PCSTYLIZE` 派发，据源码推断）。
- **样式**：命令行提示。
- **触发命令**：`POINTCLOUDSTYLIZE`（选项 `[RGB/Object color/Intensity/Elevation/Normal/Classification] <RGB>`，默认记住上次选择）；内部 `_PCSTYLIZE`、`_PCSTYLIZEAPPLY <n> <handles...>`。
- **实现位置**：`src/modules/insert/pc_stylize.rs::PointCloudStylizeCommand`（`OPTIONS` 值 1=RGB/2=Object color/3=Normal/4=Elevation/5=Intensity/6=Classification）；分发 `src/app/commands/pdf_underlay.rs::"POINTCLOUDSTYLIZE" | "_PCSTYLIZE" | "_PCSTYLIZEAPPLY "`；宿主 `stylize_point_clouds`（不支持的数据类型会在命令行报告）。

### 点云颜色映射（POINTCLOUDCOLORMAP，对话框）
- **功能简介**：打开点云颜色映射对话框，编辑强度/高程着色方案与色带。
- **UI 入口**：`命令输入框 → POINTCLOUDCOLORMAP`（提示 `Select point cloud or [None] <None>:`；选 None 编辑图纸默认方案）。
- **样式**：对话框 `src/ui/window/pdf_dialogs.rs::view_point_cloud_color_map`（卡片、分段选项、chip 开关）。
- **触发命令**：`POINTCLOUDCOLORMAP`。
- **实现位置**：`src/modules/insert/pc_stylize.rs`（`inventory::submit!`）；宿主 `src/app/commands/pc_colormap.rs`；窗口 `src/ui/window/pdf_dialogs.rs::view_point_cloud_color_map`。
- **备注**：对话框明细见第八节（点云颜色映射对话框）。

### 点云管理器（POINTCLOUDMANAGER，停靠面板）
- **功能简介**：打开点位云管理器（区域与扫描的开关）。
- **UI 入口**：`命令输入框 → POINTCLOUDMANAGER`（停靠到右侧并展开）。
- **样式**：停靠面板 `PanelId::PointCloudManager`（默认宽 280，最大 `DOCK_MAX_W`）。
- **触发命令**：`POINTCLOUDMANAGER` / `POINTCLOUDMANAGERCLOSE`。
- **实现位置**：`src/app/commands/pdf_underlay.rs::dispatch_pdf_underlay` 的 `"POINTCLOUDMANAGER"`/`"POINTCLOUDMANAGERCLOSE"` 分支；窗口 `src/ui/window/pc_manager.rs`；面板框架 `src/ui/dock.rs::PanelId::PointCloudManager`。

### 附着点云（POINTCLOUDATTACH，对话框）
- **功能简介**：附着点云扫描或项目文件。
- **UI 入口**：`Ribbon → Insert 选项卡 → Attach 下拉 → Attach Point Cloud`（或命令输入框 `POINTCLOUDATTACH`）。
- **样式**：对话框 `src/ui/window/pdf_dialogs.rs::view_point_cloud_attach`。
- **触发命令**：`POINTCLOUDATTACH`（`-POINTCLOUDATTACH` 走命令行）。
- **实现位置**：`src/modules/insert/pc_attach.rs`；分发 `src/app/commands/xref_attach.rs::("POINTCLOUDATTACH", ...)`；对话框明细见第八节。

### 从点云提取断面线（Extract Section Lines）
- **功能简介**：从点云提取断面线/二维多段线。
- **UI 入口**：`点云相关对话框 → Extract Section Lines`（`src/ui/window/pdf_dialogs.rs::view_pc_section`；具体命令接线见宿主，据源码推断）。
- **样式**：对话框 `view_pc_section`（卡片、滑条、pick_list、chip）。
- **触发命令**：不适用（对话框内部消息 `PdfDialogMsg::Sec*`）。
- **实现位置**：`src/ui/window/pdf_dialogs.rs::view_pc_section`、`PcSectionState`。
- **备注**：明细见第八节。

---

## 六、选中对象时的浮动选择/修改面板

两个浮出面板由 Ribbon 面板标题点击展开，与选择内容/工具状态相关：`src/ui/ribbon/draw_panel.rs::overlay`（Draw 与 Modify 扩展）与 `reference_overlay`（Reference 扩展）。它们以 36×36（`CELL = 36.0 * SCALE`，`SCALE = 0.7`）小图标网格或多行选项列表呈现；工具提示在下方（`tooltip::Position::Bottom`，延迟 400ms）。

### Draw 面板扩展飞出（Draw ▾）
- **功能简介**：点击 Draw 面板标题展开更多绘图工具。
- **UI 入口**：`Ribbon → Draw 选项卡 → Draw 面板 → 标题 “Draw ▾”`。
- **样式**：`src/ui/ribbon/draw_panel.rs::overlay` 的网格，列数 `cols = 7`（由宽度推算，clamp 1..7），单元格 `CELL`，间距 `GAP`；面板描边为主题主色。
- **工具列表**（`src/ui/ribbon/draw_panel.rs::TOOLS`，全部）：
  - `SPLINE` — Spline Fit（样条拟合）
  - `SPLINECV` — Spline CV（控制点样条）
  - `XLINE` — Construction Line（构造线）
  - `RAY` — Ray（射线）
  - `DIVIDE` — Divide（定数等分）
  - `MEASURE` — Measure（定距等分）
  - `REGION` — Region（面域）
  - `BOUNDARY` — Boundary（边界）
  - `HELIX` — Helix（螺旋）
  - `DONUT` — Donut（圆环）
  - `MULTIPOINT` — Multiple Points（多点；带子项下拉）
- **实现位置**：`src/ui/ribbon/draw_panel.rs::TOOLS`、`PANELS`、`overlay`、`tool_button`。
- **备注**：`MULTIPOINT` 的 `options` 非空，因此是可展开子菜单项（`tool_button` 对非空 options 追加 `themed_arrow_down` 按钮，`DropdownSelectItem` 发命令）。

### 多点下拉子项（Multiple Points）
- **功能简介**：单点/多点/点样式三种点工具。
- **UI 入口**：`Ribbon → Draw 选项卡 → Draw 面板 → “Draw ▾”飞出 → Multiple Points ▾`。
- **样式**：`overlay` 中当命中的工具 `options` 非空时，绘制为每项一行的列表（`OPTION_HEIGHT = 30.0 * SCALE`，`popup_row_style`）。
- **触发命令**：子项（`draw_panel.rs::TOOLS` 中 MULTIPOINT 的 options，全部）：
  - `POINT` — Single Point（单点）
  - `MULTIPOINT` — Multiple Points（多点）
  - `DDPTYPE` — Point Style（点样式）
- **实现位置**：`src/ui/ribbon/draw_panel.rs::overlay`（`option_tool` 分支）、`tool_button`。

### Modify 面板扩展飞出（Modify ▾）
- **功能简介**：点击 Modify 面板标题展开更多编辑工具。
- **UI 入口**：`Ribbon → Draw 选项卡 → Modify 面板 → 标题 “Modify ▾”`。
- **样式**：与 Draw 扩展相同的 36×36 网格（`PANELS` 中 `id: "modify_extension"`，`tools: modify_panel::TOOLS`）。
- **工具列表**（`src/ui/ribbon/modify_panel.rs::TOOLS`，全部）：
  - `SETBYLAYER` — Set to ByLayer
  - `LENGTHEN` — Lengthen
  - `PEDIT` — Edit Polyline
  - `SPLINEDIT` — Edit Spline
  - `HATCHEDIT` — Edit Hatch
  - `ALIGN` — Align
  - `ALIGNLEFT` — Align Left
  - `ALIGNHCENTER` — Align Horizontal Centers
  - `ALIGNRIGHT` — Align Right
  - `ALIGNTOP` — Align Top
  - `ALIGNVCENTER` — Align Vertical Centers
  - `ALIGNBOTTOM` — Align Bottom
  - `BREAK` — Break
  - `BREAKATPOINT` — Break at Point
  - `JOIN` — Join
  - `REVERSE` — Reverse
  - `NCOPY` — Copy Nested Objects
  - `DRAWORDER_FRONT` — Draw Order（带子项下拉）
- **实现位置**：`src/ui/ribbon/modify_panel.rs::TOOLS`；渲染 `src/ui/ribbon/draw_panel.rs::overlay`。
- **备注**：`ALIGN` 为「点选 3D 放置」，`ALIGNLEFT`~`ALIGNBOTTOM` 为「选择集包围盒一次性对齐」（`modify_panel.rs` 注释）。

### 绘制顺序下拉子项（Draw Order）
- **功能简介**：调整对象绘制顺序。
- **UI 入口**：`Ribbon → Draw 选项卡 → Modify 面板 → “Modify ▾”飞出 → Draw Order ▾`。
- **样式**：选项列表行（`OPTION_HEIGHT`，`popup_row_style`）。
- **触发命令**：子项（`modify_panel.rs` 中该 Tool 的 options，全部）：
  - `DRAWORDER_FRONT` — Bring to Front
  - `DRAWORDER_BACK` — Send to Back
  - `DRAWORDER_ABOVE` — Bring Above Objects
  - `DRAWORDER_UNDER` — Send Under Objects
- **实现位置**：`src/ui/ribbon/modify_panel.rs::TOOLS`；归属解析 `src/ui/ribbon/draw_panel.rs::panel_for_dropdown`/`parent_panel`。

### Reference 面板扩展飞出（Reference ▾）
- **功能简介**：从 Reference 面板标题进入外部参照编辑与淡显。
- **UI 入口**：`Ribbon → 任意选项卡（含 Reference 面板）→ 标题 “Reference ▾” 与角落右箭头启动器`。
- **样式**：面板宽 `260.0`；含 “Edit Reference” 行与 “Xref fading” 开关+滑条；角落启动器为旋转 45° 的右箭头按钮（`tooltip::Position::Bottom`）。
- **触发命令/项**：
  - `REFEDIT` — Edit Reference（弹出层首行）
  - 角落启动器：`EXTERNALREFERENCES`（外部参照选项板）
  - `XDWGFADECTL` — 淡显开关/滑条（`Message::XrefFadeToggle`/`XrefFadeSlide`/`XrefFadeCommit`）
- **实现位置**：`src/ui/ribbon/draw_panel.rs::REFERENCE_TOOLS`/`REFERENCE_PANEL_ID`/`reference_overlay`/`group_title`；`Edit Reference` 命令实现 `src/modules/draw/modify/refedit.rs::tool`。

### 面板标题展开器本身（group_title）
- **功能简介**：面板标题作为一个可点击的下拉按钮（带上下箭头），无扩展工具的面板标题仅为静态文本。
- **UI 入口**：`Ribbon → 各选项卡 → 各面板标题角`。
- **样式**：`text(title).size(9)` + 箭头（`GROUP_TITLE_ARROW_SIZE = LARGE_DROPDOWN_ARROW_SIZE * 1.5`），`tool_btn_style`，`padding([1, 4])`；展开时箭头朝上，否则朝下。Reference 面板额外带角落启动器（`row![title_button, launcher].spacing(6)`）。
- **触发命令**：`Message::ToggleRibbonDropdown(panel.id)`。
- **实现位置**：`src/ui/ribbon/draw_panel.rs::group_title`；`panel_for_dropdown`/`owns_dropdown`/`parent_panel`。

---

## 七、图纸空间与模型空间的切换 UI

### 布局选项卡组（状态栏）
- **功能简介**：在状态栏左侧以可换行的一排选项卡切换 `Model` 与各图纸布局，并支持新增布局。
- **UI 入口**：`状态栏 → 左侧 “Model / Layout1 / …” 选项卡组`。
- **样式**：`src/ui/statusbar/mod.rs::space_tab`。活动选项卡背景 `primary.weak.color`、主色描边宽 1、圆角 2、文字 12px；非活动文字 alpha 0.72；禁用（起始页）文字 alpha 0.42 并带提示 “Open or create a drawing to switch layouts.”。选项卡组用 `WrapBar` 自动换行（`spacing 2`）。是否显示由 `show_layout_tabs` 决定（`LAYOUTTAB` 控制，`src/ui/ribbon/widgets.rs::"LAYOUTTAB"`）。
- **触发命令**：`Message::LayoutSwitch(name)`。
- **实现位置**：`src/ui/statusbar/mod.rs::space_tab`、`view`（`layouts`/`block_tabs` 循环，`SB_LAYOUT_TAB`/`SB_BLOCK_TAB` report key）。
- **备注**：仅图纸布局可重排（`reorderable_layouts` 为 `layouts[1..]`，`WrapBar::ReorderTab`）；块编辑会话标签追加在布局标签之后。

### 新建布局按钮（+）
- **功能简介**：在状态栏布局选项卡组末尾新增一个图纸布局。
- **UI 入口**：`状态栏 → 布局选项卡组末尾 “+” 按钮`。
- **样式**：`button(text("+").size(12))`，`button::subtle`，`padding([4, 8])`；起始页禁用并带提示。
- **触发命令**：`Message::LayoutCreate`。
- **实现位置**：`src/ui/statusbar/mod.rs`（`add_button` / `add_btn`）。

### 布局选项卡右键菜单（Rename / Delete）
- **功能简介**：右键图纸布局选项卡进行重命名或删除。
- **UI 入口**：`状态栏 → 布局选项卡（右键）→ Rename / Delete`。
- **样式**：`ContextMenu` 包住选项卡（`ContextMenu::new`）；菜单为 `container(column![rename, delete]).spacing(0).width(160)`、`container::bordered_box`、`padding([4, 0])`；每行文字 12px、`padding([4, 12])`。
- **触发命令**：`Message::LayoutRenameStart(name)` / `Message::LayoutDelete(name)`。
- **实现位置**：`src/ui/statusbar/mod.rs::layout_tab_context_menu`。
- **备注**：仅图纸布局选项卡有右键菜单（`has_context_menu = reorderable_layouts.contains(&label)`）。

### 布局选项卡内联重命名
- **功能简介**：在选项卡位置就地编辑布局名。
- **UI 入口**：`状态栏 → 布局选项卡 → Rename（右键）后选项卡变为输入框`。
- **样式**：`text_input`（`LAYOUT_RENAME_INPUT_ID`，宽 90）+ 取消 ✕ 按钮（`CLOSE` 图标，`button::subtle`）。
- **触发命令**：`Message::LayoutRenameEdit` / `LayoutRenameCommit` / `LayoutRenameCancel`。
- **实现位置**：`src/ui/statusbar/mod.rs::space_tab`（`rename_edit` 分支）。

### 空间模式切换按钮（MODEL / PAPER）
- **功能简介**：在图纸布局中进入/退出模型空间（MSPACE/PSPACE），并指示当前空间。
- **UI 入口**：`状态栏 → “PAPER” / “MODEL” 按钮`。
- **样式**：`src/ui/statusbar/mod.rs::space_mode_btn`。Model 选项卡显示不可点击的 “MODEL”；图纸布局 PSPACE 显示 “PAPER”（点击进模型空间）；MSPACE 显示 active 的 “MODEL”（点击退出）。active 时背景 `primary.weak.color`（悬停 `primary.base.color`）、主色描边、`padding([4, 7])`。
- **触发命令**：`Message::MspaceCommand`（进入）/ `Message::ExitViewport`（退出）；命令 `MSPACE` / `PSPACE`。
- **实现位置**：`src/ui/statusbar/mod.rs::space_mode_btn`；处理 `src/app/update/mod.rs::Message::MspaceCommand`/`ExitViewport`；分发 `src/app/commands/view.rs::"MSPACE"`/`"PSPACE"`。
- **备注**：进入视口会采纳该视口的显示、UCS（`adopt_view_display`、`refresh_active_ucs`）；退出时清空模型空间选择并回到图纸空间。`MSPACE` 在 Model 布局报错 “MS is only available in paper space layouts.”。

### 视口比例弹窗（Annotation / Viewport Scale）
- **功能简介**：在模型空间选择注释比例，在图纸空间选择视口比例；列出图纸中实际存在的比例（`ACAD_SCALELIST`）。
- **UI 入口**：`状态栏 → 比例胶囊（点击）→ 比例列表弹窗`（`Annotation / Viewport Scale\nClick to change`）。
- **样式**：`src/ui/statusbar/status_menu` 的菜单；每行 `check`（`themed_check_cell`）+ 标签 11px + `button::subtle`、`padding([4, 10])`；模型空间末尾追加 “Manage...” 行（`button::primary`，`padding([5, 10])`）。
- **触发命令**：模型空间发 `Message::SetAnnotationScale(label)`；图纸空间发 `Message::SetViewportScale(label)`；“Manage...” 发 `Message::ScaleManagerOpen`。
- **实现位置**：`src/ui/popup/scale_popup.rs::menu_entries`、`scale_row`、`manage_row`；胶囊与标签 `src/ui/statusbar/mod.rs::active_scale_label`/`format_scale`。
- **备注**：`is_model` 决定高亮与派发；图纸空间仅当视口被激活/选中时胶囊才可交互（`scale_pill_enabled`）；比例列表不自行注入，只显示文件存储的比例。

### 视口/注释比例同步开关（Viewport / Annotation Scale Sync）
- **功能简介**：切换视口比例与注释比例的同步。
- **UI 入口**：`状态栏 → “Viewport / Annotation Scale Sync” 开关（仅在图纸空间且有视口时显示）`。
- **样式**：`toggle_pill`（`ST_VP_SCALE_SYNC`）。
- **触发命令**：`Message::SyncViewportAnnotationScale`。
- **实现位置**：`src/ui/statusbar/mod.rs`（`vis(StatusPill::VpScaleSync)` 分支）。

### 视口裁剪（VPCLIP / CLIP）
- **功能简介**：裁剪既有图纸空间视口（矩形、多边形、拾取闭合对象，或删除裁剪）。
- **UI 入口**：`命令输入框 → VPCLIP`（提示 `Select viewport to clip:`）。
- **样式**：命令行提示；预览为青色多边形（`mview_preview`，`WireModel::CYAN`）。
- **触发命令**：`VPCLIP`（`MviewCommand::vpclip_select`/`vpclip`）。
- **实现位置**：`src/modules/layout/mview.rs`（`Step::ClipSelect`/`ClipChoice`/`ClipLength`）；落盘 `src/app/command_driver/viewport.rs::handle_vpclip`。
- **备注**：选项 `Polygonal`(P)，`Delete`(D，仅当已裁剪)；弧模式选项 Angle/CEnter/CLose/Direction/Line/Radius/Second pt/Undo。裁剪后视口收缩到边界范围且模型在纸面上不动。

### 视口配置（VPORTS）
- **功能简介**：在模型空间切分平铺视口（SINGLE/2H/2V/4）；在图纸空间列出既有视口或创建预设布局。
- **UI 入口**：`Ribbon → View 选项卡 → Model Viewports 面板 → Viewport Configuration（VPORTS）`；或命令输入框 `VPORTS [SINGLE|2H|2V|4]`。
- **样式**：Ribbon 面板按 `src/modules/view/mod.rs::ViewModule::ribbon_groups`：`Model Viewports` 分组含 `LargeTool(vports_config)`、`Tool(vports_named)`、`Tool(vports_join)`、`Tool(vports_restore)`。
- **触发命令**：`VPORTS`（`"HORIZONTAL"`→`VPORTS 2H`、`"VERTICAL"`→`VPORTS 2V`、`"VPJOIN"`→`VPORTS SINGLE`、`"CASCADE"`→`VPORTS 4`）。
- **实现位置**：`src/app/commands/view.rs::"VPORTS"` 分支；`src/modules/view/vports_config.rs` 等。
- **备注**：模型空间用 `pane_grid` 切分（`set_model_panes`）；图纸空间先擦除本布局的既有用户视口（图层锁定时报错），再按预设矩形创建并 `auto_fit_viewport`。列出视口时显示 `On/Locked` 状态（`VPORTS` 空参数）。

### 视口图层冻结（VPLAYER）
- **功能简介**：按视口冻结/解冻图层（`F <layer>`、`T <layer>`、`F ALL <layer>`、`T ALL <layer>`）。
- **UI 入口**：`命令输入框 → VPLAYER`（需先进入某个视口）。
- **样式**：命令行提示 `VPLAYER  F <layer> = Freeze  |  T <layer> = Thaw  |  Enter = Exit`。
- **触发命令**：`VPLAYER`。
- **实现位置**：`src/modules/layout/vplayer.rs::VplayerCommand`；分发 `src/app/commands/view.rs::"VPLAYER"`（Model 布局报错 “VPLAYER: switch to a paper space layout first.”；无活动视口报错 “VPLAYER: enter a viewport first (double-click or MS).”）。
- **备注**：`Handle::NULL` 表示对所有视口生效；多个空格分隔的图层名可一次性处理；大小写不敏感。图层管理器窗口在图纸布局中另有「每视口冻结」列（见 `02-ribbon-draw.md` 图层管理器条目）。

### 图纸空间插入基点（据源码推断）
- **功能简介**：在图纸空间中插入块/内容时使用图纸空间的插入基点。
- **UI 入口**：不适用（内部状态）。
- **样式**：不适用。
- **触发命令**：不适用。
- **实现位置**：`src/app/commands/blocks.rs`（`is_paper = current_layout != "Model"`、`paper_space_insertion_base`）。
- **备注**：仅说明图纸空间分支行为，无独立 UI。

---

## 八、打印 / 绘图相关对话框逐项

### 绘图对话框（Plot / Print，`ModalKind::Plot`）
- **功能简介**：完整打印设置与输出（打印机、纸张、比例、偏移、样式、质量、输出选项），提交后送系统打印机或写 PDF。
- **UI 入口**：`图纸空间右缘 “Page Setup” / “Print All” 工具 → 打印对话框`；命令输入框 `PLOT` / `PRINT` / `PAGESETUP`；`文件 → 打印`（快捷键 `Ctrl+P` → `PLOT`，`src/app/shortcuts.rs`）。
- **样式**：`src/ui/window/plot.rs::view_window`。暗色 pill+字段风格：顶部工具栏（左：New/Copy/Delete/Import…；右：Set current / Preview / 主按钮）、左侧页面设置列表（宽 160，含 “Page setups” 标签、内联新建/重命名输入行）、右侧两列设置区（左列 Printer/Paper/Orientation/Plot area；右列 Scale/Plot style/Shaded viewport/Plot options）。分区标题 `section_label`，复选框 `checkbox` 尺寸 14、文字 11。
- **触发命令**：`PLOT` / `PRINT` / `PAGESETUP`（均 `Message::PlotDialogOpen`）。
- **实现位置**：`src/ui/window/plot.rs::view_window`、`PlotDialogState`、`PlotDlgMsg`、`PlotFlag`；宿主 `src/app/commands/display.rs::"PLOT" | "PRINT"`、`"PAGESETUP"`；模态渲染 `src/app/view/modal.rs::ModalKind::Plot`。
- **逐项内容**：
  - **底部主按钮**：`Export PDF`（to_file 时）/ `Print`（否则）/ `Apply`（Print All 选项时）—— `PlotDlgMsg::Commit`。
  - **New / Copy / Delete / Import…**（页面设置管理）：`NewSetup`/`CopySetup`/`DeleteSetup`/`Import(Pick)`。
  - **Set current**：`SetCurrent`。
  - **Preview**：`Preview`。
  - **Printer / plotter 下拉**：系统默认（可显示解析到的打印机名）、已发现打印机、`Save to PDF file…`（`OUT_DEFAULT`/`OUT_PDF`）；选 PDF 时隐藏 Copies/Properties…（`to_file`）。
  - **Copies 字段 + Properties…**：`Copies`/`PrinterProperties`（Properties… 展开内联打印机选项编辑器，见下）。
  - **纸张下拉**：打印机上报的介质或目录纸张；下方有 “Custom…” 按钮，自定义纸张被选中时另有 “Remove”。
  - **Orientation 下拉**：`Portrait`/`Landscape`（`Orientation`）。
  - **Plot upside-down 复选**（`UpsideDown`）。
  - **Plot area 下拉**：`Extents`/`Limits`/`Display`/`Window`，图纸空间非 Print All 时首项插入 `Layout`（`Area`）；选 `Window` 时旁边出现 `Pick…`（`PickWindow`）。
  - **Plot offset 字段**：`X (mm)`/`Y (mm)` 或 `X (in)`/`Y (in)`（随单位），仅当非 Layout 且未勾选居中（`OffsetX`/`OffsetY`）。
  - **Center the plot 复选**（`Center`）。
  - **Fit to paper 复选**（`FitToPaper`）。
  - **Scale 下拉**（标签固定英文 “Scale”）：文件中的比例；`Scale`。
  - **Units 下拉**：`Millimeters`/`Inches`（`PaperUnits`）。
  - **Custom 行**：`Custom: [paper] mm = [drawing] units`（`CustomScalePaper`/`CustomScaleDrawing`）。
  - **Scale lineweights 复选**（`ScaleLw`，非 fit 时可用）。
  - **Plot style table 下拉**：`<none>` + CTB 列表（`Style`）；`Load…`（`LoadStyle`）、`Edit…`（`Message::PlotStylePanelOpen`）。
  - **Plot with plot styles 复选**（`PlotStyles`）。
  - **Display plot styles 复选**（`DisplayStyles`，仅图纸空间且有样式表）。
  - **Shade plot 下拉**（`Shade`）：`As displayed`/`2D Wireframe`/`3D Wireframe`/`Hidden Line`/`Flat Shaded`/`Gouraud Shaded`/`Flat Shaded + Edges`/`Gouraud Shaded + Edges`。
  - **Quality 下拉**（`Quality`）：`Low`/`Normal`/`High`。
  - **Plot options 复选**（`Background`/`Lineweights`/`Transparency`/`MergeLines`/`Stamp` + 图纸空间专属 `HidePaperspace`/`PaperspaceLast`/`SaveToLayout`）。
  - **页面设置列表**：首项 `PAGESETUP` 无/`<none>`、`<previous>`（`SETUP_NONE`/`SETUP_PREV`）；双击某行进入内联重命名（`RenameStart`/`NameInput`/`NameCommit`/`NameCancel`）。
  - **Import…（PSETUPIN）**：`PageSetupImportDraft` + `page_setup_import_chooser`（源文件名、逐个勾选、`All`、`Import`/`Cancel`）。
- **备注**：文档级打印偏好持久化（`PlotDialogState` 序列化 + `#[serde(skip)]` 运行时字段）；`PAPERUPDATE`/`PLOTOFFSET`/`PLOTROTMODE`/`PLOTTRANSPARENCYOVERRIDE`/`BACKGROUNDPLOT` 等由配置项承载。

### 打印机属性内联编辑器（Printer properties）
- **功能简介**：在 Plot 对话框内就地编辑所选打印机的驱动选项（介质、颜色、质量、双面、纸盒…）。
- **UI 入口**：`Plot 对话框 → Printer / plotter 行 → Properties…`。
- **样式**：`printer_options_editor`（浅背景卡片，圆角 4、描边中性色、`padding 8`）；每选项一行 `label : choice`；底部 `Reset`/`Cancel`/`Apply`（Apply 仅在选项已加载时出现）。
- **触发命令**：`PrinterProperties`/`PrinterOptionSet`/`PrinterOptionsApply`/`PrinterOptionsReset`/`PrinterOptionsCancel`。
- **实现位置**：`src/ui/window/plot.rs::printer_options_editor`、`PrinterOptionsDraft`。
- **备注**：变更相对驱动默认值的集合由 `PrinterOptionsDraft::overrides()` 计算。

### 自定义纸张内联编辑器（Custom paper size）
- **功能简介**：定义用户纸张名称、尺寸、单位与可打印边距，或删除用户纸张。
- **UI 入口**：`Plot 对话框 → Paper size 行 → Custom…`（选中用户纸张时另有 “Remove”）。
- **样式**：`custom_paper_editor`（浅背景卡片）；字段为紧凑 `label : field`（侧标签宽 52、数字框宽 64）；单位下拉 `Millimeters`/`Inches`；错误文字用 danger 色。
- **触发命令**：`CustomPaper(Open/Cancel/Name/Width/Height/Units/Margin/Add/Remove)`。
- **实现位置**：`src/ui/window/plot.rs::custom_paper_editor`、`CustomPaperDraft`、`CustomPaperError`。
- **备注**：数字保持文本直到 `Add` 校验（宽高必须为正）。

### 打印全部对话框（Print All，`ModalKind::PrintAll`）
- **功能简介**：勾选若干图纸布局，批量输出为 PDF 或直接打印（每页 Layout 模式）。
- **UI 入口**：`图纸空间右缘 “Print All” 工具`；命令 `PRINTALL`。
- **样式**：`src/ui/window/print_all.rs::view_window`（尺寸 520×420，`src/app/view/modal.rs`）。每布局一行 `checkbox`（尺寸 16，文字 12）；底部一行显示 `Printer: <name>`（或 “Default printer”）、`Options…`、`PDF`、`Print`。
- **触发命令**：`PrintAllOpen`；`PrintAllSelectAll`/`PrintAllSelectNone`；`PrintAllOptions`；`PrintAllPdf`/`PrintAllPrint`。
- **实现位置**：`src/ui/window/print_all.rs::view_window`；宿主 `src/app/commands/display.rs::"PRINTALL"`、`src/app/update/file.rs::on_print_all_open`/`on_print_all_options`/`print_all_pages`、`src/app/update/mod.rs`（`PrintAllPdf`/`PrintAllPrint`）。
- **备注**：列表仅含非 Model 布局；无布局时显示 “No paper layouts are available.”。`Options…` 复用 Plot 对话框（`print_all_options = true`，强制 `paper_space = true`、`area = "Layout"`），并保留之前的覆盖设置。

### PDF 导出（EXPORT / EXPORTPDF）
- **功能简介**：直接把当前布局导出为 PDF（无完整打印对话框）。
- **UI 入口**：`命令输入框 → EXPORT / EXPORTPDF`（据源码为直接导出）。
- **样式**：文件选择 + 导出流程。
- **触发命令**：`EXPORT` / `EXPORTPDF`（`Message::PlotExport` → `PlotExportPath`）。
- **实现位置**：`src/app/commands/display.rs::"EXPORT" | "EXPORTPDF"`；`src/app/update/mod.rs::Message::PlotExport`。

### PLOTWINDOW（拾取绘图窗口）
- **功能简介**：拾取两点定义绘图窗口区域，供打印对话框使用（模型/图纸空间共用）。
- **UI 入口**：`命令输入框 → PLOTWINDOW`；打印对话框 Plot area = Window 时点 `Pick…` 也会触发。
- **样式**：命令行提示 `PLOTWINDOW  Specify first corner of plot window:` / `Specify opposite corner:`；预览为青色选择矩形（`plotwindow_preview`，`WireModel::CYAN`）。
- **触发命令**：`PLOTWINDOW`。
- **实现位置**：`src/modules/view/plot_window.rs::PlotWindowCommand`；分发 `src/app/commands/dim.rs::"PLOTWINDOW"`；对话框 `Message::PlotDlg(PlotDlgMsg::PickWindow)`（`src/app/update/mod.rs` 中 `PLOTWINDOW` 触发）。
- **备注**：窗口角落为自由矩形，不受 Ortho/Polar 约束（`window_corner_pick` 恒 true）；提交对话框时写入布局 `PlotSettings`。

### QUICKPRINT / QP（快速打印）
- **功能简介**：选择对象后回车，把选择集包围盒快速打印为 PDF（无对话框）。
- **UI 入口**：`命令输入框 → QUICKPRINT / QP`。
- **样式**：命令行提示 `QUICKPRINT  Select objects to quick-print:` / `QUICKPRINT  %d selected — Enter to plot, or keep selecting:`。
- **触发命令**：`QUICKPRINT` / `QP`。
- **实现位置**：`src/modules/view/quick_print.rs::QuickPrintCommand`；分发 `src/app/commands/dim.rs::"QUICKPRINT"`。
- **备注**：为选择收集命令（`is_selection_gathering`），回车发 `CmdResult::QuickPrint(handles)`，由宿主写 PDF。

### 绘图样式表面板（Plot Style，`ModalKind::Plotstyle`）
- **功能简介**：加载/查看/清除 CTB/STB 绘图样式表，或在 Plot 对话框按 `Edit…` 打开。
- **UI 入口**：`Plot 对话框 → Plot style table 行 → Edit…`；命令 `PLOTSTYLE [LOAD|CLEAR|STATUS]`。
- **样式**：`ModalKind::Plotstyle`（`src/app/view/modal.rs`）；模态尺寸经 `sized_flow`。
- **触发命令**：`Message::PlotStylePanelOpen`；`PLOTSTYLE`/`PLOTSTYLE LOAD`/`PLOTSTYLE CLEAR`/`PLOTSTYLE STATUS` → `PlotStyleLoad`/`PlotStyleClear`。
- **实现位置**：`src/app/commands/display.rs::"PLOTSTYLE"`；渲染 `src/app/view/modal.rs::ModalKind::Plotstyle`。

### 底图图层对话框（Underlay Layers / ULAYERS）
- **功能简介**：查看并开关底图文件内部图层。
- **UI 入口**：`选中底图 → 右缘竖直工具栏 → Edit Layers`；命令 `ULAYERS` / `UNDERLAYLAYERS`。
- **样式**：`view_layers`（卡片式）：`Underlay` 卡片含底图 pick_list；`Layers` 卡片含搜索框、`N layers, M hidden` 摘要、可滚动图层列表（每行 “On/Off” 切换按钮，宽 64，`button_style(on)`）。
- **触发命令**：`LayersUnderlay`/`LayersSearch`/`LayersToggle`/`LayersOk`。
- **实现位置**：`src/ui/window/pdf_dialogs.rs::view_layers`、`UnderlayLayersState`、`LayerTarget`；打开 `src/app/commands/pdf_dialogs.rs::open_underlay_layers_dialog`。
- **备注**：底图按选中优先排序显示；找不到底图时报错 “No underlays found.”。

### 附着 PDF 底图对话框（Attach PDF Underlay）
- **功能简介**：选择 PDF 页面（或 DWF 图纸 / DGN 模型）并设置路径类型、插入点、比例、旋转、文件详情后附着。
- **UI 入口**：`Ribbon → Insert 选项卡 → Attach 下拉 → Attach PDF`（命令 `PDFATTACH`）；命令输入框亦可。
- **样式**：`view_attach`（卡片 + 分段选择 + chip）。文件行：名称 combo_box + 页数徽标 + `Browse...`；页面缩略图 tiles（132×86，白底、选中主色描边 2.5、勾选标记）；`Placement` 卡片（插入点 X/Y/Z、Scale、Rotation，各带 “On screen” chip）；`Path type` 分段（Full path/Relative path/No path）；`File details`（Found in/Saved path/Page size）；DGN 另加 `Conversion units`（Master/Sub units）。
- **触发命令**：`PdfDialogMsg::Attach*`（`AttachName`/`AttachBrowse`/`AttachPage`/`AttachPathType`/`AttachInsertOnScreen`/`AttachInsert`/`AttachScaleOnScreen`/`AttachScale`/`AttachRotationOnScreen`/`AttachRotation`/`AttachOk`）；DGN 子单位 `AttachSubUnits`。
- **实现位置**：`src/ui/window/pdf_dialogs.rs::view_attach`、`PdfAttachState`、`AttachMemory`、`page_thumbs`/`item_thumbs`、`click_page`（Ctrl/Shift 多选）。
- **备注**：页缩略图约 220px 宽；`AttachMemory` 记忆上次路径类型/插入/比例/旋转/详情选择；插入点/比例/旋转可勾 “On screen” 交给命令行。

### 附着点云对话框（Attach Point Cloud）
- **功能简介**：选择点云文件并设置放置（插入点/比例/旋转）、路径类型与选项（锁定、缩放到点云）后附着。
- **UI 入口**：`Ribbon → Insert 选项卡 → Attach 下拉 → Attach Point Cloud`（命令 `POINTCLOUDATTACH`）。
- **样式**：`view_point_cloud_attach`（卡片）：文件行 + 摘要徽标 + `Browse...`；`Preview` 卡片；`File details`（Found in/Saved path/Data/Size/Unit）；`Placement` 卡片；`Path type` 分段；`Options` 卡片（`Use geographic location`（禁用占位）、`Lock point cloud` chip、`Zoom to point cloud` chip）。
- **触发命令**：`PdfDialogMsg::Cloud*`（`CloudBrowse`/`CloudPathType`/`CloudInsertOnScreen`/`CloudInsert`/`CloudScaleOnScreen`/`CloudScale`/`CloudRotationOnScreen`/`CloudRotation`/`CloudLock`/`CloudZoom`/`CloudOk`）。
- **实现位置**：`src/ui/window/pdf_dialogs.rs::view_point_cloud_attach`、`PointCloudAttachState`；分发 `src/app/commands/xref_attach.rs`。

### 底图图层 / PDF 导入设置对话框（PDF Import Settings）
- **功能简介**：设置 PDF 导入要包含的数据、图层处理方式与导入选项。
- **UI 入口**：`命令输入框 → PDFIMPORT` 的 Settings 选项；窗口 `view_import_settings`。
- **样式**：`view_import_settings`（三类卡片）：`PDF data to import`（chips：`Vector geometry`/`Solid fills`/`TrueType text`/`Raster images`）；`Layers` 分段（`Use PDF layers`/`Create object layers`/`Current layer`）；`Import options`（chips：`Import as block`/`Join line and arc segments`/`Convert solid fills to hatches`/`Apply lineweight properties`/`Infer linetypes from collinear dashes`）；`footer` 有 `Options...`、`Save`。
- **触发命令**：`PdfDialogMsg::Vector`/`Fills`/`Text`/`Raster`/`Layers`/`AsBlock`/`Join`/`Hatches`/`Lineweights`/`Linetypes`/`SettingsOk`/`Options`。
- **实现位置**：`src/ui/window/pdf_dialogs.rs::view_import_settings`、`settings_cards`、`PdfImportSettings`；宿主 `src/app/commands/pdf_dialogs.rs::open_pdf_import_settings`。

### 导入 PDF 文件对话框（Import PDF / File）
- **功能简介**：选择要导入的 PDF 页面并设置插入点、比例、旋转与导入选项后导入。
- **UI 入口**：`命令输入框 → PDFIMPORT` 的 File 选项；窗口 `view_import_file`。
- **样式**：`view_import_file`：文件行 + `Browse...`；`Pages` 卡片（大预览 300 高、白底、上一页/下一页箭头、页码输入框、`/ N`、`Page size`/`PDF scale`）；`Placement` 卡片（插入点 “On screen” chip、Scale、Rotation 分段 0°/90°/180°/270°）；右侧可滚动复用 `settings_cards`；`footer` 有 `Options...`、`Import`。
- **触发命令**：`PdfDialogMsg::ImportBrowse`/`ImportPage`/`ImportPageText`/`ImportInsertOnScreen`/`ImportScale`/`ImportRotation`/`ImportOk`/`Options`；`RotationChoice`（0/90/180/270）。
- **实现位置**：`src/ui/window/pdf_dialogs.rs::view_import_file`、`PdfImportFileState`、`RotationChoice`。

### 点云颜色地图对话框（Point Cloud Color Map）
- **功能简介**：编辑强度/高程着色的色带方案、颜色数量、梯度/反相、范围与越界处理。
- **UI 入口**：`命令输入框 → POINTCLOUDCOLORMAP`（提示 `Select point cloud or [None] <None>:`）；窗口 `view_point_cloud_color_map`。
- **样式**：顶部标签 `Intensity`/`Elevation`/`Classification`（Classification 禁用）；`Color ramp` 卡片（色带条 56×360，带刻度标签）；`Color scheme` 卡片（方案 pick_list + `New`、颜色数量下拉 2..=25 + 均分条 + 反相按钮 + `Delete`、`Display as gradient` chip + `Rename`、内联命名输入行）；`Range of colorized points` 卡片（强度：最大/最小强度带色块；高程：`Apply to extents of point cloud` chip + `Interval height` + 最大/最小高程带色块；`Out of range points` 下拉）。底部 `?`、`Make stylization current` chip、`Cancel`/`Apply`/`OK`。
- **触发命令**：`PdfDialogMsg::MapTab`/`MapScheme`/`MapCount`/`MapEven`/`MapReverse`/`MapGradient`/`MapNew`/`MapDelete`/`MapRename`/`MapNameInput`/`MapNameOk`/`MapNameCancel`/`MapMax`/`MapMin`/`MapInterval`/`MapExtents`/`MapOutOfRange`/`MapCurrent`/`MapApply`/`MapOk`；`OutOfRange`（`Use min/max colors`/`Use RGB scan colors`/`Hide points`）。
- **实现位置**：`src/ui/window/pdf_dialogs.rs::view_point_cloud_color_map`、`PointCloudColorMapState`、`MapRamp`、`MapSlot`、`ramp_bar`、`blend`；宿主 `src/app/commands/pc_colormap.rs`。

### 从点云提取断面线对话框（Extract Section Lines from Point Cloud）
- **功能简介**：配置并从点云提取断面线/二维多段线。
- **UI 入口**：`点云相关对话框 → Extract Section Lines`；窗口 `view_pc_section`。
- **样式**：`view_pc_section`（卡片）：`Extract` 分段（`Entire cross section`/`Perimeter only`）；`Maximum points to process` 卡片（1,000~200,000 滑条 + 数值框，`Faster`/`More accurate`/估算时间）；`Output geometry` 卡片（Layer、Color、`Lines`/`2D Polylines` 分段、Polyline width）；`Extraction tolerances` 卡片（Minimum line length、Connect lines tolerance、Collinear angle tolerance 各带 ⌖ 屏幕拾取按钮）。底部 `?`、`Preview result` chip、`Cancel`/`Create`。
- **触发命令**：`PdfDialogMsg::SecPerimeter`/`SecMaxPoints`/`SecLayer`/`SecColor`/`SecPolylines`/`SecWidth`/`SecMinLength`/`SecConnect`/`SecAngle`/`SecPick`/`SecPreview`/`SecCreate`；`LineColor`（ByLayer/ByBlock/Red/Yellow/Green/Cyan/Blue/Magenta/White）。
- **实现位置**：`src/ui/window/pdf_dialogs.rs::view_pc_section`、`PcSectionState`、`LineColor`。

### 布局管理器窗口（Layout Manager，`ModalKind::LayoutManager`）
- **功能简介**：新建/删除/重命名布局、左右移动布局顺序、设为当前；查看 Model/论文空间状态。
- **UI 入口**：`命令输入框 → LAYOUTMANAGER / LAYOUTPANEL`；窗口 `layout_manager::view_window`（尺寸 640×320）。
- **样式**：`src/ui/window/layout_manager.rs::view_window`。顶部工具栏（`New Layout`/`Delete`；右侧 `Move Left`/`Move Right`/`Set Current`）；左侧 `Layouts` 列表（宽 220，当前项带 `themed_arrow_left` 标记）；右侧详情（标题 “Model Space”/“Paper Space Layout”、Name/Status（`Active` 主色/`Inactive` 弱化）、`Rename` 输入框 + `OK`）。Delete 在 Model 上为 `button::secondary`，否则 `button::danger`。
- **触发命令**：`Message::LayoutManagerNew`/`LayoutManagerDelete`/`LayoutManagerMoveLeft`/`LayoutManagerMoveRight`/`LayoutManagerSetCurrent`/`LayoutManagerSelect`/`LayoutManagerRenameBuf`/`LayoutManagerRenameCommit`。
- **实现位置**：`src/ui/window/layout_manager.rs::view_window`；分发 `src/app/commands/view.rs::"LAYOUTMANAGER" | "LAYOUTPANEL"` → `Message::LayoutManagerOpen`；模态 `src/app/view/modal.rs::ModalKind::LayoutManager`。

### 比例管理器窗口（Scale Manager，`ModalKind::ScaleManager`）
- **功能简介**：管理标注/视口比例列表。
- **UI 入口**：`状态栏 → 比例弹窗（模型空间）→ Manage...`。
- **样式**：`ModalKind::ScaleManager`（比例名 + 比值列）。
- **触发命令**：`Message::ScaleManagerOpen`。
- **实现位置**：`src/ui/popup/scale_popup.rs::manage_row`；渲染 `src/app/view/modal.rs::ModalKind::ScaleManager`。

---

## 九、停靠面板框架（Dock，与侧栏/上下文工具共用）

### 停靠面板集合与几何
- **功能简介**：任意数量的侧面板在画布左右边缘的竖直栈中停靠，记住侧别、顺序、宽度与自动折叠；支持拖动重新停靠与拖宽。
- **UI 入口**：`画布左缘/右缘 → 停靠面板栈`。
- **样式**：`src/ui/dock.rs`。面板 `frame`：`padding 6`、固定宽度、全高、背景 `background.base`、中性边；标题栏 `title_bar`：标题 12px + 自动折叠 pin（`PIN` 图标 12）+ 关闭（`CLOSE` 图标 12），背景 `background.weak`、`padding([3, 6])`，鼠标按标题栏开始拖动（`Interaction::Grab`）；工具按钮 `tool_button`：`TOOL_H = 22.0`，按钮尺寸 `TOOL_H + 8.0`，悬停背景 `background.strong.color`、圆角 3、tooltip 在下方。宽度 `DOCK_MIN_W = 200.0`、`DOCK_MAX_W = 600.0`（ExternalReferences 允许 2×）。
- **触发命令**：`DockMsg::DockGrab`/`ResizeGrab`/`WidthReset`/`AutoCollapseToggle`/`Close`/`Hover`/`DragMove`/`DragRelease`/`HoverExit`。
- **实现位置**：`src/ui/dock.rs::DockState`/`DockPanel`/`PanelId`/`title_bar`/`tool_button`/`frame`/`drop_index`；默认布局左 `Properties`、右 `BlockPalette`。
- **备注**：`PanelId` 含 `Properties`/`BlockPalette`/`ExternalReferences`/`Browser`/`NodeGraph`/`PointCloudManager`；`ExternalReferences` 默认宽 460。当选择集变化时，右缘上下文工具栏会暂时覆盖停靠面板（源码注释「over the paper-space tools, which come back when the selection changes」）。

---

## 十、未在本文档主要 UI 出现但相关（补充）

### 侧栏工具集数据来源
- Layout：`src/modules/layout/mod.rs::paper_space_tools`。
- XREF：`src/ui/ribbon/context_tools.rs::xref_tools`。
- PDF/图片底图：`src/ui/ribbon/context_tools.rs::pdf_underlay_tools`。
- 点云：`src/ui/ribbon/context_tools.rs::point_cloud_tools`。
- REFEDIT / BEDIT 会话：`src/modules/draw/modify/refedit.rs::refedit_tools`、`block_edit.rs::block_edit_tools`（在 `src/app/view/mod.rs` 中叠加）。
- 选择上下文数据的收集：`src/app/commands/pdf_underlay.rs::sync_underlay_tab`（由选择/撤销/编辑等触发），写入 `Ribbon::set_underlay_context`/`set_point_cloud_context`。

### 视口编辑框架（MSPACE 采用）
- **功能简介**：进入视口后，平移/缩放/旋转/UCS 都作用于该视口相机，并渲染活动视口边框。
- **UI 入口**：不适用（进入视口后由导航操作驱动）。
- **样式**：活动视口边框/工具提示（`src/app/view/controls.rs::viewport_tooltip`）。
- **触发命令**：不适用。
- **实现位置**：`src/scene/mspace.rs`（`pan_active_viewport`/`zoom_active_viewport`/`orbit_active_viewport`/`viewport_edit_frame_for`/`normalize_active_viewport_view`）；导航 `src/app/navigation.rs`。

---

（文档结束）
