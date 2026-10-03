# Ribbon「Annotate」选项卡功能需求清单

本文档反推自 `src/modules/annotate/`（全部 `.rs`）、`src/modules/draw/draw/`（`wipeout.rs`、`revcloud.rs`、`centerline.rs`、`dimcenter.rs`）、`src/ui/style/`（`textstyle.rs`、`dimstyle.rs`、`mleaderstyle.rs`、`tablestyle.rs`、`anno_object_scale.rs`、`style_list.rs`）、`src/ui/window/`（`annotation_data.rs`、`geometric_tolerance.rs`、`find_replace.rs`）以及 `src/app/commands/`（`dim.rs`、`draw.rs`、`display.rs`、`inquiry.rs`）与 `src/app/tolerance_dialog.rs`。Ribbon 面板与工具的权威布局入口是 `src/modules/annotate/mod.rs::AnnotateModule::ribbon_groups()`。

面板组的权威顺序（`src/modules/annotate/mod.rs`）：Text、Dimensions、Centerlines、Leaders、Tables、Markup、Annotation Scaling。每个面板标题（`group_title`）可点击展开该组的扩展飞出面板（`src/ui/ribbon/draw_panel.rs::group_title`、`overlay`）。

`StyleComboGroup` 的样式下拉由 `src/ui/ribbon/mod.rs::style_combo_overlay` 渲染为浮动面板（含每个样式名 + ✓ / “Manage…” 行），下方小按钮行由 `src/ui/ribbon/widgets.rs::render_large` 渲染。

---

## 一、Text 面板（Ribbon → Annotate 选项卡 → Text 面板）

面板布局：`LargeDropdown(ANNOTATE_TEXT, 默认 MTEXT)` + `StyleComboGroup(TEXT_STYLE_COMBO)`。`src/modules/annotate/mod.rs`。

### 文字下拉（Multiline Text）
- **功能简介**：创建/编辑单行文字、多行文字；下拉提供三种文字相关工具。
- **UI 入口**：`Ribbon → Annotate 选项卡 → Text 面板 → 文字（LargeDropdown，标签 “Multiline\nText”，默认 MTEXT）`。
- **样式**：`LargeDropdown`（大按钮带 ▾；图标 `mtext::ICON`，标签 “Multiline Text”）。
- **触发命令**：默认 `MTEXT`。下拉子项（`src/modules/annotate/mod.rs` 内联 items，全部）：
  - `MTEXT` — MText（多行文字，`mtext::tool()`）
  - `TEXT` — Text（单行文字，`text::tool()`）
  - `DDEDIT` — Edit Text（编辑文字，`ddedit::tool()`）
- **实现位置**：`src/modules/annotate/mod.rs`（items 列表）；`src/modules/annotate/mtext.rs::tool()`、`text.rs::tool()`、`ddedit.rs::tool()`；分发 `src/app/commands/draw.rs` 的 `"MTEXT"`、`"TEXT"`、`"DDEDIT"` 分支。

### 多行文字（MText）
- **功能简介**：以对角窗口定义边界创建多行文字对象，随后在富文本编辑器中输入内容。
- **UI 入口**：`Ribbon → Annotate 选项卡 → Text 面板 → 文字下拉 → MText`。
- **样式**：下拉子项（弹出列表行）。
- **触发命令**：`MTEXT`。
- **实现位置**：`src/modules/annotate/mtext.rs::MTextCommand`（`with_defaults`）、`tool()`；分发 `src/app/commands/draw.rs::"MTEXT"`（读取 `current_text_defaults.height` 与 `current_text_style_name`）。
- **备注**：交互步骤 FirstCorner → OppositeCorner；角点步骤提供 `Height`(`HEIGHT`)/`Justify`(`JUSTIFY`)/`Line spacing`(`LINESPACING`)/`Rotation`(`ROTATION`)/`Style`(`STYLE`)/`Width`(`WIDTH`)/`Columns`(`COLUMNS`)（`mtext.rs::point_options`）。对齐选项 TL/TC/TR/ML/MC/MR/BL/BC/BR；列模式 None/Static/Dynamic，列数、列宽、列间距（`mtext.rs::prompt`）。完成角点后发 `CmdResult::OpenMTextEditor` 打开富文本编辑器。

### 单行文字（Text）
- **功能简介**：创建单行文字，可设置对正、样式、高度与旋转角。
- **UI 入口**：`Ribbon → Annotate 选项卡 → Text 面板 → 文字下拉 → Text`。
- **样式**：下拉子项。
- **触发命令**：`TEXT`。
- **实现位置**：`src/modules/annotate/text.rs::TextCommand`（`with_defaults`）、`tool()`；分发 `src/app/commands/draw.rs::"TEXT"`。
- **备注**：起始步提示 “TEXT Current style: …, Height: …, Annotative: …”；选项 `Justify`(`J`)/`Style`(`ST`)（`text.rs::options`）。对正全部 15 项：Left/Center/Right/Aligned/Middle/Fit/TL/TC/TR/ML/MC/MR/BL/BC/BR（`text.rs::set_justification`）。Aligned/Fit 进入第二端点步；固定高度样式跳过高度步。编辑关闭后按字高自动换行续写（`text.rs::open_next_line`），回车/退出结束。

### 编辑文字（DDEDIT / Edit Text）
- **功能简介**：拾取带文字实体，就地打开其编辑器（单行文字为纯文本框，MText/多行引线为富文本编辑器）。
- **UI 入口**：`Ribbon → Annotate 选项卡 → Text 面板 → 文字下拉 → Edit Text`。
- **样式**：下拉子项。
- **触发命令**：`DDEDIT`。
- **实现位置**：`src/modules/annotate/ddedit.rs::DdeditCommand`、`tool()`；分发 `src/app/commands/draw.rs::"DDEDIT"`（若已单选一个可编辑实体则直接 `begin_text_edit`）。
- **备注**：支持 Text、属性、标注替代文字、公差（Tolerance）与 MText/MultiLeader；Leader 解析为其所标注实体。发 `CmdResult::EditTextEntity { handle }`。

### 文字样式下拉及 “Find” 按钮（Text Style / Find）
- **功能简介**：切换当前文字样式；下方 “Find” 打开查找替换，对文字对象做检索与替换。
- **UI 入口**：`Ribbon → Annotate 选项卡 → Text 面板 → 文字样式（StyleComboGroup）` 与下方小按钮 `Find`。
- **样式**：`StyleComboGroup`（样式名下拉，宽 `LARGE_W*2.3`，含当前值文本 + ▾；展开列出全部文字样式 + ✓，末尾 “Manage…”）；下方 1 行小按钮 `Find`（标签 “Find”，图标 `assets/icons/find.svg`）。
- **触发命令**：下拉选择发 `Message::RibbonStyleChanged { key: TextStyle, name }`；“Manage…” 发 `Command("STYLE")`；`Find` 发 `Command("FIND")`。
- **实现位置**：`src/modules/annotate/mod.rs`（`StyleComboGroup { style_key: StyleKey::TextStyle, combo_id: "TEXT_STYLE_COMBO", manager_cmd: Some("STYLE"), rows }`）；渲染 `src/ui/ribbon/widgets.rs::render_large`（`StyleComboGroup` 分支）、`src/ui/ribbon/mod.rs::style_combo_overlay`；命令分发 `src/app/commands/layerprops.rs::"STYLE"`、`src/app/commands/inquiry.rs::"FIND"`。
- **备注**：`STYLE` 打开文字样式管理器窗口（`src/ui/style/textstyle.rs::view_window`，选项卡 “Fonts” / “Size and Effects”，含笔划字体/系统字体列表、字高、宽度因子、倾斜角、Backward/Upside down/Vertical/Annotative）。`FIND` 打开查找替换窗口（`src/ui/window/find_replace.rs::view_window`，字段 Find/Replace with，按钮 Replace / Replace All / Find Next；搜索 Text、MText、属性定义与块属性值）。快捷键 `Ctrl+F` / `Ctrl+H` 映射到 `FIND`（`src/app/shortcuts.rs`）。命令行等价 `FIND <search> REPLACE <rep>`（仅首处）与 `FINDALL <search> REPLACE <rep>`（全部）。

