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
  `ConfirmMeasurement` 命令**。测量本身不打开批注事务。
- **测量结果存为批注（F07）已接线**：确认（或自动完成）测量后记录保存在
  `SessionState::last_measurement`；`CommandId::SaveMeasurementAsAnnotation` +
  `CommandPayload::None` 把它转成 `AnnotationGeometry::Measurement`，走与其它批注
  **完全相同**的 `commit_annotation` 事务/历史路径（恰好一事务、一撤销步），并在
  命令层强制 Work 权限。无已确认记录时返回 `CadError::InvalidInput`，**不是静默
  成功**。`AnnotationGeometry::Measurement` 仍**刻意不画**在批注叠加层
  （`annotation.unsupported` 诊断），本轮只保存如实记录。

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
- 预览几何（把 `measurement_preview().points`/`cursor` 画到画布）**已接线到 CPU 场景
  叠加层**：`cad_app::render_scene::overlay::preview_overlay` 把
  `MeasurementPreview`/`AnnotationPreview` 转成 `RenderBatch`（折线 + 取点十字标记，
  光标到末点的橡皮筋），`CadView::set_measurement_preview` /
  `CadView::set_annotation_preview` 把它交给 `CadSceneController`。预览是**纯快照**：
  取消（传 `None`）即从叠加层消失，不写库、不开事务。宿主仍负责把画布点击映射为世界
  点（`set_canvas_pick_mapper`）；本轮只补齐“预览点 → 场景批次 → 桥接”这一段，
  **未**在真实 GPU/浏览器上验证像素结果。
- 预览叠加层 `draw_order` 为 `PREVIEW_DRAW_ORDER = 2_000_000`，高于批注
  （`1_000_000`）与选择高亮（`900_000`），保证工具反馈始终可见。

### 选择高亮（与预览同一条芯线）

- `cad_app::render_scene::overlay::selection_highlight` 按
  `drawing_pick_items` 展开 INSERT，把选中实体的几何离散成高亮批次；同一块的
  两个放置因 `InstancePath` 不同而独立高亮。解析失败的引用产出
  `highlight.unresolved` 诊断而不是伪造几何。
- `CadView::set_selection_highlight(SelectionSet)` 只推进**独立的** overlay 版本，
  不重建底图与批注（`cad-app` 单测 `selection_change_rebuilds_only_the_highlight_overlay`）。
- 高亮色为 `cad_scene::DEFAULT_HIGHLIGHT_COLOR`（暖色），`color_unresolved = false`；
  详见 `docs/panels.md` §3.3 与 `docs/picking-3d.md`。
- 快照里的 `kind` 也用于让「测量」按钮与下拉框选择保持一致，见
  `UiAdapter::selected_measurement_kind`。

### 查看/工作模式切换（U02）

- 会话模式是**权威**的 `cad_app::AppMode`（`SessionState::mode()`）。
  `CommandId::SetMode` + `CommandPayload::Mode(AppMode)` 经
  `SessionState::switch_mode` 切换：切换时取消未确认的工具（`ToolState::Idle`），
  绝不静默提交，也不写库、不记历史。
- `SetMode` 本身**不要求 Work 权限**（两种模式都能切回去）；但切到 Viewer 后
  所有 Work-only 命令仍在命令层返回 `PermissionDenied`
  （`CommandId::requires_work_mode()`）。
- 外壳入口：命令栏的 `mode-label` 按钮（回调 `mode-toggled`）派发
  `SetMode(相反模式)`，并立即把 `work-mode` 与 `mode-label` 更新到目标状态；
  宿主的权威推送（`UiHandle::set_mode(HostController::mode())`）会覆盖任何偏差。
- `mode-label` 文本来自目录：Work → `mode.enhanced`，Viewer → `mode.viewer`
  （两语言均有）。`UiHandle::set_work_mode` 现在同时写标志与标签；`set_locale`
  用最近一次模式重发正确的标签，不再回落到「增强」。
- 单测：`cad-app` 的 `set_mode_cancels_an_unconfirmed_tool_and_gates_work_commands`
  覆盖「切换取消工具 + Viewer 拒绝 Work-only + 切回恢复」；
  `cad-ui-slint` 的 `mode_ui_state_reflects_the_authoritative_mode` 覆盖标签/标志
  （该 crate 原生测试受本机 fontconfig 限制，见 §5）。

