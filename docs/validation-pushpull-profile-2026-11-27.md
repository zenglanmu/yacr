# Push/Pull 在闭合轮廓上创建实体（2026-11-27）

本轮把 SketchUp 的核心建模回路补上一步：**选中矩形/圆等闭合平面轮廓，按 Push/Pull，输入高度，
回车即可拉成实体**。此前 `PRESSPULL` 只接受“一个已选精确实体 + 拾取一个面”，刚画出来的
`RECTANG` 是 `LWPOLYLINE`，没有面可拾，只能显式改用 `EXTRUDE`。

## 行为

- **工具起点识别**：`cad-host` 的 `begin_solid_face_tool`（已从 `input/interaction.rs` 拆到
  `input/solid_face.rs`）在 `PRESSPULL` 起点读取选择快照；当选择是一个闭合平面轮廓时，
  改arm `DrawToolKind::Extrude`（零点工具），不再要求面拾取。
- **同一命令**：回车提交的是既有 `EXTRUDE` 命令（`CommandPayload::ExtrudeProfile`），
  一条事务、一步撤销、同一组 `extrude.*` 具名拒绝；没有第二条写路径，也没有新命令 id。
- **轮廓判定**：`cad_app::Application::closed_profile_selection` 是 `profile_from_refs` 的只读
  谓词——闭合直线 `LWPOLYLINE`、LINE 闭环、单个 `CIRCLE`；子元素引用与块实例引用永不算轮廓；
  空选择不算。它不建实体、不开事务、不 bump revision；命令在回车时再次校验同一批行。
- **边界**：轮廓仍须平行 XY、沿 ±Z 拉伸（`EXTRUDE` 的既有边界）。其它平面的轮廓按
  `extrude.not_planar` 具名拒绝，不静默换轴；没有 `solid modeler` 的载体仍在工具起点按
  `unsupported-runtime: solid-modeler` 拒绝（能力检查先于轮廓分支）。
- **网格面/精确实体路径不变**：带 `face_sources` 的网格面仍走 `cad-mesh-edit`，精确实体仍走
  `OffsetFace` 的一拾取面流程；轮廓分支只在那两者都不成立时生效。

## 命令与结果

| 命令 | 结果 |
|---|---|
| `cargo test -p cad-app --lib --locked push_pull_profile` | PASS（3 passed：闭合 `LWPOLYLINE`、LINE 环、`CIRCLE`；只读谓词不改 revision/undo） |
| `cargo test -p cad-app --lib --locked never_a_profile` | PASS（子元素/实例引用不是轮廓） |
| `cargo test -p cad-host --locked` | PASS（**485 passed**；含 4 个新起点契约：轮廓 arm `Extrude`、回车经 intent map 把 `ExtrudeProfile` 交给内核（fake modeler 具名拒绝、revision 不变）、实体仍 arm `PressPull`、无 modeler 先拒绝） |
| `cargo test -p cad-ui-core --locked` | PASS（945 passed；顺带修正上一轮遗留的 `MESHCAP`/`MESHREFINE`/`MESHSMOOTH`/`MESHREPAIR`/`MESHREBUILD` work-mode 期望表） |
| `cargo clippy -p cad-app -p cad-host -p cad-ui-core --all-targets --locked -- -D warnings` | PASS（exit 0） |
| `cargo fmt --all -- --check` | PASS（exit 0） |
| `python3 scripts/check-architecture.py` | PASS（40 packages，无环、边界不变） |
| `python3 scripts/check-file-size.py` | PASS（2910 文件，上限 1000 行） |
| `python3 scripts/check-i18n.py` | PASS（3619 键，两目录一致；无硬编码中日韩文本） |

## 证据边界

- **没有真实 GUI/GPU/像素证据**：没有在 Wayland/X11 窗口里实际点 `RECTANG` → `P` →
  输入高度 → 回车，也没有截图或测量结果；契约只证明工具路由、命令选择与只读谓词。
- **没有真实 kernel 运行**：`cad-app` 的主机测试用 fake modeler；不过仓库既有
  `cad-kernel-solidcore/tests/extrude_profile.rs` 覆盖真实 solidcore 拉伸（本轮未改内核）。
- **没有 Windows/macOS/Android/Web 运行证据**：起点分支在共享 `cad-host`，各载体只有既有
  编译证据；载体运行时 **NOT VERIFIED**。
- **平面限制**：轮廓必须平行 XY。SketchUp 的面法向推拉（垂直墙面、任意工作平面）仍
  **NOT IMPLEMENTED**，按 `extrude.not_planar` 拒绝。
- **未实现**：推拉面时相邻面自动延伸/裁剪的 SketchUp 语义仍由精确偏移（`OffsetFace`）与
  网格编辑各自承担；没有推断/吸附锁定。
