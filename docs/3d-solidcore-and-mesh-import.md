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
- ACIS 导入已发出 `Body3d` 并保留 `exact`（`crates/cad-connector-acadrust/src/support.rs`）。
- `cad-db` 强校验 `Solid ⇒ Exact 且 B-Rep 非空`；非均匀缩放降级为 `Unavailable`。
- 但：
  - `cad-kernel-adapter` 只会 **BRep → 网格**（`BrepTessellator`），没有创建/布尔/圆角；
  - `BrepSurface` 自 **P2.1b（2026-11-17）** 起有 `Nurbs { u_knots, v_knots, points }`
    （齐次控制网、隐式次数、无周期性标志），`BrepFace.domain` 是该面自己的 `(u, v)` 参数矩形；
    但**还没有 NURBS 网格器**（`solidcore-tess`，P2.1c）；
  - `UNION`/`SUBTRACT`/`INTERSECT`/`BOX` 等已由 C1/C2 接成真实命令（见 §4.3）；
    `EXTRUDE`/`REVOLVE`/`SWEEP`/`LOFT`/`FILEDGE` 等仍是具名拒绝；
  - 仓库里**没有任何**网格布尔/修复/mesh→BRep 依赖（`manifold3d`/`baby_shark`/`parry3d`/
    `truck`/`opencascade` 均未出现）。

**2026-11-17 更新（命令侧）**：`MASSPROP` 已对精确 `Body3d{Solid, Exact}` 接线（C3a，见 §4.3）；
`face_sources` 已由 `BrepTessellator` 写入 `brep:face:<shell>/<face>`（C3b）；`VSCURRENT` 已是
会话级 3D body 视觉样式（C3c，默认 SHADED，与 AutoCAD 的 2D-wireframe 初始值有意偏离）；
**剩余 C3 项：无**。**P2.0** 单边直线倒角已接成 **`CHAMFEREDGE` 命令**（一次一条边、仅平面体、
种子取像素拾取命中点 `PickHit.point`；无子边身份/高亮/预览）；**P2.1a** 圆弧边
（`BrepCurve::Arc`）与 B-Rep 容差已落地；**P2.1b（2026-11-17）** 曲面 lift 已落地：
`BrepSurface::Nurbs` + `BrepFace.domain` 进入中性模型，`Cylinder`/`Cone`/`Sphere`/`Torus` 面带
**解析 label + 精确有理 NURBS form + 精确 pcurve** 回写 record，`ModelerCapabilities::nurbs` 置 `true`
且承载曲面操作数；**仍缺 NURBS 面的显示**（`solidcore-tess`，P2.1c）与圆角/抽壳。

---

## 3. 引入 solidcore 的收益

### 3.1 能力收益（API 存在 ≠ 已验收）

solidcore 1.0.1 的公开 API 覆盖（详见 ADR-0036 与 docs.rs）：

| 能力 | solidcore 入口（示例） | 当前仓库 |
|---|---|---|
| 基本体 | `create_box`/`create_sphere`/`create_cylinder`/`create_cone`/`create_torus`/`create_pyramid` | 已接线（C1，七族命令；曲面图元自 P2.1b 起可再 lift） |
| 拉伸/旋转/扫掠 | `sweep` 模块、`primitive_creation` | **拉伸已接线**：`SolidOperation::ExtrudeProfile` → `sweep::extrude`，由 `EXTRUDE` 命令把闭合平面轮廓（LINE 闭环 / 闭合直线 LWPOLYLINE / CIRCLE）拉成新的精确实体；部分圆弧壁因中性读回限制被具名拒绝。旋转/放样/扫掠仍无 seam 操作与命令（`REVOLVE`/`LOFT`/`SWEEP`），`FOLLOWME` 仍是具名拒绝 |
| 布尔 | `boolean_union`/`boolean_difference`/`boolean_intersection`/`boolean_merge`/`non_manifold_boolean` | C2 后台任务；平面体与**曲面体**（P2.1b）都可作为操作数 |
| 圆角/倒角 | `fillet_edges`/`circular_fillet`/`variable_radius_fillet`/`chamfer_fillet`/`remove_fillet` | P2.0 **平面体单边直线倒角**（`chamfer_sides_ordered`）+ `CHAMFEREDGE` 命令；**圆角无**（`fillet` 仍 false） |
| 抽壳/偏置 | `api_shell_brep_full`/`offset_brep`/`offset_faces` | **单面偏置已接线**（`SolidOperation::OffsetFace` → `offset_faces`，带符号距离，平面与曲面面都可移动），并由 `PRESSPULL` 命令 + 一次面拾取 + `pressPullDistance` 通道接通（2026-11-25）；抽壳仍无（`shell` false） |
| 精确质量属性 | `brep_mass_properties`/`brep_volume`/`brep_area` | 已接线（C3a）；**曲面体自 P2.1b 起可测**（`nurbs` 能力位承载） |
| 拓扑遍历 | `Brep::{faces,edges,vertices,loops,shells,regions}` + 公开字段 | 只有中性子集 |
| 修复 | `heal` 模块 | 无 |
| NURBS | `SurfaceKind::BSpline(BSplineSurface)`，权重在 `CPoint.w` | 中性 `BrepSurface::Nurbs` 已有（P2.1b），写出用 `solidcore::eval::form::surface`；**显示无网格器**（`solidcore-tess`，P2.1c） |

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
  `SatDocument` 构建，非 solidcore），其余 body 按名拒绝并退回 POLYFACE。**P3b（同日）**把
  同一份 `SatDocument` 用 `SabWriter` 写成**二进制 SAB**，DWG 的 `3DSOLID` 由上游 writer 放进
  AC1032 的 AcDs 段；`BODY`/`REGION`/`SURFACE` 与曲面/NURBS/一般裁剪体仍不写回。

