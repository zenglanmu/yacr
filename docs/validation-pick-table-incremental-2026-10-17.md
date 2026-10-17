# 提交路径 pick 表增量更新 验证（2026-10-17）

**目的**：记录一次**用户报告**的交互缺陷（拖动/缩放时画面冻结、松手后跳变）的**日志证据**、
**根因机制**、**渲染器改动**、**有效性条件**、**真实宿主实测 before/after**、**差分测试**、
**被否决的替代方案**与**未测边界**。改动是**纯 renderer 改动**（`crates/cad-render-wgpu`，
实现 commit `0f5d5c7`：`rebuild_pick_table` 的增量后缀路径、`PickTable` 的惰性身份索引与增量碰撞更新）。
**未**改宿主协议、**未**改 UI、**未**新增诊断码。

**这不是**性能验收、**不是** vendor/视觉结论，也**不是**人工交互判定（见「未测 / 边界」）。单机、
单适配器、单会话、同 UID。口径与 `docs/validation-uniform-encode-skip-2026-10-17.md`（N1e 后续）、
`docs/validation-plan-cost-2026-10-17.md`（N1e 本体）、`docs/validation-gpu-time-2026-10-17.md`（P0）
**不同**，各组数字**不得混算**。选择/pick 表的上游契约见 `docs/picking-gpu-id-pass.md`。

## 环境与授权边界

| 项 | 值（**实测**） |
|---|---|
| 机器/会话 | 本机桌面会话，**原生 Wayland**，同一 UID；**无** root/`sudo`、**无**屏录、**无**虚拟/无头显示 |
| 适配器 / 后端 | **Vulkan**，`Quadro P620` |
| 缩放 | `scale=1` |
| 夹具 | `fixtures/complex-test.dwg` |
| 宿主进程 | `target/release/yacr`（**未记录**二进制哈希，故不声称具体 commit 对应关系） |
| 复现命令 | `./target/release/yacr fixtures/complex-test.dwg`（手动交互），探针 `YACR_PROBE_CAD_LOG=1 YACR_PROBE_GPU_TIME=1` |
| 计时口径 | 宿主 `[probe] cad-stage …`（`input_us`）、`[probe] cad-stages …`（`apply_us`/`frame_total_us`）与提交行的 `commit[…]`（`pick_us`） |
| 证据落盘 | 仅本地（fix lane 记录的捕获路径 `/tmp/opencode/drag-complex.log`），**未上传** |

**证据来源更正（2026-10-17，编排者补记）**：该路径的**修复前**捕获已被用户随后的手动测试**覆盖**
（现内容为修复后数据：`pick` ≤438 µs、`input_us` ≤48 µs），因此下表的"修复前"数字**无法再从该路径复核**——
它们是在捕获时从该文件读出的（编排者与 fix lane 各自读取），现仅存于本轮会话记录。**修复后**的数字仍可复核：
编排者的独立验收日志 `/tmp/opencode/verify-pickfix.log`（`pick` 866/433/541 µs、`input_us` 中位 29 µs），
以及用户修复后那次手动捕获的固定副本 `/tmp/opencode/janky-postpickfix-21h58.log`。

## 报告的症状（用户手动交互，**实测**）

手动交互 `./target/release/yacr fixtures/complex-test.dwg` 时：

- **拖动（pan）时画面冻结**——拖动过程中画面**不更新**；松手后画面**跳到新位置**。缩放（zoom）表现相同。

fix lane 用 `YACR_PROBE_CAD_LOG=1 YACR_PROBE_GPU_TIME=1` 捕获的日志行（fix lane 记录，`/tmp/opencode/drag-complex.log`）：

- `[probe] cad-stage queued=4 input_us=1047533 tick_us=3`——输入**排队约 1.05 s**；
- `[probe] cad-stages apply_us=3368276 … frame_total_us=3805231`——**一帧 3.8 s**；
- `commit[rebase=0 append=97 gc=9424 pulled=580 pick=1449615 lineweight=1325 total=1466543]`
  ——**`pick` = 1.45 s**。

**同一夹具在近期 perf 轮次之前**的日志显示 `pick=1530380` µs（**实测**，fix lane 记录；同一数值在
`/tmp/opencode/yacr-gputime-20261017T094238Z/complex-test.on.log:5152` 可复现）⇒ 这是**预存缺陷**
（pre-existing），**不是**回归；近期轮次只是把它旁边**原本就有的** `pulled` 1.95 s 消掉了。

## 机制（`crates/cad-render-wgpu`）

### 改动前

`Renderer::rebuild_pick_table` 在**每次**批次/图像变更（commit/clear/设备丢失）时**整表重建**：把
**全部**驻留批次与图像重新映射成 `(key → 完整 SelectionRef)`（`plan.rs` 的 `PickTable::build`），
并重建截断碰撞集合与身份索引。对 `complex-test.dwg` 这类 ~16 万批的图，**一次后缀提交**（只新增一条
overlay 后缀批次）也要付**整幅**的重建成本——这就是 `pick` ≈ 1.45 s 的来源。

### 改动后（commit `0f5d5c7`）

