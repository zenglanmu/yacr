# 属性面板、图层管理器与停靠系统功能需求清单

本文档反推自 `src/ui/properties.rs`、`src/ui/dock.rs`、`src/ui/window/layers.rs`、`src/ui/window/layer_state_manager.rs`、`src/ui/window/layer_translator.rs`、`src/modules/view/{properties_palette,tool_palettes,sheetset}.rs`、`src/app/{properties,layers,style_ops}.rs`、`src/app/view/{mod,modal}.rs`、`src/app/update/{mod,dialog,command}.rs`、`src/scene/view/dispatch.rs`、`src/scene/cache/properties.rs`。覆盖属性面板（Properties）、图层管理器窗口（Layer Manager）、停靠系统（Dock）、侧边面板、图层状态管理器、图层翻译器，以及 Tool Palettes / Sheet Set Manager 的开关入口。

---

## 一、属性面板（Properties 停靠面板）

属性面板由 `src/ui/properties.rs::PropertiesPanel` 渲染；内容 `sections` 由 `src/app/properties.rs::refresh_properties` 每次选择变化时重建。默认停靠在左侧边缘（`src/ui/dock.rs::DockState::default`：`left: vec![PanelId::Properties]`）。

### 属性面板头（Title bar：对象类型 / 选择计数）
- **功能简介**：显示当前选择的对象类型名或“No selection”，多选时提供按类型过滤的下拉，并把面板从当前宽度缩放到内容宽度。
- **UI 入口**：`属性面板 → 标题栏（面板默认停靠于左侧边缘）`。
- **样式**：`container`，背景 `background.weakest`，1px `background.neutral` 边框，内边距 `[4, 10]`；文本字号 `FONT_SZ = ROW_H*0.42`（≈11px），`muted_text_style`（`base.text` 透明度 0.72）。
- **触发命令**：无（随选择刷新）。
- **实现位置**：`src/ui/properties.rs::PropertiesPanel::view`（`title_content`/`title_bar`）；标题文本由 `src/app/properties.rs::refresh_properties` 与 `entity_property_sections` 生成。
- **备注**：`selection_groups` 为空时用 `text_util::elide(title, 34)` 直接显示类型名；非空时改用 `combo_box`（`selection_group_combo`）发 `Message::PropSelectionGroupChanged`（见“选择组下拉”）。

### 面板头停靠标题条（Dock title bar：Properties / Auto / Close）
- **功能简介**：面板顶部的停靠标题条，显示 “Properties”，提供自动隐藏（钉住）与关闭按钮，并可作为拖拽把手重新停靠。
- **UI 入口**：`属性面板 → 顶部停靠标题条`。
- **样式**：`src/ui/dock.rs::title_bar`；行背景 `background.weak`，1px `background.neutral` 边框，内边距 `[3, 6]`；标题字号 12；钉住图标 `PIN` 12px（未钉住用 `themed_secondary`，已钉住用 `themed_primary_weak_text` + 主色底/边框高亮），关闭图标 `CLOSE` 12px；均为 `button::subtle`，提示 tooltip 字号 10（"Auto" / "Close"）。
- **触发命令**：无。
- **实现位置**：`src/ui/dock.rs::title_bar(id, title, auto_collapse)`。
- **备注**：点击标题条发 `Message::Dock(DockMsg::DockGrab(id))` 开始拖拽；鼠标指针变 `Grab`。

### 空选择页面板提示
- **功能简介**：无选择时显示提示文本；无选择且无绘图内容时显示绘图级（Drawing）属性分组。
- **UI 入口**：`属性面板 → 内容区`。
- **样式**：提示文本 “Select an object to view properties” 字号 10，`hint_text_style`（透明度 0.48），内边距 `[10, 10]`。
- **触发命令**：无。
- **实现位置**：`src/ui/properties.rs::PropertiesPanel::view`（`content`）；空选择分组 `src/app/properties.rs::refresh_properties` 的 `0 =>` 分支。

### 属性分组折叠 / 展开（Section）
- **功能简介**：每个属性分组（General、Geometry、3D Visualization…）可折叠/展开；折叠状态为应用级偏好，跨所有打开的图纸共享并持久化。
- **UI 入口**：`属性面板 → <分组> 标题行（双击标题，或点击右端箭头）`。
- **样式**：分组头背景 `background.weak`，1px `background.neutral` 边框；标题字号 10；右端箭头为 `themed_arrow_down`（展开）/ `themed_arrow_right`（折叠）10px，`button::text`，内边距 `[0, 3]`。
- **触发命令**：无。
- **实现位置**：`src/ui/properties.rs::PropertiesPanel::render_section`；状态 `PropertiesPanel::collapsed_sections`；处理 `Message::PropSectionToggle`（`src/app/update/mod.rs`）。
- **备注**：折叠状态存于 app 级 `collapsed_property_sections`，切换时同步到每个打开文档的 `tab.properties.collapsed_sections`。

### 坐标分量行折叠（Position / Start / Scale … X/Y/Z）
- **功能简介**：连续的 “<Base> X / <Base> Y [/ <Base> Z]” 文本行默认合并为一行摘要（值以逗号连接），点击箭头展开为独立的 X/Y/Z 编辑行。
- **UI 入口**：`属性面板 → Geometry 分组 → <Position/Start/Scale…> 摘要行（点击行首箭头）`。
- **样式**：摘要行分两列（标签 `FillPortion(5)`，值 `FillPortion(6)`），行高 `ROW_H`；标签为 `button::subtle` + `themed_arrow_right`/`themed_arrow_down`；值用 `read_only::field`；展开后行标签缩进显示 “    X /     Y /     Z”。
- **触发命令**：无。
- **实现位置**：`src/ui/properties.rs::{render_section,render_group_row,coord_group_len,coord_base,coord_component}`；状态 `PropertiesPanel::expanded_groups`（键 `"{section}:{base}"`）；处理 `Message::PropGroupToggle`。
- **备注**：`View` 分组不做合并（`render_section` 对 `section.title == "View"` 跳过）；`pl3_vertex_x`/`pm_vx` 字段不参与分组。

### 只读文本值行（Read-only）
- **功能简介**：不可编辑但可选中/复制（Ctrl+C）的只读值，用于坐标、句柄、统计量等。
- **UI 入口**：`属性面板 → 任意只读属性行 → 值区`。
- **样式**：`crate::ui::read_only::field(value, FONT_SZ, Length::Fill)`；行布局 `prop_row_widget`（标签列 `FillPortion(5)`，值列 `FillPortion(6)`，行高 `ROW_H`，1px `background.neutral` 边框）。
- **触发命令**：无。
- **实现位置**：`src/ui/properties.rs::render_ro_row`；字段类型 `PropValue::ReadOnly`。
- **备注**：无 `on_input`，因此不出现光标，但文本可选中。

### 带提示的只读值行（ReadOnlyWithTooltip）
- **功能简介**：只读值，悬停显示额外说明（如 CTB 模式下 “Plot style is locked to color in Color-Dependent (CTB) mode”）。
- **UI 入口**：`属性面板 → <行> → 值区（悬停）`。
- **样式**：`tooltip`，位置 Top，gap 4，padding 6；提示框背景 `background.base`，1px `background.neutral` 边框，圆角 4。
- **触发命令**：无。
- **实现位置**：`src/ui/properties.rs::render_ro_with_tooltip_row`；字段类型 `PropValue::ReadOnlyWithTooltip`；示例注入 `src/app/properties.rs::entity_property_sections`（`plot_style` 在 CTB 模式）。
- **备注**：多选混合时保留 tooltip，值变 `*VARIES*`（`merge_prop_value`）。

### 可编辑几何文本行（EditText）
- **功能简介**：以 `text_input` 编辑几何/公共数值字段（坐标、半径、角度、比例等），输入提交后写回实体。
- **UI 入口**：`属性面板 → Geometry / General 分组 → 数值行`。
- **样式**：`text_input`，字号 `FONT_SZ`，内边距 `[3, 6]`；`text_input_style`（聚焦边框 `primary.base`，否则 `background.neutral`，圆角 2）；获得焦点整行以 `primary.weak` 高亮、边框 `primary.base`。
- **触发命令**：无（Enter 提交）。
- **实现位置**：`src/ui/properties.rs::{render_edit_row,render_prop_row}`；`prop_geom_field_id(field)` 生成控件 id；字段类型 `PropValue::EditText`；处理 `Message::PropGeomInput` / `PropGeomCommit`（`src/app/update/command.rs::on_prop_geom_commit`）。
- **备注**：编辑缓冲存 `PropertiesPanel::edit_buf`（键 `FieldKey::Geom`）；同一选择重建时保留未提交值，选择改变时丢弃；提交调用 `dispatch::apply_geom_prop`（工作平面感知）。

### 纯文本值行（PlainText）
- **功能简介**：只读的纯文本行（无 select-all 语义），用于拼接的说明性内容。
- **UI 入口**：`属性面板 → 任意纯文本行`。
- **样式**：与只读行同布局（`prop_row_widget`）。
- **触发命令**：无。
- **实现位置**：`src/ui/properties.rs::render_prop_row`（`PropValue::PlainText(_)` 走 `render_edit_row`）；字段类型 `PropValue::PlainText`。
- **备注**：面板内与 `EditText` 共用编辑渲染路径。

### 图层下拉行（Layer）
- **功能简介**：用下拉显示并切换对象所在图层；选项来自文档图层表。
- **UI 入口**：`属性面板 → General 分组 → Layer 行`。
- **样式**：`combo_box`（下拉），字号 `FONT_SZ`，内边距纵向 `COMBO_PAD_V`、横向 6，`combo_input_style`，宽度 Fill；打开时发 `Message::PropColorPickerClose` 关闭其它 picker。
- **触发命令**：无。
- **实现位置**：`src/ui/properties.rs::render_layer_row`；字段类型 `PropValue::LayerChoice`；选项状态 `PropertiesPanel::layer_combo`；处理 `Message::PropLayerChanged`（`apply_property_op(..., "CHPROP", ...)` → `dispatch::apply_common_prop(entity, "layer", ...)`）。
- **备注**：多选混合值显示占位 `*VARIES*`（`VARIES_LABEL`）；无选择时改设为当前图层默认。

