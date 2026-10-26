# 架构与公共契约

需求权威：`CAD_IMPLEMENTATION_SPEC.md` v2.0（含 §21 桌面宿主增补）。
当前桌面 UI 决策见 [`docs/adr/0010-react-linux-host-replaces-slint.md`](adr/0010-react-linux-host-replaces-slint.md)；
原生 Wayland 后端见 [`docs/adr/0011-native-wayland-subsurface.md`](adr/0011-native-wayland-subsurface.md)；
跨宿主合成模型（DOM-on-top、共享层、弹窗层）见
[`docs/adr/0023-dom-on-top-cross-host-composition.md`](adr/0023-dom-on-top-cross-host-composition.md)。

> **迁移状态（2026-10-16，2026-10-23 增补）**：桌面 UI 已由 Slint 迁移为 **React + Tauri 原生宿主**
> （`apps/app-linux`，二进制 `yacr`）；`cad-ui-slint` 与 Slint 依赖已删除。2026-10-23 起合成方向为
> **DOM/WebView 在上、CAD 原生子表面在下**（ADR 0023），React UI 归并到唯一包 `apps/desktop-ui`，
> Tauri 薄壳下沉到新共享层 `cad-desktop-shell`；**X11/XWayland 自 2026-10-24 起走 DOM-canvas**
> （wasm/WebGL2 CAD 会话画进 DOM `<canvas>`，ADR 0024；仅当 WebGL2 不可用才输出
> `yacr: unsupported-runtime: webgl2` + 退出码 6，旧「`unsupported-backend: x11` + 退出码 4」为历史合同），
> Windows 宿主已实现但视觉未验证。本文只描述**当前树中真实存在**的 crate 与契约。旧 Slint 合成
> （ADR 0002/0003）与实验宿主（ADR 0006）保留为历史记录，见各 ADR 顶部横幅；旧宿主验证证据见
> `docs/validation*.md`（历史，含 NOT RUN 边界）。

数据流：UI/CLI → Application/权限 → Command/DrawingTransaction → DB transaction →
ChangeSet → 查询/历史/依赖失效 → Representation → SceneDelta → Renderer → RenderHost。

已接线：导入→数据库（含 `OPEN` 经真实文件打开漏斗 + XDG portal 选择器）、显示表示→场景→wgpu、
绘图事务→撤销/重做、依赖失效、查询分页、**React/Tauri 宿主 + 原生 GTK/wgpu CAD 分区**
（`apps/app-linux`，见下）。

**未支持/未接线项按当前契约逐项查证，不再笼统断言整类 NotImplemented**：

- 宿主侧**保存/出图/导出**在当前 Linux 宿主显式 `Unsupported`（`apps/app-linux/src/platform.rs`）；
- `SWITCHBACKEND` 经本宿主不可用（显式拒绝，非静默）；
- **3D 观察与 ACIS 子集离散器在核心存在**（`cad-render-wgpu`、`cad-kernel-adapter`）；
  ACIS **已在导入期接线**：`3DSOLID`/`BODY`/`REGION`/`SURFACE` 离散为中性网格后正常显示
  （恒 `Partial`、源 ACIS 不保留，见 `docs/compatibility.md`「ACIS 实体显示状态」）；
  3D 观察**可达且有 GUI 控件**：视图标签的 3D 面板（2D/3D、投影、七种标准视图）+
  命令行 `SWITCH2D3D`、3D 下主键拖动 orbit（`docs/view-3d.md`），并非核心 NotImplemented；
  边界见 `docs/compatibility.md`、
  `docs/view-3d.md`、`docs/kernel-acis.md`；
- ~~当前 GUI 仅 `OPEN`/`FIT` + 原生导航接线；其余命令经真实漏斗显式拒绝，不空成功。~~
  **2026-10-26 更正（原句停在 2026-10-23 DOM-on-top 之前，已过期）**：绘制/编辑、测量、图层与属性
  读写命令均经真实漏斗接线，逐条边界以 `docs/compatibility.md` 为准；宿主侧**保存/出图/导出**在
  Linux 宿主仍显式 `Unsupported`。payload 缺失、Viewer 模式、未支持几何一律**显式拒绝**，绝不空成功。

