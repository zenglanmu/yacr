# ADR 0006：React + Tauri 原生 Linux 宿主实验（X11/XWayland-only 探针）

> **已被取代 / 定位更新（2026-10-16）**：本次**实验已被采用为正式桌面实现**，见
> [ADR 0010](0010-react-linux-host-replaces-slint.md)。下文“实验/未删旧 Slint 代码/Web 仍以 Slint
> 为基线”等描述已过时（Slint 已删除）；宿主使用说明见 `docs/react-linux-app.md`。
> 下文保留为实验过程记录。

状态：实验（用户批准替代目标；**bounded prototype 已达**，仍**非完整 CAD 产品验收**）。
最新（2026-10-14 第四轮）：Linux **GTK 中央 native 分区（nativechild）** 与 React UI **co-visible**，
实际 CAD（native Rust + P620/Vulkan）达到**有界原型**验收（见末节）；顶层透明 alpha full overlay
仍为历史 FAIL。Win/macOS 仍 **NOT IMPLEMENTED / NOT RUN**。
范围：用户批准将 Linux 桌面 UI 目标由 Slint 替换为 Tauri2 + React，并新增
**X11/XWayland-only** 的 GPU 合成探针；明确**不允许 WASM**、**不允许逐帧 readback**。
同时保留 `crates/cad-render-wgpu::SurfaceProbePresenter` 作为唯一 GPU surface 探针。

## 问题

需要证明「React/Tauri 原生 Linux 窗口 + 真实硬件 GPU 的 Vulkan 呈现 + GTK/WebKit 平台合成」
这条替代路线在真机 XWayland 上**可编译、可驱动**，作为替换 Slint 的可行性探针。
要求最小、独立、可销毁重建的探针，而**不是**正式 CAD presenter：不得以假数据或占位冒充
CAD 能力，失败必须显式状态。

## 证据（本轮实际执行）

环境（只读诊断，见 `docs/handoff.md` 实验段）：

- GNOME/mutter Wayland 会话，XWayland `:0` 可连接（`xdpyinfo` 退出码 0，扩展含
  `Present/GLX/DRI3/XWAYLAND`）；NVIDIA Quadro P620 + Intel UHD 双显卡；
  `/usr/share/vulkan/icd.d/nvidia_icd.json` 可用。
- 系统 dev 依赖已由主协调者在后台 apt 安装：`libwebkit2gtk-4.1-dev 2.52.6`、
  `libgtk-3-dev 3.24.52`、`libjavascriptcoregtk-4.1-dev`、`libsoup-3.0-dev` 等
  （`pkg-config --exists webkit2gtk-4.1 gtk+-3.0` 通过）。

编译/测试门禁（本机，debug）：

- `cargo check -p app-react-linux --all-targets --locked` → 通过。
- `cargo test -p app-react-linux --lib --locked` → **10 passed / 0 failed**（协议契约）。
- `cargo clippy -p app-react-linux --all-targets --locked -- -D warnings` → 干净。
- `python3 scripts/check-architecture.py` → 28 包，`Architecture OK`。
- 依赖解析：`tauri 2.11.6`（镜像上 `tauri-build` 独立版本线最高 2.7.2）。

## 选择

- Linux 桌面 UI 目标改为 **Tauri 2.11.6 + React**；前端代码归 designer，
  scope `apps/app-react-linux/ui/**`，本轮不动。
- **本轮保留旧 Slint 代码**（`crates/cad-ui-slint`、`apps/app-linux` 等不删不改）；
  新探针**不使用旧 Slint presenter**，二者互不依赖。
- 探针在**顶层 Tauri 窗口**的 X11 surface 上经 `cad-render-wgpu::SurfaceProbePresenter`
  呈现（Vulkan-only、拒绝 CPU adapter、偏好 `P620` 但不硬编码、无 readback、绘制识别网格 +
  移动色块 + 角标）；GTK/WebKit 子 Webview 作为**平台合成桥**叠加。
- **不声称同 device**：GTK/WebKit 与 wgpu Vulkan **不是**同一 device/queue 的所有者；
  CAD 内容与 UI 的合成为**平台（GTK/X11）合成**，不同于 ADR 0002 的 Slint 共享 Device 合成。
  本实验不主张共享 Device、也不主张单一渲染协调者。
- 强制 **`GDK_BACKEND=x11`**：已设置 `x11` 通过；未设置则宿主显式置为 `x11`；设置为其它值
  直接报错退出（不做静默回退），见 `apps/app-react-linux/src/backend.rs`。
- 版本固定：tauri 2.11.6 与其内部族 `tauri-build 2.6.3 / tauri-macros 2.6.3 /
  tauri-runtime 2.11.3 / tauri-runtime-wry 2.11.4 / tauri-utils 2.9.3 / tauri-codegen 2.6.3`
  在 Cargo.lock 固定。原因：镜像上这些内部 crate 的最新小版本（2.7/2.12/2.10）与 2.11.6 存在
  破坏性 API 差异（`do_menu_item!` 引用不存在的 `Error::UnexpectedMenuKind`、
  `Monitor: From<...>` 缺失），上游以锁文件约定组合。**未使用 Cargo patch**。