### 颜色行（Color / NamedColor / 通用字段颜色）
- **功能简介**：以内联选色控件设置对象主颜色，或设置特定字段颜色（MTEXT 背景、渐变两色、标注线颜色、文字颜色、块内容色等）。
- **UI 入口**：`属性面板 → General 分组 → Color 行`；或 `属性面板 → 各专有分组 → <Color 字段> 行`。
- **样式**：`color_selector` / `color_selector_with_name`（色块 + 颜色名 + 弹出面板）；支持 ByLayer/ByBlock/None 等逻辑项（由 `ColorExtras` 决定）。
- **触发命令**：无。
- **实现位置**：`src/ui/properties.rs::{render_color_row,render_color_varies_row}`；处理 `Message::PropColorChanged`、`PropBgColorChanged`、`PropColorFieldChanged`、`PropColorPickerToggle`、`PropColorFieldToggle`、`PropBgColorPickerToggle`。
- **备注**：`field == "background_color"` 路由到 MTEXT 背景色（`PropBgColorChanged`）；通用字段列表含 `gradient_color_1/2`、`dim_line_color`、`dim_ext_line_color`、`dim_text_color`、`dim_text_fill_color`、`line_color`、`text_color`、`block_content_color`、`background_fill_color`、`indicator_fill_color`；“More Colors…” 发 `Message::OpenColorWindow(ColorPickTarget::PropertiesField(...))`。标注类颜色以 ACI 索引写回（DWG/DXF 往返）。

### 颜色“混合值”行（ColorVaries）
- **功能简介**：多选对象颜色不一致时，色块显示为无确定值的选色控件；选择新颜色即批量应用。
- **UI 入口**：`属性面板 → General → Color 行（多选且值不同）`。
- **样式**：`color_selector_varies(...)`。
- **触发命令**：无。
- **实现位置**：`src/ui/properties.rs::render_color_varies_row`；字段类型 `PropValue::ColorVaries`（由 `merge_prop_value` 生成）。
- **备注**：选中即通过 `PropColorChanged` 对全部目标应用。

### 线型下拉行（Linetype）
- **功能简介**：设置对象线型；空串归一为 ByLayer；多选混合显示 `*VARIES*`。
- **UI 入口**：`属性面板 → General 分组 → Linetype 行`。
- **样式**：`combo_box` + `wide_menu`（菜单宽 `LINETYPE_MENU_W = 220`）；选项为 `LinetypeItem { name, art }`，显示名称 + ASCII 线型预览。
- **触发命令**：无。
- **实现位置**：`src/ui/properties.rs::render_linetype_row`；字段类型 `PropValue::LinetypeChoice`；选项状态 `linetype_combo`；处理 `Message::PropLinetypeChanged`。
- **备注**：`linetype_display_name` 把空/`ByLayer` 显示为 “ByLayer”、`ByBlock` 显示为 “ByBlock”。

### 线宽下拉行（Lineweight / FieldLw）
- **功能简介**：设置对象线宽（ByLayer/ByBlock/Default/具体毫米值）；也支持专有字段线宽（如 MLEADER 引线线宽）。
- **UI 入口**：`属性面板 → General 分组 → Lineweight 行`；或专有分组内的 FieldLw 行。
- **样式**：`combo_box`，选项为 `LwItem`（`Display` 为 “ByLayer/ByBlock/Default/{v:.2} mm”）。
- **触发命令**：无。
- **实现位置**：`src/ui/properties.rs::{render_lw_row,render_field_lw_row,render_lw_varies_row,render_field_lw_varies_row,lw_options}`；字段类型 `PropValue::{LwChoice,FieldLwChoice,LwVaries,FieldLwVaries}`；处理 `Message::PropLwChanged`、`PropFieldLwChanged`。
- **备注**：`lw_options()` 全部选项：ByLayer、ByBlock、Default、0、5、9、13、15、18、20、25、30、35、40、50、53、60、70、80、90、100、106、120、140、158、200、211。

### 布尔切换行（Yes / No）
- **功能简介**：切换布尔属性（如 Invisible、Annotative、图例/表头抑制、块参照 Uniform scale 等），按钮文本在 Yes/No 间切换。
- **UI 入口**：`属性面板 → 各分组 → <布尔行>`。
- **样式**：`button`，值真时文本与边框用 `warning.base` 色；`prop_row_widget` 布局。
- **触发命令**：无。
- **实现位置**：`src/ui/properties.rs::render_bool_row`；字段类型 `PropValue::BoolToggle`；处理 `Message::PropBoolToggle(field)`（`src/app/update/mod.rs`，含 `is_annotative`/`invisible`/`ins_uniform`/`tbl_*` 等分支）。
- **备注**：块参照 Uniform scale 勾选会把 Y/Z 缩放置为 X，取消仅展开为逐轴行（`properties.rs` 注入 + `Props_asym_scale`）。

### 步进器行（Current Vertex ◀ / ▶）
- **功能简介**：在多顶点实体（多段线、网格、面、样条、表格单元格）间切换“当前顶点/单元格”，使几何行针对该顶点编辑。
- **UI 入口**：`属性面板 → Geometry 分组 → Current Vertex 行`。
- **样式**：`row![◀, text(display), ▶]`；箭头为 `button`，边框 `background.neutral`，圆角 2；显示值居中。
- **触发命令**：无。
- **实现位置**：`src/ui/properties.rs::render_stepper_row`；字段类型 `PropValue::Stepper`；处理 `Message::PropVertexStep(delta)`（按实体顶点数回绕）。
- **备注**：选择改变时 `prop_vertex` 归零；仅在用户操作步进器后显示指示器（`prop_vertex_indicator_active`）。

### 可编辑下拉行（EditChoice：块 Name 等）
- **功能简介**：文本输入 + 下拉箭头的组合控件；可输入名称（既有名重指向 / 新名重命名），或从候选定义列表中选择；输入文本过滤列表。
- **UI 入口**：`属性面板 → 各分组 → Name 等 EditChoice 行`。
- **样式**：容器绘制边框与背景、输入框透明无边框，文本 + `caret`（`themed_arrow_down`/`themed_arrow_up` 9px）合成一个连续控件；弹出列表为 `drop_down_below` + `scrollable`，`container::bordered_box`。
- **触发命令**：无（Enter 提交）。
- **实现位置**：`src/ui/properties.rs::render_edit_choice_row`；字段类型 `PropValue::EditChoice`；处理 `Message::PropEditChoiceToggle`、`PropGeomChoiceChanged`、`PropGeomCommit`。
- **备注**：块 Name 行对常规块可编辑（匿名 `*`、外部参照、含 `|` 的块保持只读），`src/app/properties.rs::entity_property_sections` 注入选项。

### 下拉选择行（Choice / LocalizedChoice）
- **功能简介**：从固定选项列表中选择（材质、打印样式、标注样式、文字样式、箭头、引线线型、视口 UCS/命名视图/比例、图纸级 Misc 选项等），标签经 i18n 本地化。
- **UI 入口**：`属性面板 → 各分组 → <Choice 行>`。
- **样式**：`combo_box`（`choice_combos` 状态，选项为 `LocalizedChoice`），`VARIES_LABEL` 为占位。
- **触发命令**：无。
- **实现位置**：`src/ui/properties.rs::render_choice_row`；字段类型 `PropValue::Choice`；处理 `Message::PropGeomChoiceChanged`。
- **备注**：`LocalizedChoice` 保留 `raw`（写回值）与本地化 `label`；多选时合并两边选项集。

### 混合值占位（*VARIES*）
- **功能简介**：多选对象某个属性不一致时，控件显示占位 `*VARIES*`，提示该值跨对象不一，且选择新值即批量应用。
- **UI 入口**：`属性面板 → 任意属性行（多选且值不同）`。
- **样式**：`VARIES_LABEL = "*VARIES*"`（`src/ui/properties.rs`、`src/app/mod.rs`、`src/entities/common.rs`）。
- **触发命令**：无。
- **实现位置**：`src/app/properties.rs::merge_prop_value`（逐类型返回 Varies 变体）；`src/ui/properties.rs` 各 `*_varies_row`。
- **备注**：合并逻辑对 Color/Lw/Linetype/Layer/Choice/EditChoice/EditText/PlainText/Hyperlink/ReadOnly/BoolToggle/HatchPattern/FieldLw 均有专门处理。

### 超链接行（Hyperlink + “...”）
- **功能简介**：显示对象超链接，点 “...” 打开编辑。
- **UI 入口**：`属性面板 → General → Hyperlink 行`。
- **样式**：`read_only::field` + `button("...")`（`button::secondary`）。
- **触发命令**：无。
- **实现位置**：`src/ui/properties.rs::render_hyperlink_row`；字段类型 `PropValue::Hyperlink`；处理 `Message::PropHyperlinkOpen`。
- **备注**：多选时值变 `*VARIES*`。

### 标注比例行（Annotative scale + “...”）
- **功能简介**：显示对象的注释比例，点 “...” 打开注释比例管理器。
- **UI 入口**：`属性面板 → <注释分组> → Annotative scale 行`。
- **样式**：`read_only::field` + `button("...")`（`button::secondary`），后接 10px 间隔。
- **触发命令**：无。
- **实现位置**：`src/ui/properties.rs::render_annotative_scale_row`；处理 `Message::AnnoObjectScaleOpen`。
- **备注**：仅当 `prop.field == "annotative_scale"` 的 `ReadOnly` 走此分支。

### 填充图案选择器（Hatch Pattern）
- **功能简介**：交互式选择填充图案；弹出宽面板含搜索框与 2 列预览卡片网格。
- **UI 入口**：`属性面板 → Hatch 分组 → Pattern 行（点击）`。
- **样式**：头部 `button`（截断名 + 箭头，选中/打开时边框 `primary.base`）；弹出面板 `PATTERN_PICKER_W = 348`、`PATTERN_PICKER_H = 720`；卡片 `PATTERN_CARD_W = 158`，预览高 `PATTERN_PREVIEW_H = 58`（canvas 绘制）；选中卡片用 `primary.weak` 底 + `primary.base` 2px 边框；搜索框 id `"hatch-pattern-search"`。
- **触发命令**：无。
- **实现位置**：`src/ui/properties.rs::{render_hatch_pattern_row,HatchPatternPreview,filtered_hatch_patterns,hatch_preview_scale,hatch_pattern_matches}`；字段类型 `PropValue::HatchPatternChoice`；处理 `Message::PropHatchPatternPickerToggle/SearchChanged/Focus/Navigate/Confirm/Changed`（支持键盘 ↑↓、Enter 确认）。
- **备注**：面板打开时关闭其它 picker 并聚焦搜索框；无匹配显示 “No matching patterns”。

### 块属性行（AttrText）
- **功能简介**：以属性标签为行标签，编辑块属性值。
- **UI 入口**：`属性面板 → Attributes 分组 → <tag> 行`。
- **样式**：`text_input`，`text_input_style`，内边距 `[3, 6]`；id 由 `prop_attr_field_id(tag)` 生成。
- **触发命令**：无（Enter 提交）。
- **实现位置**：`src/ui/properties.rs::render_attr_row`；字段类型 `PropValue::AttrText`；处理 `Message::PropAttrInput` / `PropAttrCommit`（`on_prop_attr_commit`）。
- **备注**：编辑键为 `FieldKey::Attr(tag)`；选择改变会丢弃未提交缓冲，避免同名 tag 跨对象串值。

