# ADR-0085：构造参考线点与单条删除（GUIDE POINT / GUIDE DELETE）

- 状态：**已接受（决策，2026-11-28）**；**已实现（2026-11-28，point 行 + 单条删除；无真实 GUI/GPU 证据）**
- 后续：ADR-0087 让 `DIST GUIDE` 在两点重合时自动写 point 标记。
- 关联：ADR-0084（构造参考线）、`tables/guides.json`（minor 26 → 27）

## 背景

ADR-0084 只实现了 line 参考线与整表清理。SketchUp 还会留下**点**参考线，并且用户需要删除
单条参考线而不是清空全部。两者都落在一张非实体 guide 表上，适合在同一模型里补齐。

## 决策

### D1 point 行（minor 27）

- `Guide` 从单一 `GuideLine` 扩展为 `Guide::Line(GuideLine) | Guide::Point(GuidePoint)`。
- 行格式：line 行仍是 `{point, direction}`；point 行是 `{point}`（没有 `direction`）。
  解码按 `direction` 是否存在区分，并严格拒绝其它键。
- 存在 point 行时归档写 **minor 27**；只有 line 行仍是 **minor 26**，无 guide 仍是原档位。
- point 行与 line 行共享 64 行上限与 `guide.invalid` 校验；一次写入仍替换整表（一 revision/
  一步撤销）。

### D2 命令

- `GUIDE POINT <x> <y> <z>`：追加一个 point 标记；非有限坐标 `guide.invalid`。
- `GUIDE DELETE <n>`：删除 **1-based** 行；0/越界按 `guide.unknown_index` 拒绝，不写任何东西。
- `GUIDE LIST` 报告时带 `1. line ...` / `2. point ...` 前缀，便于 DELETE 选择。
- 两者都 Work-only（Viewer `PermissionDenied`），一次整表写入/一步撤销；`CLEAR` 行为不变。

### D3 渲染

- point 标记画成三轴十字：以模型包围盒对角线 ×0.005 为臂长（钳制在 1..25 单位），三段
  `LineSegments`，颜色/批处理与 line 参考线一致；仍使用合成 source，不进实体、拾取、导出。
- 不新增点原语；这是渲染批次没有点图元时的显式近似。

## 不在本 ADR 范围

- Tape/Protractor 在捕获中自动创建 point（point 由 `GUIDE POINT` 显式创建）；
- 虚线、角度圆弧 guide、单条拾取删除、图纸空间 guide；
- 真实 GUI/GPU 与其它载体运行时。

## 后果

- guide 表现在可持有 line 与 point 两类行，档位升到 27；`GUIDE DELETE` 让单条管理可用。
- Tape/Protractor 仍为 partial（自动 point、虚线、角度圆弧未建模），但 guide 点与单条删除的
  缺口关闭。
