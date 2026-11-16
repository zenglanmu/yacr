# 引入 SMLib（solidcore）的收益、代码改动与 SKP/STL 映射方案

> 性质：**分析与实施方案**，不是已验收结论。决策记录见
> [`ADR-0036`](adr/0036-3d-body-model-and-solid-service.md)；ACIS 现状见
> [`kernel-acis.md`](kernel-acis.md)；逐格式能力见 [`compatibility.md`](compatibility.md)。
>
> 基线：`f3fc3d6f`（3D body P0 已提交）。`solidcore` **尚未进入任何 Cargo.toml**。
>
> 术语先对齐：
> - **SMLib** = NVIDIA `NVIDIA-Omniverse/usd-brep` 里的 `source/SMLib`（Apache-2.0，商业
>   SMLib/NLib 的开源投放），**不是** Botsch/Kobbelt 的学术 SMLib。
> - **solidcore** = SMLib 的纯 Rust 移植（crates.io 1.0.1，2026-10-30；`solidcraft` 的只读
>   镜像；API 自陈 experimental）。本方案选它作为 BRep 内核。
> - **BRep** = 精确边界表示（Face/Loop/Edge/Vertex + 支撑曲面）。三角网格**不是** BRep。

---

## 1. 一句话结论

| 问题 | 结论 |
|---|---|
| 引入 solidcore 的收益 | 第一次获得**可以创建和编辑实体**的能力：基本体、布尔、圆角/倒角、抽壳/偏置、扫掠、healing、精确质量属性；并把"导入的 ACIS 实体"与"新建实体"统一到同一条 `Body3d` + `BrepData` 通道。 |
| 代价 | 新增一个大型纯 Rust 依赖和一层独立的转换/服务 crate；无 STEP/IGES、无精确算术谓词、无取消钩子、无公开 BRep→网格接口；wasm 栈与线程需要实测。 |
| SKP / STL 能变成实体吗 | **不能。** 两者都是三角网格格式，没有 BRep。正确映射是继续进入 `SemanticGeometry::Mesh`（各自 `type_key`），**不要**伪造 `Body3d`/`BodyKind::Solid`。要"可编辑"只能做**网格布尔**（产物仍是网格）。 |
| STEP 能变成实体吗 | **能，而且不需要 OCCT。** `cadmpeg-ir` 0.6.12 已经解出精确拓扑（Body/Region/Shell/Face/Loop/EdgeUse/Pcurve + 解析/NURBS 曲面），只是当前适配器只取三角面。写一个 `cadmpeg-ir → BrepData` lifter 即可让 STEP 进入 `Body3d`，再交给 solidcore 编辑。详见 §6。 |

---

## 2. 现在有什么、缺什么

`f3fc3d6f` 之后：

- `cad-domain::SemanticGeometry::Body3d(Box<Body3d>)` 已存在：
  `BodyKind{Wire,Sheet,Solid,Unknown}` + `BodyExact{Exact|Unavailable}` +
  `BodyDisplay{mesh,edges,tolerance,precision,completeness}`（`crates/cad-domain/src/body.rs`）。
- 中性 B-Rep（`BrepData`/`BrepFace`/`BrepSurface`…）已从 `cad-kernel-adapter` 下沉到
  `cad-domain`，adapter 用重导出保持路径。
- ACIS 导入已发出 `Body3d` 并保留 `exact`（`crates/cad-import-acadrust/src/support.rs`）。
- `cad-db` 强校验 `Solid ⇒ Exact 且 B-Rep 非空`；非均匀缩放降级为 `Unavailable`。
- 但：
  - `cad-kernel-adapter` 只会 **BRep → 网格**（`BrepTessellator`），没有创建/布尔/圆角；
  - `BrepSurface` 只有 `Plane|Sphere|Torus|Cylinder|Cone|Unsupported`，**没有 NURBS 曲面**；
  - `UNION`/`SUBTRACT`/`INTERSECT`/`BOX` 等已由 C1/C2 接成真实命令（见 §4.3）；
    `EXTRUDE`/`REVOLVE`/`SWEEP`/`LOFT`/`FILEDGE` 等仍是具名拒绝；
  - 仓库里**没有任何**网格布尔/修复/mesh→BRep 依赖（`manifold3d`/`baby_shark`/`parry3d`/
    `truck`/`opencascade` 均未出现）。

**2026-11-17 更新（命令侧）**：`MASSPROP` 已对精确 `Body3d{Solid, Exact}` 接线（C3a，见 §4.3）；
`face_sources` 已由 `BrepTessellator` 写入 `brep:face:<shell>/<face>`（C3b）；`VSCURRENT` 已是
会话级 3D body 视觉样式（C3c，默认 SHADED，与 AutoCAD 的 2D-wireframe 初始值有意偏离）；
**剩余 C3 项：无**。曲面 lift/曲面布尔、圆角/倒角/抽壳仍是 P2。

---

## 3. 引入 solidcore 的收益

### 3.1 能力收益（API 存在 ≠ 已验收）

solidcore 1.0.1 的公开 API 覆盖（详见 ADR-0036 与 docs.rs）：

| 能力 | solidcore 入口（示例） | 当前仓库 |
|---|---|---|
| 基本体 | `create_box`/`create_sphere`/`create_cylinder`/`create_cone`/`create_torus`/`create_pyramid` | 全部拒绝 |
| 拉伸/旋转/扫掠 | `sweep` 模块、`primitive_creation` | 全部拒绝 |
| 布尔 | `boolean_union`/`boolean_difference`/`boolean_intersection`/`boolean_merge`/`non_manifold_boolean` | 无模型 |
| 圆角/倒角 | `fillet_edges`/`circular_fillet`/`variable_radius_fillet`/`chamfer_fillet`/`remove_fillet` | 只有 2D 线线 |
| 抽壳/偏置 | `api_shell_brep_full`/`offset_brep` | 无 |
| 精确质量属性 | `brep_mass_properties`/`brep_volume`/`brep_area` | body 显式拒绝 |
| 拓扑遍历 | `Brep::{faces,edges,vertices,loops,shells,regions}` + 公开字段 | 只有中性子集 |
| 修复 | `heal` 模块 | 无 |
| NURBS | `SurfaceKind::BSpline(BSplineSurface)`，权重在 `CPoint.w` | 无 NURBS 面 |

