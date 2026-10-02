# 渲染顺序与透明策略（F14）

规范 §3.1 F14 / §5.2 / §8；审计 F14 与 F12。本文记录 `cad-render-wgpu` 的**绘制
顺序**与**透明合成**策略：哪些是纯逻辑、可在宿主验证的行为，哪些只有真实 GPU 才能
确认。与 `docs/render-3d.md`（三维网格路径）互补，不重复其中相机/法向/预算的描述。

## 数据来源

顺序与透明度只消费 `cad-scene::RenderBatch` 已有的两个字段，不新增第二套渲染路径：

| 字段 | 含义 | 当前生产者 |
|---|---|---|
| `draw_order: i64` | 与同级批次相对的绘制顺序，值越大越晚画（越在上层） | `SceneCache::build` 目前恒为 `0`；选择高亮层 `HighlightOptions::draw_order`（默认 `HIGHLIGHT_DRAW_ORDER = 900_000`）逐批递增；批注叠加层 `annotation_batches` 从 `AnnotationSceneOptions::draw_order_base`（默认 1_000_000）开始逐批递增；工具预览层 `cad_app::render_scene::overlay::PREVIEW_DRAW_ORDER = 2_000_000` |
| `alpha: f32` | 逐对象常量透明度 | `SceneCache::build` 取 `DisplayFragment::alpha`（`cad-scene::sanitize_alpha` 钳制到 `[0,1]`）；importer 在 `EntityRenderAttributes` 里记录有效透明度，`build_expanded` 解析 `ByBlock` 后写入 fragment；批注叠加层取 `AnnotationStyle::rgba[3] / 255` |

**透明度数据路径（已实现）**：acadrust 0.5.5 暴露 `EntityCommon.transparency: Transparency`
（`types/transparency.rs`：`ByLayer` / `ByBlock` / `Explicit(u8)`，0 不透明、255 全透明）
与 `tables::Layer.transparency`。importer 在 `cad-import-acadrust` 里解析：

- `ByObject`（`Explicit`）优先于 `ByLayer`：`Explicit(a)` → `alpha = 1 - a/255`；
- `ByLayer` → 取该图层已解析的透明度（`Layer.transparency`，同样 0..255 映射）；
- `ByBlock` → 保持符号值，由表示层的 INSERT 展开继承包含块引用的有效 alpha。

解析结果存入 `cad-db::EntityRenderAttributes`（随实体旁存，`DbEntity` 不变），
`ProviderRegistry::build_expanded` 沿 INSERT 链把 `ByBlock` 解析为外层 alpha 并写入
`DisplayFragment::alpha`。**限制**：`ProviderRegistry::build`（非展开入口，测试/无数据库
调用）只看到实体，没有数据库访问，因此保守地输出 `alpha = 1.0`；真实 UI 走
`build_expanded`，透明度已端到端接通。**另一显式未支持项**：绘图样式表（plot style）
的 alpha 未应用——acadrust 0.5.5 只暴露逐实体与逐图层的 `Transparency`，`plot_style`
是样式名字符串、没有解析后的 alpha，因此不猜测、不叠加。

渲染器在上传时把 `RenderBatch::centroid()`（世界坐标，`local_origin + 顶点均值`）与
`draw_order`、`alpha` 缓存到 `GpuBatch`，供每帧排序使用；相机变化只重算排序，不重传
顶点缓冲。

## 绘制顺序策略（纯逻辑，已测）

`cad-render-wgpu::geometry::plan_draw_order` 把一批 `BatchOrderEntry` 分成三段，返回
下标计划 `DrawOrderPlan { opaque, transparent, invisible }`：

1. **不透明优先**：`classify_alpha == Opaque` 的批次先画，按 `draw_order` **升序**。
2. **透明随后**：`0 < alpha < 1` 的批次在全部不透明批次之后画，**由远及近**（back-to-front）。
3. **完全透明跳过**：`alpha <= 0` 不提交，归入 `invisible` 并计入统计，绝不静默当作
   正常绘制。

**决定性 tie-break（文档化并测试）**：

- 不透明批次：`draw_order` 升序；`draw_order` 相同时保持**上传顺序**（排序稳定，
  输入下标即最终决胜项）。
- 透明批次：主键为到相机的**平方距离降序**（最远先画）；距离相同时次键为 `draw_order`
  升序；再相同则按上传下标升序。距离用平方值，单调等价且免开方；非有限距离退化为相等
  后仍由后两个键保证确定性。

排序键 `camera` 为 `Some(eye)` 时按深度排序；`None` 时透明批次退回到仅按 `draw_order`
升序（确定性但**非深度正确**）。当前 `Renderer::render`（2D）传相机中心、`render_3d`
传 `Camera3d::eye`，因此两条渲染入口总是有排序位置。

