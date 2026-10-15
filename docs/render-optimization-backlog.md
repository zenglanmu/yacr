# 渲染/性能 待优化项（Optimization Backlog）

本文是渲染与性能的**待优化项清单**（backlog），不是已完成能力表、也不是发布承诺。每项给出
现状、预期收益、前置条件/风险、需要的产品/UX 决策、相关契约/栅栏与优先级。已完成项见
`docs/adr/0007-entity-as-data-renderer.md`、`docs/validation-render-perf-2026-10-15.md`。

**当前基线（真实 NVIDIA Quadro P620 / Vulkan，`debug-default-fonts.dwg` / `complex-test.dwg`，
见 `docs/validation-render-perf-2026-10-15.md`）**：

- draw 数：10,474 → **797**；122,006 → **7,627**（§4）。
- 每帧 cpu（pan_mid）：17.96 → **1.88 ms**；46.74 → **5.91 ms**（§4）。
- 静态几何在 pan/zoom 全程零流/chunk 重传；逐帧 uniform 由 O(批数) 降为 O(pages)（ADR-0007）。

下述所有收益估算都以该基线为参照；未实测的收益一律标注为**预期**，不得当作已验证。

优先级记号：**P1–P6**（本轮/近期）、**G1–G7**（既有 deferred/已知项结转）。

---

## 性能类（P1–P3、P5–P6）

### P1 — 块级视锥剔除（chunk-level frustum culling）

- **现状/问题**：`build_instance_chunks`（`crates/cad-render-wgpu/src/plan.rs:1570`）与
  `build_mesh_chunks`（`plan.rs:1004`）在构建 chunk 时**已经算出世界坐标 AABB**
  （`batch_world_aabb` `plan.rs:1430`、`piece_world_aabb` `plan.rs:899`、
  `ChunkBuilder.min/max` `plan.rs:1472-1473`、`aabb_half_diagonal` `plan.rs:1450`），
  但上传后的 `InstanceChunk`（`plan.rs:1458`）/`MeshChunkData`（`plan.rs:813`）**未保留**该
  AABB。当前每帧对全部 chunk 提交 draw，**不判断可见性**——放大到图纸局部时，绝大多数 chunk
  在视口外仍被绘制。
- **预期收益**：按 chunk AABB 在提交前整块跳过。放大到局部时收益最大（当前"画全部"）；全览时
  收益趋近 0。**通用档（含 WebGL2）可用、纯 CPU 跳过、成本极低**。预期收益量级取决于 chunk
  粒度（受 `MAX_STREAM_RADIUS = 8192 mm` 分块约束），需实测，不预填数字。
- **前置条件/风险**：需把构建期 AABB 下沉到运行时 chunk 结构并逐帧做视锥/AABB 测试（正交相机
  下 AABB 与视锥相交即可；透视路径须独立 fence，见 ADR-0007「per-chunk f64 origin 限定于
  Ortho」）。**风险：像素无损**（纯跳过，不改变保留几何的绘制）；需 fence 证明
  **culled == 未绘制**，且 chunk 边界**无弹出**（用略放大的 AABB 或正确包含全部实例的包围盒，
  不得用会漏掉边缘实例的近似盒）。
- **需要的决策**：无产品决策；纯实现优化。
- **相关契约/栅栏**：`merged-vs-legacy` 逐字节一致 fence 的扩展——剔除后保留集合的像素须与
  未剔除一致；新增 `culled_chunks_are_not_drawn` / `frustum_culling_does_not_pop_at_chunk_borders`。
- **优先级**：P1（最高；通用档、低成本、放大场景收益大）。

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

### P3 — 帧缓冲纹理复用 / 局部重绘（framebuffer texture reuse）

- **现状/问题**：每次 pan/zoom 都整帧重画。可复用上一帧：**pan** = blit 上一帧 + 用 scissor
  重绘新暴露的 L 形条带；**zoom 进行中** = 缩放 blit（**瞬时模糊**），settle 后异步整帧矢量重绘。
- **预期收益**：仅在**百万实体量级且实测整帧重绘不足**时才有价值（当前已降到每帧 cpu 1.88 /
  5.91 ms，**未测量出必要**）。**条件性收益，未实测**。
- **前置条件/风险**：与**选择标志着色失效**交互——被标志（SELECTED/HOVERED，颜色替换）的
  区域必须重绘，不能复用旧像素（`docs/picking-gpu-id-pass.md` §0）；与高 DPI 清晰度交互
  （缩放 blit 的模糊在高 DPI 下更明显）；与 progressive 帧缓存交互（标志变化会丢弃缓存帧，
  §0「标志变化必须让已完成的 progressive 帧失效」）。
