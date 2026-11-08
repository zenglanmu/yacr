# 布局与纸空间（F04）

本文描述**表示层、桥接层与共享 UI/传输层**已经做到的模型/纸空间切换、纸空间视口几何（四角矩形、
视图变换、比例）、布局页签与 `SPACE` 命令、活动空间取景，以及明确的未支持边界。规范条款：
§3.3「纸空间支持基本布局、视口矩形裁剪、视图变换和比例，复杂视口裁剪等必须按样本标记能力」，
F04「不支持的视口状态显式报告」。

## 1. 空间选择

`cad-representation::layout` 定义：

```rust
pub enum SpaceSelection {
    Model,
    Paper(LayoutId),
}
```

- `Model`：主路径，与既有 `bridge::build_scene` 完全一致，行为不变。
- `Paper(id)`：选择数据库中的某个布局。

布局枚举由真实布局表派生，不发明布局：

```rust
pub fn enumerate_layouts(db: &DrawingDatabase) -> Vec<LayoutDescriptor>;
```

`LayoutDescriptor { id, name, supported, reason, viewport_count }`。`supported == false`
表示该布局至少有一个视口本构建无法正确绘制；`reason` 形如 `"<stable-code>: <message>"`，
前缀是机器可读的稳定原因码（见 §3）。模型空间不出现在列表里——它始终可用，用
`SpaceSelection::Model` 表示。

导入器按图纸页签顺序（acadrust `Layout.tab_order`）分配 `LayoutId(index + 1)`，并以
LAYOUT 对象的 `name` 作为显示名，绝不使用内部块名 `*Paper_Space0`。`DrawingDatabase`
的布局表按键有序，因此 `enumerate_layouts` 的顺序就是用户在页签上看到的顺序。只在
图纸完全没有纸空间布局时才写入合成的 `LayoutId(0)` `"Model"` 记录（既有消费方过滤
该 id）。没有对应 LAYOUT 对象的 `*PAPER_SPACE*` 块仍按稳定块名顺序追加，并以块名作
显示名。

桥接层导出：

```rust
// 旧 UI crate（已随 2026-10-16 退役删除；下列导出面仅为历史记录）
pub fn layout_descriptors(db: &DrawingDatabase) -> Vec<LayoutDescriptor>;
pub fn build_scene_with_space(
    db: &DrawingDatabase,
    stamp: TaskStamp,
    fonts: Option<Arc<FontEngine>>,
    overrides: &LayerOverrideSet,
    space: SpaceSelection,
) -> CadResult<SceneDelta>;
```

`build_scene` / `build_scene_with_fonts` / `build_scene_with_overrides` 保持原签名，
内部等价于 `SpaceSelection::Model`，因此既有模型空间入口不受影响。

### 1.1 共享 UI 与传输（2026-11-02 已接线）

- **`CadStatus.layouts`**：`Vec<CadLayoutTab>`，由
  `cad_app::render_scene::layout_descriptors` 的真实描述符投影，过滤合成的 `LayoutId(0)`
  （模型空间是独立页签，不是纸空间布局）。每项携带 `id`（`LayoutId` 的 `u128`）、`name`
  （LAYOUT 显示名）、`supported`、`viewport_count`，以及**仅在不支持时**的稳定 `reason`。
- **`CadStatus.activeSpace`**：线上形状 `{kind:"model"}` 或 `{kind:"paper", id:<u128>}`，
  来自会话真实的 `SpaceId`；`SpaceId::Block` 不作为可观察空间（`cad-app` 的
  `validate_space_id` 拒绝），投影时防御性归到 Model，绝不新造变体。
- **`SPACE` 文本命令**：页签点击复用既有 `cad_command` 漏斗，只发一条普通命令行文本：
  `SPACE MODEL`（大小写不敏感）、`SPACE #<u128>` 或 `SPACE <布局显示名>`。名称解析顺序是
  **精确匹配优先，其次唯一的 ASCII 大小写不敏感匹配**；未知/歧义分别按名拒绝
  （`unknown layout: …` / `ambiguous layout: …`），不猜一个 layout id；整行剩余部分都是名称，
  因此 `SPACE Sheet One` 可用。成功时命令日志只回显准入的选择子名，失败不派发。
  这与 IPC 的 `CommandPayload::Space` 是**同一个**切换命令，没有第二条写路径。
