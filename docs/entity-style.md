# 实体样式管线：颜色、线宽与线型（§3.2 / §7.1）

规范 §3.2（显示表示）与 §7.1（图层/实体样式）；审计 §2.1.3。本文记录 `cad-db` →
`cad-import-acadrust` → `cad-representation` → `cad-scene` → `cad-render-wgpu` 的
**颜色（color）**、**线宽（lineweight）**、**线型（linetype / 虚线比例）** 与
**透明度（transparency / alpha）的 `ByLayer` 解析** 数据路径，
区分已实现、显式未实现和只能在真实设备上验证的部分。alpha 进入场景/渲染器后的混合与
绘制顺序见 `docs/render-order.md`，本文只记录它的**来源图片段解析**（与颜色/线宽/线型同一规则）。

## 数据来源（acadrust 0.6.5 真实 API）

只用 acadrust 已暴露的类型（`~/.cargo/registry/.../acadrust-0.6.5/src`）：

| 来源 | 类型 | 关键取值 |
|---|---|---|
| `EntityCommon.color`（`entities/mod.rs`） | `types::Color` | `ByLayer` / `ByBlock` / `None` / `Index(u8)` / `Rgb{r,g,b}` |
| `EntityCommon.line_weight` | `types::LineWeight` | `ByLayer` / `ByBlock` / `Default` / `Value(i16)`（1/100 mm） |
| `EntityCommon.linetype` | `String` | 空串或 `"ByLayer"` = 随层；`"ByBlock"` = 随块；否则为表名 |
| `EntityCommon.linetype_handle` | `Option<Handle>` | R13/R14 无表名时的句柄（本实现以名字优先） |
| `EntityCommon.linetype_scale` | `f64` | 实体线型比例（缺省 1.0） |
| `tables::Layer.color` | `types::Color` | 图层颜色 |
| `tables::Layer.line_weight` | `types::LineWeight` | 图层线宽 |
| `tables::Layer.line_type` | `String` | 图层线型名，用于实体 `ByLayer` |
| `tables::LineType.elements` | `Vec<LineTypeElement>` | 元素长度：正=划、负=空、0=点 |
| `tables::LineType::is_complex()` | `bool` | 是否含 shape/text 复杂元素 |
| `LineType::pattern_length` | `f64` | 周期长度（本实现自行按 `|element|` 求和，未使用） |
| `CadDocument.header.linetype_scale` | `f64` | `$LTSCALE` 全局比例 |
| `Color::rgb()` | `Option<(u8,u8,u8)>` | `Rgb` 直取；`Index` 经 acadrust 自带 ACI 表解析 |
| `LineWeight::millimeters()` | `Option<f64>` | 仅 `Value(v)` → `v/100.0`；符号值返回 `None` |

**不猜测**：`Index` 颜色经 `Color::rgb()`（`aci_table::aci_to_rgb`）解析；线宽经
`LineWeight::millimeters()` 换算；线型的元素长度直接取
`LineTypeElement::length`，不自造虚线表。

## cad-db：旁存属性（`DbEntity` 不变）

`EntityRenderAttributes` 旁存的显示字段（`DbEntity` 因此不变），随实体旁存，保持
“加字段不波及每个实体构造器”的既有设计：

- `transparency: EntityTransparency`：`Explicit(f32)`（不透明度 `0..=1`）| `ByBlock`（符号）|
  `ByLayer`（缺省；表示层按图层行 `opacity` 解析）。
- `color: EntityColor`：`Explicit([u8;3])`（已解析 sRGB）| `ByBlock`（符号，待 INSERT 解析）|
  `ByLayer`（无法解析时的缺省；`Default` 取 `ByLayer`）。
- `lineweight: EntityLineWeight`：`Explicit(f32)`（毫米）| `Default`（acadrust 的
  `LineWeight::Default`）| `ByBlock`（符号）| `ByLayer`（缺省）。
- `linetype: EntityLineType`：`Explicit { name, pattern: LinetypePattern, scale }`（已解析
  虚线）| `ByBlock`（符号）| `ByLayer`（缺省，表示层按连续线画并标记未解析）。

`LinetypePattern { elements: Vec<f64>, cycle: f64 }` 采用 DWG/DXF 元素约定：正=划、负=空、
0=点；`cycle` 为 `Σ|element|`。`from_elements` 过滤非有限元素，若过滤后为空或周期 ≤ 0 则
退化为连续线。数据库另存：

