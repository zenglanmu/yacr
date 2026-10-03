# Ribbon「Insert」选项卡功能需求清单

本文档反推自 `src/modules/insert/`（`mod.rs` 及全部 `.rs`）与 `src/ui/window/`（`xref_manager.rs`、`xref_attach.rs`、`xref_help.rs`、`block_definition.rs`、`block_palette.rs`、`attribute_editor.rs`、`wblock.rs`、`pdf_dialogs.rs`、`pc_manager.rs`）以及 `src/app/commands/`（`xref_attach.rs`、`xclip.rs`、`pdf_dialogs.rs`、`pdf_import.rs`、`pdf_underlay.rs`、`point_cloud.rs`、`pc_extract.rs`、`pc_colormap.rs`、`blocks.rs`、`draw.rs`、`display.rs`、`inquiry.rs`、`styleprops.rs`）。Ribbon 面板与工具的权威布局入口是 `src/modules/insert/mod.rs::InsertModule::ribbon_groups()`。

面板组的权威顺序（`src/modules/insert/mod.rs`）：Reference、Block、Attributes、Import、Content。每个面板标题（`group_title`）可点击展开该组的扩展飞出面板。

---

## 一、Reference 面板（Ribbon → Insert 选项卡 → Reference 面板）

面板布局（`src/modules/insert/mod.rs`）：`LargeTool(Attach XREF)`、`LargeDropdown(Attach Underlay)`、`LargeTool(Clip)`、`LargeTool(Adjust)`、`LabeledTool(Underlay Layers)`、`LabeledDropdown(Frames)`、`LabeledDropdown(Snap to Underlays)`。

### 附着外部参照（Attach XREF）
- **功能简介**：把一个外部 DWG/DXF 图档作为外部参照块附着到当前图形并交互式放置。
- **UI 入口**：`Ribbon → Insert 选项卡 → Reference 面板 → 附着外部参照（LargeTool，标签“Attach XREF”）`。
- **样式**：`LargeTool`（大按钮，图标 `assets/icons/blocks/insert.svg`）。
- **触发命令**：`XATTACH`（同义命令 `ATTACH`）。
- **实现位置**：`src/modules/insert/xattach.rs::tool()`、`XAttachCommand`、`XrefAttachRequest`、`XrefPlacement`；文件拾取与对话框见 `src/app/commands/xref_attach.rs::dispatch_xref_attach`、`start_xref_attach`、`commit_xref_attach`。
- **备注**：`XATTACH` 先弹文件拾取器（`Message::XAttachPick`），选完打开“Attach External Reference”对话框（`src/ui/window/xref_attach.rs`）；命令行参数形式 `XATTACH <path>` 直接建命令。命令提示：`Specify insertion point or [Scale/X/Y/Z/Rotate/PScale/PX/PY/PZ/PRotate]:`。自我附着（`is_self_attach`）报 “Possible circular reference…”。已有同名参照复用定义并提示 “Using existing definition.”。定义与 INSERT 同属一步撤销（`commit_xref_attach`）。

### 附着底图下拉（Attach Underlay）
- **功能简介**：把一个底图文件（PDF/DWF/DGN）或点云附着到图形中并放置。
- **UI 入口**：`Ribbon → Insert 选项卡 → Reference 面板 → 附着底图（LargeDropdown，标签“Attach Underlay”，默认 PDFATTACH）`。
- **样式**：`LargeDropdown`（大按钮带 ▾；默认图标 `assets/icons/underlay_layers.svg`）。
- **触发命令**：默认 `PDFATTACH`。下拉子项（`mod.rs::UNDERLAY_ATTACH`，全部）：
  - `PDFATTACH` — Attach PDF（附着 PDF 页）
  - `DWFATTACH` — Attach DWF（附着 DWF/DWFx 图纸）
  - `DGNATTACH` — Attach DGN（附着 DGN 模型）
  - `POINTCLOUDATTACH` — Attach Point Cloud（附着点云，图标 `pc_attach::ICON`）
- **实现位置**：`src/modules/insert/mod.rs`（内联 items）；各命令实现见下节细则。

#### Attach PDF（PDFATTACH）
- **功能简介**：把 PDF 的一页或多页作为底图附着到图形中，可指定页、插入点、缩放、单位和旋转。
- **UI 入口**：`Ribbon → Insert 选项卡 → Reference 面板 → 附着底图下拉 → Attach PDF`。
- **样式**：下拉子项（弹出列表行）。
- **触发命令**：`PDFATTACH`（命令行形式 `-PDFATTACH`）。
- **实现位置**：`src/modules/insert/pdf_attach.rs::PdfAttachCommand`、`underlays_for_pages`、`ensure_underlay_definition`；分发 `src/app/commands/blocks.rs`（`PDFATTACH`/`-PDFATTACH` 分支）与 `src/app/commands/pdf_dialogs.rs::open_pdf_attach_dialog`、`pdf_attach_ok`。
- **备注**：命令提示序列：`Path to PDF file to attach:` → `Enter page number or [?] <1>:` → `Specify insertion point:` → 报告 `Base image size: Width…, Height…, <unit>` → `Specify scale factor or [Unit] <1>:` → `Specify rotation <上次角度>:`。`Unit` 选项 9 种单位：MM/Centimeter/Meter/Kilometer/Inch/Foot/Yard/MILe/Unitless。对话框可多选页（Ctrl/Shift，`click_page`），页码 “?” 列出页。

#### Attach DWF（DWFATTACH）
- **功能简介**：把 DWF/DWFx 图纸作为底图附着，按图纸名选择。
- **UI 入口**：`Ribbon → Insert 选项卡 → Reference 面板 → 附着底图下拉 → Attach DWF`。
- **样式**：下拉子项。
- **触发命令**：`DWFATTACH`（命令行 `-DWFATTACH`）。
- **实现位置**：`src/modules/insert/pdf_attach.rs::PdfAttachCommand::for_kind(UnderlayType::Dwf, …)`；分发 `src/app/commands/blocks.rs`、`src/app/commands/xref_attach.rs::start_underlay_attach`；对话框 `src/app/commands/pdf_dialogs.rs::open_underlay_attach_dialog`。
- **备注**：提示 `Enter name of sheet or [?] <首图纸>:`；DWF 底图创建时对比度 75、褪色 25（`underlays_for_pages`）。文件拾取过滤器 `dwf`/`dwfx`。

#### Attach DGN（DGNATTACH）
- **功能简介**：把 DGN 模型作为底图附着，可选主/子单位换算。
- **UI 入口**：`Ribbon → Insert 选项卡 → Reference 面板 → 附着底图下拉 → Attach DGN`。
- **样式**：下拉子项。
- **触发命令**：`DGNATTACH`（命令行 `-DGNATTACH`）。
- **实现位置**：`src/modules/insert/pdf_attach.rs::PdfAttachCommand::for_kind(UnderlayType::Dgn, …)`、`accept_conversion`；分发 `src/app/commands/blocks.rs`、`xref_attach.rs`。
- **备注**：提示 `Enter name of model or [?] <首个模型>:` → `Specify conversion units [Master/Sub] <Master>:`；Sub 时默认缩放为 `1.0 / sub_per_master`。底图创建时对比度 75、褪色 25。

