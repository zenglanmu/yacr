# ADR 0036：3D 实体模型（wireframe / surface / solid）与 solidcore 无状态建模服务

- 状态：**Accepted（2026-11-16）**，**P1 部分实现**（2026-11-17，B2 + C1 + C2 增补）：
  - P0 已落地；P0.1（存储边线直送 `RenderBatch.edges`）已完成（Linux 契约/单元证据）；
  - P1/B1：`SolidModeler` 无内核接缝已落地（`cad-kernel-adapter`）；
  - P1/B2（见 §7）：`cad-kernel-solidcore` 让**平面体**路径可用——基本体
    （BOX/WEDGE/PYRAMID 本仓精确平面 B-rep；CYLINDER/SPHERE/CONE/TORUS 走上游构造器）、
    UNION/SUBTRACT/INTERSECT、体积/面积/质心/惯性、相似变换；
  - **P1/C1（2026-11-17）**：命令层接线第一阶段——`BOX`/`WEDGE`/`PYRAMID`/`CYLINDER`/`SPHERE`/
    `CONE`/`TORUS` 由具名拒绝变成**真实创建**：显式点 → `PrimitiveSpec` → 注入的模型器 →
    中性 B-Rep → 既有 `BrepTessellator` 显示缓存 → 一条 `AcDb3dSolid` 实体（一条事务/一步撤销）。
    显示剖分用**共享的形状相对容差**（`cad_kernel_adapter::shape_relative_tolerance`，1e-3 ×
    包围盒对角线，与导入同一条规则）；平面体 `Complete`，曲面体 `Partial` 并给出共享的
    “离散网格、缩放不细化”理由。模型器由**载体注入**（`Application::set_solid_modeler`）；
    本切面只有 Linux `native-below` 注入 `SolidCoreModeler`，其它载体/`dom-canvas` 传 `None` 并报
    `unsupported-runtime: solid-modeler`。
    **同步豁免（仅 C1）**：图元命令在**命令线程内同步**执行模型器 + 剖分，理由是工作有界
    （单个图元 + 形状相对剖分，三角形数近似恒定）且取消探针恒为 `false`。**C2 的布尔不得沿用
    该豁免**：布尔必须放到 worker 线程并接活取消探针，否则一条长操作会阻塞命令线程且无法取消。
  - **P1/C2（2026-11-17）**：`UNION`/`SUBTRACT`/`INTERSECT` 由具名拒绝变成**真实后台布尔**：
    命令线程只校验（≥2 个去重实体、整实体引用、同图层/同空间、`AcDb3dSolid` 且
    `Body3d{Solid, Exact}`）并快照操作数，然后启动 `tasks::solid::SolidBooleanJob`
    （`CancellationToken` + 真 worker；UNION/INTERSECT 左折叠，SUBTRACT 第一个减其余；最终
    `BrepData` 用与 C1 相同的形状相对容差剖分）；`HostController::poll_solid_boolean` 在
    document/generation/操作数快照都仍一致时以**一条事务 + 一步撤销**删除操作数并插入一个新
    `AcDb3dSolid`，否则报 `Stale` 且图纸不变；`cancel_solid_boolean` 与文档关闭/新建都不发布。
    由于本构建的写路径不推进 `DbObject.revision`，陈旧检查比较整行 `DbEntity` 快照，而不只
    revision；丢弃结果会写 `solid.boolean_stale` 状态（不是静默丢弃），`Cancelled` 另有状态。
    worker 的 panic（通道断开/`join` 为 `Err`）映射为 `solid.boolean_worker_panicked` 的
    `Invariant`，只有取消 token 已置位且线程干净退出才是 `Cancelled`（panic 不冒充取消，宿主也
    不泄漏 panic payload）。结果继承**第一个操作数**的显式显示状态（同一事务的
    `update_entity_display_state`，redo 也恢复）；无覆盖则不写。布尔运行期间 C1 的同步图元创建
    按 `solid.boolean_busy: a boolean is running; wait for it to settle before creating another
    solid` 拒绝（共用内核互斥锁，不能阻塞命令线程），拒绝在模型器调用与事务之前。曲面操作数由
    模型器给出 `Unsupported`（`solid.lift_requires_planar`），P2 前显式拒绝；
    `MASSPROP`/`face_sources`/`VSCURRENT` 仍是 C3。
  - **曲面 body**（含 cylinder/cone/sphere/torus 面）的布尔与质量属性**显式拒绝**
    （`solid.lift_requires_planar`），曲面 lift 是 P2；
  - **仍未实现**：AutoCAD 的交互式尺寸提示；`MASSPROP` 接线、`face_sources`、`VSCURRENT`
    （C3）；曲面布尔（P2）、P2、P3。`UNION`/`SUBTRACT`/`INTERSECT` 已在 C2 接成后台任务（见下）；
    `3DSOLID`/`BODY` 是**实体类型键**（模型器在 `Registration::entity_types` 声明），不新增同名命令。
  验证边界：只有 Linux x86-64 上的契约/单元证据——测试进程里跑的是**真实**固定 revision 的
  `solidcore` 内核（无 mock、不引用上游 golden）；Windows/macOS/Android/Web 的 solidcore
  运行状态与模型器注入、任何 GPU/像素/性能结论一律 **NOT VERIFIED**。
