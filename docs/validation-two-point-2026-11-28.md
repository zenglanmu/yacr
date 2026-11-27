# 两点透视（CAMERA TWO-POINT）（2026-11-28）

`CAMERA TWO-POINT` 把当前透视相机放平：保留 eye/镜头，`target.z = eye.z`，`up = world +Z`，
竖直边保持竖直；只改视图并记 `ZOOM PREVIOUS`。设计见
[ADR-0082](adr/0082-two-point-perspective.md)。无格式档位变化、无新命令。

## 1. 已接线

- **命令**：复用 `CommandId::PositionCamera` + 新 payload `PositionCameraTwoPoint`；正交视图
  `camera.two_point_requires_perspective`、垂直视线 `camera.two_point_degenerate`、已水平时
  `camera.two_point_unchanged`（不重复快照）。
- **typed**：`CAMERA TWO-POINT` / `TWO-POINT-PERSPECTIVE` / `2POINT` 派发命令；`CAMERA WALK`
  仍是具名拒绝（WALK/LOOKAROUND 为独立工具）。
- **状态**：保留投影/FOV，进入 3D 并保存 2D 相机；不写文档、不升 revision、不产生 drawing
  undo；Viewer 合法。
- **边界**：没有 lens-shift 倾斜两点透视、相机高度锁定或 GUI 模式按钮。

## 2. 验证

| 命令 | 结果 |
|---|---|
| `cargo test -p cad-app --lib --locked` | PASS（**3543 passed**，camera_tool 11；新增放平/正交与垂直拒绝/已水平 no-op 3 个） |
| `cargo test -p cad-ui-core --lib --locked` | PASS（**989 passed**，tests_camera 2：typed 派发与 Walk 拒绝） |
| `cargo test -p cad-db --lib --locked` | PASS（**523 passed**，未改模型） |
| `(cd apps/desktop-ui && npm test)` | PASS（**2140 passed**，未改前端） |
| `cargo clippy -p cad-app -p cad-ui-core -p cad-host --all-targets --locked -- -D warnings` | PASS |
| `cargo fmt --all -- --check` | PASS |
| `python3 scripts/check-file-size.py` / `check-i18n.py` / `check-architecture.py` / `check-command-docs.py` | PASS |

**边界**：没有真实 GUI/GPU 像素证据；两点透视只有相机状态机契约，真实窗口与其它载体运行时
**NOT VERIFIED**；lens-shift 倾斜两点透视、相机高度锁定仍未建模。

## 3. NOT VERIFIED / 不在范围

- lens-shift 倾斜两点透视、眼高锁定/拖动、GUI 模式按钮；
- 真实 GUI/GPU 交互与其它载体运行时验收。
