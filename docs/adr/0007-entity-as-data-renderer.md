# ADR 0007：渲染器 “entity = data” 演进（类实例化 + 平铺合并）

状态：已实现并通过两轮独立 oracle review gate（7 项 must-fix findings 全部修复）。真实
NVIDIA Quadro P620 / Vulkan 实测；契约变更经用户 2026-10-15 授权。**P3a GPU id 回读拾取 +
实体级选择标志机已实现**（见 `docs/picking-gpu-id-pass.md`），**P3b 能力门控加速档
（vertex pulling，单类 draw）已实现并实测**（见「P3b」节），**SDF 圆/弧（解析弧）已实现**
（见 `docs/adr/0008-sdf-conic-rendering.md`）。**未**做 WebGL2/Android/
浏览器视觉验收，也未做 vendor/golden 结论。

## 背景与问题

2026-10-15 性能排查（证据目录 `/tmp/opencode/yacr-perf/`，见
`docs/validation-render-perf-2026-10-15.md`）确认三个相互独立的问题：

1. **平移/缩放 CPU 成本随实体数超线性增长**：44,591 实体的 complex-test.dwg 每步
   526 ms。根因是 `DrawingDatabase::bounds()` 在每次视口变化时被重算（overlay fingerprint
   含 `viewport_bounds`），且实现为 allocate + sort + 逐 INSERT 再展开，其中
   `bounds()` 单独 521 ms/步（526 中的 521）。
2. **每批一次 `queue.write_buffer`**：一帧内逐批上传 uniform，约占整帧 CPU 的 **55%**
   （44,591 批时 44,591 次/帧）。
3. **仅相机变化的失效**：相机变化会失效已发布场景，导致 base-only/全量帧交替与 overlay
   抖动。

用户在 2026-10-15 明确导向渲染架构 **“entity = data”**：几何以数据流承载、尽量合并 draw，
并授权相应契约变更。本轮在 P2 阶段把 draw 数从 10,474 降到 797（参考图，见下）；P3 待用户
决定。

## 决策

### 类实例化（line / segment 流）

- 把可合并的不透明 **line 批次**（`Lines` 批次，以及合并 mesh 的边 overlay）折叠为
  **instanced line 流**：每段一个 `LineInstance`（**56 B** 步长），在
  `line_instanced.wgsl` 中实例化绘制。
- 每个 chunk/流带 **f64 origin**；段端点相对该 origin 以 `select()` 精确选取，避免大坐标下
  f32 相对误差。origin 漂移受 **`MAX_STREAM_RADIUS = 8192 mm`** 约束（超出即分块/分段）。

### mesh 平铺合并

- 把相邻、同管线的不透明 **`Mesh` 批次**平铺合并为 **interleaved vertex 缓冲**（**44 B**
  步长），每段连续三角形以一次 indexed draw 提交（`build_mesh_chunks` /
  `mesh_merged.wgsl`）。
- 每顶点携带 **`entity_flags`**（存首个来源的 `EntityId` 低位，供后续 picking 阶段）与
  **per-vertex colour**；逐批 tint 烘进顶点色。**仅当**网格为常量色或白色 tint 时，结果与
  旧的逐像素乘积**逐字节一致**。
- 合并 mesh 的**边 overlay**同样折叠进 instanced line 流，不额外增加 draw。

### 静态 / 动态拆分

- **静态几何**（instanced 流与 merged chunk）在 `upload()` 时一次性常驻；平移/缩放等
  **仅相机**变化**不重传**任何流/chunk（`instance_stream_uploads`、`mesh_chunk_uploads`
  在 pan/zoom 全程保持不变）。
- **动态数据**（逐帧相机/矩阵）经共享 **`UniformArena`** 以 **dynamic offsets** 写入，
  取代每批一次 `write_buffer`。一帧的 uniform 写入数由批数降为 **O(pages)**。

### 契约边界：per-chunk f64 origin 限定于 Ortho

- per-chunk f64 origin 的精度契约**只在正交（Ortho）相机下成立**；透视（3D perspective）
  路径有独立 fence，不沿用该保证。合并/实例化路径在 3D 透视下须与旧路径保持绘制顺序与
  视觉一致（见下），但**不**宣称 f64 origin 精度在透视下等价。

### 绘制顺序 tie 语义保持不变

