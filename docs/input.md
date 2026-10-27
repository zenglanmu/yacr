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

### 1.1 宿主线载荷与点选映射（`cad-ui-core` 会话层；ADR-0012 §10 S1）

`crates/cad-ui-core/src/protocol.rs` 的 `ProbeInput` 是宿主→会话的**线载荷**；`crates/cad-ui-core/src/session.rs`
的 `CadSession` / `plan_input` 把它映射为类型化 `InteractionRequest`：

- **`button` 字段（可选）**：编码沿用 DOM `MouseEvent.button` 数值，公开常量 `BUTTON_PRIMARY = 0`、
  `BUTTON_MIDDLE = 1`、`BUTTON_SECONDARY = 2`（`protocol.rs`）。字段带 `#[serde(default)]`：**缺省该成员或
  显式 `null`** 均表示主键，使不报按键的旧调用方保持既有主键行为。
- **不做猜测**：其余任何值——**包括 DOM 在 `pointermove`/滚轮上报告“无按键变化”的 `-1`**——一律**拒绝**，稳定
  原因字符串为 `unsupported pointer button: N (expected 0, 1 or 2, or omit it for primary)`，绝不静默当作主键。
  因此**无按键变化的事件必须省略 `button` 字段**（`input.mjs` 的 `pointermove`/滚轮省略该字段），而非上报 `-1`。
- **点选 vs 拖拽判定**：`CadSession` 保留一次按下 `PointerPress { button, down, dragged }`；`pointer_move`
  优先采用按下时的按键，并在一经离开按下点 4.0 逻辑像素的圆盘后把该次按下锁存为 `dragged`
  （`DRAG_THRESHOLD_LOGICAL_PX`，`crates/cad-app/src/input.rs:77`；离去再返回仍算拖拽，故不会伪造点击）。
- **点击（select）**：`pointer_up` 的按键与按下**同一主键**、期间**未拖拽**且位移 `< 4.0` 逻辑像素时，发出
  **两个**请求——普通的 `Pointer{Up}` 释放，**以及** `InteractionRequest::SelectAt { point: 抬起点, mode: Replace }`，
  携带**当前** `metrics_revision`（陈旧 revision 由宿主显式拒绝，不臆造通过）。因此**在空白处点击即清空选择**。
- **其余情况**只发出普通释放：中键/右键抬起**永不**选择；左键拖拽与中键拖拽**都**平移（pan）。
- **框选未实现**：左键拖拽仍只是平移，Window/Crossing 框选归 ADR-0012 §10 item 5（S5）。
- **状态原子性**：按下状态**仅**在每个请求均被接纳后才提交，故被拒绝的输入不会留下幽灵按下。

#### 1.1.1 当前线载荷 kind 集合

`ProbeInput.kind`（`InputKind`）当前恰好是下列 **8** 个值，线字符串即其 snake_case 名：

`pointer_down`、`pointer_move`、`pointer_up`、`pointer_cancel`、`pointer_leave`、`wheel`、`resize`、`blur`。

其中**终态（terminal）** kind 有三个——`pointer_up`、`pointer_cancel`、`blur`（结束输入 marker/序列）；
其余为非终态，`pointer_leave` 见下（ADR-0012 §10 S2）。**历史 ADR-0006 的 kind 列表已过时**，本文件才是
当前线载荷契约；本仓库**不**回改已废弃的 ADR（ADR-0010 已取代 ADR-0006）。

#### 1.1.2 空闲移动 → hover（`MoveCursor`；ADR-0012 §10 S2）

- **空闲移动额外请求**：`plan_input` 的 `PointerMove` 分支在**无按下**（空闲）时，除普通的
  `Pointer{Move}` 外，**额外**发出一个 `InteractionRequest::MoveCursor(PointInput::CanvasLocal { logical_position,
  metrics_revision })`（`crates/cad-ui-core/src/session.rs`）。携带**当前** `metrics_revision`（陈旧 revision 由宿主显式拒绝）。
  **有按下**的移动保持旧的单请求形态（不产生 hover）。
- **语义**：这是**实体级、单个** hover——宿主用与**点选完全相同**的拾取路径（`cad_app::picking::pick_at_screen`，
  `TolerancePolicy::default()` + `BackFacePolicy::Cull`）解析，只取**排名第一**的 ref 存进宿主本地
  `cad_host::HoverState`；**无**新容差、**无**新排序。命中即点亮该实体的 `FLAG_HOVERED`（经 `OverlayInputs.hovered`），
  未命中的一次移动**立即清除** hover（**绝不**留下陈旧着色）。
- **抑制与清除**：按下/导航/捕获**拥有**指针时 hover 被抑制并在按下开始、`Cancel`、`ClearCursor`、pointer cancel、
  `blur` 与**未命中**时清除。拾取**报错**时先清 hover，再把错误作为**显式拒绝**向上传播（不静默空成功）。
- **节流规则（无时间窗）**：hover **没有**时间窗节流。它依赖 UI 既有 **rAF 合并**（`input.mjs` 把一次移动并入一帧的
  `pointer_move`），宿主在同一逻辑位置**未变**时**整段跳过**拾取查询（`HoverState.last_query`）。这是**行为契约**
  （以查询计数这类可观测量表述），**不是**频率/性能承诺。
- **分类为 overlay-only**：hover 的 `MoveCursor` 在宿主侧归 `DirtyScope::Overlay`（`crates/cad-host/src/input.rs`），
  故逐帧 hover **不会**触发 O(layers+entities) 的 `HostState` 重建。