### 协议（designer 前端对接）

- `invoke probe_status` → `{ backend, adapter, width, height, frames, stopped, error? }`
  - `backend`/`adapter` 初始化前为 `null`；`error` 在异常时为字符串，正常时为 `null`。
- `invoke probe_input`（`ProbeInput`）→ `{ accepted, sequence, marker_offset, terminal, reason? }`
  - 字段：`sequence`、`kind`，可选 `x/y/dx/dy/width/height/dpr`。
  - `kind ∈ pointer_down|pointer_move|pointer_up|pointer_cancel|wheel|resize|blur`。
  - **输入仅移动探针 marker offset 并返回状态/ack**；**不声称 CAD 已实现**，不改变任何
    CAD/数据库状态。
  - 拒绝显式：`accepted=false` 且带稳定 `reason`
    （`out_of_order_sequence`、`non_finite_parameter`）；**不存在空成功**。

### 生命周期与性能界限

- 驱动：单 Tauri 事件循环，`RunEvent::MainEventsCleared` 在**主线程** tick；不另建事件循环。
- resize：surface 跟随窗口物理尺寸；窗口尺寸为 0（最小化/未映射）→ **暂停**，不 configure 到 0。
- minimize/restore：每 tick 读窗口状态，恢复非零尺寸后继续。
- destroy：窗口 `Destroyed` → `request_stop`；tick **先 `stop()` 再释放** presenter。
- GPU 失败：写入 `ProbeStatus.error`，不伪造成功帧。
- 性能界限：tick 以 `FRAME_INTERVAL=16ms`（≤~60fps）节流；输入在命令内同步处理、**不入队**，
  无输入堆积；动画为固定有限步进。

## 影响 / 构建

- `tauri.conf.json`：`build.frontendDist = ui/dist`（designer 产物），并设
  `build.devUrl = http://localhost:1420`。**debug 构建不需要 `ui/dist` 存在**（`generate_context!`
  的 dev 分支在设置了 devUrl 时使用空 assets），因此 host 现在即可编译；release /
  `custom-protocol` 构建才嵌入 `ui/dist`。**未写 `ui/` 目录**。
- 运行入口 `apps/app-react-linux/scripts/run-probe.sh`（强制 `GDK_BACKEND=x11`）；需要
  designer 的 Vite dev server（1420）或已构建的 `ui/dist`。
- `bundle.active=false`；提供占位 `icons/icon.png`（32x32 RGBA，仅满足 tauri-codegen 默认窗口
  图标读取，待设计替换）。
- 顶层 XID：启动时打印 `[probe] top-level X11 XID ...`，即交给 `SurfaceProbePresenter` 的窗口。

## 未决（NOT RUN）

- **真实窗口 60s 探针（XWayland + P620）未运行**：需 designer 完成 `ui/dist` 后由主协调者统一
  验证。本 ADR **不**据编译/单测通过宣称渲染、合成或视觉验收。
- **GTK child 桥方案未实现**：本轮只做顶层窗口 XID 对照；child drawing area 的 XID 复用方案由
  librarian 核实后补充，尚未编码。
- **GTK/WebKit 叠加与 Vulkan surface 的平台合成未验证**（透明窗口 + 透明子 Webview 的实际叠加
  表现、遮挡、输入焦点均未测）。
- minimize/restore、drag 的真实交互未测（仅有代码路径与单元契约）。
- 真实 FPS/GPU 计时未采集（无 readback、无帧计时采集）。
- 未跑 WASM（本实验明确不允许）。
- 探针**不是** CAD presenter：不绘制 CAD 几何，不代表 CAD 能力。

## 回退

- 若 GTK/WebKit 与 Vulkan surface 的平台合成不可行：可保留顶层 surface 探针作为最小真实 GPU
  证据，或回退到 Slint 共享 device 合成（ADR 0002）。**不删除**旧 Slint 代码。

## 真实窗口验收证据（2026-10-14，Gate 1 **未通过**）

证据目录：`/tmp/opencode/yacr-react-probe-20261014T044550Z/`（`final60.log`、`final60-*.txt`
xwininfo、`run2-60s.log`、`diag-*.log`、`bridge*.log`、`final60-grab.png`）。

已确认（真机 XWayland + Quadro P620）：

- **窗口与适配器**：顶层 XID `0x1800003`（`xwininfo`：`IsViewable`、`WM_STATE Normal`、
  960x600，父为 `mutter-x11-frames`）；`presenter ready backend=vulkan adapter=Quadro P620
  device_type=discrete_gpu`。
- **真实 IPC 输入可用**：前端挂载即发 `resize seq=1 accepted=true`；`eval` 合成注入
  `pointer_down/move/up`（seq 2/3/4，marker 0.062→0.354→0.396，up `terminal=true`）与
  `wheel`（seq 5，marker 0.196）全部 `accepted=true`，无空成功。**注入为合成事件，非真实鼠标**。
