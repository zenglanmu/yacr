# ADR 0051：SKP 单一合并 run 的自由边导出

- 状态：**Accepted（2026-11-23）**。本 ADR 是**决策记录**：它取代
  [ADR-0049](0049-real-imported-edges-as-body-display.md) §2「自由边写出延后」与
  「wire 体记 `export.skp.wire_body_not_written`」的**实现边界**；ADR-0049 原文保留，只加
  superseded 注记。
- 日期：2026-11-23。
- 相关：[ADR-0049](0049-real-imported-edges-as-body-display.md)（显示边载体与反伪造不变量）、
  [ADR-0041](0041-save-export-split-and-format-provenance.md)（导出 L1 + 逐条报告）、
  [ADR-0048](0048-format-connector-crates.md)（连接器归属）、
  [ADR-0031](0031-acadrust-vendored-patch-policy.md) /
  [ADR-0035](0035-rvt-vendored-patch-policy.md)（只允许 `acadrust`/`rvt` 两个补丁）、
  `docs/compatibility.md`、`README.md`。
- 影响：`crates/cad-connector-skp/src/export/{mod,write}.rs`（合并 run、自由边、报告码）、
  `crates/cad-connector-mesh` / `-gltf` / `-ifc` 的导出报告（同一批边语义）。
  **不改变**：`cad-db` 唯一权威、`.ymodel` schema、导入侧边载体、RVT 路径（仍无真实边）、
  补丁台账、其它导出器的几何字节（无边的文档逐字节不变）。

---

## 0. 背景

SKP 导出本来按「每个源网格一个 root `GeometryRun`」构造 `skpwright::Model`，然后
`Document::from_model_with` 序列化。实际行为与报告不一致：

- **钉死的写出器只发第一个 root run**。`skpwright` 0.6.0 的 `build26.rs` 用
  `model.geometry.iter().find(|r| r.def_index.is_none())` 取根 run，后续 root run 被**静默
  丢弃**。所以「一源一 run」的布局在只有多实体文档里**只写出第一个实体**，而报告仍把这些
  实体计成 `exact`——这是既有的数据丢失，不是本轮引入的。
- **wire 体什么都没写**。没有三角形、只有 `BodyDisplay.edges` 的 body 落到
  `export.skp.wire_body_not_written`（`Unsupported`），因为写出器的旧布局没有把世界坐标
  polyline 变成 `MeshEdge` 的路径。

ADR-0049 当时把「自由边写出」定为后续事项，理由是要先决定「追加端点还是按容差吸附到已有
顶点」。本 ADR 给出决定，并同时修掉第一个 root run 的静默丢失。

## 1. 决策

### D1 — 一个合并的 root `GeometryRun`

写出器只建**一个** `skpwright::GeometryRun`（`def_index: None`），把所有已收集的源实体按
**确定性的数据库顺序**合并进去：

- 每个源的三角形索引按**运行中的顶点基址**偏移后追加；
- 顶点坐标按同一单位换算（米），不重排、不合并、不焊接；
- 同一个文档重复导出得到同一字节序列（固定时间/种子的既有约定不变）。

这条布局与钉死写出器的行为**一致**，也是唯一能保证「所有源都真的写出」的布局。

### D2 — 预算按**源实体**计数，不按 run

`max_output_entities` 计的是 `collect` 选中的**源实体数**（`sources_seen`），包括后来没有
贡献任何几何的源（全部三角形退化、全部边段零长）。理由很直接：合并后模型永远只有一个 run，
按 run 计数会让任意大的文档都通过；按「真正写出的源」计数则允许一堆被丢弃的 body 绕过预算。
`max_output_triangles` 仍按合并后 run 的 `mesh.faces` 总数检查；超限是具名
`export.skp.limits_exceeded` 拒绝，不截断、不产生部分文件。

### D3 — wire 体写成自由边（自己的端点，不焊接）

`Body3d` 没有三角形但 `display.edges` 非空时，按以下规则写成 `skpwright::MeshEdge`：

1. **追加自己的端点**：每条 polyline 把自己的点追加进 run 顶点表，**不**与已有网格顶点或
   其它 polyline 焊接（没有容差匹配、没有共享顶点推断）。
2. **N−1 段链式**：一条 N 点 polyline 写成 N−1 段，内部索引在同一 polyline 内共享。
3. **默认旗标**：每段是默认 `MeshEdge` —— `curve: None`、layer 0、无材质、默认
   soft/smooth/hidden/casts_shadows/receives_shadows。显示 polyline 没有这些属性的载体，
   所以不猜。
4. **零长段跳过**：两端点重合的段无法成为 SketchUp 边，按 `export.skp.zero_length_edge`
   逐段计数（`Partial`）。若一个 wire 体的**所有**段都是零长，它什么也写不出，整实体按具名
   `export.skp.wire_body_no_writable_edges` 丢弃，而不是在空写出后只留一条 `Partial` note。
5. **重复/回描对归一**：同一条 polyline 内重复出现的端点对（按坐标规范化后的对）只写一次。
   这是**坐标归一化**，写出的边集合不变，因此这种源仍计 `exact`；被跳过的零长段改变了边
   集合，所以它不计 `exact`。

曲线/弧身份、逐边 flag、材质、图层从显示 polyline **不可还原**，因此一次
`export.skp.edge_semantics_not_written`（`Partial`）汇总声明，而不是逐边刷屏。

### D4 — 报告码与分组丢失