#### Attach Point Cloud（POINTCLOUDATTACH）
- **功能简介**：把点云扫描（`.rcs`）或扫描项目（`.rcp`）附着到图形并放置。
- **UI 入口**：`Ribbon → Insert 选项卡 → Reference 面板 → 附着底图下拉 → Attach Point Cloud`。
- **样式**：下拉子项（图标 `assets/icons/pc_attach.svg`）。
- **触发命令**：`POINTCLOUDATTACH`（命令行 `-POINTCLOUDATTACH`）。
- **实现位置**：`src/modules/insert/pc_attach.rs::PointCloudAttachCommand`、`AttachOptions`、`PointCloudPlacement`；分发 `src/app/commands/xref_attach.rs`（`POINTCLOUDATTACH` 分支）、`src/app/commands/point_cloud.rs::attach_point_cloud`；对话框 `src/app/commands/pdf_dialogs.rs::open_point_cloud_attach_dialog`、`point_cloud_attach_ok`。
- **备注**：提示序列 `Path to point cloud file to attach:` → `Specify insertion point <0,0>:` → `Specify scale factor <1>:` → `Specify rotation angle <0>:`，成功输出 “1 point cloud attached”。定义存入 `ACAD_POINTCLOUD_EX_DICT`，按文件名命名。文件拾取过滤器：Point Cloud Project(`rcp`)、Point Cloud Scan(`rcs`)。

### 裁剪（Clip / XCLIP）
- **功能简介**：把块参照裁剪到指定边界（矩形/多边形/选多段线），并支持开关/深度/删除/生成多段线。
- **UI 入口**：`Ribbon → Insert 选项卡 → Reference 面板 → 裁剪（LargeTool，标签“Clip”）`。
- **样式**：`LargeTool`（图标 `assets/icons/xclip.svg`）。
- **触发命令**：`XCLIP`（同义 `CLIP`）。
- **实现位置**：`src/modules/insert/xclip.rs::tool()`、`XclipCommand`、`XclipAction`、`ClipCommand`；应用 `src/app/commands/xclip.rs::apply_xclip`；分发 `src/app/commands/blocks.rs`（`XCLIP`、`CLIP`、`_XREFCLIP`、`_XREFUNCLIP`）。
- **备注**：命令提示 `Enter clipping option [ON/OFF/Clipdepth/Delete/generate Polyline/New boundary] <New>:`。New 边界提示 `[Select polyline/Polygonal/Rectangular/Invert clip] <Rectangular>:`。Clipdepth 提示 `Specify front/back clip point or [Distance/Remove]:`，前后平面冲突报 `DEPTH_REJECTED`。边界预览为青色（`WireModel::CYAN`）。`CLIP` 按所选对象类型分派：块参照→XCLIP、底图→PDFCLIP、栅格图像→IMAGECLIP、视口→VPCLIP，否则 `*Invalid selection*`。

### 图像调整（Adjust / XADJUST）
- **功能简介**：对选中的栅格图像设置亮度、对比度、褪色（0~100）。
- **UI 入口**：`Ribbon → Insert 选项卡 → Reference 面板 → 调整（LargeTool，标签“Adjust”）`。
- **样式**：`LargeTool`（图标 `assets/icons/xadjust.svg`）。
- **触发命令**：`ADJUST`。
- **实现位置**：`src/modules/insert/xadjust.rs::tool()`（命令 id `ADJUST`）；分发 `src/app/commands/display.rs`（`"ADJUST"` 与 `"ADJUST "` 分支）、`SelectThenKeywordCommand`。
- **备注**：提示 `ADJUST  [Brightness / Contrast / Fade]:`，随后 0-100 数值；据此改 `RasterImage` 的 `brightness`/`contrast`/`fade`。命令名为 `ADJUST`。

### 底图图层（Underlay Layers）
- **功能简介**：开关某个 PDF/DWF/DGN 底图的内部图层。
- **UI 入口**：`Ribbon → Insert 选项卡 → Reference 面板 → 底图图层（LabeledTool，标签“Underlay Layers”）`。
- **样式**：`LabeledTool`（1 行高、带文字标签的小按钮）。
- **触发命令**：`UNDERLAYLAYERS`（别名 `ULAYERS`）。
- **实现位置**：`src/modules/insert/underlay_layers.rs::tool()`；分发 `src/app/commands/display.rs::"UNDERLAYLAYERS" | "ULAYERS"`；窗口逻辑 `src/app/commands/pdf_dialogs.rs::open_underlay_layers_dialog`、`apply_underlay_layers`；界面 `src/ui/window/pdf_dialogs.rs::view_layers`、`UnderlayLayersState`、`LayerTarget`。
- **备注**：无底图时报 “No underlays found.”；对话框下拉选择底图、搜索框过滤、逐层勾选开关；Apply 写入图层覆盖 EED（`LAYER_OVERRIDE_APP`，DGN 用 `DGN_OVERRIDE_PLACEHOLDER`），一步撤销标签 `ULAYERS`。PDF 图层来自 `pdf_layers::layers`，DWF/DGN 来自 `underlay_vector::layer_names`。

### 边框下拉（Frames）
- **功能简介**：控制底图/图像裁剪边框的显示与打印。
- **UI 入口**：`Ribbon → Insert 选项卡 → Reference 面板 → 边框（LabeledDropdown，空标签，显示所选子项文本）`。
- **样式**：`LabeledDropdown`（1 行高，图标 `assets/icons/underlay_frames.svg`）。
- **触发命令**：默认 `FRAMES1`。下拉子项（`mod.rs::FRAMES_DROPDOWN`，全部）：
  - `FRAMES0` — Hide frames（隐藏边框）
  - `FRAMES1` — Display and plot frames（显示并打印边框）
  - `FRAMES2` — Display but don't plot frames（显示但不打印）
  - `FRAMES3` — `*Frames vary*`（各边框变量不一致时显示，不可选）
- **实现位置**：`src/modules/insert/mod.rs`（内联 items）；分发 `src/app/commands/styleprops.rs::dispatch_styleprops`（`FRAMES0/1/2` → `SETVAR FRAME n`，`FRAMES3` 无操作）；`FRAME` 系统变量处理 `styleprops.rs`。
- **备注**：`FRAMES3` 仅当各底层变量（IMAGEFRAME/PDFFRAME/DWFFRAME/DGNFRAME/XCLIPFRAME…）取值不一致时展示。

