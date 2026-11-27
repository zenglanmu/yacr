# 网格内核注入四个 native-below 载体 + dom-canvas 同步例外（2026-11-27）

本轮把 ADR-0061 的 `MeshModeler` 注入从 Linux 原生宿主扩展到全部 `native-below` 载体：
Linux、Windows、macOS、Android。`dom-canvas`（Web、X11/XWayland）随后也在同一轮改为
**同步例外**（`app-web` 注入、会话线程 inline、`mesh.sync_budget` 上限），见文末
“dom-canvas 同步例外”；它不是 `native-below` 的 worker/取消模型。

## 改动

| 载体 | 依赖 | 注入点 |
|---|---|---|
| Linux `app-linux` | 已有（2026-11-26） | `apps/app-linux/src/host/tick/bootstrap.rs` |
| Windows `app-windows` | `cad-kernel-manifold.workspace = true`（`cfg(target_os = "windows")` 表） | `apps/app-windows/src/host/tick.rs` 会话创建后 `set_mesh_modeler` |
| macOS `app-macos` | 同上（`cfg(target_os = "macos")` 表） | `apps/app-macos/src/host/tick.rs` 会话创建后 `set_mesh_modeler` |
| Android `app-android` | 同上（`cfg(target_os = "android")` 表） | `apps/app-android/src-tauri/src/host/tick.rs` 会话创建后 `set_mesh_modeler` |

Android 另加源码契约 `contract::the_native_session_installs_the_mesh_kernel`，钉住
`session.set_mesh_modeler` 与真实 `cad_kernel_manifold::ManifoldMeshModeler::new()`。

架构边界不变：`cad-kernel-manifold` 仍只由 `apps/app-*` 运行时链接，`cad-*` 共享层只见
`MeshModeler` trait（`scripts/check-architecture.py`）。`Cargo.lock` 只新增三个 app 包的依赖行，
没有新第三方包。

## 命令与结果

| 命令 | 结果 |
|---|---|
| `cargo fmt --all -- --check` | PASS（exit 0） |
| `python3 scripts/check-architecture.py` | PASS（exit 0，40 packages，acyclic） |
| `cargo test -p app-android --locked` | PASS（77 passed, 0 failed） |
| `cargo test -p app-android --locked the_native_session_installs_the_mesh_kernel` | PASS（1 passed） |
| `cargo check -p app-windows --target x86_64-pc-windows-msvc --locked` | PASS（exit 0；本机 `llvm-rc` 不在 PATH，用 `/tmp/opencode/bin/llvm-rc -> /usr/bin/llvm-rc-21` 符号链接满足 `tauri-winres`，未改仓库/系统） |
| `cargo check -p app-macos --target aarch64-apple-darwin --locked` | PASS（exit 0；Linux 主机无 Apple SDK，`cc` 包装器把 `-arch arm64` 改写为 `clang --target=aarch64-apple-darwin`，只做类型检查，不链接、不产出 `.app`） |
| `cargo check -p app-android --target aarch64-linux-android --locked` | PASS（exit 0；`NDK_HOME` 指向已安装 NDK 29，`apps/app-android/dist` 已存在） |

## 证据边界

- **Linux**：沿用 2026-11-26 的真实内核命令/契约证据（`docs/validation-mesh-2026-11-26.md`）。
- **Windows / macOS / Android**：只有**跨目标编译/类型检查 + Android 源码契约**。没有启动宿主、
  没有创建窗口、没有执行 `UNION`/`SUBTRACT`/`INTERSECT`/`MESHREPAIR`/`MESHREBUILD`，也没有
  设备/真机/GPU/像素/性能证据。运行时 **NOT VERIFIED**。
- macOS 的检查不是 CI `macos-check` 的替代：它只证明 Rust 代码能针对 mac target 通过类型检查，
  不证明 AppKit/Tauri/WebView/面板/层序可用。
- **NOT RUN**：各非 Linux 载体的 clippy、完整测试、宿主 smoke；Android 设备安装；Windows
  Wine/真机；macOS 真机。
- `dom-canvas`（Web、X11/XWayland）本轮已改为同步例外，见下一节；纯面编辑/细分/平滑/补洞是
  `cad-mesh-edit` 共享层逻辑，不依赖该内核。

## 同轮后续：dom-canvas 同步例外

- `apps/app-web` 在 wasm target 依赖 `cad-kernel-manifold`，`Live::install` /
  `Live::install_empty` 创建会话后调用 `install_mesh_modeler`；宿主测试编译为 no-op。
- `cad-app::tasks::mesh::MeshJobManager::start` 在 `target_arch = "wasm32"` 下不 spawn：
  同步 `run_job`，结果放进同一个 `mpsc`，下一次 `HostCore::tick` 的
  `poll_mesh_job` 照常提交（一条事务/一步撤销）。
- 同步预算在调用内核前检查：`MESH_SYNC_MAX_TRIANGLES = 200_000` 总操作数三角面，
  超出返回稳定标记 `mesh.sync_budget`；没有中途取消/进度，页面在调用期间阻塞。
- 详细命令与证据见 [`docs/validation-web-mesh-sync-2026-11-27.md`](validation-web-mesh-sync-2026-11-27.md)。

## 同轮：网格 plan 内存优化

`MeshJobOperand` 直接 move `DbEntity`（不再 `snapshot.clone()` + 单独 `MeshData` clone），
`MeshJobPlan` 取消 `Clone`；分类用不复制几何的 `is_mesh_operand`，worker 内按需派生
`MeshData`，布尔折叠 move 第一个操作数。结果仍经 `mpsc` 回命令线程。

| 命令 | 结果 |
|---|---|
| `cargo clippy -p cad-app --all-targets --locked -- -D warnings` | PASS（exit 0） |
| `cargo test -p cad-app --lib --locked tests::drawing::mesh_solids -- --test-threads=1` | PASS（7 passed） |
| `cargo test -p cad-app --lib --locked host::tests::mesh_jobs -- --test-threads=1` | PASS（5 passed） |

这是结构性去除复制，不是内存/耗时基准；没有声称性能数字。
