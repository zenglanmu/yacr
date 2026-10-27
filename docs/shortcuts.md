# 命令行与快捷键参考

> **范围说明（2026-10-27 更正，取代原「历史文档」横幅）**：`cad-ui-slint` 与 `yacr-linux` 已随
> Slint 退役删除（部分旧脚本改为非零 NOT IMPLEMENTED）。本文 §1–§5 **描述当前 React/Tauri 宿主**；
> 唯一保留的 Slint 内容都已**就地标注为「Slint 时期（已退役）」**（§3 的键位历史段），不得当作现状。
> 当前架构与构建见 `docs/architecture.md`、`docs/react-linux-app.md`、`docs/build.md`、
> [ADR 0010](adr/0010-react-linux-host-replaces-slint.md)。
>
> **证据上限**：本文的命令名/别名有 §5b（`docs/autocad-command-coverage.md`）的机器自检；**行为语义与
> 键位仍只有源码/契约证据**，没有真实窗口、真实按键或像素验收。

本文件是本程序（命令行 + 固定键位 + 绘制/编辑工具）的**单一诚实参考**。证据以**源码契约 + 单测**
为主；历史轮次的离屏/软件 Vulkan（lavapipe）证据**不代表**真实 GPU、真实窗口或真实设备；逐轮证据与
未完成项见 `docs/handoff.md`。支持的硬件/平台结论见 `docs/compatibility.md`。

术语（**2026-10-27 更正**）：**canonical 选择子**是 `crates/cad-ui-core/src/session/commands.rs` 的
`COMMAND_TABLE`（每个都对应一个 `CommandId`）**加上**交互专用的测量/缩放步进选择子
（`DIST`/`MEASUREPOLYLINE`/`MEASUREANGLE`/`AREA`/`ID`、`ZOOMIN`/`ZOOMOUT`——它们**没有** `CommandId`）；
**别名**只来自同一文件的 `COMMAND_ALIASES`，作用仅是把输入**归一化**，不改变行为。命令行解析/补全/分发
在 `cad-ui-core`，所有宿主共享。（旧版本此处指向已删除的 `crates/cad-ui-slint/src/command_completion.rs::SELECTORS`。）

---

## 1. 命令行行为

- **前缀匹配**：输入按空白折叠并转大写后，先尝试**精确**匹配 canonical 选择子 / 别名
  （当前无门控，见下）；否则做**前缀**匹配。
  - **唯一前缀** → Enter 执行该命令。
  - **歧义前缀**（匹配多个）→ 命令行**保持打开**并给出候选（不猜测、不执行）。
  - **空 Enter**：**不执行任何命令**（`admitCommandText('')` 返回 `empty`，chrome 不回退到“重复上一条”）；
    要重跑旧命令请用 `↑` 从历史取回整行。（旧文档的“重复上一条成功分发的命令”是 **Slint 时期**行为。）
  - **无法识别的名字**：chrome 侧就按 `command.unsupported` 拒绝（`isSupportedCommand` 为假，
    不转发给路由器）；若名字绕过 chrome 直达路由器，返回稳定字符串 `unsupported CAD command: <name>`。
    **`command.unknown` 这个 i18n 键没有消费方**（只剩在目录里）——旧文档“未知 → `command.unknown`、
    已识别未实现 → `command.unsupported`”的两分法**不是现状**。
- **补全候选**（`apps/desktop-ui/src/components/CommandLine.tsx` + `commands.ts::filterCommandSuggestions`；
  旧文档指向的 `command-bar.slint` 已随 Slint 删除）：
  - 候选来源是 **`COMMAND_CATALOG`（显示目录）+ 宿主别名表**（`cad_command_aliases`），
    **不是**全部 97 个选择子；上限 **8** 条；排序 = 前缀命中 → 别名前缀 → 中缀命中 → 历史使用频次 →
    字母序（**不去重**，也**不是**纯字典序）。
  - 输入变化即时计算候选；候选只**填充**文本，**从不执行**命令。
  - `Tab`：**仅当有高亮行**时把命令行替换成该名字（**没有**“无高亮则第一项”的回退）。
  - `↑` / `↓`：候选列表打开时移动高亮；否则翻**命令历史**。
  - `Return`：无参数且存在高亮行时按该名字执行；**带参数**（如 `ZOOM 2`）时保留整行提交，
    不会被裸名字替换。
  - `Space`：**没有**特殊行为（旧文档的“填充选择器”不存在）。
  - `Esc`：关闭候选/交还焦点（IME 组合期间交给输入法）。
- **门控（2026-10-27 更正）**：当前**没有**门控。短别名（`L`、`C`、`M`、`CO`…）**始终可用**：
  `canonical_command_name` 无条件解析 `COMMAND_ALIASES`，没有任何生产代码读取
  `interaction.keyboardShortcuts`（该键只剩在 `crates/cad-app/src/viewer_config/tests.rs` 的测试夹具里）。
  旧文档「关闭时短别名保持未知」是 **Slint 时期**的配置语义，不是现状。
- 命令行**不是** payload 解析器：坐标、路径、CAD 脚本、payload 一律不从这里转发给文档命令。

---

## 2. 命令表（只列**有行为注解**的选择子）

> **完整清单不在这里**：穷尽的 **97 个 `COMMAND_TABLE` 选择子**（含源码位置与契约断言）见
> `docs/autocad-command-coverage.md` §2；本节只保留**有行为注解**的行。
> 选择子与别名的**唯一权威**是 `crates/cad-ui-core/src/session/commands.rs`
> （`COMMAND_TABLE` **97** / `MEASUREMENT_SELECTORS` **5** / `ZOOM_STEP_SELECTORS` **2** /
> `ARGUMENT_SELECTORS` **4** / `COMMAND_ALIASES` **51**）。
> 本表的 **canonical 列**只出现**被准入的名字**，**别名列**只出现 `COMMAND_ALIASES` 里的真实行；
> 键位、面板与 IPC 能力见 §2a，**不混进命令表**。

canonical 选择子（`work-only` 表示仅在工作模式可用，Viewer 模式由命令层显式拒绝）：

