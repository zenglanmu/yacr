# Ribbon「Draw」选项卡功能需求清单

本文档反推自 `src/modules/draw/`（含 `draw`、`modify`、`layers`、`clipboard`、`groups`、`inquiry`、`properties`、`select.rs`、`fence.rs`、`units.rs`、`defaults.rs`）与 `src/ui/ribbon/`（`draw_panel.rs`、`modify_panel.rs`、`widgets.rs`、`mod.rs`、`color_dropdown.rs`、`context_tools.rs`）以及 `src/app/commands/`（`draw.rs`、`layers.rs`、`inquiry.rs`、`layerprops.rs`）。Ribbon 面板与工具的权威布局入口是 `src/modules/draw/mod.rs::DrawModule::ribbon_groups()`。

面板组的权威顺序（`src/modules/draw/mod.rs`）：Draw、Modify、Annotation、Layers、Block、Properties、Groups、Clipboard、Measure。每个面板标题（`group_title`）可点击展开该组的扩展飞出面板（`src/ui/ribbon/draw_panel.rs::group_title`、`overlay`）。

---

## 一、Draw 面板（Ribbon → Draw 选项卡 → Draw 面板）

面板布局：`LargeTool(Line)`、`LargeTool(Polyline)`、`LargeDropdown(Circle)`、`LargeDropdown(Arc)`，以及三个小 `Dropdown`（Shapes、Ellipse、Hatch）。`src/modules/draw/mod.rs::ribbon_groups`。

### 直线（Line）
- **功能简介**：交互式连续绘制直线段；每次点击提交一条 `Line`，上一段终点自动成为下一段起点。
- **UI 入口**：`Ribbon → Draw 选项卡 → Draw 面板 → 直线（LargeTool）`。
- **样式**：`LargeTool`（大按钮，图标 + “Line”标签）。
- **触发命令**：`LINE`（别名 `L`，由 `CommandRegistration { names: ["LINE"] }` 注册）。
- **实现位置**：`src/modules/draw/draw/line.rs::tool()`、`LineCommand`；分发 `src/app/commands/draw.rs` 中 `"LINE"` 分支。
- **备注**：首次点击存起点；Enter / Escape 结束（据源码 `src/modules/draw/draw/line.rs` 头注）。

### 多段线（Polyline）
- **功能简介**：绘制轻量多段线，支持直线段与圆弧段混合、闭合、宽度与半宽。
- **UI 入口**：`Ribbon → Draw 选项卡 → Draw 面板 → 多段线（LargeTool）`。
- **样式**：`LargeTool`（图标 + “Polyline”）。
- **触发命令**：`PLINE`（`PL`）。
- **实现位置**：`src/modules/draw/draw/polyline.rs::tool()`、`PlineCommand`；交互子提示 `polyline.rs::options()`（`SegMode`/`Sub`）。
- **备注**：直线模式关键字 Arc / Close / Halfwidth / Length / Undo / Width；圆弧模式 Angle / CEnter / CLose / Direction / Halfwidth / Line / Radius / Second pt / Undo / Width（源码 `polyline.rs::options`）。Enter 完成，Escape 原样结束，`C`/`CL` 闭合。

### 圆下拉（Circle）
- **功能简介**：以多种几何条件创建圆。
- **UI 入口**：`Ribbon → Draw 选项卡 → Draw 面板 → 圆（LargeDropdown，默认 CIRCLE）`。
- **样式**：`LargeDropdown`（大按钮带 ▾；默认图标为“Center, Radius”，标签“Circle”）。
- **触发命令**：默认 `CIRCLE`。下拉子项（`circle::DROPDOWN_ITEMS`，全部）：
  - `CIRCLE` — Center, Radius（圆心 + 半径）
  - `CIRCLE_CD` — Center, Diameter（圆心 + 直径）
  - `CIRCLE_2P` — 2-Point（两点）
  - `CIRCLE_3P` — 3-Point（三点）
  - `CIRCLE_TTR` — Tan, Tan, Radius（切点、切点、半径）
  - `CIRCLE_TTT` — Tan, Tan, Tan（三切）
- **实现位置**：`src/modules/draw/draw/circle.rs::DROPDOWN_ID/ICON/DROPDOWN_ITEMS`、各 `Circle*Command`；分发 `src/app/commands/draw.rs` 的 `"CIRCLE"`、`"CIRCLE_CD"`、`"CIRCLE_2P"`、`"CIRCLE_3P"`、`"CIRCLE_TTR"`、`"CIRCLE_TTT"` 分支。
- **备注**：命令行内还提供关键字 `3P` / `2P` / `Ttr` / `Ttt` 与 `Diameter`（`circle.rs::options`）在圆心步切换构造方式。

### 圆弧下拉（Arc）
- **功能简介**：以多种起止/圆心/角度/方向条件创建圆弧，并支持从已有对象续画。
- **UI 入口**：`Ribbon → Draw 选项卡 → Draw 面板 → 圆弧（LargeDropdown，默认 ARC_3P）`。
- **样式**：`LargeDropdown`（默认图标 3-Point，标签“Arc”）。
- **触发命令**：默认 `ARC_3P`。下拉子项（`arc::DROPDOWN_ITEMS`，全部）：
  - `ARC_3P` — 3-Point（三点）
  - `ARC_SCE` — Start, Center, End
  - `ARC_SCA` — Start, Center, Angle
  - `ARC_SCL` — Start, Center, Length
  - `ARC_SEA` — Start, End, Angle
  - `ARC_SED` — Start, End, Direction
  - `ARC_SER` — Start, End, Radius
  - `ARC_CSE` — Center, Start, End
  - `ARC_CSA` — Center, Start, Angle
  - `ARC_CSL` — Center, Start, Length
  - `ARC_CONT` — Continue（延续上一段弧）
- **实现位置**：`src/modules/draw/draw/arc.rs::DROPDOWN_ID/ICON/DROPDOWN_ITEMS`、各 `Arc*Command`；分发 `src/app/commands/draw.rs` 的 `"ARC"`、`"ARC_3P"`、`"ARC_CSE"`、`"ARC_SCE"`、`"ARC_SCA"`、`"ARC_SCL"`、`"ARC_SEA"`、`"ARC_SER"`、`"ARC_SED"`、`"ARC_CSA"`、`"ARC_CSL"`、`"ARC_CONT"` 分支。
- **备注**：命令行 `ARC` 在圆心步提供 `SCE`/`SCA`/`SEA`/`SER`/`CSA`/`3P` 关键字切换（`arc.rs::options`）。

### 形状下拉（Shapes）
- **功能简介**：创建矩形与正多边形。
- **UI 入口**：`Ribbon → Draw 选项卡 → Draw 面板 → 形状（Dropdown，默认 RECT）`。
- **样式**：`Dropdown`（1 行小图标 + ▾）。
- **触发命令**：默认 `RECT`。下拉子项（`shapes::DROPDOWN_ITEMS`，全部）：
  - `RECT` — Rectangle - Two Corners（对角两点）
  - `RECT_ROT` — Rectangle - Rotated（旋转矩形）
  - `RECT_CEN` — Rectangle - Center（中心 + 角点）
  - `POLY` — Polygon - Inscribed（内接正多边形）
  - `POLY_C` — Polygon - Circumscribed（外切正多边形）
  - `POLY_E` — Polygon - Edge（按边定义）
- **实现位置**：`src/modules/draw/draw/shapes.rs::DROPDOWN_ID/ICON/DROPDOWN_ITEMS`、各 `Rect*Command`/`Poly*Command`；分发 `src/app/commands/draw.rs` 的 `"RECT" | "RECTANG"`、`"RECT_ROT"`、`"RECT_CEN"`、`"POLY" | "POLYGON"`、`"POLY_C"`、`"POLY_E"` 分支。
- **备注**：正多边形边数在命令行输入（`defaults::get_polygon_sides`，默认 6）；矩形支持高度/宽度/旋转/倒角/圆角等会话默认值（`defaults.rs`）。