- **`LayoutTabs`**（桌面绘图区左下）与紧凑布局的移动抽屉共用
  `src/layout/layout-tabs.ts` 的 `layoutTabsView`：同一份 `cad_status.layouts` +
  `activeSpace`、同一组 `SPACE` 命令字符串。当前活动页签没有命令（点击不派发）；**不支持的
  布局仍可见但禁用**，标题只显示目录里的 `unsupported` 标记 + 宿主稳定原因码，不发明句子。点击只是
  **队列准入**，活动态仍来自下一帧 `activeSpace`，UI 不抢先声明切换成功。
- **`ReferencesPanel`** 同样只读 `cad_status.references`（桌面 docked、紧凑布局 floating，
  同一组件、同一宿主行）；它不解析路径、不重试、不提供 Reload/Locate。XREF 语义见
  `docs/xref.md`。
- **活动空间 FIT**：切换成功后的自动取景与 `FIT` 使用**同一个**活动空间规则。模型空间取
  `drawing.bounds()`；纸空间取纸面实体与视口纸面矩形的并集，为空时退回
  `plot_settings_for` 的纸尺寸矩形；连纸尺寸也不可用时相机保持不动并显式报告
  `layout.fit_empty`。自动适配不记 `ZOOM PREVIOUS` 步骤、不写库、不产生 `cad-history`
  条目或 `ChangeSet`。
- **多布局导入**：导入器按 LAYOUT 对象的 `tab_order` 排序，以 LAYOUT 的 `name` 作显示名，
  并分配 `LayoutId(index + 1)`；没有对应 LAYOUT 对象的 `*Paper_Space*` 块仍按稳定块名顺序
  追加并保留块名。只有图纸完全没有纸空间布局时才写入合成 `LayoutId(0)` `"Model"` 记录。
  页签顺序与显示名都来自图纸表，UI 不重排、不改名。

## 2. 视口几何：四角矩形 + 视图变换

`cad-db::PaperViewport` 只携带 `clip: Vec<Point3>`、`model_to_paper: Transform3` 与
`completeness`。本层从中重建**真实的四角纸面矩形**与**带视图中心的视图变换**，
并把变换作用到模型几何上。`viewport_transform(&PaperViewport) -> ViewportState`。

### 2.1 裁剪编码

两种编码都被接受，最终都归约为同一个四角矩形：

1. **四角裁剪（首选）**：`clip = [c0, c1, c2, c3]` 是视口的四个纸面角点。矩形必须是
   轴对齐的；旋转/扭斜的纸面矩形被显式拒绝，而不是被强行“摆正”。
   视图中心由几何恢复：`model_anchor = stored(paper_center)`（见 §2.2）。
2. **三点旧格式（兼容）**：`clip = [a, b, view_center]`，`a`、`b` 是纸面矩形的两个
   **对角**，第三点是模型视图中心。当前 importer 仍按此形状写入（但写成了相邻角，
   因此被拒绝）。若该格式的 `model_to_paper` 同时带有平移，视图中心来源有歧义，会
   被拒绝（`viewport.translation_mismatch`）——视图中心属于 `clip[2]`。

无论哪种编码，`ViewportTransform` 都暴露：

```rust
pub struct ViewportTransform {
    pub paper_corners: [Point3; 4], // 逆时针，从最小角开始
    pub paper_center: [f64; 2],
    pub paper_half: [f64; 2],
    pub model_anchor: Point3,       // 落在纸面中心的模型点（视图中心）
    pub paper_per_model: f64,
    pub to_paper: Transform3,       // 实际应用的 模型 → 纸面 变换
    pub to_model: Transform3,       // 精确逆：纸面 → 模型
}
```

### 2.2 比例方向（审计 B22）

