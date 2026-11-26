# 网格内核与 SKP 面身份（P-MESH-0）验证记录 — 2026-11-26

范围：[ADR-0061](adr/0061-mesh-kernel-selection.md) 的 **P-MESH-0**（`MeshModeler` 接缝 +
`cad-kernel-manifold` + 架构/许可/门禁）与 P-MESH-2 的**导入半**（SKP 源面身份写入
`Mesh.face_sources`）。**P-MESH-1（网格布尔/修复命令）与 P-MESH-2 的编辑半未实现**。

## 1. 已运行

| 命令 | 结果 | 证据 |
|---|---|---|
| `cargo test -p cad-kernel-adapter --lib --locked` | **PASS** 66 passed, 0 failed | 新增 `mesh::tests::*`：闭合/开口/非流形分类、未焊接立方体的位置焊接分类、空网格、越界/重复索引、能力探测 |
| `cargo test -p cad-kernel-manifold --locked` | **PASS** 9 passed, 0 failed | 真实固定内核：重叠立方体 ∪/∖/∩ 体积 12/4/4；未焊接（逐三角形顶点）闭合立方体仍可布尔；不相交交为 `mesh.failed`；open shell → `mesh.not_closed`；朝向修复（有向体积由负转正）；rebuild；预取消；空操作数 `InvalidInput` |
| `cargo test -p cad-connector-skp --locked` | **PASS** 91 passed, 0 failed | 新增 `tests::sources::*`：每个三角形恰有一个 `skp:face:<n>`，同一源面共享 key，不同面不同 key，跳过面不产生条目 |
| `cargo check -p cad-kernel-manifold --target wasm32-unknown-unknown --locked` | **PASS** | 纯 Rust、无 C++/FFI；`parallel` 未启用 |
| `cargo clippy -p cad-kernel-adapter -p cad-kernel-manifold --all-targets --locked -- -D warnings` | **PASS** | |
| `cargo clippy -p cad-connector-skp --all-targets --locked -- -D warnings` | **PASS** | 由 SKP lane 报告，本文件复核命令通过 |
| `python3 scripts/check-architecture.py` | **PASS** | 39 packages；`cad-kernel-manifold` 唯一拥有 `manifold-rust`；仅载体可运行时依赖该 adapter |
| `python3 scripts/check-file-size.py` | **PASS** | 2881 files，limit 1000 |
| `cargo fmt --all -- --check` | **PASS** | |
| `cargo check -p cad-app -p cad-ui-core -p app-linux --all-targets --locked` | **PASS** | 新 crate 尚未接入 carrier，确认没有破坏既有依赖 |

### 选择探测（仓库外，非门禁）

2026-11-26 在 `/tmp/opencode/mesh-probe` 用临时 crate 运行 `manifold-rust 0.19.6`：

- `cube(2) − sphere(1,64)` → `NoError`，`volume = 3.834863`（64 段球面近似的闭式差；精确球为
  3.81121），`verts=1034`、`tris=2060`；
- 删除一个三角形后 `from_mesh_gl64_robust` → `NotClosed`；
- 原生与 `wasm32-unknown-unknown` 均编译通过。
- 未焊接立方体（36 个顶点、12 个三角形）的 robust 导入为 `NoError`、体积 8，说明内核按位置处理；因此接缝的预检必须用 `inspect_topology_welded`，不能按索引直读拒绝。

该探测只用于选型，**不是**仓库测试，也不是 GPU/真机/性能证据。

## 2. 真实 SketchUp 样本（本地运行，2026-11-26）

样本：本地 `house.skp`，**2,237,796 bytes**，SHA-256
`9cd2689df7c1bf0d1ce0d496e4e83c9367abab6d8ff3e21dc9bb8162e982dd30`。样本**不入库**（无再分发
许可）；下面的数字只属于这一份文件，不是 SketchUp 语料结论。

