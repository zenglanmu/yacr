# ADR 0014：对象捕捉获取与窗口/交叉选择

- 状态：**已接受（Accepted，2026-10-18）**，含 Gate P0-design attempt 1 的 6 项修正；实施状态另起小节追加，
  不改写决策文本。
- 范围：`cad-app` 命令/会话层、`cad-host` 交互层、`cad-ui-core` 协议、React 宿主 UI 与 renderer overlay。
  **不**覆盖纸空间视口内捕捉、3D 捕捉、对象捕捉追踪（OTRACK）、极轴追踪。
- 相关：`CAD_IMPLEMENTATION_SPEC.md` §3.3 / §4.7 / §F05、`docs/input.md`、`docs/interaction.md`、
  `docs/adr/0012-scene-spatial-index-and-interaction.md`、`docs/react-linux-app.md`、
  `docs/compatibility.md`。

## 背景（勘察事实）

1. **捕捉引擎已存在，但点捕获不消费它。** `crates/cad-measure/src/snap/mod.rs:23` 已定义 6 类候选
   （`Endpoint` / `Midpoint` / `Center` / `Quadrant` / `Perpendicular` / `LocalIntersection`）。容差来自
   `TolerancePolicy::default().interaction_logical_pixels`（`cad-domain/src/lib.rs:421`，**12.0**），
   由 `collect_candidates` 乘 `world_per_px` 换算（`snap/mod.rs:170`），排序为
   距离 → `SnapKind::priority` → x → y（`snap/mod.rs:263-280`）。
   `HostController::snap_hints_near_cursor`（`cad-app/src/host/mod.rs:689`）只把候选喂给提示叠加；
   `CapturePoint` 走 `cad-host/src/input/interaction.rs:109-116` 的原始世界点。⇒「看得见标记，点不吸附」。
2. **画图工具的光标根本没进捕捉。** `active_tool_cursor`（`cad-app/src/host/mod.rs:654`）只读
   `MeasurementPreview::cursor`，`DrawPreview::cursor`（`draw_tool/mod.rs:183`）不可见；其注释
   “draw tool is not yet wired” 已过期。LINE/CIRCLE/ARC 是捕捉的主要用户，必须一并修。
3. **没有窗口/交叉框选。** 选择只有单点 `SelectAt` 与 `SELECTALL`。
4. **F3 在当前 React 宿主完全未绑定**（`App.jsx` 仅 Ctrl+0/Ctrl+9/F2/Escape）。`docs/shortcuts.md` 的
   F3 行属于**已退役 Slint 宿主的历史**，不得当作现状引用。`StatusBar.jsx:10` 已有四个**显式禁用**的
   drafting 开关（`GRID`/`SNAP`/`ORTHO`/`OSNAP`，`status.toggle_unsupported`）；i18n
   `status.toggle.osnap` 已存在。⇒ F3 是**新绑定**，不是行为变更。
5. **CAD 画面是 GTK 最上层子窗口**（`apps/app-linux/src/gtk_bridge/native.rs:38-43`），webview 覆盖不到，
   **DOM overlay 画不出橡皮筋**。
6. **框选粗筛已存在**：`PickCatalog`（`cad-app/src/picking/mod.rs:320-412`）是保守世界 AABB 广相，
   复用 `cad_spatial::pick_world_bounds` + `ChunkGridIndex`（ADR-0012 §7）。**无需新索引**。

## 决策 A：对象捕捉获取（snap acquisition）

1. **单一策略点**：捕捉解算只在 `cad-app::HostController`（不是 `cad-host::HostCore`）实现。
   宿主不得各自实现。栅栏：契约测试断言 `cad_measure::snap` 只被 `cad-app` 引用。
