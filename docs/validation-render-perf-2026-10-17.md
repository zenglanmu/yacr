# 渲染帧成本分布验证（2026-10-17 轮）

状态：**backlog §P0 前半（帧成本分布 / measurement prerequisite）已实际执行**。本轮**只做测量**：
不动渲染行为、不改像素、**不实现 P1（块级视锥剔除）**。真实 GPU 路线在 **NVIDIA Quadro P620 / Vulkan**
上跑通合成 scale-matched 腿与 `debug-default-fonts.dwg` 腿；`complex-test.dwg` 在 P620 上**因显存不足
（OOM）未能测量**，改在 **Intel UHD 集显**上测量并**逐处显式标注适配器**（见 §5）。

**本轮没有任何 GPU 时间。** 设备从不请求 feature（两处 `DeviceDescriptor { ..Default::default() }`），
运行时实测 enabled features 为**空**、`TIMESTAMP_QUERY=false`，仓内也**没有任何** timestamp/`QuerySet`
管线。本文件报出的 `residual`（`wall − Σ CPU 相位`）**不是 GPU 时间**，只是"CPU 相位计时器没覆盖到的墙钟"，
其中包含阻塞式 device poll、以及（无 present 的）前置 CPU。详见 §2、§6。

需求与边界：`docs/render-optimization-backlog.md` §P0（及 §P1/§P3/§P6/§P7）、
`docs/adr/0007-entity-as-data-renderer.md`；历史性能轮见 `docs/validation-render-perf-2026-10-15.md`。
**本文件不改写历史轮次**，其数字（797/7,627、903/8,252）与本轮口径不同，**不得混算**。

**未**运行全量 workspace 门禁（AGENTS.md 2026-10-15 提效策略：日常只验证改动范围）；
**未**运行 WebGL2/浏览器/Android/Windows/macOS 验收；**未**做真实窗口宿主（React/Tauri）内测量。

## 0. 本轮做了什么 / 没做什么

**做了**（harness-only，纯增量、行为中性）：

1. 在既有 CPU 相位计时器之外，逐帧记录 **`wall`**（整个 `Renderer::render()` 调用）并推导
   **`residual = wall − (plan_ms + uniform_ms + encode_ms + submit_ms)`**，按 mean/median/p95 聚合，
   强制标注为 *"GPU + present + 阻塞 poll —— **不是** GPU 测量"*。
2. 把分布**跨视图状态**暴露：同一场景在 `full`（整个场景装进目标）与 `zoom`（世界矩形小、
   比例大）两种相机下各测一遍（合成腿 `zoom = full/16`；真实腿用既有 deep-zoom 腿 `fit/20`）。
3. 记录**每腿实际使用的档位/路径**：`tier`（`universal`/`accelerated`）、`accelerated`
   （本帧 opaque 是否真的走了加速档的 per-class 组织），以及 `over_budget`（**预算回退**：
   本帧只是场景的一"页"，不是整幅）。
4. 加入**全览 vs 深度放大**的成本对比（测试"帧成本是否随可见几何变化"）。
5. **调查（未实现）** `TIMESTAMP_QUERY` 若要接入该放在哪里、会改变什么（§6）。

**没做**：P1 剔除；任何 feature 请求；任何渲染/像素/性能语义改动；任何依赖或 `Cargo.lock` 改动。

改动文件（仅测试 harness，无 `src/**` 改动）：

- `crates/cad-render-wgpu/tests/render_perf.rs`（合成腿：`Full`/`Zoom` 视图态、`wall`/`residual`
  分布、`over_budget` 列、enabled-features 与 `TIMESTAMP_QUERY` 证明行）。
- `crates/cad-cli-tools/tests/realscene_render_perf.rs`（真实 DWG 腿：`wall_ms`/`residual_ms` 逐帧记录、
  每腿 `tier`/`accelerated`/预算回退、全览 vs 深度放大对比、`YACR_PERF_GPU` 适配器选择）。

均**不含**渲染器运行时代码改动 ⇒ **不可能改变像素**；无新依赖。

## 1. 环境与适配器证明

