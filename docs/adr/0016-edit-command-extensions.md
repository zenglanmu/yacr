# ADR 0016：编辑命令扩展（FILLET / CHAMFER / ARRAY / STRETCH / ALIGN / EXPLODE）

- 状态：**已接受（Accepted，2026-10-19）**。设计门禁 **SG-P4 attempt 1 = APPROVE-WITH-FIXES**（B1–B7 已按结论
  写入修订）、**attempt 2 = ACCEPT-WITH-FIXES**（B1–B7 全部核实关闭；余 T1–T4 **文本级**修正已随本状态行执行：
  T1 Dimension 实为 `Compound`/`AcDbDimension` 的事实更正、T2 STRETCH 完全包围口径、T3 Compound 成员/ARC 一句话、
  T4 措辞项）。实施进度另起小节追加，不改写决策文本。
- 范围：`crates/cad-app`（命令/工具/几何构造/宿主映射/逐顶点编辑）、`crates/cad-ui-core`（选择子、**类型化参数通道**、
  状态投影、i18n）、React 宿主 UI、以及 EXPLODE 的实体分解路径。**不**覆盖：3D/曲面倒角、关联数组的编辑模型、
  路径阵列、STRETCH 的 cpolygon/lasso 选择、ALIGN 的 3D 对齐、EXPLODE 的 xref/非均匀块。
- 相关：`docs/adr/0015-new-draw-commands.md`（参数通道先例，本 ADR **取代**其"单用途"结论）、`docs/interaction.md`、
  `docs/drawing-edit.md`、`docs/compatibility.md`、`CAD_IMPLEMENTATION_SPEC.md` §4.6/§4.7。

## 实施状态（追加，不改写决策文本）

- **P4a 完成（2026-10-19，提交 c739558）**：类型化参数通道落地（`ToolValue::{Number,Flag}` + 12 个参数成员 +
  单一请求 `SetToolParameter`），**`SetPolygonSides` 及其 IPC/队列/注册全部删除**（每参数仅一个线上入口）；
  各参数自带范围与显式拒绝（边数整型 3–1024、半径/距离 ≥0、计数 ≥1、填充角非零且非整圈、Flag 布尔）；
  `params.sides` 线上字段**冻结不变**；React 侧边数控件泛化为"活动工具参数组"（数值步进 + 布尔开关），
  并加测试断言 `cad_set_polygon_sides` 在 UI 源码中彻底消失。
- **P4b 完成（2026-10-19）**：`CHAMFER`/`FILLET`（两条独立直线）落地，含几何前置：
  `bulge_arc_geometry` 升 `pub`（单一实现）+ 新增 `infinite_line_intersection`（`Point`/`Parallel`/`Collinear`
  三态可区分）。CHAMFER 两距离**按拾取顺序**、`0/0` 与 `FILLET R=0` 有**等价性对照测试**；
  FILLET 相切弧数值验证（圆心到两线距离 = R、切点即修剪点），**圆心构造标注 UNVERIFIED（几何推论）**；
  新实体在**活动图层**（我们的选择）；拒绝面含平行/共线、同实体两次、跨文档、非直线段、点在角点、
  距离过长、缺层、Viewer。**实现偏离记录**：① `DrawPreview` 未扩展（`cad-host` 冻结，改由 `cad-ui-core`
  直接读会话参数存储投影，线上结果相同）；② 载荷 boxed（`Chamfer(Box<TwoLineCommand>)`，避免 clippy
  `large_enum_variant` 触发对冻结的 `cad-engine` 的告警）。
- **P4c 完成（提交 bad87c2）**：`ARRAYRECT`/`ARRAYPOLAR`（独立副本、一条事务一步撤销、源保留）。
  **首版两条规则有误并已修**：① 填充角规则与自己的默认值矛盾（默认 360 却拒绝整圈 ⇒ 用户步进后回不到默认）
  → 改为**有限、非零、`|angle| ≤ 360`**，并同步 UI 步进边界；② 副本数多一个（`rows×cols`/`items` 是**含原对象
  的总数**）→ 改为 `rows×cols−1` / `items−1`，1×1 与单项阵列**不产生副本并显式诊断**、不开事务不留撤销。
  所有受影响的期望（实体数、坐标、DXF 断言、一条撤销用例）已同步。
- **P4d 完成（提交 b43e1e3）**：`ALIGN`（两对点 `Exact(4)`、`AlignScale` 默认关、以 `source1` 为枢轴
  `translate∘rotate∘[scale]`、原位变换、一条事务一步撤销）；数值测试钉住 90° 旋转与 3 倍均匀缩放、
  矩形保持 2:1 边长比（证明均匀）。
