# ADR 0015：新绘图命令（PLINE / RECTANG / POLYGON / ELLIPSE / POINT / SPLINE）

- 状态：**已接受（Accepted，2026-10-19）**。设计门禁 **SG-P3 attempt 1 = APPROVE-WITH-FIXES**（B1–B4 已按结论
  写入修订）、**attempt 2 = ACCEPT**。转正时随状态行一并执行了复核指出的**文档勘误**：决策 B 的 i18n 表述改为
  「状态/过程文案不新增 CJK；`label()` 沿用**已记录的既有回退偏差**」，钉住测试改述为「cad-app 除**已登记基线**外
  不新增 CJK 字面量」（原表述与已落地的 `label()` 不符）。实施进度另起小节追加，不改写决策文本。
- 范围：`crates/cad-app`（命令/工具状态机/几何构造）、`crates/cad-ui-core`（选择子、状态投影、i18n）、
  React 宿主 UI、**POINT 的渲染表示**（overlay 标记）、以及 `cad-geometry`（SPLINE 插值纯函数）。
  **不**覆盖：弧段 polyline 造形、椭圆弧造形、样条 CV 法/切向/公差选项、`PELLIPSE`、参数持久化、3D/OCS 造形。
- 相关：`CAD_IMPLEMENTATION_SPEC.md` §3.2（基础图形支持范围，**含能力表要求**）、`docs/interaction.md`、
  `docs/adr/0008-sdf-conic-rendering.md`、`docs/compatibility.md`、`docs/handoff.md`、
  `.slim/deepwork/autocad-parity/progress.md`（P3）。

## 实施状态（追加，不改写决策文本）

- **P3a 完成（2026-10-19）**：变长捕获（`PointArity::{Exact,AtLeast}`、`remaining() -> Option<usize>`、
  `MAX_OPEN_CAPTURE_POINTS=1024`）+ `PLINE` + `RECTANG` + `ELLIPSE` 全部端到端落地；选择子 50 个。
  - `PLINE`：开放 LWPOLYLINE；`RECTANG`：闭合 4 顶点、**规范化逆时针左下起**（两种对角点击同一实体）；
    `ELLIPSE`：轴端点法、`ratio ≤ 1` 由创建路径保证、**长短轴交换已实现并标注为未查证假设**、预览 16–128 段。
  - 三者的导出均经**生产 save 路径**断言（`RECTANG` 闭合组 70=1；`ELLIPSE` 组 10/11/40/41/42 且重复导出稳定）。
  - 按 1000 行门禁拆出 `draw_tool/geometry.rs`、`host/draw_create.rs`、`overlay/tests/ellipse_preview.rs`。
