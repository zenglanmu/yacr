# AutoCAD 命令覆盖审计（total / done / gap）

> **这是审计，不是验收。** 下表的「实现」只表示**代码路径存在**并有契约/单测覆盖；它**不**表示
> 与 AutoCAD 行为等价、也不表示在真实窗口/真实 GPU/真实图纸上验证过。逐条能力边界以
> `docs/compatibility.md` 为权威，命令与别名以源码表为权威。
>
> **证据边界**：本文所有数字与状态均来自**源码表 + `docs/compatibility.md` + 契约测试**，
> **没有** GUI/像素/运行时/真窗口证据；与 AutoCAD 的逐项行为对照**基本未做**（个别条目在
> `docs/adr/` 里注明「未查证」「我们的规则」）。平台侧（wasm/Windows/Android/macOS）**NOT RUN**。

---

## 1. 「total」是什么，不是什么

仓库里**没有**任何**权威、可枚举的「AutoCAD 命令全集」**：没有 `acad.pgp` 解析器，没有
命令清单数据文件，也没有从 Autodesk 文档导入的表。所以本审计把「total」拆成两个**可辩护**的量：

1. **我们的词表**（可精确计数，源码是唯一权威）：选择子表、测量选择子表、缩放步进表、
   带参数表、别名表。这些数**可以**说「全部通过契约测试」。
2. **显式点名的 AutoCAD 名字**（不可穷尽）：只包含本仓库文档/ADR 真正讨论过的名字，
   来源是 `docs/compatibility.md`、`docs/shortcuts.md` 与各 ADR。它们的状态是**逐条读过源码**后写的。

> **禁止的读法**：不得把 `97`、`155` 或本表任何数字读成「已实现 AutoCAD 的 X%」。AutoCAD 的
> 命令总数取决于版本、产品（完整版/ LT / 工具集）、是否含扩展与第三方；我们没有那份清单，
> 也**没有**逐条比对过绝大多数命令的语义（例如同名命令的选项、提示顺序、系统变量副作用）。
> 任何 `97/N` 形式的百分比都是**编造**。本审计回答的是：**我们自己承诺的词表是否名实相符**，
> 以及**我们明确点过名的 AutoCAD 名字还缺什么**。

---

## 2. 词表总量（每项都有 source of truth）

| 表 | 数量 | 源码位置 | 断言 / 说明 |
|---|---|---|---|
| `COMMAND_TABLE` | **97** 个选择子 | `crates/cad-ui-core/src/session/commands.rs:11` | 与 `CommandId` 变体一一对应；`EXPECTED_COMMAND_COUNT = 97` 钉在 `crates/cad-ui-core/src/session/tests/tests_commands/command_vocabulary.rs:132`（并断言每变体在表中**恰好一次**、`COMMAND_TABLE.len()` 相等） |
| `MEASUREMENT_SELECTORS` | **5** | 同文件 `:207` | `DIST` / `MEASUREPOLYLINE` / `MEASUREANGLE` / `AREA` / `ID`；**交互专用**，没有自己的 `CommandId` |
| `ZOOM_STEP_SELECTORS` | **2** | 同文件 `:234` | `ZOOMIN` / `ZOOMOUT`（固定步进因子 `1.25` 与倒数，锚定视口中心） |
| `ARGUMENT_SELECTORS` | **4** | 同文件 `:252` | `ZOOM` / `SNAPUNIT` / `PDMODE` / `PDSIZE`——**接受命令行参数的基选择子**，不是新命令、不是第二条 IPC |
| `COMMAND_ALIASES` | **51** 行 | 同文件 `:522` | 别名 + 全词同义词；只收录**目标选择子真实存在**的拼写 |
| `admissible_names()` | **155** 个可输入拼写 | 同文件（`admissible_names` 函数） | 97 + 5 + 2 + 51，去重后仍为 155（无重名）；这是**拼写**数，**不是**命令数 |

> 这些数字是**当前源码表的长度**（本次复核：表 97 / 测量 5 / 缩放 2 / 参数 4 / 别名 51 / 拼写 155）。
> **组成**：92 → **95**（+`COPYBASE`/`CUTBASE`/`PASTEORIG`，剪贴板轮）→ **97**（+`CHAMFER POLYLINE`/
> `FILLET POLYLINE`，同日另一 lane）。表一变，本表与 §5 的期望输出都要跟着改；**权威是 §5b 那道
> 守门脚本的实跑输出**，不是这里的手抄值。

**为什么不把 155 当命令数**：别名只是把输入**归一化**到同一个选择子（`normalize_command`），
不改变行为；`ZOOMIN`/`ZOOMOUT` 与各测量选择子不携带 `CommandId`。命令的**行为**数上限是
`CommandId` 变体数（= 97），其中还有若干是**本项目自己的**名字（不是 AutoCAD 命令）。

