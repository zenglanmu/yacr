# 命令行与快捷键参考

> **历史文档（Slint 桌面已退役，2026-10-16）**：本文描述的是**迁移前**的 Slint 桌面/web 宿主；
> 其中 `cad-ui-slint`、`yacr-linux`、旧脚本命令均已退役（部分脚本改为非零 NOT IMPLEMENTED）。
> 当前架构与构建见 `docs/architecture.md`、`docs/react-linux-app.md`、`docs/build.md`、
> [ADR 0010](adr/0010-react-linux-host-replaces-slint.md)。下文仅作历史记录。

本文件是本程序（命令行 + 固定键位 + 绘制/编辑工具）的**单一诚实参考**。所有证据为**合成/离屏**
（软件 Vulkan lavapipe，`apps/app-linux`），**不代表真实 GPU、真实窗口或真实设备**；逐轮证据与
未完成项见 `docs/handoff.md`。支持的硬件/平台结论见 `docs/compatibility.md`。

术语：**canonical 命令**是 `crates/cad-ui-slint/src/command_completion.rs::SELECTORS` 里的大写
选择器；**别名**（aliases，受键盘快捷键开关门控）与**同义词**（synonyms，始终可用）都映射到某个
canonical 命令。命令行的解析、补全与分发集中在 `cad-ui-slint`，所有宿主共享。

---

## 1. 命令行行为

- **前缀匹配**：输入按空白折叠并转大写后，先尝试**精确**匹配 canonical 命令 / 同义词 /（门控的）
  别名；否则做**前缀**匹配。
  - **唯一前缀** → Enter 执行该命令。
  - **歧义前缀**（匹配多个）→ 命令行**保持打开**并给出候选（不猜测、不执行）。
  - **空 Enter** → 重复**上一条成功分发的**命令（CONFIRM/ENTER/CANCEL 不成为重复目标）。
  - **未知**输入 → 显式 `command.unknown` 反馈；**已识别但未实现**的 AutoCAD 命令 → 显式
    `command.unsupported` 反馈。二者都不会静默忽略。
- **补全候选**（`command-bar.slint`）：
  - 输入变化即时计算候选（最多 8 条，按字典序、去重）；候选只**填充**文本，**从不执行**命令。
  - `Tab`：填充高亮项（无高亮则第一项），**从不提交**。
  - `↑` / `↓`：移动高亮。
  - `Space`：仅当有高亮项时填充该选择器（空格本身不提交）。
  - `Return`：有高亮项时**执行该选择器**；否则提交当前输入。
  - `Esc`：关闭候选列表（在命令行内且存在候选时优先于取消当前交互）。
- **门控**：`interaction.keyboardShortcuts`（配置项）控制**短别名**（如 `L`、`C`、`M`、`ZE`）。
  关闭时短别名保持“未知”，**不会**被 canonical 前缀匹配偷偷改道到别的命令；canonical 名与
  同义词始终可用。
- 命令行**不是** payload 解析器：坐标、路径、CAD 脚本、payload 一律不从这里转发给文档命令。

---

## 2. 命令 / 别名表

