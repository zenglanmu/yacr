# ADR 0023：全客户端统一的 DOM-on-Top 合成模型（wgpu CAD 原生表面在下，DOM/WebView 在上）

- 状态：**Accepted（2026-10-23）**。实施已落地（WP1–WP4 已提交），本文是这批工作的架构决策与
  失效条款权威；证据边界见第 11 节。
- 日期：2026-10-23。
- 相关 ADR：[ADR-0010](0010-react-linux-host-replaces-slint.md)（React/Tauri 宿主；其 `:22-28`
  的“禁止跨 CAD 分区的 DOM 浮层”被本文取代）、[ADR-0011](0011-native-wayland-subsurface.md)
  （app-owned Wayland 子表面；本文把 `place_below` 转为无条件生产语义）、
  [ADR-0021](0021-popup-layer.md)（弹窗层；其第二窗口机制被本文退役，模态围栏保留）、
  [ADR-0018](0018-hatch-family.md)/[ADR-0019](0019-multileader-family.md)/
  [ADR-0020](0020-in-canvas-text-editing.md)（其“React 宿主无法在 CAD 上画 DOM”的 `:27` 前提
  被本文取代，见第 10 节）。
- 需求：`CAD_IMPLEMENTATION_SPEC.md` §21（本次同步修订）。
- 前身证据：`docs/validation-dom-on-top-linux-2026-10-22.md`（opt-in 探针：原生 Wayland 有界
  PASS、XWayland 合成 FAIL）；本轮生产证据：`docs/validation-dom-on-top-production-2026-10-23.md`。
- 影响：合成层序、输入路由、弹窗机制、共享层边界（新增 `cad-desktop-shell`、React UI 归并
  `apps/desktop-ui`）、Windows 宿主形态、平台能力表与退出码注册表。**不**改变 `cad-db` 唯一
  权威、renderer-only wgpu、importer-only acadrust、单 presenter/无整幅回读等既有不变量。

---

## 1. 问题

ADR-0010 把 React/Tauri 宿主定为桌面实现，但保留了当时的产品安排：**原生 CAD 分区（X11 子窗口 /
Wayland 子表面）盖在 WebView 之上**，因此“跨 CAD 矩形的 DOM 浮层”被禁止，下拉/补全必须就地展开、
面板必须停靠、弹窗必须另开原生窗口（ADR-0021）。这条规则带来三个持续成本：

1. **UI 层按平台分叉的风险**：每个新宿主（Windows、未来的 macOS）都要重新回答“谁能盖住谁”，
   弹窗/菜单/浮层机制可能各写一套；
2. **交互与视觉不同向**：输入早已全部走 DOM 单一漏斗（CAD 表面空 input region / 不接收 OS 输入），
   但像素方向相反——DOM 收得到 `contextmenu`，却画不到 CAD 上；
3. **弹窗机制昂贵**：ADR-0021 的第二窗口（独立 `tauri::Window` + 子 webview）承担了 993 行生命周期、
   WM 定位/焦点怪癖、per-window capability 与 `Destroyed` 清门耦合。

2026-10-22 的有界探针证明：在**分区合成**（app-owned 子表面，不是整窗 alpha 叠加）前提下，
原生 Wayland 的 `wl_subsurface.place_below(parent)` 可以让 DOM 正常遮挡、半透明混合并承载跨
chrome/CAD 边界的弹窗（Observer 判读 PASS）；同一探针在 XWayland 上合成 FAIL（透明区域透出
另一应用）。因此问题从“能不能”变成“**怎样把它变成所有客户端唯一、可回归的合成模型，并诚实处理
不支持的平台**”。

> 历史区分（必须保留）：更早的**整窗顶层透明 alpha 全屏叠加**是历史 FAIL（ADR 0006 / SPEC §21
> `:769-772`）。本次 PASS 的是**分区子表面 `place_below`**：CAD 表面仍只占 CAD 矩形，chrome 由
> 同一 toplevel 的 DOM 绘制。二者机制、证据链不同，**不得**用本次结果翻案整窗叠加。

## 2. 决策