---

## 二、Dimensions 面板（Ribbon → Annotate 选项卡 → Dimensions 面板）

面板布局：`LargeDropdown(ANNOTATE_DIM, 默认 DIMLINEAR)` + `StyleComboGroup(DIM_STYLE_COMBO)`。`src/modules/annotate/mod.rs`。

### 标注大下拉（Dimension）
- **功能简介**：创建九类标注：线性、对齐、角度、弧长、半径、折弯半径、直径、坐标与快速标注。
- **UI 入口**：`Ribbon → Annotate 选项卡 → Dimensions 面板 → 标注（LargeDropdown，标签 “Dimension”，默认 DIMLINEAR）`。
- **样式**：`LargeDropdown`（图标 `linear_dim::ICON`，标签 “Dimension”）。
- **触发命令**：默认 `DIMLINEAR`。下拉子项（`src/modules/annotate/mod.rs` 内联 items，全部 9 项）：
  - `DIMLINEAR` — Linear（线性标注，`linear_dim::tool()`）
  - `DIMALIGNED` — Aligned（对齐标注，`aligned_dim::tool()`）
  - `DIMANGULAR` — Angular（角度标注，`angular_dim::tool()`）
  - `DIMARC` — Arc Length（弧长标注，`arc_length_dim::tool()`）
  - `DIMRADIUS` — Radius（半径标注，`radius_dim::tool()`）
  - `DIMJOGGED` — Jogged Radius（折弯半径，`jogged_radius_dim::tool()`）
  - `DIMDIAMETER` — Diameter（直径标注，`diameter_dim::tool()`）
  - `DIMORDINATE` — Ordinate（坐标标注，`ordinate_dim::tool()`）
  - `QDIM` — Quick Dim（快速标注，`qdim::tool()`）
- **实现位置**：`src/modules/annotate/mod.rs`（items 列表）；各 `tool()`；分发 `src/app/commands/dim.rs` 的 `"DIMLINEAR"`、`"DIMALIGNED"`、`"DIMANGULAR"`、`"DIMARC"`、`"DIMRADIUS"`、`"DIMJOGGED" | "DIMJOG"`、`"DIMDIAMETER"`、`"DIMORDINATE"`、`"QDIM"` 分支。

#### 线性标注（DIMLINEAR / Linear）
- **功能简介**：测量两点在某轴向上的投影距离，可水平/垂直/旋转，也可选择对象自动取端点。
- **UI 入口**：`Ribbon → Annotate 选项卡 → Dimensions 面板 → 标注下拉 → Linear`。
- **样式**：下拉子项。
- **触发命令**：`DIMLINEAR`。
- **实现位置**：`src/modules/annotate/linear_dim.rs::LinearDimensionCommand`、`tool()`、`linear_dimension_entity`；分发 `src/app/commands/dim.rs::"DIMLINEAR"`。
- **备注**：起止点步按 Enter 进入“选择对象”模式（`needs_entity_pick`）；放置步选项 `MText`/`Text`/`Angle`/`Horizontal`/`Vertical`/`Rotated`（`linear_dim.rs::options`）；`Rotated` 输入标注线角度。支持经图纸视口测量（`measures_through_viewports`）。

#### 对齐标注（DIMALIGNED / Aligned）
- **功能简介**：测量两点真实距离，标注线平行于两点连线。
- **UI 入口**：`Ribbon → Annotate 选项卡 → Dimensions 面板 → 标注下拉 → Aligned`。
- **样式**：下拉子项。
- **触发命令**：`DIMALIGNED`。
- **实现位置**：`src/modules/annotate/aligned_dim.rs::AlignedDimensionCommand`、`aligned_dimension_entity`；分发 `src/app/commands/dim.rs::"DIMALIGNED"`。
- **备注**：放置步选项 `MText`/`Text`/`Angle`。

#### 角度标注（DIMANGULAR / Angular）
- **功能简介**：测量两线夹角、三点夹角，或圆弧/圆所张角度。
- **UI 入口**：`Ribbon → Annotate 选项卡 → Dimensions 面板 → 标注下拉 → Angular`。
- **样式**：下拉子项。
- **触发命令**：`DIMANGULAR`。
- **实现位置**：`src/modules/annotate/angular_dim.rs::AngularDimensionCommand`、`angular_two_line_entity`、`angular_three_point_entity`；分发 `src/app/commands/dim.rs::"DIMANGULAR"`。
- **备注**：放置步选项 `MText`/`Text`/`Angle`/`Quadrant`（`QUADRANT` 锁定圆弧象限，`angular_dim.rs::options`）；支持拾取直线/圆弧/圆/多段线弧段。

#### 弧长标注（DIMARC / Arc Length）
- **功能简介**：在圆弧或多段线弧段上创建弧长标注，可只标部分弧、加引线。
- **UI 入口**：`Ribbon → Annotate 选项卡 → Dimensions 面板 → 标注下拉 → Arc Length`。
- **样式**：下拉子项。
- **触发命令**：`DIMARC`。
- **实现位置**：`src/modules/annotate/arc_length_dim.rs::ArcLengthDimensionCommand`、`ArcSelection`；分发 `src/app/commands/dim.rs::"DIMARC"`。
- **备注**：放置步选项 `MText`/`Text`/`Angle`/`Partial`，弧段张角 > 90° 时追加 `Leader`/`No Leader`（`arc_length_dim.rs::options`）；`Partial` 依次取第一点、第二点限定子弧。

#### 半径标注（DIMRADIUS / Radius）
- **功能简介**：标注圆弧/圆/多段线弧段的半径，可指定文字位置。
- **UI 入口**：`Ribbon → Annotate 选项卡 → Dimensions 面板 → 标注下拉 → Radius`。
- **样式**：下拉子项。
- **触发命令**：`DIMRADIUS`。
- **实现位置**：`src/modules/annotate/radius_dim.rs::RadiusDimensionCommand`、`radial_dimension_entity`、`radius_constraint_entity`；分发 `src/app/commands/dim.rs::"DIMRADIUS"`。
- **备注**：放置步选项 `MText`/`Text`/`Angle`。

#### 折弯半径标注（DIMJOGGED / Jogged Radius）
- **功能简介**：对半径过大的圆弧/圆，用带折弯的标注线标注半径，可指定中心替代点与折弯位置。
- **UI 入口**：`Ribbon → Annotate 选项卡 → Dimensions 面板 → 标注下拉 → Jogged Radius`。
- **样式**：下拉子项。
- **触发命令**：`DIMJOGGED`（别名 `DIMJOG`）。
- **实现位置**：`src/modules/annotate/jogged_radius_dim.rs::JoggedRadiusDimensionCommand`、`tool()`；分发 `src/app/commands/dim.rs::"DIMJOGGED" | "DIMJOG"`（读取 `current_dimension_defaults` 与 `jog_angle`）。
- **备注**：步骤 SelectObject → 中心覆盖点 → 标注线位置 → 折弯位置；放置步选项 `MText`/`Text`/`Angle`。

