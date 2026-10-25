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
| POINT | 实现 | 实现 | **overlay 屏占比十字标记**（draw_order 850 000，始终在 overlay 层、与基础场景实体遮挡关系不同）；尺寸 = 视口高的 5%；覆盖**顶层**导入 POINT；上限 4000 且只发一条诊断；**3D 不画**（`viewport_bounds` 为 `None`）；`PDMODE`/`PDSIZE`/点样式**未实现**；**块内 POINT 不标记**（限制已记录） | 实现 | 部分 |
| SOLID / TRACE / 3DFACE | 实现 | 网格(四边形→三角) | 网格 | 实现 | 不支持 |
| INSERT / 块 | 实现(定义与实例分离) | 实现 | 实例展开块几何(嵌套有界，保留 InstancePath) | 实现 | 不支持 |
| TEXT / MTEXT | 实现(保留字体名/字高/旋转) | 实现 | 整形为线段：outline(TTF/OTF/WOFF)+SHX(shapes/unifont/bigfont)，缺失字体走回退链。**可变换（P7a）**：平移/旋转/均匀缩放；**镜像与非均匀缩放拒绝**（平面文字无法表达真反射）；捕捉面 `engine.rs` 同规则（跨面一致性有测试）。**P7a 起 `TEXT` 创建实现**：1 点 + 文本（`cad_set_tool_text`，终端语义、1024 字符上限），`AcDbText`，一条事务/一步撤销。**MTEXT 的列/背景填充/行距样式/绘制方向仍不保留**，导入改为 **`Partial` + 四条具名原因**（此前误报 `Complete`）；**编辑现有文本已实现（A+B）**：`TEXTEDIT`（拾取一个 `Text` → **预填当前串** → 终端文本提交 → `update_entity`，**一条事务/一步撤销**，只改字符串，样式/高度/旋转/位置/对齐保留）+ **属性面板可编辑字段**（当前单选，**不暴露实体 id**，宿主自行解析并拒绝）；**整串替换**（无光标级编辑、不解析 MTEXT 标记）；拒绝面：非 `Text`/块内成员/子元素、空、超 1024、非单选、拾取后目标已变。**MLEADER 文本编辑已实现（P8）**：A（`TEXTEDIT`）/B（面板内联）/C（**弹窗**）三条入口；MTEXT 与 MLEADER 文本多行、`\n`↔`\P`；TEXT 单行且**拒绝换行**（AutoCAD 不解释 `\P`）；其余控制码逐字透传；**`MLeaderText.width` 已参与排版（2026-10-21）**：`MultiLeaderGeometry::text_geometry()` → `SemanticGeometry::Text.wrap_width` → 表示层**贪心换行**（拉丁按词、CJK 按字；`\P` 段落各自独立；超宽不可断词独占一行；对齐按**每行**应用；`None`/`0`/负值/非有限 = 不换行，输出位级不变）。`Partial`：无 UAX #14 断行、无 kinsoku、跨 token 字距不测量、实体包围盒仍用未换行估计；**MTEXT 的 41 组（`rectangle_width`）导入侧仍不保留** | 同上 | 不支持 |
| HATCH（读取/显示） | 实现 | 部分(椭圆/样条边界近似) | **P6a 起为一级 `Hatch` 实体**：边界环 + **样式感知**填充（`Normal`/`Outer`/`Ignore` 按**环集筛选**，1 基深度）+ 实心/图案/渐变；超预算/坏环降 `Partial`。**渲染变化（P6a 正确性修复）**：① `Outer`/`Ignore` 此前一律按 `Normal` 填（`Outer` **只在 ≥3 层**可见、`Ignore` 有岛屿即不同）；② 图案复合修正（**缩放基点、旋转 offset**；属 **U-1 我们的实现**，**非**外部保真验证）；③ 用户自定义图案（`76=0` 且 `78=0`）此前 `Partial`，现由 `52/41/77` **合成**；④ **环深度探测**此前只看环的**首顶点**（`fill::ring_depths`），**相切导入环**可能被判为退化或错填，现改为**最多 8 点均匀探测 + 确定性回退**（与 `loop_depths` 共享同一助手）；创建路径不受影响（先拒绝相切） | 精确局部包围盒（**bulge 环退回 AABB**，声明） | 不支持 |
| HATCH（创建） | **实现（P6b，预选闭合实体）** | 部分 | **先选边界对象、再执行 `HATCH`**（与 `ARRAY`/`MOVE` 同型：0 点 arity + `requires_selection`）。接受 `Circle`/完整 `Ellipse`/**闭合** `Polyline`，每个成一环，须**共面**（圆/椭圆另须**法向一致**）。参数：图案类型（`SOLID`/用户自定义）、间距（>0）、角度（**通道度、域弧度**）、双线、样式（`Normal`/`Outer`/`Ignore`）。**圆 = 4 段精确圆弧**（`bulge=tan(π/8)`；**非** ADR 原写的两点环——校验器要求 ≥3 顶点，ADR 已更正）；椭圆按 64/圈**采样**（真椭圆非圆弧，bulge 会是假几何）。**内部点求边界、LINE 拼接、预定义图案名（`ANSI31` 等）、渐变创建、关联性、真正的填充预览均 NOT IMPLEMENTED**。拒绝面：非闭合/开放折线、`Line`/`Arc`、样条、椭圆弧、自交、相切/重复、部分重叠、退化、非共面、非法参数、无活动图层、Viewer。**创建校验基于弦**：两段 bulge 环的**弦间**相切可能漏检（声明） | — | 不支持 |
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

| 命令 | 能力 | 诚实边界 |
|---|---|---|
| LINE / CIRCLE / ARC | 创建（点捕获或几何直通） | 退化/共线/非有限输入显式拒绝 |
| MOVE | 平移选择（折叠重复 ref） | 不可变换几何整条拒绝，不静默丢弃 |
| COPY | 复制（新 id） | 同上；按对象样式覆盖无法复制时记 `drawing.copy.style`（副本继承图层样式） |
| ERASE | 删除选择 | 空选择拒绝；撤销恢复渲染属性与动态可见性成员 |
| TRIM | LINE 对 LINE / 全直线 LWPOLYLINE | 其余边界/目标 `Unsupported`；无交点/退化 `InvalidInput` |
| EXTEND | 把 LINE 延伸到最近边界 | 同 TRIM 子集；已接触/已穿过/无交点显式拒绝 |
| ROTATE / SCALE | 绕基点旋转/均匀缩放 | 相似变换（正行列式）；不支持几何 `Unsupported`。**含 `MultiLeader` 的选择：旋转可以，缩放 ⇒ 整命令失败**（ADR-0019 C.5：掩码未置位时 AutoCAD 会从样式重解析） |
| MIRROR | 镜像（**保留原件**，新增副本） | 反射下 Arc/Ellipse 参数化会反向 → 显式 `Unsupported` |
| OFFSET | LINE / CIRCLE 偏移 | 其余几何 `Unsupported`；点在线上/过圆心/在圆上显式拒绝 |
| COPYCLIP/CUTCLIP/PASTECLIP | **应用内**剪贴板 | 非 OS 剪贴板；替换文档清空；空选择/空剪贴板拒绝 |

- 变换/偏移的**不支持几何**（`Spline`/`Shape`/`Opaque`/`Compound`；`MIRROR` 下的 `Arc`/`Ellipse`；**以及 `MultiLeader`——拒绝一切缩放**）返回显式
  （**更正**：`Text` 自 P7a 起**可**平移/旋转/均匀缩放，**不在**此列表；仅镜像与非均匀缩放拒绝。）
  `Unsupported`；退化/无交点等 plan 级失败返回 `InvalidInput`。二者都不创建或部分修改实体。
- 命令行、别名、固定键位与补全行为的完整参考见 **`docs/shortcuts.md`**；`Ctrl+C/X/V` 仅为应用内
  剪贴板，不读写操作系统剪贴板。
- **导出**：`SAVE` 为**有损 DXF 另存**（`Partial`，逐条记录转换/丢弃，**非** DWG 往返保存）；
  `PLOT` 为矢量 SVG/PDF。真实图纸上的导出兼容性**未验证**。逐类：LWPOLYLINE（平面的）/ELLIPSE/POINT
  为精确映射；**SPLINE 记为 `Converted`**（`DxfSpline`，几何保留，**closed/periodic 标志不携带**）；
  Shape/Image/Mask/Opaque 被丢弃并逐条记录。

### 创建命令（ADR-0015，逐条按实施状态）

| 命令 | 状态 | 实体 | 显式未实现 |
|---|---|---|---|
| `PLINE` | **实现** | 开放 LWPOLYLINE（每顶点 bulge=0） | 弧段/`Angle`/`Center`/`Direction`/`Radius`/`Second pt`/`Halfwidth`/`Width`/`Close`/`Length`/顶点 `Undo`；3D/OCS |
| `RECTANG` | **实现** | 闭合 LWPOLYLINE（4 顶点，bulge=0） | `Chamfer`/`Fillet`/`Thickness`/`Elevation`/`Width`/`Area`/`Dimensions`/`Rotation`（含圆角会话内持久怪癖）；3D/OCS |
| `ELLIPSE` | **实现** | 单个 `Ellipse`（`center`、`+Z`、中心相对 `major_axis`、`ratio = 短/长 ≤ 1`、`start=0`、`sweep=2π`） | 轴端点法；`Center` 法/`Arc`/`Rotation`/`Isocircle`/`PELLIPSE`；**长短轴重排为未对齐假设**：本仓库把较长者存为长轴以保证 DXF 组 40 ≤ 1，AutoCAD 确切重排规则**未查证**；预览为有界弦近似（16–128 段），存储实体精确 |
| `POLYGON` | **实现** | 单个闭合 `Polyline`（n 顶点、bulge=0、`+Z`），**内接于圆**；第一顶点在"中心→半径点"射线上（指针方向决定旋转） | `Edge` 分支与**外切**模式；"数值半径 ⇒ 底边对齐捕捉旋转角"；边数**不跨会话持久化**（会话参数，默认 4，范围 3–1024，越界显式拒绝） |
| `POINT` | **实现** | 单个 `Point`（`+Z` 平面）；**单次放置**（AutoCAD 是否重复放置**未查证** ⇒ 不按猜测实现）；显示走 overlay 十字标记（见上方 POINT 绘制列） | `PDMODE`/`PDSIZE`/点样式、Multiple Points 重复放置、**3D 视图中的点标记** |
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
| `STRETCH` | **实现** | 交叉窗口（**恒 CROSSING**）：**部分包围=逐顶点移动**（只动窗内顶点）、**完全包围=整移**；分类按几何类型分两种口径（直线/无 bulge 折线=逐顶点包含；圆/椭圆/块/ARC/含 bulge 折线=保守包围盒完全包含）；**bulge 数值保留**（未查证） | cpolygon/lasso、**单独选取即整移**、`TEXT`（**相似变换语义受限** ⇒ 整命令拒绝，ADR-0019 B.2——**不是**因为变换面拒绝：P7a 起 `Text` 可平移/旋转/均匀缩放）、`MultiLeader`（**显式按 `type_key` 整命令拒绝**）、`SPLINE`/`ARC`/`HATCH`/`INSERT` 的**拉伸**（部分包围不动+诊断、完全包围整移）、3D；宽度未建模 |
| `EXPLODE` | **实现** | **一次一层**：LWPOLYLINE→线段+ARC（bulge 还原）；INSERT→直接成员（placement 烘焙，**嵌套块引用保持为引用**，二次 EXPLODE 才展开）；`Compound`/`Hatch`→**全部子项**（`Hatch` 分解为**边界折线 + 填充网格 + 图案线**；P6a 保持既有行为，块内填充仍按 insert-child 拒绝）；MINSERT（`Compound(Insert…)`）→先得 Insert 实例 | 多级一次到底（导出层的"摊平到叶子"是**另一条路径**，不在本命令内）、`EXPLMODE`、颜色/线型变化、Dimension（按 `type_key` 显式拒绝）、关联阵列（我们不存）、属性语义；`IMAGE`/`MASK` 按**策略**拒绝（其**可变换**，不得假称被变换拒绝）；`TEXT`（**按 `child_refusal` 显式拒绝**——P7a 起变换面**接受** `Text`，故**不得**假称被变换拒绝）/`SPLINE`/`SHAPE`/`OPAQUE` 与镜像 Arc/Ellipse **整命令失败**；`MULTILEADER` **显式按 `type_key` 拒绝**（顶层与块内）；非均匀 INSERT 拒绝 |
- **证据范围**：上述表格是**契约/场景**证据，不等于真实图纸兼容性。Linux 桌面宿主（React/Tauri）
  另有**真实硬件 GPU** 有界证据（Vulkan / Quadro P620 / X11 —— **CAD-on-top 时代**证据；
  2026-10-23 起 X11 被显式拒绝，勿当作当前后端能力）：打开与渲染、悬停、框选（含橡皮筋像素判据）、
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
**选项设置** 10 个类型化键（grid/axes/selectionHighlight/snapHints/plotFormat/snapEnabled/
hoverPreview/backgroundModel/backgroundPaper/crosshairSize），持久化到 `preference.json`、
**实时生效**并在 open/替换后重新播种；**语言默认**按 `显式选择 > 宿主注入 > navigator > 英文`
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
颜色的色板选择器（仅文本语法输入）；自动保存/.bak/xref/CTB/GPU 选择/快捷键自定义/夹点/
极轴/动态 UCS（AutoCAD Options 中存在但本仓库**无行为可接**，因此不显示）。

**2026-10-25 补齐（React 宿主，源码/单测证据；真机像素 NOT RUN）**：`NEW`/`SAVE`/`PLOT` 及其别名
经宿主能力布尔（`canNew`/`canSave`/`canPlot`）真实准入，Linux 保存目标走 XDG portal `save_file`；
`GRID`（F7，仅显示栅格）与 `ORTHO`（F8，仅 XY、上一点水平/垂直约束）成为真实命令；图层面板
（`LAYER`）支持搜索 / 临时显隐 / 恢复 / 设为当前层（写入仍走既有命令漏斗，不写 DWG 图层表）；
CAD 分区右键**快捷菜单**已实现（复制/剪切/粘贴/全选/删除/特性/缩放到范围/平移/放弃/重做/重复上一命令）。
仍无行为可接而**不显示**的 AutoCAD 项：`SNAP`（栅格捕捉 F9）、夹点、极轴、动态 UCS。

2026-10-10 图元显示补全轮新增边界：**光栅图像** PNG/JPEG 已解码并出纹理，但
TIFF/CCITT/EPS 等编解码器、亮度/对比/淡出/透明、outside/mask 裁剪显式未支持；
Web/Android 宿主未接图像加载器；**`draw_order` 已贯通到场景批次**（含 INSERT 继承、合批不跨顺序），
但**渲染器仍按上传顺序绘制、遮罩恒最后**（opaque mesh 还跨批次拍平，分段交错未实现）；
**WIPEOUT 掩码已实现**（非 `inverted` 与 `inverted` 都渲染；不与更高 `draw_order` 交错、矢量出图跳过并具名）。**外部内容** PDF/DWF/DGN
underlay、OLE2 嵌入对象、CoordinationModel/Navisworks NWD、ACIS 全内核（3DSOLID/REGION/
BODY/SURFACE **当前渲染为空**，仅解析为不透明负载，见上文「ACIS 实体显示状态」）、点云点数据（acadrust 0.6.5 仍只暴露元数据/范围，未暴露点数据）均不加载/不实现，显式
`Partial`/`Unsupported`，不计入完成。代理 opcode 厂商记录仍按用户要求排除（见
`docs/proxy-support.md`）。图元逐项状态见 `docs/dxf-entity-coverage.md`。
