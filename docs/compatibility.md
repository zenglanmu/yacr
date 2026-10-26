# 兼容性与能力矩阵

> **历史横幅（2026-10-16，2026-10-23 限定）**：下列横幅原本针对**平台/宿主**段落：`cad-ui-slint`、
> `yacr-linux`、旧脚本命令均已退役。**实体能力矩阵与命令矩阵仍是当前契约**，不要因为横幅而忽略它们。
> 当前架构与构建见 `docs/architecture.md`、`docs/react-linux-app.md`、`docs/windows-app.md`、
> `docs/build.md`、[ADR 0010](adr/0010-react-linux-host-replaces-slint.md) 与
> [ADR 0023](adr/0023-dom-on-top-cross-host-composition.md)。平台段落已在 2026-10-23 重写（见「平台」）。

**没有任何已验收的 DWG 版本、实体或平台。** `fixtures/manifest` 现含合成夹具与一个
开源 QCAD `flange` 样本（`Partial`，`fixtures/dxf/qcad-flange/`），因此下表的
「实现」仅表示代码路径存在并有契约测试，**不代表对真实图纸兼容**。构建证据
不等于兼容性（规范 §1、§11.4）。2026-10-01 已对 11 份公开样本执行导入端到端
（AC1014–AC1032，见 `docs/validation.md`），仍未构成验收。

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
| LWPOLYLINE/POLYLINE(bulge) | 实现 | 实现 | 实现(含 bulge) | 实现 | 折线长度实现 |
| CIRCLE / ARC / ELLIPSE | 实现 | 实现(保留 normal/OCS)；非均匀仿射重解为真椭圆 | 实现 | 实现 | 距离实现；面积限共面环 |
| SPLINE | 实现 | 实现(保留 degree/knots/weights，真实 NURBS) | 弦高自适应离散 | 实现 | 解析/采样(与显示 LOD 解耦) |
| POINT | 实现 | 实现 | **overlay 屏占比 POINT 标记**（draw_order 850 000，始终在 overlay 层、与基础场景实体遮挡关系不同）；**已实现 `$PDMODE`/`$PDSIZE` 显示样式**（只影响显示）：基础形状 = `0` 点/`1` 无/`2` `+`/`3` `×`/`4` 竖线；`PDSIZE > 0` 绝对图纸尺寸、`= 0` 视口高 2.5% 半幅（即 5% 全幅）、`< 0` 视口高的百分比；`32`/`64`/`96` 周边圆/方**未实现**（画基础形状 + 一条 `overlay.point-mode-shape-bits` 诊断）；`PDMODE 0` 的“点”在 **Lines-only** 拓扑下只能画**与配置尺寸同尺寸的** `+`（近似，一条 `overlay.point-mode-dot` 诊断，**不是**点；刻意**不**缩小——DWG 缺省就是 `0`，缩小会静默改变所有未设样式图纸的点外观）；无法表达的模式 → 按名诊断 `overlay.point-mode-unsupported`、不发明几何；非有限/超限 `PDSIZE` 回退到 `0` 行为（一条 `overlay.point-size-sanitized` 诊断）；覆盖**顶层**导入 POINT；上限 4000 且另发一条诊断；**3D 不画**（`viewport_bounds` 为 `None`，即使 `PDSIZE > 0` 可定义也不画）；**块内 POINT 不标记**（限制已记录） | 实现 | 部分 |
| SOLID / TRACE / 3DFACE | 实现 | 网格(四边形→三角) | 网格 | 实现 | 不支持 |
| INSERT / 块 | 实现(定义与实例分离) | 实现 | 实例展开块几何(嵌套有界，保留 InstancePath) | 实现 | 不支持 |
| TEXT / MTEXT | 实现(保留字体名/字高/旋转) | 实现 | 整形为线段：outline(TTF/OTF/WOFF)+SHX(shapes/unifont/bigfont)，缺失字体走回退链。**可变换（P7a）**：平移/旋转/均匀缩放；**镜像与非均匀缩放拒绝**（平面文字无法表达真反射）；捕捉面 `engine.rs` 同规则（跨面一致性有测试）。**P7a 起 `TEXT` 创建实现**：1 点 + 文本（`cad_set_tool_text`，终端语义、1024 字符上限），`AcDbText`，一条事务/一步撤销。**MTEXT 的列/背景填充/行距样式/绘制方向仍不保留**，导入改为 **`Partial` + 四条具名原因**（此前误报 `Complete`）；**编辑现有文本已实现（A+B）**：`TEXTEDIT`（拾取一个 `Text` → **预填当前串** → 终端文本提交 → `update_entity`，**一条事务/一步撤销**，只改字符串，样式/高度/旋转/位置/对齐保留）+ **属性面板可编辑字段**（当前单选，**不暴露实体 id**，宿主自行解析并拒绝）；**整串替换**（无光标级编辑、不解析 MTEXT 标记）；拒绝面：非 `Text`/块内成员/子元素、空、超 1024、非单选、拾取后目标已变。**MLEADER 文本编辑已实现（P8）**：A（`TEXTEDIT`）/B（面板内联）/C（**弹窗**）三条入口；MTEXT 与 MLEADER 文本多行、`\n`↔`\P`；TEXT 单行且**拒绝换行**（AutoCAD 不解释 `\P`）；其余控制码逐字透传；**`MLeaderText.width` 已参与排版（2026-10-21）**：`MultiLeaderGeometry::text_geometry()` → `SemanticGeometry::Text.wrap_width` → 表示层**贪心换行**（拉丁按词、CJK 按字；`\P` 段落各自独立；超宽不可断词独占一行；对齐按**每行**应用；`None`/`0`/负值/非有限 = 不换行，输出位级不变）。`Partial`：无 UAX #14 断行、无 kinsoku、跨 token 字距不测量、实体包围盒仍用未换行估计；**MTEXT 的 41 组（`rectangle_width`）导入侧仍不保留**。**MTEXT 创建（命令核心，2026-10-26）**：`MTEXT` 选择器 + `CommandPayload::MText { position, width, text, height, rotation, h_align, v_align }` 经**同一条创建漏斗**写 **`AcDbMText`**（漏斗新增**type-key 覆盖**，默认仍是 `class_name()` 的 `AcDbText`；覆盖仅允许单实体，批量覆盖按名拒绝）；**两角宽度模型**：`width` 由调用方从第二角导出，`None` = 不换行、原样存 `None`，`Some(w)` 必须有限且 `> 0` 否则**按名拒绝**（不钳制、不改写为 `None`），合法值经 `wrap_width_from_dxf` 唯一规则；换行按**唯一权威** `\n`/`\r`/`\r\n` → `\P`（与 `TEXTEDIT` 共用 `app_drawing::text_policy`），样式/字体与 `TEXT` 同一解析（`resolve_created_text_style`），一条事务/一步撤销，诊断 `drawing.mtext_created` 明列四项未建模面。**交互 MTEXT 工具已实现（2026-10-26）**：`DrawToolKind::MText`（arity `Exact(2)` + `requires_text`），捕获顺序 = **第一个角点 → 对角点 → 输入文字**；插入点 = 第一个角点，`width = |第二角.x − 第一角.x|`（要求有限且 `> 0`，**退化矩形按名拒绝 `tool.mtext_zero_rect`**，不伪造宽度），`height` = 既有会话参数 `ToolParameter::TextHeight`（复用 TEXT 的默认 2.5，**不新增高度参数**），`rotation = 0.0`，`h_align`/`v_align` = AutoCAD MTEXT 默认**左上附着**（`TextAlignH::Left` + `TextAlignV::Top`）；文字走 `TEXT` **同一条** `cad_set_tool_text` 终端通道（`requires_text` 单一闸门，无第二条捕获路径），多行按共享 `\P` 策略存储。路由：裸 `MTEXT` 选择器 → `BeginDraw(DrawToolKind::MText)`（此前“无载荷、按名拒绝”的合同已废止）；ribbon/命令目录在 TEXT 旁新增 MTEXT（专用 `mtext` 字形，无 variant 弹层）；提示串 `tool.prompt.mtext.{pick_corner,opposite_corner,type_text}`。**显式延期（未实现，无参数/提示/载荷字段）**：用户可设旋转（`TextRotation`）、用户可设附着/对齐、行距、分栏、背景填充、绘制方向、绘制流程内 MTEXT 编辑对话框；**行距样式/分栏/背景填充/绘制方向仍不建模**（即导入 `Partial` 的四条具名原因）。证据上限：仅 `cargo test -p cad-app -p cad-ui-core` + 前端 `npm test`（契约/纯函数），**无 GUI/GPU/像素/真窗口截图证据**。**TEXT 单行诚实性修复**：含换行的 `TEXT` 草案此前被存成 `AcDbText`（渲染多行、且此后 `TEXTEDIT` 必然拒绝），现于捕获层与命令层同时按名拒绝 `text.single_line_newline` | 同上 | 不支持 |
| HATCH（读取/显示） | 实现 | 部分(椭圆/样条边界近似) | **P6a 起为一级 `Hatch` 实体**：边界环 + **样式感知**填充（`Normal`/`Outer`/`Ignore` 按**环集筛选**，1 基深度）+ 实心/图案/渐变；超预算/坏环降 `Partial`。**渲染变化（P6a 正确性修复）**：① `Outer`/`Ignore` 此前一律按 `Normal` 填（`Outer` **只在 ≥3 层**可见、`Ignore` 有岛屿即不同）；② 图案复合修正（**缩放基点、旋转 offset**；属 **U-1 我们的实现**，**非**外部保真验证）；③ 用户自定义图案（`76=0` 且 `78=0`）此前 `Partial`，现由 `52/41/77` **合成**；④ **环深度探测**此前只看环的**首顶点**（`fill::ring_depths`），**相切导入环**可能被判为退化或错填，现改为**最多 8 点均匀探测 + 确定性回退**（与 `loop_depths` 共享同一助手）；创建路径不受影响（先拒绝相切）；⑤ **预定义图案名无行时补行（2026-10-26，`cad-import-acadrust` 单侧）**：`76=1`（predefined）且文件**不带** `78` 定义行时，按名字查 `cad_domain::hatch::predefined_pattern`（trimmed、case-insensitive，不做第二套归一化）命中则用**内置转录表**的行（经 `to_hatch_pattern` 旋转进 hatch 平面）+ 文件自带的 `41`/`52`/`77`，并记稳定诊断 `import.hatch_pattern_table`（消息明说 rows 是 “our transcription, ADR U-1, not vendor-verified”，故图纸级报告因此不再 `Complete`）；**文件自带定义行永远优先**（即使与表不一致也不查表、不重写、不记诊断）；表中没有的名字（`ANSI33`、厂商图案）**不造行**，仍 `Partial` 且原因由通用句扩展为 `…: predefined pattern 'ANSI33' is not in the implemented pattern table`；`76=2`（Custom）**从不**查表 | 精确局部包围盒（**bulge 环退回 AABB**，声明） | 不支持 |
| HATCH（创建） | **实现（P6b 预选边界；图案类型 `2` 与 LINE/ARC 边界拼接于 2026-10-26 增补）** | 部分 | **先选边界对象、再执行 `HATCH`**（与 `ARRAY`/`MOVE` 同型：0 点 arity + `requires_selection`）。接受 `Circle`/完整 `Ellipse`/**闭合** `Polyline`，每个成一环；**`LINE`/`ARC` 边界边**按端点**缝合**成闭合环（容差为**绝对 `1e-6` 世界单位**——本层无图面比例/单位可推相对 eps，超差即真开口，**绝不外插搭桥**；每条边可**任向消费**，混向绘制照样缝合，`ARC` 以 `bulge = tan(sweep/4)` 存为**精确 bulge 段、绝不采样**，`LINE` 为 bulge 0；四分之一弧即 `tan(π/8)`，与 `Circle` 环同一约定；一选择可产出**多环**，两个分离矩形 → 两环）。全部边界须**共面**（圆/椭圆/圆弧另须**法向一致**）。参数：图案类型（`0` 实体 / `1` 用户自定义 / `2` **预定义** = **3 个手工转录**图案 `ANSI31`/`ANSI32`/`ANSI37`，按 **1 基** 索引 `hatchPattern` 选择，闭集 = 表长，越界**按名拒绝**不钳制）、间距（>0；预定义下兼作图案比例 `HPSCALE`）、角度（**通道度、域弧度**）、双线、样式（`Normal`/`Outer`/`Ignore`）。预定义表的数值是**我们的转录值**（ADR U-1 纪律，**非** vendor 核实），经既有 `HatchPattern::families()` 出线，无渲染器改动；`76` 写 **1**。**圆 = 4 段精确圆弧**（`bulge=tan(π/8)`；**非** ADR 原写的两点环——校验器要求 ≥3 顶点，ADR 已更正）；椭圆按 64/圈**采样**（真椭圆非圆弧，bulge 会是假几何）。**内部点求边界、自交环（拒绝，不修剪/不修复）、自定义（`76=2`）图案名与 `.pat` 加载、图案选择对话框、实时预览、渐变创建、关联性、每段裁剪的虚线相位重置均 NOT IMPLEMENTED**。拒绝面：开放折线、样条、椭圆弧、退化 `LINE`/`ARC`（`hatch.boundary_degenerate`）、分支节点（`hatch.boundary_branch`，命名节点坐标与边数，**不猜配对**）、未消费的开放链（`hatch.boundary_open`，命名未消费边数）、自交、相切/重复、部分重叠、退化、非共面、非法参数（含预定义索引越界）、无活动图层、Viewer。**创建校验基于弦**：两段 bulge 环的**弦间**相切可能漏检（声明） | — | 不支持 |
| MULTILEADER | 实现（P7b-1 起为一级 `MultiLeader` 实体） | 部分 | 引线（直/样条）+ 箭头三角 + 狗腿 + 文字；**仅 MText 与无内容**迁入变体，**块/公差内容仍为 `Compound`**（渲染不变）；`90` 掩码原样、`reserved` 仅存逐字源数据。**声明**：样式快照是**解析后的有效值**（原始未置位字段不单独保留）；`EXPLODE`/`STRETCH` **显式整命令拒绝**；缩放/镜像/非均匀**整选择拒绝**；导入恒 `Partial`（颜色/线宽覆盖、背景、分栏、打断、自定义箭头、非默认平面）。**导出已实现（P7b-2）**：真 `MULTILEADER` + `MLEADERSTYLE` 按名去重 + `CLASSES`（由 acadrust 默认注册保证，U-5 已勘误）；**`90` 掩码逐字**；**不写**计算后的文字排版。**声明**：`91/341/171` 写默认；样式为**有效值快照**（掩码抑制字段非逐字往返）；未建模上下文取默认、`text_normal/direction` 派生；seeds 未建模（写读不对称）；per-line 箭头/线型 handle **悬空**；`271–273` 样式级 acadrust 无法表达；**解析但未建模的组**（实体尾 `292/293/294/174/178/295`、上下文 `44/293/295`）**归一化为默认值**并作**常设具名偏差**；**实体级 `342`（自定义箭头块）在导入侧具名**（含与逐线 `341` 不一致的情形）；尾部 `271` 由 root 0 派生。**不宣称"零丢失"**。**块内放置**：刚性 ⇒ 真实体；缩放/镜像 ⇒ **声明式摊平**（不丢内容）。**创建已实现（P7c）**：`MLEADER` = **箭头 → 落点 → 文本**，单簇/单线/2 顶点/仅直线，`Exact(2)` + 文本（**无内容不创建**）；狗腿/缺口/箭头/字高/附着取 `MLeaderStyle` 表（`Standard`，**常量为债务**）；`MLEADERSTYLE` 导入**现已喂表**。**无 `Options` 子选项**（引线类型/内容类型/最多点数/角度约束不存在，UI 不暗示）。**真机像素判据已执行（2026-10-20，有界）**：X11/XWayland + Quadro P620(Vulkan) + `scale=1` ⇒ 箭头/斜引线/狗腿/文字四要素齐全，逐边 ≤1 px，零碰撞；同轮选择行钉住 `AcDbMultiLeader`。**未在 AutoCAD 打开验证**；无真实 MULTILEADER 夹具 | 实现（引线/箭头/文字任一命中即选中整个实体） | 不支持 |
| **bulge 弧显示（P6b 跨实体修复）** | 实现 | — | **渲染变化（正确性修复）**：`append_bulge_arc` 曾把弧心放在**弦顶侧** ⇒ 每条 bulge 弧被沿弦**镜像**（R=10、`bulge=tan(π/8)` 实测画到 **R=21.95** 再直线跳回）；**半圆不变**（`cos(θ/2)=0`）⇒ 既有 bulge 测试**全过**（只断言端点/连续/半圆，**对符号不判别**）。同型错误也在 **EXPLODE 弧恢复 / 拾取包围盒 / 对象捕捉**，四处同修。**影响所有 bulge 折线，含导入图纸**（法兰 6 处位移达弧半径 1.3%–397%、`entities` 3 处其一 251.6° 弧位移 156.27 单位；既有夹具测试只断言"输出非空"，故未抓到）。另修"平面法向符号取自 Newell 绕向"⇒ **XY 平行折线一律 `+Z`**（否则顺时针 2D 环的弧被镜像，且折线填充边界可被存成 `-Z` 平面）。**残余**：倾斜折线仍取绕向符号（域内不存挤出法向）；导入 `ocs_to_wcs` 快速路径对 `(0,0,-1)` 未改 | — | — |
| **OCS `-Z` 挤出（导入，2026-10-20 修复）** | 实现 | 实现 | **正确性修复**：`-Z` 挤出的 OCS 是 **x 镜像**，此前 `polyline_ocs_points` 的快速路径把任何 Z 平行法向都当 `+Z` ⇒ 折线顶点、**bulge 方向**、TEXT/ATTDEF/ATTRIB 的**位置与旋转**、ELLIPSE 的**法向**都未镜像。现 `+Z` 走恒等、`-Z` 走镜像：点 `(-x, y, -elevation)`、**bulge 取反**、文字旋转 `π − θ`、椭圆**保留 `-Z`**（长短轴与扫掠方向随之正确）。**CIRCLE/ARC/INSERT 经审计本就正确**（`arbitrary_axis` 的 −Z 帧），**未双重纠正**；MTEXT 按 DXF 本就是 WCS。**残余**：**倾斜**挤出的 TEXT/ATTRIB 仍按原始 OCS 放置（域内 `Text` 无挤出法向，需域变更）；且文字**字形不镜像**（平面文字域无法表达真反射） | — | — |
| DIMENSION（读取/显示） | 实现 | 合成 | 无匿名块：按定义点+DIMSTYLE 合成线/箭头/测量文字；有匿名块（`*D…`）：展开块成员。**P5a 起无块与图片标注都导入为一级 `Dimension` 实体**（显示几何保留）。**线性/对齐/半径/直径/角度(2Ln/3Pt)/坐标的合成为 `Complete`**（与代码一致；未查证项见 ADR-0017 决策 E）；**圆弧长/大半径仍 `Partial`**（不在 ADR-0017 kind 集合内，保留旧 `Compound` 合成） | 经合成几何(AABB) | 不支持 |
| DIMENSION（创建） | `DIMLINEAR`/`DIMALIGNED` **实现（P5c）**；`DIMRADIUS`/`DIMDIAMETER` **实现（P5d）**；`DIMANGULAR`/`DIMORDINATE` **实现（P5e）**；`DIMBASELINE`/`DIMCONTINUE` **NOT IMPLEMENTED（P5f 已降级）** | — | **P5c–P5e 实现创建命令**：`DIMLINEAR`/`DIMALIGNED` 为**三点流程**（两尺寸界线原点 + 尺寸线位置）；`DIMRADIUS`/`DIMDIAMETER` 为**实体拾取圆弧/圆 + 尺寸线位置**；`DIMANGULAR` 为**两线拾取 + 标注弧位置**（**三点形式仅命令层、无交互捕获**）；`DIMORDINATE` 为**特征点 + 引线端点**；`DIMCENTER` **不是** DIMENSION 实体（见绘图辅助行）。`DIMBASELINE`/`DIMCONTINUE` 因**会话级"最后标注"状态**的生命周期风险**降级为 NOT IMPLEMENTED**（P5f）。**不得因为有了实体类型就暗示其余能创建**；**创建无 spec 授权**（§3.2 只要求"常用 DIMENSION 可见表示"） | — | 不支持 |
| MESH / PolyfaceMesh | 部分 | 网格契约 | 网格 | 实现 | 不支持 |
| RASTERIMAGE | 实现(相对路径解析 + PNG/JPEG 解码) | 实现(`SemanticGeometry::Image`) | 实现(GPU 纹理：每键去重/UV 方向/裁剪多边形；缺纹理 frame+`Partial`) | 经边框 | 不支持 |
| WIPEOUT | 实现(`Mask` 边界 + `inverted`) | 部分 | **非 `inverted`：真遮罩**——无颜色 `Mask` 图元，渲染器**末 pass**（`depth_compare: Always`、不写深度、**clear 颜色**），**凹形用 even-odd 填充**；**`inverted` 亦已渲染**（视口覆盖环 + 边界做 even-odd 补集；正交视口环**精确**、透视下为**宽松估计**）；矢量出图**跳过并具名**；不与更高 `draw_order` 交错（遮罩恒最后） | 经边界 | 不支持 |
| EXTENDED(RTEXT/ARCALIGNEDTEXT/GEOPOSITIONMARKER/SECTIONOBJECT/POINTCLOUD) | 实现(基础字段) | 部分 | 基础表示(Text/圆/折线/范围盒) | 部分 | 不支持 |
| CAMERA / 动态块参数与夹点 / OLEFRAME / LAYOUTPRINTCONFIG / FORMAT / LEGACY / REGISTEREDCLASS | 实现(仅 common 或无语义) | 显式无显示 | 不绘制(`Opaque`/`Unverified`) | — | — |
| 3DSOLID / BODY / REGION / SURFACE (ACIS) | 实现(acadrust 解析 SAT/SAB；**导入期离散**为中性网格) | 中性网格（**源 ACIS 不保留**） | **会绘制**：导入期经 `cad-kernel-adapter` 离散，容差 = 包围盒对角线的 **0.1 %**，恒 `Partial` 并声明「离散近似不随缩放细化 + 源不保留」；超出子集/原始 SAT/SAB 被拒 ⇒ 仍 `Opaque` + 内核具名码 | 经网格 | 不支持 |
| TEXT/MTEXT 格式 | 实现(格式 run 解析) | 部分(堆叠分数/颜色/装饰/行对齐为显式 Partial) | 按 run 整形/换行 | — | — |
| 实体颜色/线宽 | 实现(ByObject/ByLayer/ByBlock，ACI/RGB) | 实现(颜色)；线宽仅携带 | 颜色进入 shader；线宽**显式不绘制** | — | — |
| LINETYPE 虚线 | 实现(名称/表/线型比例) | 实现(ByObject/ByLayer/ByBlock) | 按弧长细分 dash/gap；复杂线型仅 dash、显式 Partial | — | — |
| 渐变 HATCH | 实现(gradient_color) | 部分(LINEAR/SPHERICAL/CYLINDER **按原值烘焙**；其余显式 Partial) | 逐顶点颜色烘焙，裁剪于边界。**角度组 460 = 弧度（已核实）**：DXF 参考、acadrust 字段文档（`HatchGradientPattern.angle`，"Gradient angle in radians"）与本仓方向回读测试（`gradient_angle_is_stored_in_radians_not_degrees`：存 `FRAC_PI_6` ⇒ 烘焙斜坡 30°）一致；`angle_raw` 按原值往返、**不做度/弧度换算**，旋转时随几何同向旋转（`angle_raw += delta`，`transform_rotates_the_gradient_ramp_with_the_geometry` 钉住）。**残余**：仍**无** vendor 产出的非零角度渐变文件往返证据；`color_name`/`book_name` **不可写**（声明损失）；ADR-0018 U-13 已解决 | 经边界 | 不支持 |
| 出图(打印) | 实现(PLOTSETTINGS/内嵌 Layout；plot-style 引用读取并显式 Unsupported) | 实现(纸张/边距/比例/旋转) | 光栅 PNG(host readback)；无矢量/CTB | — | — |
| 异步导入 | 实现(进度阶段/取消) | 实现(唯一任务、过期丢弃) | — | — | — |
| 天正/探索者代理 | 实现(仅公开缓存记录) | 仅 FillOff/UnicodeText 有证据 | 依解码结果 | 实现 | 标记缓存几何 |
| 布局 / 视口 | 实现(矩形裁剪/比例) | 复杂裁剪标记部分 | 未装配纸空间渲染 | 部分 | 纸空间测量显式禁用 |

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

> 该条**已闭环**（2026-10-26）：`units.unknown` 现由命令窗口真实消费；`docs/handoff.md`「接线验收」里
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

### ACIS 实体显示状态（3DSOLID / BODY / REGION / SURFACE，2026-10-21：**已接线**）

这些实体**现在会画**：导入期用 `cad-kernel-adapter` 的离散器把 SAT/SAB 转成**中性网格**，
以既有的 `SemanticGeometry::Mesh` 发出（**不新增域变体**，因此没有匹配臂风暴）。

- **容差是规则，不是魔数**：线性偏差 = 该实体**世界包围盒对角线的 0.1 %（1e-3）**；角度偏差取内核默认 0.35 rad。
  `fixtures/acis/cube.sat`：对角线 3.4641016 ⇒ 容差 **3.464102e-3**。
- **完整性恒 `Partial`，两条具名理由**：①按容差 T 的**离散近似，缩放不会细化**；
  ②**源 ACIS 不保留**——实体携带网格而非原始 SAT/SAB 字节 ⇒ **不能作为 3DSOLID 往返**。
- **拒绝保持原状**：超出子集、或原始 SAT/SAB 被内核拒绝 ⇒ 仍是 `Opaque` + 内核具名码
  （如 `kernel.no_acis_kernel`），**不把失败改成空成功**。
- **导出**：网格的源类型是 `AcDb3dSolid`/`AcDbRegion`/`AcDbBody`/`AcDbSurface` 时，写 **POLYFACE 网格**
  并记录具名转换（"tessellated from ACIS at import … not as a 3DSOLID"，`export.converted.mesh`）。
- **证据**：cube 夹具 **12 三角形**、顶点界 `[-1,1]³`、闭合体积 **8.0**（解析 8.0，误差 < 1e-9）。
- **仍未做**：样条曲面 / 一般裁剪 B-rep / 完整 ACIS 内核（超出范围）；离散**不随缩放细化**；**源字节不保留**。

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
| 其余创建命令 | **NOT IMPLEMENTED**（仅剩 `DIMBASELINE`/`DIMCONTINUE`） | 二者因**会话级"最后标注"状态**（撤销/删除/切换文档时失效）的生命周期风险**降级为 NOT IMPLEMENTED**（ADR-0017 决策 C / P5f）；`DIMCENTER` 见下方**绘图辅助**行（**不是** DIMENSION 实体） |
| `DIMANGULAR` 两线点语义（P5e） | **实现（U-8 已用真实夹具钉住）** | line 1 = 组 `13→14`、line 2 = `15→10`，**顶点 = 两条无限直线的交点**，**组 `16` 选择被测扇区**。依据：`fixtures/dxf/qcad-examples/*` 中 **9 条 type flag=34** 的记录全部数值复现；判别用例 `*D65` 的组 42 = `1.975688113` rad = 113.19859° 只匹配交点读法，且匿名块 ARC 圆心在交点。**U-8 关闭** |
| `DIMANGULAR` 四选一规则（P5e） | **我们的规则** | 被测角 = **包含标注弧位置拾取的扇区**；拾取**恰在边界射线**时取**自该射线起的逆时针扇区**；三点形式同规则（可选优角/劣角）。**U-1 仍标未查证**（Autodesk 该句在仅 JS 渲染的概念页） |
| `DIMANGULAR` 交互形态（P5e） | 两线可交互；**三点无交互捕获** | 工具为**两线**形式（2 次实体拾取 + 弧位置）；三点形式**仅命令/载荷层**支持并有测试——固定步数状态机表达不了"选线**或**指定顶点"首步。**声明** |
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


## acadrust 依赖能力（0.6.5）

`acadrust` 已由 0.5.5 → 0.6.3 → **0.6.5**（features `serde` + `import`，registry `rsproxy`；无
`[patch]`、未修改上游）。`cad-import-acadrust` 是唯一消费方；`cad-proxy` 不依赖 acadrust，只消费
原始字节。下表为源码核对后的能力增量，「本项目处理」一列说明 yacr 当前的读取/建模，不改变任何
Unsupported 语义。**0.6.3 → 0.6.5 的差异**单列在表后的「0.6.5 增量」小节；只有被我们**实际执行**过的
能力才写"已接入/已验证"，其余明确标注"上游已提供、本项目未消费"。

| 能力 | acadrust 0.6.5 | 本项目处理 |
|---|---|---|
| STYLE xdata 字体面 | **支持（已类型化）**：`TextStyle.true_type_font` / `true_type_font_flags`；DXF 读 `1001 ACAD`/`1000`，DWG 经 `io/dwg/typeface_eed.rs` 按 APPID 名解析 | 已接入字体解析，链为 `类型化 true_type_font > group 3/4 声明字体 > 字节扫描`（`read_styles` 对 DXF/DWG 统一，DWG 经此通道自动生效）；字节扫描自身仍不覆盖 group 3/4 |
| LAYOUT `group 72`/`group 73` | **支持**：`read_layout`/`read_plot_settings` 映射到 `Layout.plot_paper_units`/`plot_rotation`（0.5.5 亦然，旧文档「未暴露」有误） | `record_from_layout` 直接读取这两个字段；标准纸名带单位记号时仍优先用纸名（不伪造），仅当文件未声明旋转时才按视口范围推断 |
| plot-style 解析值 | **不支持**：`PlotSettings` 只有 `current_style_sheet` 与标志，无颜色/线宽/alpha；引用名已类型化 | 显式 Unsupported：`cad-import-acadrust::plot` 读取 `current_style_sheet`/`Layout.plot_style_sheet` 与非缺省 shade plot，各产出 `import.plot_style_unsupported`/`import.shade_plot_unsupported` 诊断；颜色/线宽/alpha 仍不应用（`docs/render-order.md`、`docs/plot.md`） |
| 代理记录 opcode | **不支持**：`proxy_graphics.rs` 与 0.5.5 字节一致，仅 FillOff(21, 空 payload)/UnicodeText(36)/Unknown | 显式 Unsupported：仅证据支持的记录被解码，其余保留原字节（`docs/proxy-support.md`） |
| DWG xdata（实体/对象） | **部分**：实体 EED 仅在 APPID 句柄经 `document.app_ids` 解析时解码，否则保留 `raw_dwg_eed`；新增 `document.object_xdata`（对象 EED 解码，但为 crate-private，未向消费者暴露） | 显式 Partial：未解码 EED 保留原字节，不猜测 |
| DXF 已知实体上的未知码 | **不支持（DXF）**：未知码不保留 | 显式 Unsupported；DWG 侧新增 `EntityCommon.raw_record`（仅 DWG、可变访问即失效）与 `layer_handle`，本项目暂未使用 |
| UnknownEntity 原始 DWG 数据 | 不变 | 现有 raw 保留路径不变 |

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
| **R12 原生写出 / 动态块默认值 / ACIS 颜色** | `DxfWriter::with_version(AC1009)` 真 R12；动态块默认值与实例属性；ACIS 面/边颜色 | **未消费**：导出固定默认版本（AC1024）；动态块仍按既有代理/部分支持记录；ACIS 颜色未接入 |

## Linux 宿主（2026-10-03 新增，主验收入口）

`apps/app-linux` 共用桌面与官方 Slint/wgpu 离屏宿主，Linux release App 成为第一验收标准。
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
> （「创建无 spec 授权」）同一口径。这些命令**只改内存文档**：`SAVE` 仍是有损 **DXF 另存**，
> **没有任何 DWG 回写**。

| 命令 | 能力 | 诚实边界 |
|---|---|---|
| LINE / CIRCLE / ARC | 创建（点捕获或几何直通） | 退化/共线/非有限输入显式拒绝 |
| MOVE | 平移选择（折叠重复 ref） | 不可变换几何整条拒绝，不静默丢弃。**`SPLINE` 自 2026-10-26 起可平移**：NURBS 曲线是**仿射不变**的（映射每个控制点、`degree`/`knots`/`weights` 逐字复制 = 曲线的精确仿射像），`cad-db` 的 `transform_geometry` 不再按种类拒绝样条 |
| COPY | 复制（新 id） | 同上（含 `SPLINE` 可变换）；按对象样式覆盖无法复制时记 `drawing.copy.style`（副本继承图层样式） |
| ERASE | 删除选择 | 空选择拒绝；撤销恢复渲染属性与动态可见性成员 |
| TRIM | 目标 **LINE / ARC / CIRCLE / SPLINE**（SPLINE 目标 2026-10-27 起，见行末）；边界 **LINE / ARC / CIRCLE / ELLIPSE / LWPOLYLINE**（直线段或带 bulge 弧段，可任意混用）**/ SPLINE**。LINE/ARC/CIRCLE/ELLIPSE 及 bulge 弧段用解析圆锥曲线求交（与对象捕捉同一实现）；**SPLINE 边界只走采样**：`intersect_local` 把两个操作数按 `extent × 1e-4` 弦高（每操作数最多 256 段）扁平化后求交，切点误差界为该弦高（10 单位图纸约 `1e-3`），因此**每条真正贡献切点的 SPLINE 对都会额外给一条 `drawing.trim` 诊断**说明切点来自采样近似，绝不冒充解析解。LINE 保留拾取侧子段，ARC 保留与原弧同圆心/法向/半径、**同 sweep 符号**的剩余弧，CIRCLE 变成一段 ARC（拾取点两侧相邻交点之间），一条完全落在闭合边界内的目标整体删除（含 CIRCLE；内侧判定按边界自身平面，倾斜折线不会被 XY 投影误判）。**拾取点先投影到已解析的目标上**（LINE 取垂足；ARC/CIRCLE 取 `center + radius * normalize(pick − center)` 的径向投影），所以**点在实体附近即可工作**：宿主已用拾取框解析出目标，点不必恰好落在曲线上 | **仍拒绝**：`INSERT`/`Opaque`/`Mesh` **目标**（`SPLINE` **目标**自 2026-10-27 起支持，但有两个诚实边界：**切割参数**来自采样求交 ⇒ 新端点最远偏离真实交点 `控制多边形盒对角线 × 1e-4`（≤256 段），而**保留下来的子曲线本身是精确 B 样条**（齐次点上的 Boehm 插入、两端 multiplicity = degree+1、degree/weights/内部节点逐位保留；**绝不**重拟合、**不**改参数化、**不**替换成折线）；且 `Spline` 无 closed 标志 ⇒ 闭合样条的接缝按起/终点处理，不做环绕。子曲线 `knots` 拒绝面：退化跨度、非法存储定义）（边界只有 `INSERT`/`Opaque`/`Mesh` 等未列出的类型被拒）、`|sweep| > 一周` 的 ARC 目标（`Unsupported`）；无交点/退化/**投影落到目标自身参数域之外**（不夹到最近端点——夹紧等于替用户决定保留哪一段）/ARC/CIRCLE 拾取点**径向方向为零**（点在目标轴线上，如圆心）/CIRCLE 交点少于 2 ⇒ `InvalidInput`。**闭合 SPLINE 的内侧删除规则不支持**：`SemanticGeometry::Spline` 既无闭合标志也无边界平面，无法做可靠的平面感知内侧判定，故“目标完全落在闭合 SPLINE 内且无切点”**显式 `Unsupported` 拒绝**（不删除、不臆造包含关系）；其它无解析解的边界对（ARC×ELLIPSE、ELLIPSE×ELLIPSE、非共面圆族）仍出图但同样附采样近似诊断；3D 非共面输入只会“求不到交点”而非被投影凑出结果 |
| EXTEND | 目标 LINE（沿自身无限直线）或 ARC（沿自身圆、**同号 sweep**）；**SPLINE 目标显式拒绝**（2026-10-27 起带具名理由：NURBS 由节点向量定义，移动端点会改变节点向量进而改变整条曲线的参数化，且本构建没有外推策略——不按切线近似冒充）；边界集合同 TRIM（**含 SPLINE**，同样只走采样并附 `drawing.extend` 近似诊断，弦高界同 TRIM）。拾取点同样**先投影到目标上**，投影落点决定延伸哪一端（“点在哪一头”按曲线上的落点判定，与点到曲线的距离无关）；只取拾取端**前方最近**交点，**只增不减**；ARC 的延伸量**上限为半圈**（超过半圈视为“只能绕远路到达”，显式拒绝） | CIRCLE 目标（无端点）、`|sweep| > 一周` 的 ARC、`SPLINE` 等其余**目标**、未支持的边界类型 ⇒ `Unsupported`；已接触/已穿过/前方无交点/延伸超过半圈/投影落在目标参数域之外 ⇒ `InvalidInput`；采样近似同 TRIM 另给 `drawing.extend` 诊断 |
| ROTATE / SCALE | 绕基点旋转/均匀缩放 | 相似变换（正行列式）；不支持几何 `Unsupported`。**含 `MultiLeader` 的选择：旋转可以，缩放 ⇒ 整命令失败**（ADR-0019 C.5：掩码未置位时 AutoCAD 会从样式重解析） |
| MIRROR | 镜像（**保留原件**，新增副本）：含 `Arc` 与 `Ellipse`（**含椭圆弧**，2026-10-26 起） | 反射的 `normal' = −R(normal)`，弧的 `start` 由镜像后的起点在新帧中**重新求角**、`sweep` **符号保留**（负法向已把帧的重手性翻正；若同时取反 sweep，端点映射不成立——两种写法都验证过）。椭圆：圆心/长轴向量反射、`ratio` 与参数保留。仍拒绝 `Text`/`Shape`/`Opaque`/`MultiLeader`：`Text` 只镜像拒绝（平面文字无法表达真反射），`MultiLeader` 拒绝一切缩放 |
| OFFSET | LINE / CIRCLE / **直线段 LWPOLYLINE**（2026-10-26 起）偏移 | 折线要求**每段 |bulge| ≤ 1e-12**；整条折线**同一侧**、常数距离（取拾取点到最近段直线的**平面内**垂距，面外分量只用于定侧）、**miter 斜接**、顶点数/顺序/`closed`/`bulges` 原样保留（开口端不造封口）。按名拒绝：`offset.bulge_segment`（含弧段折线）、`offset.collapsed`（顶点处近 180° 反折或偏移段塌缩/归零）、`offset.non_planar`（顶点偏离自身平面）；退化段/点数 < 2/非有限顶点/闭合零面积/拾取点落在折线上 ⇒ `InvalidInput`。**仅顶点局部塌缩检测**：远端偏移段之间的自交**未检查**（声明，不静默宣称已排除）。其余几何（ELLIPSE/SPLINE/INSERT/ARC）仍 `Unsupported` |
| COPYCLIP/CUTCLIP/PASTECLIP | **应用内**剪贴板 | 非 OS 剪贴板；替换文档清空；空选择/空剪贴板拒绝 |
| MATCHPROP（别名 `MA`） | **选择式**特性匹配（AutoCAD 特性刷的文档化变体）：当前选择按**选择顺序**（用户的拾取顺序，**从不**按 id/handle 重排）取**第一个**实体为**源**，其余每个选中实体为**目标**。转移的属性**恰好**是属性写路径支持的五项显示属性：`layer`、`color`、`linetype`、`lineweight`、`transparency`（语法与 `SETENTITYPROPERTY` 同一份，不复制第二套解析）。源/目标**类型可以不同**（只要每个值对目标合法）。目标已与源一致 ⇒ 该目标不产生变更、不进撤销补丁；全部目标都一致 ⇒ 显式 `matchprop.unchanged` 完成，**不**开事务、**不**产生空撤销步；否则一条事务、一步撤销 | **开始改写前**全部拒绝（零 DB 变更、零撤销步）：选择少于两个实体（`matchprop.needs_two`）；源**或**目标引用是块实例成员（instance path）/子图元/属于其它文档/不存在（`property.not_addressable`/`property.target_missing`，与属性写路径同一规则）；源值的存储形式无法用写语法表达、或语法拒绝该值（如 `ByBlock` 透明度、无名显式线型）⇒ `matchprop.source_value`，**点名**源实体与属性并保留解析器自己的 `property.*` 码。**明确不转移**（绝不静默近似）：MATCHPROP 的 **Settings 子集**（选择“复制哪些属性”）未实现（总是转移全部五项）；非显示属性——文字样式/字高、标注样式、填充图案等一律**不**转移；实体自身的**线型比例**不在五项之内，给名线型目标只在目标上保留**目标自己的**比例（与属性写路径一致），符号值（`ByLayer`/`ByBlock`）整值替换 |
| JOIN | 合并当前选择为**一个**实体：共线且首尾相接/重叠的 `LINE` → 一条 `LINE`；同圆心/法向/半径且扫掠相续的 `ARC` → 一条 `ARC` | 一条事务、**一步撤销**，全有或全无（先全部校验，再删除 N + 插入 1）。**仅 LINE 与 ARC**：折线/样条/圆、需要桥接空隙、跨图层/跨空间、非同心弧、块内成员 ⇒ 显式拒绝（`join.needs_two`/`join.mixed_kinds`/`join.not_collinear`/`join.not_contiguous`/`join.layer_mismatch`/`join.space_mismatch`/`join.sweep_overflow`/`join.degenerate`）。合并 `LINE` 的端点按 `(x,y,z)` 升序归一（同一批拾取总得同一几何）；`draw_order` 取源最小值（原位替换，不跳到最上层）；扫掠和恰为一整圈时**保留 `2π` ARC**，不静默转成 `CIRCLE`；反向法向的弧按「不相续」拒绝而非重新推导 |

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
- **导出**：`SAVE` 为**有损 DXF 另存**（`Partial`，逐条记录转换/丢弃，**非** DWG 往返保存）；
  `PLOT` 为矢量 SVG/PDF。真实图纸上的导出兼容性**未验证**。逐类：LWPOLYLINE（平面的）/ELLIPSE/POINT
  为精确映射；**SPLINE 记为 `Converted`**（`DxfSpline`，几何保留，**closed/periodic 标志不携带**）；
  Shape/Image/Mask/Opaque 被丢弃并逐条记录。

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
| `POINT` | **实现** | 单个 `Point`（`+Z` 平面）；**单次放置**（AutoCAD 是否重复放置**未查证** ⇒ 不按猜测实现）；显示走 overlay 标记，并读取图纸的 `$PDMODE`/`$PDSIZE`（见上方 POINT 绘制列）；**样式可改**：`PDMODE <n>` / `PDSIZE <n>` 写图纸头值（一次 revision 提升，显示立即刷新；**不进历史**、**不可撤销**；导出写入 `$PDMODE`/`$PDSIZE`，`SAVE→OPEN` 往返已用本仓库读写器验证——不是 vendor 验证） | `32`/`64`/`96` 周边圆/方形状、`PDMODE 0` 的真实点（Lines-only 拓扑下是 `+` 近似）、**`SETVAR` 间接寻址**（只支持这两个名字，无任意头变量设置、无 `SETVAR ?` 查询）、Multiple Points 重复放置、**3D 视图中的点标记** |
| `SPLINE` | **实现** | fit 点法（**≥4** fit 点）→ 三次/clamped/chord **插值** NURBS（`degree=3`、`weights=1`）；预览为 ≤256 段有界弦近似 | `Method(CV)`/`Knots`/`Degree`/`Start-End Tangency`/`Tolerance`/`Close`/`Object` 转换；**2–3 个 fit 点**；**fit 点数据不参与 SAVE→OPEN 往返**（domain 无该字段） |

- 创建命令均为 **Work 模式**（Viewer 显式拒绝）、**模型空间**（插入漏斗固定 `SpaceId::Model`）、
  **一次事务 + 一步撤销**；未支持选项一律显式拒绝，不用近似静默顶替。

### 编辑命令（ADR-0016，逐条按实施状态）

| 命令 | 状态 | 结果 | 显式未实现 / 有意偏离 |
|---|---|---|---|
| `FILLET` | **实现** | 两条**独立直线**：`R>0` ⇒ 两段修剪到切点 + **新增相切 ARC**；`R=0` ⇒ 只延伸/修剪到角点、不产生弧。新实体在**活动图层**（我们的选择，AutoCAD 规则未查证） | `Polyline` 模式、`Multiple`、命令内 `Undo`、`Trim/No Trim` 开关、**平行线自动调半径**（改为显式拒绝）、Circle/Arc 目标、3D、hatch 关联性；**圆心公式为几何推论（UNVERIFIED）** |
| `CHAMFER` | **实现** | 两条独立直线：两距离**按拾取顺序**施加；`0/0` ⇒ 只做角点（与 `FILLET R=0` 有等价性对照测试）；否则两段修剪 + **新增 LINE**；新实体在**活动图层**（我们的选择，AutoCAD 规则未查证） | `Angle` 方法/`CHAMMODE`、`Polyline` 模式、`Multiple`、`Undo`、`Trim/No Trim`、弧段替换、3D；**负距离行为未查证**（一律拒绝） |
| `ARRAYRECT` / `ARRAYPOLAR` | **实现** | **独立副本**（AutoCAD 默认关联，记为**有意偏离**）；矩形：间距=**基点间距离**、向上右生成、副本数 = `行×列−1`；极坐标：捕获中心点，副本数 = `项数−1`，**角度步长 = `i·填充角/项数`**（**我们的规则**：部分填充时 AutoCAD 是首尾跨满 `fill/(items−1)`，故 `180°/3 项` 我们为 0/60/120 而 AutoCAD 为 0/90/180——**刻意偏离**，已测试），`rotateItems` 决定绕中心旋转或仅平移（**未查证选择**） | `Path`/`Levels`/`Total`/角度间隔输入/关联编辑（`ARRAYEDIT`/grips）/`MAXARRAY` 可配置化/3D/**负间距反向**；1×1 或单项阵列**不产生副本并显式诊断**（不开事务） |
| `ALIGN` | **实现** | **两对点**：平移（第一对）+ 旋转（两对方向夹角）+ `AlignScale` 为真时按"目标对长/源对长"做**均匀**缩放；以 `source1` 为枢轴，原位变换整个选择。**含 `MultiLeader` 且 `AlignScale` 为真 ⇒ 整命令失败**（同 C.5） | 3 对点 3D 对齐、**非均匀**缩放、**1 对点移动**（等价于 `MOVE`，不做） |
| `STRETCH` | **实现** | 交叉窗口（**恒 CROSSING**）：**部分包围=逐顶点移动**（只动窗内顶点）、**完全包围=整移**；分类按几何类型分两种口径（直线/无 bulge 折线=逐顶点包含；圆/椭圆/块/ARC/含 bulge 折线=保守包围盒完全包含）；**bulge 数值保留**（未查证）。**逐顶点来源（按类，2026-10-26）**：`LINE` = 两端点；`POLYLINE`/`LWPOLYLINE` = 每个存储顶点（bulge 原样保留，未查证）；`SPLINE` = 每个存储**控制点**（`degree`/`knots`/`weights` **逐字复制**）；`ARC` = 存储**起点/终点**两点（规则见下）。**NURBS 声明**：控制点**不是曲线点**（只有 clamped 样条的首/末控制点落在曲线上），单个控制点的支撑域是 `degree+1` 个节点区间，故**部分**拉伸会**整体**重塑曲线——这是 NURBS 控制点编辑的固有性质，**不是缺陷**，也不做任何重拟合/求值；复制向量后定义仍满足 `cad-db` 不变量（契约测试用 `cad_db::validate_geometry` 断言）。**ARC 部分拉伸（本项目的选择，非 AutoCAD 断言）**：窗内端点各按其「平移后位置相对圆心的**平面内方向**」改角，`center`/`radius`/`normal` **逐位不变**（弧不离开自身平面，位移的平面外分量被丢弃），窗外端点保持原角，`sweep` 按**存储方向**重算（`sweep<0` 仍顺时针）；以下三种结果**只报诊断、该实体不写入、绝不夹取**：重算张角 ~0 或 ~2π（阈值 `1e-9`，与 tessellator / 弧端点夹点同一 slack）→ `drawing.stretch.arc_degenerate_sweep`；被拖端点无平面内方向（落在圆心或法线上）→ `drawing.stretch.arc_no_direction`；规则使两个存储角都不变（位移在弧平面内**无切向分量**，如纯径向/纯平面外）→ `drawing.stretch.arc_unchanged`。其余「不动」路径的具名码：窗内**没有**顶点/控制点/端点 → `drawing.stretch.no_vertex`；定义不可索引的样条（knot/weight 数量不符，导入路径可产生）→ `drawing.stretch.spline_inconsistent`（部分包围；**完全包围整移**则由 `cad-db` 的写路径校验按同一稳定码拒绝，整命令失败、不写入）。**完全包围的 `SPLINE` 整移**：与其它种类**同一条路径**——`cad-db` 的 `transform_geometry`（NURBS 仿射不变，见下「仿射不变规则」）；上一轮为绕过 `cad-db` 拒绝而加的应用内「纯平移控制点」例外**已删除**，故完全包围的 STRETCH 与 MOVE 对样条**不可能**给出不同结果。**INSERT 部分拉伸（本项目的选择，非 AutoCAD 断言）**：块参照在这条路径上只有**一个**可寻址「顶点」——**插入点**（块坐标系原点 `(0,0,0)` 过实例放置的 `Transform3::apply_point`，即放置矩阵的平移列）。**部分包围**时：插入点**落在交叉窗口内** ⇒ 整条参照**刚性平移** delta（走 `cad-db` 的 `transform_geometry` 平移路径，**与完全包围整移同一个调用**），放置矩阵的**线性部分（旋转/缩放/镜像列）逐位不变**、`block` 不变、只有平移列改变；插入点**不在窗口内**（窗口只压到块成员的显示几何/保守包围盒）⇒ **逐位不动** + `drawing.stretch.insert_outside`（**不写入、不 staging**）。**完全包围** ⇒ 整移（未变）；**窗口外** ⇒ 完全不动（无写入）。**诚实的边界（非目标，不得读成「已支持块」）**：块**成员不会被逐个拉伸**（实例路径在这条路径上不可寻址，规划只写顶层模型空间所有者，没有逐成员顶点编辑）；**旋转/缩放/镜像**的参照只**刚性平移**、**不重拟合**（不按窗口重塑，旋转与比例逐位保留）；这**不是** AutoCAD 的完整块参照 STRETCH 语义——定义级编辑、嵌套成员拉伸、属性重排均**未建模、不声明** | cpolygon/lasso、**单独选取即整移**、`TEXT`（**相似变换语义受限** ⇒ 整命令拒绝，ADR-0019 B.2——**不是**因为变换面拒绝：P7a 起 `Text` 可平移/旋转/均匀缩放）、`MultiLeader`（**显式按 `type_key` 整命令拒绝**）、`HATCH` 的**拉伸**（部分包围不动+诊断、完全包围整移）、`INSERT` 的**成员级**拉伸（只做插入点刚移，见前）、3D；宽度未建模 |
| `GRIPSTRETCH`（**本项目选择器**，AutoCAD 无同名命令） | **宿主拖拽已实现（仅 `native-below`）；`dom-canvas` 载体无夹点路径；UI 层 Escape 已接线（无运行时证据）** | 纯模型（`crates/cad-app/src/grips/`：`mod.rs` 句柄表/命中/文档 + `drag.rs` 各类拖拽规则）+ 命令（`crates/cad-app/src/app_drawing/grip.rs`）：**一个夹点、一个实体** → 一次 `update_entity`、一条事务 / 一步撤销（undo 恢复被拖端点/位置的**位级**原值，未拖端点/其它坐标逐位不变）。支持面：`LINE` 三夹点（`Endpoint` `index` 0 = start、1 = end，`Midpoint` `index` 2 —— 拖中点让**整条线平移**，两端走同一 delta，方向与长度不变；中点位置是两存储端点的 f64 精确中点）、`CIRCLE` 五夹点（`Center` `index` 0；`Quadrant` `index` 0..=3 = 圆自身平面内 `+ax` / `+ay` / `-ax` / `-ay`，拖象限**只改半径**：半径 = 中心到目标的**平面内**距离（不是 3D 距离、也不是 x/y 距离），圆心与法向逐位不变，故圆不会被拖成倾斜椭圆）、`ARC` 三夹点（`Center` `index` 0 = 刚性平移：半径/法向/起始角/张角逐位不变；`Endpoint` `index` 0 = 弧**起点**、`index` 1 = 弧**终点** —— 拖端点让弧留在**同一圆**上（圆心/半径/法向逐位不变），被拖端点的角度 = 目标在弧自身平面内的方向，**另一端保持原角度**，张角按**原方向**重算且必须落在开区间 `(0, 2π)`；会把弧压没（张角 ~0 或 ~2π，阈值 `1e-9` rad，与 tessellator/provider 判「整圆」同一 slack）或目标在平面内没有方向（圆心本身/圆心法线上）时按名拒绝 `grip.degenerate_arc`，**绝不夹取**；该端点规则是**本项目的选择，不是 AutoCAD 断言**）、`POLYLINE`/`LWPOLYLINE` 顶点夹点（`Vertex` `index` = **存储**顶点序号，一顶点一夹点；拖拽只把该顶点 x/y 设为目标、**保留其自身 z**，其余顶点与 `closed` 逐位不变，`bulges` 向量整体原样复制 —— **不重算任何 bulge**（进/出该顶点的段保留原 bulge；这是我们的解读，**未经 vendor 验证**）；越界 `index` 为 `grip.stale_handle`；`GripId::index` 是 `u8`，故只为顶点 `0..=255` 提供夹点，更多顶点在本版本不可寻址）、`ELLIPSE` 夹点（`Center` `index` 0；`Quadrant` `index` 0..=3 = 椭圆**自身存储轴**帧内的 `+major` / `+minor` / `-major` / `-minor`，与 `CIRCLE` 象限同 `+, +, -, -` 次序）：中心拖拽**刚体平移**（`normal`/`major_axis`/`ratio`/`start`/`sweep` 逐位不变）；轴象限拖拽把**该轴的半径**设为“中心到目标在椭圆自身平面内的距离”（不是 3D 距离、也不是 x/y 距离），中心与法向逐位不变；**轴方向永不旋转**（本项目选择，**不是 AutoCAD 断言**）：major 拖动把存储 `major_axis` 按 `distance / |major_axis|` **缩放**并把 `ratio` 重算为 `minor_len / distance`（保留存储的次半径 `|major_axis| * |ratio|`），minor 拖动只令 `ratio = distance / |major_axis|` 且 `major_axis` 逐位不变；因此目标不在轴上时也只改半径、不转动椭圆，且两轴按构造保持正交。**椭圆弧（部分椭圆）**：存储参数范围不是整圈（`|sweep| < 2π - 1e-9`，与 tessellator/provider 判「整圈」同一 slack）时**只暴露 `Center` 夹点**，象限 id 按名拒绝 `grip.partial_ellipse`（本项目选择，**不是 AutoCAD 断言**：轴点未必落在已绘参数范围内，用它改形等于悄悄重释参数范围；中心拖拽不改半径/参数，故仍可用）。域模型**没有** `minor <= major` 约束（`cad-db` 只要求 `ratio` 正且有限，渲染用它画 `|major| * |ratio|`），故拖到两半径交叉**不拒绝也不静默归一化**（交换轴会旋转存储帧并改变参数化）；只有创建路径（`ellipse_parameters`）恰好保持 `ratio <= 1`，那是轴端点法的存储假设，**不是**夹点不变量。零/非有限平面内半径、或存储帧无法导出平面内半径（`major_axis` 为零/非有限，或法向与其平行）→ `grip.degenerate_radius` 按名拒绝、**不夹取**；把该轴半径拖回原长度（含离开平面但平面内距离不变）→ `grip.zero_displacement`）、`POINT` 位置夹点（`index` 0），以及**块引用插入点夹点（2026-10-26）**——见下方 `INSERT` 条。**`TEXT`/`MTEXT` 插入点夹点（2026-10-26）**：`Insertion` `index` 0 = **存储的 `position` 原值**（三坐标逐位，**不**按 `h_align`/`v_align` 偏移、**不**绕该点按 `rotation` 旋转；对齐或旋转**不改**句柄位置）；拖拽只把 `position` 设为目标，`text`/`style`/`height`/`rotation`/`font`/`h_align`/`v_align`/`wrap_width` **逐位不变**（含 `wrap_width: Some(w)`，属普通 `f64`，只复制不重算）。`TEXT` 与 `MTEXT` 是**同一个** `SemanticGeometry::Text` 变体（只有 `DbEntity.object.type_key` 区分 `AcDbText`/`AcDbMText`），故两者夹点完全相同。**新变体 `Insertion` 而非复用 `Point`**：`Point` 是 `POINT` 实体自身的句柄，共用会让 `grip.stale_handle` 无法区分"文本插入点"与"点实体"。**旋转 / 高度 / 对齐（attachment）/ 宽度夹点一律 NOT IMPLEMENTED**——本轮**只**实现插入点。`SemanticGeometry::Polyline` 是**所有** 折线类（LWPOLYLINE / `AcDb2dPolyline` / 旧 `AcDbPolyline` / 3D `AcDb3dPolyline`）的唯一语义变体，类名不进域模型，因此**无法单独排除 3D 折线**（顶点夹点对所有折线生效；拖拽只改 x/y 并保留自身 z，与 `LINE` 端点规则一致，不做 3D 标高编辑）；**按类区分 3D 折线 NOT IMPLEMENTED**。**`SPLINE` 控制点夹点（2026-10-26）**：`ControlPoint` `index` = **存储**控制点序号，位置 = 存储 `Point3` 原值（逐位，**不做**任何 NURBS 求值——控制点一般**不在**曲线上，只有 clamped 样条的首/末控制点才在；这是定义而非 bug，标记落在曲线外是正确行为）；拖拽把**该**控制点设为目标三坐标原值，`degree`/`knots`/`weights` 与其他控制点**逐位不变**（不重算节点/权重、不做重拟合 ⇒ 结果是参数化相同的**另一个 NURBS**，曲线因此不再经过它原先经过的点；这是本项目选择，AutoCAD 同样直接拖控制点但**不声称** vendor 等价）；`GripId::index` 是 `u8`，故只暴露控制点 `0..=255`，更多控制点在本版本**不可寻址**（**不**回绕、不截断、不重编号）；越界 `index` → `grip.stale_handle`、目标等于存储控制点 → `grip.zero_displacement`、非有限目标 → `grip.non_finite_target`、该几何上的其它 kind → `grip.stale_handle`；存储定义**自相矛盾**（`control_points.len() != weights.len()`，或 `knots.len() != control_points.len() + degree + 1`）时**一个夹点都不暴露**（不猜、不补齐向量；形状有效性以 `cad-db` 校验器为准），其任何控制点 id 均为 `grip.stale_handle`。**`INSERT`（块引用）插入点夹点（2026-10-26）**：`Insertion` `index` 0 = **放置变换的插入点**，即 block-space 原点在 `Transform3` 下的像 —— `Transform3` 是 4x4 仿射且 `apply_point` **不做**齐次除法，故该像**就是**矩阵的平移列；导入器已把块基点折进放置（`insert_array_transform` 的 `translate * rotate * scale * block_placement(-base)`，使 block-space 原点落在 `INSERT` 的存入插入点上），所以这是仓库**既有**的“块引用插入点”定义，本轮**不**引入第二套约定。拖拽是**刚性移动**：只把平移列写成目标（`m[0][3]=to.x`、`m[1][3]=to.y`、`m[2][3]=to.z`），**整个 3x3 线性部分（旋转 / 非均匀缩放 / 镜像 / 剪切）与 `block` 逐位不变**（第 3 行也不变），因此拖拽**永不**旋转或缩放块引用，且插入点**恰好**落在目标上（不做求逆、**无**残差、**不**夹取/捕捉）。有限但**奇异**的放置**不**拒绝（平移仍逐位精确，线性部分原样保留；`cad-db` 本来拒绝**存储**奇异放置，故这属损坏输入）；存储放置含**非有限**系数时按名拒绝 `grip.degenerate_transform`。**块成员（instance path）/ 块内属性（ATTRIB）/ 块内点**的夹点一律 **NOT IMPLEMENTED**：`GripId` 不含 instance path 与子元素字段，`GRIPSTRETCH` 已用 `property.not_addressable` 拒绝非模型空间目标，本模型**不**另立可达性规则。`Insertion` 变体由 TEXT/MTEXT 与 INSERT 共用（靠几何变体区分，**不**新增 `GripKind`）。**`DIMENSION` 定义点夹点（2026-10-26）**：新变体 `DefinitionPoint`，`index` = **该子类型文档化的句柄次序里的位置**，一个句柄对应一个存储的 DXF 定义点，**最后一个**句柄是存储的 文字中点（group 11）：`Linear { rotation }` 与 `Aligned`（共用 `DimensionPoints::Linear`）= `0` `first`(13)、`1` `second`(14)、`2` `dimension_line`(10)、`3` `text_middle`(11)；`Radius`/`Diameter`（`Radial`）= `0` `center`(10)、`1` `point_on_arc`(15)、`2` `text_middle`(11)；`Angular2Ln` = `0` `line1_start`(13)、`1` `line1_end`(14)、`2` `line2_start`(15)、`3` `line2_end`(10)、`4` `arc_point`(16)、`5` `text_middle`(11)；`Angular3Pt` = `0` `vertex`(15)、`1` `first`(13)、`2` `second`(14)、`3` `arc_point`(10)、`4` `text_middle`(11)；`Ordinate` = `0` `feature`(13)、`1` `leader_end`(14)、`2` `base`(10)、`3` `text_middle`(11)。句柄位置 = **存储点逐位原值**（三坐标，不投影、不派生、不偏移）；`leader_length`(40)、`text_rotation`(53)、`Linear { rotation }`/`Ordinate { x_ordinate }` 等子类型标量、`normal` 与整套解析后的 `DimensionStyleValues` **都不是夹点**，任何拖拽都逐位保留。**拖拽规则 = 重合成**：克隆存储定义、只替换那**一个**点，再用 `DimensionGeometry::synthesized` 以**实体自身**的 `style_name`/`style`/`text_override` 重建 —— 故 `kind`、`text_rotation`、`normal`、`style_name`、`style`、文字覆盖串（DXF group 1，**不丢弃**）与未被拖动的定义点/`leader_length` 全部保留，而 `measurement`（group 42）与**整幅显示**由 `cad-domain` 的唯一构建器按改后的定义**重算**（`cad-app` 不手改 dimension 子几何）；拖 `text_middle` 只移动存储的 group-11 锚点并重排显示，`measurement` 不变。**非法结果一律按名拒绝、绝不夹取**：`grip.dimension_inconsistent`（子类型与点槽不符 / 点非有限；两个公开构造器都会拒绝这种定义，故命中它属**缺陷信号**）、`grip.dimension_degenerate`（改后的定义无法合成显示 —— `Radial` 的 `point_on_arc` 拖到圆心、`Angular3Pt` 的 `arc_point` 拖到顶点、两条角尺线被拖成平行等 —— 或合成出的 `measurement` 非有限 / 显示为空：**绝不**存 `NaN` group 42、**绝不**存空显示）。**声明限制**：`cad-domain` 的 `DisplaySource` 是私有的，`cad-app` 无法区分「我们的合成」与「导入的匿名块图片」分支，也没有能从改后定义重建图片分支的构造器，故拖拽**总是**重合成（图片分支的 dimension 拖后变成合成分支，`measurement` 跟随改后的定义）。`DefinitionPoint` 是**新变体**而**不**复用 `Endpoint`/`Insertion`：dimension 的句柄就是它的 DXF 定义点加文字中点，共用会让 `grip.stale_handle` 无法区分。**其余整类 NOT IMPLEMENTED**：块**成员**/块内**属性**（ATTRIB）/块内点（instance path 在 `GripId` 上无法表达，`GRIPSTRETCH` 另有 `property.not_addressable` 兜底）、`MULTILEADER`（其注记文本在本版本**不**通过文本插入点句柄暴露，故**无夹点**）、`HATCH`、`IMAGE`/`MASK`。多夹点/多实体拉伸、拖拽中捕捉、3D 夹点编辑仍不存在。载荷 `CommandPayload::GripStretch(Box<GripStretchCommand>{ id: GripId, to: Point3 })`。命令层 `to` 三坐标原样存储（**不做**工作平面投影）；**宿主**在平面视图里用 `grip_hit_planar`（x/y 距离，**不是** 3D 距离——指针经 `screen_to_world` 落在工作平面上）命中夹点，并**保留被拖夹点自身的 z**，故平面拖拽**不改标高**（**3D 夹点编辑 NOT IMPLEMENTED**）。拒绝（**全部在开事务之前**，不改 revision、不写 ChangeSet、不留空撤销步）：`grip.target_missing`（实体已不存在）、`property.not_addressable`（非模型空间目标：块定义成员 / 图纸空间 —— 复用属性写的可达性理由；`GripId` 不含 instance path / 子元素字段，故那两种形态在载荷上无法表达）、`grip.stale_handle`（kind/index 与**当前**几何不符）、`grip.zero_displacement`（零位移是**错误**而非空 `Ok`，正为避免空撤销步；**宿主**把它按**显式 no-op** 处理：不发命令、不给用户报错）、`grip.unsupported_kind`（`MULTILEADER`/`HATCH`/`IMAGE`/`MASK`/`Opaque`/`Compound`/`Mesh`/`Shape` 等整类无夹点模型；`LINE`/`CIRCLE`/`ARC`/`ELLIPSE`/`POLYLINE`/`SPLINE`/`POINT`/`TEXT`/`MTEXT`/`INSERT` 已不在此列）、`grip.degenerate_radius`（圆/椭圆象限拖拽的平面内半径为 0 或非有限值，或椭圆存储帧无法导出平面内半径 —— 按名拒绝，**不夹取最小值**）、`grip.degenerate_arc`（弧端点拖拽压没张角、张角非有限，或目标在平面内无方向 —— 同样按名拒绝，**不夹取**）、`grip.partial_ellipse`（部分椭圆/椭圆弧上的象限 id：该几何只暴露 `Center` 夹点，按名拒绝而非伪称 stale）、`grip.degenerate_transform`（块引用插入点拖拽时存储的 `Transform3` 含**非有限**系数：刚性规则保留该系数，`cad-db` 提交时本会拒绝，故按名拒绝而不是把失败推后；**有限但奇异**的线性部分**不**拒绝——规则不做求逆/分解）、`grip.dimension_inconsistent`（dimension 存储定义子类型与点槽不符或点非有限：该维度**一个夹点都不暴露**，任何 `DefinitionPoint` id 均为该码；公开构造器本会拒绝这种定义，故命中属**缺陷信号**）、`grip.dimension_degenerate`（dimension 定义点拖拽后无法合成显示，或合成的 `measurement` 非有限/显示为空：按名拒绝、**不夹取**，绝不写 `NaN` group 42 或空显示）、`grip.non_finite_target`（**在读取图形之前**拒绝）。裸 `GRIPSTRETCH` 为 payload-required，按名拒绝（不路由 `BeginDraw`、不伪造拖拽）；**绝不**经过 `StretchEntities` | **宿主侧状态（`crates/cad-host/src/input/grip_drag.rs`，2026-10-26）**：`native-below` 桌面宿主（Linux 原生 Wayland / Windows）在选中实体上绘制夹点标记，并把**单个**已建模夹点（`LINE` 端点/中点、`CIRCLE` 圆心/象限、`ARC` 圆心/两端点、`ELLIPSE` 圆心/四轴点（整圈；椭圆弧仅圆心）、`POLYLINE` 顶点、`SPLINE` 控制点、`POINT` 位置、`TEXT`/`MTEXT` 插入点、`INSERT` 块引用插入点、`DIMENSION` 定义点（按子类型次序）与文字中点）的拖拽接到 `GripStretch`（宿主对夹点种类**无分支**：枚举与命中都来自模型）：按压命中夹点即 arm（2D、非捕获、工具 `Idle`/`Selecting`、主键、选中且**可见**的可直接寻址实体），移动只更新 overlay 预览（`OverlayInputs::grip_markers` 中被拖项替换为当前目标 ⇒ **移动的标记就是预览**，不发导航 delta）；拖拽目标走**与取点相同的**会话辅助 `SessionState::constrained_point`：**先栅格捕捉**（`SNAP`/F9，按 `SNAPUNIT` 间距、以世界原点为基准、半程远离零、`z` 不变）、**再 ORTHO**（`ORTHO`/F8，轴过**被拖夹点的原位**，`|dx| >= |dy|` 取水平、平局取水平，另一坐标逐位保持原位值），**预览与提交共用同一个已约束坐标**（绝不预览一个点、提交另一个）；两开关都关时与旧行为逐位相同；间距非有限或 `<= 0` 会被会话设置拒绝、量化器原样放行（不伪造间距，开关因此无效果）；**OSNAP 已参与夹点拖拽（2026-10-26）**：目标经**取点路径的同一个** `HostController::resolve_snap`
解析（同一候选生成器、同一 `snap_enabled` 门、同一最佳候选规则与同一光圈容差），命中则**替换**原始
指针点，再走栅格捕捉与 ORTHO ⇒ 顺序为 **OSNAP → 栅格捕捉 → ORTHO**（与取点路径一致；两者同开时栅格会把
已捕捉点再量化，故落点在栅格而非候选点，除非二者重合 —— 这是取点路径的既有行为，显式声明）；
候选只贡献**平面位置**，`z` 仍取被拖夹点自身（3D 夹点编辑未实现）；**不排除自捕捉**（取点路径也不排除），
指针停在夹点自身光圈内 ⇒ 目标是夹点原位 ⇒ 模型的零位移 no-op；拖拽**不**发布 OSNAP 提示标记
（移动的夹点 marker 仍是唯一预览）。松开提交**一次**命令；落在夹点上的点击被一次性闩锁吞掉（**不**重复选中）。**`dom-canvas` 载体（X11/XWayland、web）没有任何夹点标记或拖拽**（wasm 侧无此路径，按铁律 8 显式不存在，**不是**静默回退）。**Escape 取消已接线（仅合成契约，无运行时证据）**：`cad_status` 新增布尔字段 `gripDrag`（`CadStatus::grip_drag`，取自 `HostCore::grip_drag_active()`；拖拽期间 `tool` 仍为 `null`），桌面 UI 的 Escape 门（`toolCommandForKey`）因此在**拖拽中**也发 `CANCEL`（`Enter`/`CONFIRM` 仍只认活动工具）；宿主 `InteractionRequest::Cancel` → `reset_grip_gesture()` 纯状态丢弃拖拽。该链路只有合成证据（`cad-ui-core` 状态投影契约 + `apps/desktop-ui` 的 `tsc`/纯函数/源码结构契约），**没有**真窗口、真实按键或像素证据。插入点夹点**已实现但仅限 `TEXT`/`MTEXT` 的存储插入点**（`GripKind::Insertion` `0`；旋转 / 高度 / 对齐 attachment / 宽度夹点仍**未实现**，模型层不建「永远为空」的 `GripKind` 变体）；`SPLINE` 控制点由模型提供（`GripKind::ControlPoint`），宿主对夹点种类**无分支**，故其标记/拖拽走同一通用路径（**未做**宿主运行/像素验证）；多夹点与多实体拉伸、拖拽期间的 OSNAP 捕捉、3D 夹点编辑、夹点右键菜单与 `GRIPMULTIFUNCTIONAL` 均**不存在**；证据上限：`cargo test -p cad-app -p cad-host` 契约 + 门禁，**无 GUI/像素/真窗口/真实鼠标证据**，Windows 侧更只有 Tier 0 编译 |
| `EXPLODE` | **实现** | **一次一层**：LWPOLYLINE→线段+ARC（bulge 还原）；INSERT→直接成员（placement 烘焙，**嵌套块引用保持为引用**，二次 EXPLODE 才展开）；`Compound`/`Hatch`→**全部子项**（`Hatch` 分解为**边界折线 + 填充网格 + 图案线**；P6a 保持既有行为，块内填充仍按 insert-child 拒绝）；MINSERT（`Compound(Insert…)`）→先得 Insert 实例 | 多级一次到底（导出层的"摊平到叶子"是**另一条路径**，不在本命令内）、`EXPLMODE`、颜色/线型变化、Dimension（按 `type_key` 显式拒绝）、关联阵列（我们不存）、属性语义；`IMAGE`/`MASK` 按**策略**拒绝（其**可变换**，不得假称被变换拒绝）；`TEXT`（**按 `child_refusal` 显式拒绝**——P7a 起变换面**接受** `Text`，故**不得**假称被变换拒绝）/`SPLINE`/`SHAPE`/`OPAQUE` 与镜像 Arc/Ellipse **整命令失败**；`MULTILEADER` **显式按 `type_key` 拒绝**（顶层与块内）；非均匀 INSERT 拒绝 |
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
  见 `docs/handoff.md`）。Windows 宿主 2026-10-23 起已实现（Tier 0 编译 + Wine 拒绝合同，视觉/运行时未验收）；macOS/Android/Web 仍是 NOT IMPLEMENTED 占位，未运行验收。