## 透明合成（GPU 侧行为）

- 渲染通道拆成两个逻辑 pass：**不透明 pass** 先画（深度写入开启），随后**透明 pass**
  画不透明之后的批次。
- 三个网格管线变体由同一 `mesh.wgsl` 创建：
  - `cad-mesh-pipeline` / `cad-mesh-pipeline-mirrored`：不透明，`depth_write_enabled = true`，
    镜像批次用 `front_face = Cw`；
  - `cad-mesh-pipeline-transparent` / `cad-mesh-pipeline-mirrored-transparent`：
    透明，`depth_write_enabled = false`，深度比较仍为 `Less`，混合为
    `wgpu::BlendState::ALPHA_BLENDING`（管线自创建起即配置混合）。
- 关闭透明 pass 的深度写入，使由远及近的每层按 alpha 合成到已解析的不透明结果上，
  而不是用自身深度遮挡后面的层。
- 线条/线框（`Lines`、`MeshEdges` 及网格叠加线）使用 `cad-lines-pipeline`，其深度写入
  保持开启；**透明线条的深度写入策略尚未区分**，属于显式未实现。

### alpha 边界规则（已测）

`geometry::clamp_alpha` / `classify_alpha` 的确定性规则：

| 输入 | 结果 | 理由 |
|---|---|---|
| `alpha >= 1` | 不透明 | 上界钳制 |
| `0 < alpha < 1` | 透明 | 参与 back-to-front 合成 |
| `alpha <= 0` | 不可见（跳过） | 完全透明，计入 `invisible_batches` |
| `NaN` | 不透明（`1.0`） | 不可读的透明度不得让几何消失，取保守值 |
| `+inf` | 不透明（`1.0`） | 钳制上界 |
| `-inf` | 不可见（`0.0`） | 钳制下界 |

`FrameStats` 新增 `opaque_batches` / `transparent_batches` / `invisible_batches`，调用方
可据此把"被跳过的透明批次"上报为显式状态，而不是静默丢弃。

## 与帧预算的关系

顶点/三角预算（`plan_from_gpu`）仍按**上传顺序**累计，语义与行为不变（见
`docs/render-3d.md`）。排序只作用于预算已接受的批次：先按预算取子集，再对子集做
不透明/透明分区与排序。因此"批注永远在上层"保证的是**绘制顺序**，不改变被预算截断
的批次集合——若需要让高 `draw_order` 批次优先获得预算额度，是另一个待设计策略，当前
不做。

## 证据边界

- **纯逻辑已运行并通过**（宿主无 GPU）：
  - 不透明升序 + 稳定 tie-break；
  - 透明/不透明/不可见三分区；
  - 由远及近的平方距离排序键；
  - tie-break 顺序（距离→`draw_order`→上传下标）；
  - 相机移动改变透明顺序；
  - alpha 钳制与 NaN 规则（`geometry::clamp_alpha`）；`cad-scene::sanitize_alpha` 同规则；
  - `DisplayFragment::alpha` 进入 `RenderBatch::alpha`（含 `<1`、`0`、越界钳制）；
  - `build_expanded` 的 `ByObject`/`ByLayer`/`ByBlock` 分辨率与 `geometry_source`/`precision`；
  - `RenderBatch::centroid()` 的均值/大坐标精度/空批次回退。
  见 `crates/cad-render-wgpu/src/geometry.rs` 与 `crates/cad-scene/src/lib.rs`、
  `crates/cad-representation/src/lib.rs` 的单元测试。
- **静态校验**：`mesh.wgsl` 未改动，仍由
  `crates/cad-render-wgpu/tests/wgsl_validation.rs` 用与 `wgpu 30.0.1` 同版本的 naga
  解析校验。
- **GPU 合成已在软件 Vulkan（lavapipe）实跑并通过**：
  `headless_render.rs::translucent_batch_composites_differently_from_opaque`
  （`VK_ICD_FILENAMES=.../lvp_icd.json cargo test -p cad-render-wgpu`）渲染同一三角形，
  分别 `alpha = 1.0`（不透明 pass）与 `alpha = 0.5`（透明 pass），断言帧缓冲中心像素
  不同，且 `FrameStats` 分别报告 `opaque_batches = 1` / `transparent_batches = 1`。这证明
  `RenderBatch::alpha` 确实到达混合状态、透明管线可用、alpha 改变了合成结果。
- **仍未运行验证**：多层透明批次的由远及近真实合成顺序、`depth_write_enabled = false`
  在遮挡关系下的视觉效果、镜像+透明组合的剔除、透明线条的深度写入、以及 2D 相机中心作为
  伪深度键的视觉结果。不得据此宣称渲染视觉完全正确。
