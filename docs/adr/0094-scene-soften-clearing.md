# ADR-0094：场景柔化清除（Clear non-explicit soften）

- 状态：**已接受（决策，2026-11-28）**；**已实现（2026-11-28，bounded clear list；无真实 GUI/GPU 证据）**
- 关联：ADR-0090（显式 soften 快照）、`SOFTENEDGES`

## 背景

ADR-0090 只捕获显式 `render_attributes.soften`，恢复时不会清除保存后新增的柔化值。因此场景
对“保存时没有柔化的 body”不能回到原状。要完整恢复，需要记录这些 body 的身份。

## 决策

- 场景行新增可选 `soften_cleared: [<entity id>...]`：`SCENE SAVE` 记录所有**无显式 soften**
  的 `SemanticGeometry::Body3d`（按 id 排序），最多 **1024** 行；与 `soften` 列表互斥（同一
  entity 不能同时出现）。非法/重复/超预算按 `scene.soften_cleared_invalid` 拒绝。
- 空列表省略键；无显式列表且无 cleared 的旧场景保持原档位；存在 cleared 列表升 **minor 29**。
- `SCENE RESTORE` 把 `soften_cleared` 中当前仍有值的实体合并进同一个 `update_entity_display`
  事务，将 `soften` 设为 `None`；与显式 angle 的写回共享一次 revision/撤销。Viewer 有任一差异时
  在改动前 `PermissionDenied`。
- 只记录 Body3d：`SOFTENEDGES` 只对 body 生效；其它实体即使通过直接属性写有 soften，也只会被
  显式 `soften` 快照覆盖，不会进入 cleared 列表（文档写明）。

## 不在本 ADR 范围

- 超过 1024 个无显式柔化 body 的文档（保存按名拒绝，不截断）；
- 剖面、场景缩略图；
- 真实 GUI/GPU 与其它载体运行时。

## 后果

- 场景恢复能真正清回“无显式柔化”，不再留下保存后的值；场景仍为 partial（剖面/缩略图）。
- 表格式新增 `soften_cleared`（minor 29），旧场景解码为 `None` 并保持原行为。