#### 直径标注（DIMDIAMETER / Diameter）
- **功能简介**：标注圆弧/圆/多段线弧段的直径。
- **UI 入口**：`Ribbon → Annotate 选项卡 → Dimensions 面板 → 标注下拉 → Diameter`。
- **样式**：下拉子项。
- **触发命令**：`DIMDIAMETER`。
- **实现位置**：`src/modules/annotate/diameter_dim.rs::DiameterDimensionCommand`、`diameter_constraint_entity`；分发 `src/app/commands/dim.rs::"DIMDIAMETER"`。
- **备注**：放置步选项 `MText`/`Text`/`Angle`。

#### 坐标标注（DIMORDINATE / Ordinate）
- **功能简介**：标注特征点相对原点的 X 或 Y 坐标（坐标标注）。
- **UI 入口**：`Ribbon → Annotate 选项卡 → Dimensions 面板 → 标注下拉 → Ordinate`。
- **样式**：下拉子项。
- **触发命令**：`DIMORDINATE`。
- **实现位置**：`src/modules/annotate/ordinate_dim.rs::OrdinateDimCommand`、`tool()`；分发 `src/app/commands/dim.rs::"DIMORDINATE"`。
- **备注**：引线端点步选项 `Xdatum`/`Ydatum`/`MText`/`Text`/`Angle`（`ordinate_dim.rs::options`）；未指定时按落点自动判 X/Y（`is_x_type`）。

#### 快速标注（QDIM / Quick Dim）
- **功能简介**：对一组选中对象批量生成连续、阶梯、基线、坐标、半径或直径标注。
- **UI 入口**：`Ribbon → Annotate 选项卡 → Dimensions 面板 → 标注下拉 → Quick Dim`。
- **样式**：下拉子项。
- **触发命令**：`QDIM`。
- **实现位置**：`src/modules/annotate/qdim.rs::QdimCommand`、`tool()`；分发 `src/app/commands/dim.rs::"QDIM"`（传入选中实体、`dimdli` 间距与 `quick_dimension_snap_priority`）。
- **备注**：放置步模式关键字 `Continuous`(`C`)/`Staggered`(`S`)/`Baseline`(`B`)/`Ordinate`(`O`)/`Radius`(`R`)/`Diameter`(`DI`)，另 `Datum point`(`DA`)/`Edit`(`E`)/`Settings`(`SE`)（`qdim.rs::options`）；`Settings` 选 `Endpoints`/`Intersections` 作为延长线原点优先（记忆到 `quick_dimension_snap_priority`）。

### 标注样式下拉及下方两行共 9 个小按钮（Dim Style）
- **功能简介**：切换当前标注样式；下方提供快速标注、连续、基线、公差、编辑、文字编辑、打断、间距、折弯线等标注工具。
- **UI 入口**：`Ribbon → Annotate 选项卡 → Dimensions 面板 → 标注样式（StyleComboGroup，combo_id “DIM_STYLE_COMBO”）` 与下方两行小按钮。
- **样式**：`StyleComboGroup`（标注样式名下拉，展开列样式 + ✓ + “Manage…”）；下方 2 行小按钮网格（第 1 行 3 个，第 2 行 6 个）。
- **触发命令**：下拉发 `Message::RibbonStyleChanged { key: DimStyle }`；“Manage…” 发 `Command("DIMSTYLE")`。按钮（`src/modules/annotate/mod.rs` 的 `rows`，全部 9 个）：
  - 第 1 行：`QDIM` Quick Dim、`DIMCONTINUE` Continue、`DIMBASELINE` Baseline。
  - 第 2 行：`TOLERANCE` Tolerance、`DIMEDIT` Dim Edit、`DIMTEDIT` Dim Text Edit、`DIMBREAK` Dim Break、`DIMSPACE` Dim Space、`DIMJOGLINE` Jog Line。
- **实现位置**：`src/modules/annotate/mod.rs`（`StyleComboGroup { style_key: StyleKey::DimStyle, combo_id: "DIM_STYLE_COMBO", manager_cmd: Some("DIMSTYLE"), rows }`）；渲染 `src/ui/ribbon/widgets.rs::render_large`、`src/ui/ribbon/mod.rs::style_combo_overlay`；命令分发 `src/app/commands/dim.rs`、`src/app/commands/layerprops.rs::"DIMSTYLE"`。
- **备注**：`DIMSTYLE` 打开标注样式管理器（`src/ui/style/dimstyle.rs::view_window`，7 个选项卡 “Lines”/“Symbols and Arrows”/“Text”/“Fit”/“Primary Units”/“Alternate Units”/“Tolerances”，带尺寸预览画布与 Compare with 比较）。各按钮细则见下。

#### 快速标注按钮（QDIM）
- **功能简介**：见上文“快速标注（QDIM）”。
- **UI 入口**：`Ribbon → Annotate 选项卡 → Dimensions 面板 → 标注样式下拉 → 第 1 行第 1 个（Tool）`。
- **样式**：`Tool`（小图标，图标 `assets/icons/qdim.svg`，标签 “Quick Dim”）。
- **触发命令**：`QDIM`。
- **实现位置**：`src/modules/annotate/qdim.rs::tool()`。

#### 连续标注（DIMCONTINUE / Continue）
- **功能简介**：以已有线性/对齐/角度/坐标标注为基准，连续生成首尾相接的标注。
- **UI 入口**：`Ribbon → Annotate 选项卡 → Dimensions 面板 → 标注样式下拉 → 第 1 行第 2 个`。
- **样式**：`Tool`（图标 `assets/icons/dim_continue.svg`）。
- **触发命令**：`DIMCONTINUE`。
- **实现位置**：`src/modules/annotate/dim_continue.rs::DimContinueCommand`、`tool()`；分发 `src/app/commands/dim.rs::"DIMCONTINUE"`（传入 `last_created_dimension`、`dimension_continue_mode`）。
- **备注**：选项 `Select`(`S`)/`Undo`(`U`)（`dim_continue.rs::options`）；继承源标注的图层/样式/法向/文字旋转（`SourceStyle`）。

#### 基线标注（DIMBASELINE / Baseline）
- **功能简介**：以已有标注的固定端为基准，按基线间距 `DIMDLI` 递增生成平行标注。
- **UI 入口**：`Ribbon → Annotate 选项卡 → Dimensions 面板 → 标注样式下拉 → 第 1 行第 3 个`。
- **样式**：`Tool`（图标 `assets/icons/dim_baseline.svg`）。
- **触发命令**：`DIMBASELINE`。
- **实现位置**：`src/modules/annotate/dim_baseline.rs::DimBaselineCommand`、`tool()`；分发 `src/app/commands/dim.rs::"DIMBASELINE"`（传入各样式 `dimdli` 映射、当前样式名、回退间距、`dimension_continue_mode`）。
- **备注**：选项 `Select`(`S`)/`Undo`(`U`)；间距取自样式的 `DIMDLI`，英制回退 3.75、公制 0.38（`dim.rs`）。

#### 形位公差（TOLERANCE / Tolerance）
- **功能简介**：放置 GD&T（形位公差）特征控制框。
- **UI 入口**：`Ribbon → Annotate 选项卡 → Dimensions 面板 → 标注样式下拉 → 第 2 行第 1 个`。
- **样式**：`Tool`（图标 `assets/icons/tolerance.svg`，标签 “Tolerance”）。
- **触发命令**：`TOLERANCE`。
- **实现位置**：`src/modules/annotate/tolerance_cmd.rs::ToleranceCommand`、`tool()`；对话框 `src/ui/window/geometric_tolerance.rs::view_window`、`src/app/tolerance_dialog.rs::open_tolerance_dialog`/`begin_tolerance_placement`；分发 `src/app/commands/dim.rs::"TOLERANCE"` → `open_tolerance_dialog(None)`。
- **备注**：先弹结构化编辑对话框：几何特征符号（`SYMBOLS`：Straightness/Flatness/Circularity/Cylindricity/Profile of a surface/Profile of a line/Position/Concentricity/Symmetry/Parallelism/Perpendicularity/Angularity/Circular runout/Total runout）、2 个公差值（含 ⌀ 直径与材料条件）、3 个基准参考、投影高度/投影公差带、基准标识符；OK 后进入 `ToleranceCommand` 拾取插入点（发 `CmdResult::CommitAndExit`）。重新编辑已有公差走 `ToleranceDialogApply`（`tolerance_dialog.rs::apply_tolerance_dialog_edit`）。