- **P4e 完成（提交 95bb7ff）**：`STRETCH`（4 点、窗口恒 CROSSING、分类在命令内重算且按几何类型分两种口径、
  部分包围=逐顶点、完全包围=整移、圆/椭圆/块/ARC 部分包围不动+诊断、`TEXT` 整命令拒绝、
  **bulge 数值保留**并数值钉住 0.5、无变化时不开事务只给诊断）；载荷 `delta` 为**位移点**（平移 = `delta − base`）。
- **P4f 完成（提交 a492186）**：`EXPLODE`（**一次一层**；EXPLODE 与 ERASE 一样是**无载荷直命令**、不需要工具状态，
  故只有 `draw.kind.explode` 而没有 `tool.draw.explode`）。**首版与决策文本矛盾并已修**：lane 曾照搬导出层的
  递归走查（把嵌套块**摊平到叶子**），与"一次一层"冲突 ⇒ 改为成员按一层产出、**嵌套块引用保持为引用**（placement
  复合进去）、二次 EXPLODE 才展开；导出层的循环栈/深度上限**整段删除**（一层走查下不可达，保留即等于假装加固），
  ADR 的"与导出层的关系"也改为明确写"导出=摊平到叶子、EXPLODE=一次一层，**刻意不同**；要摊平是另一个命令"。
  另将 `polyline_plane_normal` 升 `pub`（EXPLODE 需在与 tessellator 同一平面内还原 bulge 弧，避免重推导 Newell 法向）。
- **待办**：P4 阶段门禁（Oracle）；真机证据（`ARRAYRECT`/`ARRAYPOLAR`/`ALIGN`/`STRETCH`/`EXPLODE` 本轮正在采集）。

## 背景（勘察事实，2026-10-19；已按 SG-P4 逐条核对更正）

1. **六个命令当前完全不存在**：无 `CommandId`/`CommandPayload`/`DrawToolKind`/选择子/ribbon 入口/测试；
   chrome 里也不是"死入口"（从未登记）。规格里没有它们（代码注释里的"spec F-EDIT"指的是 `docs/drawing-edit.md`）。
2. **多实体一次事务 + 一步撤销已具备**：`cad-db/src/drawing/transaction.rs` 支持同一事务内 insert/update/delete
   多实体、全有或全无；`copy_entities`/`mirror_selection`/paste 已在用（测试 `mirror_of_multiple_refs_is_one_undo_step`），
   `apply_drawing_patches` 支持多 patch 撤销/重做 ⇒ "2 处 update + 1 处 insert = 一条撤销记录"**结构上已支持**
   （`DrawingUndoRecord.patches` 可容纳混合 patch；MOVE 已在 stage 多 update）——**字面覆盖由 P4b 的 FILLET 验收测试补齐**。
   注意：多实体路径**本就绕过**单实体漏斗 `commit_new_entity_in` ⇒ 栅栏应表述为**不变量集合**（图层存在、
   模型空间、一次事务/一步撤销），而不是字面复用该函数。
3. **没有逐顶点编辑路径**：`transform_geometry` 只做整实体仿射；无"移动 LWPOLYLINE 某顶点/LINE 某端点"的辅助。
   也**没有**"部分包围 vs 完全包围"的分类：`select_in_box` 只产出 ref 集合，其 `MatchPath` 只区分**精度路径**
   （exact/approx/aabb），从不做包含分类（原稿"算出又丢弃"的措辞不准确，已改）。
4. **旋转/缩放/镜像构造器已存在**：`cad-app/src/draw_tool/geometry.rs`（`rotation_transform`/`scale_transform`/
   `mirror_transform`，`pub(crate)`，有测试），`drawing_edit.rs` 已在调用 ⇒ **ALIGN 不需要新构造器**
   （`Transform3` 本身只有 identity/translation/scale/from_basis，但工作不在那里）。原稿"临时拼装、需收敛"**不成立**。
5. **可复用辅助与其可见性**：`line_segment_intersection_param`（`app_drawing/helpers.rs`，**模块私有**，且是
   无限直线 × **线段**）、`segment_segment_intersection_3d`（`cad-geometry/src/intersect.rs`，**`pub(crate)`**，
   线段×线段）、`bulge_arc_geometry`（`cad-geometry/src/transform.rs`，**`pub(crate)`**）、`collect_boundary_segments`、
   `plan_trim`、`plan_extend`、`offset_line`、`resolve_selection`、`apply_drawing_patches`。
   ⇒ 需要**升 `pub` 保持单一实现**（或在本 crate 重实现并说明理由——**不能**"re-export 别的 crate 的 `pub(crate)`"），
   并**补一个无限直线×无限直线的求交辅助**（FILLET/CHAMFER 的延伸情形需要角点，现有两个都不够）。
   `cad-geometry` 没有 fillet/chamfer/角平分线/阵列辅助。
