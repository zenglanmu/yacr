# ADR 0049：真实导入边作为 body 显示数据（不伪造，也不再整体拒绝）

- 状态：**Accepted（2026-11-23）**。本 ADR 是**决策记录**：它取代 ADR-0045 D3 中「SKP 真实边继续按
  P-SKP-3 未实现」的**实现边界**，但**完整保留**其反伪造不变量；ADR-0033 D8 的「纯边文件拒绝」对 SKP
  退役。实现按阶段落地，状态与证据边界见 §5（**本文件只提交决策，不附运行证据**）。
- 日期：2026-11-23。
- 相关：[ADR-0045](0045-mesh-wireframe-is-surface-boundary.md)（表面边界叠加、三角剖分对角线不是边、
  D3 的不伪造判据）、[ADR-0033](0033-multi-format-import.md)（SKP/RVT 的 preview 上限与
  `import.skp.edge_only_geometry`）、[ADR-0036](0036-3d-body-model-and-solid-service.md)（`Body3d`
  与显示缓存）、[ADR-0037](0037-ymodel-schema-and-persistence.md)（`cad:Body3d` 持久化）、
  [ADR-0041](0041-save-export-split-and-format-provenance.md) /
  [ADR-0048](0048-format-connector-crates.md)（导出 L1 + 逐条报告、连接器归属）、
  `docs/3d-solidcore-and-mesh-import.md` §5.3（P-SKP-3）、`docs/compatibility.md`。
- 影响：`cad-connector-core`（`ImportLimits`、新增 `MeshImport::push_body*` 与逐 `type_key` 能力行）、
  `cad-connector-skp`（携带边、属性策略、诊断、限额、导出报告）、`cad-app`（select filter 的 `MESH`
  别名、显示 body 的拾取归类）。
  **不改变**：`cad-db` 唯一权威、`.ymodel` schema 与 `FORMAT_MINOR`、`cad-domain` 的 body 模型、
  `cad-representation`/`cad-spatial` 的既有契约、`measure` 能力、RVT 路径、补丁台账、载体所有权。
- 影响文件：`crates/cad-connector-core/src/lib.rs`、`crates/cad-connector-core/src/mesh_import.rs`
  （**新增** `push_body*`，并让能力行只按实际插入的 `type_key` 产出）、
  `crates/cad-connector-skp/src/{lib,convert,edges}.rs` 与 `src/export/*`、
  `crates/cad-app/src/picking/vendor.rs`、`docs/compatibility.md`、`README.md`。

---

## 0. 背景

ADR-0045 处理的是**显示层**的假线框：场景层曾用三角形拓扑合成一套线框，把剖分对角线画成"边"。
它给出两条互不冲突的结论：

1. 显示叠加线 = 表面**边界**（索引使用次数为奇数的三角形边），这是可证明的派生，不写数据库；
2. **D3：不新造边数据、不伪造 SKP 边**。`.skp` 的真实边当时仍按 `import.skp.preview_geometry` /
   P-SKP-3 **未实现**处理，显示层推出来的边界也**不得**冒充文档数据。

现在两件事同时变了：

- **真实边可得**。`skpwright` 0.6.0 暴露文档里的真实边记录：
  `MeshEdge { pid, v0, v1, soft, smooth, hidden, casts_shadows, receives_shadows,
  layer: u16, material: Option<u16>, curve: Option<u32> }`（`src/mesh.rs`）。`v0`/`v1` 索引 run 内
  顶点，run 顶点是**米**（`Mesh.vertices`）；`Node.world` 是每个 placement 的 row-major 4×4 世界矩阵，
  平移为米。适配器今天**只读面**：walk 时把边计入 `SkpStats.edges_not_imported`（`convert.rs`），
  然后整份丢弃；纯边文件被 `import.skp.edge_only_geometry` 整体拒绝（ADR-0033 D8；外部 corpus
  763 个文件里有 25 个因此失败）。
