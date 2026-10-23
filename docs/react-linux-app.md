# React 原生 Linux 宿主（当前桌面宿主）

状态：**当前桌面 UI 实现**（2026-10-16；2026-10-23 合成方向已反转为 DOM-on-top）。取代 Slint 桌面路线，见
[`docs/adr/0010-react-linux-host-replaces-slint.md`](adr/0010-react-linux-host-replaces-slint.md)；
原生 Wayland 后端见
[`docs/adr/0011-native-wayland-subsurface.md`](adr/0011-native-wayland-subsurface.md)；
跨宿主合成模型见 [`docs/adr/0023-dom-on-top-cross-host-composition.md`](adr/0023-dom-on-top-cross-host-composition.md)。
与 `CAD_IMPLEMENTATION_SPEC.md` §21。**仍是 bounded 实现，不是完整 CAD 产品**，也不代表跨平台验收。

**后端（2026-10-23 起）**：只有**原生 Wayland**是受支持的后端。**X11/XWayland 显式 Unsupported**：
在 GTK/Tauri 初始化前拒绝，stderr 稳定标记 `yacr: unsupported-backend: x11`，退出码 **4**，
**无静默回退**。生产合成方向是 **DOM/WebView 在上、app-owned CAD 子表面在下**（`place_below`），
不再是“CAD 分区盖住 WebView”。有界验收仅限 **GNOME/Mutter + Quadro P620 + `scale=1`**
（见 `docs/validation-dom-on-top-production-2026-10-23.md`）。


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
（嵌入 `../desktop-ui/dist`）。脚本**不解锁会话、不截图、不安装系统包**。

先构建前端（`dist` 会被嵌入），再运行（原生 Wayland，唯一受支持后端）：

```bash
(cd apps/desktop-ui && npm ci && npm run build)
env -u DISPLAY -u XAUTHORITY \
    GDK_BACKEND=wayland WAYLAND_DISPLAY=wayland-0 XDG_RUNTIME_DIR=/run/user/1000 \
    ./target/release/yacr --open /abs/drawing.dwg
# 或先构建：cargo build --release --locked -p app-linux --bin yacr --features tauri/custom-protocol
```

**X11/XWayland 会被拒绝**（不是“保留的替代路径”）：

```bash
GDK_BACKEND=x11 DISPLAY=:0 ./target/release/yacr --open /abs/drawing.dwg
# stderr: yacr: unsupported-backend: x11  →  exit 4（无窗口、无截图）
```

`Auto` 在只有 `DISPLAY` 的会话里同样解析为 X11 并被拒绝；只有
`WAYLAND_DISPLAY` + `XDG_RUNTIME_DIR` 同时存在才进入原生 Wayland。理由（XWayland 合成曾 FAIL、
机制未闭环）与后续 web+canvas 路线见 [ADR 0023](adr/0023-dom-on-top-cross-host-composition.md) D3。

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
`YACR_PROBE_LIFECYCLE`、`YACR_PROBE_TOOL_EVIDENCE`、`YACR_PROBE_STACKING`、`YACR_PROBE_ONLY`、
`YACR_PROBE_NO_GPU`、`YACR_PROBE_OPAQUE`、`YACR_PROBE_NO_MAXIMIZE`、
`YACR_PROBE_ALLOW_CPU_ADAPTER`、`YACR_PROBE_CAD_LOG`。
`YACR_PROBE_ALLOW_CPU_ADAPTER` 是**存在即生效**（取值无关）的 bring-up/证据开关：让 CPU/软件 Vulkan
适配器（如 VM 里的 lavapipe）可被 probe presenter 选中，硬件适配器仍严格排在前面；**未设置**时仍
拒绝 CPU 适配器。它**不是**生产放宽——软件渲染的帧/耗时不得当作真实 GPU 结论。
**已删除**：`YACR_PROBE_DOM_ON_TOP`（旧 opt-in 探针开关；不再是有效入口）。

