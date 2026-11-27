# ADR-0068：量角器（Protractor）

- 状态：**已接受（决策，2026-11-27）**；**已实现（2026-11-27，三点角度测量 + `GUIDE` 具名拒绝；无真实 GUI/GPU 证据）**
- 关联：ADR-0015（交互捕获）、`cad-measure` 的 `Angle3Points`、ADR-0066/0067（同属“工具不再 pending”）

## 背景

SketchUp 的 Protractor 是一个三点角度量具：点顶点、再各点一条射线上的点，显示角度；第二个模式
（Ctrl/Alt/方向键）会同时创建一条按该角度旋转的 **guide line**（参考线）。

本仓库已有：

- `MEASUREANGLE` 等五个 measurement selector（没有 `CommandId`）→ `BeginMeasurement(<kind>)`；
- `MeasurementToolKind::Angle` → `MeasurementAlgorithm::Angle3Points`，引擎在**第二个**点处测角
  （`a, v, b`），结果以**度**记录并由 `UnitsDisplay::format_angle` 格式化；
- 完整的测量预览/状态行/一次性自动提交路径（`MeasurementPreview`、`auto_ready`、`confirm_measurement`）；
- 无 guide 模型：文档没有参考线实体，也没有 guide 表。

所以 Protractor 的“测量”部分可以完全复用既有引擎，唯一的新语义是**捕获顺序**（顶点先点）和
**guide 模式的诚实拒绝**。

## 决策

### D1 Protractor 是一个 measurement kind，不是新命令

- 新增 `MeasurementToolKind::Protractor`（追加在 `ALL` 末尾，保持索引稳定）：key `protractor`、
  标签 ASCII `PROTRACTOR`、算法 `Angle3Points`、3 点、一次性自动提交。
- `MEASUREMENT_SELECTORS` 增加 `("PROTRACTOR", Protractor)`：它在 `admissible_names` 里被广告、
  由 `cad_commands` 下发、没有 `CommandId`，与 `MEASUREANGLE` 同一路由
  （`BeginMeasurement(Protractor)`）。
- 捕获顺序是 SketchUp 的**顶点 → 第一条射线 → 第二条射线**。引擎在第二个点测角，因此
  `MeasurementToolKind::engine_points` 是唯一把捕获顺序映射为引擎顺序
  （`ray, vertex, ray`）的地方；预览与记录里保存的 picks 不被改写，测量记录的 `inputs`
  明确是引擎顺序。
- 状态行按步提示 `protractor: pick the angle vertex / a point on the first ray / second ray`
  （locale-neutral ASCII，避免扩大 Rust 侧 CJK 债务）；结果仍由既有 `measure.result`
  诊断与单位格式化输出角度。

### D2 Guide 模式：具名拒绝

`PROTRACTOR GUIDE` 是唯一允许的参数形式，且只用于拒绝：
`protractor.guide_not_implemented: guide-line creation is not modeled; the protractor measures
the angle only`。它不启动捕获、不写文档；其它参数按 `unsupported CAD command` 拒绝。
这样用户得到的是“这个模式没有实现”的稳定原因，而不是“命令不存在”或静默只测角。

### D3 UI

概念域工具栏与 Draw 菜单的 Protractor 从 `pending` 变成 `command: 'PROTRACTOR'` 的真实工具，
带提示键（说明只测角、guide 未建模）；catalog 增加 `PROTRACTOR` 行（`measure.kind.protractor`）。

## 不在本 ADR 范围

- guide/参考线实体、guide 表、按角度旋转的参考线、`FOLLOWME` 用 guide 作路径；
- 在 3D 工作平面上的任意朝向角度（沿用既有测量引擎的平面/空间规则）；
- 角度吸附、增量角（15°）输入、拖动修改角度；
- 真实 GUI/GPU 交互与其它载体运行时。

## 后果

- 工具目录与测量选择子各增加一项（`MeasurementToolKind::ALL` 5 → 6，选择子一一对应）；
- 没有新命令、没有新格式档位、没有新文档表；测量是只读的，Viewer 合法；
- `engine_points` 是捕获顺序与引擎顺序的唯一接缝，任何新的“不同捕获顺序”工具都应走同一模式。
