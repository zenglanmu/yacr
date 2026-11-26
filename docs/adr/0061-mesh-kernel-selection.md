# ADR-0061：网格内核选择（SketchUp 网格布尔 / 修复 / 编辑）

- 状态：**已接受（决策，2026-11-26）**；**P-MESH-0 已实现（2026-11-26）**，
  **P-MESH-1a/1b 已实现（2026-11-26，Linux 原生契约 + 真实内核命令路径）**，
  **P-MESH-2 面/边编辑与 P-MESH-3 平面/非平面投影/共面嵌套环补洞已实现（2026-11-26，`cad-mesh-edit` + 命令契约）**
- 关联：ADR-0036（3D 实体与 solidcore 接缝）、ADR-0033（多格式导入）、ADR-0048（格式连接器
  crate）、ADR-0049（SKP 存储边）、ADR-0053（应用域，SketchUp 对应 `concept`）、
  `docs/3d-solidcore-and-mesh-import.md` §5.4/§7

## 背景

`cad-connector-skp` 目前只做**预览导入**：`skpwright` 给出的三角形、实例世界变换、材质、
真实 `MeshEdge` 进入 `Mesh` / `Body3d{kind: Unknown}`，每个成功导入恒 `Partial`
（`import.skp.preview_geometry`）。STL/OBJ/PLY/OFF/glTF 同理。仓库里没有任何网格布尔、网格
修复或 mesh→BRep 的实现；`UNION`/`SUBTRACT`/`INTERSECT` 只对精确 `Body3d{Solid, Exact}` 的
操作数生效（solidcore / ADR-0036），对 `Mesh` 操作数仍是具名拒绝。

SketchUp 式需求（`concept` 域）要求对导入网格做：

1. **布尔**：并/差/交，输入经常是扫描件式的三角汤（非流形、自交、重复面、断开壳）；
2. **修复**：焊接重合顶点、统一朝向、重建自交/重复面/内部薄壁的实体；
3. **编辑**：按面/边拾取，移动、推拉、细分、平滑，并保持可撤销的文档写入。

iron rule 2/6 要求这些能力要么真实实现，要么显式拒绝：不允许把网格布尔结果标成
`Body3d{Solid, Exact}`，不允许让 kernel 的“成功”冒充工程精确性，也不允许把 open shell
自动补洞后当成功。

参考语义也支持“先闭合、再布尔”：SketchUp 官方的 Solid Tools 只对 **solid** 生效——solid 是
有限闭合体积、没有缺面、没有游离边/面；开放 groups/components 会被拒绝，补洞是独立的
Solid Inspector 工作流。因此 open shell 的布尔拒绝不是本项目的临时限制，而是与参考软件
一致的语义；补洞应作为独立修复步骤（P-MESH-3），不能混进布尔。

本 ADR 只做**内核选择与接缝/阶段定义**；P-MESH-0 之前不改任何 `Cargo.toml`、不加依赖、
不开放任何用户可见命令。**P-MESH-0 已按此边界落地**：接缝 + `cad-kernel-manifold` + 门禁 +
真实内核契约测试，仍无用户可见命令。

## 决策

### D1 采用 `manifold-rust`（纯 Rust 版 Manifold）作为网格几何内核

固定版本 **`manifold-rust = "=0.19.6"`**（Apache-2.0，MSRV 1.77，2026-11-22 发布），
不启用 `parallel` feature（默认顺序执行，结果确定；wasm 本来就是单线程）。选择依据：

- **纯 Rust、无 FFI**：不引入 C++ 工具链、不需要 `cc`/CMake/NDK 交叉编译，也不触碰
  `acadrust`/`rvt` 之外的补丁政策（iron rule 1）。`cargo check --target
  wasm32-unknown-unknown` 与 Linux 原生都通过（选择探测，见 §证据边界）。
- **两套布尔引擎**：C++ Manifold v3.5.4 的 exact 引擎移植，加上上游没有的 **robust** 引擎
  （精确有理数 + arrangement，Zhou et al. 2016）。`BooleanEngine::Auto` 在“两侧都是流形且
  无自交”时走 exact，否则走 robust；这正好覆盖 SKP/扫描件的三角汤。
