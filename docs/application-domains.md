# 应用域与界面 profile

> 决策见 [ADR-0053](adr/0053-application-domains.md)；插槽/插件扩展契约见
> [`docs/domain-ui-slots.md`](domain-ui-slots.md)。本文件记录**当前实现状态**与证据边界；
> 平台/载体结论以 `docs/compatibility.md` 与 AGENTS 平台表为准。

## 当前状态（2026-11-25，阶段 1–3）

| 能力 | 状态 | 证据 |
|---|---|---|
| 文档域字段 `application_domain`（minor 13，缺省不写） | 实现 | `cad-domain` header 契约、`cad-db` snapshot 往返、`cad-store-zip` minor 选择 |
| 导入推导（dwg/dxf/dgn→cad2d，ifc/rvt→bim，skp→concept，step→mechanical，其余不声明） | 实现 | `cad-app` import publish；无真实文件运行证据 |
| 用户写入 `cad_set_application_domain { domain }` | 实现（shared sink） | 共享 gate 契约 + `cad-app` header/脏标记契约；载体运行 **NOT VERIFIED** |
| `cad_status.applicationDomain` | 实现（无文档 `null`） | `cad-session` status 投影 |
| 前端 profile（4 域，token 解析，预览 store） | 实现 | `apps/desktop-ui/test/domains.test.ts` |
| 右导航栏切换器 + `<html data-domain>` 主题切换 | 实现 | 组件 + CSS token 契约；浏览器/真机视觉效果 **NOT RUN** |
| `cad2d` 参考结构 | 实现：文件/常用/插入/注释/参数化/视图/管理/输出/附加模块；Home/Insert/Annotate/View/Manage/Output 按参考面板顺序；已实现命令已接线（View→视口/坐标、Insert→内容、Annotate→注释缩放），无实现的面板仍显式 pending | `test/autocad-structure.test.ts` + `test/fixtures/autocad-2024-drafting.json`；视觉 **NOT RUN** |
| `concept` 参考结构 | 实现（阶段 3）：顶部菜单栏（File/Edit/View/Camera/Draw/Tools/Window/Extensions/Help）、左侧 Large Tool Set、右侧默认托盘；真实命令走统一入口，未实现工具显式 pending 并给出稳定理由 | `test/concept-domain.test.ts` + `test/fixtures/sketchup-2024-concept.json`；视觉/真机 **NOT RUN** |
| `concept` 操作逻辑 | 实现：Space=Select（取消活动工具）、官方单键 L/R/C/A/M/Q/S/F/T/O/H/Z/E/P/B/G、Shift+Z=FIT；状态栏 Measurements 复用宿主真实 `tool.params`，LINE 已支持输入长度/角度，RECTANG 宽/高，CIRCLE 半径，ROTATE 角度，MOVE 距离，SCALE 比例（0 为未设置，LINE 角度仅在长度为正时生效），其余无参数的工具显示真实步骤提示而不是假输入框。推拉、跟随、剖面、相机、材质、场景、推断/吸附锁定等宿主未实现的能力仍是具名拒绝/显式 pending | `concept-domain.test.ts` + `tests_typed_dimensions` + `docs/shortcuts.md` §3；宿主运行 **NOT VERIFIED** |
| 域/插件 UI 插槽 | 实现（客户端）：`domains/ui/` 的 `menu`/`toolbar`/`tray` 槽、域注册与插件增量 patch（菜单/工具组/托盘可扩展，命令仍过共享准入）。公开插件面 `@yacr/plugin-api` 的 `api.ui.register` 已发布：声明式贡献、字面标签、`deactivate` 自动卸载、client-local 无描述符行 | `concept-domain.test.ts` + `plugin-api/ui.test.ts` + 插件包 `test/ui.test.ts`；视觉 **NOT RUN** |
| BIM/SolidWorks | 显式占位：**回退到 `cad2d` 界面**（功能区+命令行），顶部提示条命名请求的域；文档域值与数据不变 | `domains.ts` profile 标记 + `renderedDomain` / `DomainSurface` 契约 |
| 每域主题持久化 `ui.theme.<domain>` | **未实现**（阶段 2） | — |
| `DOMAIN` 命令 / 文档属性入口 | 实现：裸 `DOMAIN` 打开切换器；`DOMAIN <id>` 有文档时写 `cad_set_application_domain`，无文档时只预览；参考名只做输入别名；未知 token 按 `domain.unknown` 具名拒绝 | `domains.test.ts` + `commands.test.ts`；文件信息面板显示只读域行 |
| 域行（文件信息） | 实现：有文档时显示中性域名（tooltip 给参考软件名），无文档留空 | `FileInfoPalette` 契约；视觉 **NOT RUN** |
| 官方手册结构 fixture | 实现（结构转录；文件内注明 File 为扩展 tab） | 同上 |
| 自有 golden 截图 | 实现（本地）：4 域 1600×1000，脚本 `scripts/ui-golden.cjs`，基线 `apps/desktop-ui/test/golden/ui/` | 两次本地运行 0.000% diff；软件 Chromium，layout-only；CI 未接线 |