**渲染后端偏好（`wgpu_backend`）**：持久偏好 `preference.json` 的顶层键 `wgpu_backend`（与其它用户
偏好同一 durable store；取值**恰为** `auto|vulkan|dx12`，缺失/空/未知一律 = `auto`）在**每次
presenter 构造**时读取一次（非每帧），映射为渲染器的 `ProbeBackendPolicy`：`vulkan` = 历史的
Vulkan-only；`dx12` 在 Linux **显式失败**（`ProbeError::UnsupportedBackendPolicy`，无静默回退）；
`auto` = Vulkan + DX12 且同层 Vulkan 优先。启动新增一行
`[probe] backend-policy wgpu_backend=… (…)` 便于证据判读（与 `[probe] adapter-policy …` 并列）。
**DX12 在 Linux 无运行证据**；`auto` 在 Linux 实际只会枚举到 Vulkan 适配器。
同一键也是 Options 对话框（User 页签）“WGPU backend”行的**写入契约**：`cad_settings` 以**蛇形**
字段 `wgpu_backend` 上报、`cad_set_setting` 用同一 `wgpu_backend` 键写入（不 camelCase、仅接受
`auto|vulkan|dx12`，其它值回 `settings.invalid_value`）；该键**不在** `ViewerConfig` schema 内，
所以启动时先把它从偏好文档剥离再做 allowed-path 校验、并在每次偏好写入时合并回顶层，写入后
**下次 presenter 构造**才生效（无热切换后端）。

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

- **未设（`Auto`）**：`WAYLAND_DISPLAY` + `XDG_RUNTIME_DIR` 同时存在 → **原生 Wayland**；
  否则仅有 `DISPLAY` → 解析为 X11，并**被显式拒绝**（标记行 + 退出码 4）；两者皆无 → 明确报错
  （退出 2，**没有无头成功**）。
- **`GDK_BACKEND=wayland`**：显式接受（需要 `WAYLAND_DISPLAY` + `XDG_RUNTIME_DIR`，否则硬错，
  **不**回退 X11）。
- **`GDK_BACKEND=x11`**：**显式 Unsupported**——在 GTK/Tauri 初始化之前打印
  `yacr: unsupported-backend: x11` 与 i18n 诊断，退出 **4**；不创建窗口，也不落到旧排列。
- 其它值 / 大小写不符 / 空或含空白 / `wayland,x11` 之类的多后端列表：硬错（引用原值），退出 2。
- 仅 `Auto` 会把**解析结果**导出为 `GDK_BACKEND`，使 GDK 不能自行回退；用户显式值从不改写。

**实际**后端/协议身份由运行证据单独证明（`[probe] display backend … resolved=…` + 顶层
`wl_surface`/`wl_display` 对 + app-owned registry/子表面协议）。选择意图**不等于**运行证据。

原生 Wayland 的 CAD 分区是 **app-owned 子表面**：宿主自建 child `wl_surface` + `wl_subsurface`，
父级为 GDK toplevel 的 `wl_surface`；渲染器只接收该 child 的 live Wayland raw handles。

- **合成方向（生产，唯一）**：`yacr_wl_shim_create_child` **无条件**调用
  `wl_subsurface_place_below(subsurface, parent)`——CAD 在 DOM/WebView **之下**。旧的
  above-parent 排列与开关**已删除**，**无双模式**；每个 surface generation 重建时重放该调用
  （N>1 次 generation 的**运行期**重放仍 NOT RUN，仅代码路径验证）。
- **私有小 C shim**（`apps/app-linux/native/wayland_shim.{c,h}`）用系统 `wayland-client-protocol.h`
  包装；借用 GDK 的 `wl_display`，**不**新建连接、**不** `dispatch`/`roundtrip`/`flush`——
  **GTK 是唯一的事件分发者**（Default Queue）。
- **异步 registry + discovery 屏障**：registry 请求后追加 `wl_display_sync`；其 `done` 表示初始
  global 已送达。**缺失/过老的 global（`wl_compositor < v3`）是显式失败**，不会永远 Pending。
- **父级 commit 语义**：`wl_subsurface.set_position` 与 `place_below` 都是父级相对的双缓冲状态，
  在下一次**父级** commit 生效；宿主只请求 GTK toplevel `queue_draw`，**从不**自行提交 GDK surface。
- **顶层 alpha（生产必需）**：GTK toplevel 必须 `set_app_paintable(true)`、在 draw 中
  `set_opaque_region(None)` 并用 `Operator::Source` 透明清屏（`gtk_bridge/owner.rs::install_parent_alpha_clear`，
  无条件执行）；否则 `place_below` 的 CAD 永远不可见。
- **空 input region**：child 的 input region 为空，使 CAD 区域的指针/滚轮事件**穿透到 DOM**（`NULL`
  表示全表面输入，禁止）。输入本来就全部走 DOM；空 region 是**防御纵深**（CAD 绝不吞事件后无人转发）。
