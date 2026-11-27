# ADR-0097：Outliner 顶层行重排（DRAWORDER）

- 状态：**已接受（决策，2026-11-28）**；**已实现（2026-11-28，前端 ↑/↓ + 既有 DRAWORDER；无真实 GUI 证据）**
- 关联：ADR-0066（Outliner）、`DRAWORDER BELOW|ABOVE`、`draw_order`

## 背景

Outliner 可以多选、建组、归组、撤销分组，但行顺序固定（主机按 entity id 排序），不能调整绘制
顺序。命令层已有 `DRAWORDER FRONT|BACK|ABOVE|BELOW`，其中 `ABOVE/BELOW` 以第一个选中对象为
参考、其余为要移动的对象。

## 决策

- host read 的顶层模型实体改为按 `(draw_order, id)` 排序（组行仍在最前，组内成员保持 capture
  order），这样 `DRAWORDER` 的结果在 Outliner 里可见。
- 顶层 `entity`/`INSERT` 行显示 ↑/↓：
  - ↑：选择 `[上一行, 当前行]`，dispatch `DRAWORDER BELOW`（当前行移到上一行之前）；
  - ↓：选择 `[下一行, 当前行]`，dispatch `DRAWORDER ABOVE`（当前行移到下一行之后）。
  - 选择顺序必须保持；`selectBimEntities` 保留 ids 顺序，命令层把第一个 ref 当参考。
  - 边界行（首/尾）按钮禁用；分组行、块成员、组内成员不参与（它们没有该层面的 draw order）。
- 每次移动是**一条**普通 DRAWORDER 命令/一步撤销；无新命令/协议/格式。
- `DRAWORDER` 的既有拒绝（空选择、ABOVE/BELOW 需要两个对象、跨 scope）原样透传。

## 不在本 ADR 范围

- 分组行排序、组内成员顺序、块定义成员顺序；
- 拖拽改变顺序（本切片只用 ↑/↓ 按钮）；
- 真实 GUI/GPU 与其它载体运行时。

## 后果

- Outliner 顶层行顺序可调整且与绘制顺序一致；Outliner 仍为 partial（嵌套/组内顺序未建模）。
- 一次移动仍是一步 undo；不引入第二份顺序权威。
