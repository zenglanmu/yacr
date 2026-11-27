# ADR-0084：持久构造参考线（DIST GUIDE / PROTRACTOR GUIDE）

- 状态：**已接受（决策，2026-11-28）**；**已实现（2026-11-28，非实体 guide 表 + 测量 guide 模式；无真实 GUI/GPU 证据）**
- 后续：ADR-0086 将 line guide 从连续实线段改为分段虚线（point 十字保持实线）。
- 关联：ADR-0063（场景表模式）、`tables/guides.json`（minor 26）、`DIST`/`PROTRACTOR`

## 背景

覆盖表把 Tape Measure 记为“只有距离”，把 Protractor 的 `GUIDE` 记为具名拒绝：本构建没有
guide 模型。SketchUp 的 Tape Measure / Protractor 在测量时可以直接留下无限构造参考线，这些
线不进实体、不导出，但必须持久化、可清除、可撤销。

## 决策

### D1 非实体 guide 表（minor 26）

- `tables/guides.json` 是有界表（**64** 行），每行一条**无限直线**：
  `{point:{x,y,z}, direction:{x,y,z}}`，方向归一化；未知键/非有限/零方向按
  `guide.invalid` 拒绝。
- 它不是实体表：guide 没有 `EntityId`、图层、draw order、块成员或导出表示；快照总是整表编码，
  没有行的增量 update。存在非空表时归档写 **minor 26**。
- 一次 guide 写入替换整表，一次 revision / 一步撤销（历史带 whole-table `GuidePatch`，
  与 display/scene 表走同样的独占回放分支）。

### D2 测量 guide 模式

- `DIST GUIDE`：启动普通 Distance 捕获；两点完成时同时写一条过两点的 guide。
- `PROTRACTOR GUIDE`：启动 Protractor 捕获；顶点 + 第一射线 + 第二射线完成后，从**顶点沿
  第二条射线**写 guide（这是本构建明确选择的语义，不声称角度圆弧）。
- 两种模式都是 Work-only（Viewer 在启动前拒绝 `guide.requires_edit`）；普通
  `DIST`/`PROTRACTOR` 仍只读、Viewer 合法。guide 模式在工具生命周期结束时清空；测量仍照常返回
  结果。表满/退化方向按 `guide.cap_exceeded` / `guide.degenerate` 拒绝，不写半条 guide。
- `GUIDE` / `GUIDE LIST` 只读报告（最多前 8 行）；`GUIDE CLEAR` 是一次整表写入/一步撤销，
  Viewer 拒绝。

### D3 渲染

- 渲染批次没有无限线原语，因此每条 guide 画成**有界实线段**：以模型的包围盒对角线 ×10 为
  半长、至少 1000 单位；没有模型包围盒时用 1000。这是明确的近似，不声明无限线。
- guide 批次使用合成 source（`EntityId(0)`，永不分配），不进入实体索引、拾取或选择；颜色固定为
  蓝灰，线型连续（本切片不做虚线）。
- 只画模型空间；图纸空间 guide 未建模。

## 不在本 ADR 范围

- guide 点、角度圆弧、虚线样式、guide 的自定义颜色/图层；
- 拾取/选择/擦除单条 guide（目前只有 `GUIDE CLEAR` 整表管理）；
- 图纸空间 guide、导出、跨载体 GUI/GPU 运行时验证。

## 后果

- 新增 `tables/guides.json`（minor 26）、`GUIDE` 命令（262）与两个测量 guide 模式；Tape/
  Protractor 行仍为 partial（guide 点/圆弧/无限渲染未建模），但“没有 guide 线”的缺口关闭。