- 日期：2026-11-16（P1/B2 增补 2026-11-17，P1/C1 增补 2026-11-17，P1/C2 增补 2026-11-17）。
- 相关：[ADR-0033](0033-multi-format-import.md)（多格式导入；D2/D9 的 BRep 边界）、
  [ADR-0031](0031-acadrust-vendored-patch-policy.md)（依赖补丁策略）、
  `docs/kernel-acis.md`（现有 ACIS 契约）、`docs/compatibility.md`（逐命令能力表）、
  `docs/3d-solidcore-and-mesh-import.md`（接入收益与 SKP/STL 映射方案）、
  `scripts/check-architecture.py`（分层门禁）、`crates/cad-kernel-adapter`。
- 影响：`cad-domain`（新增 `BrepData` 与 `Body3d`）、`cad-kernel-adapter`（数据下沉、
  保留评估接缝）、`cad-db`（校验/变换/包围盒）、`cad-representation` / `cad-scene`
  （存储边线与实体显示）、`cad-import-acadrust`（保留精确 BRep 与边线）。**不**改变：
  `cad-db` 是唯一权威；渲染是派生；内核不持有文档状态。

---

## 1. 问题

1. 当前 3D 只有一种填充载体：`cad-domain::SemanticGeometry::Mesh`
   （`crates/cad-domain/src/semantic.rs:166`）。没有 `Solid`/`Body`/`Region`/`Surface`/
   B-Rep/NURBS 曲面变体，所有几何坐标虽是 3D，但面/体类别在数据层不存在。
2. 导入端把所有 3D 类别塌缩成 `Mesh`：`3DFACE`、`POLYFACE`、`POLYGON MESH`、`MESH`、
   ACIS 的 `3DSOLID`/`BODY`/`REGION`/`SURFACE` 都只留 `DbObject.type_key` 字符串
   （`AcDbFace`/`AcDbPolyFaceMesh`/`AcDbSubDMesh`/`AcDb3dSolid`/`AcDbSurface`…）。
3. ACIS 的 SAT/SAB 在导入时丢弃（`crates/cad-import-acadrust/src/support.rs:46`），
   内核产出的边界边线也被丢弃（`support.rs:79` 只克隆 `mesh`），
   而 `RenderBatch.edges` 早已能上传并绘制。`face_sources` 结构上存在，但没有任何
   生产导入器填充。
4. 用户要求"创建并编辑实体"。候选只有 `truck` 与 `solidcore`；`truck` 已发布 crate
   无圆角/倒角/抽壳、布尔在重合几何上 panic、STEP 只能出不能进，无法满足创建+编辑，
   因此选 **solidcore**（纯 Rust、`#![forbid(unsafe_code)]`、`Send + Sync`、
   wasm32 可编译、Apache-2.0），但它是 `solidcraft` 的只读镜像、API 自陈 experimental、
   没有公开的 BRep→网格接口、没有取消钩子、没有 STEP。

## 2. 术语（避免把显示模式当实体类别）

- **实体类别**（数据）：AutoCAD 的 DXF 类，存在 `type_key` 字符串里
  （`AcDb3dSolid`、`AcDbSurface`、`AcDbRegion`、`AcDbFace`…）。
