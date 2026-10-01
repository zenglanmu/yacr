# 批注叠加渲染接线（F07）

本文记录“批注已入库但画布不显示”这一缺口的收口：批注几何如何转成场景批次、
桥接层何时重建、哪些不画以及为什么。权威需求 `CAD_IMPLEMENTATION_SPEC.md` §3.4、
§16.3；追踪 `docs/requirements.md` F07。已有批注数据契约见 `docs/annotations.md`，
捕获工具与管理面板见 `docs/annotation-tools.md`。

## 1. 转换（`cad-scene::annotations`）

新增 `crates/cad-scene/src/annotations.rs`，把 `cad-db::AnnotationGeometry` 转成**既有**
的 `cad_scene::RenderBatch` / `RenderTopology::Lines`。没有第二条渲染路径：调用方把批注
批次与图纸批次拼成一个 `SceneDelta`，交给同一个 `Renderer::upload`。

| 几何 | 画法 |
|---|---|
| `Text` | 用 `cad_representation::FontEngine` 把 `annotation.text` 轮廓化成折线；无字体引擎时显式 `Missing`，不画占位框 |
| `Leader` | 开放折线 |
| `Freehand` | 开放折线 |
| `Rectangle` | 闭合 4 角环（首尾同点） |
| `Ellipse` | 参数化离散 `p(t)=center+axis_u·cos t+axis_v·sin t`，段数复用 `cad_geometry::arc_segments_for_tolerance` 的弦高规则 |
| `Cloud` | 闭合折线（**不画**圆弧扇贝） |
| `Measurement` | **不画**：测量记录属测量叠加路径，显式 `Missing` + 诊断 |

- 出入参：`annotation_batches(annotations, visible, &AnnotationSceneOptions)`。
  `visible` 是会话可见性谓词 `Fn(AnnotationId)->bool`；本 crate 不能依赖 `cad-app`，
  所以桥接层传入 `AnnotationApp::AnnotationVisibilitySet::effective`。
- 输出 `AnnotationScene { batches, completeness, diagnostics, usage, budget_exceeded }`：
  每个受影响批注都有一条诊断（`annotation.unsupported` / `annotation.color` /
  `annotation.empty` / `annotation.text_unshaped` / `annotation.cloud_plain` /
  `annotation.budget`），并汇总成 `Completeness::Partial/Missing`。
- 顶点按批次局部原点存储，复用大坐标精度策略；常量 `alpha` 取 `style.rgba[3]/255`，
  `sources` 用 `EntityId(annotation.id.0)`，`draw_order` 从 `draw_order_base` 起递增。
- 空/退化几何（单点折线、零面积矩形、零半轴椭圆）→ 显式 `Missing`，绝不静默丢弃。
- 帧预算：每个批次经 `FrameBudget::charge` 计费；超限则停止追加、置
  `budget_exceeded` 并记 `annotation.budget` 诊断。默认预算来自 `SceneBudget::default()`。

纯转换与可见性过滤逻辑的单元测试见该模块 `#[cfg(test)]`（17 项，含椭圆在曲线上的
数值断言、去重、退化、隐藏非缺口、预算截断、大坐标精度）。

## 2. 桥接接线（`cad-ui-slint::bridge`）

- `build_scene_with_annotations(...)`：先 `build_scene_with_overrides` 出图纸批次，
  再跑 `annotation_batches`，把两者拼进同一 `SceneDelta` 返回
  `(SceneDelta, AnnotationScene)`。批注数据库为 `None` 时是显式空叠加（宿主无
  sidecar 仍可用），不是静默丢弃一个已知数据库。
- `CadView::set_annotations(Arc<AnnotationDatabase>)` / `clear_annotations()`：宿主把
  打开的批注库交给视图。
- `CadView::set_annotation_visibility(AnnotationVisibilitySet)`：应用会话显隐覆盖；
  显隐是会话状态，不推进批注 revision、不写边车。
- `annotation_fingerprint(db, visibility)`：把库 id、revision、len 与逐条覆盖一起哈希。

### 重建触发（`BeforeRendering` 内，满足任一即重建）

1. 图纸 `SceneIdentity` 变化（含同一 `DatabaseId` 换内容，审计 B04）；
2. 字体存在性变化（有/无 shaping 字体）；
3. 图层覆盖指纹变化（F03）；
4. **批注指纹变化**：批注 revision 前进（创建/修改/删除走事务路径）**或**显隐覆盖变化；
5. 设备代数变化（后端重建/设备丢失后重新初始化）。

命中后 `renderer.clear_batches()` 再 `upload(合并 delta)`，即一次重建同时刷新图纸与批注；
隐藏/创建/编辑批注不需要宿主再次调用 `set_annotations`，指纹已包含 revision 与覆盖。

## 3. 明确不画（不是完成）

- **批注颜色 RGB 不绘制**：`RenderBatch` 只携带常量 `alpha`，管线 uniform 只写 alpha
  （`cad-render-wgpu` 无逐批次颜色）。alpha 通道生效，RGB 记为 `annotation.color`
  诊断，属已知缺口，需要批次/RenderBatch 增加颜色才能补全。
- **绘制顺序**：批次按上传顺序绘制；`draw_order` 已填充但渲染器当前不排序，批注可能
  与图纸批次穿插。要保证“批注置顶”需渲染器支持排序，属后续工作。
- **测量叠加**：`Measurement` 几何不进批注叠加；测量预览/结果渲染属测量路径
  （`docs/ui.md` §2 记录同一缺口）。
- **修订云扇贝**：存储几何是点环，画成普通闭合折线。
- **文字**：需要宿主提供 `FontEngine`；无字体时文字为 `Missing`，不是假框。批注无字体
  字段，使用约定 `DEFAULT_ANNOTATION_FONT = "arial.ttf"` 并走 fallback 链。
- **穿透/遮挡**：批注是额外线批次，不参与深度偏置或置于图纸之上。

## 4. 未运行（证据边界）

- **GPU 提交未运行**：本机缺 fontconfig，`cad-ui-slint` 不能原生构建；桥接改动只由
  `cargo check --workspace --lib --target wasm32-unknown-unknown` 保证可编译。
- **无视觉/真机验收**：没有 Slint 窗口事件循环、纹理合成或设备上的帧。
- 纯转换与可见性过滤由 `cargo test -p cad-scene` 覆盖，与 GPU 无关。

## 5. 验证

```bash
cargo test -p cad-scene -p cad-annotations --locked
cargo check --workspace --lib --target wasm32-unknown-unknown --locked
cargo clippy -p cad-scene -p cad-annotations --all-targets --locked
```