- **整数 DPR**：buffer scale 取 GTK 的整数 `scale_factor`；**只测过 `scale=1`**，DPR>1 与分/分数缩放
  **不支持、未验证**（显式）。
- **生命周期不变量**：释放顺序 **presenter → subsurface → child surface**，且**早于** GTK parent
  unrealize；`Ready` 时每帧/resize 前做原生健康检查（shim/display 错误 + 当前 GDK 父 `wl_surface`
  身份），不健康则有序释放，绝不提交陈旧帧。
- **构造期 panic 边界（有界）**：wgpu-hal 30 在 `vkCreateWaylandSurfaceKHR` 失败时 panic，故对整段
  presenter 构造做有界 `catch_unwind`，诊断如实写 `presenter construction panicked: …`；仅在默认
  unwind（非 `panic = "abort"`）下有效，终态不重试。

## 原生 GTK/wgpu 分区与合成约束（DOM-on-top）

宿主把原生 CAD surface 合成在 GTK `GtkOverlay` 内的中央分区上，React `WebView` 承载 **同一棵 UI 树**；
二者 **co-visible**，且 **DOM/WebView 在上、CAD 原生子表面在下**（`place_below`）。

**z 序与透明度**：CAD 子表面被无条件压到父表面**下方**；它在该矩形内仍由 wgpu 绘制，但因位于 WebView
之下，DOM 可以在其上绘制浮层、半透明装饰与跨 chrome/CAD 边界的弹窗。GTK toplevel 必须保持透明
（app-paintable、空 opaque region、`Operator::Source` 清屏），否则下方的 CAD 不可见。

**生产透明不变量（来自真实缺陷）**：`.native-input` 的**所有祖先**在 CAD 分区矩形内必须透明——
`.shell`、`.workspace`、`.workspace__canvas` **不涂不透明底色**，各 chrome 区（ribbon/面板/命令行/状态栏）
自己涂不透明底。WP2 之后的手工测试曾发现 CAD 层不可见，根因正是这些祖先在视口区画了不透明底；
修复见 `6e4bf06`，并由 `apps/desktop-ui/test/metrics.test.mjs` 的结构性回归测试钉住。新增布局**仍须**
遵守该规则，否则会再次遮住 CAD（像素复采有界 PASS：几何满亮度、无蒙层；见验证记录）。

**输入路由（单一漏斗，跨平台同构）**：CAD 原生表面**只负责显示**——它不接收任何 OS 输入。
`.native-input`（DOM）把 `pointerdown/move/up`、`wheel`、`contextmenu` 与键盘手势经 `probe_input` 转发给
会话；空 input region（Wayland）是防御纵深。因此中键平移是 `button=1` 的 DOM 事件。**注意**：这不是
“测试用了合成输入”的取巧——CAD 表面从架构上就不接收真实 OS 输入；**真实 OS 键盘/指针从未用于验证
任何输入路径**。

**WebView 默认行为必须抑制**：右键若不处理会弹 WebKitGTK 默认菜单（含 Reload——会重新挂载 UI 而 Rust
会话与原生表面仍在，属状态分叉风险）。现已：外壳范围抑制 `contextmenu`（输入框内保留，命令行仍可
复制粘贴）；拦掉 `Ctrl+R`/`Ctrl+Shift+R`/`F5`、`Ctrl±`/`Ctrl+0`、`Ctrl+P`、**`Ctrl`+滚轮**（页面缩放会
改变 DOM 矩形与比例，与每帧“绘图区必须匹配 DOM 矩形”的守卫冲突）。

**DOM 覆盖区的命中规则**（一条产品规则，ADR 0023 D2）：

- 不透明 chrome 控件：正常命中；
- 半透明装饰面板：`pointer-events: none` 让指针落到 CAD；面板内的交互控件用 `pointer-events: auto`
  显式收回。视觉即交互——看见图纸却点到死区是 UX 陷阱。

**不依赖原生命中穿透**：不使用 `WM_NCHITTEST`/`HTTRANSPARENT`（Windows 侧）也不声称具备点击穿透；
DOM 自己产生命中与事件，原生侧只消费队列。