### 椭圆下拉（Ellipse）
- **功能简介**：创建椭圆与椭圆弧。
- **UI 入口**：`Ribbon → Draw 选项卡 → Draw 面板 → 椭圆（Dropdown，默认 ELLIPSE）`。
- **样式**：`Dropdown`（默认图标“Center, Axes”）。
- **触发命令**：默认 `ELLIPSE`。下拉子项（`ellipse::DROPDOWN_ITEMS`，全部）：
  - `ELLIPSE` — Center, Axes（中心 + 轴）
  - `ELLIPSE_AXIS` — Axis, End（轴端点 + 末端）
  - `ELLIPSE_ARC` — Ellipse Arc（椭圆弧）
- **实现位置**：`src/modules/draw/draw/ellipse.rs::DROPDOWN_ID/ICON/DROPDOWN_ITEMS`、各 `Ellipse*Command`；分发 `src/app/commands/draw.rs` 的 `"ELLIPSE"`、`"ELLIPSE_AXIS"`、`"ELLIPSE_ARC"` 分支。

### 填充下拉（Hatch）
- **功能简介**：对封闭区域填充图案、渐变，或创建边界。
- **UI 入口**：`Ribbon → Draw 选项卡 → Draw 面板 → 填充（Dropdown，默认 HATCH）`。
- **样式**：`Dropdown`（默认图标 hatch_lines）。
- **触发命令**：默认 `HATCH`。下拉子项（`hatch::DROPDOWN_ITEMS`，全部）：
  - `HATCH` — Hatch（图案填充）
  - `GRADIENT` — Gradient（渐变填充）
  - `BOUNDARY` — Boundary（创建边界）
- **实现位置**：`src/modules/draw/draw/hatch.rs::DROPDOWN_ID/ICON/DROPDOWN_ITEMS`、`HatchCommand`/`GradientCommand`/`BoundaryCommand`；分发 `src/app/commands/draw.rs` 的 `"HATCH"`、`"GRADIENT"`、`"BOUNDARY"` 分支。
- **备注**：HATCH 交互选项包括 Select objects(`O`)、Draw manually(`S`)、Keep boundaries、Associative、Separate hatches、Island style、Accept（`hatch.rs::options`）。BOUNDARY 高级模式含 Object type → Region(`R`)/Polyline(`P`)（`hatch.rs::options`）。

---

## 二、Modify 面板（Ribbon → Draw 选项卡 → Modify 面板）

面板布局（`src/modules/draw/mod.rs`）：`translate`、`copy`、`stretch`、`rotate`、`mirror`、`scale`（均为小 `Tool`），三个 `Dropdown`（trim、fillet、array），再 `delete`、`explode`、`offset`（小 `Tool`）。小按钮每列 3 个排列（`src/ui/ribbon/mod.rs::render_group` 的 `flush_small_col`）。

### 移动（Move）
- **功能简介**：将当前选择集沿指定位移移动（等同平移）。
- **UI 入口**：`Ribbon → Draw 选项卡 → Modify 面板 → 移动（Tool）`。
- **样式**：`Tool`（1 行小图标）。
- **触发命令**：`MOVE`（别名 `M`、`3DMOVE`）。
- **实现位置**：`src/modules/draw/modify/translate.rs::tool()`、`TranslateCommand`；分发 `src/app/commands/draw.rs` 的 `"MOVE" | "3DMOVE"` 分支。
- **备注**：需先选中对象或由 `SelectObjectsCommand` 收集；可用 `D` 直接输入位移向量（`translate.rs` 头注）。

### 复制（Copy）
- **功能简介**：基点复制选择集，可连续多次放置。
- **UI 入口**：`Ribbon → Draw 选项卡 → Modify 面板 → 复制（Tool）`。
- **样式**：`Tool`。
- **触发命令**：`COPY`（`CO`）。
- **实现位置**：`src/modules/draw/modify/copy.rs::tool()`、`CopyCommand`；分发 `src/app/commands/draw.rs` 的 `"COPY"` 分支。

### 拉伸（Stretch）
- **功能简介**：以交叉窗口方式仅移动窗口内的顶点/对象，实现拉伸。
- **UI 入口**：`Ribbon → Draw 选项卡 → Modify 面板 → 拉伸（Tool）`。
- **样式**：`Tool`。
- **触发命令**：`STRETCH`（`SS`）。
- **实现位置**：`src/modules/draw/modify/stretch.rs::tool()`、`StretchCommand`；分发 `src/app/commands/draw.rs`（`select.rs` 注册）中 `STRETCH`。
- **备注**：直线按端点是否在窗口内分别移动；圆弧/圆按圆心；Insert 按插入点（`stretch.rs` 头注）。

### 旋转（Rotate）
- **功能简介**：绕基点旋转选择集，支持参考角方式。
- **UI 入口**：`Ribbon → Draw 选项卡 → Modify 面板 → 旋转（Tool）`。
- **样式**：`Tool`。
- **触发命令**：`ROTATE`（`RO`）。
- **实现位置**：`src/modules/draw/modify/rotate.rs::tool()`、`RotateCommand`；分发 `src/app/commands/draw.rs` 的 `"ROTATE"` 分支。
- **备注**：默认角度取会话记忆 `defaults::get_rotate_angle`；`Reference` 关键字可用两点测量参考角。

### 镜像（Mirror）
- **功能简介**：关于两点定义的镜像线镜像选择集，可选择是否删除源对象。
- **UI 入口**：`Ribbon → Draw 选项卡 → Modify 面板 → 镜像（Tool）`。
- **样式**：`Tool`。
- **触发命令**：`MIRROR`（`MI`）。
- **实现位置**：`src/modules/draw/modify/mirror.rs::tool()`、`MirrorCommand`；分发 `src/app/commands/draw.rs` 的 `"MIRROR"` 分支。
- **备注**：第三步提示 “Erase source objects? [Yes/No] <No>”（`mirror.rs` 头注）。

### 缩放（Scale）
- **功能简介**：绕基点按比例缩放选择集，支持参考长度方式。
- **UI 入口**：`Ribbon → Draw 选项卡 → Modify 面板 → 缩放（Tool）`。
- **样式**：`Tool`。
- **触发命令**：`SCALE`（`SC`）。
- **实现位置**：`src/modules/draw/modify/scale.rs::tool()`、`ScaleCommand`；分发 `src/app/commands/draw.rs` 的 `"SCALE"` 分支。
- **备注**：拖动时因子为光标到基点距离；`R` 进入参考缩放。

### 修剪下拉（Trim / …）
- **功能简介**：按交点裁剪或延伸对象。
- **UI 入口**：`Ribbon → Draw 选项卡 → Modify 面板 → 修剪（Dropdown，默认 TRIM）`。
- **样式**：`Dropdown`（默认图标 trim.svg）。
- **触发命令**：默认 `TRIM`。下拉子项（`trim::DROPDOWN_ITEMS`，全部）：
  - `TRIM` — Trim
  - `EXTEND` — Extend
- **实现位置**：`src/modules/draw/modify/trim.rs::DROPDOWN_ID/ICON/DROPDOWN_ITEMS`、`TrimCommand`/`ExtendCommand`；分发 `src/app/commands/draw.rs` 中 `TRIM`/`EXTEND`（注册于 `trim.rs`）。
- **备注**：支持 fence 手势（`src/modules/draw/fence.rs`）；另有 `EXTRIM`（边界外全部裁剪）经命令行注册（`trim.rs:5148`）。

