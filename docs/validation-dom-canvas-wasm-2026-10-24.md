# 验证记录：DOM-canvas / wasm-WebGL2 CAD 会话（2026-10-24）

> 范围：**synthetic fixture 级**、**X11=XWayland**、**有界**。这是 ADR 0024 阶段 ②/③/⑦ 的一部分，
> 不是产品验收，也不是 X11 宿主验收（宿主接线在本记录写作时仍在进行）。
> 本记录只记录**实际执行**的观测；未执行项在最后一节显式列出。

## 1. 环境

| 项 | 值 |
|---|---|
| 会话 | GNOME/Mutter，Wayland 会话；被测客户端为 **X11/XWayland**（`GDK_BACKEND=x11`、`DISPLAY=:0`） |
| 显示器 | `eDP-1` 1920x1080，`scale=1` |
| GPU | Quadro P620（`nvidia-smi`：4096 MiB，驱动 580.178.04）；EGL vendor 同时存在 `10_nvidia.json`/`50_mesa.json` |
| WebKitGTK | 2.52.6 |
| Chromium | 141.0.7390.37（Playwright，headed，`--use-angle=gl`） |
| 工具链 | rust 1.99.0、`wasm32-unknown-unknown`、`wasm-bindgen` CLI **0.2.129**（与 crate pin 一致） |

## 2. 构建产物

- `cargo build -p app-web --target wasm32-unknown-unknown --release --locked` → rc=0，
  `app_web.wasm` **11,190,690 B**。
- `wasm-bindgen --target web --no-typescript` → `app_web_bg.wasm` **9,538,257 B**、`app_web.js`
  **122,996 B**；导出严格为 `start`/`resize`/`zoom_at`/`pan`/`status`（`start` 返回 Promise）。

## 3. WebKitGTK 2.52.6 / X11（共享 wasm 会话；X11 宿主将来复用的同一引擎）

服务 `apps/app-web/web`（HTTP，`127.0.0.1:8933`），用一次性 GTK3+WebKit2 探针（`/tmp/opencode/wgpu-spike/harness.py`，
**非仓库代码**）加载页面，`start()` 后读页面状态行，拍照走 **Mutter `ScreenCast`**
（`scripts/screencast-capture.py`，同 UID）。

- 启动状态行：`start: ok  backend=gl  format=Rgba8UnormSrgb  physical=360x240  batches=4  draws=1  presented=1`
- 像素（ScreenCast 单帧裁剪到探针窗口 813,418,360x240）：
  - 画布区为黑底；亮像素 **613 px**；竖线列位于 **x = 8 / 122 / 237 / 351**，与
    `fixtures/dwg/synthetic-four-lines.dwg` 的 4 条 LINE 一致（4 batch、1 draw call）。
- 导航（**真实 DOM `wheel` 事件** → `main.js` → wasm `zoom_at`，非直接调用）：
  - 状态行变为 `zoom: ok … presented=2`（重绘并重新 present）
  - 竖线列变为 **x = 108 / 251**（外侧两条移出视口），亮像素 **420 px**；窗口内 **3,673 / 86,400** px 变化。
- 结论：**该版本 WebKitGTK + XWayland 下，acadrust 导入 → 显示表示 → wgpu WebGL2 → canvas 出图与
  缩放平移成立**（仅对上述环境与 synthetic fixture）。

## 4. Chromium 141（浏览器宿主路径）

Playwright headed，viewport 640x480，加载同一页面：

- `start: ok  backend=gl  format=Rgba8UnormSrgb  physical=640x480  batches=4  draws=1  presented=1`
- 真实 `mouse.wheel` 后：`zoom: ok … presented=2`；无 `pageerror`。
- WebGL 归属（`WEBGL_debug_renderer_info`）：`ANGLE (Intel, Mesa Intel(R) UHD Graphics (CML GT2), OpenGL 4.6)`，
  GPU 进程持有 `/dev/dri/renderD129` + `/dev/nvidiactl` + `/dev/dri/renderD128` → **真实 GPU（Intel iGPU via ANGLE）**。

## 5. 关键约束（实测，不是推断）

1. **CSP 必须放行 wasm**：`script-src 'self'` 下 WebKitGTK 与 Chromium **都拒绝**实例化
   （`Refused to create a WebAssembly object because 'unsafe-eval' or 'wasm-unsafe-eval' is not an
   allowed source of script`）。`'wasm-unsafe-eval'` 是必需项；不允许退化成 `'unsafe-eval'`。
2. **两端都无可用 WebGPU**：WebKitGTK `navigator.gpu === false`；Chromium 该次探测为 `true` 但**未被使用**，
   方案按 WebGL2-only 设计。
3. **X11 抓屏**：`ffmpeg -f x11grab` 与 `gnome-screenshot` 的 X11 回退在本会话**全黑**；
   取证必须走 Mutter `ScreenCast`。
4. **WebKitGTK 渲染器字符串不可信**：`UNMASKED_RENDERER_WEBGL` 恒为 `"Apple GPU"`（指纹掩码）。