2. **解算发生在两个边界入口，并存储已解算点**：`HostController::move_tool_cursor(raw)` 与
   `capture_tool_point(raw)` 各自调用同一个纯函数 `resolve_snap(raw) -> CadResult<Option<SnapCandidate>>`，
   把**解算后的点**写进工具光标/捕获点。预览（读同一存储）与提交（`confirm_draw`/`ConfirmMeasurement`
   读同一存储）因此共享唯一真相。**不缓存“上一次候选”**——那是第二个真相来源，会过期；
   预览与提交只在原始输入不同（最后一次 move vs. 点击）时不同，这与 AutoCAD 一致且诚实。
3. **错误契约**：非有限点在上游已被拒绝（`SessionState::capture_point`/`move_cursor`，`lib.rs:256,276`；
   `snap_candidates_near` 对坏 cursor/`world_per_px` 返回 `InvalidInput`）。`resolve_snap` 只把
   「无 viewport / 无候选」映射为 `Ok(None)` ⇒ 返回原始点；不得把 `InvalidInput` 吞成空成功。
4. **开关与叠加是两件事**：
   - 会话级 `snap_enabled` 决定**是否吸附**；F3 只切它。
   - 提示叠加绘制条件 = `snap_enabled && view.overlays.snapHints`。
   - F3 **不得**改写持久化 viewer config（`viewer_config/schema.rs:231` 是用户可配置项）。
5. **作用域**：仅在工具处于 `Drawing`/`Measuring` 时解算；`Idle` hover 与单点选择不吸附。
6. **平面**：只在当前 work plane 上解算（`snap_candidates_near` 以 cursor 的 z 建平面，`databases.rs:220`）。
   3D 捕捉、纸空间捕捉显式 NOT IMPLEMENTED。
7. **候选类型状态**：不新增会话状态；UI 从 `snap_hints_near_cursor().first()` 派生当前类型。
   状态栏走既有 `CadStatus` 投影新增字段（见验收），文案键 `snap.kind.<as_str>`。

## 决策 B：窗口 / 交叉选择（box selection）

1. **职责切分**：拖拽方向判定在宿主输入层；**选择集合计算**在 `cad-app` 命令层
   （新 `CommandId::SelectInBox`），与 `SelectAll` 同构（`select_all.rs:20` 在命令内计算 refs）。
2. **手势仲裁（明确的唯一赢家）**：
   - **2D**：主键拖拽 = 框选；平移仍是**中键拖拽**（`pointer.rs:133-139` 的通用分支）与双指手势。
   - **3D**：主键拖拽 = 轨道旋转（维持现状，`pointer.rs:130-132`）；框选在 3D 不启用（显式 NOT IMPLEMENTED）。
   - 阈值仍是 `DRAG_THRESHOLD_LOGICAL_PX = 4.0`；低于阈值仍是单点 pick。
   - 这是**行为变更**（2D 主键拖拽由平移改为框选），必须在当前宿主文档 `docs/react-linux-app.md` 记录。
3. **语义（AutoCAD 对齐）**：左 → 右 = WINDOW（**完全包含**）；右 → 左 = CROSSING（**相交或包含**）。
4. **契约形状**：`CommandPayload::BoxSelect { rect_logical: [[f64;2];2], crossing: bool, mode: SelectionMode }`，
   矩形用 **canvas 逻辑坐标**（与 `SelectAt`/`ZoomAt` 同约定），命令内经 viewport 投影到 work plane。
   `mode` 复用 `cad_app::interaction::SelectionMode`（首版 UI 传 `Replace`，`Shift` 传 `Add`）。
5. **权限与副作用**：**Viewer 与 Work 均允许**（与 `Select`/`SelectAll` 一致），
   `CommandOutcome::none()`，**不发 ChangeSet、不改 revision、不记历史**。
6. **拒绝面（显式）**：
   - 无打开文档 → `InvalidInput("document not open")`（同 `select_all.rs:31`）。
   - 纸空间 → `Unsupported`（同 `select_all.rs:37`）。
   - 工具处于 `Drawing`/`Measuring` → 不框选（同 `select_at_mode` 的 `Idle|Selecting` 规则）。
   - 退化矩形 / 非有限角点 → `InvalidInput`。