### 3.4 成本与风险

| 风险 | 影响 | 处置 |
|---|---|---|
| API experimental、镜像仓每日变动 | 版本行为漂移 | 固定 **git revision**（不用 crates.io 1.0.1），只允许 `cad-kernel-solidcore` 命名 solidcore |
| 无公开 BRep→网格 | 显示链路接不上 | 用现有 `BrepTessellator` 对导出的 `BrepData` 剖分；**NURBS 面没有网格器**，按名报缺面并点名 `solidcore-tess`（P2.1c），不用假网格顶替 |
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
| `cad-connector-acadrust` | ACIS 发出 `Body3d`，保留 `exact`/`edges`/`tolerance`；导出：P0 只写 `POLYFACE`，**P3a 起**单壳全平面直线体的精确 B-Rep 写回 `3DSOLID`（SAT 文本），其余仍 `POLYFACE` 并记 `acis.*` |
| `cad-representation`/`cad-scene` | body → `Mesh` / `LineSegments` / `Missing`；精度与来源透传 |
| `cad-spatial`/`cad-measure`/`cad-app` | 独立 pick family `body`；ACIS/其它 body 仍**无 snap**（唯一窄例外是 SKP 的存储边载体，见 §5.2/ADR-0050）；`MASSPROP` 显式拒绝（P0 时；C3a 已改为精确测量，见 §4.3）；`select_filter` 用 `type_key` |
| 文档 | ADR-0036、`kernel-acis.md`、`compatibility.md`、`dxf-entity-coverage.md`、`handoff-archive/2026-11.md` |

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

- 状态（2026-11-17）：**B2 已落地、命令侧 C1/C2/C3 已落地、P2.1b 曲面 lift 已落地**；圆角/抽壳仍属 P2。
  `cad-kernel-solidcore` 存在并实现 `SolidModeler`；**平面体**的创建/布尔/质量属性/相似变换可用；
  曲面 primitives 可创建、可显示、可导出，自 **P2.1b** 起其**布尔与质量属性也可用**（解析曲面按
  精确 NURBS form + 精确 pcurve 回写 record，`ModelerCapabilities::nurbs` 为 `true` 且承载）；**仍然
  部分**的是**显示**：NURBS 面没有网格器（`solidcore-tess`，P2.1c），被剪裁的圆柱/圆锥侧壁仍报缺面。
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
  已是会话级 3D body 视觉样式（默认 SHADED）。**剩余 C3 项：无**；曲面 lift 已于 **P2.1b** 落地。
  **P2.0（2026-11-17）**：平面体单边直线倒角既是模型器能力（Linux 契约/单元），也已接成
  **`CHAMFEREDGE` 命令**（一拾取；body 来自当前选择、种子取 `PickHit.point`；原地改写同 id、
  一次 revision、一步撤销；无子边身份/高亮/预览/链式/NURBS）。**P2.1a（同日）**：`BrepCurve::Arc`
  与 `BrepData.tolerance`/`BrepFace.tolerance` 落地（圆弧边界的**平面**面可 lift/离散/回写）。
  **P2.1b（同日）**：`BrepSurface::Nurbs` + `BrepFace.domain` 进入中性模型；`Cylinder`/`Cone`/
  `Sphere`/`Torus` 面带解析 label + `solidcore::eval::form::surface` 的精确有理 NURBS form +
  form 参数的 `(u,v)` box + 精确 pcurve 回写 record（未命中 ⇒ `solid.pcurve_not_exact`）；
  `ModelerCapabilities::nurbs = true` 且承载（`solid.nurbs_not_supported`）。**仍 NOT IMPLEMENTED**：
  AutoCAD 的交互式尺寸提示、NURBS **面显示**（`solidcore-tess`）与圆角（P2.1c）、`SURFACE` 家族
  （P2.2）。证据只有 Linux 契约/单元（真实固定内核，无 mock）；其它载体 NOT VERIFIED。
  详见 ADR-0036 §7/§10。
