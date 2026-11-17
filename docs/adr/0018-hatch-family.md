# ADR 0018：填充族（HATCH）——一等表示、样式正确的填充、真实导出与创建子集（P6）

- 状态：**Accepted**。设计门禁 **SG-P6 attempt 1 = APPROVE-WITH-FIXES**（B1–B6、N1–N9 已并入）→
  **attempt 2 = ACCEPT-WITH-FIXES**（剩余为文案更正，已并入）→ **P6 已实施**（P6a0/a-1/a-2 + P6b）；
  **P6 阶段门禁 = closed-with-fixes**（无代码阻塞，文档更正已落）。本文件是 P6 的**唯一设计权威**；
  实施进度另起小节追加，**不改写决策文本**。
- **开篇即声明（N8）**：**本相不提供真正的填充预览**。实时预览只有后端橡皮筋链；完整的填充/图案预览需要一个
  **渲染 overlay 分支**，而 **React 宿主无法绘制 overlay**（原生 CAD 分区是 X11 子窗口 / Wayland 子表面，DOM 不得浮于其上——ADR-0010 `:27`）。
  这一点在 P5 已发生一次，此处**前置声明**，不在实施中重复发现。

  > **2026-10-23 前提更正**：`React 宿主无法绘制 overlay` 的**几何前提已失效**
  > （[ADR 0023](0023-dom-on-top-cross-host-composition.md) D1）。但**填充预览仍然未实现**：缺的是后端 overlay
  > 分支（图案/边界的真实合成），不是前端 DOM 绘制能力。声明限制以新理由保留。

- 范围：`crates/cad-domain`（新域变体 + 迁入的填充几何）、`crates/cad-geometry`（迁出 hatch 模块）、
  `crates/cad-import-acadrust`（导入迁移 + 导出）、`crates/cad-app`（创建命令/工具/参数）、`crates/cad-ui-core`
  （选择子/参数/状态/i18n）、React 宿主 UI。
- **不**覆盖：关联性（`HPASSOC`/`71`）、渐变**创建**（导入保留并往返）、**预定义/自定义图案名**（无图案表、无 `.pat`
  解析器；未来路径是**用户自带 `.pat` 文件**，不是打包 `acad.pat`）、**内部点求边界**、LINE/ARC **边界拼接**、
  注释性缩放、`HPDRAWORDER`、间隙容差（`HPGAPTOL`）、样条边界创建、MPolygon、叠加式岛屿检测（超出样式语义）、
  文字作为岛屿、非共面/自交边界、`HPSEPARATE`。
- 相关：`docs/compatibility.md`（HATCH `:40`、渐变 `:52`、STRETCH `:211`、EXPLODE `:212`）、
  `docs/dxf-entity-coverage.md:117`、`docs/shortcuts.md:155`（HATCH 现列为未实现）、`CAD_IMPLEMENTATION_SPEC.md` §3.2、
  ADR-0017（一等实体 + 真实导出 + 声明账本的先例，含其 **A.8 匹配点清单**纪律）。

---

## 0. 问题与约束

1. **填充不是一等类型**：HATCH 导入成 `SemanticGeometry::Compound`（每环一条闭合折线 + 实心/渐变 `Mesh` + 图案折线），
   而**导出把 `Compound` 摊平**（`export/entity.rs:221`）⇒ **创建的填充存盘会变成散线 + 网格，而不是 `HATCH`**。
   与 P5a 的"标注退化成散线"同类。
2. **样式在导入时被丢弃**：`style`、`pattern_type`、`seed_points`、`pixel_size`、每环 `boundary_handles`、
   `BoundaryPathFlags` 全未建模；填充按**奇偶嵌套深度**重算 ⇒ 等价 `Normal`。**精确表述（N9）**：
   - `Ignore` **在有岛屿处**就与 `Normal` 不同 ⇒ **现在被填错**；
   - `Outer` 在两环 donut 下 **与 `Normal` 等价**，**只在 ≥3 层嵌套**才不同 ⇒ 缺陷**只在深层显现**。
   两种都是"**填错区域**"（规格明列"不得省略"），故必须修，并**显式声明为渲染变化**。
3. **Autodesk（ObjectARX `Hatch` 类）明说**：*"the computed hatch lines and solid fill are **not saved**… AutoCAD
   recomputes … when a DWG or DXF file is opened"* ⇒ 持久实体存**边界环 + 图案身份 + 图案定义行 + 样式 + 种子点**，
   **不是**裁剪后的线段。
4. **acadrust 写入器的硬约束（B2）**：`section_writer.rs:4543-4554` 的
   `effective_associative = hatch.is_associative && paths carry resolvable boundary handles` ⇒ **我们无句柄 ⇒ 71 恒 0**。
   故**不建模 `associative`**（存了也既不能驱动、也不能往返——正是 P5 B.2 判定的"存储但不消费"）。
5. **用户自定义图案的真相（N4）**：`UserDefined`(76=0) **不携带定义行**，需由 `52`/`41`/`77` **合成**；而现有
   `pattern_families` **只重放文件里的 `78` 行** ⇒ 一个 `78=0` 的用户自定义填充今天直接 `Partial`
   "no line families"——**这是第三处潜伏导入缺陷**，P6a 必须一并修（且创建的填充也要能显示）。

---

## 决策 A：域表示与显示派生位置

### A.1 模型（OCS 平面 2D + 标高；角度**弧度**）

```rust
SemanticGeometry::Hatch(Box<HatchGeometry>)

pub struct HatchGeometry {               // 字段私有；唯一写入路径 new / from_import / transformed
    loops: Vec<HatchLoop>,               // **OCS 帽面 2D** 点 + bulge；见 A.2
    plane: HatchPlane,                   // normal + elevation（DXF 10/20 恒 0、30 = 标高、210 法向）
    style: HatchStyle,                   // Normal | Outer | Ignore
    fill: HatchFill,                     // Solid | Pattern(HatchPattern) | Gradient(HatchGradient)
    seeds: Vec<[f64; 2]>,                // OCS 2D 种子点；**不透明往返数据**（U-4）
    pixel_size: Option<f64>,             // DXF 组 47；**仅当某环带 DERIVED 位时写入器会发**（B3）
    pattern_type: HatchPatternType,      // UserDefined(76=0) | Predefined(1) | Custom(2)
    display: Vec<SemanticGeometry>,      // **派生**：边界折线 + 填充网格 或 图案折线（见 A.3）
}

pub struct HatchLoop {
    points: Vec<[f64; 2]>, bulges: Vec<f64>,   // 帽面 2D
    flags: u32,                                // **原始位**（见 N3），附语义访问器
    source_edges: Option<Vec<BoundaryEdge>>,   // 可选：源解析边（见 B4 的忠实度取舍）
}

pub struct HatchPattern { name: String, angle: f64 /*rad*/, scale: f64, double: bool, lines: Vec<PatternLine> }
pub struct PatternLine { angle: f64 /*rad*/, base_x: f64, base_y: f64, offset_x: f64, offset_y: f64, dashes: Vec<f64> }

pub enum HatchGradient {                    // 导入保留 + 往返（B3）
    TwoColor { name: String, reserved: i32, angle_raw: f64, shift: f64, stops: Vec<GradientStop> },
    SingleColor { name: String, reserved: i32, angle_raw: f64, shift: f64, tint: f64, stops: Vec<GradientStop> },
}
pub struct GradientStop { value: f64, rgb: [u8; 3] }   // 463 / 63+421；color_name/book_name **导入即丢**（声明）

pub enum BoundaryEdge {                     // 供导出写解析边（B4）
    Line { start: [f64; 2], end: [f64; 2] },
    CircularArc { center: [f64; 2], radius: f64, start_angle: f64, end_angle: f64, ccw: bool },
    EllipticArc { center: [f64; 2], major_axis: [f64; 2], ratio: f64, start_angle: f64, end_angle: f64, ccw: bool },
    Spline { /* degree/knots/control_points/weights/fit 数据，镜像 acadrust 的边类型 */ },
}
```

