# 渲染/性能 待优化项（Optimization Backlog）

本文是渲染与性能的**待优化项清单**（backlog），不是已完成能力表、也不是发布承诺。每项给出
现状、预期收益、前置条件/风险、需要的产品/UX 决策、相关契约/栅栏与优先级。已完成项见
`docs/adr/0007-entity-as-data-renderer.md`、`docs/adr/0008-sdf-conic-rendering.md`、
`docs/picking-gpu-id-pass.md`、`docs/validation-render-perf-2026-10-15.md`。本轮**真实宿主实测**
与**帧成本分布**分别见 `docs/validation-realhost-frame-cost-2026-10-17.md`、
`docs/validation-render-perf-2026-10-17.md`；**分页预算 pacing（N1a）**的实测见
`docs/validation-frame-pacing-2026-10-17.md`；**测量专用 GPU 帧时间（P0）**的实测见
`docs/validation-gpu-time-2026-10-17.md`；**每帧 CPU plan 成本（N1e）**的实测见
`docs/validation-plan-cost-2026-10-17.md`；**每帧 uniform/encode 成本（N1e 后续）**的实测见
`docs/validation-uniform-encode-skip-2026-10-17.md`；**提交路径 pick 表增量更新（N1f）**的实测见
`docs/validation-pick-table-incremental-2026-10-17.md`；**后缀提交保持 pacing 成本模型（N1a 后续）**的实测见
`docs/validation-pacing-suffix-commit-2026-10-17.md`；**生产 CPU 拾取接入空间索引（S4）**的实测见
`docs/validation-pick-index-2026-10-17.md`；**相机-only 导航保持选择高亮**的实测见
`docs/validation-selection-survives-camera-2026-10-17.md`。

## 当前基线（2026-10-17，真实宿主**分页**口径）

**当前基线（真实 NVIDIA Quadro P620 / Vulkan / Wayland，`target/release/yacr`，`--open` +
有界 post-load pan，见 `docs/validation-realhost-frame-cost-2026-10-17.md`）**：

| fixture | entityCount | 每页 `draw_calls` | 每页 `verts` | 每页 `tris` | `wait_ms` 中位 | `wait_ms` p90 | `cpu_ms` 中位 | plan 总工作单位 |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| `complex-test.dwg` | 44,591 | **3,027** | 5,454,559 | **1,999,514**（≈2.0M 饱和） | **19.382** | 22.652 | 8.274 | **166,346** |
| `debug-default-fonts.dwg` | 10,656 | 1,195 | 2,170,905 | 1,999,961（≈2.0M 饱和） | 2.895 | 5.369 | 2.072 | 10,548 |

- **99% 的帧 `over_budget=true` / `complete=false`**（分页/超预算）。**口径更正（2026-10-17）**：
  `over_budget=true` 的含义是"本帧**计划**越过了**按页预算**"，**不是**"这一帧慢"。两图 `tris` 都饱和在
  ≈2.0M（complex 1,999,514 / debug 1,999,961）⇒ 这是**按页预算截断**，整帧被切成多页呈现。
  **debug 不是时间受限**：其 `over_budget=true` 为 473/479 帧，但 `wait_ms > 16.6 ms` 仅 **0.4%**、
  每帧 ≈ 2.072 + 2.895 ≈ **5 ms**（**实测**，同一批**pacing 之前**的日志），在 60 fps 预算之内——
  固定顶点/tri 记账与实测帧成本脱钩（N1a 动机）在 debug 上表现为"**页预算偏小**"（每页早早撞上 2M 三角形
  上限、帧被切成多页），**而非**"帧超时"。
- **complex 有 61.3% 的帧仅 `wait_ms` 就超过 60 fps 预算**（debug 0.4%）。**`wait_ms` 是 GPU/呈现
  等待的代理，不是 GPU 时间**（`gpu_ms=None`，无 `TIMESTAMP_QUERY`）。⇒ **两 fixture 只有 complex 是
  时间受限**（同上，**实测**）。
- **每帧 CPU 相位（中位，ms；由既有日志列表化，非新运行）**：
  complex `cpu 8.364 = plan 2.590 + uniform 1.123 + encode 1.131 + submit 3.412`（未覆盖 0.109）；
  debug `2.072 = 0.484 + 0.126 + 0.482 + 0.936`（未覆盖 0.044）。
  **`submit` 是 CPU 侧最大相位**（complex 占 41%）。帧 ≈ `cpu + wait`（complex **27.75 ms ≈ 36 fps**，
  wait 占 ≈70%）。
- **每页 draw 类别（我方逐帧日志，取中位；已排除 demo 文档帧；本次自原始日志 `complex.err` /
  `debug.err` 重算）**：

  | fixture | `draw_calls` | `instanced` | `mesh` | `arc` | `legacy_opaque` | `transparent` | `images` | `instance_streams` | `draws/stream`（`instanced`÷`instance_streams`） |
  |---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
  | `complex-test.dwg` | 3,027 | 1,615 | 324 | **1,088** | **0** | 0 | 0 | 6,314 | **≈0.26** |
  | `debug-default-fonts.dwg` | 1,195 | 823 | 268 | 104 | **0** | 0 | 0 | 578 | **≈1.42** |

  **实测结论（决定 P1）**：`legacy_opaque` 为 **0**——**每一条 draw 都是"合并 run"**（instanced /
  合并 mesh / arc），而一段 run 是**恰好一条 stream = 一 chunk** 的连续切片（flush-per-run
  `crates/cad-render-wgpu/src/renderer_impl.rs:4666-4678`；per-chunk runs
  `crates/cad-render-wgpu/src/plan.rs:1804`、`:1880-1886`）⇒ **chunk 级跳过能整条消掉 draw**（P1）。
  早先"残留 **1,088** 是逐批 legacy"的假设**被推翻**：此 fixture 上残余类是 **`arc`（合并的 SDF
  conic run，`arc_draws`）**，同样是 per-stream run。**口径声明（两轮不得混算，铁律 6）**：本表（类别 / `instance_streams`）取自**容量门控之后**的同日运行（complex 366 帧、debug 482 帧），而**上面基线表**的 `wait_ms`/`cpu_ms`/超预算比例取自**门控之前**的同日运行（complex 354 帧）。门控后同口径中位：complex **`wait_ms` 19.830（p90 22.881）、`cpu_ms` 7.137（plan 2.175 + uniform 0.892 + encode 0.980 + submit 2.812）、`wait>16.6 ms` 62.8%、帧 ≈26.97 ms**；debug **`wait_ms` 2.839、`cpu_ms` 2.005、0.4%**。两组是**两次运行**，**不得**互相代入或合并（`cpu` 差 ≈14%、`wait` 差 ≈2.3%）。**口径声明**：complex 的这一张表来自一次
  **加速组织被容量跳过**的运行（见下「容量」条；`pulled=skipped-insufficient-vram`），故它描述的正是
  **通用 per-run、每帧都走的路径**。
- **分页在相机变化时重启**（同文）：complex 182 次相机变化，**182/182 把该帧完成度压到 <5%**
  （中位 **4.6%** ≈ 7,716/166,346）；356 帧里**仅 2 帧** `complete=true`。⇒ 运动帧呈现的是
  **draw-order 前缀页**，**不是**基于视口的集合。debug 变更后完成度中位 **86.3%**（这是**相机变更后第 1 帧
  的完成度中位**，**不是**"超预算帧占比"、也**不是**耗时指标；debug 一页基本就能画完）。
- **`tier` 与 `accelerated` 是两个字段**（`crates/cad-render-wgpu/src/lib.rs:473-481`）：设备档可为
  `accelerated` 而**帧**为 `accelerated=false`；`use_pulled` 要求二者相等
  （`converted_accepted == converted_total`，`crates/cad-render-wgpu/src/renderer_impl.rs:4271-4276`）。
  **实测**：每个真实场景页都报 `accelerated=false`（设备 `tier=accelerated`）。⇒ **生产交互帧走的正是
  通用 per-run 组织**。该等式的准确语义是"**每个已转换批次都被接受**"，**不是**"未分页"——故"分页 ⇒
  通用"在**实践中成立，但并非构造上必然**。

### 历史基线（2026-10-15，**整幅/无界预算/无头**口径）

**历史基线（真实 NVIDIA Quadro P620 / Vulkan，`debug-default-fonts.dwg` / `complex-test.dwg`，
见 `docs/validation-render-perf-2026-10-15.md`）**：

- draw 数：10,474 → **797**；122,006 → **7,627**（§4）。
- 每帧 cpu（pan_mid）：17.96 → **1.88 ms**；46.74 → **5.91 ms**（§4）。
- 静态几何在 pan/zoom 全程零流/chunk 重传；逐帧 uniform 由 O(批数) 降为 O(pages)（ADR-0007）。

> **两套口径不得混算（AGENTS.md 铁律 6）**：历史头是**整幅/无界预算/无头**（797/7,627 draws、
> 1.88/5.91 ms）；当前头是**分页/超预算/真实宿主**（3,027/1,195 draws、19.382/2.895 `wait_ms`）。
> 二者的 entityCount 也不同（complex 44,591 vs 122,006）——**不可**放进同一个比值或差值里。

下述所有收益估算以**当前（分页）基线**为参照；未实测的收益一律标注为**预期/估算**，不得当作已验证。

优先级记号：**P0–P7**（既有渲染项）、**N1–N5**（2026-10-17 复核 + 实测后新增，**N1 与 P1 并列最高**）、
**G1–G7**（既有 deferred/已知项结转）。

**绑定原则（binding，2026-10-17 GPU-vs-CPU 分析后新增）**：GPU 剔除 / 间接绘制是**能力门控的
bonus，永远不是架构依赖**；每项此类增强**必须**有 capability 检测 + 基础路径 + 对比基线，且
**必须先 profiling**（规范 §8.5）。**在 GPU 帧时间可测量之前，不构建任何 GPU 侧剔除**（见 P0）。
**设备 feature / limits 的协商策略须有 ADR**（与**测量专用**的 `TIMESTAMP_QUERY` 设备同轴；见 N5/N3）。

---

## 性能类（P0–P3、P5–P7；**P5 已否决、P6 已拆分**）

### P0 — 测量前置（measurement prerequisite）：GPU 帧时间 + P1 剔除 A/B（**已落地**）

- **已落地（2026-10-17）**。逐条证据见 `docs/validation-gpu-time-2026-10-17.md`；政策见 ADR 0013
  第 1 类（measurement-only）。
  - **唯一读取点与分层**：环境变量 **`YACR_PROBE_GPU_TIME`** 只在 `apps/app-linux/src/host.rs:1327`
    读取；**渲染器/引擎从不读环境**——宿主把结果装进显式 `cad_render_wgpu::gpu_time::GpuTimeConfig`，
    经 `gtk_bridge::install` → 呈现器设备创建（`native_presenter.rs`）→
    `attach_device_with_gpu_time`（`crates/cad-engine/src/render/mod.rs:1105`）→
    `Renderer::set_gpu_time_measurement` 逐层显式传参。
  - **关闭（产品默认）**：设备**不请求任何 feature**，`gpu_ms` 保持 `None`，打印
    `[probe] gpu-time … state=disabled`；**不填估算值**。**打开**：请求
    `TIMESTAMP_QUERY | TIMESTAMP_QUERY_INSIDE_ENCODERS` 与 adapter 集**取交**；本机两者**均授予**
    （`state=enabled`）；`TIMESTAMP_QUERY_INSIDE_PASSES` **刻意不请求**（写入在编码器上）；缺 feature
    时显式报 `gpu_time=unsupported(<FEATURE>)`，**绝不**静默回退。
  - **读回策略（无额外提交）**：帧**既有**编码器上帧首/帧尾 `write_timestamp`，随后 `resolve_query_set`
    + `copy_buffer_to_buffer` 到 16 B `MAP_READ` 缓冲（`renderer_impl.rs:5538-5543`），**同一次提交**；
    `map_async` 在 `submit` 之后、阻塞 poll **之前**排队，poll 完成映射后读 ticks（**无第二次等待**），
    用 `queue.get_timestamp_period()` 经 `ticks_to_ms` 换算；资源一次性创建、逐帧复用（创建计数器恒 **1**）。
  - **上报（追加式）**：`FrameStats::gpu_ms`（`Option<f64>`）+ `gpu_time` 状态；
    `[probe] actual-render` 行尾追加 `gpu_time={}`；启动一行 `[probe] gpu-time requested=… granted=… state=…`。
    **未新增**诊断码。
- **真实宿主实测（本机原生 Wayland / Vulkan / Quadro P620 / 驱动 580.178.04 / `scale=1`；同 UID、
  无 root/sudo、无屏录、无虚拟显示；四次运行均 exit 0、无 panic）**：

  | fixture | opt-in | frames | `gpu_ms` 中位 / p90 / max | `wait_ms` 中位 | `cpu_ms` 中位 | gpu/frame | gpu/wait |
  |---|---|---:|---|---:|---:|---:|---:|
  | `complex-test.dwg` | off | 697 | —（`None`） | 4.203 | 4.463 | — | — |
  | `complex-test.dwg` | on | 698 | **3.948 / 7.479 / 24.416** | 4.242 | 4.111 | **43.8%** | **93.0%** |
  | `debug-default-fonts.dwg` | off | 452 | —（`None`） | 2.234 | 2.055 | — | — |
  | `debug-default-fonts.dwg` | on | 462 | **2.643 / 6.205 / 6.600** | 2.909 | 1.439 | **55.0%** | **93.9%** |

  - settled `complete=true` 页（paced）：complex **50** 帧（`gpu_ms` 中位 0.025、`cpu_ms` 22.158、
    `wait_ms` 0.683、frame 22.930、gpu 0.1% / cpu 97.0%）；debug **376** 帧（`gpu_ms` 2.551、`cpu_ms` 1.294、
    `wait_ms` 2.806、frame 6.726、gpu 50.3% / cpu 44.3%）。
  - **关闭腿 0/697 与 0/452 帧带值**（`None` 是未请求 feature 的诚实结果，不是缺测）。
  - 每帧 CPU 相位（中位份额）：complex all-paced plan **53.2%** / uniform 22.8% / encode 5.9% / submit 17.2%；
    complex complete plan 48.0% / uniform 26.9% / encode 23.9% / submit 0.9%；debug all plan **55.2%** /
    uniform 7.1% / encode 20.4% / submit 18.1%；debug complete plan 56.3% / uniform 7.7% / encode 19.9% / submit 17.5%。