## 2.5 异步打开进度（F01）

### 已接线（核心 + UI）

- **纯快照**：`HostController::async_open_snapshot() -> Option<ImportProgressSnapshot>`
  是纯 getter。字段 `running` / `phase: Option<_>` / `entities_done` /
  `entities_total: Option<usize>` / `bytes: Option<u64>` / `cancellable` /
  `terminal: Option<ImportTerminal>` 全部来自 manager 的真实轮询，绝不猜测。
  `phase_key()` 输出稳定机器键（`reading`…），因此 `cad-ui-slint` 不依赖导入器 crate。
- `ImportProgressSnapshot::from_poll(&AsyncOpenPoll, previous)` 是 manager→UI 的
  唯一映射；`Running` 无新 tick 时保留上一次真实阶段/计数，终态被保留。
- `begin_async_open` 写入「无 tick 的运行态」（`phase=None`）；`cancel_async_open`
  置 `cancellable=false`；`poll_async_open` 只做投影，不重复 manager 判定。
- **取消命令路径**：`CommandId::CancelLoading` 在 `HostController::execute` 被拦截
  并转调 `cancel_async_open`（应用层原本返回 `Unsupported`）。适配器
  `on_cancel_open_requested` 派发 `CancelLoading`，Web/Android 的 `UiCommandSink`
  落到 `controller.execute` 后即生效，未改动既有调用者。
- **面板状态**：`ImportProgressUiState::from_snapshot(snapshot, messages)`：
  - `percent` 仅在 `entities_total == Some(>0)` 时给出，否则 `None` → 外壳以
    `import-indeterminate` 渲染不确定进度条，绝不显示假百分比；
  - `bytes` 仅在真实可测时拼入 `import-progress-text`，未知时**不**显示「0 字节」；
  - `Opened` → 面板隐藏；`Cancelled`/`Failed` → 显式终态文案且保持可见；
  - `cancellable` 启用/禁用取消按钮。
- `UiHandle::set_import_state(&state)` 一次写入外壳 `import-*` 属性；
  `set_locale` 用保留的原始快照按新目录重排文案（不会留下旧语言的阶段标签）。
- 目录键（两语言）：`import.panel`、`import.cancel`、`import.phase.*`、
  `import.progress.count`、`import.progress.indeterminate`、`import.progress.bytes`、
  `import.detail.separator`、`import.terminal.cancelled`、`import.terminal.failed`。

### 宿主接线（`apps/app-web`，本轮）

- `open_document_bytes*` / `OPEN` 经 `browser/async_open.rs::start_or_apply`：有线程的
  宿主启动后台任务，浏览器（`wasm32` 无线程）走同步导入并把**真实**终态
  (`ImportTerminal::Opened/Failed/Cancelled`) 经同一 `ImportProgressUiState` 映射推入
  面板。无伪造进度。
- 新 wasm 导出：`async_open_poll_json()`（轮询→至多发布一次→`install_opened`→
  `set_import_state`→稳定 JSON）与 `async_open_worker_available()`（诚实暴露线程能力）。
- 单一状态漏斗 `state_push::push_panel_state` 现在同时推 `set_import_state`，因此
  命令、打开、取消都会刷新面板；`cancel-open-requested` → `CancelLoading` →
  `cancel_async_open`，当前文档与未保存批注保留。
- JS：`web/host/renderer.js` 心跳每轮调用 `async_open_poll_json`（`async-open.js`
  纯解析），running 时收紧到 250ms；`window.yacrAsyncOpen` 暴露真实状态。

### 平台限制（精确）

- **浏览器看不到 running/cancellable 面板**：`cad-app` 的 worker 用 `std::thread`，
  `wasm32-unknown-unknown` 无线程，调用 `begin_async_open` 会 panic。浏览器只推真实
  **终态**面板（`Failed`/`Cancelled` 显式可见，`Opened` 隐藏）。详见
  `docs/import-async.md` 的线程限制一节。
- `apps/app-android` 未在本轮范围内接线。
- 本轮未在浏览器/真机做视觉渲染验证（`docs/validation-web.md` 记录）。

