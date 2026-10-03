# Ribbon「Parametric」选项卡功能需求清单

本文档反推自 `src/modules/parametric/`（`mod.rs`、`tools.rs`、`coincident.rs`、`concentric.rs`、`equal.rs`、`equal_distance.rs`、`fixed.rs`、`horizontal.rs`、`perpendicular.rs`、`point_on_entity.rs`、`smooth.rs` 及经 `#[path]` 引入的 `symmetric.rs`、`tangent.rs`、`geom_constraint.rs`、`dim_constraint.rs`、`value.rs`、`parameters_cli.rs`、`constraint_bar.rs`）、`src/ui/window/named_parameters.rs`、`src/ui/window/auto_constrain_settings.rs`、`src/app/drafting_settings.rs`、`src/ui/overlay.rs`（约束条/图标绘制）、`src/ui/ribbon/widgets.rs`（工具提示与渲染）、`src/ui/popup/context_menu.rs`、`src/app/commands/draw.rs`（约束命令分发）、`src/app/update/viewport.rs`（约束图标命中）、`src/app/settings.rs`（`AutoConstrainSettings`）。Ribbon 面板与工具的权威布局入口是 `src/modules/parametric/mod.rs::ParametricModule::ribbon_groups()`。

面板组的权威顺序（`src/modules/parametric/mod.rs`）：Geometric、Dimensional、Manage。面板内的每个 Ribbon 项由 `ToolDef`/`RibbonItem` 定义（`src/modules/mod.rs`），工具提示统一由 `src/ui/ribbon/widgets.rs::tool_tip_text` 生成（名称 + 描述 + `Command: <ID>`）。

---

## 一、Geometric 面板（Ribbon → Parametric 选项卡 → Geometric 面板）

面板布局（`src/modules/parametric/mod.rs::ribbon_groups`，依次）：`LargeTool(Auto Constrain)`、`LargeTool(Coincident)`、`LargeTool(Parallel)`、`LargeTool(Tangent)`、`LargeTool(Colinear)`、`LargeTool(Perpendicular)`、`LargeTool(Smooth)`、`LargeTool(Concentric)`、`LargeTool(Horizontal)`、`LargeTool(Symmetric)`、`LargeTool(Fix)`、`LargeTool(Vertical)`、`LargeTool(Equal)`、`LabeledDropdown(Show/Hide)`、`LabeledTool(Show All)`、`LabeledTool(Hide All)`。

### 自动约束（Auto Constrain）
- **功能简介**：对选定对象一次性推断并应用所有符合条件的几何约束，推断类型、优先级与容差来自「Auto Constrain 设置」。
- **UI 入口**：`Ribbon → Parametric 选项卡 → Geometric 面板 → Auto Constrain（LargeTool）`。
- **样式**：`LargeTool`（大按钮，图标 `assets/icons/constrain/auto.svg` + “Auto Constrain”标签；工具提示含命令 `AUTOCONSTRAIN`）。
- **触发命令**：`AUTOCONSTRAIN`（设置对话框为 `CONSTRAINTSETTINGS`）。
- **实现位置**：`src/modules/parametric/mod.rs`（tool id `AUTOCONSTRAIN`）；分发 `src/app/commands/draw.rs::"AUTOCONSTRAIN"`；推断逻辑 `Scene::inferred_parametric_constraints`；设置结构 `src/app/settings.rs::AutoConstrainSettings`。
- **备注**：无预选对象时进入选择收集阶段（`SelectObjectsCommand::auto_constrain`），提示 `AUTOCONSTRAIN Select objects or [Settings]:`，可键入 `S`/`SETTINGS` 打开设置（`src/modules/draw/select.rs::on_text_input` → `CmdResult::OpenAutoConstrainSettings`）。有预选时直接对选择集推断。结果输出 “N constraint(s) applied to M object(s).”（`draw.rs:1254`）。

### 重合约束（Coincident）
- **功能简介**：将两个点（或一个点与一条曲线、两个对象）约束为重合，也可对选择集自动推断重合关系。
- **UI 入口**：`Ribbon → Parametric 选项卡 → Geometric 面板 → Coincident（LargeTool）`。
- **样式**：`LargeTool`（图标 `assets/icons/constrain/coincident.svg`，标签 “Coincident”）。
- **触发命令**：`CCONSTRAINT`（别名 `GCCOINCIDENT`）。
- **实现位置**：`src/modules/parametric/coincident.rs::coincident_tool::tool()`、`CoincidentConstraintCommand`；分发 `src/app/commands/draw.rs::"CCONSTRAINT" | "GCCOINCIDENT"`。
- **备注**：首个提示 `COINCIDENT Select first point or [Object/Autoconstrain] <Object>:`；选项 `Object`(`O`)、`Autoconstrain`(`A`)；选到曲线后提示 `Select point or [Multiple]:`，选项 `Point`(`P`)、`Multiple`(`M`)，Multiple 下可连续放置点并以 Enter 结束；Autoconstrain 模式收集对象后 Enter 应用（`on_enter` → `AddAutoCoincidentConstraints`）。可接受的曲线类型：Line/LwPolyline/Polyline2D/Circle/Arc/Ellipse/Spline（`valid_curve`）。

### 平行约束（Parallel）
- **功能简介**：使第二条对象与第一条对象保持平行。
- **UI 入口**：`Ribbon → Parametric 选项卡 → Geometric 面板 → Parallel（LargeTool）`。
- **样式**：`LargeTool`（图标 `assets/icons/constrain/parallel.svg`，标签 “Parallel”）。
- **触发命令**：`PCONSTRAINT`。
- **实现位置**：`src/modules/parametric/tools.rs::parallel::tool()`；分发 `src/app/commands/draw.rs::"PCONSTRAINT" | "LCONSTRAINT" | "NRCONSTRAINT"`。
- **备注**：需恰好两个对象（首个为参考，第二个随之移动）；无选择时经 `SelectObjectsCommand` 收集，多于/少于两个时输出提示。约束类型 `ConstraintKind::Parallel`（`draw.rs:1852`）。

### 相切约束（Tangent）
- **功能简介**：使直线与圆/弧/椭圆（或两圆）相切。
- **UI 入口**：`Ribbon → Parametric 选项卡 → Geometric 面板 → Tangent（LargeTool）`。
- **样式**：`LargeTool`（图标 `assets/icons/constrain/tangent.svg`，标签 “Tangent”）。
- **触发命令**：`TCONSTRAINT`。
- **实现位置**：`src/modules/parametric/tangent.rs::tangent_command::tool()`（经 `tools::tangent::tool()` 接入）、`TangentConstraintCommand`；分发 `src/app/commands/draw.rs::"TCONSTRAINT"`。
- **备注**：两个对象在命令内依次拾取，提示 `TCONSTRAINT Select first/second object:`。支持组合：圆-圆、圆-线、线-圆、椭圆-线、线-椭圆（`pair_supported`）；同一对象或非法组合报 “Invalid selection for Tangent. Select a line, polyline segment, circle, arc or ellipse.”。预选两个对象可直接应用（`draw.rs:1790`）。