合并会改变“每批一次 draw”的粒度，因此必须证明不改变视觉顺序：

- **类内合并**：只合并同管线、同不透明类别、**相邻绘制顺序**的批次；跨顺序/跨拓扑不合并。
- **类间深度规则（文档化推理）**：mesh 管线 `depth_compare = Less`、`depth_write = true`；
  line 管线 `depth_compare = LessEqual`、`depth_write = true`。合并后同一 run 内的图元顺序
  即原上传顺序，因此类内 tie 仍确定；类间由既有 Less/LessEqual + 深度写入规则裁决，与合并
  前一致。
- **fences**：
  - **merged-vs-legacy 逐字节一致**（f32 可表示的共享 origin 下）；
  - **overlapping-tie fence**（重叠且同 `draw_order` 的批次）；
  - **3D perspective fence**（透视相机下的顺序）。

两轮独立 oracle review 的 7 项 must-fix 已全部修复后，合并路径与旧路径在上述 fenced 场景
逐字节一致。

### 数据库侧（bounds 缓存）

- `DrawingDatabase::bounds()` 改为 **revision-keyed cache** + **order-free 累加** +
  **memoized block bounds**。每步：526 ms → **0.007 ms**（complex，release）；
  1.43 → 0.006 ms（debug）。缓存随 DB revision 失效，不改变语义。

### 引擎侧（input_revision / content_revision 拆分）

- 引入 **`input_revision`**（场景身份：底图/内容）与 **`content_revision`**（准备 token）
  两个修订。仅相机变化只推进 `content_revision`，**不**推进 `input_revision`，因此平移/缩放
  期间 `current_ready_scene` 保持 `Some`，不触发 base-only/全量交替；`overlay_suffix_commits`
  保持 **0**。
  （2026-10-25 更正：**overlay-only** 变化——例如普通鼠标移动改 cursor crosshair——过去仍会进入
  base-only 截断路径并逐帧丢失悬停高亮；该路径已退役，见 ADR-0012 §10 S2 修订。现在任何帧都呈现
  完整 resident 场景，新 overlay 输入未提交时以 `FrameCompleteness::PendingOverlayInputs` 标记。）

## 明确推迟（deferred）

以下项本轮**未实现**，须显式建模为未支持，不得当作已完成：

- **解析弧**（analytic arcs）：**slice B 已实现为 SDF 圆/弧实例**（见
  `docs/adr/0008-sdf-conic-rendering.md`）。可 SDF 化的圆/弧（不透明、连续线型、slice-1 圆形）
  改由解析 SDF 实例绘制；**椭圆（slice 2）仍弦展开**。这是语义像素变化（更平滑），已获用户
  视觉验收授权。
- **glyph outline-keyed instancing**：文字轮廓仍是主要残留 draw（参考图 instanced 帧中
  glyph mesh 占 4,630 / 6,228），未按 outline 键做实例化。
- **id-readback picking**：**P3a-1 已实现**（`52ab9d5`，修复 `0f193dc`）：`Renderer::pick_at`
  按需渲进独立 id target、回读 32 位 pick key，经 CPU `PickTable` 解析**完整** `SelectionRef`
  （低 32 位截断检测并上报，多源批次给全部候选）；`LineInstance::id_flags[0]` /
  `MeshVertex::entity_flags[0]` 由「首个 source 的 EntityId 低位」改为**渲染器 pick key**。
  见 `docs/picking-gpu-id-pass.md`。CPU 拾取仍是主路径/回退。
- **selection flags**：**P3a-2 已闭环**。实体级选择/悬停/隐藏改由渲染器的 in-stream 标志机
  承载（`FLAG_SELECTED`/`FLAG_HOVERED`/`FLAG_HIDDEN`，`Renderer::set_entity_flags` 按
  `PickTable` 的 source→key 解析写入；静态流/chunk 不随选择变化重传），整实体高亮 overlay
  退役。**例外（保留 overlay）**：子元素（网格面）高亮——face 级 flags 不在静态合并流数据里，
  静态流不随选择变化重传，故 face 仍需一次小的独立 draw（用户已授权）。多源合并 batch 按
  batch 粒度着色（已知限制）。见 `docs/picking-gpu-id-pass.md` §0。