### 命名参数行（ParamRow：name + formula + 解算值 + ✕）
- **功能简介**：编辑命名参数名与公式，旁显实时解算值或错误；可删除该参数。
- **UI 入口**：`属性面板 → Parameters 分组 → <参数行>`。
- **样式**：`row![name_input, formula_input, value_label, delete_btn]`；name/formula 各 `FillPortion(3)`，值 `FillPortion(2)` 右对齐；错误时值显 danger 色，并以 danger tooltip 展示错误；删除按钮文本 `✕`（`button::text`）。
- **触发命令**：无（Enter 提交）。
- **实现位置**：`src/ui/properties.rs::render_param_row`；字段类型 `PropValue::ParamRow`；处理 `Message::PropParamInput` / `PropParamCommit` / `PropParamDelete`。
- **备注**：按字段而非逐键提交（`ParameterTable::set` 会拒绝中途公式）；控件 id `props-param-field-{index}-{name|formula}`。

### “+ Add parameter” 行
- **功能简介**：追加一个唯一命名的新参数（`param1`、`param2`…），随后就地重命名/定义。
- **UI 入口**：`属性面板 → Parameters 分组 → “+ Add parameter” 行`。
- **样式**：`button::text`，前缀 “+” 与 “Add parameter”，内边距 `[4, 8]`，宽 Fill。
- **触发命令**：无。
- **实现位置**：`src/ui/properties.rs::render_param_add_row`；字段类型 `PropValue::ParamAddRow`；处理 `Message::PropParamAddNew`。

### 参数值可见性切换行（Values On/Off）
- **功能简介**：全局开关：视口中约束标记是否显示其驱动值/参数名文本。
- **UI 入口**：`属性面板 → Parameters 分组 → Values 行`。
- **样式**：`row![text("Values"), button(可见图标 + On/Off)]`；按钮边框 `background.neutral`，圆角 2。
- **触发命令**：无。
- **实现位置**：`src/ui/properties.rs::render_params_visibility_toggle_row`；字段类型 `PropValue::ParamsVisibilityToggle`；处理 `Message::ShowConstraintValuesChanged`。

### 约束链接行（EntityLink + ✕）
- **功能简介**：Constraints 分组中的一条持久约束行（符号 + 类型名 + 解算值），点击选中该约束涉及的所有实体；冲突/冗余约束以 danger 着色；可删除。
- **UI 入口**：`属性面板 → Constraints 分组 → 约束行`。
- **样式**：整行自身为可点击面（`button`，hover 用 `background.weak`，冲突时 `danger.weak`），边框 `background.neutral` 圆角 2；删除按钮文本 `✕`。
- **触发命令**：无。
- **实现位置**：`src/ui/properties.rs::render_entity_link_row`；字段类型 `PropValue::EntityLink`；处理 `Message::PropConstraintLinkClick(handles)` / `PropConstraintDelete(id)`。
- **备注**：不采用标准 label|value 分列，因为约束行没有独立“值”单元。

### 选择组下拉（Selection group：All / 按类型）
- **功能简介**：多选时按对象类型过滤属性面板（“All (n)” 与 “{Type}(n)”）。
- **UI 入口**：`属性面板 → 标题栏（标题位置的下拉）`。
- **样式**：`combo_box`（`selection_group_combo`），字号 `FONT_SZ`，内边距 `[2, 6]`，宽 Fill。
- **触发命令**：无。
- **实现位置**：`src/app/properties.rs::build_selection_groups`；`src/ui/properties.rs::PropertiesPanel::view`；处理 `Message::PropSelectionGroupChanged`。
- **备注**：分组由 `entity_type_key`/`title_case_word` 生成；选择组改变会 `refresh_properties`。

### 属性面板入口：PROPERTIES 命令（显示/隐藏切换）
- **功能简介**：切换属性面板的显示与隐藏（停靠边缘），并同步 Ribbon 上 Properties 按钮的高亮状态。
- **UI 入口**：`命令输入框 → PROPERTIES`（别名 `PROPS`）。
- **样式**：无独立控件；切换按钮高亮见下条。
- **触发命令**：`PROPERTIES` / `PROPS`；快捷键 `CTRL+1`（Mac `CMD+1`）。
- **实现位置**：`src/app/commands/display.rs`（`"PROPERTIES" | "PROPS"` → `Message::ToggleProperties`）；`src/app/update/mod.rs::ToggleProperties`；快捷键 `src/app/shortcuts.rs`（`ACCEL+1`）。
- **备注**：`show_properties` 控制可见性；默认可见（`DockState::default` 左停靠 Properties）。

### 属性面板入口：View 选项卡 → Palettes 面板 → Properties 按钮
- **功能简介**：Ribbon View 选项卡中的属性面板开关大按钮，按面板可见性高亮。
- **UI 入口**：`Ribbon → View 选项卡 → Palettes 面板 → Properties（LargeTool）`。
- **样式**：`LargeTool`（图标 `properties.svg`，标签“Properties”）；高亮由 `src/ui/ribbon/widgets.rs` 的 `"PROPERTIES" => state.show_properties` 决定；`set_properties(on)` 更新状态。
- **触发命令**：点击发 `ModuleEvent::Command("PROPERTIES")`。
- **实现位置**：`src/modules/view/properties_palette.rs::tool()`；`src/modules/view/mod.rs::ViewModule::ribbon_groups`（Palettes 组，顺序 Tool Palettes → Properties → Sheet Set Manager）；高亮 `src/ui/ribbon/mod.rs::{set_properties,}`、`src/ui/ribbon/widgets.rs`。
- **备注**：Palettes 组布局：`LargeTool(tool_palettes)`、`LargeTool(properties_palette)`、`LargeTool(sheetset)`。

### 属性面板入口：右键上下文菜单 → Properties
- **功能简介**：在视口右键菜单中开关属性面板，并以勾选态显示当前是否打开。
- **UI 入口**：`视口 → 右键 → 上下文菜单 → Properties`。
- **样式**：`MenuItem`，带命令图标 `PROPERTIES`，`checked(props_open)` 勾选态。
- **触发命令**：`MenuAction::Properties` → `Message::ToggleProperties`。
- **实现位置**：`src/ui/popup/context_menu.rs::idle_rows`（`MenuRow::Item(Properties)`）；`src/app/update/context_menu.rs::MenuAction::Properties`。
- **备注**：同菜单还含 “Quick Select...”（`MenuAction::QuickSelect`）与常规选择行。

### 快速属性浮层（Quick Properties，紧凑浮动面板）
- **功能简介**：随选择出现的紧凑浮动只读/可编辑面板，显示同一批属性行，锚定光标位置。
- **UI 入口**：`命令输入框 → QUICKPROPERTIES`（切换）。
- **样式**：`PropertiesPanel::quick_view`：宽 230，标题条同属性面板头，内容 `Length::Shrink`、圆角 3 边框；由 `src/app/view/mod.rs` 在 `quick_properties && !tab.is_start` 时叠加。
- **触发命令**：`QUICKPROPERTIES`。
- **实现位置**：`src/ui/properties.rs::PropertiesPanel::quick_view`；`src/app/commands/display.rs`（`"QUICKPROPERTIES"`）；`src/app/update/mod.rs::ToggleQuickProperties`（记录锚点 `quick_properties_anchor`）；渲染 `src/app/view/mod.rs`。
- **备注**：无选择时 `quick_view` 返回 `None`。

---

## 二、属性面板的内容分组（按源码枚举）

单对象页由 `src/scene/view/dispatch.rs::properties_sectioned` 组装：`General` → （可选）`3D Visualization` → 实体 `geometry_properties(...)` 返回的各分组（或 `fallback_properties` 的 `Geometry`），随后 `src/app/properties.rs::entity_property_sections` 追加若干 doc 相关分组。多选合并后仅保留各实体共有的分组/字段（`merge_sections`）。

### General 分组
- **功能简介**：所有图形对象共有的常规属性。
- **UI 入口**：`属性面板 → General 分组`。
- **样式**：分组头见“属性分组折叠”；行见各控件形态。
- **触发命令**：无。
- **实现位置**：`src/scene/cache/properties.rs::general_section`。
- **备注**：字段固定为 Color、Layer、Linetype、Linetype scale、Plot style（只读）、Lineweight、Transparency（EditChoice，选项 ByLayer/ByBlock）、Hyperlink。

### 3D Visualization 分组
- **功能简介**：材质源设定（Material）。
- **UI 入口**：`属性面板 → 3D Visualization 分组`。
- **样式**：`Choice` 下拉行。
- **触发命令**：无。
- **实现位置**：`src/scene/cache/properties.rs::visualization_section`；`src/app/properties.rs::entity_property_sections`（把 Material 升级为含文档素材名的 Choice）。
- **备注**：对 Block/BlockEnd/Seqend/Leader/Wipeout/Underlay/Unknown/ViewBorder、以及点云扩展实体不显示。

### Geometry 分组（及 fallback）
- **功能简介**：实体几何参数（位置、尺寸、角度等），按实体类型不同。
- **UI 入口**：`属性面板 → Geometry 分组`。
- **样式**：坐标行可能折叠为摘要行；其余为可编辑/只读行。
- **触发命令**：无。
- **实现位置**：各实体 `geometry_properties(...)`（`crate::entities`）；无几何属性时用 `src/scene/cache/properties.rs::fallback_properties`（仅 `Type` 只读行）。
- **备注**：`dispatch::properties_sectioned` 还会为有厚度的实体在 Geometry 分组补 `Thickness` 行（若未存在）。

### Color Book 分组（命名颜色）
- **功能简介**：当对象颜色来自颜色簿时，显示颜色簿/颜色名/颜色值（只读）。
- **UI 入口**：`属性面板 → Color Book 分组`。
- **样式**：只读文本行。
- **触发命令**：无。
- **实现位置**：`src/app/properties.rs::entity_property_sections`（`Book`/`Color Name`/`Color` 三行）。
- **备注**：无命名颜色时不生成；主 Color 行改用 `PropValue::NamedColorChoice`。

### {Full/Face/Edge} Visual Style 分组
- **功能简介**：显示对象引用的视觉样式对象细节（句柄、描述、类型、面/边、扩展光照、属性袋等，只读）。
- **UI 入口**：`属性面板 → “Full/Face/Edge Visual Style” 分组`。
- **样式**：只读文本行。
- **触发命令**：无。
- **实现位置**：`src/app/properties.rs::entity_property_sections`（`style_handles` 循环 + `visual_style_properties_text`）。
- **备注**：仅当对象持有有效视觉样式句柄时出现。