- **修复原语**：`repair_orientation()`（只修绕序，保留三角形数/属性）、`rebuild_solid(rule)`
  （自交、T 形接点、重复/共面薄片、非流形边的重建）、`MeshGL::merge()`（重合顶点焊接）、
  `has_self_intersections()`（精确自交判定）、`simplify()`/`refine()`/`smooth()`。
- **取消与进度**：`CancelToken`（跨线程 `cancel()`）与 `ProgressReporter` 贯穿布尔管线；
  与仓库“布尔放 worker、接活取消探针”的既有要求（ADR-0036 C2）同形。
- **许可证**：Apache-2.0，可与本仓 AGPL-3.0-only 组合；不需要 `[patch.crates-io]`。

**不是**选择它的理由：它不是交互式拓扑编辑器（见 D2），也不提供工程精确 B-Rep。

### D2 内核只做无状态几何运算；SketchUp 式直接编辑属于共享拓扑层

`manifold-rust` 没有 half-edge 编辑 API，也不持有文档/选择/撤销。它提供的是内容进、内容出的
CSG/修复/构造器服务（cube/sphere/cylinder/extrude/revolve/level-set、hull、Minkowski、
split/trim、refine/simplify/smooth、2D cross-section）。因此：

- **布尔、修复、简化/细化、平面切割**走 `MeshModeler` 接缝（D3）；
- **按面/边拾取（边用几何派生 key）、移动、推拉、细分、平滑、撤销**不外包给内核，放在共享层
  `cad-mesh-edit`（**已实现 2026-11-26**，纯 `cad-domain` 数据，不依赖任何内核 crate）：
  拓扑普查（边界边、非流形边、连通块）、`face_sources` 面区域边界提取、移动/推拉/细分/
  平滑、边界环补洞（P-MESH-3，含共面嵌套环的环带补片），以及提交前的闭合校验。
- 需要 CSG 的编辑（推拉一个面区域后与自身求并/差）由命令层调用 `MeshModeler`；编辑层不
  链接内核。进度/取消/worker 与 ADR-0036 C2 相同：命令线程只校验与快照，kernel 在 worker，
  结果按文档/操作数快照陈旧检查后以一条事务写回。
- 不新增“网格编辑内核”依赖。`tri-mesh`/`plexus` 一类 half-edge 库只解决连通性，不提供本产品
  需要的编辑语义，仍要写同样的操作层，故不引入。

### D3 接缝：`cad-kernel-adapter::mesh` + 唯一实现 `cad-kernel-manifold`

完全镜像 ADR-0036 的 `SolidModeler` 形状，**不**复用 `SolidModeler`（精确 B-Rep 与三角网格的
语义、结果标记、失败模式都不同）：

```rust
pub trait MeshModeler: Send + Sync {
    fn registration(&self) -> Registration;          // yacr.kernel.manifold-rust + 0.19.6
    fn capabilities(&self) -> MeshModelerCapabilities;
    fn execute(
        &self,
        request: &MeshRequest<'_>,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> CadResult<MeshOutcome>;
}

pub enum MeshOperation<'a> {
    Boolean { op: MeshBooleanOp, left: &'a MeshData, right: &'a MeshData },
    Repair  { mesh: &'a MeshData, rule: MeshWindingRule },
}

pub struct MeshData { pub vertices: Vec<Point3>, pub triangles: Vec<[u32; 3]> }

pub enum MeshTopology { OpenShell, ClosedManifold, NonManifold }

pub enum MeshOutcome {
    Success   { mesh: MeshData, topology: MeshTopology, diagnostics: Vec<Diagnostic> },
    Partial   { mesh: MeshData, topology: MeshTopology, repair: MeshRepair, diagnostics: Vec<Diagnostic> },
    Unsupported { reason: String },
    Failed    { diagnostics: Vec<Diagnostic> },
}
```

- `cad-kernel-adapter` 保持“无内核”默认：没有 `MeshModeler` 时宿主报
  `unsupported-runtime: mesh-modeler`，不得静默用 `Mesh` 近似或返回空成功。
