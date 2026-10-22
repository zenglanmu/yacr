# React 原生 Linux 宿主（当前桌面宿主）

状态：**当前桌面 UI 实现**（2026-10-16）。取代 Slint 桌面路线，见
[`docs/adr/0010-react-linux-host-replaces-slint.md`](adr/0010-react-linux-host-replaces-slint.md)；
原生 Wayland 后端见
[`docs/adr/0011-native-wayland-subsurface.md`](adr/0011-native-wayland-subsurface.md)。
与 `CAD_IMPLEMENTATION_SPEC.md` §21。**仍是 bounded 实现，不是完整 CAD 产品**，也不代表跨平台验收。
后端支持 **X11/XWayland** 与**原生 Wayland** 两者（原生 Wayland 的有界 `--probe` 已跑通，CAD/视觉
仍待验证，见 `docs/validation-native-wayland-2026-10-16.md`）。

- 应用：`apps/app-linux`；二进制 `yacr`（Tauri 标识 `dev.yacr.app`，产品名 `Yacr`）。
- 前端：`apps/desktop-ui`（所有桌面宿主共用的唯一 React UI 包；Vite + React 19）。
- 平台无关 UI 逻辑：`crates/cad-ui-core`（protocol/layout/session/shell/i18n）。
- 入口脚本：`scripts/run-react-linux.sh`（在仓库任意目录调用；用 git 定位根、路径加引号、
  `set -euo pipefail`）。

## 构建依赖（系统包需预先安装，脚本不自动安装）

- **WebKitGTK 4.1 开发包**（`libwebkit2gtk-4.1-dev`）与 **GTK3 开发包**（`libgtk-3-dev`）；
  `pkg-config --exists webkit2gtk-4.1 gtk+-3.0` 应通过。
- **Node + npm**（用于 Vite 构建 `apps/desktop-ui`）。
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
(cd apps/desktop-ui && npm ci && npm run build)
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

参数（透传给宿主）：`--open <path>`、`--fonts-dir <path>`、`--images-dir <path>`、`--probe`，
外加**裸位置参数**：`yacr <path>` 与 `yacr --open <path>` 等价（flatpak 后由文件管理器
`Exec=… %F` 调用的前置条件）。**语义（已实现，有真机证据，见 `docs/handoff.md` 2026-10-17 小节）**：

- 多个位置参数只打开**第一个**，其余**显式诊断**（多文档打开 NOT IMPLEMENTED，绝不静默丢弃）；
- 同时给 `--open` 与位置参数时 **`--open` 优先**（与顺序无关），被忽略的位置参数**显式报告**；
- 任何含 `://` 的参数（如 `file://…`）**显式拒绝**且**绝不**当路径使用 ⇒ 未来 desktop entry
  **必须**用 `%F` 而非 `%U`；
- 未知 `-flag` 行为不变（仍忽略、不新失败）；不存在的路径仍走**同一**打开漏斗并**显式报错**
  （`cad_status.error`，如 `drawing path does not exist: …`），**不**静默空成功。

⚠ **构建陷阱（症状级）**：普通 `cargo build --release -p app-linux` **不会**嵌入前端——
`generate_context!` 会退回 `build.devUrl`（`http://localhost:1420`），产出**无法独立运行**的二进制。
症状：`[probe] page_load url=http://localhost:1420/`，且持续 `[probe] cad-frame no-session`
（CAD 会话永不建立）。**必须**用上面的 `--features tauri/custom-protocol`（或 `scripts/collect-evidence.sh`）。
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
呈现 chrome，二者 **co-visible**。

**当前默认堆叠顺序**：CAD 绘图区被**重新 `add_overlay` 到最后**，因此是 overlay 的**最上层** child，其原生表面（X11 子窗口 / Wayland 子表面）在该矩形内**盖住 WebView**；只有**输入**经空 input shape/region 穿透。⇒ **在此产品安排下 DOM 不能浮在 CAD 分区之上**（ADR-0010、ADR-0020），这不是 GTK/Linux 的固有要求；见下方有界探针与 ADR0010 的2026-10-22增补。

**输入路由的实际后果（2026-10-20 记录）**：CAD 原生表面**只负责显示**——它把 OS 输入全部让给下面的 DOM
（`gtk_bridge/native.rs` 的 `apply_input_passthrough`），由 `.native-input` 把 `pointerdown/move/up`（含
`button: 0/1/2`）与 `wheel` 经 `probe_input` 转发给会话。因此中键平移是 `button=1` 的 DOM 事件；
**右键也落在 WebView 上**，若不处理就会弹出 WebKitGTK 默认菜单（含 Reload —— 会重新挂载 UI 而 Rust 会话与
原生表面仍在，属**状态分叉风险**）。现已：外壳范围抑制 `contextmenu`（**输入框内保留**，命令行仍可复制粘贴）；
拦掉 `Ctrl+R`/`Ctrl+Shift+R`/`F5`、`Ctrl±`/`Ctrl+0`、`Ctrl+P`、**`Ctrl`+滚轮**（页面缩放会改变 DOM 矩形与比例，
与每帧"绘图区必须匹配 DOM 矩形"的守卫冲突）。**CAD 区右键目前无动作**，只经单一调用点发出
`cadContextMenuRequest({point, selection, tool})` 作为**未来随手菜单的钩子**（今天不产生任何可见行为）。
**注意**：这不是"测试用了合成输入"的取巧——**CAD 表面从不接收真实 OS 输入**；指针捕获、手写笔、双击时序
全都经过 DOM。要改变这一点属于架构级变更（需单独 ADR）。