- `linetypes: BTreeMap<LinetypeId, LineType>`：命名线型表（名字、pattern、`complex` 标志）；
- `linetype_scale: f64`：全局 `$LTSCALE`，缺省 1.0，由 `set_linetype_scale` 校验为正有限。

缺省条目仍表示“不透明、连续线、默认样式”，手工构造的数据库不受影响。

## cad-db：图层属性（`Layer` 不变）

图层属性与实体属性同构：**旁存**在 `DrawingDatabase::layer_attributes`
（`BTreeMap<LayerId, LayerAttributes>`），`Layer { id, name, visible }` 保持三字段不变
（全仓 87 个文件、183 处 `Layer` 字面量因此无需改动）。缺省条目由
`layer_attributes(id)` 回退到 `LayerAttributes::default()`，手工构造的数据库不需要迁移；
借用变体 `layer_attributes_ref(id)` 诚实返回 `None` 而不是伪造一行。

```rust
pub struct LayerColor { pub aci: Option<u8>, pub rgb: [u8; 3] }   // 缺省 Some(7) / 白
pub enum LayerLineWeight { Hundredths(u16), Default, ByLayer }     // 1/100 mm，缺省 ByLayer
pub struct LayerAttributes {
    pub off, frozen, frozen_in_new_viewport, locked, xref_dependent: bool,
    pub plottable: bool,          // 唯一缺省为 true 的布尔
    pub color: LayerColor, pub linetype: String,
    pub lineweight: LayerLineWeight, pub opacity: f32,   // 1.0 = 不透明
    pub plot_style: Option<String>,
}
```

各字段的 DXF 对应：`off` = 组 `62` 的**符号**（组 `70` 没有 off 位）、`frozen` = `70` bit 1、
`frozen_in_new_viewport` = `70` bit 2、`locked` = `70` bit 4、`xref_dependent` = `70` bit `0x10`、
`plottable` = `290`、`color.aci` = `62`、`color.rgb` = `420`（真彩）、`linetype` = `6`、
`lineweight` = `370`、`opacity` 的补数 = `AcCmTransparency` XDATA（源里 `0` 不透明、`255` 全透明）。

**`aci` 与 `rgb` 同时携带**：`cad-db` 不得依赖 `cad-app`（架构门禁），拿不到它的 ACI 表，
所以索引身份与显示值分开存、互不推导；真彩图层 `aci = None`，索引图层两者都有。
`lineweight` 存源的 1/100 mm 整数（不是毫米浮点），回写才能是读到的同一个整数。
`plot_style` 存的是**名字**，而 DXF/DWG 写入器只写 plot-style **句柄**（组 `390`），
所以这个名字**无法在导出中存活**（导出写缺省句柄并记一条诊断）。

**不变量**：`Layer::visible == !(off || frozen)`（导入行与 `create_layer` 行）。`visible` 仍是
全仓唯一被消费的“可见”答案；`off` 与 `frozen` 分开存，是因为图纸区分二者（冻结不是关闭），
而本阶段两者都只是“不可见”。

**行为现状（2026-10-26 更新）**：`locked` 已在**实体写路径**单点强制
（`crates/cad-db/src/drawing/lock.rs`），`off`/`frozen` 经派生的 `Layer::visible` 生效；
`frozen_in_new_viewport`/`plottable`/`plot_style` 仍是**只存储 + 往返**，冻结的独立语义、
出图与命名打印样式**显式未实现**。

**`ByLayer` 颜色/线型/线宽/透明度现在全部按图层解析（(e) 已完成）**：导入不再把 `ByLayer` 拍平成显式 sRGB
（`cad-import-acadrust` 保留符号 `EntityColor::ByLayer`），表示层
`provider/expand.rs` 在展开时用实体自身 `layer` 的 `LayerAttributes::color.rgb` 解析，
缺行则用 `layer_attributes()` 的既有 `Default`（ACI 7 白）。因此**改图层颜色会在下一次展开时
重着色其 `ByLayer` 实体，且不写任何实体、不产生实体历史**。块内实体的 `ByLayer` 按**它自己
所在图层**解析（只有 `ByBlock` 才沿父链取包含块引用的已解析颜色）；AutoCAD「图层 0 上的实体
继承块引用图层」这一特例**未建模**（声明）。**线型**按图层行的 `linetype` **名字**去图纸线型表
（大小写不敏感）取 pattern，名字不在表内则退回连续线并置 `linetype_unresolved`（绝不编造虚线）；
**线宽**按 `LayerLineWeight` 的 1/100 mm 精确换算，`Default` → 0.25 mm，图层行自身为 `ByLayer`
（无数值可给）→ 0.25 mm 且置 `lineweight_unresolved`。**透明度**按图层行的 `opacity`
（`0..=1`，缺行 = `1.0` 不透明）解析成 `DisplayFragment::alpha`，**与显式 alpha 走同一条消毒路径**
（表示层不钳制、不消毒，`cad_scene::sanitize_alpha` 是唯一消毒点：非有限 → 不透明，否则钳 `[0,1]`）。
四者都**不写实体**。