### D1 统一合成模型：DOM/WebView 像素与输入双置顶，wgpu CAD 原生子表面在下

**所有客户端只有一个合成方向**：DOM/WebView 在上，CAD 原生表面在下且**不接收 OS 输入**。
客户端 React UI **永不按平台分叉**；平台差异只存在于“如何把原生表面放到 WebView 下方”的机制里。

- **Wayland（生产）**：`yacr_wl_shim_create_child` **无条件**执行
  `wl_subsurface_place_below(subsurface, parent)`；旧的 above-parent 排列与开关已删除，**无双模式**。
  `place_below` 是 double-buffered 的子表面状态，随**父表面 commit** 生效（沿用 ADR-0011 契约）；
  `create_child` 在每个 surface generation 重建时都会重放该调用，因此 unrealize/realize、
  minimize/restore、父表面替换后的重建仍成立（代码路径验证；见第 11 节的 NOT RUN 边界）。
- **Wayland 顶层 alpha 是生产必需，不是探针装饰**：GTK toplevel 必须 `set_app_paintable(true)`、
  在 draw 中清 `set_opaque_region(None)` 并以 `Operator::Source` 透明清屏，否则 `place_below`
  的 CAD 永远不可见。该逻辑在 `gtk_bridge/owner.rs::install_parent_alpha_clear`，**无条件**执行。
- **Windows**：创建 `WS_CHILD | WS_VISIBLE | WS_DISABLED | WS_CLIPSIBLINGS` 的 CAD child HWND，
  用 `SetWindowPos(cad, wry_container, …)` 压到 wry 的 `WRY_WEBVIEW` 容器（WebView2 跨进程子窗口的
  宿主侧父容器）**紧邻之下**，并在每次布局/tick 幂等重排（`ensure_order`）；WebView2 背景 alpha
  必须**恰为 0**（`DefaultBackgroundColor`，wry 会把非 0 alpha 静默钳成 255），窗口 `transparent(true)`
  且 `backgroundColor` alpha = 0，`WS_EX_NOREDIRECTIONBITMAP` 为 best-effort（Tauri 2.11.6 无
  builder flag）。`WS_DISABLED` 是 Wayland 空 input region 的对应防御位。
- **共同不变量**：每条合成路径只有一个 CAD presenter；renderer 不做改动、不做整幅回读；原生表面
  只接收 raw handle（Windows 走 `Win32WindowHandle` → `create_surface_unsafe`）。

> **2026-10-23 增补（署期修订，正文不重写）**：[ADR 0024](0024-dom-canvas-wasm-cad-path.md) 把 D1 的
> 「单一合成方向」修订为**两种已建模像素承载机制**——`surfaceMode: native-below | dom-canvas`，
> 由宿主上报的数据标志选择，React 树不按平台分叉。**D1 的输入漏斗、单 presenter、无整幅回读、
> 原生表面只接 raw handle 四条不变量不变**；`native-below` 的机制（Wayland `place_below` /
> Windows child HWND + WebView2 alpha=0）**原样保留**。`dom-canvas` 只是 X11/XWayland 与浏览器宿主的
> **目标态**，**代码未开始**，当前 X11 仍按 D3 拒绝（退出码 4）。

### D2 单一 DOM 输入漏斗（跨平台同构）

- 输入唯一入口是 `.native-input`（DOM）→ `probe_input` IPC → host 准入门/队列 → `cad-host` 输入管线。
  指针捕获、双击时序、滚轮、右键、坐标空间与 resize 上报全部在这条漏斗内，**不因平台分叉**。
- **CAD 表面永不接收 OS 输入**：Wayland 子表面空 input region **保留**（反转后输入本已落在 DOM，
  空 region 是防御纵深：任何情况下 CAD 都不会吞事件后无人转发）；Windows 用 `WS_DISABLED`。
- **不依赖原生命中穿透**：Windows **不使用** `WM_NCHITTEST`/`HTTRANSPARENT`，也**不**声称具备
  点击穿透能力。调研中的“原生穿透不可行”针对的是“让事件穿过 WebView 到下方 child HWND”；
  本模型不需要该能力（DOM 自己产生命中与事件，原生侧只消费队列）。