### 捕捉底图下拉（Snap to Underlays）
- **功能简介**：开关对底图（矢量底图）的对象捕捉。
- **UI 入口**：`Ribbon → Insert 选项卡 → Reference 面板 → 捕捉底图（LabeledDropdown，标签“Snap to Underlays”）`。
- **样式**：`LabeledDropdown`（图标 `assets/icons/snap_underlays.svg`）。
- **触发命令**：默认 `UOSNAP1`。下拉子项（`mod.rs::UOSNAP_DROPDOWN`，全部）：
  - `UOSNAP1` — Snap to Underlays ON（开启）
  - `UOSNAP0` — Snap to Underlays OFF（关闭）
- **实现位置**：`src/modules/insert/mod.rs`（内联 items）；分发 `src/app/commands/styleprops.rs::"UOSNAP0"/"UOSNAP1"` → `SETVAR UOSNAP 0|1`；系统变量 `UOSNAP` 处理 `styleprops.rs`（同时写 PDFOSNAP/DWFOSNAP/DGNOSNAP，不一致时读为 2）。
- **备注**：`UOSNAP` 一并设置 PDF/DWF/DGN 三类底图 OSNAP；单类变量为 `PDFOSNAP`/`DWFOSNAP`/`DGNOSNAP`。

---

## 二、Block 面板（Ribbon → Insert 选项卡 → Block 面板）

面板布局（`src/modules/insert/mod.rs`）：`LargeTool(Block Palette)`、`LargeTool(Insert Block)`、`Tool(Create Block)`、`Tool(Edit Block)`、`Tool(Set Base Point)`。

### 块选项板（Block Palette）
- **功能简介**：打开/关闭停靠的“Insert Block”选项板，以缩略图网格浏览并插入块。
- **UI 入口**：`Ribbon → Insert 选项卡 → Block 面板 → 块选项板（LargeTool，标签“Block Palette”）`。
- **样式**：`LargeTool`（图标 `assets/icons/blocks/insert.svg`）。
- **触发命令**：`BLOCKPALETTE`（别名 `BLOCKSPALETTE`）。
- **实现位置**：`src/modules/insert/mview_block.rs::tool()`（id `BLOCKPALETTE`）；分发 `src/app/commands/blocks.rs::"BLOCKPALETTE" | "BLOCKSPALETTE"`；面板 `src/ui/window/block_palette.rs::view`、`BlockPalette`、`BlockPaletteMsg`。
- **备注**：面板头部含搜索框（“Search blocks…”）、 “Insert from file”（`PickFile`）、“Preview size” 循环按钮（Small→Medium→Large 三档，`PreviewSize`，每行 3/2/1 张卡片）；卡片显示块线框预览与名称（最多 12 字符省略），点击卡片发 `Insert(name)`，进入 `INSERT` 放置流程；正在放置的卡片以主色高亮（`block_card_colors`）。停靠于右侧（`PanelId::BlockPalette`），打开即展开。

### 插入块（Insert Block）
- **功能简介**：按名称插入块引用，可指定插入点、缩放与旋转，并填写属性。
- **UI 入口**：`Ribbon → Insert 选项卡 → Block 面板 → 插入块（LargeTool，标签“Insert Block”）`。
- **样式**：`LargeTool`（图标 `assets/icons/blocks/insert.svg`）。
- **触发命令**：`INSERT`（别名 `I`）。
- **实现位置**：`src/modules/insert/insert_block.rs::tool()`、`InsertBlockCommand`、`BlockPicker`（`src/modules/insert/picker.rs`）；分发 `src/app/commands/blocks.rs::"INSERT"`、`ranked_block_names`、`block_usage_snapshot`。
- **备注**：块名步增量搜索（`on_live_input`），提示 `INSERT  Enter block name:` 及 `[n of m — type to search]` / `"needle" [n matches]`；无块时报 “No user-defined blocks found in this drawing.”。插入点步支持 `Scale`/`Rotate` 关键字与对话框锁定名称路径（`new_for_block`，用于粘贴为块预览）。常量/预置属性自动填充（`advance_automatic_attributes`）；`ATTREQ`/`ATTDIA` 系统变量控制属性填写。

### 创建块（Create Block）
- **功能简介**：把选定对象定义成块（可重定义），支持保留/转换/删除对象模式。
- **UI 入口**：`Ribbon → Insert 选项卡 → Block 面板 → 创建块（Tool，标签“Create Block”）`。
- **样式**：`Tool`（1 行小图标，图标 `assets/icons/blocks/block.svg`）。
- **触发命令**：`BLOCK`（别名 `B`/`BMAKE`）。
- **实现位置**：`src/modules/insert/create_block.rs::tool()`、`CreateBlockCommand`、`BlockOnScreenCommand`、`BlockPickBasePointCommand`；分发 `src/app/commands/blocks.rs::"BLOCK" | "BMAKE"`（对话框）、`-BLOCK`/`-BMAKE`（命令行）。
- **备注**：打开“Block Definition”对话框（`src/ui/window/block_definition.rs::view_window`）：名称、基点（可屏幕指定）、对象（可屏幕指定，含 Pick/Select objects/Quick Select）、对象模式 Retain/Convert to block/Delete、Annotative、Match orientation、Scale uniformly、Allow exploding、Block unit、Description、Hyperlink。重名时提示重定义确认弹层。名称非法字符 `\ / : * ? " < > | = \`` 被拒；匿名 `*` 前缀被拒（`create_block_with_options`）。

### 编辑块（Edit Block / BEDIT）
- **功能简介**：进入块编辑空间编辑块定义。
- **UI 入口**：`Ribbon → Insert 选项卡 → Block 面板 → 编辑块（Tool，标签“Edit Block”）`。
- **样式**：`Tool`（图标 `assets/icons/edit_block.svg`）。
- **触发命令**：`BEDIT`。
- **实现位置**：`src/modules/insert/edit_block.rs::tool()`；块编辑会话与保存/放弃按钮 `src/modules/draw/modify/block_edit.rs`（`BEDIT_SAVE`/`BEDIT_DISCARD`）。
- **备注**：编辑空间激活时右侧边栏出现 Save Block / Discard Block Edit 按钮。

### 设置基点（Set Base Point）
- **功能简介**：设置当前空间（模型/图纸）的插入基点。
- **UI 入口**：`Ribbon → Insert 选项卡 → Block 面板 → 设置基点（Tool，标签“Set Base Point”）`。
- **样式**：`Tool`（图标 `assets/icons/base_point.svg`）。
- **触发命令**：`BASE`。
- **实现位置**：`src/modules/insert/base_point.rs::tool()`、`BaseCommand`；分发 `src/app/commands/blocks.rs::"BASE"`（交互拾点）与 `"BASE <x> <y> [z]"`（直接设置）。
- **备注**：提示 `BASE  Specify base point:`；直接形式写入 `model_space_insertion_base` 或 `paper_space_insertion_base`，并输出 “Base point (x, y, z) set for model/paper space.”；参数不足报 `Usage: BASE <x> <y> [z]`。

---

