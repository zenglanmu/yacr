# React 桌面外壳 UI（AutoCAD 风格）

本目录是 Linux / Windows 桌面宿主共用的唯一 React UI 包：Vite + React 19 +
TypeScript/TSX，不按平台分叉。它绘制标题、功能区、停靠面板、命令区和状态栏，
不拥有 CAD 数据库。原生宿主通过下层 CAD surface 合成图纸；`dom-canvas` 路径则由
wasm CAD 会话在 WebView 的 canvas 中绘制，wasm 会话是该路径唯一的数据库权威。

```sh
npm ci
npm test        # tsc --noEmit + node --test test/*.test.ts
npm run build   # 产出已忽略的 dist/
npm run dev     # http://127.0.0.1:1420
```

相对导入保留显式 `.ts` / `.tsx` 扩展名，类型导入使用 `import type`；仅使用可剥离语法。
`test/*.ts` 由 Node 类型剥离运行，目前不参与 `tsc` 类型检查。

## 目录与职责

| 目录 / 文件 | 职责 |
|---|---|
| `src/cad/` | CAD 状态与命令接线、选择集协议、surface 模式、canvas 传输、输入、分区度量、坐标与 3D 视图 |
| `src/cad/cad.ts` | 绘图辅助开关、快捷键映射、工具步骤 / 就绪 / 预选提示与捕捉标签；不再转导出其他模块的符号 |
| `src/tools/` | 工具参数、开放步骤、文字输入与文字编辑 |
| `src/commands/` | 命令目录与准入、命令历史、右键菜单模型、命令行控制器（`command-line-model.ts` 纯规则 + `use-command-line.ts` 状态） |
| `src/ribbon/` | 功能区模型与溢出计算 |
| `src/layout/` | 宽度布局开关（`matchMedia`，断点 1024 CSS px）、移动端抽屉状态与功能区折叠、共享拒绝句（按原因键折叠） |
| `src/layers/` | 图层投影、属性与标签 |
| `src/properties/` | 实体属性投影、显示模型与标签 |
| `src/popup/` | 弹窗状态、布局度量与文字编辑接线 |
| `src/options/` | 选项模型 |
| `src/components/` | 区域组件与共享控件（TSX） |
| `src/stacking-regression/` | 层序回归夹具，不是产品开关 |
| `src/App.tsx` | 单一 React 树的状态编排与接线，不按本次整理拆分 |
| `src/main.tsx`、`src/style.css`、`src/mobile.css` | 入口与共享样式 token；`mobile.css` 是移动端增量（全部由 `[data-layout="mobile"]` 守卫） |
| 根目录其他共享模块 | `icons.tsx`、`context.ts`、`catalogs.ts`、`i18n.ts`、`limits.ts`、`defaults.ts`、`focus-release.ts`、`fps.ts` |

新增纯逻辑优先放入对应功能目录；区域 UI 放入 `components/`。只有确实跨功能共享的基础模块
才放根目录。直接导入具体文件，不新增桶式导出或为整理目录增加抽象。
状态与导入进度直接从 `cad/cad-status.ts` 导入，命令漏斗从 `cad/cad-command.ts`
导入；选择协议、属性标签和工具参数分别从各自的实现文件导入，不经 `cad/cad.ts` 中转。

## 区域栈（自上而下）

`标题/QAT · 功能区 · 图纸标签 · 工作区（停靠面板 | CAD 分区 | 导航栏）
· 模型/布局标签 · 命令行 · 状态栏`

## 紧凑布局（≤1024 CSS px，Web / Android / 桌面宿主共用同一 bundle）

宽度是唯一判据：`src/layout/use-layout-mode.ts` 订阅 `matchMedia('(max-width: 1024px)')`，
把结果同步写到 `.shell` 与 `document.documentElement` 的 `data-layout` 上。窗口实时改变宽度即切换，
不重新加载文档，也不重建 `<Workspace />`（它在两种布局里处于同一位置；停靠面板与导航栏在
`mobile.css` 里隐藏而非卸载，因此 CAD 会话不会被重挂）。

**为什么是 1024**：布局只按屏幕宽度选（产品决定），而 1024 CSS px 是最后一个“手机/平板”而不是
“桌面窗口”的宽度——手机竖屏 390、手机横屏（2400×1080 类屏幕约 914）、平板两个方向（≤1024）都拿到
紧凑 chrome，1025 px 才是第一个桌面宽度。旧的 768 让横屏手机落到桌面 chrome，图纸只剩 37% 高度。

| 区域 | 桌面 | 紧凑 |
|---|---|---|
| 顶部 | 标题/QAT · 功能区 · 图纸标签 | `.mobile-topbar`：菜单按钮（44×44，`aria-expanded`/`aria-controls`）+ 图纸名 |
| 工具 | 92px 功能区 | 抽屉式覆盖层（同一 React 树）：标签 chip 行（横向滚动，固定不滚走）+ 触摸网格（正文滚动，底部渐隐 + 细滚动条）；高度 ≤ `68dvh` 且始终给图纸留下 ≥132px；关闭方式 = 背景点击 / Escape / 执行后 |
| 中部 | 停靠面板 | CAD 分区铺满剩余高度（`100dvh` 里减掉两条 bar） |
| 底部 | 命令行 + 状态栏 | `.mobile-commandbar`：输入、发送、消息（含错误）、向上展开的候选/历史 sheet（≤45dvh） |