### 共线约束（Colinear）
- **功能简介**：使两条直线（或多段线直线段）落在同一直线上。
- **UI 入口**：`Ribbon → Parametric 选项卡 → Geometric 面板 → Colinear（LargeTool）`。
- **样式**：`LargeTool`（图标 `assets/icons/constrain/colinear.svg`，标签 “Colinear”）。
- **触发命令**：`LCONSTRAINT`。
- **实现位置**：`src/modules/parametric/tools.rs::colinear::tool()`；分发 `src/app/commands/draw.rs::"LCONSTRAINT"`（与 Parallel/Normal 同分支，类型 `ConstraintKind::Colinear`）。
- **备注**：需恰好两个对象；提示 “Select exactly two entities (first = reference, second = the one that moves), then run this constraint again.”（`draw.rs:1846`）。

### 垂直约束（Perpendicular）
- **功能简介**：使两条直线（或直线与文本基线/椭圆轴等方向对象）相互垂直。
- **UI 入口**：`Ribbon → Parametric 选项卡 → Geometric 面板 → Perpendicular（LargeTool）`。
- **样式**：`LargeTool`（图标 `assets/icons/constrain/perpendicular.svg`，标签 “Perpendicular”）。
- **触发命令**：`GCPERPENDICULAR`（工具 id 为 `QCONSTRAINT`）。
- **实现位置**：`src/modules/parametric/tools.rs::perpendicular::tool()`、`src/modules/parametric/perpendicular.rs::PerpendicularConstraintCommand`；分发 `src/app/commands/draw.rs::"QCONSTRAINT" | "GCPERPENDICULAR"`。
- **备注**：命令内两次拾取（`Select first/second object:`）；也可预选两个对象直接应用（`draw.rs:1751`）。可拾取 Line、多段线直线段、Text/MText 基线、椭圆长/短轴；同对象或非法对象报错。

### 平滑约束（Smooth，G²）
- **功能简介**：使样条端点与另一条曲线端点保持相切连续（G² 平滑）。
- **UI 入口**：`Ribbon → Parametric 选项卡 → Geometric 面板 → Smooth（LargeTool）`。
- **样式**：`LargeTool`（图标 `assets/icons/constrain/smooth.svg`，标签 “Smooth”）。
- **触发命令**：`GCSMOOTH`。
- **实现位置**：`src/modules/parametric/smooth.rs::SmoothConstraintCommand`；tool 由 `src/modules/parametric/mod.rs` 内联定义（`GCSMOOTH`）；分发 `src/app/commands/draw.rs::"GCSMOOTH"`。
- **备注**：第一个选择必须是开放样条的端点，第二个为 Line/Arc/多段线段/开放样条的端点（`source_reference`/`target_reference`）。预选须恰好两个对象，否则报 “Smooth requires an open spline first and a target curve second.”；约束类型 `ConstraintKind::Smooth`。

### 同心约束（Concentric）
- **功能简介**：使两个圆/弧/椭圆（或多段线圆弧段）共用同一个圆心。
- **UI 入口**：`Ribbon → Parametric 选项卡 → Geometric 面板 → Concentric（LargeTool）`。
- **样式**：`LargeTool`（图标 `assets/icons/constrain/concentric.svg`，标签 “Concentric”）。
- **触发命令**：`GCCONCENTRIC`。
- **实现位置**：`src/modules/parametric/tools.rs::concentric::tool()`、`src/modules/parametric/concentric.rs::ConcentricConstraintCommand`；分发 `src/app/commands/draw.rs::"GCCONCENTRIC"`。
- **备注**：命令内 `Select first/second object:`；可拾取 Circle/Arc/Ellipse（取圆心）及多段线圆弧段（取弧段圆心 `ParametricRef::segment_center`）。同对象或非法报 “Invalid selection for Concentric. Select a circle, arc, ellipse or polyline arc segment.”；预选两个可直接应用（`draw.rs:1868`）。

### 水平约束（Horizontal）
- **功能简介**：使直线/多段线直线段水平，或使两个约束点具有相同的 Y 值。
- **UI 入口**：`Ribbon → Parametric 选项卡 → Geometric 面板 → Horizontal（LargeTool）`。
- **样式**：`LargeTool`（图标 `assets/icons/constrain/horizontal.svg`，标签 “Horizontal”）。
- **触发命令**：`GCHORIZONTAL`。
- **实现位置**：`src/modules/parametric/tools.rs::horizontal::tool()`、`src/modules/parametric/horizontal.rs::HorizontalConstraintCommand`；分发 `src/app/commands/draw.rs::"GCHORIZONTAL"`。
- **备注**：两种输入——对象模式（拾取 Line/Ray/XLine、多段线直线段、Text/MText 基线、椭圆长/短轴）或 `2Points`（两次拾取约束点）。提示 `GCHORIZONTAL Select an object or [2Points] <2Points>:`；选项 `2Points`(`2P`)，Enter 默认进入 2Points。同点报 “The object or point is already selected. …”。方向取当前工作平面 X 轴（`set_working_plane`）。

### 对称约束（Symmetric）
- **功能简介**：使两个对象（或两点）关于一条对称线对称。
- **UI 入口**：`Ribbon → Parametric 选项卡 → Geometric 面板 → Symmetric（LargeTool）`。
- **样式**：`LargeTool`（图标 `assets/icons/constrain/symmetric.svg`，标签 “Symmetric”）。
- **触发命令**：`SYCONSTRAINT`。
- **实现位置**：`src/modules/parametric/tools.rs::symmetric::tool()`、`src/modules/parametric/symmetric.rs::SymmetricConstraintCommand`；分发 `src/app/commands/draw.rs::"SYCONSTRAINT"`。
- **备注**：对象模式 `Select first object or [2Points] <2Points>:` → `Select second object:` → `Select symmetry line:`；2Points 模式先两点再选对称线。两对象须同族（Line/Circular/Ellipse）且不相同，对称线不能是其中之一（`preselected_refs`）。可预选三个对象（两对象 + 轴）直接应用（`draw.rs:1906`）。

### 固定约束（Fix）
- **功能简介**：将曲线/多段线段或某个约束点固定在当前位置（锁定）。
- **UI 入口**：`Ribbon → Parametric 选项卡 → Geometric 面板 → Fix（LargeTool）`。
- **样式**：`LargeTool`（图标 `assets/icons/constrain/fixed.svg`，标签 “Fix”）。
- **触发命令**：`FXCONSTRAINT`（别名 `GCFIX`）。
- **实现位置**：`src/modules/parametric/tools.rs::fixed::tool()`、`src/modules/parametric/fixed.rs::FixConstraintCommand`；分发 `src/app/commands/draw.rs::"FXCONSTRAINT" | "GCFIX"`。
- **备注**：提示 `FXCONSTRAINT Select point or [Object] <Object>:`；选项 `Object`(`O`)，Enter 默认对象模式。预选单个整曲线可直接应用（Line/Circle/Arc/Ellipse/Spline → whole，两顶点开放多段线 → segment）；否则进入点或对象拾取流程。约束类型 `ConstraintKind::Fixed`。

### 竖直约束（Vertical）
- **功能简介**：使直线/多段线直线段竖直，或使两个约束点具有相同的 X 值。
- **UI 入口**：`Ribbon → Parametric 选项卡 → Geometric 面板 → Vertical（LargeTool）`。
- **样式**：`LargeTool`（图标 `assets/icons/constrain/vertical.svg`，标签 “Vertical”）。
- **触发命令**：`VCONSTRAINT`（命令内部名为 `GCVERTICAL`）。
- **实现位置**：`src/modules/parametric/tools.rs::vertical::tool()`（id `VCONSTRAINT`）、`src/modules/parametric/horizontal.rs::HorizontalConstraintCommand::vertical()`；分发 `src/app/commands/draw.rs::"VCONSTRAINT" | "GCVERTICAL"`。
- **备注**：与 Horizontal 同一实现，镜像到工作平面 Y 轴；对象模式或 `2Points`，提示 `GCVERTICAL Select an object or [2Points] <2Points>:`。可接受对象同 Horizontal。

