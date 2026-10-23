# ADR 0024：DOM-canvas CAD 路径（wasm/WebGL2 CAD 会话承载 X11/XWayland 与浏览器宿主）

- 状态：**Accepted（2026-10-23，用户授权方向变更）**；**代码未开始**——本轮只产出文档与本轮探针，
  **一行实现代码都没改**。分阶段状态见第 7 节（① 已在本轮完成，②–⑧ 全部「未实现」）。
- 日期：2026-10-23。
- 相关 ADR：[ADR-0003](0003-web-composition.md)（浏览器宿主历史组合；其 `:24-25` 的「wgpu 30 在 wasm
  上启用 `webgl` feature 时提供 `Backends::GL`（WebGL2）路径」仍是现行技术事实，Slint 部分已退役）、
  [ADR-0010](0010-react-linux-host-replaces-slint.md)（React/Tauri 宿主；其 `:22-28` 的「CAD 由原生
  Rust 绘制在 child surface 上」对 Wayland 仍成立，对 X11 的目标态被本文修订）、
  [ADR-0023](0023-dom-on-top-cross-host-composition.md)（**现行合成权威**；本文修订其 D1 的「单一合成
  方向」表述与 §5 的「X11 web+canvas 显式延后」，**不**改其输入漏斗、弹窗围栏与单 presenter 不变量）。
- 需求：`CAD_IMPLEMENTATION_SPEC.md` §21（本轮已同步修订，逐条对照见第 6 节）。
- 前身证据（**本轮实跑探针**，不是历史结果复用；原始记录见第 0 节与 `docs/handoff.md` 顶部条目）：
  WebKitGTK 2.52.6 在 Wayland GNOME 会话内的 X11/XWayland 实例里 **WebGL2 上下文可用**，
  `navigator.gpu === false`，WebKit WebProcess 持有 NVIDIA render node 的 DRM fd。
- 影响：像素承载机制（新增 `surfaceMode: dom-canvas`）、`cad-db` 权威位置（wasm 侧）、`app-linux`
  X11 分支的产品意义（拒绝 → 分阶段承接）、`apps/app-web` 的复活计划、退出码注册表（+6）、
  证据边界（synthetic 级 + XWayland 限定）。**不**改变：输入单一漏斗、renderer-only wgpu、
  importer-only acadrust、无逐帧整幅回读、单 presenter（每条路径）、弹窗模态围栏。

---

## 0. 本轮探针原始结果（只记录，不外推）

环境：Wayland GNOME 会话内的 **X11/XWayland** 客户端；**WebKitGTK 2.52.6**
（本机 `pkg-config --modversion webkit2gtk-4.1` 同为 `2.52.6`，与本轮探针一致）。

| 观测项 | 本轮结果 | 能证明什么 / 不能证明什么 |
|---|---|---|
| `canvas.getContext('webgl2')` | 成功；`gl.getParameter(gl.VERSION)` = `WebGL 2.0`，`GLSL` = `WebGL GLSL ES 3.00` | 证明**该 WebKitGTK 版本 + 该后端会话**下 WebGL2 上下文可创建 |
| `clear()` + `readPixels()` | 清屏后读回像素正确 | 证明上下文**可执行**（非仅句柄非空） |
| `navigator.gpu` | **`false`（无 WebGPU）** | 证明本路径**不能**依赖 WebGPU；WebGL2 是唯一候选 |
| `WEBGL_debug_renderer_info.UNMASKED_RENDERER_WEBGL` | 固定字符串 `"Apple GPU"`；vendor `"Apple Inc."` | **不可信**：这是 WebKit 的指纹掩码常量，**不能**用来判定硬件/软件渲染 |
| WebKit WebProcess 的 DRM fd | 持有 `/dev/dri/renderD129` 与 `/dev/dri/card2` 打开（本机 NVIDIA render node，Quadro P620，驱动 580.178.04） | **OS 级**旁证：进程确实打开了 NVIDIA render node；仍需在每次证据运行中重新采样 |

