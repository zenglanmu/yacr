# Slint 外壳接线状态（U03 / U04 / U11）

本文件记录 `crates/cad-ui-slint` 与 `crates/cad-app` 之间已接线的 UI 契约，以及
**刻意未接线**的部分与原因。规范权威是 `CAD_IMPLEMENTATION_SPEC.md` v2.0；交互
状态机细节见 `docs/interaction.md`，需求追踪见 `docs/requirements.md` 与
`docs/code-audit-and-agent-handoff.md`（F05/F06/U03/U04/U11）。

本文件只描述展示层边界：`cad-ui-slint` 负责把应用状态推入 Slint 组件，并把
Slint 回调翻译成 `Command`；它不重实现测量/历史算法。

## 1. 撤销/重做（U11）

- 外壳 `ui/app.slint` 有**两个独立**属性：`can-undo` 与 `can-redo`。
  - 撤销按钮绑定 `can-undo`，重做按钮绑定 `can-redo`（不再共用 `can-undo`）。
- `UiHandle::set_can_undo` / `set_can_redo` 分别设置；`UiHandle::set_history_availability`
  接收一个 `cad_app::HistoryAvailability` 快照，一次写入两个标志。
- 快照来源：`HostController::history_availability()` → `Application::history_availability()`，
  它读取 `History` 的 undo/redo 两个栈。撤销到空后 `can_undo=false` 而
  `can_redo=true`（`cad-app` 单测 `undo_to_empty_still_allows_redo_and_refresh_is_pure`）。
- 该 getter 是纯读取，刷新按钮状态不派发命令，因此不会产生重复撤销/重做命令。

## 2. 测量工具（F06 / U04）

### 已接线

- **算法选择**：面板下拉框（距离/折线长度/角度/面积）的 `measure-kind-selected`
  回调把选中的标签交给 `MeasurementToolKind::from_label`（`cad-app` 权威标签），
  随后派发 `CommandId::Measure` + `CommandPayload::MeasureTool(kind)`，立即启动
  该算法工具。外壳不再发送无参猜测。
- **测量按钮**：`measure-requested` 派发当前下拉框选中的算法，而不是
  `CommandPayload::None`。
- **确认/取消**：面板「确认」派发 `ConfirmMeasurement`（仅在
  `measurement-can-confirm` 时可用）；「取消」派发 `CancelMeasurement`。
- **预览/步骤**：`MeasurementUiState::from_preview` 把
  `HostController::measurement_preview()` 展平为面板状态（是否活动、是否可确认、
  当前算法、步骤文案、单位标签）；`UiHandle::set_measurement_state` 一次写入。
  步骤文案由 `MeasurementPreview::status_line()` 生成，计数规则留在 `cad-app`。
- **单位上下文**：`HostController::unit_label()`；未知单位显示 "drawing units"，
  不默认成 mm。

### 命令语义

- 测量是只读操作：`ConfirmMeasurement` 返回结构化 `MeasurementRecord`，
  `changes: None`，**不写库、不记历史**；`CancelMeasurement` 只回到 `Idle`。
  因此「取消 = 零事务」由构造保证（`cad-app` 单测
  `cancelled_measurement_produces_zero_transactions`）。
- 「一次确认一事务」在本 UI 路径中的落点是：**每次确认恰好派发一条
  `ConfirmMeasurement` 命令**。测量本身不打开批注事务；把测量结果存为批注
  （F07）是后续独立工作，未在本轮接线。

### 画布取点（诚实开放项）

- 外壳在左键单击画布时触发 `canvas-pick(x, y)`（`TouchArea` 的 up 事件）。
- 只有当测量工具**处于活动状态**（`measurement-active`）时，点击才会被当作
  取点；普通导航点击保持沉默，不会误报。
- `UiAdapter` 只有在宿主通过 `UiAdapter::set_canvas_pick_mapper` 安装了
  `CanvasPickMapper`（逻辑像素 → 世界点）时才把它转成
  `CommandId::Measure` + `CommandPayload::Points([world])`。
- **当前两个宿主（`apps/app-web`、`apps/app-android`）尚未安装 mapper**，因为
  宿主接线不在本轮范围内。此时测量中的点击会设置明确的状态文案
  「取点未接线：宿主未提供画布→世界映射」，**不是静默丢弃**。
- 宿主应使用 `cad-app` 的纯函数 `Viewport::screen_to_world(logical, canvas_size)`
  求点（已有单测：中心映射到相机 target、角点对称、退化输入返回 `None`）。
- 预览几何（把 `measurement_preview().points`/`cursor` 画到画布）**尚未接线**：
  `bridge.rs` 没有消费 `measurement_preview()`，本轮只把预览作为面板状态展示。
  绘制预览需要宿主把预览点经场景/GPU 管线提交，属后续工作。