#### 标注编辑（DIMEDIT / Dim Edit）
- **功能简介**：编辑标注文字内容、文字旋转、延长线倾斜或文字归位（Home）。
- **UI 入口**：`Ribbon → Annotate 选项卡 → Dimensions 面板 → 标注样式下拉 → 第 2 行第 2 个`。
- **样式**：`Tool`（图标 `assets/icons/dim_edit.svg`）。
- **触发命令**：`DIMEDIT`。
- **实现位置**：`src/modules/annotate/dimedit.rs::DimEditCommand`、`tool()`；分发 `src/app/commands/dim.rs::"DIMEDIT"`。
- **备注**：选项 `Home`(`H`)/`New`(`N`)/`Rotate`(`R`)/`Oblique`(`O`)（`dimedit.rs::options`）；`New` 可输入 `<>` 恢复测量值；`Oblique` 限 -85°~85°；选择集完成后发 `CmdResult::EditDimensions { handles, operation }`。

#### 标注文字编辑（DIMTEDIT / Dim Text Edit）
- **功能简介**：重定位、重对正（左/右/居中/Home）或旋转已有标注文字。
- **UI 入口**：`Ribbon → Annotate 选项卡 → Dimensions 面板 → 标注样式下拉 → 第 2 行第 3 个`。
- **样式**：`Tool`（图标 `assets/icons/dim_tedit.svg`）。
- **触发命令**：`DIMTEDIT`（别名 `DIMTED`）。
- **实现位置**：`src/modules/annotate/dimtedit.rs::DimTeditCommand`、`tool()`；分发 `src/app/commands/dim.rs::"DIMTEDIT" | "DIMTED"`。
- **备注**：先选一个标注（非标注报错），再指定新文字位置或选项 `Left`(`L`)/`Right`(`R`)/`Center`(`C`)/`Home`(`H`)/`Angle`(`A`)（`dimtedit.rs::options`）；提交发 `CmdResult::UpdateEntityAndFinish`。

#### 标注打断（DIMBREAK / Dim Break）
- **功能简介**：在标注与其它对象（或手动两点）交叠处自动/手动打断，或移除打断。
- **UI 入口**：`Ribbon → Annotate 选项卡 → Dimensions 面板 → 标注样式下拉 → 第 2 行第 4 个`。
- **样式**：`Tool`（图标 `assets/icons/dim_break.svg`）。
- **触发命令**：`DIMBREAK`。
- **实现位置**：`src/modules/annotate/dimbreak.rs::DimBreakCommand`、`tool()`；分发 `src/app/commands/dim.rs::"DIMBREAK"`。
- **备注**：首步选项 `Multiple`(`MULTIPLE`)；选交叉对象步选项 `Auto`(`AUTO`)/`Manual`(`MANUAL`)/`Remove`(`REMOVE`)，Enter 默认 Auto（`dimbreak.rs::options`）。操作类型 `DimensionBreakOperation::{Auto, Object, Manual, Remove}`。

#### 标注间距（DIMSPACE / Dim Space）
- **功能简介**：调整平行线性/对齐标注之间的间距（指定值或 Auto）。
- **UI 入口**：`Ribbon → Annotate 选项卡 → Dimensions 面板 → 标注样式下拉 → 第 2 行第 5 个`。
- **样式**：`Tool`（图标 `assets/icons/dim_space.svg`）。
- **触发命令**：`DIMSPACE`（别名 `DSPACE`）。
- **实现位置**：`src/modules/annotate/dimspace.rs::DimSpaceCommand`、`tool()`；分发 `src/app/commands/dim.rs::"DIMSPACE" | "DSPACE"`。
- **备注**：步骤 选基准标注（仅线性/对齐）→ 选其它标注（Enter 结束）→ 输入非负间距或 `Auto`(`AUTO`)，Enter 默认 Auto；间距为负报错；选择时排除锁定图层（`selection_entities_exclude_locked`）；发 `CmdResult::SpaceDimensions`。

#### 标注折弯线（DIMJOGLINE / Jog Line）
- **功能简介**：为线性/对齐标注添加或移除标注线上的折弯（jog）符号。
- **UI 入口**：`Ribbon → Annotate 选项卡 → Dimensions 面板 → 标注样式下拉 → 第 2 行第 6 个`。
- **样式**：`Tool`（图标 `assets/icons/dim_jog.svg`）。
- **触发命令**：`DIMJOGLINE`。
- **实现位置**：`src/modules/annotate/dimjogline.rs::DimJogLineCommand`、`tool()`；分发 `src/app/commands/dim.rs::"DIMJOGLINE"`。
- **备注**：选项 `Remove`(`REMOVE`)；步骤 选标注 → 指定折弯位置（Enter 用 `default_dimension_jog_position` 默认位置）；发 `CmdResult::EditDimensionJog`。仅接受线性/对齐标注。

---

## 三、Centerlines 面板（Ribbon → Annotate 选项卡 → Centerlines 面板）

面板布局：`LargeTool(CENTERMARK)`、`LargeTool(CENTERLINE)`、`Dropdown(ANNOTATE_CENTER_ASSOCIATIVITY)`。`src/modules/annotate/mod.rs`。

### 圆心标记（Center Mark）
- **功能简介**：为圆弧/圆绘制十字圆心标记（遗留中心十字），一次拾取一个对象。
- **UI 入口**：`Ribbon → Annotate 选项卡 → Centerlines 面板 → 圆心标记（LargeTool）`。
- **样式**：`LargeTool`（大按钮，图标 `assets/icons/line.svg`，标签 “Center Mark”）。
- **触发命令**：`DIMCENTER`（别名 `DCE`、`CENTERMARK`）。
- **实现位置**：`src/modules/draw/draw/dimcenter.rs::DimCenterCommand`、`tool()`（id `CENTERMARK`，标签 “Center Mark”）；`CenterMarkCommand`（id `CENTERMARK`）；分发 `src/app/commands/draw.rs::"DIMCENTER"`、`"CENTERMARK"`。
- **备注**：`DimCenterCommand` 从圆弧/圆取中心与半径，按 `radius*0.2` 半长生成两条十字线（`build_cross`），发 `CmdResult::ReplaceMany`；`CENTERMARK` 走 `CenterMarkCommand`，由 `centerline_settings()` 生成关联智能标记并 `CmdResult::CommitEntity`，可连续选择直到 Enter。注册名 `DIMCENTER`/`DCE`/`CENTERMARK`（`dimcenter.rs` inventory）。

### 中心线（CENTERLINE / Center Line）
- **功能简介**：在两条直线（或线性多段线段）之间构造关联中心线，随源对象更新。
- **UI 入口**：`Ribbon → Annotate 选项卡 → Centerlines 面板 → 中心线（LargeTool）`。
- **样式**：`LargeTool`（图标 `assets/icons/line.svg`，标签 “Center Line”）。
- **触发命令**：`CENTERLINE`。
- **实现位置**：`src/modules/draw/draw/centerline.rs::CenterLineCommand`、`tool()`；分发 `src/app/commands/draw.rs::"CENTERLINE"`（传入 `centerline_settings()`）。
- **备注**：两次拾取源线（同一段重复拾取将被忽略），发 `CmdResult::CommitAndExit`；写入 `CenterLineAssociation` 扩展数据以保持关联（`association.write`）。

