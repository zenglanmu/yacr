# Ribbon「Model」选项卡功能需求清单

本文档反推自 `src/modules/model/`（`mod.rs`、`primitive_cmd.rs`、`cylinder_cmd.rs`、`boolean_cmd.rs`、`edge_cmd.rs`、`polysolid_cmd.rs`、`sectionplane_cmd.rs`、`shell_cmd.rs`、`slice_cmd.rs`）与相关实现 `src/modules/insert/solid3d_cmds.rs`（EXTRUDE/REVOLVE/LOFT/SWEEP/PRESSPULL）、`src/app/model_ops.rs`、`src/app/presspull_ops.rs`，以及命令分发 `src/app/commands/draw.rs`、`src/app/commands/dim.rs`。Ribbon 面板与工具的权威布局入口是 `src/modules/model/mod.rs::ModelModule::ribbon_groups()`；模块注册顺序见 `src/modules/registry.rs::all_modules()`（Draw、Parametric、**Model**、Insert、Annotate、View、Manage、Layout）。

面板组的权威顺序（`src/modules/model/mod.rs`）：Create、Boolean、Edges。三个面板均以 `LargeTool` / `LargeDropdown` 大按钮呈现（模型选项卡无扩展飞出面板代码）。

---

## 一、Create 面板（Ribbon → Model 选项卡 → Create 面板）

面板布局（`src/modules/model/mod.rs::ribbon_groups`）：`LargeDropdown(MODEL_PRIMITIVES)`、`LargeTool(EXTRUDE)`、`LargeTool(REVOLVE)`、`LargeTool(LOFT)`、`LargeTool(SWEEP)`、`LargeTool(PRESSPULL)`。

### 模型基本体下拉（Model Primitives）
- **功能简介**：一个大型下拉，集中创建全部 8 种 3D 基本实体（Box/Cylinder/Cone/Sphere/Pyramid/Wedge/Torus/Polysolid）。
- **UI 入口**：`Ribbon → Model 选项卡 → Create 面板 → 基本体（LargeDropdown，默认 BOX，标签 “Box”，图标 box.svg）`。
- **样式**：`LargeDropdown`（大按钮带 ▾；默认图标 `assets/icons/model/box.svg`，标签 “Box”）。
- **触发命令**：默认 `BOX`。下拉子项（`src/modules/model/mod.rs` 内联 items，全部 8 项）：
  - `BOX` — Box（长方体，图标 box.svg）
  - `CYLINDER` — Cylinder（圆柱/椭圆柱，图标 cylinder.svg）
  - `CONE` — Cone（圆锥/圆台，图标 cone.svg）
  - `SPHERE` — Sphere（球体，图标 sphere.svg）
  - `PYRAMID` — Pyramid（棱锥/棱台，图标 pyramid.svg）
  - `WEDGE` — Wedge（楔体，图标 wedge.svg）
  - `TORUS` — Torus（圆环体，图标 torus.svg）
  - `POLYSOLID` — Polysolid（多段体墙，图标 polysolid.svg）
- **实现位置**：`src/modules/model/mod.rs`（`MODEL_PRIMITIVES` 的 `items`/`default`）；命令分发 `src/app/commands/draw.rs` 的 `"CYLINDER"`（`CylinderCommand`）、`"BOX" | "WEDGE" | "CONE" | "SPHERE" | "PYRAMID" | "PYR" | "TORUS"`（`PrimitiveCommand::new(cmd)`）、`"POLYSOLID"`（`PolysolidCommand`）。
- **备注**：各基本体细节见下节。「Polysolid」在源码中实现于 `polysolid_cmd.rs`，虽列于 Create 面板下拉，实际命令名为 `POLYSOLID`。

### 长方体（Box）
- **功能简介**：以对角两点（或中心点）创建长方体，支持立方体、指定长宽、2 点定高。
- **UI 入口**：`Ribbon → Model 选项卡 → Create 面板 → 基本体下拉 → Box`。
- **样式**：`LargeDropdown` 子项（弹出列表行）。
- **触发命令**：`BOX`。
- **实现位置**：`src/modules/model/primitive_cmd.rs::PrimitiveCommand`（`Shape::Box`）、`Shape::from_id`；分发 `src/app/commands/draw.rs`。
- **备注**：交互（`box_step`）：FirstCorner → OppositeCorner；关键选项 Center(`C`)、Cube(`C`)、Length(`L`)、Height 步 2Point(`2P`)。高度无默认时需输入，Wedge 默认高度取 `default_height()`。