存储的 `model_to_paper` 按本仓库实际写入的方向解释：**纸面 → 模型**、线性比例是
*模型单位 / 纸面单位*（`view_height / paper_height`，即 B22 指出的方向）。实际应用
的**模型 → 纸面**映射是它的逆，因此：

```text
paper_per_model = paper_height / view_height   // 1:100 => 0.01
paper = (model - view_center) * paper_per_model + paper_center
```

`paper_per_model_from_view(view_height, paper_height)` 是这一方向的单一来源与测试点。
存储标量（如 100）是它的倒数——审计 B22「方向需纠正」正是指这里。

### 2.3 视图中心 / 方向 / 扭转

- **视图中心**：三点旧格式取 `clip[2]`；四角格式由存储变换恢复为
  `stored(paper_center)`。
- **视图方向**：本构建只支持垂直于纸面的视图。变换中若出现 z 与纸面 x/y 的耦合
  （`m[0][2]`、`m[1][2]`、`m[2][0]`、`m[2][1]` 非零），即倾斜的三维视图，
  报 `viewport.off_plane_view`。
- **扭转（twist）**：变换的 xy 线性部分若含旋转（`m[0][1]` / `m[1][0]` 非零），
  报 `viewport.twisted_transform`，绝不按轴对齐矩形解释。
- **镜像 / 非均匀 / 缺比例 / 奇异**：分别报 `viewport.mirror`、
  `viewport.non_uniform`、`viewport.scale`。

### 2.4 矩形裁剪

`clip_polyline_to_rect(paper, center, half) -> Vec<Vec<Point3>>` 用 Liang–Barsky
逐段裁剪：完全在外丢弃、完全在内原样保留、跨越边界的按交点截断，并把连续的幸存段
合并成折线（离开再进入会拆成多段）。**线/填充几何**的裁剪是精确的。

`clip_polygon_to_rect` 用 Sutherland–Hodgman 把凸多边形裁剪到同一轴对齐纸面矩形
（视口窗口在本构建中只接受轴对齐矩形，见 §2.1，因此窗口是凸的）。

- **网格**：`clip_mesh_to_rect(mesh, center, half)` 逐三角裁剪，把幸存多边形扇形
  三角化，并在新顶点上**线性插值 z、法线与逐顶点 sRGB 颜色**（渐变填充 HATCH 的
  颜色带因此正确，不会被拉伸）；完全在内的三角形原样复用，完全在外的消失。
  完全在窗口外的网格返回 `None`（精确省略）。
- **圆锥曲线**：`clip_conic_to_rect(conic, center, half)` 对
  `center + major*cos t + minor*sin t`（圆/圆弧/椭圆）按参数区间精确求交：把
  x、y 两个坐标约束各自解成 `[0, 2π)` 上的区间，与圆弧自身的参数域求交，输出若干
  同方向、非 `full` 的 `ConicSegment`。圆仍然是圆（不离散），完全在内时原样返回，
  完全在外时精确省略。
- **图像**：`clip_image_quad_to_rect(transform, center, half)` 裁剪单位正方形经
  `transform` 映射后的四边形，并在新顶点上**线性插值纹理坐标 UV**，返回
  `ImageVertex { position, uv }`；`DisplayPrimitive::Image` 增加可选 `clip` 多边形
  承载它，未裁剪时为 `None`。

**无法精确裁剪**的情形绝不假装：保留未裁剪几何并降为 `Partial`，附稳定
`clip_reason` 码与诊断（见 §4）。

## 3. 支持与不支持

`ViewportState::Supported(Box<ViewportTransform>)`：四角（或可归约的三点）轴对齐
矩形、均匀正比例、视图垂直于纸面、数据有限、矩形非退化。

`ViewportState::Unsupported(ViewportUnsupported { code, message })`：明确拒绝，
**绝不按猜测比例绘制**。`code` 是稳定的机器可读原因码（`viewport_reason`）：