- **拒绝边界（能力是 family 级，不是逐操作数）**：`capabilities().booleans` /
  `mass_properties` / `chamfer` / `offset_face` / `nurbs` 为 true 只表示"这一族有实现"，每次调用仍可能对单个操作数返回
  `Unsupported`，调用方必须处理：
  - 曲面操作数**需要** `nurbs` 能力位：模型器声明 `false` ⇒ `solid.nurbs_not_supported`；
    已声明 `true` 但仍不可 lift 的曲面面（无 `(u,v)` box、无精确 pcurve）→ `solid.lift_unsupported`；
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
     （形状对齐 `cad-connector-acadrust` ↔ `acadrust`）。
   - 固定 git revision `55ae49ef919b606bf77901df626ed9bb543e5212`（`solidcore-ops` +
     `solidcore-record` + P2.1b 新增的 `solidcore`（只用 `eval::form`）与 `solidcore-convert`，
     同一 pin、Apache-2.0，目标表 `cfg(not(wasm32))`，无 Cargo patch）；
     `Registration` 记 `yacr.kernel.solidcore`，上游版本串 `1.0.1+55ae49ef919b` 由
     `UPSTREAM_VERSION` 与 capabilities 声明（上游未 bump 该串）。
   - 只做三件事：`SolidModeler` 实现、`BrepData ↔ record` 双向转换（**P2.1b 起平面与解析曲面**：
     `Plane/PlaneThrough/Cylinder/Cone/Sphere/Torus` 面带精确 NURBS form + `(u,v)` box + 精确 pcurve
     回写，读回 `Spline`/`RationalSpline` → `BrepSurface::Nurbs`；曲线 `Line`/整圆/圆弧，写回侧 `Curve::Circle`
     含整圈与开弧）、能力报告。
   - 转换器有独立测试：单位（原样透传，记录按 mm 语义）、placement（相似 placement 烘焙；
     镜像拒绝）、面/环/边 canonical 顺序、`forward` 方向、顶点/边按精确位去重、容差、
     pcurve（必须是面 `(u, v)` 里的**精确闭式像**——直线/圆/弧的精确参数曲线，绝不拟合；
     占位 pcurve 会让面积积分返回 0）。
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
   稳定拒绝：`massprop.body_modeler_unavailable` / `body_not_liftable`（模型器拒绝该 B-Rep：曲面已可
   lift，剩下的是无 `(u,v)` box / 无精确 pcurve / 多壳等结构性原因 → `solid.lift_unsupported` 或
   `solid.pcurve_not_exact`）/ `body_kernel_failed` / `body_not_exact` / `body_not_solid` /
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

**P2.1b（已实现，2026-11-17）：曲面 lift 与 NURBS 载体。**

- `BrepSurface::Nurbs { u_knots, v_knots, points }`（齐次控制网 `[x·w, y·w, z·w, w]`、**隐式次数**、
  无周期性标志，权重必须有限且 `> 0`）+ `BrepFace.domain: Option<[[f64;2];2]>`（生产者自己的
  `(u, v)` 参数矩形；`None` = 写方可推导，平面可以，曲面面**必须**有）。record 的结构限制
  （每方向 2…4096 控制点、次数 1…16、`knots.len() == count + degree + 1`、权重 `>= 1e-12`）
  不在类型上重复断言，只在回写时校验。
- 读：record `Spline` → `Nurbs`（`w = 1`）、`RationalSpline` → `Nurbs`（齐次系数），net/knots/weights
  逐位通过，`Face.domain` 原样带入；`BallPatch`/`PlacedRevolution`/`Blend` 与样条/椭圆**边界曲线**
  仍按名拒绝（`solid.lift_unsupported`）。解析 label 忽略 `Face.nurbs`（label 就是几何）。
- 写：`Cylinder`/`Cone`/`Sphere`/`Torus` 面带解析 label + `solidcore::eval::form::surface` 的
  **精确有理 NURBS form** + form 参数下的 `(u,v)` box + 每个 use 的**精确 pcurve**
  （`solidcore_ops::pcurve::pcurves_within`，绝不拟合；未命中 ⇒ `solid.pcurve_not_exact`）。
  写入的 box **由面自身边界推出**，不复制 `BrepFace.domain`（生产者参数不可交给 `eval::form`：
  球是 `[0,1]²`、圆柱是弧度）；`label_box` 是**包围盒**（比裁剪片宽、u 取整圈），**绝不可收紧**：
  球的 seam 是零面积 bigon，全靠整圈 u 盒把两极两个 use 隔开一个周期。
  `eval::form` 的 knots 按每个角度方向 **π/2 重标**（精确重参数化、控制点不变）：**这不是把 form
  参数变成 label 角度**（`eval::form::param` 在四分之一圈内非线性——重标后 `0.25` ↔ `0.357 rad`，
  不是 `0.393`），只买到**单位**：一圈 = `2π`，box 端点 / `Domain::period` / seam 跳变因此同单位。
- 精确性证明：圆柱/圆锥的纬圆先**重标到面自己的 frame**（`curved::align_curved_frames`，只移参数
  零点；直接给区间做 `eval::form::on_frame` 位移会让整圈 pcurve 不覆盖 form 的 knot span，被 record
  以 “pcurve interval differs from its edge's” 拒绝），再由 `verify_pcurves` 对每个 use 在 **6 个
  样本（4 个内点）** 上按 `solidcore` 的 **1e-9 相对**判精确；未命中或**裁剪偏离四分之一网格**都是
  `solid.pcurve_not_exact`（点名 face/loop/use 与 gap）。平面面不受影响：偏网格的圆弧退回自己的
  knots，仍能 lift。
- 能力：`ModelerCapabilities::nurbs = true`（登记 `nurbs.faces`）**且承载**——布尔 / 质量属性 /
  倒角任一操作数含非平面面时必需，`false` 的模型器答 `solid.nurbs_not_supported`（family 级声明，
  逐操作数仍可能 `Unsupported`）。