| canonical | 类别 | work-only | 别名 / 同义词 |
|---|---|---|---|
| `LINE` | 绘制 | 是 | `L` |
| `CIRCLE` | 绘制 | 是 | `C` |
| `ARC` | 绘制（三点弧） | 是 | `A` |
| `MOVE` | 编辑 | 是 | `M` |
| `COPY` | 编辑 | 是 | `CO`、`CP` |
| `ERASE` | 编辑 | 是 | `E` |
| `TRIM` | 编辑 | 是 | `TR` |
| `EXTEND` | 编辑 | 是 | `EX` |
| `ROTATE` | 编辑 | 是 | `RO` |
| `SCALE` | 编辑 | 是 | `SC` |
| `MIRROR` | 编辑（保留原件） | 是 | `MI` |
| `OFFSET` | 编辑（LINE / CIRCLE / **ARC** / **直线段折线**；含弧段折线与 `Ellipse` 按名拒绝） | 是 | `O` |
| `MATCHPROP` | 编辑（**选择式**特性匹配：首个选中实体为源，其余为目标；转移 `layer`/`color`/`linetype`/`lineweight`/`transparency` 五项） | 是 | `MA` |
| `JOIN` | 编辑（合并选择为**一个**实体：共线 `LINE` 或同心相续 `ARC`；一条事务、一步撤销，仅这两种几何） | 是 | — |
| `FILLET POLYLINE` | 编辑（AutoCAD 的 **Polyline 模式**，独立选择子：**恰好 1 个**选中实体，每个**内顶点**用会话参数 `filletRadius` 换成两切点 + 一段圆弧；实体**原地重写**、一条事务/一步撤销；两线 `FILLET` 不变） | 是 | — |
| `CHAMFER POLYLINE` | 编辑（同上选择式：`chamferDistanceA` 量在**入边**、`chamferDistanceB` 量在**出边**——**我们的规则**，不是 AutoCAD 提示顺序的复刻） | 是 | — |
| `LAYOFF` | 图层（关闭选择所在图层，当前层除外；会话覆盖，不写 DWG、不升 revision） | 否 | — |
| `LAYERP` | 图层（恢复**上一次**覆盖状态；有界 16 步快照环，仅真实改动前压栈；会话内，不持久化） | 否 | — |
| `LAYISO` | 图层（隐藏选择**不在**的图层 + 当前层保留；会话覆盖，不写 DWG、不升 revision） | 否 | — |
| `RESTORELAYERS` | 图层（清空**全部**会话覆盖 = AutoCAD 的 LAYON/LAYUNISO 之意） | 否 | `LAYON`、`LAYUNISO` |
| `AREA OBJECT` | 测量（对当前选择的一个闭合实体求精确面积：圆 πr² / 完整椭圆 π·a·b / 闭合折线 bulge 解析面积；选择式，非交互拾取） | 是 | — |
| `COPYCLIP` | 剪贴板捕获（**只读**，无事务；**不记基点**） | 否 | — |
| `CUTCLIP` | 剪贴板剪切（捕获 + 一次事务删除；**不记基点**） | 是 | — |
| `COPYBASE` | 剪贴板捕获 **+ 基点**（选一个基点；只读、开 0 事务，Viewer 亦可用） | 否 | — |
| `CUTBASE` | 剪切 **+ 基点**（先删除成功、再提交剪贴板与基点；work-only） | 是 | — |
| `PASTECLIP` | 粘贴到**插入点**：平移 `插入点 − 基点`（无基点时基点为世界原点；与 COPY 同一个 `cad_db` 变换） | 是 | — |
| `PASTEORIG` | 粘贴到**原坐标**（忽略基点；等价改动前的 `PASTECLIP`） | 是 | — |
| `MEASURE` | 测量入口（默认 `DIST`＝距离） | 是 | — |
| `DIST` | 测量：2 点距离 + 角度 + Δx/Δy/Δz | 是 | — |
| `MEASUREANGLE` | 测量：3 点顶点角（纸空间显式拒绝） | 是 | — |
| `AREA` | 测量：**用户选点**闭合多边形面积（自交/非共面/少于 3 点拒绝） | 是 | — |
| `MEASUREPOLYLINE` | 测量：N 点开放链长度（需 `CONFIRM` 收尾） | 是 | — |
| `ID` | 测量（点坐标查询：单点拾取 → X/Y/Z + 空间 + 捕捉来源 + 精度；**故意跳过栅格捕捉与 `ORTHO`**） | 是 | — |
| `PDMODE` | 点显示模式（参数形式 `PDMODE <n>`：0 点/1 无/2 `+`/3 `×`/4 竖线；32 圆框 / 64 方框 / 96 两者，按固定段数线段近似）——写图纸头值、刷新显示、**不可撤销** | 是 | — |
| `PDSIZE` | 点显示大小（参数形式 `PDSIZE <n>`：>0 绝对图纸尺寸 / 0 视口比例 / <0 视口高度百分比）——同上 | 是 | — |
| `UNDO` | 历史 | 是 | `U` |
| `REDO` | 历史 | 是 | — |
| `CONFIRM` | 交互（工具进行中时 `Enter` → 确认；`ENTER` 是**真实别名**） | 是 | `ENTER` |
| `CANCEL` | 交互（取消最高优先级交互；`Esc` 是**键**，不是别名） | 否 | — |
| `SELECTALL` | 选择（全选可见模型空间对象） | 否 | — |
| `PAN` | 导航 | 否 | `P` |
| `FIT` | 导航（缩放到范围；`ZOOM EXTENTS` 是**真实别名**） | 否 | `ZOOM EXTENTS` |
| `ZOOM` | 导航（参数形式 `ZOOM <factor>` 比例因子、锚定视口中心；`ZOOM ALL`/`ZOOM EXTENTS` 走 `FIT`，裸 `ZOOM` 显式拒绝） | 否 | `Z` |
| `ZOOMIN` | 导航（固定步进放大 `1.25×`，锚定视口中心） | 否 | — |
| `ZOOMOUT` | 导航（固定步进缩小，步进为放大的倒数） | 否 | — |
| `OPEN` | 宿主 | 否 | — |
| `NEW` | 宿主 | 否 | — |
| `SAVE` | 宿主（有损 DXF 另存） | 否 | `QSAVE`、`SAVEAS` |
| `PLOT` | 宿主（矢量导出 SVG/PDF） | 否 | `EXPORT`、`PRINT` |
| `SWITCH2D3D` | 观察（2D 俯视 ↔ 3D 透视切换；切回 2D 恢复保存的相机） | 否 | — |
| `PROJECTION` | 观察（透视/正交切换） | 否 | — |
| `MODE` | 模式（**需 `Mode` 载荷**：Viewer/Work；裸调用按名拒绝） | 否 | — |
| `DIAGNOSTICS` | UI（诊断抽屉） | 否 | — |
| `GRID` | 显示（**仅**栅格叠加开关，2026-10-25 起真实选择器） | 否 | `GR` |
| `SNAP` | 显示（**栅格捕捉**开关 F9；会话状态，不写库，经 `interaction.gridSnapEnabled` 持久化） | 否 | — |
| `SNAPUNIT` | 显示（参数形式 `SNAPUNIT <value>`：栅格捕捉间距 sysvar，正十进制数，经 `interaction.snapSpacing` 持久化） | 否 | — |
| `OSNAP` | 显示（对象捕捉开关） | 否 | `OS` |
| `ORTHO` | 显示（正交约束开关；会话状态，不写库，经 `interaction.orthoEnabled` 持久化） | 否 | `OR` |
| `HOVERPREVIEW` | 显示（悬停预览开关） | 否 | — |