### Mass Properties 分组
- **功能简介**：实心/曲面/网格的质量属性（顶点/三角面/表面积/质心/惯性矩/主方向/主矩/惯量积/回转半径/体积等，只读）。
- **UI 入口**：`属性面板 → Mass Properties 分组`。
- **样式**：只读文本行（部分属性仅在网格完成时显示）。
- **触发命令**：无。
- **实现位置**：`src/app/properties.rs::entity_property_sections`（`Solid3D/Body/Surface/Mesh/PolygonMesh/PolyfaceMesh` 分支）。
- **备注**：紧凑实心（compact solid）仅显示 Centroid 及部分闭合属性（`retain_compact_solid_sections`）。

### Extended Data 分组（XDATA）
- **功能简介**：逐应用显示扩展数据记录值（只读）。
- **UI 入口**：`属性面板 → Extended Data 分组`。
- **样式**：只读文本行（标签为应用名）。
- **触发命令**：无。
- **实现位置**：`src/app/properties.rs::entity_property_sections`。
- **备注**：无 xdata 时不出现。

### Object Data / Associative Data 分组
- **功能简介**：来自对象数据表的属性；被参数约束关联时隐藏 Associative Data 分组。
- **UI 入口**：`属性面板 → Object Data / Associative Data 分组`。
- **样式**：见各行控件形态。
- **触发命令**：无。
- **实现位置**：`src/app/properties.rs::entity_property_sections`（`crate::entities::object_data::sections`；约束时 `retain` 移除 `Associative Data`）。
- **备注**：紧凑实心时不追加。

### 块参照专属：Reference / Scale / 外部参照行
- **功能简介**：块参照显示可编辑 Name（EditChoice）、Block unit、Unit factor、Scale/Uniform scale（或 Scale X/Y/Z），外部参照还显示 Saved Path、Layer overrides 等。
- **UI 入口**：`属性面板 → Geometry/Reference 分组`。
- **样式**：EditChoice、只读文本、BoolToggle（Uniform scale）各行。
- **触发命令**：无。
- **实现位置**：`src/app/properties.rs::entity_property_sections`（`Insert` 分支 + `xref_rows`）。
- **备注**：Uniform scale 逻辑同“布尔切换行”。

### 标注专属：DimStyle 与样式分组
- **功能简介**：尺寸标注样式下拉、Association status / Associative、以及解析后的标注样式分组（Lines & Arrows、Text、Fit、Units、Tolerances），并按实体级覆盖优先注入标注线/界线/文字颜色等。
- **UI 入口**：`属性面板 → General / Geometry 分组（Dimension 相关行与分组）`。
- **样式**：Choice 下拉、只读文本、颜色行。
- **触发命令**：无。
- **实现位置**：`src/app/properties.rs::entity_property_sections`（`Dimension` 分支）；`crate::entities::dimension::style_sections`；`src/entities/dim_override.rs`。
- **备注**：Association status 取值：Nonassociative / Broken reference / Unresolved reference / Partially associated / Associated。

### 引线专属（Leader / MultiLeader）
- **功能简介**：引线显示标注样式下拉、箭头块、箭头大小、引线线宽、标注线颜色、文字偏移、文字垂直位置、整体比例等；多重引线显示样式/文字样式/箭头块/引线线型/块内容选择。
- **UI 入口**：`属性面板 → 引线相关分组`。
- **样式**：Choice 下拉、EditText、FieldLw、颜色行。
- **触发命令**：无。
- **实现位置**：`src/app/properties.rs::entity_property_sections`（`MultiLeader` 与 `Leader` 分支）。
- **备注**：MLeader 选项列表含绘图中的 MultiLeaderStyle、文字样式、下划线开头箭头块、ByBlock + 文档线型、块记录名。

### 视口专属（Viewport）
- **功能简介**：图纸视口显示 Frozen Layers、UCS Name、Named View，以及来自图纸比例列表的视口比例下拉。
- **UI 入口**：`属性面板 → 视口 Geometry 分组（末尾追加）`。
- **样式**：PlainText / Choice 行。
- **触发命令**：无。
- **实现位置**：`src/app/properties.rs::entity_property_sections`（`Viewport` 分支）。
- **备注**：选项来自文档的 `ucss`、`views`、`scale_list()`。

### 打印/Plot 相关分组（图纸级与实体级）
- **功能简介**：无选择（图纸级）时显示 Plot style / Plot style table / 附加布局 / 表格类型；实体级在 General 显示 Plot style（CTB 模式只读 ByColor）。
- **UI 入口**：`属性面板 → Plot style 分组（无选择）/ General 行（对象）`。
- **样式**：Choice 下拉、只读文本（含 tooltip）。
- **触发命令**：无。
- **实现位置**：`src/app/properties.rs`（无选择页 `Plot style` 分组 + `entity_property_sections` 的 `plotstyle_mode` 分支）。
- **备注**：选项含 None + 可用 CTB 名（`crate::io::plot_style::available_ctb_names`）。

### 无选择（图纸级）页面分组：General / 3D Visualization / Plot style / View / Misc / Parameters
- **功能简介**：未选中任何对象时，属性面板显示图纸级设置（版本、当前颜色/图层/线型/线宽/透明度/厚度、当前材质、打印样式、视图中心/尺寸、注释比例、UCS 图标、视觉样式、命名参数表）。
- **UI 入口**：`属性面板 → 内容区（无选择）`。
- **样式**：各控件形态；View 分组不做坐标合并。
- **触发命令**：无。
- **实现位置**：`src/app/properties.rs::refresh_properties`（`0 =>` 分支，sections 构造）。
- **备注**：标题为 “No selection”；View 含 Center X/Y/Z、Height、Width 只读行。

### 多选聚合与超限降级
- **功能简介**：少量多选按类型聚合共有分组/字段并显示混合值；超过 `MAX_PROP_AGGREGATE = 2000` 个对象时仅显示计数标题，跳过 O(n) 聚合（批量编辑仍走 Ribbon）。
- **UI 入口**：`属性面板 → 内容区（多选）`。
- **样式**：计数标题 `t!("%{count} objects selected")`。
- **触发命令**：无。
- **实现位置**：`src/app/properties.rs::{aggregate_sections,merge_sections,merge_prop_value,aggregate_solid_history_sections,MAX_PROP_AGGREGATE}`；`src/app/properties.rs::refresh_properties`（`n if n > MAX_PROP_AGGREGATE` 分支）。
- **备注**：全部为 Hatch 的多选额外累加 `cumulative_area`。

### 属性行焦点高亮与全选（Active field）
- **功能简介**：点击进入可编辑行时整行以主色浅底高亮，并自动全选该字段值；焦点移动到其它控件时清除。
- **UI 入口**：`属性面板 → 任意可编辑行`。
- **样式**：`prop_row_with_active`：active 时标签/值列底为 `primary.weak`，行边框 `primary.base`。
- **触发命令**：无。
- **实现位置**：`src/ui/properties.rs::{prop_row_with_active,active_key_focused,build_field_key_map,sync_active_field_task}`；`src/app/update/mod.rs` 的 `Message::PropPointerPressed` 与 `PropSyncActive`。
- **备注**：`sync_active_field_task` 用 iced operation 扫描聚焦控件；仅当焦点落到新字段时全选。

### 锁定对象的属性只读化
- **功能简介**：对象所在图层被锁定时，面板将其可编辑值折叠为只读文本，避免误编辑。
- **UI 入口**：`属性面板 → 锁定图层对象 → 任意行`。
- **样式**：`ReadOnly` 行。
- **触发命令**：无。
- **实现位置**：`src/app/properties.rs`（`property.field = "locked_read_only"; value = PropValue::ReadOnly(text)`，约 3510–3555 行）。
- **备注**：Constraints/Parameters 分组的链接/参数不因锁定而改变，保留可读文本。

---

## 三、图层管理器窗口（Layer Manager，浮动窗口）

图层管理器是独立浮动窗口（`ModalKind::Layers`，尺寸 900×360，`src/app/view/modal.rs`），内容由 `src/ui/window/layers.rs::LayerPanel::view_window` 渲染。其本身不属于 Dock 停靠面板（`PanelId` 无 Layers），但图层下拉浮层是两个入口之一（见下）。

### 图层管理器窗口开关（LAYERS 命令 / LAYER 命令）
- **功能简介**：打开/关闭图层特性管理器浮动窗口。
- **UI 入口**：`Ribbon → Draw 选项卡 → Layers 面板 → Layers（LargeTool）`；或 `命令输入框 → LAYERS / LAYER / LA`。
- **样式**：`LargeTool`（图标 `layers/panel.svg`，标签“Layers”）。
- **触发命令**：`LAYERS`、`LAYER`、`LA`。
- **实现位置**：`src/modules/draw/mod.rs`/`src/modules/draw/layers/panel.rs::tool()`（`ModuleEvent::ToggleLayers`）；`src/app/commands/fileops.rs`（`"LAYERS"` → `Message::ToggleLayers`）；`src/app/commands/layerprops.rs`（`"LAYER"` 分支）；`src/app/update/mod.rs::ToggleLayers`（切换 `ModalKind::Layers`）。
- **备注**：再次触发或点击关闭会 `deactivate_tool_if("LAYERS")` 并关闭窗口。

### 图层管理器工具栏：New（新建图层）
- **功能简介**：新建唯一命名（`Layer1`、`Layer2`…）的图层，并立即进入重命名编辑。
- **UI 入口**：`图层管理器 → 工具栏 → New（图标 PLUS + “New”）`。
- **样式**：`toolbar_btn`：行图标 12px + 文本 11px，`background.weak` 底、1px `background.neutral` 边框、圆角 3，hover/pressed 用 `background.strong`，内边距 `[4, 10]`。
- **触发命令**：无（对应 `Message::LayerNew`）。
- **实现位置**：`src/ui/window/layers.rs::view_content`（`toolbar_btn(PLUS, t!("New"), Message::LayerNew)`）；`src/app/update/command.rs::on_layer_new`。
- **备注**：新层分配真实句柄（避免 DWG 保存丢层）；加入后自动滚动到该行（`LAYER_TABLE_SCROLL_ID`）。

### 图层管理器工具栏：Delete（删除图层）
- **功能简介**：删除选中图层；图层 “0” 与当前图层不可删除；非空图层的对象会被一并删除（先警告确认）。
- **UI 入口**：`图层管理器 → 工具栏 → Delete（图标 TRASH + “Delete”）`。
- **样式**：`toolbar_btn_cond`（禁用时图标 `themed_disabled`、文本 42% 透明且不响应）；选中且非 “0” 时启用。
- **触发命令**：无（`Message::LayerDelete` / `Message::LayerDeleteConfirm`）。
- **实现位置**：`src/ui/window/layers.rs::view_content`；`src/app/update/command.rs::{on_layer_delete,on_layer_delete_confirm}`；警告模态 `ModalKind::LayerDeleteWarning`。
- **备注**：空层直接删除；非空层弹出确认，确认后先移除图层记录再 `erase_entities`。