7. **过滤与精度（逐类诚实）**：
   - 复用 `select_all.rs:44-63` 的可见性过滤与 `validate_branch` 预检（cyclic/超深 INSERT、opaque、
     `Unsupported` 一律**拒绝而非伪造**）。
   - 粗筛 = `PickCatalog` 的保守 AABB。
   - **WINDOW**：保守 AABB 完全包含 ⇒ 几何必然完全包含（sound），但保守包围盒外凸处会**少选**
     （如 spline 控制凸包）——记录为精度上限。
   - **CROSSING**：AABB 相交会**多选**。精确判定按类：线段/折线用
     `cad_geometry::clip::{clip_segment_to_xy_rect, clip_polyline_to_xy_rect}`（精确）；
     arc/circle/ellipse/spline 用 `tessellate_curve` + 精确 clip，并以 `Precision::Approximate` 标注
     （与 snap 引擎同一诚实模式）；hatch/mesh/insert 走其 pick 项语义或显式 `Unsupported`。
     报告必须区分粗筛与精确阶段。
8. **结果反馈**：`CommandOutcome` 增加稳定诊断码 `select.box.count` 与 `{count}`，**文案由 i18n 提供**
   （`scripts/check-i18n.py` 拒绝 Rust 硬编码 CJK）。0 也是结果，显式显示。
9. **橡皮筋渲染**：CAD 面在 webview 之上，**禁止 DOM overlay**。橡皮筋作为**瞬态 renderer overlay 输入**
   （与 `draw_preview` 同管线）：宿主把锚点与当前光标的两个逻辑角点换算为 work plane 上 4 条世界空间线段，
   以独立 `draw_order`（`PREVIEW_DRAW_ORDER` 之下、`SNAP_HINT_DRAW_ORDER` 之上）提交。
   **不得**每帧 readback，也不得把一次性手势塞进数据库/ChangeSet。

## 栅栏（不得软化）

- 不改变既有 pick 的精确谓词与候选集；框选与捕捉都是**新增**路径。
- 不改 `acadrust`、不加 Cargo patch、不引入新第三方依赖。
- 所有新用户可见文案只进 `crates/cad-ui-core/i18n/{zh-CN,en}.json`，键集一致。
- 不得把 WINDOW 的保守少选或 CROSSING 的近似静默当作精确结果。

## 验收

- `cad-app` 契约测试：端点/中点吸附提交（含 `DrawPreview` 光标）；开关关闭不吸附；无 viewport 不吸附；
  非有限点仍被拒绝；window 只含完全包含、crossing 含相交与包含、边界擦碰、退化矩形、空结果、
  纸空间拒绝、隐藏图层过滤、3D 拒绝。
- `cad-host` 集成测试：`MoveCursor`/`CapturePoint` 走吸附；2D 主键拖拽进入框选而中键仍平移；
  3D 主键拖拽仍轨道；4px 以下仍为单点 pick。
- `cad-ui-core`：`BoxSelect` 载荷校验与 `SelectInBox` 路由；捕捉开关；状态投影（`snap.enabled`、
  当前候选类型、框选计数）。
- 全量门禁（`cargo fmt/clippy --workspace`、`check-architecture.py`、`check-i18n.py`、wasm 目标 check、
  UI `npm test && npm run build`）+ 真实 GPU 证据（`scripts/collect-evidence.sh`，
  **合成夹具 / `fixtures/dxf/qcad-flange`**；**不使用** `complex-test.dwg`）。

## 明确不做的（NOT IMPLEMENTED，显式记录）

- 3D 捕捉、纸空间捕捉、对象捕捉追踪（OTRACK）、极轴追踪/正交约束、捕捉循环（Tab 切换候选）。
- 按类型开关捕捉（首版只有全局 `snap_enabled`）。
- 纸空间框选、3D 框选。