- **坐标与单位（B3）**：环/种子/图案定义点一律**帽面 OCS 2D** + `plane`；`plane` 负责世界↔帽面映射（显示关注）。
  **图案角 `52`/`53` 用弧度**（写入器自己 `to_degrees()`，已核实）。
  **渐变角 `460` 例外（attempt 2 更正）**：写入器**原样写**、读取器**原样存**，而结构注释声称弧度 ⇒ **单位未验证**
  ⇒ `HatchGradient.angle_raw` **按不透明往返数据保存**（U-13），**不**在导入时转换；现有 `gradient.rs` 把该值直接
  喂给弧度 ramp ⇒ 方向可能已错，且**没有渐变夹具**能发现。
  （**2026-10-21 更新：U-13 已解决**——组 `460` 确为**弧度**，证据与残余见决策 F 的 U-13 条目；`angle_raw` 仍按
  原值往返、不做单位换算，且旋转时随几何同向旋转 `angle_raw += delta`。）
- **`flags: u32` 原始位**（N3）：`BoundaryPathFlags` 实为 9 位（external/polyline/derived/textbox/outermost/
  not-closed/self-intersecting/text-island/duplicate），写入器**逐位照发**；用 5 个布尔会**静默归一化**掉 4 位。
  只对**承重位**给访问器（`derived` 决定组 47；`textbox` 保留但不解释 = U-5）。
- **渐变字段**（B3）：携带原始 `470` 名、`451` 保留值、角度/位移/单色标记/`tint`、色标。**声明的不可往返损失**：
  写入器只写 `63`+`421`，**`color_name`/`book_name` 写不出去**。
- **不建模 `associative`**（§0.4）：导出 `71` **恒 0**，`ExportReport` 与能力表声明"关联性不保留"。
- **`pixel_size`**：**只在某环带 `derived` 时**写入器发组 47 ⇒ 保留该字段（B3），否则导出只能凭空造值。

### A.2 显示派生位置（B1）——沿用 Dimension 先例，把填充几何**迁入 `cad-domain`**

- 采用 **(i) 存储派生显示**：`display` 由**唯一构造器**从 `loops + style + fill + plane` 算出（`fill_rings` /
  `pattern_polylines` / `gradient_vertex_colors`），与 `Dimension` 的 `synthesized()` 同构。
- 为此把 `crates/cad-geometry/src/hatch/`（`mod/fill/pattern/gradient/simplify`，**纯 std f64 数学，已核实**）
  **迁入 `crates/cad-domain/src/hatch/`**。**这不是偏好而是被迫**：`cad-geometry` **依赖** `cad-domain`（反向不允许），
  所以"与类型同居的构造器"**无法回调 `cad-geometry`**。
  **决定：不保留 re-export**（`cad-geometry` 永久转出 `cad-domain` 的条目属仓库不鼓励的别名）；四个调用点改为
  `cad_domain::hatch::…`：`cad-import-acadrust/src/entity/hatch.rs:105,142`、
  **`crates/cad-representation/src/text/finalize.rs:142`**（字形填充）、
  **`crates/cad-representation/src/tests/basic.rs:257`**（更正：此前误写为 `cad-geometry/src/text/finalize.rs`）。
- **拆成 P6a0**（零行为变更，验收 = 既有测试全绿 + 渲染逐字节不变），与 P6a 的三处**已声明渲染变化**分开归因。
- **消费者清单（必须逐个决定）**：`cad-representation`（显示，递归 `display`）、`cad-spatial`（`pick/hit.rs:303`、
  `pick/bounds.rs:159`）、`cad-measure`（`snap/generate.rs` 的边界顶点）、`cad-db`（`bounds.rs`/`validate.rs`）、
  `export/entity.rs`、`app_drawing/helpers.rs::class_name`。
- **保持拾取等价**：`display` 含填充网格 ⇒ **点击填充内部仍会选中该填充**（与今天的 `Mesh` 子项一致）。
- **被否决的 (ii) 按需派生**：结构上更纯（`transformed` 无需重算标量），但**每次拾取都要重新三角化**，除非另做缓存；
  且与 `cad-db`/`cad-spatial` 现行的"存储派生显示"模型不一致。记录为未来可选项。

### A.3 填充语义（样式 = **环集筛选**）

- **深度**：从外向内 **1 基**计数（= `ring_depths` 的包含计数 + 1；注意 `fill.rs:136-138` 的文档用 **0 基**
  描述"偶数为实"——实施时勿差一）。
- **权威是穿越奇偶**（DXF 组 75 原文 "Hatch **odd parity** area"）：在接受的域内（环两两**不相交或严格包含**），
  **奇偶 ≡ 深度**。
- **`display` 保留全部边界环折线**（否则边界顶点不再可捕捉）；**样式筛选只作用于填充/图案/渐变区域**。
- **样式 = 环集筛选**（复用 `fill_rings`/`pattern_polylines` 不动）：
  - `Normal`：全部环；`Outer`：**丢弃深度 ≥3 的环**；`Ignore`：**丢弃深度 ≥2 的环**。
- **样式对图案与渐变同样生效**（N2）：`pattern_polylines` 的裁剪与渐变的烘焙区域都必须按筛选后的环集，验收要覆盖
  **图案线总长**与**渐变烘焙区域**，不能只测实心。
- **拒绝而非近似（仅创建路径，见 B6）**：环**自交**、环间**部分重叠**（既非不相交也非包含）、**相切/共享顶点**、
  **重复环**、**未闭合**（`closed` 位为假且首尾不重合）、**非共面** ⇒ 创建命令**整命令拒绝**并诊断。
  **导入不套用拒绝**：沿用今天的"坏环跳过 / `fill_rings` 失败 ⇒ 边界-only + `Partial`"，**不得**把现在能画的
  `Partial` 填充变成拒绝实体（那会是未声明的回归）。

### A.4 不变量

> `loops` 的嵌套关系 + `style` 共同决定**填充区域**；`fill` 定义 + `plane` 决定**图案几何**。
> `display` 是二者的**派生**，唯一写入路径是构造器与 `transformed`；**不接受**任何外部烘好的线段作为权威。

---

## 决策 B：导入迁移与既有缺陷修复

- **保留**（不再丢弃）：`style`、`pattern_type`、`seeds`、`pixel_size`、每环**原始 flags**。
- **修复三处既有缺陷**（均须**声明为渲染变化**）：
  1. `Outer`/`Ignore` 填错（§0.2；`Outer` 仅 ≥3 层可见）；
  2. **用户自定义图案 `78=0`** 现在 `Partial`（N4）⇒ 由 `52/41/77` **合成**图案族（生成算法标注为**我们的实现**，U-1）；
  3. `pattern_families` **不缩放 `base_point`**（只旋转）且**不旋转 offset**（U-7）⇒ 迁移时用夹具核对或声明。
