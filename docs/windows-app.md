# Windows 宿主（`yacr-windows.exe`）

状态：**已实现（2026-10-23），有界验收**。实现形态是 **Tauri 2 + 透明 WebView2(DOM) 在上、
同进程原生 CAD child HWND 在下**（与 Linux 宿主同一合成方向，见
[ADR 0023](adr/0023-dom-on-top-cross-host-composition.md) D1/D6）。验收上限**写死**为：

> **完整实现 + Tier 0 编译门禁 + Wine 拒绝合同 smoke。**
> 视觉/运行时验收 = **用户在真 Windows 自测 spike-1/4/5**（判据见本文 §6）。
> 闭环前一律写 **PENDING user real-Windows spike**，**不得**写“Windows 已验收”或
> “Windows 透明支持”。

Wine 只承担 smoke/诊断级；**任何 Wine 像素/截图结论必须写
`NOT VERIFIED (Wine limitation, bug 55432/58921)`**。Tier 0 编译门禁**不等于**可运行 GUI。

- 应用：`apps/app-windows`；二进制 **`yacr-windows`**（刻意与 Linux 的 `yacr` 区分，避免
  同一 workspace 主机 target 上两个包争抢/覆盖同一个输出路径）。
- 共享逻辑：`crates/cad-ui-core`（protocol/layout/session/shell/popup/i18n/launch）+
  `crates/cad-desktop-shell`（Tauri 命令/状态薄壳、窗口构建、threading、证据骨架）。
- 渲染器：`cad-render-wgpu::SurfaceProbePresenter`，从宿主自建的 **DirectComposition visual**
  构造（`SurfaceTargetUnsafe::CompositionVisual` → `CreateSwapChainForComposition`，**不是**
  HWND-targeted swapchain，见 §2.2）——**无整幅回读、单 presenter**。后端由持久偏好 `wgpu_backend`（`auto|vulkan|dx12`，缺失/
  空/未知 = `auto`）在**每次 presenter 构造**时决定一次：`dx12` = 仅 DX12，`vulkan` = 仅 Vulkan
  （历史行为），`auto` = 两者且同层 Vulkan 优先；当前 target 不支持的策略（如 Linux 上的 `dx12`）
  **显式失败**（`ProbeError::UnsupportedBackendPolicy`，点名策略），**无静默回退**。

---

## 1. 组成与平台缝

`app-windows` 只保留 **Windows 平台缝**；可移植逻辑不在这个 crate 里（ADR 0023 D5）。

| 模块 | 职责 |
|---|---|
| `src/entry.rs` | 启动：解析 launch options → **窗口创建前**检测 WebView2 → 建 `tauri::Builder` → 单事件循环（`MainEventsCleared` tick；`ExitRequested`/`Exit` 释放 surface 并触发 popup 生命周期触发④） |
| `src/support.rs` | WebView2 **缺失运行时拒绝合同**（稳定标记、退出码 5、i18n 键、透明背景 alpha=0 常量）+ registry/filesystem 检测（target-independent 的纯契约部分在任意主机可单测） |
| `src/host/init.rs` | 透明顶层窗口 + 子 WebView2 + CAD child HWND + renderer crash hook |
| `src/win32_surface.rs` | 手写 Win32 child 窗口（`YACR_CAD_CHILD`）、z-order、几何、生命周期；`WS_DISABLED` 纯显示 |
| `src/host/tick.rs` | 每 tick 布局/出帧/`ensure_order()` 幂等重排 |
| `src/platform.rs` | 平台 I/O 缝：`ReactFiles`/`ReactSurface`/`ReactConfigStore`、`rfd` 文件选择器、known-folders |
| `src/host/popup.rs` | 渲染进程失败（`ProcessFailed`）→ 复位 modal gate + `HostErrorKind::Renderer` 诊断 |
| `src/redraw.rs` | Win32 `InvalidateRect` redraw pump（与 GTK `queue_draw` 对应） |

**空壳规则**：非 Windows 目标上 `host`/`win32_surface`/`platform`/`redraw` 不编译（crate 是空壳，
保住 workspace wasm `--lib` 门禁）；二进制返回显式
`yacr-windows: … this platform is NOT IMPLEMENTED` 并以非零退出——**不是**静默空 GUI。

## 2. 合成契约（DOM on top）

