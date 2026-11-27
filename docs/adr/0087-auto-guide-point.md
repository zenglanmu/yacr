# ADR-0087：DIST GUIDE 自动参考线点（Coincident picks）

- 状态：**已接受（决策，2026-11-28）**；**已实现（2026-11-28，重合点写 point；无真实 GUI/GPU 证据）**
- 关联：ADR-0084/0085（guide 表、point/line）、`DIST GUIDE`

## 背景

`DIST GUIDE` 用两个拾取点定义一条 line guide。两点重合时没有方向，ADR-0084/0085 的
`guide.degenerate` 会拒绝整条命令，测量结果也不保留。SketchUp 的 Tape Measure 在同一位置
留下的是**点**参考线，本切片按这个语义处理重合点。

## 决策

- `add_measurement_guide(DistanceLine)`：两点距离 ≤ `1e-9` 时写 `Guide::Point(start)`，否则
  走 `GuideLine::through`。测量本身仍按 0 距离返回，命令成功。
- Protractor 的退化方向仍按 `guide.degenerate` 拒绝（它的语义是从顶点沿第二射线，没有“重合点”
  的等价解释）。
- 不改变表格式/档位：point 行已是 minor 27；不新增命令/UI。

## 后果

- `DIST GUIDE` 不再在重合点上失败，而是留下与显式 `GUIDE POINT` 相同的点标记。
- 剩余边界：point 自动捕获只在两点重合时发生；没有其它自动 point 规则或屏幕空间样式。