**弹窗与浮层**：居中模态（about/textEdit/options）与锚定浮层（右键菜单、tooltip、跨边界下拉）现在都是
**同一 React 树的 DOM 覆盖层**（portal 到顶层 overlay），由 `cad_status.popup` 驱动显示。旧的“第二个
`tauri::Window` + 子 webview”机制已退役（`CadPopup::label` 只是冻结 wire 字段）。模态期间
`ShellState.popup` 仍是**权威 gate**（`probe_input` 除 `resize` 外与 `cad_command` 均拒绝 `popup.modal_open`），
UI 侧另有 `textFocus` / `popupOpen` 两个**独立**挂起理由。清除触发有**四条**：显式
`cad_close_popup`（含 Esc/关闭按钮）、webview reload/navigation、**渲染进程崩溃/失败**（WebKitGTK
`web-process-crashed`；崩溃既无 navigation 也无 close）、进程退出。关闭 affordance 挂在正文
error-boundary **之外**的最小 chrome 壳上，正文渲染失败时仍可关。

**CAD 分区的原生表面**：

- **原生 Wayland**：`GtkDrawingArea` 保持**非 native**（**不** `gdk_window_ensure_native`），宿主自建
  **app-owned `wl_subsurface`** 并只把该 child 交给渲染器（见上节）。
- **X11/XWayland**：旧的原生 X child 窗口代码**编译/测试保留、运行时不可达**（`unreachable_since_dom_on_top`），
  后端在 GTK 初始化前即被拒绝（退出码 4）。

两种路径都**不允许**在顶层窗口或 GDK-owned surface 上直接建 Vulkan swapchain，也**不做** CPU 帧拷贝。

- `.native-input` CAD 分区始终挂载，上报 `getBoundingClientRect()`（CSS px）+ DPR 作为
  `probe_input` 的 `resize`；序号单调递增。窗口 resize、DPR 变化、ribbon 展开/折叠、面板停靠、
  命令历史展开、清屏切换都会重报，宿主 `LayoutTracker` 据此保持一致，**滚轮缩放才会锚定光标**。
- 坐标区分 **CSS / canvas-local / world / physical** 四类，配合 DPR 与 metrics revision。

### 合成回归夹具（不是产品开关）

旧的 opt-in 探针（`YACR_PROBE_DOM_ON_TOP` + `--probe`）**已删除**。现行回归夹具是
`apps/desktop-ui/src/stacking-regression/`，触发契约是**三件套，缺一即启动拒绝**：

1. `YACR_PROBE_STACKING=1`（**严格**等于 `"1"`；仅“存在”不够）；
2. init-script 注入 `window.__YACR_STACKING_REGRESSION__ === true`；
3. **真实 `--open` 会话**（显式图纸）。

`--probe` 或 `YACR_PROBE_ONLY` 与 `YACR_PROBE_STACKING=1` 同现 ⇒ **启动即拒绝**（夹具不冒充产品会话）。
一键采集用：

```bash
bash scripts/collect-evidence.sh --backend wayland --stacking-regression \
    --dwg "$(pwd)/fixtures/dwg/synthetic-four-lines.dwg" --fonts-dir "$(pwd)/fonts"
```

它会在 `wayland-stacking-regression/` 下记录 `run.log` 与四帧
（`initial.png`/`popup-open.png`/`after-synthetic-input.png`/`after-synthetic-input-later.png`），并在
`results.tsv` 记一行 `STACKING_REGRESSION`。夹具证明的是**合成与输入转发**（像素与准入计数），
**不是**视觉正确性、精度或 vendor 结论；UI 侧“DOM 测试”是 node:test 的源码结构/纯函数契约，
不是运行时 DOM 挂载。


## 外壳结构（P4 设计）

自上而下：标题/QAT → 功能区（选项卡→面板）→ 图纸标签 → 工作区（左侧停靠面板 | 原生 CAD 分区 |
右侧导航栏）→ 模型/布局标签 → 命令行（含补全与 F2 历史）→ 状态栏。所有用户可见文案来自
`crates/cad-ui-core/i18n/{zh-CN,en}.json`，组件内不写死文案。

功能区面板带为**固定高度**（`--ribbon-h`，`apps/desktop-ui/src/style.css`），所有选项卡一致，
切换选项卡**不改变** chrome 高度（因而不会重报 CAD 分区矩形）；小按钮在固定高度内按列优先网格
**横向**排布，超宽时面板带横向滚动、不静默丢命令。变体下拉目前仍是**带内浮层**（绝对定位于面板
body 内，Escape/点击外部关闭）；DOM-on-top 之后这不再是硬约束，但也没有迁移理由。契约测试见
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
origin/scale/extent）。原生 Wayland 身份由协议日志（`WAYLAND_DEBUG=client` 的 child
`wl_surface`/`get_subsurface`/`set_desync`/`set_position`/`place_below`/空 `input_region`）+
适配器遥测证明；**原生视觉**由自身同 UID 的 Mutter `ScreenCast`（无安全策略变更/无安装/无 sudo）
捕获的合成帧佐证（**有界截图视觉 PASS**，无 golden/精度声明）。X11/XWayland 现在**没有运行证据**
（后端被拒绝，退出码 4）；保留的 `apps/app-linux/scripts/x11-window-grab.c` 是历史工具，**不得**用来
冒充当前后端的证据。