| 码 | 状态 | 含义 |
|---|---|---|
| `export.skp.entity_grouping_flattened` | `Partial`，一条 | 多于一个源被写进同一 root run：每个源仍写出，但「一实体一 run」的分组不保留 |
| `export.skp.edge_semantics_not_written` | `Partial`，一条 | 自由边的 curve/arc 身份与逐边 flag/材质/图层没有载体 |
| `export.skp.zero_length_edge` | `Partial`，一条 | 有零长边段被跳过（带计数） |
| `export.skp.wire_body_no_writable_edges` | 逐实体丢弃 | 一个 wire 体的所有段都是零长，写不出任何边 |
| `export.skp.stored_edges_not_written` | `Partial`，逐实体 | **mesh 体**：三角形写出，stored 边集/分段不复制；三角形边界由写出器重新生成 |
| `export.skp.degenerate_triangle` | 逐三角形 | 退化三角形丢弃；若某 mesh 体所有三角形都退化，则它没有写出表面，只报这一点，**不**额外声称边丢失 |
| `export.skp.unsupported_geometry` | 逐实体 | 既不是网格、也不是有三角形或显示边的 body |
| ~~`export.skp.wire_body_not_written`~~ | **退役** | wire 体现在写成自由边；常量已删除，任何路径都不再产生 |

分组丢失是**整份导出一次**的 `Partial`：它不逐实体重复，也不改变每个源「已写出」的事实；
`CompatCounts.converted` 计入它一次（与每个受影响的 stored-edge 实体各计一次并列）。

### D5 — mesh 体的 stored 边**仍不**写成自由边

一个**有三角形**的 `Body3d` 写出它的三角形，stored 边集/分段**不复制**：写出的三角形边界由
写出器重新生成 `MeshEdge`。原因：

- stored 边是世界坐标 polyline，没有 run 顶点身份；要同时写它和三角形边界，必须先把 polyline
  焊接进三角形顶点集，否则同一条物理边会被写两遍（一次是面环，一次是自由边）。
- 焊接是一个几何/拓扑决定：ADR-0049 D2 明确禁止导入侧焊接、去重或发明坐标；在写出侧做同样
  的事会把「源里有几条边」变成「写出器认为该有几条边」，并且受容差影响。
- 因此这条缺口按实体记 `Partial`（`export.skp.stored_edges_not_written`）：**面边界边随三角形
  存活，游离或另行分段的 stored 边不存活**。这不是静默丢失。

### D6 — 同批语义在 mesh/glTF/IFC 导出器

不属于本 ADR 的核心布局（那些格式没有 `skpwright` 的单 root run 约束），但边语义必须一致：

- 写出的**表面**带 `BodyDisplay.edges` 时，每个受影响实体记一条 `Partial`
  `export.<fmt>.stored_edges_not_written`（`stl`/`obj`/`ply`/`off`/`gltf`/`ifc`）；
- **wire-only** body（无三角形、只有边）被整体丢弃，记 `Unsupported`
  `export.<fmt>.wire_body_not_written`，且**不**再重复记同实体的 stored-edge 丢失；
- 全部面退化的 STL 表面没有写出表面，因此不记边丢失（只记面丢弃）；
- 完全无边的文档在这些导出器里**逐字节不变**。

RVT 仍**没有**真实边，也不派生边（ADR-0049 D6）。

## 2. 被否决的方案

- **给 `skpwright` 打补丁改 root-run 选择**：补丁台账只允许 `acadrust` 与 `rvt`（ADR-0031/
  ADR-0035），SKP 不在名单内；为一个布局选择去改依赖源码会把整条 SKP lane 变成不可固定的
  vendor 分支。
- **保持一源一 run、指望写出器改**：在钉死的 0.6.0 上这就是「只写第一个实体并报 `exact`」的
  静默丢失；不改上游就只能在写出侧合并。
- **一个源一个 run + 只写第一个、其余报 dropped**：诚实但不可用——多实体文档导出结果取决于
  数据库顺序，而且每个后续实体都变成拒绝。
- **按容差焊接 polyline 到网格顶点**：引入容差语义、改变源边数量、违背 ADR-0049 D2；并且
  同一模型在不同缩放下会得到不同边数，不可重复。
- **把 curve/弧身份猜回来**：显示 polyline 只有两点弦，重建弧需要不存在的半径/端点信息；
  猜测会制造 SketchUp 里不存在的曲线实体。

## 3. 后果与未改变

- **修掉的既有缺陷**：多实体文档不再只写第一个源；wire 体不再整体丢失。
- **已接受的取舍**：
  - 分组丢失（`entity_grouping_flattened`）：SketchUp 侧看不到原文档的实体边界，只有一个 run。
  - 自由边与 mesh 端点在 run 内不共享顶点；同一位置可能出现重复顶点（显示数据本来就如此）。
  - mesh 体的 stored 边集/分段仍不复制（D5）。
  - `converted` 计数随「受影响实体数 + 分组丢失 + 自由边 note + 零长 note」增长，`Complete`
    只在完全没有这些 note 时出现。
- **未改变**：导入侧载体与判据、`.ymodel` schema、`cad-db` 权威、其它格式的几何字节、
  单位拒绝（`export.skp.units_unknown`）、固定时间/种子、连接器归属。

## 4. 证据与边界

- **只有 `skpwright` 自洽往返**：写出 → `cad-connector-skp` 读回同一三角数/边数/坐标，
  加 `skpwright::verify`。这是**同一实现自己读自己**的证据，不是 SketchUp/Trimble 接受性、
  不是 golden、不是产品验收。
- **NOT RUN / NOT VERIFIED**：真实 SketchUp 打开写出文件、真实 SKP corpus、GPU/像素、宿主
  窗口交互、Windows/macOS/Android/Web 运行；本 ADR 的结论只到编译与契约测试。
- 不得把「自洽往返通过」写成「SketchUp 兼容」；也不得把 `Partial` 的缺失项（分组、边语义、
  mesh stored 边）说成已解决。