### 圆角下拉（Fillet / …）
- **功能简介**：为两条曲线创建圆角或倒角。
- **UI 入口**：`Ribbon → Draw 选项卡 → Modify 面板 → 圆角（Dropdown，默认 FILLET）`。
- **样式**：`Dropdown`（默认图标 fillet.svg）。
- **触发命令**：默认 `FILLET`。下拉子项（`fillet::DROPDOWN_ITEMS`，全部）：
  - `FILLET` — Fillet（圆角）
  - `CHAMFER` — Chamfer（倒角）
- **实现位置**：`src/modules/draw/modify/fillet.rs::DROPDOWN_ID/ICON/DROPDOWN_ITEMS`、`FilletCommand`/`ChamferCommand`；注册 `fillet.rs:2408-2409`。
- **备注**：FILLET 支持线-线、线-弧、弧-弧；半径 `R`；CHAMFER 仅线-线，距离 `dist1`/`dist2`，默认 10（`defaults.rs`）。

### 阵列下拉（Array）
- **功能简介**：按矩形/路径/极坐标方式复制对象阵列。
- **UI 入口**：`Ribbon → Draw 选项卡 → Modify 面板 → 阵列（Dropdown，默认 ARRAYRECT）`。
- **样式**：`Dropdown`（默认图标 array_rect.svg）。
- **触发命令**：默认 `ARRAYRECT`。下拉子项（`array::DROPDOWN_ITEMS`，全部）：
  - `ARRAYRECT` — Rectangular Array（矩形阵列）
  - `ARRAYPATH` — Path Array（路径阵列，据源码为占位）
  - `ARRAYPOLAR` — Polar Array（极坐标阵列）
- **实现位置**：`src/modules/draw/modify/array.rs::DROPDOWN_ID/ICON/DROPDOWN_ITEMS`、各 array 命令；分发 `src/app/commands/draw.rs`。
- **备注**：默认行/列 2、间距 100（`defaults.rs`）；极坐标默认 6 项、360°。另有 `ARRAY3D`/`3DARRAY`（`array.rs:1036`）。阵列项数上限见 `array.rs` 常量。

### 删除（Delete）
- **功能简介**：删除选中对象（等同 ERASE）。
- **UI 入口**：`Ribbon → Draw 选项卡 → Modify 面板 → 删除（Tool）`。
- **样式**：`Tool`（图标 erase.svg，标签 “Delete”）。
- **触发命令**：`ERASE`（`E`、Delete 键）。
- **实现位置**：`src/modules/draw/modify/delete.rs::tool()`；分发 `src/app/commands/draw.rs` 的 `"ERASE"`；上下文菜单 `DeleteSelected`（`src/ui/popup/context_menu.rs::idle_rows`）。

### 分解（Explode）
- **功能简介**：将复合对象（多段线、块引用、MLine、标注等）打散为基本实体。
- **UI 入口**：`Ribbon → Draw 选项卡 → Modify 面板 → 分解（Tool）`。
- **样式**：`Tool`。
- **触发命令**：`EXPLODE`（`X`）。
- **实现位置**：`src/modules/draw/modify/explode.rs::tool()`、`explode_entity`；分发 `src/app/commands/draw.rs`。
- **备注**：支持 LwPolyline/Polyline2D/Polyline3D/Polyline/Insert/MLine/Dimension；不支持类型静默跳过（`explode.rs` 头注）。

### 偏移（Offset）
- **功能简介**：按指定距离在选定侧创建平行副本（线、弧、圆、多段线等）。
- **UI 入口**：`Ribbon → Draw 选项卡 → Modify 面板 → 偏移（Tool）`。
- **样式**：`Tool`。
- **触发命令**：`OFFSET`（`O`）。
- **实现位置**：`src/modules/draw/modify/offset.rs::tool()`、`OffsetCommand`；分发 `src/app/commands/draw.rs`。
- **备注**：默认距离 1.0（`defaults.rs`）；支持 `Multiple` 连续偏移。上下文菜单 Enter 行会显示 `<距离>` 提示（`context_menu.rs`）。

### Modify 面板扩展飞出（面板标题下拉）
- **功能简介**：点击 Modify 面板标题展开更多编辑工具。
- **UI 入口**：`Ribbon → Draw 选项卡 → Modify 面板 → 标题 “Modify ▾”（点击展开）`。
- **样式**：飞出面板为多列 36×36 网格（`draw_panel.rs::overlay`，`CELL = 36*0.7`、`GAP = 3*0.7`），面板边框颜色为主题主色。
- **工具列表**（`src/ui/ribbon/modify_panel.rs::TOOLS`，全部；每项为 `Tool` 网格项）：
  - `SETBYLAYER` — Set to ByLayer（设置为随层）
  - `LENGTHEN` — Lengthen（拉长）
  - `PEDIT` — Edit Polyline（编辑多段线）
  - `SPLINEDIT` — Edit Spline（编辑样条）
  - `HATCHEDIT` — Edit Hatch（编辑填充）
  - `ALIGN` — Align（对齐，3D 点对放置）
  - `ALIGNLEFT` — Align Left（左对齐包围盒，一次性）
  - `ALIGNHCENTER` — Align Horizontal Centers
  - `ALIGNRIGHT` — Align Right
  - `ALIGNTOP` — Align Top
  - `ALIGNVCENTER` — Align Vertical Centers
  - `ALIGNBOTTOM` — Align Bottom
  - `BREAK` — Break（打断）
  - `BREAKATPOINT` — Break at Point（点打断）
  - `JOIN` — Join（合并）
  - `REVERSE` — Reverse（反向）
  - `NCOPY` — Copy Nested Objects（复制嵌套对象）
  - `DRAWORDER_FRONT` — Draw Order（下拉，子项：Bring to Front / Send to Back / Bring Above Objects / Send Under Objects）
- **实现位置**：`src/ui/ribbon/modify_panel.rs::TOOLS`；飞出渲染 `src/ui/ribbon/draw_panel.rs::overlay/tool_button`；分发 `src/app/commands/inquiry.rs`、`src/app/commands/layers.rs`、`src/app/commands/draw.rs`。
- **备注**：Draw Order 的四个子项由 `modify_panel.rs` 中该 Tool 的 `options` 定义；扩展子菜单归属父面板由 `panel_for_dropdown`/`parent_panel` 解析（`draw_panel.rs`）。

---

## 三、Annotation 面板（Ribbon → Draw 选项卡 → Annotation 面板）

三个 `LargeDropdown`（`src/modules/draw/mod.rs`）。工具来自 `crate::modules::annotate`。

### 文字下拉（Text）
- **功能简介**：创建单行文字与多行文字。
- **UI 入口**：`Ribbon → Draw 选项卡 → Annotation 面板 → 文字（LargeDropdown，默认 TEXT）`。
- **样式**：`LargeDropdown`（图标 `text::ICON`，标签“Text”）。
- **触发命令**：默认 `TEXT`。下拉子项（全部）：
  - `TEXT` — Text（单行文字）
  - `MTEXT` — MText（多行文字）
- **实现位置**：`src/modules/draw/mod.rs`（内联 items）；`src/modules/annotate/text.rs::tool()`、`src/modules/annotate/mtext.rs::tool()`；分发 `src/app/commands/draw.rs` 的 `"TEXT"`、`"MTEXT"`。

### 标注下拉（Dimensions）
- **功能简介**：创建线性、半径、角度标注。
- **UI 入口**：`Ribbon → Draw 选项卡 → Annotation 面板 → 标注（LargeDropdown，默认 DIMLINEAR）`。
- **样式**：`LargeDropdown`（图标 `linear_dim::ICON`，标签“Dimensions”）。
- **触发命令**：默认 `DIMLINEAR`。下拉子项（全部）：
  - `DIMLINEAR` — Linear（线性标注）
  - `DIMRADIUS` — Radius（半径标注）
  - `DIMANGULAR` — Angular（角度标注）
- **实现位置**：`src/modules/draw/mod.rs`（内联 items）；`src/modules/annotate/linear_dim.rs`、`radius_dim.rs`、`angular_dim.rs::tool()`。