`rebuild_pick_table` 现在**在满足条件时走增量后缀路径**（`renderer_impl.rs:4082`）：

- **条件**（`renderer_impl.rs:4101-4103`）：`keep_prefix > 0` **且** **无驻留图像**
  （`self.images.is_empty()`）**且** `pick_table_covers_prefix(keep_prefix)` 为真。
  `pick_table_covers_prefix`（`renderer_impl.rs:4152`）是 **O(1)** 的覆盖证明：只需检查被保留前缀的
  最后一个批次的 key 在合并索引 `prefix - 1` 处**确实存在**。
- **增量动作**：`PickTable::replace_suffix(keep_prefix, retired_keys, added)`（`plan.rs:3672`）——**退役**
  被丢弃后缀的 key，**只追加**新条目（`renderer_impl.rs:4105-4113`）。计数 `pick_table_incremental_extends`
  （`renderer_impl.rs:4116`；字段见 `lib.rs:1597`）。
- **回退到整表重建**：其余一切——**base 变更**（`keep_prefix == 0`）、**图像重传**（`self.images` 非空）、
  设备丢失 / `clear_batches` 复位、前缀未被覆盖——都回退整表重建（`renderer_impl.rs:4118-4130`），计数
  `pick_table_full_rebuilds`（`renderer_impl.rs:4118`；字段见 `lib.rs:1595`）。二者互补。

**为什么增量等价于整表重建（有效性论证）**：pick key 在**一个设备世代内单调分配、永不重用、永不重编号**
（`pick_key_counter` 只在 `reset_pick_keys` 重启，`renderer_impl.rs:4164`，而它**恰在批次被清空时**发生）。
因此被保留前缀之外的条目**不变**，且追加的 key **严格大于**所有既有 key。⇒ 增量结果与对驻留集做一次
`PickTable::build` **完全一致**（条目映射、`keys_for_source` 答案、截断碰撞集合、`max_key` 都一致）。

### 同轮的两处配套改动

- **`PickTable::build` 现在消耗条目**（`plan.rs:3608`）：把 `sources` **移动**进表，而不是再克隆一次
  （此前是一次克隆进参数、一次克隆进表 = 两次；现在渲染器只克隆每个驻留批次的 `sources` **一次**）。
  渲染器仍持有自己的副本，故**不可能**出现自引用借用。
- **身份索引惰性构建**：`source_index` 现在是 `Option<SourceIdentityIndex>`（`plan.rs:3582`；类型别名
  `plan.rs:3556`），**仅当** `entity_flags_request` 非空时才构建（`renderer_impl.rs:4090` 的 `want_index`，
  并在 `apply_entity_flags` 的**首次**查询前经 `ensure_source_index`（`plan.rs:3791`）补齐，
  `renderer_impl.rs:1322-1328`）。一个**从不**发出实体标志请求的场景**永不**为它付费。
  `replace_suffix` **保留**已建索引的精确性，未建的**仍不建**。
- **截断碰撞检测改为增量**：`detect_truncation_collisions` 被**保留的** `truncated_index`
  （`plan.rs:3589`）+ 增量 `update_collisions`（`plan.rs:3905`）取代——`replace_suffix` 无需重扫每个驻留源
  即可保持 `EntityId` 截断碰撞检测**精确**（一次 fresh `build` 派生同一索引，故二者按构造一致）。

## 实测（本机真实宿主）

### before → after

| metric | before | after |
|---|---|---|
| 后缀提交 `pick` | **1,449,615 / 1,487,525 / 1,454,073** µs（7 次提交） | **866 / 433 / 541** µs |
| base-load `pick` | 1,310,440 µs | 1,021,559 µs |
| `input_us`（排队延迟） | n=18 中位 **1,045,445** µs，p90 1,069,482，max 2,092,380 | n=1185 中位 **29** µs，p90 41，max 116 |
| `cad-frame presented` | 拖动期间**冻结** | 2051–2054 行，**连续** |

- 后缀提交 `pick` 由 **~1.45 s 降到 ~0.4–0.9 ms**（**实测**）。排队延迟 `input_us` 由中位 **~1.05 s**
  降到 **29 µs**（**实测**）。
- **独立 orchestrator 复跑**（另一次宿主运行，同一夹具，**实测**）：后缀提交 `pick=866/433/541` µs、
  `input_us` 中位 29 µs（n=1185，p90 41，max 116）、`cad-frame presented` **2054** 行、exit 0。

### 测试矩阵（**实测**）

| 命令 | 结果 |
|---|---|
| `cargo test -p cad-render-wgpu --lib` | **200 passed / 3 ignored**（含差分测试，见下） |
| `cargo test -p cad-render-wgpu --test pick_id` | **32** |
| `cargo test -p cad-render-wgpu --test pick_hidden_flags` | **8** |
| `cargo test -p cad-render-wgpu --test headless_render` | **35** |
| `cargo test -p cad-render-wgpu --test accelerated_tier` | **23** |
| `cargo test -p cad-render-wgpu --test pulled_prefix_equivalence` | **9** |
| `cargo test -p cad-render-wgpu --test render_effects` | **45** |

