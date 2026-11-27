# ADR-0089：Outliner 隐藏与显示全部

- 状态：**已接受（决策，2026-11-28）**；**已实现（2026-11-28，会话态隐藏接线；无真实 GUI 证据）**
- 关联：ADR-0066（Outliner）、`HIDEOBJECTS`/`ISOLATEOBJECTS`/`UNISOLATEOBJECTS`

## 背景

Outliner 面板此前只读/可选/可改名（ADR-0088）；覆盖表把“隐藏”列在未实现项。仓库已有会话级
`HIDEOBJECTS`（隐藏当前选择）与 `UNISOLATEOBJECTS`（清除全部会话隐藏），缺的是面板入口与
隐藏态的显示。

## 决策

- host read `cad_outliner` 的每行增加 `hidden`：实体/INSERT 行查会话 `hidden_entities` 的同一
  `SelectionRef`；分组行仅当**全部**成员隐藏时为 true；块成员行恒 false（INSERT 携带隐藏身份）。
- 面板行增加 **Hide**：先 `cad_select_entities(selectIds)`，成功后 dispatch 既有
  `HIDEOBJECTS`；禁用条件为行不可选/已隐藏/该载体不支持命令。隐藏是**会话态**，不写文档、
  不进撤销、不持久化。
- 任一行 `hidden` 时显示 **Show all hidden**，dispatch `UNISOLATEOBJECTS`（清空全部会话隐藏）。
- 没有逐行 Reveal：宿主没有“按选择取消隐藏”的命令；显示全部是唯一既有语义，避免伪造新通道。

## 不在本 ADR 范围

- 持久化隐藏、逐行显示、隐藏组/层的组合规则；
- Outliner 重排/拖拽归组/建组；
- 真实 GUI/GPU 与其它载体运行时。

## 后果

- Outliner 的“隐藏未实现”缺口关闭（会话态），面板能显示隐藏状态；仍为 partial。
- 新字段 `hidden` 是 Outliner wire 的 additive 字段；旧宿主省略时前端按 `false` 读取。
