# 渲染性能改进验证记录（2026-10-15 轮）

本文件记录 2026-10-15 “entity = data” 渲染性能轮**实际执行**的测量。所有数字来自真实
NVIDIA Quadro P620 / Vulkan，除非某行**显式**标注为合成 scale-matched 场景或软件
lavapipe。证据目录在 `/tmp/opencode/yacr-perf/`。**未**运行全量 workspace 门禁（用户
2026-10-15 提效决定：日常只验证改动范围）；**未**运行 WebGL2/Android/浏览器视觉验收。

架构决策见 `docs/adr/0007-entity-as-data-renderer.md`；绘制顺序现行语义见
`docs/render-order.md`。历史轮次的 validation 文档不改写。

## 环境与适配器证明

- GPU：NVIDIA **Quadro P620**，`device_type = discrete_gpu`，驱动 **580.178.04**，默认
  Vulkan ICD。性能测量**未**设置 `VK_ICD_FILENAMES`（未用 lavapipe/SwiftShader），符合
  AGENTS.md「真实 GPU 优先」。
- 适配器字段由各证据文件的 “Adapter (actual run)” 段给出，例如
  `vulkan / Quadro P620 / discrete_gpu / driver NVIDIA`。
- 单机、单次运行；**不是**跨机、vendor 或热降频结论。运行间波动在这些批数下约 ±5%，小幅
  差值落在波动内。

## 1. 数据库 bounds 缓存（host 每步 CPU，真实 DWG）

路径：真实 `deliver_external_open`（acadrust 导入）→ 每步 `InteractionRequest` dispatch
（React host 的 `handle_input` → `dispatch_interaction`）→ preparation settle。**纯 CPU：
不绘制 wgpu 帧、不触碰 GPU**。证据：`1791991784`（before）、`1791992830`、
`1791998671-merge-ratio`。

| Fixture | 实体 | before 每步 mean | after 每步 mean |
|---|---:|---:|---:|
| debug-default-fonts.dwg | 10,656 | 1.43 ms（pan） | 0.006 ms |
| complex-test.dwg | 44,591 | 526 ms（pan）/ 521 ms（zoom） | 0.007 ms（release）/ 0.006 ms（debug） |

`DrawingDatabase::bounds()` 单独诊断：complex-test.dwg before **521.1 ms**（median，3 次）；
after 0.000 ms。before 的跨图元比：pan 369×、zoom 374×（实体数比 4.18×）→ 超线性。
根因：每次视口变化重算 `bounds()`（overlay fingerprint 含 `viewport_bounds`），
allocate + sort + 逐 INSERT 再展开。修复：revision-keyed cache + order-free 累加 +
memoized block bounds。

## 2. 共享 UniformArena（renderer 每帧 CPU，合成 scale-matched）

路径：`Renderer::render(camera, target)` 循环平移帧（每帧 3 px），场景为**合成
scale-matched**（批数 = fixture 实体数，非真实图纸几何；`cad-render-wgpu` 不能依赖
importer）。证据：`1791993747-renderer`（`before.md`/`after.md`/`after-uniform.md`/
`comparison.md`）。`cpu_ms` 排除阻塞 device poll。

| 合成场景（批数） | before 每帧 cpu | after 每帧 cpu | uniform_writes/帧 |
|---|---:|---:|---:|
| debug-default-fonts（10,656） | 35.7 ms | **11.7 ms** | 10,656 → 42 |
| complex-test（44,591） | 158.9 ms | **47.1 ms** | 44,591 → 175 |

归因：一次 ablation 构建**只跳过**逐批 `write_buffer`（其余不变、图像不用）显示其占整帧
CPU **≈55%**（debug ≈20 ms、complex ≈85 ms）。修复：合并为单个共享缓冲 + dynamic offsets。
帧组成（draw_calls / opaque / transparent / invisible）before/after **逐字节一致**
（见 `comparison.md`）。任务记录另称与 HEAD worktree 的帧缓冲为 **55,296/55,296 bytes**，
但本轮证据目录中**未找到**该字节数记录 → **NOT VERIFIED**（仅帧组成逐字节一致可复核）。

## 3. instanced line 流（renderer sweep，合成）

固定几何 88k 线段（176k 顶点）拆成 `B` 个 line 批次，拟合 `cpu_ms = a + b*B`。**合成
scale-matched**，单机单次。证据：`instanced-slice1/after-sweep.md`。

| B 批 | legacy 每帧 cpu | instanced 每帧 cpu | legacy draw | instanced draw |
|---:|---:|---:|---:|---:|
| 1,000 | 0.91 ms | 0.09 ms | 1,000 | 1 |
| 10,000 | 10.33 ms | 0.37 ms | 10,000 | 1 |
| 44,500 | 47.92 ms | 1.87 ms | 44,500 | 1 |
| 88,000 | 98.93 ms | 4.15 ms | 88,000 | 1 |

拟合：legacy `-0.408 ms + 1.120 µs/batch * B`（r²=0.9995）；instanced
`0.030 ms + 0.046 µs/batch * B`（r²=0.9939）。merge fence：合并（B=1）vs 未合并（B=88,000）
渲染**逐字节一致：true**，且对比帧非空。line 实例步长 **56 B**，origin 漂移上限
`MAX_STREAM_RADIUS = 8192 mm`。