- 机器/系统：Ubuntu 26.04.1 LTS，kernel `7.0.0-38-generic`；30 GiB RAM。
- GPU：**NVIDIA Quadro P620**（`10de:1cbd`，4096 MiB VRAM），驱动 **580.178.04**，
  **默认 Vulkan ICD**（**未**设 `VK_ICD_FILENAMES`，未用 lavapipe/SwiftShader）。
  另一枚适配器为 **Intel UHD Graphics (CometLake-H GT2)**（`vulkan` / `integrated_gpu` /
  Mesa 26.0.8），仅在 §4.2 显式使用并标注。
- 工具链：`cargo/rustc 1.99.0`（`rust-toolchain.toml`），`--locked`，**debug profile**
  （`test` profile：`opt-level=3 + debuginfo`，见 workspace profile；**不是** release）。
- 适配器由 harness **实报**（非假设）：
  - 合成腿与 `debug-default-fonts` 腿：
    `vulkan / Quadro P620 / discrete_gpu / driver NVIDIA 580.178.04`；
  - `complex-test` 腿（§4.2）：`vulkan / Intel(R) UHD Graphics (CML GT2) / integrated_gpu /
    Mesa 26.0.8-1ubuntu0.3`。
- 运行时 **enabled device features = 空**（`Features { features_wgpu: FeaturesWGPU(0x0),
  features_webgpu: FeaturesWebGPU(0x0) }`），`TIMESTAMP_QUERY = false` —— 这是"从未请求 feature"的
  实测证明，也是"本轮无 GPU 时间"的直接证据。
- 证据目录（本轮，本地）：`/tmp/opencode/yacr-perf/`
  - `p0-synthetic-debug/`（合成腿）
  - `p0-realscene-debug-ddf/`（真实 `debug-default-fonts.dwg`，P620）
  - `p0-realscene-debug-complex-igpu/`（真实 `complex-test.dwg`，**集显**）
  - `p0-realscene-debug-complex/`、`…-bt/`（P620 上 complex-test 的**失败/OOM 记录**，保留为负面证据）
  - 运行日志另存 `/tmp/opencode/yacr-p0/*.log`。

## 2. 方法与口径（`wall` / `cpu` / `residual` 的确切含义）

- `cpu_ms`（`FrameStats::cpu_ms`）：渲染线程 CPU 路径（plan/cache、uniform 打包、命令编码、
  `queue.submit`），**排除**阻塞 poll。
- `wall_ms`：整个 `Renderer::render()` 调用的墙钟，**包含**阻塞 poll。
- `residual_ms` = `wall_ms − (plan_ms + uniform_ms + encode_ms + submit_ms)`。
  **它不是 GPU 时间**：其中包含阻塞式 device poll（`wait_ms`，它**在等 GPU**）、（无 present 的）前置
  CPU（`ensure_target`、flags 表补齐）与计时噪声。它**只能**被读作"CPU 相位计时器未覆盖的墙钟上界"。
- 聚合：mean / median / p95（沿用既有 `Stats` 风格）。
- **无 `TIMESTAMP_QUERY`**（§6）⇒ 本文件**没有任何 GPU 执行时间**；`FrameStats::gpu_ms` 恒为 `None`。
- 各腿**在同一个进程、同一设备、同一场景**内先后测量；相机只平移（3 px/帧 或 8 px/帧）
  或在既有 deep-zoom 腿里改变 `world_per_px`。**未**混用不同轮次数字。

## 3. 合成 scale-matched 腿（P620 / Vulkan / debug）

命令与产物见 §7。目标 1280×800，每腿 30 帧（5 帧预热不计）。场景是**合成 scale-matched**：
批数 = fixture 实体数（10,656 / 44,591），**并非**真实图纸几何；两点共同确认
`over_budget = none`（整场景被接受），`tier = accelerated`。

**帧成本分布（ms/帧，mean）**——`zoom = full / 16`：