### 引线下拉（Leader）
- **功能简介**：创建引线与多重引线。
- **UI 入口**：`Ribbon → Draw 选项卡 → Annotation 面板 → 引线（LargeDropdown，默认 LEADER）`。
- **样式**：`LargeDropdown`（图标 `leader_cmd::ICON`，标签“Leader”）。
- **触发命令**：默认 `LEADER`。下拉子项（全部）：
  - `LEADER` — Leader（引线）
  - `MLEADER` — MLeader（多重引线）
- **实现位置**：`src/modules/draw/mod.rs`（内联 items）；`src/modules/annotate/leader_cmd.rs`、`mleader_cmd.rs::tool()`。

---

## 四、Layers 面板（Ribbon → Draw 选项卡 → Layers 面板）

### 图层特性管理器（Layer Properties，大按钮）
- **功能简介**：打开图层管理器窗口，管理图层的新建/删除/设为当前、名称/颜色/线型/线宽/透明度、开关/冻结/锁定/打印、按列排序、按名称过滤。
- **UI 入口**：`Ribbon → Draw 选项卡 → Layers 面板 → “Layers”（LargeTool）`，打开浮动窗口。
- **样式**：`LargeTool`（图标 layers/panel.svg，标签“Layers”）。
- **触发命令**：`LAYERS`（也见 `LAYER` 命令与 `src/app/commands/fileops.rs` 的 `"LAYERS"` 映射到 `Message::ToggleLayers`）。
- **实现位置**：`src/modules/draw/layers/panel.rs::tool()`（`ModuleEvent::ToggleLayers`）；窗口 `src/ui/window/layers.rs::LayerPanel`（`LayerSortCol`、`sortable_header`、`layer_row`、`toolbar_btn`）；命令 `src/app/commands/layerprops.rs::"LAYER"` 分支。
- **备注**：工具栏按钮 “New”“Delete”“Set Current” 与搜索框（`layers.rs::view_content`）；列头可点击排序（Name/On/Freeze/Lock/Plot/Color/Linetype/Lineweight/Transparency），Name 列可拖动分隔条调宽；每行可切换可见/冻结/锁定/打印、选择颜色、线型、线宽、透明度；在图纸布局中有每视口冻结列（`vp_cols`）。

### 图层组合下拉 + 两行小按钮（LayerComboGroup）
- **功能简介**：图层下拉用于设置当前图层，下方两行共 10 个图层小按钮提供常用图层操作。
- **UI 入口**：`Ribbon → Draw 选项卡 → Layers 面板 → 图层名称下拉 + 两行小图标网格`。
- **样式**：`LayerComboGroup`（图层下拉 + 两行小按钮）；下拉展示每层的 可见/冻结/锁定 切换图标、颜色块与名称，底部含 “Layer State Manager…” 与搜索框（`src/ui/ribbon/mod.rs::layer_combo_overlay`）。
- **触发命令/按钮**：
  - 第 2 行（`row2`）：`LAYOFF` Layer Off、`LAYFRZ` Layer Freeze、`LAYLCK` Layer Lock、`LAYMCUR` Make Current、`LAYISO` Isolate Layer。
  - 第 3 行（`row3`）：`LAYON` Turn All Layers On、`LAYTHW` Thaw All Layers、`LAYULK` Layer Unlock、`LAYMATCH` Match Layer、`LAYUNISO` Unisolate Layers。
- **实现位置**：`src/modules/draw/mod.rs`（`LayerComboGroup { row2, row3 }`）；各工具定义 `src/modules/draw/layers/{layoff,layfrz,laylck,make_current,layiso,layon,laythw,layulk,match_layer,layuniso}.rs::tool()`；下拉渲染 `src/ui/ribbon/widgets.rs::render_large`（`LayerComboGroup` 分支）；命令 `src/app/commands/layers.rs`。
- **备注**：`LAYMCUR` 使用 `SelectObjectsCommand::instant`（首次完成选择即生效）；`LAYMATCH`/`LAYISO` 等用标准收集（`select.rs`）。图层状态管理器命令在 `layers.rs` 的 `LAYERSTATE`/`LAS`/`LMAN`（含 LIST/SAVE/RESTORE/DELETE）。

### 图层工具各按钮细则
以下按钮与上文两行小按钮对应，逐项说明：

#### Layer Off（LAYOFF）
- **功能简介**：关闭选中对象所在图层。
- **UI 入口**：`Ribbon → Draw 选项卡 → Layers 面板 → 图层下拉 → 第 2 行第 1 个（Tool）`。
- **样式**：`Tool`（小图标，由 `LayerComboGroup` 的一行 3 列网格渲染）。
- **触发命令**：`LAYOFF`。
- **实现位置**：`src/modules/draw/layers/layoff.rs::tool()`；命令 `src/app/commands/layers.rs::"LAYOFF"`。

#### Layer Freeze（LAYFRZ）
- **功能简介**：冻结选中对象所在图层。
- **UI 入口**：`... Layers 面板 → 图层下拉 → 第 2 行第 2 个`。
- **触发命令**：`LAYFRZ`。
- **实现位置**：`src/modules/draw/layers/layfrz.rs::tool()`；`src/app/commands/layers.rs::"LAYFRZ"`。

#### Layer Lock（LAYLCK）
- **功能简介**：锁定选中对象所在图层。
- **UI 入口**：`... Layers 面板 → 图层下拉 → 第 2 行第 3 个`。
- **触发命令**：`LAYLCK`。
- **实现位置**：`src/modules/draw/layers/laylck.rs::tool()`；`src/app/commands/layers.rs::"LAYLCK"`。

#### Make Current（LAYMCUR）
- **功能简介**：将选中对象所在图层设为当前图层。
- **UI 入口**：`... Layers 面板 → 图层下拉 → 第 2 行第 4 个`。
- **触发命令**：`LAYMCUR`。
- **实现位置**：`src/modules/draw/layers/make_current.rs::tool()`；`src/app/commands/layers.rs::"LAYMCUR"`。

#### Isolate Layer（LAYISO）
- **功能简介**：隔离选中对象所在图层，隐藏其它图层。
- **UI 入口**：`... Layers 面板 → 图层下拉 → 第 2 行第 5 个`。
- **触发命令**：`LAYISO`。
- **实现位置**：`src/modules/draw/layers/layiso.rs::tool()`；`src/app/commands/layers.rs::"LAYISO"`。

#### Turn All Layers On（LAYON）
- **功能简介**：打开所有图层。
- **UI 入口**：`... Layers 面板 → 图层下拉 → 第 3 行第 1 个`。
- **触发命令**：`LAYON`。
- **实现位置**：`src/modules/draw/layers/layon.rs::tool()`；`src/app/commands/layers.rs::"LAYON"`。

#### Thaw All Layers（LAYTHW）
- **功能简介**：解冻所有图层。
- **UI 入口**：`... Layers 面板 → 图层下拉 → 第 3 行第 2 个`。
- **触发命令**：`LAYTHW`。
- **实现位置**：`src/modules/draw/layers/laythw.rs::tool()`；`src/app/commands/layers.rs::"LAYTHW"`。

#### Layer Unlock（LAYULK）
- **功能简介**：解锁选中对象所在图层。
- **UI 入口**：`... Layers 面板 → 图层下拉 → 第 3 行第 3 个`。
- **触发命令**：`LAYULK`。
- **实现位置**：`src/modules/draw/layers/layulk.rs::tool()`；`src/app/commands/layers.rs::"LAYULK"`。

#### Match Layer（LAYMATCH）
- **功能简介**：将目标对象的图层改为源对象的图层。
- **UI 入口**：`... Layers 面板 → 图层下拉 → 第 3 行第 4 个`。
- **触发命令**：`LAYMATCH`（`LAYMCH`）。
- **实现位置**：`src/modules/draw/layers/match_layer.rs::tool()`、`LayMatchCommand`（两阶段收集）；`src/app/commands/layers.rs::"LAYMATCH" | "LAYMCH"`。