### 相等约束（Equal）
- **功能简介**：使第二条（或后续多个）对象的长度或半径等于第一个对象。
- **UI 入口**：`Ribbon → Parametric 选项卡 → Geometric 面板 → Equal（LargeTool）`。
- **样式**：`LargeTool`（图标 `assets/icons/constrain/equal.svg`，标签 “Equal”）。
- **触发命令**：`ECONSTRAINT`（别名 `GCEQUAL`）。
- **实现位置**：`src/modules/parametric/tools.rs::equal::tool()`、`src/modules/parametric/equal.rs::EqualConstraintCommand`；分发 `src/app/commands/draw.rs::"ECONSTRAINT" | "GCEQUAL"`。
- **备注**：两对象均在命令内拾取，预选不使用（`draw.rs:1831` deselect）。提示 `GCEQUAL Select first object or [Multiple]:`，选项 `Multiple`(`M`)；首个对象定尺寸，第二个跟随；Multiple 下每次拾取立即应用，Enter 结束。可拾取 Line/Circle/Arc（整对象）或多段线直线段（`segment`）。错误信息含 “Invalid selection for Equal. Select a line, polyline segment, circle or arc.” 等。

### 显示/隐藏约束条下拉（Show/Hide）
- **功能简介**：对选中对象显示、隐藏或重置其约束条（约束图标）显示状态。
- **UI 入口**：`Ribbon → Parametric 选项卡 → Geometric 面板 → Show/Hide（LabeledDropdown，默认 GCSHOW）`。
- **样式**：`LabeledDropdown`（1 行高，宽 `LABELED_SMALL_W + ARROW_W`，带 ▾；默认图标 `assets/icons/constrain/show.svg`，标签 “Show/Hide”）。
- **触发命令**：默认 `GCSHOW`。下拉子项（全部）：
  - `GCSHOW` — Show（显示，图标 show.svg）
  - `GCHIDE` — Hide（隐藏，图标 hide_all.svg）
  - `GCRESET` — Reset（重置，图标 show.svg）
- **实现位置**：`src/modules/parametric/mod.rs::ribbon_groups`（`RibbonItem::LabeledDropdown { id: "GCVISIBILITY", … }`）；分发 `src/app/commands/draw.rs::"GCSHOW" | "GCHIDE"`（选中对象时调用 `Scene::set_parametric_constraint_visibility`）与 `"GCRESET"`。
- **备注**：无选择时进入 `SelectObjectsCommand::new(cmd)` 收集；有选择时更新对应对象的约束条可见性，输出 “N constraint indicator(s) updated.”。GCRESET 不经选择、对全作用域重置几何约束可见性。

### 全部显示（Show All）
- **功能简介**：显示作用域内所有几何约束条。
- **UI 入口**：`Ribbon → Parametric 选项卡 → Geometric 面板 → Show All（LabeledTool）`。
- **样式**：`LabeledTool`（图标 `assets/icons/constrain/show_all.svg` + “Show All”）。
- **触发命令**：`GCSHOWALL`。
- **实现位置**：`src/modules/parametric/mod.rs`（内联 `GCSHOWALL`）；分发 `src/app/commands/draw.rs::"GCSHOWALL" | …`。
- **备注**：`dimensional = cmd.starts_with("DC")`，此处为几何约束；输出 “N constraint indicator(s) updated.”。

### 全部隐藏（Hide All）
- **功能简介**：隐藏作用域内所有几何约束条。
- **UI 入口**：`Ribbon → Parametric 选项卡 → Geometric 面板 → Hide All（LabeledTool）`。
- **样式**：`LabeledTool`（图标 `assets/icons/constrain/hide_all.svg` + “Hide All”）。
- **触发命令**：`GCHIDEALL`。
- **实现位置**：`src/modules/parametric/mod.rs`（内联 `GCHIDEALL`）；分发 `src/app/commands/draw.rs`（同 `SHOWALL` 分支）。
- **备注**：与 Show All 相反，`visible = false`。

---

## 二、Dimensional 面板（Ribbon → Parametric 选项卡 → Dimensional 面板）

面板布局（`src/modules/parametric/mod.rs::ribbon_groups`，依次）：`LargeDropdown(DC_LINEAR_MENU / Linear)`、`LargeTool(DCALIGNED / Aligned)`、`ToolGrid { columns: [[DCRADIUS, DCANGULAR], [DCDIAMETER, DCCONVERT]] }`、`LabeledDropdown(DCVISIBILITY / Show/Hide)`、`LabeledTool(DCSHOWALL / Show All)`、`LabeledTool(DCHIDEALL / Hide All)`。

### 线性尺寸下拉（Linear）
- **功能简介**：在两约束点（或对象两端）之间创建线性尺寸约束，并指定尺寸线位置与驱动参数名/表达式。
- **UI 入口**：`Ribbon → Parametric 选项卡 → Dimensional 面板 → Linear（LargeDropdown，默认 DCLINEAR）`。
- **样式**：`LargeDropdown`（大按钮带 ▾；默认图标 `assets/icons/dim_linear.svg`，标签 “Linear”；工具提示描述见 `widgets.rs::tool_description`）。
- **触发命令**：默认 `DCLINEAR`。下拉子项（全部）：
  - `DCLINEAR` — Linear（水平或竖直由尺寸线位置决定，图标 dim_linear.svg）
  - `DCHORIZONTAL` — Horizontal（X 方向距离，图标 `constrain/distance_x.svg`）
  - `DCVERTICAL` — Vertical（Y 方向距离，图标 `constrain/distance_y.svg`）
- **实现位置**：`src/modules/parametric/value.rs::dimensional_tools::{linear,horizontal,vertical}`；`src/modules/parametric/dim_constraint.rs::DimConstraintCommand`（`DimConstraintAxis::{Linear,Horizontal,Vertical}`）；分发 `src/app/commands/draw.rs::"DCLINEAR" | "DCHORIZONTAL" | "DCVERTICAL" | "DCALIGNED"`。
- **备注**：提示起点 `DCLINEAR Specify first constraint point or [Object] <Object>:`，选项 `Object`(`O`)；Object 模式取 Line/Arc 两端点或多段线段两顶点；随后 `Specify dimension line location:` 报告 `Dimension text = …`；最后 `Enter value or name and value <dN=…>:`。键入 `name=expression` 重命名参数。按尺寸线位置决定走 X 还是 Y（`decide` → `ConstraintKind::DistanceX/DistanceY`）。

### 对齐尺寸（Aligned）
- **功能简介**：约束两点间沿指定方向的真实距离（可垂直于某条线测量），支持 Point & line 与 2Lines 方式。
- **UI 入口**：`Ribbon → Parametric 选项卡 → Dimensional 面板 → Aligned（LargeTool）`。
- **样式**：`LargeTool`（图标 `assets/icons/dim_aligned.svg`，标签 “Aligned”）。
- **触发命令**：`DCALIGNED`。
- **实现位置**：`src/modules/parametric/value.rs::dimensional_tools::aligned()`、`dim_constraint.rs::DimConstraintCommand`（`DimConstraintAxis::Aligned`）；分发 `src/app/commands/draw.rs::"DCALIGNED"`。
- **备注**：提示 `DCALIGNED Specify first constraint point or [Object/Point & line/2Lines] <Object>:`，选项 `Object`(`O`)、`Point & line`(`P`)、`2Lines`(`2L`)。2Lines 会先让第二条线平行再测距（`CmdResult::MakeParallel` / `accept_parallel_line`）。约束类型为 `ConstraintKind::Distance` 或 `DistanceDirected`。