- **redraw pump 生效**：tick 约 30–62/s（不再只随事件触发）。

**未通过项（阻塞 Gate 1）**：

- **刷新率 ~1 fps**：每帧 `slow frame: ~998 ms`（`presenter.frame()` 阻塞约 1 秒）。
  隔离证据：渲染器 ignored 用例在**普通 Xlib 窗口**上 init+4 帧+resize+rebuild 仅 0.37s
  （快），而 GTK 顶层窗口与 GTK `GtkDrawingArea` 子窗口均 ~1s/帧 → 停顿在
  **XWayland 合成 present 路径**，与 GTK 归属无关。
- **GTK 桥 B（一次有界修复）已实现并测试，未修复**：`gtk_bridge.rs`（`GtkOverlay` + windowed
  `GtkDrawingArea` + webview reparent；自带 Xlib 连接 + 生命周期 owner），修复 reparent 后
  surface 有效（`error=None`、帧递增）但仍 ~1s/帧。按“1 次修复仍失败即停止”约束**停止**；
  该桥默认**关闭**（`YACR_PROBE_GTK_BRIDGE=1` 才启用），仅作为负结果留档。
- **resize 未生效**：`set_size(1200x800)` 返回 `Ok` 但窗口保持 960x600（t=8/29/39/47 的
  `xwininfo` 均为 960x600）；`set_size(800x500)` 同样无效。
- **minimize 可暂停，restore 未恢复**：minimize 后帧停在 33 且 `paused (minimized=true)`；
  `unminimize()` 后 `is_minimized` 仍为 true，未 `resumed` → 恢复路径未验证。
- **截图不可用（工具缺口，当轮）**：rootless XWayland 的 X11 root grab 近乎全黑
  （`final60-grab.png`：142/2,073,600 非黑像素，仅鼠标）；GNOME Shell D-Bus 截图
  `AccessDenied`；无 `grim/scrot/import/gnome-screenshot/xwd`；portal 需交互授权。
  当轮**无像素/视觉证据**。**后续 composited 轮已取得 full compositor 像素证据，结论更新见
  「Compositor 截图取证」节（composite FAIL）**；此处保留当轮历史，不抹掉。

NOT RUN（本轮仍不声称）：像素/视觉验收、真实鼠标输入、GTK/WebKit 叠加合成正确性、
60fps 同步刷新、真实 FPS 计时曲线。诊断用环境开关（`YACR_PROBE_EVIDENCE`、
`YACR_PROBE_GTK_BRIDGE`、`YACR_PROBE_SKIP_WEBVIEW`、`YACR_PROBE_OPAQUE`）默认关闭，不影响正常运行。

## 有界 debug + 安全修正（2026-10-14 第二轮）

**Root cause（stage 计时实测，非推断）**：`SurfaceProbePresenter::frame` 每阶段计时显示，
**`Queue::present`（Fifo）阻塞约 1.000 s**，其余阶段均快：

- 普通 Xlib 窗口连续 10 s：`present` median **1,000,556 µs** → 10 s 仅 **14 帧**；
  同一窗口改用 `Immediate`（该 surface 只支持 `[Fifo, Immediate]`，无 `Mailbox`）：
  `present` median **77 µs** → 10 s **62,404 帧**。其余：write ~7µs、acquire ~4µs、
  submit ~45µs、scope_pop ~0µs。**1 fps 完全由 Fifo present 造成**，与 GTK 归属、
  error scope、acquire 均无关。

**修正**：

- **present 模式**：新增 `ProbePresentMode::{Fifo, MailboxElseImmediate}`；宿主使用
  `MailboxElseImmediate`（实际 `supports` 无 Mailbox → 选 `Immediate`），**显式日志**
  `requested/supported/chosen`，无静默 fallback。代价：`Immediate` 无 vsync，可能撕裂。
- **stage 计时**：`ProbeFrameStats::timings`（write_buffer/acquire/configure/submit/
  scope_pop/present/total，µs）+ configure 计时；宿主 1 Hz 状态行带 frame 索引与各阶段。
- **scope.pop 有界非阻塞**：`collect_error_scope_bounded`（100 ms 预算，小步 `device.poll`），
  validation 仍上报，超时则记日志继续，不再无限阻塞。
- **线程/所有权安全**：presenter 改为 **main-thread `thread_local`**（不在
  `Arc<Mutex<Shared>>` 内，命令线程无法跨线程 drop GPU）；共享状态**不含 GPU**，
  shared lock 不跨 GPU 工作持有。
- **surface-before-window**：`CloseRequested` 与 `ExitRequested`/`Exit` **显式
  `release_presenter()`**（stop + drop），不再只依赖 `Destroyed`（其到达时窗口已销毁且无后续 tick）。
