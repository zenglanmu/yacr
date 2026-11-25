# ADR-0053：应用域与界面 profile

- 状态：已接受（2026-11-24）
- 关联：ADR-0010（React/Linux 宿主）、ADR-0024（载体表面）、ADR-0037（`.ymodel` 版本）、
  ADR-0041（保存/导出与来源提示）

## 背景

同一个 CAD 内核要服务四类习惯差异很大的编辑工作：二维制图（AutoCAD）、BIM（Revit）、
概念建模（SketchUp）、机械设计（SolidWorks）。当前 UI 是单一 React 树，ribbon 只有一套
AutoCAD 风格表，主题只有 `dark|light` 两档；打开 RVT/SKP 文件后仍然是 AutoCAD 界面，
既不符合来源格式，也无法让用户沿用参考软件的操作习惯。

需求同时要求：初始界面由导入格式决定；界面可手动切换；每个应用域有自己的主题；参考商业
软件的**结构**（手册），但不复制其图标/截图；源格式能力不足时先在 `.ymodel` 与命令层显式
建模，再开放功能。

## 决策

### D1 域是文档属性，profile 是会话表现
`application_domain` 写入 `document.json`，由导入来源推导；界面 profile 由该值派生，
1:1 决定 ribbon/命令行的显隐与主题。切换域是**文档编辑**（一个 revision，脏 `document.json`），
不转换任何数据。没有“预览 profile 写进文档”的第二条路径。

### D2 `.ymodel` 增加可选 `application_domain`（minor 13）
根级字符串，取值 `cad2d | bim | concept | mechanical`；缺省不写，旧档保持原字节与旧 minor。
已知 minor 内出现未知 token 按 `ymodel.invalid_document` 拒绝，不回落 `cad2d`。新增域 id 视为
schema 扩展，需要再次升 minor。缺失值由内容保守派生：有 BIM 行 → `bim`，否则 `cad2d`。

### D3 写入通路
`cad_set_application_domain { domain }` 是 **shared sink** wire 命令：桌面排队、owner tick
应用到 `cad_session`，`dom-canvas`/Web 直接应用到 wasm 会话。共享 gate 拒绝模态弹窗
（`domain.busy`）、空 token 与满队列；会话拒绝未知 token（`domain.unknown: <token>`）。
写入只改 header，`ChangeScope::header` 置位。无文档时命令拒绝；界面预览是 UI 本地状态。

### D4 主题按域派生，画布背景仍归渲染器
`<html data-domain>` 选择界面 token 块；`data-theme` 继续表示宿主 `theme`（`dark|light`），
并继续决定渲染器 model/paper 底色。三个非制图域使用浅色参考色调（BIM 冷灰蓝、SketchUp 浅灰
+ 蓝、SolidWorks 冷灰 + 红），**不**覆盖 `--canvas`。**阶段 1 不持久化每域主题**：域默认主题
即主题；`ui.theme.<domain>` 是下一阶段（需要设置键与渲染器背景一起扩展）。

### D5 命令可用性由文档能力决定，profile 只决定摆放
切域**不**启用或禁用命令。缺数据/缺实现的命令保留位置但 disabled + 稳定 reason
（`domain.*` 族；载体缺失继续 `unsupported-runtime: *`）。占位域的画面不显示 AutoCAD 功能区，
也不放可点假按钮。

### D6 单一 React 树 + 纯数据 profile
`apps/desktop-ui/src/domains/` 持有 id/profile/解析/写入与本地预览 store；不 fork 组件、
不新增第二套 UI。所有载体共享同一棵树；移动端按宽度使用同一 profile。

### D7 参考保真：结构严格、视觉近似
以 AutoCAD 2024 中文“草图与注释”为第一参考；tab/panel/命令顺序按官方手册逐条转录成
契约 fixture；颜色从手册截图采样；图标自绘。官方手册截图不入库（版权），仓库内只保留
来源清单；机器对比只使用**我们自己的** golden 截图。

### D8 阶段 1：AutoCAD 全实现，其余显式占位
`cad2d` 保持现有完整界面；BIM/SketchUp/SolidWorks 只显示主题化的占位说明卡，隐藏功能区与
命令行，不模拟参考软件的工具。占位卡说明“界面与命令尚未接线，文档数据仍以文件为准”，并
提供回到二维绘图的操作入口。

### D9 无文档时只预览
欢迎页切换 profile 只改变界面，不写入、不影响 `NEW` 的默认域（始终 `cad2d` 起步），
打开文件后以文件自己的域为准。

### D10 默认命名
显示名中性（二维绘图 / 建筑信息模型 / 概念建模 / 机械设计）；参考软件名只出现在 tooltip
与内部文档（`domain.reference.*`）。命令/文件名只用稳定中性 token。

## 实施更新（2026-11-24，阶段 2）