#### Unisolate Layers（LAYUNISO）
- **功能简介**：取消图层隔离，恢复此前状态。
- **UI 入口**：`... Layers 面板 → 图层下拉 → 第 3 行第 5 个`。
- **触发命令**：`LAYUNISO`。
- **实现位置**：`src/modules/draw/layers/layuniso.rs::tool()`；`src/app/commands/layers.rs::"LAYUNISO"`。

### 图层下拉本身（LAYER_COMBO）
- **功能简介**：显示当前图层并切换当前图层；每行带可见/冻结/锁定快捷切换。
- **UI 入口**：`Ribbon → Draw 选项卡 → Layers 面板 → 图层名称显示条（点击展开）`。
- **样式**：组合按钮（可见/冻结/锁定图标 + 颜色块 + 图层名 + ▾），下拉面板宽 220，含搜索框与 “Layer State Manager…” 行（`src/ui/ribbon/mod.rs::layer_combo_overlay`）。
- **触发命令**：`Message::RibbonLayerChanged`（选择）；行内图标发 `LayerToggleVisible/Freeze/Lock`。
- **实现位置**：`src/ui/ribbon/mod.rs::layer_combo_overlay`、`src/ui/ribbon/widgets.rs::render_large`（`LayerComboGroup`）。
- **备注**：`Layer State Manager…` 打开图层状态管理器窗口（`src/ui/window/layer_state_manager.rs`）。

---

## 五、Block 面板（Ribbon → Draw 选项卡 → Block 面板）

### 创建块（Create Block）
- **功能简介**：将选定对象定义为块，或按对话框选项创建/重定义块。
- **UI 入口**：`Ribbon → Draw 选项卡 → Block 面板 → 创建块（LargeTool）`。
- **样式**：`LargeTool`（图标 blocks/block.svg，标签“Create Block”）。
- **触发命令**：`BLOCK`（`B`）。
- **实现位置**：`src/modules/insert/create_block.rs::tool()`、`CreateBlockCommand`/`BlockOnScreenCommand`；分发与对话框处理见 `src/app/commands/blocks.rs`。
- **备注**：块对象模式 Retain/Convert/Delete；支持重定义（`create_block_with_options` 校验名称、空选择、`*` 前缀）。

### 插入块（Insert Block）
- **功能简介**：按名称插入块引用，可指定插入点、缩放与旋转，并填写属性。
- **UI 入口**：`Ribbon → Draw 选项卡 → Block 面板 → 插入块（LargeTool）`。
- **样式**：`LargeTool`（图标 blocks/insert.svg，标签“Insert Block”）。
- **触发命令**：`INSERT`（`I`）。
- **实现位置**：`src/modules/insert/insert_block.rs::tool()`、`InsertBlockCommand`（含块名选择器 `BlockPicker`）。
- **备注**：块名步支持增量搜索；插入点步支持 `Scale`/`Rotate` 关键字；常量/预置属性自动填充（`advance_automatic_attributes`）。

---

## 六、Properties 面板（Ribbon → Draw 选项卡 → Properties 面板）

面板布局：`PropertiesGroup { match_prop }`（`src/modules/draw/mod.rs`）：左侧大按钮 “Match”，右侧三行下拉（对象颜色 / 线型 / 线宽）。

### 特性匹配（Match Properties）
- **功能简介**：将源对象的特性（颜色、线型、线宽、图层等）匹配到目标对象。
- **UI 入口**：`Ribbon → Draw 选项卡 → Properties 面板 → “Match”（LargeTool）`。
- **样式**：`LargeTool`（图标 match_prop.svg，标签“Match”）。
- **触发命令**：`MATCHPROP`（`MA`）。
- **实现位置**：`src/modules/draw/properties/match_prop.rs::tool()`、`MatchPropCommand`（阶段 1 源对象、阶段 2 目标集）；`src/app/commands/layers.rs::"MATCHPROP"`。

### 对象颜色下拉（Object Color）
- **功能简介**：设置当前对象颜色（ByLayer / ByBlock / 快速色板 / 索引色 / 最近色，或打开完整选色对话框）。
- **UI 入口**：`Ribbon → Draw 选项卡 → Properties 面板 → 颜色下拉（PropertiesGroup 第 1 行）`。
- **样式**：下拉行（色块 + 颜色名 + ▾）；弹出面板宽 `PANEL_W = 202`（`color_dropdown.rs`），5 段结构。
- **触发命令**：`Message::RibbonColorChanged`；`Select Color...` 发 `Message::OpenColorWindow`。
- **实现位置**：`src/ui/ribbon/color_dropdown.rs::color_dropdown_panel`；`src/ui/ribbon/mod.rs::prop_color_overlay`；渲染入口 `src/ui/ribbon/widgets.rs::render_large`（`PropertiesGroup`）；命令 `COLOR`/`BYLAYER` 见 `src/modules/draw/layers/color.rs::tool()`、`src/modules/draw/properties/bylayer.rs::tool()`。
- **备注**：面板全部子项：
  - ByLayer、ByBlock（逻辑色行，`logical_row`）
  - 45 格快速色板（5 行 × 9 列，`QUICK_PICK_GRID`）
  - Index Color 标题 + ACI 1~9 索引色块
  - Recent Colors 标题 + 最多 9 个最近色块（不足用占位块补齐）
  - “Select Color...” 行（打开选色窗口，目标 `ColorPickTarget::Ribbon`）

### 线型下拉（Linetype）
- **功能简介**：设置当前对象线型。
- **UI 入口**：`Ribbon → Draw 选项卡 → Properties 面板 → 线型下拉（PropertiesGroup 第 2 行）`。
- **样式**：下拉行（线型名 + ▾），弹出列表面板宽 220、高 200、可滚动（`prop_linetype_overlay`）。
- **触发命令**：`Message::RibbonLinetypeChanged`。
- **实现位置**：`src/ui/ribbon/mod.rs::prop_linetype_overlay`。
- **备注**：条目首位固定 `ByLayer`、`ByBlock`，随后追加文档可用线型（去重），每行显示名称与 ASCII 线型预览（`LinetypeItem`）。

### 线宽下拉（Lineweight）
- **功能简介**：设置当前对象线宽。
- **UI 入口**：`Ribbon → Draw 选项卡 → Properties 面板 → 线宽下拉（PropertiesGroup 第 3 行）`。
- **样式**：下拉行（线宽文本 + ▾），弹出面板宽 140（`prop_lw_overlay`）。
- **触发命令**：`Message::RibbonLineweightChanged`。
- **实现位置**：`src/ui/ribbon/mod.rs::prop_lw_overlay`；选项来源 `src/ui/properties.rs::lw_options`。
- **备注**：全部选项：ByLayer、ByBlock、Default、0、5、9、13、15、18、20、25、30、35、40、50、53、60、70、80、90、100、106、120、140、158、200、211。

---

## 七、Groups 面板（Ribbon → Draw 选项卡 → Groups 面板）

### 编组（Group）
- **功能简介**：将选中对象编为一个具名组。
- **UI 入口**：`Ribbon → Draw 选项卡 → Groups 面板 → 编组（LargeTool）`。
- **样式**：`LargeTool`（图标 group.svg，标签“Group”）。
- **触发命令**：`GROUP`（`G`）。
- **实现位置**：`src/modules/draw/groups/group.rs::tool()`、`GroupCommand`（Enter 用自动名，或输入名）；`src/app/commands/layers.rs::"GROUP"`。
- **备注**：提示 “GROUP  Enter group name [自动名]:”。

