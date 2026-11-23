# ADR 0050：显示边的端点/中点吸附与测量（窄声明）

- 状态：**Accepted（2026-11-23）**。本 ADR 是**决策记录**：它取代
  [ADR-0049](0049-real-imported-edges-as-body-display.md) §2「测量仍是 `Unsupported`」与其
  「不做任何 snap 声明」的**实现边界**，但**不改变** ADR-0049 的载体决策、反伪造不变量与
  「存储边只来自真实 `MeshEdge`」判据。ADR-0049 原文保留，只加 superseded 注记。
- 日期：2026-11-23。
- 相关：[ADR-0049](0049-real-imported-edges-as-body-display.md)（真实边进 `Body3d` 载体）、
  [ADR-0045](0045-mesh-wireframe-is-surface-boundary.md)（显示网格是剖分、不是语义表面）、
  [ADR-0033](0033-multi-format-import.md)（SKP/RVT 预览能力上限）、
  [ADR-0048](0048-format-connector-crates.md)（连接器归属）、
  `docs/compatibility.md`、`docs/3d-solidcore-and-mesh-import.md`。
- 影响：`cad-connector-core`（`MeshImport::declare_edge_snap_measure` 与按实际插入 type key
  产出的能力行）、`cad-connector-skp`（`import.skp.measure_edges_partial` 诊断）、
  `cad-measure`（`SnapTarget::body_edge_snaps`、`SnapCandidate.geometry_source`）、
  `cad-app`（宿主按 `type_key` 的吸附门控、测量记录的 source/precision 汇总）。
  **不改变**：`cad-db` 唯一权威、`.ymodel` schema 与 `FORMAT_MINOR`、`cad-domain` body 模型、
  网格/其它格式的 `measure` 能力、RVT 路径、补丁台账、载体所有权。

---

## 0. 背景

ADR-0049 把 SketchUp 的真实 `MeshEdge` 记录搬进 `Body3d` / `BodyDisplay.edges`，并明确
**不**打开测量/吸附，理由是「有了边不等于可以量长度，也不等于端点可吸附」。那条边界的直接
后果是：用户可以看见并拾取一条真实边，却不能用它量距离。

当时不打开的原因不是缺少数据，而是缺少**一份可检查的窄契约**：显示边是真实记录（端点、
折线段），但它**没有**面/平面/三角语义，也没有弧/圆身份（`curve` 已被弦化成两点）。如果只
按「body 有边」就打开 `MEASURE`、`DIST` 或全套 OSNAP，就会声称源文件从未提供的语义——这正是
ADR-0045 与 ADR-0049 反伪造不变量的对象。

本 ADR 决定：只打开**端点与中点**两种吸附，且只为**确实携带真实存储边的 SKP 载体**打开；
测量记录如实标注它的来源与精度上限。

## 1. 决策

### D1 — 声明边界：只有端点与中点，只在存储边上

一条 SKP 显示边（`BodyDisplay.edges` 的 polyline 段，或 `SkpPreviewEdges` wire 体的段）贡献：

- 每个段端点的 **Endpoint** 候选；
- 每个段中点的 **Midpoint** 候选（相邻内部顶点按坐标键去重，一个顶点一个候选）。

**不产生**：面吸附、三角面吸附、中心、象限、垂足、交点；**不支持**面积/体积。`MEASURE`
的既有算法（距离、折线长、角度、多边形面积）在语义上照旧可用，但**几何来源只有这些边上的
点**——不因为边存在就声称面积/体积来自精确曲面，也不把三角剖分当成面语义。

与网格的关系保持不变：普通 `Mesh` 与没有存储边的 `Body3d`（例如 ACIS body、三角形-only 的
`SkpPreviewMesh`）**不贡献任何吸附点**。显示缓存仍是剖分。

### D2 — 宿主按 `type_key` 门控，不按「是不是 body」

吸附目标由共享层 `cad-app` 生成（`drawing_pick_items` → `snap_targets_for` →
`body_aware_target`）。一个 `Body3d` 只有在**同时**满足以下两条时才是边吸附源：

1. `body.display.edges` 非空；
2. 该实体的 `type_key` 是 SKP 的显示边载体（`SkpPreviewMesh` 或 `SkpPreviewEdges`），且
   构建里编入了 `formats-skp`（否则谓词恒 false，不靠手抄字符串匹配）。

门控放在**共享层**而不是某个宿主：Linux/Windows/macOS/Android 的 `native-below` 会话与
X11/Web 的 wasm 会话都走同一份 `cad-app` 契约（铁律 9）。ACIS body 与任何其它 `Body3d` 保持
不可吸附——它们的显示边是剖分，不是存储记录。

非 body 目标的 `GeometrySource` 不再被折叠成 `Analytic`：`Mesh` 拾取报 `DirectMesh`，厂商
代理的解析替身报 `ProxyCache`，解析几何报 `Analytic`；body 自身有精确 B-rep 报
`KernelMesh`，否则 `DirectMesh`，精度取 `BodyDisplay.precision`（不硬编码 `Analytic`）。

### D3 — 能力行：只给真正携带边的 type key `Partial`

`MeshImport::declare_edge_snap_measure(true)` 是**导入器的显式选择**（默认关闭，其它导入器完全
不变）。打开后，`MeshImport::finish` 仍按**实际插入的每个 `type_key`** 各出一行，并且只为
**插入过非空 `BodyDisplay.edges` 的键**报 `measure = Partial`：

