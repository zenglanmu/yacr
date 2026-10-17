# 相机-only 导航保持选择高亮 验证（2026-10-17）

**目的**：记录一次**用户报告**的缺陷（选中一个实体、缩放后**高亮消失**）的**两条在改动前已确认的
机制**、**改动**、**已调查但未改代码的部分**、**诚实的范围划分（状态 vs 高亮）**、**测试矩阵**、
**发现的预存失败**与**未测边界**。改动落在 `crates/cad-render-wgpu`
（`renderer_impl.rs` 的 key 空间重置与保留标志请求）与 `crates/cad-engine`（`render/runtime.rs` 的
`applied_entity_flags` 重推；`render/contracts.rs` 的 3 个 fence），加 `crates/cad-app`
（`render_scene/overlay.rs` 的 1 个 fence）与 `crates/cad-render-wgpu/tests/selection_flag_lifecycle.rs`
（新套件），实现 commit `38e3ae8`。**未**改宿主协议、**未**改 UI、**未**改 `commit_upload`、
**未**新增诊断码。

不变量：**相机-only 操作不是场景变更**——选择必须存活。**这不是**性能验收、**不是** vendor/视觉结论
（见「未测 / 边界」）。选择/pick 表与标志机的上游契约见 `docs/picking-gpu-id-pass.md`。

## 环境与授权边界

| 项 | 值 |
|---|---|
| 机器/会话 | 本机桌面会话，同一 UID；**无** root/`sudo`、**无**屏录、**无**虚拟/无头显示 |
| 症状来源 | **用户手动交互**（`target/release/yacr`）；用户确认**未**看属性面板，只看了**画布高亮** |
| fence 运行 | `crates/cad-render-wgpu/tests/selection_flag_lifecycle.rs`（GPU fence）；`crates/cad-engine`
  的 3 个引擎契约测试标 `#[ignore = "hardware GPU: run explicitly on a real adapter (P620/Vulkan)"]` |
| 证据落盘 | 仅本地，**未上传** |

## 报告的症状（用户手动交互，**实测**）

用户**选中一个实体**、**缩放**，画布上的**高亮消失**。用户确认**未**检查属性面板——只观察到
**画布高亮**消失（故「选择**状态**是否也丢失」在症状层面**未被用户观察到**；见「范围划分」）。

## 两条在改动前已确认的机制（**实测**）

改动前**先确认机制、再改代码**：

### 机制 1：瞬时空驻留集销毁了保留的实体标志请求

- `Renderer::reset_pick_keys`（`crates/cad-render-wgpu/src/renderer_impl.rs:4214`）会清掉**保留的
  实体标志请求**（`entity_flags_request.clear()`）。
- `rebuild_pick_table`（`renderer_impl.rs:4114`）在提交后 `batches` 与 `images` **皆空**时**调用它并
  早退**。
- 这在**相机-only 操作**上**可达**：2D 导航回退（`suppress_overlays`）截断一个**零批 base** ⇒
  一次提交后驻留集为空 ⇒ key 空间重启、请求被销毁，且**无人**重推 ⇒ 选择着色的身份请求丢失。
- **修复**：请求现在**在瞬时空驻留集上存活**——`rebuild_pick_table` 的空集早退分支仍调用
  `reset_pick_keys`（只重启 key 空间、**不**丢请求），而请求只在 key 空间**真正重启**处丢弃：
  新设备世代（`initialize_with_device`）、`clear_batches`、`rebuild_device`、设备丢失——经新函数
  `reset_pick_keys_and_drop_request`（`renderer_impl.rs:4234`）。

### 机制 2：回退提交后没有重推标志

- `CadRenderRuntime::suppress_overlays`（`crates/cad-engine/src/render/runtime.rs:988`）提交其截断后
  **没有**像 `apply_ready_updates`（`runtime.rs:531` / `:708` 处 `set_entity_flags`）那样重推标志。
- **修复**：新增 `applied_entity_flags`（`runtime.rs:342`）记录**最后一次已应用**的实体标志（在
  `apply_ready_updates` 与 base 提交两处保留），`suppress_overlays` 在截断提交**之后**重推它
  （`runtime.rs:991`）并记录 `flags.unresolved` 报告。**这是记录的镜像、非重新推导**——控制器仍是标志的
  **唯一生产者**（**不是**第二套真相）。重推**未变**集合**不写**标志行（渲染器会 diff），故正常回退进入
  **不花** GPU 上传。`commit_upload` **未**被触碰。

## 已调查但未改代码（记录）

- **`unmatched` 状态已经暴露**：`RenderDiagnostics.result_status`（`crates/cad-engine/src/render/mod.rs:468`）
  经探针 `[probe] pan-wait … status={:?}`（`apps/app-linux/src/host.rs:636`、`:663`）与用户可见的
  `cad_status.error` 暴露 ⇒ **无需**新增诊断码。
- **子元素高亮不退化**：子元素（网格面）高亮**纯**由选择 + 数据库派生，故一次相机 settle **无法**使其
  退化——由测试 fence
  `sub_element_highlight_is_identical_across_a_camera_settle`（`crates/cad-app/src/render_scene/overlay.rs`）
  覆盖（一个相机 settle 用下一个 revision 重准备同一选择/数据库，子元素高亮**逐字节相同**且**不**加
  `highlight.unresolved` 诊断；`highlight.unresolved` 需要数据库**不再**解析某个 ref，即场景变更）。

