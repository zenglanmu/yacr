# 网格内核、布尔/修复、面编辑、补洞与在线样本（P-MESH-0/1a/1b/2/3）验证记录 — 2026-11-26

范围：[ADR-0061](adr/0061-mesh-kernel-selection.md) 的 **P-MESH-0**（`MeshModeler` 接缝 +
`cad-kernel-manifold` + 架构/许可/门禁）、**P-MESH-1a**（`UNION`/`SUBTRACT`/`INTERSECT` 的网格
操作数 + worker/取消/陈旧检查 + `AcDbSubDMesh` 提交）、**P-MESH-1b**（`MESHREPAIR`/
`MESHREBUILD` + `Partial`）、**P-MESH-2**（SKP 面身份 + 无内核 `cad-mesh-edit` 面/边移动/
推拉/细分/平滑）与 **P-MESH-3**（`MESHCAP` 边界环补洞 + 逐洞报告：平面、非平面投影、多个
独立环与共面嵌套环带）。**边身份为几何派生 `mesh:edge:<bits>` key；wire-only 边/跨多三角形边
polyline 仍不可编辑；自交/退化环、同向绕行或找不到可见桥的嵌套组仍拒绝**。

## 1. 已运行

| 命令 | 结果 | 证据 |
|---|---|---|
| `cargo test -p cad-kernel-adapter --lib --locked` | **PASS** 66 passed, 0 failed | `mesh::tests::*`：闭合/开口/非流形分类、未焊接立方体的位置焊接分类、空网格、越界/重复索引、能力探测 |
| `cargo test -p cad-kernel-manifold --locked` | **PASS** 10 passed, 0 failed | 真实固定内核：重叠立方体 ∪/∖/∩ 体积 12/4/4；未焊接（逐三角形顶点）闭合立方体仍可布尔；不相交交为 `mesh.failed`；open shell → `mesh.not_closed`；朝向修复；rebuild；闭合非流形汤 → `Partial` + 体积 16；预取消；空操作数 `InvalidInput`；布尔输出必须 `ClosedManifold` |
| `cargo test -p cad-app --lib --locked tests::drawing::mesh_solids -- --test-threads=1` | **PASS** 7 passed, 0 failed | 网格布尔命令启动、`boolean.operand_class_mismatch`、无 modeler 的 `unsupported-runtime: mesh-modeler`、open 拒绝、`MESHREPAIR` 单操作数/非网格拒绝、mesh/solid 互斥 |
| `cargo test -p cad-app --lib --locked host::tests::mesh_jobs -- --test-threads=1` | **PASS** 5 passed, 0 failed | 真实 worker：`AcDbSubDMesh` 提交、一条事务/一步撤销、repair `Partial`、open 拒绝不写库、取消不写库、操作数被编辑 → `Stale` |
| `cargo test -p cad-spatial --lib --locked` | **PASS** 60 passed, 0 failed | 边点击返回 `mesh:edge:<bits>` 子元素；面点击仍返回 `face_sources`；wire-only 边保持整实体 |
| `cargo test -p cad-ui-core --lib --locked mesh_repair_selectors` | **PASS** 1 passed, 0 failed | `MESHREPAIR`/`MESHREBUILD`/`MESHREFINE`/`MESHSMOOTH`/`MESHCAP` 各在 `COMMAND_TABLE` 出现一次，规范化正确，Work-only |
| `cargo test -p cad-mesh-edit --locked` | **PASS** 22 passed, 0 failed | 无内核编辑层：焊接、边界环、面区域、面移动/推拉、边 key 解析/移动、细分、平滑、平面/非平面投影补洞、两个独立环、共面嵌套环带（方形管上下环带体积 6、带两孔平板、带岛平板深度 2 独立补）、退化拒绝、无面身份拒绝 |
| `cargo test -p cad-app --lib --locked tests::drawing::mesh_edit -- --test-threads=1` | **PASS** 8 passed, 0 failed | `MOVE` 面区域/边、`PRESSPULL` 网格面、`MESHREFINE`（48 三角形/1 undo）、`MESHSMOOTH`、`MESHCAP` 闭合/诊断、无面身份与 no-holes 拒绝 |
| `cargo test -p cad-host --lib --locked press_pull -- --test-threads=1` | **PASS** 3 passed, 0 failed | 原精确捕获不变；无 solid modeler 时网格面 `PRESSPULL` 仍可启动并捕获面点 |
| `cargo test -p cad-cli-tools --test mesh_command --locked -- --test-threads=1` | **PASS** 5 passed, 0 failed | 真实 `manifold-rust` + 真实命令/宿主：∪/∖/∩ 体积 12/4/4、闭合、`Revision(1)`；repair 闭合；闭合非流形汤重建为 `Partial` 体积 16；`Body3d{Unknown}` 操作数；open 拒绝且 `Revision(0)` |
| `YACR_TEST_MESH_DIR=/tmp/opencode/yacr-mesh-samples-check cargo test -p cad-cli-tools --test mesh_real_samples --locked -- --nocapture` | **PASS** 1 passed, 0 failed | 11 个在线样本/34,347 三角形：5 closed 修复成功、1 open 按 `mesh.not_closed` 拒绝、5 non-manifold 结构化拒绝/失败；两个闭合样本 union 成功（178 三角形）；open 样本 `fill_refused=1`、`fill_filled=0`（边界环零面积，按 `mesh.hole_degenerate` 拒绝） |
| `cargo test -p cad-connector-skp --locked` | **PASS** 91 passed, 0 failed | `tests::sources::*`：每个三角形恰有一个 `skp:face:<n>`，同一源面共享 key |
| `cargo check -p cad-kernel-manifold --target wasm32-unknown-unknown --locked` | **PASS** | 纯 Rust、无 C++/FFI；`parallel` 未启用 |
| `cargo clippy -p cad-mesh-edit -p cad-app -p cad-host -p cad-ui-core -p app-linux --all-targets --locked -- -D warnings` | **PASS** | |
| `python3 scripts/check-architecture.py` | **PASS** | 40 packages；`cad-kernel-manifold` 唯一拥有 `manifold-rust`；`app-linux` 是唯一注入它的载体；`cad-mesh-edit` 无内核依赖 |
| `python3 scripts/check-file-size.py` | **PASS** | 2900 files，limit 1000 |
| `cargo fmt --all -- --check` | **PASS** | |
| `cargo check -p cad-app -p cad-ui-core -p cad-host -p app-linux --all-targets --locked` | **PASS** | 新命令/宿主/载体接线编译通过 |

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
- 这是**内核/导入层**证据：测试直接调用 `ManifoldMeshModeler`，不是 app 命令通过。P-MESH-1a
  的 `UNION`/`SUBTRACT`/`INTERSECT` 命令接线由 §3 的 `mesh_command`/`mesh_jobs` 证据覆盖。
  几何是 `skpwright` 的预览三角化，不是 SketchUp/Trimble 产品验收。