边界（写死）：本轮**未**截图、**未**产出渲染帧、**未**在真 X11 服务器/真 X11 窗口管理器上运行，
也**未**测帧率、DPI、多显示器或分数缩放。上表是**能力探针**，不是任何形式的视觉或性能验收。

**追加（2026-10-24）wgpu 30.0.1 / WebGL2 画布 spike**：在 `/tmp` 一次性 crate（**非仓库代码**）中
用 `wgpu 30.0.1`（`default-features = false, features = ["webgl","wgsl","std"]`）+ `wasm-bindgen
0.2.129` 走完 `Instance::new(Backends::GL)` → `create_surface(SurfaceTarget::Canvas)` →
`request_adapter(compatible_surface)` → `request_device` → `configure` → 三角形绘制 →
`queue.present`，返回 `{"backend":"Gl","format":"Rgba8UnormSrgb","frames":4,"surface_error":null}`，
adapter name 仍是被掩码的 `"Apple GPU"`。像素判读：Mutter `ScreenCast` 单帧裁剪到探针窗口后，
canvas 区内出现 clear 色的 sRGB 值 `(63,97,124)`（35,802 px）与三角形色的 sRGB 值 `(249,179,89)`
（16,358 px）。**能证明**：wgpu 的 GL/WebGL2 后端在 WebKitGTK 2.52.6 + XWayland 下可建 surface 并
出真实像素。**不能证明**：CAD 渲染、性能、硬件/软件归属（本 spike 未采 DRM fd）、真 X11、其它
WebKitGTK 版本；同一台机器上 `ffmpeg -f x11grab` 与 `gnome-screenshot` 的 X11 回退抓屏**全黑**，
X11 会话取证必须走 Mutter `ScreenCast`（`scripts/screencast-capture.py`）。

## 1. 问题

ADR 0023 D3 把 X11/XWayland 判为**显式 Unsupported**（`apps/app-linux/src/backend.rs:38` 标记行
`yacr: unsupported-backend: x11` + `:46` 退出码 4），理由是 2026-10-22 探针在 XWayland 上**原生表面
合成 FAIL**（透明区域透出另一 CAD 应用），机制未闭环。这个判断对**原生表面**路线仍然成立，但代价
是 X11 用户完全没有可用路径，且 `apps/app-web` 只是一句退役诊断
（`apps/app-web/src/lib.rs:16`）。

与此同时，本轮探针（第 0 节）说明「像素进入 WebView」在 WebKitGTK 里是**可用**的：WebGL2 上下文能建、
能清屏、能读回；缺的只是把 CAD 画进去的一条受控路径。于是问题不是「X11 能不能用原生表面」，
而是：

> **怎样新增第二种像素承载机制，而不让 React UI 按平台分叉、不破坏输入单一漏斗、不在宿主侧
> 复制出第二份 `cad-db` 权威？**

当前代码事实（必须据实理解「未实现」的深度）：

- workspace 依赖 `Cargo.toml:68` 为 `wgpu = "=30.0.1"`，`crates/cad-render-wgpu/Cargo.toml:20`
  `wgpu.workspace = true`——**没有启用 `webgl` feature**。因此 `crates/cad-render-wgpu/src/
  renderer_impl/pipelines.rs:209`（`wgpu::Backend::Gl` + wasm32 → `ActiveBackend::WebGl2`）在
  当前配置下是**不可达的死代码**。
- **没有**任何 canvas surface 创建代码：渲染器只在 wasm 之外暴露 `headless`
  （`crates/cad-render-wgpu/src/lib.rs:24-26`）与 `native_presenter`（同文件 `:36-45`）。
  `crates/cad-render-wgpu/Cargo.toml:42-45` 只在 wasm 目标声明了 `web-sys` 的
  `HtmlCanvasElement`（注释写「browser surface path lives here」），但**没有模块使用它**。
