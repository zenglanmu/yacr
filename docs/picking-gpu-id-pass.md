# GPU id 回读拾取与选择标志机（P3a-1，渲染器侧）

规范 v2.0 §5.2 / §8.5；审计 F14。本文记录 **渲染器侧** 的 id pass、id→entity 解析、
选择标志机与实测证据。CPU 精确拾取（`cad-spatial` / `cad-app::picking`）仍是**权威参考与
回退路径**，契约见 `docs/picking-3d.md`；本文只补充 GPU 侧。

状态：P3a-1 = 渲染器侧 id pass / 标志机；**P3a-2 = 实体级选择视觉切换到标志机、整实体高亮 overlay 退役**。
P3a-2 修改了 `cad-scene` / `cad-app` / `cad-render-wgpu`，并把生产会话（`cad-engine`
`CadRenderRuntime`）接线到标志机（这是需求 1「在 production session path 打开
`set_selection_flags_enabled(true)`」与需求 5c「选择变化只上传标志数据」的唯一落点，见 §9）。

## 0. P3a-2：实体级视觉走标志机（整实体 overlay 退役）

**已退役**：整实体（whole-entity）选择/高亮不再产生 overlay batch。

- `cad-scene::highlight_batches` 现在**只**处理子元素（网格面）overlay；实体级 ref 直接跳过
  （不产生 batch、不产生诊断——实体级是标志路径，应用层会把它们路由到标志解析器）。
- `cad-app::render_scene::selection_highlight` 只对 `sub_element.is_some()` 的 ref 建
  representation 与 batch；新增 `selection_entity_flags`（同一套 resolve-only-refs 逻辑的
  flags 孪生）把实体级 ref 解析成 `(SelectionRef, bits)`。
- `PreparedScene` 新增 `entity_flags`；控制器用**独立**指纹
  （`overlay_fingerprint` 只含子元素 ref，`entity_flags_fingerprint` 含实体级
  selection/hover/hidden）决定重建哪一部分。因此**实体级选择变化不重建、不重传任何 overlay
  几何**（复用同一个 `Arc<SceneDelta>`），只改标志。
- 生产会话（`cad-engine::CadRenderRuntime`）在 attach 时打开标志，并在
  `apply_ready_updates` 中：overlay `Arc` 指针未变且 base 未变时**只**调用
  `Renderer::set_entity_flags`（跳过任何 batch/stream/chunk 上传）；否则在 commit 后重推标志
  （base 变化会重建 pick key，必须对新表重解析）。
  - overlay 复用判据用 **`Arc::ptr_eq`（保留强引用）**，不是裸地址 `usize`：一个被释放后
    重新分配到同一地址的 `SceneDelta` 会让旧判据 ABA 误判、静默跳过新 overlay 上传。
  - 标志推送在 **base/图像重传之后**：base 变化会重编号 pick key（图像键每次
    `upload_images` 都新分配），先推的标志会留在退役键上，导致被标志的实体/图像在编辑后
    闪回未着色。渲染器同时**保留最后一次标志请求**，在 pick 表重建时按新键重解析（双保险）。
  - `set_entity_flags` 返回 [`EntityFlagReport`]：匹配到 0 个驻留键的 ref 必须由调用方上报为
    `flags.unresolved` 诊断，而不是静默成功（例如隐藏/冻结图层把实体排除出底图时）。

**仍然保留 overlay（有据）**：**子元素高亮**（网格面）与**工具预览**。原因：静态合并流
不随选择变化重传，face 级 flags 不在数据里；用户明确授权「为少量选中项单独 draw 一次」。
`HIGHLIGHT_DRAW_ORDER`（`900_000`）与 `HighlightOptions`（alpha/tint）只为该子元素 overlay
保留。

**语义（有意、简单、已文档化）**：着色器 tint 是**颜色替换**（`SELECTED_TINT`/`HOVERED_TINT`
直接替换 rgb，保留 alpha），不是混合；`FLAG_HIDDEN` 直接 `discard`。这与旧 overlay 的视觉
**不**保证一致（用户已授权），但选中/悬停状态清晰可见。