- **DOM 覆盖区命中规则（一条产品规则）**：
  - 不透明 chrome 控件：正常命中（默认 `pointer-events`）；
  - 半透明装饰面板：`pointer-events: none` 穿透，面板内交互控件用 `pointer-events: auto` 显式收回。
  理由：视觉即交互——用户看见图纸却点到死区是 UX 陷阱；穿透默认保住 CAD 交互的可预测性。
- **生产透明不变量（来自真实缺陷的教训）**：`.native-input` 的**所有祖先**在 CAD 分区矩形内必须透明
  （`.shell`/`.workspace`/`.workspace__canvas` 不涂不透明底色），chrome 各区自己涂不透明底。
  WP2 之后的手工测试发现 CAD 层不可见，根因正是 DOM 在视口区画了不透明底（见第 11 节与
  `docs/validation-dom-on-top-production-2026-10-23.md`）。

### D3 X11/XWayland：本轮显式 Unsupported（不是“不可能”）

- `apps/app-linux/src/backend.rs` 在 GTK/Tauri 初始化**之前**决议后端；解析结果为 X11（显式
  `GDK_BACKEND=x11` 或 `Auto` 推出）即拒绝：stderr 稳定标记行 `yacr: unsupported-backend: x11`
  + i18n 诊断（`startup.unsupported_backend`/`.detail`）+ **退出码 4**。**无静默回退**：绝不落到
  X11 继续跑旧排列。
- 旧 X11 native-child 路径**编译保留、测试保留、运行时不可达**，模块/测试注释带
  `unreachable_since_dom_on_top`；删除留作独立清理（见 handoff 待办），本轮不混入。
- **为什么是 Unsupported 而不是已失败定论**：2026-10-22 探针在 XWayland 上建立了 sibling 顺序但
  **合成 FAIL**（透明区域透出另一 CAD 应用），具体 clipping/redirect 机制**未闭环**；因此只能
  记录“本轮不支持”，不能写“X11 不可能”。X11 的 web+canvas 回退路线**显式延后**，需要新的有界
  设计与证据，不是 CSS/reorder 可修复的承诺。
- **退出码分类以标记行为准**：`scripts/collect-evidence.sh` 的 `UNSUPPORTED_BACKEND` 分类首要依据
  stderr 精确标记行，退出码 4 仅辅助（证据驱动可自定退出码）。完整注册表见 `docs/build.md` 与本文
  §9。

### D4 弹窗：全部迁入同一 React 树的 DOM 覆盖层

- **锚定浮层（右键菜单、tooltip、跨 chrome/CAD 边界的下拉）= in-DOM 覆盖层**（portal 到顶层 overlay）。
  ADR-0021 D3 的原生 `GtkMenu` 计划随之退役；菜单文案复用既有 i18n 目录（前端直接 import
  `cad-ui-core/i18n`），键盘导航/外点关闭/Esc 在 React 内实现。
- **居中模态（about/textEdit/options）= 同一 DOM 覆盖层的模态组件**。ADR-0021 D1 的“第二个 Tauri
  窗口 + 子 webview”**退役**（其前提“DOM 不可覆盖 CAD”已被探针证伪）；单 webview、确定性定位、
  React 焦点管理，天然跨 chrome/CAD 边界。`CadPopup::label` 仅保留为冻结的 wire 字段，不再用于
  寻址窗口，也没有第二处写入。
- **host 侧模态围栏完整保留（安全边界，与渲染机制无关）**：`PopupKind` allowlist、
  `ShellState::popup`、`admit_command` 与 `queue_cad_input` 的 `popup.modal_open` 二次拒绝、
  恢复时相位重校验、与 UI 侧 `textFocus`/`popupOpen` **独立双开关**语义——全部不变。
