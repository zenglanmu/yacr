# 后缀提交不再重置 pacing 成本模型 验证（2026-10-17）

**目的**：记录一次**用户报告**的交互缺陷（**全览拖动卡顿**、而**放大看细节反而顺滑**）的
**症状**、**根因机制**、**渲染器改动**、**真实宿主实测 before/after**、**独立复跑**、**测试矩阵**、
**`apply_us` 字段澄清**与**未测边界**。改动是**纯 renderer 改动**（`crates/cad-render-wgpu`，
实现 commit `0fb2bc1`：`Renderer::commit_upload` 不再在后缀提交上重置自适应页预算控制器）。
**未**改宿主协议、**未**改 UI、**未**新增诊断码。

**这不是**性能验收、**不是** vendor/视觉结论，也**不是**人工交互判定（见「未测 / 边界」）。单机、
单适配器、单会话、同 UID。口径与 `docs/validation-frame-pacing-2026-10-17.md`（N1a 本体）、
`docs/validation-pick-table-incremental-2026-10-17.md`（N1f）、
`docs/validation-uniform-encode-skip-2026-10-17.md`（N1e 后续）、
`docs/validation-gpu-time-2026-10-17.md`（P0）**不同**，各组数字**不得混算**。

## 环境与授权边界

| 项 | 值（**实测**） |
|---|---|
| 机器/会话 | 本机桌面会话，**原生 Wayland**，同一 UID；**无** root/`sudo`、**无**屏录、**无**虚拟/无头显示 |
| 适配器 / 后端 | **Vulkan**，`Quadro P620` |
| 缩放 | `scale=1` |
| 夹具 | `fixtures/complex-test.dwg` |
| 宿主进程 | `target/release/yacr`（**未记录**二进制哈希，故不声称具体 commit 对应关系） |
| 驱动 | 合成 DOM 手势 `YACR_PROBE_POSTLOAD_PAN`（**明确不是**真实鼠标；见「未测 / 边界」） |
| 计时口径 | `[probe] actual-render` 行的 `budget_scale` / `budget_cost_ema_ms` / `budget_verts` / `verts` / `draw_calls` / `gpu_ms` / `wait_ms` / `cpu_ms` / `over_budget` / `pacing` |
| 证据落盘 | 仅本地（`/tmp/opencode/janky-postpickfix-21h58.log` 等），**未上传** |

**证据来源说明**：**修复前**的分布取自用户的一次**手动捕获**的**固定副本**
`/tmp/opencode/janky-postpickfix-21h58.log`（该文件名带 "postpickfix"，指**同一会话中更早的 pick 表
修复之后**，与本轮 pacing 改动**无关**；它在本轮改动**之前**捕获）。**修复后**为**探针运行**。
二者**口径不可比**（见「混淆项」）。

## 报告的症状（用户手动交互，**实测**）

- **全览（full-view）拖动时卡顿**；而**放大到局部看细节反而顺滑**。用户自己的报告即此现象。

**为什么这正好指向本缺陷**：全览帧画的是**整幅图的 draw-order 前缀页**（无剔除），每页顶点重；
放大后**块级剔除**（backlog §P1）把视口外工作消掉，帧就变便宜——于是"放大顺滑、全览卡"的分裂现象
正是"全览页过重"的签名。

## 机制（`crates/cad-render-wgpu`）

### 改动前

`Renderer::commit_upload` **在每一次提交**上都重置自适应页预算控制器——**包括**引擎在**每次相机
settle** 发出的**后缀专用 overlay 提交**：

- **生产者**是**延迟参考网格刷新**（deferred reference-grid refresh）：仅相机的范围变化布防一次
  settle，相机静默满 **250 ms** 后 overlay 后缀被重准备并以**后缀提交**提交
  （`cad-engine/src/render/mod.rs:309-323` 的 `settle_camera_if_due`，超时常量 `mod.rs:165`；
  见 `docs/validation-pick-table-incremental-2026-10-17.md`「为什么交互期间提交会重复」）。
  该后缀提交**只追加一条 overlay 批次**（`keep_prefix = 驻留基座批次数`）。

⇒ 连续交互期间，**每次 settle 都把已学到的 `scale` 打回 `1.0`**，控制器**永不收敛**，渐进页**始终
顶点过重**。全览帧的代价因此居高不下（**实测**见下：`draw_calls≈2720`、`gpu_ms 15.80`，≈20 ms/帧
≈ 47 fps）。而放大后**剔除**把工作消掉，所以**细节缩放顺滑、全览拖动卡顿**。

### 改动后（commit `0fb2bc1`）

`Renderer::commit_upload` 只在**基座变更**时重置（`renderer_impl.rs:2284-2303`）：

