# ADR-0090：场景柔化快照（Scenes capture explicit soften）

- 状态：**已接受（决策，2026-11-28）**；**已实现（2026-11-28，显式 soften 快照；无真实 GUI/GPU 证据）**
- 关联：ADR-0063（场景表）、ADR-0064（`SOFTENEDGES`）、ADR-0080（场景 display 快照）

## 背景

`SOFTENEDGES` 把每实体的柔化角度写进 `render_attributes.soften`（minor 21），但场景不记录它：
切场景后柔化值保持不变，覆盖表把“软边不随场景切换”列为 Scenes 的缺口。

柔化是**逐实体**显示属性，不是全局记录；场景行必须有界，不能把大文档的每个 body 都写进去。

## 决策

### D1 有界显式快照（minor 28）

- 场景行新增可选 `soften: [{"entity": <id>, "angle": <deg>}]`，只包含**显式**
  `render_attributes.soften = Some(angle)` 的实体，最多 **1024** 行；角度必须有限且在
  `0..=180`，实体 id 唯一且可持久化。
- 空快照省略键，因此没有显式柔化的场景保持原档位；非空快照把归档升到 **minor 28**。
- 明确语义：恢复只写快照里的实体；保存时没有显式值的实体**不会被场景改变**（即使后来被
  `SOFTENEDGES` 改过）。这是有界快照的诚实边界，不冒充全量实体快照。

### D2 保存/恢复

- `SCENE SAVE` 遍历文档实体，捕获所有 `Some(angle)`，按 entity id 排序；超过上限按
  `scene.soften_invalid` 拒绝保存。
- `SCENE RESTORE`：对快照中存在且当前值不同的实体，执行一次 `update_entity_display` 事务
  （一条 revision/一步撤销，标签 `SCENE RESTORE <name> (soften)`），然后恢复相机/标签/display。
- 显示/柔化有差异且当前是 Viewer 时，在改动任何状态前返回 `PermissionDenied`；相机-only 恢复
  仍不写文档。柔化写入与 display 写入各自成一步，顺序为柔化→display。

### D3 边界

- 不捕获无显式柔化的实体，因此不能把后来新增的柔化清回“无”。
- 场景缩略图、剖面、逐边手工柔化/Smooth normals 仍不建模。

## 后果

- 场景行可携带显式柔化快照（minor 28）；恢复会写 `render_attributes`，不再只是会话应用。
- Scenes 仍为 partial：无全量实体快照、无剖面/缩略图；但“柔化完全不随场景切换”的缺口关闭。
