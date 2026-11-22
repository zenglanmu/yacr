# MSAA 验证记录（2026-11-22）

范围：ADR-0047 的 base 通道 MSAA、1× overlay/mask、运行时 `samples` 切换与设置链路。

## 已执行

环境：Linux，Mesa **lavapipe**（软件 Vulkan），离屏 64×64 headless 目标；不是真实 GPU、
不是像素/性能验收。

| 命令 | 结果 |
|---|---|
| `cargo test -p cad-render-wgpu --test headless_render msaa_four_samples --locked -- --test-threads=1` | PASS：3D mesh 4× 帧非空；`frame_texture().sample_count() == 1`；4×→1× 切换后 `msaa_samples() == 1` 且仍非空 |
| `cargo test -p cad-render-wgpu --test headless_render mask_rectangle --locked -- --test-threads=1` | PASS：1× 与 4× 矩形 mask 均遮住横线；4× 内部/边界列等于 clear color（无 ghost edge） |
| `cargo test -p cad-render-wgpu --test headless_render --locked -- --test-threads=1` | PASS：56/56 |
| `cargo test -p cad-render-wgpu --locked -- --test-threads=1` | PASS：全部测试套件（含 headless、merged、SDF、pick、wgsl；性能用例按既有 ignore） |
| `cargo test -p cad-render-wgpu --lib msaa --locked` | PASS：`effective_samples`/钳制诊断表测试 |
| `cargo test -p cad-app viewer_config::tests_display --locked` | PASS：默认 `4x`、三 token round-trip、`8x` 拒绝 |
| `cargo test -p cad-host settings --locked` | PASS：95/95，含 `settings_msaa` 契约 |
| `cargo test -p cad-ui-core settings --locked` | PASS：26/26 |
| `cargo test -p cad-engine render --locked` | PASS：86 passed / 16 ignored（GPU 用例按既有 ignore），含 sample-only view revision |
| `cargo test -p cad-desktop-shell --test setting_commands --locked` | PASS：9/9 |
| `(cd apps/desktop-ui && npm test)` | PASS：1946/1946（`tsc --noEmit` + node tests） |
| `(cd apps/desktop-ui && npm run build)` | PASS（Vite 生产构建；chunk 大小提示不是失败） |
| `cargo check --workspace --all-targets --locked` | PASS |
| `python3 scripts/check-i18n.py` | PASS：3129 keys |
| `python3 scripts/check-file-size.py` | PASS：所有代码文件 ≤1000 行 |
| `cargo fmt --all -- --check` | PASS |
| `cargo clippy -p cad-render-wgpu --all-targets --locked -- -D warnings` | PASS |
| `cargo clippy -p cad-engine -p cad-host -p cad-ui-core --all-targets --locked -- -D warnings` | PASS |
| `cargo clippy -p app-linux --all-targets --locked -- -D warnings` | PASS |
| `cargo check -p app-linux --all-targets --locked` | PASS |
| `cargo check -p app-web --locked` | PASS（native host target） |
| `cargo check -p app-web --target wasm32-unknown-unknown --locked` | PASS（wasm） |
| `cargo check -p app-windows --locked` | PASS（Linux 宿主上的类型检查） |
| `cargo check -p app-macos --locked` | PASS（Linux 宿主上的类型检查） |
| `cargo check -p app-android --locked` | PASS（Linux 宿主上的类型检查） |

## NOT VERIFIED

- 真实 GPU 上的 4× 视觉质量、性能、显存峰值；lavapipe 只证明离屏管线/像素路径可运行。
- 真实浏览器 WebGL2 的 MSAA：`CanvasPresenter` 对 `Backend::Gl` 显式返回 `[1]`，因此
  Web 当前**不会**启用 MSAA；真实浏览器证据为 NOT RUN。
- Windows/macOS/Android 运行时与真实 DPI/窗口合成；仅完成 Linux 宿主上的编译/类型检查。
- 2× 在具体原生驱动上的支持集：由 `supported_msaa_samples(adapter)` 查询；未在真实
  设备逐卡核对。
- 真实 GPU 上解析抗锯齿（ADR-0009）与 MSAA 叠加后的 1 px 线条粗细/深度共面 behavior
  未做像素验收。
