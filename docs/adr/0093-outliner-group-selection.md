# ADR-0093：Outliner 多选与 Group selected

- 状态：**已接受（决策，2026-11-28）**；**已实现（2026-11-28，前端多选 + 既有 GROUP CREATE；无真实 GUI 证据）**
- 关联：ADR-0066（Outliner）、ADR-0088（改名）、`GROUP CREATE`

## 背景

Outliner 只能单行选择：点一行会用 `cad_select_entities` 替换选择，无法把多个对象一起建组。
命令层已有完整的 `GROUP CREATE <name>`（当前选择 → 持久分组表，一步撤销），缺的是面板多选与
建组入口。

## 决策

- 行点击：普通点击替换为单行；Ctrl/Cmd/Shift 点击在当前本地集合上 toggle。每次变化把选中行的
  `selectIds` **并集**（去重、按行顺序）通过 `cad_select_entities` 发给宿主；不可选的行（块成员）
  不贡献 id。面板只显示 `aria-pressed` 本地选择标记，真实选择仍以 `cad_status.selection` 为准。
- 名称输入 + **Group selected**：`groupCreateCommand` 生成 `GROUP CREATE "<name>"`，名称 trim
  非空且不含 `"`/换行才可用（GROUP grammar 无转义）；先 `cad_select_entities(union)` 再执行命令，
  成功后清空名称/本地选择并刷新。
- 没有新命令/协议/格式；`GROUP CREATE` 的重名、空选择等拒绝仍由命令层给出。

## 不在本 ADR 范围

- Outliner 重排、拖拽归组、自动命名、分组嵌套编辑；
- 跨文档选择、组内成员级建组；
- 真实 GUI/GPU 与其它载体运行时。

## 后果

- Outliner 的“建组未实现”缺口关闭（多选 + GROUP CREATE）；仍为 partial（重排/拖拽归组）。
- 本地多选标记是 UI 提示，不是第二份选择权威；宿主 selection 仍是唯一状态。