说明：
- `ZOOM <factor>` 是 AutoCAD 的**比例因子**缩放（2026-10-25 起可用）：`factor > 1` 放大，`factor < 1`
  缩小（与 `ZOOMIN`/`ZOOMOUT` 同一约定，复用同一条 `zoom_step` 路径），且缩放**锚定在视口中心**
  （中心点不动）。`factor` 的语法是**正的十进制数**：仅 ASCII 数字与至多一个两侧都有数字的小数点
  （`2`、`0.5`、`1.25`、`2.0`）。`ZOOM 0`、`ZOOM -1`、`ZOOM abc`、`ZOOM 2 3`、`.5`、`1e3` 一律
  **显式拒绝**（稳定原因写入 `cad_status.error`，相机不变）——不截断、不取默认值、不静默忽略。
  `ZOOM ALL` 与 `ZOOM EXTENTS` 是 **`ZOOM` 的参数写法**（不是命令名），与别名 `ZOOM EXTENTS`→`FIT`
  都走 `FIT` 路径。命令名与参数之间允许一个或多个空格/制表符，行尾空格忽略，大小写不敏感；
  不带参数的裸 `ZOOM` 仍走既有的“该命令需要 payload”显式拒绝，不会编造一个默认因子。
  这是**命令行参数**，不是新命令、不是新 IPC。
- `GRID` 只切换**显示栅格叠加**（会话状态；宿主还要 `view.overlays.grid` 偏好同时为真才真正绘制）。
  它与栅格**捕捉**（`SNAP`/F9）是两个独立开关，`GRID` 不移动任何点。
- `SNAP` 切换**会话内的栅格捕捉**（默认**关**，与 AutoCAD 初始状态一致）：开启后每次取点把 **X 与 Y**
  四舍五入到会话捕捉间距的最近整数倍（`z` 原样保留，栅格捕捉是**二维**辅助）。**恰好一半**的值
  **远离零**取整（`x = 1.5 × spacing → 2 × spacing`，`-1.5 → -2`），这是本构建明确选择的规则。
  间距的默认值是 **10.0 图形单位**，这是**我们的选择**：AutoCAD 的初始 `SNAPUNIT` 来自样板（template）
  的单位设置，本仓库无法从图纸推断，因此显式声明而不是猜测。取点约束的**完整顺序**是
  **对象捕捉 → 栅格捕捉 → 正交**（宿主先做对象捕捉，会话内先量化再施加正交，因此正交仍决定最终轴；
  与 AutoCAD 内部顺序不同，属已记录的刻意偏差）。橡皮筋预览与提交点共用同一个会话约束函数
  （`crates/cad-app/src/session_state.rs::constrained_point`，量化规则在 `crates/cad-app/src/snap.rs`），
  因此预览不会与提交分歧。**实体拾取步骤不捕捉**（TRIM/EXTEND 等拾取的是实体而非点，与 `ORTHO` 一致）。
  它是会话/显示状态：不写数据库、不产生撤销步、不 bump revision，Viewer 与 Work 均可用。
  **用户的开关选择会持久化**：`cad_set_setting` 键 `gridSnapEnabled` 写入持久叶子
  `interaction.gridSnapEnabled`（同一份 `preference.json`），在**启动**与**每次权威换图（open/new）**后由宿主的同一 seeding 路径
  重新施加（默认仍未设置 = **关**，不写偏好就保持会话默认）。设置写入只在**活值不同**时才派发
  本开关的切换命令；值已相同是纯偏好写入，不再切换（不会把开关翻回去）。
- `SNAPUNIT <value>` 设置**会话内的捕捉间距**：只接受**正的十进制数**（`0.5`、`25`、`2.5`），语法与
  `ZOOM` 的数值部分相同（仅 ASCII 数字与至多一个两侧都有数字的小数点）。`SNAPUNIT 0`、`SNAPUNIT -5`、
  `SNAPUNIT abc`、`SNAPUNIT 2 3`、`.5`、`1e3`、`ALL` 一律**显式拒绝**（稳定原因写入
  `cad_status.error`，间距不变）；不带参数的裸 `SNAPUNIT` 仍走“该命令需要 payload”的显式拒绝，
  不会编造默认值。与 `ZOOM` 一样，这是**命令行参数**（同一套 `typed_command_argument` 机器），
  不是新命令层、不是新 IPC；成功设置只改会话状态，无数据库/历史写入。
  **间距会持久化**：`cad_set_setting` 键 `snapSpacing`（有限且 > 0 的实数）写入持久叶子
  `interaction.snapSpacing`（同一份 `preference.json`），并在**启动**与**每次权威换图**后重新施加到新会话（未设置时保持声明的 **10.0**；
  `0`、负数、非有限值一律是显式拒绝，绝不静默回落到默认值）。
