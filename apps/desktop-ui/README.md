# React 桌面外壳 UI（AutoCAD 风格）

本目录是**桌面宿主共用的唯一 React UI 包**（当前消费方是 `app-linux`；后续桌面宿主复用同一棵
UI 树，不按平台分叉）：Vite + React 19（纯 JS/JSX）。它绘制的是**宿主外壳**（标题/QAT、功能区、
图纸标签、命令区、状态栏），不是 CAD 图纸本身——图纸由 Rust 宿主以原生 surface 合成在中间分区。

```sh
npm ci
npm test        # node:test，纯合成契约
npm run build   # 产出已忽略的 dist/
npm run dev     # http://127.0.0.1:1420
```

## 区域栈（自上而下）

`标题/QAT · 功能区（选项卡→面板） · 图纸标签 · 工作区（左侧停靠面板 | CAD 分区 | 右侧导航栏）
· 模型/布局标签 · 命令行 · 状态栏`

一个区域一个组件：

| 区域 | 文件 |
|---|---|
| 标题 / 快速访问工具栏 / 语言 | `src/components/TitleBar.jsx` |
| 功能区（选项卡、面板、面板底部标题、面板设置箭头、内联下拉） | `src/components/Ribbon.jsx` + `src/ribbon-model.mjs` |
| 图纸标签（`Start` / 打开图纸） | `src/components/DocTabs.jsx` |
| 工作区（左侧面板容器 + CAD 分区 + 导航栏） | `src/components/Workspace.jsx` |
| 开始页 / 诊断面板 | `src/components/StartPanel.jsx`、`src/components/DiagnosticsPalette.jsx` |
| 导航栏 | `src/components/NavRail.jsx` |
| 模型 / 布局标签 | `src/components/LayoutTabs.jsx` |
| 命令行（补全 + 历史，F2） | `src/components/CommandLine.jsx` |
| 状态栏（坐标、绘图辅助、视图开关） | `src/components/StatusBar.jsx` |
| 共享控件（Button / IconButton / RibbonButton）、图标 | `src/components/primitives.jsx`、`src/icons.jsx` |

状态编排、输入轨道、命令漏斗、快捷键都在 `src/App.jsx`；纯逻辑拆到
`src/i18n.mjs`（查找/回退/占位符）、`src/commands.mjs`（命令目录、补全、准入）、
`src/metrics.mjs`（CAD 分区矩形）。颜色与间距是 `src/style.css` 顶部的 token。

## 合成约束（重要）

宿主把原生 CAD surface 合成在中间分区，位于 WebView **之上**。因此：

- `.native-input` 不绘制任何内容、不包含子节点，也不允许跨该分区的 DOM 浮层；
- 导航按钮停靠在画布边缘（右侧导航栏），下拉/补全/历史都在 DOM 区域内**就地展开**
  （功能区下拉会撑高功能区，命令补全出现在输入框下方），从而不会压到画布；
- 左侧面板（开始页、诊断）是停靠的，展开时会把分区**挤窄**，而不是浮在画面上。

## CAD 分区矩形上报

`probe_input` 的 `resize` 字段上报 `.native-input.getBoundingClientRect()`（CSS px）+ DPR，
序号单调递增。每次布局变化都会重新上报：窗口 resize、DPR 变化、功能区展开/折叠、面板停靠、
命令行历史展开、清屏切换。上报的矩形与宿主 surface metrics 保持一致，滚轮缩放才会锚定光标。
`src/metrics.mjs` 负责取矩形、去重与有效性判断，`src/input.mjs` 的绑定观察所有 `.app-region`
区域。

## IPC（本轮未新增）

前端只用现有四个命令：`cad_status`（严格校验状态）、`cad_command`（`OPEN`/`FIT`，`accepted`
只代表入队）、`probe_input`（输入 + 分区矩形）、`probe_status`（探针）。前端**不**实现相机、
图纸渲染、文件选择或数据库。未接线的命令一律禁用并带明确说明（`command.unsupported`、
`command.badge.unwired`、`status.toggle_unsupported`），不伪造可用。

## 文案

所有用户可见文案来自 `crates/cad-ui-core/i18n/{zh-CN,en}.json`（Vite 别名 `@i18n`），键集与
占位符必须两侧一致；组件里不写死可见文案。缺失键回退到默认语言再回退到键名本身。

## 证据（诚实）

本轮 `npm run build` exit 0，`npm test` 43/43 通过（纯合成契约，无跳过）。布局与交互状态另用
**无头 Chromium（Playwright 自带 chromium，`--headless=new`，无 GPU）+ 打桩的 Tauri IPC**
渲染已构建产物做了截图复核（开始页、打开图纸、功能区下拉、命令补全、F2 历史、诊断面板、
清屏、错误/加载、窄窗口）。这只是 DOM 布局复核：**不**证明 GTK/X11 原生合成、原生 CAD surface
出现位置、真实 GPU 渲染或真机窗口焦点；这些仍需由主协调者在真实环境验收。
