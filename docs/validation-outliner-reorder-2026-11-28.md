# Outliner 顶层行重排（DRAWORDER）（2026-11-28）

Outliner 顶层 entity/INSERT 行 ↑/↓：先按 `[相邻行, 当前行]` 选择，再 dispatch
`DRAWORDER BELOW/ABOVE`，一步撤销；host 顶层行按 `(draw_order, id)` 排序。设计见
[ADR-0097](adr/0097-outliner-row-reorder.md)。无新命令/格式变化。

## 1. 已接线

- **host read**：顶层模型行按 `(draw_order, id)`；组行仍在前，组内 capture order 不变。
- **UI**：↑ → `DRAWORDER BELOW`（当前到上一行前）；↓ → `DRAWORDER ABOVE`（当前到下一行后）；
  边界禁用；分组行/块成员不参与。
- **顺序**：`selectBimEntities` 保留 ids 顺序，DRAWORDER 把首 ref 当参考；命令后刷新。
- **边界**：组行/组内/块成员顺序、拖拽重排未建模。

## 2. 验证

| 命令 | 结果 |
|---|---|
| `cargo test -p cad-host --lib --locked` | PASS（**507 passed**，顶层 draw_order 排序 + DRAWORDER 顺序） |
| `cargo test -p cad-ui-core --lib --locked` | PASS（**994 passed**） |
| `cargo test -p cad-app --lib --locked` | PASS（**3561 passed**，DRAWORDER 既有契约） |
| `(cd apps/desktop-ui && npm test)` | PASS（**2149 passed**，reorder 计划 1 个） |
| `cargo fmt --all -- --check` | PASS |
| `python3 scripts/check-file-size.py` | PASS |
| `python3 scripts/check-i18n.py` | PASS（3768 keys） |
| `python3 scripts/check-architecture.py` / `check-command-docs.py` | PASS |

**边界**：没有真实 GUI 证据；重排只有 host 排序/前端命令生成契约，真实点击与其它载体运行时
**NOT VERIFIED**。

## 3. NOT VERIFIED / 不在范围

- 分组/组内/块成员排序、拖拽重排；
- 真实 GUI/GPU 交互与其它载体运行时验收。