- `ORTHO` 只切换**会话内的正交约束**（默认**关**，与 AutoCAD 初始状态一致）：开启后，绘图与测量的
  **下一次取点**被约束到相对该工具**上一个已取点**的水平或垂直方向（`|dx| >= |dy|` 取水平，
  恰好相等也取水平；`z` 保持原始输入值，正交是**二维**辅助，不在 Z 上移动点）。首个点没有参考点，
  因此原样使用；关闭时原样使用。它**不**包含极轴追踪、相对/绝对坐标输入模式。实现上顺序是
  **先对象捕捉、再栅格捕捉（`SNAP`）、最后正交**（与 AutoCAD 内部顺序不同，属已记录的刻意偏差）；
  只改会话/显示状态，不写数据库、不产生撤销步，Viewer 与 Work 均可用。
  橡皮筋预览与提交点共用同一约束（见 `crates/cad-app/src/ortho.rs`）。
  **用户的开关选择同样持久化**（`cad_set_setting` 键 `orthoEnabled` → 持久叶子
  `interaction.orthoEnabled`），启动与每次权威换图后重新施加；
  未设置时保持会话默认（**关**）。与 `SNAP` 一样，设置写入只在活值不同时派发切换命令。
- `COPYCLIP` 是**只读**捕获，Viewer 亦可用；`CUTCLIP`/`PASTECLIP` 为 work-only。
- `OPEN`/`NEW`/`SAVE`/`PLOT` 为**宿主自有**：宿主未提供能力时命令层返回显式 `Unsupported`
  （不会静默 no-op）。
- 别名：**唯一权威**是 `crates/cad-ui-core/src/session/commands.rs::COMMAND_ALIASES`（**51** 行），
  本页 §2b 是它的转写。旧文档引用的 `cad-ui-slint` 别名/同义词/未实现命令三张表**已随 Slint 退役删除**，
  不要再去仓库里找它们。

### 2a. 不是命令行选择子（键位 / 面板 / IPC）

以下能力**真实存在**，但**不是**可键入的 canonical 选择子；把它们写进命令表会凭空造出名字。

- **标准视图（IPC）**：命令表里只有 `STANDARDVIEW`，它**需要载荷**（裸调用被显式拒绝）；六个正交视图
  与等轴测（`top`/`bottom`/`front`/`back`/`left`/`right`/`isometric`，`StandardView::ALL` 共 **7** 个键）
  由宿主 IPC **`cad_standard_view`** 投递（`crates/cad-desktop-shell/src/commands.rs`）。
  **`VIEW TOP`、`VIEW BOTTOM`、`VIEW MODE` 之类的“命令行名”不存在。**
- **清除选择**：**没有** `CLEAR SELECTION` / `DESELECT` 这个选择子或别名。`SELECTALL` 是全选；
  `CANCEL` 取消进行中的交互；`Esc` 是**键**（见 §3），**不是**命令名、也不是 `DESELECT` 的别名。
- **chrome 本地选择子**：`LAYER`、`PROPERTIES` 由 chrome 自己消费
  （`apps/desktop-ui/src/commands/commands.ts::LOCAL_CHROME_COMMANDS`）；`OPTIONS` 在
  `apps/desktop-ui/src/App.tsx::submitCommand` 里被同样地本地拦截（打开选项对话框）——三者都
  **不经过** CAD 路由器、也**不是** `COMMAND_TABLE` 成员。**没有** `TOOLS` / `RIBBON` / `PANELS` 命令。
- **测量多词写法**：`MEASURE` 是**选择子**（默认距离），但 `MEASURE DISTANCE` / `MEASURE ANGLE` /
  `MEASURE AREA` / `MEASURE POLYLINE` 这些**多词名字不存在**；真名是 `DIST` / `MEASUREANGLE` /
  `AREA` / `MEASUREPOLYLINE`（见上表）。
- **键位与别名不是一回事**：`ENTER`→`CONFIRM` 是**唯一的**键名同形别名行；`ESC`、`ZE`、`Z E`、`DI`、
  `AA`、`ANGLE`、`DISTANCE`、`ZI`、`ZO`、`RIBBON` 等**都不在** `COMMAND_ALIASES` 里。
  键位见 §3，别名全集见 §2b。

---

## 2b. 现行 AutoCAD 风格命令别名（React 宿主，2026-10-25 起）

> 本节描述**当前** `crates/cad-ui-core/src/session/commands.rs::COMMAND_ALIASES` 的真实内容
> （Slint 时期的别名表已随之删除）。命令行候选列表由宿主 IPC `cad_command_aliases` 提供，
> 前端不再维护第二份表；别名只是**候选/便捷输入**，准入与分发仍走 `cad_command` → 路由器
> 归一化（`normalize_command`/`canonical_command_name`），别名不会改变任何命令行为。

| 别名 | canonical | 别名 | canonical |
|---|---|---|---|
| `L` | `LINE` | `TR` | `TRIM` |
| `C` | `CIRCLE` | `EX` | `EXTEND` |
| `A` | `ARC` | `O` | `OFFSET` |
| `PL` | `PLINE` | `RO` | `ROTATE` |
| `REC` | `RECTANG` | `SC` | `SCALE` |
| `EL` | `ELLIPSE` | `MI` | `MIRROR` |
| `POL` | `POLYGON` | `AR` | `ARRAYRECT` |
| `PO` | `POINT` | `AL` | `ALIGN` |
| `SPL` | `SPLINE` | `S` | `STRETCH` |
| `T` | `TEXT` | `X` | `EXPLODE` |
| `M` | `MOVE` | `H` | `HATCH` |
| `CO` | `COPY` | `U` | `UNDO` |
| `CP` | `COPY` | `Z` | `ZOOM` |
| `E` | `ERASE` | `P` | `PAN` |
| `F` | `FILLET` | `OS` | `OSNAP` |
| `CHA` | `CHAMFER` | `GR` | `GRID` |
| `DLI` | `DIMLINEAR` | `OR` | `ORTHO` |
| `DAL` | `DIMALIGNED` | `DRA` | `DIMRADIUS` |
| `DDI` | `DIMDIAMETER` | `DAN` | `DIMANGULAR` |
| `DCE` | `DIMCENTER` | `MA` | `MATCHPROP` |