| 场景（合成） | 路径 | 视图 | `world_per_px` | `draw_calls` | wall mean | wall median | wall p95 | cpu mean | residual mean | wait mean |
|---|---|---|---:|---:|---:|---:|---:|---:|---:|---:|
| debug-default-fonts | legacy | full | 2.081 | 10,656 | 16.735 | 16.524 | 17.142 | 14.788 | 1.975 | 1.944 |
| debug-default-fonts | legacy | zoom | 0.130 | 10,656 | 17.009 | 16.940 | 17.444 | 15.138 | 1.893 | 1.867 |
| debug-default-fonts | instanced | full | 2.081 | 108 | 1.290 | 1.220 | 1.705 | 0.841 | 0.469 | 0.447 |
| debug-default-fonts | instanced | zoom | 0.130 | 108 | 1.086 | 1.048 | 1.496 | 0.776 | 0.329 | 0.308 |
| complex-test | legacy | full | 8.709 | 44,591 | 72.859 | 71.847 | 77.677 | 64.172 | 8.720 | 8.683 |
| complex-test | legacy | zoom | 0.544 | 44,591 | 78.153 | 77.832 | 81.162 | 67.207 | 10.978 | 10.942 |
| complex-test | instanced | full | 8.709 | 447 | 6.048 | 5.998 | 6.476 | 4.626 | 1.460 | 1.418 |
| complex-test | instanced | zoom | 0.544 | 447 | 4.899 | 4.887 | 5.214 | 3.851 | 1.081 | 1.045 |

观察（**诚实口径**）：

- **`draw_calls` 在两个视图态完全相同**（legacy 10,656/44,591；instanced 108/447）：
  提交集合**与视图无关**（当前无剔除）。
- `residual` 在该设备上**几乎等于 `wait`**（如 complex-test instanced full：1.460 vs 1.418），
  因为 poll 在等 GPU；这正说明 `residual` **不是** GPU 时间，而是被 poll 吞掉的墙钟。
- 视图态之间 wall 差异在 ±20% 内（complex-test legacy：72.859 → 78.153；instanced：6.048 → 4.899），
  远小于"可见面积比 256×"⇒ 成本**不随可见几何缩放**（本腿确定性结论：无剔除 ⇒ 每帧全量提交）。

### 3.1 同腿的 release profile（口径单列：release / 同机 / 同 fixture）

命令同上加 `--release`（`cargo test --release --locked -p cad-render-wgpu --test render_perf
renderer_frame_cpu_measurement -- --ignored --nocapture --test-threads=1`），产物
`/tmp/opencode/yacr-perf/p0-synthetic-release/`。适配器同 §1（P620 / Vulkan / discrete_gpu），
features 仍为空、`TIMESTAMP_QUERY=false`。

| 场景（合成） | 路径 | 视图 | `world_per_px` | `draw_calls` | wall mean | wall median | wall p95 | cpu mean | residual mean | wait mean |
|---|---|---|---:|---:|---:|---:|---:|---:|---:|---:|
| debug-default-fonts | legacy | full | 2.081 | 10,656 | 11.747 | 11.729 | 12.342 | 9.789 | 1.980 | 1.955 |
| debug-default-fonts | instanced | full | 2.081 | 108 | 1.346 | 1.434 | 1.654 | 0.849 | 0.517 | 0.494 |
| debug-default-fonts | instanced | zoom | 0.130 | 108 | 0.995 | 0.895 | 1.443 | 0.660 | 0.351 | 0.333 |
| complex-test | legacy | full | 8.709 | 44,591 | 53.140 | 53.435 | 55.726 | 45.527 | 7.638 | 7.610 |
| complex-test | instanced | full | 8.709 | 447 | 4.984 | 5.027 | 5.378 | 3.807 | 1.205 | 1.174 |
| complex-test | instanced | zoom | 0.544 | 447 | 4.672 | 4.630 | 4.922 | 3.751 | 0.951 | 0.918 |

release 只把 **CPU 相位**显著压低（如 complex-test legacy cpu 64.172 → 45.527 ms），
**视图态的恒定性不变**（同为 447/44,591 draws；full↔zoom wall 4.984↔4.672 ms）。
`residual`/`wait`（即阻塞 poll）**不受 release 影响**（complex-test legacy 8.720 → 7.638 ms 基本不变）
——这符合"poll 在等 GPU"的解释。

## 4. 真实 DWG 腿（含 deep zoom 与档位/预算回退）

目标 1600×1000，每腿 24 帧；真实 `acadrust` 导入 → font-aware 表示（`with_packed_line_segments`）
→ `SceneCache::build_compact` → 真实 headless wgpu → 逐帧 `Renderer::render`。
`deep zoom = fit / 20`（可见世界面积 ≈ 全览的 **1/400**）。字体：仅 committed `fonts/` 包（osifont）
+ 系统默认 face（3 个 face；mlightcad 全量字体目录离线未拉取）。

### 4.1 `debug-default-fonts.dwg`（**P620 / Vulkan / debug**）