- **载体已存在**。域里本来就有 `Body3d` / `BodyDisplay { mesh, edges: Vec<Vec<Point3>> }`
  （`crates/cad-domain/src/body.rs`），`cad:Body3d.display.edges` 在 Phase 1 就**逐字段持久化**
  （`crates/cad-domain/src/ymodel/payload/mesh.rs`），因此**不需要升 `FORMAT_MINOR`**；显示侧
  `body_fragment`（`crates/cad-representation/src/provider/default/fragments.rs`）已经实现了三条路径：
  有三角形 + 有已存边 → `MeshWithStoredEdges`（场景用已存边，不推导）；有三角形无已存边 → 普通
  `Mesh`（ADR-0045 的表面边界叠加）；无三角形只有边 → `LineSegments`。拾取把 body 的显示边当
  polyline 求最近命中（`crates/cad-spatial/src/pick/hit.rs`）。
- **RVT 没有真实边**。当前 RVT 走 `rvt` 的内部 glTF，GLB 只有三角形 primitive：
  `vendor/rvt/src/ifc/gltf.rs` 没有 line/edge 输出（源码检视：`grep edges`/line primitive 无命中；
  不是运行证据）。RVT 的"边"只能是 ADR-0045 的派生叠线，不能变成文档数据。

所以缺的不是 schema、不是渲染能力，而是**一条映射决策**：把真实 `MeshEdge` 记录搬进
`BodyDisplay.edges`，并在不满足条件时**退回**现有网格路径。本 ADR 就是这条决策。

## 1. 决策

### D1 — 载体是 `Body3d` + `BodyDisplay.edges`；不新增 `Mesh.edges`

**不新增 `Mesh.edges` 字段，不升 `FORMAT_MINOR`**（当前 11，`cad:Body3d` 已是既有 schema）。一个
face group（placement × 有效材质，仍是今天"一个 group 一个实体"的粒度）按下面的判据二选一：

| 判据 | 载体 | `type_key` | 显示 |
|---|---|---|---|
| 组内**每个可见面**（`MeshFace.outer` 及每个 `holes` 环，含三角化失败但可见的面）沿环的**每一对相邻顶点**（含首尾闭合的一对，方向无关）都在该 run 的 `Mesh.edges` 里有一条真实 `MeshEdge` 记录 | `Body3d` | `SkpPreviewMesh` | `MeshWithStoredEdges` |
| 任一环节缺记录 | `SemanticGeometry::Mesh` | `SkpPreviewMesh` | 普通 `Mesh` + ADR-0045 表面边界叠加 |
| 没有被任何可见面环覆盖的可见边（游离边；以及整个面组没有产出三角形时的面环边） | `BodyKind::Wire` 的 `Body3d`（按边材质分组） | `SkpPreviewEdges` | `LineSegments` |

- 升级后的 body 字段是显式、保守的：`kind = BodyKind::Unknown`（网格 + 边不是 B-rep，拓扑类未知，
  不冒充 `Sheet`/`Solid`）、`exact = BodyExact::Unavailable { reason }`（`"the SketchUp source supplied
  display geometry only"`；wire 组用 `"the SketchUp source supplied edges only"`）、
  `display.precision = Precision::Unknown`（SKP 的三角形是其自身平面多边形的剖分，但适配器不持有源
  几何的误差声明——既不写 `Analytic`，也不编造 `error_bound`）、`display.tolerance = None`、
  `display.completeness = Completeness::Partial([...])`。导入报告的 `Completeness::Partial` +
  `import.skp.preview_geometry` 仍然恒发。