- **需要的决策（UX）**：缩放过程中的**模糊容忍度**（是否接受瞬时模糊、模糊持续到 settle 的时长）。
- **相关契约/栅栏**：`blit_reuses_previous_frame_and_repaints_the_exposed_strip`、
  `flagged_regions_are_repainted_not_blitted`、高 DPI 下的清晰度 fence。
- **优先级**：P3（最低优先级；先测量，确认整帧重绘不足再做）。

### P5 — RenderBundle 预录制（optional）

- **现状/问题**：静态/动态拆分已就位（ADR-0007「静态 / 动态拆分」），但**残留 legacy 逐批
  draw**（透明批次、图片、overlay，见 `docs/render-order.md:69-72`）仍逐帧经过 wgpu 校验/编码。
  可把残留 legacy draw 预录进 **RenderBundle** 以绕过逐帧校验。
- **预期收益**：draw 数已从 10,474/122,006 塌缩到 797/7,627（§4），残留 legacy 占比小 →
  **收益低**。
- **前置条件/风险**：仅对**静态** legacy 内容有效；透明批次按相机排序、overlay 随选择变化，
  两者都**动态**，不能整体预录。需先把可预录的静态子集与动态子集再拆分。
- **需要的决策**：无。
- **相关契约/栅栏**：bundle 重录时机（场景/键空间变化）与像素一致 fence。
- **优先级**：P5（低；draw 塌缩后优先级下降，可延后）。

### P6 — GPU compute 剔除 + indirect buffer（加速层 bonus）

- **现状/问题**：视锥/像素剔除若在 CPU 做（P1/P2）可覆盖通用档；更强方案是用 **compute shader**
  做剔除并输出 **indirect draw buffer**，一次 `multi_draw_indexed_indirect` 提交。
- **预期收益**：把每帧剔除成本从 CPU 移到 GPU、减少提交次数。**仅加速档可用，未实测**。
- **前置条件/风险**：**`multi_draw_indexed_indirect` 为 native-only，不在 WebGPU core**——
  因此**绝不能成为架构依赖**，只能作为 **capability-gated bonus**；WebGL2 档**永远**走
  通用路径（P1/P2 的 CPU 剔除）。无 indirect 支持时须回退到通用路径，且回退结果一致。
- **需要的决策**：无（架构约束已定）。
- **相关契约/栅栏**：capability 探测决定档位；indirect 档与通用档的**像素一致** fence；档位在
  兼容表中显式建模（不得把加速档当作默认能力）。
- **优先级**：P6（加速档 bonus，随加速层一起评估）。

---

## 视觉/语义类（P2、P4）

### P4 — SDF 圆/弧（SDF circles/arcs）

- **状态（2026-10-15）：已实现（slice B）**，见 `docs/adr/0008-sdf-conic-rendering.md`。
  可 SDF 化的圆/弧（不透明、连续线型、slice-1 圆形）渲染为抗锯齿 SDF 实例；椭圆（slice 2）、
  虚线/非连续、非不透明 conic 仍弦展开。真实 P620/Vulkan 的覆盖-解析曲线、弦基线包含、id
  pass 一致性、flags、绘制顺序、确定性/零重传与 before/after 视觉验收均已 fence（见 ADR-0008
  实施记录）。**未**做椭圆 SDF、`PulledArc`、透视域精度、浏览器/Android 验收。
- **预期收益**：把弧渲染为 **SDF 四边形**（实例化：center/radius/起始角/终止角/颜色/线宽），
  片元着色器用**有符号距离**做抗锯齿 → **分辨率无关、无需三角化、每类约 1 个 draw**。替代大量
  弧折线实例，减少 line 实例数。**预期**，未实测。
- **前置条件/风险**：需要新的 SDF 管线 + 实例布局 + 片元 SDF；与现有 line/mesh 合并路径的绘制
  顺序（`docs/render-order.md`）需对齐。**语义像素变化**：SDF 抗锯齿**比当前折线弦更平滑** →
  需**视觉验收**；不得宣称与旧折线逐字节一致。
- **需要的决策（视觉）**：抗锯齿边缘的外观、线宽语义、与折线路径并存的过渡策略。
- **相关契约/栅栏**：SDF 弧的几何精度 fence（中心/半径/角度映射）、视觉验收证据、绘制顺序与
  既有管线一致。
- **优先级**：P4（收益可观但需视觉验收；排在 P1/P2 之后）。

---

## 功能补全类（G1–G7）