### 楔体（Wedge）
- **功能简介**：以对角两点创建楔体，高度沿 Z 方向（可 2 点定义）。
- **UI 入口**：`Ribbon → Model 选项卡 → Create 面板 → 基本体下拉 → Wedge`。
- **样式**：`LargeDropdown` 子项。
- **触发命令**：`WEDGE`。
- **实现位置**：`src/modules/model/primitive_cmd.rs`（`Shape::Wedge`，与 Box 共用 `box_step` 分支）。
- **备注**：与 Box 同构；在 OppositeCorner 命中带高度时会直接提交。Height 步显示 `<默认高度>`（`BoxStep::Height if Wedge`）。上次高度以 `LAST_WEDGE_HEIGHT` 原子变量记忆。

### 圆柱（Cylinder）
- **功能简介**：创建圆柱或椭圆柱，支持 3P/2P/Ttr/Elliptical 底面、直径、2Point/Axis endpoint 定高。
- **UI 入口**：`Ribbon → Model 选项卡 → Create 面板 → 基本体下拉 → Cylinder`。
- **样式**：`LargeDropdown` 子项。
- **触发命令**：`CYLINDER`。
- **实现位置**：`src/modules/model/cylinder_cmd.rs::CylinderCommand`；分发 `src/app/commands/draw.rs::"CYLINDER"`。
- **备注**：交互选项（`cylinder_cmd.rs::prompt/options`）：
  - 底面中心步：`3P`、`2P`、`Ttr`、`Elliptical`。
  - 半径步：`Diameter`。
  - 椭圆第一步：`Center`。
  - 高度步：`2Point`、`Axis endpoint`。
  - 默认（`Defaults`）主半径/次半径/高各 1.0，跨会话记忆；动态输入 `DynRole::Radius/Diameter/Height`。

### 圆锥/圆台（Cone）
- **功能简介**：创建圆锥或圆台（可设 Top radius），支持 3P/2P/Ttr/Elliptical 底面、直径、2Point/Axis endpoint 定高。
- **UI 入口**：`Ribbon → Model 选项卡 → Create 面板 → 基本体下拉 → Cone`。
- **样式**：`LargeDropdown` 子项。
- **触发命令**：`CONE`。
- **实现位置**：`src/modules/model/primitive_cmd.rs`（`Shape::Cone`，`cone_prompt`/`cone_options`/`on_cone_point`/`on_cone_text`）。
- **备注**：选项：底面中心 `3P`/`2P`/`Ttr`/`Elliptical`，半径 `Diameter`，椭圆第一步 `Center`，高度步 `2Point`/`Axis endpoint`/`Top radius`（设置顶半径后选项收敛为 `2Point`/`Axis endpoint`）。默认 `ConeDefaults` 底 X/Y 半径 1.0、顶半径 0.0、高 1.0。

### 球体（Sphere）
- **功能简介**：以中心+半径（或直径）创建球体，支持 3P/2P/Ttr。
- **UI 入口**：`Ribbon → Model 选项卡 → Create 面板 → 基本体下拉 → Sphere`。
- **样式**：`LargeDropdown` 子项。
- **触发命令**：`SPHERE`。
- **实现位置**：`src/modules/model/primitive_cmd.rs`（`Shape::Sphere`、`SphereStep`、`sphere_through_three_points`）。
- **备注**：中心步选项 `3P`/`2P`/`Ttr`；半径步 `Diameter`。无高度步。默认半径记忆于 `sphere_radius_store`（初值 1.0）。

### 棱锥/棱台（Pyramid）
- **功能简介**：创建正棱锥或棱台，支持指定边数、按边定义、内接/外切、顶半径、2Point/Axis endpoint 定高。
- **UI 入口**：`Ribbon → Model 选项卡 → Create 面板 → 基本体下拉 → Pyramid`。
- **样式**：`LargeDropdown` 子项。
- **触发命令**：`PYRAMID`（别名 `PYR`）。
- **实现位置**：`src/modules/model/primitive_cmd.rs`（`Shape::Pyramid`、`PyramidType`、`pyramid_prompt`/`pyramid_options`/`on_pyramid_point`/`on_pyramid_text`）。
- **备注**：选项：底面中心步 `Edge`(`E`)/`Sides`(`S`)；底半径步按当前类型切换 `Inscribed`(`I`)/`Circumscribed`(`C`)；高度步 `2Point`/`Axis endpoint`/`Top radius`。默认边数 4、底半径 1.0、顶半径 0.0、高 1.0，类型 Circumscribed（`PyramidDefaults`）。