**弹窗层（ADR-0021，2026-10-20 落地第 1 步）**：模态/对话框一律是**独立原生窗口 + 自己的 child webview**
（`host/popup.rs`：`WindowBuilder` + `add_child(WebviewBuilder)`，与主窗同构），**不是** DOM 浮层也不是 `GtkPopover`；
载荷经 `initialization_script` 注入 `window.__YACR_POPUP__` 字面量。模态期间 `ShellState.popup` 是**权威 gate**
（`probe_input` 除 `resize` 外与 `cad_command` 均拒绝 `popup.modal_open`），且**只在窗口 `Destroyed` 清除**；
UI 侧另有两个**独立**挂起理由（`textFocus` / `popupOpen`），命令行失焦**不能**解除弹窗的挂起。CAD 分区的原生表面**按后端不同**：

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

### 可选 DOM-on-top 实验（不是产品迁移）

在当前用户的真实图形会话中，先确保另一终端的 Vite 开发服务已启动
（`cd apps/desktop-ui && npm run dev`），再执行：

```bash
cargo build -p app-linux --bin yacr --locked
GDK_BACKEND=wayland YACR_PROBE_DOM_ON_TOP=1 ./target/debug/yacr --probe
# 上一次退出后，单独调查 X11/XWayland（本次安排合成失败，不是推荐产品路线）：
GDK_BACKEND=x11 YACR_PROBE_DOM_ON_TOP=1 ./target/debug/yacr --probe
```

仅值 `1` 启用，必须显式 `--probe`；`YACR_PROBE_ONLY` 不替代该要求。宿主预先注入布尔全局，
前端使用隔离夹具与既有输入准入；未设置时产品行为不变。上述 debug 构建实际访问
`http://localhost:1420/`，**不是**嵌入的 `ui/dist`。诊断对象为
`window.__yacr_dom_on_top_probe`，IPC resolved 不等于 accepted。

本机原生 Wayland 的采样合成图像已通过遮挡/混合/局部弹窗与标记运动复核；XWayland 验证发现
另一应用从透明区域透出，故 **FAIL**，并非原生 sibling 排序成功就代表合成正确。两者都不是完整
产品或跨合成器结论。运行证据、安全/隐私、未测物理输入与生命周期缺口见
[2026-10-22 验证记录](validation-dom-on-top-linux-2026-10-22.md)。

## 外壳结构（P4 设计）

自上而下：标题/QAT → 功能区（选项卡→面板）→ 图纸标签 → 工作区（左侧停靠面板 | 原生 CAD 分区 |
右侧导航栏）→ 模型/布局标签 → 命令行（含补全与 F2 历史）→ 状态栏。所有用户可见文案来自
`crates/cad-ui-core/i18n/{zh-CN,en}.json`，组件内不写死文案。

功能区面板带为**固定高度**（`--ribbon-h`，`apps/desktop-ui/src/style.css`），所有选项卡一致，
切换选项卡**不改变** chrome 高度（因而不会重报 CAD 分区矩形）；小按钮在固定高度内按列优先网格
**横向**排布，超宽时面板带横向滚动、不静默丢命令。变体下拉改为**带内浮层**（绝对定位于面板
body 内，不越出功能区、不撑高），Escape/点击外部关闭。契约测试见
`apps/desktop-ui/test/ribbon.test.mjs`。

## 支持范围

**命令准入以 Rust 为唯一来源**：前端启动时经 `cad_commands` 取
`cad_ui_core::session::admissible_names()`（`COMMAND_TABLE` + 别名，见
`crates/cad-ui-core/src/session/commands.rs`），**不再**手写 `OPEN`/`FIT` 白名单；宿主未应答前目录为空，
所有命令显式 unsupported（不是默认放行）。React 宿主**显式排除** `NEW`/`SAVE`/`SAVEAS`/`QSAVE`/
`PLOT`/`EXPORT`/`PRINT`（需要原生对话框/打印归属，本宿主未实现），按钮保持禁用并给出原因。

- **可从 chrome 触达**：`OPEN`（`--open` 真实打开漏斗，或原生 portal picker 见下节）、`FIT`、`PAN`、
  `SELECTALL`、`UNDO`、`REDO`、`ORBIT`、`ZOOM`、`OSNAP`（**F3** 同义，切对象捕捉总开关）、
  `HOVERPREVIEW`（状态栏「悬停预览」开关，默认开），以及
  交互工具 `LINE`/`CIRCLE`/`ARC`/`MOVE`/`COPY`/`ROTATE`/`SCALE`/`MIRROR`/`OFFSET`/`TRIM`/`EXTEND`
  （经 `InteractionRequest::BeginDraw`）与 `MEASURE`（Distance）。
