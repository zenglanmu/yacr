# Outliner 隐藏与显示全部（2026-11-28）

Outliner 行可 Hide（`cad_select_entities` + `HIDEOBJECTS`），隐藏态由新的 `cad_outliner.hidden`
标注；Show all hidden 走 `UNISOLATEOBJECTS`。设计见 [ADR-0089](adr/0089-outliner-hide.md)。

## 1. 已接线

- **host read**：实体/INSERT 行查会话 `hidden_entities`，分组行全成员隐藏才为 true，块成员恒
  false；wire 增加 additive `hidden`。
- **面板**：行 Hide（不可选/已隐藏/无命令时禁用）；隐藏行显示 “hidden”；任一行隐藏时提供
  Show all hidden。隐藏是会话态，不写文档/撤销/持久化。
- **边界**：无逐行 Reveal；重排/拖拽归组/建组未实现。

## 2. 验证

| 命令 | 结果 |
|---|---|
| `cargo test -p cad-host --lib --locked` | PASS（**506 passed**，Outliner hidden 标注/清除） |
| `cargo test -p cad-ui-core --lib --locked` | PASS（**994 passed**，wire 投影 additive 字段） |
| `cargo test -p cad-app --lib --locked` | PASS（**3554 passed**，命令层未改） |
| `(cd apps/desktop-ui && npm test)` | PASS（**2143 passed**，Outliner hidden 解析） |
| `cargo fmt --all -- --check` | PASS |
| `python3 scripts/check-file-size.py` / `check-i18n.py` / `check-architecture.py` / `check-command-docs.py` | PASS |

**边界**：没有真实 GUI 证据；面板只有 host read/前端接线，真实点击与其它载体运行时
**NOT VERIFIED**。

## 3. NOT VERIFIED / 不在范围

- 逐行显示、持久化隐藏、重排/拖拽归组/建组；
- 真实 GUI/GPU 交互与其它载体运行时验收。