### 图层管理器工具栏：Set Current（置为当前）
- **功能简介**：将选中图层设为当前图层（同步文档 CLAYER 与各 UI 镜像）。
- **UI 入口**：`图层管理器 → 工具栏 → Set Current（图标 CHECK + “Set Current”）`。
- **样式**：`toolbar_btn_cond`；选中层不等于当前层时启用。
- **触发命令**：无（`Message::LayerSetCurrent`）。
- **实现位置**：`src/ui/window/layers.rs::view_content`；`src/app/update/command.rs::on_layer_set_current`。
- **备注**：更新 `header.current_layer_name`/`current_layer_handle`、`active_layer`、`layers.current_layer`、`ribbon.active_layer`。

### 图层管理器：名称筛选搜索框
- **功能简介**：按名称即时过滤显示行（大小写不敏感）。
- **UI 入口**：`图层管理器 → 工具栏右端 → “Search…” 输入框`。
- **样式**：`text_input`，字号 `FONT_SZ`，内边距 `[3, 6]`，宽 `Length::Fixed(180.0)`，`table_input_style`（聚焦边框 `primary.base`，圆角 2）。
- **触发命令**：无（`Message::LayerManagerFilterChanged`）。
- **实现位置**：`src/ui/window/layers.rs::view_content`；`src/app/update/mod.rs::LayerManagerFilterChanged`；过滤逻辑 `view_content` 内 `layer.name.to_lowercase().contains(&filter)`。
- **备注**：状态字段 `LayerPanel::filter`。

### 图层管理器：列头排序
- **功能简介**：点击列头按该列排序，再次点击同一列反转方向；激活列显示上/下箭头。
- **UI 入口**：`图层管理器 → 列头 → Name / On / Freeze / Lock / Plot / Color / Linetype / Lineweight / Transparency`。
- **样式**：`sortable_header`（文本 10px + `themed_arrow_up`/`themed_arrow_down` 8px；`layer_header_button_style`，hover 用 `background.strong`，`background.weak` 底）。
- **触发命令**：无（`Message::LayerSort(col)`）。
- **实现位置**：`src/ui/window/layers.rs::{sortable_header,LayerSortCol}`；`src/app/update/mod.rs::LayerSort`；排序 `LayerPanel::{sort_by,apply_sort}`。
- **备注**：默认按 Name 升序（`LayerPanel::default`）；等值以名称稳定兜底；排序后多选按名称重解析。

### 图层管理器：Name 列宽拖拽
- **功能简介**：拖动 Name 列后的分隔条调整名称列宽。
- **UI 入口**：`图层管理器 → 列头 → Name 与 Status 之间的竖直分隔条`。
- **样式**：2×14 的 `background.neutral` 竖条，鼠标指针 `ResizingHorizontally`。
- **触发命令**：无（`Message::LayerNameColGrab`）。
- **实现位置**：`src/ui/window/layers.rs::view_content`；`src/app/update/mod.rs::LayerNameColGrab`；宽度值 `app.layer_name_col_w` 传入 `view_window`。
- **备注**：宽度状态由应用层维护。

### 图层行：Status（当前层指示）
- **功能简介**：状态列以勾选图标标记当前图层，否则显示圆点。
- **UI 入口**：`图层管理器 → 行 → Status 列`。
- **样式**：当前层 `themed_success(CHECK, 13.0)`；非当前层 `themed_secondary(DOT, 9.0)`；列宽 50，水平居中。
- **触发命令**：无。
- **实现位置**：`src/ui/window/layers.rs::layer_row`（`status_dot`）。
- **备注**：当前层名取自 `LayerPanel::current_layer`。

### 图层行：Name（重命名）
- **功能简介**：点击名称单元格进入内联编辑；提交后重命名图层及其对象引用。
- **UI 入口**：`图层管理器 → 行 → Name 单元格（点击）`。
- **样式**：非编辑态为 `button`（文本可能 `text_util::elide`，截断时 hover 显示完整名 tooltip `FollowCursor`，`name_tip` 用 `background.strong` 底）；编辑态为 `text_input`（`LayerRenameEdit` / `LayerRenameCommit`）。
- **触发命令**：无（`Message::LayerRenameStart` / `LayerRenameEdit` / `LayerRenameCommit`）。
- **实现位置**：`src/ui/window/layers.rs::{layer_row,name_tip}`；`src/app/update/command.rs::on_layer_rename_commit`。
- **备注**：空名或不改名不提交；重命名保留 undo 快照；名称预算约每字符 6px。

### 图层行：On（开/关）
- **功能简介**：切换图层可见性（开/关）；批量作用于多选。
- **UI 入口**：`图层管理器 → 行 → On 列图标`。
- **样式**：`icons::layer_visible(bool)`，图标尺寸 `ICON_SZ = ROW_H*0.62`（≈16px）；`svg_btn` 用 `layer_cell_button_style`（hover `background.strong`，选中 `primary.weak`，奇偶行交替基色）。
- **触发命令**：无（`Message::LayerToggleVisible(idx)`）。
- **实现位置**：`src/ui/window/layers.rs::layer_row`；`src/app/update/mod.rs::LayerToggleVisible`；`begin_layer_undo(i,"LAYER OFF/ON",...)`。
- **备注**：目标集由 `layer_row_action_targets` 决定（点击行在多选内则批量）。

### 图层行：Freeze（冻结/解冻）
- **功能简介**：切换图层冻结状态。
- **UI 入口**：`图层管理器 → 行 → Freeze 列图标`。
- **样式**：`icons::layer_freeze(bool)`，`ICON_SZ`。
- **触发命令**：无（`Message::LayerToggleFreeze(idx)`）。
- **实现位置**：`src/ui/window/layers.rs::layer_row`；`src/app/update/mod.rs::LayerToggleFreeze`（`dl.freeze()`/`dl.thaw()`，`begin_layer_undo(...,"LAYER FREEZE",...)`）。
- **备注**：批量作用于选择集。

### 图层行：Lock（锁定/解锁）
- **功能简介**：切换图层锁定状态（影响可编辑性，不改变渲染几何）。
- **UI 入口**：`图层管理器 → 行 → Lock 列图标`。
- **样式**：`icons::layer_lock(bool)`，`ICON_SZ`。
- **触发命令**：无（`Message::LayerToggleLock(idx)`）。
- **实现位置**：`src/ui/window/layers.rs::layer_row`；`src/app/update/mod.rs::LayerToggleLock`（`begin_layer_undo(...,"LAYER LOCK/UNLOCK",...)`，并 `refresh_properties`）。
- **备注**：批量作用于选择集。

### 图层行：Plot（打印/不打印）
- **功能简介**：切换图层是否打印。
- **UI 入口**：`图层管理器 → 行 → Plot 列图标`。
- **样式**：可打印 `themed(PRINT, ICON_SZ)`；不可打印为 PRINT 叠加 `themed_danger(CLOSE, ICON_SZ*0.65)`。
- **触发命令**：无（`Message::LayerTogglePlot(idx)`）。
- **实现位置**：`src/ui/window/layers.rs::layer_row`；`src/app/update/mod.rs::LayerTogglePlot`（`begin_layer_undo(...,"LAYER PLOT/NOPLOT",...)`）。
- **备注**：批量作用于选择集；排序刷新。

### 图层行：Color（颜色选择）
- **功能简介**：为图层选择颜色；支持快捷色板、完整调色板窗口；批量作用于多选。
- **UI 入口**：`图层管理器 → 行 → Color 单元格（点击）`。
- **样式**：`color_selector_with_name`（无 ByLayer/ByBlock 逻辑项）；列宽 `COL_COLOR = 90`；下拉 `Message::LayerColorSet`，切换 `Message::LayerColorPickerToggle(index)`，“More Colors…” 发 `Message::OpenColorWindow(ColorPickTarget::Layer(index), color)`。
- **触发命令**：无（`Message::LayerColorSet` / `LayerColorPickerToggle` / `LayerColorMorePalette`）。
- **实现位置**：`src/ui/window/layers.rs::layer_row`；`src/app/update/mod.rs::{LayerColorSet,LayerColorPickerToggle}`；`begin_layer_undo(...,"LAYER COLOR",...)`。
- **备注**：图层存具体颜色（非 ByLayer/ByBlock）；设置颜色会清 `color_name`/`book_name` 并刷新依赖。

### 图层行：Linetype（线型下拉）
- **功能简介**：为图层选择线型（含 ASCII 预览）。
- **UI 入口**：`图层管理器 → 行 → Linetype 单元格`。
- **样式**：`combo_box` + `wide_menu`（菜单宽 `LINETYPE_MENU_W = 220`）；仅锚点行显示可编辑下拉，其它行显示只读文本（`muted_style`）；列宽 `COL_LT = 110`。
- **触发命令**：无（`Message::LayerLinetypeSet`）。
- **实现位置**：`src/ui/window/layers.rs::layer_row`；`src/app/update/mod.rs::LayerLinetypeSet`（`begin_layer_undo(...,"LAYER LINETYPE",...)`）。
- **备注**：批量作用于选择集（`selected_layer_names`）。

### 图层行：Lineweight（线宽下拉）
- **功能简介**：为图层选择线宽。
- **UI 入口**：`图层管理器 → 行 → Lineweight 单元格`。
- **样式**：`combo_box`（选项 `lw_options()`）；仅锚点行可编辑；列宽 `COL_LW = 90`。
- **触发命令**：无（`Message::LayerLineweightSet`）。
- **实现位置**：`src/ui/window/layers.rs::layer_row`；`src/app/update/mod.rs::LayerLineweightSet`（`begin_layer_undo(...,"LAYER LINEWEIGHT",...)`）。
- **备注**：批量作用于选择集。

### 图层行：Transparency（透明度输入）
- **功能简介**：输入 0–90 的透明度百分比；空输入按 0 处理；非法输入忽略。
- **UI 入口**：`图层管理器 → 行 → Transparency 单元格`。
- **样式**：`text_input`，透明背景，`table_input_style`，列宽 `COL_TRANS = 80`。
- **触发命令**：无（`Message::LayerTransparencyEdit(idx, s)`）。
- **实现位置**：`src/ui/window/layers.rs::layer_row`；`src/app/update/mod.rs::LayerTransparencyEdit`（`clamp(0,90)`，批量作用于 `layer_row_action_targets`）。
- **备注**：写回 `Transparency::from_percent(v/100)`；不推 undo（直接标记 dirty）。