- **拓扑维度**（数据）：ACIS 的一维 wire body、二维 sheet body、三维 solid body。
- **视觉样式**（会话状态）：`2D wireframe` / `shaded` / `hidden`… 是**视口设置**，
  不是实体类别。一个 `3DSOLID` 用 2D wireframe 显示仍然是 solid。

本 ADR 按这三层分别建模，不把它们混成一个字段。

## 3. 决策

### D1 采用 `solidcore` 作为无状态建模服务

- 新增 crate `cad-kernel-solidcore`，它是**唯一**允许命名 `solidcore` 的包，
  形状对齐 `cad-import-acadrust` ↔ `acadrust`；`scripts/check-architecture.py`
  增加 forbid 规则，禁止 `solidcore` 类型泄漏到其它 `cad-*`。
- 依赖固定在**具体 git revision**（不使用 crates.io 1.0.1；它落后 HEAD 约 2.5 周），
  并记录所用 revision 的 `Registration`（type key + version）。
- 内核是纯计算服务：内容进、内容出，不持有文档、body 句柄或可变状态。
  `BodyId`/`revision` 属于 `cad-db`（`EntityId` + `ObjectId.revision`）；跨调用只传
  `BrepData` 快照。缓存键用内容摘要（如 `BodyKey { codec, digest }`），不是 handle。

接缝（与现有 `SolidTessellator` 并列，保留后者给导入期剖分）：

```rust
pub trait SolidModeler {
    fn registration(&self) -> Registration;
    fn capabilities(&self) -> ModelerCapabilities;   // booleans/fillet/shell/nurbs
    fn execute(
        &self,
        request: &SolidRequest<'_>,                  // Tessellate | Transform | Primitive | Boolean
        cancelled: &dyn Fn() -> bool,
    ) -> CadResult<SolidOutcome>;                    // body + display + degradation + diagnostics + aborted
}
```

- 结果是新的 `BrepData`/`TessellationMesh`；应用经既有
  `apply_drawing_changes`/`ChangeSet` 写回 `cad-db`，`ChangeMask::GEOMETRY` 触发显示失效。
  不新增写路径，不直接改场景。
- **取消**：solidcore 没有取消钩子。只能在提交前与返回后检查 `cancelled()`，调用放在
  worker 上，UI 不得阻塞；取消后到达的结果标记 `aborted` 并丢弃，绝不落库。
  wasm 无线程时按载体能力显式拒绝（稳定的 `unsupported-runtime: solid-modeler`），
  不得在主线程静默阻塞。
- 错误映射：能力缺失 → `Unsupported { reason }`；内核失败 → `Failed { diagnostics }`；
  坏容差/退化操作数 → `CadError::InvalidInput`；预检查取消 → `CadError::Cancelled`。

### D2 3D 数据模型：一个 `Body3d`，类别不重复建模

```rust
pub enum BodyKind { Wire, Sheet, Solid, Unknown }
pub enum BodyExact { Exact(Box<BrepData>), Unavailable { reason: String } }
pub struct BodyDisplay {
    pub mesh: Mesh,
    pub edges: Vec<Vec<Point3>>,
    pub tolerance: Option<f64>,
    pub precision: Precision,
    pub completeness: Completeness,
}
pub struct Body3d { pub kind: BodyKind, pub exact: BodyExact, pub display: BodyDisplay }
```

- 新增 `SemanticGeometry::Body3d(Box<Body3d>)`，紧邻 `Mesh`。
- **源类别继续用 `type_key`**，不新增平行的 `BodyClass` 枚举（第二份真相）。
- `3DFACE`/`POLYFACE`/`POLYGON MESH`/`MESH` **保持 `Mesh`**：它们是面片/细分网格，
  不是 B-Rep，不得伪造 `exact`。
- `REGION` 与零厚度 ACIS 曲面都是 `BodyKind::Sheet`；`type_key` 决定 UI/导出写
  `REGION` 还是 `SURFACE`。
- `BodyKind::Solid` **要求** `BodyExact::Exact` 且 B-Rep 非空；`cad-db` 强校验。
  解析失败/未保留时只能是 `Unknown` + `Unavailable { reason }` + 显示网格——
  这正是去掉今天"3DSOLID 报告 Verified/Verified"过度声明的地方。
- `Wire` 允许只有边线（`display.mesh` 为空）。

### D3 中性 B-Rep 数据下沉到 `cad-domain`

