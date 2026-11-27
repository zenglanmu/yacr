# ADR-0066：大纲（Outliner）

- 状态：**已接受（决策，2026-11-27）**；**已实现（2026-11-27，只读投影 + 选择；无真实 GUI/GPU 证据）**
- 关联：ADR-0062/0063/0064/0065（同属“文档投影 + 面板”）、ADR-0042 D11（请求携带型 host read）

## 背景

SketchUp 的 Outliner 托盘按层级列出模型里的组、组件和原始几何：展开/折叠、点选对象，并可改名、
重排、拖拽归组。本仓库已有持久化的**分组表**（`tables/groups.json`，成员是可直接寻址的实体）、
**块定义 + INSERT 引用**（块成员在 `SpaceId::Block`）和普通模型空间实体，但没有一个统一的层级读取，
Outliner 面板仍是 `pending`。

## 决策

### D1 一个只读、有界的层级投影（`cad_outliner` / `kind: "outliner"`）

`cad-host::HostCore::outliner_tree()` 投影当前文档，**不写、不评估、不复制几何**：

- 每个持久化分组是一行 `group`（`id` = 分组序号，`label` = 名称），子行是它的成员实体
  （`kind: "entity"`，`depth: 1`，按成员顺序）；
- 不属于任何分组的模型空间实体按 **id 升序**成为顶层行：`INSERT` 是 `insert` 容器
  （`label` 优先用块名），子行是块定义的成员（`kind: "member"`，`depth: 1`）；其余是 `entity`；
- 行是**前序扁平列表** + 显式 `depth`，面板据此做展开/折叠，不需要第二次层级请求；
- `total` 统计完整投影、`rows` 上限 [`OUTLINER_MAX_ROWS`] = 1024，`truncated` 明示页面被截断；
- 没有文档是显式空答案（`revision: 0`），不是错误。

### D2 选择复用唯一的选择通道

每行携带 `selectIds`：

- 分组行是**全部成员**（一次点击 = 一次 `cad_select_entities`，与 SketchUp “组作为一个对象”一致）；
- 实体/INSERT 行是自身 id；
- 块成员行 `selectable: false`、`selectIds: []`——`cad_select_entities` 只接受模型空间 id，
  给成员一个“模型空间”引用会指向不存在的对象，所以如实标为不可选，面板提示选中组件本身。

面板**不做本地选中态**：`cad_status.selection` 才是权威。

### D3 面板

概念域托盘的 Outliner 从 pending 变为真实面板：读 `cad_outliner`，按 `depth` 缩进、
箭头展开/折叠（本地状态），点名称 dispatch 上面的选择；错误/空/截断都有独立文案。

## 不在本 ADR 范围

- 重命名、删除、重排、拖拽归组、建组/解体、隐藏/显示开关；
- 嵌套 INSERT 的递归展开（当前只展开一层）；组内 INSERT 的块成员行；
- 缩略图、筛选、搜索、多选同步高亮（选中态来自宿主 `cad_status`，不是 Outliner 本地状态）；
- 图纸空间/布局的树（当前只投影模型空间），以及真实 GUI/GPU 证据。

## 后果

- 新增一个 host-read kind 与 `cad_outliner` wire 行，无格式档位变化（纯读取）。
- 块成员行故意不可选：这是选择通道的能力边界，不是遗漏。
- 行数上限让大树保持可用；`total`/`truncated` 使“只看了一页”不会被读成“文档只有这些对象”。
