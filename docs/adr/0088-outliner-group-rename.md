# ADR-0088：Outliner 分组改名

- 状态：**已接受（决策，2026-11-28）**；**已实现（2026-11-28，行内改名 dispatch 既有命令；无真实 GUI 证据）**
- 关联：ADR-0066（Outliner 只读投影）、`GROUPEDIT RENAME`

## 背景

Outliner 面板只读：分组行可以展开/选中，但没有改名入口。命令层已经存在完整的
`GROUPEDIT <name> RENAME <new>`（分组表 patch、一步撤销、重名/空名拒绝），缺的只是面板接线。

## 决策

- 分组行（`kind: "group"`）在名称旁提供 **Rename** 按钮：行内输入框 + Save/Cancel；Save
  dispatch `GROUPEDIT "<old>" RENAME "<new>"`，复用普通命令漏斗、权限与撤销，不新增命令/协议。
- 名称按 typed grammar 的双引号形式传递：`groupRenameCommand` 拒绝空名、含 `"` 或换行的名字
  （该 grammar 无转义），此时不显示/禁用改名入口，而不是拼出另一个名字。
- 输入 `Enter` 保存、`Escape` 取消；命令成功后面板靠现有 `commandLog` 计数刷新，失败由
  `shell` 的错误通道显示。
- 只对分组行提供改名；INSERT/实体/块成员没有可写的用户名称，保持只读。

## 不在本 ADR 范围

- 重排、拖拽归组、建组、隐藏/显示、块/实体命名；
- 名称国际化/大小写规则（由命令层 `GROUPEDIT` 决定）；
- 真实 GUI/GPU 或其它载体证据。

## 后果

- Outliner 的“改名未实现”缺口关闭，且不改变任何存储格式或命令表。
- Outliner 仍为 partial：重排/拖拽归组/建组/隐藏仍未实现。