| 原因码 | 触发条件 |
|---|---|
| `viewport.complex_clip` | `completeness == Partial`（复杂/非矩形裁剪） |
| `viewport.clip_arity` | 裁剪点数既非 3 也非 4 |
| `viewport.non_finite` | 裁剪点或变换系数非有限 |
| `viewport.rotated_clip` | 四角不构成轴对齐矩形 |
| `viewport.degenerate_rect` | 矩形宽或高 ≤ 1e-12 |
| `viewport.adjacent_corners` | 三点格式写入相邻（非对角）角 |
| `viewport.scale` | 比例缺失、零或负 |
| `viewport.twisted_transform` | 变换含扭转/旋转 |
| `viewport.non_uniform` | 变换错切或非均匀 |
| `viewport.mirror` | 变换镜像 |
| `viewport.off_plane_view` | 视线不垂直于纸面 |
| `viewport.perspective` | 透视投影 |
| `viewport.translation_mismatch` | 三点格式同时带变换平移，视图中心有歧义 |

构建布局时，不支持视口不产生任何视口几何，写入诊断码
`representation.viewport_unsupported`（消息含稳定原因码）。由于纸面自身实体仍可
绘制，结果 completeness 降为 **`Partial`**（不是 `Missing`）；布局本身不存在时才
是 `Missing`（`layout.missing`）。**「读到了视口结构」不等于「能正确显示」**。

### 3.1 与当前 importer 的闭环（审计 B22）

`cad-import-acadrust::read_layouts` 现在为每个纸空间布局的**内容视口**写入：

1. **四角纸面矩形**：`clip = [c0, c1, c2, c3]`。没有 VPCLIP 边界时是
   `center ± (width/2, height/2)` 的四个轴对齐角点（acadrust 的 `Viewport.center` /
   `width` / `height`）。`clip_boundary_handle` 非空时解析其指向的实体：**只有**闭合、
   无 bulge、世界 Z 且去重后正好是四个轴对齐矩形角的 `LWPOLYLINE` 才被接受，并使用该边界
   矩形作为 `clip`（可能小于/偏移于视口矩形，这是 VPCLIP 的真实语义）。曲线、开放折线、
   旋转/多角、倾斜 OCS、其他实体类型或悬空句柄仍按 §3 置 `Partial`，绝不强行摆正。
2. **真实纸面→模型变换**：`model = (paper - paper_center) * model_per_paper + anchor`，
   其中 `model_per_paper = view_height / height`（acadrust 的 `view_height`、`height`、
   `center`），`anchor` 是纸面视口中心显示的模型点。它取自 **DXF 组码 12/22 的二维
   视图中心**（`Viewport.view_center`）；组码 17/27/37 的 `view_target` 只在
   `view_center` 恰为 `(0,0)` 且 target 非零时作为兼容回退。QCAD 等写入方把真实中心
   放在 12/22 而把 17/27/37 留为原点，若按 `view_target` 锚定，所有视口都会退化成
   「模型原点在视口中心」。本层由存储变换恢复视图中心（`stored(paper_center)`）。
3. 因此 `viewport_transform` 直接返回 `Supported`，四角、比例方向与视图中心都正确
   （见 §2.2 与 `paper_per_model_from_view`）。审计 B22 的两处方向/形状问题均已闭环。

`id == 1` 的**图纸视口**（`Viewport.id`）与 `status.is_on == false` 的视口不写入：前者是
纸面本身而非模型窗口，后者不显示（audit B22，与 OpenCADStudio 的 `is_sheet_viewport`
一致，仅作行为参考）。

导入侧对**当前无法精确绘制**的视口不猜：仍写入四角矩形与变换，但 `completeness` 置
`Partial` 并给出具体原因，本层按 §3 的稳定原因码拒绝（`viewport_transform` 会把导入原因
归类到对应码）：

| 导入原因（`Completeness::Partial` 消息） | 映射原因码 |
|---|---|
| 非矩形裁剪（`clip_boundary_handle` 非空，且边界实体无法证明为轴对齐矩形） | `viewport.complex_clip` |
| 视线不垂直于纸面（`view_direction` 非世界 Z） | `viewport.off_plane_view` |
| 视图扭转（`twist_angle != 0`） | `viewport.twisted_transform` |
| 透视投影（`status.perspective`） | `viewport.perspective` |
| `view_height` 缺失/零/非有限 | `viewport.scale` |
| 几何非有限 | `viewport.non_finite` |

