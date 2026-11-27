# ADR-0104：实体边线显示与颜色（EDGES / EDGECOLOR）

- 状态：**已接受（决策，2026-11-28）**；**已实现（2026-11-28，会话显示 + 表示层 + Styles 托盘；无真实 GUI/GPU 证据）**
- 关联：ADR-0064（soften）、ADR-0077（Styles）、`RepresentationContext::body_edges`

## 背景

Styles 托盘此前只有 `WIREFRAME`/`SHADED` 两个 body 样式；SketchUp 的 Styles > Edges 开关
可以保留填充而隐藏边线。当前表示层把实体边线作为 body 的存储边覆盖层交付，因此这是一个
**会话显示开关**，不需要文档写入，也不应假装改变实体几何。

## 决策

- `DraftingExtras.body_edges: bool` 默认 `true`；`EDGES` 是 payload-free 会话命令（bare 翻转），
  一 revision 都不产生、不写库、不入历史、Viewer 合法。
- `render_scene::DisplaySettings.body_edges` 进入 `SceneVersion`，由 `RepresentationContext::body_edges`
  消费：
  - 有填充的 body 在 `EDGES OFF` 时发 `MeshWithoutEdgeOverlay`（保留三角形，丢弃存储边覆盖层）；
  - 只有线的 wire body 在 `EDGES OFF` 时不再发线段（边线是它唯一的几何）；
  - `VSCURRENT WIREFRAME` + `EDGES OFF` 明确画空，不回退填充；
  - 独立 `LINE`/尺寸/其它非 body 几何**不受影响**（这是本切片的明确边界）。
- `cad_status.bodyEdges` 投影该会话 token；Styles 托盘提供 “实体边线” 开关，dispatch 既有
  `EDGES`（无新参数语法）。命令报告写出“独立线实体不受影响”的边界。
- `EDGECOLOR [DEFAULT|<r> <g> <b>]` 设置/重置会话 body 边线颜色；带存储边线的 body 被拆成
  `MeshWithoutEdgeOverlay` + 一条使用该颜色的 `LineSegments` fragment。`cad_status.bodyEdgeColor`
  投影当前值（缺省 = 各 body 自身颜色）；Styles 托盘有颜色输入与“恢复实体颜色”。
- **边界**：没有存储边线的 body 仍走旧的 `Mesh` 边界 overlay，`EDGECOLOR` 不改变它（比为了
  上色而导出三角剖分对角线更诚实）；独立线实体/尺寸不参与。

## 不在本 ADR 范围

- SketchUp 的其它 Edges 开关（profiles、depth cue、endpoints、extension/jitter）、材质/贴图样式、
  样式库/缩略图/水印；非 body 的全局边线隐藏。
- 真实 GUI 交互/GPU/像素与其它载体运行时。

## 后果

- 用户可以保留 body 填充并隐藏其边线覆盖层；`VSCURRENT` 与 `EDGES` 组合有明确结果。
- 会话档位不变（无 `.ymodel` minor 提升）；独立线实体边界写入文档与命令报告。