canonical 命令（`work-only` 表示仅在工作模式可用，Viewer 模式由命令层显式拒绝）：

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
| `OFFSET` | 编辑（LINE / CIRCLE / **直线段折线**；含弧段折线按名拒绝） | 是 | `O` |
| `MATCHPROP` | 编辑（**选择式**特性匹配：首个选中实体为源，其余为目标；转移 `layer`/`color`/`linetype`/`lineweight`/`transparency` 五项） | 是 | `MA` |
| `JOIN` | 编辑（合并选择为**一个**实体：共线 `LINE` 或同心相续 `ARC`；一条事务、一步撤销，仅这两种几何） | 是 | — |
| `LAYOFF` | 图层（关闭选择所在图层，当前层除外；会话覆盖，不写 DWG、不升 revision） | 否 | — |
| `LAYON` | 图层（全部打开 = 清空会话覆盖） | 否 | — |
| `COPYCLIP` | 剪贴板捕获（只读） | 否 | — |
| `CUTCLIP` | 剪贴板剪切（删除并入剪贴板） | 是 | — |
| `PASTECLIP` | 剪贴板粘贴（原坐标） | 是 | — |
| `MEASURE` | 测量入口 | 是 | — |
| `MEASURE DISTANCE` | 测量 | 是 | `DI`、`DIST`、`DISTANCE` |
| `MEASURE ANGLE` | 测量 | 是 | `ANGLE` |
| `MEASURE AREA` | 测量 | 是 | `AA`、`AREA` |
| `MEASURE POLYLINE` | 测量 | 是 | — |
| `ID` | 测量（点坐标查询：单点拾取 → X/Y/Z + 空间 + 捕捉来源 + 精度；**故意跳过栅格捕捉与 `ORTHO`**） | 是 | — |
| `UNDO` | 历史 | 是 | `U` |
| `REDO` | 历史 | 是 | — |
| `CONFIRM` | 交互 | 是 | `ENTER` |
| `CANCEL` | 交互 | 否 | `ESC` |
| `SELECTALL` | 选择 | 否 | — |
| `CLEAR SELECTION` | 选择 | 否 | `DESELECT` |
| `PAN` | 导航 | 否 | `P` |
| `ZOOM EXTENTS` | 导航 | 否 | `FIT`、`ZE`、`Z E` |
| `ZOOM <factor>` | 导航（比例因子缩放，锚定视口中心） | 否 | — |
| `ZOOM ALL` | 导航（等同 `ZOOM EXTENTS`/`FIT`，新拼写） | 否 | — |
| `ZOOM IN` | 导航 | 否 | `ZI` |
| `ZOOM OUT` | 导航 | 否 | `ZO` |
| `OPEN` | 宿主 | 否 | — |
| `NEW` | 宿主 | 否 | — |
| `SAVE` | 宿主（有损 DXF 另存） | 否 | `QSAVE`、`SAVEAS` |
| `PLOT` | 宿主（矢量导出 SVG/PDF） | 否 | `EXPORT`、`PRINT` |
| `VIEW TOP/BOTTOM/FRONT/BACK/LEFT/RIGHT/ISOMETRIC` | 观察 | 否 | — |
| `VIEW MODE` | 观察 | 否 | — |
| `PROJECTION` | 观察 | 否 | — |
| `MODE` | 模式 | 否 | — |
| `TOOLS` | UI | 否 | `RIBBON` |
| `PANELS` | UI | 否 | — |
| `DIAGNOSTICS` | UI | 否 | — |
| `GRID` | 显示（**仅**栅格叠加开关，2026-10-25 起真实选择器） | 否 | — |
| `SNAP` | 显示（**栅格捕捉**开关 F9；会话状态，不写库，经 `interaction.gridSnapEnabled` 持久化） | 否 | — |
| `SNAPUNIT <value>` | 显示（栅格捕捉间距 sysvar；正十进制数，经 `interaction.snapSpacing` 持久化） | 否 | — |
| `OSNAP` | 显示（对象捕捉开关） | 否 | — |
| `ORTHO` | 显示（正交约束开关；会话状态，不写库，经 `interaction.orthoEnabled` 持久化） | 否 | — |
| `HOVERPREVIEW` | 显示（悬停预览开关） | 否 | — |