### 中心关联下拉（Center Associativity）
- **功能简介**：重新关联、解除关联或重置选中中心标记/中心线对象。
- **UI 入口**：`Ribbon → Annotate 选项卡 → Centerlines 面板 → 中心关联（Dropdown，默认 CENTERREASSOCIATE）`。
- **样式**：`Dropdown`（1 行小图标 + ▾；图标 `assets/icons/line.svg`）。
- **触发命令**：默认 `CENTERREASSOCIATE`。下拉子项（`src/modules/annotate/mod.rs` 内联 items，全部 3 项）：
  - `CENTERREASSOCIATE` — Reassociate Center Object（重新关联）
  - `CENTERDISASSOCIATE` — Disassociate Center Object（解除关联）
  - `CENTERRESET` — Reset Center Object（重置）
- **实现位置**：`src/modules/annotate/mod.rs`（items 列表）；分发 `src/app/commands/draw.rs` 的 `"CENTERREASSOCIATE"`、`"CENTERDISASSOCIATE"`、`"CENTERRESET"` 分支；重新关联命令 `src/modules/draw/draw/dimcenter.rs::CenterMarkReassociateCommand`。
- **备注**：`CENTERREASSOCIATE` 若单选一个中心标记则进入 `CenterMarkReassociateCommand`（提示 “Select new arc or circle”），否则批量关联；`CENTERDISASSOCIATE` 批量解除（`set_centerline_association`/`set_center_mark_association`）；`CENTERRESET` 重置选中对象的中心线/中心标记（`reset_centerlines`/`reset_center_marks`），均发 `push_undo_snapshot`。

---

## 四、Leaders 面板（Ribbon → Annotate 选项卡 → Leaders 面板）

面板布局：`LargeDropdown(ANNOTATE_LEADER, 默认 MLEADER)` + `StyleComboGroup(MLEADER_STYLE_COMBO)`。`src/modules/annotate/mod.rs`。

### 多引线下拉（Multileader）
- **功能简介**：创建多重引线或传统引线。
- **UI 入口**：`Ribbon → Annotate 选项卡 → Leaders 面板 → 引线（LargeDropdown，标签 “Multileader”，默认 MLEADER）`。
- **样式**：`LargeDropdown`（图标 `mleader_cmd::ICON`，标签 “Multileader”）。
- **触发命令**：默认 `MLEADER`。下拉子项（`src/modules/annotate/mod.rs` 内联 items，全部 2 项）：
  - `MLEADER` — MLeader（多重引线，`mleader_cmd::tool()`）
  - `LEADER` — Leader（引线，`leader_cmd::tool()`）
- **实现位置**：`src/modules/annotate/mod.rs`（items 列表）；`src/modules/annotate/mleader_cmd.rs::tool()`、`leader_cmd.rs::tool()`；分发 `src/app/commands/dim.rs` 的 `"MLEADER"`、`"LEADER" | "QLEADER"` 分支。

### 多重引线（MLEADER / MLeader）
- **功能简介**：创建带箭头的多重引线（可多条引线线），内容可为 MText、块或空，带着陆线（landing/dogleg）。
- **UI 入口**：`Ribbon → Annotate 选项卡 → Leaders 面板 → 引线下拉 → MLeader`。
- **样式**：下拉子项。
- **触发命令**：`MLEADER`。
- **实现位置**：`src/modules/annotate/mleader_cmd.rs::MLeaderCommand`（`with_style`、`with_drawing_resources`）、`tool()`；分发 `src/app/commands/dim.rs::"MLEADER"`（传入当前 `MultiLeaderStyle`、注释倍数、块/图层/文字样式资源）。
- **备注**：首点步选项 `Arrowhead first`(`A`)/`Landing first`(`L`)/`Content first`(`C`)/`Text`(`T`)/`Select MText`(`S`)/`Options`(`O`)；`Options` 子项 `Leader type`(`LT`)/`Landing`(`LD`)/`Content type`(`CT`)/`Max points`(`M`)/`First angle`(`F`)/`Second angle`(`S`)/`Layer`(`LA`)/`Exit`(`X`)（`mleader_cmd.rs::options`）。Leader type = Straight/Spline/None；Content type = MText/Block/None；角度 Any/15/30/45/60/90；可选已有 MText 作为内容模板（`SelectMText`）。MText 内容为空时发 `CmdResult::CommitAndEditText` 打开富文本编辑器。

### 引线（LEADER / Leader）
- **功能简介**：创建传统引线（折线或样条），并在着陆点关联一段 MText 注释。
- **UI 入口**：`Ribbon → Annotate 选项卡 → Leaders 面板 → 引线下拉 → Leader`。
- **样式**：下拉子项。
- **触发命令**：`LEADER`（别名 `QLEADER`）。
- **实现位置**：`src/modules/annotate/leader_cmd.rs::LeaderCommand`（`with_defaults`）、`tool()`；分发 `src/app/commands/dim.rs::"LEADER" | "QLEADER"`。
- **备注**：≥2 点后选项 `Annotation`(`A`)/`Format`(`F`)/`Undo`(`U`)；`Format` 选项 `Spline`(`SPLINE`)/`Straight`(`STRAIGHT`)/`Arrow`(`ARROW`)/`None`(`NONE`)；注释选项 `None`(`N`)/`Mtext`(`M`)（`leader_cmd.rs::options`）。Enter 进入注释输入；空注释选项发 `CommitAndExit`，带注释发 `CommitManyAndEditText` 打开 MText 编辑器。

### 多引线样式下拉及 4 个小按钮（MLeader Style / 编辑工具）
- **功能简介**：切换当前多重引线样式；下方提供添加引线、删除引线、对齐引线、收集引线四个编辑工具。
- **UI 入口**：`Ribbon → Annotate 选项卡 → Leaders 面板 → 多引线样式（StyleComboGroup，combo_id “MLEADER_STYLE_COMBO”）` 与下方 2 行 4 个小按钮。
- **样式**：`StyleComboGroup`（多重引线样式名下拉，展开列样式 + ✓ + “Manage…”）；下方小按钮 2 行 × 2 列。
- **触发命令**：下拉发 `Message::RibbonStyleChanged { key: MLeaderStyle }`；“Manage…” 发 `Command("MLEADERSTYLE")`。按钮（`src/modules/annotate/mod.rs` 的 `rows`，全部 4 个）：
  - 第 1 行：`MLEADERADD` Add Leader、`MLEADERREMOVE` Remove Leader。
  - 第 2 行：`MLEADERALIGN` Align Leaders、`MLEADERCOLLECT` Collect Leaders。
- **实现位置**：`src/modules/annotate/mod.rs`（`StyleComboGroup { style_key: StyleKey::MLeaderStyle, combo_id: "MLEADER_STYLE_COMBO", manager_cmd: Some("MLEADERSTYLE"), rows }`）；渲染 `src/ui/ribbon/widgets.rs::render_large`、`src/ui/ribbon/mod.rs::style_combo_overlay`；命令 `src/app/commands/dim.rs`、`src/app/commands/layerprops.rs::"MLEADERSTYLE"`。
- **备注**：`MLEADERSTYLE` 打开多重引线样式管理器（`src/ui/style/mleaderstyle.rs::view_window`，4 个选项卡 “Leader Format”/“Leader Structure”/“Content”/“Block Content”，含引线预览画布与 Compare with）。

