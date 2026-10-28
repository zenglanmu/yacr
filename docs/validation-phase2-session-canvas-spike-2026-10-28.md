# 验证记录：**阶段 ⑧ phase 2 spike** —— 真实桌面会话在 wasm 内自绘 canvas（2026-10-28）

> 范围：**spike（调试导出）级**、**合成 fixture**、**headless Chromium + SwiftShader（软件渲染）**、
> `dpr=1`、`800x600`。这是 ADR 0024 阶段 ⑧ 的可行性验证，**不是**生产切换、**不是**产品验收、
> **不是** GPU/像素正确性/性能结论。本记录只写**实际执行**的观测；未执行项在 §7 显式列出。

## 构建开关（2026-10-28 收尾补充）

spike 默认**不进生产 wasm**：`debug_session_*` 导出、`debug_session` 模块与 `spike_tests`
全部挂在 cargo feature **`debug-session`**（`default = []`）后面。实测代价：带 feature 的
`app_web_bg.wasm` **13,409,586 B**，默认构建回到 **9,655,292 B**（即线上产物体积不变）。
证据构建方式：`cargo build -p app-web --target wasm32-unknown-unknown --release --locked --features debug-session`。

## 1. 问题与结论

Phase 1 已证明 `cad_ui_core::session::CadSession`（桌面会话：真实命令路由、`cad-db` 权威、
`RenderSessionHandle`）能在 wasm 内由**字节**构造（`CadSession::from_bytes`，见
`apps/app-web/src/platform.rs` 的契约测试）。Phase 2 要回答的是另一半：

> 这个桌面会话能不能**拥有一块浏览器 `<canvas>` 并真的画出东西**？

**答案：能。** 本次运行里，真实会话在页面 canvas 上呈现了第一帧（`presented=1`），
`LINE` 经**真实命令路由**被受理，两个点经**真实输入缝**（`CadSession::handle_input`）捕获，
`CONFIRM` 提交后文档实体数 **4 → 5**，下一帧的合成像素**确实变化**（3622 / 480000 px 不同，
0.755 %），截图中能看到点击两点之间的那条斜线。

## 2. 调试导出（spike，附加、不改生产）

新增 `apps/app-web/src/debug_session.rs`（wasm-only）与 `lib.rs` 中的 5 个导出：

```text
debug_session_start(canvas_id, css_w, css_h, dpr)  -> JSON status (async)
debug_session_tick()                               -> JSON status
debug_session_command(text)                        -> JSON status
debug_session_pointer(phase, x, y)                 -> JSON status
debug_session_status()                             -> JSON status
```

生产的 `start`/`open_bytes`/`resize`/`pan`/`zoom_at`/`status`/`document`/`shell_*` **一行未改**，
chrome、预算、Worker 路径、布局均未触碰；viewer 会话保留，两者**可同时存在**（§5 已验证）。
非 wasm 目标上这 5 个导出全部以显式 `NOT IMPLEMENTED` 拒绝（`spike_tests` 契约测试）。

## 3. 环境与产物

| 项 | 值 |
|---|---|
| 构建 | `bash scripts/build-web.sh`，`app_web_bg.wasm` **13,409,586 B**、`app_web.js` 139,264 B |
| 体积成本（实测） | 同一 `--release --locked` 构建，**移除 spike** 后为 **9,655,292 B** ⇒ 附加导出 + 真实会话（`cad-ui-core::session`/`cad-host`/命令词表）让模块 **+3,754,294 B（+38.9 %）**。这是**两者共存**的附加成本，不是生产切换的净成本（切换是**替换** viewer，`cad-app::render_scene`/导入等仍然共享） |
| 浏览器 | Playwright **Chromium**（headless），`--use-gl=angle --use-angle=swiftshader --enable-unsafe-swiftshader` |
| 渲染器 | `ANGLE (Google, Vulkan 1.3.0 (SwiftShader Device (Subzero)), SwiftShader driver)` ⇒ **软件渲染** |
| 页面 | 800x600 canvas，`devicePixelRatio = 1`（`surface=[800,600]`） |
| 夹具 | `fixtures/dwg/synthetic-four-lines.dwg`（4 条 LINE，无文字、无图像） |

