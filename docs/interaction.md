# 交互工具状态机与撤销/重做可用状态

本文件记录 `cad-app` 内与 U04（工具状态机/预览/取消）和 U11（撤销/重做）
相关的接线契约。规范权威仍是 `CAD_IMPLEMENTATION_SPEC.md` v2.0；
需求追踪见 `docs/requirements.md`（F06/F08）与
`docs/code-audit-and-agent-handoff.md`（F06/U04/U11）。

## 1. 测量工具（F06 / U04）

`crates/cad-app/src/measure_tool.rs` 提供真正的测量工具状态机：

- `MeasurementToolKind`：`Distance` / `PolylineLength` / `Angle` / `Area`。
  - `algorithm()` 显式给出对应 `MeasurementAlgorithm`，**不再按点数猜测**。
  - `exact_points()`：距离=2、角度=3；折线长度与面积是开放端点（≥2 / ≥3）。
  - `auto_completes()`：距离/角度在点数足够时自动完成；折线/面积需显式确认。
- `MeasurementTool`：跨多条命令累积点、维护 `cursor` 预览，`preview()`
  返回 `MeasurementPreview { kind, points, cursor, remaining, ready }`。
- `ToolState::Measuring(MeasurementTool)`：会话持有活动工具。

命令入口（`Application::execute`）：

| 命令 | 载荷 | 行为 |
|---|---|---|
| `CommandId::Measure` | `MeasureTool(kind)` | 启动/重启指定算法工具，返回预览诊断 `measure.preview` |
| `CommandId::Measure` | `Points(points)` | 向活动工具追加点；自动完成的工具在此提交并回到 `Idle` |
| `CommandId::Measure` | `None` | 启动默认「距离」工具（兼容旧 UI 只发 None），不再直接报错 |
| `CommandId::ConfirmMeasurement` | `None` | 确认开放端点工具（折线/面积）；点数不足返回 `InvalidInput`，状态保留 |
| `CommandId::CancelMeasurement` | `None` | 取消工具；**不产生任何事务** |
| `CommandId::Measure` | `Points(points)`（无活动工具） | 保留旧的统计推断路径，供 CLI/直接调用方使用 |

结构化结果：`CommandOutcome` 新增 `measurement: Option<MeasurementRecord>`，
提交成功时携带引擎返回的 `MeasurementRecord`（algorithm/inputs/plane/value/
units/source/precision），而不是只给格式化字符串。

面积测量使用**视口工作平面**（`MeasurementSpace::Plane`），因此非共面点会被
引擎拒绝（审计 B24），不会被静默压平；距离/折线/角度仍走 `World3d`。

取消语义：测量是只读操作，`CancelMeasurement` 只调用 `SessionState::cancel_tool`，
不写库、不记历史，因此取消恒为「零事务」（规格 §4.10）。
预览更新 `set_measurement_cursor` 是纯状态更新，不发命令。

### 仍未接线（诚实开放项）

- Slint 外壳 `crates/cad-ui-slint/ui/app.slint` 的「测量」按钮仍发送
  `CommandPayload::None`；按钮已能启动距离工具，但**点选/确认/取消与算法选择
  尚未接入 UI**（U04 的 UI 侧仍待完成）。
- 宿主（Web/Android）尚未把画布指针映射为测量点或预览几何；`PreviewState`
  还没有消费 `SessionState::measurement_preview()` 来绘制预览。
- `Switch2d3d` / `Orbit` 仍为 `pending(...)`，本任务不涉及 3D。

## 2. 撤销/重做可用状态（U11）

- `Application::can_undo` / `can_redo` 分别读取 `History` 的 undo/redo 栈，
  二者独立；撤销到空后 `can_undo=false` 而 `can_redo=true`。
- `Application::history_availability(document) -> HistoryAvailability`
  与 `HostController::history_availability()` 是**纯 getter**，用于刷新按钮
  状态，绝不派发命令，避免程序化刷新触发重复命令。
- UI 约定：撤销按钮绑定 `can_undo`，重做按钮绑定 `can_redo`，不得共用
  `can-undo`（`app.slint` 的 `can-redo` 属性与 `UiHandle::set_can_redo`
  由 UI crate 侧完成，见交接说明）。