### 3.2 结构性收益

1. **从"只能看"到"能建模"**：这是产品能力的阶跃，不是性能优化。
2. **一条通道**：导入的 ACIS `Body3d.exact` 与新建实体都是 `BrepData`，显示、拾取、
   质量属性、导出都走同一套；不再需要为"内核创建"再造一个域类型。
3. **诚实性更好**：solidcore 返回 `Result` + 显式错误；我们可以把"未移植/不支持"
   映射成 `Unsupported{reason}`，而不是今天那样整条命令被拒绝或把网格体积冒充实体。
4. **`face_sources` 解冻**：`BrepFace.id` 可以填充 `Mesh.face_sources`，让已有的
   子面拾取/高亮管线第一次真正有输入。
5. **wasm 可能性**：纯 Rust、`#![forbid(unsafe_code)]`、`Send + Sync`、CI 有
   `wasm32-unknown-unknown` check——这是 `truck`/OCCT 之外唯一能保持"一棵 React 树"的候选。

### 3.3 不能带来的（必须写进能力表，不能宣传）

- **STEP/IGES**：solidcore 没有任何 ISO 10303 代码（ADR-0033 已记录因此不作为 STEP 读取
  路径）。STEP 仍只能由 `cadmpeg-codec-step` 离散显示；要精确 STEP 实体，只有 OCCT。
- **精确算术谓词**：SMLib 靠 `SmTol` 容差模型，不是 CGAL/GMP/Shewchuk，鲁棒性是容差驱动。
- **网格处理**：没有 half-edge、平滑、细分、参数化、曲率、测地线、谱分析。
- **SAT/SAB 写出**：solidcore 只有原生 `.smb/.smp`，不能写 ACIS。**P3a（2026-11-17）已实现一个
  受限子集**：DXF 导出对**单壳全平面直线**的精确体写回 `3DSOLID` 的 ACIS **SAT 文本**（自研
  `SatDocument` 构建，非 solidcore），其余 body 按名拒绝并退回 POLYFACE；**SAB** 与 **DWG
  `3DSOLID`** 仍不写（P3b）。

### 3.4 成本与风险

| 风险 | 影响 | 处置 |
|---|---|---|
| API experimental、镜像仓每日变动 | 版本行为漂移 | 固定 **git revision**（不用 crates.io 1.0.1），只允许 `cad-kernel-solidcore` 命名 solidcore |
| 无公开 BRep→网格 | 显示链路接不上 | 用现有 `BrepTessellator` 对导出的 `BrepData` 剖分；NURBS 面在 P2 前会 `Unsupported` |
| 无取消钩子 | 长布尔无法中断 | 调用放 worker；提交前后检查 `cancelled()`；取消后到达的结果 `aborted` 并丢弃 |
| 依赖大、wasm 栈需要更大 | Web/Android 可能不可用 | 按载体报告 `modeler available/unavailable`；不可用即稳定拒绝 `unsupported-runtime: solid-modeler` |
| 许可证 | Apache-2.0，兼容 | 记录 `THIRD_PARTY_NOTICES.md`；无 LGPL 传染 |
| 结果正确性未知 | 可能产生坏实体 | 每次落库前跑 `cad-db::validate_body`；内核返回 `Failed` 时绝不落库 |

---

## 4. 对现有代码库的改动

### 4.1 已完成（P0，`f3fc3d6f`）

| 层 | 改动 |
|---|---|
| `cad-domain` | `Body3d`/`BodyKind`/`BodyExact`/`BodyDisplay`；`BrepData` 等中性数据下沉；`SemanticGeometry::Body3d` |
| `cad-db` | `validate_body`（Solid⇒Exact、边线、容差）、bounds、相似变换复合 `BrepPlacement::composed_with` |
| `cad-import-acadrust` | ACIS 发出 `Body3d`，保留 `exact`/`edges`/`tolerance`；导出：P0 只写 `POLYFACE`，**P3a 起**单壳全平面直线体的精确 B-Rep 写回 `3DSOLID`（SAT 文本），其余仍 `POLYFACE` 并记 `acis.*` |
| `cad-representation`/`cad-scene` | body → `Mesh` / `LineSegments` / `Missing`；精度与来源透传 |
| `cad-spatial`/`cad-measure`/`cad-app` | 独立 pick family `body`；无 snap；`MASSPROP` 显式拒绝（P0 时；C3a 已改为精确测量，见 §4.3）；`select_filter` 用 `type_key` |
| 文档 | ADR-0036、`kernel-acis.md`、`compatibility.md`、`dxf-entity-coverage.md`、`handoff.md` |

### 4.2 P0.1（已完成）

- `DisplayPrimitive::MeshWithStoredEdges { mesh, edges }`（**不要**复用文字 glyph 的
  `MeshWithoutEdgeOverlay`）。
- `cad-scene` 对该图元直接用 `BodyDisplay.edges` 填 `RenderBatch.edges`，不再逐三角形重推；
  顺带去掉内部重复边。
- 需要同步所有 `DisplayPrimitive` 消费者（`cad-app`、host、CLI 对等测试）。
- 理由：P0 已经把边线存下来了，但现在仍在用三角形重推，等于白白多算且显示质量更差。
- 状态（2026-11-16）：表示/场景/渲染器与消费者均已接线，契约与单元测试在 Linux 上运行；
  GPU/像素与其它平台 **NOT VERIFIED**。

### 4.3 P1：接入内核（最小可用建模）