- 10,656 实体（模型空间 5,604），导入 `partial (17 issues)`。
- 整幅（无界预算）：legacy `draw_calls` **10,545** → instanced+merged **56**（188.30×）。
- 默认预算单帧（`cad-cli-tools render` 形状，8,000,000 顶点 / 2,000,000 三角形）：
  legacy **8,126** draws（`over_budget`：`triangles requested=2001067 limit=2000000 skipped_batches=2419`）；
  instanced **1,122** draws（`over_budget` … `skipped_batches=1444`）。
- resident：544 个 instance stream（37,911 segment）、134 个 merged mesh chunk（2,351,044 顶点）。

| 腿 | `world_per_px` | `tier` | `accelerated` | 预算回退 | `draw_calls` | wall mean | wall median | wall p95 | cpu mean | residual mean | wait mean |
|---|---:|---|---:|---|---:|---:|---:|---:|---:|---:|---:|
| `pan_mid_legacy` | 380.754 | accelerated | **false** | 有（skipped 2419） | 8,126 | 22.257 | 21.861 | 26.696 | 18.673 | 3.614 | 3.528 |
| `pan_mid_instanced_universal` | 380.754 | universal | false | 有（skipped 1444） | 1,122 | 7.612 | 6.918 | 10.030 | 3.984 | 3.663 | 3.557 |
| `pan_mid_instanced` | 380.754 | accelerated | **false** | 有（skipped 1444） | 1,122 | 7.024 | 6.549 | 8.353 | 3.415 | 3.638 | 3.540 |
| `pan_deep_instanced` | 19.038 | accelerated | **false** | 有（skipped 1444） | 1,122 | 6.394 | 6.012 | 9.614 | 3.392 | 3.030 | 2.937 |
| `zoom_mid_instanced` | 361.716 | accelerated | **false** | 有（skipped 1444） | 1,122 | 6.359 | 5.993 | 9.435 | 3.436 | 2.953 | 2.855 |

**全览 vs 深度放大（成本 vs 可见几何）**：`pan_mid_instanced`（full，`world_per_px` 380.75）
↔ `pan_deep_instanced`（deep，19.04），同一 renderer/预算/装置：

| 腿 | `world_per_px` | `draw_calls` | wall mean | cpu mean | residual mean | wait mean |
|---|---:|---:|---:|---:|---:|---:|
| full（fit） | 380.754 | 1,122 | 7.024 | 3.415 | 3.638 | 3.540 |
| deep（fit/20） | 19.038 | 1,122 | 6.394 | 3.392 | 3.030 | 2.937 |
| **deep/full 比** | 1/20 | **1.00×** | **0.91×** | **0.99×** | 0.83× | 0.83× |

⇒ 可见世界面积缩到 **1/400**，而 wall 只变 **0.91×**、`draw_calls` **完全相同**：
帧成本**不随可见几何缩放**（本装置上近似恒定）。**这是结构性结论**（提交集合与视图无关），
**不是** GPU 结论。

### 4.2 `complex-test.dwg`（**Intel UHD 集显 / Vulkan / debug**，**非** P620 —— 口径单列）

> P620 上该 fixture **OOM 未跑通**（§5）。本节数字来自 **另一枚适配器**（集显，共享系统内存），
> 与 §3/§4.1 的 P620 数字**不是同一口径**，**不得**混算或当作 P620 结论。

- 44,591 实体（模型空间 7,942），导入 `partial (58 issues)`。
- 整幅（无界预算）：legacy `draw_calls` **163,378**（含 13,965 个 mesh 边缘 overlay ⇒ 有效 177,343）
  → instanced+merged **908**（179.93×）；instanced 路径 universal per-run 需 74,039 draws ⇒ 加速档省 81.54×。
- resident：5,737 个 instance stream（**27,511,456** segment）、1,164 个 merged mesh chunk（6,889,483 顶点）。
- 默认预算单帧：legacy 1,594 draws（skipped 164,751）；instanced 2,999 draws
  （accepted opaque 7,715，skipped 158,630）。

