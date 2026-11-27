# 大纲（Outliner）（2026-11-27）

Outliner 从 pending 变为**只读的文档层级投影 + 选择**：`cad_outliner` 返回分组的、INSERT 的、
模型空间实体的与块成员的扁平前序树（显式 `depth`），托盘面板本地展开/折叠，点名称走既有的
`cad_select_entities` 选择。设计见 [ADR-0066](adr/0066-outliner.md)。

## 1. 已接线

- **host read**：第 9 个 kind `outliner`（wire 行 `cad_outliner`），`HostCore::outliner_tree()`
  投影当前文档：分组行（成员顺序）→ 成员实体行；未分组模型空间实体按 id 升序（INSERT 展开一层块成员）；
  上限 `OUTLINER_MAX_ROWS = 1024`，`total` 统计完整投影，`truncated` 明示截断；无文档 = 空树。
- **选择**：每行 `selectIds`；分组行是全部成员，实体/INSERT 行是自身，块成员行
  `selectable: false` + 空 `selectIds`（`cad_select_entities` 只接受模型空间 id）。纯读取，
  无新命令、无格式档位变化。
- **UI**：概念域托盘 Outliner 从 pending 变为真实面板（缩进 + 箭头展开/折叠 + 点名称选择），
  错误/空/截断/“块成员不可选”都有独立文案；i18n 中英各 11 键。

## 2. 验证

| 命令 | 结果 |
|---|---|
| `cargo test -p cad-host --lib --locked` | PASS（**491 passed**，含 `tests::outliner` 3 个：空文档、分组/组件容器与可选性、`total`/`truncated`） |
| `cargo test -p cad-ui-core --lib --locked` | PASS（**962 passed**，含 `tests_outliner` 4 个） |
| `(cd apps/desktop-ui && npm test)` | PASS（tsc --noEmit + node --test：**2128 passed**，含 outliner 6 个） |
| `cargo clippy ... --all-targets -- -D warnings` | PASS（受影响 crate；见下） |
| `cargo fmt --all -- --check` | PASS |
| `python3 scripts/check-architecture.py` | PASS |
| `python3 scripts/check-file-size.py` | PASS |
| `python3 scripts/check-i18n.py` | PASS |
| `python3 scripts/check-command-docs.py` | PASS |

**边界**：没有真实 GUI/GPU 像素证据；面板的展开/折叠与点击选择只有前端纯函数契约与宿主投影契约，
真实窗口交互 NOT VERIFIED。

## 3. NOT VERIFIED / 不在范围

- 重命名、删除、重排、拖拽归组、建组/解体、隐藏/显示；
- 嵌套 INSERT 递归展开（只展开一层）、组内 INSERT 的块成员行；
- 图纸空间/布局的树、缩略图、搜索/筛选、Outliner 本地选中态同步；
- 其它载体运行时与真实 GUI/GPU 像素验收。