- **P3d 数学前置完成**：`cad-geometry/src/spline.rs`（`fit_points_to_cubic_spline`、`chord_parameters`、
  `cubic_spline_control_point_count`）= 三次/clamped/chord/**插值**、`knots.len()==n+4`、≥4 fit 点、
  重复与 1e-9 级近重合**显式拒绝**、10 条数值测试（含 chord≠uniform 判别、C2、共线、簇不误拒）。
- **待办**：P3b（`POLYGON` + `SetPolygonSides`）、P3c（`POINT` 命令；渲染标记进行中）、P3d（`SPLINE` 命令）、
  以及 ADR 验收里点名的 `cad_app_adds_no_new_cjk_literals_outside_the_recorded_baseline`（尚未建立基线）。

## 背景（勘察事实，2026-10-19；已按 SG-P3 核对更正）

1. **六种实体全部可存储**：`SemanticGeometry` 有 `Polyline{points,bulges,closed}`、`Ellipse{center,normal,
   major_axis,ratio,start,sweep}`、`Point(Point3)`、`Spline{degree,knots,control_points,weights}`
   （`cad-domain/src/lib.rs:297-333`）。注意 **`Spline` 没有 fit 点字段** ⇒ fit 点永不参与往返（见决策 F）。
   `cad-db/src/validate.rs:45-243` 覆盖全部变体，但有两条本 ADR 依赖却不该假设的约束：
   - `Spline` 要求 `control_points.len() >= degree + 1`（`:145-152`，提交路径同样强制）⇒ 三次插值必须有 ≥4 个控制点（见 B1 修正后的决策 F）；
   - `Ellipse.ratio` 只要求 **> 0**（`:125-127`），**没有** `≤ 1` 的校验 ⇒ `ratio ≤ 1` 是**创建路径 + DXF 约定**的
     不变量，不是 DB 不变量（见决策 D）。
2. **导出是逐类映射、无静默丢弃，但并非全部"精确"**：LWPOLYLINE（平面的）、ELLIPSE（`ratio` 原样写组 40、
   `end = start + sweep`）、POINT 为 Exact；**SPLINE 记为 `MappingOutcome::Converted`**，原因是"closed/periodic
   标志未携带"（`cad-import-acadrust/src/export/entity.rs:331-391, 361-377`）。丢弃的是 Shape/Image/Mask/Opaque。
3. **`Point` 没有专用渲染路径**：`cad-geometry/src/tessellate.rs:102` 把点退化成零长线段，默认 provider 走
   chord 回退（`cad-representation/src/provider/default.rs:681-688`）；`DisplayPrimitive` 无点图元
   （`primitive.rs:160-193`）。**今天 `docs/compatibility.md:28` 声称 POINT 绘制"实现"是过度声明**，本 ADR 要求更正。
4. **工具状态机原本是定长、纯点**：`required_points()->usize`、超量即拒、`is_ready = remaining()==0`、
   `intent()` 在 `remaining()>0` 时报错 ⇒ 需引入显式元数模型（决策 B；`PLINE` 已按此落地）。
5. **缺的是"UI→会话的数值入口"，不是"带类型参数"本身**：内部 `CommandPayload` 一直是带类型的，且
   `InteractionRequest::SetMeasurementKind`（`interaction.rs:141-142`）已是**非文本参数请求**的先例。P0 拒绝的是
   **命令行文本 payload 解析**；因此 `SetPolygonSides` 不违反该决定（决策 C 据此收窄）。
6. **插入漏斗唯一且仅限模型空间**：`commit_new_entity`（`app_drawing/trim_extend.rs:383-398` → `:406-484`）
   负责图层校验（缺层 `:424-429`）、`draw_order`、`EntityId`、**一次事务 + 一步撤销**、`publish_candidate`；
   固定 `SpaceId::Model`。新命令必须复用，不得自建插入路径。
7. Enter→`CONFIRM` 的**路由已存在**（`ENTER` 别名 `commands.rs:143` → `InteractionRequest::Confirm` → `confirm_tool`
   → `confirm_draw_traced`），但**从未对开放工具验证过**；且绘制捕获不会自动确认（`capture_point` 对 `Drawing`
   返回 `Ok(false)`，自动确认仅测量）⇒ 决策 B 的验收必须补这条测试。
8. 六个选择子此前均未绑定；`PLINE` 名已被预留（P2 刻意不用于测量），无历史包袱。

## 决策 A：交互形状与默认值（对齐 AutoCAD 文档）

| 命令 | 本阶段实现 | 与 AutoCAD 默认值的关系 |
|---|---|---|
| `PLINE` | 起点 + 若干后续点；**Enter 结束**；存为**开放** LWPOLYLINE（每顶点 bulge=0） | 与"点击即直线段、Enter 结束"一致；`PLINEWID` 初值 0 ⇒ 宽度 0 |
| `RECTANG` | **两个对角点** ⇒ 闭合 LWPOLYLINE（4 顶点，bulge=0） | 与默认"角点法"一致；`Elevation`/`Thickness`/`Width` 默认 0 |
| `POLYGON` | 边数（默认 **4**，见决策 C）+ 中心 + 半径 ⇒ 闭合 LWPOLYLINE，**内接** | 边数默认 = `POLYSIDES` 初值 4；默认 **内接**（非外切） |
| `ELLIPSE` | **轴端点法 3 点**（两轴端点 + 到另一轴的距离）⇒ 单个 `Ellipse`，`start=0`、`sweep=2π` | 与默认"轴端点法"一致（**不是**中心法，中心法需显式 `C`，本阶段未实现）；长短轴规则**见决策 D（含未查证假设）** |
| `POINT` | **单次放置**（一次点击一个点） | AutoCAD 的 `POINT` 页面只记载 `Specify a point` 单次放置，"是否重复直到 Enter"**未查证** ⇒ **不按猜测实现重复**；Multiple Points 为开放项 |
| `SPLINE` | fit 点法：**≥4** fit 点 + Enter（见决策 F/B1）；三次 + chord 节点插值成 NURBS 存储 | 与默认 `SPLMETHOD`=0（fit）、`SPLDEGREE`=3、`SPLKNOTS`=0（chord）一致；**小样本（2–3 点）行为未查证**，本阶段显式拒绝 |

出处（Autodesk Help 2026/2027，逐条见 librarian 简报）：[PLINE](https://help.autodesk.com/cloudhelp/2026/ENU/AutoCAD-Core/files/GUID-11883C70-6435-4F80-8FB4-F6E933B8FD94.htm)、
[RECTANG](https://help.autodesk.com/cloudhelp/2026/ENU/AutoCAD-Core/files/GUID-188B2DDA-6CD8-4D37-BF26-E6CF27C34C75.htm)、
[POLYGON](https://help.autodesk.com/cloudhelp/2026/ENU/AutoCAD-Core/files/GUID-E5CD464D-C0DC-4464-BFDF-50C4ABEC8B91.htm)、
[POLYSIDES](https://help.autodesk.com/view/ACD/2027/ENU/?caas=caas%2Fdocumentation%2FCIV3D%2F2014%2FENU%2FfilesACD%2FGUID-D848B99F-62E9-4605-BA87-18606BE36F0C-htm.html)、
[ELLIPSE](https://help.autodesk.com/cloudhelp/2026/ENU/AutoCAD-Core/files/GUID-45F9C588-2BA9-414D-8AFD-FB6B448BF273.htm)、
[POINT](https://help.autodesk.com/cloudhelp/2026/ENU/AutoCAD-Core/files/GUID-3F5861A1-9A63-42A6-8F12-3395771BAA6D.htm)、
[PDMODE](https://help.autodesk.com/cloudhelp/2027/ENG/AutoCAD-Core/files/GUID-82F9BB52-D026-4D6A-ABA6-BF29641F459B.htm)/[PDSIZE](https://help.autodesk.com/cloudhelp/2027/ENU/AutoCAD-Core/files/GUID-826CA91D-704B-400B-B784-7FCC9619AFB9.htm)、
[SPLINE](https://help.autodesk.com/cloudhelp/2026/ENU/AutoCAD-Core/files/GUID-5E7D51E2-1595-4E0C-85F8-2D7CBD166A08.htm)/[SPLMETHOD](https://help.autodesk.com/view/ACD/2026/ENU/?caas=caas%2Fdocumentation%2FCIV3D%2F2014%2FENU%2FfilesACD%2FGUID-300EB1EF-E101-421C-A8B8-6D91E2873787-htm.html)。

**未查证项（明确记录，不猜）**：Esc"什么都不提交"的语义（六命令均未查到 Autodesk 明文）；零长/单点结果是否被拒绝；
ELLIPSE 当第一轴实为**短轴**时 AutoCAD 是否静默交换；SPLINE 是否显示实时橡皮筋；`POINT` 是否重复放置；
**SPLINE 小样本（2–3 fit 点）行为**。

## 决策 B：变长捕获（机制）

- `DrawToolKind` 的元数用**显式模型** `PointArity::{Exact(n), AtLeast(n)}`（`PLINE` 已按此落地），取代裸 `required_points()`。
- 开放工具接受多点直到**硬上限 `MAX_OPEN_CAPTURE_POINTS = 1024`**（≈24 KiB 的 `Point3`（1024 × 24 B 字节）；
  远大于手工描图规模，又给"指针卡住"一个明确边界），超限**显式拒绝**而非无界增长。
- 就绪 = 达到最小点数（`AtLeast(n)`）⇒ 复用既有 Enter→`CONFIRM` 通路（路由已存在，但**必须补测**开放工具
  的"不自动确认 + Enter 在最小值处确认"，见验收）。
- 定长工具（现有全部）行为**与今天逐字节一致**，用测试钉住；`DrawToolKind::ALL` ↔ `from_index`/`from_label`
  的往返契约（`draw_tool/tests.rs`）在扩展 `ALL` 时必须保持。
- **诚实的状态呈现**：`remaining()` 为 `Option<usize>`（`None` = 开放且已达最小，**不得**谎报 0）；
  `DrawPreview` 携带元数/开放性（`arity`/`is_open_ended`），并且**必须投影到前端线格式**
  （`CadTool` 增 `openEnded`，空值时规则与既有字段一致），否则 UI 只能显示"还需 0 点"这种错话。
  注意 `CadTool{key,points,ready}` 是**冻结的前端契约**：扩展它需要 `cad-ui-core` 契约测试。
- **i18n 立场**：`status_line()` 现有的硬编码中文是**既有偏差**，本次**不新增** `status_line()`/过程类 CJK 文案
  （`PLINE` 的开放分支已是 locale-neutral ASCII）；新工具的用户可见文案由 React 层用 i18n 键组合
  （`tool.draw.*`/`draw.kind.*`）。`label()` **沿用已记录的既有回退偏差**（P3a 已新增 `多段线`/`矩形` 字面量；
  shell 用目录文案替换，`label()` 不是 chrome 文案源）⇒ 钉住测试的表述是
  **「cad-app 除已登记基线外不新增 CJK 字面量」**，而不是"不新增 CJK"。
- **撤销单点（PLINE 的 `Undo`）不在本阶段**：命令行不是文本 payload 解析器；若要做属独立决定。
- 与上一阶段"捕获候选着色"的相互作用：捕获候选只对**需要拾取实体**的种类解析（`move_tool_cursor`），
  自由点开放工具永不绘制它 ⇒ 结构上无冲突，仅保留一行回归断言。

## 决策 C：工具参数通道（POLYGON 边数）

AutoCAD 用提示行输入数字；本项目没有数值输入通道，且命令行**刻意不做文本 payload 解析**。因此：

- 会话级参数 `polygon_sides`（默认 **4**，对齐 `POLYSIDES` 初值），**不**写持久化 config。
- 通过**收窄的** typed 请求设置：`InteractionRequest::SetPolygonSides { sides: u32 }`（**不**做通用
  `SetToolParameter` 总线——单一消费者用泛型总线是 YAGNI；`SetMeasurementKind` 已是同形先例，且不违反 P0 决定）。
- **范围 3..=1024**（对齐 `POLYSIDES` 合法区间）；越界**显式拒绝**（不静默 clamp）。
- **生命周期**（必须测试）：会话作用域；跨**切工具**与**取消**保持（sysvar 语义）；**文档更换不重置**；
  会话重建/新建文档时回到 4；**Viewer 模式拒绝**（创建命令本就是 Work-only）。
- 投影：`CadTool` 增 `params.sides`；**规则是"活动工具不是 POLYGON 时 `sides` 为 `null`"**（无工具时整个 `tool`
  为 `null`），并用 `cad-ui-core` 契约测试钉住线格式；UI 步进控件的可用性由 `tool.key` 派生（不是本地状态）。

## 决策 D：ELLIPSE 长短轴规则（含显式假设）

- 存储 `Ellipse{center, normal:+Z, major_axis, ratio, start:0, sweep:2π}`；`ratio = 短轴/长轴`。
  **注意 `ratio ≤ 1` 不是 DB 不变量**（见背景 #1），必须由创建路径保证，否则会写出 DXF 组 40 > 1 的
  **规范非法输出**（导出按原样写）。
- 用户两次点击给出第一轴（端点 A、B）：中心 = 中点，第一轴半长 = |AB|/2；第三次点击给出垂直于第一轴的
  半轴长度（点到第一轴直线的垂直距离）。
- **重排规则（假设，未查证）**：若第一轴**短于**第三点给出的半轴，把**较长者存为 `major_axis`**（交换两轴角色）。
  **风险已被高估**：对**整椭圆**（`start=0, sweep=2π`）而言交换两轴是**保形**的——点集完全相同，只有存储的
  11 向量与参数原点不同。因此未查证的部分**仅是 AutoCAD 存储参数化的表示方式**，不是可见结果。备选更差：
  按原样存会让 `ratio > 1`（写出非法 DXF）；直接拒绝短轴开局则是惩罚一个完全自然的操作且无任何保真收益。
- 第三点落在第一轴直线上（退化，`ratio = 0`）显式拒绝。
- **能力表必须记录"长短轴重排"为未对齐项**（见验收 B2）。

## 决策 E：POINT 的渲染（必须显式处理）

- **基础场景跳过 POINT**：加显式的 `SemanticGeometry::Point =>` 分支（不再落进通用 chord 回退、不再画零长线段），
  并在完整性/诊断上如实说明"点由 overlay 标记表示"。
- **可视标记走 overlay 通路**（与 snap 提示/预览同一管线）：按 `viewport_bounds` 计算**屏占比大小**
  （对齐 `PDSIZE=0` 的屏幕相关语义），而不是固定世界尺寸——固定世界尺寸在极端缩放下失真。该通路已验证可用：
  `viewport_bounds` 参与 overlay 指纹并在相机变化时刷新。
- **标记必须覆盖导入的 POINT**，不只是本阶段新建的。
- **上限语义（明确）**：超过上限时**绘制前 N 个并给出且仅给出一条显式诊断**（不静默截断）；在 N 与 N+1 处各测一次。
  上限取 **4000**（不用 `SELECTIONPREVIEWLIMIT` 背书——该变量是小规模选择预览上限，量级不匹配）；理由：
  overlay 在**每次导航步骤**都会重建，标记几何是每帧重算的屏幕相关代价，必须与既有性能基线同级有界
  （加一条有界成本断言；如超预算再下调上限，下调属实现细节不另改 ADR）。
- `PDMODE`/`PDSIZE`/点样式**NOT IMPLEMENTED**：不实现 0–4 图形位码与围绕点画形状；标记形状固定（十字/点），
  并标注为**有意偏离**。
- **3D 视图：NOT IMPLEMENTED**——`viewport_bounds` 在 3D 为 `None`，屏占比尺寸没有定义域，因此 3D 下不画
  点标记（显式记录，不猜一个 fallback）。
- **已知偏差**：标记在 overlay 顺序里绘制（始终在最上层），与基础场景实体的遮挡关系不同。
- POINT 的拾取/测量/导出**不受影响**（CPU 侧与 DXF 导出均为既有能力）。

## 决策 F：SPLINE 造形

- 几何：`cad-geometry` 先落**纯函数**（fit 点 → 控制点），再接命令；匹配既有 NURBS 求值辅助
  （`tessellate.rs:322-349`）。
- 默认 fit 点法、**三次（degree 3）**、**chord 节点**、公差 0（插值）；节点向量取**clamped** 且
  `knots.len() == n + degree + 1` ——这是导入侧期望的 `Complete` 形式（`cad-import-acadrust/src/geometry.rs:243-255`），
  否则 SAVE→OPEN 不会干净往返。
- **B1 修正（阻塞项）**：`cad-db` 要求 `control_points.len() >= degree + 1`，三次插值下必须有 **≥4** 个控制点。
  本 ADR 选择：**fit 点少于 4 个时显式拒绝**（消息说明三次样条至少需要 4 个 fit 点）。理由：AutoCAD 的
  小样本行为本身**未查证**；不改 `validate.rs` 语义去迁就一个未验证的输入。
- **端条件**（clamped / natural）属**实现级选择**，不是 AutoCAD 保真问题（其端条件未查证）；实现时选定并写明。
- **数值边界**：近重合 fit 点导致的病态条件必须在验收里有数值测试（不只是"能跑通"）。
- 存储为 `Spline{degree:3, knots, control_points, weights:[1;n]}`（非有理）；**fit 点数据不参与往返**
  （domain 无该字段），导出只带控制点/节点/权重 ⇒ 必须在 NOT IMPLEMENTED 中声明。
- `Method(CV)`/`Knots`/`Degree`/`Start/End Tangency`/`Tolerance`/`Close`/`Object`：**NOT IMPLEMENTED**。

## 分阶段（依赖顺序）

- **P3a**：变长捕获机制 + `PLINE`（**已完成**）+ `RECTANG`（**进行中**）+ `ELLIPSE`
- **P3b**：参数通道 + `POLYGON`（含 UI 步进控件；designer 参与）
- **P3c**：`POINT` 渲染标记（overlay + 上限诊断 + 基础场景跳过）与 `POINT` 命令
- **P3d**：`SPLINE`（`cad-geometry` 插值纯函数 + 数值测试先行）
- 每个子阶段独立跑门禁；真实 GPU 证据复用既有驱动模式（新增驱动同样标注"合成 DOM 指针事件、非真实鼠标"）。

## 明确不做的（NOT IMPLEMENTED，显式记录）

- PLINE 的 `Arc`/`Angle`/`Center`/`Direction`/`Radius`/`Second pt`/`Halfwidth`/`Width`/`Close`/`Length`/`Undo`。
- RECTANG 的 `Chamfer`/`Fillet`/`Thickness`/`Elevation`/`Width`/`Area`/`Dimensions`/`Rotation`（含圆角会话内持久怪癖）。
- POLYGON 的 **`Edge` 分支**、**外切（Circumscribed）模式**、以及"以数值指定半径 ⇒ 底边对齐捕捉旋转角"的取向语义；
  **边数的跨会话持久化**（不写持久化 config）。
- ELLIPSE 的 **`Center`（中心法）**、`Arc`（椭圆弧 start/sweep）、`Rotation`、`Isocircle`、`PELLIPSE` 旧式 polyline 椭圆。
- POINT 的 `PDMODE`/`PDSIZE`/点样式、Multiple Points 重复放置、**3D 视图中的点标记**。
- SPLINE 的 CV 法/节点类型/阶数/切向/公差/`Close`/`Object` 转换，以及 **2–3 个 fit 点的小样本输入**；
  **fit 点数据不参与 SAVE→OPEN 往返**。
- 六者的 **3D/OCS 非共面输入**（沿用"+Z 平面"子集，不静默投影）；**纸空间造形**（插入漏斗固定模型空间）。
- **DWG 写回**（SAVE 仍是有损 DXF；`.dwg` 显式拒绝）。

## 栅栏（不得软化）

- 复用唯一插入漏斗 `commit_new_entity`（一次事务、一步撤销、图层显式校验、模型空间）。
- 不新增 Cargo 依赖、不改 acadrust、不加 patch；`validate.rs` 语义不为迁就输入而修改。
- 用户可见文案只进 `crates/cad-ui-core/i18n/{zh-CN,en}.json`（键集一致）；Rust 侧不得新增 CJK 字面量。
- 未支持的选项/分支必须显式 `Unsupported`/拒绝，**不得**用近似静默顶替（SPLINE、ELLIPSE 弧、POLYGON 外切尤其）。
- 不把"能建"当作"能显示"：POINT 在标记落地前后必须分别如实标注。
- 冻结的前端线格式（`CadTool`）扩展必须带 `cad-ui-core` 契约测试，空值规则与既有字段一致。

## 验收

- **能力表（§3.2 要求，阻塞项 B2）**：`docs/compatibility.md` 按命令/实体更新 读取/语义还原/绘制/拾取/测量 五列；
  **更正 `docs/compatibility.md:28` 现有的 POINT 绘制过度声明**；把"椭圆长短轴重排"记为**未对齐**；
  SPLINE 导出记为 `Converted`（几何保留、closed/periodic 丢失）。
- `cad-app` 契约测试：每命令几何构造（顶点数、闭合标志、bulge、`ratio ≤ 1`）、退化/非有限拒绝、
  一次事务 + 一步撤销、缺层拒绝、**Viewer 模式对六个选择子一律拒绝**；变长工具的元数/上限/就绪；
  固定工具行为不变；`ellipse_shorter_first_axis_swaps_axes_and_ratio_stays_le_one`；
  `exported_ellipse_round_trips_idempotently`；`spline_requires_at_least_four_fit_points_for_degree_three`；
  `spline_knots_are_clamped_chord_and_match_n_plus_degree_plus_one`（含近重合 fit 点的数值用例）；
  `open_arity_tool_never_auto_confirms_and_enter_confirms_at_minimum`；
  `open_arity_tool_refuses_point_beyond_cap_with_explicit_error`；
  `polygon_sides_defaults_to_four_survives_switch_and_cancel_and_refuses_out_of_range`
  （该用例须同时覆盖**文档更换不重置**、**新建文档回到 4**、以及 **Viewer 模式拒绝**）；
  `cad_app_adds_no_new_cjk_literals_outside_the_recorded_baseline`（钉住决策 B 的 i18n 基线表述）。
- `cad-ui-core`：每个选择子恰好一次、与其他词汇表**不相交**、路由正确；`CadTool` 扩展（`openEnded`、`params.sides`）
  的**线格式契约测试**（存在性与 null 规则），命令计数随新增更新。
- React：每个命令入口由路由器准入（不本地放行）；POLYGON 步进控件仅在工具激活时可用且禁用态由 `tool.key` 派生。
- 渲染：`point_markers_scale_with_viewport_and_stop_at_cap_with_diagnostic`（2D 下尺寸 ∝ 视口高、N/N+1 行为、
  3D 不绘制）、基础场景不再为 POINT 产出零长线段、**导入的 POINT 也被标记**。
- 导出：六种实体 SAVE 后逐类往返断言（SPLINE 按 `Converted` 断言，几何一致、closed/periodic 不保留）。
- 真实 GPU 证据：至少一条绘制命令端到端（合成 DOM 指针事件驱动，标注非真实鼠标）+ 截图判读。
- 全量门禁（`fmt`/`clippy --workspace`/`check`(workspace+all-targets)/wasm/`check-architecture.py`/
  `check-i18n.py`/`check-fixture-manifest.py`/`check-file-size.py`/`check-workflows.py` + UI `npm test && npm run build`）。