## cad-import-acadrust：解析（ByObject → ByLayer → ByBlock）

图层表现在只读入 `layer_linetypes`（**仅诊断用途**：图层线型名不在图纸线型表时按实体报一次）与
线型表；`layer_colors`、`layer_lineweights` 与 `layer_transparency` 三张旁表**已全部删除**——
颜色/线宽/透明度改为保留符号 `ByLayer` 由表示层查图层行，线型名字也由表示层查图纸线型表，
线型表读入
`linetype_patterns`（名字小写 → pattern），随后每个实体在 `push_entity` 里解析：

- **ByObject 优先**：`Color::Rgb`/`Color::Index` → `EntityColor::Explicit`；
  `LineWeight::Value` → `EntityLineWeight::Explicit(mm)`；命名线型 → 查表得到
  `EntityLineType::Explicit`（其 `scale` 取实体 `linetype_scale`）。
- **ByLayer**（颜色）：**保持符号 `EntityColor::ByLayer`**，由表示层在展开时查图层行解析
  （2026-10-26 起；此前在此处拍平成显式 sRGB）。
- **ByLayer**（线宽/线型）：**保持符号** `EntityLineWeight::ByLayer` / `EntityLineType::ByLayer`，
  由表示层在展开时查图层行/线型表解析（2026-10-26 起；此前在此处拍平）。
- **ByLayer**（透明度）：**保持符号 `EntityTransparency::ByLayer`**（并是 `Default`），
  由表示层在展开时取该图层的 `opacity` 解析（2026-10-26 起；此前在此处把图层 opacity
  拍平成 `Explicit`，图层透明度改动因此无效）。
- **ByBlock**：保持符号 `*::ByBlock`，由表示层 INSERT 展开时用包含块引用的有效值替代
  （与 alpha 同一机制）。
- `Color::None`：视为 `ByLayer`（继承图层），保留符号不画成黑色；解析由表示层按图层完成。
- 图层颜色若是退化符号值，回退白色（AutoCAD 名义默认 ACI 7）；图层线宽若是符号值，则
  该图层不提供线宽，实体留给 `ByLayer` 缺省；图层线型名若在表中不存在，同样视为
  `ByLayer` 缺省（连续线）。
- **未知命名线型**：查表失败时记 `import.linetype_unknown`，实体得到
  `Explicit { pattern: 连续, name: 原名 }`——显式连续回退，绝不编造虚线。
- **复杂线型**（含 shape/text 元素）：仍按元素长度生成划/空段落，但字形不绘制，记
  `import.linetype_complex`（`Partial`）。
- 全局 `$LTSCALE` 经 `set_linetype_scale` 存入数据库；非有限/非正时回退 1.0。

## cad-representation：分片携带 + INSERT 解析 + 虚线细分

`DisplayFragment` 新增：

- `alpha: f32`（有效不透明度 `0..=1`：`Explicit` 原样、`ByLayer` 取图层 `opacity`、`ByBlock` 继承；
  消毒/钳制在 `cad-scene::sanitize_alpha`，表示层不做第二份）；
- `color: [f32; 3]`（归一化 sRGB）与 `color_unresolved: bool`；
- `lineweight: f32`（毫米）与 `lineweight_unresolved: bool`；
- `linetype: LinetypePattern`、`linetype_unresolved: bool`、`linetype_scale: f32`。

`build`/`from_tessellation` 没有图层表，输出文档化缺省
（`DEFAULT_RENDER_COLOR = [1,1,1]`、`DEFAULT_LINEWEIGHT_MM = 0.25`、连续 pattern、`alpha = 1.0`）并置
`*_unresolved = true`，**不冒充**源值。`build_expanded` 用
`resolve_alpha`/`resolve_color`/`resolve_lineweight`/`resolve_linetype` 写入已解析值：`Explicit` 直接采用；
`ByBlock` 继承外层 INSERT 的已解析值；模型根无外层时退回缺省（`ByBlock` alpha 为不透明；
颜色/线宽/线型退回缺省且标记未解析）。`ByLayer` 在这里解析：颜色取实体自身图层的
`LayerAttributes::color.rgb`、线宽取 `lineweight`、线型取图层**名字**在图纸线型表中的 pattern、
透明度取 `opacity`，缺行一律走 `layer_attributes()` 的既有 `LayerAttributes::default()`，
**不另造本地缺省**。

