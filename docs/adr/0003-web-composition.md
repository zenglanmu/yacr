# ADR 0003：浏览器宿主的 Slint / wgpu 组合

状态：采用。范围：规范 §5.3 Web 平台组合、§6 多后端、§9.2 Web 发布。

## 问题

需要在浏览器中交付可运行产物，并满足：单一呈现协调者、CAD 用 wgpu 绘制、
可选 WebGPU / WebGL2、不逐帧 GPU→CPU 回读、UI 与 CAD 业务与 Android 共享。
Slint 官方 Web 路径是 winit + 渲染器，wgpu 后端在 wasm 上可使用 WebGPU 或
WebGL2。需要决定后端选择、事件循环移交与文件/持久化宿主形态。

## 证据

- `i-slint-backend-winit` 在 wasm32 上从 `document.getElementById("canvas")`
  取画布（`winitwindowadapter.rs`），因此宿主只需提供 `<canvas id="canvas">`。
- Slint 1.18.1 的 `WGPUSettings`（`slint::wgpu_30`）公开 `backends` 字段；
  `BackendSelector::require_wgpu_30(WGPUConfiguration::Automatic(settings))`
  可在创建任何窗口前限定 wgpu 后端。
- wgpu 30 在 wasm 上启用 `webgl` feature 时提供 `Backends::GL`（WebGL2）路径；
  `BROWSER_WEBGPU` 仅在 `navigator.gpu` 可用且能真实返回 adapter 时成立。
- `set_rendering_notifier` 在 winit + wgpu 渲染器下提供同一 Device/Queue；
  `i-slint-renderer-femtovg` 支持 `ImageInner::WGPUTexture::WGPU30Texture`
  （`images.rs`），因此现有 `bridge.rs` 在浏览器复用。
- winit 在 wasm 的事件循环以抛出的异常移交控制权
  （`platform_impl/web/event_loop/mod.rs`，文案含 "Using exceptions for
  control flow"）；这是已记录的控制流，不是错误。

## 选择

- **同一画布、同一设备**：与 Android 相同的 `cad-ui-slint` 壳与 `bridge.rs`。
  宿主只增加后端选择与 JS 入口；两个平台不复制 CAD 业务。
- **后端策略**：`Auto` 先真实请求 WebGPU adapter，失败回退 WebGL2；
  强制 WebGPU / WebGL2 通过 `WGPUSettings::backends` 固定。选择写入
  `localStorage` 并整页重建渲染会话（规范 §6 允许重启渲染会话，不要求热切换）。
- **事件循环**：使用 `BackendSelector` 公共 API；winit 的 wasm 异常由
  `main.js` 显式捕获并只忽略该条消息，不掩盖其他错误。
- **宿主最小化**：`apps/app-web/web/main.js` 只负责加载 wasm、File API 选择器、
  批注 JSON 下载；打开/导入都回到 Rust 的同一命令路径。
- **未做双 Canvas 分区**：分区是受阻时的备选（§5.3）；当前组合在无头
  Chromium 中通过，故不引入第二个 WebGL 上下文与事件路由成本。

## 影响

- 需要 `wasm-bindgen-cli` 与 crate 版本严格一致；构建脚本固定 0.2.129。
- WebGL2 基础档仍依赖 wgpu GL 后端；能力检测写入渲染报告。
  未来若需要不依赖 wgpu 的 WebGL2 路径，属于本 ADR 的修订项。
- 真机/浏览器矩阵（移动 Safari 等）未验证，不因无头 Chromium 通过而声明支持。
- 共享内存多线程 Wasm 未启用，COOP/COEP 只作为可选开关。