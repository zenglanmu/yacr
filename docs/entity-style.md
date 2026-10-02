# 实体样式管线：颜色与线宽（§3.2 / §7.1）

规范 §3.2（显示表示）与 §7.1（图层/实体样式）；审计 §2.1.3。本文记录 `cad-db` →
`cad-import-acadrust` → `cad-representation` → `cad-scene` → `cad-render-wgpu` 的
**颜色（color）** 与 **线宽（lineweight）** 数据路径，区分已实现、显式未实现和只能在
真实设备上验证的部分。透明度（alpha）见 `docs/render-order.md`，本文不重复。

> LINETYPE（虚线比例）本轮**不实现**，只保留符号位；见文末“显式未实现”。

## 数据来源（acadrust 0.5.5 真实 API）

不新增字段，只用 acadrust 已暴露的类型（`~/.cargo/registry/.../acadrust-0.5.5/src`）：

| 来源 | 类型 | 关键取值 |
|---|---|---|
| `EntityCommon.color`（`entities/mod.rs`） | `types::Color` | `ByLayer` / `ByBlock` / `None` / `Index(u8)` / `Rgb{r,g,b}` |
| `EntityCommon.line_weight` | `types::LineWeight` | `ByLayer` / `ByBlock` / `Default` / `Value(i16)`（1/100 mm） |
| `tables::Layer.color` | `types::Color` | 图层颜色 |
| `tables::Layer.line_weight` | `types::LineWeight` | 图层线宽 |
| `Color::rgb()` | `Option<(u8,u8,u8)>` | `Rgb` 直取；`Index` 经 acadrust 自带 ACI 表解析 |
| `LineWeight::millimeters()` | `Option<f64>` | 仅 `Value(v)` → `v/100.0`；符号值返回 `None` |

**不猜测**：`Index` 颜色经 `Color::rgb()`（`aci_table::aci_to_rgb`）解析，不用自造常量；
线宽经 `LineWeight::millimeters()` 换算，不引入额外比例因子。

## cad-db：旁存属性（`DbEntity` 不变）

`EntityRenderAttributes` 新增两个字段，随实体旁存，保持“加字段不波及每个实体构造器”的
既有设计：

- `color: EntityColor`：`Explicit([u8;3])`（已解析 sRGB）| `ByBlock`（符号，待 INSERT 解析）|
  `ByLayer`（无法解析时的缺省；`Default` 取 `ByLayer`）。
- `lineweight: EntityLineWeight`：`Explicit(f32)`（毫米）| `Default`（acadrust 的
  `LineWeight::Default`）| `ByBlock`（符号）| `ByLayer`（缺省）。

缺省条目仍表示“不透明、默认样式”，手工构造的数据库不受影响。

## cad-import-acadrust：解析（ByObject → ByLayer → ByBlock）

图层表先读入 `layer_colors`/`layer_lineweights`；每个实体在 `push_entity` 里解析：

- **ByObject 优先**：`Color::Rgb`/`Color::Index` → `EntityColor::Explicit`；
  `LineWeight::Value` → `EntityLineWeight::Explicit(mm)`。
- **ByLayer**：取该图层已解析的颜色/线宽；图层缺失才退回符号 `ByLayer`（不伪造）。
- **ByBlock**：保持符号 `EntityColor::ByBlock`/`EntityLineWeight::ByBlock`，由表示层
  INSERT 展开时用包含块引用的有效值替代（与 alpha 同一机制）。
- `Color::None`：视为 `ByLayer`（继承图层），而不是画成黑色；无图层时标记未解析。
- 图层颜色若是退化符号值，回退白色（AutoCAD 名义默认 ACI 7）；图层线宽若是符号值，则
  该图层不提供线宽，实体留给 `ByLayer` 缺省。

## cad-representation：分片携带 + INSERT 解析

`DisplayFragment` 新增：

- `color: [f32; 3]`（归一化 sRGB）与 `color_unresolved: bool`；
- `lineweight: f32`（毫米）与 `lineweight_unresolved: bool`。

`build`/`from_tessellation` 没有图层表，输出文档化缺省
（`DEFAULT_RENDER_COLOR = [1,1,1]`、`DEFAULT_LINEWEIGHT_MM = 0.25`）并置
`*_unresolved = true`，**不冒充**源值。`build_expanded` 用
`resolve_color`/`resolve_lineweight` 写入已解析值：`Explicit` 直接采用；`ByBlock`
继承外层 INSERT 的已解析值；模型根无外层时退回缺省且标记未解析。`ByLayer` 在真实导入里
已被 importer 替换，此处保持未解析标记。

