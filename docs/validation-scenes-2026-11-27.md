# 场景（SketchUp Scenes / Pages）与 `cad_scenes` 读取（2026-11-27）

SketchUp 的 Scenes 第一次有了真实模型：文档保存命名的相机/空间/样式快照，命令可以保存、恢复、
列出、删除，概念域托盘的 Scenes 面板通过一个 **request-carrying host read** 列出它们并 dispatch
恢复/保存/删除。设计见 [`docs/adr/0063-scenes.md`](adr/0063-scenes.md)。

## 行为

- **表**：`tables/scenes.json`，一行 = 一个 named view 对象（相机、2D/3D、保存的 2D 相机、
  工作平面、空间）+ `style`（`WIREFRAME`/`SHADED`）。≤32 行，名称 trimmed、非空、大小写不敏感
  唯一，同名保存覆盖并移到末尾；场景与 named view 命名空间独立。
- **命令**（`SCENE` typed argument）：
  - `SCENE SAVE <名称>`：捕获当前 viewport 的相机/2D-3D/工作平面 + 会话空间 + `VSCURRENT`
    样式，写表；一次 revision、一步撤销；相同状态重复保存是 `scene.unchanged`（不写）。
  - `SCENE DELETE <名称>`：一次 revision/一步撤销；`UNDO` 恢复该行。
  - `SCENE RESTORE <名称>`：会话应用相机/模式/工作平面/空间/样式，**不写文档**；有变化时推入
    `ZOOM PREVIOUS`（`VIEWBACK` 可回退）。Viewer 合法。
  - `SCENE LIST`：只读报告。
  - Viewer 模式下 `SAVE`/`DELETE` 按 `PermissionDenied` 拒绝（与 `VIEW SAVE` 同规则）。
- **host read**：新增第 5 个 kind `scenes`（`cad_scenes` wire 行）。`HostCore::scenes()` 投影
  当前文档（名称、3D 标志、空间显示名、样式），无文档是空列表；没有 seam 的载体答
  `unsupported-runtime: scenes`，不是空列表。
- **面板**：概念域托盘的 Scenes 面板是真面板：读取 `cad_scenes` 列行，点名称 →
  `SCENE RESTORE`，× → `SCENE DELETE`，输入框 + 按钮 → `SCENE SAVE`；读取永不改相机/文档。
- **格式**：`FORMAT_MINOR` 19 → 20；无场景文档保持原档位；19 及以下照常打开。

## 命令与结果

| 命令 | 结果 |
|---|---|
| `cargo test -p cad-db --locked` | PASS（**516 passed**；含 6 个 scenes 契约：一次 revision/scope、no-op、删除/撤销、快照往返、重复名/未知样式拒绝、无表空载） |
| `cargo test -p cad-app --lib --locked` | PASS（**3476 passed, 3 ignored**；含 7 个 `SCENE` 契约：保存捕获/一步撤销、相同保存 no-op、恢复相机+样式且不写文档 + `VIEWBACK`、删除/undo/redo、LIST/未知名称/空名称、Viewer 拒绝、快照往返） |
| `cargo test -p cad-ui-core --locked` | PASS（**951 passed, 1 ignored**；语法、typed 行往返、`session.scenes()` host-read 投影、`HostReadRequest::Scenes` 信封往返） |
| `cargo test -p cad-store-zip --locked` | PASS（**49 + 18 + … passed**，含 `scenes_minor` **4 passed**：minor 20、普通文档 minor 4、minor 19 仍可开、超档拒绝） |
| `cargo test -p cad-history --locked` | PASS（**43 passed**；`scene_patches` 车道；录制漏斗拆到 `record.rs` 后 lib 测试数不变） |
| `cargo test -p cad-host --locked` | PASS（**487 passed**） |
| `cargo clippy -p cad-domain -p cad-db -p cad-history -p cad-store-zip -p cad-host -p cad-app -p cad-ui-core -p cad-desktop-shell --all-targets --locked -- -D warnings` | PASS（exit 0） |
| `cargo fmt --all -- --check` | PASS（exit 0） |
| `python3 scripts/check-architecture.py` | PASS（40 packages，无环） |
| `python3 scripts/check-file-size.py` | PASS（2929 文件，≤1000 行；`cad-history` 的录制漏斗拆到 `record.rs`） |
| `python3 scripts/check-i18n.py` | PASS（3657 键，两目录一致） |
| `python3 scripts/check-command-docs.py` | PASS（0 mismatches） |
| `(cd apps/desktop-ui && npm test)` | PASS（**2114 passed**，含 4 个 Scenes 面板/解析契约） |

## 证据边界

- **没有真实 GUI/GPU/像素证据**：没有在窗口里点场景观察相机变化，也没有截图/像素断言；恢复是
  命令层契约。
- **场景不含**：可见 Tags/图层、阴影、雾、剖面、软边与缩略图（这些功能本身没有模型；存标志会是
  假数据）。样式只有 `WIREFRAME`/`SHADED` 两个真实 token。
- **没有场景改名/重排命令**：改名 = 删除+保存；排序未建模。
- **host read 只在契约层验证**：面板读取与失败提示没有真实窗口运行证据；没有 seam 的载体拒绝
  有单测，不是运行证据。
- **载体**：共享层路径在所有载体一致；Windows/macOS/Android/Web 运行时 **NOT VERIFIED**。
- **无真实 SKP 语料**：场景/相机都是自建数据。