- **redraw single-pending**：pump 仅在无未决 wake 时唤醒（tick 清零），不无限积压。
- **child size**：用窗口实际 inner size，而非初始常量。
- **撤回原型**：GTK bridge B（`gtk_bridge.rs` + `unsafe Send/Sync` provider + gtk/gdk/gdkx11
  依赖）**已删除**，记为撤回的负结果原型（其 ~1s/帧同样来自 Fifo present，非 GTK）。

**本轮真实证据**（`/tmp/opencode/yacr-react-probe-20261014T044550Z/`）：

- `xlib-continuous.log` / `xlib-continuous-immediate.log`：上面 Fifo vs Immediate 对照。
- `runA.log`：Tauri 全 React 连续 11 s，`present_mode=Immediate`，**~60 fps（651 帧）**，
  stages `present≈136–306µs`、`total≈325–741µs`；真实 IPC 输入 `resize seq=1` +
  合成 `pointer_down/move/up`+`wheel` 全部 accepted。
- `runB.log` / `runC-nogpu.log`：**resize（`set_size`）无效**（`xwininfo` 恒 960x600，WM 级，
  与 GPU 无关）；**minimize 可暂停**（`paused (minimized=true)`），**`unminimize` 未恢复**
  （无 `resumed`）。
- **窗口 drawable 抓图成功**（`scripts/x11-window-grab.c`，方法：`XGetImage(window)`，
  非 root）：`runE-grab1/2.png` 960x600 全非黑，颜色与探针 shader 一致（网格灰 149,160,179；
  橙 marker 255,196,89；红/绿角标）；两次抓图 marker 橙块质心 x **278→312**，证明确实在动。
  注：X11 **root** 抓图仍黑（rootless XWayland），但 **window drawable 可抓**。

NOT RUN / 未决：真实鼠标输入；GTK/WebKit 叠加合成正确性；`Immediate` 的撕裂/视觉质量；
resize/minimize-restore 仍依赖 WM（`set_size` 无效、`unminimize` 不恢复）→ **Gate 1 仍未通过**
（刷新率已修复且窗口 drawable 可视，但生命周期与合成正确性未闭环）。仍不接 CAD 核心。

## WASM / 跨目标隔离构建修复（2026-10-14）

**回归**：`cargo check --workspace --lib --target wasm32-unknown-unknown --locked` 退出 101——
`app-react-linux` 无条件依赖 `tauri`/`wry`/`tao`，为 wasm 目标编译 `tao`/`wry` 失败
（`tao` unsupported、`wry` 缺 `InnerWebView`）。

**修复（机械、bounded）**：

- `apps/app-react-linux/Cargo.toml`：`tauri`/`cad-render-wgpu`/`raw-window-handle`/`serde` 移到
  `[target.'cfg(target_os = "linux")'.dependencies]`；`tauri-build` 移到
  `[target.'cfg(target_os = "linux")'.build-dependencies]`。
- `build.rs`：build 脚本按 **host** 编译，故 `tauri_build` 引用按 host cfg 包裹；
  再以 `CARGO_CFG_TARGET_OS` 判断**目标**，仅 Linux 目标调用 `tauri_build::build()`。
  非 Linux host 下 build-dep 不存在、该分支为空操作，脚本仍可编译。
  （证据：Linux host + wasm target 时 `tauri-build` 仍被编译 → build-dep 按 **host** 选择；
  真正阻止 wasm 生成 Tauri context 的是运行时 TARGET 判断。）
- `src/lib.rs`：全部模块/reexport `#[cfg(target_os = "linux")]`；其它目标为空 crate，
  **不 pretend 可运行**。
- `src/main.rs`：Linux 入口；其它目标 `#[cfg(not(target_os = "linux"))] fn main` 打印
  “Linux/X11 experiment; not supported” 并 `exit(1)`。
- 版本固定：`tauri = "=2.11.6"`、`tauri-build = "=2.6.3"`（与 lock 一致，避免漂移）。
- 未改 `ui/**`、未改 native presenter、未改运行窗口行为（explorer 正在查 resize 控制）。

**构建证据**（`/tmp/opencode/`）：

- `cargo check --workspace --lib --target wasm32-unknown-unknown --locked` → **exit 0**
  （`yacr-react-wasm-check.log`）。
- `cargo check --workspace --all-targets --locked` → exit 0；`cargo check -p app-react-linux
  --all-targets --locked` → exit 0。
- `cargo test -p app-react-linux --lib` → 10 passed；`cargo test -p cad-render-wgpu --lib`
  → 59 passed / 2 ignored。
- `cargo clippy --workspace --exclude app-android --exclude app-web --all-targets --locked
  -- -D warnings` → exit 0。
- `cargo fmt --all -- --check` → 仅 `crates/cad-import-acadrust/examples/debug_fixture.rs`
  （既有、陌生 debug 文件，**未编辑**）格式失败；归因见 `yacr-react-fmt-check.log`。
- 因 Linux 上行为未变（无窗口/渲染改动），上一轮运行时证据仍**有限有效**。

