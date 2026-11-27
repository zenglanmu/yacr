# 场景阴影/雾快照（Scenes Shadows/Fog，`.ymodel` minor 24）（2026-11-27）

场景行现在保存文档 `tables/display.json` 的 Shadows/Fog 记录；`SCENE RESTORE` 在记录有差异时
执行一次与 `SHADOWS`/`FOG` 相同的显示写入（一步撤销），Viewer 下在改动前拒绝。设计见
[ADR-0080](adr/0080-scene-display-snapshot.md)。

## 1. 已接线

- **行格式**：`tables/scenes.json` 行新增可选 `display`（`DisplaySettings` 的严格 JSON）；没有
  该键的 minor 20..23 行解码为 `None`，恢复时不碰显示记录。
- **保存**：`SCENE SAVE` 捕获当前文档 Shadows/Fog 记录；与相机/标签一样总是保存。
- **恢复**：记录相同 → 不写文档；不同且 Work → 一次 `tables/display.json` 写入（一次 revision/
  一步撤销，标签 `SCENE RESTORE <name>`）后应用会话状态；不同且 Viewer → 改动前
  `PermissionDenied`，相机/显示/标签都不变。
- **版本**：带 `display` 的文档写 **minor 24**（`FORMAT_MINOR = 24`）；无该键的场景行保持
  minor 20/23。
- **边界**：剖面、软边、场景缩略图/动画仍不随场景切换。

## 2. 验证

| 命令 | 结果 |
|---|---|
| `cargo test -p cad-app --lib --locked` | PASS（**3535 passed**，scenes 13；新增保存捕获/恢复一写一撤销/Viewer 拒绝 3 个） |
| `cargo test -p cad-db --lib --locked` | PASS（**521 passed**） |
| `cargo test -p cad-domain --locked` | PASS（**290 unit + 2/9/14/3/3 integration passed**） |
| `cargo test -p cad-store-zip --test scene_display_minor --locked` | PASS（**3 passed**：minor 24 往返、无记录保持 20、越界记录 `scene.display_invalid`） |
| `cargo test -p cad-store-zip --test scene_tags_minor --locked` | PASS（**3 passed**：标签档仍为 23） |
| `cargo test -p cad-store-zip --test scenes_minor --locked` | PASS（**4 passed**） |
| `cargo test -p cad-store-zip --locked` | PASS（archive manifest 档位更新到 24） |
| `cargo test -p cad-ui-core --lib --locked` | PASS（**989 passed**，未改协议） |
| `cargo test -p cad-host --lib --locked` | PASS（**505 passed**，未改 host read） |
| `(cd apps/desktop-ui && npm test)` | PASS（**2140 passed**，未改前端逻辑） |
| `cargo clippy -p cad-app -p cad-db -p cad-domain -p cad-store-zip -p cad-ui-core -p cad-host --all-targets --locked -- -D warnings` | PASS |
| `cargo fmt --all -- --check` | PASS |
| `python3 scripts/check-file-size.py` / `check-i18n.py` / `check-architecture.py` / `check-command-docs.py` | PASS |

**边界**：没有真实 GUI/GPU 像素证据；场景显示只有行编解码与命令状态机契约，真实窗口点击与
其它载体运行时 **NOT VERIFIED**；剖面/软边/缩略图仍未建模。

## 3. NOT VERIFIED / 不在范围

- 逐场景剖面/软边/缩略图、场景动画/延迟；
- 真实 GUI/GPU 交互与其它载体运行时验收。
