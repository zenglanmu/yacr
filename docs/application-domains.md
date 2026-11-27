# 应用域与界面 profile

> 决策见 [ADR-0053](adr/0053-application-domains.md)；插槽/插件扩展契约见
> [`docs/domain-ui-slots.md`](domain-ui-slots.md)。本文件记录**当前实现状态**与证据边界；
> 平台/载体结论以 `docs/compatibility.md` 与 AGENTS 平台表为准。

## 当前状态（2026-11-25，阶段 1–5）

| 能力 | 状态 | 证据 |
|---|---|---|
| 文档域字段 `application_domain`（minor 13，缺省不写） | 实现 | `cad-domain` header 契约、`cad-db` snapshot 往返、`cad-store-zip` minor 选择 |
| 导入推导（dwg/dxf/dgn→cad2d，ifc/rvt→bim，skp→concept，step→mechanical，其余不声明） | 实现 | `cad-app` import publish；无真实文件运行证据 |
| 用户写入 `cad_set_application_domain { domain }` | 实现（shared sink） | 共享 gate 契约 + `cad-app` header/脏标记契约；载体运行 **NOT VERIFIED** |
| `cad_status.applicationDomain` | 实现（无文档 `null`） | `cad-session` status 投影 |
| 前端 profile（4 域，token 解析，预览 store） | 实现 | `apps/desktop-ui/test/domains.test.ts` |
| 右导航栏切换器 + `<html data-domain>` 主题切换 | 实现 | 组件 + CSS token 契约；浏览器/真机视觉效果 **NOT RUN** |
| `cad2d` 参考结构 | 实现：文件/常用/插入/注释/参数化/视图/管理/输出/附加模块；Home/Insert/Annotate/View/Manage/Output 按参考面板顺序；已实现命令已接线（View→视口/坐标、Insert→内容、Annotate→注释缩放），无实现的面板仍显式 pending | `test/autocad-structure.test.ts` + `test/fixtures/autocad-2024-drafting.json`；视觉 **NOT RUN** |
| `bim` 参考结构 | 实现（阶段 4，初步）：顶部 Revit 式域 ribbon（**文件** / 建筑 / 结构 / 插入 / 注释 / 分析 / 体量和场地 / 协作 / 视图 / 管理），**左侧**属性、项目浏览器（pending）、可见性、BIM 浏览、明细表（pending）、操作提示托盘（可隐藏）；无命令行。真实命令（通用编辑、标注、文字、导入 / 链接、实体、视图导航、管理、BIM 浏览）走统一入口；墙体 / 门窗 / 族 / 钢筋 / 房间 / 工作共享等无内核的 Revit 工具显式 pending | `test/bim-domain.test.ts` + `test/fixtures/revit-2024-bim.json`；视觉/真机 **NOT RUN** |
| `cad_set_view` / BIM 视图 | 实现（typed IPC，ADR-0059）：绝对 `eye/target/up/projection/mode` 或宿主按活动空间拟合的 `overview/plan/front/back/left/right`；BIM“视图”面板 6 个预设；真实变化记一步 `ZOOM PREVIOUS`，空图按 `view.set.empty` 拒绝 | `view_set` 7 项 + `tests_admission` 2 项 + `test/set-view.test.ts`；真实窗口/GPU **NOT VERIFIED** |
| `concept` 参考结构 | 实现（阶段 3）：顶部菜单栏（File/Edit/View/Camera/Draw/Tools/Window/Extensions/Help）、左侧 Large Tool Set、右侧默认托盘（Window → Default tray 可隐藏/恢复）；真实命令走统一入口，未实现工具显式 pending 并给出稳定理由 | `test/concept-domain.test.ts` + `test/fixtures/sketchup-2024-concept.json`；视觉/真机 **NOT RUN** |
| `concept` 操作逻辑 | 实现：Space=Select（取消活动工具）、官方单键 L/R/C/A/M/Q/S/F/T/O/H/Z/E/P/B/G、Shift+Z=FIT；状态栏 Measurements 复用宿主真实 `tool.params`，LINE 已支持输入长度/角度，RECTANG 宽/高，CIRCLE 半径，ROTATE 角度，MOVE 距离，SCALE 比例（0 为未设置，LINE 角度仅在长度为正时生效），其余无参数的工具显示真实步骤提示而不是假输入框。**P（Push/Pull）在所选是闭合平面轮廓时先把轮廓拉成新精确实体**（2026-11-27，`EXTRUDE` 同一命令），对已有实体仍拾取面移动；**Follow Me 同时选中轮廓与路径后把轮廓沿路径扫掠成闭合网格**（2026-11-27，零点工具 + 命令，无参数）；**旋转矩形**（三次点击的任意平面闭合矩形）与**扇形**（圆心+圆周点+终点角的闭合扇形，弧 24 段/圈）也已接线；**材质**（ADR-0062）由托盘的 Materials 色块面板 + `PAINT <名称>` 涂当前选择、`MATERIALS` 报告文档材质，`tables/materials.json` 随文档保存（minor 19）；**场景**（ADR-0063）由托盘 Scenes 面板（`cad_scenes` host read 列表，点名称/×/输入框 dispatch `SCENE RESTORE/DELETE/SAVE`）与 `SCENE LIST` 承担，保存相机、2D/3D、工作平面、空间与样式到 `tables/scenes.json`（minor 20）；**柔化边线**（ADR-0064）的托盘面板按角度 dispatch `SOFTENEDGES <angle>`，把 per-entity 柔化角度写进显示属性（元素行 `render_attributes.soften`，minor 21）；**阴影与雾**（ADR-0065）的托盘面板通过 `cad_display` host read 读当前记录，勾选框/数值/颜色 dispatch `SHADOWS ON|OFF|SUN <az> <elev>` 与 `FOG ON|OFF|RANGE <start> <end>|COLOR <r> <g> <b>`，写入 `tables/display.json`（minor 22），渲染器把太阳当方向光、可见 shader 做距离雾；**投射阴影 NOT IMPLEMENTED**；**Outliner**（ADR-0066）由 `cad_outliner` 只读投影分组/INSERT/模型空间实体/块成员（前序 + `depth`、≤1024 行），面板本地展开折叠、点名称走既有 `cad_select_entities`（分组一次选中全部成员，块成员行不可选），改名/重排/拖拽归组未实现。**自由手绘**（ADR-0067）由概念域工具栏/菜单的 Freehand 启动 `SKETCH` 拖拽捕获：host 2 px 采样、松开提交一条开放 `LWPOLYLINE`（一步撤销），点击未拖动/3D 视图按名拒绝且不写文档。**量角器**（ADR-0068）由概念域工具栏/菜单的 Protractor 启动 `BeginMeasurement(Protractor)`：先点顶点、再各点一条射线，结果走既有 `Angle3Points` 记录与角度格式化（只读）；guide 线创建未建模，`PROTRACTOR GUIDE` 按 `protractor.guide_not_implemented` 具名拒绝。**三维文字**（ADR-0069）由 `TEXT3D <x> <y> <z> <height> <extrusion> <font|-> <text>` 用宿主 outline 字体引擎的已填充字形网格挤出 `AcDbSubDMesh`（一次事务/一步撤销；SHX/超预算按名拒绝；无字体对话框/旋转/对齐）。**外壳**（ADR-0070）`OUTERSHELL` 复用 `UNION` 的 manifold 布尔，把所选闭合网格体合并成一个外壳（一次事务/一步撤销；开放壳/非流形/精确实体/超预算/无 modeler 按名拒绝）。**修剪**（ADR-0071）`TRIMSOLID` 以第一个选中体为目标、其余为刀具，经既有 manifold 布尔做 subtract 并只替换目标（刀具保留，一次事务/一步撤销；开放壳/精确实体/跨空间/超预算按名拒绝）。**分割**（ADR-0072）`SPLITSOLID` 以第一个选中体为目标、其余为切割体，先并集切割体再同时插入外部/内部两块（切割体保留，一次事务/一步撤销；无交集/一侧为空/开放壳/精确实体/超预算按名拒绝）。剖面、相机、照片匹配、推断/吸附锁定等仍未实现的部分仍是具名拒绝/显式 pending | `concept-domain.test.ts` + `tests_typed_dimensions` + `cad-host` press_pull 路由契约 + `docs/shortcuts.md` §3；宿主运行 **NOT VERIFIED** |
| 域/插件 UI 插槽 | 实现（客户端）：`domains/ui/` 的 `ribbon`/`menu`/`toolbar`/`tray` 槽、域注册与插件增量 patch（功能区/菜单/工具组/托盘可扩展，命令仍过共享准入）。公开插件面 `@yacr/plugin-api` 的 `api.ui.register` 已发布：声明式贡献、字面标签、`deactivate` 自动卸载、client-local 无描述符行 | `concept-domain.test.ts` + `plugin-api/ui.test.ts` + 插件包 `test/ui.test.ts`；视觉 **NOT RUN** |
| `mechanical` 参考结构 | 实现（阶段 5，初步）：SolidWorks 式域 ribbon（**文件** / 草图 / 特征 / 装配体 / 工程图 / 评估 / 视图 / 选项），**左侧** FeatureManager 设计树（读宿主 `cad_mechanical_tree` 投影）、PropertyManager、ConfigurationManager（pending）、操作提示托盘；无命令行，数值入口是状态栏真实工具参数 | `test/mechanical-domain.test.ts` + `test/fixtures/solidworks-2024-mechanical.json`；视觉/真机 **NOT RUN** |
| FeatureManager 树读取 `cad_mechanical_tree`（`mechanicalTree` host read） | 实现：按 `tables/features.json` 的求值顺序投影特征（`schema_only`，无求值器），其后按实体 id 投影 `Body3d`（`ready` / `unavailable`）；页大小上限 1024，`total` 与 `truncated` 如实报告；无文档返回空树 | `cad-host` 投影契约 + `host_read` 往返/无 seam 拒绝 + 前端解析契约；真实窗口 **NOT VERIFIED** |
| 每域主题持久化 `ui.theme.<domain>` | **未实现**（阶段 2） | — |
| `DOMAIN` 命令 / 文档属性入口 | 实现：裸 `DOMAIN` 打开切换器；`DOMAIN <id>` 有文档时写 `cad_set_application_domain`，无文档时只预览；参考名只做输入别名；未知 token 按 `domain.unknown` 具名拒绝。三个 profile 把同一行放在 **File**（共享 `ribbon/domain-entry.ts` 的 `DOMAIN_PANEL`：`cad2d` 文件页、`bim` 文件页、`concept` 文件菜单），与导航坞 app 图标共用 store | `domains.test.ts` + `commands.test.ts`；文件信息面板显示只读域行 |
| 域行（文件信息） | 实现：有文档时显示中性域名（tooltip 给参考软件名），无文档留空 | `FileInfoPalette` 契约；视觉 **NOT RUN** |
| 官方手册结构 fixture | 实现（结构转录；`bim` 的 File 页与九个 Revit 选项卡、`concept` 的九个菜单、`cad2d` 的选项卡均按参考顺序） | 同上 |
| 自有 golden 截图 | 实现（本地）：4 域 1600×1000，脚本 `scripts/ui-golden.cjs`，基线 `apps/desktop-ui/test/golden/ui/` | 两次本地运行 0.000% diff；软件 Chromium，layout-only；CI 未接线 |