### 圆环体（Torus）
- **功能简介**：创建圆环体，支持 3P/2P/Ttr 定主半径、主半径/直径、管半径（2Point/Diameter）。
- **UI 入口**：`Ribbon → Model 选项卡 → Create 面板 → 基本体下拉 → Torus`。
- **样式**：`LargeDropdown` 子项。
- **触发命令**：`TORUS`。
- **实现位置**：`src/modules/model/primitive_cmd.rs`（`Shape::Torus`、`TorusStep`、`torus_defaults`）。
- **备注**：选项：中心步 `3P`/`2P`/`Ttr`；主半径步 `Diameter`；管半径步 `2Point`/`Diameter`。默认主半径 100.0、管半径 25.0（`TorusDefaults`）。

### 多段体（Polysolid）
- **功能简介**：沿直线/圆弧路径创建具有高度与宽度的 3D 墙体（多段实体），可由二维对象转换。
- **UI 入口**：`Ribbon → Model 选项卡 → Create 面板 → 基本体下拉 → Polysolid`（也可命令行 `POLYSOLID`）。
- **样式**：`LargeDropdown` 子项（图标 polysolid.svg）。
- **触发命令**：`POLYSOLID`。
- **实现位置**：`src/modules/model/polysolid_cmd.rs::PolysolidCommand`；分发 `src/app/commands/draw.rs::"POLYSOLID"`。
- **备注**：起始步选项 `Object`(`O`)/`Height`(`H`)/`Width`(`W`)/`Justify`(`J`)；后续直线步 `Arc`/`Close`(≥3 点)/`Undo`，圆弧步 `Close`(≥3 点)/`Direction`/`Line`/`Second point`/`Undo`；Justification Left/Center/Right。默认高 80.0、宽 5.0、Center（`polysolid_cmd.rs::defaults`，跨会话记忆）。可选的预选二维对象（直线/圆弧/圆/椭圆/多段线/样条）作为 Object 输入。

### 拉伸（Extrude）
- **功能简介**：将闭合或开放轮廓沿法向/方向/路径拉伸为实体或曲面，可加拔模角。
- **UI 入口**：`Ribbon → Model 选项卡 → Create 面板 → 拉伸（LargeTool，标签 “Extrude”，图标 extrude.svg）`。
- **样式**：`LargeTool`（大按钮，图标 + “Extrude”）。
- **触发命令**：`EXTRUDE`。
- **实现位置**：`src/modules/insert/solid3d_cmds.rs::ExtrudeCommand`（`ExtrudeStep`、`ExtrudeDefaults`）；分发 `src/app/commands/display.rs::"EXTRUDE"`（类型分发经 `src/app/commands/draw.rs`/`mod.rs`）。
- **备注**：交互选项（`solid3d_cmds.rs::prompt/options`）：
  - 轮廓步：`Mode`、Enter(Done)；有预选时直接进入高度步。
  - 模式步：`Solid`/`Surface`（跨会话记忆于 `extrude_defaults`）。
  - 高度步：`Direction`（两点定方向）、`Path`（选路径对象）、`Taper angle`（拔模角）、`Expression`（表达式）。
  - 路径步：`Taper angle`；拔模角步：`Expression`。
  - 高度记忆 `last_height`；预览用线框 `preview_body_wires`（`WireModel::SELECTED` 色）。

### 旋转（Revolve）
- **功能简介**：将轮廓绕指定轴旋转生成实体或曲面，可指定起始角、反向、角度表达式。
- **UI 入口**：`Ribbon → Model 选项卡 → Create 面板 → 旋转（LargeTool，标签 “Revolve”，图标 revolve.svg）`。
- **样式**：`LargeTool`（图标 + “Revolve”）。
- **触发命令**：`REVOLVE`。
- **实现位置**：`src/modules/insert/solid3d_cmds.rs::RevolveCommand`（`RevolveStep`、`RevolveDefaults`）；分发 `src/app/commands/display.rs::"REVOLVE"`。
- **备注**：交互选项：
  - 轮廓步：`Mode`、Enter(Done)；有预选（含有效轮廓）时进入轴起点步。
  - 模式步：`Solid`/`Surface`。
  - 轴起点步：`Object`（选线/射线/构造线）、`X`、`Y`、`Z`、`Mode`。
  - 角度步：`Start angle`、`Reverse`、`Expression`。
  - 默认模式 Solid、角度 360°（TAU）、起始角 0°。

