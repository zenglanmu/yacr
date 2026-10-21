# ADR 0022：WIPEOUT 遮罩的渲染方式（绘制顺序 vs 深度）

- 状态：**Implemented**（C1 与 C2 均已落地，均有**真实设备**像素证据与**反例对照**）。
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
3. **绘制顺序数据（2026-10-21 已修正）**：`model_space()` 按 `DbEntity::draw_order` 升序
   （`cad-db/src/drawing/access.rs:431-439`）；此前 `DisplayFragment` 无该字段、`SceneCache` 恒发
   `SCENE_DRAW_ORDER = 0`，现已**贯通**——`DisplayFragment::draw_order` 由源实体携带（**INSERT 展开的子实体继承
   所在 INSERT 的 order**，块内自有 order 只在块定义内有意义），`RenderBatch`/`MaskBatch` 直接取其值，
   行/圆锥的合批谓词要求 `batch.draw_order == incoming`，因此**合批不会跨顺序吸收几何**；
   `SCENE_DRAW_ORDER` 已删除（`ImageBatch` 仍为 0，图像遍另议，已在注释中具名）。
   **仍未贯通的一段**：渲染器的 opaque mesh 仍**跨批次拍平合批并忽略顺序**（`plan/pack_mesh.rs`）——
   这是渲染器分段的前置条件之一。
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
  顺序数据现已从数据库穿到 `DisplayFragment` → `RenderBatch`/`MaskBatch`（事实 3 的修正），
  但**渲染器仍然按上传顺序绘制、遮罩恒为最后一遍**。
- 本决策**首版**仍为**"遮罩恒为最后"**，并**显式声明**：遮罩总是覆盖其下所有已绘制内容，
  **不保证**与更高 draw_order 实体的交错顺序。声明保持不变，但**理由已从"数据被丢掉"变成
  "渲染器分段尚未实现"**。
- 后续若要严格交错：按 `draw_order` 分段绘制（每段一次遮罩 pass），并先解除 opaque mesh 的
  跨批次拍平（`plan/pack_mesh.rs`）；**那是另一项渲染器改造**，不在本决策内。
  数据侧已就绪（2026-10-21）。

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

## 实施记录（2026-10-21）

### C1（非 `inverted`）— 已落地

- **图元**：`DisplayPrimitive::Mask(Arc<[Point3]>)`，**无颜色**、只带世界坐标闭合边界；`transformed()` 逐顶点映射。
- **表示层**：`SemanticGeometry::Mask` 非 `inverted` 且 ≥3 点 ⇒ 一个 `Mask` 片段，`Complete`；
  `inverted` ⇒ **保持今天的边界折线** + `Partial(MASK_INVERTED_NOT_RENDERED)` + `representation.mask_inverted`
  （**不假装**）；退化边界 ⇒ 折线 + 具名 `Partial`。
- **矢量出图**：遮罩**跳过**并具名 `vector.mask_unsupported`（ADR D4，不静默）。
- **场景**：`MaskBatch`（保留世界多边形），`SceneDelta.masks`，按源变更失效；<3 点**报错**而不是静默丢弃。
- **渲染器**：新增 **pass 4**（在图像与两个几何 pass 之后）：`mask_pipeline` = triangle list、
  `cull_mode: None`、`depth_write_enabled: false`、**`depth_compare: Always`**、`ALPHA_BLENDING` over，
  颜色取自共享相机 uniform 的 **clear 颜色**；**凹多边形复用 HATCH 的 even-odd `fill_rings`**（不使用三角扇）。
- **真实设备像素证据**（Vulkan `Quadro P620`，discrete，驱动 `NVIDIA 580.178.04`，**非 lavapipe**）：
  64×64、2 世界单位；一条 `y=0` 的线画在 31/32 行。矩形遮罩：外部 `[187,187,187,255]`、内部 **`[0,0,0,255]` = clear 颜色**；
  **凹形 L**：左竖条被抹 `[0,0,0,255]`、**缺口 `[187,187,187,255]`（线存活，无三角扇泄漏）**、外部不变。
  `stats.mask_draws == 1`。
- **反例对照（关键）**：把 pass 临时改成 `if false && ...` 后**两条像素测试都失败**
  （"mask interior must equal the clear colour … got [187,187,187,255]"），恢复后通过 ⇒ 测试测的是真东西。
- **仍然声明不做**：`inverted` 填充（C2）；与更高 `draw_order` 的交错（D2 子集，遮罩恒最后）；
  矢量出图（跳过并具名）；纸张空间视口裁剪（保留并报 `UNSUPPORTED_PRIMITIVE`，不伪造裁剪）；
  遮罩不可拾取（未变，无 pick key）；不引入模板、不引入 scissor、无异步改造。

### C2（`inverted`）— 已落地（2026-10-21）

**做法（D5 的选定）**：**不需要模板、也不需要新的三角化**——复用 C1 的 **even-odd `fill_rings`**：
把**视口覆盖环**与**掩码边界**作为**两个环**一起做 even-odd ⇒ 边界内被覆盖两次（**不填充**）、
边界外覆盖一次（**填充**）= 精确补集。

- **标志贯通**：`DisplayPrimitive::Mask { boundary, inverted }` → `MaskBatch.inverted` → 渲染器；
  已删除假的 `MASK_INVERTED_NOT_RENDERED` 路径（两种 clip 模式现在都产生一个 `Mask` 片段，`Complete`）。
- **视口环的构造**：可见世界矩形（**正交 = 精确**；**透视 = 以相机为中心的宽松估计**，`MASK_PERSPECTIVE_MARGIN = 64`）
  **∪ 边界 AABB**，再外扩 **25 %**（`MASK_RING_MARGIN`）。∪ 保证环**严格包含**边界——这正是 even-odd 精确的前提；
  边界比视口还大时（遮罩把屏幕围住）⇒ 可见区域**不抹**（正确）。
- **缓存**：倒置遮罩的几何在首帧构造，**仅当新视口不再被环包含**时重建；相机不动或小幅平移 ⇒ 零重建。
- **真实设备像素证据**（Quadro P620 discrete，非 lavapipe）：64×64、线在 31/32 行。
  **倒置矩形**：外部 x=−0.85/0.6 ⇒ `[0,0,0,255]`（被抹）；内部 x=−0.4/0/0.4 ⇒ `[187,187,187,255]`（**可见**）；
  **倒置 L**：左竖条**可见**、**缺口被抹**、外部被抹。`mask_draws == 1`。
- **反例对照**：把倒置遮罩强制走非倒置路径 ⇒ **两条新测试都失败**（"inverted mask must wipe outside … got [187,187,187,255]"），
  恢复后通过；C1 的非倒置测试全程保持绿。
- **仍然声明不做**：与更高 `draw_order` 的交错（遮罩恒最后）；矢量出图**跳过并具名**；纸张空间视口裁剪报
  `UNSUPPORTED_PRIMITIVE` 而不伪造；**透视视口环是宽松估计**（正交精确）；遮罩**不可拾取**；无模板、无 scissor、无异步改造。
  无法三角化的边界经 `Renderer::mask_skips()` 报告，**从不**以近似填充替代。