- `apps/app-web` 是显式 NOT IMPLEMENTED 桩（`apps/app-web/src/lib.rs:16`、`:20-25`）。

## 2. 决策

### D1 两种**已建模**的像素承载机制：`surfaceMode: native-below | dom-canvas`

像素承载从「唯一方向」改为「**两种都显式建模**的机制」，由宿主上报的**数据标志**选择，
UI 不按平台分叉：

| `surfaceMode` | 宿主 | 像素来源 | 合成方式 |
|---|---|---|---|
| `native-below` | Linux 原生 Wayland、Windows | 原生 wgpu presenter | 原生表面压在 WebView 之下（ADR 0023 D1 原样） |
| `dom-canvas` | X11/XWayland（**目标态，未实现**）、浏览器宿主（**目标态，未实现**） | wasm 侧 CAD 会话经 WebGL2 画进 `<canvas>` | **无第二原生表面**；canvas 在 DOM 树内，由 WebKit 自行合成 |

- **输入漏斗不变**：唯一入口仍是 `.native-input`（`apps/desktop-ui/src/components/Workspace.jsx:37-41`）
  → `probe_input` IPC → host 准入门/队列。`dom-canvas` **不**引入第二条输入通道，**不**让 `<canvas>`
  接事件（见 D3）。
- **数据标志而非平台分叉**：React 树只读宿主上报的 `surfaceMode`（wire 只做加法）。旧宿主缺该字段时
  按 `native-below` 处理——这是「当前唯一存在模式」的**事实回退**，不是能力猜测；新宿主必须显式上报。
- **单 presenter 不变量按路径成立**：`native-below` 路径仍只有一个原生 CAD presenter；`dom-canvas`
  路径只有一个 wasm presenter。**任何路径都不得同时存在两个 presenter 画同一块像素**。
- **wgpu 边界不变**：只有 `cad-render-wgpu` 依赖 wgpu；宿主只递交 **canvas handle**，永不接触 wgpu 对象
  （与 `crates/cad-render-wgpu/Cargo.toml:42-43` 的既有注释契约一致）。canvas handle 的具体形态
  （`<canvas>` 元素 vs `OffscreenCanvas`）属实现细节，本文不预决。

### D2 DOM-canvas 宿主：wasm 侧 CAD 会话是唯一 `cad-db` 权威（裁决 M2′）

- 在 `dom-canvas` 宿主上，**wasm 侧运行 CAD 会话**（`cad-db` + 命令/事务 + 显示表示 + 渲染），它是
  文档状态的**唯一**权威。native 壳（Tauri 宿主进程）**不得**实例化第二个 CAD 会话，**不得**镜像/
  缓存文档状态，**不得**把 native 会话的显示表示推给 wasm 充当数据源。
- native 壳在 `dom-canvas` 上只保留**策略与平台缝**：窗口/WebView 生命周期、i18n 目录、弹窗模态围栏的
  准入判定、文件选择、诊断与证据驱动、launch options。准入判定**不得**持有文档状态。
- 宿主准入门与 `ShellState.popup` 模态围栏（ADR 0023 D4）作为**策略层**保留；命令最终落到 wasm 会话。
  具体 IPC 形状在阶段 ④/⑤ 定，本文不预决 wire 细节。
- 可行性前提（**本轮未验证**）：`cad-db` 只依赖 `cad-domain`（`scripts/check-architecture.py` 强制），
  因此 wasm32 可编译；既有门禁 `cargo check --workspace --lib --target wasm32-unknown-unknown --locked`
  历史轮次 rc=0（本轮 **NOT RUN**）覆盖了 `--lib` 目标，但**不**等于「wasm 会话在运行时跑通」。

### D3 canvas 在 React 树中的位置（唯一合法位置）

- `<canvas>` 必须是 `.workspace__canvas`（`apps/desktop-ui/src/components/Workspace.jsx:35`）的子节点，
  且是 `.native-input` 的**紧邻前一兄弟**（即插在 `:36-41` 那个 div 之前）。