## cad-scene：`RenderBatch` 与消毒

`RenderBatch` 新增 `color: [f32;3]`、`color_unresolved`、`lineweight: f32`、
`lineweight_unresolved`。`SceneCache::build` 用 `sanitize_color`/`sanitize_lineweight`
消毒：非有限通道 → `1.0`（缺省），否则钳制 `[0,1]`；线宽非有限 → 缺省，负数 → `0`。
**绝不把 NaN 送进 GPU。**

批注叠加层（`annotations.rs`）此前只带 alpha、RGB 被记为 `annotation.color` 缺口；现在
`AnnotationStyle::rgba` 的 RGB 直接成为 `batch.color`（`color_unresolved = false`），该缺口
诊断已移除。`logical_width` 是世界坐标宽度、非毫米线宽，故**不**映射到
`RenderBatch::lineweight`（批注线宽标记未解析）。

## cad-render-wgpu：逐批颜色进入 shader；线宽只携带

- 每批 uniform 为 `transform`(mat4x4) + `tint: vec4`；`tint.rgb` = 批颜色（
  `renderer_color` 再次消毒），`tint.a` = `clamp_alpha(alpha)`。`line.wgsl` 输出
  `vec4(tint.rgb, tint.a)`；`mesh.wgsl` 输出 `tint.rgb * (0.25 + 0.75*diffuse)`。
  因此两个颜色不同的实体在同一帧得到不同像素（lavapipe 已实跑验证）。
- **线宽未绘制**：浏览器 WebGL2/WebGPU 的线宽恒为 1px，原生 wgpu 的宽线也不可移植。
  本轮**不**实现宽线：`FrameStats.lineweight_not_drawn` 列出每个请求非零线宽的已提交批次，
  `FrameStats.lineweight_reason` 给出稳定诊断码 `render.lineweight_not_drawn`
  （`cad-diagnostics::codes`）。绝不声称绘制了未实现的宽度。
- LINETYPE 虚线比例本轮不实现，符号位保留在 `EntityCommon.linetype`/`linetype_scale`，
  渲染侧不使用；后续轮次处理。

### 诊断码

| 码 | 含义 | 载体 |
|---|---|---|
| `render.lineweight_not_drawn` | 批次携带非零线宽但未绘制 | `FrameStats.lineweight_reason`（`Partial`） |

## 证据边界

- **纯逻辑已运行并通过**（宿主无 GPU）：
  - `cad-db`：默认即 `ByLayer`/不透明；`Explicit`/`ByBlock` 往返与符号保真。
  - `cad-import-acadrust`：ByObject→ByLayer→ByBlock 颜色/线宽解析、ACI 解析、
    `lineweight_mm` 只报具体值。
  - `cad-representation`：`build_expanded` 携带已解析颜色/线宽、ByBlock 经 INSERT 继承、
    模型根 ByBlock 标记未解析、无导入属性时标记未解析。
  - `cad-scene`：颜色/线宽进入 `RenderBatch`、未解析缺省、非有限值替换、NaN 不入 GPU。
  - `cad-render-wgpu`（逻辑）：`renderer_color`/`lineweight_reason` 规约。
  见各 crate 单元测试。
- **静态校验**：`line.wgsl`/`mesh.wgsl` 由
  `tests/wgsl_validation.rs` 用与 `wgpu 30.0.1` 同版本的 naga 解析。
- **GPU 已在软件 Vulkan（lavapipe）实跑并通过**
  （`VK_ICD_FILENAMES=.../lvp_icd.json cargo test -p cad-render-wgpu`）：
  - `render_effects.rs::batches_with_different_colors_render_different_frames`：同几何、
    红/蓝两批帧缓冲不同，且各自通道占优；
  - `render_effects.rs::resolved_layer_color_reaches_the_uniform`：默认色与“来自图层的
    红色”渲染不同；
  - `render_effects.rs::lineweight_is_reported_not_drawn`：非零线宽被报告、零线宽不报告。
- **仍未运行验证 / 显式未实现**：
  - 宽线光栅化（任意线宽几何）——未实现，仅报告；
  - LINETYPE 虚线（`linetype`/`linetype_scale`）——本轮不实现；
  - plot style 的颜色/线宽——acadrust 0.5.5 只暴露样式名字符串，无解析值，不猜测；
  - 颜色空间：颜色按 sRGB 归一传递，不做线性化/伽马转换，与既有渲染路径一致。