### 半径约束（Radius，工具网格）
- **功能简介**：约束圆或圆弧的半径值为指定值/参数。
- **UI 入口**：`Ribbon → Parametric 选项卡 → Dimensional 面板 → ToolGrid 第 1 列第 1 个（Radius）`。
- **样式**：`ToolGrid` 小图标按钮（无文字，图标 `assets/icons/dim_radius.svg`）。
- **触发命令**：`DCRADIUS`。
- **实现位置**：`src/modules/parametric/value.rs::dimensional_tools::radius()`；`dim_constraint.rs::DimConstraintCommand`（`DimConstraintAxis::Radius`）；分发 `src/app/commands/draw.rs::"DCRADIUS" | "DCDIAMETER"`。
- **备注**：提示 `DCRADIUS Select arc or circle:` → `Specify dimension line location:` → `Enter value or name and value <rN=…>:`。参数名前缀由 `next_radial_parameter_name` 生成。

### 直径约束（Diameter，工具网格）
- **功能简介**：约束圆或圆弧的直径值为指定值/参数。
- **UI 入口**：`Ribbon → Parametric 选项卡 → Dimensional 面板 → ToolGrid 第 2 列第 1 个（Diameter）`。
- **样式**：`ToolGrid` 小图标按钮（图标 `assets/icons/dim_diameter.svg`）。
- **触发命令**：`DCDIAMETER`。
- **实现位置**：`src/modules/parametric/value.rs::dimensional_tools::diameter()`；`dim_constraint.rs::DimConstraintCommand`（`DimConstraintAxis::Diameter`）；分发 `src/app/commands/draw.rs`（同 Radius 分支，`diameter` 标志）。
- **备注**：测量值取半径 ×2；提示与半径类似，参数名前缀 `next_radial_parameter_name(table, true)`。

### 角度约束（Angular，工具网格）
- **功能简介**：约束两条线（或多段线直线段）之间的夹角，或圆弧的圆心角，或三点定义的角。
- **UI 入口**：`Ribbon → Parametric 选项卡 → Dimensional 面板 → ToolGrid 第 1 列第 2 个（Angular）`。
- **样式**：`ToolGrid` 小图标按钮（图标 `assets/icons/dim_angular.svg`）。
- **触发命令**：`DCANGULAR`（别名 `ACONSTRAINT`）。
- **实现位置**：`src/modules/parametric/value.rs::dimensional_tools::angular()`、`dim_constraint.rs::DimConstraintCommand`（`DimConstraintAxis::Angular`）；分发 `src/app/commands/draw.rs::"ACONSTRAINT" | "DCANGULAR"`。
- **备注**：提示 `DCANGULAR Select first line or arc or [3Point] <3Point>:`，选项 `3Point`(`3P`)；两线方式再 `Select second line:`，随后 `Specify dimension line location:` 决定所测扇区（`angle_frame`）；三点方式依次 `Specify angle vertex:`/`first angle constraint point`/`second angle constraint point`。平行线报 “Lines are parallel.”。角度小数位取自 `angle_decimals`（DIMADEC）。

### 转换尺寸为约束（Convert，工具网格）
- **功能简介**：将选定的关联尺寸批量转换为尺寸约束（为每个尺寸新建参数并接管其尺寸文字）。
- **UI 入口**：`Ribbon → Parametric 选项卡 → Dimensional 面板 → ToolGrid 第 2 列第 2 个（Convert）`。
- **样式**：`ToolGrid` 小图标按钮（图标 `assets/icons/constrain/convert.svg`）。
- **触发命令**：`DCCONVERT`（应用阶段 `DCCONVERTAPPLY`）。
- **实现位置**：`src/modules/parametric/value.rs::dimensional_tools::convert()`；分发 `src/app/commands/draw.rs::"DCCONVERT"` → 选择关联尺寸 → `"DCCONVERTAPPLY"`。
- **备注**：提示 “Select associative dimensions to convert:”，仅收集关联尺寸（`SelectObjectsCommand::associative_dimensions`）。已驱动约束的尺寸、锁定图层、无法解析的尺寸与校验失败的均计入 refused。动态形式时尺寸迁到约束灰层（`DYNAMIC_DIMENSION_LAYER`）并改写文字；输出 “N associative dimensions converted” 与 “M associative dimension(s) could not be converted”。

### 显示/隐藏尺寸约束条下拉（Show/Hide）
- **功能简介**：对选中对象显示或隐藏其尺寸约束条。
- **UI 入口**：`Ribbon → Parametric 选项卡 → Dimensional 面板 → Show/Hide（LabeledDropdown，默认 DCSHOW）`。
- **样式**：`LabeledDropdown`（1 行高，图标 `assets/icons/constrain/show.svg`，标签 “Show/Hide”）。
- **触发命令**：默认 `DCSHOW`。下拉子项（全部）：
  - `DCSHOW` — Show（显示）
  - `DCHIDE` — Hide（隐藏）
- **实现位置**：`src/modules/parametric/mod.rs`（`RibbonItem::LabeledDropdown { id: "DCVISIBILITY", … }`）；分发 `src/app/commands/draw.rs::"DCSHOW" | "DCHIDE"`（`dimensional = true`）。
- **备注**：与 Geometric 的 Show/Hide 共用分发逻辑，但标记为尺寸约束维度。无选择时进入对象收集。

### 全部显示（Show All）
- **功能简介**：显示作用域内所有尺寸约束条。
- **UI 入口**：`Ribbon → Parametric 选项卡 → Dimensional 面板 → Show All（LabeledTool）`。
- **样式**：`LabeledTool`（图标 `assets/icons/constrain/show_all.svg` + “Show All”）。
- **触发命令**：`DCSHOWALL`。
- **实现位置**：`src/modules/parametric/mod.rs`（内联 `DCSHOWALL`）；分发 `src/app/commands/draw.rs::"DCSHOWALL"`。

### 全部隐藏（Hide All）
- **功能简介**：隐藏作用域内所有尺寸约束条。
- **UI 入口**：`Ribbon → Parametric 选项卡 → Dimensional 面板 → Hide All（LabeledTool）`。
- **样式**：`LabeledTool`（图标 `assets/icons/constrain/hide_all.svg` + “Hide All”）。
- **触发命令**：`DCHIDEALL`。
- **实现位置**：`src/modules/parametric/mod.rs`（内联 `DCHIDEALL`）；分发 `src/app/commands/draw.rs`（同 Show All 分支，`visible = false`）。

---

## 三、Manage 面板（Ribbon → Parametric 选项卡 → Manage 面板）

面板布局（`src/modules/parametric/mod.rs`）：`LargeTool(Delete Constraints)`、`LargeTool(Parameters Manager)`。