**一键自动采集**：`scripts/collect-evidence.sh` 自动完成「构建 → 启动真实 `yacr`（`--open`，非
`--probe`）→ 截图 → 汇总日志」：原生 Wayland 用 `scripts/screencast-capture.py`（同 UID Mutter
`ScreenCast`）抓合成帧（`wayland-frame`），输出
`${YACR_EVIDENCE_ROOT:-/tmp/opencode}/yacr-evidence-<UTC>Z/`（`results.tsv` 逐后端记录真实
verdict/退出码/截图是否产出 + `README.md`）。X11 仍会被运行一次以采集**拒绝合同**：`x11` 行应为
`UNSUPPORTED_BACKEND`，标注**标记行** `yacr: unsupported-backend: x11` 与退出码 4（分类以**标记行**
为第一依据，码值仅辅助），并**不产生窗口/截图**。契约测试 `python3 scripts/test-collect-evidence.py`。
该脚本以同 UID 会话运行，**不提权/不 `sudo`/不解锁会话/不改 portal 权限/不装包/不起虚拟显示**，
无真实显示时非零退出；截图仅本地保存，不代表视觉正确性。Wayland 帧为**整屏合成**
（`RecordMonitor`），会含桌面上其它窗口。

### 本轮 DOM-on-top 生产证据（2026-10-23，有界）

见 [`docs/validation-dom-on-top-production-2026-10-23.md`](validation-dom-on-top-production-2026-10-23.md)：

| 证据目录（`/tmp/opencode/`） | 内容 | 判读 |
|---|---|---|
| `yacr-evidence-wp2-final6` | X11 拒绝（exit 4 + 标记行）；Wayland 生产帧；stacking 夹具 4/4 帧 | Wayland 帧 PASS（带 caveat） |
| `yacr-evidence-wp3-final1` | Wayland 生产帧（DOM 覆盖层/about） | About 帧 PASS |
| `yacr-evidence-wp3-textedit4` | `popup_textedit_pass=1`、`EVIDENCE\PEDIT` 往返、2 张 textEdit 帧 | **INVALID wrong-window** |
| `yacr-evidence-wp3-fix1` | 透明修复后的构建与运行日志（Vulkan/P620、entityCount 4） | 截图缺失（屏保抑制 ScreenCast） |
| `yacr-evidence-recapture1` | 透明修复后复采：生产视口/About/hover/minimize/textEdit 帧 + X11 拒绝行 | **有界 PASS**（A 几何满亮度、无蒙层；C About PASS；D textEdit 编辑器未像素证明；B 箭头抑制非确定性） |

**透明修复后的像素复采与 Observer 重判已执行（有界）**：遮挡 bug 修复**像素验证 PASS**——满亮度
fixture 几何直接透出生产 DOM 视口、无蒙层；About in-DOM 覆盖层 PASS。**未升级**：textEdit 编辑器
内容（host-initiated open 降级 fallback）、光标抑制确定性、Wayland 最小化/还原恢复。
证据边界固定为 **GNOME/Mutter + Quadro P620 + `scale=1`**；其它合成器、分数缩放、多显示器、vendor、
精度、性能均无结论。


## 验证状态与限制（不夸大）

- **Linux 原生 Wayland 真实可运行**；X11/XWayland **显式 Unsupported**（退出码 4，无窗口）；
  Windows 是**已实现但视觉未验证**的宿主（Tier 0 编译门禁 + Wine 拒绝 smoke，见
  [`docs/windows-app.md`](windows-app.md)）；macOS/Android/Web 都是显式 NOT IMPLEMENTED 占位，
  不是可运行 GUI 产品。
