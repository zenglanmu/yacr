# ADR-0095：Outliner 拖拽归组（GROUPEDIT REMOVE/ADD）

- 状态：**已接受（决策，2026-11-28）**；**已实现（2026-11-28，前端拖放 + 既有 GROUPEDIT；无真实 GUI 证据）**
- 关联：ADR-0066（Outliner）、ADR-0093（多选/建组）、`GROUPEDIT ADD|REMOVE`

## 背景

Outliner 已支持多选与 `GROUP CREATE`，但无法把一个实体从旧组移到另一个组，也不能把成员从
分组中移除。命令层已有 `GROUPEDIT <name> ADD|REMOVE <ids>`（一次写入一步撤销），缺的是行的父组
信息与拖放入口。

## 决策

- host read 的 group 子行新增 additive `parentGroup`（组名）；组行/INSERT/块成员/顶层实体为
  absent。前端行模型保存该字段，旧宿主省略时按 `null`。
- 拖放：可选择的非组行 `draggable`；拖到目标组行释放时：
  1. 先对每个不同的旧 `parentGroup`（且不等于目标）dispatch
     `GROUPEDIT "<旧组>" REMOVE <ids>`；
  2. 再 dispatch `GROUPEDIT "<目标>" ADD <ids>`。
  两步是**两条**普通命令/两步撤销（没有跨命令事务）；目标组名含 `"` 时 `groupEditCommand` 返回
  null，整次拖放不做。
- 分组成员行提供 **Remove from group**，dispatch `GROUPEDIT "<parentGroup>" REMOVE <ids>`。
- 名称/ids 语法由 `groupEditCommand` 生成：名称 trim 非空且不含 `"`/换行，ids 为正安全整数并去重。
- 不新增命令/协议/格式；重名、未知组、非成员等拒绝仍由命令层给出。

## 不在本 ADR 范围

- 行顺序重排/draw order 写入、拖到未分组区域（会需要新的“移出全部组”语义）；
- 多组归属策略、自动命名、嵌套组；
- 真实 GUI/GPU 与其它载体运行时。

## 后果

- Outliner 的“拖拽归组/从组移除”缺口关闭；仍为 partial（行重排、拖到未分组区域未建模）。
- 一次 regroup 是两条命令/两步撤销，不是假装的单事务。
