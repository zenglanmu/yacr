# 输入、DPI、IME 与状态策略（U05/U06/U07/U08）

本文件记录 `crates/cad-app/src/input.rs` 内**可测试的共享策略**：单指绘制 vs
双指导航、键盘/IME 焦点、画布矩形→世界坐标映射、状态栏与诊断抽屉模型。
规范权威仍是 `CAD_IMPLEMENTATION_SPEC.md` v2.0；需求追踪见
`docs/requirements.md`（F02/F13）与 `docs/code-audit-and-agent-handoff.md`
（U05/U06/U07/U08）。

这些是**纯策略**：不做平台事件接线、不创建 GPU/窗口资源、不发命令。宿主
（Slint/Android/Web）把平台事件翻译成这里的输入类型，再把 `InputOutcome`
翻译成 `Command`。策略在这里集中，是为了让每个宿主不会各自发明一套略有
偏差的答案。

## 1. U05 指针策略（`InputPolicy` / `PointerPhase`）

指针状态机对按键数决定“绘制还是导航”，带拖动阈值与 pointer cancel 处理：

```
Idle ──1 触点按下──▶ Pending ──移动 ≥ 阈值──▶ Dragging
  ▲                     │                        │
  │   pointer cancel     │ 第 2 触点             │ 第 2 触点
  └─────────────────────┴──────────▶ Navigate ◀──┘
```

- **拖动阈值** `DRAG_THRESHOLD_LOGICAL_PX = 4.0`（逻辑像素）。低于阈值只移动
  预览；达到阈值才捕获锚点一次，之后只移动预览。鼠标“按下即抬起”是零长度
  拖动，直接在抬起点提交。
- **触点语义**：`PointerUpdate { logical_position, contacts }`。`contacts==0`
  是抬起，`1` 是单指/鼠标，`≥2` 进入导航。
- **单指**：`Idle` 按下 → `Pending`（`Preview`）；越阈值 → `Commit(anchor)`
  且转 `Dragging`；抬起 → `Commit(anchor)`。导航中剩下的单指抬起**不会**回
  到绘制（`Ignored`）。
- **双指**：进入 `Navigate`，返回 `Pan { delta }`（逻辑像素增量）；若此刻有
  进行中的绘制，先返回 `ToolCancelled { MultiTouch }`。**硬规则：多指手势
  永不提交批注/测量**，会话工具由宿主据此调 `cancel_tool`。
- **pointer cancel**：`pointer_cancelled()` 退出到 `Idle`，若有进行中手势返回
  `ToolCancelled { PointerCancelled }`，否则 `Ignored`。
- 非有限坐标一律 `Ignored`，不污染状态机。

`EscapeAction`（`escape_action`）：Esc/Android 返回**先取消工具，再处理退出**；
若文本框/IME 持有焦点则不碰画布工具，直接返回 `Exit` 让文本框或宿主处理。

## 2. U06 键盘/IME 策略

`InputPolicy` 记录两件事：`text_focus`（文本框/弹窗获得焦点）与 `composing`
（IME 组合中）。

- `shortcuts_suppressed()`：`text_focus || composing`。为真时画布快捷键全部
  暂停。
- `set_text_focus(false)` 同时清除 `composing`，避免一次被取消的组合串泄漏到
  之后的快捷键。
- `classify_key(policy, is_escape)` 返回 `Suppressed / Escape / Shortcut`。
  组合输入**永不被当成命令**：例如拼音预编辑里的 “p” 不会触发平移。

## 3. U07 视图度量（`CanvasMetrics` / `ViewMetrics`）

单一映射：画布矩形 → 逻辑像素 → 物理像素 → 世界坐标。

- `CanvasMetrics { origin_logical, size_logical, dpi_scale }`：`origin_logical`
  是画布左上角相对宿主表面的逻辑像素偏移（工具栏/安全区占用后**通常不是
  整个窗口**）。`is_valid()` 显式拒绝退化值（尺寸 ≤0/非有限、DPI ≤0/非有限）。
- `surface_to_canvas_logical`：表面逻辑点 − 画布原点。
- `canvas_to_physical`：画布逻辑点 × `dpi_scale`。
- `surface_to_world`：表面逻辑点 → 世界点，复用 `Viewport::screen_to_world`
  （camera 的 `screen_to_plan_world` + 工作平面），因此**绘制与拾取走同一公式**。
  非平面视图返回 `None`，不伪造点。
- `world_per_px()`：取自同一 viewport，供测量吸附容差换算。
- `pan_delta_world(physical_delta)`：物理像素先除以 `dpi_scale` 回到逻辑像素，
  再乘 `world_per_px`；渲染器按逻辑像素工作，避免重复计入 DPI。
- `apply_canvas_metrics(&mut viewport, &canvas)`：resize/旋转时只更新
  `logical_size` 与 `dpi_scale`，不动相机，使视图中心保持不动；退化度量返回
  `InvalidInput`。

关键不变量：同一**逻辑**表面点在 DPR=1/2/3 下映射到**相同世界点**（物理像素
不同）；画布原点被真正扣除，全窗口尺寸与画布尺寸不再混用。

## 4. U08 状态模型

- `StatusModel { loading, tool, units, unsaved }`：状态栏只要这几项。
  `summary_line()` 输出固定文案（如 `就绪 · 测量 · mm · 未保存`），**绝不**
  内嵌首条诊断消息。
- `DiagnosticsDrawer`：包装 `cad-diagnostics::DiagnosticsModel`，保留**每个
  对象、每条原因**（不塌缩成第一条），提供 `summary()`、`objects_with_reasons()`、
  `reason_count()`、`has_findings()`；另有 `backend`（实际后端）与
  `recovery_actions`（可恢复动作码，UI 本地化）。最严判决胜出：一个完整对象不
  能掩盖缺失对象。

## 5. 尚未完成（诚实边界）

- **平台事件接线**：`cad-ui-slint` / Android `ViewInput` / Web 指针与键盘事件
  尚未调用本策略；多触点身份、pointer capture、长按语义需各宿主接入并补集成
  测试。
- **真实 IME/DPI 运行**：中文组合、软键盘高度变化、浏览器缩放、横竖屏切换、
  DPR=1/2/3 真机验证尚未执行；当前只有纯逻辑单元测试。
- **安全区**：`safe_insets`（`cad-ui-slint::UiConfiguration`）尚未并入
  `CanvasMetrics`，宿主需给出扣减安全区后的画布矩形。
- **上下文菜单**：长按/右键菜单策略未在本模块建模。
- **状态来源**：`StatusModel` 目前由调用方提供 `units`/`unsaved`/`loading`，
  尚未从 `host.rs` 的完整会话一次性派生。