- **保留 `SkpPreviewMesh` 作为类型键**：`TYPE=MESH` 选择过滤按 `type_key` 判定，`cad-app` 的
  `select_filter::body_type_matches` 目前只给 `Body3d` 答 `BODY`/`SOLID`/`SURFACE`/`REGION`
  （`crates/cad-app/src/select_filter/types.rs`）。所以必须给该别名表补一条 `SkpPreviewMesh → MESH`
  （同时仍答中性 `BODY`）；否则 body 化的 SKP 组会从 `TYPE=MESH` 里静默消失。`SkpPreviewEdges` 只答
  中性 `BODY`：wire 体不是网格，不声称它答 `MESH` 或某个线实体类型。
- 面组判据对**面组**是 all-or-nothing：一个面组要么整体是 body（`BodyDisplay.edges` 存该组**可见面环**的记录），要么整体退回 mesh（`SemanticGeometry::Mesh` + ADR-0045 派生边界），不存在"一半 body 一半 mesh"。升级成 body 时，三角化失败但面环可见的面，其真实记录仍留在 body 上；只有当整个面组没有 body 载体（退回 mesh 或完全没有三角形）时，那些面环记录才改由按**边材质**分组的独立 `SkpPreviewEdges` wire body 携带。游离边（不属于任何可见面环）始终走 wire body。它们仍是源记录，只是换了载体；不因为面丢了就合成或隐藏。
- 判据覆盖**可见面**的面环：面环可见但三角化失败时，其真实记录仍随面组进入 body（不绘制面不等于
  边不存在）。只有被隐藏、不可见图层或按可见性裁剪掉的面不参与判据——它们没有可验证的可见环。
  这个选择保证"升级成 body 的组里，每条可见面的边都能追到一条源记录"。

### D2 — 反伪造不变量（继承 ADR-0045 D3，并收紧为可检查规则）

1. 存储的边**只能**来自真实 `MeshEdge` 记录。**禁止**从三角形边合成，**禁止**从"没有记录的面环"
   合成（即使两个面共享同一对顶点、即使几何上显然该有一条边）。
2. 每条 polyline 的每个点必须是 `Node.world · run.vertices[vi]` 的结果；不发明坐标、不做吸附/合并/
   细分。除 D3 的剔除与 `curve` 的两点弦化外，`MeshEdge` 记录与 polyline **一一对应**。
3. 显示用的 polyline **不是** SketchUp 边实体模型：没有边 id（`MeshEdge.pid` 不落盘）、没有共享顶点
   拓扑、没有边级选择/编辑/圆角/布尔入口。它是**显示与拾取**数据，仅此而已。
4. 共享边不建身份：同一条物理边在两个材质组里会各存一条 polyline（每组一条）。这是显示数据的代价，
   不是错误；不要为了去重而跨组焊接顶点。
5. 面环判据（D1）是第 1 条的检查形式。不满足就退回 `Mesh` + ADR-0045 的派生叠加——那是**显示**，
   不是文档数据，所以不违反本不变量。

### D3 — 属性策略：采纳可见性，忽略没有载体的语义

- **丢弃**：`hidden` 边；不可见图层（`Model::layer_of(slot).visible == false`）的边；`soft` 边
  （SketchUp 的软边是不可见的面间分界）。
- **保留但不建模**：`smooth` 边照常画线，但**不**做法向平滑——法向仍来自面的现有来源，不因边的标志
  改变。
- **逐边材质**：边界边（属于某个被绘制面）忽略 `MeshEdge.material`，组颜色优先（一个面只有一种填充
  色，边跟着组走）；**纯 wire 组**用边材质颜色（`Model::material_of` 的基色/不透明度，沿用现有材质
  规则；读不到就 `ByLayer`，不发明颜色）。