## 3. 宿主连接器（本轮范围外）

`cad-ui-slint` 只负责把状态推入外壳、把回调翻成命令；「应用状态 → 外壳」的
最后一段由宿主调用。本轮改动只限于 `cad-ui-slint`/`cad-app`，因此以下连接器
**尚未在宿主中调用**（`apps/` 不在范围）：

- `UiHandle::set_history_availability(HostController::history_availability())`：
  使撤销/重做两个按钮真正反映两个历史栈。
- `UiHandle::set_measurement_state(MeasurementUiState::from_preview(
  controller.measurement_preview(), controller.unit_label()))`：驱动测量面板。
  宿主还应调用 `state.set_can_save_annotation(controller.has_last_measurement())`
  以启用「存为批注」按钮（`HostController::save_measurement_as_annotation()`）。
- `UiHandle::set_mode(controller.mode())`：把 `work-mode` 与 `mode-label` 更新到
  权威会话模式（U02）。未调用时外壳仍可由 `mode-toggled` 乐观切换，但宿主推送是
  权威的。
- `UiAdapter::set_canvas_pick_mapper(...)`：把画布点击映射为世界点。
- `CadView::set_selection_highlight(controller.session.selection.clone())`：把选择集
  送入高亮叠加层。
- `CadView::set_measurement_preview(controller.measurement_preview())` /
  `CadView::set_annotation_preview(controller.annotation_preview())`：把工具预览
  送入叠加层（`None` 即取消）。

> 更新（见 §3.1）：Web 宿主 `apps/app-web` 现已调用上述**全部**连接器
> （历史/测量面板、画布映射、选择高亮与工具预览）；Android 宿主仍未接线。

未调用时行为是**降级而非假装**：重做按钮保持禁用、面板显示空步骤并禁用确认/
取消、测量点击提示「取点未接线」、选择/预览叠加层为空（不画假几何）。以上是本轮
明确交接给宿主接线任务的开放项。

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

**选择高亮与工具预览（已接线；无头验证待补）**：`push_panel_state` 现在是overlay
的**唯一**入口，命令执行、选择拾取、确认/取消都经它把
`CadView::set_selection_highlight(controller.selection().clone())`、
`set_measurement_preview(controller.measurement_preview())`、
`set_annotation_preview(controller.annotation_preview())` 送入渲染桥。空选择集推送
显式空 `SelectionSet`（高亮消失），无工具推送 `None`（预览消失），因此每条命令路径
都不会漏掉 overlay。纯映射抽为 `state_push::derive_overlay_push` 以便单测（见 §5）。

**预览光标（可选，已接线）**：`browser/input.rs` 的指针移动在**捕获工具活动**且非
拖动导航时，用与渲染同一套的 `pick::map_canvas_point` 求世界点，经
`SessionState::set_measurement_cursor`/`set_annotation_cursor` 写入并复用同一
state-push 漏斗；空闲/导航状态直接返回，不触碰底图。纯分类 `preview_cursor_for`
可单测（`PreviewCursor::{Measurement,Annotation,None}`）。

`apps/app-android` 仍未接线（不在本轮范围）。以上仅为源码接线与编译证据，
浏览器/真机验收待补，不构成视觉验收。

### 3.2 Android 宿主接线（选择/预览/布局切换）

`apps/app-android/src/state_push.rs::push_panel_state` 与 Web 同源：在启动、
打开图纸成功、命令执行、画布点按/拖动/缩放后统一推送面板状态，并在同一快照中把
**瞬态渲染叠加层**推入 `CadView`：

- `CadView::set_selection_highlight(snapshot.selection)`：选择集来自
  `HostController::selection()`；空选择推入空高亮（清除叠加层），命中则高亮真实
  实体，不伪造几何。
- `CadView::set_measurement_preview(snapshot.measurement_preview)` /
  `CadView::set_annotation_preview(snapshot.annotation_preview)`：来自
  `HostController::measurement_preview()` / `annotation_preview()`；无工具时传
  `None`，取消预览叠加层（命令确认/取消后同样回到 `None`）。
- 这些字段与面板一起放在纯 `PanelSnapshot` 中，因此宿主接线逻辑可在不打开 Slint
  窗口的情况下单测（`android_snapshot_*` 系列）。