抽屉与命令栏都不重写规则：抽屉用 `RIBBON_TABS` + `offeredPanels`/`offeredItems` + `runItem`/
`itemEnabled`/`unavailable`；命令栏用共享控制器 `useCommandLine()` 与 `CommandField`。
移动端不提供停靠面板开关与“干净屏幕/命令行”折叠开关（没有可折叠的 chrome），
这是与 `dom-canvas` 载体同一条“不给死按钮”的规则。桌面布局（>1024 px）不变。

抽屉里的拒绝理由是**按原因键**折叠的（`layout/mobile-ribbon.ts` 的 `sharedRefusal` +
`panelRefusalKey`）：一个面板里所有工具因为同一个原因不可用时只打印一句面板级说明，不再在每个
卡片下面重复（Web 载体没有命令词表时，旧行为是 37 行几乎一样的灰字）。原因不同时仍逐个卡片保留。

### 宿主要设置的安全区变量

`.mobile-commandbar` 的底部内边距是
`calc(8px + max(env(safe-area-inset-bottom, 0px), var(--yacr-safe-bottom, 0px)))`。
`--yacr-safe-bottom` 由**宿主**设到 `document.documentElement` 上（默认 `0px`），用于
Android 强制 edge-to-edge 时系统手势条画在页面上、而 WebView 又报 `env()` 为 0 的情况：
宿主给出平台真正保留的高度，命令栏据此让开手势条，无需在 CSS 里写死设备数值。
Android 宿主（`apps/app-android` 的 `MainActivity`）读 `WindowInsets.Type.navigationBars()`
（÷ density ⇒ CSS px）后写入该变量，并在页面文档就绪后**读回确认**（`logcat` 有
`page layout:` 行），实现与实测见 `docs/android-host.md` §5。
浏览器/桌面宿主的 `env()` 与变量都是 0，行为与之前一致（8px）。

| 区域 | 文件（相对 `src/components/`） |
|---|---|
| 标题 / 快速访问工具栏 / 语言 | `TitleBar.tsx` |
| 功能区 | `Ribbon.tsx`，模型在 `src/ribbon/ribbon-model.ts` |
| 图纸标签 | `DocTabs.tsx` |
| 工作区 | `Workspace.tsx` |
| 开始页 / 诊断 | `StartPanel.tsx`、`DiagnosticsPalette.tsx` |
| 导航栏 | `NavRail.tsx` |
| 模型 / 布局标签 | `LayoutTabs.tsx` |
| 命令行 | `CommandLine.tsx`（桌面停靠区），控制器在 `src/commands/use-command-line.ts` |
| 状态栏 | `StatusBar.tsx` |
| 紧凑布局 | `MobileTopBar.tsx`、`MobileRibbonDrawer.tsx`、`MobileCommandBar.tsx`；共享输入框 `CommandField.tsx` |
| 共享控件 | `primitives.tsx`，图标在 `src/icons.tsx` |

## 合成与输入约束

原生路径为 **DOM/WebView 在上、CAD surface 在下**（ADR 0023）。唯一输入漏斗是
`.native-input` → `probe_input`；菜单与模态弹窗在同一 React 树内，不另建 Tauri 窗口。
装饰面板使用 `pointer-events: none`，交互控件使用 `pointer-events: auto`。
`dom-canvas` 路径的 canvas 传输在 `src/cad/canvas-host.ts`，WebGL2 缺失必须显式拒绝，
不得以空白 canvas 或静默回退冒充成功。

`src/cad/metrics.ts` 负责 CAD 分区矩形、去重与有效性判断；`src/cad/input.ts`
观察布局变化并上报 CSS 像素矩形与 DPR。面板停靠、功能区折叠、窗口尺寸与命令区变化
都必须保持分区矩形与宿主度量同步。

## 文案与能力

所有用户可见文案来自 `crates/cad-ui-core/i18n/{zh-CN,en}.json`（别名 `@i18n`），
键集与占位符须一致。组件不硬编码可见文案。命令 ack 只代表入队，不代表业务完成；
能力缺失必须显式拒绝或禁用，不能制造假状态。数据库、相机与渲染算法不在此包重实现。

## 测试与证据边界

`npm test` 包含类型检查、纯函数测试和源码契约断言。`readModule()` 使用精确的
`src` 相对路径；`readAppSource()` 递归收集源码，`readComponentsSource()` 只收集组件。
源码契约不是运行时 DOM 挂载测试，也不是像素、真实 GPU 或跨平台验收。
本次目录整理的实际运行结果见 `docs/handoff.md` 对应条目；历史截图不能代替本轮验证。
