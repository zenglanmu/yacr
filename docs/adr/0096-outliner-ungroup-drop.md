# ADR-0096：Outliner 拖拽撤销分组（Ungroup drop）

- 状态：**已接受（决策，2026-11-28）**；**已实现（2026-11-28，前端 drop zone + 既有 GROUPEDIT；无真实 GUI 证据）**
- 关联：ADR-0095（拖拽归组）、`GROUPEDIT REMOVE`

## 背景

ADR-0095 支持拖到分组加入与成员行 Remove from group，但用户还需要把多个选中行一次从各自分组
移出。命令层已有 `GROUPEDIT "<group>" REMOVE <ids>`，缺的是“拖到未分组区域”的入口。

## 决策

- Outliner 列表下方新增 **Ungroup** drop zone；把行拖到该区域时：
  - 取拖拽集合中 `parentGroup !== null` 的行，按旧组分组；
  - 每个旧组 dispatch 一条 `GROUPEDIT "<旧组>" REMOVE <ids>`（一组一步撤销；无跨组事务）；
  - 已未分组的行不产生命令；没有可移除行时整次 drop 不做。
- 拖放到分组行时 `stopPropagation`，不会同时触发 Ungroup zone 的 drop。
- 复用 `groupEditCommand` 的名称/ids 校验；无新命令/协议/格式。多选行为沿用 ADR-0093/0095。

## 不在本 ADR 范围

- 行顺序重排/draw order；
- 拖拽预览动画、跨组单事务、持久化 UI 状态；
- 真实 GUI/GPU 与其它载体运行时。

## 后果

- Outliner 的“拖到未分组区域”缺口关闭；仍为 partial（行重排未建模）。
- 一次 Ungroup 每个旧组一条命令/一步撤销，文档不假装原子。