- **边界忠实度（B4，取舍须写明）**：现有采样把 arc/ellipse/spline **采样成折线**、折线 **丢 bulge** ⇒ 导出会写
  **采样后的折线路径**而非解析边。**选择**：`HatchLoop.source_edges: Option<Vec<BoundaryEdge>>` 保留**源解析边**
  供导出使用；缺失时（如新建环）退回采样，并**在报告与能力表声明**"解析边界在无源边时降级为采样折线"。
  **便宜收益**：创建的 `Circle` 环可直接写成**两点带 bulge 的折线路径**（bulge 精确表示圆）或整圆 `CircularArc` 边。
- **渐变**：保持导入支持（LINEAR/CYLINDER/SPHERICAL 的**方向**依赖 `460` 单位 ⇒ 只能声明为"按原值烘焙"，
  其余 `Partial`；**U-13 已解决：`460` 为弧度**，见决策 F），并**往返**（B3 字段 + `reserved`/`angle_raw`/色标保留，
  `color_name`/`book_name` **导入即丢**）。
- 无可用边界 ⇒ `Opaque{"AcDbHatch"}` + `Partial`（沿用）。

---

## 决策 C：导出 —— 写真 `HATCH`（写定义）

- 组码：`10/20/30` 标高（10/20 恒 0）、`210` 法向、`2` 图案名、`70`、**`71` 恒 0（§0.4）**、`91` + 每环 `92` 原始位
  + 环数据、`75`、`76`、`52`、`41`、`77`、`78` + 定义行（`53,43,44,45,46,79,49…`）、`98` + 种子点（`10/20`）、
  **`47` 仅当存在 `derived` 环**、渐变块（若原是渐变）。
- **不写裁剪后的图案线段**（§0.3）。
- **`ExportReport` 逐条声明**：样式、图案类型、种子点、"按定义重算、未写线段"、**`71=0`（关联性不保留）**、
  **渐变 `color_name`/`book_name` 不可写**、**无源边时的边界采样降级**、以及 `pixel_size` 的存在性。
- **最小有效集是推断**（U-6）。

---

## 决策 D：创建（P6b）—— 预选实体形态（**不是**新增捕获状态机）

- **边界 = 预选实体**：`DrawToolKind::Hatch` 用 `arity() = PointArity::Exact(0)` + `requires_selection() = true`
  （与 **ARRAY/MOVE** 同型；已有测试 `draw_tool/tests/array.rs` 的 "zero point arity is ready on selection alone"）。
  **不需要** `PickedSteps::UntilConfirm`——**D.3 原标注的"最大实现风险"因此消失**。
- **参数**走既有 `ToolParameter` 通道（POLYGON `sides` 的先例）：图案类型（实心/用户自定义）、间距、角度、双线、样式。
  （**2026-10-27 追加**：类型码扩到 `0..=3`，`3` 渐变带自己的四个参数——kind/角度(度)/两个 ACI 颜色；见决策 G 的 P6c 补注。）
- **接受**：`Circle`、`Ellipse`、**闭合** `Polyline`（含 bulge），每个实体各自成环；嵌套按 A.3 重算。
- **拒绝（命名、且不开事务）**：非闭合/开放折线、`Line`/`Arc`（无拼接器）、样条边界、自交、相切/重复、部分重叠、
  退化面积、非共面、缺活动图层、Viewer。
- **明确 NOT IMPLEMENTED（且 UI 不得暗示；2026-10-27 就地更正其中两项，2026-11-01 再更正一项）**：~~内部点求边界~~
  （**2026-11-01 已由 `HATCHPOINT` v1 实现，见文末补注**；`LINE`/`ARC` 自动发现仍未）、绘制边界、`remove boundaries`、~~预定义/自定义图案名~~
  （**预定义名 2026-10-26 已兑现**：三项转录表按 1 基索引、状态栏按名 chip；自定义 `76=2`/`.pat` **仍未**）、
  ~~渐变创建~~（**2026-10-27 已兑现**：类型码 `3` 双色 LINEAR/SPHERICAL，见 §决策 G 的 P6c 补注）、关联性。
  未来路径记录为**用户自带 `.pat` 文件**（无再分发问题）。
- 一条事务/一步撤销/活动图层；`type_key` = **`AcDbHatch`**、`class_name` 映射 `"AcDbHatch"`
  （**不得**写成 `AcDbCompound`）。
- **无真正创建预览**（见开篇声明）。

---

## 决策 E：匹配点清单（**B5**，照 ADR-0017 A.8 的纪律）

**通配符臂（不补就静默处理错）**：`cad-import-acadrust/src/support.rs:63` `_ => (Verified, Verified)`
（填充会被报 **假 `Verified`**，须镜像 `Compound` 递归）、`cad-spatial/src/pick/bounds.rs:114` `_ => return None`
与 `:164` `_ => local_pick_bounds`（失去精确局部包围盒 ⇒ 落 AABB）、`pick/hit.rs:118-124`（拾取族映射）、
`cad-app/src/select_box.rs:~384`（框选退化 AABB；`:377` 是 Dimension 臂，attempt 2 更正）、
**`cad-platform/src/font_keys.rs:45,60`**（递归；attempt 2 更正，此前误写 `fonts.rs`）、
`cad-import-acadrust/src/{geometry.rs:389,entity/push.rs:253}`、`cad-proxy/src/lib.rs:490`、
**`cad-measure/src/snap/resolve.rs:16`**（`is_intersectable` 的 `_ => false` ⇒ 填充会**静默失去交点捕捉**）、
**`cad-app/src/select_all.rs:116`**（`_ => Ok(())` ⇒ 校验被**静默跳过**）、
**`cad-app/src/app_drawing/explode.rs:419`**（**块内**填充今天按 insert-child 拒绝；变体化后须**保持该拒绝**）、
**`cad-geometry/src/intersect.rs:161`**（`_ => None`，判定并记录）。

**穷尽臂（编译器强制，照改即可）**：`cad-db/src/{transform,bounds,validate}.rs`、
`cad-geometry/src/{tessellate,vector,engine}.rs`、`export/entity.rs:258/751`（`kind_name`）、
`app_drawing/helpers.rs:15`（`class_name` → `"AcDbHatch"`）、`selection.rs`、`cad-representation` 主 match。

