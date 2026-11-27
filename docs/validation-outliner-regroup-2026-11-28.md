# Outliner 拖拽归组（GROUPEDIT REMOVE/ADD）（2026-11-28）

Outliner 行可拖到目标分组：先 `GROUPEDIT "<旧组>" REMOVE` 再从 `GROUPEDIT "<目标>" ADD`（两步
撤销）；分组成员行有 Remove from group。设计见 [ADR-0095](adr/0095-outliner-regroup.md)。
无新命令/格式变化。

## 1. 已接线

- **host read**：group 子行新增 additive `parentGroup`；组行/INSERT/块成员/顶层实体省略。
- **拖放**：非组可选行 draggable；拖到组行按旧组分组 REMOVE，再目标 ADD；目标名不可引用时整次
  不做；ids 去重、名称/ids 由 `groupEditCommand` 校验。
- **移除**：分组成员行 Remove from group（`GROUPEDIT "<parent>" REMOVE <ids>`）。
- **边界**：行重排/draw order、拖到未分组区域未建模；一次 regroup 是两步撤销。

## 2. 验证

| 命令 | 结果 |
|---|---|
| `cargo test -p cad-host --lib --locked` | PASS（**506 passed**，Outliner `parentGroup`） |
| `cargo test -p cad-ui-core --lib --locked` | PASS（**994 passed**，wire additive 字段） |
| `cargo test -p cad-app --lib --locked` | PASS（**3561 passed**，GROUPEDIT 既有契约） |
| `(cd apps/desktop-ui && npm test)` | PASS（**2147 passed**，parentGroup/regroup 命令 2 个） |
| `cargo fmt --all -- --check` | PASS |
| `python3 scripts/check-file-size.py` | PASS |
| `python3 scripts/check-i18n.py` | PASS（3765 keys） |
| `python3 scripts/check-architecture.py` / `check-command-docs.py` | PASS |

**边界**：没有真实 GUI 证据；拖放只有前端命令生成/组件接线与 host read 契约，真实拖拽与其它
载体运行时 **NOT VERIFIED**。

## 3. NOT VERIFIED / 不在范围

- 行重排/draw order、拖到未分组区域、多组归属策略；
- 真实 GUI/GPU 交互与其它载体运行时验收。