`fmt` / `check --all-targets` / `clippy -D warnings` / wasm check **clean**；release 构建 **OK**（**实测**）。

### 差分测试（名称即契约）

`incremental_pick_table_matches_a_fresh_build_across_commit_sequences`（`plan.rs:5143`）——
**128 个种子 × 40 步**的 append / suffix-replace / image re-upload / reset 序列，**每一步之后**断言增量
`PickTable` 与一次 fresh `build` **完全相等**：条目、碰撞、`max_key`、`truncated_index`、`source_index`
全部一致；并含事务式的**重复 key 拒绝**用例。

## 三处更正与已否决的替代方案（必须记录；均曾被提出并被证伪）

1. **`append=97` 是微秒，不是 97 个批次**。格式化点是 `apps/app-linux/src/host.rs:2086-2093`
   （`commit[rebase={} append={} …]` = `rebase_us, append_us, …`）。一次后缀提交的**真实**追加量是
   **一个**批次。
2. ⇒ 因此被提出的**「当每个追加批次都是 `PICK_KEY_NONE` 时跳过重建」**路径**没有生产者**：pick key
   总是在 `1..=MAX_PICK_KEYS` 内分配，`PICK_KEY_NONE`（0）**从不**进入表，该跳过分支**永远不会触发**。
3. **pick 时的 dirty/惰性重建：已否决**（不只是延后）。理由：增量修复后，急切成本已是
   **O(退役 + 追加)** ≈ **0.4–0.9 ms**；带保留标志请求时 `apply_entity_flags` **必须**在提交时运行
   （`set_entity_flags` 立即把 `EntityFlagReport.unmatched` 返回给运行时，`cad-engine/src/render/runtime.rs:699`）；
   `pick_collisions` / `pick_table_len` / `pick_key_for_batch` 是 `&self` 访问器，**没有**内部可变性就**无法**
   惰性构建；且新的 dirty 标志会成为**第二套失效真相**（**铁律 4**）。

## 为什么交互期间提交会重复（记录该发现）

生产者是**延迟的参考网格刷新**（deferred reference-grid refresh）：

- 一次**仅相机**的视口范围变化会**布防**一次 settle（`cad-engine/src/render/mod.rs:703-706` 的
  `arm_camera_settle`）；
- 一旦相机**静默**满 **250 ms** 的 settle 超时（`mod.rs:309-323` 的 `settle_camera_if_due`，
  常量 `mod.rs:165` 的 `CAMERA_SETTLE_TIMEOUT`），overlay 后缀被**重新准备**
  （`cad-app/src/render_scene/overlay.rs:452` 的 `grid_overlay`）并以**后缀提交**提交
  （`cad-engine/src/render/runtime.rs:656-663`，`base_changed=false`）。

⇒ **相机变化不是每帧提交，而是每次 settle 提交一次**。原始运行的 **7 次 settle** 被 **1.5–3.8 s 的提交本身
放大**（每个被阻塞的 pump 看起来都像一段新的静默期）；修复后 **3 个手势产生 3 次后缀提交**且画面**连续呈现**。
**完全避免 settle 提交**需要**与相机无关的网格**（无限 shader 网格）或**大得多的预扩展网格**——两者都是
**设计改动**；记录为**刻意「保持现状」（leave as is）**并附此理由。

## 仍开放项（本轮**未**修）

- **base 变更后的第一次后缀提交**仍付**整幅 pulled 重建**（`pulled=2,506,646` µs，`pulled_reuses=0`）；
  后续后缀提交复用（`pulled=590/731` µs）。
- **base load 本身**耗 `pulled=2,518,896` + `pick=1,021,559` µs（**一次性**）。

以上均**不是** pick 表的问题（是 pulled 重建与 base load 的一次性成本）。

## 未测 / 边界（不得省略）

- **不得从日志得出人工交互判定**：验收用的是**合成 DOM 手势驱动**（`YACR_PROBE_POSTLOAD_PAN`），
  它**明确不是**真实鼠标；日志只证明**提交路径便宜**且**帧持续呈现**，**不**证明人感知的拖动是流畅的。
  **人工判定归用户给出**。
- **`debug-default-fonts.dwg` 的相机路径未在本 fix lane 测量**（pan 驱动两次报告
  「concurrent resize during the pan invalidated the report」）；只测到其 base-load 提交
  （`pick=4675` µs）与 exit 0。
- **视觉 / 截图判定未做**：本轮只有日志（提交计时与 `input_us`），**无**像素读回。
- **单机 / 单适配器**：**不**给出 vendor/视觉/性能结论。
- **wasm / 浏览器真实运行时、Windows/macOS/Android**：未测。

## 交叉引用

- 选择/pick 表上游契约与增量更新落点：`docs/picking-gpu-id-pass.md`（§5）。
- 决策与优先级：`docs/render-optimization-backlog.md` §N1f（提交路径 pick 表增量更新）。
- 同轮相关：`docs/validation-uniform-encode-skip-2026-10-17.md`（N1e 后续；同一种「每帧/每提交 O(驻留)」
  的病）、`docs/validation-plan-cost-2026-10-17.md`（N1e 本体）。