- 两个新的直接依赖：`solidcore`（只用 `eval::form::{surface,curve}`——label 与几何被证明为同一曲面的
  唯一位置）与 `solidcore-convert`（`eval::form` 收内核 `eval` 类型，跨 record 边界需要
  `surface`/`record_surface`/`curve`/`record_curve`）。两者与 `solidcore-ops`/`solidcore-record`
  同一 pin、已在 `check-architecture.py` 允许表内，无 `[patch.crates-io]`、无版本变化。
- 实测（Linux x86-64，真实固定内核 `55ae49ef`）：圆柱 r=1 h=2 → `V=2π`、`A=6π`；球 → `4π/3`；
  圆环 R=2 r=0.5 → `V=π²`、`A=4π²Rr`；截头圆锥 r1=1 r2=0.5 h=2 →
  `πh(r1²+r1r2+r2²)/3`；**完整圆锥** r=1 h=2 → `V=πr²h/3`、`A=πr²+πr√(h²+r²)`、形心 `h/4`；
  圆柱 ∪/∖/∩ 盒 → `8+π/2`、`π/2`、`3π/2`；被剪裁的半圆柱 → `V=π`、`A=3π+4`、形心 y=`4/(3π)`；
  二次 lift（lift → kernel → read → lift）稳定。断言用内核请求的 **`1e-6` 相对精度**，实测误差远
  小于它、具体值只记在测试注释里（**不作为界**）。
  `CHAMFEREDGE` 在曲面体上现在**到达 pick**：写入方重建的**合成 seam**（两个 use 同属一个面）被
  跳过，所以点圆柱 rim 得到 **`solid.edge_not_found`**（点名 seam），不再报
  `solid.lift_requires_planar`；两个**不同**面共享的光滑 seam 仍会答 `solid.chamfer_failed`，但本
  构建没有可构造的 body 产生这种边，该路径只在谓词级钉住。曲面**边**倒角是 P2.2。

**P2.1c（未实现）：显示与圆角。**

- **NURBS 面显示**：没有任何网格器。中性离散器对 `BrepSurface::Nurbs` 按名拒绝并点名
  `solidcore-tess`（`nurbs-face: no NURBS mesher in this build (solidcore-tess, phase P2.1c)`），
  该面进 `missing_faces`，整条 body 是 `Partial`。被**剪裁**的圆柱/圆锥侧壁同样报缺面
  （`cylinder boundary is not a single full circle`，条带要求两个整圆）——**lift 成功 ≠ 显示完整**。
- 圆角：`fillet` 能力仍 `false`；产物是否为有理 NURBS 必须实测，显示同样卡在上一条。
- 其它已知缺口：`eval::form` 每面给一整圈 `u` box，绕行超过一圈的面（螺纹侧面）与 `eval::form`
  允许的 5 季度重叠都未特判（会写出自覆盖 form），未来必须在写前用 `turn_quarters`/`turned`
  或按名拒绝；`BrepFace.domain` 的语义（生产者参数 vs label 参数）需在 P2.1c 定一种。

**P2.2（未实现）：`SURFACE` 家族与曲面编辑。**

- `SURFACE` 家族：`PLANESURF`/`EXTRUDEDSURF`/`LOFTEDSURF`/`REVOLVEDSURF`/`SWEPTSURF`
  → `BodyKind::Sheet`；是否保留 `exact` 取决于能否从 ACIS/SolidWorks 读入。
- 抽壳/`Fillet`：`SolidRequest::Shell/Fillet` 不存在；圆角/抽壳产物是 B-spline 面，必须能进
  `BrepSurface::Nurbs`。
- 曲面**边**倒角：选择器只认真实共享边（合成 seam 被跳过，所以 rim 上是 `solid.edge_not_found`）；
  切 rim 需要曲线边选择器与沿曲线的退距约定。
- 空腔（void region）在布尔本身就被拒绝（`record → BrepData` 不映射 void region）；`reversed`
  面在 `to_record` 的环序/链检查处被拒绝。

### 4.5 P3：ACIS 写出

- **P3a（已实现，2026-11-17）**：`crates/cad-connector-acadrust/src/export/acis.rs` 用中性 `BrepData`
  构建 `SatDocument`（body→lump→shell→face→loop→coedge→edge→vertex→point；几何边跨面焊接、
  每条边两个 coedge、平面框架归一化并校验正交、外环绕向与面朝向一致）。子集：单壳、全平面、
  直线边界、真相似放置（无剪切/镜像）。
- **P3b（已实现，2026-11-17）**：同一份 `SatDocument` 有两个编码 —— DXF 写**内联 SAT 文本**
  （AC1024），DWG 用 `SabWriter` 写成**二进制 SAB**并由上游 `DwgWriter` 放进 **AC1032 的
  AcDs（`AcDsPrototype_1b`）段**（`Solid3D::from_sab`；`has_ds_data`/`pending_has_ds_data` 由
  `AcisData::contributes_sab()` 决定）。`BODY`/`REGION`/`SURFACE` 仍不写回。