### 删除约束（Delete Constraints）
- **功能简介**：删除选中对象所涉及的全部几何/尺寸约束（含其动态尺寸与专属参数）。
- **UI 入口**：`Ribbon → Parametric 选项卡 → Manage 面板 → Delete Constraints（LargeTool）`。
- **样式**：`LargeTool`（图标 `assets/icons/constrain/delete.svg`，标签 “Delete Constraints”）。
- **触发命令**：`DELCONSTRAINT`。
- **实现位置**：`src/modules/parametric/mod.rs`（内联 `DELCONSTRAINT`）；分发 `src/app/commands/draw.rs::"DELCONSTRAINT"`。
- **备注**：无选择时经 `SelectObjectsCommand` 收集。按对象句柄过滤约束集，删除对应约束、其动态尺寸与该约束命名的参数（`purge_dimensional_extras`），记入撤销 “Delete constraints”，输出 “N constraint(s) deleted.”。无约束时输出 “No constraints found.”。

### 参数管理器（Parameters Manager）
- **功能简介**：打开「Named Parameters」窗口，以表格编辑具名参数（名称+公式），显示解析值与“被谁使用”。
- **UI 入口**：`Ribbon → Parametric 选项卡 → Manage 面板 → Parameters Manager（LargeTool）`，打开模态窗口。
- **样式**：`LargeTool`（图标 `assets/icons/constrain/parameters.svg`，标签 “Parameters Manager”）。
- **触发命令**：`PARAMETERS`（命令行版为 `-PARAMETERS`）。
- **实现位置**：`src/modules/parametric/mod.rs`（内联 `PARAMETERS`）；映射 `src/app/commands/view.rs::"PARAMETERS" → Message::NamedParametersOpen`；窗口 `src/ui/window/named_parameters.rs::view_window`；更新 `src/app/update/mod.rs::NamedParameters*`。
- **备注**：见下文「四、Named Parameters 窗口」逐项。

---

## 四、Named Parameters 窗口（参数管理器窗口）

实现位置：`src/ui/window/named_parameters.rs`；打开 `src/app/update/mod.rs::Message::NamedParametersOpen`；应用 `OpenCADStudio::apply_named_parameter_editor_rows`。

### 说明文字与标题
- **功能简介**：窗口标题 “Named Parameters”，并提示输入方式与保存语义。
- **UI 入口**：`Ribbon → Parametric 选项卡 → Manage 面板 → Parameters Manager`。
- **样式**：标题字号 15；提示 `text` 字号 11（muted）。
- **实现位置**：`src/ui/window/named_parameters.rs::view_window`（`title`、`hint`）。
- **备注**：提示原文 “Type a name and a formula (e.g. hole_dia = 12, hole_spacing = 2 * hole_dia + 1.5). Apply to save and re-solve; closing discards unapplied edits.”。

### 参数表列（Name / Formula / Value / Used by）
- **功能简介**：每行编辑参数名称与公式，并显示解析值与被引用情况。
- **UI 入口**：窗口内表格 4 列 + 删除列。
- **样式**：表头字号 11（muted）；Name 列宽 `NAME_WIDTH=120`，Formula 列宽随窗口（`sizing.width`），Value 列宽 `VALUE_WIDTH=100`，Used by 列宽 `USED_BY_WIDTH=160`，右侧留 `GUTTER=16` 给滚动条；每行列间距 3。
- **实现位置**：`src/ui/window/named_parameters.rs::view_window`（`head`、列表循环）、`ParamEditorRow`、`ParamField`。
- **备注**：Name/Formula 为 `text_input`（字号 13），编辑发 `Message::NamedParametersInput { idx, field, value }`。

### 值预览（Value 列）
- **功能简介**：每行实时解析并在 Value 列显示 `= <值>` 或错误原因；跨行引用与循环在 Apply 前即可见。
- **UI 入口**：窗口内 Value 列。
- **样式**：成功值字号 12（muted），格式 `= {value:.4}`；错误字号 11（danger 色）。
- **实现位置**：`src/ui/window/named_parameters.rs::preview`、`duplicate_name_rows`。
- **备注**：空名行不报错（`None`）；同名行报 “duplicate name '...'”；`ParameterTable::set` 错误、未定义引用、循环引用逐行显示。预览基于整段缓冲区在每次渲染重算，行序不影响前向引用。

### 被使用列（Used by 列）
- **功能简介**：显示该参数驱动的约束种类计数摘要，悬停显示完整约束/实体清单。
- **UI 入口**：窗口内 Used by 列。
- **样式**：摘要字号 11（muted），形如 “Distance ×2, Radius ×1”；无使用时显示 muted 破折号 “—”；悬停为 `tooltip`（列检视框、padding `[4,8]`、位置 Top、`bordered_box`）。
- **实现位置**：`src/ui/window/named_parameters.rs::used_by_cell`、`usage_lines`、`entity_label`；数据源 `Scene::parameter_usage`。
- **备注**：实体标签如 “Line 0x2A”“Circle …”，悬空句柄显示 “(erased …)”。

### 删除行（✕ 按钮）
- **功能简介**：移除当前缓冲区中的该参数行（Apply 后从表中删除）。
- **UI 入口**：窗口内每行末列 ✕。
- **样式**：`button::danger`，图标 `icons::CLOSE`（尺寸 12），padding `[2,6]`。
- **触发命令**：`Message::NamedParametersRemove(idx)`。
- **实现位置**：`src/ui/window/named_parameters.rs::view_window`（`del`）；更新 `src/app/update/mod.rs`。

### 新增参数（+ Add parameter）
- **功能简介**：向缓冲区追加一个空白参数行。
- **UI 入口**：窗口底部左侧 “+ Add parameter”。
- **样式**：`button::secondary`，字号 12，padding `[4,10]`。
- **触发命令**：`Message::NamedParametersAdd`。
- **实现位置**：`src/ui/window/named_parameters.rs::view_window`（`add`）。

### 应用（Apply）
- **功能简介**：把缓冲区整表提交到 `Scene::named_parameters` 并重新求解；关闭窗口则丢弃未应用编辑。
- **UI 入口**：窗口底部右侧 “Apply”。
- **样式**：`button::primary`，字号 12，padding `[4,16]`。
- **触发命令**：`Message::NamedParametersApply`。
- **实现位置**：`src/ui/window/named_parameters.rs::view_window`（`apply`）；`OpenCADStudio::apply_named_parameter_editor_rows`。
- **备注**：同名冲突时整组拒绝（`duplicate_name_rows`），与预览规则一致；`PARAMETERS` 打开的是同一窗口的内容形态（命令行版 `-PARAMETERS` 在命令窗口内操作）。

---

## 五、Auto Constrain 设置窗口（自动约束设置）

实现位置：`src/ui/window/auto_constrain_settings.rs`；打开命令 `CONSTRAINTSETTINGS`（`src/app/commands/draw.rs:1180`，设 `ModalKind::AutoConstrainSettings`）；更新 `src/app/update/mod.rs::AutoConstrain*`；设置结构 `src/app/settings.rs::AutoConstrainSettings`。