## 三、Attributes 面板（Ribbon → Insert 选项卡 → Attributes 面板）

面板布局（`src/modules/insert/mod.rs`）：`LargeTool(Define Attributes)`、`LargeTool(Edit Attribute)`、`Tool(Manage)`、`Tool(Synchronize)`。

### 定义属性（Define Attributes）
- **功能简介**：创建属性定义（标签、提示、默认值、文字选项）。
- **UI 入口**：`Ribbon → Insert 选项卡 → Attributes 面板 → 定义属性（LargeTool，标签“Define\nAttributes”）`。
- **样式**：`LargeTool`（两行标签，图标 `assets/icons/attdef.svg`）。
- **触发命令**：`ATTDEF`。
- **实现位置**：`src/modules/insert/attdef.rs::tool()`（id `ATTDEF`）；命令实现 `src/modules/draw/draw/attdef.rs::AttdefCommand::with_text_defaults`；分发 `src/app/commands/draw.rs::"ATTDEF"`。
- **备注**：使用当前文字默认值（高度、样式、宽度因子、倾斜角，来自 `current_text_defaults`）。

### 编辑属性（Edit Attribute）
- **功能简介**：编辑块引用上的属性；选中带属性块时打开增强属性编辑器，否则进入选择与编辑流程。
- **UI 入口**：`Ribbon → Insert 选项卡 → Attributes 面板 → 编辑属性（LargeTool，标签“Edit\nAttribute”）`。
- **样式**：`LargeTool`（两行标签，图标 `assets/icons/attedit.svg`）。
- **触发命令**：`ATTEDIT`（命令行形式 `-ATTEDIT`，另一别名 `ATE`）。
- **实现位置**：`src/modules/insert/attedit.rs::tool()`；分发 `src/app/commands/inquiry.rs::"ATTEDIT"` → `open_attedit_dialog`（`src/app/commands/inquiry.rs`）；选择流程 `src/modules/draw/modify/attedit.rs::AtteditCommand`；命令行列表/快速设值 `src/app/commands/draw.rs`（`ATTEDIT <tag> <value>` 与 `-ATTEDIT`）。
- **备注**：增强属性编辑器窗口 `src/ui/window/attribute_editor.rs::view_window`，三个选项卡：Attribute（标签/提示/值列表 + 值编辑框）、Text Options（文字样式、对正、高度、旋转、宽度因子、倾斜角、Backwards、Upside down）、Properties（图层、线型、颜色、线宽）；顶部工具栏显示 “Block: <name>” 与 Apply。命令行 `-ATTEDIT` 列出选中 Insert 的属性；`ATTEDIT <tag> <value>` 快速设值。

### 管理属性（Manage / Block Attribute Manager）
- **功能简介**：块属性管理器入口（据源码路由到属性编辑器）。
- **UI 入口**：`Ribbon → Insert 选项卡 → Attributes 面板 → 管理（Tool，标签“Manage”）`。
- **样式**：`Tool`（图标 `assets/icons/attman.svg`）。
- **触发命令**：`ATTMAN`（别名 `BATTMAN`）。
- **实现位置**：`src/modules/insert/attman.rs::tool()`；分发 `src/app/commands/blocks.rs::"ATTMAN" | "BATTMAN"` → `open_attedit_dialog`。
- **备注**：据源码，`ATTMAN`/`BATTMAN` 不做命令行清单，直接路由到属性编辑器（ATTEDIT）：编辑所选块的属性，或先选块。

### 同步属性（Synchronize / ATTSYNC）
- **功能简介**：把块的所有引用与块当前的属性定义对齐：删除已不存在的标签属性、补齐新增的定义（保留仍存在属性的值）。
- **UI 入口**：`Ribbon → Insert 选项卡 → Attributes 面板 → 同步（Tool，标签“Synchronize”）`。
- **样式**：`Tool`（图标 `assets/icons/attsync.svg`）。
- **触发命令**：`ATTSYNC`（可带 `ATTSYNC <block name>`）。
- **实现位置**：`src/modules/insert/attsync.rs::tool()`；分发 `src/app/commands/blocks.rs::"ATTSYNC"`（`ValuePromptCommand` 提问块名）与 `"ATTSYNC <block>"`（执行同步）。
- **备注**：提示 `ATTSYNC  block name to sync:`；无块或块名不存在报错；同步后输出 “ATTSYNC: synchronised N insert(s) of … against M attribute definition(s).”。

---

## 四、Import 面板（Ribbon → Insert 选项卡 → Import 面板）

面板布局（`src/modules/insert/mod.rs`）：`LargeTool(Open)`、`LargeTool(Land XML)`。

### 导入 OBJ（Import OBJ / Open）
- **功能简介**：通过打开文件对话框导入图形/模型文件（OBJ 等）。
- **UI 入口**：`Ribbon → Insert 选项卡 → Import 面板 → 导入（LargeTool，标签“Open”）`。
- **样式**：`LargeTool`（图标 `assets/icons/import_obj.svg`）。
- **触发命令**：`OPEN`。
- **实现位置**：`src/modules/insert/open_obj.rs::tool()`（`ModuleEvent::OpenFileDialog`）；分发 `src/app/commands/fileops.rs::"OPEN"` → `Message::OpenFile`。
- **备注**：按钮事件为文件对话框而非命令名；据源码推断用于“Import OBJ”入口（图标 `import_obj.svg`）。

### 导入 LandXML（Land XML）
- **功能简介**：读取 LandXML 的 `<CgPoint>` 测量点，导入为 Point 对象。
- **UI 入口**：`Ribbon → Insert 选项卡 → Import 面板 → Land XML（LargeTool，标签“Land\nXML”）`。
- **样式**：`LargeTool`（两行标签，图标 `assets/icons/landxml.svg`）。
- **触发命令**：`LANDXMLIMPORT`。
- **实现位置**：`src/modules/insert/landxml.rs::tool()`；分发 `src/app/commands/display.rs::"LANDXMLIMPORT"`（`ValuePromptCommand` 提问路径）与 `"LANDXMLIMPORT <path>"`（读取并建点）、`parse_landxml_cgpoints`。
- **备注**：提示 `LANDXMLIMPORT  path to the .xml file:`；文本内容 “northing easting elevation” 映射为 `Point(easting, northing, elevation)`；无 `<CgPoint>` 报 “no <CgPoint> survey points found.”；成功后输出导入点数并建议 `ZOOM EXTENTS`。

---

## 五、Content 面板（Ribbon → Insert 选项卡 → Content 面板）

面板布局（`src/modules/insert/mod.rs`）：`LargeTool(Content Browser)`、`LargeTool(Design Center)`。