## 不变式

1. **域是文档属性**：切换写 `document.json`（一个 revision、`ChangeScope.header`），不转换数据。
2. **profile 只决定表现**：命令可用性仍由文档能力 ∩ 构建能力决定；缺能力 disabled + 稳定
   reason，不隐藏、不空成功。
3. **占位不撒谎**：四个已知域（`cad2d`/`bim`/`concept`/`mechanical`）均已接线，不再有已知域
   回退；未实现工具是带稳定理由的 disabled/pending 行，不是空成功。未知 token（旧/新宿主）仍
   回退到最成熟的 `cad2d`，并在画面上方提示中给出原始 token；文档域值与数据不变。
4. **中性 token**：文件名与命令只使用 `cad2d|bim|concept|mechanical`；参考软件名只出现在
   tooltip/内部文档。
5. **无文档只预览**：欢迎页切换不写盘、不改 `NEW` 默认。

## 阶段 3 已落地（2026-11-25，concept）

- **布局**：`concept` 的顶部是真实菜单栏（复用 `RibbonMenuBar` 的键盘/定位/拒绝语义），
  左侧为 Large Tool Set 图标列，右侧为默认托盘；三者都是真实 flex 列，宿主按缩小后的
  `.native-input` 重新呈现。概念域没有命令行，数值入口是状态栏的 Measurements 框。