矩形 VPCLIP 不是「复杂裁剪」：它的边界实体被解析成同一个四角轴对齐矩形后，
`completeness` 保持 `Complete`，布局页签可用。真实图纸里大量内容视口都带一个与视口
矩形重合的边界折线（例如 `fixtures/debug-default-fonts.dwg` 的 `开洞钢筋图`，15 个
视口全部如此）；旧实现只看句柄非空就判 `Partial`，会把这类整页错误地禁用。无法证明
为矩形的边界仍逐条保持 `Partial` 并显示稳定原因码。

**跨视口复用与诊断去重（2026-11-03）**：`build_paper_space` 是**按需**入口，只在宿主
切换到该布局时调用；没有 import-time 的全布局预计算，也没有为其他布局预先建缓存。
同一次调用内，每个可见模型实体只 `build_expanded` 一次，所有受支持视口复用同一份
表示并各自应用视图变换与矩形裁剪（几何仍是逐视口的，因为每个窗口的裁剪结果不同）。
诊断去重：模型/纸面实体诊断按 `(code, message, object)` 保留不同对象的不同问题；
同一裁剪限制按 `(reason, message)` 每个布局只报一次（消息保留首个视口索引）；相同
`viewport_unsupported` 原因也只报一次。`completeness` 仍按真实缺口置 `Partial`，
去重只影响重复叙述，不影响缺席的几何或支持状态。

### 3.2 INSERT / 块语义（审计 B31）

块展开在本层（`ProviderRegistry::build_expanded`）完成，导入侧负责把每个 `INSERT` 的
完整放置矩阵写入 `SemanticGeometry::Insert`：

```text
world = OCS(normal) · translate(insert_point) · R(rotation) · S(x_scale, y_scale, z_scale) · (p - base_point)
```

- **base point**：acadrust `BlockRecord.base_point` 被减去，块基点落在 `insert_point` 上。
- **OCS/normal**：acadrust `Insert.normal` 经 AutoCAD 任意轴算法提升到 WCS。
- **rotation**：正值逆时针（与 DXF/acadrust `Matrix4::rotation_z` 一致）。
- **数组 / MINSERT**：`column_count` × `row_count` 个单元各生成一个 `Instance`，包成
  `Compound`；单元位移在**缩放之前**施加，因此间距不被 `x_scale`/`y_scale` 缩放
  （与 OpenCADStudio `insert_instance_transform` 的行为一致）。块定义本身不被直接发出，
  不会重复绘制（审计 B31）。
- 引用了未知块名的 `INSERT` 记为 `Missing`，不是空成功。

### 3.3 2D OCS 与 SOLID/3DFACE 顶点序（审计 B31）

- **2D LWPOLYLINE / POLYLINE**：顶点是 OCS 坐标，结合 `normal` 与 `elevation` 提升到
  WCS（`polyline_ocs_points`）。倾斜挤出对直线段精确（长度守恒）；只有“倾斜 + 两顶点
  + bulge”因弧平面不唯一而记 `Partial`。
- **SOLID / TRACE**：acadrust 按 `first, second, third, fourth` 存储，但可见四边形边界是
  `first, second, fourth, third`；导入按边界顺序三角化（`solid_mesh_semantics`），并把
  角点从实体的 OCS（`Solid.normal`）提升到 WCS。有厚度（`thickness`）时只画平面并按
  `Partial` 记录。
- **3DFACE**：角点已是 WCS、且按 `first, second, third, fourth` 边界序存储，直接三角化。


## 4. 明确未完成（不是已支持）