### 放样（Loft）
- **功能简介**：按顺序在多个横截面/点端之间放样成实体或曲面，支持引导线、路径、闭合、法向、拔模角、连续性（G0/G1）、凸度。
- **UI 入口**：`Ribbon → Model 选项卡 → Create 面板 → 放样（LargeTool，标签 “Loft”，图标 loft.svg）`。
- **样式**：`LargeTool`（图标 + “Loft”）。
- **触发命令**：`LOFT`。
- **实现位置**：`src/modules/insert/solid3d_cmds.rs::LoftCommand`（`LoftStep`、`LoftState`、`LoftOptions`）；分发 `src/app/commands/display.rs::"LOFT"`（经 `LoftCommand::new`，初值模式 `ExtrudeMode::Solid`）。
- **备注**：交互选项（`LoftCommand::option_prompt/options`）：
  - 横截面步：`Point`（点端，仅首/末）、`Join`（连接边为一个截面）、`Mode`、Enter 完成；`Undo` 撤销。
  - 选项步：`Guides`、`Path`、`Cross sections only`(=`CROSSSECTIONS`)、`Settings`；若端点为点，追加 `Continuity`、`Bulge magnitude`。
  - Settings：`Normals`、`Draft angles`（仅法向为 Use draft angles 时）、`Magnitudes`（同前）、`Closed`（≥3 曲线截面且无点端）、`Align direction`、`Done`。
  - Normals（7 项）：`Ruled`、`Smooth`、`First normal`、`Last normal`、`Ends normal`、`All normal`、`Use draft angles`（也可直接输 0~6）。
  - 连续性：`G0`/`G1`；闭合/对齐方向：`Yes`/`No`；拔模角 0~180°、幅度/凸度非负。
  - 至少需 2 个截面；闭合需 ≥3 曲线截面且无点端；点截面只能位于首尾且两端之间须有曲线（校验提示见源码 notice）。选择/引导/路径候选中不能与截面重复。预览为线框 `WireModel::SELECTED`。

### 扫掠（Sweep）
- **功能简介**：将轮廓沿路径扫掠为实体或曲面，支持对齐（align）、基点、缩放（含 Reference/Points）、扭转（含 Bank）。
- **UI 入口**：`Ribbon → Model 选项卡 → Create 面板 → 扫掠（LargeTool，标签 “Sweep”，图标 sweep.svg）`。
- **样式**：`LargeTool`（图标 + “Sweep”）。
- **触发命令**：`SWEEP`。
- **实现位置**：`src/modules/insert/solid3d_cmds.rs::SweepCommand`（`SweepStep`、`SweepOptions`）；分发 `src/app/commands/display.rs::"SWEEP"`。
- **备注**：交互选项：
  - 轮廓步：`Mode`、Enter(Done)；有预选轮廓时进入路径步。
  - 模式步：`Solid`/`Surface`。
  - 路径步：`Alignment`、`Base point`、`Scale`、`Twist`。
  - 对齐步：`Yes`/`No`（是否垂直于路径）；缩放步：`Reference`（参考长度 + 新长度/Points）；新长度步：`Points`；扭转步：`Bank`。
  - 路径步中悬停高亮路径对象（`entity_pick_highlights_hover`），选中有效路径即提交。

### 按住并拖动（Presspull）
- **功能简介**：选择对象或有界区域，拖拽/输入有符号高度进行拉伸或按面偏移（Offset）编辑实体。
- **UI 入口**：`Ribbon → Model 选项卡 → Create 面板 → 按住并拖动（LargeTool，标签 “Presspull”，图标 presspull.svg）`。
- **样式**：`LargeTool`（图标 + “Presspull”）。
- **触发命令**：`PRESSPULL`。
- **实现位置**：`src/modules/insert/solid3d_cmds.rs::PresspullCommand`（`PresspullStep`、`PresspullTarget`）；场景集成 `src/app/presspull_ops.rs::presspull_pick/presspull_apply/prepare_presspull`；分发 `src/app/commands/display.rs::"PRESSPULL"`。
- **备注**：交互选项：
  - 选择步：Enter(Done)。
  - 多选步（Multiple）：`Undo`、Enter(Height)。
  - 高度步：`Multiple`、`Undo`。
  - Ctrl 修饰 = Offset 模式，Shift 修饰 = 多选（`presspull_ops.rs::presspull_pick`）。支持由轮廓拉伸（闭合→Solid3D，开放→Surface）或对面做 Extrude/Offset（`presspull_apply`）。不删除源曲线，保留可编辑构造参数（`record_history=false`）。错误如「enter a finite non-zero height」经命令行报错。

---

## 二、Boolean 面板（Ribbon → Model 选项卡 → Boolean 面板）

面板布局（`src/modules/model/mod.rs`）：`LargeTool(UNION)`、`LargeTool(SUBTRACT)`、`LargeTool(INTERSECT)`。