- **栅栏（已落地，名称即契约）**：`crates/cad-render-wgpu/tests/headless_render.rs` 的 `gpu_time_contracts`——
  `disabled_requests_no_feature_and_reports_none`（关闭 ⇒ 无 feature、`gpu_ms=None`、**0** 资源）；
  `unsupported_state_names_the_missing_feature`（缺 feature ⇒ **命名**它、**0** 资源）；
  `enabled_measures_without_changing_pixels_and_does_not_grow`（打开 ⇒ 真 `gpu_ms`、与关闭腿**逐字节一致**、
  资源创建计数器恒 **1**）。另：tick 换算为纯测试；wasm 库编译且 `wasm32` 恒报 `Disabled`。
- **本轮主结论（证据驱动重排优先级）**：N2 重开三条件据此判定——**(a) 为假**（`exec ≈ wait_ms`，比值 **0.93**
  ⇒ `wait_ms` 就是真实 GPU 执行、present/queue 仅约 6–7%）；**(b) 为真**（paced 帧超目标且 `cpu_ms` ≈ 帧时间
  **41–52%**）；**(c) 成立**（120 Hz / 8.33 ms 下 complex `cpu_ms` p90 ≈8.5 ms 单独超预算）。⇒ 工作顺序变为
  **(1) 每帧 CPU plan 成本** → **(2) N2 流水** → **(3) N5 indirect multi-draw 最后**（详见 §N1d / §N2 / §N5）。
  > **更新（2026-10-17，§N1e 后续落地后）**：工作顺序 **(1) 每帧 CPU plan 成本** 及其 **uniform/encode 后续**均已
  > 落地（见 §N1e）；条件 **(b) 已失效（moot）**——运动帧 ≈8 ms 已在 60 fps 预算内，**N2 已按实测重挂**（见 §N2）；
  > **(3) N5** 的降级不变。
- **未测 / 边界（不得软化）**：**GPU 内部归因**（只有每帧总量，未分顶点/三角形吞吐 vs draw 数 vs 带宽）；
  **N1d / N2 / N5 是否真能降低 `gpu_ms`**；**真实 adapter 的 `unsupported(<feature>)`**（只在 fence 里经
  **能力注入钩子**模拟——本机两 feature 均授予）；**wasm 运行时**（只有库编译）；**真实宿主像素捕获**（字节一致在
  headless harness 证明，宿主两次运行只有日志）；**扰动**：测量设备**不是**产品配置（多了 resolve/copy + map
  开销）；**vendor / golden / 性能结论**：单机、单适配器，**不得**外推。
- **原「未做」表述（保留供追溯）**：`gpu_ms=None`、无 `TIMESTAMP_QUERY`、`wait_ms` 只是代理、所有"GPU 收益"
  均为结构推理——这些在落地前成立；落地后仅在 **opt-in 打开**时为真时间戳，**产品默认仍 `None`**。

### P1 — 块级视锥剔除（chunk-level frustum culling）

- **已落地（2026-10-17）**。逐条证据见 `docs/validation-chunk-culling-2026-10-17.md`。
  - **谓词**：新纯模块 `crates/cad-render-wgpu/src/cull.rs`（`WorldAabb` / `ChunkCull` / `build` /
    `ortho_visible` / `perspective_visible` / `class_visible`，**10** 个纯单测）。三个构建器本已为切分
    启发式算出的 world-space `f64` AABB 现被**保留**到上传结构（`InstanceStream` / `GpuMeshChunk` /
    `ArcStream` 的 `aabb`），每帧对 `FrameProjection` 测试**一次**，**O(chunks)**，无逐实体逐帧计算。
  - **保守单侧**：非有限/空/反向 AABB 恒可见；跨 chunk 的超大源每个分片携带**整批** AABB；
    混合远处批次的 chunk 从不剔除。
  - **测试期 pad（绝不烘焙进存储 box），逐类**：line `3·wpp_line + 1e-6`；mesh `1e-6`（无 stroke/quad
    展开）；arc `2·wpp_arc + max(0.02·R, 0) + 1e-6`（`R` = chunk AABB 最大半extent）。
    `wpp_line = line_ramp_override.unwrap_or(projection.world_per_px())`、`wpp_arc = projection.world_per_px()`。
    **`lineweight` 不产生 pad 项**——它被携带但**未光栅化**（`line_instanced.wgsl`），**不是**遗漏项。
  - **Ortho**：精确 `f64` 2 轴 NDC 测试，**z 刻意不测**（深度映射不是可见性判据）。**Perspective**：
    6 平面 p-vertex 测试 + f32 相对 margin（透视原点被 f32 窄化）。
  - **记账**：被剔除 run 在每个 encode/合并点跳过（**从不重排、从不跨空隙合并**）并计**零**顶点/三角形；
    字节记账仍按批粒度。透明 legacy draw 与图像**不**剔除。
  - **pick 从不剔除**：`pick.rs` 传恒等掩码（`ChunkCull::all_visible()`），命中测试与视图剔除独立。
  - **上报（追加式）**：`FrameStats` 新增 `cull_chunks_total` / `cull_chunks_kept`；`[probe] actual-render`
    在 `pacing={}` 之后追加 `cull_chunks={kept}/{total}`。
  - **实测区间（本机真实宿主；口径与 pacing 轮不同，不得混算）**：`complex-test.dwg` 681 页、161 页
    `kept<total`，`cull_chunks` **`1/9362` … `9362/9362`**、`draw_calls` **`1` … `3027`**（3,027 基线在
    `9362/9362` 复现），最深剔除页 `draw_calls=1 verts=12 tris=4 instanced=1`；`debug-default-fonts.dwg`
    466 页、215 页 `kept<total`，`cull_chunks` **`1/806` … `806/806`**、`draw_calls` **`1` … `1299`**。
  - **同轮实现边界**：parity fence 跑在 **headless 软件 Vulkan**，**不是**硬件；真实宿主运行只产出
    **日志证据**（无像素读回）；**性能/vendor/精度结论不由此得出**。**开放问题（逐字记录）**：debug 的
    最大 `draw_calls` **1299** 高于更早的 1,195 基线，实现 lane **未**对账（可能不同的相机/页构成）。
- **现状/问题（落地前，保留供追溯）**：`build_instance_chunks`（`crates/cad-render-wgpu/src/plan.rs:1804`）与
  `build_mesh_chunks`（`plan.rs:1122`）在构建 chunk 时**已经算出世界坐标 AABB**
  （`batch_world_aabb` `plan.rs:1664-1680`、`piece_world_aabb` `plan.rs:1017-1033`、
  `aabb_half_diagonal` `plan.rs:1684-1689`）：构建器**保留**它们（`ChunkBuilder.min/max`
  `plan.rs:1706-1707`、`MeshChunkBuilder.min/max` `plan.rs:1042-1043`、`ArcChunkBuilder.min/max`
  `plan.rs:2093-2094`），但**上传结构不携带 AABB**——`InstanceChunk`（`plan.rs:1692-1697`）、
  `MeshChunkData`（`plan.rs:931-936`）、`ArcChunk`（`plan.rs:2072-2077`）只有 `origin` +
  `instances`/`vertices`/`indices`。当前每帧对全部 chunk 提交 draw，**不判断可见性**——放大到
  图纸局部时，绝大多数 chunk 在视口外仍被绘制。
- **范围（2026-10-17 反转为准）**：**生产交互帧本身就是通用 per-run 组织**——实测每个真实场景页
  都报 `accelerated=false`（设备 `tier=accelerated`），因 `use_pulled` 要求**整场景被接受**
  （`crates/cad-render-wgpu/src/renderer_impl.rs:4271-4276`）。⇒ **P1 覆盖的正是实测的 3,027
  draws/页帧**（complex）与 1,195 draws/页帧（debug）；**加速档的 per-class 组织只在"整场景被接受"
  的帧上运行**（合成腿 447/108/19/3 draws 量级），**不在生产分页帧上**。通用档 per-run 提交下**每个
  run 属于单个 chunk**（`crates/cad-render-wgpu/src/plan.rs:1804`、`:1880-1886`），故 **CPU 整块跳过**
  能同时消掉该 chunk 的全部 draw 与其顶点工作（**CPU 跳过严格优于任何 VS 内早退**，见 P7 对比）。
- **粒度实测（`draws/stream` = `instanced` ÷ `instance_streams`，中位）**：complex **≈0.26**、
  debug **≈1.42**（见文首类别表）；两 fixture `legacy_opaque=0` ⇒ **无逐批 legacy draw**——complex
  残余 1,088 是 **`arc`（合并 SDF conic run）**，同样 per-stream run，可被 chunk 级跳过整条消掉。
  先前"残留 1,088 是逐批 legacy"的假设**已推翻**。
- **预期收益**：**CPU 收益适中**——基线（真实 P620/Vulkan、debug profile，见文首）显示卸载 draw
  很便宜（ADR-0007「P3b」实测：complex-test 去掉 8,233 draws 仅省 **1.28 ms**、debug
  ≈900 draws 省 **0.52 ms** ⇒ **≈0.16–0.6 µs CPU/draw**），故移除数千 draw 约省 **~1 ms
  （估算）**；**这不是测量值，是估算**。真正的收益在 **GPU 侧（跳过顶点工作）**，而**本仓从未
  测量过 GPU 帧时间**（见 P0）⇒ 该收益**未实测**。放大到局部时收益最大（当前"画全部"），全览时
  趋近 0。**"减少提交"是弱杠杆**（提交已近乎免费），本条价值主要在通用档的整块 CPU 跳过。
- **前置条件/风险**：需把构建期 AABB 下沉到运行时 chunk 结构并逐帧做视锥/AABB 测试（正交相机下
  AABB 与视锥相交即可；透视路径须独立 fence，见 ADR-0007「per-chunk f64 origin 限定于 Ortho」）。
  **保留渲染器内部顶点 AABB 用于剔除是 ADR-0012 §4 明确许可的**——被禁止的只是把它们**外泄**到
  pick / selection / zoom-extents / 空间索引。**风险：像素无损**（纯跳过，不改变保留几何的绘制）；
  需 fence 证明 **culled == 未绘制**，且 chunk 边界**无弹出**。
  - **缩放相关的笔画边距 padding（必须）**：AA 线四边按 **≥2 世界像素**展开（`w = world_per_px`
    ⇒ **≥2 屏像素**；ADR-0009 §1、`docs/render-order.md` 的 AA 统一节），SDF 弧四边亦按
    `grow = 1 + max(0.02, 2·world_per_px / r)`（≥2 world-px 绝对下限）扩展 conic extent
    （ADR-0008）。因此**仅按顶点 AABB 测试会在深度放大时于 chunk 边界弹出**：**必须给测试本身**
    加约 `2·world_per_px` 的 padding（**padding 测试，绝不膨胀存储的 AABB**）。
  - **残留 legacy/透明/图像 draw 需要各自的 AABB**：它们不走 chunk，`GpuBatch` 目前只存
    `centroid`（`lib.rs:593-594`），要做同类剔除须另加 AABB。
- **需要的决策（未决，非已解决）**：**「segment 粒度 × 可剔除性」**——加速档上要么维持 segment
  粒度（chunk 不可剔），要么重新按可剔除单元切分 stream（代价与收益未知）；**记录为开放决策**。
  该加速档子问题现由 **N1d**（pulled 子集门）承接，**N1d 仍开放**：本轮**不做** pulled **子集** draw，
  而是在任何剔除发生时用合取项 `&& !cull.any_culled()` 禁用 pulled（`renderer_impl.rs:4620`）——
  因为一次 pulled draw 会重绘整条 stream/chunk、会重绘被剔除的几何。⇒ **剔除目前强制走通用 per-run
  组织**（每 run 属单 chunk，故整条可跳过）。
- **相关契约/栅栏（已落地，名称即契约）**：`merged-vs-legacy` 逐字节一致的扩展已实现为
  `culled_frame_is_byte_identical_and_drops_exactly_the_culled_runs`（剔除帧逐字节一致且**恰好**丢掉
  被剔除的 run）；另有 `large_lineweight_at_the_viewport_edge_is_pixel_identical`、
  `pick_is_invariant_under_chunk_culling`、`culling_does_not_pop_across_a_chunk_border`、
  `culling_is_byte_identical_on_the_3d_path`、`culling_does_not_break_the_progressive_cursor`
  （共 6 个 GPU fence，见 `crates/cad-render-wgpu/tests/headless_render.rs`）。
- **优先级**：P1（最高；**已落地**——通用档、低成本、放大场景收益大；加速档覆盖有限，见上）。

### P2 — 像素级剔除 / LOD（pixel culling / LOD）

- **现状/问题**：亚像素实体仍按完整图元绘制；文字在任何缩放级别都是 glyph 轮廓（残留 draw
  主体，见 G1）。经典 CAD 策略（亚像素实体丢弃或降为 1px 点、文字在阈值下退化为矩形）**尚未实现**。
- **预期收益**：缩小视图时大幅减少图元与文字 draw；与 G1（glyph 实例化）互补。**预期**，未实测。
- **前置条件/风险**：属 **scene/representation 层**（`DisplayPrimitive`
  `crates/cad-representation/src/primitive.rs:20` 的 LOD 决策），**不是 renderer-only**——
  需在表示层按缩放阈值选择 primitive，渲染器只消费。**视觉语义变化**（亚像素实体消失、文字变
  矩形）→ 需**用户签核 + 视觉验收证据**；不得静默改像素。
- **需要的决策（产品）**：① 亚像素阈值（世界→屏幕尺寸门槛）；② 文字矩形退化的**外观**与阈值；
  ③ 图层/批注**例外**（哪些实体永不剔除/永不退化，例如批注、当前选中/悬停实体）。
- **相关契约/栅栏**：表示层 LOD 决策须可复现（纯函数 + 单测）；视觉 fence 覆盖阈值上下两侧
  与例外集合；渲染器侧 `merged-vs-legacy` fence 需在 LOD 生效后仍成立。
- **优先级**：P2（收益大但需产品决策与视觉验收，排在 P1 之后）。

### P3 — 帧缓冲复用 / blit+scissor 局部重绘（bandwidth+fragment 优化，**非** GPU compute）