- **默认托盘可见性**：`Window → Default tray` 从 pending 改为真实开关
  （`SHELL_ACTIONS.domainTray`），托盘标题行的关闭按钮走同一转换；只隐藏整列，各面板仍用
  自己的标题折叠。与 SketchUp 的 Window → Default Tray 对应，状态是会话级 chrome 偏好，
  不写文档。
- **域切换入口**：`文件 → 应用域`（共享 `DOMAIN_PANEL`，`SHELL_ACTIONS.domainSwitcher`）与导航坞顶部的
  app 图标打开同一个 switcher popover；概念域没有命令行，这是 profile 内可见的切换路径。
- **结构来源**：官方 Help 的菜单/工具栏/托盘/默认单键表转录为
  `test/fixtures/sketchup-2024-concept.json`；图标全部自绘，不复制 SketchUp 资源、截图或手册正文。
- **插槽与插件**：`domains/ui/` 定义 `menu`/`toolbar`/`tray` 三个数据槽（阶段 4 增加 `ribbon`）；
  `registerDomainUi` 注册域布局，`registerPluginUi` 以 `domain: '*'` 或具体域增量扩展。
  公开插件面是 `@yacr/plugin-api` 的 `api.ui.register`（声明式贡献、字面标签、
  `deactivate` 自动卸载）；它是 **client-local**，没有描述符行，能力表不得写成 supported。
  `plugins/ui.ts` 只保留宿主侧注册入口。
