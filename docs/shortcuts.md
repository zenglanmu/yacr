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
| `OFFSET` | 编辑（仅 LINE/CIRCLE） | 是 | `O` |
| `COPYCLIP` | 剪贴板捕获（只读） | 否 | — |
| `CUTCLIP` | 剪贴板剪切（删除并入剪贴板） | 是 | — |
| `PASTECLIP` | 剪贴板粘贴（原坐标） | 是 | — |
| `MEASURE` | 测量入口 | 是 | — |
| `MEASURE DISTANCE` | 测量 | 是 | `DI`、`DIST`、`DISTANCE` |
| `MEASURE ANGLE` | 测量 | 是 | `ANGLE` |
| `MEASURE AREA` | 测量 | 是 | `AA`、`AREA` |
| `MEASURE POLYLINE` | 测量 | 是 | — |
| `UNDO` | 历史 | 是 | `U` |
| `REDO` | 历史 | 是 | — |
| `CONFIRM` | 交互 | 是 | `ENTER` |
| `CANCEL` | 交互 | 否 | `ESC` |
| `SELECTALL` | 选择 | 否 | — |
| `CLEAR SELECTION` | 选择 | 否 | `DESELECT` |
| `PAN` | 导航 | 否 | `P` |
| `ZOOM EXTENTS` | 导航 | 否 | `FIT`、`ZE`、`Z E` |
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

说明：
- `COPYCLIP` 是**只读**捕获，Viewer 亦可用；`CUTCLIP`/`PASTECLIP` 为 work-only。
- `OPEN`/`NEW`/`SAVE`/`PLOT` 为**宿主自有**：宿主未提供能力时命令层返回显式 `Unsupported`
  （不会静默 no-op）。
- 别名表见 `crates/cad-ui-slint/src/command_completion.rs::ALIASES`，同义词表见 `SYNONYMS`；
  已知但未实现的 AutoCAD 命令表见 `crates/cad-ui-slint/src/command_line.rs::UNSUPPORTED_COMMANDS`。

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
| `F7` | 切换网格叠加 |
| `F3` | 切换捕捉提示标记叠加（并非 running-OSNAP 开关） |
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

- **功能键**：`F1`、`F4`、`F5`、`F6`、`F8`、`F9`、`F10`、`F11`、`F12` 未接线，按下时通过
  `shortcut.unsupported` 报告（含键名），**不会**静默 no-op。（`F2`/`F3`/`F7` 已接线，见上表。）
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
  `crates/cad-ui-core/src/session/commands.rs` 的命令表（**67 个选择子**）；不在表内的名字报告
  `command.unknown`；宿主**识别但未实现**的路径（如 `PLOT`/`SAVE` 的受限路径）由宿主报告
  `command.unsupported`。两者都是显式反馈，**不静默忽略**。
- **仍未实现的 AutoCAD 名（据实）**：`HATCH` **创建**（P6b 待实施；**实体/导入/导出已实现**，见
  `docs/compatibility.md` 的 HATCH 行）、`POLYLINE`（我们有 `PLINE`）、`XLINE`、`BLOCK`、`WBLOCK`、
  `IMPORT`、`XREF`、`PUBLISH`、`DIMSTYLE`（样式编辑未实现；DIMSTYLE **变量子集**已实现）、
  `DIMBASELINE`/`DIMCONTINUE`（P5f **已降级**）。**已实现**：`DIMLINEAR`/`DIMALIGNED`/`DIMRADIUS`/
  `DIMDIAMETER`/`DIMANGULAR`/`DIMORDINATE`/`DIMCENTER`。
- **`OFFSET` 为有界子集**：仅支持 LINE 与 CIRCLE；其它几何（Polyline/Arc/Ellipse/Spline/Point/
  Mesh/Text/…）显式 `Unsupported("OFFSET supports LINE and CIRCLE only")`，不创建、不部分执行。
- **`Ctrl+C/X/V` 是应用内剪贴板**，**不是**操作系统剪贴板：不与其他应用互通，也不跨文档
  （替换文档时清空）。无 OS 剪贴板读写。
- **导出**：`SAVE` 是**有损 DXF 另存**（`Partial`，记录每次转换/丢弃），**不是** DWG 往返保存；
  `PLOT` 为矢量 SVG/PDF 导出。**真实图纸上的导出兼容性未验证**。
- 任何未支持的能力都返回显式 `Unsupported`/`Partial` 或稳定诊断码，**绝不**把失败改写成空成功。

---

## 5. 平台与证据范围

- **桌面宿主（`app-linux`，覆盖 Linux/Windows/macOS）是主验收面**：命令行、键位、绘制/编辑命令、
  应用内剪贴板在离屏 Slint + 软件 Vulkan（lavapipe）下经合成场景与源码契约验证。
- `EXTEND` **仅 Web 宿主接线**（`apps/app-web`）；Linux 宿主的取点尚未接线，与 `TRIM` 的
  `linux.trim` 缺口一致，返回显式 `Unsupported`（`linux.extend`）。
- `PLOT`/`SAVE`/`NEW` 在桌面宿主可用；Web/Android 宿主未接这些能力时显式 unsupported。
- **Android 本机未编译**（缺 Android 平台），`apps/app-android` 的绘制命令转发代码存在但**未编译
  验证**。
- **全部证据为合成/离屏**：不代表真实 GPU、真实窗口、真机或 vendor 兼容；`verify_ui` 场景见
  `docs/verify-ui.md` 与 `scripts/verify-ui.sh`，逐轮记录见 `docs/handoff.md`。