- `<canvas>` 必须 `pointer-events: none` 且 `aria-hidden="true"`：像素承载器**不是**交互目标，也不进
  可访问性树；`.native-input` 继续独占命中与 `aria-label`（`a11y.canvas_label`）。
- 该节点的**唯一**职能是像素面：不注册监听器、不持有文档状态、不做坐标换算以外的事
  （坐标仍走既有 CSS/canvas-local/world/physical 四类区分）。
- 透明不变量（ADR 0023 D2）在 `dom-canvas` 上退役但**不得误删**：`.workspace__canvas` 的
  `background: transparent`（`apps/desktop-ui/src/style.css:357`）是为 `native-below` 服务的；
  `dom-canvas` 下 canvas 自绘背景即可。两条路径的前端回归测试必须分别钉住，不能用一个断言覆盖两种模式。

### D4 后端决议仍是 GTK 初始化前的**一次性**决议，无热切换

- `apps/app-linux/src/backend.rs` 仍在 GTK/Tauri 初始化**之前**决议 `GDK_BACKEND`；解析结果为 X11 时，
  目标态是选择 `surfaceMode=dom-canvas`（阶段 ④），**不是**在运行期切换机制。
- **无热切换**：不支持「运行中从 `native-below` 换到 `dom-canvas`」或反向；跟随 SPEC §6 的既有立场
  （后端选择可整会话重建，不要求运行期切换）。
- 能力探测的时序必须诚实：`GDK_BACKEND` 决议在 GTK 前，但 **WebGL2 上下文只能在 WebView 建好之后
  真实创建**。因此「WebGL2 不可用」属于**运行时**拒绝（D5，退出码 6），与「后端策略非法」
  （退出码 2）、「后端不支持」（退出码 4）是不同故障域，不得合并码值。

### D5 能力缺失必须显式拒绝：标记行 + i18n + **退出码 6**

`dom-canvas` 路径上若 WebGL2 不可用（`canvas.getContext('webgl2')` 返回 `null`，或上下文创建后
基本操作失败），必须显式拒绝：

- stderr 稳定标记行：**`yacr: unsupported-runtime: webgl2`**（第一分类依据）。
- i18n 诊断：`startup.webgl2_unavailable` + `.detail`（**提案键名，当前不存在**；实现阶段必须
  `crates/cad-ui-core/i18n/{zh-CN,en}.json` **同批**加入，键集必须一致，`scripts/check-i18n.py` 闸门）。
- **退出码 6**（注册表见第 9 节）。
- **禁止**：空白/黑屏 canvas 当成功；静默回落到 X11 native-child（ADR 0023 D3 已判不可用）；
  静默回落到软件渲染并当作硬件路径；把 `navigator.gpu === false` 单独当作拒绝理由
  （本路径以 WebGL2 为目标后端，WebGPU 缺席**不是**错误）。
- 探测必须是**真实上下文创建 + 一次基本操作**，不得只嗅探特性字符串或 UA。

## 3. 否决的备选方案

- **M1：native 渲染 + 通过 IPC 把预构建的显示表示批次流进 wasm**。否决理由：①**双数据路径**——
  native 会话必须存在才能产出批次，与 D2 的「唯一权威」直接冲突；②**增量 wire 复杂度**——ChangeSet →
  依赖失效 → 显示表示重建的语义（SPEC §21 保留不变量）要在 IPC 上重新编码一遍，overlay 后缀、
  generation、revision 校验全部要过网；③ 放弃 wasm 侧本可直接执行的相机移动（pan/zoom 变成往返）。
  与 ADR 0023 §3 否决的「texture→canvas/整幅回读」属同一类成本论证（那条仍有效：整幅回读依旧禁止）。
