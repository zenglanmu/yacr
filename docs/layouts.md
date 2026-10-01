# 布局与纸空间（F04）

本文只描述**表示层与桥接层**已经做到的模型/纸空间切换、纸空间视口变换与矩形裁剪，
以及明确的未支持边界。规范条款：§3.3「纸空间支持基本布局、视口矩形裁剪、视图变换和比例，
复杂视口裁剪等必须按样本标记能力」，F04「不支持的视口状态显式报告」。

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
表示该布局至少有一个视口本构建无法正确绘制；`reason` 给出第一个阻塞原因。模型空间不
出现在列表里——它始终可用，用 `SpaceSelection::Model` 表示。

桥接层导出：

```rust
// cad-ui-slint
pub fn layout_descriptors(db: &DrawingDatabase) -> Vec<LayoutDescriptor>;
pub fn build_scene_with_space(
    db: &DrawingDatabase,
    stamp: TaskStamp,
    fonts: Option<Arc<FontEngine>>,
    overrides: &LayerOverrideSet,
    space: SpaceSelection,          // 新增的空间参数
) -> CadResult<SceneDelta>;
```

`build_scene` / `build_scene_with_fonts` / `build_scene_with_overrides` 保持原签名，
内部等价于 `SpaceSelection::Model`，因此既有模型空间入口不受影响。

## 2. 视口变换约定

importer 写入的 `PaperViewport`（`cad-db`）只有三个裁剪点和一支配比例：

- `clip[0]`、`clip[1]`：纸面矩形的两个对角（纸面坐标）；
- `clip[2]`：映射到矩形中心的**模型空间**点（源数据里的 view center）；
- `model_to_paper`：来源中为 `Transform3::scale(view_height / paper_height)`，即
  **模型单位 / 纸面单位**。

本层从中推导出精确的**模型 → 纸面**映射：

```text
paper = (model - anchor) * s + c
```

- `c` = 矩形中心 = `(clip[0] + clip[1]) / 2`；
- 半宽高 `h` = `|clip[1] - clip[0]| / 2`；
- `anchor` = `clip[2]`；
- `s = 1 / (模型单位/纸面单位)`，即**纸面单位 / 模型单位**。

审计 B22 指出的「比例方向需纠正」正是这里：来源存的是模型/纸面，绘制需要纸面/模型，
两者互为倒数。`ViewportTransform` 同时提供 `paper_to_model`（精确逆），纸空间测量若要
重新接线，可从这里取唯一来源——但见第 4 节，测量仍被禁用。

## 3. 支持与不支持

`viewport_transform(&PaperViewport) -> ViewportState`：

- `ViewportState::Supported(ViewportTransform)`：矩形、均匀正比例、数据有限。
- `ViewportState::Unsupported(String)`：明确拒绝，**绝不按猜测比例绘制**。

被拒绝（显式报告）的视口状态：

| 状态 | 判定 | 报告 |
|---|---|---|
| 复杂/非矩形裁剪 | `completeness == Partial(reasons)` | `viewport is not fully supported: <reason>` |
| 裁剪点不是三个 | `clip.len() != 3` | `viewport clip must be three points ...` |
| 非有限坐标 | 任一点非有限 | `viewport clip has non-finite coordinates` |
| 相邻角（非对角） | 宽或高 ≤ 1e-12 | `viewport clip corners are adjacent, not opposite ...` |
| 退化矩形 | 两角重合，宽高均为 0 | 同上（相邻角判定） |
| 缺比例 / 零 / 负 | `m[0][0]` 非有限或 ≤ 0 | `viewport scale must be ...` |
| 旋转 / 错切 / 镜像 / 非均匀 | 非纯比例 | `viewport transform is rotated, sheared or non-uniform ...` |