### 内容浏览器（Content Browser）
- **功能简介**：浏览图形中的命名内容（据源码当前以命令行列出块与图层）。
- **UI 入口**：`Ribbon → Insert 选项卡 → Content 面板 → 内容浏览器（LargeTool，标签“Content\nBrowser”）`。
- **样式**：`LargeTool`（两行标签，图标 `assets/icons/content_browser.svg`）。
- **触发命令**：`CONTENTBROWSER`。
- **实现位置**：`src/modules/insert/content_browser.rs::tool()`；分发 `src/app/commands/blocks.rs::"ADCENTER" | "CONTENTBROWSER"`。
- **备注**：据源码，当前在命令行输出 “Blocks (n): …” 与 “Layers (n): …”，替代浏览器面板。

### 设计中心（Design Center）
- **功能简介**：访问设计中心内容（据源码当前与 Content Browser 同路由，命令行列出命名内容）。
- **UI 入口**：`Ribbon → Insert 选项卡 → Content 面板 → 设计中心（LargeTool，标签“Design\nCenter”）`。
- **样式**：`LargeTool`（两行标签，图标 `assets/icons/design_center.svg`）。
- **触发命令**：`ADCENTER`。
- **实现位置**：`src/modules/insert/design_center.rs::tool()`；分发 `src/app/commands/blocks.rs::"ADCENTER" | "CONTENTBROWSER"`。
- **备注**：命令别名 `ADCENTER`；据源码与 Content Browser 共用同一分支。

---

## 六、相关对话框窗口与面板

### External References 选项板（Xref Manager）
- **功能简介**：停靠的外部参照管理器，列表/树展示引用状态、类型、大小、日期、保存路径，并提供附着、刷新、改路径等操作。
- **UI 入口**：`Ribbon → Insert 选项卡 → Reference 面板 → 底部角落启动器/命令（顶出面板）`；命令 `EXTERNALREFERENCES`；`XREF` 亦打开该选项板。
- **样式**：停靠面板（`PanelId::ExternalReferences`），标题 “External References (n)”；表格列宽默认 `DEFAULT_COL_WIDTHS = [150, 104, 76, 76, 96, 200]`，行高基准 `ROW_H=26`，字号 `FONT_SZ = ROW_H*0.42 ≈ 11px`。
- **触发命令**：`EXTERNALREFERENCES`（标签 `XREF`/`XREFSHOW` 也打开）；命令行内部操作经 `-XREF` 与 `XREF <op>`。
- **实现位置**：`src/modules/insert/xref_cmd.rs::XrefCommand`；面板 `src/ui/window/xref_manager.rs::XrefManagerPanel`、`view`、`XrefPaletteOp`、`row_menu_for`；分发 `src/app/commands/blocks.rs::"EXTERNALREFERENCES"`、`"XREF"`、`"-XREF"`；`src/app/commands/xref_attach.rs::list_xrefs`。
- **备注**：工具栏含 `Attach DWG ▾`（子项 Attach Image / Embed Image (in drawing) / Attach PDF）、`Refresh ▾`（子项 Reload All References）、`Change Path ▾`（Make Absolute / Make Relative / Remove Path / Select New Path / Find and Replace）、`Help`，以及 `List`/`Tree` 视图切换。行右键菜单：Open、Attach…、Unload、Reload、Detach、Change Path Type（子菜单：Make Absolute / Make Relative / Remove Path）、Select New Path、Find and Replace…。web 构建只读，变更操作禁用。`-XREF` 命令选项：`?/Bind/Detach/Path/pathType/Unload/Reload/Overlay/Attach/Show`（pathType：Full/Relative/None）。

### Reference Manager 帮助窗口（Xref Help）
- **功能简介**：说明 Reference Manager 在 web 构建下的工作方式与限制，并提供问题反馈链接。
- **UI 入口**：`External References 选项板 → 工具栏 Help 按钮`（`Message::XrefHelpOpen`）。
- **样式**：模态窗口，固定宽 460，标题 “Reference Manager — Web”。
- **触发命令**：无（按钮）。
- **实现位置**：`src/ui/window/xref_help.rs::view_window`、`XREF_ISSUES_URL`。
- **备注**：提供 “Report an issue” 按钮打开 `https://github.com/HakanSeven12/OpenCADStudio/issues`。

### Attach External Reference 对话框（Xref Attach）
- **功能简介**：选择附着对象后的选项：名称、参照类型（Attachment/Overlay）、缩放、插入点、路径类型、旋转、块单位与预览。
- **UI 入口**：`XATTACH`/`ATTACH` 选择文件后自动打开（`Message::XrefAttach`）。
- **样式**：模态对话框，三个分组列（Preview/Reference Type、Scale/Insertion point、Path type/Rotation/Block Unit），支持 “Show/Hide Details”。
- **触发命令**：`XATTACH`、`ATTACH`（命令行 `-XREF Attach/Overlay`）。
- **实现位置**：`src/ui/window/xref_attach.rs::view_window`、`XrefAttachState`、`XrefAttachMsg`、`PathTypeChoice`；状态处理 `src/app/commands/xref_attach.rs::edit_xref_attach`。
- **备注**：Path type 三选项：Full path / Relative path / No path；Scale 有 “Specify On-screen” 与 “Uniform Scale”；Insertion point / Rotation 均有 “Specify On-screen”；Details 显示 Found in / Saved path。OK 校验非法输入报 “Invalid input.”。

### Block Definition 对话框
- **功能简介**：创建或重定义块定义：名称、基点、对象、行为、单位、描述、超链接。
- **UI 入口**：`BLOCK`/`BMAKE`（Create Block 工具）。
- **样式**：模态对话框，分组卡片（Base point、Objects、Behavior、Settings、Description）。
- **触发命令**：`BLOCK`（`BMAKE`）。
- **实现位置**：`src/ui/window/block_definition.rs::view_window`、`BlockDefinitionState`、`BlockObjectMode`、`UnitChoice`；分发 `src/app/commands/blocks.rs`。
- **备注**：对象模式 Retain / Convert to block / Delete；勾选 Annotative 后可勾 Match block orientation to layout；Block unit 下拉来自 `units::all()`；Description 为多行编辑器；重名时弹出 “Block \"…\" already exists. Do you want to redefine it?” 确认层（默认焦点在 No）。

### Block Palette 停靠面板
- **功能简介**：可搜索的块缩略图网格，点击即插入块。
- **UI 入口**：`BLOCKPALETTE`/`BLOCKSPALETTE`（Block Palette 工具）。
- **样式**：停靠面板（`PanelId::BlockPalette`），标题 “Block Palette”；卡片带线框预览，标签字号 11。
- **触发命令**：`BLOCKPALETTE`。
- **实现位置**：`src/ui/window/block_palette.rs::view`、`BlockPalette`、`BlockPaletteMsg`、`PreviewSize`。
- **备注**：头部按钮 Insert from file / Preview size；空状态显示 “No blocks in this drawing” 或 “No matches”。

