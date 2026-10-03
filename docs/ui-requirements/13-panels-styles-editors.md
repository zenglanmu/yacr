# Ribbon 面板/组合控件、样式管理窗口、块面板、节点图与专用编辑器功能需求清单

本文档反推自 `src/ui/ribbon/`（`widgets.rs`、`collapse.rs`、`color_dropdown.rs`、`mod.rs` 的样式/图层组合部分）、`src/ui/style/` 全部文件、`src/ui/window/block_definition.rs`、`src/ui/window/block_palette.rs`、`src/ui/node_graph.rs`、`src/ui/text_util.rs`、`src/ui/wrap_bar.rs`、`src/app/style_ops.rs`、`src/app/mtext_editor.rs`、`src/app/text_inline.rs`、`src/ui/window/find_replace.rs`，以及渲染原位编辑器的 `src/app/view/overlay.rs`。图层面板本身见 `11-properties-and-docks.md`，此处不重复。

面板与工具的权威布局入口是各模块的 `CadModule::ribbon_groups()`；组合控件渲染入口是 `src/ui/ribbon/widgets.rs::render_large`。

---

## 一、Ribbon 面板容器与密度折叠系统

### 自适应面板行（CollapsePanels）
- **功能简介**：把当前选项卡的所有面板排在一行；窗口变窄时从右向左逐级降级每个面板（完整 → 紧凑图标列 → 折叠标题按钮 → 小图标紧凑按钮），行高随最高面板收缩。
- **UI 入口**：`Ribbon → 任意选项卡 → 工具面板行`（整行容器）。
- **样式**：每面板有 4 个渲染槽 `[full, compact, button, tight]`（`SLOTS = 4`）；相邻面板之间绘制 1px 分隔线（主题 `background.neutral`），但两个按钮形态面板之间不画线（`collapse.rs::draw`）。
- **触发命令**：无（自动布局）。
- **实现位置**：`src/ui/ribbon/collapse.rs::CollapsePanels`、`Panel`、`Level`、`slot`、`Widths`、`decide_levels`。
- **备注**：当所有面板都到 tight 仍溢出时，按钮间隙最多各压缩 `MAX_PANEL_SQUEEZE = 8.0` px；行高经 `report_height`、tight 状态经 `report_tight` 上报给选项卡栏。

### 面板降级级别（Level）
- **功能简介**：定义单个面板的四种密度渲染。
- **UI 入口**：不可直接点击；由布局自动或密度模式决定。
- **样式**：`Full`（大按钮整面板）、`Compact`（图标列）、`Collapsed`（大图标标题按钮）、`Tight`（小图标标题按钮）。
- **触发命令**：无。
- **实现位置**：`src/ui/ribbon/collapse.rs::Level`、`slot`、`decide_levels`。
- **备注**：Auto 模式先跑一整轮 `Compact` 再跑一整轮 `Collapsed`，最后若仍溢出则把所有 `Collapsed` 一次性降到 `Tight`（`decide_levels`）。

### 面板密度模式选择器（CollapseMode）
- **功能简介**：用户可选 Auto / Full / Compact / Collapsed，覆盖自动折叠；选择会持久化。
- **UI 入口**：`Ribbon → 选项卡栏右侧 → 密度下拉按钮（▾）`。
- **样式**：`PosReport` 包装的下拉按钮，id `COLLAPSE_MODE`（`widgets.rs::COLLAPSE_MODE_ID`）；弹出列表每项为一行。
- **触发命令**：`Message::ToggleRibbonDropdown("COLLAPSE_MODE")`；选择发 `SetCollapseMode`（消息定义于 `src/app/mod.rs`，处理于 `src/app/update/mod.rs`）。
- **实现位置**：`src/ui/ribbon/mod.rs`（`COLLAPSE_MODE_ID` 按钮与 overlay）、`src/ui/ribbon/collapse.rs::CollapseMode`（`ALL`、`label`、`forced_level`）。
- **备注**：全部选项：`Auto`、`Full`、`Compact`、`Collapsed`（`CollapseMode::ALL`）。`Full` 即使溢出也保持大按钮；`Auto` 进入 tight 时选项卡栏会隐藏模式选择器（由 `report_tight` 驱动）。

### 面板标题飞出（Flyout）
- **功能简介**：面板被折叠成按钮后，点击标题按钮弹出该面板的完整大按钮渲染作为飞出面板。
- **UI 入口**：`Ribbon → 选项卡 → 已折叠面板的标题按钮`。
- **样式**：`FlyoutOverlay` 锚定在按钮下方，四周 1px 主题 `background.neutral` 边框、`background.weakest` 底色（`collapse.rs::FlyoutOverlay::draw`）；越界时上翻/内收。
- **触发命令**：点击面板标题；外部点击发 `Message::CloseRibbonDropdown` 关闭。
- **实现位置**：`src/ui/ribbon/collapse.rs::FlyoutOverlay`、`CollapsePanels::overlay`。
- **备注**：只有 `Collapsed` / `Tight` 形态的面板才弹飞出；飞出内容复用 `elements[Level::Full]`（该槽此时空闲）。

### 组合下拉锚点上报（PosReport）
- **功能简介**：记录每个下拉按钮在屏幕上的绘制边界，使打开的下拉能精确锚定在按钮下方。
- **UI 入口**：无（内部机制）。
- **样式**：透明包装器。
- **触发命令**：无。
- **实现位置**：`src/ui/wrap_bar.rs::PosReport`、`PosReport::new/owned`、`dropdown_bounds`。
- **备注**：`DD_BOUNDS` 为线程本地表；也用于选项卡拖拽重排的落点判定。

### 自适应双块栏（WrapBar）
- **功能简介**：lead +（可选 Fill middle）+ trail 的单行栏，宽度不足时把 trail 换到下一行；用于选项卡栏（快速访问 + 撤销/重做 vs 选项卡）与状态栏。
- **UI 入口**：`Ribbon 顶部选项卡栏`、`状态栏`。
- **样式**：`WrapBar`（`spacing`、`min_row_h` 默认 28.0、`justify_end`、`middle`、`report_height`）。
- **触发命令**：无。
- **实现位置**：`src/ui/wrap_bar.rs::WrapBar`。
- **备注**：`justify_end` 时 trail 靠右；tri-slot 模式下 middle 填满首行剩余、trail 落到下方（flex-wrap）。

### 弹性换行流（WrapFlow）
- **功能简介**：把一组子项从左到右排布，放不下就换行；用于状态栏 pill、选项卡。
- **UI 入口**：`状态栏`（pill 行）、`选项卡栏`（trail 选项卡）。
- **样式**：`WrapFlow`（`spacing_x` 默认 2.0、`min_spacing_x` 可压缩、`row_h` 默认 28.0、`justify_end`、`report_natural_width`）。
- **触发命令**：无。
- **实现位置**：`src/ui/wrap_bar.rs::WrapFlow`。

### 密度替换（DensitySwap）
- **功能简介**：在若干“由宽到窄”的候选渲染中，选出能放进可用宽度的最宽变体只显示它（如工具区完整面板行 → 紧凑行）。
- **UI 入口**：Ribbon 工具区（宽度紧张时）。
- **样式**：`DensitySwap`（`variants` 宽者优先、`report_height`、`report_width0`）。
- **触发命令**：无。
- **实现位置**：`src/ui/wrap_bar.rs::DensitySwap`。
- **备注**：都不适配时显示最后一个变体（预期会换行适配）。

### 选项卡拖拽重排（ReorderTab）
- **功能简介**：把选项卡标题变成拖拽源，拖到另一选项卡左/右半区即重排，并绘制 2px 主题主色插入指示线。
- **UI 入口**：`Ribbon 顶部选项卡栏 → 选项卡标题`、`状态栏 → 布局选项卡标题`。
- **样式**：`ReorderTab`（透明包装，标题才可拖动，避免关闭按钮误触）；拖拽中光标 `Grabbing`，悬停 `Grab`。
- **触发命令**：释放发 `Message::TabReorder{from,to,after}` 或 `Message::LayoutReorder{from,to,after}`。
- **实现位置**：`src/ui/wrap_bar.rs::ReorderTab`、`ReorderState`。
- **备注**：拖拽启动阈值 `START_DISTANCE_SQUARED = 16.0`；落点边界取自 `dropdown_bounds("DOC_TAB:{to}")` / `dropdown_bounds("SB_LAYOUT_TAB:{to}")`。

---

## 二、Ribbon 按钮与组合控件（widgets.rs）

### 大按钮（LargeTool）
- **功能简介**：占满 3 行高度的图标 + 文字标签按钮，执行一个命令。
- **UI 入口**：`Ribbon → 各选项卡 → 各面板 → 大按钮`。
- **样式**：`LARGE_W = ROW_H*2.2` 宽、图标 `LARGE_ICON = ROW_H*1.5`、标签 `LARGE_LABEL_SIZE = 10.0` 最多 2 行（`LARGE_LABEL_LINES`）、按钮内边距 `[4,4,4,4]`；宽度按标签自动测量（`automatic_large_width`/`measure_large_width`，带 `LARGE_WIDTH_CACHE` 缓存）。按下发 `Message::RibbonToolClick{tool_id,event}`。悬停 400ms 显示 tooltip。
- **触发命令**：由 `ToolDef.event`（多为 `ModuleEvent::Command("…")`）决定。
- **实现位置**：`src/ui/ribbon/widgets.rs::render_large`（`RibbonItem::LargeTool/LargeTool|Tool|LabeledTool` 分支）、`tool_btn_style`、`make_icon`、`tool_tip_text`；`AutomaticLargeWidth`、`automatic_large_button`。

### 大下拉（LargeDropdown）
- **功能简介**：图标 + 标签 + 底部 ▾ 条的大按钮；点上半执行当前项，点 ▾ 展开列表。
- **UI 入口**：`Ribbon → 各选项卡 → 各面板 → 大下拉`。
- **样式**：`render_large_dropdown`：上半为 `Fill` 图标+标签，底部 ▾ 条高 `LARGE_ARR = ROW_H*0.55`、箭头 `LARGE_DROPDOWN_ARROW_SIZE = 9.0`；当前图标/标签取自 `last_cmd`（记忆上次选择）。▾ 发 `Message::ToggleRibbonDropdown(id)`。
- **触发命令**：上半发当前 `last` 命令；具体见各面板条目。
- **实现位置**：`src/ui/ribbon/widgets.rs::render_large_dropdown`、`render_large`（`RibbonItem::LargeDropdown/LabeledDropdown` 分支）。