## 三维

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
  平行棱收敛 31.2%、对角线不等 10.1%**，隐藏边不穿透（`docs/handoff.md` 2026-10-21 条目）。
  边界：俯视正交下"有面立方体"与"填色方片"**像素等价**，故 2D 帧不能证明三维性；
  无 golden/跨 GPU/vendor/性能结论。
  **着色约定**：2D 保持**世界 -Z 头灯**（位级不变）；3D 用**相机相对、偏离视轴的固定主光**
  （`L = normalize(toward_eye + (-0.10)·right + 0.35·up)`，约在视线**左上方 20°**；
  环境 0.25 + 漫反射 0.75，无高光/阴影/环境光）。**纯头灯在等轴测下会退化**：三面相对视线对称
  ⇒ 实测三面同为 `215`；偏移后等轴测为 **204 / 189 / 238**，斜视为三档可辨。
  边界：仍是**显示约定**而非物理光照，两个面在特定方向仍可能接近，反照率/曝光不可分离。
- 直接网格（Mesh/3DFACE）：数据、法向、深度、绘制顺序与透明通道已实现；透明合成有
  软件 Vulkan（lavapipe）无头测试。真实 GPU 行为未验收。
- 底图透明度：importer 读取 acadrust `Transparency`（`ByLayer`/`ByObject`/`ByBlock`）
  → `DisplayFragment.alpha` → `RenderBatch.alpha`，由渲染器分类为不透明/透明/不可见
  （`docs/render-order.md`）；plot-style 表 alpha 未应用（acadrust 只暴露样式表引用名，
  不暴露解析后的颜色/线宽/alpha 值；引用名本身由 `cad-import-acadrust` 读取并显式报告
  为 Unsupported）。