- **操作逻辑**：Space=Select、官方单键表、Shift+Z=FIT；输入框/弹窗/文本步/夹点拖拽期间让出键盘。
  Measurements 只渲染宿主真实投影的 `tool.params`；没有参数时显示真实步骤提示，**不伪造**长度/角度
  输入。
- **宿主未实现、仍显式拒绝**：`SWEEP`/`SECTIONPLANE`/`CAMERA`/`MATERIALS` 等保持稳定拒绝；
  `EXTRUDE` 自 2026-11-25 起把所选闭合平面轮廓拉成新精确实体（LINE 闭环 / 闭合直线
  LWPOLYLINE / CIRCLE，+Z 或 -Z），`PRESSPULL` 自同日移动已有实体的面；二者都**不会**把
  任意 2D 线框自动识别成面。2026-11-27 起 Push/Pull 在所选**是**闭合平面轮廓时（矩形/圆等）
  先把轮廓拉成新实体——工具在起点识别这个选择并改走 `EXTRUDE` 的同一命令与事务（一次撤销），
  轮廓行保持原位；其它平面/扫描路径仍按 `extrude.not_planar` 等具名拒绝。
  2026-11-27 起 `FOLLOWME` 已接线：同时选中一条闭合直线轮廓与一条开放直线路径后按回车，把
  轮廓沿路径扫掠成闭合网格体（拐角在角平分面剪切、两端封盖、每面有 `face_sources` 身份），
  轮廓/路径保持原位，一次事务/一步撤销；圆/弧轮廓、闭合路径与反向路径按名拒绝，不是精确
  B-Rep。SketchUp 的推断锁定、场景/阴影/雾/照片匹配仍没有实现，不以按钮或空成功冒充。`concept` 的紧凑布局把同一批工具组折进现有抽屉
  （`MobileRibbonDrawer`），没有菜单栏/托盘的第二套实现；移动端视觉与真机 **NOT VERIFIED**。

## 阶段 4 已落地（2026-11-25，bim 初步）

- **布局**：`bim` 的 profile 是 `ribbon: true`、`surface: 'domain-ribbon'`、`commandLine: false`、
  `placeholder: false`。顶部 `DomainRibbonBand` 渲染域注册的 `ribbon` 槽（保留 92px 面板带、
  溢出提示与共享 variant 弹层），不再挂载 AutoCAD 功能区；命令行不渲染。**左侧** `DomainTray`
  承载属性 / 项目浏览器 / 可见性 / BIM 浏览 / 明细表 / 操作提示。
- **文件页**：第一个选项卡是带 accent 的 **文件**（Revit 的 File 页），面板内是
  `NEW/OPEN/SAVE/EXPORT/PLOT/CLOSE` 与 Options；紧随其后的单行共享面板 `domain` 是
  `应用域`（`SHELL_ACTIONS.domainSwitcher`）。文件命令是与制图 File 页、TitleBar QAT 完全相同的
  payload-free 命令（`SAVE` 恒写 `.ymodel`，其它格式走 `EXPORT`，因此没有独立的“另存为”行），
  不增加第二条派发通路。band 默认停在第一个**非 accent** 选项卡（建筑），与制图 ribbon 默认停在
  `home` 而不是 `file` 一致。
- **托盘可见性**：托盘有自己的标题行与关闭控件；`视图 → 窗口 → 显示/隐藏面板`
  （`SHELL_ACTIONS.domainTray`）与标题行按钮走同一个 store 转换。这是 chrome 偏好而非文档事实，
  与 SketchUp 的 Window → Default Tray → Hide Tray / Revit 的 View → User Interface 对应；
  会话内有效，重新加载回到默认显示。