### 小按钮（Tool）
- **功能简介**：占 1 行、仅图标的小按钮。
- **UI 入口**：`Ribbon → 各面板 → 小按钮`、`ToolGrid` 单元、组合控件下方行。
- **样式**：`SMALL_W = ROW_H` 宽、图标 `SMALL_ICON = ROW_H*0.7`、内边距 `[4,4]`；右侧 tooltip。点按发 `Message::RibbonToolClick`。
- **触发命令**：`ToolDef.event`。
- **实现位置**：`src/ui/ribbon/widgets.rs::render_small`（`Tool|LargeTool` 分支）。

### 带标签小按钮（LabeledTool）
- **功能简介**：1 行高、图标 + 文字标签的小按钮。
- **UI 入口**：`Ribbon → 各面板 → 带标签小按钮`。
- **样式**：宽 `LABELED_SMALL_W = ROW_H*4.0`（渲染再加 `ARROW_W`）、图标 `SMALL_W` 宽、文本 10px。
- **触发命令**：`ToolDef.event`。
- **实现位置**：`src/ui/ribbon/widgets.rs::render_small`（`LabeledTool` 分支）。

### 小下拉（Dropdown）
- **功能简介**：1 行小按钮（图标）+ 右侧 ▾ 条（`ARROW_W = ROW_H*0.4`）的下拉。
- **UI 入口**：`Ribbon → 各面板 → 小下拉`。
- **样式**：主图标按钮 + ▾ 按钮并排（`PosReport` 包装），▾ 用 `icons::themed_arrow_down(8.0)`。
- **触发命令**：主按钮发 `last` 命令；▾ 发 `ToggleRibbonDropdown(id)`。
- **实现位置**：`src/ui/ribbon/widgets.rs::render_small`（`Dropdown|LargeDropdown` 分支）。

### 带标签小下拉（LabeledDropdown）
- **功能简介**：1 行、图标 + 标签 + ▾ 的小下拉；空标签时显示当前项名。
- **UI 入口**：`Ribbon → 各面板 → 带标签小下拉`。
- **样式**：同 `Dropdown` 但含 10px 标签；`is_disabled_item`（`FRAMES3`）为只显示状态项，点标签改为开列表。
- **触发命令**：`ToggleRibbonDropdown(id)`；选择发 `RibbonToolClick`。
- **实现位置**：`src/ui/ribbon/widgets.rs::render_small`（`LabeledDropdown` 分支）、`is_disabled_item`。

### 工具网格（ToolGrid）
- **功能简介**：多列小图标按钮网格，每列纵向排至多若干行（用于参数化等模块的密集工具面板）。
- **UI 入口**：`Ribbon → 参数化选项卡 → 工具网格面板`。
- **样式**：`row` 横排若干列，每列 `column` 宽 `SMALL_W`、`spacing(2)`，列间 `spacing(2)`，顶部对齐（`render_large` 中 `RibbonItem::ToolGrid` 分支）。
- **触发命令**：各单元为 `RibbonItem::Tool`，走 `render_small`。
- **实现位置**：`src/ui/ribbon/widgets.rs::render_large`（`ToolGrid` 分支）、`flush_small_col`；定义示例 `src/modules/parametric/mod.rs`。
- **备注**：参数化模块用 `ToolGrid { columns }` 组织工具（`parametric/mod.rs::ribbon_groups`）。

### 样式组合组（StyleComboGroup）
- **功能简介**：上方为当前样式名 + ▾ 的样式下拉（可切换当前样式、打开管理器），下方 1~2 行相关小按钮。
- **UI 入口**：`Ribbon → 注释 Annotate 选项卡 → Text / Dimensions / Leaders / Tables 面板`。
- **样式**：`STYLE_COMBO_W = LARGE_W*2.3`，下拉按钮为文本 + 上下三角（展开 ▾ / 收起 ▴），`combo_panel_col` 顶部对齐、内边距 `COMBO_PANEL_PAD`；列表以浮层 `Ribbon::style_combo_overlay` 渲染，避免被行高裁剪（#153）。
- **触发命令**：点按钮发 `ToggleRibbonDropdown(combo_id)`；选样式发 `Message::RibbonStyleChanged{key,name}`；列表底部 “Manage…” 发对应管理器命令。
- **实现位置**：`src/ui/ribbon/widgets.rs::render_large`（`StyleComboGroup` 分支）、`StyleContext`、`tool_row`、`combo_panel_col`；浮层 `src/ui/ribbon/mod.rs::style_combo_overlay`；数据 `StyleContext{text/dim/mleader/table_style_names, active_*}`。
- **备注**：`StyleContext::names_for`/`active_for` 按 `StyleKey`（`TextStyle`/`DimStyle`/`MLeaderStyle`/`TableStyle`）取列表与当前项。四个组合的 `combo_id` 与 `manager_cmd`：
  - Text 面板：`TEXT_STYLE_COMBO`，管理器命令 `STYLE`，下方含 `FIND`（Find）按钮（`src/modules/annotate/mod.rs`）。
  - Dimensions 面板：`DIM_STYLE_COMBO`，管理器 `DIMSTYLE`，两行按钮 `qdim`、`dim_continue`、`dim_baseline`，及 `tolerance_cmd`、`dimedit`、`dimtedit`、`dimbreak`、`dimspace`、`dimjogline`。
  - Leaders 面板：`MLEADER_STYLE_COMBO`，管理器 `MLEADERSTYLE`，两行 `mleader_edit` 的 add/remove/align/collect。
  - Tables 面板：`TABLE_STYLE_COMBO`，管理器 `TABLESTYLE`，一行 `data_extract`、`data_link`。

### 图层组合组（LayerComboGroup）
- **功能简介**：图层下拉显示当前图层的可见/冻结/锁定图标、颜色块与名称；下方两行共 10 个图层小按钮。
- **UI 入口**：`Ribbon → Draw 选项卡 → Layers 面板 → 图层下拉 + 两行小图标`。
- **样式**：`combo_w = max(LARGE_W*2.5, 工具行宽)`；下拉行含 `layer_visible`/`layer_freeze`/`layer_lock` 图标(14px)、12×12 颜色块、11px 名称（用 `text_util::elide` 缩短）、9px ▾；`combo_btn_style`，id `LAYER_COMBO`。
- **触发命令**：点按钮发 `ToggleRibbonDropdown("LAYER_COMBO")`；下方按钮为 `ToolDef`（见 `02-ribbon-draw.md` 图层工具栏细则）。
- **实现位置**：`src/ui/ribbon/widgets.rs::render_large`（`LayerComboGroup` 分支）、`comb_btn_style`、`tool_row`；下拉浮层 `src/ui/ribbon/mod.rs::layer_combo_overlay`。
- **备注**：两行由 `RibbonItem::LayerComboGroup { row2, row3 }` 定义；LAYER_COMBO 常量见 `widgets.rs::LAYER_COMBO_ID`。

### 特性组合组（PropertiesGroup）
- **功能简介**：左侧 Match 大按钮（紧凑时缩为小图标），右侧三行下拉：对象颜色（带色块）/ 线型 / 线宽。
- **UI 入口**：`Ribbon → Draw 选项卡 → Properties 面板`。
- **样式**：`PROP_W = 130.0`；下拉行含可选 12×12 色块、10px 文本（`clip(true)`）、展开时 ▴ 否则 ▾；`combo_btn_style(…, 2.0)`。id 常量 `PROP_COLOR_ID`/`PROP_LINETYPE_ID`/`PROP_LW_ID`。
- **触发命令**：Match 为 `MATCHPROP`；三行分别 `ToggleRibbonDropdown("PROP_COLOR"/"PROP_LINETYPE"/"PROP_LW")`。
- **实现位置**：`src/ui/ribbon/widgets.rs::render_large`（`PropertiesGroup` 分支，`prop_row`/`acad_color_display`/`linetype_display_name`/`LwItem`）；浮层 `src/ui/ribbon/mod.rs::prop_color_overlay/prop_linetype_overlay/prop_lw_overlay`。
- **备注**：`compact` 时 Match 用 `render_small`（`RenderCtx.compact`）。

### 顶部快速访问按钮（QuickAccess）
- **功能简介**：顶部条上的 New/Open/Save/Save As/Print 图标按钮，派发命令字符串并带悬停提示。
- **UI 入口**：`Ribbon → 顶部条 → 快速访问按钮`。
- **样式**：`quick_access_btn`：`TOP_HIST_W = 28.0` 宽、24 高、图标 16px（`icons::themed` 提亮）、`button::subtle`，tooltip 在下方。
- **触发命令**：`Message::Command(cmd)`。
- **实现位置**：`src/ui/ribbon/widgets.rs::quick_access_btn`、`TOP_HIST_W`、`TOP_HIST_GAP`。

### 撤销/重做历史控件（History Control）
- **功能简介**：撤销/重做主按钮 + 历史列表 ▾ 按钮；无历史时按钮禁用。
- **UI 入口**：`Ribbon → 顶部条 → 撤销/重做`。
- **样式**：`render_history_control`：主按钮 `TOP_HIST_W` 宽、箭头 `TOP_ARR_W = 12.0` 宽、24 高；`top_hist_btn_style`（禁用为 `background.weakest`，展开为 `primary.weak`）。tooltip 显示 “%{count} steps available”。
- **触发命令**：主按钮 `Message::Undo` / `Message::Redo`；▾ 发 `ToggleRibbonDropdown("UNDO_HISTORY"/"REDO_HISTORY")`。
- **实现位置**：`src/ui/ribbon/widgets.rs::render_history_control`、`UNDO_HISTORY_ID`/`REDO_HISTORY_ID`、`top_hist_btn_style`。