**已知限制（诚实）**：多源合并 batch 按 **batch 粒度**着色——合并 run 不记录哪段 fragment
属于哪个 source，所以对其任一 source 打标志会给整批着色（
`a_multi_source_batch_is_tinted_at_batch_granularity`）。逐 fragment 解析在合并 run 上不可行。

**实体级标志不作用于子元素 overlay**：`keys_for_source` 只匹配 **整实体** 的 batch source
（`sub_element.is_none()`）。子元素 overlay batch 的 source 是 `E#face-a`，选择整实体 `E`
时**不会**命中它，否则面高亮会被重复着色、且将来的 `FLAG_HIDDEN` 会把面高亮一起丢弃
（`a_sub_element_batch_source_is_not_matched_by_an_entity_flag`）。

**标志变化必须让已完成的 progressive 帧失效**：`set_pick_key_flags` /
`set_selection_flags_enabled` 会丢弃缓存的 progressive 帧。否则 `render` 在同一
frame key 下不再重绘、保留加标志前的像素，选择要等到相机移动才可见
（`a_flag_change_redraws_a_completed_progressive_frame`；生产会话开着 progressive，
这正是引擎侧真机用例抓到的路径）。重绘只复用驻留缓冲，**不**重传几何。

**实体级 hover/hidden 输入**：`OverlayInputs` 新增 `hovered` / `hidden`（默认空）。宿主尚未
产生二者（`cad-host` 传空）；机制与测试已就绪，宿主侧生产是后续切片。

**「只上传标志」的已知例外**：当处于 2D 导航回退（`overlays_suppressed`，即 overlay 后缀被
有意丢弃）时，一次实体级选择变化不再走标志-only 快路径——它会把 overlay 后缀重传并
`overlay_suffix_commits += 1`。这是**有意**的：此时 GPU 上的后缀本就缺失，必须靠一次完整提交
恢复，标志-only 的前提（后缀驻留）不成立。相机-only 导航仍不触碰该计数器。

## 1. id pass（按需，绝不逐帧）

`Renderer::pick_at(x, y)` 触发一次**独立的** id pass：

- 自有 color attachment（id target）与自有 depth buffer，尺寸跟随 color target；
- 绘制**与 color pass 同一批 batch**、同一相机排序（不透明在前、透明在后、图像最后）、同一
  per-batch 变换、同一 winding/cull/depth 状态；
- **不做 run 合并**：color pass 会把相邻的 instanced/merged run 合并成一次 draw，id pass
  **从不合并**（每个 batch、每个 run 各自一个 uniform 携带自己的 pick key），因此一个 fragment
  不可能继承邻批的 key。**代价（已知并列限制）**：等深并列时两者「先画者胜」的集合可能不同
  ——color 合并后的 tie 顺序与 id pass 的逐批顺序，在极少数共面/等深片段上会选出不同赢家。
  因此 parity 的「精确一致率」偏低，一部分是 **GPU-vs-GPU**（两套 draw 组织）差异，不只是
  GPU-vs-CPU 语义差异；
- **不做 progressive 累积、也不受 `charge_images` 预算约束（已知可见性限制）**：id pass 用
  `plan_from_gpu` 画**整个** accepted plan 并画**全部** resident images；而 color pass 在
  progressive 中途只画到当前 cursor、且图像受帧预算裁剪。所以 **progressive 中途或超预算时**，
  pick 可能命中 color pass 当帧尚未画出的几何。这是有意的——pick 面向「完整 resident 页面」
  而非「半成品帧」——但消费者必须知道它与「此刻屏幕上那一帧」不保证同一集合；
- **透明几何的深度状态不同（已知可见性限制）**：id pass 的透明 batch 走
  `depth_write_enabled = true` + `Less` 的 mesh pipeline（id target 不混合，写深让最前片段
  胜出），而 color pass 的透明 pipeline **关写深**以便叠加混合。两者对透明层的可见性因此不
  保证逐片段一致；
- 只回读 **1 个 texel**（`copy_texture_to_buffer` + `map_async` + poll）。