- **拒绝码（两种格式相同，退回显示缓存 POLYFACE / LINE / POLYLINE3D）**：
  `acis.curved_face_not_written` / `acis.non_line_curve_not_written` /
  `acis.no_shell_not_written` / `acis.multi_shell_not_written` / `acis.placement_not_written` /
  `acis.invalid_topology_not_written`。**没有 SAB 专用码**：转换不可失败（`SabWriter::write -> Vec<u8>`，
  输入已 `validate()`）。
- **版本**：文件含任一 `3DSOLID`（含块定义内）即整份 **DXF** 按 AC1024 写出（内联 SAT、
  避免 AC1027+ 的 `ACDSDATA` SAB），记 `export.note.acis_sat_text_version`；**DWG 保持 AC1032**，
  记 `export.note.acis_sab_dwg`（状态行 `save.exported_acis_sab[_partial]`）。AC1024 只是 DXF 规则。
- 证据：自写→acadrust 解析→自读往返（盒/楔/棱锥/带孔面，几何 ≤1e-9·对角线、体积 ≤0.1%）+
  **DWG 写入→`DwgReader` 读回（AcDs 挂回 SAB）→`sab_to_brep` 同一精确体**（无 POLYFACE，
  版本 AC1032，SAB 单独可被 `SabReader` 解析）+ 真实 solidcore BOX 的 DXF 存盘/再导入
  （精确体、体积 1±1e-3）。**无第三方 CAD 读取器证据**，无 vendor/像素/非 Linux 证据。
- **仍未做**：曲面/NURBS 面与一般裁剪 B-rep 的 ACIS 写出（仍按名拒绝）、`BODY`/`REGION`/
  `SURFACE` 写回、ASM（ShapeManager，21800）载荷 —— 本切片写出的是 **ACIS 7.0（700）** 头的
  非 ASM SAB，与我们的 SAT 构建器一致；ACAD 2013+ 原生体通常带 21800 ASM 头，这一点**未验证**。

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

**STL**（`crates/cad-connector-mesh` 的 `src/stl/`）：

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

**SKP**（`crates/cad-connector-skp`）：

| 项 | 现状 |
|---|---|
| 几何 | 每个（placement × 生效材质）组 → 一个 `Mesh`；**实例已烘焙到世界坐标**，不用 `Insert`/块；有完整真实边记录的组升级为 `Body3d`（三角形 + `BodyDisplay.edges`，ADR-0049） |
| `type_key` | `SkpPreviewMesh`（面组；wire 体为 `SkpPreviewEdges`，ADR-0049） |
| 单位 | 米、`+Z` up，`import.skp.units_metres` |
| 层级 | tag **扁平化到 layer 0**，`import.skp.layers_flattened` |
| 材质 | 逐实体 `EntityColor::Explicit` 或 `ByLayer`；纹理只取平均色，`import.skp.textures_not_sampled` |
| 透明度 | `0 ≤ opacity < 1` 时 `Explicit`，否则 `ByLayer` |
| 可见性 | 遵守隐藏实例/隐藏 tag/隐藏面，`import.skp.hidden_pruned` |
| 未导入 / 边界 | 纹理图像与 UV、场景/页面、剖切面、标注、动态组件属性；**边/曲线按真实 `MeshEdge` 记录携带**（`BodyDisplay.edges`，ADR-0049；见 §5.3），纯边文件导入为 `Partial` wire 体、不再拒绝；**吸附/测量是窄声明**（ADR-0050）：只有实际携带存储边的载体可吸附，仅端点/中点，无面/三角/中心/象限/交点与面积/体积 |
| 完整性 | 每个成功导入恒 `Partial`（`import.skp.preview_geometry`） |
| 能力 | 受 release 上限约束；`measure` 默认 `Unsupported`，只对**确实插入非空 `BodyDisplay.edges`** 的 `SkpPreviewMesh`/`SkpPreviewEdges` 报 `Partial`（端点/中点 only；三角-only 组仍 `Unsupported`；天花板把 `Partial` 收窄为 `Unverified`，不强制 `Unsupported`） |
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
| SKP 边/曲线 | **已实现（2026-11-23，ADR-0049）**：真实 `MeshEdge` 记录进 `Body3d` 载体（完整边记录的面组 → `SkpPreviewMesh` body 的 `BodyDisplay.edges`；游离/未被 body 或已三角化面环覆盖的边 → `SkpPreviewEdges` wire 体），`hidden`/`soft`/不可见图层剔除，曲线存两点弦，边只作显示/拾取 | **P-SKP-3 完成**：不新增 `LineSegments` 图元，复用既有 `Body3d` + `BodyDisplay.edges` | 不做边 id/子元素与编辑；端点/中点吸附与 `measure = Partial` 已实现（ADR-0050 的窄声明：只为**实际携带存储边**的载体，无面/三角/中心/象限/交点吸附、无面积/体积；三角-only 组仍 `Unsupported`）；纯边文件不再拒绝，导入为 `Partial` wire 体 |
| STL/SKP 的"实体编辑" | 无 | **P-MESH-1**（可选）：独立的网格体类别 + 网格布尔 | 产物仍是网格；**不要**复用 `BodyKind::Solid` |