> **标签更正（2026-10-17）**：本条此前被含糊地当作"GPU 优化"。**准确说法**：它是**带宽 + 片元**
> 优化（少画片元、少写/读显存），**本身不省任何 CPU 工作**——scissor 只裁片元，draw 仍被
> plan/encode/submit。**要省 CPU 必须与 P1 的区域过滤（对条带 rect 过滤）组合**。

- **现状/问题**：每次 pan/zoom 都整帧重画。可复用上一帧：**pan** = blit 上一帧 + 用 scissor
  重绘新暴露的 L 形条带；**zoom 进行中** = 缩放 blit（**瞬时模糊**），settle 后异步整帧矢量重绘。
- **预期收益**：仅在**百万实体量级且实测整帧重绘不足**时才有价值。**必要性现已实测**：complex
  分页帧中位 `wait_ms` 19.382、帧 ≈ **27.75 ms**，**61.3% 的帧仅等待就超 60 fps 预算**（见文首）——
  整帧重绘的代价已被测量，**不再是"未测量出必要"**。**带宽/片元收益为主，CPU 收益需靠 P1 区域
  过滤**。**条件性收益，未实测**。
- **已核实的交互（2026-10-17 分析）**：
  - **(a) 标志色失效** —— 今日渲染器在标志（flag）变化时丢弃**整幅**已完成的 progressive 帧
    （`renderer_impl.rs:1066-1068` 与 `:1147-1152`），且**没有 flag→屏幕区域映射**。因此像素缓存
    **必须采用同样的整缓存丢弃规则**，否则就要发明**第二套 damage 真相**——后者被 **ADR-0012 §5
    （不新增第二套失效真相）明确否决**。
  - **(b) progressive 累积** —— 续页用 `LoadOp::Load`（`clear = cursor == 0`，`renderer_impl.rs:4000`、
    `:4311`/`:4323`），且**图像只在清屏页绘制**（`renderer_impl.rs:4038-4055`，避免 alpha 反复累积）；
    条带重绘方案必须与之调和。
  - **(c) 高 DPI blit 模糊** —— 是**语义像素变化**，需 **UX 签核**；且规范 **§8.4 禁止把缓存图用于
    精确拾取**。
  - **(d) 内存** —— 每 4096² RGBA8 缓存纹理约 **64 MiB**（规范 §8.4）。
  - **(e) 分页重启 × 缓存协调（2026-10-17 新增）** —— 相机一变本帧完成度即塌缩（complex 中位
    **4.6%**，见文首），即**上一帧缓存只覆盖一个 draw-order 前缀页**；条带 blit/重绘方案必须与
    "每帧从头重算前缀页"调和，否则缓存内容与可见页不一致。这同样落回 (a) 的**整缓存丢弃**规则：
    宁整弃，不发明第二套 damage 真相（ADR-0012 §5）。
- **前置条件/风险**：**必须显式满足**：**P1 已落地 + 有实测的帧成本分布 + UX 模糊决策**；否则不开工。
- **需要的决策（UX）**：缩放过程中的**模糊容忍度**（是否接受瞬时模糊、模糊持续到 settle 的时长）。
- **相关契约/栅栏**：`blit_reuses_previous_frame_and_repaints_the_exposed_strip`、
  `flagged_regions_are_repainted_not_blitted`、高 DPI 下的清晰度 fence。
- **优先级**：P3（最低优先级；先满足上述前置条件。见 `docs/adr/0012-scene-spatial-index-and-interaction.md`
  §10.7：Dirty Region / Tile 明确排在交互切片之后）。

### P5 — RenderBundle 预录制（**已否决，2026-10-17**）

- **否决理由（2026-10-17 复核 + 实测）**：交互期间**被接受的集合每帧重决**——分页在相机变化时
  重启（complex 每帧相机变更都使本帧完成度塌缩到 <5%，见文首）⇒ 预录的 **RenderBundle 每帧即失效**；
  且 **P1/N1 会改变 draw 集**（块级剔除 + pulled 子集门），bundle 内容无法稳定。两重原因叠加 ⇒
  **不延后、直接删除**本项。
- **附（原表述保留供追溯）**：静态/动态拆分已就位（ADR-0007「静态 / 动态拆分」），残留 legacy 逐批
  draw（透明批次、图片、overlay，见 `docs/render-order.md`）仍逐帧过 wgpu 校验；原设想把它们预录进
  RenderBundle 以绕过逐帧校验。该设想在**静态整场景帧**上或有效，但**不是生产交互路径**。
- **优先级**：已否决（不再排期）。

### P6 — GPU compute 剔除（加速层 bonus；capability-gated；**间接绘制部分已移入 N5**）

> **拆分（2026-10-17）**：本条**只保留 compute 剔除**。**indirect draw buffer / multi-draw 部分移入
> 新增 **N5**（连同用户 2026-10-17 指令与全部 wgpu 30.0.1 更正）。P6 定位：**最后评估、能力门控**，
> 且在 **P1 落地之前无评估地位**（没有 CPU 侧剔除基线，GPU 侧剔除无从比较收益）。

- **现状/问题**：视锥/像素剔除若在 CPU 做（P1/P2）可覆盖通用档；更强方案是用 **compute shader** 做
  剔除并输出**可见列表**，减少 CPU 逐 chunk 测试。**本条不再涵盖 indirect draw buffer**（见 N5）。
- **预期收益**：**未实测**（GPU 帧时间从未测量，P0）；**且在 P1 落地前无评估地位**——CPU 块级跳过
  已能整条消掉 draw，compute 方案的**增量**收益必须相对 P1 基线对照才可判定。
- **前置条件/风险**：**capability-gated bonus**，**绝不能成为架构依赖**；WebGL2 档**永远**走 P1/P2
  的 CPU 剔除，回退结果须一致。**不得在 GPU 帧时间可测量之前构建任何 GPU 侧剔除**（见 P0）。
- **需要的决策**：无（架构约束已定）；compute 方案的评估**条件于 P0 与 P1**。
- **相关契约/栅栏**：capability 探测决定档位；compute 档与通用档的**像素一致** fence；档位在兼容表中
  显式建模（不得把加速档当作默认能力）。
- **优先级**：P6（加速档 bonus；**最后评估**；条件于 P0，且需 P1 先落地）。

### P7 — 可移植的按 chunk 可见性早退（portable per-chunk visibility early-out，VS 侧，**加速档整场景帧**）

> **前提已作废（2026-10-17）**：本条原动机（"整场景被拉取 ≈1.77×10^8 次 VS 调用/帧"）描述的是
> **加速档整场景帧**，而**生产分页帧不走该路径**（实测 `accelerated=false`；见文首）。故本条**不再
> 对生产交互帧成立**；重定为**条件于 N1d（pulled 子集门）+ P0 的 GPU 时间**才评估，且 **P1 落地后**
> 其残余价值进一步缩小。

- **现状/问题**：加速档（vertex pulling）在**整场景被接受的帧**上每帧绘制全部几何，与缩放无关——
  complex-test 的 **≈29.45M line 实例 × 6 次 VS 调用 ≈ 1.77×10^8 次 VS 调用/帧**
  （`pass.draw(0..6, 0..segment.instance_count)`，`crates/cad-render-wgpu/src/renderer_impl.rs:4636`；
  实例数见 ADR-0007「P3b」）。**但生产交互帧的分页路径不经过该整场景组织**（实测 `accelerated=false`），
  故此描述**不是**当前交互帧的真实工作量。机制：把 AABB 行喂进既有 `chunk_table`
  （`crates/cad-render-wgpu/src/lib.rs:917` / `:939`；加速档逐帧重写的 per-chunk 平移表），在 VS 内对
  **视口外 chunk**输出**退化顶点**（degenerate）→ 光栅化阶段丢弃。
- **预期收益（诚实）**：**省 0 CPU、0 提交、约 0 片元**；**只省 VS 调用次数 + 顶点取数**（这才是
  它的唯一价值）。**未实测**（GPU 帧时间从未测量）。
- **为什么不是"用 VS 早退做剔除"**：**VS 内 chunk 早退省 0 提交、约 0 片元**——
  - **通用档**：draw 是 per-run 的，每个 run 属于**单个 chunk** ⇒ **CPU chunk 跳过**可消掉整条
    draw 及其顶点工作；同一 draw 内的 VS 早退**不省任何提交**。⇒ **通用档上 CPU 跳过严格占优**。
  - **加速档**：draw 是 **per-segment** 的，而 segment 是**跨多 chunk 的整批 stream 绑定**
    （`lib.rs:723-727`）⇒ chunk 级跳过**跳不掉 draw**；VS 早退不省提交，且对视口外实例光栅化器
    本已裁剪 ⇒ **约 0 片元**。其**唯一**价值就是 VS 调用 + 顶点取数。
  - **WebGL2 是通用档**（`tier_for`，`plan.rs:129-152`）⇒ 该 VS 路线在那里**冗余**，CPU 跳过更好。
  - **具体结论（精确，非估算）**：**VS 早退省 0 ms CPU**。
- **前置条件/风险**：**明确以「N1d（pulled 子集门）+ GPU 帧时间已存在（P0）」为前置**；需
  **pixel-identical fence**（退化顶点不得改变任何保留像素）。**WebGL2 安全（构造上）但在那里冗余**。
  不得成为架构依赖（绑定原则）。
- **需要的决策**：无产品决策；**先有 N1d 与 P0 的 GPU 时间测量**再评估是否值得做。
- **相关契约/栅栏**：pixel-identical fence（早退 vs 不早退逐像素一致）；`chunk_table` AABB 行的
  正确性（越界/NULL 守卫）。
- **优先级**：P7（加速档 bonus；**条件于 N1d + P0**，排在 P6 之前评估——compute-free 即可达成同样
  收益；**P1 落地后残余价值进一步缩小**）。

---

## 交互帧模型与容量类（N1–N5；2026-10-17 复核 + 实测后新增）

> 本节条目来自一次**独立架构复核** + **真实宿主实测**（`docs/validation-realhost-frame-cost-2026-10-17.md`、
> `docs/validation-render-perf-2026-10-17.md`）。**N1 与 P1 并列最高**。所有收益标 **实测 / 估算 / 未测**。

### N1 — 交互帧模型：预算策略 / 分页语义 / pulled 页子集门（**与 P1 并列最高**）

- **现状/问题**：
  - **(a) 预算策略（顶点/tri 记账 vs 时间驱动）**：`FrameBudget` 是 **`pub` 字段**
    （`crates/cad-render-wgpu/src/lib.rs:1236`），默认 **8,000,000 顶点 / 2,000,000 三角形**
    （`crates/cad-render-wgpu/src/renderer_impl.rs:76-81`）。**同一预算在两个 fixture 上产出
    6.7× 不同的 `wait_ms`**（complex 19.382 vs debug 2.895，**实测**）⇒ 固定顶点/tri 记账与
    **实测**帧成本脱钩。改进方向：**按时间驱动**（先测量再定）。
  - **(b) 分页语义**：相机一变本帧即重启（`FrameKey` 含相机），每帧只呈现 **draw-order 前缀页**
    ——complex 每帧完成度中位 **4.6%**、356 帧仅 **2** 帧 `complete=true`（**实测**，见文首）。
    **产品决定（用户 2026-10-17，binding）**：**接受**运动过程中帧内容不全 ⇒ **不**做"运动帧按视口
    排序页内容"（该子项**已删**）；但分页重启语义须**文档化**，且 `over_budget` / `complete=false`
    必须**显式上报**（铁律 2）。
  - **(c) N1d — pulled/加速组织的子集门**：`use_pulled` 要求二者相等
    （`converted_accepted == converted_total`，`crates/cad-render-wgpu/src/renderer_impl.rs:4271-4276`），
    即**必须接受整场景**才启用 pulled ⇒ 生产分页帧一律 `accelerated=false`。N1d = 让 pulled/加速组织
    **只画一个已接受子集**（而非要求整场景被接受）。**N1d 同时是 N5 的前提**。
    - **实测重排（2026-10-17，P0 落地后）**：工作顺序变为 **(1) 每帧 CPU plan 成本** → **(2) N2 流水** →
      **(3) N5 indirect multi-draw**。N1d 是 **N5 的结构性前提**，故随 N5 一并**降级**（N5 前提已被测量、
      非瓶颈，见 §N5）；**未取消**。**(1) 每帧 CPU plan 成本已落地（见 §N1e）**。
    - **更新（2026-10-17，§N1e 后续落地后）**：工作顺序 #1 及其 **uniform/encode 后续**均已落地（见 §N1e）；
      **N2 已按实测重挂**（运动帧 ≈8 ms 在 60 fps 预算内，主导残余是 GPU 执行，见 §N2）；**N1d/N5 的降级与重促
      条件不变**。