### 并集（Union）
- **功能简介**：将选中的多个实体/面域/共面曲面合并为一个实体。
- **UI 入口**：`Ribbon → Model 选项卡 → Boolean 面板 → 并集（LargeTool，标签 “Union”，图标 union.svg）`。
- **样式**：`LargeTool`（图标 + “Union”）。
- **触发命令**：`UNION`。
- **实现位置**：`src/app/model_ops.rs::solid_boolean`（`BoolOp::Union` → `union_selected_entities`）、`union_ready`、`selected_union_groups`；命令入口 `src/app/commands/draw.rs::"UNION"` / `"UNIONAPPLY"`；`BoolOp` 定义于 `src/modules/model/boolean_cmd.rs`。
- **备注**：需选中 ≥2 个同类型（Solid/Region/共面 Surface）对象；未就绪时进入 `SelectObjectsCommand("UNION","UNIONAPPLY")` 收集选择，再执行。按类型/共面分组各自合并，结果为单一 Solid3D，源对象被擦除。

### 差集（Subtract）
- **功能简介**：从一组基体实体中减去一组切割体。
- **UI 入口**：`Ribbon → Model 选项卡 → Boolean 面板 → 差集（LargeTool，标签 “Subtract”，图标 subtract.svg）`。
- **样式**：`LargeTool`（图标 + “Subtract”）。
- **触发命令**：`SUBTRACT`。
- **实现位置**：`src/modules/model/boolean_cmd.rs::SubtractCommand`（`SubtractStep`）；场景执行 `src/app/model_ops.rs::solid_subtract`；分发 `src/app/commands/draw.rs::"SUBTRACT"`。
- **备注**：选择顺序为两阶段：先选**基体**（Bases，Enter 确认），再选**切割体**（Cutters，Enter 确认）。提示：`Select base Solids, Regions, Surfaces, or Meshes…` / `Select …to subtract…`。若基体或切割体含 Mesh（Mesh/PolygonMesh/PolyfaceMesh），追加一步 `Convert selected closed Mesh objects to solids? [Yes/No] <Yes>`（`Y`/`N`，Enter 视为 Yes）。可接受类型：Solid3D/Region/Surface/Mesh/PolygonMesh/PolyfaceMesh。

### 交集（Intersect）
- **功能简介**：取选中多个实体的公共重叠部分生成新实体。
- **UI 入口**：`Ribbon → Model 选项卡 → Boolean 面板 → 交集（LargeTool，标签 “Intersect”，图标 intersect.svg）`。
- **样式**：`LargeTool`（图标 + “Intersect”）。
- **触发命令**：`INTERSECT`。
- **实现位置**：`src/app/model_ops.rs::solid_intersect`、`intersect_ready`、`intersect_groups`；分发 `src/app/commands/draw.rs::"INTERSECT"`（未就绪时以 `SelectObjectsCommand` 收集）；`BoolOp::Intersect` 于 `src/modules/model/boolean_cmd.rs`。
- **备注**：需先选中足够对象，否则进入选择收集。布尔公用执行 `solid_boolean`（`BoolOp` → `kernel::Bool`）。

---

## 三、Edges 面板（Ribbon → Model 选项卡 → Edges 面板）

面板布局（`src/modules/model/mod.rs`）：`LargeTool(FILLETEDGE)`、`LargeTool(CHAMFEREDGE)`、`LargeTool(SHELL)`（注意 SHELL 复用 `PRESSPULL_ICON`）。

### 边圆角（Fillet Edge）
- **功能简介**：对 3D 实体的边（单边/链/环）创建圆角，输入半径，实时预览。
- **UI 入口**：`Ribbon → Model 选项卡 → Edges 面板 → 边圆角（LargeTool，标签 “Fillet Edge”，图标 fillet.svg）`。
- **样式**：`LargeTool`（图标 + “Fillet Edge”）。
- **触发命令**：`FILLETEDGE`（别名 `SOLIDFILLET`；对已选实体边时 `FILLET` 也路由到此）。
- **实现位置**：`src/modules/model/edge_cmd.rs::SolidEdgeCommand`（`EdgeOperation::Fillet`、`EdgeStep`）；分发 `src/app/commands/dim.rs::"FILLETEDGE" | "SOLIDFILLET"`；几何执行 `src/app/model_ops.rs::solid_edge_blend`。
- **备注**：交互选项：
  - 选择步：`Chain`(`C`)、`Loop`(`L`)、`Radius`(`R`)（链模式下切换 `Edge`/`Radius`）。
  - 环步：`Edge`/`Chain`/`Radius`；环确认步：`Accept`(`A`)/`Next`(`N`)。
  - 预览确认：Enter 接受，或 `Radius`(`R`) 修改。
  - 半径步：`Expression`(`E`) 输入表达式；默认半径跨会话记忆于 `FILLET_RADIUS_BITS`（初值 1.0）。
  - 拾取需包含填充面、使用曲面点、悬停高亮（`entity_pick_includes_fills`/`uses_surface_point`/`highlights_hover`）。

