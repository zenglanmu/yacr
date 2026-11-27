# ADR-0082：两点透视（CAMERA TWO-POINT）

- 状态：**已接受（决策，2026-11-28）**；**已实现（2026-11-28，放平当前透视相机；无真实 GUI/GPU 证据）**
- 关联：ADR-0073（Position Camera）、`VSCURRENT`/相机视图状态、`WALK`/`LOOKAROUND`

## 背景

`CAMERA` 已有真实的两点放置（眼睛、目标），但 `CAMERA TWO-POINT` 一直被具名拒绝，覆盖表把
“无两点透视”列为 Position Camera 的边界。两点透视的本质是**竖直线保持竖直**：在标准透视矩阵
下，这等价于相机视线水平（没有俯仰），世界 `+Z` 作为 up。

本构建没有 lens-shift 投影参数，不能在相机俯仰时把竖线校正回竖直；但“放平当前透视相机”是
一个真实、可验证的两点透视形式，且不写文档。

## 决策

### D1 命令形式

- `CAMERA TWO-POINT`（`TWO-POINT-PERSPECTIVE`/`2POINT` 同义）不再是拒绝，而是把**当前**
  透视相机放平：
  - 保留 eye 与当前镜头（FOV/投影）；
  - `target.z = eye.z`，水平方向与水平距离保持；
  - `up = world +Z`；
  - 进入 3D，保存 2D 相机；记录 `ZOOM PREVIOUS`，不写文档、不产生 revision/undo 步骤。
- 已是水平相机时答 `camera.two_point_unchanged`，不重复记 `ZOOM PREVIOUS`。
- 正交/平面视图答 `camera.two_point_requires_perspective`；垂直视线（水平分量为零）答
  `camera.two_point_degenerate`；相机帧非法仍走 `camera.invalid_frame`。
- `CAMERA WALK` 保持具名拒绝：WALK/LOOKAROUND 是独立工具，不是第二个相机模式。

### D2 接线

- 复用 `CommandId::PositionCamera`，新增 `CommandPayload::PositionCameraTwoPoint`（不新增命令
  id，命令计数不变）。
- typed 入口通过 `dispatch_typed_argument` 进入普通命令漏斗；无逐载体文件。
- `CameraArgument::TwoPoint` 从“拒绝”改为派发；`CameraArgument::Walk` 文案更新为“用 WALK/
  LOOKAROUND 工具”。

### D3 边界

- **没有 lens shift**：相机有俯仰时的倾斜两点透视仍未建模；本实现要求先放平。
- 没有相机高度锁定/拖动、眼高约束、两点透视的视口按钮状态。
- 真实 GUI/GPU 像素与其它载体运行时不在本 ADR。

## 后果

- `CAMERA TWO-POINT` 从具名拒绝变成真实的视图操作；覆盖表 Position Camera 仍为 partial
  （lens-shift 两点透视、高度锁定未建模），但“无两点透视”缺口关闭。
- 无格式档位变化、无新命令、无新表。