#### 添加引线（MLEADERADD / Add Leader）
- **功能简介**：为已有多重引线添加一条新的引线线。
- **UI 入口**：`Ribbon → Annotate 选项卡 → Leaders 面板 → 多引线样式下拉 → 第 1 行第 1 个`。
- **样式**：`Tool`（图标 `assets/icons/mleader_add.svg`）。
- **触发命令**：`MLEADERADD`。
- **实现位置**：`src/modules/annotate/mleader_edit.rs::MLeaderAddCommand`、`tool_add()`；分发 `src/app/commands/dim.rs::"MLEADERADD"`。
- **备注**：选多重引线 → 指定箭头位置 → 连续指定新引线点（Enter 结束）；新引线线继承模板根的连接点/方向/着陆距离及线型/颜色/线宽/箭头（`add_leader_root`）；发 `CmdResult::ReplaceEntity`。

#### 删除引线（MLEADERREMOVE / Remove Leader）
- **功能简介**：从多重引线中删除一条引线线。
- **UI 入口**：`Ribbon → Annotate 选项卡 → Leaders 面板 → 多引线样式下拉 → 第 1 行第 2 个`。
- **样式**：`Tool`（图标 `assets/icons/mleader_remove.svg`）。
- **触发命令**：`MLEADERREMOVE`。
- **实现位置**：`src/modules/annotate/mleader_edit.rs::MLeaderRemoveCommand`、`tool_remove()`；分发 `src/app/commands/dim.rs::"MLEADERREMOVE"`。
- **备注**：选多重引线 → 点击要删除的引线线附近（按 XY 距离取最近段）；当引线线仅剩一条时拒绝删除；发 `CmdResult::ReplaceEntity`。

#### 对齐引线（MLEADERALIGN / Align Leaders）
- **功能简介**：将多个多重引线沿一点到另一点的指定方向对齐。
- **UI 入口**：`Ribbon → Annotate 选项卡 → Leaders 面板 → 多引线样式下拉 → 第 2 行第 1 个`。
- **样式**：`Tool`（图标 `assets/icons/mleader_align.svg`）。
- **触发命令**：`MLEADERALIGN`。
- **实现位置**：`src/modules/annotate/mleader_edit.rs::MLeaderAlignCommand`、`tool_align()`；分发 `src/app/commands/dim.rs::"MLEADERALIGN"`。
- **备注**：先框选多重引线（Enter 结束）→ 指定对齐方向起点 → 终点；发 `CmdResult::AlignMLeaders`。

#### 收集引线（MLEADERCOLLECT / Collect Leaders）
- **功能简介**：将多个（块内容的）多重引线收集到同一位置。
- **UI 入口**：`Ribbon → Annotate 选项卡 → Leaders 面板 → 多引线样式下拉 → 第 2 行第 2 个`。
- **样式**：`Tool`（图标 `assets/icons/mleader_collect.svg`）。
- **触发命令**：`MLEADERCOLLECT`。
- **实现位置**：`src/modules/annotate/mleader_edit.rs::MLeaderCollectCommand`、`tool_collect()`；分发 `src/app/commands/dim.rs::"MLEADERCOLLECT"`。
- **备注**：先框选多重引线（Enter 结束）→ 指定收集位置；发 `CmdResult::CollectMLeaders`。

---

## 五、Tables 面板（Ribbon → Annotate 选项卡 → Tables 面板）

面板布局：`LargeTool(TABLE)` + `StyleComboGroup(TABLE_STYLE_COMBO)`。`src/modules/annotate/mod.rs`。

### 表格（Table）
- **功能简介**：打开“插入表格”对话框并按设定创建带样式的空表格。
- **UI 入口**：`Ribbon → Annotate 选项卡 → Tables 面板 → 表格（LargeTool）`。
- **样式**：`LargeTool`（图标 `assets/icons/table.svg`，标签 “Table”）。
- **触发命令**：`TABLE`。
- **实现位置**：`src/modules/annotate/table_cmd.rs::TableCommand`（`with_style`、`configured`）、`tool()`；对话框 `src/ui/window/annotation_data.rs::table_insert_view`；分发 `src/app/commands/dim.rs::"TABLE"` → `open_table_insert()`。
- **备注**：对话框字段（`TableInsertState`）：表格样式选择、插入来源（空表 / 数据链接 / 数据提取，`TableSource`）、插入方式（指定插入点 / 指定窗口，`TableInsertion`）、列数、数据行数、列宽、行高、前三行单元样式（Title/Header/Data），以及 Preview 复选框。命令行交互（未走对话框时）：列数 → 数据行数 → 列宽 → 行高 → 插入方式 `Point`/`Window` → 插入点或两角窗口；会话默认记忆（`saved_defaults`）。默认 3 列、4 数据行、列宽 2.0、行高 0.5。单元格拾取/编辑另有 `TABLEDIT`（`TableditCommand`、`TableCellEditCommand`），空 Enter 退出、`/n` 或 `\n` 换行、锁定/只读单元格不可编辑。

### 表格样式下拉及 2 个小按钮（Table Style / Data）
- **功能简介**：切换当前表格样式；下方提供数据提取与数据链接工具。
- **UI 入口**：`Ribbon → Annotate 选项卡 → Tables 面板 → 表格样式（StyleComboGroup，combo_id “TABLE_STYLE_COMBO”）` 与下方 1 行 2 个小按钮。
- **样式**：`StyleComboGroup`（表格样式名下拉，展开列样式 + ✓ + “Manage…”）；下方小按钮 1 行 2 个。
- **触发命令**：下拉发 `Message::RibbonStyleChanged { key: TableStyle }`；“Manage…” 发 `Command("TABLESTYLE")`。按钮（`src/modules/annotate/mod.rs` 的 `rows`，全部 2 个）：
  - `DATAEXTRACTION` — Extract Data（提取数据，`data_extract::tool()`，标签 “Extract\nData”）
  - `DATALINK` — Link Data（链接数据，`data_link::tool()`，标签 “Link\nData”）
- **实现位置**：`src/modules/annotate/mod.rs`（`StyleComboGroup { style_key: StyleKey::TableStyle, combo_id: "TABLE_STYLE_COMBO", manager_cmd: Some("TABLESTYLE"), rows }`）；渲染 `src/ui/ribbon/widgets.rs::render_large`、`src/ui/ribbon/mod.rs::style_combo_overlay`；命令 `src/app/commands/layerprops.rs::"TABLESTYLE"`。
- **备注**：`TABLESTYLE` 打开表格样式管理器（`src/ui/style/tablestyle.rs::view_window`，选项卡 “General”/“Data Row”/“Header Row”/“Title Row”，可编辑单元文字样式/字高/文字色/填充色/对齐/数据类型/单位/格式，6 边（左/右/上/下/内横/内竖）的边框类型/线宽/颜色/间距/隐藏，含表格预览画布与 Compare with）。

#### 提取数据（DATAEXTRACTION / Extract Data）
- **功能简介**：打开数据提取向导，从图形对象提取特性并生成表格或输出到外部文件。
- **UI 入口**：`Ribbon → Annotate 选项卡 → Tables 面板 → 表格样式下拉 → 第 1 行第 1 个`。
- **样式**：`Tool`（图标 `assets/icons/data_extract.svg`，标签 “Extract Data”）。
- **触发命令**：`DATAEXTRACTION`（别名 `EATTEXT`、`ATTEXT`）。
- **实现位置**：`src/modules/annotate/data_extract.rs::tool()`；向导 `src/ui/window/annotation_data.rs::data_extraction_view`、`DataExtractionState`；分发 `src/app/commands/inquiry.rs::"DATAEXTRACTION" | "EATTEXT" | "ATTEXT"` → `open_data_extraction()`。
- **备注**：8 页向导（`ExtractionPage`）：Begin（新建/以先前提取为模板/编辑现有，含设置路径）、Source（当前图形/当前图形选定对象/图形文件夹，含当前图形、子文件夹、添加图形/文件夹、清除）、Objects、Properties、Refine（合并相同行、显示计数/名称列）、Output（插入表格/输出外部文件）、Table Style（样式、标题）、Finish；导航 Back/Cancel/Next/Finish。完成后由 `DataLinkPlaceCommand::unlinked` 于插入点放置表格。

