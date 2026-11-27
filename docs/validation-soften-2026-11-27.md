# 柔化边线（Soften Edges，`.ymodel` minor 21）（2026-11-27）

SketchUp 的 Soften Edges 第一次有了真实模型：每个三维体可带一个柔化角度，渲染时隐藏相邻面夹角
≤ 阈值的显示边，`SOFTENEDGES [<angle>]` 写入该角度，托盘面板提供角度输入；`.ymodel` 以
**minor 21** 保存并在重开后保持。设计见
[`docs/adr/0064-soften-edges.md`](adr/0064-soften-edges.md)。

## 行为

- **模型**：`EntityRenderAttributes::soften: Option<f64>`（度，`0..=180`），per-entity 显示属性，
  与颜色/透明度/线宽同一显示状态路径。元素行**只在设置时**写 `render_attributes.soften`。
- **命令**：`SOFTENEDGES [<angle>]`（typed argument）。裸命令默认 **20°**；`SOFTENEDGES 0`
  只隐藏完全共面的边。只接受当前选择的三维体（整实体、顶层）；一次写入 = 一次 revision = 一步
  撤销（patch 携带 before/after 显示状态）；同一角度重复应用是 `soften.unchanged`。
- **拒绝**：`soften.angle_invalid`（非有限或越界）、`soften.selection_required`、
  `soften.not_addressable`（块成员/子元素）、`soften.not_a_body`、`soften.target_missing`、
  `soften.unchanged`；Viewer 模式 `PermissionDenied`。
- **渲染**：`cad-representation` 只在存储边与显示三角形的无向边**精确匹配**、且恰有两个相邻
  三角形、法向夹角 ≤ 阈值时隐藏该段；边界边/非流形边/无法匹配的段保持可见。输出
  `MeshWithStoredEdges` 带 `derive_when_empty: false`，全部柔化时只画填充、不回退到三角网线框；
  `VSCURRENT WIREFRAME` 同样过滤（全柔化时线框为空）。
- **格式**：`FORMAT_MINOR` 20 → 21；无柔化文档保持原档位；20 及以下照常打开；解码拒绝越界角度。
- **面板**：概念域托盘的 Soften Edges 面板是真面板：角度输入（本地校验 0..=180）+ 应用按钮
  dispatch `SOFTENEDGES <angle>`。

## 命令与结果

| 命令 | 结果 |
|---|---|
| `cargo test -p cad-app --lib --locked` | PASS（**3483 passed, 3 ignored**；含 7 个 soften 契约：裸命令默认 20°、撤销/重做、typed 角度、同角度 no-op、角度/空选择/非三维体/子元素拒绝、Viewer 拒绝、快照往返） |
| `cargo test -p cad-representation --locked` | PASS（**378 passed**；含 5 个 soften 契约：共面对角线在 0°/20°/30° 隐藏、90° 脊线在 30° 保留 / 120° 隐藏且不回退、无法匹配的段保持、wireframe 过滤、未柔化保持 legacy 片段） |
| `cargo test -p cad-db --locked` | PASS（**516 passed**；`render_attributes` 往返含 `soften`，未设置时不写键） |
| `cargo test -p cad-store-zip --locked` | PASS（**49 + … passed**，含 `soften_minor` **4 passed**：minor 21、无柔化普通文档 minor 4、minor 20 仍可开、超档拒绝） |
| `cargo test -p cad-ui-core --locked` | PASS（**953 passed, 1 ignored**；`SOFTENEDGES` 语法/分发 2 项 + 命令表/参数表） |
| `cargo test -p cad-scene --locked` | PASS（**88 passed**；`derive_when_empty` 回退行为） |
| `cargo test -p cad-history --locked` / `cad-host` | PASS（**43** / **487 passed**） |
| `cargo clippy`（12 个受影响 crate，`-D warnings`） | PASS（exit 0） |
| `cargo fmt --all -- --check` | PASS（exit 0） |
| `check-architecture.py` / `check-file-size.py` / `check-i18n.py` / `check-command-docs.py` | PASS（40 packages / 2946 文件 / 3664 键 / 0 mismatches） |
| `(cd apps/desktop-ui && npm test)` | PASS（**2117 passed**；含 3 个 Soften 面板/目录契约） |

## 证据边界

- **没有真实 GUI/GPU/像素证据**：没有在窗口里应用柔化观察边消失，也没有截图/像素断言；渲染契约是
  representation fragment 的边集断言。
- **没有 Smooth normals**：只隐藏边，不融合明暗（SketchUp 的另一个开关）。
- **没有逐边手工柔化**：命令是整实体角度；边选择器、Soft Edge 显示模式未实现。
- **匹配限制**：存储边必须与显示三角形的端点精确一致才会被柔化；无法匹配的边保持可见（保守，
  不猜）。
- **场景不含柔化**：场景快照仍只有相机/空间/样式。
- **载体**：共享层契约；Windows/macOS/Android/Web 运行时 **NOT VERIFIED**。
