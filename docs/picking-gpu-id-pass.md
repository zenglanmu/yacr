# GPU id 回读拾取与选择标志机（P3a-1，渲染器侧）

规范 v2.0 §5.2 / §8.5；审计 F14。本文记录 **渲染器侧** 的 id pass、id→entity 解析、
选择标志机与实测证据。CPU 精确拾取（`cad-spatial` / `cad-app::picking`）仍是**权威参考与
回退路径**，契约见 `docs/picking-3d.md`；本文只补充 GPU 侧。

状态：P3a-1 = **仅渲染器侧**（`crates/cad-render-wgpu`）。高亮体系退役与宿主接线由 P3a-2
负责；`cad-scene` / `cad-app` / `cad-engine` 本轮**未改动**。

## 1. id pass（按需，绝不逐帧）

`Renderer::pick_at(x, y)` 触发一次**独立的** id pass：

- 自有 color attachment（id target）与自有 depth buffer，尺寸跟随 color target；
- 绘制**与 color pass 相同的可见几何**：同一批 batch、同一相机排序（不透明在前、透明在后、
  图像最后）、同一 per-batch 变换、同一 winding/cull/depth 状态；
- **不做 run 合并**：每个 batch（及其每个 run）用自己的 uniform 携带自己的 pick key，
  因此一个 fragment 不可能继承邻批的 key；
- 只回读 **1 个 texel**（`copy_texture_to_buffer` + `map_async` + poll）。

因此 pick 不触碰 color target、不写 color pass 的 uniform 槽位：**pick 后的一帧与从未 pick
过的一帧逐字节相同**（fence：`the_id_pass_does_not_disturb_the_color_frame`）。

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
`pick-sub-element-requires-cpu-refinement`，子元素拾取仍走 CPU 细化。**不伪造精度**。

## 3. 公开 API

| 项 | 说明 |
| --- | --- |
| `Renderer::pick_at(x, y) -> Result<Option<PickHit>, RenderError>` | 目标像素左上角原点。`Ok(None)` = 背景/未知 key；`Err` = 设备/帧错误（不与「无命中」混淆） |
| `PickHit { entity_key, source, candidates, sub_element, reason, key_collision }` | `entity_key` 为原始 key；`source` 仅在唯一解析时给出完整身份 |
| `Renderer::pick_collisions() -> &[PickCollision]` | 当前场景检测到的低 32 位碰撞 |
| `Renderer::pick_key_for_batch(i)` / `pick_table_len()` | 诊断 |
| `Renderer::id_target_format()` / `set_id_format_override(..)` | 当前/强制 id target 格式（测试钩子） |
| `Renderer::id_format_for_features(features)` | 纯决策函数，可离线复现 |

`pick_at` 返回 `Result<Option<..>>`（而非裸 `Option`）：设备丢失/帧错误必须能与「没拾到」
区分，符合本仓库既有 `RenderError` 约定。

## 4. id target 格式：R32Uint 优先，packed RGBA8 回退

`IdTargetFormat::R32Uint`（默认）：WebGPU 规范的 `guaranteed_format_features` 把
`R32Uint` 列为 render attachment，wgpu 的 GL 后端同样对它给出 `COLOR_ATTACHMENT`
（`wgpu-hal/src/gles/adapter.rs`），所以一个 32 位整数通道直接装下整个 key，无需打包。
`IdTargetFormat::PackedRgba8`：key 拆成四个 `unorm8` 通道（低字节在 `r`），凡能渲染
`Rgba8Unorm` 的层级都能用；`unorm8` 往返是精确的（`/255.0` 后再 `*255.0` 取整可还原每个
字节，单测 `packed_encoding_round_trips_every_byte`）。

选择在 `initialize_with_device` 由设备特性探测决定，并可用
`set_id_format_override` 强制（因此 packed 回退能在支持 `R32Uint` 的硬件上被真实测到：
`the_packed_rgba8_fallback_picks_identically`）。

## 5. 选择标志机（默认关闭）

- 标志表：`Rgba8Uint` 的 `texture_2d<u32>`，**一个 texel 一个 pick key**（超过
  `max_texture_dimension_2d` 时按行折叠，shader 用 `(key % width, key / width)` 寻址）；
  以 `textureLoad` 读取（无 sampler），**WebGL2 安全**；
- 绑定在 **group 1**（所有几何管线一致；image 的纹理因此移到 group 2）；
- 控制 uniform（16B）：`enabled` + 行宽；
- 着色器 `line` / `mesh` / `line_instanced` / `mesh_merged` / `image` 读标志并施加 tint：
  `FLAG_SELECTED` / `FLAG_HOVERED` 换色，`FLAG_HIDDEN` `discard`；
- **`enabled == 0` 是默认值**，此时输出与加标志前**逐像素相同**——既有高亮 overlay 批次照旧
  工作，不会双重高亮；
- 更新只写标志表：`set_pick_key_flags(key, bits)` 只上传包含该 key 的**一行**（加 16B 控制
  uniform），不碰静态 instance/merged stream、batch buffer 或 per-batch uniform
  （fence：`a_flag_update_uploads_only_the_flags_table`）；
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

## 7. 实测证据

命令（真实 GPU，默认 Vulkan ICD，未设 `VK_ICD_FILENAMES`）：

```bash
cargo test -p cad-render-wgpu --locked -- --test-threads=1
YACR_PICK_PARITY_GRID=12x9 cargo test -p cad-cli-tools --test pick_parity --locked -- --ignored --nocapture
```

- 适配器（本次运行实测）：`Quadro P620 (vulkan, discrete_gpu)`，`id_format=r32uint`。
- `crates/cad-render-wgpu`：106 lib + 19 headless + 16 pick_id + 44 render_effects +
  7 wgsl_validation 全绿；`pick_id` 覆盖 lines / glyph mesh（merged）/ 带边 overlay mesh
  （merged 与 legacy 两条路径）/ 图像以外的全部可见类、背景、多源、碰撞、格式回退、
  标志开关与「只上传标志表」。

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
  拾取。id target 格式的回退路径已实现并测试，但**未在真实 WebGL2 浏览器上运行**。
- **Android 真机 / 模拟器**：未验证。
- **高亮退役与宿主接线**：P3a-2。
- **视觉 tint**：占位常量，最终视觉归 UI 切片。
- **逐帧 id pass**：不做（按需）。
- `docs/picking-3d.md`、`docs/handoff.md` 的交叉引用与交接条目未在本轮更新（P3a-1 作用域
  限定为渲染器源码 + 新测试文件 + 本文档）。
