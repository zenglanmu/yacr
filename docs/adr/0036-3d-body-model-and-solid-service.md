# ADR 0036：3D 实体模型（wireframe / surface / solid）与 solidcore 无状态建模服务

- 状态：**Accepted（2026-11-16）**。实现进度：P0 已落地；P0.1（存储边线直送
  `RenderBatch.edges`）已完成，仅有 Linux 上的契约/单元证据（尚无 GPU/像素证据）；
  P1–P3 **NOT IMPLEMENTED**。
  验证边界：本轮只有 Linux 上的契约/单元证据；Windows/macOS/Android/Web 的
  solidcore 运行状态、任何 GPU/像素结论一律 **NOT VERIFIED**。
- 日期：2026-11-16。
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
