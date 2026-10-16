# 架构与公共契约

需求权威：`CAD_IMPLEMENTATION_SPEC.md` v2.0（含 §21 桌面宿主增补）。
当前桌面 UI 决策见 [`docs/adr/0010-react-linux-host-replaces-slint.md`](adr/0010-react-linux-host-replaces-slint.md)；
原生 Wayland 后端见 [`docs/adr/0011-native-wayland-subsurface.md`](adr/0011-native-wayland-subsurface.md)。

> **迁移状态（2026-10-16）**：桌面 UI 已由 Slint 迁移为 **React + Tauri 原生宿主**
> （`apps/app-linux`，二进制 `yacr`）；`cad-ui-slint` 与 Slint 依赖已删除。本文只描述**当前树中
> 真实存在**的 crate 与契约。旧 Slint 合成（ADR 0002/0003）与实验宿主（ADR 0006）保留为历史
> 记录，见各 ADR 顶部横幅；旧宿主验证证据见 `docs/validation*.md`（历史，含 NOT RUN 边界）。

数据流：UI/CLI → Application/权限 → Command/DrawingTransaction → DB transaction →
ChangeSet → 查询/历史/依赖失效 → Representation → SceneDelta → Renderer → RenderHost。

已接线：导入→数据库（含 `OPEN` 经真实文件打开漏斗 + XDG portal 选择器）、显示表示→场景→wgpu、
绘图事务→撤销/重做、依赖失效、查询分页、**React/Tauri 宿主 + 原生 GTK/wgpu CAD 分区**
（`apps/app-linux`，见下）。

**未支持/未接线项按当前契约逐项查证，不再笼统断言整类 NotImplemented**：

- 宿主侧**保存/出图/导出**在当前 Linux 宿主显式 `Unsupported`（`apps/app-linux/src/platform.rs`）；
- `SWITCHBACKEND` 经本宿主不可用（显式拒绝，非静默）；
- **3D 观察与 ACIS 子集离散在核心/渲染器已有实现**（`cad-render-wgpu`、`cad-kernel-adapter`），
  属**当前 GUI 未接线**（控件显式禁用），并非核心 NotImplemented；边界见 `docs/compatibility.md`、
  `docs/view-3d.md`、`docs/kernel-acis.md`；
- 当前 GUI 仅 `OPEN`/`FIT` + 原生导航接线；其余命令经真实漏斗**显式拒绝**，不空成功。

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
| 共享 UI 层 | **cad-ui-core：平台无关的 wire protocol、逻辑布局（layout）、CAD 会话（session，`CadSession`/`CadStatus`/44-command 表）、宿主 admission/error 状态（shell，`ShellState`）、i18n 目录；UI 框架与 GPU 无关 |
| 宿主编排 | cad-host：HostCore（命令路由、打开/保存/出图管线、状态推送、导航/绘制 sink）；**不依赖任何 UI 工具包，也不依赖 slint/wgpu/web-sys/Android 类型** |
| 宿主平台契约 | cad-platform：PlatformFiles/ConfigStore/SurfaceSource/FontLoader/Persistence/TaskExecutor/Clipboard/HostLifecycle/RenderHost |
| 宿主组合根 | apps：app-linux（React/Tauri，真实 Linux GUI）、app-windows/app-macos/app-android/app-web（显式 NOT IMPLEMENTED 占位） |
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

## 桌面宿主：React/Tauri + 原生 GTK/wgpu 分区

决策见 ADR 0010（宿主）与 ADR 0011（原生 Wayland 后端）；使用说明见 `docs/react-linux-app.md`；
需求增补见 `CAD_IMPLEMENTATION_SPEC.md` §21。

- **组合**：GTK `GtkOverlay` 内的中央原生 CAD 分区承载原生 surface（Rust + renderer-only wgpu），
  Tauri `WebView` 作为透明 sibling 呈现 React chrome。二者 **co-visible**；顶层透明 alpha 全屏叠加
  是历史 FAIL（保留于 ADR 0006）。
- **后端与原生表面**：宿主支持 **X11/XWayland** 与**原生 Wayland**。
  - X11：中央 `GtkDrawingArea` 获得自己的**原生 X child 窗口**，渲染器用其 Xlib XID/display。
  - Wayland：`GtkDrawingArea` 保持**非 native**（**禁止** `gdk_window_ensure_native`）；宿主自建
    **app-owned child `wl_surface` + `wl_subsurface`**，父级为 GDK toplevel 的 `wl_surface`，渲染器
    只接收该 child 的 live Wayland raw handles。该协议逻辑由 `app-linux` **私有小 C shim**
    （`apps/app-linux/native/wayland_shim.{c,h}`，`build.rs` 经 `cc`+`pkg-config wayland-client` 编译）
    封装：借用 GDK 的 `wl_display`，**从不** `dispatch`/`roundtrip`/`flush`（**GTK 独占事件分发**）。
  - 两种后端都**不**在顶层/GDK-owned surface 上建 Vulkan swapchain，也**不**做 CPU 帧拷贝。
- **合成约束**：**不允许**跨 CAD 分区的 DOM 浮层。下拉/自动补全/命令历史**就地展开**（撑高所在
  region），面板（Start、Diagnostics）**停靠**并挤窄分区，从而绝不覆盖原生 surface。`.native-input`
  CAD 分区始终挂载并上报 `getBoundingClientRect()`（CSS px + DPR），宿主 `LayoutTracker` 据此更新
  surface metrics——这是滚轮缩放锚定光标的前提。
- **平台范围**：**Linux（X11/XWayland 或原生 Wayland）** 宿主真实可运行（后端由
  `apps/app-linux/src/backend.rs` 显式决议 `GDK_BACKEND`，拒绝静默回退）。非 Linux 目标上
  `app-linux` 编译为空库、二进制返回显式 NOT IMPLEMENTED；Windows/macOS/Android/Web 均为**显式
  NOT IMPLEMENTED 占位**，不是可运行 GUI 产品。
- **支持范围**：仅 **OPEN / FIT** 接线到真实漏斗；其余 UI 控件显式禁用并标注原因；原生导航按
  实际源码能力支持（缩放/平移/fit）。**不新增投机 IPC**；需要 payload 的命令经真实漏斗显式拒绝，
  绝不空成功。**不是完整 CAD 产品**。
- **不变量不变**：单 presenter per drawable、renderer-only wgpu、db 唯一权威、无整幅回读；
  `FrameCompleteness::BaseOnlyPendingOverlays` 可**临时**省略 overlay 后缀（显式标记，非静默）。

## 历史（已退役 / 被取代）

- ADR 0002（Slint/wgpu 合成）、ADR 0003（浏览器 Slint/wgpu）描述**旧的 Slint 基线**；
  桌面部分被 ADR 0010 取代，Web/Android UI 已退役（见 ADR 0010 与 `docs/handoff.md`）。
- ADR 0006（React + Tauri 实验）是本次迁移的**前身实验**，其“实验/未删旧代码”描述已过时。
- `docs/code-structure.md` 中描述 Slint 渲染桥文件职责的段落属历史；当前宿主结构见本文与
  `docs/react-linux-app.md`。