- **结构来源**：Revit 2024 的选项卡与面板顺序转录为 `test/fixtures/revit-2024-bim.json`；只转录
  结构，不复制图标、截图或手册正文。域 ribbon 的选项卡状态是**局部表现状态**，不写
  `shell.ribbonTab`（那是 AutoCAD 表的状态）。
- **诚实边界**：墙体 / 门窗 / 楼板 / 屋顶 / 楼梯 / 梁柱支撑 / 基础 / 钢筋 / 房间 / 空间 /
  标高轴网 / 地形 / 工作集同步 / 复制监视等没有内核的 Revit 工具是 `RibbonPendingItem`
  （`bim.pending.kernel`），不是可点假按钮。项目浏览器与 Revit 式明细表没有数据源，保持
  `pending` 面板。属性、可见性（图层）、BIM 浏览、通用编辑/标注/导入/实体/视图/管理命令是真实入口。
- **托盘位置**：Revit 把属性与项目浏览器停靠左侧，因此 `bim` profile 的 `traySide` 是 `left`
  （`concept` 保持 SketchUp 的右侧）。托盘在 `.workspace__canvas` 之前渲染，`concept-tray--left`
  只翻转朝向画布的那条边，并停止为右缘导航坞预留 `--navrail-w`。**仍不声称**：没有停靠/浮动/
  拖动重排，左侧托盘也不是 Revit 的多面板独立窗口。
- **域切换入口**：`文件 → 应用域`（共享 `DOMAIN_PANEL`，`SHELL_ACTIONS.domainSwitcher`）与导航坞
  顶部的 app 图标打开同一个 switcher popover。三个 profile 都把这一行放在 File，位置与文案一致；
  BIM/concept 没有命令行（键入 `DOMAIN` 仍只属于 `cad2d`），File 菜单是它们 profile 内的可见路径。
- **紧凑布局**：`MobileRibbonDrawer` 把 BIM 的已注册面板折入现有抽屉（无第二套 ribbon），不显示
  AutoCAD 选项卡 chips；移动端视觉与真机 **NOT VERIFIED**。
- **BIM 视图**（ADR-0059）：`bim.views` 面板的 6 个真实 item（总览/平面/前/后/左/右立面）使用
  `viewPreset` 派发族 → `cad_set_view { preset }`；宿主按**活动空间**拟合，`plan` 是 2D 正交，
  其余是 3D 透视。绝对 `eye/target/up/projection/mode` 也走同一条 typed 通道（BIM UI 暂未提供
  相机对话框）。真实变化记一步 `ZOOM PREVIOUS`，空活动空间按 `view.set.empty` 拒绝。
- **未实现**：Revit 的上下文 “修改” 选项卡（选择时出现）不建模，通用编辑命令放在“建筑→修改”面板；
  视图/图纸树/视图样板、族编辑器、工作共享与 BIM 构件内核仍未实现。

## 阶段 5 已落地（2026-11-25，mechanical 初步）

- **布局**：`mechanical` 的 profile 是 `ribbon: true`、`surface: 'domain-ribbon'`、
  `commandLine: false`、`placeholder: false`、`traySide: 'left'`。顶部 `DomainRibbonBand`
  渲染域注册的 `ribbon` 槽；左侧 `DomainTray` 承载 FeatureManager / PropertyManager /
  ConfigurationManager（pending）/ 操作提示。命令行不渲染；数值入口是状态栏的工具参数控件
  （`status.tool.params`），对应参考软件的 PropertyManager 数值字段，不是第二套输入框。
- **结构**：SolidWorks 的 CommandManager 是**上下文相关**的，而本构建没有文档类型模型，因此
  `domains/mechanical/ribbon.ts` 固定一个**初步子集**（文件 / 草图 / 特征 / 装配体 / 工程图 /
  评估 / 视图 / 选项），由 `test/fixtures/solidworks-2024-mechanical.json` 锁定；它**不是**整个
  参考产品的完整转录，也不复制图标/截图/手册正文。第一个选项卡是带 accent 的 **文件**，面板内含
  `NEW/OPEN/SAVE/EXPORT/PLOT/CLOSE`、Options 与共享 `domain` 面板（应用域切换器）。