**虚线细分发生在表示层**（`subdivide_dashes`，底层为 `cad-geometry::dash_polyline`），
不在渲染器里：

- 连续线：`subdivide_dashes` 原样返回单条折线，**与今天完全一致**；
- 非连续线：每条 `DisplayPrimitive::Lines` 按**弧长**切成多个划段子折线（直线与
  离散化曲线同规则），一个划段一个 `DisplayFragment` → 场景里一个 `RenderBatch`，因此
  虚线实体在 GPU 上真实地呈现为间断的划段；
- 比例：`combined = entity_linetype_scale × global LTSCALE`。二者都不合法时回退 1.0；
  有效 `< 0`/0/NaN 由 `resolve_linetype`/`dash_polyline` 拒绝并走回退路径。
- **退化/零周期/全空/非法比例**：`dash_polyline` 永不 panic，返回
  `DashOutcome::Continuous(reason)`；`subdivide_dashes` 原样返回连续线并给出原因，由
  `build_expanded` 追加 `Completeness::Partial` 与 `representation.linetype_fallback`
  诊断，**绝不静默丢线或伪造虚线**。
- 前导空段被跳过，使线从第一个划段开始（AutoCAD `A` 对齐）；短于首个划分的线仍绘制
  被裁剪的划段。

点（长度 0 的元素）在仅画线的渲染器里没有长度，折叠为空；点线型仍可终止，不会死循环。

## cad-scene：`RenderBatch` 与消毒

`RenderBatch` 新增 `color: [f32;3]`、`color_unresolved`、`lineweight: f32`、
`lineweight_unresolved`，以及 `linetype: LinetypePattern`、`linetype_unresolved`。
`SceneCache::build` 用 `sanitize_color`/`sanitize_lineweight` 消毒颜色/线宽，并把分片的
`linetype`/`linetype_unresolved` 原样带到批次（**诊断用途**：渲染器不消费该字段，因为
划段已在表示层切好）。颜色非有限通道 → `1.0`（缺省），否则钳制 `[0,1]`；线宽非有限 →
缺省，负数 → `0`。**绝不把 NaN 送进 GPU。**

批注叠加层（`annotations.rs`）此前只带 alpha、RGB 被记为 `annotation.color` 缺口；现在
`AnnotationStyle::rgba` 的 RGB 直接成为 `batch.color`（`color_unresolved = false`），该缺口
诊断已移除。`logical_width` 是世界坐标宽度、非毫米线宽，故**不**映射到
`RenderBatch::lineweight`（批注线宽标记未解析）。批注与高亮叠加层都没有源线型，一律
标记 `linetype_unresolved`（连续线）。

## cad-render-wgpu：逐批颜色进入 shader；线宽只携带；虚线已在表示层切好

- 每批 uniform 为 `transform`(mat4x4) + `tint: vec4`；`tint.rgb` = 批颜色（
  `renderer_color` 再次消毒），`tint.a` = `clamp_alpha(alpha)`。`line.wgsl` 输出
  `vec4(tint.rgb, tint.a)`；`mesh.wgsl` 输出 `tint.rgb * (0.25 + 0.75*diffuse)`。
  因此两个颜色不同的实体在同一帧得到不同像素（lavapipe 已实跑验证）。
- **线宽未绘制**：浏览器 WebGL2/WebGPU 的线宽恒为 1px，原生 wgpu 的宽线也不可移植。
  本轮**不**实现宽线：`FrameStats.lineweight_not_drawn` 列出每个请求非零线宽的已提交批次，
  `FrameStats.lineweight_reason` 给出稳定诊断码 `render.lineweight_not_drawn`
  （`cad-diagnostics::codes`）。绝不声称绘制了未实现的宽度。
- **线型虚线**：渲染器**不**新增虚线逻辑，也不消费 `RenderBatch::linetype`。虚线由表示层
  切成的多条子折线实现，GPU 端仍只提交普通 `LineList`——因此虚线在软件 Vulkan
  （lavapipe）上可直接观测到间断，且不依赖驱动的线宽/线型扩展。

### 诊断码