- 只跑了一个本地样本；更多 release/复杂组件、真实 GPU/像素、Windows/macOS/Android/Web 均未验证。

## 3. 在线网格样本语料（真实下载，2026-11-26）

`scripts/fetch-mesh-samples.sh` 从两个公开来源下载 11 个样本到仓库外目录（默认
`/tmp/opencode/yacr-mesh-samples`），逐个校验 SHA-256；样本**不入库**：

- `manifold` 上游测试模型（Apache-2.0 仓库，固定 commit
  `c5ffc47388b86250c2eb70c2ddf0b64d3678207c`）：`Cray_left/right`、`r1/r2`、
  `hull-body/mask`、`dedupe_stale_duplicate`、`degenerate_split_quad_shared_a/b`、
  `openscad-nonmanifold-crash`；
- Stanford 3D Scanning Repository bunny（`bun_zipper_res2.ply`，研究用途；上游许可是权威）。

运行：

```text
bash scripts/fetch-mesh-samples.sh /tmp/opencode/yacr-mesh-samples-check
YACR_TEST_MESH_DIR=/tmp/opencode/yacr-mesh-samples-check \
  cargo test -p cad-cli-tools --test mesh_real_samples --locked -- --nocapture
```

本次输出（逐文件）：

```text
Cray_left.obj: entities=1 triangles=576 closed=1 open=0 non_manifold=0
Cray_right.obj: entities=1 triangles=12 closed=1 open=0 non_manifold=0
bun_zipper_res2.ply: entities=1 triangles=16301 closed=0 open=0 non_manifold=1
dedupe_stale_duplicate.obj: entities=1 triangles=224 closed=0 open=0 non_manifold=1
degenerate_split_quad_shared_a.obj: entities=1 triangles=82 closed=0 open=0 non_manifold=1
degenerate_split_quad_shared_b.obj: entities=1 triangles=82 closed=0 open=0 non_manifold=1
hull-body.obj: entities=1 triangles=5460 closed=1 open=0 non_manifold=0
hull-mask.obj: entities=1 triangles=7980 closed=1 open=0 non_manifold=0
openscad-nonmanifold-crash.obj: entities=1 triangles=1441 closed=0 open=0 non_manifold=1
r1.obj: entities=1 triangles=1121 closed=0 open=1 non_manifold=0
r2.obj: entities=1 triangles=1068 closed=1 open=0 non_manifold=0
SUMMARY files=11 meshes=11 triangles=34347 closed=5 open=1 non_manifold=5 repaired=5 refused_open=1 non_manifold_repaired=0 non_manifold_failed=3 non_manifold_refused=2 fill_filled=0 fill_refused=1 fill_no_loop=0 union=ok(triangles=178)
```