- 快照里的 `kind` 也用于让「测量」按钮与下拉框选择保持一致，见
  `UiAdapter::selected_measurement_kind`。

## 3. 宿主连接器（本轮范围外）

`cad-ui-slint` 只负责把状态推入外壳、把回调翻成命令；「应用状态 → 外壳」的
最后一段由宿主调用。本轮改动只限于 `cad-ui-slint`/`cad-app`，因此以下连接器
**尚未在宿主中调用**（`apps/` 不在范围）：

- `UiHandle::set_history_availability(HostController::history_availability())`：
  使撤销/重做两个按钮真正反映两个历史栈。
- `UiHandle::set_measurement_state(MeasurementUiState::from_preview(
  controller.measurement_preview(), controller.unit_label()))`：驱动测量面板。
- `UiAdapter::set_canvas_pick_mapper(...)`：把画布点击映射为世界点。

未调用时行为是**降级而非假装**：重做按钮保持禁用、面板显示空步骤并禁用确认/
取消、测量点击提示「取点未接线」。以上是本轮明确交接给宿主接线任务的开放项。

### 3.1 Web 宿主接线（已接线；无头/真机验收待补）

`apps/app-web` 现在通过 `browser/state_push.rs::push_panel_state` 在每条命令执行后、
打开文档后、批注导入/导出/恢复后统一推送：历史可用性（撤销/重做分别由
`HostController::history_availability()` 驱动，撤销到空后 `can_redo` 不再陈旧）、
测量面板、图层面板 + 有序 `LayerId`、属性面板、批注面板 + 有序 `AnnotationId`、
布局面板 + 有序 `LayoutId`、诊断抽屉。空/多值文案来自 `MessageSource` 目录（新增
`annotation.empty`）。

画布取点：`browser/pick.rs::WebCanvasPickMapper` 经
`UiAdapter::set_canvas_pick_mapper` 安装，使用 `ViewMetrics`/`Viewport::screen_to_world`
与 `UiHandle::cad_surface_size()`；退化输入返回 `None`（不伪造点）。无捕获工具时的
画布轻触走 `cad_app::pick_at_screen`（`TolerancePolicy::default()`、
`BackFacePolicy::Cull`），派发 `Select` + 命中 `SelectionRef`（空 vec 清空选择），
结果经属性面板呈现；超过共享拖动阈值的拖动不触发选择。

诊断抽屉数据经 wasm 导出 `diagnostics_report_json()`（`encode_model_redacted`）与
`window.yacr.diagnostics_report` 暴露；未导入报告前为显式空模型。

`apps/app-android` 仍未接线（不在本轮范围）。以上仅为源码接线与编译证据，
浏览器/真机验收待补，不构成视觉验收。

## 4. 工具面板（U03）

新增一条紧凑的工具/状态栏（高度 40px），只显示真实状态：

- 测量算法下拉框（工作模式可用）；
- 确认/取消按钮（按 `measurement-active` / `measurement-can-confirm` 启用）；
- 步骤文案（来自状态机）；
- 单位标签（来自文档 `UnitContext`）。

面板不创建每实体控件，不显示假数据。

### 刻意未做（原因）

- **图层 / 属性 / 批注列表 / 布局 / 资源 / 3D 面板**：缺少可安全绑定的只读数据
  通道（F03/F05/F09/F04/F10/F13 的核心闭环本身未完成），现在加面板只能是空壳或
  假数据，违反 AGENTS.md 约束 2。等待对应功能接线后再加，不先建空 UI。
- **选择高亮/属性**（F05）：`Select` 目前只设置 `Selecting` 状态，没有精确拾取与
  属性查询接线，因此没有属性面板。
- **2D/3D、Orbit**：`Switch2d3d`/`Orbit` 仍是 `pending(...)`，本轮不涉及。

## 5. 测试与证据边界

- 可测试的纯逻辑都放在 `cad-app`，并由 `cargo test -p cad-app` 覆盖：
  `MeasurementToolKind::{key,from_key,from_label,index,from_index,ALL}`、
  `MeasurementPreview::{can_confirm,status_line}`、`Viewport::screen_to_world`、
  `HostController::{measurement_preview,unit_label}`。
- `cad-ui-slint` 中 `MeasurementUiState` 与外壳定义字符串的测试位于该 crate 的
  `#[cfg(test)]`，但**本机无法构建 `cad-ui-slint` 测试**（宿主缺 fontconfig），
  因此这些测试本轮未执行；`cargo check --workspace --lib --target
  wasm32-unknown-unknown` 是 `cad-ui-slint` 的编译门，且已通过。
- Slint 渲染、窗口事件循环、真实指针映射、GPU 合成**本轮均未运行**；上述仅为
  源码接线与编译证据，不构成视觉/真机验收。
