# 每帧 uniform/encode 成本：超集安全的编码跳过与 slot 压缩 验证（2026-10-17）

**目的**：记录 `docs/render-optimization-backlog.md` §N1e 的**后续项**（"每帧 uniform/encode 成本"）落地后的
**本机真实宿主实测**、**机制说明**、**超集安全性论证**与**测试矩阵**。本轮是**纯 renderer 改动**
（`crates/cad-render-wgpu`；实现 commit `1058af2`），**未**改宿主协议、**未**改 UI、**未**新增诊断码。

**这不是**性能验收、**不是** vendor/视觉结论。单机、单适配器、单会话、同 UID。口径与
`docs/validation-plan-cost-2026-10-17.md`（N1e 本体）、`docs/validation-gpu-time-2026-10-17.md`、
`docs/validation-chunk-culling-2026-10-17.md`、`docs/validation-frame-pacing-2026-10-17.md` **不同**，
各组数字**不得混算**。

## 环境与授权边界

| 项 | 值（**实测**） |
|---|---|
| 机器/会话 | 本机桌面会话，**原生 Wayland**，同一 UID；**无** root/`sudo`、**无**屏录、**无**虚拟/无头显示 |
| 适配器 / 后端 | **Vulkan**，`Quadro P620` |
| 缩放 | `scale=1` |
| 宿主进程 | `target/release/yacr`（本轮**未记录**二进制哈希，故不声称具体 commit 对应关系） |
| 退出/卫生 | 自然 **exit 0**、**无** panic、**无**残留进程 |
| 计时口径 | 宿主逐帧 `[probe] actual-render …` 的 `plan_ms` / `uniform_ms` / `encode_ms` / `submit_ms` / `cpu_ms` |
| 证据落盘 | 仅本地（`${YACR_EVIDENCE_ROOT:-/tmp/opencode}` 下本轮输出目录），**未上传** |

## 病根（实测）

N1e 落地后，planner 已能接受**整条**有序表（`complex-test.dwg` **166,346** 条）而只发**一次** draw，但
**uniform 循环与 encode 循环仍逐条走完全部批**：一张 1-draw 的 settled 页仍耗 `uniform_ms` **6.071**、
`encode_ms` **5.365**，`cpu_ms` 合计 **22.42**。这是与 N1e 同源的病——**每帧 O(驻留场景)，与可见工作无关**。

## 机制（`crates/cad-render-wgpu`）

1. **紧凑的每批 run class**（`plan.rs:927-941` 的 `BatchFacts::class: Vec<u8>`）——每批**一个字节**的位域
   （`CLASS_INSTANCED` / `CLASS_MESH` / `CLASS_ARC`，`plan.rs:944-949`），`0` 表示该批**不拥有任何 run**
   （走 legacy 逐批分支）。同一 `BatchFacts` 还保留 `legacy_batches` 列表（`plan.rs:937-941`）。二者均
   **每批集变更只算一次**（复用既有 `draw_plan = None` 失效）；`class` 是**缓存驻留**的字节，而驻留
   `GpuBatch` 约 **200 B**（帧顺序 ≠ 上传顺序时是一次 DRAM 未命中）。
2. **每帧 `EncodeEmitMask`**（`plan.rs:1048-1120`）——标记本帧 encode 阶段**参与**的批：
   - 无 run 的批（走 legacy 分支，flush 挂起的合并 run）；
   - 有 run **且该 run 通过本帧剔除**的批；
   - 恒活动的 legacy quad/mesh 集（`always_active`）。
   仅当 `!cull.any_culled()` 时整掩码退化为 `all`（不建位集）；`Default` 亦是 `all`（未重建的掩码**绝不**
   跳过任何东西）。重建点 `renderer_impl.rs:4543`（`rebuild(&batch_facts, &cull)`）。
3. **超集安全**：掩码是 planner **活动集的超集**（`legacy_batches ∪ always_active ∪ 可见 chunk 的批`），
   故**绝不**丢 draw、**绝不**丢 flush。一张 run 全被剔除的**已转换**批在 encode 循环里是**纯 no-op**
   （其按类分支只加计数、不发 draw、不碰挂起 run），因此可在其约 **200 B** `GpuBatch` 被读取**之前**跳过
   （`renderer_impl.rs:5220-5235`：`class` 字节判定 + `encode_emit_mask.participates(i)`）。
