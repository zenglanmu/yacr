# 场景柔化清除（Clear non-explicit soften，`.ymodel` minor 29）（2026-11-28）

场景行记录保存时**没有显式 soften** 的 body（`soften_cleared`，≤1024），恢复时在同一 display
属性事务里清空；显式值仍按 ADR-0090 写回。设计见
[ADR-0094](adr/0094-scene-soften-clearing.md)。

## 1. 已接线

- **行**：`soften_cleared`（entity id 数组，≤1024，唯一、可持久化、与 `soften` 互斥）；空列表
  省略，存在则写 **minor 29**；旧场景解码 `None`。
- **保存**：遍历 Body3d，`soften=None` 进入 cleared；`soften=Some` 进入显式列表；按 id 排序。
- **恢复**：显式值与 cleared body 合并为一次 `update_entity_display` 事务/一步撤销；cleared 将
  `soften=None`；Viewer 有差异时改动前拒绝。
- **边界**：>1024 个无显式 body 的文档保存按 `scene.soften_cleared_invalid` 拒绝；非 Body3d 的
  直接属性写不进入 cleared；剖面/缩略图未建模。

## 2. 验证

| 命令 | 结果 |
|---|---|
| `cargo test -p cad-app --lib --locked` | PASS（**3561 passed**，scenes 16：新增“无显式值恢复后清空+undo”） |
| `cargo test -p cad-db --lib --locked` | PASS（**526 passed**） |
| `cargo test -p cad-domain --locked` | PASS（**290 unit + 2/9/14/3/3 integration**） |
| `cargo test -p cad-store-zip --test scene_display_minor --locked` | PASS（**5 passed**：含 minor-29 cleared 往返） |
| `cargo test -p cad-store-zip --locked` | PASS（archive manifest 档位更新到 29） |
| `cargo test -p cad-ui-core --lib --locked` | PASS（**994 passed**） |
| `cargo test -p cad-host --lib --locked` | PASS（**506 passed**） |
| `(cd apps/desktop-ui && npm test)` | PASS（**2143 passed**） |
| `cargo fmt --all -- --check` / file-size / i18n / architecture / command-docs | PASS |

**边界**：没有真实 GUI/GPU 像素证据；场景柔化只有编解码/命令契约，真实窗口与其它载体运行时
**NOT VERIFIED**。

## 3. NOT VERIFIED / 不在范围

- 超限文档的策略、非 Body3d 的 cleared、剖面/缩略图；
- 真实 GUI/GPU 交互与其它载体运行时验收。