- `cad-kernel-manifold` 是**唯一**允许命名 `manifold_rust` 的 crate；`scripts/check-architecture.py`
  增加与 `solidcore` 同款的 require/forbid 规则（含 build/dev/target 表），并禁止其它
  `cad-*` 运行时依赖 `cad-kernel-manifold`；只有载体 app 注入实现。
- 稳定诊断码（不是本地化文案）：`mesh.no_modeler`、`mesh.empty`、`mesh.invalid_mesh`、
  `mesh.not_closed`、`mesh.not_manifold`、`mesh.boolean_failed`、`mesh.repair_failed`、
  `mesh.cancelled`、`mesh.worker_panicked`。

### D4 诚实标记：网格永远不是精确实体

- 网格布尔/修复的产物仍是 `SemanticGeometry::Mesh`（或 `Body3d{kind: Unknown, exact:
  Unavailable}`），**不得**写 `Body3d{Solid, Exact}`，**不得**让 `MASSPROP` 报“精确体积”。
- 若 P-MESH-1 之后暴露体积/面积，只能按 `Precision::Approximate { error_bound: None }` 或
  具名“网格估计”报告；当前 `cad-connector-*` 的 `measure = Unsupported` 不因本决策改变。
- **不做 mesh→BRep**（ADR-0036 §5.4 C 已拒绝）：从三角面反推平面/NURBS 会制造“看起来能编辑、
  实际尺寸错”的结果，违反 iron rule 2。需要精确几何应换输入格式（STEP）或从零建模。
- `.ymodel` **不新增 schema**：几何仍写现有 `cad:Mesh`；拓扑分类、修复报告是派生数据，
  打开时重算，不持久化（因此不升 `FORMAT_MINOR`）。面身份若需要编辑，复用已持久化的
  `Mesh.face_sources`（`SubElementId`，例如 `skp:face:<n>`，`topology_revision = Revision(0)`），
  不新增字段、不伪造多边形环。

### D5 修复边界：closed 是准入条件，补洞不是本期内核能力

`manifold-rust` 的 robust 导入要求输入**几何闭合且可定向**；不闭合返回 `Error::NotClosed`，
不会被自动补洞。P-MESH-0/1 的处理：

- **先做位置焊接的拓扑普查**：STL 每个三角形自带三个顶点、SKP 每个面复制顶点，所以索引直读
  会把一个几何闭合的立方体误判成 open shell。`inspect_topology_welded` 先按**位相同的坐标**
  合并顶点（报告 `welded_vertices`），再数边界边/非流形边；近重合由 robust 引擎自己处理。
  应用层预检与内核必须使用同一份焊接视图，不能只在内核里焊而让应用先拒绝。
- open shell：`mesh.not_closed` 显式拒绝布尔/重建，保留原网格不变；
- 非流形但闭合：robust 引擎接受为 triangle soup；输出标 `Partial` 并在 `MeshRepair` 里记录
  实际做了什么（焊接/重建/绕序翻转）；
- **补洞（boundary loop fill）不在本期**。若后续实现，必须单独设计并逐洞报告，不能把
  “补出来的面”混进用户原始数据而不留痕。

### D6 导入与编辑的前提：SKP 面身份必须显式携带

当前 SKP 导入把所有面三角化后**丢弃源面身份**（`face_sources` 为空），只有存储边保留
（ADR-0049）。没有面身份时：

- 三角形级编辑只能按三角形拾取，无法实现 SketchUp 的“推拉一个面”；
- 重新三角化/布尔后也无法稳定选择同一个面。

P-MESH-2 在 `cad-connector-skp` 为每个源面写 `Mesh.face_sources`（`skp:face:<index>`，
同一源面的所有三角形共享），并把面的可见性/材质与面身份对齐；不改变几何、不升
`FORMAT_MINOR`。在此之前，任何“SketchUp 面编辑”都按 `mesh.face_identity_unavailable`
显式拒绝，不拿三角形拼假面。

### D7 执行与取消

- 布尔/重建在 worker 线程执行，命令线程不阻塞；结果按 `document_id + generation + 操作数
  快照` 做陈旧检查，陈旧则丢弃并写 `mesh.boolean_stale`（复用 ADR-0036 C2 的状态模式）。