| 腿 | `world_per_px` | `tier` | `accelerated` | 预算回退 | `draw_calls` | wall mean | wall median | wall p95 | cpu mean | residual mean | wait mean |
|---|---:|---|---:|---|---:|---:|---:|---:|---:|---:|---:|
| `pan_mid_legacy` | 587.646 | accelerated | false | 有（skipped 164,751） | 1,594 | 37.560 | 33.907 | 50.064 | 13.197 | 24.418 | 23.351 |
| `pan_mid_instanced_universal` | 587.646 | universal | false | 有（skipped 158,630） | 2,999 | 136.056 | 138.063 | 145.976 | 32.568 | 103.558 | 101.897 |
| `pan_mid_instanced` | 587.646 | accelerated | **false** | 有（skipped 158,630） | 2,999 | 135.391 | 138.310 | 149.064 | 32.390 | 103.073 | 101.411 |
| `pan_deep_instanced` | 29.382 | accelerated | **false** | 有（skipped 158,630） | 2,999 | 39.830 | 36.808 | 57.652 | 18.575 | 21.300 | 20.417 |
| `zoom_mid_instanced` | 558.256 | accelerated | **false** | 有（skipped 158,630） | 2,999 | 70.957 | 55.568 | 141.336 | 24.507 | 46.506 | 45.303 |

**全览 vs 深度放大**：`pan_mid_instanced` ↔ `pan_deep_instanced`：

| 腿 | `world_per_px` | `draw_calls` | wall mean | cpu mean | residual mean | wait mean |
|---|---:|---:|---:|---:|---:|---:|
| full（fit） | 587.646 | 2,999 | 135.391 | 32.390 | 103.073 | 101.411 |
| deep（fit/20） | 29.382 | 2,999 | 39.830 | 18.575 | 21.300 | 20.417 |
| **deep/full 比** | 1/20 | **1.00×** | **0.29×** | **0.57×** | 0.21× | 0.20× |

诚实读法（**不要**过度解释）：`draw_calls` 与提交集合**完全不变**，但 wall 降到 0.29×。
在集显上 GPU 片元/带宽与提交背压都进了 `wait`/`submit`，故 `wait`（101.4 → 20.4 ms）本身就是差异来源。
**能确定的只有**：放大 **20×**（可见面积 1/400）并没有把帧成本按可见几何按比例削减；
**不能**由此推出"集显也恒定"，也**不能**外推到 P620。

### 4.3 release profile（与"体感"直接相关，口径单列）

同一 harness 加 `--release` 重跑（产物 `p0-realscene-release-ddf/`、`p0-realscene-release-complex-igpu/`；
适配器**不变**：ddf = P620，complex-test = Intel UHD 集显）。表格口径与 §4.1/§4.2 相同的字段。

`debug-default-fonts.dwg`（**P620**，release）：

| 腿 | `world_per_px` | `accelerated` | 预算回退 | `draw_calls` | wall mean | wall median | wall p95 | cpu mean | residual mean | wait mean |
|---|---:|---|---:|---:|---:|---:|---:|---:|---:|---:|
| `pan_mid_legacy` | 380.754 | false | 有 | 8,126 | 17.019 | 17.021 | 17.321 | 13.421 | 3.622 | 3.581 |
| `pan_mid_instanced` | 380.754 | false | 有 | 1,122 | 5.347 | 5.326 | 5.603 | 2.323 | 3.045 | 3.005 |
| `pan_deep_instanced` | 19.038 | false | 有 | 1,122 | 5.587 | 5.599 | 5.774 | 2.740 | 2.871 | 2.824 |

⇒ release 下 ddf 全览/深度放大都 ≈ **5.3–5.6 ms/帧**（headless，含阻塞 poll），
远低于 debug 的 6.4–7.0 ms；**视图切换不改变 draw 数（1,122）**。

`complex-test.dwg`（**Intel UHD 集显**，release；**非** P620）：

| 腿 | `world_per_px` | `tier` | `accelerated` | 预算回退 | `draw_calls` | wall mean | wall median | wall p95 | cpu mean | residual mean | wait mean |
|---|---:|---|---:|---|---:|---:|---:|---:|---:|---:|---:|
| `pan_mid_legacy` | 587.646 | accelerated | false | 有 | 1,594 | 26.096 | 25.092 | 28.718 | 8.492 | 17.639 | 17.450 |
| `pan_mid_instanced_universal` | 587.646 | universal | false | 有 | 2,999 | 115.325 | 116.389 | 127.689 | 26.785 | 88.599 | 88.220 |
| `pan_mid_instanced` | 587.646 | accelerated | **false** | 有 | 2,999 | 115.955 | 115.466 | 126.547 | 27.370 | 88.646 | 88.262 |
| `pan_deep_instanced` | 29.382 | accelerated | **false** | 有 | 2,999 | 29.155 | 26.534 | 39.992 | 13.201 | 15.989 | 15.795 |
| `zoom_mid_instanced` | 558.264 | accelerated | false | 有 | 2,999 | 57.990 | 45.148 | 116.718 | 20.317 | 37.721 | 37.425 |

