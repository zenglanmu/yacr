# OpenCADStudio UI 层功能需求反推 —— 总目录

## 本项目使用边界（2026-10-04 用户确认）

OpenCADStudio 仅作为**功能规格与 UI 交互参考**，不是源码、算法、shader 或依赖采用来源。
本目录的上游路径、符号及命令名仅保留调查出处，不要求复制、移植或复用对应实现。
本项目使用 Slint 与自有数据库/命令/渲染架构独立实现，以 `CAD_IMPLEMENTATION_SPEC.md`
为唯一需求权威；931 个参考条目不是全部必须交付的功能，也不表示本项目已支持。
原始图元编辑、DWG 回写、参数化建模等仍按规范的非目标/扩展边界处理。
纳入范围的条目须映射到本项目功能编号、命令及验收测试；未支持能力须显式报告。

本目录由 `src/ui/` 与 `src/modules/` 的 UI 层源码反推得到，逐项列出用户可见的每一项功能。每篇文档内的条目均包含固定字段：**功能简介 / UI 入口 / 样式 / 触发命令 / 实现位置（源码文件::符号）/ 备注**。

反推方法：以各 `src/modules/*/mod.rs` 的 `ribbon_groups()` 为带状界面权威布局，以 `src/ui/` 下的窗口、面板、弹出层、状态栏、命令行、属性面板为界面容器，以 `src/command.rs`、`src/app/command_driver/`、`src/app/commands/` 为命令与交互来源。所有路径、命令名、控件标签均取自源码。

---

## 文档分册

| 编号 | 文件 | 覆盖范围 | 条目数 |
| --- | --- | --- | --- |
| 02 | [02-ribbon-draw.md](02-ribbon-draw.md) | Ribbon「Draw」选项卡：Draw / Modify / Annotation / Layers / Block / Properties / Groups / Clipboard / Measure 面板 | 52 |
| 03 | [03-ribbon-annotate.md](03-ribbon-annotate.md) | Ribbon「Annotate」选项卡：文字、尺寸标注、中心线、引线、表格、标记、注释缩放 | 29 |
| 04 | [04-ribbon-insert.md](04-ribbon-insert.md) | Ribbon「Insert」选项卡：外部参照、底图、点云、块、属性、导入、内容浏览器及所辖对话框 | 48 |
| 05 | [05-ribbon-view.md](05-ribbon-view.md) | Ribbon「View」选项卡：视口工具、导航、模型视口、视觉样式、投影、预设视图、调色板、界面、打印 | 33 |
| 06 | [06-ribbon-model.md](06-ribbon-model.md) | Ribbon「Model」选项卡：实体图元、布尔运算、边编辑，及未上带的三维命令 | 32 |
| 07 | [07-ribbon-parametric.md](07-ribbon-parametric.md) | Ribbon「Parametric」选项卡：几何约束、尺寸约束、管理、约束条、参数管理器 | 56 |
| 08 | [08-ribbon-manage.md](08-ribbon-manage.md) | Ribbon「Manage」选项卡：自定义、清理、应用，并逐页列出 Options 对话框 | 28 |
| 09 | [09-paper-space-and-side-toolbar.md](09-paper-space-and-side-toolbar.md) | 图纸空间工具、右侧竖直工具栏、XREF/底图/点云上下文工具、打印对话框、Dock 框架 | 72 |
| 10 | [10-statusbar-and-commandline.md](10-statusbar-and-commandline.md) | 状态栏每个控件、状态栏自定义与右键菜单、命令行、各状态弹出层 | 104 |
| 11 | [11-properties-and-docks.md](11-properties-and-docks.md) | 属性面板、图层管理器/状态管理器/翻译器、停靠系统、调色板入口 | 108 |
| 12 | [12-context-menus-and-popups.md](12-context-menus-and-popups.md) | 各类右键菜单、弹出浮层、模态框清单、宽菜单、只读提示、节点图 | 43 |
| 13 | [13-panels-styles-editors.md](13-panels-styles-editors.md) | Ribbon 组合控件、面板折叠、颜色下拉、各样式管理器窗口、块对话框、原位文字编辑器、查找替换 | 81 |
| 14 | [14-file-and-document-ui.md](14-file-and-document-ui.md) | 文件标签栏、文件命令、打开/保存对话框、保存版本、最近文件、启动页、撤销重做、恢复/更新 | 81 |
| 15 | [15-command-input-and-selection.md](15-command-input-and-selection.md) | 命令交互、动态输入、对象捕捉与追踪、选择方式与夹点、鼠标手势、快捷键、别名、自动补全 | 124 |
| 16 | [16-plugin-and-ui-infrastructure.md](16-plugin-and-ui-infrastructure.md) | 插件管理器、插件可用的 UI 元素、图标系统、主题系统、无障碍、全局窗口框架 | 40 |

**合计：931 个功能条目。**

---

## 带状界面（Ribbon）总体结构

- 选项卡顺序（`src/modules/registry.rs::all_modules()`）：**Draw → Parametric → Model → Insert → Annotate → View → Manage**，外加仅图纸空间可见的 **Layout** 上下文模块（`src/modules/layout/mod.rs`）。
- 三维按钮规格定义于 `src/ui/ribbon/widgets.rs`，统一以 `src/ui/mod.rs` 的 `ROW_H = 26px` 为基准缩放：
  - `LargeTool` / `LargeDropdown`：占满 3 行，图标 + 文字标签（下拉带 ▾）。
  - `Tool` / `Dropdown`：占 1 行，仅图标。
  - `LabeledTool` / `LabeledDropdown`：1 行高但带文字标签。
  - `ToolGrid`：多列小图标按钮网格。
  - `StyleComboGroup` / `LayerComboGroup` / `PropertiesGroup`：下拉 + 小按钮行的组合控件。
- 面板密度支持折叠（`src/ui/ribbon/collapse.rs`），折叠后以飞出层呈现，并记忆每个面板最后使用的工具。

## 主题与视觉

- 内置主题对（`src/ui/style/fusion_theme.rs`）：
  - **Fusion Black** —— 近黑界面 `#1A1A1A` + 白色画布 `#FAFAFA` + 蓝色主色 `#0696D7`。
  - **Fusion White** —— 浅色界面 `#FAFAFA` + 近黑文字 `#1A1A1A` + 主色 `#0277BD`。
- 所有内置主题须满足 WCAG AA 文本对比度（`src/ui/theme_accessibility_tests.rs`、`src/ui/style/common.rs`）。

## 作者与维护说明

- 本目录为**功能规格参考稿**，用于核查用户可见行为及调查出处，不作为源码移植清单。
- 若源码发生变更，请以对应条目中的「实现位置」路径为准重新核对；`src/modules/*/mod.rs` 的 `ribbon_groups()` 是带状界面的单一事实来源。
- 未覆盖：`src/ui/icons.rs` 之外的纯资源、`src/ui/theme_accessibility_tests.rs` 的断言细节已在第 16 篇归纳；几何/内核算法不在 UI 需求范围。