解释：5 个闭合网格全部被 `Orientation` 修复接受；1 个开放网格按 `mesh.not_closed` 拒绝；
5 个非流形/退化样本没有崩溃或空成功——0 个被接受，3 个按 `mesh.failed` 结构化失败，2 个按
`mesh.not_closed`/`mesh.not_manifold` 结构化拒绝。非流形闭合汤成功重建为 `Partial` 的证据由
`cad-kernel-manifold` 的 `a_closed_non_manifold_soup_rebuilds_to_a_partial_closed_product`
与 `cad-cli-tools/tests/mesh_command.rs` 的 `MESHREBUILD` 用例提供（两个面贴合立方体、双重共享墙，
重建后体积 16）。两个 `Cray` 闭合样本的 kernel-level `Union` 返回 178 三角形的闭合网格。
**这只属于这 11 个文件**；样本来源/许可见脚本头注释，不是 SketchUp 语料或产品兼容性结论。

## 4. NOT RUN / 未验证

- **P-MESH-2 边身份是几何派生 key**：三角形边点击返回 `mesh:edge:<bits>`，`MOVE` 平移两个端点；
  **wire-only body 的边/跨多个三角形边的 stored polyline 仍是整实体**，没有假边 id。
- **P-MESH-3 边界**：平面环、非平面投影环、多个独立环、共面嵌套环带已实现；自交/退化环、
  同向绕行（`mesh.hole_nested_inconsistent_winding`）或找不到可见桥的嵌套组仍按名拒绝并保持
  开口；没有“所有开放网格都能闭合”的声明。
- 真实 `.skp` 语料（`YACR_TEST_SKP`）**NOT RUN**；SKP 面身份只有合成模型测试与
  `skpwright` 自洽 roundtrip 证据。
- **内核运行期取消未测**：只验证了预取消与宿主令牌取消（取消后不写库）。原生看门线程已实现，
  但没有可复现的长布尔用例证明内核在计算中间被中断；wasm 明确只有预/后检查，不得声称运行期取消。
- Windows/macOS/Android 的 `manifold-rust` 运行与载体注入 **NOT VERIFIED**；只有 Linux 原生
  契约/单元证据与 wasm 编译证据。
- Web/`dom-canvas` 的网格**布尔/修复**仍无执行路径；会话没有注入 `MeshModeler`，因此按
  `unsupported-runtime: mesh-modeler` 显式拒绝（不是静默主线程执行）。网格面编辑/细分/平滑/
  补洞是纯 `cad-mesh-edit` 逻辑，不依赖该 modeler。
- 任何 GPU/像素/性能/真实 SketchUp 兼容性结论均 **NOT RUN**。
- 本仓库的网格布尔尚未与 SketchUp/Trimble 对照；上游 Thingi10K 语料结论是
  `manifold-rust` 自述，不是本仓证据。只有 11 个公开样本与一份本地 `house.skp`，不是语料统计。

## 5. 平台/能力边界

| 载体 | 状态 |
|---|---|
| Linux 原生 `native-below` | `ManifoldMeshModeler` 已注入；P-MESH-1a/1b 命令/worker/commit 与 P-MESH-2/3 编辑/补洞契约真实执行 |
| Linux `dom-canvas`（X11/XWayland） | 无 `MeshModeler`：布尔/修复按 `unsupported-runtime: mesh-modeler` 拒绝；纯面编辑/细分/平滑/补洞不依赖内核（运行时未在 X11 专项验证） |
| Windows / macOS / Android | 无 `MeshModeler` 注入；编译/运行时 NOT VERIFIED |
| Web | 无 `MeshModeler`：布尔/修复拒绝；纯面编辑/补洞代码可编译，浏览器运行时 NOT VERIFIED |

`docs/concept-sketchup.md` 是 SketchUp 工具面的可数覆盖表；本文件记录 P-MESH-0/1a/1b/2/3 与
样本语料的证据边界，不把 75.4% 工具面覆盖写成网格能力验收。