- **N1a — 时间驱动自适应页预算：已落地（2026-10-17）**。纯控制器
  `crates/cad-render-wgpu/src/pacing.rs`（`PacingState`/`Sample`/`PacingMode`/`step`/`apply_scale`，
  12 个纯测试）。常数（**实测**实现值）：`target_ms = 13.0`（默认，可由
  `Renderer::set_frame_budget_target_ms` 覆盖）；信号 = 本帧**实测** `cpu_ms + wait_ms`
  （**不是** tick 间隔——被 vsync 量化、会振荡；**不是** `frame_total_us`/`present_us`——含预算控制不了的
  固定 apply/present 成本）；EWMA `0.25*new + 0.75*old`，**首个样本只播种**（第三个被观测帧前不收缩）；
  死区 `0.95–1.05 × target`；连续 **2** 帧超目标 ⇒ `×0.75`，连续 **8** 帧低于目标 ⇒ `×1.08`；
  `scale ∈ [0.05, 4.0]`（顶点与三角形同缩放、保持比例，`max_bytes` 不动）；方向翻转清反向 streak；
  **接受 0 批次的帧不是观测**。预算语义：`Renderer::frame_budget` 是**未 pacing 的上限**，
  `active_frame_budget()` 是三个分页规划点使用的**活动预算**，**pick/snap 不 pacing**。三态：
  `pacing=active`（渐进 + 完成信号）/ `pacing=at-ceiling`（非渐进）/ `pacing=disabled-no-completion-signal`
  （wasm/无信号；预算与静态默认**逐位一致**）。**进度下限**：cursor 批次能装进**上限**时一页至少接受 1 批，
  被下限强制的一页报超预算、原因 `pacing-floor`；只有单个批次**超出上限**才保留既有显式错误。
  上报为**追加式**（`budget_scale`/`budget_target_ms`/`budget_cost_ema_ms`/`budget_verts`/`budget_tris`/
  `pacing`，位于 `pulled={}` 之后），`over_budget` 字面含义与诊断码 `render.frame_over_budget` **不变**。
  **同轮修掉的预存缺陷（先于本轮存在）**：图像只在清屏页绘制，而此前只按**剩余**页预算记账 ⇒ 几何在页 1
  饱和时被扣下的图像在续页**永不绘制**、"complete" 帧**静默缺图**；现在清屏页**先预留整幅图像记账**
  （paced 预算装不下全部图像时回落上限），几何再填剩余，**绘制顺序不变**（图像最后）。
  **真实宿主实测轨迹（本机原生 Wayland / Vulkan / Quadro P620 / 驱动 580.178.04 / `scale=1`，exit 0、
  无 panic）**：`complex-test.dwg` 790 帧全 `pacing=active`，**709 ms** 首帧离群把 `scale`
  **1.000 → 0.750 → 0.562 → 0.422 → 0.100**（seq 4–21）而 `wait_ms` **21.0 → 2.3 ms**，后段 `wait_ms`
  **7.7 → 11.3 ms**（seq 746–750）把 EMA 抬到 **13.87/14.67**、`scale` **0.336 → 0.252**（seq 751），
  稳定页最终 `complete=true`；`debug-default-fonts.dwg` 458 帧全 `pacing=active`，**46 ms** 离群把 `scale`
  **1.000 → 0.422**，随后慢升抬到上夹紧 **`budget_scale=4.000` / 32,000,000 verts**（seq 240）。
  逐条证据、测试矩阵与**未测边界**见 `docs/validation-frame-pacing-2026-10-17.md`、`docs/handoff.md`
  本轮条目。**未测**：wasm/浏览器真实运行时 disabled 路径、跨页 z-order、视觉/截图判定、GPU 时间、
  120 Hz/刷新率自适应。
- **N1a 后续 — 后缀提交不再重置 pacing 成本模型：已落地（2026-10-17）**。逐条证据见
  `docs/validation-pacing-suffix-commit-2026-10-17.md`（实现 commit `0fb2bc1`）。这是 N1a 的**收敛性**残余项：
  控制器此前**每次提交**都重置。
  - **症状（用户手动交互，实测）**：**全览拖动卡顿**，而**放大看细节反而顺滑**。机制上二者是同一现象的
    两面——全览帧画的是**整幅 draw-order 前缀页**（无剔除），放大后 **P1 块级剔除**把视口外工作消掉，
    帧就便宜。
  - **病根（实测）**：`Renderer::commit_upload` **在每一次提交上**重置自适应页预算控制器——**包括**
    引擎**每次相机 settle** 发出的**后缀专用 overlay 提交**（延迟参考网格刷新：仅相机范围变化布防 settle，
    相机静默满 **250 ms** 后 overlay 后缀被重准备并以**后缀提交**提交，`cad-engine/src/render/mod.rs:309-323`
    的 `settle_camera_if_due`，超时常量 `mod.rs:165`；该后缀提交**只追加一条 overlay 批次**）。⇒ 连续交互
    期间**每次 settle 都把已学 `scale` 打回 `1.0`**，控制器**永不收敛**、渐进页**始终顶点过重**（实测全览帧
    `draw_calls≈2720`、`gpu_ms 15.80`、`wait_ms 16.15`、`over_budget=true` 69/75）。
  - **改动**：`commit_upload` 只在**基座变更**时重置——`if keep_prefix == 0 { self.reset_pacing(); }`
    （`crates/cad-render-wgpu/src/renderer_impl.rs:2284-2303`）。**判据 `keep_prefix == 0` 正是 N1f
    pick 表增量路径所用的同一判别式**（`keep_prefix > 0` = 保留了真实前缀 = 后缀提交）。
    - **仍重置的集合**：**基座变更**（`keep_prefix == 0`）、`clear_batches`、**新设备世代**、**设备丢失**、
      **图像重传**（保留重置——它替换**整幅驻留图像集**、**从不**是增量后缀）。
    - **`reset_pacing` 只清成本模型状态**（`renderer_impl.rs:2776-2779`，用 `pacing::PacingState::new`
      重建）：`scale` / `cost_ema_ms` / `over_under_streak` / `seeded` 等**学习状态**。
      **restart-pending 是另一路信号，对后缀提交被保留**（后缀提交不调用 `reset_pacing`，故一次真实重置
      置起的 pending 仍**正确保持**）。
    - **未引入 dirty/invalidation 标志**：源码注释明确这是**启发式成本模型**、**不是**正确性状态，故
      **刻意不**建模为失效标志，重置**不与任何其它状态耦合**（**铁律 4**：不新增第二套失效真相）。
  - **实测（本机原生 Wayland / Vulkan / Quadro P620 / `scale=1`；同 UID、无 root/sudo、无屏录、无虚拟显示；
    合成 `YACR_PROBE_POSTLOAD_PAN` 手势）**——`budget_scale` 分布 before → after：

    | 运行 | 帧数 | `=1.000` | `<1.0` | `>1.0` | EMA 重置 |
    |---|---:|---:|---:|---:|---|
    | before（用户手动捕获固定副本 `janky-postpickfix-21h58.log`） | 75 | **35（47%，撞顶）** | 40 | 0 | —（`budget_cost_ema_ms=0.00` 出现在 15 帧＝反复重学） |
    | after（探针运行） | 1888 | **6（仅前 6 帧）** | **1758（93.1%）** | **125（6.6%）** | **1**（仅初始） |
    | 独立复跑（编排者） | 1842 | 6 | **1808** | 28 | **2**（初始 + base 变更） |

    **后缀提交逐次保持已学 `scale`**：`0.435 → 0.435`、`0.485 → 0.485`、`0.471 → 0.471`（**实测**）。
    after 的 `>1.0` 是**合法**的——合成 pan 把图拖进**空白区**，廉价相位里控制器**合理地爬回 1.0 以上**
    去寻 13 ms 目标；after 的 EMA **只重置一次**（初始）。
  - **栅栏（已落地，名称即契约；`crates/cad-render-wgpu/tests/pacing_commit_suffix.rs`，3 个 GPU fence）**：
    `suffix_only_commit_preserves_pacing_knowledge_while_a_base_change_resets_it`（后缀提交**逐位**保持
    `pacing_scale()`/`pacing_cost_ema_ms()`，基座变更仍重置）；
    `repeated_suffix_commits_do_not_return_the_scale_to_one`（**六轮**后缀提交从不把 `scale` 打回 `1.0`）；
    `suffix_commit_after_an_over_target_observation_keeps_the_shrunk_budget`（超目标观测之后的后缀提交保持
    `budget_scale < 1.0` 且 `budget_vertices` 低于上限）。
  - **既有 fixture 披露**：`headless_render::scene_mutation_resets_pacing_knowledge`（`tests/headless_render.rs:603`）
    原以 `renderer.upload(…)` 作"场景变更"，但 `upload` 的提交是 `keep_prefix = self.batches.len()`——一次
    **后缀追加提交**、**不是**基座变更——故在修正后的规则下**不再**重置。该 fixture 现改为执行**真实基座变更**
    （`prepare_upload_batches` + `commit_upload(staged, 0)`）；其**契约不变**（"一次**基座变更**的场景变更重置
    成本知识"）。**这是本轮唯一被修改的既有测试**。
  - **混淆项（不得软化）**：**before/after 的每帧 GPU 数字不可比**——baseline 窗口**最大化**，探针运行用
    **926×266** 窗口（被 `YACR_PROBE_NO_MAXIMIZE=1` 强制）；且驱动是**合成 DOM 手势、不是真实鼠标**。
    ⇒ **不得**给出"倍数下降"结论，**不得**给出**人工交互判定**（**归用户**）。**单机/单适配器**，**不外推**。
  - **优先级**：已落地（N1a 的收敛性残余项）。
- **N1e — 每帧 CPU plan 成本：剔除感知的分段页规划：已落地（2026-10-17）**。这是 P0 结论工作顺序的
  **#1**（"每帧 CPU plan 成本"）。逐条证据见 `docs/validation-plan-cost-2026-10-17.md`。
  - **病根（实测）**：`plan_ordered_page` 本应在首个越预算批处停下，但**完全被剔除的批计零**、产品里
    `max_bytes = usize::MAX` ⇒ 零计费批**永不失败**；剔除清空视图时线性游走会跑到有序表末尾（complex-test
    166,346 条）只为接受一切并画一个 run。实测病态页 `plan_ms` 中位 **10.5** / p90 **18.1**、`cpu_ms` **22.4**。
  - **机制**：剔除无关的 `BatchFacts`（**每个批集变更只算一次**，复用既有 `draw_plan = None` 失效，
    `crates/cad-render-wgpu/src/plan.rs:896-995`）+ 每次 `ordered` 重建时的 `OrderedPageIndex`（批→位置逆表
    `batch_pos` + **字节前缀和** `bytes_prefix`，`plan.rs:997-1047`，随既有重启清扫重建
    `crates/cad-render-wgpu/src/renderer_impl.rs:4409`）+ `plan_ordered_page` 只走**活动批**（可见 chunk 的批
    ∪ `always_active`）并由它们的**空隙**导出被接受前缀，字节边界用**前缀和二分**（`plan.rs:1080-…`）。
  - **回退阈值**：`!cull.any_culled()` 或估计活动批 **> 4096**（`SEGMENTED_ACTIVE_THRESHOLD`，
    `plan.rs:894`）⇒ 原线性游走，**不变**（缩放出去的大页与改动前完全一致）。
  - **字节计费仍按批粒度**：被完全剔除的已转换批**仍计字节**（**不是**免费的）；`always_active` **刻意不**是
    `batch_has_visible_work`（`plan.rs:912-916`）。
  - **附带修订（用户批准）**：`plan_ordered_page` 与 `plan_from_gpu` 统一失败时的 `usage` 契约——顶点/三角形
    失败**保留失败批的字节**；该字段**不**出现在 probe 输出。
  - **实测（本机真实宿主；单机单适配器，见证据文）**：complex-test 稳定 1-draw 页 `plan_ms` 中位/p90
    **10.747/18.084 → 1.221/2.749**、`cpu_ms` **22.42/30.78 → 13.65/21.94**；debug 稳定 1-draw 页 `plan_ms`
    **0.677/0.759 → 0.114/0.213**。**独立复跑**：接受整条有序表的帧 `plan_ms` 中位 **10.547（n=47）→
    1.168（n=5）**，深剔除帧 **10.541 → 0.588**。
  - **测试（实测）**：`--lib` **192 passed / 3 ignored**（**7** 个新 planner 测试，含 4000 例随机差分）；
    `--test headless_render` **35**、accelerated_tier **23**、pulled_prefix_equivalence **9**、render_effects
    **45**、pick_id **32**。
  - **边界（不得软化）**：complex-test `plan_ms` **p90**（跨运行 2.53–2.75）**正落在 2.5 ms 边界上**
    （中位达成，p90 勉强）；**paced 帧数字被墙钟轨迹混淆**，只有稳定 1-draw 页是预算无关的干净 before/after；
    稳定帧仍付**重启清扫**（每次相机移动 O(166,346)）。**更正（2026-10-17，N1e 后续落地后）**：此条原写稳定帧
    另付 `uniform_ms`（~6 ms）/`encode_ms`（~5.2 ms）/`submit_ms`（~5 ms）——`submit_ms` 的 **~5 ms 估算
    是错的**（settled 页实测 **0.206 ms**，提交从来不是因素），`uniform_ms`/`encode_ms` 已由后续修复（见下）；
    **重启清扫**仍未修。
  - **N1e 后续 —— 每帧 uniform/encode 成本：超集安全的编码跳过 + slot 压缩：已落地（2026-10-17）**。
    这是 N1e 落地后 CPU 侧的**下一个残余项**（`uniform_ms`/`encode_ms` 两个每帧循环与重启清扫是**同一种病**）。
    逐条证据见 `docs/validation-uniform-encode-skip-2026-10-17.md`（实现 commit `1058af2`）。
    - **机制**：`BatchFacts` 增一个**紧凑每批 run class**（一个缓存驻留字节，`0` = 不拥有 run）+ `legacy_batches`
      列表（`crates/cad-render-wgpu/src/plan.rs:927-941`）；每帧 `EncodeEmitMask`（`plan.rs:1048-1120`）标记
      参与 encode 的批——无 run 的批、有 run 且该 run 存活本帧剔除的批、加恒活动的 legacy quad/mesh 集。
      掩码是 planner 活动集的**超集**，**绝不**丢 draw 或 flush；一张 run 全被剔除的**已转换**批在 encode 循环里
      是纯 no-op，在其 ~200 B `GpuBatch` 被读取**之前**即跳过（`renderer_impl.rs:5220-5235`）。
    - **slot 压缩**：只有无 run 的批拿每批 uniform slot；per-stream/chunk/conic slot 只为可见 chunk 写，经运行
      slot 计数器压缩（settled 页 `total_slots` **41,322 → 1**）。每个存活条目的 96 B uniform 由**同一次调用、
      同一输入**产出，故字节**逐字节相同**；上报计数从 class 字节**精确复现**。
    - **刻意不跳过**：透明 pass（转换要求不透明批 ⇒ 每个透明批都是真实发出的工作）与加速计数循环
      （`use_pulled` 要求无剔除）。两个主循环仍迭代 Θ(accepted)/Θ(opaque)——每个被跳过条目只花**一次字节读**，
      因为上报计数定义在整个被接受集上。
    - **实测（settled 1-draw 页；中位 / p90）**：

      | fixture | `plan_ms` | `uniform_ms` | `encode_ms` | `submit_ms` | `cpu_ms` |
      |---|---|---|---|---|---|
      | complex-test | 10.747 / 18.084 → **1.130 / 1.341** | 6.071 / 6.862 → **0.151 / 0.197** | 5.365 / 5.598 → **0.656 / 0.802** | 0.206 → 0.186 | 22.42 / 30.78 → **2.180 / 2.511** |
      | debug-default-fonts | 0.677 / 0.759 → **0.132 / 0.207** | 0.135 → **0.026** | 0.247 → **0.090** | 0.140 → 0.224 | 1.244 / 1.367 → **0.522 / 0.796** |

      **独立复跑**（complex-test，另一宿主运行）：settled 帧 **50 → 128**，`plan_ms` **10.499 → 1.131**、
      `uniform_ms` **6.056 → 0.155**、`encode_ms` **5.335 → 0.652**、`submit_ms` 0.207 → 0.182、`cpu_ms`
      **22.158 → 2.207**；两次运行都出现同一病态页签名（`draw_calls=1 verts=12 tris=4 cull_chunks=1/9362`）。
      settled 页的确定性字段 before/after **逐字段一致**（仅 `budget_verts`/`budget_tris` 因墙钟相关 scale 移动）。
    - **测试（实测）**：`--lib` **195 passed / 3 ignored**（**3** 个新纯测试：
      `encode_skip_is_superset_safe_and_preserves_counters` 4000 例超集安全差分、
      `fully_culled_batch_with_legacy_quads_is_still_processed` legacy-quad 回归、
      `skipped_uniform_slots_leave_surviving_bytes_bit_identical` 2000 例 uniform 字节同一）；
      `--test headless_render` **35**、accelerated_tier **23**、pulled_prefix_equivalence **9**、
      render_effects **45**、pick_id **32**。
    - **边界（不得软化）**：**只有 settled 1-draw 页是干净的 before/after**（paced 帧被墙钟轨迹混淆：after 运行
      的 paced 页接受约 1.6× 顶点 / 2× draw，按单位发出工作计 encode/submit 成本**没有**增加）；**full-view
      （无剔除）帧仍未修、未测**（跳过只在有剔除时生效）；**GPU 执行份额未归因**、**是否还有值得做的 CPU 工作
      未测**；字段一致是**日志级**证据，不是像素级。