全词同义词（同样在 `COMMAND_ALIASES` 内）：`EXPORT`/`PRINT`→`PLOT`、`SAVEAS`/`QSAVE`→`SAVE`、
`ZOOM EXTENTS`→`FIT`、`ENTER`→`CONFIRM`、`DBA`→`DIMBASELINE`、`DCO`→`DIMCONTINUE`、
`LAYUNISO`/`LAYON`→`RESTORELAYERS`、`NEWLAYER`→`CREATELAYER`（项目同义词，非 `acad.pgp`）。
本页缩写表 40 行 + 本句 11 行 = **51** 行，与 `COMMAND_ALIASES` 的长度一致。

诚实边界：这张表是**面向已实现命令的 AutoCAD `acad.pgp` 子集**，不是 `acad.pgp` 解析器，
也**不支持用户自定义别名**；目标命令不存在的缩写一律不收录，绝不用别名凭空造出命令。
`MA`（`MATCHPROP`）在该命令实现（选择式特性匹配）后进入本表；`MATCHPROP` 本身**无新 IPC**，
裸命令名即全部输入（`CommandPayload::None`，选择在应用时读取）。`SNAP`（栅格捕捉，F9）没有别名行：
它已是**自己的**规范选择子（不是别名），与 `OSNAP`（对象捕捉）和 `GRID`（显示栅格）互不为别名。

---

## 3. 固定键位（当前 React 宿主；Slint 时期契约另附）

**键处理的真实位置（2026-10-27 更正，随文档加速器落地再次更新）**：全局 `keydown` 在
`apps/desktop-ui/src/App.tsx::onKeyDown`；文档加速器的**纯映射表**在
`apps/desktop-ui/src/commands/accelerators.ts`（`acceleratorCommand` / `acceleratorDispatch`）；
工具键在 `apps/desktop-ui/src/commands/commands.ts::toolCommandForKey`；制图功能键映射在
`apps/desktop-ui/src/cad/cad.ts::DRAFTING_ACCELERATORS`；浏览器默认键的抑制在
`apps/desktop-ui/src/defaults.ts`。旧页头引用的 `ui/app.slint` **已随 Slint 退役删除**。

**当前源码里真的接线了的全局键**（`Alt`/`Meta` 一律放行给系统）：

| 键 | 动作 | 来源 |
|---|---|---|
| `Ctrl+A` | `SELECTALL`（全选可见模型空间对象） | 加速器表 |
| `Ctrl+C` | `COPYCLIP`（**应用内**剪贴板，不是 OS 剪贴板） | 加速器表 |
| `Ctrl+X` | `CUTCLIP`（应用内剪切：一次事务/一步撤销） | 加速器表 |
| `Ctrl+V` | `PASTECLIP`（应用内粘贴：进入**插入点**工具，平移 `插入点 − 基点`） | 加速器表 |
| `Ctrl+Shift+C` | `COPYBASE`（捕获 **+ 基点**；只读，Viewer 亦可用） | 加速器表 |
| `Ctrl+Shift+X` | `CUTBASE`（剪切 **+ 基点**；先删除再提交剪贴板） | 加速器表 |
| `Ctrl+Z` | `UNDO` | 加速器表 |
| `Ctrl+Y` / `Ctrl+Shift+Z` | `REDO` | 加速器表 |
| `Ctrl+Home` | `FIT`（缩放到范围） | 加速器表 |
| `Ctrl+O` | `OPEN`（需 `canOpen`） | 加速器表 |
| `Ctrl+N` | `NEW`（需 `canNew`，否则不派发） | 加速器表 |
| `Ctrl+S` / `Ctrl+Shift+S` | `SAVE`（有损 DXF 另存；需 `canSave`） | 加速器表 |
| `Ctrl+P` | `PLOT`（矢量 SVG/PDF；需 `canPlot`） | 加速器表 |
| `Ctrl+0` | 切换 clean screen（隐藏 chrome） | `App.tsx` 直接分支 |
| `Ctrl+1` | 切换属性面板 | `App.tsx` 直接分支 |
| `Ctrl+9` | 切换命令行展开 | `App.tsx` 直接分支 |
| `F2` | 切换命令**历史**面板 | `App.tsx` 直接分支 |
| `F3` | 切换对象捕捉 `OSNAP` | `DRAFTING_ACCELERATORS` |
| `F7` | 切换显示栅格 `GRID` | `DRAFTING_ACCELERATORS` |
| `F8` | 切换正交 `ORTHO` | `DRAFTING_ACCELERATORS` |
| `F9` | 切换**栅格捕捉** `SNAP`（默认关） | `DRAFTING_ACCELERATORS` |
| `Enter` | **仅当工具有效且焦点不在输入框**：确认（发 `CONFIRM`）；否则交给输入控件/命令行 | `toolCommandForKey` |
| `Esc` | 有活动工具或夹点拖拽时发 `CANCEL`；否则关闭历史/诊断面板、让聚焦输入框失焦（**不清除选择**） | `App.tsx` + `toolCommandForKey` |

**加速器表的三条规则（`commands/accelerators.ts`，一个调用点）**

- **表是纯函数**：`acceleratorCommand(event, context)` 只做「键 → 命令名」，不含 DOM/React/dispatch；
  `acceleratorDispatch(event, context, admitted)` 再套一层**共享准入**（`shell.canCommand` ＝
  `admitCommandText`，与 ribbon/QAT/命令行**同一个**函数，含 `canOpen`/`canNew`/`canSave`/`canPlot`、
  busy/loading、`FIT` 的 `canFit && documentLabel && entityCount` 门槛）。
