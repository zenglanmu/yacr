# 材质编辑表单（名称/颜色/透明度）（2026-11-28）

文档材质行 Edit 展开名称/颜色/透明度，按变化字段生成 `MATERIAL RENAME/COLOR/TRANSPARENCY`
命令；多字段保存是多条命令/多步撤销。设计见 [ADR-0100](adr/0100-material-edit-forms.md)。

## 1. 已接线

- **helper**：`materialEditCommands(current, draft)` 只发变化字段，顺序 rename→color→transparency；
  rename 后使用新名称；不安全名称/非 `#rrggbb`/透明度越界返回 null。
- **UI**：Edit 行内表单 + Save/Cancel；Save 禁用条件由 helper 与命令可用性决定；无变化不发命令。
- **边界**：多字段保存多步撤销；贴图/材质库/缩略图/拖放未建模。

## 2. 验证

| 命令 | 结果 |
|---|---|
| `(cd apps/desktop-ui && npm test)` | PASS（**2153 passed**，material edit 命令顺序/拒绝 2 个） |
| `cargo test -p cad-ui-core --lib --locked` | PASS（**996 passed**，i18n keys） |
| `cargo test -p cad-app --lib --locked` | PASS（**3565 passed**，MATERIAL 既有契约） |
| `cargo fmt --all -- --check` | PASS |
| `python3 scripts/check-file-size.py` | PASS |
| `python3 scripts/check-i18n.py` | PASS（3777 keys） |
| `python3 scripts/check-architecture.py` / `check-command-docs.py` | PASS |

**边界**：没有真实 GUI 证据；表单只有前端命令生成/组件接线，真实点击与其它载体运行时
**NOT VERIFIED**。

## 3. NOT VERIFIED / 不在范围

- 贴图/材质库/缩略图/拖放；
- 真实 GUI/GPU 交互与其它载体运行时验收。