- **`curve`**：flatten 成 2 点弦（`v0` → `v1` 的世界坐标），不重建弧/样条，不采样中间点。
- **阴影标志**：`casts_shadows`/`receives_shadows` 忽略——渲染器没有逐边阴影通道。
- **诊断**：新增
  - `import.skp.edge_flags_not_modeled`（informational，一条摘要；detail 给 soft/hidden/图层/阴影各自
    的命中计数。`flag_records > 0` 或存在被丢弃的畸形记录——`zero_length_skipped > 0` /
    `index_skipped > 0`——才发；畸形计数在该摘要里点名，文档总量仍在 `import.skp.document`）；
  - `import.skp.edge_curves_flattened`（informational，flatten 的曲线边数 > 0 才发）；
  - 扩展既有 `import.skp.hidden_pruned` 的 detail 到边（边级的 hidden/不可见图层剔除计入同一口径）。
  - 组内**真的没携带**的边（该组退回 `Mesh` 时的真实游离边等）记进既有 `import.skp.document` 的
    informational 计数：该字段今天写作 "N edge(s) not importable"，语义收窄为"看到了但没携带"，
    并补一个"携带"计数；不新增码。
- **文档化回退**：一个组的**所有**边界边都被 hidden/soft/不可见图层剔除时，该组退回纯 `Mesh`（不产出
  "有网格但没有边"的 body），ADR-0045 的叠加线因此回来。这是有记录的回退，不是静默降级：按
  `import.skp.edges_hidden_fallback`（Partial，带回退组数）报告。wire 组把边全部剔除后不产出实体并计数；
  若整份文件因此没有任何可见几何，既有 `import.skp.no_geometry` 照常适用。

### D4 — 纯边 SKP 不再拒绝

- `import.skp.edge_only_geometry` **退役**：不再产生。纯边文件导入为 `Completeness::Partial` 的
  `Body3d::Wire`（D1 的第三行），信息码 `import.skp.edges_only`。
- 拒绝码常量可以删除，或保留但标注 retired；**任何路径都不得再产生它**，文档/日志不得把它当当前行为。
- 能力行沿用共享层：`MeshImport::finish` 按**实际插入的每个 `type_key`** 各出一行，`read/render/pick`
  受 release 上限约束、`measure` 恒 `Unsupported`。因此新增 `SkpPreviewEdges` 行是同一契约的产物，
  不是第二套记账。
- **corpus 计数必须重跑**：ADR-0033 / `fixtures/skp/SOURCE.md` 的
  `763 → 728 成功 / 25 edge_only_geometry / 10 no_geometry` 是**旧行为**下的一次有界扫描。重跑前这组
  数字一律视为 **stale / NOT RUN**，不得当成当前结果引用，更不得在没有重跑的情况下改写成新数字。

### D5 — 限额：文件级边段预算，拒绝而不截断

- 新增 `ImportLimits.max_mesh_edges: Option<usize>`：**一次导入整份文件**允许发出的边段总数（跨
  placement/组累计）。`None` = reader 自己的预算（与 `max_mesh_faces`/`max_mesh_face_indices` 的
  `None` 语义一致，默认值不悄悄改变接受边界）。
- 为什么是文件级：SKP 每个 placement × 材质组一个实体，per-entity 上限约束不了一个文件的总量；
  `max_entities` 也不约束单个实体内部的边数。边是唯一会随"很多组 × 很多小边"放大的量。
- SKP reader 默认 **2,000,000** 段。超限 → `InvalidInput(import.skp.limits_exceeded)`，detail 必须给
  真实计数与预算；**绝不截断**后宣称成功。
- 预算在发出前累计；达到上限即整体失败，不产生"部分写入"的数据库。

### D6 — RVT：没有真实边，不推导

- 当前 GLB 路径只有三角形（`vendor/rvt/src/ifc/gltf.rs` 无 line primitive），**没有**边记录可携带，
  因此 RVT 的文档数据里**没有**边。
- **不得**从三角形推导 RVT 边并落成文档数据（那是 ADR-0045 D3 禁止的伪造）。今天 RVT 网格显示上的
  轮廓线仍只是 ADR-0045 的**派生**表面边界叠加。
- `docs/compatibility.md` 与 `README.md` 必须写明这一点（**absent, not derived**：没有边；可见的边界
  线是派生显示，不是边数据）。**本 ADR 不改代码**；文案更新与 SKP 一起在实现阶段落地（见 §5）。