- **派发 ⟺ `preventDefault`**：`acceleratorDispatch` 在准入拒绝时返回 `null`，调用点既不派发也
  **不** `preventDefault`——被拒的 chord 一个键都不吞（它随后是否被 `defaults.ts` 的浏览器默认守卫
  抑制，是既有行为，与加速器表无关）。
- **声明在前、决定在后**：它挂在 `Ctrl+0` 分支之后、`defaultPrevented` 守卫**之前**，正是因为
  `defaults.ts`（`bindBrowserDefaults`）会替 webview 抢 `Ctrl+P` 的打印动作；两个 `window` 监听器的
  注册顺序不能决定 `PLOT` 能不能用。该守卫本身只拦 reload / print / 页面缩放这几条浏览器命令。
- **拒答顺序**（每个 `return null` 都在查表之前，所以被拒的按键连映射都不会发生）：① IME 组合
  （`isComposing` 或遗留 `keyCode === 229`）→ ② 焦点在文本入口（`INPUT`/`SELECT`/`TEXTAREA` 或
  `contentEditable`，原生复制/粘贴/撤销/全选保持可用）→ ③ 宿主对话框开着（`cad_status.popup`）→
  ④ `toolActive || textStep || gripDragActive`（那三种情形归 Enter/Esc）→ ⑤ 按住 `Meta`/`Alt`
  或没有 `Ctrl` → ⑥ 查表（`Ctrl+Home`；字母里带 Shift 的形式只有 `Shift+C`/`Shift+S`/`Shift+X`/
  `Shift+Z` 四个，因此 `Ctrl+Shift+V` 与 `Ctrl+Shift+Home` **不会被误降级**成粘贴/缩放到范围）。
- **Viewer 模式仍是宿主侧拒绝**：加速器只做 UI 准入，`MODE` 为 Viewer 时 work-only 命令由命令层
  显式报错——键位不是第二道模式门，也不会静默改成 no-op。

**仍然没有全局键的**：`Ctrl+Shift+V`（`PASTEBLOCK`，命令本身未实现）、`Ctrl+Shift+Home`，以及其余
未列出的 `Ctrl+<字母/数字>`——加速器表**故意**拒绝任何别的带 Shift 字母，所以它们不会被降级成
非 Shift 的兄弟命令。这些键**不被本应用拦截**，webview 默认
照旧；其中 `F5`/`Ctrl+R`/`Ctrl+Shift+R`（重载）与 `Ctrl+=`/`Ctrl+-`（页面缩放）被 `defaults.ts`
抑制为无动作。`Ctrl+P` **不**属于这一类：它归 `PLOT`，`defaults.ts` 的打印抑制只在 `PLOT` 未被
准入（无文档/无 `canPlot`）时才会落到键上。

**Slint 时期（已退役）的键位契约——历史记录，勿当作当前行为**：旧宿主用 capture/bubbling 两层处理器
（`ui/app.slint`，文件已删除），并通过 `shortcut.unsupported` 报告未实现的 chord。该实现与报告通道
**都已随 Slint 退役**：`shortcut.unsupported` 现在只存在于两个 i18n 目录里，**没有任何代码消费**。

**绘制/编辑工具键**：工具本身**没有**专用快捷键，经命令行选择器（如 `L`/`C`/`M`/`CO`/`TR`/`E`/`A`/
`EX`/`RO`/`SC`/`MI`/`O`）或 ribbon 按钮启动；命令行对 MOVE/COPY/ROTATE/SCALE/MIRROR/OFFSET 与
ERASE 等的空选择/多选拒绝都是显式反馈。

---

## 4. 明确未支持的 AutoCAD 快捷键/命令（诚实）

> 逐条审计（词表总量 / 实现·部分·未实现三档 / 缺口规模）见 `docs/autocad-command-coverage.md`；
> 本节的「未实现」清单与它一致。

- **功能键**：`F1`、`F4`、`F5`、`F6`、`F10`、`F11`、`F12` 未接线。（`F2`/`F3`/`F7`/`F8`/`F9`
  已接线，见 §3；`F5` 另被 `defaults.ts` 抑制为无动作。）**未接线的键不会“报告”**：当前 React 宿主
  **没有** `shortcut.unsupported` 的消费路径（该键只存在于两个 i18n 目录，无代码引用），所以按下是
  **静默无动作**（少数浏览器默认被 `defaults.ts` 抑制）。这是**已知的诚实缺口**，旧文档的
  “通过 `shortcut.unsupported` 报告、不会静默 no-op”是 **Slint 时期**的行为，**不是现状**。
- **Ctrl 组合键**：§3 表里的 **13 个命令 / 15 个键写法**
  （`Ctrl+A/C/X/V/Z/Y/Shift+Z/Home/O/N/S/Shift+S/P` 加上 `Ctrl+Shift+C`/`Ctrl+Shift+X`；其中 `Ctrl+Y` 与
  `Ctrl+Shift+Z` 都是 `REDO`、`Ctrl+S` 与 `Ctrl+Shift+S` 都是 `SAVE`）已接线。
  以下 AutoCAD 常用组合键**本仓库没有接线**（静默无动作，或落到浏览器/系统默认，
  **不是** `shortcut.unsupported`）：
  - 面板/对话框：`Ctrl+2`（DesignCenter）、`Ctrl+3`（工具选项板）、`Ctrl+4`（图纸集）、
    `Ctrl+6`（dbConnect）、`Ctrl+7`（标记集）、`Ctrl+8`（快速计算器）。
  - 绘图开关：`Ctrl+B`（捕捉）、`Ctrl+D`（动态 UCS/坐标）、`Ctrl+E`（等轴测平面）、
    `Ctrl+F`（对象捕捉）、`Ctrl+G`（栅格）、`Ctrl+H`（拾取样式）、`Ctrl+I`（坐标）、
    `Ctrl+J`/`Ctrl+M`（重复）、`Ctrl+L`（正交）、`Ctrl+R`（视口）、`Ctrl+T`（数字化仪）、
    `Ctrl+W`（选择循环）、`Ctrl+U`（极轴）、`Ctrl+Q`（退出）。
  - 对象/超链接：`Ctrl+K`（超链接）未接线；`Ctrl+Shift+V`（**粘贴为块 `PASTEBLOCK`**）**仍未实现**
    （`cad-db` 没有块写 API），所以加速器表**故意**拒绝它、也拒绝把它降级成 `PASTECLIP`。
    （`Ctrl+Shift+C`/`Ctrl+Shift+X` 已接线到 `COPYBASE`/`CUTBASE`，见 §3。）
  - 面板/组/约束：`Ctrl+Shift+H`（选项板）、`Ctrl+Shift+A`（组）、`Ctrl+Shift+I`（推断约束）、
    `Ctrl+Shift+L`（选择对象）、`Ctrl+Shift+P`（快速特性）、`Ctrl+Shift+W`（选择循环）。
  - `Ctrl+P` **已接线到 `PLOT`**（旧文档此处说它被打印守卫占住，已过时）：`PLOT` 未被准入时
    （无文档 / 无 `canPlot`）加速器不派发，键才落到 `defaults.ts` 的打印抑制上。
  - 未列出的 `Ctrl+<其他字母/数字>` 与所有 `Alt+` 组合**未映射**：`Alt`/`Meta` 一律放行给系统，
    其余未映射组合不拦截。