- **视口裁剪的剩余边界**：线/填充几何按段精确裁剪；`Mesh` 逐三角精确裁剪（位置、
  z、法线、逐顶点颜色插值）；`Conic`（圆/圆弧/椭圆）按参数区间精确裁剪，圆保持解析
  不解散；`Image` 四边形精确裁剪（纹理 UV 插值），且当图像**同时**
  带实体级裁剪与视口裁剪时，两者**精确求交**（Sutherland–Hodgman，UV 插值；见
  `docs/dxf-entity-coverage.md`）；带字体名、被 `FontEngine` 成功 shape 的 Text 其轮廓
  就是 `Lines`，走线裁剪（精确）。仍 **无法精确裁剪** 的情形保留未裁剪几何并降为
  `Partial`，附稳定 `clip_reason` 码与 `representation.viewport_clip_partial` 诊断
  （绝不静默丢弃、绝不声称已裁剪）：
  - `viewport.clip_conic_unclippable`：圆锥曲线参数非有限（退化/溢出的防御性输入）；
  - `viewport.clip_text_font_dependent`：未 shape 的 `Text` 占位（无字体时 glyph
    几何依赖字体，本层无从裁剪）；
  - `viewport.clip_mesh_unclippable`：网格索引越界、顶点非有限或超出 32 位顶点寻址；
  - `viewport.clip_image_degenerate`：图像变换把四边形映到非有限坐标；
  - `viewport.clip_image_intersection_unsupported`：实体级裁剪为**非凸**多边形（provider
    契约外的防御性输入），保留实体裁剪并显式报 `Partial`，绝不替换/加宽；
  - `viewport.clip_unsupported_primitive`：未展开的 `Instance` 等没有可裁剪几何的
    原语。
  实体级**非凸**图像裁剪在 provider 侧即拒绝（frame+`Partial`+`image.clip_unsupported`），
  不会到达此处。非矩形/非凸视口裁剪在上游 `viewport_transform` 就已显式 `Unsupported`
  （§3），不会到达逐三角裁剪。
- **打印输出**：无 plot/打印路径。布局绘制是屏幕显示，不是可交付的图纸输出。
- **倾斜/扭转/透视视口、非矩形裁剪、非均匀视口、注释性缩放、动态块**：导入时置
  `Partial` 并给出原因，本层以稳定原因码显式 `Unsupported`，按样本标记，不假装支持。
- **布局切换 UI/传输已接线（2026-11-02）**：`LayoutPanelState`（旧 UI crate）已随桌面 UI 迁移
  删除；当前桌面用 `apps/desktop-ui/src/components/LayoutTabs.tsx`，移动抽屉复用同一
  `layout/layout-tabs.ts` 投影，数据来自 `cad_status.layouts` / `activeSpace`，点击发
  `SPACE MODEL|#<id>`（见 §1.1）。`cad-app` 仍提供并测试校验与切换命令（`SwitchSpace` 校验
  真实布局表、失败保留原空间、不修改图纸、不产生 `ChangeSet`）。切换成功后只做视图适配：
  `FIT`/切换都按**活动空间**取景——模型空间取 `drawing.bounds()`；纸空间取纸面实体与视口
  纸面矩形的并集，为空时退回 `plot_settings_for` 的纸尺寸矩形。若连纸尺寸也不可用，相机保持
  不动并显式报告 `layout.fit_empty`（不是伪造的空成功）。自动适配不记 `ZOOM PREVIOUS` 步骤、
  不写库、不产生 `cad-history` 条目或 `ChangeSet`。**仍未支持**的是 §3 列出的视口能力
  （非矩形/复杂裁剪、扭转/倾斜/透视、非均匀、镜像、缺比例等）：布局页签仍可见但禁用并显示
  稳定原因码，不绘制猜出来的视口。

## 5. 测试

`cargo test -p cad-representation` 覆盖：

- 比例方向：`paper_per_model_from_view`（1:100 → 0.01，存储标量是其倒数）；
- 四角裁剪：四角、中心/半宽高、边界、视图中心落于纸面中心、1:100 模型/纸面距离、
  四角经逆变换精确往返；