## 6. X11 宿主运行（`apps/app-linux`，`surfaceMode=dom-canvas`）

构建：`cargo build -p app-linux --locked` rc=0（debug）。运行
`GDK_BACKEND=x11 YACR_PROBE_NO_MAXIMIZE=1 ./target/debug/yacr --open fixtures/dwg/synthetic-four-lines.dwg`，
前端为 Vite dev（`http://localhost:1420/`）。

- 宿主标记行（stderr）：`[probe] surfaceMode=dom-canvas …`、`native CAD composition skipped …`、
  `GTK overlay drawing area, subsurface shim and wgpu presenter skipped …`、
  `native CAD evidence drivers not spawned …` —— 即 X11 上**没有**第二份 CAD 会话/原生表面。
- 诊断（宿主注入的 DOM diag）：`canvas` 已挂载，rect **`[300,175,626,341]`**，
  `canvasBacking` = **`[626,341]`**（说明 wasm `start()` 已 configure 画布）、`errors: []`。
- 像素（Mutter `ScreenCast` 单帧，裁剪到 canvas rect）：亮像素 **~980–1094**，竖线列
  **x = 188 / 387 / 585**（黑底，clear 色为黑），无蒙层；同一次运行内多次抓帧一致。
- **曾出现的真实缺陷（已修）**：首次运行失败于
  `canvas runtime failed step=ui-module-load detail=TypeError: Importing a module script failed.`。
  根因：Vite 8 dev 把动态 `import(/* @vite-ignore */ url)` 改写为
  `__vite__injectQuery(url,'import')`，对 public 产物请求 `/canvas/app_web.js?import` 返回 **500**；
  `@vite-ignore` 不阻止改写，CSP 不是原因（用同款 CSP meta 在 plain WebKitGTK 下 import+init 成功）。
  修法：桌面宿主改用 **classic script（wasm-bindgen `--target no-modules`）** + 显式
  `app_web_bg.wasm` URL，彻底不经过 ESM 动态导入；构建脚本从同一 wasm 同时产出两种 target。
  修复后失败消失、画布正常出图（上列数据）。

### 6.1 明确 NOT VERIFIED：X11 的 OS 级输入

- 本会话的**输入注入整体不可用**：`xdotool`（XTEST）与 `ydotool`（uinput，`BTN_LEFT`）的点击、
  滚轮、按键均未被任何应用接收（连 GNOME 顶栏点击都是 0 像素变化）。
- 页面侧计数器（宿主注入的 capture-phase 计数）显示 `pointermove/mousemove` 可到达，
  而 `pointerdown/mousedown/wheel/keydown` 恒为 0 —— **在未改动的 Wayland 原生路径上同样如此**，
  因此**不是** `dom-canvas` 引入的回归，而是取证工具限制。
- **2026-10-24 用户手动测试（用户报告，非 agent 抓帧）**：在 X11 宿主上滚轮缩放与拖拽平移
  **工作正常**。因此「X11 上可操作」成立，但**本条没有 agent 可复现的像素差证据**，也没有截图；
  不得据此写"已验收"，只能写"用户手动确认可用"。取证工具的限制仍在（后续可用输入注入的会话可补像素差）。
- 结论（修正后）：X11 宿主上的 pan/zoom **有用户手动确认**，**无 agent 级像素证据**。导航代码路径的证据来自
  §3（WebKitGTK 内**合成 DOM wheel**）与 §4（Chromium 的 **trusted wheel**，CDP 注入），
  二者都不是 X11 宿主上的真机输入证据。
- 需要补的证据：可用输入注入的会话（或人工操作）下重跑，记录 wheel/drag 前后的画布像素差。

## 7. 明确的 NOT VERIFIED / NOT RUN

- **WebKitGTK 这条路的硬件/软件归属：NOT VERIFIED。** `UNMASKED_RENDERER` 被掩码；本轮该进程未持有
  `/dev/dri/*` fd，`nvidia-smi pmon` 也未列出 webkit 进程。因此**不得**把本节结论写成"真实 GPU"。
- **app-linux 宿主的 X11 `dom-canvas` 运行**：已运行（见 §6）；但**OS 级输入未验证**（§6.1），
  所以 X11 上"能画"成立、"能操作"**不成立**。X11 的其它后端/其它会话未测。
- **未见到的**：真 X11 服务器/真 X11 WM、其它 WebKitGTK 版本、其它合成器、分数缩放、多显示器、
  帧率/延迟/内存任何性能结论、vendor DWG、精度、视觉 golden、Windows/macOS/Android。
- 无 DOM 挂载测试、无 Playwright 驱动的仓库内测试（本轮 Playwright 仅用于一次性人工复核）。
- 宿主 DOM diag 的 `canvas/wasmBinding/counts` 字段是**本轮新增的诊断**（`apps/app-linux/src/host/dom.rs`），
  它们是挂载/输入到达证据，**不是**像素或视觉验收。