## 范围划分（诚实，不得模糊）

- 本修复覆盖**高亮着色/overlay**，**不是**选择**状态**。
- **滚轮缩放不能清掉状态**：滚轮缩放映射到 `CommandId::Zoom`（`crates/cad-ui-core/src/session.rs:782`、
  `crates/cad-host/src/input.rs:229`），它只置 `navigation_dirty`（`crates/cad-host/src/input.rs:240`、
  `crates/cad-host/src/lib.rs:790-794`）；只有带 replace 载荷的 `CommandId::Select` 才清选择，而
  `SelectAt{Replace}` **仅在点击**时发出。
- 因此「状态丢失」在机制上**不可达**（滚轮缩放路径），本修复针对的是**高亮的身份请求/着色**。
  用户观察到的现象**是**高亮消失——与机制 1/2 一致。

## 测试（**实测**）

| 套件 | 结果 |
|---|---|
| `crates/cad-render-wgpu/tests/selection_flag_lifecycle.rs` | **3** 测试（新套件） |
| `crates/cad-engine/src/render/contracts.rs`（3 个 `#[ignore]` 硬件 fence） | **3** 测试 |
| `crates/cad-app/src/render_scene/overlay.rs`（子元素 fence） | **1** 测试 |
| `cargo test -p cad-render-wgpu --lib` | **200** |

**renderer 生命周期套件（名称即契约）**：

- `transient_empty_resident_commit_keeps_the_retained_flag_request`（`selection_flag_lifecycle.rs:148`）：
  一次瞬时空驻留提交后，**未**重推任何标志地重传**同一**场景，实体**再次**被着色，像素与着色帧
  **逐字节相同**（修复前**失败**）。
- `explicit_clear_drops_the_retained_flag_request`（`selection_flag_lifecycle.rs:213`）：
  `clear_batches` 后请求被丢弃，无实体被着色（修复前**失败**）。
- `device_generation_change_drops_the_retained_flag_request`（`selection_flag_lifecycle.rs:249`）：
  设备世代变化（`rebuild_device` + `initialize_with_device`）后请求被丢弃，无实体被着色。
- 其中**两个**（`transient_empty_resident_commit_keeps_the_retained_flag_request`、
  `explicit_clear_drops_the_retained_flag_request`）在**修复前代码上失败**——经**临时回退**验证
  （**实测**）。

**引擎契约测试（`#[ignore]`，需真实适配器 P620/Vulkan）**：

- `selection_survives_a_camera_zoom_and_settle`（`contracts.rs:98`）：选中一个 quad、**四个**缩放步 +
  一次 settle；选择状态、**保留请求的解析 key**、像素着色均**不变**，**无** `unresolved` 诊断；
  再清选择后采样像素**必须不同**（证明着色相对未选中场景确实存在）。
- `camera_settle_fallback_reasserts_the_retained_entity_flags`（`contracts.rs:184`）：直接清空渲染器的
  保留请求（模拟瞬时空驻留重置），进入 2D 回退（overlay 输入变化但**无**批产出 ⇒
  `FrameCompleteness::BaseOnlyPendingOverlays`），断言回退**重推**了保留标志（key 恢复），且全场景恢复
  后仍保持。
- `unmatched_selection_reaches_the_probe_status`（`contracts.rs:271`）：一个 `flags.unresolved` 的
  未匹配选择经 `RenderDiagnostics.result_status` 到达探针可见处——**不**发明新真相，它就是
  `RenderSessionHandle::diagnostic` 暴露的同一运行时诊断。

## 发现的预存失败（记录为**既有遗留**，非本轮引入）

两个 `--ignored` 引擎测试：

- `base_only_navigation_renders_latest_camera_and_restores_overlays`
- `pending_overlay_upload_never_labels_stale_suffix_current`

在**纯净 HEAD** 上**同样失败**（用 `git stash` 复现；差异 `left: 3064, right: 3048`）。⇒ **预存缺陷**，
**不是**本 commit 引入。本 commit **未**修这两个用例，**未**对账其数值差异（记录为遗留项）。

## 未测 / 边界（不得省略）

- **视觉判定**：**未**做像素级的**人工**视觉判读；引擎 fence 只做**着色像素**的机器比较（选中 vs 未选中），
  不构成视觉验收。
- **`--ignored` 硬件 fence**：需真实适配器（P620/Vulkan）**显式**运行；**本轮**这些 fence 的**运行记录**
  见上表，但**不得**据此宣称完整原生支持或 vendor/精度结论。
- **人工交互判定**：归**用户**给出；日志/fence 只证机制与像素不变，**不**证用户感知。
- **单机 / 单适配器**：**不**给出 vendor/视觉/性能结论。
- **wasm / 浏览器真实运行时、Windows/macOS/Android**：未测。

## 交叉引用

- 选择标志机、保留请求、增量 pick 表、已知限制：`docs/picking-gpu-id-pass.md`（§0、§5、§8）。
- 相机-only 不是身份变更的相关 fence 与决策：`docs/adr/0012-scene-spatial-index-and-interaction.md`。
- 提交路径 pick 表增量（同轮相邻修复）：`docs/validation-pick-table-incremental-2026-10-17.md`。
- 生产 CPU 拾取接入空间索引（同轮第二项修复）：`docs/validation-pick-index-2026-10-17.md`。