- **预期收益（诚实）**：**(a)** 时间驱动预算使**每帧预算与实测成本对齐**（收益**未测**，需 P0 的
  GPU 时间 + 跨 fixture 校准）——**N1a 已把"对齐"落地为活动预算 + 逐帧上报**，但其**性能收益**仍**未测**
  （无 GPU 时间，且视觉/截图未判读）；**(b)** 纯文档/显式上报，无性能收益；**(c) N1d** 让加速档能在**分页
  帧**上生效——是 N5/P7 的**结构性前提**，其自身收益**未测**。
- **前置条件/风险**：(a) 改预算策略触及帧卫生（pixel/顺序不变性），须 fence 预算可逐帧调整且不改
  像素；(c) 改 `use_pulled` 判据须保证 **pulled 子集页与通用页逐像素一致**；(b) **不得**引入视口
  排序（产品已决）。
- **需要的决策（产品/UX）**：预算是否改为**时间目标**（例如每页目标 `ms`），以及超时是否仍显式上报。
- **相关契约/栅栏**：`budget_is_adjustable_per_frame`、`accelerated_tier_engages_on_paginated_frames`、
  `pulled_page_subset_matches_universal_pixels`。
- **优先级**：N1（**最高，与 P1 并列**）。其中 **(c) N1d 已按实测降级**为 N5 的前提（见 §N1d 与 §N5）；
  **(a) 的"时间驱动预算"已由 N1a 落地**，但其性能收益仍**未测**。

### N1f — 提交路径的 pick 表增量更新（**已落地**）

- **已落地（2026-10-17）**。逐条证据见 `docs/validation-pick-table-incremental-2026-10-17.md`
  （实现 commit `0f5d5c7`）。这是 N1e 之后的**下一个 CPU 残余项**，但病在**提交路径**而非每帧循环：
  `Renderer::rebuild_pick_table` 在**每次**批次/图像变更时**整表重建**（~16 万批的图 ⇒ 一次后缀提交也要
  付整幅重建成本）。**触发场景**：用户报告的交互缺陷——拖动/缩放时画面冻结、松手后跳变
  （`fixtures/complex-test.dwg`）。
- **病根（实测）**：后缀提交 `pick` = **1,449,615 / 1,487,525 / 1,454,073** µs（7 次提交），
  `input_us` 排队延迟中位 **1,045,445** µs（n=18），拖动期间 `cad-frame` 冻结。**同夹具在近期 perf 轮次
  之前** `pick=1530380` µs ⇒ **预存缺陷，非回归**。
- **机制**：`rebuild_pick_table`（`crates/cad-render-wgpu/src/renderer_impl.rs:4082`）**在满足条件时走
  增量后缀路径**——`keep_prefix > 0` **且** **无驻留图像** **且** `pick_table_covers_prefix(keep_prefix)`
  （O(1) 覆盖证明，`:4152`）；增量动作为 `PickTable::replace_suffix`（`crates/cad-render-wgpu/src/plan.rs:3672`）
  ——**退役**被丢弃后缀的 key、**只追加**新条目（`:4105-4113`）。计数 `pick_table_incremental_extends`
  （`lib.rs:1597`）。
- **回退（一切其余情形 ⇒ 整表重建）**：base 变更（`keep_prefix == 0`）、图像重传（`self.images` 非空）、
  设备丢失 / `clear_batches` 复位、前缀未被覆盖；计数 `pick_table_full_rebuilds`（`lib.rs:1595`，
  与 `pick_table_incremental_extends` 互补）。
- **有效性条件**：pick key 在一个设备世代内**单调分配、永不重用、永不重编号**（`pick_key_counter` 只在
  `reset_pick_keys` 重启，`renderer_impl.rs:4164`，**恰在批次被清空时**发生）⇒ 被保留前缀之外的条目**不变**、
  追加 key **严格更大** ⇒ 增量结果与 fresh `PickTable::build` **完全一致**（条目 / `keys_for_source` /
  截断碰撞集合 / `max_key`）。
- **配套改动**：`PickTable::build`（`plan.rs:3608`）**消耗**条目（一次克隆而非两次；渲染器仍持自己的副本，
  **不可能**自引用借用）；实体标志身份索引**惰性构建**（`source_index: Option<SourceIdentityIndex>`
  `plan.rs:3582`，仅 `entity_flags_request` 非空时构建，`apply_entity_flags` 首次查询前经
  `ensure_source_index` `plan.rs:3791`）；`detect_truncation_collisions` 被保留的 `truncated_index`
  （`plan.rs:3589`）+ 增量 `update_collisions`（`plan.rs:3905`）取代。
- **实测（本机真实宿主）**：后缀提交 `pick` **1,449,615 / 1,487,525 / 1,454,073 → 866 / 433 / 541** µs；
  base-load `pick` 1,310,440 → 1,021,559 µs；`input_us` 中位 **1,045,445（n=18）→ 29（n=1185）** µs
  （p90 41、max 116）；`cad-frame presented` 由**冻结**变为 **2051–2054 行连续**。**独立复跑**：后缀提交
  `pick=866/433/541` µs、`input_us` 中位 29 µs、**2054** 行、exit 0。
- **测试（实测）**：`--lib` **200 passed / 3 ignored**（含
  `incremental_pick_table_matches_a_fresh_build_across_commit_sequences`：**128 种子 × 40 步**差分，
  每步断言与 fresh `build` 完全相等 + 重复 key 事务式拒绝）；`--test pick_id` **32**、`pick_hidden_flags`
  **8**、`headless_render` **35**、`accelerated_tier` **23**、`pulled_prefix_equivalence` **9**、
  `render_effects` **45**；`fmt`/`check --all-targets`/`clippy -D warnings`/wasm check clean。
- **三处更正 / 已否决方案（记录）**：① `append=97` 是**微秒**（`apps/app-linux/src/host.rs:2086-2093`），
  真实追加量是**一个**批次；② 因此「追加批次全为 `PICK_KEY_NONE` 时跳过重建」**无生产者**（key 恒在
  `1..=MAX_PICK_KEYS`，`PICK_KEY_NONE`（0）从不进表）；③ **pick 时 dirty/惰性重建：已否决**（急切成本已
  ≈0.4–0.9 ms；`apply_entity_flags` 必须在提交时运行——`runtime.rs:699`；`pick_collisions`/`pick_table_len`/
  `pick_key_for_batch` 是 `&self` 访问器；新 dirty 标志 = 第二套失效真相，**铁律 4**）。
- **重复提交的发现（记录）**：生产者是**延迟参考网格刷新**——仅相机的范围变化布防 settle
  （`cad-engine/src/render/mod.rs:703-706`），相机静默满 **250 ms**（`mod.rs:309-323`、常量 `mod.rs:165`）后
  overlay 后缀被重准备（`cad-app/src/render_scene/overlay.rs:452`）并以**后缀提交**提交
  （`runtime.rs:656-663`，`base_changed=false`）⇒ **相机变化每次 settle 提交一次，不是每帧**。原始 7 次
  settle 被 1.5–3.8 s 的提交本身放大；修复后 3 手势 = 3 后缀提交且画面连续。**完全避免 settle 提交**需
  相机无关网格（无限 shader 网格）或大得多的预扩展网格——**设计改动**；**刻意保持现状**（记录理由）。
- **仍开放（未修）**：base 变更后**第一次**后缀提交仍付整幅 pulled 重建（`pulled=2,506,646` µs，
  `pulled_reuses=0`）；后续后缀提交复用（`pulled=590/731` µs）；base load 本身
  `pulled=2,518,896` + `pick=1,021,559` µs（一次性）。**均非** pick 表问题。
- **边界（不得软化）**：**不得从日志得出人工交互判定**——验收用**合成 DOM 手势驱动**
  （`YACR_PROBE_POSTLOAD_PAN`，**不是**真实鼠标），日志只证**提交路径便宜**且**帧持续呈现**，不证
  人感知的拖动流畅；**人工判定归用户**。`debug-default-fonts.dwg` 相机路径**未测**（pan 驱动两次报
  「concurrent resize … invalidated the report」），只测到 base-load 提交 `pick=4675` µs 与 exit 0。
  单机/单适配器，**不**给 vendor/视觉/性能结论。
- **优先级**：已落地（N1e 之后的提交路径残余项）。

### N2 — CPU/GPU 流水（pipelining；消除每帧阻塞 poll）

- **现状/问题**：每帧末尾有**阻塞式 device poll**（`crates/cad-render-wgpu/src/renderer_impl.rs:5615` 的
  `device_scope_outcome`；`PollType::Wait` 在 `:5790`），它把 **CPU(n+1) 串在 GPU(n) 之后** ⇒ 帧时间 ≈
  `cpu + wait`。`wait_ms` 在 **P0 落地后实测为真实 GPU 执行**（`gpu/wait` 比值 **0.93**，见 §P0 与
  `docs/validation-gpu-time-2026-10-17.md`）。
- **本轮重估（2026-10-17，§N1e 后续落地后实测）——N2 的旧理由已不成立**：
  - **运动（paced）帧现在约 8 ms**：`cpu_ms ≈ 3.6` + `wait_ms ≈ 4.2`（**实测**，逐运行被墙钟轨迹混淆，
    但两相位现均小）⇒ **已在 60 fps 预算（16.7 ms）之内且有裕量**。
  - **主导残余是 GPU 执行**：`gpu_ms ≈ 3.95`（**实测**，见 §P0），约为 `wait_ms` 的 **93%**；**任何 CPU 侧
    杠杆（含 N2）都不减少 GPU 执行**。
  - **N2 只能买延迟裕量，不是补预算缺口**：流水化把 `cpu` 与 `gpu` 重叠，或可把运动帧从 ≈8 ms 压到
    **≈4–5 ms**（**估算，未测**）。
- **预期收益（估算，未测；已按本轮数字更新）**：帧时间 ≈ `max(cpu, wait)` ⇒ 运动帧 ≈ **8 → 4–5 ms**
  （**估算**），**零 GPU 工作改动**；**这是估算，不是测量**。
- **旧重开条件（写于 2026-10-17 延后时，保留供追溯；已逐条标注现状）**：
  - **① P0 的 GPU 时间显示 paced 帧上 `exec ≪ wait_ms`** —— **已答为假**（实测 `exec ≈ wait_ms`，比值 **0.93**；
    present/queue 只占约 6–7%）⇒ 「延后阻塞 poll 近乎免费」不成立。
  - **② paced 帧持续超目标且 `cpu_ms ≥ 帧时间 ~40%`** —— **已失效（moot）**：§N1e 后续落地后运动帧 ≈8 ms
    **在预算内**，此条件不再成立。
  - **③ 120 Hz（8.3 ms）成为产品需求** —— **仍有效**（唯一成立的重开触发）。
- **本轮决定（2026-10-17 重估）：重挂（re-gated）于产品需求，不是按旧理由开工**：
  - 旧判定（2026-10-17 P0 落地后）曾为"满足 ②③ ⇒ 重开评估、工作顺序 **#2**"——该判定**已作废**：② 现不成立
    （运动帧在预算内），故 **N2 不再是 #2 的既定下一步**。
  - **重开触发**：仅在出现明确产品需求时重开——例如 **120 Hz 目标**，或**更重的图纸**使运动帧重新越过预算。
    **记录的重开条件须按上述这些数字重新评估**（①② 已按此重评）。
- **前置条件/风险**：须**保留 device-lost / `FrameError` 的分类**（不得因异步化把错误吞成空成功）；
  需**有界队列深度**（内存上界）；需与 **pick readback** 交互（读回与在飞帧的同步）。
- **需要的决策**：队列深度与在飞帧数上界（内存权衡）；**以及是否真有需要 N2 的产品需求**（120 Hz / 更重图纸）。
- **相关契约/栅栏**：device-lost/`FrameError` 分类保持；有界队列深度；pick-readback 交互 fence。
- **优先级**：N2（**重挂（2026-10-17 重估）**：旧理由已不成立——运动帧 ≈8 ms 在 60 fps 预算内，主导残余是
  **GPU 执行**（CPU 杠杆不可及）；**仅在 120 Hz 或更重图纸使运动帧再次越预算时重开**）。

### N3 — 显存容量（4 GB 卡）：显式建模取代 panic

- **现状/问题（实测）**：`fixtures/complex-test.dwg` 曾 **panic**：`wgpu error: Out of Memory`，
  发生在 `Renderer::rebuild_pulled` → `Device::create_buffer`（backtrace 与 exit 101 见
  `docs/validation-realhost-frame-cost-2026-10-17.md`）。**根因**：pulled rebuild **一次性**分配整幅
  图纸的 pulled payload（**实测需要 2,281,858,648 B**）；宿主请求 `Limits::default()` ⇒ 设备
  `max_buffer_size = 268,435,456 B（256 MiB）`。**注意**：**adapter** 上限是 **4,292,870,144 B**，
  故这是**请求限制（requested-limits）**，**不是硬件限制**。另有同轮 harness **legacy 逐批** OOM 记录
  （`docs/validation-render-perf-2026-10-17.md` §5）。