### 图层行：视口冻结列（VP Freeze，图纸布局）
- **功能简介**：在含视口的图纸布局中，为每个视口显示一列，切换该图层在该视口中的冻结。
- **UI 入口**：`图层管理器 → 行 → <视口标签> 列图标`。
- **样式**：列头为视口标签（字号 10，`muted_style`，宽 `COL_ICON = 44`）；单元格为 `icons::layer_freeze(bool)` 按钮。
- **触发命令**：无（`Message::LayerToggleVpFreeze(layer_idx, vp_col_idx)`）。
- **实现位置**：`src/ui/window/layers.rs::{VpCol,layer_row}`；`src/app/update/command.rs::on_layer_toggle_vp_freeze`（推 undo `"VPLAYER"`，修改 `Viewport::frozen_layers`）。
- **备注**：仅当 `LayerPanel::vp_cols` 非空（`sync_with_viewports` 从场景视口列表填充）时出现。

### 图层行选择（单选 / Ctrl 多选 / Shift 范围）
- **功能简介**：单击选中一行（锚点），Ctrl/Cmd 点击切换多选，Shift 点击按锚点到该行扩展范围；批量属性修改作用于所有选中行。
- **UI 入口**：`图层管理器 → 行（点击 / Ctrl+点击 / Shift+点击）`。
- **样式**：选中行底为 `primary.weak`，text 用配对色；奇偶行交替 `background.base`/`background.weak`；行高 `ROW_H`。
- **触发命令**：无（`Message::LayerSelect(idx)`）。
- **实现位置**：`src/ui/window/layers.rs::layer_row`；`src/app/update/mod.rs::LayerSelect`（使用全局 `shift_down`/`ctrl_down`）。
- **备注**：点击已选行中的属性下拉时保留多选，以支持批量编辑。

### 图层管理器行高亮与当前层区分
- **功能简介**：选中行高亮，当前层以勾选图标标记；二者可同时体现。
- **UI 入口**：`图层管理器 → 行`。
- **样式**：`layer_cell_button_style`（hover → `background.strong`；选中 → `primary.weak`；否则交替基色）。
- **触发命令**：无。
- **实现位置**：`src/ui/window/layers.rs::layer_cell_button_style`。
- **备注**：仅锚点行显示可编辑 linetype/lineweight 下拉（`is_anchor`）。

### 图层同步与系统层过滤
- **功能简介**：图层窗口从文档图层表重建，隐藏以 `*` 开头的系统图层（如 `*ADSK_CONSTRAINTS`），并同步每视口冻结状态。
- **UI 入口**：`图层管理器 → 行列表`。
- **样式**：无独立控件。
- **触发命令**：无。
- **实现位置**：`src/ui/window/layers.rs::LayerPanel::sync_with_viewports`；调用方 `src/app/layers.rs::refresh_layer_panel`。
- **备注**：重建时按名称保留当前选择。

### 图层管理器关闭
- **功能简介**：关闭浮动窗口。
- **UI 入口**：`图层管理器 → 窗口关闭按钮 / 再次触发 LAYERS`。
- **样式**：浮动窗口标题栏关闭按钮（`sized_flow` 提供）。
- **触发命令**：`LAYERS`（再次）。
- **实现位置**：`src/app/update/mod.rs::ToggleLayers`。
- **备注**：关闭时重设模态几何。

---

## 四、停靠系统（Dock）

停靠系统由 `src/ui/dock.rs`（纯布局/几何）与 `src/app/update/dialog.rs::on_dock`、`src/app/view/mod.rs::build_edge_stack`（渲染）共同实现。

### 停靠面板集合（PanelId）
- **功能简介**：应用已知的可停靠面板枚举，每项有标题、默认宽度与最大宽度。
- **UI 入口**：无独立控件（系统定义）。
- **样式**：`PanelId`：Properties(“Properties”，默认 250)、BlockPalette(“Block Palette”，260)、ExternalReferences(“External References”，460，最大为共享上限的 2 倍)、Browser(“Browser”，230)、NodeGraph(“Node Graph”，220)、PointCloudManager(“Point Cloud Manager”，280)。
- **触发命令**：无。
- **实现位置**：`src/ui/dock.rs::{PanelId,title,default_width,max_width}`。
- **备注**：`ensure_settings` 为每个已知面板补齐设置项，兼容旧配置。

### 边缘停靠栈（左侧 / 右侧）
- **功能简介**：任意数量的可停靠面板以有序垂直栈锚定在视口左或右边缘；布局持久化（哪侧、顺序、宽度、是否自动折叠）。
- **UI 入口**：`工作区 → 视口左/右边缘列`。
- **样式**：`DockState { left, right, panels }`；默认 `left=[Properties]`、`right=[BlockPalette]`；每面板 `DockPanel { width, auto_collapse }`；边缘面板 `frame` 背景 `background.base`，1px `background.neutral` 边框、0 圆角、内边距 6。
- **触发命令**：无。
- **实现位置**：`src/ui/dock.rs::{DockState,stack,location,settings,width}`；渲染 `src/app/view/mod.rs::build_edge_stack`。
- **备注**：列宽等于当前显示面板中最宽者（`build_edge_stack` 的 `col_w`）。

### 拖拽重新停靠（DockGrab / DragMove / DragRelease）
- **功能简介**：按住面板标题条拖动，可把面板移到左/右边缘并在栈内重新排序；拖动时显示边缘着色、面板幽灵与插入线预览。
- **UI 入口**：`面板标题条（按住拖动）`。
- **样式**：拖动预览：边缘整列 `primary.weak`（透明度 0.35）着色；幽灵面板为面板原尺寸，2px `primary.base` 边框，头部 `primary.base` 底，顶部 3px `primary.base` 插入线（`src/app/view/mod.rs`）。
- **触发命令**：无（`DockMsg::DockGrab/DragMove/DragRelease`）。
- **实现位置**：`src/ui/dock.rs::{DockMsg,drop_index}`；`src/app/update/dialog.rs::on_dock`；`src/app/view/mod.rs` 拖动预览分支。
- **备注**：落点侧按指针 x 与窗口中线比较；索引按指针 y 与可用高度计算（`drop_index`）；释放时 `dock.dock(...)` 成功则保存配置。

### 面板宽度拖拽与重置（ResizeGrab / WidthReset）
- **功能简介**：拖动面板与视口之间的分隔条调整宽度；双击分隔条恢复默认宽度。
- **UI 入口**：`停靠面板 → 靠视口侧的分隔条（拖动 / 双击）`。
- **样式**：`dock_divider`：5px 宽 `background.neutral` 竖条，鼠标指针 `ResizingHorizontally`。
- **触发命令**：无（`DockMsg::ResizeGrab` / `WidthReset`）。
- **实现位置**：`src/app/view/mod.rs::dock_divider`；`src/app/update/dialog.rs::on_dock`；`src/ui/dock.rs::{DOCK_MIN_W,DOCK_MAX_W,set_width,reset_width,width}`。
- **备注**：宽度限制 `DOCK_MIN_W = 200`、`DOCK_MAX_W = 600`；实际宽度还受窗口宽度 45% 上限约束；ExternalReferences 允许双倍最大宽度。

### 自动隐藏 / 钉住（AutoCollapseToggle）
- **功能简介**：钉住（auto_collapse）后，面板收起为边缘窄标签条，仅在悬停时展开为完整面板；取消钉住则始终展开。
- **UI 入口**：`面板标题条 → PIN 按钮（Auto）`。
- **样式**：钉住时 PIN 图标用 `themed_primary_weak_text` + `primary.weak` 底 + `primary.base` 1px 边框；未钉住用 `themed_secondary`。
- **触发命令**：无（`DockMsg::AutoCollapseToggle`）。
- **实现位置**：`src/ui/dock.rs::{title_bar,DockState::auto_collapse,set_auto_collapse}`；`src/app/update/dialog.rs::on_dock`；`src/app/view/mod.rs::{build_edge_stack,rail_slice}`。
- **备注**：钉住状态持久化到配置。

### 自动隐藏标签条（Rail / Tab strip）
- **功能简介**：所有钉住（自动折叠）面板在边缘列出窄竖条标签，各占 1/N 高度，可悬停展开或拖动。
- **UI 入口**：`工作区 → 视口边缘 → 窄竖条（标签条）`。
- **样式**：`DOCK_RAIL_W = 28.0`；竖排文字（`VBarLabel` canvas，左侧顺时针/右侧逆时针）；激活时底 `primary.weak` + `primary.base` 边框，拖拽时 `primary.weak`（0.55），其它 `background.base`，边框 `background.neutral`。
- **触发命令**：无（`DockMsg::DockGrab` / `Hover`）。
- **实现位置**：`src/app/view/mod.rs::rail_slice`。
- **备注**：点击标签条即开始拖拽，悬停发 `Hover` 展开；指针移出边缘发 `HoverExit` 收起。

### 悬停展开与收起（Hover / HoverExit）
- **功能简介**：指针悬停在自动折叠面板的标签上时展开该面板；指针离开边缘列时收起。
- **UI 入口**：`工作区 → 边缘标签条（悬停 / 移出）`。
- **样式**：展开的面板浮在展开栈末尾（最上层）。
- **触发命令**：无（`DockMsg::Hover` / `HoverExit`）。
- **实现位置**：`src/app/update/dialog.rs::on_dock`；`src/app/view/mod.rs::build_edge_stack`。
- **备注**：拖拽/调整宽度时忽略 Hover/HoverExit，避免误折叠。

### 关闭面板（Close）
- **功能简介**：关闭/隐藏停靠面板；对 Properties 会同步关闭 ribbon 高亮。
- **UI 入口**：`停靠面板 → 标题条 → CLOSE 按钮（Close）`。
- **样式**：`themed_secondary(CLOSE, 12.0)`，`button::subtle`，tooltip “Close” 字号 10。
- **触发命令**：无（`DockMsg::Close`）。
- **实现位置**：`src/ui/dock.rs::title_bar`；`src/app/update/dialog.rs::on_dock`（按 `PanelId` 设 `show_* = false`）。
- **备注**：隐藏面板保留其栈槽位但不占据屏幕空间（拖动预览只计可见面板）。

### 停靠面板内容分派（Properties / Block Palette / Xref / Browser / NodeGraph / PointCloud）
- **功能简介**：将各 `PanelId` 映射为对应面板视图，并附可调宽分隔条。
- **UI 入口**：`停靠面板 → 内容区`。
- **样式**：每面板接收 `width` 与 `auto_collapse`；左侧面板 `row![panel, divider]`，右侧 `row![divider, panel]`。
- **触发命令**：无。
- **实现位置**：`src/app/view/mod.rs::expanded_panel`（分派到 `tab.properties.view`、`block_palette::view`、`xref_manager.view`、`browser::view`、`tab.graph.panel`、`pc_manager::view`）。
- **备注**：Properties 与其它面板共用同一停靠框架。