1. **DOM/WebView2 在上，CAD child HWND 在下**。
   - Tauri `WindowBuilder`: `transparent(true)` + `background_color(rgba 0,0,0,0)`。
   - WebView2: `transparent(true)` + `background_color alpha=0`。**alpha 必须恰为 0**：wry 会把
     任何非 0 alpha **静默钳成 255**，把“透明”变成不透明且不报错；启动时读回
     `ICoreWebView2Controller2::DefaultBackgroundColor` 记录 `alpha read-back=…`。
   - `WS_EX_NOREDIRECTIONBITMAP`：该 ex-style **只能在窗口创建时**设置，因此宿主在 **Tauri
     `WindowBuilder`** 上用 `no_redirection_bitmap(true)`（tauri **2.12.3**，API 见
     `src/window/mod.rs` 的 `pub fn no_redirection_bitmap(self, enable: bool)`；**自 2.12.0 起存在**）
     在**创建时**请求它——这修掉了旧实现在窗口已存在后 `SetWindowLongPtrW` 的**必然失败**
     （`ERROR_INVALID_PARAMETER`），旧 helper 已删除而非保留。创建后只做**读回验证**，日志如实报告
     `[probe] win32 ex-style: WS_EX_NOREDIRECTIONBITMAP set (creation-time, tauri no_redirection_bitmap)`
     或 `… FAILED: WIN32_ERROR(n) (absent from the created window, ex-style 0x…; the runtime ignored the
     creation-time request)`；读回值才是权威，运行期是否真的落地仍需真 Windows spike（§6/§8）。
     **CAD child**（Tauri 不创建它）用同一 flag，诊断开关 `YACR_PROBE_CHILD_NO_REDIRECT`（§6.0.1）。
     **不使用** `WS_EX_LAYERED`（WebView2 在 layered 窗口会渲染异常）。
2. **CAD child + DirectComposition（合成路径，唯一可用路径）**：`CreateWindowExW` 建
   `WS_CHILD | WS_VISIBLE | WS_DISABLED | WS_CLIPSIBLINGS` 的 `YACR_CAD_CHILD`，用
   `SetWindowPos(cad, wry_container, …)` 压到 wry 的 `WRY_WEBVIEW` 容器**紧邻之下**；每次
   布局/tick 幂等重排（`ensure_order`）。`WS_DISABLED` 是 Wayland 空 input region 的对应防御位：
   CAD 表面**永不接收 OS 输入**。
   **合成（mandated form）**：`Win32CadSurface` 自己持有 `IDCompositionDevice`
   （`DCompositionCreateDevice(None)`）、**顶层窗口**的 `IDCompositionTarget`
   （`CreateTargetForHwnd(top_level, FALSE)` = 层 2，即**在所有子窗口之下**的文档化槽位）、
   根 `IDCompositionVisual`（`CreateVisual` + `SetRoot`）与 `IDCompositionRectangleClip`；
   presenter 只拿到 **visual 的裸指针**（`new_from_composition_visual`）。wgpu-hal 30 据此用
   `IDXGIFactory2::CreateSwapChainForComposition` 建链，并在**每次 configure** 时
   `IDCompositionVisual::SetContent`。
   **DIP 规则**：visual 的 `SetOffset` = CAD rect 的 logical `x/y`（DIP，**不乘 scale**），
   `SetClip`/`SetRect(0, 0, width, height)` = logical `width/height`（DIP），而 swapchain 仍是
   **physical** 尺寸（`physical_content_size`，§2.4 的尺寸权威）。clip 到自身内容矩形意味着
   布局/尺寸竞态最多“少显示”，绝不会把 CAD 像素溢出到 chrome 上。
   **Commit 协议**：视觉树的改动只在 `IDCompositionDevice::Commit()` 后生效。宿主按两个条件提交：
   presenter 的 `configure_generation` 变化（init / `resize` / 内部 `Outdated` 重配），**或**
   layout 驱动的 offset/clip 变化（`apply_layout` 记录 pending placement，**即使尺寸没变**也提交，
   否则纯移动的重排会一直停在旧位置；普通 tick 是 no-op）。成功 reconfig 日志
   `[probe] surface reconfigure A -> B`，每次布局应用日志
   `[probe] composition placement offset=(x,y) clip=WxH dip swapchain=WxH`。
   **安全契约**：device/target/visual/clip 由宿主持有并必须在 surface 存活期间存活；释放顺序是
   presenter(surface) → composition 树 → child HWND（`release_gpu`/`Drop` 强制）；visual 指针在
   host 结构存活期内有效，wgpu 另持一份 COM ref。
   **为什么不挂在 child 上**：`CreateTargetForHwnd` 接受子窗口（MSDN 允许，不能称非法），但在
   **本配置**下 child-attached 内容（flip 或 DComp）都到不了合成输出，机制未定（RC1：父窗口带
   `WS_EX_NOREDIRECTIONBITMAP` 时 child-attached 内容不并入合成输出；RC2：`WS_CLIPSIBLINGS`
   + 全客户区 `WRY_WEBVIEW` 兄弟窗口把 CAD child 的可见区域裁空 —— 判别器分别是
   `YACR_PROBE_GDI_MARKER` 与 `YACR_PROBE_NO_CLIPSIBLINGS`，见 §6.0.1）。顶层 target 的
   `topmost=FALSE` 槽位是**文档化**的“在子窗口之下”层，正是 DOM-on-top；这也是 wgpu
   （`Dx12SwapchainKind::DxgiFromVisual`：supports transparent windows）与 wry 示例采用的形态。
   **约束**：同一窗口最多两个 DComp target（每层一个，我们只用层 2 这一个）；visual 几何一律 DIP。
   **没有** HWND swapchain 回退路径：composition 创建/commit 失败 = 显式
   `cad-present-error class=surface_composition`（`BridgeState::Failed`），不是静默黑区。
   就绪行带 `surface=composition target=top`（`target=child` 仅作为**类型化证据 token**保留，
   用于 build-to-build 区分；Linux/GTK 宿主仍是 `surface=hwnd`）。