### 边倒角（Chamfer Edge）
- **功能简介**：对 3D 实体的边（环）创建等距/两距离倒角，输入 Distance1/Distance2，实时预览。
- **UI 入口**：`Ribbon → Model 选项卡 → Edges 面板 → 边倒角（LargeTool，标签 “Chamfer”，图标 chamfer.svg）`。
- **样式**：`LargeTool`（图标 + “Chamfer”）。
- **触发命令**：`CHAMFEREDGE`（别名 `SOLIDCHAMFER`；对已选实体边时 `CHAMFER` 也路由到此）。
- **实现位置**：`src/modules/model/edge_cmd.rs::SolidEdgeCommand`（`EdgeOperation::Chamfer`、`new_chamfer`、`EdgeStep::ChamferDistance1/2`）；分发 `src/app/commands/dim.rs::"CHAMFEREDGE" | "SOLIDCHAMFER"`（距离取自文档头 `chamfer_distance_a/b`）。
- **备注**：交互选项：
  - 选择步：`Loop`(`L`)、`Distance`(`D`)（倒角仅作用于同一基准面 `base_face` 上的边）。
  - 环步：`Edge`(`E`)、`Distance`(`D`)；环确认：`Accept`/`Next`。
  - 预览确认：Enter 接受，或 `Distance`(`D`)。
  - Distance1/Distance2 步：`Expression`(`E`)；距离默认来自文档头，`new_chamfer` 以 `positive_default` 归一。启动时命令行输出 `Distance1 = …, Distance2 = …`。

### 抽壳（Shell）
- **功能简介**：将 3D 实体按指定偏移距离抽壳，可移除/添加指定面。
- **UI 入口**：`Ribbon → Model 选项卡 → Edges 面板 → 抽壳（LargeTool，标签 “Shell”，图标 presspull.svg）`。
- **样式**：`LargeTool`（图标 + “Shell”）。
- **触发命令**：`SHELL`（经立面编辑入口 `SOLIDEDIT`）。
- **实现位置**：`src/modules/model/shell_cmd.rs::ShellCommand`（`ShellStep`、`ShellFaceAction`）；分发 `src/app/commands/draw.rs::"SHELL" | "SOLIDEDIT"`（预选单个 Solid3D 时直接进入面移除步）；几何执行 `src/app/model_ops.rs::solid_shell`。
- **备注**：交互选项：
  - 选择实体步：`Select a 3D solid`。
  - 移除面步：`Undo`(`U`)、`Add`(`A`)、`ALL`（Enter 进入距离步）。
  - 添加面步：`Undo`(`U`)、`Remove`(`R`)、`ALL`。
  - 距离步：输入有符号偏移距离（默认 1.0，非零且有限，否则报错 `Value must be nonzero.`）。
  - 经 `SOLIDEDIT` 时先进入 `[Body/eXit]` → `[Shell/eXit]` 两级选项再进入抽壳。拾取面包含填充、使用曲面点、悬停高亮。

---

## 四、未在 Ribbon 出现但属于 Model 模块的入口

以下命令实现于 `src/modules/model/`，但未出现在 Model 选项卡固定面板中，通过命令输入框（`inventory::submit!(CommandRegistration)`）或命令行分发触发。

### 剖切（Slice）
- **功能简介**：以三点/平面对象/曲面/坐标轴/视图/XY/YZ/ZX 平面剖切实体或曲面，并选择保留一侧或两侧。
- **UI 入口**：`命令输入框 → SLICE`（无 Ribbon 按钮）。
- **样式**：剖切平面以线框网格预览（`WireModel::solid_f64` 青色 `[0.2,0.55,1.0]`，`plane_grid`），三点/轴端点用青色线段预览（`slice_triangle`/`slice_normal`）。
- **触发命令**：`SLICE`（别名 `SL`；另有 `SLICE [X|Y|Z] <value>` 直接数值剖切）。
- **实现位置**：`src/modules/model/slice_cmd.rs::SliceCommand`（`Step`、`PlaneKind`、`SliceEntities`/`SliceSurfaceEntities`）；分发 `src/app/commands/draw.rs::"SLICE" | "SL"`；场景执行 `src/app/model_ops.rs::solid_slice/solid_slice_surface/slice_selected`。
- **备注**：交互选项：
  - 目标收集步：选 Solid3D/Surface，Enter 确认。
  - 首点步：`planar Object`(`O`)、`Surface`(`S`)、`Zaxis`(`Z`)、`View`(`V`)、`XY`、`YZ`、`ZX`、`3points`(`3`)（Enter 默认 3 点）。
  - 保留侧步：`Both`(`B`)（Enter 默认 Both）。选平面对象不支持平面时给出 notice 提示；曲面剖切要求单面片切割片（`planar_body_plane` 或单面片 → `SliceSurfaceEntities`）。