- **加速档（accelerated tier）**：**P3b 已实现**（见下节）。vertex pulling 的单类 draw
  已完成；**multi-draw indirect 未做**（不在 WebGPU core，故意不架构其上），**WebGL2 不支持**
  VS storage buffer，故按能力门控降级。
- **MeshEdges sentinel gap**：`MeshEdges` 拓扑在当前合并路径仍是**既有 no-op 哨兵**
  （无实际 draw），该缺口未闭合。

## P3b：能力门控的加速档（vertex pulling，单类 draw）

**状态**：已实现并通过本机真实 GPU（Quadro P620 / Vulkan，未设 `VK_ICD_FILENAMES`）验证。
契约变更经用户 2026-10-15 授权；仅渲染器 crate 源码/测试/shaders + 本文档 + ADR-0007 变更。

### 设计

- **tier 选择**：`initialize_with_device` 读取**设备自身**的 backend/limits 决定
  [`RenderTier`]：需要 VS 只读 storage buffer（WebGL2 无此能力）、
  `max_storage_buffers_per_shader_stage >= 3`（真实需求：`mesh_pulled.wgsl` 一次绑 3 个
  只读 storage——顶点、索引、chunk 表；`line_pulled.wgsl` 绑 2 个。旧门限 `< 1` 会让只报
  1–2 个 binding 的设备通过、随后在建 pipeline 时失败）、
  `max_storage_buffer_binding_size >= 64 KiB`。**从不向宿主设备请求 feature**；不满足即
  **静默但带标签**降级（`Renderer::render_tier()` / `FrameStats::tier` /
  `FrameStats::accelerated`）。拉取资源建不起来时（例如缺少 flags layout）tier 也在此
  **降级**（`finalize_tier`），不会以 `Accelerated` 之名缺拉取通路。测试钩子
  `set_storage_buffer_support_override(Some(false))` 可在有能力设备上复现降级。
- **单类 draw**：color pass 的 opaque 阶段把已转换的 line 类与 merged mesh 类各折叠为
  **每段一次 draw**（不是每 chunk 一次）。逐图元数据从**只读 VS storage buffer** 拉取
  （`shaders/line_pulled.wgsl` / `shaders/mesh_pulled.wgsl`），并用
  `@builtin(instance_index)` / `@builtin(vertex_index)` 索引。
- **精度契约不变**：per-chunk f64 origin 仍在 **CPU 上传时**做 `(world - origin) as f32`
  约简；shader 只把**半径受限的 f32** 位置加到 per-chunk 平移列上。平移列 =
  `FrameProjection::batch_matrix_f64(origin)` 的**最后一列**（仍是 CPU f64 计算），
  camera uniform 只带**与 origin 无关的前三列**，故 `mat4x4<f32>(c0,c1,c2,t)` 与通用路径的
  per-chunk 矩阵**逐位相同**，乘法的 f32 运算序列一致。
- **line 类不复制几何**：加速档的实例 buffer 同时是 `VERTEX | STORAGE` 的拉取 storage
  段（64 B 步长，属性偏移与 56 B 版一致，故 `line_instanced.wgsl` 与
  `pick_line_instanced.wgsl` **无需修改**）。段按整条 stream 边界切分（每段 ≤
  `max_storage_buffer_binding_size`），一段一次 draw。**id pass 与分页回退仍走通用 per-run
  路径**，绑同一批段 buffer；段内实例偏移由 `InstanceStream::base` 给出，id pass 与 color
  回退路径都以 **`base + run.offset`** 寻址（`run.offset` 是 stream 局部偏移，`base` 才是段内
  全局起点）。**唯一例外**：当某条 stream 本身超过一个 binding 容量、无法在不切断 run 的前提
  下分段时，line 类**整体回退通用 per-run**，并为每条 stream 单独上传一份真实的
  `VERTEX | STORAGE`（64 B 步长）缓冲——此时 line 几何确有第二份拷贝，但它是回退路径的真实
  数据，不是被隐藏的优化，也绝不画 prepare 期的 4 字节占位。
- **mesh 类**：仍额外保存一份 whole-scene 拉取 storage（64 B/顶点 + 4 B/索引），
  **已知内存代价**（见下）。mesh chunk 的 per-chunk buffer 始终存在，故 mesh 类回退时通用
  per-run 路径始终有真实几何。
