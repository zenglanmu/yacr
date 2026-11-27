# ADR-0086：参考线虚线段（Dashed guides）

- 状态：**已接受（决策，2026-11-28）**；**已实现（2026-11-28，场景批次分段；无真实 GUI/GPU 证据）**
- 关联：ADR-0084/0085（guide 表、point/line）、`subdivide_dashes`、`LinetypePattern`

## 背景

ADR-0084 把 line guide 画成一条连续实线段：渲染批次没有无限线，场景批次也没有对直接构造的
fragment 做 dash 细分（dash 只发生在 provider 的 `emit_polyline` 路径）。SketchUp 的参考线
是虚线，本切片把 guide 批次接入现有 `subdivide_dashes`。

## 决策

### D1 场景批次分段

- line guide 先按包围盒得到有界端点（不变），再用现有
  `cad_representation::subdivide_dashes` 与 `LinetypePattern::from_elements` 切成 dash/gap
  runs；每个 run 的连续顶点转成独立 `LineSegments` 对。
- dash period 与 guide 的有界长度绑定：`period = max(extent / 100, 5)`，dash = 0.6×period，
  gap = 0.4×period。这样极大模型也不会把一条 guide 变成数万段，且视觉上始终可辨。
- fragment 的 `linetype` 写该 pattern（`linetype_unresolved=false`）；没有产生 dash 的退化路径
  仍是连续线，但保持真实 pattern 标记，不谎称虚线。
- point guide 的三轴十字保持连续（点标记不是线）。

### D2 测试与文档

- 场景测试断言 line guide 批次有 >2 个顶点、每段长度 ≤ dash 长度、`linetype.elements` 非空；
  point 十字仍是 6 顶点连续批次。
- 没有新命令、没有格式档位变化；只是场景批次的呈现改变。

## 不在本 ADR 范围

- 用户可配置 dash 长度/颜色、屏幕空间恒定 dash、抗锯齿像素验证；
- 其它 guide 类型（角度圆弧/自动 point）与图纸空间。

## 后果

- guide 现在是分段虚线，不再是与普通构造线相同的连续线；point 标记不变。
- 仍是有界近似（不是真无限线），真实 GPU/像素证据仍未验证。