### Enhanced Attribute Editor 窗口（Attribute Editor）
- **功能简介**：编辑单个块引用的属性：标签/提示/值、文字选项、对象特性。
- **UI 入口**：`ATTEDIT`（选中带属性块时）、双击带属性块、`ATTMAN`/`BATTMAN`。
- **样式**：模态对话框，顶部工具栏（Block 名 + Apply），三个选项卡按钮（Attribute / Text Options / Properties）。
- **触发命令**：`ATTEDIT`。
- **实现位置**：`src/ui/window/attribute_editor.rs::view_window`、`AttrRow`、`AttrTab`、`JUSTIFY`；打开逻辑 `src/app/commands/inquiry.rs::open_attedit_dialog`、`open_attribute_editor`。
- **备注**：Attribute 选项卡列出 Tag/Prompt/Value；Text Options 含文字样式、对正（15 项 `JUSTIFY`）、高度、旋转、宽度因子、倾斜角、Backwards、Upside down；Properties 含图层、线型、颜色（ByLayer/ByBlock/Red/Yellow/Green/Cyan/Blue/Magenta/White/自定义）、线宽（`lw_options`）。

### Write Block 对话框（WBLOCK）
- **功能简介**：把块、选定对象或整个图形写出为外部 DWG/DXF 文件。
- **UI 入口**：`WBLOCK`/`WB`（命令输入框；工具定义 `src/modules/insert/wblock.rs::tool()` 当前 `#[allow(dead_code)]` 未挂 Ribbon）。
- **样式**：模态对话框，分组卡片（Source / Destination）。
- **触发命令**：`WBLOCK`（别名 `WB`）；命令行 `-WBLOCK`、`WBLOCK <block> | *`。
- **实现位置**：`src/ui/window/wblock.rs::view_window`、`WblockState`、`WblockSourceMode`、`WblockObjectMode`；命令 `src/modules/insert/wblock.rs::extract_block_to_doc`、`extract_entities_to_doc_with_base`、`WblockPickBasePointCommand`；分发 `src/app/commands/blocks.rs::"WBLOCK" | "WB"`、`-WBLOCK`。
- **备注**：Source 三模式：Block（下拉选块）/ Entire drawing / Objects（Retain / Convert to block / Delete from drawing）；Base point 可 Pick point；Destination 含文件名路径、“…” 浏览按钮与 Insert units 下拉；默认路径 `<图档目录>/new_block.dwg`。命令行 `WBLOCK *` 用当前选择集导出。

### PDF Attach 对话框
- **功能简介**：附着 PDF/DWF/DGN 底图的选项：名称、页面/图纸缩略图多选、插入点、缩放、旋转、路径类型、DGN 子单位。
- **UI 入口**：`PDFATTACH`/`DWFATTACH`/`DGNATTACH` 选择文件后打开（`Message::PdfAttach`）。
- **样式**：模态对话框，页面缩略图网格（约 220px 宽），页信息含尺寸。
- **触发命令**：`PDFATTACH`、`DWFATTACH`、`DGNATTACH`。
- **实现位置**：`src/ui/window/pdf_dialogs.rs::view_attach`、`PdfAttachState`、`PdfDialogMsg`、`PageThumb`、`page_thumbs`、`item_thumbs`、`click_page`；状态处理 `src/app/commands/pdf_dialogs.rs`。
- **备注**：PDF 可 Ctrl/Shift 多选页；DWF/DGN 一次一个；`Rotate`、`Specify On-screen` 复选项；`Details` 展开路径；底部按钮 Attach / Cancel / Help；DGN 有 conversion units（Master/Sub）选择并按子单位改默认缩放。注明 “Attaches a PDF file as an underlay.”（Help 文本随 kind 变化）。

### Attach Point Cloud 对话框
- **功能简介**：附着点云的选项：文件、数据摘要、插入点、缩放、旋转、路径类型、Lock、Zoom。
- **UI 入口**：`POINTCLOUDATTACH` 选择文件后打开（`Message::PointCloudAttach`）。
- **样式**：模态对话框，含预览缩略图、数据摘要（RGB/Intensity/Normals/Classification/Segmentation）、尺寸与单位信息。
- **触发命令**：`POINTCLOUDATTACH`。
- **实现位置**：`src/ui/window/pdf_dialogs.rs::view_point_cloud_attach`、`PointCloudAttachState`、`AttachMemory`；状态处理 `src/app/commands/pdf_dialogs.rs::open_point_cloud_attach_dialog`、`point_cloud_attach_ok`。
- **备注**：复选项 “Lock point cloud”“Zoom to point cloud”；插入/缩放/旋转均有 Specify On-screen；路径类型 Full/Relative/None；OK 校验数值（正数、非零）。

### Point Cloud Manager（点云管理器）
- **功能简介**：以树展示图形的点云及其区域/扫描/未分配点，逐项开关显示隐藏。
- **UI 入口**：`POINTCLOUDMANAGER`（停靠面板，右侧，打开即展开）。
- **样式**：停靠面板（`PanelId::PointCloudManager`），标题 “Point Cloud Manager”；表头 “REGIONS AND SCANS”（主色斜体加粗），带折叠/展开全部按钮；开关为复选框，混合态显示减号。
- **触发命令**：`POINTCLOUDMANAGER`（关闭 `POINTCLOUDMANAGERCLOSE`）。
- **实现位置**：`src/ui/window/pc_manager.rs::view`、`PcManager`、`PcManagerMsg`、`Row`、`Switch`、`clouds`、`switch_state`、`toggled`；状态处理 `src/app/commands/pdf_underlay.rs::update_pc_manager`。
- **备注**：行类型：Cloud / Scans / Scan(id) / Unassigned Points / Regions（Regions 未读数据，禁用置灰）；搜索框过滤并自动展开匹配节点。切换写 `hidden_scans`/`hidden_regions`，撤销标签 `POINTCLOUDMANAGER`。

### Point Cloud Color Map 对话框（点云颜色映射）
- **功能简介**：设置点云按强度/高程着色的配色方案、范围与色带，管理图形的颜色方案。
- **UI 入口**：`POINTCLOUDCOLORMAP`（选择点云或 `None`）；选中点云时可从 `_PCCOLORMAP <handle>` 打开。
- **样式**：模态对话框，Intensity / Elevation 两个选项卡，色带预览条、方案下拉、颜色数下拉、New/Delete/Rename 小按钮、Out-of-range 下拉、范围与间隔字段、Apply/OK。
- **触发命令**：`POINTCLOUDCOLORMAP`。
- **实现位置**：`src/modules/insert/pc_stylize.rs::PointCloudColorMapCommand`；窗口 `src/ui/window/pdf_dialogs.rs::view_point_cloud_color_map`、`PointCloudColorMapState`、`MapRamp`、`MapSlot`、`OutOfRange`、`blend`、`short`；状态处理 `src/app/commands/pc_colormap.rs::open_point_cloud_color_map`、`update_point_cloud_color_map`、`apply_point_cloud_color_map`。
- **备注**：颜色方案存于 `ACAD_POINTCLOUD_COLORMAP_DICT`；默认强度方案 “Spectrum”、高程方案 “Earth”；内部编码 scheme（可选 RGB/Object color/Intensity/Elevation/Normal/Classification，见下）；范围校验失败报 “Invalid range of colorized points.”。