- **清除触发器从“窗口 Destroyed”改为四触发（缺一即卡门向量）**：
  1. 显式 `cad_close_popup`（含 Esc/关闭按钮）；
  2. webview reload/navigation → host 复位 `modal_open`；
  3. **renderer 进程崩溃/渲染进程失败** → host 复位 `modal_open` 并记 `HostErrorKind::Renderer`
     诊断（WebKitGTK `web-process-crashed`、WebView2 `ProcessFailed`；崩溃既无 navigation 也无
     close，是旧模型天然覆盖、新模型必须显式补的路径；crash ≠ 空成功）；
  4. 进程退出自然清理。
  **Esc/关闭按钮可达性**：关闭 affordance（Esc 监听 + 关闭按钮）挂在最小 chrome 壳上、位于正文
  error-boundary 之外——正文渲染失败时仍必须可关（旧模型 WM 关窗的等价物）。
- **wire 只做加法**：`CadPopup`/`cad_status.popup`/`PopupAck` 保留；`cad_open_popup` ack 追加
  **可选** `payload`（`about` 构建事实、`textEdit` 宿主派生文本、`options` 无 payload、
  `existing:true` 无 payload），无 payload 时字段整体省略；payload 由宿主派生，调用方仍不能传入
  内容/label/URL/脚本。

### D5 共享层契约：纯逻辑 / Tauri 壳 / 唯一 React 树 / 平台缝

- **`cad-ui-core`（扩展，不新增依赖边）**：wire protocol、逻辑布局、CAD 会话、宿主 admission/error
  状态（shell）、i18n、**popup 纯状态机**（`PopupKind`/`PopupAck`/`CadPopup`/allowlist/四触发状态）、
  launch options 解析、typed admission。必须保持 UI 工具包与 GPU 无关。
- **`cad-desktop-shell`（新 crate，唯一允许依赖 `tauri` 的共享层）**：`#[tauri::command]` 薄壳与注册、
  主窗口/子 webview 构建（init scripts、`start_maximized` 策略、label 常量）、popup 命令到
  `cad-ui-core::popup` 状态机的接线、与工具包无关的 `threading`（主线程 run-and-wait）、通用证据驱动
  骨架、参数化平台实现（`ReactFiles`/`ReactSurface`/`ReactConfigStore`，chooser 由 app 注入）。
  **禁止** gtk/gdk/gdkx11/ashpd/wgpu/cad-render-wgpu/cad-engine/slint/iced/acadrust（脚本强制）；
  位于 `cad-ui-core` 之上、`app-*` 之下；非 Linux/Windows 目标编译为空壳（保 wasm 门禁）。
- **`apps/desktop-ui` 是唯一桌面 React UI 包**：单 React 树、`.native-input` 单漏斗、in-DOM 弹窗/
  菜单；两个宿主的 `tauri.conf.json` 都指向 `../desktop-ui/dist`。
- **`app-*` 只保留平台缝**：GTK/overlay/owner、Wayland C shim、Win32 child HWND、portal/rfd 文件
  选择器、known-folders、redraw pump/tick（GTK `queue_draw` 与 Win32 `InvalidateRect` 机制不同，
  各自薄封装）。**可移植逻辑不得住在 `app-*`**——这是 WP3 的放置纠正：payload 组装、`\P` 映射、
  gate 逻辑、threading 协议都在共享层，`app-linux` 只剩 `!Send` 会话读取 + WebKitGTK 崩溃信号接线，
  `app-windows` 只需 5 个薄适配。
- **锁序不变量**：两个 `Arc<Mutex<…>>`（`Shared` + `RenderTelemetry`）固定获取顺序
  **Shared → RenderTelemetry**，严禁反向；`ShellTelemetry` 实现永不触碰 `Shared`。

### D6 Windows 宿主形态与验收上限

- 形态见 D1；WebView2 运行时缺失 = **显式 Unsupported**：先于任何窗口检查，标记行
  `yacr: unsupported-runtime: webview2` + i18n 诊断 `startup.webview2_missing`(+`.detail`) +
  **退出码 5**，绝不空白窗口、绝不静默回退。检测证明“存在”不证明“可用”：损坏运行时会在窗口
  构建处显式失败（exit 1 + 明文错误）。
