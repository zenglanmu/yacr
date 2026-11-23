# 真实导入边验证（2026-11-23）

目标：ADR-0049 —— 把 SketchUp 的真实 `MeshEdge` 记录作为 `Body3d`/`BodyDisplay.edges` 显示数据
导入；纯边文件不再拒绝。RVT 不走这条路径（当前 GLB 只有三角形，禁止派生）。

> 本记录只有**编译/契约测试**证据。没有任何运行时、真实 GPU、像素、性能、真机或 vendor
> 格式验收证据；真实 SKP corpus 未重跑。

> **Superseded（2026-11-23）**：本记录中「测量仍 `Unsupported`（无 snap）」与「SKP 自由边写出
> 未做」两句已被 ADR-0050/0051 取代，当前行为见 `docs/validation-edge-followups-2026-11-23.md`
> 与 `docs/adr/0050-display-edge-snap-and-measure.md`、`docs/adr/0051-skp-single-run-free-edge-export.md`；
> 其余内容按历史原样保留。

## 跑过的命令与结果（最终工作树）

| 命令 | 结果 |
|---|---|
| `cargo test -p cad-connector-skp --locked` | **75 passed**（含边提取、body/wire 载体、hidden/soft/纯边、失败面、预算、导出回读） |
| `cargo test -p cad-connector-core --locked` | **48 passed**（`push_body*`、多 type_key 能力行、边-only 插入） |
| `cargo test -p cad-app --lib select_filter --locked` | **25 passed**（`SkpPreviewMesh` 仍匹配 `TYPE=MESH`） |
| `cargo test -p cad-app --lib a_display_body_is_a_direct_mesh_pick_item_not_analytic --locked` | **1 passed** |
| `cargo test -p cad-scene -p cad-representation -p cad-spatial --locked` | **365 / 87 / 59 passed** |
| `cargo check --workspace --all-targets --locked` | 通过 |
| `cargo clippy -p cad-connector-core -p cad-connector-skp --all-targets --locked -- -D warnings` | 通过 |
| `cargo fmt --all -- --check` | 通过 |
| `python3 scripts/check-architecture.py` | `Architecture OK: 38 packages, acyclic and core boundaries intact` |
| `python3 scripts/check-file-size.py` | `file-size OK: 2633 code file(s), limit 1000 lines` |
| `python3 scripts/check-i18n.py` | `i18n OK: 3129 keys, 2 catalogs` |
| `python3 scripts/check-command-docs.py` | `OK: 0 mismatches` |

关键契约：存储边只能来自真实 `MeshEdge`（面环判据，缺记录退回 `Mesh` + ADR-0045 派生边界）；
hidden/soft/不可见图层的边剔除并有计数；纯边导入为 `Partial` 的 `SkpPreviewEdges` wire body；
`max_mesh_edges` 默认 2,000,000、按文件累计、超限拒绝而不截断；测量仍 `Unsupported`。

## 明确 NOT RUN

- 真实 SKP corpus（`YACR_TEST_SKP` 未设置）：旧的 `763/728/25/10` 扫描是 ADR-0049 之前的
  有界实验结果，边-only 文件现在的实际成功数**没有新证据**；`fixtures/skp/SOURCE.md` 与
  `docs/compatibility.md` 已把旧数字标为历史。
- 全量 `cargo test --workspace --locked -- --test-threads=1`。
- 真实 GPU / 像素 / 性能；`native-below` 宿主打开 `house.skp` 的窗口级复看。
- Windows/macOS/Android/Web 载体运行（共享层契约一致，但没有运行证据）。
- SKP 自由边写出（导出只保留三角形并报告 stored edge set/segmentation 未复制）、
  STL/OBJ/PLY/OFF/glTF/IFC 的边通道（这些写出器只读 `display.mesh`，本轮未改）。
- 边级选择/编辑、测量/端点吸附：均不在本轮范围。

## 边界

- 边是**显示与拾取**数据：没有边 id、共享顶点拓扑或子元素身份；wire body 属性为 0 顶点/0 三角形，
  由 `type_key = SkpPreviewEdges` 说明。
- RVT 预览路径没有真实边/曲线，且不派生；这是与 SKP 的证据边界差异。
- `.ymodel` schema 与 `FORMAT_MINOR` 不变（`cad:Body3d` 已持久化 edges）。