**release 下能直接读出的三件事**（其余**不作结论**）：

1. **本装置上 `complex-test` 的分页帧远慢于 `debug-default-fonts`**：release 全览分页帧
   `pan_mid_instanced` = **115.955 ms** vs ddf 同口径 **5.347 ms**（≈21.7×）；debug 下是
   135.391 vs 7.024（≈19.3×）。与"release 下 `complex-test` 卡、`debug-default-fonts` 顺"
   的体感**方向一致**（但体感来自真实窗口宿主，**本轮未测宿主**）。
2. **分页帧的档位回退是常态**：两 fixture、两 profile、**所有** `pan_*` 腿的 `accelerated` 均为
   `false`，即这些帧都走 **universal per-run** 组织（`pan_mid_instanced` 与
   `pan_mid_instanced_universal` 数值几乎相同：115.955 vs 115.325 ms，draws 同为 2,999）。
3. **`complex-test` 的 instanced 分页帧（2,999 draws / 116 ms @release-igpu）比 legacy 分页帧
   （1,594 draws / 26 ms）更贵**，尽管 legacy 整幅要 163,378 draws。两腿接受集合不同
   （instanced accepted opaque = 7,715 批 vs legacy = 1,594 批），**不是同一工作量的对比**；
   可确定的是：**"逐帧成本"由被接受页的 draw/流组织决定，而不是由可见几何决定**
   （放大 20× 后 draws 仍为 2,999）。其机制（run 是否整段提交等）**本轮未证实**，留给 P1/P6。

**release 与我们体感的边界**：本 harness **无 present、无真实窗口**，且 poll 是**阻塞式**的
（等于"每帧等 GPU 完成"）。真实宿主里帧率还受 present 模式、事件循环与合成器影响，
**不得**把上表当作宿主 FPS 预测。

### 4.4 关于「档位/路径实际使用」的关键事实（两腿共同）

- `tier` 报 `accelerated`（设备具备存储缓冲能力），但**每一腿的 `accelerated` 都是 `false`**：
  这些帧都**触发了默认帧预算**（`over_budget = Some(...)`），即**只接受场景的一"页"**；
  按 `FrameStats` 语义（`crates/cad-render-wgpu/src/lib.rs:346-351`），
  一页**没有接受全部已转换批次**时本帧回退到 **universal per-run 组织**。
- 直接后果：`pan_mid_instanced` 与 `pan_mid_instanced_universal`（强制关档）**几乎无差**
  （debug 腿 P620：7.024 vs 7.612 ms；debug 腿集显：135.391 vs 136.056 ms，`draw_calls` 同为 2,999）。
- 也就是说，**交互式的"分页/超预算"帧根本没吃到加速档的 per-class 组织**，而是 2,999 次
  per-run draw（release 数值见 §4.3）。这是"release 下 `complex-test` 卡、`debug-default-fonts` 顺"
  这一体感的一个 **候选结构解释**（`debug-default-fonts` 的分页帧只有 1,122 draws；`complex-test`
  是 2,999，且每帧仍要规划/上传/提交整幅 27.5M 段所在页）。**本文件不把它当作已证结论**：
  该体感来自真实窗口宿主（有 present、无 headless poll），**本轮未测量宿主**。

## 5. 未能测量 / 失败（负面证据，保留）

`complex-test.dwg` 在 **P620（4096 MiB）** 上**三次**均失败，形态一致：

```text
thread 'realscene_render_frame_perf' panicked at …/wgpu-30.0.1/src/backend/wgpu_core.rs:1619:18:
wgpu error: Out of Memory
  … <cad_render_wgpu::Renderer>::vertex_buffer        (crates/cad-render-wgpu/src/renderer_impl.rs:2562)
  … <cad_render_wgpu::Renderer>::prepare_upload_batches (…:1913)
  … <cad_render_wgpu::Renderer>::prepare_upload         (…:1514)
  … <cad_render-wgpu::Renderer>::upload                 (…:1506)
  … realscene_render_perf::run_fixture (tests/realscene_render_perf.rs:738  = legacy.upload(&delta))
```

