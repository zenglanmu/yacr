# Outliner 多选与 Group selected（2026-11-28）

Outliner 行支持 Ctrl/Cmd/Shift 多选，选中行的 `selectIds` 并集发 `cad_select_entities`；
名称输入 + Group selected dispatch 既有 `GROUP CREATE "<name>"`。设计见
[ADR-0093](adr/0093-outliner-group-selection.md)。无新命令/格式变化。

## 1. 已接线

- **多选**：普通点击替换为单行；修饰键点击 toggle；每次变化发选中行 `selectIds` 的去重并集；
  不可选块成员不贡献 id；`aria-pressed` 只是本地提示。
- **建组**：`groupCreateCommand` 仅接受 trim 非空、不含 `"`/换行的名称；先选择并集再
  `GROUP CREATE "<name>"`，成功后清空名称/选择并刷新。
- **边界**：无重排/拖拽归组/自动命名；真实选择以宿主 `cad_status.selection` 为准。

## 2. 验证

| 命令 | 结果 |
|---|---|
| `(cd apps/desktop-ui && npm test)` | PASS（tsc --noEmit + node --test：**2145 passed**，outliner 新增并集/建组 2 个） |
| `cargo test -p cad-ui-core --lib --locked` | PASS（**994 passed**，i18n/命令未改） |
| `cargo test -p cad-app --lib --locked` | PASS（**3561 passed**，GROUP CREATE 既有契约） |
| `cargo fmt --all -- --check` | PASS |
| `python3 scripts/check-file-size.py` | PASS |
| `python3 scripts/check-i18n.py` | PASS（3762 keys） |
| `python3 scripts/check-architecture.py` / `check-command-docs.py` | PASS |

**边界**：没有真实 GUI 证据；面板只有前端选择/命令生成与组件接线，真实点击与其它载体运行时
**NOT VERIFIED**；重排/拖拽归组未建模。

## 3. NOT VERIFIED / 不在范围

- 重排/拖拽归组、自动命名、组内成员级建组；
- 真实 GUI/GPU 交互与其它载体运行时验收。