## Compositor 截图取证（2026-10-14 第三轮，**composite FAIL**）

**本轮取得 full compositor 像素证据（此前"无像素"结论已更新）**，且合成仍失败。

- **方法/工具（自动，无需人工点击）**：`xdg-desktop-portal` `org.freedesktop.portal.Screenshot`
  v2，`interactive=false`/`modal=false` + 确定性 `handle_token`；Python gi/Gio **先订阅**
  `org.freedesktop.portal.Request.Response` 再 invoke（避免竞态），20s 超时。
  6 张 1920x1080 全桌面 PNG：`/tmp/opencode/yacr-react-composited-20261014T082703Z/shot1..6.png`。
- **授权（用户明确授权，均已恢复）**：临时 `loginctl unlock-session 1`（仅发 Unlock 信号，
  **未读取密码**）；临时 `PermissionStore.SetPermission screenshot true screenshot '' ['yes']`
  自动批准 portal。**finally 恢复**：原 `''` 不存在 → `DeletePermission screenshot/screenshot ''`；
  原 `wechat: ['yes']` 授权**保留未动**（verify：`Lookup` 回到 `{'wechat': ['yes']}`）；
  `loginctl lock-session 1` 并 verify `GetActive=true`/`LockedHint=yes`。
- **清理/隐私**：仅关闭本轮启动的 probe 进程（进程组），`ps` 确认无残留；截图为**私有桌面**，
  **仅本地保存，未上传外网**。未改 unsafe-mode/gsettings/图形 config；未编辑项目源码。
- **observer 结论（真实 GNOME 截图 shot2..5.png）**：目标 960x600 **客户区只有网格、
  orange marker 与 corner markers**；顶部白色 “yacr react probe” 条是 **WM decoration**，
  **不是 React ribbon**；**React ribbon / 命令框 / DOM 浮层均不可见**，且**未被外窗遮挡**；
  marker 逐帧向左移动。→ **Gate composite FAIL**：**无法区分"被覆盖"与"未加载"**；
  IPC 只能证明逻辑通路，**不能证明可见**。
- **不改动既有结论**：native Wayland、真实 DWG、真实鼠标、正式 CAD 仍 **NOT RUN**；
  旧每轮 history（含各轮 NOT RUN）保留，不抹掉。**不继续代码实现**；
  **不据此宣称 Tauri 整体不可行**（仅本轮 composite 未通过）。

## React 原生桌面宿主跨平台路线契约（2026-10-14 增补）

**范围与状态**：用户批准以 **React 替代 Slint** 作为桌面 UI 目标、**Tauri** 作为桌面壳、
**native Rust CAD + renderer-only wgpu**；本轮仅在 **Linux X11/XWayland** 上获批并实测，
**Win32 与 macOS 为未来实现参照，NOT IMPLEMENTED / NOT RUN**。本节是给两端未来实现的契约参照，
**不得**作为任一端已实现或已验收的证据。规范侧条款见 `CAD_IMPLEMENTATION_SPEC.md` §21。

### 用户已确认目标（本轮授权）

- 桌面 UI 由 **React DOM** 承担以替代 Slint；**Tauri** 提供桌面窗口/WebView 壳。
- CAD 核心保持 **native Rust**（既有 `cad-*` / `cad-engine` / `cad-host`），不改写为 JS。
- **renderer-only wgpu**：wgpu 仍仅由 `cad-render-wgpu` 依赖并经其 API/re-export 提供；平台层不直接依赖 wgpu。
- **不允许 WASM**（限本桌面 React 路线；Web 应用仍按规范 §2.1.5/§6 的 WebGL2/WebGPU+Wasm 要求，不受本节影响）。
- **不允许逐帧整幅 GPU→CPU 回读**作为 CAD 合成路径。

### 共同架构（跨平台目标形态）

```
React DOM UI + input
  → typed bounded IPC
  → HostCore EngineClient
  → RenderSession (cad-engine)
  → renderer（cad-render-wgpu）单一 CAD Surface
底图 texture 留 GPU；UI 合成走平台桥，不回读整幅帧。
```

- UI 仅发有限命令/输入；CAD 状态、事务与呈现仍由 Rust 侧权威持有。
- 单 CAD presenter：**每个 native drawable 只允许一个 CAD presenter**，不与 UI 争抢同一 drawable；
  平台合成桥**不是** shared Device WebView。

### 平台差异（候选，均未验）

| 平台 | UI 壳 | CAD 呈现候选 | 合成方式 | 状态 |
|---|---|---|---|---|
| Linux（本轮） | Tauri/GTK3 + WebKit | **GTK 中央 native 分区（nativechild）** | React DOM 与中央 native CAD **co-visible**（分区，非透明 alpha 全屏） | X11/XWayland；**实际 CAD 有界验收 accepted**（限见末节）；顶层透明 alpha full overlay 为历史 FAIL |
| Windows（未来） | Tauri + **transparent WebView2** | **native Vulkan** 候选 | 主窗口透明 webview 叠层 | **NOT IMPLEMENTED / NOT RUN** |
| macOS（未来） | Tauri + **transparent WKWebView** | **native Metal**（wgpu backend） | 主窗口透明 webview 叠层 | **NOT IMPLEMENTED / NOT RUN** |