- **复活 X11 native-child（`unreachable_since_dom_on_top` 那套）**。否决理由：2026-10-22 探针已在
  XWayland 上**合成 FAIL**（透明区域透出另一 CAD 应用），机制未闭环（ADR 0023 D3、§3）；重新启用等于
  把已知失败当交付。注意：该代码仍在树内编译保留、运行时不可达，删除是独立清理项，**不**因本文复活。
- 记录一条**未被否决但受约束**的推理（不是证据）：`dom-canvas` 路径**不存在**第二原生表面，因此
  2026-10-22 那个「独立原生表面被其它应用透出」的失效模式在**结构上**不适用。这是设计论证，
  必须由阶段 ⑥ 的真实运行证据确认，本文不据此宣称 X11 合成问题已解决。

## 4. 后果

**正面**：

- X11/XWayland 从「永久拒绝」变为「**有分阶段承接路径**」（当前仍未实现、仍拒绝，见 D5 与第 7 节）；
- 浏览器宿主（`apps/app-web`）与 X11 共享**同一条** wasm canvas 路径，不再各自发明机制；
- React UI 仍是一棵树、一个输入漏斗；平台差异收敛到 `surfaceMode` 数据标志与宿主平台缝；
- wasm 成为 `cad-db` 单一权威后，浏览器/桌面两端文档语义天然一致（阶段 ②/③ 之后才成立）。

**代价与风险**：

- **测试矩阵扩大**：两种 `surfaceMode` × 两套前端回归断言 × 两个证据驱动；任何「只在一种模式成立」
  的结论都必须标注模式。
- **WebGL2 是受限目标**：无 compute、无 storage buffer、无 indirect draw
  （`crates/cad-render-wgpu/src/renderer_impl/pipelines.rs:217-222` 的能力表已显式建模为
  `compute: false` 等）。真实管线的能力降级与性能 **NOT VERIFIED**。
- **性能未知**：wasm 侧几何/上传开销、WebKit 合成路径、XWayland 下的 GL 转发开销全部未测；
  本文**不**给出任何帧率承诺。
- **权威分裂风险（架构层面最大风险）**：只要 native 壳保留任何「顺手持有会话」的便利代码，D2 就会被
  悄悄破坏。铁律条（`AGENTS.md`）与阶段 ④ 的契约测试必须钉住。
- **证据债务**：本轮只有能力探针；X11 canvas 的可见几何/pan/zoom 证据为零（阶段 ⑥ 才产出）。

## 5. 非目标（显式不做 / 不由本文覆盖）

- **Windows 不变**：仍走 `native-below`（透明 WebView2 + `WS_DISABLED` child HWND），不引入 canvas 模式；
  WebView2 缺失仍是标记行 `yacr: unsupported-runtime: webview2` + 退出码 5。
- **macOS 仍是 NOT IMPLEMENTED 占位**（`apps/app-macos`）。
- **无热切换**（D4）。
- **import-in-wasm 分期**：阶段 ①–⑦ 全部只覆盖 **synthetic fixture**；DWG 导入/代理/资源解码在 wasm 侧属
  阶段 ⑧，**另立项**。不得把「canvas 能画线」当作「wasm 能打开 DWG」。
- **ADR 0023 §5 的「平台合成 ADR（RenderTarget/HostTexture token 所有权）」仍独立延期**，**不被本文覆盖**：
  它是 renderer 资源契约，与像素承载机制正交；`docs/handoff.md` 该条目原样保留。
- 不做 X11 下的原生 drop-shadow/装饰窗口优化、不做 Wayland canvas 回退、不做 WebGPU 优先策略
  （`navigator.gpu` 为 false 已是本轮事实；若未来可用，属另行决策）。

## 6. 失效条款清单（逐条前后对照）

> 下表 `SPEC §21` 与 ADR 0010/0023 的**行号均为 2026-10-23 修订前的基线**；本轮已在原文件插入署期
> 注记，因此当前行号已下移。原文一律保留，未删除任何段落。