- ACIS 曲面（显示）：**已接线**——导入期离散为中性网格（容差 = 包围盒对角线的 0.1 %），
  恒 `Partial` 并声明「离散近似不随缩放细化 + 源 ACIS 不保留」；超出子集/原始 SAT/SAB 被拒仍为 `Opaque`
  并带内核具名码。详见「ACIS 实体显示状态」。完整 ACIS/样条面支持仍超出范围。

## 平台

> **2026-10-23 重写**：旧段落的 Android/Web/“iOS/Linux/macOS/Windows 无宿主”描述属 Slint 时代，
> 已整体替换为当前状态。能力 ≠ 验收；**没有任何平台是“完整 CAD 产品验收通过”**。

| 平台 | 宿主状态 | 合成 | 有界证据 | 未验证 / 上限 |
|---|---|---|---|---|
| **GPU 显存预算（原生）** | 记账 + 显式预算（默认 3 GiB 估算）、拒绝 `RenderError::GpuMemory` / CLI 退出码 7 | 渲染器按类目记账（current/peak）；驱动 allocator 交叉校验 | Vulkan/P620 + `complex-test.dwg`：常驻 2.11 GB、交叉校验差 0.07 %、`stream` 1.70 GB；强制 8 MiB → exit 7 且无 PNG | 显存**总量**不可查（wgpu 无 API）；其它 GPU/驱动/样本、驱动 OOM 现场、D3D12 阈值均未验证 |
| **Linux / 原生 Wayland** | **受支持，真实可运行**（`apps/app-linux`，bin `yacr`） | **DOM on top**：app-owned `wl_subsurface` 无条件 `place_below(parent)`；GTK toplevel 透明 + 空 opaque region | 生产 `--open` 运行（真实 Vulkan/Quadro P620）、合成帧由同 UID Mutter `ScreenCast` 捕获；WP2 四条判据 PASS（带 caveat）、WP3 About 帧 PASS、**透明修复后复采有界 PASS**（A 几何满亮度/无蒙层、C About PASS；D textEdit 编辑器未像素证明、B 箭头抑制非确定性）；历史 textEdit 帧 **INVALID wrong-window** | 仅 **GNOME/Mutter + P620 + `scale=1`**；DPR>1/分数缩放、多显示器、跨合成器、性能、vendor/golden 均无结论 |
| **Linux / X11 / XWayland** | **已实现 `dom-canvas` 载体**（`apps/app-linux`；退出码 4 已是历史合同） | **DOM-canvas**：wasm/WebGL2 CAD 会话画进 WebView 内 `<canvas>`（`surfaceMode: dom-canvas`），**无**原生 CAD 表面 | 有界运行：`surfaceMode=dom-canvas`、canvas 挂载 + backing `626×341`、合成 fixture 出图（Mutter ScreenCast 抓帧）；**用户手动确认**滚轮缩放/拖拽平移（用户报告，**无** agent 像素差证据）；WebGL2 缺失 → `yacr: unsupported-runtime: webgl2` + **退出码 6** | 仅 **XWayland + GNOME/Mutter + `scale=1`**；真 X11 服务器/WM、其它合成器、**硬件归属（NOT VERIFIED）**、DPR/多显示器、性能、vendor/golden 均无结论；agent 侧输入注入不可用 |
| **Windows** | **已实现**（`apps/app-windows`，bin `yacr-windows`） | **DOM on top**：透明 WebView2（alpha 恰 0）在上，`WS_CHILD|WS_DISABLED` CAD child HWND 在下 | **Tier 0 编译门禁全 rc=0**（msvc clippy/check/build、mingw build、wasm 空壳 check）；**Wine 拒绝合同 smoke 3/3 rc=5** | **视觉/运行时全部 NOT RUN**；spike-1/4/5 **PENDING user real-Windows spike**；Wine 像素结论固定 `NOT VERIFIED (Wine limitation, bug 55432/58921)`；无发布打包 |
| **macOS** | 显式 NOT IMPLEMENTED 占位（`yacr-macos` 失败桩） | — | 无 | — |
| **Android** | 显式 NOT IMPLEMENTED 占位（入口打印退役诊断后 abort） | — | 仅 `cargo check --target aarch64-linux-android` 编译门禁 | 旧模拟器/真机证据属已退役 UI，**不得**复用 |
| **Web** | **wasm CAD 会话已复活**（`apps/app-web`）；浏览器 bundle 本地可构建 | **DOM-canvas**：同一 wasm 会话画进 `<canvas>`（无第二份文档） | 本地运行（Chromium 141 + WebKitGTK 2.52.6）：合成 fixture 出图与缩放；真实 DWG（4.2 MB）在 wasm 内导入（1.43 GB 堆 / 11.2 s）；Chromium **Worker 隔离**（`isolation=worker`），WebKitGTK 显式 `main-thread (no WebGL2 in worker)`；bundle 由 `scripts/build-web.sh` 构建 | **CI/发布未恢复**；大图预算未达标（峰值由导入阶段决定）、浏览器矩阵、移动端、性能均 **NOT VERIFIED**；旧 Web（Slint）证据不得复用 |