- **用户可见修正（行为变更）**：此前会话**从不**发出 `MoveCursor`，故在绘制/测量**捕获进行中**的空闲移动**不**到达
  既有的工具光标分支；现在这类移动会到达（并由捕获拥有），工具预览光标因此跟随指针。这是**用户可见的修复**，
  并非改动 `move_tool_cursor` 语义。

#### 1.1.3 `pointer_leave`：离开表面（瞬态、非终态）

- **线字符串**恰为 `pointer_leave`（`crates/cad-ui-core/src/protocol.rs`）。
- **非终态**：它是**瞬态**事件，**刻意不**结束输入 marker/序列——离开表面是暂时的，指针可能**重新进入**并继续；
  `InputTracker::apply` 对它做**保状态空操作**（sequence 仍被消费，marker/press/dirty 均不变）。
- **映射**：会话把 `PointerLeave` 映射到**既有**的 `InteractionRequest::ClearCursor`；它**不携带**坐标、**不要求**
  revision。
- **不扰动进行中的按下**：`ClearCursor` **不动**进行中的按下（其后的 `PointerUp` 仍按常规解析）。React 侧亦仅在
  **无按下**时上报该 kind（见下）。
- **宿主分类**：`cad-host` 中，`ClearCursor` 在**无捕获**时为 **overlay-only**（与 hover 的 `MoveCursor` 同构）；
  仅当有绘制/测量**捕获**激活时才保持 `Full`（因为清除捕获光标会改变 `HostState` 里的预览）。

#### 1.1.4 React 侧 flush-before-leave 顺序（必须）

`apps/desktop-ui/src/cad/input.ts` 的 `leave` 处理器（绑定 `pointerleave`）在**无按下**时才压入 `{ kind: 'pointer_leave' }`
（载荷**恰为** kind + lane `sequence`，**无**位置/按键/metrics，**不** `preventDefault`），且**必须先 flush**
任何仍待发的 rAF 合并移动，正如 `down`/`up`/`wheel` 一样。**为何必须**：若合并移动留到**下一帧**才 flush，
它会在 leave **之后**到达宿主——宿主先因 leave 清掉 hover，又据一个**已离面的陈旧位置**重新点亮 hover。
这是与早期 `button: -1` 同类的**跨 lane 对账缺陷**：两侧各自单测都不会暴露，**仅靠跨 lane 阅读两处源码**
才发现。

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
- `inset_canvas_metrics(surface_origin, surface_size, safe_insets, dpi)`：把安全区
  `[top, right, bottom, left]`（逻辑像素）折入表面矩形，得到**扣减安全区后**的画布
  原点与尺寸，DPI 原样透传。非有限/负 inset、非正表面尺寸、inset 大于表面（画布为
  空）一律返回 `None`——**不裁剪、不臆造**。三个宿主据此给出画布矩形：Android 从
  `set_surface_insets` 入口接收系统栏/软键盘 inset，Web 由 JS 读取 CSS
  `env(safe-area-inset-*)` 后经 `web_safe_insets` 传入，Linux 无便携安全区来源，
  显式传零（见 §5）。Web 的 CSS 已对 `#canvas-host` 施加安全区，桌面 `env()` 为 0，
  宿主再按测得值折入度量；真机是否产生双重留白**未验证**。

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

- **平台事件接线**：`cad-ui-slint`（已随 Slint 退役，不再是宿主）/ Android `ViewInput` /
  Web 指针与键盘事件尚未调用本策略；多触点身份、pointer capture、长按语义需各宿主接入并补集成
  测试。
- **真实 IME/DPI 运行**：中文组合、软键盘高度变化、浏览器缩放、横竖屏切换、
  DPR=1/2/3 真机验证尚未执行；当前只有纯逻辑单元测试与 Linux 离屏（lavapipe）集成
  测试。
- **安全区**：共享助手 `inset_canvas_metrics` 已把 `safe_insets` 折入
  `CanvasMetrics`（退化输入显式拒绝）。宿主接线：Web 由 JS 读取
  `env(safe-area-inset-*)` 经 `web_safe_insets` 传入；Linux 无便携安全区来源，
  显式传零并记录；Android 提供 `set_surface_insets` 入口（系统栏/软键盘），但
  **Activity 侧尚未把 OS inset 回调转发到该入口**（未使用未验证的 OS inset API，
  不臆造），故真机 inset 仍未生效。
- **上下文菜单**：长按/右键菜单策略未在本模块建模。
- **状态来源**：`StatusModel` 目前由调用方提供 `units`/`unsaved`/`loading`，
  尚未从 `host.rs` 的完整会话一次性派生。
- **点选/hover 端到端运行未做**：§1.1 的 DOM 点选闭环（`SelectAt` → `pick_at_screen` → 高亮/属性）与
  §1.1.2/§1.1.3 的 hover / `pointer_leave` 只在**合成/本地**单元测试中覆盖；**未**运行真实 GUI、原生窗口、
  真实 GPU 或真实 DWG，`SelectionRef` 与 hover 高亮均**未**在真实宿主窗口闭环。可选探针
  `YACR_PROFILE_PICK hover elapsed_ms=… picked=…` 只在 `YACR_PROFILE_PICK=1` 时输出，其**任何数字仅为
  合成/本地**（记录运行用的是 6 实体 demo 文档，无真实 DWG/GPU），**不构成**性能结论。框选（S5）仍未实现。
  证据与 NOT RUN 边界见 `docs/handoff.md` 的 2026-10-17 S1/S2 小节与 `docs/render-optimization-backlog.md` 的 S1/S2 项。