- `cancelled` 闭包在调用前/后检查；调用期间在**原生**目标用看门线程把闭包映射到
  `manifold_rust::cancel::CancelToken::cancel()`，内核返回 `Error::Cancelled` 时整体返回
  `CadError::Cancelled`，绝不写库。
- **wasm 没有线程**：`dom-canvas` 的 mesh 布尔在 P-MESH-1 按 `unsupported-runtime:
  mesh-modeler` 显式拒绝，除非后续用 Web Worker 提供同样可取消的执行路径；不得在主线程
  静默阻塞。`native-below`（Linux/Windows/macOS/Android）按载体注入。

### D8 用户可见命令不在本 ADR 开放

P-MESH-0 只落接缝与内核 crate（无命令）。P-MESH-1/2/3 决定命令拼写与域摆放：
**已实现（2026-11-26）**——布尔复用 `UNION`/`SUBTRACT`/`INTERSECT`，修复用 `MESHREPAIR`/
`MESHREBUILD`，网格编辑用 `MOVE`（面选择）/`PRESSPULL`（面选择）/`MESHREFINE`/
`MESHSMOOTH`，补洞用 `MESHCAP`。边身份使用几何派生 key（`mesh:edge:<bits>`），没有 schema
字段；端点移动后 key 改变。wire-only 边和跨多个三角形边的 stored polyline 仍不可编辑，不得
放可点假按钮（ADR-0053 D8）。

## 候选比较

| 候选 | 事实（2026-11-26） | 结论 |
|---|---|---|
| **`manifold-rust` 0.19.6** | 纯 Rust Manifold 移植 + robust 引擎；Apache-2.0；MSRV 1.77；exact/robust/Auto；`CancelToken`/进度；`repair_orientation`/`rebuild_solid`/`merge`/`simplify`/`refine`；WASM 官方 demo；robust 要求 closed | **选用**（D1） |
| `manifold-csg` 0.4.1 | 同一 C++ manifold3d 内核的安全 Rust FFI；Apache-2.0；支持 native/WASM target；**没有** `manifold-rust` 的 robust 引擎 | 备选：仅当 Rust 移植出现可复现缺陷时换 FFI；它不能处理非流形/自交输入，且引入 C++ 构建链与跨目标风险，不作为首选 |
| `csgrs` | 纯 Rust BSP CSG（union/difference/intersection/xor） | 否：BSP 布尔没有 manifold/robust 保证，也没有自交/非流形修复；不满足“修复”要求 |
| `baby_shark` | 纯 Rust 重网格/简化/ARAP 变形 | 否：没有布尔；可作为将来的重网格工具，不是网格内核 |
| MeshLib | C++ 核心，官方 Rust/WASM 绑定；布尔 + 开放壳补洞 + 自交/退化修复很强（`fillHole`/`fixSelfIntersections`/voxel repair） | 否：**source-available，商业使用需付费许可**，与 AGPL 分发/开源承诺不兼容；且 C++ 核心、体积与交叉编译成本高。若将来只做内部非商业评估，也不得进入本仓依赖 |
| `truck` | 已在 ADR-0036 因重合几何布尔 panic、无圆角/抽壳被否 | 否 |
| CGAL / OpenMesh / libigl / OpenCASCADE | C++；CGAL/OCCT 为 GPL/LGPL 系，OpenMesh/libigl 为宽松许可；工具链/交叉编译/体积成本高 | 否：破坏纯 Rust + wasm/Android 的载体约束；AGPL 虽可与 GPL 组合，但不是选择它们的理由 |
| 自研 exact predicate 布尔 | 需要 arrangement/符号扰动/鲁棒谓词，工作量与验证面远大于本轮 | 否：不诚实地低估 |

## 阶段与验收