- **Runtime 版本敏感**：透明是逐版本回归能力（145.x #5481 已修；154.0.4258.62 #5752 open，报告环境
  正是 Tauri v2/wry 0.55.1）。因此**任何透明结论只能限定“该 Runtime 版本 + 该 GPU/驱动 + 该
  Windows 版本 + 该缩放”**；Fixed Version 钉定与否是 spike-5 的产出，本文不预判。
- **Wine 降级为 smoke/诊断级**：DComp 是 stub（bug 58921），目标形态（透明 +
  `NOREDIRECTIONBITMAP`）落在 bug 55432 的黑屏分支 ⇒ Wine **无法**验收透明、层序或视觉。
  Wine 只承担：启动不崩、WebView2 检测与拒绝诊断正确、`fixme:dcomp` 等已知缺陷特征记录、故障域
  切分；**任何 Wine 像素/截图结论必须写 `NOT VERIFIED (Wine limitation, bug 55432/58921)`**。
- **验收上限（写死）**：本轮 Windows = **完整实现 + Tier 0 编译门禁 + Wine 拒绝合同 smoke**；
  视觉/运行时验收 = **用户在真 Windows 自测 spike-1/4/5**（判据与证据格式见
  `docs/windows-app.md`）。闭环前一律写 “PENDING user real-Windows spike”，**不得**写
  “Windows 已验收”。spike-1 失败时按应急回退（tiling 分块 webview）与用户决策，不擅自分片重构。

## 3. 备选方案与否决理由

- **保持 CAD-on-top + 原生弹窗（ADR-0021 原样）**：需要每平台各写弹窗/菜单机制，且 DOM 永远无法
  在 CAD 上绘制（预览、浮层、拖拽提示都受限）。**否决**：成本随宿主数量线性增长。
- **整窗顶层透明 alpha 叠加（历史方案）**：历史 FAIL，且把 chrome 与 CAD 放在同一全局 alpha 层，
  合成器差异大。**否决**：本次成功的是分区子表面，不是整窗叠加。
- **X11 本轮强行跟随**：探针合成 FAIL、机制未闭环；强行上线等于把“另一应用透出”当交付。**否决**：
  显式 Unsupported + 退出码 4，保留后续 web+canvas 路线。
- **Windows 透明不可行 → texture→canvas/整幅回读**：破坏单 presenter/无整幅回读不变量，且像素
  延迟与 4K 带宽不可接受。**否决**；应急回退是 tiling（多 webview 分片），代价是共享 UI 运行模型
  分片，须用户决策。
- **原生点击穿透（`WM_NCHITTEST` 等）**：不需要（D2），也没有官方稳定 API。**不作为契约**，仅
  记录为未来可选优化。

## 4. 后果

**正面**：

- React UI 一份代码覆盖 Linux/Windows，浮层、弹窗、菜单不再按平台分叉；
- 弹窗生命周期从“第二窗口 + `Destroyed` 清门”收缩到共享状态机 + 四触发，wire 只做加法；
- 平台缝更薄：`app-windows` 复用共享命令/状态/弹窗/threading，仅实现 HWND 与运行时检测；
- 合成方向与输入方向一致，模型更诚实。

**代价与风险**：

- **顶层 alpha 合成开销**：透明窗口 + 无 opaque region 可能丧失合成器不透明优化；性能本轮
  **NOT VERIFIED**，后续可按 DOM 覆盖情况动态设置 opaque region（不在本轮）。
- **生产透明脆弱性**：任何祖先节点加不透明底色都会再次遮住 CAD；已有前端回归测试钉住
  `.shell`/`.workspace`/`.workspace__canvas`，但新增布局仍需遵守规则。
- **X11 用户被显式拒绝**（退出码 4），直到 web+canvas 路线立项。
- **Windows 视觉未闭环**：spike-1/4/5 未跑，z-order/DPI/ProcessFailed 活体/选择器/出帧均 NOT RUN。
- **证据有界**：Linux 仅 GNOME/Mutter + Quadro P620 + `scale=1`；其它合成器、分数缩放、多显示器、
  vendor、精度、性能均无结论。

## 5. 非目标与显式延期