### 解组（Ungroup）
- **功能简介**：解散选中的组。
- **UI 入口**：`Ribbon → Draw 选项卡 → Groups 面板 → 解组（LargeTool）`。
- **样式**：`LargeTool`（图标 ungroup.svg，标签“Ungroup”）。
- **触发命令**：`UNGROUP`。
- **实现位置**：`src/modules/draw/groups/ungroup.rs::tool()`、`UngroupCommand`；`src/app/commands/layers.rs::"UNGROUP"`。

---

## 八、Clipboard 面板（Ribbon → Draw 选项卡 → Clipboard 面板）

### 粘贴下拉（Paste）
- **功能简介**：将剪贴板内容粘贴到图形中。
- **UI 入口**：`Ribbon → Draw 选项卡 → Clipboard 面板 → 粘贴（LargeDropdown，默认 PASTECLIP）`。
- **样式**：`LargeDropdown`（图标 paste.svg，标签“Paste”）。
- **触发命令**：默认 `PASTECLIP`。下拉子项（`paste::MENU_ITEMS`，全部）：
  - `PASTECLIP` — Paste（粘贴，指定插入点）
  - `PASTEORIG` — Paste to Original Coordinates（粘贴到原坐标）
  - `PASTEBLOCK` — Paste as Block（粘贴为块）
- **实现位置**：`src/modules/draw/mod.rs`（`PASTE_MENU`）；`src/modules/draw/clipboard/paste.rs::ICON/MENU_ITEMS/PasteCommand`。
- **备注**：大剪贴板（>20000 线或 >300000 点）改用包围盒线框做预览（`paste.rs` 常量 `MAX_PREVIEW_WIRES`/`MAX_PREVIEW_POINTS`）。

### 复制（Copy to Clipboard）
- **功能简介**：将选中对象复制到剪贴板。
- **UI 入口**：`Ribbon → Draw 选项卡 → Clipboard 面板 → 复制（Tool）`。
- **样式**：`Tool`（图标 copy_clip.svg，标签“Copy”）。
- **触发命令**：`COPYCLIP`。
- **实现位置**：`src/modules/draw/clipboard/copy_clip.rs::tool()`。
- **备注**：另有 `COPYBASE`（指定基点复制）经命令行注册（`src/modules/draw/clipboard/copy_base.rs`），并出现在右键剪贴板子菜单 “Copy with Base Point”。

### 剪切（Cut）
- **功能简介**：将选中对象剪切到剪贴板。
- **UI 入口**：`Ribbon → Draw 选项卡 → Clipboard 面板 → 剪切（Tool）`。
- **样式**：`Tool`（图标 cut.svg，标签“Cut”）。
- **触发命令**：`CUTCLIP`。
- **实现位置**：`src/modules/draw/clipboard/cut.rs::tool()`。

---

## 九、Measure 面板（Ribbon → Draw 选项卡 → Measure 面板）

### 测量下拉（Measure）
- **功能简介**：测量两点距离或区域面积。
- **UI 入口**：`Ribbon → Draw 选项卡 → Measure 面板 → 测量（LargeDropdown，默认 DIST）`。
- **样式**：`LargeDropdown`（图标 dist::ICON，标签“Measure”）。
- **触发命令**：默认 `DIST`。下拉子项（`src/modules/draw/mod.rs` 内联，全部）：
  - `DIST` — Distance（距离）
  - `AREA` — Area（面积）
- **实现位置**：`src/modules/draw/mod.rs`（`MEASURE_MENU`）；`src/modules/draw/inquiry/dist.rs::DistCommand`、`src/modules/draw/inquiry/area.rs::AreaCommand`。

#### Distance（DIST）
- **功能简介**：报告两点间距离、XY 平面夹角、与 XY 平面夹角及 Delta X/Y/Z。
- **UI 入口**：`Ribbon → Draw 选项卡 → Measure 面板 → 测量下拉 → Distance`。
- **样式**：下拉子项（弹出列表行）。
- **触发命令**：`DIST`（`DI`）。
- **实现位置**：`src/modules/draw/inquiry/dist.rs::DistCommand`。

#### Area（AREA）
- **功能简介**：测量点集或对象的面积与周长，支持累加/减去模式。
- **UI 入口**：`Ribbon → Draw 选项卡 → Measure 面板 → 测量下拉 → Area`。
- **样式**：测区以青色预览（`area_preview`）；模式区以 `AreaPreviewRegion` 着色。
- **触发命令**：`AREA`。
- **实现位置**：`src/modules/draw/inquiry/area.rs::AreaCommand`。
- **备注**：选项 Objects(`OBJECTS`)、Add(`ADD`)、Subtract(`SUBTRACT`)；对象收集时 `BACK` 返回。面积支持 curves/3D 多段线/样条（`curve_measurement`/`point_measurement`）。

---

## 十、Draw 面板扩展飞出（面板标题下拉）

- **功能简介**：点击 Draw 面板标题展开更多绘图工具。
- **UI 入口**：`Ribbon → Draw 选项卡 → Draw 面板 → 标题 “Draw ▾”（点击展开）`。
- **样式**：飞出面板为多列 36×36（`CELL = 36*0.7`）小图标网格，滚动，边框主题主色（`src/ui/ribbon/draw_panel.rs::overlay`）。
- **工具列表**（`src/ui/ribbon/draw_panel.rs::TOOLS`，全部）：
  - `SPLINE` — Spline Fit（拟合样条）
  - `SPLINECV` — Spline CV（控制点样条）
  - `XLINE` — Construction Line（构造线）
  - `RAY` — Ray（射线）
  - `DIVIDE` — Divide（定数等分）
  - `MEASURE` — Measure（定距等分）
  - `REGION` — Region（面域）
  - `BOUNDARY` — Boundary（边界）
  - `HELIX` — Helix（螺旋）
  - `DONUT` — Donut（圆环）
  - `MULTIPOINT` — Multiple Points（多点，带下拉：`POINT` Single Point / `MULTIPOINT` Multiple Points / `DDPTYPE` Point Style）
- **实现位置**：`src/ui/ribbon/draw_panel.rs::TOOLS`、`Panel`/`PANELS`、`overlay`；归属解析 `panel_for_dropdown`/`parent_panel`；各命令实现见下节。
- **备注**：MULTIPOINT 的 `options` 非空，所以它是可展开子菜单项（`draw_panel.rs::tool_button`）。

### 扩展飞出工具细则
- **Spline Fit（SPLINE）**：点击拟合点创建样条；命令行可选 Fit/Control vertices、Chord/Square root/Uniform、Method/Object、Degree、Knots、Close、Tangency、Undo（`src/modules/draw/draw/spline.rs`）。
- **Spline CV（SPLINECV）**：以控制顶点方式创建样条（同 `spline.rs`，`SPLINE`/`SPLINECV` 注册）。
- **Construction Line（XLINE）**：无限构造线；关键字 Hor/Ver/Ang/Bisect/Offset、Angle 下 Reference、Offset 下 Through（`src/modules/draw/draw/ray.rs`）。
- **Ray（RAY）**：半无限射线（`ray.rs`）。
- **Divide（DIVIDE）**：沿曲线定数等分放置点或块；步进 Pick→Amount→BlockName→Align；关键字 Block(`B`)、Yes(`Y`)/No(`N`)（`src/modules/draw/inquiry/divide.rs::DivideCommand`）。
- **Measure（MEASURE）**：沿曲线按段长放置点或块；关键字 Block、长度/段数、Yes/No（`divide.rs::MeasureCommand`）。
- **Region（REGION）**：将闭合对象转为面域（`src/app/commands/draw.rs::"REGION" | "REG"`）。
- **Boundary（BOUNDARY）**：创建闭合边界；高级模式 Object type → Region/Polyline（`src/modules/draw/draw/hatch.rs::BoundaryCommand`）。
- **Helix（HELIX）**：创建螺旋；Diameter、Axis endpoint、Turns、Turn Height、Twist、Clockwise/Counterclockwise（`src/modules/draw/draw/helix.rs`；默认半径 1、高 1、圈数 3、逆时针，`defaults.rs`）。
- **Donut（DONUT）**：创建圆环（内外直径）；默认内径 0.5、外径 1.0（`src/modules/draw/draw/donut.rs`、`defaults.rs`）。
- **Multiple Points（MULTIPOINT）**：单点 `POINT` / 多点 `MULTIPOINT` / 点样式 `DDPTYPE`（`src/modules/draw/draw/point.rs`；`PDMODE`/`PDSIZE`/`DDPTYPE` 注册）。