- 导入闭环：importer 形状的 1:100 四角视口 → `Supported` 且比例/视图中心正确；导入的
  扭转/倾斜/透视/复杂裁剪视口 → `Partial` 并映射到对应稳定原因码（`viewport.twisted_`
  `transform` / `off_plane_view` / `perspective` / `complex_clip`）；
- 三点旧格式：仍能推导矩形与锚点；带平移时显式拒绝；
- 不支持拒绝（含稳定原因码）：复杂裁剪、旋转纸面矩形、扭转变换、镜像、非均匀、
  倾斜视线、透视、缺比例、相邻角、退化矩形、非有限、点数错误；
- 矩形裁剪：跨越截断、完全在外丢弃、完全在内保留、连续合并、离开再进入拆分；
- 逐三角 / 逐四边形裁剪：多边形在内外、三角形完全在内原样复用、完全在外丢弃、
  部分裁剪的顶点数与面积（扇形三角化 `n-2`）、逐顶点颜色在裁剪顶点插值、索引越界
  与非有限顶点显式拒绝（`viewport.clip_mesh_unclippable`）、图像四边形 UV 插值、
  图像变换保留 UV、以及纸空间集成（模型网格逐三角裁剪后仍为 `Complete`；未 shape
  的 Text 保留为 `Partial` 且带 `viewport.clip_text_font_dependent`；图像经视口
  裁剪后 UV 正确）；
- 布局枚举：真实布局表的 id/名称/支持/原因（原因以稳定码开头），按键序（即导入器
  分配的页签顺序）返回并携带显示名，空库为空；
- 纸空间构建：模型几何映射并裁剪、不支持视口显式 `Partial` 且不绘制、纸面实体直接
  绘制、缺失布局为 `Missing`、可见性谓词生效。

`cargo test -p cad-app` 覆盖布局校验与切换（未知 → `InvalidInput`、已知但不可绘 →
`Unsupported`、失败保留原空间、成功不修改图纸且无 `ChangeSet`），切换后的活动空间取景
（纸空间取纸面实体 + 视口矩形，切回模型取模型 bounds，不可取景的空布局报
`layout.fit_empty` 且相机不动），以及 `viewport_measurement_space` 提供已验证逆变换 /
不可支持时禁用模型测量。

`cargo test -p cad-import-acadrust` 覆盖：1:100 视口四角与纸面→模型变换、图纸/关闭视口
不写入、扭转/倾斜/复杂裁剪视口为 `Partial`；布局按 LAYOUT `tab_order` 排序、显示名来自
LAYOUT 对象、纸空间实体与视口仍关联到正确布局 id、无 LAYOUT 对象的纸块按块名追加；
INSERT 基点相减、正值逆时针旋转、OCS normal 提升、数组单元为 pre-scale 行主序、未知块为
`Missing`；SOLID 边界顶点序（面积守恒）与非 Z 挤出提升；倾斜 OCS 折线长度守恒。

`cargo test -p cad-ui-core` 覆盖布局页签与活动空间投影、`SPACE` 参数解析：`cad_status.layouts`
与 `layout_descriptors` 一致（过滤模型空间合成 id）、`activeSpace` 来自真实会话、`SPACE MODEL`/
`#<id>`/精确或唯一大小写不敏感显示名都能切换，未知/歧义名称按名拒绝且不派发、失败保留原空间；
`CadStatus` 无文档时为空的 layouts/references 且不省略字段。`apps/desktop-ui` 的
`layout-tabs.test.ts` 覆盖页签渲染顺序、活动态、点击只发一条 `SPACE`、不支持页签可见但禁用
且只带稳定原因码、缺少/畸形 status 块被拒绝而不篡改。

旧 UI crate 的桥接测试与 `LayoutPanelState` 测试**已随桌面 UI 迁移删除**（该 runner 在仓库中
不存在）；对应能力现在由 `LayoutTabs`/`layout-tabs.ts` 与上述 `cad-ui-core` 测试承接。本节有效
验证是 `cargo test -p cad-representation`、`-p cad-app`、`-p cad-import-acadrust`、
`-p cad-ui-core` 与 `apps/desktop-ui` 的 `npm test`。