- **命令名的处理（2026-10-20 更正）**：此前本节引用 `crates/cad-ui-slint/src/command_line.rs`
  的 `UNSUPPORTED_COMMANDS` 表，**该 crate 已随 Slint→React 迁移删除**，此处据实改写。现行机制：命令名对照
  `crates/cad-ui-core/src/session/commands.rs` 的命令表（**97 个选择子**，与 `CommandId` 变体一一对应；
  P6b 后由 67 增加，2026-10-25 再加 `GRID`，`MATCHPROP` 批，**2026-10-26 再加 `JOIN`/`LAYISO`/`LAYOFF`/
  `PDMODE`/`PDSIZE`，2026-10-27 再加 `LAYERP`/`AREA OBJECT`，**2026-10-28 再加 `COPYBASE`/`CUTBASE`/
  `PASTEORIG`**——`ID` 是测量选择子、
  `LAYUNISO`/`LAYON` 是既有 `RESTORELAYERS` 的别名，都不新增 `CommandId`；`LAYERP` **已不是**别名）；
  不在表内的名字在 chrome 侧就按 `command.unsupported` 拒绝（**`command.unknown` 这个键没有消费方**，
  见 §1）；宿主侧「认识这个名字、但该路径不可用」（如 `PLOT`/`SAVE` 的受限路径）同样报告
  `command.unsupported`。两者都是显式反馈，**不静默忽略**。
- **仍未实现的 AutoCAD 名（据实）**：`POLYLINE`（旧式 heavy 2D 折线实体；我们只有 `PLINE` 创建的
  LWPOLYLINE，**不**给它造别名）、`XLINE`、`BLOCK`、`WBLOCK`、`INSERT` 的**创建**侧（块**引用**的读取/
  显示/拾取已实现，见 `docs/compatibility.md`）、`IMPORT`、`XREF`、`PUBLISH`、`DIMSTYLE`（样式**编辑**
  命令未实现；DIMSTYLE **变量子集**与 `Standard` 样式表已实现）。**已实现**：`DIMLINEAR`/`DIMALIGNED`/
  `DIMRADIUS`/`DIMDIAMETER`/`DIMANGULAR`/`DIMORDINATE`/`DIMCENTER`，以及
  **`DIMBASELINE`/`DIMCONTINUE`**（2026-10-21 **解禁**并落地，P5f：**显式拾取基准线性标注**，命令
  **不持有任何跨命令状态**；N 点 + 回车、**一条事务/一步撤销**；基线按 `DIMDLI` 逐行外推；
  **仅线性基准、不写 `DIMCONTINUEMODE`、非关联**）；以及 **`HATCH` 创建**（P6b：**先选闭合边界实体** +
  `SOLID`/用户自定义图案/预定义图案 + 样式参数；**`LINE`/`ARC` 边界边已可按端点缝合**（弧存精确 bulge，
  容差绝对 `1e-6`））。**预定义图案名已实现（2026-10-27）**：内置表**恰好三项**
  `ANSI31`/`ANSI32`/`ANSI37`，状态栏按**名字**选择（分段 chip），线上传的仍是 **1 基索引**
  （1/2/3，经 `cad_set_tool_parameter`）；表值是**我们的转录值**，**非** vendor 核实。
  **渐变创建已实现（2026-10-27）**：填充类型码 **`3`** + 四个参数（`hatchGradientKind`
  `0` Linear/`1` Spherical、`hatchGradientAngle` **度**、`hatchGradientColorA`/`B` **ACI 整数**），
  颜色**只有 ACI**（无真彩），ACI `0`（ByBlock）被拒，默认 1 红 / 5 蓝；`SPHERICAL` 在矩形/圆边界上
  会读成平色（按三角化顶点烘焙，不是径向）。**HATCH 仍未实现**：**内部点求边界**、
  **自定义（`76=2`）图案名与 `.pat` 加载**、**关联性**（其余拒绝面见 `docs/compatibility.md` 的 HATCH 行）。
- **`TEXT`/`MTEXT` 创建的两个参数（2026-10-28）**：**`textRotation`**（通道**度**，任意有限、**不 wrap**、
  默认 `0`；TEXT 与 MTEXT 都读，创建时**一次** `.to_radians()` 存弧度）与 **`mtextAttachment`**
  （闭集 **`1..=9`**，默认 `1`，**只给 MTEXT**：`1..=3` top / `4..=6` middle / `7..=9` bottom，每行左/中/右——
  `h = (code−1) % 3`、`row = (code−1)/3`，**永不产出 `TextAlignV::Baseline`**）。越界 code 与非有限旋转**按名拒绝**，
  不钳制、不折到邻居；`0` 度是合法值，默认值永不触发提交期拒绝。两个参数都经 `cad_set_tool_parameter`
  （**不新增选择子**，命令表仍 **97**）。**仍未实现**：MTEXT 的行距、分栏、背景填充、绘制方向、绘制流程内的
  MTEXT 对话框；`TEXT` 仍是**单行、在插入点**。