> 2026-10-05 变更：用户批注子系统整体移除（提交 `0120d09`），`cad-annotations` crate
> 不再存在，F07/F08/F09 退出产品范围。2026-10-16 变更：Slint 桌面 UI 退役，见 ADR 0010。

| 层 | 包与关键契约 |
|---|---|
| 领域 | cad-domain：强类型 ID、f64 几何、单位、容差、来源/完整性、TaskStamp |
| 权威数据库 | cad-db：DrawingDatabase/Builder、DrawingTransaction、ChangeSet |
| 数据一致性 | cad-dependencies：DependencyIndex/Invalidation；cad-history：UndoRecord/RecoveryJournal |
| 几何 | cad-geometry：GeometryEngine；cad-kernel-adapter：SolidTessellator/SAT/SAB 隔离 |
| 导入资源 | cad-import-acadrust：Importer/ImportReport；cad-proxy：ProxyRecordDecoder/ProxyPlayer；cad-resources：ResourceResolver |
| 显示 | cad-representation：ProviderRegistry/DisplayRepresentation；cad-spatial：SpatialIndex/PickHit |
| 场景/GPU | cad-scene：CacheKey/SceneDelta/SceneCache；cad-render-wgpu：Renderer/RenderTarget（唯一依赖 wgpu） |
| 渲染会话 | cad-engine：UI 无关的 RenderSessionHandle、会话状态与 typed render/session protocol；直接依赖 cad-app/cad-db/cad-domain/cad-render-wgpu/cad-representation |
| 工作能力 | cad-measure：MeasurementEngine/SnapCandidate |
| 应用 | cad-app：Document/SessionState/Viewport/Workspace、CommandHandler/Tool |
| 查询 | cad-query：分页查询/Mixed/Unset |
| 共享 UI 层 | **cad-ui-core：平台无关的 wire protocol、逻辑布局（layout）、CAD 会话（session，`CadSession`/`CadStatus`/44-command 表）、宿主 admission/error 状态（shell，`ShellState`）、**弹窗纯状态机（popup，`PopupKind`/`PopupAck`/`CadPopup`/allowlist/四触发）**、launch options、i18n 目录；UI 框架与 GPU 无关 |
| 桌面壳共享层 | **cad-desktop-shell：唯一允许依赖 `tauri` 的共享层**——`#[tauri::command]` 薄壳与注册、主窗口/子 webview 构建（init scripts、`start_maximized`）、popup 命令到 `cad-ui-core::popup` 的接线、工具包无关 `threading`、通用证据驱动骨架、参数化平台实现（`ReactFiles`/`ReactSurface`/`ReactConfigStore`）；**禁止** gtk/gdk/gdkx11/ashpd/wgpu/cad-render-wgpu/cad-engine/slint/iced/acadrust |
| 宿主编排 | cad-host：HostCore（命令路由、打开/保存/出图管线、状态推送、导航/绘制 sink）；**不依赖任何 UI 工具包，也不依赖 slint/wgpu/web-sys/Android 类型** |
| 宿主平台契约 | cad-platform：PlatformFiles/ConfigStore/SurfaceSource/FontLoader/Persistence/TaskExecutor/Clipboard/HostLifecycle/RenderHost |
| 宿主组合根 | apps：app-linux（GTK/Wayland 平台缝 + 真实 Linux GUI）、app-windows（Win32/WebView2 平台缝，已实现、视觉未验证）、app-macos/app-android/app-web（显式 NOT IMPLEMENTED 占位）；**唯一桌面 React UI：`apps/desktop-ui`** |
| 自动化 | cad-diagnostics：统计/脱敏报告；cad-cli-tools：共用 Application 命令路径 |

## 必须保持的边界

- 底图只能由 Builder 导入；领域修改只能通过事务正式提交。会话导航不使文档变脏。
- Document 共享不可变底图；Viewport 保存独立相机，不复制数据库。
- GPU/内核/UI 工具包/平台类型不得进入 domain/db；第三方类型不随意 re-export。
- `cad-engine` 是 UI 无关的渲染会话协调层：直接依赖 `cad-app`、`cad-db`、`cad-domain`、
  `cad-render-wgpu` 与 `cad-representation`；不依赖 `cad-ui-core`/`cad-host`/任何 UI 工具包，
  也不直接依赖 wgpu；wgpu 类型仅由 renderer 通过其 API/re-export 提供。`cad-app` 不依赖 `cad-engine`。