### 图标与提示辅助（Icon/Tooltip）
- **功能简介**：统一图标渲染（字形/SVG 语义色）与按钮 tooltip 文本（名称 + 可选描述 + `Command: <id>`）。
- **UI 入口**：所有 Ribbon 按钮的悬停提示。
- **样式**：`make_icon`（Glyph 按 `size*0.7`，Svg 走 `icons::semantic`）；`make_tip` 11px；`tip_style` 用 `background.strong`、1px 中性边框、圆角 3。tooltip 在右侧、延迟 400ms（`TipPos::Right`）。
- **触发命令**：无。
- **实现位置**：`src/ui/ribbon/widgets.rs::make_icon`、`make_tip`、`tip_style`、`tool_tip_text`、`tool_description`（为 DC* 约束工具提供描述句）。

### 组合控件下方工具行（tool_row）
- **功能简介**：组合下拉下方的一行小图标按钮，带 tooltip（供图层/特性/样式组合复用）。
- **UI 入口**：`LayerComboGroup` / `StyleComboGroup` 的下方行。
- **样式**：`row().spacing(2)`，每按钮图标 16px、内边距 `[2,5]`、`tool_btn_style`。
- **触发命令**：`ToolDef.event → module_event_to_message`（`Command`/`OpenFileDialog`/`ClearModels`/`SetVisualStyle`/`ToggleLayers`/`PluginFileDialog`）。
- **实现位置**：`src/ui/ribbon/widgets.rs::tool_row`、`module_event_to_message`。

---

## 三、对象颜色下拉与 ACI 调色板（color_dropdown.rs）

### 颜色下拉面板（Color Dropdown）
- **功能简介**：五段式颜色飞出：逻辑色（ByLayer/ByBlock）、45 格快速色板、标准索引色（ACI 1~9）、最近色、打开完整选色对话框。
- **UI 入口**：`Ribbon → Draw 选项卡 → Properties 面板 → 颜色下拉（PropertiesGroup 第 1 行）`。
- **样式**：`color_dropdown_panel`，面板宽 `PANEL_W = 202.0`，外边距 `Padding{top:6,bottom:6,left:12,right:12}`，`popup_panel_style`；色块 `SWATCH_SIZE = 18.0`、`SWATCH_GAP = 2.0`；段间 1px `divider`；标题 “Index Color”/“Recent Colors” 用 `muted_text_style` 10px。
- **触发命令**：色块/逻辑色发 `Message::RibbonColorChanged(color)`；`Select Color...` 发 `Message::OpenColorWindow(ColorPickTarget::Ribbon, active_color)`。
- **实现位置**：`src/ui/ribbon/color_dropdown.rs::color_dropdown_panel`、`logical_row`、`swatch_btn`、`placeholder_swatch`、`more_colors_row`、`divider`；渲染入口 `src/ui/ribbon/widgets.rs::render_large`（PropertiesGroup）；浮层 `src/ui/ribbon/mod.rs::prop_color_overlay`。

### 逻辑色行（ByLayer / ByBlock）
- **功能简介**：把当前对象颜色设为随层或随块。
- **UI 入口**：`… 颜色下拉 → ByLayer / ByBlock 行`。
- **样式**：`logical_row`：色块 + 11px 标签，选中时 2px 主题主色边框，否则 1px 中性边框；行用 `popup_row_style`。
- **触发命令**：`Message::RibbonColorChanged(AcadColor::ByLayer | ByBlock)`。
- **实现位置**：`src/ui/ribbon/color_dropdown.rs::logical_row`。

### 快速色板（Quick Pick Grid）
- **功能简介**：45 个精选 RGB 色块（5 行 × 9 列）直接选色。
- **UI 入口**：`… 颜色下拉 → 中部 5×9 色格`。
- **样式**：`QUICK_PICK_GRID: [[(u8,u8,u8);9];5]`，每格 18×18、圆角 1；选中 2px 主色边框、悬停 1.5px 文本色边框、默认 1px 中性边框（`swatch_btn`）。
- **触发命令**：`Message::RibbonColorChanged(Color::Rgb{..})`。
- **实现位置**：`src/ui/ribbon/color_dropdown.rs::QUICK_PICK_GRID`、`swatch_btn`。

### 标准索引色（Index Color ACI 1–9）
- **功能简介**：9 个标准 ACI 索引色块。
- **UI 入口**：`… 颜色下拉 → “Index Color” 标题下的 9 格`。
- **样式**：同 `swatch_btn`，颜色取 `AcadColor::Index(1..=9)`。
- **触发命令**：`Message::RibbonColorChanged(Color::Index(n))`。
- **实现位置**：`src/ui/ribbon/color_dropdown.rs::color_dropdown_panel`（section3）、`aci_1_9`。

### 最近色（Recent Colors）
- **功能简介**：最多 9 个最近使用颜色，不足用占位块补齐。
- **UI 入口**：`… 颜色下拉 → “Recent Colors” 标题下`。
- **样式**：`swatch_btn` + 占位 `placeholder_swatch`（弱底、半透明边框）。
- **触发命令**：`Message::RibbonColorChanged(color)`。
- **实现位置**：`src/ui/ribbon/color_dropdown.rs::color_dropdown_panel`（section4）、`placeholder_swatch`。

### 选择颜色…（Select Color…）
- **功能简介**：打开完整选色对话框做精细选色。
- **UI 入口**：`… 颜色下拉 → 底部 “Select Color...”`。
- **样式**：`more_colors_row`，整行可点，11px 文本，`popup_row_style`。
- **触发命令**：`Message::OpenColorWindow(ColorPickTarget::Ribbon, active_color)`。
- **实现位置**：`src/ui/ribbon/color_dropdown.rs::more_colors_row`。

---

## 四、样式管理窗口通用框架（style_manager.rs / style_list.rs / common.rs / form.rs）

### 样式管理器共享框架（Scaffold）
- **功能简介**：所有命名样式管理器（文字/标注/表格/多重引线/多线）共用的外框：顶部工具栏（New / Copy / Delete | Set Current / Apply）、左侧样式列表、右侧属性编辑器。
- **UI 入口**：`各管理器窗口顶部栏 + 左侧列表 + 右侧编辑器`。
- **样式**：工具栏底色 `background.weak`、内边距 `[5,8]`；`tb_button`（内部 `btn_s`：accent 或普通，圆角 4）；左列表宽 170、内边距 `{top:12,right:8,bottom:12,left:12}`，列表容器 `background.weak` + 1px 中性边框；`vsep`/`hdivider` 分隔；整窗 `background.base`。
- **触发命令**：`on_new`/`on_copy`/`on_delete`/`on_set_current`/`on_apply`（各管理器提供具体消息，如 `TextStyleDialogNew`）。
- **实现位置**：`src/ui/ribbon/../ui/style/style_manager.rs::view`、`Scaffold`、`tb_button`、`tb_button_enabled`、`hdivider`、`vsep`。
- **备注**：`read_only` 时 Delete/Set Current/Apply 禁用；引用样式（referenced）只能复制，不能改/改名/删除/设为当前。

### 右侧编辑器外壳（EditorShell）
- **功能简介**：右侧编辑器统一组成：样式名 + 状态、预览区、Compare with 下拉、属性页签、滚动内容。
- **UI 入口**：`各样式管理器右侧`。
- **样式**：`editor_shell`：13px 主色样式名、10px 状态（`muted_text_style`）、预览区 `background.weak` + 1px 中性边框、页签按钮 `tab_button_style`（激活 `primary.strong`）、内容滚动 `padding [12,12]`。
- **触发命令**：页签发 `EditorTab.on_press`（如 `TextStyleDialogTab(0/1)`）；比较发 `EditorComparison.on_select`。
- **实现位置**：`src/ui/style/style_manager.rs::editor_shell`、`EditorShell`、`EditorTab`、`EditorComparison`、`tab_button_style`、`primary_text_style`。

### 样式列表行（style_list::item）
- **功能简介**：单行样式名，单击选择、双击进入内联改名；当前样式带 ✓。
- **UI 入口**：`各样式管理器左侧列表`。
- **样式**：整行是 `mouse_area`（而非 button，避免双击被吞）；选中时 `primary.strong` 底色/文字；`themed_check_cell` 固定宽 ✓ 列；改名时变 `text_input`（id `style-rename-input`）。
- **触发命令**：单击发 `on_select(kind,name)`；双击发 `Message::StyleRenameStart(kind,name)`；提交 `Message::StyleRenameCommit(kind)`；输入 `Message::StyleRenameEdit`。
- **实现位置**：`src/ui/style/style_list.rs::item`、`rename_input_id`。

### 表单通用构件（form.rs）
- **功能简介**：对话框通用 UI 积木：动作按钮、单选、下拉行、字段行、分隔线、分区标题。
- **UI 入口**：`各类对话框（块定义、点样式、绘图单位、打印等）`。
- **样式/构件**：`dialog_button(label,msg,primary)`（12px、`[6,18]`、primary/secondary）、`dialog_button_styled`、`dialog_button_styled_opt`、`form_radio`（16px、间距 6、11px）、`labeled_pick_list(_enabled)`（标签宽 `LABEL_WIDTH=92.0`）、`labeled_field(_enabled)`（12px 字段）、`labeled_field_compact`、`button_style(accent)`、`field_style`、`hdivider`/`vseparator`/`vsep`、`section_label`。
- **触发命令**：由调用方传入消息。
- **实现位置**：`src/ui/style/form.rs` 全部函数与 `LABEL_WIDTH`。

### 样式辅助与可访问性（common.rs）
- **功能简介**：共享 `muted_style`（`background.base.text` 68% 透明）与 WCAG 对比度工具；并提供 CJK 弹窗宽度估算。
- **UI 入口**：无（内部）。
- **样式**：`muted_style`/`muted_text_style`（alpha 0.68）。
- **触发命令**：无。
- **实现位置**：`src/ui/style/common.rs::muted_style`、`muted_text_style`、`to_linear`、`wcag_luminance`、`wcag_contrast`、`accessible_accent(_threshold)`、`canvas_is_light`、`is_wide`、`dropdown_popup_width`。
- **备注**：`btn_s` 在 `plotstyle.rs` 与 `style_manager.rs` 有意不合并（disabled 处理不同，见 `common.rs` 头注与测试）。