- 状态（2026-11-17）：**B2 已落地、命令侧 C1/C2/C3 已落地**；曲面 lift/曲面布尔属 P2。
  `cad-kernel-solidcore` 存在并实现 `SolidModeler`；**平面体**的创建/布尔/质量属性/相似变换可用；
  曲面 primitives 可创建、可显示、可导出，但其布尔与质量属性**显式拒绝**。
  **阶段 C1**：`BOX`/`WEDGE`/`PYRAMID`/`CYLINDER`/`SPHERE`/`CONE`/`TORUS` 七条命令由具名拒绝改为
  真实创建（显式点 → 模型器 → 一条 `AcDb3dSolid` 实体），模型器由载体注入
  （`Application::set_solid_modeler`；**只有 Linux `native-below` 注入**，Linux `dom-canvas`
  （X11/XWayland，CAD 会话在 WebView 里是 wasm）、Web、Windows、macOS、Android 都是 `None`，
  七条命令报 `unsupported-runtime: solid-modeler`；这些载体的运行时行为 NOT VERIFIED）。
  显示剖分用与导入共享的**形状相对容差**（1e-3 × 包围盒对角线），平面体 `Complete`、
  曲面体 `Partial`（“离散网格、缩放不细化”）。图元同步跑在命令线程（**C1 豁免**，工作有界）；
  C2 布尔必须 worker + 取消探针，不得沿用。
  **阶段 C2**：`UNION`/`SUBTRACT`/`INTERSECT` 是真实后台布尔任务（ADR-0036 §3 C2）。
  **阶段 C3（2026-11-17 完成）**：**C3a** `MASSPROP` 已对精确 `Body3d{Solid, Exact}` 接线
  （注入模型器同步测量，density 1.0、请求相对精度 1e-6、显示网格不参与）；**C3b**
  `face_sources` 已由 `BrepTessellator` 写入 `brep:face:<shell>/<face>`；**C3c** `VSCURRENT`
  已是会话级 3D body 视觉样式（默认 SHADED）。**剩余 C3 项：无**；曲面 lift 仍是 **P2**。
  **仍 NOT IMPLEMENTED**：AutoCAD 的交互式尺寸提示、曲面 lift/曲面布尔（P2）、圆角/倒角/抽壳（P2）。
  证据只有 Linux 契约/单元（真实内核，无 mock）；其它载体 NOT VERIFIED。详见 ADR-0036 §7。
- **拒绝边界（能力是 family 级，不是逐操作数）**：`capabilities().booleans` /
  `mass_properties` 为 true 只表示"这一族有实现"，每次调用仍可能对单个操作数返回
  `Unsupported`，调用方必须处理：
  - 曲面 body（非 `Plane` 面或非 `Line` 边）→ `solid.lift_requires_planar`；
  - **结构不一致**（开放 sheet、每条边只用一次、反向面/重复面、环有缺口）→
    `solid.lift_unsupported`（这类输入若放行会给出看似合理的面积/体积，实测 40 vs 52、
    36 vs 24，所以 lift 后做结构一致性检查）；
  - **空腔（void region）的结果在布尔本身就被拒绝**（`solid.lift_unsupported`）；
    **多壳结果**（例如不相交的两个体 `UNION`）**会作为 body 成功返回**（2 个 shell），
    直到**下一次 lift**（质量属性 / 再次布尔）才被拒绝——所以**质量属性要求恰好一个闭合壳**
    （空腔/多壳都不算，不把两块体积相加）；
  - 不可烘焙的 placement/平面 frame → `solid.placement_not_supported`；非相似变换 →
    `solid.nonuniform_transform`。

1. **新 crate `cad-kernel-solidcore`**（已实现）
   - 唯一允许命名 `solidcore` 的包；其它 `cad-*` 被 `check-architecture.py` 拒绝
     （形状对齐 `cad-import-acadrust` ↔ `acadrust`）。
   - 固定 git revision `55ae49ef919b606bf77901df626ed9bb543e5212`（`solidcore-ops` +
     `solidcore-record`，Apache-2.0，目标表 `cfg(not(wasm32))`，无 Cargo patch）；
     `Registration` 记 `yacr.kernel.solidcore`，上游版本串 `1.0.1+55ae49ef919b` 由
     `UPSTREAM_VERSION` 与 capabilities 声明（上游未 bump 该串）。
   - 只做三件事：`SolidModeler` 实现、`BrepData ↔ record` 双向转换（平面面 + `Line` 环、
     `Plane/PlaneThrough/Cylinder/Cone/Sphere/Torus` + `Line`/整圆）、能力报告。
   - 转换器有独立测试：单位（原样透传，记录按 mm 语义）、placement（相似 placement 烘焙；
     镜像拒绝）、面/环/边 canonical 顺序、`forward` 方向、顶点/边按精确位去重、容差、
     pcurve（必须是面 `(u, v)` 的精确一次 spline，否则面积积分返回 0）。
2. **接缝（`cad-kernel-adapter`）**（已实现，B1）
   ```rust
   pub trait SolidModeler: Send + Sync {
       fn registration(&self) -> Registration;
       fn capabilities(&self) -> ModelerCapabilities;
       fn execute(&self, request: &SolidRequest<'_>,
                  cancelled: &(dyn Fn() -> bool + Sync)) -> CadResult<SolidResult>;
   }
   pub enum SolidOperation<'a> {
       Primitive { spec: PrimitiveSpec },
       Boolean { op: BooleanOp, left: &'a BrepData, right: &'a BrepData },
       Transform { brep: &'a BrepData, transform: &'a Transform3 },
       MassProperties { brep: &'a BrepData, density: f64 },
   }
   ```
   结果 `SolidOutcome { Success(Body { brep, diagnostics } | Mass(..)) | Unsupported { reason } |
   Failed { diagnostics } }`：**没有** `aborted` 标志、没有"空成功"；剖分仍由既有
   `BrepTessellator` 在模型器之外完成（模型器不做显示网格）。
   身份是 `cad-db` 的 `EntityId` + `ObjectId.revision`；跨接缝只传 `BrepData` 快照。