#### 链接数据（DATALINK / Link Data）
- **功能简介**：管理指向外部表格数据（如 CSV）的持久数据链接，并可插入链接表格。
- **UI 入口**：`Ribbon → Annotate 选项卡 → Tables 面板 → 表格样式下拉 → 第 1 行第 2 个`。
- **样式**：`Tool`（图标 `assets/icons/data_link.svg`，标签 “Link Data”）。
- **触发命令**：`DATALINK`（命令行 `DATALINK <path.csv>` 直接创建）。
- **实现位置**：`src/modules/annotate/data_link.rs::DataLinkPlaceCommand`（`new`/`existing`/`unlinked`）、`tool()`；管理器 `src/ui/window/annotation_data.rs::data_link_view`、`DataLinkManagerState`；分发 `src/app/commands/display.rs::"DATALINK"`（`open_data_link_manager(false)`）与 `"DATALINK ..."` 分支。
- **备注**：管理器左侧列出 “Spreadsheet Links” 与 “Create a New Data Link”；右侧编辑器字段：数据链接名称、文件路径（带 “…” 浏览）、路径类型（Full/Relative/FileName）、链接选项（Entire sheet/Named range/Cell range）、工作表/命名范围/范围、More options（允许写入源文件、使用源格式、保持与源格式同步）、保存后插入表格；非编辑态显示选中链接的 Details 与 Preview，操作按钮 Edit/Delete/Insert Table/Close。已存在链接的表格每个已填充单元被锁定并指向同一链接（`CellStateFlags::LINKED`）。放置时发 `on_point` → `CommitAndExit`。

---

## 六、Markup 面板（Ribbon → Annotate 选项卡 → Markup 面板）

面板布局：`LargeTool(WIPEOUT)`、`LargeTool(REVCLOUD)`。`src/modules/annotate/mod.rs`。

### 区域覆盖（Wipeout）
- **功能简介**：绘制多边形遮罩或从闭合多段线派生遮罩，遮住其下方的图形。
- **UI 入口**：`Ribbon → Annotate 选项卡 → Markup 面板 → 区域覆盖（LargeTool）`。
- **样式**：`LargeTool`（图标 `assets/icons/wipeout.svg`，标签 “Wipeout”）。
- **触发命令**：`WIPEOUT`。
- **实现位置**：`src/modules/draw/draw/wipeout.rs::WipeoutCommand`、`tool()`、`wipeout_from_polyline`、`wipeout_frame_mode`；分发 `src/app/commands/draw.rs::"WIPEOUT" | "WO" | starts_with("WIPEOUT ")`。
- **备注**：首步选项 `Frames`(`F`)/`Polyline`(`P`)（`wipeout.rs::options`）；绘点步选项 `Close`(`C`)/`Undo`(`U`)；`Frames` 步选项 `Off`(`OFF`)/`On`(`ON`)/`Display but not plot`(`D`) 发 `WIPEOUTFRAME <mode>`；`Polyline` 步选择闭合平面零宽直线多段线，随后询问 `Erase source polyline? [Yes/No] <No>`。预览为青色（`Wipeout::CYAN`，屏幕固定宽 2.0）。

### 修订云线（Revision Cloud）
- **功能简介**：创建或修改弧凸修订云线，支持矩形、多边形、自由手绘、对象转换与修改。
- **UI 入口**：`Ribbon → Annotate 选项卡 → Markup 面板 → 修订云线（LargeTool）`。
- **样式**：`LargeTool`（图标 `assets/icons/revcloud.svg`，标签 “Rev Cloud”）。
- **触发命令**：`REVCLOUD`（别名 `REVCLOUD_RECTANGULAR`、`REVCLOUD_POLYGONAL`、`REVCLOUD_FREEHAND`）。
- **实现位置**：`src/modules/draw/draw/revcloud.rs::RevCloudCommand`、`tool()`；分发 `src/app/commands/draw.rs::"REVCLOUD" | "REVCLOUD_RECTANGULAR" | ...`。
- **备注**：通用选项 `Arc length`(`A`)/`Object`(`O`)/`Rectangular`(`R`)/`Polygonal`(`P`)/`Freehand`(`F`)/`Style`(`S`)/`Modify`(`M`)（`revcloud.rs::common_options`）。`Arc length` 输入弧长（默认 1.0，记忆）；`Style` 选项 `Normal`(`N`)/`Calligraphy`(`C`)；`Object` 选闭合的圆/椭圆/多段线/样条并询问反向 `Reverse arc direction? [Yes/No] <No>`；`Modify` 选云线后重绘替换（`ModifyDraw`/`ModifyErase`），修改步选项 `Undo`(`U`)/`First point`(`F`)；创建模式记忆（`LAST_CREATION`/`LAST_STYLE`）。

---

## 七、Annotation Scaling 面板（Ribbon → Annotate 选项卡 → Annotation Scaling 面板）

面板布局：`LabeledDropdown(ANNOTATION_CURRENT_SCALE, 默认 “OBJECTSCALE ADD”)`、`Tool(SCALELISTEDIT)`、`Tool(OBJECTSCALE)`、`Tool(ANNORESET)`。`src/modules/annotate/mod.rs`。

### 当前比例下拉（Add/Delete Current Scale）
- **功能简介**：把当前注释比例加入或从选中对象的比例表示中移除。
- **UI 入口**：`Ribbon → Annotate 选项卡 → Annotation Scaling 面板 → 当前比例（LabeledDropdown，标签 “Add Current Scale”，默认 “OBJECTSCALE ADD”）`。
- **样式**：`LabeledDropdown`（1 行高带文字标签；图标 `assets/icons/add_scale.svg`）。
- **触发命令**：默认 `OBJECTSCALE ADD`。下拉子项（`src/modules/annotate/mod.rs` 内联 items，全部 2 项）：
  - `OBJECTSCALE ADD` — Add Current Scale（添加当前比例）
  - `OBJECTSCALE DELETE` — Delete Current Scale（删除当前比例）
- **实现位置**：`src/modules/annotate/mod.rs`（items 列表）；分发 `src/app/commands/display.rs` 中 `"OBJECTSCALE ADD"`/`"OBJECTSCALE DELETE"`（action 分支，取 `creation_annotation_scale_handle`、按需进 `AnnotationScaleSelectionCommand`）；命令前端 `src/modules/annotate/annotation_scale.rs::AnnotationScaleSelectionCommand`。
- **备注**：无预选时先进入选择收集（提示 “OBJECTSCALE  Select annotative objects:”，完成发 `CmdResult::Relaunch(action, handles)`）；有预选则直接执行。ADD 为支持注释上下文的对象创建当前比例上下文并置为 annotative（`create_annotation_context` + `set_entity_annotative`）；DELETE 移除当前比例上下文；均 `push_undo_snapshot`。