- macOS 后端是 **Metal**（经 wgpu backend）；**不得**错误称为“macOS Vulkan”。
- **不得**把“各平台同一种原生叠层结构”承诺为已验证：三者合成路径不同，Linux 已实测失败，Win/macOS 完全未实现。

### RapidRAW 参照（用户提供定位，本仓未独立核验/复现）

- 固定 commit：`74597d066685283588502edbecd747bfe80404cf`。
- Win/macOS：CAD surface 挂在主窗口上，**透明 webview** 叠层（与本节 Win/macOS 候选形态一致）。
- Linux/Android：`use_wgpu_renderer=false`、`surface=None`、**readback JPEG + binary IPC**；**它没有 GTK child 路径**。
- 相关文件定位：`gpu_processing.rs`、`lib.rs`、`tauri.conf.json`。
- **关键差异**：RapidRAW 的 Linux/Android readback 路径**不是**本项目采用的方案——本项目契约
  **禁止 readback 兜底**；Linux 必须走**独立 GTK nativechild 桥**（未验），不是 readback。
  参照仅用于 Win/macOS 透明 webview 叠层形态，不构成本项目实现或验收。

### 合成与坐标契约

- 统一坐标：**CSS / canvas-local / world / physical** 四类，配合 **DPR** 与 **metrics revision**；
  坐标与度量变化统一更新映射。
- 覆盖关系：DOM bounds、scissor clipping、ribbon、popup 由 UI 层负责；**React 负责输入与
  非透明 hit-through**（命中路由不得被上层透明/不透明层吞掉）。
- **平台合成桥不是 shared Device WebView**；不主张 GTK/WebView 与 wgpu 同 device/queue。

### 输入与生命周期契约

- 输入带 **sequence**，有界 **latest-wins**：仅连续同 context 的 `move` 可合并；`down/up/cancel/
  blur/wheel` 为**顺序屏障**。拒绝显式（稳定 reason），**无空成功**。
- **旧 doc/generation 不覆盖**新状态；过期输入/结果丢弃。
- **dirty wake 有界**：不无条件全帧重新解析底图；沿用 ChangeSet → 依赖失效 → 显示表示重建。
- GPU 生命周期：attach / detach / resize / minimize-restore / **lost** 全部显式处理；失败上报，不伪造成功帧。
- **每个 native handle 销毁前，CAD surface 必须完全 drop**；owner 在**主线程**，**不得 `unsafe` Send GTK 对象跨线程**。
- fonts / resources / failed-open **保守处理**：缺失即诊断，**不空成功**。
- 双入口（有限命令与 UI 按钮）走**同一命令路径**；文案来自现有 `cad-ui-slint/i18n/*.json` 键集，
  不在 UI 硬编码中文。

### 明确不承诺

- **不承诺跨语言稳定 IPC**：仅为 **bounded 实验 adapter**，typed protocol 不等于稳定序列化契约。
- 不扩张权限、不引入新系统权限/安全面。
- 不把 synthetic 测试或单平台结果**跨平台外推**。

### 验收要求（每平台独立）

每平台须有 **真实窗口 + 硬件 backend + full compositor 截图 + 输入/resize/DPR/恢复/shutdown**
的真实运行证据；synthetic/单测**不能**替代或外推。未运行须显式 NOT RUN。

### 未来 implementation checklist（Win/macOS 参照）

1. [ ] 选定 UI 壳（Tauri + WebView2 / WKWebView）并完成 native Rust CAD 与 `cad-engine`/`cad-host` 接线。
2. [ ] 建立 **typed bounded IPC**（有限命令/输入，非稳定跨语言协议）；错误显式。
3. [ ] 实现**每 drawable 单 CAD presenter**；不与 UI 争抢 drawable；确认合成桥非 shared Device。
4. [ ] 平台后端：Windows **native Vulkan** 候选 / macOS **native Metal**（wgpu backend）；透明 webview 叠层。
5. [ ] 统一坐标 + DPR + metrics revision；DOM bounds/scissor/ribbon/popup 覆盖；React 输入 hit-through。
6. [ ] 输入 sequence + latest-wins + 屏障；旧 doc/generation 不覆盖。
7. [ ] dirty wake 有界；GPU attach/detach/resize/minimize-restore/lost 处理；handle 销毁前 surface drop；主线程 owner，无 unsafe Send。
8. [ ] fonts/resources/failed-open 保守；双入口同命令路径；i18n 用现有 JSON。
9. [ ] 真实窗口 + 硬件 backend + full compositor 截图 + 输入/resize/DPR/恢复/shutdown 证据；否则标 NOT RUN。

