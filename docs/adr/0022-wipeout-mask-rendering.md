# ADR 0022：WIPEOUT 遮罩的渲染方式（绘制顺序 vs 深度）

- 状态：**Proposed**（设计已按现有渲染管线事实推导；未实施）。
- 日期：2026-10-21。
- 相关：[ADR-0002](0002-gpu-ui-composition.md)（已退役）、`docs/architecture.md`（渲染器边界）、
  [ADR-0018](0018-hatch-family.md)（填充/渐变）、`docs/compatibility.md`（WIPEOUT 行）。

---

## 问题

`WIPEOUT` 导入为 `SemanticGeometry::Mask`（边界 + `inverted`），**报 `Partial`**（`primitives.rs:139-143`：
"mask boundary only; the mask fill is not rendered"）。今天它在屏幕上只是一圈**多余轮廓**，
**不遮挡任何东西**；而 `cad-domain/src/lib.rs:441-447` 的文档却写着"渲染器用视口背景色填充它"——
**该路径不存在**（表示层根本没有 `Mask` 分支，落到 `push_chord_fragment` → 边界折线）。

于是要做的不只是"加一个填充"，而是回答：**在一个按深度测试的单 pass 渲染器里，怎么让后画的填充真正盖住先画的东西。**

## 事实（已勘察，附出处）

1. **单 pass + 深度测试**：`DEPTH_FORMAT = Depth32Float`（`cad-render-wgpu/src/lib.rs:95`），
   mesh 用 `Less` 且写深度（`pipelines.rs:129-135,181-187`），线用 `LessEqual` 且写深度（`init.rs:232-236`），
   透明体测深度但**不写**（`init.rs:387-405`），图像最后画、`LessEqual` 且写（`init.rs:489-497`）。
   `encode.rs:125-129` 自己写明：opaque mesh 与 line **跨类绘制顺序不可观察**。
   ⇒ **同一世界 z 下，后画的不透明 mesh 会被深度拒绝**；"再画一个背景色填充"**不成立**。
2. **没有模板**：`StencilState::default()` + `stencil_ops: None`（`encode.rs:96`），深度格式**无模板位**。
   全仓库**没有** `set_scissor_rect`。
3. **绘制顺序数据存在，但在上一层被丢掉**：`model_space()` 已按 `DbEntity::draw_order` 升序
   （`cad-db/src/drawing/access.rs:431-439`），但 `DisplayFragment` **没有 `draw_order` 字段**
   （`cad-representation/src/primitive.rs:273-319`），`SceneCache` 恒发 `SCENE_DRAW_ORDER = 0`
   （`cad-scene/src/cache.rs:6-14`），且 opaque mesh **跨批次拍平合批、忽略顺序**（`plan/pack_mesh.rs:119-129`）。
4. **背景色在渲染器私有侧**：`CAD_CLEAR_COLOR`（黑，`pub(crate)`，`cad-render-wgpu/src/lib.rs:72-82`）；
   表示层只有 `DEFAULT_RENDER_COLOR = [1,1,1]`。⇒ 表示层**无法**自己给出"背景色填充"。
5. `inverted`（DXF clip_mode = Inside）今天只被变换/包围盒/属性/导出丢弃使用，**渲染侧未消费**。

## 决策

### D1 遮罩是**独立图元 + 独立 pass**，不是"背景色 mesh"

- 表示层新增**无颜色**的 `DisplayPrimitive::Mask`（承载**已三角化的补集多边形**，见 D3）；
  它**不带颜色**，因为"背景"属于渲染器，而 `cad-representation` 必须保持 GPU/宿主无关
  （`scripts/check-architecture.py` 的边界）。
- 渲染器新增**最后一个 pass**：`depth_compare: Always`（不测深度、不写深度）、以 **clear 颜色**绘制。
  这样它**必然**盖住此前一切已经画出的像素，与"wipeout 应该在视觉上抹掉下面的东西"一致。
- **不引入模板**（深度格式无模板位），也**不做** scissor（与形状无关）。

### D2 与绘制顺序的关系：**声明一个子集**

- 真正的 WIPEOUT 语义是**按显示顺序**遮挡：draw_order 更低的实体不应被 draw_order 更高的遮罩挡住。
  但**顺序数据在表示层已被丢弃**（事实 3），要正确实现必须先把 `draw_order` 一路穿到
  `DisplayFragment` →批次→排序（并解决 opaque mesh 合批忽略顺序的问题）。
- 本决策**首版**采用**"遮罩恒为最后"**（简单、可预测），并**显式声明**：遮罩总是覆盖其下所有已绘制内容，
  **不保证**与更高 draw_order 实体的交错顺序。
- 后续若要严格交错：把 `draw_order` 穿到表示层并按顺序分段绘制（每段一个遮罩 pass），
  **那是另一项渲染器改造**，不在本决策内。

### D3 `inverted` 用**补集多边形**表达，不用模板

- 非 `inverted`：遮罩 = 边界多边形（even-odd 填充，复用 HATCH 的 `fill_rings` 思路）。
- `inverted`：遮罩 = **视口矩形 + 边界作为洞**的 even-odd 网格（几何即补集），
  从而在不引入模板的前提下把"边界外"抹掉。
- 视口矩形在**世界坐标**下由当前相机/裁剪范围给出 ⇒ 表示层需要"视口四角"的既有输入
  （若无，则渲染器在 pass 内以屏幕空间全屏三角形 + 边界裁剪实现；**二选一在实施时定**，见 D5）。

### D4 其它消费方

- **矢量出图（plot_vector）**：本决策**不覆盖**；遮罩在矢量出图中**跳过并具名声明**（不静默）。
- **拾取/捕捉/包围盒**：**不变**（遮罩不改变几何语义；今天已是边界参与）。
- **`inverted` 的属性/导出**：**不变**（导出仍丢弃并具名）。

### D5 待实施时确定的点

- 补集的实现位置：**表示层世界坐标补集网格** vs **渲染器屏幕空间全屏 + 边界裁剪**。
  前者与既有图元一致、无需新管线；后者对任意多边形更稳。**按实测选定并记录**。
- 一个 pass 内多个遮罩之间的顺序：按 `draw_order` 升序（同批次内稳定）。

## 后果

- 表示层多一个**无颜色**图元；渲染器多一个 pass（一次额外的全屏/多边形绘制，开销随遮罩数量线性）。
- **诚实性提升**：`cad-domain` 里"渲染器用背景色填充"的假陈述必须与实现同步（要么实现，要么改写为"未渲染"）。
- **明确不承诺**：与更高 draw_order 实体的交错顺序、矢量出图、`inverted` 以外的高级裁剪。

## 验证计划（真实 GPU）

1. **契约测试**：遮罩图元的三角化（非 `inverted` = 边界；`inverted` = 矩形减边界）；
   `Completeness`/理由不变；拾取/包围盒不变。
2. **真实设备读回**（`cad-render-wgpu` 的 headless render 已能读像素）：
   一条**穿过遮罩边界内的线**在遮罩区域内必须**不可见**、在区域外可见；`inverted` 时相反。
   用像素断言，不用"看起来对"。
3. **真机截图**：与读回二选一或并用；记录合成器/GPU/scale 边界，不推广。
4. **反例保留**：把"后画 mesh 被深度拒绝"的对照写进测试注释，防止有人把它"简化"成背景色 mesh
   而再次得到不遮罩的结果。
