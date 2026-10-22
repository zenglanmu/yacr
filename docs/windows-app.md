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
- 渲染器：`cad-render-wgpu::SurfaceProbePresenter`，从 `Win32WindowHandle`/`WindowsDisplayHandle`
  构造——**renderer 无改动、无整幅回读、单 presenter**。

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
   - `WS_EX_NOREDIRECTIONBITMAP` 是 **best-effort**：Tauri 2.11.6 没有 builder flag，宿主在窗口
     显示前用 Win32 设置；失败只记诊断（可能有创建白闪），不致命。干净的 `noRedirectionBitmap`
     支持需 Tauri ≥ 2.12.3（见 §7）。**不使用** `WS_EX_LAYERED`（WebView2 在 layered 窗口会渲染异常）。
2. **CAD child**：`CreateWindowExW` 建 `WS_CHILD | WS_VISIBLE | WS_DISABLED | WS_CLIPSIBLINGS`
   的 `YACR_CAD_CHILD`，用 `SetWindowPos(cad, wry_container, …)` 压到 wry 的 `WRY_WEBVIEW`
   容器**紧邻之下**；每次布局/tick 幂等重排（`ensure_order`）。`WS_DISABLED` 是 Wayland 空
   input region 的对应防御位：CAD 表面**永不接收 OS 输入**。
3. **单一输入漏斗**：`.native-input`（DOM）→ `probe_input` IPC → host 准入门/队列 → `cad-host`
   输入管线。**不使用** `WM_NCHITTEST`/`HTTRANSPARENT`，**不声称**具备点击穿透。
4. **几何唯一来源**：已接受的 React CAD rect → `cad_ui_core::layout::LayoutTracker`（logical px
   按 DPI 转 physical）→ child 窗口尺寸。残差**不**靠拉伸图像“修复”。
5. **不变量**：一个 CAD presenter；renderer 只拿 raw handle；无 CPU 帧拷贝/整幅回读。

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
- GPU 与驱动（`dxdiag`/`nvidia-smi`；记录 Vulkan 还是 DX12，`[probe] status backend=… adapter=…`）；
- 显示缩放（100%/125%/150%）与显示器数量；
- 启动命令（含 `--open <fixture>`；**不用** `--probe` 伪装真实 CAD 内容）。

**证据三件套（缺一不算通过）**：①截图（透明 DOM 区域可见 CAD 帧）；②`EnumChildWindows` +
`GetWindowLongPtr(GWL_EXSTYLE)` dump（证明没有 `WS_EX_LAYERED`、CAD 与 `WRY_WEBVIEW` 的兄弟序）；
③Runtime 版本号 + 日志中的 `[probe] win32 z-order (…)`、`[probe] webview2 background alpha read-back=…`。

### 6.1 spike-1：透明 DOM 下方可见 wgpu CAD 帧

**PASS 必须同时满足**：

1. 启动后透明 DOM 区域（`.native-input` 的 CAD 视口）能看到真实 CAD 帧（不是黑块/白块/旧背景），
   且 `[probe] webview2 background alpha read-back=0` 或 `… repaired to 0` 出现；
2. 打开真实 fixture DWG（`--open`，`cad_mode=true`）后画面与 CAD rect 对齐（无拉伸；对照
   `[probe] geometry … dom=…x… native_surface=…x…`，整数舍入内一致）；
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
  - log: alpha read-back=0 | repaired to 0; geometry top=… dom=… native_surface=…
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
- 真 Windows 结论只对“该 Runtime 版本 + 该 GPU/驱动 + 该 Windows 版本 + 该缩放”成立；
  **禁止**推广为“Windows 透明支持”或“跨平台已验收”。

## 9. 已知风险与后续

- **Runtime 版本回归**：透明逐版本敏感；spike-5 的产出决定是否必须 pin Fixed Version。
- **Tauri 2.11.6 没有 `noRedirectionBitmap` builder flag**：当前只能 best-effort 设置
  `WS_EX_NOREDIRECTIONBITMAP`，创建白闪可能残留；干净修法在 Tauri ≥ 2.12.3。
- **依赖锁**：`gpu-allocator 0.28.0` 通过仅 lock 的调整解析到 `windows 0.62.2` 以避免重复
  COM 绑定；未来 `cargo update` 可能重新引入冲突，升级时需复核 `cargo tree -d`。
- **不要**用 `--probe`、空截图或退出码 0 单独证明 Windows 视觉正确。