| 载体 | `measure` |
|---|---|
| `SkpPreviewMesh` 且确实存了边 | `Partial`（仅端点/中点吸附） |
| `SkpPreviewMesh` 三角-only（没有存储边记录） | `Unsupported` |
| `SkpPreviewEdges` wire 体 | `Partial`（仅端点/中点吸附） |
| 其它所有导入器 / ACIS / RVT | 不变（`Unsupported`） |

`Partial` 的含义是**吸附/测量可用但有明确语义缺口**：没有面/三角/中心/象限/垂足/交点，也没
有面积/体积；不是「已验收的测量能力」。

**release 天花板只收窄、不强制**：`apply_ceiling` 用同一个 `weaker()` 作用到 `measure`，所以
`Partial + Unverified → Unverified`（2021–2025 的容器等价读法）；`Unsupported` 永远不被抬成
可用，也永远不被改写成别的状态。

### D4 — 测量记录：最少权威来源 + 最弱精度

一次测量只有**一个** `source`/`precision` 对，因此必须写**汇总规则**，不能把某一采集点的来源
冒充整条记录：

- **没有任何吸附点**（全部是原始拾取）→ `UserPoints` / `Analytic`：值来自用户给的坐标，
  没有近似来源可归因。
- **至少一个吸附点** → 取这些点依赖的**最少权威**来源：任一点来自显示/直接网格 →
  `DirectMesh`；否则任一点来自厂商代理缓存 → `ProxyCache`；否则任一点来自精确内核网格 →
  `KernelMesh`；否则（全部解析）若还存在**原始拾取输入点** → `UserPoints`；只有「全部吸附且
  全部解析」才是 `Analytic`。
- **精度**取吸附点中最弱者（`Unknown` < `Approximate` < `Analytic`）；原始拾取不声称精度，
  既不加强也不削弱；两个 `Approximate` 取误差界较大者（`None` 视为最弱）。
- `snaps` 与 `points` 位置一一对应；长度不一致按 `measurement.snaps_not_parallel` 拒绝，
  绝不把一个采集点的来源挪给另一个点。

### D5 — 诊断：`import.skp.measure_edges_partial`

当一次 SKP 导入确实发出了边段（`stats.edges.emitted > 0`）时，恒发一条 informational 诊断
`import.skp.measure_edges_partial`，detail 明确写下「端点/中点吸附 only，无面/三角/中心/象限/
交点，无面积/体积」。没有边就不发；它不是拒绝码，也不改变 `Partial` 的既有理由。

## 2. 被否决的方案

- **对所有 `Body3d` 泛化开吸附**：ACIS body 的 `display.edges` 是剖分边界，通用打开会把
  三角化对角线当可吸附边、把 body 的显示缓存当文档拓扑。这需要「所有 body 的边都是真实
  记录」这个不成立的全局前提。
- **面/三角面吸附**：显示网格没有面身份或平面语义；给三角形做面吸附会把剖分结构暴露成 CAD
  面，且不同三角化密度会给出不同结果（ADR-0045 明确禁止）。
- **不做门控、只按导入器声明推断能力**：只把 `measure` 从 `Unsupported` 改成 `Partial` 而不检查
  「这个 `type_key` 实际有没有插入边」，会让三角-only 的 SKP 组也宣称可测量。能力行必须由
  **实际载体**产生。
- **把精度写成 `Analytic`**：SKP 边是源文件的显示记录，没有误差声明；`Precision::Unknown`
  是唯一诚实的答案。
- **用 `Partial` 冒充完整测量**：那会把「端点/中点 only」卖成 CAD OSNAP。窄声明必须连同
  「不支持什么」一起写进能力表、诊断与 UI 可达路径。

## 3. 后果与未改变

- **未改变**：`cad-db` 唯一权威、`.ymodel` schema / `FORMAT_MINOR`、body 模型、显示三条路径、
  拾取契约（仍是实体级，无子元素身份）、网格/STEP/IFC/RVT 的 `measure = Unsupported`、补丁
  台账、载体所有权与连接器编译裁剪。
- **已接受的取舍**：
  - 只有 SKP 显示边载体可测；其它格式即便有真实边记录也不在本 ADR 的声明内（需要各自的能力
    行与门控）。
  - 一条边没有 id、没有共享顶点身份，因此测量记录只到实体级；没有边级选择/编辑。
  - `Partial` 的语义细节（端点/中点 only）必须同时写在 ADR、`docs/compatibility.md` 与导入
    诊断里；漏一处就会变成过度承诺。

## 4. 证据与边界

- 本决策落在共享层（`cad-connector-core` / `cad-connector-skp` / `cad-measure` / `cad-app`），
  没有按 `surfaceMode` 分叉的第二份实现；SKP lane 只编进 `full-formats` 的载体，浏览器 slim
  bundle 不含 SKP（那边按 `unsupported-runtime: import.skp` 拒绝，也没有第二套吸附实现）。
- **证据只有契约/单元测试**（见 `docs/validation-edge-followups-2026-11-23.md`）：端点/中点候选、
  去重、来源汇总、门控、能力行与天花板。真实 SKP 样本上的吸附/测量行为、真实 GPU/像素、宿主
  窗口交互、其它载体运行均为 **NOT RUN / NOT VERIFIED**。
- 不得把契约测试写成「SKP 测量已验收」；也不得写成「任意 body 可吸附」。