| 阶段 | 内容 | 验收（必须真实运行） |
|---|---|---|
| **P-MESH-0** | **已实现（2026-11-26）**：`cad-kernel-adapter::mesh` 接缝 + `cad-kernel-manifold` + 架构门禁 + `THIRD_PARTY_NOTICES` 登记；原生目标用看门线程把 `cancelled` 映射到内核 `CancelToken`（wasm 只有预/后检查，已声明）；**不接命令** | `cargo test -p cad-kernel-adapter -p cad-kernel-manifold --locked`（66 + 9 passed）；`cargo check -p cad-kernel-manifold --target wasm32-unknown-unknown --locked`；`python3 scripts/check-architecture.py`；无 `MeshModeler` 时 `unsupported-runtime: mesh-modeler`（宿主接线前） |
| **P-MESH-1a** | **已实现（2026-11-26）**：`MeshModeler` 注入 `native-below` 宿主；`concept` 域网格布尔命令（复用 `UNION`/`SUBTRACT`/`INTERSECT`，混合选择具名拒绝）；worker + 取消 + 陈旧检查；结果写新 `SemanticGeometry::Mesh`（`AcDbSubDMesh`，无 `face_sources`/UV/材质，类型/来源不冒充）；结果必须 `ClosedManifold`；mesh/solid 布尔互斥（共用命令面，应用层拒绝交错启动），文档替换/关闭同时中止两者；仍非精确 | `cad-app` 命令/host 契约 12 passed；`cad-cli-tools/tests/mesh_command.rs` 真实内核 5 passed（∪/∖/∩ 体积 12/4/4、`Revision(1)`、一条撤销、`Body3d{Unknown}` 操作数、闭合非流形汤重建 `Partial`）；mixed → `boolean.operand_class_mismatch`；open → `mesh.not_closed`；预取消/宿主取消不写库；陈旧结果丢弃；宿主无 mesh modeler → `unsupported-runtime: mesh-modeler` |
| **P-MESH-1b** | **已实现（2026-11-26）**：网格修复命令 `MESHREPAIR`（`Orientation`）/`MESHREBUILD`（winding `Rebuild`）+ `Partial` 报告；补洞仍属 P-MESH-3 | 真实内核命令路径：闭合/翻转网格修复后仍 `ClosedManifold` 且外向；open → `mesh.not_closed` 不写库；非流形闭合汤可能 `Partial` 或按 `mesh.failed`/`mesh.not_manifold` 结构化拒绝（在线样本 5 例）；一条事务/一步撤销；导出仍非精确 |
| **P-MESH-2** | **已实现（2026-11-26）**：SKP 面身份（`face_sources`）+ 面选择 + `MOVE`（面区域/边）/`PRESSPULL`（面区域，沿法向）/`MESHREFINE`（1→4 细分）/`MESHSMOOTH`（Laplacian，边界固定）；**边身份是几何派生 key**（`mesh:edge:<两端点的精确位模式>`，无 schema 变更），拾取落在三角形边附近时返回该 key，`MOVE` 平移其两个端点；wire-only/跨多个三角形边的 stored polyline 仍是整实体，不做假边。编辑层 `cad-mesh-edit` 不链接内核 | 同一 SKP 面所有三角形共享 `skp:face:<n>`；面/边编辑后 `cad-db` 校验通过、闭合/流形性保持；无面身份时 `mesh.face_identity_unavailable`；边 key 在端点移动后按设计改变；`cad-mesh-edit` 22 + `cad-app` 命令 9 + `cad-host` 捕获 1 passed |
| **P-MESH-3** | **已实现（2026-11-26）**：`MESHCAP` 对每个简单边界环做 ear-clip 补片 + 逐洞报告；平面环直接补，**非平面环投影到最佳拟合平面后补**并标 `non_planar`；多个独立边界环各自补；**共面嵌套环按奇偶深度成组**——外环以直接内环为孔做环带（annulus）补片，深度 2 的岛再独立补，环带不覆盖孔洞；自交/退化环、同向绕行的嵌套组、找不到可见桥的嵌套组按名拒绝并保持开口，输出仍是非精确网格 | 闭合 cube 去掉顶/底面 → 两个环都补、闭合、体积 8；非平面环 → 填充 + `non_planar=true`；方形管（外 2×2、内 1×1、高 2）上下两个环带 → 闭合、体积 6；带两个孔的平板 → 3 个环都补；带岛平板 → 外环环带 + 岛独立补；退化/自交 → `mesh.hole_degenerate`/`mesh.hole_self_intersecting` + `mesh.fill_failed`；真实 open 样本 r1.obj → `fill_refused=1`；`cad-mesh-edit` 22 passed |