6. **EXPLODE 的 INSERT 展开在导出层已有原型**（组合 placement 变换、成员、循环守卫、深度上限 32），**写路径没有**。
   **拒绝面更正**：`transform_geometry` 只拒绝 **Text/Spline/Shape/Opaque/Compound** 与**镜像下的 Arc/Ellipse**
   （曲线类的非相似变换亦拒绝）；**`Image`/`Mask` 是可变换的**——EXPLODE 拒绝它们必须给出**策略理由**
   （AutoCAD 拒绝分解图像；其官方条文**未查证**），**不能**假称"被既有变换拒绝"。
   **MINSERT 已被建模**为 `Compound(Insert 实例…)`（`cad-import-acadrust/src/entity/insert.rs`）⇒ "一次一层"规则
   天然处理：第一次 EXPLODE 得到 Insert 实例，下一次才展开成员；应如此表述，而非"不适用"。
   动态块的成员可见性在导出层按活动状态过滤（`export/entity.rs`）——我们的策略须显式声明（见决策 G）。
7. **`SemanticGeometry::Polyline` 没有宽度字段** ⇒ 原稿中"EXPLODE 丢弃宽度"与"STRETCH 不改宽度"都是**空话**
   （宽度在导入时即丢失、根本没建模）；保留为"未建模"的说明，不写成能力。
8. **夹具限制（属范围选择，非必然）**：现有夹具是四条**平行**竖线，无法演示 FILLET/CHAMFER/EXPLODE；但该夹具
   本身是**仓库内用 acadrust `DwgWriter` 生成**的 ⇒ 需要的话新造一个"相交直线 + 矩形 + 闭合 bulge 折线 + 块 INSERT"
   的合成夹具是**便宜且 provenance 干净**的。本 ADR 选择**先不新造**（真实 GPU 证据用 P3 的绘图命令现场造几何），
   并记录这是选择而非限制。

## 决策 A：类型化参数通道（取代 P3 的"单用途"结论）

P3 的"只做 `SetPolygonSides`、不建通用总线"以"单一消费者"为前提；本阶段出现**多个数值参数 + 布尔参数 + 两种阵列类型**
⇒ 前提消失。决策：

- 值模型：**`ToolValue::{Number(f64), Flag(bool)}`**；请求：`InteractionRequest::SetToolParameter { parameter, value }`。
  枚举成员（本阶段）：`PolygonSides`、`FilletRadius`、`ChamferDistanceA`、`ChamferDistanceB`、`AlignScale`、
  `ArrayRows`、`ArrayColumns`、`ArrayRowSpacing`、`ArrayColumnSpacing`、`ArrayItems`、`ArrayFillAngle`、`ArrayRotateItems`。
- **`SetPolygonSides` 迁移**到该通道（**不留两套入口**，避免同一参数两个真相）。迁移面**逐项列出**：
  `InteractionRequest::SetPolygonSides`、`CadSession::set_polygon_sides`、IPC `cad_set_polygon_sides` 与其准入、
  app-linux 的 `cad_polygon_sides` 队列与 tick、`entry.rs` 注册、**证据驱动的脚本字符串与其契约测试**。
  迁移后 `params.sides` 名称与语义**保持不变**（冻结字段），但**边数改经 `Number` 承载** ⇒ 必须钉住**整型规则**
  （如 4.5 显式拒绝），并新增"单一 wire 路径"测试。
- 各参数**各自**定义合法范围与拒绝（显式拒绝、**不 clamp**）：边数 3–1024 整型；FILLET 半径 / CHAMFER 距离 ≥ 0 且有限；
  阵列行/列/项数 ≥ 1 整型；行/列间距有限；**填充角**：要求**有限、非零、且 `|angle| ≤ 360`**（`360` 是"整圈"默认值，
  因此**必须可被重新设置**；`0` 与 `|angle| > 360` 显式拒绝，不做隐式取模/环绕。原稿"非整圈倍数"的措辞与
  默认值 360 自相矛盾，已按此更正）；
  `Flag` 取布尔。