| 文档 | 位置 | 原文（摘要） | 修订后 | 处置 |
|---|---|---|---|---|
| SPEC §21 | `:755-757` | 「本实验路线**不允许 WASM**、不允许逐帧整幅 GPU→CPU 回读」 | **允许** wasm 作为 `dom-canvas` 宿主（X11/浏览器）的 CAD 会话与渲染承载；**逐帧整幅回读仍禁止**；wgpu 边界不变 | 已加 2026-10-23 修订注记（原文保留） |
| SPEC §21 | `:768-769` | 「合成模型：DOM/WebView 在像素与输入两个方向都置顶……客户端 React UI 不按平台分叉」 | 输入方向单一漏斗不变 + **两种已建模像素承载机制**（`native-below`/`dom-canvas`），用户可见契约一致；React 树按 `surfaceMode` 数据标志渲染 | 已加修订注记 |
| SPEC §21 | `:800` | 「Linux 原生 Wayland：**唯一生产路径**」 | 原文措辞保留并加括注：指**原生表面**承载路径；`dom-canvas` 已立项但未实现，**不**构成第二条已交付生产路径 | 已加限定注记 |
| SPEC §21 | `:806-809` | X11「显式 Unsupported……web+canvas 回退路线**显式延后**」 | X11 = `dom-canvas` 路径，**分阶段、当前未实现**；WebGL2 不可用或路径未就绪时**保留显式拒绝**（新标记行 + 码 6）；旧 `unsupported-backend: x11` + 码 4 作为**历史合同**保留说明 | 已加修订注记 |
| SPEC §21 | `:815-818` | 「明确 NOT RUN / NOT IMPLEMENTED」清单 | 追加：X11 canvas 路径已立项（本文），**代码未开始**；浏览器宿主**仍未构建** | 已追加 |
| ADR 0023 | D1 `:47-67` | 「所有客户端只有一个合成方向」 | 修订为**两种已建模像素承载机制**；**输入漏斗与单 presenter 不变量不变** | 已加署期增补（正文未重写） |
| ADR 0023 | §5 `:205` | 「X11/XWayland 的 web+canvas 回退路线：显式延后」 | 该路线已由本文**立项**（仍分阶段、未实现） | 已加署期增补 |
| ADR 0023 | §9 `:253-262` | 退出码表止于 5 | 需要 +6 行 | **本轮未同步**（docs-only lane 不能扩散）→ **同步债务** |
| ADR 0010 | `:22-28` | 「CAD 由原生 Rust 绘制在 `GtkDrawingArea`/`GtkOverlay` 的 child surface 上」 | 对 `native-below`（Wayland/Windows）仍成立；X11 目标态改由 wasm canvas 承载 | **本轮未改**（docs-only 范围只含 5 个文件）→ **同步债务**；其前提已被 ADR 0023 §10 解释过，本文不重写正文 |
| ADR 0010 | `:92-105` | 2026-10-22 增补「合成顺序是产品选择，不是平台必然」 | 结论方向不变，应追加指向本文（X11 承接路径） | **本轮未改** → **同步债务** |
| ADR 0003 | `:1-7` | 浏览器宿主「已退役（2026-10-16）」 | 目标态将在阶段 ⑦ 复活为共享 wasm canvas 路径；**当前仍是桩** | 增补注记（本轮未改，列为债务） |
| `docs/build.md` | `:55-63` 退出码表 | 止于码 5 | 需 +6 行 | **同步债务** |
| `crates/cad-ui-core/i18n/{zh-CN,en}.json` | `:711-714` 邻近 | 无 webgl2 键 | 需加 `startup.webgl2_unavailable`(+`.detail`) | **同步债务**（两目录同批） |
| `AGENTS.md` | 铁律 / 平台状态 / 授权边界 | — | 已同步本轮 | 本轮已改 |

## 7. 迁移与分阶段计划（每阶段当前状态必须如实标注）