```rust
if keep_prefix == 0 {
    self.reset_pacing();
}
```

- **判据**：`keep_prefix == 0`——**正是** pick 表增量路径（N1f）所用的**同一个判别式**
  （`keep_prefix > 0` 表示保留了真实前缀 = 后缀提交）。
- **仍然重置的集合**：**基座变更**（`keep_prefix == 0`）、`clear_batches`、**新设备世代**、
  **设备丢失**、以及**图像重传**（`reset_pacing` 也被这些路径调用；图像重传保留重置，因为它替换
  **整幅驻留图像集**、**从不**是增量后缀）。
- **`reset_pacing` 只清成本模型状态**（`renderer_impl.rs:2776-2779`）：用
  `pacing::PacingState::new(target_ms)` 重建控制器——即清掉 `scale` / `cost_ema_ms` /
  `over_under_streak` / `seeded` 等**学习状态**。
- **restart-pending 是另一路信号，对后缀提交被保留**：后缀提交**不调用** `reset_pacing`，
  故早先一次**真实重置**置起的 restart-pending 仍**正确保持 pending**。
- **未引入 dirty/invalidation 标志**：源码注释明确这是**启发式成本模型**、**不是**正确性状态，
  故**刻意不**建模为失效标志（**铁律 4**：不新增第二套失效真相）。重置**不与任何其它状态耦合**。

## 实测（本机真实宿主）

### `budget_scale` 分布（before → after）

| 运行 | 帧数 | `=1.000` | `<1.0` | `>1.0` | EMA 重置 |
|---|---:|---:|---:|---:|---|
| **before**（用户手动捕获固定副本 `janky-postpickfix-21h58.log`） | 75 | **35（47%，撞顶）** | 40 | 0 | —（反复重学：`budget_cost_ema_ms=0.00` 出现在 15 帧） |
| **after**（探针运行） | 1888 | **6（仅前 6 帧）** | **1758（93.1%）** | **125（6.6%）** | **1**（仅初始） |
| **独立复跑**（编排者，另一次宿主运行） | 1842 | 6 | **1808** | 28 | **2**（初始 + base 变更） |

- before 的 `budget_scale` 具体取值：`1.000×35, 0.750×18, 0.562×14, 0.422×7, 0.316×1`。
- after：`1.0` 仅出现在**前 6 帧**（初始播种阶段），此后 **93.1%** 的帧 `<1.0`、**6.6%** `>1.0`。
  `>1.0` 是**合法**的：合成 pan 把图拖进**空白区**，廉价相位里控制器**合理地爬回 1.0 以上**去寻
  13 ms 目标（控制器设计即如此，见 §N1a）。
- after 的 EMA **只重置一次**（初始），**不是**反复重学；独立复跑中 EMA 重置 **2** 次
  （初始 + 一次 base 变更），两腿都 **`pacing=active` 全帧**。

### 后缀提交逐次保持已学 `scale`（**实测**）

| 后缀提交 | `budget_scale` before → after |
|---|---|
| 1 | **0.435 → 0.435** |
| 2 | **0.485 → 0.485** |
| 3 | **0.471 → 0.471** |

三次真实后缀提交都**逐字（verbatim）保持**已学 `scale`，即"一次 settle **不是**一次成本剖面变化"。

### before 的全览帧签名（**实测**）

| 字段 | 值 |
|---|---|
| 帧数（`actual-render`） | 75 |
| `budget_verts` 中位 | **6,000,000** |
| `verts` 中位 | **4,061,234** |
| full-view 帧 `draw_calls` | **≈2720** |
| full-view 帧 `gpu_ms` | **15.80** |
| full-view 帧 `wait_ms` | **16.15** |
| full-view 帧 `cpu_ms` | **4.96** |
| `over_budget=true` | **69/75** |

**读法**：`budget_verts` 中位**恰好**是静态默认上限（6,000,000 = 8,000,000 × 0.75 的整数倍级、
即未收敛的整档），`gpu_ms` 15.80 与 `wait_ms` 16.15 都**越过 60 fps 预算**——这就是"全览页过重"的
直接证据。

## 测试矩阵（**实测**）

| 命令 | 结果 |
|---|---|
| `cargo test -p cad-render-wgpu --lib` | **200 passed / 3 ignored** |
| `cargo test -p cad-render-wgpu --test pacing_commit_suffix` | **3 passed**（新套件，见下） |
| `cargo test -p cad-render-wgpu --test headless_render` | **35** |
| `cargo test -p cad-render-wgpu --test accelerated_tier` | **23** |
| `cargo test -p cad-render-wgpu --test pulled_prefix_equivalence` | **9** |
| `cargo test -p cad-render-wgpu --test pick_id` | **32** |
| `cargo test -p cad-render-wgpu --test pick_hidden_flags` | **8** |
| `cargo test -p cad-render-wgpu --test render_effects` | **45** |