- 复现：① 两 fixture 同进程（第一次跑就 OOM，第一枚 fixture 成功）；② 单 fixture 单跑；③ 加
  `RUST_BACKTRACE=1` 的单跑（等价 + 栈）。命令见 §7。
- 归因（**只报观测**）：OOM 发生在 **legacy 路径的 `upload`**（逐批顶点缓冲），**不是**多适配器
  争抢 —— 运行时该卡 `memory.used = 5 MiB / 4096 MiB`（无其它 GPU 客户端），即该 fixture 自身的
  逐批 GPU 足迹超过了这张 4 GB 卡；harness 已在路径间 `drop` + `poll` 回收。
- ⇒ **P620 上的 `complex-test.dwg` 帧成本 = NOT RUN（OOM）**。§4.2 的集显数字**替代不了**它。
- 该失败**与本轮改动无关**：改动只在两个测试文件内（harness 计时/记录），未碰 `src/**`，
  也未改上传路径。

## 6. `TIMESTAMP_QUERY` 调查（**只调查，未实现**）

**结论：本轮不请求任何 feature；若将来要精确 GPU 时间，必须是一次有文档的、measurement-only 的偏离。**

- **现状（实测）**：运行时 enabled features **为空**，`TIMESTAMP_QUERY` **false**（报告里直接打印）。
- **两处设备创建点**（都是零 feature 请求）：
  - `crates/cad-render-wgpu/src/headless.rs:155-158`
    （`adapter.request_device(&wgpu::DeviceDescriptor { label: Some("cad-headless-device"),
    ..Default::default() })`）—— **测量/离屏路径**；
  - `crates/cad-render-wgpu/src/native_presenter.rs:491-494`
    （`… { label: Some("cad-surface-probe-device"), ..Default::default() }`）—— 表面探针。
  - 另有测试内的一处同形请求：`crates/cad-render-wgpu/src/pulled_prefix_gpu_tests.rs:281`
    （测试专用）。
- **仓内没有任何 timestamp 管线**：全仓 `grep` 无 `QuerySet` / `write_timestamp` /
  `resolve_query_set`；仅有的 `timestamp_writes: None` 与 `occlusion_query_set: None` 是 wgpu
  既有描述符字段（`renderer_impl.rs:4329-4330`、`native_presenter.rs:682/788`、`pick.rs:673-674`），
  **不是**测量实现。`FrameStats::gpu_ms`（`lib.rs:231`）存在但**恒为 `None`**。
- **若要接入，请求要放在哪里**：设备创建点即 `headless.rs:155`（以及镜像的
  `native_presenter.rs:491` 若要探针同样测量）。它会改变**设备契约**：
  `DeviceDescriptor.required_features` 从 `empty()` 变为（至少）`TIMESTAMP_QUERY`，
  且真正的测量还需要在**渲染器热路径**里创建/写入/解析 `QuerySet`
  （`renderer_impl.rs` 的 `render_with_projection`：`RenderPassTimestampWrites` 需
  `TIMESTAMP_QUERY_INSIDE_ENCODERS`；只写 pass 边界则需 `MULTI_DRAW_*` 无关的
  `TIMESTAMP_QUERY_INSIDE_PASSES`）。也就是说这**不是纯 harness 改动**：它会把测量关注点
  带进被测量的代码路径。
- **可用的现成只读接口**：`Renderer::device_features()`（`renderer_impl.rs:891-897`）暴露
  已启用 features —— 本轮的 harness 正是用它来**证明** `TIMESTAMP_QUERY` 未启用。
- **建议（仅建议，不实现）**：**不要**为测量而修改生产设备；若将来决策要精确 GPU 时间，
  应走**独立的、显式门控的测量设备**（例如 harness 自己 `request_device` 一个带
  `TIMESTAMP_QUERY` 的设备 + 独立 `QuerySet`），并且**先验证适配器是否真的支持**
  （`Adapter::features()`;注意 P620 默认设备上该 feature 未启用），
  同时在报告里把"真时间戳"与"估算代理"分口径。这与 backlog §P0/P6 的决策项一致。