| # | 阶段 | 状态 | 判据 / 产出 |
|---|---|---|---|
| ① | 文档：本文 + SPEC §21 修订注记 + ADR 0023 增补 + `AGENTS.md` + `docs/handoff.md` | **本轮完成** | 五个 Markdown 文件；**零代码改动** |
| ② | wasm renderer canvas 接通（启用 `wgpu/webgl`：`Cargo.toml:68` 加 feature，`cad-render-wgpu` 增 canvas surface 创建，wasm-only） | **未实现** | `ActiveBackend::WebGl2` 不再是死代码；配置/边界契约测试；架构门禁仍过 |
| ③ | wasm 合成场景 + 相机（synthetic fixture → 场景批次 → pan/zoom，无 import） | **未实现** | 纯 wasm 单测/契约测试；无 GPU 断言只到批次层 |
| ④ | `app-linux` X11 走 `dom-canvas`（`backend.rs` 一次性决议；WebGL2 不可用 → 标记行 + 码 6） | **未实现** | 拒绝合同测试（标记行 + 码 6）；D2 权威不变量契约测试 |
| ⑤ | React `<CadCanvas/>`（`.workspace__canvas` 内、`.native-input` 前一兄弟、`pointer-events:none`/`aria-hidden`；按 `surfaceMode` 渲染） | **未实现** | 源码结构/纯函数前端测试（**不是**运行时挂载证据） |
| ⑥ | **有界 X11 运行证据**（真实显示 + 真实 X11/XWayland 会话；fixture 可见 + pan/zoom） | **未实现** | 每轮新目录；软件渲染必须标注；含可见几何与前台确认 |
| ⑦ | `apps/app-web` 复活并共享同一 wasm canvas 路径 | **未实现** | 退役桩替换；`build-web.sh` 退役非零状态需用户决策后才改 |
| ⑧ | 阶段 2：真实管线 in wasm（import/DB/命令/事务） | **未实现（另立项）** | 不在本文范围；须独立 ADR 或 SPEC 修订 |

**阶段 1 验收定义**（用户已定）：**synthetic fixture 可见 + pan/zoom 可用**，仅此。
不含 DWG 打开、测量、批注、精度、性能、vendor 结论。

## 8. 证据边界（写死，防误读）

- **synthetic 级**：阶段 1 的一切「能画/能动」只对 synthetic fixture 成立，**不**构成 CAD 产品能力。
- **X11 = XWayland**：本轮探针在 Wayland 会话内的 XWayland 上运行，**不是**真 X11 服务器、**不是**
  真 X11 窗口管理器；结论不得推广到真 X11、其它 WM、NVIDIA/Intel/AMD 之外的 vendor。
- **渲染器字符串不可信**：`UNMASKED_RENDERER_WEBGL` 返回固定 `"Apple GPU"`/`"Apple Inc."`（WebKit 指纹
  掩码）。硬件归属只能靠 **OS 级**证据（DRM render node fd、驱动版本，如本轮
  `/dev/dri/renderD129` + `/dev/dri/card2` + 580.178.04），且必须**每次运行重新采样**，不得沿用旧值。
- **软件渲染必须标注**：若某次运行实际由软件栅格化（llvmpipe/SwiftShader 等），该次结论必须显式写
  「软件渲染」，**不得**计入真实 GPU 证据，也不得用于性能结论。
- **CSP 必须放行 wasm（2026-10-24 实测，硬阻断）**：在 `script-src 'self'` 的 meta-CSP 页面下，
  **WebKitGTK 2.52.6 与 Chromium 141 都拒绝** WebAssembly 实例化——
  `Refused to create a WebAssembly object because 'unsafe-eval' or 'wasm-unsafe-eval' is not an
  allowed source of script`（Chromium 文案同义）。因此 DOM-canvas 宿主的 CSP 必须包含
  `'wasm-unsafe-eval'`（`apps/app-linux/tauri.conf.json:11` 当前为 `script-src 'self'`，阶段 ④ 必须改）；
  放行失败时按 D5 显式拒绝（标记行 + 码 6），**不得**空白 canvas。证据：本轮 `/tmp/opencode/csp-probe`
  的本地静态页探针（仅验证 CSP 语义，**不是** CAD 运行证据）。