因此 pick 不触碰 color target。id pass 与 color pass **共享同一个 uniform arena**：pick 会写
它自己用到的槽位（`pick.rs` 的 `ensure_slots`/`write_slot`/`flush`）。**逐字节相同**之所以
成立，是因为每个 color 帧都会重写它要画的所有槽位（几何 + 图像），于是被 pick 覆盖过的槽位会
在下一帧被写回。这是一个**依赖**，不是「pick 不写 color pass 的 uniform 槽位」的保证：若将来
出现只写部分槽位的 color 帧，本 fence 不再自动成立（fence：
`the_id_pass_does_not_disturb_the_color_frame`）。

id pass 用「最近一帧的投影」（`render`/`render_3d` 时记录）。尚未渲染任何帧时 `pick_at`
返回 `Ok(None)`——没有可拾取的视图，而不是伪造一个命中。

## 2. pick key 与 id→entity 解析

### key 是渲染器 key，不是截断的 EntityId

`u128` 的 `EntityId` 放不进 32 位 id target。把低 32 位写进 GPU 会让**两个不同实体共享同一个
fragment 值**——静默误拾。因此：

- GPU 写入的 32 位 **pick key = 1 + 批次序号**（`0` = 背景）；由
  `Renderer` 的原子计数器在 `prepare_upload_batches` 分配，跨 `PreparedUpload::append`
  的多个分片也唯一；
- CPU 侧 `PickTable`（`plan.rs`）保存每个 key 的**完整** `SelectionRef` 列表，解析**按构造精确**；
- 低 32 位截断仍被**检测并上报**（`PickTable::collisions` / `Renderer::pick_collisions` /
  `PickHit::key_collision`）：两个**不同**完整身份（含同一实体在不同 INSERT 实例下）共享低
  32 位时记为碰撞。这是给「只有 32 位可用的消费者」的显式告警，而不是误拾来源。

### 多源批次

场景构建会把**相邻的不同实体的线片段拼接进同一个 batch**（`cad-scene` 的 compact merge），
flat-merge 又把这种 batch 串进同一个 chunk。表示后的几何**不记录哪个片段属于哪个 source**，
所以：

- 命中一个多源 batch → `PickHit::source == None`、`candidates` 列出**全部** source、
  `reason == "pick-multi-source-batch"`；
- **绝不**静默取 `sources.first()`。

### 子元素（网格面）

id pass **不解析** mesh face：wgpu 的 fragment stage 拿不到 primitive index。因此
`PickHit::sub_element` 恒为 `None`，`reason` 标注
`pick-sub-element-requires-cpu-refinement`。注意**精确命中也会带这个 reason**——判断
「是否歧义」要用 `PickHit::is_ambiguous()`（等价于 `source.is_none()`），而不是
`reason.is_some()`。子元素拾取仍走 CPU 细化。**不伪造精度**。

## 3. 公开 API

| 项 | 说明 |
| --- | --- |
| `Renderer::pick_at(x, y) -> Result<Option<PickHit>, RenderError>` | 目标像素左上角原点。`Ok(None)` = 背景/未知 key；`Err` = 设备/帧错误（不与「无命中」混淆） |
| `PickHit { entity_key, source, candidates, sub_element, reason, key_collision }` | `entity_key` 为原始 key；`source` 仅在唯一解析时给出完整身份 |
| `Renderer::pick_collisions() -> &[PickCollision]` | 当前场景检测到的低 32 位碰撞 |
| `Renderer::pick_key_for_batch(i)` / `pick_table_len()` | 诊断 |
| `Renderer::id_target_format()` / `set_id_format_override(..)` | 当前/强制 id target 格式（测试钩子） |
| `Renderer::id_format_for_features(features)` | 纯决策函数，可离线复现 |
| `Renderer::set_entity_flags(&[(SelectionRef, u32)]) -> Result<EntityFlagReport, _>` | P3a-2：把实体级 `(ref, bits)` 解析到驻留**整实体** batch 的 pick key 并写标志表；只上传位变化的行、清除掉出集合的行；请求被保留，pick 表重建（base 变化 / 图像重传）后按新键重解析。多源 batch 按 batch 粒度着色；无匹配键的 ref 在 `EntityFlagReport::unmatched` 中返回（调用方须上报 `flags.unresolved`） |
| `Renderer::entity_flag_keys()` | P3a-2：当前带实体级标志的 key（诊断/测试） |