**布局（图纸空间）切换走默认命令路径，未安装 `LayoutSwitchSink`**：适配器
`on_layout_selected` 在宿主未安装 sink 时派发
`CommandId::SwitchSpace` + `CommandPayload::Space(...)`；`HostSink::send` 将其交给
`HostController::execute`，后者按真实布局表校验（未知布局显式拒绝），随后
`sync_view_camera` 重同步相机与活动空间、`push_panel_state` 重推布局面板。
Android **刻意不安装** `LayoutSwitchSink`：一旦安装，适配器会改调 sink 而**不再**
发送 `SwitchSpace` 命令，反而绕过验证与相机重同步。宿主侧用
`android_layout_selection_routes_through_switch_space`（含切回模型空间、未知布局被拒）
断言该命令路径。

以上仅为源码接线与 Android 目标类型检查证据；**未在设备上复测**，不构成视觉验收。

## 4. 工具面板（U03）

新增一条紧凑的工具/状态栏（高度 40px），只显示真实状态：

- 测量算法下拉框（工作模式可用）；
- 确认/取消按钮（按 `measurement-active` / `measurement-can-confirm` 启用）；
- 「存为批注」按钮（仅在存在已确认测量记录时显示，
  `measurement-can-save-annotation`；无记录时命令层也会拒绝）；
- 步骤文案（来自状态机）；
- 单位标签（来自文档 `UnitContext`）。

命令栏还有一个模式切换按钮（`mode-label`，常显），显示当前真实模式并派发
`SetMode`（见 §2 后的「查看/工作模式切换」）。

面板不创建每实体控件，不显示假数据。

### 刻意未做（原因）

- **图层 / 属性 / 批注列表 / 布局 / 资源 / 3D 面板**：缺少可安全绑定的只读数据
  通道（F03/F05/F09/F04/F10/F13 的核心闭环本身未完成），现在加面板只能是空壳或
  假数据，违反 AGENTS.md 约束 2。等待对应功能接线后再加，不先建空 UI。
- **选择高亮/属性**（F05）：`Select` 目前只设置 `Selecting` 状态，没有精确拾取与
  属性查询接线，因此没有属性面板。
- **2D/3D、Orbit**：`Switch2d3d`/`Orbit` 仍是 `pending(...)`，本轮不涉及。

## 5. 无障碍边界与键盘可达（U12 / U08）

DOM 宿主对 Slint 画布**不宣称等价原生语义**。画布是一个不透明的
`<canvas>`，没有可按实体建模的 ARIA 结构；本轮只提供**明确的替代入口**，不实现
完整 ARIA canvas 模型（无逐实体 role、无网格导航）。

- **声明为应用区域**：`apps/app-web/web/index.html` 的 `#canvas-host` 标记
  `role="application"`、`aria-label`（目录键 `a11y.canvas_label`）与
  `tabindex="0"`，因此键盘 Tab 可到达并有可见焦点环（`style.css`
  `#canvas-host:focus-visible`）。这是“可达”，不是“语义完整”。
- **单一播报通道**：`#a11y-status` 是视觉隐藏的 `role="status"`
  `aria-live="polite"` `aria-atomic="true"` 区域（`.visually-hidden`，非
  `display:none`），由 `web/host/a11y.js` 经 `web/host/i18n.js` 的
  `setStateKey`/`setStateText` 漏斗更新。它与 `#host-state` **同一条节奏**
  （`renderer.js` 轮询），但输出的是面向屏幕阅读器的短句，而不是原始
  `renderer_state_report()` 文本。
- **避免重复播报**：`#host-state` 改为 `aria-live="off"`，只保留 `role="status"`
  语义；真正的播报只发生在 `#a11y-status`。失败时 live region 使用
  `a11y.status_failed`（不含 `adapter=…`/`error=Some(…)` 技术转储）。
- **U08 宿主状态条不再常驻压住 Slint 状态**：`#host-state` 是加载/失败专用行；
  轮询判定就绪时给 `<body>` 加 `renderer-ready`，CSS 规则
  `body.renderer-ready:not(:has(#retry-renderer:not([hidden]))) #host-state` 将其
  隐藏。失败时重试按钮显示，`:has` 守卫重新显示该行，错误不会被静默吞掉。
