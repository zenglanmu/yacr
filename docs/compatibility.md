# 兼容性与能力矩阵

> **历史横幅（2026-10-16，2026-10-23 限定）**：下列横幅原本针对**平台/宿主**段落：旧 UI crate、
> `yacr-linux`、旧脚本命令均已退役。**实体能力矩阵与命令矩阵仍是当前契约**，不要因为横幅而忽略它们。
> 当前架构与构建见 `docs/architecture.md`、`docs/react-linux-app.md`、`docs/windows-app.md`、
> `docs/build.md`、[ADR 0010](adr/0010-react-linux-host.md) 与
> [ADR 0023](adr/0023-dom-on-top-cross-host-composition.md)。平台段落已在 2026-10-23 重写（见「平台」）。

**没有任何已验收的 DWG 版本、实体或平台。** `fixtures/manifest` 现含合成夹具与一个
开源 QCAD `flange` 样本（`Partial`，`fixtures/dxf/qcad-flange/`），因此下表的
「实现」仅表示代码路径存在并有契约测试，**不代表对真实图纸兼容**。构建证据
不等于兼容性（规范 §1、§11.4）。2026-10-01 已对 11 份公开样本执行导入端到端
（AC1014–AC1032，见 `docs/validation.md`），仍未构成验收。

## 应用域（界面 profile，2026-11-24）

阶段 1：文档域写入 `.ymodel`（可选 `application_domain`，minor **13**）；`cad2d` 是唯一完整界面，
BIM/SketchUp/SolidWorks 为**显式占位**（隐藏功能区与命令行，显示说明卡，零域命令）。切换器在右
导航栏；主题按域选择 CSS token（`html[data-domain]`），渲染器画布底色仍只认宿主 `theme` 的
dark/light。有文档时切换写文档（`cad_set_application_domain`，一个 revision），无文档只预览。
阶段 2：AutoCAD 2024 参考 tab/panel 顺序由 `autocad-structure` fixture 锁定；已实现命令的参考
入口已接线（视图→视口/坐标、插入→内容、注释→注释缩放），裸 `DOMAIN` 打开切换器、`DOMAIN <id>`
写文档或预览；无实现的参考面板仍显式 `pending`。每域主题持久化（`ui.theme.<domain>`）与 golden
截图 lane 的 CI **未实现**；golden 基线只在本地软件 Chromium 运行过，不能当 GPU/像素/真机验收。
Windows/macOS/Android/Web 的域切换运行 **NOT VERIFIED**。详见 `docs/application-domains.md` 与
[ADR-0053](adr/0053-application-domains.md)。

## 实体能力（读取 / 语义 / 绘制 / 拾取 / 测量）

2026-10-03 新增 DXF ASCII/二进制导入入口，共用 DWG 语义/数据库转换，只有合成 LINE
与截断拒绝针对性契约，不宣称复杂 DXF 兼容。旧式 SHX 字号修复与网络 DWG 对比见
`docs/ui-dxf-text-fixes.md`；原字体缺失的回退整形保留 Partial。DIMENSION 现对无匿名块的
线性/对齐/半径/直径按定义点+DIMSTYLE 合成显示几何，文字仍需宿主字体，见
`docs/validation-dxf-flange.md`。**2026-10-19（ADR-0017 P5a）**：标注成为**一级域实体**
（`SemanticGeometry::Dimension`）；无块与图片标注都导入为该实体，导出为带内容块的真实 DXF
`DIMENSION`。**2026-10-19（ADR-0017 P5c）**：首批创建命令 `DIMLINEAR`/`DIMALIGNED` 落地。
**2026-10-19（ADR-0017 P5d）**：`DIMRADIUS`/`DIMDIAMETER`（实体拾取圆弧/圆 + 尺寸线位置）与
`DIMCENTER`（**普通 LINE 几何**的中心标记/中心线，**非** DIMENSION 实体）落地。
**2026-10-19（ADR-0017 P5e）**：`DIMANGULAR`（**两线交互形式**；三点形式仅命令/载荷层）与 `DIMORDINATE` 落地；
**U-8（两线角度点语义）已用真实 QCAD 夹具钉住**（顶点 = 两条无限直线的交点、组 16 选扇区），并**修复**
角度标注导出组 42 的单位（此前误写**度**，DXF 语义为**弧度**）。`DIMBASELINE`/`DIMCONTINUE` **已实现（P5f，2026-10-21 解禁）**：**显式拾取基准线性标注**（无会话级状态）、
N 点 + 回车、**一条事务/一步撤销**提交 N 个标注；基线按 `DIMDLI` 逐行外推（法向**背离被测几何**，不可定则拒绝），
连续链式同偏移不堆叠（投影 ≤ 0 拒绝）。**范围声明**：仅线性基准、无 `DIMCONTINUEMODE`、非关联、无文字覆盖、无重复循环。详见下方 DIMENSION 分栏与子表。

| 对象 | 读取 | 语义 | 绘制 | 拾取 | 测量 |
|---|---|---|---|---|---|
| LINE | 实现 | 实现 | 实现(线) | 实现(AABB) | 实现(2D/3D) |
| RAY / XLINE | **实现（有界降级，非无损）** | **部分**：库里**没有无限直线几何** ⇒ 压成有限 `SemanticGeometry::Line`（源类名 `AcDbRay`/`AcDbXline` 留在 `type_key`）。裁到**运行中的模型包围盒**（每边外扩对角线 5%、至少 1.0）⇒ **裁多长取决于导入顺序**；还没有任何几何时用**默认 1000 单位盒**。方向退化 ⇒ `Opaque` + `Partial`；**裁不到盒内** ⇒ `Opaque` + `Partial`；否则 `Line` + **`Partial`**（具名常量 `support::XLINE_SEGMENT_REASON`，无范围时 `XLINE_DEFAULT_BOX_REASON`），并额外记一次 `import.construct_line` | 按有限段绘制（缩放后也不会延伸）；**导出只能写 DXF `LINE`**，报 `Converted`（`export.converted.line`，点名「entity type changes / no longer infinite」），**不是** `Exact` | 按有限段（AABB） | 按有限段（同 `LINE`，但它只是裁剪结果；**有界创建命令 `XLINE`/`RAY`（2026-11-01）**、没有无限表示——真正的无限几何是 ADR 级，见 `docs/autocad-2026-command-coverage.md` 的「缺口清单与规模」） |
| LWPOLYLINE/POLYLINE(bulge) | 实现 | 实现 | 实现(含 bulge) | 实现 | 折线长度实现 |
| CIRCLE / ARC / ELLIPSE | 实现 | 实现(保留 normal/OCS)；非均匀仿射重解为真椭圆 | 实现 | 实现 | 距离实现；面积限共面环 |
| SPLINE | 实现 | 实现(保留 degree/knots/weights，真实 NURBS) | 弦高自适应离散 | 实现 | 解析/采样(与显示 LOD 解耦) |
| POINT | 实现 | 实现 | **overlay 屏占比 POINT 标记**（draw_order 850 000，始终在 overlay 层、与基础场景实体遮挡关系不同）；**已实现 `$PDMODE`/`$PDSIZE` 显示样式**（只影响显示）：基础形状 = `0` 点/`1` 无/`2` `+`/`3` `×`/`4` 竖线；`PDSIZE > 0` 绝对图纸尺寸、`= 0` 视口高 2.5% 半幅（即 5% 全幅）、`< 0` 视口高的百分比；`32`/`64`/`96` 周边圆/方**已实现（2026-10-28）**：在**同一个** POINT 标记 `Lines` 批次里画**固定段数**线段——圆 = **16 段闭合弦环**（弦高 ≈ 半径的 **1.9 %**）、方 = **4 段**，**半径/半边长 = 标记半幅**（该比例是**我们的选择**，**未与 AutoCAD 核对**）；`96` = 两者；`PDMODE 0` 的“点”在 **Lines-only** 拓扑下只能画**与配置尺寸同尺寸的** `+`（近似，一条 `overlay.point-mode-dot` 诊断，**不是**点；刻意**不**缩小——DWG 缺省就是 `0`，缩小会静默改变所有未设样式图纸的点外观）；无法表达的模式 → 按名诊断 `overlay.point-mode-unsupported`、不发明几何；**这些 overlay 近似诊断（`point-mode-dot` 等）只留在 `PreparedScene::highlight_diagnostics`，不再提升为 `cad_status.error`（2026-11-22，ADR 0015 增补；只有 `Completeness::Missing` 如 `highlight.unresolved` 才进宿主错误横幅）**；非有限/超限 `PDSIZE` 回退到 `0` 行为（一条 `overlay.point-size-sanitized` 诊断）；覆盖**顶层**导入 POINT；上限 4000 且另发一条诊断；**3D 不画**（`viewport_bounds` 为 `None`，即使 `PDSIZE > 0` 可定义也不画）；**块内 POINT 不标记**（限制已记录） | 实现 | 部分 |
| SOLID / TRACE / 3DFACE | 实现 | 网格(四边形→三角) | 网格 | 实现 | 不支持 |
| HATCH（导入渲染，2026-11-17） | 实现 | 实现 | 图案 · 实体 · 渐变。**顺时针边界弧按"补角/镜像"读**（`ccw=0` 存的是 `360°−θ`，扫角取存储差值、方向走镜像帧；ADR 0018 §B.7-1 / U-14）；**存储图案定义行是"已求值"行**（角度与比例已烘入，导入反求值、导出再烘，`families()` 组合结果等于文件值；§B.7-2）；**文本岛/文本框（`92` 位 8/128）环不绘制**，但保留在 `loops()` 里参与奇偶与导出（§B.7-3）。边界不闭合（交点残差 > 路径对角 10%）记 `import.hatch_boundary_gap` 并写进该 HATCH 的 `Partial` 理由，几何照读不修正 | 实现（环奇偶） | 不支持 |
| INSERT / 块 | 实现(定义与实例分离) | 实现 | 实例展开块几何(嵌套有界，保留 InstancePath) | 实现 | **导出写出** `BLOCK_RECORD`/`BLOCK`/`INSERT`（嵌套递归、各写一次；无法表达为单条 INSERT 的摆放展开并逐条声明） |
| TEXT / MTEXT | 实现(保留字体名/字高/旋转) | 实现 | 整形为线段：outline(TTF/OTF/WOFF)+SHX(shapes/unifont/bigfont)，缺失字体走回退链。**可变换（P7a）**：平移/旋转/均匀缩放；**镜像与非均匀缩放拒绝**（平面文字无法表达真反射）；捕捉面 `engine.rs` 同规则（跨面一致性有测试）。**P7a 起 `TEXT` 创建实现**：1 点 + 文本（`cad_set_tool_text`，终端语义、1024 字符上限），`AcDbText`，一条事务/一步撤销；**旋转自 2026-10-28 起取会话参数 `textRotation`**（通道度、任意有限、**不 wrap**、默认 0，创建时一次 `.to_radians()`；载荷里没有旋转字段）。**MTEXT 的列/背景填充/行距样式/绘制方向仍不保留**，导入改为 **`Partial` + 四条具名原因**（此前误报 `Complete`）；**编辑现有文本已实现（A+B）**：`TEXTEDIT`（拾取一个 `Text` → **预填当前串** → 终端文本提交 → `update_entity`，**一条事务/一步撤销**，只改字符串，样式/高度/旋转/位置/对齐保留）+ **属性面板可编辑字段**（当前单选，**不暴露实体 id**，宿主自行解析并拒绝）；**整串替换**（无光标级编辑、不解析 MTEXT 标记）；拒绝面：非 `Text`/块内成员/子元素、空、超 1024、非单选、拾取后目标已变。**MLEADER 文本编辑已实现（P8）**：A（`TEXTEDIT`）/B（面板内联）/C（**弹窗**）三条入口；MTEXT 与 MLEADER 文本多行、`\n`↔`\P`；TEXT 单行且**拒绝换行**（AutoCAD 不解释 `\P`）；其余控制码逐字透传；**`MLeaderText.width` 已参与排版（2026-10-21）**：`MultiLeaderGeometry::text_geometry()` → `SemanticGeometry::Text.wrap_width` → 表示层**贪心换行**（拉丁按词、CJK 按字；`\P` 段落各自独立；超宽不可断词独占一行；对齐按**每行**应用；`None`/`0`/负值/非有限 = 不换行，输出位级不变）。`Partial`：无 UAX #14 断行、无 kinsoku、跨 token 字距不测量、实体包围盒仍用未换行估计；**MTEXT 的 41 组（`rectangle_width`）导入侧仍不保留**。**MTEXT 创建（命令核心，2026-10-26）**：`MTEXT` 选择器 + `CommandPayload::MText { position, width, text, height, rotation, h_align, v_align }` 经**同一条创建漏斗**写 **`AcDbMText`**（漏斗新增**type-key 覆盖**，默认仍是 `class_name()` 的 `AcDbText`；覆盖仅允许单实体，批量覆盖按名拒绝）；**两角宽度模型**：`width` 由调用方从第二角导出，`None` = 不换行、原样存 `None`，`Some(w)` 必须有限且 `> 0` 否则**按名拒绝**（不钳制、不改写为 `None`），合法值经 `wrap_width_from_dxf` 唯一规则；换行按**唯一权威** `\n`/`\r`/`\r\n` → `\P`（与 `TEXTEDIT` 共用 `app_drawing::text_policy`），样式/字体与 `TEXT` 同一解析（`resolve_created_text_style`），一条事务/一步撤销，诊断 `drawing.mtext_created` 明列四项未建模面。**交互 MTEXT 工具已实现（2026-10-26）**：`DrawToolKind::MText`（arity `Exact(2)` + `requires_text`），捕获顺序 = **第一个角点 → 对角点 → 输入文字**；插入点 = 第一个角点，`width = \|第二角.x − 第一角.x\|`（要求有限且 `> 0`，**退化矩形按名拒绝 `tool.mtext_zero_rect`**，不伪造宽度），`height` = 既有会话参数 `ToolParameter::TextHeight`（复用 TEXT 的默认 2.5，**不新增高度参数**），**`rotation` 自 2026-10-28 起取 `textRotation`**（度→弧度一次）、**`h_align`/`v_align` 取 `mtextAttachment`**（默认 `1` = 左上；见下）；文字走 `TEXT` **同一条** `cad_set_tool_text` 终端通道（`requires_text` 单一闸门，无第二条捕获路径），多行按共享 `\P` 策略存储。路由：裸 `MTEXT` 选择器 → `BeginDraw(DrawToolKind::MText)`（此前“无载荷、按名拒绝”的合同已废止）；ribbon/命令目录在 TEXT 旁新增 MTEXT（专用 `mtext` 字形，无 variant 弹层）；提示串 `tool.prompt.mtext.{pick_corner,opposite_corner,type_text}`。**创建参数（2026-10-28）**：`textRotation`（`Number`，**度**，任意有限、**不 wrap**、默认 `0`；TEXT 与 MTEXT 都读，创建时**一次** `.to_radians()`；`TEXTEDIT` 不会重转、MLEADER 保持自己的朝向规则）与 `mtextAttachment`（`Number`，闭集 **`1..=9`**，默认 `1`；**只给 MTEXT**：`h = (code−1) % 3`、`row = (code−1)/3`，即 `1..=3` top / `4..=6` middle / `7..=9` bottom × Left/Center/Right，**永不产出 `TextAlignV::Baseline`**）。两者都走 `cad_set_tool_parameter`（**不新增选择子**）。**越界 code（含 0/10/小数/非有限）与非有限旋转按名拒绝**，不钳制、不折到邻居；`0` 度是合法值，所以默认值永不触发提交期拒绝。**显式延期（仍未实现）**：行距、分栏、背景填充、绘制方向、绘制流程内 MTEXT 编辑对话框；`TEXT` 仍是**单行、在插入点**。**行距样式/分栏/背景填充/绘制方向仍不建模**（即导入 `Partial` 的四条具名原因）。**证据上限**：仅 `cargo test -p cad-app -p cad-ui-core` + 前端 `npm test`（契约/纯函数），**无 GUI/GPU/像素/真窗口截图证据**；桌面控件由另一 lane 落地，本行不声称已接线。**TEXT 单行诚实性修复**：含换行的 `TEXT` 草案此前被存成 `AcDbText`（渲染多行、且此后 `TEXTEDIT` 必然拒绝），现于捕获层与命令层同时按名拒绝 `text.single_line_newline` | 同上 | 不支持 |
| `HATCH` / `-HATCH` | 完整实现 | 整实体闭合边界（`Circle`/完整 `Ellipse`/闭合 `Polyline`）+ `LINE`/`ARC` 边链；**`HATCH PATTERN <name>` 与 `HATCH PATTERNFILE <path.pat> [<name>]`**（文件优先→内置回退，无名的文件须恰好一个图案，组 76=2/1）；solid/user/predefined/gradient 四种填充与角度/比例/`HPDOUBLE`/样式；hatch 写入边界所在空间（模型空间走共享漏斗、图纸布局走布局自身 draw-order；块定义 `hatch.block_space`、混合空间 `hatch.mixed_space` 按名拒绝）；空边界 `hatch.selection`、未知/缺名/文件读失败逐条具名拒绝。**全局范围**：创建期关联性（`HatchGeometry` 只存派生环，无边界句柄/反应器）、实时预览与图案选择对话框 |
| HATCH（创建） | **实现（P6b 预选边界；图案类型 `2` 与 LINE/ARC 边界拼接于 2026-10-26 增补；渐变类型 `3` 于 2026-10-27 增补；内部点 `HATCHPOINT` 于 2026-11-01 增补）** | 部分 | **两个入口（2026-11-01）**：`HATCH` **先选边界对象**（与 `ARRAY`/`MOVE` 同型：0 点 arity + `requires_selection`）；`HATCHPOINT` 是**本项目选择子**，拾取**一个内部点**（1 点 arity、不要求选择），在**可见的整实体闭合** `Circle` / 完整 `Ellipse` / 闭合 `Polyline` 中取**最小包含边界**并把**嵌套闭合实体**作为岛屿；**一条事务/一步撤销**，边界实体保留，种子存进 HATCH 的 `seeds`（DXF 组 98）。**HATCHPOINT v1 上限**：**已支持**可见 `LINE`/`ARC` 边链（平面半边面行走，分支/悬挂边由面行走处理，弧存**精确 bulge**）、**不**使用当前选择、只认 `+Z` 同标高、仅模型空间；`SPLINE`/椭圆弧边、块成员边仍不支持；点落在边界上（`hatch.boundary_on_edge`）/找不到包含边界（`hatch.boundary_not_found`）/两个等面积包含边界（`hatch.boundary_ambiguous`）都是**具名拒绝**。接受 `Circle`/完整 `Ellipse`/**闭合** `Polyline`，每个成一环；**`LINE`/`ARC` 边界边**按端点**缝合**成闭合环（容差为**绝对 `1e-6` 世界单位**——本层无图面比例/单位可推相对 eps，超差即真开口，**绝不外插搭桥**；每条边可**任向消费**，混向绘制照样缝合，`ARC` 以 `bulge = tan(sweep/4)` 存为**精确 bulge 段、绝不采样**，`LINE` 为 bulge 0；四分之一弧即 `tan(π/8)`，与 `Circle` 环同一约定；一选择可产出**多环**，两个分离矩形 → 两环）。全部边界须**共面**（圆/椭圆/圆弧另须**法向一致**）。参数：图案类型（`0` 实体 / `1` 用户自定义 / `2` **预定义** = **3 个手工转录**图案 `ANSI31`/`ANSI32`/`ANSI37`，按 **1 基** 索引 `hatchPattern` 选择，闭集 = 表长，越界**按名拒绝**不钳制 / **`3` 渐变**，见下）。**状态栏按名选择（2026-10-27）**：预定义图案现在状态栏以 **`ANSI31`/`ANSI32`/`ANSI37` 三段 chip** 呈现、按**名字**选中，而**线上传与存储的仍是 1 基索引**（`cad_set_tool_parameter` 的 `hatchPattern`，1 = `ANSI31`、2 = `ANSI32`、3 = `ANSI37`）——名字与索引是**同一个**选择，没有第二条写路径；图案类型不是 `2`（预定义）时该行**仍可见但惰性**，并给出原因提示。**渐变创建（2026-10-27，类型码 `3`）**：四个 `Number` 参数（均经 `cad_set_tool_parameter`）——`hatchGradientKind` 闭集 **`0` LINEAR / `1` SPHERICAL**（**从不**写 `CYLINDER`：域会把它当球面烘焙，等于一个行为两个名字）、`hatchGradientAngle`（**通道度**，任意有限值，**一次** `.to_radians()` 转成实体里的 `angle_raw`；导入侧按组 `460` **逐字**存弧度，两侧约定由 create→export→reimport 测试钉住）、`hatchGradientColorA`/`B`（**整数 ACI**，默认 **1 红 / 5 蓝**；通道域 `0..=255`，但 **`0` 是 ByBlock 占位、没有 sRGB，按名拒绝**）。实体写 `HatchGradient::TwoColor { reserved: 0, angle_raw: 弧度, shift: 0.0, stops: [0.0 → A 的 rgb, 1.0 → B 的 rgb] }`，`pattern_type` 仍是 **`Predefined`（`76=1`）**（渐变按「实体填充 + 450–470 块」写、没有定义行；该映射是**未查证的 vendor 约定**，不是往返证据），`HatchStyle` 按所选写。**渐变无视图案的间距/角度/双线**（陈旧的非法间距不得拒绝一个渐变；反之陈旧的非法渐变颜色也不得拒绝图案）。拒绝面：kind 不在 `{0,1}`、ACI 非整数/越界/`0`、角度非有限、图案类型不在 `{0,1,2,3}`。**已声明的三个性质缺口**：`SPHERICAL` 在矩形/圆这类「边界顶点同半径」的形状上**烘焙成平色/椭圆感、不是径向**（按三角化顶点取色，要按值细分内部三角形才行）；颜色**只有 ACI**（无真彩、无名/色簿），stop 会被实体/图层 tint 相乘；**无单色/tint 变体、无逐站命名**。**证据边界**：图案名 chip 与渐变控件都只有**源码结构/纯函数契约**（`apps/desktop-ui` 的 `tsc --noEmit`、`node --test` 692 通过、`vite build`），**无像素/运行时/真窗口证据**，也**没有** vendor 文件对照渐变 DXF 形状；间距（>0；预定义下兼作图案比例 `HPSCALE`）、角度（**通道度、域弧度**）、双线、样式（`Normal`/`Outer`/`Ignore`）。预定义表的数值是**我们的转录值**（ADR U-1 纪律，**非** vendor 核实），经既有 `HatchPattern::families()` 出线，无渲染器改动；`76` 写 **1**。**圆 = 4 段精确圆弧**（`bulge=tan(π/8)`；**非** ADR 原写的两点环——校验器要求 ≥3 顶点，ADR 已更正）；椭圆按 64/圈**采样**（真椭圆非圆弧，bulge 会是假几何）。**~~内部点求边界~~ 已实现（2026-11-01，`HATCHPOINT`，v1 上限见上）；自交环（拒绝，不修剪/不修复）、AutoCAD 式模态图案选择对话框、实时预览、关联性、每段裁剪的虚线相位重置均仍 NOT IMPLEMENTED（关联性/预览/对话框属全局范围；`76=2` 自定义图案名与 `.pat` 加载已由 `HATCH PATTERN`/`PATTERNFILE` 提供）**。拒绝面：开放折线、样条、椭圆弧、退化 `LINE`/`ARC`（`hatch.boundary_degenerate`）、分支节点（`hatch.boundary_branch`，命名节点坐标与边数，**不猜配对**）、未消费的开放链（`hatch.boundary_open`，命名未消费边数）、自交、相切/重复、部分重叠、退化、非共面、非法参数（含预定义索引越界）、无活动图层、Viewer。**创建校验基于弦**：两段 bulge 环的**弦间**相切可能漏检（声明） | — | 不支持 |
| MULTILEADER | 实现（P7b-1 起为一级 `MultiLeader` 实体） | 部分 | 引线（直/样条）+ 箭头三角 + 狗腿 + 文字；**仅 MText 与无内容**迁入变体，**块/公差内容仍为 `Compound`**（渲染不变）；`90` 掩码原样、`reserved` 仅存逐字源数据。**声明**：样式快照是**解析后的有效值**（原始未置位字段不单独保留）；`EXPLODE`/`STRETCH` **显式整命令拒绝**；缩放/镜像/非均匀**整选择拒绝**；导入恒 `Partial`（颜色/线宽覆盖、背景、分栏、打断、自定义箭头、非默认平面）。**导出已实现（P7b-2）**：真 `MULTILEADER` + `MLEADERSTYLE` 按名去重 + `CLASSES`（由 acadrust 默认注册保证，U-5 已勘误）；**`90` 掩码逐字**；**不写**计算后的文字排版。**声明**：`91/341/171` 写默认；样式为**有效值快照**（掩码抑制字段非逐字往返）；未建模上下文取默认、`text_normal/direction` 派生；seeds 未建模（写读不对称）；per-line 箭头/线型 handle **悬空**；`271–273` 样式级 acadrust 无法表达；**解析但未建模的组**（实体尾 `292/293/294/174/178/295`、上下文 `44/293/295`）**归一化为默认值**并作**常设具名偏差**；**实体级 `342`（自定义箭头块）在导入侧具名**（含与逐线 `341` 不一致的情形）；尾部 `271` 由 root 0 派生。**不宣称"零丢失"**。**块内放置**：刚性 ⇒ 真实体；缩放/镜像 ⇒ **声明式摊平**（不丢内容）。**创建已实现（P7c）**：`MLEADER` = **箭头 → 落点 → 文本**，单簇/单线/2 顶点/仅直线，`Exact(2)` + 文本（**无内容不创建**）；狗腿/缺口/箭头/字高/附着取 `MLeaderStyle` 表（`Standard`，**常量为债务**）；`MLEADERSTYLE` 导入**现已喂表**。**无 `Options` 子选项**（引线类型/内容类型/最多点数/角度约束不存在，UI 不暗示）。**真机像素判据已执行（2026-10-20，有界）**：X11/XWayland + Quadro P620(Vulkan) + `scale=1` ⇒ 箭头/斜引线/狗腿/文字四要素齐全，逐边 ≤1 px，零碰撞；同轮选择行钉住 `AcDbMultiLeader`。**未在 AutoCAD 打开验证**；无真实 MULTILEADER 夹具 | 实现（引线/箭头/文字任一命中即选中整个实体） | 不支持 |
| **bulge 弧显示（P6b 跨实体修复）** | 实现 | — | **渲染变化（正确性修复）**：`append_bulge_arc` 曾把弧心放在**弦顶侧** ⇒ 每条 bulge 弧被沿弦**镜像**（R=10、`bulge=tan(π/8)` 实测画到 **R=21.95** 再直线跳回）；**半圆不变**（`cos(θ/2)=0`）⇒ 既有 bulge 测试**全过**（只断言端点/连续/半圆，**对符号不判别**）。同型错误也在 **EXPLODE 弧恢复 / 拾取包围盒 / 对象捕捉**，四处同修。**影响所有 bulge 折线，含导入图纸**（法兰 6 处位移达弧半径 1.3%–397%、`entities` 3 处其一 251.6° 弧位移 156.27 单位；既有夹具测试只断言"输出非空"，故未抓到）。另修"平面法向符号取自 Newell 绕向"⇒ **XY 平行折线一律 `+Z`**（否则顺时针 2D 环的弧被镜像，且折线填充边界可被存成 `-Z` 平面）。**残余**：倾斜折线仍取绕向符号（域内不存挤出法向）；导入 `ocs_to_wcs` 快速路径对 `(0,0,-1)` 未改 | — | — |
| **OCS `-Z` 挤出（导入，2026-10-20 修复）** | 实现 | 实现 | **正确性修复**：`-Z` 挤出的 OCS 是 **x 镜像**，此前 `polyline_ocs_points` 的快速路径把任何 Z 平行法向都当 `+Z` ⇒ 折线顶点、**bulge 方向**、TEXT/ATTDEF/ATTRIB 的**位置与旋转**、ELLIPSE 的**法向**都未镜像。现 `+Z` 走恒等、`-Z` 走镜像：点 `(-x, y, -elevation)`、**bulge 取反**、文字旋转 `π − θ`、椭圆**保留 `-Z`**（长短轴与扫掠方向随之正确）。**CIRCLE/ARC/INSERT 经审计本就正确**（`arbitrary_axis` 的 −Z 帧），**未双重纠正**；MTEXT 按 DXF 本就是 WCS。**残余**：**倾斜**挤出的 TEXT/ATTRIB 仍按原始 OCS 放置（域内 `Text` 无挤出法向，需域变更）；且文字**字形不镜像**（平面文字域无法表达真反射） | — | — |
| DIMENSION（读取/显示） | 实现 | 合成 | 无匿名块：按定义点+DIMSTYLE 合成线/箭头/测量文字；有匿名块（`*D…`）：展开块成员。**P5a 起无块与图片标注都导入为一级 `Dimension` 实体**（显示几何保留）。**线性/对齐/半径/直径/角度(2Ln/3Pt)/坐标的合成为 `Complete`**（与代码一致；未查证项见 ADR-0017 决策 E）；**圆弧长/大半径仍 `Partial`**（不在 ADR-0017 kind 集合内，保留旧 `Compound` 合成） | 经合成几何(AABB) | 不支持 |
| DIMENSION（创建） | `DIMLINEAR`/`DIMALIGNED` **实现（P5c）**；`DIMRADIUS`/`DIMDIAMETER` **实现（P5d）**；`DIMANGULAR`/`DIMORDINATE` **实现（P5e）**；`DIMBASELINE`/`DIMCONTINUE` **实现（P5f，2026-10-21 解禁）** | — | **P5c–P5e 实现创建命令**：`DIMLINEAR`/`DIMALIGNED` 为**三点流程**（两尺寸界线原点 + 尺寸线位置）；`DIMRADIUS`/`DIMDIAMETER` 为**实体拾取圆弧/圆 + 尺寸线位置**；`DIMANGULAR` 为**两线拾取 + 标注弧位置**，或**三点（顶点 + 两侧点）+ 标注弧位置**；`DIMORDINATE` 为**特征点 + 引线端点**；`DIMCENTER` **不是** DIMENSION 实体（见绘图辅助行）。`DIMBASELINE`/`DIMCONTINUE` **已实现**：**显式拾取一个基准线性标注**（命令**不持有任何跨命令状态**，因此撤销/删除/切换文档后不存在悬挂引用）、N 点 + 回车、**一条事务/一步撤销**提交 N 个标注；基线按 `DIMDLI` 逐行外推（法向**背离被测几何**，不可定则拒绝），连续链式同偏移**不堆叠**（投影 ≤ 0 拒绝）。**范围**：**仅线性基准**、**不写 `DIMCONTINUEMODE`**、**非关联**、无文字覆盖、无「重复到 Esc」循环（详见下方 DIMENSION 族逐行表）。**不得因为有了实体类型就暗示其余标注种类能创建**（圆弧长/大半径仍是读取侧 `Partial`）；**创建无 spec 授权**（§3.2 只要求"常用 DIMENSION 可见表示"） | — | 不支持 |
| MESH / PolyfaceMesh | 部分 | 网格契约 | 网格 | 实现 | 不支持 |
| RASTERIMAGE | 实现(相对路径解析 + PNG/JPEG 解码) | 实现(`SemanticGeometry::Image`) | 实现(GPU 纹理：每键去重/UV 方向/裁剪多边形；缺纹理 frame+`Partial`) | 经边框 | 不支持 |
| WIPEOUT | 实现(`Mask` 边界 + `inverted`) | 部分 | **非 `inverted`：真遮罩**——无颜色 `Mask` 图元，渲染器**末 pass**（`depth_compare: Always`、不写深度、**clear 颜色**），**凹形用 even-odd 填充**；**`inverted` 亦已渲染**（视口覆盖环 + 边界做 even-odd 补集；正交视口环**精确**、透视下为**宽松估计**）；矢量出图**按页面背景 flood**（`inverted` 为多环 even-odd 补集）；不与更高 `draw_order` 交错（遮罩恒最后） | 经边界 | 不支持 |
| EXTENDED(RTEXT/ARCALIGNEDTEXT/GEOPOSITIONMARKER/SECTIONOBJECT/POINTCLOUD) | 实现(基础字段) | 部分 | 基础表示(Text/圆/折线/范围盒) | 部分 | 不支持 |
| **AEC Arch（AEC_WALL/DOOR/WINDOW/STAIR/SPACE/…，2026-11-03）** | **实现（acadrust 0.6.8 已类型化）**：`ExtendedEntityData::Arch` 携带 `ArchKind`、类名、**proxy-graphics 预览轮廓（世界坐标）**与原始 DWG/DXF 体 | **部分**：只有轮廓是真实的；开洞、3D 体量、类参数、样式/锚点对象未建模 | 每个 ≥2 点轮廓画成**开放折线**（记录无闭合标志，不臆造 `closed`）；无轮廓 ⇒ `Opaque` + 具名 `Partial` | 按折线 | 不支持 |
| CAMERA / 动态块参数与夹点 / OLEFRAME / LAYOUTPRINTCONFIG / FORMAT / LEGACY / REGISTEREDCLASS | 实现(仅 common 或无语义) | 显式无显示 | 不绘制(`Opaque`/`Unverified`) | — | — |
| 3DSOLID / BODY / REGION / SURFACE (ACIS) | 实现(acadrust 解析 SAT/SAB；导入期离散为**显示网格**；解析成功时**保留中性 B-Rep**。曲面 = plane/cylinder/cone/sphere/torus，以及 **NURBS**（record `Spline`/`RationalSpline` → `BrepSurface::Nurbs`：齐次控制网 + 隐式次数 + `Face.domain`，`Spline` ⇒ 权重 `1`；**恰好 `half_angle == 0` 的 record cone 按 cylinder 读**——这是内核圆角的实际写法，见 P2.1c-2；`BallPatch`/`PlacedRevolution`/`Blend` 仍按名拒绝，样条/椭圆**边界曲线**也仍按名拒绝）；边曲线 = 直线、整圆、**圆弧**（`ellipse-curve` 且 `ratio == 1` 的**开边**，两个顶点必须同圆且同面，半径与平面的容差 `1e-9·max(1,r)`；跨度 = `(end_param − start_param).rem_euclid(2π)` —— ACIS 参数可跨 seam 回绕，按 `\|end − start\|` 读会静默镜像到补弧；非半弧由跨度定方向、**半弧**按 ACIS 逆时针约定取 `about_normal`；跨度 ≤ `1e-9` 是独立码 `arc-without-span`，且 **v400 SAT 的 Arc 边不可导入**——acadrust 升版转换器把边参数补 `0.0`）。容差 `BrepData.tolerance` / `BrepFace.tolerance` 是**模型单位长度**的 `Option<f64>`：`None` = 源未给出、绝不臆造；组合 `BrepPlacement` **不**改容差，`record::bake` 是**唯一**缩放点（进内核 record 时按比例把 body 与 face 各乘一次）；非均匀变换仍降级 `BodyExact::Unavailable`。写 record 的 body 容差：body 有值用它，否则取**已声明面容差的最大值**，再否则用**我们的**默认 `1e-7`；body 与任一已声明面容差相对偏差 > `1e-6`、或面容差非有限/非正 ⇒ `solid.lift_unsupported`（点名 face id 与两个数）；读回面级恒 `None`（record 没有逐面容差槽位）。**导出**：P3a（2026-11-17）DXF 写内联 **ACIS SAT 文本**的 `3DSOLID`（AC1024），P3b（同日）DWG 写**二进制 SAB**（AC1032 AcDs 段）；**P3 版本选择器（2026-11-17）**把两个常量变成显式选择：`acisVersion` ∈ {`7.0`(700), `21.0`(ASM 21800)}、`dxfVersion` ∈ {`2010`(AC1024), `2013`(AC1027), `2018`(AC1032)}，默认 `7.0`+`2010`（与原 SAT 文本路径相同）；DXF R2013/R2018 把所选载荷作为 SAB/ASM 块写进 `ACDSDATA`，`DxfText+R2010+21.0` 具名拒绝 `export.acis_asm_requires_r2013`，`Dwg+非 R2018` 具名拒绝 `export.dwg_version_fixed`（DWG 恒 AC1032）。**版本不扩大可写子集**：仍是单壳、全平面、**直线**边界、真相似放置；其余退回 POLYFACE 并记 `acis.*`（**P2.1b 的曲面 lift 也不扩大导出子集**：曲面体仍按 `acis.curved_face_not_written` 退回 POLYFACE）。**未运行任何第三方 CAD 读取器**（AutoCAD/ODA/vendor 均未打开这些字节）；AC1027+ 容器的 **ASM 21800 载荷**与 AC1027+ 本身都只有 acadrust 自读证据，且 AC1027+ 的 `acis_revision` 块被**置零**写出（没有可保留的源 revision）——三项都是**未验证的 vendor 接受性风险**、不声明） | `SemanticGeometry::Body3d`：`exact`（中性 B-Rep，仅当 lift 成功）+ `display`（网格 + 边线 + 弦高） | **会绘制**：导入期经 `cad-kernel-adapter` 离散，容差 = 包围盒对角线的 **0.1 %**；精确 B-Rep 保留时平面体可 `Complete`，曲面体 `Partial`（解析曲面按参数采样）。**NURBS 面**：自 **P2.1c-1（2026-11-17）** 起由 `SolidTessellator` 接缝接入内核 `solidcore-tess`（同 pin、`publish=false`；只有 `cad-kernel-solidcore` 可命名），**只对含 NURBS 面的 body 委派**，解析体仍走中性离散器；输出恒 `Precision::Approximate`（`error_bound` = 实际使用的弦高），缺面进 `missing_faces`，不到请求精度/按裁剪放行/边采样截断记 `kernel.tolerance_not_met`。**可达性**：ACIS 读取器今天对 spline 曲面**按名拒绝**，所以导入侧还产不出 `BrepSurface::Nurbs`；该显示路径要等曲面导入或内核圆角产物，`cad-connector-acadrust` **有意未注入**网格器。**圆弧边界**：平面面可 lift、按**弦近似**离散（报弦高上界，`Precision::Approximate`）并以精确有理二次 NURBS + 精确 UV pcurve 回写 record；**圆弧边界的曲面侧面**（半圆柱壁、圆角槽侧壁等）现在**能 lift**（P2.1b part 2，2026-11-17）：曲面面带其解析 label、`solidcore::eval::form::surface` 给出的**精确有理 NURBS form**、form 参数下的 `(u,v)` box 与每个 use 的**精确 pcurve**（`solidcore_ops::pcurve::pcurves_within`，只接受闭式解、绝不拟合；未命中 ⇒ 具名 `solid.pcurve_not_exact`）回写 record（纬圆先重标到面自己的 frame，再由 `verify_pcurves` 的 6 样本/1e-9 相对比对证明；**裁剪偏离四分之一网格**同样答 `solid.pcurve_not_exact`）。被剪裁的侧面与**完整圆锥**的 apex band（一条母线用两次、apex 作对端顶点）都按内核 record 自己的写法落地——**不是只有截头圆锥能 lift**。**lift 成功 ≠ 显示完整**：中性离散器的圆柱侧面条带要求“两个整圆”，被剪裁的侧壁按名报缺面（`cylinder boundary is not a single full circle`）；NURBS 面在**未注入**网格器的调用里按名缺面，注入了 `SolidCoreTessellator` 的载体（Linux）改由内核网格器绘制（P2.1c-1），整条曲面体仍可能是 `Partial`。曲面 lift 的能力位是 `ModelerCapabilities::nurbs`（已置 `true`，登记字符串 `nurbs.faces`）；布尔 / 质量属性 / 倒角任一操作数含非平面面时该位**必需**，声明 `false` 的模型器直接答 `solid.nurbs_not_supported`。**圆弧只在其平面内 ⇒ 全平面闭合壳含圆弧即零厚度圆盘**，内核对它返回 status `1001` → `solid.kernel_failed`（**内核拒绝退化体，不是转换缺口**）。超出子集/原始 SAT/SAB 被拒 ⇒ 仍 `Opaque` + 内核具名码 | 经显示网格 | 不支持 |
| TEXT/MTEXT 格式 | 实现(格式 run 解析) | 部分(堆叠分数/颜色/装饰/行对齐为显式 Partial) | 按 run 整形/换行 | — | — |
| 实体颜色/线宽 | 实现(ByObject/ByLayer/ByBlock，ACI/RGB) | 实现(颜色)；线宽仅携带 | 颜色进入 shader；线宽**显式不绘制** | — | — |
| LINETYPE 虚线 | 实现(名称/表/线型比例) | 实现(ByObject/ByLayer/ByBlock) | 按弧长细分 dash/gap；复杂线型仅 dash、显式 Partial | — | — |
| 渐变 HATCH | 实现(gradient_color) | 部分(LINEAR/SPHERICAL/CYLINDER **按原值烘焙**；其余显式 Partial) | 逐顶点颜色烘焙，裁剪于边界。**角度组 460 = 弧度（已核实）**：DXF 参考、acadrust 字段文档（`HatchGradientPattern.angle`，"Gradient angle in radians"）与本仓方向回读测试（`gradient_angle_is_stored_in_radians_not_degrees`：存 `FRAC_PI_6` ⇒ 烘焙斜坡 30°）一致；`angle_raw` 按原值往返、**不做度/弧度换算**，旋转时随几何同向旋转（`angle_raw += delta`，`transform_rotates_the_gradient_ramp_with_the_geometry` 钉住）。**残余**：仍**无** vendor 产出的非零角度渐变文件往返证据；`color_name`/`book_name` **不可写**（声明损失）；ADR-0018 U-13 已解决 | 经边界 | 不支持 |
| 出图(打印) | 实现(PLOTSETTINGS/内嵌 Layout；plot-style 引用读取并显式 Unsupported) | 实现(纸张/边距/比例/旋转) | 光栅 PNG(host readback)；无矢量/CTB | — | — |
| 异步导入 | 实现(进度阶段/取消) | 实现(唯一任务、过期丢弃) | — | — | — |
| 天正/探索者代理 | 实现(仅公开缓存记录；派生 primitive 可拾取，2026-11-05 ADR 0030)；**2026-11-23 P0**：墙/洞/文字/块插入的版本无关语义写入 `.ymodel` `tables/vendor.json`（minor 12），重开由 `YANS` 重算；`openProgress`/`documentReady`/`document-ready` 与 SAVE/EXPORT/PLOT 门禁已接线 | 仅 FillOff/UnicodeText 有证据；属性面板/编辑拦截为契约级；**TCH proxy 的缓存几何不可靠**（常见零 primitive），重开 `.ymodel` 不能靠代理缓存兜底；**P1 其余类语义、copy hook、CLI 显式拒绝未实现** | 依解码结果；`Opaque` 有派生几何时进入拾取目录（Lines/Segments/Arc/Circle/Text 锚点；Block 不展开） | 实现（源实体 id；无派生几何仍显式 `skipped=opaque`） | 标记缓存几何；raw class body 默认不落盘；证据见 `docs/validation-vendor-ymodel-2026-11-23.md` |
| 布局 / 视口 | 实现(矩形裁剪/比例；矩形 VPCLIP 边界实体可证明为轴对齐矩形时按其矩形裁剪；`tab_order` 顺序 + LAYOUT 显示名) | 复杂/非矩形裁剪标记部分 | **实现（活动空间渲染已接线；`CadStatus.layouts`/`activeSpace` + `LayoutTabs`/`SPACE`，2026-11-02）** | 部分（真正不支持的布局页签可见但禁用，只显示稳定原因码） | 纸空间测量显式禁用 |
| **XREF（外部参照）** | **部分（仅导入期只读解析，2026-11-02）** | **实现：acadrust `BlockRecord` 的 xref 标志/`xref_path` + `INSERT`；无独立 `Xref` 对象；无 `XREF` 命令/ATTACH/DETACH/RELOAD/BIND/编辑** | **加载成功的模型空间内容合并进宿主 xref 块后按普通块绘制；未加载状态显式缺席，不画假内容** | 合并成员随普通实体拾取；无 xref 专用拾取 | 不支持 |

### Bentley DGN V7 / V8（矢量文档，2026-11-14）

DGN 是**矢量文档** lane（ADR-0033 D11），不是网格预览：level、线型、文字样式、共享单元与
Line/Polyline/Circle/Arc/Ellipse/Spline/Text/Point 直接进入 `cad-db`。适配器在
`crates/cad-connector-dgn`，依赖 `cadkit-dgn =0.5.4`（MIT OR Apache-2.0，纯 Rust，
`default-features = false`，读取器自带 CFB 解析，未修改上游、无 patch）。检测：V7 固定魔数；
V8 是 OLE/CFB，`detect_format` 只返回 `Ole`，`cad_app::import::FormatImporter` 先用
`cadkit_dgn::sniff` 的 `Dgn~H` 根存储判定 DGN，否则仍走 RVT；不按扩展名猜。

| 格式 | 读取 | 语义 | 绘制 | 拾取 | 测量 | 明确边界 |
|---|---|---|---|---|---|---|
| Bentley DGN V7（ISFF）/ V8（OLE-CFB） | **实现（`cadkit-dgn` 0.5.4，reader-only，精确固定）** | **矢量文档**（level→Layer、linetype、text style、shared cell→Block/Insert、Line/Polyline/Circle/Arc/Ellipse/Spline/Text/Point；complex/text-node/cell group 的成员扁平化；layer 只写名称/可见性；0.5.4 起上游可从 240 字节 level-entry 解出颜色索引（写 `Layer.color`）与 raw weight/style index，但**仍不解码 freeze/lock**，适配器**仍不写** `LayerAttributes`） | 实现（精确几何）；Text/Insert 为 `Unverified`（字体/块成员能力） | 实现（精确几何） | Line/Polyline/Circle/Arc/Ellipse/Spline/Point 实现；Text/Insert/Raster/Unknown 不支持 | **单位**：`cadkit` 已把 UOR 转成主单位；按 `meters_per_unit` 精确匹配 m/mm/in/ft，其它值（含 survey foot/Custom）回退 `drawing_units()`，绝不假设毫米。**linetype 的 pattern 自 `cadkit-dgn` 0.5.4 起是 drawing units**（内置装饰线型 1..=7 由纸面 mm 经 master unit 换算），适配器**不再二次除 UOR**；文件声明 master-unit 尺寸时得到真实纸面长度（契约夹具 `DGN_DOT_DASH` 由 0.00045 m 变为正确的 0.0045 m）；只有“内置纸面线型 + 文件无 master-unit 尺寸”才记 `import.dgn.linetype_pattern_unscaled` 并退 continuous，code 不变、仍显式建模。**明确缺口**：level display/freeze/lock 未解码（全部 level 按可见，`import.dgn.level_symbology_undecoded`；**每个成功导入恒 `Partial`**）；只导入 `Document::model_space()`，其它模型跳过（`import.dgn.model_skipped`）；references/xrefs、XAttributes、dimensions/multiline/mesh/solid/surface 未建模（`import.dgn.unknown_element`/`element_not_modelled`）；Item Types/tags 见行末 BIM 语义段；栅格 `size_px` 已知时写 `SemanticGeometry::Image`（u/v 按每像素，外部 TIF 不加载），未知时 `Opaque`，均记 `import.dgn.raster_not_loaded`；DGN tags/Item Types 作为 PSet 进入 `cad-db`（见行末 BIM 语义段；`import.dgn.tags_not_stored` 只覆盖确实无法解码/无行的条目）；扩展色 >255 仍由上游回退 `ByLayer`，并新增上游按文档聚合的 warning `dgn.color_extended_unresolved`；缺块 Insert 跳过 + Partial（不伪造空块）；无 DGN 写出。**0.5.4 行为变化（2026-11-21，源码检视，不是运行/样本验收）**：V8 property bit `0x0080` 现使图元 `visible=false`（适配器仍按既有 hidden 集隐藏、不删除；此前上游总报 visible）；`ff fe` BOM + UTF-16LE 的 linkage 名称（level/model/cell/tag set/单位 label）可解码，V8 无 stored-length 的文本由 range 推导，2D tag 旋转改按 angle 读（兼容识别旧 quaternion）；复合链/形 header 会提前关闭超声明计数的父链（上游 issue #27）并给出上游 warning `dgn.complex_count`。这两个上游 warning 不属于本适配器的 `import.dgn.*` 稳定码，也不进入 `ImportReport`（适配器当前不转发 `Document::warnings`）。**没有几何/全部元素未知 → `Unsupported(import.dgn.no_geometry / import.dgn.unknown_elements_only)`，不是空成功**。`DGNIMPORT`/`DGNATTACH`/`DGNCLIP`/`DGNLAYERS`/`DGNADJUST`/`DGNEXPORT` 仍是具名拒绝（打开文档 ≠ 合并进当前 DWG）。证据：GDAL MIT/X11 的 V7 `smalltest.dgn` 与 V8 `test_dgnv8.dgn`（ODA Teigha 写出，非 MicroStation）；`YACR_TEST_DGN` 未设置即跳过，跳过不是已运行证据；Linux 真实 GPU 视觉运行（嵌套 Weston + Vulkan Quadro P620，debug）见 `docs/evidence/dgn-linux-gpu/`；Windows/macOS/Android/Web 运行 **NOT VERIFIED**；**BIM 语义（2026-11-21，ADR-0042 Phase 3，`Unverified`）**：**已落库为 PSet**（`crates/cad-connector-dgn/src/bim.rs`）——tags/Item Types 按 `set` 合成 PSet（无 set 归 `DGN_TAGS`），浮点保持 `f64` 不格式化，`evidence=Declared`、整体 `Unverified`；仅当标签名为 `Category`/`IfcType`/`IFCEntity`（大小写不敏感）且值是单个 token 时写 `bim.category`；**level/cell/几何不推断**；诊断 `import.dgn.bim.{category_from_tag,no_category,unverified,budget_exceeded}`，`import.dgn.tags_not_stored` 收窄为「确实无法解码/无行」（已提交夹具 `test_dgnv8.dgn` 只有一个只能叫 `tag_0` 的标签，因此仍按此报告）；端到端走合成的 `cadkit_core::Document`，**上游 tag 解码本身未在真实 V8i/CONNECT 文件验证**。该视觉证据不是 golden/像素/性能验收。 |

### 网格/实体格式（OBJ / STL / PLY / OFF / STEP / IFC / RVT / SKP，2026-11-13）

`cad_app::import::FormatImporter` 在进入适配器前做一次保守格式嗅探
（`cad-connector-core::detect_format`：SKP 固定 32 字节头 `FF FE FF 0E` + UTF-16LE
`SketchUp Model`（**最先**探测：定长签名比 OLE/`AC10xx` 便宜，且没有别的格式以 `FF FE FF`
开头）⇒ `Skp` → DGN V7 固定魔数 ⇒ `Dgn` → OLE/CFB 魔数 `D0 CF 11 E0 A1 B1 1A E1` ⇒ `Ole`
（`cad_app::import::FormatImporter` 先用 `cadkit_dgn::sniff` 的 `Dgn~H` 根存储判定 DGN V8，
否则交给 RVT）→ DWG
签名 → ASCII/二进制 DXF 段流 → ISO 10303-21（`FILE_SCHEMA` 为 IFC ⇒ `Ifc`，否则 `Step`）→
STL → PLY → OFF → OBJ，否则 `Unknown` 显式拒绝）。OLE 魔数只能说明“复合文档”，
项目/模板与家族/家族模板由 RVT 适配器按流区分（见下），检测层不按扩展名猜；SKP 头也只说明
“SketchUp 模型”，release 与可读性由 SKP 适配器回答（`import.skp.version_unsupported`）。
IFC 与 STEP 共用 Part 21 容器，但 IFC 有独立 schema 与独立导入 lane：dispatcher 按
`FILE_SCHEMA` 把 IFC 交给 `cad-connector-ifc`，**不会**把 IFC 交给 STEP 适配器，cadmpeg 也完全
不接触 IFC（STEP 适配器对 IFC 仍按名拒绝 `import.step.ifc_schema_refused`）。
OBJ/STL/PLY/OFF/STEP/IFC 都只产生**中性网格**，入库走共享的 `cad-connector-core::MeshImport`，因此
这些源的能力行一致：绘制/拾取 `Verified`，测量 `Unsupported`（网格不是 MEASURE 目标；
STEP/IFC 的精确 B-rep 也没有暴露给域）。**RVT 与 SKP 是例外**：它们都是预览路径，绘制/拾取的上限由
`rvt`/`skpwright` 的 release 状态决定。RVT 测量恒为 `Unsupported`；SKP 只在**确实携带真实存储边**的
显示边载体上答 `measure = Partial`（仅端点/中点吸附，ADR-0050），三角-only 的 `SkpPreviewMesh` 仍
`Unsupported`（见下表）。

| 格式 | 读取 | 语义 | 绘制 | 拾取 | 测量 | 明确边界 |
|---|---|---|---|---|---|---|
| Wavefront OBJ | 实现（`tobj`，三角化后单索引） | 中性网格 | 实现 | 实现 | 不支持 | 单位未知（`UnitContext::drawing_units()`，绝不假设毫米）；`f` 面 >3 顶点由读取器**扇三角化**，记录 `import.obj.ngon_fan_triangulated` 并置 `Partial`（凹 n-gon 可能不正确）；退化/零面积三角形丢弃并记录 `import.obj.degenerate_triangle`；超出 `max_mesh_faces`/`max_mesh_face_indices` 是命名计数的 `InvalidInput`，不截断；`mtllib` 外部材质库**不加载**（字节流无文件源），记录 `import.obj.mtl_not_loaded` 并回退 `ByLayer`；**不导入纹理或 UV**：OBJ 声明了 `vt`/`mtllib` 时记 `import.obj.texture_not_imported`（一份文档一条，不逐面刷屏）；无外部参照（`XrefOptions` 仅 DWG）；逐顶点颜色只在源确实提供时映射。**边（ADR-0051）**：写出表面带 `BodyDisplay.edges` 时逐实体记 `export.obj.stored_edges_not_written`（`Partial`）；wire-only body 记 `export.obj.wire_body_not_written`（`Unsupported`，不重复记）；无边文档逐字节不变 |
| STL（ASCII / 二进制） | 实现（`stl_io`，纯 Rust 无传递依赖） | 中性网格 | 实现 | 实现 | 不支持 | 单位未知（图纸单位）；STL 法向是**逐面**的，入库按三角形展开顶点保存为逐顶点法向（平面着色）；零/非有限法向由三角形几何推导并记录 `import.stl.normal_recomputed`（不改变完整性）；退化面丢弃并记录 `import.stl.degenerate_triangle`（`Partial`）；截断的二进制面计数是 `CorruptData`，不是空成功；无颜色、无外部参照；**边（ADR-0051）**：写出表面带 `BodyDisplay.edges` 时逐实体记 `export.stl.stored_edges_not_written`（`Partial`；全部面退化的表面没有写出几何，因此不额外声称边丢失），wire-only body 记 `export.stl.wire_body_not_written`（`Unsupported`，不重复记）；无边文档逐字节不变 |
| PLY（Polygon File Format，ASCII / binary_little_endian / binary_big_endian 1.0） | 实现（`cad-connector-mesh` 内**手写解析器**，不新增第三方依赖） | 中性网格（单文件**一个**网格实体） | 实现 | 实现 | 不支持 | 单位未知（`UnitContext::drawing_units()`，绝不假设毫米）；`element vertex` 必须存在，`x,y,z` 按**名称**从任意属性顺序读取；`nx,ny,nz` 可选法向（部分声明/非有限 → 整体丢弃 + `import.ply.normals_dropped`，不置 `Partial`）；`red,green,blue` / `r,g,b` 可选颜色，仅 uchar 0–255 或 float 0–1 映射，其它类型/越界/混合类型 → 丢弃颜色 + `import.ply.colors_dropped`（不猜）；面为 `property list <ct> <it> vertex_indices`（或 `vertex_index`），n>3 **扇三角化** + `import.ply.ngon_fan_triangulated` + `Partial`（凹 n-gon 可能不正确）；n<3 → `import.ply.face_too_few_vertices` + `Partial`；退化三角形丢弃 + `import.ply.degenerate_triangle`；**载荷短于声明元素计数** → `CorruptData`（`import.ply.truncated_payload`，绝不部分成功丢尾）；元素后的非空白尾随字节 → `CorruptData`（`import.ply.trailing_bytes`）；非有限坐标 / 越界索引 → `CorruptData`；未知 header 指令、format 版本/编码、标量类型 → 显式 `CorruptData`（`import.ply.malformed_header`/`unsupported_format`/`unsupported_property_type`）；超出 `max_mesh_faces`/`max_mesh_face_indices` → 命名真实计数的 `InvalidInput`；未知元素/属性按声明宽度跳过（不猜测）；**纹理坐标不导入**：声明 `s`/`t` 时整组丢弃并记 `import.ply.texture_coords_not_imported` + `Partial`（没有纹理字节可采样）；无外部参照；单网格/文件。**边（ADR-0051）**：写出表面带 `BodyDisplay.edges` 时逐实体记 `export.ply.stored_edges_not_written`（`Partial`），wire-only body 记 `export.ply.wire_body_not_written`（`Unsupported`，不重复记）；无边文档逐字节不变 |
| OFF / COFF（Object File Format，**仅 ASCII**） | 实现（`cad-connector-mesh` 内**手写解析器**，不新增第三方依赖） | 中性网格（单文件**一个**网格实体） | 实现 | 实现 | 不支持 | 单位未知（`UnitContext::drawing_units()`，绝不假设毫米）；只接受 `OFF` 与 `COFF`，`NOFF`/`CNOFF`/`STOFF`/`4OFF`/`4NOFF` 等按名拒绝 `import.off.variant_refused`（dispatcher 会把已知家族前缀路由到该适配器，不落成泛化 `Unknown`）；第二行 `numVertices numFaces numEdges`（边数接受但忽略）；顶点 `x y z [r g b [a]]`，颜色接受 0–255 整数或 0–1 浮点，**整数 0/1 三元组歧义**（两种读法都成立）→ 整体丢弃 + `import.off.colors_dropped`，不猜；alpha 不存储 + `import.off.alpha_dropped`；面 `n i0 … i(n-1) [faceColor]`，n>3 **扇三角化** + `import.off.ngon_fan_triangulated` + `Partial`（凹 n-gon 可能不正确）；n<3 → `import.off.face_too_few_vertices` + `Partial`；退化三角形丢弃 + `import.off.degenerate_triangle`；面颜色 token 不导入 + `import.off.face_color_not_imported`；**行数少于声明计数** → `CorruptData`（`import.off.truncated_payload`）；声明计数之后的非注释数据 → `CorruptData`（`import.off.trailing_data`）；非 ASCII 输入 → `CorruptData`（`import.off.non_ascii`）；非有限坐标 / 越界索引 → `CorruptData`；超出 `max_mesh_faces`/`max_mesh_face_indices` → 命名真实计数的 `InvalidInput`；无外部参照；单网格/文件。**边（ADR-0051）**：写出表面带 `BodyDisplay.edges` 时逐实体记 `export.off.stored_edges_not_written`（`Partial`），wire-only body 记 `export.off.wire_body_not_written`（`Unsupported`，不重复记）；无边文档逐字节不变 |
| ISO 10303-21 STEP（AP203/AP214/AP242） | **实现（`cadmpeg-codec-step` 0.6.12 解码，`cadmpeg-tessellate` 逐面三角化）** | 中性网格（**精确 B-rep 不进入域**，只保留面片） | 实现（每个 body 一个 `DbEntity`；同一 body 被多个 occurrence 实例化时每个实例一个实体；occurrence 定位后写入模型空间） | 实现 | 不支持（网格不是 MEASURE 目标，精确 B-rep 未暴露） | **面片化成功即显示 `Verified`，但拒绝面真实存在**：参考样本 NIST `nist-ctc05` AP242 156 面中 155 面成功、1 面拒绝（`a wrapping loop bounds a strip with no far side`），ABC AP214 样本 25 面中 22 面成功、3 面拒绝；因此导入通常为 `Partial`。每个拒绝面 → `import.step.face_refused` + `Partial`；全部面被拒 → `Unsupported`（`import.step.no_tessellated_faces`），**不是空成功**；无 B-rep 面（如只有 AP242 面片化几何）→ `import.step.no_brep_faces`。**单位**：`cadmpeg-ir` 只存规范**毫米**且不暴露 STEP 声明的源单位，因此报告毫米并记 `import.step.units_canonicalised`；若解码报告出现 `geometry.document-length-unit-unresolved`，回退 `UnitContext::drawing_units()` 并记 `import.step.units_unknown`（不假设毫米）。**装配定位**：`Model::occurrences`（`Occurrence::transform` 沿 `OccurrenceParent` 复合 + `Occurrence::scale`）与 `ProductDefinition::bodies`；解码报告出现未解析/歧义 placement 且文件有多个 body 时按名拒绝 `import.step.assembly_placement_unsupported`，**绝不**把各 body 堆在原点；body 有定义但无 occurrence 的多 body 文件同样拒绝；`Body::transform`（独立 `MAPPED_ITEM` 的 world 放置）只在 occurrence 为恒等时应用，两者同时出现按名拒绝（避免重复定位）。n-gon 由 tessellator 自行三角化；解码 loss 逐条以 `import.step.loss.<code>` 传播并保留严重度（warning/error → `Partial`）；其他诊断：`import.step.face_empty`、`import.step.degenerate_triangle`、`import.step.normals_dropped`、`import.step.empty_body`、`import.step.face_unattributed`、`import.step.body_uninstantiated`。无外部参照；`vendor_records` 为空 |
| IFC（IFC2X3 / IFC4 / IFC4X3） | **实现（`ifc-lite-core` + `ifc-lite-geometry` 28.3.0，MPL-2.0，精确固定版本）** | 中性网格（精确 B-rep/参数化体不进入域，只保留三角面片）；**BIM 语义**另存为元素可选 `bim` 块 + 定义表 + 关系表（`Partial`，见行末与「BIM 语义」节） | 实现（每个有几何的 product 一个 `DbEntity`；placement 链由路由器烘焙为世界坐标；`IfcOpeningElement` 只作为 cutter，不入库） | 实现 | 不支持（网格不是 MEASURE 目标，精确 B-rep 未暴露） | **单位**：路由器按 IFC 项目声明的长度单位把坐标与 placement 缩放为**米**；数据库存米，显示按可表示的声明单位（mm/m/in/ft）换算，其他声明单位显示为米；无 `IfcUnitAssignment`/单位链解析失败 → `UnitContext::drawing_units()` + `import.ifc.units_unknown` + `Partial`，**绝不假设毫米**；声明了不可用的转换因子 → 按名拒绝 `import.ifc.units_refused`。**定位**：`process_element*` 沿 `IfcLocalPlacement` 烘焙世界坐标（合成 IFC2X3 夹具固定两面墙 x 0..4 m / 6..10 m，不堆原点）。**洞口/CSG**：`process_element_with_voids` + `IfcRelVoidsElement` 做布尔裁剪（合成墙 12→32 三角），CSG 失败逐条 `import.ifc.csg_failure` + `Partial`；被路由器丢弃的 representation item 按类型汇总为 `import.ifc.item_unsupported` + `Partial`；NURBS/复杂 CSG 是 best-effort。单个 product 网格失败 → `import.ifc.element_failed`；空 product → `import.ifc.element_empty`；全部 product 失败 → `Unsupported`（`import.ifc.all_elements_failed`），**不是空成功**；无几何 product → `CorruptData`（`import.ifc.no_geometry_products`）；没有 `IfcRelVoidsElement` 宿主的孤立 opening 不绘制并记 `import.ifc.opening_without_host` + `Partial`。`IfcStyledItem` 颜色/材质不导入（`import.ifc.style_not_imported` + `Partial`，回退 `ByLayer`）。**纹理（2026-11-20，ADR-0038）**：基色纹理经 `process_element_with_submeshes_textured` 连 UV0 + RGBA→确定性 PNG 导入；`IfcImageTexture` 外链不读、`import.ifc.external_texture_unavailable`；带 void 的构件走既有裁剪路径、拿不到 UV 时回退样式颜色并记 `import.ifc.textured_void_fallback`；UV 与顶点不配对、PNG 编码失败分别记 `import.ifc.uvs_dropped`/`import.ifc.texture_encode_failed`；纹理**导出**不在本轮范围。**28.3.0 行为变化（2026-11-21，源码检视，不是运行/样本验收）**：void 宿主改为在元素 root frame 内切割后再应用 RTC；CSG consolidate / CDT incircle / mesh orient / weld 有修正；新增 `output_policy`。使用的 API 无破坏性变化，但同一模型的三角化输出**可能**变化，现有契约测试通过**不能**写成几何 golden 验收。默认 feature 现在会编入 CityJSON 写出器（新传递依赖 `i_overlay`）与 native result cache（`ifc-lite-processing` 的 build.rs 在 unix 编 `libc`，运行时默认不启用）。**BIM 语义导入（2026-11-21，ADR-0042 Phase 1，天花板 `Partial`）**：第二遍扫描（`crates/cad-connector-ifc/src/semantics.rs`，几何路径不变）读 `IfcRoot.GlobalId`/`Name`、`IfcElement.Tag`（spatial 元素的 slot 7 是 `LongName`，不读）、`IfcProject.GlobalId`（→ `document_id`）、`IfcBuildingStorey`（elevation 按项目长度单位换算到米，单位不可解析存 `None`）、`IfcRelAggregates`/`IfcRelContainedInSpatialStructure`（向上最多 16 条边、防环）、`IfcTypeObject` 家族 + `IfcRelDefinesByType`、`IfcPropertySet` +`IfcRelDefinesByProperties` + `IfcPropertySingleValue`、`IfcMaterial` + `IfcRelAssociatesMaterial`；`IfcSpace` 作为普通元素 `category="IfcSpace"`。**明确不存**：`IfcComplexProperty`/枚举/有界/Table（只计数，`import.ifc.bim.property_value_unsupported`）、`IfcPhysicalComplexQuantity` 等不压平（`import.ifc.bim.quantity_set_not_stored` 收窄为「确实读不出」）、`IfcStyledItem` 颜色（仍是 `import.ifc.style_not_imported`）。**数量集已存为 PSet（Phase 3，2026-11-21）**：`IfcElementQuantity` 以源名（如 `Qto_WallBaseQuantities`）存进 `bim.psets`，Length/Area/Volume/Weight/Time → `real{value, ifc_type: 源实体名, unit}`、Count → `integer`；**不换算**（3000 mm 存 3000 + `unit: "mm"`），单位取自条目 `Unit` 或项目单位、解析不到就 `unit: None`（不猜米），信息性 `import.ifc.bim.quantity_set_stored` **不降级**。**关系已落 `tables/bim-relations.json`（Phase 3，2026-11-21）**：`IfcRelAggregates→aggregates`、`IfcRelNests→nests`、`IfcRelContainedInSpatialStructure→contains`、`IfcRelAssignsToGroup→group`、`IfcRelConnectsElements→connects`；端点 = 项目 GlobalId + 对象 GlobalId，关系自身 GlobalId/Name → `source`/`label`；端点缺 GlobalId 跳过并记 `import.ifc.bim.relation_endpoint_missing`，行超预算整表拒绝 `relation_budget_exceeded`；**仅存在关系不降级**。只有几何、无关系级 IFC 仍 `Complete`（仅多出 source/category/name）；出现关系级 BIM 数据即`Partial`——映射只是**代码契约证据**，没有 vendor 交叉验证。无外部参照（`XrefOptions` 仅 DWG）；`vendor_records` 为空。**IFC4 导出已实现（2026-11-20）**：`cad-connector-ifc` 用 `ifc-lite-export` 28.3.0（MPL-2.0）从 `cad-db` 网格写 `IfcBuildingElementProxy` + `IfcTriangulatedFaceSet`；源单位必须已知（mm/cm/m/in/ft）并**一次**换算为米，`DrawingUnits`/缺单位按名拒绝 `export.ifc.units_unknown`；模型空间之外、非网格、空网格逐条落报告；颜色/法向/图层不写入时记 `export.ifc.colors_dropped`/`normals_dropped`/`layers_dropped` 并把完整性降为 `Partial`。证据仅**自洽往返**（写出→`cad-connector-ifc` 读回同一三角数/顶点数与米坐标），**未经 IFC 厂商/IfcOpenShell/buildingSMART 验证**，不是 golden 或产品验收；**IFC4 导出已回写 BIM 语义（Phase 2，2026-11-21）**：`database.bim_elements()` 非空时写 `IfcBuildingStorey`（按 `(elevation,name,id)` 排序、米）、`IfcClass::Named(category)`（显式白名单 + 内置 IFC4 registry 逐项对照，未知回退 `IfcBuildingElementProxy`）、`<Category>Type`（白名单；与源 class 不一致则跳过不替换）、`IfcMaterial`（按名去重）、PSet，并复用源 `GlobalId`（自带 22 字符 IFC base64 → `u128` 解码；非法/重复回退 `stable_global_id` 并报 `export.ifc.bim.*`）。**已知边界**：**关系只导出 `contains`/`nests`**（`IfcRelContainedInSpatialStructure`/`IfcRelNests`）；`aggregates`/`group`/`connects`/`other:*` 因上游 plan API 无法表达任意产品间关系而**不导出并报 `export.ifc.bim.relation_not_written`**（不写成容器/嵌套冒充）；`cad_db::BimValue::Text` 无 `ifc_type`，一律写 `Label`；未知单位（`kN`/`psi` 等）不写出并计数、`Null` 跳过。**无 BIM 的路径逐字节不变**（既有夹具仍是 2196 字节非空 / 1374 字节空，只出现 `IfcBuildingElementProxy`）。证据仍只有自洽往返（导出 → `cad-connector-ifc` 读回 GlobalId 一致），**无 vendor checker**。**存储显示边（ADR-0051）**：IFC 没有边通道，写出表面带 `BodyDisplay.edges` 时逐实体记 `export.ifc.stored_edges_not_written`（`Partial`），wire-only body 记 `export.ifc.wire_body_not_written`（`Unsupported`，不重复记）；无边文档逐字节不变 |
| Autodesk Revit 项目/项目模板（`.rvt` / `.rte`，OLE/CFB） | **预览实现（`rvt` 0.25.0，Apache-2.0，精确固定版本；RVT → 内部 glTF → 中性网格）** | 中性网格（每个 glTF node/mesh 实例一个 `DbEntity`；**预览几何**，不保留 Revit 语义/参数） | 实现，但**受 `rvt` 的 release 状态上限约束**（`verified`→`Verified`、`partial`→`Partial`、`experimental`/未知→`Unverified`；当前没有任何项目 release 是 `verified`） | 同左（与绘制同一上限） | 不支持（网格不是 MEASURE 目标；`rvt` 自己的文档明确禁止把输出用于测量/协调） | **几何保真度**：Revit 2008–2023 的构件体是**包围盒**（在可读的连接处切分），2024/2025 为 `partial`，2026/2027 为 `experimental`。`.rfa`/`.rft` 家族/家族模板只有元数据、预览图与类型，**没有家族几何** → 按名拒绝 `import.rvt.family_no_geometry`（按字节判定：`PartAtom` 带 `<A:family>` 类型表即为家族；否则有 `ProjectInformation` 流判为项目；两者都读不到时按项目解码，没有几何再由 `import.rvt.no_geometry` 拒绝；不看扩展名）。**单位**：`rvt` 在 glTF 根节点用 `0.3048` 把英尺坐标缩放到米并转成 +Y up（`src/ifc/gltf.rs:286`），因此入库坐标是**米、+Z up**（`UnitContext::from_meter()`），记 `import.rvt.units_metres`；不假设 Revit 显示单位或毫米。**定位**：逐节点应用 `parent_world * local`（`transform.matrix()` 是列主序）；合成双节点 GLB 断言世界包围盒 x∈[-1.5,-0.5] 与 [0.5,1.5] 不重叠，**不堆在原点**。**颜色/透明度（2026-11-10，2026-11-13 补丁；2026-11-14 重挂 0.22.0；2026-11-15 重挂 0.24.0；2026-11-20 重挂 0.25.0）**：以 glTF 材质的 `pbrMetallicRoughness.baseColorFactor` 为准（`rvt` 按 Revit 类别、图层颜色 RE-53、单一材质类型 RE-82 合成材质）；它是**线性**值，按 IEC 61966-2-1 传函转回 sRGB 存入 `EntityColor::Explicit`（正是 `rvt` `colour_material` 线性化的逆运算，合成夹具断言字节级往返）；alpha 仅在 `alphaMode=BLEND` 且 `a<1.0` 时存 `EntityTransparency::Explicit(opacity)`（`1.0` 为不透明），`a=1.0` 不写多余显式值；顶点未带材质 → `ByLayer`（保持原行为，不发明颜色）；非有限/越界 `baseColorFactor`、以及同一 node 合并的多 primitive 材质不一致 → 保持 `ByLayer` 并记 `import.rvt.material_ignored`；`material` 索引越界由 glTF 校验直接拒绝整份 GLB（`import.rvt.invalid_glb`），不会 panic 也不会静默降级。**多材质构件的透明度（2026-11-13，`vendor/rvt` 受控补丁，ADR-0035）**：上游 0.22.0/0.24.0/0.25.0 在构件类型由多个材质组成时把整块预览体退回类别颜色且 alpha=1（单材质但颜色未读到时同样丢 alpha）。补丁后整块预览体沿用类别颜色，alpha 取该构件材质里**最透明**的（`1 - transparency` 的最小值）（仅当至少一个材质读到了 transparency），未按层绘制的层集构件按层材质同样处理；`extras.colourSource`/`alphaSource`（`"file"` = Revit 读到，`"rvt-rs"` = 导出器选择）由上游 0.24.0/0.25.0 写（#1818）。`cad-connector-rvt` 把 `alphaSource="rvt-rs"` 的显式不透明度计进稳定诊断 `import.rvt.alpha_chosen`，**不得**当作该构件自身的透明度。真实本地样本（未提交）580 → 746 个显式不透明度（504 来自文件材质，242 为导出器选择：76 房间类别默认 + 166 个多材质预览体，全部计入 `import.rvt.alpha_chosen`）；Revit 透明度保真度 **NOT VERIFIED**。**PBR 标量（2026-11-20，ADR-0039）**：读 glTF 材质的 `metallicFactor`/`roughnessFactor`（`rvt` 0.25.0 的 `src/ifc/pbr.rs` 按材质**名**启发式生成，不是 Revit 作者数据），建 `MeshPbr` 并每个文档记一条 `import.rvt.pbr_heuristic` note（不降完整性）；非有限/越界值不建 `MeshPbr` 并记 `import.rvt.pbr_ignored`（`Partial`）。RVT 仍无纹理/UV，PBR 贴图恒缺省。**视图专有显示（V/G 覆盖）不读出**：按图元/类别/过滤器的 Transparency、Surface Transparency 与 Halftone 都属于视图态，不在数据导出契约内（上游 `docs/support-matrix.json` 的 `view-graphics-overrides` = `unsupported`，#1812；Revit 自家 IFC 导出同样不应用）。GLB 里没有视图信息，`cad-connector-rvt` 既不伪造也不产生诊断——这条边界只在本表与上游文档声明。**纹理不存在是设计边界，不是缺陷**：`rvt` 0.25.0 不导出任何纹理（`GltfDocument` 无 images/textures/samplers，也无 `TEXCOORD_0`），且 `Mesh` 只有逐顶点 `colors`、没有 UV 通道，所以纹理既不可得也不可表示。**诚实通道**：每个成功导入永远是 `Completeness::Partial`（`import.rvt.preview_geometry`）；release 状态、`confidence`（level/score/has_geometry）、`source_coverage`（status/fractions，未测时保持 unset/空）、`rvt` 跳过记录、未读流、unsupported features、export notes 全部落成稳定诊断；没有几何 → `Unsupported`（`import.rvt.no_geometry`），**不是空成功**；release 没有 reader → `import.rvt.release_unsupported`；非 Revit 的 OLE → `CorruptData`（`import.rvt.not_a_revit_file`）。glTF 在这里是**内部实现细节**：RVT lane 不按扩展名检测、不暴露能力行；用户可见的 `.gltf`/`.glb` 由 `cad-connector-gltf`（导入 + 导出）承担（见「连接器与导出矩阵」）。**纹理**：`rvt` 0.25.0 的 glTF 写出没有 `images`/`textures`/`TEXCOORD_0`，因此**不导入纹理、不放 UV**；若 GLB 里出现 `TEXCOORD_0`/`images`，`cad-connector-rvt` 记 `import.rvt.texture_not_imported`（防御性，现有上游不会产生）。**BIM 语义（2026-11-21，ADR-0042 Phase 2，`Unverified`）**：**已导入**——`decode_to_glb` 在 `model_to_glb` 前把 `rvt` 的 `IfcModel` 映射成 `RvtSemantics`（`crates/cad-connector-rvt/src/semantics.rs`），并用 GLB 的 `node.extras().entityIndex` 把几何行对齐到 `model.entities[i]`：`BuildingElement{ifc_type,name}` → category/name、`global_ids.elements[i]` → `bim.source.id`、`building_storeys[storey_index]`（`elevation_feet×0.3048`）→ level、`element_type_ids` + `Family:Type` → type（名取 Type 段）、`materials[material_index]` → material、`property_set` + `IfcEntity::ElementPropertySet` → psets；`type_guid`（Revit ElementId）→ `native_id`；文档级 `global_ids.document` **不用**（上游说每次保存都变）；`Aggregate` parts 不归组并计数；8 个 `import.rvt.bim.*` 稳定码。**仅代码契约观察**：单个真实样本（`rmeadvancedsampleproject.rvt`）读到 `elements=6783, levels=4, types=90, materials=0, element_psets=44928, with_category=6783, with_level=6702, with_type=6650, with_material=0`（该样本材料表为空，材料路径只有手工 `IfcModel` 单测证据）；**真实 RVT BIM 语义仍 NOT VERIFIED**（`rvt` 自述输出不得用于测量/协调）。**边（ADR-0049 D6）**：RVT 预览路径**没有**真实边/曲线数据（内部 GLB 只有三角形 primitive），因此**不推导**出边数据；网格上可见的边界线只是 ADR-0045 的派生显示，不是文档数据。ADR-0050/0051 的吸附/自由边写出**不改变**这一点（RVT 既无吸附边源，也无边可写）。无外部参照（`XrefOptions` 仅 DWG）；`vendor_records` 为空 |
| SketchUp 模型（`.skp`；stream 容器 3–8/2013–2020，ZIP/tagged-record 容器 2026/2027） | **预览实现（`skpwright` 0.6.0，GPL-3.0-only，纯 Rust、单一依赖 `miniz_oxide`，workspace 中精确固定版本）** | 中性网格 + 显示边载体（每个 geometry run 实例一个 `DbEntity`；三角形 + 组合后的实例世界变换 + tag 扁平化 + 材质颜色/不透明度；**预览几何**，不保留 SketchUp 语义） | 实现，但**受 release 状态上限约束**（3–8、2013–2020、2026、2027 = `Verified`；2021–2025 只按容器等价读、无观测样本 ⇒ `Unverified`；其余按名拒绝 `import.skp.version_unsupported`） | 同左（与绘制同一上限） | **`Partial`（仅端点/中点吸附，ADR-0050）**：只为**确实插入非空 `BodyDisplay.edges`** 的 SKP 载体报（有边的 `SkpPreviewMesh` / `SkpPreviewEdges`）；没有面/三角/中心/象限/垂足/交点吸附，也没有面积/体积；三角-only `SkpPreviewMesh`、其它格式与 ACIS body 仍 `Unsupported`；release 天花板只收窄（`Partial` → `Unverified`），绝不强制 `Unsupported` | **许可**：`skpwright` `=0.6.0` 是 **GPL-3.0-only**；主仓 AGPL-3.0（AGPLv3 §13）可与 GPL-3.0 组合，GPL 部分对下游仍按 GPLv3 分发（见 `THIRD_PARTY_NOTICES.md`）。**读取**：三角形、实例/组件世界变换、tag（图层）扁平成 layer 0 但**遵守可见性**（隐藏实例/隐藏面/不可见图层被裁剪并计数）、材质颜色 + 不透明度 + **正面基色纹理**（`Material::Textured.image_bytes` 原样登记为文档纹理，UV 由 `face.uv_xform(Side::Front, …)` 逐顶点求值；背面纹理不导入、记 `import.skp.back_texture_not_imported`）。**边（ADR-0049）**：`skpwright` 的真实 `MeshEdge` 记录会被携带——一个材质组当且仅当其**每个可见面环**沿环的每一对相邻顶点都有真实记录时才升级为 `Body3d`（`BodyKind::Unknown`，三角形显示 + `BodyDisplay.edges` 存边界边，`type_key` 仍 `SkpPreviewMesh`），否则保持 `SemanticGeometry::Mesh` 并使用 ADR-0045 的派生边界；判据覆盖三角化失败但可见的面。没有被 body 或已三角化面环覆盖的可见记录（游离边；面组没有 body 载体时，三角化失败但可见的面环记录）成为按边材质分组的 `Body3d::Wire`（`SkpPreviewEdges`）。`hidden`/`soft`/不可见图层的边被剔除，`smooth` 不做法向平滑，逐边材质只决定 wire 组颜色，曲线按两点弦存储。边作**显示/拾取**数据（没有边 id/子元素、不可编辑）；吸附/测量是**窄声明**（ADR-0050）：只为实际携带存储边的载体报 `measure = Partial`，仅**端点/中点**，无面/三角/中心/象限/垂足/交点、无面积/体积，三角形不是测量源。**不导入**：场景（scenes/pages）、剖切面、标注、动态组件属性。诊断：`import.skp.edge_flags_not_modeled`、`import.skp.edge_curves_flattened`、`import.skp.edges_hidden_fallback`、`import.skp.groups_without_edge_records`、`import.skp.edges_only`、`import.skp.measure_edges_partial`（有边段发出时恒发，写明端点/中点 only 的窄声明）。纹理只取**正面基色**：外部图像引用记 `import.skp.texture_unavailable`、拿不到 UV 记 `import.skp.uv_unavailable`、背面纹理记 `import.skp.back_texture_not_imported`；无图像可用的纹理组回退材质平均色并记 `import.skp.textures_not_sampled`。**三角化**：先**耳切**（ear clipping）并做面积校验；失败回退 **even-odd**（带洞的面直接走 even-odd）；两者都失败才丢弃该面并记 `import.skp.faces_dropped`（绝不近似）。**单位**：`skpwright` 已把存储的英寸与节点平移换算为**米**，入库 `UnitContext::from_meter()`、`+Z` up，记 `import.skp.units_metres`。**边文件与拒绝**：纯边文件**不再拒绝**（ADR-0049），导入为 `Completeness::Partial` 的 `SkpPreviewEdges` wire 体并记信息码 `import.skp.edges_only`；`import.skp.edge_only_geometry` 已退役、任何路径都不再产生。没有可视面 → `import.skp.no_geometry`；头缺失/截断 → `CorruptData`（`import.skp.not_a_sketchup_file`）；头匹配但解析失败 → `CorruptData`（`import.skp.parse_failed`）；超限 → 命名真实计数的 `InvalidInput`（`import.skp.limits_exceeded`）。每个成功导入恒 `Completeness::Partial`（`import.skp.preview_geometry`），2021–2025 另记 `import.skp.version_unverified`。无外部参照（`XrefOptions` 仅 DWG）；`vendor_records` 为空。**SKP 导出已实现（2026-11-20）**：`cad-connector-skp` 用 `skpwright` 0.6.0 的写出器生成 **2026 容器**（`Document::from_model_with`，固定时间与种子 ⇒ 字节确定）；源单位必须已知并换算为米（`export.skp.units_unknown` 否则）；全部源合并进**一个** root `GeometryRun`（ADR-0051），每个三角形一个带单位 Newell 法向的 `MeshFace` 与去重 `MeshEdge`；退化三角形/非模型空间/非网格逐条落报告，逐顶点颜色/法向记 `export.skp.colors_dropped`/`normals_dropped`。**写出布局与边（ADR-0051）**：钉死的写出器只发第一个 root run，旧的一源一 run 布局会静默丢掉第一个之后的实体，因此所有源合并进同一 run；多于一个源写出时记一次 `export.skp.entity_grouping_flattened`（`Partial`，实体分组不保留），`max_output_entities` 按**源实体**计（含没写出几何的源）。无三角形的 wire 体写成**自由边**：追加自己的端点、不与网格/其它 polyline 焊接、N−1 段链式、默认 flags/layer/material、`curve: None`；零长段跳过并记 `export.skp.zero_length_edge`（若一个 wire 体的所有段都是零长，整实体按 `export.skp.wire_body_no_writable_edges` 丢弃），同一 polyline 内重复端点对只写一次（边集不变，仍计 `exact`）；曲线/弧身份与逐边属性不可还原，记 `export.skp.edge_semantics_not_written`；`export.skp.wire_body_not_written` **退役**、不再产生。**mesh 体**的 stored 边集/分段仍**不复制**（ADR-0049 §2 / ADR-0051 D5），逐实体记 `export.skp.stored_edges_not_written`（`Partial`，三角形边界由写出器重新生成；全部三角形退化的体不额外声称边丢失）。STL/OBJ/PLY/OFF/glTF/IFC 导出器同样没有边通道：写出的表面带 stored 边时逐实体记 `export.<id>.stored_edges_not_written`，wire-only body 整体丢弃记 `export.<id>.wire_body_not_written`（`Unsupported`，不重复记），无边的文档逐字节不变；**不声称任何格式的 vendor 边往返**。证据仅**自洽往返**（写出→`cad-connector-skp` 读回同一三角数与 1 mm→0.001 m 坐标 + `skpwright::verify`），**未经 SketchUp/Trimble 验证**，不是产品验收 |

证据边界：以上只有**合成夹具 + cadmpeg 上游测试夹具 + crate 契约测试**
（`cargo test -p cad-connector-mesh` / `-p cad-connector-step` / `-p cad-connector-ifc` / `-p cad-connector-rvt` / `-p cad-connector-skp` / `-p cad-connector-dgn`），
没有真实厂商 STEP/IFC 样本对照，也不构成产品验收或跨平台运行证据。DGN 只有 GDAL 的 V7 玩具与
ODA Teigha 写出的 V8 玩具（非 MicroStation），不能按“真实 Bentley 文件已兼容”理解。NIST/ABC 的数字来自上游 spike
的有界样本扫描，不是本仓库的 golden 回归。IFC 的 9 个 buildingSMART 样本（IFC2X3/4/4X3，
127–6,393 实体）只是库选型 spike 的本地有界实验，文件未提交、未做 golden 回归；`YACR_TEST_IFC`
未设置时的跳过**不是已运行证据**。IFC 的 CSG/NURBS 高级几何是 best-effort，真实文件不能按
“全部几何成功”理解。RVT 只有 `rvt` 上游的**元数据脚手架**（0 几何，断言显式拒绝）与本仓库
合成的双节点 GLB（unit-2 定位契约）与合成材质 GLB（unit-2 颜色/透明度契约）；**没有任何真实 Revit 模型**，所以真实 RVT 几何/包围盒/
定位/单位在真文件上的行为是 **NOT VERIFIED**，不能由这些夹具推论。SKP 的契约测试是
`cargo test -p cad-connector-skp`，**真实文件证据来自外部 corpus**（`skpwright` 的语料，
未提交、不可再分发）：全语料 763 个 `.skp` ⇒ 728 成功、25 `import.skp.edge_only_geometry`、
10 `import.skp.no_geometry`、0 失败 / 0 panic（**ADR-0049 之前的旧行为扫描**：纯边文件现在导入为
`Partial` wire 体，这组数字在重跑前标 **NOT RE-RUN**，只作历史测量引用，不得当成当前结果）；box 1 实体/12 三角、house 64/1440、
face-with-hole 1/8、instance-scaled 3/36；theater-2017 1246 实体/96,333 三角、theater-2026
1246/96,344，面积 **1020.981869 m²** 一致（相对差 5e-14）。`YACR_TEST_SKP`/`YACR_TEST_SKP_2026`
未设置时的跳过**不是已运行证据**；这些数字是有界语料扫描，不是 golden 回归，也不是产品验收、
性能或跨平台证据（见 `fixtures/skp/SOURCE.md`）。

### 连接器与导出矩阵（2026-11-20）

`cad-app::ConnectorRegistry` 是**唯一**派发表：导入按有序 probe，导出按 `format_id`。每个格式的
`ConnectorDescriptor` 由其实现 crate 声明（`cad-connector-*`），`cad-app` 只负责
接线。未接线的写出一律返回稳定拒绝，**不得**静默成功。

**导出入口（2026-11-23，ADR-0052）**：`EXPORT` 是单个规范命令。裸 `EXPORT` 打开全格式选择器，
建议名取源格式的扩展名（`.dwg`/`.skp` 这两个实验性写回、`.ymodel`/空白/只读源回退 `.dxf`）；
`EXPORT <TOKEN>`（`DXF DWG STL OBJ PLY OFF COFF IFC SKP GLB GLTF`）是它的参数形式，建议该格式的
名字，目标扩展名仍是最终权威，不符在写前以 `export.format_mismatch` 拒绝。所有载体仍共用一个
“全部导出格式”过滤器（`begin_save` 不接收格式）；菜单不按载体隐藏，slim Web 里未编译的格式由
`unsupported-runtime: export.<id>` 显式拒绝。

**按载体编译的子集（2026-11-22）**：表由 `cad-app` 的 `formats-*` features 决定。所有原生载体，以及
Linux X11/XWayland 的 `dom-canvas` wasm（`scripts/build-canvas-wasm.sh` 传 `app-web` 的
`full-formats`），都编译**全部**格式；**只有部署到 Cloudflare Pages 的浏览器 bundle**
（`scripts/build-web.sh`）编译子集：DWG/DXF 导入 + DWG/DXF 导出。其余格式在该产物里按稳定标记拒绝：
`unsupported-runtime: import.<id>`（`step/ifc/dgn/skp/obj/stl/ply/off/gltf/ole`）与
`unsupported-runtime: export.<format_id>`。`descriptors()` 只列出已编译的 lane，`import.rs` 的
“不是图纸”文案在 slim 构建里只列 `.dwg/.dxf`；**不得**把编译掉的 lane 说成成功，也不得让它退化成
`export.unknown_format`（STEP/DGN/RVT 的键仍在 `KNOWN_EXPORT_KEYS` 里）。

| 格式 | 导入 | 导出 | 宿主入口 | 报告 | 证据边界 |
|---|---|---|---|---|---|
| DWG | 实现（AC1009..AC1032） | **实验性**（AC1032，上游 `DwgWriter`） | registry `dwg`；Save As `.dwg` | `CompatReport` + `tables/compat.json` | acadrust 自读；**vendor/AutoCAD NOT VERIFIED** |
| DXF | 实现（AC1009..AC1032） | 实现（lossy，AC1024/1027/1032） | registry `dxf`；`.dxf` | 同上 | 契约测试；**L1 不动点 NOT RUN** |
| STL | ASCII/二进制 | 二进制；ASCII 走 registry `stl-ascii` | `.stl` | 同上 | 导出→`cad-connector-mesh` 自洽往返 |
| OBJ | 实现 | 实现 | `.obj` | 同上 | 导出→`cad-connector-mesh` 自洽往返 |
| PLY | 3 种编码 | 二进制 LE 1.0 | `.ply` | 同上 | 导出→`cad-connector-mesh` 自洽往返 |
| OFF/COFF | ASCII | ASCII（`.off`/`.coff`） | `.off`/`.coff` | 同上 | 导出→`cad-connector-mesh` 自洽往返 |
| IFC4 | IFC2X3/4/4X3 | **IFC4**（`IfcTriangulatedFaceSet`） | `.ifc` | 同上 | `ifc-lite-export` 28.3.0 自洽往返；**vendor/IfcOpenShell NOT VERIFIED** |
| SKP | 预览实现 | **2026 容器** | `.skp` | 同上 | `skpwright` 写出→`cad-connector-skp` 自洽往返；**SketchUp NOT VERIFIED** |
| glTF 2.0 / GLB | 实现（`gltf` 1.4.1 校验 + 手写场景遍历，字节直读；`asset.version` 必须 2.x） | **GLB** + **JSON `.gltf`**（buffer 内嵌 base64 data URI） | `.gltf`/`.glb` | 同上 | **颜色**：`COLOR_0`/`baseColorFactor` 按 glTF 的线性语义处理（导入相乘后转 sRGB，导出 sRGB→线性）；无材质索引保持 `ByLayer`，`alphaMode` 按 OPAQUE/BLEND/MASK 处理；显式实体色/不透明度写成去重 `materials[]`，顶点色覆盖与顶点色+不透明度损失逐条报告。**纹理**：`baseColorTexture` 的 `texCoord` 集合连图像字节（GLB bufferView 或 `data:` URI）导入为 UV0 + 文档纹理，导出写 `TEXCOORD_0`/`images`/`samplers`/`baseColorTexture`；外部图像、`KHR_texture_transform`、MASK、UV1+ 逐条按名报告（见「网格基色纹理」）。**PBR（2026-11-20，ADR-0039）**：导入/导出 metallic-roughness 标量与四张贴图（metallicRoughness/normal/emissive/occlusion）；只有文件确实声明的通道才建 `MeshPbr`（仅 `baseColorFactor`/`baseColorTexture` 的材质保持 `None`），非法因子、`normalTexture.scale != 1`、`KHR_materials_emissive_strength`、非 UV0 槽分别按名报告（见「网格 PBR 通道」）。导出→`cad-connector-gltf` 自洽往返；**three.js/Blender/vendor NOT VERIFIED**。**存储显示边（ADR-0051）**：glTF 没有边通道，写出表面带 `BodyDisplay.edges` 时逐实体记 `export.gltf.stored_edges_not_written`（`Partial`，不改变字节）；wire-only body 整体丢弃记 `export.gltf.wire_body_not_written`（`Unsupported`，不重复记）；无边文档逐字节不变 |
| STEP | 实现（AP203/214/242） | **拒绝** `export.step.not_implemented` | — | descriptor `export: None` | cadmpeg 有写出器（AP242），本轮未接线 |
| DGN | V7/V8 | **拒绝** `export.dgn.not_implemented` | — | 同上 | 上游只有 seed-based V8 写出器 |
| RVT | 预览实现 | **拒绝** `export.rvt.not_implemented` | — | 同上 | 只有预览读入 |
| `.ymodel` | Linux、Android、Windows、macOS | Linux、Android、Windows、macOS | archive | 原生 | Linux/macOS/Windows 临时文件→原子替换 + `.bak`；Android SAF 写**非原子**、`.bak` 仅在 provider 支持时创建（否则 `save.no_backup`）；**Web** `unsupported-runtime: ymodel` |

共同边界：
- **模型空间**：写出只覆盖 `SpaceId::Model`；块定义成员/图纸空间实体逐条记
  `export.<id>.space_not_exported`，不静默混入。
- **单位**：IFC/SKP/glTF 要求已知源单位并换算为米（否则按名拒绝）；STL/OBJ/PLY/OFF 无单位头，
  坐标按存储值写出并附 `export.<id>.units_not_stored`/`units_unknown` 说明。
- **glTF 坐标系与外部资源**：glTF 的规范坐标系是**米、+Y up**；导入时先烘焙节点世界变换，再按
  `(x, y, z) → (x, −z, y)` 转成 CAD 的**米、+Z up**（记 `import.gltf.units_metres` /
  `import.gltf.frame_converted`）；导出反向做同一次换算并要求**已知源单位**，`DrawingUnits`/缺单位
  按名拒绝 `export.gltf.units_unknown`，不假设毫米。buffer 只接受 `data:` base64 或 GLB 自身的 BIN
  chunk；外部 `.bin` 按名拒绝 `import.gltf.external_buffer_unavailable`。base-colour 纹理在字节
  可得时（GLB bufferView 或 `data:` URI）导入为文档纹理 + UV0；仅外部文件图像按名拒绝
  `import.gltf.external_image_unavailable`（UV 保留、纹理缺省、`Partial`）——**绝不联网抓取**任何
  外部资源。
- **报告落盘**：`tables/compat.json` 每条记录含 `connector/direction/level/counts|null/
  completeness/capabilities/entries`；`tables/sources.json` 记录导入来源。导入记录在导入时写入，
  导出记录在 Save As 成功后写入（最多保留 32 条导出记录，导入记录不淘汰）。报告**弹窗 UI 未实现**。
- **格式版本**：两张报告表把 `FORMAT_MINOR` 提到 **2**；网格基色纹理的 `textures/<key>` 条目与
  `cad:Mesh` 的 `uvs`/`texture` 再提到 **3**；网格 PBR 的 `cad:Mesh.pbr` 再提到 **4**；
  外部资源打包的 `resources/{font,xref,image}/<key>` 条目与 `document.json.embed_resources`
  把**可读上限**提到 **5**；BIM 语义的元素行**可选** `bim` 块与 `tables/bim-definitions.json`
  （仅在非空时写）再提到 **6**，多对多关系的 `tables/bim-relations.json`（仅非空时写）再提到 **7**，
  参数化的 `tables/parameters.json` 与元素行**可选** `params` 块（ADR-0043 P0）再提到 **8**；
  P1 的 `neg`/`pow`/`fn`/`if` 表达式与 `tables/configurations.json`、P2 的
  `tables/features.json`、P3 的 `tables/relations.json` 依次提到 **9 / 10 / 11**；
  写入时按文档内容取 **11 / 10 / 9 / 8 / 7 / 5 / 4**（有 in-context 关系 → 11；有特征定义 → 10；
  有 P1 表达式/配置 → 9；有参数 → 8；有 BIM 元素/定义/关系 → 7；只有资源 → 5；否则 → 4）；
  **6 是只读旧档**，本构建能打开 minor-6 归档但再保存一律写当前档。
  minor ≤ 10 的归档仍可打开（缺 `bim` 键 → `None`、缺定义表 → 空定义、缺关系表 → 空关系；
  缺 `params` 键 → 无绑定、缺参数表 → 空参数；缺配置/特征/关系表 → 空表；
  缺 `pbr` → `None`；缺 UV/纹理按空/无解码；缺 `embed_resources` → `false`）；
  minor > 11 的归档按 `ymodel.unsupported_minor` 拒绝打开（不静默丢字段）。BIM 块、定义表与关系表的预算
  （定义/关系各 100 000 行、各 32 MiB 编码体积、单元素 64 PSet / 每 PSet 256 属性 / 字符串 4 KiB）在
  **写读两侧都强制**，超限是显式拒绝而不是截断，稳定标记 `ymodel.bim.invalid` /
  `ymodel.bim.budget_exceeded` / `ymodel.bim.dangling_reference`。
- **载体**：Linux 的 Save As 选择器提供交换格式扩展名；Windows/macOS/Android/Web 的选择器本轮
  未扩展（只能由宿主/测试按显式路径触发，Web 下载流程为 **NOT IMPLEMENTED**），运行证据
  **NOT VERIFIED**。
- **保真等级**：以上写出都是 L1（语义投影）+ 显式损失报告；**L0 源字节直通未实现**；L3 像素
  等同永不承诺。

### 网格基色纹理（UV0 + base-colour texture，2026-11-20，ADR-0038）

范围只有一条能力：**每个网格可带一组 UV0，并引用一张文档自有的基色贴图**。不是 PBR 材质系统，
也不改变"`cad-db` 是唯一权威"。

| 层 | 实现 | 边界 |
|---|---|---|
| 域模型 | `cad_domain::Mesh` 增 `uvs: Vec<[f64;2]>`（空，或与顶点一一对应）与 `texture: Option<String>`（文档键 `<sha256 前 32 hex>.png\|.jpg`） | `uvs` 为空或长度等于顶点数；有纹理必须有 UV，否则 `cad-db::validate` 按 `InvalidInput` 拒绝 |
| 文档资源 | 纹理字节存在 `cad-db`（`DrawingDatabase::textures()`），内容寻址（sha256）、同字节去重 | `ImportLimits` 默认单张 16 MiB、合计 64 MiB、最多 256 张；超限是命名拒绝 `import.texture.too_large`/`budget_exceeded`/`too_many`，非 PNG/JPEG 为 `import.texture.unsupported_format`（不截断、不改写） |
| `.ymodel` | 受管二进制条目 `textures/<key>`（manifest 记 `len`/`sha256` + zip CRC，读取逐条校验，`foreign: false`），`FORMAT_MINOR` **3** | 不写进 `raw/`（外来条目区）；minor ≤ 2 归档仍可打开（缺省空 UV/无纹理）；旧构建读 minor 3 按 `ymodel.unsupported_minor` 拒绝 |
| glTF 导入 | `baseColorTexture.texCoord` 指向的 UV 集 + GLB bufferView / `data:` URI 图像字节 | 外部图像 `import.gltf.external_image_unavailable`；`data:` 载荷非法或 bufferView 越界记 `import.gltf.image_base64`/`import.gltf.image_out_of_range`；外部 buffer、退化 UV、非基色通道、`KHR_texture_transform`、MASK、UV1+ 分别记 `import.gltf.external_buffer_unavailable`/`import.gltf.uvs_dropped`/`import.gltf.texture_channel_not_imported`/`import.gltf.texture_transform_not_imported`/`import.gltf.alpha_mask_not_imported`/`import.gltf.uv_set_ignored` |
| IFC 导入 | `process_element_with_submeshes_textured` + RGBA→确定性 PNG | `IfcImageTexture` 外链不读（`import.ifc.external_texture_unavailable`）；带 void 构件保留裁剪路径并记 `import.ifc.textured_void_fallback`；UV 不配对/编码失败记 `uvs_dropped`/`texture_encode_failed`；`IfcStyledItem` 颜色仍是 `import.ifc.style_not_imported` |
| SKP 导入 | 正面 `uv_xform` + `Material::Textured.image_bytes` | 外部图像 `import.skp.texture_unavailable`、无 UV `import.skp.uv_unavailable`、背面纹理 `import.skp.back_texture_not_imported`、无图像回退平均色 `import.skp.textures_not_sampled` |
| OBJ / PLY | **不导入** | OBJ 声明 `vt`/`mtllib` → `import.obj.texture_not_imported`；PLY 声明 `s`/`t` → `import.ply.texture_coords_not_imported` + `Partial`（整组丢弃，不猜） |
| RVT | **不导入** | `rvt` 0.25.0 不写 `images`/`TEXCOORD_0`；防御性记 `import.rvt.texture_not_imported`；要支持需扩展 ADR-0035 补丁台账 |
| glTF 导出 | 写 `TEXCOORD_0`、内联 `images`（bufferView + `mimeType`）、`samplers`、`materials[].pbrMetallicRoughness.baseColorTexture`；材质去重键 = `(rgb, alpha, texture)` | 无纹理输出逐字节不变；UV/字节缺失时记 `export.gltf.texture_not_written` 并降为 `Partial` |
| 其它导出 | **不写纹理** | STL/OBJ/PLY/OFF/IFC4/SKP/DXF/DWG 的写出器对每个**写出了几何**的带纹理网格各记一条 `export.<id>.texture_not_written`（`stl`/`obj`/`ply`/`off`/`ifc`/`skp`/`dxf`/`dwg`，`Partial`，消息含实体 id 与纹理键），字节与同网格无纹理导出一致；已被丢弃的实体不重复记纹理。这些格式仍**没有**纹理导出 |
| 渲染 | **独立 `mesh_textured` 通道**：UV 在 `@location(3)`，group-2 纹理 + sampler，片元 `sampled × 顶点色 × tint`、alpha blending；既有 mesh/merged/pulled/pick 通道不变 | 纹理批次**永不**进入 flat-merge 或 pulled 通道（一个纹理批次一次 draw，属已知批处理退化）；缺纹理/解码失败按无纹理绘制并记 `image.unresolved`，`Renderer::mesh_texture_fallbacks()` 列出回退键，绝不画占位图 |
| 宿主 | `cad_platform::images::local::load_image_cache_for_document` 从 `cad-db` 字节解码进 `DecodedImageCache`，不做文件系统访问 | Web 载体没有解码器，按名拒绝 `unsupported-resources: images: … document texture(s)`，不静默画成无纹理 |

**ADR-0038 范围内不做**：PBR 其余通道（metallic/roughness/normal/emissive/occlusion；已由
ADR-0039 单独覆盖，见下节）、UV1+ 与 per-corner UV、`KHR_texture_transform`、`alphaMode`
MASK 镂空、mipmap 与 sampler 选项、SKP/IFC 纹理**导出**、RVT 纹理、OBJ/MTL 字节。

**证据边界**：库层/宿主契约测试见 `docs/validation-textures-2026-11-20.md`；真实样本端到端、
three.js/Blender/glTF-Validator/IfcOpenShell/SketchUp 验收、Windows/macOS/Android/Web 运行、
wasm 纹理路径、像素与性能结论一律 **NOT RUN / NOT VERIFIED**。软件渲染（SwiftShader/lavapipe）
不得用作像素或性能证据。

### 网格 PBR 通道（metallic-roughness + normal/emissive/occlusion，2026-11-20，ADR-0039）

范围：每个网格可带一组**源确实声明**的 PBR 通道（标量 + 四张 UV0 贴图），并有一个独立
`mesh_pbr` 着色通道。`Mesh.pbr: Option<MeshPbr>` 的 **`None` 表示源没有声明 PBR**，不是
“默认材质”：glTF 里只有 `baseColorFactor`/`baseColorTexture` 的材质保持 `None`，因为
`metallicFactor = 1` 是规范默认值，套用到无光照 CAD 显示会伪造黑色金属。仍不改变
“`cad-db` 是唯一权威”。

| 层 | 实现 | 边界 |
|---|---|---|
| 域模型 | `cad_domain::MeshPbr { metallic, roughness, emissive(线性 RGB), occlusion_strength, metallic_roughness_texture, normal_texture, emissive_texture, occlusion_texture }`，挂在 `Mesh.pbr` | 标量有限且 ∈ `[0,1]`；纹理键是文档键（与基色纹理同规则）；任一 PBR 贴图要求 `uvs` 非空，否则 `cad-db::validate` 按 `InvalidInput` 拒绝；`None` 不填规范默认值 |
| `.ymodel` | `cad:Mesh.pbr` 可选嵌套对象（`null`/缺失 → `None`），`FieldKind::MeshPbr`，`FORMAT_MINOR` **4** | 纹理复用 `textures/<key>`；minor ≤ 3 归档可读（缺 `pbr` → `None`）；旧构建读 4 按 `ymodel.unsupported_minor` 拒绝 |
| glTF 导入 | 读 4 张 PBR 贴图与 3 个标量（加上既有基色通道共 5 条），仅限原始 JSON 声明的槽；图像字节走既有 bufferView/`data:` URI 路径 | 逐槽 `texCoord != 0` → 该槽不导入 + `import.gltf.uv_set_ignored`；`normalTexture.scale != 1` → `import.gltf.normal_scale_not_imported`；`KHR_materials_emissive_strength` → `import.gltf.emissive_strength_not_imported`；非法因子 → `import.gltf.pbr_ignored`；外部图像按名拒绝，绝不联网 |
| glTF 导出 | 写 `metallicFactor`/`roughnessFactor`/`metallicRoughnessTexture`/`normalTexture(scale 1)`/`occlusionTexture(strength)`/`emissiveTexture`/`emissiveFactor`；材质去重键含整个 PBR 元组 | `pbr: None` 的网格输出逐字节不变（不写任何 PBR 键） |
| RVT | 读 `metallicFactor`/`roughnessFactor`（上游 0.25.0 按材质名启发式），建 `MeshPbr` + 每文档一条 `import.rvt.pbr_heuristic` note | 非有限/越界 → 不建 `MeshPbr` + `import.rvt.pbr_ignored`（`Partial`）；RVT 仍无纹理/UV，PBR 贴图恒缺省 |
| 其它格式 | IFC/SKP/OBJ 一律 `pbr: None`；IFC 样式颜色仍是 `import.ifc.style_not_imported`，SKP/OBJ 无 PBR 数据不产生 PBR 诊断 | 非 glTF 写出器仍**不写** PBR；也**没有**独立的 PBR 丢失码——基色纹理损失由既有 `export.<id>.texture_not_written` 覆盖，PBR 标量/贴图不在其消息内 |
| 渲染 | **独立 `mesh_pbr` 通道**：Cook-Torrance GGX，单方向光 `camera.light`，`F0 = mix(0.04, base, metallic)`，`diffuse = base*(1-metallic)`，metallicRoughness 的 G/B 缩放 roughness/metallic，occlusion 只乘漫反射与环境项，emissive 相加，再乘顶点色与批次 tint，沿用 ink-flip/sRGB 约定。**IBL（2026-11-21，ADR-0044）**：**程序化解析天空**（split-sum + 解析 BRDF 近似，WGSL 内联计算，无环境贴图资产），**只作用于 `mesh_pbr` 通道**；会话开关命令 `IBL`（裸 `IBL` 翻转，`IBL ON\|OFF\|1\|0` 显式），**默认开**；IBL 开时 AO 同时乘间接镜面；**OFF = 旧单方向光 + 常数环境光 `AMBIENT = 0.25`，逐字相同并由 fixture 锁定**；`pbr: None` 的网格仍走 `mesh_textured`，**没有 IBL**。**HDR 与色调映射（ADR-0044 §6，2026-11-21）**：IBL 路径的环境是 **HDR**（天顶≈1.66 / 地平线≈1.34 / 地面≈0.30 的亮度，外加暖白太阳盘 `pow(dot,900)*60` 与宽辉光 `pow(dot,24)*2.4`），并在光照求和后、写 sRGB 目标前应用 **ACES 拟合曲线**（命名曝光 `IBL_EXPOSURE`），alpha 不变；**IBL OFF 仍是旧 LDR 数值 + 无 tonemap 的 legacy 路径，逐字节不变** | 切线由 CPU 按 UV 生成；退化 UV 关闭法线通道并记 `render.pbr.tangent_unavailable`（**不**发明切线）；缺失通道绑定精确恒等纹理（白/黑/平面法线）＝“通道关闭”；PBR 批次**永不** flat-merge 或进入 pulled；原地覆盖拒绝 `pbr_not_updatable`；PBR 网格（`pbr.is_some()`）**不生成三角边叠加**（`edges` 为空，与 `MeshWithoutEdgeOverlay` 同形；导入资产不携带线框，非静默抑制）；IBL 是会话态：不写文档、不进 ChangeSet/撤销、不升 `FORMAT_MINOR`；环境**相机相对**（`camera.light` 是头灯，太阳随视角移动，已声明限制）；**tonemap 只属于 IBL 路径**——`pbr: None` 的网格与 IBL OFF 的输出不做色调映射，曲线也不碰选择/悬停 tint（UI 颜色）。**采样 wrap（ADR-0044 §7，2026-11-21）**：网格材质纹理（基色 + 四张 PBR 贴图）按 glTF 2.0 的默认 **REPEAT** 采样（U/V = `Repeat`，W 为 `ClampToEdge` 但二维采样从不使用）；导入端不把 `wrapS`/`wrapT` 存进 `Mesh`/`MeshPbr`，因此文档声明非 REPEAT 的槽（含镜像）在字节导入成功后按 `import.gltf.sampler_wrap_not_imported` **按名报告而不是静默改义**（`Partial`）；缺省 sampler / `{}` / 显式 `10497` 是 REPEAT，不产生诊断。CAD 栅格图像四边形仍用独立的 `image_sampler`，**保持 `ClampToEdge`** |
| 宿主 | `SceneBatch.pbr` 携带 4 个额外贴图键；`mesh_texture_keys` 收集全部 5 个键，`load_image_cache_for_document` 按 `db.textures()` 全量解码 | 无占位图：缺键按无贴图通道处理并报告；Web 载体仍按名拒绝文档纹理，不静默画成无纹理 |

**明确不在本轮范围**：阴影、多光源、`KHR_materials_*`、`alphaMode = MASK` 镂空、
UV1+/`KHR_texture_transform`、SKP/IFC/OBJ 的 PBR、RVT 纹理、非 glTF 导出器的 PBR
（IBL/环境贴图已由 [ADR-0044](adr/0044-procedural-ibl.md) 覆盖，见上表的「渲染」行；tonemap
已按 ADR-0044 §6 收窄：**只** IBL 路径做 ACES 曲线，legacy 与非 PBR 通道仍不做）。

**证据边界**：契约测试与真实 GPU 用例见 `docs/validation-pbr-2026-11-20.md`，IBL 的
**契约测试 + 真实 GPU 定点用例**与 HDR/色调映射前后的实测亮度表记在
`docs/validation-ibl-2026-11-21.md`（逐条状态与证据层级以该文件为准）；合成
`pbr-channels.glb` 的端到端渲染记录 **待补**；three.js/Blender/glTF-Validator 等厂商验收、
Windows/macOS/Android/Web 运行、**真实 GPU 像素 / Metal / DX12 / Android / 浏览器 GPU**、
像素/golden/性能结论一律 **NOT RUN / NOT VERIFIED**。
软件渲染（SwiftShader/lavapipe）与 headless readback 只证明公式与接线，不是产品验收，
不得用作像素或性能证据。

### 外部资源打包（自包含 `.ymodel`，2026-11-20，ADR-0040）

`.ymodel` 可选把**字体、外部参照源文件、栅格 `IMAGE` 字节**打进同一个归档，使单文件可换机打开。
开关是**文档属性** `document.json.embed_resources`（布尔，仅 `true` 时写；缺省 `false`），
保存对话框在保存前询问并列出探测到的资源与合计大小。网格贴图（`textures/<key>`）本来就自包含。

| 层 | 实现 | 边界 |
|---|---|---|
| 归档布局 | `resources/{font,xref,image}/<32 lowercase hex>.<ext>`，内容寻址，与 `textures/<key>` 同构；`manifest` 记 `len`/`sha256`/zip CRC，逐条校验 | 键必须等于 `sha256(bytes)` 前 32 hex，写入与读取两侧都断言；`ext` 白名单外拒绝（`resource.unsupported_kind`）；listed 但不合法的条目按 `ymodel.unsupported_minor` 拒绝，不当外来字节静默保留 |
| 逻辑映射 | `tables/resources.json` = 纯 JSON **数组** `[{key,kind,name,source}]`；`name` 与被引用处逐字一致，`source` 仅供诊断 | 表存在 ⟺ 确有资源字节（空表不写）；表与快照映射不一致时**拒绝保存**，不做"表赢"；同名不同字节 `resource.name_conflict` |
| 版本 | `FORMAT_MINOR` 上限 **5**，写入按文档取 **4 或 5**（无资源且未开开关 → 4） | minor ≤ 4 归档正常打开（无该字段 → `false`）；带资源的 5 被旧构建按 `ymodel.unsupported_minor` 拒绝——这是设计意图 |
| 增量 | 未改动的资源条目按上版归档 raw-copy（字节不改写）；与未列出的同路径外来条目碰撞时按**实际字节摘要**比对，相同则接管、不同则 `resource.conflict` | 首次流写 → raw-copy 会重写 zip local header，所以字节不变性只从第二次保存起成立（既有行为，测试按实测断言） |
| 预算 | 单条 32 MiB、单次保存合计 64 MiB；超限/读不到**跳过该条并报告**，不让保存失败；读取侧同样有单条上限 | 标记 `resource.budget.entry` / `budget.total` / `unavailable`；采集结果是本轮保存状态，**不写** `tables/compat.json`（该表只留最多 32 条既往导出记录） |
| 解析优先级 | 内嵌 > 宿主（字体目录/图纸目录/`images/`）> 显式缺失 | 内嵌必须优先；宿主文件更新时不"取较新者"。内嵌按**逻辑键**命中（`IMAGE` 用 `image_resource_key(name)`，绕过绝对路径拒绝），归档键只是存储 |
| 恢复快照 | 恢复归档**强制**不打包资源（`recovery.resources_omitted` 显式记录） | 恢复文件每 60 s 写一次；恢复路径的 `previous` 是 `None`，沿用文档策略会周期性全量重写 |
| 载体 | **Linux/Windows/macOS/Android** 已接线（Linux `ymodel_archive.rs`；Windows COM `IFileSaveDialog` + `ReplaceFileW`；macOS `NSSavePanel` + `rename`；Android SAF 非原子写 + `save.no_backup`）；字体目录设置与目录选择器只在 `native-below` 可用 | Web 打开/保存 `.ymodel` 仍 `unsupported-runtime: ymodel`；Windows/macOS/Android 的真实运行/对话框/`.bak`/恢复快照一律 **NOT VERIFIED**（见 AGENTS 平台表）；`dom-canvas`（Web 与 X11）不能读本地目录，字体目录行渲染为不可用，写设置按 `unsupported-runtime: fontsDir` 拒绝 |

**明确不做**：解压到磁盘（字节只在内存流转）、把外部参照变成活链接（内嵌源只保证重新导入/导出
保真，`XREF RELOAD` 未实现）、字体子集化与授权校验、加密/签名/口令、`raw/source-*.bin` 的 L0 直通。

**证据边界**：库层与宿主契约测试见 `docs/validation-resources-2026-11-20.md`；真实 DWG 跨机自包含、
字体度量一致性、外部参照重新导入保真 **未运行**；目录选择器的真实 portal 交互 **NOT VERIFIED**；
Windows/macOS/Android 仅编译。

### 保存与导出分离、非公开格式来源提示（2026-11-20，ADR-0041）

`SAVE` 与"导出其它格式"不再混为一个动作：

| 层 | 实现 | 边界 |
|---|---|---|
| `SAVE` / `SAVEAS` / `QSAVE` | **只写原生 `.ymodel`**，默认名 `<stem>.ymodel`，选择器只给 `*.ymodel` | 无归档适配器的载体在**打开选择器之前**就以 `unsupported-runtime: ymodel` 失败（不弹框、不写盘、不静默退回 DXF） |
| `EXPORT` | 真命令（不再是 `PLOT` 别名）；选择器含 dxf/dwg/stl/obj/ply/off/coff/ifc/skp/glb/gltf，**目标扩展名决定写出器** | `PLOT`/`PRINT` 仍是矢量 SVG/PDF，行为不变；`export.step|dgn|rvt.not_implemented` 仍按名拒绝 |
| 覆盖源文件 | **允许**（不再预填 `<stem>-export.dwg`、不再弹 DWG 确认框、不再有 `save.refused_source_path` 拦截） | 安全性由原子写的 `.bak` 兜底（Linux `atomic_replace` 对所有 `write_atomic` 写出生效，已有契约测试断言旧字节在 `.bak` 里）；没有原子替换能力的载体必须显式说"没有备份"，不得假装有 |
| 打开提示 | `cad_status.openedSource`（仅 `spec = reverse_engineered` 时出现），非模态横幅；**“知道了”只关本次**，**“不再提示”写宿主 `formatNoticeEnabled=false` 并在读回确认为 `false` 后才关闭** | 四个逆向连接器：DWG、DGN、SKP、RVT（`SpecVisibility` + `ExportCapability.experimental` 为唯一来源）；**全局开关**（Options → 用户页）关闭 = 对四种格式全部静默，重新打开 = 恢复所有提示（无按格式记录）；持久化为各载体 `preference.json` 的**根级布尔**（默认 `true`；Linux/Windows/macOS/Android 落盘，Web/X11 `dom-canvas` 是内存态、刷新后回到默认）；写入未被宿主读回确认时横幅保留并显示 `role="alert"` 原因，**不假关闭**；DXF/STEP/IFC/glTF/网格格式为公开规范，不提示；`.ymodel` 与 New/Close 清除该字段 |

横幅文案按格式分开（`open.notice.detail.{dwg,dgn,skp,rvt}`），并如实说明回存能力：DWG/SKP 写出为实验性，
DGN/RVT 无写出器。**不阻止打开**，也不在保存时重复弹窗。

**证据边界**：命令/状态/横幅均为代码路径 + 契约测试；"导出可能无法被原软件打开、会丢信息" 是依据本仓库
既有 vendor `NOT VERIFIED` 结论的保守表述，**不是**逐格式的实测兼容性结论。真实 AutoCAD / MicroStation /
SketchUp / Revit 打开我们导出的文件一律 **NOT RUN**。

**明确不做**：导出预设、批量导出、`EXPORTOFFICE`、真 `WBLOCK`、L0 源字节直通、导出后保持源文件其余部分不变。

### BIM 语义（构件 / 类型 / 楼层 / 材质 / 属性 / 关系 / 查询，2026-11-21，ADR-0042 Phase 1–3）

范围是：**每个元素可带一份源里确实存在的 BIM 语义，可只读查看、按类别/楼层/类型/材质/属性查询，
并能按语义回写 IFC**。它不改变"`cad-db` 是唯一权威"，也不引入 EC Schema/ECSQL 或跨文件联邦。
天花板：**IFC 导入/导出 `Partial`**（契约证据 + 自洽往返，无 vendor checker）；**RVT/DGN 语义
`Unverified`**。**IFC 关系只导出 `contains`/`nests`**（`aggregates`/`group`/`connects`/
`other:*` 因 plan API 不可表达而不导出并按 kind 报告）；BIM 查询经**通用 host-read seam**
（`cad_ui_core::host_read`：`HostReadRequest{kind:"bimQuery"}` → `HostReadAnswer`；请求携带型读取
共用一条 seam，新增 kind 不再新增每载体文件）在 `cad-host`（native）与 Web（wasm `DirectReads`）
已有接线（运行时 **NOT VERIFIED**）；四个 native 载体都装同一条 seam（Linux 有运行证据，
Windows/macOS/Android 只有编译/契约证据，见 `docs/validation-bim-2026-11-21.md` §17），没有 seam 的
宿主/kind 按 `unsupported-runtime: bimQuery`（未知 kind 为 `unsupported-runtime: hostRead.kind`）
显式拒绝。

| 层 | 实现 | 边界 |
|---|---|---|
| 存储 | 元素行**可选** `bim` 块（`category/name/source{connector,document_id,id,native_id}/level/type/material/psets/evidence`）；`tables/bim-definitions.json` 与 `tables/bim-relations.json` 仅在非空时写 | `None` 时**省略整键**（不写 `"bim": null`）；`value` 只允许 `text\|real{ifc_type,unit}\|integer\|bool\|null`；Complex/枚举/有界/Table 不压字符串 |
| 关系（Phase 3） | `tables/bim-relations.json`：`kind ∈ aggregates\|nests\|contains\|group\|connects\|other:<token>`；`parent`/`child`/`source` 复用 `BimKey`（`source{connector,document_id,id}` 或 `entity{entityId}`）；`source`/`label` 恒写键，缺省 `null` | 端点**不要求**解析到现存元素；`(kind,parent,child)` 重复在写/读两侧拒绝；无 `IdKind` 12/13、不占高水位 |
| 定义 id | `level:<connector>:<doc>:<key>` / `type:…` / `material:…` 复合字符串 | **不是** `PersistentId`：不进 `IdKind`、不占高水位；`IdKind` 11/12/13 **未启用** |
| 版本 | `FORMAT_MINOR` **7**，写入按文档取 **7/5/4**（有 BIM → 7；只有资源 → 5；否则 → 4）；6 为只读旧档 | 旧构建读 7 按 `ymodel.unsupported_minor` 拒绝；新构建读 minor ≤ 6 时缺 `bim` → `None`、缺定义表 → 空定义、缺关系表 → 空关系 |
| 预算 | 定义 100 000 行、定义表编码 32 MiB、关系 100 000 行、关系表编码 32 MiB、单元素 64 PSet、每 PSet 256 属性、字符串 4 KiB；**写读两侧都强制** | 超限**显式拒绝**（不截断）：`ymodel.bim.invalid` / `ymodel.bim.budget_exceeded` / `ymodel.bim.dangling_reference` |
| IFC 导入 | `crates/cad-connector-ifc/src/semantics.rs` 第二遍扫描：GlobalId/Name/Tag、项目 id、楼层（elevation 换算到米）、层级归属（≤16 边、防环）、类型 + 类型 PSet、实例 PSet、材质；`IfcSpace` 是普通元素 | `Partial`：Complex/枚举/有界/Table 只计数（`import.ifc.bim.property_value_unsupported`），样式颜色仍是 `import.ifc.style_not_imported`；只有几何的 IFC 仍 `Complete` |
| IFC 数量集（Phase 3） | `crates/cad-connector-ifc/src/semantics/quantity.rs`：`IfcElementQuantity` 以源名（`Qto_*`）存进 `bim.psets` | Length/Area/Volume/Weight/Time → `real{value, ifc_type: 源实体名, unit}`、Count → `integer`；**不换算**；单位解析不到 `unit: None`；`import.ifc.bim.quantity_set_stored` 信息性、不降级；`IfcPhysicalComplexQuantity` 等不压平 |
| IFC 关系（Phase 3） | `tables/bim-relations.json`（仅非空时写）：`aggregates\|nests\|contains\|group\|connects`；端点 = 项目 + 对象 GlobalId，关系 GlobalId/Name → `source`/`label` | 端点缺 GlobalId 跳过 + `relation_endpoint_missing`；超预算整表拒绝 `relation_budget_exceeded`；**仅存在关系不降级**；**导出只写 `contains`/`nests`**，`aggregates`/`group`/`connects`/`other:*` 报 `export.ifc.bim.relation_not_written` |
| IFC 导出 | **已回写（Phase 2）**：`database.bim_elements()` 非空时写 `IfcBuildingStorey`（按 `(elevation,name,id)`、米）、`IfcClass::Named(category)`（白名单 + 内置 IFC4 registry 对照，未知回退 Proxy）、`<Category>Type`（白名单，class 不一致跳过）、`IfcMaterial`（按名去重）、PSet、源 `GlobalId` 复用（22 字符 IFC base64 → `u128`，非法/重复回退 `stable_global_id`） | **无 BIM/无关系路径逐字节不变**（既有夹具 2196 / 1374 字节，只出现 `IfcBuildingElementProxy`）；**关系导出（2026-11-21 补齐）**：`bim_relations.rs` 只写 `contains`（覆盖默认楼层容器）与 `nests`（`IfcRelNests`，宿主先写、环丢边计数），`aggregates`/`group`/`connects`/`other:*` 不伪造、按 kind 报 `export.ifc.bim.relation_not_written`，端点未导出跳过并计 `relation_endpoint_missing`；`BimValue::Text` 无 `ifc_type` 一律写 `Label`；未知单位/`Null` 不写出并计数；诊断 `export.ifc.bim.*`；证据仅自洽往返，**无 vendor checker** |
| RVT | **已导入（Phase 2，`Unverified`）**：`semantics.rs` 在 `model_to_glb` 前映射 `IfcModel`，GLB `entityIndex` 对齐几何行；category/name/source/level/type/material/psets，`type_guid` → `native_id`，`global_ids.document` 不用 | `Aggregate` parts 不归组并计数；8 个 `import.rvt.bim.*` 稳定码；真实样本（`rmeadvancedsampleproject.rvt`）观察 `elements=6783, levels=4, types=90, materials=0, element_psets=44928`——**契约观察不是验收**，真实 RVT BIM 语义 **NOT VERIFIED** |
| DGN | **已落库为 PSet（Phase 3，`Unverified`）**：tags/Item Types 按 `set` 合成 PSet（无 set → `DGN_TAGS`），`evidence=Declared`；仅 `Category`/`IfcType`/`IFCEntity` 标签的单个 token 写 `bim.category` | 浮点保持 `f64`；**level/cell/几何不推断**；`import.dgn.bim.*` + `tags_not_stored` 仅剩「确实无法解码/无行」；上游 tag 解码未在真实 V8i/CONNECT 文件验证 |
| 属性面板 | `cad-app/src/selection_bim.rs`：`PropertyGroup::Bim` + 7 个固定静态键（category/name/level/type/material/source/evidence）+ 每个非 null PSet 属性一条动态行 `bim.pset.<set>.<prop>`（上限 64 行/元素）；无 BIM 元素零行。**BIM 面板（Phase 3）**：`components/BimPanel.tsx` + `src/bim/`，非模态浮窗（>1024px）/ 全宽底部抽屉（≤1024px） | 动态标量行在选中引用直接可寻址时 `editable = true`（写入由宿主执行），7 个静态行恒只读；`Null` 不产生行；引用解析失败显示原始 id；`bim_evidence` 的**值**目前显示 `declared`/`heuristic` 机器 token |
| 查询/过滤/明细表 | **已接线（Phase 3）**：`crates/cad-query/src/bim.rs` 按 `(DatabaseId, Revision)` 懒建 category/level/type/material/property 索引（成员 `BimKey`，统计按 `BimKey` 去重）；facet 间 AND、facet 内 OR，属性算子 `Eq/Ne/Contains`；wire 冻结 camelCase 契约；UI `src/bim/` + `components/BimPanel.tsx`（chip 点击即筛选、150ms 防抖、generation 丢弃过期响应） | term 上限 64（`bim.filter.too_many_terms`）；行排序确定（category,name,source id,最低 EntityId,BimKey）；UI 区分「无 BIM 数据/过滤无匹配/无图纸/载体不支持/会话未就绪/stale」；运行时 **NOT VERIFIED** |
| `cad_bim_query`（页 + `summary`，**一次往返**） | `crates/cad-ui-core/src/bim.rs`（冻结 camelCase wire；`summary` 块随查询应答返回，`summary.total == total`）+ `session/bim.rs`；`cad-host/src/bim.rs`（11 条契约，一次绑定同时算 page 与 summary，`BimQueryHost`/`BimQueryReads`/`BimQuerySeam`）；`cad-desktop-shell` 命令 + 注册表 fixture（**`cad_bim_summary` 已删除**）；Web `DirectReads`（5 条测试）；四载体 `bim_host.rs` seam；React `bim-store` 每次动作只发一个 `invoke` | **Linux native（`cad-host`）与 Web 已接线**，运行时 NOT VERIFIED；**Windows 仅交叉编译**；**macOS/Android 仅编译/契约，运行时 NOT VERIFIED**；未装 seam 的载体仍按 `unsupported-runtime: bimQuery` 拒绝（不是空页） |
| `cad_select_entities` | payload **`{entityIds: number[]}`**（camelCase 已用测试钉住）；空数组 → `selection.empty`（**不清空**）；未知 id → 整单拒绝 `selection.unknown_entity: <id>`（不部分选中）；无文档 → `selection.no_document`；无选择缝 → `unsupported-runtime: cad_select_entities` | Linux 原生 / X11(`dom-canvas`) / Web / Windows/macOS/Android 的 tick 均已接线（Linux/X11/Web 有契约测试；Windows/macOS/Android 仅源码一致性，**NOT VERIFIED**） |

**证据边界**：BIM 映射只有**合成 IFC 夹具**的库层 + 宿主端到端契约、**单个 RVT 样本的代码契约观察**、
合成 `cadkit_core::Document` 的 DGN 契约，以及 IFC 导出**自洽往返**（`docs/validation-bim-2026-11-21.md`）；
真实 IFC 文件（`YACR_TEST_IFC` 未设）、Revit/Bentley/IfcOpenShell 交叉验证、真实 RVT BIM 语义验收、
DGN 真实 V8i/CONNECT 的 tag 解码、Windows/macOS/Android 的 BIM 查询与 `cad_select_entities` 运行、
wasm/Android 运行时、设备/GPU/像素与跨机自包含一律 **NOT RUN / NOT VERIFIED**；**IFC 关系导出只覆盖 `contains`/`nests`**（其余 kind 显式报告，无产物）。
RVT 那份样本计数是**契约观察，不是验收**。

### 参数化依赖与求值（2026-11-21，ADR-0043 P0）

范围是「改一个命名参数，直接绑定它的构件按依赖顺序重算」的**最小垂直切片**；它不是特征图、约束求解器、
配置或自上而下外部参考（ADR-0043 D14）。`cad-db` 仍是唯一权威：参数表与元素槽位绑定是权威，几何与
显示是派生；打开时重算并诊断，但**不在打开时写回**。

| 层 | 实现 | 边界 |
|---|---|---|
| 存储 | `tables/parameters.json`：`{seq,name,kind,unit?,expr}`，`seq` 是表内身份（非 `PersistentId`、不占 `IdKind` 12/13 或高水位）；元素行可选 `params` 块（`slot → expr`），`None`/空时**省略整键** | 名称文档内唯一；重复名、未知槽位、悬空 `ref`、环、超预算一律显式拒绝（`param.*`）；`"params": null` 拒绝 |
| 版本 | `FORMAT_MINOR` **11**，写入按文档取 **11/10/9/8/7/5/4**（有 in-context 关系 → 11；有特征定义 → 10；有 P1 表达式/配置 → 9；有参数 → 8；有 BIM → 7；只有资源 → 5；否则 4）；10 及以下可打开 | 旧构建读 11 按 `ymodel.unsupported_minor` 拒绝；新构建读旧档缺表 = 空表、缺块 = 无绑定 |
| 表达式（P0/P1） | P0：`{"num"} / {"ref"} / {"op": add\|sub\|mul\|div} / {"const":"pi"}`；P1：`{"neg"} / {"pow","exp"} / {"fn","arg"} / {"if":{cond,then,else}}`（`abs/sqrt/sin/cos/tan/floor/ceil/round`）。量纲在解析/求值期检查，规范单位 `mm`/`rad`，不静默换算；`number` 参数无量纲，可缩放长度；`pow`/`sqrt` 只接受无量纲操作数；`if` 只求值被选中的分支 | 深度 ≤ 32、单表达式节点 ≤ 256、参数 ≤ 4096、闭包 ≤ 10 000 元素；超限 `param.budget_exceeded`，**整单拒绝**；未知 `fn`/多余 tag/缺 `exp`/`arg` 在解码期拒绝 |
| 配置（P1） | `tables/configurations.json`：`{name,parent,overrides{seq:expr},suppressed[seq]}`；父链先应用、子覆盖；派生表 = 基表 + 活动配置；活动配置是**运行时状态**（不落盘），`PARAM CONFIG` 切换并重算全部绑定元素（一 revision/ChangeSet，**不记 undo**）；`param.<name>` 在活动配置覆盖/抑制该参数时按 `param.config_override` 拒绝 | 未知父/环 `param.config_cycle`/`param.unknown_config`；覆盖/抑制不存在的 `seq` 拒绝；抑制被其它表达式引用 → 配置无效；绑定到被抑制参数 → 元素 `param.suppressed` 保留 last-good |
| 槽位（P0/P1） | `cad:Circle.radius`、`cad:Polyline.width`、`cad:Text.height`（均不改变拓扑） | `cad:Insert.scale` **未实现**（需要 transform/base 拆分）：按 `param.no_evaluator` 显式拒绝；其它类型/槽位 `param.no_evaluator` / `param.unknown_slot`，不猜 |
| 写入 | `DrawingDatabase::apply_parametric_update`：先全量校验 → 再改 → **一个 revision / 一个 ChangeSet / 一条 undo**；`ChangeScope` 同时标 `TableKind::Parameters` 与每个被重写元素的 shard | 空批不 bump；锁定层按 `layer.locked` 拒绝；几何构造失败只发 `param.blocked` 并保留 last-good，参数仍提交（D6 第二档） |
| 打开 | 按 D5 重算并与存储几何比较：不一致只发 `param.recomputed`，构造失败发 `param.blocked`，**不修改、不 bump、不产生脏 scope** | 存储行是上一次成功求值结果（D7）；诊断目前只在 `cad-db` 可读（`param_diagnostics()`），宿主 UI 尚未展示 |
| undo/redo | 参数 patch 与绑定 patch 同一步；绑定 patch 携带 `before/after_geometry`，undo 恢复精确的绑定前几何（值编辑由纯函数重算） | 参数步拒绝与 record 实体行/其它表批次的混合（实现说明见 ADR-0043 D8）；`PARAM CONFIG` 不记 undo |
| 命令 | `PARAM {op: create\|delete\|rename\|bind\|unbind\|config}`（Work-only，一命令一事务；config 例外：一 revision 但无 undo）；值编辑走 `SETENTITYPROPERTY` 的 `param.<name>` | **`PARAM` 只有命令层**：没有 payload producer / IPC / UI 路径，生产可达的写入是 `param.<name>`；多选时每个选中元素都必须直接引用该参数，否则 `param.mixed` 整单拒绝；无变化不记历史 |
| 面板 | 动态行 `param.<name>`（`PropertyGroup::Param`，label = 参数名，值 = 求值结果），只列当前选择**直接引用**的参数；多选按 key 交集，部分绑定不出现该行 | P0/P1 不做文档级参数面板；`paramList` 只有 wire（`host_read` 新 kind，见下），没有 React 消费者 |
| `paramList`（P1 host-read） | 唯一 `cad_ui_core::host_read` seam 的新 kind：`HostReadRequest::ParamList` → `HostReadAnswer::ParamList`；`cad-host::HostCore::param_list()` 返回 base/effective 值、`inherited/overridden/suppressed` 状态与活动配置名；无文档 = 空表 | 无 seam 的载体按 `unsupported-runtime: paramList` 拒绝（未知 kind 仍是 `hostRead.kind`）；只有 Linux 编译/契约证据 |
| 特征（P2，schema only） | `tables/features.json`：`{id(元素),order,operation,inputs,parameters}`；`order` 唯一、按序求值；操作枚举含 extrude/revolve/loft/sweep/boolean_*/fillet/shell/hole/pattern；构建期校验元素/输入存在、无自输入、槽位可解析 | **没有求值器**：`evaluate_feature` 对所有操作（含 fillet/shell）返回 `rel.not_implemented`，不伪造几何、不 bump revision；solidcore 接入是后续工作 |
| `voids`/`fills`（P2） | 作为 `BimRelationKind` 具名 kind 导入/存储；D1 归类为 Reference，`relation_propagation` 返回 `NoRule`（`rel.no_rule`）；IFC 导出仍只写 `contains`/`nests`，它们按 kind 计 `export.ifc.bim.relation_not_written` | **(host,hosted) 规则未注册**：没有“墙厚变了门跟着动”的传播；不得当成参数化更新 |
| in-context（P3，schema only） | `tables/relations.json`：`{source,target,reference{kind:face\|edge\|vertex,key},state,label}`；`(source,target,reference)` 唯一；状态机 `InContext/OutOfContext/Locked/Broken/Dangling`，`Dangling` 终态；`update_relation_state` 一 revision + `TableKind::Relations` scope | **没有几何传播**：表是显式 typed 记录，不是可执行的 rebuild；`cad-dependencies` 未接入统一调度 |

**仍明确未实现**：`Insert.scale`（缺少 transform/base 拆分，按 `param.no_evaluator` 拒绝）、文本模板
（`cad:Text.text` 的 `{name}` 占位）、文档级参数面板的 React 消费者、P2 的特征求值器（solidcore 内核
接入）、`voids`/`fills` 的 `(host,hosted)` 传播规则、P3 的几何 in-context 传播与 `cad-dependencies`
统一调度。**二维几何/尺寸约束求解器**已由 ADR-0054（2026-11-24）放开为有界支持域；在实现与契约
测试落地前，`GEOMCONSTRAINT`/`DIMCONSTRAINT`/`DC*`/`DELCONSTRAINT`/`AUTOCONSTRAIN`/
`CONSTRAINTBAR`/`CONSTRAINTSETTINGS` 仍是具名拒绝。三维配合/装配求解、全局优先级、特征历史
rollback、跨文档外部参考仍是**非目标**（ADR-0043 D14 由 ADR-0054 收窄）。

**证据边界**：见 `docs/validation-parametric-2026-11-21.md`。只有库层/命令层契约与单元测试、
`cargo check`/clippy/门禁、以及无 GPU 的 headless 求值测试；**没有**真实 GPU/像素/性能、真实宿主运行、
跨机自包含或 vendor 验收。Linux 是唯一有测试运行的载体，其余平台 **NOT VERIFIED**。

### 测量命令与 §3.3 的逐条对应（2026-10-26 对账补齐）

`spec §3.3` 的测量条款此前只有零散行，**没有**逐条「要求 ↔ 实现」对照，读表会误判。按源码与契约测试
（非像素证据）对账如下：

| §3.3 条款 | 当前实现 | 证据 / 边界 |
|---|---|---|
| 自交多边形面积「拒绝或明确规定算法」 | **拒绝**（不返回数字） | `crates/cad-geometry/src/area.rs:73`（`self_intersects` 判定）→ `AreaError::SelfIntersecting`；`crates/cad-measure/src/engine.rs:100-111` 映射为 `InvalidInput("area rejected: …")`；测试见 `docs/validation.md:149` |
| 三点角度 | **实现**（顶点 = 第 2 点，按活动空间/工作平面求角） | `crates/cad-measure/src/engine.rs:91-99`（`Angle3Points` + `angle_at_vertex`）；命令面 `MEASUREANGLE` |
| 纸空间 / 视口反变换 | 纸面直接测量；`ViewportModel` **需有效反变换**，否则**禁用**模型测量；纸空间 `ANGLE` 显式拒绝 | `crates/cad-app/src/space.rs:52-116`、`crates/cad-measure/src/engine.rs:138-141` |
| 显示 LOD 不影响计算 / 代理标记缓存几何 | 实现（SPLINE 解析/采样与显示 LOD 解耦；天正代理标缓存几何） | 见上表 SPLINE 行与「天正/探索者代理」行 |
| **源单位未知 → 显示「图纸单位」** | **已实现（2026-10-26 补齐）**：测量与点回显的 `unit: null`（宿主对 `Unit::DrawingUnits` 的唯一投影）现在渲染目录键 `units.unknown`（图纸单位 / “Drawing units”），面积按其平方模板得 `图纸单位²`；`angle` 仍只带 `°`（角度自带单位，不是未知源单位）。**区分保留**：字段缺失/非字符串/空串仍**不**加后缀——那是坏行，不是宿主声明「图纸单位」 | `crates/cad-host/src/command_log.rs:183-192`（`DrawingUnits → None`）、`apps/desktop-ui/src/commands/command-log.ts:144-147`（`unitLabel`）、`:154-157`（`areaUnit` 平方同一个标签）、ID 点行同路径；测试 `apps/desktop-ui/test/command-log.test.ts`「an unknown source unit reads as the drawing-units label in the active language」 |

> 该条**已闭环**（2026-10-26）：`units.unknown` 现由命令窗口真实消费；`docs/handoff-archive/2026-10.md` 的 2026-10-26 条目「接线验收」里
> 「未知源单位显示图纸单位」一句至此成立。仍未做的是**用户指定单位/校准**（spec 同句后半段）——
> 那需要新的单位设置面，不在本批范围。

#### 测量命令面（含 2026-10-26 新增 `ID`）

| 命令 / 选择子 | 行为 | 诚实边界 |
|---|---|---|
| `DIST`（`MEASURE DISTANCE`，别名 `DI`） | 2 点：距离 + 角度 + Δx/Δy/Δz | 单点不足即拒绝；结果随 `measurement` 回显 |
| `MEASUREPOLYLINE`（`MEASURE POLYLINE`） | N 点开放链长度 | 需 `CONFIRM` 收尾 |
| `MEASUREANGLE`（`MEASURE ANGLE`，别名 `ANGLE`） | 3 点顶点角 | 纸空间 `ANGLE` 显式拒绝 |
| `AREA`（`MEASURE AREA`，别名 `AA`） | 用户选点闭合多边形面积 | 自交拒绝、非共面拒绝、3 点起 |
| **`AREA OBJECT`（2026-10-27 新增）** | 对**当前选择的一个**闭合实体求**精确**面积：`Circle` ⇒ πr²、**完整** `Ellipse` ⇒ π·a·b、**闭合** `Polyline` ⇒ 平面内 **bulge 感知**的环面积（弧段用解析式，不采样） | **选择式**，不是 AutoCAD 的交互 Object 拾取（与 MATCHPROP 同型的声明式偏离）；`AREA` 的 `Add`/`Subtract` 未实现。拒绝面（全部按名、零改动）：0 或 >1 选择、开放折线、`Arc`（即使整圈）、`Spline`/`Line`/其它种类、折线平面不可定（含**两顶点**闭合 bulge 折线——仅凭顶点无法定平面与 bulge 符号）、非共面、弦环自交、椭圆弧、退化/零面积。`inputs` 诚实：圆/椭圆为空（不造点）、闭合折线为**自身顶点**且带自身平面；`precision = Analytic`。测量**不产生事务/撤销/revision** |
| **`ID`（2026-10-26 新增）** | **单点拾取 → 该点世界坐标 X/Y/Z + 空间（`model`/`paper`）+ 捕捉来源 + 精度 token**，作为一条 `point` 回显行（不是 `measurement`：点坐标不是标量测量） | **无 spec 授权**：§3.3 只要求距离/折线长度/三点角度/面积，「点坐标查询」是**项目自选范围**（与 DIMENSION 创建同一口径）。**`ID` 故意跳过栅格捕捉与 `ORTHO`**——报告的就是捕捉点本身，两者都不会把坐标挪走（**我们的规则**，见 `crates/cad-app/src/point_report.rs`）。`precision` = 捕捉几何的精度，无捕捉时 `Analytic`；`pixelDistance` = 原拾取到捕捉点的逻辑像素距离（无捕捉为 `null`）。拒绝面：块空间、透视/非正交视图、`Points` 载荷路径（`InvalidInput`，只接受真实拾取）。**不写库、不进历史、无撤销步** |

### ACIS 实体状态（3DSOLID / BODY / REGION / SURFACE，2026-11-16：一级 3D body）

这些实体**会画**，并且不再伪装成普通网格：导入期用 `cad-kernel-adapter` 的离散器把 SAT/SAB 转成
显示缓存，再以 `SemanticGeometry::Body3d` 发出（ADR-0036）。`3DFACE`/`POLYFACE`/`POLYGON MESH`/
`MESH` **仍是 `Mesh`**——它们是面片/细分网格，不是 B-Rep。

- **类别按能证明的事实取**：`3DSOLID → BodyKind::Solid`、`REGION`/`SURFACE → Sheet`、
  `BODY`/其它 → `Unknown`。`Solid` 只有在精确 B-Rep 存在时才成立；否则降为 `Unknown`
  并把原因留在 `display.completeness` 里。
- **精确几何**：`SolidExchange::Brep` 且 B-Rep 非空时存 `BodyExact::Exact`；否则
  `BodyExact::Unavailable { reason }`（复用 `ACIS_SOURCE_NOT_RETAINED_REASON`）。**绝不**从三角形
  反推 B-Rep。
- **容差是规则**：线性偏差 = 该实体**世界包围盒对角线的 0.1 %（1e-3）**，角度偏差取内核默认 0.35 rad；
  `BodyDisplay.tolerance` 记录实际请求值。`fixtures/acis/cube.sat` 对角线 3.4641016 ⇒ **3.464102e-3**。
- **边曲线与 B-Rep 容差（P2.1a，2026-11-17）**：除直线与整圆，**圆弧**（`BrepCurve::Arc{center,
  normal, u_dir, radius, start, end}`：绕 `normal` **逆时针**，两端是点；跨度 =
  `(end_param − start_param).rem_euclid(2π)`）也进入中性 B-Rep；圆弧边界的**平面**面可
  lift/离散/回写 record，**圆弧边界的曲面侧面**自 **P2.1b part 2（2026-11-17）起也能 lift**
  （见下条「曲面 lift」），但显示仍按名缺面。跨度 ≤ `1e-9` 是独立的
  `arc-without-span`（**不是** `partial-circle`），且 **v400 SAT 的 Arc 边不可导入**（acadrust 升版
  转换器把边参数补 `0.0`）；非半弧由跨度定方向，半弧按 ACIS 逆时针约定。
  `BrepData.tolerance` / `BrepFace.tolerance` 是**模型单位长度**（`Option<f64>`，`None` = 源没给，
  不编造）：组合 placement **不**改容差，`record::bake` 是**唯一**缩放点（进 record 时按比例把
  body 与 face 各乘一次）；非均匀变换照旧降级。写 record 的 body 容差取 body 值，否则取**已声明
  面容差的最大值**，再否则用**我们的**默认 `1e-7`；body 与面容差相对偏差 > `1e-6`（或面容差
  非有限/非正）⇒ `solid.lift_unsupported` 并点名 face id 与两个数；读回面级恒 `None`（record
  没有逐面容差槽位）。SAT 的 body/face/edge/vertex 访问器**没有**几何容差，
  因此导入侧诚实报 `None`，不取三个 SAT 头数字（那是文档级模型器设置）。
- **曲面 lift（P2.1b part 2，2026-11-17）：曲面体重新进入内核**。写入方向不再只写平面：
  `Cylinder`/`Cone`/`Sphere`/`Torus` 面带其解析 label + `solidcore::eval::form::surface` 给出的
  **精确有理 NURBS form** + form 参数下的 `(u,v)` box + 每个 use 的**精确 pcurve**
  （`solidcore_ops::pcurve::pcurves_within`，只接受闭式解、绝不拟合；未命中是
  `solid.pcurve_not_exact`）。完整圆边界（`BrepCurve::Circle`）现在也写出（`Curve::Circle` label +
  整圈 form + 一个 seam 顶点）；闭合边的遍历方向由共享该圆的**平面面**解析，两个非平面面共用时按名
  拒绝。被剪裁的侧面（弧 + 母线）与**完整圆锥**（`radius_top = 0`，写成 apex band：一条母线用两次、
  apex 作对端顶点）都能 lift。**精确性怎么证明**：圆柱/圆锥的纬圆先**重标到面自己的 frame**
  （`curved::align_curved_frames`，只移参数零点），再由 `verify_pcurves` 对每个 use 在 **6 个样本
  （4 个内点）**上按 `solidcore` 的 **1e-9 相对**判精确；未命中或**裁剪偏离四分之一网格**都是
  `solid.pcurve_not_exact`（诊断点名 face/loop/use 与 gap）。**两处偏差**：写入的 `(u,v)` box 由面
  **自身边界**推出（生产者的 `BrepFace.domain` 在它自己的 form 参数里——球是 `[0,1]²`、圆柱是弧度，
  不能直接交给 `eval::form`）；`label_box` 是**包围盒**（比裁剪片宽，u 取整圈），**绝不可收紧**——
  球的 seam 是零面积 bigon，全靠整圈 u 盒把两极两个 use 隔开一个周期。`eval::form` 的 knots 按每个
  角度方向 **π/2 重标**（精确重参数化、控制点不变）：**这不是把 form 参数变成 label 角度**
  （`eval::form::param` 在四分之一圈内非线性，重标后 `0.25` ↔ `0.357 rad`，不是 `0.393`），只是让
  **一圈 = `2π`**，使 box/`Domain::period`/seam 跳变同单位。
  **能力**：`ModelerCapabilities::nurbs = true`（登记字符串 `nurbs.faces`），且**承载**——布尔 /
  质量属性 / 倒角任一操作数含非平面面时该位必需，声明 `false` 的模型器直接答
  `solid.nurbs_not_supported`；**族优先**——族本身未声明的请求答 `solid.unsupported`，即使操作数
  带曲面。**显示**：NURBS 面自 P2.1c-1 起可经 `SolidTessellator` 接缝的 `solidcore-tess` 绘制
  （只对含 NURBS 面的 body 委派；未注入网格器的调用仍按名报缺面），被剪裁的圆柱/圆锥侧壁仍要求
  两个整圆因而报缺面
  （`cylinder boundary is not a single full circle`）——**lift 成功 ≠ 显示完整**。
  **证据**：`cad-kernel-solidcore` 的 `tests/curved.rs` / `tests/arcs.rs` 由**真实固定内核**
  （`55ae49ef`）积分闭式解（圆柱 `V=2π`/`A=6π`、球 `4π/3`、圆环 `V=π²`/`A=4π²Rr`、截头圆锥
  `πh(r1²+r1r2+r2²)/3`、完整圆锥 `πr²h/3` 与 `πr²+πr√(h²+r²)`、圆柱∪/∖/∩盒 `8+π/2`、`π/2`、
  `3π/2`、半圆柱 `V=π`/`A=3π+4`）。断言是内核请求的 **1e-6 相对精度**（或 `1e-6` 绝对，视量而定）；
  实测误差远小于它，具体数值只记在测试注释里、**不作为界**；Linux x86-64 契约/单元，
  无 GPU/像素/宿主/其它载体证据。
- **边线不再丢弃**：`TessellationMesh.edges` 进入 `BodyDisplay.edges`。有显示网格时表示层把它作为
  `DisplayPrimitive::MeshWithStoredEdges` 直送 `RenderBatch.edges`（P0.1 已完成，表示/场景契约与
  单元证据；渲染器的 GPU/像素结果 **NOT VERIFIED**）。
- **面身份（C3b，2026-11-17）**：`BrepTessellator` 为每个显示三角形写 `Mesh.face_sources`，键为
  `brep:face:<shell>/<face>`，`topology_revision` 是占位 `Revision(0)`（**不是**拓扑版本；
  拓扑改变后不得沿用）。body 的**整实体拾取**与**子面高亮**复用 mesh 路径；`face_sources`
  缺失 / 列表短于三角形 / 该三角形为 `None` 时如实报 `mesh-face-sources-absent` /
  `mesh-face-source-missing` / `mesh-face-source-index-out-of-range`；body 拾取分支**不再**
  产出 `body-face-identity-unmodelled`（该键只为旧显示缓存保留）。`cad-db` 只校验列表为
  **空或与三角形等长**，不校验键内容。
- **完整性**：精确 B-Rep + 解析离散 ⇒ `Complete`；曲面/近似离散 ⇒ `Partial` 并带具名近似理由；
  只离散不保留精确几何 ⇒ `Partial`。**只离散的实体不再报 `Verified/Verified`**。
- **拒绝保持原状**：超出子集、或原始 SAT/SAB 被内核拒绝 ⇒ 仍是 `Opaque` + 内核具名码
  （如 `kernel.no_acis_kernel`），**不把失败改成空成功**。
- **导出（P3a/P3b，2026-11-17：精确平面体写回 `3DSOLID`）**：`Body3d{Solid, Exact}` 且 B-Rep 属于
  **单壳 / 全平面 / 直线边界 / 相似变换（无剪切、无镜像）** 子集时，导出为真 `3DSOLID`——DXF 里是
  本仓库用 acadrust SAT 构建器生成的 **ACIS SAT 文本**（`crates/cad-connector-acadrust/src/export/acis.rs`：
  body → lump → shell → face → loop → coedge → edge → vertex → point，几何边跨两面焊接、每条边恰好两个
  coedge，平面法向/`u_dir` 归一化并校验正交、每个面的外环 Newell 方向须与面朝向一致）。报告记
  `export.converted.acis_solid`，含「无第三方 CAD 读取器验证」「载荷由保留的精确 B-Rep 重建、非原始
  SAT/SAB 字节」两条明示，以及被丢弃的显示缓存（三角形/边线）说明。
  - **拒绝即退回显示缓存**，每条一个稳定码：`acis.curved_face_not_written`（曲面/未知面；
    **P2.1b 的曲面 lift 不扩大导出子集**——能 lift 的曲面体照样走此码）、
    `acis.non_line_curve_not_written`（非直线边界）、`acis.no_shell_not_written`、`acis.multi_shell_not_written`
    （多壳/空腔）、`acis.placement_not_written`（剪切/镜像/非均匀）、`acis.invalid_topology_not_written`
    （开放/非流形边、面朝向反转、退化环）。退回时仍写 POLYFACE（仅边线时 `LINE`/`POLYLINE3D`）。
  - **DWG 写同一个子集（P3b，2026-11-17）**：同一份 B-Rep 写成真 DWG `3DSOLID`，载荷是**二进制
    SAB**（`SabWriter` 从**同一个** `SatDocument` 生成，`Solid3D::from_sab` 挂到实体上），由上游
    `DwgWriter` 放进 **AC1032 的 AcDs（`AcDsPrototype_1b`）段**；报告记 `export.converted.acis_solid`
    （DWG 措辞为 "binary SAB blob in the DWG AC1032 AcDs section"），note `export.note.acis_sab_dwg`
    明示载荷由保留的精确 B-Rep 重建、非原始 SAT/SAB 字节，且**未运行任何第三方 CAD 读取器**；SAVEAS
    状态行为 `save.exported_acis_sab[_partial]`。`BODY`/`REGION`/`SURFACE` 仍不写回。
    **注意**：含 ACIS `3DSOLID` 时报告恒为 `Partial`（显示缓存被丢弃），所以**干净**的
    `save.exported_acis_sab` / `save.exported_acis_sat` 目前**不可达**，只有 `_partial` 会真正出现。
  - **拒绝是按体的，不是按格式的**：子集外的体在 **DXF 与 DWG 都**退回显示缓存并记同一个稳定码
    （见上）；DWG 不再有独立的「不写 3DSOLID」码。SAB **没有**独立拒绝码：`SabWriter::write -> Vec<u8>`
    不可失败，输入是本仓库已 `validate()` 的同一份 SAT 文档（不可达的码不写进文档）。
  - **版本是文档级选择（仅 DXF）**：只要文件里有任一 `3DSOLID`（模型空间**或**已写出的块定义内），
    整个 DXF 按 **AC1024** 写出（`$ACADVER`），使载荷保持内联 SAT 文本而不是 AC1027+ 的 `ACDSDATA`
    SAB 块；报告 note `export.note.acis_sat_text_version`，SAVEAS 状态行也会明说
    （`save.exported_acis_sat[_partial]`）。**这条规则不适用于 DWG**：DWG 保持文档自身的 **AC1032**，
    正是为了让上游 writer 写出 AcDs 段。
  - **证据**：DXF —— 本仓库自己的写入→acadrust 解析→本仓库再 lift 的往返（盒/楔/棱锥/带方孔面拓扑计数、
    面参数精确、几何 ≤1e-9·对角线、`BrepTessellator` 体积 ≤0.1%）；`cad-cli-tools` 用**真实 solidcore**
    建 BOX → 导出 DXF → 再导入得到**精确体、体积 1±1e-3**。DWG —— 写入→上游 `acadrust::DwgReader`
    读回（AcDs 段把 SAB 挂回实体）→`sab_to_brep`→**同一精确体**；断言无 POLYFACE、文件版本 AC1032、
    SAB 单独可被 `SabReader` 解析。**没有第三方 CAD 打开过这些字节**（AutoCAD/ODA/BricsCAD 均未运行）：
    SAB 的头是 **ACIS 7.0（700，非 ASM）**，与 SAT 构建器一致，ACAD 2013+ 原生体通常带 21800 ASM 头，
    因此 **vendor 接受性未验证、不声明**。无 vendor/像素/非 Linux 证据。
- **证据**：cube 夹具 **12 三角形**、顶点界 `[-1,1]³`、闭合体积 **8.0**（解析 8.0，误差 < 1e-9）。
- **仍未做**：NURBS/样条曲面的**显示**（没有任何 NURBS 网格器）、一般裁剪 B-rep、完整 ACIS 内核；
  SAT/SAB **源字节仍不保留**（只保留 lift 后的中性 B-Rep）。**2026-11-17 更新**：`cad-kernel-solidcore`
  模型器（P1/B2）已能在 Linux 契约/单元里对**平面体**创建/布尔/算体积·面积·质心·惯性；**P2.1b
  （同日）起曲面体也能重新 lift**——解析曲面写精确 NURBS form + 精确 pcurve，`ModelerCapabilities::nurbs`
  已置 `true` 并在布尔/质量属性/倒角上承载（`false` 的模型器答 `solid.nurbs_not_supported`）。
  **命令层已接线七种图元的创建（C1）与三个布尔（C2，真实后台任务）**，所以 `BOX` 等能建体、
  `UNION`/`SUBTRACT`/`INTERSECT` 能对平面体与曲面体合并/相减/取交（任务 settle 后一条事务/一步撤销），
  `MASSPROP` 已对**精确 `Body3d{Solid, Exact}`** 接线（C3a，2026-11-17）：经**注入的**
  模型器**同步**测量（density 1.0、请求相对精度 1e-6，报告明写“估计、不是已证明的界”），
  **显示网格永不参与**积分；稳定拒绝为 `massprop.body_modeler_unavailable`（载体无模型器）、
  `body_not_liftable`（模型器拒绝该 B-Rep：曲面**可** lift，剩下的是无 `(u,v)` box 的曲面面、
  无精确 pcurve 的边界、多壳/空腔等结构性原因 → `solid.lift_unsupported`）、
  `body_kernel_failed`、`body_not_exact`（无精确 B-Rep）、`body_not_solid`、
  `mixed_selection`（body 与平面区域混选）、`kernel_busy`（布尔 worker 占用内核时快速拒绝）、
  `body_not_positive_volume`（反射/内外翻转体）。body 面的子元素引用解析到其所属 body（同一
  body 的重复引用只计一次）；非 body 几何的子元素仍 `not_addressable`。证据仅 Linux 契约/单元（真实内核路径见
  `cad-cli-tools` 的 BOX/CYLINDER，后者现报 `volume=62.831853 surface_area=87.964594`）。模型器的能力是 **family 级**：`booleans`/`mass_properties` 为 true
  不等于每个操作数都能算——开放 sheet、无 `(u,v)` box 或无精确 pcurve 的曲面面、反向面/重复面等结构不一致的操作数逐次返回
  `Unsupported`（`solid.lift_unsupported`）；**空腔（void region）的结果在布尔本身就被拒绝**，
  而**多壳结果**（例如不相交的两个体 `UNION`）会作为 body 成功返回、在**下一次 lift**
  （质量属性/再次布尔）才被拒绝，所以质量属性要求**恰好一个闭合壳**。这些拒绝是**模型器**行为，
  命令层必须逐次处理（C3a 已把 `Unsupported` 映射为 `massprop.body_not_liftable`）。

### DIMENSION 族（ADR-0017 P5a，逐行，非脚注）

P5a 让标注成为一级域实体并迁入两分支导入 + 导出；**P5b** 落地 `cad-db` 的 `DimStyle` 表 + B.2(a)/(b)
变量子集 + 公制默认 + 导入/导出接线（**无 UI 命令、0 个新 i18n 键**）；**P5c** 落地**首批创建命令**
`DIMLINEAR`/`DIMALIGNED`；**P5d** 落地 `DIMRADIUS`/`DIMDIAMETER`（**实体拾取圆弧/圆 + 尺寸线位置**）
与 `DIMCENTER`（**普通 LINE 几何**，**非** DIMENSION 实体，按**绘图辅助**记录）。以下逐行声明其能力边界
（决策 D/E 的边界与"创建无 spec 授权"都在此表内，不在脚注）。

| 能力 | 状态 | 诚实边界 |
|---|---|---|
| 创建（`DIMLINEAR`/`DIMALIGNED`） | **实现（P5c）** | **三点流程**（两尺寸界线原点 + 尺寸线位置），经共享合成从**当前样式** `Standard`（P5b `resolve_dim_style`）构造、经**单一漏斗**插入**一个** `AcDbDimension` 实体（一条事务、一步撤销、活动图层）；**非关联**（不写 `ACAD_DIMASSOC`）。**创建无 spec 授权**：§3.2 只要求"常用 DIMENSION 可见表示"；"创建标注"是**项目自选范围**，全部行为选择标为**我们的选择**而非"符合规范" |
| 创建（`DIMRADIUS`/`DIMDIAMETER`） | **实现（P5d）** | **实体拾取圆弧/圆 + 尺寸线位置**（AutoCAD 述"Select arc or circle"为**唯一入口**；**FILLET 生成的 Arc 也算**）。位置拾取定**方向**（圆心→位置），组 15 = 圆上该方向点；组 10 = **圆心（半径）**或**对径点（直径）**；组 40 写 **0.0**（引线延长未建模，声明）；测量值 = 半径/直径，经共享合成 + `Standard` 快照构造，经**单一漏斗**插入**一个** `AcDbDimension`（一条事务、一步撤销、活动图层）；**非关联**。**我们的选择**：`Arc` 时引线点落在**该弧的圆**上、不钳制到弧的扫掠范围（声明） |
| `DIMBASELINE`/`DIMCONTINUE`（基准/连续） | **已实现（P5f，2026-10-21 解禁）** | **显式拾取一个基准标注**（必须是 `Linear`，对齐/角度/半径/坐标按名拒绝；命令**不持有跨命令状态**）+ **N 个点** + **回车** → **一条事务/一步撤销**批量插入 N 个标注。`DIMBASELINE`：第 `i` 行尺寸线 = `base.location + n̂ · (i · DIMDLI)`（`n̂` 为**背离被测几何**的法向，点积为 0 按名拒绝，不猜边）；`DIMCONTINUE`：链式 `base.second → P₁`、`P_{i−1} → P_i`，尺寸线**不堆叠**（= `base.location`），投影 ≤ 0 拒绝；确认后工具回 `Idle`（**不做**「重复到 Esc」循环）。**边界**：**仅线性基准**、**不写 `DIMCONTINUEMODE`**（acadrust 无法表达）、**非关联**、无文字覆盖、无每点撤销（all-or-nothing）。**证据上限**：`cargo test -p cad-app` 契约 + 2026-10-21 一次**有界真机运行**（像素判据与其边界见 `docs/adr/0017-dimension-family.md` 的实施补充），**不是** vendor/AutoCAD 对照。`DIMCENTER` 见下方**绘图辅助**行（**不是** DIMENSION 实体） |
| `DIMANGULAR` 两线点语义（P5e） | **实现（U-8 已用真实夹具钉住）** | line 1 = 组 `13→14`、line 2 = `15→10`，**顶点 = 两条无限直线的交点**，**组 `16` 选择被测扇区**。依据：`fixtures/dxf/qcad-examples/*` 中 **9 条 type flag=34** 的记录全部数值复现；判别用例 `*D65` 的组 42 = `1.975688113` rad = 113.19859° 只匹配交点读法，且匿名块 ARC 圆心在交点。**U-8 关闭** |
| `DIMANGULAR` 四选一规则（P5e） | **我们的规则** | 被测角 = **包含标注弧位置拾取的扇区**；拾取**恰在边界射线**时取**自该射线起的逆时针扇区**；三点形式同规则（可选优角/劣角）。**U-1 仍标未查证**（Autodesk 该句在仅 JS 渲染的概念页） |
| `DIMANGULAR` 交互形态（P5e） | **两线与三点都可交互** | 捕获状态机改为**逐步语法**：首步 `PickOrPoint`（选线**或**指定顶点），首次捕获决定形式，另一类捕获按 `dimangular.capture.mixed` 具名拒绝；两线仍为 2 次拾线 + 弧位置，三点为顶点 + 两侧点 + 弧位置。React 步骤提示为四步（首条「选线或指定顶点」）。**2026-11-15 更新（原为「三点无交互捕获」）** |
| `DIMANGULAR` 拾取范围（P5e） | **仅 `Line`** | `Arc`/`Circle`/折线的两线形式为显式 `Unsupported` 拒绝（ADR 范围排除弧/圆角度变体） |
| `DIMORDINATE` 自动 X/Y（P5e） | **我们的规则** | `|Δy| > |Δx|`（feature 与 leader）⇒ **X**，否则 **Y** ⇒ **相等取 Y**（文档条件式的 "otherwise" 分支）。**U-5 的相等边界 Autodesk 未述**；`axis` 可强制 |
| `DIMORDINATE` 基准与量值（P5e） | 实现（沿用 P5b 语义） | 组 10 = **世界原点 `(0,0,0)`**；测量 = feature 的 X/Y **量值**（基准相对取绝对值，**带符号未建模**）；**零 ordinate 合法**（不拒绝）；引线长须非零；输入须 `+Z` 平面 |
| **角度标注组 42 单位（P5e 修复）** | **潜伏单位损坏已修** | 此前导出把角度组 42 写成**度**，而 DXF 语义是**弧度**（夹具全部为弧度）⇒ 旧导出**数值错误**。现 `measurement()` 仍以**度**供文字格式化，**导出组 42 转弧度**并报告声明 |
| 渲染变化（P5e） | **正确性修复 = 像素变化** | 角度合成改用"交点顶点 + 扇区选择"后，**已导入的角度标注渲染输出会变**（与 P5b 的渲染增量同性质）；文字仍为度 |
| `DIMLINEAR` 水平/垂直规则 | 实现（**我们的选择**） | 以两原点**中点**为基准，`\|Δy\| > \|Δx\|` ⇒ 水平（`rotation 0`，量 X），否则垂直（`rotation π/2`，量 Y）；由**尺寸线位置拾取**决定方向（AutoCAD 述，但严格提示状态机**未查证**） |
| `DIMLINEAR` 平局拒绝 | 显式拒绝（**我们的选择**） | 位置拾取恰在两原点**中点**（或 `\|Δx\| = \|Δy\|`）时方向未定义 ⇒ **按名拒绝**，不做任意打破平局；AutoCAD 对该退化拾取的行为**未查证** |
| 平面外拒绝（D.6） | 显式拒绝 | 定义点必须共一个 `+Z` 平面；**这是 ADR D.6 的平面外情形，不是视口相机检查**——命令接收世界点、**无视口依赖** |
| 零测量值拒绝 | 显式拒绝（**超出 ADR 清单**） | 所选方向把两不同原点投影到同一点（`measurement ≤ 1e-9`）⇒ **按名拒绝**，否则会存下结构性退化标注 |
| 当前样式 | 硬编码 `Standard` | ADR B.3：P5 **无交互式样式管理器**；创建从**唯一存储样式** `Standard` 解析快照；数据库**无** `Standard` 时是**防御性显式拒绝** |
| 对象选择变体 | **NOT IMPLEMENTED** | 仅**点选流程**；选直线/圆/弧自动取端点的对象选择变体未实现（决策 C） |
| 文字覆盖（`Text`/`Mtext`） | **NOT IMPLEMENTED** | 只画自动测量值；`text_override` 字段保留以备导入（决策 C） |
| 标注预览 | **声明限制：无真正标注预览** | 放置期间实时预览是后端**橡皮筋链**（`preview.rs` 把两种 kind 路由到 `preview_chain`）——用户看到已捕获原点 + 到光标的链，**看不到**合成标注（尺寸线/尺寸界线/箭头/测量文字）。真预览需**后端 overlay 分支**；React UI **无法**绘制 overlay（原生 CAD 分区是 X11 子窗口 / Wayland 子表面，DOM 不得浮于其上——ADR-0010 `:27`）。React 宿主因此**改为每步提示**。**2026-10-23 更正**：“React UI 无法绘制 overlay”的几何前提已失效（[ADR 0023](adr/0023-dom-on-top-cross-host-composition.md) D1）；**限制本身仍在**——缺的是后端 overlay 分支，不是前端绘制能力 |
| 创建标注的导出形状 | 复用 P5a 形状 | 与 P5a 相同：内容块 + 组 42 = 测量值 + DIMSTYLE 记录；**往返现首次在"创建"的标注上演练**（导出 → 再导入 → 同 kind、同定义点） |
| 域实体 | 实现 | `SemanticGeometry::Dimension`；`display` 私有、单写入路径；无块与图片两分支都迁（A.6） |
| ArcLength / LargeRadial | 读取合成 `Partial`；**不迁移** | 不在 ADR-0017 kind 集合（无点槽/组码）；保留旧 `Compound` 合成；**导出仍与从前一样有损**，P5 **不**声称 DIMENSION 完整往返 |
| 关联性（`DIMASSOC=2`） | **NOT IMPLEMENTED** | 创建与迁移的标注都是**非关联单实体**；编辑被测量几何**不**更新标注；导出**不写** `ACAD_DIMASSOC` |
| 导出形状 | `Partial`（有损另存） | 真实 DXF `DIMENSION` + 生成 `*D…` 内容块 + 组 42 + DIMSTYLE 记录；**组 70 的 32 位不写、0x80 位从不置位、组 210 不写、组 51/52/71/72 丢弃、DIMSCALE 不保留**；内容块的必要性依据**第三方文档**（ezdxf）且**未用第三方读取器验证**（NOT RUN） |
| 行为变更①：捕捉目标 | **新增** | 标注自 P5a 起**首次**成为捕捉目标（捕捉目标经 `transform_geometry` 烘焙，此前 `Compound` 被拒） |
| 行为变更②：`EXPLODE` | **由成功转拒绝** | 对**带匿名块**导入的标注，`EXPLODE` 由成功（原为 `Insert`）变为**拒绝**（ADR-0016），且 picture 块**不再存在于块表** |
| Picture 分支 | `Partial` | 成员不可放置（`Spline`/`Shape`/`Opaque`，或含 `Text` 的嵌套 `Compound`/`Insert`）⇒ **整个**标注 `Opaque`+`Partial`（绝不部分变换） |
| 样式（`DimStyle` 表） | 快照，不追溯 | 实体存**已解析快照**；编辑 DIMSTYLE **不**追溯（ADR-0017 B.1）；`DIMSTYLE` 编辑命令未实现（B.3）。**P5b**：`cad-db` 新增 `DimStyle` 表（`crates/cad-db/src/dim_style.rs`）承载 **B.2(a)+B.2(b) 全量变量**；`standard_metric` 为钉住的公制默认、`resolved` 把 `DIMSCALE` 烘进尺寸；创建时 API `resolve_dim_style`/`dim_style_by_name`/`dim_style(s)` 供 **P5c** 使用；`finish` 保证总有 `Standard`。实体快照现携带 **B.2(a) 全集**，**每个变量都被合成/格式化真正消费**（不再"存而不消费"） |
| 渲染增量（P5b） | **正确性改进 = 像素变化** | P5b **确实**改变已导入标注的渲染输出（一般）：`DIMTAD` 2/3/4 现**真正实现**、`DIMSE1/2`、`DIMSD1/2`、`DIMCEN≠0`、`DIMRND`、`DIMPOST`、`DIMZIN` bit 2、`DIMDSEP` **新被消费**。**已提交的 QCAD flange 经验证不变**（其 `QCADDimStyle` 只设 DIMASZ 2.5 / DIMEXO 0.625 / DIMEXE 1.25 / DIMTIH 0 / **DIMTAD 1** / DIMZIN 8 / DIMTXT 2.5 / DIMGAP 0.625，六条标注**全为 aligned/rotated linear，无 radial**）——**"不变"的断言只限该夹具** |
| `DIMCEN` 默认 | **刻意偏离**（0.0 vs 文档公制 2.5） | 变量页写公制初值 **2.5**，我们**刻意钉 `0.0`**（中性：不给否则不变的显示添加中心标记，见 ADR-0017 U-4 更新）。**不**静默改已导入标注（导入从源样式解析；默认只到达手工构造的库与缺失样式回退）；但**新建**的 radial 标注（**P5d**）在 AutoCAD 公制会画 2.5 中心标记处**不画中心标记** |
| `DIMCEN` 绘制近似 | 近似 | AutoCAD 只在**标注线落在圆外**时为 radius/diameter 画标记，**我们总是画**；负 `DIMCEN` 在 AutoCAD 是"中心标记 + 带缺口的中心线"，我们画**普通长十字** |
| `DIMCEN` 符号读数（P5d） | 实现（**钉住的读数**） | `> 0` **中心标记**（十字 LINE 对，臂长 `\|DIMCEN\|`）；`< 0` **中心线**（十字 LINE 对，达 `radius + \|DIMCEN\|`，伸出圆外）；`0` **无**。**近似（声明）**：AutoCAD 的负值"中心标记 + **带缺口**中心线"变体**未建模**（我们画普通十字，与径向合成同一声明近似）；ADR 决策 C/E 述输出为"普通 LINE/ARC"，**我们只产 LINE**（ARC 变体未建模） |
| `DIMCENTER`（绘图辅助，**非** DIMENSION） | **实现（P5d）** | 文档明确 DIMCENTER 创建**非关联中心标记/中心线** ⇒ 产出**普通 `Line` 几何**（`type_key` `AcDbLine`），**不是** `DIMENSION` 实体（ADR-0017 D.7）。一次实体拾取圆弧/圆；经**批量单一漏斗**插入（一条事务、一步撤销、活动图层）。**`DIMCEN = 0` 产出 0 个实体 + 显式诊断 + 无事务**（同 ARRAY 1×1，**绝不静默空成功**）。拒绝：非圆弧/圆拾取、平面外实体、退化半径、缺失活动图层、Viewer 模式 |
| `DIMCEN` 中心（B1 修复） | 修复 | 中心标记画在**真实圆心**（`Diameter` 为**两弦点中点**，非组 10 的**对径点**）；中心线触及**半径相对**（`radius + \|DIMCEN\|`），非引线全长 |
| `DIMTAD` 2/3/4 | 实现 | `=2`（外侧）**文档化**（Autodesk 措辞："背离定义点的一侧"）；`=3`（JIS）为**独立代码**但**垂直放置等于"上方"**，JIS 特有的**水平放置与文字适配规则**为**声明未实现**；`=4`（下方）实现 |
| `DIMTAD`/`DIMTVP` 适用面 | **仅 linear/aligned** | radial/angular 保留各自的 leader/弧放置，**不读** `DIMTAD`/`DIMTVP` |
| `DIMSE1/2`、`DIMSD1/2` | **仅 linear/aligned** | 抑制第一/二条延长线、第一/二段标注线+箭头；radial/angular 不读 |
| `DIMSD1/2` 分割 | 近似 | 在**文字中点**处分割标注线，是 AutoCAD 文字间隙断口的**近似** |
| `DIMGAP` | 仅文字位置 | 只影响**文字位置**；文字周围的**标注线断口未被建模**（B.2(a) 原行过度承诺） |
| `DIMRND` | 仅显示文字 | 取整**只作用于显示文字**；组 42 保留**未取整**的测量值 |
| `DIMPOST` | 有读法偏离 | 无 `<>` 的模板按**后缀**处理（对未言明情形的读法）；AutoCAD 的 `DIMPOST = "."` 表示"无后缀"**未被遵循**（会渲染出 `42.5.`） |
| `DIMLUNIT` / `DIMDSEP` | 实为 (a)；≠2 降级 | 二者**被格式化消费**，故实为 **B.2(a)**（B.2 原表把它们列在 (b)）；`DIMLUNIT ≠ 2` 是**声明降级**，导入浮出 `Completeness::Partial`（诚实测试见 ADR-0017 P5b：`dimlunit_other_than_decimal_is_a_declared_degradation` / `non_decimal_dimlunit_is_declared_partial_not_a_silent_decimal`）；导出从**实体快照**写（非表） |
| `DIMZIN` | bits 2+3；英尺/英寸未实现 | 现消费 **bits 2+3**（尾随 + 前导零抑制）；B.2(a) 原"仅消费 bit 3"格**已过时**；英尺/英寸位（1/2/16）**未实现** |
| `DIMCONTINUEMODE` | 存储不可写 | 图纸级、acadrust 未建模；表带文档化默认，导出**无法写出** |
| DIMSTYLE 记录来源 | **拆分（N1）** | B.2(a) 驱动显示的值来自**实体快照**，B.2(b) 来自**表**；缺失样式名回退到钉住的 `Standard`（**声明过的**报告行）；声明为 NOT IMPLEMENTED 的样式变量按 `DimStyle::new` 的**中性值**写出（非源值）；`DIMDLI`/`DIMDLE` **未缩放**地写进 `DIMSCALE=1` 记录，`DIMSCALE` 本身**不保留** |
| `DIMTIH`/`DIMTOH`/`DIMTOFL` | **U 行（需出处或降级）** | 在 `standard_metric` 里断言为"Autodesk 公制 Standard"，但只有**英制**初值可核实（On/On，与 acadrust `dxf_defaults` 一致）；文档化的公制样式（ISO-25）让文字**与标注线对齐**（off/off）。属 B.2(b)（显示忽略）故**显示安全**，但该断言需要**引用出处**或**降级** |
| `insert_dim_style` | 拒绝而非覆盖 | 一个**已被别的名字占用**的样式 id 被**拒绝**（调用方 bug），而不是覆盖并**遗弃**旧名的索引项 |
| 文字 | 受宿主字体门控 | 测量文字是 `SemanticGeometry::Text`，无宿主字体则"不由场景绘制"（既有 `Partial`）；标注**继承**此限制 |
| i18n | P5b **0 新键**；P5c **+9 键** | P5b **无 UI 表面**（无命令/面板/状态投影），诚实计数为**零**（计划里 P5c 行写的 "+ i18n" 指 **P5b**）；**P5c** 新增 9 键：`tool.prompt.dim.{origin_first,origin_second,location_linear,location_aligned,ready}`、`tool.draw.{dimlinear,dimaligned}`、`draw.kind.{dimlinear,dimaligned}` |


## acadrust 依赖能力（0.6.8）

`acadrust` 已由 0.5.5 → 0.6.3 → 0.6.5 → **0.6.8**（features `serde` + `import`，registry `rsproxy`；
上游代码**未修改**、当前没有 `[patch]`；如确需补丁，只允许 `acadrust` 且必须固定版本/revision 并在
`docs/acadrust-patches.md` 登记，`scripts/check-architecture.py` 强制该台账——见 ADR-0031）。
`cad-connector-acadrust` 是唯一消费方；`cad-proxy` 不依赖 acadrust，只消费原始字节。下表为源码核对后的能力增量，「本项目处理」一列说明 yacr 当前的读取/建模，不改变任何
Unsupported 语义。**0.6.5 → 0.6.8 的差异**单列在表后的「0.6.8 增量」小节；只有被我们**实际执行**过的
能力才写"已接入/已验证"，其余明确标注"上游已提供、本项目未消费"。

| 能力 | acadrust 0.6.8 | 本项目处理 |
|---|---|---|
| STYLE xdata 字体面 | **支持（已类型化）**：`TextStyle.true_type_font` / `true_type_font_flags`；DXF 读 `1001 ACAD`/`1000`，DWG 经 `io/dwg/typeface_eed.rs` 按 APPID 名解析 | 已接入字体解析，链为 `类型化 true_type_font > group 3/4 声明字体 > 字节扫描`（`read_styles` 对 DXF/DWG 统一，DWG 经此通道自动生效）；字节扫描自身仍不覆盖 group 3/4 |
| LAYOUT `group 72`/`group 73` | **支持**：`read_layout`/`read_plot_settings` 映射到 `Layout.plot_paper_units`/`plot_rotation` | `record_from_layout` 直接读取这两个字段；标准纸名带单位记号时仍优先用纸名（不伪造），仅当文件未声明旋转时才按视口范围推断 |
| plot-style 解析值 | **不支持**：`PlotSettings` 只有 `current_style_sheet` 与标志，无颜色/线宽/alpha；引用名已类型化（0.6.8 未变） | 显式 Unsupported：`cad-connector-acadrust::plot` 读取 `current_style_sheet`/`Layout.plot_style_sheet` 与非缺省 shade plot，各产出 `import.plot_style_unsupported`/`import.shade_plot_unsupported` 诊断；颜色/线宽/alpha 仍不应用（`docs/render-order.md`、`docs/plot.md`） |
| 代理记录 opcode | **不支持（0.6.8 未变）**：`proxy_graphics.rs` 与 0.6.5 **逐字节相同**，仅 FillOff(21, 空 payload)/UnicodeText(36)/Unknown | 显式 Unsupported：仅证据支持的记录被解码，其余保留原字节（`docs/proxy-support.md`） |
| DWG xdata（实体/对象） | **部分**：实体 EED 仅在 APPID 句柄经 `document.app_ids` 解析时解码，否则保留 `raw_dwg_eed`。**0.6.8 起对象 EED 可读**：`CadDocument::object_xdata(handle) -> Option<&ExtendedData>` 公开，另有 `set_object_xdata_record`/`remove_object_xdata_record`（0.6.5 的 `document.object_xdata` 字段是 crate-private） | 实体 xdata 仍为显式 Partial（未解码 EED 保留原字节，不猜测）。**对象 EED 已读入报告（2026-11-03）**：`ImportReport.object_xdata: Vec<ObjectXdataApplication>`（按应用名排序、`objects` 为“携带该应用的**对象数**”，同一对象重复记录不重复计数）+ 一条 `xdata.object_ignored` 诊断（最多列 5 个应用名）。**只报告、不应用**到几何/图层/任何导入状态 |
| DXF 已知实体上的未知码 | **不支持（0.6.8 未变）**：未知码不保留；`EntityCommon.raw_record` 仍只由 DWG 读取器填充 | 显式 Unsupported；DWG 侧 `raw_record`（可变访问即失效）与 `layer_handle` 本项目仍未使用 |
| UnknownEntity 原始数据 | 不变（仅 `Entity` trait 形状改变） | 现有 raw 保留路径不变 |
| **ACAD Architecture（AEC）实体** | **新支持（0.6.8）**：`ExtendedEntityData::Arch(ArchEntityData)`；`classes::arch` 提供 `ArchKind`（wall/door/window/stair/space/…）、**已解码的 `profiles: Vec<Vec<Vector3>>`（世界坐标，取自 proxy-graphics 预览）**、`DecodedAec`（`decode()`：placement/strings/handles）、`straight_wall_geometry()`（仅 AC1032 `AEC_WALL` 的直墙帧）、以及创建/编辑 API | **已接入读取（2026-11-03）**：① **直墙优先用原生参数**——`straight_wall_geometry()`（仅 AC1032 DWG 且类体为直墙参数布局的 `AEC_WALL`）存在时，用 `continuous_wall_outline([start, end], width)` 画**闭合定向轮廓**（矩形足迹），具名声明“高度不画、开洞不切、共享端点不做 miter（每段方头）”；② 否则每个 ≥2 点的 profile 画成**开放折线**（编码记录本身没有闭合标志；首尾重合的开放折线与闭合轮廓渲染等价，不臆造 `closed`），恒 `Partial` 并声明“只有 proxy-graphics 预览轮廓；开洞、3D 体量、类参数与样式对象未建模”；③ 无原生几何且无 profile ⇒ `Opaque` + `Partial` 具名原因。**不消费**创建/编辑/连续墙链级 miter |
| 导入资源上限 | **新支持（0.6.8）**：`DwgReadOptions` 新增 `max_mesh_faces` / `max_mesh_face_indices` / `max_decompressed_bytes`（默认全 `None`） | **已接入（2026-11-03）**：`ImportLimits` 新增同三字段（`Option<usize>`，默认全 `None`），经 `dwg_read_options()` 逐字传入 DWG 读取器。默认 `None` = acadrust 自身行为，因此**不静默改变**接受/拒绝；宿主设 `Some(n)` 才收紧。**证据边界**：契约测试只证明选项透传，未用带 MESH 的 DWG 验证截断语义 |


### 0.6.5 → 0.6.8 增量（源码核对，2026-11-03）

升级由 `Cargo.toml`/`Cargo.lock` 完成（`acadrust 0.6.8`，无 `[patch]`、未改上游）。上游在该区间新增
**ACAD Architecture（AEC）类实体**、文档级镜像/旋转/展开助手、DWG 读取资源预算，以及若干实体级
修复；同时做了一次 **`Entity` trait 重构**。逐项：

| 变更 | acadrust 0.6.8 行为 | 对 yacr 的影响 |
|---|---|---|
| **`Entity` trait 重构（破坏性）** | trait 不再有 `handle()`/`set_layer()`/`color()`… 逐字段方法，改为 `common() -> &EntityCommon` + `common_mut()`；各实体只实现这两个 | **源码级破坏**：本仓库唯一受影响的是测试里的 `.common()` 调用（需 trait 在作用域或直接用 `common` 字段）。业务代码不消费旧访问器 ⇒ 无行为变化 |
| **ACAD Arch（AEC）实体** | 新增 `ExtendedEntityData::Arch` 变体与 `classes::arch` 模块：`ArchKind`（wall/door/window/opening/stair/railing/slab/roof/space/curtain wall/…）、`profiles`（proxy-graphics 预览轮廓，世界坐标）、`DecodedAec`（placement/strings/handles）、`straight_wall_geometry()`、创建与参数编辑 API、`continuous_wall_outline`、`decode_profiles`/`polyline_from_type6` | **已接入读取**（见上表 AEC 行）：直墙原生参数 → 闭合定向足迹；其余 profile → 开放折线；恒 `Partial`；创建/编辑/链级 miter 不消费 |
| **DXF 组 70 位 32（维度）** | `write_dimension_base` **无条件**把组 70 的 32 位置位（`type_flags` OR `0x20`，“block 仅被本标注引用”）；`text_user_positioned` 时仍写 `0xA0` | **导出字节变化**：aligned/linear 为 `1+32`/`0+32`（即 33/32），angular 为 `34`/`37`。本仓库为每条标注生成独占 `*D` 块 ⇒ 该位正确；ADR-0017 的“位 32 不写”作废（见该 ADR 的 0.6.8 复核小节） |
| **角度标注构造函数单位** | `DimensionAngular2Ln::new` / `DimensionAngular3Pt::new` 的 `actual_measurement` 由**度**改成**弧度**（与文件单位一致）；新增 `Dimension::stored_measurement()` | **无影响**：本仓库从不调用这两个构造函数，组 42 一直自己写弧度（P5e 已修） |
| **DWG 读取资源预算** | `DwgReadOptions` 新增 `max_mesh_faces`/`max_mesh_face_indices`/`max_decompressed_bytes`（默认 `None` = 旧行为），另加 `with_mesh_limits`/`with_unlimited_mesh`/`with_max_decompressed_bytes`/`mesh_budget`/`set_mesh_budget` | **已接入 `ImportLimits`**（默认 `None` 保持旧行为）；透传由 `dwg_read_options_copies_import_limits` 钉住，截断语义未用 MESH 夹具验证 |
| **对象 EED 公开** | `CadDocument::object_xdata(handle)` 现为公开访问器（0.6.5 为 crate-private 字段），另有 set/remove；`ExtendedData::records()` 给出按应用名分组的记录 | **已读入报告**：`ImportReport.object_xdata` + `xdata.object_ignored` 诊断；**只报告、不应用**。实体 xdata 仍为显式 Partial |
| **文档级几何助手** | 新增 `CadDocument::mirror_entity`/`mirror_entity_with_options`（尊重 `$MIRRTEXT`）、`rotate3d` 系列、`rename_layer`、`restore_constraint_vertex_ids`；实体级 `Transform`/`Mirror`/`MirrorOptions` 移至 `entities/modification` | **未消费**：yacr 的 MIRROR/ROTATE 走自己的 `cad-db::transform_geometry`（且明确拒绝 Text 镜像；上游的 `$MIRRTEXT` 语义不改变我们的声明） |
| **R11/R12 识别与代码页保真** | 新增 `is_r11_r12`/`r11_r12`、`code_page_is_faithful`/`dwg_code_page_is_known`、`decode_cif_escapes_in`/`decode_legacy_escapes_in` | **已接入诊断（2026-11-03）**：`format.r11_r12`（源为 AC1009 家族）与 `format.code_page_approximate`（无精确解码表）。两码**故意不带 `import.` 前缀**——它们不证明内容丢失，不得伪造 `Partial`；契约测试钉住“与基线 completeness 相同”。R12 **写出**仍不支持 |
| **导出写出的头变量** | 新建文档多写 `$DIMTXSTY`/`$DIMLTYPE`/`$DIMLTEX1`/`$DIMLTEX2`/`$DIMFXLON`/`$DIMARCSYM`/`$UNITMODE`；类描述串把 `AutoCAD` 改成 `ACAD`（Express Tools/subD mesh 等） | **仅字节差异，无语义变化**：已重新生成提交的 `fixtures/dxf/synthetic-acis-cube.dxf`（+37/−5 行）并复核仍是一实体立方体、可离散 |
| **ACIS 解析器/写出器** | `acis/{parser,sab,types,writer}.rs` 有改动（含 SAT/SAB 读写细化） | **未重新验证**：本仓库仍只用 `cube.sat` 夹具的离散路径；没有新的 vendor SAT/SAB 证据，**不得**宣称 ACIS 兼容性提高 |
| **依赖** | `acadrust` 新增 `serde_json` 依赖；`windows-core`/`windows-link` 去重 | `Cargo.lock` 相应变化；无新的直接依赖、无 `[patch]` |

**仍未改变**（0.6.8 源码核对，逐项）：代理记录 opcode（`proxy_graphics.rs` 逐字节相同）、
`PlotSettings` 的逐 ACI 颜色/线宽/alpha、`DIMCONTINUEMODE` 建模、DXF 已知实体未知码保留、
点云点数据（仍只有元数据/范围）、独立 `Xref` 对象（仍只有 `BlockRecord` 标志 + `xref_path`）。

### 0.6.3 → 0.6.5 增量（源码核对，2026-10-20）

| 变更 | acadrust 0.6.5 行为 | 对 yacr 的影响 |
|---|---|---|
| **标注标志映射修正** | 只改 **DWG** 标志字节位 0：0.6.3 把位 0 当"文字被用户定位"，0.6.5 反转为"文字在默认位置"（`text_user_positioned = (flags & 0x01) == 0`）。**DXF 组 70 的 `0x80` 读取路径未变** | **无影响**：本项目不消费 `text_user_positioned`/`dwg_flags_byte`（见 ADR-0017 P5d） |
| **标注数据保留** | `DimensionBase` 新增组 12（插入点）写出（仅非零时）；`Dimension(Box<..>)`/`MText(Box<..>)` 装箱（API 破坏性变更） | 已适配 `Box::new`；我们从不设置 `insertion_point`（零值 ⇒ 不写出）⇒ **输出无变化** |
| **DIMSTYLE 表 / 默认值** | `DimStyle` 结构、`DimStyle::new()` 默认值、`write_dimstyle_table`/`write_dimstyle_entry` 的组码（含组 3）与 xref 标志**逐行未变**；读取器仅新增把 DIMSTYLE 组 50（`DIMJOGANG`）由**度**转弧度存储 | 导出"每个样式名一条 DIMSTYLE 记录"的行为不变，组 3 仍可解析（往返测试通过）；本项目不建模 `DIMJOGANG` ⇒ 无可见变化 |
| **DIMJOGANG（0.6.4）** | DXF 头 `$DIMJOGANG` 按弧度写出并夹在 5°..90°；DIMSTYLE 组 50 改为**度** | 本项目**不建模** `DIMJOGANG`（默认 `π/4` ⇒ 不写组 50），导出新建空文档 ⇒ 仅多出默认头变量，**无语义变化** |
| **DXF 头保留（#127，0.6.4）** | 写出/往返 `$ANGBASE`、`$ANGDIR`、`$ELEVATION`、`$THICKNESS`、`$FILLETRAD`、`$CHAMFER{A,B}`（另有 R2004+ 摘要信息、`$HYPERLINKBASE` 等） | 导出**不复用源 header**（`CadDocument::new()` + 仅 `$INSUNITS`）⇒ **无影响**；导入侧这些字段本不消费 |
| **旧代码页（0.6.4，`oem_cp`）** | 精确 OEM / Mac Roman / Johab 解码与编码（`Cargo.lock` 因此新增 `oem_cp 2.1.2`、`phf 0.11.3`） | 经 `DxfReader`/`DwgReader` 间接受益（legacy `$DWGCODEPAGE` 文本更准）。**未**用 legacy 编码样本验证 ⇒ 不宣称已验证 |
| **线型元素（#128，0.6.4）** | 原生零缩放的普通虚线不再被误解析为复杂 shape 元素 | 导入线型模式（`LinetypePattern`）的输入因此更准；**未**用该缺陷样本做对比验证 |
| **CTB/STB 绘图样式编解码** | 新增 `acadrust::plot_style::PlotStyleTable`（`from_bytes`/`to_bytes`/`resolve_color`/`resolve_lineweight`），**文件级**编解码；DXF/DWG 文档内 `PlotSettings` 仍只有样式表**名**与标志，无逐 ACI 解析值 | **未消费**：yacr 打印用显示解析色，不读 `.ctb`/`.stb` ⇒ 上方 plot-style 行仍为显式 Unsupported（颜色/线宽/alpha 不应用） |
| **图层透明度 XDATA 解码** | 新增 `Transparency::from_layer_alpha_value`：图层 `AcCmTransparency` 值 `0` = **不透明**（0.6.3 会解成 `ByLayer`），未知 method 字节不再被默认成已知模式（DXF 与 DWG 读取器均改用）；实体侧 `from_alpha_value` 未变 | 导入的 `layer_opacity` 直接消费该值 ⇒ 对带显式 `AcCmTransparency` XDATA 的图层，**透明度解析结果会变**（更符合"图层 0 = 不透明"）。**无专门夹具验证**，不声称已回归 |
| **审计与恢复** | `CadDocument::audit()`、`DxfReader::recover()`、`DwgReader::recover()`（保留有界辅助记录、孤立几何不入模型空间） | **未消费**：导入失败仍走既有显式错误/诊断，不把恢复结果当成功 |
| **R12 原生写出 / 动态块默认值 / ACIS 颜色** | `DxfWriter::with_version(AC1009)` 真 R12；动态块默认值与实例属性；ACIS 面/边颜色 | **未消费**：导出默认 AC1032，含 `3DSOLID` 时按 AC1024 写出内联 SAT 文本（P3a）；动态块仍按既有代理/部分支持记录；ACIS 颜色未接入 |

## Linux 宿主（2026-10-03 新增，主验收入口）

`apps/app-linux` 共用桌面与官方 wgpu 离屏宿主，Linux release App 成为第一验收标准。
软件 Vulkan 合成 smoke、窗口系统、真实 GPU、真实 DWG 结论独立，当前窗口/真实 GPU 未验收。
文件打开/侧车通过显式路径参数；无文件选择器/恢复决策/后台导入/Trim 取点时显式 Unsupported，
不会默默 discard 未保存修改。用户批注子系统已于 2026-10-05 整体移除（见
`CAD_IMPLEMENTATION_SPEC.md` 顶部变更记录），本表不再包含批注能力。实现及本轮运行证据见
`docs/linux-app.md`，不更改实体兼容结论。

## 绘制/编辑命令（命令行程序，2026-10-10 / 2026-10-11）

`cad-app` 现提供一组**真实、事务化**的绘制/编辑命令（均 work-only，Viewer 由命令层显式拒绝；
每条命令一次事务、一步撤销，失败不改任何状态、也不记历史）：

> **授权口径（2026-10-26 补齐，此前缺失）**：本节与下方「创建命令」的**编辑/创建命令族不在 spec
> 授权范围内**。规范在初期范围里明确「仅测量及其管理，不修改 DWG 原始图元」「不允许原图修改命令」
> 「不实现 DWG 回写、原始实体移动/删除/修改」，完整编辑亦被排除在初期之外。因此 MOVE/COPY/ERASE/
> TRIM/EXTEND/ROTATE/SCALE/MIRROR/OFFSET/CLIPBOARD/MATCHPROP、各类创建命令、夹点拖拽
> **全部是项目自选范围**：行为选择标为**我们的选择**，不得表述为「符合规范」。与 DIMENSION 创建行
> （「创建无 spec 授权」）同一口径。这些命令**只改内存文档**：`SAVE` 写原生 `.ymodel`；
> 导出 DXF 走 `EXPORT`（**块定义与 `INSERT` 以及 XCLIP 的 `ACAD_FILTER`/`SPATIAL_FILTER` 链现已写出**；
> 无法表达为单条 INSERT 的摆放仍展开并逐条声明），导出 DWG 是实验性的显式动作（ADR-0041）。

| 命令 | 能力 | 诚实边界 |
|---|---|---|
| LINE / CIRCLE / ARC | 创建（点捕获或几何直通）；`CIRCLE 2P`/`CIRCLE 3P`/`CIRCLE D`（2026-11-01，多词 canonical）分别按直径两端点、三点外接圆、圆心+直径距离创建一个 `Circle` | 退化/共线/非有限输入显式拒绝；`CIRCLE D` 的拾取距离是**直径**，半径 = 距离/2 |
| XLINE / RAY | 创建有界构造线：两点 → 有限 `Line`（type-key `AcDbXline`/`AcDbRay`），裁到模型包围盒（外扩 5%/至少 1.0）或默认 1000 盒；一条事务/一步撤销 | **不是无限几何**；方向退化/出界/非有限点按名拒绝；导出写 `LINE`（`Converted`）；无 3D/OCS 版本 |
| INSERT | 创建块参照（**点-后-名**：先拾取插入点，再输入**本图已有块**的名称；`T(插入点)·T(−块基点)`，type-key `AcDbBlockReference`，活动图层/模型空间，一条事务/一步撤销） | **无块表写入**（只读块定义 + 名称/基点元数据）；无块选择器/palette、比例/旋转/属性值/动态块参数；未知名称/缺元数据/空名/换行/非有限点按名拒绝；`WBLOCK` 为具名拒绝（未实现单文件写出；`PASTEBLOCK` 已实现 v1） |
| BLOCK | **选择 → 基点 → 名称**（终端文本）：选中模型空间顶层实体写入新块定义并移入 `SpaceId::Block`，几何**逐位保留**、只改空间；元数据存基点；参照为 identity `T(基点)·T(−基点)`，后续 `INSERT` 仍按 `T(I)·T(−基点)`；一条事务/一步撤销，块级 undo 补丁 | **块属性模型按 ADR-0055 正在实现**（ATTDEF/ATTRIB/值编辑/提取；在实现与契约落地前仍按名拒绝/只读）；动态块参数、retain/delete 选择、块选择器；重名（精确/ASCII 大小写不敏感）、实例路径/子元素、图纸空间/块成员、空/换行/非法名称、非有限基点、锁定图层按名拒绝 |
| MOVE | 平移选择（折叠重复 ref） | 不可变换几何整条拒绝，不静默丢弃。**`SPLINE` 自 2026-10-26 起可平移**：NURBS 曲线是**仿射不变**的（映射每个控制点、`degree`/`knots`/`weights` 逐字复制 = 曲线的精确仿射像），`cad-db` 的 `transform_geometry` 不再按种类拒绝样条 |
| COPY | 复制（新 id） | 同上（含 `SPLINE` 可变换）；按对象样式覆盖无法复制时记 `drawing.copy.style`（副本继承图层样式） |
| ERASE | 删除选择 | 空选择拒绝；撤销恢复渲染属性与动态可见性成员 |
| BREAK | **实现** | 两断点 + `F` 首点覆盖；`LINE`/`ARC`/`CIRCLE`/`SPLINE`/`LWPOLYLINE`/整椭圆走精确子曲线；第二点可离对象并投影到曲线；`CIRCLE`→`ARC`（补集）；源保留为第一段、第二段新 id；一条事务/一步撤销 | 部分椭圆弧打断未在 AutoCAD 文档中列出 ⇒ 具名 `break.elliptical_arc_unsupported`；无多段打断（AutoCAD 亦无此模式）；块成员/非模型空间按名拒绝 |
| DIVIDE | **实现** | 选择式：`LINE`/`ARC`/`CIRCLE`/`SPLINE`/`ELLIPSE`/含 bulge `LWPOLYLINE` 按等弧长放 `POINT`；开放 N−1、闭合 N；块选项 `divideUseBlock` + `divideAlignBlock`（Align=Yes 按局部切线旋转，比例 1）；源保留，一条事务/一步撤销 | 非有限/零长/选择数≠1 按名拒绝；POINT 落活动图层，不分割源实体 |
| LENGTHEN | 完整实现 | `DE`/`PER`/`TOT` 从**靠近拾取点**的端点改长度（正延长、负裁剪）；`DY` 拖一端；支持 `LINE`/`ARC`/开放 `LWPOLYLINE`/椭圆弧/开放 `SPLINE`；一条事务/一步撤销 | 开放样条的延长是**声明的切线直尾**（增量精确、接缝 C0/G1、曲率不延续）；闭合对象按名拒绝（`lengthen.closed`），未钳制/切线退化/非正增量各自具名拒绝；首末控制点重合的样条按闭合处理（模型无 closed 标志） |
| SCALETEXT | **选择式**：一个或多个 `TEXT`/`MTEXT`；`scaleTextFactor` 有限 **> 0** 缩放字高，MTEXT 的 `wrap_width: Some(w)` 同步；`1.0` 为具名 no-op；位置/旋转/对齐/样式逐位保留；一条事务/一步撤销 | 混合/非文本整命令拒绝（不部分写入）；无 `Height`/`Matches` 子选项、无逐实体高度输入；通道只拒绝 ≤0/非有限，`1.0` 由命令层拒绝 |
| TORIENT | **选择式**（Express 工具）：裸命令 Most Readable，把每个 Text/MText 的 `rotation` 按 180° 步进归一到 `(-π/2, π/2]`；`TORIENT <angle-degrees>` 设为绝对旋转（有限十进制，可带符号）；只改 rotation，位置/字高/内容/样式/对齐/层/空间/绘制顺序逐位保留；一条事务/一步撤销 | 非 Text（含 MULTILEADER、INSERT 属性、标注文字）逐条 `torient.not_reset`，不整体失败；无变化 ⇒ `torient.unchanged` 且不开事务；属性定义/块属性未建模 |
| TCASE | **选择式**（Express 工具）：裸命令 Title（文档化默认），`TCASE UPPER/LOWER/TITLE/SENTENCE/TOGGLE`（或 `U/L/T/S/G`）显式选择；只改可见字符，MTEXT 格式转义（`\P`、`\f…;`、`\H…;`、`\C…;`、`{}`、`\~`、`\\`）逐字保留；MULTILEADER 文本走 `with_text`；一条事务/一步撤销 | 属性/标注文字未建模 ⇒ `tcase.not_reset`；无变化 ⇒ `tcase.unchanged` 且不开事务；非文本不整体失败 |
| TXT2MTXT | **选择式 v1**：至少一个模型空间 Text/MText；按上→下（Y 降序）、左→右（X 升序）稳定排序，`\P` 原样拼接成一个 `AcDbMText`；沿用首个源的 position/style/font/height/rotation/h_align/v_align/layer/space/draw order/`wrap_width`；同一事务删除全部源；一条事务/一步撤销 | 设置对话框（single mtext/排序/词换行/强制统一行距）未实现，显式 `txt2mtxt.settings_dialog`/`txt2mtxt.uniform_line_spacing`；非 Text 整命令 `txt2mtxt.unsupported`，块成员/子元素 `txt2mtxt.not_addressable`，图纸空间/块空间 `txt2mtxt.unsupported`；不发明宽度 |
| TRIM | 目标 **LINE / ARC / CIRCLE / ELLIPSE / SPLINE / LWPOLYLINE**（SPLINE 目标 2026-10-27 起，见行末；**折线目标 2026-11-12 起**：复用 BREAK/DIVIDE 的折线参数模型，按本项目的 TRIM 契约**保留**含拾取点投影的那一段、其余丢弃——开放折线两条切点保留其中一个窗口（**原地**替换，不产生新实体），闭合折线需两条切点、拾取窗口变成一段开放折线；顶点与 bulge 精确（弧段按 `tan(sweep/4)` 求子 bulge，绝不压成弦）、保持原顶点顺序/方向，一条事务/一步撤销；`trim.polyline_no_intersection` / `trim.polyline_pick_outside` / `trim.polyline_degenerate` / `trim.polyline_nonplanar_bulge` 为具名拒绝，宽度拒绝 `trim.polyline_width_unsupported` 预留（当前存储无宽度字段，不可达））；边界 **LINE / ARC / CIRCLE / ELLIPSE / LWPOLYLINE**（直线段或带 bulge 弧段，可任意混用）**/ SPLINE**。LINE/ARC/CIRCLE/ELLIPSE 及 bulge 弧段用解析圆锥曲线求交（与对象捕捉同一实现）；**SPLINE 边界只走采样**：`intersect_local` 把两个操作数按 `extent × 1e-4` 弦高（每操作数最多 256 段）扁平化后求交，切点误差界为该弦高（10 单位图纸约 `1e-3`），因此**每条真正贡献切点的 SPLINE 对都会额外给一条 `drawing.trim` 诊断**说明切点来自采样近似，绝不冒充解析解。LINE 保留拾取侧子段，ARC 保留与原弧同圆心/法向/半径、**同 sweep 符号**的剩余弧，CIRCLE 变成一段 ARC（拾取点两侧相邻交点之间），一条完全落在闭合边界内的目标整体删除（含 CIRCLE；内侧判定按边界自身平面，倾斜折线不会被 XY 投影误判）。**拾取点先投影到已解析的目标上**（LINE 取垂足；ARC/CIRCLE 取 `center + radius * normalize(pick − center)` 的径向投影），所以**点在实体附近即可工作**：宿主已用拾取框解析出目标，点不必恰好落在曲线上 | **仍拒绝**：`INSERT`/`Opaque`/`Mesh` **目标**（`SPLINE` **目标**自 2026-10-27 起支持，但有两个诚实边界：**切割参数**来自采样求交 ⇒ 新端点最远偏离真实交点 `控制多边形盒对角线 × 1e-4`（≤256 段），而**保留下来的子曲线本身是精确 B 样条**（齐次点上的 Boehm 插入、两端 multiplicity = degree+1、degree/weights/内部节点逐位保留；**绝不**重拟合、**不**改参数化、**不**替换成折线）；且 `Spline` 无 closed 标志 ⇒ 闭合样条的接缝按起/终点处理，不做环绕。子曲线 `knots` 拒绝面：退化跨度、非法存储定义）（边界只有 `INSERT`/`Opaque`/`Mesh` 等未列出的类型被拒）、`|sweep| > 一周` 的 ARC 目标（`Unsupported`）；无交点/退化/**投影落到目标自身参数域之外**（不夹到最近端点——夹紧等于替用户决定保留哪一段）/ARC/CIRCLE 拾取点**径向方向为零**（点在目标轴线上，如圆心）/CIRCLE 交点少于 2 ⇒ `InvalidInput`。**闭合 SPLINE 的内侧删除规则不支持**：`SemanticGeometry::Spline` 既无闭合标志也无边界平面，无法做可靠的平面感知内侧判定，故“目标完全落在闭合 SPLINE 内且无切点”**显式 `Unsupported` 拒绝**（不删除、不臆造包含关系）；其它无解析解的边界对（ARC×ELLIPSE、ELLIPSE×ELLIPSE、非共面圆族）仍出图但同样附采样近似诊断；3D 非共面输入只会“求不到交点”而非被投影凑出结果 |
| EXTEND | 目标 LINE（沿自身无限直线）或 ARC（沿自身圆、**同号 sweep**）；**SPLINE 目标显式拒绝**（2026-10-27 起带具名理由：NURBS 由节点向量定义，移动端点会改变节点向量进而改变整条曲线的参数化，且本构建没有外推策略——不按切线近似冒充）；边界集合同 TRIM（**含 SPLINE**，同样只走采样并附 `drawing.extend` 近似诊断，弦高界同 TRIM）。拾取点同样**先投影到目标上**，投影落点决定延伸哪一端（“点在哪一头”按曲线上的落点判定，与点到曲线的距离无关）；只取拾取端**前方最近**交点，**只增不减**；ARC 的延伸量**上限为半圈**（超过半圈视为“只能绕远路到达”，显式拒绝） | CIRCLE 目标（无端点）、`|sweep| > 一周` 的 ARC、`SPLINE` 等其余**目标**、未支持的边界类型 ⇒ `Unsupported`；已接触/已穿过/前方无交点/延伸超过半圈/投影落在目标参数域之外 ⇒ `InvalidInput`；采样近似同 TRIM 另给 `drawing.extend` 诊断 |
| ROTATE / SCALE | 绕基点旋转/均匀缩放 | 相似变换（正行列式）；不支持几何 `Unsupported`。**含 `MultiLeader` 的选择：旋转可以，缩放 ⇒ 整命令失败**（ADR-0019 C.5：掩码未置位时 AutoCAD 会从样式重解析） |
| MIRROR | 镜像（**保留原件**，新增副本）：含 `Arc` 与 `Ellipse`（**含椭圆弧**，2026-10-26 起） | 反射的 `normal' = −R(normal)`，弧的 `start` 由镜像后的起点在新帧中**重新求角**、`sweep` **符号保留**（负法向已把帧的重手性翻正；若同时取反 sweep，端点映射不成立——两种写法都验证过）。椭圆：圆心/长轴向量反射、`ratio` 与参数保留。仍拒绝 `Text`/`Shape`/`Opaque`/`MultiLeader`：`Text` 只镜像拒绝（平面文字无法表达真反射），`MultiLeader` 拒绝一切缩放 |
| OFFSET | LINE / CIRCLE / **ARC** / **LWPOLYLINE**（直线段自 2026-10-26、**含弧段自 2026-10-28**；ARC 自 2026-10-27）偏移 | 折线整条**同一侧**、常数距离（拾取点在最近段上的 `m = cross(n,u)` 位移；弧段沿同一 `m` **径向**量）。**弧段同心偏移**：`R' = R − s·sign(sweep)`（圆心不变；`R' <= tol` ⇒ `offset.arc_collapsed`，**不删除、不与邻居混合**）。**接点**：line/line 用钉住的 miter；line/circle 与 circle/circle 取**离源顶点最近**的解析交点（0 个或两个等距 ⇒ `offset.join_failed`；**相切按闭式**判定，不会被浮点判别式抖成失败）；弧的 `start`/`sweep` 由接点重新导出并过 `bulge_arc_geometry` 回验（`offset.arc_mismatch`）。**闭合环一趟**（含 wrap 角）；开口折线的弧端**不封口**。按名拒绝：`offset.arc_collapsed`、`offset.join_failed`、`offset.ambiguous_plane`（全弦共线却带 bulge，平面不可定）、`offset.arc_mismatch`、`offset.bulge_segment`（**只剩**非有限或不可构造的 bulge）、`offset.collapsed`（顶点处近 180° 反折或偏移段塌缩/归零）、`offset.non_planar`（顶点偏离自身平面）；退化段/点数 < 2/非有限顶点/闭合零面积/拾取点落在折线上 ⇒ `InvalidInput`。**ARC 与 CIRCLE 共用同一距离模型**（新半径 = `\|拾取点 − 圆心\|` 的**3D** 距离：拾取点在圆外则放大、在圆内则缩小；`center`/`normal`/`start`/`sweep` **逐位保留**，顺时针 sweep 仍为**负**，只有半径变，因此偏移弧覆盖同一角跨）；退化存储半径/拾取点落在圆心/拾取点落在该弧自身圆上（零偏移）⇒ 按名 `InvalidInput`，不创建、不改状态、不记撤销步。**`ELLIPSE` 单独按名拒绝**（椭圆的等距偏移**不是**椭圆，不拿近似冒充）；其余几何（SPLINE/INSERT/…）走通用 `Unsupported`（串中点名 `LINE, CIRCLE, ARC and straight-segment LWPOLYLINE`——**含弧段折线已支持，这串措辞尚未更新**）。**声明的偏差**：**不做** AutoCAD 的弧段删除与邻居重接（半径耗尽即 `offset.arc_collapsed`）；**无全局自交修剪**（只在顶点处检测局部塌缩，远端偏移段之间的自交**未检查**）；**2 顶点（纯弧）或全弦共线的带 bulge 折线** ⇒ `offset.ambiguous_plane`；弧的修剪允许**增长**，上限 `\|new\| <= \|old\| + π`（外凸 miter 会延长弧；严格的 `+tol` 上限会拒绝合法的外偏） |
| COPYCLIP / CUTCLIP / COPYBASE / CUTBASE / PASTECLIP / PASTEORIG | **应用内**剪贴板：两个捕获形状 + 两个粘贴形状（2026-10-28） | 非 OS 剪贴板；替换文档清空；空选择/空剪贴板拒绝。**捕获**：`COPYCLIP` 只读（0 事务，Viewer 可用）、`CUTCLIP` 捕获 + 一次事务删除（一步撤销）；两者**不记基点**（`clipboard_base = None`，陈旧基点不会改写它们）。`COPYBASE`（只读、Viewer 可用）/`CUTBASE`（work-only）在**预选择**之上再抓**一个基点**并存进剪贴板元数据；`CUTBASE` 是**先删除成功、再提交剪贴板与基点**，失败的剪切不毁旧剪贴板。**粘贴**：`PASTECLIP` 是**单点插入点工具**，`delta = 插入点 − 基点`（`base = clipboard_base.unwrap_or(世界原点)`），复用 COPY 的同一个 `cad_db::transform_geometry`——**不再是原坐标粘贴**；`PASTEORIG` 是显式**原坐标**粘贴（忽略基点，等价改动前的 `PASTECLIP`）。两者都是新 id + 一条事务/一步撤销，几何不可平移时**在开事务前整命令拒绝**。**`PASTEBLOCK` 已实现 v1（2026-11-01，`Ctrl+Shift+V`）**：应用内剪贴板 → 自动命名 `PasteBlockN` 块 + 一个插入点参照；成员为新 id 克隆、保持原坐标与显示属性；基点 = `clipboard_base` 或世界原点；一条事务/一步撤销；剪贴板不清空、成功后选中新参照；块成员/图纸空间条目按名拒绝。**不是** OS 剪贴板，也不写 `*U` 匿名块 |
| MATCHPROP（别名 `MA`） | **实现** | 选择式：首个选中为源；Basic 六项（layer/color/linetype/lineweight/transparency/linetype scale）+ Settings 子集；Special 组按目标类：Text→文字样式+字体、Dimension→标注样式、Hatch→填充样式/图案/渐变；一条事务/一步撤销 | 字高、标注覆盖值、填充原点**不**复制（与 AutoCAD 一致）；plot style/thickness 未建模（全局范围）；异类目标跳过该组并报告 |
| REVCLOUD | **对象模式 v1（2026-11-07）**：恰好 1 条选中的**闭合、全直线段** LWPOLYLINE；每条边换成一段**向外**弧（bulge 幅度 `0.5`，sweep ≈ 106.26°；CCW 环 `+0.5`、CW `-0.5`），实体**原地重写**、一条事务/一步撤销 | 开放折线/已有 bulge/非折线/`+Z` 外/非有限/零面积/多选或空选 ⇒ 具名拒绝；**无**矩形/多边形交互绘制、无 `Arc length` 参数、无对象拾取模式；不是 AutoCAD 的全功能 REVCLOUD |
| SELECTSIMILAR | **实现** | 对象类型强制；默认**图层+类型**；Color/Linetype/Lineweight/Name/Object style 独立开关；`Name` 按根块参照名匹配展开成员；模型/图纸空间各自匹配；只读 | 透明不参与（AutoCAD 亦不参与）；块定义不是图形对象；空选择/跨空间种子按名拒绝 |
| MODEL | **payload-free（2026-11-07）**：等价于 `SPACE MODEL`，复用同一校验 + 自动取景；会话状态 only，无数据库写/历史/revision；Model 标签页直接派发 `MODEL` | 只切**已存在的模型空间**；不是第二套空间状态；图纸标签仍走 `SPACE #<id>` |
| PLAN | **Top/plan 2D 视图（2026-11-07）**：payload-free，等价于 `StandardView::Top`；已在 2D 时是具名 no-op，不记 `ZOOM PREVIOUS` | **无** UCS/World/Current 选项（本构建无 UCS 模型）；不是任意 UCS 平面视图 |
| AUDIT | **只检测（2026-11-07）**：逐实体调用 `cad_db::validate_entity`，输出 `audit.clean` 或 `audit.summary` + 每个 `audit.invalid_entity`；只读，无事务/历史/revision | **不修复**（AutoCAD 的「corrects some errors」未实现）；不删除、不改写任何实体；只覆盖实体级校验，不是完整 DWG 修复器 |
| JOIN | 合并当前选择为**一个**实体：共线且首尾相接/重叠的 `LINE` → 一条 `LINE`；同圆心/法向/半径且扫掠相续的 `ARC` → 一条 `ARC` | 一条事务、**一步撤销**，全有或全无（先全部校验，再删除 N + 插入 1）。**仅 LINE 与 ARC**：折线/样条/圆、需要桥接空隙、跨图层/跨空间、非同心弧、块内成员 ⇒ 显式拒绝（`join.needs_two`/`join.mixed_kinds`/`join.not_collinear`/`join.not_contiguous`/`join.layer_mismatch`/`join.space_mismatch`/`join.sweep_overflow`/`join.degenerate`）。合并 `LINE` 的端点按 `(x,y,z)` 升序归一（同一批拾取总得同一几何）；`draw_order` 取源最小值（原位替换，不跳到最上层）；扫掠和恰为一整圈时**保留 `2π` ARC**，不静默转成 `CIRCLE`；反向法向的弧按「不相续」拒绝而非重新推导 |

| QTEXT | **完整实现（2026-11-16）**：`QTEXT` payload-free，翻转会话令牌；`qtext` 经 `DisplaySettings` → `RepresentationContext::qtext` 到达派生批次，字形片段被替换为按其本应绘制几何 bbox 的闭合 `Lines` 矩形；会话状态 only，无事务/修订/历史，Viewer-legal | 无法推导范围的文字保留原绘制并报 `qtext.extent_unknown`（不编造宽度）；出图与显示共用同一上下文；无像素证据 |
| FILL | **完整实现（2026-11-16）**：翻转 `fill`（默认 ON），经 `DisplaySettings` → `RepresentationContext::fill` 到达派生批次；`FILL OFF` 时 HATCH 画边界环、`AcDbSolid` 画奇偶边界边、宽折线画闭合偏移轮廓；矢量出图同路径 | 声明不切换：裸内核/B-rep 网格、标注箭头、WIPEOUT 遮罩（自有令牌）、图像；会话状态 only，无库写/无撤销；无像素证据 |
| TFRAMES | **完整实现（2026-11-24）**：翻转会话 `tframes`（默认 ON），经 `DisplaySettings::frames_visible` → `RepresentationContext::image_frames`（图像边框）与 `wipeout_frames && frames_visible`（遮罩边框）到达派生批次；ON 显示光栅图像边框并按 `WIPEOUTFRAME` 显示遮罩边框，OFF 同时隐藏两者而 WIPEOUT 仍遮罩、图像仍绘制纹理；翻转在 `SceneVersion` 内，基场景重建一次；会话状态 only，无库写/修订/历史，Viewer-legal | 外部参照边框**未建模**（xref 是展开的块几何，无 XCLIPFRAME/边框实体可切换），命令按名声明；无纹理的图像仍保留占位边框与 `image.*` 诊断；出图不跟随该显示令牌（出图各自解析边框）；无 GPU/像素证据 |
| REGENAUTO | **会话令牌 v1（2026-11-09）**：翻转 `regenauto`；会话状态 only，Viewer-legal | 本构建无自动重生成开关；需显式 `REGEN` | 近似实现：仅命令路径/状态 |
| ISODRAFT | **会话令牌 v1（2026-11-09）**：翻转 `isodraft`；会话状态 only，Viewer-legal | 不切换等轴测平面或光标、不改变捕捉/正交行为 | 近似实现：仅命令路径/状态 |
| BCOUNT | **完整实现（2026-11-24）**：统计整图（模型空间 + 各图纸布局）的 `SemanticGeometry::Insert` 实例，按块名分组，输出 `bcount.block` + `bcount.summary`；选择集非空时只统计所选块参照（`bcount.non_block_ignored`/`bcount.unresolved` 按名报告）；块定义内嵌套参照按 `bcount.nested_not_counted` 列名不计数（与 AutoCAD 的 BCOUNT 限制一致）；无事务/修订/历史，Viewer-legal | 无 AutoCAD 的 `Select objects` 交互提示（ADR 全局范围；宿主选择集已是同一作用域）；不修改图形 | 只读，无写 |
| COUNT | **只读报告 v1（2026-11-09）**：当前选择非空时按选择统计，否则统计全部模型空间顶层实体；按 `type_key` 分组，输出 `count.type`/`count.summary`；块成员/子元素/失效引用另报 `count.unresolved`；不改选择、无事务/修订/历史，Viewer-legal | 不实现 AutoCAD 的计数高亮/导航面板；不统计块内成员 | 只读，无写 |
| EDITTIME | **只读报告 v1（2026-11-09）**：当前 UTC 时钟 + 文档 id/修订/实体数（`edittime.report`）；创建/修改时间与会话用时恒 `edittime.not_tracked`；Viewer-legal | 数据库不存时间戳，会话不跟踪 uptime；不伪造日期 | 只读，无写 |
| JULIAN | **完整实现（2026-11-24）**：裸命令报当前 UTC 时刻的儒略日（整数日 + 小数，`julian.value`）；`JULIAN <YYYY-MM-DD>`（也接受 `YYYY/MM/DD`，可跟 `[T ]HH:MM[:SS]`）把日历时刻（UTC，前推格里高利历）转换为儒略日；非法/不存在的日期按 `julian.invalid_date` 拒绝，不 clamp；Viewer-legal | 无交互 DATE 窗口；LISP 转换函数（CTOJ/DTOJ/JTOC/JTOD/JTOW）不建模 | 只读，无写 |
| XLIST | **只读报告 v1（2026-11-09）**：列出 `drawing.blocks()` 的每个块定义与成员数（`xlist.block` + `xlist.summary`）；`xlist.xdata_not_retained` 明说 XDATA 不保留；Viewer-legal | 不是 AutoCAD 的“列出嵌套对象类型/块名/图层/颜色/线型”全格式；不列出 xref 嵌套、不读 XDATA | 只读，无写 |
| DRAGMODE | **会话令牌 v1（2026-11-09）**：payload-free，翻转 `drafting_extras.dragmode`（默认 ON），`dragmode.toggled`；会话状态 only，无事务/修订/历史，Viewer-legal | 渲染器不消费该标志：本构建无拖动预览，不显示拖动中的图像 | 近似实现：仅命令路径/状态 |
| APERTURE | **只读报告 v1（2026-11-09）**：报告会话靶框令牌（默认 10），`aperture.value`；Viewer-legal | `APERTURE <n>` 设置形式 **NOT IMPLEMENTED**；不绘制捕捉靶框，也不改变捕捉判定 | 只读，无写 |
| ISOPLANE | **会话令牌 v1（2026-11-09）**：循环 `LEFT→TOP→RIGHT`，`isoplane.toggled`；会话状态 only，Viewer-legal | 无等轴测光标、无捕捉/正交变化，绘图平面不真正切换 | 近似实现：仅命令路径/状态 |
| HIGHLIGHTNEW | **会话令牌 v1（2026-11-09）**：翻转 `highlightnew`（默认 ON），`highlightnew.toggled`；Viewer-legal | 不渲染新功能高亮（无橙点） | 近似实现：仅命令路径/状态 |
| TRAYSETTINGS | **完整实现（2026-11-24）**：翻转会话 `traysettings`（默认 ON），经 `cad_status.traySettings` 投影；状态栏托盘（右侧图标/通知簇）跟随该旗标显示/隐藏，托盘设置面的唯一开关即该旗标；紧凑布局无状态栏时键入按名拒绝（`traysettings.refusal.no_status_bar`）；Viewer-legal，无库写/修订/历史 | 声明的偏差：本构建的托盘只含自己的状态栏右侧控件簇，不模拟 AutoCAD 的托盘图标清单；无 GUI/像素证据 |
| TEXTSCR | **完整实现（2026-11-24）**：置会话文本屏幕旗标（`cad_status.textScreen`），共享 chrome 的文本屏幕窗口随之显示本次会话的命令历史；正文复用命令行历史的同一折叠与序列化（`commandWindowRows` + `commandWindowText`），**无第二套历史模型**；`GRAPHSCR`/Esc 经 router 回写旗标；Viewer-legal，无库写/修订/历史 | 无历史通道时显式说明（`textscreen.unavailable`）而不是空窗；窗口/像素证据 NOT VERIFIED |
| GRAPHSCR | **完整实现（2026-11-24）**：清除会话文本屏幕旗标（`cad_status.textScreen`），共享 chrome 的同一文本屏幕窗口关闭并返回图形；Viewer-legal，无库写/修订/历史 | 与 TEXTSCR 共用同一窗口与旗标（无第二套状态）；窗口/像素证据 NOT VERIFIED |
| FINDINCLUDE | **完整实现（2026-11-24）**：会话令牌 `find_include`（默认 ON）是 `-FIND`/`FIND` 的块通道开关；ON 搜索块定义与**已加载**外部参照物化出的块（按 `external reference 'NAME'` 标注），OFF 只搜索模型空间与各图纸布局，并在 `find.result` 写明 `block-definition and external-reference text excluded by FINDINCLUDE OFF`；未加载/无块的参照无论令牌如何都以 `'NAME' (missing|refused-path|...)` 列为「content not attached」；Viewer-legal，无库写/修订/历史 | 声明边界：外部参照自身文件不做跨文件解析（只搜索导入已物化的内容）；无 GUI/像素证据 |
| FINDMATCH | **完整实现（2026-11-24）**：翻转会话 `find_match_case`（默认 OFF），`-FIND`/`FIND` 的**报告与替换共用**该标志：ON 按精确字节（区分大小写），OFF 按 ASCII 不区分大小写；覆盖 MTEXT（按投影文本）、单行 TEXT、MULTILEADER 与表格单元格；Viewer-legal，无库写/修订/历史 | 比较是字面子串：无正则、无 Unicode 折叠/排序规则（ASCII 大小写是声明的唯一折叠）；无 GUI/像素证据 |
| REVCLOUDPROPERTIES | **只读报告 v1（2026-11-09）**：报告会话弧长令牌（默认 15），`revcloud.properties`；Viewer-legal | `REVCLOUDPROPERTIES <length>` 设置形式 **NOT IMPLEMENTED**；`REVCLOUD` 不读取该令牌 | 只读，无写 |
| SYSVARMONITOR | **会话监视集（2026-11-12）**：`ADD`/`REMOVE`/`LIST [<name>]`；`LIST`/裸命令对比上次快照，变化报 `sysvarmonitor.changed`，并报 `sysvarmonitor.value`/`.summary`；Viewer-legal | 仅在被查询时检测变化，没有后台通知；闭集之外的变量不建模也不杜撰 | 只有会话状态，无写 |
| SYSVDLG | **完整实现（2026-11-24）**：裸命令报告全部已建模变量（`sysvdlg.value`/`.summary`/`.not_tracked`）；`SYSVDLG <name>` 读取单值、`<name> <value>` 编辑、`SAVE|RESTORE <file.json>` 保存/恢复版本化 `yacr.sysvar` JSON（宿主 named-file 桥；多条 header 值经 `apply_header_values` 一条 revision）；未建模变量按名字拒绝，文件未知变量/重复名/非法值在写入前整体拒绝；Viewer 可报告/读取但拒绝编辑/保存/恢复 | 无交互对话框（ADR 全局范围）；**不是 AutoCAD `.svf`**（自有 JSON 格式，目标扩展名必须 `.json`）；保存省略无值变量，恢复不清空；只有 15 个已建模变量 | 编辑/恢复写 header 或会话状态；header 写一条 revision、无 undo patch；`SAVE` 写宿主文件 |
| VTOPTIONS | **只读报告 v1（2026-11-09）**：`vtoptions.not_tracked` 明说视图过渡设置未建模（代码串按 Wave 2 契约原样保留）；Viewer-legal | 不报告也不杜撰平滑/时长/缩放平移过渡选项 | 只读，无写 |
| ALIASEDIT | **完整实现（2026-11-24）**：`ADD <alias> <target>`/`REMOVE <alias>`/`LIST`/bare；运行时别名先于静态表解析并遮蔽内建，仅删除运行时行；`ADD`/`REMOVE` 经宿主 `preference.json` 根键 `commandAliases` 持久化并在新会话恢复；写入失败按 `aliasedit.not_persisted` 回滚；目标失效/冲突的持久行启动时按名丢弃并记诊断 | 无交互对话框（ADR 全局范围）；不解析 `acad.pgp` 文件 | 写宿主偏好（不是绘图）；`LIST`/裸命令只读 |
| WHOHAS | **只读报告 v1（2026-11-09）**：`whohas.single_document`；Viewer-legal | 单文档构建：无网络查询，无法列出其他用户、其他打开文档或文件锁 | 只读，无写 |
| ACADINFO | **部分实现（只读报告，2026-11-24 扩充）**：产品版本 + debug/release 构建配置 + 真实目标 OS/架构 + 文档修订/实体数/单位/精度/应用域（`acadinfo.report`）；`acadinfo.not_collected` 明说许可证/硬件/模块枚举不收集（全局范围）；`acadinfo.file_not_written` 明说本构建不创建信息文件；Viewer-legal | **缺口**：AutoCAD 命令创建信息文件，本 crate 无宿主文件 sink，不写文件；许可证/硬件/模块枚举属全局范围 | 只读，无写 |
| PTYPE | **只读报告 v1（2026-11-09）**：报告 `$PDMODE`/`$PDSIZE`（`ptype.report`）；Viewer-legal | 写入仍只走现有 `PDMODE`/`PDSIZE`；PTYPE 对话框形式 **NOT IMPLEMENTED** | 只读，无写 |
| SCALELISTEDIT | 完整实现 | 真实 `ACAD_SCALELIST`：只读报告（行、活动 `CANNOSCALE`、首个图纸布局页面）+ `ADD`/`RENAME`/`DELETE` 写（各一条记录/一次修订，Viewer 拒绝写）；`RENAME` 级联 `CANNOSCALE` 与全部注释性覆盖；`DELETE` 按名字引用拒绝（`scalelistedit.in_use` 报两处计数，并说明视口/打印比例是变换不是名字引用）；导出回写并有往返测试 | `RESET`（无权威默认比例模板）与模态对话框未实现；无撤销步（`scalelistedit.no_history`）；含 `"` 的比例名无法由命令行键入（拒绝而非静默删引号），表/改名/删除/导出仍可处理该行 |
| HYPERLINKOPTIONS | **具名拒绝 v1（2026-11-09）**：`CadError::Unsupported` / `hyperlinkoptions.unsupported: this build has no hyperlink storage model`；不写库/不升修订/不入历史，Viewer-legal | 无超链接存储模型：不保存、不显示任何超链接 | 拒绝，无写 |
| INPUTSEARCHOPTIONS | **具名拒绝 v1（2026-11-09）**：`inputsearchoptions.unsupported: input-search settings are not modeled`；Viewer-legal | 输入搜索设置未建模（对话框形式） | 拒绝，无写 |
| -INPUTSEARCHOPTIONS | **具名拒绝 v1（2026-11-09）**：`-inputsearchoptions.unsupported: input-search settings are not modeled`；本构建的**独立选择子**（不是别名）；Viewer-legal | 同上，命令行拼写 | 拒绝，无写 |
| SIGVALIDATE | **具名拒绝 v1（2026-11-09）**：`sigvalidate.unsupported: this build has no digital-signature model`；Viewer-legal | 无数字签名模型：不读取、不校验签名 | 拒绝，无写 |
| BASE | 完整实现 | `BASE` 报告已存基点，`BASE <x> <y> [<z>]` 写 `$INSBASE`（一条记录/一次修订）并导出回写；`SETVAR INSBASE` 可读；Work-only | 导入侧尚未读 `$INSBASE`（`base.not_stored` 明说）；无撤销步（`base.no_history`） |
| LIMITS | 完整实现 | `LIMITS <minx> <miny> <maxx> <maxy>` 一次修订写两角；`SETVAR LIMMIN/LIMMAX` 保留另一角；导出回写两个头变量（半存 `export.note.limits_half_stored`）；Work-only | `LIMITS ON\|OFF`（LIMCHECK）具名拒绝（本构建无点钳制路径）；无撤销步（`limits.no_history`） |
| VIEWPLOTDETAILS | **具名拒绝 v1（2026-11-09）**：`viewplotdetails.unsupported: no plot-details model`；Viewer-legal | 无打印详细信息模型：不列出已完成打印/发布任务 | 拒绝，无写 |
| VPLAYER | **具名拒绝 v1（2026-11-09）**：`vplayer.viewport_layer_state_unsupported: no per-viewport layer state`；Viewer-legal | 无逐视口图层状态 | 拒绝，无写 |
| MVIEW | **具名拒绝 v1（2026-11-09）**：`mview.viewport_creation_unsupported: no viewport creation API`；Viewer-legal | 无创建视口 API（现有 `VPORTS` 等路径不创建视口） | 拒绝，无写 |
| PSBSCALE | 完整实现 | 同一 paper/model 因子作用于块参照与 `MULTILEADER`/`TABLE` 文本（标注文字高 + 宽度边界 + 样式快照高度，蒙版/非蒙版读法不分歧；根/引线/箭头/平面、列宽/行高/合并/边框不动）；独立 `TEXT` 按 `psbscale.not_block` 拒绝；一条事务/一步撤销 | 退化长度 `psbscale.degenerate`、非法结果 `psbscale.invalid_result` |
| PSTSCALE | 完整实现 | 同一因子作用于 `TEXT`/`MTEXT`（高度 + MTEXT wrap 宽度）与 `MULTILEADER`/`TABLE` 文本（含样式快照高度）；块参照 `pstscale.not_text` 拒绝；一条事务/一步撤销 | 退化 `pstscale.degenerate`、非法结果 `pstscale.invalid_result` |
| MVSETUP | **具名拒绝 v1（2026-11-09）**：`mvsetup.unsupported: no viewport-group model`；Viewer-legal | 无视口组模型 | 拒绝，无写 |
| MSPACE | **会话切换 v1（2026-11-09）**：payload-free，复用 `MODEL`/`SPACE` 的 `switch_space_command` 切到模型空间，`mspace.switched`；已在模型空间为 `mspace.noop`；会话状态 only，无数据库写/修订/历史，Viewer-legal | 不是第二套空间权威；不创建或移动视口，不改变图纸空间内容 | 会话状态，无写 |
| PSPACE | 完整实现 | 复用同一 `switch_space_command`：切到活动图纸布局；**已在图纸空间时是 `pspace.noop`**（保留布局与相机，位级不变）；无图纸布局 `pspace.no_layout`；不写库/不升修订/不入历史、不动 `CVPORT`；Viewer-legal | 无交互视口双击切换 |
| STYLE | **只读报告 v1（2026-11-09）**：列出 `drawing.styles()` 每个文字样式（名称/id/资源键数，`style.entry` + `style.summary`）；Viewer-legal | 创建或修改文字样式 NOT IMPLEMENTED；不解析字体文件内容 | 只读，无写 |
| -STYLE | **只读报告 v1（2026-11-09）**：与 `STYLE` 同一列表，使用 `-style.entry` + `-style.summary`；本构建的**独立选择子**（不是别名）；Viewer-legal | 编辑文字样式 NOT IMPLEMENTED | 只读，无写 |
| MLEADERSTYLE | **只读报告 v1（2026-11-09）**：列出 `drawing.mleader_styles()`（名称/content type/字高/箭头/文字样式，`mleaderstyle.entry` + `mleaderstyle.summary`）；Viewer-legal | 创建、修改或设当前样式 NOT IMPLEMENTED；没有样式对话框 | 只读，无写 |
| BLOCK? | **只读报告 v1（2026-11-09）**：列出 `drawing.blocks()` 每个块定义的名称与成员数（`block.entry` + `block.summary`）；Viewer-legal | 只读列表，不是块编辑器；不列嵌套/xref/XDATA | 只读，无写 |
| DIMREASSOC | **部分实现**：`dimreassoc.non_associative`；不写库/不升修订/不入历史，Viewer-legal | Express Tool「恢复被覆盖/修改的标注文字到测量值」未实现：标注文字覆盖（`DIMEDIT` 组 1）已建模，本命令却不恢复测量值 ⇒ **真实可写缺口**，不是关联性全局范围 | no-op，无写 |
| DIMREASSOCIATE | **完整实现（2026-11-24）**：`dimreassociate.non_associative`；关联性属 ADR 全局范围（本构建标注全部非关联），无对象/点关联可建立即该命令语义的全部；不写库/不升修订/不入历史，Viewer-legal | 无交互 Select objects；关联性（`DIMASSOC=2`）属全局范围，不是该命令的独立缺口 | no-op，无写 |
| DIMJOGGED | **具名拒绝 v1（2026-11-09）**：`dimjogged.unsupported: the dimension domain stores no jog field`；Viewer-legal | 标注域不存储折弯字段：不创建折弯标注 | 拒绝，无写 |
| DIMINSPECT | **具名拒绝 v1（2026-11-09）**：`diminspect.unsupported: the dimension domain stores no inspection data`；Viewer-legal | 标注域不存储检验数据 | 拒绝，无写 |
| -DIMINSPECT | **具名拒绝 v1（2026-11-09）**：`-diminspect.unsupported: the dimension domain stores no inspection data`；Viewer-legal | 同一拒绝的命令行拼写；不存储检验数据 | 拒绝，无写 |
| ANNORESET | **具名拒绝 v1（2026-11-09）**：`annoreset.unsupported: this build has no annotative-object model`；Viewer-legal | 无注释性对象模型：不能重置替代比例表示的位置 | 拒绝，无写 |
| ANNOUPDATE | **具名拒绝 v1（2026-11-09）**：`annoupdate.unsupported: this build has no annotative-object model`；Viewer-legal | 无注释性对象模型：不能按样式更新注释性对象 | 拒绝，无写 |
| OBJECTSCALE | **具名拒绝 v1（2026-11-09）**：`objectscale.unsupported: this build has no annotative-object model`；Viewer-legal | 无注释性对象模型：不能增删支持比例 | 拒绝，无写 |
| -OBJECTSCALE | **具名拒绝 v1（2026-11-09）**：`-objectscale.unsupported: this build has no annotative-object model`；Viewer-legal | 同一拒绝的命令行拼写；无注释性对象模型 | 拒绝，无写 |
| QLATTACH | **具名拒绝 v1（2026-11-09）**：`qlattach.unsupported: this build has no QLEADER annotation-attachment model`；Viewer-legal | 无 QLEADER 注释附着模型：不能把引线附到 MText/公差/块参照 | 拒绝，无写 |
| QLATTACHSET | **具名拒绝 v1（2026-11-09）**：`qlattachset.unsupported: this build has no QLEADER annotation-attachment model`；Viewer-legal | 无 QLEADER 注释附着模型：不能全局附着引线 | 拒绝，无写 |
| QLDETACHSET | **具名拒绝 v1（2026-11-09）**：`qldetchset.unsupported: this build has no QLEADER annotation-attachment model`；Viewer-legal | 无 QLEADER 注释附着模型：不能全局分离引线 | 拒绝，无写 |
| `ARRAY` / `-ARRAY` / `ARRAYCLASSIC` | 部分实现 | 路由 + **键入形式** `ARRAY RECT <rows> <cols> <row-spacing> <col-spacing>` / `ARRAY POLAR <items> <fill-angle> <center-x> <center-y>`，写入规划器已读的同一会话通道并重派发既有载荷；三种拼写证明是同一命令（`array_router`）；拒绝 `array.count`/`array.spacing`/`array.angle`/`array.cap`（规划器常量）/`array.selection` | **关联性、`Levels`、旋转网格（`ANGLE`）、`Path` 无模型/无通道**；`ARRAYCLASSIC` 的旧式对话框属 chrome/全局 |
| ARRAYEDIT | **具名拒绝 v1（2026-11-09）**：`arrayedit.unsupported: this build has no associative-array edit state`；Viewer-legal | 无关联阵列编辑状态：不能编辑阵列源对象 | 拒绝，无写 |
| BEDIT | **具名拒绝 v1（2026-11-09）**：`bedit.unsupported: this build has no block editor`；Viewer-legal | 无块编辑器：不能打开块定义进行编辑 | 拒绝，无写 |
| -BEDIT | **具名拒绝 v1（2026-11-09）**：`-bedit.unsupported: this build has no block editor`；Viewer-legal | 同一拒绝的命令行拼写；无块编辑器 | 拒绝，无写 |
| BCLOSE | **具名拒绝 v1（2026-11-09）**：`bclose.unsupported: this build has no block editor`；Viewer-legal | 无块编辑器可关闭 | 拒绝，无写 |
| BCYCLEORDER | **具名拒绝 v1（2026-11-09）**：`bcycleorder.unsupported: this build has no dynamic-block grip model`；Viewer-legal | 无动态块夹点模型：不能改变夹点循环顺序 | 拒绝，无写 |
| BGRIPSET | **具名拒绝 v1（2026-11-09）**：`bgripset.unsupported: this build has no dynamic-block parameter model`；Viewer-legal | 无动态块参数模型：不能创建/删除/重置参数夹点 | 拒绝，无写 |
| BESETTINGS | **具名拒绝 v1（2026-11-09）**：`besettings.unsupported: this build has no block editor`；Viewer-legal | 无块编辑器设置对话框 | 拒绝，无写 |
| BTESTBLOCK | **具名拒绝 v1（2026-11-09）**：`btestblock.unsupported: this build has no block editor`；Viewer-legal | 无块编辑器：不能测试动态块 | 拒绝，无写 |
| LAYERPALETTE | **具名拒绝 v1（2026-11-09）**：`layerpalette.unsupported: there is no separate layer palette; use LAYER`；Viewer-legal | 本构建没有独立图层调色板；请使用现有 `LAYER` | 拒绝，无写 |
| CONSTRAINTBAR | **具名拒绝 v1（2026-11-09）**：`constraintbar.unsupported: this build has no constraint model`；Viewer-legal | 无约束模型：不能显示/隐藏几何约束栏 | 拒绝，无写 |
| CONSTRAINTSETTINGS | **具名拒绝 v1（2026-11-09）**：`constraintsettings.unsupported: this build has no constraint model`；Viewer-legal | 无约束模型：不能控制约束栏显示 | 拒绝，无写 |
- 变换/偏移的**不支持几何**（`Spline`/`Shape`/`Opaque`/`Compound`；**以及 `MultiLeader`——拒绝一切缩放**）返回显式
  （**更正**：`Text` 自 P7a 起**可**平移/旋转/均匀缩放，**不在**此列表；仅镜像与非均匀缩放拒绝。
  **2026-10-26 起 `MIRROR` 的 `Arc`/`Ellipse` 不再被拒**，见上表 MIRROR 行。）
  `Unsupported`；退化/无交点等 plan 级失败返回 `InvalidInput`。二者都不创建或部分修改实体。
- 命令行、别名、固定键位与补全行为的完整参考见 **`docs/shortcuts.md`**；`Ctrl+C/X/V` 仅为应用内
  剪贴板，不读写操作系统剪贴板。
- **MATCHPROP 的顺序规则（契约）**：选择集合的顺序就是输入——`SelectionSet` 保留用户拾取顺序
  （`crates/cad-app/src/selection.rs`，重复对象按首次出现去重），命令取 `refs()[0]` 为源、其后为
  目标，**不按 id/handle 排序**。因此同一拾取序列总是同一个源；想换源就换选择顺序。
  AutoCAD 的**交互式**流程（先选源、再逐个选目标并显示“特性刷”光标）**未实现**——它需要新的
  draw-tool 类型与绘制光标，本项只实现选择式变体。命令无 payload（`CommandPayload::None`），
  选择在应用时从会话读取。
- **保存与导出分离（ADR-0041）**：`SAVE` 只写原生 `.ymodel`（无损、可再次打开）；导出其它格式是显式
  `EXPORT`，目标扩展名决定写出器。`EXPORT` 选 `.dxf` 时是**有损 DXF 导出**（`Partial`，逐条记录转换/丢弃，
  **非** DWG 往返保存）；**块定义 + `INSERT`（含嵌套、动态块活动可见性、非均匀缩放）与 XCLIP 裁剪链已可写出**，
  损失面收窄为「无法表达为单条 INSERT 的摆放（镜像/倾斜/剪切/零或非有限缩放/非仿射/定义环/嵌套超限/xref/重名）」
  与「inverted 裁剪、裁剪深度面」。`EXPORT` 选 `.dwg` 时写入**实验性 DWG 导出**（上游 `acadrust::DwgWriter`）：
  有损、不完整；与旧设计不同，**覆盖目标（包括被打开的源文件）是允许的**，安全性由原子写的 `.bak` 兜底
  （旧行为的 `<stem>-export.dwg` 预填名、`save.dwg_confirm` 确认框与 `refused_source_path` 拦截已删除）。
  成功状态仍标注 experimental 与 `Partial`。**证据只有合成的 acadrust self-roundtrip
  （writer→reader），AutoCAD/vendor 兼容性 NOT VERIFIED**。`PLOT` 为矢量 SVG/PDF。真实图纸上的
  导出兼容性**未验证**。逐类：LWPOLYLINE（平面的）/ELLIPSE/POINT 为精确映射；**SPLINE 记为
  `Converted`**（`DxfSpline`，几何保留，**closed/periodic 标志不携带**）；Shape/Image/Mask/Opaque
  被丢弃并逐条记录。

- **2026-10-26 新增命令的运行时证据边界**：`ID` / `JOIN` / `LAYISO` / `LAYUNISO` 的 UI 入口
  （ribbon 按钮、命令目录、工具提示、命令窗口的 `point` 回显行）目前只有**源码结构/纯函数契约 +
  Rust 契约测试**证据；真实窗口点击、像素判据、与 AutoCAD 的逐项对照**均 NOT RUN**。`point` 回显行的
  单位后缀沿用既有规则（`unit:null` 不加后缀，见上文测量偏离），**不得**据此宣称已实现「图纸单位」文案。

### 创建命令（ADR-0015，逐条按实施状态）

| 命令 | 状态 | 实体 | 显式未实现 |
|---|---|---|---|
| `PLINE` | **实现** | 开放 LWPOLYLINE（每顶点 bulge=0） | 弧段/`Angle`/`Center`/`Direction`/`Radius`/`Second pt`/`Halfwidth`/`Width`/`Close`/`Length`/顶点 `Undo`；3D/OCS |
| `RECTANG` | **实现** | 闭合 LWPOLYLINE（4 顶点，bulge=0） | `Chamfer`/`Fillet`/`Thickness`/`Elevation`/`Width`/`Area`/`Dimensions`/`Rotation`（含圆角会话内持久怪癖）；3D/OCS |
| `ELLIPSE` | **实现** | 单个 `Ellipse`（`center`、`+Z`、中心相对 `major_axis`、`ratio = 短/长 ≤ 1`、`start=0`、`sweep=2π`） | 轴端点法；`Center` 法/`Arc`/`Rotation`/`Isocircle`/`PELLIPSE`；**长短轴重排为未对齐假设**：本仓库把较长者存为长轴以保证 DXF 组 40 ≤ 1，AutoCAD 确切重排规则**未查证**；预览为有界弦近似（16–128 段），存储实体精确 |
| `POLYGON` | **实现** | 单个闭合 `Polyline`（n 顶点、bulge=0、`+Z`），**内接于圆**；第一顶点在"中心→半径点"射线上（指针方向决定旋转） | `Edge` 分支与**外切**模式；"数值半径 ⇒ 底边对齐捕捉旋转角"；边数**不跨会话持久化**（会话参数，默认 4，范围 3–1024，越界显式拒绝） |
| `POINT` | **实现** | 单个 `Point`（`+Z` 平面）；**单次放置**（AutoCAD 是否重复放置**未查证** ⇒ 不按猜测实现）；显示走 overlay 标记，并读取图纸的 `$PDMODE`/`$PDSIZE`（见上方 POINT 绘制列）；**样式可改**：`PDMODE <n>` / `PDSIZE <n>` 写图纸头值（一次 revision 提升，显示立即刷新；**不进历史**、**不可撤销**；导出写入 `$PDMODE`/`$PDSIZE`，`SAVE→OPEN` 往返已用本仓库读写器验证——不是 vendor 验证） | `PDMODE 0` 的真实点（Lines-only 拓扑下是 `+` 近似）、**`SETVAR` 间接寻址**（只支持这两个名字，无任意头变量设置、无 `SETVAR ?` 查询）、Multiple Points 重复放置、**3D 视图中的点标记** |
| `SPLINE` | **实现** | fit 点法（**≥4** fit 点）→ 三次/clamped/chord **插值** NURBS（`degree=3`、`weights=1`）；预览为 ≤256 段有界弦近似 | `Method(CV)`/`Knots`/`Degree`/`Start-End Tangency`/`Tolerance`/`Close`/`Object` 转换；**2–3 个 fit 点**；**fit 点数据不参与 SAVE→OPEN 往返**（domain 无该字段） |

- 创建命令均为 **Work 模式**（Viewer 显式拒绝）、**模型空间**（插入漏斗固定 `SpaceId::Model`）、
  **一次事务 + 一步撤销**；未支持选项一律显式拒绝，不用近似静默顶替。

### 编辑命令（ADR-0016，逐条按实施状态）

| 命令 | 状态 | 结果 | 显式未实现 / 有意偏离 |
|---|---|---|---|
| `FILLET` | **实现** | 两条**独立直线**：`R>0` ⇒ 两段修剪到切点 + **新增相切 ARC**；`R=0` ⇒ 只延伸/修剪到角点、不产生弧。新实体在**活动图层**（我们的选择，AutoCAD 规则未查证）。**Polyline 模式不在本命令里**：它自 2026-10-28 起是独立选择子 `FILLET POLYLINE`（见下两行） | `Multiple`、命令内 `Undo`、`Trim/No Trim` 开关、**平行线自动调半径**（改为显式拒绝）、椭圆/样条曲线对、3D、hatch 关联性；**圆心公式为几何推论（UNVERIFIED）**；Circle/Arc 两两组合已实现（`corner_curve.rs`） |
| `CHAMFER` | **实现** | 两条独立直线：两距离**按拾取顺序**施加；`0/0` ⇒ 只做角点（与 `FILLET R=0` 有等价性对照测试）；否则两段修剪 + **新增 LINE**；新实体在**活动图层**（我们的选择，AutoCAD 规则未查证）。**Polyline 模式不在本命令里**：见 `CHAMFER POLYLINE` | `Angle` 方法/`CHAMMODE`、`Multiple`、`Undo`、`Trim/No Trim`、弧段替换、3D；**负距离行为未查证**（一律拒绝） |
| `FILLET POLYLINE` | **实现**（Polyline 模式的选择式变体，2026-10-28） | **恰好 1 条**选中实体：**开放**、≥3 顶点、**每段直线**（`\|bulge\| ≤ tol`）、`+Z` 平面的 LWPOLYLINE。每个**内顶点**换成两切点 + 一段**会话半径**的圆弧（`filletRadius`）：`phi = acos((-u_in)·u_out)`、`d = R/tan(phi/2)`，圆心在内角平分线上；sweep 由圆心角导出、经 `cad_geometry::arc_sweep_to_bulge` 转 bulge，再用 `bulge_arc_geometry` **回验**（圆心/半径/sweep 不一致 ⇒ `fillet.polyline_arc_mismatch`，整条命令拒绝）。实体**原地重写**（id/图层/空间/draw order/样式保留），一条事务/一步撤销；端顶点不动、不造端帽、不产生新实体 | 全部**具名且在任何写入之前**：带 payload、选择 ≠ 1、不可寻址/缺失、不是折线、**闭合环**、<3 顶点、**含 bulge 段**、**非 `+Z` 平面**、重复/零长段、共线顶点、退化接缝、`R ≤ 0`（两线 `R=0` 的移角形式无折线等价物）、**角放不下**（`fillet.polyline_too_large`，**不**做 AutoCAD 的自动缩半径）、缺会话参数、弧不自洽；Viewer 由命令层拒绝。**不建模 `TRIMMODE`**。**证据**：`cad-app` 契约测试（含与两线 `plan_fillet` 在 `1e-12` 内的对照、DXF 导出往返），**无 GUI/像素/真窗口证据** |
| `CHAMFER POLYLINE` | **实现**（同上选择式，2026-10-28） | 形状同上（开放、≥3 顶点、直线段、`+Z`）；每个内顶点按 `chamferDistanceA` 量**入边**、`chamferDistanceB` 量**出边**切成两点，连接段 `bulge = 0`；同样是原地重写 + 一条事务/一步撤销 | 拒绝面同 `FILLET POLYLINE`（`A=B=0` 或负数、角放不下都具名），另加：**A/B 方向是我们的规则**（入边/出边），**不是** AutoCAD 提示顺序的复刻；**无 `Angle` 方法**、不建模 `TRIMMODE`、不做任何自动缩距。**证据同上：契约测试，无 GUI/像素/真窗口证据** |
| `ARRAYRECT` / `ARRAYPOLAR` | **实现** | **独立副本**（AutoCAD 默认关联，记为**有意偏离**）；矩形：间距=**基点间距离**、向上右生成、副本数 = `行×列−1`；极坐标：捕获中心点，副本数 = `项数−1`，**角度步长 = `i·填充角/项数`**（**我们的规则**：部分填充时 AutoCAD 是首尾跨满 `fill/(items−1)`，故 `180°/3 项` 我们为 0/60/120 而 AutoCAD 为 0/90/180——**刻意偏离**，已测试），`rotateItems` 决定绕中心旋转或仅平移（**未查证选择**） | `Path`/`Levels`/`Total`/角度间隔输入/关联编辑（`ARRAYEDIT`/grips）/`MAXARRAY` 可配置化/3D/**负间距反向**；1×1 或单项阵列**不产生副本并显式诊断**（不开事务） |
| `ALIGN` | **实现** | 1 对点 = 纯移动；2 对点 = 平移+旋转，`AlignScale` 时按“目标对长/源对长”做**均匀**缩放；3 对点 = 平面**刚性**（旋转+平移，**不缩放**）；一条事务/一步撤销 | 3 对点的 3D 平面旋转属全局范围；非均匀缩放 AutoCAD 也不提供；非共面/非全等/镜像按名拒绝（`align.non_coplanar`/`align.non_congruent`/`align.mirrored`） |
| `STRETCH` | **实现** | 交叉窗口（**恒 CROSSING**）：**部分包围=逐顶点移动**（只动窗内顶点）、**完全包围=整移**；分类按几何类型分两种口径（直线/无 bulge 折线=逐顶点包含；圆/椭圆/块/ARC/含 bulge 折线=保守包围盒完全包含）；**bulge 数值保留**（未查证）。**逐顶点来源（按类，2026-10-26）**：`LINE` = 两端点；`POLYLINE`/`LWPOLYLINE` = 每个存储顶点（bulge 原样保留，未查证）；`SPLINE` = 每个存储**控制点**（`degree`/`knots`/`weights` **逐字复制**）；`ARC` = 存储**起点/终点**两点（规则见下）。**NURBS 声明**：控制点**不是曲线点**（只有 clamped 样条的首/末控制点落在曲线上），单个控制点的支撑域是 `degree+1` 个节点区间，故**部分**拉伸会**整体**重塑曲线——这是 NURBS 控制点编辑的固有性质，**不是缺陷**，也不做任何重拟合/求值；复制向量后定义仍满足 `cad-db` 不变量（契约测试用 `cad_db::validate_geometry` 断言）。**ARC 部分拉伸（本项目的选择，非 AutoCAD 断言）**：窗内端点各按其「平移后位置相对圆心的**平面内方向**」改角，`center`/`radius`/`normal` **逐位不变**（弧不离开自身平面，位移的平面外分量被丢弃），窗外端点保持原角，`sweep` 按**存储方向**重算（`sweep<0` 仍顺时针）；以下三种结果**只报诊断、该实体不写入、绝不夹取**：重算张角 ~0 或 ~2π（阈值 `1e-9`，与 tessellator / 弧端点夹点同一 slack）→ `drawing.stretch.arc_degenerate_sweep`；被拖端点无平面内方向（落在圆心或法线上）→ `drawing.stretch.arc_no_direction`；规则使两个存储角都不变（位移在弧平面内**无切向分量**，如纯径向/纯平面外）→ `drawing.stretch.arc_unchanged`。其余「不动」路径的具名码：窗内**没有**顶点/控制点/端点 → `drawing.stretch.no_vertex`；定义不可索引的样条（knot/weight 数量不符，导入路径可产生）→ `drawing.stretch.spline_inconsistent`（部分包围；**完全包围整移**则由 `cad-db` 的写路径校验按同一稳定码拒绝，整命令失败、不写入）。**完全包围的 `SPLINE` 整移**：与其它种类**同一条路径**——`cad-db` 的 `transform_geometry`（NURBS 仿射不变，见下「仿射不变规则」）；上一轮为绕过 `cad-db` 拒绝而加的应用内「纯平移控制点」例外**已删除**，故完全包围的 STRETCH 与 MOVE 对样条**不可能**给出不同结果。**INSERT 部分拉伸（本项目的选择，非 AutoCAD 断言）**：块参照在这条路径上只有**一个**可寻址「顶点」——**插入点**（块坐标系原点 `(0,0,0)` 过实例放置的 `Transform3::apply_point`，即放置矩阵的平移列）。**部分包围**时：插入点**落在交叉窗口内** ⇒ 整条参照**刚性平移** delta（走 `cad-db` 的 `transform_geometry` 平移路径，**与完全包围整移同一个调用**），放置矩阵的**线性部分（旋转/缩放/镜像列）逐位不变**、`block` 不变、只有平移列改变；插入点**不在窗口内**（窗口只压到块成员的显示几何/保守包围盒）⇒ **逐位不动** + `drawing.stretch.insert_outside`（**不写入、不 staging**）。**完全包围** ⇒ 整移（未变）；**窗口外** ⇒ 完全不动（无写入）。**诚实的边界（非目标，不得读成「已支持块」）**：块**成员不会被逐个拉伸**（实例路径在这条路径上不可寻址，规划只写顶层模型空间所有者，没有逐成员顶点编辑）；**旋转/缩放/镜像**的参照只**刚性平移**、**不重拟合**（不按窗口重塑，旋转与比例逐位保留）；这**不是** AutoCAD 的完整块参照 STRETCH 语义——定义级编辑、嵌套成员拉伸、属性重排均**未建模、不声明** | cpolygon/lasso、**单独选取即整移**、`TEXT`（**相似变换语义受限** ⇒ 整命令拒绝，ADR-0019 B.2——**不是**因为变换面拒绝：P7a 起 `Text` 可平移/旋转/均匀缩放）、`MultiLeader`（**显式按 `type_key` 整命令拒绝**）、`HATCH` 的**拉伸**（部分包围不动+诊断、完全包围整移）、`INSERT` 的**成员级**拉伸（只做插入点刚移，见前）、3D；宽度未建模 |
| `GRIPSTRETCH`（**本项目选择器**，AutoCAD 无同名命令） | **宿主拖拽已实现（仅 `native-below`）；`dom-canvas` 载体无夹点路径；UI 层 Escape 已接线（无运行时证据）** | 纯模型（`crates/cad-app/src/grips/`：`mod.rs` 句柄表/命中/文档 + `drag.rs` 各类拖拽规则）+ 命令（`crates/cad-app/src/app_drawing/grip.rs`）：**一个夹点、一个实体** → 一次 `update_entity`、一条事务 / 一步撤销（undo 恢复被拖端点/位置的**位级**原值，未拖端点/其它坐标逐位不变）。支持面：`LINE` 三夹点（`Endpoint` `index` 0 = start、1 = end，`Midpoint` `index` 2 —— 拖中点让**整条线平移**，两端走同一 delta，方向与长度不变；中点位置是两存储端点的 f64 精确中点）、`CIRCLE` 五夹点（`Center` `index` 0；`Quadrant` `index` 0..=3 = 圆自身平面内 `+ax` / `+ay` / `-ax` / `-ay`，拖象限**只改半径**：半径 = 中心到目标的**平面内**距离（不是 3D 距离、也不是 x/y 距离），圆心与法向逐位不变，故圆不会被拖成倾斜椭圆）、`ARC` 三夹点（`Center` `index` 0 = 刚性平移：半径/法向/起始角/张角逐位不变；`Endpoint` `index` 0 = 弧**起点**、`index` 1 = 弧**终点** —— 拖端点让弧留在**同一圆**上（圆心/半径/法向逐位不变），被拖端点的角度 = 目标在弧自身平面内的方向，**另一端保持原角度**，张角按**原方向**重算且必须落在开区间 `(0, 2π)`；会把弧压没（张角 ~0 或 ~2π，阈值 `1e-9` rad，与 tessellator/provider 判「整圆」同一 slack）或目标在平面内没有方向（圆心本身/圆心法线上）时按名拒绝 `grip.degenerate_arc`，**绝不夹取**；该端点规则是**本项目的选择，不是 AutoCAD 断言**）、`POLYLINE`/`LWPOLYLINE` 顶点夹点（`Vertex` `index` = **存储**顶点序号，一顶点一夹点；拖拽只把该顶点 x/y 设为目标、**保留其自身 z**，其余顶点与 `closed` 逐位不变，`bulges` 向量整体原样复制 —— **不重算任何 bulge**（进/出该顶点的段保留原 bulge；这是我们的解读，**未经 vendor 验证**）；越界 `index` 为 `grip.stale_handle`；`GripId::index` 是 `u8`，故只为顶点 `0..=255` 提供夹点，更多顶点在本版本不可寻址）、`ELLIPSE` 夹点（`Center` `index` 0；`Quadrant` `index` 0..=3 = 椭圆**自身存储轴**帧内的 `+major` / `+minor` / `-major` / `-minor`，与 `CIRCLE` 象限同 `+, +, -, -` 次序）：中心拖拽**刚体平移**（`normal`/`major_axis`/`ratio`/`start`/`sweep` 逐位不变）；轴象限拖拽把**该轴的半径**设为“中心到目标在椭圆自身平面内的距离”（不是 3D 距离、也不是 x/y 距离），中心与法向逐位不变；**轴方向永不旋转**（本项目选择，**不是 AutoCAD 断言**）：major 拖动把存储 `major_axis` 按 `distance / |major_axis|` **缩放**并把 `ratio` 重算为 `minor_len / distance`（保留存储的次半径 `|major_axis| * |ratio|`），minor 拖动只令 `ratio = distance / |major_axis|` 且 `major_axis` 逐位不变；因此目标不在轴上时也只改半径、不转动椭圆，且两轴按构造保持正交。**椭圆弧（部分椭圆）**：存储参数范围不是整圈（`|sweep| < 2π - 1e-9`，与 tessellator/provider 判「整圈」同一 slack）时**只暴露 `Center` 夹点**，象限 id 按名拒绝 `grip.partial_ellipse`（本项目选择，**不是 AutoCAD 断言**：轴点未必落在已绘参数范围内，用它改形等于悄悄重释参数范围；中心拖拽不改半径/参数，故仍可用）。域模型**没有** `minor <= major` 约束（`cad-db` 只要求 `ratio` 正且有限，渲染用它画 `|major| * |ratio|`），故拖到两半径交叉**不拒绝也不静默归一化**（交换轴会旋转存储帧并改变参数化）；只有创建路径（`ellipse_parameters`）恰好保持 `ratio <= 1`，那是轴端点法的存储假设，**不是**夹点不变量。零/非有限平面内半径、或存储帧无法导出平面内半径（`major_axis` 为零/非有限，或法向与其平行）→ `grip.degenerate_radius` 按名拒绝、**不夹取**；把该轴半径拖回原长度（含离开平面但平面内距离不变）→ `grip.zero_displacement`）、`POINT` 位置夹点（`index` 0），以及**块引用插入点夹点（2026-10-26）**——见下方 `INSERT` 条。**`TEXT`/`MTEXT` 插入点夹点（2026-10-26）**：`Insertion` `index` 0 = **存储的 `position` 原值**（三坐标逐位，**不**按 `h_align`/`v_align` 偏移、**不**绕该点按 `rotation` 旋转；对齐或旋转**不改**句柄位置）；拖拽只把 `position` 设为目标，`text`/`style`/`height`/`rotation`/`font`/`h_align`/`v_align`/`wrap_width` **逐位不变**（含 `wrap_width: Some(w)`，属普通 `f64`，只复制不重算）。`TEXT` 与 `MTEXT` 是**同一个** `SemanticGeometry::Text` 变体（只有 `DbEntity.object.type_key` 区分 `AcDbText`/`AcDbMText`），故两者夹点完全相同。**新变体 `Insertion` 而非复用 `Point`**：`Point` 是 `POINT` 实体自身的句柄，共用会让 `grip.stale_handle` 无法区分"文本插入点"与"点实体"。**旋转 / 高度 / 对齐（attachment）/ 宽度夹点一律 NOT IMPLEMENTED**——本轮**只**实现插入点。`SemanticGeometry::Polyline` 是**所有** 折线类（LWPOLYLINE / `AcDb2dPolyline` / 旧 `AcDbPolyline` / 3D `AcDb3dPolyline`）的唯一语义变体，类名不进域模型，因此**无法单独排除 3D 折线**（顶点夹点对所有折线生效；拖拽只改 x/y 并保留自身 z，与 `LINE` 端点规则一致，不做 3D 标高编辑）；**按类区分 3D 折线 NOT IMPLEMENTED**。**`SPLINE` 控制点夹点（2026-10-26）**：`ControlPoint` `index` = **存储**控制点序号，位置 = 存储 `Point3` 原值（逐位，**不做**任何 NURBS 求值——控制点一般**不在**曲线上，只有 clamped 样条的首/末控制点才在；这是定义而非 bug，标记落在曲线外是正确行为）；拖拽把**该**控制点设为目标三坐标原值，`degree`/`knots`/`weights` 与其他控制点**逐位不变**（不重算节点/权重、不做重拟合 ⇒ 结果是参数化相同的**另一个 NURBS**，曲线因此不再经过它原先经过的点；这是本项目选择，AutoCAD 同样直接拖控制点但**不声称** vendor 等价）；`GripId::index` 是 `u8`，故只暴露控制点 `0..=255`，更多控制点在本版本**不可寻址**（**不**回绕、不截断、不重编号）；越界 `index` → `grip.stale_handle`、目标等于存储控制点 → `grip.zero_displacement`、非有限目标 → `grip.non_finite_target`、该几何上的其它 kind → `grip.stale_handle`；存储定义**自相矛盾**（`control_points.len() != weights.len()`，或 `knots.len() != control_points.len() + degree + 1`）时**一个夹点都不暴露**（不猜、不补齐向量；形状有效性以 `cad-db` 校验器为准），其任何控制点 id 均为 `grip.stale_handle`。**`INSERT`（块引用）插入点夹点（2026-10-26）**：`Insertion` `index` 0 = **放置变换的插入点**，即 block-space 原点在 `Transform3` 下的像 —— `Transform3` 是 4x4 仿射且 `apply_point` **不做**齐次除法，故该像**就是**矩阵的平移列；导入器已把块基点折进放置（`insert_array_transform` 的 `translate * rotate * scale * block_placement(-base)`，使 block-space 原点落在 `INSERT` 的存入插入点上），所以这是仓库**既有**的“块引用插入点”定义，本轮**不**引入第二套约定。拖拽是**刚性移动**：只把平移列写成目标（`m[0][3]=to.x`、`m[1][3]=to.y`、`m[2][3]=to.z`），**整个 3x3 线性部分（旋转 / 非均匀缩放 / 镜像 / 剪切）与 `block` 逐位不变**（第 3 行也不变），因此拖拽**永不**旋转或缩放块引用，且插入点**恰好**落在目标上（不做求逆、**无**残差、**不**夹取/捕捉）。有限但**奇异**的放置**不**拒绝（平移仍逐位精确，线性部分原样保留；`cad-db` 本来拒绝**存储**奇异放置，故这属损坏输入）；存储放置含**非有限**系数时按名拒绝 `grip.degenerate_transform`。**块成员（instance path）/ 块内属性（ATTRIB）/ 块内点**的夹点一律 **NOT IMPLEMENTED**：`GripId` 不含 instance path 与子元素字段，`GRIPSTRETCH` 已用 `property.not_addressable` 拒绝非模型空间目标，本模型**不**另立可达性规则。`Insertion` 变体由 TEXT/MTEXT 与 INSERT 共用（靠几何变体区分，**不**新增 `GripKind`）。**`DIMENSION` 定义点夹点（2026-10-26）**：新变体 `DefinitionPoint`，`index` = **该子类型文档化的句柄次序里的位置**，一个句柄对应一个存储的 DXF 定义点，**最后一个**句柄是存储的 文字中点（group 11）：`Linear { rotation }` 与 `Aligned`（共用 `DimensionPoints::Linear`）= `0` `first`(13)、`1` `second`(14)、`2` `dimension_line`(10)、`3` `text_middle`(11)；`Radius`/`Diameter`（`Radial`）= `0` `center`(10)、`1` `point_on_arc`(15)、`2` `text_middle`(11)；`Angular2Ln` = `0` `line1_start`(13)、`1` `line1_end`(14)、`2` `line2_start`(15)、`3` `line2_end`(10)、`4` `arc_point`(16)、`5` `text_middle`(11)；`Angular3Pt` = `0` `vertex`(15)、`1` `first`(13)、`2` `second`(14)、`3` `arc_point`(10)、`4` `text_middle`(11)；`Ordinate` = `0` `feature`(13)、`1` `leader_end`(14)、`2` `base`(10)、`3` `text_middle`(11)。句柄位置 = **存储点逐位原值**（三坐标，不投影、不派生、不偏移）；`leader_length`(40)、`text_rotation`(53)、`Linear { rotation }`/`Ordinate { x_ordinate }` 等子类型标量、`normal` 与整套解析后的 `DimensionStyleValues` **都不是夹点**，任何拖拽都逐位保留。**拖拽规则 = 重合成**：克隆存储定义、只替换那**一个**点，再用 `DimensionGeometry::synthesized` 以**实体自身**的 `style_name`/`style`/`text_override` 重建 —— 故 `kind`、`text_rotation`、`normal`、`style_name`、`style`、文字覆盖串（DXF group 1，**不丢弃**）与未被拖动的定义点/`leader_length` 全部保留，而 `measurement`（group 42）与**整幅显示**由 `cad-domain` 的唯一构建器按改后的定义**重算**（`cad-app` 不手改 dimension 子几何）；拖 `text_middle` 只移动存储的 group-11 锚点并重排显示，`measurement` 不变。**非法结果一律按名拒绝、绝不夹取**：`grip.dimension_inconsistent`（子类型与点槽不符 / 点非有限；两个公开构造器都会拒绝这种定义，故命中它属**缺陷信号**）、`grip.dimension_degenerate`（改后的定义无法合成显示 —— `Radial` 的 `point_on_arc` 拖到圆心、`Angular3Pt` 的 `arc_point` 拖到顶点、两条角尺线被拖成平行等 —— 或合成出的 `measurement` 非有限 / 显示为空：**绝不**存 `NaN` group 42、**绝不**存空显示）。**声明限制**：`cad-domain` 的 `DisplaySource` 是私有的，`cad-app` 无法区分「我们的合成」与「导入的匿名块图片」分支，也没有能从改后定义重建图片分支的构造器，故拖拽**总是**重合成（图片分支的 dimension 拖后变成合成分支，`measurement` 跟随改后的定义）。`DefinitionPoint` 是**新变体**而**不**复用 `Endpoint`/`Insertion`：dimension 的句柄就是它的 DXF 定义点加文字中点，共用会让 `grip.stale_handle` 无法区分。**其余整类 NOT IMPLEMENTED**：块**成员**/块内**属性**（ATTRIB）/块内点（instance path 在 `GripId` 上无法表达，`GRIPSTRETCH` 另有 `property.not_addressable` 兜底）、`MULTILEADER`（其注记文本在本版本**不**通过文本插入点句柄暴露，故**无夹点**）、`HATCH`、`IMAGE`/`MASK`。多夹点/多实体拉伸、拖拽中捕捉、3D 夹点编辑仍不存在。载荷 `CommandPayload::GripStretch(Box<GripStretchCommand>{ id: GripId, to: Point3 })`。命令层 `to` 三坐标原样存储（**不做**工作平面投影）；**宿主**在平面视图里用 `grip_hit_planar`（x/y 距离，**不是** 3D 距离——指针经 `screen_to_world` 落在工作平面上）命中夹点，并**保留被拖夹点自身的 z**，故平面拖拽**不改标高**（**3D 夹点编辑 NOT IMPLEMENTED**）。拒绝（**全部在开事务之前**，不改 revision、不写 ChangeSet、不留空撤销步）：`grip.target_missing`（实体已不存在）、`property.not_addressable`（非模型空间目标：块定义成员 / 图纸空间 —— 复用属性写的可达性理由；`GripId` 不含 instance path / 子元素字段，故那两种形态在载荷上无法表达）、`grip.stale_handle`（kind/index 与**当前**几何不符）、`grip.zero_displacement`（零位移是**错误**而非空 `Ok`，正为避免空撤销步；**宿主**把它按**显式 no-op** 处理：不发命令、不给用户报错）、`grip.unsupported_kind`（`MULTILEADER`/`HATCH`/`IMAGE`/`MASK`/`Opaque`/`Compound`/`Mesh`/`Shape` 等整类无夹点模型；`LINE`/`CIRCLE`/`ARC`/`ELLIPSE`/`POLYLINE`/`SPLINE`/`POINT`/`TEXT`/`MTEXT`/`INSERT` 已不在此列）、`grip.degenerate_radius`（圆/椭圆象限拖拽的平面内半径为 0 或非有限值，或椭圆存储帧无法导出平面内半径 —— 按名拒绝，**不夹取最小值**）、`grip.degenerate_arc`（弧端点拖拽压没张角、张角非有限，或目标在平面内无方向 —— 同样按名拒绝，**不夹取**）、`grip.partial_ellipse`（部分椭圆/椭圆弧上的象限 id：该几何只暴露 `Center` 夹点，按名拒绝而非伪称 stale）、`grip.degenerate_transform`（块引用插入点拖拽时存储的 `Transform3` 含**非有限**系数：刚性规则保留该系数，`cad-db` 提交时本会拒绝，故按名拒绝而不是把失败推后；**有限但奇异**的线性部分**不**拒绝——规则不做求逆/分解）、`grip.dimension_inconsistent`（dimension 存储定义子类型与点槽不符或点非有限：该维度**一个夹点都不暴露**，任何 `DefinitionPoint` id 均为该码；公开构造器本会拒绝这种定义，故命中属**缺陷信号**）、`grip.dimension_degenerate`（dimension 定义点拖拽后无法合成显示，或合成的 `measurement` 非有限/显示为空：按名拒绝、**不夹取**，绝不写 `NaN` group 42 或空显示）、`grip.non_finite_target`（**在读取图形之前**拒绝）。裸 `GRIPSTRETCH` 为 payload-required，按名拒绝（不路由 `BeginDraw`、不伪造拖拽）；**绝不**经过 `StretchEntities` | **宿主侧状态（`crates/cad-host/src/input/grip_drag.rs`，2026-10-26）**：`native-below` 桌面宿主（Linux 原生 Wayland / Windows）在选中实体上绘制夹点标记，并把**单个**已建模夹点（`LINE` 端点/中点、`CIRCLE` 圆心/象限、`ARC` 圆心/两端点、`ELLIPSE` 圆心/四轴点（整圈；椭圆弧仅圆心）、`POLYLINE` 顶点、`SPLINE` 控制点、`POINT` 位置、`TEXT`/`MTEXT` 插入点、`INSERT` 块引用插入点、`DIMENSION` 定义点（按子类型次序）与文字中点）的拖拽接到 `GripStretch`（宿主对夹点种类**无分支**：枚举与命中都来自模型）：按压命中夹点即 arm（2D、非捕获、工具 `Idle`/`Selecting`、主键、选中且**可见**的可直接寻址实体），移动只更新 overlay 预览（`OverlayInputs::grip_markers` 中被拖项替换为当前目标 ⇒ **移动的标记就是预览**，不发导航 delta）；拖拽目标走**与取点相同的**会话辅助 `SessionState::constrained_point`：**先栅格捕捉**（`SNAP`/F9，按 `SNAPUNIT` 间距、以世界原点为基准、半程远离零、`z` 不变）、**再 ORTHO**（`ORTHO`/F8，轴过**被拖夹点的原位**，`|dx| >= |dy|` 取水平、平局取水平，另一坐标逐位保持原位值），**预览与提交共用同一个已约束坐标**（绝不预览一个点、提交另一个）；两开关都关时与旧行为逐位相同；间距非有限或 `<= 0` 会被会话设置拒绝、量化器原样放行（不伪造间距，开关因此无效果）；**OSNAP 已参与夹点拖拽（2026-10-26）**：目标经**取点路径的同一个** `HostController::resolve_snap`
解析（同一候选生成器、同一 `snap_enabled` 门、同一最佳候选规则与同一光圈容差），命中则**替换**原始
指针点，再走栅格捕捉与 ORTHO ⇒ 顺序为 **OSNAP → 栅格捕捉 → ORTHO**（与取点路径一致；两者同开时栅格会把
已捕捉点再量化，故落点在栅格而非候选点，除非二者重合 —— 这是取点路径的既有行为，显式声明）；
候选只贡献**平面位置**，`z` 仍取被拖夹点自身（3D 夹点编辑未实现）；**不排除自捕捉**（取点路径也不排除），
指针停在夹点自身光圈内 ⇒ 目标是夹点原位 ⇒ 模型的零位移 no-op；拖拽**不**发布 OSNAP 提示标记
（移动的夹点 marker 仍是唯一预览）。松开提交**一次**命令；落在夹点上的点击被一次性闩锁吞掉（**不**重复选中）。**`dom-canvas` 载体（X11/XWayland、web）没有任何夹点标记或拖拽**（wasm 侧无此路径，按铁律 8 显式不存在，**不是**静默回退）。**Escape 取消已接线（仅合成契约，无运行时证据）**：`cad_status` 新增布尔字段 `gripDrag`（`CadStatus::grip_drag`，取自 `HostCore::grip_drag_active()`；拖拽期间 `tool` 仍为 `null`），桌面 UI 的 Escape 门（`toolCommandForKey`）因此在**拖拽中**也发 `CANCEL`（`Enter`/`CONFIRM` 仍只认活动工具）；宿主 `InteractionRequest::Cancel` → `reset_grip_gesture()` 纯状态丢弃拖拽。该链路只有合成证据（`cad-ui-core` 状态投影契约 + `apps/desktop-ui` 的 `tsc`/纯函数/源码结构契约），**没有**真窗口、真实按键或像素证据。插入点夹点**已实现但仅限 `TEXT`/`MTEXT` 的存储插入点**（`GripKind::Insertion` `0`；旋转 / 高度 / 对齐 attachment / 宽度夹点仍**未实现**，模型层不建「永远为空」的 `GripKind` 变体）；`SPLINE` 控制点由模型提供（`GripKind::ControlPoint`），宿主对夹点种类**无分支**，故其标记/拖拽走同一通用路径（**未做**宿主运行/像素验证）；多夹点与多实体拉伸、拖拽期间的 OSNAP 捕捉、3D 夹点编辑、夹点右键菜单与 `GRIPMULTIFUNCTIONAL` 均**不存在**；证据上限：`cargo test -p cad-app -p cad-host` 契约 + 门禁，**无 GUI/像素/真窗口/真实鼠标证据**，Windows 侧更只有 Tier 0 编译 |
| `EXPLODE` | **完整实现** | **一次一层**：LWPOLYLINE→线段+ARC（bulge 还原）；INSERT→直接成员（placement 烘焙，**嵌套块引用保持为引用**，二次 EXPLODE 才展开；非均匀放置走可精确表示子集）；`Dimension`→线/弧/箭头网格/文字，`MultiLeader`→引线/样条/箭头网格/MTEXT；`Compound`/`Hatch`→**全部子项**；MINSERT（`Compound(Insert…)`）按名拒绝。**`EXPLMODE`/`EXPMODE` 是真实会话开关**（`EXPLODE MODE 0\|1` 与 `SETVAR EXPLMODE/EXPMODE`；默认 1）：0 时非均匀 INSERT 在写前以 `explode.expmode_off` 拒绝；一条事务/一步撤销 | 多级一次到底（导出层的"摊平到叶子"是**另一条路径**，不在本命令内）、颜色/线型变化、关联阵列（我们不存）、属性语义；`IMAGE`/`MASK` 按**策略**拒绝（其**可变换**）；`TEXT`/`SPLINE`/`SHAPE`/`OPAQUE`/`Table` 作为子项或目标按名拒绝；镜像 Arc/Ellipse 的块直接成员与非均匀不可精确表示成员按名拒绝；箭头是 `MESH`（无 2D-solid 变体）；块内 `MULTILEADER` 仍按 `type_key` 拒绝 |
### 2026-11-07 2D 命令批次（AutoCAD 2026）

> 本批次以**当前选择为输入**（或独立捕获工具），不是 AutoCAD 的交互 `Select objects` 提示；除特别
> 说明外，证据只有 `cad-app`/`cad-ui-core`/`cad-host` 与前端纯函数契约测试，**没有** GUI、像素、
> 真实 GPU、真机或 vendor-AutoCAD 证据。逐条官方清单状态见 `docs/autocad-2026-command-coverage.md`。

| 命令 | 状态 | 结果 | 显式未实现 / 有意偏离 |
|---|---|---|---|
| `LAYCUR` / `LAYM C U R` | 部分实现 | LAYCUR 把每个选中模型空间实体移到会话当前层（一条事务/一步撤销）；LAYMCUR 以**第一个**选中实体的层为当前层（仅会话状态，复用既有的 locked/frozen/off 拒绝） | 无交互 Select objects；块实例成员/图纸空间/子图元按名拒绝；多选时 LAYMCUR 只用第一个（`laymcur.first_pick` 诊断） |
| `LAYFRZ` / `LAYLCK` / `LAYULK` | **完整实现** | 跨模型空间/图纸空间/块成员选择（嵌套按有效层解析）；写 `LayerAttributes.frozen/locked`；`LAYFRZ VP [idx,…]` 写每视口 `layer_overrides`；单事务 ⇒ 一步撤销 | 交互对话框属 ADR 全局范围；`LAYFRZ` 的 Viewports 形式在目标视口包含 `CVPORT` 时拒绝当前层；`frozen_in_new_viewport` 不由 `LAYFRZ` 写 |
| `SETBYLAYER` | 实现 | 恢复颜色/线型/线宽/**透明度**为随层；`Include blocks` 递归块成员的显示状态；一条事务/一步撤销 | 材质/命名打印样式未建模（全局范围）；锁定图层拒绝；无交互 ByBlock 询问（全局对话框范围） |
| `REVERSE` | 部分实现 | 反转选中 LINE / LWPOLYLINE（含 bulge 与闭合映射）/ SPLINE（控制点+节点镜像）方向，一条事务/一步撤销 | 仅当前选择；无 helix/3D；其它种类按名拒绝 |
| `OOPS` | 完整实现 | 恢复**最近一次 ERASE 的整批**实体，并覆盖 `BLOCK` 移出的整批行（定义保留成员、OOPS 放回独立副本）；新 id；一条事务/一步撤销；重复执行是具名空操作 `oops.already_restored`，**撤销该恢复会让快照重新可用** | `WBLOCK` 本构建无删除路径（声明）；`CUTCLIP/CUTBASE` 不记录快照 |
| `OVERKILL` | 部分实现 | 删除同空间/同图层/同几何（1e-9）的**完全重复**选中对象，保留最低 draw order；一条事务/一步撤销 | 无部分重叠、共线合并、圆弧覆盖圆、折线段优化、容差选项、忽略图层/颜色选项 |
| `BREAKATPOINT` | 完整实现 | 交互工具：拾取对象后指定断点（可投影到对象）；LINE/ARC/CIRCLE/整椭圆与椭圆弧/开放 SPLINE/开放或闭合 LWPOLYLINE。开放曲线拆成两段；闭合环（圆、闭合多段线、整椭圆）在断点开成**单段整圈弧**并在诊断里明说（单点无法留下两段非退化片段）；一条事务/一步撤销 | 闭合 SPLINE 需旋转节点域，按名拒绝（`breakatpoint.closed`）；TEXT/IMAGE/HATCH/DIMENSION/MULTILEADER/TABLE/MESH/Body3d 等非曲线种类按名拒绝；无选项 |
| `LIST` | 实现 | 读取当前选择；报告模型/图纸/块空间、id/handle/type/layer/space/draw order、颜色/线型/线宽/透明度/线型比例/可见性、文字样式/标注样式/块名与几何摘要；只读 | 不是交互 Select objects（输入是当前选择）；厚度/拉伸方向未建模（全局范围）；不可寻址引用逐条报告后继续 |
| `STATUS` | 完整实现 | 报告数据库/修订/实体与表计数、模型空间范围、当前层、会话模式与绘图开关，**以及存储的基点/图形界限真实值**（`status.limits_base`），缺值时按名声明（`status.not_stored`） | 创建/编辑时间与内存仍不跟踪（`status.not_tracked`） |
| `FS` | 完整实现 | 取当前唯一选择为种子并入接触对象；`FSMODE=ON` 走传递相切链（解析切线，样条精确 NURBS 导数），`OFF` 与旧直接接触逐字节一致；`fs.chain_nontangent`/`.chain_ambiguous`/`.chain_bound`（上限 64，不截断）在选择变更前拒绝；`FSMODE ON|OFF|1|0` 是真实命令、`SETVAR FSMODE` 读写同一令牌；`fs.mode` 报告模式/实体数/接触数 | 无功能区按钮（Express 工具，仅命令行）；无对话框 |
| `TORIENT` | 实现 | 裸命令 Most Readable（归一到 `(-π/2, π/2]`）、`TORIENT <角度>` 绝对旋转、`TORIENT REVERSE/R` 再加 180°（**本项目扩展**）；只改 TEXT/MTEXT rotation，位置不变 | ATTDEF/带属性块因无属性模型不适用（全局范围）；MULTILEADER/标注文字不在 AutoCAD TORIENT 范围内，报告跳过 |
| `TCASE` | 部分实现 | 裸命令按 Title，`TCASE U/L/T/S/G` 分别大写/小写/标题/句首/切换；保留 MTEXT 转义序列 | 属性定义/标注文字未建模；MULTILEADER 只在可实现时改写 |
| `TXT2MTXT` | 完整实现 | 选中 Text/MText 排序合并为一个 `AcDbMText`（含 `UNIFORM` 行距），同时删除来源（一条事务/一步撤销） | Settings 对话框与分栏未实现（全局范围）；保留原字符串转义 |
| `HATCHGENERATEBOUNDARY` | 完整实现 | 每个选中 HATCH（模型空间或图纸边界）的每个 loop 生成一条同层/同空间、非关联的闭合 LWPOLYLINE（保留 bulge、含内岛）；图纸布局边界走既有 layout 实体写入路径；一条事务/一步撤销 | 无关联性（AutoCAD 亦为非关联）；无交互选项提示 |
| `GRADIENT` | 完整实现 | 当前选择为闭合边界（整实体 Circle/完整 Ellipse/闭合 Polyline/LINE+ARC 边链），复用 HATCH 创建路径并强制渐变填充（会话 kind/角度/双色 ACI，默认线性 0°、ACI 1→5；测试钉住 SPHERICAL/45°/ACI 3→7 落到存储的 `AcDbHatch`）；一个 `AcDbHatch`、一条事务/一步撤销；边界实体保留 | 全局范围：无渐变预览、无 `-GRADIENT` 选项提示、无 .pat/关联性；非 `+Z` 平面/非闭合按名拒绝 |
| `HATCHEDIT` | **完整实现** | 多目标；`Style`/`PatternType`/`PATTERN`/`PATTERNFILE`/`ORIGIN`/`Angle`/`Scale`/`Solid`/`Gradient`/`Color`/`Transparency`/`Layer`/`DrawOrder`/`Recreate Boundary`/`Separate`/`Add|Remove Boundaries`；一条事务/一步撤销 | 全局范围：`Associate`/`Disassociate`/`Annotative`、`DRAWORDER BOUNDARY`；无交互对话框；证据只有契约测试 |
| `HATCHTOBACK` | 完整实现 | 把所有 HATCH 的 draw order 排到其所在 `SpaceId` 范围内的所有非 HATCH 对象之后（模型空间、各图纸布局、各块定义逐范围规划后一次写入），保留相对顺序；一条事务/一步撤销 | 无 AutoCAD 独立的 Draw Order 表（沿用实体存储的 `draw_order`）；块定义重排对全部块参照生效（模型固有）；无 Draw Order 对话框 |
| `WIPEOUT` | 部分实现 | 开放多边形捕获（≥3 点、`+Z` 平面）生成一个 `AcDbWipeout`（`Mask{inverted:false}`）；`Frames ON/OFF/DISPLAY` 真实（显示/出图分离） | 无 `FRAME` 覆盖、无 Polyline/Erase Polyline 选项、无自交拒绝、无既有 wipeout 拾取 |
| `POLYGON CIRCUMSCRIBED` | 部分实现 | 独立选择子：中心 + 切圆上一点，用 `R=r/cos(π/n)`、顶点在 `θ±π/n` 生成闭合外切正多边形 | `POLYGON` 的 `Edge` 分支、数值半径的捕捉旋转规则仍未实现 |
| `DIMROTATED` | 部分实现 | 三次拾取（两原点 + 尺寸线位置）+ 会话 `dimRotation` 角度，复用 `DimensionKind::Linear{rotation}` 合成一个非关联尺寸 | 无 AutoCAD 的选项子提示；非 `+Z` 平面/退化输入按名拒绝 |
| `DIMSPACE` | 实现 | 首个线性/对齐/角度标注为基准；显式间距（`0` = 对齐）或 Auto = **2 × 基准样式文字高度**；角度标注按同心弧半径步进；一条事务/一步撤销 | 模型空间；关联重排未实现（全局范围）；径向/坐标/混合类型按名拒绝 |
| `PEDIT` | 部分实现 | 恰好一条可寻址模型空间 LWPOLYLINE；`PEDIT CLOSE/OPEN/REVERSE` 改闭合标志或反转顶点+bulge，一条事务/一步撤销；无变化时显式 no-op | 无 `Join`/`Width`/`Fit`/`Spline`/`Decurve`/`Ltype gen`/`Undo`/`Multiple`/`Edit vertex`，无交互选项提示、无 line/arc 转换；裸 `PEDIT` 或未知参数按名拒绝 |
| `BOUNDARY` / `-BOUNDARY` | 完整实现 | 一个内部点 → 每个发现的闭合环一条闭合非关联多段线；边界仅整实体圆/完整椭圆/闭合多段线、同 `+Z` 平面；**`Boundary set`**（载荷携带）用命名实体子集替换全图发现，空集/非候选按名拒绝**绝不放宽**；**`Island detection`** 开启时外层+每个内层各写一条折线并以 `boundary.islands` 明说孔洞关系只存在于该诊断，关闭时只写外层并逐条具名跳过；一条事务/一步撤销 | 无 region / 对象类型选择（本构建无 region 模型，载荷也无该字段）；`BO`/`BPOLY` 无别名；选项为载荷携带（无交互提示环） |
| `TEXTTOFRONT` | 完整实现 | 选择式；按 `SpaceId` 逐范围（模型空间、各图纸布局、各块定义）把 Text/MultiLeader/Dimension 相对连续排到该范围非合格对象之上；其它选中对象逐条 `texttofront.skipped`；一条事务/一步撤销，`texttofront.unchanged` 不建事务，耗尽 `texttofront.order_exhausted` | 载荷无「全图文字」选项（声明）；无 Draw Order 对话框 |

#### 2026-11-07 文字编辑批次（10 条）

| 命令 | 状态 | 结果 | 显式未实现 / 有意偏离 |
|---|---|---|---|
| `JUSTIFYTEXT` / `TJUST` | 部分实现 | `JUSTIFYTEXT <H> <V>`（H∈LEFT/CENTER/RIGHT，V∈BASELINE/BOTTOM/MIDDLE/TOP）；只改 `h_align`/`v_align`，`position` 不动；一条事务/一步撤销；`TJUST` 是同一 `CommandId` 的别名 | 非文字/不可寻址目标逐条 `justifytext.not_text` 跳过（TORIENT 策略）；空选择拒绝；无对话框 |
| `TSCALE` | 部分实现 | `TSCALE <factor>`；复用 SCALETEXT 的唯一 `scaled_text` 规则（`height` + MText `wrap_width`）；factor=1 为 `tscale.unchanged` 且不开事务 | factor 必须有限且 >0；非文字目标整命令拒绝；无 AutoCAD 的“按参照缩放”交互 |
| `TEXTALIGN` | 完整实现 | 15 个对齐拼写按并集包围盒两轴对齐；`DISTRIBUTE [X\|Y]`（默认 Y）按阅读序等间隙（首尾固定、≥3）；`SETSPACING <d> [X\|Y]` 固定间距（首固定）；`CURRENTVERTICAL`/`CURRENTHORIZONTAL` 按各文本自身 justification 对齐到首个目标；只移动参照点，一条事务/一步撤销 | 无交互 base-object + second-point 步骤（并集包围盒为确定性替代）；宽高为 `height × 字符数` 估算，忽略旋转 |
| `TCOUNT` | 部分实现 | `TCOUNT <start> <step> [prefix] [suffix]`；按上→下/左→右顺序把每个选中 Text/MText 替换为 `prefix+编号+suffix`；一条事务/一步撤销 | `step` 必须 >0；AutoCAD 的逐项前缀/后缀对话框、Find & Replace 模式未实现；替换整串、不解析格式 run |
| `TCIRCLE` | 完整实现 | `TCIRCLE CIRCLE\|SLOT\|RECTANGLE [OFFSET <factor>] [VARIABLE\|CONSTANT [BOTH\|WIDTH\|HEIGHT]]`；估算文字外框 + 默认 0.35 偏移因子；在文字图层建 `Circle` / 闭合 `Polyline`（slot 半圆端）或矩形；保留文字；一条事务/一步撤销 | 无关联性（AutoCAD 亦为非关联）；外框是估算值；无交互 Object/Multiple 提示 |
| `TEXTMASK` | 部分实现 | 为每个选中 Text/MText 用估算外框创建独立 `SemanticGeometry::Mask`（type-key `AcDbWipeout`），`draw_order = 文字 − 1`；一条事务/一步撤销 | 无关联模型（遮罩是独立实体）；无“掩膜偏移/边框”选项；外框估算 |
| `TEXTUNMASK` | 部分实现 | 只删除**选中**的、落在选中文字估算外框内的 `Mask`；没有选中文字时删除选中的 Mask；一条事务/一步撤销 | 启发式无法证明 Mask 由 TEXTMASK 创建；未选中的 Mask 永不扫描；无关联性 |
| `MTEDIT` | 部分实现 | `TEXTEDIT` 的别名（同一 `CommandId`/路径）：编辑当前单选的 Text/MText/MULTILEADER 字符串 | 无第二编辑路径；无 AutoCAD 的 MText 就地编辑器/格式工具栏 |
| `-MTEXT` | 部分实现 | `MTEXT` 创建工具的别名（同一两角 + 终端文本捕获），无第二实现 | 命令行变体的选项提示（高度/对正/行距/旋转）未实现；见 MTEXT 行 |

> **证据边界**：以上 10 条只有 Rust 契约测试（`cad-app` 1788 + `cad-ui-core` 401）与前端纯函数/
> 结构测试（1269）；**没有** GUI、像素、真实 GPU、真机或 vendor-AutoCAD 证据。前端测试在合并后
> 于主仓库运行通过；worktree 内因缺 `node_modules` 为 NOT RUN。

#### 2026-11-07 图层管理批次（10 条）

| 命令 | 状态 | 结果 | 显式未实现 / 有意偏离 |
|---|---|---|---|
| `-LAYER` | 实现（2026-11-11） | 裸命令/`?` 仍是只读图层表报告；写形式一行一个选项：`NEW\|N`、`MAKE\|M`、`SET\|S`、`RENAME\|R`、`DELETE\|D`、`ON`/`OFF`、`FREEZE\|F`/`THAW\|T`、`LOCK\|LO`/`UNLOCK\|U`、`COLOR\|C`（ACI 数字/七个标准色名/`aci:N`/`#RRGGBB`/`rgb:…`）、`LTYPE\|L`、`LWEIGHT\|LW`、`PLOT\|P`；名单逗号分隔，含空格的值/名称可加双引号；整行原子、一条事务/一步撤销（多行 patch 同一 `DrawingUndoRecord`）；写形式经 Work-only `SetLayerProperty` 载体，Viewer 拒绝，裸报告 Viewer-legal | 逐选项交互重提示属全局范围；多行 patch 仍按 `cad-db` 的 setter 规则每改一行升一次 revision；当前层选择/删除回落是会话状态，不进入层表撤销步；`?` 支持 `*`/`?` 通配符过滤，无匹配报 `layer.no_match` |
| `LAYMCH` / `-LAYMCH` | 部分实现 | 选择式：第一个选中对象的图层为目标，其余对象移入该层（一条事务/一步撤销）；目标层锁定由 `cad-db` 拒绝 | 无 AutoCAD 的交互 `Select objects`；空选择、<2 个对象、非模型空间/块实例/子图元、任何 payload 按名拒绝 |
| `LAYMRG` / `-LAYMRG` | 完整实现 | 选择式：把选中对象所在层上的所有实体（模型/图纸/块内）重写到当前层，删除源层行并剪除各图纸视口该层 `layer_overrides` 键；实体/布局/层行一条事务/一步撤销（重做再次合并） | 声明的偏离：目标固定为当前层、无交互 source/target 提示，目标可以是 `0` 层；当前层与 `0` 层不能作源；会话 `LAYERSTATE` 不重写 |
| `LAYDEL` / `-LAYDEL` | 完整实现 | 选择式（模型空间或任一图纸布局的顶层实体；块内成员/子元素拒绝）：删除选中对象所在层在模型空间、每个图纸布局与每个块定义内的**全部**实体，剪除各图纸视口的该层 `layer_overrides` 键，再删除层行；实体/布局/层行一条事务/一步撤销 | 当前层/`0` 层拒绝；锁定层的实体删除由数据库以 `layer.locked` 拒绝且不发布（不是静默成功） |
| `LAYERSTATE` | 部分实现 | `LAYERSTATE SAVE/RESTORE/LIST/DELETE <name>`；会话内最多 32 个命名快照（逐层属性 + 当前层 + 覆盖），恢复经 `set_layer_attributes`，**多行写入仍是一条事务/一步撤销** | 无 DWG 命名层状态表持久化、无 `.las` 文件、无图层状态管理器对话框；未知/悬挂层与保存时隐藏的当前层拒绝；活动层/覆盖选择是会话状态，不进入该撤销步 |
| `LAYERPMODE` | 部分实现 | `LAYERPMODE ON\|OFF`；会话标志，`OFF` 时任何图层设置变更（覆盖或层表属性）都不推 `LAYERP` 快照；裸命令报告当前值 | 无 `LAYERPMODE` 持久化；不改变已应用的变更 |
| `LTSCALE` | 完整实现 | 一个正有限小数；写数据库 `$LTSCALE`（一次 revision、无撤销/历史）；DXF/DWG 头经 `header.linetype_scale` 写出，SAVE→OPEN 往返（导出测试断言 group 40 与往返） | 无 `CELTSCALE`/`PSLTSCALE`/`MSLTSCALE` 联动（属全局变量族）；DWG 分支仍是实验性 `DwgWriter`，非 vendor 验证 |
| `LAYWALK` | 完整实现 | 逗号分隔层名或层名通配过滤（`*`/`?`，与 `-LAYER ?` 同一大小写不敏感规则）；只显示命名/匹配层 + 当前层，走与 `LAYISO` 相同的会话覆盖原子路径 | 仅会话覆盖、不写库；未知/空列表/零匹配过滤拒绝；由 `LAYUNISO`/`RESTORELAYERS` 清除，计一次 `LAYERP` 快照；对话框与逐对象 walk 属交互 chrome |

#### 2026-11-07 选择/过滤/查找/可见性批次（10 条）

| 命令 | 状态 | 结果 | 显式未实现 / 有意偏离 |
|---|---|---|---|
| `QSELECT` | 部分实现 | `QSELECT <PROPERTY> <VALUE>`（TYPE/LAYER/COLOR/LINETYPE，ASCII 不区分大小写）；替换选择；零匹配 = 空选择 + 诊断 | 无 Quick Select 对话框、无 `Append`/`Exclude` 模式、无用户自定义对象类型/特性列表；只枚举顶层模型空间（不展开块成员）；层 off/frozen 不参与过滤 |
| `FILTER` | 部分实现 | 同 QSELECT 语法，但**并入**现有选择（空选择也可并入） | 无命名过滤器列表/对话框/逻辑组合；只枚举顶层模型空间 |
| `GETSEL` | 完整实现 | `GETSEL LAYER\|TYPE\|COLOR\|LINETYPE\|SPACE <value>`；替换选择；只读（不写库/不升修订/不入历史） | 无对话框；只枚举当前空间顶层实体（声明 scope）；更丰富的 QSELECT/FILTER 特性仍按名拒绝 |
| `SSX` | 部分实现 | 以**第一个选中对象**的五特性（类型/层/颜色/线型/空间）为模板，把匹配的模型空间对象并入选择 | 只枚举顶层模型空间；空选择/不可寻址模板拒绝；无交互选择提示 |
| `SELECTCOUNT` | 完整实现 | 只读：统计与第一个选中对象五特性相同的模型空间对象数并报告；图纸布局按 `selectcount.model_space_only` 具名拒绝 | 不修改选择、不写库、无事务；不展开块成员 |
| `-FIND` | 完整实现 | `-FIND <needle>`：与 `FIND` 同一遍历与匹配（模型空间 + 全部图纸布局，按**画出来的文本**匹配 MTEXT/单行 TEXT/带文字 MULTILEADER/表格单元格），只报告、不写库；比较方式读取会话 `FINDMATCH`（默认 OFF = ASCII 不区分大小写，ON = 区分大小写） | 只读；块定义内命中按只读通道具名（不改块定义）；无跨块/xref 搜索（`FINDINCLUDE` 仍 partial） |
| `FIND` | 完整实现 | `FIND <needle> [=> <replacement>]`：遍历模型空间与全部图纸布局（分组报告：`model space` / `layout 'X'`），跨空间替换一条事务/一步撤销，拒绝时任何空间都不写；MTEXT / 带文字 MULTILEADER / `AcDbText`（含导入为 `Text` 的 ATTRIB/ATTDEF）/ 表格单元格都按**画出来的文本**匹配与替换（`\P` 换行、`%%d`/`\~` 转义、`\S` 分数、`^I` 制表）；多 run 命中保留各段格式码，插入文本被转义；比较方式读取会话 `FINDMATCH`（默认 OFF = ASCII 不区分大小写，ON = 区分大小写），报告与替换共用同一标志 | **全局范围**：无 Find 对话框/逐项确认/缩放高亮；块定义内命中只读具名（`block definitions, read-only (edit with BEDIT)`，一份定义被所有实例绘制，FIND 不动它）；部分命中结构化码 `find.mtext_partial_code`、替换被语法重画 `find.mtext_replacement_not_literal`；单行目标拒绝换行 `text.single_line_newline`；动态块未激活可见性成员不搜索（不绘制） |
| `HIDEOBJECTS` | 部分实现 | 会话隐藏集：隐藏当前选择；共享可见性谓词影响拾取与渲染（`FLAG_HIDDEN`） | 无数据库写/撤销；隐藏对象仍留在常驻场景，仅 fragment 丢弃；屏幕空间 POINT 标记 overlay 仍可能绘制（声明） |
| `ISOLATEOBJECTS` | 部分实现 | 会话隐藏集：隐藏除当前选择外的顶层模型空间对象 | 只按顶层身份隔离（只选中块成员时不会保留该成员所在的整个块）；无数据库写/撤销 |
| `UNISOLATEOBJECTS` | 部分实现 | 清空会话隐藏集，恢复所有隐藏对象 | 无数据库写/撤销；不恢复已被删除的对象 |

> **证据边界**：以上 20 条只有 Rust 契约测试与前端纯函数/结构测试；`cad-host` 的隐藏标志有宿主
> 契约覆盖，但**没有** GUI、像素、真实 GPU、真机或 vendor-AutoCAD 证据。前端测试在主仓库合并后
> 运行通过；worktree 内因缺 `node_modules` 为 NOT RUN。

#### 2026-11-07 实用工具/命名对象批次（10 条）

| 命令 | 状态 | 结果 | 显式未实现 / 有意偏离 |
|---|---|---|---|
| `PURGE` / `-PURGE` | 完整实现 | 清理未使用的**图层**（复用 `DELETELAYER` 引用拒绝，层表写不可撤销）、**块/文字样式/线型**（`delete_block`/`delete_style`/`delete_linetype` + `ensure_*` 在用判定，块成员同批删除），逐行报告删除/拒绝；块/样式/线型一条批/一条事务/一步撤销 | 图层删除刻意不记历史（`DELETELAYER` 既有行为）；仅导入才存在的未命名块行不作为候选；无对话框/嵌套清理选项 |
| `RENAME` / `-RENAME` | 完整实现 | `RENAME <kind> <old> <new>`：`LAYER`/`STYLE`/`DIMSTYLE`/`MLEADERSTYLE`/`BLOCK`/`LINETYPE` 真实写，一条表补丁/一步撤销；`LINETYPE` 改名在 `cad-db` 单次校验写入内把层属性与实体属性的引用一并级联（一次 revision、一次 `ChangeSet`），撤销/重做对称 | 创建/删除线型仍不支持（`linetype.delete_unsupported`）；`Continuous` 不可改名；无对话框 |
| `SETVAR` | 完整实现 | `SETVAR <name>` 报告、`SETVAR <name> <value>` 写，闭集 **13 个**：`PDMODE/PDSIZE/SNAPUNIT/LTSCALE/GRID/SNAP/ORTHO/TEXTSIZE/WIPEOUTFRAME/CVPORT/INSBASE/LIMMIN/LIMMAX`，全部复用各自命令（`CVPORT` 纯会话、不写库；`INSBASE`/`LIMIN*` 经 `BASE`/`LIMITS`） | 未知名 `setvar.unknown_variable`；已建模但无值 `setvar.not_stored`（不编造默认值）；`LIMMIN`/`LIMMAX` 另一角未存时 `limits.other_corner_unset` |
| `TIME` | 部分实现 | 只读报告：文档 revision/实体数 + UTC 时钟 | 创建/编辑时间、会话 uptime、本地时区格式显式 NOT TRACKED；不发明日期 |
| `DBLIST` | 完整实现 | 只读列出模型空间、各图纸布局与各块定义内的全部实体（id/handle/type/layer/space/draw order）；块成员单独成行（`dblist.block_object`，`Block(<name>)`）；XDATA/扩展字典/字典/代理对象按 `dblist.not_modelled` 命名 | 无分页/过滤对话框（ADR 全局范围） |
| `TREESTAT` | 部分实现 | 只读；显式 `treestat.not_tracked` | 本构建不保留可查询的持久空间索引，不造假计数 |
| `MREDO` | 部分实现 | 重做 N 步（默认 1），每步一条既有 `Redo` 历史移动；空栈拒绝；不足时 `mredo.partial` | 不合并历史粒度；不新增 redo 栈 |
| `MULTIPLE` | 完整实现 | 裸命令/`MULTIPLE <捕获命令>` 武装重复：每次成功的绘图/编辑捕获确认后经唯一确认漏斗重开同一捕获（`multiple.repeated`），`Esc`/`CANCEL` 清除武装、重开失败 `multiple.repeat_failed`、512 次守卫 `multiple.repeat_limit`；裸命令首次确认后固定为该命令；会话状态 only，无库写/修订/历史 | 一次性命令与测量工具没有可重开的捕获，按名 `multiple.not_repeatable` 拒绝且不武装（不伪造重复）；交互提示串行化属全局范围 |

#### 2026-11-07 绘制/修改补充批次（10 条）

| 命令 | 状态 | 结果 | 显式未实现 / 有意偏离 |
|---|---|---|---|
| `DONUT` | 完整实现 | 类型化 `DONUT <cx> <cy> <inner> <outer>`；用 SOLID 三角网格路径建填充圆盘（`inner=0`，三角扇）或环（`inner>0`，内外环三角带），一条事务/一步撤销 | 无中心点拾取提示（中心必须键入）；无带宽折线表示（用网格面） |
| `SOLID` | 完整实现 | 3-4 个 `x,y` 点 → 两三角 `Mesh`（type-key `AcDbSolid`），一条事务/一步撤销；导出写**真类记录**：`AcDbSolid`→`0/SOLID`、`AcDbTrace`→`0/TRACE`（均 `100/AcDbTrace`，边界→存储角序末两位互换，三角重复 `13`）；类溯源由导入器 `Solid::is_trace` 保留，DXF 与 DWG 双路径都有导入→导出→再导入往返测试 | 声明式边界：5+ 点、逐点颜色、不在 `+Z` 共面（3D 旋转块内无 OCS 重建）、异三角化 → 保留 polyface 并在 `export.converted.mesh` 里注明类；无厚度/挤出（导入侧标 Partial） |
| `HELIX` | 部分实现 | 平面 2D 螺旋 `Polyline`（64 段/圈，上限 4096 顶点），一条事务/一步撤销 | `height>0` 的 3D 弹簧按 `helix.3d_not_implemented` 拒绝；无扭转/半径渐变 |
| `MLINE` | 部分实现 | 2..=1024 点基线，每段每元素一条 `LINE`，拐点按**斜接**偏移（默认接头）；180° 反向 `mline.join_degenerate` | 无 cap/fill/bevel/弧接头（需 `MLSTYLE` 元素/端帽表）；无存储的 `AcDbMline` 实体（导入折叠为 `Compound`） |
| `MLSTYLE` | 部分实现 | 会话内最多 16 个命名多线样式（`LIST`/`SET`），喂 MLINE | 不写 DWG/不持久化；无对话框、无元素/接头样式 |
| `TREX` | 部分实现 | `TREX TRIM\|EXTEND`；选择集既是被处理对象又互为边界，复用既有 `plan_trim`/`plan_extend`；一条事务/一步撤销 | 无交互边界提示；TRIM 保留含中点的片段、EXTEND 只长 `u=1` 端 |
| `EXTRIM` | 完整实现 | 第一个选中为边界、其余为被裁对象；目标集 = TRIM 可裁集（`LINE`/`ARC`/`CIRCLE`/`SPLINE`/`ELLIPSE`/椭圆弧/`LWPOLYLINE`）；不可裁目标按 `extrim.unsupported_target` 逐对象跳过并保留其余裁剪；采样近似切点以 `extrim.approximate_cut` 明示；一条事务/一步撤销 | 无边界拾取提示；载荷无侧点通道，按目标中点选侧（声明）；边界必须是单个可解析图元，否则整命令具名拒绝 |
| `MPEDIT` | 部分实现 | 对每个选中 LWPOLYLINE 应用 `CLOSE\|OPEN\|REVERSE`，一条事务/一步撤销 | 非多段线/无变化逐条具名跳过；无 Join/Width/Fit/Spline/Decurve/Undo |
| `-OVERKILL` | 完整实现 | `OVERKILL` 的命令行拼写：同一语法、同一去重实现，同输入字节级相同结果 | 同 `OVERKILL` 的边界：仅完全重复，无部分重叠/共线合并 |
| `DRAWORDER` | 完整实现 | `FRONT`/`BACK`/`ABOVE`/`BELOW` 按 `SpaceId` 逐范围（模型空间、各图纸布局、各块定义）先规划后写入，范围内保持相对顺序，耗尽 `draworder.order_exhausted` 整批拒绝；ABOVE/BELOW 参照与移动对象跨范围时 `draworder.reference_scope`；一条事务/一步撤销 | 无 Draw Order 对话框 |

> **证据边界**：以上 20 条只有 Rust 契约测试与前端纯函数/结构测试；**没有** GUI、像素、真实 GPU、
> 真机或 vendor-AutoCAD 证据。前端测试在主仓库合并后运行通过；worktree 内因缺 `node_modules`
> 为 NOT RUN。

#### 2026-11-07 视图/空间批次（10 条）

| 命令 | 状态 | 结果 | 显式未实现 / 有意偏离 |
|---|---|---|---|
| `REDRAW` | 部分实现 | payload-free；推进当前视口刷新代次（session generation），不改相机、不写库、不进 `ZOOM PREVIOUS` | 不是 GPU 级重绘命令；只改会话代次，真实呈现由宿主消费 |
| `REDRAWALL` | 部分实现 | 同 REDRAW，对文档的所有视口 | 同上 |
| `REGEN` | 部分实现 | payload-free；推进当前视口 regeneration stamp，使派生显示缓存重建 | 不重解析底图、不写库；真实 GPU 重建未验证 |
| `REGENALL` | 部分实现 | 同 REGEN，对文档的所有视口 | 同上 |
| `VIEW` | **完整实现** | `SAVE/RESTORE/LIST/DELETE <name>`；命名视图持久化到文档 `tables/views.json`（`.ymodel` minor 18：相机+2D/3D 模式+保存的 2D 相机+工作平面+活动空间），名称不区分大小写；上限 32，第 33 个按 `view.cap_exceeded` 拒绝且不驱逐；`SAVE`/`DELETE` 一条事务/一步撤销，`RESTORE` 只应用会话相机/空间；Viewer 下 `SAVE`/`DELETE` 按 `PermissionDenied` 拒绝（`RESTORE`/`LIST` 仍可用） | 无 View 对话框/缩略图（构建级全局范围）；DXF/DWG `VIEW` 表经 acadrust 导入导出，源只存世界高度 ⇒ 正交比例按 `height/1000` 转换并逐行报告 `import/export.view_camera_approximated`；无布局名的 paper-space 行按 `import.view_paper_space_unresolved` 跳过 |
| `-VIEW` | **完整实现** | `VIEW` 的别名（同一 `CommandId`/路径/持久化表） | 无命令行选项提示差异 |
| `VIEWGO` | **完整实现** | `VIEWGO <name>`；走与 `VIEW RESTORE` 相同的持久化恢复路径，未知名 `view.unknown` | 无自动补全/对话框 |
| `VIEWBACK` | 部分实现 | 从每视口既有 `ZOOM PREVIOUS` 栈弹出；空栈为 `view.back.empty` no-op；记录前进项 | 不是 AutoCAD 的完整 ViewBack 顺序；pan/滚轮/orbit 不入栈（既有声明） |
| `VIEWFORWARD` | 部分实现 | 从新的每视口前进栈（上限 16）弹出并回记后退项；空栈为 `view.forward.empty` | **声明式偏离**：AutoCAD 的 VIEWFORWARD 与 ZOOM PREVIOUS 共用历史；本实现另建有界前进栈 |
| `VPOINT` | 部分实现 | 向量或 `TOP/BOTTOM/FRONT/BACK/LEFT/RIGHT`；标准词复用标准视图路径，向量进入 3D 透视方向；零/非有限向量按 `vpoint.invalid_vector` 拒绝；不改模型 | 无 `Rotate`/`Set UCS` 等子选项；向量形式的相机距离/目标沿用现有 3D 进入规则 |

> **证据边界**：以上 10 条只有 Rust 契约测试与前端纯函数/结构测试；**没有** GUI、像素、真实 GPU、
> 真机或 vendor-AutoCAD 证据。view-batch 的前端测试在主仓库合并后运行通过；worktree 内因缺
> `node_modules` 为 NOT RUN。

#### 2026-11-07 布局/视口批次（10 条）

> **重要**：`cad-db` 没有 post-import 的布局/`PaperViewport` 写 API（只有 builder 写）。因此本批
> 只有 **`LAYOUT LIST`/`LAYOUT SET`/`VPMAX`/`VPMIN`** 是可用的真实路径；其余写形式在完成名称/参数
> 校验后以稳定具名原因**拒绝**，不会克隆后静默发布，也不会伪造成功。

| 命令 | 状态 | 结果 | 显式未实现 / 有意偏离 |
|---|---|---|---|
| `LAYOUT` | 完整实现 | `LIST` 读布局表、`SET <name>` 切换活动空间（会话状态）、`NEW`/`RENAME`/`DELETE`、`COPY <source> <new-name>` 在一个校验批次里复制布局行+全部视口（clip 含旧 3 点形、`model_to_paper` 比例、`layer_overrides`）+图纸空间实体（新 id）+存储页面设置（标 Authored）；`AcDbViewport` 边框实体不复制并具名报告 | `TEMPLATE` 为解析期具名拒绝（本构建无跨图纸模板导入，绝不映射成 `COPY`）；无布局对话框/选项卡右键菜单 |
| `LAYOUTMERGE` | 部分实现 | 校验 source/target（缺失/同名）后按 `layoutmerge.write_unsupported` 拒绝 | 无布局/实体移动或布局删除 API；不移动任何实体 |
| `LAYOUTWIZARD` | 完整实现 | 创建布局 + Authored 纸张 + `ARRANGE <排列>` 等分格视口，布局行/视口/页面设置一条 `LayoutPatch`/一步撤销；未知排列/未知键/重复键/缺值具名拒绝且不写 | 打印设备/打印样式表/标题栏按请求逐条 `layoutwizard.*_not_implemented`（全局范围）；交互式向导提示未实现；仅等分格排列，无浮动视口与视口选择 |
| `VPORTS` / `-VPORTS` | 部分实现 | 校验 `rows`/`cols` 正整数与图纸空间后按 `vports.write_unsupported` 拒绝 | 无 `PaperViewport` 创建 API；`-VPORTS` 是同一实现 |
| `VPMAX` | 部分实现 | 会话标记活动图纸布局的第一个视口（无每视口选择模型）；不写库 | 不改变视口几何；真实最大化显示由宿主消费（未验证） |
| `VPMIN` | 部分实现 | 恢复会话标记；空状态为 `vpmin.nothing_to_restore` | 同上；不写库 |
| `VPCLIP` | 完整实现 | 矩形（`RECT`）与多边形（N 点）clip 都写入并真实渲染：表示层把矩形分派到原半平面裁剪器（输出逐字节不变），多边形按交点分段 / 圆锥曲线参数切分 / 网格与填充与图像耳切凸窗求交；无索引时作用于 `CVPORT` 当前视口，显式索引优先；一条事务/一步撤销 | 声明的边界：无平移的 3 点边界按 `*.ambiguous_triangle` 拒绝（否则会存成旧 3 点记录）；视口「孔洞」（多环裁剪）不建模；退化/自交/非共面按 `viewport.polygon_*` 具名拒绝 |
| `VPSCALE` | 完整实现 | 写目标视口的比例：无索引 = `CVPORT` 当前视口，显式索引可指定；存对角线 `1/factor` 并重算平移以保留模型锚点；零/非有限/未知比例与越界索引按名拒绝；一条事务/一步撤销 | 无「按视口比例统一图纸单位」的 `PSLTSCALE` 联动；无比例列表对话框（`SCALELISTEDIT` 另计） |
| `LAYVPI` | 完整实现 | 无索引 = 冻结所选图层于**除 `CVPORT` 当前视口外的全部视口**（写入各视口 `layer_overrides`，纸空间表示按视口过滤模型几何），显式索引形式优先；当前视口是唯一视口时 `layvpi.no_other_viewport`；一条事务/一步撤销；**绝不**全局冻结 | 导入的 VPLAYER 记录未解析（属导入侧，不在本命令范围）；无 `Settings` 提示环 |

> **证据边界**：以上 10 条只有 Rust 契约测试与前端纯函数/结构测试；**没有** GUI、像素、真实 GPU、
> 真机或 vendor-AutoCAD 证据。layout-batch 的前端测试在主仓库合并后运行通过；worktree 内因缺
> `node_modules` 为 NOT RUN。

#### 2026-11-07 标注编辑批次（10 条）

> **重要**：`DIMJOGLINE`/`DIMBREAK` 与 `DIMSTYLE SET` 在域/数据库中没有可写字段或 post-import
> 写 API，因此只有参数校验 + 稳定具名拒绝；其余命令是真实读写路径。

| 命令 | 状态 | 结果 | 显式未实现 / 有意偏离 |
|---|---|---|---|
| `DIMSTYLE` | 完整实现 | `LIST`/`CURRENT` 报告存储行；`SET`（会话当前样式，只影响新建标注）、`NEW`（以**存储的** `Standard` 或指定基准为模板）、`RENAME`、`DELETE`、`MODIFY <var> <value>` 都真写样式表，一条记录/一次修订，重拼写为权威空操作；被已存标注快照引用的行不可改名（`dimstyle.in_use`）；**非追溯**（ADR-0017）：既有标注保留解析快照并在诊断中说明；SAVE→OPEN 往返有测试 | 行内没有的字段按名拒绝；`Standard` 不可删除；无标注样式对话框 |
| `-DIMSTYLE` | 完整实现 | `DIMSTYLE` 的命令行拼写：同一解析器与同一处理器，七个选项逐项测试（无第二条路径） | 同 `DIMSTYLE` 的边界（`SET` 等若在该行有界，此处一致） |
| `DIMTEDIT` | 完整实现 | 恰好一个选中标注；`MOVE`/`LEFT`/`RIGHT`/`CENTER`/`HOME`/`ANGLE`；Left/Right/Center 移动锚点并设置字形级 `h_align`，**角标注按已绘制弧的起/终/中角锚定、坐标标注按引线特征端/引线端/中点锚定**（覆盖全部标注类型）；`OBLIQUE <deg>` 可键入并按 `dimtedit.oblique_unsupported` 拒绝；一条事务/一步撤销；无变化 `dimtedit.unchanged` | 唯一残留：角标注显示为导入图片（无弧子图）时 `dimtedit.justify_unsupported`，不写；无交互文字拖动；导入默认 Center、导出不写组 72 |
| `DIMEDIT` | 部分实现 | 所有选中标注；`TEXT <override>` 逐字存组 1（`<>` 占位，≤1024），`RESET` 删除；一条事务/一步撤销 | 延伸线编辑 NOT IMPLEMENTED；无对话框/逐项确认 |
| `DIMOVERRIDE` | 部分实现 | 闭集的实体快照变量（`DIMTXT`…`DIMLUNIT`）；一条事务/一步撤销；样式表不改 | 未知变量、仅表变量、坏值、未实现值分别具名拒绝；无对话框 |
| `DIMREGEN` | **完整实现（2026-11-24）** | payload-free；本构建标注全部非关联（关联性属 ADR 全局范围），返回 `dimregen.non_associative` 且不改变任何状态；无关联标注可重算即该命令语义的全部 | 无交互 Select objects；no-op 不产生 revision/历史 |
| `DIMDISASSOCIATE` | **完整实现（2026-11-24）** | payload-free；本构建标注全部非关联（关联性属 ADR 全局范围），返回 `dimdisassociate.already_non_associative` 且不改变任何状态；无关联可删即该命令语义的全部 | 无交互 Select objects；no-op 不删除/重建标注 |
| `QDIM` | 部分实现 | v1 仅直线 `LINE`：每条线创建一个 `DimensionKind::Aligned`（端点距离，尺寸线按 `DIMDLI` 偏移），一条事务/一步撤销 | 圆/弧/点/多段线等按名拒绝；无径向/坐标 QDIM、无位置提示/点输入 |
| `DIMJOGLINE` | 部分实现 | `ADD`/`REMOVE` 语法校验后按 `dimjogline.unsupported` 拒绝 | 域无 jog 字段/存储；不伪造折线 |
| `DIMBREAK` | 部分实现 | 按 `dimbreak.unsupported` 拒绝 | 无标注线打断记录；绝不删除/裁剪标注本身 |

> **证据边界**：以上 10 条只有 Rust 契约测试与前端纯函数/结构测试；**没有** GUI、像素、真实 GPU、
> 真机或 vendor-AutoCAD 证据。dim-batch 的前端测试在主仓库合并后运行通过；worktree 内因缺
> `node_modules` 为 NOT RUN。

#### 2026-11-07 组/注释批次（10 条）

> **重要**：`cad-db` 没有 DWG group 对象/组表，`HatchGeometry` 没有图案原点字段，本构建也不保留
> xdata。因此组系列是**会话内**组表（不写库、不过撤销栈）、`HATCHSETORIGIN`/`XDLIST` 是稳定具名
> 拒绝/只读报告；其余是真实写入路径。

| 命令 | 状态 | 结果 | 显式未实现 / 有意偏离 |
|---|---|---|---|
| `GROUP` / `-GROUP` | **完整实现** | 文档持久编组表 `tables/groups.json`（minor 17；行序列号/名字/描述/DXF selectable/有序成员实体 id，上限 64、名字大小写不敏感）；裸命令/`LIST` 只读、`CREATE` 从选择集建组、`DELETE` 只移除名字；一条事务/一步撤销，`ChangeScope.tables = {Groups}`；DXF/DWG 按 `AcDbGroup` 导入导出（名字经 `ACAD_GROUP` 字典），描述/selectable 保留 | 未写出的成员（v1 图纸空间延后）按 `export.group_member_not_written` 具名报告；含空格的组名用双引号寻址（如 `UNGROUP "My Group"`） |
| `UNGROUP` | **完整实现** | `UNGROUP <name>` 移除一个持久编组行，保留全部实体；未知组名拒绝；一条事务/一步撤销 | 不写实体几何 |
| `GROUPEDIT` | **完整实现** | `<name> ADD|REMOVE|RENAME <arg>`；成员来自当前选择集或键入实体 id；`RENAME` 允许仅改大小写；持久表一条事务/一步撤销 | 不写实体几何；无组选择对话框 |
| `QLEADER` / `LEADER` | 部分实现 | `<arrow x,y> <landing x,y> <文本>` 创建**一个** `MULTILEADER`（旧实体映射为 MULTILEADER）；一条事务/一步撤销 | 无 QLEADER 设置对话框、引线样式、注释类型、多引线段/附着选项 |
| `TEXTFIT` | 部分实现 | `<目标宽度>` 仅按 `height = target / char_count` 改字形高度 | 真实宽度拉伸/压缩 NOT IMPLEMENTED；坏宽度、空选择、非文字、空文本具名拒绝 |
| `HATCHSETBOUNDARY` | 完整实现 | 恰好一个 hatch + 一条闭合 Polyline/整圆/**完整 Ellipse**（整椭圆采样为单个 64 点环、无 bulge），共面性校验；`HatchGeometry::new` 重建并丢弃旧种子；一条事务/一步撤销 | 部分椭圆 `hatchsetboundary.open_ellipse` 拒绝且不开事务；无交互边界选择、无关联边界、无椭圆弧/块内边界 |
| `HATCHSETORIGIN` | 完整实现 | `<x> <y>` 绝对图案原点：锚为首个定义行的合成基点（`absolute_pattern_origin()`），写入位移 `P − 锚`，行列/角度/比例参与；一条事务/一步撤销 | 无定义行的图案无法定位 → 收窄的 `hatchsetorigin.missing_field`；非有限位移 `unrepresentable`；非模型空间 hatch 不可寻址 |
| `XDLIST` | 部分实现 | 只读；每个选中实体报告 `xdlist.not_retained` | 本构建不保留 xdata；不伪造条目 |

> **证据边界**：以上 10 条只有 Rust 契约测试与前端纯函数/结构测试；**没有** GUI、像素、真实 GPU、
> 真机或 vendor-AutoCAD 证据。annot-batch 的前端测试在主仓库合并后运行通过；worktree 内因缺
> `node_modules` 为 NOT RUN。

#### 2026-11-07 绘制/视图补充批次（10 条，其中 `TRACE` 同名不同义不计入覆盖）

> **重要**：`TRACE` 在 AutoCAD 2026 是**协作追踪**（打开/管理 trace），本构建的同名命令是
> **旧式宽线带**（闭合 4 顶点 LWPOLYLINE）。二者语义不同，因此该条按“同名不同义”处理，
> **不计入覆盖**（与 `MEASURE` 同一口径）；其余 9 条按下列边界计入。
> `VIEWRES` 的 token 已被显示构建消费；`VSCURRENT`（2026-11-17 起）只切 **3D body** 的视觉样式
> （`RepresentationContext::body_wireframe`），其余几何不变，像素/GPU 效果 NOT VERIFIED。

| 命令 | 状态 | 结果 | 显式未实现 / 有意偏离 |
|---|---|---|---|
| `SKETCH` | 部分实现 | 仅 `CommandPayload::Points`：一条活动层开放 LWPOLYLINE；少于两点或非有限点拒绝；一条事务/一步撤销 | 交互式定时采样 NOT IMPLEMENTED（采样由宿主负责） |
| `TRACE` | 同名不同义（不计入） | 旧式宽线带：`TRACE x1 y1 x2 y2 width` 生成闭合 4 顶点 LWPOLYLINE；一条事务/一步撤销 | 不是 AutoCAD 2026 的协作 `TRACE`；无 trace/标记/评审模型 |
| `MOCORO` | 部分实现 | `MOVE|COPY dx dy`、`ROTATE bx by degrees`、`SCALE bx by factor` 作用于当前选择，委托既有 MOVE/COPY/ROTATE/SCALE 路径；COPY 一条事务/一步撤销 | 无交互 gizmo/选项循环；空选择 `mocoro.selection_required` |
| `MSTRETCH` | 完整实现 | `MSTRETCH <base_x> <base_y> <delta_x> <delta_y> <x1> <y1> <x2> <y2> […]` 与载荷一一对应（窗口逻辑像素、base/delta 世界点）；一个包络计划、一条事务/一步撤销 | 奇数 token `mstretch.argument_odd`（绝不截断半窗）；退化窗口/零位移在事务前拒绝 |
| `-PAN` / `PAN` | 部分实现 | `-PAN` 是 `PAN` 别名；`dx dy` 或两点，走既有相机路径 | 只改视图：不写库、不 bump 修订、不入历史 |
| `VIEWRES` | 完整实现 | 整数 `1..=1000`（越界 `viewres.out_of_range`）成为弦分段上限：约束样条、折线 bulge 弧、非 SDF 圆锥的弦回退与宽折线带，`1..=2` 夹到 3 段下限；无库写/无撤销 | 声明：圆/弧/椭圆是解析 SDF，无弦可限，本变量不改变其平滑度；这是显示容差上限，不是平滑度设置；像素效果 NOT VERIFIED |
| `VSCURRENT` | 部分实现 | 会话级 **3D body 视觉样式**（**默认 SHADED**，与 AutoCAD 的 2D-wireframe 初始值**有意偏离**，以保留 token 之前的填充网格显示）；接受 `WIREFRAME`/`2DWIREFRAME`/`2D` → wireframe（有存储 body 边线就用它，否则用去重的三角形边段），`SHADED`/`SHADEDWITHEDGES` → 填充；其余样式 `vscurrent.style_not_implemented` | **仅 body**：只切 `RepresentationContext::body_wireframe`（唯一消费者是 body 片段构建器），其它几何不变；无库写/无 revision/无历史；保存与矢量出图**不接收** `body_wireframe`（出图仍为阴影/填充）；契约证据 only，像素/GPU NOT VERIFIED |
| `REGEN3` | 部分实现 | `REGEN` 的别名（同一 `CommandId`/路径） | 不修复 3D 实体/曲面显示异常（无该能力） |
| `LINETYPE` / `-LINETYPE` | 完整实现 | `LIST`、`SET <name>`、`CREATE <name> <dash>,<gap>,… [DESCRIPTION <text>]`、`DELETE <name>`；两条写命令各一条事务/一步撤销，Viewer 拒绝写入而 `LIST` 仍合法；名称语法/重复/保留名/图案元素（组 49，正划负空、有限、至少一段可见划）与在用引用逐条具名拒绝；`-LINETYPE` 经别名表归一化到同选择子/同解析器/同处理器；导出写全部行、导入可重建（创建行有往返测试） | `DESCRIPTION` 带文本按 `linetype.description_not_modelled` 拒绝（行无描述列，需域字段+导入导出扩展）；`LOAD`（`.lin` 文件）仍 NOT IMPLEMENTED（无文件装载通道，独立命令）；id 分配为 `max(id)+1`（无分配器字段，补丁携带整行故撤销/重做仍精确） |

> **证据边界**：以上 9 条（`TRACE` 除外）只有 Rust 契约测试与前端纯函数/结构测试；**没有** GUI、
> 像素、真实 GPU、真机或 vendor-AutoCAD 证据。extras-batch 的前端测试在主仓库合并后运行通过；
> worktree 内因缺 `node_modules` 为 NOT RUN。

**仿射不变规则（`SPLINE`，2026-10-26）**：NURBS 曲线对**仿射**映射不变——把每个控制点过同一个
仿射映射，而 `degree`、`knots`、`weights` **原样保留**，得到的正是原曲线的**精确仿射像**。因此平移、旋转、
**均匀与非均匀**缩放、镜像与剪切**全部精确**：无需重拟合、无需重参数化、权重不变。`cad-db` 的
`transform_geometry` 据此对样条只映射控制点并逐字复制三个定义向量（`Transform3::apply_point` 只用
0..2 行、不做齐次除法，故严格仿射，任何投影行都不参与）；`knots.len() == 控制点数 + degree + 1`、
`weights.len() == 控制点数`、节点非降序这三条不变量**按构造成立**（向量是复制而非重算），写入前仍由
`validate_geometry` 复核。纯逐点映射**不**是「移动曲线的一段」——控制点的支撑域是 `degree + 1` 个节点
区间，这一性质与部分包围 STRETCH 的整曲线重塑同源。仍**拒绝**（同一理由的保守拒绝，本轮未放宽）：
`SHAPE`（存储的是 `position`/`size`/`rotation` 的单字形，只有相似变换有处安放，且其变换臂未实现）、
`Opaque`（代理字节，无几何可映射）、`Compound`（需逐成员映射，成员可能含本构建拒绝的种类）。
`TEXT` 的相似变换限制、`MultiLeader` 的 `type_key` 拒绝等**全部逐字未变**。

- **证据范围**：上述表格是**契约/场景**证据，不等于真实图纸兼容性。Linux 桌面宿主（React/Tauri）
  另有**真实硬件 GPU** 有界证据（Vulkan / Quadro P620 / X11 —— **CAD-on-top 时代**证据；
  2026-10-23 起原生子表面路径在 X11 上被拒绝，**2026-10-24 起 X11 改走 DOM-canvas**（ADR 0024，
  合成夹具出图 + 用户手动确认缩放平移，见 `docs/validation-dom-canvas-wasm-2026-10-24.md`；
  硬件归属 NOT VERIFIED）：打开与渲染、悬停、框选（含橡皮筋像素判据）、
  TRIM 的交互链（实体拾取/漏点拒绝/CONFIRM 到达命令层）、以及 **ERASE 的真实提交**（`entityCount` 4→3）。
  **注意**：这些运行的输入由**合成 DOM 指针事件**驱动（Rust probe 经 `webview.eval` 注入、在 webview 内
  `dispatchEvent`，走正常 UI→IPC→宿主路径），**不是真实鼠标**、不经 evdev/XTest——因此它们证明的是
  "UI→宿主→命令→文档"的接线与结果，不构成 OS 级输入集成证据。**桌面端 `TRIM`/`EXTEND` 的取点已接线**（选择目标 + 边界两次实体拾取 → `CONFIRM`），旧文档中的
  `linux.trim`/`linux.extend` 显式 `Unsupported` **已不存在**；但当前唯一在库 DWG 夹具是四条**平行**竖线，
  因此"桌面端真的裁剪出几何改动"与本仓库的 EXTEND 演示**尚未取得真机证据**（命令层与宿主层有确定性测试，
  见 `docs/handoff-archive/2026-10.md` 的同日期条目）。Windows 宿主 2026-10-23 起已实现（Tier 0 编译 + Wine 拒绝合同，视觉/运行时未验收）；
  macOS 宿主已实现但仅 CI 编译；Android 自 2026-11-01 起是原生 `native-below` 载体（编译级证据，
  设备运行/Vulkan/像素 **NOT VERIFIED**）；2026-11-02 起 Android 已接 SAF 打开与 DXF 另存（设备/模拟器
  选择器与实际读写 **NOT RUN**）；Web 的 wasm 会话已实现，其 host target 仍是空壳。

## 三维

- **缩放与视图历史（2026-10-28）**：`ZOOM WINDOW`（两角框选，**仅 2D 正交**——3D 按名拒绝 `zoom.window.requires_2d`；另一捕获或模态 pan 占着指针时 `zoom.window.capture_busy`；退化/非有限矩形**不派发**；拟合 margin **`1.0`**、`FIT` 是 **`1.05`**（**我们的选择**）、同一条较紧轴规则；`view_mode` 保持 2D、**不进历史、不 bump `session.generation`**）与 `ZOOM PREVIOUS`（**每视口**一条上限 **16** 的**离散**视图栈，最旧丢弃；快照 = camera + view_mode + work_plane；在 FIT/`ALL`/`EXTENTS`/`ZOOM <factor>`/`ZOOMIN`/`ZOOMOUT`/旧 `Points`/`ZOOM WINDOW`/`STANDARDVIEW`（2D 下 plan 是 no-op）/`SWITCH2D3D`/`PROJECTION`/`RESETVIEW` **之前**记录；**pan、滚轮缩放、orbit 拖动不记录**——**声明的偏离**；空栈 ⇒ 非错误诊断 `zoom.previous.empty`）。两者都是 `ZOOM` 的**参数写法**（只认全词，`ZOOM W`/`ZOOM PREV` 拒绝；不新增选择子）。**证据边界**：只有契约测试（`cad-app` / `cad-ui-core` / `cad-host`），**无 GUI/像素/真实按键证据**。
- **`ZOOM OBJECT` / `nX` / `nXP`（2026-11-11）**：`OBJECT` 以当前选择为输入，模型空间经 `drawing_pick_items` + `pick_world_bounds`（块成员/选中 INSERT 展开）、图纸空间只接受可直接寻址的布局实体，空选择/无界/3D/跨文档/不可寻址逐条具名拒绝，成功时用同一条 rect→camera 规则与 FIT margin `1.05`；`nX` 是视口中心的相对倍率；`nXP` 是**图纸空间视口比例**（`2XP` ⇒ `VPSCALE 1:2`），实际写入复用已有 Work-only `VPSCALE` 路径（一条事务/一步撤销），模型空间拒绝 `zoom.xp.requires_paper`。**`ZOOM DYNAMIC` 仍是具名未实现**（`zoom.dynamic_not_implemented`），不伪装成交互缩放。UI：View → navigate 提供 `ZOOM OBJECT` 按钮（`selectAll` 图标）；`nX`/`nXP` 仅命令行。证据只有 Linux 编译 + 契约测试；GUI/像素/真实鼠标 **NOT VERIFIED**。
- 观察（轨道/标准视图/正交-透视）：相机/投影/标准视图/轨道命令与宿主接线齐全
  （`crates/cad-app/src/camera/` → `cad-host/src/state.rs` 的 `ViewSnapshot` →
  `cad-engine` 按 `view_mode` 选 `render`(2D)/`render_3d`(`Camera3d`)）。
  **可达性**：命令行输入 `SWITCH2D3D` 进入 3D（无负载、在命令表内），3D 下**主键拖动即 orbit**；
  `ORBIT` 按设计是手势命令（`GESTURE_ONLY_COMMANDS`），不是按钮。
  **标准视图**经专用 typed IPC `cad_standard_view { view }`（有界队列 + 与命令同门禁，
  名字大小写不敏感、未知值具名拒绝），走真实 `CommandId::StandardView`；`PROJECTION` 无载荷、
  命令行本就可达。**Ribbon 已接线**（2026-10-21）：`view` 标签的 3D 面板提供 `2D/3D`、`投影`、
  七种标准视图，`环绕` 按设计**可见但禁用**（手势理由）；文案复用既有 `view.*` 键。
  **真机 3D 出图已验证**（Quadro P620 / Vulkan / X11 —— CAD-on-top 时代证据，X11 现被显式拒绝）：ACIS 立方体在斜视下为 **6 顶点凸包、三面、
  平行棱收敛 31.2%、对角线不等 10.1%**，隐藏边不穿透（`docs/handoff-archive/2026-10.md` 2026-10-21 条目）。
  边界：俯视正交下"有面立方体"与"填色方片"**像素等价**，故 2D 帧不能证明三维性；
  无 golden/跨 GPU/vendor/性能结论。
  **着色约定**：2D 保持**世界 -Z 头灯**（位级不变）；3D 用**相机相对、偏离视轴的固定主光**
  （`L = normalize(toward_eye + (-0.10)·right + 0.35·up)`，约在视线**左上方 20°**；
  环境 0.25 + 漫反射 0.75，无高光/阴影/环境光）。**纯头灯在等轴测下会退化**：三面相对视线对称
  ⇒ 实测三面同为 `215`；偏移后等轴测为 **204 / 189 / 238**，斜视为三档可辨。
  边界：仍是**显示约定**而非物理光照，两个面在特定方向仍可能接近，反照率/曝光不可分离。
- **轨道 / 标准视图的载体能力（2026-11-13）**：ViewCube 的轨道请求经专用 typed IPC
  **`cad_orbit { yaw, pitch }`**（弧度、两者必须有限）投递，与 `cad_standard_view` 同一门禁顺序
  （模态弹窗 → CAD 模式/probe → 有界队列）、同一 `{accepted:true}` 回执语义（桌面 = 已入队，
  `dom-canvas` = 已内联应用），走真实 `CommandId::Orbit` + `CommandPayload::Orbit`。
  载体能力由 `cad_status.hostProfile.orbit` 声明（三态，与 `standardViews` 同形）：
  `null` = 未声明（历史允许），`false` = 已声明的**不可用**。

  | 载体 | `cad_standard_view` | `cad_orbit` | `hostProfile.orbit` | 拒绝 |
  |---|---|---|---|---|
  | `native-below`（Wayland / Windows / macOS / Android） | 支持 | **支持**（仅 3D；2D 是宿主真实拒绝 `Orbit is only defined in the three-dimensional view`） | `null`（未声明 = 历史允许） | 非有限角：`orbit.angles_not_finite: cad_orbit needs finite yaw and pitch`；队列满：`CAD orbit queue is full`；模态：`popup.modal_open`；probe：`CAD is disabled in probe mode` |
  | `dom-canvas`（X11/XWayland / Web） | 声明 `standardViews: []`（前端禁用七键） | **声明不可用**：`orbit: false`，adapter/front-end 按名拒绝；wasm 会话是 2D-only，若请求到达会话亦为同一真实拒绝 | `false` | 同上（会话侧真实拒绝；载体侧前端显式禁用） |

  **证据边界**：只有编译 + 契约测试（`cad-ui-core` 的 admission/wire/`CadSession::orbit` 测试、
  `app-web` 双 sink 等价矩阵）。轨道**手感**、方位球（ViewCube 组件）交互、真机输入、GPU/像素、性能一律
  **NOT VERIFIED**；Windows/macOS/Android 只有编译门禁（未构建的平台写 NOT RUN），不得写成已验收。
- **3D 默认观察 / 3D FIT / 透视缩放（2026-11-22）**：native 载体（`native-below`：Wayland /
  Windows / macOS / Android）打开文档后，若 model space 含 `SemanticGeometry::Body3d`，或包围盒
  `z` 范围显著（`extent_z > max(diagonal × 1e-6, 1e-9)`），默认应用等轴测 3D
  （`StandardView::Isometric`）；纯 2D 文档仍是俯视 plan。判定在 `cad-app::document_is_3d`，
  打开漏斗是 `cad-host::HostCore::fit_opened_document`（同步 deliver / 异步 publish /
  `.ymodel` / demo）。自动进入 3D **不额外记录** `ZOOM PREVIOUS`；plan fit 自身仍按既有规则记一步。
  **3D `FIT` 只重取景**：保持当前 eye 方向与 `up`，target 移到包围盒中心（含 `z`），距离按
  `radius / tan(fov/2) × FIT_3D_MARGIN` 重算，`view_mode` 保持 3D；2D FIT 不变（plan + `1.05`）。
  **透视 `zoom_at`**：target 固定做 dolly（距离 `/factor`），再在屏幕平面平移 eye+target，使光标下
  世界点的像素不变；因此后续 orbit 围绕模型的 target，不再被缩放推过模型。orbit 速率未改
  （`0.0035 rad/逻辑像素 × orbitSensitivityPercent/100`）。**证据**：`cad-app` / `cad-host`
  契约测试 + `cargo check -p app-linux` + 前端 `npm test`/`build`
  （`docs/validation-3d-view-2026-11-22.md`）；真实 GUI/像素、真实 3D DWG 打开手感、
  Windows/macOS/Android/Web 运行时 **NOT VERIFIED**。Web/`dom-canvas` 仍是 2D-only
  （`hostProfile.orbit=false`、`standardViews: []`），不套用这套 3D 默认。
- **方位球（2026-11-22）**：ViewCube 从 56px 导航列移到绘图区独立浮层（桌面右上、列左侧；
  **紧凑布局同为右上**，让开右上角导航卡），渲染为不透明球面 + 六个方向帽 + 网格；`--navrail-w` 仍为 **56px**，
  导航列宽度与 CAD 分区矩形不变。点击方向帽 / Home 仍走 `cad_standard_view`，拖动仍走
  `cad_orbit`；sr-only 六键、admission、可见性门禁不变。**证据**：`apps/desktop-ui`
  契约测试 + Vite build；真实 GUI/像素 **NOT VERIFIED**。
- 直接网格（Mesh/3DFACE）：数据、法向、深度、绘制顺序与透明通道已实现；透明合成有
  软件 Vulkan（lavapipe）无头测试。真实 GPU 行为未验收。
- 底图透明度：importer 读取 acadrust `Transparency`（`ByLayer`/`ByObject`/`ByBlock`）
  → `DisplayFragment.alpha` → `RenderBatch.alpha`，由渲染器分类为不透明/透明/不可见
  （`docs/render-order.md`）；plot-style 表 alpha 未应用（acadrust 只暴露样式表引用名，
  不暴露解析后的颜色/线宽/alpha 值；引用名本身由 `cad-connector-acadrust` 读取并显式报告
  为 Unsupported）。
- ACIS 曲面（显示）：**已接线**——导入期离散为中性网格（容差 = 包围盒对角线的 0.1 %），
  恒 `Partial` 并声明「离散近似不随缩放细化 + 源 ACIS 不保留」；超出子集/原始 SAT/SAB 被拒仍为 `Opaque`
  并带内核具名码。详见「ACIS 实体显示状态」。完整 ACIS/样条面支持仍超出范围。

### 触控板缩放（pinch / trackpad zoom，2026-11-13）

**契约（跨载体，共享层先行）**：两指 pinch 是 **CAD 缩放**，**永不缩放页面**；某载体观察不到该手势
时必须显式说明，绝不静默缩放页面。共享入口是 `cad-ui-core` 的 `NativePinch` / `PinchViewport` /
`native_pinch_input`（`crates/cad-ui-core/src/session/input.rs`）：载体上报**自身坐标**，owner tick 用自己的
布局度量（CSS 矩形原点 + `devicePixelRatio / parent_scale`）换算成 canvas-local CSS px，再交给**同一个**
`CadSession` pinch 语义（`InputKind::Pinch` 的增量 `scale`）。原生样本经
`cad-desktop-shell` 的有界队列（`Shared::cad_pinches`，上限 `NATIVE_PINCH_CAP`）由 tick 取出，并**追加在
该 tick 的 DOM 输入之后**（一次 `handle_inputs` 发布）。`sequence = 0`：该路径不经 `InputTracker`。

DOM 信号路线不需要上述原生队列：WebView2/Chromium 把触控板 pinch 合成为 `wheel` + `ctrlKey`，WKWebView/
Safari 发 `gesturestart/gesturechange/gestureend`，两者由 `apps/desktop-ui/src/cad/input.ts` 归一成同一条
`kind:'pinch'`（位置为手势中点、`dx/dy = 0`、`scale` 为**增量**乘数），经唯一的 `probe_input` 漏斗送到
**已持有** `cad-db` 的会话（`native-below` 的 native 会话或 `dom-canvas` 的 wasm 会话）。Linux/WebKitGTK
没有 DOM 事件，才走上面的 UI 进程取样路线。

| 载体 | 手势来源 | 页面是否缩放 | pinch → CAD | 证据 |
|---|---|---|---|---|
| Linux 原生 Wayland（`native-below`） | WebKitGTK 的 pinch 在 **UI 进程**（`WebKitWebViewBase` 的 `GtkGestureZoom`），**没有 DOM 事件**，`preventDefault()` 也拦不住 | **否**：`gtk_bridge::pinch` 在 webview 创建时接管——2.52.6 把私有 `wk-view-zoom-gesture`（`GtkGestureZoom`）的 **propagation phase 置为 `NONE`**，再在 widget 的 **`event` 信号**上拦截 `GdkEventTouchpadPinch` 并返回 `Stop`（事件到不了 WebKit 的 vfunc，`pageScaleFactor` 不变；对象/数据不销毁不释放）；>= 2.54 走公开 `WebKitWebView:magnification`（钉在 `1.0`） | **是**：`event` 信号取 `scale`/位置 → `NativePinch` → tick → 与 DOM 两接触 pinch 同一会话语义（锚点必须在 partition 内，否则丢弃）。触屏两指是 `GDK_TOUCH_*` 序列、不是 touchpad 事件，仍归 DOM 两接触路径 | 编译 + 契约（`cad-ui-core` 换算/拒绝测试、`pinch_plan` 纯测试、intake 越界丢弃测试）+ **合成 GDK `TOUCHPAD_PINCH` 实测**（真实 Wayland 会话：`visualViewport.scale` 保持 `1.0`，`cad_status.camera.scale` 按累计 `1.25×1.6≈2.0` 变化）；**真实触控板 NOT VERIFIED** |
| Linux X11/XWayland（`dom-canvas`） | 同上（仍是 WebKitGTK UI 进程手势） | **否**（同样接管/中和） | **否**：该载体没有原生 CAD 会话（铁律 8，`surface_plan().per_frame_tick == false`），所以**不装**原生取样队列；`cad_status` 之前已有的 DOM 两接触路径只覆盖触屏。pinch 因此**不产生 CAD 缩放**，日志明说 | 编译 + 契约；**运行时 NOT VERIFIED** |
| Windows（`native-below`） | WebView2（Chromium）：OS 把精确式触控板 pinch 翻译为 `WM_MOUSEWHEEL` + `MK_CONTROL`，Chromium 在渲染器侧派发**可取消**的 `wheel`（`ctrlKey === true`、`deltaMode = 0`、delta 为 **120 的整数量化倍数**，不与 log(scale) 成正比） | **否**：wry 已 `SetIsPinchZoomEnabled(false)`；`IsPinchZoomEnabled`/`IsZoomControlEnabled` 只拦页面缩放**应用**，wheel 仍先到渲染器，所以 `apps/desktop-ui/src/defaults.ts` 无条件取消 `ctrl`+wheel 的页面缩放 | 共享 DOM 路线：`ctrl`+wheel（WebView2/Chromium）或 `gesturechange`（WKWebView/Safari）→ `InputKind::Pinch`（与 Linux 原生手势同一会话语义） | 编译 + 契约测试（`test/input.test.ts` / `test/dom-pinch.test.ts`）；真机手势 NOT VERIFIED |
| macOS（`native-below`） | WKWebView：`gesturestart/gesturechange/gestureend` 仍会触发且**可取消**（`allowsMagnification` 缺省 `false`、wry 未设置 ⇒ 页面永不缩放）；Safari 15+ 对同一次 pinch **也发 `ctrl`+wheel** | **否**：`allowsMagnification` 缺省 `false`（未显式钉住，缺口即此处声明），`apps/desktop-ui/src/defaults.ts` 取消三个 gesture 事件与 `ctrl`+wheel | 共享 DOM 路线：`ctrl`+wheel（WebView2/Chromium）或 `gesturechange`（WKWebView/Safari）→ `InputKind::Pinch`（与 Linux 原生手势同一会话语义）。两种信号同时到达时**手势优先**：共享漏斗在手势进行中与 `gestureend` 后 500 ms（`GESTURE_WHEEL_GUARD_MS`）内丢弃该 `ctrl`+wheel，一次 pinch 只缩放一次 | 编译 + 契约测试（`test/input.test.ts` / `test/dom-pinch.test.ts`）；真机手势 NOT VERIFIED |
| Android（`native-below`） | Android WebView | **否**：`MainActivity.disableWebViewPageZoom`（`setSupportZoom(false)` + `setBuiltInZoomControls(false)` + `setDisplayZoomControls(false)`），`scripts/test-android-host.py` 钉住这三条 | **是（DOM 路线）**：WebView 把两接触 pinch 作为 DOM 事件交给页面，`apps/desktop-ui/src/cad/input.ts` 的 `pinch` 送到**原生**会话 | Kotlin 静态契约 + 编译；**设备 NOT VERIFIED** |
| Web / Chromium / Safari | DOM：`ctrl`+wheel（Chromium，以及 Safari 15+/macOS）或 `gesturestart/change/end`（Safari/WKWebView；iOS 只有 gesture） | **否**：`apps/desktop-ui/src/defaults.ts` 取消 `ctrl`+wheel 与三个 gesture 事件；`.native-input` 另加 `touch-action: none` | 共享 DOM 路线：`ctrl`+wheel（WebView2/Chromium）或 `gesturechange`（WKWebView/Safari）→ `InputKind::Pinch`（与 Linux 原生手势同一会话语义；`dom-canvas` 时由 wasm 会话消费）。Safari 15+ 两种信号同时到达时按 `GESTURE_WHEEL_GUARD_MS = 500` 去重（手势期间 + `gestureend` 后 500 ms 丢弃 `ctrl`+wheel） | 纯函数 + 绑定契约（`test/defaults.test.ts` / `test/input.test.ts` / `test/dom-pinch.test.ts`）；浏览器运行时 NOT VERIFIED |

**边界一句话**：本表全部是**实现/契约**级结论——两指 pinch 真的缩放 CAD 且页面不缩放、Android 设备、
Windows/macOS 手势、任何像素/性能结论都 **NOT VERIFIED**。人工验收路径见 `AGENTS.md`「运行与证据」：
`bash scripts/run-react-linux.sh --open <dwg>` → 在图纸上两指 pinch → 预期**页面不缩放**、CAD 缩放，
且启动日志出现一行 `[probe] pinch: …`（`webkit-gesture-neutralised` / `magnification-pinned` /
`gesture_unavailable`），它说明**实际**装上了哪条路线。

Windows/macOS 的人工验收（本轮 **NOT VERIFIED**，无真机）：在真实宿主里打开一张 DWG，把指针放在图纸上
两指 pinch，预期**页面不缩放**（不出现缩放指示、`visualViewport.scale` 保持 `1.0`），且 **CAD 缩放**
（`cad_status.camera.scale` 变化、手势结束后不继续漂移）。

## 平台

> **2026-10-23 重写**：旧段落的 Android/Web/“iOS/Linux/macOS/Windows 无宿主”描述属早期桌面 UI 时代，
> 已整体替换为当前状态。能力 ≠ 验收；**没有任何平台是“完整 CAD 产品验收通过”**。

| 平台 | 宿主状态 | 合成 | 有界证据 | 未验证 / 上限 |
|---|---|---|---|---|
| **GPU 显存预算（原生）** | 记账 + 显式预算（默认 3 GiB 估算）、拒绝 `RenderError::GpuMemory` / CLI 退出码 7 | 渲染器按类目记账（current/peak）；驱动 allocator 交叉校验 | Vulkan/P620 + `complex-test.dwg`：常驻 2.11 GB、交叉校验差 0.07 %、`stream` 1.70 GB；强制 8 MiB → exit 7 且无 PNG | 显存**总量**不可查（wgpu 无 API）；其它 GPU/驱动/样本、驱动 OOM 现场、D3D12 阈值均未验证 |
| **Linux / 原生 Wayland** | **受支持，真实可运行**（`apps/app-linux`，bin `yacr`） | **DOM on top**：app-owned `wl_subsurface` 无条件 `place_below(parent)`；GTK toplevel 透明 + 空 opaque region | 生产 `--open` 运行（真实 Vulkan/Quadro P620）、合成帧由同 UID Mutter `ScreenCast` 捕获；WP2 四条判据 PASS（带 caveat）、WP3 About 帧 PASS、**透明修复后复采有界 PASS**（A 几何满亮度/无蒙层、C About PASS；D textEdit 编辑器未像素证明、B 箭头抑制非确定性）；历史 textEdit 帧 **INVALID wrong-window** | 仅 **GNOME/Mutter + P620 + `scale=1`**；DPR>1/分数缩放、多显示器、跨合成器、性能、vendor/golden 均无结论 |
| **Linux / X11 / XWayland** | **已实现 `dom-canvas` 载体**（`apps/app-linux`；退出码 4 已是历史合同） | **DOM-canvas**：wasm/WebGL2 CAD 会话画进 WebView 内 `<canvas>`（`surfaceMode: dom-canvas`），**无**原生 CAD 表面 | 有界运行：`surfaceMode=dom-canvas`、canvas 挂载 + backing `626×341`、合成 fixture 出图（Mutter ScreenCast 抓帧）；**用户手动确认**滚轮缩放/拖拽平移（用户报告，**无** agent 像素差证据）；WebGL2 缺失 → `yacr: unsupported-runtime: webgl2` + **退出码 6** | 仅 **XWayland + GNOME/Mutter + `scale=1`**；真 X11 服务器/WM、其它合成器、**硬件归属（NOT VERIFIED）**、DPR/多显示器、性能、vendor/golden 均无结论；agent 侧输入注入不可用。**连接器**：wasm 走 `full-formats`（`scripts/build-canvas-wasm.sh`），格式集与 native 相同，不随浏览器部署包的 DWG/DXF 裁剪 |
| **Windows** | **已实现**（`apps/app-windows`，bin `yacr-windows`） | **DOM on top**：透明 WebView2（alpha 恰 0）在上，`WS_CHILD|WS_DISABLED` CAD child HWND 在下 | **Tier 0 编译门禁全 rc=0**（msvc clippy/check/build、mingw build、wasm 空壳 check）；**Wine 拒绝合同 smoke 3/3 rc=5** | **视觉/运行时全部 NOT RUN**；spike-1/4/5 **PENDING user real-Windows spike**；Wine 像素结论固定 `NOT VERIFIED (Wine limitation, bug 55432/58921)`；无发布打包 |
| **macOS** | **已实现**（`apps/app-macos`，bin `yacr-macos`） | **DOM on top**：透明 WKWebView 在上，原生 CAD 子 `NSView`（`addSubview:positioned:relativeTo:` `.below`）在下；wgpu/Metal presenter | 仅 **CI `macos-check` 编译门禁**（`macos-latest`，前端构建后 `cargo check`，不运行） | **窗口/透明合成/层序/Retina/Metal present/对话框/像素/性能全部 NOT VERIFIED**；renderer crash 触发 ④ 未实现；无签名/打包/分发 |
| **Android** | **原生 `native-below` 宿主**（2026-11-01 起，`apps/app-android`）：Tauri WebView + 共享 React chrome 在上，原生 `CadSession` 持有 `cad-db`，wgpu/Vulkan 呈现到 WebView 之下的 `SurfaceView`；wasm 会话不实例化 | **DOM on top**：`SurfaceView` 在 WebView 之下，WebView 背景清透明 | **编译级 + 开发机契约**：`cargo test -p app-android`（48 passed）、`cargo check --target aarch64-linux-android`、clippy、Kotlin 编译、`javap` 核对 JNI 符号、`test-android-host.py`（SAF/五方法/关联/ProGuard 静态断言）；release APK 已构建并本地签名（R8 `classes.dex` 核对），未安装 | **设备/模拟器运行、Vulkan adapter、像素、旋转/前后台、JNI 运行时链接、SAF 选择器与实际读写、外部「打开方式」全部 NOT VERIFIED**；**打开** = `ACTION_OPEN_DOCUMENT`（`*/*`）→ worker 读 → 共享 `FileEvent::Opened`；**另存** = `ACTION_CREATE_DOCUMENT` 导出 **DXF**；源为 `.dwg` 时导出**实验性 DWG**（上游 `DwgWriter`，默认新文件名 `<stem>-export.dwg`；SAF URI 守卫 + 归一化名回退都拒绝覆盖来源，属导出副本）。**DWG 只有合成 acadrust self-roundtrip 证据，AutoCAD/vendor 兼容性 NOT VERIFIED**；**New/Plot 不支持**；`content://` 无 sidecar xref/栅格；导入同步在 tick（`importWorker=false`，超大 DWG 停顿未测量）；弹窗接缝已接（in-DOM，Options/About/text-edit 可用，源码/契约级）且偏好持久化到应用 `filesDir`（设备落盘 NOT VERIFIED）；FIT 自 2026-11-08 起按**可见 CAD 矩形**取景（`SurfaceMetrics.origin` + `ViewSnapshot.canvas_origin` + `place_canvas_camera2d`），渲染目标仍为整窗；**3D 透视取景**未做离轴投影，属显式 **Partial**；2026-10-28 的 `dom-canvas` 模拟器证据属已退役载体，不得复用 |
| **Web** | **wasm CAD 会话已复活**（`apps/app-web`）；浏览器 bundle 本地可构建 | **DOM-canvas**：同一 wasm 会话画进 `<canvas>`（无第二份文档） | 本地运行（Chromium + WebKitGTK 2.52.6）：合成 fixture 出图与缩放；真实 DWG（4.2 MB）在 wasm 内导入（1.43 GB 堆 / 11.2 s）；Chromium **Worker 隔离**（`isolation=worker`），WebKitGTK 显式 `main-thread (no WebGL2 in worker)`；共享 chrome 由 `scripts/build-web-ui.sh` 构建。**文件访问（2026-11-01，2026-11-22 收窄）**：`OPEN` = 浏览器上传（隐藏 `<input type=file>`，slim 构建只接受 `.dwg/.dxf`；`.ymodel` 仍留在 picker 里，让会话自身的 `unsupported-runtime: ymodel` 到达用户；页面 transport 抬 `canOpen`，会话本身仍报 `canOpen:false`）；`SAVE` 无归档适配器 → `unsupported-runtime: ymodel`；写文件走 `EXPORT`（`BrowserFiles` 在 `begin_save` 内合成 `download://<name>`，`DomDownload` 用 `Blob`+`<a download>`），slim 构建只有 `.dxf` 写出器（默认文件名由 `default_save_name(...,"dxf")` 决定），`PLOT` = **SVG/PDF** 下载（仅契约测试）。**连接器裁剪（2026-11-22，见「连接器与导出矩阵」）**：STEP/IFC/RVT/DGN/SKP/OBJ/STL/PLY/OFF/glTF 导入与 STL/OBJ/PLY/OFF/IFC/SKP/glTF 导出在该 bundle 里被**编译掉**，按 `unsupported-runtime: import.<id>` / `export.<id>` 拒绝；两个 web 产物都是 slim（`dist/pkg` 与 Pages 站点实际加载的 `dist-ui/canvas`，后者由 `scripts/build-web-ui.sh` 在 Vite 之后 stage），slim 产物 11,680,561 字节（`opt-level="z"` + `wasm-opt -Oz`，2026-11-22 实测）。本机 Chromium+SwiftShader：smoke 8/8 (216/216)、端到端 `OPEN`→4 实体、`SAVE`→48,206 B DXF (`0 SECTION…EOF`) | CI `web-build`/`web-smoke`(SwiftShader)/`web-deploy` 已按 tag/dispatch 合同运行（v0.1.4 发布成功，线上 `yacr-examples.pages.dev` **是修复前 bundle**，需新 tag 才带上文件保存）；大图预算未达标（峰值由导入阶段决定）、Firefox/Safari/WebKit/移动端矩阵、GPU、像素、性能均 **NOT VERIFIED**；`PLOT` 无浏览器级证据；旧 Web（早期桌面 UI 时代）证据不得复用 |

### ymodel 原生文档格式（2026-11-18，ADR-0037）

`ymodel` 是本仓库的**原生**保存格式：zip 容器 + zstd 帧承载的分片 JSON + 保留的原始源字节。
DWG / DXF / STEP / SKP 等一律是连接器格式（打开/另存为边界）。保真等级：**L0** 源字节保留（`revision ==
manifest.open_revision` 且源字节仍在才同格式直通）、**L1** 已建模语义不动点、**L3** 视觉等同
（**永不承诺**）。

| 项 | 状态 | 证据 / 边界 |
|---|---|---|
| 文档模型与快照 | 已实现 | `cad-db` 323 测试；文档级不动点（`snapshot → hydrate → snapshot` 逐字段相等） |
| 容器读写 | 已实现 | `cad-store-zip` 契约测试（本轮见 `docs/validation-ymodel-zstd-2026-11-22.md`）：JSON 条目是 zip `Stored` 承载的标准 zstd 帧（`structured-zstd =0.4.3`，level 6），`textures`/`resources` 为 `Stored` raw；manifest 记 `codec`/`len`/`len_c`/`sha256`/`sha256_c`/CRC；版本门禁只接受 `read == 2`（旧归档 `unsupported_version`）、更高 `minor` 拒绝、foreign entry 原样保留、未变分片按压缩字节 raw-copy、两次保存字节相同、超预算 abort |
| 增量保存（Phase 2 dirty-shard raw-copy） | 已实现（库层 + 宿主接线） | 宿主累积每个 commit 的 `ChangeScope`（命令、interaction、plugin commit、engine funnel）；保存以文档**自身**的上一个归档为 base：未触及分片 raw-copy、脏分片重编码、删除分片不复活、header/classes/models/tables 始终重写；打开/New/Close 与成功保存后清空累积范围。`cad-store-zip/tests/raw_copy.rs` 6 项 + `cad-app` dirty-scope 契约 4 项 + `cad-host/tests/ymodel.rs` 的保存范围断言。**边界**：`sha256_c`/`len_c` 由 ADR-0046 写入并在解压前校验；复制条目继承上一归档的完整记录（含真实 `sha256_c`）；**host FakeArchive → 真实 store 的端到端 narrowed-save 与真实大文件性能 NOT RUN**；X11 dom-canvas / Web / Android / Windows / macOS 的归档保存仍 `unsupported-runtime: ymodel` |
| 文件级往返 | 已实现 | `cad-store-zip/tests/document_fixed_point.rs` 7 项（含 layout 与 block 序列相同的碰撞回归）；`apps/app-linux/tests/ymodel_round_trip.rs` 走真实适配器 |
| 原生打开/保存（Linux） | 已接线 | `cad-host` 的 `ymodel` 契约 11 项（全 crate 398 测试内）：安装文档、缺适配器拒绝、损坏归档不破坏当前文档、保存推进 `saved_revision`、外部修改拒绝、保存目的按源格式选择、第二次保存只携带增量范围、Save As 保留源归档与范围。保存前把上一版留在 `<name>.bak`（硬链接，失败回退复制；首存不建；配置写入不建），失败只告警不阻断——重命名本身仍是原子的 |
| 自动恢复快照 | 已接线（仅 Linux 载体） | 脏文档**默认每 60 s**（选项里可改，0 = 关闭）把**同一份** `.ymodel` 写到 `<config>/recovery/<key>.ymodel`（+ 同名 `.json` 记录），从不触碰文档本身；成功保存后丢弃；安装文档时 `record.revision <= saved_revision` 判为过期并丢弃；只有严格更新才通过 `cad_status.error` 的一次性横幅告知（`recovery.available`，指向可用 `OPEN` 打开的恢复文件）。13 项 host 契约 + 8 项 Linux 存储 + 2 项 wire + 2 项命令接缝 + 7 项前端测试；单快照上限 512 MiB。**欢迎页**：无 `--open` 启动时进入无文档状态（`StartDocument::Empty`），恢复快照与最近文件一起列在开始页，点击按**key**打开（UI 不接触路径）。**边界**：`File > Revert` 仍未做；Web/Android 无恢复快照；真实崩溃恢复、横幅在真实窗口中的呈现 **NOT RUN** |
| Windows / macOS | **已接线（运行时 NOT VERIFIED）** | `apps/app-{windows,macos}/src/ymodel_archive.rs` + 原生保存对话框（COM `IFileSaveDialog` / `NSSavePanel`）+ 临时文件→原子替换 + `.bak` + 载体恢复目录；Win32/AppKit 分支只做过类型检查（`x86_64-pc-windows-msvc` / `aarch64-apple-darwin`） |
| Web | **NOT IMPLEMENTED** | 无归档适配器，显式 `unsupported-runtime: ymodel`（禁止静默改导出别的格式）；写文件走 `EXPORT` 的浏览器下载 |
| Android | **已接线（设备 NOT VERIFIED）** | `apps/app-android/src-tauri/src/ymodel_archive.rs` + SAF 读/写 + `filesDir/recovery`；写入先落缓存再灌入 URI（**非原子**），`.bak` 尽力而为，缺失时 `save.no_backup`；恢复回读走 `file://` + `ContentResolver`，设备路径未跑 |
| L0 源字节直通 | **未实现** | `raw/source-*.bin` 尚未写入；需要连接器保留源字节 |
| L1 不动点（DWG/DXF → ymodel → 原格式） | **未验证** | 需要真实 DWG 语料与连接器往返测试；`$INSUNITS=5` 的 100× 问题已修（见下） |
| 真实窗口内保存/重开 | **NOT RUN** | 只有库层与 carrier 契约证据；无 agent 侧 GUI 操作 |
| 性能（打开 / 保存 / 峰值磁盘） | **NOT RUN** | ADR-0037 D17 的门槛数字未测量 |

已知格式限制（记录在 ADR，不是待办猜测）：ADR-0046 起 JSON 条目在 zip 内是标准 zstd 帧，
**不是**可直接 `unzip -p` 读的明文；外部检查需 `unzip -p … | zstd -d`（manifest 仍是明文）。
`sha256_c`/`len_c` 已写入并在解压前校验，不再有"压缩字节哈希不可计算"的限制。
`tables/*.json` 的行表（图层/块/样式/线型/比例/布局/页面设置/xref）是 v1 的**显式存储偏差**，
对象模型里它们仍是 DefinitionElement。

### 插件管理器载体矩阵（2026-11-05）

插件管理器管理离线厂商解码器（`tch` + 用户安装），**不是**第二份 CAD 文档权威。所有行都是
契约/源码级判定，除 Linux 契约测试外没有运行证据。

| 载体 | `surfaceMode` | 插件管理器 | 拒绝 / 存储 |
|---|---|---|---|
| Linux 原生 Wayland | `native-below` | **支持** | `$XDG_CONFIG_HOME/yacr`（或 `~/.config/yacr`）的 `plugins.json` + `plugins/<id>/` |
| Linux X11/XWayland | `dom-canvas` | **不支持** | `supported:false` + `plugin.unsupported_carrier: dom-canvas`；mutations `accepted:false`；内置 `tch` 仍 always-on |
| Windows | `native-below` | **契约支持** | `%APPDATA%\yacr\plugins.json` + `plugins/<id>/`；`APPDATA` 缺失时 `NonPersistentPluginStore`（安装/写入拒绝） |
| macOS | `native-below` | **契约支持** | `$HOME/Library/Application Support/yacr/plugins.json` + `plugins/<id>/`；`HOME` 缺失时同上 |
| Android | `native-below` | **契约支持** | 应用私有 `filesDir` 下 `plugins.json` + `plugins/<id>/`；`nativeAssetsReady` 前 `unsupported-runtime: plugin-store-unavailable` |
| Web | `dom-canvas` | **不支持（无持久存储）** | adapter 按名拒绝 `unsupported-runtime: cad_plugin_*`；内置 `tch` 仍随 bundle 提供 |

插件改动**立即持久化、下次启动生效**（`restartRequired = activeDecoder != sessionActiveDecoder`）；
没有热重载/热卸载。当前证据只有 `native-below` 的 Linux 契约测试与共享层静态接线；
Windows/macOS/Android 真机运行、真实 GUI/浏览器、GPU/像素均 **NOT VERIFIED**，见
`docs/vendor-plugin.md` §8 与 `docs/adr/0029-plugin-manager.md`。

- 平台 I/O（文件、配置、表面度量、文件选择器）只存在于 `app-*`；共享层不含平台类型。
- **单一 React UI**：`apps/desktop-ui`（两个桌面宿主共用）。DOM 侧“测试”是 node:test 的源码
  结构/纯函数契约，**不是**运行时 DOM 挂载，也**不是**像素/视觉验收。
- Windows 的判定上限复述：**实现 + Tier 0 + Wine 拒绝 smoke**，视觉由用户真 Windows 自测；
  **不得**写“Windows 已验收”。


## GPU 设备能力协商（2026-10-17，政策见 `docs/adr/0013-device-capability-negotiation.md`）

**本表由设备/适配器探测产出，不是按 backend 类别推断**：`Renderer::caps_for` 自 2026-11-06 起
委托纯函数 `plan::caps_from(backend, wasm_target, limits)`——backend 只决定**标签**
（`webgpu`/`webgl2`/`vulkan`/`metal`/`dx12`/`opengl`），`compute` 与 `storage_buffers` 由
`device.limits()` **推导**（`max_storage_buffers_per_shader_stage > 0 && max_storage_buffer_binding_size > 0`；
`max_compute_workgroup_size_x > 0 && max_compute_invocations_per_workgroup > 0`），
`indirect_draw` 仍按后端（仅 WebGL2 为 `false`，`wgpu::Limits` 无对应字段）。此前的
“WebGl2 之外一律 `true`”兜底已删除，因此降级/低配的 native 或 WebGPU 适配器不会再被**报告**成
能力齐全。每格的"本机探测"为**实测**；未在本机跑过的平台一律
`未测`，不得据规范保证表填 `Native`。三态 `Native` / `Emulated` / `Unsupported` 描述**能力**，
**不是**实现进度；feature 存在也**不是**性能结论（铁律 6），本机仍 `gpu_ms=None`。

| 能力 | 类别 | 本机探测（实测） | 三态 | 说明 |
|---|---|---|---|---|
| `MULTI_DRAW_INDIRECT_COUNT` | runtime-optional | Vulkan/P620 适配器 `true`；同卡 **GL 后端**适配器 `false` | `Native`（Vulkan，**未启用**）／`Emulated`（GL：wgpu 展开为一串 `draw_indirect`）／`Unsupported`（WebGL2/GLES 无 `INDIRECT_EXECUTION`，`未测`运行） | 未请求；indirect 路径**未实现**（backlog N5） |
| `INDIRECT_FIRST_INSTANCE` | runtime-optional | Vulkan/P620 适配器 `true` | `Native`（**未启用**） | 随 N5 一并请求 |
| `DownlevelFlags::INDIRECT_EXECUTION` | runtime-optional | Vulkan/P620 `true` | `Native`／WebGL2 `Unsupported`（构造上） | downlevel flag，不随请求变化 |
| `TIMESTAMP_QUERY`（含 inside encoders/passes） | **measurement-only** | Vulkan/P620 适配器 `true` | `Native`（**未启用**） | 仅测量门控 opt-in；不得成为运行时依赖；未请求时 `gpu_ms=None` |
| 设备 limits（`max_buffer_size`） | limits | adapter `max_buffer_size` **4,292,870,144 B** | — | **requested（policy，实测）**：**4,292,870,144 B**（`negotiated_device_limits`，`crates/cad-render-wgpu/src/headless.rs:232`）／**granted（device，实测）**：**4,292,870,144 B**、**`degraded=false reason=-`**。实测后果：`complex-test.dwg` 的 pulled 重建（需 2,281,858,648 B）现为 **`pulled=active`**（改动前为 `skipped-insufficient-vram`）。降级面 = `request_device` 失败后用 `Limits::default()` 重试并报 `degraded=true reason=…` |
| 设备 limits（`max_storage_buffer_binding_size`） | limits | adapter **2,147,483,644 B** | — | **requested（policy，实测）**：**134,217,728 B（WebGPU 默认，刻意不抬）**／**granted（device，实测）**：134,217,728 B。理由（实测，非风格）：抬到 adapter 值会让 pulled 分段器产出约 2 GiB 单次分配，`create_buffer` 失败返回**无效 handle**，后续使用报 **validation** 错误（`ErrorFilter::OutOfMemory` 捕获不到）⇒ abort exit 101；锁回默认后同一重建成功且**像素不变**。详见 ADR 0013「实现状态」 |
| `pulled` 容量/分配判定（`PulledState`） | limits | 本机两 fixture 均 **`pulled=active`**（实测） | — | 显式枚举（`crates/cad-render-wgpu/src/lib.rs`）：`active`／`skipped-insufficient-vram(required_bytes,budget_bytes)`／`skipped-out-of-memory(required_bytes,budget_bytes,message)`／**`skipped-allocation-failed(required_bytes,budget_bytes,message)`**／`skipped-unsegmentable`／`skipped-no-converted-geometry`／`unavailable`。`skipped-allocation-failed` **不**自称显存耗尽证据（设备给的是非 OOM 错误）；三条 skip 路径都必须保留通用 per-run 回退且像素一致 |

- **feature 能力位仍为空，但 limits 已协商（2026-10-17）**：两处设备创建点
  （`headless.rs`、`native_presenter.rs`）**仍未请求任何 `Features`**；变化只发生在 limits——
  旧的“两处都用 `DeviceDescriptor { ..Default::default() }`、零请求”已**仅对 feature 成立**。
  实现状态见 `docs/adr/0013-device-capability-negotiation.md`「实现状态（2026-10-17）」。`pulled=active`
  是**本机两个 fixture** 的实测，不代表任意图纸都能走 pulled（更大 payload 仍会命中 `skipped-insufficient-vram`）。
- **limits 行的三列（requested / granted / degraded reason）必须同时存在**：只写 granted
  会掩盖“这是请求限制而非硬件上限”（P620 的 adapter 上限是 4 GiB 量级）。
- **未测**：Windows/macOS/Android、真实浏览器 WebGL2、其它 vendor/驱动、任何 GPU 时间结论。
- **WebGPU 只有显式入口，默认仍是 WebGL2（2026-11-06）**：`CanvasPresenter::create_with_preference`
  的 `BackendPreference::WebGpu` 是唯一请求 `Backends::BROWSER_WEBGPU`（经
  `wgpu::util::new_instance_with_webgpu_detection`）的值；没有 `navigator.gpu` 时**拒绝**
  （`yacr: unsupported-runtime: webgpu`）而不是回退。宿主侧（`apps/app-web`）**尚未**提供选择入口，
  两个已测引擎也都没有 `navigator.gpu`，因此该分支**未在浏览器中运行过**（NOT VERIFIED）。

### 投影精度 / RTE（2026-11-06 标注为 `Partial`）

大坐标防抖在 CPU 侧按 batch 的 `f64` 局部原点先做减法、再窄化到 `f32`
（`crates/cad-render-wgpu/src/plan/projection.rs`，`FrameProjection::batch_matrix_f64`）：

| 投影 | 状态 | 事实 |
|---|---|---|
| Ortho | 实现 | 原点与相机中心保持 `f64` 直到减法完成后才窄化，这是 2D 主视图路径 |
| Perspective | **`Partial`** | `FrameProjection::Perspective` 在 `translate_left` **之前**把原点窄化为 `f32`（`projection.rs:203-208`，源码注释已声明），因此 Ortho 那条 `f64` 相机相对减法在 3D 预览路径上**不适用**；km 级图纸的透视预览会出现可见偏移 |

**为什么不修**：CAD 的权威视图是 Ortho，Perspective 只服务 3D 预览；修它要把 `f64` 原点以
hi/lo 两个 `f32` 传进 shader，或每 batch 在 `f64` 里烘矩阵，属于另立提案（需自己的证据）。
**未测**：透视路径的像素/几何误差量化 —— NOT RUN。

## 已明确的不支持项

字体/图标等资源授权未核查；TTF/OTF/WOFF 与 SHX（shapes/unifont/bigfont）文本已整形为
线段绘制，缺失字体按回退链替代（源码见 `docs/fonts.md`）。**2026-10-21 起回退链按图纸的
`$DWGCODEPAGE` 选择**（936/GBK→`zh-Hans`、950/Big5→`zh-Hant`、932→`ja`、949→`ko`，其余拉丁）：
中文候选 `gbcbig/hztxt/chineset/bigfont/simhei/simsun` 置前，拉丁顺序不变；换图时就地重定向。
**`BigFont` 由 `NotImplemented` 改为 `Partial`**（只有在字体文件存在且声明 `encoding` 时才能作为
回退面使用；仍无 primary+bigfont 配对、无按代码页选择样式自带的 bigfont）。仓库 `fonts/` **不含
任何 CJK SHX**，因此本机打开中文图纸会**明确报缺**（每文档一条 `font.locale_fallback_missing`
并列出候选），CJK 字符按 `text.glyph_missing` 缺字形，**不伪装成渲染成功**；TEXT 对齐、复杂文字整形、
动态块参数/夹点求值、复杂视口裁剪、代理无缓存几何均按样本标记为未支持或未验证，
不会报告为完整图纸。出图仅光栅 PNG（无矢量 PDF/HPGL/CTB）；线宽的复杂/嵌入形状段
仍按显式 `Partial`/未实现报告，不计入完成。动态块**可见性状态**读取/切换与
**注释性缩放（TEXT/MTEXT）**已实现，但动态块参数/夹点编辑、非文本注释性类型仍为
显式 `Partial`，且宿主比例切换 UI 未接线。


2026-10-21/22 批次新增的**有界**能力（各自带未完成边界，勿扩大解读）：
**命令行回显** `cad_status.commandLog`（有界 200、`sequence` 单调、换图清空；条目**无任何人类可读
文本字段**，只有 `command`/`code`/`measurement`，文案全部来自 i18n；回显 UI 以 4 位小数显示
Distance/Angle/Delta X/Y/Z，点不足两个时只显示距离，`unit:null` 不加单位后缀，无"清空"按钮因为
宿主没有该 API）；**属性面板读侧**（`group` 分组、多选取**共同键交集**、`mixed`、`editable`、
`kind`、`options` 取图纸真实图层/线型表名，上限 256）与**写侧** `SETENTITYPROPERTY`（一次事务/
一步撤销；`layer`、`color`=ByLayer/ByBlock/`aci:0..=256`/`#RRGGBB`、`linetype`、`lineweight`
= 规范毫米列表、`transparency`=0..90 百分比→不透明度；非直接可寻址引用拒绝；无变化不产生撤销步）；
**选项设置** **20** 个类型化键（grid/axes/selectionHighlight/snapHints/plotFormat/snapEnabled/
hoverPreview/gridSnapEnabled/orthoEnabled/snapSpacing/backgroundModel/backgroundPaper/
crosshairSize/pickboxSize/apertureSize/gpuBudgetPercent/wgpu_backend/leftDragPans/
orbitSensitivityPercent/msaaMode，另有 `theme`/`navbarVisible`/`navvcubeDisplay`/`viewCubeIn2d`/
`viewCubeIn3d` 五个根级叶子见导航一节），持久化到
`preference.json`、**实时生效**并在 open/替换后重新播种（七个 `interaction.*` 叶子：对象捕捉、悬停预览、
`SNAP`、`ORTHO`、`SNAPUNIT` 间距、左键拖动平移、3D 旋转灵敏度倍率）；**语言默认**按 `显式选择 > 宿主注入 > navigator > 英文`
（`zh-TW/zh-Hant` 无对应目录 ⇒ **英文**，不静默给简体）；**显示**：模型/图纸背景色 `#rrggbb`
（sRGB→linear 在配置→渲染器接缝，真机核对 0.5 读回 188、遮罩内部同值）与十字光标 1..100%
（默认 5、以光标为中心对称、按视图尺寸计算；100% 位级不变），均**即时**生效。

**MSAA（2026-11-22，ADR-0047；离屏 lavapipe 证据，真机 NOT VERIFIED）**：显示选项新增
`msaaMode`（`off`/`2x`/`4x`，默认 `4x`，持久化 `view.msaa.mode`）。MSAA 只在 3D 视图
作用于持久 base 通道；2D 视图固定 1×（保持解析线条像素），presented overlay 与 wipeout
mask 始终 1×（否则 mask 边界会透出被遮几何），pick 也保持 1×。`n==1` 保持原 copy 路径；`n>1` 由 base pass 直接 resolve 到 presented
单采样目标。运行时切换会重建 base 管线并重新分配附件，不清空场景。渲染器按适配器支持集
下调并以 `render.msaa.unsupported_samples` 诊断记录一次；WebGL2 的 `CanvasPresenter`
显式报告 `[1]`，因此 Web 当前不启用 MSAA。证据见 `docs/validation-msaa-2026-11-22.md`。

**输入模态（2026-11-01，源码/契约测试证据；设备 NOT RUN）**：React 输入层按每个
`PointerEvent.pointerType` 在指针事件上附带可选 `device: "mouse" | "touch" | "pen"`（缺失/未知 ⇒ mouse；
逐事件判定）。默认手势：mouse 中键拖拽 = 平移、2D 主键拖拽 = 框选、滚轮 = 缩放；touch 单指拖拽 = 平移、
tap = 选择/取点、双指 `pinch` = 以中点缩放（含中点平移），**touch 从不框选**；pen 同 mouse。`pinch` 是连续
输入 `{x,y,dx,dy,scale}`，宿主任一时刻只取消活动工具并平移/缩放，不框选、不提交捕获；触控板 pinch 仍是
Ctrl+滚轮。`interaction.leftDragPans`（默认 false，Options → Drafting「左键拖动平移」）开启后 mouse/pen 的
2D 主键拖拽改为平移；仲裁在共享 `cad-host`，原生与 wasm 载体一致。**3D 视角拖拽**：3D 下 mouse/pen
主键拖拽与 touch 单指拖拽都是 orbit（`yaw = -dx·K`、`pitch = +dy·K`），速率
`K = 0.0035 rad/逻辑像素 × orbitSensitivityPercent/100`（`interaction.orbitSensitivityPercent`,
25–200 闭区间，默认 100；宿主每次事件读 effective config，鼠标与触控共用同一倍率；方位球拖拽经
`cad_status.navigation.orbitSensitivityPercent` 用同一倍率，旧宿主缺该字段按 100）。**上限**：Web 的 Options 可用但只在会话内
持久；Android 的 Options 可用且偏好写入应用 `filesDir`（`preference.json`，设备落盘 NOT VERIFIED），此前
「Android 无弹窗宿主/配置不持久」的记载已被 2026-11-01 的弹窗与 `AndroidConfigStore` 一轮取代。真实触摸硬件、
手势与设备运行 **NOT VERIFIED / NOT RUN**。

**属性面板（2026-10-22）**：按 `entity/geometry/text/display` 分组；多选显示**共同键交集**并以
`properties.mixed` 占位（写入作用于**整个选中集**）；`layer/color/linetype/lineweight/transparency`
五行可编辑（`enum`/`number`/`text` 编辑器按 `kind` 选，值语法与写侧一致），提交走
`cad_set_selected_property`（本地暂存 → Enter/失焦/下拉即提交，拒绝则回滚并显示 i18n 文案、
code 只进 `title`）；`Ctrl+1` 与命令行 `PROPERTIES` 均可开关（Chrome 本地命令，**不进** Rust 路由）。
**2026-11-22 自适应宽度与紧凑布局**：桌面打开宽度 = `clamp(round(绘图区宽 × 0.38), 300, min(560, 绘图区宽))`，
拖拽/键盘上限 = `min(绘图区宽, 720)`（下限仍 280；永不超出实测绘图区），workspace preset 范围同步放宽到
[280, 720]；宽度仍是 `PaletteWindow` 的 absolute overlay 内联 style，不改 CAD 分区矩形。紧凑布局
（≤1024px）不再拒绝属性面板：`PROPERTIES`/`QUICKPROPERTIES`、`Ctrl+1` 与抽屉入口打开**全宽底部抽屉**
（`MobilePropertiesSheet`，同一个 `PropertiesDock`/编辑状态机，`variant="floating"`），backdrop 或头部
关闭按钮回到同一个 `propertiesOpen`；图层面板与其它 docked palette 仍按 `mobile.refusal.docked_panel`
拒绝。运行时、真机、像素 **NOT VERIFIED**（仅契约测试 + Vite build）。

**选项弹窗（2026-10-22）**：独立弹窗（ADR-0021），五个页签、12 个写键 + 语言 + 只读字体目录；
Apply/OK/Cancel 为 AutoCAD 语义（本地暂存、只发 diff、写失败标字段）；入口三处（File 页、
`OPTIONS` 命令行拦截、状态栏设置按钮）。**拾取框/孔径**：`view.pickbox.sizePixels`(1..=50, 默认 3；
AutoCAD 的 0＝关闭拾取框/选择预览未建模，故拒绝 0)
与 `view.aperture.sizePixels`(1..=50, 默认 12) 已接线到 hover/拾取/光标/捕捉四条路径，实时生效。

**GUI 验收（2026-10-22，有界）**：真实 GPU（Vulkan + Quadro P620）下已实拍主界面、属性面板
（对象/几何/显示 三组 + 可编辑控件 + 单选行值）、命令行 `DIST` 回显（距离/角度/X·Y·Z 增量）、
选项弹窗（720×560 五页签）——证据驱动为 `YACR_PROBE_UI_DIALOGS`（**触发非真鼠标**，下游为
生产路径）。**仍未做（明确不声称）**：弹窗内交互与属性编辑提交/拒绝的 GUI 实测（只有模型+源码测试）；真实 GUI/视觉验收的完整覆盖面（本批**没有**截图或像素判据，选项弹窗与属性面板的
真实窗口行为仅由纯函数模型 + 源码扫描 + 宿主契约测试覆盖，语言的双窗口传播标为 **NOT VERIFIED**）；
自动保存/.bak/xref/CTB/GPU 选择/快捷键自定义/夹点/
极轴/动态 UCS（AutoCAD Options 中存在但本仓库**无行为可接**，因此不显示）。（**颜色色板选择器已于
2026-10-26 补上**，见下方「颜色选择器」条。）

**2026-11-08 补齐**：生产宿主默认启动**空白 `untitled` 图纸**（`HostOptions::start_blank=true`；不再加载 demo 几何），`--open`/位置参数仍优先；`CLOSE` 成为宿主自有命令，关闭当前唯一文档并回到开始页（无未保存提示——项目没有 dirty 模型；多文档/`CLOSEALL` 仍不支持）。开始页与 About 改为跨平台开源 CAD 介绍，取消标签页 `+`（多标签未实现）。

**2026-11-18 更正**：生产载体改为 `HostOptions::document_at_start = StartDocument::Empty`——启动**没有文档**（与 `CLOSE` 后同一状态），用户看到的是开始页；`StartDocument::Blank` 保留「启动即空白 `untitled` 图」语义（需要文档的测试/夹具显式选择），`StartDocument::Demo` 仍是测试与 headless 夹具的合成 demo。配置了 `--open` 时启动打开照常运行。

**2026-10-25 补齐（React 宿主，源码/单测证据；真机像素 NOT RUN）**：`NEW`/`SAVE`/`PLOT` 及其别名
经宿主能力布尔（`canNew`/`canSave`/`canPlot`）真实准入，Linux 保存目标走 XDG portal `save_file`；
`GRID`（F7，仅显示栅格）、`SNAP`（F9，会话级栅格捕捉，间距由 `SNAPUNIT <value>` 设置）与
`ORTHO`（F8，仅 XY、上一点水平/垂直约束）成为真实命令；图层面板
（`LAYER`）支持搜索 / 临时显隐 / 恢复 / 设为当前层，并新增**图层表结构写**
（`CREATELAYER` / `RENAMELAYER` / `DELETELAYER`，`NEWLAYER` 是项目同义词别名）：内联新建、
行内重命名、按行删除确认（默认层 `0` 不提供入口），写经**既有**有界队列 ack 后重读。
图层写**不可撤销**（历史只有实体 patch），也**不写回原 DWG**（`SAVE` 是 DXF 或实验性新 DWG 导出，不覆盖源）。

> **授权口径（2026-10-26 补齐，此前缺失）**：F03 只要求图层**临时显隐 / 搜索 / 恢复**，并明确
> 「不修改 DWG 图层表」。已落地的 `CREATELAYER`/`RENAMELAYER`/`DELETELAYER` 与
> `SETLAYERPROPERTY` 的**存储型**键（`linetype`/`lineweight`/`plottable`）**超出 F03**，属
> **项目自选范围（我们的选择）**；它们只改内存文档与会话覆盖，**不写 DWG**、**不可撤销**。
> 会话级显隐覆盖（`cad_set_layer_visible` / 恢复）仍在 F03 内，且**不使底图变脏、不升 revision**。

**2026-10-26 新增 `LAYISO` / `LAYUNISO`（AutoCAD 图层隔离）**：`LAYISO` 读当前选择 → keep 集 =
**所选实体的图层 ∪ 当前层**（**我们的规则**：与既有「当前层不可关闭」合同一致；当前层不在选择内时
给 `layers.isolate.current_layer` 诊断），其余图层经**既有原子批量路径** `set_layer_visibilities`
一次性置为隐藏（校验先行、无部分状态、无静默半成品）；只改**会话覆盖**，**不写 DWG、不升 revision、
不进历史**。全部图层都已在 keep 集 ⇒ `layers.isolate.noop` 且不产生覆盖抖动；空选择拒绝
（否则会把所有层藏掉，画布上无从恢复）。`LAYUNISO` 与 `LAYON` 解析到**既有** `RESTORELAYERS`
（清空全部覆盖）。**`LAYERP`（2026-10-27 起）不再是别名**：它有独立 `CommandId::LayerPrevious`，恢复**上一次**
图层状态——会话内保留**有界**（上限 **16** 条）的快照环，仅在真实改动前压栈。快照同时记录**会话覆盖**与
**层表属性的改动前行**（`off`/`frozen`/`locked`/colour/linetype/lineweight/transparency/plottable）；
覆盖改动来自 `ToggleLayer` / `SETLAYERS` 及其派生 `LAYISO`/`LAYOFF` / `RESTORELAYERS`，属性改动来自
`LAYFRZ`/`LAYLCK`/`LAYULK`/`LAYTHW`/`SETLAYERPROPERTY`/`-LAYER` 属性写与 `LAYERSTATE RESTORE`。
属性恢复经 `LayerPatch` **一条事务/一步撤销**（撤销/重做对称）；**纯覆盖步仍不写 DWG、不升 revision、
不进历史**。no-op 与 `LAYERP` 自身**不压栈**；历史空 ⇒ `layers.previous.empty` 且不改状态；快照命名的层行
已被删除 ⇒ `layers.previous.dangling_layer` 且**不写任何东西**、快照保留。快照**不持久化**（会话级）。
`DELETELAYER` 的覆盖清理**刻意**不是一步 `LAYERP` 状态。`LAYISO`/`LAYUNISO` 在 Viewer 模式**可用**（视图操作，不是实体写）。

**2026-10-26 新增 `LAYOFF` / `LAYON`**：`LAYOFF`（新 `CommandId`，payload-free，Viewer 可用）把当前
**选择所在图层**全部关闭（会话覆盖），**当前层除外**（与其他写路径的 `layer.current_off` 合同一致）；
选择只落在当前层 ⇒ `layers.off.current_layer` 且**不改变任何状态**；空选择 ⇒ `layers.off.needs_selection`。
与 `LAYISO` 共用同一原子批量覆盖助手，**不写 DWG、不升 revision、不进历史**。`LAYON` 是既有
`RESTORELAYERS` 的**别名**（不新增 `CommandId`），命令表因此为 **88**。

**2026-10-26 更正（图层属性存储/往返轮）**：图层的**颜色/线型/线宽/关闭/冻结/锁定/可打印/
透明度**现已**建模并随导出写入、可往返**（`LayerAttributes` 旁存于 `Layer` 行；
`docs/entity-style.md` 有存储形状与 `visible == !(off || frozen)` 不变量）。导出表也不再只带
“图层名 + off”：不再把“冻结”改写成“关闭”（re-import 后的**有效可见性**与旧行为一致），
并新增 `is_plottable`/颜色（ACI 或真彩）/`6` 线型名/`370` 线宽/`AcCmTransparency`；
plot-style **名字**无法写出（写入器只发句柄）→ 写缺省并记 `export.layer_plot_style_dropped`，
未定义线型名 → 写 `Continuous` 并记 `export.layer_linetype_substituted`（绝不发悬挂引用）。
**但（除锁定外）仍是零行为**：没有代码读冻结/可打印/plot style 去改变行为（**锁定**后来获得行为，
见下方「图层锁定」轮），**出图未实现**，
`ByLayer` 实体颜色**当时仍在导入时被拍平**成显式 sRGB（**同日随后改为按层解析**，见下方
「`ByLayer` 颜色已按层解析」），改图层颜色的效果随该轮一并生效；
面板色块仍只是名字哈希。图层**写**（新建/重命名/删除）不写 DWG、不可撤销的结论不变。
**2026-10-26 图层属性编辑轮（本轮）**：图层属性现在**可经命令/IPC 编辑**——
`SETLAYERPROPERTY`（命令）与 `cad_set_layer_property { layer, key, value }`（IPC），
一行一条属性、一个 revision bump、**不记录撤销**（历史仍只有实体 patch），并**不写 DWG**
（`SAVE` 是 DXF 或实验性新 DWG 导出，不覆盖源）。键与今天的真实语义（**逐条，勿过度承诺**）：

| 键 | 取值语法（`crates/cad-app/src/layer_property.rs` 是唯一语法） | 今天的语义 |
|---|---|---|
| `off` / `frozen` | `true` / `false` / `0` / `1` | **可见性键**：数据库在同一次变更里重算 `visible == !(off || frozen)`，因此关闭/冻结经**既有可见性路径**隐藏图层（实体不再进场景），两者都清掉才重新可见。**本仓库不为实体写路径新增冻结规则**（见下方锁定轮）；但把**当前图层**关闭/冻结会被拒绝（`layer.current_off` / `layer.current_frozen`，见「当前图层轮」） |
| `locked` | `true` / `false` / `0` / `1` | **实体写的行为门**（2026-10-26 锁定轮增补，见下）：锁定层上的实体**不可创建/修改/删除**，拒绝码 `layer.locked`；显示属性写与图层表写**仍允许**。锁定层**不可设为当前层**（`layer.current_locked`，见「当前图层轮」） |
| `plottable` | `true` / `false` / `0` / `1` | **只存储**：出图未实现，没有任何行为 |
| `color` | `aci:1..=255` / `#RRGGBB` / `rgb:#RRGGBB` / `r,g,b` | **有行为（仅显示）**：写入 `LayerColor{aci,rgb}`（`aci:` 同时写调色板 RGB，与实体颜色语法共用**同一张** ACI 表）。`ByLayer`/`ByBlock` 拒绝；真彩写 `aci=None`。**改图层颜色会在下一次展开时重着色该层的 `ByLayer` 实体**（2026-10-26 起表示层按层解析），**不写实体、不产生实体历史**；`Explicit` 实体颜色不受影响 |
| `linetype` | 图纸线型表里**存在**的名字（先精确后唯一大小写不敏感） | **只存储**；不存在的名字 `layer.linetype_missing`（绝不存悬挂引用）。`ByLayer`/`ByBlock` 拒绝 |
| `lineweight` | `byLayer` / `default` / **1/100 mm** `0..=211` | **只存储**；单位是 DXF `370`（`35` = 0.35 mm），**不是** `SETENTITYPROPERTY` 的毫米字符串 |
| `transparency` | `0..=100`（**不透明度百分比**，`0`=不透明） | **有行为（仅显示，2026-10-26 起）**：存 `1 − v/100`。`ByLayer` 实体在下一次展开时取该值作为 `DisplayFragment::alpha`（缺行 = 不透明），所以**改图层透明度会改变该层 `ByLayer` 实体的混合**，**不写实体、不产生实体历史**；`Explicit` 实体透明度不受影响。图层行自身不再有别的消费者 |
| `plotstyle` | — | **明确拒绝为未实现**（`layer.property.unsupported` 点名）：导出器只发 plot-style **句柄**，存下来的名字无法存活，接受即撒谎 |

`name` / `visible` 不是可写键（前者属 `RENAMELAYER`，后者由 `off||frozen` 派生）。
线缆（`cad_layers` 行）新增 `color{aci,rgb}` / `linetype` / `lineweight` / `transparency` /
`locked` / `frozen` / `off` / `plottable`：它们仍是**显示/编辑输入**（线缆只是把数据库状态投影给面板，
不参与判定；**锁定行为在数据库写路径上，读取线缆不改变任何行为**）；`dom-canvas` 载体
与其它图层写一样**显式拒绝**该 IPC。面板 UI 已接线：真实色块（`row.color.rgb`，缺省才回落名字哈希且标注）、`off`/`frozen`/`locked`/`plottable` chips（静止行最多两枚 + `+N`，其余在展开详情里），以及只读的线型/线宽/透明度；文案明确写道「关闭与冻结当前同走可见性路径、打印只存储」。

**2026-10-26 颜色选择器（属性/图层共用；源码 + 单测证据，运行时挂载与真机像素 NOT RUN）**：属性面板的
`color` 行与图层详情行的颜色单元现在共用一个 **in-DOM** 选择器
（`apps/desktop-ui/src/components/ColorChooser.tsx`、纯模型 `src/color/color-value.ts`），不再依赖
webview 自带颜色对话框。它提供且**只**提供写语法真正接受的东西：① 符号项——实体颜色给
`ByLayer`/`ByBlock`（`crates/cad-app/src/property_value` 的 `color` 语法两者都收；AutoCAD 显示的
`ByObject` 是 `ByBlock` 的别名，**不在**语法内，因此不提供该字面量），图层颜色**不提供任何符号项**
（`crates/cad-app/src/layer_property` 按名字拒绝 `ByLayer`/`ByBlock`，因为图层之上没有图层）；
② 标准索引色板 **ACI 1..=255** 的色块网格，点击写入 **`aci:N`**——与行投影同一拼写、同一解析器，
**没有新值形式**；色板逐项镜像 `crates/cad-app/src/property_value/aci.rs`，并由测试直接读该 Rust 源比对；
③ 属性行保留**原有文本输入**作为兜底（任何网格表达不了的语法仍在那里输入），图层行提供按该键语法归一化的
自由输入（`colorPayload`：`aci:1..=255` / `#RRGGBB`，其余不发送）。写入仍只走既有唯一通道：属性 =
`shell.applySelectedProperty`（**不** `invoke`），图层 = store 的 `setProperty` 单飞与读回；拒绝照旧进各自
既有错误行，**没有**新 toast。浮层行为沿用 App 既有 in-DOM 规则（`position: fixed` + `placeContextMenu`
翻转夹取、Escape 关闭并把焦点还给触发器、外部 `pointerdown` 关闭、window blur/resize/外部滚动关闭），
网格是**单一 tab 停靠点** + 方向键/Home/End 游标。证据边界：`tsc --noEmit`、`node --test`（含 ACI 表与
Rust 源逐项比对、双目录键/占位符、五条浮层行为与无障碍属性的源码契约）、`check-i18n.py`、
`check-file-size.py`；**未**做组件运行时挂载、真实窗口交互或像素证据，**不是** GUI 验收。

**2026-10-26 图层锁定轮（实体写的唯一行为门）**：`locked` 从「只存储」变成**唯一一处**执行的门，
执行点全部在 `crates/cad-db`（`crates/cad-db/src/drawing/lock.rs`），命令层**不写第二份检查**：

- **判据唯一**：`DrawingDatabase::ensure_change_allowed_on_locked_layer`。两个入口调用同一条判据：
  `DrawingTransaction::stage`（五个 stager 都经它，**被拒时什么都没 staging**，事务保持空）与
  `DrawingDatabase::apply_drawing_changes`（唯一验证写路径，撤销/重放也走它；整批先验证后变更，
  因此**不可能半应用**）。
- **拒绝语义（AutoCAD 子集，属我们的选择）**：锁定层上的实体**不可修改**（含移动/变换/改属性）、
  **不可删除**、**不可插入**（含把解锁层的实体改到锁定层上）；错误码 **`layer.locked`**，消息点名
  实体 id 与图层名/id 并提示先解锁。revision、实体表、图层表、撤销深度全部不变（不建撤销步）。
- **不拦（明确）**：① **显示属性写** `update_entity_display`（颜色/线型/线宽/透明度）——它是表示重建，
  不是图形编辑；判据只拒「有效变更」，而该路径 staging 的是**原样**实体行（AutoCAD 同样会重新生成
  锁定层，故不拦）；② **图层表写**（新建/重命名/删除/`SETLAYERPROPERTY`），它们也是解锁的出口；
  ③ 解锁后同一编辑立刻成功（锁状态无缓存）。
- **冻结仍只是可见性**：冻结/关闭层已不可见（`visible == !(off || frozen)`），因此进不了场景与选择漏斗；
  本轮**不**在写路径上新增第二条冻结规则（冻结前已选中的对象仍可被编辑——已知并记录，本轮不改）。
- **历史重放规则（已实现并测试）**：撤销/重做把记录的 `DrawingPatch` 重放经 `apply_drawing_changes`，
  所以**当前锁状态优先**——某步现在会写锁定层即被 `layer.locked` 拒绝，`cad-app` 把该步**放回**
  （`cancel_pending_drawing`），文档不变；不静默丢弃、也不静默越权；解锁后同一步可正常撤销。
- **当前层规则（2026-10-26 两轮，已实现）**：`SETACTIVELAYER` 拒绝把**锁定 / 冻结 / 关闭**层设为当前层
  （`layer.current_locked` / `layer.current_frozen` / `layer.current_off`，判定顺序 locked→frozen→off，
  消息点名图层），拒绝时活动图层、revision、撤销深度**全部不变**。规则唯一：
  `crates/cad-app/src/app_drawing/drawing_create.rs` 的 `Application::ensure_layer_can_be_current`；
  `cad-db` 里没有第二份检查，也没有第二套名字解析（命令载荷已是解析后的 `LayerId`）。
  **关闭层也被拒绝**是本仓库的选择、**不是** AutoCAD 结论：关闭层在这里不可见且创建漏斗不看可见性，
  允许它成为当前层只会画出看不见的几何。
- **当前层不可被隐藏（补漏，已实现）**：`SETLAYERPROPERTY` 对**当前图层**写 `off=true` / `frozen=true` 也拒绝，
  同样复用 `layer.current_off` / `layer.current_frozen`，且**先于任何写**（属性行、`Layer::visible`、revision、
  会话当前层与撤销深度不变）。写成 `false`（打开/解冻）、`locked` 与其它键仍允许；**非当前图层**的
  `off`/`frozen` 也仍允许（面板对其它行照常工作）。规则唯一：`app_drawing/layer_edit.rs` 的
  `ensure_current_layer_stays_visible`（只有会话知道谁是当前层）；`cad-db` 未加检查、API 与不变量未改。
  与上面的准入拒绝合起来，当前图层在「成为当前」与「成为之后」都不会不可见。
- **仍未实现（缺口）**：按锁定过滤选择（AutoCAD 可选中但不可改）未做。
- **`ByLayer` 颜色已按层解析（2026-10-26）**：导入**不再拍平** `ByLayer` 颜色（保留符号
  `EntityColor::ByLayer`），表示层 `provider/expand.rs` 在展开时取实体自身 `layer` 的
  `LayerAttributes::color.rgb`（缺行回落 `Default`，ACI 7 白）。因此 `SETLAYERPROPERTY color`
  **会**在下一次展开时重着色其 `ByLayer` 实体，且**不写实体、不产生实体历史**；导出把
  `ByLayer` 写作**符号值**（省略组 62 = DXF 的 256），不再拍平，原
  `export.note.bylayer_flattened` 已改名 `export.note.style_resolved` 并改说真话。
  块内 `ByLayer` 取实体**生效图层**：自身图层，或（自身为图层 `0` 时）包含 `INSERT` 的图层，
  逐层链式解析；AutoCAD「图层 0 继承块引用图层」自 2026-10-26 起**已建模**
  （`cad_db::LayerScope`，见下方同名条目），`ByBlock` 仍沿父链。
- **`ByLayer` 线型/线宽同样按层解析（同日）**：导入保留符号
  `EntityLineWeight::ByLayer` / `EntityLineType::ByLayer`；表示层按图层行线宽（1/100 mm 精确换算、
  `Default` → 0.25 mm、图层行自身 `ByLayer` → 0.25 mm 且置未解析标记）与线型**名字**去图纸线型表
  （大小写不敏感）取 pattern（名字缺失 → 连续线 + 未解析标记，**不**编造虚线）解析。三者改层即生效、
  **不写实体**。导出三者**全部符号化**：组 62 省略（=256）、组 6 = `BYLAYER`、组 370 省略（=-1）。
  **实体线型比例（组 48）不再是缺口**（2026-10-26 收尾）：因子改由
  `EntityRenderAttributes::linetype_scale` 携带，**与线型变体无关**，符号 `ByLayer`/`ByBlock` 也保留源值；
  表示层把它乘进已解析 pattern（`ByLayer` + `2.0` 与显式 `scale = 2.0` 渲染相同，`$LTSCALE` 行为不变），
  导出对符号线型也写回组 48。显式变体内的 `scale` 仍是显式路径的权威值，逐位不变。
- **`ByLayer` 透明度同样按层解析（2026-10-26，`by-layer` 家族最后一项）**：`EntityTransparency`
  新增 **`ByLayer`** 并成为 **`#[default]`**（新建实体继承图层，而不是把“不透明”烘进实体）；
  导入**不再拍平** `ByLayer`（此前把图层 `opacity` 复制成 `Explicit`，`layer_transparency` 旁表
  因此**删除**），`ByBlock` 仍符号、`Explicit` 仍按 `1 − byte/255` 逐位不变。表示层
  `resolve_alpha` 在展开时取实体自身 `layer` 的 `LayerAttributes::opacity`（缺行 = `Default` 的
  `1.0`），且**不在表示层钳制**——`cad_scene::sanitize_alpha`（非有限 → 不透明，否则钳 `[0,1]`）
  是显式与 `ByLayer` 共用的唯一消毒点。`SETLAYERPROPERTY transparency` 因此**会**在下一次展开时
  改变其 `ByLayer` 实体的混合，**不写实体、不产生实体历史**。块内规则与其它三项相同：`ByLayer`
  按**自身图层**解析，只有 `ByBlock` 沿父链（模型根无父 = 不透明）。导出也**符号化**：
  组 440 省略即 DXF 的“随层”（acadrust 只在非 `ByLayer` 时写 440，图层表用
  `AcCmTransparency` XDATA 携带值），`export.note.style_resolved` 已改为点名
  **colour / linetype / lineweight / transparency 四项**。
- **「图层 0 继承块引用图层」已建模（2026-10-26，曾记为未建模）**：AutoCAD 的规则是**块定义内**
  实体的自身图层为 `0` 时，按**包含它的 `INSERT` 的图层**绘制；其它图层保持自身。实现是
  **单点解析** `cad_db::LayerScope`（`crates/cad-db/src/drawing/layer_inherit.rs`），由
  `DrawingDatabase::root_layer_scope()` 每次遍历解析**一次**（按**名字**找 `0` 行，**不是**
  `LayerId(0)`：id 是表索引）并以 `child_scope` 逐层下传，因此：
  - 四项 `ByLayer`（颜色/透明度/线宽/线型）**在同一处**取生效图层
    （`crates/cad-representation/src/provider/expand.rs`），不可能互相不一致；
  - **规则逐层链式**：自身在 `0` 的 `INSERT` 把它自己的生效图层交给子实体，所以「`0` 实体 ⊂ `0` 块参照 ⊂
    RED 块参照」⇒ RED（一次递归解一层，不回到根）；
  - `ByBlock` 语义**未变**（仍取包含 `INSERT` 的已解析值），不会与图层规则**叠加**；
  - 图纸里**没有**名为 `0` 的图层时规则**整体关闭**（`layer_zero_id()` 为 `None`，不 panic、不猜）；
  - **绝不改写** `DbEntity::layer`：属性面板、导出与往返仍读到源图纸存储的事实，这是纯**显示解析**规则。
  **可见性/拾取同源**：`cad_app::picking::{ref_visibility, ref_effective_layer}` 用同一条
  `LayerScope` 链判定 instance path 每一步的生效图层，`SELECTALL`、框选（`SELECTINBOX`）、拾取
  （`ref_is_visible`）、`STRETCH` 的可见项分组与 `LAYISO`/`LAYOFF` 的「选中实体属于哪一层」
  都调用它——所以「图层 `0` 关闭」不再隐藏一个画在可见 `INSERT` 图层上的块成员，隐藏 `INSERT`
  的图层也会一并隐藏它。**仍存在的既有分歧（未修）**：模型空间场景只为**顶层实体**做图层过滤
  （`render_scene/assembly.rs` 走 `visible_model_entities`），块内成员**完全不做**图层过滤，
  因此「把 `INSERT` 的图层关掉」目前只影响拾取/选择，**不**让屏幕上的块成员消失。

CAD 分区右键**快捷菜单**已实现（复制/剪切/粘贴/全选/删除/特性/缩放到范围/平移/放弃/重做/重复上一命令）。
夹点已有**纯模型 + 命令 + 宿主拖拽**（`GripStretch`：`LINE` 端点与中点 / `CIRCLE` 圆心与四象限 / `ARC` 圆心 / `POINT` 位置，
一条事务一步撤销，拒绝时不建事务）；`native-below` 宿主已有 marker 与 arm 路径，`dom-canvas` 载体
（X11/XWayland、web）仍无夹点路径。**仍不实现**：弧端点、椭圆/样条/多段线顶点、块成员、多夹点/多实体拉伸、
拖拽中捕捉、3D 夹点编辑。
仍无行为可接而**不显示**的 AutoCAD 项：极轴、动态 UCS。

2026-10-10 图元显示补全轮新增边界：**光栅图像** PNG/JPEG 已解码并出纹理，但
TIFF/CCITT/EPS 等编解码器、亮度/对比/淡出/透明、outside/mask 裁剪显式未支持；
Web/Android 宿主未接图像加载器；**`draw_order` 已贯通到场景批次**（含 INSERT 继承、合批不跨顺序），
但**渲染器仍按上传顺序绘制、遮罩恒最后**（opaque mesh 还跨批次拍平，分段交错未实现）；
**WIPEOUT 掩码已实现**（非 `inverted` 与 `inverted` 都渲染；不与更高 `draw_order` 交错、矢量出图按页面背景 flood，`inverted` 为多环 even-odd 补集）。**外部内容** PDF/DWF/DGN
underlay、OLE2 嵌入对象、CoordinationModel/Navisworks NWD、**超出 ACIS lift 子集的**
`3DSOLID`/`REGION`/`BODY`/`SURFACE`（可 lift 的平面/解析曲面子集**已绘制**，见上文「ACIS 实体显示状态」；
其余仍是 `Opaque` + 内核具名码）、点云点数据（acadrust 0.6.8 仍只暴露元数据/范围，未暴露点数据）均不加载/不实现，显式
`Partial`/`Unsupported`，不计入完成。代理 opcode 厂商记录仍按用户要求排除（见
`docs/proxy-support.md`）。图元逐项状态见 `docs/dxf-entity-coverage.md`。

### 2026-11-07 度量/查询补充

| 命令 | 结果 | 明示未实现 / 证据边界 |
| --- | --- | --- |
| `MASSPROP` | **完整实现（只读；2D 区域 + 精确 3D body）**：逐对象与汇总报告面积、周长、精确包围盒、形心，以及 **`Ix`/`Iy`/`Ixy`（绕各自形心）、`J`、`kx`/`ky`**；汇总再给出**主惯性矩 `I1`/`I2`、主轴 `axis1`/`axis2`、主角 `principal_angle_deg`** 与所用坐标系（`origin/u/v/n`）。二阶矩：圆 `πr⁴/4`、整椭圆 `πa³b/4`/`πab³/4`（旋转到区域系）、闭合折线用 Green 公式 + 每段 bulge 的**圆缺精确矩**（扇形 − 等腰三角形；`α<1e-2` 用级数避免抵消）、HATCH 按环嵌套奇偶符号求和（岛取自身形心矩）、闭合 SPLINE 与导入时被拉直的填充边为**采样并标注**（`exact=false`）；各区域矩先平移到形心，再经一次精确换标 `reexpress` 汇入同一选择坐标系，平动/旋转写错即拒。当前选择为**闭合、共面**的 `CIRCLE` / 完整 `ELLIPSE`（`|sweep| >= 2π − 1e-9`）/ 闭合 `POLYLINE`（任意 bulge，同一平面）。逐对象与汇总诊断报告**面积、周长、精确包围盒（min/max）、形心**；圆/椭圆用解析值，bulged 闭合折线用 `cad_geometry::measure_bulged_ring_area` 的精确面积、直线+圆弧精确周长、直线边+圆缺一阶矩形心。椭圆周长是 **Ramanujan 第二近似**，以 `massprop.approx_ellipse_perimeter` 明示，不是精确椭圆积分。空选择 `massprop.selection`；跨对象法向（符号无关）/标高不一致 `massprop.not_coplanar`；开放折线/圆弧/椭圆弧/样条/文字/块参照/填充/标注/无精确 B-Rep 的 3D 几何/不可寻址引用分别按名拒绝。**无**数据库写、revision、撤销或 `ChangeSet`；Viewer 与 Work 均可用 | **body 与区域是两条报告形状（C3a，2026-11-17）**：选择**全部**为 `Body3d{Solid, Exact}` 时，逐体经**注入的**模型器**同步**报**体积、表面积、质心、`Ixx`/`Iyy`/`Izz`/`Iyz`/`Izx`/`Ixy`（绕形心）、密度与质量**（density 1.0、请求相对精度 1e-6，明写“估计、不是已证明的界”；无惯性张量时写 `NOT REPORTED`；**显示网格永不积分**），行码 `massprop.object`；稳定拒绝 `massprop.body_modeler_unavailable`（载体无模型器）/`body_not_liftable`（模型器拒绝该 B-Rep：曲面已可 lift，剩下的是无 `(u,v)` box 的曲面面、无精确 pcurve 的边界、多壳/空腔等结构性原因）/`body_kernel_failed`/`body_not_exact`/`body_not_solid`/`mixed_selection`（body 与平面区域混选）。**区域报告仍无体积**：`massprop.solid_unsupported` 仍是区域路径对体积/质量/密度/3D 惯性矩的具名拒绝；`Add`/`Subtract` 区域组合与 `.mpr` 文件（交互提示环/文件输出通道）仍未建模。退化面积 `massprop.inertia_degenerate`。证据：区域为源码 + `cad_app`/`cad_geometry` 契约测试（矩形 `Ix=bh³/12`、椭圆 `3π/4`/`27π/4`、旋转 3×4 的 `Ix=Iy=12.5`/`Ixy=−3.5`/主轴 16/9 与 45°、平行轴合成、样条采样标注、带岛填充 `Ix=16−2/3`）；body 为 `cad-app` 契约 + `cad-cli-tools` **真实内核**的 `solid_command` 契约（BOX 报 `volume=1.000000`/`surface_area=6.000000`/形心 0.5³，CYLINDER 报 `volume=62.831853 surface_area=87.964594`，即 20π/28π）；无 GUI/真实图纸/像素证据 |
| `MEASUREGEOM` | **完整实现（选项路由 + 只读报告）**：`MEASUREGEOM DISTANCE\|ANGLE\|AREA\|POLYLINE` 发送与 `DIST`/`MEASUREANGLE`/`AREA`/`MEASUREPOLYLINE` **相同**的 `BeginMeasurement` 请求；`RADIUS` 只读报告单个 `ARC`/`CIRCLE`；`VOLUME` 走与 `MASSPROP` **相同**的单体精确 body 路径（`mass_prop_bodies`/`integrate_body`）报告体积/表面积/质心 | **未实现**：多体选择（`measuregeom.volume.multiple`）、非实体选择（`measuregeom.volume.unsupported`）、无精确 B-Rep/非 solid 拓扑（沿用 `massprop.body_*` 具名拒绝）；AutoCAD 其它 `MEASUREGEOM` 选项与动态测量未接线。证据为 `cad-ui-core` 路由契约 + `cad_app` RADIUS/VOLUME 契约；无真机/像素/性能证据 |

### 2026-11-09 Wave 4 扩展命令（真实 2D 编辑 + 具名拒绝）

| 命令 | 结果 | 明示未实现 / 证据边界 |
| --- | --- | --- |
| `CENTERMARK` | **完整实现**：每个选中顶层模型空间 `CIRCLE`/`ARC` 圆心创建两条交叉 `LINE`；默认臂长 = 半径 **10%**，`ARM <distance>` 或 `<factor>x` 可配；一条事务/一步撤销 | 产出普通 `LINE`，**非关联**（AutoCAD 的 associative 对象未建模，声明边界） |
| `CENTERLINE` | **完整实现**：恰好两条非退化 `LINE`，平行容差内；创建一条中间 `LINE`，跨两条投影重叠段；`EXTENSION <d>`/`GAP <d>` 可调；一条事务/一步撤销 | 产出普通 `LINE`，**非关联**（AutoCAD 的 associative 对象未建模，声明边界）；非平行/混合选择按名拒绝 |
| `NCOPY` | **部分实现**：选中 `INSERT` 的可见成员几何按世界变换复制到模型空间（新顶层实体）；一条事务/一步撤销 | 无嵌套选择界面；非 INSERT 按名拒绝 |
| `COPYTOLAYER` | **部分实现**：首个选中实体的图层为目标；其余选中顶层模型空间实体以新 id 复制到该层，原实体不变；一条事务/一步撤销 | 无图层提示；块成员/图纸空间按名拒绝 |
| `-COPYTOLAYER` | **部分实现**：与 `COPYTOLAYER` 相同的实现，命令行拼写；一条事务/一步撤销 | 同上 |
| `CDORDER` | **部分实现**：按颜色索引升序重写选中（空选择 = 全部模型空间）实体的存储绘制顺序；保留候选占用的原 draw-order 槽位；一条事务/一步撤销 | 无对话框；ByBlock=0、非调色板真彩=256、缺层属性=7 是声明排序规则 |
| `FLATTEN` | **部分实现**：`LINE`/`POLYLINE`/`CIRCLE`/`ARC`/`ELLIPSE`/`POINT`/`SPLINE` 的 z 改写为 0；新增 `TEXT`/`SHAPE`/`MASK`/已平行世界 XY 的 `IMAGE`/`COMPOUND` 与**竖直 `INSERT`**（只移动插入点 z 与属性文字 z，块定义不改；成员非平或放置倾斜按名拒绝）；一条事务/一步撤销 | Express 的投影语义简化为 z=0；`MESH`/`Body3d`、派生显示缓存的 `DIMENSION`/`HATCH`/`MULTILEADER`/`TABLE`、非 XY 平面 `IMAGE` 按名拒绝 |
| `XPLODE` | **完整实现**：块参照的一级 EXPLODE 分解 + `All`/`Color`/`LAyer`/`LType`/`LWeight`/`Inherit from parent block`/`Explode` 控制；`layer`/`color`/`linetype`/`lineweight` 经 `plan_entity` 写入每个子实体，`Inherit` 把 layer 0 与 `ByBlock` 按参照/参照图层解析；块属性值丢弃并由块定义 `ATTDEF` 作为普通实体产出（`BURST` 保值）；一条事务/一步撤销 | 仅块参照（与 `BURST` 相同的一级块范围）；`transparency`/`ltscale`/thickness/material/plot style 按名拒绝；`Inherit` 与显式属性列表互斥；`Individually` 交互提示不提供（全局交互范围）；非均匀放置/冻结层/XREF 按名拒绝 |
| `BTRIM` | **部分实现**：首个选中 `INSERT` 的成员几何为边界，其余选中实体复用既有 TRIM 规划器修剪（拾取点为曲线中点）；一条事务/一步撤销 | 无交互式边界提示；不可规划的目标按名拒绝 |
| `BEXTEND` | **部分实现**：同上，复用既有 EXTEND 规划器（拾取点为曲线终点）；一条事务/一步撤销 | 无交互式边界提示；不可规划的目标按名拒绝 |
| `CHANGE` | **部分实现**：与 `CHPROP` 相同的 typed 属性列表（`layer`/`color`/`linetype`/`lineweight`/`transparency`，与 `SETENTITYPROPERTY` 共用语法与 `plan_entity` 应用器），模型与图纸空间直取实体；一条事务/一步撤销 | AutoCAD 的 change-point 分支（移动点/文字）与交互式特性提示未实现；未建模属性按名拒绝 |
| `CHPROP` | **完整实现**：typed 属性列表（`layer`/`color`/`linetype`/`lineweight`/`transparency`），与 `SETENTITYPROPERTY` 共用 `PropertyValue` 语法与 `plan_entity` 应用器；模型与图纸空间直取实体；一条事务/一步撤销 | 未建模属性（`text`、thickness、material、plot style、linetype scale）按名拒绝（`chprop.unsupported_property`）；无交互式属性提示（全局交互范围） |
| `BSCALE` | **部分实现（具名拒绝）**：`bscale.unsupported: BSCALE needs a factor and an insertion reference; use SCALE on the selected insert` | 不写库/不升修订/不入历史 |
| `MINSERT` | **部分实现（具名拒绝）**：`minsert.unsupported: MINSERT needs a block name and row/column counts; use INSERT then ARRAYRECT` | 同上 |
| `BLOCKREPLACE` | **部分实现（具名拒绝）**：`blockreplace.unsupported: BLOCKREPLACE needs two block definitions; this build has no block-replace editor` | 同上 |
| `BREPLACE` | **部分实现（具名拒绝）**：`breplace.unsupported: BREPLACE needs two block definitions; this build has no block-replace editor` | 同上 |
| `-BREPLACE` | **部分实现（具名拒绝）**：`-breplace.unsupported: BREPLACE needs two block definitions; this build has no block-replace editor` | 同上 |
| `SPLINEDIT` | **部分实现（具名拒绝）**：`splinedit.unsupported: this build has no NURBS edit surface` | 同上 |
| `MLEDIT` | **部分实现（具名拒绝）**：`mlinedit.unsupported: this build has no MLINE join/cap edit model` | 同上 |
| `-MLEDIT` | **部分实现（具名拒绝）**：`-mlinedit.unsupported: this build has no MLINE join/cap edit model` | 同上 |
| `BURST` | **完整实现**：一级 EXPLODE + 属性值转 TEXT（含常量）；一条事务/一步撤销 | 无对话框；导入的非均匀/镜像引用仍按名报告 |
| `CLIP` | **完整实现**：`RECT`（目标自身裁剪空间）/`POLY`（仅光栅图像，凸且不自交）/`ON`/`OFF`/`DELETE`，分派到既有 `IMAGECLIP`/`XCLIP` 写路径，一条事务/一步撤销 | 块参照的多边形裁剪按名拒绝（`clip.polygon_block_not_renderable`，表示层只画矩形块裁剪）；旋转/倾斜参照沿用 `xclip.rotation_not_renderable`/`xclip.non_xy_plane`；图像 ON/OFF 无字段；PDF/DGN/DWF 底图未建模 |
| `CLIPIT` | **完整实现**：预选择 1 个图像目标 + 闭合边界（`LINE`/`ARC`/`LWPOLYLINE`，复用 JOIN 链与 `VPCLIP` 多边形校验），一条事务/一步撤销 | 块参照目标按名拒绝（`clipit.polygon_block_not_renderable`，用 `CLIP RECT`）；开放/自交/退化/非凸/多目标/无边界具名拒绝；圆弧边界为 32 弦近似 |
| `REGION` | **部分实现（具名拒绝）**：`region.unsupported: this build has no Region geometry model` | 同上 |
| `ADDSELECTED` | **完整实现**（后续 Wave 26 落地） | 点锚定型 LINE/CIRCLE/ARC/POLYLINE/INSERT/POINT/TEXT/MTEXT 新建，保留五项属性；一条事务/一步撤销 | 其它种类按名拒绝；仅模型空间 |
| `WBLOCK` | **部分实现（具名拒绝）**：`wblock.unsupported: writing a block definition to a separate drawing file is not implemented` | 同上 |
| `-WBLOCK` | **部分实现（具名拒绝）**：`-wblock.unsupported: writing a block definition to a separate drawing file is not implemented` | 同上 |
| `BSAVE` | **部分实现（具名拒绝）**：`bsave.unsupported: this build has no block editor` | 同上 |
| `BSAVEAS` | **部分实现（具名拒绝）**：`bsaveas.unsupported: this build has no block editor` | 同上 |
| `CHSPACE` | 完整实现 | 模型↔图纸按**活动视口已解析变换**（`to_paper`/`to_model`，与显示同一解析）改写几何并重指 space，一条事务/一步撤销 | 无可用视口（`chspace.no_viewport`/`unsupported_viewport`）、视口实体、跨空间混合选择、块内成员、标注（本构建无关联模型，`chspace.dimension_not_associative`）、无法精确变换的种类（`MULTILEADER`/`SHAPE`/`OPAQUE`/`COMPOUND`）逐条具名拒绝且不写 |

### 2026-11-09 Wave 5 扩展命令（属性 + 外部参照）

| 命令 | 结果 | 明示未实现 / 证据边界 |
| --- | --- | --- |
| `ATTDISP` | **完整实现**：Normal/On/Off 显示模式；实例展开按定义顺序绘制值（constant 恒显），不写库/不升修订；出图用 Normal | 无 GPU/像素证据；斜体角度导入导出但绘制仍直立（具名诊断） |
| `BATTMAN` | **完整实现**：完整块属性管理器报告（定义/值/顺序/stale） | 无对话框/面板选择器（ADR 全局范围） |
| `BATTORDER` | **完整实现**：写属性定义顺序并显式重写到引用；裸命令报告 | 无对话框 |
| `XREF` | **部分实现（只读）**：读取 `drawing.external_references()` 导入期溯源，`xref.entry`/`xref.summary`（名称/状态/原始与解析路径/深度/overlay/块） | 无附着/卸载/重载/绑定/编辑；空映射报告 0 行，不虚构；浏览器仍按导入期 `browser-unavailable` 状态显示 |
| `-XREF` | **部分实现（只读）**：同一报告，使用 `-xref.entry`/`-xref.summary` 诊断码 | 同上；命令行拼写不能附着/卸载/重载/绑定 |
| `EXTERNALREFERENCES` | **部分实现（只读）**：同一报告，`externalreferences.entry`/`.summary` | 本构建**无 External References 选项板**（`externalreferences.palette_not_implemented`）；不打开任何 UI |
| `EXTERNALREFERENCESCLOSE` | **部分实现（具名 no-op）**：`externalreferences.close_not_implemented`，不改变任何状态 | 没有选项板可关闭 |
| `ATTDEF` | **完整实现**：在块定义中创建属性定义（tag/prompt/default/flags/order/text placement）；裸命令列出定义 | 无属性对话框（ADR 全局范围）；真实 DWG 往返 NOT RUN |
| `-ATTDEF` | **完整实现**：`ATTDEF` 的命令行拼写，同一实现 | 同上 |
| `ATTEDIT` | **完整实现**：单选 INSERT 的属性值编辑；tag 大小写不敏感，constant/unknown/locked 按名拒绝 | 无属性对话框；不处理嵌套实例成员 |
| `-ATTEDIT` | **完整实现**：`ATTEDIT` 的命令行拼写，作用于该块全部 INSERT | 同上 |
| `ATTIPEDIT` | **完整实现**：单选 INSERT 的值编辑 | 无对话框 |
| `ATTIN` | **部分实现（具名拒绝）**：`attin.unsupported: this build has no block-attribute model` | 同上 |
| `ATTOUT` | **部分实现（具名拒绝）**：`attout.unsupported: this build has no block-attribute model` | 同上 |
| `ATTEXT` | **部分实现（具名拒绝）**：`attext.unsupported: this build has no block-attribute model` | 同上 |
| `-ATTEXT` | **部分实现（具名拒绝）**：`-attext.unsupported: this build has no block-attribute model` | 同上 |
| `EATTEDIT` | **完整实现**：多选 INSERT 的值 + 块局部位置/字高/旋转编辑；一条事务/一步撤销 | Enhanced Attribute Editor 对话框属全局范围 |
| `EATTEXT` | **部分实现（具名拒绝）**：`eatttext.unsupported: this build has no block-attribute model` | 同上（原因串按历史拼写 `eatttext`，与选择子 `EATTEXT` 一致地稳定） |
| `-EATTEXT` | **部分实现（具名拒绝）**：`-eatttext.unsupported: this build has no block-attribute model` | 同上 |
| `ATTREDEF` | **完整实现**：重定义块属性并显式重写所有引用值；一条事务/一步撤销 | 无交互对话框；真实 DWG 往返 NOT RUN |
| `ATTSYNC` | **完整实现**：把块定义属性同步到所有引用（显式写值）；裸命令报告/同步选择 | 无对话框 |
| `GATTE` | **完整实现**：按块名全局修改所有引用值；一条事务/一步撤销 | 无对话框 |
| `XDATA` | **部分实现（具名拒绝）**：`xdata.unsupported: this database does not retain entity XDATA` | 同上 |
| `XBIND` | **部分实现（具名拒绝）**：`xbind.unsupported: this build has no xref bind/insert model` | 同上 |
| `-XBIND` | **部分实现（具名拒绝）**：`-xbind.unsupported: this build has no xref bind/insert model` | 同上 |
| `XATTACH` | **部分实现（具名拒绝）**：`xattach.unsupported: this build does not attach external drawings as xrefs` | 同上 |
| `ATTACH` | **部分实现（具名拒绝）**：`attach.unsupported: this build does not attach external references or underlays` | 同上 |
| `-ATTACH` | **部分实现（具名拒绝）**：`-attach.unsupported: this build does not attach external references or underlays` | 同上 |
| `XOPEN` | **部分实现（具名拒绝）**：`xopen.unsupported: this build has no xref editing session` | 同上 |
| `CLASSICXREF` | **已实现（只读报告 + 界面面板）**：`classicxref.entry`/`.summary` 读取 `drawing.external_references()`；界面本地经典面板列出 `cad_status.references` 行 | 附着/拆离/重载保持 `classicxref.*_unsupported` 具名拒绝 |

### 2026-11-09 Wave 6 扩展命令（文件 / 出图 / 导出）

| 命令 | 结果 | 明示未实现 / 证据边界 |
| --- | --- | --- |
| `DWGLOG` | **部分实现（会话令牌）**：翻转会话 `dwglog` 令牌（默认 **OFF**）；`dwglog.toggled`；无事务/修订/历史 | 本构建**不写日志文件**，也不写图；没有日志消费者 |
| `LOGFILEON` | **部分实现（会话令牌）**：把同一 `dwglog` 令牌置 **ON**；`logfileon.toggled` | 不写日志文件 |
| `LOGFILEOFF` | **部分实现（会话令牌）**：把同一 `dwglog` 令牌置 **OFF**；`logfileoff.toggled` | 不写日志文件 |
| `DWGPROPS` | **完整实现（2026-11-24）**：报告文档 id/修订号/实体数 + `title`/`author`/`subject`/`keywords`/`comments`/自定义属性（`dwgprops.field`/`.custom`/`.summary`/`.empty`）；`DWGPROPS <field> <text>` 与 `CUSTOM <name> <text>` 写入 `document.json` → `variables.metadata`（空值清除，一条 revision/header-only `ChangeSet`/无 undo patch）；`.ymodel` 快照往返有契约；Viewer 可报告但拒绝写入 | 无 Drawing Properties 对话框（全局范围）；DXF/DWG 导出器不携带摘要信息（连接器边界，不杜撰）；自定义属性上限 256/名称 255 字符/值 4096 字符 | 写 `document.json`，无元素/表格行 |
| `DWFFORMAT` | **部分实现（只读能力报告）**：`dwfformat.not_supported` 说明本构建不导出 DWF/DWFx，并指向 `PLOT` 的 SVG/PDF | 不是错误；没有默认格式设置，不写任何文件 |
| `EXPORTPDF` | **部分实现（只读能力报告）**：`exportpdf.use_plot` 说明 PDF 导出由宿主经 `PLOT`（平台保存对话框）负责 | 不是错误；本命令自身不写 PDF，`cad-app` 不重实现宿主文件 I/O |
| `SAVEALL` | **部分实现（具名拒绝）**：`saveall.single_document: this build has one document; use SAVE` | 不写库/不升修订/不入历史 |
| `QNEW` | **部分实现（具名拒绝）**：`qnew.single_document: use NEW` | 同上 |
| `REVERT` | **部分实现（具名拒绝）**：`revert.unsupported: no reopen-to-source path` | 同上 |
| `CLOSEALL` | **部分实现（具名拒绝）**：`closeall.single_document: use CLOSE` | 同上 |
| `CLOSEALLOTHER` | **部分实现（具名拒绝）**：`closeallother.single_document: only one document exists` | 同上 |
| `QQUIT` | **部分实现（具名拒绝）**：`qquit.unsupported: application exit is host-owned` | 同上 |
| `QUIT` | **部分实现（具名拒绝）**：`quit.unsupported: application exit is host-owned` | 同上 |
| `PROPULATE` | **部分实现（具名拒绝）**：`propulate.unsupported: no property-propagation model` | 同上 |
| `PLOTTERMANAGER` | **部分实现（具名拒绝）**：`plottermanager.unsupported: no plotter configuration model` | 同上 |
| `STYLESMANAGER` | **部分实现（具名拒绝）**：`stylesmanager.unsupported: no CTB/STB plot-style model` | 同上 |
| `EXPORTSETTINGS` | **部分实现（具名拒绝）**：`exportsettings.unsupported: no export-settings model` | 同上 |
| `PREVIEW` | **部分实现（具名拒绝）**：`preview.unsupported: no plot-preview window` | 同上 |
| `EXPORTLAYOUT` | **部分实现（具名拒绝）**：`exportlayout.unsupported: use PLOT (one layout, SVG/PDF)` | 同上 |
| `PLOTSTAMP` | **部分实现（具名拒绝）**：`plotstamp.unsupported: no plot-stamp model` | 同上 |
| `-PLOTSTAMP` | **部分实现（具名拒绝）**：`-plotstamp.unsupported: no plot-stamp model` | 同上 |
| `-PLOT` | **部分实现（具名拒绝）**：`-plot.unsupported: use PLOT; this build has one vector plot path` | 同上 |
| `-EXPORT` | **部分实现（具名拒绝）**：`-export.unsupported: use PLOT/EXPORT; this build has one vector export path` | 同上 |
| `ETRANSMIT` | **部分实现（具名拒绝）**：`etransmit.unsupported: no eTransmit/zip packaging model` | 同上 |
| `-ETRANSMIT` | **部分实现（具名拒绝）**：`-etransmit.unsupported: no eTransmit/zip packaging model` | 同上 |
| `ARCHIVE` | **部分实现（具名拒绝）**：`archive.unsupported: no sheet-set archive packaging model` | 同上 |
| `-ARCHIVE` | **部分实现（具名拒绝）**：`-archive.unsupported: no sheet-set archive packaging model` | 同上 |
| `PUBLISH` | **部分实现（具名拒绝）**：`publish.unsupported: no batch publish model` | 同上 |
| `-PUBLISH` | **部分实现（具名拒绝）**：`-publish.unsupported: no batch publish model` | 同上 |
| `AUTOPUBLISH` | **部分实现（具名拒绝）**：`autopublish.unsupported: no automatic publish model` | 同上 |

### 2026-11-09 Wave 7 扩展命令（动态块 / 约束）

| 命令 | 结果 | 明示未实现 / 证据边界 |
| --- | --- | --- |
| `BVSTATE` | **部分实现（只读）**：逐选中 INSERT 报告 `block_dynamic_visibility` 的活动状态；`bvstate.report` | 无 INSERT 报 `bvstate.no_selection`；无描述符或活动状态未解析报 `bvstate.not_set`，不猜测/不设置状态 |
| `-BVSTATE` | **部分实现（只读）**：同 `BVSTATE`，使用 `-bvstate.*` 诊断码 | 同上；不设置状态 |
| `BACTION` | **部分实现（具名拒绝）**：`baction.unsupported: this build has no dynamic-block action model` | 不写库/不升修订/不入历史 |
| `BACTIONBAR` | **部分实现（具名拒绝）**：`bactionbar.unsupported: this build has no dynamic-block action model` | 同上 |
| `BACTIONSET` | **部分实现（具名拒绝）**：`bactionset.unsupported: this build has no dynamic-block action model` | 同上 |
| `BACTIONTOOL` | **部分实现（具名拒绝）**：`bactiontool.unsupported: this build has no dynamic-block action model` | 同上 |
| `BASSOCIATE` | **部分实现（具名拒绝）**：`bassociate.unsupported: this build has no dynamic-block action model` | 同上 |
| `BAUTHORPALETTE` | **部分实现（具名拒绝）**：`bauthorpalette.unsupported: this build has no block editor` | 同上 |
| `BAUTHORPALETTECLOSE` | **部分实现（具名拒绝）**：`bauthorpaletteclose.unsupported: this build has no block editor` | 同上 |
| `BCONSTRUCTION` | **部分实现（具名拒绝）**：`bconstruction.unsupported: this build has no dynamic-block construction geometry` | 同上 |
| `BCPARAMETER` | **部分实现（具名拒绝）**：`bcparameter.unsupported: this build has no dynamic-block parameter model` | 同上 |
| `-BCPARAMETER` | **部分实现（具名拒绝）**：`-bcparameter.unsupported: this build has no dynamic-block parameter model` | 同上 |
| `BPARAMETER` | **部分实现（具名拒绝）**：`bparameter.unsupported: this build has no dynamic-block parameter model` | 同上 |
| `BLOOKUPTABLE` | **部分实现（具名拒绝）**：`blookuptable.unsupported: this build has no dynamic-block lookup table` | 同上 |
| `BTABLE` | **部分实现（具名拒绝）**：`btable.unsupported: this build has no dynamic-block lookup table` | 同上 |
| `BVHIDE` | **部分实现（具名拒绝）**：`bvhide.unsupported: no per-visibility-state entity hide API` | 同上 |
| `BVSHOW` | **部分实现（具名拒绝）**：`bvshow.unsupported: no per-visibility-state entity show API` | 同上 |
| `RESETBLOCK` | **部分实现（具名拒绝）**：`resetblock.unsupported: no dynamic-block parameter reset API` | 同上 |
| `AUTOCONSTRAIN` | **部分实现（具名拒绝）**：`autoconstrain.unsupported: this build has no geometric-constraint model` | 同上 |
| `DELCONSTRAINT` | **部分实现（具名拒绝）**：`delconstraint.unsupported: this build has no geometric-constraint model` | 同上 |
| `DIMCONSTRAINT` | **部分实现（具名拒绝）**：`dimconstraint.unsupported: this build has no dimensional-constraint model` | 同上 |
| `GEOMCONSTRAINT` | **部分实现（具名拒绝）**：`geomconstraint.unsupported: this build has no geometric-constraint model` | 同上 |
| `DCALIGNED` | **部分实现（具名拒绝）**：`dcaligned.unsupported: this build has no dimensional-constraint model` | 同上 |
| `DCANGULAR` | **部分实现（具名拒绝）**：`dcangular.unsupported: this build has no dimensional-constraint model` | 同上 |
| `DCCONVERT` | **部分实现（具名拒绝）**：`dcconvert.unsupported: this build has no dimensional-constraint model` | 同上 |
| `DCDIAMETER` | **部分实现（具名拒绝）**：`dcdiameter.unsupported: this build has no dimensional-constraint model` | 同上 |
| `DCDISPLAY` | **部分实现（具名拒绝）**：`dcdisplay.unsupported: this build has no dimensional-constraint model` | 同上 |
| `DCFORM` | **部分实现（具名拒绝）**：`dcform.unsupported: this build has no dimensional-constraint model` | 同上 |
| `DCHORIZONTAL` | **部分实现（具名拒绝）**：`dchorizontal.unsupported: this build has no dimensional-constraint model` | 同上 |
| `DCLINEAR` | **部分实现（具名拒绝）**：`dclinear.unsupported: this build has no dimensional-constraint model` | 同上 |

### 2026-11-09 Wave 8 扩展命令（会话令牌 / 稳定拒绝）

| 命令 | 结果 | 明示未实现 / 证据边界 |
| --- | --- | --- |
| `ELEV` | 完整实现 | `ELEV <height>`（有限小数，可负）与会话令牌；`SETVAR ELEV` 可读写；创建漏斗三个入口在事务开启前按标高做纯 z 平移（唯一变换权威），`ELEV 0.0` 与旧构建逐字节相同；复制/编辑路径保留源 z（`app_drawing/elevation.rs` 逐条列出） | 不适用于块内成员/复制类命令（声明）；非有限值 `elev.invalid_value` 且不开事务 |
| `UCSICON` | **部分实现（会话开关）**：翻转 `ucsicon`（默认关，`ucsicon.toggled`） | 本构建不渲染 UCS 图标，不声称图标可见性/位置/外观；不写库/不升修订 |
| `UCS` | **部分实现（具名拒绝）**：`ucs.unsupported: this build has no UCS model` | 不写库/不升修订/不入历史 |
| `UCSMAN` | **部分实现（具名拒绝）**：`ucsman.unsupported: this build has no UCS model` | 同上 |
| `TABLET` | **部分实现（具名拒绝）**：`tablet.unsupported: no tablet/digitizer model` | 同上 |
| `TASKBAR` | **部分实现（具名拒绝）**：`taskbar.unsupported: application window chrome is host-owned` | 同上 |
| `REINIT` | **部分实现（具名拒绝）**：`reinit.unsupported: no device/digitizer initialization model` | 同上 |
| `SYSWINDOWS` | **部分实现（具名拒绝）**：`syswindows.unsupported: application window layout is host-owned` | 同上 |
| `MENU` | **部分实现（具名拒绝）**：`menu.unsupported: no menu-file model; the UI is fixed` | 同上 |
| `RIBBON` | **部分实现（具名拒绝）**：`ribbon.unsupported: ribbon visibility is host-owned chrome` | 同上 |

### 2026-11-09 Wave 9 扩展命令（2D 绘制/修改 + 别名 + 报告）

> 证据仅 Linux 编译 + `cad-app`/`cad-ui-core` 契约测试 + 前端 `node --test`；GUI/GPU/像素/真机 NOT VERIFIED。`部分实现` 只表示命令路径存在并如实工作/拒绝。

| 命令 | 结果 | 明示未实现 / 证据边界 |
| --- | --- | --- |
| `ARRAY` / `-ARRAY` / `ARRAYCLASSIC` | **部分实现（路由）**：`ArrayRect`/`ArrayPolar` 载荷转交现有 `ARRAYRECT`/`ARRAYPOLAR`；无载荷时 `array.needs_payload` 只读报告 | 无旧式对话框、无关联阵列/Levels/Path；路由本身不新增写路径 |
| `BLEND` | **部分实现（真实 2D 编辑）**：两条开放 `LINE`/`ARC`/`LWPOLYLINE` 最近端点间一条三次 `SPLINE` | 内部控制点沿源切线取间隙 1/3（声明规则）；无曲线连续性/多段混合；仅开放曲线 |
| `BREAKLINE` | **部分实现（真实 2D 编辑）**：两点间一条开放折线含锯齿折断符号（15% 宽 / 5% 深） | 符号是声明式规则，不是 AutoCAD 块式折断符号；无尺寸选项 |
| `-BLOCK` / `-INSERT` | **已实现（别名）**：调用唯一 `BLOCK` / `INSERT` 处理器 | 同一事务/撤销；无第二套实现 |
| `-BOUNDARY` / `-HATCH` / `-HATCHEDIT` / `EXOFFSET` | 别名（各自父命令同一处理器） | `-BOUNDARY` 与 `EXOFFSET` **完整实现**（与已完成的 `BOUNDARY`/`OFFSET` 完全一致，含边界集/岛屿检测与全部偏移种类）；`-HATCHEDIT` **完整实现**（与 `HATCHEDIT` 一致，含 `ORIGIN`/`PATTERNFILE`）；`-HATCH` **完整实现**（与 `HATCH` 同一支持域；关联性/预览属全局范围） |
| `DIM` | **部分实现（只读路由报告）**：`dim.available` 列出已实现 `DIM*` 选择子；`dim.router_only` | 不创建标注；无交互式类型提示 |
| `UNITS` | **部分实现（2026-11-24 真实写入）**：裸命令读取实时 `Document.units`（单位/精度，不再是写死的 4 位）；`UNITS <unit> <precision>` 写声明单位与线性小数位（一条 header revision、header-only `ChangeSet`、无 undo patch）；Viewer 拒绝写 | 角度格式/精度/方向/基准角、长度 *format* 表（Scientific/Engineering/Architectural/Fractional）、`$INSUNITS` 插入比例语义与 UNITS 对话框未建模（`units.not_implemented`） | 写 `document.json` |
| `ARCTEXT` | **部分实现（具名拒绝）**：`arctext.unsupported: text along an arc is not modeled; no combined arc+text payload` | 不写库/不升修订/不入历史 |
| `MLEADERALIGN` | **部分实现（具名拒绝）**：`mleaderalign.unsupported: no multileader alignment model` | 同上 |
| `MLEADEREDIT` | 完整实现 | `ADD <x> <y>` / `REMOVE <index>`（可清空一个根；整条最后一条 `mleaderedit.last_line`）+ `PATHTYPE <i> LINE\|SPLINE\|INVISIBLE`（组 `170`+`93` bit1）、`ARROWSIZE <i> <size>`（组 `40`+bit16）、`BYSTYLE <i> PATH\|ARROW`（清位并从样式重盖）；全部经唯一 MULTILEADER 写入器往返；一条事务/一步撤销 | 逐线颜色/线型/线宽与自定义箭头块句柄无可用语义模型，按名拒绝；无逐顶点 setter（顶点由 `ADD`/`MLEADERALIGN` 改） |
| `MLEADERCOLLECT` | 完整实现 | 合并 ≥2 MULTILEADER 为一条（根/线拼接、文本 `\P` 连接、根落点重定向）并删除源；样式/覆盖掩码/平面不一致的源**接受**：逐线路径类型（`170`/覆盖字 `93` bit1）与箭头大小（`40`/bit16）按逐线覆盖保留，各根保留自身狗腿，无逐线组的字段与平面框取自首条并逐项报告（`mleadercollect.style_adopted`/`.plane_adopted`）；一条事务/一步撤销，DXF 往返有测试 | 带 `reserved` 源数据（断口/背景/分栏）按名拒绝（无合并语义）；无对话框 |
| `TOLERANCE` | **部分实现（具名拒绝）**：`tolerance.unsupported: no feature-control-frame (tolerance) object model` | 同上 |
| `DIMARC` | **部分实现（具名拒绝）**：`dimarc.unsupported: the dimension domain has no arc-length kind` | 同上 |
| `CAL` | **部分实现（具名拒绝）**：`cal.unsupported: no expression evaluator is implemented` | 同上 |
| `DIMEX` / `DIMIM` | **部分实现（具名拒绝）**：`dimex.unsupported` / `dimim.unsupported`：标注样式文件未建模 | 同上 |
| `SPACETRANS` | **已实现（只读换算）**：`SPACETRANS <distance>` 用当前布局视口的 `viewport_transform` 报 `spacetrans.converted`/`.factor` | 模型空间或无可用视口具名拒绝 |
| `TXTEXP` | **部分实现（具名拒绝）**：`txtexp.unsupported: no font-outline-to-geometry path is implemented` | 同上 |
| `UPDATEFIELD` | **部分实现（具名拒绝）**：`updatefield.unsupported: fields are not modeled` | 同上 |

### 2026-11-09 Wave 10 扩展命令（图层/视图/会话报告 + chrome 本地）

> 证据仅 Linux 编译 + `cad-app`/`cad-ui-core` 契约测试 + 前端 `node --test`；GUI/GPU/像素/真机 NOT VERIFIED。

| 命令 | 结果 | 明示未实现 / 证据边界 |
| --- | --- | --- |
| `-LAYER` | **部分实现（只读报告）**：真实图层表（名称/可见/off/frozen/locked/当前层） | 图层增删改名/开关/冻结/锁定写形式未实现；不写库 |
| `LAYERSTATESAVE` | **部分实现（载荷路由）**：`LayerState` 载荷转交既有 `LAYERSTATE` 会话存储 | 无收录对话框；无载荷只报告；会话状态不持久化 |
| `NEWVIEW` | **部分实现（载荷路由）**：`View` 载荷转交持久化命名视图表（`tables/views.json`，minor 18） | 无窗口/名称提示；无矩形窗口定义形式；无载荷只报告 `newview.needs_name` |
| `-SCALELISTEDIT` | **部分实现（只读报告）**：真实 `ACAD_SCALELIST` 条目 | 创建/编辑/删除/排序比例未实现 |
| `LWEIGHT` / `-LWEIGHT` | **部分实现（只读报告）**：会话线宽显示令牌（默认 ON） | 渲染器不消费该标志；LWEIGHT 对话框未实现 |
| `COLOR` / `-COLOR` | **完整实现**：`<0..=256>` / `BYLAYER` / `BYNAME` / `BYBLOCK` 写会话当前颜色（`color.set`，无库写、无撤销），裸命令报告（`color.report`，默认 256=ByLayer）；**新实体由创建漏斗写入该 ACI**（`256` 行逐字节不变，`0` = ByBlock 符号值，`1..=255` 经 `property_value::rgb_for_aci` 解析），撤销随实体行一起回退 | **全局范围**：无调色板/Select Color 对话框（`src/components/ColorChooser.tsx` 是实体属性选择器，不是会话令牌）；无颜色名表（只接受 ACI 与三个符号词）；`CELTSCALE` 等颜色相关变量不在范围 |
| `-UNITS` | **部分实现（2026-11-24 真实写入）**：与 `UNITS` 同一实现臂；裸命令报告实时单位，`-UNITS <unit> <precision>` 写声明单位与线性小数位（一条 header revision） | 角度格式/精度/方向/长度 *format* 表/`$INSUNITS`/Units 对话框未建模 | 写 `document.json` |
| `COPYHIST` | **部分实现（只读报告）**：`copyhist.not_tracked` | 会话不存命令历史文本；不复制 |
| `HELP` / `CLASSICINSERT` | **部分实现（具名拒绝）**：`help.unsupported` / `classicinsert.unsupported` | 不写库/不升修订/不入历史 |
| `FILETAB` / `FILETABCLOSE` / `HIDEPALETTES` / `SHOWPALETTES` / `RIBBONCLOSE` / `GOTOSTART` | **部分实现（具名拒绝）**：窗口/选项板/功能区/开始页 chrome 由宿主负责 | 同上 |
| `TRANSPARENCY` | **部分实现（具名拒绝）**：`transparency.unsupported: image transparency is not modeled` | 同上 |
| `DSETTINGS` / `QUICKPROPERTIES` / `CLASSICLAYER` / `COMMANDLINE` / `COMMANDLINEHIDE` | **部分实现（chrome 本地选择子）**：分别打开 Options / 特性面板 / 图层面板，或显示/隐藏命令行 | 不读写图纸；紧凑布局下**特性面板改为全宽底部抽屉**（不再拒绝），**图层面板**仍给既有拒绝句 |

### 2026-11-09 Wave 11 扩展命令（3D/网格/曲面拒绝）

> 3D 优先级低于 2D。以下名字只提供诚实命令路径（具名拒绝），不是可用 3D 功能；证据仅 Linux 编译 + 契约测试。

| 命令 | 结果 | 明示未实现 / 证据边界 |
| --- | --- | --- |
| `3DALIGN` | **部分实现（具名拒绝）**：`3dalign.unsupported: 3D alignment is not modeled; use the 2D ALIGN path` | 不写库/不升修订/不入历史 |
| `3DARRAY` | **部分实现（具名拒绝）**：`3darray.unsupported: 3D arrays are not modeled; use ARRAY/ARRAYRECT/ARRAYPOLAR` | 不写库/不升修订/不入历史 |
| `3DCLIP` | **部分实现（具名拒绝）**：`3dclip.unsupported: no 3D clipping-plane model` | 不写库/不升修订/不入历史 |
| `3DCORBIT` | **部分实现（具名拒绝）**：`3dcorbit.unsupported: no continuous 3D orbit controller` | 不写库/不升修订/不入历史 |
| `3DDISTANCE` | **部分实现（具名拒绝）**：`3ddistance.unsupported: no interactive 3D distance navigation` | 不写库/不升修订/不入历史 |
| `3DDWF` | **部分实现（具名拒绝）**：`3ddwf.unsupported: DWF/DWFx export is not implemented; use PLOT for SVG/PDF` | 不写库/不升修订/不入历史 |
| `3DEDITBAR` | **部分实现（具名拒绝）**：`3deditbar.unsupported: no NURBS/spline edit bar` | 不写库/不升修订/不入历史 |
| `3DFACE` | **部分实现（具名拒绝）**：`3dface.unsupported: no 3DFACE entity model` | 不写库/不升修订/不入历史 |
| `3DFLY` | **部分实现（具名拒绝）**：`3dfly.unsupported: no 3D fly navigation` | 不写库/不升修订/不入历史 |
| `3DFORBIT` | **部分实现（具名拒绝）**：`3dforbit.unsupported: no free 3D orbit controller` | 不写库/不升修订/不入历史 |
| `3DMESH` | **部分实现（具名拒绝）**：`3dmesh.unsupported: no free-form mesh creation model` | 不写库/不升修订/不入历史 |
| `3DMOVE` | **部分实现（具名拒绝）**：`3dmove.unsupported: no 3D move gizmo` | 不写库/不升修订/不入历史 |
| `3DORBIT` | **完整实现**：`ORBIT` 的别名，同一相机/拖动处理与拒绝路径 | 无第二导航引擎；`3DORBITCTR` 仍是独立具名拒绝 |
| `3DORBITCTR` | **部分实现（具名拒绝）**：`3dorbitctr.unsupported: no 3D orbit-center model` | 不写库/不升修订/不入历史 |
| `-3DOSNAP` | **部分实现（具名拒绝）**：`-3dosnap.unsupported: no 3D object-snap model` | 不写库/不升修订/不入历史 |
| `3DOSNAP` | **部分实现（具名拒绝）**：`3dosnap.unsupported: no 3D object-snap model` | 不写库/不升修订/不入历史 |
| `3DPAN` | **完整实现**：`PAN` 的别名，同一导航漏斗与拒绝路径 | 无第二导航引擎 |
| `3DPOLY` | **部分实现（具名拒绝）**：`3dpoly.unsupported: no 3D polyline entity model` | 不写库/不升修订/不入历史 |
| `3DPRINT` | **部分实现（具名拒绝）**：`3dprint.unsupported: no 3D print model` | 不写库/不升修订/不入历史 |
| `3DPRINTSERVICE` | **部分实现（具名拒绝）**：`3dprintservice.unsupported: no STL export or 3D print service` | 不写库/不升修订/不入历史 |
| `3DROTATE` | **部分实现（具名拒绝）**：`3drotate.unsupported: no 3D rotate gizmo` | 不写库/不升修订/不入历史 |
| `3DSCALE` | **部分实现（具名拒绝）**：`3dscale.unsupported: no 3D scale gizmo` | 不写库/不升修订/不入历史 |
| `3DSWIVEL` | **部分实现（具名拒绝）**：`3dswivel.unsupported: no 3D swivel navigation` | 不写库/不升修订/不入历史 |
| `3DWALK` | **部分实现（具名拒绝）**：`3dwalk.unsupported: no 3D walk navigation` | 不写库/不升修订/不入历史 |
| `3DZOOM` | **部分实现（具名拒绝）**：`3dzoom.unsupported: no perspective 3D zoom navigation` | 不写库/不升修订/不入历史 |

### 2026-11-09 Wave 12 扩展命令（文件/导入导出/扩展运行时拒绝）

> 唯一矢量输出仍是 `PLOT`（SVG/PDF）。以下名字只提供诚实命令路径（具名拒绝），不是可用导入导出/扩展功能；证据仅 Linux 编译 + 契约测试。

| 命令 | 结果 | 明示未实现 / 证据边界 |
| --- | --- | --- |
| `3DSIN` | **部分实现（具名拒绝）**：`3dsin.unsupported: 3DS import is not implemented` | 不写库/不升修订/不入历史 |
| `ACISIN` | **部分实现（具名拒绝）**：`acisin.unsupported: ACIS (SAT) import is not implemented` | 不写库/不升修订/不入历史 |
| `ACISOUT` | **部分实现（具名拒绝）**：`acisout.unsupported: ACIS (SAT) export is not implemented` | 不写库/不升修订/不入历史 |
| `BMPOUT` | **部分实现（具名拒绝）**：`bmpout.unsupported: raster BMP export is not implemented; use PLOT for vector SVG/PDF` | 不写库/不升修订/不入历史 |
| `JPGOUT` | **部分实现（具名拒绝）**：`jpgout.unsupported: raster JPEG export is not implemented; use PLOT for vector SVG/PDF` | 不写库/不升修订/不入历史 |
| `PNGOUT` | **部分实现（具名拒绝）**：`pngout.unsupported: raster PNG export is not implemented; use PLOT for vector SVG/PDF` | 不写库/不升修订/不入历史 |
| `TIFOUT` | **部分实现（具名拒绝）**：`tifout.unsupported: raster TIFF export is not implemented; use PLOT for vector SVG/PDF` | 不写库/不升修订/不入历史 |
| `WMFIN` | **部分实现（具名拒绝）**：`wmfin.unsupported: Windows metafile import is not implemented` | 不写库/不升修订/不入历史 |
| `WMFOUT` | **部分实现（具名拒绝）**：`wmfout.unsupported: Windows metafile export is not implemented` | 不写库/不升修订/不入历史 |
| `IGESIMPORT` | **部分实现（具名拒绝）**：`igesimport.unsupported: IGES import is not implemented` | 不写库/不升修订/不入历史 |
| `IGESEXPORT` | **部分实现（具名拒绝）**：`igesexport.unsupported: IGES export is not implemented` | 不写库/不升修订/不入历史 |
| `STLOUT` | **部分实现（具名拒绝）**：`stlout.unsupported: STL export is not implemented` | 不写库/不升修订/不入历史 |
| `IMPORT` | **部分实现（具名拒绝）**：`import.unsupported: the generic IMPORT picker is not implemented` | 不写库/不升修订/不入历史 |
| `EXPORTTOAUTOCAD` | **部分实现（具名拒绝）**：`exporttoautocad.unsupported: exporting to another AutoCAD version is not implemented` | 不写库/不升修订/不入历史 |
| `DWGCONVERT` | **部分实现（具名拒绝）**：`dwgconvert.unsupported: batch DWG conversion is not implemented` | 不写库/不升修订/不入历史 |
| `PLT2DWG` | **部分实现（具名拒绝）**：`plt2dwg.unsupported: HPGL (PLT) import is not implemented` | 不写库/不升修订/不入历史 |
| `DXBIN` | **部分实现（具名拒绝）**：`dxbin.unsupported: DXB import is not implemented` | 不写库/不升修订/不入历史 |
| `APPLOAD` | **部分实现（具名拒绝）**：`appload.unsupported: no plug-in/application loader model` | 不写库/不升修订/不入历史 |
| `ARX` | **部分实现（具名拒绝）**：`arx.unsupported: no ObjectARX loader model` | 不写库/不升修订/不入历史 |
| `NETLOAD` | **部分实现（具名拒绝）**：`netload.unsupported: no .NET application loader model` | 不写库/不升修订/不入历史 |
| `VBAIDE` | **部分实现（具名拒绝）**：`vbaide.unsupported: no VBA IDE model` | 不写库/不升修订/不入历史 |
| `VBALOAD` | **部分实现（具名拒绝）**：`vbaload.unsupported: no VBA loader model` | 不写库/不升修订/不入历史 |
| `VBARUN` | **部分实现（具名拒绝）**：`vbarun.unsupported: no VBA runtime` | 不写库/不升修订/不入历史 |
| `VBAUNLOAD` | **部分实现（具名拒绝）**：`vbaunload.unsupported: no VBA runtime` | 不写库/不升修订/不入历史 |
| `SCRIPT` | **部分实现（具名拒绝）**：`script.unsupported: no script-file runner` | 不写库/不升修订/不入历史 |

### 2026-11-09 Wave 13 扩展命令（页面设置/表格/字段/转换）

> 证据仅 Linux 编译 + 契约测试；GUI/GPU/像素/真机 NOT VERIFIED。

| 命令 | 结果 | 明示未实现 / 证据边界 |
| --- | --- | --- |
| `PAGESETUP` | **完整实现**：裸命令只读报告每个布局的入库 `PLOTSETTINGS`（纸张/旋转/比例/出图类型/provenance）；typed 形式写入 Authored 记录，未知布局/空纸名/非正尺寸/非正比例在写前按名拒绝，一条事务/一步撤销。 | 绘图仪/CTB/STB 属 ADR 全局范围；报告形式无写 |
| `-PAGESETUP` | **完整实现**：同一报告与 typed 写入的命令行拼写。 | 绘图仪/CTB/STB 属 ADR 全局范围；报告形式无写 |
| `PSETUPIN` | **部分实现（2026-11-24 更正）**：从**同一图纸**的源布局复制已存 `Authored` `PLOTSETTINGS` 到目标布局，一条事务/一步撤销；无源记录按 `psetupin.no_source` 拒绝，不合成 A4 | **缺口**：跨文件导入未实现——宿主 named-file 桥只能读字节，`cad-app` 没有从 DWG/DXF/DWT 或源文档快照提取页面设置的路径；无文件对话框 |
| `-PSETUPIN` | **部分实现（2026-11-24 从完整降回）**：`PSETUPIN` 的命令行拼写，同一实现臂（`page_setup_write`），一次 revision 复制**同一图纸内**源布局已存 `PLOTSETTINGS` | **缺口**：跨文件导入未实现（同 `PSETUPIN`）；无文件对话框 |
| `PLOTSTYLE` | **部分实现**：具名拒绝（`plotstyle.unsupported`）：命名打印样式未实现（无 CTB/STB 表）。 | 不写库/不升修订/不入历史（报告类只读） |
| `-PLOTSTYLE` | **部分实现**：具名拒绝（`-plotstyle.unsupported`）：打印样式列表的命令行拼写。 | 不写库/不升修订/不入历史（报告类只读） |
| `CONVERTPSTYLES` | **部分实现**：具名拒绝（`convertpstyles.unsupported`）：打印样式转换未实现（无 CTB/STB 模型）。 | 不写库/不升修订/不入历史（报告类只读） |
| `CONVERTCTB` | **部分实现**：具名拒绝（`convertctb.unsupported`）：CTB 转换未实现（无 CTB/STB 模型）。 | 不写库/不升修订/不入历史（报告类只读） |
| `CONVERTPOLY` | **部分实现**：只读报告（`convertpoly.single_representation`）：本构建只存一种 `LWPOLYLINE` 表示，无旧式多段线可转换。 | 不写库/不升修订/不入历史（报告类只读） |
| `CONVERT` | **部分实现**：具名拒绝（`convert.unsupported`）：通用对象转换未实现。 | 不写库/不升修订/不入历史（报告类只读） |
| `TABLE` | 部分实现 | 创建 + 导入模型可 MOVE/ROTATE/SCALE、FIND 逐格、真实 DXF 导出；逐格对齐/旋转/文字色/填充/逐边边框颜色与线宽、**逐格边距**、**逐边双线间距**都能存储、绘制、导出并可由 `TABLEDIT FORMAT|BORDERCOLOR|LINEWEIGHT`、`TABLESTYLE [ROW k]`、`MATCHCELL` 编辑；未设置时文件与几何逐字节不变 | 逐边框**线型**三重阻塞（LTYPE 句柄/实体写入器不发/`expand.rs` 覆盖片段线型）→ 具名拒绝；边框线宽渲染器只画 1px；块/FIELD 单元格、`TABLECONTENT` 名称描述、`annotative`、逐行边距缺；互操作：边距/间距挂在 `TABLECONTENT` 对象上 |
| `-TABLE` | **部分实现（真实创建）**：命令行拼写，与 `TABLE` 同实现 | 同 `TABLE` |
| `TABLEDIT` | 完整实现 | `TABLEDIT <row> <col> [text]`（1-based、选中一个表格、剩余整行文本、空文本清除）+ `MERGE <row> <col> <height> <width>` / `UNMERGE <row> <col>` / `STYLE <name>`；合并保留被覆盖单元格自身文本（诊断明示，不拼接），越界/覆盖锚点/重叠/未合并逐条具名拒绝；一条事务/一步撤销 | 无交互单元格对话框；块内容/字段单元格未建模（`TINSERT`/字段=全局） |
| `TABLEEXPORT` | 完整实现 | `TABLEEXPORT <path.csv>` 经宿主文件通道（`HostResources::write_named_file`）写已建模网格：RFC 4180、CRLF、合并区锚点+覆盖格留空、临时文件+rename、64 MiB 上限、不建目录；只读（无事务/无修订、Viewer 合法）；浏览器宿主 `unsupported-runtime: table-export` | 单元格块/字段内容未建模，故不导出；裸 IPC 形式 `tableexport.needs_path` |
| `TABLESTYLE` | 完整实现 | 实体样式记录（名称/说明/边距/**数据行/标题行/表头行三套**文字高与对齐/四条边框/标题与表头开关），经 `ACAD_TABLESTYLE` 三条行记录 DXF 往返；单元格按值相等继承；`TABLESTYLE [ROW k] …` 可只改一套；裸命令报告三套；一条事务/一步撤销 | 样式说明与样式级双线间距在本 writer 无对应组（声明）；单元格颜色/填充/线宽不在显示模型；无样式对话框 |
| `TINSERT` | **部分实现（拒绝）**：无块进入单元格的创建；导入单元格块内容未建模 | `tinsert.unsupported` |
| `MATCHCELL` | 完整实现 | `MATCHCELL <src_r> <src_c> <dst_r> <dst_c> […]` 复制源单元格的**完整逐格格式**：文字高、样式（名/id/字体）、四条边框、对齐（1–9）与文字旋转；文本、合并跨度、位置不动；一次重建/一条事务/一步撤销 | 逐格颜色/填充/边框线宽不在模型（与 `TABLE` 同一显示模型限制）；越界目标 `matchcell.out_of_range` |
| `FIELD` | **部分实现**：具名拒绝（`field.unsupported`）：字段未实现（无字段模型）。 | 不写库/不升修订/不入历史（报告类只读） |
| `RTEXT` | **部分实现**：具名拒绝（`rtext.unsupported`）：远程文字未实现（无 RTEXT 模型）。 | 不写库/不升修订/不入历史（报告类只读） |
| `RTEDIT` | **部分实现**：具名拒绝（`rtedit.unsupported`）：远程文字未实现（无 RTEXT 模型）。 | 不写库/不升修订/不入历史（报告类只读） |
| `SPELL` | **部分实现**：具名拒绝（`spell.unsupported`）：无拼写检查词典/模型。 | 不写库/不升修订/不入历史（报告类只读） |
| `SUPERHATCH` | **部分实现**：具名拒绝（`superhatch.unsupported`）：基于图像/块的超级填充未实现。 | 不写库/不升修订/不入历史（报告类只读） |
| `ALIGNSPACE` | 完整实现 | 键入形式为**八位小数**（两对模型↔图纸点，处理器需要两对推出比例）；构造与载荷相同的 `Points`，写 `model_to_paper`，一条事务/一步撤销 | 旋转按 `alignspace.rotation_ignored` 声明（视口模型无旋转字段）；需 Work 模式 + 图纸空间 + 可用视口 |
| `VPSYNC` | **完整实现**：`VPSYNC [<master>]` 把**0 基**主视口（默认 `0`）的 `model_to_paper` 复制到同布局其余视口；索引按视口数校验（越界 `vpsync.master_out_of_range`、不 clamp），主视口自身不改写；一条事务/一步撤销，已同步为 `vpsync.unchanged` 空操作 | 需 Work 模式 + 图纸空间 + 至少两个视口；无逐视口交互选择（索引来自命令行）；跨载体/像素 NOT VERIFIED |
| `LAYTRANS` | **部分实现**：具名拒绝（`laytrans.unsupported`）：图层转换标准未实现。 | 不写库/不升修订/不入历史（报告类只读） |

### 2026-11-09 / 2026-11-17 Wave 14 扩展命令（3D 实体图元与其余拒绝）

> **ADR-0036 阶段 C1（2026-11-17）**：`BOX`/`CONE`/`CYLINDER`/`SPHERE`/`TORUS`/`WEDGE`/`PYRAMID`
> 从具名拒绝晋升为**真实创建命令**：显式点 → `SolidRequest` → 模型器（`SolidModeler`）→
> 中性 B-Rep → 既有 `BrepTessellator` 显示缓存 → 一条 `AcDb3dSolid` 实体（一条事务/一步撤销）。
> **ADR-0036 阶段 C2（2026-11-17）**：`UNION`/`SUBTRACT`/`INTERSECT` 从具名拒绝晋升为**真实后台
> 任务**：命令只校验/拒绝并快照操作数（≥2 个整实体、同图层/同空间、`AcDb3dSolid` + 精确
> `Body3d{Solid, Exact}`），worker 折叠模型器结果并剖分，宿主 poll 到当前且未陈旧的结果后
> 在**一条事务/一步撤销**里删除操作数并插入一个 `AcDb3dSolid`；曲面操作数**也参与内核布尔**
> （P2.1b part 2 起曲面体可 lift；无 `(u,v)` box 或无精确 pcurve 的曲面面仍按名拒绝
> `solid.lift_unsupported`，声明 `nurbs: false` 的模型器答 `solid.nurbs_not_supported`），操作数在任务期间被编辑则整条结果按 `Stale`（状态 `solid.boolean_stale`，
> 不静默）丢弃；布尔运行期间图元创建按 `solid.boolean_busy` 拒绝。
> **ADR-0036 阶段 C3a/C3b/C3c（2026-11-17）**：`MASSPROP` 对精确 body 接线（体积/表面积/质心/
> `Ixx/Iyy/Izz/Iyz/Izx/Ixy`，模型器同步、density 1.0、请求相对精度 1e-6、显示网格不参与）、
> `face_sources` 由 `BrepTessellator` 写入 `brep:face:<shell>/<face>`、`VSCURRENT` 成为真实会话级
> 3D body 视觉样式（默认 SHADED）。**仍未实现**：AutoCAD 的交互式尺寸提示、圆角/抽壳、曲面**面显示**
> （P2.1c）与 `SURFACE` 家族（P2.2）；曲面 lift/曲面布尔自 **P2.1b（2026-11-17）** 起已可用，倒角
> 自 P2.0 起可用。**代码事实**：只有 Linux 的 **`native-below`**（原生 Wayland）
> 会话注入模型器；Linux `dom-canvas`（X11/XWayland，其 CAD 会话在 WebView 里是 wasm）、Web、
> Windows、macOS、Android **不注入**，七条图元命令与三条布尔命令都按
> `unsupported-runtime: solid-modeler` 具名拒绝；无模型器的载体上 `MASSPROP` 对 body 按
> `massprop.body_modeler_unavailable` 拒绝（不是 `unsupported-runtime`），`VSCURRENT` 只要会话状态、
> 各载体都接受（它只改 `Body3d` 显示）。
> **证据仅 Linux 契约/单元**（真实 solidcore，无 GPU/像素）；非 Linux 载体与 `dom-canvas` 的运行时
> 行为 **NOT VERIFIED**（仅编译/契约证据），不得写成可用。`3DSOLID`/`BODY` 仍是**实体类型键**，
> 不是命令。
> 其余名字依旧只提供诚实命令路径（具名拒绝），不是可用 3D 功能。

| 命令 | 结果 | 明示未实现 / 证据边界 |
| --- | --- | --- |
| `BOX` | **完整实现**：显式两点 + typed `Cube`/`Center`+corner/`Length`；plan 基面取会话 `ELEV`；一条事务/一步撤销 | 交互提示/布尔属 ADR 全局范围；无模型器载体 `unsupported-runtime: solid-modeler`；证据仅 Linux 契约（真实 solidcore），GPU/像素 NOT VERIFIED |
| `CONE` | **完整实现**：显式三点 + typed 底面 `Radius`/`Diameter`/`2P` + `Top radius`；plan 基面取会话 `ELEV`；上下半径相等按 `cone.equal_radii` 拒绝；一条事务/一步撤销 | 交互提示/布尔属 ADR 全局范围；无模型器载体 `unsupported-runtime: solid-modeler`；证据仅 Linux 契约，GPU/像素 NOT VERIFIED |
| `CYLINDER` | **完整实现**：显式三点 + typed 底面 `Radius`/`Diameter`/`2P`；plan 基面取会话 `ELEV`；一条事务/一步撤销 | 交互提示/布尔属 ADR 全局范围；无模型器载体 `unsupported-runtime: solid-modeler`；证据仅 Linux 契约，GPU/像素 NOT VERIFIED |
| `SPHERE` | **完整实现**：显式两点 + typed `Diameter`/`2P`（`2P` 保留自身 z）；一条事务/一步撤销 | 交互提示/布尔属 ADR 全局范围；无模型器载体 `unsupported-runtime: solid-modeler`；证据仅 Linux 契约，GPU/像素 NOT VERIFIED |
| `TORUS` | **完整实现**：显式三点 + typed plan ring `Radius`/`Diameter`/`2P` + tube `Radius`/`Diameter`/`2P`；plan 基面取会话 `ELEV`；次半径须小于主半径；一条事务/一步撤销 | 交互提示/布尔属 ADR 全局范围；无模型器载体 `unsupported-runtime: solid-modeler`；证据仅 Linux 契约，GPU/像素 NOT VERIFIED |
| `WEDGE` | **完整实现**：显式三点 + typed `Cube`/`Center`+corner/`Center`+`Length`/`Length`；plan 基面取会话 `ELEV`；高面固定在最小 x 一侧；一条事务/一步撤销 | 交互提示/布尔属 ADR 全局范围；无模型器载体 `unsupported-runtime: solid-modeler`；证据仅 Linux 契约，GPU/像素 NOT VERIFIED |
| `PYRAMID` | **部分实现**：显式三点（正方形底面对角点、顶点）创建 `AcDb3dSolid` 实体（`Body3d{Solid,Exact}` + 显示网格/边线）；底面须为正方形且两点共面（`pyramid.base_not_coplanar`），顶点须在底面中心正上方 | **未实现**：交互式尺寸提示、布尔运算（C2）；错误载荷 `InvalidInput`（`pyramid.requires_*_points`/`pyramid.degenerate_extent`）；无模型器载体 `unsupported-runtime: solid-modeler`；证据仅 Linux 契约（真实 solidcore），GPU/像素 NOT VERIFIED |
| `POLYSOLID` | **部分实现（具名拒绝）**：`polysolid.unsupported: 3D solid primitives are not implemented` | 不写库/不升修订/不入历史 |
| `EXTRUDE` | **部分实现（具名拒绝）**：`extrude.unsupported: 3D extrusion is not implemented` | 不写库/不升修订/不入历史 |
| `REVOLVE` | **部分实现（具名拒绝）**：`revolve.unsupported: 3D revolution is not implemented` | 不写库/不升修订/不入历史 |
| `SWEEP` | **部分实现（具名拒绝）**：`sweep.unsupported: 3D sweep is not implemented` | 不写库/不升修订/不入历史 |
| `LOFT` | **部分实现（具名拒绝）**：`loft.unsupported: 3D loft is not implemented` | 不写库/不升修订/不入历史 |
| `INTERSECT` | **完整实现（后台布尔，C2，单壳结果）**：选中 ≥2 个平面或曲面 `AcDb3dSolid` 实体 → worker 左折叠 → poll 在一条事务/一步撤销里删除操作数并插入交集实体；提交前校验结果为单个连通非空壳，多壳/空壳按 `solid.boolean_result_not_single_shell` 拒绝且不写库/不升修订/不退操作数 | 曲面操作数参与内核布尔（P2.1b part 2）；无精确 pcurve / 无 `(u,v)` box 的曲面面仍按名拒绝（`solid.lift_unsupported`），`nurbs: false` 的模型器答 `solid.nurbs_not_supported`；`boolean.needs_two`/`boolean.non_solid_operand`/`boolean.sub_element_not_supported`/`boolean.layer_mismatch`/`boolean.space_mismatch`；操作数被编辑 → `Stale`（图纸不变）；无模型器 → `unsupported-runtime: solid-modeler`；证据仅 Linux 契约（真实 solidcore），GPU/像素 NOT VERIFIED |
| `UNION` | **完整实现（后台布尔，C2，单壳结果）**：选中 ≥2 个平面或曲面 `AcDb3dSolid` 实体 → worker 左折叠 → poll 提交一个合并实体（一条事务/一步撤销）；提交前校验结果为单个连通非空壳，多壳/空壳按 `solid.boolean_result_not_single_shell` 拒绝且不写库/不升修订/不退操作数 | 同上：曲面操作数参与内核布尔（P2.1b）、无精确 pcurve/box 的曲面面仍按名拒绝、具名校验码、`Stale` 丢弃（`solid.boolean_stale` 状态）、`solid.boolean_busy` 拒绝、无模型器拒绝；证据仅 Linux 契约，GPU/像素 NOT VERIFIED |
| `SUBTRACT` | **完整实现（后台布尔，C2，单壳结果）**：第一个选中实体减去其余实体 → poll 提交差集实体（一条事务/一步撤销）；操作数可为平面或曲面体（P2.1b）；提交前校验结果为单个连通非空壳，多壳/空壳按 `solid.boolean_result_not_single_shell` 拒绝且不写库/不升修订/不退操作数 | 同上：曲面操作数参与内核布尔、无精确 pcurve/box 的曲面面仍按名拒绝、其余具名校验码、`Stale` 丢弃（`solid.boolean_stale` 状态）、`solid.boolean_busy` 拒绝、无模型器拒绝；证据仅 Linux 契约，GPU/像素 NOT VERIFIED |
| `SLICE` | **部分实现（具名拒绝）**：`slice.unsupported: no 3D slicing model` | 不写库/不升修订/不入历史 |
| `THICKEN` | **部分实现（具名拒绝）**：`thicken.unsupported: no surface-to-solid model` | 不写库/不升修订/不入历史 |
| `PRESSPULL` | **部分实现（具名拒绝）**：`presspull.unsupported: no press/pull model` | 不写库/不升修订/不入历史 |
| `IMPRINT` | **部分实现（具名拒绝）**：`imprint.unsupported: no 3D imprint model` | 不写库/不升修订/不入历史 |
| `OFFSETEDGE` | **部分实现（具名拒绝）**：`offsetedge.unsupported: no 3D face/edge offset model` | 不写库/不升修订/不入历史 |
| `CHAMFEREDGE` | **可达（P2.0 接线，2026-11-17）**：由 Linux `native-below` 渲染栈的宿主工具接线——当前选择须为**恰好一个**精确 `AcDb3dSolid` 实体（面引用折叠到所属 body，实例/块引用拒绝），随后一次边拾取提供内核的最近边种子（`PickHit.point`，来自既有像素拾取漏斗，**绝不是**工作平面投影）；边选择归内核（`max(1e-9, 1e-3 × 包围盒对角线)`），一次一条边、边选择只认**内核 record 里的真实共享边**（当前只有 `Line` 边可测距、可被选中）；写入方为闭合曲面重建的**合成 seam**（一条边的两个 use 落在**同一个面**上）会被跳过，所以点在圆柱 rim 上得到如实的 **`solid.edge_not_found`**（诊断点名被跳过的 seam），**不是** `solid.chamfer_failed`。两个**不同**面共享的光滑 seam 仍会答 `solid.chamfer_failed`（“no blend (Smooth)”），但本构建没有任何可构造的 body 会产生这种边，因此该路径只在**谓词级**被钉住；曲面**边**倒角是 P2.2。退距取 `chamferDistanceA/B`（两者都须 `> 0`；通道默认 `0/0` 是 2D `CHAMFER` 的合法值，首次回车即 `chamferedge.distance_invalid` 且**保留拾取**；参数与 2D `CHAMFER` 共享，CHAMFEREDGE 总是把两个值都当 3D 退距）。原地改写：同一实体 id、一次 revision、一步撤销 | 无子边身份、无边高亮、无预览、无 `FILLETEDGE`/链式/多边/NURBS 圆角；非模型空间、非精确 payload、选择变化均按名拒绝（`chamferedge.*`），内核答复原样透传（`solid.edge_not_found`/`solid.chamfer_failed`/`solid.kernel_failed`）。**证据**：合成 `cad-app`/`cad-host` 契约 + 真实内核 `solidcore` 契约（Linux x86-64，`cad-cli-tools/tests/solid_command.rs`）；无 GPU/像素/性能/非 Linux 载体证据 |
| `FILLETEDGE` | **部分实现（具名拒绝，仍不可达）**：`filletedge.unsupported: no 3D edge fillet model`。模型器 `chamfer` 就绪不等于圆角就绪：`ModelerCapabilities.fillet` 仍为 `false`，`SolidOperation` 没有 `Fillet` 变体。**内核侧已实测并接通读取**：pin `55ae49e` 的 `blend_ordered(Blend::Round, 一条全长直边)` 产出 `Surface::Cone{half_angle: 0}` + `Face.nurbs = RationalSpline`（3×2、权重含 `√2/2`、确实有理），**不是** `Blend` 标签；读取端把**恰好**零斜率的 cone 映射成 `BrepSurface::Cylinder` 后，圆角体能 lift、质量属性给出闭式体积（相对 1e-10）、布尔并回写（真实内核，`cad-kernel-solidcore/tests/fillet.rs`）。角部（两条圆角相接）与 `Blend::Curvature`（需要 NURBS **曲线**族）仍未承载 | 不写库/不升修订/不入历史 |
| `EDGE` | **部分实现（具名拒绝）**：`edge.unsupported: no 3D face-edge visibility model` | 不写库/不升修订/不入历史 |
| `MIRROR3D` | **部分实现（具名拒绝）**：`mirror3d.unsupported: no 3D mirror-plane model` | 不写库/不升修订/不入历史 |
| `ROTATE3D` | **部分实现（具名拒绝）**：`rotate3d.unsupported: no 3D rotate-axis model` | 不写库/不升修订/不入历史 |

### 2026-11-09 Wave 15 扩展命令（曲面/网格拒绝）

> 3D 低于 2D；以下名字只提供诚实命令路径（具名拒绝），不是可用曲面/网格功能。

| 命令 | 结果 | 明示未实现 / 证据边界 |
| --- | --- | --- |
| `SURFBLEND` | **部分实现（具名拒绝）**：`surfblend.unsupported: surface blending is not implemented` | 不写库/不升修订/不入历史 |
| `SURFEXTEND` | **部分实现（具名拒绝）**：`surfextend.unsupported: surface extension is not implemented` | 不写库/不升修订/不入历史 |
| `SURFEXTRACTCURVE` | **部分实现（具名拒绝）**：`surfextractcurve.unsupported: surface curve extraction is not implemented` | 不写库/不升修订/不入历史 |
| `SURFFILLET` | **部分实现（具名拒绝）**：`surffillet.unsupported: surface filleting is not implemented` | 不写库/不升修订/不入历史 |
| `SURFNETWORK` | **部分实现（具名拒绝）**：`surfnetwork.unsupported: surface network creation is not implemented` | 不写库/不升修订/不入历史 |
| `SURFOFFSET` | **部分实现（具名拒绝）**：`surfoffset.unsupported: surface offset is not implemented` | 不写库/不升修订/不入历史 |
| `SURFPATCH` | **部分实现（具名拒绝）**：`surfpatch.unsupported: surface patching is not implemented` | 不写库/不升修订/不入历史 |
| `SURFSCULPT` | **部分实现（具名拒绝）**：`surfsculpt.unsupported: surface sculpting is not implemented` | 不写库/不升修订/不入历史 |
| `SURFTRIM` | **部分实现（具名拒绝）**：`surftrim.unsupported: surface trimming is not implemented` | 不写库/不升修订/不入历史 |
| `SURFUNTRIM` | **部分实现（具名拒绝）**：`surfuntrim.unsupported: surface untrimming is not implemented` | 不写库/不升修订/不入历史 |
| `MESH` | **部分实现（具名拒绝）**：`mesh.unsupported: free-form mesh creation is not implemented` | 不写库/不升修订/不入历史 |
| `MESHCAP` | **部分实现（具名拒绝）**：`meshcap.unsupported: mesh capping is not implemented` | 不写库/不升修订/不入历史 |
| `MESHCOLLAPSE` | **部分实现（具名拒绝）**：`meshcollapse.unsupported: mesh collapse is not implemented` | 不写库/不升修订/不入历史 |
| `MESHCREASE` | **部分实现（具名拒绝）**：`meshcrease.unsupported: mesh creasing is not implemented` | 不写库/不升修订/不入历史 |
| `MESHEXTRUDE` | **部分实现（具名拒绝）**：`meshextrude.unsupported: mesh extrusion is not implemented` | 不写库/不升修订/不入历史 |
| `MESHMERGE` | **部分实现（具名拒绝）**：`meshmerge.unsupported: mesh merge is not implemented` | 不写库/不升修订/不入历史 |
| `MESHOPTIONS` | **部分实现（具名拒绝）**：`meshoptions.unsupported: no mesh tessellation options model` | 不写库/不升修订/不入历史 |
| `MESHPRIMITIVEOPTIONS` | **部分实现（具名拒绝）**：`meshprimitiveoptions.unsupported: no mesh primitive options model` | 不写库/不升修订/不入历史 |
| `MESHREFINE` | **部分实现（具名拒绝）**：`meshrefine.unsupported: mesh refinement is not implemented` | 不写库/不升修订/不入历史 |
| `MESHSMOOTH` | **部分实现（具名拒绝）**：`meshsmooth.unsupported: mesh smoothing conversion is not implemented` | 不写库/不升修订/不入历史 |
| `MESHSMOOTHLESS` | **部分实现（具名拒绝）**：`meshsmoothless.unsupported: no mesh smoothness level model` | 不写库/不升修订/不入历史 |
| `MESHSMOOTHMORE` | **部分实现（具名拒绝）**：`meshsmoothmore.unsupported: no mesh smoothness level model` | 不写库/不升修订/不入历史 |
| `MESHSPIN` | **部分实现（具名拒绝）**：`meshspin.unsupported: mesh edge spin is not implemented` | 不写库/不升修订/不入历史 |
| `MESHSPLIT` | **部分实现（具名拒绝）**：`meshsplit.unsupported: mesh face split is not implemented` | 不写库/不升修订/不入历史 |
| `MESHUNCREASE` | **部分实现（具名拒绝）**：`meshuncrease.unsupported: mesh uncreasing is not implemented` | 不写库/不升修订/不入历史 |

### 2026-11-09 Wave 16 扩展命令（图像/PDF/DGN 底图拒绝）

> 以下名字只提供诚实命令路径（具名拒绝），不是可用底图功能。

| 命令 | 结果 | 明示未实现 / 证据边界 |
| --- | --- | --- |
| `IMAGE` | **部分实现（只读报告）**：列出所有 IMAGE 实体的 id/空间/file key/pixels/visible/clip/origin（`image.entry`/`image.report`，空图 `image.none`）；Viewer-legal | 无 External References 面板/重载/卸载；不写库 |
| `-IMAGE` | **部分实现（只读报告）**：`-IMAGE` 命令行拼写，`-image.*` 诊断码；无写 | 同上，命令行拼写 |
| `IMAGEATTACH` | **完整实现（真实写入）**：`IMAGEATTACH <file> <x> <y> <w_px> <h_px> <units_per_pixel> [<rotation_deg>]` 创建真实 IMAGE 实体；逻辑 key 规范化（控制符/`..`/绝对路径拒绝）；可选绕 `+Z` 逆时针旋转烘进 `u`/`v` 基（0° 逐字节不变）；目标是会话活动空间——模型空间走共享创建漏斗，图纸布局走该布局自身 draw-order 的实体写入路径（`imageattach.block_space`/`.unknown_layout`/`.invalid_rotation` 按名拒绝）；一条事务/一步撤销；宿主在提交后按 key 集变化重载纹理，大小写敏感文件系统逐段不敏感回退 | 无对话框/文件选择器；未解析 key 显示边框并进入宿主图像报告；web carrier 显式拒绝（`unsupported-runtime: images`）；跨载体纹理/GPU 证据 NOT VERIFIED |
| `IMAGECLIP` | **完整实现（真实写入）**：`<minx> <miny> <maxx> <maxy>` / `POLY <x1> <y1> …` / `OFF`；写 `ImageClip{inside:true}`，一条事务/一步撤销；裁剪多边形与 `[0,pixels]` 求交（全在外→`image.clip_unsupported`），x 沿 u、y 沿 v（自下而上）；非凸环在表示层耳切成凸片段逐块绘制 | **全局范围**：`inside=false`（补集裁剪）未授权；无交互拖拽裁剪顶点；自交/退化按 `image.clip_*` 具名拒绝 |
| `IMAGEADJUST` | **部分实现（真实写入）**：选中一个 IMAGE，`IMAGEADJUST <brightness> <contrast> <fade> [MONO|COLOR]` / `RESET`（0..=100，50/50/0 中性），一条事务/一步撤销；渲染 mono→contrast→brightness→fade-to-background | 无对话框；导出仍整体丢弃图像（含这些参数）为声明损失；跨载体/GPU 像素 NOT VERIFIED |
| `-IMAGEADJUST` | **部分实现（真实写入）**：选中一个 IMAGE，`-IMAGEADJUST <brightness> <contrast> <fade> [MONO|COLOR]` / `RESET`（0..=100，50/50/0 中性），一条事务/一步撤销；渲染 mono→contrast→brightness→fade-to-background | 无对话框；导出仍整体丢弃图像（含这些参数）为声明损失；跨载体/GPU 像素 NOT VERIFIED |
| `IMAGEQUALITY` | **部分实现（具名拒绝）**：`imagequality.unsupported: no image display-quality model` | 不写库/不升修订/不入历史 |
| `IMAGEAPP` | **部分实现（具名拒绝）**：`imageapp.unsupported: no external image-editor association model` | 不写库/不升修订/不入历史 |
| `IMAGEASYNCWAIT` | **部分实现（具名拒绝）**：`imageasyncwait.unsupported: no asynchronous image loading model` | 不写库/不升修订/不入历史 |
| `IMAGEEDIT` | **部分实现（具名拒绝）**：`imageedit.unsupported: no external image-editor model` | 不写库/不升修订/不入历史 |
| `CLASSICIMAGE` | **部分实现（具名拒绝）**：`classicimage.unsupported: no image manager; use IMAGE` | 不写库/不升修订/不入历史 |
| `PDFATTACH` | **部分实现（具名拒绝）**：`pdfattach.unsupported: PDF underlays are not implemented` | 不写库/不升修订/不入历史 |
| `-PDFATTACH` | **部分实现（具名拒绝）**：`-pdfattach.unsupported: PDF underlays are not implemented` | 不写库/不升修订/不入历史 |
| `PDFCLIP` | **部分实现（具名拒绝）**：`pdfclip.unsupported: no PDF underlay clip model` | 不写库/不升修订/不入历史 |
| `PDFLAYERS` | **部分实现（具名拒绝）**：`pdflayers.unsupported: no PDF underlay layer model` | 不写库/不升修订/不入历史 |
| `PDFADJUST` | **部分实现（具名拒绝）**：`pdfadjust.unsupported: no PDF underlay adjustment model` | 不写库/不升修订/不入历史 |
| `PDFIMPORT` | **部分实现（具名拒绝）**：`pdfimport.unsupported: PDF geometry import is not implemented` | 不写库/不升修订/不入历史 |
| `-PDFIMPORT` | **部分实现（具名拒绝）**：`-pdfimport.unsupported: PDF geometry import is not implemented` | 不写库/不升修订/不入历史 |
| `PDFSHXTEXT` | **部分实现（具名拒绝）**：`pdfshxtext.unsupported: PDF SHX text conversion is not implemented` | 不写库/不升修订/不入历史 |
| `-PDFSHXTEXT` | **部分实现（具名拒绝）**：`-pdfshxtext.unsupported: PDF SHX text conversion is not implemented` | 不写库/不升修订/不入历史 |
| `DGNATTACH` | **部分实现（具名拒绝）**：`dgnattach.unsupported: DGN underlays are not implemented` | 不写库/不升修订/不入历史 |
| `-DGNATTACH` | **部分实现（具名拒绝）**：`-dgnattach.unsupported: DGN underlays are not implemented` | 不写库/不升修订/不入历史 |
| `DGNCLIP` | **部分实现（具名拒绝）**：`dgnclip.unsupported: no DGN underlay clip model` | 不写库/不升修订/不入历史 |
| `DGNLAYERS` | **部分实现（具名拒绝）**：`dgnlayers.unsupported: no DGN underlay layer model` | 不写库/不升修订/不入历史 |
| `DGNADJUST` | **部分实现（具名拒绝）**：`dgnadjust.unsupported: no DGN underlay adjustment model` | 不写库/不升修订/不入历史 |

### 2026-11-10 Wave 17-24 扩展命令（200 条具名拒绝）

> 以下 200 个名字只提供诚实命令路径（`CadError::Unsupported` + 稳定原因串），
> 不是可用功能：本构建没有对应的模型/API（动作录制、面板、OLE/超链接、LISP、
> 灯光/材质/约束、数据提取、DWF/DGN 底图、标记协作、GIS、比较、图纸集等）。
> 全部 Viewer-legal：不写库、不升修订、不入历史。唯一权威是
> `docs/autocad-2026-command-coverage.md`；此表只复述原因串。

| 命令 | 结果 | 明示未实现 / 证据边界 |
| --- | --- | --- |
| `ACTBASEPOINT` | **部分实现（具名拒绝）**：`actbasepoint.unsupported: action-recorder macros are not modeled (no recorder state, file, or playback)` | 不写库/不升修订/不入历史 |
| `ACTIVITYINSIGHTSCLOSE` | **部分实现（具名拒绝）**：`activityinsightsclose.unsupported: this build has no Activity Insights data model` | 不写库/不升修订/不入历史 |
| `ACTIVITYINSIGHTSOPEN` | **部分实现（具名拒绝）**：`activityinsightsopen.unsupported: this build has no Activity Insights data model` | 不写库/不升修订/不入历史 |
| `ACTMANAGER` | **完整实现**：Manage→Action Recorder 面板 + `LIST/PLAY/DELETE/CONTINUE`；回放逐步经正常命令漏斗，失败报步骤号 | `DELETE` 只删会话条目、不删磁盘文件；无真实文件对话框/GUI 证据 |
| `ACTRECORD` | **完整实现**：开始会话宏录制，只捕获成功执行的非控制命令；typed 目标路径经宿主文件桥 | 无文件桥/写失败按 `unsupported-runtime: action-recorder` 拒绝；无 GUI/文件对话框证据 |
| `-ACTSTOP` | **完整实现**：`ACTSTOP` 的命令行拼写，同一实现 | 同上 |
| `ACTSTOP` | **完整实现**：停止录制并写版本化 JSON 宏 | 无文件桥/写失败按名拒绝；`ACTMANAGER DELETE` 不删磁盘文件 |
| `ACTUSERINPUT` | **完整实现**：记录输入暂停步骤，回放等待 `ACTMANAGER CONTINUE` | 回放输入仍是用户动作，不伪造指针事件 |
| `-ACTUSERMESSAGE` | **完整实现**：`ACTUSERMESSAGE` 的命令行拼写，同一实现 | 同上 |
| `ACTUSERMESSAGE` | **完整实现**：记录消息步骤并由回放显示/暂停 | 不执行任意脚本 |
| `ADCCLOSE` | **部分实现（具名拒绝）**：`adcclose.unsupported: this build has no DesignCenter content palette` | 不写库/不升修订/不入历史 |
| `ADCENTER` | **部分实现（具名拒绝）**：`adcenter.unsupported: this build has no DesignCenter content palette` | 不写库/不升修订/不入历史 |
| `ADCNAVIGATE` | **部分实现（具名拒绝）**：`adcnavigate.unsupported: this build has no DesignCenter content palette` | 不写库/不升修订/不入历史 |
| `ADJUST` | **部分实现（具名拒绝）**：`adjust.unsupported: this build has no underlay/image fade-contrast-monochrome model` | 不写库/不升修订/不入历史 |
| `AMECONVERT` | **部分实现（具名拒绝）**：`ameconvert.unsupported: this build has no AME solid model` | 不写库/不升修订/不入历史 |
| `ANALYSISCURVATURE` | **部分实现（具名拒绝）**：`analysiscurvature.unsupported: this build has no 3D curvature/draft/zebra analysis display` | 不写库/不升修订/不入历史 |
| `ANALYSISDRAFT` | **部分实现（具名拒绝）**：`analysisdraft.unsupported: this build has no 3D curvature/draft/zebra analysis display` | 不写库/不升修订/不入历史 |
| `ANALYSISOPTIONS` | **部分实现（具名拒绝）**：`analysisoptions.unsupported: this build has no 3D curvature/draft/zebra analysis display` | 不写库/不升修订/不入历史 |
| `ANALYSISZEBRA` | **部分实现（具名拒绝）**：`analysiszebra.unsupported: this build has no 3D curvature/draft/zebra analysis display` | 不写库/不升修订/不入历史 |
| `ANIPATH` | **部分实现（具名拒绝）**：`anipath.unsupported: this build has no camera-animation model` | 不写库/不升修订/不入历史 |
| `APPAUTOLOADER` | **部分实现（具名拒绝）**：`appautoloader.unsupported: this build has no plug-in autoloader registry` | 不写库/不升修订/不入历史 |
| `APPSTORE` | **部分实现（具名拒绝）**：`appstore.unsupported: the host owns this browser/store/chrome action` | 不写库/不升修订/不入历史 |
| `ARRAYPATH` | **部分实现（真实）**：当前选择第一个实体为路径、其余为项目；等弧长 `ARRAYPATH <count> [ALIGN]`；一条事务/一步撤销 | 无关联性/持久路径参数/项目基点模型；ALIGN 是切向角近似的声明式规则 |
| `ASSISTANTCLOSE` | **部分实现（具名拒绝）**：`assistantclose.unsupported: this build has no Autodesk Assistant model` | 不写库/不升修订/不入历史 |
| `ASSISTANTOPEN` | **部分实现（具名拒绝）**：`assistantopen.unsupported: this build has no Autodesk Assistant model` | 不写库/不升修订/不入历史 |
| `ATTACHURL` | **部分实现（具名拒绝）**：`attachurl.unsupported: this build has no hyperlink storage model` | 不写库/不升修订/不入历史 |
| `BDETECT` | **部分实现（具名拒绝）**：`bdetect.unsupported: this build has no object-detection model` | 不写库/不升修订/不入历史 |
| `BLOCKICON` | **部分实现（具名拒绝）**：`blockicon.unsupported: this build has no block thumbnail/preview model` | 不写库/不升修订/不入历史 |
| `BLOCKSDATAOPTION` | **部分实现（具名拒绝）**：`blocksdataoption.unsupported: this build has no data-collection consent dialog model` | 不写库/不升修订/不入历史 |
| `BLOCKSPALETTE` | **部分实现（具名拒绝）**：`blockspalette.unsupported: this build has no Blocks palette; block insertion is the INSERT path` | 不写库/不升修订/不入历史 |
| `BLOCKSPALETTECLOSE` | **部分实现（具名拒绝）**：`blockspaletteclose.unsupported: this build has no Blocks palette; block insertion is the INSERT path` | 不写库/不升修订/不入历史 |
| `BLOCKTOXREF` | **部分实现（具名拒绝）**：`blocktoxref.unsupported: block-to-xref replacement needs a writable block table and xref attachment, neither modeled` | 不写库/不升修订/不入历史 |
| `BREP` | **部分实现（具名拒绝）**：`brep.unsupported: this build has no 3D solid history/brep model` | 不写库/不升修订/不入历史 |
| `BROWSER` | **部分实现（具名拒绝）**：`browser.unsupported: the host owns this browser/store/chrome action` | 不写库/不升修订/不入历史 |
| `BSEARCH` | **部分实现（具名拒绝）**：`bsearch.unsupported: this build has no convert-to-block search model` | 不写库/不升修订/不入历史 |
| `CAMERA` | **部分实现（具名拒绝）**：`camera.unsupported: this build has no 3D camera/target view model` | 不写库/不升修订/不入历史 |
| `CENTERDISASSOCIATE` | **部分实现（具名拒绝）**：`centerdisassociate.unsupported: center marks/centerlines are not modeled, so associativity/reset has nothing to edit` | 不写库/不升修订/不入历史 |
| `CENTERREASSOCIATE` | **部分实现（具名拒绝）**：`centerreassociate.unsupported: center marks/centerlines are not modeled, so associativity/reset has nothing to edit` | 不写库/不升修订/不入历史 |
| `CENTERRESET` | **部分实现（具名拒绝）**：`centerreset.unsupported: center marks/centerlines are not modeled, so associativity/reset has nothing to edit` | 不写库/不升修订/不入历史 |
| `CHECKSTANDARDS` | **部分实现（具名拒绝）**：`checkstandards.unsupported: this build has no CAD standards (DWS) model` | 不写库/不升修订/不入历史 |
| `CHURLS` | **部分实现（具名拒绝）**：`churls.unsupported: this build has no hyperlink storage model` | 不写库/不升修订/不入历史 |
| `COMMANDMACROS` | **部分实现（具名拒绝）**：`commandmacros.unsupported: this build has no command-macro palette model` | 不写库/不升修订/不入历史 |
| `COMMANDMACROSCLOSE` | **部分实现（具名拒绝）**：`commandmacrosclose.unsupported: this build has no command-macro palette model` | 不写库/不升修订/不入历史 |
| `-COMPARE` | **部分实现（具名拒绝）**：`-compare.unsupported: this build has no drawing-compare/diff model` | 不写库/不升修订/不入历史 |
| `COMPARE` | **部分实现（具名拒绝）**：`compare.unsupported: this build has no drawing-compare/diff model` | 不写库/不升修订/不入历史 |
| `COMPARECLOSE` | **部分实现（具名拒绝）**：`compareclose.unsupported: this build has no drawing-compare/diff model` | 不写库/不升修订/不入历史 |
| `COMPAREEXPORT` | **部分实现（具名拒绝）**：`compareexport.unsupported: this build has no drawing-compare/diff model` | 不写库/不升修订/不入历史 |
| `COMPAREIMPORT` | **部分实现（具名拒绝）**：`compareimport.unsupported: this build has no drawing-compare/diff model` | 不写库/不升修订/不入历史 |
| `COMPAREINFO` | **部分实现（具名拒绝）**：`compareinfo.unsupported: this build has no drawing-compare/diff model` | 不写库/不升修订/不入历史 |
| `COMPILE` | **部分实现（具名拒绝）**：`compile.unsupported: this build has no SHX shape/font compiler or loader` | 不写库/不升修订/不入历史 |
| `CONVERTOLDLIGHTS` | **部分实现（具名拒绝）**：`convertoldlights.unsupported: this build has no legacy light/material conversion model` | 不写库/不升修订/不入历史 |
| `CONVERTOLDMATERIALS` | **部分实现（具名拒绝）**：`convertoldmaterials.unsupported: this build has no legacy light/material conversion model` | 不写库/不升修订/不入历史 |
| `CONVTOMESH` | **部分实现（具名拒绝）**：`convtomesh.unsupported: 3D solids, surfaces and meshes are not modeled` | 不写库/不升修订/不入历史 |
| `CONVTONURBS` | **部分实现（具名拒绝）**：`convtonurbs.unsupported: 3D solids, surfaces and meshes are not modeled` | 不写库/不升修订/不入历史 |
| `CONVTOSOLID` | **部分实现（具名拒绝）**：`convtosolid.unsupported: 3D solids, surfaces and meshes are not modeled` | 不写库/不升修订/不入历史 |
| `CONVTOSURFACE` | **部分实现（具名拒绝）**：`convtosurface.unsupported: 3D solids, surfaces and meshes are not modeled` | 不写库/不升修订/不入历史 |
| `COORDINATIONMODELATTACH` | **部分实现（具名拒绝）**：`coordinationmodelattach.unsupported: this build has no Navisworks coordination-model attachment model` | 不写库/不升修订/不入历史 |
| `COPYFROMTRACE` | **部分实现（具名拒绝）**：`copyfromtrace.unsupported: this build has no collaboration trace model` | 不写库/不升修订/不入历史 |
| `COPYLINK` | **部分实现（具名拒绝）**：`copylink.unsupported: OLE/clipboard linking is owned by the host` | 不写库/不升修订/不入历史 |
| `COPYM` | **部分实现（具名拒绝）**：`copym.unsupported: this build has no multi-copy (Repeat/Array/Divide/Measure) session model` | 不写库/不升修订/不入历史 |
| `COUNTAREA` | **部分实现（具名拒绝）**：`countarea.unsupported: this build has no object-count session model` | 不写库/不升修订/不入历史 |
| `COUNTAREACLOSE` | **部分实现（具名拒绝）**：`countareaclose.unsupported: this build has no object-count session model` | 不写库/不升修订/不入历史 |
| `COUNTCLOSE` | **部分实现（具名拒绝）**：`countclose.unsupported: this build has no object-count session model` | 不写库/不升修订/不入历史 |
| `COUNTFIELD` | **部分实现（具名拒绝）**：`countfield.unsupported: this build has no object-count session model` | 不写库/不升修订/不入历史 |
| `COUNTLIST` | **部分实现（具名拒绝）**：`countlist.unsupported: this build has no object-count session model` | 不写库/不升修订/不入历史 |
| `COUNTLISTCLOSE` | **部分实现（具名拒绝）**：`countlistclose.unsupported: this build has no object-count session model` | 不写库/不升修订/不入历史 |
| `COUNTNAVNEXT` | **部分实现（具名拒绝）**：`countnavnext.unsupported: this build has no object-count session model` | 不写库/不升修订/不入历史 |
| `COUNTNAVPREV` | **部分实现（具名拒绝）**：`countnavprev.unsupported: this build has no object-count session model` | 不写库/不升修订/不入历史 |
| `COUNTTABLE` | **部分实现（具名拒绝）**：`counttable.unsupported: this build has no object-count session model` | 不写库/不升修订/不入历史 |
| `CUI` | **部分实现（具名拒绝）**：`cui.unsupported: this build has no CUIx customization model` | 不写库/不升修订/不入历史 |
| `CUIEXPORT` | **部分实现（具名拒绝）**：`cuiexport.unsupported: this build has no CUIx customization model` | 不写库/不升修订/不入历史 |
| `CUIIMPORT` | **部分实现（具名拒绝）**：`cuiimport.unsupported: this build has no CUIx customization model` | 不写库/不升修订/不入历史 |
| `CUILOAD` | **部分实现（具名拒绝）**：`cuiload.unsupported: this build has no CUIx customization model` | 不写库/不升修订/不入历史 |
| `CUIUNLOAD` | **部分实现（具名拒绝）**：`cuiunload.unsupported: this build has no CUIx customization model` | 不写库/不升修订/不入历史 |
| `CUSTOMIZE` | **部分实现（具名拒绝）**：`customize.unsupported: this build has no tool-palette customization model` | 不写库/不升修订/不入历史 |
| `CVADD` | **部分实现（具名拒绝）**：`cvadd.unsupported: NURBS control-vertex editing is not modeled` | 不写库/不升修订/不入历史 |
| `CVHIDE` | **部分实现（具名拒绝）**：`cvhide.unsupported: NURBS control-vertex editing is not modeled` | 不写库/不升修订/不入历史 |
| `CVREBUILD` | **部分实现（具名拒绝）**：`cvrebuild.unsupported: NURBS control-vertex editing is not modeled` | 不写库/不升修订/不入历史 |
| `CVREMOVE` | **部分实现（具名拒绝）**：`cvremove.unsupported: NURBS control-vertex editing is not modeled` | 不写库/不升修订/不入历史 |
| `CVSHOW` | **部分实现（具名拒绝）**：`cvshow.unsupported: NURBS control-vertex editing is not modeled` | 不写库/不升修订/不入历史 |
| `-DATAEXTRACTION` | **部分实现（具名拒绝）**：`-dataextraction.unsupported: this build has no data-extraction/attribute-template model` | 不写库/不升修订/不入历史 |
| `DATAEXTRACTION` | **部分实现（具名拒绝）**：`dataextraction.unsupported: this build has no data-extraction/attribute-template model` | 不写库/不升修订/不入历史 |
| `DATALINK` | **部分实现（具名拒绝）**：`datalink.unsupported: this build has no external data-link model` | 不写库/不升修订/不入历史 |
| `DATALINKUPDATE` | **部分实现（具名拒绝）**：`datalinkupdate.unsupported: this build has no external data-link model` | 不写库/不升修订/不入历史 |
| `DBCCLOSE` | **部分实现（具名拒绝）**：`dbcclose.unsupported: this build has no dbConnect/external database model` | 不写库/不升修订/不入历史 |
| `DBCCONFIGURE` | **部分实现（具名拒绝）**：`dbcconfigure.unsupported: this build has no dbConnect/external database model` | 不写库/不升修订/不入历史 |
| `DBCDEFINELLT` | **部分实现（具名拒绝）**：`dbcdefinellt.unsupported: this build has no dbConnect/external database model` | 不写库/不升修订/不入历史 |
| `DBCDEFINELT` | **部分实现（具名拒绝）**：`dbcdefinelt.unsupported: this build has no dbConnect/external database model` | 不写库/不升修订/不入历史 |
| `DBCONNECT` | **部分实现（具名拒绝）**：`dbconnect.unsupported: this build has no dbConnect/external database model` | 不写库/不升修订/不入历史 |
| `DCRADIUS` | **部分实现（具名拒绝）**：`dcradius.unsupported: this build has no dimensional-constraint model` | 不写库/不升修订/不入历史 |
| `DCVERTICAL` | **部分实现（具名拒绝）**：`dcvertical.unsupported: this build has no dimensional-constraint model` | 不写库/不升修订/不入历史 |
| `DELAY` | **部分实现（具名拒绝）**：`delay.unsupported: this build has no script runner, so a timed pause has no effect` | 不写库/不升修订/不入历史 |
| `DETACHURL` | **部分实现（具名拒绝）**：`detachurl.unsupported: this build has no hyperlink storage model` | 不写库/不升修订/不入历史 |
| `DETECTCLOSE` | **部分实现（具名拒绝）**：`detectclose.unsupported: this build has no object-detection model` | 不写库/不升修订/不入历史 |
| `DETECTCONVERT` | **部分实现（具名拒绝）**：`detectconvert.unsupported: this build has no object-detection model` | 不写库/不升修订/不入历史 |
| `DETECTPRIMARY` | **部分实现（具名拒绝）**：`detectprimary.unsupported: this build has no object-detection model` | 不写库/不升修订/不入历史 |
| `DETECTREMOVE` | **部分实现（具名拒绝）**：`detectremove.unsupported: this build has no object-detection model` | 不写库/不升修订/不入历史 |
| `DETECTREVIEWNEXT` | **部分实现（具名拒绝）**：`detectreviewnext.unsupported: this build has no object-detection model` | 不写库/不升修订/不入历史 |
| `DETECTREVIEWPREV` | **部分实现（具名拒绝）**：`detectreviewprev.unsupported: this build has no object-detection model` | 不写库/不升修订/不入历史 |
| `-DGNEXPORT` | **部分实现（具名拒绝）**：`-dgnexport.unsupported: this build has no DGN exporter` | 不写库/不升修订/不入历史 |
| `DGNEXPORT` | **部分实现（具名拒绝）**：`dgnexport.unsupported: this build has no DGN exporter` | 不写库/不升修订/不入历史 |
| `-DGNIMPORT` | **部分实现（具名拒绝）**：`-dgnimport.unsupported: this build has no DGN importer/exporter or mapping model` | 不写库/不升修订/不入历史 |
| `DGNIMPORT` | **部分实现（具名拒绝）**：`dgnimport.unsupported: this build has no DGN importer/exporter or mapping model` | 不写库/不升修订/不入历史 |
| `DGNMAPPING` | **部分实现（具名拒绝）**：`dgnmapping.unsupported: this build has no DGN importer/exporter or mapping model` | 不写库/不升修订/不入历史 |
| `DIGITALSIGN` | **部分实现（具名拒绝）**：`digitalsign.unsupported: this build has no digital-signature model` | 不写库/不升修订/不入历史 |
| `DISTANTLIGHT` | **部分实现（具名拒绝）**：`distantlight.unsupported: 3D light objects are not modeled` | 不写库/不升修订/不入历史 |
| `DOWNLOADMANAGER` | **部分实现（具名拒绝）**：`downloadmanager.unsupported: this build has no download-manager model` | 不写库/不升修订/不入历史 |
| `DRAWINGRECOVERY` | **部分实现（具名拒绝）**：`drawingrecovery.unsupported: this build has no drawing-recovery manager model` | 不写库/不升修订/不入历史 |
| `DRAWINGRECOVERYHIDE` | **部分实现（具名拒绝）**：`drawingrecoveryhide.unsupported: this build has no drawing-recovery manager model` | 不写库/不升修订/不入历史 |
| `DVIEW` | **部分实现（具名拒绝）**：`dview.unsupported: this build has no 3D camera/target view model` | 不写库/不升修订/不入历史 |
| `DWFADJUST` | **部分实现（具名拒绝）**：`dwfadjust.unsupported: this build has no DWF/DWFx underlay or export model` | 不写库/不升修订/不入历史 |
| `-DWFATTACH` | **部分实现（具名拒绝）**：`-dwfattach.unsupported: this build has no DWF/DWFx underlay or export model` | 不写库/不升修订/不入历史 |
| `DWFATTACH` | **部分实现（具名拒绝）**：`dwfattach.unsupported: this build has no DWF/DWFx underlay or export model` | 不写库/不升修订/不入历史 |
| `DWFCLIP` | **部分实现（具名拒绝）**：`dwfclip.unsupported: this build has no DWF/DWFx underlay or export model` | 不写库/不升修订/不入历史 |
| `DWFLAYERS` | **部分实现（具名拒绝）**：`dwflayers.unsupported: this build has no DWF/DWFx underlay or export model` | 不写库/不升修订/不入历史 |
| `EDGESURF` | **部分实现（具名拒绝）**：`edgesurf.unsupported: 3D mesh surfaces are not modeled` | 不写库/不升修订/不入历史 |
| `EDITSHOT` | **部分实现（具名拒绝）**：`editshot.unsupported: this build has no ShowMotion named-view/motion model` | 不写库/不升修订/不入历史 |
| `EXPLAN` | **部分实现（具名拒绝）**：`explan.unsupported: this build has no UCS model; the world +Z plan path is PLAN` | 不写库/不升修订/不入历史 |
| `EXPORTDWF` | **部分实现（具名拒绝）**：`exportdwf.unsupported: this build has no DWF/DWFx exporter` | 不写库/不升修订/不入历史 |
| `EXPORTDWFX` | **部分实现（具名拒绝）**：`exportdwfx.unsupported: this build has no DWF/DWFx exporter` | 不写库/不升修订/不入历史 |
| `EXPRESSMENU` | **部分实现（具名拒绝）**：`expressmenu.unsupported: this build has no Express Tools menu/library loader` | 不写库/不升修订/不入历史 |
| `EXPRESSTOOLS` | **部分实现（具名拒绝）**：`expresstools.unsupported: this build has no Express Tools menu/library loader` | 不写库/不升修订/不入历史 |
| `FADEMARKUP` | **部分实现（具名拒绝）**：`fademarkup.unsupported: this build has no collaboration markup/trace model` | 不写库/不升修订/不入历史 |
| `FLATSHOT` | **部分实现（具名拒绝）**：`flatshot.unsupported: 3D solids are not modeled, so there is nothing to flatten` | 不写库/不升修订/不入历史 |
| `FREESPOT` | **部分实现（具名拒绝）**：`freespot.unsupported: 3D light objects are not modeled` | 不写库/不升修订/不入历史 |
| `FREEWEB` | **部分实现（具名拒绝）**：`freeweb.unsupported: 3D light objects are not modeled` | 不写库/不升修订/不入历史 |
| `GCCOINCIDENT` | **部分实现（具名拒绝）**：`gccoincident.unsupported: this build has no geometric-constraint model` | 不写库/不升修订/不入历史 |
| `GCCOLLINEAR` | **部分实现（具名拒绝）**：`gccollinear.unsupported: this build has no geometric-constraint model` | 不写库/不升修订/不入历史 |
| `GCCONCENTRIC` | **部分实现（具名拒绝）**：`gcconcentric.unsupported: this build has no geometric-constraint model` | 不写库/不升修订/不入历史 |
| `GCEQUAL` | **部分实现（具名拒绝）**：`gcequal.unsupported: this build has no geometric-constraint model` | 不写库/不升修订/不入历史 |
| `GCFIX` | **部分实现（具名拒绝）**：`gcfix.unsupported: this build has no geometric-constraint model` | 不写库/不升修订/不入历史 |
| `GCHORIZONTAL` | **部分实现（具名拒绝）**：`gchorizontal.unsupported: this build has no geometric-constraint model` | 不写库/不升修订/不入历史 |
| `GCPARALLEL` | **部分实现（具名拒绝）**：`gcparallel.unsupported: this build has no geometric-constraint model` | 不写库/不升修订/不入历史 |
| `GCPERPENDICULAR` | **部分实现（具名拒绝）**：`gcperpendicular.unsupported: this build has no geometric-constraint model` | 不写库/不升修订/不入历史 |
| `GCSMOOTH` | **部分实现（具名拒绝）**：`gcsmooth.unsupported: this build has no geometric-constraint model` | 不写库/不升修订/不入历史 |
| `GCSYMMETRIC` | **部分实现（具名拒绝）**：`gcsymmetric.unsupported: this build has no geometric-constraint model` | 不写库/不升修订/不入历史 |
| `GCTANGENT` | **部分实现（具名拒绝）**：`gctangent.unsupported: this build has no geometric-constraint model` | 不写库/不升修订/不入历史 |
| `GCVERTICAL` | **部分实现（具名拒绝）**：`gcvertical.unsupported: this build has no geometric-constraint model` | 不写库/不升修订/不入历史 |
| `GEOGRAPHICLOCATION` | **部分实现（具名拒绝）**：`geographiclocation.unsupported: this build has no geographic/GIS or online-map model` | 不写库/不升修订/不入历史 |
| `GEOMAP` | **部分实现（具名拒绝）**：`geomap.unsupported: this build has no geographic/GIS or online-map model` | 不写库/不升修订/不入历史 |
| `GEOMAPIMAGE` | **部分实现（具名拒绝）**：`geomapimage.unsupported: this build has no geographic/GIS or online-map model` | 不写库/不升修订/不入历史 |
| `GEOMAPIMAGEUPDATE` | **部分实现（具名拒绝）**：`geomapimageupdate.unsupported: this build has no geographic/GIS or online-map model` | 不写库/不升修订/不入历史 |
| `GEOMARKLATLONG` | **部分实现（具名拒绝）**：`geomarklatlong.unsupported: this build has no geographic/GIS or online-map model` | 不写库/不升修订/不入历史 |
| `GEOMARKME` | **部分实现（具名拒绝）**：`geomarkme.unsupported: this build has no geographic/GIS or online-map model` | 不写库/不升修订/不入历史 |
| `GEOMARKPOINT` | **部分实现（具名拒绝）**：`geomarkpoint.unsupported: this build has no geographic/GIS or online-map model` | 不写库/不升修订/不入历史 |
| `GEOMARKPOSITION` | **部分实现（具名拒绝）**：`geomarkposition.unsupported: this build has no geographic/GIS or online-map model` | 不写库/不升修订/不入历史 |
| `GEOREMOVE` | **部分实现（具名拒绝）**：`georemove.unsupported: this build has no geographic/GIS or online-map model` | 不写库/不升修订/不入历史 |
| `GEOREORIENTMARKER` | **部分实现（具名拒绝）**：`georeorientmarker.unsupported: this build has no geographic/GIS or online-map model` | 不写库/不升修订/不入历史 |
| `GOTOURL` | **部分实现（具名拒绝）**：`gotourl.unsupported: this build has no hyperlink storage model` | 不写库/不升修订/不入历史 |
| `-GRAPHICSCONFIG` | **部分实现（具名拒绝）**：`-graphicsconfig.unsupported: renderer/graphics configuration is owned by the host` | 不写库/不升修订/不入历史 |
| `GRAPHICSCONFIG` | **部分实现（具名拒绝）**：`graphicsconfig.unsupported: renderer/graphics configuration is owned by the host` | 不写库/不升修订/不入历史 |
| `HIDE` | **部分实现（具名拒绝）**：`hide.unsupported: the renderer has no 3D hidden-line removal` | 不写库/不升修订/不入历史 |
| `HLSETTINGS` | **部分实现（具名拒绝）**：`hlsettings.unsupported: this build has no hidden-line display settings model` | 不写库/不升修订/不入历史 |
| `-HYPERLINK` | **部分实现（具名拒绝）**：`-hyperlink.unsupported: this build has no hyperlink storage model` | 不写库/不升修订/不入历史 |
| `HYPERLINK` | **部分实现（具名拒绝）**：`hyperlink.unsupported: this build has no hyperlink storage model` | 不写库/不升修订/不入历史 |
| `-INSERTCONTENT` | **部分实现（具名拒绝）**：`-insertcontent.unsupported: this build has no content-palette insertion model` | 不写库/不升修订/不入历史 |
| `INSERTOBJ` | **部分实现（具名拒绝）**：`insertobj.unsupported: this build has no OLE object storage model` | 不写库/不升修订/不入历史 |
| `-INTERFERE` | **部分实现（具名拒绝）**：`-interfere.unsupported: 3D solids are not modeled, so interference has nothing to compute` | 不写库/不升修订/不入历史 |
| `INTERFERE` | **部分实现（具名拒绝）**：`interfere.unsupported: 3D solids are not modeled, so interference has nothing to compute` | 不写库/不升修订/不入历史 |
| `LIGHT` | **部分实现（具名拒绝）**：`light.unsupported: 3D light objects are not modeled` | 不写库/不升修订/不入历史 |
| `LIGHTLIST` | **部分实现（具名拒绝）**：`lightlist.unsupported: 3D light objects are not modeled` | 不写库/不升修订/不入历史 |
| `LIGHTLISTCLOSE` | **部分实现（具名拒绝）**：`lightlistclose.unsupported: 3D light objects are not modeled` | 不写库/不升修订/不入历史 |
| `LIVESECTION` | **部分实现（具名拒绝）**：`livesection.unsupported: this build has no section-object model` | 不写库/不升修订/不入历史 |
| `LOAD` | **部分实现（具名拒绝）**：`load.unsupported: this build has no SHX shape-file loader` | 不写库/不升修订/不入历史 |
| `LSP` | **部分实现（具名拒绝）**：`lsp.unsupported: this build has no AutoLISP runtime or compiler` | 不写库/不升修订/不入历史 |
| `LSPSURF` | **部分实现（具名拒绝）**：`lspsurf.unsupported: this build has no AutoLISP runtime or compiler` | 不写库/不升修订/不入历史 |
| `MAKELISPAPP` | **部分实现（具名拒绝）**：`makelispapp.unsupported: this build has no AutoLISP runtime or compiler` | 不写库/不升修订/不入历史 |
| `MARKUP` | **部分实现（具名拒绝）**：`markup.unsupported: this build has no collaboration markup/trace model` | 不写库/不升修订/不入历史 |
| `-MARKUPASSIST` | **部分实现（具名拒绝）**：`-markupassist.unsupported: this build has no collaboration markup/trace model` | 不写库/不升修订/不入历史 |
| `MARKUPASSIST` | **部分实现（具名拒绝）**：`markupassist.unsupported: this build has no collaboration markup/trace model` | 不写库/不升修订/不入历史 |
| `MARKUPCLOSE` | **部分实现（具名拒绝）**：`markupclose.unsupported: this build has no collaboration markup/trace model` | 不写库/不升修订/不入历史 |
| `-MARKUPIMPORT` | **部分实现（具名拒绝）**：`-markupimport.unsupported: this build has no collaboration markup/trace model` | 不写库/不升修订/不入历史 |
| `MARKUPIMPORT` | **部分实现（具名拒绝）**：`markupimport.unsupported: this build has no collaboration markup/trace model` | 不写库/不升修订/不入历史 |
| `MATBROWSERCLOSE` | **部分实现（具名拒绝）**：`matbrowserclose.unsupported: this build has no material/texture model` | 不写库/不升修订/不入历史 |
| `MATBROWSEROPEN` | **部分实现（具名拒绝）**：`matbrowseropen.unsupported: this build has no material/texture model` | 不写库/不升修订/不入历史 |
| `MATEDITORCLOSE` | **部分实现（具名拒绝）**：`mateditorclose.unsupported: this build has no material/texture model` | 不写库/不升修订/不入历史 |
| `MATEDITOROPEN` | **部分实现（具名拒绝）**：`mateditoropen.unsupported: this build has no material/texture model` | 不写库/不升修订/不入历史 |
| `MATERIALASSIGN` | **部分实现（具名拒绝）**：`materialassign.unsupported: this build has no material/texture model` | 不写库/不升修订/不入历史 |
| `MATERIALATTACH` | **部分实现（具名拒绝）**：`materialattach.unsupported: this build has no material/texture model` | 不写库/不升修订/不入历史 |
| `MATERIALMAP` | **部分实现（具名拒绝）**：`materialmap.unsupported: this build has no material/texture model` | 不写库/不升修订/不入历史 |
| `MATERIALS` | **部分实现（具名拒绝）**：`materials.unsupported: this build has no material/texture model` | 不写库/不升修订/不入历史 |
| `MATERIALSCLOSE` | **部分实现（具名拒绝）**：`materialsclose.unsupported: this build has no material/texture model` | 不写库/不升修订/不入历史 |
| `MIGRATEMATERIALS` | **部分实现（具名拒绝）**：`migratematerials.unsupported: this build has no material/texture model` | 不写库/不升修订/不入历史 |
| `MKLTYPE` | **部分实现（具名拒绝）**：`mkltype.unsupported: this build has no .lin definition-file writer` | 不写库/不升修订/不入历史 |
| `MKSHAPE` | **部分实现（具名拒绝）**：`mkshape.unsupported: this build has no SHX shape compiler` | 不写库/不升修订/不入历史 |
| `MOVEBAK` | **部分实现（具名拒绝）**：`movebak.unsupported: the backup-folder location is owned by the host filesystem` | 不写库/不升修订/不入历史 |
| `MSLIDE` | **部分实现（具名拒绝）**：`mslide.unsupported: this build has no slide (SLD) writer` | 不写库/不升修订/不入历史 |
| `NAVBAR` | **部分实现**：真实设置写入 `NAVBAR [ON\|OFF]`（持久化导航栏可见性）；无 AutoCAD 完整导航工具集（SteeringWheels/ShowMotion/3Dconnexion） | 仅写 `preference.json` 根键 `navbarVisible`；不写库/不升修订/不入历史 |
| `NAVSMOTION` | **部分实现（具名拒绝）**：`navsmotion.unsupported: this build has no 3D navigation/ViewCube model` | 不写库/不升修订/不入历史 |
| `NAVSMOTIONCLOSE` | **部分实现（具名拒绝）**：`navsmotionclose.unsupported: this build has no 3D navigation/ViewCube model` | 不写库/不升修订/不入历史 |
| `NAVSWHEEL` | **部分实现（具名拒绝）**：`navswheel.unsupported: this build has no 3D navigation/ViewCube model` | 不写库/不升修订/不入历史 |
| `NAVVCUBE` | **部分实现**：真实设置写入 `NAVVCUBE [ON\|OFF]`（按当前 2D/3D 视图类别切 `navvcubeDisplay` 位）；`PROPERTIES` 具名拒绝 `navvcube.properties_not_implemented`；无罗盘/边角目标/动画 | 仅写 `preference.json` 根键；不写库/不升修订/不入历史 |
| `NEWSHEETSET` | **部分实现（具名拒绝）**：`newsheetset.unsupported: this build has no sheet-set model` | 不写库/不升修订/不入历史 |
| `NEWSHOT` | **部分实现（具名拒绝）**：`newshot.unsupported: this build has no ShowMotion named-view/motion model` | 不写库/不升修订/不入历史 |
| `OLECONVERT` | **部分实现（具名拒绝）**：`oleconvert.unsupported: this build has no OLE object storage model` | 不写库/不升修订/不入历史 |
| `OLELINKS` | **部分实现（具名拒绝）**：`olelinks.unsupported: this build has no OLE object storage model` | 不写库/不升修订/不入历史 |
| `OLEOPEN` | **部分实现（具名拒绝）**：`oleopen.unsupported: this build has no OLE object storage model` | 不写库/不升修订/不入历史 |
| `OLERESET` | **部分实现（具名拒绝）**：`olereset.unsupported: this build has no OLE object storage model` | 不写库/不升修订/不入历史 |
| `OLESCALE` | **部分实现（具名拒绝）**：`olescale.unsupported: this build has no OLE object storage model` | 不写库/不升修订/不入历史 |
| `OPENDWFMARKUP` | **部分实现（具名拒绝）**：`opendwfmarkup.unsupported: this build has no DWF markup model` | 不写库/不升修订/不入历史 |

### 2026-11-10 P4：图层表写入纳入 `DrawingUndoRecord`

> `cad-db` 新增 `LayerRow`/`LayerPatch` 与组合应用入口；`cad-history` 的 `DrawingUndoRecord.layer_patches` 参与校验/大小/合并拒绝；`cad-app` 的 `UNDO`/`REDO` 与应用器按同一 revision 重放图层行。以下命令的图层表写入现在是**一条事务/一步撤销**；旧的“图层表写入不可撤销”界已删除。图纸空间/块内实体与 Settings 选项边界仍在，因此状态仍为**部分实现**。

| 命令 | 当前行为 | 剩余边界 |
| --- | --- | --- |
| `LAYDEL` / `-LAYDEL` | 删除全部空间（模型/图纸布局/块定义）的该层实体 + 各图纸视口该层覆盖键 + 层行，实体/布局/层行同一记录/一步撤销 | 当前层/`0` 层拒绝；锁定层由数据库拒绝 |
| `LAYMRG` / `-LAYMRG` | 重写实体 + 删除源层 + 剪除视口覆盖键，同一记录/一步撤销 | 当前层/0 层不能作源；目标固定为当前层（声明的偏离） |
| `LAYFRZ` | **完整实现**：绘图级冻结 + `VP [idx,…]` 逐视口冻结，跨空间/块成员选择，一步撤销 | 交互对话框全局范围；当前层仅在目标视口含 `CVPORT` 时拒绝 |
| `LAYLCK` / `LAYULK` | **完整实现**：跨空间/块成员选择，锁定/解锁所在层，一步撤销 | 交互对话框全局范围 |
| `LAYTHW` | 解冻全部层：绘图级标志 + 每个图纸布局/视口的冻结覆盖，层补丁 + 布局补丁一步撤销 | 保留 `frozen_in_new_viewport` 与显式 `false` 覆盖（不是冻结） |
| `RENAME` / `-RENAME` | 六种 kind 真实重命名（含 `LINETYPE` 与引用级联），一步撤销 | 无对话框；创建/删除线型仍不支持 |
| `LAYERSTATE RESTORE` | 恢复的图层属性写入一步撤销 | 状态仍为会话内、不持久化 DWG |
| `PURGE` | **有意不记历史**（复用无历史删除体） | 只清未使用图层 |

### 2026-11-10 P5：布局/视口写 API 与撤销

> `cad-db` 新增 `LayoutRow`/`LayoutPatch`、单调 `next_layout_id`、`create/rename/delete_layout`、`set_layout_viewports` 与组合实体+布局应用入口；`cad-history` 的 `DrawingUndoRecord.layout_patches` 参与校验/大小/合并拒绝；`UNDO`/`REDO` 按同一 revision 重放布局行与 plot settings。以下写入均为**一条事务/一步撤销**。

| 命令 | 当前行为 | 剩余边界 |
| --- | --- | --- |
| `LAYOUT` `NEW`/`RENAME`/`DELETE` | 真实创建/重命名/删除布局；删除级联 `plot_settings`，删除活动布局时回退到首个剩余布局或模型空间；一步撤销 | `COPY`/`TEMPLATE` 未实现 |
| `LAYOUTMERGE` | 源布局的图纸空间实体移入目标，删除源布局并级联 plot settings；一步撤销；**升为完整实现** | 同一布局拒绝 |
| `LAYOUTWIZARD` | 真实创建布局 + 纸张设置 + 等分格视口排列，一步撤销 | 打印设备/打印样式/标题栏（全局范围）；仅等分格 |
| `VPORTS` / `-VPORTS` | 活动布局创建等分格视口网格：显式 `rows×cols` 或对称标准配置 `SINGLE`/`2 VERTICAL 或 HORIZONTAL`/`3 VERTICAL 或 HORIZONTAL`/`4 [EQUAL]`（同一网格助手、四角 clip、`model_per_paper=1`），一步撤销；模型空间按 `vports.paper_space_required` 拒绝 | 命名配置（`SAVE`/`RESTORE`/`DELETE`/`JOIN`）按 `vports.named_configuration_not_implemented` 拒绝：无持久化命名视口表（新 `.ymodel` table/minor 不在本切片）；非对称标准配置（`Three:Left` 等）按 `vports.asymmetric_configuration_not_implemented` 拒绝且不近似 |
| `VPCLIP RECT` | 对活动布局第一个视口应用矩形裁剪，一步撤销 | 无视口选择；多边形裁剪未实现 |
| `VPSCALE` | 设置活动布局第一个视口比例（对角线 `1/factor`，保留模型锚点），一步撤销 | 无视口选择 |
| `PAGESETUP` / `-PAGESETUP` | **完整实现**：裸命令只读报告；typed 形式写入 Authored `PLOTSETTINGS`（纸张/比例/旋转），非法布局/纸名/尺寸/比例在写前按名拒绝，一步撤销 | 绘图仪/CTB/STB 属 ADR 全局范围 |
| `PSETUPIN` / `-PSETUPIN` | 同图纸内复制已存 `PLOTSETTINGS` 到目标布局，一步撤销（两条都保持部分实现） | 跨文件导入未实现（无源文档页面设置提取路径）；无源记录拒绝；无文件对话框 |
| `MVIEW` | 活动图纸空间创建矩形视口，一步撤销 | 无多边形/非矩形；无视口选择 |
| `VPMAX` / `VPMIN` | 会话标记（不变） | 仅第一个视口 |
| `MVSETUP` | 仍为具名拒绝 | 无视口组模型 |

## P2 晋升与边界更新（2026-11-09）

> 以下条目**取代正文中同名命令的旧界**。依据：AutoCAD 2026 ENU 帮助页逐条核对；证据仅 Linux
> 编译 + 契约测试，GUI/GPU/像素 NOT VERIFIED。**严格口径**：依赖「未建模能力」的命令保留
> `部分实现`，不把模型缺口当作完成豁免。

| 命令 | 状态 | 当前行为 | 剩余边界 |
| --- | --- | --- | --- |
| `CHAMFER` | **完整实现** | `Distance`/`Angle`/`Method`/`Trim`；双零 ⇒ 仅延伸相交；`CHAMFER POLYLINE` 全顶点、弧段替换、过短跳过、开口闭合 | `Multiple`/命令内 `Undo` 属逐选项提示全局范围；负距离未文档化 ⇒ 具名拒绝；圆弧/圆/样条 AutoCAD 也不接受 |
| `FILLET` | 完整实现 | 见 2026-11-15 行：任意 2D 平面对 `R>0` 相切弧 + `R=0` 尖角，一条事务/一步撤销 | 仅全局交互范围（`Multiple`/`Undo`/多解循环） |
| `PEDIT` | 部分实现 | 全选项行；`Join` 接受线/弧/多段线；`Fit`/`Spline` 含闭合环；`Decurve`/`Reverse`/`Undo`；LINE/ARC/SPLINE 转换 | **`Width`/`Ltype gen`/`Edit vertex` 未实现**（多段线无宽度/`plinegen`/顶点子菜单模型）；偶数环无精确弧拟合 ⇒ `pedit.fit_closure` |
| `JOIN` | 完整实现 | 源对象模式；`Multiple` 把 LINE/ARC/多段线/SPLINE/椭圆弧精确焊接为**有理 B 样条**；`JOIN CLOSE` 的混合 SPLINE 焊接后置 `Spline.closed`（DXF 70 bit 1 往返，闭合样条按闭环出图/拾取），一条事务/一步撤销 | 链端不重合 `join.not_closed`、有间隙 `join.not_contiguous`（不桥接）、已闭合对象作源 `join.closed_object`；焊接仍是 C0 拼接（非周期/平滑）；非共面多段线属 3D 全局范围 |
| `OFFSET` | **完整实现** | `Through`/`Erase`/`Layer`/`Multiple`（单步）/距离；椭圆与椭圆弧→样条；`OFFSETGAPTYPE` Sharp/**Fillet**/**Chamfer** | `OFFSET MULTIPLE` 的多点循环拒绝（单点捕获，属逐选项提示全局范围）；开放折线远端自交未检查（AutoCAD 未文档化） |
| `STRETCH` | 完整实现 | 交叉矩形、交叉多边形（点击 + `CONFIRM`/回点首顶点闭合）与自由手绘 lasso（拖拽采样、释放闭合、CROSSING 预览）都经工具捕获进入同一 `polygon_window` 规划；even-odd 包含；部分包围逐顶点、完全包围整移；圆按圆心、块/文字按插入点；标注按定义点；一条事务/一步撤销 | `MULTILEADER` 整命令拒绝；环<3 顶点/非有限/零面积/未闭合具名拒绝；仅 2D 平面视图；lasso 采样是原始指针点（不经捕捉/正交） |
| `MIRROR` | **完整实现** | `MIRRTEXT=0` 可读镜像（位置镜像、文字保持可读）；`MIRRTEXT=1` 真反射（`Text`/MTEXT 的字形镜像位 + 基线半转，标注测量文字同样置位）；`MIRROR ERASE YES/NO`；标注反射后重合成；镜像位随 `.ymodel` minor 15 与 DXF/DWG 往返 | `Shape`/`Opaque`/`MULTILEADER` 与块定义成员按名拒绝（无镜像表示）；块参照只反射放置、不改写块定义 |
| `EXPLODE` | 部分实现 | 标注→线/弧/箭头网格/文字；多重引线→引线/样条/箭头网格/MTEXT；非均匀块可表示子集；嵌套一层；MINSERT/XREF 拒绝 | **多重引线的背景/分栏/自定义箭头/打断/非默认平面未导入也未产出**；块属性值丢弃（无属性模型） |
| `QSELECT` | 部分实现 | `Equals`/`Not Equal`/`>`/`<`/`>=`/`<=`/通配符/`Select All`；整图（当前空间）或当前选择；`Include`/`Exclude`/`Append`；嵌套块不参与；只读 | **分类/自定义对象特性未建模**，属性集为基础子集；通配符仅 `*`/`?` |
| `FILTER` | 部分实现 | `AND`/`OR`/`XOR`/`NOT` 递归逻辑树（括号、元数校验）、相对运算符、已建模属性集；只读 | **分类/自定义特性未建模**；**命名 `.nfl` 过滤器未实现**（宿主文件 I/O） |

## P3 晋升与边界更新（2026-11-09）

> 以下条目**取代正文中同名命令的旧界**。依据：AutoCAD 2026 ENU 帮助页（Express Tools 的提示文案
> 仅旧版手册有，已标注）；证据仅 Linux 编译 + 契约测试，GUI/GPU/像素 NOT VERIFIED。

| 命令 | 状态 | 当前行为 | 剩余边界 |
| --- | --- | --- | --- |
| `OVERKILL` | **完整实现** | 容差（`0`=精确）、忽略 Color/Layer/Linetype/LTScale/Lineweight/Transparency、多段线段优化 + `Do Not Break Polylines`、共线部分重叠/端到端合并、弧覆盖圆、重复段删除；保留最低 `(draw_order,id)`；一条事务/一步撤销 | 关联保持属关联性全局范围；段宽/厚度/打印样式未建模；逐段 bulge 数组布局按名报告 |
| `AUDIT` | **完整实现** | `Fix any errors detected? [Yes/No]`（默认 No，键入 `AUDIT YES/NO/LOG`）；分段报告；只修复 `validate_entity` 可证明可修复的行；当前空间错误对象进入 Previous 映射 | 可修复错误类受数据库校验器限制（AutoCAD 也不公布清单）；修复不可撤销（`audit.repairs_not_undoable`）；`AUDITCTL` `.adt` 日志属宿主文件 I/O |
| `SSX` | 部分实现 | Express **筛选表构建器**（非 SELECTSIMILAR）：`BLOCK/COLOR/ENTITY/LAYER/LTYPE/STYLE/PICK/CLEAR`，单值覆盖/移除；模板来自当前选择；当前空间；替换选择；只读 | **`FLAG`/`THICKNESS`/`VECTOR` 未建模**（实体标志位/厚度/挤出矢量）⇒ 具名拒绝；无 Previous 选择集，映射为替换当前选择 |
| `QDIM` | **完整实现** | `Continuous`（默认）/`Staggered`/`Baseline`/`Ordinate`/`Radius`/`Diameter` + `Datum Point`/`Edit`/`Settings`；既有标注可作为输入；一条事务/一步撤销 | 无交互位置/选项循环（逐选项提示全局范围）；角度不是 QDIM 模式；Staggered 步长 = `DIMDLI`（声明）；`INTERSECTIONS` 只用选中 LINE 对 |
| `DIMTEDIT` | 完整实现 | `MOVE`/`LEFT`/`RIGHT`/`CENTER`/`HOME`/`ANGLE`；Left/Right/Center 移动锚点并设置字形级 `h_align`，角标注锚定已绘制弧、坐标标注锚定引线端点；Angle 固定文字中心，`0` 复位；一条事务/一步撤销 | `OBLIQUE` 为具名拒绝（字形倾斜无字段，全局范围）；角标注为导入图片时 `dimtedit.justify_unsupported` |
| `HATCHEDIT` | 完整实现 | 多目标；`Style`/`PatternType`/`PATTERN <name>`/`PATTERNFILE <path.pat> [<name>]`/`ORIGIN <x> <y>`/`Angle`/`Scale`/`Solid`/`Gradient`/`Color`/`Transparency`/`Layer`/`DrawOrder`/`Recreate Boundary`/`Separate Hatches`/`Add|Remove Boundaries`；一条事务/一步撤销 | 全局范围：`Associate`/`Disassociate`/`Annotative`、`DRAWORDER BOUNDARY`（需边界实体 id 即反应器模型）、逐 loop 图案原点、交互对话框 |
| `REVCLOUD` | 部分实现 | `Object`/`Rectangular`/`Polygonal`/`Freehand`/`Arc Length`/`Style`/`Modify`；等弦弧（106.26°）；默认弦长 = 源路径 bbox 对角线 5%（声明替代）；一条事务/一步撤销 | **`Calligraphy` 样式未实现**（多段线无样式字段）；`Modify` 用确定性规则替代交互拾取；`RECT/POLY/FREEHAND/MODIFY` 的路径来自唯一选中 LWPOLYLINE |
| `WIPEOUT` | 完整实现 | `Frames ON/OFF/DISPLAY` 与 `WIPEOUTFRAME 0|1|2`（同一会话令牌，`SETVAR` 可读写，越界 `wipeoutframe.out_of_range`）：ON = 填充 + 边框（显示与矢量出图都画边界），OFF = 仅填充，DISPLAY = 显示边框、出图不画；边框在 mask fill 之后以 Always-depth 绘制；`Polyline` + `Erase Polyline Yes/No`、自由点串；**矢量出图按页面背景 flood**（普通遮罩在几何之后按 draw order，`inverted` 为页面环 + 边界环的多环 even-odd 补集，SVG `fill-rule="evenodd"`、PDF `f*`，单环输出与旧版逐字节一致）；已有遮罩**沿边界可拾取**，`SELECTALL`/`QSELECT TYPE WIPEOUT`/`ERASE`/`MOVE`/`HIDEOBJECTS` 走通用实体路径 | **全局范围**：`FRAME` 本身（图像/xref/PDF 边框显示模型不存在）、遮罩选择/悬停高亮（渲染器只解析 GPU pick key，引擎报 `flags.unresolved`）、遮罩顶点 grip 拖拽、`inverted` 仅导入/显示/出图、跨图纸视口窗口不裁剪（`representation.viewport_clip_partial`）；自交按 even-odd 如实填充（只拒零净面积）；无真实 GPU/像素验收（Linux 编译 + 契约测试） |
| `TEXTMASK` | 部分实现 | 偏移因子（默认 0.35）、`Wipeout` 遮罩、文本+遮罩的**会话私有配对**（不是文档 `GROUP`）、重跑替换旧遮罩、`TEXTUNMASK` 按配对优先；一条事务/一步撤销 | **`3dface`/`Solid` 遮罩类型未实现**（无对应实体）；文本/遮罩配对为会话存储（DWG 不落关联记录） |
| `TXT2MTXT` | 完整实现 | `COMBINE`/`SEPARATE`、`ORDER TOPDOWN/SELECT`、`WRAP`（`wrap_width`）、`UNIFORM <factor>`（0.25–4.0，写入 MTEXT `TextLineSpacing` 且样式 `Exactly`）、源对象在同一事务删除 | `UNIFORM` 之外没有 Settings 对话框（全局范围）；导入的越界 factor 仍渲染（存储契约只要求正且有限）；行距仍以 `1.2×factor×行高` 近似，不是字体真实垂直度量 |

## Wave 26 边界更新（2026-11-12）

> 以下条目**取代正文中同名命令的旧界**。依据：AutoCAD 2026 ENU 帮助页语义 + 本轮源码/契约测试；
> 证据仅 Linux 编译 + 契约测试，GUI/GPU/像素/真机 NOT VERIFIED。

| 命令 | 状态 | 当前行为 | 剩余边界 |
| --- | --- | --- | --- |
| `FILLET` | 完整实现 | `R>0`：LINE/ARC/CIRCLE/ELLIPSE（含椭圆弧）/SPLINE 任意两两组合——SPLINE 用精确 `sub_curve` 修剪（不重拟合），椭圆/样条走数值相切解 + 交替投影抛光；插入一段相切 ARC；`R=0`：全部 10 种平面对只修剪不插入，CIRCLE↔CIRCLE 复用 TRIM 的两交点 CIRCLE→ARC 规则把两个圆都切成拾取侧弧，LINE 可延伸到角点；被修剪对象保持 id/图层/空间/draw order；一条事务/一步撤销 | `Multiple`、命令内 `Undo` 与多解循环属全局交互范围；相切/不相交/非共面/非平面逐条具名拒绝；CIRCLE→ARC 后实体 `type_key` 仍为 `Circle`（与 TRIM 同一约定） |
| `BREAK` | **完整实现** | `LINE`/`ARC`/`CIRCLE`/`SPLINE`/整椭圆/**椭圆弧**/`LWPOLYLINE`；`F` 首点覆盖；圆→弧；一条事务/一步撤销 | 无多段打断（AutoCAD 无此模式） |
| `DIVIDE` | **完整实现** | 直线/弧/圆/样条/**椭圆与椭圆弧**/含 bulge 折线按等弧长；开放 `N−1`、闭合 `N`；块对齐 | 非曲线种类按名拒绝 |
| `PEDIT` | 部分实现 | 全选项行 + **Edit vertex 子菜单**（NEXT/PREVIOUS/INSERT/MOVE/STRAIGHTEN）；JOIN 接受 SPLINE 拾取 | `WIDTH`/`LTYPE GEN`/`Edit vertex TANGENT` 无域字段 |
| `TRIM`/`EXTEND` | 部分实现 | **ELLIPSE/椭圆弧目标**；EXTEND 弧可超过半圈（至整圈前）；**LWPOLYLINE 目标（仅 TRIM，2026-11-12 起）**：保留含拾取点投影的窗口、其余丢弃（原地替换），闭合折线需两条切点；INSERT/HATCH/MESH/OPAQUE/COMPOUND 等目标按名拒绝 | 折线目标**没有 EXTEND**（`extend.target_polyline`），也无宽度语义；SPLINE 目标的 EXTEND 无外推策略 |
| `ARRAYRECT`/`ARRAYPOLAR` | 部分实现 | 有符号行/列间距；极坐标步长按 AutoCAD `fill/(items−1)`（半圈）/`fill/items`（整圈） | 独立非关联副本；`Levels`/网格角度无工具参数；关联编辑未建模 |
| `HATCH` | 完整实现 | **`.pat` 解析器/名称解析**（内置 ANSI31/32/37 同源解析）+ `PATTERN`/`PATTERNFILE` 命令名通道；整实体闭合边界与 LINE/ARC 边链、solid/user/predefined/gradient 全部真实写入 | 关联性/实时预览属全局范围 |
| `HATCHEDIT` | 部分实现 | `PATTERNTYPE CUSTOM`；**Add/Remove Boundaries**（闭合/共面校验） | `Origin` 无字段、`Associate`/`Annotative`/Boundary draw order 无模型（历史条目；当前 `ORIGIN` 已实现，见前表） |
| `TXTEXP` | **完整实现** | 字体轮廓→每轮廓一条 `LWPOLYLINE`；闭合 sfnt 环闭合、SHX 笔画开放；删除源；一条事务/一步撤销 | 宿主必须安装字体引擎；MTEXT 宽度换行未应用（声明） |
| `ARCTEXT` | **完整实现** | 沿选中 ARC 等角步逐字符 `TEXT`（切线旋转/倒置翻转）；源文字消费 | 每字符一个 TEXT（表示偏离）；仅 +Z、单行 |
| `VPLAYER` | **完整实现** | `FREEZE/THAW <layer,...> [<index>]` 写单视口 `layer_overrides`；bare 报告；命令行 grammar 已接 | 一次一个视口（无交互多视口循环）；全局层表不动 |
| `VPMAX`/`VPMIN` | **完整实现** | 真实最大化编辑态：相机+视图模式+工作平面+空间快照，`VPMIN` 位级恢复 | 一次一个视口；宿主无差异化渲染（无像素证据） |
| `VPCLIP`/`MVIEW` | 完整实现 | 矩形与多边形 clip/视口都写入并真实渲染（多边形裁剪对折线/圆锥曲线/网格/填充/图像全覆盖，矩形输出逐字节不变）；`CVPORT` 当前视口定位；导入侧按 DXF 组 340 解析真实多边形边界 | 声明的边界：多环「孔洞」视口不建模；无平移的 3 点边界 `*.ambiguous_triangle`；无交互拖拽改视口顶点 |
| `PSBSCALE`/`PSTSCALE` | 部分实现 | 按活动视口 paper-per-model 因子缩放图纸空间块/文字 | 单一方向/单一视口（声明）；不处理 MULTILEADER/TABLE 文本 |
| `SHAPE` | **完整实现** | `SHAPE <name-or-code> <x> <y> [size] [rotation_deg]` 创建 `SemanticGeometry::Shape` | 按 name 只建行不解析（本构建 SHX 只保留编号）；表示层保持既有 `Partial` |
| `SHP2BLK` | **完整实现** | 选中 SHAPE → `(font, code)` 块（轮廓多段线）+ 每个源位置 INSERT，删除源 | 需宿主安装字体引擎；块不链接回 SHX 字体 |
| `MINSERT` | **完整实现** | 矩形 INSERT 阵列（ARRAYRECT 单元规则 + INSERT `T(-base)`），一条事务 | 独立副本（域无 MINSERT 实体，非关联） |
| `ADDSELECTED` | 完整实现 | 点锚定型 LINE/CIRCLE/ARC/POLYLINE/INSERT/POINT/TEXT/MTEXT 新建，保留图层/颜色/线型/线宽/透明度；ARC 以圆心、POLYLINE 以首顶点、INSERT 以插入点锚定 | 其它种类（ELLIPSE/SPLINE/HATCH 等）按名拒绝；仅模型空间 |
| `IMAGECLIP` | 完整实现 | 矩形 + `POLY` N 边形 + `OFF`；世界坐标经 u/v 映射；非凸环耳切成凸片段（每片段与图像矩形各自求交），面积精确、凹口为空、`Completeness::Complete` 且无诊断；凸环顶点逐字节不变 | **全局范围**：无交互拖拽裁剪顶点（grip 编辑未建模）、`inside=false`（补集裁剪）未授权；自交/点数不足/零面积按 `image.clip_*` 具名拒绝 |
| `APERTURE` | **完整实现** | `APERTURE <n>` 写会话孔径，snap/候选拾取真实使用 | `PICKBOX` 保持独立；未显式设置时配置默认值不变 |
| `PTYPE` | **完整实现** | `PTYPE <code> [size]` 委托 PDMODE/PDSIZE 头部写入 | 两次 header 写、无 header undo 补丁 |
| `REVCLOUDPROPERTIES` | **完整实现** | `REVCLOUDPROPERTIES <arc-length>` 写 token，REVCLOUD 真实消费 | 仅会话状态，不持久化 DWG |
| `QUICKCALC`/`QVDRAWING`/`QVLAYOUT`/`WORKSPACE`/`WSSAVE`/`WSSETTINGS`/`TOOLBAR` | **完整实现** | chrome 本地选择子/面板/浮窗/对话框；不进入 `cad_command` | 会话内存工作空间；无跨重启持久化 |
| `RIBBON`/`RIBBONCLOSE`/`HIDEPALETTES`/`SHOWPALETTES`/`FILETAB`/`FILETABCLOSE` | **完整实现** | chrome 可见性设置（set，不 toggle）；单文档文件标签 | 视觉/像素 NOT VERIFIED |

### Wave 26 续（2026-11-12，样式表 / 引线编辑 / 系统变量）

| 命令 | 状态 | 当前行为 | 剩余边界 |
| --- | --- | --- | --- |
| `STYLE`/`-STYLE` | **完整实现** | `NEW`/`RENAME`/`DELETE`/`SET`（FONT/KEYS/ADDKEY/REMOVEKEY）/`?`；一条 style-table patch/一步撤销 | 域 `Style` 只有 name/resource_keys；无“当前文字样式”会话变量；修改不回写既有文字 |
| `DIMSTYLE` | **完整实现** | `LIST`/`CURRENT`/`SET`/`NEW`/`RENAME`/`DELETE`/`MODIFY`；一条 patch/一步撤销；`Standard` 不可删、被引用不可改删 | `MODIFY` 不级联已烘焙标注快照（`DIMREGEN` 是关联性全局范围下的非关联 no-op） |
| `MLEADERSTYLE` | **完整实现** | `?`/`LIST`/`SET`/`NEW`/`RENAME`/`DELETE`/`MODIFY`；一条 patch/一步撤销；新引线使用当前样式 | 既有引线保留烘焙快照；`Standard` 不可删 |
| `RENAME` | 部分实现 | LAYER/STYLE/DIMSTYLE/MLEADERSTYLE/BLOCK 真实写 | `LINETYPE` 无 rename API（具名拒绝）；块删除仍不可用 |
| `MLEDIT`/`-MLEDIT` | 部分实现 | 两独立 LINE 的 MERGE/CLOSED/CROSS/TEE 清理、OPEN 断开、BREAK 带移除；一条事务 | 无 `AcDbMline` 顶点/join/cap/fill 模型；四种清理拼写归并为一种可表示操作 |
| `MLEADERALIGN` | 部分实现 | LEFT/RIGHT/CENTER + 等距纵排；箭头锚定，其余平移 | 无 distribute/column 选项；有保留断点的引线拒绝 |
| `MLEADEREDIT` | 部分实现 | ADD 根 0 一条线 / REMOVE 一条（不可清空） | 无逐线覆盖/箭头编辑；不可见路径样式拒绝 |
| `MLEADERCOLLECT` | 部分实现 | ≥2 条合并为一条（根/线拼接、文本 `\P`、首样式/平面、根落点重定向），删除源 | 样式/覆盖掩码/平面必须一致；无堆叠文本/分栏/关联性 |
| `TOLERANCE` | 部分实现 | `TOLERANCE <x> <y> <text>` 生成边框+分隔线+每格 TEXT | 独立实体非 `AcDbFcf`；轴对齐；格宽/行高为估计；≤6 格；**单元格语法缺口**：未实现对话框的槽位语义（两个公差槽、三个基准槽、投影高度、基准标识）与 GDT 符号字体、`%%v`、`^J`（`tolerance.gdt_symbol`/`tolerance.multiline`/`tolerance.dxf_separator` 具名拒绝） |
| `CLASSICXREF` | 部分实现 | 只读 xref 报告 + chrome 面板（真实 `cad_status.references`） | attach/detach/reload 仍按名拒绝；面板不写库 |
| `PMTOGGLE` | **完整实现** | 切换选项板可见性（与 HIDEPALETTES/SHOWPALETTES 同状态） | chrome 本地选择子，不进入 `cad_command` |
| `SYSVARMONITOR` | 部分实现 | ADD/REMOVE/LIST + 查询时快照变化检测 | 无后台通知；会话状态不持久化；仅 SETVAR 闭集变量 |
| `SYSVDLG` | 完整实现 | 报告 + `<name>` 读取 + `<name> <value>` 编辑 + `SAVE|RESTORE <file.json>`（版本化 JSON）；未建模变量按名拒绝 | 无交互对话框（全局范围）；不是 `.svf`；无值变量保存时省略；仅 15 个已建模变量 |
| `SPACETRANS` | **完整实现** | `SPACETRANS <distance>` 模型↔图纸换算并报告方向/因子 | 活动视口 = VPMAX 视口否则 index 0；无逐视口选择 |

### Wave 26 续（2026-11-12，classic 面板 / 命令词表控制）

| 命令 | 状态 | 当前行为 | 剩余边界 |
| --- | --- | --- | --- |
| `CLASSICINSERT` | **完整实现** | chrome 面板列出真实块定义（名称/成员数/基点）；Insert 走既有 `INSERT` 点位+名称流程 | 无块元数据的定义不可插入；无直接 payload 插入路径 |
| `CLASSICGROUP` | 部分实现 | chrome 面板列出文档持久编组（`tables/groups.json`）/成员数/成员实体 id | 面板无 create/delete/rename/add/remove 控件；“按组选择成员”仅禁用提示（无宿主替换选择路径） |
| `CLASSICIMAGE` | 部分实现 | chrome 面板只读列出 IMAGE 实体（id/空间/file key/pixels/可见/clip 摘要） | attach/clip/adjust 是独立命令；面板不写库 |
| `REDEFINE` | **完整实现** | 重新启用被 `UNDEFINE` 禁用的内建命令；essential 双向拒绝 | 会话状态；不持久化 DWG |
| `UNDEFINE` | **完整实现** | 会话内禁用内建命令；`command.undefined: <NAME>`；别名指向禁用目标亦禁用 | 会话状态；不持久化 DWG |
| `ALIASEDIT` | **完整实现** | `ADD`/`REMOVE`/`LIST`；运行时别名先于静态表解析并遮蔽内建；仅删除运行时行；`ADD`/`REMOVE` 持久化到宿主 `preference.json` 根键 `commandAliases` 并在新会话恢复，写入失败回滚 | 无交互对话框（ADR 全局范围）；不解析 `acad.pgp` 文件；原生宿主同一 pre-tick 批次内新增并使用需等下一 tick |
## Wave 27 边界更新（2026-11-12）

> 以下条目**取代正文中同名命令的旧界**。证据仅 Linux 编译 + Rust 契约测试 + React 单测；
> GUI/GPU/像素/真机 NOT VERIFIED，Windows/macOS/Android/Web NOT VERIFIED。

| 命令 | 状态 | 当前行为 | 剩余边界 |
| --- | --- | --- | --- |
| `RECOVER` | 部分实现 | 只读：当前文档逐实体 `validate_entity`，`recover.summary` + 每个无效实体 `recover.invalid_entity`（ASCII 安全） | 磁盘 DWG 修复/重新打开是宿主文件 I/O + importer 级 salvage，未建模（`recover.file_level_not_modeled`）；不写库/不升修订/不入历史 |
| `RECOVERALL` | 部分实现 | 同上单文档扫描 | 本构建单文档、无 xref 批处理（`recoverall.single_document`） |
| `PURGEAECDATA` | 部分实现 | 只读能力报告：无 AEC 对象模型，无数据可清理（`purgeaecdata.no_aec_model`） | 不导入/不代理 AEC 对象；不写库 |
| `SHOWURLS` | 部分实现 | 只读：报告数据库不保留超链接/URL（`showurls.not_retained`） | 无 hyperlink 存储；不虚构 URL 行、不编辑 |
| `TOOLPALETTES` | 部分实现 | chrome 本地非模态浮窗；Blocks 页列出真实块定义并启动既有 INSERT/`cad_set_tool_text` 流程 | 只有 Blocks 页；无工具/命令工具内容；不进入 `cad_command`；视觉/像素 NOT VERIFIED |
| `TOOLPALETTESCLOSE` | **完整实现** | chrome 本地选择子：关闭浮窗（幂等） | chrome 本地，不进入 `cad_command` |
| `TPNAVIGATE` | 部分实现 | `TPNAVIGATE Blocks`（大小写/空白不敏感）打开或聚焦 Blocks 页并取得键盘 | 其它页名与裸词走 `toolpalettes.refusal.*` 错误行；不转发 `cad_command` |
| `PERFANALYZER` | 部分实现 | chrome 本地非模态面板：只读 `cad_status.performance`（CPU/RSS/GPU/VRAM）与文档/会话计数；null 显示 N/A | 无命令录制/采样/历史；所有数值来自宿主，缺失即 N/A；视觉/像素 NOT VERIFIED |
| `PERFANALYZERCLOSE` | **完整实现** | chrome 本地选择子：关闭面板（幂等） | chrome 本地，不进入 `cad_command` |

### Wave 27 具名拒绝（91 条）

> 以下 91 个名字只提供诚实命令路径（`CadError::Unsupported` + 稳定原因串），不是可用功能：
> 本构建没有 3D 实体/曲面/剖切、点云、render/灯光/视觉样式/视图基、云共享、图纸集、
> 外部参照比较、部分打开、块/外部参照 clip 边界或 clipboard 特殊粘贴模型。全部 Viewer-legal：
> 不写库、不升修订、不入历史。唯一权威是 `docs/autocad-2026-command-coverage.md`；此表只复述原因串。

| 命令 | 结果 | 明示未实现 / 证据边界 |
| --- | --- | --- |
| `PCEXTRACTCENTERLINE` | **部分实现（具名拒绝）**：`pcextractcenterline.unsupported: no point-cloud extraction model` | 不写库/不升修订/不入历史 |
| `PCEXTRACTCORNER` | **部分实现（具名拒绝）**：`pcextractcorner.unsupported: no point-cloud extraction model` | 不写库/不升修订/不入历史 |
| `PCEXTRACTEDGE` | **部分实现（具名拒绝）**：`pcextractedge.unsupported: no point-cloud extraction model` | 不写库/不升修订/不入历史 |
| `PCEXTRACTSECTION` | **部分实现（具名拒绝）**：`pcextractsection.unsupported: no point-cloud extraction model` | 不写库/不升修订/不入历史 |
| `PFACE` | **部分实现（具名拒绝）**：`pface.unsupported: no 3DFACE/mesh entity model` | 不写库/不升修订/不入历史 |
| `PLANESURF` | **部分实现（具名拒绝）**：`planesurf.unsupported: no planar-surface entity model` | 不写库/不升修订/不入历史 |
| `PROJECTGEOMETRY` | **部分实现（具名拒绝）**：`projectgeometry.unsupported: no 3D solid edge/geometry projection model` | 不写库/不升修订/不入历史 |
| `TABSURF` | **部分实现（具名拒绝）**：`tabsurf.unsupported: no 3D surface (NURBS) model` | 不写库/不升修订/不入历史 |
| `REVSURF` | **部分实现（具名拒绝）**：`revsurf.unsupported: no 3D surface (NURBS) model` | 不写库/不升修订/不入历史 |
| `RULESURF` | **部分实现（具名拒绝）**：`rulesurf.unsupported: no 3D surface (NURBS) model` | 不写库/不升修订/不入历史 |
| `XEDGES` | **完整实现**：存储的 `BodyDisplay.edges` 逐段复制为 `LINE` 实体（同层/同空间、源 body 保留、零长段跳过），一条事务/一步撤销；空选择/非 body/子元素/无显示边按名拒绝 | 不焊接、不去重、不删除源；证据仅 Linux 契约，GPU/像素 NOT VERIFIED |
| `SECTION` | **部分实现（具名拒绝）**：`section.unsupported: no 3D section-plane model` | 不写库/不升修订/不入历史 |
| `SECTIONPLANE` | **部分实现（具名拒绝）**：`sectionplane.unsupported: no 3D section-plane model` | 不写库/不升修订/不入历史 |
| `SECTIONPLANEJOG` | **部分实现（具名拒绝）**：`sectionplanejog.unsupported: no 3D section-plane model` | 不写库/不升修订/不入历史 |
| `SECTIONPLANESETTINGS` | **部分实现（具名拒绝）**：`sectionplanesettings.unsupported: no 3D section-plane model` | 不写库/不升修订/不入历史 |
| `SECTIONPLANETOBLOCK` | **部分实现（具名拒绝）**：`sectionplanetoblock.unsupported: no 3D section-plane model` | 不写库/不升修订/不入历史 |
| `SECTIONSPINNERS` | **部分实现（具名拒绝）**：`sectionspinners.unsupported: no 3D section-plane model` | 不写库/不升修订/不入历史 |
| `SOLIDEDIT` | **部分实现（具名拒绝）**：`solidedit.unsupported: no 3D solid-history/brep model` | 不写库/不升修订/不入历史 |
| `SOLPROF` | **部分实现（具名拒绝）**：`solprof.unsupported: no 3D solid-to-2D drawing view model` | 不写库/不升修订/不入历史 |
| `SOLVIEW` | **部分实现（具名拒绝）**：`solview.unsupported: no 3D solid-to-2D drawing view model` | 不写库/不升修订/不入历史 |
| `SOLDRAW` | **部分实现（具名拒绝）**：`soldraw.unsupported: no 3D solid-to-2D drawing view model` | 不写库/不升修订/不入历史 |
| `-POINTCLOUDATTACH` | **部分实现（具名拒绝）**：`-pointcloudattach.unsupported: no point-cloud attachment/crop/style model` | 不写库/不升修订/不入历史 |
| `POINTCLOUDATTACH` | **部分实现（具名拒绝）**：`pointcloudattach.unsupported: no point-cloud attachment/crop/style model` | 不写库/不升修订/不入历史 |
| `POINTCLOUDCOLORMAP` | **部分实现（具名拒绝）**：`pointcloudcolormap.unsupported: no point-cloud attachment/crop/style model` | 不写库/不升修订/不入历史 |
| `POINTCLOUDCROP` | **部分实现（具名拒绝）**：`pointcloudcrop.unsupported: no point-cloud attachment/crop/style model` | 不写库/不升修订/不入历史 |
| `POINTCLOUDCROPSTATE` | **部分实现（具名拒绝）**：`pointcloudcropstate.unsupported: no point-cloud attachment/crop/style model` | 不写库/不升修订/不入历史 |
| `-POINTCLOUDMANAGER` | **部分实现（具名拒绝）**：`-pointcloudmanager.unsupported: no point-cloud attachment/crop/style model` | 不写库/不升修订/不入历史 |
| `POINTCLOUDMANAGER` | **部分实现（具名拒绝）**：`pointcloudmanager.unsupported: no point-cloud attachment/crop/style model` | 不写库/不升修订/不入历史 |
| `POINTCLOUDMANAGERCLOSE` | **部分实现（具名拒绝）**：`pointcloudmanagerclose.unsupported: no point-cloud attachment/crop/style model` | 不写库/不升修订/不入历史 |
| `POINTCLOUDSTYLIZE` | **部分实现（具名拒绝）**：`pointcloudstylize.unsupported: no point-cloud attachment/crop/style model` | 不写库/不升修订/不入历史 |
| `POINTCLOUDUNCROP` | **部分实现（具名拒绝）**：`pointclouduncrop.unsupported: no point-cloud attachment/crop/style model` | 不写库/不升修订/不入历史 |
| `-RENDER` | **部分实现（具名拒绝）**：`-render.unsupported: no render engine; only the 2D viewport renderer exists` | 不写库/不升修订/不入历史 |
| `RENDER` | **部分实现（具名拒绝）**：`render.unsupported: no render engine; only the 2D viewport renderer exists` | 不写库/不升修订/不入历史 |
| `RENDERCROP` | **部分实现（具名拒绝）**：`rendercrop.unsupported: no render engine; only the 2D viewport renderer exists` | 不写库/不升修订/不入历史 |
| `RENDERENVIRONMENT` | **部分实现（具名拒绝）**：`renderenvironment.unsupported: no render engine; only the 2D viewport renderer exists` | 不写库/不升修订/不入历史 |
| `RENDERENVIRONMENTCLOSE` | **部分实现（具名拒绝）**：`renderenvironmentclose.unsupported: no render engine; only the 2D viewport renderer exists` | 不写库/不升修订/不入历史 |
| `RENDEREXPOSURE` | **部分实现（具名拒绝）**：`renderexposure.unsupported: no render engine; only the 2D viewport renderer exists` | 不写库/不升修订/不入历史 |
| `RENDEREXPOSURECLOSE` | **部分实现（具名拒绝）**：`renderexposureclose.unsupported: no render engine; only the 2D viewport renderer exists` | 不写库/不升修订/不入历史 |
| `RENDERONLINE` | **部分实现（具名拒绝）**：`renderonline.unsupported: no render engine; only the 2D viewport renderer exists` | 不写库/不升修订/不入历史 |
| `RENDERPRESETS` | **部分实现（具名拒绝）**：`renderpresets.unsupported: no render engine; only the 2D viewport renderer exists` | 不写库/不升修订/不入历史 |
| `RENDERPRESETSCLOSE` | **部分实现（具名拒绝）**：`renderpresetsclose.unsupported: no render engine; only the 2D viewport renderer exists` | 不写库/不升修订/不入历史 |
| `RENDERWIN` | **部分实现（具名拒绝）**：`renderwin.unsupported: no render engine; only the 2D viewport renderer exists` | 不写库/不升修订/不入历史 |
| `RENDERWINDOW` | **部分实现（具名拒绝）**：`renderwindow.unsupported: no render engine; only the 2D viewport renderer exists` | 不写库/不升修订/不入历史 |
| `RENDERWINDOWCLOSE` | **部分实现（具名拒绝）**：`renderwindowclose.unsupported: no render engine; only the 2D viewport renderer exists` | 不写库/不升修订/不入历史 |
| `RPREF` | **部分实现（具名拒绝）**：`rpref.unsupported: no render engine; only the 2D viewport renderer exists` | 不写库/不升修订/不入历史 |
| `RPREFCLOSE` | **部分实现（具名拒绝）**：`rprefclose.unsupported: no render engine; only the 2D viewport renderer exists` | 不写库/不升修订/不入历史 |
| `SHOWRENDERGALLERY` | **部分实现（具名拒绝）**：`showrendergallery.unsupported: no render engine; only the 2D viewport renderer exists` | 不写库/不升修订/不入历史 |
| `-VISUALSTYLES` | **部分实现（具名拒绝）**：`-visualstyles.unsupported: no visual-style/3D display model; VSCURRENT owns the 2D display token` | 不写库/不升修订/不入历史 |
| `VISUALSTYLES` | **部分实现（具名拒绝）**：`visualstyles.unsupported: no visual-style/3D display model; VSCURRENT owns the 2D display token` | 不写库/不升修订/不入历史 |
| `VISUALSTYLESCLOSE` | **部分实现（具名拒绝）**：`visualstylesclose.unsupported: no visual-style/3D display model; VSCURRENT owns the 2D display token` | 不写库/不升修订/不入历史 |
| `POINTLIGHT` | **部分实现（具名拒绝）**：`pointlight.unsupported: no 3D light model` | 不写库/不升修订/不入历史 |
| `SPOTLIGHT` | **部分实现（具名拒绝）**：`spotlight.unsupported: no 3D light model` | 不写库/不升修订/不入历史 |
| `WEBLIGHT` | **部分实现（具名拒绝）**：`weblight.unsupported: no 3D light model` | 不写库/不升修订/不入历史 |
| `TARGETPOINT` | **部分实现（具名拒绝）**：`targetpoint.unsupported: no 3D light model` | 不写库/不升修订/不入历史 |
| `SUNPROPERTIES` | **部分实现（具名拒绝）**：`sunproperties.unsupported: no sun/sky lighting model` | 不写库/不升修订/不入历史 |
| `SUNPROPERTIESCLOSE` | **部分实现（具名拒绝）**：`sunpropertiesclose.unsupported: no sun/sky lighting model` | 不写库/不升修订/不入历史 |
| `-SHADEMODE` | **部分实现（具名拒绝）**：`-shademode.unsupported: no 3D shaded display mode; VSCURRENT owns the 2D display token` | 不写库/不升修订/不入历史 |
| `SHADEMODE` | **部分实现（具名拒绝）**：`shademode.unsupported: no 3D shaded display mode; VSCURRENT owns the 2D display token` | 不写库/不升修订/不入历史 |
| `VIEWBASE` | **部分实现（具名拒绝）**：`viewbase.unsupported: no 3D model-to-drawing base/detail/section view model` | 不写库/不升修订/不入历史 |
| `VIEWCOMPONENT` | **部分实现（具名拒绝）**：`viewcomponent.unsupported: no 3D model-to-drawing base/detail/section view model` | 不写库/不升修订/不入历史 |
| `VIEWDETAIL` | **部分实现（具名拒绝）**：`viewdetail.unsupported: no 3D model-to-drawing base/detail/section view model` | 不写库/不升修订/不入历史 |
| `VIEWDETAILSTYLE` | **部分实现（具名拒绝）**：`viewdetailstyle.unsupported: no 3D model-to-drawing base/detail/section view model` | 不写库/不升修订/不入历史 |
| `VIEWEDIT` | **部分实现（具名拒绝）**：`viewedit.unsupported: no 3D model-to-drawing base/detail/section view model` | 不写库/不升修订/不入历史 |
| `VIEWPROJ` | **部分实现（具名拒绝）**：`viewproj.unsupported: no 3D model-to-drawing base/detail/section view model` | 不写库/不升修订/不入历史 |
| `VIEWSECTION` | **部分实现（具名拒绝）**：`viewsection.unsupported: no 3D model-to-drawing base/detail/section view model` | 不写库/不升修订/不入历史 |
| `VIEWSECTIONSTYLE` | **部分实现（具名拒绝）**：`viewsectionstyle.unsupported: no 3D model-to-drawing base/detail/section view model` | 不写库/不升修订/不入历史 |
| `VIEWSETPROJ` | **部分实现（具名拒绝）**：`viewsetproj.unsupported: no 3D model-to-drawing base/detail/section view model` | 不写库/不升修订/不入历史 |
| `VIEWSKETCHCLOSE` | **部分实现（具名拒绝）**：`viewsketchclose.unsupported: no 3D model-to-drawing base/detail/section view model` | 不写库/不升修订/不入历史 |
| `VIEWSTD` | **部分实现（具名拒绝）**：`viewstd.unsupported: no 3D model-to-drawing base/detail/section view model` | 不写库/不升修订/不入历史 |
| `VIEWSYMBOLSKETCH` | **部分实现（具名拒绝）**：`viewsymbolsketch.unsupported: no 3D model-to-drawing base/detail/section view model` | 不写库/不升修订/不入历史 |
| `VIEWUPDATE` | **部分实现（具名拒绝）**：`viewupdate.unsupported: no 3D model-to-drawing base/detail/section view model` | 不写库/不升修订/不入历史 |
| `SHARE` | **部分实现（具名拒绝）**：`share.unsupported: no Autodesk cloud sharing/view model` | 不写库/不升修订/不入历史 |
| `SHAREDVIEWS` | **部分实现（具名拒绝）**：`sharedviews.unsupported: no Autodesk cloud sharing/view model` | 不写库/不升修订/不入历史 |
| `SHAREDVIEWSCLOSE` | **部分实现（具名拒绝）**：`sharedviewsclose.unsupported: no Autodesk cloud sharing/view model` | 不写库/不升修订/不入历史 |
| `-SHAREVIEW` | **部分实现（具名拒绝）**：`-shareview.unsupported: no Autodesk cloud sharing/view model` | 不写库/不升修订/不入历史 |
| `SHAREVIEW` | **部分实现（具名拒绝）**：`shareview.unsupported: no Autodesk cloud sharing/view model` | 不写库/不升修订/不入历史 |
| `PUSHTODOCSOPEN` | **部分实现（具名拒绝）**：`pushtodocsopen.unsupported: no Autodesk Docs connector model` | 不写库/不升修订/不入历史 |
| `PUSHTODOCSCLOSE` | **部分实现（具名拒绝）**：`pushtodocsclose.unsupported: no Autodesk Docs connector model` | 不写库/不升修订/不入历史 |
| `SHEETSET` | **部分实现（具名拒绝）**：`sheetset.unsupported: no sheet-set (DST) model` | 不写库/不升修订/不入历史 |
| `OPENSHEETSET` | **部分实现（具名拒绝）**：`opensheetset.unsupported: no sheet-set (DST) model` | 不写库/不升修订/不入历史 |
| `SHEETSETHIDE` | **部分实现（具名拒绝）**：`sheetsethide.unsupported: no sheet-set (DST) model` | 不写库/不升修订/不入历史 |
| `XCOMPARE` | **部分实现（具名拒绝）**：`xcompare.unsupported: no xref comparison/diff model` | 不写库/不升修订/不入历史 |
| `XCOMPARECLOSE` | **部分实现（具名拒绝）**：`xcompareclose.unsupported: no xref comparison/diff model` | 不写库/不升修订/不入历史 |
| `XCOMPARERCNEXT` | **部分实现（具名拒绝）**：`xcomparercnext.unsupported: no xref comparison/diff model` | 不写库/不升修订/不入历史 |
| `XCOMPARERCPREV` | **部分实现（具名拒绝）**：`xcomparercprev.unsupported: no xref comparison/diff model` | 不写库/不升修订/不入历史 |
| `PARTIALOPEN` | **部分实现（具名拒绝）**：`partialopen.unsupported: no partial layer-index open/load model` | 不写库/不升修订/不入历史 |
| `PARTIALOAD` | **部分实现（具名拒绝）**：`partiaload.unsupported: no partial layer-index open/load model` | 不写库/不升修订/不入历史 |
| `SHELL` | **部分实现（具名拒绝）**：`shell.unsupported: the host owns process spawning; this command layer has no OS shell` | 不写库/不升修订/不入历史 |
| `SECURITYOPTIONS` | **部分实现（具名拒绝）**：`securityoptions.unsupported: no security/executable-trust settings model` | 不写库/不升修订/不入历史 |
| `XCLIP` | 完整实现 | 真实（display-only）+ **导入/导出双向**：矩形/旋转矩形/任意简单多边形真实裁剪（嵌套取交集）；**裁剪深度** `front/back` 存块局部 z 区间（`None` 逐字节不变），DXF `40`/`41`+`72`/`73` 往返，显示真过滤（跨区间整块保留并具名）；导入解析 `ACAD_FILTER`/`SPATIAL_FILTER` 链（含 `display_enabled`），导出写同一链 ⇒ 导入→导出→再导入边界/`visible`/`inside`/深度相等；`XCLIP CLIPDEPTH` 一条事务/一步撤销 | 声明边界：`INVERT` 无模式字段（按名拒绝）；`40`/`41` 按 z 区间读写、不镜像 AutoCAD 的「back 沿 −normal」约定；跨区间不按深度面切割（2D 优先，逐图元具名）；`CLIP POLY` 对块参照仍是 `CLIP` 的边界 |
| `PASTESPEC` | **部分实现（具名拒绝）**：`pastespec.unsupported: clipboard special-paste (OLE/format selection) is not modeled; use PASTECLIP or PASTEBLOCK` | 不写库/不升修订/不入历史 |
## Wave 28 边界更新（2026-11-12，官方名收尾：39 条具名拒绝）

> 以下 39 个名字**在 Wave 28 时**只提供诚实命令路径（`CadError::Unsupported` + 稳定原因串），不是可用功能；
> 其中 `REFEDIT` 族已在 2026-11-12 升级为真实实现、2026-11-24 严格收口（见本节之后的
> 「REFEDIT / REFSET / REFCLOSE」小节），其余仍是具名拒绝。官方 981 条至此**没有未实现的可用命令名**；
> 仅 `MEASURE`/`TRACE` 因同名不同义不计入。全部 Viewer-legal：不写库、不升修订、不入历史。

| 命令 | 结果 | 明示未实现 / 证据边界 |
| --- | --- | --- |
| `OPENFROMWEBMOBILE` | **部分实现（具名拒绝）**：`openfromwebmobile.unsupported: no Autodesk Account (web/mobile) file-exchange model` | 不写库/不升修订/不入历史 |
| `-PARAMETERS` | **完整实现**：命令行列/编辑器 `NEW|SET|RENAME|DELETE` + 表达式语法；读形式输出参数表；一条事务/一步撤销 | 表达式语法没有函数调用；求解器驱动的约束参数未建模 |
| `PARAMETERS` | **完整实现**：chrome-local Parameters Manager 面板（列表、创建/重命名/删除、值/表达式编辑；一条事务/一步撤销） | 求解器驱动的二维约束参数未建模；面板像属性面板一样在无 palettes 的载体上不挂载，但 `-PARAMETERS` 仍可用 |
| `PARAMETERSCLOSE` | **完整实现**：幂等关闭 Parameters Manager 面板 | 再次关闭仍成功 |
| `-PARTIALOAD` | **部分实现（具名拒绝）**：`-partiaload.unsupported: no partial layer-index open/load model` | 不写库/不升修订/不入历史 |
| `PASTEASHYPERLINK` | **部分实现（具名拒绝）**：`pasteashyperlink.unsupported: no hyperlink storage model` | 不写库/不升修订/不入历史 |
| `PSOUT` | **部分实现（具名拒绝）**：`psout.unsupported: PostScript output is not implemented; use PLOT for SVG/PDF` | 不写库/不升修订/不入历史 |
| `QUICKCUI` | **部分实现（具名拒绝）**：`quickcui.unsupported: no CUI (menu-customization) model` | 不写库/不升修订/不入历史 |
| `REDIR` | **部分实现（具名拒绝）**：`redir.unsupported: no xref/image path-redirection model` | 不写库/不升修订/不入历史 |
| `REDIRMODE` | **部分实现（具名拒绝）**：`redirmode.unsupported: no xref/image path-redirection model` | 不写库/不升修订/不入历史 |
| `REFCLOSE` | **完整实现**（2026-11-24 严格收口；Wave 28 时曾为 `refclose.unsupported`） | 见后文「REFEDIT / REFSET / REFCLOSE」小节 |
| `-REFEDIT` | **完整实现**（2026-11-24 严格收口；Wave 28 时曾为 `-refedit.unsupported`） | 见后文「REFEDIT / REFSET / REFCLOSE」小节 |
| `REFEDIT` | **完整实现**（2026-11-24 严格收口；Wave 28 时曾为 `refedit.unsupported`） | 见后文「REFEDIT / REFSET / REFCLOSE」小节 |
| `REFSET` | **完整实现**（2026-11-24 严格收口；Wave 28 时曾为 `refset.unsupported`） | 见后文「REFEDIT / REFSET / REFCLOSE」小节 |
| `REPURLS` | **部分实现（具名拒绝）**：`repurls.unsupported: no hyperlink storage model` | 不写库/不升修订/不入历史 |
| `RESUME` | **部分实现（具名拒绝）**：`resume.unsupported: no script-file engine` | 不写库/不升修订/不入历史 |
| `RSCRIPT` | **部分实现（具名拒绝）**：`rscript.unsupported: no script-file engine` | 不写库/不升修订/不入历史 |
| `SCRIPTCALL` | **部分实现（具名拒绝）**：`scriptcall.unsupported: no script-file engine` | 不写库/不升修订/不入历史 |
| `RTUCS` | **部分实现（具名拒绝）**：`rtucs.unsupported: no dynamic UCS model` | 不写库/不升修订/不入历史 |
| `SAVEIMG` | **部分实现（具名拒绝）**：`saveimg.unsupported: raster image export is host-owned; this command layer has no file handle (use PLOT PNG)` | 不写库/不升修订/不入历史 |
| `SAVETOWEBMOBILE` | **部分实现（具名拒绝）**：`savetowebmobile.unsupported: no Autodesk Account (web/mobile) file-exchange model` | 不写库/不升修订/不入历史 |
| `STANDARDS` | **部分实现（具名拒绝）**：`standards.unsupported: no CAD standards (DWS) model` | 不写库/不升修订/不入历史 |
| `TRACEBACK` | **部分实现（具名拒绝）**：`traceback.unsupported: no collaboration trace model` | 不写库/不升修订/不入历史 |
| `TRACEEDIT` | **部分实现（具名拒绝）**：`traceedit.unsupported: no collaboration trace model` | 不写库/不升修订/不入历史 |
| `TRACEFRONT` | **部分实现（具名拒绝）**：`tracefront.unsupported: no collaboration trace model` | 不写库/不升修订/不入历史 |
| `TRACEPALETTECLOSE` | **部分实现（具名拒绝）**：`tracepaletteclose.unsupported: no collaboration trace model` | 不写库/不升修订/不入历史 |
| `TRACEPALETTEOPEN` | **部分实现（具名拒绝）**：`tracepaletteopen.unsupported: no collaboration trace model` | 不写库/不升修订/不入历史 |
| `TRACEVIEW` | **部分实现（具名拒绝）**：`traceview.unsupported: no collaboration trace model` | 不写库/不升修订/不入历史 |
| `ULAYERS` | **部分实现（具名拒绝）**：`ulayers.unsupported: no DWF/PDF/DGN underlay layer-display model` | 不写库/不升修订/不入历史 |
| `VBAMAN` | **部分实现（具名拒绝）**：`vbaman.unsupported: no VBA runtime/IDE` | 不写库/不升修订/不入历史 |
| `VBAPREF` | **部分实现（具名拒绝）**：`vbapref.unsupported: no VBA runtime/IDE` | 不写库/不升修订/不入历史 |
| `VBASTMT` | **部分实现（具名拒绝）**：`vbastmt.unsupported: no VBA runtime/IDE` | 不写库/不升修订/不入历史 |
| `VIEWPLAY` | **部分实现（具名拒绝）**：`viewplay.unsupported: no named-view animation model` | 不写库/不升修订/不入历史 |
| `VLISP` | **部分实现（具名拒绝）**：`vlisp.unsupported: no AutoLISP runtime/IDE` | 不写库/不升修订/不入历史 |
| `VSLIDE` | **部分实现（具名拒绝）**：`vslide.unsupported: no slide (.sld) format model` | 不写库/不升修订/不入历史 |
| `VSSAVE` | **部分实现（具名拒绝）**：`vssave.unsupported: no slide (.sld) format model` | 不写库/不升修订/不入历史 |
| `WALKFLYSETTINGS` | **部分实现（具名拒绝）**：`walkflysettings.unsupported: no 3D walk/fly navigation model` | 不写库/不升修订/不入历史 |
| `WEBLOAD` | **部分实现（具名拒绝）**：`webload.unsupported: no JavaScript/web-load runtime` | 不写库/不升修订/不入历史 |
| `WMFOPTS` | **部分实现（具名拒绝）**：`wmfopts.unsupported: no Windows metafile import model (WMFIN is not implemented)` | 不写库/不升修订/不入历史 |
### 2026-11-12 HATCHEDIT 图案名（Wave 29b）

| 命令 | 状态 | 当前行为 | 剩余边界 |
| --- | --- | --- | --- |
| `HATCHEDIT` | 部分实现 | 新增 `PATTERN <name>`：按名称从现有 `.pat` 库替换图案填充，保留 hatch 的 angle/scale/double，一条事务/一步撤销 | 任意用户 `.pat` 图案文件需宿主文件 I/O；未知名/缺名/实心/渐变分别 `hatchedit.unknown_pattern`/`needs_pattern_name`/`pattern_requires_pattern`；无交互对话框 |
### 2026-11-12 REFEDIT / REFSET / REFCLOSE（有界 in-place 块参照编辑；2026-11-24 严格收口）

| 命令 | 状态 | 当前行为 | 剩余边界 |
| --- | --- | --- | --- |
| `REFEDIT`/`-REFEDIT` | **完整实现** | 仅接受一个直接可寻址、相似变换（均匀缩放+旋转+平移、非镜像）的模型空间 INSERT；把**可见**块成员按 INSERT 变换克隆到模型空间（一条事务），源 INSERT 进入会话隐藏集；选择变为克隆。`refedit.selection`/`nested`/`not_insert`/`non_uniform`/`active`/`block_missing`/`clone_failed`/`order_exhausted` 等具名拒绝；嵌套 INSERT 成员按引用保留（合成放置/逆变换写回），嵌套实例选择 `refedit.nested` 拒绝 | 全局范围：参照淡显/锁定显示概念（其他宿主对象仍可选，REFSET 是显式选择）、交互拾取提示/对话框、外部参照编辑子系统（本构建无 xref 编辑会话）。隐藏的动态可见性成员不提取、不改写且描述符保留；只在应用层阻止 QSAVE（`refedit.save_blocked`）；会话状态不可撤销（撤销删光克隆会清理会话并恢复源可见性） |
| `REFSET` | **完整实现** | `ADD|REMOVE`（裸=ADD）把当前选择加入/移出工作集；会话状态、不写图/不升修订/不入历史；`refedit.no_session`/`selection_empty`/`nested` 拒绝；自引用块实例拒绝；重复 ADD/REMOVE 是计数为 0 的空操作 | 交互拾取提示属全局范围；ADD 的宿主对象在 SAVE 时是**移入**块（同 id），REMOVE 在 SAVE 时删除原成员、模型空间副本保留——这是 AutoCAD 的 ADD 语义，不是缺口 |
| `REFCLOSE` | **完整实现** | `SAVE` 逆变换把工作集写回块定义：成员几何、**图层**（取克隆）、**绘图次序**（克隆相对序在既有成员槽位上置换）、变更的**渲染属性**；新增宿主对象移入块、移出成员删除并从动态可见性描述符剪除其 id；删除克隆，一条事务/一步撤销。`DISCARD` 删除克隆且块定义不变；`refclose.needs_option` 拒绝裸命令 | 交互式 Save/Discard 提示属全局范围；外部参照编辑子系统未建模（构建级范围）；无参照文件回写（本构建无 xref 编辑） |
### 2026-11-12 TEXTALIGN 15 模式 + DISTRIBUTE

| 命令 | 状态 | 当前行为 | 剩余边界 |
| --- | --- | --- | --- |
| `TEXTALIGN` | 完整实现 | 15 个对齐拼写按并集包围盒两轴对齐；`DISTRIBUTE [X\|Y]`（默认 Y）按阅读序等间隙（首尾固定、≥3）；`SETSPACING <d> [X\|Y]` 固定间距（首固定）；`CURRENTVERTICAL`/`CURRENTHORIZONTAL` 按各文本自身 justification 对齐到首个目标；只移动参照点，一条事务/一步撤销 | 无交互 base-object + second-point 步骤（并集包围盒为确定性替代）；宽高为 `height × 字符数` 估算，忽略旋转 |
### 2026-11-12 XCLIP（display-only 块参照裁剪）

| 命令 | 状态 | 当前行为 | 剩余边界 |
| --- | --- | --- | --- |
| `XCLIP` | 部分实现 | `RECT <minx> <miny> <maxx> <maxy>` / `POLY <x1> <y1> ...` / `ON` / `OFF` / `DELETE` / `?`，作用于一个直接可寻址的模型空间 INSERT；边界以**块局部坐标**存于 `EntityRenderAttributes.clip`（一条事务/一步撤销，STYLE 变更失效）；表示层在块展开时按 INSERT 变换映射为世界轴对齐矩形，用既有精确裁剪器裁 LINE/ARC/圆/MESH/IMAGE，嵌套 clip 取交集，完全在外时不产生 fragment | **display-only**：拾取/选择/ZOOM 范围仍按未裁剪几何；无 DXF/DWG 持久化（导入不填、导出丢弃）；仅 0/90/180/270 世界旋转可渲染（其它 `xclip.rotation_not_renderable`）；多边形只存不渲染（`xclip.polygon_not_renderable`）；`INVERT` 拒绝；TEXT/MASK 保留并报告 `representation.xclip_primitive_unsupported`；`XCLIP ?` 只报告当前选中的一个 INSERT |
### 2026-11-12 TEXTFIT 真实宽度因子（group 41）

| 命令 | 状态 | 当前行为 | 剩余边界 |
| --- | --- | --- | --- |
| `TEXTFIT` | 部分实现 | 只改存储 `SemanticGeometry::Text.width_factor`（DXF group 41）；字高/位置/justification/旋转/内容/样式逐位保留；因子 = `target_width / 批量估算宽度`，逐实体诊断 `textfit.estimated_width`；空串/非正目标/退化因子具名拒绝；一条事务/一步撤销；表示层 shaping、bounds/pick 均按因子拉伸 | 因子来自 `height × 字符数` 估算（非真实字形度量），渲染宽度只在估算精度内等于目标；MTEXT 无 group 41 导出通道；ATTRIB/ATTDEF/RTEXT/表格/标注派生文字保持 1.0；仅导入 TEXT group 41 |
### 2026-11-12 PLINE / PEDIT 统一宽度

| 命令 | 状态 | 当前行为 | 剩余边界 |
| --- | --- | --- | --- |
| `PLINE` | 已实现 | `Geometry(Polyline)` 与捕获路径共享一个统一 `width`：捕获读 `polylineWidth` 工具参数（默认 0=细线），程序化 payload 自带 `width`；表示层以中心线 ±w/2 生成实心带宽（斜接，超过 4×半宽转斜切；开放端平头；闭合环回绕），并参与 XCLIP 矩形裁剪；相似变换缩放宽度 | 无逐段 start/end 锥形宽度（DXF 40/41）；宽虚线仍画实心并报告 `representation.polyline_width_dashes`；SPLINE/DECURVE 后宽度归 0；OFFSET/BREAK/JOIN/STRETCH/FILLET/LENGTHEN 派生折线仍从 0 开始 |
| `PEDIT` | 部分实现 | `WIDTH <w>` 对多选写统一宽度（0 复位细线），一条事务/一步撤销；CLOSE/OPEN/REVERSE/FIT/顶点编辑保留宽度；非法值 `pedit.width_invalid`，无变化 `pedit.width_unchanged` | 每段锥形宽度（`pedit.vertex_width_unsupported`）；`Ltype gen`；SPLINE 实体无宽度字段 |
### 2026-11-12 EXTEND 开放 SPLINE 目标

| 命令 | 状态 | 当前行为 | 剩余边界 |
| --- | --- | --- | --- |
| `EXTEND` | 部分实现 | 开放 SPLINE 目标沿**拾取端切线的直线尾段**延伸：保留原 degree/weights/控制点，追加 degree+1 个共线控制点与新端节点（C0/G1，与 JOIN 的拼接规则一致），交点取自边界集合、只接受切线射线前方最近命中；一条事务/一步撤销 | 闭合/首尾重合样条 `extend.spline_closed`；非钳制端点 `extend.spline_unclamped`；只查切线射线（内部穿过但不与射线相交的边界不可见）；非共面边界与普通未命中无法区分（`extend.no_intersection`）；无任意长度上限 |
