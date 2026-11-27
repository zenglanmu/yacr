# Outliner 拖拽撤销分组（Ungroup drop）（2026-11-28）

Outliner 列表下的 Ungroup drop zone：拖入行后按旧 `parentGroup` 分组，每组 dispatch
`GROUPEDIT "<旧组>" REMOVE <ids>`；无新命令/格式。设计见
[ADR-0096](adr/0096-outliner-ungroup-drop.md)。

## 1. 已接线

- **helper**：`ungroupCommands(rows, selectedKeys)` 按旧组聚合 ids、去重，已未分组行跳过；
  返回每条可用的 `GROUPEDIT REMOVE` 命令。
- **UI**：拖拽中的行在 Ungroup zone 上 drop 后逐条执行并刷新；分组行 drop 阻止冒泡；无命令时
  不改变任何状态。
- **边界**：行重排/draw order、跨组单事务、拖拽动画未建模。

## 2. 验证

| 命令 | 结果 |
|---|---|
| `(cd apps/desktop-ui && npm test)` | PASS（tsc --noEmit + node --test：**2148 passed**，outliner 新增 ungroup 命令分组 1 个） |
| `cargo test -p cad-host --lib --locked` | PASS（**506 passed**，parentGroup 既有契约） |
| `cargo test -p cad-ui-core --lib --locked` | PASS（**994 passed**） |
| `cargo test -p cad-app --lib --locked` | PASS（**3561 passed**） |
| `cargo fmt --all -- --check` | PASS |
| `python3 scripts/check-file-size.py` | PASS |
| `python3 scripts/check-i18n.py` | PASS（3766 keys） |
| `python3 scripts/check-architecture.py` / `check-command-docs.py` | PASS |

**边界**：没有真实 GUI 证据；拖放只有前端命令生成/组件接线，真实拖拽与其它载体运行时
**NOT VERIFIED**。

## 3. NOT VERIFIED / 不在范围

- 行重排、跨组单事务、拖拽动画；
- 真实 GUI/GPU 交互与其它载体运行时验收。