3. **单一输入漏斗**：`.native-input`（DOM）→ `probe_input` IPC → host 准入门/队列 → `cad-host`
   输入管线。**不使用** `WM_NCHITTEST`/`HTTRANSPARENT`，**不声称**具备点击穿透。
4. **几何唯一来源 + 尺寸单一权威**：已接受的 React CAD rect → `cad_ui_core::layout::LayoutTracker`
   （logical px）→ child 窗口。**尺寸**由 `cad_ui_core::layout::physical_content_size(rect, scale)`
   决定——它**委托**渲染会话自己的 `cad_engine::render::render_target_size`（同一次
   `round(side·scale)`）——**位置**才独立取 `round(x·s)`。因此不变量是
   **`dom_physical == native_surface == child_desired`**，且必须在**运行时**成立：每次布局变化，
   `Win32CadSurface::apply_layout` 在移动 child 的**同一步**把 swapchain 重配到权威尺寸
   （`[probe] surface reconfigure <from> -> <to>`；失败 = `cad-present-error
   class=surface_reconfigure`），帧路径的 `resize_to` 只是幂等兜底。不满足即 **FAIL**（日志
   `[probe] geometry … size_authority=ok|FAIL`）；presenter 的尺寸守卫只是**兜底**，不是尺寸来源。
   **禁止**再用“缩放后的 insets 相减”（`round(container·s) − round(start·s) − round(end·s)`）推导
   尺寸：该第二条舍入路径在 125%/150% 等分数 DPI 下可与 `round((container−start−end)·s)` 差 1 px，
   而 presenter 对尺寸不一致的纹理**每帧拒绝**（`TextureInvalid`），症状是 CAD 区**全透明** +
   粘住的 `cad present failed`。残差**不**靠拉伸图像“修复”。
5. **不变量**：一个 CAD presenter；renderer 只拿到宿主创建的 composition visual 指针（从不拿
   窗口句柄、也不让 wgpu 对象外泄）；无 CPU 帧拷贝/整幅回读。

## 3. WebView2 缺失 = 显式 Unsupported（退出码 5）

React chrome 本身就是 WebView2 文档：运行时缺失 = 没有 UI。宿主在**创建任何窗口之前**检测，
失败时输出：

```
yacr: unsupported-runtime: webview2      ← 稳定标记行（第一行，证据分类依据）
[probe] webview2 runtime missing: …      ← 技术细节
<startup.webview2_missing>               ← 本地化标题
<startup.webview2_missing.detail>        ← 本地化详情
→ exit 5
```

- **绝不**空白窗口、**绝不**静默回退。
- 检测 registry-first（Edge Update client key）+ 文件系统回退，并尊重 Fixed Version 策略
  （`WEBVIEW2_BROWSER_EXECUTABLE_FOLDER`）。
- **检测证明“存在”，不证明“可用”**：注册了但损坏的运行时会在窗口/WebView 构建处显式失败
  （exit 1 + 明文错误）。
- 分类纪律：证据脚本以**稳定标记行**为第一依据，退出码 5 仅辅助。

## 4. 退出码注册表（全宿主同源，见 `docs/build.md`）

| 码 | 含义 | Windows 产点 |
|---|---|---|
| 0 | 成功（含正常退出） | `entry.rs` 事件循环自然结束 |
| 1 | 通用失败（Tauri 构建失败、启动合同拒绝、窗口/WebView 构建失败） | `app-windows/entry.rs`、`main.rs` |
| 2 | 无效/不可用的显示后端策略（Linux） | —（Windows 不使用） |
| 3 | postload-pan 证据驱动失败（Linux） | — |
| 4 | 不支持的后端（X11/XWayland，Linux） | — |
| **5** | **不支持的运行时（WebView2 缺失）** | `app-windows/src/support.rs` |

## 5. 构建与门禁（Tier 0）

**CI**：`.github/workflows/build.yml` 的 `windows-check` 在 `windows-latest` 上跑
`cargo check -p app-windows --all-targets --locked`（原生 MSVC，编译门禁，不运行 exe）。
`windows-release` 仍为 `if: false` 退役状态（`scripts/package-windows-release.sh` 非零退出），
**没有 Windows 发布包流水线**。注意：workflow 里的 summary 文案仍沿用“NOT IMPLEMENTED
placeholder”的旧措辞，与实际代码不符；它是**文案腐化**，不影响编译门禁本身，但不得据此认为
Windows 仍是占位。