## 2. 跨载体契约：拾取、测量、导出（不新增能力）

- **拾取仍是实体级**。`cad-spatial` 对 `Body3d` 的显示边按 polyline 求最近命中，命中里
  `sub_element: None`（边没有子元素身份）；面命中仍走缓存的 `face_sources`，SKP 组没有该映射，因此
  报既有的 `mesh-face-source*` 理由而不是发明面 id。结论：显示边是**拾取目标**，不是可选择/可编辑的
  边子元素。这条契约在 `cad-spatial`（共享层），所有载体一份，不按宿主分支。
- **测量仍是 `Unsupported`**。`MeshImport::finish` 的能力行固定 `measure: Unsupported`
  （`cad-measure` 没有网格入口）；对象捕捉对 `Mesh`/`Body3d` 也不生成任何点
  （`crates/cad-measure/src/snap/generate.rs` 的分支注释已写明"Body edge snapping is a later concern"）。
  有了边**不**等于可以量长度、也不等于端点可吸附——本 ADR 不打开这条路径，也不做任何 snap 声明。
- **SKP 导出（写出侧，ADR-0041/0048）**：
  - mesh + stored edges 的 body：今天 `SkpExporter::collect` 只取 `body.display.mesh`，**静默丢掉**
    edges。改为逐实体报 `export.skp.stored_edges_not_written`（`Partial`：**stored 边集与分段**没有复制；
    写出器会为每个被写出的三角形重新生成边界 `MeshEdge`），继续写三角形。**自由边写出延后**：store 的
    边是世界坐标 polyline、没有 run 顶点身份，写出去要先决定"追加端点还是按容差吸附到已有顶点"，并需要
    读回测试；在它落地前不声称任何 SKP 边往返。
  - wire body（无 mesh、只有边）：今天落到 `export.skp.unsupported_geometry`；改为具名
    `export.skp.wire_body_not_written`，方向 `Unsupported`（该实体什么都没写出，记 dropped，不是
    `Partial`）。
  - 写侧**已有的**"每个面环一条 `MeshEdge`"不是伪造：`skpwright` 要求面环闭合，那些边就是**同一批
    被写出的三角形**的边界，属于容器要求；本 ADR 的反伪造不变量约束的是**导入后进入文档**的数据。

## 3. 被否决的方案

- **给 `Mesh` 加 `edges` 字段**：`FORMAT_MINOR` 要升到 12，并且要同时改序列化、变换、包围盒、校验、
  拾取、导出与全仓 **221** 处 `Mesh { .. }` 字面量（`grep` 计数，含测试）。更根本的是它会制造两个边
  载体（`Mesh.edges` 与 `Body3d.display.edges`）并逼着每个消费方回答"网格的边算不算几何"。body 载体
  已经存在、已持久化、已渲染、已可拾取，没有理由再开一条。
- **每个 mesh 配一个兄弟 Wire 实体**：一个 SKP 组会变成两行（网格 + 线），选择、图层、撤销、报告都
  要成对维护，并且破坏"一个 group 一个实体"的今天不变量；两个实体的颜色/可见性也会分叉。
- **从三角形推导 RVT（或任何格式）的边并入库**：直接违反 ADR-0045 D3；派生边界只允许留在显示层。
- **把 SKP 组声称成 `BodyKind::Sheet`/`Solid`**：过度声称拓扑。没有 B-rep 就没有闭合壳；`Solid` 在
  `cad-db` 里甚至要求非空 `BodyExact::Exact`，写进去会被 `validate_body` 拒绝。

## 4. 后果与未改变

- **未改变**：`.ymodel` schema 与 `FORMAT_MINOR`（11）、`cad-db` 唯一权威、`cad-domain` 的 body 模型、
  显示层的三条既有路径、拾取契约、measure 能力、RVT 路径、连接器归属（ADR-0048）、补丁台账；
  ADR-0045 的派生规则继续对**普通 `Mesh`** 生效（退回网格的组仍有表面边界叠加）。