- **仅 OPEN/FIT + 原生导航接线**；其余 UI 能力未接。**不是完整 CAD 产品**。
- 运行时语言切换不能重排宿主侧字符串（宿主启动时读一次 `YACR_LOCALE`）。
- **原生 Wayland 有界运行已执行（有界、分区）**：scoped `cargo check --all-targets`、`cargo test --lib`
  （`app-linux` **58/58**、`cad-ui-core` **61/61**）、clippy（0 warning）、wasm32 cfg 隔离、
  `check-architecture.py`、`check-i18n.py`、scoped `fmt`；原生 `--probe`（物理 Vulkan P620、真实 child
  `wl_subsurface` 协议身份）；原生 **CAD `--open`**（4 实体、`current_scene`、`error=None`）+ **9 个合成 DOM 输入**
  （全 `accepted`，相机实际改变——**≠ 真实用户指针像素精度验收**）+ resize 遥测；强制 **overlay** 生命周期
  释放/重建/恢复（**非** toplevel `wl_surface` 替换）；几何修复后 **Observer 有界截图视觉 PASS**；X11 运行回归
  PASS。退出码按 case：原生 baseline 捕获后 SIGTERM **143**，resized 与 X11 **自然 0**。完整命令/退出码/日志/
  边界见 `docs/validation-native-wayland-2026-10-16.md`。**历史证据**：此项的 X11 运行回归属
  CAD-on-top 时期；自 DOM-on-top 起 X11 被显式拒绝（exit 4）。
- **仍未运行 / 未支持**：**DPR>1/分数缩放**（只测 `scale=1`）、GTK **toplevel**
  替换、GPU 设备丢失、vendor 广度、几何/像素精度、性能/60fps、跨平台、打包、vendor/golden。
  **不得**据此宣称“完整原生 Wayland CAD 支持”。
- **DOM-on-top 迁移（WP1–WP4）已落地并提交**（2026-10-22/23）：`cad-ui-core` popup/launch/admission、
  新 crate `cad-desktop-shell`、React UI 归并 `apps/desktop-ui`、Wayland `place_below`、Windows
  child-HWND 宿主。门禁：Tier 0 编译全 rc=0（含 Windows msvc `clippy/check/build`、mingw `build`、
  wasm 空壳 check、i18n 716 键、file-size 851）；Linux 生产 Wayland 有界运行与 Observer 判读见上表与
  [`docs/validation-dom-on-top-production-2026-10-23.md`](validation-dom-on-top-production-2026-10-23.md)。
  **透明修复后的像素复采有界 PASS**（几何满亮度、无蒙层；About 覆盖层 PASS；textEdit 编辑器
  内容未像素证明、光标抑制非确定性）；UI “DOM 测试”是源码结构/纯函数契约，不是运行时挂载。
- **Windows**：实现完成，但视觉/运行时 **NOT RUN**，上限为 Tier 0 编译 + Wine 拒绝合同 smoke；
  真 Windows 上的 spike-1/4/5 **PENDING user real-Windows spike**。Wine 不能验收透明/层序/视觉
  （DComp stub，bug 55432/58921）——任何 Wine 像素结论必须写
  `NOT VERIFIED (Wine limitation, bug 55432/58921)`。
- **DOM-on-top 之后仍未运行 / 未支持**：DPR>1/分数缩放（只测 `scale=1`）、N>1 次 generation 的
  运行期重放（仅代码路径验证）、toplevel 替换、GPU 设备丢失、动态 opaque region/合成开销、
  vendor 广度、几何/像素精度、性能/60fps、跨合成器。
- **最终 Oracle（W3 attempt3）：ACCEPT-WITH-LIMITATIONS（无 blocker）** —— **有界本地验收**（GNOME/Mutter +
  Quadro P620 + `scale=1`），不是完整产品/跨平台/vendor/golden/性能验收；X11 视觉**未**由 observer 判读
  （仅运行证据 PASS）。**该终判属本轮之前的合成方式**（CAD-on-top）；DOM-on-top 的终判见生产验证记录。

- **P6 最终验证已执行（有界、分区）**：前端 43/43 + 构建、`cargo fmt/clippy`、四个 Python 门禁与十个
  Python 契约（含 18 项迁移守卫）、`--locked` 编译/wasm/workspace 测试（1825 passed/0 failed/19 ignored）、
  真实硬件 GPU（Quadro P620 / Vulkan / 真实 GTK XWayland）的 open+渲染 smoke、原生分区生命周期
  （forced unrealize→新 XID + generation 2、自然 exit 0）、外部 WM minimize→restore 与真实 resize；
  完整命令/退出码/日志见 `docs/validation-react-linux-migration-2026-10-16.md`。**历史证据**：其 XWayland
  运行路径自 DOM-on-top 起**已被拒绝**（exit 4），不得作为当前后端结论。
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