**本机（Linux，WP4 门禁实跑，日志在 `$HOME/.cache/yacr-wine/logs/`）**：

```bash
# 主机测试/格式/lint
cargo fmt -p app-windows -- --check
cargo clippy -p app-windows --all-targets --locked -- -D warnings
cargo test -p app-windows --lib --locked

# MSVC 目标（与 CI 同 target；需要 llvm-rc 在 PATH）
PATH=/usr/lib/llvm-21/bin:$PATH cargo xwin clippy -p app-windows \
    --target x86_64-pc-windows-msvc --locked -- -D warnings
PATH=/usr/lib/llvm-21/bin:$PATH cargo xwin check -p app-windows \
    --target x86_64-pc-windows-msvc --locked
PATH=/usr/lib/llvm-21/bin:$PATH cargo xwin build -p app-windows \
    --release --target x86_64-pc-windows-msvc --locked

# MinGW GNU 目标（交叉链接）
cargo build -p app-windows --release --target x86_64-pc-windows-gnu --locked

# 非 Windows 空壳隔离（workspace wasm 门禁的一部分）
cargo check -p app-windows --lib --target wasm32-unknown-unknown --locked
```

WP4 门禁结果：上列全部 **rc=0**；另有 `check-i18n.py`（716 键 / 2 目录）、
`check-file-size.py`（851 文件，上限 1000）、`check-architecture.py`（28 包）通过；
MSVC release 产物约 27 MB。**编译门禁 ≠ GUI**：`cargo xwin build` 只证明可链接。

## 6. 用户真 Windows 自测：spike-1/4/5

spike-2/3（原生输入穿透、DOM 转发延迟）**不做**：输入走与 Linux 同构的单一 DOM 漏斗，
CAD child 是 `WS_DISABLED` 纯显示表面，不需要 hit-testing 穿透。

### 6.0 运行前固定记录（每次 spike 都要）

- OS 版本/build（`winver`，如 `Windows 11 24H2 26100.xxxx`）；
- WebView2 Runtime 版本与策略（Fixed Version 目录或 Evergreen；对应
  `[probe] webview2 runtime kind=… version=… source=…`）；
- GPU 与驱动（`dxdiag`/`nvidia-smi`；记录 Vulkan 还是 DX12，`[probe] status backend=… adapter=…`），
  并记录本次构造实际采用的策略行 `[probe] backend-policy wgpu_backend=… (…)` 与
  `[probe] … presenter ready backend=…`；**DX12 路径在真 Windows 上 NOT RUN**（Tier 0 编译 +
  Wine 拒绝合同不构成 DX12 运行证据）；
- 显示缩放（100%/125%/150%）与显示器数量；
- 启动命令（含 `--open <fixture>`；**不用** `--probe` 伪装真实 CAD 内容）。
- **无硬件 Vulkan 的 VM**：证据运行可设 `YACR_PROBE_ALLOW_CPU_ADAPTER`（**存在即生效**，取值无关），
  允许 CPU/软件 Vulkan 适配器（如 lavapipe）被 probe presenter 选中；硬件适配器仍优先，**未设**
  时 CPU 适配器仍被拒绝。该情形属软件渲染，spike 结论必须显式标注，**不得**记为真实 GPU 证据。

**证据三件套（缺一不算通过）**：①截图（透明 DOM 区域可见 CAD 帧）；②`EnumChildWindows` +
`GetWindowLongPtr(GWL_EXSTYLE)` dump（证明没有 `WS_EX_LAYERED`、CAD 与 `WRY_WEBVIEW` 的兄弟序）；
③Runtime 版本号 + 日志中的 `[probe] win32 z-order (…)`、`[probe] webview2 background alpha read-back=…`。

### 6.0.1 诊断开关与诊断标记（**均不是验收证据**）

以下开关与日志行只用于**定位**故障域，产出**不构成**视觉/运行验收，不得写进 PASS 结论：

- `YACR_PROBE_SOLID_FRAME`（**存在即生效**，取值无关）：`frame_cad` 跳过纹理 blit **与**尺寸守卫，
  直接 acquire → `LoadOp::Clear` 到品红/橄榄交替色 → present（`[probe] solid-frame active
  (diagnostic; not acceptance evidence)` 启动各一次）。用于判别“表面根本 present 不了” vs
  “纹理/尺寸路径有缺陷”；交替色让旧缓冲重放也可见。**不是**产品路径。
- `YACR_PROBE_GDI_MARKER`（**存在即生效**）：在 CAD child 的 `WM_ERASEBKGND`/`WM_PAINT` 里用 GDI
  `FillRect` 画一个 **40×40 纯红方块**（child 左上角），并在 install 时 `InvalidateRect` 强制一次
  paint。用途 = **V1 标定器**：先回答“**我们**子窗口自己的内容到底有没有被合成/是否能在抓屏里看到”，
  这是 0-delta 证据缺失的那一步；不触碰 swapchain 路径。激活日志
  `[probe] gdi-marker active: 40x40 red square at the CAD child's top-left (diagnostic; not
  acceptance evidence)`。