- **生命周期**同 `polygon_sides`（会话作用域；跨切工具/取消保持；文档更换/新建回默认；Viewer 拒绝）。
  **有意偏离**：AutoCAD 的 `FILLETRAD`/`CHAMFERA/B` **存于图纸**（`TRIMMODE` 依社区资料存注册表，**官方页未查证**），
  我们**只存会话**，能力表标注。
- 投影：`CadTool.params` 扩展（新字段 + `null` 规则同前），线格式扩展带 `cad-ui-core` 契约测试。
- UI：P3 的边数步进**泛化为"活动工具的数值参数控件"**（可用态由 `tool.key` 派生；不做乐观更新；待发累积复用），
  `Flag` 参数用开关控件；由 designer 实现。

## 决策 B：FILLET（两条独立直线）

- **交互**：两次**实体拾取**（沿用 TRIM/EXTEND/Offset 的"取点即解算实体"路径——`DrawIntent::Trim` 已携带捕获的
  `SelectionRef`；原稿把它归给 P3 绘图命令，已更正）。先拾取者为"第一对象"；**拾取侧**决定方向（AutoCAD：
  "始终选取最靠近希望端点出现位置的对象"）。
- **几何**：`R = FilletRadius`（默认 **0**，对齐 `FILLETRAD` 初值）。`R = 0` ⇒ **只把两段延伸/修剪到角点，不产生弧**
  （AutoCAD 明示，需直线×直线角点辅助）。`R > 0` ⇒ 生成与两段相切的 ARC，两段被修剪到切点；
  结果 = **两处 `update_entity` + 一处 insert（ARC）**，一条事务、一条撤销记录。
- **弧的构造与标注**：圆心到两直线距离均为 `R` 且位于**拾取侧**；Autodesk **未给出圆心公式** ⇒ 实现注释、
  能力表与栅栏均标 **UNVERIFIED（几何推论）**，并用"拾取侧选择角点"的测试钉住该推论。
- **新实体的图层归属**（AutoCAD 规则未查证）：本阶段取**当前活动图层**，显式声明为**我们的选择**并记入能力表。
- **拒绝（显式，不写库）**：非有限/负半径；两段不共面（`+Z` 子集）；**平行**（AutoCAD 会自动调半径——本阶段
  **显式拒绝**并说明）；拾取到的不是直线段；**同一实体被拾取两次**；跨文档 ref；缺层；**段过短**以致无法容纳半径
  （AutoCAD "不修改"，我们**显式拒绝**，避免静默不动）。
- **NOT IMPLEMENTED**：`Polyline` 模式、`Multiple`、命令内 `Undo`、`Trim/No Trim` 开关（**默认按 `TRIMMODE=1` 修剪**，
  不提供关闭）、平行线自动半径、Circle/Arc 目标、3D、hatch 关联性；**圆心公式的 Autodesk 原文**（推论已标注）。

## 决策 C：CHAMFER（两条独立直线）

- **交互**同 FILLET；**两个距离按拾取顺序施加**（Dist1 → 第一对象；AutoCAD 明示）。
- **几何**：`A/B = ChamferDistanceA/B`（默认 **0/0**）。`A = B = 0` ⇒ **只延伸/修剪到角点**（与 `R=0` 的 FILLET 等价，
  需一条对照测试）。否则按距离在两条线上取点、修剪两段并**新增一条 LINE** ⇒ 两处 update + 一处 insert，一条撤销。
- **拒绝**：与 FILLET 同构；**负距离**按我们的规则一律拒绝（**AutoCAD 对负距离的行为未查证**，标注）。
- **NOT IMPLEMENTED**：`Angle` 方法与 `CHAMMODE`、`Polyline` 模式、`Multiple`、`Undo`、`Trim/No Trim`、
  "中间夹一段弧则替换该弧"、3D。

## 决策 D：ARRAY（`ARRAYRECT` + `ARRAYPOLAR` 两个选择子，**非关联副本**）

- **类型选择不进参数通道**：改用 AutoCAD 本身就有的**两个具体命令选择子** `ARRAYRECT` / `ARRAYPOLAR`；
  通用 `ARRAY` 的"类型提示"流程 **NOT IMPLEMENTED**（能力表写明"请用具体命令"）。选择子因此为 7 个
  （FILLET/CHAMFER/ARRAYRECT/ARRAYPOLAR/STRETCH/ALIGN/EXPLODE）。