---

## 十一、Reference 面板扩展飞出（面板标题下拉）

- **功能简介**：外部参照相关工具从面板标题角落启动器进入。
- **UI 入口**：`Ribbon → Draw 选项卡 → Reference 面板 → 标题 “Reference ▾” 与角落右箭头启动器`。
- **样式**：`Reference` 面板飞出（宽 260）；包含 “Edit Reference” 行与 “Xref fading” 开关 + 滑块（0~90）（`src/ui/ribbon/draw_panel.rs::reference_overlay`）。
- **触发命令/项**：
  - `REFEDIT` — Edit Reference（编辑参照）
  - 角落启动器：`EXTERNALREFERENCES`（外部参照选项板）
  - Xref fade 切换/滑块：`XDWGFADECTL`（`Message::XrefFadeToggle`/`XrefFadeSlide`/`XrefFadeCommit`）
- **实现位置**：`src/ui/ribbon/draw_panel.rs::REFERENCE_TOOLS`/`REFERENCE_PANEL_ID`/`reference_overlay`；`src/modules/draw/modify/refedit.rs::tool`（`refedit_tools` 提供保存/放弃按钮）。

---

## 十二、未在 Ribbon 出现但属于 Draw 模块的 UI 入口

以下命令未出现在 Draw 选项卡固定面板中，但通过命令行（命令输入框自动补全 `CommandRegistration`）、右键上下文菜单、选择驱动的侧边工具栏或对话框触发。入口与实现如下。

### 1. 命令行可直接触发的绘图/修改命令
- **入口**：`命令输入框`（`inventory::submit!` 注册的名字均可补全并执行）。
- **清单与实现**（全部 Draw 模块命令名，去重后）：
  - 绘制：`LINE`、`MLINE`、`WIPEOUT`、`IMAGE`/`IMAGEATTACH`/`IM`、`IMAGEEMBED`、`REVCLOUD`(+`_RECTANGULAR`/`_POLYGONAL`/`_FREEHAND`)、`ATTDEF`、`DONUT`、`CIRCLE`(+`_CD`/`_2P`/`_3P`/`_TTR`/`_TTT`)、`ARC`(+全部 `ARC_*`)、`RECT`/`RECTANG`、`RECT_ROT`、`RECT_CEN`、`POLY`/`POLYGON`、`POLY_C`、`POLY_E`、`PLINE`、`3DPOLY`、`3DMESH`、`3DFACE`/`EDGE`、`SOLID`/`SOLID2D`/`SO`、`HELIX`、`TRACE`、`CENTERLINE`(+`CENTERRESET`/`CENTERREASSOCIATE`/`CENTERDISASSOCIATE`)、`DIMCENTER`/`DCE`/`CENTERMARK`、`SKETCH`、`POINT`/`MULTIPOINT`、`PDMODE`/`PDSIZE`/`DDPTYPE`、`RAY`、`XLINE`/`CONSTRUCTIONLINE`、`HATCH`、`HATCHEDIT`、`GRADIENT`、`BOUNDARY`、`ELLIPSE`(+`_AXIS`/`_ARC`)、`SPLINE`/`SPLINECV`、`REGION`/`REG`。
  - 修改：`MOVE`/`3DMOVE`、`COPY`、`ROTATE`、`TORIENT`、`SCALE`、`MIRROR`、`STRETCH`、`ERASE`、`TRIM`、`EXTEND`、`EXTRIM`、`FILLET`、`CHAMFER`、`ARRAYRECT`/`ARRAY`、`ARRAYPATH`、`ARRAYPOLAR`、`ARRAY3D`/`3DARRAY`、`OFFSET`、`EXPLODE`、`ALIGN`、`ALIGNLEFT`/`ALIGNHCENTER`/`ALIGNRIGHT`/`ALIGNTOP`/`ALIGNVCENTER`/`ALIGNBOTTOM`、`BREAK`、`BREAKATPOINT`、`JOIN`、`REVERSE`、`NCOPY`、`DRAWORDER_*`、`SETBYLAYER`/`SETBYLAYERMODE`/`-SETBYLAYER`、`LENGTHEN`、`PEDIT`、`SPLINEDIT`、`HATCHEDIT`、`MLEDIT`、`OVERKILL`、`FLATTEN`、`BLEND`/`BLE`、`ATTEDIT`/`-ATTEDIT`、`BEDIT`/`BEDIT_SAVE`/`BEDIT_DISCARD`、`REFEDIT`/`REFCLOSE`/`REFCLOSE_SAVE`/`REFCLOSE_DISCARD`。
  - 查询/测量：`DIST`、`AREA`、`ID`、`MEASUREGEOM`/`MEA`、`DIVIDE`、`MEASURE`、`LIST`、`DBLIST`、`MASSPROP`、`COUNT`、`CAL`/`QUICKCALC`。
  - 图层：`LAYER`/`LA`、`LAYOFF`、`LAYFRZ`、`LAYLCK`、`LAYULK`、`LAYON`、`LAYTHW`、`LAYMCUR`、`LAYISO`、`LAYUNISO`、`LAYDEL`、`LAYMRG`、`LAYTRANS`、`LAYERSTATE`/`LAS`/`LMAN`、`LAYMATCH`/`LAYMCH`、`ISOLATEOBJECTS`、`HIDEOBJECTS`、`UNISOLATEOBJECTS`、`DWGUNITS`。
  - 剪贴板：`COPYCLIP`、`CUTCLIP`、`PASTECLIP`/`PASTE`、`PASTEORIG`、`PASTEBLOCK`、`COPYBASE`。
  - 编组/特性：`GROUP`、`UNGROUP`、`MATCHPROP`。
- **实现位置**：各文件 `inventory::submit!(CommandRegistration {..})`（见上）；命令分发 `src/app/commands/draw.rs`、`src/app/commands/layers.rs`、`src/app/commands/inquiry.rs`、`src/app/commands/layerprops.rs`。

### 2. 右键上下文菜单入口
- **入口**：`视口 → 右键 → 上下文菜单`（`MenuContext::Idle` / `Command` / `Grip`）。
- **Draw 模块相关行**：
  - Clipboard 子菜单：Cut(`CUTCLIP`)、Copy(`COPYCLIP`)、Copy with Base Point(`COPYBASE`)、Paste(`PASTECLIP`)、Paste as Block(`PASTEBLOCK`)、Paste to Original Coordinates(`PASTEORIG`)。
  - 选择编辑块：Erase(`DeleteSelected`)、Move(`MOVE`)、Copy Selection(`COPY`)、Scale(`SCALE`)、Rotate(`ROTATE`)、Mirror(`MIRROR`)、Draw Order 子菜单（Bring to Front `DRAWORDER F`、Send to Back `DRAWORDER B`、Bring Above Object、Send Under Object）。
  - Isolate 子菜单：Isolate Objects(`ISOLATEOBJECTS`)、Hide Objects(`HIDEOBJECTS`)、End Object Isolation(`UNISOLATEOBJECTS`)。
  - 选择工具：Select Similar、Invert Selection、Deselect All、Select All(`SELECTALL`)、Quick Select...。
  - 特性：Properties、Options...