- `YACR_PROBE_NO_CLIPSIBLINGS`（**存在即生效**）：创建 CAD child 时**去掉** `WS_CLIPSIBLINGS`
  （只丢这一位，两个变体都被单测钉住）。用途 = **RC2 判别器**：验证“全客户区的 `WRY_WEBVIEW`
  兄弟窗口是否把 CAD child 的可见区域裁空”。激活日志
  `[probe] cad-child style: WS_CLIPSIBLINGS dropped (diagnostic; not acceptance evidence)`。
- `YACR_PROBE_DOM_OVERLAY`（**存在即生效**）：注入的初始化脚本额外给 `.workspace__canvas` 加
  inline `background: rgba(255,0,255,0.5)`。用途 = **V0 判别器**：单独判断 DOM/WebView2 链路在
  该环境下是否真的做 alpha 混合（VM 的“平色”与笔记本的“透视”可能是两个不同的子故障）。激活日志
  `[probe] dom-overlay active: .workspace__canvas painted rgba(255,0,255,0.5) (diagnostic; not
  acceptance evidence)`。
- `YACR_PROBE_CHILD_NO_REDIRECT`（**存在即生效**）：CAD **child** 的 `CreateWindowExW` ex-style 加
  `WS_EX_NOREDIRECTIONBITMAP`——该 flag **只能在创建时**设置，所以 child 只能在创建时带上它
  （顶层窗口现在已由 §2.1 的 `WindowBuilder::no_redirection_bitmap` 在创建时请求；Tauri 不创建
  CAD child，故本开关是它的创建时入口）。启动后读回并记录
  `[probe] win32 cad-child ex-style: WS_EX_NOREDIRECTIONBITMAP set` 或
  `… failed: WIN32_ERROR(n)`。默认（变量不存在）= ex-style 为空，行为不变。
- 诊断标记行 `[probe] cad-present-error class=<apply_updates|render|device_lost|acquire|texture_invalid|frame_validation|surface_reconfigure|surface_composition|gpu_memory> text=…`：
  每次 present/布局失败在状态栏文案**之前**输出；class 来自 typed `ProbeError`/`RenderError`/布局
  失败类型（**不是**文本匹配），状态栏 `cad present failed: …` / `win32 CAD child placement failed: …`
  文案不变。`surface_reconfigure` = child 放置或 swapchain 没能跟到尺寸权威（见 §2.4）；
  `surface_composition` = DirectComposition 设备/目标/视觉创建或 `Commit` 失败（§2.2：没有可用的
  HWND 回退路径，内容将永远不合成）；`gpu_memory` = 显式显存拒绝（应用预算或设备 OOM，
  `cad_render_wgpu::RenderError::GpuMemory`）——**不是**设备丢失也不是普通帧错误，保留常驻场景
  与设备（与 Linux 宿主同名 token）。连续相同的标记只打印一次（下一次成功 present 会重置）。
- 诊断标记行（每次真实渲染，**不是**旧纹理重放）`[probe] render-exec rev=… size=WxH batches=… instances=…`：
  `rev` = 该次渲染时已提交的场景 revision，`size` = 渲染目标（尺寸权威），`batches`/`instances` =
  实际画出的 instanced 批/段。只在 renderer **真的执行**时出现（observation 只交付一次）。
- 尺寸权威运行时标记 `[probe] surface reconfigure <from> -> <to>`：每次 child 布局变化后 swapchain
  成功跟到 `physical_content_size` 的新尺寸时输出一次；失败走上面的
  `cad-present-error class=surface_reconfigure`。不变量 `dom_physical == native_surface ==
  child_desired` 必须在**运行时**每次布局变化后成立，`[probe] geometry … size_authority=ok|FAIL`
  是判定（`FAIL` = presenter 尺寸守卫会拒绝每一帧）。
- 诊断标记行（1 Hz）`[probe] actual-render batches=… instances=… pulled=… frames_rendered=… dirty=… rendered_rev=… committed_rev=…`：
  真实渲染活动。`[probe] status … frames=` 只数 present 调用（含旧纹理重放），所以
  **`frames=` 递增而 `frames_rendered=` 不增 = 只在重放旧纹理**，不能当作“CAD 有渲染”的证据。
  两个 revision 区分剩余歧义：`rendered_rev` = **烘进保留纹理**的场景 revision（完整渲染前为
  `None`），`committed_rev` = GPU 已提交的**最新**场景 revision。`committed_rev > rendered_rev`
  且 `dirty=false` = dirty/commit 记账没有重新触发渲染；`committed_rev == rendered_rev` 且有
  `render-exec` 执行而像素仍全黑 = 光栅化/裁剪问题（另一条调查线，不是本标记的结论）。

### 6.1 spike-1：透明 DOM 下方可见 wgpu CAD 帧

**PASS 必须同时满足**：