- **`ARRAYRECT`**：参数 = 行数、列数、行距、列距（AutoCAD 语义：**基点之间的距离**，不是边到边间隙）；
  被阵列对象视为**左下角**、向上/右生成；**基点取选择集合包围盒的左下角**（我们的规则，显式声明）。
  **副本数 = 行数 × 列数 − 1**（AutoCAD 的 rows×cols 是**含原对象的总数**，原对象保留原位、不额外复制）；
  若该值为 0（1×1）⇒ **不产生任何副本**并给出**显式诊断**（0 也是结果，不得静默成功）。
  负间距的"反向"行为**未查证** ⇒ 本阶段**拒绝负间距**（不做未验证的反向）。
- **`ARRAYPOLAR`**：捕获**中心点**（点，不进参数通道）+ 参数 项数、填充角、是否旋转副本。
  **副本数 = 项数 − 1**（同理：`items` 是含原对象的**总数**，原对象位于其自身方向、不额外复制）；
  **放置数学（须显式声明）**：`rotate = true` ⇒ 每个副本绕中心旋转（顶点整体旋转）；
  `rotate = false` ⇒ **仅平移**到各角度位置、不旋转自身（"基点"取选择包围盒左下角）。
  该规则**AutoCAD 未查证**，标注为我们的选择并加测试。
- **存储**：**独立副本实体**，经既有 N 副本一次事务路径（`copy_entities`/`mirror_selection` 形状）。
  **有意偏离**：AutoCAD 默认**关联**（`ARRAYASSOCIATIVITY`：阵列是"类似块的单一对象"、可 grips/`ARRAYEDIT` 编辑、
  可 `EXPLODE` 打散）；本栈**没有任何关联对象/grip 机制**，发明它会把 P4 整个拖大 ⇒ 我们存副本，
  关联编辑与"EXPLODE 关联阵列"**都不存在**，能力表明确标注。
- **拒绝**：行/列/项数非整或 < 1；间距/角度非有限；负间距（见上）；选择为空；源几何无法仿射变换（既有 `Unsupported` 面）；
  **实体总数超过显式上限 100 000**（AutoCAD `MAXARRAY` 量级）⇒ **显式拒绝**而非截断。
- **NOT IMPLEMENTED**：`Path` 阵列、`Levels`（Z 层）、`Total` 替代输入、**角度间隔**输入、关联性与 `ARRAYEDIT`/grips、
  `MAXARRAY` 可配置化、3D、负间距反向。

## 决策 E：STRETCH（交叉窗口 + 逐顶点）

- **交互（4 点）**：两次点击给**交叉窗口**两个对角点 → 基点 → 位移点。窗口**恒按 CROSSING 语义**处理
  （AutoCAD 明确要求交叉窗口/多边形）。**`CommandPayload::Stretch { rect_logical, base, delta }`**（canvas 逻辑坐标，
  与 `BoxSelect` 同约定），命令内部重算分类。
- **分类口径（更正原稿的"同一套谓词"过度声明）**：分类**不在选择集合里保存**（单一命令内从矩形与几何重算）。
  口径按几何类型**分两种**（T2 修正）：
  - **直线 `LINE` 与 LWPOLYLINE 的直线段**：**逐顶点包含**（`Rect::contains_point`）——"完全包围 = 全部顶点在窗内"
    （矩形是凸的，等价于几何包含）；
  - **`CIRCLE`/`ELLIPSE`/`INSERT` 以及含 bulge 段的折线**：用**保守包围盒完全包含**（即既有框选的 WINDOW 路径
    `contains_aabb`）判定"完全包围"；**bulge 弧可能鼓出窗口而两端点都在窗内**，此边界情形按**我们的选择**处理并标注。
  - "部分包围" = 与窗口有交（复用框选的**命中**判定）但不满足上述"完全包围"。
- **语义（严格对齐 Autodesk 原文）**：**部分包围 ⇒ 拉伸**（只移动**落在窗口内**的顶点/端点）；
  **完全包围 ⇒ 移动**（整实体平移）。**"单独选取"不在本阶段交互内** ⇒ 该子句 **NOT IMPLEMENTED**（原稿的死文本已删）。
- **可拉伸几何**：`LINE` 与 **LWPOLYLINE**（含 bulge 段——原稿"直线段折线"与 bulge 保留规则自相矛盾，已改）。
  **`CIRCLE`/`ELLIPSE`/`INSERT` 不能拉伸**（Autodesk 明示）：完全包围时按**移动**处理；部分包围时**不动**并给出诊断。
  **`ARC` 亦不可拉伸**（可变换但非可拉伸对象）：完全包围时按**移动**处理，部分包围时**不动 + 诊断**。
  **`TEXT`**：Autodesk **未记载**其行为，且 `transform_geometry` 对 Text 本就 `Unsupported`（今天连 MOVE 都动不了文本）
  ⇒ **整命令拒绝**并说明。