- **Chromium 侧同样无 WebGPU**：Playwright 141（headed，X11/XWayland）`navigator.gpu === false`，
  WebGL2 走 ANGLE（`ANGLE (Intel, Mesa Intel(R) UHD Graphics (CML GT2), OpenGL 4.6)`），GPU 进程持有
  `/dev/dri/renderD129` + `/dev/nvidiactl` + `/dev/dri/renderD128`。故 **wasm 侧一律按 WebGL2-only
  设计**，不得把 WebGPU 当可用能力。
- **UI "DOM 测试" ≠ 运行时**：`apps/desktop-ui` 的测试是源码结构/纯函数契约，**不是**运行时挂载、
  不是像素、不是挂载后层序证据。
- **能力探针 ≠ 视觉验收**：第 0 节没有截图、没有帧；阶段 ⑥ 之前，X11 canvas 视觉证据为零。
- **不跨模式复用证据**：`native-below` 的历史 PASS（Wayland/P620/`scale=1`）**不得**冒充
  `dom-canvas` 结论；反之亦然。

## 9. 退出码注册表（本文起 +6）

| 码 | 含义 | 产点 | 标记行 |
|---|---|---|---|
| 0 | 成功（含正常退出、生命周期驱动自然结束） | `app-linux/entry.rs` | — |
| 1 | 通用失败（Tauri 构建失败、启动合同拒绝、Windows 窗口构建失败） | `app-linux/src/entry.rs`、`app-windows/src/entry.rs` | — |
| 2 | 无效/不可用的显示后端策略 | `app-linux/src/entry.rs`（`BackendRefusal::Invalid`） | — |
| 3 | postload-pan 证据驱动失败 | `app-linux/src/host/postload.rs` | — |
| 4 | **不支持的后端（X11/XWayland，历史合同）** | `app-linux/src/backend.rs:46` | `yacr: unsupported-backend: x11` |
| 5 | **不支持的运行时（WebView2 缺失）** | `app-windows/src/support.rs:45` | `yacr: unsupported-runtime: webview2` |
| **6** | **不支持的运行时（`dom-canvas` 宿主缺 WebGL2）** | 待实现（阶段 ④；探测点在 WebView 建好之后） | `yacr: unsupported-runtime: webgl2` |

分类纪律（不变）：证据脚本以**稳定标记行**为第一依据，码值仅辅助；不得以退出码 0 或非空截图
单独证明视觉正确。**码 4 在阶段 ④ 完成前仍是 X11 的现行合同**；完成后码 4 转为历史合同（保留在
注册表与代码注释里，不静默复用）。
**同步债务**：`docs/build.md:55-63`、ADR 0023 §9 尚未加入码 6（本轮 docs-only 范围限制）。

## 10. 历史保留（不抹、不改写）

- 2026-10-22 的 **XWayland 合成 FAIL** 记录**继续有效**，其对象是**原生表面**叠加；本文的
  `dom-canvas` 路径不是它的翻案，是**另一个机制**。不得用本文的推理去改写那次 FAIL 的结论。
- ADR 0023 D3 的「本轮不支持」与退出码 4 的实现**保留**，直到阶段 ④ 真实改变行为；在那之前
  `apps/app-linux` 的实际行为仍是拒绝，**文档不得写成「X11 已支持」**。
- ADR 0023 §5 的 RenderTarget/HostTexture 平台合成 ADR **仍独立延期**，本文不碰。
- `apps/app-web` 的退役记录与 `build-web.sh` 非零失败**保留**；阶段 ⑦ 之前「Web 宿主可用」是假话。
- 本轮探针**不**产生新的验收等级：X11 canvas 的最高诚实表述是
  「**已立项 + 能力探针通过（WebGL2 可用、无 WebGPU）；实现与视觉证据均为 NOT RUN**」。
