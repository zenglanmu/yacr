# 每帧 CPU plan 成本：剔除感知的分段页规划 验证（2026-10-17）

**目的**：记录 `docs/render-optimization-backlog.md` §N1e（P0 结论工作顺序的 **#1「每帧 CPU plan 成本」**）
落地后的**本机真实宿主实测**、**算法说明**与**测试矩阵**。本轮是**纯 renderer 改动**
（`crates/cad-render-wgpu`），**未**改宿主协议、**未**改 UI、**未**新增诊断码。

**这不是**性能验收、**不是** vendor/视觉结论。单机、单适配器、单会话、同 UID。口径与
`docs/validation-gpu-time-2026-10-17.md`、`docs/validation-chunk-culling-2026-10-17.md`、
`docs/validation-frame-pacing-2026-10-17.md`、`docs/validation-realhost-frame-cost-2026-10-17.md`
**不同**，各组数字**不得混算**。

## 环境与授权边界

| 项 | 值（**实测**） |
|---|---|
| 机器/会话 | 本机桌面会话，**原生 Wayland**，同一 UID；**无** root/`sudo`、**无**屏录、**无**虚拟/无头显示 |
| 适配器 / 后端 | **Vulkan**，`Quadro P620` |
| 缩放 | `scale=1` |
| 宿主进程 | `target/release/yacr`（本轮**未记录**二进制哈希，故不声称具体 commit 对应关系） |
| 退出/卫生 | 自然 **exit 0**、**无** panic、**无**残留进程 |
| 计时口径 | 宿主逐帧 `[probe] actual-render …` 的 `plan_ms` / `cpu_ms` 字段 |
| 证据落盘 | 仅本地（`${YACR_EVIDENCE_ROOT:-/tmp/opencode}` 下本轮输出目录），**未上传** |

## 病根（实测）

`plan_ordered_page` 本应在首个越预算批处停下并保留 cursor。但**完全被剔除的批计零**顶点/三角形，而产品里
`max_bytes` 为 `usize::MAX`（见 `docs/render-optimization-backlog.md` 文首），故零计费批**永不失败**：当剔除
把视图清空，线性游走会走到**有序表末尾**（`complex-test.dwg` 为 **166,346** 条），为每一条算顶点/三角形成本，
只为接受全部并画一个 run。实测该病态页 `plan_ms` 中位 **10.5** / p90 **18.1**、`cpu_ms` **22.4**。

## 算法（`crates/cad-render-wgpu/src/plan.rs`）

1. **批集事实 `BatchFacts`（剔除无关，每批集变更一次）** —— `plan.rs:896-995`：
   - 按类（`[instance, mesh, arc]`）× chunk 索引的 batch 列表 `chunk_batches`；
   - `always_active`：**无论剔除都计费**的批——有 legacy line quad（`line_quad_count > 0`）的批，或无 run 的
     legacy mesh（`mesh_runs` 空且 `RenderTopology::Mesh`）；
   - 类标志 `has_line_class` / `has_mesh_class`、`converted_total`、紧凑 `upload_bytes`（避免逐帧 DRAM 未命中
     ~200 B 的 `GpuBatch`）。
   - **只**在 `draw_plan = None` 失效时重建（`renderer_impl.rs:4364-4377`），**不新增**失效真相。
2. **有序页索引 `OrderedPageIndex`（每次 `ordered` 重建）** —— `plan.rs:997-1047`：批→位置逆表 `batch_pos`
   （不在有序集内为 `u32::MAX`）+ `bytes_prefix`（`ordered` 的字节前缀和）。在**既有重启清扫**里随 `ordered`
   重建（`renderer_impl.rs:4409`），跨重启复用分配。