**本项目自己的名字（AutoCAD 无同名，或同名不同义）**，不计入 AutoCAD 覆盖：
`RESETVIEW`、`SWITCHBACKEND`、`SWITCH2D3D`、`STANDARDVIEW`（AutoCAD 的 `VIEW` 是命名的视图管理）、
`MODE`（Viewer/Work）、`RESOURCES`、`DIAGNOSTICS`、`SPACE`（模型/图纸空间切换）、
`SELECTINBOX`、`CONFIRM`/`CANCELMEASURE`、`GRIPSTRETCH`、`HOVERPREVIEW`、`TOGGLELAYER`/
`SETLAYERS`/`RESTORELAYERS`/`SETACTIVELAYER`/`CREATELAYER`/`RENAMELAYER`/`DELETELAYER`/
`SETLAYERPROPERTY`（AutoCAD 的对应物是 `-LAYER`/`LAYER` 子选项，语义不同）、
`SETENTITYPROPERTY`、`MEASUREPOLYLINE`/`MEASUREANGLE`（AutoCAD 的 `PLINE` 现在是绘制命令，
`ANGLE` 不是 AutoCAD 命令）、`AREA OBJECT`（AutoCAD 的 `AREA`→`Object` 提示的**选择式**变体）。
**`MEASURE` 是同名不同义**：AutoCAD 的 `MEASURE` 沿对象按间距放置点/块；本仓库的 `MEASURE`
是**测量工具入口**（默认距离），不产生任何实体。把它算作「已覆盖 MEASURE」是错的。

---

## 3. AutoCAD 命名命令的状态表

状态口径（三档 + 一条排除）：

- **实现**：有真实交互/命令路径，几何或状态**落库**（该落库的），未支持选项显式拒绝。
- **部分**：能用，但**有写明的界**（列在「界」列；界之外一律显式 `Unsupported`/`InvalidInput`）。
- **未实现**：没有命令路径（输入该名得到 `command.unknown`，或宿主显式 `command.unsupported`）。
- **同名不同义**：见 §2 末段，不算覆盖。

### 3.1 绘制