`fmt` / `check --all-targets` / `clippy -D warnings` / wasm check **clean**；release 构建 **OK**（**实测**）。

### 新套件（名称即契约，`crates/cad-render-wgpu/tests/pacing_commit_suffix.rs`）

- `suffix_only_commit_preserves_pacing_knowledge_while_a_base_change_resets_it`——后缀提交**逐位
  （bit-identically）**保持 `pacing_scale()` / `pacing_cost_ema_ms()`；基座变更仍重置。
- `repeated_suffix_commits_do_not_return_the_scale_to_one`——**六轮**后缀提交**从不**把 `scale`
  打回 `1.0`。
- `suffix_commit_after_an_over_target_observation_keeps_the_shrunk_budget`——一次**超目标观测**之后
  的后缀提交保持 `budget_scale < 1.0` 且 `budget_vertices` 低于上限。

## 必须披露：一个既有 fixture 被修改

`headless_render::scene_mutation_resets_pacing_knowledge`（`crates/cad-render-wgpu/tests/headless_render.rs:603`）
原以 `renderer.upload(…)` 作为"场景变更"。但 `upload` 的提交是 `keep_prefix = self.batches.len()`——
即一次**后缀追加提交**，**不是**基座变更——因此在**修正后的规则**下它**不再**触发重置。

该 fixture 现改为执行一次**真实基座变更**（`prepare_upload_batches` + `commit_upload(staged, 0)`）。
其**契约不变**（"一次**基座变更**的场景变更重置成本知识"），只是用来触发它的动作从"伪基座变更"换成
了真正的基座变更。**这是本轮唯一被修改的既有测试**。

## 字段澄清（只读发现，记录在此供查探针字段者）

**`cad-stages apply_us` 不是 GPU draw，也不是交互帧渲染成本**。它是
`RenderSession::apply_ready_updates` 的**墙钟**时间
（`apps/app-linux/src/gtk_bridge.rs:926-932` → `crates/cad-engine/src/render/mod.rs:1137` →
`crates/cad-engine/src/render/runtime.rs:494`），即**有界分块的场景增量 CPU 暂存**：
每次调用 **≤4 个 slice 或 ≤8 ms**（`runtime.rs:576-612`），主要由 `prepare_upload_batches` 主导。

- 它**不是** GPU draw，**不是**交互帧的渲染成本。
- 用户在**同一捕获**中看到的 `apply_us` 中位 **~9.9 ms** 是 `complex-test.dwg` 的**一次性批量应用**
  （**166k 批次**在 ~10 ms 块中暂存），该行 `presented=false`、`render_us=1`——即"这一帧没有真正呈现
  图像，只是在暂存"，**不是**逐帧渲染开销。

## 混淆项（不得软化）

- **before/after 的每帧 GPU 数字不可比**：baseline 窗口是**最大化**的，而探针运行用的是
  **926×266** 窗口（被 `YACR_PROBE_NO_MAXIMIZE=1` 强制）；且驱动是**合成 DOM 手势，不是真实鼠标**。
  ⇒ **不得**给出任何"倍数下降"的结论，也**不得**给出**人工交互判定**——**该判定归用户**。
- **单机 / 单适配器**：**不**给出 vendor/视觉/性能结论。

## 未测 / 边界（不得省略）

- **人工交互判定归用户**：验收用**合成 DOM 手势驱动**（`YACR_PROBE_POSTLOAD_PAN`，**明确不是**
  真实鼠标）；日志只证明**控制器收敛**且**后缀提交保持已学 `scale`**，**不**证明人感知的拖动变流畅。
- **视觉 / 截图判定未做**：本轮只有日志（`budget_scale`/`budget_verts`/`gpu_ms` 等），**无**像素读回。
- **单机 / 单适配器**，**不外推**到其它 vendor / 更大图纸 / 浏览器；相机只做平移，**zoom 腿未跑**。
- **wasm / 浏览器真实运行时、Windows/macOS/Android**：未测。

## 交叉引用

- 决策与优先级：`docs/render-optimization-backlog.md` §N1a 后续（后缀提交保持 pacing 成本模型）。
- 同源机制：`docs/validation-frame-pacing-2026-10-17.md`（N1a 本体）、
  `docs/validation-pick-table-incremental-2026-10-17.md`（N1f；后缀提交的生产者与判别式）。
- 字段口径：`docs/render-3d.md`「分页预算与每帧活动预算」节；探针字段描述另见
  `docs/validation-complex-pan-2026-10-16.md`（`cad-stages` 括号定义）。
