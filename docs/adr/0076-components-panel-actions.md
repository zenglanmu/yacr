# ADR-0076：组件面板动作（Components：INSERT / BLOCK）

- 状态：**已接受（决策，2026-11-27）**；**已实现（2026-11-27，面板动作 + 预置块名；无真实 GUI/GPU 证据）**
- 关联：ADR-0055（块/属性）、`BLOCK`/`INSERT` 命令、ADR-0075（面板模式）

## 背景

概念域托盘的 Components 面板此前只读：`cad_status.blocks` 列出块定义（名称 + 成员数），但没有
插入/定义入口。`INSERT` 与 `BLOCK` 都是**先点一个点、再输入块名**的捕获
（`DrawToolKind::InsertBlock` / `BlockDefine`），所以面板即便启动工具也帮不上“选中的那个组件”。

## 决策

### D1 预置块名（pending tool text）

- `SessionState` 新增 `pending_tool_text: Option<String>`：一个**捕获级**的待用文本，只在工具生命周期
  内存在（`begin_draw`/`cancel_tool`/`complete_tool` 清空）。
- `capture_point_with_snap` 在点捕获后，若当前 kind 是 `InsertBlock`/`BlockDefine` 且已进入
  `awaiting_text`，就把待用文本作为 `set_text` 应用并返回 `auto_ready = true`——宿主随即确认。
  因此 **点一下就完成**放置/定义，不需要再手输名字。
- 空/多行名称不会被记录（`set_pending_tool_text` 只保留 trim 后的非空单行）；命令层仍校验重名、
  非法字符、长度与空选择。

### D2 两条 typed 入口

- `INSERT <name>`：启动 `BeginDraw(InsertBlock)`，并把 `<name>` 设为 pending text；裸 `INSERT`
  保持原来的 point-then-name 捕获。
- `BLOCK <name>`：启动 `BeginDraw(BlockDefine)`，同样预置名称；裸 `BLOCK` 不变。
- 语法只接受一个 trim 后非空、单行的名称；其它文本是具名语法拒绝。两个 selector 进入
  `ARGUMENT_SELECTORS`，由同一 `DrawSession` 派发。

### D3 面板

Components 面板（`DomainsTray`）：
- 每行显示名称与成员数，并有一个 **Insert** 按钮 dispatch `INSERT <name>`（无名称的行禁用）；
- 名称输入 + **Create from selection** 按钮 dispatch `BLOCK <name>`（无名称禁用；没有选择时由命令
  按名拒绝）；
- 提示文案说明“点一下放置/定义”和未建模的组件语义（缩略图、替换、嵌套就地编辑、拖放/搜索）。

## 不在本 ADR 范围

- 组件缩略图/预览、组件浏览器搜索与拖放；
- 替换/重载/重定义已有引用、嵌套组件的就地编辑；
- 组件库/外部文件、i18n 名称与属性编辑面板；
- 真实 GUI/GPU 交互与其它载体运行时。

## 后果

- 新增一个捕获级状态与两条 typed 入口；**无新命令、无格式档位变化**（`INSERT`/`BLOCK` 命令与
  实体行都不变）。
- 面板从“只读列表”变成“可放置/可定义”的真实入口；`Components` 仍是 `partial`（未建模的组件
  语义明确列出），但不再有“创建/插入未接线”的缺口。