4. **只有无 run 的批拿每批 uniform slot**（`renderer_impl.rs:4804-4828`）：legacy `draw_batch` 是
   `frame_slots` 的唯一读者，而它只对无 run 的批取用；有 run 的批通过 stream/chunk/arc slot 绘制，全剔除的批
   **什么都不发**。per-stream/chunk/conic slot **只为可见 chunk 写**（`renderer_impl.rs:4898-4908` 的 arc 例等），
   并经运行 slot 计数器 `next_slot` **压缩**（settled 页 `total_slots` **41,322 → 1**，`renderer_impl.rs:4911`）。
5. **字节同一**：每个存活条目的 **96 B** uniform 由**同一次调用、同一输入**产出，故其字节**逐字节相同**，
   只有内部 slot 索引变化。
6. **上报计数从 class 字节精确复现**（`renderer_impl.rs:5221-5231`）：`converted_batches` /
   `merged_mesh_batches` / `instanced_batches` / `arc_batches` 计的是**被接受的已转换批**，不是发出的 draw。

### 刻意**不**跳过

- **透明 pass**：转换要求不透明批 ⇒ 每个透明批都是 legacy、都是真实发出的工作，**不跳过**。
- **加速计数循环**（`renderer_impl.rs:5056-5075`）：`use_pulled` 要求**无任何剔除**，此时没有可跳过的项。
- **两个主循环仍迭代 Θ(accepted)/Θ(opaque)**——每个被跳过条目**只花一次字节读**——因为**上报的计数定义在
  整个被接受集上**（不能只统计存活项）。

## 实测（本机真实宿主）

### before → after（settled 1-draw 页；中位 / p90）

| fixture | `plan_ms` | `uniform_ms` | `encode_ms` | `submit_ms` | `cpu_ms` |
|---|---|---|---|---|---|
| complex-test | 10.747 / 18.084 → **1.130 / 1.341** | 6.071 / 6.862 → **0.151 / 0.197** | 5.365 / 5.598 → **0.656 / 0.802** | 0.206 → 0.186 | 22.42 / 30.78 → **2.180 / 2.511** |
| debug-default-fonts | 0.677 / 0.759 → **0.132 / 0.207** | 0.135 → **0.026** | 0.247 → **0.090** | 0.140 → 0.224 | 1.244 / 1.367 → **0.522 / 0.796** |

- `submit_ms` **无 p90**（本轮只在 settled 页记录中位）。complex 的 `22.42 = 10.75 + 6.07 + 5.37 + 0.21`
  即 before 页的相位和。
- 本表的 `plan_ms`（1.130 / 1.341）是**本次运行**的值，与 N1e 本体证据文中的 after 值（1.221 / 2.749）
  **是不同运行**，**不得混算**。

### 独立 orchestrator 复跑（另一次宿主运行，complex-test）

| 观察 | before | after |
|---|---|---|
| settled 帧数 | **50** | **128** |
| `plan_ms` 中位 | **10.499** | **1.131** |
| `uniform_ms` 中位 | **6.056** | **0.155** |
| `encode_ms` 中位 | **5.335** | **0.652** |
| `submit_ms` 中位 | 0.207 | 0.182 |
| `cpu_ms` 中位 | **22.158** | **2.207** |

两次运行都出现**同一病态页签名**（`draw_calls=1 verts=12 tris=4 over_budget=false pacing=active
cull_chunks=1/9362`）。

### 确定性字段（settled 页，before/after **逐字段一致**）

两个 fixture 的 settled 页上，以下打印字段 before/after **完全相同**：`draw_calls`、`verts`、`tris`、
`instanced`、`mesh`、`legacy_opaque`、`arc`、`transparent`、`images`、`accelerated`、`instance_streams`、
`instanced_batches`、`tier`、`over_budget`、`prog=`、`complete`、`pacing=`、`cull_chunks=`。
**唯一移动的打印字段**是 `budget_verts` / `budget_tris`——二者**仅**因 pacing 控制器的 scale 依赖墙钟而不同。

## 两处诚实更正（必须记录）

1. **settled 页 `submit_ms ~5 ms` 的估算是错的**：实测为 **0.206 ms**（`22.42 = 10.75 + 6.07 + 5.37 + 0.21`），
   即**提交在该页从来不是因素**。此前证据/计划文本不得再暗示相反（更正落点见下）。
2. **paced 帧数字被混淆**：after 运行的 paced 页接受约 **1.6×** 顶点、约 **2×** draw，因为更快的 settled 帧
   让依赖墙钟的 pacing 控制器**放大** scale。**按单位发出工作计**，encode/submit 成本**没有**增加。
   **只有 settled 1-draw 页是干净的 before/after。**

### 更正落点

- `docs/validation-plan-cost-2026-10-17.md`「未测 / 边界」的 `submit_ms`（~5 ms）条已加**dated 更正注**。
- `docs/render-optimization-backlog.md` §N1e「边界」与「后续」条的 `submit_ms`（~5 ms）已更正。