## 不变式

1. **域是文档属性**：切换写 `document.json`（一个 revision、`ChangeScope.header`），不转换数据。
2. **profile 只决定表现**：命令可用性仍由文档能力 ∩ 构建能力决定；缺能力 disabled + 稳定
   reason，不隐藏、不空成功。
3. **占位不撒谎**：未接线的已知域（`bim`/`mechanical`）与其原生工具不冒充：界面回退到
   最成熟的 `cad2d`，并在画面上方提示“请求的域尚未接线，当前显示二维绘图界面”；文档域值与数据
   不变。未知 token 同样回退并在提示中给出原始 token。已接线域（`concept`）只把真实命令做成
   可点项，未实现工具是带稳定理由的 disabled 行，不是空成功。
4. **中性 token**：文件名与命令只使用 `cad2d|bim|concept|mechanical`；参考软件名只出现在
   tooltip/内部文档。
5. **无文档只预览**：欢迎页切换不写盘、不改 `NEW` 默认。

## 阶段 3 已落地（2026-11-25，concept）

- **布局**：`concept` 的顶部是真实菜单栏（复用 `RibbonMenuBar` 的键盘/定位/拒绝语义），
  左侧为 Large Tool Set 图标列，右侧为默认托盘；三者都是真实 flex 列，宿主按缩小后的
  `.native-input` 重新呈现。概念域没有命令行，数值入口是状态栏的 Measurements 框。
- **结构来源**：官方 Help 的菜单/工具栏/托盘/默认单键表转录为
  `test/fixtures/sketchup-2024-concept.json`；图标全部自绘，不复制 SketchUp 资源、截图或手册正文。
- **插槽与插件**：`domains/ui/` 定义 `menu`/`toolbar`/`tray` 三个数据槽；
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
  任意 2D 线框自动识别成面。（选中一个精确实体、拾取一个面、在
  Measurements 输入带符号距离），但它**只移动已有实体的面**，不会把 2D 闭合轮廓拉成新实体。
  SketchUp 的推断锁定、场景/阴影/雾/照片匹配、`FOLLOWME` 仍没有实现，不以按钮或空成功冒充。`concept` 的紧凑布局把同一批工具组折进现有抽屉
  （`MobileRibbonDrawer`），没有菜单栏/托盘的第二套实现；移动端视觉与真机 **NOT VERIFIED**。

## 后续阶段

- 把 `scripts/ui-golden.cjs` 接入 CI（需要固定 Chromium + 字体包，当前 build 需要网络），
  增加 concept 的 1600×1000 自有 golden（软件 Chromium，layout-only）。
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

## 验证命令

```bash
cargo test -p cad-domain -p cad-db -p cad-app -p cad-ui-core --lib --locked
cargo check -p app-web --target wasm32-unknown-unknown --locked
(cd apps/desktop-ui && npm test)
(cd apps/desktop-ui && node --test test/concept-domain.test.ts)
python3 scripts/check-i18n.py && python3 scripts/check-file-size.py
```

全量门禁与真机/真实 GPU/截图对比：**NOT RUN**。