### 截面平面（Section Plane）
- **功能简介**：创建持久化 SECTIONOBJECT 截面对象（类型 Plane/Slice/Boundary/Volume），不修改源实体，可正交对齐、按绘制线或过两点定位。
- **UI 入口**：`命令输入框 → SECTIONPLANE`（无 Ribbon 按钮）。
- **样式**：交互式生成 `ExtendedEntityData::SectionObject`（状态 state 1/2/4，指示色索引 9，indicator_alpha 70；名称 “Section Plane (n)”）。
- **触发命令**：`SECTIONPLANE`。
- **实现位置**：`src/modules/model/sectionplane_cmd.rs::SectionPlaneCommand`（`SectionKind`、`Orthographic`、`Step`）；分发 `src/app/commands/draw.rs::"SECTIONPLANE"`（由场景包围盒与已有序号初始化）。
- **备注**：交互选项：
  - 定位步：`Draw section`(`D`)、`Orthographic`(`O`)、`Type`(`T`)，或直接选面/点。
  - 穿过点步：`Specify through point`。
  - 绘制步：起点→若干下一点（Enter 完成）→方向点（`finish_draw`）。
  - 正交步：`Front`/`Back`(`A`)/`Top`/`Bottom`/`Left`/`Right`。
  - 类型步：`Plane`(`P`)/`Slice`(`S`)/`Boundary`(`B`)/`Volume`(`V`)。
  - 绘制至少需两个不同点、连续点不得相同（notice 提示）。

### 三维旋转（3D Rotate，Model 相关）
- **功能简介**：绕 X/Y/Z 轴或有符号角度旋转选中实体。
- **UI 入口**：`命令输入框 → 3DROTATE / ROTATE3D`（无 Model Ribbon 按钮）。
- **触发命令**：`3DROTATE`（`ROTATE3D`，可带 `X|Y|Z <angle>`）。
- **实现位置**：`src/app/commands/draw.rs::"3DROTATE" | "ROTATE3D"`（`SelectThenKeywordCommand`）；几何 `src/app/model_ops.rs::solid_rotate3d`。
- **备注**：先选实体，再选轴，再输入角度（度）。

### 三维镜像（3D Mirror）
- **功能简介**：关于 X/Y/Z 平面镜像选中实体。
- **UI 入口**：`命令输入框 → 3DMIRROR / MIRROR3D`。
- **触发命令**：`3DMIRROR`（`MIRROR3D [X|Y|Z]`）。
- **实现位置**：`src/app/commands/draw.rs::"3DMIRROR" | "MIRROR3D"`；几何 `src/app/model_ops.rs::solid_mirror3d`。

### 三维对齐（3D Align）
- **功能简介**：用 3 个源点对齐到 3 个目标点来放置选中实体。
- **UI 入口**：`命令输入框 → 3DALIGN / ALIGN3D`（`3DALIGN <18 个数字>`：3 源点 + 3 目标点）。
- **触发命令**：`3DALIGN`（`ALIGN3D`）。
- **实现位置**：`src/app/commands/draw.rs`（前缀匹配）；几何 `src/app/model_ops.rs::solid_align3d`。

### 三维干涉检查（Interfere）
- **功能简介**：从选中实体的重叠部分生成新实体（非破坏性交集）。
- **UI 入口**：`命令输入框 → INTERFERE`（无 Model Ribbon 按钮）。
- **触发命令**：`INTERFERE`（`INF`）。
- **实现位置**：`src/app/commands/draw.rs::"INTERFERE"`；执行 `src/app/model_ops.rs::solid_interfere`。

### 加厚（Thicken）
- **功能简介**：将曲面按厚度加厚为实体。
- **UI 入口**：`命令输入框 → THICKEN`。
- **触发命令**：`THICKEN`。
- **实现位置**：`src/modules/insert/solid3d_cmds.rs::ThickenCommand`（`ThickenStep`）；分发 `src/app/commands/display.rs::"THICKEN"`（注册名见 `solid3d_cmds.rs`）。
- **备注**：虽实现于 insert 模块，但属于 3D 实体建模动作，由 Model 类命令共用。

