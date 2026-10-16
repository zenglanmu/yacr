# React 原生 Linux 宿主（当前桌面宿主）

状态：**当前桌面 UI 实现**（2026-10-16）。取代 Slint 桌面路线，见
[`docs/adr/0010-react-linux-host-replaces-slint.md`](adr/0010-react-linux-host-replaces-slint.md)；
原生 Wayland 后端见
[`docs/adr/0011-native-wayland-subsurface.md`](adr/0011-native-wayland-subsurface.md)。
与 `CAD_IMPLEMENTATION_SPEC.md` §21。**仍是 bounded 实现，不是完整 CAD 产品**，也不代表跨平台验收。
后端支持 **X11/XWayland** 与**原生 Wayland** 两者（原生 Wayland 的有界 `--probe` 已跑通，CAD/视觉
仍待验证，见 `docs/validation-native-wayland-2026-10-16.md`）。

- 应用：`apps/app-linux`；二进制 `yacr`（Tauri 标识 `dev.yacr.app`，产品名 `Yacr`）。
- 前端：`apps/app-linux/ui`（Vite + React 19）。
- 平台无关 UI 逻辑：`crates/cad-ui-core`（protocol/layout/session/shell/i18n）。
- 入口脚本：`scripts/run-react-linux.sh`（在仓库任意目录调用；用 git 定位根、路径加引号、
  `set -euo pipefail`）。

## 构建依赖（系统包需预先安装，脚本不自动安装）

- **WebKitGTK 4.1 开发包**（`libwebkit2gtk-4.1-dev`）与 **GTK3 开发包**（`libgtk-3-dev`）；
  `pkg-config --exists webkit2gtk-4.1 gtk+-3.0` 应通过。
- **Node + npm**（用于 Vite 构建 `apps/app-linux/ui`）。
- Rust 工具链按仓库 `rust-toolchain.toml`。
- 默认 `debug` 构建即可快速运行；`scripts/run-react-linux.sh` 默认 `--release`（可选，较慢）。
- **不**自动安装系统依赖；缺失时脚本报错退出。

## 启动

```bash
bash scripts/run-react-linux.sh --open "$(pwd)/fixtures/dwg/synthetic-four-lines.dwg" --fonts-dir "$(pwd)/fonts"
```

脚本**不强制任何后端**：它在 `GDK_BACKEND` 未设时只做粗粒度会话准入（原生 Wayland 需
`WAYLAND_DISPLAY` **且** `XDG_RUNTIME_DIR`；X11/XWayland 需 `DISPLAY`；两者皆无则快速失败，退出 2），
随后由宿主 `apps/app-linux/src/backend.rs` 在 GTK 启动前做出**权威**的后端决议（见下节）。
`node_modules` 不存在时先 `npm ci`，再 `npm run build`；最后以 `custom-protocol` 运行宿主
（嵌入 `ui/dist`）。脚本**不解锁会话、不截图、不安装系统包**。

先构建前端（`ui/dist` 会被嵌入），再按需显式选择后端运行。**原生 Wayland**（真实原生，非 XWayland）：

```bash
(cd apps/app-linux/ui && npm ci && npm run build)
# 精确复测既有二进制（当前为 sha256 7618d0d2…）：直接跑 target/release/yacr
env -u DISPLAY -u XAUTHORITY \
    GDK_BACKEND=wayland WAYLAND_DISPLAY=wayland-0 XDG_RUNTIME_DIR=/run/user/1000 \
    ./target/release/yacr --open /abs/drawing.dwg
# 或先构建：cargo build --release --locked -p app-linux --bin yacr --features tauri/custom-protocol
```

**显式 X11/XWayland** 替代（保留的既有路径）：

```bash
GDK_BACKEND=x11 DISPLAY=:0 ./target/release/yacr --open /abs/drawing.dwg
```

（`scripts/run-react-linux.sh` 与之等价，但内部是 `cargo run --release --locked …`，**必要时会重新构建**
二进制并处理 `node_modules`/前端构建；若要复测**当前** `target/release/yacr` 而不触发重建，请用上面的
直接命令。手动本地验收由用户自行进行。）

参数（透传给宿主）：`--open <path>`、`--fonts-dir <path>`、`--images-dir <path>`、`--probe`。
环境变量（证据/诊断）：`YACR_TEST_DWG`、`YACR_LOCALE`、`YACR_PROBE_EVIDENCE`、
`YACR_PROBE_LIFECYCLE`、`YACR_PROBE_ONLY`、`YACR_PROBE_NO_GPU`、`YACR_PROBE_OPAQUE`、
`YACR_PROBE_NO_MAXIMIZE`、`YACR_PROBE_CAD_LOG`。

