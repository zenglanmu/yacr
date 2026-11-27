# 实体边线显示与颜色（EDGES / EDGECOLOR）（2026-11-28）

会话级 `EDGES` 开关控制 3D body 的边线覆盖层；填充保留，独立线实体不受影响。设计见
[ADR-0104](adr/0104-body-edges-toggle.md)。

## 1. 已接线

- **会话/命令**：`DraftingExtras.body_edges` 默认 ON；`EDGES` payload-free 翻转，一 revision/
  undo 都不产生，不写库；Viewer 合法；Styles 托盘按钮 dispatch `EDGES`。
- **表示层**：`DisplaySettings.body_edges` → `RepresentationContext.body_edges`；shaded body 发
  `MeshWithoutEdgeOverlay`（保留 mesh、丢弃存储边）；wire-only body 画空；`WIREFRAME + EDGES OFF`
  画空；独立线/尺寸不隐藏。
- **状态**：`cad_status.bodyEdges`/`bodyEdgeColor` 投影当前 token（旧宿主缺字段时 UI 默认 ON）。
- **颜色**：`EDGECOLOR [DEFAULT|<r> <g> <b>]` 设置/重置会话色；带存储边线的 body 拆成
  `MeshWithoutEdgeOverlay` + 彩色 `LineSegments`；无存储边线的 body 仍用旧 `Mesh` overlay，颜色
  不生效（明确边界）。
- **边界**：只作用于 body 边线，不是 SketchUp 的全局 Edges；profiles/depth cue/endpoints 未建模。

## 2. 验证

| 命令 | 结果 |
|---|---|
| `cargo test -p cad-representation --lib --locked` | PASS（**384 passed**：去边、wire body 画空、彩色边线分片） |
| `cargo test -p cad-app --lib --locked` | PASS（**3580 passed**：EDGES/EDGECOLOR 无库写、Viewer） |
| `cargo test -p cad-ui-core --lib --locked` | PASS（**1001 passed**：EDGES/EDGECOLOR + 状态投影） |
| `cargo test -p cad-scene --lib --locked` | PASS（**93 passed**） |
| `(cd apps/desktop-ui && npm test)` | PASS（**2164 passed**：EDGES/EDGECOLOR admission + Styles 控件） |
| `cargo fmt --all -- --check` / file-size / i18n / architecture / command-docs | PASS |

**边界**：没有真实 GUI 点击/GPU/像素证据；其它载体运行时 **NOT VERIFIED**。

## 3. NOT VERIFIED / 不在范围

- 全局边线隐藏、profiles/depth cue/endpoints、材质/贴图样式、样式库;
- 真实 GUI/GPU 交互与非 Linux 载体。