## 证据边界

- 本 ADR 的库事实来自 `manifold-rust` 0.19.6 的 docs.rs/README/源码与 crates.io 元数据，
  不是本仓运行结果。
- **选择探测**（2026-11-26，仓库外临时 crate，未提交、不是仓库门禁）：
  `cargo run --release` 编译 `manifold-rust 0.19.6` 并执行 `cube(2) − sphere(1,64)`，得到
  `NoError`、`volume = 3.834863`（64 段球面近似对应的闭式差；精确球为 3.81121）；删除一个
  三角形后 `from_mesh_gl64_robust` 返回 `NotClosed`；`cargo check --target
  wasm32-unknown-unknown` 通过。该探测只证明可编译与上述行为，不是仓库测试、不是
  GPU/真机/性能结论。
- **真实 SKP 样本**（2026-11-26，本地 `house.skp`，2,237,796 bytes，SHA-256
  `9cd2689df7c1bf0d1ce0d496e4e83c9367abab6d8ff3e21dc9bb8162e982dd30`，样本不入库）：
  `skpwright` 导入 64 meshes / 1440 triangles；`face_sources` 1440 条、28 个 distinct keys；
  kernel 拓扑普查 21 closed / 41 open / 2 non-manifold（共 1379 boundary edges；2 个非流形也带
  boundary edge，按 `mesh.not_closed` 拒绝）；21 个闭合网格的 `Orientation` 修复全部 `Partial`，
  41 个开放网格全部 `mesh.not_closed`；前两个闭合网格的 kernel-level `Union` 返回非空 mesh。
  **这只代表这一份文件**，不是 SketchUp 语料/产品验收；P-MESH-1a/1b 的 app 命令接线由
  `docs/validation-mesh-2026-11-26.md` §3 的 `mesh_command`/`mesh_real_samples` 覆盖。
- **非 Linux 平台运行状态仍为 NOT VERIFIED**；P-MESH-0 的契约测试在 Linux 原生执行，
  Windows/macOS/Android 仅编译/类型检查，Web/`dom-canvas` 的 mesh 布尔仍拒绝。
- `manifold-rust` 是 0.19.x（pre-1.0）且 2026-07 才发布；固定精确版本、不启用 `parallel`，
  其上游测试通过/Thingi10K 语料结论是上游自述，不是本仓证据。升级必须重新跑 P-MESH-1
  的样本与显式记录差异。
- **性能风险**：robust 引擎在很大且重度自交的网格上可能很慢（上游自述 Thingi10K 中少数样本
  超过 120 s，按 `Cancelled` 处理）。P-MESH-1 必须给布尔设置可取消的预算/超时，超时按
  `mesh.cancelled` 拒绝而不是写一个半成品；exact 引擎只用于可证明安全的输入
  （`BooleanEngine::Auto`），不得为了速度把非流形输入强行送 exact。

## 后果

- 仓库第一次有可执行的网格布尔/修复/编辑/补洞路径，且与精确 B-Rep（solidcore）严格分离；
- 新增一个纯 Rust 内核依赖、一个内核 crate 与一个无内核编辑 crate（`cad-mesh-edit`）；wasm
  产物增加内核 crate 的代码量，受既有 20/25 MiB 预算约束（P-MESH-0 实测）；
- 面编辑依赖 P-MESH-2 的 `face_sources`；边编辑依赖几何派生 `mesh:edge:<bits>` key（端点移动后
  改变），wire-only 边/跨多个三角形边的 polyline 仍不可编辑，不得用三角形索引冒充 SketchUp 边；
- 补洞已覆盖平面环、非平面投影环、多个独立环和共面嵌套环的环带；自交/退化环、同向绕行或
  无可见桥的嵌套组仍拒绝，open shell 仍不能进入布尔/重建，补洞必须逐洞报告，不得当成
  “所有开放网格已修复”。