3. **命令接入**（`cad-app`，C1/C2/C3 **已实现**）
   - **C1（本轮）**：`BOX/CYLINDER/SPHERE/CONE/TORUS/WEDGE/PYRAMID` 从拒绝改成创建：
     `CommandPayload::Points` → `PrimitiveSpec`（显式点契约见
     `crates/cad-app/src/app_drawing/solids.rs` 模块文档）→ `execute_solid` → 中性 B-Rep →
     `tessellate_exchange`（显示缓存）→ `commit_new_entity_typed(.., "AcDb3dSolid", ..)`
     （一条事务/一步撤销）。模型器由载体注入（`Application::set_solid_modeler`），当前只有
     Linux `native-below` 注入；无模型器载体报 `unsupported-runtime: solid-modeler`。
     剖分容差是共享的形状相对规则（`cad_kernel_adapter::shape_relative_tolerance`），
     曲面体的 `BodyDisplay.completeness` 是 `Partial([discrete_mesh_reason])`。
   - **C2（已实现）**：`UNION/SUBTRACT/INTERSECT` 作用于 `Body3d(Solid)`，是真实后台任务；
     替换操作数 + 插入结果在一条事务/一条 undo 记录里（细节见 ADR-0036 的 C2 段）。
   - **C3（2026-11-17 完成，剩余项无）**：
     - **C3a `MASSPROP`**：精确 body 经注入模型器同步报体积/表面积/质心/惯性张量；
     - **C3b `face_sources`**：`BrepTessellator` 用 `BrepFace.id` 写 `brep:face:<shell>/<face>`；
     - **C3c `VSCURRENT`**：会话级 3D body 视觉样式（见下条 6）。
   - `3DSOLID`/`BODY` 是**实体类型键**（模型器 `Registration::entity_types` 声明），不是 AutoCAD
     命令：不新增同名 `CommandId`。
   - 逐命令契约测试 + `compatibility.md` 行；i18n 两个 catalog 同步。
4. **质量属性**（模型器已实现；命令 **C3a 已接线**）：`MASSPROP` 选择**全部**为精确
   `Body3d{Solid, Exact}` 时，逐体经注入模型器**同步**报体积/表面积/质心/惯性
   （density 1.0、请求相对精度 1e-6，报告明写“估计、不是已证明的界”），**显示网格永不积分**。
   稳定拒绝：`massprop.body_modeler_unavailable` / `body_not_liftable`（曲面 P1 →
   `solid.lift_requires_planar`）/ `body_kernel_failed` / `body_not_exact` / `body_not_solid` /
   `mixed_selection`（与平面区域混选）/ `kernel_busy` / `body_not_positive_volume`；body 面子元素
   引用解析到所属 body。模型器返回
   `MassAccuracy::Requested { relative }`（估计，不是证明），不得标 `Analytic`。
   成功条件是**恰好一个闭合壳**（空腔/多壳 → `Unsupported` + `solid.lift_unsupported`）；
   内核"不确定该 region 是否材料"时返回 `Failed(solid.unsure_region)`，不是 `Unsupported`。
5. **`face_sources`**（**C3b 已实现**）：`BrepTessellator` 用 `BrepFace.id` 为每个显示三角形写
   `Mesh.face_sources`，键 `brep:face:<shell>/<face>`，`topology_revision` 是占位 `Revision(0)`；
   子面拾取与子面高亮复用 mesh 路径（缺失/过短/`None` 报 `mesh-face-source*`）。`cad-db` 只校验
   列表为空或与三角形等长。
6. **`VSCURRENT`**（**C3c 已实现**）：会话级 3D body 视觉样式，**默认 SHADED**（与 AutoCAD 的
   2D-wireframe 初始值有意偏离，保留 token 前的填充网格显示）；wireframe 落成
   `DisplayPrimitive::LineSegments` → `RenderTopology::Lines`（`MeshEdges` 仍未被本路径使用），
   不写 `cad-db`、不升修订、不入历史。

### 4.4 P2：曲面、圆角与 NURBS

- `BrepSurface::Nurbs { degree_u/v, knots, ctrl, weights }`（有理 B 样条）。
- `SURFACE` 家族：`PLANESURF`/`EXTRUDEDSURF`/`LOFTEDSURF`/`REVOLVEDSURF`/`SWEPTSURF`
  → `BodyKind::Sheet`；是否保留 `exact` 取决于能否从 ACIS/SolidWorks 读入。
- 圆角/倒角/抽壳：`SolidRequest::Fillet/Shell`；产物是 B-spline 面，必须能进
  `BrepSurface::Nurbs`，否则只能显示网格并显式 `Unverified`。
- NURBS 显示剖分优先走 `SolidModeler::Tessellate`（内核已有评估器），避免在域内再写第二套。

### 4.5 P3：ACIS 写出

- **P3a（已实现，2026-11-17）**：`crates/cad-import-acadrust/src/export/acis.rs` 用中性 `BrepData`
  构建 `SatDocument`（body→lump→shell→face→loop→coedge→edge→vertex→point；几何边跨面焊接、
  每条边两个 coedge、平面框架归一化并校验正交、外环绕向与面朝向一致），`DxfWriter` 写成
  `3DSOLID` 的内联 SAT 文本。子集：单壳、全平面、直线边界、真相似放置（无剪切/镜像）。
  拒绝码：`acis.curved_face_not_written` / `acis.non_line_curve_not_written` /
  `acis.no_shell_not_written` / `acis.multi_shell_not_written` / `acis.placement_not_written` /
  `acis.invalid_topology_not_written`，退回显示缓存（POLYFACE / LINE / POLYLINE3D）。