说明：
- `ZOOM <factor>` 是 AutoCAD 的**比例因子**缩放（2026-10-25 起可用）：`factor > 1` 放大，`factor < 1`
  缩小（与 `ZOOMIN`/`ZOOMOUT` 同一约定，复用同一条 `zoom_step` 路径），且缩放**锚定在视口中心**
  （中心点不动）。`factor` 的语法是**正的十进制数**：仅 ASCII 数字与至多一个两侧都有数字的小数点
  （`2`、`0.5`、`1.25`、`2.0`）。`ZOOM 0`、`ZOOM -1`、`ZOOM abc`、`ZOOM 2 3`、`.5`、`1e3` 一律
  **显式拒绝**（稳定原因写入 `cad_status.error`，相机不变）——不截断、不取默认值、不静默忽略。
  `ZOOM ALL`（新拼写）与既有的 `ZOOM EXTENTS` 同走 `FIT` 路径（`EXTENTS` 仍是别名）。命令名与
  参数之间允许一个或多个空格/制表符，行尾空格忽略，大小写不敏感；不带参数的裸 `ZOOM` 仍走既有的
  “该命令需要 payload” 显式拒绝，不会编造一个默认因子。这是**命令行参数**，不是新命令、不是新 IPC。
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
- 别名表见 `crates/cad-ui-slint/src/command_completion.rs::ALIASES`，同义词表见 `SYNONYMS`；
  已知但未实现的 AutoCAD 命令表见 `crates/cad-ui-slint/src/command_line.rs::UNSUPPORTED_COMMANDS`。
  **（以上三处均随 Slint 退役删除，现行别名表见 §2b。）**

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
`ZOOM EXTENTS`→`FIT`、`ENTER`→`CONFIRM`、`DBA`→`DIMBASELINE`、`DCO`→`DIMCONTINUE`。

诚实边界：这张表是**面向已实现命令的 AutoCAD `acad.pgp` 子集**，不是 `acad.pgp` 解析器，
也**不支持用户自定义别名**；目标命令不存在的缩写一律不收录，绝不用别名凭空造出命令。
`MA`（`MATCHPROP`）在该命令实现（选择式特性匹配）后进入本表；`MATCHPROP` 本身**无新 IPC**，
裸命令名即全部输入（`CommandPayload::None`，选择在应用时读取）。`SNAP`（栅格捕捉，F9）没有别名行：
它已是**自己的**规范选择子（不是别名），与 `OSNAP`（对象捕捉）和 `GRID`（显示栅格）互不为别名。

---

## 3. 固定键位（`ui/app.slint`）

**capture 处理器**（不消费文本编辑组合键；仅在 `application-ui` 且 `keyboard-shortcuts-enabled`
时生效；`Alt`/`Meta` 一律放行给系统）：

| 键 | 动作 |
|---|---|
| `Ctrl+O` | 打开图纸（需 `can-open`） |
| `Ctrl+N` | 新建空图纸（需 `can-new`，否则显式 unsupported） |
| `Ctrl+S` | 有损 DXF 另存为（需 `can-save`，否则显式 unsupported） |
| `Ctrl+Shift+S` | 同上（另存为） |
| `Ctrl+P` | 矢量出图/导出 SVG+PDF（需 `can-plot`，否则显式 unsupported） |
| `Ctrl+1` | 切换侧面板 |
| `Ctrl+9` | 切换命令行展开（命令栏可见时） |
| `Ctrl+0` | 切换 clean screen（隐藏 chrome） |
| `F7` | 切换**显示栅格**会话开关（`GRID`；不再只是叠加偏好） |
| `F3` | 切换对象捕捉会话开关（`OSNAP`；不是捕捉提示叠加偏好） |
| `F8` | 切换正交约束会话开关（`ORTHO`；默认关，约束下一次取点） |
| `F9` | 切换**栅格捕捉**会话开关（`SNAP`；默认关，量化下一次取点的 X/Y） |
| `F2` | 切换命令行展开（命令栏可见时） |
| `Esc` | 取消最高优先级的进行中交互/命令；无进行中交互且有选择时清除选择 |

**bubbling 处理器**（聚焦文本编辑器获得**首拒绝权**，其本地撤销/选中/剪贴板优先）：