- 平台 I/O（文件、配置、表面度量、文件选择器）只存在于 `app-*`；共享层不含平台类型。
- **单一 React UI**：`apps/desktop-ui`（两个桌面宿主共用）。DOM 侧“测试”是 node:test 的源码
  结构/纯函数契约，**不是**运行时 DOM 挂载，也**不是**像素/视觉验收。
- Windows 的判定上限复述：**实现 + Tier 0 + Wine 拒绝 smoke**，视觉由用户真 Windows 自测；
  **不得**写“Windows 已验收”。


## GPU 设备能力协商（2026-10-17，政策见 `docs/adr/0013-device-capability-negotiation.md`）

**本表由设备/适配器探测产出，不是按 backend 类别推断**：`Renderer::caps_for` 按 backend 类别
（WebGl2 vs 其他）报告，**不得**作为本表来源。每格的"本机探测"为**实测**；未在本机跑过的平台一律
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
**选项设置** 17 个类型化键（grid/axes/selectionHighlight/snapHints/plotFormat/snapEnabled/
hoverPreview/gridSnapEnabled/orthoEnabled/snapSpacing/backgroundModel/backgroundPaper/
crosshairSize/pickboxSize/apertureSize/gpuBudgetPercent/wgpu_backend），持久化到 `preference.json`、
**实时生效**并在 open/替换后重新播种（五个 `interaction.*` 叶子：对象捕捉、悬停预览、`SNAP`、
`ORTHO`、`SNAPUNIT` 间距）；**语言默认**按 `显式选择 > 宿主注入 > navigator > 英文`
（`zh-TW/zh-Hant` 无对应目录 ⇒ **英文**，不静默给简体）；**显示**：模型/图纸背景色 `#rrggbb`
（sRGB→linear 在配置→渲染器接缝，真机核对 0.5 读回 188、遮罩内部同值）与十字光标 1..100%
（100% 位级不变），均**即时**生效。