- **DWG 与 SAB 仍显式拒绝**：任一体分支在 DWG 都记 `acis.dwg_3dsolid_unsupported`；不写 SAB。
- **版本**：文件含任一 `3DSOLID`（含块定义内）即整份 DXF 按 AC1024 写出（内联 SAT、避免
  AC1027+ 的 `ACDSDATA` SAB），记 `export.note.acis_sat_text_version`；这是文档级约束。
- 证据：自写→acadrust 解析→自读往返（盒/楔/棱锥/带孔面，几何 ≤1e-9·对角线、体积 ≤0.1%）+
  真实 solidcore BOX 的 DXF 存盘/再导入（精确体、体积 1±1e-3）。**无第三方 CAD 读取器证据**，
  无 vendor/像素/非 Linux 证据；P3b（SAB、DWG 3DSOLID、曲面/NURBS 面）仍未做。

### 4.6 载体影响

| 载体 | P1 预期 | 证据要求 |
|---|---|---|
| Linux Wayland（`native-below`） | 主目标，内核可用 | 实际运行 + 契约测试 |
| Windows / macOS | 纯 Rust 依赖，应可编译 | 只有编译证据，运行时 **NOT VERIFIED** |
| Android | NDK 交叉编译纯 Rust 可行 | 需实测；不可用则显式拒绝 |
| Web（`dom-canvas`） | `cargo check --target wasm32` 是上游证据 | 必须实测栈/性能；不可用则 `unsupported-runtime: solid-modeler` |

---

## 5. SKP / STL 如何映射到 `cad-db`

### 5.1 先判类型：它们不是 BRep

| 格式 | 内容 | 有拓扑/精确曲面吗 | 能不能当 `Body3d` |
|---|---|---|---|
| STL（ASCII/二进制） | 三角面片 + 逐面法向 | 没有；连闭合性都不保证 | **不能** |
| SketchUp `.skp` | 三角化后的面 + 组件实例变换 + 材质/tag | 没有（`skpwright` 只给三角网格） | **不能** |
| SolidWorks `.stl` | 就是普通 STL，没有 SolidWorks 元数据 | 没有 | **不能** |
| SolidWorks `.sldprt` / STEP | 精确 BRep | 有 | 可以（但需要 STEP/Parasolid 读取器） |

把 STL/SKP 塞进 `BodyKind::Solid` 会直接违反 `cad-db` 的 `Solid ⇒ Exact` 不变量；
塞进 `Sheet` 又会让 UI 误以为它有裁剪曲面。两者都是"编造能力"。

### 5.2 现有映射（已实现，事实）

**STL**（`crates/cad-import-stl`）：

| 项 | 现状 |
|---|---|
| 几何 | 一个文件 → 一个 `DbEntity`，`SemanticGeometry::Mesh` |
| `type_key` | `StlMesh` |
| 单位 | `UnitContext::drawing_units()`——STL 没有单位字段，**不假设 mm** |
| 朝向 | 不重映射，顶点原样入 `f64` |
| 法向 | STL 逐面 → 展开顶点成逐顶点（平面着色）；零/非有限法向由几何重算并记 `import.stl.normal_recomputed` |
| 退化面 | 丢弃 + `import.stl.degenerate_triangle`，整体 `Partial` |
| 闭合性 | **完全不检查**（仓库唯一的 `count_open_edges` 只在 ACIS 测试里用） |
| 完整性 | 干净文件 `Complete`；丢面则 `Partial` |
| 能力 | `read/render/pick = Verified`，`measure = Unsupported`（`MeshImport` 固定） |

**SKP**（`crates/cad-import-skp`）：

| 项 | 现状 |
|---|---|
| 几何 | 每个（placement × 生效材质）组 → 一个 `Mesh`；**实例已烘焙到世界坐标**，不用 `Insert`/块 |
| `type_key` | `SkpPreviewMesh` |
| 单位 | 米、`+Z` up，`import.skp.units_metres` |
| 层级 | tag **扁平化到 layer 0**，`import.skp.layers_flattened` |
| 材质 | 逐实体 `EntityColor::Explicit` 或 `ByLayer`；纹理只取平均色，`import.skp.textures_not_sampled` |
| 透明度 | `0 ≤ opacity < 1` 时 `Explicit`，否则 `ByLayer` |
| 可见性 | 遵守隐藏实例/隐藏 tag/隐藏面，`import.skp.hidden_pruned` |
| 未导入 | 边/曲线、纹理图像与 UV、场景/页面、剖切面、标注、动态组件属性；**只有边几何的文件显式拒绝** |
| 完整性 | 每个成功导入恒 `Partial`（`import.skp.preview_geometry`） |
| 能力 | 受 release 上限约束；测量恒 `Unsupported` |
| 许可 | `skpwright = 0.6.0` 是 **GPL-3.0-only**；本仓库 AGPL-3.0 兼容，但任何非 AGPL 分发都会被阻断 |

### 5.3 目标映射矩阵

| 源数据 | 现在 | 建议落点 | 说明 |
|---|---|---|---|
| STL 三角面 | `Mesh`/`StlMesh` | **保持** | 没有 BRep；不要升级成 `Body3d` |
| STL 闭合性 | 不检查 | `Mesh` + 新诊断 `import.stl.open_shell`（仅信息） | 用现有 `count_open_edges`；仍然不是实体，但用户知道它不闭合 |
| SKP 三角面 | `Mesh`/`SkpPreviewMesh` | **保持** | 同上 |
| SKP 组件实例 | 烘焙 | **P-SKP-1**：`BlockDefinition` + `Insert` | 保留组件身份、可移动/旋转实例、数据库更小；需要嵌套变换与选择语义 |
| SKP tag | 扁平化到 layer 0 | **P-SKP-2**：tag → `Layer` | 已有诊断；改后可按图层显隐/过滤 |
| SKP 材质/透明度 | 逐实体颜色 | 保持 | `Mesh.colors` 目前未用；纹理平均色已是诚实降级 |
| SKP 边/曲线 | 拒绝 | **P-SKP-3**：`LineSegments` | 补齐 "wireframe" 观感；纯边文件不再只能拒绝 |
| STL/SKP 的"实体编辑" | 无 | **P-MESH-1**（可选）：独立的网格体类别 + 网格布尔 | 产物仍是网格；**不要**复用 `BodyKind::Solid` |