`cad2d` 的功能区已按 D7 重排：顶层 tab 为 文件（扩展）/常用/插入/注释/参数化/视图/管理/输出/
附加模块，取消自造“绘图/测量”顶层 tab；面板顺序转录进 `apps/desktop-ui/test/fixtures/autocad-2024-drafting.json`
并由结构契约测试锁定。**已实现命令的 UI 接线已补齐**：视图→视口（`VPORTS`/`MVIEW`/`VPCLIP`/`VPMAX`/
`VPMIN`/`VPSCALE`/`LAYVPI`）、视图→坐标（`UCSICON`）、插入→内容（`TOOLPALETTES`）、注释→注释缩放
（`SCALELISTEDIT`）不再显示 pending；参数化/插入 Data/链接、CAD 标准、动作录制器、自定义、发送、
第三方应用等无命令实现的面板继续显式 `pending`，不放可点假按钮。`DOMAIN` 命令入口已接线：裸
`DOMAIN` 打开切换器，`DOMAIN <id>` 有文档时写 `cad_set_application_domain`、无文档时只预览，未知
token 以 `domain.unknown` 具名拒绝；文件信息面板显示只读域行。自有 golden 截图（4 域、1600×1000、
软件 Chromium、两次 0.000% diff）已落地在 `scripts/ui-golden.cjs` + `apps/desktop-ui/test/golden/ui/`，
但 **CI 未接线**；每域主题持久化仍未实现（见 `docs/application-domains.md`）。

## 实施更新（2026-11-25，阶段 3：concept）

- `concept` 不再是占位：profile 为 `ribbon: true`、`surface: 'menu'`、`commandLine: false`、
  `placeholder: false`。顶部区域由 `DomainMenuBar` 渲染，不再挂载 AutoCAD ribbon；命令行不渲染。
- 新增**域/插件 UI 插槽**（`apps/desktop-ui/src/domains/ui/`）：`menu`（`RibbonTab[]`）、
  `toolbar`（`RibbonPanel[]`，左列）、`tray`（声明式 `TrayPanel[]`，右列）。域用
  `registerDomainUi` 注册；插件用 `registerPluginUi({domain:'*'|id,...})` 增量扩展菜单/工具组/
  托盘，命令仍走 `runItem` → `shell.canCommand` → `cad_command`/`runAction`。公开插件面是
  `@yacr/plugin-api` 的 `api.ui.register`（2026-11-25 发布，client-local，无描述符行）；
  `apps/desktop-ui/src/plugins/ui.ts` 只保留宿主侧注册入口，`ui.register` 不得写进已发布能力表。
- 结构按官方 Help 转录到 `apps/desktop-ui/test/fixtures/sketchup-2024-concept.json`：
  9 个顶层菜单、8 个工具栏分组、托盘面板顺序、官方默认单键表。未实现工具用 `pendingKey`
  的 disabled 行表示（`RibbonPendingItem`），不摆假按钮。
- 操作逻辑：`ConceptShortcuts` 是唯一调用点；Space=Select（活动工具时发 `CANCEL`），
  官方单键 + `Shift+Z`=FIT，输入框/弹窗/文本步/夹点期间让出键盘。状态栏为 concept 渲染
  Measurements 框，复用宿主真实 `tool.params`；无参数时显示真实步骤提示，不伪造可输入值。
- **未接线域回退**（用户指令，2026-11-25；阶段 5 起四个已知域全部接线，回退只剩未知 token）：
  未接线的已知域与不认识的 token 不再隐藏功能区与命令行，而是回退到 `cad2d` 这一最成熟界面；
  `DomainSurface` 顶部提示条命名请求的域/token，文档的 `application_domain` 与数据不变。`renderedDomain()` 是唯一回退判定，
  ribbon/命令行/`data-domain` 主题都读它；`currentDomain()` 仍保留请求值用于提示与切换器。
- **仍未实现/未验证**：推拉、跟随、剖面、位置相机/漫游/环视、材质、场景/阴影/雾/照片匹配、
  推断锁定与吸附锁定；这些保持稳定拒绝或显式 pending。真实窗口/GPU/像素、Windows/macOS/
  Android/Web 运行结论一律 **NOT VERIFIED**；concept 的紧凑抽屉只折入工具组（无第二套菜单/托盘）。
  插件 `ui.register` 已发布；仍未实现的是插件自定义托盘 body、插件自带 locale 切换，以及
  移动端抽屉读取 `uiLayoutFor`。

## 实施更新（2026-11-25，阶段 4：bim 初步）

- `bim` 不再是占位：profile 为 `ribbon: true`、`surface: 'domain-ribbon'`、`commandLine: false`、
  `placeholder: false`。顶部不再挂载 AutoCAD ribbon；`DomainRibbonBand` 渲染域注册的 `ribbon` 槽，
  右侧 `DomainTray` 渲染属性 / 项目浏览器 / 可见性 / BIM 浏览 / 明细表 / 操作提示。命令行不渲染。
- 槽模型从三个扩为四个：`ribbon`（`RibbonTab[]`，top ribbon band）、`menu`、`toolbar`、`tray`。
  `ribbon` 与 `menu` 数据形状相同但渲染器不同；`cad2d` 继续使用内建 `Ribbon`，不读取槽，
  因此插件对 `domain: '*'` 的 patch **不能**替换制图功能区。
