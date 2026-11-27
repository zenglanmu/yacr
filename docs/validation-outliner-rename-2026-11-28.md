# Outliner 分组改名（2026-11-28）

Outliner 分组行增加行内改名：dispatch 既有 `GROUPEDIT "<name>" RENAME "<new>"`，复用命令层
的校验/撤销，无新命令或格式变化。设计见 [ADR-0088](adr/0088-outliner-group-rename.md)。

## 1. 已接线

- **面板**：分组行名称旁 **Rename**；行内输入 + Save/Cancel，Enter 保存、Escape 取消；成功后
  依赖 `commandLog` 计数刷新。
- **命令生成**：`groupRenameCommand` 只接受 trim 后非空、且不含 `"`/换行的名字（typed grammar
  无转义）；否则禁用入口，不拼错名。
- **范围**：只对 `kind:"group"` 行；INSERT/实体/块成员仍只读。

## 2. 验证

| 命令 | 结果 |
|---|---|
| `(cd apps/desktop-ui && npm test)` | PASS（tsc --noEmit + node --test：**2142 passed**，outliner 新增改名/引号拒绝 2 个） |
| `cargo test -p cad-ui-core --lib --locked` | PASS（**994 passed**，i18n keys 一致） |
| `cargo test -p cad-app --lib --locked` | PASS（**3554 passed**，命令层未改） |
| `cargo fmt --all -- --check` | PASS |
| `python3 scripts/check-file-size.py` | PASS（3055 files） |
| `python3 scripts/check-i18n.py` | PASS（3754 keys） |
| `python3 scripts/check-architecture.py` / `check-command-docs.py` | PASS |

**边界**：没有真实 GUI 证据；面板只有前端命令生成与组件接线，真实点击与其它载体运行时
**NOT VERIFIED**；重排/拖拽归组/建组/隐藏仍未实现。

## 3. NOT VERIFIED / 不在范围

- 重排/拖拽归组/建组/隐藏、块/实体命名；
- 真实 GUI/GPU 交互与其它载体运行时验收。