- `BrepData`/`BrepShell`/`BrepFace`/`BrepLoop`/`BrepCurve`/`BrepSurface`/`BrepPlacement`
  移入 `cad-domain`（纯数据、基于 `Point3`），`cad-kernel-adapter` 保留
  `pub use` 兼容路径与**评估/剖分**职责（`BrepTessellator` 等）。
- 理由：`cad-db` 只允许依赖 `cad-domain`（`scripts/check-architecture.py:380`），
  精确几何必须能被 DB 拥有、校验、变换与包围盒计算；sidecar 表做不到。
- **不使用**不透明 `BodyHandle`/二进制 blob：handle 违反"内核句柄不过接缝"，
  blob 则无法校验/变换/导出，等于把"未验证"扩散到所有载体。

### D4 精确几何与显示缓存分开，且显示缓存是派生的

- `BodyDisplay` 是缓存，永远可由 `exact` 重建；命令不得原地改它。
- 一次写入同时替换 `exact` 与 `display` 两半（同一个 `DbEntity` 行），避免跨表漂移。
- 变换：相似变换（均匀缩放 + 旋转/反射 + 平移）更新 `BrepPlacement` 并变换显示；
  非均匀缩放**不烘焙网格**，降级为 `Unavailable { reason: "body.nonuniform_scale" }`，
  `Solid` 同时降为 `Unknown`。

### D5 显示：存储边线，不再从三角形重推

- 新增 `DisplayPrimitive::MeshWithStoredEdges { mesh, edges }`，仅 `Body3d` 产出；
  **不要**复用语义为"文字 glyph"的 `MeshWithoutEdgeOverlay`。
- `cad-scene` 对这类图元直接用存储边线填 `RenderBatch.edges`，跳过逐三角形重推，
  顺带去掉内部重复边（今天 `cache.rs:416` 每个三角形三条边都发）。
- `BodyKind::Wire` 且无网格时发 `LineSegments`。

### D6 视觉样式属于会话，不属于数据库

- `VSCURRENT` 与 `FILL`/`QTEXT`/`VIEWRES` 同层（`cad-representation` 的显示模式 +
  `cad-ui-core` 会话校验），不写入 `cad-db`、不推进 revision。
- `RenderTopology::MeshEdges`（今天从未被场景产出）作为 2D wireframe 的落点。
  P0 不实现视觉样式，只保留该方向。

### D7 分阶段（诚实的能力增长）

- **P0（本轮）**：`Body3d` + 中性 B-Rep 下沉 + `cad-db` 校验/变换/包围盒 +
  导入保留 `exact`/`edges` + 表示/场景用显示缓存 + 全量 match 分支显式化。
  不接 solidcore、不加命令、不加 NURBS。**P0.1（已完成）**：表示层新增
  `MeshWithStoredEdges`，把 `BodyDisplay.edges` 直接送进 `RenderBatch.edges`，替换
  当前由三角形重推的边线覆盖层；渲染器改为按 `edges` 位置画线。
- **P1**：`cad-kernel-solidcore` + `SolidModeler`；基本体（BOX/CYLINDER/SPHERE/
  CONE/TORUS/WEDGE/PYRAMID）、UNION/SUBTRACT/INTERSECT、`3DSOLID`/`BODY`/`REGION`
  命令注册、体积/质心/惯性、由 `BrepFace.id` 产出 `face_sources`、`VSCURRENT`。
- **P2**：圆角/倒角/抽壳；`BrepSurface::Nurbs`（有理 B 样条 + 权重）与 NURBS 显示；
  `SURFACE` 家族（PLANESURF/EXTRUDEDSURF/LOFTEDSURF/REVOLVEDSURF/SWEPTSURF）。
  曲线/曲面评估优先走 solidcore 服务，避免在域内再写第二套评估器。
- **P3**：ACIS SAT/SAB 写出（今天完全没有），才谈得上 `3DSOLID` 往返。

## 4. 后果与风险

1. **match 分支扩散**：新增变体会让约 26 个生产文件、48 个文件（含测试）各加一条
   显式分支。这是本决策的**目的**：编译器强制每个消费者表态，而不是把 body 当三角汤。
   禁止用 `_ => {}` 兜底 `Body3d`。
