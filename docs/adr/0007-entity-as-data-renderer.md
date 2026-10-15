# ADR 0007：渲染器 “entity = data” 演进（类实例化 + 平铺合并）

状态：已实现并通过两轮独立 oracle review gate（7 项 must-fix findings 全部修复）。真实
NVIDIA Quadro P620 / Vulkan 实测；契约变更经用户 2026-10-15 授权。**未**做 WebGL2/Android/
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

## 明确推迟（deferred）

以下项本轮**未实现**，须显式建模为未支持，不得当作已完成：

- **解析弧**（analytic arcs）：弧仍先离散为线段；无解析弧的实例化路径。
- **glyph outline-keyed instancing**：文字轮廓仍是主要残留 draw（参考图 instanced 帧中
  glyph mesh 占 4,630 / 6,228），未按 outline 键做实例化。
- **id-readback picking**：`entity_flags` 已写入顶点，但**没有** id 回读 picking 通路。
- **selection flags**：**P3a-2 已闭环**。实体级选择/悬停/隐藏改由渲染器的 in-stream 标志机
  承载（`FLAG_SELECTED`/`FLAG_HOVERED`/`FLAG_HIDDEN`，`Renderer::set_entity_flags` 按
  `PickTable` 的 source→key 解析写入；静态流/chunk 不随选择变化重传），整实体高亮 overlay
  退役。**例外（保留 overlay）**：子元素（网格面）高亮——face 级 flags 不在静态合并流数据里，
  静态流不随选择变化重传，故 face 仍需一次小的独立 draw（用户已授权）。多源合并 batch 按
  batch 粒度着色（已知限制）。见 `docs/picking-gpu-id-pass.md` §0。
- **加速档（accelerated tier）**：vertex pulling / multi-draw 等更强合并档未做；且 **WebGL2
  不支持** multi-draw，属 **capability-gated future**。
- **MeshEdges sentinel gap**：`MeshEdges` 拓扑在当前合并路径仍是**既有 no-op 哨兵**
  （无实际 draw），该缺口未闭合。

## 后果

- draw 数大幅下降，pan/zoom 的每帧 CPU 从随批数线性/超线性变为近似常数：合成 scale-matched
  场景每帧 cpu 35.7→11.7 ms（10,656 批）、158.9→47.1 ms（44,591 批，UniformArena）；
  真实 DWG + 字体 debug-default-fonts 17.96→1.88 ms、complex-test 46.74→5.91 ms。
- 静态/动态拆分使 pan/zoom 期间零流/chunk 重传；相机-only 不再失效已发布场景。
- 合并路径与旧路径在 fenced 场景逐字节一致；**未**做浏览器/Android/视觉 golden 验收，不得
  外推为跨平台或完整 CAD 视觉验收。
- 上述 deferred 项与 `MeshEdges` 缺口必须在 capability/兼容表中保持显式未支持状态。

## 证据

- 性能与适配器证据：`/tmp/opencode/yacr-perf/`（`1791991784`、`1791992830`、
  `1791993747-renderer`、`1791997393-step2-phases`、`instanced-slice1`、
  `p2b-meshmerge-realscene`、`1792005717-realscene`、`1791998671-merge-ratio`）。
- 汇总表见 `docs/validation-render-perf-2026-10-15.md`。
- 绘制顺序现行语义见 `docs/render-order.md`。