---

## 七、点云与底图上下文工具（选择驱动的侧边工具栏）

以下工具经 `src/ui/ribbon/context_tools.rs` 在选择相应对象时出现在视口右侧边工具栏（host-only 命令名），与本选项卡 Reference 功能同源。

### 底图工具（PDF/DWF/DGN underlay）
- **功能简介**：对选中底图做单色/显示/捕捉/裁剪/取消裁剪/导入等快捷操作。
- **UI 入口**：`视口右侧边工具栏（选中底图时出现）`。
- **触发命令**：`_PDFULMONO`（单色）、`_PDFULSHOW`（显示开关）、`_PDFULSNAP`（捕捉开关，写 PDFOSNAP/DWFOSNAP/DGNOSNAP）、`_PDFULCLIP`（新建裁剪边界）、`_PDFULUNCLIP`（取消裁剪）、`_PDFULIMPORT`（导入 PDF 内容）、`EXTERNALREFERENCES`、`ULAYERS`。
- **实现位置**：`src/app/commands/pdf_underlay.rs::dispatch_pdf_underlay`；工具定义 `src/ui/ribbon/context_tools.rs::pdf_underlay_tools`。

### 外部参照工具（Xref）
- **功能简介**：对选中外部参照做编辑/打开/裁剪/取消裁剪。
- **UI 入口**：`视口右侧边工具栏（选中 Xref 时出现）`。
- **触发命令**：`_XREFEDIT`（→`REFEDIT`）、`_XREFOPEN`（→`XOPEN`）、`_XREFCLIP`（新建裁剪边界）、`_XREFUNCLIP`（删除裁剪）、`EXTERNALREFERENCES`。
- **实现位置**：`src/app/commands/pdf_underlay.rs::dispatch_pdf_underlay`；工具定义 `src/ui/ribbon/context_tools.rs::xref_tools`。

### 点云工具（Point Cloud）
- **功能简介**：对选中点云做矩形/多边形/圆形裁剪、显示/反转裁剪、取消裁剪、按样式着色、打开管理器、颜色映射等。
- **UI 入口**：`视口右侧边工具栏（选中点云时出现）`。
- **触发命令**：`_PCCROPRECT`（矩形裁剪）、`_PCCROPPOLY`（多边形裁剪）、`_PCCROPCIRC`（圆形裁剪）、`_PCCROPSHOW`（裁剪显示开关）、`_PCCROPINVERT`（反转裁剪）、`_PCUNCROP`（取消裁剪）、`POINTCLOUDSTYLIZE`（→`_PCSTYLIZE`→`_PCSTYLIZEAPPLY`）、`POINTCLOUDMANAGER`、`POINTCLOUDCOLORMAP`、`POINTCLOUDCROP`、`POINTCLOUDUNCROP`、`EXTERNALREFERENCES`。
- **实现位置**：`src/app/commands/pdf_underlay.rs::dispatch_pdf_underlay`、`stylize_point_clouds`、`edit_selected_point_clouds`；`src/ui/ribbon/context_tools.rs::point_cloud_tools`。

---

## 八、未在 Ribbon 固定面板出现但属于 Insert 模块的入口

以下命令未出现在 Insert 选项卡固定面板中，但通过命令行（`CommandRegistration` 自动补全）、右键上下文菜单或对话框触发。

### 点云样式化（POINTCLOUDSTYLIZE）
- **功能简介**：按一种样式给选中的点云着色。
- **UI 入口**：`命令输入框 → POINTCLOUDSTYLIZE`；或点云侧边工具栏按钮。
- **触发命令**：`POINTCLOUDSTYLIZE`。
- **实现位置**：`src/modules/insert/pc_stylize.rs::PointCloudStylizeCommand`、`OPTIONS`；应用 `src/app/commands/pdf_underlay.rs::stylize_point_clouds`。
- **备注**：选项（`OPTIONS`，全部）：`RGB`(R,1)、`Object color`(O,2)、`Intensity`(I,5)、`Elevation`(E,4)、`Normal`(N,3)、`Classification`(C,6)。提示 `Enter a stylization option [RGB/Object color/Intensity/Elevation/Normal/Classification] <上次>: `；缺少对应数据的点云会被点名跳过；分类着色不支持（stylization 6 → `supported=false`）。

### 点云裁剪/取消裁剪（POINTCLOUDCROP / POINTCLOUDUNCROP）
- **功能简介**：把点云裁剪到矩形/多边形/圆形，或清除全部裁剪。
- **UI 入口**：`命令输入框`；或点云侧边工具栏。
- **触发命令**：`POINTCLOUDCROP`、`POINTCLOUDUNCROP`。
- **实现位置**：`src/modules/insert/pc_crop.rs::PointCloudCropCommand`、`PointCloudExCrop`；分发 `src/app/commands/blocks.rs::"POINTCLOUDCROP" | "POINTCLOUDUNCROP"`。
- **备注**：提示 `Select point cloud:` → `Specify first corner point or [Polygon/Circular]:`（有裁剪时含 `Invert/Remove last/ON/OFF`）→ `Keep points inside or outside? [Inside/Outside] <Inside>:`。裁剪预览为青色。

### 点云颜色映射命令（POINTCLOUDCOLORMAP）
- **功能简介**：选择点云或 `None` 打开颜色映射对话框。
- **UI 入口**：`命令输入框 → POINTCLOUDCOLORMAP`。
- **触发命令**：`POINTCLOUDCOLORMAP`。
- **实现位置**：`src/modules/insert/pc_stylize.rs::PointCloudColorMapCommand`；对话框见上文。

### 点云截面/边缘提取（PCEXTRACTEDGE / PCEXTRACTCORNER / PCEXTRACTCENTERLINE / PCEXTRACTSECTION）
- **功能简介**：从点云提取边缘平面、拐角、中心线，或沿活动截面把保留侧点迹描为线。
- **UI 入口**：`命令输入框`；`PCEXTRACTSECTION` 的设置对话框 `_PCSECTIONDLG`。
- **触发命令**：`PCEXTRACTEDGE`、`PCEXTRACTCORNER`、`PCEXTRACTCENTERLINE`、`PCEXTRACTSECTION`（命令行 `-PCEXTRACTSECTION`）、`_PCSECTIONDLG`。
- **实现位置**：`src/modules/insert/pc_extract.rs::PlanesCommand`、`CenterlineCommand`、`SectionCommand`、`SectionDistanceCommand`、`settings`/`set_settings`；分发 `src/app/commands/pc_extract.rs::dispatch_pc_extract`、`open_pc_section_dialog`、`update_pc_section`；对话框 `src/ui/window/pdf_dialogs.rs::view_pc_section`、`PcSectionState`、`LineColor`。
- **备注**：截面设置对话框项：Perimeter、Max points、Layer（Use Current + 各图层）、Color（`LineColor::ALL` 9 色）、Polylines、Polyline width、Minimum line length、Connect lines tolerance（可 `SecPick` 在屏测量）、Collinear angle tolerance（0~10）、Preview result；按钮 Create；提示 “Traces the points on the kept side of a live section as lines.”。

