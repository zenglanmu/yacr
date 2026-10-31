# macOS 宿主（`Yacr.app` / `yacr-macos`）

> **状态（2026-10-31）**：`apps/app-macos` 已是**真实实现的 Tauri 2 原生宿主**（不再是 NOT
> IMPLEMENTED 失败桩）。合成方向与 Linux/Windows 相同：**透明 WKWebView（React chrome）在上、
> 原生 CAD 子 `NSView` 在下**，渲染器仍是同一个 renderer-only wgpu（macOS 上走 **Metal**）。
> **验收上限 = 编译**：`macos-check` 在 GitHub `macos-latest` 上做 `cargo check`（不运行）；
> 窗口、Metal、透明合成、层序、DPI、原生对话框、配置写入与像素/视觉**全部 NOT VERIFIED**。
> 打包仍未实现（`scripts/package-macos-release.sh` 非零退出）。

## 组成

- `apps/app-macos`（bin `yacr-macos`）：唯一 macOS 宿主 crate。
  - `src/entry.rs`：Tauri 应用 + 单事件循环；`MainEventsCleared` 驱动 `host::tick`，
    `ExitRequested`/`Exit` 先释放 CAD 表面与弹窗模态再退出。非 macOS 目标返回显式
    NOT IMPLEMENTED（二进制非零退出）。
  - `src/host/{init,mod,tick,popup,layers}.rs`：窗口/子 webview 构建、共享状态与 telemetry、
    每 tick 的 CAD 命令/输入/布局/出帧、in-DOM 弹窗生命周期、图层面板投影。
  - `src/macos_surface.rs`：CAD 子 `NSView` + wgpu/Metal presenter 的生命周期与几何权威
    （见下）。
  - `src/platform.rs`：`cad-platform` 的 macOS 实现（文件打开、配置/偏好、DOM surface 度量）。
  - `src/main_thread.rs` / `src/redraw.rs`：共享 `cad-desktop-shell` 主线程调度 seam 的 Tauri
    适配器，以及让单事件循环保持唤醒的 redraw pump（与 Linux/Windows 同一协议）。
- 共享层：`cad-ui-core`（协议/布局/CAD 会话/i18n）、`cad-desktop-shell`（`#[tauri::command]`
  薄壳与共享状态）、`apps/desktop-ui`（唯一桌面 React UI，构建产物 `../desktop-ui/dist`）。
  macOS 宿主直接 import，不保留 re-export 兼容层。
- `cad-render-wgpu` 增加 **Metal** 后端选择：`ProbeBackendPolicy::Metal`（`"metal"`），并且
  `Auto` 在 macOS 上等于 `Backends::METAL`（其他平台仍是 `VULKAN | DX12`）。显式
  `vulkan` 策略在 macOS 上被拒绝（`UnsupportedBackendPolicy`），**没有**静默换后端。
  headless 侧的 `BackendPreference::{Auto,WebGpu}` 在 macOS 同样走 Metal。

## 合成方向与几何

```
NSWindow (Tauri, transparent)
└── contentView (window.ns_view())
    ├── CAD child NSView          ← wgpu/Metal surface, display-only
    └── WKWebView (webview.inner(), React chrome, single input funnel)
```

- CAD 子视图用 `addSubview:positioned:relativeTo:` **插在 WKWebView 之下**
  （`NSWindowOrderingMode::Below`）；`ensure_order` 每 tick 幂等复查兄弟顺序，发现被 wry/主题
  重排后重新插到 webview 之下。CAD 视图不覆盖 responder、不装 gesture recognizer，**永不接收
  OS 输入**；输入唯一漏斗仍是 `.native-input` → `probe_input` IPC。WKWebView 在 `setup` 中按
  Objective-C 类名（`isKindOfClass:WKWebView`）定位：Tauri 只在**异步** `with_webview` 回调里
  暴露其指针，而锚点必须在 `setup` 里同步获得。
- wry 的 **child** webview 在 macOS 上使用 `ViewMinYMargin`（固定尺寸，不随窗口），因此
  `auto_resize()` 在该路径无效：宿主在 `WindowEvent::Resized` 时显式 `webview.set_size()`，
  让唯一 DOM 视口始终等于窗口客户区；CAD 子视图仍只服从 React CAD rect。
