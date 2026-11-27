# ADR-0098：Outliner 分组成员重排（DRAWORDER）

- 状态：**已接受（决策，2026-11-28）**；**已实现（2026-11-28，组内 ↑/↓ + 既有 DRAWORDER；无真实 GUI 证据）**
- 关联：ADR-0097（顶层行重排）、`GROUP` 成员顺序、`DRAWORDER ABOVE|BELOW`

## 背景

ADR-0097 让 Outliner 顶层行按绘制顺序显示并可 ↑/↓。分组内部的成员仍按 `tables/groups.json`
的 capture order 列出，`DRAWORDER` 结果在组内不可见，也无法调整组内相对顺序。

## 决策

- host read 的 group 成员按 `(draw_order, id)` 排序后输出；**表里的成员 capture order 不变**，
  这只是同一绘制顺序的投影。组行 `selectIds` 随排序后的成员顺序。
- Outliner 分组内的 `entity` 子行显示 ↑/↓：
  - 同 scope 行 = 同一 `parentGroup` 的 depth-1 entity rows；
  - ↑：选择 `[上一成员, 当前成员]`，dispatch `DRAWORDER BELOW`；
  - ↓：选择 `[下一成员, 当前成员]`，dispatch `DRAWORDER ABOVE`；
  - 边界禁用；组行本身、块成员（不可选、无 model-space 地址）不参与。
- 每次移动是一条普通 DRAWORDER 命令/一步撤销；不改变组表成员集合，只改实体 `draw_order`。
- 无新命令/协议/格式。

## 不在本 ADR 范围

- 块定义成员顺序、嵌套组展开/顺序；
- 组表 capture order 的重写、拖拽重排；
- 真实 GUI/GPU 与其它载体运行时。

## 后果

- 组内成员顺序可在 Outliner 中调整并与绘制顺序一致；Outliner 仍为 partial（块成员/嵌套）。