---

## 五、文字样式管理器（textstyle.rs）

### 文字样式管理器窗口
- **功能简介**：管理文字样式：字体（笔画/系统）、高度、宽高比、倾斜角、反向/倒置/垂直、注释性等，并实时预览。
- **UI 入口**：`Ribbon → Annotate 选项卡 → Text 面板 → 样式下拉 → Manage…`，或 `命令输入框 → STYLE`（别名 `TEXTSTYLE`），或 `STYLE DIALOG/UI`。
- **样式**：共享 `Scaffold` + `EditorShell`；两个页签 “Fonts”“Size and Effects”；`TextPreviewCanvas` 高 112。
- **触发命令**：`STYLE`（`StyleKind::Text`）；消息前缀 `TextStyleDialog*` / `TextStyleEdit` / `TextStyleApply`。
- **实现位置**：`src/ui/style/textstyle.rs::view_window`、`TextStyleView`、`TextPreviewCanvas`、`input_row`、`toggle`、`font_list`；命令 `src/app/commands/layerprops.rs`（`STYLE`/`TEXTSTYLE`）；数据 `src/app/style_ops.rs`。

### 字体页签（Fonts）
- **功能简介**：从笔画字体列表或系统字体列表选择字体，并可直接编辑字体文件名。
- **UI 入口**：`文字样式管理器 → Fonts 页签`。
- **样式**：两列 `font_list`（各 `FillPortion(1)`，高 250 可滚动，选中项 `list_item(active)`）；字段行 `Font file`/`Big font`/`System font`（只读为 `read_only::field`）。
- **触发命令**：`TextStyleFontPick(font)`（笔画）、`TextStyleEdit{field:"ttf",value}`（系统）。
- **实现位置**：`src/ui/style/textstyle.rs::view_window`（tab 0）、`font_list`、`input_row`；字体源 `BUILTIN_FONTS`、`crate::scene::text::sysfont::families()`。
- **备注**：`BUILTIN_FONTS` 全部：Standard、ISO、Simplex、RomanS、RomanD、RomanC、RomanT、ItalicC、ItalicT、ScriptS、ScriptC、GothGBT、GothGRT、GothITT、GreekC、Symbol、ISO3098、Unicode。

### 尺寸与效果页签（Size and Effects）
- **功能简介**：设置文字高度（固定/可变/注释性纸面高度）、宽度因子、倾斜角、反向、倒置、垂直、注释性。
- **UI 入口**：`文字样式管理器 → Size and Effects 页签`。
- **样式**：`input_row`（标签宽 150、字段宽 180）；`toggle` 为 14px 复选框（11px 文本）；垂直在选定系统字体时禁用。
- **触发命令**：`TextStyleEdit{field}`（`height`/`width`/`oblique`）、`TextStyleToggle(field)`（`backward`/`upside_down`/`vertical`/`annotative`）。
- **实现位置**：`src/ui/style/textstyle.rs::view_window`（tab 1）、`input_row`、`toggle`。
- **备注**：注释性开启时高度标签显示 “Paper text height”，否则 “Fixed height”；`0 = variable`。

### 文字样式比较（Compare with）
- **功能简介**：与另一样式比较，列出差异分区。
- **UI 入口**：`文字样式管理器 → 预览区下方 “Compare with” 下拉`。
- **样式**：`pick_list` 宽 150 + 10px 摘要（`无差异`/`Different: …`）。
- **触发命令**：`Message::TextStyleDialogCompare(name)`。
- **实现位置**：`src/ui/style/textstyle.rs::view_window`（`comparison`）；通用 `editor_shell`。

---

## 六、标注样式管理器（dimstyle.rs）

### 标注样式管理器窗口
- **功能简介**：管理标注样式（DIMSTYLE），含 7 个页签与实时标注预览。
- **UI 入口**：`Ribbon → Annotate 选项卡 → Dimensions 面板 → 样式下拉 → Manage…`，或命令 `DIMSTYLE`（别名 `DDIM`）。
- **样式**：共享 `Scaffold` + 右侧自绘面板；页签为 `button`（`tab_btn_style`，`[4,10]`）；预览 `DimensionPreview` 高 112；字段宽 100/标签宽 180、下拉宽 150/枚举宽 150。
- **触发命令**：`DIMSTYLE`（`StyleKind::Dim`）；消息前缀 `DimStyleDialog*`/`DsEdit`/`DsToggle`/`DsSetHandle`/`DsZero*`/`DsCenterMarkMode`/`DsToleranceMode`/`DsColorMore`。
- **实现位置**：`src/ui/style/dimstyle.rs::view_window`、`DimStyleValues`、`DimensionPreview`、`enum_field`、`color_row`、`hrow`、`zero_controls`；命令 `src/app/commands/layerprops.rs`（`DIMSTYLE`：DIALOG/LIST/NEW/SET）。

### 页签：Lines
- **功能简介**：设置尺寸线/尺寸界线/文本间隙、抑制、颜色、线宽、线型、固定长度延伸线。
- **UI 入口**：`标注样式管理器 → Lines 页签`。
- **样式**：分行 `label:field`；颜色为 `color_selector`（含 ByLayer/ByBlock，可 `more`）、线宽/线型为 `pick_list`。
- **触发命令**：`DsEdit(field,value)`、`DsToggle`、`DsSetHandle{field,value}`、`DsColorMore`。
- **实现位置**：`src/ui/style/dimstyle.rs::view_window`（tab 0）、`color_row`、`hrow`、`enum_field`。
- **备注**：字段 `Dimdle`/`Dimdli`/`Dimgap`/`Dimsd1`/`Dimsd2`/`Dimexe`/`Dimexo`/`Dimse1`/`Dimse2`/`Dimclrd`/`Dimlwd`/`Dimclre`/`Dimlwe`/`Dimfxlon`/`Dimfxl`/`Dimltex*`。

### 页签：Symbols and Arrows
- **功能简介**：设置箭头块、箭头大小、圆心标记方式与大小、刻度线大小、不同首末箭头、弧长符号、半径折角。
- **UI 入口**：`标注样式管理器 → Symbols and Arrows 页签`。
- **样式**：`hrow_enabled`（按条件启用）选择块/线型；圆心标记与公差等为 `pick_list`。
- **触发命令**：`DsSetHandle{field:"dimblk"/…}`、`DsCenterMarkMode(code)`、`DsEdit`、`DsToggle`。
- **实现位置**：`src/ui/style/dimstyle.rs::view_window`（tab 1）、`hrow_enabled`、`center_choices`、`enum_field`。
- **备注**：圆心标记选项 None / Center mark / Centerlines（由 `dimcen` 符号判定）；弧长符号 Before text/Above text/None。

### 页签：Text
- **功能简介**：设置文字高度、文字样式、垂直/水平放置、颜色、内外水平、垂直偏移、背景填充及颜色、左右阅读方向。
- **UI 入口**：`标注样式管理器 → Text 页签`。
- **样式**：`text_style_field` 为下拉；`text_height_note` 提示样式固定高度；背景色仅在 “Color” 时可选。
- **触发命令**：`DsEdit(Dimtxsty/…)`、`DsToggle`、`DsColorMore`。
- **实现位置**：`src/ui/style/dimstyle.rs::view_window`（tab 2）、`text_style_field`、`color_row`、`enum_field`。

### 页签：Fit
- **功能简介**：空间不足时的适配策略、文字移动方式、注释性与总体比例、放置微调。
- **UI 入口**：`标注样式管理器 → Fit 页签`。
- **样式**：`enum_field` 适配/移动；注释性时“总体比例”显示为 “Automatic”。
- **触发命令**：`DsEdit(Dimatfit/Dimtmove/…)`、`DsToggle(Annotative/Dimupt/Dimtofl)`。
- **实现位置**：`src/ui/style/dimstyle.rs::view_window`（tab 3）。

### 页签：Primary Units
- **功能简介**：线性单位格式/精度/分数格式/小数分隔符/舍入/测量模板/测量比例/零抑制，以及角度单位格式/精度/零抑制。
- **UI 入口**：`标注样式管理器 → Primary Units 页签`。
- **样式**：`OPT_LUNIT`、`OPT_LINEWEIGHT` 复用；`zero_controls` 提供 “Feet and inches”/前导零/尾零。
- **触发命令**：`DsEdit(field)`、`DsZeroBase`、`DsZeroFlag`。
- **实现位置**：`src/ui/style/dimstyle.rs::view_window`（tab 4）、`OPT_LUNIT`、`OPT_LINEWEIGHT`、`zero_controls`。
- **备注**：`OPT_LUNIT` 全部：Scientific/Decimal/Engineering/Architectural/Fractional/Windows desktop（码 1–6）；角度格式 Decimal degrees/DMS/Gradians/Radians/Surveyor's units。

### 页签：Alternate Units
- **功能简介**：启用替代单位及其乘数、精度、单位格式、公差精度、舍入、模板、零/公差零抑制。
- **UI 入口**：`标注样式管理器 → Alternate Units 页签`。
- **样式**：字段按 `dimalt` 启用；`zero_controls` 复用。
- **触发命令**：`DsToggle(Dimalt)`、`DsEdit`、`DsZeroFlag`。
- **实现位置**：`src/ui/style/dimstyle.rs::view_window`（tab 5）。

### 页签：Tolerances
- **功能简介**：公差方式（无/对称/偏差/极限/基本）、上下值、精度、高度比例、垂直位置、零抑制。
- **UI 入口**：`标注样式管理器 → Tolerances 页签`。
- **样式**：`tolerance_method_field` 由 `dimgap` 负号判定 Basic；字段按条件启用。
- **触发命令**：`DsToleranceMode(code)`、`DsEdit`、`DsZeroFlag`。
- **实现位置**：`src/ui/style/dimstyle.rs::view_window`（tab 6）、`tolerance_choices`。

