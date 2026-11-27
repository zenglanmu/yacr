# 文档材质与 Paint Bucket（2026-11-27）

SketchUp 的 Paint Bucket 第一次有了真实模型：文档拥有一张命名材质表（颜色 + 透明度）和每个实体
一个材质赋值；`PAINT <名称>` 涂当前选择，渲染在展开时立刻用材质色/透明度，`.ymodel` 以
**minor 19** 保存并在重开后保持。设计见 [`docs/adr/0062-document-materials.md`](adr/0062-document-materials.md)。

## 行为

- **表**：`tables/materials.json` 一行一个材质（`seq`/`name`/`color`/可选 `transparency`），
  `entities` 内联该材质涂刷的实体 id；名称 trimmed、大小写不敏感唯一、≤64 字符、≤256 行；
  透明度 0..=1；一个实体最多一个材质。
- **命令**：
  - `PAINT <名称>`（typed argument）涂**当前选择**（顶层整实体）。名称先在文档表解析，未命中查
    12 个内置扁平色（Red…Glass），命中内置时复制成文档行再赋值；同名文档行优先。
  - 一次涂刷 = 一次 `tables/materials.json` 写入 = 一次 revision = 一步撤销；重复涂同一材质按
    `paint.no_change` 拒绝（不写、不记撤销）。
  - `MATERIALS` 从具名拒绝毕业为**只读报告**：文档材质（名称/颜色/涂刷数量）+ 内置调色板名称；
    Viewer 合法、不写文档。
- **拒绝**（全部在写入前）：`paint.requires_material`、`paint.unknown_material`、
  `paint.selection_required`、`paint.not_addressable`（块成员/子元素）、`paint.selection_changed`、
  `paint.insert_not_paintable`（INSERT 不级联）、`paint.no_change`。
- **渲染**：`cad-representation` 在解析实体自带颜色/透明度后查赋值：命中时用材质 sRGB 颜色、
  `alpha *= 1 - transparency`、清掉 `color_unresolved`；未涂实体保持原 `ByLayer`/显式外观。
- **删除**：`remove_entity` 清掉赋值；快照解码拒绝悬空赋值（实体或材质不存在）。
- **格式**：`FORMAT_MINOR` 18 → 19；没有材质行的文档仍是原档位；`cad-store-zip` 把 materials
  放在最高档，读侧 18 及以下照常打开。
- **UI**：概念域托盘的 Materials 面板是真面板（12 个色块 + “材质报告”按钮），色块 dispatch
  `PAINT <名称>`；工具栏/菜单的 Paint Bucket 从 pending 改成真实 `MATERIALS` 命令。

## 命令与结果

| 命令 | 结果 |
|---|---|
| `cargo test -p cad-db --locked` | PASS（**510 passed**；含 10 个 materials 契约：一次 revision/一个 scope、no-op、删除剪枝、快照往返、重复名/悬空赋值/越界透明度拒绝、内置调色板、同名不覆盖） |
| `cargo test -p cad-app --lib --locked` | PASS（**3469 passed, 3 ignored**；含 8 个 `paint`/`MATERIALS` 契约：内置落行、undo/redo 往返、no-change、未知名称、空选择/子元素、INSERT 拒绝、只读报告、文档行优先） |
| `cargo test -p cad-ui-core --locked` | PASS（**947 passed, 1 ignored**；`PAINT` typed grammar/dispatch、bare `PAINT` 拒绝、`MATERIALS` 只读、Work-only 表） |
| `cargo test -p cad-representation --locked` | PASS（**373 passed**；3 个新契约：材质色、透明度→alpha、涂刷覆盖显式颜色且不影响未涂实体） |
| `cargo test -p cad-store-zip --locked` | PASS（**49 + 18 + … passed**，含 `materials_minor` **4 passed**：minor 19、普通文档 minor 4、minor 18 仍可开、超档拒绝） |
| `cargo test -p cad-history --locked` | PASS（**43 passed**；历史车道的字面量更新） |
| `cargo test -p cad-host --locked` | PASS（**487 passed**） |
| `cargo clippy -p cad-domain -p cad-db -p cad-history -p cad-representation -p cad-store-zip -p cad-app -p cad-ui-core --all-targets --locked -- -D warnings` | PASS（exit 0） |
| `cargo fmt --all -- --check` | PASS（exit 0） |
| `python3 scripts/check-architecture.py` | PASS（40 packages，无环） |
| `python3 scripts/check-file-size.py` | PASS（2916 文件，≤1000 行） |
| `python3 scripts/check-i18n.py` | PASS（3647 键，两目录一致） |
| `python3 scripts/check-command-docs.py` | PASS（0 mismatches） |
| `(cd apps/desktop-ui && npm test)` | PASS（**2110 passed**，含 `tsc --noEmit`；Materials 托盘面板、PAINT/MATERIALS 目录行与拒绝批次更新） |

## 证据边界

- **没有真实 GUI/GPU/像素证据**：没有在窗口里点色块观察颜色变化，也没有截图/像素断言；渲染契约是
  `cad-representation` 的 fragment 颜色/alpha 断言。
- **只有实体级**：一个网格是一个实体，涂刷整块；**面级材质**（`face_sources` 键）未建模，
  纹理/图片材质也未建模（网格 `texture`/`pbr` 保持导入器语义）。
- **INSERT 不级联**：涂块引用按 `paint.insert_not_paintable` 拒绝；块成员可按自身 id 涂。
- **文档材质只读**：没有改名/改色/删除/透明度编辑命令；内置调色板是唯一的创建来源（首次涂刷落行）。
- **载体**：命令与表在共享层，所有载体走同一路径；Windows/macOS/Android/Web 运行时 **NOT VERIFIED**。
- **无真实 SKP 语料**：调色板与测试都是自建数据，不代表任何厂商材质库。