**SKP 写出（2026-11-23，ADR-0051）**：所有源写进**一个**合并的 root `GeometryRun`——钉死的
`skpwright` 0.6.0 写出器只发第一个 root run（`build26.rs` 的 `find(|r| r.def_index.is_none())`），
旧的一源一 run 布局会静默丢掉第一个之后的实体，而报告仍计 `exact`。合并后多源时记一次
`export.skp.entity_grouping_flattened`（`Partial`：实体分组不保留），`max_output_entities` 按
**源实体**计（含没写出几何的源）。无三角形的 wire 体写成**自由边**：追加自己的端点、不与网格
或其它 polyline 焊接、N−1 段链式、默认 flags/layer/material、`curve: None`；零长段跳过并记
`export.skp.zero_length_edge`，同一 polyline 内重复端点对只写一次（边集不变，仍 `exact`；全段零长的
wire 体按 `export.skp.wire_body_no_writable_edges` 整实体丢弃）；
曲线/弧身份与逐边属性不可还原，记 `export.skp.edge_semantics_not_written`；
`export.skp.wire_body_not_written` 退役。**mesh 体**的 stored 边集/分段仍不复制（逐实体
`export.skp.stored_edges_not_written`，`Partial`），因为把世界坐标 polyline 写进写出器的顶点集
需要焊接/容差决定，而焊接会发明源里没有的共享顶点身份（ADR-0049 D2 对导入侧已有同一条约束）。
证据只有 **`skpwright` 自洽往返**
（写出→本仓库读回同一三角/边数与坐标），**不是** SketchUp/Trimble 接受性。

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

1. **发射**：`cad-connector-skp/src/lib.rs` / `cad-connector-mesh/src/stl/mod.rs`。今天都走
   `cad-connector-core::MeshImport`，它只构造 `Mesh`、固定 layer 0、默认 `measure = Unsupported`
   （ADR-0050 起导入器可显式 `declare_edge_snap_measure`，且只为实际插入的非空显示边载体报
   `Partial`）。
   要发 `Body3d` 曾必须绕过 `MeshImport`（参照 `cad-connector-acadrust/src/support.rs`）；ADR-0049
   起 `MeshImport` 提供 `push_body*`，SKP 的 wire 体键为 `SkpPreviewEdges`。
2. **类型键与能力行**：`SkpPreviewMesh` / `SkpPreviewEdges` / `StlMesh` + `MeshImport::finish()`
   的能力行（按实际插入的 `type_key` 各出一行）。
3. **分发**：只有格式嗅探变化才需要动 `cad-connector-core::detect` 与
   `cad-app::import::FormatImporter`；纯映射变化不需要。

---

## 6. STEP 文件

### 6.1 现状（已实现，事实）

| 项 | 现状 |
|---|---|
| 解码链 | `cad-connector-step`：`cadmpeg-codec-step` 0.6.12（Apache-2.0）→ `cadmpeg-ir` 中性拓扑 → `cadmpeg-tessellate` 逐面三角化 |
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
| `Face.surface = Nurbs` | `BrepSurface::Nurbs`（**载体已随 P2.1b 落地**；STEP lifter 仍未实现） | 未扩展前该面 `Unsupported` |
| `Face.surface = Transformed` | 复合到 `BrepPlacement` 或面级放置 | 非相似变换需 P2 |
| `Face.surface = Polygonal` | **不是 BRep**：只保留三角面 | body 不得标 `Exact`；`Partial` + 具名原因 |
| `Face.surface = Unknown` | 该面 `BrepSurface::Unsupported { type_key }` | 绝不猜曲面 |
| `Face.sense` | `BrepFace.reversed` | |
| `Face.loops` + coedge 方向 + `PcurveUse` | `BrepLoop.edges`（按 loop 方向） | pcurve 目前不进 `BrepData`；UV 裁剪是 P2 |
| `Edge.curve = Line / Circle` | `BrepCurve::Line / Circle` | 圆弧需确认是否整圆 |
| `Edge.curve = Ellipse / Nurbs / Composite …` | 需扩展 `BrepCurve` | 未扩展前该面 `Unsupported` |
| `Face.tolerance` | 面级容差（`Option<f64>`，模型单位长度） | 字段已随 P2.1a 落地；未知填 `None`，聚合/冲突规则见 P2.1a 行 |
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
- 容差：`Face.tolerance` 已进入中性模型（P2.1a，`Option<f64>` **模型单位长度**）；lifter 必须
  如实填 `Option`（源未给出就是 `None`），不得用默认值假装已知。
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
| `crates/cad-connector-step/src/brep_lift.rs`（新） | `lift(&CadIr) -> Vec<LiftedBody { kind, brep, diagnostics }>`；只做映射与显式拒绝 | 1a |
| `crates/cad-connector-step/src/lib.rs` | lifter 与 tessellator 组合；改用能发 `Body3d` 的构建路径（参照 `cad-connector-acadrust/src/support.rs`）；新增 `STEP_BODY_TYPE_KEY = "StepBody"`，`StepTessellatedMesh` 留给纯面片文件 | 1a |
| `crates/cad-domain/src/brep.rs` | 1a 不改；`BrepSurface::Nurbs` 已随 P2.1b 落地，1b 只补 `BrepCurve::{Nurbs, Ellipse}`（`Arc` 与 `BrepFace.tolerance` 已随 P2.1a，`Face.domain` 随 P2.1b） | 1b |
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