### 约束类型清单（Priority / Constraint Type / Apply）
- **功能简介**：按优先级列出 9 种可推断约束类型，每行可勾选是否启用，点击行选中以便上移/下移。
- **UI 入口**：`Auto Constrain 设置窗口 → 左栏列表`（点击行选中）。
- **样式**：表头 `Priority`（宽 70）/`Constraint Type`（Fill）/`Apply`（宽 60）；每行按钮选中时 `button::primary`，未选中 `button::text`，padding `[5,8]`；列表可滚动（高 260）。
- **触发命令**：行选择 `Message::AutoConstrainSelectRow(index)`；复选框 `Message::AutoConstrainToggleKind(kind)`。
- **实现位置**：`src/ui/window/auto_constrain_settings.rs::view_window`（`list`）；`src/app/settings.rs::AutoConstraintKind`（9 种：Coincident、Collinear、Parallel、Perpendicular、Tangent、Concentric、Horizontal、Vertical、Equal；`label()`）。
- **备注**：默认启用除 Equal 外的全部类型（避免产生冗余关系，`AutoConstrainSettings::default`）。

### 顺序调整与批量操作（Move Up / Move Down / Select All / Clear All / Reset）
- **功能简介**：调整推断优先级，或一键全选/清空/恢复默认。
- **UI 入口**：`Auto Constrain 设置窗口 → 右栏按钮组`。
- **样式**：右侧 `column` 宽 125，间距 6；Move Up 在选中行 > 0 时可用，Move Down 在非末行时可用（`on_press_maybe`）。
- **触发命令**：`Message::AutoConstrainMoveUp` / `MoveDown` / `SelectAll` / `ClearAll` / `Reset`。
- **实现位置**：`src/ui/window/auto_constrain_settings.rs::view_window`（`reorder`）；更新逻辑 `src/app/update/mod.rs`。
- **备注**：Reset 恢复 `AutoConstrainSettings::default()` 并重置选中行与容差输入框。

### 相切/垂直附加条件（复选框）
- **功能简介**：控制推断相切、垂直约束时是否要求对象确实共享交点。
- **UI 入口**：`Auto Constrain 设置窗口 → 类型列表下方复选框`。
- **样式**：`checkbox` + 标签（字号默认）。
- **触发命令**：`Message::AutoConstrainToggleTangentPoint`、`Message::AutoConstrainTogglePerpendicularIntersection`。
- **实现位置**：`src/ui/window/auto_constrain_settings.rs::view_window`（`conditions`）；字段 `tangent_must_share_point`、`perpendicular_must_intersect`（默认均为 true）。
- **备注**：标签原文 “Tangent objects must share an intersection point” 与 “Perpendicular objects must share an intersection point”。

### 容差（Tolerances：Distance / Angle）
- **功能简介**：设定推断距离容差与角度容差（度）。
- **UI 入口**：`Auto Constrain 设置窗口 → Tolerances 区`。
- **样式**：标题 “Tolerances” 字号 14；`Distance`/`Angle` 标签宽 130，输入框宽 150（角度带 “°”）。
- **触发命令**：`Message::AutoConstrainDistanceChanged`、`Message::AutoConstrainAngleChanged`。
- **实现位置**：`src/ui/window/auto_constrain_settings.rs::view_window`（`tolerances`）；默认距离 0.05、角度 1.0（`AutoConstrainSettings::default`）。
- **备注**：OK/Apply 时校验为非负有限数，否则报 “Distance and angle tolerances must be non-negative numbers.”；通过后 `sanitize()` 并持久化。

### 动作按钮（OK / Apply / Cancel）
- **功能简介**：确认保存、保存后继续留在窗口、或放弃本次修改并恢复打开前状态。
- **UI 入口**：`Auto Constrain 设置窗口 → 右下按钮组`。
- **样式**：`dialog_button`；OK 为主按钮（`true`），Apply/Cancel 为普通（`false`）。
- **触发命令**：`Message::AutoConstrainOk` / `AutoConstrainApply` / `AutoConstrainCancel`。
- **实现位置**：`src/ui/window/auto_constrain_settings.rs::view_window`（`actions`）；更新 `src/app/update/mod.rs`。
- **备注**：打开时 `auto_constrain_saved` 保存当前设置快照；OK/Apply 持久化设置；OK 关闭模态；Cancel 恢复快照并关闭（`draw.rs:1181`、`update/mod.rs:5758`）。

---

## 六、约束条（Constraint Bar）与视口约束图标

实现位置：绘制 `src/ui/overlay.rs`（`constraint_glyphs`、`draw_*_constraint_glyph` 等）、放置/命中 `src/scene/parametric_constraints.rs::constraint_glyph_placements_screen`/`constraint_glyph_hit`；视口逻辑 `src/app/update/viewport.rs`；显示开关 `src/app/settings.rs`（`constraint_bar_display`、`constraint_bar_mode`、`show_constraint_values`）。

### 约束图标/约束条绘制
- **功能简介**：在几何附近以胶囊（pill）绘制几何约束符号与尺寸值/参数名；冗余或冲突约束以危险色显示。
- **UI 入口**：`视口 → 几何对象附近（选择或应用约束后出现）`。
- **样式**：普通胶囊底色 `Color::from_rgb8(103,109,118)`、前景白；重合约束用蓝 `from_rgb8(35,145,230)` 且圆角更小（compact）；冲突/冗余用主题 `danger` 底色与文字；选中约束以主题 `primary.strong` 描边（宽 2.0）；固定约束对象模式锁定为红 `from_rgb8(214,76,76)`，点模式为白；Equal 符号 “=” 为绿 `from_rgb8(72,199,116)`。图标字号 `GLYPH_SIZE = 14.0`（`scene/parametric_constraints.rs`），胶囊尺寸由 `constraint_glyph_size` 依标签长度计算。
- **触发命令**：随约束应用/选择自动显示；显示规则由 `CONSTRAINTBARDISPLAY`/`CONSTRAINTBARMODE` 与 `show_constraint_values` 控制。
- **实现位置**：`src/ui/overlay.rs::constraint_glyphs` 分支、`draw_tangent_constraint_glyph`、`draw_smooth_constraint_glyph`、`draw_concentric_constraint_glyph`、`draw_fixed_constraint_glyph`、`draw_vertical_constraint_glyph`；标签生成 `src/scene/parametric_constraints.rs::constraint_glyph_placements_screen`（`glyph_label`、`fixed_glyph_label`、`vertical_glyph_label`、`DYNAMIC_DIMENSION_GLYPH`）。
- **备注**：动态尺寸图标（`DYNAMIC_DIMENSION_GLYPH`）不填充胶囊底色，直接以锁形绘制（`is_dynamic_dimension_glyph`）。对称约束按引用显示 “S│”“S•”“S◇”（`constraint_glyph_placements_screen`）。

### 悬停提示（tooltip）与悬停标记
- **功能简介**：鼠标停在约束图标上显示该约束种类/表达式的提示，并在相关几何点上绘制红色叉形标记。
- **UI 入口**：`视口 → 悬停约束图标`。
- **样式**：提示框背景 `background.strong`，边框 `background.strong.text` 宽 1.0，圆角 4；文字居中字号 12；红色（`Color::from_rgb(1,0,0)`）宽 1.5 的叉线，标记半径 `CONSTRAINT_HOVER_MARKER_RADIUS`。
- **触发命令**：悬停（可能需 dwell 延迟，见 `viewport.rs::constraint_glyph_tooltip_appears_after_hover_dwell`）。
- **实现位置**：`src/ui/overlay.rs::constraint_glyphs` 悬停分支（`constraint_glyph_tooltip`）；命中判定 `src/app/update/viewport.rs::constraint_glyph_under`、`constraint_glyph_hit`。
- **备注**：仅在 `current_layout == "Model"` 且 `ShowConstraintValues` 相应设置下计算。