| 码 | 含义 | 载体 |
|---|---|---|
| `render.lineweight_not_drawn` | 批次携带非零线宽但未绘制 | `FrameStats.lineweight_reason`（`Partial`） |
| `import.linetype_unknown` | 实体引用的命名线型不在表中，按连续线画 | `ImportReport.diagnostics`（`Partial`） |
| `import.linetype_complex` | 复杂线型只画划/空段，字形不画 | `ImportReport.diagnostics`（`Partial`） |
| `representation.linetype_fallback` | 虚线无法按弧长生成，回退连续线 | `DisplayRepresentation`（`Partial`） |

### 精确 vs 显式 Partial 边界

**精确（Complete）**：

- 实体显式命名线型在表中存在 → 按 `element` 与比例精确切分；
- 实体/图层 `ByLayer` 且图层线型在表中存在 → 精确；
- `ByBlock` 经 INSERT 展开继承到具体 pattern → 精确；
- 标准 `Continuous` → 单条折线（与线型功能前完全一致）；
- 曲线/圆弧按弧长切分，划段长度与 pattern 一致（仅末段按线尾裁剪）。

**显式 Partial（有原因，不静默）**：

- `import.linetype_unknown`：命名线型查无此表 → 连续线回退；
- `import.linetype_complex`：复杂线型 → 只画划/空段（字形省略）；
- `representation.linetype_fallback`：`DashIssue`（空 pattern / 零周期 / 全空 / 非法比例 /
  退化折线）→ 连续线回退。`DashIssue::reason()` 给出稳定字符串：
  - `linetype pattern has no elements`
  - `linetype pattern has a degenerate cycle length`
  - `linetype pattern contains only gaps`
  - `linetype scale is not a positive finite number`
- `ByLayer`/`ByBlock` 在模型根且无可解析 pattern → 连续线 + `linetype_unresolved = true`
  （不是错误，是符号未解析）。

**仍未实现（显式）**：

- 复杂线型内嵌的 shape/text 字形（只画划/空段）；
- 线型端部/拐角自适应的 `A` 对齐微调（本实现每段折线独立从头生成）；
- 3D 折线按真实三维弧长切分（本实现按三维点距的弧长，方向正确）；
- 点（长度 0）本身的可视标示（仅能终止，不单独渲染）。

## 证据边界

- **纯逻辑已运行并通过**（宿主无 GPU）：
  - `cad-db`：默认即 `ByLayer`/不透明/连续；`Explicit`/`ByBlock` 往返与符号保真；
    `LinetypePattern` 构造过滤非有限元素、零周期退化连续。
  - `cad-import-acadrust`：ByObject→ByLayer→ByBlock 颜色/线宽/线型解析、ACI 解析、
    `lineweight_mm` 只报具体值、未知命名线型连续回退、复杂线型标志、非有限比例回退。
  - `cad-geometry`：`dash_polyline` 的划/空/点、比例、前导空跳过、弧长曲线、折线拐点、
    零周期/全空/非法比例/退化线的永不 panic 回退。
  - `cad-representation`：`build_expanded` 携带已解析颜色/线宽/线型、ByBlock 经 INSERT
    继承、模型根 ByBlock 标记未解析、无导入属性时标记未解析；显式虚线把一条线切成多个
    子折线（含曲线按弧长）、连续线保持单条、`linetype_scale` 与全局 `LTSCALE` 生效。
  - `cad-scene`：颜色/线宽/线型进入 `RenderBatch`、未解析缺省、非有限值替换、NaN 不入 GPU。
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
  - 虚线渲染为普通 `LineList`（表示层已切分），无需新的 GPU 路径；`cad-render-wgpu`
  测试套件已实跑通过。
- **仍未运行验证 / 显式未实现**：
  - 宽线光栅化（任意线宽几何）——未实现，仅报告；
  - 复杂线型内嵌 shape/text 字形——未实现，只画划/空段并记 `import.linetype_complex`；
  - 线型端部/拐角 `A` 对齐微调——未实现，每段折线独立从头生成；
  - 点线型的点本身——无长度可画，仅保证终止；
  - plot style 的颜色/线宽——DXF/DWG 文档内仍只有样式名字符串，无逐 ACI 解析值（0.6.5 新增的是
    独立 `.ctb`/`.stb` **文件**编解码 `plot_style::PlotStyleTable`，本项目未消费），不猜测；
  - 颜色空间：颜色按 sRGB 归一传递，不做线性化/伽马转换，与既有渲染路径一致。