| 键 | 动作 |
|---|---|
| `Ctrl+A` | 全选可见模型空间对象 |
| `Ctrl+C` | 应用内剪贴板**复制**（非 OS 剪贴板） |
| `Ctrl+X` | 应用内剪贴板**剪切**（一次事务/一步撤销，删除并入剪贴板） |
| `Ctrl+V` | 应用内剪贴板**粘贴**（新 id、原坐标 PASTEORIG） |
| `Ctrl+Z` | 撤销（工作模式） |
| `Ctrl+Y` / `Ctrl+Shift+Z` | 重做（工作模式） |
| `Ctrl+Home` | 缩放到范围（fit） |

**绘制/编辑工具键**：工具本身**没有**专用快捷键，经命令行选择器（如 `L`/`C`/`M`/`CO`/`TR`/`E`/`A`/
`EX`/`RO`/`SC`/`MI`/`O`）或 ribbon 按钮启动；命令行对 MOVE/COPY/ROTATE/SCALE/MIRROR/OFFSET 与
ERASE 等的空选择/多选拒绝都是显式反馈。

---

## 4. 明确未支持的 AutoCAD 快捷键/命令（诚实）

- **功能键**：`F1`、`F4`、`F5`、`F6`、`F9`、`F10`、`F11`、`F12` 未接线，按下时通过
  `shortcut.unsupported` 报告（含键名），**不会**静默 no-op。（`F2`/`F3`/`F7`/`F8` 已接线，见上表。）
- **Ctrl 组合键**：上表列出的组合键已接线。以下 AutoCAD 常用组合键**未实现**，按下时通过
  `shortcut.unsupported` 报告（含精确 chord 名），**不会**静默 no-op：
  - 面板/对话框：`Ctrl+2`（DesignCenter）、`Ctrl+3`（工具选项板）、`Ctrl+4`（图纸集）、
    `Ctrl+6`（dbConnect）、`Ctrl+7`（标记集）、`Ctrl+8`（快速计算器）。
  - 绘图开关：`Ctrl+B`（捕捉）、`Ctrl+D`（动态 UCS/坐标）、`Ctrl+E`（等轴测平面）、
    `Ctrl+F`（对象捕捉）、`Ctrl+G`（栅格）、`Ctrl+H`（拾取样式）、`Ctrl+I`（坐标）、
    `Ctrl+J`/`Ctrl+M`（重复）、`Ctrl+L`（正交）、`Ctrl+R`（视口）、`Ctrl+T`（数字化仪）、
    `Ctrl+W`（选择循环）、`Ctrl+U`（极轴）、`Ctrl+Q`（退出）。
  - 对象/超链接：`Ctrl+K`（超链接）、`Ctrl+Shift+C`（复制为块 COPYBASE）、`Ctrl+Shift+V`
    （粘贴为块）、`Ctrl+Shift+X`（剪切为块 CUTBASE）。
  - 面板/组/约束：`Ctrl+Shift+H`（选项板）、`Ctrl+Shift+A`（组）、`Ctrl+Shift+I`（推断约束）、
    `Ctrl+Shift+L`（选择对象）、`Ctrl+Shift+P`（快速特性）、`Ctrl+Shift+W`（选择循环）。
  - `Ctrl+O`（打开）与 `Ctrl+N/S/Shift+S/P` 在宿主缺少对应能力时同样报告 `shortcut.unsupported`。
  - 未列出的 `Ctrl+<其他字母/数字>` 与所有 `Alt+` 组合**未映射**：`Alt`/`Meta` 一律放行给系统，
    其余未映射组合不拦截。
- **命令名的处理（2026-10-20 更正）**：此前本节引用 `crates/cad-ui-slint/src/command_line.rs`
  的 `UNSUPPORTED_COMMANDS` 表，**该 crate 已随 Slint→React 迁移删除**，此处据实改写。现行机制：命令名对照
  `crates/cad-ui-core/src/session/commands.rs` 的命令表（**88 个选择子**，与 `CommandId` 变体一一对应；
  P6b 后由 67 增加，2026-10-25 再加 `GRID`，`MATCHPROP` 批，**2026-10-26 再加 `JOIN`/`LAYISO`/`LAYOFF`**——
  `ID` 是测量选择子、`LAYUNISO`/`LAYERP`/`LAYON` 是既有 `RESTORELAYERS` 的别名，都不新增 `CommandId`）；不在表内的名字报告
  `command.unknown`；宿主**识别但未实现**的路径（如 `PLOT`/`SAVE` 的受限路径）由宿主报告
  `command.unsupported`。两者都是显式反馈，**不静默忽略**。