### 停靠布局持久化
- **功能简介**：面板的停靠侧、顺序、宽度与自动折叠状态写入配置，跨重启保留。
- **UI 入口**：无。
- **样式**：`DockState` 序列化（`DockPanel`/`panels` 映射）。
- **触发命令**：无。
- **实现位置**：`src/ui/dock.rs::DockState`（`Serialize/Deserialize`）；`src/app/update/dialog.rs` 中 `save_config()` 调用。
- **备注**：`Config` 里的 `DockSide` 见 `src/app/config.rs`。

### 其它面板停靠入口（Xref / Block Palette / PointCloud 等）
- **功能简介**：通过对应命令打开时把面板停靠到右边缘并展开。
- **UI 入口**：`命令输入框 → EXTERNALREFERENCES / BLOCKPALETTE 等`。
- **样式**：同停靠框架。
- **触发命令**：如 `EXTERNALREFERENCES`。
- **实现位置**：`src/app/update/mod.rs::ToggleXrefManager` 等（`self.dock.dock(id, DockSide::Right, usize::MAX)`）；另见 `src/app/commands/{blocks,pdf_underlay,layerprops}.rs`、`src/app/node_graph.rs`。
- **备注**：`ToggleXrefManager` 若已停靠则仅展开并刷新。

### 清屏模式（CLEANSCREEN，隐藏周围面板）
- **功能简介**：一键折叠周围面板以获得完整画布。
- **UI 入口**：`命令输入框 → CLEANSCREEN`（快捷键 `CTRL+0`）。
- **样式**：无。
- **触发命令**：`CLEANSCREEN`。
- **实现位置**：`src/app/commands/display.rs`（`"CLEANSCREEN"` → `Message::ToggleCleanScreen`）；快捷键 `src/app/shortcuts.rs`。
- **备注**：与属性面板显隐独立。

---

## 五、图层状态管理器窗口（Layer State Manager）

浮动窗口（`ModalKind::LayerStateManager`，720×420，`src/app/view/modal.rs`），列表视图 `src/ui/window/layer_state_manager.rs::view_window`，编辑器视图 `view_editor`。由 `LAYERSTATE`/`LAS`/`LMAN` 命令打开，也由图层下拉底部的 “Layer State Manager…” 行打开。

### 图层状态管理器开关
- **功能简介**：打开图层状态管理器窗口，管理图纸内保存的图层状态。
- **UI 入口**：`命令输入框 → LAYERSTATE / LAS / LMAN`；或 `图层下拉 → Layer State Manager…`。
- **样式**：浮动窗口。
- **触发命令**：`LAYERSTATE`、`LAS`、`LMAN`。
- **实现位置**：`src/app/commands/layers.rs`（`"LAYERSTATE" | "LAS" | "LMAN"` → `Message::LayerStateManagerOpen`）；`src/app/update/mod.rs::LayerStateManagerOpen`；窗口 `src/ui/window/layer_state_manager.rs::view_window`。
- **备注**：Start（欢迎）标签无文档时提示 “Open or create a drawing to manage layer states.”。

### 状态列表与搜索
- **功能简介**：左侧列出所有图层状态，支持按名称/说明搜索；每行显示状态名与说明（或 “%{n} layers”）。
- **UI 入口**：`图层状态管理器 → 左侧列表 → 搜索框 / 状态行`。
- **样式**：搜索框 “Search layer states…”（字号 11，内边距 `[5, 8]`）；列表行 `button`（选中 `button::primary`，否则 `button::subtle`），字号 12 + 10 副标题。
- **触发命令**：无（`Message::LayerStateManagerFilter` / `LayerStateManagerSelect`）。
- **实现位置**：`src/ui/window/layer_state_manager.rs::view_window`；`src/app/update/mod.rs::{LayerStateManagerFilter,LayerStateManagerSelect}`。
- **备注**：无状态显示 “No layer states in this drawing”，无匹配显示 “No matching layer states”。

### 状态详情区
- **功能简介**：显示选中状态的保存详情：图层数、当前图层、恢复的属性集合；未选中时显示 “New layer state” 说明。
- **UI 入口**：`图层状态管理器 → 右侧详情区`。
- **样式**：标题字号 13；字段标签字号 10（muted，宽 92），值字号 11；`mask_summary` 汇总 On/Off、Freeze、Lock、Plot、New VP Freeze、Color、Linetype、Lineweight、Plot style、Transparency。
- **触发命令**：无。
- **实现位置**：`src/ui/window/layer_state_manager.rs::{view_window,mask_summary}`。
- **备注**：`state.current_layer` 为空显示 “—”。

### New（新建状态）与名称/说明输入
- **功能简介**：启用新建模式，输入状态名与可选说明；名空报错，重名报错。
- **UI 入口**：`图层状态管理器 → 右侧工具栏 → New`；以及右侧 “Name”“Description” 输入框。
- **样式**：`button`（`button::secondary`，内边距 `[5, 12]`）；`text_input`（字号 11，内边距 `[5, 8]`），名称框支持 Enter 提交。
- **触发命令**：无（`Message::LayerStateManagerNew` / `LayerStateManagerName` / `LayerStateManagerDescription`）。
- **实现位置**：`src/ui/window/layer_state_manager.rs::view_window`；`src/app/update/mod.rs` 对应分支；`src/app/layers.rs::load_layer_state_editor`（自动生成 “Layer State {n}”）。
- **备注**：默认名取第一个不冲突的 “Layer State n”。

### Edit（编辑状态）
- **功能简介**：打开编辑器，逐图层编辑该状态保存的值。
- **UI 入口**：`图层状态管理器 → 右侧工具栏 → Edit`。
- **样式**：`button::secondary`，内边距 `[5, 12]`。
- **触发命令**：无（`Message::LayerStateManagerEdit`）。
- **实现位置**：`src/app/update/mod.rs::LayerStateManagerEdit`（切到 `ModalKind::LayerStateEditor`，草稿 `layer_state_edit_draft`）；编辑器 `src/ui/window/layer_state_manager.rs::view_editor`。
- **备注**：未选中时按钮无响应。

### Restore（恢复状态）
- **功能简介**：把选中状态保存的图层设置恢复到图纸（推 undo `LAYERSTATE RESTORE`）。
- **UI 入口**：`图层状态管理器 → 右侧工具栏 → Restore`。
- **样式**：`button::primary`（accent），内边距 `[5, 12]`。
- **触发命令**：无（`Message::LayerStateManagerRestore`）。
- **实现位置**：`src/app/update/mod.rs::LayerStateManagerRestore`。
- **备注**：恢复后刷新图层面板并输出 “restored ”name” (n layer(s))”。

### Delete（删除状态）
- **功能简介**：删除选中状态（推 undo `LAYERSTATE DELETE`）。
- **UI 入口**：`图层状态管理器 → 右侧工具栏 → Delete`。
- **样式**：`button::danger`，内边距 `[5, 12]`。
- **触发命令**：无（`Message::LayerStateManagerDelete`）。
- **实现位置**：`src/app/update/mod.rs::LayerStateManagerDelete`。
- **备注**：删除后选择列表首个状态。

### Save（保存 / 从图纸更新）
- **功能简介**：保存新状态或从图更新已有状态（捕获每图层当前设置）；按钮文本在 “Save New State” 与 “Update from Drawing” 间切换。
- **UI 入口**：`图层状态管理器 → 右侧底部 → Save New State / Update from Drawing`。
- **样式**：`button::primary`，字号 11，内边距 `[6, 14]`；说明文本 “Layer states are stored inside the drawing…”。
- **触发命令**：无（`Message::LayerStateManagerSave`）。
- **实现位置**：`src/app/update/mod.rs::LayerStateManagerSave`（可改名 `rename_layer_state` + `capture_layer_state`，推 undo `LAYERSTATE SAVE`）。
- **备注**：提交前校验空名与（除自身外的）重名。

### 图层状态编辑器：掩码属性开关
- **功能简介**：切换该状态恢复哪些属性（On/Off、Freeze、Lock、Plot、New VP、Color、Linetype、Lineweight、Plot style、Transparency）。
- **UI 入口**：`图层状态管理器 → Edit → “Properties restored by this state” → 属性按钮`。
- **样式**：`mask_button`（选中 `button::primary` 并显示 “✓”，否则 `button::subtle`），字号 10，内边距 `[4, 7]`。
- **触发命令**：无（`Message::LayerStateEditorMaskToggle(property)`）。
- **实现位置**：`src/ui/window/layer_state_manager.rs::{view_editor,mask_button,mask_for}`；`src/app/update/mod.rs::LayerStateEditorMaskToggle`。
- **备注**：对应 `LayerStateMask` 各位。

### 图层状态编辑器：名称/说明/当前图层
- **功能简介**：编辑草稿状态的名称、说明，并用下拉选择该状态的当前图层。
- **UI 入口**：`图层状态管理器 → Edit → 顶部 Name/Description 行 + Current layer 下拉`。
- **样式**：`text_input`（名称宽 220，说明可变宽）；`pick_list`（当前图层宽 180，字号 11，内边距 `[3, 6]`）。
- **触发命令**：无（`Message::LayerStateEditorName` / `LayerStateEditorDescription` / `LayerStateEditorCurrentLayer`）。
- **实现位置**：`src/ui/window/layer_state_manager.rs::view_editor`；`src/app/update/mod.rs` 对应分支。
- **备注**：标题显示 “%{n} saved layers”。

### 图层状态编辑器：逐图层表格
- **功能简介**：逐图层编辑该状态的保存值：Layer 名、On/Freeze/Lock/Plot/New VP 复选、Color、Linetype、Lineweight、Plot style、Transparency。
- **UI 入口**：`图层状态管理器 → Edit → “Saved layer values” 表格`。
- **样式**：`editor_header`（标签 10px，各列固定宽：Layer 170、On 44、Freeze 54、Lock 44、Plot 44、New VP 54、Color 135、Linetype 150、Lineweight 115、Plot style 135、Transparency 105）；布尔为 `checkbox(size 14)`；颜色为 `color_selector`；线型/线宽/透明度为 `pick_list`；Plot style 为 `text_input`；奇偶行交替底色。
- **触发命令**：无（`Message::LayerStateEditorLayerFlagToggle` / `LayerStateEditorLayerColor` / `LayerStateEditorLayerLinetype` / `LayerStateEditorLayerLineweight` / `LayerStateEditorLayerPlotStyle` / `LayerStateEditorLayerTransparency`）。
- **实现位置**：`src/ui/window/layer_state_manager.rs::{view_editor,editor_header,editor_layer_row,bool_cell,transparency_options}`；`src/app/update/mod.rs` 对应分支。
- **备注**：透明度选项为 “Not set” + 0%..90%（步进 10），若当前值不在其中则插入。