**属性面板（2026-10-22）**：按 `entity/geometry/text/display` 分组；多选显示**共同键交集**并以
`properties.mixed` 占位（写入作用于**整个选中集**）；`layer/color/linetype/lineweight/transparency`
五行可编辑（`enum`/`number`/`text` 编辑器按 `kind` 选，值语法与写侧一致），提交走
`cad_set_selected_property`（本地暂存 → Enter/失焦/下拉即提交，拒绝则回滚并显示 i18n 文案、
code 只进 `title`）；`Ctrl+1` 与命令行 `PROPERTIES` 均可开关（Chrome 本地命令，**不进** Rust 路由）。
**选项弹窗（2026-10-22）**：独立弹窗（ADR-0021），五个页签、12 个写键 + 语言 + 只读字体目录；
Apply/OK/Cancel 为 AutoCAD 语义（本地暂存、只发 diff、写失败标字段）；入口三处（File 页、
`OPTIONS` 命令行拦截、状态栏设置按钮）。**拾取框/孔径**：`view.pickbox.sizePixels`(1..=20, 默认 3)
与 `view.aperture.sizePixels`(1..=50, 默认 12) 已接线到 hover/拾取/光标/捕捉四条路径，实时生效。

**GUI 验收（2026-10-22，有界）**：真实 GPU（Vulkan + Quadro P620）下已实拍主界面、属性面板
（对象/几何/显示 三组 + 可编辑控件 + 单选行值）、命令行 `DIST` 回显（距离/角度/X·Y·Z 增量）、
选项弹窗（720×560 五页签）——证据驱动为 `YACR_PROBE_UI_DIALOGS`（**触发非真鼠标**，下游为
生产路径）。**仍未做（明确不声称）**：弹窗内交互与属性编辑提交/拒绝的 GUI 实测（只有模型+源码测试）；真实 GUI/视觉验收的完整覆盖面（本批**没有**截图或像素判据，选项弹窗与属性面板的
真实窗口行为仅由纯函数模型 + 源码扫描 + 宿主契约测试覆盖，语言的双窗口传播标为 **NOT VERIFIED**）；
自动保存/.bak/xref/CTB/GPU 选择/快捷键自定义/夹点/
极轴/动态 UCS（AutoCAD Options 中存在但本仓库**无行为可接**，因此不显示）。（**颜色色板选择器已于
2026-10-26 补上**，见下方「颜色选择器」条。）