- **仍未实现的 AutoCAD 名（据实）**：`POLYLINE`（我们有 `PLINE`）、`XLINE`、`BLOCK`、`WBLOCK`、
  `IMPORT`、`XREF`、`PUBLISH`、`DIMSTYLE`（样式编辑未实现；DIMSTYLE **变量子集**已实现）、
  `DIMBASELINE`/`DIMCONTINUE`（P5f **已降级**）。**已实现**：`DIMLINEAR`/`DIMALIGNED`/`DIMRADIUS`/
  `DIMDIAMETER`/`DIMANGULAR`/`DIMORDINATE`/`DIMCENTER`；以及 **`HATCH` 创建**（P6b：**先选闭合边界实体** +
  `SOLID`/用户自定义图案 + 样式参数；**`LINE`/`ARC` 边界边已可按端点缝合（弧存精确 bulge，容差绝对 `1e-6`）**，**内部点求边界、预定义图案名、渐变创建、关联性未实现**，
  详见 `docs/compatibility.md` 的 HATCH 行）。
- **`OFFSET` 为有界子集**：仅支持 LINE 与 CIRCLE；其它几何（Polyline/Arc/Ellipse/Spline/Point/
  Mesh/Text/…）显式 `Unsupported("OFFSET supports LINE and CIRCLE only")`，不创建、不部分执行。
- **`Ctrl+C/X/V` 是应用内剪贴板**，**不是**操作系统剪贴板：不与其他应用互通，也不跨文档
  （替换文档时清空）。无 OS 剪贴板读写。
- **导出**：`SAVE` 是**有损 DXF 另存**（`Partial`，记录每次转换/丢弃），**不是** DWG 往返保存；
  `PLOT` 为矢量 SVG/PDF 导出。**真实图纸上的导出兼容性未验证**。
- 任何未支持的能力都返回显式 `Unsupported`/`Partial` 或稳定诊断码，**绝不**把失败改写成空成功。

---

## 5. 平台与证据范围

- **桌面宿主是主验收面**：React/Tauri 宿主（Linux 原生 Wayland；X11 走 dom-canvas 载体）上的
  命令行、键位、绘制/编辑命令与应用内剪贴板均已接线；证据以源码契约 + 单测为主，真实窗口/像素
  证据见 `docs/validation-*.md`（历史轮次，不得推广）。
- `TRIM`/`EXTEND` 在桌面宿主已接线（与 Web 宿主同一命令路径）：目标支持 `LINE`/`ARC`（`TRIM` 另支持
  `CIRCLE`），边界支持 `LINE`/`ARC`/`CIRCLE`/`ELLIPSE`/`LWPOLYLINE`（含 bulge）。`SPLINE`/`INSERT`
  等仍显式 `Unsupported`；几何子集见 `docs/compatibility.md` 与 `docs/drawing-edit.md`。
- `PLOT`/`SAVE`/`NEW` 在桌面宿主可用（保存目标为 XDG portal `save_file`；`SAVE` 为有损 DXF）；
  Web/Android 宿主未接这些能力时显式 unsupported。
- **Android 本机未编译**（缺 Android 平台），`apps/app-android` 的绘制命令转发代码存在但**未编译
  验证**。
- **全部证据为合成/离屏**：不代表真实 GPU、真实窗口、真机或 vendor 兼容；`verify_ui` 场景见
  `docs/verify-ui.md` 与 `scripts/verify-ui.sh`，逐轮记录见 `docs/handoff.md`。