- **分页回退**：per-class draw 覆盖**全部**已转换 batch，因此只有当本帧接受了全部已转换
  batch 时才走加速档；否则（progressive 分页 / over-budget）**回退通用 per-run**，绝不拉取
  被预算挡住的几何。此时 `FrameStats::accelerated == false`（显式报告）。
- **id pass 不变**：`pick_*.wgsl` 字节未动，id pass 仍按 batch/run 各自 draw（速度无关）。

### 等价性（真实 GPU 实测，逐像素）

`crates/cad-render-wgpu/tests/accelerated_tier.rs`（21 用例，Quadro P620 / Vulkan 全绿）在
**同一场景上同时跑加速档与通用档**并断言逐字节相同：instanced lines、merged meshes、
mirrored merged meshes、混合类、大坐标、深缩放 mixed-origin、3D 透视、重叠 tie、
选择/隐藏标志、分页回退（回退页重建出的帧等于加速参考帧）、**强制多段**（`set_pulled_line_segment_elems_override`
把 line 类切成多段后仍逐字节一致）、**多 stream 拾取**（一段内 ≥2 条 stream，各自拾回自己的实体，
fence B1 的 `base + offset` 重定基）、**超大 stream 降级**（reduced-limit 下不产生非法 binding、
回退帧与通用档一致）、**字节记账**（拉取几何只记一次）。另有能力降级用例断言降级后**一像素不差**。

诚实边界：
- mirrored 与 non-mirrored 合并 chunk 在加速档被分组（先 non-mirrored 后 mirrored），
  **跨镜像类的等深 tie 顺序**与通用档可能不同（非重叠场景已 fenced；本文不外推逐字节）。
  通用档/加速档之间的等价只在**算术一致处**成立。
- **加速档的类 draw 先于残留未转换 opaque batch 发出**（line 类 → mesh 类 → 残留 per-batch），
  而通用档严格按 opaque 批序；因此「已转换类 vs 残留未转换批」以及「残留未转换批之间」在
  **等深并列**处的胜者可能与通用档不同（非重叠/非共面场景不受影响，已在像素 fence 中覆盖）。
  这类 tie 差异与 ADR 已记录的镜像分组属同一类**已知等价例外**，不外推逐字节。

### 真实场景实测（P620 / Vulkan，`crates/cad-cli-tools/tests/realscene_render_perf.rs`）

| 夹具 | legacy draws | 通用（转换后）draws | 加速档 draws | pan_mid cpu 通用 → 加速 |
| --- | --- | --- | --- | --- |
| `debug-default-fonts.dwg` | 10474 | 903 | **3** | 2.37 → **1.85 ms** |
| `complex-test.dwg` | 122006 | 8252 | **19** | 8.00 → **6.72 ms** |

- 加速档实测为 `tier=accelerated`、`accelerated=true`；`instanced_draws` 3 / 19 分别对应
  line 段（1 / 15）+ merged mesh 段（2 / 4）。相对通用 per-run 分别少 **301× / 434×** draw。
- 合成 sweep（`render_perf.rs`）同样报告 tier：B=88000 时 legacy 88,000 draws / 129.5 ms →
  加速档 1 draw / 6.59 ms。
- 诚实标签：这是**单机、单适配器、debug-profile** 数字；`complex-test` 加速档 6.72 ms 与
  旧文 `5.91 ms` 不同量测口径，不构成跨版本回归结论。分页/over-budget 帧仍回退通用
  （报告 `accelerated=false`）。
- **内存（诚实、已知代价）**：拉取的 line storage 是实例 buffer 的**同一份**（不复制）；
  唯一的例外是**超大 stream 回退**（见上），此时 line 类确有 per-stream 的真实缓冲。
  merged mesh 仍额外一份拉取 storage。`complex-test` 的 line 实例约 2,945 万条，整机 4 GB
  VRAM 下必须在 `rebuild_pulled` 里按 `max_storage_buffer_binding_size` 分段；
  `max_buffer_size` 仅 256 MiB，故**不能**用单一大 buffer 共享。因此 mesh 类的复制是当前
  实现的**残余成本**，不是被隐藏的优化。

### 字节记账 / 静态性