2. **导出回归**：DWG 导出今天把离散 ACIS 写 `POLYFACE`。P0 必须保持字节同构，并在
   报告中明确"精确 B-Rep 未写出"，不得写成 `3DSOLID`。
3. **双真相**：靠 `BodyDisplay` 是缓存 + 单行同时替换 + `Solid ⇒ Exact` 不变量约束；
   P2 再引入 `Arc<BrepData>` 共享以控制历史内存。
4. **wasm**：solidcore 是大型纯 Rust 依赖且无取消钩子；wasm 无线程。P1 起按载体报告
   可用性，不可用即拒绝；`cargo check --target wasm32-unknown-unknown` 进 P1 门禁。
5. **solidcore 波动**：API 自陈 experimental、docs 有过期 `Unported` 注释、镜像仓每日变动。
   唯一的隔离手段是 `cad-kernel-solidcore` + 固定 revision + 转换层测试；任何
   "已支持" 必须由本地运行证据支撑，不能引用上游 golden。

## 5. 未验证

- solidcore 任何操作的实际运行结果（本轮未执行）。
- 圆角产物是否为有理 NURBS（决定 P2 能否无损表达）。
- 裁剪环的绕向与曲面法向关系（rustdoc 未声明）。
- wasm32 上的栈需求与浏览器运行（只有 `cargo check` 级别的上游证据）。

## 6. 验证方式

- P0：`cad-domain`/`cad-kernel-adapter`/`cad-db` 的单元与契约测试；
  `Body3d` 的 `Solid ⇒ Exact`、非均匀缩放降级、存储边线、包围盒。
- P1 起：`cad-kernel-solidcore` 的转换层测试（`BrepData ↔ solidcore Brep`）+ 一条
  真实运行链（create → boolean → mass properties → tessellate），失败必须显式。

## 7. P1/B2 落地事实（2026-11-17）

本轮只落**一个 crate** 与它的文档/门禁，不接命令、不改 `cad-app`（那是 P1/C）。

| 项 | 事实 |
|---|---|
| crate | `crates/cad-kernel-solidcore`（唯一允许命名 `solidcore` 的包，`#![cfg(not(target_arch = "wasm32"))]`） |
| 依赖 | `solidcore-ops` + `solidcore-record`（`ops` 传递引入内核与 `solidcore-convert`），git revision `55ae49ef919b606bf77901df626ed9bb543e5212`，Apache-2.0，目标表 `cfg(not(wasm32))`；无 `[patch.crates-io]` |
| 注册 | `type_key = yacr.kernel.solidcore`，`priority = 10`，`entity_types = 3DSOLID/BODY`（**只有**这两类：`REGION`/`SURFACE` 是零厚度 sheet body，本切面既不创建也不接受为操作数，声明它们会过度承诺；P2 的 SURFACE 家族再加）；`Registration::version` 是 `u32`（本适配器契约版本 `1`），上游版本串 `1.0.1+55ae49ef919b` 由 `SolidCoreModeler::UPSTREAM_VERSION` 与 capabilities 声明 |
| 能力 | primitives 全真（7 族）、booleans 真、mass_properties 真；fillet/shell/nurbs 假。**booleans 是 family 级声明**：单个操作数不可 lift（曲面 body、开放 sheet、多于一个 shell、空腔、结构不一致）时，该次调用返回 `Unsupported` 并带稳定码，调用方**必须**逐次处理，不得把 family 级 `true` 当成「每个操作数都能算」 |
| 转换 | `BrepData → record`：仅平面面 + `Line` 环、单 shell、可烘焙 placement；`record → BrepData`：Plane/PlaneThrough/Cylinder/Cone/Sphere/Torus + Line/整圆 |
| 精确性 | 作者化 box/wedge/pyramid 由内核 `Body::from_record(.., false)` 验证后才交出；体积用闭式断言（1.0 / l·w·h、l·w·h/2、s²·h/3） |
| 结构一致性 | lift 之后除 record 的 `validate` 还做：每条边**恰好两次**使用且两个面在**壳的朝向**下反向遍历（拒绝反向面/重复面/把 sheet 当 solid）、每个环按精确位首尾相接、以及 `solidcore_ops::record_check::agreement` 的几何一致性检查。任一失败 → `Unsupported` + `solid.lift_unsupported`（不是「可算但算错」） |
| 拒绝码 | `solid.lift_requires_planar` **只**用于两处非平面（非 `Plane` 面 / 非 `Line` 边）；结构/退化/空/容差/记录校验 → `solid.lift_unsupported`；placement 或平面 frame 不可烘焙（镜像、非正/非有限缩放、奇异或含剪切矩阵）→ `solid.placement_not_supported`；非相似变换 → `solid.nonuniform_transform` |
| 内核错误 | `Unsupported`/`Unported` → `Unsupported`；`UnsureRegion` → `Failed(solid.unsure_region)`；`Empty` → `Failed(solid.empty_result)`；`Internal`（会话捕获的内核 panic）→ `Failed(solid.kernel_panic)` 且**不回传 panic 文本**；`Cancelled` → `Err(Cancelled)` |
| 质量属性 | `Body::mass_properties(1e-6)`，报 `MassAccuracy::Requested`（估计，不证明），**不**标 `Analytic` |
| 取消 | 前/后检查 + 布尔用 `solidcore_ops::cancel::{CancelToken, within}`（看门狗线程每 50 ms 轮询接缝回调，即**检测粒度**；调用结束用 `unpark` 立即唤醒，成功路径不额外等 50 ms；`StopGuard` 在 `Drop` 里置位并唤醒，内核调用 panic 也不会把 `thread::scope` 卡在睡着的看门狗上），`Error::Cancelled → CadError::Cancelled`，无 `aborted` 产物 |
| 证据 | `cargo test -p cad-kernel-solidcore`（真实内核）、clippy `-D warnings`、`check-architecture.py`；完整命令与退出码见 `docs/handoff.md` 2026-11-17 条目 |

