# 组件面板动作（Components：INSERT / BLOCK）（2026-11-27）

Components 托盘从只读列表变成可操作面板：每行 **Insert** 走 `INSERT <name>`，名称输入 +
**Create from selection** 走 `BLOCK <name>`；块名作为捕获的 pending text，点一下即放置/定义。
设计见 [ADR-0076](adr/0076-components-panel-actions.md)。无新命令、无格式档位变化。

## 1. 已接线

- **会话**：`SessionState.pending_tool_text`（捕获级）；`capture_point_with_snap` 在
  `InsertBlock`/`BlockDefine` 点捕获后把待用名称作为 terminal text 应用并自动确认；`begin_draw`/
  `cancel_tool`/`complete_tool` 清空（新捕获不继承旧名称）；空/多行名称不记录。
- **typed 入口**：`INSERT <name>` / `BLOCK <name>`（一个 trim 后非空单行名称；其它文本语法拒绝），
  裸选择器仍是 point-then-name 捕获。
- **UI**：Components 面板列出块定义（名称 + 成员数），每行 Insert；名称输入 + Create from selection；
  没有文档/没有名称时禁用并按提示说明；未建模的组件语义（缩略图、替换、嵌套就地编辑、拖放、搜索）
  在提示与覆盖表写明。
- **面板助手**：`domains/concept/components.ts` 生成 `INSERT <name>` / `BLOCK <name>` 并做
  单行/非空校验。

## 2. 验证

| 命令 | 结果 |
|---|---|
| `cargo test -p cad-app --lib --locked` | PASS（**3529 passed**，含 `block_insert` 4 个：INSERT/BLOCK 预置名、捕获清空、非法名） |
| `cargo test -p cad-ui-core --lib --locked` | PASS（**985 passed**，含 `tests_block_names` 3 个：语法、INSERT 路由、BLOCK 路由） |
| `cargo test -p cad-host --lib --locked` | PASS（**503 passed**） |
| `(cd apps/desktop-ui && npm test)` | PASS（tsc --noEmit + node --test：**2134 passed**，含 components 2 个） |
| `cargo clippy -p cad-app -p cad-host -p cad-ui-core --all-targets --locked -- -D warnings` | PASS |
| `cargo fmt --all -- --check` | PASS |
| `python3 scripts/check-file-size.py` | PASS（3017 files） |
| `python3 scripts/check-i18n.py` | PASS（3741 keys） |
| `python3 scripts/check-architecture.py` / `check-command-docs.py` | PASS |

**边界**：没有真实 GUI/GPU 像素证据；面板动作只有会话状态机与前端命令生成契约，真实窗口点击与
其它载体运行时 **NOT VERIFIED**；组件缩略图/替换/嵌套就地编辑/拖放/搜索仍未建模。

## 3. NOT VERIFIED / 不在范围

- 组件缩略图、替换/重载、嵌套组件就地编辑、拖放/搜索；
- 组件库/外部文件与属性编辑面板；
- 真实 GUI/GPU 交互与其它载体运行时验收。