### 支持状态矩阵

| 能力 | Linux（本轮） | Windows | macOS |
|---|---|---|---|
| React/Tauri 壳 | 已实现（bounded prototype）；窗口实测 | NOT IMPLEMENTED | NOT IMPLEMENTED |
| native Rust CAD 接线 | 已接（bounded）：synthetic four-line `entity=4`；flange DXF `419` | NOT IMPLEMENTED | NOT IMPLEMENTED |
| 硬件 backend | Vulkan（P620）实测，无软件后端 | 候选 native Vulkan，NOT RUN | 候选 Metal，NOT RUN |
| React 覆盖合成 | 中央 native 分区 **co-visible accepted（bounded）**；透明 alpha full overlay 历史 FAIL | NOT RUN | NOT RUN |
| 独立 nativechild 桥 | **已实现（GTK 中央分区）** | N/A（WebView2 叠层） | N/A（WKWebView 叠层） |
| app 是否用 Slint presenter | **否**（React 宿主无 Slint presenter）；旧 Slint 代码**未删除** | N/A | N/A |
| 输入/resize/DPR/恢复/shutdown | pan/wheel/Fit/typed PASS；resize remap **Drop-before-native** + close/Exit PASS；minimize→restore 仅 **WM 激活** PASS（Tauri `unminimize()` API 限制） | NOT RUN | NOT RUN |
| full compositor 像素 | co-visible 截图取得；flange 视觉 **PARTIAL** | NOT RUN | NOT RUN |

## 第四轮：GTK 中央 native 分区 + 实际 CAD 有界验收（2026-10-14，latest accepted limit）

**目标与方案变化**：用户目标为 **React 替代 Slint + Tauri 桌面壳 + native Vulkan**；基本 UI 已
implemented 并 validated 为 **bounded prototype**。第三轮「顶层透明 alpha 全屏叠加」composite
FAIL 的历史**保留**（见「Compositor 截图取证」节，不抹掉）；本轮改为 **Linux GTK 中央 native 分区
（nativechild）在 React UI 上**：CAD native surface 作为 **GTK 中央分区**与 React DOM（ribbon、
底部命令条）**同时可见（co-visible）**，**不是** transparent alpha full overlay。

**证据目录**：

- 实际 CAD 验收 `/tmp/opencode/yacr-react-cad-acceptance-20261014T112301Z`；
  输入/键盘终轮 `/tmp/opencode/yacr-react-cad-input-final-20261014T115552Z`。
- 生命周期 `/tmp/opencode/yacr-react-lifecycle-20261014T101035Z`；
  WM restore `/tmp/opencode/yacr-react-restore-20261014T102828Z`。

**已验收（bounded prototype，含限制）**：

- synthetic four-line DWG 打开：`entityCount=4`、`backend=vulkan`、`adapter=Quadro P620`。
- React ribbon + 底部命令条与**中央 native CAD co-visible**；OS pan / wheel（上、下）/ Fit 按钮与
  typed 命令均生效。
- 未知命令 `BOGUS` 走既有 **not supported** 路径（不空成功）。
- 实际 React **OPEN** 走 native portal picker → licensed flange DXF：`entityCount=419`、
  **同进程** `importCount=2`（同一进程内第二次导入）。
- 非法坏 DWG：显式 `CorruptData("input is neither a DWG signature nor an ASCII/binary DXF
  section stream")`，**保留旧 DB**（419 不变）。
- Cancel：保留 DB，报清晰错误。
- resize：unrealize/remap → detach/attach，**Drop 先于 native**；正常 close/Exit 同样先 Drop。
- minimize→restore：**WM pager 激活路径 PASS**；Tauri `unminimize()` API 请求**未被 mutter 采纳**
  → 记为**限制**（仅真实 WM 激活可恢复）。
- GPU：真实 **P620 / Vulkan / Immediate**，**无软件后端**。
- 错误策略（已 hardening）：Host bind/present/Tick 错误投影到前端；**bind failure 对同一 presenter
  generation 最多重试一次**，错误保持可见，**新 generation 可重试**；**closing/failed 禁止 bind**。

**有界 / 不夸大**：

- 60fps 仅为上一轮 **PROBE** 的 10s 实测；**实际 CAD root 不宣称 60fps**（无稳定 CAD bench）。
  renderer 有 GPU frame present 统计，但**不外推**。
- `importCount` 是**成功文档观察**计数，**不是 parser 调用数**；**无运行时重解析的直接插桩
  NOT RUN**，仅有架构导航路径 + 600 synthetic 契约的 counter 不变 proxy。
- observer 视觉：four-line **basic smoke PASS** + pan/fit 位置；flange **PARTIAL**——主几何可见、
  无明显翻转，但布局与参考差异（缺 paper frame / title block 右侧细节、大量文字），
  **不能声称 full visual golden / vendor DWG**；当前显示是否为模型空间**原因 UNDETERMINED**。
