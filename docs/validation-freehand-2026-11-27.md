# 自由手绘（Freehand）（2026-11-27）

Freehand 从 pending 变成**真实拖拽工具**：概念域工具栏/菜单启动 `SKETCH`，host 按下开始采样、
移动追加（2 逻辑像素步长、≤1024 点）、松开投影为世界点并提交**一条开放 `LWPOLYLINE`**（一次
事务/一步撤销）。设计见 [ADR-0067](adr/0067-freehand.md)。无格式变化（无新命令、无新表）。

## 1. 已接线

- **工具**：`DrawToolKind::Freehand`（`PointArity::AtLeast(2)`，追加在 `ALL` 末尾，64 → 65），
  `DrawIntent::Freehand { points }` → `CommandId::Sketch` + `CommandPayload::Points`；
  `SKETCH` 裸选择器路由到 `BeginDraw(Freehand)`。
- **host 手势**：复用 lasso 的 `LassoGesture` 采样（同一 2 px 步长/1024 上限），
  按下开始 / 移动追加 / 松开经 `screen_to_world` 投影后逐点喂入捕获并 `confirm_tool`；
  轨迹走既有 `BoxSelectionPreview` 实线 overlay。取消/多指接管丢弃采样。
- **拒绝**：点击未拖动 `sketch.too_few_points`（工具继续等待、不写文档）；3D/透视
  `sketch.requires_plan_view`（不写文档）；非有限点/少于 2 点由 `SKETCH` 命令拒绝。
- **UI**：概念域工具栏与 Draw 菜单的 Freehand 从 `pending` 变成 `command: 'SKETCH'` 的真实工具，
  新增 `concept.tool.freehand.hint`（中英）。

## 2. 验证

| 命令 | 结果 |
|---|---|
| `cargo test -p cad-app --lib --locked` | PASS（**3490 passed**，含 draw-tool 手绘 intent/状态行契约） |
| `cargo test -p cad-host --lib --locked` | PASS（**494 passed**，含 `tests::freehand` 3 个：拖拽一步撤销、点击拒绝、取消不写） |
| `cargo test -p cad-ui-core --lib --locked` | PASS（**963 passed**，含 `SKETCH` 路由到 `draw.freehand`） |
| `(cd apps/desktop-ui && npm test)` | PASS（tsc --noEmit + node --test：**2128 passed**，Freehand 不再是 pending） |
| `cargo clippy -p cad-app -p cad-host -p cad-ui-core --all-targets --locked -- -D warnings` | PASS |
| `cargo fmt --all -- --check` | PASS |
| `python3 scripts/check-file-size.py` | PASS（2977 files） |
| `python3 scripts/check-i18n.py` | PASS（3694 keys） |
| `python3 scripts/check-architecture.py` / `check-command-docs.py` | PASS |

**边界**：没有真实 GUI/GPU 像素证据；拖拽采样/提交只有 host 契约（合成指针事件）与前端布局契约，
真实窗口手感、性能与其它载体运行时 **NOT VERIFIED**。

## 3. NOT VERIFIED / 不在范围

- 平滑/曲线拟合、自动闭合、压感、笔画抽稀拟合；
- 3D 空间手绘（只在 2D 平面视图投影）；
- 把笔画自动切成边/面（SketchUp 的“边切分面”仍未建模）；
- 真实 GUI/GPU 交互、其它载体运行时验收。