**本轮学到并写进代码的事实**（不是上游文档的推断）：

1. 记录里的边曲线端点可能与它的顶点相差 1 ULP（booleans 产物见到
   `1.0` vs `0.9999999999999999`）。`record → BrepData` 必须用**顶点**而不是曲线端点，
   否则中性环不再精确闭合，反向转换会报 `disconnected loop`。
2. 面域（area/质心/惯性）是在**参数域**上按 pcurve 裁剪积分：用占位 pcurve 时体积仍正确
   但面积返回 `0`。因此 `BrepData → record` 写的是每条边在该面 `(u, v)` 里的**精确**一次
   spline 像，而不是占位。
3. 曲面 primitives 的记录形状：cylinder/cone 侧面是"两条整圆 + 接缝线"的一个环，sphere/torus
   是"只有接缝边"的单面。中性剖分器要求 cylinder/cone 每环一条整圆、sphere/torus **无环**，
   所以 `record → BrepData` 会先验证接缝（线在曲面上、球面端点在两极、环面边全部闭合）再丢弃它。
4. 上游 `1.0.1` 版本串在固定 revision 上没有变化，**不能**用它识别 revision。
5. **布尔的合法产物可能超出本切面的 lift 能力，且两个失效点不同**：
   - **空腔（void region）**：内核把腔存成 void region，`from_record` 不接，所以结果**在布尔本身**
     就被拒绝（`Unsupported` + `solid.lift_unsupported`）；
   - **多于一个 shell**（例如不相交的两个体 `UNION`）：`BrepData` 能表达，布尔**返回成功的 body**
     （2 个 shell），直到**下一次 lift**（质量属性或再次布尔）才被拒绝，因为 `to_record` 只接
     单闭合壳。所以"mass 需要恰好一个闭合壳"是调用方会遇到的真实约束，而不是布尔会提前拦住它。

   实测的反向症状是：反向面的 body 会给出**看似合理**的面积（40 vs 52），重复面的 body 会给出
   36 vs 24 的体积——所以 lift 之后必须做结构一致性检查，而不能只信 `record.validate`。
6. **质量属性要求恰好一个闭合壳**：空腔与多壳（例如另有一个分离的体）都不算；多壳的 body 是
   布尔返回的合法产物，`mass_properties` 在对它 lift 时返回 `Unsupported` + `solid.lift_unsupported`，
   绝不把两块的体积相加当结果。

**仍未做**（P2/P3）：NURBS 曲面/曲线与 pcurve 的中性表达、曲面 body 的布尔/质量属性、
圆角/倒角/抽壳、`SURFACE` 家族、ACIS SAT/SAB 写出、任何命令接线。