- 拉取 storage 的字节按 batch 计入 `upload_size_bytes`（line 段 64 B/实例，mesh 拉取
  64 B/顶点 + 4 B/索引），与其它缓冲一致地进入帧预算。**lifetime `uploaded_bytes` 只在
  `prepare_upload_batches` 记一次**：`rebuild_pulled` 只把每类的 chunk 表（`n × 16 B`）计入，
  不再重复累加段缓冲，否则同一份拉取几何会被记约 2×（line）/更高倍（mesh）。已知的有界例外：
  一个因超限而未建成的 mesh chunk 仍在 prepare 处被记过它的拉取份额，故该类可能被高报。
- **静态流不随相机重传**：相机移动只重写小的 per-chunk 平移表（storage，等价于通用档的
  per-stream uniform 槽）+ 一个 48 B camera uniform。fence
  `accelerated_camera_move_does_not_reupload_static_storage` 断言
  `instance_stream_uploads` / `mesh_chunk_uploads` 在 10 帧 pan 全程不变。
- **`keep_prefix` 增量提交不是增量的（诚实记录）**：任何 commit（包括保留前缀的批注追加）都会
  `rebuild_pulled` **整场景**并**重传全部拉取段缓冲**，即每次 commit 是
  `O(总驻留几何)` 的 GPU 上传，而不是 `O(追加部分)`。这是**已知代价**，不是被隐藏的回归；
  增量段更新是 deferred 优化（见 `docs/render-optimization-backlog.md`）。
- 残余：透明批次、图像、`MeshEdges` 哨兵、以及**未被接受的**已转换 batch 仍走通用
  per-run；未转换的 opaque 批次（本例中 `instanced_batches < opaque`）每 batch 一次 draw。
- 上述 deferred 项与后续优化项（视锥剔除、LOD、SDF 弧、加速档等）的完整清单见
  `docs/render-optimization-backlog.md`。

## 后果

- draw 数大幅下降，pan/zoom 的每帧 CPU 从随批数线性/超线性变为近似常数：合成 scale-matched
  场景每帧 cpu 35.7→11.7 ms（10,656 批）、158.9→47.1 ms（44,591 批，UniformArena）；
  真实 DWG + 字体 debug-default-fonts 17.96→1.88 ms、complex-test 46.74→5.91 ms。
- P3b 加速档进一步把已转换的 opaque 类折叠为**每类每段一次 draw**：真实场景
  debug-default-fonts 903→3、complex-test 8252→19 draw（相对通用 per-run 少 301×/434×），
  pan_mid cpu 2.37→1.85 ms / 8.00→6.72 ms（同轮同机实测）。这是**能力门控**档：WebGL2
  自动降级，宿主设备不被请求任何 feature。
- 静态/动态拆分使 pan/zoom 期间零流/chunk 重传；相机-only 不再失效已发布场景。加速档保持
  同一不变量（只重写 per-chunk 平移表 + 48 B camera uniform）。
- 合并路径与旧路径在 fenced 场景逐字节一致；加速档与通用档在算术一致处逐字节一致
  （`tests/accelerated_tier.rs`，真实 P620）。**未**做浏览器/Android/视觉 golden 验收，不得
  外推为跨平台或完整 CAD 视觉验收。
- 已知残余/代价：mesh 类仍额外一份拉取 storage；分页/over-budget 帧回退通用 per-run；
  未转换 opaque 批次与透明/图像/`MeshEdges` 仍逐项 draw。这些在 capability/兼容表中保持
  显式状态。

## 证据

- 性能与适配器证据：`/tmp/opencode/yacr-perf/`（`1791991784`、`1791992830`、
  `1791993747-renderer`、`1791997393-step2-phases`、`instanced-slice1`、
  `p2b-meshmerge-realscene`、`1792005717-realscene`、`1791998671-merge-ratio`）。
- P3b 加速档证据：`/tmp/opencode/yacr-p3b-realscene/`（真实 DWG，适配器
  `Quadro P620 / vulkan / discrete_gpu / driver NVIDIA 580.178.04`）、
  `/tmp/opencode/yacr-p3b-synth/`（合成 sweep / frame cpu）。逐像素等价 fence 见
  `crates/cad-render-wgpu/tests/accelerated_tier.rs`（21 用例，真实 P620 全绿）。
- 汇总表见 `docs/validation-render-perf-2026-10-15.md`。
- 绘制顺序现行语义见 `docs/render-order.md`。