**2026-10-25 补齐（React 宿主，源码/单测证据；真机像素 NOT RUN）**：`NEW`/`SAVE`/`PLOT` 及其别名
经宿主能力布尔（`canNew`/`canSave`/`canPlot`）真实准入，Linux 保存目标走 XDG portal `save_file`；
`GRID`（F7，仅显示栅格）、`SNAP`（F9，会话级栅格捕捉，间距由 `SNAPUNIT <value>` 设置）与
`ORTHO`（F8，仅 XY、上一点水平/垂直约束）成为真实命令；图层面板
（`LAYER`）支持搜索 / 临时显隐 / 恢复 / 设为当前层，并新增**图层表结构写**
（`CREATELAYER` / `RENAMELAYER` / `DELETELAYER`，`NEWLAYER` 是项目同义词别名）：内联新建、
行内重命名、按行删除确认（默认层 `0` 不提供入口），写经**既有**有界队列 ack 后重读。
图层写**不可撤销**（历史只有实体 patch），也**不写 DWG**（`SAVE` 仍是 DXF 导出）。

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
覆盖状态——会话内保留**有界**（上限 **16** 条）的快照环，仅在覆盖**真的改变**前压栈（`ToggleLayer` /
`SETLAYERS` 及其派生 `LAYISO`/`LAYOFF` / `RESTORELAYERS`），no-op 与 `LAYERP` 自身**不压栈**；
历史空 ⇒ `layers.previous.empty` 且不改状态；**不写 DWG、不升 revision、不进历史、不持久化**
（无 `LAYRSTATE`/`LAYRDO`）。`DELETELAYER` 的覆盖清理**刻意**不是一步 `LAYERP` 状态。`LAYISO`/`LAYUNISO` 在 Viewer 模式**可用**（视图操作，不是实体写）。

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
（`SAVE` 仍是 DXF 导出）。键与今天的真实语义（**逐条，勿过度承诺**）：

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
**WIPEOUT 掩码已实现**（非 `inverted` 与 `inverted` 都渲染；不与更高 `draw_order` 交错、矢量出图跳过并具名）。**外部内容** PDF/DWF/DGN
underlay、OLE2 嵌入对象、CoordinationModel/Navisworks NWD、ACIS 全内核（3DSOLID/REGION/
BODY/SURFACE **当前渲染为空**，仅解析为不透明负载，见上文「ACIS 实体显示状态」）、点云点数据（acadrust 0.6.5 仍只暴露元数据/范围，未暴露点数据）均不加载/不实现，显式
`Partial`/`Unsupported`，不计入完成。代理 opcode 厂商记录仍按用户要求排除（见
`docs/proxy-support.md`）。图元逐项状态见 `docs/dxf-entity-coverage.md`。
