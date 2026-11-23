# 显示边吸附/测量与 SKP 导出边验证（2026-11-23，收尾轮）

目标：ADR-0050（SKP 存储边的端点/中点吸附与 `measure = Partial` 窄声明）与 ADR-0051
（SKP 单一合并 root run + 自由边写出），以及 mesh/glTF/IFC 导出器的 stored-edge / wire-body
报告。**只有编译与契约测试证据**；没有任何运行时、真实 GPU、像素、性能、真机或
SketchUp/Trimble 验收证据。

> 本文件记录的是实现与 provenance lane 在本轮跑出的结果，不是产品验收。真实 SKP corpus
> **未重跑**（旧的 `763/728/25/10` 扫描仍是 ADR-0049 之前的历史数字）。

## 跑过的命令与结果

| 命令 | 结果 |
|---|---|
| `cargo test -p cad-connector-core --locked` | **50 passed**（`push_body*`、`declare_edge_snap_measure`、逐 type key 能力行 + 天花板） |
| `cargo test -p cad-connector-skp --locked` | **89 passed**（导入边载体/纯边、吸附可用性、单一合并 run、wire 自由边、零长/全零丢弃、报告码） |
| `cargo test -p cad-measure --locked` | **58 + 2 passed** |
| `cargo test -p cad-connector-mesh --locked` | **88 passed**（STL/OBJ/PLY/OFF 的 stored-edge `Partial`、wire-body 丢弃、全退化面无边 note、无边字节不变） |
| `cargo test -p cad-connector-gltf --locked` | **76 passed**（同一批边语义 + 既有几何/材质回归） |
| `cargo test -p cad-connector-ifc --locked` | **80 passed**（同上，含既有 BIM 导出回归） |
| `cargo test -p cad-app --lib --locked` | **3154 passed**（来源汇总、取证精度、宿主按 `type_key` 门控、吸附候选、all-zero wire 实体） |
| `cargo check --workspace --all-targets --locked` | 通过 |
| `cargo check -p cad-app --no-default-features --locked` | 通过（slim/无 `formats-skp` 时的门控谓词与类型引用） |
| `cargo clippy -p cad-connector-core -p cad-connector-skp -p cad-measure -p cad-app -p cad-connector-mesh -p cad-connector-gltf -p cad-connector-ifc --all-targets --locked -- -D warnings` | 通过 |
| `cargo fmt --all -- --check` | 通过 |
| `python3 scripts/check-architecture.py` | `Architecture OK: 38 packages, acyclic and core boundaries intact` |
| `python3 scripts/check-file-size.py` | `file-size OK: 2637 code file(s), limit 1000 lines`（SKP 导出测试已拆为 `tests/{basic,geometry,wire}.rs`） |
| `python3 scripts/check-i18n.py` | `i18n OK: 3129 keys, 2 catalogs` |
| `python3 scripts/check-command-docs.py` | `OK: 0 mismatches` |

注：上表是最终工作树上的收尾复跑；`cad-app` 的全量 lib 套件由 provenance 修复 lane 在最终代码上运行（3154 passed）。未设置 `YACR_TEST_SKP` 的用例仍按跳过处理，跳过不是已运行证据。

## 关键契约（本轮固定）

- **吸附/测量**：只有实际插入非空 `BodyDisplay.edges` 的 `SkpPreviewMesh` / `SkpPreviewEdges`
  报 `measure = Partial`；三角-only 组与 ACIS/其它 body 仍 `Unsupported`。候选只有端点与中点；
  无面/三角/中心/象限/垂足/交点，无面积/体积。宿主按 `type_key`（且编入 `formats-skp`）门控，
  不在宿主文件里按字符串分叉。release 天花板只收窄（`Partial` → `Unverified`）。
- **测量来源汇总**：无吸附点 → `UserPoints`/`Analytic`；有吸附点时取最少权威来源
  （`DirectMesh` → `ProxyCache` → `KernelMesh` → 有原始拾取点则 `UserPoints` → `Analytic`），
  精度取吸附点中最弱者；`ProxyCache` 目标精度为 `Approximate`（无 error bound）；
  `snaps` 与 `points` 不平行按 `measurement.snaps_not_parallel` 拒绝。
- **导入诊断**：`stats.edges.emitted > 0` 时恒发 `import.skp.measure_edges_partial`，detail 写明
  窄声明；没有边不发。
- **SKP 导出**：所有源并入一个 root `GeometryRun`（修复「只写第一个实体」）；多源记
  `export.skp.entity_grouping_flattened`（`Partial`）；`max_output_entities` 按源实体计。wire 体
  写成自由边（自己的端点、不焊接、N−1 链式、默认 flags/layer/material、`curve: None`；零长段
  `export.skp.zero_length_edge`；重复端点对只写一次；曲线/属性
  `export.skp.edge_semantics_not_written`）；`export.skp.wire_body_not_written` 退役；整条 wire
  体全部是零长段时按 `export.skp.wire_body_no_writable_edges` 丢弃（`Missing`）。
- **mesh 体 stored 边**：仍不复制为自由边，逐实体 `export.skp.stored_edges_not_written`
  （`Partial`）；全退化三角形不额外声称边丢失。
- **mesh/glTF/IFC**：写出的表面带 stored 边时逐实体 `export.<id>.stored_edges_not_written`
  （`Partial`）；wire-only body 整体丢弃 `export.<id>.wire_body_not_written`（`Unsupported`，
  不重复记）；无边文档逐字节不变。
- **RVT**：仍没有真实边，也不派生（ADR-0049 D6 不变）。

## 明确 NOT RUN

- 真实 SKP corpus（`YACR_TEST_SKP` 未设置）；旧的 `763/728/25/10` 仍是历史扫描，未重跑。
- 真实 GPU / 像素 / 性能；`native-below` 或 `dom-canvas` 宿主窗口里的吸附/测量复看。
- Windows / macOS / Android / Web 载体运行；只有编译/契约证据。
- **SketchUp / Trimble 打开导出文件**：SKP 导出证据仅 `skpwright` 自洽往返
  （写出→本仓库读回同一三角/边数与坐标），不是 vendor 接受性，也不是 golden。
- 边级选择/编辑、面/三角/中心/象限/交点吸附、面积/体积。

## 边界

- 自洽往返 = **同一实现读自己写的字节**。它不能证明 SketchUp 接受该文件，也不能证明任何
  vendor 语义。
- 吸附/测量只覆盖 SKP 显示边载体；其它格式的真实边（如果将来出现）需要各自的能力行与门控。
- `.ymodel` schema 与 `FORMAT_MINOR` 不变；RVT 的「无真实边、不派生」不变。