- **实现位置**：`src/ui/popup/context_menu.rs::build_context_menu/idle_rows/command_rows/grip_rows`。
- **备注**：命令运行时的菜单列出该命令当前步骤的关键字（`CmdOption`）、Enter/Cancel、Recent Input、Snap Overrides、Pan/Zoom（`command_rows`）。grip 编辑菜单提供 Stretch/Move/Rotate/Scale/Mirror/Base Point/Copy/Undo/Exit（`grip_rows`）。

### 3. 选择驱动的侧边工具栏（上下文工具）
- **入口**：`视口右侧边工具栏`（依据选择内容出现）。
- **Draw 模块相关工具**（`src/ui/ribbon/context_tools.rs`）：
  - PDF 底图：`_PDFULMONO`、`_PDFULCLIP`、`_PDFULUNCLIP`、`_PDFULSHOW`、`_PDFULSNAP`、`EXTERNALREFERENCES`、`ULAYERS`，PDF 另有 `_PDFULIMPORT`（`pdf_underlay_tools`）。
  - 点云：`_PCCROPRECT`、`_PCCROPPOLY`、`_PCCROPCIRC`、`_PCCROPSHOW`、`_PCCROPINVERT`、`_PCUNCROP`、`EXTERNALREFERENCES`（`point_cloud_tools`）。
  - 外部参照：`_XREFEDIT`、`_XREFOPEN`、`_XREFCLIP`、`_XREFUNCLIP`、`EXTERNALREFERENCES`（`xref_tools`）。
- **实现位置**：`src/ui/ribbon/context_tools.rs`。

### 4. REFEDIT / BEDIT 会话侧边按钮
- **入口**：编辑参照/块编辑空间激活时，右侧边栏出现保存/放弃按钮。
- **项**：`REFCLOSE_SAVE`(Save Block Edit)、`REFCLOSE_DISCARD`(Discard Block Edit)（`src/modules/draw/modify/refedit.rs::refedit_tools`）；`BEDIT_SAVE`(Save Block)、`BEDIT_DISCARD`(Discard Block Edit)（`src/modules/draw/modify/block_edit.rs::block_edit_tools`）。
- **实现位置**：`src/modules/draw/modify/refedit.rs`、`src/modules/draw/modify/block_edit.rs`。

### 5. 选择收集器与手势（非独立按钮，所有修改命令共用）
- **功能简介**：修改命令未预选对象时进入 “Select objects:” 收集阶段，支持单选、窗口、交叉、Fence、WPolygon、CPolygon。
- **UI 入口**：命令行提示按钮与右键菜单选项。
- **触发关键字**：Window(`W`)、Crossing(`C`)、Fence(`F`)、WPolygon(`WP`)、CPolygon(`CP`)、All(`ALL`)、Add(`A`)、Remove(`R`)、Previous(`P`)、Last(`L`)（`src/modules/draw/select.rs::SelectObjectsCommand::options`）。
- **实现位置**：`src/modules/draw/select.rs::SelectObjectsCommand`（`instant`/`plain`/`routed`/`with_prompt`/`auto_constrain` 变体）；手势 `src/modules/draw/fence.rs::FencePick`（虚线青色预览、交叉窗口绿色预览）。
- **备注**：`SelectObjectsCommand` 注册补全名：`ARRAY`、`ARRAYPATH`、`ARRAYPOLAR`、`ARRAYRECT`、`BLOCK`、`COPY`、`COPYCLIP`、`CUTCLIP`、`ERASE`、`EXPLODE`、`GROUP`、`LAYFRZ`、`LAYLCK`、`LAYMCUR`、`LAYOFF`、`LAYULK`、`MIRROR`、`MOVE`、`ROTATE`、`SCALE`、`STRETCH`。

### 6. 图层转换（Layer Translator）
- **功能简介**：把本图图层映射到另一图纸的图层集并迁移对象。
- **UI 入口**：`命令输入框 → LAYTRANS`（命令行 `LAYTRANS <path>` 打开/加载）；窗口由 `src/ui/window/layer_translator.rs` 提供。
- **触发命令**：`LAYTRANS`。
- **实现位置**：`src/modules/draw/layers/laytrans.rs::load_targets/map_same/translate/merge_layer`；`src/app/commands/layers.rs::"LAYTRANS"`。

### 7. 图形单位（Drawing Units）
- **功能简介**：设置长度/角度格式（LUNITS/AUNITS）与单位标签（INSUNITS），并可换算几何。
- **UI 入口**：`状态栏单位按钮 → 单位弹窗`（`src/ui/popup/units_popup.rs`）；命令 `DWGUNITS`。
- **触发命令**：`DWGUNITS`。
- **实现位置**：`src/modules/draw/units.rs::linear_formats/angular_formats/all/conversion_factor`；命令 `src/app/commands/layers.rs::"DWGUNITS"`；窗口 `src/ui/window/drawing_units.rs`。

### 8. 会话默认值（Last-used defaults）
- **功能简介**：记忆圆半径、旋转角、缩放因子、偏移距离、圆角半径、倒角距离、阵列行列/间距/项数、多边形边数、矩形宽度/旋转/倒角/圆角、圆环内外径、螺旋参数等。
- **UI 入口**：无独立按钮；作为命令提示中的默认值出现。
- **实现位置**：`src/modules/draw/defaults.rs`（thread_local 默认值与 accessor）。
- **备注**：例如 OFFSET 默认 1.0、FILLET 半径 1.0、CHAMFER 距离 10/10、ARRAY 行/列 2 与间距 100、HELIX 半径 1/高 1/圈数 3、DONUT 内径 0.5/外径 1.0。

---

## 附：其他 Draw 模块内部/辅助功能

### 报告（Report）与变更日志（Changelog）
- **功能简介**：Draw 模块内定义的 “Report” 与 “Changelog” 工具（当前无 Ribbon 按钮，`#[allow(dead_code)]`）。
- **实现位置**：`src/modules/draw/report.rs::tool()`（`REPORT`）、`src/modules/draw/changelog.rs::tool()`（`CHANGELOG`）。
- **备注**：据源码推断为备用/待接线入口；不在当前 Draw 选项卡中。

### 屏幕菜单/别名命令（3D 与约束类）
- **功能简介**：`src/app/commands/draw.rs` 中还包含大量 3D 与参数约束命令（`CYLINDER`、`BOX`、`SPHERE`、`UNION`、`INTERSECT`、`SUBTRACT`、`FLATSHOT`、`POLYSOLID`、`SECTION`、`SECTIONPLANE`、`3DALIGN`、`3DMIRROR`/`MIRROR3D`、`3DROTATE`/`ROTATE3D`、`SLICE`、各类 `*CONSTRAINT`/`DC*`/`GC*` 等）。
- **入口**：`命令输入框`（部分经 Insert/其它模块面板）。
- **实现位置**：`src/app/commands/draw.rs`。
- **备注**：这些命令虽在同一分发函数中，但归属于其它模块面板或未在当前 Draw 选项卡暴露；此处仅说明未在 Ribbon Draw 面板出现的入口。

### 属性面板（Properties 停靠面板）
- **功能简介**：选中对象时显示/编辑对象特性；颜色/线型/线宽与 Ribbon Properties 面板同源。
- **UI 入口**：`右键 → Properties`（`MenuAction::Properties`）；停靠面板 `src/ui/properties.rs::PropertiesPanel`。
- **实现位置**：`src/ui/properties.rs`（`lw_options`、`LinetypeItem`、`render_color_row`、`render_linetype_row`）；颜色选择器 `src/ui/color_select.rs::color_selector_with_name`。

### 对象颜色选择器（完整选色窗口）
- **功能简介**：从颜色下拉 “Select Color...” 打开的完整选色对话框。
- **UI 入口**：`Ribbon → Properties 面板 → 颜色下拉 → Select Color...`。
- **触发命令**：`Message::OpenColorWindow(ColorPickTarget::Ribbon, color)`。
- **实现位置**：`src/ui/color_select.rs`；窗口 `src/ui/window/`（颜色选择相关）。

---

（文档结束）
