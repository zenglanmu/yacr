# ADR-0073：放置相机（Position Camera）

- 状态：**已接受（决策，2026-11-27）**；**已实现（2026-11-27，两点透视放置；无真实 GUI/GPU 证据）**
- 后续：ADR-0082 实现了 `CAMERA TWO-POINT`（放平当前透视相机）；本 ADR 的其余边界仍有效。
- 关联：ADR-0015（交互捕获）、`VPOINT`/`VIEW` 的视图路径、ADR-0067/0068（工具不再 pending）

## 背景

SketchUp 的 Position Camera 是一个**视图**工具：点一个眼睛位置，再点一个看向的目标点，进入该
位置的透视视图（默认眼高、两点透视是另外的模式）。本仓库已有：

- `Camera { eye, target, up, projection }` 与 `Camera::validate`；
- `VPOINT x y z` 已是“世界方向 → 3D 透视相机”的视图命令（设置 eye/target/up、保存 2D 相机、
  记 `ZOOM PREVIOUS`），并且是 Viewer 合法的只读视图操作；
- 交互捕获（`BeginDraw` + 点序列）、typed `ARGUMENT_SELECTORS` 与具名拒绝模式；
- `CAD` 的 extended refusal 表里曾有一个 `CAMERA` 具名拒绝——那是“没有相机模型”的旧状态，
  现在与真实命令冲突。

因此 Position Camera 只需把已有的两点捕获接到已有的相机设置上；没有新的几何/文档语义。

## 决策

### D1 两点捕获 + 一次性相机设置

- 新命令 `CommandId::PositionCamera`（selector `CAMERA`）。裸 `CAMERA` 路由到
  `BeginDraw(DrawToolKind::PositionCamera)`：**第一次点击是眼睛，第二次是目标**，两次后自动提交。
- 捕获意图 `DrawIntent::PositionCamera { eye, target }` → `CommandPayload::PositionCamera` →
  `Application::position_camera`：
  - 两个点必须有限（`camera.invalid_point`）；
  - eye == target 时没有视向（`camera.eye_target_coincident`）；
  - up 取世界 `+Z`，视向接近 ±Z 时取 `+Y`（与 `VPOINT` 同一规则）；
  - 投影沿用当前透视镜头（正交时用文档的默认 FOV），即进入透视；
  - `Camera::validate` 失败按 `camera.invalid_frame` 拒绝；
  - 成功时 `push_view_snapshot()`（`ZOOM PREVIOUS` 可撤销）、设置相机、保存 2D 相机并进入
    `ThreeD`；**只改视图**：无文档写入、无 revision、无 undo 步，Viewer 合法。
- 结果诊断 `camera.positioned` 明确“模型未改变”。

### D2 不支持的模式按名拒绝

`CAMERA TWO-POINT` / `TWO-POINT-PERSPECTIVE` / `2POINT` → `camera.two_point_not_implemented`
（单透视帧、世界 +Z up，没有两点透视的竖线保持）；`CAMERA WALK` →
`camera.walk_not_implemented`（Walk/Look Around 仍是 pending 工具）；其它参数是语法拒绝。
被拒绝的行不启动捕获、不移动相机。

### D3 UI 与旧拒绝的退场

- 概念域工具栏/菜单的 Position Camera 从 pending 变成 `command: 'CAMERA'` 的真实工具；catalog
  新增真实行（`draw.kind.camera`，`view` 组）。
- 旧 Wave 17-24 的 `CAMERA` 具名拒绝行从 `catalog-refusals.ts`/`RefusalId` 中移除（它和真实
  命令同名），refusal 计数 310 → 309；前端 Wave 17 测试批次 189 → 188。`CAMERA` 仍在
  `DOCUMENT_SCOPED_COMMANDS` 中（没有文档时禁用）。

## 不在本 ADR 范围

- 两点透视（竖线保持）、眼高锁定/拖动、Walk 与 Look Around 模式；
- 相机动画、路径、VR/全景；
- 真实 GUI/GPU 交互与其它载体运行时。

## 后果

- 新增一个命令、一个捕获 kind、一个 payload 与一条 typed 拒绝路径；**无格式档位变化**
  （相机是会话/视图状态，不写文档）。
- 旧的 `CAMERA` 拒绝从两处权威（Rust refusal registry + 前端 refusal catalog）同时退场，
  不留下同名重复；refusal 计数普查同步更新。