- **诚实边界（未做）**：未做真实屏幕阅读器测试（VoiceOver/NVDA），未做逐实体
  可访问导航、对比度实测或缩放实测；选择/工具的完整语义仍在 Slint 侧，DOM 只镜像
  **状态**（加载/就绪/失败与宿主人文状态），不镜像实体级选择或工具步骤。上述均为
  源码/契约证据，不构成无障碍验收。

## 6. 测试与证据边界

- 可测试的纯逻辑都放在 `cad-app`，并由 `cargo test -p cad-app` 覆盖：
  `MeasurementToolKind::{key,from_key,from_label,index,from_index,ALL}`、
  `MeasurementPreview::{can_confirm,status_line}`、`Viewport::screen_to_world`、
  `HostController::{measurement_preview,unit_label}`，以及新增的
  `render_scene::overlay::{selection_highlight,preview_overlay,overlay_fingerprint}` 与
  `CadSceneController::prepare_with_overlays` 的“选择变化只重建高亮叠加层”契约。
- `crates/cad-scene/src/highlight.rs` 的 `HIGHLIGHT_DRAW_ORDER` 已从 `2_000_000` 调整为
  `900_000`（高于底图 0、低于批注 1_000_000），其单测同步更新；`cad-scene` 单测通过。
- 本轮新增的 `cad-app` 单测：`save_measurement_as_annotation_uses_one_transaction`、
  `save_measurement_without_a_record_is_refused_and_writes_nothing`、
  `save_measurement_as_annotation_is_work_only`、
  `set_mode_cancels_an_unconfirmed_tool_and_gates_work_commands`、
  `set_mode_requires_a_mode_payload`，以及 `tests/contracts.rs` 的 Work-only /
  允许列表更新。`cad-ui-slint` 新增
  `measurement_save_affordance_requires_a_confirmed_record`、
  `mode_ui_state_reflects_the_authoritative_mode`、
  `shell_exposes_the_save_and_mode_switch_affordances`、
  `mode_labels_resolve_in_both_catalogs`。
- 本轮新增：`cad-app` 的 `snapshot_is_none_when_idle_and_never_fabricates_a_phase`、
  `snapshot_keeps_unknown_totals_unknown_and_real_totals_exact`、
  `snapshot_projection_retains_running_fields_and_maps_terminals`、
  `controller_snapshot_tracks_a_real_async_open_to_its_terminal`、
  `cancelling_via_the_command_path_flips_cancellable_and_reports_cancelled`、
  `failed_open_retains_an_explicit_terminal_snapshot`（覆盖不确定 vs 确定、
  终态、CancelLoading→cancel）。`cad-ui-slint` 新增
  `shell_exposes_the_async_open_progress_panel_and_cancel`、
  `import_ui_state_is_hidden_when_idle_or_opened`、
  `import_ui_state_never_fabricates_a_percent_or_a_byte_count`、
  `import_ui_state_reports_cancelled_and_failed_terminals_explicitly`、
  `import_phase_keys_all_resolve_in_both_catalogs`。
- `cad-ui-slint` 中 `MeasurementUiState` 与外壳定义字符串的测试位于该 crate 的
  `#[cfg(test)]`，但**本机无法构建 `cad-ui-slint` 测试**（宿主缺 fontconfig），
  因此这些测试本轮未执行；`cargo check --workspace --lib --target
  wasm32-unknown-unknown` 是 `cad-ui-slint` 的编译门，且已通过。
- Slint 渲染、窗口事件循环、真实指针映射、GPU 合成**本轮均未运行**；上述仅为
  源码接线与编译证据，不构成视觉/真机验收。
- DOM 无障碍契约（U12/U08）由 `node --test scripts/test-web-a11y.mjs` 覆盖：
  断言 `#a11y-status` 存在且 `aria-live="polite"`、`#host-state` 非 polite、画布
  `role="application"`+`aria-label`+`tabindex`、`.visually-hidden` 使用 clip 而非
  `display:none`、CSS 在就绪后隐藏 `#host-state`、播报漏斗去重且失败不夹带原始
  report、轮询就绪时加 `renderer-ready`。该测试不需要 wasm/浏览器。