### 转换为曲面（Convert to Surface）
- **功能简介**：将选中实体转换为曲面实体。
- **UI 入口**：`命令输入框 → CONVTOSURFACE`。
- **触发命令**：`CONVTOSURFACE`。
- **实现位置**：`src/app/commands/draw.rs::"CONVTOSURFACE"`；执行 `src/app/model_ops.rs::solid_convtosurface`。

### 平面摄影（Flatshot）
- **功能简介**：将选中实体边投影为 Z=0 处的 2D 线。
- **UI 入口**：`命令输入框 → FLATSHOT`。
- **触发命令**：`FLATSHOT`。
- **实现位置**：`src/app/commands/draw.rs::"FLATSHOT"`；执行 `src/app/model_ops.rs::solid_flatshot`。

### 剖视（Section，数值）
- **功能简介**：沿 X/Y/Z 平面按指定偏移对选中实体取截面。
- **UI 入口**：`命令输入框 → SECTION`（`SECTION [X|Y|Z] <value>`）。
- **触发命令**：`SECTION`。
- **实现位置**：`src/app/commands/draw.rs::"SECTION"`（`SelectThenKeywordCommand`）；执行 `src/app/model_ops.rs::solid_section`。

### 拟合样条（Spline Fit，Model 相关）
- **功能简介**：将选中多段线拟合为平滑样条。
- **UI 入口**：`命令输入框 → SPLINEFIT / FITSPLINE`。
- **触发命令**：`SPLINEFIT`（`FITSPLINE`）。
- **实现位置**：`src/app/commands/draw.rs::"SPLINEFIT" | "FITSPLINE"`；执行 `src/app/model_ops.rs::fit_spline`。

### 三维棱锥（Pyramid，数值/直接命令）
- **功能简介**：直接生成棱锥实体的场景执行入口（与 Create 面板 `PYRAMID` 交互式命令共用几何）。
- **UI 入口**：`命令输入框 → PYRAMID / PYR`（也可经 Create 面板下拉）。
- **触发命令**：`PYRAMID`（`PYR`）。
- **实现位置**：`src/app/commands/draw.rs::"PYRAMID" | "PYR"`；几何 `src/app/model_ops.rs::solid_pyramid`；交互式 `src/modules/model/primitive_cmd.rs`。

---

## 附：命令注册与分发说明

- **自动补全注册**（`inventory::submit!(CommandRegistration)`）：
  - `src/modules/model/primitive_cmd.rs`：`BOX`、`WEDGE`、`CYLINDER`、`CONE`、`SPHERE`、`PYRAMID`、`PYR`、`TORUS`。
  - `src/modules/model/boolean_cmd.rs`：`UNION`、`SUBTRACT`、`INTERSECT`。
  - `src/modules/model/edge_cmd.rs`：`FILLETEDGE`、`SOLIDFILLET`、`CHAMFEREDGE`、`SOLIDCHAMFER`。
  - `src/modules/model/shell_cmd.rs`：`SHELL`、`SOLIDEDIT`。
  - `src/modules/model/sectionplane_cmd.rs`：`SECTIONPLANE`。
  - `src/modules/model/slice_cmd.rs`：`SLICE`、`SL`。
  - `src/modules/model/cylinder_cmd.rs`：由 `"CYLINDER"` 分发处理（未单独注册补全）。
  - `src/modules/model/polysolid_cmd.rs`：由 `"POLYSOLID"` 分发处理。
  - `src/modules/insert/solid3d_cmds.rs`：`EXTRUDE`、`THICKEN`、`PRESSPULL`；`LOFT`；`REVOLVE`；`SWEEP`。
- **命令分发**：`src/app/commands/draw.rs`（基本体、圆柱、抽壳、布尔、多段体、截面平面、剖切、3D 变换、干涉、转换、平面摄影、样条拟合、棱锥）与 `src/app/commands/dim.rs`（`FILLETEDGE`/`CHAMFEREDGE`/实体 `FILLET`/`CHAMFER`）、`src/app/commands/display.rs`（`EXTRUDE`/`REVOLVE`/`LOFT`/`SWEEP`/`PRESSPULL`/`THICKEN`）。
- **图标资源**：`assets/icons/model/` 下 `box.svg`、`cylinder.svg`、`cone.svg`、`sphere.svg`、`pyramid.svg`、`wedge.svg`、`torus.svg`、`polysolid.svg`、`extrude.svg`、`revolve.svg`、`loft.svg`、`sweep.svg`、`presspull.svg`、`union.svg`、`subtract.svg`、`intersect.svg`、`fillet.svg`、`chamfer.svg`。

（文档结束）