### 标注预览（DimensionPreview）
- **功能简介**：按当前值绘制尺寸线/界线/箭头或刻度/文字/极限框，并显示测量文本（含替代单位、公差、极限）。
- **UI 入口**：`标注样式管理器 → 预览区`。
- **样式**：canvas 120+，尺寸线 y=bounds*0.58，箭头块大小由 `dimasz` 映射、基准框由 `dimgap` 负号触发；文字 12px。
- **触发命令**：无（随值刷新）。
- **实现位置**：`src/ui/style/dimstyle.rs::DimensionPreview::draw`、`view_window` 的 `preview_text` 计算。

---

## 七、多重引线样式管理器（mleaderstyle.rs）

### 多重引线样式管理器窗口
- **功能简介**：管理多引线样式（MLEADERSTYLE），含四个页签与引线预览。
- **UI 入口**：`Ribbon → Annotate 选项卡 → Leaders 面板 → 样式下拉 → Manage…`，或命令 `MLEADERSTYLE`。
- **样式**：共享 `Scaffold` + `EditorShell`；页签 “Leader Format”“Leader Structure”“Content”“Block Content”；预览 `LeaderPreviewCanvas` 高 150。
- **触发命令**：`MLEADERSTYLE`（`StyleKind::MLeader`）；消息前缀 `MLeaderStyleDialog*`/`MLeaderStyleEdit`/`MLeaderStyleSetEnum/SetHandle`/`MLeaderStyleToggle`/`MLeaderColorMore`。
- **实现位置**：`src/ui/style/mleaderstyle.rs::view_window`、`MLeaderStyleView`、`LeaderPreviewCanvas`、`enum_row`、`handle_row`、`color_row`、`input_row`、`toggle`；命令 `src/app/commands/layerprops.rs`（MLEADERSTYLE：DIALOG/LIST/NEW/SET）。

### 页签：Leader Format
- **功能简介**：描述、路径类型、线颜色/线宽/线型、箭头、箭头大小、断口大小。
- **UI 入口**：`多引线样式管理器 → Leader Format 页签`。
- **样式**：`input_row`（标签 165、字段 190）/`color_row`/`enum_row`（宽 220）/线宽 `pick_list`。
- **触发命令**：`MLeaderStyleEdit{field}`、`MLeaderStyleSetEnum/SetHandle`、`MLeaderStyleLineWeightChanged`。
- **实现位置**：`src/ui/style/mleaderstyle.rs::view_window`（tab 0）。
- **备注**：路径类型 None/Straight/Spline；线宽选项来自 `crate::ui::properties::lw_options`。

### 页签：Leader Structure
- **功能简介**：启用基线/狗腿、基线距离与间隙、最大引线点数、首/次段角度、比例因子、对齐间距、绘制顺序、注释性。
- **UI 入口**：`多引线样式管理器 → Leader Structure 页签`。
- **样式**：`toggle` + `input_row` + `enum_row`；注释性时比例因子显示为 “By annotation scale”（只读）。
- **触发命令**：`MLeaderStyleToggle`、`MLeaderStyleEdit`、`MLeaderStyleSetEnum`。
- **实现位置**：`src/ui/style/mleaderstyle.rs::view_window`（tab 1）、`LEADER_DRAW_ORDERS`、`MULTILEADER_DRAW_ORDERS`。

### 页签：Content
- **功能简介**：内容类型（无/块/文本/公差）、默认文本、文字样式、文字高度、文字颜色、文字角度/对齐/附着方向、四向附着点、文本框与始终左对齐。
- **UI 入口**：`多引线样式管理器 → Content 页签`。
- **样式**：`enum_row` 用 `ATTACHMENTS` 11 项列表；两个 toggle 同行。
- **触发命令**：`MLeaderStyleSetEnum/SetHandle`、`MLeaderStyleEdit`、`MLeaderStyleToggle`。
- **实现位置**：`src/ui/style/mleaderstyle.rs::view_window`（tab 2）、`ATTACHMENTS`。
- **备注**：`ATTACHMENTS` 全部：Top of top line / Middle of top line / Middle of text / Middle of bottom line / Bottom of bottom line / Bottom line / Underline bottom line / Underline top line / Underline all text / Center of text / Center of text with overline。

### 页签：Block Content
- **功能简介**：块内容类型下的块名、颜色、连接方式、旋转、X/Y/Z 缩放、启用缩放/旋转。
- **UI 入口**：`多引线样式管理器 → Block Content 页签`。
- **样式**：`handle_row` 选择块、`color_row`、`enum_row`（`BLOCK_CONNECTIONS`）、`input_row`、`toggle`。
- **触发命令**：`MLeaderStyleSetHandle/SetEnum`、`MLeaderStyleEdit`、`MLeaderStyleToggle`。
- **实现位置**：`src/ui/style/mleaderstyle.rs::view_window`（tab 3）、`BLOCK_CONNECTIONS`（Block extents / Base point）。

### 引线预览（LeaderPreviewCanvas）
- **功能简介**：按当前样式绘制引线路径、箭头（闭合填充/开放/点/刻度/斜线/方框）、基线、内容文本或块框、文本框与附着偏移。
- **UI 入口**：`多引线样式管理器 → 预览区`。
- **样式**：canvas 高 150；虚线由线型名判定；块内容绘制方框+对角线；底部/顶部附信息行 9px。
- **触发命令**：无。
- **实现位置**：`src/ui/style/mleaderstyle.rs::LeaderPreviewCanvas::draw`、`aci_color`、`choice_label`、`handle_label`。

---

## 八、表格样式管理器（tablestyle.rs）

### 表格样式管理器窗口
- **功能简介**：管理表格样式（TABLESTYLE），含 General 与三类行（数据/表头/标题）编辑页签及表格预览。
- **UI 入口**：`Ribbon → Annotate 选项卡 → Tables 面板 → 样式下拉 → Manage…`，或命令 `TABLESTYLE`（别名 `TS`）。
- **样式**：共享 `Scaffold` + `EditorShell`；页签 General / Data Row / Header Row / Title Row；预览 `TablePreviewCanvas` 高 150。
- **触发命令**：`TABLESTYLE`（`StyleKind::Table`）；消息前缀 `TableStyleDialog*`/`TableStyleEdit`/`TableStyleCell*`/`TableStyleBorder*`/`TableStyleSetFlow`/`TableStyleToggle*`/`TableColorMore`。
- **实现位置**：`src/ui/style/tablestyle.rs::view_window`、`TableStyleView`、`TablePreviewCanvas`、`cell_editor`、`input_row`、`preview_rows`；命令 `src/app/commands/layerprops.rs`（TABLESTYLE：DIALOG/LIST/NEW）。

### 页签：General
- **功能简介**：描述、流向（上→下 / 下→上）、水平/垂直边距、抑制标题行/表头行、注释性。
- **UI 入口**：`表格样式管理器 → General 页签`。
- **样式**：`input_row`（标签 155、字段 170）、`pick_list` 流向、三个 checkbox（14px）。
- **触发命令**：`TableStyleEdit{field}`、`TableStyleSetFlow`、`TableStyleToggle`、`TableStyleToggleAnnotative`。
- **实现位置**：`src/ui/style/tablestyle.rs::view_window`（tab 0）。

### 页签：行编辑器（Data/Header/Title Row）
- **功能简介**：逐行类型设置文字样式/高度/颜色、填充色、对齐、背景填充开关、数据类型/单位/格式串，以及六条边框的类型/权重/颜色/间距/隐藏。
- **UI 入口**：`表格样式管理器 → Data Row / Header Row / Title Row 页签`。
- **样式**：`cell_editor`：`input_row` + `cell_color`（`color_selector`）+ 对齐 `pick_list`(170) + 填充 checkbox + 边框行（类型 `pick_list` 82、Weight 68、Color 62、Spacing 68、Hidden checkbox）。
- **触发命令**：`TableStyleCellEdit{row,field,value}`、`TableStyleCellSetAlign`、`TableStyleCellToggleFill`、`TableStyleBorderSetType`、`TableStyleBorderEdit{field}`、`TableStyleBorderToggleInvisible`、`TableColorMore`。
- **实现位置**：`src/ui/style/tablestyle.rs::cell_editor`、`cell_input`、`cell_color`、`borders` 列表。
- **备注**：对齐 9 项 TopLeft…BottomRight；边框 6 条 Left/Right/Top/Bottom/Inside horizontal/Inside vertical；类型 Single/Double，可 Hidden。

### 表格预览（TablePreviewCanvas）
- **功能简介**：按当前样式绘制三类行的填充、文字、边框（含双线）、抑制行/流向。
- **UI 入口**：`表格样式管理器 → 预览区`。
- **样式**：canvas 高 150；行序 `[2,1,0]` 并按抑制/流向调整；文字高度映射 7–16px；双线间距 `Spacing` 1–5。
- **触发命令**：无。
- **实现位置**：`src/ui/style/tablestyle.rs::TablePreviewCanvas::draw`、`draw_line`、`border_width`、`preview_rows`。

---

## 九、多线样式管理器（mlstyle.rs）

### 多线样式管理器窗口
- **功能简介**：管理多线样式（MLSTYLE），含元素列表与端帽/填充设置及预览。
- **UI 入口**：命令 `MLSTYLE`（`MLSTYLE DIALOG/UI`），或命令面板（`StyleKind::MLine`）。
- **样式**：共享 `Scaffold` + `EditorShell`；两个页签 “Elements”“Caps and Fill”；预览 `MLinePreviewCanvas` 高 150。
- **触发命令**：`MLSTYLE`（`StyleKind::MLine`）；消息前缀 `MlStyleDialog*`/`MlStyleEdit`/`MlStyleToggle`/`MlStyleElement*`。
- **实现位置**：`src/ui/style/mlstyle.rs::view_window`、`MlStyleView`、`MLinePreviewCanvas`、`input_row`、`toggle`；命令 `src/app/commands/layerprops.rs`（MLSTYLE：LIST/NEW/SET/DEL）。