| 命令 | 结果 |
|---|---|
| `YACR_TEST_SKP=<house.skp> cargo test -p cad-connector-skp --locked a_real_sketchup_model_imports_when_supplied -- --nocapture` | **PASS**：`meshes=64 triangles=1440 vertices=2590 area=457.597332 completeness=partial`；诊断含 `import.skp.preview_geometry`/`units_metres`/`layers_flattened`/`hidden_pruned`/`measure_edges_partial` 等 |
| `YACR_TEST_SKP=<house.skp> cargo test -p cad-connector-skp --locked a_real_sketchup_model_carries_face_identity_when_supplied -- --nocapture` | **PASS**：`meshes=64 triangles=1440 sources=1440 distinct_keys=28`；每个非空网格的 `face_sources.len() == triangles.len()`，`topology_revision == Revision(0)` |
| `YACR_TEST_SKP=<house.skp> cargo test -p cad-cli-tools --locked --test mesh_real_skp -- --nocapture` | **PASS**：`meshes=64 triangles=1440 closed=21 open=41 non_manifold=2 boundary_edges=1379 repaired=21 repaired_non_manifold=0 refused_open=41 refused_non_manifold_closed=2 refused_non_manifold_manifold=0 union=mesh` |

解释：

- **21 个闭合网格**全部被 `ManifoldMeshModeler` 的 `Orientation` 修复接受（`Partial`），其中前两个
  闭合网格的 kernel-level `Union` 返回非空 `Mesh`。
- **41 个开放网格**全部按 `mesh.not_closed` 拒绝；**没有自动补洞**。
- **2 个非流形网格**（同时有 boundary edge）按 `mesh.not_closed` 结构化拒绝（没有崩溃、没有空成功）。
- 这是**内核/导入层**证据：`UNION`/`SUBTRACT`/`INTERSECT` 的 app 命令仍未接线（P-MESH-1a 未实现），
  `union=mesh` 不是 UI 命令通过。几何是 `skpwright` 的预览三角化，不是 SketchUp/Trimble 产品验收。
- 只跑了一个本地样本；更多 release/复杂组件、真实 GPU/像素、Windows/macOS/Android/Web 均未验证。

## 3. NOT RUN / 未验证

- **P-MESH-1 未实现**：`UNION`/`SUBTRACT`/`INTERSECT` 仍只对精确 `Body3d{Solid,Exact}` 生效；
  网格体命令、worker、undo、宿主轮询、`unsupported-runtime: mesh-modeler` 均未接线。
- **P-MESH-2 编辑半未实现**：没有网格 `PRESSPULL`、面/边拾取、移动/推拉/细分/平滑。
- 真实 `.skp` 语料（`YACR_TEST_SKP`）**NOT RUN**；SKP 面身份只有合成模型测试与
  `skpwright` 自洽 roundtrip 证据。
- **运行期取消未测**：只验证了预取消（闭包一开始为 true → `CadError::Cancelled`）。原生看门
  线程 → `CancelToken` 的映射已实现，但没有可复现的长布尔用例证明内核在运行中被中断；
  wasm 明确只有预/后检查，不得声称运行期取消。
- Windows/macOS/Android 的 `manifold-rust` 运行与载体注入 **NOT VERIFIED**；只有 Linux 原生
  契约/单元证据与 wasm 编译证据。
- Web/`dom-canvas` 的网格布尔仍无执行路径；按 ADR-0061 D7 应显式拒绝，不得主线程静默阻塞。
- 任何 GPU/像素/性能/真实 SketchUp 兼容性结论均 **NOT RUN**。
- 本仓库自写的网格布尔尚未与 SketchUp/Trimble 对照；上游 Thingi10K 语料结论是
  `manifold-rust` 自述，不是本仓证据。

## 4. 平台/能力边界

| 载体 | 状态 |
|---|---|
| Linux 原生 `native-below` | P-MESH-0 契约测试真实执行；P-MESH-1 未接线 |
| Linux `dom-canvas`（X11/XWayland） | 未接线；wasm 可编译不是运行证据 |
| Windows / macOS / Android | 未接线；NOT VERIFIED |
| Web | 未接线；NOT VERIFIED |

`docs/concept-sketchup.md` 是 SketchUp 工具面的可数覆盖表；本文件只记录 P-MESH-0 的证据，
不把 50% 工具面覆盖写成网格能力验收。