- macOS / Android / iOS：不在本轮；Android/Web 仍是显式 NOT IMPLEMENTED 占位。
- **X11/XWayland 的 web+canvas 回退路线**：显式延后，需独立设计/证据。
  - **2026-10-23 增补**：该路线已由 [ADR 0024](0024-dom-canvas-wasm-cad-path.md) **立项**（wasm/WebGL2
    DOM-canvas 路径，分阶段；**代码未开始**，X11 当前仍拒绝）。本节其余延期条款不变——尤其
    **RenderTarget/HostTexture 平台合成 ADR 仍独立延期，不被 ADR 0024 覆盖**。
- **平台合成 ADR（RenderTarget/HostTexture token 所有权）——本文显式延期，未覆盖。**
  `docs/handoff.md` 的“已定义，但尚需设计审查”条目（RenderTarget/HostTexture 目前只是自有数字
  token；共享 Device/Queue 的拥有者、token 生命周期与安全访问方式须在**平台合成 ADR** 中固定）
  属于 **renderer 资源契约**，与本文的“层序/合成方向”正交。本文**不**定义 token 所有权，也**不**
  凭 token 猜测 GPU 对象；该待办**原样保留**在 handoff，另立 ADR 或并入 renderer 文档。
- 动态 opaque region、性能优化、tiling 分片抽象：未立项（YAGNI；等 spike-1 失败证据再决策）。

## 6. 对既有 ADR / SPEC 的影响（失效条款清单）

| 文档 | 位置 | 处置 |
|---|---|---|
| ADR 0010 | `:22-28`“禁止跨 CAD 分区的 DOM 浮层”、`:92-105` 2026-10-22 增补 | **产品选择已被本文取代**；历史决策与验收事实保留，增补补 2026-10-23 失效注记 |
| ADR 0011 | 决策 4（空 input region）、5（父 commit 语义）、验证状态 | 增补：`place_below` 无条件化；generation 重放；探针/生产证据引用 |
| ADR 0017 | `:602`、`:870` 的“DOM 不得浮于其上”前提 | 前提失效（增补注记）；标注预览仍缺后端 overlay 分支，限制以新理由保留 |
| ADR 0018 | `:6-8`“React 宿主无法绘制 overlay” | 前提失效（增补注记）；填充预览未实现的事实不变 |
| ADR 0019 | `:13`、`:63` 的 `:27` 前提 | 前提失效（增补注记）；就地编辑/DOM 路线的历史决策不重写 |
| ADR 0020 | `:10`、`:61`、`:99` 的“DOM 浮层不可行” | 前提失效（增补注记）；A/B/C 入口决策保持 |
| ADR 0021 | 约束 1/3/4/6、D1、D3、D6 | D1/D3/D6 与约束 1/3/4/6 被本文取代；**D2 围栏保留**并补四触发；真实 OS 键盘未验证的诚实边界保留 |
| SPEC §21 | `:738-777` | 同步修订：层序/事件/弹窗/平台状态/证据行；§1–§20 不动 |

## 7. 迁移与退役

- `cad-ui-core` 新增 popup/launch/admission 模块；新 crate `cad-desktop-shell`；React UI 从
  `apps/app-linux/ui` 移到 `apps/desktop-ui`（工作流、门禁脚本、vite 别名、tauri conf 同批更新）。
- `apps/app-linux/src/host/popup.rs` 的窗口创建/`Destroyed`/窗口证据阶段删除；`PopupRoot.jsx` 退役，
  `PopupOverlay.jsx` 接管。
- `dom_on_top_probe.rs` 删除；`YACR_PROBE_DOM_ON_TOP` 开关与 `window.__YACR_PROBE_DOM_ON_TOP__`
  注入**不再存在**。探针 UI 改造成**回归夹具** `apps/desktop-ui/src/stacking-regression/`，触发契约
  三件套：`YACR_PROBE_STACKING=1`（严格 `"1"`）+ init-script 注入
  `window.__YACR_STACKING_REGRESSION__ === true` + **真实 `--open` 会话**；`--probe` 与
  `YACR_PROBE_ONLY` 同现启动即拒绝。该夹具**不是产品开关**，缺一即诊断拒绝。

## 8. 证据与验证边界