### 页签：Elements
- **功能简介**：多线各元素的偏移/颜色/线型列表，可增删与逐项编辑。
- **UI 入口**：`多线样式管理器 → Elements 页签`。
- **样式**：表头列（Offset 110 / Color 100 / Line type 170 / Add 按钮）+ 每行三个 `text_input` + Delete 按钮。
- **触发命令**：`MlStyleElementAdd`、`MlStyleElementEdit{index,field}`、`MlStyleElementDelete(index)`。
- **实现位置**：`src/ui/style/mlstyle.rs::view_window`（tab 0）。

### 页签：Caps and Fill
- **功能简介**：描述、填充与接头开关、填充色、起/止端帽（直线/内弧/外弧）与角度。
- **UI 入口**：`多线样式管理器 → Caps and Fill 页签`。
- **样式**：`toggle` 分组 + `input_row`（标签 165、字段 190）。
- **触发命令**：`MlStyleToggle(field)`（`fill`/`joints`/`start_square`/`start_inner`/`start_round`/`end_*`）、`MlStyleEdit{field}`（`description`/`fill_color`/`start_angle`/`end_angle`）。
- **实现位置**：`src/ui/style/mlstyle.rs::view_window`（tab 1）。

### 多线预览（MLinePreviewCanvas）
- **功能简介**：按元素偏移绘制多条线（颜色/线型，可为虚线）、填充多边形、起止端帽与接头。
- **UI 入口**：`多线样式管理器 → 预览区`。
- **样式**：canvas 高 150；元素按偏移降序排序；文字行显示描述与元素数。
- **触发命令**：无。
- **实现位置**：`src/ui/style/mlstyle.rs::MLinePreviewCanvas::draw`、`element_points`、`draw_arc`、`aci_color`。

---

## 十、点样式与注释比例等专用样式窗口

### 点样式对话框（Point Style，DDPTYPE）
- **功能简介**：从 PDMODE 字形网格选点标记形状并设点大小（相对屏幕 % 或绝对单位）。
- **UI 入口**：`命令输入框 → DDPTYPE`（打开模态 `ModalKind::PointStyle`）。
- **样式**：`view_window`，标题 18px；字形网格 4 行（enclosure 0/32/64/96）× 5 列（shape 0–4），每格 `CELL_PX = 44.0`，选中 `primary.strong`；大小输入 110 宽 + `%`/`units`；两个 `form_radio`；底部 OK（primary）。
- **触发命令**：`DDPTYPE`；消息 `PointStyleSetMode(i16)`、`PointStyleSizeRelative(bool)`、`PointStyleSizeInput`、`PointStyleApplySize`、`PointStyleOk`。
- **实现位置**：`src/ui/style/point_style.rs::view_window`、`GlyphCanvas`、`cell`、`ENCLOSURES`、`SHAPES`；命令 `src/app/commands/styleprops.rs::"DDPTYPE"`。
- **备注**：字形 nibble：0 点、1 无、2 `+`、3 `×`、4 `|`；enclosure 位 32 圆、64 方、96 两者。命令 `PDMODE`/`PDSIZE` 可无对话框设置。

### 注释比例管理器（Scale Manager）
- **功能简介**：增删改绘图 ACAD_SCALELIST 中的注释比例（name + paper:drawing）。
- **UI 入口**：`Ribbon → Annotate 选项卡 → Annotation Scaling 面板 → Scale List（SCALELISTEDIT）`，或状态栏注释比例 pill。
- **样式**：与样式管理器同框：工具栏 `New / Copy / Delete | Set Current / Apply`、左侧 Scales 列表（宽 190，选中 `primary.strong`）、右侧 “Paper units / Drawing units” 字段；行内改名 `text_input`（id `scale-rename-input`）。
- **触发命令**：`SCALELISTEDIT`（`Message::ScaleManagerOpen`）；消息 `ScaleManagerNew/Copy/Delete/SetCurrent/Apply/Select/PaperBuf/DrawingBuf`、`ScaleRename*`。
- **实现位置**：`src/ui/style/scale_manager.rs::view_window`、`rename_input_id`、`field_style`；命令 `src/app/commands/display.rs::"SCALELISTEDIT"`；暂存 `src/app/style_ops.rs::scale_stage_*`、`ScaleSnapshot`/`ScaleStage`（Apply 一次撤销 `SCALELISTEDIT`）。
- **备注**：命令行 `SCALELISTEDIT ADD 1:50` / `DELETE 1:50`；当前比例不可删除。

### 注释对象比例对话框（Annotation Object Scale）
- **功能简介**：为单个选中对象增减其携带的注释比例表示。
- **UI 入口**：`Ribbon → Annotate 选项卡 → Annotation Scaling 面板 → Add/Delete Scales（OBJECTSCALE）`，或 Properties 面板的按钮。
- **样式**：与样式管理器同框：顶部 “Object: <label>” + Close（primary）；比例列表每行 `themed_check_cell(member)` + 名称 + 比例文本，点击整行切换。
- **触发命令**：`OBJECTSCALE`（`Message::AnnoObjectScaleOpen`）；切换 `AnnoObjectScaleToggle(name)`。
- **实现位置**：`src/ui/style/anno_object_scale.rs::view_window`；命令 `src/app/commands/display.rs::"OBJECTSCALE"`；消息处理 `src/app/update/mod.rs`。
- **备注**：需先选中一个对象，否则提示 “Select one object first, then run OBJECTSCALE.”；点击添加/移除会合成/删除该比例的 per-object context。

### 打印样式表编辑器（Plot Style Table Editor）
- **功能简介**：编辑 CTB/STB 打印样式表：逐 ACI 颜色设置打印颜色、线宽、淡显，查看使用该 ACI 的图层。
- **UI 入口**：`命令输入框 → PLOTSTYLE`（别名 `PLOTSTYLEPANEL`/`PLOTSTYLEEDITOR`/`STYLESMANAGER`），或打印对话框的按钮。
- **样式**：整窗；工具栏 `Load CTB/STB`、`Save`（primary）、`Save As…`、`Clear Table` + 表名；左侧 ACI 列表（宽 280）每行色块 + 等宽字体标签（含 override 颜色/线宽或 “(default)”）+ 图层使用数圆点；右侧编辑区：ACI、Plot color（色块 + Choose color… + Reset）、Lineweight 下拉、Screening 输入（0-100）、Current values 摘要、Layers using ACI 列表。
- **触发命令**：`PLOTSTYLE [LOAD|?|STATUS|CLEAR]`；消息 `PlotStyleLoad`/`PlotStylePanelSaveDirect`/`PlotStylePanelSave`/`PlotStyleClear`/`PlotStylePanelSelectAci`/`PlotStylePanelColorBuf`/`PlotStylePanelLwSet`/`PlotStylePanelScreenBuf`/`OpenColorWindow(ColorPickTarget::PlotStyle)`。
- **实现位置**：`src/ui/style/plotstyle.rs::view_window`、`plot_lineweight_options`、`build_layer_usage`、`PlotLineweightItem`；命令 `src/app/commands/display.rs`；常量映射 `src/io/plot_style.rs::{DEFAULT_PLOT_STYLE, MONOCHROME_PLOT_STYLE, …}`、`LW_TABLE`。
- **备注**：`build_layer_usage` 按 ACI（1..256）分桶、忽略 truecolor 图层；STB 表不显示图层使用列（`show_layer_usage`）；线宽特殊值 255 = Use object lineweight。

---

## 十一、样式管理通用操作与暂存（style_ops.rs）

### 通用样式操作（New / Copy / Delete / Rename / Set Current / Apply）
- **功能简介**：五个样式管理器共用的列表 CRUD；改名会重写按名引用与当前指针；Apply 才真正提交并推一次撤销。
- **UI 入口**：`各样式管理器 → 顶部工具栏 / 列表双击改名`。
- **样式**：见 `style_manager.rs::view`。
- **触发命令**：各前缀消息（`TextStyleDialogNew` 等）。
- **实现位置**：`src/app/style_ops.rs::style_new/style_copy/style_delete/style_rename_start/style_rename_commit`、`StyleKind`、`style_names/style_exists/style_in_use`、`insert_default_style/clone_style_as/remove_style_storage/rename_style_storage`。
- **备注**：`Standard` 不可删/改名；在用或当前样式不可删；新名取 `Style{n}`、复制名取 `{base} ({n})`。存储分两类：表驱动（Text/Dim，按大写名键）与对象驱动（Table/MLeader/MLine，按 handle 键）。

### 样式暂存事务（Style Stage）
- **功能简介**：管理器打开即快照样式状态，编辑实时改文档做预览但不标脏/不入撤销；Apply 提交（标脏、一次撤销、重建），关闭未 Apply 则还原。
- **UI 入口**：`各样式管理器`（应用/关闭行为）。
- **样式**：无。
- **触发命令**：`style_stage_begin/commit/discard`。
- **实现位置**：`src/app/style_ops.rs::style_stage_begin/style_stage_commit/style_stage_discard`、`StyleStateSnapshot`、`StyleStage`、`capture_style_state/restore_style_state`、`changed_keys`。
- **备注**：`ensure_standard_styles` 在每次打开文件时补齐 Standard（含从 `ACAD_MLEADERSTYLE` 字典导入多引线样式名并写回）；`sync_mleaderstyle_dictionary` 维护多引线样式字典（软属主 350）。

---

## 十二、块定义对话框与块面板