## 4. 实测（`results.json`，21/21 检查通过）

- `debug_session_start` → `{ok:true, label:"synthetic-four-lines.dwg", entityCount:4, surface:[800,600]}`；
  启动内的一次 frame attempt 已呈现 **1** 帧（`presented=1`，`waited=0`）。
- 第一帧：`presented=2`（第 2 次来自 `debug_session_tick`），`presenterFrames=2` 与会话自报一致；
  合成像素**非单色**：distinctColours **17**、非众数像素 3940 / 480000（0.82 %）。
- `debug_session_command("LINE")` → `tool="draw.line"`，`sessionError=null`（真实路由受理）。
- 4 次 `debug_session_pointer`（down/up ×2，坐标 `(220,200)`、`(560,400)`）全部 `sessionError=null`。
- `debug_session_command("CONFIRM")` → `tool=null`、`entityCount` **4 → 5**（+1，恰好一条）。
- 编辑后新帧：`presented=3`；像素 distinctColours 17 → **40**、非众数占比 0.82 % → **1.57 %**；
  两张 canvas 截图逐像素差 **3622 / 480000（0.755 %）**，远高于 0.05 % 阈值。
- 人工查看截图（本 agent 实际读取）：
  - `canvas-first-frame.png`：黑底 + 夹具的 3 竖 1 横亮线；
  - `canvas-after-line.png`：同上，**新增从 (220,200) 到 (560,400) 的斜线**（正是两次点击的位置），
    外加宿主光标十字与 pickbox 覆盖层。
- 堆：`heapBytes`（wasm 线性内存高水位）在 fixture 打开并出图后 = **2,752,512 B**。
  这是**模块自身 wasm heap，不是进程 RSS**；且同一模块随后启动 viewer 会把它推到 3 MB 以上，
  所以该数字**只**代表 spike 会话单独运行时的读数，不可与 viewer 的 `heap_after_open` 直接对比。

证据目录：`/tmp/opencode/yacr-phase2-20261028T095020Z/`（`results.json`、`01-start.png`、
`02-first-frame.png`、`03-after-line.png`、`04-both-sessions.png`、`canvas-*.png`、`page/`、`run.cjs`）。
驱动脚本 `run.cjs` 与其页面是**一次性/非仓库代码**；截图仅本地保存、不上传。

## 5. 使用的缝（就是生产要用的那些）

| 环节 | 缝 |
|---|---|
| 设备 | `CanvasPresenter::device_queue()` → `RenderSessionHandle::attach_device(device, queue)` |
| 表面度量 | `BrowserPlatform::surface().report([css_w, css_h], dpr)` + `CadSession::set_canvas_metrics` |
| 打开 | `CadSession::from_bytes(options, platform, handle, clock, BytesDocument{..})` |
| 命令 | `CadSession::command(text)`（真实选择器：`LINE` / `CONFIRM` / `CANCEL` / `FIT` …） |
| 输入 | `CadSession::handle_input(&ProbeInput{kind,x,y,button,sequence})` —— 与桌面 `probe_input` IPC 同一缝 |
| 时钟 | `RequestClock = Rc<dyn Fn() -> f64>`，用 `web_time::Instant`（浏览器即 `performance.now()`），**不用 `std::time`** |
| 帧 | `CadSession::tick()` → `apply_ready_updates()` → `render(logical, scale)` → `PresentedFrame.texture` → `CanvasPresenter::present_texture` |

**一帧需要三个调用，不是一个**：`tick`（宿主 pass + 场景准备）、`apply_ready_updates`
（分期上传，桌面宿主在 present bridge 里驱动）、`render`。`render` 在场景未提交前返回
`Ok(None)`——spike 把它计为 `waited`，**绝不**当成已呈现。

## 6. 与 viewer 的共存（已验证）

同一 wasm 模块内先跑 spike 会话（entityCount=5、presented=3），再调用生产 `start('cad-canvas-2', 200, 150, 1)`：
viewer 启动成功（`presented=1`、`target=html-canvas`、`document().label` 为同一夹具），
随后 spike 会话状态**未受影响**（仍 `entityCount=5`、`presented=3`、presenter 计数 3）。
`04-both-sessions.png` 中两块 canvas 各自出图。