3. **分段游走 `plan_ordered_page_segmented`** —— `plan.rs:1080-…`：
   - **字节边界**：对 `ordered` 做前缀和**二分查找**，找首个满足
     `initial.bytes + (prefix[p+1] - prefix[cursor]) > max_bytes` 的位置 `p`（前缀和非减 ⇒ 谓词单调，
     `plan.rs:1100-1119`）；`max_bytes == usize::MAX` 时边界为 +∞。**空隙仍计字节**（字节计费按批粒度、与剔除
     无关）。
   - **活动位置**：可见 chunk 的批 ∪ `always_active`，按 `batch_pos` 排序去重，剔除已画（`pos < cursor`）与
     不在 `ordered` 的批；**总是**评估 cursor 批（图像预留可能已超预算）。
   - **顶点/三角形边界**：只对活动批累积，首个越界即止（`plan.rs:1154-1187`）；若字节失败位置在前，则字节
     类别优先（`charge_bytes` 先跑）。
   - **精确**：空隙（零计费批）无条件被接受，故由活动批的**空隙**即可导出被接受前缀。
4. **回退 `plan_ordered_page_linear`（不变）** —— `plan.rs:792-836`：`!cull.any_culled()` 或
   `estimated_active(facts, cull) > SEGMENTED_ACTIVE_THRESHOLD` 时走原线性游走。阈值 **4096**
   （`plan.rs:894`），覆盖所有剔除后可见集很小的页（稳定 1-draw 页 `m` 为个位数）。
5. **统一 `usage` 契约**：`plan_ordered_page` 与 `plan_from_gpu` 失败时共用一条——顶点/三角形失败**保留失败批的
   字节**（先计字节后查顶点），字节失败**不保留**（`plan.rs:671-679`）。该字段**不**出现在 probe 输出里。

**未触碰**：`plan_from_gpu` / `plan_cursor_batch` 函数体、进度下限、draw-order 切分、图像预留/上限回落、
`ChunkCull` 公式。

## 实测（本机真实宿主）

### before → after（中位 / p90）

| fixture | 页 | `plan_ms` before → after（中位 / p90） | `cpu_ms` before → after（中位 / p90） |
|---|---|---|---|
| complex-test | 稳定 1-draw（`cull=1/9362`） | 10.747 / 18.084 → **1.221 / 2.749** | 22.42 / 30.78 → **13.65 / 21.94** |
| complex-test | 全部帧 | 2.075 / 4.349 → 0.923 / 1.416 | 4.111 / 8.542 → 3.260 / 7.921 |
| debug-default-fonts | 稳定 1-draw（`cull=1/806`） | 0.677 / 0.759 → **0.114 / 0.213** | 1.244 / 1.367 → 1.144 / 1.829 |

> **paced 帧数字被混淆**：自适应 pacing 控制器轨迹依赖墙钟，不同运行走不同 `budget_scale`/接受路径。
> **只有**稳定 1-draw 页是**预算无关**的（一切零计费 ⇒ 该页接受全部），因此是干净的 before/after；表中
> "全部帧"行与任何 paced 数字都必须带此声明。

### 独立 orchestrator 复跑（另一次宿主运行，同一 fixture）

| 观察 | before | after |
|---|---|---|
| 接受**整条**有序表的帧（`prog=Some(166346)/Some(166346)`）`plan_ms` 中位 | **10.547**（n=47） | **1.168**（n=5） |
| 深剔除帧（kept<100 / 9362）`plan_ms` 中位 | **10.541** | **0.588** |

两次运行都出现**同一病态页**（`draw_calls=1 verts=12 cull=1/9362`），现 `plan_ms=1.168`。

## 测试矩阵（**实测**）

| 命令 | 结果 |
|---|---|
| `cargo test -p cad-render-wgpu --lib` | **192 passed / 3 ignored**（含 **7** 个新 planner 测试） |
| `cargo test -p cad-render-wgpu --test headless_render` | **35** |
| `cargo test -p cad-render-wgpu --test accelerated_tier` | **23** |
| `cargo test -p cad-render-wgpu --test pulled_prefix_equivalence` | **9** |
| `cargo test -p cad-render-wgpu --test render_effects` | **45** |
| `cargo test -p cad-render-wgpu --test pick_id` | **32** |

