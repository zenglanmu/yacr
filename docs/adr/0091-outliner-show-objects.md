# ADR-0091：Outliner 逐行显示（SHOWOBJECTS）

- 状态：**已接受（决策，2026-11-28）**；**已实现（2026-11-28，会话命令 + 行内 Show；无真实 GUI 证据）**
- 关联：ADR-0089（Outliner 隐藏/显示全部）、`HIDEOBJECTS`/`UNISOLATEOBJECTS`

## 背景

ADR-0089 提供了 Hide 与 Show all，但隐藏行不能单独恢复：`UNISOLATEOBJECTS` 只清空全部
`hidden_entities`。Outliner 需要逐行 Reveal，而宿主没有“按选择取消隐藏”的命令。

## 决策

- 新增会话命令 `SHOWOBJECTS`（无 payload）：把当前 `session.selection` 的每个 ref 从
  `hidden_entities` 移除；未隐藏的 ref 忽略。空选择 `showobjects.selection_required`，全部
  未隐藏答 `showobjects.unchanged`，有移除答 `showobjects.shown`。
- 只改会话隐藏集，不写文档、不产生 revision/history，因此 Viewer 合法（与 HIDEOBJECTS/
  ISOLATEOBJECTS/UNISOLATEOBJECTS 同类）。
- Outliner 隐藏行增加 **Show**：先 `cad_select_entities(selectIds)`，成功后 dispatch
  `SHOWOBJECTS`；组行会选中并显示全部成员。Show all 仍走 `UNISOLATEOBJECTS`。
- 命令数 262 → 263；无格式档位/表变化。

## 不在本 ADR 范围

- 持久化隐藏、按层/按组级联隐藏状态；
- 重排/拖拽归组/建组；
- 真实 GUI/GPU 与其它载体运行时。

## 后果

- Outliner 的逐行 Reveal 缺口关闭（会话态）；仍为 partial（重排/分组/建组未建模）。
