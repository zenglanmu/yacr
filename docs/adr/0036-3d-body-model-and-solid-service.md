# ADR 0036：3D 实体模型（wireframe / surface / solid）与 solidcore 无状态建模服务

- 状态：**Accepted（2026-11-16）**，**P1 已实现到 C3；P2.0（倒角 + `CHAMFEREDGE`）、P2.1a（圆弧
  边 + B-Rep 容差）、P2.1b（NURBS 载体 + 曲面 lift）、P2.1c-1（NURBS 面显示：`solidcore-tess`
  接入显示接缝）、P3b（DWG SAB）、P3-版本选择器（ASM 21800 与 DXF AC1027+）已实现受限子集**
  （2026-11-17，B2 + C1 + C2 + C3a/C3b/C3c + P2.0/P2.1a/P2.1b/P2.1c-1/P3a/P3b/P3-选择器增补）：
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
    solid` 拒绝（共用内核互斥锁，不能阻塞命令线程），拒绝在模型器调用与事务之前。曲面操作数自
    **P2.1b** 起参与内核布尔（由 `nurbs` 能力位承载）；无 `(u,v)` box / 无精确 pcurve 的曲面面仍是
    `Unsupported`（`solid.lift_unsupported`）。
  - **P1/C3（2026-11-17，C3a + C3b + C3c）**：
    - **C3a `MASSPROP`**：精确 `Body3d{Solid, Exact}` 经**注入的**模型器**同步**测量
      （`SolidOperation::MassProperties`，density 1.0、请求相对精度 1e-6；报告明写“估计、
      不是已证明的界”，绝不标 `Analytic`），**显示网格永不积分**；稳定拒绝
      `massprop.body_modeler_unavailable` / `body_not_liftable` / `body_kernel_failed` /
      `body_not_exact` / `body_not_solid` / `mixed_selection` / `kernel_busy` /
      `body_not_positive_volume`；一次 `massprop.object` 行/体，
      无库写/无 revision/无历史；
    - **C3b `face_sources`**：`BrepTessellator` 为每个显示三角形写 `brep:face:<shell>/<face>`
      （`topology_revision = Revision(0)` 占位），body 拾取与子面高亮复用 mesh 路径；
      `cad-db` 只校验空或等长；
    - **C3c `VSCURRENT`**：会话级 3D body 视觉样式（见 D6），不写 `cad-db`。
  - **P2.0（2026-11-17）**：**倒角模型器能力**（不是命令）。`cad-kernel-adapter` 新增
    `SolidOperation::Chamfer { brep, selector: SolidEdgeSelector::NearestToPoint(p), distance,
    second }`、`SolidOperationFamily::Chamfer` 与 `ModelerCapabilities.chamfer`（seam 默认
    `false`；solidcore 模型器对**平面体**声明 `true`）。`cad-kernel-solidcore::ops::chamfer`
    走与布尔同一条 lift 门（`BrepData → record → Body::from_record`）和同一个取消看门狗
    （抽成 `ops::watched`），调用上游 `solidcore_ops::chamfer_sides_ordered`；边由"离给定点最近的
    `Line` 记录边"解析（容差 `max(1e-9, 1e-3 × 包围盒对角线)`，否则 `solid.edge_not_found`），
    `along` 面取该边在面下标最小的面上的 use，`second` 缺省与 `distance` 相同（对称 45°）。产物仍然全平面，
    因此可再次 lift；上游把倒角的直线边写成**共线三次 Bézier**，`record → BrepData` 只在
    控制点全部落在弦上（容差内）且弦端点为该边顶点时判为 `Line`（精确，不是 NURBS lift），
    其它样条仍拒绝。**同轮把 `CHAMFEREDGE` 接成命令**（一次边拾取、种子取 `PickHit.point`，
    详见下方「P2.0 接缝」）；NURBS 显示、圆角、
    SHELL、`SURFACE` 家族仍未实现（曲面 lift 由 P2.1b 补齐）。证据：Linux 契约/单元（真实内核）。**实测范围**：凸 **90°
    平面边**（单位 box、wedge 的底/背面交线；含 `distance=0.1` / `second=0.2` 非对称）；
    **凹边与非 90° 二面角未测**，能力声明不覆盖它们。
  - **P2.1b（2026-11-17，已实现）**：`cad-domain` 增加 `BrepSurface::Nurbs { u_knots, v_knots,
    points }`（齐次控制网、隐式次数、无周期性标志）与 `BrepFace.domain`；record `Spline`/
    `RationalSpline` 读成 `Nurbs`（`Spline` ⇒ `w=1`），`BallPatch`/`PlacedRevolution`/`Blend`
    与样条/椭圆边界曲线仍按名拒绝；写侧把 `Cylinder`/`Cone`/`Sphere`/`Torus` 面带解析 label +
    `solidcore::eval::form::surface` 的**精确有理 NURBS form** + form 参数 `(u,v)` box + 精确
    pcurve（`pcurves_within`，未命中或偏网格裁剪 ⇒ `solid.pcurve_not_exact`）回写 record（纬圆先
    重标到面自己的 frame，再由 `verify_pcurves` 的 6 样本/1e-9 相对比对证明；**完整圆锥**的
    apex band 也写出）；
    `ModelerCapabilities::nurbs = true` 且**承载**（曲面操作数缺该位 ⇒ `solid.nurbs_not_supported`）。
    **显示仍不完整**：NURBS 面没有网格器（`solidcore-tess`，P2.1c），被剪裁侧壁报缺面。详见 §10。
  - **曲面 body**（含 cylinder/cone/sphere/torus 面）的布尔、质量属性与倒角自 **P2.1b** 起**可参与**
    （由 `nurbs` 能力位承载）；仍按名拒绝的是无 `(u,v)` box 的曲面面、无精确 pcurve 的边界、
    多壳/空腔等结构性原因（`solid.lift_unsupported`），以及未声明 `nurbs` 的模型器
    （`solid.nurbs_not_supported`）；曲面**边**倒角与圆角仍是 P2.1c/P2.2；
  - **P3a（2026-11-17，已实现一个受限子集）**：DXF 导出把**单壳、全平面、直线边界、真相似
    放置**的精确 `Body3d{Solid, Exact}` 写成真 `3DSOLID`，载荷是本仓库用 acadrust SAT 构建器
    生成的 **ACIS SAT 文本**（`crates/cad-import-acadrust/src/export/acis.rs`）。其余 body
    （曲面面、非直线边界、多壳/空腔、剪切/镜像放置、开放/非流形/反转拓扑）**按名拒绝**并退回
    既有 POLYFACE 显示缓存：`acis.curved_face_not_written`、`acis.non_line_curve_not_written`、
    `acis.no_shell_not_written`、`acis.multi_shell_not_written`、`acis.placement_not_written`、
    `acis.invalid_topology_not_written`（这六个码在 DXF 与 DWG 下相同）。**P3b（同日）**：同一份
    `SatDocument` 也用 `SabWriter` 写成**二进制 SAB**，DWG 的 `3DSOLID` 把它放进 **AC1032 的
    AcDs（`AcDsPrototype_1b`）段**，记 `export.note.acis_sab_dwg` / 状态行
    `save.exported_acis_sab[_partial]`；**没有 SAB 专用拒绝码**（转换不可失败，见「P3b 决策」）。
    因为 `3DSOLID` 的 ACIS 载荷只在 AC1027 之前内联为 SAT 文本，**只要文件含任一 3DSOLID
    （含块定义内）整个 DXF 按所选容器写出**并记 `export.note.acis_sat_text_version`（R2010）或
    `export.note.acis_sab_dxf`（R2013/R2018）（SAVEAS 状态行同义）——这是**文档级**约束，未来任何
    需要另一容器的实体特性必须重审该路径（**仅 DXF**；DWG 保持 AC1032）。证据：本仓库自写→acadrust
    解析→自读往返（盒/楔/棱锥/
    带孔面；拓扑计数、面参数精确、几何 ≤1e-9·对角线、`BrepTessellator` 体积 ≤0.1%）+
    `solid_command` 的真实 solidcore BOX 存盘/再导入（精确体、体积 1±1e-3）+ P3b 的
    DWG 写入→`DwgReader` 读回（AcDs）→`sab_to_brep` 同一精确体；**无第三方 CAD 读取器、无
    vendor/像素/非 Linux 证据**（默认载荷仍是 ACIS 7.0（700，非 ASM）；700 与 21800 两个版本、
    以及 AC1027+ 容器的 vendor 接受性均未验证，见 §12）。
  - **P3-版本选择器（2026-11-17，见 §12）**：容器与载荷版本不再是隐含常量。`ExportRequest` 新增
    `acis_version`（`V7_0` = 700、`V21_0` = ASM 21800）与 `dxf_target`（`R2010` = AC1024、
    `R2013` = AC1027、`R2018` = AC1032），默认仍是 700 + R2010（即此前的内联 SAT 文本路径）；
    宿主设置键 `acisVersion` / `dxfVersion` 经同一条 Save As 请求到达导出层。矩阵在写任何记录前
    拒绝：`DxfText + R2010 + V21_0` → `export.acis_asm_requires_r2013`；`Dwg + 非 R2018` →
    `export.dwg_version_fixed`（DWG 的版本由文档决定、恒 AC1032，绝不被静默套用）。DXF
    R2013/R2018 把所选载荷（700 或 21800）作为二进制 SAB/ASM 块写进 `ACDSDATA`，R2010 仍内联
    SAT 文本。**副作用**：DXF 容器的**默认**由「无实体时 AC1032」变为 **AC1024**（此前含实体时
    本来是 AC1024——这是把默认固定到同一条已证路径，不是新的写入能力）。
  - **仍是 P2 及以后**：AutoCAD 的交互式尺寸提示；**圆角**（P2.1c-2，需先实测 blend 产物）与
    曲面**边**倒角；`SURFACE` 家族（P2.2）；`BODY`/`REGION`/`SURFACE` 写回；曲面/NURBS/一般裁剪
    B-rep 的 ACIS 写出（版本选择器**不**扩大可写子集，仍是那六个 `acis.*_not_written` 码）。
    P2.0 的倒角、P2.1a 的圆弧 lift 与 P2.1b 的曲面 lift 已实现，且倒角已接成 `CHAMFEREDGE` 命令
    （模型器能力不再是不可达）；证据只有 Linux 契约/单元。
    `UNION`/`SUBTRACT`/`INTERSECT` 已是 C2 后台任务（见下）；
    `3DSOLID`/`BODY` 是**实体类型键**（模型器在 `Registration::entity_types` 声明），不新增同名命令。
  - **阶段总结**：**P0/P0.1/P1（内核 + C1/C2/C3）已实现**；**P2.0（倒角 + `CHAMFEREDGE` 命令）
    已实现**；**P2.1a（`BrepCurve::Arc` + B-Rep 容差）已实现**；**P2.1b（NURBS 载体 + 曲面 lift）
    已实现**；**P2.1c-1（NURBS 面显示接缝 + `solidcore-tess`）已实现**；**P3a/P3b（平面子集
    SAT 文本与 DWG SAB）与 P3 版本选择器已实现**；**P2.1c-2（圆角）与 P2.2（`SURFACE` 家族、
    抽壳、曲面边倒角）未实现**。
  验证边界：只有 Linux x86-64 上的契约/单元证据——测试进程里跑的是**真实**固定 revision 的
  `solidcore` 内核（无 mock、不引用上游 golden）；Windows/macOS/Android/Web 的 solidcore
  运行状态与模型器注入、任何 GPU/像素/性能结论一律 **NOT VERIFIED**。
- 日期：2026-11-16（P1/B2 2026-11-17，P1/C1 2026-11-17，P1/C2 2026-11-17，P1/C3 2026-11-17，
  P2.0/P2.1a/P3a/P3b 2026-11-17）。
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

- `VSCURRENT` 与 `FILL`/`QTEXT`/`VIEWRES` 同层（`cad-app` 会话状态 + `cad-host` 映射），
  不写入 `cad-db`、不推进 revision、不入历史。
- **落地（C3c，2026-11-17）**：3D body 的 2D wireframe 落成
  `DisplayPrimitive::LineSegments` → `RenderTopology::Lines`（有存储 body 边线就用边线，否则用
  去重的三角形边段）；`RenderTopology::MeshEdges` **仍未被该路径使用**（渲染器支持该拓扑，
  但 body wireframe 不走它）。
- **默认 SHADED**：本构建**刻意偏离** AutoCAD 的 2D-wireframe 初始值——`Shaded` 正是 token
  之前的填充网格显示，改成 2D wireframe 会改变所有既有 body 的显示。接受
  `WIREFRAME`/`2DWIREFRAME`/`2D` 与 `SHADED`/`SHADEDWITHEDGES`，其余样式
  `vscurrent.style_not_implemented`；会话级、**仅 body**，保存与矢量出图不接收该标志。
- 证据：`cad-app`/`cad-host`/`cad-representation`/`cad-scene` 契约测试；**无 GPU/像素证据**。

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
  **P2.0（已实现，含命令接线）**：平面体上的单边直线倒角（`SolidOperation::Chamfer`）与
  `CHAMFEREDGE` 一次边拾取（种子 `PickHit.point`，原地改写）；其余 P2 内容仍未实现。
  **P2.1a（已实现）**：`BrepCurve::Arc`（跨度 `(end_param − start_param).rem_euclid(2π)`；
  跨度 ≤ `1e-9` 是 `arc-without-span`，**v400 SAT 的 Arc 边不可导入**）与
  `BrepData.tolerance`/`BrepFace.tolerance`（**模型单位长度**、`Option`、`None` 不臆造；
  只有 `record::bake` 按 placement 缩放一次）；圆弧边界的**平面**面可 lift/离散/回写。
  **P2.1b（已实现，见 §10）**：NURBS 载体 + 曲面 lift（弧边界曲面侧面随之可 lift，但显示仍缺面）。
  **P2.1c-1（已实现，见 §11）**：NURBS 面的**显示**经 `SolidTessellator` 接缝接入
  `solidcore-tess`（同 pin、publish=false）；只有**含 NURBS 面**的 body 委派给内核网格器，
  解析体保持中性离散器（不churn），缺面/超差仍具名。**圆角（P2.1c-2）仍未实现**。
  曲线/曲面评估优先走 solidcore 服务，避免在域内再写第二套评估器。
- **P3**：ACIS 写出。**P3a（已实现）**：平面子集的 SAT 文本（DXF `3DSOLID`）；**P3b（已实现）**：
  同一子集的 SAB 二进制与 DWG `3DSOLID`（AC1032 AcDs 段），见上方状态块与「P3b 决策」。
  **P3-版本选择器（已实现，见 §12）**：`acisVersion`（700 / ASM 21800）与 `dxfVersion`
  （2010/2013/2018），不可能的容器×载荷组合按名拒绝。
  **仍未实现（显式拒绝，不新增码）**：曲面/NURBS/一般裁剪 B-rep 的写出
  （`acis.curved_face_not_written` / `acis.non_line_curve_not_written`）、`BODY`/`REGION`/
  `SURFACE` 写回。

## 4. 后果与风险

1. **match 分支扩散**：新增变体会让约 26 个生产文件、48 个文件（含测试）各加一条
   显式分支。这是本决策的**目的**：编译器强制每个消费者表态，而不是把 body 当三角汤。
   禁止用 `_ => {}` 兜底 `Body3d`。
2. **导出回归**：DWG 导出把离散 ACIS 写 `POLYFACE`（P0 保持字节同构，报告明确"精确 B-Rep
   未写出"）。**P3b 起**平面子集改写真 `3DSOLID`（DWG 的 SAB 在 AC1032 AcDs 段），其余体仍
   退回 `POLYFACE` 并逐个记 `acis.*`——不得把"退回了显示缓存"写成 `3DSOLID`。
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
| 能力 | primitives 全真（7 族）、booleans 真、mass_properties 真、chamfer 真（P2.0，**平面体**上的单边直线倒角）、offset_face 真（2026-11-25，`SolidOperation::OffsetFace`：单面沿外法向的带符号移动 = PRESSPULL 的原子操作，登记 `offset.face`；曲面面同样可移动）、extrude_profile 真（2026-11-25，`SolidOperation::ExtrudeProfile`：闭合平面轮廓 → 封顶实体，登记 `extrude.profile`）、nurbs 真（P2.1b，登记 `nurbs.faces`，**承载**曲面操作数）；fillet/shell 假。**booleans / chamfer / nurbs 是 family 级声明**：单个操作数不可 lift（开放 sheet、无 `(u,v)` box 或无精确 pcurve 的曲面面、多于一个 shell、空腔、结构不一致）时，该次调用返回 `Unsupported` 并带稳定码，调用方**必须**逐次处理，不得把 family 级 `true` 当成「每个操作数都能算」 |
| 转换 | `BrepData → record`：平面面 + `Line` 环，**自 P2.1b 起**解析曲面面带解析 label + `eval::form` 精确 NURBS form + `(u,v)` box + 精确 pcurve（`Line`/整圆/圆弧），单 shell、可烘焙 placement；`record → BrepData`：Plane/PlaneThrough/Cylinder/Cone/Sphere/Torus + **`Spline`/`RationalSpline` → `Nurbs`** + `Face.domain` + Line/整圆/圆弧 |
| 精确性 | 作者化 box/wedge/pyramid 由内核 `Body::from_record(.., false)` 验证后才交出；体积用闭式断言（1.0 / l·w·h、l·w·h/2、s²·h/3） |
| 结构一致性 | lift 之后除 record 的 `validate` 还做：每条边**恰好两次**使用且两个面在**壳的朝向**下反向遍历（拒绝反向面/重复面/把 sheet 当 solid）、每个环按精确位首尾相接、以及 `solidcore_ops::record_check::agreement` 的几何一致性检查。任一失败 → `Unsupported` + `solid.lift_unsupported`（不是「可算但算错」） |
| 拒绝码 | `solid.lift_requires_planar` **只**用于 `BrepData → record` 里非 `Line`/整圆/`Arc` 的边界曲线与 `Unsupported` 曲面种类（P2.1b 起解析曲面面不再走此码）；结构/退化/空/容差/记录校验 → `solid.lift_unsupported`；曲面面缺 `(u,v)` box 或某个 use 无精确 pcurve → `solid.lift_unsupported` / `solid.pcurve_not_exact`；模型器未声明 `nurbs` → `solid.nurbs_not_supported`；placement 或平面 frame 不可烘焙（镜像、非正/非有限缩放、奇异或含剪切矩阵）→ `solid.placement_not_supported`；非相似变换 → `solid.nonuniform_transform`。倒角：选择点离所有边都远 → `solid.edge_not_found`；内核建不出该倒角 → `solid.chamfer_failed`（两者都是 `Unsupported`）。单面移动：选择点离所有**裁剪面**都远 → `solid.face_not_found`；内核建不出该移动（无空间/折叠/无效果）→ `solid.offset_face_failed`。轮廓拉伸：轮廓未闭合/自交/无法封顶 → `solid.extrude_profile_failed` |
| 内核错误 | `Unsupported`/`Unported` → `Unsupported`；`UnsureRegion` → `Failed(solid.unsure_region)`；`Empty` → `Failed(solid.empty_result)`；`Internal`（会话捕获的内核 panic）→ `Failed(solid.kernel_panic)` 且**不回传 panic 文本**；`Cancelled` → `Err(Cancelled)`。**倒角另有映射**（`error::from_chamfer`）：布尔对应的 `Argument`/`Status`/`NoRoom`/`TooLarge`/`NoEffect` → `Unsupported(solid.chamfer_failed)`（几何上建不出来，不是 `Failed` 的空成功）。**单面移动另有映射**（`error::from_offset_face`）：`NoRoom`/`WallsTouch`/`NoEffect`/`WallFolds`/`OffsetRefused`/`OffsetPastTolerance`/`NotExact`/`Degenerate` → `Unsupported(solid.offset_face_failed)`，内核完整性错误仍是 `Failed(solid.kernel_failed)` |
| 质量属性 | `Body::mass_properties(1e-6)`，报 `MassAccuracy::Requested`（估计，不证明），**不**标 `Analytic` |
| 取消 | 前/后检查 + 布尔与倒角共用 `ops::watched`（内部是 `solidcore_ops::cancel::{CancelToken, within}`；看门狗线程每 50 ms 轮询接缝回调，即**检测粒度**；调用结束用 `unpark` 立即唤醒，成功路径不额外等 50 ms；`StopGuard` 在 `Drop` 里置位并唤醒，内核调用 panic 也不会把 `thread::scope` 卡在睡着的看门狗上），`Error::Cancelled → CadError::Cancelled`，无 `aborted` 产物 |
| 证据 | `cargo test -p cad-kernel-solidcore`（真实内核）、`cargo test -p cad-kernel-adapter`、clippy `-D warnings`、wasm `cargo check`、`check-architecture.py`；完整命令与退出码见 `docs/handoff-archive/2026-11.md` 的「迁移快照」（2026-11-17） |

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

**仍未做**（P2.1c/P2.2 与 P3 剩余）：NURBS **面的显示**（无网格器，`solidcore-tess`）、圆角、
抽壳、`SURFACE` 家族、`BODY`/`REGION`/`SURFACE` 写回、曲面/NURBS/一般裁剪 B-rep 的 ACIS 写出、
ASM（21800）载荷、DXF AC1027+ 的 SAB 输出（本构建**没有版本选择器**）。曲面 body 的布尔/质量属性/
倒角、NURBS carrier 与 pcurve 已在 P2.1b 落地（见 §10）。（命令接线 C1/C2/C3a/C3b/C3c 与 P2.0 的
`CHAMFEREDGE` 已落地，见上方状态块与 D6。）

## 8. P2.0 接缝：`CHAMFEREDGE` 用拾取命中点，不用工作平面

`CapturePoint(PointInput::CanvasLocal { logical_position, .. })` 本来就携带逻辑像素，但历史上的
宿主路径走 `world_point` → `Viewport::screen_to_world`，会把 `z` 硬写成 `work_plane.origin.z`。
在 3D/透视视图里那是一个**编造的点**。`CHAMFEREDGE` 因此用**既有拾取漏斗**
（`pick_at_screen_with_visibility_and_vendor`）解析它的唯一一次拾取并取 `PickHit.point`；body 来自
当前选择（面引用折叠到所属 body，实例/块路径拒绝）。种子以 `SolidEdgeSelector::NearestToPoint`
传下去，边由**内核自己的容差**选择，`cad-app` **不添加第二套容差**。没有子边身份、没有边高亮、
没有预览几何、没有圆角、没有链式，也没有新的内核能力。命令原地改写被选中的行（一次 revision、
一步撤销），内核的具名答复（`solid.lift_requires_planar` / `solid.edge_not_found` /
`solid.chamfer_failed` / `solid.kernel_failed`）**原样透传，不加前缀**。

可达拒绝码（命令路径）：`unsupported-runtime: solid-modeler`、`chamferedge.selection_required`、
`chamferedge.selection_multiple`、`chamferedge.not_addressable`、`chamferedge.not_a_solid`、
`chamferedge.not_exact`、`chamferedge.not_model_space`、`chamferedge.selection_changed`、
`chamferedge.distance_invalid`、`chamferedge.pick_missed`、`chamferedge.pick_not_selected_body`、
`chamferedge.kernel_busy`、`chamferedge.unexpected_mass`，以及类型化但无载荷调用的
`chamferedge.requires_payload`。`chamfer_edge_target` 是宿主工具启动门、点击匹配器与命令**共用**
的唯一选择/body 解析器；`PickHit` 从 `cad_app::picking` 重导出，宿主不必为同一漏斗再引入
spatial 依赖。

**操作数约束（评审后补）**：两个退距必须 `> 0`；通道默认 `0/0`（对 2D `CHAMFER` 合法）下第一次
回车就是具名 `chamferedge.distance_invalid`，**拾取保留**，状态/提示行已写明原因。
`chamferDistanceA/B` **按设计**与 2D `CHAMFER` 共享：`ChamferMethod`/角度/`trim` 是 2D-only
概念，CHAMFEREDGE 总是把**两个值都**当 3D 退距消费（2D ANGLE 模式留下的旧 `B` 会成为真实退距）。
工具启动还把 Work-only 命令的权限门放在模型器/选择门**之前**，所以 Viewer 调用在两条路径上都是
同一个 `PermissionDenied`（此前会产生两个不同码）。

## 9. P3b 决策：DWG 的 `3DSOLID` 用 `SabWriter` + `Solid3D::from_sab`

**决定**：DWG 的附着路径是显式的 `SabWriter::write(&sat)` → `Solid3D::from_sab(bytes)`（DXF 仍是
`Solid3D::from_sat(text)`），而不是让上游从 SAT 文本二次解析。

**理由（acadrust 0.6.8 源码实测）**：`write_solid3d` 先算
`pending_has_ds_data = needs_acds_section() && e.acis_data.contributes_sab()`；`contributes_sab`
两条路径都满足，但 `queue_sab_entry` 的 **SAT 分支**会再解析文档，**解析失败时静默不 push，而
`has_ds_data` 已经是 true**——那会留下一个 AcDs 里没有载荷的孤儿 `3DSOLID`。`from_sab` 把
`is_binary = true`、`version = Version2`、`sat_data` 置空，正是 writer 的 AC1027+ 分支期待的
形状；载荷由**我们持有的字节**决定，不依赖 writer 内的一次解析。今天 `brep_to_sat` 已先
`validate()`，所以 SAT 分支的失败态够不到，但它仍是**不用** SAT 文本走 DWG 的原因。AcDs 段由
`needs_acds_section = version.r2013_plus(document.version)` 触发，而 `document.version` 是
**AC1032**——这也是 DXF 的 AC1024 钉死规则**不得**作用于 DWG 的原因。测量事实：1×2×3 的盒附着
5271 字节，读回后的 blob **字节数相同**、解析为 6 面、体积精确 6。

DWG 覆盖**与 DXF 相同的平面子集**；子集外的体在两种格式下都退回显示缓存并记同一组六个
`acis.*_not_written` 码——拒绝是**体**的属性，不是容器的属性。**没有** SAB 专用拒绝码：
`SabWriter::write` 返回 `Vec<u8>`（不失败），输入是本仓库已 `validate()` 的同一份 `SatDocument`，
一个不可达的码不写进文档。

**状态与限制**：默认 SAB 头是 **ACIS 7.0（700，非 ASM）**（构建器是 `SatDocument` 的
`SatHeader`，与 DXF 路径同源）；自 §12 起载荷版本可由 `acisVersion` 选择 ASM 21800。
`SatVersion::V21_0` 的数字是 **2100**（`major*100 + minor*10 + patch`），低于 `SabWriter` 的 ASM
阈值 21800，所以 21800 必须用 `SatVersion::from_sat_number(21800)` 构造（字段 218/0/0，产品串由
上游拼成 `ACIS 218.0`）。AutoCAD/ODA/任何 vendor 是否接受我们的 700 SAB 或 21800 ASM
**未验证、不声明**。
**第二项未验证的 vendor 风险**：每个 AC1027+ `3DSOLID` 还带一个 `acis_revision` 块，而我们把它
**置零**写出（`has_guid = false`、全零 GUID）——`Solid3D::from_sab` 设 `AcisRevision::default()`，
`write_solid3d` 无条件写该块。acadrust 自己的文档要求该块在写出时保留，省略会破坏实体流（文件
在 ACAD/TrueView/BricsCAD 里打不开）；我们**没有**可保留的源 revision，且**没有测试能抓到**
（往返只走 acadrust 的宽松读取器）。这是与 ASM 头并列的、**未验证的 vendor 接受性风险**，不声明。
DWG 往返只经 acadrust **自己的** `DwgReader`（`AcDb:AcDsPrototype_1b` → `SabReader` →
`sab_to_brep`）。AcDs 记录表读取器需要 ≥2 条记录才用 handle 表，单实体文件按位置附着（已实测
单盒、单块成员与三个夹具）；多实体只由混合文档（solid + MTEXT）与块用例覆盖，**没有**多实体
压力夹具。写出 ACIS 总会丢弃显示缓存，报告恒 `Partial`，所以**干净**的
`save.exported_acis_sab` / `save.exported_acis_sat` 目前不可达（键保留，UI 不声称它们出现）。
证据只有 Linux 契约/单元 + 本仓库自写/acadrust 自读往返；**没有** GPU、像素、非 Linux 载体或
第三方 CAD 读取器证据；`cad-cli-tools` **没有**导出 DWG 的路径，所以“真实内核 → DWG”这一用例
不存在、也没有被伪造（NOT RUN）。

## 10. P2.1b 决策：曲面 lift 写「label + 精确 form + 精确 pcurve」

**背景**：P2.1a 之后（P2.1b 之前），圆弧边界的**曲面侧面**（半圆柱壁、圆角槽侧壁）与所有曲面 body
的布尔/质量属性仍被 `solid.lift_requires_planar` 拒绝——不是因为我们不认识这些曲面，而是
`BrepData → record` 只会写平面。而 record 侧其实早就有 `SurfaceKind::BSpline`、`Face.nurbs`、`Face.domain` 与逐 use 的
pcurve 槽位；缺的是我们这一侧的写方与读方。

**决定**：P2.1b 一次性把这两个方向补齐，且**只用闭式解**：

1. **读侧（载体）**：`BrepSurface::Nurbs { u_knots, v_knots, points }`（齐次控制网 `[x·w,y·w,z·w,w]`、
   **隐式次数**、**无周期性标志**；权重有限且 `> 0`）与 `BrepFace.domain: Option<[[f64;2];2]>`。
   record `Spline` → `Nurbs`（`w = 1`）、`RationalSpline` → `Nurbs`（齐次系数），net/knots/weights
   逐位通过；`Face.domain` 是**生产者自己的** `(u,v)` 矩形，唯一来源，逐字带入。`BallPatch`/
   `PlacedRevolution`/`Blend` 与样条/椭圆**边界曲线**仍按名拒绝：没有 curve 变体、没有
   `BrepFace.approximation`，不把“没证明”的东西认证成解析曲面。record 的结构限制
   （2…4096 控制点/方向、次数 1…16、`knots.len() == count + degree + 1`）不在类型上断言，
   只在回写中性体时校验（只有它能分辨畸形网与不寻常网）。
2. **写侧（form）**：`Cylinder`/`Cone`/`Sphere`/`Torus` 面写**解析 label** +
   `solidcore::eval::form::surface` 给出的**精确有理 NURBS form**。不拟合、不采样：`eval::form`
   是内核里唯一把 label 与几何证明为同一曲面的地方，所以直接用它。完整圆边界
   （`BrepCurve::Circle`）也随之写出（`Curve::Circle` + 整圈 form + 一个 seam 顶点）；闭合边
   没有第二个顶点可读 `forward`，方向由共享该圆的**平面面**解析，两个非平面面共用时按名拒绝。
   **完整圆锥**（`radius_top = 0`）也能写：读回的退化 apex 环被写成内核 record 自己的 apex band
   （**一条母线用两次**、apex 作其对端顶点——几何仍是同一母线），因此 `V = πr²h/3`、
   `A = πr² + πr√(h²+r²)`、形心在 `h/4` 可测；**不是只有截头圆锥**。
3. **pcurve 只用精确闭式解，且精确性要被证明**：每个 use 的 pcurve 来自
   `solidcore_ops::pcurve::pcurves_within`——它只接受 kernel 能证明的闭式像，**绝不拟合**；
   未命中是新的稳定码 **`solid.pcurve_not_exact`**。理由已实测：用占位/拟合 pcurve 时，参数域上
   的面积积分返回 0（体积仍对，面积错），所以宁可拒绝一个面，也不写一个“能算但算错”的体。
   证明机制（**不是**“内核的少数样本”）：
   - 闭式解在内核里说 **label 参数**，record 存的是**面的 form 参数**，两者只在边界落在共享的
     **四分之一网格**上才逐项一致。写方因此把圆柱/圆锥的**纬圆重标到面自己的 frame**
     （`curved::align_curved_frames`，只移动参数零点、圆不动）。**不能**改用
     `solidcore::eval::form::on_frame` 去位移角度区间：整圈 pcurve 会不再覆盖 form 的 knot span，
     record 以 “pcurve interval differs from its edge's” 拒绝（已实测）。
   - `verify_pcurves` 对每个 use 在 **6 个样本（4 个内点）** 上比对 `pcurve(t)` 的面点与 `edge(t)`
     的点，按 `solidcore` 的 **1e-9 相对**（`1e-9·(1+max|坐标|)`）判“精确”；未命中 ⇒
     `solid.pcurve_not_exact`，诊断点名 **face / loop / use**、gap 与原因。
   - **偏离网格的裁剪**（旋转过的盒切圆柱等）没有精确闭式 pcurve：拒绝 `solid.pcurve_not_exact`
     并写明 “an exact closed form exists only where the boundary shares the face's quarter grid”，
     不把 record 的内部报错（“geometry disagrees with its topology”）漏给用户。**平面**面照旧
     lift（圆弧退回自己的 knots）。
4. **能力位承载**：`ModelerCapabilities::nurbs = true`（登记字符串 `nurbs.faces`）。布尔 / 质量
   属性 / 倒角任一操作数含非平面面时该位**必需**；声明 `false` 的模型器答
   **`solid.nurbs_not_supported`**（不是静默尝试、也不是泛化的 kernel 错误）。它是 **family 级**
   的：`true` 只表示这一族可用，单个操作数仍可能因无 box / 无 pcurve / 结构不一致而 `Unsupported`
   ——这与 `booleans`/`chamfer` 的既有约定一致，命令层必须逐次处理。**族先于几何**：族本身未声明的
   请求答 **`solid.unsupported`**，即使操作数带曲面也不报 `solid.nurbs_not_supported`（否则会把
   用户指向一个改能力位也修不好的原因）。

**其余具名拒绝**（全部 `Unsupported`，与上面同一条 record 方向，不属于 ACIS 导出）：
`solid.pcurve_not_exact` 同时覆盖“闭式解未命中”与“裁剪偏离四分之一网格”；`solid.lift_unsupported`
也用于 label 不在 `parameter_scale` 覆盖的四种解析族内、曲面 form 无法重标（不是 NURBS）、**圆弧边
没有有理 NURBS form 可投影进面**、同一整圆与一个**不包含它**的平面面共用（法向不平行，含垂直——
垂直曾被误读成“反向遍历”）、调用方给的 seam 点不在该圆的参考半径上。

**两处必须照实记录的偏差（都由实测逼出）**：

- **写入的 `(u,v)` box 由面自身边界推出**，不复制 `BrepFace.domain`。生产者 box 在**生产者自己的
  form 参数**里（球 `[0,1]²`、圆柱弧度），直接交给读 label 参数的 `eval::form` 会把每个球写成小片
  并拒绝。中性 box 的**存在**仍是生产者“这是被剪裁片”的声明：曲面面未声明 ⇒ `solid.lift_unsupported`
  （写方不发明参数盒）。修正的是“复制数字 → 得到错误的 form 范围”。
  `label_box` 是**包围盒**：by construction 比裁剪片宽（u 取整圈、v 取边界范围），**绝不可收紧**。
  球的 seam 是**零面积 bigon**，唯一的合法性来源就是整圈 u 盒——两极跳变把第二个 use 放在一个
  **周期**之外；收紧盒子会让两个 use 叠在一处，内核直接拒绝该面。
- **`eval::form` 的 knots 按每个角度方向 π/2 重标，但这不是“form 参数 = label 角度”**。
  `eval::form` 的参数 `q + quarter_u(ψ)` 在四分之一圈内**非线性**：重标后参数 `0.25` 对应
  `0.357 rad`，**不是** `0.393 rad`；任何代码都不得把 form 参数当角度用。重标只买到**单位**：
  一圈 = `2π`，于是 box 端点、解析曲面的 `Domain::period`（`2π`）与 seam 跳变同单位，落在四分之一
  线上的边界正好压在 knot 上。不重标时单位对不上（圆柱 seam 落在 `u = 2π` 而端圆在 `u = 4`，
  球面经线起点偏离边 0.65 mm）。pcurve 的精确性由第 3 条的 `align_curved_frames` + `verify_pcurves`
  保证，**不是**由这次重标保证。

**torus seam 的顶点算术（M1）**：seam 顶点**只算一次**，同一个点穿进两个 edge use。旧实现把
`(o + x·R) + x·r`（tube 侧）与 `o + x·(R+r)`（赤道侧）分别求值——record 顶点按**精确位**去重——
两者只在 frame 对齐时逐位相等（x 对齐帧 0 % 不一致，一般单位帧约 50 % 差一个 rounding step），
所以只测原点/轴对齐的用例一直是绿的。现在由 `a_torus_on_a_generic_frame_keeps_one_seam_vertex`
（手写一般单位帧）与 `a_rotated_torus_away_from_the_origin_keeps_one_seam_vertex`（非原点 + 旋转，
经公共 transform 烘焙）钉住。

**`CHAMFEREDGE` 与曲面体（M3）**：曲面体不再在 lift 处被拒，倒角会走到**选边**。写入方为闭合曲面
重建的**合成 seam**（一条边的两个 use 同属**一个**面）现在被 `nearest_edge` **跳过**，所以在圆柱
rim 上点击得到如实的 **`solid.edge_not_found`**（诊断点名被跳过的 seam），**不是**
`solid.chamfer_failed`，body 不变。两个**不同**面共享的光滑 seam 仍会答 `solid.chamfer_failed`
（“no blend (Smooth)”），但本构建没有任何可构造的 body 会产生这种边，因此该路径只在**谓词级**
（`is_seam` 的单元测试）被钉住，没有真实 body 证据。曲面**边**倒角仍是 P2.2。

**显示缺口与 P2.1c**：`BrepSurface::Nurbs` 没有任何网格器。中性离散器对该面按名拒绝，并在原因里
点名 **`solidcore-tess`**（`nurbs-face: no NURBS mesher in this build (solidcore-tess, phase
P2.1c)`），该面进 `missing_faces`，body 保持 `Partial`；被**剪裁**的圆柱/圆锥侧壁同样报缺面
（条带要求两个整圆）。**lift 成功、可布尔、可测量，但不等于显示完整**。`solidcore-tess` 接入是
P2.1c 的路径；在它落地前不允许用采样网格冒充实体的精确面。

**依赖与门禁**：`eval::form` 收内核的 `eval` 类型，所以本 crate 直接命名了**两个新依赖**——
`solidcore`（只用 `eval::form::{surface,curve}`）与 `solidcore-convert`（record↔kernel 的
`surface`/`record_surface`/`curve`/`record_curve`）。两者与既有的 `solidcore-ops`/`solidcore-record`
同一 pin（`55ae49ef`），同一仓库，**无 `[patch.crates-io]`、无版本变化**；
`scripts/check-architecture.py` 的允许表已包含这四个名字，且只有 `cad-kernel-solidcore` 可以命名它们。

**证据**（Linux x86-64、真实固定内核；不是 GPU/像素/宿主/其它载体）：圆柱 r=1 h=2 → `V=2π`/`A=6π`；
球 `4π/3`；圆环 R=2 r=0.5 → `V=π²`/`A=4π²Rr`；截头圆锥 r1=1 r2=0.5 h=2 →
`πh(r1²+r1r2+r2²)/3`；**完整圆锥** r=1 h=2 → `V=πr²h/3`、`A=πr²+πr√(h²+r²)`、形心 `h/4`；圆柱
∪/∖/∩ 盒 → `8+π/2`/`π/2`/`3π/2`；被剪裁半圆柱 → `V=π`/`A=3π+4`/形心 y=`4/(3π)`；二次 lift 稳定。
**断言一律是内核请求的 `1e-6` 相对精度**（个别绝对量用 `1e-6` 绝对），实测误差远小于它、具体数值只
记在测试注释里，**不钉成界**。测试在 `cad-kernel-solidcore/tests/curved.rs`（含
`a_full_cone_measures_its_apex_band`、两个 torus seam 用例、
`a_trimmed_face_cut_off_the_quarter_grid_is_refused_by_name`）与 `tests/arcs.rs`（真实内核），
NURBS 载体/命名/pcurve 的单元测试在 `record/nurbs_tests.rs`、`record/to_record_tests.rs`
（`pcurves_are_exact_at_sixteen_interior_samples`）、`brep/tests.rs`、`solid/tests.rs` 与
`ops/chamfer.rs`（`a_seam_is_skipped_and_a_shared_edge_is_not`）。

## 11. P2.1c-1 决策：NURBS 面显示走 `SolidTessellator` 接缝的 `solidcore-tess`

**背景**：P2.1b 让 NURBS 面**可承载/可 lift/可布尔/可测量**，但中性离散器对
`BrepSurface::Nurbs` 一直按名拒绝（`nurbs-face: no NURBS mesher in this build (solidcore-tess,
phase P2.1c)`），该面进 `missing_faces`、body 停在 `Partial`。

**决定**：显示网格器**进内核 crate**、走**既有** `SolidTessellator` 接缝，而不是在
`cad-kernel-adapter` 里写第二套 NURBS 评估器。

- `crates/cad-kernel-solidcore/src/tess.rs`：`SolidCoreTessellator`（ZST，`type_key =
  yacr.kernel.solidcore.tess`）。`lift()` 中性 B-Rep → `solidcore_tess::plan(&record, tolerance,
  size)` → `TessPlan::evaluate()` → 映射成接缝的 `TessellationMesh`。
- 依赖：`solidcore-tess` 与既有四个 solidcore crate **同一 git pin**（`55ae49ef`），上游
  `publish = false`，**无 `[patch.crates-io]`、无版本变化**；`Cargo.lock` 只新增
  `solidcore-tess`/`spade`/`robust`（rayon 只在上游的 non-wasm target 上）。只有
  `cad-kernel-solidcore` 可以命名它（`check-architecture.py` 早就保留了该名字）。
- 接缝：`SolidTessellator` 增加 `Send + Sync` 超 trait（与 `SolidModeler` 一致；布尔 worker 需要
  把 `Arc<dyn SolidTessellator>` 搬进线程）；`cad-kernel-adapter` 新增
  `tessellate_exchange_with(exchange, tolerance, budget, Option<&dyn SolidTessellator>,
  cancelled)`，既有 `tessellate_exchange` 行为逐字不变（以 `None` 委派）。
- **委派规则（刻意保守）**：只有 `SolidExchange::Brep` 且**至少一个** `BrepSurface::Nurbs` 面时
  才问接缝。解析体（plane/cylinder/cone/sphere/torus）继续走中性离散器，**显示不 churn**。
- 请求里的 `GeometryHandle::Resolved(ObjectId(0))` 与 `TaskStamp::new(DocumentId(0), 0)` 是
  **合成身份**（导入期/命令期调用没有对象 id；stamp 只是接缝的结果标签，调用方丢弃），代码注释
  明写“不得当作实体身份”。
- 失败语义：接缝给出可用网格（`Success`/`Partial` 且非空）就原样返回；否则跑中性路径，并把接缝
  的拒绝**保留可见**（诊断前置，或把 detail 折进 `UnsupportedReason`）。中性 `Partial` 仍是
  `Partial`。取消是调用**边界**检查（`plan` 不接取消令牌），不声称 mid-plan 取消。

**容差**：请求的 `linear_deflection`/`angular_deflection` 映射为
`MeshTolerance { chord, angle: Some(..) }`（**绝不用上游自己的 display 默认**）。上游内部有
`1e-6 × size` 的弦高下限与 1° 的角度下限，所以 `Precision::Approximate { error_bound }` 报告
**实际使用的弦高** `max(linear_deflection, 1e-6 × size)`；这里**永不**发 `Precision::Analytic`。
请求细于下限时产出在下限、并用**新稳定码** `kernel.tolerance_not_met` 说明。

**诚实降级映射**：`PlanStats.unmeshed` 与 `evaluate()` 的 `RefusedFace` → `missing_faces`
（按面去重，理由原文）；`lenient`（按裁剪放行的面）、`over_tolerance` 的 `within == false`、
`edges_capped` 的 `within == false`、`unsampled_edges` → `kernel.tolerance_not_met` 诊断；
空网格 → `Failed(kernel.empty_geometry)`（没有“成功但空网格”）；超预算 →
`Failed(kernel.budget_exceeded)`。**不做**位置式开边扫描：plan 对每条边只采样一次并把同一批点
交给相邻面，再做一套位置权威只会与它冲突。

**拒绝码**：lift 拒绝原样透传（`solid.lift_unsupported` 等，**不改写**内核的话）；原始 SAT/SAB
→ `kernel.no_acis_kernel`（网格器不解析字节）；plan 无法规划 → `kernel.unsupported_surface`；
捕获到的上游 panic → `Failed(kernel.failed)`，消息**不含** panic 文本。

**注入与可达性**：`Application::set_solid_tessellator`（与 `set_solid_modeler` 同构），
Linux `app-linux` 与 `cad-cli-tools` 契约测试注入 `SolidCoreTessellator`，布尔 worker 一并携带；
`cad-import-acadrust` **有意不接**——ACIS 读取器今天对 spline 曲面**按名拒绝**，没有任何导入路径
能产出 `BrepSurface::Nurbs`，所以不存在可达缺口；等曲面导入落地时按同一条接缝接上即可（写在
`bootstrap.rs` 的注释里）。

**证据**（Linux x86-64、真实固定内核）：手写**双线性 NURBS 平面**顶面的闭合盒 → lift 成功 →
网格 >0 三角形、`face_sources` 落在面范围内、边线非空、`Approximate` 且 `error_bound ≤` 请求；
弦高减半**不减少**三角形数（容差单调）；双壳 body → `Unsupported(solid.lift_unsupported)` 且
**无网格**；`max_faces = 1` → `Failed(kernel.budget_exceeded)`；取消；registration。
适配器用 stub 证明：NURBS body 委派、`None` 仍是具名缺面的中性 `Partial`、解析体**从不**调用
stub、接缝拒绝可见、原始 SAT 从不委派。**没有 GPU/像素/宿主证据**；`app-linux` 的注入只是
**编译**证据。NURBS 面今天只可能来自内核产物（例如未来的圆角 blend 面），导入侧仍不可达。

## 12. P3 版本选择器决策：`acisVersion` × `dxfVersion`

**决定**：把此前隐含的两个常量变成**显式、可拒绝**的选择，默认与 P3a 完全一致：

| 选择 | 取值 | 默认 |
|---|---|---|
| `acisVersion`（ACIS 载荷头版本） | `"7.0"`（700）/ `"21.0"`（ASM 21800） | `"7.0"` |
| `dxfVersion`（DXF 容器 `$ACADVER`） | `"2010"`（AC1024）/ `"2013"`（AC1027）/ `"2018"`（AC1032） | `"2010"` |

`AcisVersion::V21_0.sat_version()` 必须用 `SatVersion::from_sat_number(21800)`：上游常量
`SatVersion::V21_0` 是 `(21,0,0)` → 数字 **2100**，低于 `SabWriter` 的 `>= 21800` ASM 阈值；
21800 的字段是 `(218,0,0)`，产品串由上游拼成 `ACIS 218.0`。

**矩阵（在写任何记录/字节之前拒绝，绝不静默降级）**：

| 组合 | 结果 |
|---|---|
| DXF R2010 + 700 | 接受：AC1024，内联 ACIS 7.0 SAT 文本（历史路径） |
| DXF R2010 + 21800 | 拒绝 `export.acis_asm_requires_r2013` |
| DXF R2013/R2018 + 700 / 21800 | 接受：SAB 头 700/21800 写进 `ACDSDATA`（`export.note.acis_sab_dxf`） |
| DWG + 700 / 21800 | 接受：AC1032，SAB 头 700/21800 写进 AcDs |
| DWG + 非 R2018 | 拒绝 `export.dwg_version_fixed`（DWG 版本由文档决定，恒 AC1032） |

**编码路径**：R2010 走 `Solid3D::from_sat`（内联文本）；R2013/R2018 与 DWG 走
`Solid3D::from_sab(SabWriter::write(sat))`，让 writer 把**我们持有的字节**原样入段（否则 DXF
的 `queue_sab_data` 会二次解析 SAT 文本，失败时静默留下空段）。**子集不变**：版本只改载荷编码，
不扩大可写体（仍是单壳、全平面、直线边界、真相似放置；六个 `acis.*_not_written` 码不变）。

**设置**：`acisVersion`/`dxfVersion` 进 `cad-host` 的闭合键表（`ALL` 24 → 26），非法 token 是
`settings.invalid_value` 且**不写入**；持久化在 `ui.export.*` 叶子；Save As 安装到
`HostController::export_options`（**不是**数据库状态：无 revision、无历史）。

**自读回证据**：每个允许组合都由本仓库自写→自读：SAB 头数字 == 选择（700/21800），盒
`(1 shell, 6 faces, 12 edges)`、楔 `(1, 5, 9)`，`assert_brep_equivalent` 通过；SAB flags
700 → `0`、21800 → `12`；DXF R2013/R2018 输出确有 `ACDSDATA`/`ACDSRECORD`，310 块拼回的 blob
以 `ACIS BinaryFile` 开头且版本字段为所选值，`DxfReader` 能重新附着（`is_binary`）；R2010 在
DXF 解密后含 `End-of-ACIS-data` 且无 `ACDSDATA`；DWG 仍是 AC1032 + AcDs。

**未验证（不声明）**：ASM 21800 与**所有** AC1027+ 容器只被本仓库/acadrust 自己的读取器读过，
**没有任何第三方 CAD 打开过这些字节**；`acis_revision` 块仍按 `AcisRevision::default()` **全零**
写出（没有可保留的源 revision，也不臆造 major/minor）——这是与 ASM 头并列的 vendor 接受性风险。
**行为变化**：DXF 容器默认由此前“无实体时 AC1032”变为 **AC1024**；含 3DSOLID 时与以前相同。

## 13. P2.1c-2 实测：圆角产物是「零斜率 cone + 有理网」，按 cylinder 读

**问题**：圆角（`FILLETEDGE`）一直被「产物是否为有理 NURBS、能不能回写」挡住。本轮不再猜，直接在
pin `55ae49e` 上实测 `solidcore_ops::blend_ordered`。

**实测（真实内核，单位立方体，一条全长直边，`Blend::Round`，r=0.1，`Corners::Filled`，
默认 `BlendOptions`）**：

- 成功：6 → **7 面**、12 → **15 边**，`Blended.made == [6]`；内核能重新读回自己的 record。
- 圆角面的 label 是 **`Surface::Cone { half_angle: 0.0 }`**（frame 原点 `(r, 1−r, −0.5)`、轴 `+z`、
  `radius_at_origin = r`），**不是** `Surface::Blend`；几何挂在 `Face.nurbs = Some(RationalSpline)`：
  `u_knots = [0,0,0,1,1,1]`（次数 2）、`v_knots = [0.25,0.25,0.75,0.75]`（次数 1）、控制网 **3×2**、
  权重 **`[1, 1, √2/2, √2/2, 1, 1]`**（`√2/2` 在中排）——**确实是有理** NURBS。
- 圆角面的环是 `[line, circle, line, circle]`（两条相切线 + 两段端部弧），两个端面各从 4 个 use
  变成 5 个；`domain = [[0,1],[0.25,0.75]]`。
- `Blend::Chord` 形状相同；`Blend::Curvature` 不同：label 直接是 `RationalSpline`（`Face.nurbs =
  None`，4×8、次数 (3,7)），**边界 use 是样条曲线**。
- **没有** `Surface::Blend`：`record/surface.rs` 与 `from_record.rs` 的 “blend” 具名拒绝**不是**
  圆角的阻塞点（它只在别的 pin/别的 blend 种类下才可能出现）。

**决定**：读者把**恰好** `half_angle == 0.0` 的 record cone 映射成
`BrepSurface::Cylinder { origin, axis: z, ref_dir: x, radius: radius_at_origin }`（半径按圆柱的
`finite && > 0` 校验）。**精确比较是刻意的**：`1e-9` 的斜率仍是 cone（单测钉住 `1e-9`/`1e-3`/`0.5`
保持 `Cone` 且 `sin`/`cos` 逐位）。`from_record` 里 “no slope is a cylinder” 的拒绝保留为**防御性
守卫**，对经 `to_brep` 读出的 record 已不可达。

**为什么不把网当 NURBS 读**：把 `Face.nurbs` 直接当 `BrepSurface::Nurbs` 会让 lift 失败
`solid.pcurve_not_exact`，原文 `no exact pcurve for this face's boundary in its surface: … not
supported yet: no exact pcurve of a circle on a spline surface`（内核没有“圆在样条曲面上的精确
pcurve”的闭式解；我们的写入方给的是 `given: None`，没有第二条来源）。所以**label 是唯一的精确
载体**，这也是“零斜率 cone → cylinder”必须发生在读取端的原因。

**证据（真实内核）**：圆角体质量属性 `Success`，体积 `0.997853981730947` vs 闭式
`1 − (1 − π/4)r² = 0.997853981633974`（相对 `9.7e-11`；r=0.25/0.49 同样成立，见
`tests/fillet.rs`）；对它做 `Boolean` 减法 → `Success`、**9 面**，圆角面回到中性模型是**一个**
`BrepSurface::Cylinder`（r=0.1、轴 +z、环 `[line, arc, line, arc]`），体积 `0.837853981730947`
vs `0.837853981633974`。读取映射本身在 `record/surface.rs` 的单测里钉住（`pub(crate)`，集成测试
调不到）。

**仍未做（不假装）**：

- **没有圆角操作**：`SolidOperation`/`SolidOperationFamily` 没有 `Fillet`，
  `ModelerCapabilities::fillet` 仍 `false`，没有 `FILLETEDGE`。`solidcore_ops::blend*` 从接缝仍
  不可达——用户还不能创建圆角。本轮只让**内核产出的**圆角能映射、lift、测量、布尔。
- 两条圆角相接的**角部**（ball patch / mitre patch）未实测；`Blend::Curvature` 仍然不能承载
  （它需要 NURBS **曲线**族，中性模型没有）。
- 被裁剪的圆角侧面**显示**仍按 P2.1b 规则是部分（圆柱条带要求两个整圆）；没有像素证据。
- 圆角体上再倒角会撞上 `from_record` 对 “spline 边曲线” 的具名拒绝（实测副产物，未修，记在这里
  供下一步参考）。