### 差分测试（输入空间与它 pin 住什么）

`segmented_page_planner_matches_the_linear_reference`（`plan.rs:5129-5202`）对 `#[cfg(test)]` 的**旧规划器
参照** `plan_ordered_page_reference`（`plan.rs:838-887`）做 **4000 例随机差分**：

- **输入空间**：随机批（每批 1–4 个 instance stream / 1–4 个 mesh chunk、随机 `line_quad_count`、随机
  instanced/mesh/arc run、随机 topology、随机 upload bytes）；`ordered` 为批索引的随机置换（有时**丢掉**一项
  以建模不可见批）；随机 cursor；随机剔除掩码（**强制至少剔除一个 chunk** 以走分段路径而非回退）；
  `max_bytes` ∈ {`usize::MAX`, 0, <400, <4000}；`max_vertices` / `max_triangles` ∈ {`usize::MAX`, 0, 小, 中}；
  随机 `initial` 预留（含**已超预算**的图像预留情形）。
- **pin 住**：被接受前缀 `accepted`、`usage.vertices`、`usage.triangles`、**统一** `usage.bytes`（由参照 +
  失败批字节推导）、`over_budget` 的完整 `report`（类别 / requested / limit / skipped）与其**存在性**。
- 配套性质测试：`fallback_path_matches_the_reference`（200 例，恒等掩码 ⇒ 线性回退路径）；
  `accepted_with_work_sequence_and_pacing_trajectory_are_identical`（300 例，逐页 `accepted_with_work` 序列与
  `pacing::step` 的 `scale` / `cost_ema_ms` 轨迹一致）。
- 定点测试：`fully_culled_batches_are_accepted_and_line_quad_batches_stay_active`（`always_active` **不是**
  `batch_has_visible_work`）、`progressive_cursor_advances_monotonically_to_completion`、
  `byte_category_wins_when_bytes_and_vertices_cross_together`、
  `both_planners_share_the_failing_batch_bytes_usage_contract`。

## 未测 / 边界（不得省略）

- **complex-test 的 `plan_ms` p90**（跨运行 2.53–2.75）**正落在 2.5 ms 边界上**：中位目标达成，p90 目标只是
  勉强。
- **同一种病的剩余循环**：稳定帧仍付**重启清扫**（每次相机移动 O(166,346)）与 `uniform_ms`（~6 ms）、
  `encode_ms`（~5.2 ms）、`submit_ms`（~5 ms）——稳定帧 `cpu_ms` 因此是 13.65 ms 而非 ~1 ms。**本轮未修**，
  是显式后续项（见 backlog §N1e「后续」）。
- **逐帧渐进重启**是 **probe/证据驱动重新发出视图变更**（每稳定帧前一次合成 `pointer_move` + `view_rev`
  同步递增），**不是**相机 f32 抖动；未修复，也不应修。
- **视觉 / 截图判定未做**：本轮只有日志（`plan_ms` / `cpu_ms`），**无**像素读回。
- **GPU 时间未测**：本轮不涉及 `gpu_ms`（P0 是独立 opt-in，见 `docs/validation-gpu-time-2026-10-17.md`）。
- **wasm / 浏览器真实运行时、Windows/macOS/Android**：未测。
- **vendor / golden / 性能结论**：单机、单适配器，**不得**外推。

## 交叉引用

- 决策与优先级：`docs/render-optimization-backlog.md` §N1e（及 §N1a / §N1d / §N2 / §N5）。
- 上游测量：`docs/validation-gpu-time-2026-10-17.md`（P0；本条即其结论工作顺序 #1）。
- 相关轮次：`docs/validation-chunk-culling-2026-10-17.md`（P1；`cull=…` 口径）、
  `docs/validation-frame-pacing-2026-10-17.md`（N1a；pacing 口径）。
- 3D 预算/分页口径：`docs/render-3d.md` 的「分页预算与每帧活动预算」节。