### 块定义对话框（Block Definition，BLOCK / BMAKE）
- **功能简介**：创建或重定义块定义：命名、基点、对象来源与处理方式、行为开关、块单位、说明与超链接。
- **UI 入口**：`Ribbon → Draw 选项卡 → Block 面板 → 创建块（BLOCK）`，或命令 `BLOCK`（别名 `B`、`BMAKE`）。
- **样式**：`view_window` 全窗；左右两列（左 `FillPortion(1)`、右 `FillPortion(2)`）；分组框 `group`（1px `background.strong` 边框、圆角 4、内边距 `[5,8]`）；`labeled_checkbox`（14px）；单选 `form_radio`(16px)；字段 `field_style`；底部 `dialog_button` OK(primary)/Cancel/Help；错误横幅（danger 底色 0.18）；重定义确认用 `stack` + 半透明遮罩 + 圆角面板。
- **触发命令**：`BLOCK`/`BMAKE`（分发 `src/app/commands/blocks.rs::"BLOCK" | "BMAKE"`）；消息前缀 `BlockDef*`。
- **实现位置**：`src/ui/window/block_definition.rs::view_window`、`BlockDefinitionState`、`BlockObjectMode`、`group`、`labeled_checkbox`、`UnitChoice`。
- **备注**：
  - 名称行 `combo_box`（已有块名可下拉/输入），实时校验非法字符 `\ / : * ? " < > | = \`` 与重名（重名提示将重定义）。
  - Base point 组：`Specify On-screen` 复选、`Pick point` 按钮、X/Y/Z 输入。
  - Objects 组：`Specify On-screen`、`Select objects`、Quick Select 图标按钮、对象模式 `Retain`/`Convert to block`/`Delete`（默认 Convert）、已选数量状态（0 时警告图标 “No objects selected”）。
  - Behavior 组：`Annotative`、`Match block orientation to layout`（仅注释性时可用）、`Scale uniformly`、`Allow exploding`（默认开）。
  - Settings 组：`Block unit` 下拉（`units::all()`）、`Hyperlink...`（有值时显示 `Hyperlink*`）。
  - Description 组：`text_editor` 多行说明。
  - 重定义守卫：`BlockDefConfirmRedefine(true/false)`，默认焦点在 “No”。

### 块面板（Block Palette，BLOCKPALETTE）
- **功能简介**：停靠的 “Insert Block” 面板：可搜索的块缩略图网格，点击即插入，支持从文件插入与缩略图大小切换。
- **UI 入口**：`Ribbon → Insert 选项卡（或插入块面板）→ BLOCKPALETTE`，或命令 `BLOCKPALETTE`（别名 `BLOCKSPALETTE`），或 `Ribbon` 的 `BLOCKPALETTE` 切换按钮。
- **样式**：`view`：`dock::title_bar(PanelId::BlockPalette,…)` + 头部（搜索框 + “Insert from file” + “Preview size” 图标按钮）+ 网格主体（卡片间距 6）。卡片 = 缩略图 canvas（`PreviewSize.box_height`）+ 单行标签（`MAX_LABEL_CHARS = 12`、`LABEL_LINE_H = 16.0`、`text_util::elide`）；占用中卡片 `primary.base`、悬停 `background.strong`、普通 `background.base`，边框 1px（`block_card_colors`/`block_card_border`）。
- **触发命令**：`BLOCKPALETTE`/`BLOCKSPALETTE`（切换 `show_block_palette`）；消息 `BlockPalette(Search/CyclePreviewSize/PickFile/Insert/Refresh/FilePicked)`。
- **实现位置**：`src/ui/window/block_palette.rs::view`、`BlockPalette`、`BlockPaletteMsg`、`PreviewSize`、`cycle_preview_size`、`BlockPreviewCanvas`、`block_card`、`icon_button`；命令 `src/app/commands/blocks.rs::"BLOCKPALETTE" | "BLOCKSPALETTE"`；工具定义 `src/modules/insert/mview_block.rs`。
- **备注**：缩略图尺寸三档 Small（3 列 / 56 高）、Medium（2 列 / 84）、Large（1 列 / 120），`CyclePreviewSize` 循环切换；搜索按块名不区分大小写过滤；无块显示 “No blocks in this drawing”，无匹配显示 “No matches”；正在放置的块高亮。缩略图绘制实心填充三角形（alpha 0.55）+ 折线（NaN 分隔为断点，线宽→2.0/1.0）。

---

## 十三、节点图（node_graph.rs）

### 节点图面板（Node Graph Palette）
- **功能简介**：停靠的节点库面板：搜索框 + New/Open/Save，按类别（Objects 与 graph 库分类）折叠列出节点，点击或拖拽到画布添加。
- **UI 入口**：`命令输入框底部 “Node Graph” 按钮`（`src/ui/command_line.rs` 发 `Graph(GraphMsg::Toggle)`），或若已停靠则右侧停靠面板。
- **样式**：`panel`：`dock::title_bar(PanelId::NodeGraph, "Node Graph")` + 工具行（搜索框 + `DOC_NEW`/`FOLDER_OPEN`/`SAVE`）+ 可滚动分类树；类别用 `themed_arrow_toggle` 展开标记，标题底 `background.weak`；条目 `mouse_area` 图标+12px 标签、`Interaction::Grab`。
- **触发命令**：`GraphMsg::Toggle/New/Open/Save/Search/Category/PalettePress/PaletteRelease`。
- **实现位置**：`src/ui/node_graph.rs::Graph::panel`、`palette_entry`、`PaletteItem`、`ObjectKind`、`GraphMsg`；处理 `src/app/node_graph.rs::on_graph`；停靠 `src/app/view/mod.rs`（`PanelId::NodeGraph`）。
- **备注**：首次开启自动停靠到右侧并展开（`on_graph::Toggle`）；Objects 分类初始展开、其余初始折叠（`toggled_categories`）；搜索时所有匹配项强制展开。

### 节点画布（Node Graph Canvas）
- **功能简介**：覆盖视口的节点图画布：节点卡片、连线、拖拽、平移、缩放、删除、连线/断线；对象节点行即该实体的 Properties 面板行。
- **UI 入口**：`Node Graph 面板存在时 → 视口画布区域`。
- **样式**：节点宽 `NODE_W = 280.0`；标题高 `HEADER_H = 26.0`、分区标题 `SECTION_H = 22.0`、行高 `ROW_H = 22.0`、标签宽 `LABEL_W = 96.0`、端口 `PORT_W = 14.0`/命中 `PORT_HIT = 10.0`；节点卡片 `background.base` + 1px `background.strong` 边框、圆角 4；标题底 `primary.weak`；连线为贝塞尔曲线、`primary.base`，进行中的连线 `primary.weak`，线宽 `2.0*zoom`；画布蒙层 `background.base` alpha 0.6。端口圆点 8×8，已连接填充 `primary.base`。
- **触发命令**：`GraphMsg::{Moved,Pressed,Released,NodePress,NodeDelete,Section,OutPress,InPress,Input,Commit,Set,Zoom}`；页面级 `Message::Graph(GraphMsg)`。
- **实现位置**：`src/ui/node_graph.rs::Graph::view/node_view/row_view`、`Wires`、`Zoomed`、`port_dot`、`wire_path`、`update_ui`；对象节点构建 `src/app/node_graph.rs::graph_object_sections/graph_all_sections/graph_write/graph_step`。
- **备注**：
  - 缩放步进 `ZOOM_STEP = 1.1`，范围 `ZOOM_STEPS = -15..=12`；滚轮缩放以光标为锚点。
  - 拖拽种类 `Drag::{Palette,Node,Link,Pan}`；抓取节点会置顶；拖输入端口已连线会先断开再重接。
  - 节点行控件随 spec：`RowWidget::Slider`（Number/Integer Slider 的 value，含 min/max/step）、`RowWidget::Toggle`（Boolean）、`RowWidget::Field`（其余，文本提交时按 JSON 解析否则字符串）。
  - 节点类别来自 `graph::CATEGORIES`，对象类别固定为 “Objects”（`OBJECTS` 10 种：Line/Circle/Arc/Ellipse/Point/Polyline/Ray/XLine/Text/MText）。

### 节点图存取（.ocg 文件）
- **功能简介**：将图、画布与绘图指纹存为 `.ocg`；打开时仅在指纹相同的图纸中复用实体。
- **UI 入口**：`Node Graph 面板 → Save / Open 图标`。
- **样式**：系统文件对话框（`.ocg` 过滤）标题 “Save Node Graph”/“Open Node Graph”。
- **触发命令**：`GraphMsg::Save/Open/Saved/Loaded`。
- **实现位置**：`src/ui/node_graph.rs::Graph::to_file/from_file`；处理 `src/app/node_graph.rs::on_graph`。
- **备注**：格式 `{"format":"ocg","version":1,"drawing":<fingerprint_guid>,"graph":…,"canvas":{pan,zoom,nodes}}`；保存时若图纸无 `fingerprint_guid` 会即时生成并标脏。评估与撤销：`graph_step` 推一次 `GRAPH` 撤销。

---

## 十四、原位文字编辑器（text_inline.rs / mtext_editor.rs / overlay.rs）

### 单行文字原位编辑器（Text Inline）
- **功能简介**：在插入点处弹出的纯文本框，输入单行文字，Enter 提交、Esc 取消；无格式工具栏。
- **UI 入口**：`命令 TEXT / DDEDIT / 双击文字`（`begin_text_edit` 路由到纯文本框）。
- **样式**：`text_inline_overlay`：`text_input`（id `text_inline_input`，占位 “Text”，宽 240、13px、内边距 6）+ 面板 `background.weak` + 1px 中性边框、圆角 5；锚点 `(screen_anchor.x-6, screen_anchor.y-18)` 限制在画布内（`PANEL_W`/`PANEL_H`）。
- **触发命令**：`Message::TextInlineInput`/`TextInlineOk`（Enter）；取消 `text_inline_cancel`。
- **实现位置**：`src/app/text_inline.rs::TextInlineState`、`open_text_inline`、`text_inline_commit`、`text_inline_cancel`、`begin_text_edit`、`read_text_field`/`write_text_field`、`TextEntityField`；渲染 `src/app/view/overlay.rs::text_inline_overlay`。
- **备注**：`TextEntityField` 决定读写槽：`Text`/`AttDef`/`AttEnt`/`Dim`/`Tolerance`（纯文本）与 `MText`/`MLeader`（富文本）；Leader 会沿 `annotation_handle` 链解析到被标注实体（最多 8 跳）；Tolerance 改为打开公差对话框；图层锁定时不打开。空白内容对新建丢弃、对编辑保留原值；新建文字继承当前文字样式。