- `cad-ui-core` 位于 UI 无关的应用/宿主契约**之上**（它依赖 `cad-host`，会话包装 `HostCore`）、
  具体宿主**之下**；必须保持 UI 工具包与 GPU 无关（无 slint/wgpu/tauri/gtk/portal/平台类型），
  由 `scripts/check-architecture.py` 强制。
- **`cad-desktop-shell` 是唯一允许依赖 `tauri` 的共享层**，位于 `cad-ui-core` 之上、`app-*` 之下；
  非 Linux/Windows 目标编译为空壳（保住 wasm 门禁）。它不依赖 gtk/gdk/gdkx11/ashpd/wgpu/
  cad-render-wgpu/cad-engine/slint/iced/acadrust（脚本强制）。
- **`apps/desktop-ui` 是唯一桌面 React UI 包**：单 React 树、`.native-input` 单一输入漏斗、
  in-DOM 弹窗/菜单；两个宿主的 `tauri.conf.json` 都指向 `../desktop-ui/dist`，**不得**再复制第二套 UI。
- **`app-*` 只保留平台缝**：GTK/overlay/owner、Wayland C shim、Win32 child HWND、portal/rfd 文件选择器、
  known-folders、redraw pump/tick（GTK `queue_draw` 与 Win32 `InvalidateRect` 机制不同，各自薄封装）。
  **可移植逻辑不得住在 `app-*`**——payload 组装、`\P` 映射、gate 逻辑、threading 协议都在共享层。
- **锁序不变量**：两个 `Arc<Mutex<…>>`（`Shared` + `RenderTelemetry`）固定获取顺序
  **Shared → RenderTelemetry**，严禁反向；`ShellTelemetry` 实现永不触碰 `Shared`。
- `cad-host` 不直接依赖 `slint`/`wgpu`/`web-sys`/Android 类型；平台 I/O 一律经 `cad-platform` 的
  `PlatformFiles`/`ConfigStore`/`SurfaceSource` 契约（事件式、无 futures、无 `Send`），
  宿主特有的路径/对话框/表面度量只存在于 `app-*`。Slint 桌面退役后 `cad-host` 不再依赖任何 UI crate。
- **renderer-only wgpu**：`cad-render-wgpu` 是唯一依赖 wgpu 的 crate。**importer-only acadrust**：
  `cad-import-acadrust` 是唯一依赖 acadrust 的 crate；**禁止 Cargo patch**。
- ChangeSet revision 不连续时重建快照；元数据不默认引发曲线重新离散。
- 测量结果锚点由原始 handle + instance path + subelement + 坐标回退构成；GPU 索引不是身份。
- TaskStamp 不匹配拒绝发布；取消不可中断库调用时至少阻止结果进入当前会话。
- DB revision、渲染设备 generation、任务 generation 分开处理。
- 注册冲突须明确排序/拒绝；当前注册表并未实现扩展分发。

`scripts/check-architecture.py` 校验当前依赖图、循环和核心隔离（含全 workspace 禁止
`slint`/`slint-build`/`iced`，覆盖 build/dev 与 target 表、workspace 继承别名）。
traits 是契约草案（0.1），实现前可通过 ADR 修正不足，不代表已稳定公共 API。

## 桌面宿主：React/Tauri + DOM-on-top 合成

决策见 ADR 0010（宿主）、ADR 0011（原生 Wayland 后端）与 ADR 0023（跨宿主合成模型）；
使用说明见 `docs/react-linux-app.md`；需求增补见 `CAD_IMPLEMENTATION_SPEC.md` §21。

