# 时间驱动分页预算（pacing，N1a）验证（2026-10-17）

**目的**：记录 **N1a —— 时间驱动自适应页预算**落地后的**本机真实宿主实测**与**测试矩阵**。
代码面：新纯控制器 `crates/cad-render-wgpu/src/pacing.rs`（`PacingState` / `Sample` / `PacingMode` /
`step` / `apply_scale`，12 个纯测试）+ 三个分页规划点的接入 + 清屏页**图像先预留**修复 +
`FrameStats` / `[probe] actual-render` 的**追加式**上报字段。

**这不是**性能验收、**不是** GPU 时间测量、**不是**跨平台/vendor/发布版结论，**不是**视觉判定。
单机、单适配器、单会话、同 UID。口径与 `docs/validation-realhost-frame-cost-2026-10-17.md`（**pacing 之前**的
359/479 帧运行）**不同**，两组数字**不得混算**。

## 环境与授权边界

| 项 | 值（**实测**） |
|---|---|
| 机器/会话 | 本机桌面会话，**原生 Wayland**，同一 UID；**无** root/`sudo`、**无**屏录、**无**虚拟/无头显示 |
| 适配器 / 后端 | **Vulkan**，`Quadro P620`，驱动 **580.178.04** |
| 缩放 | `scale=1` |
| 宿主进程 | `target/release/yacr`（本轮**未记录**二进制哈希，故不声称具体 commit 对应关系） |
| 退出/卫生 | 自然 **exit 0**、**无** panic、**无** `is invalid`（无效 buffer）行、**无**残留进程 |
| 计时口径 | 宿主逐帧 `[probe] actual-render …`。**`gpu_ms=None`**（无 `TIMESTAMP_QUERY`）；`wait_ms` 是**阻塞 poll / 呈现的代理，不是 GPU 时间** |
| 证据落盘 | 仅本地（`${YACR_EVIDENCE_ROOT:-/tmp/opencode}` 下本轮输出目录），**未上传** |

## 本轮改了什么

### 控制器（纯逻辑，无 wgpu 类型）

`crates/cad-render-wgpu/src/pacing.rs`。以下常数是**实现值（实测）**，不是估算：

- `target_ms = 13.0`（默认；可由 `Renderer::set_frame_budget_target_ms` 覆盖，非有限/非正数回落默认）；
- 输入信号 = 本帧**实测** `cpu_ms + wait_ms`（决策理由：`frame_total_us`/`present_us` 含预算无法控制的
  固定 apply/present 成本；tick 间隔被 vsync 量化、会来回振荡）；
- EWMA `0.25 * new + 0.75 * old`；**首个样本只播种 EMA**（第三个被观测帧之前不会收缩）；
- 死区 `0.95–1.05 × target`；连续 **2** 帧超目标 ⇒ `×0.75`；连续 **8** 帧低于目标 ⇒ `×1.08`；
- `scale ∈ [0.05, 4.0]`，同时作用于 `max_vertices` 与 `max_triangles`（保持比例），**不动** `max_bytes`；
- 方向翻转清掉反向 streak；**本帧接受 0 个批次**的帧**不是**观测（没画东西，不为页大小付费）。

### 预算语义：上限 + 每帧活动预算

- `Renderer::frame_budget` 现在是**未 pacing 的上限**（ceiling）；`active_frame_budget()` 产出三个分页
  规划点实际使用的**活动预算**。
- **pick/snap 保持不 pacing**（用上限）。
- 三种显式状态：`pacing=active`（渐进 + 有完成信号）、`pacing=at-ceiling`（非渐进帧按上限规划）、
  `pacing=disabled-no-completion-signal`（wasm/无完成信号；预算与静态默认**逐位一致**）。
- **进度下限**：当 cursor 批次**能装进上限**时，一页至少接受 1 个批次；被下限强制的一页报**超预算**、
  原因 `pacing-floor`（诊断码仍是 `render.frame_over_budget`）；只有**单个批次超出上限**才保持既有的显式错误。

### 预存缺陷修复（**先于本轮存在**，本轮修掉）

图像**只在清屏页**绘制（续页 `LoadOp::Load`，避免 alpha 反复累积）。此前图像只按**剩余**页预算记账，
于是当几何在页 1 就饱和时，被扣下的图像在续页**永远不会被画**，而"complete"帧**静默缺图**。
现在清屏页**先预留整幅图像记账**（当 paced 预算装不下全部图像时回落到**上限**），几何再填剩余；
**绘制顺序不变**（图像仍在最后）。

### 上报（追加式，不改既有字段含义）

`FrameStats` 与 `[probe] actual-render` 行在 `pulled={}` **之后**追加
`budget_scale`、`budget_target_ms`、`budget_cost_ema_ms`、`budget_verts`、`budget_tris`、`pacing`。
`over_budget` **字面含义不变**（本帧计划越过了**活动**预算），诊断码仍是 `render.frame_over_budget`；
新字段只做分类。**未新增**诊断码。

## 逐帧证据（原文，中段以 `…` 省略）

`fixtures/complex-test.dwg`（**790** 帧，全部 `pacing=active`；`complete=true` 出现在 seq 1、2、3、171、790）：