### 多行文字编辑器（MText Editor）
- **功能简介**：富文本编辑器：格式化工具栏 + 实渲染预览（含选择/光标/换行标尺）+ 宽度滑块 + 列/段落/查找替换；支持 MTEXT 与 MultiLeader 文本。
- **UI 入口**：`命令 MTEXT / DDEDIT / 双击 MText 或 MLeader 文本`。
- **样式**：经共享模态框（标题 “Text Editor”）；顶部动作栏 `Apply`（primary）、`Close Text Editor`（primary，即 `MTextOk`）；四行工具栏 + 宽度滑块 + 预览区；图标按钮 18px、`btn_style`（`background.weak`/悬停 `strong`）；预览区 1px 中性边框、`background.base`。
- **触发命令**：`MTEXT`/`DDEDIT`；消息前缀 `MText*`（`MTextOk`/`MTextApply`/`MTextFmt`/`MTextStyle`/`MTextFont`/`MTextHeight`/`MTextColorChanged` 等）。
- **实现位置**：`src/app/mtext_editor.rs::MTextEditorState`、`open_mtext_editor`、`mtext_commit`、`mtext_apply`、`mtext_cancel`、`rebuild_mtext_preview`、各类 `mtext_*` 编辑函数、`Cell`/`doc_to_cells`/`cells_to_doc`；工具栏渲染 `src/app/view/overlay.rs::mtext_editor_overlay`/`mtext_editor_content`、`MTextPreview`；常量 `MTEXT_TEXT_ID`、`MTEXT_PREVIEW_EM_PX = 15.0`、`MTEXT_PREVIEW_PAD = 12.0`、`MTEXT_EDITOR_WRITING_WIDTH`。
- **备注**：`build_mtext` 从被编辑实体出发以保留列/旋转/法向/背景等未暴露字段；新 MText 继承当前文字样式；注释性样式自动置注释性并建比例 context；`Apply` 后编辑器绑定新实体，重复 Apply 不产生重复。内部以 `Cell` 扁平可见字符索引作光标/选区空间，堆叠（`\S`）原子不可分。

### 工具栏第 1 行：样式 / 字体 / 高度 / 字符格式 / 颜色
- **功能简介**：选文字样式、注释性、字体、高度，切换粗体/斜体/下划线/上划线/删除线/大写/小写，选颜色。
- **UI 入口**：`MText 编辑器 → 工具栏第 1 行`。
- **样式**：样式 `pick_list`(96)、注释性 checkbox(14) + 标签、字体 `pick_list`(120)、高度输入(64)、7 个格式图标按钮(18px)、颜色 `color_selector`(150)。
- **触发命令**：`MTextStyle`、`MTextAnnotative`、`MTextFont`、`MTextHeight`、`MTextFmt(Bold/Italic/Underline/Overline/Strike/Uppercase/Lowercase)`、`MTextColorChanged`/`MTextColorPickerToggle`/`OpenColorWindow(ColorPickTarget::MText)`。
- **实现位置**：`src/app/view/overlay.rs::mtext_editor_content`（`row1`）、`MTEXT_FONTS`；处理 `src/app/mtext_editor.rs::mtext_apply_fmt/mtext_apply_font/mtext_apply_color`。
- **备注**：`MTEXT_FONTS` 全部：[Style default]、Standard、ISO、Simplex、RomanS、RomanD、ItalicC、ScriptS、GothGBT、RomanC。有选区时按 run 应用，无选区时设置全局默认；粗体用更重的 Gothic 面/字体 bold 标志，斜体为 15° 倾斜。

### 工具栏第 2 行：撤销重做 / O-W-◊ / 段落对齐 / 附着 / 行距 / 堆叠 / 清除 / 特殊字符
- **功能简介**：撤销/重做；倾斜角、宽度因子、字符间距；左/中/右/两端对齐；9 点附着下拉；行距 1/1.5/2；堆叠、清除格式；插入 ° ± ⌀。
- **UI 入口**：`MText 编辑器 → 工具栏第 2 行`。
- **样式**：↶/↷ 按钮、标签 “O”/“W”/“◊” + 输入(48)、4 个对齐图标、`JustifyChoice` 下拉(112)、行距按钮、`Stack`/`Clear`、三个特殊字符按钮。
- **触发命令**：`MTextUndo`/`MTextRedo`、`MTextOblique`/`MTextWidth`/`MTextCharSpace`、`MTextAlign(ParaAlign)`、`MTextJustify(AttachmentPoint)`、`MTextLineSpacing(f32)`、`MTextStack`、`MTextClearFormatting`、`MTextInsert(String)`。
- **实现位置**：`src/app/view/overlay.rs`（`row2`）、`JustifyChoice`；处理 `src/app/mtext_editor.rs::mtext_undo/mtext_redo/mtext_apply_span_number/mtext_apply_align/mtext_stack_selection/mtext_clear_formatting`。
- **备注**：`JustifyChoice::ALL` 9 项 Top Left…Bottom Right；堆叠按选区首个 `/`/`#`/`^` 分隔为分数（Limit/Diagonal/Horizontal）。

### 工具栏第 3 行：列与段落
- **功能简介**：列模式（无/静态/动态自动/动态手动）、列数、列高/宽/间距、反向；段落首行/左/右缩进与段前/段后间距。
- **UI 入口**：`MText 编辑器 → 工具栏第 3 行`。
- **样式**：列模式 `pick_list`(120)、Count/Height/Width/Gutter 输入、Reverse checkbox、段落 5 个数字输入(48)。
- **触发命令**：`MTextColumnMode`、`MTextColumnCount`、`MTextColumnHeight`、`MTextColumnWidth`、`MTextColumnGutter`、`MTextColumnFlowReversed`、`MTextParagraphNumber(ParaNumber, value)`。
- **实现位置**：`src/app/view/overlay.rs`（`row3`）、`ParaNumber`；处理 `src/app/mtext_editor.rs::mtext_apply_paragraph_number`、`build_mtext`（列数据）。

### 工具栏第 4 行：查找 / 替换
- **功能简介**：在编辑器文本内查找下一个、替换当前、全部替换。
- **UI 入口**：`MText 编辑器 → 工具栏第 4 行`。
- **样式**：Find/Replace 输入各 140、`Next`/`Replace`/`Replace All` 按钮。
- **触发命令**：`MTextFindText`/`MTextReplaceText`、`MTextFindNext`、`MTextReplaceNext`、`MTextReplaceAll`。
- **实现位置**：`src/app/view/overlay.rs`（`row4`）；处理 `src/app/mtext_editor.rs::mtext_find_next/mtext_replace_next/mtext_replace_all`。
- **备注**：查找在扁平字符序列上循环匹配（含换行），替换按选中匹配进行并继承相邻格式。

### 宽度滑块与预览区
- **功能简介**：拖动滑块设置 MText 换行矩形宽度；预览区实渲染文字、选中高亮、光标、换行标尺，支持点击选择、拖拽选择、双击选词、直接键入。
- **UI 入口**：`MText 编辑器 → 宽度滑块行 + 预览区`。
- **样式**：滑块下方标签 “Width: %{value}” 与百分比；预览 canvas 双向滚动（`Direction::Both`）；换行标尺 1px 主色（alpha 0.65）、选中高亮主色 alpha 0.45、光标为橙色（`warning.base`）1.5px 竖条。
- **触发命令**：`MTextRectWidth`；预览交互 `MTextSelStart`/`MTextSelTo`、`MTextSelWord`、`begin_text_edit` 相关；键盘编辑 `mtext_type`/`mtext_backspace`/`mtext_delete`/`mtext_caret_move(_vertical)`/`mtext_select_all`。
- **实现位置**：`src/app/view/overlay.rs::MTextPreview`、`offset_at`、`mtext_editor_content`（`width_slider`/`body`）；处理 `src/app/mtext_editor.rs::mtext_*`、`MTextEditorState::preview_scale`。
- **备注**：预览按 `preview_scale` 固定屏幕上字高 `MTEXT_PREVIEW_EM_PX`；空文本显示左上橙色光标；多列内容按真实内容宽可横向滚动。

---

## 十五、查找替换窗口（find_replace.rs）

### 查找替换窗口（Find / Replace）
- **功能简介**：在整个图纸范围内（Text/MText/属性定义/块属性值）查找并替换文本，支持逐条替换与全部替换。
- **UI 入口**：`命令输入框 → FIND`（快捷键 Ctrl+F / Ctrl+H），或 `Annotate 选项卡 → Text 面板 → 样式组合下方 Find 按钮`。
- **样式**：`view_window`：两行 `label:field`（Find / Replace with 标签宽 90、输入 13px、内边距 `[6,8]`）；说明行 “Searches Text, MText, Attribute Definitions, and block attribute values.”（11px）；状态行；底部按钮 `Close`(secondary)/`Replace`(secondary)/`Replace All`(danger)/`Find Next`(primary)。
- **触发命令**：`FIND`；消息 `FindReplaceOpen`/`FindReplaceSearchChanged`/`FindReplaceReplacementChanged`/`FindReplaceNext`/`FindReplaceOne`/`FindReplaceAll`（输入框 id `FIND_INPUT_ID`，Enter = Find Next）。
- **实现位置**：`src/ui/window/find_replace.rs::view_window`、`FIND_INPUT_ID`；逻辑 `src/app/find_replace.rs::open_find_replace/find_replace_next/find_replace_one/find_replace_all`；命令 `src/app/commands/inquiry.rs`（FIND）、快捷键 `src/app/shortcuts.rs`。
- **备注**：搜索不区分大小写；导航时居中相机并选中匹配对象（含块内实体与块属性）；替换前会检查图层锁定并在无改动时回退撤销；状态显示 “{index} of {total} — {label}” 或未找到提示；`Replace`/`Replace All`/`Find Next` 仅在搜索框非空时可用。

---

## 十六、文本裁剪与换行辅助

### 文本省略（elide）
- **功能简介**：将过长用户文本截断到字符预算并加省略号，避免换行破坏固定行高或溢出窄面板。
- **UI 入口**：`图层组合标签、块卡片标签等`。
- **样式**：按 `char` 计数（多字节安全），`max` 含省略号字符。
- **触发命令**：无。
- **实现位置**：`src/ui/text_util.rs::elide`。
- **备注**：`max == 0` 返回空串；超长时取 `max-1` 字符 + `…`。

---

（文档结束）