- **"与窗口相交但窗口内没有任何顶点"**：**几何不变 + 明确诊断**（不静默）。
- **逐顶点路径的位置**：**`cad-app`**（新点是纯函数构造，随后按 TRIM 的 Keep 分支同样方式 stage `update_entity`；
  `cad-db` 在提交时重新校验）。**不需要**改 `transform_geometry`。
- **bulge（未查证项）**：Autodesk 只保证"不修改宽度/切向/曲线拟合信息"，**未记载**拖动 bulge 段端点时 bulge 如何处理
  ⇒ 我们的规则是**保留 bulge 数值不变**（只移动顶点），标注 **UNVERIFIED** 并用测试钉住。
- **拒绝粒度**：窗口内出现不可变换实体 ⇒ **整命令失败并说明**（与 `copy_entities` 的"整命令拒绝"先例一致），
  **禁止**部分成功。
- **NOT IMPLEMENTED**：cpolygon/lasso、**单独选取即整移**、`TEXT`/`SPLINE`/`ARC`/`HATCH`/`INSERT` 的**拉伸**、3D、
  宽度/切向修改（宽度未建模）、负向/非平面窗口。

## 决策 F：ALIGN（两对点；缩放经 `Flag` 参数，默认关）

- **交互**：**两对点（`Exact(4)`）**。**1 对点（纯移动）NOT IMPLEMENTED**：其效果与既有 `MOVE` 等价，
  且"2 或 4 点、成对确认"现有 arity 模型表达不了（`PointArity::{Exact,AtLeast}` 无法表达"偶数点确认"）——
  与其新增 arity 变体，不如只做两对点这条主路径（原稿的 1 对点承诺已删）。
- **数学**：移动 = 第一对源→目标；旋转 = 两对方向夹角（**直接调用既有的 `rotation_transform`**，
  不需要新构造器、也不需要"收敛"工作项）；平移与旋转合成后整实体变换选择。
- **缩放**：AutoCAD 在第二对点后提示"是否按对齐点缩放"，**默认 No**、只做**均匀**缩放。本阶段**实现**它：
  `AlignScale`（`Flag`，默认 false）为 true 时按 **目标对长度 / 源对长度** 施加**均匀**缩放（复用 `scale_transform`）。
  这属默认路径 + 一个布尔参数，代价很小（原稿"通道没有布尔参数所以不做"的理由在决策 A 落地后**不再成立**）。
- **拒绝**：源/目标点非有限；两对点退化（重合、方向向量为零）；选择为空；无法变换的几何（既有 `Unsupported` 面）。
- **NOT IMPLEMENTED**：3 对点 3D 对齐、**非均匀**缩放、1 对点移动。

## 决策 G：EXPLODE（一次一层）

- **交互**：对**当前选择**执行（无额外提示，AutoCAD 亦如此），Work-only。
- **可分解（本阶段，一次一层）**：
  - **LWPOLYLINE → 直线段 + ARC**（bulge 段经 `bulge_arc_geometry` 还原，需**升 `pub`** 保持单一实现）；
    "宽度丢弃"**不写**（宽度未建模，是空话）。
  - **INSERT（均匀缩放）→ 成员实体**，placement 变换**烘焙**进成员；**一次一层**——成员若**本身是块引用**
    （嵌套 INSERT/MINSERT），它**保持为块引用**（只把 placement 复合进去），其内部成员留待**再次 EXPLODE**；
    **不递归展开**（这与导出层的"摊平到叶子"不同，导出是另一条路径）。因走查只有一层，导出层的
    循环栈/深度上限在此**不需要**（自引用块一次爆炸只会再得到一个自引用引用，不会无限循环），
    实现注释须写明这一点，不得保留不可达的守卫充当"已加固"。
  - **`Compound` → 其全部子几何**（一次一层）。**更正**：hatch 的 `Compound` 子项**包含边界环 + 填充 Mesh +
    图案 Polyline** ⇒ 只分解成"环"是**静默丢失**；本阶段分解为**全部子项**，并把"与 AutoCAD 的 hatch 分解结果
    不同"记为**有意偏离**（AutoCAD 的 hatch 分解产物未逐条查证）。
  - **MINSERT**：已建模为 `Compound(Insert…)` ⇒ 第一次分解得到 Insert 实例，下一次才展开成员（自然落入"一次一层"）。
  - **动态块**：成员可见性沿用导出层的**活动状态**规则（AutoCAD 对动态块的 EXPLODE 行为**未查证**），标注为我们的选择。
  - **成员图层**：保留块定义中各成员自己的图层（与 AutoCAD 一致），加一条测试。
