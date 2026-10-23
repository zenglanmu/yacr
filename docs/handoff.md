## 2026-10-24 DOM-canvas 落地第一批实现：X11 真机出图，OS 级输入仍未验证

### 1. 结论

ADR 0024 的阶段 ②③④⑤ 已**落地代码**（阶段 ① 文档此前完成）：wasm CAD 会话在 WebGL2 上跑通，
X11/XWayland 宿主以 `surfaceMode=dom-canvas` 启动并**真实出图**（合成 fixture），浏览器宿主路径
（`apps/app-web`）与桌面宿主共享同一 wasm 路径。**但 X11 上的 pan/zoom 未取得 OS 级输入证据**
（本会话输入注入整体不可用，详见验证记录 §6.1），因此**不得**写成「X11 已支持」或「已可操作」。
证据与边界：`docs/validation-dom-canvas-wasm-2026-10-24.md`。

### 2. 本轮落地

- `crates/cad-render-wgpu`：wasm 目标启用 `wgpu/webgl`；新增 `canvas/{policy,presenter,tests}`（WebGL2
  canvas surface、显式错误分类、`resize`、`present_texture`）；把 textured fullscreen blit 抽到
  `src/blit.rs` 与原生 presenter 共享（原生语义未变）；`upload.rs` 的 `std::time::Instant` 换 `web_time` 别名。
- `apps/app-web`：从退役桩复活为 **wasm CAD 会话**（`include_bytes!` 合成 fixture → `AcadrustImporter`
  → `cad_app::render_scene::build_scene` → `prepare_upload_batches/commit_upload` → `Renderer::render`
  → `CanvasPresenter::present_texture`），导出 `start/resize/zoom_at/pan/status`；附无框架浏览器壳。
- `apps/desktop-ui`：`surfaceMode` 数据标志 + `<canvas id="cad-canvas">`（`.native-input` 前兄弟、
  `pointer-events:none`、`.native-input` 仍是唯一输入面）；`canvas-host.mjs` 提供 wasm transport 与
  `bindCanvasNavigation`；`native-below` 路径零改动。
- `apps/app-linux`：X11 → `SurfaceMode::DomCanvas`（不再退出码 4）；**不建**原生 CAD 会话 /
  `GtkDrawingArea` / Wayland 子表面 / presenter；`cad_status` 加 `surfaceMode`；新增
  `yacr: unsupported-runtime: webgl2` + **退出码 6** 拒绝合同（`canvas_runtime_failed`）；CSP 加
  `'wasm-unsafe-eval'`；DOM diag 增 `canvas/canvasBacking/wasmBinding/counts` 字段。
- `scripts/build-canvas-wasm.sh`：同一 wasm 产出 **web(ESM)** + **no-modules(classic script)** 两种
  绑定，落 `apps/desktop-ui/public/canvas/`（gitignored）。
- 文档：ADR 0024（含 §0 探针与 spike 原始结果、§8 CSP/无 WebGPU 约束）、SPEC §21 修订、ADR 0023 增补、
  AGENTS 铁律 8 + 授权边界放宽、`docs/build.md` 退出码 6、本轮验证记录。

### 3. 本轮修掉的真实缺陷

1. **CSP 阻断 wasm**：`script-src 'self'` 下 WebKitGTK 与 Chromium 都拒绝实例化；必须
   `'wasm-unsafe-eval'`（实测文案见验证记录 §5）。
2. **Vite dev 改写动态 import**：`import(/* @vite-ignore */ url)` 被改写成
   `__vite__injectQuery(url,'import')`，public 产物 `?import` 返回 **500** → WebKit 只报
   `Importing a module script failed.`。改为 classic script + 显式 wasm URL。
3. **wasm 运行期 panic 源**：`cad-engine`/`cad-ui-core`/`cad-host`/`cad-render-wgpu` 运行时路径上的
   `std::time::Instant::now()` 已改 `web_time`（native 行为不变）。

### 4. 门禁（有界，按日常策略）

`cargo check/test/clippy/fmt`：`cad-render-wgpu`（native 串行 238+，wasm check/clippy）、
`cad-engine 105`、`cad-ui-core 218`、`cad-host 196`、`app-linux 149`、`app-web` wasm+native、
`apps/desktop-ui npm test 450`。脚本：`check-architecture`、`check-i18n`(723)、`check-file-size` 均通过。
**NOT RUN**：全 workspace 全量门禁、Windows/macOS/Android、`collect-evidence.sh` 全矩阵。

### 5. 未闭环 / 下一步

1. **X11 输入证据（部分闭环）**：2026-10-24 **用户手动测试确认 X11 下滚轮缩放与拖拽平移正常**
   （用户报告，无截图、无 agent 像素差证据）。agent 侧仍无法注入输入（`xdotool`/`ydotool` 点击滚轮
   不被任何应用接收，连 GNOME 顶栏都无反应；未改动的 Wayland 原生路径同样如此 ⇒ 取证工具限制，
   非本轮回归）。若要 agent 级像素证据，需换到可注入输入的会话；在此之前**不得**写"X11 已验收"。
2. **WebKitGTK 硬件归属**：renderer 字符串被掩码、本轮未见 DRM fd 归属 ⇒ 该路径只能写
   「WebGL2 出图成立，硬件/软件归属 NOT VERIFIED」。
3. `scripts/collect-evidence.sh` + `test-collect-evidence.py` 仍把 X11 归为 `UNSUPPORTED_BACKEND` 码 4
   （**同步债务**）；`docs/build.md` 已记，脚本未改。
4. `documentLabel` 在 dom-canvas 宿主为 `null`（原生壳不镜像文档状态），左侧面板可能显示「未打开图纸」
   而画布已有几何 —— **产品/UX 决策待定**（不得靠 native 镜像文档解决）。
5. 阶段 ⑦：`apps/app-web` 的浏览器发布/CI 未恢复（`scripts/build-web.sh` 仍退役非零），
   wasm 产物也未进 CI；阶段 ⑧（真实管线 in wasm / import-in-wasm 内存预算）另立项。
6. `cad-host/src/lib.rs` 已 994 行（上限 1000），后续改动需先拆分。

---



### 1. 结论

用户当日授权**方向变更**：**X11/XWayland 不再永久是「显式 Unsupported」**，改为 **DOM-canvas 宿主**——
CAD 像素由 **WebView 内的 wasm（WebGL2）CAD 会话**绘制；**原生 Wayland 仍保留**现有 native wgpu 子表面
DOM-on-top 路径；退役的浏览器宿主 `apps/app-web` 计划**复活并共享同一 wasm canvas 路径**。
架构裁决（oracle）：**M2′**——DOM-canvas 宿主上 **wasm 侧运行 CAD 会话、是唯一 `cad-db` 权威**，
native 壳**不得**实例化或镜像第二个会话。契约权威：
[ADR 0024](adr/0024-dom-canvas-wasm-cad-path.md)（本轮新建）。
**本轮只有文档 + 能力探针：一行实现代码都没改**；X11 仍按退出码 4 拒绝，`apps/app-web` 仍是退役桩，
WebGL2 缺失的新拒绝合同（标记行 + **退出码 6**）**尚未实现**。

### 2. 用户决定与裁决记录

- **决定（2026-10-23）**：接受 DOM-canvas 作为第二种像素承载机制；阶段 1 验收 =
  **synthetic fixture 可见 + pan/zoom**，仅此（不含 DWG 打开/测量/批注/性能/vendor）。
- **oracle 裁决：M2′**（wasm 侧为唯一权威）。**否决**：M1（native 渲染 + 显示表示批次经 IPC 流入
  wasm：双数据路径、增量 wire 复杂、与「唯一权威」冲突）；**否决** 复活 X11 native-child
  （2026-10-22 XWayland 合成 FAIL 未闭环，见 ADR 0023 D3）。
- **不变量保持**：输入单一漏斗（`.native-input` → `probe_input` → host 准入门）、弹窗模态围栏、
  单 presenter/路径、**无逐帧整幅 GPU→CPU 回读**、renderer-only wgpu（宿主只递交 canvas handle）。

### 3. 本轮探针原始结果（只记录，不外推）

环境：Wayland GNOME 会话内的 **X11/XWayland** 客户端；**WebKitGTK 2.52.6**（本机
`pkg-config --modversion webkit2gtk-4.1` 同为 `2.52.6`）。

| 观测项 | 结果 |
|---|---|
| `canvas.getContext('webgl2')` | 成功；`WebGL 2.0` / `WebGL GLSL ES 3.00` |
| `clear()` + `readPixels()` | 正确（上下文可执行，非仅句柄非空） |
| `navigator.gpu` | **`false`（无 WebGPU）** → 本路径只能以 WebGL2 为目标后端 |
| `UNMASKED_RENDERER_WEBGL` | 固定 `"Apple GPU"` / vendor `"Apple Inc."` → **指纹掩码，不可信** |
| WebKit WebProcess DRM fd | 持有 `/dev/dri/renderD129` + `/dev/dri/card2`（NVIDIA render node，Quadro P620，驱动 580.178.04） |

边界：**未**截图、**未**产出渲染帧、**未**在真 X11 服务器/真 X11 WM 上运行；硬件归属只能靠
OS 级 fd/驱动证据（渲染器字符串不可信）；软件渲染运行必须显式标注。X11 canvas 的视觉证据
**为零**（NOT RUN）。

### 4. 代码现状（据实：为什么是「未开始」）

- `Cargo.toml:68` `wgpu = "=30.0.1"` **未启用 `webgl` feature** ⇒
  `crates/cad-render-wgpu/src/renderer_impl/pipelines.rs:209` 的 wasm `ActiveBackend::WebGl2` 映射是
  **不可达死代码**。
- **没有** canvas surface 创建：渲染器仅在 wasm 之外暴露 `headless`
  （`crates/cad-render-wgpu/src/lib.rs:24-26`）与 `native_presenter`（同文件 `:36-45`）；
  `crates/cad-render-wgpu/Cargo.toml:42-45` 的 wasm `HtmlCanvasElement` 声明**无模块使用**。
- `apps/app-web/src/lib.rs:16` 仍是退役诊断；`start_web` 直接 `Err`。
- X11 拒绝照旧：`apps/app-linux/src/backend.rs:38` 标记行 + `:46` 退出码 4。

### 5. 分阶段计划（全部「未实现」，除 ①）

① 文档（**本轮完成**）；② wasm renderer canvas 接通（启用 `wgpu/webgl`）；③ wasm 合成场景 + 相机；
④ `app-linux` X11 走 `dom-canvas`（WebGL2 不可用 → 标记行 + 码 6）；⑤ React `<CadCanvas/>`
（`.workspace__canvas` 内、`.native-input` 前一兄弟、`pointer-events:none`/`aria-hidden`、按
`surfaceMode` 渲染）；⑥ 有界 X11 运行证据（真实会话，fixture 可见 + pan/zoom）；⑦ `app-web` 复活共享
路径；⑧ 阶段 2 真实管线 in wasm（**另立项**）。逐阶段细节与判据见 ADR 0024 §7。

### 6. 同步债务（本轮 docs-only 范围之外，勿当作已完成）

- `docs/build.md:55-63` 退出码表、ADR 0023 §9 表：**缺码 6 行**。
- `crates/cad-ui-core/i18n/{zh-CN,en}.json`：缺 `startup.webgl2_unavailable`(+`.detail`)，
  实现阶段必须**两目录同批**加入（`scripts/check-i18n.py` 要求键集一致）。
- ADR 0003 的「浏览器宿主已退役」注记：阶段 ⑦ 才需要改。
- `scripts/collect-evidence.sh` 内嵌授权说明仍写旧边界（不提权/不 sudo），本轮未改脚本。

### 7. 门禁

**本轮未改任何代码**（仅 5 个 Markdown 文件），因此**未跑** cargo/npm/架构门禁（**NOT RUN**，不适用）。
只做文档自检：`rg -n "unsupported-runtime: webgl2" docs CAD_IMPLEMENTATION_SPEC.md AGENTS.md` 命中；
`git status --porcelain` 仅涉及 5 个文件（`CAD_IMPLEMENTATION_SPEC.md`、`AGENTS.md`、
`docs/adr/0023-…`、`docs/handoff.md` 为已跟踪修改，`docs/adr/0024-…` 为未跟踪新文件）。

### 8. 下一步 lane（建议顺序）

1. **实现 lane A**：阶段 ②（`wgpu/webgl` + canvas surface，wasm-only，含边界/配置契约测试）。
2. **实现 lane B**：阶段 ③（wasm synthetic 场景 + 相机，纯 wasm 测试）。
3. **实现 lane C**：阶段 ④（`app-linux` 决议 + 码 6 拒绝合同 + D2 权威不变量测试）。
4. **实现 lane D**：阶段 ⑤（`<CadCanvas/>` + `surfaceMode` 数据标志 + 前端结构测试）。
5. **证据 lane**：阶段 ⑥（真实会话有界运行；软件渲染必须标注）。
6. **后续**：阶段 ⑦（`app-web` 复活，需用户对 `build-web.sh` 退役状态的决策）、阶段 ⑧（另立项）。

---

## 2026-10-23 DOM-on-top 跨宿主合成落地（WP1–WP5）

### 1. 结论

生产合成方向已反转为 **DOM/WebView 在上、wgpu CAD 原生表面在下**：Linux 原生 Wayland 是唯一
生产路径（有界 Observer 判读）；**X11/XWayland 显式 Unsupported**（标记行 + 退出码 4，GTK 前拒绝）；
**Windows 宿主完整实现**，上限为 **Tier 0 编译门禁（全部 rc=0）+ Wine 拒绝合同 smoke（三次 rc=5）**，
视觉/运行时验收 **PENDING 用户真 Windows spike-1/4/5**。`macOS`/`Android`/`Web` 仍是显式
NOT IMPLEMENTED 占位。架构权威：[ADR 0023](adr/0023-dom-on-top-cross-host-composition.md)；
生产证据：[validation-dom-on-top-production-2026-10-23.md](validation-dom-on-top-production-2026-10-23.md)。
**透明修复后的像素复采已完成（有界）**：遮挡 bug 修复**像素验证 PASS**（几何满亮度、无蒙层；
About 覆盖层 PASS）；textEdit 编辑器内容、光标抑制确定性与 Wayland 最小化恢复仍未闭环。

### 2. 提交与范围（分支 `dom-on-top-layering`）

- `dece71c` UI 归并 `apps/desktop-ui`（WP1c）；`10a61e5` 新 crate `cad-desktop-shell`（WP1b）；
  `0093dae` WP1 门禁修复。
- `359d58a` WP2：Wayland `place_below` 无条件转正、旧排列删净、顶层 alpha 清屏生产化、
  X11 拒绝（标记行 + 退出码 4 + i18n 双目录）、探针转**回归夹具**、generation 重放。
- `9ecce95` WP3：弹窗迁入同一 React 树的 in-DOM 覆盖层；第二 Tauri 窗口机制退役；
  host 侧 D2 模态围栏保留，清除改**四触发**；`cad_open_popup` ack 加法式 `payload`。
- `6e4bf06` 生产透明修复（人工复测发现，见 §5）。
- `dd23bea` WP4：Windows 真实宿主（Tauri 2 + 透明 WebView2 + `WS_DISABLED` child HWND +
  单一 DOM 漏斗）；`9fba811` WP4 门禁修复（布局反转守卫、z-order 失败去重、退出码注册表文档）。
- WP5 文档收口（本条目及 `docs/**`、`AGENTS.md`、`CAD_IMPLEMENTATION_SPEC.md` §21）**已提交**（commit 13747be；含 WP5 门禁 4 项行级修复）。

### 3. 行为变化（用户可见 / 契约）

- **输入不变**：唯一漏斗仍是 `.native-input` → `probe_input` IPC → host 准入门/队列；CAD 表面
  永不接收 OS 输入（Wayland 空 input region；Windows `WS_DISABLED`）。**不声称**原生点击穿透。
- **DOM 命中规则**：不透明 chrome 正常命中；半透明装饰面板 `pointer-events: none` 穿透，
  面板内交互控件 `pointer-events: auto` 收回。
- **弹窗**：锚定菜单与居中模态都是同一 React 树的 DOM 覆盖层；Esc/关闭按钮在正文
  error-boundary 之外；renderer 崩溃/reload/进程退出均清门。
- **X11 用户**：启动即拒绝（`yacr: unsupported-backend: x11` + 中文诊断 + 退出码 4），无窗口、
  无静默回退；X11 web+canvas 路线**显式延后**。旧 X11 native-child 代码保留但运行时不可达
  （`unreachable_since_dom_on_top`），删除留作独立清理。
- **Windows**：真实宿主，非占位；WebView2 缺失 = 显式 Unsupported（标记行
  `yacr: unsupported-runtime: webview2` + i18n + 退出码 5）。
- **退出码注册表**（与 `docs/build.md`、ADR 0023 §9 同源）：0 成功 / 1 通用失败 / 2 非法后端策略 /
  3 postload-pan 证据驱动失败 / 4 不支持的显示后端 / 5 不支持的运行时。
- **回归夹具**：唯一触发器 `YACR_PROBE_STACKING=1`（严格 `"1"`）+ init-script
  `window.__YACR_STACKING_REGRESSION__ === true` + 真实 `--open`；`--probe`/`YACR_PROBE_ONLY`
  同现即拒绝。`YACR_PROBE_DOM_ON_TOP` 与注入对象**已删除**，不再有该开关。

### 4. 门禁（本轮实跑，仅改动范围）

- WP2：`app-linux` 138/138、`cad-ui-core` 212/212、`desktop-ui` 403/403、`clippy -D warnings`、
  `fmt --check`、architecture/workflows/linux-workflow/migration-guards/fixture-manifest/i18n/
  `test-collect-evidence.py` 全 rc=0。
- WP3：`cad-ui-core` 215/215、`cad-desktop-shell` 11/11 + wire goldens 4/4、`app-linux` 134/134、
  `desktop-ui` 411/411、fmt/clippy、file-size 840、architecture 28、i18n 709 全 rc=0。
- WP4：12 项 Tier 0 全 rc=0（fmt/clippy host/msvc、`app-windows --lib` 6 passed、xwin check/build、
  mingw build、wasm 空壳、i18n 716、file-size 851、architecture 28、migration guards 18）。
- **NOT RUN**：全 workspace 测试与全量 CI 门禁矩阵（按日常有界策略）；Android/wasm 平台矩阵；
  N>1 次 surface generation 的**运行期**重放（仅代码路径验证）；renderer 崩溃活体注入；
  Windows 全部视觉/运行时项；textEdit 编辑器像素证明（host-initiated open 降级 fallback）；
  光标抑制确定性；Wayland 最小化/还原恢复。

### 5. 人工复测抓到的生产 bug 与修复

- **症状**：迁移后生产 UI 中 CAD 层不可见，但 stacking 夹具帧正常。根因是
  `.shell`/`.workspace`/`.workspace__canvas` 在 CAD 视口区画了**不透明底色**。
- **修复**：`6e4bf06` 三者改 `background: transparent`；回归测试
  `apps/desktop-ui/test/metrics.test.mjs` 钉住透明不变量。
- **教训（已写入 `docs/testing-dwg.md` 证据规则）**：夹具验证层序 ≠ 生产 UI 透明性；证据取景
  必须包含**可见几何**；捕获时必须确认**目标窗口在前台**（WP3 textEdit 帧因此作废为
  wrong-window）；像素复采用新目录，不覆盖旧证据。
- **复采状态（已完成，有界）**：`/tmp/opencode/yacr-evidence-recapture1`（同 UID
  GNOME/Mutter + P620/Vulkan + `scale=1`，整屏 ScreenCast，一启动一帧）。独立 Observer 判读：
  - **A 透明视口** PASS：4 条竖 fixture 线 + 水平线满亮度、边缘锐利，canvas `(0,0,0)`、无蒙层；
    `frame1`/`arrowcheck-13`/`arrowcheck-16` 的 canvas 区域像素一致（三次启动可复现）。
    caveat：像素不能证明几何由哪一层绘制（旁证：CSS 回归测试 + 构建后 `background: 0 0`）。
  - **B crosshair** PASS（1px 全跨带精确裁到 canvas 边缘 + 6×6 空心交点标记）；系统箭头
    **observed, non-deterministic / NOT VERIFIED deterministic**——3 个启动缺失，`recon1-hover`
    与 `frame-about` 出现；后者有设计解释（模态打开 → `status.crosshair=false`），前者是未解释的
    1-in-many 离群，本捕获协议不能证明光标抑制。
  - **C About 覆盖层** PASS：卡片完整、scrim 压暗整个 DOM root（几何 luma 177→84 ≈47% 仍可见），
    CAD 竖线在卡片上方/下方明亮、卡片矩形内消失（z-order 决定性证据）。
  - **D textEdit** chrome PASS，但显示 fallback 单行「textEdit 在本版本不可用」，**无编辑器/页脚**；
    host-initiated open 不带 `popupPayload`，编辑器内容 **NOT pixel-proven**。另有测量分歧：
    lane 测得几何被压暗（fill 119,121,123），Observer 测得 ring luma 212 + fill 119 ⇒ fallback 帧
    的 scrim 覆盖 **INDETERMINATE**（About 帧无歧义）。
  - **E 结构缺陷**：无；仅外观项（About 版本槽空「—」、textEdit 空壳卡片）与 `recon2-minimize`
    仅桌面（Tauri unminimize 不恢复 Wayland 窗口，属捕获产物）。
  - `frame3-textedit*` 两行为 `PLANNED_TIMEOUT 143`（`--lifecycle 0` 的弹窗阶段 t≈238s 才出现，
    SIGTERM 为计划内收尾）；`x11-refusal` 行 `UNSUPPORTED_BACKEND` 4、无截图。
  - **结论**：遮挡 bug 修复**已像素验证**（有界）；仍开放 textEdit 编辑器像素证明、光标抑制确定性、
    Wayland 最小化/还原恢复。

### 6. Windows 证据与上限（禁止过度声称）

- Tier 0 全部 rc=0（MSVC/ mingw 产物 + wasm 空壳）；**这只证明目标可编译/链接，不是可运行 GUI**。
- Wine smoke W1–W3 均 **rc=5**，标记行/i18n/细节分支端到端成立；**Wine 不是 Windows**：
  DComp 是 stub（bug 58921），`WS_EX_LAYERED|NOREDIRECTIONBITMAP` 黑屏（bug 55432）。
  Wine 像素结论唯一允许的标注是 **`NOT VERIFIED (Wine limitation, bug 55432/58921)`**。
- **NOT RUN**：z-order 稳定性、DPI/多显示器、ProcessFailed 活体、真实 COM 选择器、
  runtime-present 启动/出帧、Windows 硬件 Vulkan 精度/性能。透明结论只能限定
  “该 Runtime 版本 + 该 GPU/驱动 + 该 Windows 版本 + 该缩放”；**禁止**写“Windows 已验收”。

### 7. 剩余工作与风险

1. **PENDING 用户真 Windows spike-1/4/5**（判据与证据格式见 `docs/windows-app.md`；
   spike-1 失败时按 tiling 应急回退并与用户决策，不擅自分片重构）。
2. **复采后续（复采本身已完成，见 §5）**：textEdit 编辑器像素证明（UI-path probe hook 或真实点击；
   host-initiated open 不带 payload）；光标抑制确定性（需真实指针移动协议）；Wayland 最小化/还原恢复。
3. **依赖锁风险**：本轮 `gpu-allocator 0.28.0 → windows 0.62.2` 只是 `Cargo.lock` 调整；
   未来 `cargo update` 可能重新引入重复 COM 绑定，需在 Windows 目标上复核。
4. **Tauri `WS_EX_NOREDIRECTIONBITMAP`**：2.11.6 无 builder flag，仅 best-effort；升级
   Tauri ≥2.12.3 后改用官方配置位并重跑 spike-1/N9。
5. **平台合成 ADR（RenderTarget/HostTexture token 所有权）**：**ADR 0023 §5 显式延期**，属
   renderer 资源契约，与层序正交；本文件「已定义，但尚需设计审查」中的原条目**原样保留**，
   不得凭 token 猜测 GPU 对象。
6. X11 web+canvas 路线显式延后；旧 X11 native-child 路径（不可达）待独立清理。
7. UI “DOM 测试”只是 `node:test` 源码结构/纯函数契约，**不是**运行时挂载或像素证据；
   不得据此宣称 UI 交互已验证。

### 8. 历史结论处理（不删除，只加署期注记）

- ADR 0010（`:27` 与 2026-10-22 增补后）、ADR 0011、ADR 0017（`:602`/`:870`）、ADR 0018、
  ADR 0019、ADR 0020、ADR 0021 均已加 **2026-10-23 取代/前提更正注记**：其“DOM 不得浮于 CAD /
  React 宿主无法绘制 overlay”的前提失效，但各自的**未实现事实**（标注预览、填充预览、就地编辑
  入口等）不因此自动完成。ADR 0021 的 D2 模态围栏**保留**，仅 D1/D3/D6 与约束 1/3/4/6 被取代。
- 历史整窗透明 alpha FAIL 仍然成立；本轮 PASS 的是分区子表面 `place_below`，两者不得互相翻案。

## 2026-10-22 DOM-on-top 有界探针（默认产品安排不变）

- 新 opt-in：仅 `YACR_PROBE_DOM_ON_TOP=1` 且显式 `--probe`，保留 marker/真实输入准入。
  原生 Wayland（Mutter/P620/scale1）三张采样合成帧的遮挡、混合、弹窗跨边界、标记运动由 Observer
  判 PASS；**XWayland 合成 FAIL**：透明区域透出另一 CAD 应用（PID3186773），不是自身 marker。
- native sibling 排序成功不是合成成功；X11 机制未完全诊断，不能推导 X11 不可能。ADR0010 增补
  明确“禁止 DOM 覆盖 CAD”是当前产品安排，不是 Linux 必然；**不构成生产迁移/联合替代验收**。
- Rust160、check/build、探针前端6、前端 build/i18n 通过；本次 debug 实际使用 Vite1420 服务的
  最新夹具而非嵌入 dist。真实输入/直接诊断对象回读、全 workspace 门禁 NOT RUN；最小化恢复与
  实际 resize 仍有限制。X11 attempt2 最终独立 Observer 确认 FAIL（取代 LIMITED），预期 marker/grid
  缺失，运动 INDETERMINATE；具体 clipping 根因未定，最终集成由父会话完成。
- 详见 [验证记录](validation-dom-on-top-linux-2026-10-22.md)；本机证据
  `/tmp/opencode/yacr-dom-on-top-20261022T043353Z/`（含 `RESULTS.txt`、`x11-attempt2/FINDINGS.txt`）。
  整屏截图仅本地、不上传；未解锁或改权限，只清理本轮进程。

## 2026-10-22 GUI 验收（有界）与 `YACR_PROBE_UI_DIALOGS` 证据驱动

### 1. 新增证据驱动（默认 OFF）
`apps/app-linux/src/evidence/ui_dialogs*.rs`：`YACR_PROBE_UI_DIALOGS=1` 时按时间轴用
`webview.eval` 走**与真实 UI 相同**的 JS 路径——`Ctrl+1` 的 `KeyboardEvent`（`App.jsx` 的全局
keydown 分支）、命令行输入 + Enter 提交（`CommandLine.jsx` 的 onChange/keydown 路径）、画布
`PointerEvent` 拾取（复用 `tool_click_world_js`）、`OPTIONS` 由 chrome 在路由前拦截、弹窗经
`cad_open_popup`/`cad_close_popup` 真实路径。每步输出可 grep 的 `[probe] ui-dialogs: t=<n>s …`，
汇总行明确 `ui_dialogs_trigger=probe_driver_not_a_real_click`。24 个契约测试。

### 2. 本轮实测（真实 GPU：Vulkan + Quadro P620，1853×1011 最大化，`synthetic-four-lines.dwg`）
- **主界面**：chrome（ribbon 中文页签、状态栏、命令行）与 CAD 子窗同窗渲染正常。
- **属性面板**（`Ctrl+1`）：分组 **对象 / 几何 / 显示**；图层/线型/线宽为下拉、透明度为数字输入、
  颜色为文本输入，只读值是纯文本。选中一条 `AcDbLine` 的行值与 `cad_status` 一致：
  `id=2、句柄 31、类型 AcDbLine、图层 0 (0)、空间 model、绘制顺序 2、颜色 aci:7、线型 ByLayer、
  线宽 ByLayer、透明度 0`。
- **命令行回显**（用户最初的痛点）：`命令: DIST / 距离 = 3.1623 / 角度 = 18.4349° / X 增量 = 3.0000 /
  Y 增量 = 1.0000 / Z 增量 = 0.0000`；数值正确（√10、atan2(1,3)），探针日志同时记录
  `algorithm=distance value=3.1623 points=(0,0,0);(3,1,0)`。
- **选项弹窗**：720×560，页签 显示/草图与选择/打印与发布/文件/用户偏好；显示页含模型/图纸背景
  色板、十字光标 %、栅格/UCS 图标/高亮选中对象/捕捉标记；底部 `尚未修改 / 取消 / 应用 / 确定`。
  弹窗打开期间主窗口模态门禁生效（`popup=options:popup-1:true`）。

### 3. 环境事实（诚实记录）
- **uinput 与 XTEST 注入进不了本 Wayland/XWayland 会话**：指针不动、按键无效（虚拟设备虽创建、
  udev 已打 `ID_INPUT`，Mutter 不认领）。只有**真实 evdev 写入**（root `evemu-event` 到
  `/dev/input/event3`）能产生活动。
- **Mutter 空闲变暗后 WebView chrome 变黑**（帧时钟被掐），CAD 子窗口不受影响；这与此前
  "chrome 全黑"是同一现象。`org.freedesktop.ScreenSaver` 的 Inhibit 对 Mutter 无效。
  为完成本轮取证，已把 `org.gnome.desktop.session idle-delay` 从 **120** 设为 **0**
  （**改动了会话设置**，恢复：`gsettings set org.gnome.desktop.session idle-delay 120`）。
- `x11-window-grab` 取目标窗口自身像素；黑帧根因是变暗，不是布局或 z-order。

### 4. 仍未验证
选项弹窗内**交互**（改值/应用/取消/写失败标记）只有纯模型与源码扫描测试；属性面板的
**编辑提交与拒绝**同样未在 GUI 中点过；多选（2 个以上）面板形态、语言双窗口传播仍无 GUI 证据。
以上均为 NOT RUN，不得据本节声称已验证。

## 2026-10-22 批次：选项弹窗、属性面板、拾取框/孔径、属性行 + IPC

（接上一批；上一批的"仍未做"清单以本节为准）

### 1. 属性显示行 + 编辑 IPC
`color/linetype/lineweight/transparency` 四行进入 `Display` 组，值取 `property_value` 投影
（**保证原样回写**）；`lineweight` 的下拉选项与写侧共用同一份 `lineweight_options()`；
`editable = property_row_editable && property_value::accepts`（`ByBlock` 透明度、非规范线宽这类
无语法可表达的值**显示为只读**而不是假编辑器）。IPC `cad_set_selected_property {key,value}` 进
队列（有界、owner tick 消费），**受模态门禁**——与 `cad_set_selected_text` 不同（弹窗自己的
通道必须在弹窗打开时可写），两条通道各有测试钉住。

### 2. 拾取框 / 孔径
`view.pickbox.sizePixels`（1..=20，默认 3）与 `view.aperture.sizePixels`（1..=50，默认 12），
**拒绝而非截断**。`ViewConfig::tolerance_policy()` 是**唯一**容差来源，hover / 点选 / 光标拾取框 /
捕捉四条路径全部改读配置；默认值**位级不变**（3 px → 1.5 世界单位、12 → 今日捕捉半径）。
实时生效（下一次拾取即用新值），open/换图后重新播种。**未做**：名词-动词选择、夹点、AutoSnap
颜色/磁吸、极轴、`APERTURE`/`APBOX` 命令面。

### 3. 选项弹窗（AutoCAD 风格，独立窗口）
`PopupKind::Options` + `options.mjs`（纯模型）+ `OptionsDialog.jsx`，720×560，左侧页签轨 +
右内容列 + **Apply/OK/Cancel**（本地暂存、只发 diff、失败标字段并用 `options.error.<code>` 文案）。
页签与控件**只有真后端**：Display（背景×2、十字光标 %、grid/axes/selectionHighlight/snapHints）、
Drafting & Selection（snap、selection preview、拾取框、孔径）、Plot and Publish（默认格式）、
Files（字体目录，只读 + 角标）、User Preferences（语言）。wire 里不存在的字段显示 `—` 并标
"Not reported by this build"，**不 dirty、不写**。语言写 `yacr.locale`，主窗口靠 `storage` 事件 +
聚焦重读同步——**双窗口传播 NOT VERIFIED**（无 GUI 会话）。入口：File 页、命令行 `OPTIONS`
（Chrome 本地拦截，**不进** Rust 路由）、状态栏齿轮（原 disabled 占位）。

### 4. 属性面板
`PropertiesPalette.jsx` 重做 + `properties-view.mjs`（分组/编辑器选择）+ `properties.mjs`（写语法
与拒绝码映射）。分组固定 `entity → geometry → text → display`，组内保留后端行序；只读行是纯文本
（无 disabled 控件）；`enum` → select、`number` → 数字输入（透明度 0..=90 **拒绝**）、`text` → 文本
输入（颜色/图层做客户端语法镜像）。**本地暂存 → Enter/失焦/下拉即提交**；拒绝则回滚并显示
`properties.edit.*`，code 只进 `title`；混合行以 `properties.mixed` 占位且写入作用于**整个选中集**
（有 help 文案）。`Ctrl+1` 与命令行 `PROPERTIES` 开关面板，ribbon 两个灰项改为可用（`LAYER` 不动）。
未确认的写入显示 `properties.edit.unconfirmed`，**不伪造成功**。

### 5. 门禁与证据边界
已运行：`fmt`/`clippy -D warnings`/`check --workspace --all-targets`、架构(27)/文件大小(807)/
i18n(685)/fixture/workflows；Rust **2175 passed / 0 failed**（CPU crates）+ **550 / 0**（GPU crates
串行）；UI **395 passed**，`npm run build` OK。
**未运行**：GUI/真机视觉验收、真实 DWG、默认并发的 GPU 测试（lavapipe 既存 SIGSEGV，串行通过）。
选项弹窗与属性面板的**真实窗口行为没有像素或交互证据**，只有纯函数模型、源码扫描与宿主契约测试。

## 2026-10-21/22 批次：回显、属性、选项、语言、MLeader 宽度、字体回退、背景/光标

### 1. 命令行回显（AutoCAD 式执行记录）
`cad_status.commandLog`：有界 **200** 条、`sequence` 单调、**换图即清空**；条目三类
`command|result|error`，**结构上没有任何 message/text 字段**——只有稳定键 `command`、错误
`code`、以及类型化 `measurement{algorithm,value,unit,precision,points}`。`DIST` 是**交互式选择器**
（无 `CommandId`），稳定名只能在路由层记，故 `CadSession::command` 走 `HostCore::record_command`。
UI（`command-log.mjs` + `CommandLine.jsx`）：4 位小数；`distance` 用拾取点现算 `Angle`（XY，
`[0,360)`）与 `Delta X/Y/Z`；点不足两个**只打印距离**（不补造点）；`unit:null` 不加后缀；
`precision` 仅非 `analytic` 时以淡色小标提示；错误码只进 `title`。**没有"清空"按钮**（宿主无 API）。

### 2. 属性面板后端（读 + 写；UI 未做）
读：每行 `group`（entity/geometry/text/display）、`mixed`、`editable`、`kind`、`options`
（`layer`/`linetype` 取图纸真实表名，上限 256）；多选取**共同键的交集**，值全同给值、不同给
`mixed`（`mixedKeys` 由 mixed 行派生，单一来源）。单选 48 行的顺序/值**钉死不变**。
写：`CommandId::SetEntityProperty` + `CommandPayload::EntityProperty{key,value}` →
`SETENTITYPROPERTY`；**一个事务/一步撤销**，全有或全无；同时更新实体行与 `EntityDisplayState`，
并把 `STYLE` 并入 `ChangeSet` 让依赖失效重建；无变化**不产生撤销步**（`property.unchanged`）；
实例路径/子元素**拒绝**。值语法：`layer`=表名；`color`=ByLayer/ByBlock/`aci:0..=256`/`#RRGGBB`
（ACI→规范 sRGB 表，反向取**最小索引**）；`linetype`=表名/ByLayer/ByBlock；`lineweight`=规范毫米
列表（`0.37` 拒绝、不吸附）；`transparency`=0..90 百分比 → 存不透明度（只换算一次）。
**未做**：Tauri IPC、面板 UI、`Ctrl+1`、`PROPERTIES` 命令打开面板。

### 3. 选项设置（类型化读写；弹窗 UI 未做）
`CadSettings` = grid/axes/selectionHighlight/snapHints/plotFormat/snapEnabled/hoverPreview/
backgroundModel/backgroundPaper/crosshairPercent + 只读 fontsDir；写键是**封闭枚举**，未知键/坏值
返回稳定码且**不动文件**。持久化走既有 `preference.json`（允许路径只加了这 12 个叶），
**实时生效**（`sync_render` 重发 `ViewSnapshot`/`OverlayInputs`），并在 `load_startup`/换图后
**重新播种** `SessionState`。`cad_settings`/`cad_set_setting` **不受模态门禁**——否则选项窗自己
无法保存。
**诚实边界**：AutoCAD Options 里自动保存/.bak/xref/CTB/GPU 选择/快捷键自定义/右键/夹点/极轴/
动态 UCS/口令 **在本仓库没有任何行为可接**，一律**不显示**。

### 4. 语言默认（系统 → 英文）
顺序：`localStorage.yacr.locale`（合法成员才算）> **宿主注入** `window.__YACR_HOST_LOCALE__` >
`navigator.languages` > `en`。宿主解析唯一来源：`YACR_LOCALE`，否则 `LC_ALL > LC_MESSAGES >
LANGUAGE > LANG`；注入用 Tauri `initialization_script`（document-start），主窗口与**弹窗**都注入，
因此界面与原生对话框标签**不会再分叉**。映射：`zh*/zh-CN/zh-Hans → zh-CN`，`en* → en`，
**其余（含 `zh-TW`/`zh-Hant`）→ 英文**——仓库没有 Hant 目录，不静默给简体。`index.html` 的静态
`lang` 改为 `en`，运行时再按解析结果写回。

### 5. `MLeaderText.width` 参与排版
`width`（DXF 43）此前死在 `text_geometry()`；现在 `SemanticGeometry::Text` 新增
`wrap_width: Option<f64>`（`wrap_width_from_dxf`：正有限值才 `Some`，0/负/非有限 = `None`），
表示层 `layout_parsed` 用既有 `measure` 闭包做**贪心换行**：拉丁按词、CJK 按字、`\P` 段落独立、
超宽不可断词独占一行、对齐按每行、`None` **位级不变**；`WRAP_EPSILON=1e-9` 只吸收浮点松弛。
复制/变换（db transform、TEXTEDIT、dimension、geometry engine）**保留并按世界长度缩放**。
`Partial` 声明：无 UAX #14/kinsoku、跨 token 字距不测、MTEXT 41 组仍不导入、包围盒仍用未换行估计。

### 6. 字体回退按图纸代码页
`$DWGCODEPAGE` → `DrawingDatabase::code_page` → 单一映射表 → 语言候选链（中文
`gbcbig/hztxt/chineset/bigfont/simhei/simsun` 前置，拉丁三件不变）；同一语言且字体齐全时**复用**
引擎，换语言就地 `with_fallback` 重定向，否则按新语言重载。缺面时每文档**一条**
`font.locale_fallback_missing`（带语言与候选），CJK 字符给 `text.glyph_missing`。
`BigFont` 能力从 `NotImplemented` 改 `Partial`。**本机 `fonts/` 无 CJK SHX ⇒ 真中文图纸会明确报缺**。

### 7. 显示设置（背景色 / 十字光标）
`view.background.model|paper`（严格 `#rrggbb`）与 `view.crosshair.sizePercent`（1..=100，**拒绝**
而非截断）；渲染器 `RenderTarget::clear_color` 承接（`CAD_CLEAR_COLOR` 仅作默认），遮罩底色跟随
背景；sRGB→linear 在配置接缝完成，**真机核对** `0.5` 读回 `188` 且遮罩内部同值；100% 光标**位级
不变**，50%/1% 有钉死数值。全部**即时生效**（不是下次打开）。

### 8. 验证与证据边界
已运行：`cargo fmt/clippy/check`（全 workspace，`-D warnings`）、架构/文件大小/i18n/fixture/
workflows 脚本；Rust 测试 **2144 passed / 0 failed**（CPU crates）+ **550 passed / 0 failed**
（GPU crates 串行）；UI **334 passed**，`npm run build` OK；i18n **607** 键。
**未运行**：GUI/真机视觉验收（本批**没有**任何截图或像素判据）、真实 DWG、全 workspace 默认并发的
GPU 测试（`cargo test -p cad-render-wgpu` 在 lavapipe 下有**既存** SIGSEGV，干净 HEAD 可复现，串行通过）。

## `draw_order` 贯通到场景批次（2026-10-21，行为中性）

**做了什么**：`DisplayFragment` 新增 `draw_order: i64`，由源 `DbEntity::draw_order` 携带；
`cad-scene` 的 `RenderBatch`/`MaskBatch` 直接取其值，`SCENE_DRAW_ORDER` 删除。**合批不跨顺序**：
行/圆锥的合批谓词要求 `batch.draw_order == incoming`（同序内仍以 upload 顺序为最终 tie-break）。
`ImageBatch` 仍为 0，并在注释里**具名**为图像遍的后续项，不再是静默默认。

**INSERT 语义（明确写下）**：块展开的子实体继承**所属 INSERT 实体**的 order（每层取最近的 INSERT），
不用子实体在块定义里的自有 order，也不给合成的默认值——因为数据库是按 INSERT 排序的。

**测试**：实体 order 被携带（含 WIPEOUT 遮罩）；INSERT 子实体继承；同序可合批、**异序不可合批**；
合成夹具 `0/1/3` + 遮罩 `2` 得到 `delta.added` 顺序 `[0,1,3]` 与 `delta.masks` 的 `2`。

**行为中性**：`cad-render-wgpu` **未改动**，所以遮罩**仍然最后一遍**、绘制仍按上传顺序。
ADR-0022 的"遮罩恒最后"声明**保持不变**，但理由已从"数据在表示层被丢掉"改为
"渲染器分段尚未实现"；另外 **opaque mesh 仍跨批次拍平并忽略顺序**（`plan/pack_mesh.rs`），
这是分段的前置条件之一，已在 ADR 里具名。文档已同步（`docs/adr/0022`、`docs/compatibility.md`）。

## 3D 视图的 GUI 面板（2026-10-21）

**做了什么**：`view` 标签新增两个面板（复用**既有** i18n 键，未新增文案）：
- **三维视图**：`二维 / 三维`（大按钮，`SWITCH2D3D`）、`投影`（`PROJECTION`）、
  `环绕`（**可见但禁用**——orbit 是画布拖动，按钮只能伪造负载；理由用 `view.orbit.gesture`）。
- **标准视图**：俯/仰/前/后/左/右/等轴测七项，各自经新 typed IPC `cad_standard_view` 派发
  （渲染器**不**把 `standardView` 当 `command` 走 `cad_command`）。
同时删掉了「导航」面板里**永远只能禁用**的旧标准视图下拉（其三个变体既无 `command` 也无 `action`）。
开关**不**声称状态读数（`cad_status` 没有 3D/投影状态），所以没有用 `view.2d/3d` 之外的伪状态标签；
无图时按既有机制禁用并给"没有打开图纸"的理由。

**验证**：`npm test` **293 passed / 0 failed**（原 286，新增 7 条 + 扩展既有模型/i18n 测试）、
`npm run build` 通过、`check-i18n` 578 键不变、无硬编码 CJK。

**像素判读**（休眠前的真实抓图，`/tmp/opencode/view3d-ui-check/`）：
带区 y 56–147 完整绘制、无黑洞；`环绕` 图标与文字**同为禁用色 `#98A3B4`（对比 3.85:1）**，
`投影` 为 `#C3CCDA`（6.07:1）⇒ 是"刻意禁用"而不是"坏掉"；七个标准视图均为启用色 `(191,200,214)`；
View 标签 960 宽**不溢出**（中文右缘 734、英文 848）。视觉语言与既有 ribbon 的 token/间距一致。

**判读发现的真问题（未修，cosmetic）**：
1. **常用（Home）标签在 960 宽溢出并切断字形**（墨迹到 x=959，无省略号、无可视滚动条）；
   View 标签不溢出，但英文只剩 111 px 余量，窗口再窄到约 850 px 会同样截断。最大化宽（1853）不存在。
2. 标准视图按**列优先**排布，语义配对 `前/后`、`左/右` 被拆到不同列；6 个面视图图标在 1× 下
   仅靠 3–4 px 短线区分（有文字兜底，未发现错标）。

## ⚠ 证据环境边界：显示器休眠会让 WebView 呈现停止（2026-10-21，非代码缺陷）

**现象**：某时刻起窗口抓图里 chrome 全黑（顶层抓图 5 色），但 DOM 正常（73 按钮、`errors: []`）、
WebView 窗口 960×600 `mapped=true native_viewable=true`、WebKit 进程存活、`/dev/shm` 正常；
`WEBKIT_DISABLE_COMPOSITING_MODE=1` 无效。

**已反证（含对照实验）**：
- 怀疑 `reparent_webview` 的 `widget.unrealize()`：**去掉后 6 次启动仍是 5 色**（尺寸/mapped/viewable 不变）⇒ 排除；
- 临时跳过 reparent：仍 1 色 ⇒ 排除；
- **完全不装 yacr 的原生 GTK3 + WebKitGTK 4.1 窗口**（20 行，加载普通 HTML）：同样不绘制，
  `gtk_widget_add_tick_callback` 探针 **`ticks=0`（7 秒）** ⇒ **GDK 帧时钟死了**；
- Mutter：`DisplayConfig PowerSaveMode = 3`（显示器已省电关闭）、`IdleMonitor GetIdletime ≈ 4,166,272 ms`（约 69 分钟）、
  `ScreenCast CreateSession = "Session creation inhibited"`；时间线吻合：13:52:26 最后一次正常抓图（1759 色）→ 14:02:14 首次全黑。

**结论**：WebKitGTK **只在帧时钟 tick 上呈现**，显示器休眠时没有 vsync/帧回调 ⇒ chrome 不画；
CAD 子窗口照画是因为我们**自己推 Vulkan 帧**，与 GTK 时钟无关。**这是环境状态，不是 `apps/app-linux` 的代码问题**，
也没有把任何 `WEBKIT_*` 绕过手段写进宿主（那只是遮盖）。
**复验方式（需人唤醒显示器）**：`/tmp/opencode/yacr-chrome-loop.sh 6 0` 期望 6/6 约 1750–1990 色；
`yacr-chrome-loop.sh 1 1` 为最大化。若显示器已唤醒（`PowerSaveMode` 为 1、idle 很小）仍 ≤5 色，
则帧时钟假设不完整，`unrealize()` 的问题需要重新检验——工具已就绪。

**对证据的影响**：休眠期间**无法**产出新的 UI 截图；上文的 ribbon 判读用的是休眠前（13:52）的真实抓图，
同一份 UI 代码（此后 UI 未再改动）。CAD 内容证据（真实 GPU headless 读回 + CAD 子窗口抓图）不受影响。

**唤醒后复验（已完成，结论成立）**：显示器唤醒后 `IdleMonitor` 空闲时间从约 4,166,272 ms 降到 **9,421 ms**，
同一脚本 `/tmp/opencode/yacr-chrome-loop.sh 6 0` 连续 6 次启动的结果为
**1825 / 1826 / 1826 / 1825 / 1826 / 1826 色（6/6 呈现）**，每次 `webview size` 均为
`requested=960x600 ... native_viewable=true mapped=true`；**最大化默认配置 1853×1011 亦呈现（1739 色）**。
即：全黑由**显示器省电导致的 GDK 帧时钟停摆**引起，**不是** `apps/app-linux` 的代码问题，代码行为是确定的。

## 主光偏离视轴：等轴测下三面可辨（2026-10-21，接上一条）

**上一条的发现被证实并修掉**：相机相对**纯头灯**（`light = normalize(eye - target)`）在**等轴测**下
对三个可见面的 `n·L` **完全相等**（对称），实测三面填色同为 `(215,215,215)`，只靠白边区分——
最常用的 3D 视角反而读不出面。

**修法**：`L = normalize(toward_eye + KEY_LIGHT_RIGHT·right + KEY_LIGHT_UP·up)`，
`KEY_LIGHT_RIGHT = -0.10`（屏幕左）、`KEY_LIGHT_UP = 0.35`（上），即视线**左上方约 20°**；
`right/up` 取自 look-at 用的同一组相机基向量。2D 仍是硬编码 `(0,0,1)`。

**实测（真实 GPU，universal 与 accelerated/pulled 两层一致）**：

| 视角 | 面 | 纯头灯 | 偏移主光 |
|---|---|---|---|
| 等轴测 | +X / +Y / +Z | 215 / 215 / 215 | **204 / 189 / 238** |
| 斜视 az 27° | -Y / +X / +Z | 233 / 194 / 210 | **218 / 169 / 235** |
| 斜视 az 62° | -Y / +X / +Z | 196 / 233 / 210 | **193 / 208 / 235** |
| 俯视（3D） | +Z | 255 | **250**（仍最亮；背向环境底仍是 137） |

- **第二方位对照仍成立**：`-Y` vs `+X` 的明暗分配随方位翻转（27°：218 vs 169；62°：208 vs 193）。
  因为主光在视轴上方，`+Z` 在两个方位都是最亮 ⇒ 原来的"正面 > 顶面 > 侧面"断言**按设计改为**
  "顶面 > 正面 > 侧面"，并注明**最亮面不再可能全局翻转**（这是偏移主光的必然结果，不是退化）。
- **2D 位级不变**：两层 256×256 RGBA dump 前后 sha256 相同（`adbe87cd…`）。
- **真机 orbit 帧恢复干净**：并行 lane 发现拾取相位把实体**选中**后抓帧变成选中色 `(255,196,97)`，
  已把拾取相位门控为独立 env `YACR_PROBE_VIEW3D_PICK`（默认关）；`YACR_PROBE_VIEW3D=1` 现在**一次点击都不发**，
  抓到的帧三面为 **254 / 137 / 157**、选中色 **0 像素**；开 `..._PICK=1` 时三次选择计数仍为 1 / 0 / 1。

**边界**：仍是显示约定（无阴影/高光/能量守恒声明），反照率与曝光不可分离；斜视下某个侧面可能落到
环境底（`137`），这是"背向光"的正常结果，不是伪影。

## STANDARDVIEW 通道 + 第三个 3D 入口的取景缺陷（2026-10-21）

**为什么**：`STANDARDVIEW` 需要 `CommandPayload::StandardView(view)`，而 `admit_command` 只排队**名字字符串**，
带载荷的变体一律被拒 ⇒ 这个选择器**从任何 UI 都不可达**（`PROJECTION` 无载荷，命令行本就可达，实测确认）。

**做法（遵循项目既定模式，不在 `cad_command` 里解析文本）**：专用 typed IPC
`cad_standard_view { view }` → `parse_standard_view`（`top/bottom/front/back/left/right/isometric`，
大小写不敏感；未知/空值**具名拒绝**）→ 独立有界队列（门禁顺序与 `admit_command` 一致：
弹窗模态 → CAD 关闭 → 队列满）→ 宿主 tick 排空 → `CadSession::standard_view` →
真实 `CommandId::StandardView` + payload。

**顺带修掉的第三个入口缺陷**：`apply_standard_view` 是继 `SWITCH2D3D`、`PROJECTION` 之后的**第三个**进 3D 的入口，
它**从不调用 `enter_3d`**，直接复用 2D 平面的固定 **1000 世界单位**眼高：
对 2 单位大的立方体是**约 2 px 的斑点**，对 4000×3000 的图则把眼点放进模型内部。
现在：从平面进入时按文档包围盒取景（与 `SwitchProjection` 同规则），**已在 3D 时保留当前距离**
（重新定向不该重置用户缩放），平面视图**位级还原**保存的 2D 相机（此前会把正交比例重置成 `DEFAULT_ORTHO_SCALE`）。

**证据**（真实 GPU，X11/XWayland，P620 + Vulkan，`pulled=active`）：
- `YACR_PROBE_VIEW3D_STANDARD=isometric`（**零合成输入**，纯 IPC）：
  `ack={accepted:true}`、`camera` present→absent、`error` 全程 null；CAD 帧非背景 47,827 px、
  bbox 264×273、六边形填充比 0.634 ⇒ **等轴测视图真的被取景**，不是斑点。
- orbit 回归（原 `YACR_PROBE_VIEW3D`）报告行不变：`camera present→absent`、选择 1/0/1、35 次拖动移动。

**新发现（仍未修，下一步）**：等轴测帧里**三个可见面填色完全相同**（`215,215,215`），只靠白边区分。
原因不是 bug：相机相对头灯在等轴测下对三个面的 `n·L` **本来相等**（对称）。
但作为显示方案它让"等轴测"这个最常用的 3D 视角**读不出面**——需要把光源**偏离视轴一个固定角度**
（如向上偏 20°、向左偏 20°），仍是相机相对，但等轴测下三面会有不同明暗。

## 更正：3D 拾取本来就已经可用（2026-10-21）

**背景**：我上一轮把"3D 不可拾取"当作缺口，据此准备改代码。**读码后先做了测量，结论是我错了**：
`cad-host` 的 hover（`input/hover.rs:178`）与点击选择（`input/interaction.rs:404`）都调用
`cad_app::picking::pick_at_screen_with_visibility`，它内部走 `Camera::screen_to_ray` → `pick_ray`；
`pick_tolerance`（`picking/mod.rs:107-127`）对 **透视投影**按 `depth × FOV` 推出世界/像素。
既有单测 `perspective_pick_hits_3d_geometry_along_the_view_ray` 也早就在。

**真机证据**（`YACR_PROBE_VIEW3D` 扩展了拾取相位；X11/XWayland + P620 + Vulkan，跑两次一致）：
- 2D 对照点击画布 (463, 153.45) ⇒ `selection_count_2d = 1`；
- 切 3D → orbit → 先点**空白**（95%/95%）⇒ `selection_count_3d_empty = 0`（**排除残留选择**）；
- 再点同一点 ⇒ `selection_count_3d = 1`，同一 `id=1 handle=31 type=AcDb3dSolid vertices=24 triangles=12`；
- 三次点击 `error` 全 `null`，`entityCount` 恒 1（选择不改图）。

**限制（如实记录）**：`cad_status` **没有** 3D 相机、**没有** hovered-entity 字段 ⇒ 3D hover 只能用
`hoverPreview`/`snapKind` 观测，orbit 只能用拖动像素观测；报告里显式写了 `hover_channel`/
`orbit_angle_channel` 两条"通道缺失"说明。GPU ID pass 是另一条路径，宿主当前未用。

**文档更正**：`docs/view-3d.md` 与 `docs/render-3d.md` 里"无 ray/scene 求交、无 PickHit 选择、
仍待上层"的说法已按上述证据改写（那两句是过期断言，不是设计决定）。

## 实体着色：3D 改为相机相对头灯（2026-10-21）

**症状**：斜视立方体的两个侧面**同为 137 = sRGB(0.25)**（环境光底），多面读不出来——因为着色的
`diffuse = dot(n, +Z)` 用的是**世界固定**头灯；侧面的 `n.z = 0`，于是全部落到环境光。几何没错，是**约定不适合 3D**。

**改动**：共享 `Camera` uniform **96 → 112 字节**（末尾加 `light: vec4<f32>`，rgb = 指向光源的单位向量、
a = 0），**13 个 `struct Camera` 的 WGSL** 同步同位置加字段；`uniform_bytes` 与全部调用点、`UNIFORM_STRUCT_BYTES`、
批处理计划与尺寸断言一并更新。**关键发现**：加速层（vertex pulling）**不绑 `Camera`**，它有自己的
`PullCamera` + `mesh_pulled.wgsl`，而**宿主实际选的就是该层**（`pulled=active`）——所以 `PullCamera` 也加了
`light`（64 → 80 字节），否则"改了也没改"。

**语义**：2D 传 `(0,0,1)`（`dot(n,light) == n.z`，**位级不变**）；3D 传 `Camera3d::headlight() =
normalize(eye - target)`。环境 0.25 / 漫反射 0.75 不变，无高光/阴影/环境光。

**证据**：
- **2D 位级相同**（实测，不只推理）：临时探针对 universal 与 accelerated 两层各导出 256×256 RGBA，
  改动前后 `cmp` 完全一致（sha256 `d38e7608968a8ae8…`，262144 字节）；`solid_fill` 的期望值**未改**且通过。
- **3D 三档面明暗**（真实 GPU，斜视）：`248 / 184 / 177`（此前 255 / 137 / 137）。
  **像素判读**独立验证：三面顺序与相机轴一致（面法向偏离视轴 24.3° / 75.4° / 71.0° ⇒ 越正对越亮），
  面内**完全平坦**（95% / 92% / 73% 为单一值，无渐变/条纹），六边形轮廓不变，无伪影；
  反解点积 (0.918, 0.306, 0.253)、平方和 1.0006，**排除了世界 Z 固定灯**（会给 137/137/255）与
  **逐像素眼点头灯**（会给 243/145/158）。
- **灯随相机变化**：单帧只能证明"有一个沿 eye−target 的恒定方向灯"；"随 orbit 重跟踪"由 **headless**
  的第二方位对照证明（方位 27° → `-Y/+X/+Z = 233/194/210`；方位 62° → `196/233/210`，最亮面由 `-Y` 换到 `+X`），
  外加 `oblique_cube_shading_follows_the_camera` 守住。

**诚实边界**：两个侧面在当前方位只差 **7 级（ΔL\* 2.6）**——"可分辨"但**不宽裕**，同向面仍可能接近；
不是物理光照，无阴影/高光，反照率与曝光不可分离；不能据此推断立方体比例或 ACIS 源。

## 3D 观察：斜视证据闭环（2026-10-21）

**为什么开这一轮**：ACIS 实体在 2D 俯视下是"填色方片"，与"有面立方体"**像素等价** ⇒ 上一轮只能证明"有填充"，
不能证明三维性。本轮补上斜视证据，并在此过程中修掉两个真缺陷。

**可达性（重要，且未变）**：`SWITCH2D3D` 是**无负载**选择器、在 `cad-ui-core` 命令表内，所以
**命令行输入即可进入 3D**；3D 下**主键拖动即 orbit**（`cad-host/src/input/pointer.rs`，0.008 rad/px）；
`ORBIT` 在 `apps/app-linux/ui/src/commands.mjs` 的 `GESTURE_ONLY_COMMANDS` 中，**按设计不提供按钮**。
**React chrome 没有 3D 控件**，`STANDARDVIEW`（需 view 载荷）与 `PROJECTION` 目前**没有任何通道可发**
⇒ 标准视图/投影切换**不可达**（声明缺口，非静默）。

**修掉缺陷 1：进入 3D 不设距离**。2D 俯视相机的 eye 偏移不是模型量级
（`FitDrawing` 还刻意保留 `eye.z`），而 `Viewport::set_view_mode` 只换投影、不动距离 ⇒ 眼点落在实体
附近/内部，近平面把立方体切掉，实测只剩 **4×3 px、8 个非黑像素**的残块。
修法：`Camera::enter_3d(bounds, fov)` —— target 取**文档 3D 包围盒中心**（含 z），
`distance = radius / tan(fov/2) * 1.2`（radius = 包围盒对角线一半），**保持视线方向与 up**；
空文档/退化范围回落到有限正值。`SWITCH2D3D` 与 `PROJECTION` 两个入口都走它，`saved_2d` 位级还原不变。
cube 夹具：radius 1.7320508 ⇒ distance 5.0178487，可见高度 4.1569219 ≥ 对角线 3.4641016。
**2D 帧位级不变**（sha256 相同）。

**修掉缺陷 2：orbit 俯仰轴取反**。`Camera::orbit` 的俯仰轴原为 `horizontal × +Z`，即**屏幕右向的反向**；
宿主每次 pointer move 发一条增量 `Orbit`，于是**抬一次又压回去**——36 次移动后眼点只偏离 +Z
**0.0000–0.057°**（应为 23.5°），画面是顶面被滚转后的**纯白正方形**（旋转 52°、对角线等长、零内边、零明暗）。
改为 `+Z × horizontal`（= 视图基的 right）后增量正确累积。新增
`orbit_from_plan_tilts_the_eye_by_the_pitch_angle` 与 `incremental_orbit_accumulates_the_pitch`（36 次增量仍为 0.41 rad）。

**证据（真实 GPU：Quadro P620 / Vulkan / X11，`YACR_PROBE_VIEW3D=1`）**：
驱动读**真实** `cad_status`（`camera` present→absent、`error` null、`downs=1/moves=36/ups=1`），
不合成角度；斜视抓图判读结果：
- **6 顶点凸六边形**（不是四边形：角点 C/E 分别偏离 4 顶点模型 13.3 / 4.4 px）；
- **三面**：顶面 255（36,176 px）+ 两个侧面同为 137（4,043 / 1,466 px），第 4 个顶面角**严格在内部**（立方体俯视拓扑）；
- **透视**：世界平行棱收敛 **31.2%**（282.95 → 194.75 px），顶面对角线差 **10.1%**、中点相距 17.2 px
  ⇒ 非仿射；Caprile–Torre 正交性检验通过（三组消失点给出同一 f = 417.1 px，正交误差 0.00007）；
- **遮挡**：隐藏边**不穿透**可见面（远角探针 0/300、0/200 非白），0 空洞、0 z-fighting、0 游离三角；
- **三角化**：两个侧面各有一条**角到角对角缝**（与预测差 ≤1.5 px）⇒ 侧面确实是 2 三角形。

**证据边界（判读原话，不得推广）**：顶面的对角缝在 255 上**不可见**，所以"12 三角形"整体**未被证明**；
不能从像素推断立方体比例（顶点残差 2.7–5 px）、**背面剔除 vs 深度测试**、光照模型、ACIS 源本身；
无 golden / 跨 GPU / vendor / 性能结论。**回归守卫**：headless `oblique_cube`（6 角、>100 内部边像素、
透视收敛）+ 反例对照 `top_down_cube_is_a_square_control`（4 角、上下等宽）。

## 正交深度映射修复：实体终于能填充（2026-10-21）

**症状**：ACIS 立方体只画线框，内部 100% 背景色；而 hatch（同一 mesh 管线）填充正常。

**根因（不是绕序/不是剔除/不是没发填充）**：正交相机的深度映射是**一比一**
（`clip_z = origin.z - center.z + z_plane`），而 wgpu 正交裁剪只接受 `0..w` ⇒ **只有平面上方 1 个单位**
在视锥内。`[-1,1]³` 的顶面落到 **clip z = 1.0 = 远平面**：mesh 用 `Less`（`1.0 < 1.0` 假）⇒ 填充全被丢弃；
线用 `LessEqual` ⇒ 线框照画；hatch 在 `z=0` 所以不受影响。**这是通用的既有缺陷**（任何 z>1 的实体都画不出面）。

**修法**：把世界 z 带映射到 `[0.05, 0.95]`（`OrthoDepth`），相机专用矩阵的 z 行从投影回读以防漂移；
批次的 `z_range` 在 upload 时记录、逐帧取并集。**方向约定**：世界 z 越大越近（2D 视图沿 `-Z` 看，外向 CCW 为正面）。
顺带修了 `render_effects` 里一个把深度轴写反的透明度场景。

**像素证据（真实设备 Vulkan / Quadro P620）**：修复前中心 `[0,0,0,255]`（clear）、对角线 `[148,148,148,255]`；
修复后中心 `[255,255,255,255]`、内部探针 `(24,32)/(40,32)` 同为纯白、外部角 `(2,2)` 仍为 clear。
**反例对照**：把映射临时改回一比一 ⇒ **两条新像素测试都失败**（"top face did not rasterize the centre pixel"）。

**窗口级证据（ACIS 夹具，真实窗口）**：`entityCount=1`、`error=null`，画面上是 **326×326.8 px 的纯白实心方块**，
居中误差 **0 px**、≈163 px/单位、无 z-fighting/接缝/摩尔纹。判读**接受**"实体已填充"，
并如实指出：**俯视正交下"有面立方体"与"填色方片"像素等价**，因此该帧**不能**证明三维性；
需要斜视/透视视图才能判 3D 与遮挡。

## ✅ 已解决：窗口抓图看不到 WebView（主 WebView 一�"1×1"、chrome 全黑，2026-10-21）

**根因（三条都可引用源码，不是猜）**：

1. **`add_child` 的 bounds 在 Linux 被丢弃**：Tauri 走 `WebviewKind::WindowChild` → `build_gtk(window.default_vbox())`；
   wry 的 `add_to_container` 对 `GtkBox` 只做 `pack_start(...)`，**从不读 `attributes.bounds`**。
2. **因此 `auto_resize()` 完全无效**：wry 的 `set_bounds` 只在"自有 x11 子窗口"或"`GtkFixed` 父容器"两情形生效，
   这里两者皆否 ⇒ 恒为 no-op；而 `reparent_webview` 之后 widget 被移出那个 `GtkBox` 变成 overlay child，
   **再没有任何东西给它定尺寸**。
3. **repaint 后 widget 从未被 map**：`Container::remove` 会 unmap 但保持 visible，`widget.show()` 对已 visible 的 widget 是空操作
   ⇒ `GdkWindow` 始终是 `1x1+-1,-1`（与 `xwininfo` 一致），**没有 chrome 像素**；WebKit 还会对未映射视图**节流**，
   这正是"合成开启时布局也停住（626 启动态）"的原因。

**修法**：`reparent` 前 `unrealize()`（在最终父级重建 `GdkWindow`）→ `map()` → `size_allocate(父分配)`；
**`reparent` 之后每帧 tick 都做一次幂等同步**（不依赖 GTK 信号是否触发，本宿主不主动 drain 队列布局）；
目标尺寸规则：**`1×1` 视为"尚未布局"**（回落到窗口内尺寸），真实小尺寸仍尊重，**永不为 0×0**；
"settled" 判定改为 **`allocation 匹配 && mapped && GdkWindow::is_viewable()`**（GTK mapped 但原生不可见 ⇒ 继续重试，
那正是黑窗状态）。**不设 `WEBKIT_DISABLE_COMPOSITING_MODE`、不伪造尺寸。**

**验证（真实 X11/XWayland）**：`[probe] webview size (tick): requested=960x600 allocated=960x600 native=960x600
native_viewable=true native_xid=0xc00021 mapped=true`；`xwininfo` 为 `960x600 IsViewable`；
窗口抓图 **1965–1992 色**（修复前 5–26 色）；**y<175 无一个纯黑像素**，各带精确平铺、1px 分隔；
CAD 分区恢复 **926×341**（文档已打开、左坞关闭）。回归守卫是 `grep 'webview size'` 的日志行。

**附带真机结论**：ACIS 立方体在该窗口内为 **326×327 纯白实心方块**、**在视口内居中到像素**
（窗口中心偏左 17px = 34px 导航条的一半，属正确）；判读**接受**"chrome 正确 + 实体已填充"，
并如实指出**俯视正交下"有面立方体"与"填色方片"像素等价** ⇒ 三维性仍需斜视图。

**判读附带发现（未修，cosmetic，且只在窄窗）**：960 宽下 ribbon 的"修改"面板溢出窗口，`镜`/`缩` 两个字被边缘切掉，
**没有溢出指示**。**默认最大化下不存在**：1853×1011 实测窗口/WebView 同步、1850 色、
ribbon 右缘均匀为背景色 `(59,68,83)`（无裁切）。证据脚本用 `--no-maximize` 才会看到，故不属默认配置缺陷。

## 显示保真切片 C2：`inverted` 遮罩补集（ADR-0022，2026-10-21）

- **不需要模板**：复用 C1 的 even-odd 填充，把**视口覆盖环**与**边界**当两个环一起做 ⇒ 边界内两次覆盖（不填）、
  外部一次覆盖（填）= 精确补集。**D5 的选定即"渲染器侧屏幕/视口环"，不是表示层世界补集网格。**
- **环的构造**：可见世界矩形（**正交精确**；**透视宽松估计**，边距 64）**∪ 边界 AABB** 再外扩 **25%** ⇒ 环**严格包含**边界，
  这是 even-odd 精确的前提；边界包围整个视口时**不抹可见区域**（正确）。首帧构造，视口仍在环内则**零重建**。
- **标志贯通**：图元 `Mask { boundary, inverted }` → 场景 `MaskBatch.inverted` → 渲染器；**删除了假的"未渲染"路径**。
- **真实设备像素证据**：倒置矩形 ⇒ 外部被抹 `[0,0,0,255]`、**内部线可见** `[187,187,187,255]`；倒置 L ⇒ 竖条可见、缺口被抹、外部被抹。
  **反例对照**：强制走非倒置路径 ⇒ 两条新测试**都失败**；C1 测试全程绿。
- **仍声明不做**：不与更高 `draw_order` 交错；矢量出图跳过并具名；纸张空间视口裁剪报 `UNSUPPORTED_PRIMITIVE`；
  **透视环是估计**；不可拾取；无 stencil/scissor/异步改造；无法三角化的边界经 `mask_skips()` 报告，**不近似填充**。
- **门禁**：crate 子集（含 `--test-threads=1` 的 GPU 套件）通过 / 0 失败、clippy、fmt、architecture(27)、i18n 578、file-size 774。

## 显示保真切片 C1：WIPEOUT 真遮罩（ADR-0022，2026-10-21）

**范围**：`cad-representation`（新图元/提供者/矢量出图跳过）、`cad-scene`（mask 批次）、
`cad-render-wgpu`（新 pass + 着色器 + 填充）、`cad-engine`/`cad-app`/`cad-cli-tools`（接线）。

- **为什么需要新 pass**：单 pass 深度测试下"后画一个背景色填充"会被**深度拒绝**（mesh `Less` 写深度、线 `LessEqual` 写深度）。
  故遮罩走**独立末 pass**：`depth_compare: Always`、不写深度、用 **clear 颜色**，必然盖住此前像素。
- **无颜色图元**：`DisplayPrimitive::Mask(Arc<[Point3]>)` 只带世界边界，**背景色留在渲染器**（表示层保持 GPU 无关）。
- **凹形正确**：复用 HATCH 的 **even-odd `fill_rings`**，不用三角扇（L 形缺口不泄漏）。
- **真实设备像素证据**（Vulkan Quadro P620，discrete，**非软件后端**）：矩形遮罩内像素 = **clear 颜色**、外不变；
  **L 形缺口内线存活**；`mask_draws == 1`。
  **反例对照**：临时禁用该 pass ⇒ **两条测试都失败**；恢复后通过。
- **`inverted` 未做（C2）**：保持边界折线 + `Partial(MASK_INVERTED_NOT_RENDERED)` + `representation.mask_inverted`，
  **明确未渲染**，不假装。
- **其它已声明**：矢量出图**跳过并具名** `vector.mask_unsupported`；不与更高 `draw_order` 交错（遮罩恒最后）；
  纸张空间视口裁剪保留并报 `UNSUPPORTED_PRIMITIVE`（不伪造裁剪）；遮罩**不可拾取**（未变）；无模板、无 scissor、无异步改造。
- **门禁**：crate 子集通过 / 0 失败（含 `--test-threads=1` 的 GPU 套件）、clippy、fmt、architecture(27)、i18n 578、file-size 771。

## 显示保真切片 B：ACIS 假声明改为真话（2026-10-21）

**结论：不接线，只修文档。** 闸门（"有界才实现，否则说实话"）判定**不可有界**，依据三条：

1. **数据在导入时就没了**：`acis_semantics` 发出 `SemanticGeometry::Opaque { type_key, version, payload }`，
   `payload` 只是**原始 SAT/SAB 字节**，不含 shells/faces/loops，也没有 `BrepData`。要把字节变成 `BrepData`
   必须经 **acadrust 的 `AcisData::parse()`/`SatDocument`**，而它只在 `cad-import-acadrust/src/solid.rs` 可达。
2. **分层禁止"顺手调用"**：`check-architecture.py:174-175` 断言**除 `cad-import-acadrust` 外任何包不得依赖 acadrust**
   （"parser escaped importer"）。`cad-representation` 在导入层**之下**，在那里拉 acadrust 会**反转层序**。
   正确做法是让**导入**把 `SolidExchange::Brep` 持久化到域模型 ⇒ `cad-domain` + 导入 + 表示 + 下游消费者，
   **是跨 crate 重设计，不是 ≤4 文件**。
3. **没有显示级预算/缓存**：场景装配确实在后台 `LatestTask` 里（不阻塞 UI 帧），但每次内容 revision 都会重建，
   显示路径没有 tessellation 缓存/容差/预算策略。

**保持不变的诚实状态**：`Opaque` ⇒ `Completeness::Missing("no display representation in this build")` + `representation.opaque`；
`DisplayRepresentation::from_tessellation`、`solid_exchange_from_entity`、`sat_to_brep` **只有测试调用者**。

**已更正的文档**（把"已实现"改成"渲染为空 + 存在但未接线 + 阻塞点"）：
`compatibility.md`（ACIS 行 + 新增「ACIS 实体显示状态」小节 + 两处三维/不可实现段落）、
`dxf-entity-coverage.md`（新增 ACIS 缺口行；§6"所有可绘制 EntityType 均已接线"补**显式例外**）、
`architecture.md`（ACIS 一行）、`kernel-acis.md`（`Unchecked` → `Unsupported`/`Missing` + 未接线说明）。

**这一片没有改变任何像素**——它去掉的是一个**假声明**。若要真的画出 ACIS，需要的是**域模型持久化 B-rep**
（另一项特性），不是接线。

## 显示保真切片 A：MESH 族停止谎报 + 渐变单位/旋转（2026-10-21）

**范围**：`cad-import-acadrust`（三种 mesh 的完整性判定）、`cad-domain`（渐变旋转）、文档。

- **两处假声明中的第一处已修**：`SUBD MESH` / `POLYFACE MESH` / `POLYGON MESH` 三种此前报 **`Completeness::Complete`**，
  实际丢掉了：**细分级别**（画的是基础笼而非光滑面）、**面边可见性**（负索引标记的隐藏边被 `unsigned_abs()` 抹掉后照画）、
  面颜色、闭合标志、`smooth_type`/密度、`elevation`/`normal`、`blend_crease`、per-vertex bulge/宽度/id。
  现全部改为 **`Partial` + 逐类具名理由**（`import.{subd,polyface,polygon}_mesh_partial`，按 `reported_reasons` **每次导入只报一次**），
  **几何逐字未变**（三角化不变，仅判定与理由变）。
  **值得记的事实**：**没有任何测试曾钉住那个 `Complete`** —— 假声明只活在 `entity/convert.rs:242-244` 三个分派分支里，
  而 `compatibility.md` 的 MESH 行**本来就写"部分"**，即代码与文档当时互相矛盾。
- **渐变组 460 = 弧度（已核实，U-13 关闭）**：DXF 参考、acadrust 字段文档（`Gradient angle in radians`）、
  以及新增的**方向回读测试**三方一致（30° 输入回读 30°±1°；若按度误解会得到 ≈0.52°）。截断保留：
  **仍无 vendor 产出的非零角度渐变文件往返证据**。
- **修掉一个真缺陷**：旋转渐变填充时**图案角度跟着转、渐变方向不转**（几何转了、色带没转）。
  现 `angle_raw += delta`（与图案角度同一约定：相似变换把平面内方向 `φ→φ+delta`）。
  测试**已验证是真钉**：临时回退修复 ⇒ 方向断言在 0° 失败；错号会得 150°。
  顺带覆盖 `SingleColor` 模式，防止该分支悄悄滞后。
- **文档纠错**：`hatch-gradient.md` 组号表（`460` 角度 / `452` 单色 / `462` 着色 / `470` 名称）与
  `GradientDef` 路径（`cad-geometry` → `cad-domain/src/hatch/gradient.rs`）已更正。
- **门禁**：crate 子集 **1716+** 通过 / 0 失败、clippy、fmt、architecture(27)、i18n 578、file-size 770。

## 夹具 `Standard` DIMSTYLE 更正（2026-10-21，尺寸类视觉证据的前提）

- **问题**：`fixtures/dwg/synthetic-four-lines.dwg` 的 `Standard` DIMSTYLE **量级自相矛盾**
  （`DIMTXT/DIMASZ = 0.18` 英制量级配 `DIMGAP/DIMEXO/DIMEXE = 0.625/1.25` 公制量级，而几何仅 ~1 单位）。
  后果：标注文字浮在尺寸线上方 160 px、延伸线被推到线上方 102/248 px、`DIMDLI=3.75`≈745 px ⇒ 基线行**必然出画布**。
  `DIMLINEAR` 与 P5f 共用同一样式解析路径，故**全部尺寸创建**都受影响（既有问题）。
- **修法**：新增**可复现生成器** `crates/cad-import-acadrust/tests/fixtures_generate.rs`（`#[ignore]`、幂等、
  只改 DIMSTYLE、**几何不一致则拒绝覆盖**、跨 DWG 读写迭代到字节固定点）：
  `cargo test -p cad-import-acadrust --test fixtures_generate -- --ignored`。
- **改动值**：`DIMGAP 0.625→0.06`、`DIMEXO 0.625→0.06`、`DIMEXE 1.25→0.12`、`DIMDLI 3.75→0.5`；其余不变；
  **四条 `LINE` 坐标/数量逐字不变**（生成器三次断言）。新 SHA-256 已入 `fixtures/manifest`。
- **如实记录的副作用**（与内容无关、稳定后幂等）：首次往返把 `CLASSES` 按哈希序重排、丢弃一个 DWG 写入器不持久化的
  TABLESTYLE 行抑制标志（组 71）；已写入 `known_limits`。
- **复跑证据**：文字偏移 **160→49 px**；**三行基线首次在画面上**（y=215/116/16.5、共左端 x=249、间距 99 px、
  长度 201/281/361 px、各带外向箭头）；连续仍为背对背链式。
  **仍不完美并归因证据版式**：最高一行文字被画面上缘裁掉（→ **三行渲染、两行带标注**）；标注与夹具家具交叠；
  外侧延伸线因共享原点须贯穿整叠而被裁切。**要更干净的截图应换更稀疏图纸或调点击位置，不是产品缺陷。**
- **教训（写死）**：**尺寸/标注类的视觉证据必须使用单位量级自洽的图纸**；本夹具已自洽，但家具仍密。

## P5f：`DIMBASELINE` / `DIMCONTINUE`（ADR-0017 决策 C 解禁，2026-10-21）

**范围**：`cad-domain`（`dim_line_increment`）、`cad-db`（解析）、`cad-app`（命令/工具/创建）、
`cad-ui-core`（路由/计数）、React（ribbon/图标/提示）。

- **解禁理由被正面对待**：原降级原因是**会话级 `last_dimension` 生命周期风险** ⇒ 改为**显式拾取基准标注**，
  跨命令**零状态**，撤销/删除后下次自然要求重拾；实体里**不存**基准身份或行号（非关联边界不变）。
- **交互**：拾基准 → N 点 → 回车；`AtLeast(2)`（**实施更正**：`push_pick` 把基准也存为点，写 `AtLeast(1)` 会零测量点即 ready）。
- **几何**：基线 `first = base.first`、`second = P_i`、`dimension_line = base.location + n̂_i·(i·DIMDLI)`，
  `n̂` **背离被测几何**、点积 0 **具名拒绝**；连续 `first = prev`、`second = P_i`、**同偏移**，投影 ≤ 0 拒绝（不折返）。
- **提交**：`commit_new_entities_in` ⇒ 一条事务、**一个** `DrawingUndoRecord`、一次 revision，N 实体全有或全无。
- **`DIMDLI` 一次性偏差（已声明）**：进解析快照、**按 DIMSCALE 缩放**、**只用于基线行距**、**显示仍忽略**。
- **修的共享缺陷**：`push_point` 的"就绪后拒加点"规则此前**对开放式工具也生效**；`PLINE` 无前导拾取未触发，P5f 暴露 ⇒ 现按固定/开放式分派。
- **v1 范围**：仅线性基准；无 `DIMCONTINUEMODE`、非关联、无文字覆盖、无每点撤销、无"重复到 Esc"。
- **门禁**：crate **2283/0**、UI **286/286（0 跳过）**、clippy、fmt、architecture(27)、i18n **578**（**无重复键**）、file-size **763**、manifest 23。
- **真机证据（有界）**：`bc_pass=1`；基准 `+1`、基线 `+3`、连续 `+3`，均无错误；选中行
  `type:AcDbDimension, subtype:Linear{rotation:0.0}, measurement:0.603834, display:6`。
  像素：尺寸线**共线**贯穿 `x 249→610`、8 个外向箭头、关节**背对背**（链式非堆叠）、段长与 `0.6/0.4/0.4/0.4` 一致
  ⇒ **连续语义由像素证明**。
- **基线行不可见（声明）**：该夹具 `DIMDLI=3.75` 单位 ≈ **745 px** > 画布 341 px ⇒ 堆叠行在视野外；基线证据是**计数**。
- **一处"看起来坏了"的澄清（归因既有）**：文字在线上方 ~160 px、延伸线看似缺失。实测与公式吻合（误差 1–2 px）：
  `DIMGAP+DIMTXT`+半 em = 0.805 单位 ≈ 160 px；延伸线**画了**但 `DIMEXO 0.625` 比真实深度（22 px）大 5.6 倍，
  被推到线上方 102/248 px。**根因：夹具 `Standard` DIMSTYLE 量级自相矛盾**（0.18 英制配 0.625/1.25 公制，图形 ~1 单位），
  `DIMLINEAR` 同样如此 ⇒ **不是 P5f 的几何错误**。**教训：尺寸类视觉证据要用单位自洽的图纸。**

## P8：文字弹窗编辑（TEXT / MTEXT / MLEADER，ADR-0021 D4，2026-10-20/21）

**范围**：`cad-domain`（`with_text`）、`cad-app`（目标解析/换行权威/选择投影/宿主载荷）、`cad-ui-core`（新状态字段）、
`apps/app-linux`（`textEdit` kind、尺寸、主线程编组）、React（对话框、面板入口、键位）、证据驱动。

- **域层**：`MultiLeaderGeometry::with_text(&self, &str)` —— 换字符串并**重建 `display`**（关键：`update_entity` **不重建**，
  `validate_geometry` **只查 display 有限性、不查与 text 一致** ⇒ 不重建就会"文字改了、屏幕没变"）；拒绝
  `mleader.content_not_text`、`mleader.text_required`；`override_mask`/样式/`width`/附着**不变**。
- **换行权威（唯一实现，两条入口共用）**：`AcDbMText` 与 `MultiLeader(Text)`：`\r\n`/`\r`/`\n` → `\P`；
  `AcDbText`：含换行 ⇒ **`textedit.newline_text`**（消息写明理由：AutoCAD 的 TEXT 不解释 `\P`，否则**跨应用不一致**）。
  其余控制码（`{}`/`\f…;`/`\W…;`/`\Q…;`/`\~`）**逐字透传**，编辑器不解析不美化。
- **打开侧**：`\P` → `\n` **只对未转义**的 `\P`（`\\P` 逐字保留，否则会吃字符）；其余逐字。
- **载荷宿主生成**：`cad_open_popup{kind:"textEdit"}` ⇒ 宿主读**当前单选**注入 `{text, multiline}`；不可编辑 ⇒
  `popup.no_text_selection`、**不开窗**。`cad_open_popup` **整体编组到 GTK 主线程**（会话主线程本地，异步读会拿到陈旧选择）。
- **应用**仍走 `cad_set_selected_text`（不新增 IPC）；`TEXTEDIT` 与面板内联**不变**（A/B 保留）。
- **状态**：`CadSelection.text_multiline`（线名 snake_case，**真实字段**）—— `false` 当 `text` 为 `null`；
  MTEXT/带文本 MLEADER 为 `true`；TEXT 为 `false`。**禁止**从展示行 `type` 推断。
- **UI**：`textEdit` 640×480；邮箱式防御解析（缺 `text`/`multiline` 或类型不对 ⇒ **回落普通外壳**）；
  多行 `Enter` 换行/`Ctrl+Enter` 应用，单行 `Enter` 应用、`Shift+Enter` **既不换行也不提交**；IME 组合期间**永不提交**；
  本地/宿主拒绝都**保草稿**，**成功是唯一关窗路径**；面板与弹窗**共用同一个键位判定函数**（parity 有测试）。
- **证据（真实显示，有界）**：draw 阶段建并选中 MULTILEADER（`mleader_pass=1`）→ `textEdit` 打开（`textEdit:popup-1:true`）、
  载荷 `LEAD`/`multiline=1` → 经**真实准入队列**应用 `EVIDENCE\nEDIT` → 存成 **`EVIDENCE\PEDIT`**、实体数 **52→52**、
  `tool=-`、`error=-`、关闭后 **gate 已释放**、`te_pass=1`。弹窗自身窗口被抓（640×480）并像素判读。
- **像素判读（两轮）**：第一轮 **accept-with-fixes**（提示缺"怎么提交"、无焦点环/光标、元信息对比度、头部失衡）；
  已修：**提示补 `Ctrl+Enter`**、**焦点环 `#7DB8FF`**、头部 16/14、元信息 `#C3CCDA`（6.9:1）、字段内边距。
  第二轮判读**四项 fixed**，并**驳回上一轮一条误判**（标题一直是 13px/粗体，与 11px 提示不同级）。
- **证据补足（已完成）**：驱动新增 **reopen** 步骤（重开在**已存储结果**上）。像素证明：
  ① **多行**——字段两行 `EVIDENCE`/`EDIT`、行距 18 px，且在 590 px 宽字段里 52 px 处换行**几何上不可能** ⇒ 是显式换行
  （`\P`→`\n` 生效）；② **光标**——五帧中两帧捕获 `1×18 px` 竖条，紧贴 `EDIT` 之后（**文末**，与打开态一致），帧间仅该 18 像素在变。
- **证据边界（如实）**：XTest 注入的真实按键 **未落地**（`Z9` 无痕、计数仍 13）；最可能因**驱动用计时器开窗**，合成器不视作
  用户手势 ⇒ 未给键盘焦点（ADR-0021 D1 已警告）。**不是产品缺陷**（真实使用由点击打开），但由此确立一条**项目级边界**：
  **至今没有任何输入路径被真实 OS 键盘验证过**，弹窗继承该边界。
- **既有缺口（声明不修）**：`MLeaderText.width`（组 43）**不参与排版** ⇒ 改字串**不重排/不重适配**；忙碌文案复用 `linux.busy`；
  未知 kind 文案复用既有键。
- **门禁**：crate **2262/0**、UI **280/280（0 跳过）**、clippy、fmt、architecture(27)、i18n **571**、file-size **755**。

## 弹窗管线第 1 步 + N7 对抗性夹具（2026-10-20）

### 弹窗层（ADR-0021 D1/D2，已实施并验证）

- **机制**：独立原生窗口 + 自己的 child webview（**不是** DOM 浮层、**不是** `GtkPopover`——Wayland 上 popover 是 `wl_subsurface`，
  GTK3 的 raise 是空函数，必被 CAD 子表面盖住）；`initialization_script` 注入 `window.__YACR_POPUP__` 字面量 ⇒ 无竞态。
- **模态**：`ShellState.popup` 为权威 gate，**只在 `Destroyed` 清除**（IPC/Esc/窗管关闭同路）；陈旧记录在开窗前对账，
  **漏事件不会卡死输入**；`probe_input`（除 `resize`）与 `cad_command` 拒绝 `popup.modal_open`。
- **N5 已按设计落地**：UI 侧挂起有**两个独立理由**（`textFocus`/`popupOpen`），命令行 `onBlur` 只能清自己的 ⇒
  "弹窗开着 + 命令行失焦"**仍挂起**（有回归测试）。
- **capability 不需要**：无 app ACL manifest ⇒ 本地 origin 的应用命令对所有窗口可用（已核实）。
- **证据（真实显示，有界）**：`popup_pass=1`（开/状态/拒绝/关/再准入全绿），并用 `x11-window-grab <xid>` 抓到
  **弹窗自身 520×360** 像素；判读**接受**，四处布局缺陷（195px 空洞、内边距不一致、关闭图标不对称、标头字重偏弱）**已修并复判 fixed**。
- **行为变更（已声明）**：语言改为 `localStorage["yacr.locale"]` 持久化 ⇒ **重启后保留**，且**弹窗文档读到同一选择**
  （此前运行时切换进不了弹窗，重启即丢）。
- **契约冲突（已裁定）**：`dialog.about.version` 是**模板**（含 `{version}`/`{build}`），UI 侧原先当"标签"用会字面打印占位符；
  已统一为模板，缺 `build` ⇒ 模板内补 `—`，缺 `version` ⇒ 整行 `—`（`版本 —（—）` 属**编造**读数）。
- **未做**：右键菜单（用户暂缓，钩子保留）；文字弹窗编辑（后端 `SetSelectedText` 已就绪）；冻结帧（不采用）。
- **小瑕疵**：版本行全角括号左侧留白偏大，读起来像多一个空格；未知 kind 文案复用既有键（当前不可达）。

### N7：把"已声明的归一化"变成可执行断言

四个**对抗性**夹具（源侧构造，绝不喂自洽输出），全部通过：
1. **尾部 `271` 由 root 0 派生**：源实体级 `271=Vertical`、root 0 `Horizontal` ⇒ 断言导入只留 root 0、再导出写 `271=0`，root 1 仍 `1`；
2. **空串 MText ⇒ 无内容**：`has_text_contents=true` + `text_string=""` ⇒ 导入为 `None`、导出写 `172=0`、不写文本组；
3. **`break` 镜像由记录重导**：伪造镜像 `7`/`99`、记录 `3`/`4` ⇒ 断言写出的是 `3`/`4`，读回 `break_info_count=2`；
4. **端到端**：对抗源 → 导入 → 导出 → **acadrust 读回**，断言有效字段全存活 **且**三条归一化逐一成立（注释标明是**声明**而非保真）。

**如实报告的细节**：我们的写入器其实是**拷贝** `segment_index`/`break_info_count`，真正的"重导"发生在 **acadrust 的读/写**里
⇒ 声明描述的是**端到端可观察行为**（正确），但实现位置不在我们这边。

## P7 阶段门禁：ACCEPT-WITH-FIXES → 两项阻塞已修（2026-10-20）

**门禁结论**：实现与 ADR-0019/0020 一致、证据诚实且有界、两处偏离已贯穿**代码/测试/ADR/handoff**；
阻塞项**只在诚实层**，无需返工架构、**无需新一轮门禁**。

- **B1（已修，纯文档/注释）**：`compatibility.md` 仍在说**已不成立**的话——`STRETCH` 的 `TEXT`"未查证 ⇒ 整命令拒绝"、
  `EXPLODE` 的"按既有变换拒绝面整命令失败"（P7a 起变换面**接受** `Text`，实际是 `child_refusal` **显式拒绝**）、
  缺 `MultiLeader` 的整命令拒绝说明；`ROTATE/SCALE` 与 `ALIGN` 行未警告"含 MultiLeader 时缩放/`AlignScale` 整命令失败"；
  变换不支持几何列表仍把 `Text` 列为不支持；`dxf-entity-coverage.md` 仍是变体前的理由集；`kind.rs` 注释"Only TEXT…"过期；
  `TEXT/MTEXT` 行尾"MLEADER 文本编辑**等 P7b**"的**理由已失效**（P7b 已落地、仍**未实现**）⇒ 改为当前缺口的诚实声明。
- **B2（已修，代码+测试+文档）**："无静默丢失"是**假的**。补：
  ① **实体级 `342`（自定义箭头块）在导入侧具名**（**恢复 P7b 前的理由**），与逐线 `341` **不一致**时**另加**一条具名理由，
  **两种句柄都点名**、不静默择一；② **常设具名偏差** `REASON_UNMODELLED_GROUPS` 覆盖被归一化为默认值的
  实体尾 `292/293/294/174/178/295` 与上下文 `44/293/295`；③ 模块文档**删掉"零丢失"**，改为"声明只限写入器能观察到的内容"。
  将 `342` 存进 `reserved` 是**后续**项。
- **记录在案的遗留（非阻塞）**：
  - **N1** 尾部 `271` 由 **root 0** 派生（源实体级 `text_attachment_direction` 未读）⇒ 源中不一致时静默归一化（已声明）；
  - **N5（跨决策）** 命令行 `onBlur` **无条件**解除指针挂起，而外部点击先触发 `blur` ⇒ **将来模态 `input_gate` 必须自己重新断言**，
    不能依赖单一所有者；已写入 ADR-0021 D2；
  - **N6** `has_text_contents=true` + 空串 ⇒ `MLeaderContent::None` ⇒ 导出 `172=0`，属**内容类型归一化**且无具名理由；
  - **N7** 往返测试用**自洽**夹具，上述偏差**按构造不可见** ⇒ 需一个对抗性夹具断言这些**已声明**的归一化。
- **门禁复跑**：见提交信息（crate 全量子集 / UI 212 / clippy / fmt / architecture(27) / i18n 558 / file-size / manifest 23）。

## P7c 真机证据 + 一个真实交互缺陷的修复（2026-10-20）

- **证据**（GNOME/Mutter + Quadro P620 Vulkan + `GDK_BACKEND=x11` + `scale=1`，分区 926×341）：
  `mleader_pass=1`、`delta=1`、工具收尾、无错误；`select_count=1`，选择行
  `type:AcDbMultiLeader, layer:0_(0), space:model, style:Standard, roots:1, lines:1, display:4`。
  **像素判读 ACCEPT**：箭头尖 (269.5,240)、实心三角 13×36、引线倾角 20.6° 直线度残差 0.46 px、
  **狗腿肘部可见**（y=89–90，x 272–326，末端距落点 17 px = 一个 landing gap）、`LEAD` 文字 bbox 逐边 ≤1 px 且水平中心与落点精确对齐、**零碰撞**（最紧 13 px）。
- **证据边界**：像素不证实体类型，类型由同一轮 `select_rows` 钉住（两者互补）；单合成器/单卡/`scale=1`；
  **无真实 MULTILEADER 夹具**；**未在 AutoCAD 打开**。
- **`--lifecycle 0` 才是关闭**：`--lifecycle exit` 会启用生命周期驱动并**提前 `app.exit(0)`** ⇒ 首轮 `NATURAL_EXIT`、无截图。
- **首轮失败的原因（已修证据选点）**：点选落在既有一笔之上 + 两次拾取同 y ⇒ 引线压在既有线上、狗腿与引线共线，
  不可判读且命中 2 个实体。已移到空白走廊并令引线倾斜。
- **同时修掉的真实交互缺陷**：**命令行打完字后点画布的第一下被静默吞掉**。根因：`onFocus` → `suspendPointer(true)`，
  `input.mjs` 的 `down` 在 `!accepting` 时 return，而真实 `pointerdown` **先于**焦点转移（blur）派发。
  修复：命令行聚焦期间用 **window 捕获阶段** `pointerdown`，外部目标先 `blur()` 再让**同一个**事件送达 `.native-input`；
  不 `preventDefault`/不 `stopPropagation`；命令行内部点击不受影响；聚焦结束/卸载即移除。UI **208 → 212**。
- **附着语义更正**：`attachment 2` 只是**水平**居中；**垂直**取样式 `left_attachment=1` ⇒ `Top` ⇒ 文字在落点**上方一个字高**
  （实测 37 px）。复合锚是 **top-centre**，此前报的"居中"是错的；与 AutoCAD 复合码的对应（U-3）仍未验证。

## P7c：`MLeaderStyle` 表 + `MLEADER` 创建（ADR-0019 决策 D/F.2/F.3，2026-10-20）

**范围**：`cad-db`（新表）、`cad-import-acadrust`（`MLEADERSTYLE` 喂表 + 创建往返测试）、`cad-app`（命令/工具/创建）、
`cad-ui-core`（选择子/计数/5 个 i18n 键）、`apps/app-linux/ui`（ribbon 入口 + 字形 + 三段提示）。

- **表**：`MLeaderStyle`（`standard_metric()` + 按名解析，先精确后忽略大小写），`finish()` **始终 pin `Standard`**；
  默认值是**常量**（狗腿 `0.36`、缺口 `0.09`、箭头 `0.18`、字高 `0.18` 等）——**不是 vendor 验证过的样式表**，
  与 `TEXT` 的 2.5 同类债务；样式表**未建模字段具名不存**。
- **`MLEADERSTYLE` 导入现已喂表**（此前只按实体临时解析后丢弃）。
- **创建**：箭头 → 落点 → 文本；`Exact(2)` + 文本；单簇/单线/2 顶点/仅直线；**一个** `AcDbMultiLeader`、
  一条事务/一步撤销/活动图层、掩码 `0`、`AcDbMultiLeader`。**无内容不创建**（仅导入/显示/导出）。
- **精确几何（钉住）**：箭头 `(0,0,0)` + 落点 `(10,2,0)` ⇒ 顶点 `[(0,0,0),(9.64,2,0)]`、狗腿线 `(9.64,2)→(9.91,2)`
  （`0.36 − 0.09`）、文字在落点、高 `0.18`、附着点 `2`（**居中**——样式记录**没有水平附着侧**，声明）。
- **修复既有潜伏 bug**：`MultiLeaderGeometry::new` **实际不可达**（对 `from_import` 的任何理由都判失败，而
  `build_display` 总返回信息性的"颜色/线宽未应用"）⇒ 现只校验**结构性**缺陷；`from_import` 未改。
- **拒绝面**：`mleader.payload`、`tool.text_empty`、`tool.text_too_long`、`tool.text_points_incomplete`、
  `tool.text_not_awaiting`、`tool.text_required`、`mleader.style_missing`、`mleader.text_required`、
  `mleader.content_unsupported`、`mleader.path_type`、`mleader.points_not_finite`、`mleader.plane`、
  `mleader.landing_too_close`、`mleader.leader_degenerate`、`mleader.style_invalid`、缺活动图层、Viewer。
- **UI**：`OFFSET → TEXT → MLEADER → HATCH`（HATCH 仍收尾）；MLEADER 按钮**无 caret / 无 variants**（不暗示 `Options`）；
  点 0 → arrow、点 1 → landing；落点后的"待文本"窗口用 `TOOL_READY_PROMPTS`（否则会错误落到"标注已定义"）。
- **门禁（编排者复跑）**：crate **2215/0**、UI **208/208（0 跳过）**、clippy/fmt/architecture(27)/i18n(**558**)/
  file-size(**734**)/manifest(**23**)。
- **NOT RUN**：全量 workspace、跨平台、**真机像素判据**（ADR J 对 P7c 的要求，留作独立一步）、真实 MULTILEADER 夹具。
- **下一步**：**P7 阶段门禁**（含真机证据），然后弹窗层第 1 步（管线骨架）按用户确认。

## P7b-2：真 `MULTILEADER` 导出（`MLEADERSTYLE` 去重 + `CLASSES`，ADR-0019 决策 E，2026-10-20）

**范围**：`crates/cad-import-acadrust/src/export/{multileader.rs(新), entity.rs, mod.rs}` 与测试。

- **从域定义写，不写派生显示**：`CONTEXT_DATA` / `LEADER{}`（落点、狗腿向量、每簇长度、`leader_index`、附着方向、打断）/
  `LEADER_LINE{}`（顶点逐点、`index`、覆盖位、箭头尺寸/handle、颜色/线型/线宽、打断）/ 尾部（`340`、**`90` 逐字**、
  `170`、`172`、`343`、`173/95/174/175`、`92`、`292`、`45`、`271–273`）。**不写计算后的文字排版**。
- **U-5 勘误（推翻 ADR 原前提）**：`CadDocument::new()` **已预注册约 260 条**，含
  `("MULTILEADER","AcDbMLeader",1025,"ACDB_MLEADER_CLASS",true)`（`acadrust-0.6.5/src/classes/mod.rs:338`）与
  `MLEADERSTYLE` 对应项 ⇒ 原"`CLASSES` 默认空、必须插入两条"**错误**。实现改为**缺失才补**，且**仍断言**写出的
  精确字符串/flags；**group 1 从不写 `MLEADER`**。
- **`MLEADERSTYLE`** 按名去重（`Standard` 优先），同名不同快照**取首个 + 报告冲突**。
- **块内放置与 `Compound` 对齐（修正了一次内容丢失）**：刚性 ⇒ 真实体；**缩放/镜像 ⇒ `Converted` 摊平**
  （`export.converted.multileader_flattened`，复用同一个 `flatten_children`），每个子项记自己的结果
  （镜像时 `Text` 子项按域规则单独拒绝）。**第一版整条 `Dropped` 会让缩放块内的标注从导出里消失**，与其它图元不一致，已改。
- **具名偏差（全部进 `ExportReport`）**：`91/341/171` 写默认；样式是**有效值快照**（掩码抑制字段**非逐字往返**）；
  未建模上下文取显式默认、`text_normal/direction` 派生、`content_base_point = text.location`；seeds 未建模
  （且 acadrust 读回把 `300` 映进 `description` ⇒ **写读不对称**）；per-line 箭头/线型 handle **悬空**；
  `271–273` 样式级 **acadrust 无法表达**。
- **往返**：合成 → 导出 → **acadrust 读回** → 再导入，比**有效值**；**不**断言掩码抑制字段字节相等。
- **门禁（编排者复跑）**：受影响 crate **2191 通过 / 0 失败**；UI **200/200**；clippy/fmt/architecture(27)/
  i18n(**553**)/file-size(**732**)/manifest(**23**)。**NOT RUN**：全量 workspace、跨平台、真机/像素；
  **无真实 MULTILEADER 夹具，未在 AutoCAD 打开验证** ⇒ U-4/U-5 仍为推断。
- **下一步**：P7c（`MLEADER` 创建：2 点 + 文本、直引线、单簇、2 顶点）→ **P7 阶段门禁**。

## 弹窗层决策：居中模态窗口 + 原生锚定菜单（ADR-0021，2026-10-20）

**纯文档**（未改代码）。起因：用户问"后续 DOM 弹窗（选项菜单、右键菜单、text 图元弹窗编辑）怎么处理"，
并明确 **除右键菜单外，弹窗居中即可、与 wgpu CAD 层位置无关**。

- **约束回到事实**：CAD 分区是 overlay **最上层**、**不透明**原生表面（X11 X child / Wayland `wl_subsurface`），
  WebView 在其**下面**；输入穿透只让**输入**过去，**像素不过去** ⇒ **CAD 矩形内的 DOM 看不见但收得到事件**。
- **D1 居中模态 = 第二个 Tauri webview 窗口**：`parent(&main)`（= `gtk_window_set_transient_for`）、
  `decorations(false)`、`transparent(true)`、`.focused(true)` + GTK 层 `set_modal(true)`（Tauri 2.11.6 **无** `modal()`）；
  同一 bundle/`tauri://` origin 复用 React 与 i18n；**capabilities 必须加该 label**（否则新窗口**无 IPC 权限**）；
  **Wayland 下不承诺像素定位**（toplevel 无位置请求）；**必须用户手势打开**（`xdg_activation` 禁止自激活）。
- **D2 模态由我们在输入漏斗里强制**：`probe_input` 转发与除 Esc 外的全局快捷键暂停 + **host 侧二次拒绝**；
  关闭后**重新校验工具相位**。本项目 CAD 输入只有一个漏斗 ⇒ 成本低。
- **D3 右键菜单 = 原生 `GtkMenu`**（`gtk_menu_popup_at_pointer`，**无需绝对坐标**故 Wayland 可用；真 seat grab、
  外部点击关闭、键盘导航、a11y）；文案只从 `cad-ui-core/i18n` 读；`GtkCssProvider` 近似 chrome；
  **声明偏离：原生外观 ≠ React chrome**。触发链：`cadContextMenuRequest` 钩子 → **新 IPC `cad_open_context_menu`**。
- **D4 文字弹窗编辑（原 C）重新纳入**：ADR-0020 已加局部修正说明——A/B 不变，**C 以模态窗口形式**用于
  多行/MTEXT/长文本/MLEADER（P7b 后）。
- **D5 冻结帧就地编辑不采用**：4K 一帧 ≈33 MB（base64 ≈44 MB）过 IPC + 编辑期渲染冻结 + 需新建 GPU 回读路径。
- **D6 chrome 浮层：已审计，**规则本来就在代码里**（**纠正初稿误判**）：唯一绝对定位浮层是 ribbon 变体下拉，
  已是 **in-band popover**（限制在 `.ribbon-panel__body` 内 + `max-height:100%` + 内部滚动），**不会**越到 CAD 矩形；
  约束注释见 `ui/src/style.css:243-247`。其余（completions/history-panel/palette）在文档流内，改变的是布局而非遮挡。
- **关键反证（已入 ADR）**：GTK3 `GtkPopover` 在 Wayland 上是 **`wl_subsurface` 而非 `xdg_popup`**，
  `gdk_window_wayland_raise()` **是空函数**、z 序只按创建顺序 ⇒ **CAD 子表面会永远盖住 popover**，
  且其 modal 只是进程内 grab，不拦原生表面；WebView-in-popover 无官方支持、键盘/IME **依赖合成器**。
- **未决（需用户定）**：① 右键菜单是否接受**原生外观**（否则只能 X11 DOM + Wayland 原生 ⇒ **后端分叉**）；
  ② C 的排期（紧跟 A/B 还是等 P7b 后一次做全）。

## P7b-1：MultiLeader 一等表示 + 导入迁移 + 匹配点清单 + **显示等价**（ADR-0019 决策 C/G，2026-10-20）

**范围**：`crates/cad-domain`（新 `multileader.rs`）、`crates/cad-import-acadrust`（迁移 + 临时导出）、
以及 `cad-db`/`cad-geometry`/`cad-spatial`/`cad-app`/`cad-measure`/`cad-platform`/`cad-proxy`/`cad-representation`
的匹配点。**未**实现 DXF 写入（P7b-2）。

- **变体**：`SemanticGeometry::MultiLeader(Box<MultiLeaderGeometry>)`，私有字段 + `new`/`from_import`/`transformed`；
  `override_mask` **原样**；`reserved` **仅逐字源数据**（打断、背景/分栏、未建模 per-line 位；**每个源组码只导一次**，有测试）；
  保留每簇/每线索引与 per-line 覆盖位、箭头字段、`plane`。
- **显示等价（本片验收核心，已钉）**：派生显示是旧合成的**逐字搬迁**——同一 `Polyline`、同一 `Spline`
  （degree `3.min(n-1).max(1)`、空 knots/weights）、同一箭头 `Mesh`、同一狗腿公式、同一 `Text`
  ⇒ **迁移不改变渲染**（测试对比迁移前的捕获数值）。
- **导入**：**仅 MText 与无内容**迁入变体；**块/公差内容保持旧 `Compound` 路径**（**渲染不变**，`Partial` + 具名原因）。
  变体路径恒 **`Partial` + 具名原因**（颜色/线宽覆盖、背景、分栏、打断、自定义箭头、非默认平面）——**不报 `Complete`**。
- **变换**：平移/旋转（重建 `display`）；**缩放（含均匀）/镜像/非均匀 ⇒ 整选择 `Unsupported`**。
- **匹配点**：`EXPLODE` **顶层与块内均显式按 `type_key` 拒绝**（不写任何东西，不依赖已失效的变换副作用）；
  `STRETCH` 显式整命令拒绝；`class_name = "AcDbMultiLeader"`、`kind_name = "MultiLeader"`；
  pick/bounds/snap 与等价 `Compound` 平价（有测试）。
- **导出（临时）**：`export/entity.rs` 按旧 `Compound` 方式摊平 `display()`，记录 `export.converted.multileader`
  并注明由 P7b-2 替换；**尚未写 `MULTILEADER`**。
- **两处如实记录的偏离**（已写入 ADR 实施小节）：
  1. **样式是"解析后的有效值"快照**，不是原始 MLEADERSTYLE 记录 ⇒ 掩码**未置位**而实体值与样式不同的**原始值未保留**
     ⇒ **P7b-2 不得声称这些字段逐字往返**；往返验收比**有效值**，且"自洽断言"在此为**构造性成立**。
  2. **狗腿起点**按代码取"**第一条有末顶点的引线**"（非 ADR 原措辞"最后一条"）——为显示等价以**代码为准**，ADR 已更正。
- **门禁（编排者复跑）**：受影响 crate **2177 通过 / 0 失败**；UI **200/200**；clippy(`-D warnings`)、fmt、
  architecture(27)、i18n(**553**)、file-size(**728**)。**NOT RUN**：全量 `cargo test --workspace`；跨平台；
  **无真实 MULTILEADER 夹具**（全部合成）；真机/像素（本片未新增渲染几何，但要证"没变"只能靠合成数值对比）。
- **顺带修掉的复发问题（第三次）**：`selection.rs` 新增 `PropertyRow` 键（`roots`/`lines`）缺目录与 switch ⇒
  已在 **AGENTS.md 写入机械规则**（新增属性行必须同时补两目录 + `cad.mjs` 的 `propertyRowLabel`）。
- **待办**：P7b-2（真 `MULTILEADER` 写入 + `MLEADERSTYLE` 去重 + `CLASSES` 两条 + 往返同一性）、P7c（创建）。

## P7a-3/4：编辑现有文本（`TEXTEDIT` + 属性面板可编辑字段，ADR-0020 增补，2026-10-20）

**范围**：`crates/cad-app`（命令/工具/应用层/拒绝面）、`crates/cad-ui-core`（请求与状态投影）、
`crates/cad-host`（请求派发）、`apps/app-linux`（新 IPC）、React 属性面板；另含 9 个 i18n 键。

- **A：`TEXTEDIT`**：`CommandId::TextEdit`（选择子 `TEXTEDIT`，`EXPECTED_COMMAND_COUNT` 69 → 70）；
  `DrawToolKind::TextEdit`（`Exact(1)`、`picked_step_count = 1`、`requires_text`）。**拾取一个 `Text` → 预填当前串
  （`text_prefill`）→ 终端文本提交 → `update_entity`**：**只替换字符串**，样式/高度/旋转/位置/font/对齐**保留**，
  **一条事务/一步撤销**（撤销标签 `edit text`）。**无 ribbon 入口**（打字命令；有选择时面板编辑器即主入口）。
- **B：面板编辑**：`InteractionRequest::SetSelectedText { text }` + Tauri `cad_set_selected_text { text }`
  （有界准入、`CadCommandAck { accepted: true }`）。**payload 不含实体 id**——宿主解析**当前单选**并在应用时
  **重新校验**，UI 不伪造身份。`CadSelection.text` 携带**可往返**的当前字符串（**与展示用 `PropertyRow` 分离**）。
- **UI**：属性面板（**chrome、在 CAD 矩形之外**）内 `textarea`（`input` 会吞换行 ⇒ 破坏逐字往返）；逐字显示/回传；
  **选择变化时草稿保留为 detached**（Apply 停用 + 说明如何恢复，**绝不**应用到别的对象）；空文本**可点但具名拒绝**；
  超限显示 `{used} / {max}` 且**不截断**；`Ctrl/Cmd+Enter` 提交、**普通 Enter 是换行**；IME 双信号不提交；
  异步失败**回滚到图纸实际值**（宿主权威）。
- **契约核对**：`text_prefill` 用 **snake_case**（`CadTool` 是 camelCase 重命名，漏写会被**静默忽略**）；
  `cad_set_tool_text` 与 `cad_set_selected_text` **都**返回 `CadCommandAck { accepted: true }`（与前端判定一致）。
- **拒绝面（具名、不开事务）**：`textedit.target_not_text`/`target_missing`（含**块内成员与子元素**）、
  `tool.text_empty`、`tool.text_too_long`（1024 `chars()`）、`tool.text_not_awaiting`、`tool.text_points_incomplete`、
  `textedit.selection_empty`/`selection_multiple`。
- **门禁（编排者复跑）**：受影响 crate **2157 通过 / 0 失败**；UI **200/200（0 跳过）**；clippy(`-D warnings`)、
  fmt、architecture(27)、i18n(**551**)、file-size(**722**)。**NOT RUN**：全量 `cargo test --workspace`；跨平台；
  真机（本片未新增渲染几何；**编辑链路无真机证据**）。
- **如实记录的缺口**：① `status.tool_parameter.text_height` 的 UI 步进器**未接线**（默认 2.5）；
  ② **i18n 门禁盲区**：`check-i18n.py` 只扫**静态字面量**，而 UI 侧键经**导出常量**引用（`TEXT_EDIT_KEYS`）⇒
  键缺失可**绕过门禁**（本次靠 UI 测试具名报告才发现）；③ 拒绝码（`textedit.*`、`tool.text_empty` 等）是
  **机器码**、按既有惯例**原样**进入 `CadStatus.error`，不属目录键。
- **MLEADER 文本编辑仍等 P7b**（其文字今天是 `Compound` 子项、无可寻址身份）。

## P7a：文本通道 + `TEXT` 创建 + `Text` 变换 + MTEXT 诚实性（ADR-0019 决策 A/B/F.1/H，2026-10-20）

**范围**：`crates/cad-ui-core`（wire/状态/选择子/i18n）、`crates/cad-app`（`SetToolText`/`TEXT`/变换/拒绝面）、
`crates/cad-db`（Text 变换）、`crates/cad-geometry`（捕捉面同规则）、`crates/cad-import-acadrust`（MTEXT `Partial`）、
React 宿主 UI（命令行文本模式 + 浏览器默认行为抑制）；另含 ADR-0020 的就地编辑否决与编辑入口决策。

- **文本通道（新平台能力）**：`InteractionRequest::SetToolText` **终端语义**——文本是最后一步，**提交即完成**
  （`ready && !awaiting_text`），**无需第二次 Confirm**。**专用 IPC `cad_set_tool_text`**（与
  `cad_set_tool_parameter` 同型，**不走 `cad_command`**）；`CadToolStatus.awaiting_text` 有**专用 wire 名**
  （`#[serde(rename = "awaiting_text")]`，避免 camelCase 漂移）。
  **拒绝面**：空/纯空白、**超 1024 字符**（按 `chars()` 计）、非待文本状态、点未齐、**待文本期间 `Confirm`**
  （否则会从后门完成成无文本）；全部具名且**不开事务**。
- **`Text` 可变换**：平移/旋转/**均匀缩放**；**镜像与非均匀缩放拒绝**。**两个变换面一起改**：
  `cad-db/src/transform.rs` 与**捕捉面** `cad-geometry/src/engine.rs`（后者注释明确"两面必须给出相同 Text 结果"），
  并有**跨面一致性测试**。
- **既有行为保持（ADR-0019 决策 B.2）**：`STRETCH` 选中 TEXT **仍整命令拒绝**、`EXPLODE` 含 Text 成员的 INSERT
  **仍整命令失败**——但两者的**理由已更正**（不再依赖"变换面拒绝 Text"这一副作用），并有测试钉住。
- **`TEXT` 创建**：`CommandId::Text`（选择子 `"TEXT"`，`EXPECTED_COMMAND_COUNT` 68 → 69）、
  `PointArity::Exact(1)` + `requires_text`、`textHeight` 参数（>0，默认 2.5——**`Style` 表无高度字段**，常量即债务）、
  单一漏斗、一条事务/一步撤销/活动图层、`type_key`/`class_name` = **`AcDbText`**。
- **MTEXT 诚实性**：`MTEXT → Text` 此前报 `Complete` 而**静默丢**列/背景/行距样式/绘制方向；
  现为 **`Partial` + 四条具名原因**（`text.{background,columns,line_spacing_style,drawing_direction}_unsupported`）。
- **UI**：命令行**文本模式**（同一输入元素切换，不跳位）；`Enter → cad_set_tool_text`；**1024 按 Unicode 标量值**
  计数（与 Rust `chars().count()` 对齐）；**超限不截断**、如实显示；IME（`isComposing` + `keyCode 229`）不提交；
  **Esc 两次**（失焦 → 取消）；`TEXT` 进 draw 面板（OFFSET 与 HATCH 之间，HATCH 仍收尾）+ 独立字形。
- **宿主默认行为（用户报告）**：CAD 原生表面**按设计对输入透明**（X11 空 input shape / Wayland 子表面空 input region）
  ⇒ **所有** CAD 输入经 DOM 合成，右键**落到 WebView** 并弹出 WebKit 默认菜单（含 Reload，属**状态分叉风险**）。
  已修：`contextmenu` 外壳抑制（**输入框内保留**，命令行仍可复制粘贴）；`Ctrl+R`/`Ctrl+Shift+R`/`F5`、
  `Ctrl±`/`Ctrl+0`、`Ctrl+P`、**`Ctrl`+滚轮**全部拦掉（即使焦点在文本框）。**为未来留钩子**：
  `cadContextMenuRequest({point, selection, tool})` **单一调用点、今天不产生任何可见行为**（不伪造菜单）。
  **devtools 如实报告**：`tauri.conf.json` 表达不了（`"windows": []`、无 `devtools` 字段），release 默认不启用
  但**非本仓库固定**；抑制右键菜单后"Inspect"入口已消失。
- **文件规模**：`draw_tool/mod.rs` 曾达 1079 行（超 1000 硬上限）⇒ `DrawIntent` 文档重枚举迁至
  `draw_tool/intent.rs`，`mod.rs` **744** 行、`intent.rs` 266 行，**公开路径不变**（`pub use`）。
- **门禁（编排者复跑）**：受影响 crate **2060 通过 / 0 失败**；clippy(`-D warnings`)、fmt、architecture(27)、
  i18n(**542**，P7a +8)、file-size(**711**)、UI **185/185**（含 +14 文本模式、+8 浏览器默认行为）。
  **NOT RUN**：全量 `cargo test --workspace`（`accelerated_tier` 超时）；跨平台；**真机证据**（本片未新增渲染几何，
  文本渲染路径已由 P5e 的字体设施覆盖；P7a 的真机走查留待与 P7b 合并）。
- **待办**：**A+B 编辑入口**（ADR-0020 增补）：`TEXTEDIT` 命令 + 属性面板可编辑字段（需"实体身份 + 可回填文本"的
  新状态字段，**不得**复用 `PropertyRow` 展示串）；`status.tool_parameter.text_height` 的 UI 步进器未接线；
  MLEADER 文本编辑等 P7b。

## 技术债清偿：环深度探测、`-Z` 挤出导入、bulge 展开等价性与语料不变量（2026-10-20，P6 门禁点名）

**提交**：`102bb1c`。**范围**：`cad-domain`（`hatch/common.rs` 共享助手 + `fill.rs` + `hatch_entity`）、
`cad-import-acadrust`（`geometry.rs`/`entity/convert.rs`）、以及 `cad-geometry`/`cad-spatial`/`cad-measure`/
`cad-representation`/`cad-cli-tools` 的**测试**。

- **① `fill::ring_depths` 只看首顶点（**未声明的孪生**，已修）**：它驱动**奇偶配平**（谁算孔洞）⇒
  **相切导入环**可能被错填；实测旧代码常直接退化为 `Degenerate`（面积自检 **112** vs 几何 **88**）。
  现与 `loop_depths` **共享同一"最多 8 点多数 + 确定性回退"助手**（挪进 `hatch/common.rs`，不再各存一份）。
  **非相切输入不变**（各探测点一致）；**创建路径不受影响**（先拒绝相切）。新测试在旧代码上**失败**、现在通过。
- **② `-Z` 挤出导入（正确性修复，用户可见）**：真正的快速路径是 **`polyline_ocs_points`**（`ocs_to_wcs` 本就精确），
  它把**任何 Z 平行法向**当 `+Z`，而 `-Z` 的 OCS 是 **x 镜像** ⇒ 折线顶点、**bulge 方向**、TEXT/ATTDEF/ATTRIB 的
  **位置与旋转**、ELLIPSE 的**法向**均未镜像。现 `+Z` 恒等、`-Z` 镜像：点 `(-x, y, -elevation)`、**bulge 取反**、
  文字旋转 `π − θ`、椭圆**保留 `-Z`**。**审计确认 CIRCLE/ARC/INSERT 本就正确**（`arbitrary_axis` 的 −Z 帧）⇒
  **无双重纠正**；MTEXT 按 DXF 本就 WCS。四项 `minus_z_*` 测试中三项**在旧代码上失败**，一项是已正确路径的回归钉。
  **残余**：**倾斜**挤出的 TEXT/ATTRIB 仍按原始 OCS 放置（域内 `Text` 无挤出法向，需域变更）；**文字字形不镜像**
  （平面文字域无法表达真反射）。
- **③ bulge 展开**七份实现**统一钉在手工计算的解析期望**上（四分弧 / 半圆 / **优弧** / 负 bulge / 极小 bulge /
  **非轴对齐弦**，centre/radius/included-angle/apex 皆独立推导）：显示三角化、EXPLODE 弧恢复、拾取包围盒、
  对象捕捉、导入器边界展开、填充采样、**SHX 字形弧**。**回退任一份的符号都会让测试失败**（已实测）。
  生产改动仅为**可见性**（3 处 `fn` → `pub(..) fn`），无逻辑改动。
- **④ 语料几何不变量（替代"只断言非空"）**：对**每条非零 bulge 段**断言"采样顶点在弦的 `sgn(bulge)` 侧 +
  采样落在解析圆上"，**输出计数**：**11 个夹具 / 9 段**（法兰 6、`entities` 3），跳过项也计数
  ⇒ 语料静默变化会体现为计数变化，而不是"静默通过"。这正是此前**六个法兰 + 三个 entities** 弧被镜像而
  夹具测试全绿的原因。
- **门禁（编排者复跑）**：受影响 crate **2026 通过 / 0 失败**；clippy(workspace `-D warnings`)、fmt、
  architecture(27)、i18n(**534**)、fixture(**23**)、file-size(**704**)、UI **163/163**。
  **NOT RUN**：全量 `cargo test --workspace`（`accelerated_tier` 超时）；跨平台；**真机证据**（本片未新增渲染路径改动，
  故未重跑；前一轮 P6 的像素判据仍为其自身结论的边界）。

## P6b：`HATCH` 创建（预选闭合实体）+ 真机像素判据（ADR-0018，2026-10-20）

**范围**：`crates/cad-app`（命令/工具/参数/边界/拒绝面）、`crates/cad-ui-core`（选择子/参数投影/i18n）、
React draw 面板；另有证据驱动 `apps/app-linux/src/evidence/draw_hatch.rs`。**提交**：`6dbf536`（后端）、
`0f7b62a`（UI + ADR/能力表更正）、`751ca67`（证据驱动 + 像素判读）。

- **形态 = 预选**：`DrawToolKind::Hatch` 用 `arity() = Exact(0)` + `requires_selection()`（**与 `ARRAY`/`MOVE` 同型**），
  `picked_step_count() = 0` ⇒ **未引入新的捕获状态机**。`COMMAND_TABLE` **67 → 68**。
- **边界**：`Circle`、完整 `Ellipse`、**闭合** `Polyline`，每个成一环，须**共面**（圆/椭圆另须**法向一致**）；
  全部经 `HatchGeometry::new` ⇒ A.3 拒绝面为**整命令拒绝且不开事务**。**保留边界实体**、单事务/单撤销/活动图层、
  `type_key`/`class_name` = **`AcDbHatch`**。
- **参数**：唯一 `ToolParameter` 通道——图案类型（`0` 实心/`1` 用户自定义）、间距（>0）、角度（**通道度、域弧度**）、
  双线、样式（`0`/`1`/`2`）。**用户自定义**复用导入侧合成规则（不透明名 `USER`）；实心记 `Predefined` 以与 SOLID 夹具一致。
- **与 ADR 的一处更正（已入档）**：圆的环是 **4 段精确圆弧**（`bulge = tan(π/8)`），**不是** ADR 原写的两点环——
  已提交校验器要求 **≥3 顶点**且按**弦**测面积 ⇒ 两点环必判退化；改校验器是更大且未经复核的语义改动 ⇒ 本相不做。
  椭圆按 **64/圈采样**（真椭圆非圆弧，bulge 会是**假几何**）；椭圆弧非闭合 ⇒ 拒绝。**弦间相切可能漏检**（声明）。
- **UI（`0f7b62a`）**：draw 面板末位按钮（两处 tab）、catalog、**五个参数行**、**预选式提示**（零点工具 ⇒ 既有
  `TOOL_STEP_PROMPTS` 表达不了，新增窄 `PRESELECT_PROMPTS`，按宿主 `can_confirm` 判就绪，且**早于**共享的
  `tool.prompt.dim.ready` 返回 ⇒ 断言不会继承标注文案）、`hatch` 字形（16/20/26 px 实渲染核对）。
  **按钮刻意小号、无变体弹层** ⇒ 不会出现未实现的图案名/渐变入口。
- **真机证据（X11 / Quadro P620 / Vulkan；合成 DOM 指针事件、非真实鼠标）**：运行
  `/tmp/opencode/yacr-evidence-p6b-run2`，verdict `PLANNED_TIMEOUT` / exit 143，`shot-cad.png` 926×341。
  三相位各 **+1** 实体、`tool` 清空、`error` 为空；预选快照 2/1/2；参数回读与设定一致。
  **像素判据（决定性）**：`Normal` **环带 8656/8656 = 1.0000、内圆 0/1789 = 0.0000**；同几何 `Ignore`
  **环带 1.0000、内圆 1789/1789 = 1.0000** ⇒ **孔洞规则与样式派发都在屏幕上成立**（规则错则内圆被填）。
  用户自定义图案为 **11–12 条离散平行线、30.0°、间距 9.94 px = 0.05 世界 × 198.73 px/世界**（非实心）。
  两个空对照区均 **0.0000**。**`Outer` 刻意不做相位**：两环 donut 下与 `Normal` 等价（ADR §0.2），**不伪造差异**。
  参数经**状态栏步进器同一 IPC** 设置并回读；**`spacing = 0.05` 步进器不可达**（步长 0.5）⇒ 同通道按值下发，已说明。
- **证据暴露的既有缺陷（已立项，未修）**：创建出的**圆形**填充，其**派生 bulge 边界折线**渲染为
  **四段大弧 + 四段过冲直线**；**矩形填充无此现象**（边界无 bulge）；填充网格、解析边界圆与相位计数均不受影响。
  证据侧用 `cad_geometry` 的 bulge-arc 构造在屏幕空间**复现了该墨迹**，而填充采样器用另一种构造且能画出正确的圆 ⇒
  **两套构造不一致，须判明哪一侧错**（**不按"改绕向迁就渲染器"处理**）。若属显示路径，则**所有** bulge 折线
  （含导入）都受影响，须作为**渲染变化**声明。**已诊断并修复**（见下方专节：错在显示/三角化侧，且影响导入图纸）。
  **修复后重跑（`/tmp/opencode/yacr-evidence-p6b-run3`，重建二进制 sha256 `4091e116…`，硬件 Vulkan / P620）**：
  边界外环带（r 82–92，排除停驻十字与世界 X 轴两行）**275/5306 = 0.0518 → 0/5306 = 0.0000**；
  过冲（r>92）**592 → 0 px**；越过 r=84 的射线（排除 X 轴扇区）**109/537 → 0/537**；最远墨迹半径
  **91.90 → 82.20 px**；边界迹由 **651–652 单段 + 68–69 多段（碎片）** 变为 **720 单段 / 0 多段 / 0 空**（单条闭合迹）。
  全局墨迹 **58 220 → 54 719（−3 501 px）**，即被移除的"花瓣"。**填充分区逐区域计数未变**：`Normal` 1.0000/0.0000、
  `Ignore` 1.0000/1.0000、图案 30.06°/9.93 px（期望 9.94）、两对照 0.0000 ⇒ **修复移动的是边界迹，不是填充**。
- **UI 的两处诚实缺口（未绕过）**：① 两个**闭集仍是数字步进器**（`StatusBar.jsx` 只有步进器/标志，无 select；
  且 `status.tool_parameter.hatch_pattern_type.{solid,user_defined}` 与 `…hatch_style.{normal,outer,ignore}`
  **四个键不存在**，硬编码人类词会违反 i18n 铁律）⇒ 现以"数字 + 提示写明含义"呈现。② **无自由数字输入**
  （`spacing` 无法键入 0.25 这类值），零点点工具还会多显示 "0 points"。**是否补 4 个键 + 控件，待决**。
- **门禁（编排者复跑）**：受影响 crate **1960 通过 / 0 失败**（P6b 后端时点）；UI **163/163** + build；
  clippy(`-D warnings`)、fmt、architecture(27)、i18n(**534**)、file-size(**702**)、fixture(23)。
  **NOT RUN**：全量 `cargo test --workspace`（`accelerated_tier` 超时）；跨平台；第三方读取器验证。
- **收口**：**bulge 边界显示缺陷已修并复验**（`b52066e` 代码、`f4e6586` 声明、`6a9a828` 修复后证据，见下方专节）；
  **P6 阶段门禁（Oracle）= `closed-with-fixes`**：**无代码阻塞**；要求的三处文档更正（`shortcuts.md` 选择子数
  与 HATCH 创建状态、`dxf-entity-coverage.md` 创建状态、ADR 状态行缺 attempt 2 结论）与本条均已落。
  **门禁另点名**（已并入上方局限）：`fill.rs::ring_depths` 只探首点是**未声明的孪生**；以及**夹具测试只断言
  "输出非空"** 这一系统性弱点 ⇒ **新立任务：语料几何不变量断言**（对每条非零 bulge 折线断言"顶点在弦的
  `sgn(bulge)` 侧"且采样点落在解析圆容差内），**不在 P6 内做**。

## 既有缺陷修复：bulge 圆弧画在弦的**镜像侧**（P6b 证据暴露，**跨实体**可见变化，2026-10-20）

**症状**：P6b 真机证据看到创建出的**圆形**填充周围有"四段大弧 + 四段过冲直线"，**矩形填充没有**（边界无 bulge）。

**根因（测量判定，非推测）**：`append_bulge_arc` 把弧心放在**弦的顶侧**（`mid - u·r·cos(θ/2)`，正确为
`mid + u·r·cos(θ/2)`；`u` 已含 bulge 符号）⇒ **每条 bulge 弧被沿弦镜像**。`R=10`、`bulge=tan(π/8)` 的
四象限环上，画出的弧**半径达 21.95**，随后**直线**跳回下一顶点 —— 与屏幕墨迹完全一致。

**为什么既有测试全过**：半圆是**奇异**的（`cos(θ/2)=0`），恰好不变；而所有既有 bulge 测试只断言**端点、
连续性、半圆**，**对圆弧的弯曲侧不判别**。

| 环（R=10，`bulge=tan(π/8)`） | 修复前显示偏差 | 填充采样器 | 独立参考 | 修复后显示 |
|---|---|---|---|---|
| CCW `+b`（即创建的环） | **11.950121**（半径至 21.950121） | 0 | 0 | **0** |
| CCW `−b` | 0（恰在此组合重合） | 5.857864 | 5.857864 | 5.857864 |
| CW `+b` | **11.950121** | 5.857864 | 5.857864 | 5.857864 |
| CW `−b` | 0 | 0 | 0 | **0** |

⇒ 误差是**沿弦的常数镜像**、与绕向无关；零偏差只出现在 `|bulge|=1`（半圆）与两处**恰好重合**的组合。

**四处同修**（同一错误构造）：显示三角化、`EXPLODE` 的弧恢复、拾取包围盒、对象捕捉。
**第二处独立缺陷**（显示与填充一致后才暴露）：显示用 **Newell 法向的符号**（即绕向）推平面法向，而填充/导入
在 `+Z` 帧读 bulge ⇒ **顺时针 2D 环的弧被镜像**、折线填充边界可被存成 `−Z` 平面。现 **XY 平行折线一律 `+Z`**
（显示/拾取/捕捉/hatch 边界构造四处一致）。

**影响面（不只是填充）**：**所有 bulge 折线**的显示都会变，**含导入图纸**。已提交夹具确有此几何：
`flange.dxf` **6 处**非零 bulge（位移 0.042–21.65 单位 = 各弧半径的 **1.3%–397%**）、
`entities.dxf` **3 处**（其一 **251.6°** 弧、弦 80/半径 49.31，位移 **156.27 单位**）；
既有夹具测试只断言**输出非空**，因此无人抓到。`EXPLODE` 产出的弧、拾取框、bulge 弧捕捉随之移动。

**回归证据**：新增显示层测试 `tests/bulge_display.rs`（四象限环 × 正反绕向 × 正负 bulge 的**解析圆容差 +
无过冲**、四分弧根因回归、EXPLODE 弧一致性、填充/显示一致性）+ 域侧 `sample_loop` 对应 + 创建圆测试改为
**对显示边界**做三角化断言；**负向对照**：回退 tessellate 符号 ⇒ 5 条新测试中 4 条失败；回退 hatch `newell_normal`
⇒ 新的顺时针测试失败。

**残余（声明）**：① **倾斜**（非 XY）折线仍用绕向定法向**符号**——域内 `Polyline` 不存挤出法向，顺时针倾斜
bulge 环仍**歧义**（好在此刻显示/拾取/捕捉/填充**彼此一致**）；② 导入 `ocs_to_wcs` 的快速路径对 `(0,0,-1)`
法向仍走 x 镜像而不调整点与 bulge 符号 —— **未改、未测**，单独立项。

**流程事故（记录在案）**：我在该 lane 仍在写文件时执行了 `git add -A`，把它的**临时测量文件**
`crates/cad-geometry/tests/tmp_bulge_measure.rs` 与一处临时域测试**扫进了** `06e822d`；
修复提交 `b52066e` 已删除该临时文件并换成永久测试。**教训：后台 lane 在写树时禁止 `git add -A`。**

## P6a：填充族一等实体 + 样式正确填充 + 真 `HATCH` 导出（ADR-0018，2026-10-20）

**范围**：`crates/cad-domain`（新 `hatch_entity` + 迁入的 `hatch/`）、`crates/cad-geometry`（迁出 hatch 模块）、
`crates/cad-import-acadrust`（导入迁移 + 新 `export/hatch.rs`）、以及 `cad-db`/`cad-spatial`/`cad-app`/
`cad-measure`/`cad-platform`/`cad-proxy`/`cad-representation` 的匹配点。**无创建命令**（P6b）。

- **P6a0（`b639663`）**：hatch 几何 `cad-geometry` → **`cad-domain`**（`git mv`，**内容零变化**、不保留 re-export），
  **零行为变更**（断言与期望值一个未改），为的是让后续三处**已声明渲染变化**可归因。
- **P6a-1（`e8bfaf0`）**：`SemanticGeometry::Hatch`（私有字段 + `new`/`from_import`/`transformed`）；
  `display` **保留全部边界环**、样式只筛填充区域；**样式 = 环集筛选**（`Normal` 全部 / `Outer` 去 1 基深度 ≥3 /
  `Ignore` 去 ≥2）。**修三处既有缺陷（均声明为渲染变化）**：`Outer`/`Ignore` 曾按 `Normal` 填、用户自定义
  `78=0` 曾 `Partial`、**图案复合**（缩放基点 + 旋转 offset，U-7 已修）。匹配点清单全部落实（复核：**无错臂**）。
  `EXPLODE` 保持既有分解、`class_name = "AcDbHatch"`、非均匀/镜像拒绝。
- **P6a-2（`0f7e859`）**：**真 `HATCH` 导出**（定义行、种子、原始 `92` 位、优先解析边、`47` 仅 derived、
  渐变块仅渐变；**`71` 恒 0**；**永不写裁剪线段**），不再摊平 `display`；新增**合成多样式夹具**
  `fixtures/dxf/hatch-styles/multi-style.dxf`（manifest 23 个）。
- **复核（Oracle，聚焦）**：`ACCEPT-WITH-FIXES`**无阻塞**。关键判断：图案复合**不是纯循环**——一条**真判别性
  不变量**（offset ⊥ 最终方向 + 幅值缩放，**旧代码会失败**），另两条为公式自证，**无外部保真比对** ⇒
  **可保留**但"verified against fixture"**说法过强**，统一改为"**夹具值核对 + 一条形状不变量**"；
  样式派发**算术独立重算通过**（68/64/100、34/32/50 能区分三样式与两个差一阈值）；匹配清单**无错臂**。
- **复核后的修正**：`engine.rs::fallback_scale` 与 `pick/bounds.rs::cached_local_pick_bounds` 补 Hatch 臂；
  修正 `hatch_entity/tests.rs` 的**陈旧注释**（图案行实际 `y=0,2,4,6,8`）；补**非循环**的相似变换等式测试；
  两外环配置补**图案/渐变**覆盖。**ADR 的 i18n 计数更正**：P6a 实际 **+3 键**（属性行穷尽匹配所致），非 0。
- **声明与已知局限**：实心写 `2 = SOLID`；折线路径形式**强制 `92` POLYLINE 位**（记为**转换**）；法兰 6 个填充
  中 5 个在 `Title` 块、只被图纸空间引用 ⇒ **模型空间导出只出 1 个 `HATCH`**（已断言丢弃）；渐变 `460` 单位
  **未查证**（U-13，`angle_raw` 不透明往返、**无渐变夹具**）；**深度判定**：`loop_depths` 已由 `0fa3d8e`
  改为**最多 8 点均匀探测 + 确定性回退**（残余仅"探测均分时回退首顶点"），但 **`fill.rs` 的 `ring_depths`
  仍只探 `ring[0]`** 且驱动**奇偶配平** ⇒ **导入路径相切环仍可能被填错**（**待办**；创建路径拒绝相切）；
  **无图形级图案保真判据**。
- **文档更正（本轮一并）**：`docs/shortcuts.md` 的"未实现命令"整条此前引用**已删除的 `cad-ui-slint`**，
  且 `DIM` 族多已实现 ⇒ **据实重写**；`docs/compatibility.md` 的 HATCH 行拆**读取/显示**与**创建（NOT IMPLEMENTED）**、
  渐变行补 `460` 未查证与不可写损失、`EXPLODE` 行补 `Hatch`；`docs/dxf-entity-coverage.md` 补 P6a 条目。
- **门禁（编排者复跑）**：受影响 crate **1927 通过 / 0 失败**；workspace check(all-targets)、clippy(`-D warnings`)、
  fmt、architecture(27)、i18n(**520**)、fixture(**23**)、file-size(**695**)、UI **154/154**。
  **NOT RUN**：全量 `cargo test --workspace`（`accelerated_tier` 超时）；**真机像素判读**（填充分区由契约测试钉住，
  ADR 的像素读留待 P6a 证据批）；跨平台；第三方读取器验证。
- **待办**：**P6b**（创建：预选闭合实体 + `SOLID`/用户自定义图案 + 样式参数）；之后 **P6 阶段门禁**。

## P5 阶段关闭（Oracle 门禁，2026-10-20）

- **门禁**：`closed-with-fixes` → F1/F2/F3 修复 + F4 文档更正后 **P5 CLOSED**。门禁自跑受影响套件与全部卫生门禁，
  并程序比对 `COMMAND_TABLE`(67) == `EXPECTED_COMMAND_COUNT`(67)。
- **F1（阻塞）**：Diameter 组 10/15 曾写反（自洽但非约定，且**无测试**能抓到）⇒ 改代码双向对齐 + 组值断言 +
  导入方向测试；**回退验证**证明新测试在旧代码上失败。
- **F2**：presenter 尺寸守卫改为**有界等待**（2 tick 后仍不符才报错），绝不拉伸；**残留**：tick 的布局守卫
  仍可能在中途 resize 上报错（已记录，未扩宽）。
- **F3**：`ensure_fonts_for_text` 移到 `execute_engine_request`（所有编辑请求的漏斗），非 confirm 提交也装引擎；
  未覆盖：宿主 Open/New/Plot/Save 漏斗与直接用 `cad-app` 的场景。
- **F4**：四处文档腐化已就地更正（P5e NOT RUN、P5a 偏离 ⑪、`P5c/P5d`→`P5d`、`ALL` 28→30）；门禁数字标注时点。
- **门禁（编排者复跑）**：受影响 crate **1885 通过 / 0 失败** + `cad-render-wgpu` **202**；workspace check(all-targets)、
  clippy(`-D warnings`)、fmt、architecture(27)、i18n(**517**)、file-size(**689**)、UI **154/154**。
  **NOT RUN**：全量 `cargo test --workspace`（`accelerated_tier` 超时）；跨平台；第三方读取器验证。
- **带入 P6 的显式债务**：无真正创建预览（DOM 不得跨 CAD 分区浮动，ADR-0010 `:27`）；样式快照不追溯；`DIMPOST="."` 渲染为
  `42.5.`；DIMSD 断口为文字中点近似；F2 布局守卫残留；F3 未覆盖路径。

# 后续 agent 接手入口

> **当前状态横幅（2026-10-16）**
>
> 本文是**逐轮历史日志**（最新在顶部）。桌面 UI 已由 Slint 迁移为 **React + Tauri 原生宿主**
> （`apps/app-linux`，二进制 `yacr`），`cad-ui-slint` 与 Slint 依赖已删除；Windows/macOS/Android/Web
> 当前是**显式 NOT IMPLEMENTED 占位**。要了解**当前**架构与构建，先读：
> `docs/architecture.md`、`docs/adr/0010-react-linux-host-replaces-slint.md`、
> `docs/adr/0011-native-wayland-subsurface.md`、`docs/adr/0012-scene-spatial-index-and-interaction.md`、
> `docs/react-linux-app.md`、
> `docs/build.md`、`docs/ci.md`、`AGENTS.md`。下列各轮小节中的 Slint/`app-linux`(旧)/`yacr-linux`/
> 旧脚本命令等均属**当时事实**，多数已退役；请不要据旧小节推断当前可用能力。历史验证证据见
> `docs/validation*.md`，不得把旧 Slint 或软件渲染 PASS 当作 React 宿主/真实 GPU 结论。

## acadrust 0.6.3 → 0.6.5 升级与标注结论复核（2026-10-20）

**范围**：工作区依赖升到 `acadrust 0.6.5`（features `serde` + `import`，registry `rsproxy`），
`cargo update -p acadrust --precise 0.6.5`；**未改 acadrust、无 `[patch]`**。传递新增
`oem_cp 2.1.2`、`phf 0.11.3`、`phf_shared 0.11.3`（0.6.4 旧代码页解码）。

**编译修复（0.6.5 破坏性 API）**：`EntityType::Dimension(Box<Dimension>)`、
`EntityType::MText(Box<MText>)` ⇒ 导出/测试构造加 `Box::new`、测试匹配改 `as_ref()`；
`GradientColorEntry` 新增 `color_name`/`book_name` ⇒ 测试用 `..Default::default()`。

**复核 ADR-0017 中"因 acadrust 写入器而偏离"的条目**（源码逐函数 diff + 导出字节断言）：
- **组 70 位 32：仍然无法置位**（0.6.5 `write_dimension_base` 只写 `type_flags | 0x80`；`dwg_flags_byte`
  只被 DWG 写入器消费）⇒ 偏离保持，已在 ADR 标注复核。
- **组 42：未变**（仍总写、仍取 `actual_measurement`；角度弧度断言继续成立）。
- **角度组 70：仍为 2 / 5**（新增断言，导出字节实测；不是 34/37）。
- **DIMSTYLE：组码/组 3 未变**，`DimStyle` 结构零 diff；仅组 50 `DIMJOGANG` 改按度写，本项目不建模
  该变量 ⇒ 无影响。
- **0.6.5 的"dimension flag mapping"修正只改 DWG 标志字节位 0 的语义**（文字默认位置），本项目不消费
  `text_user_positioned`；DXF 组 70 `0x80` 读取未变。
- **#127 头保留 / DIMJOGANG 0.6.4**：导出新建空文档、不复用源 header ⇒ 无影响。

**往返证据**：法兰 `flange.dxf` 6 条无块标注（kind/定义点/样式/测量/文字覆盖保持）；新增
`qcad_angular_dimensions_round_trip_with_radian_group_42` 覆盖 `entities.dxf` 的 8 条角度标注
（kind/样式/测量保持，**组 42 仍为弧度**）。诚实边界：该夹具角度标注是块成员，导出烘焙 INSERT 变换
⇒ 定义点由块坐标变世界坐标（既有有损导出行为），故该测试不断言点相等。

**门禁（本机执行）**：workspace check(all-targets)、wasm lib check、`cad-import-acadrust`（197 + 新增
1 条 = 198 通过）、`cad-app`、`cad-cli-tools`、clippy(workspace `-D warnings`)、fmt、architecture、
i18n、file-size、fixture、workflow **全部通过**。**NOT RUN**：真实 GPU 帧（无渲染/UI 行为变化）、
全量 `cargo test --workspace`、跨平台、vendor 兼容。文档更新见 `docs/compatibility.md`「0.6.5 增量」、
`docs/adr/0017-dimension-family.md`「P5d」。

## 人工复测修复：视口十字光标 + 对象捕捉标记（2026-10-19，**非** ADR-0017 范围）

**来源**：用户人工复测报告两处缺陷（视口是系统默认箭头、捕捉标记过小且形状不辨）。**不是** ADR-0017 的一部分，属交互/overlay 修复。

**根因（三层，逐条有据）**：
1. **光标从未被设置**——仓库无 GTK cursor 代码、视口无 cursor CSS 规则；
2. **捕捉标记尺寸是世界单位且宿主从未覆盖**——`PreviewOptions::marker_size` 文档为 "in world units"，三处构造均用 `PreviewOptions::default()`（`render_scene/mod.rs:360/367/385`）⇒ 屏幕尺寸随缩放漂移；
3. **字形与 AutoCAD 对调**（中点/象限）+ **标记与拾取框同级重叠且拾取框绘制在其上** ⇒ 独立像素判读全帧只找到一个闭合轮廓（拾取框），形状无从分辨。

**落地**：
- **十字光标 + 拾取框**走 renderer overlay（新模块 `overlay/cursor.rs`，`CURSOR_DRAW_ORDER = 2_500_000`）；拾取框半边长 = `pickbox_logical_pixels`(3) × 视图比例（**与点击拾取同一常量**，故框的内接圆恰是一次点击能命中的范围）。颜色 `[0.72,0.75,0.80]`（**偏离 AutoCAD 的白**——白在默认纯白几何上不可见）。
- **系统光标只在宿主确实在画十字时置空**：新增诚实的 `cad_status.crosshair: bool`（取自 `overlays().cursor.is_some()`），React 侧加 `.is-crosshair`；缺失字段保留箭头。**3D 永远保留箭头**（overlay 纯世界空间，3D 无有效视口矩形）。**已记录局限**：随 ~250 ms 轮询，进入 2D 时箭头最多滞留一个周期。
- **捕捉标记改为像素尺寸**（`SNAP_MARKER_HALF_EXTENT_PIXELS = 9.0` ⇒ 18 px；拾取框 6 px ⇒ **3:1**），**层级提到十字之上**（3 000 000 > 2 500 000；十字仍在工具预览 2 000 000 之上）。**字形按 AutoCAD 约定**：端点=方块、**中点=三角**、圆心=圆、**象限=菱形**、**垂足=⊥**、交点=X；`snap_marker_glyphs_match_autocad_per_kind` 钉住每种类型的**精确线段集合**。

**修掉一个本轮引入的回归**：3D 下光标曾被置空却无十字 ⇒ **3D 完全没有指针**（比修复前更糟）。现由上述 `crosshair` 标志控制。

**真机证据（X11 / Quadro P620 / Vulkan；合成 DOM 指针事件、**非**真实鼠标）**：指针在捕捉点上 → `snapKind:"endpoint"` ×26，**琥珀方块 19×24 px** 与 **6×7 px 拾取框同心且可分辨**；偏离 10 px → 两者**并排**仍可分辨；**中点** → **三角形**（与端点方块明显不同）。十字真机帧：贯穿视口的横线 x 0→959、竖线 y 0→438，交点与光标位置吻合；拾取框跨度与 `pickbox_logical_pixels` × 视图比例一致。
**诚实边界**：① 夹具只有直线 ⇒ 只拍到端点与中点两种字形，⊥/菱形/圆/X **未拍照**（测试钉住）；② 截图仅含应用窗口自身；③ `cad_status` 无几何通道 ⇒ 只证"计数 + 内容存在"；④ 像素判读发现**正方形标记渲染为 19×24 px**——那是**并行的既有拉伸缺陷**（见下），不是尺寸 bug。

**已记录的偏离**：十字颜色非白；拾取框按**半径**语义（框边 = 2×PICKBOX 逻辑像素）；捕捉标记尺寸与层级是**我们的选择**（AutoCAD 未给出数值）。

## 既有缺陷立项：视图纵向拉伸 ≈1.24×（2026-10-19 发现，**非**本轮引入）

**现象**（三条独立证据）：夹具 1 世界单位横向 **205.7 px** / 纵向 **251–256 px**；**正方形拾取框渲染为 13×17 px**；橡皮筋屏幕斜率反推 **1.24**。

**根因（@oracle 只读诊断，定论级）**：**两个从未互相校验的尺寸来源**——① 投影/相机拟合/**全部指针输入**用 **React DOM 的 CAD 矩形**（`shell.input.last_rect()` → `cad-engine` → `render-wgpu`，纹理内部各向同性）；② **GTK 表面**用**绘图区分配尺寸**（失败运行中恒为 **960×439**，来自 `gtk_bridge/owner.rs:52-53` 的**硬编码安装期边距** `set_margin_top(101)`/`bottom(60)`）。拉伸发生在最后一步：`present_texture` 用**全屏三角形**把纹理贴满表面，**无纵横比校正**，且**从不校验纹理尺寸与表面配置尺寸** ⇒ 因子 `(960/926, 439/341) = (1.037, 1.287)`，各向异性 **1.241 = 实测斜率 1.24**。**"持久双源错配，其对自身对齐是竞态的"**：设计意图是用边距把绘图区恰好对齐 DOM 矩形，但重分配异步不可靠——某次运行**整轮未落上**（全程拉伸），另一次最终落上 `resize configure 926x341`（不拉伸，**这解释了更早法兰帧里圆是圆的**）。

**输入受影响**：输入/拾取/捕捉与投影**共用同一 DOM 尺寸**故**彼此一致**（驱动的合成点击确实落在其意图的世界目标上），但**与显示像素不一致**（差贴图变换**外加原点偏移**：表面原点 (0,101) vs DOM 矩形 (0,175)）⇒ **人眼可见的"指针与所指特征"对应关系是坏的**。**这直接影响本轮的光标修复**：十字画在模型空间，会跟着几何一起被拉伸并偏移，于是出现在"应用以为指针在的地方"而非"指针真正在的地方"。

**修法（oracle 指定）**：① **真修**——`apps/app-linux/src/gtk_bridge/owner.rs` 让布局的 inset **真正落到绘图区分配**（今日安装期边距获胜、`set_margin_*` 不可靠地产生 size-allocate；须强制布局并通过并**断言** `drawing_area == dom rect` 而非照样呈现）；② **守卫**（纵深防御，铁律 2）——`crates/cad-render-wgpu/src/native_presenter/presenter.rs:538` 的 `present_texture` **拒绝**纹理尺寸 ≠ 表面配置尺寸，**显式报错**；**不得**改成"按表面尺寸渲染"（那样图像方了但**输入错位**，因为输入走 DOM 矩形）。

**修复结果（2026-10-19，已落地）**：根因比初诊更精确——`reparent_webview` 把绘图区从 `GtkOverlay` 的**主 child** 移成 **overlay child** 后，GTK 把位置放在一个**中间原生窗口**里；`set_margin_*` 只**排入一个宿主从不排空的 resize**，而 **GtkOverlay 对 overlay child 忽略位置边距** ⇒ 安装期边距 101/60 **永久生效**。修法：`apply_layout` 显式驱动布局（沿祖先链 `queue_resize` + `GtkWindow::check_resize()`，**同步**完成尺寸协商），`tick` 在呈现前**校验** `drawing_area == dom rect`（±1 逻辑像素、`None` 永不算匹配），不一致先驱动一次，**仍不一致则显式报错**而不是画出去；`present_texture` 对尺寸不匹配返回 `ProbeError::TextureInvalid`。
**验证（竞态缺陷，故多次运行）**：修前基线比例 **1.174**（横 304.0 / 纵 259.0 px/单位，表面 960×439 vs 纹理 626×341）；**修后 4 次运行全部 0.987**（1166×541，横 198.0 / 纵 200.7 px/单位），拾取框 **8×8 px、纵横比 1.000**（修前 13×17），零 `does not match the DOM` / `refusing to stretch` / 呈现错误，每次尺寸变化都触发 `resize configure`。**这是既有缺陷**（2026-10-18 的运行里同样可见），修好后此前"圆是圆的"那次只是**恰好对齐**。

## P5e：`DIMANGULAR` + `DIMORDINATE`；U-8 以真实夹具结案；标注文字缺陷修复（ADR-0017 决策 C，2026-10-19）

**范围**：`crates/cad-app`（命令/工具/几何/宿主映射）、`crates/cad-domain`（角度合成）、`crates/cad-import-acadrust`
（角度组 42 单位 + 夹具钉住）、`crates/cad-ui-core`（选择子/i18n）、React 标注面板；另含**标注文字渲染**的跨层修复
（`cad-platform`/`cad-representation`/`cad-host`/`cad-cli-tools`）。

- **命令面**：`DIMANGULAR`（**两线交互形式**：两次实体拾取 + 标注弧位置）与 `DIMORDINATE`（特征点 + 引线端点）；
  `EXPECTED_COMMAND_COUNT` 65 → 67；单一插入漏斗、一条事务/一步撤销、活动图层、显式命名拒绝且拒绝时不开事务。
- **U-8 结案（真实工件）**：解码 `fixtures/dxf/qcad-examples/*` 中 **9 条 type flag=34** 的角度标注；判别用例 `*D65`
  （`14 ≠ 15` 且 15 不是交点）的 **组 42 = 1.975688113 rad = 113.19859°** 只匹配"**顶点 = 两条无限直线的交点**"，
  且匿名块 **ARC 圆心在交点** ⇒ line 1 = `13→14`、line 2 = `15→10`、**组 16 选扇区**。九条全部数值复现；两条测试钉住
  （夹具 + 域）。**U-8 不再是未查证项**。
- **修掉一处潜伏单位损坏**：角度标注导出组 42 曾写**度**（DXF 语义为**弧度**）⇒ 旧导出**数值错误**。现文字仍用度、
  **导出租 42 转弧度**，并加"弧度 vs 度数文字"对照测试。
- **声明的选择**：U-1 四选一 = **包含弧位置拾取的扇区**（落在边界射线取逆时针扇区，三点同规则）——**我们的规则**；
  U-5 自动 X/Y = `|Δy| > |Δx|` ⇒ X，否则 **Y（相等取 Y）**——**我们的规则**；三点形式**仅命令/载荷层**
  （固定步数状态机表达不了"选线**或**指定顶点"，**无交互捕获**）；两线形式**只收 `Line`**；
  ordinate 基准 = 世界原点、量值取绝对值（**带符号未建模**）、零 ordinate 合法；角度合成改为交点顶点后
  **已导入角度标注的渲染会变**（正确性修复 = 像素变化）。
- **标注文字缺陷（P5 暴露，已修）**：独立全域像素判读发现该帧**全无字形簇、同帧所有标注都没有文字**。
  根因**三道闸**：① `load_engine` 在 `requested`/`explicit` 皆空时**在使用 `font_dir` 之前**早退（`--fonts-dir`
  也救不回）；② 表示层**只在 `font: Some(..)` 时整形**（自建标注 `text_font: None` ⇒ 永不整形）；③ 字体只在
  **打开漏斗**安装（新建标注后不重装）。修复：`needs_text_shaping(database)` 与"是否命名字体"解耦、`shape_unstyled`
  走**既有回退链**（无引擎 ⇒ `ResourceMissing`，**绝不伪造字形**）、提交漏斗内 `ensure_fonts_for_text`；
  **保留**"无文字就不加载多兆字体"的优化（有测试钉住）。
- **真机证据（X11 / Quadro P620 / Vulkan；合成 DOM 指针事件、非真实鼠标）**：`DIMANGULAR` 41→42、`DIMORDINATE`
  42→43（各 **+1**），工具清空、无错误、27 个派发 0 错误。**空间判据**：两线取样 1.000/1.000；角度弧在 `r=0.25`
  的 3°…42° **40/40 = 1.000**，而**同半径空扇区 0/31 = 0.000**、`r=0.60` 对照 0/36 ⇒ **画的是选中扇区**。
  **文字修复后**：与修复前帧差分 **+1856 墨迹像素 / 0 移除**；读到 **"0.1"**（`DIMORDINATE`）、**"0.45"**、
  **"R0.18"`**；`DIMLINEAR`/`DIMANGULAR` 文字**被画布顶边裁切**（驱动固定放置所致）⇒ 路径已证、`DIMANGULAR`
  完整字形**未测量**。
- **诚实边界**：合成 DOM 事件非真实鼠标；`cad_status` **无几何通道** ⇒ 真机只证"计数 + 无错误 + 内容存在"，
  **哪一扇区被量 / U-8 / 组 42 弧度**均由契约测试钉住；截图仅含应用窗口自身；单次有界本地运行，无跨平台结论。
- **门禁（编排者复跑）**：受影响 crate **1881 通过 / 0 失败**；workspace check(all-targets)、wasm、clippy(`-D warnings`)、
  fmt、architecture(27)、i18n(**517**)、fixture(22)、file-size(**682**)、workflow、UI **154/154** + build 全通过。
  **NOT RUN**：全量 `cargo test --workspace`（`accelerated_tier` 超时）；跨平台；第三方读取器验证。





## P5d：`DIMRADIUS` + `DIMDIAMETER` + `DIMCENTER`（ADR-0017 决策 C，2026-10-19）

**范围**：`crates/cad-app`（`command_id.rs` 三个变体 + `app_drawing/dimension.rs` 命令体与纯辅助 + `draw_tool/{kind,mod}.rs`
工具种类、`picked_step_count` 与 `DrawIntent` + `app_drawing/trim_extend.rs` 的**批量漏斗** `commit_new_entities` +
`host/draw_intent_map.rs` 映射 + 契约测试）、`crates/cad-ui-core`（选择子 + `BeginDraw` 映射 +
`EXPECTED_COMMAND_COUNT` **62→65** + i18n **+9** 键）。**未**改 acadrust/架构边界；创建命令**非关联**；**未**碰 React UI
（`apps/app-linux/ui/**` 由 designer lane 落地）。

- **命令面**：`CommandId::{DimRadius,DimDiameter,DimCenter}`（`requires_work_mode()`）；选择子
  `("DIMRADIUS", …)`/`("DIMDIAMETER", …)`/`("DIMCENTER", …)`；`EXPECTED_COMMAND_COUNT` **62 → 65**。
- **绘制工具**：`DrawToolKind::{DimRadius,DimDiameter,DimCenter}`（`ALL` **28** 种）；radial 对 `Exact(2)`、
  `DimCenter` `Exact(1)`；机器键 `"dimradius"`/`"dimdiameter"`/`"dimcenter"`。
- **混合拾取（新机制）**：`DrawToolKind::picked_step_count()`（前导实体拾取步数；radial 族 = 1）；`needs_picked_entities()`
  改为 `picked_step_count() > 0`；`push_point`/`push_pick`/`requires_entity_pick`/`is_ready`/`intent` 改读它 ⇒ radial 首击经
  宿主 `capture_tool_point` 的 `draw_requires_entity_pick` 路由为**实体拾取**，次击为自由点。
- **`DrawIntent` 变体**：`DimRadius{target,location}`/`DimDiameter{…}`/`DimCenter{target}`；宿主映射为**新增** payload
  `DimRadial(Box<RadialDimensionCommand>)` / `DimCenter(SelectionRef)`。
- **命令实现**（`app_drawing/dimension.rs`）：radial 对解析 `Arc`/`Circle` → 位置拾取定方向 → 组 15 = 圆上点、组 10 =
  圆心（半径）或对径点（直径）、组 40 = 0.0 → 共享合成（`Standard` 快照）→ `commit_new_entity` 插入**一个** `AcDbDimension`。
  `DIMCENTER` 经**批量漏斗** `commit_new_entities` 插入**普通 LINE**（`AcDbLine`，**非** Dimension）。
- **声明的决策（逐条写入 ADR-0017 P5d「声明的决策」与 `docs/compatibility.md`）**：① 位置拾取**只定方向**，文字由合成放在
  引线中点（"位置定文字位置"未实现，声明）；② `Arc` 引线点落在**该弧的圆**上、**不钳制**扫掠（我们的读法）；③ 组 40 写 `0.0`
  （引线延长未建模）；④ `DIMCEN` 读数：`>0` 十字标记（臂 `|DIMCEN|`）/`<0` 十字中心线（达 `radius+|DIMCEN|`）/`0` 无——
  **近似声明**：AutoCAD 负值"中心标记 + 带缺口中心线"变体**未建模**（普通十字，同径向合成 P5b 偏离 3），ADR 述"LINE/ARC"
  而**我们只产 LINE**；⑤ `DIMCENTER` 十字对齐**世界 X/Y 轴**；⑥ `DIMCEN=0` 产 0 实体 + 显式诊断 + **无事务**（同 ARRAY 1×1）；
  ⑦ 平面外拒绝 = ADR D.6（法向含 X/Y 分量即拒；`−Z` 接受；位置 `z` 须共面）；⑧ **退化半径**经校验数据库**不可达**
  （`cad-db` validate 已拒），故在**纯辅助单测**上按名钉住，命令层为防御性；⑨ `DIMCENTER` 按**绘图辅助**记录（D.7）；
  ⑩ 沿用 P5c：**无真正标注预览**。
- **门禁（本 lane 复跑，2026-10-19）**：`cargo check --workspace --all-targets --locked` 通过；
  `cargo test -p cad-app -p cad-ui-core -p cad-domain -p cad-db -p cad-import-acadrust --locked` 全通过
  （cad-app **663**、cad-ui-core **140**、cad-domain **53**、cad-db **84**、cad-import-acadrust **195**）；
  clippy(workspace `-D warnings`)、fmt、architecture(27)、file-size(**673** 文件)、i18n(**507** 键，P5d **+9**) 通过；
  wasm lib check 通过（含 1 条**既有** `cad-engine` dead-code 警告）；`apps/app-linux/ui && npm test` **133/133**。
- **受影响 crate 契约测试**：见 `crates/cad-app/src/tests/drawing/dimension_radial.rs`、
  `crates/cad-app/src/draw_tool/tests/dimension.rs`、`crates/cad-app/src/host/tests/dimension.rs`、
  `crates/cad-app/src/app_drawing/dimension.rs`（纯辅助单测）、`crates/cad-ui-core/.../command_vocabulary.rs`、
  `crates/cad-ui-core/.../tests_tools.rs`。
- **NOT RUN / 未证**：**真机证据**（本 lane 未跑；`cad_status` 无几何通道，沿用 P4/P5c 边界）；全量
  `cargo test --workspace`（`accelerated_tier` 超时）；跨平台；**第三方读取器验证**（本机无 `ezdxf`/`dwgread`/`dxfgrabber`）。
- **待办**：P5e（`DIMANGULAR`+`DIMORDINATE`）；P5f 已降级 NOT IMPLEMENTED；**React 标注面板/每步提示**（P5d 命令由 designer lane 接线）。

## P5c：`DIMLINEAR` + `DIMALIGNED` 创建命令（ADR-0017 决策 C，2026-10-19）

**范围**：`crates/cad-app`（`command_id.rs` 两个变体 + `app_drawing/dimension.rs` 命令体 + `draw_tool/{kind,mod}.rs`
工具种类与 `DrawIntent` + `host/draw_intent_map.rs` 映射 + 合成/宿主契约测试）、`crates/cad-ui-core`（选择子 +
`InteractionRequest::BeginDraw` 映射 + `EXPECTED_COMMAND_COUNT` 60→62）、React 宿主 UI
（`ribbon-model.mjs`/`commands.mjs`/`icons.jsx`/`cad.mjs`/`StatusBar.jsx` + 测试 + i18n 9 键）。
**未**改 acadrust/架构边界；创建命令**非关联**。

- **命令面**：`CommandId::{DimLinear,DimAligned}`（`requires_work_mode()`）贯通命令面；`cad-ui-core` 选择子表
  `("DIMLINEAR", …)`/`("DIMALIGNED", …)` 与 `session/mod.rs` 的 `BeginDraw(DrawToolKind::{DimLinear,DimAligned})`
  映射；`EXPECTED_COMMAND_COUNT` **60 → 62** 断言 + 选择子契约同步。
- **绘制工具**：`DrawToolKind::{DimLinear,DimAligned}`（`ALL` 25 种），`arity = PointArity::Exact(3)`；
  机器键 `"dimlinear"`/`"dimaligned"`；`DrawIntent::{DimLinear,DimAligned}{first,second,location}` 经
  `draw_intent_map.rs` 映射为 **复用**的 `CommandPayload::Points([first,second,location])`。
- **命令实现**（`app_drawing/dimension.rs`）：校验 → 从**当前样式** `Standard`（P5b `resolve_dim_style`）解析
  快照 → 经**共享合成** `DimensionGeometry::synthesized` 构造 → 经**单一漏斗** `commit_new_entity` 插入**一个**
  `AcDbDimension` 实体（一条事务、一步撤销、活动图层）。
- **React 面**：标注族放在既有 annotate 面板**首位**（linear 在前、aligned 在后）；专用图标（两竖直尺寸界线 +
  尺寸线，aligned 旋转 45°）；`TOOL_STEP_PROMPTS` 按**宿主真实** `tool.key`/`tool.points` 派生每步提示
  （`StatusBar.jsx` 经 `toolStepPromptKey`），**从不**来自本地镜像。
- **声明的决策（逐条写入 ADR-0017 P5c「声明的决策」与 `docs/compatibility.md` DIMENSION 子表）**：
  ① 三点拾取**复用** `CommandPayload::Points`，**不新增** payload 变体；② **`DIMLINEAR` 的 H/V 规则**——以两原点
  **中点**为基准 `|Δy| > |Δx|` ⇒ 水平（`rotation 0`，量 X）否则垂直（`rotation π/2`，量 Y）；**平局（含中点拾取）
  是显式拒绝**（标记为**我们的选择**，AutoCAD 对该退化拾取**未查证**）；③ **"非平面视图"拒绝 = ADR D.6 的平面外**
  情形，**不是**相机检查（命令接收世界点、无视口依赖）；④ **额外拒绝：零测量值**（所选方向把两原点投影到同一点）
  按名拒绝；⑤ **当前样式硬编码 `Standard`**（B.3：无交互式样式管理器），数据库无它时**防御性显式拒绝**；
  ⑥ 每步提示**派生自宿主真实工具状态**，`DIMLINEAR` 位置提示写明决定水平/垂直、`DIMALIGNED` **刻意不写**；
  ⑦ **声明缺口——放置期间无真正标注预览**（见下）；⑧ **未加 ribbon 激活态高亮**（今天无命令有，故与邻居一致）；
  ⑨ **潜在发现**：后端 `DrawPreview::status_line()` 携带**硬编码中文**、**不在 React 线上**（`CadTool` 只有
  `key/points/ready/openEnded/params`）——**既有且已冻结**（`tests/cjk_baseline.rs` 仅在**增长**时失败），P5c
  **未新增** CJK，记为**已基线化债务**。
- **声明缺口（必须记）**：放置期间**没有真正的标注预览**——实时预览是后端**橡皮筋链**
  （`render_scene/overlay/preview.rs` 把两种 kind 路由到 `preview_chain`），用户看到已捕获原点 + 到光标的链，
  **看不到**合成标注（尺寸线/尺寸界线/箭头/测量文字）。真预览需**后端 overlay 分支**；React UI **无法**绘制
  overlay（原生 CAD 分区是 X11 子窗口 / Wayland 子表面，DOM 不得浮于其上——ADR-0010 `:27`）。React 宿主因此**改为每步提示**。**不**当作已完成。
- **真机证据（2026-10-19）**：真实硬件 GPU（Vulkan / **Quadro P620** / `device_type=discrete_gpu`）+ X11；驱动扩展
  `YACR_PROBE_DRAW_EVIDENCE` 的标注相位（合成 DOM 指针事件、**非真实鼠标**）：派发选择子 → **三次合成点击** → CONFIRM。
  **`DIMLINEAR` `entityCount` 30→31（+1）、`DIMALIGNED` 31→32（+1）**，两者 `after_tool` 为 `null`（工具清空）、
  `after_error` 为 `-`，无任何 `*_failed`；总点击 50（此前 44 + 新 6）。运行目录
  `/tmp/opencode/yacr-evidence-20261019T111646Z`（verdict `PLANNED_TIMEOUT` / exit 143 属预期，截图 `captured`）。
  **像素判据**：墨迹恰好出现在两个标注的世界区域（DIMLINEAR 区 0.1003、DIMALIGNED 区 0.0405，对比此前无标注帧的
  0.0015 / 0.0017）；`DIMLINEAR` 的尺寸线横跨 cols 839–944，**两端有箭头**（端点带墨迹 0.42 / 0.40）。
  **诚实边界**：① 输入是 Rust 经 `webview.eval` 派发的**合成 DOM 指针事件**（不经 evdev/XTest）；② `cad_status`
  **无几何通道**（报告含 `dimension_geometry_channel=none_...`）⇒ 真机只证"**提交成功 + 计数 + 无错误 + 内容存在**"，
  **几何正确性由 `cad-app` 契约测试钉住**（15 条通过）；③ 截图仅含**应用窗口自身**（X11 `XGetImage`），非根/整屏抓取，
  不含视觉正确性/vendor 结论；④ 对比基线帧**窗口尺寸不同**（926×341 vs 960×439），故为**世界区域对齐**比较，**非**像素相同。
- **门禁（编排者复跑，2026-10-19）**：受影响 crate **1814 通过 / 0 失败**（`cad-app` 595、`cad-ui-core` 140）；
  workspace check(all-targets)、wasm lib、clippy(workspace `-D warnings`)、fmt、architecture(27)、i18n(**498**，P5c **+9**)、
  fixture(22)、file-size(**668**)、workflow、UI **133/133** + build 全通过。
- **受影响 crate 契约测试**：见
  `crates/cad-app/src/tests/drawing/dimension.rs`、`crates/cad-app/src/draw_tool/tests.rs`、
  `crates/cad-app/src/host/tests/dimension.rs`、`crates/cad-ui-core/.../command_vocabulary.rs`、React
  `apps/app-linux/ui/test/{dimension-tools,commands,i18n}.test.mjs`。**本文件 lane 已跑**：
  `python3 scripts/check-i18n.py`（**498** 键，P5c **+9**）与 `python3 scripts/check-file-size.py`（**668** 文件）
  均通过。
- **NOT RUN / 未证**：全量 `cargo test --workspace`（`accelerated_tier` 超时）；本文件 lane 未独立复跑其余门禁；
  跨平台；**第三方读取器验证**（本机无 `ezdxf`/`dwgread`/`dxfgrabber`）。
- **待办**：P5d（`DIMRADIUS`+`DIMDIAMETER`+`DIMCENTER`）；P5e（`DIMANGULAR`+`DIMORDINATE`）；P5f 已降级 NOT IMPLEMENTED。

## P5b：DIMSTYLE 表 + B.2(a)/(b) 子集 + 公制默认 + 导入/导出接线（ADR-0017 决策 B，2026-10-19）

**范围**：`crates/cad-db`（新 `dim_style.rs`：`DimStyle` 表 + `standard_metric`/`resolved`/`resolve_dim_style`/
`dim_style_by_name`/`dim_style(s)`/`insert_dim_style`）、`crates/cad-domain`（`DimensionStyleValues` 扩到
B.2(a) 全集 + 合成/格式化消费 + `transformed` 缩放快照）、`crates/cad-import-acadrust`（`read_dim_styles` 导入
填充 + 导出 DIMSTYLE 记录来源拆分 + `DIMCEN` 直径修复）。**无 UI 命令**；**未**改 acadrust/架构边界。

- **`cad-db` 新增 `DimStyle` 表**（`crates/cad-db/src/dim_style.rs`）：承载 **B.2(a)+B.2(b) 全量变量**；
  `standard_metric` 为**钉住的公制默认**（决策 B.1/N7）；`resolved` 把 `DIMSCALE` **烘进各尺寸**（不含测量值）；
  创建时 API `resolve_dim_style`/`dim_style_by_name`/`dim_style(s)` **供 P5c 使用**；`finish` 保证总有 `Standard`。
- **`DimensionStyleValues` 扩到 B.2(a) 全集，且每个变量都被真正消费**（消费映射逐行写入 ADR-0017 P5b 表）：
  `DIMTXT`/`DIMASZ`/`DIMEXO`/`DIMEXE`/`DIMGAP`/`DIMTAD`/`DIMTVP`/`DIMSCALE`/`DIMDEC`/`DIMZIN`/`DIMLFAC`/`DIMRND`/
  `DIMPOST`/`DIMDSEP`/`DIMLUNIT`/`DIMCEN`/`DIMTXSTY`/`DIMSE1/2`/`DIMSD1/2`。
- **导入填充**：`read_dim_styles`（`dimension/values.rs`）读入 DIMSTYLE 表 → `cad-db` `DimStyle`；缺失样式**回退到
  钉住的 `Standard`**（非沉默的任意默认）。
- **导出（N1 拆分）**：DIMSTYLE 记录携带**表中**的 B.2(b) 值，而**两个驱动显示的变量（`DIMLUNIT`/`DIMDSEP`）
  从实体快照写出**——它们在实体里真正喂了格式化；该拆分**显式声明**。
- **`transformed` 按均匀缩放同步缩放样式快照（N2）**，使导出记录的尺寸匹配**被缩放后**的显示。
- **`DIMCEN` 直径修复（B1）**：中心标记画在**真实圆心**（`Diameter` 为两弦点中点，非组 10 的对径点）；
  中心线**触及半径相对**（`radius + |DIMCEN|`）。
- **F.4 债务结清**：两条 `DIMLUNIT` 诚实测试落地——`dimlunit_other_than_decimal_is_a_declared_degradation`
  （`cad-domain/src/dimension_tests.rs`）与 `non_decimal_dimlunit_is_declared_partial_not_a_silent_decimal`
  （`cad-import-acadrust/src/tests/dimension.rs`）。
- **i18n：0 个新键**——P5b **无 UI 表面**；计划里 P5c 行写的 "+ i18n" 指 **P5b**，诚实计数为**零**（与 B.3 一致）。
- **偏离/决策（逐条写入 ADR-0017 P5b「偏离/决策」与 `docs/compatibility.md` 的 DIMENSION 子表）**：
  ① **渲染增量**——P5b 确实改变已导入标注的渲染输出（一般），是**正确性改进也是像素变化**；**已提交 QCAD flange
  经验证不变**（其样式只设 DIMASZ 2.5/DIMEXO 0.625/DIMEXE 1.25/DIMTIH 0/DIMTAD 1/DIMZIN 8/DIMTXT 2.5/
  DIMGAP 0.625，六条全 aligned/rotated linear 无 radial），**"不变"只限该夹具**；② `DIMCEN` 默认**刻意钉 0.0**
  （非文档公制 2.5）——不静默改导入标注，但**新建** radial 标注在 AutoCAD 公制会画 2.5 处**不画**；③ `DIMCEN`
  其余近似（AutoCAD 只在标注线落圆外画标记、**我们总是画**；负 `DIMCEN` 的带缺口中心线我们画**普通长十字**）；
  ④ `DIMTAD=3`（JIS）为独立代码但**垂直放置等于"上方"**，JIS 特有水平放置/文字适配**未实现**；⑤ `DIMSD1/2`
  在**文字中点**分割标注线（文字间隙断口的**近似**）；⑥ `DIMRND` 只取整**显示文字**（组 42 保留未取整）；⑦ `DIMPOST`
  无 `<>` 按**后缀**处理、`DIMPOST="."` 的"无后缀"**未遵循**；⑧ `DIMLUNIT≠2` 为**声明降级**（导入 `Partial`）、
  `DIMZIN` 英尺/英寸位未实现；⑨ `DIMCONTINUEMODE` 存储但**不可写**；⑩ `DIMTAD`/`DIMTVP` 仅 linear/aligned；
  ⑪ DIMSTYLE 记录来源拆分 + NOT-IMPLEMENTED 变量写**中性值** + `DIMDLI`/`DIMDLE` 未缩放 + `DIMSCALE` 不保留；
  ⑫ `insert_dim_style` **拒绝**已被别的名字占用的 id；⑬ **取代 P5a 偏离 11 的两条现已错误表述**（"DIMTAD 2/3/4
  塌缩为 1" 与 "(b) 写未钉住默认值" 均**不再成立**）。
- **B.2 拆分更正（追加注记）**：`DIMLUNIT`/`DIMDSEP` **被格式化消费**故实为 (a)；`DIMZIN` 现消费 **bits 2+3**
  （B.2(a)"仅 bit 3"格过时）；`DIMSE/DIMSD` 仅 **linear/aligned**；`DIMGAP` 只影响**文字位置**，标注线断口**未建模**。
- **决策 E 更新（追加注记）**：**U-4 结清**（公制 `DIMCEN` 初值 2.5，我们**刻意偏离**为 0.0，记为"已文档化、
  刻意偏离"而非"未查证"）；**`DIMTAD=2` 从推断升级为文档化**（Autodesk："背离定义点的一侧"）；**新增 U 行**：
  `DIMTIH`/`DIMTOH`/`DIMTOFL` 断言为"Autodesk 公制 Standard"，但只核实了**英制**初值（On/On，同 acadrust
  `dxf_defaults`），文档化公制（ISO-25）为 off/off——B.2(b) 显示忽略故**显示安全**，但断言**需出处或降级**。
- **门禁状态（诚实）**：受影响 crate 契约测试见 `cad-domain/src/dimension_tests.rs`、
  `cad-import-acadrust/src/tests/dimension.rs`、`crates/cad-db/src/dim_style.rs` 内联测试。
  **P5b gate 判定 = ACCEPT-WITH-FIXES（无阻塞）**；门禁**抓到两件事**：① **`DIMCEN` 直径 bug（B1）**——中心标记
  画在组 10（对径点）而非**真实圆心**、中心线触及按引线全长而非**半径相对**；② **渲染增量（B2）**——P5b 会改变
  已导入标注的渲染输出（正确性改进 + 像素变化），须**限定"不变"断言到 flange 夹具**。两者均已修/已声明。
  **门禁（编排者复跑，2026-10-19）**：受影响 crate **1803 通过 / 0 失败**；`cargo check --workspace --all-targets --locked`、
  wasm lib、clippy(workspace `-D warnings`)、fmt、architecture（27 包）、i18n（**489** 键，**P5b 新增 0 键**——
  它无 UI 表面）、fixture（22）、file-size（**666**）、workflow、UI **126/126** + build。
- **NOT RUN / 未证**：全量 `cargo test --workspace`（`accelerated_tier` 超时）；**真机证据**（P5b 无 UI 表面，
  未跑）；跨平台；**第三方读取器验证**（本机无 ezdxf/dwgread/dxfgrabber）。
- **待办**：P5c（`DIMLINEAR` + `DIMALIGNED`，将使用 P5b 的创建时解析 API）；P5d/P5e；P5f 已降级 NOT IMPLEMENTED。

## P5a：标注族域变体 + 导入迁移 + 导出（ADR-0017，2026-10-19）

**范围**：`crates/cad-domain`（新 `dimension.rs`/`dimension_synthesis.rs`/`dimension_tests.rs`）、
`crates/cad-db`（校验/变换/包围盒）、`crates/cad-geometry`（离散化/第二变换面）、`crates/cad-representation`
（显示分派）、`crates/cad-import-acadrust`（导入迁移 + 导出 + 匹配臂）、`crates/cad-measure`/`cad-spatial`/
`cad-proxy`/`cad-platform`/`cad-app`（通配符臂）。**未**改 acadrust/架构边界；**无创建命令**。

- **域表示**：`SemanticGeometry::Dimension(Box<DimensionGeometry>)`；`display` 私有、唯一写入路径
  `synthesized`/`from_picture`/`transformed`；`DimensionDefinition` = kind + 逐 kind 定义点 + 组 11/53 +
  组 210 法向。
- **合成下沉**：合成原样迁入 `cad-domain`（创建与无块导入共用同一构造器，数值不变）；旧导入中间体改名
  `SynthesizedDimension`，域类型保留 `DimensionGeometry`。
- **导入两分支都迁**：无块 → 合成进 `Dimension`；有匿名块（`*D…`）→ `from_picture`（`display` = 展开块成员）；
  **圆弧长/大半径不迁**，保留旧 `Compound` 合成。
- **导出**：真实 DXF `DIMENSION` + 生成 `*D…` 内容块（成员 = `display`）+ 组 42 + `add_or_replace` DIMSTYLE
  记录（组 3 不悬空）；`ExportReport` 逐条声明形状与丢弃（`export.converted.dimension`）。
- **通配符臂**：A.8 的 12 处 + 全部穷尽臂已补显式 `Dimension` 臂；`class_name` 映射 `"AcDbDimension"`。
- **偏离/决策（16 条，逐条写入 ADR-0017「实施状态」与 `docs/compatibility.md` 的 DIMENSION 子表）**：
  ① ArcLength/LargeRadial **不迁**、导出仍有损，P5 不声称完整往返；② boxed（`-D warnings` clippy 门禁）；
  ③ kind 只在 `definition`；④ 组 11/53 零哨兵在变换下保留（"原点"与"未设置"合并，声明）；⑤ 导出
  `DIMSCALE=1`（**不保留**）；⑥ 总 `add_or_replace` DIMSTYLE；⑦ picture 分支坏成员 ⇒ 整实体 `Opaque`+`Partial`
  （**比 P5a 前更严**）；⑧ picture 分支 measurement/文字**冻结**（残差：均匀缩放后画面数字不匹配被缩放几何）；
  ⑨ ordinate 改**基准相对**（基准非原点者渲染**正确性修复**；导入侧**从不做 datum 重基**，旧值是绝对值；
  取**投影绝对值**——带符号 ordinate 坐标**未建模**，代价已声明）；⑩ 导入摊平加**块栈**护栏（此前无此递归，故无护栏）；
  ⑪ 样式只带**合成消费子集**（其余 (a)+(b) 是 P5b；DIMTAD 2/3/4 导入塌缩为 1——**此句已被 P5b 作废**：
  P5b 真正实现了 DIMTAD 2/3/4，见下方 P5b 条目与 ADR P5b 偏离 13）；⑫ 组 70 的 32 位不写、
  0x80 从不置位（导入曾置位者丢失，**声明丢弃**）；⑬ 组 210 不写（−Z 角度标注仍按源法向渲染，但导出丢 210）；
  ⑭ `DIMLFAC ≠ 1` 测试**已落地**；只有 `DIMLUNIT≠2` 诚实测试**结构性推迟 P5b**（`DimensionStyleValues`
  根本没有该字段）；⑮ **A.3 修订项**：`DimensionDefinition` 新增 `normal` 字段（A.3 点表原无此项，
  恢复 −Z 挤出角度标注的旧绕向必需）；⑯ N3 自证测试已改为**非平凡**断言（导入显示 == 用其自身定义 +
  导入的样式值做合成的结果），"逐字节相同"的措辞已改为诚实表述（数值不变的证据是保留的合成数值测试）。
- **N9 矛盾已核**：`docs/compatibility.md` 旧称"角度/坐标/圆弧长/大半径仍 Partial"，而代码对
  `Angular2Ln`/`Angular3Pt`/`Ordinate` 报 `Completeness::Complete`（`dimension/convert.rs`；ADR §N9 亦记录
  代码对这三类报 `Complete`）。P5a 已让表与代码一致——**取代码的 `Complete`**（这三类在域变体里有完整点槽与
  共享合成），表已改；`LargeRadial`/`is_partial` 弧标注仍 `Partial`。
- **未用第三方读取器验证**：内容块必要性依据**第三方文档**（ezdxf，ADR §0.6），本机无
  `ezdxf`/`dwgread`/`dxfgrabber` ⇒ 往返只是**内部一致性**，**不声称 AutoCAD 兼容**。**NOT RUN**。
- **门禁（编排者复跑，2026-10-19）**：受影响 crate **1778 通过 / 0 失败**（含 `cad-import-acadrust` 的往返、
  组码、内容块、picture 逐成员、递归护栏、N9 有损成员等新测试）；`cargo check --workspace --all-targets --locked`、
  wasm lib、clippy(workspace `-D warnings`)、fmt、architecture（27 包）、i18n（**489** 键，含本轮新增
  `properties.row.subtype/measurement/display`）、fixture（22）、file-size（**665**）、workflow、UI **126/126** + build。
  **NOT RUN / 未证**：全量 `cargo test --workspace`（`accelerated_tier` 超时）；**真机证据**；跨平台；
  **第三方读取器验证**（本机无 ezdxf/dwgread/dxfgrabber）。
- **真机证据（2026-10-19，回归性）**：真实硬件 GPU（Vulkan / **Quadro P620** / `device_type=discrete_gpu`）+ X11 打开
  `fixtures/dxf/qcad-flange/flange.dxf`（**含 6 条无匿名块 DIMENSION**，SHA-256 与 `fixtures/manifest` 核对一致）：
  `entityCount 419`、`loading=false`、`importCount=1`、**352 次采样 `error` 全为 null**、持续出帧（末帧 5430）、
  画面中央有结构化亮线（墨迹 4.16%，`x=[107,636] y=[0,333]`）。运行目录 `/tmp/opencode/yacr-p5a-evidence`
  （verdict `PLANNED_TIMEOUT` / exit 143 属预期）。**诚实边界**：① `cad_status` **无几何通道** ⇒ 本帧只证
  "文件已打开 + 计数 + 无错误 + 内容存在"，**迁移后标注的显示正确性由契约测试钉住，不由本帧证明**；
  ② **没有 P5a 前的基线截图** ⇒ 这是"**仍能渲染**"检查，**不是**像素级"未变"证明；③ 截图仅含应用窗口自身
  （X11 自窗口抓取），不含视觉正确性/精度/vendor/性能结论。脚本参数为 `--dwg`（非 `--open`）。
- **observer 独立判读同一截图**（像素分类 + 连通域/游程测量，**非**目测）：**"标注已渲染"成立**——≥5–6 条
  **蓝色尺寸线**（如 `y≈148 x 327→417`、`y≈170 x 308→435`）配**挑出的蓝色延长线**（如 `x 327–328 y 140–152`）、
  端部**实心加宽块**（长约 4–6 px、约线宽 1.5–2 倍），尺寸线中点上方有**蓝色测量字形**（约 `x 363–374 y 137–143`）；
  蓝色标注像素 1179。**法兰几何亦成立**（外圆 r≈43.5、内孔 r≈30、双耳外形约 149×90 px、8 个螺栓孔在 r≈37
  分度圆上）。**两条"未证实"如实记录**：① **箭头形状无法确认**——能看到端部**加宽**，但在该分辨率下**分辨不出
  三角锥形**（不猜）；② **测量文字不可读**——字形存在但**读不出数字**（不猜数值）。另有蓝色斜引线
  （≈`(345,92)→(398,80)`）。窗口 chrome 无错误/警告/模态。
- **待办**：P5b（DIMSTYLE 存储 + B.2(a)/(b) 子集 + 默认 + i18n + `DIMLUNIT≠2` 诚实测试）；P5c–P5e（创建命令）；
  P5f 已降级 NOT IMPLEMENTED。

## P4：编辑命令扩展（ADR-0016，2026-10-19）

**范围**：`crates/cad-app`（命令/工具/几何构造/宿主映射）、`crates/cad-geometry`（两处辅助）、
`crates/cad-ui-core`（选择子、**类型化参数通道**、状态投影、i18n）、React 宿主 UI。**未**改 acadrust/wgpu/架构边界。

- **设计先行**：ADR-0016 经 Oracle 设计门禁 **SG-P4 attempt 1 = APPROVE-WITH-FIXES / attempt 2 = ACCEPT-WITH-FIXES**
  → 修正后转 **Accepted**。7 项阻塞全是**事实/设计**纠正，其中 3 条直接推翻了我的前提（旋转构造器**已存在**、
  `Image`/`Mask` **可变换**、MINSERT **已建模为 `Compound(Insert…)`**），另 4 条是设计缺口（`ToolParameter`
  装不下布尔/多参数、ALIGN 的"成对确认"现有 arity 表达不了、STRETCH 自相矛盾、hatch 分解只取环会**静默丢失**）。
  复核阶段又抓出 T1（Dimension 实为 `Compound`/`AcDbDimension`，按新规则**会被**分解 ⇒ 改为按 `type_key` 显式拒绝）
  与 T2（STRETCH 的"完全包围"口径对无顶点几何未定义 ⇒ 按几何类型分两种口径）。
- **P4a（提交 c739558）**：类型化参数通道取代 P3 的"单用途"结论——`ToolValue::{Number,Flag}` + 12 个参数成员 +
  单一请求 `SetToolParameter`；**`SetPolygonSides` 及其 IPC/队列/注册全部删除**（每参数仅一个线上入口）；
  各参数自带范围与显式拒绝（**不 clamp**）；`params.sides` 线上字段冻结；React 侧泛化为"活动工具参数组"，
  并有测试断言旧 IPC 名在 UI 源码中彻底消失。
- **P4b（提交 f82c348）**：`CHAMFER`/`FILLET`（两条独立直线）。几何前置：`bulge_arc_geometry` 升 `pub`（单一实现）、
  新增 `infinite_line_intersection`（`Point`/`Parallel`/`Collinear` 三态）。CHAMFER 两距离**按拾取顺序**、
  `0/0` 与 `FILLET R=0` 有**等价性对照测试**；FILLET 相切弧**圆心构造标注 UNVERIFIED（几何推论）**；
  新实体在**活动图层**（我们的选择）；拒绝面含平行/共线、同实体两次、跨文档、非直线段、点在角点、距离过长、缺层、Viewer。
  **两处实现偏离已记录**：`DrawPreview` 未扩展（`cad-host` 冻结 ⇒ 由 `cad-ui-core` 读会话存储投影，线上结果相同）；
  载荷 boxed（避免 clippy `large_enum_variant` 触发对冻结 `cad-engine` 的告警）。
- **真实 GPU 证据（Vulkan / Quadro P620 / X11；`YACR_PROBE_DRAW_EVIDENCE`，输入为合成 DOM 指针事件、非真实鼠标）**：
  用**绘图命令现场造**两对相交直线（各 `+2`），经**通用参数通道**设半径/距离，FILLET 中一次**故意点空被当场拒绝**
  且不推进，两次提交各 `entityCount +1` 且无错误；POLYGON 阶段用**新通道**重跑成功（越界拒绝消息含参数名）。
  **observer 独立判读**同一截图：FILLET 处为**真圆弧**（最小二乘 r≈22.1 px ≈ 0.111 世界单位，与设定 0.12
  **同量级**（截图拟合，**非精度证据**）；残差 21.8–22.5 px 排除折线），与两线**相切**、两线止于切点；CHAMFER 处两线分别修剪 8.5/9.5 px（≈0.043/0.048，
  与 0.05 相符）并由 12.4 px 的 45° 段连接；窗口内无错误/告警样式。
  **诚实边界**：`cad_status` 无几何/顶点通道，故真机只证"提交成功 + 计数 +1 + 像素存在"，
  弧的半径/相切与修剪距离由 crate 契约测试数值钉住（不在真机证据内）。
- **P4c（提交 bad87c2）**：`ARRAYRECT`/`ARRAYPOLAR`（**独立副本**、一条事务一步撤销、源对象保留；矩形间距=**基点间距离**、
  向上右生成；极坐标捕获中心点、`rotateItems` 决定绕中心旋转或仅平移）。**首版两条规则有误并已修**：
  ① 填充角规则与自己的默认值矛盾（默认 360 却拒绝整圈 ⇒ 用户步进后**回不到默认**）→ 改为**有限、非零、`|angle| ≤ 360`**，
  并同步 UI 步进边界；② 副本数多一个（`rows×cols`/`items` 是**含原对象的总数**）→ 改为 `rows×cols−1` / `items−1`，
  1×1 与单项阵列**不产生副本并显式诊断**、不开事务不留撤销。所有受影响期望（实体数/坐标/DXF/一条撤销）已同步。
- **P4d（提交 b43e1e3）**：`ALIGN`（两对点 `Exact(4)`、`AlignScale` 默认关、以 `source1` 为枢轴
  `translate∘rotate∘[scale]`、**原位**变换、一条事务一步撤销）；数值测试钉住 90° 旋转与 3 倍均匀缩放、
  矩形保持 2:1 边长比（证明均匀）。**1 对点移动显式未实现**（等价于 `MOVE`）。
- **P4e（提交 95bb7ff）**：`STRETCH`（4 点、窗口**恒 CROSSING**、分类在命令内重算且按几何类型分两种口径、
  **部分包围=逐顶点移动 / 完全包围=整移**、圆/椭圆/块/ARC 部分包围不动+诊断、`TEXT` **整命令拒绝**、
  **bulge 数值保留**并数值钉住 0.5、无变化时**不开事务**只给诊断）；载荷 `delta` 为**位移点**（平移 = `delta − base`）。
- **P4f（提交 a492186）**：`EXPLODE`（**一次一层**；与 `ERASE` 一样是**无载荷直命令**，不需要工具状态，故只有
  `draw.kind.explode` 而无 `tool.draw.explode`）。**首版与决策文本矛盾并已修**：lane 照搬导出层的递归走查
  （把嵌套块**摊平到叶子**）与"一次一层"冲突 ⇒ 改为成员按一层产出、**嵌套块引用保持为引用**（placement 复合进去）、
  二次 EXPLODE 才展开；导出层的循环栈/深度上限**整段删除**（一层走查下不可达，保留即等于假装加固）。
  `Compound` 分解为**全部子项**（含 hatch 填充网格与图案线，只取环是静默丢失）；`Dimension`/`MTable`/`MLeader`
  按 `type_key` 显式拒绝；`IMAGE`/`MASK` 按**策略**拒绝（它们**可变换**，不得假称被变换拒绝）。
- **真实 GPU 证据（Vulkan / Quadro P620 / X11；`YACR_PROBE_DRAW_EVIDENCE`，44 次合成 DOM 指针事件、非真实鼠标）**：
  `ARRAYRECT`（2×3）`entityCount` **+5**、`ARRAYPOLAR`（4 项）**+3**、`EXPLODE`（闭合矩形→四段线）**+3**；
  `ALIGN` 与 `STRETCH` 为**原位**变换故计数**不变**且无错误。**observer 独立判读**同一截图：
  矩形阵列为 2 行×3 列**相邻副本**（行距 49.5 px ≈ 0.25、列距 59.5 px ≈ 0.30，与设定一致）；
  极坐标阵列为 **4 个副本按 90° 等角、半径 ≈60 px（0.30）**，**但副本自身未旋转**——这正是 `arrayRotateItems`
  默认 **false**（仅平移）的路径，**旋转路径只有契约测试覆盖**；圆角处为**真圆弧**（拟合 r ≈ 17 px ≈ 0.085–0.09，
  与设定 0.12 **同量级**（截图拟合，**非精度证据**））、切角处为 **45° 直切**（≈9 px ≈ 0.045）。
  **三条诚实的边界**：① `EXPLODE` 的"已分解"**在像素上不可分辨**（四段相邻线段与闭合折线视觉相同），
  故分解仅由**计数 +3** 证明，不由像素证明；② 上述像素判据为**临时判读、输出未落盘**（见开放项），
  故它只是"内容存在"的旁证，**不是可复核工件**；③ `cad_status` 无几何/顶点通道，故真机只证"提交成功 + 计数 +
  内容存在"，各命令的**几何正确性由 crate 契约测试数值钉住**（不在真机证据内）。
- **门禁**：cad-app 578 / cad-ui-core 140 / cad-representation 188 / cad-host 143 / cad-geometry 79 / cad-spatial 47 /
  cad-engine 104 / cad-measure 52；clippy(workspace)、fmt、check(workspace+all-targets)、wasm、architecture、
  i18n 486、fixture 22、file-size 660、workflow、UI 126。
- **NOT RUN / 未证**：全量 `cargo test --workspace`（`accelerated_tier` 超 10 分钟被杀）；真机**几何精度**
  （无观测通道）；跨平台。
- **待办**：P4 阶段门禁（Oracle）；P5（DIM 族）及以后见排期。
- **开放项（本轮新增）**：UI 的 `draw.fillet/chamfer/arrayrect/arraypolar/align` 工具行是**前瞻性占位**
  （`tool.key` 若与假设不符需改行）；`statusKey` 假设等于机器键（仅 `polygonSides→sides` 是历史例外）；
  填充角 `projectable` 放宽以显示默认 360；Flag 参数无待发累积；证据工具链的 README 模板仍会列出未产出的
  wayland 工件、像素判据输出未落盘。

## P3：六个新绘图命令（ADR-0015，2026-10-19）

**范围**：`crates/cad-app`（工具状态机/几何构造/命令/宿主映射）、`crates/cad-ui-core`（选择子、状态投影、
i18n）、`crates/cad-geometry`（SPLINE 插值纯函数）、`crates/cad-representation` + `cad-app` 渲染层
（POINT 标记）、`apps/app-linux`（专用 IPC、证据驱动）、React 宿主 UI。**未**改 acadrust/wgpu/架构边界。

- **设计先行**：ADR-0015 经 Oracle 设计门禁 **SG-P3 attempt 1 = APPROVE-WITH-FIXES / attempt 2 = ACCEPT**。
  四项阻塞项都源于真实源码事实，值得留档：① `cad-db` 要求 `Spline.control_points ≥ degree+1` ⇒ 三次插值
  **2–3 个 fit 点根本存不进去**（改为显式拒绝 <4）；② §3.2 要求的**能力表**此前既缺更新、又含一条**假声明**
  （POINT 绘制"实现"，实际退化为零长线段）；③ POLYGON 参数缺范围/生命周期，且 `CadTool` 是**冻结的前端线格式**；
  ④ POINT 上限语义含糊、曾误用 `SELECTIONPREVIEWLIMIT` 为 2 万量级背书、且 **3D 下 `viewport_bounds` 为 `None`**。
- **交互形状对齐 AutoCAD 文档**（librarian 查证）：`ELLIPSE` 默认是**轴端点法**（不是中心法）、`POLYGON` 默认
  **内接**且边数默认 4、`SPLINE` 默认 fit/degree 3/chord、`POINT` **是否重复放置 Autodesk 未记载**（故只做
  单次放置，不猜）。
- **六个命令**：`PLINE`（变长、Enter 结束、开放折线）、`RECTANG`（两对角点 → 规范化逆时针闭合折线，两种对角
  点击**同一实体**）、`ELLIPSE`（轴端点法；**长轴存较长者**以保证 DXF 组 40 ≤ 1——**未查证假设**，能力表记为
  未对齐；预览 16–128 段有界弦近似）、`POLYGON`（内接；第一顶点在半径射线上）、`POINT`（单次放置）、
  `SPLINE`（≥4 fit 点 → 三次/clamped/chord 插值 NURBS）。全部经唯一插入漏斗 `commit_new_entity`
  （一次事务 + 一步撤销），且导出经**生产 save 路径**断言。
- **变长捕获机制**：`PointArity::{Exact,AtLeast}`、`remaining() -> Option<usize>`（开放工具**不谎报** 0）、
  上限 1024 显式拒绝、Enter 复用既有 `CONFIRM`（补测了"不自动确认"）。
- **POINT 渲染（本轮修掉的真缺口）**：基础场景加**显式 `Point =>` 跳过**（**不碰共享 `tessellate`**，因为拾取/
  包围盒/几何引擎仍依赖 `[p,p]`），可视标记走 overlay（draw_order 850 000，在网格之上、选中与实时反馈之下），
  尺寸 = 视口高的 5%（`PDSIZE=0` 式屏幕相关语义），**覆盖导入的 POINT**，上限 4000 且**只发一条诊断**，
  **3D 显式不画**；相机变化只重算标记不重建底图。上限规模（4000 点）构建为**单批次、每标记恰 4 顶点、单次
  预分配顶点向量**；契约测试 `point_marker_overlay_cap_build_is_structurally_bounded` 断言该结构界与一条
  **宽裕的墙钟 sanity 上限**（250 ms，仅用于捕捉量级退化，**非**性能目标）。**≈71 µs 为估算值，非实测**
  （无 bench/工件支撑）。**块内 POINT 不标记**（限制已记录在代码与测试中，且此前同样不可见）；
  `PDMODE`/`PDSIZE`/点样式声明未实现（有意偏离）。
- **参数通道（POLYGON 边数）**：会话参数（默认 4、范围 3–1024、越界**显式拒绝不 clamp**、切工具/取消保持、
  文档编辑不重置、重建/新建回 4、Viewer 拒绝）+ 收窄的 `InteractionRequest::SetPolygonSides` +
  `CadTool.params.sides` 线格式契约测试；因 UI 只有"命令名"通道，另加**专用 IPC**
  `cad_set_polygon_sides(sides)`（同一有界主线程队列、ack 语义与 `cad_command` 一致），以及状态栏**步进控件**
  （仅 POLYGON 激活时出现、值只读真实状态、**不做乐观更新**、有界待发累积上限 10 使快速连点正确合成）。
- **CJK 基线诚实测试**：`cad_app_adds_no_new_cjk_literals_outside_the_recorded_baseline`（记录 76 个既有
  字面量，新增即失败并提示必须**显式**改基线；同时检测基线陈旧）。
- **真实 GPU 证据（Vulkan / Quadro P620 / X11，`YACR_PROBE_DRAW_EVIDENCE`；输入为合成 DOM 指针事件、
  **非真实鼠标**）**：`PLINE` 三点+Enter → `entityCount 4→5`；`POLYGON` 经真实 IPC 设 6 → 状态可见
  `params.sides=6`、提交后 `+1`，**越界 2 被宿主真实拒绝**（`POLYGON side count must be between 3 and 1024, got 2`）
  且可见值不变；`POINT` → `+1` 且截图含 **126 个标记像素、单个连通块**，其中心与**相机换算的点击分数位置**
  吻合（≤1 px；世界点本身无观测通道）；`RECTANG` → `+1`。
  像素判据脚本 `apps/app-linux/scripts/check-point-pixels.py`（容差 0.03，脚本内说明了为何比带判据的 0.06 更严：
  0.06 会被 CAD 网格灰主导）。**诚实限制**：`cad_status` **无顶点数通道**，故**不**声称多边形实体是 6 边。
- **门禁**：cad-app 481 / cad-representation 187 / cad-host 143 / cad-ui-core 128 / cad-engine 104 /
  cad-geometry 77 / cad-measure 52；clippy(workspace)、fmt、check(workspace+all-targets)、wasm、architecture、
  i18n 450、fixture、file-size 612、workflow、UI 114。
- **NOT RUN / 未证**：全量 `cargo test --workspace`（`accelerated_tier` 超 10 分钟被杀）；POLYGON 实体的**顶点数**
  （无观测通道）；`SPLINE` 的真实 GPU 端到端（本轮证据覆盖 PLINE/POLYGON/POINT/RECTANG）；跨平台。
- **开放项**：PLINE 弧段/Close/Width/Undo；RECTANG 圆角/倒角等选项；ELLIPSE 中心法/椭圆弧/`PELLIPSE`；
  POLYGON `Edge`/外切/边数跨会话持久化；POINT `PDMODE`/`PDSIZE`/重复放置/3D 标记/块内标记；SPLINE CV 法/节点/阶数/
  切向/公差/`Close`/2–3 点小样本/fit 点往返；六者的 3D/OCS 与纸空间造形；**证据工具链**：`collect-evidence.sh`
  的 README 模板会列出该次运行**未产出**的 wayland 工件（x11-only 运行也会列），且像素判据（
  `check-band-pixels.py`/`check-point-pixels.py`）的输出**未落盘**到证据目录——两者都应修，属工具链而非产品；
  **文件行数**：`lib.rs`/`tests_commands.rs`/`host/mod.rs`/`draw_tool/mod.rs` 已按门禁拆分（本轮），
  `crates/cad-spatial/src/pick/tests.rs`（991）列入下一阶段待拆清单。

## P2：编辑工作流接线 + TRIM/EXTEND 取点 + chrome 死入口（2026-10-19）

**范围**：`crates/cad-app`（取点即解算实体、`DrawPreview.picks/candidate`、选择/`importCount` 一致性）、
`crates/cad-host`（捕获候选着色）、`crates/cad-ui-core`（测量类型选择子、`PAN`→`BeginPan`、`ZOOMIN/OUT`、
`ORBIT` 明确禁用）、`apps/app-linux/ui`（ERASE 入口、测量按钮、`ORBIT` 说明）、`apps/app-linux/src/evidence`
（`YACR_PROBE_EDIT_EVIDENCE`）、docs。**未**改 acadrust/wgpu/Cargo.lock/架构边界。

- **TRIM/EXTEND 改为取点即解算实体**（此前是两次"盲点"，实体只在 `CONFIRM` 时解算）：每次点击都走与
  单击选择/悬停**同一条**拾取路径；**点空处当场拒绝**（沿用 `draw pick did not hit supported geometry`，
  状态不变、可立即重点）；命中则**原子**存入 `SelectionRef` + 原始点；`DrawIntent::Trim/Extend` 带捕获的
  refs，`confirm` **不再二次拾取**（避免"看到的"与"提交的"不一致）；就绪判定 = 两步都解析到实体。
- **命令内对象高亮**：捕获步骤里**已拾取的实体与光标下候选一起**走**同一 `FLAG_HOVERED` 通道**着色
  （`OverlayInputs::hovered`，按 `resolve_pick_transform`——即 flags 生产者所用的同一谓词——过滤，非解析 ref
  只不着色、不伪造；空闲 hover 在捕获期本就抑制，故只有一个生产者）；候选/pick 变化是 **flags-only** 发布
  （`input_revision` 不变，有测试钉住）；切空间/切模式/**文档内容变化**（`scene_identity` 纳入捕获上下文比较，
  因此捕获期间从命令行派发 ERASE/UNDO 也会清候选与 hover，不会再冒出 `highlight.unresolved`）/换文档/取消/
  确认/指针离开都会清除，不会留下过期着色（副作用：文档变化也会清掉**空闲** hover——这是有意的放宽，下一次
  移动即重新解算）。
- **chrome 死入口**（审计后修）：`DIST`→Distance、`AREA`→Area、`MEASUREPOLYLINE`→PolylineLength、
  `MEASUREANGLE`→Angle（**刻意不占用** `PLINE`/`ANGLE`：AutoCAD 的 `PLINE` 是多段线**绘制**命令，属 P3；
  `ANGLE` 不是 AutoCAD 命令）；`PAN` 改走 `InteractionRequest::BeginPan`（不再因缺 `Points` 载荷报错）；
  `ZOOMIN/ZOOMOUT` 变为围绕视口中心 ±25% 的真实缩放；`ORBIT` **不做点击特效**（轨道是 3D 拖拽手势），
  改为可见但禁用并给出"切到 3D 后拖拽"的理由。新增 ERASE 入口（无载荷、作用于当前选择）。
  仍保留的显式不可用项：`GRID`/`SNAP`/`ORTHO`（未实现的制图辅助；`SNAP` 是栅格捕捉 F9，故意不占名）、
  `LAYER`/`PROPERTIES`（面板，后续阶段）。
- **词汇表**：新增 `MEASUREMENT_SELECTORS` / `ZOOM_STEP_SELECTORS` 并入 `admissible_names()`，
  `COMMAND_TABLE` 仍是 `CommandId` 的唯一映射（每个 `CommandId` 恰好一个 selector）；`PLINE`/`ANGLE`/`SNAP`
  有"当前不可用"回归测试，防止以后被误接。
- **证据（真实 GPU：Vulkan / Quadro P620 / X11，`YACR_PROBE_EDIT_EVIDENCE`）**：
  - TRIM 交互链**已在真机运行中证明**（输入由**合成 DOM 指针事件**驱动，**非真实鼠标**、不经 evdev/XTest；
    该限定词同样适用于本轮全部证据驱动）：`startack=true`、`trim_started_tool=draw.trim`、漏点后 `miss_advanced=false`
    且 `miss_error=InvalidInput("draw_pick_did_not_hit_supported_geometry")`、目标点击 `target_points=1`、
    边界点击 `boundary_points=2` + `boundary_ready=true`、`CONFIRM` 到达命令层并以
    `InvalidInput("TRIM found no intersection between the target and the boundary")` 拒绝（正确——该夹具
    只有**四条平行竖线**，不存在可裁剪的相交边界）。
  - **ERASE 真实提交**：`entityCount` 4→3（`erase_count_delta=-1`，两次运行复现）。
  - **EXTEND 无法演示**：夹具无"非平行边界"，`extend_demonstrable=false`，未硬凑。
  - **夹具前提更正**：`fixtures/dwg/synthetic-four-lines.dwg` 实际是 **x=0,1,2,3、y=0..1 的四条平行竖线**
    （无水平线、无 y<0 段）。此前按截图误判为有水平线，已纠正。
  - 宿主层（非真机）确定性测试补上了**几何改动**：`trim_confirm_commits_the_real_geometry_change_through_confirm_draw`
    断言目标 LINE 由 `(-200,100)→(200,100)` 变为 `(-200,100)→(0,100)`（|Δ|<1e-9）、恰好一次事务 + 一步撤销；
    平行边界对照测试断言 `Unsupported/InvalidInput` 且几何**逐字节不变**。
- **真机暴露并修复的两处诚实性缺陷**：① ERASE 成功后会话选择仍持有已删除实体（`selection.count=1` +
  `unresolved` + `error="selected entity 4 is not present in the document"`）→ 编辑删除后按"仍能解析的 refs"
  重建选择；② `importCount` 在非导入提交上自增（1→2）→ 只统计**导入**造成的权威发布。
- **NOT RUN / 未证**：桌面端 TRIM 的**实际几何裁剪**与 EXTEND 演示（受夹具所限，见上）；全量
  `cargo test --workspace`（`accelerated_tier` 超 10 分钟被杀）；Windows/macOS/Android/Web 未运行。
- **开放项**：多边界/曲线边界/多目标 TRIM、`Enter`=全部、TRIM 目标扩到 ARC/SPLINE/INSERT、编辑工具的
  AutoCAD 选项（ROTATE 参考角、COPY 多份、MOVE 多份）、ERASE 之外的删除类命令选择一致性、子元素/实例级编辑。

## P1b：框选手势 + 橡皮筋 overlay + OSNAP/F3（2026-10-18/19）

**范围**：`crates/cad-app`（overlay 契约 + `ToggleSnap` + 框选命令已在 P1a）、`crates/cad-host`（手势仲裁 +
metrics 竞态修复）、`crates/cad-ui-core`（`OSNAP` 选择子、`snapEnabled`、协议可选 `shift`）、
`apps/app-linux/ui`（F3、OSNAP 开关）、`apps/app-linux/src/evidence.rs`（框选证据驱动）、
`crates/cad-app/src/lib.rs`（悬停延时默认值）、i18n（未新增键）、docs。**未**改 acadrust/wgpu/Cargo.lock/架构边界。

- **手势仲裁（ADR-0014 决策 B，行为变更）**：**2D 左键拖拽（≥ `DRAG_THRESHOLD_LOGICAL_PX=4`）= 框选**，
  左→右 WINDOW / 右→左 CROSSING；`Shift`=Add、否则 Replace；**中键拖拽=平移**（2D/3D）；3D 左键仍轨道旋转；
  阈值以下仍是单点 pick；捕获（Drawing/Measuring）期间不框选；Secondary 之外的非主键（Back/Forward）**在
  协议层就被拒**（`crates/cad-ui-core/src/session/input.rs`），现状只有 Secondary 拖拽仍平移。
- **橡皮筋**：宿主把两个逻辑角点经既有 `screen_to_world` 投到工作面，作为**瞬态 renderer overlay 输入**
  `OverlayInputs::selection_box: Option<BoxSelectionPreview { segments, crossing }>`（`BOX_SELECT_DRAW_ORDER
  = 1_750_000`，介于 SNAP_HINT 与 PREVIEW 之间；WINDOW 蓝实线 / CROSSING 绿虚线 screen-space 5px、
  总段数 ≤512）。**不是 DOM overlay**（CAD 面是 GTK 最上层子窗口）。只重建 overlay，不进数据库/ChangeSet/历史。
- **提交**：抬起时经正常命令通路口派发 `SelectInBox` + `CommandPayload::BoxSelect { rect_logical, crossing,
  mode }`；失败显式上报，不吞成空成功。
- **OSNAP/F3**：`CommandId::ToggleSnap`、选择子 `OSNAP`（`COMMAND_TABLE` 46→47）、
  `CadStatus.snapEnabled`、状态栏 OSNAP 由禁用占位改为真实开关、**F3** 经正常命令通路派发；只切会话级
  `snap_enabled`，**不写**持久化 viewer config，无 ChangeSet/revision/历史。协议新增**可选** `shift`
  （缺省/null=false；非布尔拒绝，不强制转换），只供指针请求用。
  **刻意不提供 `SNAP` 别名**（AutoCAD `SNAP`=栅格捕捉 F9，与本项目对象捕捉是两件事；曾短暂别名到 `OSNAP`，
  会把"栅格捕捉"按钮接到对象捕捉上，已移除；`SNAP` 现与 `GRID`/`ORTHO` 一样是显式不可用占位）。
- **本轮修掉的真实缺陷（重要）**：`crates/cad-host/src/lib.rs` 的 `set_canvas_metrics` 原先**无条件**
  `clear_gestures()`。真实 GUI 里布局 resize 与按下会同批到达（日志：`resize seq=2 width=926` 紧跟
  `pointer_down seq=3`），刚建立的按下被清掉 → 拖拽退化为零面积手势 → `SelectInBox` **从未派发**。
  现象是"WINDOW 框选什么都选不中"。修法：只在**确有在飞手势**（`dragged`/`navigating`/`box_gesture`）时清理，
  静止按下存活的锚点仍是合法角点；world 空间 hover 仍清理；revision 仍推进、陈旧 revision 请求仍被拒。
- **悬停延时默认值**：`hover_dwell_ms` 由 300 **下调为 20**（产品负责人决定；60Hz 下约 1 帧，只过滤帧内抖动，
  不再抑制刻意快速扫掠）。仍是**显式偏离**（AutoCAD 无悬停计时），`0` = 即时。
- **证据（真实硬件 GPU：Vulkan / Quadro P620 / X11；`scripts/collect-evidence.sh --backend x11 --lifecycle 0`）**：
  驱动 `YACR_PROBE_BOX_EVIDENCE`（合成 DOM 指针事件，**非真实鼠标**，不经 evdev/XTest；由 Rust `webview.eval`
  注入、在 React 侧 `.native-input` 上 `dispatchEvent`，走 UI→IPC→宿主 正常路径）报告：
  `baseline_count=0 wide_window_count=4 window_count=0 crossing_count=1 discriminating=true entity_count=4
  snap_enabled=true box_overlay_channel=none downs=3 moves=3 ups=3`（输出 `/tmp/opencode/yacr-evidence-20261018T161400Z`；
  同轮日志 `input kind` 计数 3/3/3，与驱动自报一致 ⇒ 无污染输入）。
  命令级真实夹具测试 `crates/cad-app/tests/select_box_real_fixture.rs` 锁定同一组期望（全画布 WINDOW=4、
  2%→98% WINDOW=4、25%→60% WINDOW=0、60%→25% CROSSING=1）。
  **视觉（2026-10-19 最终；此前一版更正留有历史）**：`161400Z`（修复后）截图内**没有**橡皮筋像素——原因是
  UI 合并通道只在 rAF/按下/抬起时才 flush，保持期宿主只有 `pointer_down`（报告的**计数**仍真实）。该机制顺带
  暴露并修复了一个**真实输入丢失缺陷**（见下）。修复后 `172001Z`：**机器判据** `green=1826 window-blue=0`；
  **observer 独立判读**为**绿色虚线闭合矩形**，范围 x 25.0–60.1% / y 24.6–60.1%（与 60%→25% 保持中的拖拽一致，
  ≤1px），无任何蓝色/实线元素，拖拽期间无图元被着色（选择在抬起后生效），窗口内无错误横幅/对话框；observer
  独立计数 1814（阈值敏感，与 1826 一致）。观感为"淡绿"是抗锯齿/混合所致（线性 `[0.35,0.85,0.45]`
  → sRGB ≈ (160,237,179)，边缘混黑后偏暗），非语义差异。更早一轮（`153655Z`，分步 paced 驱动）也读到过带，
  两轮一致。
- **NOT RUN / 未证**：WINDOW 的**正选**在真实 GUI 里只用"近乎全幅"矩形证到（夹具几何下 25%→60% 矩形
  本就无"完全包含"图元，0 是正确语义）；橡皮筋**无 probe 可见通道**（`box_overlay_channel=none`，未伪造）；
  全量 `cargo test --workspace` 未完成（`accelerated_tier` 二进制跑逾 10 分钟被杀 → 记 NOT RUN，其余 crate
  均通过）。
- **开放项（尚未实现，勿当已对齐）**：`select.box.count` 的 `count`/`precision` **无 UI 显示通道**（命令侧
  已产出，但只进控制器内部 `last_status`，无生产读取者；`SELECTALL` 亦然，只有 `selection.count` 可见）；
  3D/纸空间框选；`SELECTIONPREVIEWLIMIT`；`PREVIEWFILTER` 锁定图层/xref 排除；Secondary 的右键快捷菜单语义；
  把 `check-band-pixels.py` 接进 `scripts/collect-evidence.sh` 作为常规判据。
- **已解释的观察（不再是疑问）**：更早一轮（`153655Z`）日志里 23 次驱动未派发的 `pointer_move` 与该轮截图里
  真实存在的橡皮筋是**同一原因**——那版驱动把拖拽**分步 paced** 发送（因此每步都触发 flush、带是活的），
  之后驱动被改写为"单步 move + 保持"才出现"保持期内 move 未送达"的问题（该问题的真正机制见下方
  Oracle 收敛条目：rAF 被节流时合并通道永不 flush）；该轮 `crossing_count=2` 仍**不作为结论**采用
  （干净运行取 `crossing_count=1`）。夹具矩形边界会擦到图元，crossing 计数对矩形精度敏感。
- **Oracle P1 gate（ACCEPT-WITH-FIXES）后的收敛，2026-10-19**：
  - blocker（证据表述）已按事实更正：见上"视觉（更正）"；驱动加了保持期周期微移与 `hold_moves` 计数，
    并新增 `apps/app-linux/scripts/check-band-pixels.py`（PPM/PNG 像素判据，绿=带 / 蓝=window；已知带
    1132Z=979、1365Z=1052 像素，无带=0）。
  - **真实输入丢失缺陷（gate 引出，已修）**：`apps/app-linux/ui/src/input.mjs` 的 move 合并通道只在
    `frame === null` 时排 rAF；rAF 被节流时句柄永不重置 ⇒ 后续 move 只覆盖缓冲，**直到按下/抬起才送达**
    （实测保持期发 47 次 move，宿主只收到 3 次）。修复：加**有界兜底 flush**（短 timer），恰好一次、最新优先，
    按下/抬起/离开/失焦仍即时并取消待发。
  - **cad-host 硬化（4 项 + 1）**：捕获/模态平移中途接管框选 ⇒ 立即结束手势且不再派发被拒命令；局部抬起
    不继承已离开接触点的 button/修饰键；CROSSING dash 改为按周长自适应（≥1080p 全屏不再截断成开口矩形，
    始终回到起点、段数 ≤512）；退化/非有限矩形不再静默（走既有 `set_external_status` 通道，附 ADR B.6 引用）；
    直接 API 的陈旧 metrics `Up` 现在与队列路径一致地清理指针生命。
  - **cad-app / ui-core 三项**：工具占用时框选改为显式 `select.box.count count=0;precision=none`（不再静默
    空成功）；CROSSING 逐类精度加回归测试（线段=exact、圆/弧=approximate、bulge 折线/spline=aabb）；
    移除 `SNAP`→`OSNAP` 别名（AutoCAD `SNAP` 是栅格捕捉 F9，会把栅格按钮接到对象捕捉上），UI 侧该 ribbon 项
    经路由器准入自动显式禁用，测试镜像同步。
  - **ADR 验收补齐**：`CadStatus.snapKind`（复用已缓存 hint 路径，无二次扫描；6 个 `snap.kind.*` 双语键；
    状态栏无候选时不渲染）。**仍未实现**：`select.box.count` 的 `count`/`precision` **无 UI 显示通道**
    （命令侧已产出，但只进控制器内部 `last_status`，无生产读取者；`SELECTALL` 亦然，只有 `selection.count`
    可见）——已钉住该事实的测试并记为开放项。
  - **新增项目门禁**：`scripts/check-file-size.py`（代码文件 ≤1000 行；文档不计入）+ 自测 + `core.yml` +
    `check-workflows.py` 契约；两个超限文件已按模块拆分（`apps/app-linux/src/evidence.rs` → `evidence/*`
    模块树；`crates/cad-app/src/host/tests.rs` → `host/tests/{document,snap,status,toggles,tools}.rs`，
    测试逐字未改、计数守恒）。

## 悬停预览开关 + pickbox/捕捉光圈分离 + 悬停停留延时（2026-10-18）

**范围**：`crates/cad-domain`（`TolerancePolicy.pickbox_logical_pixels`）、`crates/cad-app`（会话开关 +
`ToggleHoverPreview` 命令 + 拾取容差）、`crates/cad-host`（悬停抑制/停留状态机/每帧 due 应用）、
`crates/cad-ui-core`（`HOVERPREVIEW` 选择子、`CadStatus.hoverPreview`）、`apps/app-linux/ui`（状态栏开关 +
测试）、i18n 3 键、`docs/interaction.md`。**未**改 acadrust/wgpu/Cargo.lock/架构边界。

- **产品决定（与 AutoCAD 对齐）**：空闲悬停高亮**默认开启**（AutoCAD `SELECTIONPREVIEW` 初值 3=开）。
  提供 `HOVERPREVIEW` 命令与状态栏"悬停预览"开关；关闭时空闲移动**零拾取**、零高亮。
  （落地时曾先按"默认关闭"实现，产品负责人随后决定与 AutoCAD 保持一致 → 已改为默认开启。）
- **光圈分离（对齐 AutoCAD）**：新增 `TolerancePolicy::pickbox_logical_pixels = 3.0`（AutoCAD `PICKBOX`
  初值 3，管**单击与悬停预览**），`interaction_logical_pixels = 12.0` 只服务 snap 候选（`APERTURE`
  初值 10 的对应物，12 的差异如实记录）。**用户可见后果**：单击命中范围由 12 收紧到 3，细线更难命中
  —— 这是 AutoCAD 原样行为，且现在是单一可调数值。
- **悬停停留延时（显式偏离）**：`SessionState.hover_dwell_ms` 默认 **20**（由 300 下调，产品负责人决定；
  60Hz 下约 1 帧，只过滤帧内抖动，不再抑制刻意快速扫掠）；
  `0` = 即时（AutoCAD 语义）。行为：离开光圈/身份变化后重新解算，但**同一候选需稳定满 `hover_dwell_ms`（默认 20ms）才上色**；
  换目标重新计时（快速扫过完全不亮）；离开图元**立即**清除；`clear_hover()`/工具启动/捕获/关闭开关
  同时清除 applied 与 pending。由既有每帧 `HostCore::tick` 的 `apply_due_hover()` 应用，未引入线程/定时器；
  测试用可注入 `Instant` seam（`#[cfg(test)] set_hover_clock_for_test`）确定性驱动。
- **文献依据**：AutoCAD **没有**悬停延时概念（sysvar 集合无任何毫秒级悬停门控；`TOOLTIPDELAY` 不是
  AutoCAD 系统变量；Options 的"seconds"只属于 ribbon/toolbar 工具提示）。因此停留延时是**人机改善**，
  不是 parity，已在 `docs/interaction.md` §3 与代码注释中双处标注。
- **证据**：`cargo test` cad-domain 25 / cad-app 371+42 / cad-host 114 / cad-ui-core 105 /
  cad-engine 103 / cad-geometry 104 / cad-measure 52 全通过；`cargo check --workspace --all-targets`、
  clippy（workspace，`-D warnings`）、wasm `--lib`、architecture/i18n 脚本、UI `npm test` 74/74 全绿。
  新增判别性测试：`pick_and_hover_use_the_pickbox_value_and_snap_keeps_the_wider_aperture`、
  `dwell_defers_the_tint_until_the_deadline_then_tick_applies_it`（在 300ms 覆盖下 299ms 不上色/无修订 churn，300ms 上色；默认值为 20ms）、
  `a_changed_pending_ref_restarts_the_dwell_timer`、`leaving_geometry_clears_the_applied_tint_immediately`、
  `idle_hover_preview_is_off_by_default_so_cursor_moves_run_no_pick`。
- **NOT RUN**：真实 GPU 复测（改完即待用户手测）；全量 `cargo test --workspace`；原生 Wayland；跨平台。
- **后续**：悬停延时/预览的 UI 控件（当前只有开/关，延时值需改代码或后续加选择器）、
  `SELECTIONPREVIEWLIMIT` 式上限、`PREVIEWFILTER` 锁定图层/xref 排除、`COMMANDPREVIEW` 互斥、P1b 框选手势。

## 悬停不再触发重准备：flags-only 发布 + 光圈粘滞 + pick 图层过滤 + snap/求交成本（2026-10-18）

**范围**：`crates/cad-engine`（渲染输入分类与 `flags_revision`）、`crates/cad-app`（`render_scene` flags
解析器、`picking` 可见性过滤、snap 目标缓存）、`crates/cad-host`（hover 光圈粘滞）、
`crates/cad-geometry`（`intersect_local` 回退弦高/段预算）、`docs/adr/0012`、`docs/interaction.md`。
**未**改 acadrust、wgpu、`Cargo.lock`、架构边界、UI 文案。

- **用户报告**：鼠标左键**移动**（非拖拽/非点击）时 CAD 随鼠标刷新。**勘察纠正**：拾取本身很便宜
  （实测 0.03–0.13 ms/次，`PickIndex` 线程本地缓存），病因在**渲染输入分类**：
  `RenderSessionHandle::set_overlays` 把「悬停实体变化」当作内容变化，同时推进 `input_revision` 与
  `content_revision` ⇒ `prepared_identity != input_revision` ⇒ `render()` 退回
  `base_only_navigation_eligible` → `render_base_only` → `suppress_overlays` 截断 overlay 后缀，
  **反而把已存在的 flags-only 快路径（`overlay_identity_matches`）挡掉**，每次悬停变化都重传 overlay。
- **修复 A（引擎，根因）**：`set_overlays` 改三分类——视觉 overlay 指纹变化 → `advance_input()`（不变）；
  仅 `viewport_bounds` → `advance_content()`（不变）；**仅实体标志指纹变化 → 只推进新增的
  `flags_revision`**（`State::flags_publication_revision` / `PreparedScene::flags_revision` /
  `CadRenderRuntime::applied_flags_revision`），两个既有修订都不动；`ready.revision` 与
  `scene_publication_revision` 语义不变（避免 F4 陷阱：未应用的新 revision 会让 `scene_committed=false`
  重新落回 base-only）。发布边界从**当前** overlay 重解析 flags，worker 结果无法回退更新的 hover。
- **修复 B（宿主，AutoCAD 口径）**：悬停**光圈粘滞**——光标未离开 12 逻辑像素（
  `TolerancePolicy::default().interaction_logical_pixels`，与点击同一光圈）且 hover 身份
  （`scene_identity()` + 图层覆盖指纹）未变时不重跑拾取（AutoCAD：高亮持续到指针离开 pickbox）。
- **修复 C（正确性）**：`pick_at_screen`/`pick_ray` 此前**完全没有**图层可见性过滤，hover/单击可命中
  临时隐藏图层上的对象（渲染底图里并不存在）⇒ 产生 `flags.unresolved` 幽灵标志。现新增
  `pick_at_screen_with_visibility`/`pick_ray_with_visibility`/`ref_is_visible`，复用既有
  `LayerOverrideSet::is_entity_visible`；`resolve_hover` 与 `select_at_mode` 走可见性感知入口。
- **修复 D（性能）**：① snap 目标按场景身份缓存（与 `PickIndex` 同一把钥匙），每次移动只解算一次
  （原先 `move_tool_cursor`/`capture_tool_point` 解算一次、`sync_render` 再走一次）；②
  `cad-geometry::intersect_local` 的非圆锥回退路径原本用 `computation_world.max(1e-9)` 作弦高，
  实际把 **圆 × 带 bulge 折线** 细分到 4096 段上限做 ~1600 万次段测试 ⇒ **实测 209.9 ms/次**
  （宿主 pump 阶段 264.5 ms，曾把 `cad-host` 队列的时间预算耗尽）。现改为**尺度相对 + 有界**
  （`FALLBACK_CHORD_FRACTION = 1e-4` 乘自身尺度、`FALLBACK_MAX_SEGMENTS = 256`）⇒ **0.141 ms**。
  解析圆锥曲线路径仍精确；**近似回退路径精度由 ~1e-9 绝对变为 ~1e-4 相对**，已在注释与测试中显式标注。
- **证据（真实硬件 GPU；夹具 `fixtures/dwg/synthetic-four-lines.dwg`，未用 `complex-test.dwg`）**：
  同一 `YACR_PROBE_HOVER=1 YACR_PROFILE_PICK=1 scripts/collect-evidence.sh --backend x11 --no-build`
  （180 次纯 hover，X11 + Vulkan/Quadro P620）前后对比——hover 拾取 **181 → 100**；
  `input_rev` 区间 **3→18 → 3→4**；`uploads_started` **0→16 → 0→5**；`suffix_commits` **0→26 → 0→3**；
  `suppressed=true` **14 → 0**；`complete=base_only_pending_overlays` **14 → 0**。
  前 `/tmp/opencode/yacr-hover-20261018T113344Z/`、后 `/tmp/opencode/yacr-hover-after-20261018T124227Z/`
  （输入为**合成 DOM 事件，非真实鼠标**；单机单夹具，**不**构成 vendor/性能结论）。
- **门禁**：`cargo fmt --check`、clippy（workspace，`-D warnings`）、`cargo check --workspace
  --all-targets`、wasm `--lib` 目标、`check-architecture.py`/`check-i18n.py`/`check-fixture-manifest.py`/
  `check-workflows.py`、UI `npm test` 70/70 全绿；测试 cad-geometry 104、cad-measure 52、cad-engine 103、
  cad-app 411、cad-host 104、cad-ui-core 103 全通过（`cad-host` 队列测试在 **snap 打开**下恢复通过）。
- **既有失败（非本轮引入，已在 HEAD `9839d8f` 的独立 worktree 复现）**：ignored 硬件测试
  `render::contracts::gpu_base::pending_overlay_upload_never_labels_stale_suffix_current`
  在 HEAD 与本树**同样失败**。
- **NOT RUN**：全量 `cargo test --workspace` 未跑；原生 Wayland 后端未跑；硬件 `--ignored` 全套未跑
  （仅单条相关项按需执行）；无 vendor/跨平台/性能结论。
- **后续项**：`SELECTIONPREVIEW` 式用户可见开关与 `SELECTIONPREVIEWLIMIT` 式上限（当前 hover 恒为单
  实体，上限尚不必要，已记录不变量）；`COMMANDPREVIEW` 式命令预览与 hover 互斥（等 TRIM/EXTEND 预览
  命令落地再接线）；snap 候选的空间窄化（仅在缓存后仍实测到热点时再做）。
- **已记录的 hover 残留风险（Oracle Gate H1 attempt 2，均非阻塞）**：① **相机变化**（滚轮缩放/键盘平移/
  fit）不改变 hover 身份，静止光标上的高亮会留到指针移动 >12px，且 12px 的世界尺寸随缩放改变 ⇒ 建议把
  `world_per_px`/`view_revision` 并入身份元组并在相机变化时清 `last_query`；② **空间切换**（模型↔布局）
  不清 hover ⇒ 建议在切换路径清 hover；③ `DrawPreview::kind` **不在**任何指纹里，而预览几何按 kind
  分支（`preview.rs`），kind-only 变化会落到防御性 else → 无效 worker 往返 + 预览几何陈旧 ⇒ 建议把
  `kind` 哈希进 `overlay_fingerprint`（2 行 + 契约测试）；④ `flags_only_publish` 跳过 `prepared_identity`
  的推理（`token == content_revision` 蕴含自提交以来无 `advance_input`）目前是隐式的，加一句注释锁住。

## React 宿主工具接线：命令准入单源 + 交互工具真正可用（2026-10-18）

**范围**：`crates/cad-ui-core`（session 命令路由、捕捉点、`CadStatus.tool`、`admissible_names`）、
`crates/cad-host`（`HostCore::tool_snapshot`）、`apps/app-linux`（`cad_commands` IPC、
`YACR_PROBE_TOOL_EVIDENCE` 证据驱动）、`apps/app-linux/ui`（准入目录、Enter/Esc、状态栏工具 chip）、
i18n `tool.*` 18 键、`docs/react-linux-app.md`、`docs/adr/0014-object-snap-and-box-selection.md`。
**未**改 acadrust、wgpu、`Cargo.lock`、架构边界。

- **问题（勘察纠正）**：React 宿主准入表硬编码 `OPEN`/`FIT`（`ui/src/commands.mjs`），且
  `InteractionRequest::BeginDraw`/`BeginMeasurement`/`Confirm`/`Cancel` **没有任何 IPC 入口**；
  `plan_input` 的单击恒为 `SelectAt`。⇒ 绘制/编辑/测量只在命令层测试里存在，GUI 里点不到。
- **修复**：`CadSession::command` 把 `LINE`/`CIRCLE`/`ARC`/`MOVE`/`COPY`/`ROTATE`/`SCALE`/`MIRROR`/
  `OFFSET`/`TRIM`/`EXTEND` 路由到 `InteractionRequest::BeginDraw(<kind>)`、`MEASURE` 到
  `BeginMeasurement(Distance)`、`CONFIRM`/`CANCEL` 到 `Confirm`/`Cancel`；`plan_input(press, input,
  revision, capturing)` 在捕获中把单击发成 `CapturePoint`（不再 `SelectAt`）；新增 `cad_commands`
  把 `admissible_names()`（`COMMAND_TABLE` + 别名）交给前端，React **不再**手写白名单（宿主未应答前
  目录为空 ⇒ 全部显式 unsupported）。
- **交互语义**：工具激活后 `Enter`=CONFIRM、`Esc`=CANCEL（**仅在无 draw/measure 捕获时**才把
  `CANCEL` 转发给宿主 open-cancel）；`CadStatus.tool = {key, points, ready}` 投影真实工具状态，
  状态栏显示工具名与已取点数。绘制工具**不会**自动提交（`capture_point` 对 `Drawing` 恒返回 `false`），
  提交必须显式 `CONFIRM`；测量工具按自身 `auto_ready` 自动提交。
- **证据（真实硬件 GPU，本机 X11）**：`YACR_PROBE_TOOL_EVIDENCE=1 scripts/collect-evidence.sh
  --backend x11 --no-build --capture-at 22`，夹具 `fixtures/dwg/synthetic-four-lines.dwg`
  （**未使用 `complex-test.dwg`**）：`clicks=2 before_count=4 after_count=5 delta=1
  before_tool=- after_tool=- start_ack=accepted=true confirm_ack=accepted=true`，
  `adapter=Quadro P620 (vulkan)`、`present_mode=Immediate`、`verdict=NATURAL_EXIT exit=0
  screenshot=captured`；证据目录 `/tmp/opencode/yacr-p0-tool2-20261018T105219Z/`
  （输入为**合成 DOM 事件，非真实鼠标**）。
- **门禁**：`cargo fmt --check`、`cargo clippy --workspace --exclude app-android --exclude app-web
  --all-targets --locked -- -D warnings`、`cargo check --workspace --all-targets --locked`、
  wasm `--lib` 目标、`check-architecture.py`（27 包）、`check-i18n.py`（422 键/2 catalogs）、
  `check-fixture-manifest.py`、`check-workflows.py`、UI `npm test`（70/70）、`npm run build` 全绿。
  受影响 crate 测试：`cad-ui-core` 101、`cad-host` 102（1 ignored：真实 DWG 性能基线）。
- **NOT RUN（不得当作已通过）**：全量 `cargo test --workspace` 因 harness 超时**未完成**；
  原生 Wayland 后端本轮**未跑**（只跑 X11）；**无** vendor/精度/性能/跨平台结论。
- **仍然未接线**：drafting 开关（`GRID`/`SNAP`/`ORTHO`/`OSNAP`）、图层/特性面板、
  `ZOOMIN`/`ZOOMOUT` 别名、测量子类型（`DIST`/`PLINE`/`ANGLE`/`AREA`）、`NEW`/`SAVE`/`PLOT`（React 侧
  显式排除）、窗口/交叉框选与对象捕捉获取（设计见 ADR-0014，待实施）。
- **已记录的后续项（Gate P0 评审，均非阻塞）**：① 把 `CommandId → 工具` 路由表下沉到 `cad-app`
  （`SessionState::begin_draw` 已有反向映射），避免在测量子类型落地前出现第二套词汇表；
  ② `tick` 先排空全部命令再排空全部输入，同 tick 内"先点击后 LINE"会被判为取点——P1 拖拽分类前需固化该顺序；
  ③ `submit_interactions` 局部接纳失败时可能留下过期 press（既有行为，现无注释）；
  ④ 为 `CANCEL` 仲裁补一个**有区分度**的会话级测试（需要一个可观察的 `dispatch_command` seam；
  现有测试只能证明决策函数与 idle-safe，**不能**抓到"改回提交后采样"的回归）；
  ⑤ 证据驱动可加中途 `tool.key` 断言，使 `entityCount` 增量严格归因于工具路径。

## 每 chunk rstar 最终落地（调优 + 粗网格之上）（2026-10-18；commit `7031404`）

**范围**：`crates/cad-spatial`（`chunk.rs` 每 chunk `RTree` + 节点尺寸参数 `PickTreeParams`；新
`grid.rs` 共享网格算术；`lib.rs` 导出）、`crates/cad-spatial/Cargo.toml` 与 workspace `Cargo.toml`
（`rstar = { version = "0.13.0", default-features = false }`）、`Cargo.lock`（纯新增 +31/-0）、
`crates/cad-app/src/picking.rs`（1 行）。**未**改宿主协议、**未**改 UI、**未**新增诊断码。是 ADR-0012
§10 **S4 拾取半**的收尾；逐条证据见 `docs/validation-pick-index-2026-10-17.md`。

- **背景**：上一轮（`c4b66c6`）把**每 chunk `rstar`** 实测为**每个单元尺寸都净回归**，落地的是**粗网格 +
  扁平 per-chunk 列表**（见下一条）。**产品负责人随后明确选择 ADR 一致性、接受实测代价**（"全面落地
  rstar"）。
- **落地结构**：每**非空** chunk 一棵 `RTree`（`bulk_load_with_params` 重建式 + 不可变快照），空 chunk
  不分配；查询先按查询区域包络取候选，再走既有精确 slab 谓词。**粗网格保留**（目标 **512** 条目/chunk
  ⇒ **1,419** chunk），故单松弛格放置规则不把跨格条目推进恒扫描的 oversized 列表（**17,903 → 598**）。
  候选数与 `PickReport`、保守超集语义**不变**。
- **节点尺寸调优（关键）**：rstar 库默认 `MIN=3`/`MAX=6` 对这些 chunk 尺寸是差拟合；从 3/6 到 512/1024
  扫描显示更大节点每轴更好，**`(256,1024)`** 作为最佳平衡点落地，把未调优代价**大致减半**（构建
  **691 → 554 ms**、warm 中位 **0.407 → 0.319 ms**、warm 最差 **39.5 → 33.1 ms**、索引 RSS
  **~146 → ~136 MB**）。小 chunk 扁平路径在 512/128 阈值上测量**均被否决**。
- **最终实测权衡（相对扁平列表；候选数不变：中位 842 / 最大 21,762）**：构建 ~520 → **554 ms**、warm
  中位 ~0.309 → **0.319 ms**、warm 最差 ~34.6 → **33.1 ms**（略好）、索引 RSS ~126 → **~136 MB**。⇒
  **构建与中位变差、warm 最差略好**——**这是为 ADR 一致性接受的代价，不是性能改进**。
- **依赖落点**：`rstar` **0.13.0**、`default-features = false`、**仅** `cad-spatial`
  （`cad-app`/`cad-scene`/`cad-measure` 不依赖）。§10.2 式核查（解析后 `Cargo.lock` + 已发布元数据）
  **无失配**（crates.io、`MIT OR Apache-2.0`、`no_std`、默认 feature 为空、MSRV 1.85）。`Cargo.lock` 差为
  **纯新增（+31/-0）**；**诚实注记**：一次普通 `cargo check` 会重解析并顶起约八个无关包，故锁被还原、
  **恰好**插入三个解析器条目（`rstar`/`heapless`/`hash32`），再以 `cargo check --locked` 与锁定 release
  构建验证。
- **残余缺口闭合**：§7 要求「复用/扩展 `GridSpatialIndex` 的网格算术」——共享 helper
  `crates/cad-spatial/src/grid.rs` 现由 `GridSpatialIndex` **与** 拾取 `ChunkGridIndex` **共用**（每个
  调用方保留自己的尺寸输入；新公式经核对与旧内联算术**逐位相同**，无既有尺寸改变）。
- **测试（实测）**：`cad-spatial` **47**、`cad-app` **358**（+4/+11/+17 集成）、`cad-host` **102**，
  全 0 failed；随机差分与边界测试**原样通过**；`check-architecture.py` OK、wasm lib check OK、
  `clippy -D warnings` 干净、`fmt --all --check` 干净、锁定 release 构建 OK。
- **边界（不得软化）**：**单个已提交夹具、纯 CPU 拾取**；**不给** GPU/视觉/vendor/跨平台或通用性能
  结论。RSS 是**构建前后的进程 RSS 增量**（**上界**，依赖分配器，逐次波动 **±~10 MB**），且**含**共享的
  `items: Vec<SelectionRef>`。表中「flat + coarse」行的**简报**数字（504.8/0.290/33.1 ms、115,812 kB）
  来自**上一轮**（`c4b66c6`）、**本轮未在该 SHA 重跑**；实现 lane 在**当前树内**重测 flat+coarse
  （~520/0.309/34.6 ms、~126 MB），编排者**独立复跑** rstar 腿（553.6/0.328/33.05 ms、138,984 kB），
  三组数字**分别来自不同运行、不得混算**。热拾取中位由**稀疏区**拾取主导、最差由**稠密区**拾取的
  **精确阶段**主导，均不单独隔离 broad phase。ADR-0012 §10 S4 的另一半
  （`SceneCache::apply_changes` / `CacheKey`）**仍**未接入生产。

## 粗网格落地、每 chunk rstar 实测否决（2026-10-18；纯 `cad-spatial` 改动）

**范围**：`crates/cad-spatial/src/chunk.rs`（网格尺寸由条目数派生、`TARGET_ENTRIES_PER_CHUNK = 512`；
`SMALL_CHUNK_MAX` 移除；头部注释记录偏离）；**未**改宿主协议、**未**改 UI、**未**改 `Cargo.*`、
**未**新增诊断码。commit `c4b66c6`（`perf(pick)`），是 ADR-0012 §10 **S4 拾取半**的后续；逐条证据见
`docs/validation-pick-index-2026-10-17.md`。

- **背景**：ADR-0012 §7 **字面**要求**每 chunk 的 `rstar`**（`bulk_load` 重建式 + 不可变快照）。
- **做了并实测**：per-chunk `rstar` **已实现**（`rstar` **0.13.0** 仅接入 `cad-spatial`；§10.2
  版本/来源/许可核查按解析后的 `Cargo.lock` 与已发布元数据完成：crates.io、`MIT OR Apache-2.0`、
  `no_std`、默认 feature 为空、MSRV 1.85），在 `fixtures/complex-test.dwg`（**723,587** 展开条目）上以
  release 拾取 harness 对**旧扁平 per-chunk 列表**做**单元尺寸扫描**（约 11 → 65536 条目/chunk）。
- **结论（实测）**：per-chunk R-tree **在每个尺寸上都是净回归**——构建时间、热拾取中位/最差、RSS 全面
  变差，候选数**完全相同**（中位 842、最大 21,762）。**原因**：拾取容差与条目间距同量级，被访问 chunk
  的查询包络覆盖其大部分，R-tree 几无可剪枝却仍付每棵树开销；**单独把网格变粗就有收益**，因为 ADR
  强制的单松弛格放置规则不再把跨格条目推进恒扫描的 oversized 列表。
- **落地结构（commit `c4b66c6`）**：**粗网格**（目标 **512** 条目/chunk ⇒ **1,419** chunk，而非
  65,637）+ **扁平 per-chunk 列表**；`SMALL_CHUNK_MAX` 移除。编排者**独立复测**：构建 **504.8 ms**、
  热拾取中位 **0.290 ms**、最差 **33.1 ms**、索引 RSS 增量 **115,812 kB（~113 MB）**、oversized
  **598**（原 2,066）、候选不变——**每个指标**都优于旧细扁平网格与 rstar 变体。**`rstar` 依赖从未
  提交、也不需要**（最终树 `Cargo.toml`/`Cargo.lock` 未动）。§7 其余要求全部保留：松弛 2× 边界 +
  单格放置、独立 oversized 列表、build-once 不可变快照、`Revision` 键控版本、保守超集语义、
  `YACR_PICK_EXHAUSTIVE` 穷举回退；随机差分测试原样通过。
- **偏离（记录，不隐藏）**：这是**实测驱动**的偏离，本 ADR 自身的门允许（§10 S6「按实测再评估」、
  §8.5 profiling 先行）。**残余缺口**：§7 要求「复用/扩展 `GridSpatialIndex` 的网格算术」，但拾取
  chunk 布局**复制**了该算术而非共享；`GridSpatialIndex`/`SpatialIndex`/`PendingSpatialIndex` 在
  `cad-spatial` 之外**无消费者**（grep 确认），故只改了 `chunk.rs` 的副本，`lib.rs`/`pick.rs`/
  `tests.rs` 未动。抽公共 helper 被**有意**略过，以免改动他人正在调整的尺寸。
- **测试（实测）**：`cad-spatial` **47**、`cad-app` **358**、`cad-host` **102**、
  `check-architecture.py` OK、wasm lib check OK、release build OK。
- **边界（不得软化）**：**单个已提交夹具、纯 CPU 拾取**；**不给** GPU/视觉/vendor/跨平台或通用性能
  结论。RSS 是**构建前后的进程 RSS 增量**（**上界**，依赖分配器，逐次波动 **±~10 MB**），且**含**共享的
  `items: Vec<SelectionRef>`。扫描表中 pre-rstar「细扁平」基线行来自**上一轮** harness 运行，**本轮未
  重测**。热拾取中位由**稀疏区**拾取主导、最差由**稠密区**拾取的**精确阶段**主导，**都不单独隔离**
  broad phase（结构对比仍同口径：同网格、同放置、同候选）。

## 空间索引接入生产 CPU 拾取 + 相机-only 导航保持选择高亮（2026-10-17；两项修复）

本轮两项修复：**A** 把 ADR-0012 规定的空间索引真正接入**生产 CPU 拾取**（`perf(pick)`，commit
`13f7808`）；**B** 修复**相机-only 导航**把已选实体的**高亮**抹掉（`fix(render)`，commit `38e3ae8`）。
逐条证据分别见 `docs/validation-pick-index-2026-10-17.md` 与
`docs/validation-selection-survives-camera-2026-10-17.md`。两项**均未**改宿主协议、**未**改 UI、
**未**新增诊断码。

### A. 生产 CPU 拾取接入空间索引（commit `13f7808`）

**范围**：`crates/cad-spatial`（新 `chunk.rs` 的 `ChunkGridIndex`；`pick.rs` 的 `pick_world_bounds` /
`index_local_bounds` / `bulge_arc_bounds`；`lib.rs` 导出）与 `crates/cad-app/src/picking.rs`
（`PickIndex`、`cached_pick_index`、`pick_ray_exhaustive`、`pick_ray_with_index`、`last_pick_stats`；
`lib.rs` 导出）。**未**改 `Cargo.*`。这是 ADR-0012 §10 **S4 的拾取部分**。

- **症状（实测）**：在 `fixtures/complex-test.dwg`（44,591 实体）上**点选一个实体停顿约一秒**；
  同一函数还在**每次指针移动**上跑（hover）。
- **病根（实测/源码核查）**：生产拾取是**纯 CPU**——`SelectAt` → `cad_app::picking::pick_at_screen`
  把 `model_space()` 的**每个**实体（含**递归 INSERT**）展开成一个列表（`borrowed_pick_items`，
  `picking.rs:141`），再由 `pick_closest_borrowed` **逐项**测试、且**每次调用**新建一次 bounds 缓存；
  ADR-0012 规定的空间索引存在但**在拾取链里没有调用方**（`filter_by_index` 是死代码）。
- **改动**：新增**持久化** `PickIndex`（`picking.rs:311`）——缓存展开条目 + 一个**保守包围盒的两级
  chunk 网格**（`ChunkGridIndex`，`crates/cad-spatial/src/chunk.rs`，ADR-0012 §7），键为
  `DrawingDatabase::scene_identity()`（内嵌 `Revision`）+ document id，**复用既有单一失效真相**
  （ADR-0012 §5），**不**新增第二套失效真相。查询把射线的 X/Y 投影裁剪到网格区域并按**拾取容差**
  展开，复用精确阶段**同一个** slab 谓词（`ray_intersects_expanded_bounds`），故 broad/narrow
  **按构造**一致。无法给出保守盒的家族（opaque、权重非正/NaN 的样条、含无界子节点的 compound）
  **恒为候选**，保留 `skipped` 记账与可选择性。超大图元（跨 chunk 数 > `MAX_CHUNK_SPAN` = **16**）
  进 **overflow 列表**，**不**跨块复制（规范 §8.1）。
- **回退与上报（无静默改变）**：索引不可用或 `YACR_PICK_EXHAUSTIVE` 诊断门打开 ⇒ 穷举扫描；
  **索引路径**打印 `pick_broad_phase mode=indexed items=… candidates=…`，**穷举路径**打印
  `borrowed_pick_items …`，两种模式的机读区分是 `last_pick_stats()`（`picking.rs:65`）的 `indexed` 字段。
- **实测（release harness 驱动**同一**生产 `pick_at_screen`，同一夹具；**非**宿主交互）**：展开条目
  **723,587**；热拾取中位 **~1055 ms → ~0.32 ms**（**842** 个候选）；一次性建索引 **~535 ms**；
  最差热拾取（大半径，**13,491** 候选）**~20 ms**。
- **测试（实测）**：随机差分 `indexed_and_exhaustive_picks_agree_over_random_scenes`（**16 种子 × 30
  射线** = 480 次差分；**源码为准**，commit message 记为「60 rays」），覆盖**全部实体家族**、稠密/稀疏、
  嵌套 INSERT、子元素来源、容差边界、chunk 边界、无命中射线，断言索引路径与穷举路径的 `PickReport`
  **完全相等**；另有 `the_index_examines_fewer_items_on_a_large_scene`（5,000 线场景 <1/10）。
  `cad-spatial` **41**、`cad-app` **358**（+4/+11/+17 集成）、`cad-host` **102**、`cad-engine` **100**、
  `check-architecture.py` OK。
- **它抓到的一个真实几何 bug（记录）**：带凸度的 polyline，其圆弧在 polyline 于 3D 中**非共面**时
  可**远超**弦盒（超过矢高）——凸度弧现由**绕 tessellator 弧心的球**来界定（`bulge_arc_bounds`，
  `pick.rs:1142`）。
- **偏离项（记录，不隐藏）**：ADR-0012 §7 命名的是**每 chunk 的 `rstar`**；本轮 `Cargo.*` 超出范围，
  故每个 chunk 持**扁平 payload 列表**（`rstar` **仍未实现**）。该索引在本夹具上占 **~90–100 MB**
  （**真实**内存足迹，**未**削减）。
- **边界（不得软化）**：after 数字来自 **release harness**，**不是**宿主交互——**没有任何探针钩子能
  hover 或 select**（`YACR_PROBE_POSTLOAD_PAN` 以按下开始，故无 hover；baseline 那次唯一的宿主 hover
  来自一次**不确定**的合成器移动）。harness **纯 CPU、不触碰 GPU** ⇒ **不给** GPU/视觉/vendor 结论；
  单机单会话。ADR-0012 §10 S4 的另一半（`SceneCache::apply_changes` / `CacheKey`）**未**在本轮接入生产。

### B. 相机-only 导航保持选择高亮（commit `38e3ae8`）

**范围**：`crates/cad-render-wgpu`（`renderer_impl.rs` 的 `reset_pick_keys` / 新
`reset_pick_keys_and_drop_request` / `rebuild_pick_table` 空集早退分支；新 fence 套件
`tests/selection_flag_lifecycle.rs`）、`crates/cad-engine/src/render/runtime.rs`
（`applied_entity_flags` + `suppress_overlays` 重推）、`crates/cad-app/src/render_scene/overlay.rs`
（1 个 fence）、`crates/cad-engine/src/render/contracts.rs`（3 个 fence）。**未**改 `commit_upload`。

- **症状（用户手动交互，实测）**：用户**选中一个实体、缩放**，画布上的**高亮消失**（用户确认**未**看
  属性面板，只看了画布高亮）。不变量：**相机-only 操作不是场景变更**——选择必须存活。
- **两条在改动前已确认的机制（实测）**：
  1. `Renderer::reset_pick_keys`（`renderer_impl.rs:4214`）会清掉**保留的实体标志请求**，而
     `rebuild_pick_table`（`:4114`）在提交后 `batches` 与 `images` **皆空**时**调用它并早退**——这在
     相机-only 操作上**可达**（2D 导航回退截断一个**零批 base**），于是选择的身份请求被销毁且**无人**
     重推。请求现在**在瞬时空驻留集上存活**，只在 key 空间**真正重启**处丢弃（新设备世代、
     `clear_batches`、`rebuild_device`、设备丢失），经 `reset_pick_keys_and_drop_request`（`:4234`）。
  2. `CadRenderRuntime::suppress_overlays`（`runtime.rs:988`）提交其截断后**没有**像
     `apply_ready_updates` 那样重推标志；它现在重推**最后一次已应用**的实体标志
     （`applied_entity_flags`，`runtime.rs:342`；**记录**，非重新推导；**不是**第二套真相）并记录
     `flags.unresolved` 报告。重推**未变**集合**不写**标志行（渲染器会 diff），故正常回退进入**不花**
     GPU 上传。
- **已调查但未改代码（记录）**：`unmatched` 状态**已经**经 `RenderDiagnostics.result_status`
  （`cad-engine/src/render/mod.rs:468`；探针 `[probe] pan-wait … status={:?}`，`host.rs:636`）与用户
  可见的 `cad_status.error` 暴露；子元素高亮**纯**由选择 + 数据库派生，故一次相机 settle **无法**使其
  退化（fence `sub_element_highlight_is_identical_across_a_camera_settle`）。
- **范围划分（诚实，不得模糊）**：本修复覆盖**高亮着色/overlay**，**不是**选择**状态**——滚轮缩放
  **不能**清掉状态（它映射到 `CommandId::Zoom`，`session.rs:782` / `input.rs:229`，只置
  `navigation_dirty`；只有带 replace 载荷的 `CommandId::Select` 才清选择，而 `SelectAt{Replace}` **仅在
  点击**时发出）。
- **测试（实测）**：新 renderer 生命周期套件 `selection_flag_lifecycle`（**3** 测试；其中**两个**
  ——`transient_empty_resident_commit_keeps_the_retained_flag_request`、
  `explicit_clear_drops_the_retained_flag_request`——在修复前代码上**失败**，经**临时回退**验证），以及
  **3** 个引擎契约测试（`#[ignore]`，需真实适配器）含跨**四个**缩放步 + 一次 settle 的**像素着色**比较；
  `cad-render-wgpu --lib` **200**。
- **发现的预存失败（记录为**既有遗留**，非本轮引入）**：两个 `--ignored` 引擎测试
  （`base_only_navigation_renders_latest_camera_and_restores_overlays`、
  `pending_overlay_upload_never_labels_stale_suffix_current`）在**纯净 HEAD** 上**同样失败**
  （用 `git stash` 复现；差异 `left: 3064, right: 3048`）。本 commit **未**修它们。
- **边界（不得软化）**：**未**做人工视觉判读（引擎 fence 只做**着色像素**的机器比较，不构成视觉验收）；
  `--ignored` 硬件 fence 需**显式**在真实适配器上运行；**人工交互判定归用户**；单机单适配器，
  **不给** vendor/视觉/性能结论；wasm/Windows/macOS/Android 未测。

## 后缀提交不再重置 pacing 成本模型：全览拖动卡顿修复（2026-10-17；纯 renderer 改动）

**范围**：`crates/cad-render-wgpu`（`renderer_impl.rs:2284-2303` 的 `commit_upload` 重置条件 +
`reset_pacing` 注释；**1** 个新 GPU fence 套件 `tests/pacing_commit_suffix.rs`；**1** 个既有 fixture
`headless_render::scene_mutation_resets_pacing_knowledge` 改用真实基座变更；实现 commit `0fb2bc1`）。
**未**改宿主协议、**未**改 UI、**未**新增诊断码。这是 `docs/render-optimization-backlog.md` §N1a 的
**收敛性残余项**；逐条证据见 `docs/validation-pacing-suffix-commit-2026-10-17.md`。

- **症状（用户手动交互，实测）**：**全览拖动卡顿**，而**放大看细节反而顺滑**。二者是同一现象的两面——
  全览帧画的是**整幅 draw-order 前缀页**（无剔除），放大后 **P1 块级剔除**把视口外工作消掉，帧就便宜。
- **病根（实测）**：`Renderer::commit_upload` **在每一次提交上**重置自适应页预算控制器——**包括**引擎
  **每次相机 settle** 发出的**后缀专用 overlay 提交**（延迟参考网格刷新，只追加一条 overlay 批：
  `cad-engine/src/render/mod.rs:309-323` 的 `settle_camera_if_due`，超时常量 `mod.rs:165` 的 250 ms）。
  ⇒ 连续交互期间**每次 settle 都把已学 `scale` 打回 `1.0`**，控制器**永不收敛**、渐进页**始终顶点过重**
  （全览帧 `draw_calls≈2720`、`gpu_ms 15.80`、`wait_ms 16.15`、`over_budget=true` 69/75）。
- **改动**：`commit_upload` 只在**基座变更**时重置——`if keep_prefix == 0 { self.reset_pacing(); }`
  （`renderer_impl.rs:2284-2303`）。**判据 `keep_prefix == 0` 正是 N1f pick 表增量路径所用的同一判别式**。
  - **仍重置的集合**：基座变更（`keep_prefix == 0`）、`clear_batches`、**新设备世代**、**设备丢失**、
    **图像重传**（保留——它替换**整幅驻留图像集**、**从不**是增量后缀）。
  - **`reset_pacing` 只清成本模型状态**（`renderer_impl.rs:2776-2779`）：`scale`/`cost_ema_ms`/
    `over_under_streak`/`seeded`；**restart-pending 是另一路信号、对后缀提交被保留**。**未引入
    dirty/invalidation 标志**——启发式成本模型、非正确性状态（**铁律 4**：不新增第二套失效真相）。
- **实测（本机原生 Wayland / Vulkan / Quadro P620 / `scale=1`；同 UID、无 root/sudo、无屏录、无虚拟显示；
  合成 `YACR_PROBE_POSTLOAD_PAN` 手势）**——`budget_scale` 分布 before → after：

  | 运行 | 帧数 | `=1.000` | `<1.0` | `>1.0` | EMA 重置 |
  |---|---:|---:|---:|---:|---|
  | before（用户手动捕获固定副本 `janky-postpickfix-21h58.log`） | 75 | **35（47%，撞顶）** | 40 | 0 | —（`budget_cost_ema_ms=0.00` 出现在 15 帧＝反复重学） |
  | after（探针运行） | 1888 | **6（仅前 6 帧）** | **1758（93.1%）** | **125（6.6%）** | **1**（仅初始） |
  | 独立复跑（编排者） | 1842 | 6 | **1808** | 28 | **2**（初始 + base 变更） |

  **后缀提交逐次保持已学 `scale`**：`0.435 → 0.435`、`0.485 → 0.485`、`0.471 → 0.471`（**实测**）。
  after 的 `>1.0` **合法**——合成 pan 把图拖进空白区，廉价相位里控制器合理爬回 1.0 以上去寻 13 ms 目标；
  两腿均 **`pacing=active` 全帧**。
- **测试（实测）**：`--lib` **200 passed / 3 ignored**；新套件 `--test pacing_commit_suffix` **3 passed**
  （`suffix_only_commit_preserves_pacing_knowledge_while_a_base_change_resets_it` 后缀提交**逐位**保持
  `pacing_scale()`/`pacing_cost_ema_ms()`；`repeated_suffix_commits_do_not_return_the_scale_to_one` **六轮**
  后缀提交从不把 `scale` 打回 `1.0`；`suffix_commit_after_an_over_target_observation_keeps_the_shrunk_budget`
  超目标观测后的后缀提交保持 `budget_scale < 1.0` 且 `budget_vertices` 低于上限）；`--test headless_render`
  **35**、`accelerated_tier` **23**、`pulled_prefix_equivalence` **9**、`pick_id` **32**、`pick_hidden_flags`
  **8**、`render_effects` **45**；`fmt`/`check --all-targets`/`clippy -D warnings`/wasm check clean；release
  构建 OK。
- **既有 fixture 披露**：`headless_render::scene_mutation_resets_pacing_knowledge`（`tests/headless_render.rs:603`）
  原以 `renderer.upload(…)` 作"场景变更"，但 `upload` 的提交是 `keep_prefix = self.batches.len()`——一次
  **后缀追加提交**、**不是**基座变更——故在修正后的规则下**不再**重置。该 fixture 现改为执行**真实基座变更**
  （`prepare_upload_batches` + `commit_upload(staged, 0)`）；其**契约不变**（"一次**基座变更**的场景变更重置
  成本知识"）。**这是本轮唯一被修改的既有测试**。
- **字段澄清（记录，只读发现）**：`cad-stages apply_us` **不是 GPU draw、也不是交互帧渲染成本**——它是
  `apply_ready_updates` 的**墙钟**（`gtk_bridge.rs:926-932` → `render/mod.rs:1137` → `runtime.rs:494`），
  即**有界分块的场景增量 CPU 暂存**（每次 **≤4 slice 或 ≤8 ms**，`runtime.rs:576-612`，由
  `prepare_upload_batches` 主导）；用户同一捕获中 `apply_us` 中位 **~9.9 ms** 是 `complex-test.dwg` 的
  **一次性批量应用**（166k 批在 ~10 ms 块中暂存，`presented=false`、`render_us=1`）。
- **边界（不得软化）**：**before/after 的每帧 GPU 数字不可比**——baseline 窗口**最大化**，探针运行用
  **926×266** 窗口（被 `YACR_PROBE_NO_MAXIMIZE=1` 强制）；且驱动是**合成 DOM 手势、不是真实鼠标**。
  ⇒ **不得**给出"倍数下降"结论，**不得**给出**人工交互判定**（**归用户给出**）。**视觉/截图未判读**
  （只有日志）；相机只做平移、**zoom 腿未跑**；单机/单适配器，**不外推**到其它 vendor/更大图纸/浏览器；
  **wasm 运行时 / Windows/macOS/Android 未测**。

## 提交路径 pick 表增量更新：拖动冻结修复（2026-10-17；纯 renderer 改动）

**范围**：`crates/cad-render-wgpu`（`renderer_impl.rs` 的 `rebuild_pick_table` 增量后缀路径 +
`pick_table_covers_prefix`；`plan.rs` 的 `PickTable::replace_suffix` / 消费式 `build` / 惰性
`source_index` / 增量 `update_collisions` + `truncated_index`；`lib.rs` 两个诊断计数；**1** 个差分测试；
实现 commit `0f5d5c7`）。**未**改宿主协议、**未**改 UI、**未**新增诊断码。这是
`docs/render-optimization-backlog.md` §N1e 之后**提交路径**的残余项；逐条证据见
`docs/validation-pick-table-incremental-2026-10-17.md`。

- **症状（用户手动交互，实测）**：`./target/release/yacr fixtures/complex-test.dwg` 拖动时**画面冻结**
  （拖动中不更新）、松手后**跳到新位置**；缩放相同。日志（`YACR_PROBE_CAD_LOG=1 YACR_PROBE_GPU_TIME=1`）：
  `[probe] cad-stage queued=4 input_us=1047533 tick_us=3`（输入排队 **≈1.05 s**）、
  `[probe] cad-stages apply_us=3368276 … frame_total_us=3805231`（**3.8 s 一帧**）、
  `commit[… pick=1449615 … total=1466543]`（**`pick`=1.45 s**）。**同夹具在近期 perf 轮次之前**
  `pick=1530380` µs ⇒ **预存缺陷，非回归**（近期轮次只消掉了旁边的 `pulled` 1.95 s）。
- **病根（实测）**：`Renderer::rebuild_pick_table` 每次批次/图像变更都**整表重建**；~16 万批的图上一次
  **后缀提交**（只加一条 overlay 后缀批）也要付整幅重建 ⇒ `pick` ≈ 1.45 s。
- **改动**：`rebuild_pick_table`（`renderer_impl.rs:4082`）在 `keep_prefix > 0` **且**无驻留图像 **且**
  `pick_table_covers_prefix(keep_prefix)`（O(1) 覆盖证明，`:4152`）时走**增量后缀路径**——
  `PickTable::replace_suffix`（`plan.rs:3672`）**退役**被丢弃后缀 key、**只追加**新条目
  （`:4105-4113`）。**其余一切**（base 变更、图像重传、设备丢失 / `clear_batches` 复位、前缀未覆盖）
  **回退整表重建**，计数 `pick_table_full_rebuilds`（`lib.rs:1595`，与 `pick_table_incremental_extends`
  `lib.rs:1597` 互补）。**有效性论证**：pick key 在设备世代内**单调分配、永不重用、永不重编号**
  （`pick_key_counter` 只在 `reset_pick_keys` 重启，`renderer_impl.rs:4164`，**恰在批次清空时**发生）
  ⇒ 前缀外条目不变、追加 key 严格更大。配套：`PickTable::build`（`plan.rs:3608`）**消耗**条目（一次克隆
  而非两次；渲染器仍持自己的副本，**不可能**自引用借用）；实体标志身份索引**惰性构建**
  （`source_index: Option<SourceIdentityIndex>` `plan.rs:3582`，仅 `entity_flags_request` 非空时建，
  `apply_entity_flags` 首次查询前经 `ensure_source_index` `plan.rs:3791`）；`detect_truncation_collisions`
  被保留的 `truncated_index`（`plan.rs:3589`）+ 增量 `update_collisions`（`plan.rs:3905`）取代。
- **实测（本机原生 Wayland / Vulkan / Quadro P620 / `scale=1`；同 UID、无 root/sudo、无屏录、无虚拟显示）**：

  | metric | before | after |
  |---|---|---|
  | 后缀提交 `pick` | 1,449,615 / 1,487,525 / 1,454,073 µs（7 次） | **866 / 433 / 541** µs |
  | base-load `pick` | 1,310,440 µs | 1,021,559 µs |
  | `input_us`（排队延迟） | n=18 中位 **1,045,445** µs、p90 1,069,482、max 2,092,380 | n=1185 中位 **29** µs、p90 41、max 116 |
  | `cad-frame presented` | 拖动期间**冻结** | 2051–2054 行，**连续** |

  **独立复跑**（另一次宿主运行，同一夹具）：后缀提交 `pick=866/433/541` µs、`input_us` 中位 29 µs
  （n=1185，p90 41，max 116）、**2054** 行、exit 0。
- **测试（实测）**：`--lib` **200 passed / 3 ignored**（含
  `incremental_pick_table_matches_a_fresh_build_across_commit_sequences`：**128 种子 × 40 步**的
  append / suffix-replace / image re-upload / reset 序列，**每步之后**断言增量 `PickTable` 与一次 fresh
  `build` 完全相等——条目、碰撞、`max_key`、`truncated_index`、`source_index`，外加事务式重复 key 拒绝）；
  `--test pick_id` **32**、`pick_hidden_flags` **8**、`headless_render` **35**、`accelerated_tier` **23**、
  `pulled_prefix_equivalence` **9**、`render_effects` **45**；`fmt`/`check --all-targets`/`clippy -D warnings`/
  wasm check clean；release 构建 OK。
- **三处更正 / 已否决方案（记录）**：① **`append=97` 是微秒，不是 97 个批次**——格式化点
  `apps/app-linux/src/host.rs:2086-2093`（`commit[rebase={} append={} …]` = `rebase_us, append_us, …`），
  一次后缀提交的真实追加量是**一个**批次；② ⇒ 被提出的「当每个追加批次都是 `PICK_KEY_NONE` 时跳过重建」
  路径**没有生产者**（pick key 恒在 `1..=MAX_PICK_KEYS`，`PICK_KEY_NONE`（0）从不进表）；
  ③ **pick 时的 dirty/惰性重建：已否决**（增量修复后急切成本已 ≈0.4–0.9 ms；带保留标志请求时
  `apply_entity_flags` **必须**在提交时运行——`set_entity_flags` 立即把 `EntityFlagReport.unmatched`
  返回给运行时，`cad-engine/src/render/runtime.rs:699`；`pick_collisions`/`pick_table_len`/
  `pick_key_for_batch` 是 `&self` 访问器，无内部可变性**无法**惰性构建；新 dirty 标志 = **第二套失效真相**，
  **铁律 4**）。
- **为什么交互期间提交会重复（记录该发现）**：生产者是**延迟参考网格刷新**——仅相机的范围变化**布防**
  settle（`cad-engine/src/render/mod.rs:703-706`），相机静默满 **250 ms**（`mod.rs:309-323`，常量
  `mod.rs:165`）后 overlay 后缀被重准备（`cad-app/src/render_scene/overlay.rs:452` 的 `grid_overlay`）并以
  **后缀提交**提交（`cad-engine/src/render/runtime.rs:656-663`，`base_changed=false`）⇒ **相机变化每次
  settle 提交一次，不是每帧**。原始运行的 7 次 settle 被 1.5–3.8 s 的提交本身放大（每个被阻塞的 pump 看起
  来都像新的静默期）；修复后 **3 个手势 = 3 次后缀提交**且画面连续。**完全避免 settle 提交**需要**与相机
  无关的网格**（无限 shader 网格）或**大得多的预扩展网格**——两者都是**设计改动**；记录为**刻意「保持现状」
  （leave as is）**并附此理由。
- **仍开放（本轮未修）**：base 变更后**第一次**后缀提交仍付整幅 pulled 重建（`pulled=2,506,646` µs、
  `pulled_reuses=0`）；后续后缀提交复用（`pulled=590/731` µs）；base load 本身 `pulled=2,518,896` +
  `pick=1,021,559` µs（一次性）。**均非** pick 表问题。
- **边界（不得软化）**：**不得从日志得出人工交互判定**——验收用**合成 DOM 手势驱动**
  （`YACR_PROBE_POSTLOAD_PAN`，**明确不是**真实鼠标），日志只证**提交路径便宜**且**帧持续呈现**，**不**证
  人感知的拖动流畅；**人工判定归用户给出**。`debug-default-fonts.dwg` 的相机路径**未在本 fix lane 测量**
  （pan 驱动两次报「concurrent resize during the pan invalidated the report」），只测到其 base-load 提交
  （`pick=4675` µs）与 exit 0。**视觉/截图未判读**（只有日志）。单机/单适配器，**不**给 vendor/视觉/性能结论；
  **wasm 运行时 / Windows/macOS/Android 未测**。

## 每帧 CPU uniform/encode 成本：超集安全的编码跳过与 slot 压缩（2026-10-17；纯 renderer 改动）

**范围**：`crates/cad-render-wgpu`（`plan.rs` 的 `BatchFacts` 紧凑 run class + `EncodeEmitMask`；`renderer_impl.rs`
的 uniform slot 压缩与 encode 跳过；**3** 个新纯测试；实现 commit `1058af2`）。**未**改宿主协议、**未**改 UI、
**未**新增诊断码。这是 `docs/render-optimization-backlog.md` §N1e 的**后续项**（"每帧 uniform/encode 成本"）；
逐条证据见 `docs/validation-uniform-encode-skip-2026-10-17.md`。

- **病根（实测）**：N1e 后 planner 已能接受**整条**有序表（complex-test **166,346** 条）而只发**一次** draw，
  但 **uniform 循环与 encode 循环仍逐条走完全部批**——一张 1-draw 的 settled 页仍耗 `uniform_ms` **6.071**、
  `encode_ms` **5.365**，`cpu_ms` 合计 **22.42**。
- **改动（超集安全，无第二套失效真相）**：
  - `BatchFacts` 增**紧凑每批 run class**（一个缓存驻留字节，`0` = 不拥有 run）+ `legacy_batches` 列表
    （`plan.rs:927-941`）；每帧 `EncodeEmitMask`（`plan.rs:1048-1120`）标记参与 encode 的批：无 run 的批、
    有 run 且该 run 存活本帧剔除的批、加恒活动 legacy quad/mesh 集。掩码是 planner 活动集的**超集**，
    **绝不**丢 draw 或 flush；run 全被剔除的已转换批在 encode 循环里是纯 no-op，在其 ~200 B `GpuBatch`
    被读取**之前**即跳过（`renderer_impl.rs:5220-5235`）。`Default` 掩码恒 `all`（未重建绝不跳过）。
  - **slot 压缩**：只有无 run 的批拿每批 uniform slot（`renderer_impl.rs:4804-4828`）；per-stream/chunk/conic
    slot 只为可见 chunk 写，经运行 slot 计数器压缩（settled 页 `total_slots` **41,322 → 1**）。每个存活条目的
    96 B uniform 由**同一次调用、同一输入**产出 ⇒ 字节**逐字节相同**；上报计数从 class 字节**精确复现**。
  - **刻意不跳过**：透明 pass（转换要求不透明批 ⇒ 每个透明批都是真实发出的工作）与加速计数循环
    （`use_pulled` 要求无剔除）。两个主循环仍迭代 Θ(accepted)/Θ(opaque)——每个被跳过条目只花**一次字节读**，
    因为上报计数定义在整个被接受集上。
- **真实宿主实测（本机原生 Wayland / Vulkan / Quadro P620 / `scale=1`；同 UID、无 root/sudo、无屏录、
  无虚拟显示、exit 0）**，settled 1-draw 页（中位 / p90）：

  | fixture | `plan_ms` | `uniform_ms` | `encode_ms` | `submit_ms` | `cpu_ms` |
  |---|---|---|---|---|---|
  | complex-test | 10.747 / 18.084 → **1.130 / 1.341** | 6.071 / 6.862 → **0.151 / 0.197** | 5.365 / 5.598 → **0.656 / 0.802** | 0.206 → 0.186 | 22.42 / 30.78 → **2.180 / 2.511** |
  | debug-default-fonts | 0.677 / 0.759 → **0.132 / 0.207** | 0.135 → **0.026** | 0.247 → **0.090** | 0.140 → 0.224 | 1.244 / 1.367 → **0.522 / 0.796** |

- **独立 orchestrator 复跑（另一次宿主运行，complex-test）**：settled 帧 **50 → 128**，`plan_ms` **10.499 → 1.131**、
  `uniform_ms` **6.056 → 0.155**、`encode_ms` **5.335 → 0.652**、`submit_ms` 0.207 → 0.182、`cpu_ms` **22.158 → 2.207**；
  两次运行都出现同一病态页签名（`draw_calls=1 verts=12 tris=4 over_budget=false pacing=active cull_chunks=1/9362`）。
- **确定性字段（settled 页）**：`draw_calls`/`verts`/`tris`/`instanced`/`mesh`/`legacy_opaque`/`arc`/`transparent`/
  `images`/`accelerated`/`instance_streams`/`instanced_batches`/`tier`/`over_budget`/`prog=`/`complete`/`pacing=`/
  `cull_chunks=` before/after **逐字段一致**；**唯一移动**的是 `budget_verts`/`budget_tris`（仅因 pacing scale
  依赖墙钟）。
- **测试（实测）**：`cargo test -p cad-render-wgpu --lib` **195 passed / 3 ignored**（**3** 个新纯测试：
  `encode_skip_is_superset_safe_and_preserves_counters` 4000 例超集安全差分、
  `fully_culled_batch_with_legacy_quads_is_still_processed` legacy-quad 回归、
  `skipped_uniform_slots_leave_surviving_bytes_bit_identical` 2000 例 uniform 字节同一）；`--test headless_render`
  **35**、accelerated_tier **23**、pulled_prefix_equivalence **9**、render_effects **45**、pick_id **32**。
- **两处诚实更正**：
  - **settled 页 `submit_ms ~5 ms` 估算错误**：实测 **0.206 ms**（`22.42 = 10.75 + 6.07 + 5.37 + 0.21`），
    提交在该页**从来不是因素**。已更正 `docs/validation-plan-cost-2026-10-17.md`（dated 注）与 backlog §N1e。
  - **paced 帧数字被混淆**：after 运行的 paced 页接受约 **1.6×** 顶点 / **2×** draw（更快的 settled 帧让墙钟
    相关的 pacing 控制器放大 scale）；按单位发出工作计 encode/submit 成本**没有**增加。**只有 settled 1-draw
    页是干净的 before/after**。
- **N2 重估（本轮第二项结论）**：运动（paced）帧现约 `cpu_ms ≈ 3.6` + `wait_ms ≈ 4.2` ≈ **8 ms**（**实测**，
  逐运行被墙钟轨迹混淆，但两相位现均小）⇒ **在 60 fps 预算之内且有裕量**；`gpu_ms ≈ 3.95`（**实测**）约为
  `wait_ms` 的 **93%** ⇒ 主导残余是 **GPU 执行**，任何 CPU 杠杆（含 N2）都不减少它。N2 或可把运动帧从 ≈8 ms
  压到 **≈4–5 ms**（**估算，未测**）⇒ 买延迟裕量而非补预算缺口。⇒ **不以旧理由开工**；N2 **重挂于产品需求**
  （120 Hz，或更重图纸使运动帧再次越预算）。backlog §N2 已按此重写（旧重开条件保留为历史并逐条标注现状：
  ① 已答为假、② 已失效 moot、③ 仍有效）。
- **边界（不得软化）**：**只有 settled 1-draw 页是干净的 before/after**；**full-view（无剔除）帧仍未修、未测**
  （跳过只在有剔除时生效）；**GPU 执行份额未归因**、**是否还有值得做的 CPU 工作未测**；字段一致是**日志级**
  证据，不是像素级；**wasm 运行时 / Windows/macOS/Android 未测**；单机/单适配器，**不**给出 vendor/视觉/性能结论。

## 每帧 CPU plan 成本：剔除感知的分段页规划（2026-10-17；纯 renderer 改动）

**范围**：`crates/cad-render-wgpu`（`src/plan.rs` 的 `plan_ordered_page` 分段化 + 每批集变更一次的
`BatchFacts` + 每次重启重建的 `OrderedPageIndex` + `plan_from_gpu` 的统一 `usage` 契约 + 7 个新
planner 测试）。**未**改宿主协议、**未**改 UI、**未**新增诊断码。设计动机与状态：
`docs/render-optimization-backlog.md` 的 §N1e（P0 结论工作顺序的 **#1**）；逐条证据见
`docs/validation-plan-cost-2026-10-17.md`。

- **病根（实测）**：`plan_ordered_page` 本应在首个越预算批处停下并保留 cursor，但**完全被剔除的批计零**
  顶点/三角形，而产品的 `max_bytes` 为 `usize::MAX` ⇒ 零计费批**永不失败**；剔除把视图清空时，线性游走
  会**跑到有序表末尾（complex-test 166,346 条）**，为每一条算成本，只为接受一切并画一个 run。实测该病态页
  `plan_ms` 中位 **10.5** / p90 **18.1**、`cpu_ms` **22.4**。
- **改动（分层，无第二套失效真相）**：
  - **剔除无关的批集事实 `BatchFacts`**（`plan.rs:896-995`）——按类（instance/mesh/arc）× chunk 的
    chunk→batch 索引、`always_active`（**无论剔除都计费**的批）集合、类标志 `has_line_class`/
    `has_mesh_class`、`converted_total`、紧凑 `upload_bytes`——**每个批集变更只算一次**，复用既有的
    `draw_plan = None` 失效（`renderer_impl.rs:4364-4377`），**不新增**失效真相。
  - **每次 `ordered` 重建时的索引 `OrderedPageIndex`**（`plan.rs:997-1047`）——批→位置逆表 `batch_pos`
    + **字节前缀和** `bytes_prefix`——在**既有的重启清扫**里随 `ordered` 一起重建
    （`renderer_impl.rs:4409`），跨重启复用分配。
  - `plan_ordered_page` 只走**活动批**（可见 chunk 的批 ∪ `always_active`），由它们的**空隙**导出被接受
    前缀；字节边界来自**前缀和二分查找**（`plan.rs:1100-1119`）。字节计费**仍按批粒度**，故**完全被剔除的
    已转换批仍计字节**（**不是**免费的）——`always_active` 谓词**刻意不**是 `batch_has_visible_work`
    （`plan.rs:912-916`）。
  - **回退**：无剔除、或估计活动批超过 **4096**（`SEGMENTED_ACTIVE_THRESHOLD`，`plan.rs:894`）⇒ 走
    **原线性游走**，不变（缩放出去的大页行为与改动前**完全一致**）。
  - **统一 `usage` 契约（用户批准）**：`plan_ordered_page` 与 `plan_from_gpu` 在失败时共用一条契约——
    顶点/三角形失败**保留失败批的字节**（先计字节后查顶点），字节失败**不保留**（`plan.rs:671-679`）；
    该字段**不**出现在任何 probe 输出里。
  - **未触碰**：`plan_from_gpu` / `plan_cursor_batch` 的函数体、进度下限、draw-order 切分、图像预留/上限
    回落、`ChunkCull` 公式。
- **真实宿主实测（本机原生 Wayland / Vulkan / Quadro P620 / `scale=1`；同 UID、无 root/sudo、无屏录、
  无虚拟显示、exit 0）**：

  | fixture | 页 | `plan_ms` before → after（中位 / p90） | `cpu_ms` before → after（中位 / p90） |
  |---|---|---|---|
  | complex-test | 稳定 1-draw（`cull=1/9362`） | 10.747 / 18.084 → **1.221 / 2.749** | 22.42 / 30.78 → **13.65 / 21.94** |
  | complex-test | 全部帧 | 2.075 / 4.349 → 0.923 / 1.416 | 4.111 / 8.542 → 3.260 / 7.921 |
  | debug-default-fonts | 稳定 1-draw（`cull=1/806`） | 0.677 / 0.759 → **0.114 / 0.213** | 1.244 / 1.367 → 1.144 / 1.829 |

- **独立 orchestrator 复跑（另一次宿主运行，同一 fixture）**：接受**整条**有序表的帧
  （`prog=Some(166346)/Some(166346)`）`plan_ms` 中位由 **10.547（n=47）** 降到 **1.168（n=5）**；
  深剔除帧（kept<100 / 9362）由 **10.541** 降到 **0.588**；两次运行都出现**同一病态页**
  （`draw_calls=1 verts=12 cull=1/9362`），现 `plan_ms=1.168`。
- **测试（实测）**：`cargo test -p cad-render-wgpu --lib` **192 passed / 3 ignored**（**7** 个新 planner
  测试，含一个 **4000 例随机差分测试**，对 `#[cfg(test)]` 的旧规划器参照断言**完全一致**的被接受前缀、
  `over_budget` 类别/requested/limit/skipped、cursor 推进与 pacing 样本）；`--test headless_render`
  **35**；`--test accelerated_tier` **23**、`--test pulled_prefix_equivalence` **9**、
  `--test render_effects` **45**、`--test pick_id` **32**。
- **边界（不得软化）**：
  - complex-test 的 `plan_ms` **p90** 改动后（跨运行 2.53–2.75）**正落在 2.5 ms 边界上**——中位目标达成，
    p90 目标只是勉强。
  - **paced 帧数字被混淆**：自适应 pacing 控制器的轨迹依赖墙钟，不同运行走不同 `budget_scale`/接受路径。
    **只有**稳定的 1-draw 页是**预算无关**的（一切零计费，故该页接受全部），因此是干净的 before/after。
    **引用任何 paced 数字时必须显式说明这一点**。
  - 稳定帧仍要付**重启清扫**（每次相机移动 O(166,346)）外加 `uniform_ms`（~6 ms）、`encode_ms`（~5.2 ms）、
    `submit_ms`（~5 ms）循环——这就是稳定帧 `cpu_ms` 是 13.65 ms 而不是 ~1 ms 的原因。这些循环是**同一种病**，
    仍是显式后续项；本轮**未**修它们。
    **更正（2026-10-17，同日下一轮）**：上面的 `submit_ms`（~5 ms）**有误**——稳定 1-draw 页实测
    `submit_ms` 为 **0.206 ms**（`22.42 = 10.75 + 6.07 + 5.37 + 0.21`），提交在该页**从来不是**瓶颈；
    上述三个循环已由同日下一轮修掉（见本文件顶部条目与
    `docs/validation-uniform-encode-skip-2026-10-17.md`）。
  - 稳定帧上观察到的**逐帧渐进重启**是 **probe/证据驱动重新发出视图变更**（每个稳定帧前都有一次合成
    `pointer_move` 且 `view_rev` 同步递增），**不是**相机 f32 抖动。未做修复，也不应修；记录证据。
  - 单机/单适配器；**不**给出 vendor/视觉/性能结论。

## 测量专用 GPU 帧时间（P0，2026-10-17；measurement-only）

**范围**：`crates/cad-render-wgpu`（新纯模块 `src/gpu_time.rs` + `Renderer::set_gpu_time_measurement`
一次性资源创建 + 帧内两查询 bracket + 帧尾读回 + `FrameStats` 追加字段）+ `crates/cad-engine`
（`attach_device_with_gpu_time` 透传）+ `apps/app-linux`（宿主读 `YACR_PROBE_GPU_TIME` 并经
`gtk_bridge::install` → 设备创建 → 会话透传）。**未**改宿主协议、**未**改 UI、**未**新增诊断码、
**未**改渲染/像素。政策依据：`docs/adr/0013-device-capability-negotiation.md` 第 1 类
（measurement-only）；逐条证据见 `docs/validation-gpu-time-2026-10-17.md`。

- **唯一读取点与分层**：环境变量 **`YACR_PROBE_GPU_TIME`** 只在 `apps/app-linux/src/host.rs:1327`
  读取；**渲染器/引擎从不读环境**——宿主把结果装进显式 `GpuTimeConfig`，经
  `gtk_bridge::install(&window, …, gpu_time)` → 呈现器设备创建（`native_presenter.rs`
  `build_with_surface(…, gpu_time_config)`）→ `session.attach_device_with_gpu_time`
  （`crates/cad-engine/src/render/mod.rs:1105`）→ `Renderer::set_gpu_time_measurement` 逐层显式传参。
- **关闭时（产品默认）**：设备**不请求任何 feature**，`gpu_ms` 保持 `None`，打印一行 `[probe] gpu-time …`
  报 `state=disabled`；**不填估算值**。
- **打开时**：请求 `TIMESTAMP_QUERY | TIMESTAMP_QUERY_INSIDE_ENCODERS`，与 adapter 支持集**取交**；
  本机（P620/Vulkan）两者**均授予**（`state=enabled`）。`TIMESTAMP_QUERY_INSIDE_PASSES` **刻意不请求**
  （两次写入都在**编码器**上）。缺 feature 时显式报 `gpu_time=unsupported(<FEATURE>)`，**绝不**静默回退。
- **读回策略（无额外提交）**：帧**既有**的单个编码器上帧首/帧尾各 `write_timestamp`，随后
  `resolve_query_set` + `copy_buffer_to_buffer` 到 16 B `MAP_READ` 缓冲（`renderer_impl.rs:5538-5543`），
  **同一次提交**；`map_async` 在 `submit` 之后、阻塞 poll **之前**排队，poll 完成映射后读 ticks，
  **不做第二次等待**；ticks 用 `queue.get_timestamp_period()` 经 `ticks_to_ms` 换算。资源
  （`QuerySet` + 256 B `QUERY_RESOLVE` 缓冲 + 16 B `MAP_READ` 缓冲）**一次性创建、逐帧复用**
  （创建计数器恒为 **1**）。
- **上报（追加式）**：`FrameStats::gpu_ms`（`Option<f64>`）与 `gpu_time` 状态；`[probe] actual-render`
  行尾追加 `gpu_time={}`（`host.rs:1993`）；启动一行 `[probe] gpu-time requested={:?} granted={:?} state={}`
  （`native_presenter.rs:530`）。**未新增**诊断码。
- **真实宿主实测（本机原生 Wayland / Vulkan / Quadro P620 / 驱动 580.178.04 / `scale=1`；同 UID、
  无 root/sudo、无屏录、无虚拟显示；四次运行均 exit 0、无 panic、无残留进程）**：
  - 每帧（on/off 各一次）：`complex-test.dwg` off **697** 帧（`gpu_ms=None`、`wait_ms` 中位 4.203、
    `cpu_ms` 4.463）、on **698** 帧（`gpu_ms` 中位 **3.948** / p90 7.479 / max 24.416、`wait_ms` 4.242、
    `cpu_ms` 4.111、gpu/frame **43.8%**、gpu/wait **93.0%**）；`debug-default-fonts.dwg` off **452** 帧
    （`None`、2.234、2.055）、on **462** 帧（`gpu_ms` 中位 **2.643** / p90 6.205 / max 6.600、`wait_ms` 2.909、
    `cpu_ms` 1.439、gpu/frame **55.0%**、gpu/wait **93.9%**）。**关闭腿 0/697 与 0/452 帧带值**（`None` 是
    未请求 feature 的诚实结果，不是缺测）。
  - settled `complete=true` 页（paced）：complex **50** 帧（`gpu_ms` 中位 0.025、`cpu_ms` 22.158、
    `wait_ms` 0.683、frame 22.930、gpu 0.1% / cpu 97.0%）；debug **376** 帧（`gpu_ms` 2.551、`cpu_ms` 1.294、
    `wait_ms` 2.806、frame 6.726、gpu 50.3% / cpu 44.3%）。
  - 每帧 CPU 相位（中位份额）：complex all-paced plan **53.2%** / uniform 22.8% / encode 5.9% / submit 17.2%；
    complex complete plan 48.0% / uniform 26.9% / encode 23.9% / submit 0.9%；debug all plan **55.2%** /
    uniform 7.1% / encode 20.4% / submit 18.1%；debug complete plan 56.3% / uniform 7.7% / encode 19.9% / submit 17.5%。
- **本轮主结论（证据驱动重排优先级）**：N2 重开规则三条件据此判定——**(a) 为假**（`exec ≈ wait_ms`，
  比值 0.93 ⇒ `wait_ms` 就是真实 GPU 执行、present/queue 仅约 6–7%）；**(b) 为真**（paced 帧超目标且
  `cpu_ms` ≈ 帧时间 41–52%）；**(c) 成立**（120 Hz / 8.33 ms 下 complex `cpu_ms` p90 ≈8.5 ms 单独超预算）。
  ⇒ 工作顺序变为 **(1) 每帧 CPU plan 成本**（`plan_ms` 占 CPU 48–56% 且大体与页大小无关——1-draw settled 页
  仍 ~22 ms CPU）→ **(2) N2 流水**（`cpu ≈ gpu ≈ 4 ms`，重叠大致可把帧时间减半，但不减少 CPU 工作量本身）→
  **(3) N5 indirect multi-draw 最后**（只打击 encode 6–24% / submit 1–18%，均不主导，且碰不到 `plan_ms` 与
  GPU 执行）。**N5 前提已被测量且不是瓶颈 ⇒ 降级而非取消**（重促条件见 backlog §N5）。
- **测试（实测）**：`cargo test -p cad-render-wgpu --lib` **185 passed / 3 ignored**；
  `--test headless_render` **35**（含 **3** 个新 `gpu_time_contracts` fence：
  `disabled_requests_no_feature_and_reports_none`、`unsupported_state_names_the_missing_feature`、
  `enabled_measures_without_changing_pixels_and_does_not_grow`）；`--test accelerated_tier` **23**、
  `--test pulled_prefix_equivalence` **9**；`cargo test -p cad-engine` **100 passed / 8 ignored**。
- **门禁（实测）**：`fmt` / `check --all-targets` / `clippy -D warnings` /
  `check --lib --target wasm32-unknown-unknown` 全 clean；release 构建（`--features tauri/custom-protocol`）OK。
- **P0 的未测 / 边界（不得软化）**：**GPU 内部归因未测**（只有每帧总量，未分顶点/三角形吞吐 vs draw 数 vs
  带宽）；**N1d / N2 / N5 是否真能降低 `gpu_ms` 未测**；**真实 adapter 的 `unsupported(<feature>)` 未测**
  （只在 fence 里经**能力注入钩子**模拟——本机两 feature 均授予，无缺该 feature 的真 adapter）；
  **wasm 运行时未测**（只有库编译）；**真实宿主像素捕获未做**（字节一致在 headless harness 证明，宿主两次运行
  只有日志）；**扰动**：测量设备**不是**产品配置（多了 resolve/copy + map 开销）；**vendor / golden / 性能
  结论**：单机、单适配器，**不得**外推。

## 块级视锥剔除（P1，2026-10-17；纯 renderer 改动）

**范围**：`crates/cad-render-wgpu`（新 `src/cull.rs` + 三个 chunk 构建器保留既有 AABB + 逐帧一次可见性测试 +
encode/合并点跳过被剔除 run + `FrameStats`/`[probe] actual-render` 追加字段）。**未**改宿主协议、**未**改 UI、
**未**新增诊断码。设计状态：`docs/render-optimization-backlog.md` 的 P1；逐条证据见
`docs/validation-chunk-culling-2026-10-17.md`。

- **谓词（纯逻辑，无 GPU 依赖）**：`crates/cad-render-wgpu/src/cull.rs`（`WorldAabb` / `ChunkCull` / `build` /
  `ortho_visible` / `perspective_visible` / `class_visible`，**10** 个纯单测）。三个构建器（`plan.rs`）本已为
  各自切分启发式算出的 world-space `f64` AABB 现被**保留**到上传结构（`InstanceStream.aabb` / `GpuMeshChunk.aabb` /
  `ArcStream.aabb`），每帧对 `FrameProjection` 测试**一次**，**O(chunks)**——**不新增**逐实体逐帧计算。
- **保守且单侧**（只会保留，绝不误剔除）：非有限/空/反向 AABB ⇒ 恒可见；跨 chunk 的超大源每个分片携带**整批**
  AABB；混合远处批次的 chunk 从不剔除。
- **测试期 pad（绝不烘焙进存储 box），逐类**：line `3·wpp_line + 1e-6`；mesh `1e-6`（无 stroke/quad 展开）；
  arc `2·wpp_arc + max(0.02·R, 0) + 1e-6`（`R` = chunk AABB 最大半extent）。
  `wpp_line = line_ramp_override.unwrap_or(projection.world_per_px())`、`wpp_arc = projection.world_per_px()`。
  **`lineweight` 不产生 pad 项**——它被**携带但未光栅化**（`line_instanced.wgsl`），**不是**遗漏项。
- **Ortho / Perspective**：Ortho 为精确 `f64` 2 轴 NDC 测试、**z 刻意不测**（深度映射不是可见性判据）；
  Perspective 为 6 平面 p-vertex 测试 + f32 相对 margin（透视原点被 f32 窄化）。
- **记账**：被剔除 run 在**每个** encode/合并点跳过（**从不重排、从不跨空隙合并**）并计**零**顶点/三角形；
  字节记账仍按批粒度（`max_bytes` 产品里为 `usize::MAX`，建模上传驻留而非 draw 工作量）。透明 legacy draw
  与图像**不**剔除。
- **`use_pulled` 合取项**：加速 pulled 路径在**任何**剔除发生时被禁用（`&& !cull.any_culled()`，
  `renderer_impl.rs:4620`）——一次 pulled draw 会重绘**整条 stream/chunk**、会重绘被剔除的几何。**不做
  pulled 子集 draw**（那仍是 **N1d**，**仍开放**）⇒ **剔除目前强制走通用 per-run 组织**（每 run 属单 chunk，
  故整条可跳过）。
- **pick 从不剔除**：`pick.rs:495` 传恒等掩码 `ChunkCull::all_visible()`，命中测试与视图剔除**独立**。
- **上报（追加式）**：`FrameStats` 新增 `cull_chunks_total` / `cull_chunks_kept`；`[probe] actual-render` 在
  `pacing={}` **之后**追加 `cull_chunks={kept}/{total}`。
- **真实宿主实测（本机原生 Wayland / Vulkan / Quadro P620；同 UID、无 root/sudo、无屏录、无虚拟显示；
  两次运行均 exit 0、无 panic、无 invalid-buffer/validation/device-lost 行）**：
  - `fixtures/complex-test.dwg` **681** 页、**161** 页 `kept<total`；`cull_chunks` 区间 **`1/9362` … `9362/9362`**；
    `draw_calls` 区间 **`1` … `3027`**（剔除前基线 **3,027** 在 `cull_chunks=9362/9362` 复现）；最深剔除页
    `draw_calls=1 verts=12 tris=4 instanced=1`；`accelerated=false`、`pulled=active`、`budget_scale` 0.050…1.000、
    `pacing=active`、`complete=true` 50 个 probe、`over_budget=true` 653 页。
  - `fixtures/debug-default-fonts.dwg` **466** 页、**215** 页 `kept<total`；`cull_chunks` 区间 **`1/806` … `806/806`**；
    `draw_calls` 区间 **`1` … `1299`**；最深剔除页 `draw_calls=1 verts=12 tris=4 instanced=1`；`accelerated`
    混合（**163** true / **303** false）、`pulled=active`、`budget_scale` 0.562…4.000、`complete=true` 381 个 probe。
  - post-load 的**平移（pan）**已足以驱动深剔除（`kept` 最低到 **1**）；**无需**单独的 zoom 钩子。
- **测试（实测）**：`cargo test -p cad-render-wgpu --lib` **177 passed / 0 failed**（**10** 个新纯 cull 单测）；
  `--test headless_render` **32 passed**（**6** 个新 GPU fence：`culled_frame_is_byte_identical_and_drops_exactly_the_culled_runs`、
  `large_lineweight_at_the_viewport_edge_is_pixel_identical`、`pick_is_invariant_under_chunk_culling`、
  `culling_does_not_pop_across_a_chunk_border`、`culling_is_byte_identical_on_the_3d_path`、
  `culling_does_not_break_the_progressive_cursor`）；`--test accelerated_tier` **23**、
  `--test pulled_prefix_equivalence` **9**、`--test render_effects` **45**、`--test pick_id` **32**、
  `--test pick_hidden_flags` **8**、`--test sdf_arcs` **17**。
- **门禁（实测）**：`fmt` / `check --all-targets` / `clippy -D warnings` / `check --lib --target wasm32-unknown-unknown`
  全 clean；release 构建（`--features tauri/custom-protocol`）OK。
- **P1 的未测 / 边界（不得软化）**：**逐字节 parity fence 跑在 headless 软件 Vulkan 适配器上，不是硬件**；
  真实宿主两次运行只产出**日志证据**（**无**像素读回）⇒ **剔除比不推出任何 vendor / 精度 / 性能结论**。
  **开放问题（逐字记录，非解释）**：debug 的最大 `draw_calls` 为 **1299**，**高于**更早的 **1,195** 基线，
  实现 lane **未**对账（**可能**不同的相机/页构成）。**未测（NOT RUN）**：全量 workspace 门禁矩阵、打包路径、
  X11 沙箱、其它 GPU/vendor、**视觉判定**。

## 时间驱动分页预算 pacing（N1a，2026-10-17；纯 renderer 改动）

**范围**：`crates/cad-render-wgpu`（新 `src/pacing.rs` + 三个分页规划点接入 + 上报字段）。**未**改宿主协议、
**未**改 UI、**未**新增诊断码。设计动机与状态：`docs/render-optimization-backlog.md` 的 N1a；逐条证据见
`docs/validation-frame-pacing-2026-10-17.md`。

- **控制器（实测实现值）**：纯函数 `step(state, sample)` + `apply_scale`（`pacing.rs`，12 个纯测试）。
  `target_ms=13.0`（默认，可由 `Renderer::set_frame_budget_target_ms` 覆盖）；信号 = 本帧**实测**
  `cpu_ms + wait_ms`（**不是** tick 间隔——那被 vsync 量化、会振荡；也**不是** `frame_total_us`/`present_us`
  ——那含预算控制不了的固定 apply/present 成本）；EWMA `0.25*new + 0.75*old`，**首个样本只播种**
  （第三个被观测帧前不收缩）；死区 `0.95–1.05 × target`；连续 **2** 帧超目标 ⇒ `×0.75`；连续 **8** 帧
  低于目标 ⇒ `×1.08`；`scale ∈ [0.05, 4.0]`（同时作用于顶点与三角形，保持比例，`max_bytes` 不动）；
  方向翻转清反向 streak；**接受 0 批次的帧不是观测**。
- **预算语义**：`Renderer::frame_budget` 是**未 pacing 的上限**；`active_frame_budget()` 是三个分页规划点
  实际使用的**活动预算**；**pick/snap 不 pacing**（用上限）。三态：`pacing=active`（渐进 + 完成信号）、
  `pacing=at-ceiling`（非渐进帧按上限规划）、`pacing=disabled-no-completion-signal`（wasm/无信号；预算与
  静态默认**逐位一致**）。**进度下限**：cursor 批次能装进**上限**时一页至少接受 1 批；被下限强制的一页报
  超预算、原因 `pacing-floor`；只有单个批次**超出上限**才保留既有的显式错误。
- **预存缺陷修复（先于本轮存在，本轮修掉）**：图像**只在清屏页**绘制（续页 `LoadOp::Load`，避免 alpha
  累积），而此前图像只按**剩余**页预算记账 ⇒ 几何在页 1 饱和时被扣下的图像在续页**永不绘制**、
  "complete" 帧**静默缺图**。现在清屏页**先预留整幅图像记账**（paced 预算装不下全部图像时回落**上限**），
  几何再填剩余；**绘制顺序不变**（图像最后）。栅栏：`paced_pages_keep_images_in_the_settled_frame`。
- **上报（追加式）**：`FrameStats` 与 `[probe] actual-render` 在 `pulled={}` **之后**追加 `budget_scale`、
  `budget_target_ms`、`budget_cost_ema_ms`、`budget_verts`、`budget_tris`、`pacing`。`over_budget` 字面含义
  **不变**（本帧计划越过**活动**预算），诊断码仍是 `render.frame_over_budget`；新字段只做分类。
- **真实宿主实测（本机原生 Wayland / Vulkan / Quadro P620 / 驱动 580.178.04 / `scale=1`；同 UID、无
  root/sudo、无屏录、无虚拟显示；exit 0、无 panic、无 `is invalid`、无残留进程）**：
  - `complex-test.dwg` **790** 帧，全部 `pacing=active`，`complete=true` 出现在 seq 1/2/3/171/790。
    逐帧原文（中段 `…` 省略）：
    `[probe] actual-render seq=200 … cpu_ms=3.367 … wait_ms=5.631 … over_budget=true
    prog=Some(2447)/Some(166346) complete=false … pulled=active budget_scale=0.262 budget_target_ms=13.00
    budget_cost_ema_ms=8.94 budget_verts=2099799 budget_tris=524950 pacing=active`。
    轨迹：**709 ms** 首帧离群把 `budget_scale` 从 **1.000 → 0.750 → 0.562 → 0.422 → 0.100**（seq 4–21），
    同时 `wait_ms` **21.0 → 2.3 ms**；后段 `wait_ms` **7.7 → 11.3 ms**（seq 746–750）把 EMA 抬到
    **13.87/14.67**、`scale` **0.336 → 0.252**（seq 751）；稳定下来的一页达到 `complete=true`。
  - `debug-default-fonts.dwg` **458** 帧，全部 `pacing=active`，`complete=true` **375** / `false` **83**。逐帧原文：
    `[probe] actual-render seq=200 … cpu_ms=1.102 … wait_ms=2.886 … over_budget=false
    prog=Some(10548)/Some(10548) complete=true … pulled=active budget_scale=3.567 budget_target_ms=13.00
    budget_cost_ema_ms=3.97 budget_verts=28535313 budget_tris=7133828 pacing=active`。
    轨迹：一个 **46 ms** 离群把 `scale` **1.000 → 0.422**，随后慢升路径抬到上夹紧
    **`budget_scale=4.000` / 32,000,000 verts**（到 seq 240）。
- **测试（实测）**：`cargo test -p cad-render-wgpu --lib -- --test-threads=1` **167 passed / 3 ignored**；
  `--test headless_render` **26**（含 6 个新 GPU fence：逐帧可调预算 / pick 不变 / 进度下限 /
  settled 帧图像完整 / disabled 与静态默认逐位一致 / 场景变更重置代价知识）；`--test accelerated_tier` **23**、
  `--test pulled_prefix_equivalence` **9**、`--test render_effects` **45**、`--test pick_id` **32**、
  `--test pick_hidden_flags` **8**；`cargo test -p cad-engine` **100 passed / 8 ignored**。
- **N1a 的未测**：wasm/浏览器真实运行时（只有 wasm **库**编译通过；`disabled-no-completion-signal` 是在
  native 上用测试 override 断言的）；Windows/macOS/Android；图像与后续页几何重叠时的**跨页 z-order**
  （图像只画清屏页；fence 只断言图像完整性）；**视觉/截图判定**（headless 只做逐字节 parity，宿主运行是
  **从日志读的**）；**任何 GPU 时间**（`gpu_ms=None`，`wait_ms` 仍是阻塞 poll/呈现代理）；
  **120 Hz / 刷新率自适应**（目标是显式 13 ms 常数，不是由刷新率推导）。
- **N2（去掉每帧阻塞 poll）本轮决定：延后，不取消**——理由与重开条件见
  `docs/render-optimization-backlog.md` 的 N2。P0 现状：pacing 现提供**每帧代价信号**，但**仍无 GPU 时间**。

## 设备 limits 协商 + Flatpak 打包重启（2026-10-17，两项交付）

**范围**：A = renderer 设备 limits 协商与 pulled 分配失败显式化（Rust 源码改动）；B = React/Tauri 宿主的
Flatpak 打包重启（`packaging/flatpak/**` + `scripts/package-linux-release.sh`）。两项**同日、独立**，
证据各自独立；A 的宿主数字来自 `target/release/yacr`，B 的宿主数字来自沙箱内 `flatpak run`。

### A. 设备 limits 协商（实测；单机 Quadro P620 / Vulkan）

- **改动前**：两处设备创建点（`crates/cad-render-wgpu/src/headless.rs`、`native_presenter.rs`）都用
  `DeviceDescriptor { ..Default::default() }` ⇒ 设备 `max_buffer_size` **268,435,456 B（256 MiB）**，
  而 adapter 报 **4,292,870,144 B**。这是**请求限制**，不是硬件上限。
- **改动**：纯策略函数 `negotiated_device_limits`（`headless.rs:232`）以
  `Limits::default().or_worse_values_from(adapter)` 为底，**只**把 `max_buffer_size` 抬到 adapter 值；
  两处设备创建点都经同一 `request_device_with_negotiated_limits`（`headless.rs:273`）请求设备。
  本机实测：`[probe] device-limits adapter_max_buffer_size=4292870144
  requested_max_buffer_size=4292870144 granted_max_buffer_size=4292870144
  requested_max_storage_buffer_binding_size=134217728
  granted_max_storage_buffer_binding_size=134217728 degraded=false reason=-`。
- **`max_storage_buffer_binding_size` 刻意不抬**（adapter 报 2,147,483,644 B，仍请求 WebGPU 默认
  134,217,728 B）：**实测**抬高后 pulled 分段器产出约 2 GiB 单次分配，一次 pulled 重建的
  `create_buffer` 失败、wgpu 30.0.1 返回**无效 handle**，其下一次使用表现为 **validation** 错误
  （`Buffer with 'cad-pulled-line-storage' label is invalid`），`ErrorFilter::OutOfMemory` 作用域
  **捕获不到** ⇒ 默认处理器 abort（exit 101）。锁回默认值后同一重建成功，仅分段更多、draw 更多，
  **像素不变**。
- **降级**：`request_device` 以协商值失败 ⇒ 用 `Limits::default()` 重试并报 `degraded=true reason=…`；
  两次都失败 ⇒ 显式错误。不静默、不 panic。
- **真实宿主（实测）**：`fixtures/complex-test.dwg` 由
  `pulled=skipped-insufficient-vram(required_bytes=2281858648,budget_bytes=268435456)` 变为
  **`pulled=active`**，`entityCount=44591`、settled 帧 `complete=true`、自然 exit 0；
  `fixtures/debug-default-fonts.dwg` `pulled=active`、`entityCount=10656`。无 panic。
- **同轮修掉的预存缺陷（被本次放宽 unmask，不是本次引入）**：驱动拒绝的 `create_buffer` 返回无效
  handle，随后的 `write_buffer` 以 **validation** 错误出现，OOM-only 作用域不捕获 ⇒ 进程 abort。
  256 MiB 预算下这条路实际不可达，4 GiB 下可达。修复在
  `crates/cad-render-wgpu/src/renderer_impl.rs`：pulled 构建置于嵌套 `Validation` + `OutOfMemory`
  作用域（每条路径 LIFO push/pop，`ErrorScopeGuard::drop` 兜底），每次分配单独守护且**先验证创建**
  再上传/绑定，首次失败即中止并丢弃半成品，本帧回退通用 per-run 并给出显式状态：
  设备报 OOM ⇒ `pulled=skipped-out-of-memory(required_bytes=…,budget_bytes=…,message=…)`；
  其它错误 ⇒ 新增 **`pulled=skipped-allocation-failed(…)`**（**不**冒充显存耗尽证据）。
  容量门、通用路径与像素不变。
- **测试（实测）**：`cargo test -p cad-render-wgpu --lib -- --test-threads=1` → 155 passed / 0 failed /
  3 ignored；`--test accelerated_tier` 23、`--test pulled_prefix_equivalence` 9、`--test headless_render`
  19。新增确定性设备测试 `refused_allocation_is_reported_without_panicking`（请求
  `max_buffer_size + 1` 触发真实的 invalid-handle/validation 机制，不耗显存）。
- **修复后真实宿主（实测）**：`complex-test.dwg` 持续 `pulled=active`、`complete=true`、exit 0、
  零 `is invalid`、零 panic。
- **A 的未测**：真实驱动/显存 OOM 在本机**不可模拟**（新测试触发的不是 OOM）；其它 GPU/vendor、
  feature 请求（仍为空）、`gpu_ms`（仍 `None`）均未测。策略与实现状态见
  `docs/adr/0013-device-capability-negotiation.md`。

### B. Flatpak 打包重启（实测；本机 GNOME/Mutter + P620 + `scale=1`）

- **工具链（实测）**：`flatpak 1.16.6`、`flatpak-builder 1.4.8`，`org.gnome.Platform//51` 与
  `org.gnome.Sdk//51` 装在 **user scope**。**镜像说明**：运行时/依赖经本机 `flathub-mirror` 远端
  取得（直连 CDN 约 38 KB/s）；摘要签名用官方 key 校验通过（subkey `562702E9E3ED7EE8`，属
  Flathub Repo Signing Key `4184DD4D907A7CAE`）⇒ 这是**传输镜像**，不是替代信任根。
- **glibc/webkit 前置（实测）**：release 二进制最高需要 `GLIBC_2.43`（`objdump -T`）；
  `org.gnome.Platform//51` 提供 **glibc 2.44** 与 `libwebkit2gtk-4.1.so.0` / `libgtk-3`。GNOME 50 运行时
  为 glibc 2.42（低于该要求）⇒ 这也是选 51 的原因（**未**单独跑 50 验证加载失败）。
- **新增/重启的产物**：`packaging/flatpak/dev.yacr.app.yml`（app-id `dev.yacr.app`、runtime
  `org.gnome.Platform` 51、`command: yacr`、finish-args `--share=ipc --socket=wayland
  --socket=fallback-x11 --device=dri --filesystem=home`、prebuilt `type: dir` payload
  `target/flatpak/payload` = `bin/yacr` + committed `fonts/`，**不含 CLI**）、`dev.yacr.app.desktop`
  （`Exec=yacr %F`）、`dev.yacr.app.metainfo.xml`（AGPL-3.0 / CC0-1.0、mediatypes）、
  `scripts/package-linux-release.sh`（从 NOT IMPLEMENTED stub 重写）；
  `scripts/test-package-linux-release.py` 断言**重启后**的静态契约（11/11 实测通过）。
- **`%F` 是硬要求**：宿主拒绝任何含 `://` 的参数（`LaunchOptions::parse_args`），`%U` 会让文件管理器
  打开失败；裸位置参数现在按 `--open` 处理。
- **UI 不单独发运**：`ui/dist` 在构建时嵌入，`--features tauri/custom-protocol` 必需。脚本用
  **hashed asset 文件名**（`ui/dist/assets/index-*.js|css`）验证嵌入；**不**用
  `http://localhost:1420` 当判据——该串在 dev 与 release 里都存在（嵌入的 `tauri.conf.json` 带
  `build.devUrl`），第一版守卫是**假阳性**，会挡下正确构建（2026-10-17 发现并修掉）。
- **构建/安装（实测）**：`scripts/package-linux-release.sh` → exit 0；产出
  `target/release/dist/yacr-0.1.2-linux-x86_64.flatpak`，**6,724,496 B**，sha256
  `cd453e230aac16f64583c25f34552c561ce0761f7338a9c4d31fbde484661e90`；`flatpak install --user`
  用同 app-id/branch 覆盖旧 0.1.1 Slint 包；`flatpak info dev.yacr.app` 显示 0.1.2 / AGPL-3.0 /
  runtime `org.gnome.Platform//51`；`xdg-mime query default image/vnd.dwg` → `dev.yacr.app.desktop`。
- **沙箱运行（同 UID，无 root/sudo，无屏录，Wayland 会话；实测）**：
  `debug-default-fonts.dwg` → `tauri://localhost` Finished、Vulkan `Quadro P620`、
  `entityCount=10656`、`pulled=active`、exit 0；`complex-test.dwg` → 523 帧 `pulled=active`、
  `granted_max_buffer_size=4292870144`、`degraded=false`、exit 0、**无 panic、无 `is invalid`**；
  `--file-forwarding … @@ <path> @@`：**home** 路径原样传入（`synthetic-four-lines.dwg`，4 实体）；
  **`/tmp`** 路径变为 `/run/user/1000/doc/9f5d78c3/fwd-outside.dwg`（document portal，4 实体）——
  该模式下**图纸的同级文件不可见**，故保留 `--filesystem=home`。
- **未发布 Flathub**（用户 2026-10-17 明确决定）：bundle 只是**本地产物**；没有提交、审核或 CI 发布。
- **B 的未测**：沙箱内 X11（`--socket=fallback-x11`）；本机之外的宿主；沙箱内 portal 文件选择器；
  沙箱窗口的视觉/截图像素判定；Wayland 子表面的视觉判读；其它 GPU/vendor；非 x86_64 Flatpak。
- **历史说明**：`packaging/flatpak/**` 曾在 `7d83572` 被删除，2026-10-17 是**重新引入**；旧删除是历史，
  不是本轮回滚的“事故”。旧 Slint 流水线事实见 `docs/linux-app.md` 历史小节。
- 详情与逐条结果：`docs/flatpak.md`、`docs/validation-flatpak-linux-2026-10-17.md`；
  `docs/compatibility.md` 的 limits 行与 `docs/render-optimization-backlog.md` 的 P0/N3 已同步。

## 自动证据采集脚本（2026-10-17；本轮交付；无 Rust 改动）

- **新增文件**（**未**改任何 Rust 源码、**未**改既有脚本）：`scripts/collect-evidence.sh`（一键：
  前端构建 → `cargo build --release --locked -p app-linux --bin yacr --features tauri/custom-protocol`
  → 按后端启动真实 `yacr --open … --fonts-dir …`（**非** `--probe` 假打开）→ 截图 → 汇总日志）；
  `scripts/screencast-capture.py`（同 UID `org.gnome.Mutter.ScreenCast` → GStreamer `pipewiresrc`
  单帧 PNG）；`scripts/test-collect-evidence.py`（22 契约）。
- **行为**：X11 后端抓窗口**自身 drawable**（`shot-top`/`shot-cad`，`gcc` 现场编译
  `apps/app-linux/scripts/x11-window-grab.c`）；原生 Wayland 后端抓合成帧（`wayland-frame.png`，整屏
  monitor）。输出 `${YACR_EVIDENCE_ROOT:-/tmp/opencode}/yacr-evidence-<UTC>Z/`（`results.tsv` 逐后端
  记录 `NATURAL_EXIT`/`PLANNED_TIMEOUT`/`SKIPPED`/`NO_DISPLAY` + 真实退出码 + 截图 `captured`/`missing`
  + `README.md`）。CLI `--backend auto|x11|wayland|both`；探针 `YACR_PROBE_CAD_LOG=1` +
  `YACR_PROBE_LIFECYCLE=<mode>`（默认 `exit`）+ 默认 `YACR_PROBE_NO_MAXIMIZE=1`。
- **本轮实际运行证据（本机 GNOME/Mutter + Quadro P620，非软件后端）**：
  `/tmp/opencode/yacr-evidence-20261017T005210Z/` — `x11` 与 `wayland` 两后端均
  `NATURAL_EXIT exit=0 screenshot=captured`；X11 `shot-top.png` 1200x800、`shot-cad.png` 1166x513
  （与当轮日志 `geometry` 一致，`cad-frame … complete=current_scene`、`entityCount:4`、camera
  `[1.5,0.5]/0.00503`）；Wayland `wayland-frame.png` 1920x1080（`bridge wayland presenter ready …
  adapter=Quadro P620`）。`bash -n` / `py_compile` / 契约测试 **22/22** 全过。
- **截图视觉读判（observer，独立读判，非产品验收）**：三张 PNG 均为**真实应用画面、非空**——
  React chrome（tabs 文件/常用/绘图/测量/批注/视图、语言 zh-CN、ribbon、命令行、状态栏）+ 纯黑 CAD
  视口内的 fixture 五段线（白、反走样）；`shot-cad` 的 `mean=1.0` 属"黑底细白线"正常现象，非缺陷；
  Wayland 帧为整屏合成（含桌面其它窗口、`RecordMonitor` 特性）。**不**据此宣称视觉正确性/精度/golden。
- **授权边界（脚本设计即如此，非事后声明）**：以同 UID 用户会话运行，**不提权 / 不 `sudo` / 不以 root /
  不解锁或修改会话 / 不改 portal·ScreenCast 权限 / 不装包 / 不起虚拟或离屏显示**；无真实显示 ⇒ **非零
  退出**而非假装成功；截图仅本地保存、不上传。**不**据此宣称完整/跨平台/vendor/精度/性能验收；
  X11 窗口截图**不是**原生证据。

## 场景空间索引与交互架构（2026-10-17；ADR-0012；架构固化轮本无实现，S1 点选切片随后交付）

- **决策（Accepted）**：`docs/adr/0012-scene-spatial-index-and-interaction.md` 固化五级空间过滤模型
  （Scene/Viewport → Chunk → Entity Bounds / R-tree → PickProxy·Primitive Bounds → Exact Geometry → Ranking）、
  三段纪律（BBox 找候选 / Geometry 定命中 / screen-space distance 定排序）与**早判不对称性**：
  Pick 的 bounds 不相交是可靠早退、但**未知/无效 bounds 必须落回精确判定**；Window Selection 只有
  `rect ⊇ bounds` 是**可靠早接受**、`¬contains` **不得**判 reject（本仓 bounds 保守、且 text/凸度 polyline/mesh
  的 `local_pick_bounds` 为 `None`，会误拒已完全选中的图元）；Crossing Selection 只有
  `rect ∩ bounds = ∅` 是可靠早退、相交**仅作候选**。任何 bounds 早判都必须基于**当前** geometry 版本。
- **索引结构**：顶层固定/松弛 chunk 网格 + 每 chunk 内 `rstar 0.13`（MIT/Apache-2.0，MSRV 1.85，`no_std`）
  `bulk_load` 不可变快照；`rstar` 唯一落点 `cad-spatial`；**必须**实现规范 §8.1 强制的**超大图元策略**
  （松弛边界 + 单独列表 / 子 AABB，禁止跨块复制膨胀）；**不**新增并行版本计数器（快照版本取自
  `Revision`，陈旧判定复用 `ChangeSet::follows()` / `TaskStamp`）。
- **明确否决**：全局单棵 rstar 先行；`DynamicEntitySet`（§4.7 拖拽只改 `PreviewState` + 一次提交）；
  chunk 级并行渲染/框选；`SpatialQueryService` trait 层；每帧重新收集 Primitive / 合批（与 ADR-0007 静态
  `GpuBatch` + §8.1 冲突）；`render_bounds(view_scale)` 尺度参数化（迫使 pan/zoom 重建索引）；新脏标记体系
  与 `EntityChange`（第二套失效真相，违铁律 4）；独立 `PickProxy` 类型层级；现在切换屏幕距离判定/排序
  （正交 2D 下与世界射线容差等价，纯返工）。
- **采纳但受限**：`trait Pickable`（用户要求引入）**必须**三态返回
  `PickVerdict::{ Hit(PickHit), Miss, Unsupported(&'static str) }`，**禁止**裸 `Option`（会把"未支持"静默成
  "未命中"，违反铁律 2）。圆/弧拾取按 AutoCAD 惯例**仅圆周可选**，内部可选须显式产品选项 + 测试。
- **实施顺序（垂直切片优先）**：S1 点选闭环（DOM → `SelectAt` → `pick_at_screen` → 高亮/属性）→ S2 Hover
  （兼作全量拾取路径的第一次真实压力测量）→ S3 Snap 接线 → S4 索引与增量失效接入热路径（由 S2 实测门控）
  → S5 Window/Crossing 框选（串行）→ S6 按实测再评估；**Dirty Region / Tile 排最后**（backlog P3）。
- **本轮证据边界**：ADR-0012 为**架构固化**（该 ADR 轮**未**运行任何测试/门禁/渲染）；`rstar` **尚未引入**，
  `Cargo.lock` **未改**（更新与许可核查为未决项）。相关待办见 `docs/render-optimization-backlog.md`
  的 S1–S6 节。**S1 实现轮的证据与 NOT RUN 边界单列于下方 S1 小节**。

### S1 交付：点选垂直切片（2026-10-17，有界、合成/本地）

- **两条并行 lane**（ADR-0012 §10 item S1）：
  - **Rust**（`crates/cad-ui-core`、`crates/cad-host`）：`protocol.rs` 新增公开常量
    `BUTTON_PRIMARY = 0` / `BUTTON_MIDDLE = 1` / `BUTTON_SECONDARY = 2`；`ProbeInput` 新增可选
    `button: Option<i32>`（`#[serde(default)]`），缺省/`null` ⇒ 主键，超范围值（含 DOM 的 `-1`）以稳定原因
    拒绝；`InputKind` **未**扩展。`session.rs` 保留 `PointerPress { button, down, dragged }`，`pointer_move`
    优先按下键，越过 4.0 逻辑像素圆盘即锁存 `dragged`；主键“按下+抬起、无拖拽、位移 < 4.0 逻辑像素”发出
    `Pointer{Up}` **与** `SelectAt{Replace}`（携带当前 `metrics_revision`）。**主键专属**；中/右键永不选择；
    左/中键拖拽平移；框选**未**实现（S5）。按下状态仅在全部请求被接纳后才提交。
  - **React**（`apps/app-linux/ui`、`crates/cad-ui-core/i18n/{zh-CN,en}.json`）：`input.mjs` 在指针载荷携带
    DOM `button`；**P0 修复**让 `pointermove`/滚轮**省略**该字段（DOM 上报 `-1`，而线协议拒绝 `-1` 而非猜测）。
    `cad.mjs` 的 `parseCadStatus` 严格校验 `selection` 块（畸形 ⇒ `CAD_STATUS_INVALID`，不臆造空状态）；未知
    行键容忍并原样渲染机器键。新增只读属性面板 `components/PropertiesPalette.jsx`（状态栏停靠，默认**关闭**——
    停靠会改变分区宽度且原生 CAD 表面几何敏感，启动几何保持逐字节一致），仅在打开文档时显示；两目录共加 40 个
    `properties.row.<machine key>` 标签键 + `properties.close`。
- **僵化 `selection` 状态块**（`CadStatus.selection`，**永久存在、永不为 `null`**，camelCase）：
  `{"count": <int>=0>, "empty": <bool>, "rows": [{"key": "<机器键，永不翻译>", "value": "<string>"}], "mixedKeys": ["<机器键>"]}`；
  不变量 `empty == (count == 0)`，`rows` 仅在 `count == 1` 时非空，`mixedKeys` 仅在 `count > 1` 时非空；
  探测/无能力状态携带 `{count:0, empty:true, rows:[], mixedKeys:[]}`；单个不可解析引用如实报告 `id` +
  `status:"unresolved"` 行，不伪造几何。
- **按键契约**：`button` 可选，按 DOM `MouseEvent.button` 编码（0/1/2）；**无按键变化的事件必须省略该字段**；
  `-1` 及任何超范围值被拒，稳定原因
  `unsupported pointer button: N (expected 0, 1 or 2, or omit it for primary)`。
- **P0 集成缺陷（对账时发现）**：`input.mjs` 原本**无条件**上报 `event.button`，而 DOM 在 `pointermove`/滚轮上
  报告 `-1`（“无按键变化”）；线协议**拒绝** `-1` 而非猜测主键。若放任，**正常移动即会以稳定原因失败**。该冲突
  **仅靠跨 lane 阅读两处源码才被发现**（Rust lane 的拒绝语义 vs React lane 的无条件上报）——任一 lane 的**单侧**
  测试都不会暴露它。**修复已合入并验证**：`input.mjs` 仅在确有按键变化时携带该字段（`buttonField`；`pointermove`/
  滚轮**省略**），UI 测试显式断言 `pointermove` 与滚轮载荷**不含** `button` 键（`npm test` 58/58 通过）。
- **证据（实际运行，合成/本地）**：`cargo fmt -p cad-ui-core -p cad-host -- --check` PASS；
  `cargo check -p cad-ui-core -p cad-host --locked` PASS；`cargo clippy -p cad-ui-core -p cad-host --all-targets
  --locked -- -D warnings` PASS（0 warning）；`cargo test -p cad-ui-core --locked` **77 passed / 0 failed**；
  `cargo test -p cad-host --locked` **92 passed / 0 failed / 1 ignored**（预存 `perf_baseline`，需 `YACR_TEST_DWG`）；
  额外 `cargo check -p app-linux --locked` PASS；`(cd apps/app-linux/ui && npm ci)` OK、`npm test`
  **58 passed / 0 failed**、`npm run build` OK（41 modules）；`check-i18n.py` **OK: 404 keys, 2 catalogs**。
- **门禁（主控在合入前实际运行，全树）**：`scripts/check-architecture.py`（27 packages、无环、核心边界完整）、
  `check-i18n.py`（404 keys / 2 catalogs）、`check-fixture-manifest.py`（22 fixtures）、`check-workflows.py` 全部 PASS；
  `cargo fmt -p cad-ui-core -p cad-host -- --check` PASS；`cargo test -p cad-ui-core --locked` **77 passed** 与
  `cargo test -p cad-host --locked` **92 passed / 1 ignored** 由主控复跑一致。
- **NOT RUN（诚实边界）**：任何 GUI/原生窗口/真实 GPU 运行、任何截图、任何真实 DWG、全量 workspace 测试矩阵，
  以及本切片的 wasm/Android/Windows/macOS 目标。**这是有界、合成/本地的垂直切片**，**不**构成完整 CAD / 跨平台 / 真机 / 视觉
  验收；点选自始至终只在**合成/本地会话**中验证，`SelectionRef` **未**在真实宿主窗口闭环。

### S2 交付：hover 接线 + `pointer_leave` 跟进（2026-10-17，有界、合成/本地）

- **落地（按 crate）**：
  - **`crates/cad-ui-core`**（`protocol.rs`）：新增 `InputKind::PointerLeave`，线字符串恰为 `pointer_leave`，
    **非终态**（`is_terminal` 仅含 `pointer_up`/`pointer_cancel`/`blur`）；`InputTracker::apply` 对其做**保状态空操作**
    （sequence 仍被消费，marker/press/dirty 均不变）。据此**当前 kind 集合**为 `pointer_down`、`pointer_move`、
    `pointer_up`、`pointer_cancel`、`pointer_leave`、`wheel`、`resize`、`blur`（`docs/input.md` §1.1.1 是当前线载荷契约；
    **未**改已废弃的 ADR-0006）。
  - **`crates/cad-ui-core`**（`session.rs`）：`plan_input` 的 `PointerMove` 分支在**空闲**（无按下）时，除普通
    `Pointer{Move}` 外**额外**发出 `InteractionRequest::MoveCursor(CanvasLocal{ logical_position, metrics_revision })`；
    有按下保持旧单请求形态。`PointerLeave` → **既有** `ClearCursor`（不带坐标、不要求 revision，且**不扰动**进行中的按下）。
  - **`crates/cad-host`**：`HoverState`（宿主本地、**单个** ref；`lib.rs:186-208`）+ `MoveCursor` hover 分支用与点选**完全相同**的
    `pick_at_screen`（`TolerancePolicy::default()` + `BackFacePolicy::Cull`，**无**新容差/排序），结果喂 `OverlayInputs.hovered`
    （`state.rs`；此前恒空）；`ClearCursor`/hover `MoveCursor` 无捕获时归 `DirtyScope::Overlay`（`input.rs:8-31`）。
  - **`apps/app-linux/ui/src/input.mjs`**：`pointerleave` 处理器仅在**无按下**时压入 `{ kind: 'pointer_leave' }`
    （载荷恰为 kind + lane `sequence`，**无**位置/按键/metrics、**不** `preventDefault`），且**先 flush** 待发的 rAF 合并移动。
  - `crates/cad-app` **未改**；`hidden` 生产者**未**接线（受 G2 门控）。
- **冻结行为**：
  - hover 是**实体级、单个**：空闲移动 → 与点选相同拾取路径取**排名第一** ref；命中点亮 `FLAG_HOVERED`，未命中**立即清除**（无陈旧着色）。
  - **抑制/清除**：按下/导航/捕获**拥有**指针时抑制，按下开始/`Cancel`/`ClearCursor`/pointer cancel/`blur`/未命中即清除；
    拾取**报错**先清 hover，再把错误作为**显式拒绝**传播（不静默空成功）。
  - **节流（无时间窗）**：依赖 UI 既有 **rAF 合并**；宿主在逻辑位置**未变**时**整段跳过**拾取查询。这是行为契约
    （以查询计数这类可观测量表述），**不是**频率/性能承诺。
  - **`pointer_leave` 契约**：瞬态、**非终态**；映射既有 `ClearCursor`；**不携带**坐标、**不要求** revision；**不扰动**进行中的按下；
    宿主无捕获时为 **overlay-only**。
  - **用户可见修正（行为变更）**：此前会话**从不**发 `MoveCursor`，故捕获（绘制/测量）进行中的空闲移动**不**到达既有工具光标分支；
    现在这类移动到达（由捕获拥有），工具预览光标跟随指针。这是**用户可见修复**，**非**改 `move_tool_cursor` 语义。
- **对账发现的顺序缺陷（同类于早期 `button: -1`）**：`pointerleave` 若不**先 flush** 待发的 rAF 合并移动，合并移动会在**下一帧**、
  即 leave **之后**到达——宿主先因 leave 清掉 hover，又据一个**已离面的陈旧位置**重新点亮 hover。该冲突**仅靠跨 lane 阅读两侧
  源码**才发现（React lane 的 flush 时机 vs 宿主 leave→clear 语义），任一 lane 的**单侧**测试都不会暴露。**修复已合入并验证**
  （`input.mjs` 的 `leave` 先 `flush()` 再压 `pointer_leave`；UI 测试覆盖）。
- **证据（主控在最终树独立复跑，全部 PASS）**：`scripts/check-architecture.py`（27 packages、无环、核心边界完整）、
  `scripts/check-i18n.py`（404 keys、2 catalogs）、`scripts/check-fixture-manifest.py`（22 fixtures）、`scripts/check-workflows.py`；
  `cargo fmt -p cad-ui-core -p cad-host -- --check`；`cargo clippy -p cad-ui-core -p cad-host --all-targets --locked -- -D warnings`
  （0 warning）；`cargo check -p app-linux --locked` PASS；`cargo test -p cad-ui-core --locked` **91 passed / 0 failed**；
  `cargo test -p cad-host --locked` **102 passed / 0 failed / 1 ignored**（预存 `perf_baseline`，需 `YACR_TEST_DWG`）；
  `(cd apps/app-linux/ui && npm test)` **62 passed / 0 failed**、`npm run build` OK；可选探针行仅在 `YACR_PROFILE_PICK=1`
  时出现（未设该变量时 **0** 行）。该探针 `YACR_PROFILE_PICK hover elapsed_ms=… picked=…` 的**任何数字仅为合成/本地**
  （记录运行用 6 实体 demo 文档，无真实 DWG/GPU），**不是**性能结论。
- **NOT RUN（诚实边界）**：任何 GUI / 原生窗口 / 真实 GPU / 截图 / 真实 DWG、全量 workspace 测试矩阵，以及本切片的
  wasm/Android/Windows/macOS 目标。**这是有界、合成/本地的切片**：hover 与点选**从未**在真实宿主窗口闭环，**不**构成完整
  CAD / 跨平台 / 真机 / 视觉验收。

### 宿主支持位置参数打开图纸（2026-10-17，真机验证）

- **需求**：`yacr <path>` 直接打开图纸——flatpak 后由文件管理器（`Exec=… %F`）调用 `yacr` 的前置条件。
- **实现**（`apps/app-linux/src/host.rs:91-162`）：参数解析拆为**纯函数** `LaunchOptions::parse_args`
  → `(LaunchOptions, Vec<String>)`，`from_args` 只负责打印诊断；`--open/--fonts-dir/--images-dir/--probe`
  行为**逐字节保持**。裸位置参数＝图纸路径；多个位置参数只取**第一个**并**显式诊断**其余；`--open`
  **优先**（与顺序无关）且被忽略的位置参数**显式报告**；含 `://` 的参数（`file://`）**显式拒绝**、
  绝不误当路径；未知 `-flag` 仍忽略；无参数 ⇒ `drawing = None`（demo 文档）。诊断串为英文
  `[probe]`/`error:` 前缀，**未**新增 i18n 键、**未**新增用户可见文案，`entry.rs` 未改。
- **真机证据（主控在本机用户自身 Wayland 会话内实测，真实 GPU）**：
  - 构建：`cargo build --release --locked -p app-linux --bin yacr --features tauri/custom-protocol` → OK。
  - `./target/release/yacr fixtures/debug-default-fonts.dwg`（**位置参数**）→ `exit=0`（平移驱动自然退出）、
    `[probe] page_load url=tauri://localhost`（内嵌前端）、`positional drawing argument:` 与 `startup drawing:`
    各 1 行、**473 行 `actual-render`**、`cad_status` 报 `documentLabel=fixtures/debug-default-fonts.dwg,
    entityCount=10656`、**零 OOM / 零 device lost**。
  - 回归：`--open <same>` → 同样 `entityCount=10656`、`exit=0`。
  - 边界：`/tmp/opencode/does-not-exist.dwg` → `cad_status.error` 显式报 `drawing path does not exist: …`
    （96 行），**不**静默成功；`file:///…` → `error: unsupported argument …` 且 **0** 行 `startup drawing`
    （**未**误加载）。
  - 单元测试 `cargo test -p app-linux` **74 passed / 0 failed**（含 6 个新测）；`fmt`/`clippy -D warnings`
    /`check-i18n.py`(404 keys) 由主控独立复跑，全 PASS。
- **流程教训（诚实记录）**：主控一度用**普通** `cargo build --release -p app-linux` 重建，产出指向
  `http://localhost:1420` 的 **dev-URL 二进制**（症状 `page_load url=http://localhost:1420/` + 持续
  `cad-frame no-session`），与既有文档 `docs/react-linux-app.md:46` 已写明的正确命令不符——属**流程疏漏**，
  按正确命令重建后恢复正常。该陷阱的**症状**已补记入 `docs/react-linux-app.md` 启动段。
- **NOT RUN**：X11/XWayland 后端下的同一验证；flatpak 打包与 desktop entry（打包管线 2026-10-16
  已退役，`MimeType`/`Exec=… %F` 属未来替代打包轮）；多文档打开（NOT IMPLEMENTED）。

### 真实宿主帧成本探针：解释手测的 `complex-test.dwg` 卡顿（2026-10-17，真实运行；**非**验收）

- **缘起**：用户手测 release 宿主：`debug-default-fonts.dwg` 的 pan/zoom 较流畅，`complex-test.dwg` 明显卡顿；
  且**深度放大到大部分图元出屏后仍卡**。主控在本机**用户自身 Wayland 会话内**用**已有** release 二进制
  （**未**重新构建）实测取证：`YACR_PROBE_CAD_LOG=1` + 有界 `YACR_PROBE_POSTLOAD_PAN=1`（一次 8 s 真实平移手势），
  两个 fixture 各一次，均 `exit=0`。本轮**未**跑 cargo/npm/门禁（无源码改动）。
- **环境**：后端 **Wayland**（宿主决议并打印）、`adapter=Quadro P620`、`api=vulkan`、**`tier=accelerated`**（两图一致）；
  `target/release/yacr` `sha256 61d5d4ae…`（08:53 构建，prebuilt）；**`gpu_ms=None`——无 timestamp query，GPU 时间仍未测量**，
  `wait_ms` 仅为 GPU/呈现等待的**代理**。
- **实测（逐帧中位）**：complex `entityCount=44,591`、359 帧、`draw_calls` **3,027**、`verts` **5,454,559**、
  `tris` 1,999,514（**饱和在 ≈2.0M**）、`wait_ms` **19.382**（p90 22.652）、`cpu_ms` 8.274；debug `entityCount=10,656`、
  479 帧、1,195 / 2,170,905 / 1,999,961 / **2.895**（p90 5.369）/ 2.072。
- **结论**：① 手感差异由 **GPU/呈现等待主导**——complex 单帧约 **8.3 ms CPU + 19.4 ms 等待 ≈ 28 ms（≈36 fps）**，
  debug 约 **2.1 + 2.9 ≈ 5 ms**；② **complex 有 61.3% 的帧仅"等待"就超过 60 fps 预算**（debug 0.4%）；
  ③ 两图 **99% 的帧都 `over_budget=true` 且 `complete=false`**（渐进分页，单页三角形饱和 ≈2.0M），
  complex 整帧工作量单位总量 166,346 vs debug 10,548；④ 与"**放大后仍卡**"一致——整帧工作量由**整套图纸计划**决定、
  **不随可见矩形收缩**，即**缺少视口剔除**的直接后果（backlog P1/P7 的动机）。
- **对既有推断的更正（重要）**：合成 scale-matched 场景下加速档只有 3/19 次 draw，据此曾推断"P1 在加速档≈省 0 次 draw"；
  **真实 complex 场景加速档每页约 3,027 次 draw**（差三个数量级）。因此该推断**必须**先按 draw 类别分解重验，
  **不得**据合成场景结论直接判定 P1 在加速档无收益。
- **证据位置**：`${YACR_EVIDENCE_ROOT:-/tmp/opencode}/yacr-evidence-20261017T024549Z-realhost-probe/`
  （原始 `stderr.log` + `results.tsv`，**仅本地、未上传**）；文档见 `docs/validation-realhost-frame-cost-2026-10-17.md`。
- **NOT RUN（诚实边界）**：**zoom 腿**——仓库只有 `YACR_PROBE_POSTLOAD_PAN[_GESTURES]`，**没有 zoom 驱动**，
  故"放大后仍卡"是**用户观察**＋结构性推理，**未经本轮脚本化量化**；GPU 时间本身（`gpu_ms=None`，无 `TIMESTAMP_QUERY`）；
  任何视觉/精度/vendor/golden 判定；全量门禁/测试矩阵与跨平台目标。单机、单适配器、一对图纸、**prebuilt** 二进制、
  未做重复轮次统计，**不得**外推。
- **口径提醒**：本轮 complex 的 `entityCount=44,591` 与 `docs/validation-render-perf-2026-10-15.md` 的 **122,006
  不是同一口径**，**禁止混用或相除**。
- **已澄清（2026-10-17）**：用户确认那次手测是**从 UI 打开**的 ⇒ 该前提疑点**不成立**。位置参数支持随后已实现
  （见本文件「宿主支持位置参数打开图纸」小节）。
- **再分析（同一批日志，未新跑，仍是测量）**：每帧 CPU 相位中位 complex `cpu 8.364 = plan 2.590 + uniform 1.123 +
  encode 1.131 + **submit 3.412**`（`submit` 是 CPU 最大相位，3,027 draws ⇒ ≈1.13 µs/draw），`wait 19.389`
  ⇒ 帧 ≈ **27.75 ms**；debug `2.072 + 2.895 ≈ 4.97 ms`。**帧时间由 `wait_ms` 主导（≈70%）**。
- **分页重启（实测）**：complex **182 次相机变更全部**使本帧完成度塌缩（中位 **4.6%** ≈ 7,716/166,346），
  356 帧仅 **2** 帧 `complete=true`；debug 中位 86.3%、6 帧完成 ⇒ 交互期间每帧只呈现 **draw-order 前缀首页**
  （`FrameKey` 含相机，`renderer_impl.rs:3856-3890`）。
- **`tier` ≠ `accelerated`（源码核对）**：`FrameStats` 两字段并存（`crates/cad-render-wgpu/src/lib.rs:353-364`）；
  `use_pulled` 要求整场景全接纳（`renderer_impl.rs:4085-4090`）⇒ **分页/超预算帧一律走通用 per-run**，
  这也解释了每页 `instanced`=1,615（按 stream 合并的 run）而非 pulled 路径的 ~15 段量级。
- **产品决定（2026-10-17，用户）**：**接受**运动过程中的帧内容不全 ⇒ **不**做可见性排序页内容
  （取消原拟的"运动中帧"截图判读）；但分页重启的**代价不变**——每帧仍重付 plan/submit，`wait_ms` 仍主导帧时间。

### 修复 wasm32 门禁（2026-10-17，**既有缺陷**，非本轮引入）

- **现象**：文档化门禁 `cargo check --workspace --lib --target wasm32-unknown-unknown` 在
  `crates/cad-engine/src/render/runtime.rs` 报两处 `E0425: cannot find value 'started'`。
  `git blame` 指向 **`7d83572`**（本轮 8 个提交**之前**）⇒ **既有缺陷**；因日常验证按 AGENTS.md
  只验改动范围，这道门禁长期未被跑到，直到全量矩阵才暴露。
- **原因**：`started`（函数起始 `Instant`）仅在 `#[cfg(not(target_arch = "wasm32"))]` 下声明
  （wasm 无单调时钟），而 `apply_ready_updates` 中两处 `total_us:` 初始化**无条件**使用它。
- **修复**（`42c8e37`）：两处 `total_us` 改为在**同一 cfg** 下求值；wasm 分支报 `0`（无时钟可用）。
- **证据**：`cargo check --workspace --lib --target wasm32-unknown-unknown` 转 **PASS**；
  `fmt`/`clippy -p cad-engine --all-targets -D warnings`/host 目标 `check` 均通过。
- **边界**：Android 目标检查（`--target aarch64-linux-android`）**NOT RUN**（需 Android target/NDK）。

### 归档：GPU-vs-CPU 剔除分析（2026-10-17，复核轮；纯文档，无实现/无运行）

- **问题**：视口剔除 / 几何检查 / 局部重绘中，**哪些能移到 GPU**？（由资深复核读渲染器 + wgpu 30.0.1
  源码回答；**本轮未运行任何 cargo/npm/门禁/渲染**。）
- **分析结论**：**提交已近乎免费**——ADR-0007「P3b」A/B 去掉 8,233 draws 仅省 **1.28 ms** CPU
  （complex-test）、≈900 draws 省 **0.52 ms**（debug）⇒ **≈0.16–0.6 µs CPU/draw**；残留每帧 ~5–7 ms 是
  plan/sort/uniform/encode 的**每批记账**，**任何**剔除方案都降不下来。唯一值得 GPU 侧剔除的地方是
  **加速档**——它每帧绘制**全部**几何（complex-test ≈29.45M line 实例 × 6 ≈ **1.77×10^8** 次 VS 调用/帧），
  而 **GPU 帧时间在本仓从未测量**（`tests/render_perf.rs:559`：无 timestamp queries）。
- **对首轮答案的更正（首轮答案部分错误，已记入 backlog）**：
  - **提交不是瓶颈**："减少提交"是**弱杠杆**；间接绘制的提交节省**微弱**（≈0.16–0.6 µs/draw 的 A/B 即证）。
  - **间接绘制措辞更正**：`draw_indirect`/`multi_draw_*` 在 wgpu 30.0.1 **API 齐全**，仅受
    `DownlevelFlags::INDIRECT_EXECUTION` 门控；**WebGL2（GLES 3.0）连单次 draw-indirect 都没有**；
    真正 native-only 的是**未模拟**的 `MULTI_DRAW_INDIRECT_COUNT`（缺它时 wgpu 把 `multi_draw_*` 模拟为
    一串 `draw_indirect` ⇒ **不省提交**）。**仓库两处设备创建点均零 feature 请求**
    （`headless.rs:155-158` / `native_presenter.rs:491-494` 的 `DeviceDescriptor { ..Default::default() }`）
    ⇒ `MULTI_DRAW_INDIRECT_COUNT` 与 `TIMESTAMP_QUERY` **在 P620 上亦未启用**；请求任一**本身即架构决策**。
  - **VS 早退不是提交流程的胜者**：**VS 内 chunk 早退省 0 提交、约 0 片元**（通用档 CPU 跳过严格占优；
    加速档 segment 跨多 chunk、chunk 跳过跳不掉 draw）⇒ **VS 早退省 0 ms CPU（精确，非估算）**，只省
    VS 调用 + 顶点取数。
- **由此产生的 backlog 修订**（`docs/render-optimization-backlog.md`）：新增 **P0**（测量前置，**blocking**：
  GPU 帧时间 + P1 剔除 A/B，用既有 `FrameStats` 相位计时器 `plan_ms`/`uniform_ms`/`encode_ms`/`submit_ms`
  `lib.rs:284-298` 与剔除比）；**P1** 修正陈旧行号、限定范围为**通用档 draw 跳过**（加速档只到残留
  per-batch draw；记录「segment 粒度 × 可剔除性」为**开放决策**）、新增**缩放相关笔画边距 padding**
  （测试加 ~`2·world_per_px`，fence `cull_test_pads_stroke_margin_at_deep_zoom`）；**P3** 重标为
  **bandwidth+fragment 优化，非 GPU compute，且本身不省 CPU**；新增 **P7**（可移植的 per-chunk 可见性
  VS 早退，**条件于 P0**）；**P6** 重述其唯一幸存收益 + 更正间接绘制措辞；文档头新增**绑定原则**
  （能力门控 bonus、永非架构依赖、须 capability 检测 + 基础路径 + 对比基线 + 先 profiling）；新增
  「证据边界与陷阱」节。**ADR-0012 仅追加一行**到「显式延后（NOT NOW）」：GPU 侧剔除保持能力门控的
  渲染器内部 bonus，永非架构依赖 / pick 权威 / 第二套失效真相，且 GPU 帧时间存在前不得构建。
- **NOT RUN 边界（本轮）**：**GPU 帧时间从未测量**（每次"GPU 收益"均为**结构推理，非证据**）；基线
  **单机 / debug profile / Quadro P620 + Vulkan + 驱动 580.178.04**，不外推 release/其它 vendor/更大图纸/
  浏览器；**797/7,627 与 903/8,252 是不同轮次、不同口径，绝不可混算**；`caps_for` 按 backend 类别
  （非运行时探测）报能力，**不得**当设备测量（与 G6 同类）；**WebGL2 真实浏览器运行仍 NOT RUN（G7）**。

## 原生 Wayland 后端（2026-10-16，有界；ADR-0011）

- **决策**：`apps/app-linux` 在保留 X11/XWayland 的同时加入**真实原生 Wayland** 后端（**非**
  XWayland，无 UI 重设计，无 CPU 帧拷贝）。CAD 分区在 Wayland 上是 **app-owned child
  `wl_surface` + `wl_subsurface`**（父级为 GDK toplevel 的 `wl_surface`），渲染器只接收该 child 的
  live raw handles；X11 仍走既有原生 X child 窗口。见 `docs/adr/0011-native-wayland-subsurface.md`。
- **后端策略**：`apps/app-linux/src/backend.rs` 在 GTK 启动前显式决议 `GDK_BACKEND`——`Auto` 优先
  原生 Wayland（需 `WAYLAND_DISPLAY` + `XDG_RUNTIME_DIR`），否则 X11；显式 `x11`/`wayland` 原样保留；
  非法/歧义值硬错；**无静默回退**。选择意图与**实际**后端/协议证据分开记录。
- **私有 C shim**：`apps/app-linux/native/wayland_shim.{c,h}` 用系统 `wayland-client-protocol.h` 包装；
  借用 GDK 的 `wl_display`，**GTK 独占分发**（不 dispatch/roundtrip/flush）；异步 registry + 
  `wl_display_sync` discovery 屏障；缺失/过老 global 显式失败（不永远 Pending）；空 input region；
  整数 buffer scale；父级 commit 语义（只 `queue_draw`，不提交 GDK surface）；释放顺序
  presenter→subsurface→child 且早于 GTK parent unrealize；Ready 健康检查（shim/display 错误 + 当前父
  `wl_surface` 身份）；构造期有界 `catch_unwind` 诚实诊断。
- **有界证据（实际运行）**：scoped `cargo check --all-targets`、`cargo test --lib`（`app-linux` **58/58**、
  `cad-ui-core` **61/61**）、clippy（0 warning）、wasm32 cfg 隔离、`check-architecture.py`、`check-i18n.py`、
  scoped `fmt`；真实原生 Wayland `--probe`（`DISPLAY` 未设、`GDK_BACKEND=wayland`、物理 Vulkan Quadro P620、
  真实 app-owned child `wl_subsurface` 协议身份）；以及 W3 有界运行（CAD/输入/生命周期/几何/视觉/X11，见上）。
  最终 release 二进制 `target/release/yacr` sha256 `7618d0d263498bac75c68eabc18ec6fcaba310a412a06d2467d83552b1393129`。
  完整命令/退出码/日志/边界见 `docs/validation-native-wayland-2026-10-16.md`；源码门禁见
  `.slim/deepwork/native-wayland/gate-w2-attempt2.md` 与 `gate-w3-geometry.md`。
- **W3 有界运行（实际运行）**：原生 CAD（`--open`，4 实体、`current_scene`、P620、`error=None`）、9 个合成 DOM
  输入全 `accepted` 且相机实际改变（**≠ 真实用户指针精度验收**）、resize 遥测；强制 **overlay** 生命周期
  释放/重建/恢复（**不是** toplevel `wl_surface` 替换证明）；分类修复后 gap 由旧 120 error 变为 **0 error/122
  waiting**、gen2 新 child `device_epoch=2`、自然退出 0；**几何修复**（子缓冲 52×99 过小 → 最终 sha
  `7618d0d2…` 下 `DOM 926x266/1166x513` 与 native 一致，margins `34/84`）；**Observer 终态：有界截图视觉
  PASS**（Mutter 同 UID ScreenCast 自身捕获 baseline/resized；灰带消失、无覆盖/间隙/溢出，**无** golden/精度/
  vendor 声明）；**X11 运行证据 PASS**（真实 XID、自然退出 0；X11 PNG **非**原生证据，reader 无法解码故不判视觉）。
  退出码**按 case 精确**：原生 baseline `capA` 捕获后 SIGTERM **143**，resized `capB` 与 X11 **自然 0**。
- **最终 Oracle 终判（W3 attempt3）：ACCEPT-WITH-LIMITATIONS（无 blocker）** —— **有界本地验收**
  （GNOME/Mutter + Quadro P620 + `scale=1`），**不是**完整产品/跨平台/vendor/golden/性能验收；判词见
  `.slim/deepwork/native-wayland/gate-w3-final.md`。生命周期运行用的 `6894e423` **未**在最终几何二进制
  `7618d0d2` 上复跑（admission 在几何轮未改动）；X11 视觉**未**由 observer 判读（仅运行证据 PASS）。
- **NOT RUN / 未支持（不得夸大）**：**DPR>1/分数缩放**（只测 `scale=1`）、
  GTK **toplevel** 替换、GPU 设备丢失、vendor/adapter 广度、几何/像素精度、性能/60fps、跨平台、打包。
  **首次 scoped 编译失败（exit 101）与源码 REJECT 保留为负面证据**。
- **边界不变**：`cad-host`/`cad-ui-core` 工具包与 GPU 无关；renderer-only wgpu、importer-only
  acadrust 不变；C shim 与 GDK Wayland 访问只存在于 `app-linux`。非 Linux 占位与退役脚本未变。

## Slint → React/Tauri 桌面迁移（2026-10-16，源码级；ADR-0010）

- **决策**：桌面 UI 正式由 Slint 迁移为 **React（Vite）chrome in Tauri 2 WebView + 原生 GTK/wgpu
  CAD 分区**；见 `docs/adr/0010-react-linux-host-replaces-slint.md`（取代 ADR 0006 的实验定位，
  使 ADR 0002/0003 的 Slint 桌面基线作废）。本文件以下历史小节**不改写**。
- **宿主**：`apps/app-linux`（Linux 优先真实 GUI，bin `yacr`，Tauri id `dev.yacr.app`）；非 Linux
  目标编译为空库、二进制返回显式 NOT IMPLEMENTED。配置目录 `~/.config/yacr/`（由 `yacr-react`
  迁移，**不自动迁移**）。
- **共享层**：新增 `cad-ui-core`（protocol/layout/session/shell/i18n），位于 UI 无关的
  `cad-app`/`cad-host`/`cad-engine`/`cad-platform` 契约之上；宿主显式 `state.shell.*` + 直接 `use
  cad_ui_core::…`，无过渡 re-export。
- **退役**：删除 `cad-ui-slint`（含重复 i18n 副本）+ workspace 的 `slint`/`slint-build`；
  `Cargo.lock` 按可达性剪枝 845 → 622 包；旧 Slint 验证/离屏/真实 GPU 测试与 web shell/Flatpak 退役；
  6 个公共构建/打包/部署脚本改为非零 NOT IMPLEMENTED；8 个 CI job `if: false` 硬禁用；**Linux release
  暂时不可用，等待替代打包验收**。
- **守卫**：`check-architecture.py` 全 workspace 禁 `slint`/`slint-build`/`iced`（含 build/dev/target
  表与 workspace 继承别名）；`check-i18n.py` 权威目录改为 `cad-ui-core/i18n` 并新增 React CJK 扫描；
  新契约 `scripts/test-ui-migration-guards.py`（18 项负向/正向控制，已接入 CI 契约步骤）。
- **支持范围（诚实）**：仅 **OPEN / FIT** 接线；其余控件显式禁用并标注原因；**不新增投机 IPC**；
  **不是完整 CAD 产品**。
- **证据（分级诚实）**：**P6 最终验证已执行（有界、分区）**——前端 `npm ci`/`npm test` 43/43/`npm run
  build`、`cargo fmt --check`、clippy、四个 Python 门禁与十个 Python 契约（含 18 项迁移守卫）、
  `--locked` metadata/clippy/workspace all-targets check/wasm lib check/workspace 测试（1825 passed/0 failed/
  19 ignored）/桩 lib 测试，以及真实硬件 GPU（Quadro P620/Vulkan，真实 GTK XWayland）的 open+渲染 smoke、
  原生分区生命周期（forced unrealize→新 XID+generation 2、自然 exit 0）、外部 WM minimize→restore 与真实
  resize；完整命令/退出码/日志/限制见 `docs/validation-react-linux-migration-2026-10-16.md`（及
  `.slim/deepwork/replace-slint-ui/p6-*.md`）。解析器一度被手工锁阻断，已按**批准的最小离线同步**修复
  （仅 `Cargo.lock` 622→595，纯删 27 孤儿，无版本变动）。**Gate P6 attempt2：ACCEPT-WITH-FIXES（文档限定
  已应用）**；observer 终判所观察截图**无重大视觉缺陷**；**有界本地 Linux 迁移验收通过**，**不是**完整 CAD
  产品/跨平台/vendor/golden/性能验收。应用侧 `unminimize()`+强制 unrealize 序列**未解决**。
  **早期阶段在其当时分支/代码树上另有有界执行**：P1 lib 契约（59+27→86）与 check/fmt；P2a host/wasm/
  aarch64 编译检查；P4 前端构建 + 43/43 测试。**P2b/P3b/P5 为纯源码实施/评审**。旧 `#[ignore]` Slint
  真实 GPU 套件与其 runner 已退役，其结论不得复用。

## 线/弧抗锯齿统一轮（2026-10-15，`309e1a2` / ADR-0009）

- 用户批准「1+3」：把**所有线类**几何改为与 SDF 弧（ADR-0008）**同语义**的抗锯齿笔画——
  `coverage = saturate(1 − d/w)`、`w = world_per_px`、**1.0 px 线性墨量**、相位相关峰值。覆盖：
  实例化线流、legacy/奇数顶点线、透明线、**`MeshEdges` 叠加（此前是文档化的静默 no-op，本轮关闭
  backlog G3）**、网格叠加线框、弦展开 conic；拾取 shader 光栅化同一 SDF，字面 ±0.5 px。决策与
  语义见 `docs/adr/0009-antialiased-line-strokes.md`。
- **两个真实 bug（实现中实测）**：① 水平线段帧退化——`cross(dir, ref)` 对水平线段给出屏幕法线
  （z）方向，正交俯视下四边形零面积；改为绘制平面内垂直 `vec2(-dir.y, dir.x)`。② 加速档类绘制
  顺序——合并 mesh 类**先于**线类（此前相反），使 AA 线边缘在穿越处按 mesh 之上混合，两档帧逐字节
  一致（`accelerated_overlapping_ties_match_universal_pixels`）。
- **Fence 重划（显式、诚实）**：「legacy vs instanced 逐字节一致」→ **转换路径等价**；**与硬抗锯齿
  线的逐像素一致声明作废**（已授权的语义像素变化）；大坐标精度、确定性与相机移动零重传保留；新增
  跨类线性墨量、逐相位峰值包络、折线连续性、接头 alpha 并集、确定性 fences。**弧视觉验收的「SDF ≥
  弦展开 AA 灰阶数」排序也作废**：弦线现在同样是 AA，灰阶计数是 sRGB 启发式而非可靠平滑度度量；
  改为断言两帧都确有 AA（各 ≥32 中间灰阶）。该 fence 在本轮由绿转红（父提交 `b449337` 通过、
  `309e1a2` 失败），已修正。
- 线性墨量一致（同帧，真实 GPU P620/Vulkan，`unified_line_ink_parity_across_classes`）：

  | 类 | 线性墨量 (px) |
  |---|---|
  | SDF 弧 | 1.000 |
  | 实例化直线 | 1.000 |
  | `MeshEdges` | 1.000 |
  | 透明线（÷α） | 0.994 |
  | 弦展开 conic | 1.004 |

  逐相位峰值：mean 0.748、[0.497, 1.000]（0.5–1.0 随相位是设计语义，非缺陷）。
- 视觉证据目录：`/tmp/opencode/yacr-sdf-arcs-v3/`（弧 before 弦 / after SDF）、`.../v4/`（线统一
  **初版**捕获，**保留不覆盖**、标注缺陷）、`.../v5/`（**修正版**线 before/after）。`v4`/`v5` 各含
  `METRICS.md`（测量契约：ink = **线性覆盖积分**；sRGB 编码积分虚增软笔画 ≈1.4×，ADR-0008 §2 已
  关闭；峰值随相位 0.5–1.0 为设计）。
- **本 bookkeeping lane 修正的两个 harness 缺陷**：① 视觉验收 harness 对同一 delta 两次 `upload`
  ——`Renderer::upload` 是增量暂存、不替换常驻场景，故圆被重复混合；② 亚像素 ramp 的
  `coverage <= 0 → discard` 会丢弃垂直相位落在两像素行之间的线段（v4 `before` 丢失两条水平线）。
  修正：**一次 upload、两帧只改 ramp uniform**，场景改按屏幕像素定义（保证相位对齐），并在测试内
  断言两帧 core-ink 掩码连通分量相同、异或 0（v5 实测分量 4/4、异或 0、union 503 px；线区部分覆盖
  像素 hard 0 vs unified 64）。
- **NOT RUN / 边界（诚实）**：浏览器/WebGL2/WebGPU、Android 设备、vendor DWG、视觉 golden **未验**；
  `lineweight` 仍「携带不绘制」，**未**光栅化线宽（backlog）；全量 workspace 门禁未跑（用户政策）。
  本轮仅测试/文档/证据改动，**未 commit**（由 orchestrator 提交）。

## 渲染性能 “entity = data” 轮（2026-10-15）

- 用户导向渲染架构 **“entity = data”** 并授权相应契约变更。本轮修三个独立问题：①
  `DrawingDatabase::bounds()` 每次视口变化重算（overlay fingerprint 含 `viewport_bounds`，
  allocate+sort+INSERT 再展开）导致 pan/zoom CPU 随实体数**超线性**（complex-test.dwg
  44,591 实体 526 ms/步，其中 `bounds()` 单独 521 ms）；② 逐批 `queue.write_buffer` 约占
  整帧 CPU **55%**；③ 相机-only 变化失效已发布场景 → base-only/全量交替 + overlay 抖动；
  另修 React host resize 后 zoom 不以光标为锚（canvas metrics 创建时冻结）。
- 修复：`crates/cad-db` revision-keyed bounds 缓存 + order-free 累加 + memoized block
  bounds（每步 526 → 0.007 ms）；`crates/cad-render-wgpu` 共享 `UniformArena` dynamic offsets
  （每帧 cpu 35.7→11.7 ms @10.7k、158.9→47.1 ms @44.5k，帧组成与 HEAD worktree 逐字节一致；
  任务记录的 55,296/55,296 bytes 在证据目录中未找到 → NOT VERIFIED）；`crates/cad-engine`
  `input_revision`（场景身份）/`content_revision`
  （prep token）拆分（pan/zoom 保持 `current_ready_scene` Some、`overlay_suffix_commits`=0）；
  `apps/app-react-linux` tick 内同步实时 metrics（相机数学本身正确，`camera.rs:609-664`）。
- draw-call 折叠：line/segment instanced 流（`LineInstance` 56 B、per-chunk f64 origin
  `MAX_STREAM_RADIUS` 8192 mm、`select()` 精确端点）+ mesh flat-merge（44 B 步长、逐顶点
  `entity_flags`/colour、`mesh_merged.wgsl`）+ 边 overlay 作为 LineInstance。真实 DWG + 字体：
  debug-default-fonts 10,474→**797** draws（663 line + 134 mesh）、每帧 cpu 17.96→**1.88 ms**；
  complex-test 122,006→**7,627** draws、46.74→**5.91 ms**；pan/zoom 零流/chunk 重传。绘制顺序
  语义经 fences（merged-vs-legacy 逐字节一致、overlapping-tie、3D perspective）保持。两轮独立
  oracle review gate 的 7 项 must-fix 全部修复。
- 脚本与浏览器检查改为**默认真实 GPU**（软件 = 显式 `YACR_SOFTWARE_GPU=1`/`--software` 并标注
  SOFTWARE；CI web-smoke 显式 opt-in）。AGENTS.md「验证范围」记用户 2026-10-15 提效决定：日常
  只验证改动范围，全量门禁/全量测试仅在用户要求或发版前。
- 证据目录 `/tmp/opencode/yacr-perf/`：`1791991784`、`1791992830`、`1791993747-renderer`、
  `1791997393-step2-phases`、`instanced-slice1`、`p2b-meshmerge-realscene`、`1792005717-realscene`、
  `1791998671-merge-ratio`。汇总表与适配器证明见 `docs/validation-render-perf-2026-10-15.md`；
  决策见 `docs/adr/0007-entity-as-data-renderer.md`；绘制顺序见 `docs/render-order.md`。
- **NOT RUN / 左失败**：全量 workspace 门禁 NOT RUN（用户政策）；WebGL2/Android/浏览器视觉验收
  NOT RUN。pre-existing 失败按用户指示保持失败（`/tmp/opencode/yacr-verify/REPORT.md` 分类为
  HEAD 既有）：5 项浏览器检查（mobile label i18n staleness、ribbon viewport、drawing LINE、
  drawing-safety capture、overlay selection flake）与 `app-linux` 的
  `plot_export`/`save_export`/`verify_ui*`。本轮仅文档/证据，未 commit。
- 本轮之后待优化项（视锥剔除/LOD/SDF 弧/RenderBundle/加速层 + G 系列结转项）清单见
  `docs/render-optimization-backlog.md`。

## 渲染性能 “entity = data” 轮 — 续：GPU 拾取 / 选择标志 / 加速档 / SDF 弧（2026-10-15）

- 承接上一节（上半段）。下半段在同一轮内把渲染架构继续推进到 **GPU id 回读拾取 → 标志机 →
  能力门控加速档 → SDF 解析弧**，全部已提交：上半段 `3bb3446`…`0d2034c`（含端点 9 个提交）、
  下半段 `52ab9d5`…`1fc395e`（9 个提交）。本文档条目本身为文档更新，**未提交**（本次改动仅限
  `docs/handoff.md` 与 backlog/ADR-0007 的状态对齐，无代码改动）。
- **P3a-1 GPU id 回读拾取**（`52ab9d5`，修复 `0f193dc`）：`Renderer::pick_at(x,y)` 按需把同一
  可见几何渲进**独立 id target**（自有深度缓冲、run **不合并**，片段不会继承邻批 key），回读
  1 texel。GPU 32 位 pick key → CPU `PickTable` 保存**完整 u128** 身份，解析按构造精确；低 32 位
  截断**检测并上报**（`pick_collisions`），多源批次给出**全部**候选，**绝不**静默取
  `sources.first()`。`R32Uint` 主格式 + 已测 packed-RGBA8 回退；标志纹理默认**关闭**（像素相同）。
  uniform 80→96 B（`key_flags`），槽步长不变。真实 DWG parity（渲染器 id pass vs CPU
  `cad_spatial`）：**CPU 可复现 GPU 实体 1.000、不可解释冲突 0**；精确一致率偏低（0.579 / 0.103）
  已定位为共面/等深并列的排序差异（harness 不断言该率）。CPU 拾取仍是**主路径/回退**，mesh 面
  仍走 CPU 细化。详见 `docs/picking-gpu-id-pass.md`。
- **P3a-2 实体级选择视觉走渲染器标志**（`933e45d`，修复 `a6192a6`）：**整实体高亮 overlay batch
  退役**；选择/悬停/隐藏经标志表（`Renderer::set_entity_flags` 按 `PickTable` 的 source→key 解析、
  只 diff 上传变化的行）。**子元素（网格面）高亮与工具预览**保留一次小 overlay（有据，用户授权）。
  生产会话（`cad-engine`）走 **flags-only 快路径**（overlay `Arc::ptr_eq` 身份判据——review 抓到的
  ABA 修复）；图像标志生命周期修复（base/图像重传会重编号 key，标志按新键重解析）；标志变化丢弃
  已完成的 progressive 帧，**选择立即着色**，不必等相机移动。限制：多源合并 batch 按 **batch
  粒度**着色；hover/hidden **生产者未接线**（`cad-host` 传空）；id pass **忽略 `FLAG_HIDDEN`**
  （hidden 切片落地前**必须先修**）。
- **P3b 能力门控加速档**（`dc1aabb`，修复 `b3a9453`）：已转换的不透明 line/mesh 类经 VS 只读
  storage（vertex pulling）折叠为**每段一次 draw**；档位由**设备自身 limits** 决定（**从不**向
  宿主设备请求 feature），WebGL2 **自动降级并带标签**（`render_tier()` / `FrameStats::tier` /
  `FrameStats::accelerated`）。精度契约不变（per-chunk 矩阵**逐位相同**、f32 运算序列一致）。
  真实 DWG + 字体（P620）：debug-default-fonts **3** draws / **1.85 ms** 帧 cpu，complex-test
  **19** draws / **6.72 ms**（legacy 分别为 10,474 / 122,006 draws；相对通用 per-run 少 301×/434×）。
  review 抓到的修复：id pass 多 stream **`base + offset`** 重定基（曾静默读错实例）、line 回退
  改为**真实数据** + oversize 守卫、tier 门限改为 **≥3** 个 VS storage binding、字节遥测重复计数。
- **SDF 圆/弧**（`1fc395e`，`docs/adr/0008-sdf-conic-rendering.md`）：圆/弧改为抗锯齿 **SDF 实例**
  （`ArcInstance` 72 B，共轭半直径参数化；**三角形滤波覆盖峰值 1.0**；ramp 宽度用**解析
  world-per-px**——`fwidth` 单独在网格对齐点塌缩；**强制包围四边形 `discard`**）。发射门把
  eligible conic（不透明、连续线型、slice-1 圆形）翻到 SDF，**虚线/椭圆/退化仍计数走弦**；资格在
  **变换后** conic 上判定，故 INSERT 造出的椭圆保持诚实。`pick_arc` 光栅化**同一 SDF**，字面
  ±0.5 px。**椭圆 SDF = slice 2（未做）**。视觉证据 `/tmp/opencode/yacr-sdf-arcs-v3/`（混合弧+线
  before 弦 / after SDF；连续性 + 满不透明 + **线性墨量对齐**已 fence；峰值随相位 0.5–1.0 是
  正确 AA——**峰值与墨量不可兼得**，直线硬别名 1 px、弧按墨量对齐；sRGB 通道直接积分会把软边
  笔画高估 ≈√2，是取证伪影，非宽度缺陷，详见 ADR 0008 §2/§实施记录 4）。
- **Conic 中性载体**（`226651c`）：Circle/Arc/Ellipse 以 `DisplayPrimitive::Conic`（f64 世界空间、
  仿射保持参数角）上行，下游消费者仍**全部弦展开**（输出与旧路径逐字节一致，已 fence）；SDF 资格
  记入 `ConicStats`。**渲染/性能 backlog**（`844ab14`）为独立历史项。
- **复核门（本半段）**：4 个**独立** code-review gate（id pass / 标志切换 / 加速档 / SDF），findings
  依次为 **1 blocker + 4 should-fix**、**2 blockers + 4**、**2 blockers + 4**、**1 blocker + 4**，
  **全部**修复，能在不伪造证据的前提下都补了**负向测试栅栏**。另有**一次图像取证**抓到 code review
  曾合理化的 **50% 覆盖缺陷**（弧断成半权重/虚线，即 `fwidth` 塌缩 + 0.5 峰值）。
- **NOT RUN / 边界（诚实）**：**全量 workspace 门禁整场未跑**（用户政策：只验证改动范围；全量门禁
  仅在用户要求或发版前）；**flags + SDF 弧的 WebGL2 真实浏览器验证未做**（「WebGL2 安全」目前由
  离线 GLSL ES 3.00 back-translation 测试覆盖，非真实浏览器）；**椭圆 SDF（slice 2）、`PulledArc`、
  虚线 SDF 弧、线宽光栅化**未做；**hidden 切片**（id pass 先读 `FLAG_HIDDEN`）、hover/hidden
  生产者未接线；**像素 LOD / 视锥剔除 / 帧缓冲复用**等见 `docs/render-optimization-backlog.md`。
  pre-existing 失败按用户指示**保持失败**（沿用上半段条目所列，不重复）：5 项浏览器检查、
  `app-linux` 的 `plot_export`/`save_export`/`verify_ui*`、`cad-engine`
  `pending_overlay_upload_never_labels_stale_suffix_current`。
- **状态对齐（本轮已改）**：`docs/render-optimization-backlog.md` 的 **P4（SDF）→ 已实现**（见
  ADR-0008；椭圆/虚线/宽度为后续）、**加速层 A → 已实现（P3b）**、**G7 的 parity 复跑 → 已重跑**
  （P3a-1，`docs/picking-gpu-id-pass.md` §7）；`docs/adr/0007-entity-as-data-renderer.md` 的
  deferred 列表已交叉引用 ADR-0008，并把 **id-readback picking 由「无通路」更新为 P3a-1 已实现**
  （指向 picking 文档）。

## cad-engine P4 codegate 与最终证据状态

- Oracle P3、P4 App/engine/Host/Web codegate 均 accept。最终 sweep `sh_137d0698a001J14BRMEOwCRSk6` exit 0：workspace all-targets/tests，App343/engine73/Host83/Android30/Web23/UI132+37 integration/Linux12 passed + 9 ignored，workspace WASM lib、Android aarch64 UI/app local-Skia、Clippy（既有 exclusions）、7 packages fmt、architecture/fixture/workflow/i18n306 与 diff check。
- 后续 Host 专项新增 3 tests 到 86、Host clippy 通过；Linux 新 ignored test 的 Arc 解构机械修复后 clippy/fmt 通过。最终差异门禁 `sh_137df9c5b00120goh09biLHuUM` exit 0：workspace all-targets check、Linux lib 12 passed/10 ignored、7 packages fmt、architecture/fixture/workflow/i18n306、diff check。该末次 shell 复验了生产源码不变情况下的当前差异；不要把整个 dirty workspace 归属本轮。
- P3 六用例+async cursor 与 P4 新 API 窗口用例的最终硬件报告为 `.slim/deepwork/cad-engine-decoupling/p4-real-gpu.md`，artifacts `/tmp/opencode/yacr-p4-final-20261014-004223/`（实际 report 时间标记 20261014-004223）：六用例脚本 6/6、async cursor 1/1、新 API/events 用例 1/1，共 8 terminal PASS；Wayland/winit + P620 Vulkan，CPU load stage 无 GPU context。新用例覆盖 controlled Commit、selection/highlight、sequence/origin/request ID/transaction/共享 `Arc<ChangeSet>`、Undo/Redo、idle；不覆盖 vendor/golden/browser/Android。
- P4 App `cad-app` 原子 Commit/selection validation/traced command/shared `Arc<ChangeSet>` Oracle accept，343 tests/clippy；engine protocol/queue/events Oracle accept，focused 70 tests + clippy，最终 sweep engine73。UI shell `sh_137adcff0001KP0ImwLEq3shNv` UI132 lib +37 integration/clippy；Host post-commit render failure/Undo、external failure real completion ID、reentry Busy 的新增 3 tests 后 Host86/clippy。Host/Web codegate accepted；Android wrapper included in final sweep/tests. 无自动订阅或插件 UI，`.slint`/catalog 未改。P4 不宣称 plugin loader、React、双 surface、跨平台产品完整交付。
- P4 最终差异门禁已 exit 0；实际范围见 shell 上述记录。仍未验收 browser RAF/Reflect/GC/WebGPU、Android device、vendor DWG、视觉 golden；queue pressure/lag/reserve/coalesce/32 subscribers 是合成合同，browser ring 不跨页面持久化。P2 paper-only/grid 初轮失败和 P4 reject 修复历史均保留。全仓陌生 `debug_fixture.rs`/`debug-block.dwg` 未碰；本任务仅文档，未 commit。

## cad-engine P2 真实 GPU 分轮证据 / P3 接续（2026-10-14，已更新）

- 首轮硬件六用例脚本在 NVIDIA Quadro P620 + Vulkan 真实 winit 窗口跑出 1 PASS / 5 FAIL；其中 1–4 是纸空间-only
  `a4-layout.dwg` 与模型空间假设不匹配，改用授权 synthetic four-lines DWG 后单独复跑 4/4 PASS；case 5 的 MOVE PASS
  来自 Host bounds 修复前。case 6 是真实 Host Y 倒置 bounds bug；修后只重跑 case 6，全流程 PASS、四边差分
  `[781,2520,851,278]` 且 controller/render 尺寸一致。**不是同一轮六例全绿**，不得把先前失败或未导出
  `YACR_TEST_DWG` 的操作者失误记录删改，也撤回“sampling fragile”归因。完整分轮及硬件 PID 证据见验证记录。
- P2 最新 workspace check、host 34、grid 两合同、clippy 与 fmt 通过；WASM/Android compile 是 Host bounds 修复前证据，
  修后未复跑。无 golden/vendor/Android/Web 运行验收；不得扩展成视觉或跨平台结论。
- P3 queue contract 已冻结；后续 P3 实现已通过 Oracle 最终 codegate 和指定 compile/check/test sweep，详见本文件顶部新状态与验证记录。
  P3 六用例及独立异步 `MoveCursor` 真实窗口用例均已通过，覆盖边界详见本文顶部与验证记录。P4 App/engine/Host/Web codegate 与指定 wrappers 验证已完成；最终 sweep 与末次差异门禁通过，详见顶部。当前章节不覆盖下方旧暂停/P1 历史。

## cad-engine P2-B 当前状态与复核入口（2026-10-13）

- Host/Slint/apps 的 P2 source remediations 已齐，核心已去 UI 依赖，维持单 session、单 dispatch、config authority。
  Oracle attempt 4 接受 P2 架构/源码 codegate；最终串行 compile/check/test sweep 也全部 exit 0。attempt 1/2 findings
  已闭环；attempt 3 的正交 3D primary-drag mode 与 UI summary 对象计数已修复。此结论不包含场景运行或 E2E。
- 此处为 2026-10-13 历史快照：当时 P3 queue writer 尚未编译/集成；后续状态见本文顶部 2026-10-14 更新。源码 lane 未改依赖/catalog/fixture/
  workflow，architecture/fixture/workflow/i18n 四门禁此前通过，仍由主协调器 final rescan。保留既有暂停与 P1 历史，
  不覆盖失败记录。
- Skia Android tarball 已在 `/tmp/opencode/yacr-skia-cache/` 完成 gzip/tar/key/tag/aarch64 与 SHA-256 校验；
  未做签名验证。最终串行验证以该本地 tarball 完成 Android aarch64 UI/app compile check（18.46s），不代表设备运行。
- 终态串行记录 `sh_135e64476001QjMcVRgdJvRBJB` exit 0：workspace all-targets、host 33、UI 129 lib + 36
  integration、WASM lib、local-Skia Android aarch64 UI/app check（18.46s）及 Clippy（既有排除项）。详情与此前 shell
  记录索引、command/NOT RUN 证据见 `docs/validation-engine-decoupling.md`。GPU/窗口/DWG/Android 设备/Web browser E2E
  未运行；全仓 fmt baseline 不触碰。

## cad-engine P2-B 恢复与 P4 设计续做入口（2026-10-13）

- P2-B writers 当前活跃，HostCore/UI 尚未整合。清理 206.8 GiB `target` 后 workspace 全目标冷编译通过；
  曾有 HostCore HTTP 403 partial，正由同会话恢复，不得写成已完整实现或验证通过。最小
  `SetMeasurementKind`/`BeginPan`/`ClearCursor` 与 `same_session` 已落地并 focused 验证，细节见
  `.slim/deepwork/cad-engine-decoupling/p2-interface-addendum.md`。
- 单一命令 + 受控 API 双入口和事件基础的原始设计契约见
  `.slim/deepwork/cad-engine-decoupling/api-events-plan.md`；P4 App/engine/UI/Host/Android/Web 源码接线与指定 workspace 集成门禁已完成，产品级 browser/device/vendor/plugin 验收范围仍有限，详见顶部当前状态。
  写入与命令共享事务/队列，写 API 不能越序；完整契约见规范 §4.11。
- P4 核心约束：expected-revision 原子 batch 限 Insert(Line/Circle/Arc)/Translate/Delete，由 engine 分配 ID，
  校验权限与有效 layer/space，并走事务/history/权威 ChangeSet；禁止裸 DB mut。选择只解析当前文档顶层
  ObjectId，块实例必须给明确有效 `SelectionRef` 路径。命令/API 共 queue，pending 时同步 `Busy`。
- 生命周期事件反映真实请求/命令与 ChangeSet/selection/tool 状态；仅显式成功 Move 意图生成 Moved，失败/取消
  零实体变化，noop 不伪造事务。有界 poll 订阅显式报告 lag 并要求重取 snapshot/watermark；无锁内 callback，
  事件不是第二数据库。P4 是受控 API/events，不是 plugin loader；React/loader/sandbox 不在本轮。
- 本轮没有完成 P2-B 集成或实现 P4；React、loader、sandbox 不在当前范围。保留单 presenter 与 renderer-only
  wgpu 边界；真实 GPU 和 Android native check 本轮未运行，不得从冷编译推导运行/render 验收。

## cad-engine P2 暂停检查点（2026-10-13）

- P1 `df6df85` 已完整保存。P2-A 的 `Drawing` capture/selection snapshot/tool generation、Select/SelectAll
  版本已落地，app all-targets check 与 331 项 synthetic tests 通过；`EngineClient`/`HostUpdates` schema
   已落地，但尚无 HostCore 实现或 UI 接线。按用户要求本轮已暂停，P2 源码与文档改动保留在工作树、尚未 commit。
- `Measure(None)` 已恢复为 `Distance`；新增 Area 后 `None` 仍为 `Distance` 的合同测试通过，
  `confirm_draw` 经 `HostController::execute`。Engine 31 项 synthetic tests、4 项静态门禁、owned fmt 与
   `git diff --check` 通过。最后兼容修补后 workspace all-targets、WASM lib、Clippy 复核通过；
   `cargo fmt -p cad-app -p cad-engine -- --check` 通过，收尾纠正了局部 formatter 版本造成的格式偏移。
- 全仓 fmt 仍被基线未跟踪文件 `crates/cad-import-acadrust/examples/debug_fixture.rs` 格式问题阻塞，
  不要修改该文件。真实 GPU 验收和 Android native check 未运行；虽有显示硬件，本轮没有 render 验收。
- 续做入口：P2-B 移除 HostCore→UI 依赖，实现 EngineClient + 语义 updates/config 单 authority；接入
  Slint EngineAdapter 与已有 session bridge 安装入口；apps 持有 UI handle/adapter 并迁移 HostResources。
  旧 Slint draw 路径仍持有 `DrawTool`、尚未切换，禁止让旧路径与新 app 状态双驱动。随后再做 P3 有界队列
  与 event-driven 唤醒。不得据此宣称整个架构已完成。

## cad-engine 解耦 P1 检查点（2026-10-13，P1 审查通过；整体实施中）

- `cad-engine` 当前 manifest 直接依赖 `cad-app`、`cad-db`、`cad-domain`、`cad-render-wgpu`、
  `cad-representation`；均为应用/数据/领域/renderer/显示表示核心契约，不是 UI 依赖。runtime/render
  提交仍封装在 renderer；Slint 仍是单 surface 协调者与 presenter，P1 host 暂保留 UI 过渡依赖。
- Oracle gate 1 attempt 3 接受，先前会话一致性阻塞已闭环。P1 focused engine + UI all-targets、engine
  29 项 synthetic tests、app interaction 4 项 synthetic tests、post-fix workspace all-targets、WASM lib
  check 与 4 项静态门禁已通过；均非 GPU/真机运行验收。整体仍实施中：P2 host/tool 与 P3 queue 尚未实现。
- Clippy 初次发现 4 个机械错误，修补后 workspace Clippy（按既有排除项）通过；修补后 WASM
  lib 编译与 29 项引擎合成测试也通过。fmt 全仓仍受基线未跟踪文件
  `crates/cad-import-acadrust/examples/debug_fixture.rs` 格式问题阻塞；不要修改该文件。
- 未运行真实 GPU 验收或 Android native check。
- 规范、架构与阶段协议见 `CAD_IMPLEMENTATION_SPEC.md` §4.11、`docs/architecture.md` 与
  `docs/adr/0005-cad-engine-decoupling.md`。依赖门禁当前只约束 engine/app/UI 方向；host 无 UI 断言待 P2。

## 默认 workspace host 编译门禁（2026-10-12）

- 默认 `cargo check` 现统一为 `cargo check --workspace --all-targets --locked`，覆盖所有
  workspace 成员的 host target；clippy 排除项保持不变。
- host 包含 `app-android` / `app-web` 不代表 Android/WASM 原生目标编译或平台运行验收；
  独立目标门禁保持原样。此前 validation 与逐轮实现记录中的命令均为当时历史证据，不改写。
- 本轮 `python3 scripts/check-workflows.py`、`python3 scripts/test-linux-workflow.py`（6 项）、
  `python3 scripts/test-web-deploy-workflow.py`（4 项）、默认命令排除项变异契约（1 项）、
  `git diff --check` 与 `cargo check --workspace --all-targets --locked` 均通过。Cargo check
  仅为 host debug 编译，不是 Android/WASM 目标或运行验收。

## SFNT 填充安全与 mesh edge overlay 阻塞修订（2026-10-12）

- 新增 `DisplayPrimitive::MeshWithoutEdgeOverlay` 契约。Scene 为其保留三角索引但不构造边 overlay
  几何；一般 `Mesh` 维持原有边 overlay。`Renderer::upload` 只在 batch 携带 edges 时生成排序/去重
  edge indices，`draw_batch` 对无 edge index 的 mesh 只提交填充 draw；上传 budget 的 edge-index
  字节也按 overlay 可选计算。新增 scene topology/upload-size、renderer 上传索引和无 GPU draw plan
  测试。
- `FontEngine::outline` 恢复纯轮廓输出：`ShapedText.outlines` 保留 SFNT 与 SHX 的变换后轮廓，
  SFNT 成功填充的 `polylines` 仍为空，不产生常规 wireframe。排版/变换共用 finalize 路径。
- `fill_rings` 本身仍是 HATCH 的 even-odd 契约，未修改。SFNT 调用前验证简单边界、组件间无边界
  相交/重叠、嵌套层绕向交替；不满足时跳过 Mesh，保留边线，附 `text.sfnt_fill_partial` 并使结果
  Partial。缺字输出 `text.glyph_missing`。空白判定按解析后 run 的可见字符，契约覆盖空格、`\P`、
  `\~`、`{\H2; }`。
- 本轮定向测试通过：`cargo test -p cad-representation --lib --locked`（177 passed）、
  `cargo test -p cad-scene --lib --locked`（55 passed）、`cargo test -p cad-render-wgpu --lib --locked`
  （44 passed）。`cargo check -p cad-app -p cad-cli-tools --locked`、
  `cargo test -p app-linux --lib real_gpu_cjk_sfnt_text_fill_acceptance --locked --no-run`、
  `YACR_TEST_SHX=$PWD/apps/app-android/assets/fonts/txt.shx cargo test -p cad-representation --lib shx_text_provider_keeps_outline_lines_when_font_is_available --locked`
  （1 passed）、`cargo fmt --all -- --check`、`git diff --check` 均通过。
  修正 MTEXT outline 契约后的可选字体测试通过：
  `YACR_TEST_FONT=$PWD/fonts/osifont.ttf cargo test -p cad-representation --lib shapes_mtext_runs_and_reports_approximations_when_font_provided --locked`
  （1 passed）。`cargo clippy --workspace --exclude app-android --exclude app-web --all-targets --locked -- -D warnings`、
  `cargo check --workspace --exclude app-android --exclude app-web --all-targets --locked` 与
  `cargo check --workspace --lib --target wasm32-unknown-unknown --locked` 均通过。
- 最终真 GPU 中文专项通过：证据目录 `/tmp/opencode/yacr-cjk-final-20261012-114326/`，Quadro P620 + Vulkan，
  使用 Noto TTC 和合成 DXF；observer 检查截图确认中文字形实心并保留字腔、无可见三角边。此为合成 DXF
  的 Linux 真 GPU 视觉证据，不代表真实 DWG、其他字体、vendor 或其他平台兼容性。
- 最后执行 `cargo fmt --all -- --check` 与 `git diff --check` 均通过。

## Linux CAD SFNT 中文空心填充实现（2026-10-12）

- `cad-representation::FontEngine::shape` 现在把每个 SFNT glyph 的全部轮廓作为一组交给
  `cad-geometry::fill_rings`，按 even-odd 生成 `Mesh`；成功的 SFNT glyph 不再作为闭合轮廓
  `Lines` 绘制，因此不会重复描边或暴露三角内部边。SHX 仍然是线稿；变换、按 glyph fallback、
  MTEXT run 排版、对齐/旋转与既有字体失效缓存语义不变。合法空白不伪造 Missing。填充失败
  保留边界线、`Partial` 和 `text.sfnt_fill_partial` 诊断。
- 合成契约：授权 fixture `fonts/osifont.ttf` 的 `O` 有 Mesh、中心孔不被三角覆盖、无常规轮廓
  线；provider 输出与带 fallback 的 MTEXT 混合 run 均有覆盖；填充错误明确测试；本机
  `apps/app-android/assets/fonts/txt.shx` 只作未提交测试输入，focused SHX provider 用例确认仍输出
  Lines。**本轮合成测试通过不代表 CJK 字体/真实 DWG 兼容或视觉验收。**
- 新增 ignored Linux 专项 `host::validation::tests::real_gpu_cjk_sfnt_text_fill_acceptance`：生成只含
  “中文空心字”的临时 DXF，用指定字体走 Linux 真窗口、真实 Vulkan、CadView 并保存截图；断言
  CAD canvas 有像素，且 renderer 是 Native/Vulkan 并持有 `/dev/nvidia0` fd。需要合法可用
  的本地 CJK SFNT 字体、桌面 display 与 NVIDIA GPU。建议串行独立进程执行：
  `YACR_CJK_FONT=/absolute/path/to/cjk.ttf YACR_CJK_OUTPUT=/tmp/opencode/yacr-cjk-font-<unique> \
  cargo test -p app-linux --lib real_gpu_cjk_sfnt_text_fill_acceptance --locked -- --ignored --test-threads=1 --nocapture`
  （环境需保留 Wayland/X11 与 `/run/user/1000`，不得设置 Vulkan/software adapter override；本地截图
  和 renderer 证据写入指定新目录）。本轮实跑证据及边界见上方“最终真 GPU 中文专项通过”。
- 初始实现轮已运行：`cargo test -p cad-representation --locked`（173 passed）及带真实本地 SHX 的 provider
  focused test（1 passed）；
  `cargo test -p app-linux --lib real_gpu_cjk_sfnt_text_fill_acceptance --locked --no-run` 编译通过；
  `cargo fmt --all -- --check` 通过。后续修订后的测试与真 GPU 证据见本节之前的最新记录。

## Offscreen Android UI suite 已在真实 hardware Vulkan 上通过（2026-10-12）

- 修正 `cad-ui-slint::offscreen` 的测试 adapter 策略：先 high-performance，再在非硬件 adapter 时探测 low-power，
  仅 hardware 类均不可用时才接受并明确标记 CPU software fallback；仍要求 Vulkan 并创建 FemtoVG wgpu renderer。
  未改业务生产逻辑，未强制 ICD。主机 `nvidia-smi` 可见 Quadro P620，测试内真实 adapter 日志为
  `name=Quadro P620, device_type=discrete_gpu, backend=vulkan`。
- 上轮 app-android full suite 唯一失败的
  `adapter_suppression_cancels_android_tail_and_restores_next_tap_and_tool_path` focused 复跑 1/1 passed；随后串行
  `cargo test -p app-android --lib --locked -- --test-threads=1 --nocapture` **26/26 passed**。这是 headless adapter
  与 UI interaction-contract 证据，不等于真实窗口/Android device/视觉或性能验收。
- `cargo fmt --all -- --check` 和 workspace Clippy `-D warnings` 均通过。没有 adb、build、安装或设备操作。
  证据目录：`/tmp/opencode/yacr-offscreen-hardware-20261012-084758/`。详情见
  [validation-android.md §20](validation-android.md#20-offscreen-adapter-优先硬件并复验-android-ui-suite-2026-10-12)。

## Linux HostCore navigation bridge 真窗口用例已取得终态（2026-10-12）

- 对新增 ignored test `host::validation::tests::real_gpu_host_navigation_push_updates_bridge_and_preserves_full_push_semantics` 执行了独立真实窗口验收，1/1 passed（15.70 秒），使用 Wayland/X11 和 Quadro P620，无 GPU override；timer 有 90 秒 deadline，命令还有 150 秒外部 timeout。
- 实际 renderer evidence：Native/Vulkan、1 frame、`/dev/nvidia0` fd 存在；同目录 `host-navigation.txt` 有导航/桥接语义证据。此结果只证明该 Linux 测试宿主路径，不推广为视觉或性能、Android 真机结论。详见 [validation-android.md §21](validation-android.md#21-linux-hostcore-轻量导航真实窗口-bridge-acceptance-2026-10-12)，证据目录 `/tmp/opencode/yacr-git-push-20261012-091936/`。

## 统一门禁修复已完成；现有装机 APK 仍对应前一源码指纹（2026-10-12）

- 仅格式化 writer 已修改且 fmt 报告的 Rust 文件，并把 `crates/cad-host/src/input.rs` 中
  `wheel_anchor_tests` 模块移至文件末尾；没有改逻辑、添加 lint allow、触碰陌生 Rust 文件或格式化全仓写回。
- 当前 `cargo fmt --all -- --check`、workspace Clippy `-D warnings`、`cargo test -p cad-host --locked`（22/22）
  与四项 Python 静态门禁均通过。Android offscreen 全 suite 的既有限制仍是 25/26：单一用例要求软件 Vulkan，
  却观察到真实 Quadro P620；本轮不将该结果写成通过、不软件模拟真机。native/wasm check 与 Android focused
  pinch/pan/scroll 测试复用上一轮未受这些纯格式/test 位置调整影响的结果。细节见
  [validation-android.md §19](validation-android.md#19-wheelmetrics-统一门禁修复-2026-10-12未重建未操作设备)。
- dirty production fingerprint 已从旧版 `51b59eeb92d2057d06b96212f3d2bedf80e1f808cab9fe70d5686698e718bc65`
  变为当前 `a0159bf87b4c92138bc6a1c1d9f05debfb58e835f65bfd12f8d9e257cc2f9922`。此任务按要求未重新构建/安装/操作设备；
  已装 APK `8bedbf51399fafac0df7d80bb21388dcc699186b474b8e4a8cac56f8d3593e5f` 仍含完整 100 项字体，但对应旧 source
  fingerprint，不能宣称由当前 source 构建。
- 证据目录：`/tmp/opencode/yacr-gate-fix-20261012-083111/`。未提交。

## wheel/metrics/font 版 Android release 已安装，待解锁验收与门禁修正（2026-10-12）

- 本轮 dirty production fingerprint 为
  `51b59eeb92d2057d06b96212f3d2bedf80e1f808cab9fe70d5686698e718bc65`，构建前后相同；untracked
  `crates/cad-host/src/zoom_anchor_tests.rs` 是测试文件，已单独 fingerprint。最终 APK 含当次完整字体清单
  100 项，SHA-256 `8bedbf51399fafac0df7d80bb21388dcc699186b474b8e4a8cac56f8d3593e5f`，并已在
  `ebfb4268` 以 `install -r -t` 保留数据安装。全量证据见
  [validation-android.md §18](validation-android.md#18-wheelmetricsfont-合并版验证与含全字体-apk-2026-10-12设备画面仍待解锁)。
- **门禁阻塞待 writer 处理**：`cargo fmt --all -- --check` 有多处 Rustfmt diff；workspace Clippy 被
  `crates/cad-host/src/input.rs:150` 的 `items_after_test_module` 阻塞（wheel test module 放在后续 impl 前）。
  本验证 owner 未运行 fmt 写回、未修改源码。其他 architecture/fixture/workflow/i18n、native workspace check、
  wasm lib check 通过。cad-host 22 tests 通过；Android 相关 pinch/pan/scroll focused tests 与 fonts 8 tests 通过。
  app-android 全 suite 25/26，唯一失败为 offscreen 用例拒绝 NVIDIA hardware adapter、要求 software Vulkan；未切软件后端重跑。
- 真机应用启动返回 OK/PID `24137`，但手机休眠：截图黑屏，logcat 明确为
  `canvas metrics are degenerate`，没有可见 app frame 或 wgpu/Vulkan canvas 证据；设备唤醒后仍是锁屏，未绕锁。
  **需用户手动解锁，再从 launcher 打开 Yacr 并留在前台**，否则不能继续点 UI。专用授权 fixture 已放入
  `/sdcard/Download/YacrFinal-20261012-081219/synthetic-four-lines.dwg`，只包含仓库授权的四条 LINE。
- 解锁后的步骤：在 app 用 SAF 只打开此 fixture，切到 Model/模型空间，先确认图形可见；单指拖动画布并点 zoom
  in/out controls。双指 pinch in/out 请用户在真机屏幕上实际操作；若接有实体鼠标/触控板，再在画布上滚动 wheel。
  不打开用户文件，不用 `adb input swipe` 冒充 pinch/wheel；反馈每个动作是否到达、方向/画面变化及主观响应。
- `gfxinfo` 的 0 frame/Skia-HWUI 数据不覆盖 Slint wgpu canvas；性能流畅性未量化。用户卡顿目前只是减少重复 work，
  不能报告已解决。没有有效图像帧时也不下字体运行时加载/可视性的结论。
- 当前证据：`/tmp/opencode/yacr-android-final-20261012-081219/`。无提交。

## pinch/navigation 修订版 Android 真机待现场验收（2026-10-12）

- 本轮 production dirty-source fingerprint 构建前后相同，正式无代理 arm64 release 构建成功，APK
  已通过 hash/ABI/SAF dex/签名/manifest 检查，并在唯一设备 `ebfb4268` 上用 `install -r -t`
  保留数据安装、启动（PID `20719`）。详见
  [validation-android.md §17](validation-android.md#17-pinchnavigation-修订版-arm64-release-部署2026-10-12等待解锁后交互验收)。
- 设备当时休眠；启动截图黑屏，唤醒后显示系统锁屏。**当前用户需先解锁设备，再从 launcher 打开
  Yacr 并保持 app 在前台**；不要用 adb 注入多指或绕过锁屏。本次只看到了 Qualcomm Vulkan/wgpu 初始化
  日志，没有可见画布/渲染帧，也没做新版 UI、pan/zoom 或 pinch 的交互验收。
- 唯一授权样本已准备在
  `/sdcard/Download/YacrTest-20261012-025646/synthetic-four-lines.dwg`（4 条 LINE，SHA-256
  `68637e120e71e471eccb5cfe593a2da156444b8ef2e8669bc4f521ce32c38135`）。解锁后在 app “文件→打开图纸”
  中只选此样本；切至 Model/模型空间，先确认几何可见，再试单指拖动与缩放按钮。双指 pinch in/out
  请直接在设备屏幕上进行并记录主观响应；若无法证明 raw 触点确实到达 app，不报告 pinch 通过。
- `adb shell input swipe` 只能作单指输入，不是双指验收。当前 `gfxinfo` 0 帧且 Pipeline 为
  Skia/OpenGL，不能覆盖 Vulkan canvas，也不能证明流畅。设备支持的 canvas 帧时间采集方式尚未确认；
  无合适工具时诚实标记性能量化未测。证据目录：`/tmp/opencode/yacr-android-acceptance-20261012-025646/`。

## Android 真机交互/性能待办（2026-10-12）

- 真机 release 已在 `ebfb4268` 成功安装并启动；同一轮 SAF 打开 manifest 授权的
  `synthetic-four-lines.dwg` 后标题成功更新，但当前 `*Paper_Space` 画布空白，诊断有
  `import.annotation_scale_unknown`。内容/实体/视觉完整性没有验收通过。详见
  [validation-android.md §16](validation-android.md#16-真机-arm64-release-启动saf-与手势初测2026-10-12交互性能仍待补测)。
- 用户报告：Android 无双指缩放，拖图与缩放很卡。只读定位 lane 确认 Android
  `view.rs` 移动 contacts=1、canvas `TouchArea` 未传第二指；该处需实现 owner 后续处理，本轮不改代码。
- 本轮通过 ADB 单指 swipe 操作 demo，第二次平移使 canvas crop 变化 11,227 px；单击 zoom-in/out
  工具按钮也改变视图。没有物理双指手势、连续帧/响应时间基准；`gfxinfo` 是 HWUI 而非 wgpu canvas，
  不能据其 0 帧/0 jank 数声称流畅。设备现自动休眠并显示锁屏，本轮不绕过锁屏继续操作。
- **后续验收待办（不代表已复现）**：用户解锁并保持测试页前台后，获取可靠的多触点输入方式，
  在授权 DWG 与已确认实体可见的样本上记录 pinch in/out、平移与缩放时序、PID/前台、app logcat、
  可覆盖 native Vulkan canvas 的帧/性能证据；主观卡顿与客观帧时间分开记录。`HostCore::push` 每次
  Pan/Zoom 重复派生面板状态仅为待分析候选，尚未证实是瓶颈。
- 新证据：`/tmp/opencode/yacr-android-device-20261012-010448/`；APK 安装成功/应用存活不等同于
  手势/性能/视觉验收通过。

## Android USB 真机 release 已构建，安装受设备限制（2026-10-12）

- 当前 checkout `96d631fe8dcaf6658ebee850008ad2f55034d39f` 上本轮实际运行无代理 release 正式脚本成功，
  arm64 APK 已含 SAF dex/manifest 并验签；详情与 SHA-256 见
  [validation-android.md §15](validation-android.md#15-arm64-release-真机构建与安装尝试2026-10-12等待设备授权)。
- `adb -s ebfb4268 install -r -t .../target/release/apk/yacr.apk` 被设备拒绝：
  `INSTALL_FAILED_USER_RESTRICTED: Install canceled by user`。安装后设备无 app package/pid，未启动；
  未卸载/清数据/读取用户文件。
- 继续前需用户解锁真机，在系统/开发者选项允许 USB 安装；如果设备显示授权对话框，请允许/安装。
  然后再对相同 serial 执行 `adb install -r`，保留应用数据，继续启动/交互/日志/截图验收。证据目录
  `/tmp/opencode/yacr-android-device-20261012-005711/`。

## Android USB 真机：用户报告构建成功后的制品核验（2026-10-12，等待 release APK）

- 用户报告手动运行 `build-android.sh` 成功；本轮未重复构建。唯一设备 `ebfb4268` 在线，型号
  Xiaomi `23046RP50C`，Android 15/API 35/arm64-v8a。当前设备未安装 `dev.yacr.app`。
- `target/release/apk/yacr.apk` 仍是早前 `x86_64` artifact，不适用于设备；最新可见
  `target/debug/apk/yacr.apk` 虽为 arm64、有 SAF dex/manifest 并可验签，但为约 742 MB debug APK，
  本轮不违反既定 release 安装规则，故未安装。另一个旧外部 arm64 release artifact 缺 `classes.dex`
  且 `hasCode=false`，不可启动。不能仅凭用户的成功报告断言已获得当前源码对应的 release APK。
- 因无适用 arm64 release，本轮没有启动/交互/应用日志/截图/GPU 渲染结论，也没有卸载、清数据或读取用户文件。
  产物及设备证据：`/tmp/opencode/yacr-android-device-20261012-005421/`，详见
  [validation-android.md §14](validation-android.md#14-用户报告构建成功后的-apk-核验2026-10-12安装被-release-制品阻塞)。
  需提供 `target/release/apk/yacr.apk`（或明确的 `$CARGO_TARGET_DIR/release/apk/yacr.apk`）后再继续。

## Android USB 真机：指定代理重试（2026-10-11/12，仍阻塞）

- `scripts/build-android.sh` 新增可选 `ANDROID_BUILD_HTTPS_PROXY`：先在隔离子 shell 中清除环境代理，
  仅该变量非空时将其设置为小写 `https_proxy`；默认仍不使用代理，source 时调用者环境不变。
  `scripts/test-android-build-proxy.py` 覆盖代理传入、其他代理清除和父 shell 原值保留。
- 以 `http://192.168.8.1:10809` 做有界探测：Skia GitHub release 单字节 Range 返回 206；
  `skia.googlesource.com` DEPS 在 5 秒连接超时。随后按正式脚本构建，两个 Android `cargo check`
  通过，但 `skia-bindings` 链接 30 分钟无日志进展，达到上限后退出码 124；未留下构建进程。
- 未得到 arm64 APK，未安装/启动应用。设备仍在线，但用户数据未触碰；原有旧 x86_64 APK 未用。
  本轮设备/探测/构建证据在 `/tmp/opencode/yacr-android-device-20261011-235628/`；细节见
  [validation-android.md §13](validation-android.md#13-usb-真机重试2026-10-1112指定-https-代理构建超时)。

## Android USB 真机 smoke 尝试（2026-10-11，本轮阻塞）

- 连接设备唯一：小米 `23046RP50C`，Android 15/API 35，`arm64-v8a`；adb serial
  `ebfb4268`。设备声明 Adreno 730 / Vulkan 能力，但 app 未启动，不能据此声称应用 GPU 渲染通过。
- 以当前源码 `96d631fe8dcaf6658ebee850008ad2f55034d39f` 运行 release 正式脚本，卡在
  `skia-bindings` 下载：原代理连接下载中断；新增脚本隔离代理后直连 GitHub 下载超时，多个
  googlesource 依赖 clone 也连接超时。构建失败，无 APK 可供本轮安装；未安装旧 x86_64 APK，
  亦未对设备安装/清理/启动任何应用。
- 本轮新增 `scripts/build-android.sh` 子 shell 作用域与代理清除（8 个大小写变量），并添加
  `scripts/test-android-build-proxy.py`，测试构建子命令无代理且 `source` 不污染调用者环境；
  `bash -n`、契约测试通过。`docs/build.md` 已说明此行为。
- 本轮构建错误、设备属性、旧 APK 元数据与限制详见
  [validation-android.md §12](validation-android.md#12-usb-真机验证尝试2026-10-11构建阻塞未安装)，证据目录
  `/tmp/opencode/yacr-android-device-20261011-231245/`。后续需在 Skia 依赖可下载的环境重新构建
  arm64 release，再执行安装更新、启动与交互验证。

## 桌面 MOVE 选择快照与关闭取消（2026-10-11，本轮）

- 桌面 `MOVE` 在工具开始时由 `HostCore` 快照所选 `SelectionRef`；确认沿用该快照，不再读取确认时的当前选择。新工具开始会替换快照，取消会清除；命令仍走既有事务/历史漏斗，失败不改为空成功。契约覆盖“选 A → 开始 MOVE → 选择变为 B → 确认只移动 A”、取消/重启重置及撤销/重做。
- Linux 50 ms timer 现在忽略 `CadError::Cancelled`（例如窗口销毁时 surface 已不可用），其余错误仍进入 `linux.failed`。
- 集成后 targeted 验证：`cargo test -p cad-host --locked`（16 passed）；`cargo test -p app-linux --lib timer_ignores_cancellation_but_reports_real_errors --locked`（1 passed）；workspace clippy/native/wasm 检查及 `cargo fmt --all -- --check` 通过。五用例真实硬件复验为 1 passed / 4 failed；更新后的详细门禁、GPU PID 证据、失败原因与截图路径见 [validation-real-gpu-2026-10-11.md](validation-real-gpu-2026-10-11.md)。不得用修改前 2/4 的基线覆盖本轮结果。
- 随后按 `YACR_PROFILE_PICK=1` 的硬件剖析复跑为 **3 passed / 2 failed**：小 DXF 真实窗口选择/MOVE/Undo/Redo 与 grid/zoom 用例通过；complex-test 两项选择延迟仍约 2.9s。剖析将瓶颈定位于 `pick_closest_borrowed` 对 723,587 个候选的 CPU 遍历（约 2.66–2.68s），push 仅约 1.7ms；本轮未改性能代码。日志/PID/GPU/PNG 与限制见验证文档新增「带选取剖析的真实 GPU 复跑」节；更新后的 complex-test 应用 PID `3901294` 仍打开。
- 最终六用例实机轮 **3 passed / 3 failed**：CPU load、grid/zoom/axes、小 DXF selection/MOVE/history 通过；complex-test 两项 pick 延迟约 2.79/2.82s，maximized/crosshair 用例停于 phase 2 的 `frames=5` readiness 条件（其余 view 状态 ready，诊断详见验证文档）。最终门禁 clippy 因 `cad-spatial/src/pick.rs:579 needless_option_as_deref` 失败，i18n scanner 因 `host/validation.rs` 末尾多个 `#[cfg(test)]` 失败；其它 fmt/native/wasm/architecture/fixture/workflow 通过。仅执行 `cargo fmt --all` 机械格式化，未作行为修复。全证据与逐项日志见 [validation-real-gpu-2026-10-11.md](validation-real-gpu-2026-10-11.md)；最终 app PID `4006015` 保持打开 complex-test，P620 PID/显存采样已记录。
- compound-cache 与 hover 截图等待改动后的最新六用例轮仍 **3 passed / 3 failed**。两项大 DWG pick 为 3.729/3.624s，其中 closest scan 3.464/3.347s；未达到 2s，未观测到 next-heartbeat/最终像素成功。maximized 测试已通过 grid/crosshair/zoom/pan，失败在恢复尺寸：请求 1773×953，实际 1280×800。gate 当前为 fmt/native/wasm/4 Python PASS，clippy 因测试中的 `redundant_pattern_matching` FAIL。更新证据、PNG 与 GPU pid 见验证文档最新一节；app PID `4052352` 仍打开 complex-test，P620 863 MiB。
- 仅 compound local-bounds cache（无 leaf HashMap）及 Wayland configure 稳定等待之后的最新实机轮 **5 PASS / 1 FAIL**：两项 complex DWG 选择 1.384/1.397s，通过 2s 阈值，并记录 30 ticks、最大事件间隔 1.447/1.461s、CAD 帧 24/26、变化像素 42,884；maximized 用例所有阶段、7 PNG、resize/re-maximize 均完成，但外层测试结束断言 `left: 0,right: 8` 失败。fmt/clippy/native/wasm/i18n 均 PASS；architecture/fixture/workflow 沿用最近有效通过结果。全日志、family visits/culls、GPU pid 与截图见验证文档最新节；最终应用 PID `4092870` 打开 complex-test，P620 863 MiB。
- 父级修复 phase completion 共享计数后，定向 P620 maximized 用例 PASS：grid 四边、crosshair、zoom/pan、restore `1088×680`、re-maximize `1853×1013` 与最终阶段断言全部通过，7 PNG 已保存。结合前述同一 compound-cache 轮次的五项有效 PASS，六项 acceptance 证据均通过；并未重跑整个六用例脚本。fmt/clippy app-linux scoped/native app check/i18n 均通过；旧 PID `4092870` 已精确核验并终止。最终 app PID `4134293` 已打开 complex-test，P620 863 MiB。详见验证文档最后一节。

## Android 文件功能：SAF 打开 / 另存 / New（P4，2026-10-11，本轮）

用户要求：给 Android 宿主接线**真实文件打开（SAF `ACTION_OPEN_DOCUMENT`）**、**另存
（SAF `ACTION_CREATE_DOCUMENT`）**与**New（空白图纸）**，经既有 `PlatformFiles` 缝，
并在无头 x86_64 模拟器上以 adb 实测。

- 能力位：`apps/app-android/src/lib.rs` 翻到 `{open:true, new:true, save:true, plot:false,
  import_worker:true}`；`can-new/can-save` 由共享 `HostCore::push` 推送，Ctrl+N / `NEW`、
  Ctrl+S / `SAVE` 变为可用。plot 在 Android 仍显式不支持（无矢量导出面）。
- Java 垫片：`apps/app-android/android/dev/yacr/app/YacrNativeActivity.java`
  （`extends android.app.NativeActivity`，是 manifest 声明的启动 Activity）。`onCreate`
  无操作，`launch(Context,boolean,String)` 用 `startActivityForResult` 起 SAF 选择器，
  `onActivityResult` 把结果写入静态槽（`sResult`/`sUri`/`sName`），并提供 `readUri`/
  `writeUri`（`ContentResolver.openInputStream/openOutputStream`）与 `queryDisplayName`。
- **偏离已批准设计（已记录）**：设计原为“再加一个 `SafProxyActivity` + 后处理改写二进制
  `AndroidManifest.xml`”。cargo-apk 0.10 只支持**一个** activity（`ndk-build::manifest`
  的 `Application.activity` 是单值），二进制 manifest 改写脆弱；故改为把**唯一**启动
  Activity 换成 `NativeActivity` 子类（经 `[package.metadata.android.application.activity].name`
  指定，`has_code=true`），子类同样能收 `onActivityResult`，且无需触碰编译后的 manifest。
  JNI 侧与设计一致（见下）。
- Rust JNI：`apps/app-android/src/saf.rs`（Android-only）。经 `JavaVM` + **activity 指针**
  （`app.vm_as_ptr()`/`app.activity_as_ptr()`，在 `android_main` 里于 `slint::android::init`
  之前捕获；注意 `ndk_context::android_context().context()` 是 **Application** 而非 Activity，
  无法 `startActivityForResult`）驱动 Java 静态方法；类经 **activity 的 ClassLoader** 解析
  （`FindClass` 在无 Java 帧的原生线程上取不到应用类），缓存为 `GlobalRef`。字节读写走
  Java 的 `byte[]`（`readUri`/`writeUri`），JNI 面最小。错误以未翻译串返回，由
  `platform.rs` 用目录键本地化。
- `platform.rs`：Android 上 `begin_open`/`begin_save` 启动 SAF 并把 `armed` 置为对应
  `FilePurpose`；`poll` 读结果（open 解析 URI 字节 → `FileEvent::Opened`，save →
  `FileEvent::SaveTarget`，取消/失败 → 对应事件）；`write_atomic` 经 `ContentResolver`
  写；`refuses_source_overwrite` 比较 opaque（SAF content URI）。宿主目标（非 Android）
  仍回退到样本路径探测，`begin_save`/`write_atomic` 显式 `Unsupported`。
- 异步轮询：`HostCore` 新增 `pub fn save_pending()`；`poll.rs` 的 `poll_tick` 保持轮询条件
  改为 `loading() || save_pending()`；`AndroidSink::send` 对 Save/Plot 也 `ensure_polling()`，
  使异步 SAF 另存选择器的结果被后续 tick 抽干并导出。
- 构建后处理：`scripts/build-android.sh` 在 `cargo apk build` 后 `javac`（`--source/target 8`
  + `android-34/android.jar`）→ `d8 --min-api 23` → `zip` 注入 `classes.dex` → `zipalign` →
  `apksigner` 重签 → `aapt2 dump xmltree` 校验 manifest 声明了 `YacrNativeActivity` 且
  `hasCode=true`，任一步失败即中止（无跳过开关：manifest 声明了 `hasCode=true` 与
  `YacrNativeActivity`，缺 `classes.dex` 的 APK 启动即 `ClassNotFoundException`）。缺
  `dev-release.jks` 时按文档口令自动生成。
- i18n：新增 3 键（两份 catalog 一致）：`android.saf_unavailable` / `android.saf_failed` /
  `android.saf_write_failed`。
- 模拟器渲染门槛：为让无头模拟器的 Vulkan 适配器不被 wgpu 隐藏，`android_main` 设置
  `WGPU_ALLOW_UNDERLYING_NONCOMPLIANT_ADAPTER=1`（对合规真机驱动为 no-op）。

验证（本机；**无头 x86_64 模拟器，非真机**；证据 `/tmp/opencode/yacr-android-p4/`）：

- `cargo fmt --all -- --check`；`cargo clippy --workspace --exclude app-android --all-targets
  --locked -- -D warnings`；`cargo clippy -p app-android --target x86_64-linux-android
  --all-targets --locked -- -D warnings`；`check-architecture.py`（26 包）；`check-i18n.py`
  （306 键）；`check-fixture-manifest.py`；`check-workflows.py`。
- `cargo check --workspace --exclude app-android --all-targets --locked`；
  `cargo check --workspace --lib --target wasm32-unknown-unknown --locked`；
  `cargo check --target aarch64-linux-android -p cad-ui-slint -p app-android --all-targets --locked`；
  `cargo check --target x86_64-linux-android -p cad-ui-slint -p app-android --all-targets --locked`。
- `cargo test -p app-android -p cad-host --locked`：app-android **18 通过**、cad-host **14 通过**，0 失败。
- 构建：`TARGET=x86_64-linux-android bash scripts/build-android.sh --release` 产出含
  `classes.dex` 的 APK（`hasCode=true`，`launchable-activity=dev.yacr.app.YacrNativeActivity`）。
- 模拟器 e2e（`yacr35`，API 35，`google_apis`，x86_64，`-gpu swiftshader`，KVM）：
  - **打开**：点顶栏“打开” → DocumentsUI `ACTION_OPEN_DOCUMENT` → Downloads 选 `flange.dxf`
    → 返回 content URI（`...raw%3A%2Fstorage%2Femulated%2F0%2FDownload%2Fflange.dxf`）→
    应用标题变 `flange.dxf`、画布渲染法兰几何（截图 `12-open-loaded.png`）。
  - **另存**：`Ctrl+S`（`input keycombination 113 47`）→ DocumentsUI
    `ACTION_CREATE_DOCUMENT`（默认名 `yacr-demo.dxf`）→ SAVE →
    `/sdcard/Download/yacr-demo.dxf` 写出 48333 字节，首行 `0 SECTION 2 HEADER`、末行 `EOF`
    （拉回证据 `yacr-demo.dxf`）。
  - **New**：`Ctrl+N`（`input keycombination 113 42`）→ 标题变 `untitled`、画布空白
    （截图 `16-new-done.png`）。
  - 全程无 `FATAL EXCEPTION` / `ANR in dev.yacr` / panic（logcat 扫描为 0）。
- **模拟器（`yacr`，API 30）无法渲染（环境阻塞，非本轮改动）**：其 goldfish/SwiftShader
  Vulkan 驱动报 `maxBufferSize=0`，wgpu `request_device` 失败（
  `Limit 'max_buffer_size' value 268435456 is better than allowed 0`）；适配器选择还需
  `WGPU_ALLOW_UNDERLYING_NONCOMPLIANT_ADAPTER`。API 35 镜像（`yacr35`）无此问题，故 e2e
  在其上完成。见 `docs/validation-android.md`。
- 未运行：**真机**、OEM provider（Google Drive/厂商云盘）、多 API 矩阵、SAF 源文件覆盖
  守卫的模拟器场景（`ACTION_CREATE_DOCUMENT` 产生新 URI，无法稳定复现同 URI；守卫逻辑由
  cad-host 单测覆盖）、Android 离屏渲染。

补强（P4 评审后，无范围扩张）：

- **M1** 删除 Java 侧 `PURPOSE_*`/`sPurpose`（Java 不再镜像交互目的）；交互目的由 Rust 的
  `armed: FilePurpose` 单独拥有并传入 `saf::poll(purpose)`，两侧不可能再对不上。`saf.rs`
  注释更正为只镜像 `STATE_*`。
- **M2** `onActivityResult` 的 `takePersistableUriPermission` 移入独立 try/catch（失败仅
  忽略），不再把一次成功的选择降级为 `STATE_FAILED`。
- **M3** 移除 `YACR_ANDROID_SAF=0` 跳过开关（会产出无法启动的 APK）；`docs/build.md`
  更正为“后处理强制”。
- **M4** `cad-host::open::request_open` 在 `save_pending()` 时显式拒绝（`linux.busy`，非静默
  no-op）；`AndroidFiles::begin_open/begin_save` 在 `armed.is_some()` 时返回显式错误；目的/
  结果不匹配改为 `FileEvent::Failed`（不再猜成 `SaveDxf`）。映射抽成 host 可测的
  `platform::saf_event(purpose, SafResult)`，新增两个单测。
- **JNI 加固**：`run` 用 `push_local_frame(32)`/`pop_local_frame` 包住一次交互（消除永久附加
  线程的本地引用泄漏）；Java `snapshot()` 在 `LOCK` 下一次性返回
  `{sResult,sUri,sName,sError}`，Rust 单次调用读取（消除 ARM 上的可见性竞态）；
  `onActivityResult` 对未知 requestCode 提前返回，`RESULT_OK` 无 URI 映射为 `STATE_FAILED`
  （非取消）；修正 `launch` javadoc（在 android_main 线程调用）。
- **Minor**：`Cargo.toml` min/target SDK 对齐为实际生效的 23/30（顶层键本就不生效）；
  `MIN_SDK` 注释对齐；`android_save_is_an_explicit_unsupported` 更名
  `host_fallback_save_is_explicit_unsupported`。`write_atomic` 的 I/O 失败仍用
  `CadError::InvalidInput`——与 `app-linux` 的 `atomic_write` 一致，且 `cad-domain` 无更贴切
  变体（新增变体会破坏穷尽匹配，超范围）。
- **e2e 复跑发现的连带修复（预存在 bug）**：`cad-host::open::poll_loading` 原先在清
  `presenting_open` **之前**计算 `set_open_available`/`set_save_available`，于是“打开完成”
  那一 tick 仍以 `loading()==true` 刷新，而 Android 的轮询在 idle 时自停，导致**打开之后
  `can-save` 仍为 false**（Ctrl+S 被当成 unsupported）。改为在 presentation 状态结算**之后**
  最后刷新可用性。复跑确认打开后 Ctrl+S 立即打开 `ACTION_CREATE_DOCUMENT`。
- 复跑 e2e（`yacr35`，API 35，`-gpu swiftshader`；证据 `/tmp/opencode/yacr-android-p4/`
  `r4-flange-loaded.png`/`r5-open.png`/`r10-new.png`/`flange-out.dxf`/`logcat-remediation.txt`）：
  打开 `flange.dxf`（标题+几何）→ 打开后立即 Ctrl+S → 改名 `flange-out.dxf` 保存
  （**145282 字节**，`0/SECTION/2/HEADER … EOF`）→ Ctrl+N 得 `untitled` 空白；无
  `FATAL EXCEPTION`/`ANR`/panic。`cargo test -p app-android -p cad-host` = **20 / 14 通过**。

## Web 文件功能：New / 另存 DXF / 出图 SVG·PDF 下载（P3，2026-10-11，本轮）

用户要求：为 Web 宿主接线 **New（空白图纸）**、**另存 DXF 下载**、**出图 SVG/PDF 下载**，
经共享 `HostCore` + `PlatformFiles` 统一漏斗，并把 Web 能力位翻到
`{open:true, new:true, save:true, plot:true, import_worker:false}`；新增 Playwright
**headful 真实 GPU**（非 SwiftShader）端到端。

- 能力位：`apps/app-web/src/browser.rs` 的 `HostCapabilities` 四项全开、`import_worker=false`；
  `can-new/can-save/can-plot` 由共享 `HostCore::push` 推送（桌面/Web 同一门）。Ctrl+N / `NEW`、
  `SAVE`、`PLOT` 在 Web 变为可用。
- 下载：`WebFiles::begin_save` 在点击同一手势内同步合成 `download://<name>` 目标，核心立即
  drain + poll，整次导出在一次手势内完成（设计使然；`<a download>`+Blob 本身不要求用户激活）。
  `WebFiles::write_atomic` 用 web-sys 触发下载：`Blob`（`application/octet-stream`）+
  `createObjectURL` + `<a download=name>` 点击（anchor 先入 DOM 以兼容 Safari），对象 URL 于
  `setTimeout(0)` 后 revoke（`Closure::once_into_js` 自销毁，不泄漏）；空字节显式拒绝。
- 共享漏斗：`cad-host::save.rs` 新增 `finish_or_wait_save`——同步平台（浏览器）手势内完成导出，
  异步对话框（桌面）仍留 `save_pending` 等定时器；`request_plot` 默认扩展名取自新增的
  **schema 校验字段 `ui.export.plotFormat`**（`svg`/`pdf`，默认 `svg`，未知值 serde 拒绝）。
- 出图字体：wasm `load_plot_fonts` 复用视图上已装载的浏览器字体引擎（新增只读
  `CadView::fonts()`）；未装载且图纸引用文本字体时显式 `Unsupported`，无文本引用无需字体。
- `.png` 出图仍为显式 `plot.png_cli_only` 不支持（无表面回读）。
- i18n：复用既有 `save.*`/`plot.*`，**无新增键**（297 键，两份 catalog 一致）。

验证（本机，**headful 真实 GPU WebGL2**，非 SwiftShader；详见 `docs/validation-web.md` §15）：

- `cargo fmt --all -- --check`；`cargo clippy --workspace --exclude app-android --all-targets
  --locked -- -D warnings`；`cargo clippy -p app-web --target wasm32-unknown-unknown
  --all-targets --locked -- -D warnings`；`check-architecture.py`（26 包）；`check-i18n.py`（297 键）。
- `cargo check --workspace --exclude app-android --all-targets --locked`；
  `cargo check --workspace --lib --target wasm32-unknown-unknown --locked`。
- `cargo test -p cad-host -p cad-app --locked`：cad-host **14 通过**、cad-app **349 通过**，0 失败。
- `node --test scripts/test-web-host.mjs`：**15 通过**（新增 `ui.export.plotFormat` 转发/拒绝契约）。
- `DIST=/tmp/opencode/yacr-web-p3 WITH_FONTS=0 scripts/build-web.sh`：release 链接成功，
  wasm **20,698,521 bytes**，build stamp `a514afcf2bc687ce`。
- `node scripts/check-web-files.mjs`（headful chromium，ANGLE/Mesa Intel UHD，**非**
  SwiftShader）：**passed**；上传 fixture → New（`entities=0`）→ 另存 `a4-layout.dxf`（47,210 B）
  → 出图 `a4-layout.svg`（含 `<polyline>`）→ `updateConfig({"ui":{"export":{"plotFormat":"pdf"}}})`
  → 出图 `a4-layout.pdf`（`%PDF-`）；证据 `/tmp/opencode/yacr-web-p3-e2e/`。
- 桌面回归：`cargo test -p app-linux --test plot_export --test save_export --test new_drawing
  --test save_source_guard --locked`（lavapipe）：**4 通过 / 0 失败**。
- 未运行：WebGPU（本机 headful 无 `navigator.gpu`，回退 WebGL2）、移动/桌面浏览器矩阵、真机、
  带 CAD 文本的 Web 出图成功场景（字体未装载时为显式 Unsupported）。
- 已修复（后续轮）：`scripts/test-host-panel-parity.mjs` 的 Android 断言曾按旧结构查找
  `set_layer_state`/`set_overlay_visibility`；现 Android 条目断言共享面板来自
  `crates/cad-host/src/state.rs`，并单独断言 `apps/app-android/src/state_push.rs` 的 overlay 符号
  （`set_import_state`/`set_resources_sections`）。

补强（P3 评审后，无范围扩张）：
- **F1** `scripts/check-web-files.mjs` 新增 `page.on("console")` 收集并断言 error 级为空
  （与 `check-web-ui.mjs` 一致），使“无 page/console 错误”的说法有证据；文档同步改为
  “无 page/console 错误（两者均断言为空）”。
- **F2** 强化内容断言：cad-host `a_synchronous_plot_writes_real_vector_content` 增断言 SVG 含
  `<polyline>/<polygon>/<path>`（不再只查 `<svg`）；`check-web-files.mjs` 的 PDF 增断言含路径
  算子（`l`/`re`），不会对空页 PDF 误判通过。
- **F5** `apps/app-web/src/browser/persistence.rs`：`document.body()` 缺失时返回显式
  `Unsupported`（不再点击游离 anchor 后谎报“已导出”）。
- **F3** 新增 6 个 `web.*` i18n 键（两份 catalog）承载下载/字体错误文案；`persistence.rs`/
  `platform.rs` 的 `Unsupported` 内层文案与 `fonts.rs` wasm 字体缺失文案经目录本地化；
  `cad-host/save.rs` 的 `plot.export_failed`/`save.export_failed` 原因改用 `error_text`
  解包，避免 `Unsupported("…")` 的 Debug 引号。
- **F4** 更正“浏览器仅在用户手势内允许下载”的过度陈述（`platform.rs`/`save.rs`/文档）：改为
  设计意图（把导出收敛到点击同一手势），并说明 `<a download>`+Blob 本身不要求用户激活。
- **F6** `persistence.rs` 的对象 URL revoke 改用 `Closure::once_into_js`（自销毁，不再 `forget()`
  泄漏）。
- **F7** `apps/app-linux/tests/plot_export.rs` 捕获建议文件名：默认断言 `.svg`，经
  `handle.update_config_json({"ui":{"export":{"plotFormat":"pdf"}}})` 后断言 `.pdf`（并加 PDF 路径
  算子断言）。
- 补强后证据：`cargo test -p cad-host`（14 通过）、`cargo test -p app-linux --test plot_export`
  （1 通过，lavapipe）；`node --test …`（47 通过 / 0 失败）；`check-i18n.py`（303 键）；
  Playwright e2e `check-web-files.mjs` 重建 web-dist 后 **passed**（headful Chromium/WebGL2，
  `consoleErrors=[]`、`errors=[]`）。

## Android 宿主迁移到 cad-host（P2，2026-10-11，本轮）

用户要求：把 `apps/app-android` 迁移到 P0/P1 抽取的 `cad_host::HostCore`，删除 Android 侧
重复编排（`state_push.rs`/`view.rs`/`draw.rs` → 薄/浏览器等价适配器），**行为保持**；并修复
i18n-law 债务（Android 硬编码中文状态文案迁入 i18n 键，两份 catalog 键集一致）。Android 的 SAF
打开/另存（P4）本轮**不做**：能力位保持 `open=true`、`new/save/plot=false`，打开仍为硬编码
样本路径探测。

- cad-host 扩展（行为保持）：新增 `HostResources` 策略（`HostPlatform.resources`），把桌面
  `install_fonts`/`install_images` 逻辑原样迁入 `DesktopResources`；`UnsupportedResources` 供 Web
  （不被调用）；`HostCore::install_fonts/install_images` 改为委托策略。`metrics` 允许 handle 未安装
  （无窗口/测试核心仍应用表面映射，只跳过窗口布局重分类；真实宿主都在 handle 之后调用）。
- 新增 `apps/app-android/src/platform.rs`：`AndroidFiles`（`begin_open` 探测样本路径同步读入并
  入队 `FileEvent::Opened`；无样本/读取失败为显式 `FileEvent::Failed`；`begin_save`/`write_atomic`
  显式 `Unsupported`；`refuses_source_overwrite` 比较 opaque 句柄——无保存故不被调用）、
  `AndroidConfigStore`（无持久化存储：加载 `Absent`，写入显式 `Unsupported`）、`AndroidSurface`
  （表面槽 + 安全区）、`AndroidResources`（`HostResources`：字体经 `android_fonts` APK assets 装载，
  图像不建模 → `images_modeled:false`）。
- `lib.rs` 变为组合根：以核心作为 `UiCommandSink` 委托目标（`AndroidSink` 仅 execute + 重放 Android
  叠加；打开后 `ensure_polling` 启动定时器）、`LayoutSwitchSink`、`CanvasPickMapper`、绘制预览 sink；
  `core.set_handle`/`set_view` 后 `core.metrics()` + `push_android_state` 承担通用面板推送。
- 删除的重复编排：`src/host.rs`（`HostSink`）、`src/state_push.rs` 的 `push_panel_state`/`snapshot`/
  `view_state`/`import_diagnostics`/`AndroidCanvasPickMapper`/`apply_surface_size`（并入/复用
  `cad-host`）；`view.rs`/`draw.rs` 缩减为薄适配（保留 `InputPolicy` 状态机与 Trim/Extend 解析）。
  保留 `android_fonts.rs`（asset 加载）、`poll.rs`（Slint 定时器 + 表面入口）、`set_surface_*`。
- i18n：Android 自身状态文案迁入 11 个新键（两份 catalog 键集一致）：`android.ready`/
  `android.no_sample`/`android.read_failed`/`android.save_unavailable`/`android.no_config_store`/
  `android.pick_viewport_missing`/`android.pick_no_document`/`android.pick_failed`/`android.selected`/
  `android.pick_miss`/`android.fonts.summary`；`fonts.failed` 等复用既有键。
- 行为保持点：`android_main`/`slint::android::init`/`ensure_polling` Slint 定时器/公开
  `set_surface_size`/`set_surface_insets` 不变；真实后台导入 worker（`import_worker=true`）与 100ms
  轮询不变；能力位不变；打开仍探测 `/sdcard/Download/yacr-sample.dwg` 等。**命令与打开状态文案改由
  共享目录给出**（`linux.opened`/`linux.failed`/`linux.picker_waiting`/`command.completed`/
  `command.failed` 等）——这是采纳共享漏斗的必然结果（同 P1）；Android 自身的选择/字体/平台读取
  文案仍走 Android 键，字体摘要经 pending 槽在打开状态之后显示。

验证（本机；Android **真机/模拟器未运行**，下列为编译/契约证据）：

- `cargo fmt --all`；`cargo clippy --workspace --exclude app-android --all-targets --locked
  -- -D warnings`；`check-architecture.py`（26 包）；`check-i18n.py`（**296 键**）；
  `check-fixture-manifest.py`（22 fixture）；`check-workflows.py`。
- `cargo check --workspace --exclude app-android --all-targets --locked`；
  `cargo check --workspace --lib --target wasm32-unknown-unknown --locked`；
  `cargo check --target aarch64-linux-android -p cad-ui-slint -p app-android --all-targets --locked`；
  `cargo check --target x86_64-linux-android -p cad-ui-slint -p app-android --all-targets --locked`。
- `cargo test -p app-android --locked`：**17 通过 / 0 失败**（宿主目标；2 个
  `cfg(target_os="android")` 异步打开用例仅为 Android 目标编译、本机不运行）。
  `cargo test -p cad-host --locked`：**10 通过**；`cargo test -p app-linux --locked`（lavapipe）：
  **16 通过 / 0 失败 / 4 忽略**（无回归）。
- 未运行：Android 模拟器/真机安装运行、SAF、真实触摸手势、Android 离屏渲染。

补强（P2 评审后，行为保持；无 P4/SAF）：
- **B1 打开失败不得显示“命令完成”**：`poll_open` 的取消/失败评分抽成 `grade_pending_open()`，
  `request_open` 在同步平台于手势内完成时也调用它（失败/取消立即显示）；`execute` 的成功分支新增
  `open_status_graded` 守卫，已评分的失败不会被泛化的“完成”覆盖；`AndroidSink::send` 在
  `OpenDrawing` 后即使尚未 `loading()` 也确保一次 poll tick。新增宿主目标测试：空 `sample_paths`
  + `AndroidSink::send(OpenDrawing)` → 状态为显式失败（`command.failed` + `android.no_sample`），
  非“完成”，且保留演示文档。
- **B2 Android 不再显示 `linux.picker_waiting`**：`request_open` 仅在 `open_pending` 仍为真（确有
  选择器/读取在进行）时才显示该文案；`AndroidSink::send` 在 `loading()` 时改显示新键
  `android.opening`（两份 catalog）。
- **B3 异步打开路径的字体安装失败不再被吞**：`AndroidResources::install_fonts` 的两条错误路径先
  `set_pending_font_status(fonts.failed …)` 再返回 `Err`；`poll_loading` 的 opened 分支把字体/图像
  安装视为**非致命**（仍 `push()`），并在 opened/presenting 评分之后把该错误作为本 tick 的**最终**
  状态，避免 ~100ms 内被 presenting 恢复覆盖。
- **M1（已知差异，记录）**：旧 `open_drawing` 在某候选解析失败时会继续尝试下一个 `sample_paths`
  候选；现结构在 `AndroidFiles::begin_open` 选中第一个**可读**候选后即提交（解析失败发生在异步
  导入阶段，平台无法重试）。旧文案“打开 {path} 失败”无替代；此为已知行为差异。
- **M2**：`poll_loading` 现对 `AsyncOpenPoll::Cancelled` 评分（`linux.picker_cancelled`），不再留旧状态。
- **M3**：`apps/app-android/src/state_push.rs` 改用 `core.import_snapshot()`（含读取中合成态），
  不再直接读 `controller.async_open_snapshot()` 清空面板；cad-host 新增“异步发布失败保留当前文档”
  测试。为可观测性新增 `HostCore::set_status`/`last_status`（宿主状态镜像与无窗口测试）。
  补强后：`cargo test -p cad-host`（12 通过）、`cargo test -p app-android`（宿主目标，见验证）。

## Web 宿主迁移到 cad-host（P1，2026-10-11，本轮）

用户要求：把 `apps/app-web` 迁移到 P0 抽取的 `cad_host::HostCore`，删除 Web 侧重复编排，
**行为保持**——wasm 导出面（`apps/app-web/src/lib.rs`）、JS 宿主（`apps/app-web/web/`）、
能力门（new/save/plot 保持 `false`）不变；文件功能留待 P3。

- 新增 `apps/app-web/src/browser/platform.rs`：`cad-platform` 契约的浏览器实现。
  `WebFiles`：`begin_open` 触发隐藏 `<input type=file>` 点击；`open_document_bytes`
  入队 `FileEvent::Opened`，`poll()` 排空（`documents::drain_open_events` 同步安装）；
  `begin_save` 在手势内同步合成 `download://` 目标（正确实现，P3 用）；`write_atomic`
  本阶段显式 `Unsupported`（尚无下载，不翻能力位）；`refuses_source_overwrite` 返回
  `false`（浏览器无文件身份）。`WebConfigStore`：`load_config` 恒 `Absent`（有效配置在内存
  构建），`load_preference`/`persist_preference` 走 `localStorage`。`WebSurface`：CSS 表面
  尺寸 + 安全区（`SURFACE`/`SAFE_INSETS`）。
- `browser.rs` 变为组合根：用上述实现构造 `HostCore`（能力位
  `open=true`、`new/save/plot=false`、`import_worker=false`），以核心作为 `UiCommandSink` 的
  委托目标、`LayoutSwitchSink`、`CanvasPickMapper`、绘制预览 sink；`core.set_handle`/
  `set_view` 后 `core.push()` 承担通用面板推送。`HostRuntime` 改为持有
  `core`/`files`/`incoming`；`current_handle`/`resize`/`set_safe_insets`/`renderer_report`/
  `apply_locale` 全部改经核心（`metrics`/`sync_view`/`push`）。
- 删除的重复编排：`browser/state_push.rs`（`push_panel_state` 与各 `derive_*` 面板映射、
  `WebCanvasPickMapper`、`WebDrawPreviewSink` 全并入/复用 `cad-host`）。保留并缩减为薄适配：
  `input.rs`（`WebSink` 仅拦截 `OpenDrawing`/`SwitchBackend`，其余委托
  `HostCore::execute`；`WebViewInput` 保留浏览器指针/轻触语义与预览光标）、
  `draw.rs`（`WebDrawSink` 保留 Trim/Extend 世界点解析——桌面核心显式拒绝；预览 sink 复用
  核心）、`pick.rs`（仅纯映射/选择拾取辅助，供 `WebViewInput` 使用）、`state_push.rs`
  （仅浏览器叠加：同步回退进度面板 + 资源抽屉字体摘要 + 诊断模型编码）。
- 行为保持点：wasm 导出 23 个不变（`start_web`…`viewer_interaction_json`）；`async_open_poll_json`
  JSON 契约与同步回退（`worker_available()==false`，不 spawn 线程）不变；
  `install_with_preference` 渲染桥、`browser/fonts.rs` Fetch 与 `spawn_font_load` 不变；
  `browser/persistence.rs` 选择器、`shell.rs` 触控、`config.rs` 的 `viewer_*` 导出不变；
  能力位保持 `open=true, new/save/plot=false`。**命令状态文案改由共享目录给出**
  （`linux.completed`/`linux.failed`/`fit.empty`），不再是 Web 内联中文——这是采纳共享漏斗的
  必然结果，已在契约测试与本文档记录。
- Web 无 Rust 定时器：JS 心跳经 `async_open_poll_json` 驱动打开进度；核心 `tick()` 不用于
  Web。`HostCore` 的 wasm 字体/图像装载仍为显式 `Unsupported` 且**不被 Web 调用**（Web 打开
  走自身同步导入 + 浏览器字体路径）。

验证（本机；Web 无头/真实 GPU/真机**未验证**，下列仅为编译与契约证据）：

- `cargo fmt --all -- --check`；`cargo clippy --workspace --exclude app-android --all-targets
  --locked -- -D warnings`；`cargo clippy -p app-web --target wasm32-unknown-unknown
  --all-targets --locked -- -D warnings`（Web 代码此前不在默认门禁，本轮单独跑）。
- `python3 scripts/check-architecture.py`（26 包）；`check-i18n.py`（284 键）；
  `check-fixture-manifest.py`（22 fixture）；`check-workflows.py`。
- `cargo check --workspace --exclude app-android --all-targets --locked`；
  `cargo check -p app-web --target wasm32-unknown-unknown --locked`；
  `cargo check --workspace --lib --target wasm32-unknown-unknown --locked`。
- `cargo test -p cad-host --locked`：7 通过 / 0 失败。
- `node --test scripts/test-web-host.mjs scripts/test-web-touch.mjs
  scripts/test-host-panel-parity.mjs scripts/test-web-responsive.mjs`：**43 通过 / 0 失败**
  （`test-host-panel-parity.mjs` 已改指新结构：linux+web 共用 `crates/cad-host/src/state.rs`，
  android 仍指 `apps/app-android/src/state_push.rs`，未削弱断言）。另
  `node --test scripts/test-web-a11y.mjs scripts/test-web-icon.mjs`（12 通过）、
  `node --test scripts/test-web-pixels.mjs`（1 通过）；`test-web-cache-busting.py`/
  `test-web-backend-fallback.py`/`test-web-deploy-workflow.py` 各 3/4/4 通过。
- `DIST=/tmp/opencode/yacr-web-p1 WITH_FONTS=0 scripts/build-web.sh`：release 链接成功，
  `wasm-bindgen` 导出 23 个（与迁移前一致），wasm 19,171,091 字节。
- 未运行：浏览器/Playwright 视觉、真实 GPU/WebGPU、真机、wasm 运行时执行。

补强（P1 评审后，行为保持）：
- **M1 共享状态文案去 Linux 品牌 + 错误去 Debug 包装**：新增 `command.completed`/`command.failed`
  （两份 catalog 键集一致），`HostCore::execute`（lib.rs）与 `report_nav`（input.rs）改用之；
  新增 `error_text()` 把 `Unsupported(reason)`/`NotImplemented(feature)` 解包为内层字符串（不再
  `Debug` 引号）。删除仅剩未引用的 `linux.completed`（其余 `linux.*` 为桌面管线专用，保留）。
- **M2 布局点击走 Web 漏斗**：`WebSink` 实现 `LayoutSwitchSink`，经 `run_command` 转发 `SwitchSpace`；
  `browser.rs` 安装 `WebSink`（而非裸 `HostCore`），布局点击不再用共享 `push` 覆盖浏览器字体汇总 /
  导入面板，也不再谎报 `images_modeled: true`。
- **M3 运行期语言切换到达共享目录**：`HostCore` 增加**共享** `locale: Rc<RefCell<String>>` 槽 +
  `set_locale`/`messages()`；`browser::apply_locale` 在 `push_state` 前调用 `core.set_locale(tag)`，
  使 completed/failed、fit.empty、layers/properties/layout 空态文案随语言切换（跨 clone 生效）。
- **E 面板 parity 去同义反复**：`test-host-panel-parity.mjs` 新增 Web 专属断言——覆盖
  `apps/app-web/src/browser/state_push.rs` 的 `set_import_state`/`set_resources_sections`/
  `images_modeled: false`，以及 `browser.rs` 调用 `core.push()` + `push_web_overlay` + 安装 Web
  layout sink。
- **F 能力门加固**：`request_new/request_plot/request_save` 在对应 capability 为 false 时显式
  `Unsupported`（不再静默替换文档 / 排队永不导出的 `download://`）；`poll_loading` 的
  `set_save_available` 现尊重 `capabilities.save`。
- 杂项：`HostCore::execute` 不再让 `push()` 失败掩盖命令自身的错误（push 失败仅在命令成功时上浮），
  并在 `push()` 文档注明其 fail-fast（web `push_state` 记日志后继续）。
- 新增 cad-host 单测（fake `PlatformFiles`）：克隆间 `set_locale` 生效；无 capability 的
  New/Plot/Save 被拒绝且无副作用；`error_text` 解包 Unsupported/NotImplemented。补强后
  `cargo test -p cad-host`：**10 通过 / 0 失败**；`check-i18n.py`：**285 键**；
  `node --test …`：**44 通过 / 0 失败**。

## 宿主编排抽取：cad-host（P0，2026-10-11，本轮）

用户要求：把 `apps/app-linux` 的平台公共宿主编排抽到新 crate `cad-host`（`HostCore`），以
小型平台 trait 参数化，`app-linux` **行为保持不变**地迁移；Web/Android 迁移与文件功能为后续
阶段（本轮不动 `app-web`/`app-android` 行为）。

- 新增 `crates/cad-host`（依赖 cad-domain/cad-db/cad-app/cad-platform/cad-representation/
  cad-diagnostics/cad-resources/cad-ui-slint，**不依赖 `slint`**）。`HostCore`（`Clone` 句柄）
  实现 `UiCommandSink`/`LayoutSwitchSink`/`ViewInput`/`CanvasPickMapper`/`DrawCommandSink`/
  `DrawPreviewSink`；文件布局：`lib.rs`/`open.rs`/`save.rs`/`state.rs`/`input.rs`/`fonts.rs`/
  `policy.rs`。
- `crates/cad-platform` 重塑 0 实现草案 `FileAccess` 为事件式、object-safe、无 futures、无
  `Send` 的契约：`PlatformFiles`（`begin_open`/`begin_open_target`/`begin_save`/`write_atomic`/
  `refuses_source_overwrite`/`set_source`/`poll` → `FileEvent`）、`ConfigStore`（`ConfigLoad`
  区分缺失/可读/不可读，保留启动诊断）、`SurfaceSource`、`HostCapabilities`；`FilePurpose`
  覆盖打开/保存的取消与失败（设计草案只列了保存 purpose，扩展以保留打开取消状态）。保留
  `FontLoader` 原样。
- `apps/app-linux/src/host.rs` 变为薄封装：保留 `LinuxApp`/`LinuxOptions`/`FilePicker`/
  `SavePathProvider` 公共 API 与 `entry.rs`（Windows/macOS 薄封装未动）；新增
  `host/platform.rs` 提供桌面 `PlatformFiles`/`ConfigStore`/`SurfaceSource`（ashpd/rfd 选择器 +
  读取 + 原子写 + dev/inode 源保护；XDG/APPDATA/Application-Support；窗口表面零 insets）；
  `host/state.rs`/`input.rs`/`fonts.rs` 移入 `cad-host`；`config_file.rs`/`file_picker.rs`/
  `validation.rs` 保留。
- 行为保持：`last_layout_key` 呈现短路；`tick` 顺序（poll_open → poll_pending_save → metrics →
  sync_view）与单一 error→status 漏斗；每 tick 可用性刷新（`set_new/plot/save_available(!loading())`）；
  `owns_status`（PlotDrawing|SaveDrawingAs）与 `fit.empty`；取消状态（`plot.cancelled`/
  `save.cancelled` ≠ 失败）；`CancelLoading`；`drawing_path` 仅在**成功**打开后武装（源覆盖保护
  随成功安装才 `set_source`）；`set_can_trim/set_can_switch_backend(false)`；`on_config_changed`
  overlay 镜像；`push()` 会**持有 controller 借用**依次调用 `handle.set_*`/`view.*`（既有行为，非本轮
  回归）；其余方法遵循“每方法一次 `borrow_mut`，不在持有 controller 借用时回调 sink/UI”的纪律）。

补强（P0 抽取评审后，行为保持）：
- **F1 打开读取取消 + 进度**：`CancelLoading` 现设置 `open_abort` 并取消异步导入；`deliver_open` 对
  取消后到达的字节**丢弃**（清 `presenting_open`，不安装、不武装源保护）；`poll_loading` 经
  `import_snapshot()` 在“读取中且无异步任务”时合成 `ImportProgressSnapshot::running()`，其
  `cancellable = !open_abort`（取消前可取消、取消后置灰、完成/丢弃后回到 idle）。恢复旧 app-linux
  的 `pending_read.cancelled` 行为。
- **F2 `FileEvent::Opened` 携带 `FileTarget`**：源保护改用平台自身的 `target.opaque`（SAF URI/下载
  句柄等），不再从 `display_name` 反推路径；异步路径经 `open_target` 暂存，导入发布时才 `set_source`。
- **F3 保存忙守卫**：`request_plot`/`request_save` 的忙判断加上 `pending_save.borrow().is_some()`，
  避免已选目标尚未导出时被第二次保存静默丢弃。
- **F4 失败文案**：`on_file_failure` 改为携带 `Cancelled | Failed(String)`，`reason` 已本地化、按原样
  展示（去掉一层 `InvalidInput` 包装）；桌面平台不再预包装，picker 读到缺失文件时恢复
  “drawing path does not exist: …”。
- **F5 死代码**：删除 `config_file.rs` 中重复且无引用的 `load_startup`/`persist_preference`/
  `HOST_ALLOWED_PATHS_JSON`，仅保留路径解析器与文件名常量；allowed-paths 常量只保留 `cad-host` 一处。
- 杂项：`HostPlatform`/`HostCore` 的三个平台契约统一为 `Rc`；`AGENTS.md` 图注明确 `cad-host` 不依赖
  `cad-cli-tools`。新增 `crates/cad-host/src/tests.rs`（fake `PlatformFiles`）：取消后到达的字节不替换
  文档且进度可取消→idle；异步路径暂存平台 target；同步路径以 `opaque` 武装源保护。

验证（本机，Wayland + Quadro P620 真实 GPU；离屏测试用软件 Vulkan lavapipe）：

- `cargo fmt --all --check`；`cargo clippy --workspace --exclude app-android --exclude app-web
  --all-targets --locked -- -D warnings`；`check-architecture.py`（26 包）；`check-i18n.py`
  （284 键）；`check-fixture-manifest.py`（22 fixture）；`check-workflows.py`。
- `cargo check --workspace --exclude app-android --exclude app-web --all-targets --locked`；
  `cargo check --workspace --lib --target wasm32-unknown-unknown --locked`。
- `VK_ICD_FILENAMES=…/lvp_icd.json cargo test -p app-linux --locked`：**16 通过 / 0 失败 / 4 忽略**
  （host_contracts 4、host_config_disk 1、new_drawing 1、plot_export 1、save_export 1、
  save_source_guard 1、verify_ui 1、verify_ui_drawing 1、overlay_grid_toggle 1、lib 单测 4；4 忽略
  为桌面/真实 GPU 用例）。`cargo test -p cad-host --locked`：4 通过（policy 纯单测）。
- 真实 GPU 验收：`cargo test -p app-linux --lib real_gpu_grid_zoom_axes_acceptance --locked --
  --ignored --test-threads=1 --nocapture`（不设 `VK_ICD_FILENAMES`）→ `backend=Some("vulkan")`，
  `grid_changed_pixels=6325`，wheel-up 缩小 world_per_px。证据写 `/tmp/opencode/yacr-real-gpu`。
- 说明：本轮只迁移桌面宿主；Web/Android 未迁移（`app-web`/`app-android` 行为未改），
  `cad-host` 的 wasm 字体/图像装载为显式 `Unsupported`（尚无调用者）。（**P1 已迁移 Web**，
  见本文件顶部；wasm 字体/图像装载仍为 `Unsupported`，Web 走自身浏览器字体路径。）

## 程序级汇总：命令行 / 快捷键 / 绘制编辑（2026-10-10 → 2026-10-11）

本程序（多轮）交付**应用内命令行 + 固定键位 + 一组真实绘制/编辑命令 + 应用内剪贴板**。

- 单一诚实参考：**`docs/shortcuts.md`**（命令行行为、命令/别名表、固定键位、显式未支持清单、
  平台与证据范围）。能力表更新见 `docs/compatibility.md` 的「绘制/编辑命令」小节。
- 命令（均 work-only，一次事务/一步撤销，失败不改状态）：LINE/CIRCLE/ARC、MOVE/COPY/ERASE、
  TRIM/EXTEND（LINE 子集）、ROTATE/SCALE/MIRROR（MIRROR 保留原件）、OFFSET（仅 LINE/CIRCLE）、
  应用内剪贴板 COPYCLIP/CUTCLIP/PASTECLIP（非 OS 剪贴板）。
- **证据诚实**：全部为合成/离屏（软件 Vulkan lavapipe）；`EXTEND` 仅 Web 接线；Android 未编译；
  未做真实 GPU/窗口/真机验收。逐轮明细见下方各轮条目。

## 文档整合 + 命令行离屏场景（2026-10-11，本轮）

用户要求（最后一轮整合）：整合命令行/快捷键程序文档，并加一个离屏 UI 集成场景（**仅文档与测试/
证据**，不加产品功能）。

- 新增 `docs/shortcuts.md`（中文）：命令行前缀匹配/补全/门控行为、完整命令与别名表、固定键位表、
  **显式未支持清单**（F1/F4–F12 与 Ctrl+2/3/4/6/7/8、Ctrl+B/D/E/F/G/H/I/J/L/M/R/T/W、
  Ctrl+Shift+H/A/I/L/P/W 等未实现组合键均报 `shortcut.unsupported`，不再是静默 no-op；OFFSET 仅
  LINE/CIRCLE；Ctrl+C/X/V 仅应用内剪贴板；DWG 导出未验证）、平台与证据范围（桌面为主、EXTEND 仅
  Web、导出/新建桌面专有、Android 未编译、全为合成/离屏）。
- 更新 `docs/compatibility.md`：新增「绘制/编辑命令」小节，列出 LINE/CIRCLE/ARC/MOVE/COPY/ERASE/
  TRIM/EXTEND/ROTATE/SCALE/MIRROR/OFFSET 与剪贴板及各自诚实边界；明确有损 DXF 导出为 `Partial`。
- 更新 `README.md` 文档索引，加入 `docs/shortcuts.md`。
- 扩展 `apps/app-linux/tests/verify_ui.rs` 离屏场景：向命令行输入前缀 `CI` → 断言补全候选包含
  `CIRCLE` → `Enter`（`command-submitted`）解析并启动真实 CIRCLE 捕获 → `Esc` 结束 → 关闭补全后
  候选清空。纯 Slint 状态，不依赖 GPU 读取，稳健于 lavapipe 环境。
- **产品行为变更**：`crates/cad-ui-slint/ui/app.slint` 的 CAPTURE 键处理器新增显式
  `shortcut.unsupported` 分支，覆盖未实现的面板/绘图/对象类 Ctrl 组合键（`Ctrl+2/3/4/6/7/8`、
  `Ctrl+B/D/E/F/G/H/I/J/K/L/M/Q/R/T/U/W`、`Ctrl+Shift+A/C/H/I/L/P/V/W/X`），并为能力门控的
  `Ctrl+O` 增加 `can-open=false` 时的显式 unsupported 分支；这些组合键不再是静默 no-op。
- 相应更新源码契约测试 `crates/cad-ui-slint/tests/shortcut_map_contract.rs`（切片断言新增
  `root.shortcut-unsupported(...)` 分支存在、且 Ctrl+A 选中仍留在 bubbling 处理器）与
  `crates/cad-ui-slint/tests/keyboard_shortcuts.rs`（实际按键，断言本地化
  `shortcut.unsupported` 文案序列、状态栏与“无 CAD 回调”）。

验证（本机，离屏软件 Vulkan lavapipe）：`cargo fmt --all` / `--check`；
`cargo clippy --workspace --exclude app-android --exclude app-web --all-targets --locked -- -D warnings`；
`cargo clippy -p cad-ui-slint --all-targets --locked -- -D warnings`；
`check-i18n.py`（284 键）、`check-architecture.py`（25 包）、`check-workflows.py`；
`cargo check --workspace --exclude app-android --exclude app-web --all-targets --locked`；
`VK_ICD_FILENAMES=…/lvp_icd.json cargo test -p cad-ui-slint --locked`（171 通过 / 0 失败）；
`VK_ICD_FILENAMES=…/lvp_icd.json cargo test -p app-linux --locked`（16 通过 / 0 失败 / 4 忽略；
新增命令行补全断言在 `verify_ui_fixed_scenario_replay` 内，未新增测试函数）。
新增场景为**合成/离屏**证据，不代表真实 GPU、窗口或真机。产品代码变更即上述 `app.slint` 键处理分支。

## 偏移命令：OFFSET（LINE / CIRCLE 子集）（2026-10-11）

用户要求：实现 OFFSET 的**有界、诚实子集**——仅 LINE 与 CIRCLE；其它几何一律显式 `Unsupported`。

- 命令层：新增 `CommandId::OffsetEntity`（work-only）与 `CommandPayload::Offset { refs, side_point }`
  （沿用 MOVE/COPY 的 refs 约定；命令层强制**恰好一个**选中实体）。DrawTool 新增
  `DrawToolKind::Offset`（键 `offset`，1 点，`requires_selection`）与
  `DrawIntent::Offset { side_point }`。
- 手势/语义：选中实体为源，单点 `side_point` 同时定义**侧**与**距离**：
  - LINE：`d` = `side_point` 到无限直线的垂距；新直线 = 原直线沿 `d · 单位法向` 平移（法向指向拾取点
    一侧）；`d <= tol` → 显式拒绝。3D 精确（垂足在直线与点所成平面内）。
  - CIRCLE：新半径 = `|side_point − centre|`（点在外→增大，在内→缩小）；点过圆心或落在圆上
    （零偏移）→ 显式拒绝。
  - 其它几何（Polyline/Arc/Ellipse/Spline/Point/Mesh/Text/…）→ 显式
    `Unsupported("OFFSET supports LINE and CIRCLE only, got …")`，不创建、不部分执行。
  - 0 或多个选中 → 显式 `InvalidInput`；Viewer → 命令层 `PermissionDenied`。
- 新实体：**新 id**，源实体的 layer/space 与显示状态（按对象颜色/线型/线宽 + 动态可见性成员，经
  `entity_display_state`/`restore_entity_display_state` 携带，并写入补丁 `after_state` 使重做保持）；
  单事务 + 单撤销。诊断 `drawing.offset`（含偏移量），绝不空成功。
  - `commit_new_entity` 重构为薄封装，新增 `commit_new_entity_in(layer, space, display, label)` 复用。
- 宿主 sink：linux/web/android 三处转发 `DrawIntent::Offset → OffsetEntity`（与 COPY/MOVE 相同的
  选择 refs 解析）。
- 预览：Offset 并入通用橡皮筋链分支（不伪造偏移幽灵）。
- 命令行：`SELECTORS` += `OFFSET`（work-only）；`ALIASES` += `("O","OFFSET")`；从
  `UNSUPPORTED_COMMANDS` 移除 `OFFSET`；分发到 `begin_draw_tool("offset")`。
- i18n：新增 `draw.kind.offset`（zh `偏移` / en `Offset`）；`draw.status.idle` 增列 Offset。两份 catalog
  键集一致（284 键）。

验证（本机，离屏用软件 Vulkan lavapipe）：`cargo fmt --all` / `--check`；
`cargo clippy --workspace --exclude app-android --exclude app-web --all-targets --locked -- -D warnings`；
`check-i18n.py`（284 键）、`check-architecture.py`（25 包）；
`cargo check --workspace --exclude app-android --exclude app-web --all-targets --locked`；
`cargo check --workspace --lib --target wasm32-unknown-unknown --locked`（含 web lib）；
`VK_ICD_FILENAMES=…/lvp_icd.json cargo test -p cad-app -p cad-ui-slint --locked`（**515 通过 / 0 失败**）；
`VK_ICD_FILENAMES=…/lvp_icd.json cargo test -p app-linux --locked`（16 通过 / 0 失败 / 4 忽略）。
新增合成契约：LINE 偏移（上/下两侧垂距正确、源不变）、点在线上拒绝、空/多选拒绝、Viewer 拒绝、一步
撤销移除 + 重做恢复；CIRCLE 外扩/内缩、过圆心/在圆上拒绝；不支持几何（Arc/Insert）显式 `Unsupported`
且不创建；偏移副本携带源渲染属性；命令行 `OFFSET`/`O` 启动工具与 work-only 目录。均为合成/离屏证据，
未做真实窗口、真实 GPU、真机或交互手势验收；Android 本机缺 Android 平台，app-android 编译未运行。

## 应用内实体剪贴板：Ctrl+C / Ctrl+X / Ctrl+V（2026-10-11）

用户要求：实现应用内（in-app）实体剪贴板——Ctrl+C 复制、Ctrl+X 剪切、Ctrl+V 粘贴；**仅进程内**，
不使用操作系统/系统剪贴板；无跨进程粘贴。

- 命令层：新增 `CommandId::CopyToClipboard`（Viewer 与 Work 均可，只读捕获）、
  `CommandId::CutToClipboard`（work-only）、`CommandId::PasteFromClipboard`（work-only）；payload 均为
  `None`。剪贴板内容存于 `SessionState.clipboard: Vec<DbEntity>`（进程内克隆）。
- 语义：
  - Ctrl+C / COPYCLIP：把当前选择克隆进剪贴板；**不**修改数据库、**不**开事务、**不**记撤销。
    空选择 → 显式 `InvalidInput` 拒绝。
  - Ctrl+X / CUTCLIP：先捕获进剪贴板，再在一次事务 + 一步撤销内删除选择（复用 Phase-5 的删除/patch
    路径，撤销会连同渲染属性与动态可见性成员一起恢复）。空选择在捕获前即显式拒绝。
  - Ctrl+V / PASTECLIP：以**新 id** 在**原坐标**插入剪贴板实体（PASTEORIG 语义，不发明插入点手势），
    单事务 + 单撤销；保留来源 layer/space，并**携带并恢复**每个实体的显示状态（按对象颜色/线型/线宽
    覆盖 + 动态可见性成员），因此粘贴与“剪切的撤销”对称；撤销记录写入 `after_state`，重做也会恢复。
    空剪贴板 → 显式 `InvalidInput` 拒绝。整批粘贴经事务原子校验，任一非法内容则整批拒绝（绝不部分插入）。
  - 剪贴板在文档被替换（New/成功 Open，`HostController::reset_for_new_content` 重建 session）时清空，
    因此粘贴**不可能跨文档**。
- 接线：`app.slint` 新增 `copy-requested`/`cut-requested`/`paste-requested` 回调；在 **bubbling**
  `key-pressed` 处理器内把 Ctrl+C/X/V 从 `shortcut-unsupported` 换成真实回调（聚焦文本编辑器仍获得
  首拒绝权），保留 `keyboard-shortcuts-enabled` 门控，capture 处理器不消费这些组合键。`adapter.rs`
  将三个回调映射到三个 CommandId（错误经 `draw_error_text` 显式显示）。这些在 cad-ui-slint 集中接线，
  全部宿主共享。
- 命令行：`SELECTORS` += `COPYCLIP`（work-only=false）/`CUTCLIP`（true）/`PASTECLIP`（true），分发到
  对应回调；Ctrl+C/X/V 是按键，不额外加键入别名。
- i18n：新增 `draw.error.clipboard_empty`（en `Clipboard is empty: nothing to paste` / zh
  `剪贴板为空：没有可粘贴的内容`）；`draw_error_text` 把稳定的 “clipboard is empty” / “non-empty
  selection” 短语映射到专用键与既有 `draw.error.selection_required`。两份 catalog 键集一致（283 键）。

验证（本机，离屏用软件 Vulkan lavapipe）：`cargo fmt --all` / `--check`；
`cargo clippy --workspace --exclude app-android --exclude app-web --all-targets --locked -- -D warnings`；
`check-i18n.py`（283 键）、`check-architecture.py`（25 包）；
`cargo check --workspace --exclude app-android --exclude app-web --all-targets --locked`；
`cargo check --workspace --lib --target wasm32-unknown-unknown --locked`（含 web lib）；
`VK_ICD_FILENAMES=…/lvp_icd.json cargo test -p cad-app -p cad-ui-slint --locked`（**506 通过 / 0 失败**）；
`VK_ICD_FILENAMES=…/lvp_icd.json cargo test -p app-linux --locked`（16 通过 / 0 失败 / 4 忽略）。
新增合成契约：COPY 只读填充剪贴板（revision/撤销不变）、空选择拒绝、Viewer 允许 COPY；CUT 一步撤销
删除并填充、撤销恢复几何与渲染属性、空选择拒绝、Viewer 拒绝 CUT/PASTE；PASTE 新 id 原坐标克隆、一步
撤销移除全部、空剪贴板拒绝、Viewer 拒绝；`new_blank_document` 清空剪贴板（同一 reset 路径用于 open）；
app.slint/键盘 Ctrl+C/X/V 真实回调 + 文本编辑器首拒绝 + 门控源码契约；命令行 COPYCLIP/CUTCLIP/PASTECLIP
分发与 work-only 目录、Viewer 拒绝 CUT/PASTE。均为合成/离屏证据，未做真实窗口、真实 GPU、真机或
交互手势验收；Android 本机缺 Android 平台，app-android 编译未运行。**未使用 OS 剪贴板**。

补强（评审后）：**F1** 粘贴此前只克隆 `DbEntity`，会静默丢失按对象颜色/线型/线宽覆盖。现剪贴板条目为
`ClipboardEntry { entity, display }`：CAPTURE 时用 `entity_display_state(id)` 快照渲染属性 + 动态可见性
成员，PASTE 后用 `restore_entity_display_state` 恢复，并把 `after_state` 写进撤销补丁，使“重做粘贴”也
恢复（`apply_drawing_patches` 已处理）。**F2** 剪切此前把“已复制”作为首条诊断；现首条为
`drawing.clipboard.cut`（“已剪切 N 个实体（已删除并移入剪贴板）”），删除动作不再被“复制”文案掩盖。
**F3** 剪切改为先捕获到局部、`erase_entities` 成功后才写入 `session.clipboard`，失败的剪切不会破坏旧
剪贴板。**F5** 只读捕获对实例 ref 使用“已按整实体捕获（只读，不影响实例）”，不再谎称“影响所有实例”
（`resolve_selection` 增 `read_only_capture` 标志）。**F6** 删除无生产调用者的 `SessionState::clear_clipboard`
（`reset_for_new_content` 经 `SessionState::new` 已清空）。**F4** 新增测试：命令编辑器聚焦时 Ctrl+C/X/V
不触发 CAD 回调且文本保留（真实 Slint LineEdit 首拒绝）；N=2 粘贴为一步撤销且两 id 均被撤销移除；
粘贴副本保留源渲染属性且 undo+redo 保持。补强后 cad-app+cad-ui-slint **506 通过 / 0 失败**。

## 三个选择变换命令：ROTATE / SCALE / MIRROR（2026-10-11）

用户要求：沿用 MOVE/COPY 的 draw-tool + 单事务/单撤销框架，新增 ROTATE、SCALE、MIRROR 三个选择
变换命令；仅此三项（剪贴板/OFFSET 留待后续）。宿主编译门禁不因 web/android 而回退。

- 命令层：新增 `CommandId::RotateEntities`/`ScaleEntities`/`MirrorEntities`（均 `requires_work_mode`），
  Viewer 由命令层显式 `PermissionDenied` 拒绝。新增 payload：`Rotate { refs, base, angle }`、
  `Scale { refs, base, factor }`、`Mirror { refs, axis_start, axis_end }`（沿用 `Copy { refs, delta }` 形状）。
- 手势语义（`draw_tool`，纯状态机）：
  - ROTATE：工具键 `rotate`，2 点 `[base, direction]`；`angle = atan2(direction−base)`（相对 +X 的绝对角），
    绕 base 在 XY 平面（+Z 轴）旋转。
  - SCALE：工具键 `scale`，3 点 `[base, reference, target]`；`factor = |target−base| / |reference−base|`，
    `reference==base`（零距离）→ 显式拒绝，绝不产生无穷缩放。绕 base 均匀缩放。
  - MIRROR：工具键 `mirror`，2 点 `[p1, p2]`；轴为过 p1、p2 的直线（p1==p2 或零长 XY 轴 → 显式拒绝）。
    **决策：镜像保留原件（AutoCAD 默认）** —— 新增镜像副本、源对象不动。
  - 三者 `requires_selection` 为真（非空选择），均 work-only。
- 变换构造（`draw_tool::rotation_transform`/`scale_transform`/`mirror_transform`）：旋转/缩放用
  `T(base) ∘ M ∘ T(−base)`（`from_basis`+`matrix_mul`）；镜像为负行列式仿射，无法用 `from_basis`
  构造，故用显式 2×2 反射矩阵（`[[ux²−uy², 2uxuy],[2uxuy, uy²−ux²]]`）再平移过 p1。
- 行为：先用 `transform_geometry` 预校验**每个**选中几何；任一不支持（Text/Spline/Shape/Opaque/
  Compound，或 MIRROR 下的 Arc/Ellipse——其参数化会反向）→ 整条命令显式 `Unsupported`，绝不静默丢弃。
  - ROTATE/SCALE：就地更新选中几何，单事务 + 单撤销补丁；保留 layer/space/object/draw_order 与渲染
    属性（`transform_entity` 仅改几何，更新路径不动渲染属性）。
  - MIRROR：同一事务内以新 id 插入镜像副本（源不动），单撤销步骤；对子图元/实例 ref 记
    `drawing.mirror.sub_element`/`drawing.mirror.instance` 诊断，并按对象样式覆盖记 `drawing.mirror.style`。
- 预览：`draw_preview_overlay` 对 Rotate/Scale/Mirror 使用橡皮筋链（rotate 为 base→direction、scale 为
  base→reference→target、mirror 为轴 p1→p2）；不伪造镜像幽灵（预览只有取点，无选择几何）。
- 宿主 draw sink：三者在 linux/web/android 均转发（与 COPY 同为“选择 refs + 参数”，无需取点解析；
  空选择由宿主或命令层显式拒绝）。
- 命令行：`SELECTORS` += `ROTATE`/`SCALE`/`MIRROR`（work-only）；`ALIASES` += `RO`/`SC`/`MI`；
  从 `UNSUPPORTED_COMMANDS` 移除三者；分发到 `begin_draw_tool("rotate"/"scale"/"mirror")`。
- i18n：新增 `draw.kind.rotate`/`scale`/`mirror`；`draw.status.idle` 增列三者。两份 catalog 键集一致
  （282 键）；选择/无交点等复用既有错误键。

验证（本机，离屏用软件 Vulkan lavapipe）：`cargo fmt --all` / `--check`；
`cargo clippy --workspace --exclude app-android --exclude app-web --all-targets --locked -- -D warnings`；
`check-i18n.py`（282 键）、`check-architecture.py`（25 包）；
`cargo check --workspace --exclude app-android --exclude app-web --all-targets --locked`；
`cargo check --workspace --lib --target wasm32-unknown-unknown --locked`（含 web lib）；
`VK_ICD_FILENAMES=…/lvp_icd.json cargo test -p cad-app -p cad-ui-slint --locked`（**494 通过 / 0 失败**；
cad-app 292 + 集成 32；cad-ui-slint 137 + 集成 33）；
`VK_ICD_FILENAMES=…/lvp_icd.json cargo test -p app-linux --locked`（16 通过 / 0 失败 / 4 忽略）。
新增合成契约：ROTATE（绕 base 90° 就地旋转端点正确、一步撤销恢复、空选择拒绝、Viewer 拒绝）；
SCALE（factor 2 使距离翻倍、一步撤销恢复、非正/非有限 factor 拒绝、reference==base 在工具层拒绝、
Viewer 拒绝）；MIRROR（对角轴镜像坐标正确、保留原件 + 一步撤销移除全部副本、Arc/Ellipse 选中整条
`Unsupported` 且不改、一步撤销移除副本、Viewer 拒绝）；变换 ref 的子图元/实例诊断；命令行
`ROTATE`/`RO`、`SCALE`/`SC`、`MIRROR`/`MI` 启动对应工具；`viewer_mode_rejects...` 与 work-only 目录
纳入三命令。均为合成/离屏证据，未做真实窗口、真实 GPU、真机或交互手势验收；Android 本机缺 Android
平台，app-android 编译未运行。

补强（评审后）：**G1** ROTATE 的零长方向（方向点与 base 重合或仅在 Z 上不同）此前会静默退化为 0°；
现按与 MIRROR 相同的阈值 `MIN_DIRECTION_EXTENT=1e-12` 先行拒绝（`InvalidInput`），不再产生假 0°。
**G2** 补齐测试：ROTATE/MIRROR 混合选择 `[line, Opaque]` 整条 `Unsupported` 且支持对象逐字段不变、
revision 不变、`can_undo==false`；ROTATE/SCALE 对不支持图元（Opaque）显式拒绝；多目标 MIRROR 一步
撤销移除全部副本；非原点共轭（绕 (1,0) 旋转 90° 得 (1,−1)-(1,9)；以 x=1 为轴镜像 (0,0)-(10,0) 得副本
(2,0)-(−8,0)）；SCALE/MIRROR 空选择拒绝；`drawing.mirror.style` 诊断。**G3** SCALE 文档注明
`transform_geometry` 的 `det<1e-12` 守卫会拒绝 factor≲1e-4；变换预校验不再把真正的 `InvalidInput`
（如塌缩变换）改标为 `Unsupported`；`DrawIntent::Mirror` 注明仅为 XY 平面轴、Z 原样保留；错误文案与
COPY 的 “COPY cannot transform …” 风格对齐（`ROTATE/SCALE cannot transform …`）。

## 两个真实绘制/编辑命令：ARC 与 EXTEND（2026-10-11）

用户要求：沿用 LINE/CIRCLE/TRIM/draw-tool 框架，新增 ARC（三点圆弧）与 EXTEND（把 LINE 延伸到边界）
两个真实命令；仅此两项（变换/剪贴板/OFFSET 留待后续）。

- 命令层：新增 `CommandId::CreateArc` 与 `CommandId::ExtendEntity`（均 `requires_work_mode`），
  Viewer 模式由命令层显式 `PermissionDenied` 拒绝。新增 `CommandPayload::Extend { target, boundary,
  pick_point }`（与 `Trim` 同形）。
- ARC：`DrawToolKind::Arc`（键 `"arc"`，3 点），`DrawIntent::Arc { points: [Point3;3] }`，
  payload 复用 `Points([start, through, end])`（也接受 `Geometry(Arc)` 直通）。`draw_tool::arc_parameters`
  由三点求外接圆（中心/半径/起始角/带符号 sweep，工作平面 +Z）；共线、重合或非有限点 → 显式
  `InvalidInput` 拒绝且不创建实体。经 `commit_new_entity` 一次事务 + 一步撤销。
- EXTEND：`DrawToolKind::Extend`（键 `"extend"`，与 TRIM 同交互：先目标后边界），
  `DrawIntent::Extend { target_pick, boundary_pick }`。`extend_entity` 复用
  `collect_boundary_segments` + 新增 `line_segment_intersection_param`/`plan_extend`：把拾取端最近的
  LINE 端点延伸到无限目标线与边界段的最近交点。诚实子集同 TRIM：仅 LINE 目标对 LINE/全直线
  LWPOLYLINE 边界；非 LINE 目标、弧段边界、已接触/已穿过边界、无交点均为显式拒绝且不改。一次事务
  + 一步撤销；沿用 TRIM/ERASE 的 patch/apply 路径（更新路径保留渲染属性）。
- 预览：`draw_preview_overlay` 新增 Arc 分支（三点已知则画真实圆弧，否则回退橡皮筋链）；
  Line/Move/Trim/Copy/Extend 共用链预览。
- 宿主 draw sink：ARC 在 linux/web/android 三处转发 `DrawIntent::Arc → CreateArc`；EXTEND 在
  web/android 复用 `world_pick_ref` 解析两点后发 `ExtendEntity`，linux 与 TRIM 一致地返回显式
  `Unsupported`（`linux.extend`，取点未接线）。
- 命令行：canonical `ARC`/`EXTEND`（均 work-only）与别名 `A`/`EX`；从 `UNSUPPORTED_COMMANDS` 移除
  `ARC`/`EXTEND`；分发到 `begin_draw_tool("arc")`/`begin_draw_tool("extend")`。
- i18n：新增 `draw.kind.arc`、`draw.kind.extend`、`linux.extend`；`draw.status.idle` 增列 Arc/Extend；
  `draw.error.no_intersection` 从“无法修剪”改为通用“无法完成操作”（覆盖 TRIM/EXTEND）。两份 catalog
  键集一致（279 键）。

验证（本机，离屏用软件 Vulkan lavapipe）：`cargo fmt --all`；
`cargo clippy --workspace --exclude app-android --exclude app-web --all-targets --locked -- -D warnings`；
`check-i18n.py`（279 键）、`check-architecture.py`（25 包）；
`cargo check --workspace --exclude app-android --exclude app-web --all-targets --locked`；
`cargo check --workspace --lib --target wasm32-unknown-unknown --locked`（含 web lib）；
`VK_ICD_FILENAMES=…/lvp_icd.json cargo test -p cad-app -p cad-ui-slint --locked`（**465 通过 / 0 失败**）；
`VK_ICD_FILENAMES=…/lvp_icd.json cargo test -p app-linux --locked`（16 通过 / 0 失败 / 4 忽略）。
新增合成契约：ARC（三点得到外接圆中心/半径/sweep；共线显式拒绝且无实体；一步撤销移除；Viewer
拒绝）；EXTEND（目标延伸到最近边界、一步撤销恢复原线并保留渲染属性；已接触、非 LINE 目标、
无交点均显式拒绝且不改；Viewer 拒绝）；命令行 `ARC`/`A` 与 `EXTEND`/`EX` 启动对应工具。均为合成/
离屏证据，未做真实窗口、真实 GPU、真机或交互手势验收。Android 本机缺 Android 平台（`No Android
platforms found`），app-android 编译未运行。

验收范围（精确）：ARC 工具在**桌面宿主 + Web** 均接线（`DrawIntent::Arc → CreateArc`）；
EXTEND 仅在 **Web** 接线（`world_pick_ref` 解析两点），**Linux 宿主未接线**（与 TRIM 的
`linux.trim` 缺口一致，返回显式 `Unsupported`）；Android 宿主已改代码但**未编译**。全部证据为
**合成/离屏**（软件 Vulkan lavapipe），不代表真实窗口、真实 GPU 或真机。

补强（评审后）：**F1** `arc_parameters` 原实现的外接圆解与退化判据依赖世界坐标（平移相关），在
大坐标处会误拒合法小弧或算出错误圆心。现改为在 `start` 平移后的局部坐标系求解，退化判据用
平移/尺度不变的相对叉积 `|a×b| ≤ eps·|a|·|b|`（另加重合点守卫），并对 `|z−start.z|>tol` 的非共面
取点显式拒绝（文档化 XY-only 子集，绝不静默投影成错误的 3D 弧）。新增测试：把既有三点夹具平移
(1e6,1e6) 断言中心/半径 1e-9 且三点都在圆上；大坐标单位弧被接受；近满弧；非共面拒绝。**F2**
DXF 导出对负 sweep 弧曾按 `end=(start+sweep)` 直写，等价于镜像到互补弧却记 `Exact`（假成功）。
DXF ARC 是无向点集，现按 `sweep<0` 交换首末角（`start_angle=(start+sweep).rem_euclid(τ)`、
`end_angle=start.rem_euclid(τ)`），同一 `Exact`；新增负 sweep 往返测试断言端点互换且 CCW 跨度为
`|sweep|`。**（已核实）** acadrust 写入 group 50/51 时执行 `.to_degrees()`
（`acadrust` `io/dxf/writer/section_writer.rs`），即其 `DxfArc.start_angle/end_angle` 为弧度；导出器
存弧度**正确**，无需改动。**F3** TRIM 的 plan 级拒绝（退化/无交点/拾取点不在线上）原映射
`Unsupported`，与 `draw_error_text` 文档（no-intersection 走 `InvalidInput`）及 EXTEND 不一致；
现 TRIM/EXTEND 的 plan 级拒绝均为 `InvalidInput`（命中原生 `draw.error.no_intersection`），仅
**不支持的几何**（非 LINE 目标、弧段/CIRCLE 边界）保持 `Unsupported`。**F4** `plan_extend` 的
`crosses` 现与 `touches` 一样无条件拒绝，文档不再夸大。**杂项** `segment_intersection_param` 改为
`line_segment_intersection_param` 的薄封装。新增 EXTEND 契约：grow-start、多交点取最近、
已穿过拒绝、直线 LWPOLYLINE 边界成功、弧段折线/CIRCLE 边界 `Unsupported`；ARC：正 sweep、
`Geometry(Arc)` 直通。

补强后验证（本机，离屏用软件 Vulkan lavapipe）：`cargo fmt --all` / `--check`；
`cargo clippy --workspace --exclude app-android --exclude app-web --all-targets --locked -- -D warnings`；
`check-i18n.py`（279 键）、`check-architecture.py`（25 包）；
`cargo check --workspace --exclude app-android --exclude app-web --all-targets --locked`；
`cargo check --workspace --lib --target wasm32-unknown-unknown --locked`；
`VK_ICD_FILENAMES=…/lvp_icd.json cargo test -p cad-app -p cad-ui-slint -p cad-import-acadrust --locked`
（**652 通过 / 0 失败 / 1 忽略**）；`VK_ICD_FILENAMES=…/lvp_icd.json cargo test -p app-linux --locked`
（16 通过 / 0 失败 / 4 忽略）。

## 两个真实编辑命令：ERASE 与 COPY（2026-10-10）

用户要求：沿用 MOVE/LINE/CIRCLE/TRIM 框架，新增 ERASE 与 COPY 两个真实编辑命令；仅此两项
（ARC/变换/剪贴板留待后续）。宿主编译门禁不因 web/android 而回退。

- 命令层：新增 `CommandId::EraseEntities` 与 `CommandId::CopyEntities`（均 `requires_work_mode`），
  Viewer 模式由命令层显式 `PermissionDenied` 拒绝。新增 `CommandPayload::Copy { refs, delta }`
  （与 `Move` 同形）。
- ERASE：读取**当前会话选择**（无 payload）；空选择 → 显式 `InvalidInput`，不改任何状态。否则
  在一次 `DrawingTransaction` 内按唯一实体删除全部选中项，并记**一条**撤销补丁（整次删除为一步
  撤销）。重复引用同一实体的 ref 折叠为一次删除。
- COPY：复用 MOVE 的交互（`DrawToolKind::Copy`，同样两点 delta，`requires_selection`）。`copy_entities`
  空选择 → 显式拒绝；先对每个选中实体预校验几何可变换（不可变换 → 整个命令 `Unsupported`，
  绝不静默丢弃），随后为每个实体分配**新 id**、按 delta 平移几何、保留 layer/space 与类型，
  在一次事务内插入全部副本并记**一条**撤销补丁。按对象颜色/线型/线宽覆盖无法经现有写入路径
  复制时记显式诊断（副本继承图层样式），不静默改变。
- UI/命令入口：`app.slint` 新增 `callback erase-requested();`；adapter 将 `erase-requested` 映射到
  `CommandId::EraseEntities`（错误经 `draw_error_text` 显式显示）。命令行新增 canonical `ERASE`
  （work-only）与 `COPY`（work-only）、别名 `E`/`CO`/`CP`；从 `UNSUPPORTED_COMMANDS` 移除
  `ERASE`/`COPY`；`ERASE` 分发到 `erase-requested`，`COPY` 分发到 `begin_draw_tool("copy")`。
- 宿主 draw sink（linux/web/android）：`DrawIntent::Copy { delta }` → `CopyEntities`（附会话选择 refs）。
- i18n：新增 `draw.kind.copy`（zh `复制` / en `Copy`）；两份 catalog 键集一致（276 键）。

验证（本机，离屏用软件 Vulkan lavapipe）：`cargo fmt --all`；
`cargo clippy --workspace --exclude app-android --exclude app-web --all-targets --locked -- -D warnings`；
`check-i18n.py`（276 键）、`check-architecture.py`（25 包）；
`cargo check --workspace --exclude app-android --exclude app-web --all-targets --locked`；
`cargo check --workspace --lib --target wasm32-unknown-unknown --locked`（含 web/android lib）；
`VK_ICD_FILENAMES=…/lvp_icd.json cargo test -p cad-app -p cad-ui-slint --locked`
（cad-app 247 通过；cad-ui-slint 171 通过 / 0 失败）；
`VK_ICD_FILENAMES=…/lvp_icd.json cargo test -p app-linux --locked`（16 通过 / 0 失败 / 4 忽略）。
新增合成契约：cad-app ERASE（删除恰为选中项且一步撤销恢复；空选择显式拒绝）与 COPY（副本数量与
平移几何正确、一步撤销移除全部；空选择拒绝；Viewer 拒绝；不可变换几何显式拒绝且不改）；cad-ui-slint
命令行 `ERASE`/`E` 回调与 `COPY`/`CO`/`CP` 启动 copy 工具、源码契约。均为合成/离屏证据，未做真实
窗口、真实 GPU、真机或交互手势验收。

补强（评审后）：**B1** 删除会剪除按实体渲染属性与动态可见性成员关系，撤销记录此前只带 `DbEntity`，
故 UNDO 静默丢样式。现 `DrawingPatch` 增加 `EntityDisplayState`（渲染属性 + `VisibilityMembership`），
`cad-db` 新增 `entity_display_state`/`restore_entity_display_state`（capture/restore），`apply_drawing_patches`
在重新插入时恢复；ERASE 与 TRIM 的整实体删除改用 `drawing_patch_deleted`。**B2** ERASE/COPY 现对
子图元/实例路径 ref 记 `drawing.erase.sub_element`/`drawing.erase.instance`/
`drawing.copy.sub_element`/`drawing.copy.instance`（与 MOVE 同契约），不再静默整实体化。**B3**
`draw.error.selection_required` 改为通用“Select objects first / 请先选择对象”（覆盖 Move/Copy），
`draw.status.idle` 加入 Copy。**B5** `EraseEntities` 现要求 `CommandPayload::None`，错误 payload 显式
拒绝且不改；`chrome.rs` 记录 config ribbon 缺 `draw.copy`（编辑面板有）的漂移。测试补强：ERASE 带非
默认渲染属性 → UNDO 后属性与几何/图层完全恢复；COPY 记 `drawing.copy.style` 且副本属性为默认；ERASE/
COPY 子图元/实例 ref 诊断；ERASE Viewer 拒绝与错误 payload 拒绝；`viewer_mode_rejects_all_drawing_commands_without_change`
与 `drawing_commands_are_work_only_in_the_catalogue` 纳入两命令。

验证（补强后，本机，离屏用软件 Vulkan lavapipe）：`cargo fmt --all`；
`cargo clippy --workspace --exclude app-android --exclude app-web --all-targets --locked -- -D warnings`；
`check-i18n.py`（276 键）、`check-architecture.py`（25 包）；
`cargo check --workspace --exclude app-android --exclude app-web --all-targets --locked`；
`cargo check --workspace --lib --target wasm32-unknown-unknown --locked`；
`VK_ICD_FILENAMES=…/lvp_icd.json cargo test -p cad-app -p cad-ui-slint -p cad-history -p cad-db --locked`
（**536 通过 / 0 失败**）；`VK_ICD_FILENAMES=…/lvp_icd.json cargo test -p app-linux --locked`
（16 通过 / 0 失败 / 4 忽略）。

## 桌面端“另存为 / 导出图纸”（有损 DXF）（2026-10-10）

用户要求：把桌面 GUI 的“另存为 / 导出图纸”接到 Phase 4a 的**有损** DXF 导出核心，接到命令行
与 Ctrl+S/Ctrl+Shift+S。仅 GUI 保存；CLI `export` 子命令另轮。web/android 必须仍可编译且显式
不支持。

- 命令层：新增 `CommandId::SaveDrawingAs`（`requires_work_mode = false`），与 `OpenDrawing`/
  `PlotDrawing` 一样属**宿主自有**（`Application::execute` 返回 `Unsupported`）。
- UI/命令入口：`app.slint` 新增 `in property <bool> can-save: false;` 与 `callback save-requested();`；
  Ctrl+S / Ctrl+Shift+S 在 `can-save` 时发 `save-requested()`，否则 `shortcut-unsupported`。命令行
  `SAVE` 加入 `SELECTORS`（work-mode false），`QSAVE`/`SAVEAS` 作为 `SYNONYMS` 解析到 `SAVE`；三者
  从 `UNSUPPORTED_COMMANDS` 移除；分发时 `!can-save` → 显式 `command.unsupported`。adapter 将
  `save-requested` 映射到 `CommandId::SaveDrawingAs`。`EXPORT` 仍映射到 `PLOT`。
- 业务路径：`HostController::export_dxf()`（cad-app）反向映射当前文档为有损 DXF 字节 + 报告
  （`ExportFormat::DxfText`，模型空间，`ExportLimits::default()`，带 `UnitContext`）；app-linux 只做
  路径选择与写盘（保持宿主薄层，不新增 crate 依赖）。
- 宿主保存（app-linux）：复用离屏保存对话框 worker（`SavePathProvider`/`pending_save`/`poll_pending_save`，
  对话框不阻塞 UI 线程），默认文件名 = 当前文档名 + `.dxf`。按扩展名：`.dxf` → 有损导出核心；
  `.dwg` → 显式拒绝（`save.dwg_deferred`，DWG 写出尚未验证）；无扩展名/其它扩展名显式失败；绝不把
  DXF 字节写进 `.dwg`/其它名字。**覆盖保护**：目标规范化后等于被打开的源文件路径 → 硬拒绝
  （`save.refused_source_path`，防数据损坏）；其它已存在文件依赖系统对话框的覆盖确认。写盘复用
  `atomic_write`。
- 分级状态（按 `ExportReport`）：`save.exported`（converted==0 且 dropped==0）、
  `save.exported_partial`（含 `{converted}`/`{dropped}`）、`save.export_failed`、
  `save.refused_source_path`、`save.dwg_deferred`、`save.cancelled`、`save.picker_waiting`、
  `save.no_extension`、`save.unknown_format`。两份 catalog 键集一致（275 键）。
- 政策：这是**有损另存**，不是往返保存；不宣称 DWG 保存，不新增“已保存/未保存”文档状态，
  不改 `DocumentIdentity`，不重导入输出。

补强（评审后）：**源路径守卫只在打开成功后布防**（`open_path` 不再在尝试时写 `drawing_path`；成功分支与
`poll_loading` 的 `opened` 处布防），失败打开的路径不再移动守卫；`is_source_path` 在 Unix 上优先用
设备号+inode 比较文件身份（覆盖大小写不敏感挂载/符号链接/绑定挂载），否则回退路径比较；分级状态改为
直接采用导出报告的 `completeness`（不再由计数重新推导）；`set_save_available` 每 tick 随打开空闲刷新，
使非无头打开完成后 Save As 可用（不改 `can-plot`/`can-new` 行为）；`handle.rs` 文档说明 `can-save` 在
加载期间也为 false。测试补强：`save_source_guard`（打开 A、尝试解析失败的 B、另存到 A → 拒绝且 A 字节
不变）、`save_export` 增加 `save.no_extension`/`save.unknown_format` 显式失败且不落盘、取消改为
目录清单前后比对并复检源文件字节、部分导出文案由 catalog 派生（不再硬编码“部分”）。

验证（本机，离屏用软件 Vulkan lavapipe）：`cargo fmt --all`；
`cargo clippy --workspace --exclude app-android --exclude app-web --all-targets --locked -- -D warnings`；
`check-i18n.py`（275 键）、`check-architecture.py`（25 包）；
`cargo check --workspace --exclude app-android --exclude app-web --all-targets --locked`；
`cargo check --workspace --lib --target wasm32-unknown-unknown --locked`（含 web/android lib）；
`VK_ICD_FILENAMES=…/lvp_icd.json cargo test -p cad-ui-slint --locked`（171 通过 / 0 失败）；
`VK_ICD_FILENAMES=…/lvp_icd.json cargo test -p app-linux --locked`（16 通过 / 0 失败 / 4 忽略）。
新增合成契约：app-linux `save_export`（打开已提交 fixture 的副本后另存 `.dxf`，断言非空且含
`SECTION`/`ENTITIES`、状态按报告为 exact 或 partial；`.dwg` 显式拒绝且不落盘；无扩展名/未知扩展名
显式失败且不落盘；另存到源文件路径被拒绝且源文件字节不变；取消 → 目录清单不变且 `save.cancelled`
非失败）、`save_source_guard`（失败打开不移动源路径守卫）；cad-ui-slint 命令行
`SAVE`/`QSAVE`/`SAVEAS` 的 `can-save` 门控、Ctrl+S/Ctrl+Shift+S 门控与源码契约。均为合成/离屏
证据，未做真实窗口、真实 GPU、真实保存对话框、浏览器或真机验收。

## 有损 DXF 导出核心模块（导出模块 + 报告 + 文本写入器）（2026-10-10）

用户要求：实现“另存为”式的**有损**导出核心模块：把域数据库反向映射回 acadrust 文档并写出
ASCII DXF，同时给出显式的有损完整性报告。本轮仅核心模块 + 报告 + 测试；**不做 CLI/GUI 接线**
（后续子阶段）。

- 位置（架构约束：仅本 crate 可依赖 acadrust）：`crates/cad-import-acadrust/src/export/`
  （`mod.rs`/`entity.rs`/`tables.rs`/`report.rs`）。
- 公共 API：`ExportRequest { database, document_id, space: ExportSpace::Model, format:
  ExportFormat::DxfText, limits: ExportLimits, units: Option<UnitContext> }`；
  `export(&req) -> CadResult<(Vec<u8>, ExportReport)>` 只返回字节，文件 I/O 由调用方负责。
  `ExportReport { format, counts{exact,converted,dropped,bytes}, entries, notes, completeness }`；
  `MappingOutcome::{Exact, Converted(reasons), Dropped(reasons)}`；`ExportLimits
  { max_output_entities, max_explode_depth }` 超限即显式 `CadError`。
  `completeness` 仅在 `converted == 0 && dropped == 0` 时为 `Complete`（有 drop 记 `Missing`，
  仅 conversion 记 `Partial`）。
- 映射（穷尽、**无 catch-all**，新增 `SemanticGeometry` 变体会编译失败）：Line/Polyline
  （z 常量→LWPOLYLINE 保 bulge；非平面无 bulge→POLYLINE3D；非平面有 bulge→线性化并记 Converted）/
  Circle/Arc/Ellipse/Point/Spline/Text（单行 Exact；含换行→MTEXT Converted）/Mesh（→POLYFACE，
  丢颜色/法线）/Insert（按放置变换展开块成员，深度受限、环检测，丢块身份）/Compound（展开）/
  Shape/Image/Mask/Opaque（显式 Dropped 及稳定 code）。仅导出 `SpaceId::Model`；纸空间实体与
  未被引用的块定义以显式原因计数 Dropped，绝不静默省略。
- OCS 陷阱：Circle/Arc 的 `center` 在域中为 WCS，导出必须用 `arbitrary_axis(normal)^T · center`
  反投影回 OCS，否则倾斜圆被静默位移（已加倾斜法线 + bulge 折线的往返测试）。
- 表与样式：先建图层/线型/文本样式（`CadDocument::new()`、`add_or_replace`），实体经
  `add_entity` 分配句柄（不伪造句柄）；ByLayer 颜色/线型/线宽在导入时已被展平，导出按解析值
  回写（视觉一致、语义变化），并以一条全局 note 说明；缺文本样式名→写 `Standard` + 诊断，
  不产生悬空引用。`UnitContext.source → $INSUNITS`（不可表示时记 note，坐标不缩放）。
- 已知边界：这是**有损另存**，不是往返保存；DWG 写出、纸空间、块定义导出均延后。写出仅经
  `acadrust::io::dxf::DxfWriter::new(&doc).write_to_vec()`（ASCII 文本）。

补强（评审后）：多面网格顶点/面索引超 i16 范围时**显式 Dropped**（`export.dropped.polyface_index_range`），
不再截断成负索引/0 索引而破坏文件；缺失块成员显式 `export.dropped.insert_member`；缺文本样式名的
替换原因折进该实体的 `Converted`（不再同时计 Exact，也不再双重计数）；写出实体透明度（组 440）与
线型比例（组 48）；非默认文本对齐写出对齐点（组 11，导入规则的逆）；MTEXT 写出 attachment_point；
动态块按活动可见状态只导出可见成员并记原因；Spline 改为 `Converted`（域模型无 closed/periodic
标志，"Exact" 仅指对域数据库忠实）；圆弧 end_angle 取模 2π；未引用块定义原因含 block id；空名线型
的 dash pattern 丢弃记原因；`ensure_text_style` 字体首写优先，后续不同字体键记诊断；bylayer 说明
措辞改为“已解析值显式写出，未解析 ByLayer/ByBlock 保持符号”。**M2（缺失块成员）经构建器与事务
校验不可通过公开 API 构造，故该分支为防御性代码，未加直测**（已在源码注释说明）。

验证（本机）：`cargo fmt --all`；`cargo clippy -p cad-import-acadrust --all-targets --locked -- -D
warnings`；`cargo test -p cad-import-acadrust --locked`（**175 通过 / 0 失败 / 1 忽略**，忽略者为
既有用例）；`check-architecture.py`（25 包，边界完整）；`check-fixture-manifest.py`（22 夹具）；
`cargo check --workspace --exclude app-android --exclude app-web --all-targets --locked`；
`cargo check --workspace --lib --target wasm32-unknown-unknown --locked`。新增合成契约：逐类映射
（写出的 DXF 由本仓读者读回 = **内部一致性**，非第三方兼容性结论）、倾斜圆/bulge 折线往返、
三个已提交夹具（a4-layout.dwg、synthetic-four-lines.dwg、flange.dxf）的有损 code 集合硬编码断言
与 `dropped>0 ⇒ completeness != Complete`、Dropped 负向契约、微小程序 DXF 文本确定性，以及本轮
补强测试：多面索引范围、样式回退、透明度/线型比例、对齐文本往返、MTEXT attachment、动态可见性、
插入环、插入深度上限、旋转/缩放插入矩阵顺序、`$INSUNITS`、绘制顺序。**不声称 AutoCAD/ODA 或任何
第三方能打开输出**。

## 矢量打印/导出 SVG + PDF——桌面端（2026-10-10）

用户要求：桌面宿主端到端实现 GUI 打印/导出 SVG 与 PDF（纯 CPU 矢量路径，不经 GPU），
接到命令行与 Ctrl+P；仅 SVG/PDF，DXF 另轮。web/android 必须仍可编译且显式不支持。

- 命令层：新增 `CommandId::PlotDrawing`（`requires_work_mode = false`）。与 `OpenDrawing`/
  `NewDrawing` 一样属**宿主自有**：`Application::execute` 返回 `Unsupported`，由平台宿主接管。
- UI/命令入口：`app.slint` 新增 `in property <bool> can-plot: false;` 与 `callback plot-requested();`；
  Ctrl+P 在 `can-plot` 时发 `plot-requested()`，否则 `shortcut-unsupported("Ctrl+P")`。命令行
  `PLOT` 加入 `SELECTORS`（work-mode false），`EXPORT`/`PRINT` 作为 `SYNONYMS` 解析到 `PLOT`；
  三者从 `UNSUPPORTED_COMMANDS` 移除。分发时 `!can-plot` → 显式 `command.unsupported`，否则
  `invoke_plot_requested()`；adapter 将 `plot-requested` 映射到 `CommandId::PlotDrawing`。
  web/android 不设置 `can-plot`，保持默认 `false`。
- 宿主导出（app-linux）：新增可注入的保存路径选择器 `SavePathProvider`
  （`Arc<dyn Fn(&str)->CadResult<PathBuf>>`，`LinuxApp::new_with_providers`）。真实桌面保存对话框：
  **Linux 用 ashpd 门户 `SaveFile`**（与打开选择器同一 XDG portal 风格，含 `current_name`/过滤），
  **Windows/macOS 用 `rfd::FileDialog::save_file()`**；默认文件名为当前文档名 + `.svg`。测试注入
  临时路径，不弹窗。保存对话框在**工作线程**上运行（复用 `pending_open`/`poll_open` 的 worker +
  定时器轮询模式），不阻塞 UI 线程；取消安全：取消不写文件、不改失败状态。
- 格式按所选扩展名：`.svg`/`.pdf` 显式构建矢量文档
  （`ProviderRegistry::with_default_provider()` + `RepresentationContext`（带桌面字体引擎）+
  `plan_plot_for_record` → `build_paper_space` → `build_vector_document` → `render_svg`/`render_pdf`，
  `enumerate_layouts` 选默认布局：排除 Model、取视口最多者，否则第一个），随后 `atomic_write`；
  **不再走只返回字节、会丢弃 completeness/diagnostics 的 `render_plot_svg`/`render_plot_pdf`**：
  未成形文字、未支持图像等按 `document.diagnostics.len()` 计数，非零时状态为
  `plot.exported_partial`（含“{count} 项未表达”），绝不把部分导出报成干净成功；仅当无丢弃项才
  报 `plot.exported`。`.png` 显式拒绝（GUI PNG 需表面回读，请用 CLI `plot --png`），无扩展名
  用 `plot.no_extension` 显式失败，其它扩展名用 `plot.unknown_format` 显式失败。无布局或空输出
  显式失败，绝不写空成功，也绝不把矢量内容写进 `.png` 名。
- 新增 i18n（两份 catalog 一致）：`plot.exported`、`plot.exported_partial`、`plot.export_failed`、
  `plot.cancelled`、`plot.save_title`、`plot.save_filter`、`plot.picker_waiting`、`plot.picker_empty`、
  `plot.png_cli_only`、`plot.unknown_format`、`plot.no_extension`、`plot.empty_output`、
  `plot.no_layout`、`plot.no_document`。
- 已知漂移风险（P6）：GUI 的 `plot_layout`/`plot_context` 与 CLI 的 `select_plot_layout`/
  `representation_context` 行为等价但为两份实现；提取共享 helper 需落在 `cad-representation`，
  超出本轮范围，故在源码注释中记录而非静默分叉。

验证（本机，离屏用软件 Vulkan lavapipe）：`cargo fmt --all`；
`cargo clippy --workspace --exclude app-android --exclude app-web --all-targets --locked -- -D warnings`；
`cargo check --workspace --exclude app-android --exclude app-web --all-targets --locked`；
`cargo check --workspace --lib --target wasm32-unknown-unknown --locked`（含 web/android lib）；
`check-i18n.py`（266 键）、`check-architecture.py`；
`VK_ICD_FILENAMES=…/lvp_icd.json cargo test -p cad-ui-slint --locked`（171 通过 / 0 失败）；
`VK_ICD_FILENAMES=…/lvp_icd.json cargo test -p app-linux --locked`（14 通过 / 0 失败 / 4 忽略）。
新增合成契约：app-linux `plot_export`（注入保存路径，打开合成 plot fixture `fixtures/plot/a4-layout.dwg`
后导出 SVG/PDF，断言 SVG 含真实描边几何、PDF 含 `stream`/`xref`；`.txt` 未知扩展名显式拒绝且不落盘；
取消返回 `Cancelled` 时无文件、状态为 `plot.cancelled` 而非失败）；cad-ui-slint 命令行
`PLOT`/`EXPORT`/`PRINT` 的 `can-plot` 门控与 Ctrl+P 门控、源码契约更新。均为合成/离屏证据，
未做真实窗口、真实 GPU、真实保存对话框、浏览器或真机验收。

## 新建空白图纸（New）——桌面端（2026-10-10）

用户要求：端到端实现“新建空白图纸”，接到命令行与 Ctrl+N；仅桌面（app-linux）为宿主，
web/android 必须仍可编译且**显式不支持**，不得静默无操作。

- 命令层：新增 `CommandId::NewDrawing`（`requires_work_mode = false`）。与 `OpenDrawing`
  一样属**宿主自有**：`Application::execute` 对其返回 `Unsupported`，由平台宿主接管。
- 控制器：`HostController::new_blank_document()` 构建仅含默认图层 `"0"` 的空库
  （`blank_database()`），以**新的** `DocumentIdentity::Temporary(n)` 原子替换单文档槽，
  并 `reset_for_new_content()` 重建历史/查询/会话（清空选择、图层覆盖、活动工具），
  丢弃旧导入报告。返回显式 `NewDrawing { entities, has_extent }`（空图纸 `has_extent=false`）。
- 空图纸 fit 安全：`Application::fit_viewport` 在无边界时改为**记录在案的空操作**（不改
  相机、不增 generation），不再是错误；不会 panic，也不伪造取景。
- 宿主接线：`app-linux` 在 `execute` 拦截 `NewDrawing` → `request_new()`（替换文档 + 取消
  UI 绘制捕获），并随 `push()` 推 `set_new_available(!loading)`；web/android 不设置，
  `can-new` 保持默认 `false`。
- UI/命令入口：`app.slint` 新增 `in property <bool> can-new: false;` 与 `callback new-requested();`；
  Ctrl+N 在 `can-new` 时发 `new-requested()`，否则 `shortcut-unsupported("Ctrl+N")`。
  命令行 `NEW` 从 `UNSUPPORTED_COMMANDS` 移入 `SELECTORS`（work-mode false），分发时
  `!can-new` → 显式 `command.unsupported`，否则 `invoke_new_requested()`；adapter 将
  `new-requested` 映射到 `CommandId::NewDrawing`。
- 未新增用户可见文案（复用 `command.unsupported` / `shortcut.unsupported`）；两份 catalog
  键集仍一致。

补强（评审后）：New 现在还会**重置活动视口相机**为确定性的新图纸默认平面视图（原点锚定、
`MIN_ORTHO_SCALE`），不再残留旧图纸取景；并**取代进行中的打开**（`import_manager.cancel()`
+ 清空 `pending_open_label`/`pending_open_guard`/`async_open`），`request_new` 增加 `loading()`
守卫，忙时返回显式错误（`linux.busy`）而非竞态。`fit_viewport` 改为返回 `CadResult<bool>`，
空图纸 FIT 由 `fit_drawing` 返回带 `fit.empty` 诊断的 `CommandOutcome`，桌面宿主据此显示
本地化“图形为空，无可缩放范围”而不是泛化的“命令完成”。新增两份 catalog 键 `fit.empty`、
`linux.busy`。`NewDrawing.entities` 改为取自 `drawing.entity_count()`，并移除硬编码英文
`last_status`。

验证（本机，离屏用软件 Vulkan lavapipe）：`cargo fmt --all`；
`cargo clippy --workspace --exclude app-android --exclude app-web --all-targets --locked -- -D warnings`；
`cargo check --workspace --exclude app-android --exclude app-web --all-targets --locked`；
`cargo check --workspace --lib --target wasm32-unknown-unknown --locked`（含 web/android lib）；
`check-i18n.py`（252 键）、`check-architecture.py`；
`VK_ICD_FILENAMES=…/lvp_icd.json cargo test -p cad-app -p cad-ui-slint --locked`（444 通过 / 0 失败）；
`VK_ICD_FILENAMES=…/lvp_icd.json cargo test -p app-linux --locked`（13 通过 / 0 失败 / 4 忽略，
忽略者为既有真实 GPU `--ignored` 用例）。
新增合成契约：cad-app 空白文档/身份/相机重置/取代打开/fit 诊断/应用层拒绝；cad-ui-slint
命令行 `NEW` 与 Ctrl+N 门控；app-linux `new_drawing`（打开真实合成 fixture、建立选择与绘制
捕获后 New 为空且有效，并断言空图纸 FIT 的本地化提示）。均为合成/离屏证据，未做真实窗口、
真实 GPU、浏览器或真机验收。

## 固定 AutoCAD 默认键位映射（2026-10-10）

用户要求：接入一套固定（不可重绑定）的 AutoCAD 默认键位到**已存在**命令/开关，并对尚无
后端能力的按键显式“不支持”，不得静默。

- 键位表放在 `crates/cad-ui-slint/ui/app.slint`（键字符串为 Slint 私有）；Rust 只负责分发。
  新增 `shortcut-unsupported(string)` 回调 + 双语 `shortcut.unsupported`，在 `adapter.rs`
  写入状态栏并展开命令行。
- 视图/UI 开关：F7 网格、F3 捕捉提示标记叠加、F2/Ctrl+9 命令行展开、Ctrl+1 侧栏、Ctrl+0
  净屏（新增 `clean-screen`；布局统一改读 `chrome-visible`，默认等价 `application-ui`）。
- 显式不支持：F1、F4–F6、F8–F12、Ctrl+N/S/Shift+S/P/C/X/V（后续阶段落地）。
- 诚实替代（不夸大）：F2/Ctrl+9 是命令行**展开**，非历史浏览器/命令可见性；Ctrl+1 是侧栏，
  非属性选项板；F3 是捕捉**标记叠加**，非运行 OSNAP；均受 `command-visible`/配置约束。
- 已知限制：净屏（Ctrl+0）会隐藏 chrome，故不支持文案虽写入 `status_label` 却不可见
  （由 `shortcut_map_wiring.rs` 固定，不宣称可见反馈）。
- 门控：整表仍受 `application-ui && keyboardShortcuts` 约束，不吞 Alt/Meta；文本编辑器保留
  Ctrl+Z/Y/A/C/X/V 与 Ctrl+Home 的优先处理。
- 验证（本机）：`cargo fmt --all`；`cargo clippy -p cad-ui-slint --all-targets --locked -- -D warnings`；
  `VK_ICD_FILENAMES=…/lvp_icd.json cargo test -p cad-ui-slint --locked`（171 通过 / 0 失败）；
  `check-i18n.py`、`check-workflows.py` 通过。均为合成/离屏证据，未做真实窗口/GPU/真机验收。

## 端到端巡检：清理失效的导出/导入控件（2026-10-10）

用户要求：对 Linux 客户端做端到端测试，找出可能的 bug、修复并提交。

**本轮验证（真实 GPU 优先 + 离屏契约）**

- **真实硬件 GPU**（本机 Wayland + Quadro P620，运行时不设 `VK_ICD_FILENAMES`）：
  `real_gpu_grid_zoom_axes_acceptance`、`desktop_dwg_event_loop_and_navigation_remain_responsive`、
  `desktop_dwg_open_after_start_and_interaction_remain_responsive` 各自**独立进程**运行**通过**。
  注意：这些 ignored 用例必须一个一个跑；同一进程内第二次创建 winit 事件循环会
  `EventLoop can't be recreated`，属 winit 单例约束，**不是产品缺陷**。
- **全 workspace 测试**（`--exclude app-android --exclude app-web`）**通过**；`app-linux` 全套
  （offscreen）**通过**；`verify_ui`、`verify_ui_drawing`、`host_contracts`、`host_config_disk`、
  `overlay_grid_toggle` **通过**。

**发现的缺陷与修复**

- **导出/导入是“启用但无处理函数”的死按钮**：`app.slint`（顶栏）与 `ribbon.slint`（“文件”页签）
  仍保留导出（⇧）/导入（⇩）按钮，`can-export`/`can-import` 默认 `true`，但全仓库**没有任何**
  `on_export_requested`/`on_import_requested` 处理函数——点击静默无响应；其标签属性
  `export-label`/`import-label` 也从未被赋值（空文本）。批注导入/导出早已随标注能力从产品移除
  （见下方“标注删除”轮），这里是清理遗漏，违反铁律 2（未支持能力须显式建模，禁止静默空操作）。
  **修复**：删除 `export-requested`/`import-requested` 回调、顶栏与 ribbon 文件页签的按钮、
  `cmd-export-visible`/`cmd-import-visible`/`can-export`/`can-import` 属性及其向 `CadRibbon`
  的透传，以及不再可达的 `action(10)`/`action(11)` 分支；`labels` 数组删除两项，
  ribbon 中诊断按钮由 `labels[11]` 改为 `labels[9]`。新增 `cad-ui-slint` 单元测试
  `shell_has_no_dead_export_import_controls` 防回归。

**覆盖补强（无缺陷，锁定行为）**

- `apps/app-linux/tests/overlay_grid_toggle.rs` 由“仅网格”扩为在**单进程内**同时校验网格与
  **坐标轴**两个状态栏叠加开关真正到达渲染器（开启/关闭双向、仅画布矩形的像素比对）。
  离屏 Slint 平台的 rendering notifier 是进程级单例（同进程第二个 `LinuxApp::show()` 会
  `set_rendering_notifier failed`），故两个开关合并为一个测试、共享一个 `LinuxApp`。
  结果：坐标轴开关与网格同类，均可达渲染器。

**未修复 / 已知项（本轮未处理，记录待评估）**

- 配置字段 `ui.components.navigationToolbar.commands` 被解析却从未被读取（navigation 工具栏只
  消费 `visible` 布尔）——属静默忽略配置数据的待设计项。
- `apps/app-linux/src/host/input.rs` 的 MOVE 在提交时读取**当前**选择，而非工具启动时捕获的引用
  集合；拖拽期间选择若变化会移动不同集合（当前 UI 难以触发）。
- 50ms 定时器把 `metrics()` 的 `Cancelled`（窗口消失）当失败并写 `linux.failed` 状态，关机路径
  可能出现短暂的伪造失败文案。

**门禁**（本机）：`cargo fmt --all -- --check`；`cargo clippy --workspace --exclude app-android
--exclude app-web --all-targets --locked -- -D warnings`；`check-architecture.py`、
`check-fixture-manifest.py`、`check-workflows.py`、`check-i18n.py`；
`cargo check --workspace --exclude app-android --exclude app-web --all-targets --locked`；
`cargo check --workspace --lib --target wasm32-unknown-unknown --locked`（含 app-web/app-android
lib）均通过。

## Linux 客户端端到端验证三项修复 + 真实 GPU 验证（2026-10-10）

用户要求：基于当前开发环境用 **Linux 客户端打开文件**做端到端验证；其手动运行发现三处缺陷
（网格开启不显示、滚轮缩放方向相反、十字/坐标轴左边短）。用户随后要求**验证优先真实硬件 GPU**
（本机为 Wayland 桌面 + Quadro P620，非无头 LXC），并同步更新 `AGENTS.md`。

**根因与修复**

1. **网格开关不生效**（`apps/app-linux/src/host.rs`）：桌面状态栏的 overlay 切换只改了共享配置
   存储与按钮态，`CadView` 的 `overlay_visibility` 仅在 `Runtime::push()`（启动/开图）里被镜像；
   配置变更后没有 `push()`，网格批次从未重建。修复：宿主安装 `on_config_changed` 观察者，在真实
   配置变更时把 `config.view.overlays` 镜像到 `view.set_overlay_visibility`（观察者拿到配置本身，
   不在 `publish` 期间回借存储）。Web/Android 本就在各自 state-push 漏斗里同步，桌面此处遗漏。
2. **滚轮方向相反**（`apps/app-linux/src/host/input.rs`）：桌面滚轮经 Slint 的 winit 后端到达，
   winit 文档「正值=内容下移/露出上方」即**正 `delta_y`=上滚**；原式 `1 - dy*0.0015` 使上滚=缩小。
   改为 `1 + dy*0.0015`（上滚=放大）。Web（浏览器 `deltaY` 向下为正）与 Android 保留各自公式，未改。
3. **坐标轴十字左边短**（`crates/cad-app/src/render_scene/overlay.rs`）：`axes_overlay` 原按单侧
   模型 bounds 裁剪（X 跨 `[min.x,max.x]`、Y 跨 `[min.y,max.y]`），原点偏向一侧时一侧臂明显更短
   （软件截图实测法兰：左 120px vs 右 387px，上 113 vs 下 362）。改为**关于原点对称**
   `[-extent, extent]`（每轴取较大绝对边界），两臂等长，读作真正的十字。

**契约测试**

- 新增 `apps/app-linux/tests/overlay_grid_toggle.rs`：生产 `LinuxApp`（offscreen）里状态栏切网格，
  断言**仅 CAD 画布矩形**像素变化、关闭后逐字节还原（避免状态栏按钮高亮造成假通过）。
- 新增 in-crate `host::input::tests::desktop_wheel_up_zooms_in`：真实 Slint 组件派发
  `PointerScrolled delta_y=+120`，断言 `world_per_px` 变小（上滚放大）。
- 更新 `axes_overlay_draws_a_symmetric_origin_cross_below_the_highlight`。
- 新增 in-crate `host::validation::tests::real_gpu_grid_zoom_axes_acceptance`（`#[ignore]`，
  **真实 GPU**）：真实 winit 窗口 + 硬件 Vulkan，切网格、上滚并写 PNG 证据。

**真实 GPU 端到端验证（本机，2026-10-10）**

- `cargo test -p app-linux --lib real_gpu_grid_zoom_axes_acceptance --locked -- --ignored`
  （不设 `VK_ICD_FILENAMES`；`GpuSelection::Auto`→`HighPerformance`）：**通过**，
  `backend=vulkan`、`grid_changed_pixels=6325`、`world_per_px 0.3208 → 0.2718`（上滚放大）。
- 直接运行客户端 `target/debug/yacr-linux --open fixtures/dxf/qcad-flange/flange.dxf`：
  `nvidia-smi` 显示该进程占用 **53 MiB**（Quadro P620）——确认真实硬件，非软件渲染。
- PNG 证据 `/tmp/opencode/yacr-real-gpu/{01-axes-initial,02-grid-on,03-wheel-up}.png`。
  视觉分析确认（真实 GPU 像素测量）：切换网格后新增网格线（RGB 107,115,128，间距≈62.4px，
  6325px）；坐标轴十字由修复前的**左 120px / 右 387px** 变为**左 386px ≈ 右 387px**（X 轴已对称，
  左端被画布左缘裁剪）；上滚后几何放大约 **1.18×**。竖直臂上端仅 125px 是**画布顶缘裁剪**（视口
  裁剪，非模型裁剪），下端自由端接近画布底——即对称的长轴被视口截断，而非"上短"。
- 回归：`cargo test -p app-linux --test verify_ui`、`host_contracts`、`overlay_grid_toggle`、
  `desktop_wheel_up_zooms_in` 均通过；`cargo test -p cad-app --lib render_scene` 29 项通过；
  `cargo fmt --all -- --check`、`clippy -D warnings`（cad-app + app-linux）通过。

**AGENTS.md**：新增「渲染/端到端验证优先真实硬件 GPU」规则；`真实 DWG 回归`与无头 UI 循环改为
**无显示环境的回退**。

**未运行/限制**：未用真实 DWG（用提交的 flange DXF fixture）；**web/Android 滚轮方向未验证**（只改
桌面路径）；Windows/macOS 桌面共享同一 winit 后端公式但未真机验证；offscreen/lavapipe 契约测试保留
为 CI/无显示环境回归，其证据已标注为软件渲染。

## DXF/DWG 图元显示补全轮（2026-10-10）

用户要求：检查项目对 DXF 与 DWG 各 entity 的实现情况，**除 proxy 代理图元外实现所有 DXF
entity 的显示**，并把进度与实在实现不了的项记入本文。经问询确认范围：**含光栅图像纹理
管线**、**非标准 class-based（Extended）图元也要基础表示**。铁律不变（不改 acadrust、
不用 Cargo patch、显式 `Unsupported`/`Partial`、不造假成功）。

**结论**：`crates/cad-import-acadrust` 的 `convert` 已覆盖全部 acadrust `EntityType`
变体；本轮把此前仅 `Opaque`/无显示或 `Partial` 的图元补齐/提升，并新增**光栅图像端到端
显示**（domain→importer→representation→scene→renderer→resources→host）。DXF 与 DWG 共用
同一导入路径，故两格式同步受益。

### 改动

1. **光栅图像（`IMAGE`/`AcDbRasterImage`）端到端显示（新增，跨 7 层）**
   - `cad-domain`：`SemanticGeometry::Image{origin,u,v,pixels,file,clip,visible}`、
     `ImageClip`、`SemanticGeometry::Mask{boundary,inverted}`、`Transform3::from_basis`。
   - `cad-resources`：`decode_image`（PNG+JPEG→RGBA8，依赖 `png`+`zune-jpeg`；**不用**
     `image` crate，其 MSRV 1.88 > 工程 1.85）、`DecodedImageCache`、
     `image_resource_key`（小写化**相对路径**键，避免 `a/logo.png`/`b/logo.png` 同名碰撞）；
     能力表 `Image` = resolve `Verified` / decode `Partial`；新增码
     `resource.image_unsupported_format`、`resource.image_decode_failed`。
   - `cad-import-acadrust`：`raster_image_semantics` 发 `Image`（`file` 为逻辑键、`clip`
     像素空间、`visible`=SHOW_IMAGE）；`wipeout_semantics` 发 `Mask`（仅边界）。
   - `cad-representation`：provider `Image` arm → `DisplayPrimitive::Image`，把 `ImageClip`
     映射为世界坐标+UV 的裁剪多边形（2 角矩形展开为 4 角，**仅凸多边形**）；缺纹理→frame+
     `Partial`+`image.unresolved`；不可表示裁剪（outside/退化/**非凸**）→frame+`Partial`+
     `image.clip_unsupported`。纸空间视口裁剪与实体裁剪**精确求交**（Sutherland–Hodgman，
     UV 插值）；防御性非凸实体裁剪保留原裁剪并报 `Partial`+`viewport.clip_image_intersection_unsupported`。
   - `cad-scene`：独立 `ImageBatch` 列表（不混入 `RenderBatch`），按来源失效、独立预算、
     `image_evictions()` 计数（记录驱逐数；当前仅 `cad-scene` 暴露，UI 尚未消费）。
   - `cad-render-wgpu`：`shaders/image.wgsl` + texture/sampler bind group；每 `ResourceKey`
     共享一张 GPU 纹理（`image_texture_count()`）；UV 方向修正（`uv.y=1-v`）；消费裁剪
     多边形；纹理字节计入帧预算；**仅在清屏帧绘制**（渐进分页不重复合成）。
   - `cad-platform`：`requested_images` + 安全图像加载器（相对图纸目录/`--images-dir`、
     拒绝绝对路径与 `..` 逃逸、缺失/失败上报，不插入占位）；`cad-app` 新增
     `build_scene_with_space_and_images`；CLI render/plot 上传并输出 `images` JSON；
     桌面 `cad-ui-slint`（`CadView::set_images`→`PreparationInput`→controller→
     `PreparedScene`→runtime→`upload_images`）与 `app-linux`（`install_images`）接线。
2. **图元保真补全（`cad-import-acadrust`）**
   - **TABLE**：按 `CellStyle`/`CellBorder` 逐边绘制（`invisible` 跳过、`double_spacing`
     双线、合并抑制内边）；`Partial`（逐边颜色/线宽、`override_flags`、`border_type` 未应用）。
   - **非 proxy `Extended`**：`RTEXT`→Text（flags/非 ±Z OCS 法向未处理则 `Partial`）、
     `ArcAlignedText`→Text（`Partial`：按直线近似）、`GeoPositionMarker`→半径圆+注记+内嵌
     MText（`Partial`）、`SectionObject`→折线（`Partial`）、`PointCloud/Ex`→范围盒
     （`Partial`：无点数据）；`Camera`/动态块参数与夹点实体/`CoordinationModel`/`OleFrame`/
     `LayoutPrintConfig`/`Format`/`Legacy`/`RegisteredClass` 保持 `Opaque`/`Unverified`（按设计
     无显示或外部内容）。
   - **MLINE**：从 `doc.objects` 的 `MLinestyle` 解析元素偏移/线型/`flags`，绘制
     justification/scale 基准下的元素、caps（方/圆近似）与 joins；`Partial`。
   - **DIMENSION**：`LargeRadial` 用 `jog_angle` 定向（`Partial`）；`Arc` 画引线且
     `is_partial`→`Partial`；线性/对齐/半径/直径/角度/坐标/弧长为 `Complete`。
   - **MULTILEADER**：`MLeaderStyle` 解析、箭头三角、`path_type`（Straight/Spline/Invisible）、
     dogleg/landing、文本附着枚举；`Partial`。
   - **HATCH 边界**：`SplineEdge` 改用 `tessellate_spline`（尊重 `knots`/`weights`，含回退）、
     `PolylineEdge` 展开 `.z` bulge。
   - **TOLERANCE**：按 `text_lines()` 行数定框高（宽度仍按字符估计，`Partial`）。
3. **契约测试**：`crates/cad-import-acadrust/src/tests_display_{table_extended,mline_tolerance,
   dimension_multileader,hatch_boundary}.rs`（约 40 项），及 resources/platform/representation/
   scene/render-wgpu 各自新增测试。

### 验证（实际执行，2026-10-10）

- **静态门禁全绿**：`cargo fmt --all -- --check`、严格 `clippy -D warnings`、
  `check-architecture.py`（25 包无环）、`check-fixture-manifest.py`、`check-workflows.py`、
  `check-i18n.py`（248 键）；`cargo check --workspace --exclude app-android --exclude app-web
  --all-targets`、`cargo check --workspace --lib --target wasm32-unknown-unknown`、
  `cargo check -p app-web --target wasm32-unknown-unknown` 均通过。
- **测试（lavapipe 串行）**：`cad-import-acadrust` **150**、`cad-representation` **164**、
  `cad-resources` **34**、`cad-scene` **54**、`cad-render-wgpu` **72**（含 4 个新 GPU 用例：
  两色朝向、裁剪多边形、同键纹理去重、渐进分页只画一次）、`cad-app` **265**、
  `cad-ui-slint` **159**、`cad-platform` **23**、`cad-cli-tools` 等；除下述既有失败外全绿。
- **光栅图像渲染证据为 lavapipe（软件 Vulkan）**，不是真实 GPU；见 `docs/validation.md`
  本轮小节。

### 既有失败修复（非本轮引入，已在干净 HEAD `37b494d` worktree 复现）

`cad-cli-tools/tests/dxf_fixture.rs` 两项在 HEAD 即失败，本轮一并修复（根因是**既有测试
相对后续行为变更过期**，非图元显示范围）：

- `committed_qcad_flange_builds_a_non_empty_representation`（期望 `kind_counts.texts>=6`）：
  字体轮（2026-10-09）后 CLI 自动加载默认轮廓面，`build-representation` 把文字成型为线
  几何，`texts` 恒为 0。修复：`build-representation` 改为**与宿主字体无关**（仅显式
  `--font` 才成型，`render`/`plot` 不变），flange 实测 `texts=7`，测试恢复通过。
- `flange_plot_defaults_to_the_populated_paper_layout`（期望 297×210/旋转 0）：acadrust
  0.6.3 升级后应用 LAYOUT `group 72`/`group 73`，该样本声明 `group 73=1`（旋转 90°），
  按既定规则（仅当文件未声明旋转时才轴交换）应保留纵向 210×297 + 旋转 90。修复：测试
  期望更新为该行为（`docs/cli.md`、`docs/dxf-entity-coverage.md` 同步）。

修复后 `cargo test -p cad-cli-tools --test dxf_fixture`（lavapipe）**3/3 通过**。

### 未运行 / 限制

- 真实 GPU、真机、浏览器、**真实光栅 DWG/DXF 的视觉验收** NOT RUN；朝向与「u=每像素
  向量」的放置约定仅内部自洽，未经真实文件核对。
- Web/Android 宿主**未接图像加载器**（`cad_platform::images::local` 为
  `cfg(not(wasm32))`）；桌面仅 `app-linux` 与 CLI 已接。
- 图像裁剪在显示层仅支持 inside 多边形；outside/mask 裁剪、亮度/对比/淡出/透明未应用；
  TIFF/CCITT/EPS 等编解码器→显式 `image_unsupported_format`。
- **`draw_order` 未贯通**（`DbEntity→DisplayFragment→RenderBatch` 本轮正式推迟，见
  `cad-scene` 注释）；故绘制顺序=上传顺序，WIPEOUT 无法按序覆盖。
- **WIPEOUT 掩码填充未实现**：仅画边界并 `Partial`，`inverted` 未被渲染消费；透明实体
  覆盖顺序洞仍在。
- 桌面路径的 representation 级 `image.*` 诊断未上抛到 UI（仅 host loader 级上报）。
- 多数图元仍为保真 `Partial`（详见 `docs/dxf-entity-coverage.md` §4）。

### 实在实现不了（显式记录，不伪造）

- 代理 opcode 厂商记录（本轮按用户要求**排除**，仍 `Unsupported`，见 `docs/proxy-support.md`）。
- 外部内容：PDF/DWF/DGN underlay、OLE2 嵌入对象、`CoordinationModel`/Navisworks NWD。
- ACIS 全内核（`3DSOLID`/`REGION`/`BODY`/`SURFACE` 仅子集离散，无几何内核）。
- 点云点数据（acadrust 0.6.3 未暴露；仅范围盒 `Partial`）。
- 完全顺序正确的 WIPEOUT 透明覆盖掩码（受渲染批 alpha 分类限制）。
- 不支持的栅格编解码器（TIFF/CCITT/EPS 等）。

图元逐项状态见 `docs/dxf-entity-coverage.md`；能力矩阵见 `docs/compatibility.md`。

## 应用图标接入轮（2026-10-10，本轮）

用户要求「使用 assets 目录中的图标」：先从其 `dev.snakeheartgo.top:~/sources/yacr/assets/`
把 `yacr-icon.svg`（SHA-256 `d380e3e3…`）拷入本仓库 `assets/`，再让五个 app 全部使用它。
新增 `docs/app-icon.md` 作为图标单一来源与各端接入说明。

**改动**

1. **单一来源 + 派生资产**：`assets/yacr-icon.svg` 为唯一来源；`scripts/generate-icons.py`
   用 `rsvg-convert`+Pillow 生成 `assets/yacr-icon.{ico,icns}`、`assets/icons/yacr-*.png`
   与 `apps/app-android/res/`（传统 + 自适应启动图标），`--check` 逐字节防漂移（已通过）。
2. **桌面**：`cad-ui-slint` 的 `YacrWindow` 设 `icon: @image-url("../../../assets/yacr-icon.svg")`
   （编译期嵌入；winit 后端设置窗口/任务栏图标）。新增 `tests/app_icon_contract.rs`。
3. **macOS 包**：`package-macos-release.sh` 复制 `Yacr.icns` 并写 `CFBundleIconFile`。
4. **Linux 包**：staging `share/icons/hicolor/{scalable,256x256}/apps/yacr.{svg,png}` 与
   `share/applications/yacr.desktop`（`Icon=yacr`）。
5. **Windows 包**：`apps/app-windows/build.rs`（`winresource`）把 `assets/yacr-icon.ico`
   编入 `yacr.exe` 的 PE 资源（MSVC `rc.exe` / GNU `windres`，非 Windows 目标 no-op）；
   另附 `yacr.ico`。`package-windows-release.sh` 用 `check-pe-imports.py --has-icon`
   断言资源存在。
6. **Android**：`[package.metadata.android] resources="res"` +
   `[package.metadata.android.application] icon/label`（配置 schema 依据 cargo-apk 0.10.0
   与 `ndk-build` `AndroidManifest` 源码核对）。
7. **Web**：SVG favicon + `manifest.webmanifest` + `apple-touch-icon`；`build-web.sh`
   拷入 `assets/` 与 manifest。
8. **门禁接线**：`scripts/test-web-icon.mjs`（接入 `build.yml`）、
   `scripts/test-android-icon.py`（接入 `core.yml` workflow-contracts）；三个
   `test-package-*-release.py` 增加图标保证。

**验证**：已运行 `cargo check -p cad-ui-slint`、`app_icon_contract`（2 passed）、
`node --test scripts/test-web-icon.mjs`、`test-android-icon.py`、三个包契约、
`generate-icons.py --check`、`check-workflows.py`；Windows 图标另经
`cargo build -p app-windows --target x86_64-pc-windows-gnu` +
`check-pe-imports.py --has-icon` 验证（本机 GNU 交叉）。**未运行**：真实窗口/Wayland 图标、
Android APK 构建与真机、macOS `.app`（MSVC `.exe` 由 CI windows-release 构建时验证）。
详见 `docs/app-icon.md`。

## UI item 5 收尾轮（2026-10-09/10，本轮）

用户要求继续 handoff 收尾；问询确认范围为「全量 UI item 5」：布局面板（纸空间/视口
裁剪与比例）、资源/3D 抽屉、安全区/软键盘并入坐标映射，另含可收口小项（plot-style
显式建模、批注移除文档债）。四条并行工作流 + 主控集成验证。

**改动**

1. **纸空间视口裁剪闭环**（`cad-representation`）：mesh 逐三角形 Sutherland–Hodgman
   裁剪（z/法线/逐顶点 sRGB 颜色插值）、图像四边形 UV 插值裁剪
   （`DisplayPrimitive::Image` 新增可选 `clip`）；未成形 Text/坏 mesh/退化图像等
   不可裁剪情形**保留几何 + `Partial` + 稳定原因码**（`viewport.clip_*`）。
   15 项新契约；`docs/layouts.md` §2.4/§4/§5 同步。
2. **UI 抽屉与布局比例**（`cad-ui-slint`）：资源抽屉（字体/导入/代理/引用/图像真实
   分区，无数据显式空态，图像未建模显式说明）、3D 观察抽屉（复用既有
   `Switch2d3d`/`SwitchProjection`/`StandardView` 命令；环绕=手势说明；3D fit 显式
   禁用）、布局比例**显示级**（`LAYOUT_SCALE_CONTROL_WIRED=false`，命令面不存在
   不伪造）。i18n **202→248 keys**，含 `diagnostic.representation.viewport_clip_partial`、
   `diagnostic.import.plot_style_unsupported`、`diagnostic.import.shade_plot_unsupported`。
3. **plot-style 显式 Unsupported**（`cad-import-acadrust`，铁律 2 补齐）：
   `PlotSettings.current_style_sheet`/`Layout.plot_style_sheet`/`shade_plot_mode` 首次
   被读取并以 `import.plot_style_unsupported`/`import.shade_plot_unsupported` 显式
   建模（此前**完全未读取、静默忽略**，与文档"显式 Unsupported"声称不符）。5 项契约。
4. **批注移除文档债**：`CAD_IMPLEMENTATION_SPEC.md`、`docs/architecture.md`、
   `docs/core-invariants.md`、`docs/compatibility.md`、`docs/code-audit-and-agent-handoff.md`、
   `docs/panels.md`、`docs/ui-redesign.md`、本文 7 项清单的 F07/F08/F09 与
   `cad-annotations` 引用按「产品已移除（2026-10-05）」重写；历史证据段保留并标注。
5. **宿主接线**（`apps/app-linux|app-web|app-android`）：三宿主安装
   `LayoutSwitchSink`（走 `CommandId::SwitchSpace` 校验路径，未知布局显式拒绝）；
   `cad_app::input::inset_canvas_metrics` 把安全区折入 `CanvasMetrics`（退化输入
   显式拒绝；Web 经 `env(safe-area-inset-*)`、Android 提供 `set_surface_insets` 入口、
   Linux 显式零）；资源/3D 抽屉真实数据推送（桌面 `fonts: None`：加载器丢弃
   `FontLoadReport`，显示显式空态而非臆造摘要）。`docs/input.md`/`docs/responsive-ui.md`
   同步。
6. **主控修复潜伏测试缺陷**：`app-android` 的
   `android_view_input_scroll_zooms_the_camera` 断言方向与三宿主共享公式
   （`factor=1-dy*0.0015`）及 `Camera::zoom_at`（factor>1 zooms in）契约**相反**——
   该测试不在任何已记录门禁内、从未运行过，属潜伏缺陷非本轮回归。改为双向断言
   （scroll down=zoom out、scroll up=zoom in）并修正 `view.rs` 错误注释，属对齐
   契约而非弱化。

**门禁（本机 ThinkPad 桌面；GPU 测试 lavapipe 串行）**：fmt、clippy(`-D warnings`)、
architecture、fixture-manifest、workflows、i18n(248) 全绿；workspace all-targets
check、wasm32 全 workspace lib check、app-web wasm check 全绿；node 六套契约全绿；
`cargo test`：cad-app 263、cad-representation 157、cad-import-acadrust 111/0/1 ignored、
cad-ui-slint 131+全部集成（含 offscreen 外壳渲染）、app-linux 全部（含 verify_ui/
verify_ui_drawing）、app-web 全部、app-android **19/0**（修复后）。
**Android 门（本轮补装 SDK 后实跑）**：本机原缺 Android SDK/NDK；已按 `docs/build.md`
布局安装（`~/android-sdk`，build-tools 34.0.0 / platforms 34+30 / NDK 27.0.12077973，
`~/jdk17`，env 写入 `~/.bashrc`），随后
`cargo check --target aarch64-linux-android -p cad-ui-slint -p app-android --all-targets`
与同目标严格 clippy（`-D warnings`）**通过**（顺带清掉两处 app-android 遗留问题：
`poll_import_once`/`ImportPollOutcome` 再导出的 cfg 限定为 android+test，
`lib.rs` 两处 `clone_on_copy`）。

**未运行/限制**：真实 DWG/真实 GPU 像素验收/浏览器/真机 NOT RUN（lavapipe/合成为限）；
Web 生产部署仍待 Cloudflare token；Android Activity 侧 OS inset 回调**未转发**到
`set_surface_insets`（真机安全区未生效，显式未接线）；3D fit、视口比例命令面、
图像实体建模仍显式缺失（见 `docs/panels.md` §3.4）；`cargo-apk`/APK 打包与模拟器
不在本轮（SDK/NDK 已就绪，可按 `docs/build.md` 后续执行）。中途两次用户中断/恢复，
被取消的工作流残留改动已由主控就地核对合并，无回滚。

## acadrust 0.5.5 → 0.6.3 升级（2026-10-09，本轮）

用户要求把工作区 `acadrust` 从 `=0.5.5` 升到 `0.6.3`（features `serde` + `import`），
修 API 破坏并跑绿门禁，随后同步文档与契约测试。

**改动**

- `Cargo.toml:61`：`acadrust = { version = "0.6.3", features = ["serde", "import"] }`；
  `Cargo.lock` 由 `cargo update -p acadrust` 刷新（新增 `quick-xml 0.36.2`、`foldhash`、
  `serde`；`thiserror 1→2`；去掉 `ahash`）。
- 唯一编译破坏：`EntityType::Surface` 现为 `Box<Surface>`（0.6.0 同时 box 了
  Helix/MultiLeader/Table/Extended），改 `crates/cad-import-acadrust/src/tests.rs`。
- 未 patch/vendor acadrust；`cad-proxy` 不加依赖；Unsupported 建模未动。

**门禁（本机，无 GPU）**：fmt、clippy(`-D warnings`)、architecture、fixture-manifest、
workflows、i18n、workspace all-targets check、wasm32 lib check 全绿；
`cargo test -p cad-import-acadrust -p cad-proxy -p cad-db -p cad-domain` 212 passed / 0 failed / 1 ignored。
新增契约测试：LAYOUT `group 72/73` 被 acadrust 类型化（`acadrust_types_dxf_layout_group_72_and_73`、
`plot::layout_embedded_plot_fields_are_read_verbatim`），STYLE xdata 字体面类型化与
`read_styles` 优先级（`acadrust_types_the_style_xdata_face_and_the_scan_keeps_group_3`、
`importer_resolves_the_typed_true_type_face_without_the_scan`、
`importer_style_font_chain_is_typed_face_then_group_3_then_scan`）。

**能力增量（源码核对，详见 `docs/compatibility.md`）**

| 能力 | 0.6.3 | yacr |
|---|---|---|
| STYLE xdata 字体面 | 已类型化（`true_type_font`，DXF+DWG 经 `typeface_eed`） | 已接入；链=类型化 > group 3/4 > 字节扫描 |
| LAYOUT group 72/73 | 已暴露（0.5.5 亦然，旧文档有误） | 直接读取；纸名带单位时优先纸名 |
| plot-style 解析值 | 不支持 | 显式 Unsupported |
| 代理 opcode | 不支持（`proxy_graphics.rs` 字节一致） | 显式 Unsupported |
| DWG xdata | 部分（新增 crate-private `object_xdata`） | 显式 Partial |
| DXF 未知码 | 不支持（DWG 新增 `raw_record`/`layer_handle`） | 未使用 |

**已决策（2026-10-09）**

- **Path A：类型化 `true_type_font` 优先。** 解析链固定为
  `类型化 true_type_font > group 3/4 声明字体 > dxf_style_xdata_fonts 字节扫描`。
  理由：类型化字段是 acadrust 权威解码出的字体面，字节扫描只是回退；QCAD 类文件
  （group 3 为空）行为不变。DWG `true_type_font` 经 `read_styles` 统一消费（DWG 由
  `io/dwg/typeface_eed.rs` 填充），无需新代码。契约注释与文档已按此对齐，测试已固定该行为。

**未决项**

- `dxf_style_xdata_fonts` 字节扫描现与类型化路径基本重复，是否简化（本轮**不删**，仅记录）。
- `EntityCommon.raw_record`/`layer_handle` 未使用，保留为潜在能力。
- plot-style 解析值、代理 opcode、DXF 已知实体未知码仍显式 Unsupported。

## Web 生产部署 LinkError：跨部署 glue/wasm 缓存错配修复（2026-10-09，本轮）

用户报告线上 <https://yacr-examples.snakeheartgo.top/> 启动失败：
`LinkError: import object field '__wbg_adapterInfo_f9744bfff61e772c' is not a Function`。

- 实测**当前生产自洽**（干净 Playwright 浏览器 `chosen=WebGl2 cad_frames=1`，无错误）；
  上一版不可变部署 `200b4aee` 的旧 `yacr.js` **不含** `adapterInfo` 胶水 →
  定位为**旧 `pkg/yacr.js` + 新 `pkg/yacr_bg.wasm` 的跨部署缓存错配**（文件名无内容哈希、
  无共同版本），不是构建内 wasm-bindgen 版本问题。
- 修复：`scripts/build-web.sh` 以 `sha256(wasm)[:16]` 为 build stamp 注入 `index.html` 的
  `<meta name="yacr-build">`；`index.html` 按 stamp 载入 `main.js?v=`；`main.js` 按同一 stamp
  `import(\`./pkg/yacr.js?v=\`)` 并把 `yacr_bg.wasm?v=` URL 传给 `init()`，js/wasm 始终成对取用。
- 新增 `scripts/test-web-cache-busting.py`（静态+变异）接入 `core.yml`；`build-web.sh` 缺
  `__YACR_BUILD__` 占位符即中止。
- 本机 Playwright：三个请求均带同一 stamp 且应用正常；`check-web-ui.mjs` 通过。
- **未部署**：无 Cloudflare token，生产仍是旧 bundle，需下次 `web-deploy` 生效；已受影响客户端
  需硬刷新/清站点数据一次。详见 `docs/validation-web.md` §14。

## Web：真实 GPU Playwright 调试 + WebGPU 软适配器静默挂起修复（2026-10-09，本轮）

用户要求「本机开启 web wasm 测试，真机调试」，随后「改用 playwright 调试」。本机是**真实桌面**
（ThinkPad P15v，Wayland，Intel UHD + Quadro P620），非历史无头 LXC。

**实际执行**

- 安装 `wasm-bindgen-cli 0.2.129`（匹配 `Cargo.lock`），`DIST=/tmp/opencode/yacr-web-dev
  WITH_FONTS=1 scripts/build-web.sh` 成功；`scripts/serve-web.py` 起静态服务。
- 用已装的 flatpak Chrome 155（CDP 附加 + 仓库外 shim，**未改仓库脚本**）跑
  `check-web-ui.mjs` 真实 GPU 通过，证据记入 `docs/validation-web.md` §12。
- 改走 Playwright：`playwright-core 1.64.0` + 自带 Chromium 156.0.8078.4（headful 真实 GPU）。
  调试脚本在仓库外 `/tmp/opencode/webtest/{debug-web,webgpu-probe,webgpu-pref}.mjs`。
- **定位并修复**：`--enable-unsafe-webgpu` 下 `navigator.gpu` 给的是 SwiftShader 软适配器，
  JS 预探测选 `webgpu`，但 Rust 侧 Slint/wgpu 渲染 setup 永不完成 →
  `chosen=WebGpu adapter=None lifecycle=Detached cad_frames=0`，**无错误无回退的静默空白**。
  修复：`cad-ui-slint::web::select_backend` 强制 `WebGpu` 经真实 `webgpu_available()` 门；
  `webgpu_available()` 拒绝 `wgpu::DeviceType::Cpu` 软适配器并 `console_log` 探测结果，
  失败由 `app-web::start_with_preference` 既有分支回退 `WebGL2`。
- 重构建 + Playwright 复验：`--enable-unsafe-webgpu` 下显示探测 `device_type=Cpu`、
  `GpuFailure(...)`、最终 `chosen=WebGl2 … cad_frames=1 lifecycle=Ready`；默认路径不变；
  `check-web-ui.mjs` 通过。
- 新增 `scripts/test-web-backend-fallback.py`（静态+变异）并接入 `core.yml`
  workflow-contracts；本机通过 fmt、`cargo check -p cad-ui-slint -p app-web --target
  wasm32-unknown-unknown --locked`、`check-workflows.py`。

**未运行/限制**：真 WebGPU 硬件适配器仍未验证（本机只有 SwiftShader 软适配器，修复刻意不用它）；
非真实 DWG、非手机真机；证据与命令见 `docs/validation-web.md` §12/§13。

## CI 发布 job 首次运行失败与修复（2026-10-09）

`v0.1` tag 触发 `build.yml` 后，`windows-release` 与 `macos-release` 在打包步骤失败
（`linux-app`/`windows-check`/`macos-check`/`web-build` 等通过）。经配置 GitHub API 鉴权后拉取
真实日志，确认是**两个确定性脚本 bug**（非环境问题）：

1. **Windows**：`check-pe-imports.py` 在 Windows 文本模式下输出 CRLF，`package-windows-release.sh`
   的 `while read` 读到 `kernel32.dll\r`，与锚定正则 `^kernel32\.dll$` 不匹配，于是把**所有**
   导入误判为「非系统 DLL」并中止。修复：`check-pe-imports.py` 用
   `sys.stdout.reconfigure(newline="\n")` 输出 LF；脚本读取时 `name="${name%$'\r'}"` 兜底。
   本机已用真实 MSVC 产物验证：CR 剥离前 10 个导入全被误判，剥离后全部识别为系统库。
2. **macOS**：`otool -L` 对 **universal（fat）** 二进制会为**每个架构**打印一行
   `path (architecture ARCH):` 头；脚本原来 `tail -n +2` 只跳过第一行，于是 `arm64:` 头被当作
   「非系统依赖」。修复：改为只取缩进行
   `otool -L "$binary" | awk '/^[[:space:]]/ {print $1}'`（thin/fat 均正确）。
3. **Windows（第二处）**：`fetch-fonts.sh` 的合并步骤用
   `pathlib.Path.read_text()/write_text()` 未指定编码，Windows 默认 `cp1252` 无法解码 UTF-8 的
   `fonts.json`（含非 ASCII 字体名）→ `UnicodeDecodeError`。修复：读写显式 `encoding="utf-8"`。

契约测试补了对应回归守卫（`test-package-windows-release.py` 要求 CR 剥离片段、
`test-package-macos-release.py` 要求 `awk` 过滤片段、新增 `scripts/test-fetch-fonts.py` 守卫
UTF-8 编码与「缺字体即失败」）；`test-package-macos-release.py` 与 `test-fetch-fonts.py` 一并
接入 `core-quality`。

**已验证（2026-10-09）**：`v0.1` tag 强制更新到修复提交后，run
`https://github.com/zenglanmu/yacr/actions/runs/37916376185` **全部通过**：
`windows-release`（11m39s，`yacr-windows-release` 57,385,763 B）、`macos-release`（6m14s，
`yacr-macos-release` 72,217,415 B，universal）、`linux-release`（3m51s，`yacr-linux-release`
61,648,871 B）均 success 并上传含字体产物；`android-apk`/`android-release`/`web-smoke`/
`web-deploy` 按能力门控 SKIPPED。这是 CI 打包证据，**不是**真机/真实 GPU/安装运行证据。

## CI 发布层补全：Linux / Android release job（2026-10-09，本轮）

用户要求：在 CI workflow 里也加上 Linux app、Android app（等）。经问询确认：新增
`linux-release` 与 `android-release`，与 `windows-release`/`macos-release` 一样在
`workflow_dispatch` 或 `v*` tag 触发并上传含字体产物。随后用户要求「加入 android 发布包，
指向 jdk17」，故 `android-release` 改为**自装 JDK 17 + 固定 SDK/NDK**、不再能力门控。

**改动**

1. `build.yml` 新增 `linux-release`（`ubuntu-latest`）：安装 Slint 依赖 →
   `cargo fetch --locked` 预热（`package-linux-release.sh` 以 `--offline` 构建）→
   `WITH_FONTS=1 scripts/package-linux-release.sh` → 上传 `yacr-linux-release`
   （tar.gz + sha256）。脚本在打包内实际跑 CLI `--help` 与 GUI `--headless` 解析错误，
   校验 RPATH/无缺失库；不跑窗口/GPU。
2. `build.yml` 新增 `android-release`（按需/tag，`ubuntu-latest`，自装工具链）：
   `actions/setup-java@v4`（temurin **JDK 17**）→ `sdkmanager` 安装
   `build-tools;34.0.0`/`platforms;android-34`/`platforms;android-30`/`ndk;27.0.12077973`
   并写入 `ANDROID_HOME`/`ANDROID_NDK_HOME` → 前置校验 → `cargo-apk 0.10.0` → 生成 gitignored
   开发 keystore → `scripts/fetch-fonts.sh apps/app-android/assets/fonts` →
   `scripts/build-android.sh --release` → 断言锁文件未改 → `aapt2 dump badging` 记录事实 →
   上传 `yacr-android-release` APK。`android-apk`（每次 push/PR，`ANDROID_CI_ENABLED` 门控）
   保留为构建+事实 job。
3. `scripts/check-workflows.py`：REQUIRED_JOBS 纳入两者；`android-release` **不**加入
   `GATED_JOBS`（已自装工具链，不再需要能力开关）；`docs/ci.md`（job 表、十七个必需 job、
   gated 列表、两节说明、NOT RUN、action 表新增 `actions/setup-java`）、`docs/build.md`、
   `docs/handoff.md`、`AGENTS.md` 同步。

**实际执行并通过（本机 Linux）**

- `python3 scripts/check-workflows.py`（PyYAML 解析 + 结构检查，含两个新 job）、
  `scripts/test-linux-workflow.py`、`check-architecture.py`、`check-i18n.py` 通过。

**已实跑验证**：`linux-release` 在 run 37916376185 通过；`android-release`（自装 JDK 17 +
固定 SDK/NDK）在 run 37920264484 通过（7m59s），产出 `yacr-android-release`（`dev.yacr.app` /
arm64-v8a / targetSdk 30 / 53,017,904 B）；`web-smoke`（`WEB_SMOKE_ENABLED=true`）在 run
37922705082 通过（修复：runner 上不存在 `/tmp/opencode`，`tee` 因 `pipefail` 失败——改用
`$RUNNER_TEMP`）。run 37924085023（`workflow_dispatch`）**全绿**：四个发布包 +
`web-build`/`web-smoke`/`web-deploy` 全部通过，`web-deploy` **真实发布**到
`https://yacr-examples.pages.dev`。**限制**：APK 打包**不等于安装/真机运行**（无设备/模拟器）；
`android-apk` 仍门控未启用。详见 `docs/ci.md`、`docs/validation.md`。

## macOS release 包：GitHub Actions（`macos-latest`）+ 含字体 `Yacr.app` tar.gz（2026-10-09，本轮）

用户要求：构建 macOS 下应用，最终类似 Linux/Windows 版，输出含字体的压缩包。经问询确认：
按仓库既有的 CI 模式实现（代码 cfg 分支 + `app-macos` + `package-macos-release.sh` +
CI `macos-latest` 出包 + 文档），产物形式为 **tar.gz：含 `Yacr.app` bundle + fonts/docs**。
本机是 Linux，**没有 Apple SDK/链接器，无法产出 Mach-O**，故本机不伪造 macOS 产物。

**改动**

1. **共享桌面宿主**：`apps/app-linux` 的宿主实现由
   `cfg(any(target_os = "linux", target_os = "windows"))` 放宽为
   `cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))`；平台差异按
   cfg 分派——文件选择（macOS 用 `rfd`/`NSOpenPanel`，`host/file_picker.rs`）、配置目录
   （`$HOME/Library/Application Support/yacr`，新增纯函数 `resolve_macos_config_dir`，
   `host/config_file.rs`）、系统默认字体（`/System/Library/Fonts` + `/Library/Fonts`，
   `cad-platform::fonts::local`，macOS 无 `fc-match`）。
2. **macOS 宿主**：新增 `apps/app-macos`（二进制 `yacr-macos`），薄封装共享入口
   `app_linux::entry::run()`。`--headless` 离屏验证平台强制软件 Vulkan（Linux/Windows），
   macOS 无 Vulkan，故 `entry.rs` 在 macOS 上把 `--headless` 显式报告为不支持，而非静默
   降级或编译失败。
3. **字体目录候选**：`cad_platform::fonts::local::font_dir_candidates` 增加
   `.app/Contents/Resources/fonts` 候选，使 `Yacr.app` 内 GUI 与同级 CLI 都能自动发现字体
   （含新增契约断言）。
4. **打包**：新增 `scripts/package-macos-release.sh`（**只能在 macOS 运行**，非 Darwin 直接
   退出）。默认 `MACOS_ARCH=universal`：构建 `aarch64-apple-darwin` + `x86_64-apple-darwin`
   并用 `lipo -create` 合成通用二进制；产出 `Yacr.app`（`Info.plist` + GUI +
   `Contents/Resources/fonts/`）、`bin/cad-cli-tools`、`bin/yacr-macos` 与 `fonts` 符号链接、
   文档与脚本，打成 `yacr-<version>-macos-universal.tar.gz`（+ sha256）。`otool -L` 断言动态
   依赖只来自系统库，出现 `@rpath`/Homebrew 即中止；`WITH_FONTS=0` 离线仅 osifont；
   `YACR_MACOS_SMOKE=1` 跑 CLI `--help` 与 GUI 参数解析错误。
5. **CI**：`build.yml` 新增 `macos-check`（`macos-latest`，原生 arm64
   `cargo check -p app-macos --all-targets --locked`，每次 push/PR，不运行）与
   `macos-release`（`workflow_dispatch` / `v*` tag，universal 打包并上传 `yacr-macos-release`
   artifact）；`check-workflows.py` 的 REQUIRED_JOBS 纳入两者；`docs/ci.md` 同步。
6. **契约/文档**：新增 `scripts/test-package-macos-release.py`、`docs/macos-app.md`；更新
   `docs/build.md`、`docs/ci.md`、`docs/fonts.md`、`README.md`、`AGENTS.md`。

**实际执行并通过（本机 Linux，Rust 1.99.0）**

- 主机门禁：fmt、严格 clippy（`-D warnings`）、architecture/workflows/i18n、
  `test-package-macos-release.py`、`cargo check --workspace ... --all-targets` 通过；
  `app-macos` 在 Linux 上走 `cfg(not(macos))` 分支编译通过（仅证明新增 workspace 成员不破坏
  非 macOS 构建）。
- **macOS 门控代码的交叉类型检查**（本机 Linux，`rustup target add aarch64-apple-darwin`）：
  `cargo check -p app-macos --all-targets --target aarch64-apple-darwin --locked` 与
  `cargo clippy -p app-macos -p app-linux --all-targets --target aarch64-apple-darwin
  --locked -- -D warnings` 均通过（过程中修掉 `validation.rs`/`entry.rs` 对
  `cad_ui_slint::offscreen` 的无条件引用）。只类型检查，**不**链接、不产出 Mach-O、不运行。

**未运行/限制**：CI 的 `macos-check`/`macos-release` job **未在本环境运行**（无 macOS
runner）；Linux 主机**无法**产出 Mach-O，也没有 Apple SDK，不能本地复现；真实 Mac 的窗口、
Retina/DPI、Metal GPU 渲染与像素验收、原生文件对话框、配置目录写入、`--headless`（macOS 显式
不支持）、代码签名/notarization 全部 **NOT RUN**。详见 `docs/macos-app.md`。

## Windows release 包：GitHub Actions（MSVC）+ 含字体 zip（2026-10-09，本轮）

用户要求：构建 Windows 下 exe 应用，最终类似 Linux 版，输出含字体的压缩包；随后要求
**改走 GitHub Actions 的 Windows runner（MSVC）构建**。

**改动**

1. **共享桌面宿主**：`apps/app-linux` 的宿主实现由 `cfg(target_os = "linux")` 放宽为
   `cfg(any(target_os = "linux", target_os = "windows"))`，成为两个桌面宿主共享的实现
   层；平台差异按 cfg 分派——文件选择（Linux `ashpd` / Windows `rfd`，`host/file_picker.rs`）、
   配置目录（XDG / `%APPDATA%`，`host/config_file.rs`）、系统默认字体
   （`fc-match` / `%WINDIR%\Fonts`，`cad-platform::fonts::local`）。新增
   `app_linux::entry::run()` 供两个桌面二进制共用，避免解析/后端选择漂移。
2. **Windows 宿主**：新增 `apps/app-windows`（二进制 `yacr.exe`），薄封装共享入口；
   `cad-ui-slint::offscreen` 的 cfg 同步放宽到 Linux+Windows。
3. **打包（工具链无关）**：`scripts/package-windows-release.sh` 自动识别宿主 target：
   Windows 原生 MSVC 默认并自动 `+crt-static`（无需 VC++ redistributable）；Linux 可
   MinGW GNU 交叉或 `CARGO_XWIN=1` 走 MSVC。字体走 `fetch-fonts.sh`（该脚本也改为
   python3/python 自适应，便于 Git Bash）；非系统导入由新增 `scripts/check-pe-imports.py`
   （纯 Python PE 解析，替代 `objdump`，同时支持 Linux 与 Windows runner）检测，缺失即
   **中止**；zip + sha256 由 Python 生成（跨平台）。`WITH_FONTS=0` 离线仅 osifont。
4. **CI**：`build.yml` 新增 `windows-check`（`windows-latest`，原生 MSVC
   `cargo check -p app-windows --all-targets --locked`，每次 push/PR，不运行）与
   `windows-release`（`workflow_dispatch` / `v*` tag，打包并上传 `yacr-windows-release`
   artifact）；`check-workflows.py` 的 REQUIRED_JOBS 纳入两者；`docs/ci.md` 同步。
5. **契约/文档**：新增 `scripts/test-package-windows-release.py`、`docs/windows-app.md`；
   更新 `docs/build.md`、`docs/fonts.md`、`docs/validation.md`、`AGENTS.md`。

**实际执行并通过（本机 Linux，Rust 1.99.0）**

- 主机门禁：fmt、严格 clippy（`-D warnings`）0 警告、architecture/fixture/workflows/i18n、
  `test-package-windows-release.py` 通过；`cargo check --workspace ... --all-targets` 通过。
- GNU 交叉：`cargo check -p app-windows --target x86_64-pc-windows-gnu` 通过；MinGW
  release `cad-cli-tools.exe` 13,945,778 / `yacr.exe` 34,123,856 字节；
  `YACR_WINDOWS_SMOKE=1 scripts/package-windows-release.sh` exit 0。
- MSVC（`cargo xwin`，与 CI 同目标）：release 构建通过；`+crt-static` 下**无非系统导入**；
  `TARGET=x86_64-pc-windows-msvc CARGO_XWIN=1 WITH_FONTS=1 YACR_WINDOWS_SMOKE=1
  scripts/package-windows-release.sh` exit 0。产物
  `target/x86_64-pc-windows-msvc/release/dist/yacr-0.1.0-windows-x86_64.zip`，
  **57,364,212 字节**，sha256
  `deb9cc9ec3c31dddf1aeef9aa66bd7cd5a0d9184678c6b34e6929faa8c78abb7`（含构建时间，
  重跑会变）。gitignored `target/` 下，不入库。
- `check-pe-imports.py` 的 DLL 集合与 `x86_64-w64-mingw32-objdump -p` 一致。
- Wine（软件翻译）实际运行 GNU/MSVC 产物：`yacr.exe --bogus`（参数解析，exit 1）、
  `cad-cli-tools.exe --help`（exit 0）、`scan entities.dxf`（602 实体 / 75 块定义，exit 0）、
  `build-representation` 字体差分（带包内 `fonts/` 1464 primitives vs 无 1553）——证明打包
  字体被 Windows 二进制加载。

**未运行/限制**：CI 的 `windows-check`/`windows-release` job **未在本环境运行**（无 runner）；
真实 Windows 的窗口/DPI/原生文件对话框/配置写入、真实 GPU（DX12/Vulkan）与像素验收、
`--headless` 离屏、代码签名/SmartScreen、Windows 上 DWG `render`（无 wine+Vulkan 证据）、
完整 Linux `cargo test` 与离屏未在本轮重跑。Wine 只是软件翻译，不是 Windows 兼容性/真机
结论；GNU 与 MSVC 目标行为可能存在差异。详见 `docs/windows-app.md`。

## Linux release 包并入 GUI 主应用（2026-10-09，本轮）

用户指出 `scripts/package-linux-release.sh` 只打包无头 CLI，缺少 GUI 主应用。经问询确认
「并入现有 tar.gz」+「附带需打包的 `.so`」。

**改动**

1. **脚本**：同时 `cargo build -p cad-cli-tools` 与 `cargo rustc -p app-linux --bin
   yacr-linux`（后者带 `-C link-arg=-Wl,--disable-new-dtags,-rpath,$ORIGIN/../lib`）；staging
   增加 `bin/yacr-linux` 与 `lib/`；`copy_gui_libs` 用 `ldd` 收集非基础系统库（显式排除
   glibc/loader/libgcc/libstdc++）；打包内校验 `ldd` 无缺失、`readelf` 的 RPATH、库命中
   包内 `lib/`、`bin/yacr-linux --headless` 缺 `--output` 的既定错误加载成功；缺失即中止。
   新增 `BUNDLE_LIBS=0`（裸二进制）、`YACR_LINUX_SMOKE=1`（包内 GUI 离屏出图 + 报告断言）。
   `docs/` 纳入 `linux-app.md`、`fonts.md`；`PACKAGE.txt` 重写运行前提与限制。
2. **契约测试**：新增 `scripts/test-package-linux-release.py`（静态 + 变异），并接入
   `core.yml` 的 workflow-contracts 步骤与 `docs/ci.md`。只检查脚本文本，不跑 release 构建。
3. **文档**：`docs/build.md`、`docs/linux-app.md`、`docs/validation.md` 增补发布包布局、
   RPATH/库迁入、运行前提与边界。

**实际执行**：`VK_ICD_FILENAMES=.../lvp_icd.json YACR_LINUX_SMOKE=1 bash
scripts/package-linux-release.sh` 通过；迁入 8 个非基础库，包内 GUI 用 llvmpipe 离屏出图
`cadFrames=2`、`renderError=null`；产物 61732836 字节，sha256
`1473725b...af0094`（含构建时间，重跑会变）。release 构建命中缓存，非冷编译耗时。

**未运行/限制**：无真实 DWG（合成几何）、无窗口系统/真实 GPU/真机、无跨发行版/libc 兼容与
`xdg-desktop-portal` 桌面打开验证；包内库与构建发行版绑定，换发行版应 `BUNDLE_LIBS=0`
或从源码重建。

## 字体文件设计（2026-10-09，上一轮）

用户要求设计并**实现**字体文件方案（只做实现，不单独出设计文档），关键决策经问询确认：
字体来源 = QCAD + mlightcad；浏览器走 CDN，缺字体回退默认轮廓面；客户端输出带字体文件
（同级 `fonts/`）；客户端缺字体同样回退默认；QCAD `osifont.ttf` 提交进仓库，`.cxf` 不支持。

**改动**

1. **提交的字体包 `fonts/`**：`osifont.ttf`（QCAD 副本，GPL-3 + 字体例外；来源/哈希见
   `fonts/SOURCE.md`，许可证 `fonts/COPYING.GPL-3`）+ `fonts/fonts.json`（目录条目）。
   mlightcad 字体仍不入库。`THIRD_PARTY_NOTICES.md` 已记录。
2. **默认轮廓回退面（`cad-platform::fonts`）**：`load_font_engine` 在图纸字体缺失/拉取失败时
   保证注册一个默认面（`DEFAULT_FALLBACK_NAMES = ["osifont","arial","simplex"]`；
   `FontLoadReport.default_face` 记录）。新增 `register_default_face`（保留键
   `__yacr_default__`，置为第一回退）与 `fonts::local`（非 wasm）——可执行文件同级
   `fonts/` 目录的本地 `DirFontLoader`、目录候选/解析、fontconfig 系统默认候选与缓存、
   及组装引擎的 `load_engine`。
3. **Linux 桌面（`app-linux`）**：启动与每次打开图纸后重载字体：同级 `fonts/`（或
   `--fonts-dir`，显式目录必须存在）→ `--font` → 系统默认字体为第一回退。
4. **CLI（`cad-cli-tools`）**：`load_fonts` 增加 `requested` 入参，原生宿主复用
   `fonts::local`（同级 `fonts/` + 系统默认）；`run.rs` 先开图再取字体检清单。
5. **输出打包**：`scripts/package-linux-release.sh` 把 `fonts/` 复制到发布包
   `bin/` 同级；`scripts/fetch-fonts.sh`（平台无关唯一打包入口）把提交的
   `fonts/` 合并进目录清单与字节（web-dist/fonts、assets/fonts）。
6. **文档**：`docs/fonts.md`（来源/布局/回退/授权/未完成重写）、`docs/font-host-loading.md`
   （分层加桌面+默认面）、`docs/cli.md`、`docs/linux-app.md`、`docs/dxf-entity-coverage.md`。
   同轮评审后续：`fetch-web-fonts.sh`/`fetch-android-fonts.sh` 已并入**平台无关**的
   `scripts/fetch-fonts.sh <DEST> [FONTS...]` 并删除旧名——字体组装逻辑与目标平台解耦，
   各宿主（web/android/桌面）只传不同 `DEST`。见 `docs/fonts.md` `fonts/README.md`。

**实际执行并通过**：`cad-platform` 13 项（新增默认面/local 5 项）、`cad-cli-tools` lib 20、
`cli_contracts` 14（lavapipe 串行）、`app-linux` lib 1+3 ignored；fmt、严格 clippy
（`-D warnings`）、architecture/fixture/workflow/i18n、native 全 workspace check、
wasm 全 workspace lib check 均通过。CLI 差分 smoke：`entities.dxf` 无字体目录
lines=1443，有 `fonts/`（仓库根）lines=1497——缺字体时默认轮廓面成形，texts 均 0。
`fetch-fonts.sh` 合并 smoke：100 条目（99 mlightcad + osifont）成功。

**同轮后续（评审意见）**：Linux 发布包只带提交的 osifont 太单薄——用户指出 mlightcad
CDN 字库应随客户端输出内置（与 Web 一致）。`package-linux-release.sh` 已加 `WITH_FONTS=1`
（默认）：用平台无关的 `fetch-fonts.sh "$STAGE/fonts"` 组装 mlightcad 全量 + osifont，
`WITH_FONTS=0` 保留仅 osifont 的离线打包；PACKAGE.txt/文档已同步，mlightcad 再分发授权
仍由打包方负责。

**未运行/限制**：浏览器/真机下载与整形未验证；Android 未打包安装；无真实 GPU/窗口/
视觉验收；系统默认面依赖 `fc-match`，本机 `sans-serif` 命中 `.ttc`（引擎跳过）后取
`DejaVu Sans`，不同系统默认字形不同（预期）；Web 的「浏览器默认」是目录/CDN 轮廓面，
不是浏览器内建字体字节（wasm 无该能力）。

## 资源占用/显卡加速排查与三项修复（2026-10-08，本轮）

用户在真实 Wayland 桌面（Intel UHD 核显 + NVIDIA Quadro P620 混合显卡）上排查
`fixtures/complex-test.dwg`（4.2 MB，已在 `.gitignore`）的资源占用与「是否使用显卡加速」。
实测结论与修复如下；**本机为 ~30 GiB RAM 的桌面，非无头 LXC**。

**排查结论（均已实测）**

- **用了 GPU 加速，但排查时默认走 Intel 核显**：GUI（`yacr-linux`）与 CLI 在修复前
  默认都选中第一个 Vulkan 适配器 = Intel UHD Graphics（ANV）。DRM fdinfo 证据：进程在
  `renderD129`（Intel）上有 `drm-engine-render` 时间，`renderD128`（NVIDIA）为 0；
  `nvidia-smi` 看不到本应用。Slint 强制要求 GPU-backed 适配器
  （llvmpipe 需 `SLINT_WGPU_CPU=1`），能启动就意味着真在用 GPU。
- **空闲 CPU ~40%（单核）且与图纸无关**：启动无图纸（demo）也恒定 ~40%，
  原因是有 50 ms `Timer` 每 tick 无条件写 Slint 属性 + `CadView::request_redraw`，
  等价以 ~20fps 重绘。
- **大图 OOM 根因**：`complex-test.dwg` 只有 44,591 实体，但 1,081 个块定义嵌套
  展开成 **20,856,815 个 primitive / 46,064,001 顶点**；CLI `render` 之前
  `SceneCache::build`（不合批），`delta.added` 累积到 **~20 GB RSS（41 GB VM）被
  OOM-kill**（dmesg 已留证据）。

**改动**

1. **空闲重绘（`crates/cad-ui-slint/src/bridge/view.rs`、`apps/app-linux/src/host.rs`）**：
   `apply_view_snapshot` 对相同快照直接返回（不再 `set_view_state`+`request_redraw`）；
   `Runtime::metrics` 缓存 `(物理尺寸, config revision)`，未变化不调
   `refresh_window_layout`（不再每 tick 序列化有效配置 JSON）；`BridgeState` 新增
   `redraw_requests` 计数供测试/宿主观测。实测 release 空闲 CPU **~40% → ~0–2%**。
2. **显式 GPU 选择 + 双显卡默认独显（`cad-render-wgpu`/`cad-ui-slint`/`app-linux`/`cad-cli-tools`）**：
   新增稳定枚举 `GpuSelection`（`auto`/`high`/`low`）与
   `create_headless_gpu_with(preference, gpu)`（后端内按 `device_type` 优选，
   缺类型回退首个适配器，不跨后端）；`select_wgpu_backend_with` 映射到 wgpu
   `PowerPreference`；Linux App `--gpu`、CLI `render/plot --gpu`。**`auto` 与 `high`
   同义、在双显卡上默认优先独显**（用户要求），`low` 优先核显。实测修复后默认
   （`--gpu auto`）即选中 Quadro P620，`nvidia-smi` 显示进程 40–56 MiB / 1–4%。
   Slint 自动路径原生支持 `WGPU_ADAPTER_NAME`/`WGPU_POWER_PREF`（CLI 无头路径不读
   前者，文档已注明）。
3. **大图内存保护（`cad-cli-tools`）**：`render` 改用与桌面一致的打包虚线 +
   `SceneCache::build_compact`（97k 批次级合批），并新增 CLI 硬上限
   `--max-batches`/`--max-vertices`（默认 `4_000_000`/`128_000_000`，`0`=关闭），
   超限以 `invalid_input` **显式失败**（信息给出当前值与上限），绝不 OOM。
   `complex-test.dwg` 实测：**OOM ~20 GB → 峰值 RSS 3.85 GB、24 s、exit 0**
   （`scene.batches=97012`、`vertices=50251439`，帧按预算绘制部分批次——与桌面一致）。
   `plot` 路径同样套用预算守卫。

**测试与门禁（实际执行通过）**：`cargo test -p cad-render-wgpu`（40+12+13+3）、
`cad-ui-slint`（148，含新增 `tests/idle_redraw.rs`）、`app-linux`（8 passed /
3 ignored）、`cad-cli-tools` lib（20，含 `scene_budget_fails_explicitly_instead_of_oom`、
`gpu_selection_stable_names...`）与 `cli_contracts`（14，含 `--gpu`/`--max-batches`
解析契约）；fmt、严格 clippy（`-D warnings`）、architecture/fixture/workflow/i18n、
host 全 workspace 检查、wasm 全 workspace lib 检查均通过。

**未运行/限制**：Android/Web 未重编译运行（调用方签名未变）；无真实 GPU 像素矩阵、
真机、浏览器验收；`complex-test.dwg` 的数字是**本机软件 Vulkan（lavapipe）/Intel
核显**证据，不是性能或兼容性结论；NVIDIA 独显只验证到能出帧与占用（无视觉验收）；
GUI 空闲 CPU 数字来自本机 Wayland 桌面单次采样。

## Rust 工具链升级 1.98.1 → 1.99.0（2026-10-08，本轮）

用户要求把项目 Rust 工具链升级到最新 stable。以下为实际改动与**实际执行并通过**的门禁。

- **版本**：`rustup check` 报告最新 stable 为 **1.99.0**（`b940084d7`，2026-09-28）。
  `rust-toolchain.toml` 的 `channel` 由 `1.98.1` 改为 `1.99.0`，`components` 仍为
  `rustfmt`/`clippy`，target 仍为 `wasm32-unknown-unknown`。
- **同步镜像**：`.github/workflows/core.yml`（3 处 `uses` + 顶部注释）、
  `.github/workflows/build.yml`（5 处 `uses`）的 `dtolnay/rust-toolchain@1.98.1` →
  `@1.99.0`；`AGENTS.md`、`docs/build.md`、`docs/ci.md`（Action 表 + 锁定工具表）、
  `docs/validation.md`（环境小节）、`scripts/build-web.sh`、`scripts/build-android.sh`
  的版本说明同步更新。
- **未改动**：workspace `rust-version = "1.85"`（MSRV）保持不变——本次是工具链 channel
  升级，不是提高最低支持版本。
- **本机工具链**：`rustup toolchain install 1.99.0 --component rustfmt --component clippy
  --target wasm32-unknown-unknown`。
- **环境补装**：默认门禁包含 `cad-ui-slint`/`app-linux`，本机此前缺 fontconfig/freetype
  开发头，`yeslogic-fontconfig-sys` 构建脚本失败。经用户授权
  `sudo apt-get install pkgconf libfontconfig1-dev libfreetype6-dev` 后通过；此前
  `docs/validation-web.md` 记录的原生 Slint 构建阻塞在本机解除（该文档为历史证据，
  未回改）。

**实际执行并通过**（本机 1.99.0）：

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --exclude app-android --exclude app-web --all-targets
  --locked -- -D warnings`（含 `cad-ui-slint`、`app-linux`，0 警告）
- `cargo check --workspace --exclude app-android --exclude app-web --all-targets --locked`
- `cargo check --workspace --lib --target wasm32-unknown-unknown --locked`（完整
  workspace，含 UI/Host）
- `cargo check -p app-web --target wasm32-unknown-unknown --locked`
- `python3 scripts/check-architecture.py`、`check-fixture-manifest.py`、
  `check-workflows.py`、`check-i18n.py`

**未运行**：完整 `cargo test`、Linux 离屏/release、Android/Web 构建与真机（不属默认
提交门禁）；本文件下方与 `docs/validation.md` 中的详细证据仍标注其收集时的 1.98.1 环境。

## 完全移除用户批注（annotation）模块（2026-10-05）

按用户明确指示，整体移除用户批注（“批注/annotation”）功能，保留测量（F06）与 DXF
注释性缩放（annotative scaling，图纸实体特性）。范围与执行证据：

- **删除的 crate 与类型**：`crates/cad-annotations` 整包；`cad-db` 的 `Annotation`/
  `AnnotationDatabase`/`AnnotationGeometry`/`AnnotationStyle`/`AnchorStatus`/
  `EntityAnchor`/`validate_annotation`；`cad-domain::AnnotationId`。`MeasurementRecord`/
  `MeasurementAlgorithm` 从原 `annotation.rs` 迁至新的 `cad-db/src/measurement.rs` 保留。
- **历史/查询/场景**：`cad-history` 改为仅绘制实体撤销/重做（删除 `AnnotationPatch`/
  `UndoRecord`/`patch`/标注撤销 API/`MemoryJournal`/`RecoveryJournal` 与共享标记机制）；
  `cad-query` 删除 `AnnotationRow`/`annotations()`；`cad-scene` 删除 `annotations` 模块。
- **应用层**：删除 `annotation_tool`/`annotation_list`/`app_annotation` 与全部
  `CommandId`/`CommandPayload` 标注变体；`SessionState` 删除标注可见性/选择；`Document`
  删除 `annotations` 字段；`host_files`/unsaved 决策/恢复快照整体移除；`host` 删除
  标注导入/导出/恢复与 `save_measurement_as_annotation`；`recovery.rs` 仅保留后端结果模型。
- **绘制预览保留**：绘制/编辑工具的实时预览原先复用标注预览覆盖层，现改为
  `cad-app::render_scene::overlay::draw_preview_overlay`（`OverlayInputs.draw_preview`），
  行为等价（LINE/MOVE/TRIM 橡皮筋、CIRCLE 圆），不依赖已删除类型。
- **UI/宿主**：`cad-ui-slint` 删除标注面板/搜索/工具按钮/命令/预览桥接，`panels.slint`/
  `app.slint`/`ribbon.slint` 移除标注控件（ribbon 页签重编号为 文件/视图/测量/工具）；
  i18n 中英文各删除 19 个标注键（221→202，键集仍一致）；`app-linux`/`app-web`/`app-android`
  删除标注状态推送、sidecar 导入导出、恢复快照与未保存决策；Web JS/HTML 删除
  `annotation-input`、`export_annotations`、恢复快照入口；CLI 删除 `import-notes`/
  `export-notes` 与 `--notes`/`--allow-fingerprint-mismatch`。
- **测量**：`measure.save` 命令、`MeasurementUiState.can_save_annotation`、
  `StatusModel.unsaved` 等“存为批注/未保存”接口一并移除；测量本身（距离/折线/角度/面积、
  预览、确认/取消）保持。

**实际执行并通过的门禁**（本机、离线、排除 Android/Web 宿主编译）：
`cargo fmt --all -- --check`；`cargo clippy --workspace --exclude app-android
--exclude app-web --all-targets --offline -- -D warnings`；
`cargo check --workspace --exclude app-android --exclude app-web --all-targets --offline`
；`cargo check --workspace --lib --target wasm32-unknown-unknown --offline`（含 UI/Host）；
`python3 scripts/check-architecture.py`、`check-fixture-manifest.py`、`check-workflows.py`、
`check-i18n.py`；`node --test scripts/test-web-host.mjs scripts/test-host-panel-parity.mjs`。
`cargo check -p app-web --target wasm32-unknown-unknown --all-targets --offline` 通过。

**未运行/未闭环**：Android 目标（`aarch64-linux-android` 工具链与本机 target 不可用，
`--offline` 亦无法获取 `android-activity`）未编译；“各项测试通过”不包含 Android/Web
浏览器/真实 GPU/真机运行；文档 `CAD_IMPLEMENTATION_SPEC.md` §3.4/§16.3 与
`docs/architecture.md`、`docs/core-invariants.md`、`docs/compatibility.md`、
`docs/code-audit-and-agent-handoff.md` 中仍残留 F07/F08/F09 与 `cad-annotations` 引用，
尚未按“产品已移除该能力”重写，属已知文档债。真实 DWG 端到端见下一节。

1. 阅读规范、AGENTS.md、requirements.md、architecture.md 与最近 Git diff。
   此次恢复中发现并发实现与提交，结束时可能仍有其他实现工作；本文描述的是框架交接，
   请以当前源码和最新构建证据确认状态，不覆盖陌生变更。
2. 运行 README 的 check/test/architecture 命令；不要删除陌生的已有实现。
3. 用 `rg 'pending\(' crates apps` 查找显式占位；文件路径与字符串标识对应实施单元。
4. Android 的 Slint + wgpu 组合已验证到打包层（ADR 0002、docs/validation.md）；
   本轮进一步在无头模拟器（KVM + SwiftShader，x86_64）实际安装、启动、渲染并验证
   画布平移/适应（docs/validation-android.md）。**真机仍未运行**。Web 已可运行：
   wasm 静态产物在无头 Chromium 以 WebGL2 通过加载/导航/双语（docs/validation-web.md）；
   **WebGPU 与真实 GPU 未验证**。
5. 逐项实现 Builder/事务/历史 → 导入/基础语义 → 显示/索引/场景 → 应用/工具 → 宿主闭环；
   不把这个依赖顺序当作排期，也不绕过优先 GPU/UI 验证。
6. 每次移除占位同步补齐规范测试、能力表、构建说明、Git 提交。
7. DWG 打开/渲染回归遵循 `docs/testing-dwg.md`：有真实显示/GPU 时以真实硬件 GPU 为主
   （桌面 winit + 硬件 Vulkan、Android 模拟器 `-gpu host`），无 GPU 才回退 lavapipe；WASM +
   Playwright 为第二层；出图 smoke 与参考图视觉验收分开记录。

## Linux 桌面加载响应与不完整出图（2026-10-04，最新一轮）

- 桌面读取/导入及 native CPU 准备移出 UI 线程，采用单 worker + latest pending 的内容键
  与过期拒绝，保留原数据库/命令与增量缓存；未保存决策不被重新解释。
- renderer staged upload 不再复制整份 SceneDelta；桥接按每帧原预算累积绘制全部有序
  场景，保留透明顺序与深度，修复 5025 万顶点样本只提交前 800 万顶点的问题。
- 桌面 post-start debug/release 回归实际通过：当前可见批次 97,013/97,013，加载事件间隔
  最大 391/309ms，完成约 27.99/19.97 秒；点击 1.92/1.49 秒，仍不是普遍流畅保证。
- 本轮新截图能看到多组平面图和表格；不是参考 CAD 全图视觉验收。当前输入实际完成才
  显示已打开，旧 demo 帧/预算前缀不算就绪。证据/限制见 `docs/linux-dwg-ui-freeze.md` 顶部。
- 不提交原图/截图；WASM CPU 准备仍同步，Android/Web 运行、重复第二张大图、完整视觉
  与长期稳定性未验证。保留之前 clippy 修复；本轮修改未提交。

## Linux 桌面真实 DWG 无响应修复（2026-10-04，前轮历史）

**后续 clippy 修复**：`cad-app/tests/select_all.rs` 的拒绝断言改为 `expect_err`，
不改变拒绝路径和状态不变契约。`cargo test -p cad-app --test select_all --locked`
实际执行 **17/17 合成契约通过**；全树严格 clippy（按主机门禁排除 Android/Web）与
`cargo fmt --all -- --check` 通过。以下 clippy 失败是前轮历史证据，不代表当前状态。

**后续纠正**：用户手工 release 仍卡死，窗口启动后打开新增回归复现长停顿；最新
部分优化和未闭环项见本文「Linux release 后续排查」与 `docs/linux-dwg-ui-freeze.md` 顶部，
不能将下面前轮短测试作为加载响应已解决的证据。

用户确认当前环境为有 GPU/显示的 Wayland 桌面，不是历史无头环境。本轮使用用户提供的
仓库外 DWG 复现：主线程在复合几何子图元处理中反复深拷贝父实体；消除复制后仍生成
20,842,742 个底图批次，GPU 资源分配造成严重阻塞。修复只读子几何遍历，宿主 model/paper
场景采用不透明同样式折线的局部 line-list 合批，保留选择来源/样式/透明边界与局部坐标
精度约束；不修改 acadrust、数据库/测量几何，不隐藏填充或虚线。

新增 6 项合成契约和 1 项默认忽略的真实桌面回归；representation **139/139**、scene
**66/66** 通过。最终 debug/release 桌面回归分别实际通过：20 事件回调、3 CAD frames、
滚轮相机与 CAD 像素变化。首帧仍需约 **25–27 秒**，CPU 准备/GPU 上传仍同步，不能宣称
加载期间持续响应、流畅交互或整图视觉正确；Slint 默认选中设备的 vendor 未单独记录。

主机/wasm debug 编译、定向严格 clippy、fmt、架构/fixture/workflow/i18n 及最终 Linux
debug/release 二进制构建通过。全树严格 clippy 在未修改的 `cad-app/tests/select_all.rs`
失败（`err_expect`）；`cad-app --lib` **293 passed / 2 failed**（两项 unknown-layer 契约），
不记为全树门禁/测试通过。未执行完整 workspace 测试、无头离屏、Android/Web 运行、
参考图视觉验收或长期压力测试。详见 `docs/linux-dwg-ui-freeze.md`；外部图纸与截图未提交。

## 功能参考契约修订（2026-10-04）

用户明确 OpenCADStudio 仅为功能规格参考，不为源码参考。已同步规范 §2/§10/§13–15、
UI 需求总目录与实施追踪：从 `docs/ui-requirements/`（该目录已于 2026-10-15 随参考练习结束而移除）对照用户可见功能，核心独立实现，
不要求源码抽取、移植、上游依赖或算法采用；参考清单不扩大只读/测量/批注范围。
`docs/migration-map.md` 保留历史调查事实，旧迁移待办不再有效；既有实现与历史证据不删除。
本轮仅文档与 Rust 文档注释修订，不宣称新的产品运行或兼容性证据。
已运行 `python3 scripts/test-feature-reference-contract.py`：3 项合成文档契约通过。
未运行 Rust 编译/测试、Linux 离屏或其他平台验收。

## 独立分支并行审查第五轮（2026-10-04）

3 个并行子代理分别在 `review/20261004-{query-updates,annotation-files,history-recovery}`
独立分支/worktree 审查查询增量状态、批注文件校验、历史恢复日志，仅修改代码、合成契约
及专属文档，不编译/执行测试/检查/格式化/提交/推送。主控另外修复诊断资源预算饱和计数。
主控已格式化、提交并无冲突合并三个分支，统一执行 debug 编译与快速静态检查。
完整 Rust 测试、Linux 离屏和 release 构建均不在本轮执行范围。

- query：保留真实文档身份，以数据库查询建立绑定；增量通知严格遵循 `ChangeSet::follows`，
  未知/外部数据库、重放、错误前后修订与溢出边界均拒绝且不污染已发布状态。
- annotations：编码拒绝重复 ID 和原始保留边带；旧/不完整快照不能把当前数据库标为已保存。
- history：日志追加校验结构，整批恢复成功才发布，失败不留下部分重放状态。
- diagnostics（主控）：分类与总预算计数改为检查加法，最大上限仍拒绝溢出且保持计费原子性。

新增 **19 项合成 Rust 契约**（query 9、annotations 4、history 4、diagnostics 2）。
本轮最终统一代码树主机 debug 全目标编译（含 Linux App/Slint 与测试代码）、严格 clippy、
wasm 全 workspace lib 检查、fmt、架构/fixture/workflow/i18n 和 diff 空白检查均实际通过。
**NOT RUN**：Rust 契约未执行；完整测试、Linux 离屏、release 构建、真实 DWG/真实 GPU/
桌面窗口/浏览器/真机与远程 CI 未运行。详见 `docs/review-query-updates.md`、
`docs/review-annotation-files.md`、`docs/review-history-recovery.md`、`docs/review-diagnostic-budget.md`。
单个查询服务仍只绑定最新数据库；恢复临时克隆增加内存开销，身份/冲突检测/幂等/持久化
仍未闭环；导出编码成功不代表实际磁盘持久化，宿主须在成功写入后确认 revision。
提交前发现其他会话的功能参考规范/文档与几何注释修改；不纳入本轮提交，不覆盖。
这些修改后，全树 diff 空白检查因规范文档尾随空白失败；本轮文件的定向空白检查通过。
上述编译/门禁证据不推广到后续并发修改；共享交接文档只暂存本轮新增小节。

## 独立分支并行审查第四轮（2026-10-04）

主控启动 3 个并行子代理，分别在
`review/20261004-{db-transactions,domain-transforms,resource-resolution}` 独立分支/worktree
审查数据库事务、领域变换与资源解析。子代理仅修改代码、合成契约与专属文档，
不编译、不执行测试/检查/格式化、不提交或推送；主控审查后提交分支并统一合并验证。

本轮沿用快速 debug 门禁与英文提交，不运行完整 Rust 测试、Linux 离屏或 release 构建。
三个分支已由主控格式化、提交并无冲突合并：

- 数据库：实体换空间时清理旧块及动态状态引用；可见性切换不再误报非受控成员。
- 领域：极端尺度相似变换判定、NaN 尺度传播、非法容差/精确奇异矩阵拒绝、单位换算校验。
- 资源：字体目录拒绝路径型文件名；URL 路径段完整编码，避免 query/fragment/百分号歧义。

新增 **17 项合成 Rust 契约**（db 5、domain 9、resources 3）。主控在最终统一代码树
实际完成：主机 debug `cargo check --workspace --exclude app-android --exclude app-web
--all-targets --locked`（含 Linux App/Slint 与测试代码）、严格 clippy、wasm 全 workspace
lib 检查、fmt、架构/fixture/workflow/i18n 及 diff 空白检查，均通过。

**NOT RUN**：所有 Rust 契约仅编译、未执行；无完整测试、Linux 离屏、release 构建、
真实字体/真实 DWG/真实 GPU/桌面窗口/浏览器/真机运行或远程 CI 证据。
详见 `docs/review-db-transactions.md`、`docs/review-domain-transforms.md`、
`docs/review-resource-resolution.md`。保留限制：极端原始行列式、变换算子范数上界、
空间迁移后动态成员恢复/下游缓存、字体 base URL/重定向/重复解码仍须独立设计审查。

## 独立分支并行审查第三轮（2026-10-04）

3 个子代理在 `review/20261004-{geometry-robustness,scene-consistency,query-contracts}`
独立分支/worktree 仅修改代码、合成契约测试与专属文档，不执行编译、测试、格式化、
提交或推送。主控审查后格式化、提交并合并三个分支，再统一编译与执行提交门禁。

- geometry：向量长度溢出/下溢、大有限向量归一化、平移后面积消减、工作平面正交校验。
- scene：线拓扑三角形误计、不可能缓存预留破坏状态、预留与预算/队列计数溢出。
- query：空选择重复矛盾属性、属性查询文档身份校验、零上限分页越界。
- 主控：SHAPE 实例包围盒重复应用缩放，补充放大/缩小/平移合成契约。
- 用户要求已将 `AGENTS.md` 的 commit message 语言改为英文；默认门禁改为 debug
  全目标编译与静态检查。Linux 离屏、release 编译及完整测试不再是默认提交要求。
  已同步 core/build CI、workflow 校验器与变异契约、规范、README 和构建文档。
  Web/Android 发布打包配置保留，不作为本地默认 debug 门禁。

新增 13 项测试并加强 1 项既有测试；详细缺陷与限制见
`docs/review-geometry-robustness.md`、`docs/review-scene-consistency.md`、
`docs/review-query-contracts.md`、`docs/review-db-shape-bounds.md`。

统一最终代码树 debug 主机全目标检查（含 Linux App/Slint 与测试代码）、严格 clippy、
wasm 全 workspace lib 检查、fmt、架构/fixture/workflow/i18n 和 diff 空白检查已通过。
Python workflow 合成契约 Linux **5/5**、Web deploy **4/4** 通过；无远程 CI 执行证据。

**NOT RUN**：Rust 回归测试仅编译，未执行；完整 Rust 测试曾按旧门禁启动，随后在
编译阶段收到用户门禁变更并由主控 SIGTERM 终止，不计通过也不计测试失败。
后台链路因此未执行后续 Linux release 离屏和 wasm 检查；wasm 已由主控另行实际重跑通过。
原中止输出：
`/home/zenglanmu/.local/share/opencode/shell/da739b3910d63162f4687d9e8c13d613a0959f68/sh_104b620bd001up8HASB09SkXvs.out`。
本轮没有执行 Linux 离屏、release 构建、真实 DWG/真实 GPU/桌面窗口/真机验证，不以旧证据代替本轮。

## 独立分支并行审查第二轮（2026-10-04）

4 个子代理分别在 `review/20261004-{history,dependencies,mapping,resource-limits}`
独立分支/worktree 只写代码与测试，无编译/检查/格式化/提交/推送；主控审查、
格式化、单 crate 验证后整合开发分支，无冲突，主控执行完整门禁后提交/推送。
修复：历史合并消去及 patch 身份/重复验证、依赖失效 revision 连续性和根/深度预算、
仿射方向向量的大平移精度与矩阵校验、图片像素计数整数溢出。
主控同步接入 `resource.size_overflow` 诊断双语文案（203 keys）。

新增 31 项合成测试；主控 worktree 单 crate 测试通过：history 23、dependencies 20、
annotations 24+15、resources 21；i18n、稳定代码双语描述、Node 六套契约 53/53 通过。
统一代码树最终 fmt、严格主机 clippy、架构/fixture/workflow/i18n、wasm 全 workspace
lib 检查均通过；完整主机测试（Linux App + Slint，lavapipe 串行）
**1229 passed / 0 failed / 1 ignored**。Linux release 实际离屏 smoke 通过，证据
`/tmp/opencode/yacr-linux-20261004-020914-462863`（合成图纸/软件 GPU，2 CAD frames，
导航相机与像素变化，无渲染错误；不是参考图视觉验收）。
详见 `docs/review-branches.md`。真实 GPU/桌面窗口/真实 DWG/真机未运行。

## Rust 审查第一轮（2026-10-04）

后续按用户要求开启两个并行子代理，分别只写空间索引输入校验和测量数值稳健性
代码/回归测试，不运行编译、测试或格式化；已交付，主控审查并统一格式化/验证。
空间索引拒绝非法 bounds/ray 且更新失败不改旧索引；角度归一化避免大有限臂的溢出。
新增 5+6 项合成测试，主控执行 spatial **25**、measure **51**、history **17**、
resources **17 passed / 0 failed**。详见 `docs/review-spatial-measure.md`。
当前统一代码树严格主机 clippy、wasm 全 workspace lib 检查通过；完整主机测试
**1198 passed / 0 failed / 1 ignored**（合成/软件 GPU，真实 DWG 条件用例未提供输入，
不是其已执行证据）。新一轮 Linux release lavapipe 离屏 smoke 实际通过：
`/tmp/opencode/yacr-linux-20261004-013052-436072`，输入为合成图纸，
2 CAD frames、导航相机/像素均改变、无渲染错误；参考图视觉验收仍未执行。
Node 六套无 wasm/GPU 模块契约 **53 passed / 0 failed**。

集中修复 `cad-history`：批注撤销/重做数据库失败不再丢栈顶；UUID 数值不再
参与字节估算；绘图标记/payload 计数对称并保留原事务标记；合并执行预算淘汰；
待提交绘图步骤拒绝重入写历史；批注不能使用保留绘图合并键。新增 8 项合成回归，
修复前全部失败、修复后历史 crate **17 passed / 0 failed**。
详细审查范围与尚存预算/API 限制见 `docs/review-history.md`。

本轮已通过 fmt、严格主机 clippy（排除 Android/Web）、架构/fixture/workflow/i18n、
wasm 全 workspace lib 检查以及 Linux release 编译。
完整主机测试首次在编译阶段超时（120 秒），取消时限重跑后通过；此运行包含历史
修复，不包含后续资源/并行轮改动。原输出保存于
`/home/zenglanmu/.local/share/opencode/shell/da739b3910d63162f4687d9e8c13d613a0959f68/sh_10471d8940017619qmCkjQzIQN.out`。

资源轮补修 `MapResolver` 替换的预算重复计费和 `ResolverChain` 非缺失错误被吞；
2 项回归修复前均失败，修复后 resources/history 各 **17 passed / 0 failed**。
Linux release 已实际通过 lavapipe 合成离屏 smoke（含资源修复），证据
`/tmp/opencode/yacr-linux-20261004-011217-419337`。详见 `docs/review-resources.md`。
真实 DWG/真实 GPU/桌面窗口/真机未运行；不沿用历史证据作为本轮通过结论。

## 上轮状态（compact，2026-10-03）

**迭代 6：宿主捕捉接线 + minimal 命令集收窄 + 紧凑抽屉布局列表**：

- **宿主捕捉接线（`ws-snap-host`）**：`cad_app::host::snap_candidates_near` 从
  `drawing_pick_items` 构造 `SnapTarget`（烘焙实例变换，无法精确变换的 Text/Spline/Shape/
  Opaque/Compound 显式跳过，不误捕），`HostController::snap_hints_near_cursor` 在活动工具
  有世界光标时返回候选；Linux/Web 状态漏斗调用 `view.set_snap_hints`，捕捉提示层首次由
  真实几何驱动。
- **minimal 命令集收窄（`ws-minimal-cmds`）**：`preset_command_visible` 显式策略——minimal
  保留查看/测量/批注/打开/图层/布局/诊断（32 个）并隐藏绘制/编辑/模式/后端（8 个）；新增
  分区与单调性测试，CanvasOnly ⊆ Minimal ⊆ Full。
- **紧凑抽屉布局列表（`ws-layout-panel`）**：手机抽屉（`tools-open`）新增 52px 布局条，
  复用真实 `layout-rows` 切换模型/图纸空间；桌面标签不变。

主控修复：紧凑布局条使手机抽屉上移 52px，更新 `concept_offscreen` 的 measure 点选坐标并
注明原因。验证：fmt、严格 clippy（含 app-web）、架构/i18n/fixture/workflows、node 六套契约、
`cargo test --workspace --exclude app-android`（含 app-web）、wasm 全 workspace lib check、
Android aarch64 check、Linux release 离屏 smoke 全通过。仍为软件 Vulkan/合成。

**迭代 5：浮动溢出弹层 + 状态栏覆盖开关 + 真实捕捉提示**：

- **浮动 ribbon 溢出（`ws-floating-overflow`）**：配置 ribbon 分组超过 6 个命令时，▾ 现弹出
  **真实锚定 `PopupWindow` 飞出层**（不再内联撑高行高），每项派发并关闭；`close-on-click-outside`
  与显式 × 关闭。
- **状态栏覆盖开关（`ws-statusbar-toggles`）**：桌面状态栏新增坐标轴/网格/捕捉提示三个可勾选
  开关，经 `overlay-toggled` → `update_config_json({"view":{"overlays":{...}}})` → 单一漏斗
  重推，`view.overlays.*` 首次可由用户直接切换；标签取自新目录键 `overlay.axes/grid/snap_hints`
  （中英同键集，202 keys）。
- **真实捕捉提示（`ws-snap-hints`）**：复用 `cad_measure::SnapCandidate/SnapKind`（`cad-app`
  已依赖 `cad-measure`，架构允许），`snap_hint_overlay` 按捕捉类型绘制可区分标记
  （draw order 1_500_000），`OverlayInputs.snap_hints_input` + `CadView::set_snap_hints`
  接线，`view.overlays.snapHints` 门控；`snapHints` 关闭时该层不绘制。宿主适配留待后续轮。

主控修复合并后一处 `app.slint` 注释/换行错位，并补齐 `overlay.*` 目录键与状态栏开关标签接线。
验证：fmt、严格 clippy（含 app-web）、架构/i18n/fixture/workflows、node 六套契约、
`cargo test --workspace --exclude app-android`（含 app-web）、wasm 全 workspace lib check、
Android aarch64 check、Linux release 离屏 smoke 全通过。仍为软件 Vulkan/合成。

**迭代 4：ribbon 显示模式/溢出 + minimal 语义 + 宿主面板一致性契约**：

- **Ribbon 显示模式与溢出（`ws-ribbon-modes`）**：`RibbonGroup.display`
  (`iconAndLabel`|`iconOnly`|`labelOnly`，默认 iconAndLabel) 经 `ResolvedRibbonCommand.display`
  进入 Slint 模型（int 0/1/2），按项图标/文字；组内命令超过 6 个时渲染真实内联溢出列表，
  全部命令仍可派发（不静默丢弃）。**未做浮动锚定弹层**（诚实标注为内联）。
- **minimal 预设显式语义（`ws-minimal-preset`）**：新增私有 `PresetComponents` 真值表，
  `resolve` 改为查表；新增「minimal 组件语义」与「CanvasOnly ≤ Minimal ≤ Full 单调性」
  测试，行为对既有预设逐位不变。
- **宿主面板一致性契约（`ws-android-parity`）**：新增 wasm-free `scripts/test-host-panel-parity.mjs`
  （8 项），断言 Linux/Web/Android 三宿主的 panel/overlay setter 集合与
  `effective_config()` overlay 门控一致，并以 `handle.rs` 的 `set_*` 集合做子集守卫；已加入
  CI `web-host-contracts`。

主控修复合并后两处编译问题（`json_display` move、测试内 `ViewerConfig` 导入与 `RibbonGroup.display`
初值）。验证：fmt、严格 clippy（含 app-web）、架构/i18n/fixture/workflows、node 六套契约、
`cargo test --workspace --exclude app-android`（含 app-web）、wasm 全 workspace lib check、
Android aarch64 check、Linux release 离屏 smoke 全通过。仍为软件 Vulkan/合成。

**迭代 3：可配置 ribbon 命令派发 + 响应式视口契约 + 能力钳制修复**：

- **剩余 ribbon 命令（`ws/ribbon-unwired`）**：`view.reset` 现派发真实 `ResetView`；其余 8 个
  白名单命令因缺目标/手势保持显式理由（`ribbon.command_needs_target`/
  `ribbon.command_needs_gesture`），不伪造。新增“每个白名单命令都有分类动作/理由”的漂移
  回归测试（覆盖全部 `COMMAND_IDS`）。
- **响应式视口契约（`ws/panels-mobile`）**：新增 wasm-free `scripts/test-web-responsive.mjs`
  （9 项）覆盖 360×800/800×360/800×1280/1280×800 × DPR1/2/3 × 安全区，并含对
  `viewer_config.rs` 阈值（720/1200/540）的漂移守卫；已加入 CI `web-host-contracts`。
  顺带澄清 `cad-ui-slint::responsive` 的 600/1024 为历史帮助函数、非 live 权威（live 为
  `cad_app::viewer_config`）。
- **能力钳制修复（主控）**：`ui.commandOverrides.<id>.visible` 加入 `is_clamped_path`，
  用户偏好不得重新启用被预设/能力禁用的命令；修复 `apply_preference` 新建 override 键
  时绕过钳制的漏洞，新增回归测试。

主控验证：fmt、严格 clippy（含 app-web）、架构/i18n/fixture/workflows、node 触控/宿主/
可访问性/像素/响应式契约通过；`cargo test --workspace --exclude app-android`（含 app-web）
通过；wasm 全 workspace lib check、Android aarch64 check 通过；Linux release 离屏 smoke
通过。仍为软件 Vulkan/合成，未做真实 GPU/真机验收。

**迭代 2：数据驱动 Ribbon + Web 触控门控**：主控派发两个隔离 worktree 子代理（ribbon 分组、
web 触控门控与契约），主控统一合并/编译/门禁。

- **数据驱动 Ribbon（`ws/ribbon-groups`）**：`ui.components.ribbon.tabs[].groups[].commands[]`
  经 `cad_app::resolve_ribbon` 解析为纯模型并按 `commandVisibility` 过滤，`chrome.rs` 映射为
  新 Slint `[RibbonTabModel]`（`ribbon-model.slint`），`ribbon.slint` 在
  `ribbon-config-driven` 为真时渲染配置选项卡/分组/命令按钮，`on_ribbon_command` 映射到真实
  命令（未支持的合法命令显式 `ribbon.command_unsupported`）。无自定义 tabs 时内置面板不变。
  新增 10 个 i18n 键（中英同键集，197 keys）。
- **Web 触控门控与契约（`ws/web-interaction`）**：新增 wasm 导出 `viewer_interaction_json`、
  JS 谓词 `interactionAllowsTouch`/`readInteractionAllowsTouch`；`touch.js` 在 `touch=false`
  时不再捕获指针或转发触摸，并在 `yacr-config-changed` 时重估。`scripts/test-web-touch.mjs`
  扩展到 12 项契约（含畸形 JSON 默认放行的文档化行为）。

主控修复合并后一处生成类型导入错误（`Ribbon*Model` 由 `include_modules!` 提供）。验证：
fmt、严格 clippy（工作区，含 app-web）、架构/i18n/fixture/workflows、node 触控契约通过；
`cargo test --workspace --exclude app-android`（**含 app-web**）**1151 passed / 0 failed /
1 ignored**；wasm 全 workspace lib check、Android aarch64 check 通过；Linux release 离屏
smoke 通过。仍为软件 Vulkan/合成，未做真实 GPU/真机验收。

**UI 配置门控三工作流集成（迭代 1）**：主控派发三个隔离 worktree 子代理，子代理只写代码与
单元测试，主控统一合并、编译、门禁。三个工作流文件不相交，合并零冲突：

- **overlay 门控（`ws/overlay-gating`）**：`view.overlays.*` 首次真正控制绘制。
  `OverlayInputs` 增加 `visibility`（默认全开，保持既有调用行为），并进入
  `overlay_fingerprint`；`prepare_inner` 据此门控选择高亮、已提交批注，并新增
  `axes_overlay`/`grid_overlay`（按 `Drawing::bounds` 生成、`GRID_MAX_LINES=400` 上限、
  无 bounds 时 `overlay.bounds-unavailable` 诊断）；预览十字受 `snapHints` 门控。
  `CadView::set_overlay_visibility` 由 Linux/Web/Android 状态漏斗按有效配置推送，切换只
  重建瞬态叠加层不动底图。
- **interaction 门控（`ws/interaction-gating`）**：共享 `UiAdapter` 提升配置文件存储后，
  在事件时按 `interaction.pointer` 早退指针/滚轮/画布拾取；命令别名仅在
  `keyboardShortcuts` 为真时展开（完整命令名仍可用）；Web 触控 wasm 入口按
  `interaction.touch` 空操作。新增 `crates/cad-ui-slint/tests/interaction_gating.rs`。
- **原生偏好持久化（`ws/native-prefs`）**：`apps/app-linux/src/host/config_file.rs` 从
  `$XDG_CONFIG_HOME/yacr/`（回退 `$HOME/.config/yacr/`）读取 `config.json`（完整
  `ViewerConfig`）与 `preferences.json`（`allowedPaths` 投影），新增
  `--config`/`--preferences`；`LinuxApp::apply_user_preference_json` 应用后原子持久化
  投影。无 XDG/HOME 时显式不持久化；解析失败保留默认并报告。

主控验证：`cargo fmt --check`、严格 clippy（工作区，排除 android/web）、架构/i18n/fixture/
workflows 全通过；`cad-app` 275、`cad-ui-slint` 82+1 离屏+1 门控、`app-linux`
`host_contracts` 4 + `host_config_disk` 1 + `verify_ui` 1 + `verify_ui_drawing` 1
（lavapipe 离屏）通过。仍为软件 Vulkan/合成，未做真实 GPU/真机验收。详见
`docs/ui-redesign.md`「仍未闭环」与 `docs/linux-app.md`。

**verify-ui 循环轮 E：minimal 预设语义契约（本轮）**：`verify_ui.rs` 场景加入
`minimal` 预设断言：保留应用框架/图层面板/导航工具栏/布局标签，隐藏 Ribbon、命令栏、
状态栏（`UiPresentationModel` 的既有语义，此前只有 canvas-only 被验证）。截图
`05b-minimal`。此前轮 D：`ui-drawing` 增加“切到每个图纸布局再切回模型后模型画面逐字节
恢复”的断言。轮 A/B/C 见下。


**verify-ui 循环轮 C：命令面 ESC/CONFIRM 作用于当前命令（本轮）**：按 AutoCAD 约定修复
命令输入区：`CANCEL`/`ESC` 此前只取消绘制捕获，现在按活动状态依次取消测量/批注/绘制并
退出平移；`CONFIRM` 也按活动状态作用于测量/批注/绘制，而不是只确认绘制。新增
`command_line_confirms_or_cancels_whatever_tool_is_active` 源码接线契约，并在
`verify_ui.rs` 场景加入“开启测量/批注/平移后 ESC 必须结束”的断言。`cad-ui-slint` 单元
80 passed、`verify_ui` 场景通过。


**verify-ui 循环轮 B：真实图纸操作 + 选择缺陷修复（本轮）**：新增
`apps/app-linux/tests/verify_ui_drawing.rs`（`scripts/verify-ui.sh` 的 `ui-drawing`
层），用已提交开源 fixture `fixtures/dxf/qcad-examples/entities.dxf` 真实打开后执行：
多图层显隐改变并恢复合成像素、模型/图纸布局切换、真实指针点选几何、清除后再次点选、
真实 MOVE 一次事务且撤销恢复、命令别名 `L`/`C`/`TR`。首次运行发现**真实缺陷**：
`CommandId::Select` 会把会话置为 `ToolState::Selecting`，而 `apps/app-linux` 的
`Navigation::select` 只在 `ToolState::Idle` 时拾取，因此第一次点选后无法再更改选择
（清除后点选同样失效）。已改为允许 `Idle | Selecting`，测量/批注/平移仍阻止拾取；
“清除后再次点选”保留为该场景的回归断言。对比只取 CAD 画布区域以排除状态栏文案；
大图纸场景重建较慢，用“连续两帧相同”作为收敛条件，并保留确定性自检。证据见
`docs/verify-ui.md`；仍为软件 Vulkan/合成/开源 fixture，不代表真实 GPU/真机。


**verify-ui 循环轮 A：AutoCAD 深色外观 + 命令面操作（本轮）**：按用户要求把循环焦点从
批注转到“UI 匹配 AutoCAD 风格 + 其它操作”。外观：`ui/theme.slint` 改为 AutoCAD 深色
（深灰 chrome/panel、浅色文字、蓝色选中、近黑模型空间），`ribbon`/`button`/`canvas`/
浮动工具栏的硬编码浅色同步；`YacrWindow` 设 `Palette.color-scheme = dark`，使
std-widgets 的命令栏输入、下拉、确认按钮、进度条也变深（此前为浅色，与深色 chrome 冲突）。
`concept_offscreen` 的“chrome 为浅色”像素断言改为“chrome 为深色”。场景新增：
标题/Ribbon/侧栏/画布**平均亮度深色检查**，以及命令输入区真实操作（`TOOLS`/`PANELS`
开关、`LINE`+`ESC`、`CIRCLE` 未取点 `CONFIRM` 显式报“参数无效”且保留捕获、`MOVE` 无选择
显式报“移动需要先选择对象”、未知命令显式报错、`FIT`）。迭代修复：`ColorScheme` 是内建
枚举不能从 `std-widgets.slint` 导入（首次编译失败已记录并改正）；场景初版把不完整
`CIRCLE` 的拒绝文案猜成“点”，实际为 `draw.error.invalid`（“参数无效”），按真实目录修正。
证据见 `docs/verify-ui.md`、`docs/ui-redesign.md` 顶部说明；仍为软件 Vulkan/合成图纸。


**verify-ui 无头 UI 循环（本轮）**：新增轻量入口 `scripts/verify-ui.sh` +
`scripts/verify-ui-summary.py`，对应测试计划第二层：用 Slint 官方 offscreen 平台
（`Platform`/`WindowAdapter` + FemtoVGWGPURenderer）+ Mesa lavapipe 软件 Vulkan，实际
运行真实 `LinuxApp`、自动操作控件、逐层超时、截图、panic 扫描、结构化证据包
（`verify-ui.json`/`environment.json`/`logs`/`screenshots`）。**不安装 X11/Wayland**，
offscreen 平台即虚拟窗口运行时；`environment.json` 如实记录 `xvfbAvailable=false` 与
`realDevice=not-run`。新增 `apps/app-linux/tests/verify_ui.rs` 固定场景：测量取消不写库、
距离自动完成并存批注、撤销重做、LINE 提交、文字批注需文字、矩形批注两点自动提交、
批注显隐/删除改变并恢复合成像素、**图层显隐改变且恢复合成像素**、布局/标准视图、
真实 `WindowEvent` 滚轮缩放与左键拖动平移、**2D→3D→2D 合成帧逐像素无损往返**、
canvas-only、中英切换、侧车导出回导、四尺寸矩阵。迭代中发现无字体时文字批注按设计
不绘制、Measurement 批注不进叠加层，显隐断言改用不依赖字体的矩形批注，未把设计限制
误判为缺陷。
首次运行发现脚本预创建证据目录导致 `--headless` 正确拒绝覆盖（exit 17），已修复为不预建；
场景初版误把“距离可确认”当契约，实际距离两点自动完成、`can_confirm` 只服务开放型工具，
已改为分别覆盖两条路径。`verify-ui.sh` 全层通过（6 层 + 可选真实图纸层），
`YACR_TEST_DWG=fixtures/dxf/qcad-examples/entities.dxf` 时 app-dwg 通过；证据
`/tmp/opencode/yacr-verify-ui-run2-*/`、`...-dwg-*/`、`...-iter-*/`。范围与限制见
`docs/verify-ui.md`。仍未做：真实 GPU、窗口系统、真实 DWG、真机。


**QCAD examples 语料与图元扩展（2026-10-03）**：下载并提交 QCAD `examples/` 其余 9 个
DXF 与 `flange.svg`（`fixtures/dxf/qcad-examples/`，来源/SHA/许可见 `SOURCE.md` 与
`fixtures/manifest`）。实现 LEADER（顶点折线+实心箭头）及通用 Polyline、ATTRIB/ATTDEF、
MESH/PolyfaceMesh/PolygonMesh、WIPEOUT、HELIX；带字体名的 Text 由 `Unsupported` 改判
`Unverified`。新增无需 GPU 的 `crates/cad-cli-tools/tests/dxf_samples.rs` 与
`scripts/check-qcad-examples.py`（lavapipe 批量出图+覆盖报告，可选 ezdxf 参考导出）。
实测 **9/9 通过**。随后补齐 `VIEWPORT`（边框）、`TOLERANCE`（框+文字）、`MLINE`（中心线）、
`MULTILEADER`（引线+文字）接线，并把 `Text` 统一为 `Unverified`（类型已支持，字体依赖宿主）；
新增 `dxf_samples` 的**零-`unsupported` 验证**（`proxy-report`），9 语料 + `flange` 全部通过。
按用户要求**放弃 ezdxf 像素对比**。随后按顺序补齐：`RAY/XLINE`（裁剪到模型范围）、
`SHAPE`（SHX 字形，新增 `SemanticGeometry::Shape`）、全部 DIMENSION 子类（角度/坐标/
圆弧长/大半径）、`TABLE`/`RASTERIMAGE`。另修复纸空间 `plot`：纸张单位从纸名解析、默认选有 viewport 的
布局、超限纹理返回结构化 `gpu_failure`；flange 纸空间边框/标题栏可出图（视口比例近似）。
另补回 DXF STYLE XDATA 字体名（`1001 ACAD`/`1000`，DXF-only 扫描，组件优先，不改
acadrust）。仍剩外部内容加载、光栅像素纹理、纸空间视口合成保真等 `Partial`；详见
`docs/dxf-entity-coverage.md`。

**QCAD flange 渲染回归样本（2026-10-03）**：按用户要求提交 QCAD 开源 `flange.dxf`
及 PNG/PDF 参考（`fixtures/dxf/qcad-flange/`，来源/许可/SHA 见其 `SOURCE.md` 与
`fixtures/manifest`），新增 `scripts/check-dxf-reference.py` 与无需 GPU 的
`crates/cad-cli-tools/tests/dxf_fixture.rs`。原生 release + lavapipe 实际执行：
419 entities / 223 model、0 build failure；实现无匿名块 DIMENSION 合成（线性/对齐/半径/
直径：线、实心箭头、按 DIMSTYLE 的测量文字）后表示 329 primitives（含 11 箭头 mesh + 6
标注文字），`dxf_fixture` 2/2 通过并新增箭头/文字回归断言。带宿主字体时 1024×768 非空帧
1.87%，四视图几何、剖面线、尺寸线与测量文字与参考一致；仍 `Partial`，因为文字绘制需要
宿主字体，且纸空间图框/标题栏不在 `render`（模型空间）范围内。证据
`docs/validation-dxf-flange.md`、`/tmp/opencode/yacr-dxf-reference-font/`。同时按用户要求
把“外部图纸/参考图不得入库”改为“授权可再分发即可入库并记 provenance”。

**Linux 文件选择/关闭崩溃/后端诊断修复（release 门禁进行中）**：桌面打开经 ashpd 调用系统
`xdg-desktop-portal` 文件选择器（独立线程等待，取消/失败区分，脏状态选择前后检查）；
`--open` 为启动路径，无窗口模式仍不调用选择器。Slint setup/teardown 不再写图片属性，
避免用户 Wayland 关闭回溯中的 RefCell 重入。共享设备实际 API 决定能力/诊断，软件
Vulkan/原生 Vulkan 不再误标 webgpu。契约与执行范围见 `docs/linux-app.md`；本轮未做
真实桌面 portal/Wayland/真实 GPU 验收，不沿用历史门禁作本轮证据。主机本轮合成回归
**1103/0/1 ignored**、针对性 Linux/UI 83 项、fmt/严格 clippy/四项 Python 门禁通过；
release 主验收与 WASM 门禁仍在执行（后台串行，日志 `/tmp/opencode/yacr-linux-picker-*.log`）。

**UI/DXF/文字修复（已提交推送及发布）**：用户指出图标/禁用/导航与规范不一致，要求支持 DXF、
网络 DWG 文字对比，只跑 Linux/Web 构建、不跑完整测试，并发布 Pages。范围及真实文字
14 倍放大缺陷证据见 `docs/ui-dxf-text-fixes.md`；缺原字体仍显式 Partial，不能宣称视觉全通过。
Linux/Web release 构建、4 项针对性契约、Linux DXF smoke、Web DXF File API 与真实四按钮
点击/平移抽查通过。大图 Linux App + TIMES 尝试 240s 超时，CLI 对比有效但不算宿主性能通过。
**未跑完整 workspace/clippy 门禁（用户要求）**，不用上一轮全量通过替代本轮证据。
实现提交 `d269140` 已推送；Pages 生产部署 `a6ea4bfe-9c00-40ae-8133-76535a242983`
API 确认成功，`https://yacr-examples.pages.dev/` 实际浏览器合成 DXF 重跑通过。

**并行三工作流（本轮新增，2026-10-03）**：主控派发三个隔离 worktree 子代理，分别闭环
三处显式缺口，主控统一合并/构建/门禁；子代理只写代码与单元测试，所有构建与运行由主控执行。

- **批注 sidecar 保真（`ws/annotation-fidelity`）**：`cad-annotations` 现在保留注解对象及
  嵌套 `geometry`/`style`/`precision` 的未知字段（私有顶层边带
  `yacr.nested_extensions`，`AnnotationFile::nested_extensions` 为类型化视图），编码时与
  已知字段冲突或命名缺失注解则拒绝；时间戳解码同时接受整数 Unix 毫秒与 RFC 3339
  （`Z`/偏移/小数秒），非法或 `modified < created` 拒绝；新增
  `MIN_SCHEMA_VERSION`/`migrate_file` 显式版本策略（更高 `Unsupported`、更低且无确定性
  迁移则 `CorruptData`）。`RecoverySnapshot` 保留未知顶层字段并拒绝更新版本（不降级）。
  新增 6 项注解契约测试 + 3 项恢复快照测试。详见 `docs/annotations.md`、`docs/recovery.md`。
- **矢量出图（`ws/vector-plot`）**：`cad-representation::plot_vector` 新增纯 CPU 路径文档 +
  自包含 SVG/PDF writer（无第三方 crate、不建 GPU 设备）；PDF 透明度用真实
  `ExtGState`（`/ca`/`CA`，页面 `/Resources` 引用、`gs` 选择后恢复不透明）。CLI 新增
  `--plot-format png|svg|pdf`（默认 png）。无法表达为路径的图元逐项 `vector.*` 诊断并降级
  `completeness`，不静默丢图。176 项相关单测通过（含 SVG/PDF 结构、透明度资源、诊断）。
  详见 `docs/plot.md` §9、`docs/cli.md`。
- **ViewerConfig 协议（`ws/viewer-config`）**：`cad-app::viewer_config` 补齐
  `features`/`view.overlays`/`interaction`/`commandOverrides`/`userCustomization.allowedPaths`
  /Ribbon tabs/groups/commands/panel placement/initiallyOpen；解析顺序
  默认→预设→宿主→宿主允许的用户偏好，用户偏好 clamp（不得重新启用被禁止项），
  数组整体替换、显式 false 生效、失败原子保留旧值与 revision；`ViewerConfigStore` 提供
  revision + 数据驱动 observer + 有效配置查询。Web 暴露
  `window.yacr.setConfig/updateConfig/applyUserPreference/clear/config` 与
  `yacr-config-changed` 事件，localStorage 仅投影 allowedPaths；Slint 按新的
  `UiPresentationModel` 分面板/overlays/features 门控。详见 `docs/ui-redesign.md`。

主控集成：合并三支后修复 3 处子代理遗漏（app-web 格式、`web.rs` 构造 CustomEvent、
`run.rs` wasm 下 `run_plot` 引用、`chrome.rs` 未用 import、app-web 未用 re-export）；
门禁与运行证据见下方“三工作流集成轮”。

**Linux App 主验收（本轮完成）**：`apps/app-linux` 提供桌面与无窗口共用 LinuxApp，
真实 HostController 命令与数据库/共享 Slint/wgpu 桥，release smoke 入口
`bash scripts/check-linux-app.sh`。规范 §11.0、AGENTS/build/ci 已改为 Linux 主验收，
GitHub `linux-app` 默认启用；原生质量层包含 Slint/Linux App 串行测试。执行证据与未接线
范围见 `docs/linux-app.md`；未运行远程 workflow/桌面窗口不计通过。
最终主机合成测试 1048 passed / 0 failed / 1 ignored，fmt/clippy/Python/WASM 门禁通过。
release 实际运行证据 `/tmp/opencode/yacr-linux-app-final-retry/`：软件 Vulkan，2 CAD 帧、
4553 画布像素导航变化；已抽查合成截图，无真实 DWG/桌面窗口/真实 GPU 验收结论。

**Concept UI 重设计进行中**：设计依据 `docs/ui-spec/`，配置/布局纯契约已新增到
`cad-app::viewer_config`（3 项合成测试已执行通过）。用户授权解除原生 Slint 编译限制，
后续使用 Linux Slint/wgpu/lavapipe 离屏主测试，不跑 Android 模拟器，最后 WASM 浏览器抽查。
完整范围与配置未闭环项见 `docs/ui-redesign.md`。
Slint 已改为浅银/蓝选中 concept：桌面左侧面板+布局/状态；手机标题+底部四组和工具/图层抽屉。
新增 Linux 官方 FemtoVG/wgpu 离屏平台；原生 UI 合成单元 76 项、真实离屏集成 1 项通过。
截图 `/tmp/opencode/yacr-concept-round4/`。浮动控件的浏览器触控排除命中已补契约。
本轮三个里程碑已完成限定范围：配置布局子集、共享 Slint concept 重排、原生主回归及
桌面 WASM 第二层抽查。最终 Linux 合成 1045 passed / 0 failed / 1 ignored，严格 clippy
（含 UI 和 Web wasm）及全部门禁通过。原生截图 `/tmp/opencode/yacr-concept-final-native/`；
浏览器 `/tmp/opencode/yacr-concept-final-web/integration.png` 人工抽查确认初始合成图不再裁切，
导航/双语/偏好/空批注往返通过。完整配置协议、真实 DWG/真实 GPU/真机仍未验证。

**真实 DWG 回归（最新）**：以用户外部 `anteen.dwg` 与去色参考图，优先 Linux
原生 wgpu/lavapipe。修复 2D 投影重复 Y 翻转、CLI render/plot 忽略 `--font`；
核心串行 **965/0/1 ignored**。原生导入/出图通过，但填充/字体保真未验收。
WASM/Playwright 发现打开后旧演示纹理停滞，增加事件循环唤醒后真实文件
CAD 帧 **1 → 4**，最终 WebGL2 打开/出图 smoke 通过。修复前 120 秒超时证据保留；
目录 99 个字体但本图注册 0 个，Web 缺文字，**视觉保真仍未通过**。
新增可复用 `check-dwg-native.py` / `check-web-dwg.mjs`；证据与命令见
`docs/validation-dwg.md`，产物 `/tmp/opencode/yacr-canteen-validation/`。

**修复与 Pages 发布轮（最新，2026-10-03）**：继续修复 Web 第二触点/touchcancel 不取消
shell 绘制捕获、Web/Android 成功换图纸后残留旧取点、TRIM 使用固定世界拾取容差的问题。
新增 `check-web-drawing-safety.mjs`，真实浏览器证明第二触点（未移动）、touchcancel、换图纸
之后确认不写库；TRIM 复用共享屏幕像素拾取容差。核心 **963/0/1 ignored**、JS **28 passed**，
跨平台编译/严格 clippy/架构/i18n/fixtures/workflows 通过。

发布候选 `/tmp/opencode/yacr-pages-release/` 含完整 99 个同源 CAD 字体；四种工具、输入安全、
桌面 UI 通过。mobile 三场景独立进程复跑通过（`/tmp/opencode/yacr-pages-mobile/`），
此前超时记录不删除；最终 Ribbon 触控复验仍超时，**不宣称最终 Ribbon 回归通过**。
修复提交 `01f9f2b` 已推送 main 并部署至已有 `yacr-examples` 的生产分支。
生产地址 `https://yacr-examples.pages.dev/`（自定义域名 `yacr-examples.snakeheartgo.top`）；
两地址 wasm 哈希匹配验证包。线上桌面 UI、LINE/CIRCLE、输入取消安全通过；MOVE/TRIM
线上截图超时，本地像素通过，**不能宣称所有线上绘制回归通过**。详情见 validation-web §11。
Android 本轮仍只有编译门，未新增 APK/真机/WebGPU 证据。

**绘制/编辑合并后主控验收（历史）**：基于 `39077ec` 的数据库写事务、应用命令、
UI 工具及 Web/Android sink 合并结果，完成核心与无头 Web 验证；没有另启 subagent。

- 修复命令后渲染仍持有旧底图 Arc：两宿主状态漏斗通过 `CadView::sync_drawing`
  发布当前数据库快照，创建/编辑/撤销/重做可见；导航与叠加保持原 Arc，不重导入。
- 修复绘制主指针同时进入宿主导航/选择：MOVE 取空白锚点不再清空选择，取消不残留
  意外选择高亮。绘制 sink 返回真实命令错误，失败保留捕获参数，不冒充提交成功。
- 新增 `scripts/check-web-drawing.mjs`：真实 Slint 命令栏取点/确认，经宿主与事务到
  WebGL2；LINE/CIRCLE/MOVE/TRIM 四场景均改变底图像素，撤销恢复相同 CAD 像素哈希；
  LINE/CIRCLE 确认前与取消不新增实体，重做恢复实体计数。每种工具独立进程串行运行。
- 核心串行 **963 passed / 0 failed / 1 ignored**；JS **27 passed**；fmt、核心与 app-web
  严格 clippy、架构、i18n（159 keys）、fixture/workflow、wasm workspace lib、UI/Web wasm
  测试编译、Android aarch64 测试编译通过。UI/宿主 Rust 测试为**编译而非执行**。
- 本地无头 Chromium/SwiftShader：四工具、overlay、ribbon（1280/390/320px）、桌面 UI
  通过；证据 `/tmp/opencode/yacr-drawing-final/`、`/tmp/opencode/yacr-draw-final-*`。
  `check-web-mobile.mjs` 两次超时，本轮**未通过**；连续截图停滞记录在 validation-web §9。
  本轮未部署生产、未运行 Android APK/真机/WebGPU，测试 bundle 不含 CAD 字体。

详情：`docs/drawing-edit.md` §6、`docs/validation-web.md` §9。绘制/编辑是受控内存库
子集，不支持保存修改后的 DWG；不能据此宣布规范的“未来底图编辑”完整产品验收。

**UI 宿主接线轮 2（历史，iterations 2–4）**：Ribbon 文档的宿主接线与渲染叠加全部闭环，
并继续补齐审计项。四个迭代的顺序合入均通过主控验证：

- **叠加层宿主接线**：`CadView::set_selection_highlight/set_measurement_preview/
  set_annotation_preview` 已由 Web 与 Android 的状态漏斗在每次命令/拾取/打开/确认取消后
  推送；选择变化复用底图与批注 Arc。
- **选择高亮端到端修复**：修复 Web 画布点击永不可选（`!was_dragging` 门把每次点击都当
  拖动拒绝）；现在单击命中即派发 `Select`，无头截图证明圆被高亮且清空回到基线。
- **测量存为批注（F06/F07）**：`CommandId::SaveMeasurementAsAnnotation`，一次事务/一次
  撤销，无记录时 `InvalidInput`；UI 按钮「存为批注」仅在确认记录存在时可用。
- **查看/工作模式（U02）**：`CommandId::SetMode` + 命令栏开关，权限仍在命令层强制。
- **窄屏命令栏**：手机上模式开关移入可展开行，320px 不再挤压命令输入框。
- **异步导入（F01）**：核心快照 + `ImportProgressUiState`；Web 在 wasm（无线程）只推真实
  终态并诚实说明，Android 走真实 `std::thread` worker + 100ms 轮询 + 取消；面板无伪造进度。
- **可访问性（U12）/ 状态重叠（U08）**：`aria-live=polite` 人类摘要区域、`role=application`
  可聚焦画布、`host-state` 就绪时隐藏且不再双重播报。
- **U07**：Android 暴露 `set_surface_size` 入口（Activity 回调仍为显式未接钩子）。

证据：核心串行 **830 passed / 0 failed / 1 ignored**；JS 契约 **26 passed**；i18n
**151 keys**；架构、fixture manifest、workflows、Android aarch64 检查全通过；无头
Chromium（SwiftShader/WebGL2）ribbon/UI/mobile/overlay 四套脚本在 1280×800、390×844
DPR3、320×740 DPR2 通过，并直接对 **Cloudflare Pages 生产 URL 重跑通过**。新增
`scripts/check-web-overlay.mjs` 端到端证明选择高亮改变像素并清空回基线。仍开放：真机、
WebGPU/真实 GPU、Android Activity resize/SAF。（绘制/编辑后续进展见上方主控验收。）

**UI 宿主接线轮（iteration 1）**：Ribbon 文档里"宿主连接器（本轮范围外）"与"预览几何尚未
接线"两项已落地并端到端验证。三个并行 workstream 已合入 main：

- **Web 宿主**：`browser/state_push.rs::push_panel_state` 成为唯一状态漏斗，在每次
  命令/打开/批注变更/启动恢复后推送历史可用性（undo+redo 独立）、测量、图层+有序 id、
  属性、批注+有序 id、布局、诊断抽屉（来自真实 `ImportReport`）；安装
  `WebCanvasPickMapper`（逻辑像素→世界点，退化输入返回 None）；无工具时的轻触经
  `pick_at_screen` 派发 `Select`（空命中＝清空选择），不再静默。新增
  `diagnostics_report_json`。空态/多值文案全部来自目录。
- **渲染叠加**：`cad-app::render_scene::overlay` 新增选择高亮（按 `drawing_pick_items`
  展开 INSERT，未解析引用显式诊断不伪造）与工具预览（测量/批注 rubber-band + 捕捉点
  + 矩形/椭圆）批次；`CadView::set_selection_highlight/set_measurement_preview/
  set_annotation_preview` 独立 `overlay_revision`，选择变化复用底图与批注 Arc。
  `prepare_shared_with_overlays` 为向后兼容的新入口。高亮 `draw_order` 900_000，
  预览 2_000_000，位于底图与批注之间/之上。
- **Android + Web 手势**：Android 接入同一状态漏斗 + `AndroidCanvasPickMapper` +
  tap/drag 判定（`InputPolicy`）+ `apply_surface_size`（横竖屏不改相机中心；Activity
  resize 回调仍为显式未接钩子）；修复 `web/host/touch.js` 手指数变化时重定基准会跳变
  的缺陷，`node --test scripts/test-web-touch.mjs` 6 passed。

证据：核心串行 **819 passed / 0 failed / 1 ignored**；wasm lib check、clippy、
架构、i18n（132 keys）、fixture manifest、Android aarch64 check 全通过；无头 Chromium
（SwiftShader/WebGL2）ribbon/UI/mobile 三套脚本在 1280×800、390×844 DPR3、320×740
DPR2 通过，并直接对 **Cloudflare Pages 生产 URL 重跑通过**。截图
`/tmp/opencode/yacr-ribbon-final/`、`/tmp/opencode/yacr-ui-final.png`。仍开放：真机、
WebGPU/真实 GPU、Android Activity resize/SAF、INSERT 根选择高亮（仅叶子高亮）。
详见 `docs/ribbon-ui.md`、`docs/ui.md`、`docs/panels.md`。

**Ribbon UI 布局轮**：保留 Slint/shared-wgpu，界面拆成 app/ribbon/button/command-bar/
canvas/panels，桌面 Ribbon 与命令栏可收展；手机默认仅底部命令栏，TOOLS 打开工具。
手机 48px 按钮，真实 Slint 文件选择器导入 DWG、双指缩放与拖动已接应用命令。
未接绘制/编辑菜单禁用且显式标注，交接详见 `docs/ribbon-ui.md`。独立无 Slint CAD
显示 wasm 尚未提取，当前 app-web.wasm 不算独立引擎，由下一阶段实施。

**渲染桥职责重构**：继续采用 Slint/shared Device/Queue。CPU controller 移至
`cad-app/render_scene`，GPU runtime 与 Slint presenter 分离；字体 revision、
上传成功后发布、底图/批注组更新、画面脏标记、纹理身份和设备生命周期已接线。
`IncomingDocument=None` 明确表示关闭文档。详见 `docs/bridge-runtime.md`。
最终核心串行测试 875 passed / 0 failed / 1 ignored；新增 GPU 契约实际运行。
桌面及 320/390px 高 DPI 无头 WebGL2 回归通过，UI-only 语言切换不增加 CAD 绘制数。
原生 Slint 测试仍受环境阻塞；wasm 测试仅编译。后台 CPU 准备、分批上传预算和真实
设备丢失恢复尚未验收。并行核心 GPU 测试一次驱动 SIGSEGV，串行重跑通过，不能隐去。

**宿主结构拆分**：Web Rust 宿主拆为 documents/annotations/fonts/input/persistence，
JS 宿主拆为 i18n/files/renderer/runtime；Slint 桥分离 scene/camera/tests，既有接口和
13 项桥测试保留。核心 870 passed、新增 JS 契约 5 passed、Web release 与两次本地
Playwright 通过（含模块 HTTP 200、批注下载/空 sidecar 回导与状态保留）。CI 与部署
检查包含所有 JS 模块。结构与环境阻塞见 `docs/code-structure.md`、
`docs/validation-web.md` §6；原生 Slint 测试未执行完成，不能当作通过。

四个并行 workstream 已合入 main 并验证：

- **Android 运行闭环**：x86_64 release APK 在无头模拟器安装/启动/渲染；修复画布输入
  未接线（现在单指拖动平移、滚轮/捏合缩放）、初始状态文案、后端日志；像素 diff 证明
  平移生效。证据 `docs/validation-android.md`（含截图/日志）。
- **Web 运行闭环**：修复桥固定 WebGPU、Slint 缺 `renderer-femtovg-wgpu`、wasm 轮询
  误判设备丢失、WebGL2 MSAA present 失败；`web-dist/` 在无头 Chromium 以 WebGL2 通过
  加载/导航/双语，语言偏好持久化；B29 冒烟脚本修复。证据 `docs/validation-web.md`。
- **3D / 纸空间宿主接线**：bridge 按 `SpaceSelection` 与视图模式分派 `render`/`render_3d`，
  UI 提供 2D/3D、投影、标准视图、拖动轨道与布局选择；退化/不支持视图显式诊断。
  宿主编排者已补 `CadView::sync_session`（空间+相机+模式）。`docs/view-3d.md`。
- **透明度与代理**：acadrust `Transparency`（ByLayer/ByObject/ByBlock）经
  `DisplayFragment.alpha` 进入透明管线；代理保留全部片段与真实来源/精度。软件 Vulkan
  透明合成测试通过。`docs/render-order.md`、`docs/proxy-support.md`。

核心测试 **573 passed / 0 failed**；Wasm、Android target、fmt、clippy(0)、架构、
`check-i18n.py`（116 keys）、`check-fixture-manifest.py`、`check-workflows.py` 全通过；
`cad-render-wgpu` 在 lavapipe 下 47 passed（含透明合成）。

**集成轮 2（核心 CAD，Linux 构建）**：曲线升级为真实 NURBS + 椭圆 OCS 法向 +
仿射真椭圆 + 解析交点（`docs/curve-geometry.md`）；F15 ACIS 打通
acadrust SAT/SAB → 中性 B-rep → 平面/球/柱/环面子集离散（`docs/kernel-acis.md`，
合成夹具入 manifest）；纸空间 4 角视口/正确比例 + 空间感知测量（`docs/layouts.md`、
`docs/measure.md`）；对象捕捉六类 + HATCH 多环含孔洞填充。核心测试 **667 passed /
0 failed**。

**集成轮 3（核心 CAD，Linux 构建）**：实体颜色（ByObject/ByLayer/ByBlock，ACI/RGB）与
线宽（mm）进入渲染（`docs/entity-style.md`，线宽显式不绘制）；MTEXT 格式 run 解析与
整形（`docs/mtext.md`，堆叠分数/颜色/装饰为显式 Partial）；导入 4 角纸空间视口/完整变换/
INSERT/OCS/SOLID（闭合 round-2 `docs/layouts.md` §3.1）；网格面 `SubElementId` 拾取 +
选择高亮叠加（`docs/picking-3d.md`）。核心测试 **735 passed / 0 failed**，软件 Vulkan
**50 passed**；fmt/clippy/架构/i18n/fixtures/wasm `--lib` 全通过（仍只构建 Linux 核心，
未构建 Android/Web）。仍开放：ACIS 真实授权样本与锥面/带环球面、LINETYPE 虚线、渐变
HATCH、动态块求值、注释性缩放、打印/出图、异步可取消导入与进度、性能基准与预算实测。

**集成轮 4（核心 CAD，Linux 构建）**：LINETYPE 虚线端到端（`docs/entity-style.md`）；
纸空间布局出图到 PNG + `PLOTSETTINGS` 导入（`docs/plot.md`）；性能预算
（`cpu_bytes`/`queued_tasks`/`upload_bytes_per_frame`）真实计费 + 可复现 benchmark
（`docs/performance.md`）；渐变 HATCH 逐顶点颜色烘焙（`docs/hatch-gradient.md`）；
异步可取消导入（进度 + 过期结果丢弃，F01）（`docs/import-async.md`）。集成测试
**824 passed / 0 failed**（lavapipe），fmt/clippy/架构/i18n/fixtures/wasm `--lib` 全通过。
仍开放：矢量出图（PDF/HPGL/CTB）、复杂/嵌入形状线型、宿主异步导入面板接线、
手机内存/FPS 实测。

**集成轮 5（核心 CAD，Linux 构建）**：动态块可见性状态读取与切换（GEOMETRY 增量，
`docs/dynamic-blocks.md`）；ACIS 锥面（完整圆锥 + 截头圆锥）与环形圆环面离散，修复
截头圆锥母线缝合在环长不等时夹紧末点导致的非流形开边（`docs/kernel-acis.md`）；
注释性缩放：`Scale` 表 + `CANNOSCALE` 导入、TEXT/MTEXT 按活动比例缩放并支持
按比例位置覆盖，非文本/非法比例显式 `Partial`（`docs/annotative-scaling.md`）。集成测试
**870 passed / 0 failed**（lavapipe），fmt/clippy(0)/架构/i18n/fixtures/wasm `--lib` 全通过。
仍开放：真实授权 ACIS 样本、带环球面、非圆椭圆/样条面、矢量出图、复杂线型形状、
宿主比例切换与异步导入面板接线、真机内存/FPS 实测。

仍开放（受环境/外部依赖限制）：**F15 真实 ACIS 离散**（需内核 + SAT/SAB 解析器 + 授权样本）；
**真机**（模拟器 SwiftShader 不等同真机）；**WebGPU / 真实 GPU / 移动与桌面浏览器矩阵**；
Android surface 尺寸/安全区（U07）、SAF、量测/批注拾取与面板状态推送；自托管 GPU/Android
runner；**大型授权 DWG/字体与跨后端黄金图矩阵**（`fixtures/manifest` 现有开源 QCAD
flange 样本，但仅 `Partial`，不构成兼容性或黄金图验收）；MultiLeader；
复杂文字整形；自动保存/崩溃恢复保留策略。详见各功能 `docs/*.md` 的"未完成"。

## 下一轮优先：UI 界面未实现功能（用户指定）

用户明确要求本集成轮结束后优先补齐 UI 未实现功能。下表按源码现状刷新为**真实状态**，
不再保留过期待办。`docs/ui-redesign.md`「仍未闭环」与 `docs/panels.md` /
`docs/responsive-ui.md` 的缺口同步更新。

| # | 项目 | 状态 | 证据 / 位置 |
|---|---|---|---|
| 1 | Ribbon 自定义分组渲染 | **已实现** | `ui.components.ribbon.tabs[].groups[].commands[]` 真正渲染并按 `commandVisibility` 过滤、`display` 模式与内联溢出；未声明自定义 tabs 时内置面板不变。`crates/cad-ui-slint/ui/ribbon.slint:98–156,205` |
| 2 | overlay 开关驱动合成层 | **已实现** | `view.overlays.{axes,grid,selectionHighlight,snapHints}` 端到端门控绘制（axes/grid 真实世界参考几何，捕捉标记按 `SnapKind`；`annotations` 开关随批注子系统移除）。`crates/cad-app/src/render_scene/mod.rs:213–265` |
| 3 | interaction 门控 | **已实现** | `interaction.{pointer,touch,keyboardShortcuts}` 在共享 Slint 适配器逐项门控输入（指针/滚轮/拾取/别名展开）。`crates/cad-ui-slint/src/adapter.rs:160–167`、`tests/interaction_gating.rs` |
| 4 | 精简预设语义 | **已实现** | `minimal` 有显式组件真值表与命令白/黑名单（`canvasOnly ⊆ minimal ⊆ full`）。`crates/cad-app/src/viewer_config.rs:202–311` |
| 5 | 面板细节（布局面板；资源/3D/诊断抽屉；`responsive-ui.md` §6 缺口；软键盘/安全区） | **已完成（本轮，边界显式）** | 诊断抽屉 + 资源/3D 抽屉 + 布局比例（显示级）已接线（`docs/panels.md` §3.4）；纸空间 mesh/图像逐三角裁剪闭环（`docs/layouts.md` §4）；`safe_insets` 已折入 `CanvasMetrics`（`docs/input.md` §5）。显式缺失：3D fit、视口比例命令面、图像实体建模、Android OS inset 回调转发 |
| 6 | 原生宿主偏好持久化 | **已实现** | 原生宿主读取 `$XDG_CONFIG_HOME/yacr/config.json` 与 `preferences.json`，支持 `--config`/`--preferences` 覆盖，无 XDG/HOME 时显式不持久化。`apps/app-linux/src/host/config_file.rs:103–175` |
| 7 | 真机/浏览器移动矩阵 | **环境/证据缺口** | 需真机与移动浏览器矩阵实测；属验收环境缺口，不是代码待办。 |

注：用户批注子系统已于 2026-10-05 整体移除（见 `CAD_IMPLEMENTATION_SPEC.md` 顶部变更
记录），`docs/ui-redesign.md` 中涉及批注门控/批注面板的旧描述按“已移除”理解。

## 已定义，但尚需设计审查

- RenderTarget/HostTexture 当前仅为自有数字 token；共享 Device/Queue 的拥有者、
  token 生命周期与安全访问方式须在平台合成 ADR 中固定，不可凭 token 猜测 GPU 对象。
- AnnotationId(u128) 只表达 UUID 位宽；生成/碰撞防护由宿主服务和数据库校验完成。
  JSON UUID 编码、时间格式、extension 原始 JSON 校验尚未实现。
- FingerprintPolicy 的显式策略不意味着导入已经能自动建立锚点；fingerprint 计算需分段/异步。
- CommandPayload 的类型校验、防止 ID 与 payload 不匹配、事务权限与撤销能力仍需接线。
  当前 Application 提前检查模式与文档身份，但 Work 模式执行仍返回 NotImplemented。
- RepresentationProvider 的字体/样式上下文目前只提供最小配置，需要扩充自有只读 views。
- DB 中已有基础实现应保留并复查几何边界/块实例/验证完整性，不代表工业语义已验收。
- 数据库/几何已有基础真实实现与合成测试，代理也已有有界 framing/记录回放代码。
  这些是在恢复过程中保留的新增实现；需审查实际支持范围，不能据此宣称真实 DWG/工业兼容。
- 扩展点包含 Importer/Tool/CommandHandler 等；注册/优先级/冲突策略需统一实现。
- PropertyProvider、字体 shaping、缓存容量、Worker 二进制传输、Journal codec、schema migrations
  都需要完整接线和验收，不允许以现有结构体宣称闭环。

## 接线验收

业务不调用 draw；批注确认一次事务，取消零事务；UI 回填不触发重复命令；
revision 不连续重建；GPU 重建不改数据库；设备失败和退出保护未保存批注；
局部失败保留对象级报告；未知源单位显示图纸单位。Android/Web 真机与浏览器分开记录。
## 2026-10-04 Linux release 后续排查（部分优化，问题未闭环）

- 用户手工 release 仍卡死；补窗口启动后打开和实际 pointer/click 探针，基线 release
  重现 17 秒事件循环停顿。前轮 `--open` 回归不覆盖加载响应。
- 增加不透明虚线 `LineSegments` packed 表示及弧长索引，所有 consuming boundary 识别
  独立端点对；默认 provider 与透明 run 顺序不变。CPU 场景约 17.6→14.0 秒。
- 拾取改为借用几何，约 72 万 items 的收集 1.85→0.20 秒；debug 点击仍约 2.05 秒超阈值。
- release `--open` 加 pointer/click 短回归通过：17.68 秒首次帧，1.58 秒点击，max gap 1.60 秒；
  **不是窗口启动后打开/加载响应通过，也不是手工或视觉验收**。
- 尚未实现 native 异步准备、空间 broad phase、GPU 上传背压；详细证据/限制见
  `docs/linux-dwg-ui-freeze.md` 最新小节。修改未提交；不覆盖前轮与并发工作。

## 2026-10-14 CI `web-host-contracts` 修复（旧源码契约迁移）

- CI run `38495981291` 仅 `web-host-contracts` job 失败：`scripts/test-host-panel-parity.mjs`
  8/60 fail（`not ok 1–6/8/9`），其余 job 成功/跳过。
- 根因：该 source-text 契约仍锚定 P2 前结构（`HostCore::push` + `cad-host/state.rs` 直接 setter +
  `set_overlay_visibility(handle.effective_config()...)` + 平台 overlay `core.push()`）。P2/P4 后共享腿
  改为 typed `HostUpdates` → `EngineAdapter::apply_updates`/`UiHandle::apply_host_state`，render overlay
  gate 在 `cad-host/state.rs::sync_render` 的 `OverlayInputs`/`OverlayVisibility::from(effective config)`，
  平台 overlay 在 `binding.flush_updates()` 之后只补 host-specific 通道。逐通道核对无真实漏线。
- 修复仅 `scripts/test-host-panel-parity.mjs`：重指真实 owner，断言 overlay 顺序与「不覆盖 authority」，
  并加 negative-fixture 合同（丢 channel/换 order/抢 authority 必须失败）；未删/skip/放松断言、未改 workflow
  gate、未恢复旧 `core.push()`。exact Node 命令终态 60/60 pass，`git diff --check` 通过。详见
  `docs/validation-engine-decoupling.md` 末节。browser RAF/GC/WebGPU 等仍未验收。

## 2026-10-14 React + Tauri 原生 Linux 探针实验（进行中，未验收）

用户批准以 Tauri2 + React 替换 Linux 桌面 Slint UI，并新增 X11/XWayland-only GPU 合成探针；
**不允许 WASM、不允许逐帧 readback**。决策见 `docs/adr/0006-react-native-linux-experiment.md`。

本轮交付（scope：`apps/app-react-linux/**`（除 `ui/**` 归 designer）、根 `Cargo.toml`、
`Cargo.lock`、ADR 0006、本段）：

- 新增 workspace member `apps/app-react-linux`（bin `yacr-react-probe`，Tauri 2.11.6）。
  强制 `GDK_BACKEND=x11`（未设置则置为 x11，设置为其它值报错退出）；透明顶层窗口 +
  全窗透明子 Webview；单 Tauri 事件循环 `MainEventsCleared` 主线程驱动
  `cad-render-wgpu::SurfaceProbePresenter`（顶层 XID 路径，打印 `[probe] top-level X11 XID ...`）。
  生命周期：resize / 零尺寸暂停 / minimize 恢复 / destroy 先 `stop()` 再释放 / GPU 失败显式
  写入状态。协议见 ADR 0006（`probe_status`、`probe_input`）。
- 依赖：新增 `tauri ~2.11`、`tauri-build ~2.6`、`raw-window-handle`。Cargo.lock 将 tauri 内部族
  固定为 `tauri-build/macros 2.6.3、tauri-runtime 2.11.3、tauri-runtime-wry 2.11.4、
  tauri-utils 2.9.3、tauri-codegen 2.6.3`（镜像最新小版本与 2.11.6 破坏性不兼容；未用 Cargo patch）。
- 未改渲染器：沿用上轮 `crates/cad-render-wgpu/src/native_presenter.rs`（本文件所在 crate 的
  `lib.rs`/`native_presenter.rs` 改动来自上一轮，非本轮新增）。

已运行证据（本机 debug）：

- `cargo check -p app-react-linux --all-targets --locked` 通过；
- `cargo test -p app-react-linux --lib --locked` → 10 passed / 0 failed（序列号/有限参数/终结事件契约）；
- `cargo clippy -p app-react-linux --all-targets --locked -- -D warnings` 干净；
- `python3 scripts/check-architecture.py` → 28 包 `Architecture OK`。
- 系统 dev 依赖（`libwebkit2gtk-4.1-dev`、`libgtk-3-dev` 等）由主协调者后台 apt 安装，非本轮安装。

NOT RUN（不得据此宣称验收）：

- **真实窗口 60s 探针（XWayland + Quadro P620）未运行**：待 designer 完成 `ui/dist` 后由主协调者
  统一验证（本机无 `ui/dist`；debug 因设 `devUrl` 才可编译，release 才嵌 `ui/dist`）。
- **GTK child 桥未实现**：仅顶层 XID 对照；child drawing area 的 XID 复用方案待 librarian 核实后补充。
- GTK/WebKit 叠加与 Vulkan surface 的平台合成、minimize/restore/drag 真实交互、真实 FPS 计时均未测。
- 未跑 WASM（本实验不允许）。探针不是 CAD presenter，不绘制 CAD 几何、不代表 CAD 能力。

### 真实窗口验收（2026-10-14，Gate 1 未通过）

证据：`/tmp/opencode/yacr-react-probe-20261014T044550Z/`。真机 XWayland + Quadro P620：
顶层 XID `0x1800003` viewable；`backend=vulkan adapter=Quadro P620 discrete_gpu`；真实 IPC
输入可用（前端 `resize seq=1` + 合成 `pointer_down/move/up`+`wheel`，均 accepted、marker 变化、
up 为 terminal；注入为合成、非真实鼠标）。

阻塞项：`presenter.frame()` 每帧阻塞 **~998ms（~1fps）**——渲染器 ignored 用例在普通 Xlib
窗口 0.37s 内完成 init+4 帧，而 GTK 顶层/`GtkDrawingArea` 均 ~1s/帧，说明停顿在
**XWayland 合成 present 路径**而非 GTK 归属。一次有界修复（GTK 桥 B：`GtkOverlay`+windowed
`GtkDrawingArea`+webview reparent）已实现测试，**未修复**（surface 有效但仍 ~1s/帧），按约束
停止并默认关闭（`YACR_PROBE_GTK_BRIDGE=1` 才启用）。`set_size` 返回 Ok 但窗口不变
（960x600）；minimize 可暂停、`unminimize` 未恢复。当轮截图工具缺口（rootless XWayland root grab
近全黑、GNOME D-Bus 截图 AccessDenied、无 grim/scrot/import/xwd）→ 当轮无像素/视觉证据；
**后续 composited 轮已取得 full compositor 像素且 composite FAIL，见下节**。
NOT RUN：视觉验收、真实鼠标、GTK/WebKit 叠加合成、60fps 同步刷新。详见 ADR 0006。

### 有界 debug + 安全修正（2026-10-14 第二轮，Gate 1 仍未通过）

Stage 计时实测 root cause：`Queue::present`（Fifo）阻塞 **~1.000 s**（其余阶段 µs 级）；
同一 surface 改 `Immediate` 后 present median **77 µs**、10 s **62,404 帧**。宿主改用
`ProbePresentMode::MailboxElseImmediate`（supports 无 Mailbox→Immediate，显式日志）后，
Tauri 全 React 连续 11 s 达 **~60 fps（651 帧）**，真实 IPC 输入（resize + 合成
pointer_down/move/up + wheel）全部 accepted。安全修正：presenter 改 main-thread
`thread_local`、共享状态去 GPU、`CloseRequested`/`Exit` 显式 `release_presenter()`
（surface-before-window）、redraw single-pending、child 用真实 size、`scope.pop` 有界非阻塞、
每阶段计时；GTK bridge 原型已删除（撤回，其慢同为 Fifo present）。窗口 **drawable 抓图成功**
（`XGetImage(window)`，非 root；960x600 全非黑，颜色与 shader 一致，marker 质心 x 278→312）；
root 抓图仍黑（rootless XWayland）≠ 窗口不可抓。仍阻塞：`set_size` 未观察到尺寸变化（原因未定）、
`unminimize` 不恢复、合成正确性与真实鼠标未验。NOT RUN 同上；仍不接 CAD 核心。

### WASM/跨目标隔离构建修复（2026-10-14，bounded 机械）

`cargo check --workspace --lib --target wasm32-unknown-unknown --locked` 曾 exit 101
（`app-react-linux` 无条件拉 `tauri/wry/tao`）。修复：`app-react-linux` 的
`tauri/cad-render-wgpu/raw-window-handle/serde` 与 `tauri-build` 改为 Linux-target 依赖；
`build.rs` 以 host cfg + `CARGO_CFG_TARGET_OS`（目标）双重判断，仅 Linux 目标调用
`tauri_build::build()`；`lib.rs` 全模块 `cfg(target_os="linux")`（其它目标空 crate，不 pretend）；
`main.rs` 其它目标显式 unsupported 退出。固定 `tauri =2.11.6`/`tauri-build =2.6.3`。
未改 `ui/**`/native presenter/窗口行为。证据：workspace wasm lib check **exit 0**
（`/tmp/opencode/yacr-react-wasm-check.log`）、workspace all-targets exit 0、app 10/10、
renderer 59/2、workspace clippy exit 0；`cargo fmt --all -- --check` 仅既有陌生
`debug_fixture.rs` 失败（未编辑，`/tmp/opencode/yacr-react-fmt-check.log`）。

### Compositor 截图取证（2026-10-14 第三轮，**composite FAIL**）

本轮**取得 full compositor 像素证据**（更新此前"无像素"结论），但合成仍失败。
方法：`xdg-desktop-portal` `org.freedesktop.portal.Screenshot` v2（`interactive=false` +
`handle_token`，先订阅 Response 再 invoke，20s 超时），6 张 1920x1080 全桌面截图
`/tmp/opencode/yacr-react-composited-20261014T082703Z/shot1..6.png`。
授权（用户明确授权，均已恢复）：临时 `loginctl unlock-session 1`（仅信号，未读密码）+
临时 `PermissionStore.SetPermission screenshot true screenshot '' ['yes']`；finally
`DeletePermission screenshot/screenshot ''`（原 `''` 不存在），`wechat: ['yes']` 保留未动
（verify 回到 `{'wechat': ['yes']}`）；`lock-session` 并 verify `GetActive=true`。
仅关闭本轮 probe（进程组），无残留；截图为私有桌面，**仅本地保存未上传**；未改
unsafe-mode/gsettings/图形 config、未编辑源码。
observer 结论（shot2..5.png）：960x600 **客户区仅网格 + orange marker + corner markers**；
顶部白色 “yacr react probe” 条是 **WM decoration 非 React ribbon**；**React ribbon/命令框/
DOM 浮层均不可见且未被外窗遮挡**；marker 逐帧向左移动 → **Gate composite FAIL**：
**无法区分"被覆盖"与"未加载"**，IPC 仅证逻辑不证可见。
native Wayland / DWG / 真实鼠标 / 正式 CAD 仍 **NOT RUN**；不继续代码实现；
**不据此宣称 Tauri 整体不可行**。详见 ADR 0006。

## 2026-10-14 React/Tauri 原生桌面路线文档契约增补（docs lane，未验收）

本轮为**文档 lane**（不改代码/manifest/lock/UI），把用户已确认的 React/Tauri 原生桌面路线契约
写入 spec 与 ADR，供未来 Windows/macOS 实现参照；Linux 探针代码修复由**另 lane** 进行，不在本 lane。

- `CAD_IMPLEMENTATION_SPEC.md`：新增 §21「实验范围增补：React/Tauri 原生桌面宿主」——授权
  React 替代 Slint（桌面实验）、Tauri 壳、native Rust CAD、renderer-only wgpu、no WASM、no 逐帧
  readback；覆盖 §4.11/ADR 0005 的“React 不在本轮”历史限制（**仅限桌面实验路线**），保留 db 唯一
  权威、单 presenter、增量语义、显式 Unsupported 等核心不变量。§2.1.3、§4.11 就地加最小指针，
  未改 P1–P4 历史。
- `docs/adr/0006-react-native-linux-experiment.md`：追加「React 原生桌面宿主跨平台路线契约」增补
  （共同架构、平台差异、RapidRAW 参照、合成/坐标/输入/生命周期契约、明确不承诺、验收要求、
  未来 implementation checklist、支持状态矩阵）。
- `docs/architecture.md`：最小增补一条实验宿主指针。

本轮状态（如实记录，不得外推）：

- 用户授权：React 替代 Slint 目标、Tauri 桌面壳、native Rust CAD、renderer-only wgpu / noWASM /
  no readback；本轮 **X11/XWayland 获批**。
- Linux：顶层 Vulkan 覆盖 React **合成 FAIL**（full compositor 像素已取得，无法区分“被覆盖”与
  “未加载”）；**独立 GTK nativechild 桥未验**。
- Windows/macOS：**NOT IMPLEMENTED / NOT RUN**。**禁止 readback 兜底**（未授权）。
- 不得把跨平台同原生叠层结构、synthetic/单平台结果承诺为已验。

验证：`git diff --check` 见本轮报告。

## 2026-10-14 React 原生 Linux 宿主 closeout（bounded prototype accepted，含限制）

用户目标：React 替代 Slint + Tauri 桌面壳 + native Vulkan；基本 UI 已 implemented 并 validated 为
**bounded prototype**（**非完整 CAD 产品**）。**旧「顶层透明 alpha 全屏叠加」FAIL 保留为历史**；
本轮实际方案为 **Linux GTK 中央 native 分区（nativechild）在 React UI 上**，React DOM（ribbon、
底部命令条）与中央 native CAD **co-visible**。契约见 ADR 0006 第四轮节与规范 §21；使用说明见
`docs/react-linux-app.md`。

最终证据目录：

- 实际 CAD `/tmp/opencode/yacr-react-cad-acceptance-20261014T112301Z`；
  输入/键盘终轮 `/tmp/opencode/yacr-react-cad-input-final-20261014T115552Z`。
- 生命周期 `/tmp/opencode/yacr-react-lifecycle-20261014T101035Z`；
  WM restore `/tmp/opencode/yacr-react-restore-20261014T102828Z`。
- 门禁 `/tmp/opencode/yacr-react-final-gates-20261014T112155Z`。

已验收（bounded，含限制）：

- synthetic four-line DWG 打开 `entityCount=4`、`backend=vulkan`、`adapter=Quadro P620`；React
  ribbon + 底部命令条与中央 native CAD **co-visible**；OS pan / wheel（上/下）/ Fit 按钮与 typed
  命令生效；未知 `BOGUS` 走 **not supported**（不空成功）。
- React **OPEN** native portal picker → licensed flange DXF：`entityCount=419`、**同进程**
  `importCount=2`；非法坏 DWG 显式 `CorruptData(...)` **保留旧 DB**；Cancel 保留 DB、报清晰错误。
- resize unrealize/remap → detach/attach **Drop 先于 native**；正常 close/Exit 同样先 Drop；
  minimize→restore 仅 **WM pager 激活** PASS（Tauri `unminimize()` API 在 XWayland 上**未采纳**，
  限制）。GPU 真实 P620/Vulkan/Immediate，**无软件后端**。
- 错误策略 hardening：同 presenter generation bind 失败**最多重试一次**、错误保持可见、新
  generation 可重试；**closing/failed 禁止 bind**。

不夸大 / NOT RUN：

- 60fps 仅上一轮 **PROBE** 10s 实测；**实际 CAD root 不宣称 60fps**（无稳定 CAD bench）；renderer
  GPU frame present 统计**不外推**。`importCount` 为**成功文档观察**非 parser 调用；**无运行时重解析
  的直接插桩 NOT RUN**，仅架构导航路径 + 600 synthetic 契约 counter 不变 proxy。
- observer：four-line **basic smoke PASS** + pan/fit；flange **PARTIAL**（主几何可见、无明显翻转，
  但布局与参考有差异：缺 paper frame/title block 右细节、大量文字），**不构成 full visual golden /
  vendor DWG**；当前是否模型空间**原因 UNDETERMINED**。popup 跨 CAD **Unsupported**；诊断在
  ribbon 内；**无 Android/Web 迁移**；不从 HostCore 资源完整兼容外推。

测试/门禁（per logs）：app **71**（契约合成，含 2 条新增 bounded-bind contract）、UI 16、
renderer 64、WGSL 4；check/clippy/fmt PASS。**71 为契约合成计数，与此前 GPU runtime 证据分开，
不互相替代**。门禁 host/wasm/clippy + 4 static 均 0；`fmt` 仅既有陌生未跟踪 `debug_fixture.rs`
失败（未编辑）。

新增入口/文档：`scripts/run-react-linux.sh`（检查 DISPLAY + 始终 `GDK_BACKEND=x11`；`node_modules`
缺失时 `npm ci`，再 `npm run build`；`cargo run --locked -p app-react-linux --bin yacr-react-probe
--features tauri/custom-protocol -- "$@"`；**不**装系统依赖、**不**解锁、**不**截图）与
`docs/react-linux-app.md`（构建依赖、用法、支持范围、坐标/DPR 契约、portal helper 授权、
隐私/锁状态）。旧 `apps/app-react-linux/scripts/run-probe.sh` **未改**。

Win/macOS 仍 **NOT IMPLEMENTED / NOT RUN**；macOS 候选 **Metal**（非 Vulkan）；GPU **无 CPU
readback**；React 宿主**无 Slint presenter**，**旧 Slint 代码未删除**。清理：截图临时权限恢复
baseline；**保留 unknown/desktop 原锁状态**（最近启动为 unlocked → 保持 unlocked，**不得声称
always locked**）；全屏截图**仅本地未上传**；外部 `debug_fixture.rs`/`debug-block.dwg` **未索引/未用**。

## 2026-10-14 React 原生 Linux 宿主 final closeout（窄 2D fastpath + 性能基线，latest accepted）

在第四轮基础上，本轮实现并验收**窄 2D 导航 fastpath**与**实测性能基线**；**旧各轮 FAIL history 与
标签全部保留**，本段为最新 summary。契约见 ADR 0006 第五轮节与规范 §21；使用说明见
`docs/react-linux-app.md`。

用户 4 项请求（已 implemented）：

1. 可见 **FPS 真实计数**：`delta frameCount / performanceNow`、rolling **2s**、warmup **1s**，
   `reset`/`stopped`/`error` 显示**不可用**；**非**占位 React 动画。
2. UI chrome 中性灰 **`#dedede`**；CAD 由真实 GPU **clear 纯黑 `#000000`**（不透明，全空间默认，
   **无**旧背景偏好覆盖）。
3. 默认窗口**最大化**：helper 前验证 `_NET_WM_STATE MAXIMIZED_HORZ/VERT`，尺寸 **1853×1011**，
   `fallback=false`。
4. 入口脚本由**用户自改**为默认 `--release` 并**保留不变**（本 lane 未改该脚本）；`debug` 快速运行
   可用直接 cargo 命令（可选）。

窄 2D 导航合同：同一权威底图（GPU-resident、同一 DB revision、fonts/images/layers/space 一致），
Ready fastpath 取最新 camera、不展示旧 doc；overlay 后缀**临时省略**并以显式
`FrameCompleteness::BaseOnlyPendingOverlays` 标记；`PresentedFrame.view_revision` = 实际 camera；
`CurrentScene` = **GPU 已提交当前场景**（非渐进全像素），当前 drawing `presented=false`+`baseOnly`；
严格 CPU-ready/GPU-未提交返回 `None`、不回旧底图；restore 同底图 overlay 提交（静态 base prefix
保留）；device change 阻塞至新上传；**3D 严格路径不变**。Root lag：严格 view prep token advancing，
多数帧 `None`，实际输入 ~23µs / render ≤3ms。**取舍**：拖拽 transient grid/axes/highlights/snaps/
previews 可能暂时隐藏，prepared 后恢复（<30ms 窗口测量，非精确延迟）。

性能（source hash locked）：基线 `/tmp/opencode/yacr-react-baseline-perf-20261014T125447Z`、postfix
`/tmp/opencode/yacr-react-postfix-perf-20261014T135021Z`；同 4s/494 events/~120Hz，fourline/flange
drag waiting **65.3%/67.8%→0.0%/0.0%**、presented/s **21.0/19.5→60.7/59.9**；
`view_current==view_rendered` **709/701 帧 0 mismatch**；post-release waiting **0**、无 history、
位置稳定（148/146 帧）；`input_age_us`（Rust admission→drain）p95 **15.722/15.022ms**（**非
input→visible**）；scope pop p50/max **1/3µs**；frame p95 **0.873/3.132ms**；UI FPS **60/61**；
observer max black/gray/align PASS。**input→visible p95<50ms UNVERIFIED**（portal 静态截图非真实
视频捕获）；**不宣称无性能回归 / full visual golden**（vendor 仍 `Partial`）。

测试/门禁：engine **76** pure/synthetic + **5** ignored（real P620，显式运行全部 **PASS**）；
App **73** contracts（latest host telemetry）、renderer **65**（lib）、UI **22**；WGSL shaders
unchanged（新代码 Vulkan，无 CPU）。门禁 `/tmp/opencode/yacr-react-drag-gates-20261014T135430Z`
host/wasm/clippy + 4 static / diff **均 0**；`fmt` 仅既有陌生未跟踪 `debug_fixture.rs` 失败
（未编辑）。**Oracle 独立 accepted** post-GPU publication gate。**device fastpath 测试不等于平台/
全产品验收**；不承诺未支持平台（Win/macOS 仍 future）字段；跨平台同契约、仍引用 RapidRAW 固定
版本、**Linux 无 readback**。清理：会话本就 unlocked → **保持 unlocked**（保留原状态）；portal 权限
恢复（保留 `wechat`）、虚拟 uinput 与自身 PID 已清理；全屏截图**仅本地私有，未远程上传**。