### 图像透明度（TRANSPARENCY）
- **功能简介**：控制栅格图像是否按存储颜色显示透明像素。
- **UI 入口**：`命令输入框 → TRANSPARENCY`（选中图像后提示模式）。
- **触发命令**：`TRANSPARENCY`、`TRANSPARENCY ON`、`TRANSPARENCY OFF`、`TRANSPARENCY MODE`。
- **实现位置**：`src/modules/insert/image_transparency.rs::TransparencyCommand`。
- **备注**：提示 `Select image(s):` → `Enter transparency mode [ON/OFF] <默认>: `。

### 图像参照与嵌入（IMAGE / IMAGEATTACH / IMAGEEMBED / IMAGECLIP）
- **功能简介**：附着栅格图像、嵌入图像、裁剪图像。
- **UI 入口**：`命令输入框`；External References 工具栏 Attach ▾ 子项 Attach Image / Embed Image (in drawing)。
- **触发命令**：`IMAGE`/`IMAGEATTACH`/`IM`（附着）、`IMAGEEMBED`（嵌入）、`IMAGECLIP`（裁剪）。
- **实现位置**：`src/app/commands/draw.rs::"IMAGE"/"IMAGEATTACH"/"IM"`（→`Message::ImagePick`）、`"IMAGEEMBED"`（→`Message::ImageEmbedPick`）；裁剪 `src/modules/insert/pdf_clip.rs::PdfClipCommand::image()`，分发 `src/app/commands/blocks.rs::"IMAGECLIP"`。

### 底图/图像/点云/视口裁剪入口（CLIP / PDFCLIP / DWFCLIP / DGNCLIP / VPCLIP）
- **功能简介**：分别裁剪纸面底图、栅格图像或布局视口。
- **UI 入口**：`命令输入框`；底图/图像也可经 `CLIP` 选择对象后自动分派。
- **触发命令**：`CLIP`、`PDFCLIP`、`DWFCLIP`、`DGNCLIP`、`IMAGECLIP`、`VPCLIP`。
- **实现位置**：`src/modules/insert/pdf_clip.rs::PdfClipCommand`、`XclipCommand`（`src/modules/insert/xclip.rs`）、`MviewCommand::vpclip`；分发 `src/app/commands/blocks.rs`。

### MINSERT（阵列插入，Array Insert）
- **功能简介**：把块引用按矩形阵列一次性插入（行/列/行距/列距）。
- **UI 入口**：`命令输入框 → MINSERT`（工具定义 `src/modules/insert/minsert.rs::tool()` 当前 `#[allow(dead_code)]`，未挂 Ribbon）。
- **触发命令**：`MINSERT`。
- **实现位置**：`src/modules/insert/minsert.rs::MinsertCommand`、`ParamIdx`；分发 `src/app/commands/blocks.rs::"MINSERT"`。
- **备注**：提示块名（增量搜索）→ `Specify insertion point for "<name>":` → `Enter number of rows <n>:` → columns → row spacing → column spacing。提交单个带阵列字段的 `Insert`，渲染器按行列复制；无属性填写。

### WBLOCK（写块）
- **功能简介**：把块/选择集/整个图形写出为外部 DWG/DXF。
- **UI 入口**：`命令输入框 → WBLOCK`（对话框见上文）。
- **触发命令**：`WBLOCK`（`WB`）。
- **实现位置**：`src/modules/insert/wblock.rs`。

### XOPEN / REFEDIT / REFCLOSE / BEDIT（参照与块编辑）
- **功能简介**：打开参照、就地编辑参照、保存/放弃参照编辑、编辑块。
- **UI 入口**：`命令输入框`；外部参照侧边工具栏 `_XREFEDIT`/`_XREFOPEN`；REFEDIT/BEDIT 会话侧边按钮。
- **触发命令**：`XOPEN`、`REFEDIT`、`REFCLOSE`（`REFCLOSE_SAVE`/`REFCLOSE_DISCARD`）、`BEDIT`（`BEDIT_SAVE`/`BEDIT_DISCARD`）。
- **实现位置**：`src/app/commands/blocks.rs::"XOPEN"`；`src/modules/draw/modify/refedit.rs::refedit_tools`、`src/modules/draw/modify/block_edit.rs::block_edit_tools`。

### 底图相关系统变量（FRAME / IMAGEFRAME / PDFFRAME / DWFFRAME / DGNFRAME / XCLIPFRAME / UOSNAP）
- **功能简介**：设置边框显示与底图捕捉的底层变量。
- **UI 入口**：`命令输入框 → SETVAR`；`FRAMES0/1/2`、`UOSNAP0/1` 亦为命令。
- **触发命令**：`FRAME`、`IMAGEFRAME`、`PDFFRAME`、`DWFFRAME`、`DGNFRAME`、`XCLIPFRAME`、`UOSNAP`、`PDFOSNAP`、`DWFOSNAP`、`DGNOSNAP`。
- **实现位置**：`src/app/commands/styleprops.rs::dispatch_styleprops`、SETVAR 系统变量处理。

### RECAP（点云）
- **功能简介**：占位命令（据源码未实现）。
- **UI 入口**：`命令输入框 → RECAP`（工具定义 `src/modules/insert/recap.rs::tool()` 未挂 Ribbon）。
- **触发命令**：`RECAP`。
- **实现位置**：`src/app/commands/display.rs::"RECAP" | "SYNCPVIEWPORTS"` → 输出 “not yet implemented.”。

### 实体/大体量命令模块（solid3d_cmds）
- **功能简介**：`src/modules/insert/solid3d_cmds.rs` 提供三维实体相关命令（与本选项卡未直接关联，仅说明模块包含）。
- **实现位置**：`src/modules/insert/solid3d_cmds.rs`。

---

## 附：Insert 模块内定义但未挂入当前面板的工具

### Box / Cylinder / Sphere / Clear / Recap
- **功能简介**：`src/modules/insert/` 下定义的 `BOX`、`CYLINDER`、`SPHERE`、`CLEAR`（清空模型）、`RECAP`（点云）工具。
- **实现位置**：`src/modules/insert/box_prim.rs::tool()`、`cylinder.rs::tool()`、`sphere.rs::tool()`、`clear.rs::tool()`（`ModuleEvent::ClearModels`）、`recap.rs::tool()`。
- **备注**：这些工具未出现在 `InsertModule::ribbon_groups()` 中；据源码推断为备用/经其它入口或命令行触发。`solid3d_cmds.rs` 为相关三维命令集合。

---

（文档结束）