- **真实命令**：草图（LINE/RECTANG/CIRCLE/ARC/POLYGON/ELLIPSE/SPLINE/PLINE/XLINE、TRIM/EXTEND/
  OFFSET/MIRROR/MOVE/ROTATE/SCALE/COPY/ERASE）、特征（EXTRUDE、SUBTRACT/UNION/INTERSECT、
  PRESSPULL、CHAMFEREDGE、ARRAYRECT/ARRAYPOLAR、HATCH）、工程图注释（DIMLINEAR/DIMALIGNED/
  DIMRADIUS、TEXT/MTEXT、QLEADER、VPORTS）、评估（MASSPROP、DIST/AREA/MEASUREGEOM）、视图
  （标准视图、VSCURRENT、PROJECTION、GRID、ORBIT/PAN/ZOOM/FIT/VIEWBACK/SWITCH2D3D）、选项
  （Options/Parameters/Properties/File info、LAYER、DIMSTYLE、STATUS/AUDIT/PURGE）全部走
  `runItem` → 共享准入；不新增派发通路。
- **原子读取 API**：新增 `cad_mechanical_tree`（`kind: "mechanicalTree"`，无参数）host read。
  `cad-host` 的投影只读 `tables/features.json` 与 `Body3d` 行，不评估特征、不重算网格、不复制
  B-rep；特征行状态固定为 `schema_only`（P2 只有 schema，`evaluate_feature` 仍是
  `rel.not_implemented`），实体行报 `ready`/`unavailable` 与拓扑 token。结果按 1024 行截断并显式
  返回 `total`/`truncated`；无 seam 的载体按 `unsupported-runtime: mechanicalTree` 具名拒绝。
- **诚实边界**：旋转/扫描/放样/抽壳/孔/阵列特征、拉伸切除、3D 圆角边、3D 镜像、基准面/轴、
  草图约束/智能尺寸、装配体零部件与配合、爆炸视图、工程图视图、材质/外观都没有内核，保持
  `RibbonPendingItem`（`mechanical.pending.*`）；ConfigurationManager 没有创建/切换 UI，保持
  pending 面板。`MASSPROP` 仍只测闭合平面区域，不是体积/质量属性——其 hint 原文保留该边界。

## 后续阶段

- 把 `scripts/ui-golden.cjs` 接入 CI（需要固定 Chromium + 字体包，当前 build 需要网络）。
  `bim.png` / `concept.png` 基线早于已接线的 BIM/concept 界面，需重新生成（本轮 **NOT RUN**）。
- 移动端 concept 抽屉目前只折入工具组；后续再把 `menu` 槽（文件/编辑/相机等）作为抽屉头部入口，
  不新增第二套 UI。
- `ui.theme.<domain>` 每域主题设置；渲染器画布背景按域预设（当前 `theme` 仍只管 dark/light）。
- `ui.register` 已发布（client-local，无描述符行）。后续：插件自定义托盘 body（当前只有
  `pending`）、插件自带的 locale 切换（当前注册时固定字面文本）、以及移动端 concept 抽屉读取
  `uiLayoutFor` 而不是直接读 `CONCEPT_TOOLBAR`。
- `LINE`/`RECTANG`/`CIRCLE`/`MOVE`/`ROTATE`/`SCALE`/`OFFSET` 的长度/角度/比例/距离参数已接线；
  推断/吸附锁定与推拉/跟随是更大的内核工作，需要先做内核 spike。
- 其余参考面板（参数化、数据/链接、CAD 标准、动作录制器、自定义、发送、第三方应用）
  仍无对应命令实现，继续显式 pending；后续按 rewrite 指令逐条落地后再接线。
- `mechanical` 的后续：ConfigurationManager 的创建/切换界面、特征求值（ADR-0043 P2 仍只有
  schema）、装配体零部件/配合、工程图视图、材质/外观；这些在实现前保持 pending，不得用
  `cad_mechanical_tree` 的 `schema_only` 行冒充几何。

## 验证命令

```bash
cargo test -p cad-domain -p cad-db -p cad-app -p cad-ui-core --lib --locked
cargo test -p cad-host mechanical --locked
cargo test -p cad-desktop-shell --test wire_goldens the_registered_command_set_equals_the_pre_migration_set --locked
cargo check -p app-web --target wasm32-unknown-unknown --locked
(cd apps/desktop-ui && npm test)
(cd apps/desktop-ui && node --test test/bim-domain.test.ts test/concept-domain.test.ts test/mechanical-domain.test.ts)
python3 scripts/check-i18n.py && python3 scripts/check-file-size.py
```

全量门禁与真机/真实 GPU/截图对比：**NOT RUN**。