- **表面尺寸**只来自 `cad_ui_core::layout::physical_content_size`（与 render session 同一
  `round(side · scale)` 公式），presenter 的 size guard 因此不会因独立取整拒绝每一帧。
- **视图 frame** 用 AppKit 的**逻辑点**（左下原点）：DOM 的 top-left `y` 换算为
  `content_height − (y + height)`；Retina 缩放由层系统处理。
- **Metal 层**：宿主不创建 `CAMetalLayer`。wgpu-hal（经 `raw-window-metal`）把子 `NSView`
  设为 layer-backed 并插入/观察 `CAMetalLayer`，resize 与 backing scale 自动跟随；因此
  表面创建**必须在主线程**完成（`raw-window-metal` 会 panic），`try_init_gpu` 在非主线程时
  显式返回失败而不是 panic。
- present mode 固定 **Fifo**：Metal 不保证广告 `Mailbox`，Windows 的
  `MailboxElseImmediate` 会退到 `Immediate`，因此 macOS 不使用该策略。
- 没有 DirectComposition 对应步骤：presenter 直接呈现到 layer-backed swapchain；成功初始化
  后 `BridgeState::Ready`，布局变化在同一 tick 内 `resize` 到权威尺寸。

## 文件、配置与弹窗

- **打开**：`NSOpenPanel`（AppKit）只允许本地 `.dwg`/`.dxf`；取消是 `FileEvent::Cancelled`，
  读取失败是显式 `Failed`，从不伪造成功。AppKit 要求主线程，因此 `begin_open` 的 worker
  用 `AppHandle::run_on_main_thread` 把面板送到宿主主线程并等待结果（单飞：一次只允许一个
  打开请求，终态事件容量 1）。
- **配置/偏好**：`$HOME/Library/Application Support/yacr/{config.json,preference.json}`；
  `HOME` 不可解析时 store 明确拒绝持久化（回退内存默认），不静默写别处。
  `wgpu_backend` 仍从同一 `preference.json` 读取（`auto`/`vulkan`/`dx12`/`metal`）。
- **保存/导出**：`begin_save` / `write_atomic` 显式 `Unsupported`（与 Windows 宿主同一边界）。
- **弹窗**：仍是同一 React 树的 in-DOM 覆盖层；共享 `cad-desktop-shell::popup` 的模态围栏与
  `close`/`reload`/`exit` 触发保留。**renderer crash 触发 ④ 未实现**：tauri 2.12.3 / wry 0.57
  没有暴露 WKWebView 内容进程终止事件，宿主只打印一条诚实缺口日志，不伪造崩溃通知。
- **透明窗口**：`WindowBuilder::transparent(true)` + `WebviewBuilder::transparent(true)`；
  Tauri 2.12.1+ 不再需要 `macOSPrivateApi`（该配置项为 no-op），本仓库也未启用它。

## 构建与 CI

**唯一编译证据来自 GitHub Actions `macos-latest`**（`.github/workflows/build.yml`）：

- `macos-check`（每次 push/PR）：先 `npm ci && npm test && npm run build` 生成
  `apps/desktop-ui/dist`（`tauri::generate_context!` 在编译期嵌入它），再执行
  `cargo check -p app-macos --all-targets --locked`。**只编译，不运行**；日志上传为 artifact。
- Linux 本机**没有 Apple SDK**，`objc2-exception-helper` 的 `try_catch.m` 不能用宿主 `cc`
  编译。作为**类型检查的替代**（不是构建、不是链接），可以为该 C 步骤提供一个失败即无输出的
  假 `cc`（`cargo check` 不链接），然后运行：
  `CC_aarch64_apple_darwin=<shim> TARGET_CC=<shim> cargo check -p app-macos --target
  aarch64-apple-darwin --all-targets --locked`。
  **边界**：这会让真实 tauri/wgpu/objc2 依赖的 Rust 代码被完整类型检查（含 macOS 专属模块），
  但 **没有** Apple SDK、没有 C 编译、没有链接、没有 Mach-O；**不得**冒充 `macos-check` 或任何
  运行/视觉证据。