## 4. 真实 DWG + 字体（真实场景，P620/Vulkan）

路径：真实 DWG 导入（acadrust）→ font-aware 表示（`with_packed_line_segments`）→
`SceneCache::build_compact` → 真实 headless wgpu → 逐帧 `Renderer::render`。**真实 DWG
几何，非合成**。证据：`p2b-meshmerge-realscene`（instanced + flat-merge）、
`1792005717-realscene`（仅 instanced）、`1791998671-merge-ratio`（merge ratio）。

### draw 数（整幅，无界帧预算）

| Fixture | 实体 | legacy draw | instanced+merged draw | 减少 |
|---|---:|---:|---:|---:|
| debug-default-fonts.dwg | 10,656 | 10,474 | **797**（663 line 段 + 134 mesh 段） | 13.14× |
| complex-test.dwg | 44,591 | 122,006 | **7,627**（6,463 + 1,164） | 16.00× |

（`1792005717-realscene` 仅 line 实例化的中间态为 10,474 → 6,228；flat-merge 后到 797。）

### 每帧 cpu（pan_mid，24 帧，真实 GPU）

| Fixture | legacy 每帧 cpu | instanced+merged 每帧 cpu | draw_calls | uniform_writes |
|---|---:|---:|---:|---:|
| debug-default-fonts.dwg | 17.96 ms | **1.88 ms** | 9,303 → 745 | 37 → 3 |
| complex-test.dwg | 46.74 ms | **5.91 ms** | 22,203 → 1,525 | 87 → 27 |

pan/zoom 期间 `instance_stream_uploads`、`mesh_chunk_uploads` 全程不变 → **零流/chunk 重传**。
字体：仅注册 3 个 face（`__yacr_default__`、`osifont.ttf`）；mlightcad 全字体目录**离线未
拉取**；缺字回退 osifont。

### merge ratio（表示 → draw item）

| Fixture | fragments_in | batches_out | ratio | mergeable lines |
|---|---:|---:|---:|---:|
| debug-default-fonts.dwg | 10,621 | 5,840 | 1.819 | 4,325 |
| complex-test.dwg | 12,498,923 | 97,791 | 127.813 | 5,834 |

## 5. 引擎：input_revision / content_revision 拆分

仅相机变化只推进 `content_revision`，不推进 `input_revision`，平移/缩放期间
`current_ready_scene` 保持 `Some`、`overlay_suffix_commits` 保持 **0**。合同测试
`camera_only_navigation_does_not_churn_the_overlay_suffix` 通过（合成单测；不是 GPU/视觉
验收）。

## 6. React 宿主 cursor-anchored zoom 修复

症状：resize 后 React host 缩放不以光标为锚。根因：canvas metrics 在 session 创建时冻结，
`viewport.logical_size` 与实时 rect 不一致。修复：tick 中同步实时 metrics。相机数学本身
**正确**（`camera.rs:609-664`，测试通过）。属 UI/host 接线修复，非渲染契约变更。

## 绘制顺序 fence（合成/真实混列）

合并路径与旧路径的逐字节一致 fence 在合成 sweep（B=1 vs B=88,000）与 3D perspective fence
下通过；overlapping-tie fence 覆盖重叠且同 `draw_order` 批次。两轮独立 oracle review 的
7 项 must-fix 全部修复后达成。mesh 管线 `depth_compare = Less`、line 管线 `LessEqual`，
均 `depth_write = true`（类间 tie 由该规则裁决，与合并前一致）。

## 软件 / lavapipe 标注

- 上文所有性能数字均为**真实 GPU**，**未**设置 `VK_ICD_FILENAMES`。
- 仅**离线单测**（不属本性能轮成绩）使用钉住的软件 ICD 以求确定性，例如
  `VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json cargo test -p cad-render-wgpu
  -p cad-cli-tools --locked -- --test-threads=1`（lavapipe，软件渲染）。软件后端的耗时
  **不得**用作真实硬件性能结论。

## NOT RUN / 左失败（本轮）

- **全量 workspace 门禁 NOT RUN**：用户 2026-10-15 提效政策——日常只验证改动范围，全量
  测试/门禁仅在用户明确要求或发版前运行。本轮未跑全量 `cargo check/clippy/fmt` 与平台矩阵。
- **WebGL2 / Android / 浏览器视觉验收 NOT RUN**：本轮无 browser/Android 运行验收，无视觉
  golden。
- **`MeshEdges` sentinel gap**：`MeshEdges` 拓扑在合并路径仍为既有 no-op 哨兵（无实际 draw），
  未闭合。
- **deferred 项**（解析弧、glyph outline-keyed instancing、id-readback picking、selection
  flags、加速档 vertex pulling/multi-draw、WebGL2 multi-draw 不支持）未实现。
- **左失败（pre-existing，按用户指示保持失败）**：`/tmp/opencode/yacr-verify/REPORT.md` 将下列
  分类为 HEAD 既有失败，本轮**未修复**：
  - 5 项浏览器检查：mobile label i18n staleness、ribbon viewport、drawing LINE、
    drawing-safety capture、overlay selection flake；
  - `app-linux` 的 `plot_export` / `save_export` / `verify_ui*`。