1. 启动后透明 DOM 区域（`.native-input` 的 CAD 视口）能看到真实 CAD 帧（不是黑块/白块/旧背景），
   且 `[probe] webview2 background alpha read-back=0` 或 `… repaired to 0` 出现；
2. 打开真实 fixture DWG（`--open`，`cad_mode=true`）后画面与 CAD rect 对齐（无拉伸），且
   `[probe] geometry … size_authority=ok`——即 `dom_physical == native_surface == child_desired`
   （三者都是 `physical_content_size` 的同一个数）；出现 `size_authority=FAIL` 即 FAIL，不允许
   “整数舍入内一致”的宽容解释；
3. resize / 最大化 / 还原后**不残留旧背景**（#2419 类症状），CAD 帧随 CAD rect 重新贴合；
4. 事件循环持续出帧（`[probe] status … frames=` 递增），无 WebView2 进程崩溃/重启循环。

**FAIL**：任一不满足即 FAIL，并按故障域切分（先跑“纯 wgpu child + 不透明顶层窗口”基线，再叠
透明 WebView2）。**不要**改成“空成功”；按 ADR 0023 D6 停下来与用户决策（接受 Windows 显式
`Partial`/Unsupported 降级 vs tiling 分片方案 C）。

```
spike-1: <PASS|FAIL>
win: <OS build> / webview2: <version> (<FixedVersion|Evergreen>) / gpu: <adapter+driver> / scale: <n>%
cmd: yacr-windows --open <fixture.dwg>
evidence:
  - shot-before-resize.png, shot-after-maximize.png, shot-after-restore.png
  - ex-style dump: window=<0x…> exstyle=<0x…> layered=<no> noredirectionbitmap=<yes|no>
  - child order: container=<0x…> below=<cad HWND> (GetWindow(GW_HWNDNEXT))
  - log: alpha read-back=0 | repaired to 0; geometry top=… dom=… native_surface=… child_desired=… size_authority=ok
  - log: cad-present-error class=<…> (none | one line) ; actual-render frames_rendered=<n> (must increase while CAD renders)
  - child ex-style (only with YACR_PROBE_CHILD_NO_REDIRECT): noredirectionbitmap=<set|failed: WIN32_ERROR(n)>
notes: <残留/对齐/崩溃等观察>
```

### 6.2 spike-4：Z-order 稳定性

操作序列（每步后检查兄弟序）：① 反复 show/hide（最小化/还原）；② 最大化/还原（含跨显示器）；
③ DPI 切换（100% ↔ 125%/150%）；④ 打开 WebView2 右键菜单；⑤ 打开 DevTools（若启用）；
⑥ 拖动 resize 与多显示器切换。

**PASS**：每一步之后 `GetWindow(container, GW_HWNDNEXT) == cad`（或日志出现
`[probe] win32 z-order (re-assert)` 且 CAD 仍可见），CAD 从未被永久遮挡；DOM 仍可交互
（点击命中 React 控件，CAD 不吃 OS 输入）。

**FAIL**：任一操作后 CAD 被容器覆盖且 `ensure_order()` 无法恢复，或出现闪烁/每帧抢序
（每 tick 刷 `re-assert` 也算不稳定）。

```
spike-4: <PASS|FAIL>
steps:
  show/hide:  order=<cad_below_container:yes|no> observed=<…>
  maximize/restore: <…>
  dpi-change (100->150): <…>
  webview2-context-menu: <…>
  devtools: <…>
  resize/multi-monitor: <…>
log-count: re-assert lines=<n> over <duration>s (steady-state should be ~0)
```

### 6.3 spike-5：Runtime 版本敏感性

透明是**逐 Runtime 版本回归**的能力：145.x #5481 已修；**154.0.4258.62 #5752 open**，而报告环境
正是 Tauri v2/wry 0.55.1（本项目栈）。

**PASS**：在 **154.0.4258.62** 与**一个较早版本**上分别跑 spike-1 的透明检查，两者 alpha 读回
均为 0 且视觉可见 CAD 帧 → 可选 Evergreen + 版本入证据。若任一版本失败 → **必须 pin Fixed
Version**（`WEBVIEW2_BROWSER_EXECUTABLE_FOLDER`），并把版本写入发布配置与证据边界。

**FAIL/不可判定**：只跑过一个版本或拿不到第二个版本 → 标 `PARTIAL`，默认按 **pin Fixed
Version** 处理（保守，不得宣称“Windows 透明支持”）。

```
spike-5: <PASS|FAIL|PARTIAL>
runtime-A: <version> -> alpha=<0|255>, cad_visible=<yes|no>, shot=<file>
runtime-B: <version> -> alpha=<0|255>, cad_visible=<yes|no>, shot=<file>
decision: <Evergreen+version-recorded | pin FixedVersion=<version> | PARTIAL->default-pin>
```

## 7. Wine smoke（诊断级，不是验收）

