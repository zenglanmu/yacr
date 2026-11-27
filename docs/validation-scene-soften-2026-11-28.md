# 场景柔化快照（Scenes soften，`.ymodel` minor 28）（2026-11-28）

`SCENE SAVE` 捕获有显式 `render_attributes.soften` 的实体（≤1024），`SCENE RESTORE` 按一次
display 属性事务写回；无显式值的实体不被场景改变。设计见
[ADR-0090](adr/0090-scene-soften-snapshot.md)。

## 1. 已接线

- **行**：`tables/scenes.json` 行的 `soften`（entity + angle，≤1024，0..=180，唯一）；空快照省略，
  非空写 **minor 28**；无显式柔化的场景保持原档位。
- **保存**：遍历文档实体捕获 `Some(angle)`，排序；超限 `scene.soften_invalid` 拒绝。
- **恢复**：仅快照实体、仅当前值不同者，一次 `update_entity_display` 事务/一步撤销；与
  display/tags 顺序为柔化→display→相机；Viewer 有差异时改动前 `PermissionDenied`。
- **边界**：无显式值的实体不恢复；剖面/缩略图/全量实体快照未建模。

## 2. 验证

| 命令 | 结果 |
|---|---|
| `cargo test -p cad-app --lib --locked` | PASS（**3557 passed**，scenes 16：捕获、恢复+undo、Viewer 拒绝、旧场景不清柔化） |
| `cargo test -p cad-db --lib --locked` | PASS（**526 passed**） |
| `cargo test -p cad-domain --locked` | PASS（**290 unit + 2/9/14/3/3 integration**） |
| `cargo test -p cad-store-zip --test scene_display_minor --locked` | PASS（**4 passed**：含 minor-28 soften 往返） |
| `cargo test -p cad-store-zip --locked` | PASS（archive manifest 档位更新到 28） |
| `cargo test -p cad-ui-core --lib --locked` | PASS（**994 passed**） |
| `cargo test -p cad-host --lib --locked` | PASS（**506 passed**） |
| `(cd apps/desktop-ui && npm test)` | PASS（**2143 passed**） |
| `cargo clippy -p cad-app -p cad-db -p cad-domain -p cad-history -p cad-store-zip -p cad-ui-core -p cad-host --all-targets --locked -- -D warnings` | PASS |
| `cargo fmt --all -- --check` | PASS |
| `python3 scripts/check-file-size.py` / `check-i18n.py` / `check-architecture.py` / `check-command-docs.py` | PASS |

**边界**：没有真实 GUI/GPU 像素证据；场景柔化只有编解码/命令契约，真实窗口与其它载体运行时
**NOT VERIFIED**。

## 3. NOT VERIFIED / 不在范围

- 无显式柔化实体的清回、全量实体快照、剖面、缩略图；
- 真实 GUI/GPU 交互与其它载体运行时验收。