- 结构按 Revit 2024 转录到 `apps/desktop-ui/test/fixtures/revit-2024-bim.json`：9 个选项卡
  （建筑/结构/插入/注释/分析/体量和场地/协作/视图/管理）及其面板顺序。只转录结构，图标自绘，
  不复制参考软件资源、截图或手册正文。
- 命令接线仍按 D5：通用编辑/标注/文字/导入/链接/实体/视图导航/管理/BIM 浏览等真实命令走
  `runItem` → `shell.canCommand` → `cad_command`/`runAction`；墙体、门窗、楼板、屋顶、楼梯、
  梁柱支撑、基础、钢筋、房间、空间、标高/轴网、地形、工作共享、复制/监视等无内核的 Revit 工具
  是 `RibbonPendingItem`（`bim.pending.kernel`）。项目浏览器与 Revit 式明细表没有数据源，
  保持 pending 面板；Revit 的上下文“修改”选项卡不建模。
- BIM 视图/camera placement 走 ADR-0059 的 typed `cad_set_view`（绝对位姿或宿主拟合预设），
  BIM“视图”面板 6 个 item 用新的 `viewPreset` 派发族；`dom-canvas` 3D 预设沿用既有
  `hostProfile.orbit === false` 的声明禁用，`plan` 仍可用。
- 托盘只有一个槽且在右侧；Revit 的左侧停靠是位置近似，面板顺序与含义按参考，不声称停靠一致。
  紧凑布局把已注册的 BIM 面板折入现有 `MobileRibbonDrawer`。
- **未实现/未验证**：BIM 构件内核、视图/图纸树、族编辑器、工作共享；真实窗口/GPU/像素与
  Windows/macOS/Android/Web 运行结论一律 **NOT VERIFIED**。`bim.png` golden 基线早于本界面，
  需重新生成（NOT RUN）。

## 实施更新（2026-11-25，阶段 5：mechanical 初步）

- `mechanical` 不再是占位：profile 为 `ribbon: true`、`surface: 'domain-ribbon'`、
  `commandLine: false`、`placeholder: false`、`traySide: 'left'`。顶部不再挂载 AutoCAD ribbon；
  `DomainRibbonBand` 渲染域注册的固定初步子集（文件 / 草图 / 特征 / 装配体 / 工程图 / 评估 /
  视图 / 选项），左侧 `DomainTray` 渲染 FeatureManager 设计树 / PropertyManager /
  ConfigurationManager（pending）/ 操作提示。命令行不渲染。
- SolidWorks 的 CommandManager 是上下文相关的，本构建没有文档类型模型，因此
  `test/fixtures/solidworks-2024-mechanical.json` 只锁定**本构建建模的固定子集**，不声称完整
  转录参考产品；图标自绘、文案自有。
- 新增一个**原子只读 API**：`cad_mechanical_tree`（host read `kind: "mechanicalTree"`，无参数）。
  `cad-host::HostCore::mechanical_tree` 只读 `tables/features.json`（按 `order`）与 `Body3d` 行，
  不评估特征、不重算网格、不复制 B-rep；特征行状态 `schema_only`（P2 无求值器，
  `evaluate_feature` 仍为 `rel.not_implemented`），实体行 `ready`/`unavailable`；答案按
  `MECHANICAL_TREE_MAX_ROWS = 1024` 截断并返回真实 `total`/`truncated`。无文档为空树，无 seam 的
  载体按 `unsupported-runtime: mechanicalTree` 具名拒绝。
- 真实命令仍是草图/实体/注释/评估/视图/选项里的既有 payload-free 命令，走同一 `runItem` 准入；
  旋转/扫描/放样/抽壳/孔/阵列特征、拉伸切除、3D 圆角/镜像、基准面/轴、草图约束、装配体配合、
  工程图视图、材质等无内核工具保持 `RibbonPendingItem`，不摆可点假按钮。
- **未实现/未验证**：ConfigurationManager 创建/切换界面、特征求值、装配体、工程图、材质；真实
  窗口/GPU/像素与 Windows/macOS/Android/Web 运行结论一律 **NOT VERIFIED**。

## 后果

- 新增 minor 13；minor ≤ 12 归档照常可读，带域的 13 归档被旧构建按 `unsupported_minor` 拒绝。
- `cad_status` 增加 `applicationDomain`（无文档为 `null`）；前端据此选择 profile。
- 主题目前是“域默认”，不是用户可保存的每域设置；画布背景与浅色 chrome 的不一致是有界、
  已声明的阶段限制。
- 后续阶段：其余参考面板（参数化、数据/链接、CAD 标准、动作录制器、自定义、发送、第三方应用）
  的命令实现与接线、每域停靠布局、`ui.theme.<domain>`、golden 截图 lane。

## 证据边界

阶段 1/2 只有契约测试（Rust 域读写/状态/写入 gate、前端 profile/主题/CSS/i18n、面板接线与
`DOMAIN` 解析）；没有运行截图、没有真实 GPU/像素结论，Windows/macOS/Android/Web 仅编译或契约。
`ui-golden` 本地基线已有，但 CI 与手册截图对照 **NOT RUN**（下一阶段）。