- **组合（唯一方向）**：DOM/WebView **在上**，GTK `GtkOverlay` 内的中央原生 CAD 分区**在下**。
  - Wayland（生产）：app-owned `wl_subsurface` **无条件** `place_below(parent)`；GTK toplevel 必须
    app-paintable + 空 opaque region + 透明清屏，否则 CAD 不可见。`place_below` 是父级相对的双缓冲
    状态，随**父表面 commit** 生效，并在每个 surface generation 重建时重放。
  - Windows：`WS_CHILD | WS_VISIBLE | WS_DISABLED | WS_CLIPSIBLINGS` 的 CAD child HWND 用
    `SetWindowPos` 压到 wry 容器紧邻之下（每次布局/tick 幂等重排）；WebView2 背景 alpha 必须恰为 0。
  - X11/XWayland：**DOM-canvas 载体**（ADR 0024）——wasm/WebGL2 CAD 会话画进 WebView 内
    `<canvas>`（`surfaceMode: dom-canvas`），native 壳**不**建 CAD 会话/原生表面；WebGL2 不可用时
    显式拒绝（标记 `yacr: unsupported-runtime: webgl2` + 退出码 6）。旧 X child 路径编译/测试保留
    但运行时不可达；`yacr: unsupported-backend: x11` + 退出码 4 为**历史**合同。
- **历史区分**：整窗顶层透明 alpha 全屏叠加是历史 FAIL（ADR 0006）；本次成功的是**分区子表面
  `place_below`**，二者证据链不同。
- **单一输入漏斗**：CAD 表面**从不接收 OS 输入**。`.native-input`（DOM）→ `probe_input` → host
  准入门/队列 → `cad-host` 输入管线；指针捕获、双击时序、滚轮、右键、坐标空间与 resize 上报全部
  在这条漏斗内。Wayland 空 input region 与 Windows `WS_DISABLED` 是**防御纵深**。**不依赖原生
  命中原生穿透**。
- **弹窗层**：居中模态与锚定浮层都是**同一 React 树的 DOM 覆盖层**；host 侧模态围栏
  （`ShellState.popup` + `PopupKind` allowlist + 二次拒绝 + 四触发清除）是安全边界，与渲染机制无关。
- **合成/布局契约**：`.native-input` CAD 分区始终挂载并上报 `getBoundingClientRect()`（CSS px + DPR），
  宿主 `LayoutTracker` 据此更新 surface metrics——这是滚轮缩放锚定光标的前提。**生产透明不变量**：
  `.shell`/`.workspace`/`.workspace__canvas` 在 CAD 矩形内不得涂不透明底色（有结构性回归测试）。
- **平台范围**（2026-10-26 更正）：**原生 Wayland** 是唯一生产 Linux 后端；**X11/XWayland 走
  DOM-canvas**（ADR 0024，wasm/WebGL2 在 DOM `<canvas>`，WebGL2 缺失才拒绝 + 退出码 6）；Windows 宿主
  已实现（Tier 0 编译 + Wine 拒绝 smoke，视觉 **PENDING user real-Windows spike**）；macOS/Android 是
  显式 NOT IMPLEMENTED 占位；Web 的 wasm CAD 会话已实现，其 host target 仍是 NOT IMPLEMENTED 空壳。
- **支持范围**：命令面**不以 OPEN/FIT 为界**（见上一条更正），以 `docs/compatibility.md` 与宿主能力
  布尔为准；未接线的 UI 控件显式禁用并标注原因；原生导航按实际源码能力支持（缩放/平移/fit）。
  **不新增投机 IPC**；需要 payload 的命令经真实漏斗显式拒绝，绝不空成功。**不是完整 CAD 产品**。
- **不变量不变**：单 presenter per drawable、renderer-only wgpu、db 唯一权威、无整幅回读；
  overlay 后缀**不再截断**（2026-10-25 更正）：新输入未提交时呈现完整 resident 场景并以
  `FrameCompleteness::PendingOverlayInputs` 显式标记（非静默）；实体 flags 经独立 catch-up 增量。

## 历史（已退役 / 被取代）

- ADR 0002（Slint/wgpu 合成）、ADR 0003（浏览器 Slint/wgpu）描述**旧的 Slint 基线**；
  桌面部分被 ADR 0010 取代，Web/Android UI 已退役（见 ADR 0010 与 `docs/handoff.md`）。
- ADR 0006（React + Tauri 实验）是本次迁移的**前身实验**，其“实验/未删旧代码”描述已过时。
- `docs/code-structure.md` 中描述 Slint 渲染桥文件职责的段落属历史；当前宿主结构见本文与
  `docs/react-linux-app.md`。