- **已接受的取舍**：
  - 同一个 SKP 文件可能出现两种载体与两个类型键（D1 的表）；`TYPE=MESH` 依赖别名表里的一条映射。
  - 边作为 body 显示缓存的一部分被持久化，`.ymodel` 会变大；导入侧只有 D5 的文件级预算，保存侧没有
    额外上限（沿用既有归档能力）。
  - 退回 `Mesh` 的组只保留派生边界，真实游离边不入库（在 `import.skp.document` 计数里可见）。
  - 自由边 SKP 写出、边级拾取身份、边测量/吸附都不在本 ADR 的完成范围内。
  - **已知导出缺口（后续事项）**：`cad-connector-mesh`（STL/OBJ/PLY/OFF）、`cad-connector-gltf`、
    `cad-connector-ifc` 的写出路径只读 `body.display.mesh`，没有 edge 通道，也不报告
    `BodyDisplay.edges` 的丢失。把 SKP 导入模型导出到这些格式会只保留三角面；本 ADR 不声称这些格式
    的边往返，后续应补一条显式 conversion note。
- **跨载体**（铁律 9）：本决策全部落在共享层（`cad-connector-core`/`cad-connector-skp`/`cad-app`/
  `cad-domain`，含 `cad-representation`/`cad-spatial` 的既有契约），没有按 `surfaceMode` 分叉的第二份
  实现。SKP lane 只编进 `full-formats` 的载体（native-below 各宿主与 Linux X11/XWayland 的
  `dom-canvas`）；浏览器 slim bundle 本就不含 SKP（按 `unsupported-runtime: import.skp` 拒绝），
  因此那边既没有边能力也没有第二套实现。运行证据目前只能来自 Linux 宿主；Windows/macOS/Android/Web
  的结论只能写"仅编译/仅契约"。

## 5. 实施与证据

### 5.1 当天代码事实（阅读源码，不是运行证据）

- 已存在、无需新建：`Body3d`/`BodyDisplay`（`crates/cad-domain/src/body.rs`）、`cad:Body3d` 的
  `display.edges` 持久化（`crates/cad-domain/src/ymodel/payload/mesh.rs`）、`validate_body` 的 wire 规则
  （`BodyKind::Wire` 接受"空 mesh + 非空 edges"，拒绝都没有）、`body_fragment` 的
  `MeshWithStoredEdges`/`Mesh`/`LineSegments` 三路径、`pick/hit.rs` 的显示边拾取、
  `MeshImport::finish()` 的逐 `type_key` 能力行。
- 本变更新增/修改：`MeshImport::push_body*`（此前不存在，与 `insert_geometry` 的逐键记账一起加入）、
  `ImportLimits.max_mesh_edges`（D5）、SKP 读侧的边携带与 D3 属性策略、`SkpPreviewEdges` /
  `SkpPreviewMesh` 的 select-filter 别名与 capability 记账（只有真正插入的 carrier 才出行）、
  D4 的纯边导入与诊断、SKP 导出两个报告码、`cad-app` 拾取把显示 body 归为 `DirectMesh`，以及
  ADR-0033 / `docs/compatibility.md` / `README.md` 的文案与 corpus 重跑。

### 5.2 NOT RUN

- `.skp` corpus 重跑、真实 SKP/RVT 样本、`cad-cli-tools` 出图、宿主窗口截图：**NOT RUN**。实现轮的
  编译与契约测试记录在本轮验证材料里，但它们不是 vendor、真实样本或 corpus 证据。
- GPU/像素/性能、Windows/macOS/Android/Web 运行：**NOT VERIFIED**。
- ADR-0033 的 corpus 数字在重跑前为 **stale**，不得引用为当前结果。