- **拒绝（显式，不写库）**：`TEXT`/`SPLINE`/`SHAPE`/`OPAQUE` 及镜像 Arc/Ellipse 子几何（**已被既有变换拒绝**）；
  **`IMAGE`/`MASK`**：拒绝的理由是**策略**（AutoCAD 拒绝分解图像，其条文**未查证**），**不得**假称被变换拒绝；
  **`Dimension`/`MTable`/`MLeader` 类复合体**：它们导入即 `Compound`（`type_key` 为 `AcDbDimension` 等），
  按"Compound→全部子项"的一般规则**本会被分解**——本阶段**按 `type_key` 显式拒绝**并说明理由（AutoCAD 的
  dimension 分解语义是"关联性丢失 + 产出 2D solid/文本"，本项目未建模该语义；且其子项常含 `Text`，按既有拒绝面
  也会整命令失败——显式拒绝比通用失败更清楚）；
  **非均匀缩放的 INSERT** 与含上述不可变换子几何（**含 `Compound` 成员**，如块内 MINSERT）的 INSERT ⇒
  按"成员按**一层**产出（嵌套块引用保持为引用）；命中既有变换拒绝面即**整命令失败**"处理（不得部分成功）。
- **与导出层的关系**：**不得**从写路径调用导出代码。导出的走查是"**摊平到叶子**"（带循环栈 + 深度上限 32），
  而 EXPLODE 是"**一次一层**"——两者**刻意不同**，因此本命令**不**镜像导出的递归与守卫（一层走查下它们不可达，
  保留即等于把不可达代码当作"已加固"）。若将来需要摊平语义，那是**另一个命令**（如 FLATTEN），不是 EXPLODE。
- **NOT IMPLEMENTED**：多级一次到底（AutoCAD 本就一次一层）、`EXPLMODE` 门控、颜色/线型/线宽变化的复刻（未建模）、
  Dimension 分解（无对应 `SemanticGeometry`）、关联阵列分解（我们不存关联阵列）、属性语义（无属性）。

## 分阶段（依赖顺序）

- **P4a**：类型化参数通道（`ToolValue` + `SetToolParameter` + **迁移 `PolygonSides`**）+ UI 数值/开关控件泛化（designer）。
  **阵列参数必须落在本阶段**（否则 P4c 会再造一条通道）。
- **P4b**：`CHAMFER`（最便宜）→ `FILLET`（相切弧 + 角点退化；补直线×直线求交辅助）
- **P4c**：`ARRAYRECT` + `ARRAYPOLAR`（独立副本）
- **P4d**：`ALIGN`（两对点 + `AlignScale`）
- **P4e**：`STRETCH`（逐顶点 + 交叉分类）
- **P4f**：`EXPLODE`（一次一层：折线/INSERT/Compound/MINSERT）
- 另：`crates/cad-spatial/src/pick/tests.rs`（991 行）在**开工前**按模块拆分。
- 每个子阶段独立跑门禁；真实 GPU 证据复用既有驱动模式（**先用绘图命令现场造几何**；证据一律标注
  "合成 DOM 指针事件、非真实鼠标"）。

## 明确不做的（NOT IMPLEMENTED，显式记录）

- FILLET：`Polyline` 模式、`Multiple`、命令内 `Undo`、`Trim/No Trim` 开关、平行线自动半径、Circle/Arc 目标、3D、
  hatch 关联性；**圆心公式的 Autodesk 原文**（推论已标注）。
- CHAMFER：`Angle`/`CHAMMODE`、`Polyline` 模式、`Multiple`、`Undo`、`Trim/No Trim`、弧段替换、3D；**负距离行为未查证**。
- ARRAY：通用 `ARRAY` 的类型提示流程、`Path`、`Levels`、`Total`、角度间隔输入、**关联性与 `ARRAYEDIT`/grips**、
  `MAXARRAY` 可配置化、3D、**负间距反向**。
- STRETCH：cpolygon/lasso、**单独选取即整移**、`TEXT`/`SPLINE`/`ARC`/`HATCH`/`INSERT` 拉伸、3D、宽度/切向修改
  （宽度未建模）；**bulge 保留为未查证选择**。
- ALIGN：3 对点 3D、非均匀缩放、**1 对点移动**。
- EXPLODE：多级一次到底、`EXPLMODE`、颜色/线型变化、Dimension、关联阵列、属性语义、xref 语义。
- 全部：**3D/OCS 非共面**、**纸空间**、**DWG 写回**；参数**不持久化**（AutoCAD 存图纸/注册表）。