### 比例列表（Scale List）
- **功能简介**：列出/添加/删除图形注释比例（命令行形式）。
- **UI 入口**：`Ribbon → Annotate 选项卡 → Annotation Scaling 面板 → Scale List（Tool）`。
- **样式**：`Tool`（图标 `assets/icons/scale_list.svg`，标签 “Scale List”）。
- **触发命令**：`SCALELISTEDIT`。
- **实现位置**：`src/modules/annotate/mod.rs`（内联 `ToolDef { id: "SCALELISTEDIT", event: Command("SCALELISTEDIT") }`）；分发 `src/app/commands/display.rs::"SCALELISTEDIT"` 与 `starts_with("SCALELISTEDIT ")`。
- **备注**：无参列出全部比例；`SCALELISTEDIT ADD 1:50` 添加（比例为 paper:drawing，二者须 > 0）；`SCALELISTEDIT DELETE 1:50` 删除（不能删当前比例）；关键字 `Add`(`ADD`)/`Delete`(`DELETE`)，删除也接受 `REMOVE`。使用 `Scene::add_scale`/`remove_scale`/`scale_list`。

### 添加/删除比例（Add/Delete Scales）
- **功能简介**：打开“注释对象比例”对话框，为单个对象逐项勾选/取消其携带表示的比例。
- **UI 入口**：`Ribbon → Annotate 选项卡 → Annotation Scaling 面板 → Add/Delete Scales（Tool）`。
- **样式**：`Tool`（图标 `assets/icons/add_scale.svg`，标签 “Add/Delete Scales”）。
- **触发命令**：`OBJECTSCALE`。
- **实现位置**：`src/modules/annotate/mod.rs`（内联 `ToolDef { id: "OBJECTSCALE", event: Command("OBJECTSCALE") }`）；分发 `src/app/commands/display.rs::"OBJECTSCALE"` → `Message::AnnoObjectScaleOpen`；对话框 `src/ui/style/anno_object_scale.rs::view_window`、`Message::AnnoObjectScaleToggle`。
- **备注**：对话框显示 “Object: <标签>”，逐行列出每个比例名 + paper:drawing 比值 + ✓（成员）；点击行切换成员（添加会在该比例合成 per-scale 上下文 `AcDb*ObjectContextData`，移除则删除）；关闭按钮发 `Message::CloseModal`。

### 同步比例位置（Sync Scale Positions / ANNORESET）
- **功能简介**：把选中对象的所有备用比例表示位置重置为当前注释比例下可见表示的位置。
- **UI 入口**：`Ribbon → Annotate 选项卡 → Annotation Scaling 面板 → Sync Scale Positions（Tool）`。
- **样式**：`Tool`（图标 `assets/icons/sync.svg`，标签 “Sync Scale Positions”）。
- **触发命令**：`ANNORESET`。
- **实现位置**：`src/modules/annotate/mod.rs`（内联 `ToolDef { id: "ANNORESET", event: Command("ANNORESET") }`）；分发 `src/app/commands/display.rs::"ANNORESET"`；选择前端 `src/modules/annotate/annotation_scale.rs::AnnotationScaleSelectionCommand`。
- **备注**：无预选时进入选择收集；仅对拥有 >1 个比例成员的对象处理（`object_scale_memberships`），调用 `reset_annotation_context_positions`；无可处理对象时提示 “no selected object has alternate scale representations.”。

---

## 附：其它经命令行/对话框触发、未在面板固定出现的标注相关工具

以下命令在 `src/app/commands/` 中分发，但不在 Annotate 选项卡固定面板中；入口为命令输入框（`CommandRegistration` 自动补全）。

### 文字编辑模式（TEXTEDIT / TEDIT / TEXTEDITMODE）
- **功能简介**：就地编辑文字/多行文字/标注文字；可切换单次/多次模式。
- **UI 入口**：`命令输入框 → TEXTEDIT`。
- **触发命令**：`TEXTEDIT`（`TEDIT`）、`TEXTEDITMODE`。
- **实现位置**：`src/modules/annotate/textedit.rs::TexteditCommand`、`TexteditmodeCommand`、`parse_texteditmode`；分发 `src/app/commands/draw.rs::"TEXTEDIT" | "TEDIT"`、`"TEXTEDITMODE"`。
- **备注**：选项 `Undo`(`U`)/`Mode`(`M`)；模式 `Single`/`Multiple`（参数映射 `0/m/multiple/false` ↔ Multiple，`1/s/single/true` ↔ Single）。

### 文字对正（JUSTIFYTEXT）
- **功能简介**：修改选中文字/多行文字的对接方式。
- **UI 入口**：`命令输入框 → JUSTIFYTEXT`。
- **触发命令**：`JUSTIFYTEXT`。
- **实现位置**：`src/app/commands/dim.rs::"JUSTIFYTEXT"` 与 `starts_with("JUSTIFYTEXT ")`；`CommandRegistration` 见 `src/app/commands/mod.rs`。
- **备注**：选项 Left/Center/Right/Middle/Aligned/Fit；对 Text 改水平/垂直对齐，对 MText 改 `attachment_point`。

### 文字大小写（TCASE）
- **功能简介**：将选中文字/多行文字转换为大写/小写/句首大写/标题大写。
- **UI 入口**：`命令输入框 → TCASE`。
- **触发命令**：`TCASE`。
- **实现位置**：`src/app/commands/dim.rs::"TCASE"` 与 `starts_with("TCASE ")`。
- **备注**：选项 Upper/Lower/Sentence/Title（`sentence_case`、`title_case`）。

### 文字遮罩（TEXTMASK）
- **功能简介**：为选中文字按其包围盒生成 wipeout 遮罩，并把文字置于其前。
- **UI 入口**：`命令输入框 → TEXTMASK`。
- **触发命令**：`TEXTMASK`。
- **实现位置**：`src/app/commands/dim.rs::"TEXTMASK"` 与 `starts_with("TEXTMASK ")`。
- **备注**：仅处理 Text/MText；生成 `Wipeout::from_corners` 后 `select` 并 `DRAWORDER FRONT`。

### 文字宽度适配（TEXTFIT）
- **功能简介**：调整选中单行文字的宽度因子，使其渲染宽度匹配目标值。
- **UI 入口**：`命令输入框 → TEXTFIT`。
- **触发命令**：`TEXTFIT`。
- **实现位置**：`src/app/commands/dim.rs::"TEXTFIT"` 与 `starts_with("TEXTFIT ")`。
- **备注**：仅单行 Text；按包围盒当前宽度计算 `width_factor`。

### 文字顺序编号（TCOUNT）
- **功能简介**：按阅读顺序（先上后下、再左到右）对选中单行文字加顺序编号。
- **UI 入口**：`命令输入框 → TCOUNT`。
- **触发命令**：`TCOUNT`（`TCOUNT <start> <increment> <placement>`）。
- **实现位置**：`src/app/commands/dim.rs::"TCOUNT"` 与 `starts_with("TCOUNT ")`；命令 `src/command` 中 `TCountCommand`。
- **备注**：placement = `O`/Overwrite、`P`/Prefix、`S`/Suffix；默认 start 1、increment 1。

### 表格单元编辑（TABLEDIT）
- **功能简介**：拾取表格单元并编辑其文字内容。
- **UI 入口**：`命令输入框 → TABLEDIT`；也可双击表格单元触发。
- **触发命令**：`TABLEDIT`。
- **实现位置**：`src/modules/annotate/table_cmd.rs::TableditCommand`、`TableCellEditCommand`、`table_cell_at`、`cell_locked`；分发 `src/app/commands/dim.rs::"TABLEDIT"`。
- **备注**：空 Enter 结束；单元内容为自由文本，Space 输入空格，Enter 完成，`/n`/`\n` 换行；锁定/只读单元不可编辑（`CONTENT_LOCKED`/`CONTENT_READ_ONLY`）。

---

（文档结束）