- **`OFFSET` 为有界子集**：支持 **LINE / CIRCLE / ARC / 直线段 LWPOLYLINE**；拾取点同时决定**侧**与
  **距离**。**ARC 与 CIRCLE 共用同一距离模型**（新半径 = `|拾取点 − 圆心|`，`center`/`normal`/`start`/
  `sweep` **逐位保留**，顺时针 sweep 仍为负，只改半径）；`LINE` 取到无限直线的垂距；折线整条同侧、
  `miter` 斜接、顶点/bulge 原样保留。**按名拒绝**：`offset.bulge_segment`（含弧段折线）、
  `offset.collapsed`（局部塌缩）、`offset.non_planar`；**`ELLIPSE` 单独按名拒绝**——椭圆的等距偏移
  **不是**椭圆，不拿近似当实现。其余几何走通用 `Unsupported`
  （`OFFSET supports LINE, CIRCLE, ARC and straight-segment LWPOLYLINE only, got …`），不创建、不部分执行。
  **没有全局自交修剪**：只在顶点处检测局部塌缩，远端偏移段之间的自交**不检查**（声明）。
- **`FILLET POLYLINE` / `CHAMFER POLYLINE` 的 v1 界限**（选择式，恰好 1 条**开放**、**直线段**、`+Z` 平面
  LWPOLYLINE，≥ 3 顶点；每个**内顶点**被改，两个端顶点不动、不造端帽、不产生新实体）：**闭合环**按名拒绝
  （`*.polyline_closed`，v1 只做开口；AutoCAD 会连闭合环一起倒角/圆角）；**任何带 bulge 的段**按名拒绝
  （`*.polyline_bulged_segment`，不悄悄拉直）；**非 `+Z` 平面**拒绝（`*.polyline_not_planar`）；`FILLET` 的
  `R ≤ 0` 拒绝（两线版 `R=0` 的移角形式没有折线等价物）；倒角 `A=B=0` 或负数拒绝；**角太大放不下**按名拒绝
  （`*.polyline_too_large`，**不**做 AutoCAD 的自动缩半径）；**不建模 `TRIMMODE`**（被修剪的源永远是这条折线
  本身，没有第二个对象）；**倒角 A/B 方向是我们的规则**（入边/出边）。全部拒绝都具名且在任何写入之前，
  Viewer 由命令层拒绝。
- **剪贴板是应用内的**，**不是**操作系统剪贴板：不与其他应用互通，也不跨文档（替换文档时清空），
  无 OS 剪贴板读写。**两个捕获形状 + 两个粘贴形状**（2026-10-28 起）：
  `COPYCLIP`/`CUTCLIP`（`Ctrl+C`/`Ctrl+X`）**不记基点**；`COPYBASE`/`CUTBASE`
  （`Ctrl+Shift+C`/`Ctrl+Shift+X`）在预选择之上**再选一个基点**并把它存进剪贴板元数据。
  **`PASTECLIP` 已改为插入点粘贴**（`Ctrl+V` 进入单点工具，平移 `插入点 − 基点`，无基点时基点为
  世界原点，复用 COPY 的同一个 `cad_db` 变换）；**要原坐标请用 `PASTEORIG`**——它就是改动前的
  `PASTECLIP`。`CUTBASE` 是「先删除成功、再提交剪贴板（含基点）」，失败的剪切不会毁掉旧剪贴板。
  **`PASTEBLOCK`（`Ctrl+Shift+V`）仍未实现**（没有块写 API），`Ctrl+Shift+V` 保持不接线。
- **导出**：`SAVE` 是**有损 DXF 另存**（`Partial`，记录每次转换/丢弃），**不是** DWG 往返保存；
  `PLOT` 为矢量 SVG/PDF 导出。**真实图纸上的导出兼容性未验证**。
- 任何未支持的能力都返回显式 `Unsupported`/`Partial` 或稳定诊断码，**绝不**把失败改写成空成功。

---

## 5. 平台与证据范围

- **桌面宿主是主验收面**：React/Tauri 宿主（Linux 原生 Wayland；X11 走 dom-canvas 载体）上的
  命令行、绘制/编辑命令与应用内剪贴板（ribbon / 右键菜单 / 命令行选择子 / §3 的 `Ctrl+C/X/V` 与
  `Ctrl+Shift+C/X` 加速器）均已接线；**键位只有 §3 列出的那些**（其余 `Ctrl+*` 未接线）。证据以源码契约 + 单测为主，
  真实窗口/像素证据见 `docs/validation-*.md`（历史轮次，不得推广）。
- `TRIM`/`EXTEND` 在桌面宿主已接线（与 Web 宿主同一命令路径）：目标支持 `LINE`/`ARC`（`TRIM` 另支持
  `CIRCLE`，且自 2026-10-27 起支持 `SPLINE` 目标——子曲线精确、**切割参数**来自采样），边界支持
  `LINE`/`ARC`/`CIRCLE`/`ELLIPSE`/`LWPOLYLINE`（含 bulge）。`INSERT` 目标与 `EXTEND` 的 `SPLINE` 目标
  仍显式 `Unsupported`；几何子集见 `docs/compatibility.md` 与 `docs/drawing-edit.md`。
- `PLOT`/`SAVE`/`NEW` 在桌面宿主可用（保存目标为 XDG portal `save_file`；`SAVE` 为有损 DXF）；
  Web/Android 宿主未接这些能力时显式 unsupported。
- **Android 本机未编译**（缺 Android 平台），`apps/app-android` 的绘制命令转发代码存在但**未编译
  验证**。
- **全部证据为合成/离屏**：不代表真实 GPU、真实窗口、真机或 vendor 兼容。`verify_ui` 的**文档**仍在
  `docs/verify-ui.md`（其顶部已标 **已退役（2026-10-16）**），但它描述的运行器 `scripts/verify-ui.sh`
  **已删除**（仓库里不存在；`scripts/check-linux-app.sh` 同样已退役），因此**没有可运行的 verify-ui
  脚本**，不得把它当作验证入口。逐轮记录见 `docs/handoff.md`。