| AutoCAD 名 | 状态 | 界 |
|---|---|---|
| `LINE` | 实现 | 两点；退化/共线/非有限拒绝 |
| `CIRCLE` | 实现 | 圆心 + 半径点；无 `2P`/`3P`/`D`（直径）入口 |
| `ARC` | 实现 | **三点弧**；无 起点-圆心-端点 / 起点-圆心-角 / 连续弧 |
| `PLINE` | 实现 | 只产**开放 LWPOLYLINE**、**每段 bulge=0**；无弧段/`Close`/`Width`/`Length`/`Undo` |
| `RECTANG` | 实现 | 两对角点 → 闭合 4 顶点 LWPOLYLINE；无 `Chamfer`/`Fillet`/`Width`/`Elevation`/`Rotation`/`Area` |
| `POLYGON` | 实现 | **内接于圆**、边数 3–1024；无 `Edge` 分支、**无外切模式** |
| `ELLIPSE` | 实现 | 轴端点法**整椭圆**；无 `Arc`/`Center` 法/`Rotation`/`Isocircle` |
| `POINT` | 实现 | **单次放置**；显示走 `$PDMODE`/`$PDSIZE`（基础 `0`–`4`，周边 `32` 圆/`64` 方/`96` 两者，线段近似）；无 `Multiple` 重复放置 |
| `SPLINE` | 实现 | fit 点法（≥4 点）→ 三次/clamped/chord 插值 NURBS；无 `CV`/`Knots`/`Degree`/`Close`/`Tangency`/`Object` |
| `POLYLINE` | **未实现** | 旧式 heavy 2D 折线实体（`AcDb2dPolyline` + 顶点子实体）。我们**只创建 LWPOLYLINE**；别名到 `PLINE` 会**凭空发明语义**，故不做 |
| `XLINE`（含 `RAY`） | **未实现**（无创建命令；**导入**有有界降级） | 库里**没有无限直线几何**：导入期把 XLINE/RAY 压成**有限 `SemanticGeometry::Line`**，裁到**运行中的模型包围盒**（盒每边外扩对角线 5%、至少 1.0）⇒ 裁多长**取决于导入顺序**（先出现什么几何，盒就是什么）；还没有任何几何时用**默认 1000 单位盒**。判定：方向退化 → `Opaque` + `Partial`；裁不到盒内 → `Opaque` + `Partial`；否则 `Line` + **`Partial`**（原因是具名常量 `support::XLINE_SEGMENT_REASON` / `XLINE_DEFAULT_BOX_REASON`，并额外记一次 `import.construct_line`）。**导出**只能写 DXF `LINE`，报 `Converted`（`export.converted.line`，点名「不再是无限线」），**不是**精确往返。没有创建命令、没有无限表示。**规模**：命令 **M**（建在既有有界切片上）；真正的无限几何 **L / ADR 级**（见 §4） |
| `HATCH` | 部分 | **先选闭合边界实体**（非内部点拾取）；填充类型 `0` 实体 / `1` 用户自定义 / `2` 预定义 / **`3` 渐变（2026-10-27）**；预定义图案**恰好** `ANSI31`/`ANSI32`/`ANSI37`（状态栏**按名**选，线上传 1 基索引）。**渐变创建**：双色两站（`stop 0.0` = `hatchGradientColorA`、`stop 1.0` = B），kind 只有 `LINEAR`/`SPHERICAL`（**从不**写 `CYLINDER`），角度**通道给度、实体存弧度**（一次 `.to_radians()`），颜色是 **ACI 整数 `1..=255`**（`0` 是 ByBlock 占位、无 sRGB，按名拒绝），默认 1 红 / 5 蓝；实体写 `HatchGradient::TwoColor { reserved: 0, shift: 0.0 }` + `pattern_type = Predefined`（`76=1`，**未查证的 vendor 约定**）。**界**：`SPHERICAL` 在矩形/圆这类「边界顶点同半径」的形状上**烘焙成平色/椭圆感、不是径向**（按三角化顶点取值）；只有 ACI 调色板（无真彩、无名/色簿）；stop 颜色下游还会被实体/图层 tint 乘；**无单色/tint 变体、无逐站命名**。无内部点求边界、无 `76=2`/`.pat`、无关联性、无实时预览 |
| `MLEADER` | 部分 | 单簇/单线/2 顶点/**仅直线**；无 `Options`（引线类型/内容类型/最多点数/角度约束）；狗腿/箭头/字高取 `Standard` 样式表（常量为债务） |
| `TEXT` | 实现 | **单行**、1024 字符上限、**拒绝换行**；不解释 MTEXT 控制码（逐字存储）；**旋转取会话参数 `textRotation`**（通道**度**、任意有限、**不 wrap**、默认 `0`，创建时**一次** `.to_radians()` 存弧度；载荷本身没有旋转字段） |
| `MTEXT` | 部分 | **两角宽度模型** + 终端文本；**旋转取 `textRotation`**（同上，度→弧度一次）、**附着取 `mtextAttachment`**（闭集 `1..=9` 的 3×3 网格：`1..=3` top / `4..=6` middle / `7..=9` bottom，每行左/中/右——`h = (code−1) % 3`、`row = (code−1)/3`，**永不产出 `TextAlignV::Baseline`**；默认 `1` 左上），越界 code 与非有限旋转**按名拒绝**、不钳制；**仍未实现**：行距、分栏、背景填充、绘制方向、绘制流程内 MTEXT 对话框 |
| `TEXTEDIT` | 部分 | 拾取一个文本 → **预填当前串** → 整串替换；无光标级编辑、不解析 MTEXT 标记（MLEADER 三条入口已接） |

### 3.2 标注

| AutoCAD 名 | 状态 | 界 |
|---|---|---|
| `DIMLINEAR` / `DIMALIGNED` | 实现 | **三点流程**（两原点 + 尺寸线位置）；**非关联**；无文字覆盖；当前样式恒为 `Standard` |
| `DIMRADIUS` / `DIMDIAMETER` | 实现 | **实体拾取圆弧/圆** + 位置；组 40 写 `0.0`（引线延长未建模）；弧的引线点落在**整圆**上、不钳到扫掠范围 |
| `DIMANGULAR` | 部分 | **仅两线形式可交互**；三点形式只到命令/载荷层；拾取范围**仅 `Line`** |
| `DIMORDINATE` | 实现 | 特征点 + 引线端点；自动 X/Y 规则为**我们的规则**（`\|Δy\| > \|Δx\|` ⇒ X） |
| `DIMCENTER` | 实现 | 产出**普通 `LINE` 几何**，**不是** DIMENSION 实体；`DIMCEN = 0` ⇒ 0 实体 + 具名诊断 |
| `DIMBASELINE` / `DIMCONTINUE` | 实现 | **显式拾取基准线性标注**（零跨命令状态）、N 点 + Enter、**一条事务/一步撤销**、`DIMDLI` 行距；**仅线性基准**、**不写 `DIMCONTINUEMODE`**、非关联、无文字覆盖 |
| `DIMSTYLE` | **未实现（编辑）** | 表与 **B.2(a)+B.2(b) 变量子集**已建模（`cad-db` 的 `DimStyle`，`Standard` 恒存在），但**没有样式编辑命令/UI**，实体存**已解析快照**、编辑样式**不追溯** |
| `DIMEDIT` / `DIMTEDIT` / `DIMOVERRIDE` / `DIMBREAK` / `DIMSPACE` / `QDIM` / `DIMARC` / `DIMJOGGED` | **未实现** | ADR-0017 非目标（逐条列在 `docs/adr/0017-dimension-family.md:10-12`） |
| 关联性（`DIMASSOC=2`） | **未实现** | 创建与迁移的标注都是非关联单实体；导出不写 `ACAD_DIMASSOC` |

### 3.3 修改

| AutoCAD 名 | 状态 | 界 |
|---|---|---|
| `MOVE` | 实现 | 平移选择；不可变换几何（`Shape`/`Opaque`/`Compound`…）整条拒绝；`SPLINE` 可平移（NURBS 仿射不变） |
| `COPY` | 实现 | 按位移复制、新 id；样式覆盖无法复制时记 `drawing.copy.style` |
| `ROTATE` | 实现 | 绕基点旋转（相似变换）；选择含 `MultiLeader` 时**仍可旋转**（缩放则整命令失败，见 `SCALE`） |
| `SCALE` | 实现 | **均匀**缩放；非均匀拒绝；`MultiLeader`/`TEXT` 等按类拒绝 |
| `MIRROR` | 部分 | 镜像并**新增副本**（保留原件）；含 `Arc`/`Ellipse`（含椭圆弧）；**拒绝 `Text`**（平面文字无法表达真反射）与 `MultiLeader` |
| `TRIM` | 部分 | 目标 `LINE`/`ARC`/`CIRCLE`/`SPLINE`；边界含 `LINE`/`ARC`/`CIRCLE`/`ELLIPSE`/`LWPOLYLINE`（含 bulge）/`SPLINE`（**采样**，附诊断）；拒绝 `INSERT`/`Opaque`/`Mesh` 目标、无交点/投影出域 |
| `EXTEND` | 部分 | 目标 **`LINE` 或 `ARC`**（弧延伸上限半圈）；**`SPLINE` 目标显式拒绝**（NURBS 端点移动会改参数化，无外推策略）；`CIRCLE` 目标拒绝 |
| `OFFSET` | 部分 | `LINE` / `CIRCLE` / `ARC` / `LWPOLYLINE`（直线段**与含弧段**，后者 2026-10-28）；`ARC` 与 `CIRCLE` 共用 `\|p−c\|` 半径模型。**弧段同心**（`R' = R − s·sign(sweep)`；`R' <= tol` ⇒ `offset.arc_collapsed`，不删除不混合）；接点 = miter（line/line）/ 最近解析交点（line-circle、circle-circle；0 或两个等距 ⇒ `offset.join_failed`），相切闭式；弧 start/sweep 由接点重导并过 `bulge_arc_geometry`（`offset.arc_mismatch`）；闭合环含 wrap 角、开口弧端不封口。**按名拒绝**：`arc_collapsed`/`join_failed`/`ambiguous_plane`/`arc_mismatch`/`bulge_segment`（只剩非有限或不可构造）/`collapsed`/`non_planar` 与 `ELLIPSE`；**不做**弧段删除与邻居混合；**无全局自交修剪**；弧增长上限 `\|new\| <= \|old\| + π` |
| `ARRAYRECT` / `ARRAYPOLAR` | 部分 | **独立副本**（非关联，有意偏离）；无 `Path`/`Levels`/`Total`/角度间隔输入/关联编辑；极坐标角度步长 `i·fill/items` 是**我们的规则**（与 AutoCAD 的 `fill/(items−1)` 不同，已测试） |
| `STRETCH` | 部分 | 恒**交叉窗口**；逐顶点（折线）/控制点（样条）；`TEXT` 与 `MultiLeader` **整命令拒绝**；`HATCH` 部分包围不动 + 诊断；块参照只**刚性平移插入点**、不逐成员拉伸 |
| `ALIGN` | 部分 | **两对点**（平移 + 旋转 + 可选均匀缩放）；无 3 对点 3D 对齐、无非均匀缩放、无「1 对点移动」 |
| `EXPLODE` | 部分 | **一次一层**；`LWPOLYLINE`→线段+`ARC`、`INSERT`→直接成员（嵌套保持引用）、`Hatch`→子项；**拒绝** `Dimension`/`MULTILEADER`/非均匀 `INSERT`；`TEXT` 按子项 `child_refusal`；`SPLINE`/`SHAPE`/`OPAQUE`/镜像 `Arc`·`Ellipse` **整命令失败**；无 `EXPLMODE` |
| `JOIN` | 部分 | **仅**共线首尾相接 `LINE` 或同心相续 `ARC`；折线/样条/圆、需桥接空隙、跨图层/空间 ⇒ 按名拒绝 |
| `ERASE` | 实现 | 空选择拒绝；撤销恢复渲染属性与动态可见性 |
| `CHAMFER` | 部分 | **两线形式**：两条独立直线、两距离按拾取顺序施加；无 `Angle` 方法/`Multiple`/`Trim` 开关。**Polyline 模式是独立选择子** `CHAMFER POLYLINE`（2026-10-28，选择式、恰好 1 条开放直线段 `+Z` LWPOLYLINE；`chamferDistanceA` 沿**入边**、`B` 沿**出边**——**我们的规则**） |
| `FILLET` | 部分 | **两线形式**：两条独立直线 + 半径（`R=0` 只修角点）；平行线**显式拒绝**（不自动调半径）；无 `Multiple`/`Trim` 开关。**Polyline 模式是独立选择子** `FILLET POLYLINE`（2026-10-28，选择式、同上形状；半径取 `filletRadius`、每个内顶点加圆弧；`R ≤ 0` 按名拒绝、**不做** AutoCAD 的自动缩半径） |
| `MATCHPROP` | 部分 | **选择式**（AutoCAD 特性刷的文档化变体：首个选中为源）；只转移 `layer`/`color`/`linetype`/`lineweight`/`transparency` 五项；**无 Settings 子集**；文字样式/标注样式/填充图案一律**不**转移 |

### 3.4 块 / 外部参照 / 发布

| AutoCAD 名 | 状态 | 界 |
|---|---|---|
| `INSERT`（**读取**侧） | 实现 | 块定义与实例分离、实例展开几何（嵌套有界，保留 instance path）、可拾取、插入点夹点可拖（仅 `native-below` 宿主） |
| `INSERT`（**创建**侧） | **未实现** | 没有「插入块参照」命令/面板路径 |
| `BLOCK` | **未实现** | 无块定义创建命令（域与 DB 有块表，但不存在写入口令） |
| `WBLOCK` | **未实现** | 无「写块到文件」 |
| `XREF` | **未实现** | 无附着/加载/重载/绑定；仓库里只有 `cad-resources` 的 **xref 深度预算**与图层的 `xref_dependent` 标记，**不是** xref 能力 |
| `IMPORT` | **未实现** | 打开 DWG/DXF 是**打开管线**，不是 `IMPORT` 命令（无 `IMPORT` 选择子） |
| `PUBLISH` | **未实现** | 无多布局批量出图 |

### 3.5 图层 / 显示 / 开关

| AutoCAD 名 | 状态 | 界 |
|---|---|---|
| `LAYISO` | 实现 | 会话级显隐覆盖；keep = **所选实体图层 ∪ 当前层**（**我们的规则**）；**不写 DWG、不升 revision、不进历史** |
| `LAYOFF` | 实现 | 关闭选择所在图层，当前层除外；同样只是会话覆盖 |
| `LAYON` | 实现 | `RESTORELAYERS` 的**别名**（清空全部会话覆盖），不新增 `CommandId` |
| `LAYERP` | 部分 | 会话内**有界 16 步**快照环（仅真实改动前压栈）；**不持久化**，无 `LAYRSTATE`/`LAYRDO` |
| `GRID` | 实现 | **仅显示栅格**开关（另需 `view.overlays.grid` 偏好为真才绘制）；不移动任何点 |
| `SNAP` | 实现 | 会话级**栅格捕捉**（F9，二维：X/Y 量化，`z` 原样）；默认间距 **10.0 图形单位**是**我们的选择** |
| `OSNAP` | 实现 | 对象捕捉开关；候选生成/最佳候选规则见 `crates/cad-host`（不在本文范围） |
| `ORTHO` | 实现 | 仅 XY 约束；约束顺序 **OSNAP → 栅格捕捉 → 正交**，与 AutoCAD 内部顺序不同（已记录的刻意偏差） |
| `PDMODE` | 部分 | `0`/`1`/`2`/`3`/`4` 已画；`32`/`64`/`96` 周边圆/方**已实现（2026-10-28）**：圆 = 16 段弦环、方 = 4 段，半径 = 标记半幅（比例是我们的选择，**未经 AutoCAD 核对**）；写图纸头、**不可撤销** |
| `PDSIZE` | 实现 | `>0` 绝对尺寸 / `0` 视口比例 / `<0` 视口高度百分比；非有限/超限回退 `0` 行为 + 诊断 |

### 3.6 剪贴板 / 视图 / 文件

| AutoCAD 名 | 状态 | 界 |
|---|---|---|
| `COPYCLIP` / `CUTCLIP` / `COPYBASE` / `CUTBASE` / `PASTECLIP` / `PASTEORIG` | 部分（**集群已实现**，`PASTEBLOCK` 缺） | **应用内**剪贴板：**不是** OS 剪贴板、不跨应用、替换文档即清空。**两个捕获形状**：`COPYCLIP`/`CUTCLIP`（`Ctrl+C`/`Ctrl+X`，**不记基点**；`COPYCLIP` 只读、Viewer 可用；`CUTCLIP` work-only）与 `COPYBASE`/`CUTBASE`（`Ctrl+Shift+C`/`Ctrl+Shift+X`，在预选择之上**再抓一个基点**存进剪贴板元数据；`COPYBASE` 只读、Viewer 可用；`CUTBASE` work-only，**先删除成功再提交剪贴板（含基点）**，失败的剪切不毁旧剪贴板）。**两个粘贴形状**：`PASTECLIP`（`Ctrl+V`，**单点插入点工具**，平移 `插入点 − 基点`，`base = clipboard_base.unwrap_or(世界原点)`，复用 COPY 的同一个 `cad_db` 变换——**不再是原坐标粘贴**）与 `PASTEORIG`（原坐标，忽略基点，等价改动前的 `PASTECLIP`）。所有权重走 `commands/accelerators.ts` → `admitCommandText` 的同一准入。**仍缺**：`PASTEBLOCK`（`Ctrl+Shift+V`）**未实现**——`cad-db` 没有块写 API，加速器表故意拒绝该 chord、不降级成 `PASTECLIP` |
| `ZOOM` | 部分 | `ZOOM <factor>`（正十进制，锚定视口中心）、`ZOOM ALL`/`EXTENTS`（⇒ `FIT`）、`ZOOMIN`/`ZOOMOUT`；**无** `WINDOW`/`PREVIOUS`/`OBJECT`/`DYNAMIC`，也**无**图纸空间比例 `nXP` |
| `PAN` | 实现 | 视图平移（实时） |
| `VIEW` | 部分 | 预设标准视图（top/bottom/front/back/left/right/isometric）经 IPC；**无**命名视图的保存/恢复/删除 |
| `ORBIT` | 部分 | `{yaw, pitch}` 载荷，**仅三维视图**；无连续/受约束 orbit 交互 |
| `PROJECTION` | 实现 | 透视/正交切换 |
| `OPEN` | 实现 | 打开 DWG/DXF（宿主提供能力时） |
| `NEW` | 实现 | 新建空图纸；宿主缺能力时显式 `command.unsupported` |
| `SAVE` | 部分 | **有损 DXF 另存**（逐条记录转换/丢弃）；**没有** DWG 回写、没有 DWG 往返 |
| `PLOT` | 部分 | `PLOT` 是**矢量 SVG/PDF 导出**；无 CTB/STB 打印样式、无批量、无多布局发布；实体能力表里另记的「光栅 PNG（host readback）」是**既有出图通道**，不在本命令的默认路径里 |

### 3.7 测量选择子（5 个 + `MEASURE` 同名不同义）

| 选择子 | 状态 | 界 |
|---|---|---|
| `DIST` | 实现 | 2 点：距离 + 角度 + Δx/Δy/Δz；单点不足即拒绝。**没有** `DI` 别名，也不是 `MEASURE DISTANCE` 这种多词名字（那个名字不存在） |
| `AREA` | 实现 | **用户选点**闭合多边形面积；自交/非共面/少于 3 点拒绝；**无** `Add`/`Subtract`。**没有** `AA` 别名，也不是 `MEASURE AREA` |
| `ID` | 实现 | 单点拾取 → X/Y/Z + 空间 + 捕捉来源 + 精度；**故意跳过**栅格捕捉与 `ORTHO`（**我们的规则**）；不写库、无撤销步 |
| `MEASUREPOLYLINE` | 实现（**本项目选择器**） | N 点开放链长度，需 `CONFIRM` 收尾 |
| `MEASUREANGLE` | 实现（**本项目选择器**） | 3 点顶点角；纸空间 `ANGLE` 显式拒绝（`ANGLE` 只是**拒绝文案里提到的 AutoCAD 名**，不是本仓库的选择子，也不是别名） |
| `MEASURE` | **同名不同义** | 本仓库是**测量入口**（默认 `DIST`＝距离），不沿对象放置点/块；不构成 AutoCAD `MEASURE` 的覆盖。它是**独立选择子**（`CommandId::Measure`），与上表五个交互选择子不是别名关系 |

> **这张表没有 `（别名 …）` 单元是有意的**：`COMMAND_ALIASES` 里没有任何指向这五个交互选择子或
> `MEASURE` 的别名——`DI`/`AA`/`ANGLE` 都不存在。§5b 的自检做两件事：① 本表第一列必须是准入名字；
> ② 全文里每一个被写成别名的反引号 token 都必须真在 `COMMAND_ALIASES` 里。两条都只防「再造名字」，
> 其余语义仍是**人工核对**。

---

## 4. 缺口清单与规模

规模口径（**粗估，不含验证成本**）：**S** ≈ 一个有界补丁（单模块、无新契约面）；
**M** ≈ 一个模块（新命令 + 载荷 + 测试 + i18n + 文档）；**L** ≈ 子系统 / 需要 ADR 决策。

| 缺口 | 规模 | 为什么不是「顺手补一下」 |
|---|---|---|
| `POLYLINE`（旧式 heavy 2D 折线**实体**）创建 | **M** | 需要写 `AcDb2dPolyline` + 顶点子实体（本仓库的唯一折线语义变体是 `Polyline`，类名不进域模型）；**不得**把 `POLYLINE` 别名到 `PLINE`——那会把 heavy 折线**假装**成 LWPOLYLINE，凭空发明语义 |
| `XLINE`（含 `RAY`） | **M（命令）/ L（真正的无限几何）** | **命令（M）**只建在**既有有界切片**上：复用导入期的有限段降级 + 一条创建命令/工具，牵动 `TRIM`/`EXTEND` 的无限直线求交语义与出图裁剪，但不新增几何种类。**真正的无限几何（L，ADR 级）**要新增一个 `SemanticGeometry` 变体：约 **14 处**编译强制点（validate / bounds / transform / tessellate / engine / spatial pick·bounds / representation provider / measure snap / `geometry_kind` / 导出 / 选择属性行）＋约 **12 处**静默兜底（`intersect` 的 `_ => None`、extent 回退、grips 的 `_ => Vec::new()` 等）——而 `cad-domlib` 级的 bounds 累积会被无限线**毒化**成无限模型范围。这不是补丁 |
| `BLOCK` / `WBLOCK` / `INSERT` 创建 | **L** | 块定义写入口、基点、属性定义（ATTDEF/ATTRIB）、嵌套与命名冲突、与 EXPLODE/夹点/STRETCH 的相互作用；WBLOCK 还要单独文件写出 |
| `XREF` | **L** | 外部文件解析、相对/绝对路径解析、循环深度（已只有预算钩子）、图层名前缀、绑定/重载语义；跨文件一致性属新契约面 |
| `IMPORT` | **L** | 依赖 acadrust 的格式能力与 `cad-import-acadrust` 的迁移策略；每个格式都要自己的 capability 表与拒绝面（DWG/DXF 的打开走 `OPEN`，不是 `IMPORT`） |
| `PUBLISH` | **L** | 多布局/多图幅批量出图 + 设备/图幅/打印样式；`PLOT` 现在只到矢量单页 |
| `DIMSTYLE` **编辑** | **M** | 表与变量子集已在 `cad-db`；缺的是命令/UI 与**追溯语义**决策（实体存已解析快照，改样式是否回写实体是产品决策，ADR-0017 B.1 定为**不追溯**） |
| HATCH **内部点求边界** | **M** | 需要射线/环嵌套搜索 + 岛屿判定 + 与既有「预选边界」路径的**同一**校验器；失败面（未闭合、自交、相切）必须逐条具名 |
| HATCH 自定义 `76=2` / `.pat` 加载 | **M** | `.pat` 解析器（含 `*`/`;` 注释、角度/原点/偏移/虚线定义）+ 名称解析 + 缺失图案的显式拒绝；现在只有**我们转录**的三项内置表 |
| ~~HATCH **渐变创建**~~ | **已关闭（2026-10-27）** | 双色 `LINEAR`/`SPHERICAL` 已实现（类型码 `3` + 四个参数，见 §3.1）。**仍未闭合**的是它的三个性质缺口，不再算「创建缺失」：`SPHERICAL` 在矩形/圆边界上烘焙成平色（需要按值细分内部三角形）、只有 ACI（无真彩/色簿）、`76=1` 的梯度约定**未用 vendor 文件核实** |
| HATCH **关联性** | **L** | 需要边界→填充的持久引用（类似 `ACAD_DIMASSOC`），并定义边界编辑后的重生成/失效规则；与「实体非关联」的既有边界冲突，需 ADR |
| ~~`FILLET`/`CHAMFER` 的 **Polyline 模式**~~ | **已关闭（2026-10-28）** | 两个独立选择子 `FILLET POLYLINE`/`CHAMFER POLYLINE`（选择式 `CommandPayload::None`、恰好 1 条开放直线段 `+Z` LWPOLYLINE、原地重写一条事务/一步撤销）；**仍未闭合**的是四条**声明偏差**：闭合环、含 bulge 段、非 `+Z` 平面都按名拒绝（不是 AutoCAD 的全功能子选项），`TRIMMODE` 不建模，AutoCAD 的**自动缩半径**不复刻，倒角的 `A`/`B` 方向是**我们的规则** |
| `PASTEBLOCK`（`Ctrl+Shift+V`，粘贴为块） | **L** | 需要「把剪贴板实体包成一个块定义再插入」——`cad-db` **没有块写 API**（无 `BlockDefinition` 写入路径，现有 `insert_block` 只服务测试/导入构造器），要先有块定义写入口 + 命名/去重 + 与 EXPLODE/夹点/STRETCH 的交互；在它存在之前**不得**把 `Ctrl+Shift+V` 降级成 `PASTECLIP` |
| ~~OFFSET **含弧段折线**~~ | **已关闭（2026-10-28）** | 弧段现在**精确同心**偏移（`R' = R − s·sign(sweep)`），接点按最近交点 + 闭式相切求解，弧经 `bulge_arc_geometry` 回验。**仍未闭合**的是三条**声明偏差**：不做 AutoCAD 的弧段删除与邻居重接（半径耗尽即 `offset.arc_collapsed`）、**全局自交修剪未做**（只在顶点检局部塌缩）、**2 顶点（纯弧）或全弦共线带 bulge ⇒ `offset.ambiguous_plane`** |
| OFFSET **`ELLIPSE`** | **L** | 等距偏移曲线**不是**椭圆（非圆二次曲线的高阶偏移），要么新增偏移曲线类型、要么折线近似并显式声明；两条都不是补丁 |

> 其它已知缺口（不属上述名字，但读表时容易误判为已完整）：`MATCHPROP` 的 Settings 子集、
> `ARRAY` 的 Path/关联编辑、`STRETCH` 的 HATCH 部分拉伸、
> OS 剪贴板（真系统剪贴板）与 `PASTEBLOCK`（粘贴为块）、`ZOOM WINDOW/PREVIOUS`、命名视图、CTB/STB 打印样式、
> 非均匀缩放与镜像的文字/`MULTILEADER`（域限制，非命令层缺失）。

---

## 5. 如何复跑这次审计

```bash
# 1) 命令表 / 选择子表 / 别名表的真实数量（源码是唯一权威）
python3 - <<'PY'
import re
src = open('crates/cad-ui-core/src/session/commands.rs').read()
def rows(pat):                       # 取 (\"NAME\", …) 形式首列
    m = re.search(pat, src, re.S)
    return re.findall(r'\("([^"]+)"', m.group(1))
table = rows(r'pub const COMMAND_TABLE: &\[\(&str, CommandId\)\] = &\[(.*?)\n\];')
meas  = rows(r'pub const MEASUREMENT_SELECTORS: &\[\(&str, MeasurementToolKind\)\] = &\[(.*?)\n\];')
zoom  = rows(r'pub const ZOOM_STEP_SELECTORS: &\[\(&str, f64\)\] =\s*&\[(.*?)\];')
# ARGUMENT_SELECTORS 是常量标识符（不是字符串字面量）
args  = re.findall(r'^\s{4}([A-Z_]+_SELECTOR),$',
                   re.search(r'pub const ARGUMENT_SELECTORS: &\[&str\] = &\[(.*?)\n\];', src, re.S).group(1), re.M)
aliases = [a for a, _ in re.findall(r'\("([^"]+)",\s*"([^"]+)"\)',
           re.search(r'pub const COMMAND_ALIASES: &\[\(&str, &str\)\] = &\[(.*?)\n\];', src, re.S).group(1))]
names = list(dict.fromkeys(table + meas + zoom + aliases))   # = admissible_names() 的顺序
print(len(table), len(meas), len(zoom), len(args), len(aliases), "admissible:", len(names))
PY
# 期望输出：97 5 2 4 51 admissible: 155

# 2) 钉住 97 的契约（变体数、表长、每变体恰好一次）
cargo test -p cad-ui-core --locked command_normalization_covers_every_variant
cargo test -p cad-ui-core --locked admissible_names_cover_the_vocabulary_exactly_once

# 3) 命令表本身（人工复核分组）
sed -n '11,190p' crates/cad-ui-core/src/session/commands.rs

# 4) 别名表
sed -n '507,600p' crates/cad-ui-core/src/session/commands.rs

# 5) 逐条能力边界（本文第 3 节的权威来源）
grep -n "^| " docs/compatibility.md
grep -n "仍未实现的 AutoCAD 名" -A 20 docs/shortcuts.md
```

复跑时**必须**同时读 `docs/compatibility.md` 的「绘制/编辑命令」「创建命令」「编辑命令」
「DIMENSION 族」四节：本文第 3 节只是它们的**索引与归类**，不重述全部拒绝码。

### 5b. 名字自检（**会非零退出**；已是仓库脚本 + CI 门禁）

`docs/shortcuts.md` §2 曾经混进**不存在的命令名与别名**（`MEASURE DISTANCE`、`CLEAR SELECTION`、
`VIEW TOP/…`、`TOOLS`/`RIBBON`、`ZOOM IN`/`ZOOM OUT`、`DI`/`ZE`/`ESC`/`RIBBON` 等 **35 处**），本文 §3.7
也曾把 `DI`/`AA`/`ANGLE` 写成测量选择子的别名。**规则不再以代码块形式住在本文件里**：唯一可运行副本是
`scripts/check-command-docs.py`，自测是 `scripts/test-check-command-docs.py`，CI 的 `core-quality` 每次
都跑两者（见 `docs/ci.md` 的 `core-quality` 行与脚本清单）。四条检查（§2 canonical 列、§2 别名单元、
§2b 全表、§3.7 第一列 + 别名提法）的判据只写在脚本的 docstring 里，**本文不复制**。

```bash
python3 scripts/check-command-docs.py        # 一致 → 0 / 有 mismatch → 1 / 读不出结构 → 2
python3 scripts/test-check-command-docs.py   # 自测：注入伪造 canonical 与伪造别名，必须退出 1
```

**本次实测（仓库当前树）**：

```
tables: 97/5/2/4 selectors, 51 aliases, 104 canonical selectors (aliases excluded)
§2 rows: 59, §2b aliases: 51, §3.7 rows: 6, alias mentions: 1
OK: 0 mismatches
```

（`104` 是 **canonical 选择子**数：97 表 + 5 测量 + 2 缩放步进，**不含** 51 个别名；`155` 才是
「可输入拼写」数，见 §2。两个数不要混用。）

**它确实会失败**（同一个脚本，换输入就红）：

- `python3 scripts/check-command-docs.py --doc <修复前的 shortcuts.md>` →
  退出码 1、**35 处** mismatch：18 个非准入 canonical + 14 个假别名 + §2b 缺 3 行
  （`LAYON`/`LAYUNISO`/`NEWLAYER`）。
- 把本文 §3.7 的 `DIST` 行改回带 `DI` 别名的旧写法 → 同样退出 1，
  报 `§3.7 not admitted: …` 与 `alias mention 'DI' is not in COMMAND_ALIASES`。

**守门范围（别把它当全页保险）**：只守**名字**（§2 / §2b / §3.7 第一列 / 别名提法）。
`docs/shortcuts.md` 的 §1（命令行行为）、§3（键位）、§4（未实现清单）、§5（证据边界）以及本文 §3
的**语义**（部分/实现的界）仍是**人工按源码核对**，没有机器守卫。

---

## 6. 本审计**不**主张的事

1. **不**主张任何「AutoCAD 兼容率」或百分比（见 §1）。
2. **不**主张同名命令行为等价：多处已标「我们的规则」「未查证」「有意偏离」
   （极坐标阵列角度步长、`MIRROR` 保留原件、`ARRAYPOLAR` 独立副本、`DIMORDINATE` 自动 X/Y、
   HATCH 预定义图案值为**我们的转录**）。
3. **不**主张真实图纸/真实窗口/真实 GPU 验证；除个别历史有界运行（见各 ADR 的实施补充）外，
   证据是**契约测试与源码结构**。
4. **不**把「未实现」读成「永远不做」：缺口规模只是工作量分类，不是排期承诺。
