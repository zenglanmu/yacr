# Outliner 分组成员重排（DRAWORDER）（2026-11-28）

Outliner 分组成员按 `(draw_order,id)` 排序，组内 entity 行 ↑/↓ 选择同组相邻成员并 dispatch
`DRAWORDER BELOW/ABOVE`；一次一条命令/一步撤销。设计见
[ADR-0098](adr/0098-outliner-member-reorder.md)。无新命令/格式变化。

## 1. 已接线

- **host read**：group 成员排序投影（表 capture order 不变）；组行 selectIds 同步排序。
- **UI**：组内 entity 行 ↑/↓ 走 `[相邻成员, 当前成员]` + `DRAWORDER BELOW/ABOVE`；边界禁用；
  组行、块成员不参与。
- **边界**：块成员顺序（不可选）、嵌套组展开/顺序、拖拽重排未建模。

## 2. 验证

| 命令 | 结果 |
|---|---|
| `cargo test -p cad-host --lib --locked` | PASS（**508 passed**，组内 draw_order 排序 + DRAWORDER） |
| `cargo test -p cad-ui-core --lib --locked` | PASS（**994 passed**） |
| `cargo test -p cad-app --lib --locked` | PASS（**3561 passed**，DRAWORDER 既有契约） |
| `(cd apps/desktop-ui && npm test)` | PASS（**2150 passed**，组内 reorder 计划） |
| `cargo fmt --all -- --check` | PASS |
| `python3 scripts/check-file-size.py` | PASS |
| `python3 scripts/check-i18n.py` | PASS（3768 keys） |
| `python3 scripts/check-architecture.py` / `check-command-docs.py` | PASS |

**边界**：没有真实 GUI 证据；重排只有 host 排序/前端命令生成契约，真实点击与其它载体运行时
**NOT VERIFIED**。

## 3. NOT VERIFIED / 不在范围

- 块成员/嵌套组顺序、拖拽重排；
- 真实 GUI/GPU 交互与其它载体运行时验收。