`Face.tolerance` 已在中性模型里（P2.1a，`Option<f64>`，**模型单位长度**）：lifter 源里没有该值
就填 `None`，**不能**用默认值假装已知；body 与已声明面容差的聚合/冲突在 `to_record` 处执行
（冲突按 `solid.lift_unsupported` 拒绝，见 `docs/kernel-acis.md` §11）。

---

## 7. 建议顺序与验收

| 阶段 | 内容 | 验收 |
|---|---|---|
| P0.1 | 存储边线直送 `RenderBatch.edges`；拆分到限文件（**已完成**，Linux 契约/单元证据） | 表示/场景契约测试；无三角形重推 |
| P-SKP-1/2/3 | SKP 组件实例、tag 图层、边线（**P-SKP-3 已实现 2026-11-23，ADR-0049**：真实边进 `Body3d`/`BodyDisplay.edges`；显示边吸附/测量按 ADR-0050 的窄声明，SKP 写出按 ADR-0051 合并单 run + 自由边） | 导入契约测试 + 同一文件重复导入稳定；`Partial` 理由收窄；边证据仅契约测试，corpus 重跑 **NOT RUN**；导出证据仅 `skpwright` 自洽往返 |
| P-MESH-1（可选） | `manifold3d` 网格布尔 + `MeshBody` | 网格布尔样本 + 显式"非精确"标记；`MASSPROP` 仍拒绝精确体积 |
| P-STEP-1a | `cadmpeg-ir → BrepData` 解析曲面 lifter（Plane/Cylinder/Cone/Sphere/Torus + Line/Circle） | 解析样本 → `Body3d{Solid,Exact}`；三角面与现 `StepTessellatedMesh` 一致；失败面显式 |
| P-STEP-1b | NURBS/椭圆/部分圆弧/pcurve | 依赖 P2；逐面计数，禁止无样本证据的"全部精确" |
| P-STEP-2 | STEP 装配改为 `BlockDefinition` + `Insert` | 同一 body 两次实例 → 两个 `Insert`，不是两份网格 |
| P1 | `cad-kernel-solidcore` + `SolidModeler` + 基本体/布尔/质量属性（**模型器与命令侧 C1/C2/C3 均已实现 2026-11-17**） | 命令侧：C1 创建、C2 后台布尔、C3a `MASSPROP`、C3b `face_sources`、C3c `VSCURRENT`（Linux 契约/单元，真实内核；body 含 `cad-cli-tools` BOX/CYLINDER 用例）；曲面 lift/曲面布尔见 P2.1b |
| P2.0 | **已实现（2026-11-17）：平面体单边直线倒角，模型器能力 + `CHAMFEREDGE` 命令接线**（`SolidOperation::Chamfer`；边由最近点选、两距离、对称 45° 缺省；产物仍全平面并可再 lift；命令为一次边拾取、种子取 `PickHit.point`、原地改写同一实体） | Linux 契约/单元（真实内核）：单位立方体一边 0.1 倒角 → 体积 `1 - 0.1²/2`（闭式）、全平面、`tessellate_exchange` 水密、再 lift 质量属性；非对称 `distance=0.1, second=0.2` → `1 - 0.1·0.2/2 = 0.99`；wedge 底/背面凸 90° 交线 → `L·W·H/2 - d²/2·W = 11.985`；倒角→UNION→质量属性链；拒绝：离边过远 `solid.edge_not_found`、距离 0/负 InvalidInput、预取消 Cancelled、几何上建不出该倒角 `solid.chamfer_failed`（与内核完整性 `solid.kernel_failed` 分开）；**曲面体自 P2.1b 起可达 pick**：写入方重建的合成 seam（两个 use 同属一个面）被跳过，点在圆柱 rim 上答 `solid.edge_not_found`（点名 seam），不再是 `solid.chamfer_failed`、也不再是 `solid.lift_requires_planar`；两个不同面共享的光滑 seam 仍答 `solid.chamfer_failed`，但本构建没有可构造的 body 产生它（仅谓词级钉住）。命令侧为合成 `cad-app`/`cad-host` 契约 + `cad-cli-tools/tests/solid_command.rs` 的真实内核用例。**凹边/非 90° 二面角未测**；无子边身份/高亮/预览/链式/NURBS；GPU/像素/非 Linux **NOT VERIFIED** |
| P2.1a | **已实现（2026-11-17）**：`BrepCurve::Arc`（逆时针绕 `normal`，两端为点；跨度 = `(end_param − start_param).rem_euclid(2π)`，非半弧由跨度定方向、**半弧**按 ACIS 逆时针约定；跨度 ≤ `1e-9` = `arc-without-span`，而 **v400 SAT 的 Arc 边不可导入**）+ `BrepData.tolerance`/`BrepFace.tolerance`（**模型单位长度**、`Option`，`None` = 源未给出）；圆弧边界的**平面**面可 lift/离散/回写 record | Linux 单元/契约：圆弧环在接缝精确闭合、四分之一圆盘面积对 `π/4` 在 1e-5 弦高预算内 0.1 %、半弧 bulge 有界、跨度跨 seam 回绕（不镜像到补弧）与方向判定、容差在相似变换下仍是同一模型长度（只有 `record::bake` 缩放一次）、非均匀缩放降级、面级容差的聚合/冲突拒绝与 `from_record` 恒 `None`；**含圆弧的全平面闭合壳被内核以 status `1001` 拒绝**（`solid.kernel_failed`，退化体而非转换缺口）；圆弧边界的圆柱/圆锥侧面的 lift 由 P2.1b 补齐 |
| P2.1b | **已实现（2026-11-17）：NURBS 中性载体 + 曲面 lift**（`BrepSurface::Nurbs{u_knots,v_knots,points}`（齐次、隐式次数、无周期性标志）+ `BrepFace.domain`；读侧 `Spline`/`RationalSpline` → `Nurbs`；写侧 `Cylinder`/`Cone`/`Sphere`/`Torus` 面带解析 label + `eval::form::surface` 的精确有理 NURBS form + `(u,v)` 包围盒 + 精确 pcurve（纬圆先 `align_curved_frames` 重标到面 frame，再由 `verify_pcurves` 6 样本/1e-9 相对证明）；`ModelerCapabilities::nurbs = true` 且承载（族优先：未声明族答 `solid.unsupported`）；新码 `solid.pcurve_not_exact`（含偏网格裁剪）、`solid.nurbs_not_supported`；**完整圆锥**也 lift） | Linux 契约/单元（真实固定内核 `55ae49ef`，断言为内核请求的 **1e-6 相对精度**；实测误差远小于它，数值不钉成界）：圆柱 r=1 h=2 → `V=2π`/`A=6π`；球 `4π/3`；圆环 R=2 r=0.5 → `V=π²`/`A=4π²Rr`；截头圆锥 `πh(r1²+r1r2+r2²)/3`；**完整圆锥** `V=πr²h/3`、`A=πr²+πr√(h²+r²)`、形心 `h/4`（`a_full_cone_measures_its_apex_band`）；圆柱∪/∖/∩盒 `8+π/2`/`π/2`/`3π/2`；被剪裁半圆柱 `V=π`/`A=3π+4`/形心 y=`4/(3π)`；二次 lift 稳定。**显示仍 `Partial`**（NURBS 面无网格器；被剪裁侧壁报缺面） |
| P2.1c（未实现） | **NURBS 面显示**（`solidcore-tess`）与**圆角**（`fillet` 仍 false）；`eval::form` 多圈 `u` box 与 `BrepFace.domain` 语义待定 | 网格器接入后逐面比较；圆角产物是否为有理 NURBS 必须实测 |
| P2.2（未实现） | `SURFACE` 家族（`PLANESURF`/`EXTRUDEDSURF`/`LOFTEDSURF`/`REVOLVEDSURF`/`SWEPTSURF`）、抽壳、曲面**边**倒角 | 逐命令契约 + 能力表 |
| P3 | **P3a/P3b 已实现（2026-11-17）**：同一份 `SatDocument` → DXF 内联 SAT 文本（AC1024）/ DWG 二进制 SAB（AC1032 AcDs）；只覆盖单壳全平面直线子集，`BODY`/`REGION`/`SURFACE` 不写回 | DXF 与 DWG 的往返样本（后者经 acadrust `DwgReader` 自读）；**无第三方 CAD 读取器**，SAB 头 ACIS 7.0（700，非 ASM）的 vendor 接受性未验证 |