## 7. 生产切换的阻塞项（按可测性排序）

1. **ADR 0025 预算没有覆盖这条路径**：`from_bytes` 不做 `budget::check_input_bytes`
   （解析前）也不做 `check_heap_after_build`（建场后），这两项目前只在 viewer 的
   `load_within_budget` 里。切换前必须把预算搬到共享入口，否则大图会在**没有预算拒绝**的情况下解析。
   更关键：`CadSession` **保留数据库**（`entityCount` 才能读），而 viewer 在出图后释放
   （`db_retained:false`、实测 1.43 GB）。因此同一 `complex-test.dwg` 的 wasm 峰值**大概率更高**，
   必须实测而不是推断——本次**没有**测。
2. **真实图纸的字体/图像**：`BrowserResources` 在图纸请求字形/位图时显式拒绝
   （`unsupported-resources`），打开不失败但文字**不整形**、图像不解码。生产切换需要浏览器侧
   字体获取与图像解码策略（phase 1 文档已列为不在范围内）。
3. **输入管线**：`apps/desktop-ui` 的唯一漏斗 `.native-input` 在生产 X11 宿主里把事件发给
   `probe_input` IPC，而 `web-adapter.ts` 目前对 `probe_input` 一律 `unsupported-runtime` 拒绝。
   切换需要适配器把 DOM 指针映射成 `ProbeInput`（本次验证的正是这个形状），并保证
   `set_canvas_metrics` 在 resize 时更新（`metrics_revision` 会随点标记，过期会被宿主拒绝），
   以及现有的 popup 门禁。
4. **popup / shell 面**：`shell_status`/`shell_command` 现在由 viewer 事实投影（`entityCount:null`、
   只有 `FIT`/`ZOOMIN`/`ZOOMOUT`）。真实会话能投影真实的 `entityCount`、tool、selection、
   command log、`standard_view`，也能打开 POPUP 文本编辑（`selected_text_edit` 等），
   但把这些接到 chrome 是独立的一段工作，本 spike **未接线**。
5. **帧循环与 resize**：viewer 每次导航同步出图；真实会话需要 rAF 循环
   （`tick` → `apply_ready_updates` → `render` → `present`）+ `ResizeObserver`
   （`presenter.resize` + `set_canvas_metrics`）。本次只跑「开图/编辑后各出一帧」，**没有**持续帧循环、
   **没有** resize、**没有** pan/zoom/FIT 走真实会话。
6. **`open_bytes` 语义映射**：viewer 现在是「新 presenter + 全新会话」（旧 presenter 先 drop）；
   真实会话的对应物是同一个会话内 `HostCore::deliver_external_open`（失败不破坏在开文档）。
   语义更贴近桌面，但需要一个新的「在活会话上打开」入口（并在此之前跑 byte 预算），
   本次只有构造期的 `from_bytes`。
7. **Worker 隔离**：spike 只跑主线程载体；同一份代码在 Worker 的 wasm 实例内应可复用
   （单线程），但 Chromium 之外（WebKitGTK 无 Worker 内 WebGL2）仍是既有边界。

## 8. NOT VERIFIED（不得被读成结论）

- **软件渲染**：SwiftShader。硬件 GPU、驱动、vendor 结论一律没有。
- 单一 Chromium 版本 + 单一夹具（`synthetic-four-lines.dwg`）；**没有**真实 DWG、大图、文字/图像图。
- **像素正确性**（颜色空间、抗锯齿、相机、比例）**未判**；只断言「非单色」与「编辑前后确有差异」，
  并**人工看过**两张截图。
- `dpr=1` 单值；多显示器/分数 DPI 未测。resize、持续帧循环、`pan`/`zoom_at`/`FIT` 走真实会话未测。
- 内存预算：只读了 fixture 的 wasm heap 高水位（2.6 MiB），**没有**任何 `complex-test.dwg` 数据。
- 帧时/性能：`ticks`/`frames`/`waited` 是计数，**不是**性能结论。
- Android/WebKitGTK/X11 宿主、chrome（React）、popup、Worker 载体均未接线、未运行。