`pick_at` 返回 `Result<Option<..>>`（而非裸 `Option`）：设备丢失/帧错误必须能与「没拾到」
区分，符合本仓库既有 `RenderError` 约定。

## 4. id target 格式：R32Uint 优先，packed RGBA8 回退

`IdTargetFormat::R32Uint`（默认）：WebGPU 规范的 `guaranteed_format_features` 把
`R32Uint` 列为 render attachment，wgpu 的 GL 后端同样对它给出 `COLOR_ATTACHMENT`
（`wgpu-hal/src/gles/adapter.rs`），所以一个 32 位整数通道直接装下整个 key，无需打包。
**注意这是「来源引用，非运行时探测」**：本渲染器不在运行时调用
`Adapter::get_texture_format_features` 去实测 `R32Uint` 是否可渲染，而是走
`id_format_for_features`（读规范保证表 + 设备 features）；上面对 `wgpu-hal` 的引用只是
「为什么 GL 后端也安全」的源码依据，并非本机实测。
`IdTargetFormat::PackedRgba8`：key 拆成四个 `unorm8` 通道（低字节在 `r`），凡能渲染
`Rgba8Unorm` 的层级都能用；`unorm8` 往返是精确的（`/255.0` 后再 `*255.0` 取整可还原每个
字节，单测 `packed_encoding_round_trips_every_byte`）。

选择在 `initialize_with_device` 由设备特性探测决定，并可用
`set_id_format_override` 强制（因此 packed 回退能在支持 `R32Uint` 的硬件上被真实测到：
`the_packed_rgba8_fallback_picks_identically`）。

## 5. 选择标志机（渲染器 API 默认关闭；生产会话已打开）

- 标志表：`Rgba8Uint` 的 `texture_2d<u32>`，**一个 texel 一个 pick key**（超过
  `max_texture_dimension_2d` 时按行折叠，shader 用 `(key % width, key / width)` 寻址）；
  以 `textureLoad` 读取（无 sampler），**WebGL2 安全**；该 claim 由离线 **GLSL ES 3.00
  back-translation** 测试覆盖（`color_shaders_back_translate_to_glsl_es_300`：五个 color
  shader 都经 naga `glsl-out` 成功译出 `#version 300 es`），**非**真实浏览器运行；
- 绑定在 **group 1**（所有几何管线一致；image 的纹理因此移到 group 2）；
- 控制 uniform（16B）：`enabled` + 行宽；
- 着色器 `line` / `mesh` / `line_instanced` / `mesh_merged` / `image` 读标志并施加 tint：
  `FLAG_SELECTED` / `FLAG_HOVERED` 换色，`FLAG_HIDDEN` `discard`；
- **`enabled == 0` 是默认值**，此时输出与加标志前**逐像素相同**——既有高亮 overlay 批次照旧
  工作，不会双重高亮（fence：`flags_disabled_is_pixel_identical_for_every_draw_class` 覆盖
  instanced line / merged mesh / overlay / image 各类，legacy quad 由
  `flags_are_off_by_default_and_a_flag_update_leaves_pixels_unchanged` 覆盖）；
- 更新只写标志表：`set_pick_key_flags(key, bits)` 只上传包含该 key 的**一行**（加 16B 控制
  uniform），不碰静态 instance/merged stream、batch buffer 或 per-batch uniform
  （fence：`a_flag_update_uploads_only_the_flags_table`）；
- **宽度分桶 + 镜像保留**：标志表行宽按 2 的幂分桶（`flags_table_width`），增量提交在同一桶内
  增长 key 空间**不重建**表；跨桶重建时也**保留** CPU 镜像并整表重传，因此已设置的
  SELECTED/HOVERED 位不会因 key 空间增长而清零——宿主**无需**在每次提交后重推全部标志
  （fence：`selection_flags_survive_an_incremental_commit_that_grows_the_key_space`）；
- **重复 key 是硬错误**：`PickTable::build` 对重复 key 返回 `DuplicatePickKey` 而非「先到先得」。
  key 单调分配，重复只可能来自「重置 key 空间时仍有驻留 key」的生命周期漏洞；`clear_batches`
  现在会一并清空驻留图像（fence：
  `a_scene_reset_with_resident_images_does_not_alias_pick_keys`）；