**窗口默认（最大化，非全屏）**：Linux 宿主默认以**最大化窗口**启动，该策略是窗口管理器**最大化**，
**保留标题栏/系统面板**，**不是**全屏/无边框 kiosk。源码依据：`apps/app-linux/src/host.rs` 的
`start_maximized(..)` 默认返回 `true`（`!no_maximize_env_present`），`WindowBuilder::maximized(maximize)`
在 `init` 中生效；`tauri.conf.json`（`"windows": []`）没有可覆盖它的窗口配置。**仅**验收/证据运行用
`YACR_PROBE_NO_MAXIMIZE`（**其存在**即退出最大化，取值无关）恢复普通窗口。注意：该行为由**当前工作树**
（React/Tauri 宿主重写，未提交）提供；未在本轮运行验证，且**跨窗口管理器/平台不保证**（NOT VERIFIED）。

**配置目录迁移（有意）**：由旧的 `~/.config/yacr-react/` 改为 `~/.config/yacr/`；**不做自动迁移**，
用户需自行迁移（或接受默认值）。

## 后端策略与原生 Wayland 子表面

`GDK_BACKEND` 由 `apps/app-linux/src/backend.rs` 在 GTK 启动前显式决议（**无静默回退**）：

- **未设（`Auto`）**：`WAYLAND_DISPLAY` + `XDG_RUNTIME_DIR` 同时存在 → **原生 Wayland（优先）**；
  否则仅有 `DISPLAY` → X11/XWayland；两者皆无 → 明确报错（退出 2，**没有无头成功**）。
- **`GDK_BACKEND=x11` / `wayland`**：显式接受并**原样保留**（`wayland` 需要 `WAYLAND_DISPLAY` +
  `XDG_RUNTIME_DIR`，否则硬错，**不**回退 X11）。
- 其它值 / 大小写不符 / 空或含空白 / `wayland,x11` 之类的多后端列表：硬错（引用原值），退出 2。
- 仅 `Auto` 会把**解析结果**导出为 `GDK_BACKEND`，使 GDK 不能自行回退；用户显式值从不改写。

**实际**后端/协议身份由运行证据单独证明（`[probe] display backend … resolved=…` + 顶层
`wl_surface`/`wl_display` 对 + app-owned registry/子表面协议）。选择意图**不等于**运行证据。

原生 Wayland 的 CAD 分区是 **app-owned 子表面**：宿主自建 child `wl_surface` + `wl_subsurface`，
父级为 GDK toplevel 的 `wl_surface`；渲染器只接收该 child 的 live Wayland raw handles。

- **私有小 C shim**（`apps/app-linux/native/wayland_shim.{c,h}`）用系统 `wayland-client-protocol.h`
  包装；借用 GDK 的 `wl_display`，**不**新建连接、**不** `dispatch`/`roundtrip`/`flush`——
  **GTK 是唯一的事件分发者**（Default Queue）。
- **异步 registry + discovery 屏障**：registry 请求后追加 `wl_display_sync`；其 `done` 表示初始
  global 已送达。**缺失/过老的 global（`wl_compositor < v3`）是显式失败**，不会永远 Pending。
- **父级 commit 语义**：`wl_subsurface.set_position` 是父级相对的双缓冲状态，在下一次**父级** commit
  生效；宿主只请求 GTK toplevel `queue_draw`，**从不**自行提交 GDK surface。
- **空 input region**：child 的 input region 为空，使 CAD 区域的指针/滚轮事件**穿透到 DOM**（`NULL`
  表示全表面输入，禁止）。Wayland 上**不**对 GDK 窗口调用 `input_shape_combine_region`（会改到共享的
  顶层表面）。
- **整数 DPR**：buffer scale 取 GTK 的整数 `scale_factor`；**只测过 `scale=1`**，DPR>1 与分/分数缩放
  **不支持、未验证**（显式）。
- **生命周期不变量**：释放顺序 **presenter → subsurface → child surface**，且**早于** GTK parent
  unrealize；`Ready` 时每帧/resize 前做原生健康检查（shim/display 错误 + 当前 GDK 父 `wl_surface`
  身份），不健康则有序释放，绝不提交陈旧帧。