- popup 跨 CAD 覆盖 **Unsupported**；诊断在 ribbon 内；**无 Android/Web 迁移**；
  **显式不是完整 CAD 产品**；**不从 HostCore 资源完整兼容外推**。

**测试 / 门禁（per logs reported）**：

- app **71**（契约合成：含 2 条新增 bounded-bind contract）、UI 16、renderer 64、WGSL 4；
  `check` / `clippy` / `fmt` 均 PASS。**71 为契约合成测试计数，与更早的 GPU runtime 证据分开记录，
  不互相替代**。
- 门禁日志 `/tmp/opencode/yacr-react-final-gates-20261014T112155Z`：host check / wasm check / clippy
  + 4 static（architecture/fixture/workflow/i18n）**均 0**；`fmt` 仅既有陌生未跟踪
  `debug_fixture.rs` 失败（**未编辑**）。

**清理 / 隐私 / 锁状态**：截图临时权限已恢复 baseline；**保留 unknown/desktop 原锁状态**——
最近一次启动时 session 为 unlocked，故**保持 unlocked**，**不得声称 always locked**。
全屏截图**仅本地保存，未上传**。既有外部 `debug_fixture.rs` / `debug-block.dwg`
**未索引 / 未使用**。

**Win/macOS**：仍 **NOT IMPLEMENTED / NOT RUN**；macOS 候选为 **Metal**（非 Vulkan）；
GPU **无 CPU readback**；React 宿主**无 Slint presenter**，**旧 Slint 代码未删除**。

## 第五轮：窄 2D 导航 fastpath + 实测性能基线（2026-10-14，latest accepted）

**核心（窄 2D 导航合同）**：导航复用**同一权威底图**（GPU-resident、**同一 DB revision**，
fonts/images/layers/space 一致），**Ready fastpath 取最新 camera**，**不展示旧 doc**；拖拽期间
overlay 后缀**临时省略**并以显式 `FrameCompleteness::BaseOnlyPendingOverlays` 标记（**非静默**）。
`PresentedFrame` 元数据带 `view_revision`（= 实际 camera）；`CurrentScene` 表示 **GPU 已提交的当前
场景**，**不是**渐进式全像素；当前 drawing 严格标记 `presented=false` + `baseOnly`。严格模式：
CPU ready 但 **GPU 未提交**返回 `None`、**不返回旧底图**；restore 后同一底图 overlay 提交（静态
base prefix 保留）；**device change 阻塞直到新上传**；**3D 严格路径不变**。

**Root lag 处理**：严格 view prep token advancing，允许**多数帧 `None`**，而实际输入处理 ~23µs、
render ≤3ms。

**用户可见取舍**：拖拽期间 transient grid/axes/highlights/snaps/previews 可能**暂时隐藏**，直至
prepared 恢复（<30ms 窗口测量，**非精确延迟**声明）。

**性能证据（source hash locked）**：基线 `/tmp/opencode/yacr-react-baseline-perf-20261014T125447Z`
与 postfix `/tmp/opencode/yacr-react-postfix-perf-20261014T135021Z`：同 4s、494 events、~120Hz，
fourline/flange drag waiting **65.3%/67.8% → 0.0%/0.0%**、presented/s **21.0/19.5 → 60.7/59.9**；
`view_current == view_rendered` **709/701 帧 0 mismatch**；post-release waiting **0**、无 history、
最终位置稳定（148/146 帧）；`input_age_us`（Rust admission→drain）p95 **15.722/15.022ms**（**非
input→visible**）；scope pop p50/max **1/3µs**；frame p95 **0.873/3.132ms**；UI FPS **60/61**；
observer max black/gray/align **PASS**。**input→visible p95 < 50ms UNVERIFIED**（portal 静态截图，
非真实视频捕获）。**不宣称无性能回归 / full visual golden**（vendor 仍 `Partial`）。

**测试 / 门禁**：engine **76** pure/synthetic + **5** ignored（real P620，显式运行全部 **PASS**）；
App **73** contracts（latest host telemetry）、renderer **65**（lib）、UI **22**；WGSL shaders
unchanged（新代码 Vulkan，无 CPU）。门禁 `/tmp/opencode/yacr-react-drag-gates-20261014T135430Z`
host/wasm/clippy + 4 static / diff **均 0**；`fmt` 仅既有陌生未跟踪 `debug_fixture.rs` 失败（未编辑）。
**Oracle 独立 accepted** post-GPU publication gate。

**不夸大**：**device fastpath 测试不等于平台/全产品验收**；不承诺未支持平台（Win/macOS 仍 future）
字段；跨平台沿用同一契约，仍引用 RapidRAW 固定版本，**Linux 无 readback**。清理：会话本就
unlocked → **保持 unlocked**（保留原状态）；portal 权限恢复（保留 `wechat`）、虚拟 uinput 与自身
PID 已清理；全屏截图**仅本地私有，未远程上传**。**旧各轮 FAIL history 与标签全部保留**，本段为
最新 summary。
