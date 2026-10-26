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
| `src/tools/` | 工具参数、开放步骤、文字输入与文字编辑 |
| `src/commands/` | 命令目录与准入、命令历史、右键菜单模型 |
| `src/ribbon/` | 功能区模型与溢出计算 |
| `src/layers/` | 图层投影、属性与标签 |
| `src/properties/` | 实体属性投影、显示模型与标签 |
| `src/popup/` | 弹窗状态、布局度量与文字编辑接线 |
| `src/options/` | 选项模型 |
| `src/components/` | 区域组件与共享控件（TSX） |
| `src/stacking-regression/` | 层序回归夹具，不是产品开关 |
| `src/App.tsx` | 单一 React 树的状态编排与接线，不按本次整理拆分 |
| `src/main.tsx`、`src/style.css` | 入口与共享样式 token |
| 根目录其他共享模块 | `icons.tsx`、`context.ts`、`catalogs.ts`、`i18n.ts`、`limits.ts`、`defaults.ts`、`focus-release.ts`、`fps.ts` |

新增纯逻辑优先放入对应功能目录；区域 UI 放入 `components/`。只有确实跨功能共享的基础模块
才放根目录。直接导入具体文件，不新增桶式导出或为整理目录增加抽象。

## 区域栈（自上而下）

`标题/QAT · 功能区 · 图纸标签 · 工作区（停靠面板 | CAD 分区 | 导航栏）
· 模型/布局标签 · 命令行 · 状态栏`

| 区域 | 文件（相对 `src/components/`） |
|---|---|
| 标题 / 快速访问工具栏 / 语言 | `TitleBar.tsx` |
| 功能区 | `Ribbon.tsx`，模型在 `src/ribbon/ribbon-model.ts` |
| 图纸标签 | `DocTabs.tsx` |
| 工作区 | `Workspace.tsx` |
| 开始页 / 诊断 | `StartPanel.tsx`、`DiagnosticsPalette.tsx` |
| 导航栏 | `NavRail.tsx` |
| 模型 / 布局标签 | `LayoutTabs.tsx` |
| 命令行 | `CommandLine.tsx` |
| 状态栏 | `StatusBar.tsx` |
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