## 8. 真实 DWG 进 wasm（`open_bytes`，阶段 ⑧ 第一片）

驱动：Playwright（Chromium 141，headed，900x700）加载 `apps/app-web/dist` 的浏览器 bundle，
用 **File API**（`<input type=file>`）把真实 DWG 交给 `open_bytes`，不经任何 native 解析。

| 观测项 | 合成 fixture | 真实 DWG `fixtures/complex-test.dwg` |
|---|---|---|
| 字节数 | 20,755 | **4,228,788** |
| label | `synthetic-four-lines.dwg` | `complex-test.dwg` |
| 导入 + 建场景 + 上传 | 63 ms | **14,118 ms** |
| batches / draws | 4 / 1 | **145,947 / 7,707** |
| **wasm heap** | 2,621,440 B | **1,909,850,112 B（≈1.91 GB）** |
| JS `usedJSHeapSize` | 10.0 MB | 1,917,633,246 B |
| 画布像素 | 4 条竖线 | 非黑像素 **18,364**、3,030 种颜色、bbox `8,12–882,693` |
| page error | 0 | **0** |

**能证明**：真实 DWG 的完整管线（acadrust 导入 → 显示表示 → 上传 → WebGL2 渲染 → canvas）
**在 wasm 内成立**，画布出非平凡几何。**不能证明**：性能/内存预算达标（见下）、vendor 兼容、
精度、视觉 golden、其它浏览器。

**暴露的硬约束（阶段 ⑧ 的风险）**：一个 4.2 MB 的 DWG 就让 **wasm 堆到 ~1.91 GB**（32-bit wasm
上限 4 GB），且单次导入 ~14 s。这意味着：
- 大图在 wasm 内**必然撞上限**，不能承诺"浏览器打开合格样本范围内的 DWG"而不做预算工程；
- 可行方向（需用户决策，**不得**自行发明 batch 线协议）：导入后**释放中间表示**、场景分批/流式上传、
  解析与渲染解耦，或对超大图**显式拒绝**并给出诊断（`Unsupported`/预算诊断，不得空白/静默降级）。
- 这些数字是**单次测量**，不是基准；也不代表其它机器/浏览器/构建配置。

## 9. X11 宿主终态与修复记录（2026-10-24 追加）

### 9.1 修掉的三个真实缺陷（均由真机运行暴露，非推测）

1. **Vite dev 改写动态 import**：`import(/* @vite-ignore */ url)` → `__vite__injectQuery(url,'import')`
   → public 产物带 `?import` 返回 **500** → WebKit 只报
   `Importing a module script failed.`。**修法**：改由 public 下的 `bootstrap.js`（ESM）加载，
   运行时不再有被改写的动态 import。
2. **WebKitGTK `document.currentScript === undefined`（不是 `null`）**：`--target no-modules` 的
   glue 顶层 `document.currentScript !== null` 判空失效 → `.src` 抛 `TypeError` → 顶层 `let
   wasm_bindgen` 停在 TDZ → 后续 `typeof` 抛 `ReferenceError: Cannot access 'wasm_bindgen' before
   initialization`。**修法**：放弃 no-modules，改用标准 ESM target。
3. **Vite dev 拒绝把 `public/` 文件当模块加载**（截图证据：错误覆盖层原文
   `This file is in /public … should not be imported from source code`）。**修法**：新增**仅 dev 生效**
   的 Vite 插件（`apply: 'serve'`、`enforce: 'pre'`），自己按模块语义返回 `/canvas/<name>.js`
   的磁盘字节（含 realpath 包含性校验与拒绝 `..`/嵌套/`.wasm`），生产构建不受影响
   （`dist/canvas/*` 仍原样复制、运行时加载）。

三个缺陷都被**显式拒绝**接住过（`yacr: unsupported-runtime: webgl2` + 退出码 6），没有出现空白窗口。

### 9.2 终态帧（observer 独立判读，真实 X11/XWayland 会话）

- 无任何错误覆盖层/横幅/红框（全图饱和红像素 **0**）。
- CAD 分区：canvas `926×341`（backing 同尺寸）、纯黑（采样非线条像素最大值 **0**，无蒙层/渐变/带状）；
  **4 条等距竖线** x=189/388/587/786（间距 199），中性 `R=G=B`，外侧两条因亚像素相位峰值为 202、
  内侧 239（**不是**调暗层）；无水平线、无网格/UCS/光标残留。
- 图纸标识：图纸标签页的**活动标签**显示 `synthetic-four-lines.dwg`（带关闭 `×`）；标题栏仍只有 `Yacr`。
- 启动面板/空态内容不可见；状态栏与命令行无重叠、无裁切。
- observer 终判：**yes —— 该帧读作“图纸已打开并显示”**。

边界不变：X11=XWayland + GNOME/Mutter + `scale=1`；**硬件归属 NOT VERIFIED**；pan/zoom 为**用户手动
确认**（无 agent 像素差证据）；真 X11 服务器/WM、其它合成器、DPR/多显示器、性能、vendor/golden 均未验。