### 5.4 如果一定要"编辑 STL/SKP"

三条路，按诚实度排序：

**A. 保持网格（推荐，零新依赖）**

- 能做的：显示、拾取、面积、图层/材质、导出。
- 不能做的：布尔、圆角、体积、精确质量属性。
- 这是今天的行为；只要 UI 不写"实体"就没有过度承诺。

**B. 网格布尔（可选，P-MESH-1）**

- 依赖 `manifold3d`（Apache-2.0/MIT，稳健网格 CSG）或 `baby_shark`（表面重建/重网格）。
- 需要新的域概念，例如：
  ```rust
  pub enum MeshTopology { OpenShell, ClosedShell, NonManifold }
  pub struct MeshBody { pub topology: MeshTopology, pub mesh: Mesh, pub repair: RepairStatus }
  ```
- 布尔结果仍是 `Mesh`；**不得**写成 `Body3d.exact`，也不得让 `MASSPROP` 报精确体积。
- 适用于 3D 打印/可视化，不适用于工程精确编辑。

**C. mesh→BRep 重建（不推荐）**

- 从三角面拟合平面/圆柱/NURBS，误差不可控、拓扑身份不可靠；工程件上会制造"看起来能编辑、
  实际尺寸错"的结果，违反"不得伪造成功"。
- 如果确实需要 SolidWorks 精确几何，正确做法不是重建，而是**换输入格式**。

### 5.5 SolidWorks 专门说明

- `.stl` 只是三角网格，**不带** SolidWorks 特征树、装配关系、精确曲面。
- 要精确实体，SolidWorks 侧应导出：
  - **STEP AP242/AP214**（首选）：**可行，且不必引入 OCCT** —— `cadmpeg-ir` 已经暴露精确
    B-Rep，缺的是把它 lift 成 `BrepData` 的适配器（§6）。
  - **Parasolid `.x_t`**：仓库完全不支持；需要专门的 Parasolid 读取器，工作量远大于
    solidcore 接入。
- 因此在 P1 之后，"SolidWorks → 可编辑实体"是 **STEP BRep lifter** 问题（§6），
  不是 solidcore 问题。solidcore 只解决"从零创建/编辑实体"。

### 5.6 接线点（改映射时只需动三处）

1. **发射**：`cad-import-skp/src/lib.rs` / `cad-import-stl/src/lib.rs`。今天都走
   `cad-import-core::MeshImport`，它只构造 `Mesh`、固定 layer 0、`measure = Unsupported`。
   要发 `Body3d` 必须绕过 `MeshImport`（参照 `cad-import-acadrust/src/support.rs`）。
2. **类型键与能力行**：`SkpPreviewMesh` / `StlMesh` + `MeshImport::finish()` 的能力行。
3. **分发**：只有格式嗅探变化才需要动 `cad-import-core::detect` 与
   `cad-app::import::FormatImporter`；纯映射变化不需要。

---

## 6. STEP 文件

### 6.1 现状（已实现，事实）

| 项 | 现状 |
|---|---|
| 解码链 | `cad-import-step`：`cadmpeg-codec-step` 0.6.12（Apache-2.0）→ `cadmpeg-ir` 中性拓扑 → `cadmpeg-tessellate` 逐面三角化 |
| 粒度 | 每个 body 一个 `DbEntity`；同一 body 的每个 occurrence 实例各一个实体 |
| 定位 | `Model::occurrences` 沿 `OccurrenceParent` 复合后**烘焙进模型空间**；未解析/歧义且多 body 时按名拒绝 `import.step.assembly_placement_unsupported` |
| 几何 | `SemanticGeometry::Mesh`，`type_key = StepTessellatedMesh` |
| 单位 | `cadmpeg-ir` 只存规范**毫米**；报告毫米 + `import.step.units_canonicalised`；解码报告无法解析长度单位时回退 `drawing_units` + `import.step.units_unknown` |
| 完整性 | 逐面拒绝 → `import.step.face_refused` + `Partial`；全部拒绝 → `Unsupported`（`import.step.no_tessellated_faces`）；无 B-Rep 面 → `import.step.no_brep_faces` |
| 能力 | read/render/pick `Verified`；**measure `Unsupported`**；精确 B-Rep **不进入域** |

参考样本上的真实拒绝率：NIST `nist-ctc05` AP242 156 面中 155 面成功、1 面拒绝；ABC AP214
25 面中 22 面成功、3 面拒绝。所以 STEP 导入通常是 `Partial`，不是"全成功"。

### 6.2 关键事实：精确 B-Rep 已经在 `cadmpeg-ir` 里

当前适配器只调用 tessellator，把 IR 的拓扑丢掉了；但 `cadmpeg-ir` 0.6.12 公开暴露：

- 拓扑：`Body` / `Region` / `Shell` / `Face` / `Loop`（`LoopBoundary` 的 coedge ring）/ `EdgeUse` / `Vertex` / `PcurveUse`；
- `Face { id, shell, surface, sense, loops, name, color, tolerance }`；
- 曲面 `SolvedSurfaceGeometry`：`Plane` / `Cylinder` / `Cone` / `Sphere` / `Torus` / `Nurbs` /
  `Polygonal` / `Transformed` / `Unknown`；
- 曲线 `SolvedCurveGeometry`：`Line` / `Circle` / `Ellipse` / `Parabola` / `Hyperbola` /
  `Degenerate` / `Composite` / NURBS；
- `FaceLoops` 区分 `Unspecified` 与 `Classified`（外环/内环）；`PcurveUse` 带参数曲线、
  `isoparametric` 与参数区间；