- **构造期 panic 边界（有界）**：wgpu-hal 30 在 `vkCreateWaylandSurfaceKHR` 失败时 panic，故对整段
  presenter 构造做有界 `catch_unwind`，诊断如实写 `presenter construction panicked: …`；仅在默认
  unwind（非 `panic = "abort"`）下有效，终态不重试。

## 原生 GTK/wgpu 分区与合成约束

宿主把原生 CAD surface 合成在 GTK `GtkOverlay` 内的中央分区上，React `WebView` 作为透明 sibling
呈现 chrome，二者 **co-visible**。CAD 分区的原生表面**按后端不同**：

- **X11/XWayland**：`GtkDrawingArea` 获得自己的**原生 X child 窗口**，渲染器用其 Xlib XID/display。
- **原生 Wayland**：`GtkDrawingArea` 保持**非 native**（**不** `gdk_window_ensure_native`），宿主自建
  **app-owned `wl_subsurface`**（见「后端策略与原生 Wayland 子表面」）并只把该 child 交给渲染器。

两种后端都**不允许**在顶层窗口或 GDK-owned surface 上直接建 Vulkan swapchain，也**不做** CPU 帧拷贝。

- **不允许**跨 CAD 分区的 DOM 浮层。下拉/自动补全/命令历史**就地展开**（撑高所在 region），
  面板（Start、Diagnostics）**停靠**并挤窄分区，绝不覆盖原生 surface。
- `.native-input` CAD 分区始终挂载，上报 `getBoundingClientRect()`（CSS px）+ DPR 作为
  `probe_input` 的 `resize`；序号单调递增。窗口 resize、DPR 变化、ribbon 展开/折叠、面板停靠、
  命令历史展开、清屏切换都会重报，宿主 `LayoutTracker` 据此保持一致，**滚轮缩放才会锚定光标**。
- 坐标区分 **CSS / canvas-local / world / physical** 四类，配合 DPR 与 metrics revision。

## 外壳结构（P4 设计）

自上而下：标题/QAT → 功能区（选项卡→面板）→ 图纸标签 → 工作区（左侧停靠面板 | 原生 CAD 分区 |
右侧导航栏）→ 模型/布局标签 → 命令行（含补全与 F2 历史）→ 状态栏。所有用户可见文案来自
`crates/cad-ui-core/i18n/{zh-CN,en}.json`，组件内不写死文案。

## 支持范围

- **OPEN / FIT 已支持**：`--open` 经真实文件打开漏斗；OPEN 也可走原生 portal picker；FIT 适应图纸。
- **原生导航**按实际源码能力支持（滚轮缩放锚定光标、平移）；其余命令/面板/绘图编辑均**未接线**：
  一律**显式禁用并标注原因**（`command.unsupported`、`command.badge.unwired`、
  `status.toggle_unsupported`、`layout.switch_unwired` 等），不伪造可用。
- **只读**：不写回 DWG；**不新增投机 IPC**；未知命令走既有 **not supported** 路径，不空成功。
- 语言切换（chrome 文案）已接线；**运行时语言切换不重排 Rust 宿主的宿主侧字符串**（见限制）。
- 导航复用**同一权威底图**（GPU-resident、同一 DB revision）；拖拽期间 overlay 后缀可经显式
  `FrameCompleteness::BaseOnlyPendingOverlays` **临时省略**（非静默），`PresentedFrame.view_revision`
  = 实际 camera。

## 原生文件选择器（portal helper）与授权

`OPEN` 使用桌面 **native portal picker**。该路径需要一个**已授权的 portal helper**，使用
**transient**（一次性）工具，并要求**用户可观察**的交互确认；它**不修改默认权限**——脚本与文档
**不**对 `PermissionStore`/portal 做常驻授权变更。运行需要真实显示与桌面会话；
**无显示的 SSH/无头环境不受支持**。

## 证据机制（真实 GPU 路线）

宿主内置 env-gated 证据驱动：`apps/app-linux/src/evidence.rs`（`YACR_PROBE_EVIDENCE` 合成输入/
resize/minimize/restore；`YACR_PROBE_LIFECYCLE` 生命周期），以及 host telemetry 日志
（`[probe] …` 行，`YACR_PROBE_CAD_LOG` 输出 JSON 状态，`[probe] geometry …` 记录 DOM rect/容器/DrawingArea/
origin/scale/extent）。窗口捕获可用 `apps/app-linux/scripts/x11-window-grab.c`（**X11** 窗口）。原生 Wayland
身份由协议日志（`WAYLAND_DEBUG=client` 的 child `wl_surface`/`get_subsurface`/`set_desync`/`set_position`/
空 `input_region`）+ 适配器遥测证明；**原生视觉**由自身同 UID 的 Mutter `ScreenCast`（无安全策略变更/无安装/
无 sudo）捕获的合成帧佐证（**有界截图视觉 PASS**，无 golden/精度声明）。X11 窗口截图**不是**原生证据。

