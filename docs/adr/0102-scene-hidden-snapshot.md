# ADR-0102：场景隐藏实体快照（`SCENE SAVE/RESTORE` 的 `hidden` 行）

- 状态：**已接受（决策，2026-11-28）**；**已实现（2026-11-28，场景行 + 会话应用；无真实 GUI 交互证据）**
- 关联：ADR-0063（scenes）、ADR-0078（tags）、ADR-0090/0094（soften）、`tables/scenes.json`

## 背景

`HIDEOBJECTS`/`SHOWOBJECTS` 的隐藏是**会话态**（`SessionState.hidden_entities`），场景此前只保存
相机/样式/tags/display/soften。用户切换场景时隐藏集不会跟着变，覆盖表把 Scenes 的未建模项写成
“剖面与场景缩略图”，但“保存隐藏对象”同样是 SketchUp 场景的常用状态。隐藏 ref 带有
`instance`/`sub_element`，而场景行只能按 entity id 表达，因此需要一个明确的边界。

## 决策

- `Scene.hidden: Option<Vec<EntityId>>`：
  - `Some([...])` = 保存时的**顶层**隐藏实体（空数组表示“当时没有隐藏”，是真实快照）；
  - `None` = 旧档没有捕获，恢复时**不触碰**当前隐藏集。
- `SCENE SAVE` 捕获当前文档中 `instance` 为空且无 `sub_element` 的隐藏 refs，按 id 排序去重，
  上限 [`SCENE_HIDDEN_CAP`] = 1024；行键为 `hidden`，新保存的场景总是带该键。
- `SCENE RESTORE` 先移除当前文档的顶层隐藏 refs，再插入快照；**保留**嵌套实例与子元素隐藏
  （它们没有行内表达）。整个过程是会话应用，不写 `cad-db`、不产生 revision，Viewer 合法。
- 行带 `hidden` 时归档写 **minor 31**（包括空数组）；minor ≤ 30 的读者得到 `None`，不猜测。
- 重复 id、非正 id、超过 1024 行按稳定标记 `scene.hidden_invalid` 拒绝。

## 不在本 ADR 范围

- 剖面（section planes）、场景缩略图、逐边/隐藏几何快照；
- 隐藏状态跨文档/跨载体持久化的额外语义（行仍只属于当前文档）。

## 后果

- “保存隐藏对象”成为真实场景状态；隐藏集的顶层部分可恢复，嵌套/子元素部分明确保留。
- Scenes 档位升到 31；未产生场景的普通文档不抬高 minor。