- **修复（2026-10-17 早先落地，已实现）**：**容量门 + 显式三态**（`PulledState` 枚举：
  `Active`、`SkippedInsufficientVram{required,budget}`、`SkippedOutOfMemory{..}`、
  `SkippedUnsegmentable{..}`、`SkippedNoConvertedGeometry`、`Unavailable`；
  `crates/cad-render-wgpu/src/lib.rs:183-235`；门 `pulled_capacity_fits`
  `crates/cad-render-wgpu/src/renderer_impl.rs:5490`，用法 `:3005-3017`）现**跳过** pulled 而**不 panic**：
  - complex-test **现可打开**（**exit 0、0 panic、371 渲染帧、entityCount 44,591**），
    `pulled=skipped-insufficient-vram(required_bytes=2281858648,budget_bytes=268435456)`（**实测**）；
  - `debug-default-fonts` 仍 `pulled=active`（180,550,060 ≤ 256 MiB）。
- **后续（2026-10-17，同轮已实现）：limits 协商 + 分配失败显式化**。
  - **limits**：`negotiated_device_limits`（`crates/cad-render-wgpu/src/headless.rs:232`）以
    `Limits::default().or_worse_values_from(adapter)` 为底，**只**把 `max_buffer_size` 抬到 adapter 值；
    本机实测 requested = granted = **4,292,870,144 B**、`degraded=false`。
    `max_storage_buffer_binding_size` **刻意**保持 WebGPU 默认 **134,217,728 B**（adapter 报
    2,147,483,644 B）：抬高会让 pulled 分段器产出约 2 GiB 单次分配，`create_buffer` 失败返回无效
    handle，后续使用报 **validation**（OOM-only 作用域捕获不到）⇒ abort exit 101；锁回默认后重建成功、
    **像素不变**。
  - **分配失败判定**：pulled 构建置于嵌套 `Validation` + `OutOfMemory` 作用域，逐分配先验证创建再上传，
    首次失败即丢弃半成品并回退通用 per-run；新增显式状态 **`skipped-allocation-failed`**
    （与 `skipped-out-of-memory` 分开，**不**把非 OOM 错误说成显存耗尽）。
  - **实测（真实宿主）**：`complex-test.dwg` 由 `skipped-insufficient-vram` 变为 **`pulled=active`**
    （`entityCount=44591`、settled 帧 `complete=true`、exit 0、无 panic/`is invalid`）；
    `debug-default-fonts.dwg` 仍 `pulled=active`（`entityCount=10656`）。
  - **未测**：真实驱动/显存 OOM 在本机不可模拟（新设备测试触发的是无效 handle/validation 路径）；
    其它 GPU/vendor 未验证。逐帧证据见 `docs/handoff.md` 本轮条目，政策见 ADR 0013。
- **诚实边界（必须记录）**：① 上述均为**本机判定**（真实宿主，单适配器）；② 预算是**单缓冲**限制、
  却拿去和**总 payload** 比较 ⇒ **刻意保守**（limits 抬高后仍保留，作为最后一道安全网）；
  ③ limits 协商后 `complex-test.dwg` 的 pulled 已可用，但**任意图纸**是否可用仍取决于单次分配 payload
  与设备 `max_buffer_size`——**不是**“pulled 已通用可用”的结论。另：`prepare_us` 字段单位是**微秒**
  （早先一条注记误读为"秒"，**已更正**）。
- **预期收益**：无性能收益；**正确性/容量**——用**显式建模取代 panic**（铁律 2）。
- **前置条件/风险**：容量门与 limits 协商已落地；仍开放的候选是**共享顶点 arena**（降低单次分配）。
  保守预算的语义须保留并在兼容表中显式。
- **需要的决策**：**已决（2026-10-17）**：`max_buffer_size` 按 adapter 协商、
  `max_storage_buffer_binding_size` 封顶 WebGPU 默认；仍开放：是否改**共享顶点 arena**以让 pulled
  覆盖更大图纸。
- **相关契约/栅栏**：三态显式建模（**不 panic**）fence；pulled 预算保守性 fence；容量门测试
  （`crates/cad-render-wgpu/src/pulled_budget_gpu_tests.rs`）。
- **优先级**：N3（高；容量/正确性）。

### N4 — id-pass 覆盖集 parity（id pass vs color page）

- **现状/问题（源码核对）**：**id pass 用的是 upload-order 前缀**（`crates/cad-render-wgpu/src/pick.rs:488-490`，
  `plan_from_gpu(&self.batches, &self.frame_budget)` 按批次顺序、非渐进），而**color page 用 draw-order
  前缀**（见文首分页语义）⇒ **拾取覆盖集与可见页覆盖集可能不同**。
- **预期收益**：无性能收益；**一致性/正确性**——避免"看得见/拾得到"错位。
- **前置条件/风险**：须**让二者覆盖同一 accepted 集**，或**显式记录该已知差异**（二选一，不得静默）。
- **需要的决策**：对齐二者，还是把差异显式建模为已知限制。
- **相关契约/栅栏**：fence `id_pass_covers_the_same_accepted_set_as_the_color_page`（**新增**），
  或显式记录为已知差异。
- **优先级**：N4（中；一致性）。

### N5 — capability-gated indirect multi-draw（含用户 2026-10-17 指令）

> 本条吸收 P6 原 indirect 部分 + 用户 2026-10-17 指令：indirect/multi-draw 只能作为**协商的可选能力 +
> 显式回退**，**不得**成为架构依赖，且**须有 ADR**。

- **现状/问题**：把已接受子集编码为 **indirect draw buffer**、一次 `multi_draw_indexed_indirect` 提交。
  wgpu 30.0.1 的 `draw_indirect` / `multi_draw_*` API 齐全，仅受 `DownlevelFlags::INDIRECT_EXECUTION`
  门控；**WebGL2（GLES 3.0）根本没有 draw-indirect**；**未模拟**的 `MULTI_DRAW_INDIRECT_COUNT`
  （DX12 / Vulkan 1.2+，native-only）缺失时 wgpu 把 `multi_draw_*` 模拟为**一串 `draw_indirect`** ⇒
  **不省 CPU 提交**。**仓库事实**：两处设备创建点都用 `DeviceDescriptor { ..Default::default() }`
  （`crates/cad-render-wgpu/src/headless.rs:155`、`native_presenter.rs:491`），**零 feature 请求**。
- **预期收益（诚实上限）**：**提交侧收益很弱**。ADR-0007「P3b」A/B 实测 **≈0.16–0.6 µs CPU/draw**；
  本轮实测 **`submit_ms` = 3.412 ms / 3,027 draws**（文首）⇒ indirect 最多省 **≈10–12% 帧时间（估算）**，
  且**对主导相 `wait_ms` 19.4 ms 零影响**。**这是估算，不是测量**；GPU 时间**未测**（P0）。
- **P0 落地后判定（2026-10-17，实测）——前提被测量，非瓶颈**：每帧 CPU 相位（中位份额）实测 `plan` **48–56%**、
  `uniform` 7–27%、`encode` 6–24%、`submit` 1–18% ⇒ **CPU 提交成本不是瓶颈**，`encode`/`submit` 都不主导，
  且 N5 **碰不到** `plan_ms` 与 GPU 执行（`gpu_ms` 实测中位 3.948 / 2.643，见 P0）。⇒ **N5 降级（demote）而非取消**，
  在新的工作顺序里排 **#3**（在"每帧 CPU plan 成本"与 N2 之后）。**#1 及其 uniform/encode 后续均已落地（见 §N1e）**。
  - **一致性更新（2026-10-17，§N1e 后续落地后）**：上述相位份额是 **N1e 后续之前**的口径；settled 1-draw 页上
    `uniform_ms`/`encode_ms` 已大幅收缩（**6.071 → 0.151** / **5.365 → 0.656** ms），`submit_ms` 的 **~5 ms
    估算已更正**（settled 页实测 **0.206 ms**，从来不是因素）——见
    `docs/validation-uniform-encode-skip-2026-10-17.md`。⇒ **N5 的降级结论不变**（`encode`/`submit` 仍不主导，
    N5 仍碰不到 `plan_ms` 与 GPU 执行）。**N2 已按实测重挂**（见 §N2），故"在 N2 之后"只是**顺序**表述，
    不是 N2 即将开工。
  - **重促（re-promote）条件（实测门控）**：仅在 **N1d 已落地**（结构性前提）**且**在"每帧 CPU plan 成本"
    与 N2 流水之后、`encode_ms + submit_ms` 仍占据**剩余帧时间的显著份额**（实测）时才重促——即 N5 打击的相位
    必须真正成为残余瓶颈。在此之前**不开工**。
- **前置条件/风险（结构性）**：
  - 生产帧用**per-run draw + 逐 draw 动态 uniform 偏移**（`crates/cad-render-wgpu/src/plan.rs:246-260`、
    `:347-350`、`:418-422`），**单次 multi-draw 无法逐 draw 变 offset** ⇒ **必须 N1d 先行**（先有
    "pulled 子集页"模型）。
  - 未模拟 multi-draw 需 **`MULTI_DRAW_INDIRECT_COUNT` 设备 feature 请求**——这会改变已文档化的
    「从不向宿主设备请求 feature」规则；**必须是协商的可选能力 + 显式回退**（**三态：native / emulated
    / unsupported**，`docs/compatibility.md`）。
  - **Metal HAL 会静默 no-op**（`draw_indirect_count` 是空 TODO）；**WebGL2/GLES 没有
    `INDIRECT_EXECUTION`**（构造上不支持）。
- **需要的决策**：**是否请求 `MULTI_DRAW_INDIRECT_COUNT` feature**（架构决策）；且**须新写一份 ADR**
  规定**设备 feature / limits 的协商策略**（与**测量专用**的 `TIMESTAMP_QUERY` 设备是**同一政策轴**）。
- **相关契约/栅栏**：draw-order/并列（tie）parity；id-pass parity；`indirect_page_does_not_draw_rejected_batches`；
  续页**不得**重绘已画内容。
- **优先级**：N5（**已降级（2026-10-17，P0 落地后）**：前提被测量、非瓶颈 ⇒ 实测工作顺序 **#3**；
  capability-gated、ADR 门控、**需 N1d 先行**；重促条件见上）。

---

## 视觉/语义类（P2、P4）

### P4 — SDF 圆/弧（SDF circles/arcs）

- **状态（2026-10-15）：已实现（slice B，`1fc395e`）**，见 `docs/adr/0008-sdf-conic-rendering.md`。
  可 SDF 化的圆/弧（不透明、连续线型、slice-1 圆形）渲染为抗锯齿 SDF 实例（`ArcInstance` 72 B、
  三角形滤波覆盖峰值 1.0、解析 world-per-px ramp、强制包围四边形 `discard`）；发射门把 eligible
  conic 翻到 SDF，**虚线/椭圆/退化仍计数走弦**（资格在**变换后** conic 上判定，INSERT 椭圆保持
  诚实）；`pick_arc` 光栅化**同一 SDF**、±0.5 px。真实 P620/Vulkan 的覆盖-解析曲线、弦基线包含、
  id pass 一致性、flags、跨类强度一致性、绘制顺序、确定性/零重传与 before/after 视觉验收均已
  fence（见 ADR-0008 实施记录）。
- **后续（显式未做）**：**椭圆 SDF（slice 2）**、`PulledArc`（弧现走通用 per-run 残留路径）、
  **虚线/非连续 SDF 弧**、**线宽光栅化**、透视域精度；浏览器/WebGL2/Android 验收未做。
- **原设计意图（实施前评估，保留供追溯）**：把弧渲染为 **SDF 四边形**（实例化：
  center/radius/起始角/终止角/颜色/线宽）→ **分辨率无关、无需三角化、每类约 1 个 draw**，减少
  line 实例数。**语义像素变化**（SDF 抗锯齿**比折线弦更平滑**）需**视觉验收**——已获用户授权并
  fence（ADR-0008 §8(h)）；**不**做与旧折线逐字节一致声明。
- **相关契约/栅栏**：几何精度（覆盖-解析曲线）、弦基线包含、id parity、flags、跨类强度一致性、
  绘制顺序（`docs/render-order.md`）、视觉验收证据——均见 ADR-0008。
- **优先级**：已实现（其余见上「后续」）。

---

## 功能补全类（G1–G7）

### G1 — glyph outline-keyed instancing（文字轮廓实例化）

- **现状/问题**：文字轮廓**未按 outline 键做实例化**（ADR-0007:94-95）。**口径提醒（2026-10-17）**：
  早先"参考图 instanced 帧中 glyph mesh 占 **4,630 / 6,228**"是 **2026-10-15 的中间过程数字**，
  对本轮**分页**口径**已过时**，**不得**当作当前残留构成。
- **预期收益**：相同 glyph 轮廓跨实例复用 → 削减文字顶点/draw（**预期**）。
- **前置条件/风险**：**先插桩测残余构成与每页顶点大头（glyph 占比）**——当前每页残余是 `arc` 等
  合并 run、`legacy_opaque=0`（见文首），glyph 的真实占比**未测**；并需按 outline 键分桶的实例化
  路径；与 P2（文字 LOD）互补但独立。
- **需要的决策**：无（收益明确）。
- **相关契约/栅栏**：glyph 实例化与逐 glyph 路径的像素一致 fence。
- **优先级**：G1（残留 draw 主体，收益明确）。

### G2 — id-pass `FLAG_HIDDEN` 硬前置（hidden 切片落地前必须完成）

- **现状/问题**：`pick_*.wgsl` 写**原始 pick key**，不读标志表，故被 `FLAG_HIDDEN` 的实体会
  **继续可拾取并遮挡**其它几何（color pass 已 `discard`，id pass 没有）。当前 **hidden 生产者
  尚未接线**（`cad-host` 传空 `hidden`），缺陷暂不可见（`docs/picking-gpu-id-pass.md` §8）。
- **预期收益**：无直接性能收益；**正确性前置**。**hidden 切片落地前必须**让 id pass 也按 key
  查标志表并跳过 `FLAG_HIDDEN` 片段，否则违反铁律 #2 显式建模要求。