- 场景重置（`clear_batches`）会把整张表**重新上传为零**，退役 key 的位不会残留
  （fence：`a_scene_reset_clears_the_flag_table`）。

着色器里的 tint 常量是**机械占位**，最终选择视觉系统归后续 UI 切片；本文不主张视觉设计。

## 6. 其它不变量

- **uniform 结构 80 → 96 字节**（`mat4x4` + `tint` + `key_flags`）。动态偏移步长仍按
  `min_uniform_buffer_offset_alignment` 对齐（默认 256B），因此每页槽位数不变（fence：
  `the_uniform_struct_grew_for_the_pick_key_without_moving_the_slot_stride`）。
- **`MeshEdges` 仍然什么都不画**：`mesh_edges_batch_currently_renders_nothing` 保留，
  本轮没有静默改变它（id pass 同样沿用 `draw_batch` 的既有语义）。
- **静态流语义变化（有意）**：`LineInstance::id_flags[0]` / `MeshVertex::entity_flags[0]`
  从「首个 source 的 EntityId 低 32 位」改为**渲染器 pick key**。这是 P3a-1 的核心修正，
  已在代码注释与本文档中显式记录。

## 6b. P3b 加速档对 id pass 的影响（组织不变，段内寻址须重定基）

P3b 引入**能力门控的加速档**（color pass 的 opaque 阶段按类折叠为 per-class draw，
逐图元数据从 VS storage buffer 拉取；见 ADR-0007「P3b」节）。对 id pass：

- **id pass 完全不变**：`pick_*.wgsl` 逐字节未动，id pass 仍**不合并** run、每个 batch/run
  各自一次 uniform，仍按需触发。加速档只改变 color pass 的组织，不改变拾取语义。
- **line 类不复制几何（加速档）**：加速档的实例 buffer 同时是
  `VERTEX | STORAGE` 的拉取 storage 段（64 B 步长）。id pass 的 `pick_line_instanced`
  管线在加速档用 **64 B 顶点布局**读取**同一批段 buffer**（属性偏移与 56 B 版一致，shader
  无需改），因此 id pass 与 color 加速档**共用**一份 line 几何，没有「通用路径 + 拉取路径」
  两份拷贝。
  **段内实例偏移必须以 `base + run.offset` 计算**：`run.offset`（`plan.rs` 的
  `build_instance_chunks`）是 **stream 局部**偏移，而加速档一段 buffer 内多条 stream 共享同一
  buffer，`InstanceStream::base` 才是该 stream 在段内的全局起点（color 回退路径同样用
  `base + run.offset`，见 `renderer_impl.rs`）。id pass 曾只喂 `run.offset`，会在多 stream
  段里读错实例/用错 origin；现已在 `pick.rs` 修正，并由 fence
  `accelerated_multi_stream_line_pick_reads_the_right_instance`（一段内两条 stream、各自拾回
  自己的实体）覆盖。
- **mesh 类仍保留各自的 chunk buffer**：merged mesh 的拉取 storage 是**额外**一份（已知内存
  代价，见 ADR-0007）。id pass 继续用 `GpuMeshChunk` 的 per-chunk buffer。
- **超大 stream 回退对 id pass 的影响**：当某条 line stream 超过一个 storage binding 容量、
  无法分段时，line 类**整体回退通用 per-run**，并为每条 stream 单独上传真实的
  `VERTEX | STORAGE`（64 B 步长）缓冲（`rebuild_pulled` 的 `build_line_fallback`）。此时
  id pass 与 color 回退路径读的是**同一批** per-stream 真实缓冲，不会读到 prepare 期的 4 字节
  占位（铁律 #2：降级必须是真实的，不是空成功）。
- **tier 是标签而非隐式**：`Renderer::render_tier()` / `FrameStats::tier` /
  `FrameStats::accelerated` 报告当前档位；一个分页/over-budget 的 color 帧会回退通用
  per-run 并报告 `accelerated=false`，此时 id pass 与 color 的组织重新一致。