Wine 的 DirectComposition 是 stub（bug 58921），目标形态（透明 + `NOREDIRECTIONBITMAP`）落在
bug 55432 的黑屏分支 ⇒ Wine **无法**验收透明、层序或视觉。Wine 只承担：

- 启动不崩；WebView2 检测与**拒绝诊断正确**（标记行/i18n/exit 5 端到端）；
- 已知缺陷特征记录（`fixme:dcomp:*`、GPU 进程重启循环）；
- 故障域切分。

WP4 实跑（Wine 10.0，专用 64 位 prefix，prefix 内无 WebView2）：

| # | 产物/场景 | 退出码 | 观察 |
|---|---|---|---|
| W1 | GNU release，Evergreen/Fixed 均缺失 | **5** | 标记行为第一行，随后 `[probe]` 细节与 zh-CN 标题/详情；无窗口 |
| W2 | GNU release，`WEBVIEW2_BROWSER_EXECUTABLE_FOLDER=C:\no-such-webview2` | **5** | 细节点名坏 Fixed-Version 路径（`msedgewebview2.exe is not there`） |
| W3 | MSVC release（真实交付目标），无运行时 | **5** | 同 W1；MSVC 产物静态链接 loader，不依赖 `WebView2Loader.dll` |

日志/rc 在 `$HOME/.cache/yacr-wine/{logs,smoke,smoke-msvc}/`。GNU 产物需在 exe 旁放
`WebView2Loader.dll` 才能在 Wine 走到 `main`（仅 smoke 目录，不入库）。

**所有 Wine 视觉结论固定 `NOT VERIFIED (Wine limitation, bug 55432/58921)`。**

## 8. NOT RUN / 未验证（不得推广）

- 真 Windows 上的**透明视觉**、层序稳定性、DPI/多显示器、`ProcessFailed` 活体路径、文件选择器、
  配置目录写入、真实 GPU（DX12/Vulkan）出帧与性能 —— 全部 **NOT RUN / PENDING user real-Windows
  spike**。
- GitHub Actions 的 `windows-check` job 本环境**未运行**（无 runner）；只有本机 xwin/MSVC 编译证据。
- Windows 发布打包（`scripts/package-windows-release.sh` 仍非零退役）；未做代码签名。
- 真实 OS 键盘/指针从未用于验证任何输入路径（所有宿主同此边界）。
- **wgpu 30 的 present 状态盲区**：`Queue::present` 返回 `()`，`Ok` 状态被 API 丢弃，present 期间的
  validation 错误只会进未捕获错误处理器（panic）。本宿主因此把 present 包进显式 validation scope
  （失败 → `ProbeError::Frame("present rejected: …")`），但**不得**把“没有错误”当作 present 成功
  的证明；present 本身在真 Windows 上仍 **NOT RUN**。
- 真 Windows 结论只对“该 Runtime 版本 + 该 GPU/驱动 + 该 Windows 版本 + 该缩放”成立；
  **禁止**推广为“Windows 透明支持”或“跨平台已验收”。

## 9. 已知风险与后续

- **Runtime 版本回归**：透明逐版本敏感；spike-5 的产出决定是否必须 pin Fixed Version。
- ~~**Tauri 2.11.6 没有 `noRedirectionBitmap` builder flag**~~：**已修（2026-10-23）**——Tauri 升级到
  **2.12.3**（`tauri-build 2.7.2`，随动 `tauri-runtime(-wry) 2.12.x`、`tauri-macros/codegen 2.7.x`、
  `tauri-utils 2.10.x`、`tao 0.37`、`wry 0.57`），顶层窗口改为**创建时**请求
  `no_redirection_bitmap(true)`，旧的后创建 `SetWindowLongPtrW` 路径已删除，创建后只读回验证
  （§2.1）。**done pending runtime verification**：真 Windows 上“创建白闪是否消失”仍
  **PENDING user real-Windows spike**（§6.1/§8），编译/读回日志不构成视觉证据。
- **HWND flip swapchain 在透明顶层窗口下不可合成（已定性，改走 DirectComposition）**：wgpu-hal 30
  的 dx12 后端硬编码 `DXGI_SWAP_EFFECT_FLIP_DISCARD`，该链只对**顶层** DWM visual 生效，`WS_CHILD`
  没有 visual。VM 实测（两个 no-redirect flag 都开 + `YACR_PROBE_SOLID_FRAME=1`）：
  `write=0 acquire=45 submit=324 present=255 error=None`，ROI 内 0 品红/0 橄榄采样 —— 帧被生产/
  present 但不被合成。现已改为 **`CreateSwapChainForComposition` + `IDCompositionVisual`**（§2.2），
  旧 HWND 路径在 Windows 宿主**已删除**（不是保留为静默回退）。**done pending runtime
  verification**：真 Windows/VM 上 CAD 区是否真的出现像素仍 **NOT RUN**；编译与日志不构成视觉证据。