**必须决定的行为**（逐条写进实施）：
- **`transform_geometry`**：平移/旋转/**均匀缩放**变换 `loops`（帽面 2D）+ `plane` + `seeds`，并**重建 `display`**；
  **非均匀缩放/镜像 ⇒ 整实体 `Unsupported`**（同 Dimension A.5）。
- **`EXPLODE`**：**保持今天的行为**——分解为边界折线 + 填充网格 + 图案折线（今天是 `Compound` 的分解，
  由 `tests/drawing/explode.rs:224` 钉住，且 `refuse_compound_type_key` 不含 `AcDbHatch`）⇒ 只把变体名换掉，
  **不改成拒绝**（拒绝会是白给的行为变更）。
- **`STRETCH`**：**采用 `compatibility.md:211` 的既有口径**（部分包围 ⇒ 不动 + 诊断；完全包围 ⇒ 整移），
  **不重新开放该选择**（attempt 2）。
- **`snap`**：边界顶点今天经折线子项**是**捕捉点 ⇒ 迁移后必须保持（`display` 含边界折线即可）。
- **拾取**：填充内部可选中（`display` 含网格）。

---

## 决策 F：未查证 / UNVERIFIED（**N7 更正后的清单**）

- **U-1 图案线生成公式**：哪一族是 k=0、偏置符号、`HPORIGIN` 与旋转/缩放的复合顺序、dash 相位锚点——**无公式**
  ⇒ 只存定义、按定义重算，生成结果标注为**我们的实现**（同时覆盖 N4 的用户自定义合成）。
- **U-2 `41` 是否也缩放 dash 长度**：未述 ⇒ 声明。
- ~~U-3~~ ⇒ **更正**：`Normal` = 奇偶是**文档化的**（DXF 组 75 "odd parity"、acadrust 枚举注释），**不是**我们的读法。
  **真正属于"我们的读法"的是 `Outer` 在 ≥3 层**（DXF 措辞 "external to outer loop" 与 acadrust 的 "outermost area only"
  不完全一致）⇒ 记为 **U-3'**。
- **U-4 种子点语义**：与岛屿的交互、重算时是否重推岛屿——**未述** ⇒ 当**不透明往返数据**。
- **U-5 `92` 位组合**与 `92=1/16/0` 的区别：未定义 ⇒ 保留位、不解释；`textbox` 保留但不解释。
- **U-6 最小有效 HATCH**：Autodesk **无声明** ⇒ 我们的最小集是推断。
- **U-7 `pattern_families`**：不缩放 `base_point`、不旋转 `offset`——与真实文件的一致性**未验证**。
- **U-8 渐变名集合**：官方只述 `470` 为字符串；其余 8 名来自社区 ⇒ 标注。
- **U-9 重算的版本绑定**：计算出的图案线按版本重算 ⇒ **像素级 golden 不可跨版本**。
- **U-10（新）组 47 语义**：像素尺寸仅述为"交集/射线密度"⇒ 保留值、不解释。
- **U-11（新）用户自定义 `76=0` 时 `78` 的优先级**：文件同时带 `52/41/77` 与 `78>0` 时 AutoCAD 采哪一个——**未查证**。
  证据：`flange.dxf` 的 `76=0` 填充带 `78=1`。
- **U-12（新）渐变不可往返项**：`color_name`/`book_name` 写入器不写 ⇒ 声明损失。
- **U-13（新，attempt 2）渐变角 `460` 的单位**：写入器**原样写**、读取器**原样存**，结构注释称弧度但**无实证**；
  且没有任何渐变夹具 ⇒ 按**不透明往返**处理，`gradient.rs` 的烘焙方向**可能已错**，声明为未验证。
  **已解决（2026-10-21）**：单位为**弧度**。证据：① DXF 参考组 `460` 为弧度；② acadrust 字段文档
  `entities/hatch.rs` `HatchGradientPattern.angle` = "Gradient angle in radians"；③ 本仓方向回读测试
  `gradient_angle_is_stored_in_radians_not_degrees`（存 `FRAC_PI_6` ⇒ 烘焙斜坡 30°）与
  `transform_rotates_the_gradient_ramp_with_the_geometry`（+30° 旋转 ⇒ 斜坡 +30°）。**残余**：仍**无**
  vendor 产出的非零角度渐变夹具 ⇒ 结论是"参考文档 + 合成测试"，不是真实文件往返证据；`angle_raw` 继续
  原值往返、不做单位换算。

---

## 决策 G：分阶段、测试与证据

### 阶段

| 子阶段 | 内容 | 验收 |
|---|---|---|
| **P6a0** | **仅模块搬迁**：`cad-geometry/src/hatch/` → `cad-domain/src/hatch/`，四个调用点改路径、**不保留 re-export** | **零行为变更**：既有测试全绿 + 渲染逐字节不变 |
| **P6a** | 域变体 + 导入迁移（保留字段、修**三处**既有缺陷）+ 导出真 `HATCH` + 显示按样式与定义重算 + 匹配点清单 | 往返保全（样式/`pattern_type`/种子/原始 `92` 位/图案行/渐变名与色标）+ **三种样式的填充分区与图案线总长**数值测试 + 导出组码断言（含 **`71` 恒 0** 与 **`47` 仅 derived**）+ **渲染变化声明** |
| **P6b** | `HATCH` 创建：**预选闭合实体** → 环；`SOLID` + **用户自定义图案**（间距/角度/双线）+ 样式参数；一条事务 | 数值几何 + 拒绝面 + 导出往返 + 真机证据 |
| ~~P6c~~ | 内部点求边界 / LINE 拼接 / 预定义图案名 / 渐变创建 / 关联性 / 用户自带 `.pat` | **NOT IMPLEMENTED**（显式记录；**预定义图案名 2026-10-26、渐变创建 2026-10-27、内部点求边界 2026-11-01 已兑现**，见下方补注） |

**P6c 补注（2026-10-26，追加，不改写上面历史行）**：已实现一张**极小的手工转录**预定义图案表
（`ANSI31`/`ANSI32`/`ANSI37`，`cad-domain/src/hatch/patterns.rs`），由 `HATCH` 图案类型码 `2` +
1 基索引 `hatchPattern` 选择，经既有 `HatchPattern::families()` 出线，`76` 写 `1`。表内数值是
**我们的转录值**（ADR U-1 纪律，**未**与 vendor 文件/参考渲染比对），越界索引**按名拒绝**。
**未**兑现的部分仍是文档化的未来路径：**用户自带 `.pat` 文件加载**（无解析器、无 `acad.pat`
打包/再分发）、自定义（`76=2`）图案名、图案选择对话框、实时预览、虚线相位按裁剪段重置。
预定义表只是让"选一个名字画几种常见图案"可用，**不**等于图案库支持。

**P6c 补注之二（2026-10-27）**：**渐变创建**也已兑现——`HATCH` 图案类型码 `3` + 四个 `Number` 参数
（`hatchGradientKind` `{0 LINEAR, 1 SPHERICAL}`、`hatchGradientAngle` **度**、`hatchGradientColorA/B`
**整数 ACI `1..=255`**），实体写 `HatchGradient::TwoColor` 且 `pattern_type` 仍记 `Predefined`（`76=1`）。
它**不**改变上面的结论：`.pat`/自定义 `76=2`、图案选择对话框、实时预览、关联性仍未实现；渐变自身还有三条
已声明性质缺口（`SPHERICAL` 在矩形/圆边界上读平色、只有 ACI 无真彩、`76=1` 的渐变约定**未用 vendor 文件核实**），
见 `docs/hatch-gradient.md` §2b 与 `docs/compatibility.md` 的 HATCH（创建）行。

### 测试（含门禁点名的缺口）

1. **样式派发数值**：同心 3 环 + **两个不相交外环**；断言**实心面积**（`Normal={1,3}`、`Outer={1}`、`Ignore={1,2,3}`）、
   **图案线总长**（今天 `tests/hatch.rs` **零样式覆盖**）、**渐变烘焙区域**；再加拒绝用例（相切/重复/部分重叠/自交）。
2. **导出字节断言**：`71=0` **恒定**（钉住 B2）、`2/70/75/76/92 位/93/72/73/97/52/41/77/78+53,43,44,45,46,79,49/98/47`
   （`47` **恰在**存在 derived 环时出现）；导入→导出→导入保全。
3. **夹具往返**：`flange.dxf`（5 个多环 SOLID + 1 个 `76=0` 带 `78=1`）、`isometric_grid.dxf`（9 环 SOLID），
   断言环数、`type_key == "AcDbHatch"` 且**不是** `Compound`。
4. **新增合成夹具（N5）**：所有已提交夹具的样式**全是 `75=0`** ⇒ `Outer`/`Ignore` 修复只能靠合成测试。
   手写一张 **3 层嵌套 + 一个 `Outer` + 一个 `Ignore` + 两个不相交外环** 的 DXF，按 `fixtures/manifest` 的
   provenance 规则登记。
5. **匹配臂回归**（P5 F.3 三连的一般化）：填充内部拾取等价、框选与等价 `Compound` 同分类、边界顶点仍可捕捉、
   `display_support` 不假 `Verified`、`kind_name`/`class_name` = `AcDbHatch`、变换（平移/旋转/均匀缩放 + 非均匀/镜像拒绝）、
   `EXPLODE` 行为不变。
6. **一次像素判读**：3 层嵌套下 `Normal` vs `Outer` 的真机帧（廉价高信号）；**诚实说明**样式证据来自**合成**夹具。

### 文档更正清单（N8，P5 教训的对应物）

`docs/shortcuts.md:155`（HATCH 现列为未实现）、`docs/compatibility.md:40`（HATCH 行、拾取列"经边界(AABB)"须重述）、
`:52`（渐变行）、`:211`（STRETCH）、`:212`（EXPLODE 行"Compound→全部子项(含 hatch…)"随变体改变）、
`docs/dxf-entity-coverage.md:117`。

### 文件计划（N6）

`cad-domain/src/hatch.rs`（**`lib.rs` 已 931 行，变体 + 结构必破 1000**）、迁入的 `cad-domain/src/hatch/*`、
`export/hatch.rs`（`export/entity.rs` 已 770）、`app_drawing/hatch.rs`（`helpers.rs` 已 914）、
注意 `draw_tool/mod.rs`（897）。**无白名单豁免。**

### i18n（N6）

逐阶段列出新增键：P6a **0 个**（无 UI 面）；P6b 需 `draw.kind.hatch`、`tool.draw.hatch`、
`tool.prompt.hatch.{select_boundary,ready}` 与参数标签（图案类型/间距/角度/双线/样式），两目录键集一致。

### 证据边界

沿用既有：合成 DOM 指针事件（**非真实鼠标**）、`cad_status` **无几何通道** ⇒ 真机只证"提交成功 + 计数 + 内容存在"；
**填充分区正确性由契约测试 + 像素判读共同钉住**（填错区域是可见的）。截图仅含应用窗口自身。

---

## 实施状态（追加，不改写决策文本）

### P6a0（模块搬迁，提交 b639663）

- `cad-geometry/src/hatch/` → **`cad-domain/src/hatch/`**（`git mv`，**内容零变化**，仅一处模块文档的路径句）；
  **不保留 re-export**，四个调用点改 `cad_domain::hatch::…`（导入侧边界/填充/图案/渐变、**表示层的字形填充**
  `cad-representation/src/text/finalize.rs`、一个测试导入路径）。
- **验收 = 零行为变更**：既有测试全绿、**断言与期望值一个未改**、architecture 无向上依赖。**U-7 的缺陷被看到但
 刻意不修**（保持本片可归因），见 P6a-1。

### P6a-1（域变体 + 样式正确填充 + 导入迁移，提交 e8bfaf0）

- `crates/cad-domain/src/hatch_entity.rs`（+`validate/geometry/tests`）：`HatchGeometry`，**私有字段 + 唯一写入路径**
  `new`/`from_import`/`transformed`；环在**帽面 OCS 2D**、**原始 `u32` flags**、可选 `source_edges`（`BoundaryEdge`）、
  `plane`/`style`/`fill`/`seeds`/`pixel_size`/`pattern_type`/`display`；渐变 `reserved` + **不透明 `angle_raw`**。
- **显示 = 单一构造器**：`display` **保留全部边界环折线**（边界顶点仍可捕捉、填充内部仍可选中），**样式只筛
  填充/图案/渐变区域**；`filtered_loops`：`Normal` 全部、`Outer` 去 1 基深度 ≥3、`Ignore` 去 ≥2。
- **修三处既有缺陷（均为渲染变化）**：① `Outer`/`Ignore` 曾按 `Normal` 填；② 用户自定义 `76=0`/`78=0` 曾
  `Partial`，现由 `52/41/77` 合成；③ **图案复合**：`pattern_families` 并入 `HatchPattern::families`，
  现在**缩放基点 + 旋转 offset**（U-7 已修）。
- **匹配点清单**：ADR 决策 E 全部落实；复核逐个核对**无"编译通过但返回错"的臂**。
- **行为**：变换（平移/旋转/均匀缩放，重建 `display`；非均匀/镜像**整实体拒绝**）；**EXPLODE 保持既有分解**且
  既有测试期望不变（块内填充仍拒绝）；`STRETCH` 沿用能力表口径；边界顶点仍可捕捉；填充内部仍可选中；
  `class_name` = **`"AcDbHatch"`**。

### P6a-2（真 `HATCH` 导出，提交 0f7e859）

- 新 `export/hatch.rs`：写**真 `HATCH`**（`10/20/30`、`210`、`2`、`70`、**`71` 恒 0**、`91` + 每环 `92` 原始位、
  **优先解析边**、否则采样折线并**声明降级**、`75`/`76`/`52`/`41`/`77`/`78` + 定义行、`98` + 种子、
  **`47` 仅 derived 环**、渐变块仅在渐变时）；**永不读 `display()` 的裁剪线段**。
- **不再摊平**：`export/entity.rs` 把 `Hatch` 派发到写入器；**始终 `Converted` 而非 `Exact`**（须逐实体声明
  style/type/seeds/`71=0`）。
- **新增合成多样式夹具** `fixtures/dxf/hatch-styles/multi-style.dxf`（3 层嵌套 + 一个 `Outer` + 一个 `Ignore` +
  两个不相交外环），manifest 记 provenance/SHA-256/已知限制。
- **声明**：实心写 `2 = SOLID`（域不存实心图案名）；折线路径形式**强制 `92` POLYLINE 位**（无源边的合成环"加位"
  记为**转换**而非静默归一化）；法兰 6 个填充中 5 个在 `Title` 块且只被图纸空间引用 ⇒ **模型空间导出只出 1 个 `HATCH`**
  （已断言该丢弃）；acadrust 不能写渐变的 `color_name`/`book_name`（声明损失）。

### 修正与复核结论（P6a 之后）

- **i18n 计数更正**：ADR G 曾写"P6a **0 个**新键"，实际 **+3**（`properties.row.{loops,style,fill}`）——
  属性行是**穷尽匹配**，新增变体必然新增行 ⇒ **是 ADR 的计数错，不是代码多做的**。i18n 现 **520** 键。
- **图案复合的证据等级（复核）**：四条断言中**一条是真判别性不变量**（offset 与最终方向**垂直**且幅值按 `scale`
  缩放——**旧复合会失败**），两条是公式自证，**没有任何**与独立实现/AutoCAD 的比对。⇒ 结论：**可以保留**
  （它是两种复合法里唯一"定义的**相似变换**"），但**"verified against the fixture" 的说法过强**，
  统一改为"**用夹具值核对 + 一条形状不变量**"；**U-7 已修但 U-1（生成公式）仍是"我们的实现"**。
- **样式派发的算术**（复核独立重算）：10×10 / 6×6 / 2×2 ⇒ 区域 64/32/4 ⇒ `Normal={1,3}=68`、`Outer={1}=64`、
  `Ignore=100`；图案行实际落在 `y = 0,2,4,6,8`（测试注释曾按旧复合写成 1,3,5,7,9，**已修正**），
  总长 34/32/50 **能区分三种样式与两个差一阈值**。
- **复核点出的两处未列通配符（已处理）**：`cad-geometry/src/engine.rs::fallback_scale` 补 Hatch 臂
  （此前落 `_ => 0.0`，采样尺度被静默改变）；`cad-spatial/src/pick/bounds.rs::cached_local_pick_bounds`
  补 Hatch 臂（此前**精确但未缓存**，与 `Compound`/`Dimension` 不一致）。
- **仍存的局限（声明，2026-10-20 复核后重述）**：**`loop_depths` 已不是"只探首点"**——`0fa3d8e` 改为
  **最多 8 个均匀分布的探测点 + 确定性回退**（`hatch_entity/validate.rs`，有"首点相切"的判别测试）；
  真正更窄的残余是**探测点均分（部分重叠）时回退到首顶点**。
  **未声明的孪生**：`cad-domain/src/hatch/fill.rs` 的 **`ring_depths` 仍只探测 `ring[0]`**，而它驱动
  **奇偶填充配平**（谁算孔洞）⇒ **导入路径上相切环仍可能被填错**（创建路径拒绝相切，故不受影响；
  `Normal` 与所有已提交夹具不受影响）。**列为待办**（修它需改填充配平语义 + 声明渲染变化）。
  另：**无渐变夹具** ⇒ `460` 单位（U-13）与渐变方向**无回归覆盖**；**无图形层面的图案保真判据** ⇒
  图案复合只能由上述不变量与合成测试守住。
  （**2026-10-21 更正**：U-13 **已解决**，`460` 单位为弧度，已由 `cad-domain` 的方向回读测试覆盖
  “单位”与“随几何旋转”两项；**仍无 vendor 渐变夹具**，故不宣称真实文件往返。）

### P6b（`HATCH` 创建：预选闭合实体，提交 6dbf536）

- **命令面**：`CommandId::Hatch` 贯通（`COMMAND_TABLE` 选择子 `HATCH`，`EXPECTED_COMMAND_COUNT` **67 → 68**）。
- **形态 = 预选**：`DrawToolKind::Hatch` 用 `arity() = Exact(0)` + `requires_selection() = true`（与 `ARRAY`/`MOVE` 同型），
  `picked_step_count() = 0` ⇒ **未引入新的捕获状态机**（ADR 原标注的"最大实现风险"确实消失）。
- **参数**：走**唯一** `ToolParameter` 通道（不新增第二套请求类型）——`hatchPatternType`（`0` 实心 / `1` 用户自定义）、
  `hatchSpacing`（有限且 **> 0**，默认 `1.0`）、`hatchAngle`（**通道用度**、域内弧度；有限、**不做取模**）、
  `hatchDouble`（标志）、`hatchStyle`（`0` Normal / `1` Outer / `2` Ignore）；每参数**具名校验**并列出可接受集合。
- **边界**：`Circle`、完整 `Ellipse`、**闭合** `Polyline`；每个实体成一环；**全部共面**（`dot(p,n) = elevation` 容差
  `1e-9·(1+|p|)`）；**圆/椭圆另须法向一致**（反向拒绝），折线只要求共面。
- **创建走 `HatchGeometry::new`** ⇒ A.3 的拒绝面（自交/相切/重复/部分重叠/未闭合/退化/非有限）为**整命令拒绝且不开事务**。
- **用户自定义图案**：与导入侧 `import_pattern` **同一条合成规则**（一行 `angle 0, offset (0,1)`、`scale = spacing`、`double`、
  实体级 `52 = angle`），图案名用不透明 `"USER"`（无图案表）；**实心**记 `pattern_type = Predefined` 以与已提交 SOLID 夹具一致。
- **其它**：边界实体**保留**（无关联性/`DELOBJ`）、种子为空、`pixel_size = None`、环 flags 仅 `POLYLINE`
  （`EXTERNAL` 不由我们推导；嵌套仍按几何重算）；插入走单一漏斗，`type_key`/`class_name` = **`AcDbHatch`**，
  一条事务/一步撤销/活动图层。

#### 与决策 D 的**更正**（实施中发现，必须写明）
1. **圆的环是 4 段精确圆弧，不是 ADR 原写的"两点带 bulge"**。原因：**已提交的校验器**要求环 **≥3 个顶点**、
   且 `ring_degenerate` 按**弦**测面积 ⇒ 两点环必然被判退化。故圆用**四个象限顶点 + `bulge = tan(π/8)`**（四段精确
   四分之一弧），**仍是精确圆**、**仍导出为精确的 bulge 折线边界**；`source_edges = None`。
   把校验器改成 bulge 感知（含 `rings_touch`/`loop_depths`）是**大得多且未经复核的语义改动**，本相不做。
2. **椭圆按 64/圈采样**（真椭圆不是圆弧，用 bulge 会是**假**几何）；**椭圆弧不是闭合环 ⇒ 拒绝**。
3. **折线**须 `closed == true` 且 ≥3 顶点；3D 折线共面时接受；其自身 2D 点与 bulge 保留。
4. **声明局限**：创建校验基于**弦**，两段 bulge 环的**弦之间**相切可能漏检（"精确圆接触但其内接正方形相切"会被拒绝并有测试）。
5. **参数单位**：通道**度**、域**弧度**——与 P5e 的度/弧度事故同类陷阱，此处显式分层并在测试中钉住。
6. **未实现项**（与决策 D 一致，且 UI 不得暗示）：~~内部点求边界~~（**2026-11-01 已兑现**：`HATCHPOINT` v1，见文末补注；LINE/ARC 自动发现仍未）、LINE 拼接、~~预定义图案名~~（**2026-10-26 已兑现**：三项转录表按 1 基索引；**2026-10-27 起**状态栏按名 chip，见决策 G 的 P6c 补注）、~~渐变创建~~（**2026-10-27 已兑现**：类型码 `3` 双色 LINEAR/SPHERICAL，四个 `Number` 参数，ACI-only；其 `SPHERICAL` 平色与 `76=1` 未查证约定仍声明）、关联性、
   **真正的填充预览**（`preview.rs` 对 `Hatch` 明确返回无预览链）。
7. **i18n**：P6b **+14 键**（`draw.kind.hatch`、`tool.draw.hatch`、`tool.prompt.hatch.{select_boundary,ready}`、
   `status.tool_parameter.hatch_{pattern_type,spacing,angle,double,style}` + `.hint`），两目录一致；总数 **534**。

#### 证据暴露并修复的一处**跨实体**缺陷（非 P6 设计项）

- P6b 的真机像素判读发现创建出的**圆形**填充周围有"四段大弧 + 四段过冲"，矩形填充没有 ⇒ 追到
  `append_bulge_arc` 把**弧心放在弦的顶侧**（应为另一侧，`u` 已含 bulge 符号）：**每条 bulge 弧被沿弦镜像**，
  半圆因 `cos(θ/2)=0` 而**恰好不变**，故既有测试全过。共同错误还在 **EXPLODE 弧恢复 / 拾取包围盒 / 对象捕捉**。
  另修"平面法向符号取自绕向"⇒ **XY 平行折线一律 `+Z`**。
- **影响所有 bulge 折线，含导入图纸**（法兰 6 处、`entities` 3 处，位移达弧半径 397% / 156 单位）⇒
  **渲染变化声明**见 `docs/compatibility.md` 的「bulge 弧显示」行与 `docs/handoff-archive/2026-10.md`
  的专节。修复提交 `b52066e`。
- **修复后的真机复验**（`…p6b-run3`，重建二进制，硬件 Vulkan / P620）：边界外环带 **0.0518 → 0.0000**、过冲
  **592 → 0 px**、越界射线 **109/537 → 0/537**、最远墨迹半径 **91.90 → 82.20 px**、边界迹 **720 单段 / 0 多段 / 0 空**；
  全局墨迹 **−3 501 px**（被移除的花瓣）。**填充分区逐区域计数未变** ⇒ 修复只动了边界迹，未动填充。

## 2026-11-01 补注：`HATCHPOINT`（内部点）v1

**不改写上面的历史决策**；本条只记录新增入口与仍然存在的上限。

- **入口是独立选择子**：`HATCHPOINT`（`CommandId::HatchPoint`，本项目选择子；AutoCAD 的 `HATCH` 在内部点提示里做同一件事）。
  `DrawToolKind::HatchPoint` 是 **1 点 arity**、`requires_selection = false`、不取实体 pick；`HATCH` 的预选形态不变。
  命令表 **97 → 98**（2026-11-01）。
- **发现算法（v1）**：在**可见的模型空间整实体**里只考虑 `Circle`、完整 `Ellipse`、闭合 `Polyline`；每个候选走
  与 `HATCH` **完全相同**的 `boundary_shapes → stitch_boundary → resolve_plane → boundary_loops` 管线，取**最小面积包含边界**，
  其余“环在它内部”的候选全部作为岛屿；最终环集再走一次同一管线并交给**唯一** `HatchGeometry::new` 校验器。
- **诚实拒绝**：种子非有限（`hatch.boundary_invalid`）、非模型空间（`Unsupported`）、点在任一候选边界上
  （`hatch.boundary_on_edge`）、没有可见闭合边界包含它（`hatch.boundary_not_found`）、两个包含边界面积相等
  （`hatch.boundary_ambiguous`）。所有拒绝都在开事务之前。
- **声明的 v1 上限**：`LINE`/`ARC` 边链**不自动发现**（仍用 `HATCH` 预选，既有 `stitch_boundary` 要求整个边集闭合、
  分支/残余边会拒绝，不能直接喂全图）；当前选择**不参与**候选；只认 `+Z` 且与种子同标高；仅模型空间；
  边界实体保留（无关联性）；已有“弦/ bulge”校验盲点照旧。
- **种子落库**：内部点按 HATCH 平面 OCS 2D 存进 `HatchGeometry::seeds`，导出为 DXF 组 98；不新增实体字段。
- **证据边界**：契约测试（`crates/cad-app/src/tests/drawing/hatch_internal_point.rs`，14 项）与
  `cad-app`/`cad-ui-core` 门禁；**无**真实 DWG、GUI、像素或 vendor 对照。`LINE`/`ARC` 自动发现、非 `+Z` 平面、
  图纸空间与关联性仍 **NOT IMPLEMENTED**。

## 2026-11-17 补注：边界弧绕向与图案行「已求值」（真文件调试，U-14）

**不改写上面的历史决策**；本条记录一次**真实 DWG 渲染调试**定位的两处导入缺陷、证据等级与仍然存在的上限。
触发文件：外部 AC1018 真图（工程地质纵断面，663 个 HATCH、2596 条边界路径、495 条椭圆弧边），参考图由用户提供
（AutoCAD 侧导出的 PNG）。**该文件不在仓库 `fixtures/`**（AGENTS.md：外部大样本放仓库外），因此本条的数字是
外部证据，不是仓库内的 golden。

### 症状

全部模型空间渲染坍缩成 5 个巨大彩色圆盘，纵断面本体只有约 1 px：`scan` 的图界是 **5528 × 10782** 单位，而断面本体
约 **1908 × 527**。根因有两条，各自独立、各自足够致命。

### B.7-1 顺时针边界弧的角度是**镜像存储**的（U-14）

- **现象**：7 条路径的椭圆弧边半径 600–2750，而相邻边界顶点只相距 5–30 单位（"平弧"）。我们的
  `directed_sweep` 对 `ccw=false` 且 `end > start` 的角度补整圈，把一条 **0.9°、27 单位弦**的边读成 **359°**；
  单个采样点落在离边界 3 500 单位处，`drawing.bounds()` 被这条弧撑爆，渲染视角随之坍缩。
- **真实规则**：边界弧的 `counter_clockwise` 不只是方向——**顺时针边的存储角度是补角**（`360° − θ`），
  即边实际从 `−start` 走到 `−end`。等价写法是"负号照在椭圆短轴上"（`minor = (maj.y, −maj.x)·ratio`）。
  因此扫角必须取**存储差值本身**（恒正，整圈只在 `start=0,end=2π` 时保留），方向由参数帧（镜像）表达，
  不能再对角度做整圈补正。
- **证据等级**（全部为"实践约定"，非 Autodesk 文档）：
  - ezdxf #175（<https://github.com/mozman/ezdxf/issues/175>）："DXF stores the clockwise oriented start- and end
    angles for HATCH arc- and ellipse edges as complementary angles (360 − angle)"，并且明确指出
    **"There's no official documentation on this"**；
  - GDAL `ogrdxf_hatch.cpp` 对顺时针边取负角；acadrust 自身 `count.rs`（`s = -1` 轴）与
    `explode/mod.rs`（`(-end, -start)`）都按镜像读；
  - acadrust #241 用 **44 个生产 DWG** 实测：镜像读法在 40 个文件里让边界边以 1e-7 接上，字面读法**44 个全部错开**
    （0.45 至 1.7e6 单位）；
  - 本文件自证：同一图案在 hatch 角 0° 与 90° 时，存储行角分别是 45° 与 135°、基点从 `(5.656, 0)` 变成
    `(0, 5.656)`（见 B.7-2），说明写盘侧确实把 52/41 烘进行里，顺时针弧同理。
- **决定**：在**导入边界**处按上述约定采样（`boundary_arc_sweep` / `boundary_arc_parameter`），不做"猜测式选读"：
  文档读法永远优先，绝不为了让边界闭合成环而逐边挑角度（那会把一条合法的 >180° 弧掰成补弧）。
- **仍然诚实报告**：每条路径采样后测量相邻边交点残差与闭合残差；残差超过**该路径自身对角线 10%** 时
  记 `import.hatch_boundary_gap` 诊断并把实测残差写进该 HATCH 的 `Partial` 理由（`hatch boundary edges do not
  join …`）。几何**照存储读数保留**，不丢边、不改读、不伪造闭合（ADR B6）。本文件命中该诊断的路径，
  是文件自身边界不闭合的部分，不是被"修正"的对象。
- **修复后**（同一台机器、同一 Vulkan P620、`cad-cli-tools render --width 1305 --height 501`）：图界
  **5528×10782 → 2010×1012**；最大 HATCH 绘制范围 **5 482 → 635 单位**；渲染输出从"5 个圆盘"变为完整的
  三幅断面 + 图例。

### B.7-2 存储的图案定义行是**已经求值**的（角度 + 比例已烘入）

- **现象**：修完 B.7-1 后断面可见，但图案几乎不可见——每条图案线的间距是 64 单位、方向 225°，而参考图同级地层
  是 8 单位、135°。原因：`HatchPattern::families()`（ADR A.1：行是**图案空间**，角度/比例在组合时应用）把
  文件里**已经烘过 52/41 的行**又变换了一次。
- **自证数据**（同一图案 `ZBM22`，`pattern_scale = 8`）：
  | hatch 角 | 存储行角 | 存储基点 | 存储偏移 | 存储 dash |
  |---|---|---|---|---|
  | 0° | 45° | (0, 0) / (5.656, 0) | (-5.6569, +5.6569) | 1.6 / −1.6 |
  | 90° | 135° | (0, 0) / (0, 5.656) | (-5.6569, −5.6569) | 1.6 / −1.6 |
  行角 = 45° + 52；基点 = 图案空间点乘 41 后按 52 旋转；偏移模长 8 = 1 × 41；dash 1.6 = 0.2 × 41。三项同时吻合，
  说明写盘侧存的是**求值后**的行。
- **决定**：导入时把文件行**反求值**回图案空间（`unevaluated_pattern_row`：`R(−52)·行/41`、dash ÷ 41、
  行角 − 52），于是 `families()` 组合出的正是文件里的数字；导出侧对应**求值落盘**（`dxf_pattern` 烘 52/41），
  所以"导入 → 导出"仍逐值复现文件（`a_pattern_hatch_round_trips…` 与 `…raw_rows…` 契约测试）。
  比例不可用（0/非有限）时按 1 处理并保持既有"原点不可写"的声明。
- **声明的副作用**：`HatchPatternLine` 的字段语义（图案空间）不变，但**导入值不再等于文件值**；凡是把
  `pattern.lines` 当"文件原值"读的代码/测试必须改看 `families()`。本次改了 4 处测试与 1 处导出。

### B.7-3 文本岛边界不再当作可见几何绘制

- 边界路径的 `92` 位 8（TEXTBOX）/128（TEXT_ISLAND）是**填充挖洞**，不是作者画出的边界：参考图在这些位置没有
  任何矩形，我们此前把每个环都画成闭合折线，于是断层里出现大量小方框。
- 决定：`display()` 与 `FILL OFF` 的边界half 都跳过文本岛/文本框环；`loops()` 仍保留它们，**填充奇偶、捕捉顶点、
  导出**都不变（`text_island_loops_are_not_drawn_but_still_cut_the_fill`）。其它环仍全量绘制（ADR A.3 未改）。

### B.7-4 同名风险面排查：`ARC` / `ELLIPSE` / 折线 bulge 不受影响

同一类"绝对角度 + 方向位"的记录只有 **HATCH 边界边**。三处相邻面各有不同的存储约定，**不得**把 B.7-1 的镜像规则
推广过去（`crates/cad-import-acadrust/src/tests/arc_sweep.rs` 是本条的回归钉子）：

| 载体 | 存储 | 方向如何确定 | 是否会重演本次缺陷 |
|---|---|---|---|
| HATCH 边界 `CircularArc`/`EllipticArc` | 绝对角度 + `73` ccw 位 | 顺时针**存补角**（B.7-1） | 会（已修） |
| `ARC` 实体 | 绝对角度，**无方向位** | 恒为 `start → end` **逆时针**，`normalize_sweep(end − start)` 取 `(0, 2π]` | 不会：没有位可"读反"，`start > end` 本就该走长弧（实测 `1.7 → 1.6` = 354°） |
| `ELLIPSE` 实体 | 参数 `start/end`，**无方向位** | 同上，恒逆时针（`DXF` 整椭圆即 `0 → 2π`） | 不会 |
| 折线 / HATCH 边界 `Polyline` 的 bulge | `bulge = tan(θ/4)`，**带符号** | 符号即方向，几何由 `θ = 4·atan(b)` 唯一确定 | 不会：没有"绝对角度绕过 0"的歧义，`θ` 最大只能到接近 ±2π 且由数值本身给出 |

- **三份 bulge 实现已对齐**（`cad-geometry::append_bulge_arc` 显示路径、`cad-domain::hatch_entity::geometry::push_bulge_points`、
  `cad-import-acadrust::hatch::push_bulge_segment`）：弦长 + bulge ⇒ `r = c/(2·sin(θ/2))`（带符号）、
  圆心 `mid + left·r·cos(θ/2)`；负 bulge 两者同样落在弦的另一侧。历史上"弧心放在弦顶侧"的镜像缺陷
  已于 2026-11-01 修复（本文件更早的补注），本轮复核未发现分歧。
- **实测（同一外部 AC1018 文件）**：`ARC` 实体 **2** 个（半径 2.535、张角 1.438 rad，正常）；`ELLIPSE` 实体 **0** 个；
  折线 bulge 段 **563** 段，最大 `|θ| = 1.54 rad`、无退化、无近整圈；其中 4 段是"大半径平弧"（r 113–195、弦 30–36、
  θ 0.19–0.26），因为 bulge 带符号，它们**不存在** B.7-1 的读法歧义。
- **残余（未查证，未改）**：`normalize_sweep(0) = 2π`，即一条 `start == end` 的 `ARC` 会被画成整圆而不是零长弧。
  这种记录在 AutoCAD 侧的行为**未验证**，因此保持现状并在此声明，不做猜测性修改。

### U-14（新增未查证项）

**`ratio ≈ 1` 时椭圆短轴手性（等价地：椭圆参数符号）无法从记录本身确定**；顺时针补角约定是**实践约定**，
不是 Autodesk 文档（DXF/DWG 参考只写 "start angle" / "is counterclockwise"）。本次只在**一个外部 AC1018 文件**
上验证，且**未**用第二个实现（LibreDWG/ODA）逐字段比对、**未**做 AutoCAD 另存 DXF 的对照。
`ratio ≠ 1` 时"DXF 50/51 是圆角、DWG 是椭圆参数"这一点同样 **NOT VERIFIED**（本文件全部 `ratio = 1`，无法区分）。

### 证据边界（本轮实际运行 / 未运行）

- **已运行**：`cad-import-acadrust` 501 lib 测试（新增 `tests/hatch_boundary_arcs.rs` 5 项）、
  `cad-domain` 162、`cad-representation` 357、`cad-app` 3012、`cad-spatial` 55、`cad-scene` 71、
  `cad-measure` 59；`cargo fmt --all -- --check`、`clippy -D warnings`（三个改动 crate）、
  `check-architecture.py`、`check-file-size.py`、`check-i18n.py`。
- **已运行（外部文件，非仓库 golden）**：`cad-cli-tools scan/render` 于
  `test-hatch-render.dwg`（真 Vulkan / P620），修复前后同参数对比；**视觉判读**由本会话对着用户提供的参考 PNG 完成。
- **NOT RUN**：全量 `cargo test --workspace`、GUI 宿主（`yacr`）真机运行、`linux-host-smoke`、
  Windows/macOS/Android/Web 任一载体、像素/性能结论。参考 PNG 与我们的渲染**不是同视口**（参考图是裁切视图，
  本图含下方钻孔柱状文本），因此**未**做逐像素或 golden 比对。
