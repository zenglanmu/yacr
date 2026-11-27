# Outliner 逐行显示（SHOWOBJECTS）（2026-11-28）

新增会话命令 `SHOWOBJECTS`：把当前选择从 `hidden_entities` 移除；隐藏 Outliner 行可 Show。
设计见 [ADR-0091](adr/0091-outliner-show-objects.md)。无格式档位变化。

## 1. 已接线

- **命令**：`SHOWOBJECTS`（无 payload，命令数 263）；空选择 `showobjects.selection_required`，
  未隐藏 `showobjects.unchanged`，成功 `showobjects.shown`；会话态、Viewer 合法、不写文档。
- **面板**：隐藏行 Show 先 `cad_select_entities(selectIds)` 再 dispatch `SHOWOBJECTS`；组行
  显示全部成员；Show all 仍 `UNISOLATEOBJECTS`。
- **边界**：隐藏不持久化；重排/拖拽归组/建组仍未实现。

## 2. 验证

| 命令 | 结果 |
|---|---|
| `cargo test -p cad-app --lib --locked` | PASS（**3559 passed**，show_objects 2：逐行 reveal/未隐藏 no-op/Viewer、空选择） |
| `cargo test -p cad-ui-core --lib --locked` | PASS（**994 passed**，命令表/词汇表 263） |
| `cargo test -p cad-host --lib --locked` | PASS（**506 passed**） |
| `(cd apps/desktop-ui && npm test)` | PASS（**2143 passed**，隐藏行 Show 接线） |
| `cargo fmt --all -- --check` | PASS |
| `python3 scripts/check-file-size.py` | PASS |
| `python3 scripts/check-i18n.py` | PASS（3760 keys） |
| `python3 scripts/check-architecture.py` / `check-command-docs.py` | PASS |

**边界**：无真实 GUI 证据；面板只有命令接线与前端按钮，真实点击与其它载体运行时
**NOT VERIFIED**。

## 3. NOT VERIFIED / 不在范围

- 持久化隐藏、重排/拖拽归组/建组；
- 真实 GUI/GPU 交互与其它载体运行时验收。
