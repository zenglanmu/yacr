# 放置相机（Position Camera，`CAMERA`）（2026-11-27）

`CAMERA` 从 pending 变成真实的两点视图工具：第一次点击是眼睛、第二次是目标，设一个透视相机
（eye/target/世界 +Z up、沿用当前 FOV），保存 2D 相机并进入 3D；只改视图、记 `ZOOM PREVIOUS`，
无文档写入/撤销。设计见 [ADR-0073](adr/0073-position-camera.md)。无格式档位变化。

## 1. 已接线

- **命令**：`CommandId::PositionCamera`（`CAMERA`）；裸选择器路由到
  `BeginDraw(PositionCamera)`（两点捕获，自动提交），`Viewer` 合法。
- **效果**：`Application::position_camera` 设 eye/target/up/projection、保存 2D 相机、进入
  `ThreeD`；`push_view_snapshot` 让 `ZOOM PREVIOUS` 可返回；无 revision/undo 步。
- **拒绝**：`camera.invalid_point`、`camera.eye_target_coincident`、`camera.invalid_frame`；
  typed `CAMERA TWO-POINT` → `camera.two_point_not_implemented`、`CAMERA WALK` →
  `camera.walk_not_implemented`、其它参数 → 语法拒绝（都不启动捕获/不移动相机）。
- **旧拒绝退场**：`CAMERA` 从 Rust `RefusalId` 与前端 `catalog-refusals` 中移除
  （refusal 计数 309；Wave 17 批次 188），catalog/工具栏改为真实命令。
- **UI**：概念域工具栏/Camera 菜单的 Position Camera 不再是 pending（`command: 'CAMERA'` +
  提示键）；catalog 新增 `draw.kind.camera`。

## 2. 验证

| 命令 | 结果 |
|---|---|
| `cargo test -p cad-app --lib --locked` | PASS（**3516 passed**，含相机 3 个 + 捕获 intent/状态行 1 个 + refusal 计数更新） |
| `cargo test -p cad-host --lib --locked` | PASS（**500 passed**，含 `tests::position_camera` 视图端口相机契约） |
| `cargo test -p cad-ui-core --lib --locked` | PASS（**977 passed**，含 `tests_camera` 2 个：路由、TWO-POINT/WALK 具名拒绝） |
| `(cd apps/desktop-ui && npm test)` | PASS（tsc --noEmit + node --test：**2128 passed**，Position Camera 不再是 pending；Wave 17 批次 188） |
| `cargo clippy -p cad-app -p cad-host -p cad-ui-core --all-targets --locked -- -D warnings` | PASS |
| `cargo fmt --all -- --check` | PASS |
| `python3 scripts/check-file-size.py` | PASS（2997 files） |
| `python3 scripts/check-i18n.py` | PASS（3708 keys） |
| `python3 scripts/check-architecture.py` / `check-command-docs.py` | PASS |

**边界**：没有真实 GUI/GPU 像素证据；相机放置用 app/host 合成契约，真实窗口交互、手感与其它载体
运行时 **NOT VERIFIED**；两点透视/Walk/Look Around 未实现。

## 3. NOT VERIFIED / 不在范围

- 两点透视、眼高锁定/拖动、Walk/Look Around；
- 相机动画/路径/VR；
- 真实 GUI/GPU 交互与其它载体运行时验收。