- **交互语义**：工具激活后画布单击 = 取点（`CapturePoint`，不再是选择）；`Enter` = `CONFIRM`，
  `Esc` = `CANCEL`（`CANCEL` 的捕获态在提交**之前**采样：有 draw/measure 捕获时只取消工具，不转发宿主
  open-cancel）；状态栏显示 `status.tool`（机器键 `draw.*`/`measure.*` + 已取点数 + 可确认性）。
  提交语义：**绘制/编辑工具不会自动提交**，必须显式 `CONFIRM`（`cad_app::SessionState::capture_point`
  对 `Drawing` 恒返回 `false`）；测量工具按自身 `auto_ready` 自动提交。
- **鼠标手势（ADR-0014 决策 B，行为变更）**：**2D 左键拖拽（≥4 逻辑像素）= 框选**（左→右 WINDOW /
  右→左 CROSSING，按住时画橡皮筋：走 **renderer overlay**，`BOX_SELECT_DRAW_ORDER`；WINDOW 蓝实线、
  CROSSING 绿虚线）；`Shift` 拖拽 = 加选，否则替换；**中键拖拽 = 平移**（2D/3D），滚轮 = 缩放；
  3D 左键拖拽仍是**轨道旋转**（3D 框选显式 NOT IMPLEMENTED）；阈值以下仍是单点 pick。Secondary/
  Back/Forward 的拖拽行为本轮**未改**（仍为平移；AutoCAD 该位置是右键快捷菜单，未实现）。
- **选择要求**：编辑类工具需要先有选择；空选择在确认时由 `cad_app`（`DrawTool::is_ready`）显式拒绝，
  UI 不预判、不静默丢弃。
- **仍未接线**：`GRID`/`SNAP`/`ORTHO` drafting 开关（AutoCAD 的 `SNAP`=栅格捕捉 F9，与本项目的
  对象捕捉 `OSNAP` 是两件事，前者未实现）、图层/特性面板、`ZOOMIN`/`ZOOMOUT`
  别名（router 名为 `ZOOM`）、测量子类型 `DIST`/`PLINE`/`ANGLE`/`AREA`、`NEW`/`SAVE`/`PLOT`、
  `SELECTIONPREVIEWLIMIT` 式预览上限、`PREVIEWFILTER` 的锁定图层/xref 排除、Secondary 拖拽的右键菜单。
- **只读**：不写回 DWG；未知命令走既有 not supported 路径，不空成功。
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
resize/minimize/restore；`YACR_PROBE_LIFECYCLE` 生命周期；**`YACR_PROBE_TOOL_EVIDENCE`** 交互工具
端到端——读真实 `cad_status` 基线 → `cad_command LINE` → 两次合成点击取点 → `cad_command CONFIRM`
→ 读回 `entityCount`，通过判据是**真实实体数增量**且 `tool` 回到 `null`，**非**伪造），以及 host telemetry 日志
（`[probe] …` 行，`YACR_PROBE_CAD_LOG` 输出 JSON 状态，`[probe] geometry …` 记录 DOM rect/容器/DrawingArea/
origin/scale/extent）。窗口捕获可用 `apps/app-linux/scripts/x11-window-grab.c`（**X11** 窗口）。原生 Wayland
身份由协议日志（`WAYLAND_DEBUG=client` 的 child `wl_surface`/`get_subsurface`/`set_desync`/`set_position`/
空 `input_region`）+ 适配器遥测证明；**原生视觉**由自身同 UID 的 Mutter `ScreenCast`（无安全策略变更/无安装/
无 sudo）捕获的合成帧佐证（**有界截图视觉 PASS**，无 golden/精度声明）。X11 窗口截图**不是**原生证据。

**一键自动采集**：`scripts/collect-evidence.sh` 自动完成「构建 → 启动真实 `yacr`（`--open`，非
`--probe`）→ 截图 → 汇总日志」：X11 用 `apps/app-linux/scripts/x11-window-grab.c` 抓窗口自身 drawable
（`shot-top`/`shot-cad`），原生 Wayland 用 `scripts/screencast-capture.py`（同 UID Mutter `ScreenCast`）
抓合成帧（`wayland-frame`），输出 `${YACR_EVIDENCE_ROOT:-/tmp/opencode}/yacr-evidence-<UTC>Z/`
（`results.tsv` 逐后端记录真实 verdict/退出码/截图是否产出 + `README.md`）。契约测试
`python3 scripts/test-collect-evidence.py`。该脚本以同 UID 会话运行，**不提权/不 `sudo`/不解锁会话/
不改 portal 权限/不装包/不起虚拟显示**，无真实显示时非零退出；截图仅本地保存，不代表视觉正确性。
X11 截图只含应用窗口本身；Wayland 帧为**整屏合成**（`RecordMonitor`），会含桌面上其它窗口。

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