- **已知等价例外（诚实）**：加速档的类 draw 先于**残留未转换 opaque batch** 发出（line 类 →
  mesh 类 → 残留 per-batch），而通用档严格按 opaque 批序。因此「已转换类 vs 残留未转换批」
  以及「残留未转换批之间」在**等深并列**处的胜者可能与通用档不同（非重叠/非共面场景不受影响）。
  这与 ADR-0007 记录的镜像分组属同一类已知例外；id pass 本身逐 batch/run 组织，不受该重排影响。
- **未验证**：加速档的 WebGL2 降级路径由离线能力门控 + `tier_for` 单测覆盖，
  **未在真实 WebGL2 浏览器运行**；id pass 本身是 native-only。

## 6c. SDF 弧的 id pass（slice B）

SDF 圆/弧（`RenderTopology::Conics`，见 `docs/adr/0008-sdf-conic-rendering.md`）在 id pass 里
有**自己的** `PickDraw::ArcRun`（镜像 `LineRun`）与 `pick_arc.wgsl` 管线：

- **不合并**：与 line/mesh run 一样逐 run 一次 uniform，携带该 batch 的 pick key；
  段内实例偏移同样以 **`base + run.offset`** 重定基（`base` 恒为 0，弧流不共享 storage 段）。
- **同一 SDF**：`pick_arc.wgsl` 光栅化与 `arc.wgsl` **完全相同**的距离场。二者都用三角形滤波
  `coverage = saturate(1 − d/w)`（`w` = 1 屏像素的 `world_per_px`，峰值 1.0）；pick 在字面的
  `coverage < 0.5` 处丢弃，因此 `coverage ≥ 0.5 ⇔ d ≤ 0.5·w`，即 **±0.5 px 拾取容差**。
- **实测**：`crates/cad-render-wgpu/tests/sdf_arcs.rs::sdf_arc_id_pass_parity`（真实
  Quadro P620 / Vulkan）沿一条穿越环的扫描线**双向**断言 `coverage ≥ 0.55` 可拾取、
  `coverage ≤ 0.45` 不可拾取，背景像素不可拾取。

## 7. 实测证据

命令（真实 GPU，默认 Vulkan ICD，未设 `VK_ICD_FILENAMES`）：

```bash
cargo test -p cad-render-wgpu --locked -- --test-threads=1
YACR_PICK_PARITY_GRID=12x9 cargo test -p cad-cli-tools --test pick_parity --locked -- --ignored --nocapture
```

- 适配器（本轮重跑实测）：`Quadro P620 (vulkan, discrete_gpu)`，`id_format=r32uint`。
- `crates/cad-render-wgpu`（P3a-2 本轮重跑，上面第一行命令）：108 lib + 19 headless + **28**
  pick_id + 44 render_effects + 8 wgsl_validation 全绿（`pick_id` 单跑
  `--test pick_id -- --test-threads=1 --nocapture` exit 0，每个用例打印适配器
  `Quadro P620 (vulkan, discrete_gpu)`）。新增 7 个实体级 flags 用例：source→key 解析、
  INSERT 实例只命中该 placement、多源 batch 按 batch 粒度、selected/hovered 可见 tint 且
  hidden 不画、instanced line 被 tint、选择变化只上传标志数据（stream/chunk 计数不变）、
  标志开启但为空逐像素不变；另加 2 个 `plan.rs` 纯数据单测（`keys_for_source`）。
- `crates/cad-scene` / `crates/cad-app` 单测：`cargo test -p cad-scene -p cad-app --locked
  -- --test-threads=1` 全绿（cad-scene 56；cad-app 348）。`cargo check -p cad-engine
  -p cad-host --all-targets` 通过；`cargo clippy -p cad-scene -p cad-app -p cad-render-wgpu
  --all-targets -- -D warnings` 通过；`scripts/check-architecture.py` OK。
- **未运行**：`cad-engine` 的既有真实 GPU 会话用例（`base_only_*` / `pending_overlay_*` /
  `camera_only_*`）本轮未重跑；parity harness 本轮未重跑。旧数字保留仅供追溯。**本轮新增**
  两个 `cad-engine` 真机（P620/Vulkan）`#[ignore]` 用例并已运行通过：
  `selection_flags_only_fast_path_tints_without_reuploading_geometry`（选择 A→着色且只涨
  标志计数，切到 B→A 退色 B 着色，stream/chunk/suffix 计数不变；含 B1 的 overlay `Arc`
  保留断言）与 `a_selected_entity_excluded_from_the_base_reports_a_diagnostic`（隐藏图层
  排除的选中实体产出 `flags.unresolved` 诊断）。

