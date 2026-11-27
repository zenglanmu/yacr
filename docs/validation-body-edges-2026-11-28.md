# 实体边线显示开关（EDGES）（2026-11-28）

会话级 `EDGES` 开关控制 3D body 的边线覆盖层；填充保留，独立线实体不受影响。设计见
[ADR-0104](adr/0104-body-edges-toggle.md)。

## 1. 已接线

- **会话/命令**：`DraftingExtras.body_edges` 默认 ON；`EDGES` payload-free 翻转，一 revision/
  undo 都不产生，不写库；Viewer 合法；Styles 托盘按钮 dispatch `EDGES`。
- **表示层**：`DisplaySettings.body_edges` → `RepresentationContext.body_edges`；shaded body 发
  `MeshWithoutEdgeOverlay`（保留 mesh、丢弃存储边）；wire-only body 画空；`WIREFRAME + EDGES OFF`
  画空；独立线/尺寸不隐藏。
- **状态**：`cad_status.bodyEdges` 投影当前 token（旧宿主缺字段时 UI 默认 ON）。
- **边界**：只隐藏 body 边线，不是 SketchUp 的全局 Edges 开关；profiles/depth cue/endpoints 未建模。

## 2. 验证

| 命令 | 结果 |
|---|---|
| `cargo test -p cad-representation --lib --locked` | PASS（**383 passed**：shaded 去边、wire body 画空） |
| `cargo test -p cad-app --lib --locked` | PASS（**3579 passed**：EDGES 翻转/无库写/Viewer） |
| `cargo test -p cad-ui-core --lib --locked` | PASS（**1000 passed**：EDGES selector + bodyEdges 投影） |
| `cargo test -p cad-scene --lib --locked` | PASS（**93 passed**） |
| `(cd apps/desktop-ui && npm test)` | PASS（**2163 passed**：EDGES admission + Styles 控件） |
| `cargo fmt --all -- --check` / file-size / i18n / architecture / command-docs | PASS |

**边界**：没有真实 GUI 点击/GPU/像素证据；其它载体运行时 **NOT VERIFIED**。

## 3. NOT VERIFIED / 不在范围

- 全局边线隐藏、profiles/depth cue/endpoints、材质/贴图样式、样式库;
- 真实 GUI/GPU 交互与非 Linux 载体。
