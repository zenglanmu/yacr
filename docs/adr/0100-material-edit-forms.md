# ADR-0100：材质编辑表单（名称/颜色/透明度）

- 状态：**已接受（决策，2026-11-28）**；**已实现（2026-11-28，行内表单 + 既有 MATERIAL 命令；无真实 GUI 证据）**
- 关联：ADR-0099（MATERIAL 命令）、ADR-0079（文档材质行）

## 背景

ADR-0099 已提供 `MATERIAL <name> RENAME|COLOR|TRANSPARENCY|DELETE`，但面板只暴露 Delete。材质
“编辑器”仍缺名称/颜色/透明度的用户入口。

## 决策

- 文档材质行新增 **Edit**：行内展开名称文本、`<input type="color">` 和透明度数值（0..1）。
- `materialEditCommands(current, draft)` 只生成**变化字段**的命令，顺序为 rename → color →
  transparency；rename 后 color/transparency 使用新名称。名称 trim 非空且不含 `"`/换行，颜色必须是
  `#rrggbb`，透明度 0..1；任何违规返回 null，Save 禁用。
- 多字段保存是**多条普通命令/多步撤销**（没有跨字段事务）；面板不伪造单步。
- 取消不写任何东西；命令失败由既有 `shell` 错误通道显示。无新命令/格式变化。
- 面板仍不提供材质缩略图、拖放、贴图/材质库。

## 不在本 ADR 范围

- 贴图/纹理材质、材质库、缩略图、拖放涂刷；
- 名称大小写规则/国际化（由 MATERIAL 命令层决定）；
- 真实 GUI/GPU 与其它载体运行时。

## 后果

- 材质名称/颜色/透明度有了真实 UI 入口；Materials 仍为 partial（贴图/库/缩略图）。
- 多字段编辑的撤销步数与发出的命令条数一致，文档明确不承诺原子单步。