### 图层状态编辑器：图层搜索
- **功能简介**：按图层名过滤表格行。
- **UI 入口**：`图层状态管理器 → Edit → “Saved layer values” 右侧 → Search layers…`。
- **样式**：`text_input`，字号 11，内边距 `[4, 7]`，宽 190。
- **触发命令**：无（`Message::LayerStateEditorFilter`）。
- **实现位置**：`src/ui/window/layer_state_manager.rs::view_editor`。
- **备注**：过滤大小写不敏感。

### 图层状态编辑器：Cancel / Save Changes
- **功能简介**：取消编辑（不改动状态）或保存更改；提示 “Changes affect the saved state only; the drawing is unchanged until Restore.”。
- **UI 入口**：`图层状态管理器 → Edit → 底部 Cancel / Save Changes`。
- **样式**：`button::secondary`（Cancel）/ `button::primary`（Save Changes），内边距 `[5, 12]`。
- **触发命令**：无（`Message::LayerStateEditorCancel` / `LayerStateEditorSave`）。
- **实现位置**：`src/app/update/mod.rs::{LayerStateEditorCancel,LayerStateEditorSave}`。
- **备注**：编辑只改保存状态，需 Restore 才作用于图纸。

---

## 六、图层翻译器窗口（Layer Translator）

浮动窗口（`ModalKind::LayerTranslator`，760×460，`src/app/view/modal.rs`），视图 `src/ui/window/layer_translator.rs::view_window`。由 `LAYTRANS` 命令打开/加载目标文件。

### 图层翻译器开关与加载目标
- **功能简介**：把本图图层映射到从另一图纸加载的图层集，然后一次性翻译（单个 undo 步）。
- **UI 入口**：`命令输入框 → LAYTRANS`；或 `LAYTRANS <path>` 直接加载目标文件。
- **样式**：浮动窗口，双列表布局。
- **触发命令**：`LAYTRANS`（可带路径参数）。
- **实现位置**：`src/app/commands/layers.rs`（`"LAYTRANS"` 与 `LAYTRANS <path>`）；`src/app/update/mod.rs::{LayerTranslatorLoad,LayerTranslatorLoaded}`；`src/app/commands/layers.rs::open`。
- **备注**：`load_targets` 读取目标文件图层；加载后移除已失效映射。

### 翻译列表：Translate from / Translate to
- **功能简介**：左列列出本图图层（排除已映射者），右列列出目标标准图层；各选一个以建立映射。
- **UI 入口**：`图层翻译器 → “Translate from” 面板 / “Translate to” 面板`。
- **样式**：`pane`（标题字号 11 muted，内容 `scrollable`，1px `background.strong` 边框、圆角 4）；行 `list_row`（选中 `primary.weak` 底 + `primary.weak` 文字，圆角 3）；空列表显示 “(none)”。
- **触发命令**：无（`Message::LayerTranslatorSelectFrom` / `LayerTranslatorSelectTo`）。
- **实现位置**：`src/ui/window/layer_translator.rs::{view_window,list_row,pane}`；`src/app/update/mod.rs` 对应分支。
- **备注**：已映射的源图层不再出现在 from 列。

### 控件：Load… / Map / Map same
- **功能简介**：Load… 选择目标标准文件；Map 建立所选 from→to 映射；Map same 按名称相同自动配对。
- **UI 入口**：`图层翻译器 → 顶部控件行`。
- **样式**：`button`，字号 12，内边距 `[4, 10]`；Map 在未双选时禁用，Map same 在无目标时禁用；右端显示已加载文件名或 “No standard loaded.”（字号 11 muted）。
- **触发命令**：无（`Message::LayerTranslatorLoad` / `LayerTranslatorMap` / `LayerTranslatorMapSame`）。
- **实现位置**：`src/ui/window/layer_translator.rs::view_window`；`src/app/update/mod.rs` 对应分支；`src/modules/draw/layers/laytrans::{load_targets,map_same}`。
- **备注**：Map same 保留既有映射不重复。

### 映射列表（%{n} mapping(s) + Remove）
- **功能简介**：列出已建立的 “from → to” 映射，可逐条移除。
- **UI 入口**：`图层翻译器 → 中部 “%{n} mapping(s)” 面板`。
- **样式**：每行 `text("from  →  to")` + “Remove” `button::text`（字号 11）。
- **触发命令**：无（`Message::LayerTranslatorUnmap(from)`）。
- **实现位置**：`src/ui/window/layer_translator.rs::view_window`；`src/app/update/mod.rs::LayerTranslatorUnmap`。
- **备注**：映射仅在 Translate 时应用，可反复调整。

### 选项：Force objects to ByLayer / Write translation log
- **功能简介**：强制对象颜色/线型等转为 ByLayer；写出翻译日志文件。
- **UI 入口**：`图层翻译器 → 底部选项行`。
- **样式**：`checkbox(size 14)` + 文本字号 11。
- **触发命令**：无（`Message::LayerTranslatorForceByLayer` / `LayerTranslatorWriteLog`）。
- **实现位置**：`src/ui/window/layer_translator.rs::view_window`；`src/app/update/mod.rs` 对应分支。
- **备注**：`force_bylayer` 传入 `laytrans::Options`。

### 动作：Save mappings… / Load mappings… / Cancel / Translate
- **功能简介**：保存/加载映射文件；取消关闭；Translate 应用映射（推 undo `LAYTRANS`，可选写日志）。
- **UI 入口**：`图层翻译器 → 底部动作行`。
- **样式**：`button`（字号 12，内边距 `[4, 10]`）；Translate 为 `button::primary`，无映射时禁用；Cancel 为 `dialog_button`。
- **触发命令**：无（`Message::LayerTranslatorSaveMappings` / `LoadMappings` / `MappingsPath` / `LayerTranslatorTranslate` / `CloseModal`）。
- **实现位置**：`src/ui/window/layer_translator.rs::view_window`；`src/app/update/mod.rs` 对应分支；`src/app/commands/layers.rs::{layer_translator_mappings_file,finish_layer_translation,write_layer_translation_log}`。
- **备注**：Translate 成功后关闭模态并汇报 “translated {layers} layer(s), {objects} object(s)”。

---

## 七、Tool Palettes / Sheet Set Manager / 侧边面板开关

### Tool Palettes（工具选项板，命令占位）
- **功能简介**：Ribbon View 选项卡提供 “Tool Palettes” 大按钮；当前命令为占位，仅输出未实现提示。
- **UI 入口**：`Ribbon → View 选项卡 → Palettes 面板 → Tool Palettes（LargeTool）`。
- **样式**：`LargeTool`（图标 `tool_palettes.svg`，标签 “Tool\nPalettes”）。
- **触发命令**：`TOOLPALETTES`。
- **实现位置**：`src/modules/view/tool_palettes.rs::tool()`；`src/app/commands/display.rs`（`"TOOLPALETTES"` → 提示 “Tool Palettes not yet implemented.”）。
- **备注**：无实际面板（据源码为占位）。

### Sheet Set Manager（图纸集管理器，命令占位）
- **功能简介**：Ribbon View 选项卡提供 “Sheet Set Manager” 大按钮；当前命令为占位，仅输出未实现提示。
- **UI 入口**：`Ribbon → View 选项卡 → Palettes 面板 → Sheet Set Manager（LargeTool）`。
- **样式**：`LargeTool`（图标 `sheetset.svg`，标签 “Sheet Set\nManager”）。
- **触发命令**：`SHEETSET`。
- **实现位置**：`src/modules/view/sheetset.rs::tool()`；`src/app/commands/display.rs`（`"SHEETSET"` → 提示 “Sheet Set Manager not yet implemented.”）。
- **备注**：无实际面板（据源码为占位）。

### Properties palette（View 选项卡入口）
- **功能简介**：Ribbon View 选项卡的 Properties 大按钮，切换属性面板显隐（与 `PROPERTIES` 命令同源，高亮随面板状态）。
- **UI 入口**：`Ribbon → View 选项卡 → Palettes 面板 → Properties（LargeTool）`。
- **样式**：`LargeTool`（图标 `properties.svg`，标签 “Properties”），高亮见“属性面板入口”。
- **触发命令**：点击发 `ModuleEvent::Command("PROPERTIES")`；命令行 `PROPERTIES`。
- **实现位置**：`src/modules/view/properties_palette.rs::tool()`；`src/modules/view/mod.rs::ViewModule::ribbon_groups`；`src/ui/ribbon/mod.rs::set_properties`。
- **备注**：与左侧停靠属性面板为同一面板。

### 侧边面板可见性判定与过滤器
- **功能简介**：Dock 渲染前按各面板的显示开关过滤边缘栈；被关闭的面板不渲染也不参与列宽/预览计算。
- **UI 入口**：无（内部逻辑）。
- **样式**：无。
- **触发命令**：无。
- **实现位置**：`src/app/view/mod.rs::visible_panel`（对 `show_properties`、`show_block_palette`、`show_external_references`、`show_browser`、`show_node_graph`、`pc_manager.show` 逐个判定）；`src/app/update/dialog.rs::dock_panel_visible`。
- **备注**：`PropPointerPressed` 只在属性面板可见时同步焦点。

### 停靠面板可见性持久化字段
- **功能简介**：各停靠面板的显示状态字段（`show_properties` 等）随配置保存。
- **UI 入口**：无。
- **样式**：无。
- **触发命令**：无。
- **实现位置**：`src/app/mod.rs`（`show_properties` 等字段；注释“Whether the Properties panel is shown on the left (PROPERTIES)”）；`src/app/config.rs`（`DockSide`）。
- **备注**：`ToggleProperties` 同时更新 `ribbon.set_properties`。

---

## 附：相关但非停靠的窗口（避免混淆）

### 布局管理器窗口（Layout Manager）
- **功能简介**：管理模型/图纸布局（新建、删除、左右移动、置为当前、重命名），**不是**停靠系统的一部分。
- **UI 入口**：`命令输入框 → 布局相关命令`（窗口 `src/ui/window/layout_manager.rs::view_window`）。
- **样式**：全窗口模态，工具栏 + 左列表 + 右详情。
- **触发命令**：`LAYOUT` 系列（见其它文档）。
- **实现位置**：`src/ui/window/layout_manager.rs`。
- **备注**：其工具栏含 New Layout / Delete / Move Left / Move Right / Set Current / Rename，与 Dock 无关；此处列出以澄清停靠系统不含布局管理。

---

（文档结束）