## N2 重估（本轮第二项结论）

settled 页落地后，运动（paced）帧的账目变了，N2 的价值随之改变（详见
`docs/render-optimization-backlog.md` §N2）：

- 运动帧现约 `cpu_ms ≈ 3.6` + `wait_ms ≈ 4.2` ≈ **8 ms**（**实测**，逐运行被墙钟轨迹混淆，但两相位现均小）
  ⇒ **在 60 fps 预算（16.7 ms）之内且有裕量**。
- `gpu_ms ≈ 3.95`（**实测**，见 `docs/validation-gpu-time-2026-10-17.md`），约为 `wait_ms` 的 **93%**
  ⇒ **主导残余是 GPU 执行**，**任何 CPU 侧杠杆（含 N2）都不减少它**。
- N2 只能重叠 `cpu` 与 `gpu`，或可把运动帧从 ≈8 ms 压到 **≈4–5 ms**（**估算，未测**）⇒ **买延迟裕量，
  不是补预算缺口**。⇒ **不得以旧理由开工**；N2 **重挂于产品需求**（120 Hz 目标，或更重图纸使运动帧再次
  越预算）。

## 测试矩阵（**实测**）

| 命令 | 结果 |
|---|---|
| `cargo test -p cad-render-wgpu --lib` | **195 passed / 3 ignored**（含 **3** 个新纯测试） |
| `cargo test -p cad-render-wgpu --test headless_render` | **35** |
| `cargo test -p cad-render-wgpu --test accelerated_tier` | **23** |
| `cargo test -p cad-render-wgpu --test pulled_prefix_equivalence` | **9** |
| `cargo test -p cad-render-wgpu --test render_effects` | **45** |
| `cargo test -p cad-render-wgpu --test pick_id` | **32** |

### 三个新纯测试（名称即契约）

- `encode_skip_is_superset_safe_and_preserves_counters`（`plan.rs:5676`）——**4000 例随机差分**，对一个
  **从不跳过**的参照游走 `encode_reference_emits` 断言：掩码**绝不**跳过参照会发出的批
  （`participates == false ⇒ emits == false`），且被跳过/存活的发出集合与**由批派生的上报计数**逐项一致。
- `fully_culled_batch_with_legacy_quads_is_still_processed`（`plan.rs:5763`）——**legacy-quad 回归**：一张全剔除
  但 `line_quad_count > 0` 的批**仍必须**参与（谓词的 `always_active` 一侧）。
- `skipped_uniform_slots_leave_surviving_bytes_bit_identical`（`plan.rs:5791`）——**2000 例**：把旧（不跳过）
  slot 集与新（跳过）slot 集对比，**每个仍获得 slot 的条目字节逐字节相同**；被丢弃的条目**恰好**是 arc-only
  批（它们从不读 `frame_slots`）。

## 未测 / 边界（不得省略）

- **paced 帧数字被墙钟轨迹混淆**（见「两处诚实更正」②）：**只有** settled 1-draw 页是干净的 before/after。
- **full-view（无剔除）帧仍未修、未测**：本轮的跳过**只在有剔除时**生效（`!cull.any_culled()` ⇒ 掩码 `all`）；
  全览页没有可跳过的项，其 `uniform_ms`/`encode_ms` 是否值得进一步优化**未测**。
- **GPU 执行份额未归因**：`gpu_ms` 只有每帧总量（P0），**未**分顶点/三角形吞吐 vs draw 数 vs 带宽；
  **是否还有值得做的 CPU 工作**——**未测**（运动帧 ≈8 ms 已在预算内，主导残余是 GPU 执行）。
- **视觉 / 截图判定未做**：本轮只有日志（相位计时与字段），**无**像素读回；字段一致是**日志级**证据，
  不是像素级证据。
- **wasm / 浏览器真实运行时、Windows/macOS/Android**：未测。
- **vendor / golden / 性能结论**：单机、单适配器，**不得**外推。

## 交叉引用

- 决策与优先级：`docs/render-optimization-backlog.md` §N1e（后续）、§N2（重估）、§N5（一致性）。
- 上游：`docs/validation-plan-cost-2026-10-17.md`（N1e 本体，本轮即其「后续」项）、
  `docs/validation-gpu-time-2026-10-17.md`（P0；`gpu_ms` / `wait_ms` 口径）。
- 相关：`docs/validation-chunk-culling-2026-10-17.md`（P1；`cull_chunks` 口径）。
- 3D 预算/分页口径：`docs/render-3d.md` 的「分页预算与每帧活动预算」节。
