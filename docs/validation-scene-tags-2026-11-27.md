# 场景可见标签（Scenes visible tags，`.ymodel` minor 23）（2026-11-27）

场景行现在保存每个图层**保存时的有效可见性**（图层表标志 + 会话覆盖），`SCENE RESTORE` 把它
作为会话图层覆盖重新应用，不写图层表。设计见 [ADR-0078](adr/0078-scene-visible-tags.md)。

## 1. 已接线

- **行格式**：`tables/scenes.json` 行新增可选 `tags` 数组（`{name, visible}`）；没有该键的
  minor 20..22 行解码为 `None`，恢复时不碰可见性。
- **保存**：`SCENE SAVE` 捕获全部图层的有效可见性（名称 + 可见），校验上限 1024、名称长度与
  大小写不敏感唯一（`scene.tags_invalid`）。
- **恢复**：`SCENE RESTORE` 按名称解析当前图层（精确优先，然后唯一大小写不敏感）；保存值与
  图层表标志相同则移除覆盖，不同则设置覆盖；改名/删除的图层跳过；不产生 revision/history。
- **版本**：带 `tags` 的文档写 **minor 23**（`FORMAT_MINOR = 23`）；无场景或旧场景行的文档
  仍按各自档位写出。
- **边界**：阴影、雾、剖面、软边仍不随场景切换；无“场景属性”复选框；缩略图/动画未建模。

## 2. 验证

| 命令 | 结果 |
|---|---|
| `cargo test -p cad-app --lib --locked` | PASS（**3532 passed**，含 scenes 10 个：新增保存/恢复/无覆盖捕获 3 个） |
| `cargo test -p cad-db --lib --locked` | PASS（**521 passed**） |
| `cargo test -p cad-domain --locked` | PASS（**290 unit + 2/9/14/3/3 integration passed**） |
| `cargo test -p cad-store-zip --test scene_tags_minor --locked` | PASS（**3 passed**：minor 23 往返、无标签场景保持 20、缺 `visible` 的 `scene.tags_invalid`） |
| `cargo test -p cad-store-zip --test scenes_minor --locked` | PASS（**4 passed**：旧 minor-20 行不变） |
| `cargo test -p cad-ui-core --lib --locked` | PASS（**987 passed**，未改协议） |
| `cargo test -p cad-host --lib --locked` | PASS（**503 passed**，未改 host read） |
| `(cd apps/desktop-ui && npm test)` | PASS（**2137 passed**，未改前端） |
| `cargo clippy -p cad-app -p cad-db -p cad-domain -p cad-store-zip -p cad-host -p cad-ui-core --all-targets --locked -- -D warnings` | PASS |
| `cargo fmt --all -- --check` | PASS |
| `python3 scripts/check-file-size.py` / `check-i18n.py` / `check-architecture.py` / `check-command-docs.py` | PASS |

**边界**：没有真实 GUI/GPU 像素证据；场景标签只有行编解码与命令状态机契约，真实窗口点击与
其它载体运行时 **NOT VERIFIED**；阴影/雾/剖面/软边仍不随场景切换。

## 3. NOT VERIFIED / 不在范围

- 场景缩略图、动画/延迟、逐场景阴影/雾/剖面/软边；
- 真实 GUI/GPU 交互与其它载体运行时验收。