- **依赖锁**：`gpu-allocator 0.28.0` 通过仅 lock 的调整解析到 `windows 0.62.2` 以避免重复
  COM 绑定；未来 `cargo update` 可能重新引入冲突，升级时需复核 `cargo tree -d`。
- **不要**用 `--probe`、空截图或退出码 0 单独证明 Windows 视觉正确。

---

## TODO-ADR-0023-amendment draft（草稿，未提交；`docs/adr/**` 不在本轮写入范围）

> 本节是给 ADR 0023 的**修订草案**，不是生效的 ADR 正文。不要把它当作规范引用。

**背景（事实，非推测）**：ADR 0023 规定“DOM/WebView 在上、CAD 原生表面在下、且不接收 OS 输入”，
Windows 侧的形态是透明顶层窗口（`WS_EX_NOREDIRECTIONBITMAP`）+ 同进程 `WS_CHILD` CAD 表面。
ADR 原文把“原生表面如何出像素”视为渲染器内部细节，未规定 Windows 的**合成**方式。

**需要补的一条（D6 增补）**：Windows 的原生 CAD 表面**必须**通过 DirectComposition 合成，且形态
固定为：宿主创建 `IDCompositionDevice` + **`CreateTargetForHwnd(top_level, FALSE)`（顶层窗口，
非 child）** + 根 `IDCompositionVisual` + `IDCompositionRectangleClip`；渲染表面是
`IDXGIFactory2::CreateSwapChainForComposition` 的 swapchain，绑定方式为
`IDCompositionVisual::SetContent` + `IDCompositionDevice::Commit`。视觉几何遵循两条约束：
(a) **DIP 单位**——`SetOffset` = logical `x/y`（不乘 scale），`SetClip/SetRect` = logical
`width/height`，swapchain 仍是 physical 尺寸；
(b) **每个窗口最多两个 DComp target**（按层），本设计只占 `topmost=FALSE` 的层 2（在子窗口之下）。

**理由**：
1. 该窗口必须是透明的（半透明装饰面板是产品设计，ADR 0023 D1/D6），而 tao 的透明实现落在
   顶层窗口的空 opaque region / `WS_EX_NOREDIRECTIONBITMAP` 上。
2. **（更正，勿再引用旧表述）** 旧稿写的“`WS_CHILD` 没有自己的 visual / HWND 链只对顶层 DWM
   visual 生效”是**过度断言**：`IDCompositionDevice::CreateTargetForHwnd` 的当前 MSDN 明确允许
   **子窗口**（有商业实现如此用），因此“child target”本身**不违法**。事实只能说到：在**本配置**下
   child-attached 内容（无论 HWND flip 链还是 DComp child target）**都到不了合成输出**，机制尚未
   定论，有两个候选且各有判别器：
   - **RC1**：父窗口带 `WS_EX_NOREDIRECTIONBITMAP` 时，child-attached 内容不并入合成输出；
     判别器 = `YACR_PROBE_GDI_MARKER`（V1，先证明“我们自己的子窗口内容是否可见”）。
   - **RC2**：`WS_CLIPSIBLINGS` + 全客户区 `WRY_WEBVIEW` 兄弟窗口把 CAD child 的可见区域裁空
     （MSDN：合成层被窗口可见区域裁剪；子窗口可能裁剪视觉树）；判别器 =
     `YACR_PROBE_NO_CLIPSIBLINGS`。
   另一方面，**顶层 target + `topmost=FALSE`** 是有文档层语义的配置：“在所有子窗口之下”，因此
   DOM-on-top 是**构造保证**，而且这是 wgpu（`Dx12SwapchainKind::DxgiFromVisual` = transparent
   windows）与 wry 示例采用的形态。即：不是“child target 非法”，而是“child target 在本配置下
   未被证明可达合成输出，顶层 target 才是已知良好单元”。
3. DirectComposition 正是 Windows 上“把一个表面按视觉树合成到某窗口区域内”的公共机制；它让
   “CAD 内容在 DOM **下方**合成”成为可实现的层序，与 ADR 0023 的层级语义一致，并且不需要
   `WS_EX_LAYERED`（WebView2 在 layered 窗口渲染异常）、不需要点击穿透、不改变单一输入漏斗。
4. 代价与约束（应写入 ADR）：宿主持有 composition 设备/目标/视觉/clip 的生命周期，必须在每次
   `configure` **以及每次 offset/clip 变化**后 `Commit`（纯移动的重排不会改 `configure` 代数）；
   surface 证据必须带 `surface=composition target=top` 类型化 token（`target=child` 仅为
   build-to-build 区分保留）；composition 不可用时必须显式失败
   （`class=surface_composition`），**不得**回退到未证明可达合成输出的 HWND 链。
5. **不改动 ADR 0023 §5**：平台合成 ADR（RenderTarget/HostTexture token 所有权）仍**显式延期**，
   在 `docs/handoff.md` 中原样保留；本增补只规定 Windows 的视觉合成形态，**不**解决 token 所有权。
