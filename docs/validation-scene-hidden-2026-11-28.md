# 场景隐藏实体快照（`tables/scenes.json` 的 `hidden`，minor 31）（2026-11-28）

`SCENE SAVE` 捕获会话隐藏的顶层实体，`SCENE RESTORE` 只替换当前文档的顶层隐藏 refs；空数组是
真实快照，旧档缺字段则不触碰隐藏集。设计见 [ADR-0102](adr/0102-scene-hidden-snapshot.md)。

## 1. 已接线

- **模型**：`Scene.hidden: Option<Vec<EntityId>>`，上限 1024；`hidden` 行写入 `tables/scenes.json`，
  带该键（含 `[]`）的归档写 **minor 31**；重复/非正/超预算按 `scene.hidden_invalid` 拒绝。
- **保存**：只捕获当前文档、空 `instance`、无 `sub_element` 的隐藏 refs，按 id 排序去重；新场景
  总是带 `hidden`。
- **恢复**：移除当前文档的顶层隐藏 refs 后插入快照，保留嵌套/子元素隐藏；会话应用，无 revision，
  Viewer 合法；旧场景（`hidden: None`）不改变隐藏集。
- **UI**：Scenes 面板既有的 Save/Restore 直接生效，无新命令或按钮。
- **边界**：只捕获顶层实体；隐藏几何/边、剖面、缩略图未建模。

## 2. 验证

| 命令 | 结果 |
|---|---|
| `cargo test -p cad-app --lib --locked` | PASS（**3570 passed**；scenes 子集 18） |
| `cargo test -p cad-store-zip --test scenes_minor --locked` | PASS（**6 passed**：minor-31 往返、空数组仍 stamp 31） |
| `cargo test -p cad-domain --locked` | PASS（**290 passed**） |
| `cargo test -p cad-ui-core --lib --locked` | PASS（**997 passed**） |
| `cargo test -p cad-host --lib --locked` | PASS（**508 passed**） |
| `(cd apps/desktop-ui && npm test)` | PASS（**2159 passed**） |
| `cargo fmt --all -- --check` / file-size / i18n / architecture / command-docs | PASS |

**边界**：没有真实 GUI 点击/像素证据；Windows/macOS/Android/Web 运行时 **NOT VERIFIED**。

## 3. NOT VERIFIED / 不在范围

- 隐藏几何/逐边隐藏快照、剖面、场景缩略图；
- 真实 GUI 交互与其它载体。