- provenance 里有 `Exactness` 标注，可区分"精确/派生/未知"。

**因此"STEP 要精确实体只能靠 OCCT"是错的。** 缺的是一个 `cadmpeg-ir → cad_domain::BrepData`
的 lifter，而不是一个新的内核。

### 6.3 映射方案

| STEP / `cadmpeg-ir` | `cad-db` 目标 | 备注 |
|---|---|---|
| `Body` / `Region` / `Shell` 闭合实体 | `BrepData.shells` + `BodyKind::Solid` | 只有**全部面 lift 成功且壳闭合**才标 `Solid` |
| 开口壳 / `SHELL_BASED_SURFACE_MODEL` | `BrepData.shells` + `BodyKind::Sheet` | `type_key` 保留 STEP 源类 |
| 线框 body | `BodyKind::Wire` + `display.edges` | |
| `Face.surface = Plane/Cylinder/Cone/Sphere/Torus` | `BrepSurface` 同名变体 | 当前中性模型已覆盖 |
| `Face.surface = Nurbs` | `BrepSurface::Nurbs`（P2 才有） | 未扩展前该面 `Unsupported` |
| `Face.surface = Transformed` | 复合到 `BrepPlacement` 或面级放置 | 非相似变换需 P2 |
| `Face.surface = Polygonal` | **不是 BRep**：只保留三角面 | body 不得标 `Exact`；`Partial` + 具名原因 |
| `Face.surface = Unknown` | 该面 `BrepSurface::Unsupported { type_key }` | 绝不猜曲面 |
| `Face.sense` | `BrepFace.reversed` | |
| `Face.loops` + coedge 方向 + `PcurveUse` | `BrepLoop.edges`（按 loop 方向） | pcurve 目前不进 `BrepData`；UV 裁剪是 P2 |
| `Edge.curve = Line / Circle` | `BrepCurve::Line / Circle` | 圆弧需确认是否整圆 |
| `Edge.curve = Ellipse / Nurbs / Composite …` | 需扩展 `BrepCurve` | 未扩展前该面 `Unsupported` |
| `Face.tolerance` | 面级容差 | `BrepFace` 需要新增字段或落诊断 |
| `Occurrence.transform` | `BrepPlacement`；装配可保留为 `Insert` + 块 | 现在烘焙；P-STEP-2 改成实例 |
| `Exactness` 标注 | `BodyExact::Exact` vs `Unavailable` | 未知/派生就不标精确 |

**诚实降级规则**（与 ACIS 路径一致）：

- 全部面 lift 成功且闭合 → `Solid` + `Exact` + 可按面 `Complete`；
- 任一面失败 → **不标 `Solid`**：`BodyKind::Unknown` + `Exact`（含 `Unsupported` 面）+
  `Partial` + 逐面诊断；或者 `Unavailable` 只留显示网格。二者都行，但不能两者兼得；
- 全是 `Polygonal`（AP242 面片化几何）→ `BodyKind::Unknown` + `Unavailable`，
  显示网格照常；
- 装配 placement 未解析 → 继续按名拒绝，**不**把 body 堆在原点。

### 6.4 分阶段

| 阶段 | 内容 | 说明 |
|---|---|---|
| **P-STEP-1a** | 解析曲面子集 lifter：Plane/Cylinder/Cone/Sphere/Torus + Line/Circle | **不需要 NURBS**，可直接用现有 `BrepSurface`/`BrepCurve`；覆盖大量机械件；可与 P1 并行 |
| **P-STEP-1b** | NURBS 曲面/曲线、椭圆/部分圆弧、pcurve | 依赖 P2 的中性模型扩展；trimmed NURBS 剖分优先走 solidcore |
| **P-STEP-2** | 装配保留为 `BlockDefinition` + `Insert` | 与 P-SKP-1 同一套实例模型；现在烘焙 |
| **P-STEP-3** | AP242 特征树/PMI | `cadmpeg-codec-step` 0.6.12 不解码 feature tree，明确不在范围 |

### 6.5 成本与风险

- **lifter 是新代码，但不是新内核**：解析子集（1a）是机械映射；NURBS/裁剪面（1b）才是大头，
  因为 `BrepData` 目前没有 pcurve，`BrepTessellator` 也不会剖 NURBS。
- 单位：IR 统一毫米，源单位不可见；报告必须写毫米 + 诊断，不能假装知道原始单位。
- 容差：`Face.tolerance` 需要进入中性模型，否则 `Solid` 的精确性没有容差依据。
- 精确性：`Polygonal` 与 `Unknown` 永远不是 BRep；不得因为"看起来闭合"就标 `Solid`。
- 与 solidcore 的关系：lifter 负责 **STEP → `BrepData`**；solidcore 负责 **`BrepData` →
  编辑后的 `BrepData`**。两者可以独立推进，但 P-STEP-1a 只有先于/并行于 P1 才有意义。

### 6.6 验收

- P-STEP-1a：一个解析曲面 STEP（盒体 + 圆柱）→ `Body3d{kind: Solid, exact: Exact}`，
  三角面与今天的 `StepTessellatedMesh` 一致；`MASSPROP` 仍 `Unsupported`（P1 未接）；
  导出仍按显示缓存（`POLYFACE`，P3a 不覆盖 STEP 来源）；**不**声称往返。
- P-STEP-1b：NIST/ABC 样本逐面比较 `exact` 面数、`Unsupported` 面数与诊断；任何
  "全部面精确"的声明必须有该样本的实测数字。
- 装配：同一 body 出现两次时得到两个 `Insert` 实例，而不是两份独立网格（P-STEP-2）。

### 6.7 代码落点（lifter 放哪、改什么）

调用链：