- **前置条件/风险**：依赖 G5（hidden 生产者接线）；本切片**不做投机实现**。
- **需要的决策**：无（硬性前置）。
- **相关契约/栅栏**：`id_pass_skips_hidden_fragments`；hidden 实体不可拾取的回归 fence。
- **优先级**：G2（hard prerequisite；随 hidden 切片一起闭环）。

### G3 — `MeshEdges` legacy arm no-op 哨兵缺口

- **现状/问题**：`RenderTopology::MeshEdges` 分支在 `plan.rs:363-372` 读取
  `batch.indices`，`None` 时直接 `return`（**什么都不画**）；合并路径同样保留该 no-op 哨兵。
  `GpuBatch.indices = None` 是**今日未被生产**的取值（ADR-0007:105-106、
  `docs/render-order.md:72`、`docs/picking-gpu-id-pass.md` §6）。缺口未闭合。
- **预期收益**：无直接性能收益；**功能补全**（MeshEdges 应产生真实 draw）。
- **前置条件/风险**：**不得静默改变**当前哨兵行为——若 producer 开始产出 `indices`，绘制语义
  必须显式定义并测试，不得"意外开始画"。
- **需要的决策**：MeshEdges 的预期绘制语义（线框？边 overlay？）。
- **相关契约/栅栏**：保留 `mesh_edges_batch_currently_renders_nothing`
  （`docs/picking-gpu-id-pass.md` §6），直到语义显式落地。
- **优先级**：G3（已知缺口，显式未支持）。

### G4 — 子元素/面级 flags（merged multi-source batch 按 batch 粒度着色）

- **现状/问题**：合并 run 不记录哪段 fragment 属于哪个 source，故对多源合并 batch 的任一
  source 打标志会给**整批**着色（`a_multi_source_batch_is_tinted_at_batch_granularity`，
  `docs/picking-gpu-id-pass.md` §0；ADR-0007:100-102）。面级（子元素）flags 不在静态合并流里，
  故仍走独立 overlay draw（用户已授权）。
- **预期收益**：无直接性能收益；**正确性补全**——逐 source/逐面着色。
- **前置条件/风险**：需在合并 run 中建立 **fragment→source 映射**（例如逐顶点/逐三角形记录
  source id 并查表），成本与内存需权衡；在此之前多源 batch 只能 batch 粒度着色。
- **需要的决策**：是否值得为逐面着色付出映射成本。
- **相关契约/栅栏**：`multi_source_batch_tints_per_source`（新）；在映射落地前保留现有
  batch 粒度行为与测试。
- **优先级**：G4（已知限制，显式未支持）。

### G5 — hover 生产者已接线；hidden 生产者仍未接线（受 G2 门控）

- **现状/问题（更新于 2026-10-17）**：`OverlayInputs::hovered` / `hidden` 机制与测试就绪。**hover 半已闭环**：
  宿主（`cad-host`）在空闲移动时用与点选相同的拾取路径产生**单个**实体级 ref，喂给 `OverlayInputs.hovered`
  （该输入在生产会话中由此**非空**；ADR-0012 §10 S2、`docs/input.md` §1.1.2、`docs/handoff.md` 2026-10-17 S2 小节）。
  **`hidden` 半仍未接线**：`cad-host` 仍传空 `SelectionSet`（`crates/cad-host/src/state.rs`），默认空
  （`docs/picking-gpu-id-pass.md` §0、§8；ADR-0007:97-102）。
- **预期收益**：无直接性能收益；**功能补全**——悬停高亮已在生产会话可用（hover 半）；隐藏实体在生产会话可用（hidden 半，未做）。
- **前置条件/风险**：**hidden 生产者接线必须先完成 G2**（id pass 必须先按 key 跳过 `FLAG_HIDDEN` 片段，否则隐藏实体
  仍可拾取，违反铁律 #2）。hover 半**不**依赖 G2（本切片**未**接线 `hidden`）。
- **需要的决策**：无。
- **相关契约/栅栏**：hover 进入生产会话的端到端 fence（已随 S2 落地，合成/本地）；hidden 端到端 fence 与 G2 的
  `id_pass_skips_hidden_fragments` 同步（未做）。
- **优先级**：G5（后续宿主切片）；**hover 半已闭环**，`hidden` 半受 G2 门控。

### G6 — R32Uint 选择是「来源引用，非运行时探测」

- **现状/问题**：`IdTargetFormat::R32Uint` 默认选择由 `id_format_for_features` 依据 WebGPU
  规范保证表 + 设备 features 决定，**不在运行时**调用 `Adapter::get_texture_format_features`
  实测 `R32Uint` 可渲染；对 `wgpu-hal` GL 后端的引用只是源码依据，**非本机实测**
  （`docs/picking-gpu-id-pass.md` §4）。
- **预期收益**：无性能收益；**证据诚实**——把"依据规范"与"运行时实测"显式区分。
- **前置条件/风险**：如需运行时保证，须在支持 `R32Uint` 的硬件上真实探测并记录；packed
  回退已实现并测试（`the_packed_rgba8_fallback_picks_identically`）。
- **需要的决策**：是否投入运行时探测。
- **相关契约/栅栏**：`id_format_for_features` 纯决策函数（可离线复现）保留；运行时探测若加入
  须补真实 GPU 证据。
- **优先级**：G6（证据标注项）。

### G7 — 真实浏览器 WebGL2 验证（DWG pick-parity 复跑已在 P3a-1 完成）

- **现状/问题**：① 标志块「WebGL2 安全」由离线 **GLSL ES 3.00 back-translation** 测试覆盖
  （`color_shaders_back_translate_to_glsl_es_300`），**非**真实浏览器运行；id target 回退路径
  已实现并测试但未在真实 WebGL2 浏览器上运行（`docs/picking-gpu-id-pass.md` §8）。② parity
  harness（渲染器 id pass vs CPU `cad_spatial`）**已在 P3a-1 重跑**（`docs/picking-gpu-id-pass.md`
  §7：CPU 可复现 GPU 实体 **1.000**、**不可解释冲突 0**；`complex-test.dwg` 精确一致率偏低已定位
  为共面/等深并列差异，harness 不断言该率）。**未完成**的仅剩真实浏览器 WebGL2 运行；未来变更后
  仍须复跑 parity。
- **预期收益**：无性能收益；**验证补全**——把"离线可译"升级为"真实浏览器运行"，并在未来变更后
  复跑 pick-parity。
- **前置条件/风险**：需要真实 WebGL2 浏览器（headful + 真实 GPU，非 SwiftShader）；DWG
  pick-parity 需真实 DWG（`YACR_TEST_DWG`）与真实 GPU。
- **需要的决策**：无。
- **相关契约/栅栏**：真实浏览器 WebGL2 flags 路径运行证据；DWG pick-parity 复跑记录（变更后
  重跑，不用旧数字冒充）。
- **优先级**：G7（验证缺口，显式 NOT RUN）。

---

## 加速层（P6 + P3b 已完成项）

### A — capability-gated accelerated tier（vertex pulling via storage buffers）

- **状态（2026-10-15）：已实现（P3b，`dc1aabb` + `b3a9453`）**，见
  `docs/adr/0007-entity-as-data-renderer.md`「P3b」节。已转换的不透明 line/mesh 类经 VS 只读
  storage（vertex pulling）折叠为**每段一次 draw**；档位由**设备自身 limits** 决定（**从不**向
  宿主设备请求 feature），**WebGL2 自动降级并带标签**。真实 P620 实测 debug-default-fonts
  903→**3** draws、complex-test 8,252→**19** draws；逐像素等价 fence 见
  `crates/cad-render-wgpu/tests/accelerated_tier.rs`。
  **口径更正（2026-10-17）**：上述 **903→3 / 8,252→19** 是**整场景单帧**口径（整场景被接受，
  `accelerated=true`）；在**分页交互帧**上该 per-class 组织**不生效**——**实测**每个生产页
  `accelerated=false`（设备 `tier=accelerated`），此时走通用 per-run（P1 覆盖，见 N1c）。
- **后续（显式未做）**：**multi-draw indirect 未做**（不在 WebGPU core，故意不架构其上，见 P6）；
  **WebGL2 真实浏览器验证未做**；`PulledArc` 未做（弧走通用 per-run 残留）。
- **原设计意图（实施前评估，保留供追溯）**：进一步减少每批 uniform/绑定开销、提升顶点读取效率；
  加速档**不得**成为架构依赖、回退必须像素一致——已按此实现（能力门控 + 带标签降级）。
- **需要的决策**：无（架构约束已定）。
- **相关契约/栅栏**：档位探测 + 回退一致性 fence；加速档在兼容表中显式建模。
- **优先级**：已实现（P6 的 indirect 部分仍待评估）。

---

## 交互与空间索引类（S1–S6；架构见 ADR-0012）

> 本节为**交互 / 空间查询**待办，与上方渲染项分属不同层。架构、不变量、明确否决项与延后项见
> `docs/adr/0012-scene-spatial-index-and-interaction.md`。所有收益均**未实测**，一律标「预期」。

### S1 — 点选闭环接线（SelectAt → pick_at_screen → 高亮/属性）

- **现状/问题（更新于 2026-10-17）**：**点选已接线**（`crates/cad-ui-core/src/session.rs`、`crates/cad-host`、
  React `apps/app-linux/ui`；ADR-0012 §10 S1）。宿主线载荷 `ProbeInput` 新增可选 `button`
  （`BUTTON_PRIMARY/MIDDLE/SECONDARY` = 0/1/2，见 `docs/input.md` §1.1）；`plan_input` 在**主键**按下+抬起、
  位移 `< DRAG_THRESHOLD_LOGICAL_PX`（4.0 逻辑像素）、期间无拖拽时发出 `Pointer{Up}` **与**
  `InteractionRequest::SelectAt{mode: Replace}`（携带当前 `metrics_revision`）；`cad-host` 把 `SelectAt` 落到
  `pick_at_screen`（`crates/cad-app/src/picking.rs:313`）并回填只读 `selection` 状态块与 `FLAG_SELECTED` 高亮。
  React 端新增只读属性面板（`components/PropertiesPalette.jsx`，默认关闭），`selection` 块经 `cad.mjs` 的
  `parseCadStatus` 严格校验。**仍缺**：**框选**——左键拖拽至今仍是 pan（`SelectAt` 只做单点 replace），
  Window/Crossing 属 **S5**（ADR-0012 §10 item 5），本切片**未**实现。
- **预期收益**：**首个用户可见的鼠标拾取能力**（点选）已落地，也是 hover/snap/框选的前提。非性能项。
- **前置条件/风险**：保持 `SelectionRef` 标识完整（ADR-0012 §6；规范 §4.4）；不改变既有判定语义（否则触及
  GPU id-pass parity 栅栏）。
- **需要的决策**：左键“点击 vs 拖拽”阈值已取 4.0 逻辑像素（复用 `DRAG_THRESHOLD_LOGICAL_PX`）；与 pan 手势的
  互斥已定：主键**无拖拽**点击=选择、**有拖拽**=pan（**框选**语义待 S5）。
- **相关契约/栅栏**：DOM→`SelectAt`→`pick_at_screen` 端到端契约；点选结果携带 `SelectionRef` 的回归测试；
  `selection` 状态块形状僵化（见 `docs/handoff.md` 2026-10-17 S1 小节）。
- **证据（2026-10-17，合成/本地）**：`cargo fmt`/`check`/`clippy`（`cad-ui-core`+`cad-host`）PASS；
  `cargo test -p cad-ui-core` **77 passed / 0 failed**；`cargo test -p cad-host` **92 passed / 0 failed / 1 ignored**
  （预存 `perf_baseline`，需 `YACR_TEST_DWG`）；`app-linux` `cargo check` PASS；`(cd apps/app-linux/ui && npm ci)`
  OK、`npm test` **58 passed / 0 failed**、`npm run build` OK（41 modules）；`check-i18n.py` **404 keys / 2 catalogs**。
  主控合入前复核（全树）：`check-architecture.py`（27 packages）/ `check-i18n.py`（404 keys）/ `check-fixture-manifest.py`
  / `check-workflows.py` 以及 `cargo fmt --check`、`cargo test -p cad-ui-core`（77）、`-p cad-host`（92，1 ignored）
  均 PASS。**NOT RUN**：任何 GUI/原生窗口/真实 GPU/真实 DWG、全量 workspace 测试矩阵、本切片的
  wasm/Android/Windows/macOS 目标。
- **优先级**：S1（最高；用户可见价值 × 可证据化比值最高）；**框选部分转入 S5**。

### S2 — Hover 接线（MoveCursor → 节流拾取 → hover 高亮）

- **现状/问题（更新于 2026-10-17）**：**hover 生产者已接线**（有界、合成/本地；`crates/cad-ui-core/src/session.rs`、
  `crates/cad-host`；ADR-0012 §10 S2、`docs/input.md` §1.1.2）。空闲指针移动（**无按下**）除普通 `Pointer{Move}` 外
  **额外**发出 `InteractionRequest::MoveCursor(CanvasLocal{ logical_position, metrics_revision })`；`cad-host` 用与点选
  **完全相同**的拾取路径（`pick_at_screen` + `TolerancePolicy::default()` + `BackFacePolicy::Cull`，**无**新容差/排序）
  只取**排名第一**的实体级 ref 存进宿主本地 `cad_host::HoverState`，并喂给 `OverlayInputs.hovered`（`state.rs`）——
  此前恒为空的 hover 生产者由此变为**非空**（`FLAG_HOVERED` 机制**无需**改批）。**仍未接线**：`hidden` 生产者
  （`cad-host` 仍传空，**受 G2 门控**）。**节流规则（无时间窗）**：依赖 UI 既有 **rAF 合并**，且宿主在**逻辑位置未变**时
  **整段跳过**拾取查询；不存在按时间的节流窗口。**抑制/清除**：按下/导航/捕获拥有指针时抑制，按下开始/`Cancel`/`ClearCursor`/
  pointer cancel/`blur`/未命中即清除（拾取**报错**先清 hover 再显式拒绝，不静默空成功）；hover 在宿主侧归
  **`DirtyScope::Overlay`**，逐帧 hover **不**触发 O(layers+entities) 的 `HostState` 重建。
- **预期收益**：悬停高亮可用（已落地）；**并且是对全量拾取路径（`picking.rs` 每次全量遍历 `model_space()`）的
  第一次真实压力测量**，是启动 S4 的诚实触发条件。**性能收益仍为「预期」**——本轮**未**用真实 DWG/GPU 测出任何数字。