```
[probe] actual-render seq=200 … cpu_ms=3.367 … wait_ms=5.631 … over_budget=true prog=Some(2447)/Some(166346) complete=false … pulled=active budget_scale=0.262 budget_target_ms=13.00 budget_cost_ema_ms=8.94 budget_verts=2099799 budget_tris=524950 pacing=active
```

`fixtures/debug-default-fonts.dwg`（**458** 帧，全部 `pacing=active`；`complete=true` **375** 帧 / `false` **83** 帧）：

```
[probe] actual-render seq=200 … cpu_ms=1.102 … wait_ms=2.886 … over_budget=false prog=Some(10548)/Some(10548) complete=true … pulled=active budget_scale=3.567 budget_target_ms=13.00 budget_cost_ema_ms=3.97 budget_verts=28535313 budget_tris=7133828 pacing=active
```

## 两条轨迹（**实测**）

### `complex-test.dwg`：首帧离群 → 快速收缩 → 稳定

- **709 ms** 的首帧离群把 `budget_scale` 从 **1.000 → 0.750 → 0.562 → 0.422 → 0.100**（seq 4–21），
  同时 `wait_ms` 从 **21.0 → 2.3 ms** 落下；
- 之后 `wait_ms` 从 **7.7 → 11.3 ms**（seq 746–750）把 EMA 抬到 **13.87 / 14.67**，`scale` 随之
  **0.336 → 0.252**（seq 751）；
- 随后**稳定下来的一页**达到 `complete=true`。

⇒ 即：控制器在**首个重页**上快速收缩，在**代价回升**时再次收缩，最终把一页压进目标区间并画完。

### `debug-default-fonts.dwg`：慢升到上夹紧

- 一个 **46 ms** 的离群把 `scale` 从 **1.000 → 0.422**；
- 之后**慢升路径**把它抬到上夹紧 **`budget_scale=4.000` / 32,000,000 verts**（到 seq 240）。

⇒ 即：debug 的每帧代价远低于 13 ms 目标，页预算因此**放大到夹紧**——这正是"固定顶点/tri 记账与实测
帧成本脱钩"（N1a 动机）在反方向的直接观测。

## 测试矩阵（**实测**）

| 命令 | 结果 |
|---|---|
| `cargo test -p cad-render-wgpu --lib -- --test-threads=1` | **167 passed / 3 ignored** |
| `cargo test -p cad-render-wgpu --test headless_render` | **26**（含 6 个新 GPU fence，真实 P620） |
| `cargo test -p cad-render-wgpu --test accelerated_tier` | **23** |
| `cargo test -p cad-render-wgpu --test pulled_prefix_equivalence` | **9** |
| `cargo test -p cad-render-wgpu --test render_effects` | **45** |
| `cargo test -p cad-render-wgpu --test pick_id` | **32** |
| `cargo test -p cad-render-wgpu --test pick_hidden_flags` | **8** |
| `cargo test -p cad-engine` | **100 passed / 8 ignored** |

`pacing.rs` 的 12 个纯测试覆盖：播种不收缩、第三帧才允许收缩、两帧超目标收缩 / 八帧低于目标增长、
死区不动作、方向翻转清反向 streak、0 接受不是观测、restart 重播种、夹紧区间、`apply_scale` 保持比例与
字节语义、量化代价轨迹的有界振荡。

`headless_render` 新增的 6 个 fence（名称即契约）：

| fence | 断言 |
|---|---|
| `paced_budget_is_adjustable_per_frame_without_skipping_or_duplicating` | 逐帧可调预算不跳批、不重批 |
| `pick_is_invariant_under_a_tiny_active_page_budget` | pick 不受活动页预算影响（pick 不 pacing） |
| `progress_floor_keeps_the_cursor_monotone_for_ceiling_fitting_batches` | 进度下限使 cursor 单调推进 |
| `paced_pages_keep_images_in_the_settled_frame` | 图像先预留 ⇒ settled 帧**不缺图**（本次缺陷修复的栅栏） |
| `disabled_pacing_uses_the_static_default_budget` | `disabled-no-completion-signal` 与静态默认逐位一致 |
| `scene_mutation_resets_pacing_knowledge` | 场景变更/`clear_batches`/设备代际重置代价知识；**相机移动不重置** |

## 未测 / 边界（不得省略）

- **wasm/浏览器真实运行时**的 disabled 路径**未测**：只有 wasm **库**编译通过；`disabled-no-completion-signal`
  状态是在 **native** 上用测试 override（`set_pacing_signal_override`）断言的，**不是**浏览器实测。
- **Windows / macOS / Android** 未测。
- **跨页 z-order**（图像与后续页几何重叠时）未测：图像**只在清屏页**绘制；上述 fence 只断言**图像完整性**，
  **不**断言与后续页几何的相对叠放正确。
- **视觉/截图像素判定**未做：headless 测试只做**逐字节 parity**；两次宿主运行是**从日志读的**，未做截图判读。
- **GPU 时间**未测：`gpu_ms=None`；`wait_ms` 仍是阻塞 poll/呈现的**代理**。
- **120 Hz / 刷新率自适应**未测：目标是显式 **13 ms 常数**，**不是**由显示器刷新率推导的。
