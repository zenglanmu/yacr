# ADR-0077：样式面板（Styles：VSCURRENT token + 主题）

- 状态：**已接受（决策，2026-11-27）**；**已实现（2026-11-27，真实 token 行 + 当前值高亮；无真实 GUI/GPU 证据）**
- 关联：`VSCURRENT`（早期 extras 批次）、ADR-0076（面板动作模式）、场景样式快照

## 背景

概念域托盘的 Styles 面板此前只有界面主题切换（`ThemeToggle`），不展示也不切换任何真实
样式，与 SketchUp 的 Styles 面板差距没有被说明。仓库里实际上已有**一个具名样式模型**：
`VSCURRENT` 的会话级 3D 体显示 token（`WIREFRAME` / `SHADED`），它不影响几何、不写文档。

## 决策

### D1 暴露两个真实 token

- `cad_status` 增加可选字段 `visualStyle`（`WIREFRAME`/`SHADED`），由会话的真实
  `view_extras.visual_style` 投影；无状态快照省略该字段（不是伪造默认）。
- Styles 面板列出这两个 token（本地化标签），点击 dispatch 既有 `VSCURRENT <token>`；当前
  token 用 `aria-pressed` 与“当前”标注高亮。
- 其它 SketchUp 样式名（`REALISTIC`/`CONCEPTUAL`/`XRAY` 等）**不出现**在面板里，命令层继续按
  `vscurrent.style_not_implemented` 具名拒绝；面板绝不会把未知样式映射到这两个 token。

### D2 主题与样式分开

界面主题（`ThemeToggle`）保留，但文案明确它是宿主外观，不是 SketchUp Style；面板提示同时列出
未建模的样式语义（材质/贴图样式、边/面开关、样式库/缩略图/水印、背景/天空等）。

### D3 UI 接线

`domains/concept/styles.ts` 冻结两个 token、派发线与标签键；Styles body 只渲染这两个按钮 +
主题开关。`visualStyle` 是可选 wire 字段（旧宿主省略时两行都可用，只是没有高亮）。

## 不在本 ADR 范围

- 基于材质/贴图的样式、边/面显示开关、轮廓线/深度提示/端点/背景/天空；
- 样式库、保存/加载命名样式、缩略图、水印；
- 把样式变成文档字段（`VSCURRENT` 仍是会话状态，场景快照会保存它）；
- 真实 GUI/GPU 交互与其它载体运行时。

## 后果

- 新增一个可选 status 字段、一个前端 token 模块与两个面板按钮；**无新命令、无格式档位变化**。
- Styles 仍是 `partial`（未建模的样式语义明确列出），但不再是“只有主题切换”的空壳。