**证据边界**：

- solidcore 的路径已在 `cad-kernel-solidcore` 的 Linux 测试进程里真实执行（真实固定内核，
  create → boolean → chamfer → mass properties，以及 P2.1b 的曲面 lift / 曲面布尔 / 二次 lift；
  体积/面积/质心/惯性用闭式值断言），这是**有界契约/单元证据**，不是其它载体/GPU/性能结论。
  **曲面 lift（P2.1b）本身已实现**，但 **NURBS 面的显示**（`solidcore-tess`）、圆角、抽壳、
  `SURFACE` 家族仍未实现，其行为是 **NOT VERIFIED**；被剪裁的曲面侧壁 lift 成功而显示为缺面。
  P2.0 的倒角、P2.1a 的圆弧 lift 与 P2.1b 的曲面 lift 都只有 Linux 契约/单元与固定 revision 内核
  证据；`CHAMFEREDGE` 的宿主/命令接线是合成契约（UI 只由前端套件覆盖），无 GPU/像素/非 Linux
  证据。P3a/P3b 的写出证据是本仓库自写 + acadrust 自读往返，**没有第三方 CAD 读取器**。
- SKP/STL 的映射结论基于已提交的导入器代码；`P-SKP-1/2` 与 `P-MESH-1` 仍未实现（`P-SKP-3`
  已实现，见 §5.3/ADR-0049–0051）。
- STEP 的 §6 是**方案**：lifter 未实现。"`cadmpeg-ir` 已含精确拓扑"来自 0.6.12 源码
  （`topology.rs` / `geometry.rs` 的公开枚举），不是运行结果；样本上的保留率/精确率
  **NOT VERIFIED**。
- Windows/macOS/Android/Web 的 solidcore 运行状态全部 **NOT VERIFIED**；只有 Linux
  契约测试是可信的。
- `docs/compatibility.md` 的 SKP/STL/STEP 行仍是当前唯一权威能力表；本文件不替代它。