### 点击约束图标选择
- **功能简介**：在视口中点击约束图标，将参与该约束的实体整体选中并提示约束被选中；随后可删除或进行约束条操作。
- **UI 入口**：`视口 → 点击约束图标`。
- **样式**：无独立控件，行为式。
- **触发命令**：左键点击（空闲状态且无活动命令）。
- **实现位置**：`src/app/update/viewport.rs::on_viewport_left_press`（`constraint_glyph_under` → `select_entities`，输出 “{kind:?} constraint selected.”）。
- **备注**：命中图标后清除框选/套索手势，并刷新夹点与特性面板。

### 右键删除约束（上下文菜单）
- **功能简介**：选中某约束后，右键菜单给出最小编辑块，可直接删除该约束。
- **UI 入口**：`视口 → 悬停/选中约束后右键 → Delete`。
- **样式**：上下文菜单项（菜单行）。
- **触发命令**：`MenuAction::ConstraintDelete(id)`。
- **实现位置**：`src/ui/popup/context_menu.rs::MenuItem`/`MenuAction::ConstraintDelete`（`selected_constraint` 分支，`context_menu.rs:669-676`）。
- **备注**：该分支不显示实体相关操作，只有 Delete 与分隔符。

### 显示开关（Options）
- **功能简介**：控制约束标记是否显示驱动的值/参数名（关闭则只显示约束符号）。
- **UI 入口**：`菜单栏 → Options → 复选框 “Show values and parameter names on constraint markers”`。
- **样式**：`checkbox` 尺寸 15 + 说明文字（字号 12）。
- **触发命令**：`Message::ShowConstraintValuesChanged(bool)`。
- **实现位置**：`src/ui/window/options.rs::"Show values and parameter names on constraint markers"`（`options.rs:448`）；更新 `src/app/update/mod.rs::ShowConstraintValuesChanged`。
- **备注**：默认开（`default_show_constraint_values` → true）。另有 Properties 面板近旁的同名快速开关（`src/ui/properties.rs::render_params_visibility_toggle_row`，On/Off 按钮）。

### 约束条显示/模式系统变量
- **功能简介**：通过系统变量控制约束条的显示时机（应用后/选中时）与显示的约束类型位掩码。
- **UI 入口**：`命令输入框 → SETVAR`（或 `CONSTRAINTBARDISPLAY` / `CONSTRAINTBARMODE`）。
- **样式**：无独立控件，命令行读写。
- **触发命令**：`CONSTRAINTBARDISPLAY`（默认 3）、`CONSTRAINTBARMODE`（默认 4095，全类型）、`CONSTRAINTSOLVEMODE`、`CONSTRAINTINFER`、`CONSTRAINTNAMEFORMAT`、`DYNCONSTRAINTDISPLAY`。
- **实现位置**：`src/app/commands/styleprops.rs`（`styleprops.rs:1454-1486` 的读取/设置）；字段 `src/app/settings.rs`（`constraint_bar_display`、`constraint_bar_mode`、`constraint_solve_mode`、`constraint_infer`）。
- **备注**：`constraint_bar_display` 位含义 “1 after applying, 2 on selection”（`settings.rs:482`）；`constraint_bar_mode` 为几何约束类型位掩码（默认全部）。加载时夹取到 `0..=3` 与 `0..=4095`（`src/app/update/file.rs:821`）。

### 约束条选项命令（CONSTRAINTBAR）
- **功能简介**：对已选（或现场收集）对象执行约束条显示/隐藏/重置选项。
- **UI 入口**：`命令输入框 → CONSTRAINTBAR`（内部也用于约束条交互）。
- **样式**：命令行提示与关键字。
- **触发命令**：`CONSTRAINTBAR`（应用阶段 `CONSTRAINTBAR_OPTIONS`、`CONSTRAINTBAR_RESET`）。
- **实现位置**：`src/modules/parametric/constraint_bar.rs::ConstraintBarOptionCommand`；分发 `src/app/commands/draw.rs::"CONSTRAINTBAR" | "CONSTRAINTBAR_OPTIONS"`、`"CONSTRAINTBAR_RESET"`。
- **备注**：提示 `CONSTRAINTBAR Enter an option [Show/Hide/Reset] <Show>:`，选项 `Show`(`S`)、`Hide`(`H`)、`Reset`(`R`)；Show→`GCSHOW`、Hide→`GCHIDE`、Reset→`CONSTRAINTBAR_RESET`（`Relaunch` 保持选择集）。无选择时先进入对象收集（`SelectObjectsCommand::routed`）。

---

## 七、命令行与其它入口（未在固定面板出现或作为补充）

### 几何约束总控（GEOMCONSTRAINT）
- **功能简介**：命令行方式选择几何约束类型，转派到对应专用命令（与 Ribbon 同一实现）。
- **UI 入口**：`命令输入框 → GEOMCONSTRAINT`。
- **样式**：命令行提示与关键字下拉。
- **触发命令**：`GEOMCONSTRAINT`。
- **实现位置**：`src/modules/parametric/geom_constraint.rs::GeomConstraintCommand`。
- **备注**：选项 `Horizontal`(`H`)/`Vertical`(`V`)/`Perpendicular`(`P`)/`Parallel`(`PA`)/`Tangent`(`T`)/`Smooth`(`SM`)/`Coincident`(`C`)/`Concentric`(`CON`)/`Collinear`(`COL`)/`Symmetric`(`SY`)/`Equal`(`E`)/`Fix`(`F`)，Enter 默认 Coincident。`dispatch` 将关键字映射到 `GCHORIZONTAL`/`VCONSTRAINT`/`GCPERPENDICULAR`/`PCONSTRAINT`/`TCONSTRAINT`/`GCSMOOTH`/`GCCOINCIDENT`/`GCCONCENTRIC`/`LCONSTRAINT`/`SYCONSTRAINT`/`ECONSTRAINT`/`FXCONSTRAINT`。

### 尺寸约束总控（DIMCONSTRAINT 与 DCFORM）
- **功能简介**：命令行选择尺寸约束类型；DCFORM 切换约束形式（Annotational/Dynamic）。
- **UI 入口**：`命令输入框 → DIMCONSTRAINT`、`命令输入框 → DCFORM`。
- **样式**：命令行提示与关键字下拉。
- **触发命令**：`DIMCONSTRAINT`、`DCFORM`（应用 `DCFORM_SET <form>`）。
- **实现位置**：`src/modules/parametric/dim_constraint.rs::DimConstraintMenuCommand`、`ConstraintFormCommand`；分发 `src/app/commands/draw.rs::"DIMCONSTRAINT"`、`"DCFORM"`、`cmd.starts_with("DCFORM_SET ")`。
- **备注**：DIMCONSTRAINT 选项 `Linear`(`L`)/`Horizontal`(`H`)/`Vertical`(`V`)/`Aligned`(`A`)/`ANgular`(`AN`)/`Radius`(`R`)/`Diameter`(`D`)/`Form`(`F`)/`Convert`(`C`)，Enter 取上次使用的类型（`self.dim_constraint_last`）。DCFORM 影响后续转换/新建尺寸的显示形态（`constraint_form_annotational`）。