## 栅栏（不得软化）

- **不变量集合**（替代"字面复用唯一漏斗"的措辞）：新实体写入必须满足**图层存在**、**模型空间**、
  **一次事务 + 一步撤销**；多实体操作沿用既有 N 副本/多 patch 形状，单实体沿用 `commit_new_entity`。
- 不新增外部依赖、不改 acadrust、不加 patch。`cad-geometry` 的 `pub(crate)` 辅助**升 `pub` 保持单一实现**
  （不得声称"在别的 crate 里 re-export `pub(crate)`"）。
- 用户可见文案只进 `crates/cad-ui-core/i18n/{zh-CN,en}.json`；Rust 侧不得新增 CJK 字面量（CJK 基线测试会拒绝）。
- 未支持的几何/选项必须显式 `Unsupported`/`InvalidInput`；**禁止**部分成功、静默丢弃、"看起来像成功"。
- 证据诚实：AutoCAD 未查证项（FILLET 圆心公式、bulge 拉伸、TEXT 拉伸、CHAMFER 数值默认与负距离、`CHAMMODE`/`EXPLMODE`
  官方页缺失、动态块 EXPLODE、负间距反向、极性阵列非旋转放置、hatch 分解产物、新实体图层归属）必须标
  **NOT FOUND/UNVERIFIED/我们的选择**，不得以推断冒充原文。

## 验收

- `cad-app` 契约测试（逐命令正常路径 + 逐条拒绝 + 一次事务/一步撤销）：
  - FILLET：`fillet_picks_choose_the_corner_and_the_arc_centre_lies_on_the_pick_side`；
    `fillet_zero_radius_trims_met_lines_and_extends_unmet_lines_to_the_corner`；
    `chamfer_zero_zero_matches_fillet_zero_radius`（对照）；`fillet_and_chamfer_new_entity_follows_the_declared_layer_rule`；
    平行/同一实体两次/跨文档/段过短/非直线段的显式拒绝。
  - CHAMFER：`chamfer_distances_apply_in_pick_order`；负距离拒绝。
  - ARRAY：`array_count_at_cap_is_accepted_and_cap_plus_one_is_refused`；间距=基点距离；行列数；负间距拒绝；
    `array_polar_without_item_rotation_follows_the_declared_placement_rule`。
  - STRETCH：`stretch_window_crossing_a_segment_with_no_endpoint_inside_changes_nothing_with_diagnostic`；
    `stretch_fully_enclosed_line_equals_move_result`；部分包围=逐顶点；圆/块部分包围不动且有诊断；bulge 保留；
    文本整命令拒绝。
  - ALIGN：两对点移动+旋转；`align_confirm_at_three_points_is_refused`（arity 规则）；`AlignScale=true` 的均匀缩放
    与默认 false 的差异；退化点对拒绝。
  - EXPLODE：`explode_minsert_compound_yields_insert_instances_then_members_on_the_next_explode`；
    `explode_compound_keeps_every_child_including_hatch_fill`（无静默丢失）；成员保留自身图层；
    不可变换子几何 ⇒ 整命令失败；非均匀 INSERT 拒绝。
- `cad-ui-core`：7 个选择子恰好一次、与既有词汇表**不相交**、路由正确；`ToolParameter`/`ToolValue` 线格式契约测试；
  `set_tool_parameter_replaces_polygon_sides_on_one_wire_path`（迁移：旧请求消失、4.5 拒绝）；命令计数更新。
- React：7 个入口由路由器准入；数值/开关控件仅对活动工具出现、可用态由 `tool.key` 派生。
- 导出：**FILLET 的 ARC、CHAMFER 的 LINE** 以及阵列/分解产物经**生产 save 路径**逐类断言。
- 真实 GPU 证据：先用绘图命令现场造相交直线，跑 FILLET 或 CHAMFER 端到端 + 截图判读；输入标注非真实鼠标。
- 能力表（§3.2）：七行逐列更新，并显式记录全部**有意偏离/未查证项**（非关联阵列、参数不持久化、FILLET 圆心推论、
  新实体图层归属、极性非旋转放置、负间距、hatch 分解 ≠ AutoCAD、宽度未建模、CHAMFER 负距离）。
- 全量门禁（`fmt`/`clippy --workspace`/`check`(workspace+all-targets)/wasm/架构/i18n/fixture/文件行数/workflow + UI）。
