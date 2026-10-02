# 布局与纸空间（F04）

本文描述**表示层与桥接层**已经做到的模型/纸空间切换、纸空间视口几何（四角矩形、
视图变换、比例），以及明确的未支持边界。规范条款：§3.3「纸空间支持基本布局、视口
矩形裁剪、视图变换和比例，复杂视口裁剪等必须按样本标记能力」，F04「不支持的视口
状态显式报告」。

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

桥接层导出：

```rust
// cad-ui-slint
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
合并成折线（离开再进入会拆成多段）。线/填充几何的裁剪是精确的。

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
| `viewport.translation_mismatch` | 三点格式同时带变换平移，视图中心有歧义 |

构建布局时，不支持视口不产生任何视口几何，写入诊断码
`representation.viewport_unsupported`（消息含稳定原因码）。由于纸面自身实体仍可
绘制，结果 completeness 降为 **`Partial`**（不是 `Missing`）；布局本身不存在时才
是 `Missing`（`layout.missing`）。**「读到了视口结构」不等于「能正确显示」**。

### 3.1 与当前 importer 的关键差距（必须显式记录）

`cad-import-acadrust::read_layouts` 现在把 `clip` 写成两个**下角**（共享同一 y），
没有存储纸面高度；`model_to_paper` 也只是标量 `view_height / height`（模型/纸面），
没有 view center 平移、没有 view_direction / view_target / twist 的编码，比例方向也是
B22 指出的反的。因此真实导入的视口目前会被显式拒绝（相邻角），而不是猜一个高度。

要闭环真实文件的纸空间显示，importer 必须：

1. 把矩形写成**四角**（或至少两个真正的对角角点）；
2. 让第三/四角编码视图中心（或把视图中心放入变换平移）；
3. 保持比例方向与实现约定一致，并把 direction/target/twist 显式编码；本层已能识别
   旋转/倾斜并显式拒绝。

上述属导入工作流，不在本轮 `cad-representation` / `cad-app` / `cad-measure` 范围。
一旦导入侧给出四角矩形与正确变换，`viewport_transform` 立即支持，且已有测试覆盖。
模型空间显示不受影响（始终可用）。

## 4. 明确未完成（不是已支持）

- **视口内网格 / 文字的逐三角裁剪**：桥接层对线几何精确裁剪；`Mesh`/`Text`/`Image`
  只做变换，并把 completeness 降为 `Partial`（`viewport clip applied to line geometry
  only`），不声称已裁剪。
- **打印输出**：无 plot/打印路径。布局绘制是屏幕显示，不是可交付的图纸输出。
- **旋转/非均匀视口、注释性缩放、动态块**：显式 `Unsupported`，按样本标记，不假装支持。
- **布局切换 UI 控件接线**：`LayoutPanelState`（`cad-ui-slint`）已把布局列表与支持/原因
  投影成面板状态；`.slint` 面板控件与宿主命令接线属 UI 工作流。`cad-app` 提供并测试了
  校验与切换命令（`SwitchSpace` 校验真实布局表、失败保留原空间、不修改图纸、不产生
  `ChangeSet`）。

## 5. 测试

`cargo test -p cad-representation` 覆盖：

- 比例方向：`paper_per_model_from_view`（1:100 → 0.01，存储标量是其倒数）；
- 四角裁剪：四角、中心/半宽高、边界、视图中心落于纸面中心、1:100 模型/纸面距离、
  四角经逆变换精确往返；
- 三点旧格式：仍能推导矩形与锚点；带平移时显式拒绝；
- 不支持拒绝（含稳定原因码）：复杂裁剪、旋转纸面矩形、扭转变换、镜像、非均匀、
  倾斜视线、缺比例、相邻角、退化矩形、非有限、点数错误；
- 矩形裁剪：跨越截断、完全在外丢弃、完全在内保留、连续合并、离开再进入拆分；
- 布局枚举：真实布局表的 id/名称/支持/原因（原因以稳定码开头），空库为空；
- 纸空间构建：模型几何映射并裁剪、不支持视口显式 `Partial` 且不绘制、纸面实体直接
  绘制、缺失布局为 `Missing`、可见性谓词生效。

`cargo test -p cad-app` 覆盖布局校验与切换（未知 → `InvalidInput`、已知但不可绘 →
`Unsupported`、失败保留原空间、成功不修改图纸且无 `ChangeSet`），以及
`viewport_measurement_space` 提供已验证逆变换 / 不可支持时禁用模型测量。

`cad-ui-slint` 的桥接测试与 `LayoutPanelState` 测试需在可构建 Slint 的宿主运行；
本机缺 fontconfig，故以 wasm32 `--lib` 检查替代编译验证（见诚实边界说明）。