### 距离/角度数值约束（DCONSTRAINT / ACONSTRAINT）
- **功能简介**：对单个直线/圆/弧键入长度或半径/直径/角度的驱动值或参数名。
- **UI 入口**：`命令输入框 → DCONSTRAINT`、`ACONSTRAINT`（也用于 `DCLINEAR`/`DCHORIZONTAL`/`DCVERTICAL`/`DCALIGNED` 的单对象快捷路径）。
- **样式**：命令行提示，`InputKind::SingleToken`。
- **触发命令**：`DCONSTRAINT`、`ACONSTRAINT`。
- **实现位置**：`src/modules/parametric/value.rs::DistanceConstraintCommand`、`DistanceMode`；分发 `src/app/commands/draw.rs::"DCONSTRAINT"`、`"ACONSTRAINT" | "DCANGULAR"`。
- **备注**：提示如 `Specify distance <…> or [Xdistance/Ydistance]:`，圆/弧为 `[Diameter]`，直径模式为 `[Radius]`；键入已有参数名优先于同拼写的模式关键字（`parse_driving_value`）；分量距离（X/Y）接受带符号值，普通距离须为正有限数。

### 尺寸值编辑命令（DCVALUE）
- **功能简介**：双击动态尺寸后弹出的值编辑入口，替代参考实现的就地编辑器；支持 `name=expression` 改名。
- **UI 入口**：`视口 → 双击动态尺寸`（命令行 `DCVALUE <name> <value>`）。
- **样式**：命令行提示 `DCVALUE Enter value or name and value <name=current>:`。
- **触发命令**：`DCVALUE`（应用 `cmd.starts_with("DCVALUE ")`）。
- **实现位置**：`src/modules/parametric/dim_constraint.rs::DimensionValueCommand`；分发 `src/app/commands/draw.rs::"DCVALUE …"`（`apply_parameter_input`）。
- **备注**：输入非空即转派 `DCVALUE <name> <text>`。

### 参数管理器命令行版（-PARAMETERS）
- **功能简介**：在命令行内对具名参数执行 LIST/NEW/EDIT/RENAME/DELETE。
- **UI 入口**：`命令输入框 → -PARAMETERS`。
- **样式**：命令行提示与选项下拉。
- **触发命令**：`-PARAMETERS`（子操作 `-PARAMETERS LIST/NEW/EDIT/SET/RENAME/DELETE`）。
- **实现位置**：`src/modules/parametric/parameters_cli.rs::ParametersCliCommand`；分发 `src/app/commands/draw.rs::"-PARAMETERS"`、`cmd.starts_with("-PARAMETERS ")`。
- **备注**：提示 `-PARAMETERS Enter a parameter option [New/Edit/Rename/Delete/?]:`，选项 `New`(`N`)/`Edit`(`E`)/`Rename`(`R`)/`Delete`(`D`)/`?`；`?` 直接 LIST。LIST 以分割线输出 `Parameter / Expression / Value`；EDIT 先打印旧表达式再进入表达式步（`ParametersCliCommand::edit_expression` → `SET`）。

### 相等距离约束（EDCONSTRAINT）
- **功能简介**：使两对点之间的距离相等（区别于 Equal 比较整对象）。
- **UI 入口**：`命令输入框 → EDCONSTRAINT`（无固定 Ribbon 按钮）。
- **样式**：命令行提示（顺序拾取四个点）。
- **触发命令**：`EDCONSTRAINT`。
- **实现位置**：`src/modules/parametric/equal_distance.rs::EqualDistanceConstraintCommand`（工具 `equal_distance_tool` id `EDCONSTRAINT`）；分发 `src/app/commands/draw.rs::"EDCONSTRAINT"`。
- **备注**：提示依次 `Specify first point of first pair:` → `second point of first pair` → `first point of second pair` → `second point of second pair`；满四点生成 `AddEqualDistanceConstraint`。

### 中点/中心点/曲线上点约束（MIDPOINT / CENTERPOINT / ONCURVE）
- **功能简介**：把拾取的点约束到先前选中实体的中点、圆心或曲线本身上。
- **UI 入口**：`命令输入框 → MPCONSTRAINT` / `CPCONSTRAINT` / `OCCONSTRAINT`（工具 `midpoint_tool`/`center_point_tool`/`point_on_curve_tool` 定义在 `point_on_entity.rs`，但未出现在当前 Ribbon 面板）。
- **样式**：命令行提示 `Specify point:`。
- **触发命令**：`MPCONSTRAINT`（Midpoint）、`CPCONSTRAINT`（Center Point）、`OCCONSTRAINT`（Point on Curve）。
- **实现位置**：`src/modules/parametric/point_on_entity.rs::PointOnEntityConstraintCommand`、`midpoint_tool`/`center_point_tool`/`point_on_curve_tool`；分发 `src/app/commands/draw.rs::"CPCONSTRAINT" | "MPCONSTRAINT" | "OCCONSTRAINT"`。
- **备注**：需先选中恰好一个实体（未选时经 `SelectObjectsCommand` 收集），随后只拾取一个点。对应 `ConstraintKind::Midpoint/CenterPoint/PointOnCurve`。

### 法线约束（Normal / NRCONSTRAINT）
- **功能简介**：使直线垂直于圆/弧在其接触点的切线（在本项目的 Line/Circle 模型中即“直线过圆心”），区别于仅处理线-线的 Perpendicular。
- **UI 入口**：`命令输入框 → NRCONSTRAINT`（工具定义 `tools::normal`，未出现在当前 Ribbon 面板）。
- **样式**：命令行提示（与 Parallel/Colinear 共用分发）。
- **触发命令**：`NRCONSTRAINT`（工具 id；提示无固定 Ribbon 按钮）。
- **实现位置**：`src/modules/parametric/tools.rs::normal::tool()`；分发 `src/app/commands/draw.rs::"NRCONSTRAINT"`（类型 `ConstraintKind::Normal`）。
- **备注**：由 `PointOnLine` 基元构建（见 `tools.rs` 头注）；需恰好两个对象。

### 选择收集器（约束命令共用）
- **功能简介**：多数几何约束在无预选时进入 “Select objects:” 收集阶段，支持窗口/交叉/Fence 等。
- **UI 入口**：`命令行提示 + 视口拾取`。
- **样式**：与 Draw 模块一致的收集预览与提示。
- **触发命令**：约束命令内部（`SelectObjectsCommand::new/routed/auto_constrain/associative_dimensions`）。
- **实现位置**：`src/modules/draw/select.rs::SelectObjectsCommand`；各约束命令 `draw.rs` 分支中的收集调用。
- **备注**：`auto_constrain` 变体额外提供 `Settings`(`S`) 关键字打开设置；`associative_dimensions` 变体（DCCONVERT）仅保留关联尺寸。

### 关联尺寸/动态尺寸与参数（场景层）
- **功能简介**：约束驱动动态尺寸文字、参数命名、冲突/冗余检测等场景级支撑（非独立按钮）。
- **UI 入口**：无独立按钮；体现为视口动态尺寸、Named Parameters 窗口与参数管理器。
- **触发命令**：随约束命令与双击动态尺寸触发。
- **实现位置**：`src/scene/parametric_constraints.rs`（`ConstraintKind`、`ParametricRef`、`constraint_glyph_placements_screen`、`dynamic_dimension_constraint`、`measured_expression`、`next_dimensional_parameter_name`/`next_radial_parameter_name`/`next_angular_parameter_name`、`angle_decimals` 等）；`src/scene/dimension_assoc.rs::constraint_from_associative_dimension`。
- **备注**：动态尺寸位于 `DYNAMIC_DIMENSION_LAYER`，颜色 `rgb(103,109,118)`（`draw.rs:1474-1480`）。

---

（文档结束）