### G1 — glyph outline-keyed instancing（文字轮廓实例化）

- **现状/问题**：文字轮廓仍是主要残留 draw（参考图 instanced 帧中 glyph mesh 占
  **4,630 / 6,228**）；未按 outline 键做实例化（ADR-0007:94-95）。
- **预期收益**：相同 glyph 轮廓跨实例复用 → 大幅削减文字顶点/draw。
- **前置条件/风险**：需按 outline 键分桶的实例化路径；与 P2（文字 LOD）互补但独立。
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

### G5 — hover/hidden 生产者未接线

- **现状/问题**：`OverlayInputs::hovered` / `hidden` 机制与测试已就绪，但宿主尚未产生二者
  （`cad-host` 传空），默认空（`docs/picking-gpu-id-pass.md` §0、§8；ADR-0007:97-102）。
- **预期收益**：无直接性能收益；**功能补全**——悬停高亮与隐藏实体在生产会话可用。
- **前置条件/风险**：**hidden 生产者接线必须先完成 G2**（否则隐藏实体仍可拾取，违反铁律 #2）。
- **需要的决策**：无。
- **相关契约/栅栏**：hover/hidden 进入生产会话的端到端 fence；G2 的 hidden 拾取 fence 同步。
- **优先级**：G5（后续宿主切片；受 G2 门控）。

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

### G7 — 真实浏览器 WebGL2 验证 + DWG pick-parity 复跑

- **现状/问题**：① 标志块「WebGL2 安全」由离线 **GLSL ES 3.00 back-translation** 测试覆盖
  （`color_shaders_back_translate_to_glsl_es_300`），**非**真实浏览器运行；id target 回退路径
  已实现并测试但未在真实 WebGL2 浏览器上运行（`docs/picking-gpu-id-pass.md` §8）。② parity
  harness（渲染器 id pass vs CPU `cad_spatial`）本轮未重跑；「不可解释冲突 = 0」是关键指标
  （§7，`complex-test.dwg` 精确一致率偏低已定位为等深并列差异，harness 不断言该率）。
- **预期收益**：无性能收益；**验证补全**——把"离线可译"升级为"真实浏览器运行"，并在未来变更后
  复跑 pick-parity。
- **前置条件/风险**：需要真实 WebGL2 浏览器（headful + 真实 GPU，非 SwiftShader）；DWG
  pick-parity 需真实 DWG（`YACR_TEST_DWG`）与真实 GPU。
- **需要的决策**：无。
- **相关契约/栅栏**：真实浏览器 WebGL2 flags 路径运行证据；DWG pick-parity 复跑记录（变更后
  重跑，不用旧数字冒充）。
- **优先级**：G7（验证缺口，显式 NOT RUN）。

---

## 加速层（P6 + 本轮进行中项）

### A — capability-gated accelerated tier（vertex pulling via storage buffers）

- **现状/问题**：更强合并档（vertex pulling / storage buffer 取顶点；配合 P6 的 indirect）**尚未
  落地**。ADR-0007 记为 deferred（`docs/adr/0007-entity-as-data-renderer.md:103-104`），并明确
  **WebGL2 不支持 multi-draw**，属 **capability-gated future**。
- **预期收益**：进一步减少每批 uniform/绑定开销、提升顶点读取效率；**仅加速档**，未实测。
- **前置条件/风险**：**capability 探测决定档位**；WebGPU/native 走加速档，**WebGL2 自动回退**
  通用路径。加速档**不得**成为架构依赖，回退必须像素一致。
- **需要的决策**：无（架构约束已定）。
- **相关契约/栅栏**：档位探测 + 回退一致性 fence；加速档在兼容表中显式建模。
- **优先级**：加速层（**本轮进行中，状态见 ADR-0007**）。

---

## 交叉引用

- 渲染架构决策与 deferred 列表：`docs/adr/0007-entity-as-data-renderer.md`。
- GPU id 回读拾取 / 选择标志机 / 已知限制与未做项：`docs/picking-gpu-id-pass.md`（尤其 §0、§4、§6、§7、§8）。
- 绘制顺序与透明策略（instanced / merged / legacy 并存）：`docs/render-order.md`。
- 性能基线与证据边界：`docs/validation-render-perf-2026-10-15.md`。
- 能力表与显式未支持项：`docs/compatibility.md`。

> 证据诚实：本文所有未实测收益均标注为「预期」；未运行项（真实浏览器 WebGL2、DWG
> pick-parity 复跑、视觉 golden、Android 真机）标注为 NOT RUN / 未验证，不得外推为跨平台或
> 完整 CAD 视觉验收。