- **Linux 生产（真实 GPU）**：`docs/validation-dom-on-top-production-2026-10-23.md`。运行环境
  GNOME/Mutter + Wayland + Quadro P620/Vulkan + `scale=1`；证据目录
  `/tmp/opencode/yacr-evidence-wp2-final6`、`wp3-final1`、`wp3-textedit4`、`wp3-fix1`、
  `yacr-evidence-recapture1`（本地、不上传）。Observer 已判：WP2 四条判据 PASS（带 caveat）；
  WP3 About 帧 PASS；textEdit 帧 **INVALID wrong-window**；**透明修复后的复采有界 PASS**
  （A 几何满亮度/无蒙层、C About PASS），但 textEdit 编辑器内容 **NOT pixel-proven**（host-initiated
  open 降级 fallback）、B 箭头抑制 **observed, non-deterministic / NOT VERIFIED deterministic**。
- **X11**：仅拒绝证据（标记行 + 退出码 4 + 无窗口/无截图）；X11 运行/视觉此后一律 NOT RUN。
- **Windows**：Tier 0 编译门禁全部 rc=0；Wine 三次拒绝合同 smoke rc=5；**视觉/运行时全部 NOT RUN**，
  上限见 D6 与 `docs/windows-app.md`。
- **回归夹具**：结构性契约 + 前端测试是**源码结构/纯函数测试**，不是运行时 DOM 挂载或像素证据；
  合成帧仅是像素，不等于视觉正确性/精度/vendor 结论。

## 9. 退出码注册表（与 `docs/build.md` 同源）

| 码 | 含义 | 产点 |
|---|---|---|
| 0 | 成功（含正常退出、生命周期驱动自然结束） | `entry.rs` 默认返回 |
| 1 | 通用失败（Tauri 构建失败、启动合同拒绝；Windows 窗口构建失败） | `app-linux/entry.rs`、`app-windows/entry.rs` |
| 2 | 无效/不可用的显示后端策略 | `app-linux/entry.rs`（`BackendRefusal::Invalid`） |
| 3 | postload-pan 证据驱动失败 | `app-linux/src/host/postload.rs` |
| 4 | **不支持的后端（X11/XWayland）** | `app-linux/src/backend.rs`（标记行 + 码 4） |
| 5 | **不支持的运行时（WebView2 缺失）** | `app-windows/src/support.rs`（标记行 + 码 5） |

分类纪律：证据脚本以**稳定标记行**为第一依据，码值仅辅助；不得以退出码 0 或非空截图单独证明
视觉正确。

## 10. 被取代前提的逐条说明（不抹历史）

本文**不删除**任何历史决策与验收记录；只在原文件加署期增补。要点：

- ADR 0010 `:24-28` 的“CAD 在 WebView 之上、禁止 DOM 浮层”是 2026-10-16 的产品选择，已被
  D1/D4 取代；其“顶层透明全屏叠加是历史 FAIL”的事实仍然成立，且与本次分区 `place_below` PASS
  是不同证据链。
- ADR 0018 `:6-8`、ADR 0019 `:13`/`:63`、ADR 0020 `:10`/`:61`/`:99`、ADR 0017 `:602`/`:870`
  的“React 宿主无法绘制 overlay / DOM 不得浮于 CAD”是当时几何前提的推论；前提已由 D1 改变，
  但**它们各自的未实现事实（标注预览、填充预览、就地编辑入口选择）不因此自动完成**。
- ADR 0021 的约束 1/3/4/6（像素不可过、popover 非 xdg_popup、GTK 模态不覆盖原生表面、模态必须
  在漏斗里强制）与 D1/D3/D6：其中约束 1 的**像素方向**被反转；约束 3/4 的 popover 讨论随 D3
  退役而失去对象；**约束 6 的结论（模态必须由我们在唯一漏斗里强制）仍然有效**，由 D4 的 host
  围栏 + 四触发继续实现。D2 原样保留。D5 冻结帧方案仍然不采用。
- 真实 OS 键盘从未验证过任何输入路径（ADR-0021 的原话）**继续成立**；本轮一切“输入”证据仍为
  合成 DOM 事件。