```text
StepImporter::import
  ├─ codec.decode → CadIr                      （已有）
  ├─ step_brep_lift::lift(&ir)                 （新：精确几何）
  │    Body/Region/Shell/Face/Loop/EdgeUse → BrepData
  ├─ cadmpeg_tessellate per body               （已有：显示缓存）
  ├─ placement → BrepPlacement + 显示顶点
  └─ Body3d { kind, exact, display } → DbEntity
```

| 文件 | 改动 | 阶段 |
|---|---|---|
| `crates/cad-import-step/src/brep_lift.rs`（新） | `lift(&CadIr) -> Vec<LiftedBody { kind, brep, diagnostics }>`；只做映射与显式拒绝 | 1a |
| `crates/cad-import-step/src/lib.rs` | lifter 与 tessellator 组合；改用能发 `Body3d` 的构建路径（参照 `cad-import-acadrust/src/support.rs`）；新增 `STEP_BODY_TYPE_KEY = "StepBody"`，`StepTessellatedMesh` 留给纯面片文件 | 1a |
| `crates/cad-domain/src/brep.rs` | 1a 不改；1b 增加 `BrepSurface::Nurbs`、`BrepCurve::{Nurbs, Ellipse, Arc}`、`BrepFace.tolerance` | 1b |
| `crates/cad-kernel-adapter/src/brep/` | 1a 复用 `BrepTessellator`；1b 增加 NURBS 剖分（或走 `SolidModeler::Tessellate`） | 1b |
| `crates/cad-db` | 无新逻辑；`validate_body` 的 `Solid ⇒ Exact` 直接约束 lifter 输出 | — |
| `crates/cad-app` | 无命令改动；能力行不变（`measure` 仍 `Unsupported`，直到 P1） | — |
| `docs/compatibility.md` | STEP 行改为 `Body3d` + 精确 B-Rep 的现状/边界 | 1a |
| `scripts/check-architecture.py` | 无新依赖，无需改 | — |

三条禁止：

1. **不按 occurrence 复制 BRep**：同一 body 的多个实例共享一份 `BrepData`；实例位置用
   `BrepPlacement`（1a）或 `Insert` + 块（P-STEP-2）。
2. **`Exactness` 未知/派生时不标 `Exact`**：宁可 `Unavailable { reason }`，也不要把
   "解出来但没证明精确"的东西写成精确。
3. **不从三角面反推 pcurve/NURBS**：`Polygonal` 与 `Unknown` 面在 **exact 层面**只能显式
   `Unsupported`（显示网格照常保留）；UV 裁剪必须来自 STEP 的 pcurve，而不是猜。

`Face.tolerance` 目前在中性模型里没有落点（`BrepFace` 无该字段）：1a 要么新增字段，
要么记具名诊断后丢弃——**不能**静默忽略，否则 `Solid` 的精确性没有容差依据。

---

## 7. 建议顺序与验收

| 阶段 | 内容 | 验收 |
|---|---|---|
| P0.1 | 存储边线直送 `RenderBatch.edges`；拆分到限文件（**已完成**，Linux 契约/单元证据） | 表示/场景契约测试；无三角形重推 |
| P-SKP-1/2/3 | SKP 组件实例、tag 图层、边线 | 导入契约测试 + 同一文件重复导入稳定；`Partial` 理由收窄 |
| P-MESH-1（可选） | `manifold3d` 网格布尔 + `MeshBody` | 网格布尔样本 + 显式"非精确"标记；`MASSPROP` 仍拒绝精确体积 |
| P-STEP-1a | `cadmpeg-ir → BrepData` 解析曲面 lifter（Plane/Cylinder/Cone/Sphere/Torus + Line/Circle） | 解析样本 → `Body3d{Solid,Exact}`；三角面与现 `StepTessellatedMesh` 一致；失败面显式 |
| P-STEP-1b | NURBS/椭圆/部分圆弧/pcurve | 依赖 P2；逐面计数，禁止无样本证据的"全部精确" |
| P-STEP-2 | STEP 装配改为 `BlockDefinition` + `Insert` | 同一 body 两次实例 → 两个 `Insert`，不是两份网格 |
| P1 | `cad-kernel-solidcore` + `SolidModeler` + 基本体/布尔/质量属性（**模型器与命令侧 C1/C2/C3 均已实现 2026-11-17**） | 命令侧：C1 创建、C2 后台布尔、C3a `MASSPROP`、C3b `face_sources`、C3c `VSCURRENT`（Linux 契约/单元，真实内核；body 含 `cad-cli-tools` BOX/CYLINDER 用例）；曲面 lift/曲面布尔仍 P2 |
| P2 | NURBS 面、圆角/倒角/抽壳、SURFACE 家族 | 逐命令契约 + 能力表；圆角产物是否为有理 NURBS 必须实测 |
| P3 | ACIS SAT/SAB 写出 | DXF/DWG 往返样本；否则不声称往返 |

**证据边界**：

- solidcore 的**平面体**路径已在 `cad-kernel-solidcore` 的 Linux 测试进程里真实执行
  （真实内核，create → boolean → mass properties；体积/面积/质心/惯性用闭式值断言），
  这是**有界契约/单元证据**，不是其它载体/GPU/性能结论。曲面 lift、圆角/倒角/抽壳、
  NURBS、SURFACE 家族与命令接线均**未实现**；其行为仍是 **NOT VERIFIED**。
- SKP/STL 的映射结论基于已提交的导入器代码；`P-SKP-*` 与 `P-MESH-1` 均未实现。
- STEP 的 §6 是**方案**：lifter 未实现。"`cadmpeg-ir` 已含精确拓扑"来自 0.6.12 源码
  （`topology.rs` / `geometry.rs` 的公开枚举），不是运行结果；样本上的保留率/精确率
  **NOT VERIFIED**。
- Windows/macOS/Android/Web 的 solidcore 运行状态全部 **NOT VERIFIED**；只有 Linux
  契约测试是可信的。
- `docs/compatibility.md` 的 SKP/STL/STEP 行仍是当前唯一权威能力表；本文件不替代它。
