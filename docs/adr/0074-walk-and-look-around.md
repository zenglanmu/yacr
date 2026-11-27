# ADR-0074：漫游与环视（Walk / Look Around）

- 状态：**已接受（决策，2026-11-27）**；**已实现（2026-11-27，两点视图导航工具；无真实 GUI/GPU 证据）**
- 关联：ADR-0073（Position Camera）、`VPOINT`/`VIEW` 的视图路径、ADR-0015（交互捕获）

## 背景

SketchUp 的 Walk 让观察者沿水平方向移动（保持眼高），Look Around 让固定眼睛转动视线。两者都是
**视图操作**，不修改模型。本仓库已有：

- `Camera { eye, target, up, projection }`、`ViewBasis`、`Camera::validate`；
- Position Camera（ADR-0073）已证明“捕获点 → 透视相机 → `push_view_snapshot`”的路径；
- 一点捕获（`DrawToolKind::Exact(1)`）与 typed 参数两条入口；
- `VPOINT` 等只读视图命令是 Viewer 合法且记 `ZOOM PREVIOUS` 的先例。

## 决策

### D1 Walk：水平移动眼睛

- 新命令 `CommandId::Walk`（selector `WALK`）。
- 裸 `WALK` 启动一点捕获（`BeginDraw(Walk)`）：**点一个地面点，走到那里**——眼睛的 x/y 变成该点，
  z 保持不变；target 随 eye 平移，因此视向与视距不变。
- typed `WALK <distance>`：沿当前视向的**水平分量**走 `distance`（负值后退）。
- 只在 3D 透视视图可用：2D/正交按 `walk.requires_perspective` 拒绝（Walk 是透视行为，平面/正交没有
  眼高概念）；视向水平分量退化（直视上/下）按 `walk.degenerate_view` 拒绝；零步/非有限值分别是
  `walk.no_distance` / `walk.invalid_distance` / `walk.invalid_point`；构造出的相机失败按
  `walk.invalid_frame` 拒绝。
- `push_view_snapshot()` + 设置相机；**无文档写入**、无 revision/undo，Viewer 合法。

### D2 Look Around：固定眼睛转动视线

- 新命令 `CommandId::LookAround`（selector `LOOKAROUND`）。
- 裸 `LOOKAROUND` 启动一点捕获：**固定 eye 看向该点**（视距取当前相机距离，目标点只给方向）。
- typed `LOOKAROUND <yaw> <pitch>`：绕世界 `+Z` 偏航、绕相机右轴俯仰（度），保持 eye 与视距；
  用 Rodrigues 旋转（`rotate_about`），俯仰后仍要求视向非垂直。
- 拒绝：`lookaround.requires_perspective`（同上）、`lookaround.invalid_point` /
  `invalid_angle`、看向眼点本身 `lookaround.no_direction`、垂直极点 `lookaround.pole`、
  相机失败 `lookaround.invalid_frame`；只改视图、`ZOOM PREVIOUS`、Viewer 合法。

### D3 共享的视图接缝

两个命令都在 `app_view.rs`，复用 `camera::{add, sub, scale3, dot3, cross, length3, normalize3}`
与 `ViewBasis`；`up_for(forward)` 与 Position Camera 的 +Z/+Y 规则一致。没有第二套相机数学、没有
新的渲染通道；draw-tool 侧各追加一个 one-point kind（`Walk`/`LookAround`，`ALL` 末尾追加），
intent 走同一 `DrawIntent → payload` 映射。

### D4 UI

概念域工具栏/Camera 菜单的 Walk/Look Around 从 pending 变成 `command: 'WALK'` /
`'LOOKAROUND'` 的真实工具；catalog 增加两行（`draw.kind.walk` / `draw.kind.lookaround`）与提示键。

## 不在本 ADR 范围

- 碰撞检测、上下楼梯/重力、连续按键漫游、速度/加速度设置；
- 自由拖动环视（drag 连续转动）、游戏手柄/VR；
- 两点透视、相机高度锁定；
- 真实 GUI/GPU 交互与其它载体运行时。

## 后果

- 新增两个命令、两个 one-point 捕获 kind、两个 payload 与两条 typed 语法；**无格式档位变化**
  （视图是会话状态）。
- Walk 是离散步进（一次捕获或一次 typed 距离），不是连续按帧漫游；Look Around 是离散转向，
  不是鼠标自由环视——这是明确的 partial 边界，覆盖表与提示都写明。
- 工具层覆盖表不再有 pending 工具；仍 pending 的只剩 Match Photo/Overlays 两个托盘面板。