- **前置条件/风险**：高频 Move 的节流语义已按上述冻结（rAF 合并 + 位置未变跳过，**无**时间窗）；不得宣称未实现的
  频率精度。可选探针 `YACR_PROFILE_PICK hover elapsed_ms=… picked=…` 在**既有** `YACR_PROFILE_PICK` 门控下（默认关闭）
  打印一行；其**任何数字仅为合成/本地**（记录运行用 6 实体 demo 文档，无真实 DWG/GPU），**不是**性能结论。
- **需要的决策**：无（节流策略已定为帧对齐 + 位置未变跳过）。
- **相关契约/栅栏**：hover 只在空闲移动产生、有按下不产生；未命中清除（无陈旧着色）；位置未变不重跑拾取（计数，非墙钟）；
  按下/`blur`/`Cancel` 清除；拾取错误显式拒绝。与 G5 合并闭环；`hidden` 半仍受 G2 门控。
- **证据（2026-10-17，合成/本地）**：`cargo fmt -p cad-ui-core -p cad-host -- --check` PASS；
  `cargo clippy -p cad-ui-core -p cad-host --all-targets --locked -- -D warnings` **0 warning**；
  `cargo check -p app-linux --locked` PASS；`cargo test -p cad-ui-core --locked` **91 passed / 0 failed**；
  `cargo test -p cad-host --locked` **102 passed / 0 failed / 1 ignored**（预存 `perf_baseline`，需 `YACR_TEST_DWG`）；
  `(cd apps/app-linux/ui && npm test)` **62 passed / 0 failed**、`npm run build` OK；探针行仅在 `YACR_PROFILE_PICK=1`
  时出现（未设该变量时 **0** 行）。**NOT RUN**：任何 GUI/原生窗口/真实 GPU/截图/真实 DWG、全量 workspace 测试矩阵、
  本切片的 wasm/Android/Windows/macOS 目标。**hover 与点选从未在真实宿主窗口闭环**。
- **优先级**：S2（高；兼作测量）；**hover 半已闭环**，`hidden` 半仍待 G2。

### S3 — Snap 接线（db → SnapTarget → collect_candidates / best_candidate）

- **现状/问题**：`cad-measure/src/snap.rs` 引擎完整（`snap.rs:26` `SnapKind`、`:158` `collect_candidates`），
  但**没有任何调用方从数据库解析 `SnapTarget`**；`MeasurementEngine::snap()` 按设计返回 `Unsupported`。
- **预期收益**：测量/绘图工具的吸附可用（差异化能力）。
- **前置条件/风险**：候选解析必须**候选优先**，不得每次全量展开；越出规范 §3.3 的 snap 类型
  （Tangent / NearestPoint）须显式 `Unsupported`（铁律 2）。
- **需要的决策**：吸附标记的视觉与优先级（UX）。
- **相关契约/栅栏**：相机/缩放变化下吸附结果稳定的 fence（容差按逻辑像素，ADR-0012 §9）。
- **优先级**：S3（高；引擎已存在）。

### S4 — 空间索引与增量失效接入热路径（chunk 网格 + local rstar）

- **已落地（2026-10-17）——生产 CPU 拾取接入空间索引（拾取半）**。逐条证据见
  `docs/validation-pick-index-2026-10-17.md`（实现 commit `13f7808`）。这是 ADR-0012 §10 **S4 的
  拾取部分**；`SceneCache::apply_changes` / `CacheKey` 半**未**动（仍见下方「现状/问题」）。
  - **病根（实测）**：生产拾取是**纯 CPU**，`pick_at_screen` 每次展开 `model_space()` **全部**实体
    （含递归 INSERT）并逐项测试；`filter_by_index` 是**死代码**，索引在拾取链里**无调用方**。在
    `fixtures/complex-test.dwg`（44,591 实体、**723,587** 展开条目）上点选**停顿约一秒**，且每次指针
    移动都跑同一路径（hover）。
  - **改动**：`cad-app::picking` 新增持久化 `PickIndex`（展开条目 + `cad-spatial` 的 `ChunkGridIndex`
    两级 chunk 网格），键为 `scene_identity()`（内嵌 `Revision`）+ document id；`pick_ray` 走
    `pick_ray_with_index`，把射线 X/Y 投影裁剪到网格区域并按容差展开，复用精确阶段**同一** slab 谓词；
    超大图元（跨 chunk > `MAX_CHUNK_SPAN` = **16**）进 overflow 列表、**不**跨块复制；无法给出保守盒的
    家族（opaque / 非正或 NaN 权重样条 / 含无界子节点的 compound）**恒为候选**。
  - **回退与上报**：索引不可用或 `YACR_PICK_EXHAUSTIVE` ⇒ 穷举扫描；模式经 `last_pick_stats()` 的
    `indexed` 字段机读给出（索引路径另打印 `pick_broad_phase mode=indexed …`）。
  - **实测（release harness 驱动**同一**生产 `pick_at_screen`，**非**宿主交互）**：热拾取中位
    **~1055 ms → ~0.32 ms**（842 候选）、一次性建索引 **~535 ms**、最差热拾取（13,491 候选）**~20 ms**。
  - **栅栏（名称即契约）**：随机差分 `indexed_and_exhaustive_picks_agree_over_random_scenes`
    （**16 种子 × 30 射线** × 全部家族，断言 `PickReport` **完全相等**；源码为准），外加容差边界 /
    chunk 边界 / 无命中 / 容差下限各一个；`the_index_examines_fewer_items_on_a_large_scene`
    （5,000 线 <1/10）。
  - **偏离项（记录）**：ADR-0012 §7 命名**每 chunk 的 `rstar`**；本轮 `Cargo.*` 超出范围，每 chunk 为
    **扁平 payload 列表**（`rstar` 仍**未实现**）。索引在本夹具占 **~90–100 MB**（真实足迹，未削减）。
  - **边界**：after 数字来自 release harness，**不是**宿主交互（无探针钩子可 hover/select）；harness
    **纯 CPU、不触碰 GPU** ⇒ **不给** GPU/视觉/vendor 结论；单机单会话。
- **现状/问题（更新于 2026-10-17）**：**拾取半已接线**（见上）。`SceneCache::apply_changes`
  （`cad-scene/src/cache.rs:90`）**只在测试调用**，生产每次全量 rebuild；`CacheKey`
  （`cad-scene/src/batch.rs:8`）**定义即废弃**。这两项仍待接入。
- **预期收益**：拾取/框选/查询成本与图纸总规模脱钩（**拾取半已实测**，见上；框选/查询仍**预期**）。
- **前置条件/风险**：按 ADR-0012 §7 落两级结构（顶层网格 + local `rstar` 0.13 不可变快照）——顶层
  网格与超大图元策略**已落地**（拾取半），**每 chunk 的 `rstar` 仍未实现**（当前为扁平 payload 列表）；
  版本取自 `Revision`，陈旧判定复用 `ChangeSet::follows()` / `TaskStamp`，**不**新增并行版本计数器。
- **需要的决策**：`rstar` 引入的 `Cargo.lock` 更新与许可核查结论（ADR-0012 未决项）；以及
  `SceneCache::apply_changes` / `CacheKey` 接入生产的排期。
- **相关契约/栅栏**：索引接入前后 pick 结果一致 fence（**已落地**，见上）；`CacheKey` 版本键有效性测试；
  超大图元不跨块复制的内存 fence（**已落地**：overflow 列表）。
- **优先级**：S4（**拾取半已落地**；`SceneCache`/`CacheKey` 半仍中；由 S2 实测驱动，不得在无测量时先做）。

### S5 — Window / Crossing 框选（串行）

- **现状/问题**：仓库**零**框选类型；只有单点选与 `SelectAll`（`crates/cad-app/src/select_all.rs:20`，全量遍历）。
- **预期收益**：框选能力；**不依赖**空间索引即可先落地。
- **前置条件/风险**：必须按 ADR-0012 §3 的不对称性实现 —— Window 只有 `rect ⊇ bounds` 是**可靠早接受**，
  `¬contains` **不得**判 reject（本仓 bounds 保守、缺失 bounds 存在，会误拒）；Crossing 只有
  `rect ∩ bounds = ∅` 是**可靠早退**，相交**仅作候选**。缺失/无效 bounds 必须落回精确判定。
- **需要的决策**：Window/Crossing 的方向语义与视觉（沿用 AutoCAD 惯例：左→右 Window、右→左 Crossing）。
- **相关契约/栅栏**：`window_accepts_only_fully_contained`、`window_does_not_reject_on_conservative_bounds`、
  `crossing_rejects_only_on_disjoint_bounds`、`crossing_intersects_is_candidate_not_hit`。
- **优先级**：S5（中；独立于索引）。

### S6 — 按实测再评估项（多核 / 子元素索引 / Block 局部索引 / hover 缓存）

- **现状/问题**：chunk 级并行、子元素（分段/字形）索引、Block 局部索引、Pick Cell / Snap cache 均
  **未实现**，且当前**无实测证据**支持。
- **预期收益**：各为**条件性收益**，未实测。
- **前置条件/风险**：渲染会话属主线程、`cad-platform` 契约无 futures/无 `Send`（ADR-0012 §7）；子元素
  阈值需以 `MAX_PICK_SEGMENTS`（`crates/cad-spatial/src/pick.rs:53`）为参照；Block 局部索引不得破坏
  §18.2 的 per-insert 标识。
- **需要的决策**：全部待实测输入。
- **相关契约/栅栏**：各优化落地时补一致性与回退 fence。
- **优先级**：S6（低；profiling 先行，§8.5）。

---

## 证据边界与陷阱（渲染/性能复核，2026-10-17）

> 本节记录多次复核（读渲染器 + wgpu 30.0.1 源码）的结论与**诚实边界**。**首轮答案部分错误**
> （见 P6/P7 更正）；本轮（独立复核 + 真实宿主实测）把 **P5 否决、P6 拆分、P7 重定门控、P1 范围反转**，
> 并新增 **N1–N5**。所有"GPU 收益"仍为**结构推理，非证据**。

- **当前基线口径（2026-10-17）**：文首**当前**数字来自**真实宿主**（`target/release/yacr`、
  Quadro P620 / Vulkan / Wayland、有界 post-load pan、**prebuilt release**）、两个真实 DWG fixture
  ⇒ **不外推**到其它 vendor / 更大图纸 / 浏览器；相机只做平移，**zoom 腿未跑**。
- **历史基线口径（2026-10-15）**：文首**历史**数字来自**一台机器 / debug（test）profile / 无头
  （headless）**、**整幅无界预算** ⇒ 与当前**分页/超预算/真实宿主**口径**不同**，**不可混算**。
- **不同轮次/口径的数字不得混算**：**797/7,627**（`docs/validation-render-perf-2026-10-15.md` §4，
  整幅/无界预算）与 **903/8,252**（ADR-0007「P3b」通用转换后档）是**不同轮次、不同口径**——
  **绝不可**放进同一个比值；本轮 **3,027/1,195**（分页）与它们**同样不可混算**；`entityCount`
  **44,591**（本轮 complex）与 **122,006**（历史轮）**不是同一口径**。
- **`caps_for` 不是设备测量**：`caps_for`（`crates/cad-render-wgpu/src/renderer_impl.rs:1385-1418`）
  对**每个非 WebGL2 backend 按 backend 类别**报 `compute` / `storage_buffers` / `indirect_draw = true`，
  **不**做运行时探测 ⇒ **不得**当作设备测量引用（与 **G6** 同类证据等级）。真实能力查询见 P0 的
  `crates/cad-render-wgpu/tests/gpu_capabilities.rs`。
- **`wait_ms` 是 GPU/呈现等待的代理，不是 GPU 时间**；`gpu_ms` 在**产品默认（`YACR_PROBE_GPU_TIME`
  关闭）**下仍为 `None`（无 `TIMESTAMP_QUERY`）；仅**测量 opt-in 打开**时为真时间戳（见 §P0 与
  `docs/validation-gpu-time-2026-10-17.md`）。
- **WebGL2 真实浏览器运行仍 NOT RUN（G7）**；加速档的降级路径只有**离线单测**。
- **GPU 帧时间仅在 opt-in 下测量**——§P0 已落地（**每帧总量**已实测）；但 **GPU 内部归因**
  （顶点/三角形吞吐 vs draw 数 vs 带宽）与 **N1d/N2/N5 是否真能降低 `gpu_ms`** 仍**未测**，故上文
  "GPU 收益"表述（P1 加速档部分、P6、P7、N5）仍为**结构推理，不是证据**；N2 的收益估算已按本轮实测更新为
  **8 → 4–5 ms**（仍为**估算**，且 N2 已按实测**重挂于产品需求**，见 §N2）；N5 的 ≈10–12% 仍为**估算**。

---

## 交叉引用

- 场景空间索引与 Pick/Snap/Selection 架构（含明确否决项与延后项）：
  `docs/adr/0012-scene-spatial-index-and-interaction.md`。
- 渲染架构决策与 deferred 列表：`docs/adr/0007-entity-as-data-renderer.md`。
- SDF 圆/弧（解析弧）决策与实施记录：`docs/adr/0008-sdf-conic-rendering.md`。
- GPU id 回读拾取 / 选择标志机 / 已知限制与未做项：`docs/picking-gpu-id-pass.md`（尤其 §0、§4、§6、§7、§8）。
- 绘制顺序与透明策略（instanced / merged / legacy 并存）：`docs/render-order.md`。
- 性能基线与证据边界（历史**整幅**口径）：`docs/validation-render-perf-2026-10-15.md`。
- 帧成本分布 + P0 前半（CPU 相位、`TIMESTAMP_QUERY` 调查）：`docs/validation-render-perf-2026-10-17.md`。
- **真实宿主分页帧成本 + 4 GB 卡 OOM + 产品决定**：`docs/validation-realhost-frame-cost-2026-10-17.md`。
- **生产 CPU 拾取接入空间索引（S4）**：`docs/validation-pick-index-2026-10-17.md`；**相机-only 导航保持
  选择高亮**：`docs/validation-selection-survives-camera-2026-10-17.md`。
- 能力表与显式未支持项：`docs/compatibility.md`。

> 证据诚实：本文所有未实测收益均标注为「预期」；未运行项（真实浏览器 WebGL2、DWG
> pick-parity 复跑、视觉 golden、Android 真机）标注为 NOT RUN / 未验证，不得外推为跨平台或
> 完整 CAD 视觉验收。