parity harness（真实 DWG，**渲染器 id pass** vs **CPU `cad_spatial` 参考**）：

| 夹具 | 批次 | 采样 | 双方都命中 | 精确一致 | CPU 可复现 GPU 实体 | GPU-only | 不可解释冲突 | GPU 单次 pick |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `debug-default-fonts.dwg` | 10474 | 107 | 107 | 0.579 | **1.000** | 0 | **0** | 11.5 ms |
| `complex-test.dwg` | 122006 | 107 | 107 | 0.103 | **1.000** | 0 | **0** | 64.1 ms |

- 采样取自 color pass **实际绘制**的像素（均匀网格在大图上几乎全落在空白处，说明不了问题）。
- 精确一致率偏低的原因已定位：图上存在大量**共面/等深**线条。GPU 在等深时由
  `LessEqual` 保留**先画**的片段，而 CPU 参考按「离射线最近」在等深处用**垂距**决胜——
  两者对等深并列的偏好不同。这类冲突被归类为 `ranking`（CPU 在同像素同容差下**也能**
  看到 GPU 报的实体）。`complex-test.dwg` 的 glyph-mesh 类精确一致为 5/5。
- **不可解释冲突为 0**：没有任何一个采样是「GPU 报了一个 CPU 在同像素同容差下完全看不到
  的实体」。这是 id pass 正确性的关键指标，也是 harness 断言的部分。
- harness **不**断言精确一致率：它受等深并列与容差排序影响，属于两套语义的固有差异。
  CPU 拾取按设计仍是主路径/回退；GPU id pass 是可选加速，冲突一律打印，绝不掩盖。

## 8. 明确未做 / 回退

- **子元素（网格面）拾取**：id pass 不做，回退 CPU 细化（见 §2）。
- **Web 层级**：`pick_at` 为 native-only（浏览器无法阻塞回读）；浏览器/WebGL2 继续用 CPU
  拾取。id target 格式的回退路径已实现并测试，但**未在真实 WebGL2 浏览器上运行**；标志块的
  「WebGL2 安全」由离线 **GLSL ES 3.00 back-translation** 测试覆盖
  （`color_shaders_back_translate_to_glsl_es_300`），同样**不是**真实浏览器运行。
- **Android 真机 / 模拟器**：未验证。
- **整实体高亮退役与生产接线**：**P3a-2 已完成**（见 §0）：整实体 overlay 退役、生产会话走
  标志机、选择变化只上传标志数据。子元素 overlay 与工具预览仍保留（有据）。
- **id pass 忽略 `FLAG_HIDDEN`（必须的后续项）**：`pick_*.wgsl` 写的是**原始 pick key**，不读
  标志表，所以被 `FLAG_HIDDEN` 的实体会**继续可拾取并遮挡**其它几何（color pass 已 `discard`，
  id pass 没有）。当前 **hidden 生产者尚未接线**（`cad-host` 传空的 `hidden`），因此该缺陷
  暂不可见；但 **hidden 切片落地前必须**让 id pass 也按 key 查标志表并跳过 `FLAG_HIDDEN`
  的片段（否则「隐藏」实体仍会被拾取，违反铁律 #2 的显式建模要求）。本切片**不**做投机实现，
  仅在此记录为硬性前置项。
- **实体级 hover / hidden 的宿主生产**：机制与 `OverlayInputs::hovered`/`hidden` 已就绪，
  宿主尚未产生（`cad-host` 传空）；属后续宿主切片。
- **视觉 tint**：`SELECTED_TINT`/`HOVERED_TINT` 仍是机械常量（颜色替换，非混合），最终视觉
  归 UI 切片；与旧 overlay 的视觉**不**保证一致（用户已授权）。
- **逐帧 id pass**：不做（按需）。
- `docs/picking-3d.md`、`docs/handoff.md` 的交叉引用与交接条目本轮未更新（P3a-2 作用域限定为
  渲染器/scene/app/engine 源码 + 测试 + 本文档 + ADR-0007 deferred 列表）。
