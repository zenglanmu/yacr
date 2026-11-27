# 漫游与环视（Walk / Look Around）（2026-11-27）

`WALK` 与 `LOOKAROUND` 从 pending 变成真实的**视图导航**工具：一点捕获或 typed 数值移动/转动透视
相机，只改视图、记 `ZOOM PREVIOUS`，不写文档。设计见
[ADR-0074](adr/0074-walk-and-look-around.md)。无格式档位变化。

## 1. 已接线

- **WALK**：裸选择器一点捕获（走到地面点，眼睛 z 不变，target 随动）；typed `WALK <distance>`
  沿水平视向前进（负值后退）。
- **LOOKAROUND**：裸选择器一点捕获（固定 eye 看向该点，视距保持）；typed
  `LOOKAROUND <yaw> <pitch>` 绕世界 +Z / 相机右轴转动。
- **共享规则**：仅 3D 透视（否则 `walk.requires_perspective` / `lookaround.requires_perspective`）；
  拒绝 `walk.no_distance`/`invalid_distance`/`invalid_point`/`degenerate_view`/`invalid_frame` 与
  `lookaround.no_direction`/`invalid_angle`/`invalid_point`/`pole`/`invalid_frame`；Viewer 合法。
- **UI**：概念域工具栏/Camera 菜单 Walk/Look Around 不再是 pending（`command: 'WALK'` /
  `'LOOKAROUND'` + 提示键）；catalog 增加 `draw.kind.walk` / `draw.kind.lookaround`。

## 2. 验证

| 命令 | 结果 |
|---|---|
| `cargo test -p cad-app --lib --locked` | PASS（**3522 passed**，含 walk/look 5 个 + 捕获 intent 1 个 + keys/arity 更新） |
| `cargo test -p cad-host --lib --locked` | PASS（**501 passed**，含 host 相机导航契约） |
| `cargo test -p cad-ui-core --lib --locked` | PASS（**980 passed**，含 `tests_walk` 3 个：两个选择器路由 + 数值语法） |
| `(cd apps/desktop-ui && npm test)` | PASS（tsc --noEmit + node --test：**2128 passed**，Walk/Look Around 不再是 pending） |
| `cargo clippy -p cad-app -p cad-host -p cad-ui-core --all-targets --locked -- -D warnings` | PASS |
| `cargo fmt --all -- --check` | PASS |
| `python3 scripts/check-file-size.py` | PASS（3002 files） |
| `python3 scripts/check-i18n.py` | PASS（3714 keys） |
| `python3 scripts/check-architecture.py` / `check-command-docs.py` | PASS |

**边界**：没有真实 GUI/GPU 像素证据；导航是离散步进/转向（不是连续按帧漫游或鼠标自由环视），
只有 app/host 合成契约；其它载体运行时 **NOT VERIFIED**。

## 3. NOT VERIFIED / 不在范围

- 碰撞、重力/上下楼、连续漫游、速度设置；
- 鼠标拖动自由环视、手柄/VR；
- 两点透视、相机高度锁定；
- 真实 GUI/GPU 交互与其它载体运行时验收。