一个不支持视口的布局在 `enumerate_layouts` 中 `supported = false`；构建该布局时
不产生任何视口几何，并写入诊断码 `representation.viewport_unsupported`，
completeness 降为 `Missing`。布局本身不存在时诊断为 `layout.missing`。
**「读到了视口结构」不等于「能正确显示」**，本层按此区分。

### 矩形裁剪

`clip_polyline_to_rect(paper, center, half) -> Vec<Vec<Point3>>` 用 Liang–Barsky
逐段裁剪：完全在外丢弃、完全在内原样保留、跨越边界的按交点截断，并把连续的幸存段
合并成折线（离开再进入会拆成多段）。线/填充几何的裁剪是精确的。

## 3.1 与当前 importer 的关键差距（必须显式记录）

`cad-import-acadrust::read_layouts` 现在把 `clip` 写成
`[center - half, (center.x + half.x, center.y - half.y), view_center]`，即**两个下角
共享同一 y**，且没有存储纸面高度；`model_to_paper` 方向也是反的（B22）。这意味着
`PaperViewport` 目前**无法无损表达一个矩形**：缺少另一个纸面维度，任何重构都是猜测。

本层因此对「相邻角」显式拒绝（`viewport clip corners are adjacent, not opposite ...`），
而不是猜一个高度。要闭环真实文件的纸空间显示，importer 必须把矩形写成
**两个对角**（并保持 `clip[2]` 为模型 view center、修正比例方向）；这属导入工作流，
不在本轮 `cad-representation` / `cad-ui-slint` 范围。本层已就绪：一旦导入侧给出
对角矩形与正确比例，`viewport_transform` 立即支持，且已有测试覆盖。模型空间显示
不受影响（始终可用）。

## 4. 明确未完成（不是已支持）

- **纸空间测量**：`cad-measure::check_space_policy` 对 `Paper` 与 `ViewportModel`
  一律返回 `Unsupported`。本层**没有**、也不应改写该策略；`ViewportTransform::paper_to_model`
  只是为将来「已验证的逆变换」预留单一来源。见 `docs/measure.md`。
- **视口内网格 / 文字的逐三角裁剪**：桥接层对线几何精确裁剪；`Mesh`/`Text`/`Image`
  只做变换，并把 completeness 降为 `Partial`（`viewport clip applied to line geometry only`），
  不声称已裁剪。
- **打印输出**：无 plot/打印路径。布局绘制是屏幕显示，不是可交付的图纸输出。
- **旋转/非均匀视口、注释性缩放、动态块**：显式 `Unsupported`，按样本标记，不假装支持。
- **布局切换 UI**：`LayoutPanelState`（`cad-ui-slint`）已把布局列表与支持/原因投影成
  面板状态，但 `.slint` 面板控件与宿主命令接线属 UI 工作流；本轮只提供可被调用的
  枚举与切换 API。

## 5. 测试

`cargo test -p cad-representation` 覆盖：

- 变换推导：中心/半宽高、比例取逆（`supported_viewport_derives_centre_half_and_inverted_scale`）、
  锚点落到中心与「1 纸面单位 = N 模型单位」的方向（`model_to_paper_places_the_anchor_at_the_centre`）、
  往返逆变换；
- 矩形裁剪：跨越截断、完全在外丢弃、完全在内保留、连续合并、离开再进入拆分；
- 不支持拒绝：复杂裁剪、零比例、旋转、退化矩形、裁剪点数目错误；
- 布局枚举：真实布局表的 id/名称/支持/原因，空库为空；
- 纸空间构建：模型几何映射并裁剪、不支持视口显式报告且不绘制、纸面实体直接绘制、
  缺失布局为 `Missing`、可见性谓词生效。

`cad-ui-slint` 的桥接测试（模型入口不变、纸空间构建、缺失布局、布局枚举）与
`LayoutPanelState` 测试需在可构建 Slint 的宿主运行；本机缺 fontconfig，故以
wasm32 `--lib` 检查替代编译验证（见诚实边界说明）。