## 7. 复现命令（逐字）

合成腿（P620 / 默认 Vulkan ICD / discrete 优先）：

```bash
export PATH="$HOME/.cargo/bin:$PATH"
YACR_PERF_TAG=p0-synthetic-debug \
YACR_PERF_OUT=/tmp/opencode/yacr-perf/p0-synthetic-debug \
  cargo test --locked -p cad-render-wgpu --test render_perf \
    renderer_frame_cpu_measurement -- --ignored --nocapture --test-threads=1
```

真实 DWG 腿（`debug-default-fonts.dwg`，P620）：

```bash
YACR_PERF_DWGS="$PWD/fixtures/debug-default-fonts.dwg" \
YACR_PERF_OUT=/tmp/opencode/yacr-perf/p0-realscene-debug-ddf \
YACR_PERF_DEEP_ZOOM=20 YACR_PERF_FRAMES=24 \
  cargo test --locked -p cad-cli-tools --test realscene_render_perf \
    -- --ignored --nocapture --test-threads=1
```

真实 DWG 腿（`complex-test.dwg`，**集显**；P620 会 OOM，见 §5）：

```bash
YACR_PERF_GPU=low \
YACR_PERF_DWGS="$PWD/fixtures/complex-test.dwg" \
YACR_PERF_OUT=/tmp/opencode/yacr-perf/p0-realscene-debug-complex-igpu \
YACR_PERF_DEEP_ZOOM=20 YACR_PERF_FRAMES=24 \
  cargo test --locked -p cad-cli-tools --test realscene_render_perf \
    -- --ignored --nocapture --test-threads=1
```

P620 上的预期失败（负面证据复现）：

```bash
YACR_PERF_DWGS="$PWD/fixtures/complex-test.dwg" \
YACR_PERF_OUT=/tmp/opencode/yacr-perf/p0-realscene-debug-complex \
YACR_PERF_DEEP_ZOOM=20 YACR_PERF_FRAMES=24 \
  cargo test --locked -p cad-cli-tools --test realscene_render_perf \
    -- --ignored --nocapture --test-threads=1   # → wgpu error: Out of Memory
```

（`YACR_PERF_GPU=auto|high|low` 默认 `auto`＝discrete 优先；`YACR_PERF_ZOOM_FACTOR` 调合成腿
zoom 倍数，默认 16。）

## 8. 未运行 / 边界

- **单机、单适配器、debug（`test`）profile 为主**：§3/§4.1/§4.2 的数字来自**一台机器**、
  **debug profile**、两个具体 fixture + 合成 sweep。release profile 已在 §3.1/§4.3 **实际补跑**
  （同机同 fixture），但**仍不外推**到其它 vendor/驱动、更大图纸、浏览器或真机移动平台，
  也**不**代表真实窗口宿主的帧率（无 present）。
- **两个不同口径不得混算**：§3/§4.1 是 **P620 / debug**，§4.2 是 **Intel UHD 集显 / debug**，
  §3.1 是 **P620 / release**，§4.3 是 **P620（ddf）与集显（complex-test）/ release**。
  历史轮次（797/7,627、903/8,252；`docs/validation-render-perf-2026-10-15.md`）是**另外的轮次与口径**，
  与本轮**不可混算**。
- **无 GPU 时间**：无 `TIMESTAMP_QUERY`（未请求、未启用、无管线）；`residual` 与 `wall` **都不是**
  GPU 时间。任何"GPU 收益"（P1 加速档部分、P6、P7）在本轮之后**依然是结构推理，不是证据**。
- **P620 上 `complex-test.dwg` NOT RUN（OOM）**；**没有**在真实窗口宿主（React/Tauri，有 present）
  内测量；**没有**视觉/golden 验收；**没有**全量 workspace 门禁与平台矩阵。
- **未实现 P1（块级视锥剔除）**：本轮**只**建立基线分布；"剔除能省多少"仍**未实测**。
- **未做**：`cargo fmt/check/clippy/*` 之外的全量 workspace 门禁；WebGL2/浏览器；Android 真机；
  Windows/macOS；`cad-render-wgpu` 全量集成测试串行复跑（见 §9）。
- 证据诚实：退出码 0、报告非空、`error=None` 都**不能**单独证明视觉得分；本节所有"未运行"均
  标 **NOT RUN**，不得以旧证据冒充。