- 本机可运行的非 macOS 证据：`cargo check -p app-macos`（空壳/显式拒绝分支）、
  `python3 scripts/test-macos-host.py`（静态接线契约 + 自测）、`check-architecture.py`、
  `check-workflows.py`、`check-i18n.py`。
- **打包未实现**：`scripts/package-macos-release.sh` 仍非零退出；`macos-release` job 仍
  `if: false` 的 RETIRED 占位。没有 `Yacr.app`、签名或 notarization。

## 证据与限制

**已执行（本轮）**

- `cargo check -p app-macos --all-targets --locked`（Linux 主机，非 macOS 分支）通过。
- **macOS 目标的完整类型检查（有界）**：用仅替换 ObjC helper C 步骤的假 `cc`
  （`cargo check` 不链接）运行
  `cargo check -p app-macos --target aarch64-apple-darwin --all-targets --locked` 与
  `cargo clippy -p app-macos --target aarch64-apple-darwin --all-targets --locked -- -D warnings`
  **均通过**。这覆盖真实 tauri/wgpu/objc2 依赖下的 `macos_surface`/`host`/`platform` 类型与
  lint，但**不是** Apple SDK 构建、不是链接、不是运行证据；`macos-check` 才是编译门禁。
- 为让 macOS 目标成立而修改的核心契约：`cad-render-wgpu` 增加 Metal 策略（`Auto` 在 macOS =
  Metal），并把 Vulkan-only 的 VRAM 探测（`gpu_memory::vram`）在 macOS 上显式返回 `None`
  （Metal 无该查询，预算保持 `Unprobed`，**不猜**显存）；`cad-desktop-shell` 的 target 门禁
  加入 macOS。相关 Linux 侧 clippy/单测通过。
- `cargo clippy -p cad-render-wgpu --all-targets --locked -- -D warnings` 与
  `cargo test -p cad-render-wgpu --lib native_presenter::tests`（33 passed）通过：Metal 策略/
  token/`Auto` 平台映射与既有 Vulkan/DX12 行为契约均成立。
- `python3 scripts/test-macos-host.py`、`check-architecture.py`、`check-workflows.py`、
  `check-i18n.py` 通过（源码/清单/工作流静态契约，**不是**运行时证据）。
- **GitHub `macos-check` PASS**（2026-10-31，commit `db40dec`，run
  [41186722245](https://github.com/zenglanmu/yacr/actions/runs/41186722245)，`macos-latest`，
  约 3m09s）：前端构建后 `cargo check -p app-macos --all-targets --locked` 在**真实 macOS
  runner** 上通过；同一 run 的 `linux-app`/`windows-check`/`web-build`/`web-smoke`/
  `shader-validation` 也通过。**这只是编译证据**（`cargo check`，无 `cargo build`、不运行、
  无 Mach-O/签名/打包），不是窗口/Metal/像素验收。

**NOT VERIFIED / NOT RUN**

- 真实 Mac 上的窗口创建、透明合成（WKWebView `drawsBackground=NO` + 窗口非不透明）、
  CAD 子视图层序（是否始终在 WKWebView 之下）、Retina/DPI、Metal adapter 选择、present、
  像素/视觉、OCR/截图、输入事件、`NSOpenPanel`、配置写入。
- 高 DPI 下 `physical_content_size` 与层 bounds/contentsScale 的像素一致性；窗口 resize 时
  `Outdated`/reconfigure 的时延。
- 渲染器 crash 触发 ④（显式未实现）；原生菜单/快捷键、辅助功能、App Store 私有 API
  （wry 的 `drawsBackground`）合规性。
- 性能、功耗（透明窗口在 macOS 上有已知合成开销）、跨 macOS 版本 / Intel Mac 行为。
- 打包、签名、notarization、Gatekeeper。
- 本机 macOS 交叉检查的**真实 C 编译/链接**（ObjC helper 用假 `cc` 跳过，见上）。

以上均不得推广为 macOS 兼容性、视觉正确性或性能结论。历史 Slint 时期（`app-linux` 共享入口、
`rfd`、`--headless` 强制 Vulkan）的实现与证据**已随 Slint 退役**，不得与当前 React/Tauri 宿主混用。