## 验证状态与限制（不夸大）

- **Linux（X11/XWayland 或原生 Wayland）真实可运行**；Windows/macOS/Android/Web 都是显式
  NOT IMPLEMENTED 占位，不是可运行 GUI 产品。
- **仅 OPEN/FIT + 原生导航接线**；其余 UI 能力未接。**不是完整 CAD 产品**。
- 运行时语言切换不能重排宿主侧字符串（宿主启动时读一次 `YACR_LOCALE`）。
- **原生 Wayland 有界运行已执行（有界、分区）**：scoped `cargo check --all-targets`、`cargo test --lib`
  （`app-linux` **58/58**、`cad-ui-core` **61/61**）、clippy（0 warning）、wasm32 cfg 隔离、
  `check-architecture.py`、`check-i18n.py`、scoped `fmt`；原生 `--probe`（物理 Vulkan P620、真实 child
  `wl_subsurface` 协议身份）；原生 **CAD `--open`**（4 实体、`current_scene`、`error=None`）+ **9 个合成 DOM 输入**
  （全 `accepted`，相机实际改变——**≠ 真实用户指针像素精度验收**）+ resize 遥测；强制 **overlay** 生命周期
  释放/重建/恢复（**非** toplevel `wl_surface` 替换）；几何修复后 **Observer 有界截图视觉 PASS**；X11 运行回归
  PASS。退出码按 case：原生 baseline 捕获后 SIGTERM **143**，resized 与 X11 **自然 0**。完整命令/退出码/日志/
  边界见 `docs/validation-native-wayland-2026-10-16.md`。
- **仍未运行 / 未支持**：**DPR>1/分数缩放**（只测 `scale=1`）、GTK **toplevel**
  替换、GPU 设备丢失、vendor 广度、几何/像素精度、性能/60fps、跨平台、打包、vendor/golden。
  **不得**据此宣称“完整原生 Wayland CAD 支持”。
- **最终 Oracle（W3 attempt3）：ACCEPT-WITH-LIMITATIONS（无 blocker）** —— **有界本地验收**（GNOME/Mutter +
  Quadro P620 + `scale=1`），不是完整产品/跨平台/vendor/golden/性能验收；X11 视觉**未**由 observer 判读
  （仅运行证据 PASS）。

- **P6 最终验证已执行（有界、分区）**：前端 43/43 + 构建、`cargo fmt/clippy`、四个 Python 门禁与十个
  Python 契约（含 18 项迁移守卫）、`--locked` 编译/wasm/workspace 测试（1825 passed/0 failed/19 ignored）、
  真实硬件 GPU（Quadro P620 / Vulkan / 真实 GTK XWayland）的 open+渲染 smoke、原生分区生命周期
  （forced unrealize→新 XID + generation 2、自然 exit 0）、外部 WM minimize→restore 与真实 resize；
  完整命令/退出码/日志见 `docs/validation-react-linux-migration-2026-10-16.md`。
- **仍未验证 / 未支持**：vendor DWG 兼容、精确视觉 golden、`input→visible` 性能、60fps 声明、
  真实鼠标光标锚定像素验收、真实 portal picker、设备丢失/跨平台一致性、打包。**gate 结论**：Gate P6
  attempt2 **ACCEPT-WITH-FIXES**（文档限定已应用）；observer 终判所观察截图**无重大视觉缺陷**；
  **有界本地 Linux 迁移验收通过**，**不是**完整 CAD 产品/跨平台/vendor/golden/性能验收。应用侧
  `unminimize()`+强制 unrealize 序列**未解决**。
  旧 Slint 阶段的视觉/性能数字属历史证据，不能移植为 React 宿主结论。
- 旧 Slint 真实 GPU 测试套件与其多进程 runner 已退役，**无 cargo-test 等价替代**；P6 的真实 GPU 程序
  由 env-gated 证据驱动 + 硬件 Vulkan 完成，**不得**用软件后端或旧结论冒充，也不等同于旧 7 用例套件。
- 不承诺跨语言稳定 IPC；不扩张系统权限。

## 隐私 / 锁状态

截图/证据**仅本地保存，未上传**。脚本不改变会话锁状态；不得声称 always locked。
