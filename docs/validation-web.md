# Web 宿主验证（wasm 构建 + 浏览器冒烟 + i18n 宿主同步）

## 2026-10-03 concept 重设计最终抽查

`DIST=/tmp/opencode/yacr-concept-final-web WITH_FONTS=0 bash scripts/build-web.sh` 成功；
Chromium 153.0.8010.12 / 无头 SwiftShader WebGL2 执行 `scripts/check-web-ui.mjs` 通过。
截图 `integration.png` 人工抽查确认浅银 Ribbon/侧栏/布局栏及完整合成图边界；导航像素变化
0.417%，双语切换/偏好恢复/349 字节空批注 JSON 往返通过，无 console/page 错误。
首轮截图发现整壳 fit 裁切，修正为启动使用 CAD 实际区域后重新构建并测试，未复用旧证据。
最终 wasm 15,985,060 bytes；SHA-256
`2d60a7d5aa6e99af205d1e3ee063391f660debc49d58457435818e86c7f83dee`。
这是桌面合成集成抽查，非真实 DWG、第三方字体、移动浏览器或真实 GPU 兼容验收。
本轮主验证为原生 Slint/wgpu/lavapipe，详见 `docs/ui-redesign.md`。

本文记录 web 工作流在 **本机 LXC（无 3D GPU，SwiftShader）** 上真实执行的构建与浏览器
验证。它与 `docs/validation.md`（总表）分开，后者由协调者汇总。命令、产物、sha256、
浏览器版本与 NOT RUN 项都在此，不得以“代码已实现”代替运行证据。

## 1. 构建（静态 bundle）

```bash
export PATH="$HOME/.cargo/bin:$PATH"
export CARGO_TARGET_DIR=/home/zenglanmu/sources/yacr/target
export CARGO_BUILD_JOBS=2
PROFILE=release bash scripts/build-web.sh
```

`scripts/build-web.sh` 依次执行：

1. `cargo build -p app-web --target wasm32-unknown-unknown --profile release --locked`；
2. `wasm-bindgen --target web --no-typescript --out-name yacr --out-dir web-dist/pkg`；
3. 复制 `web/index.html`、`main.js`、`style.css`；
4. 复制 `crates/cad-ui-slint/i18n/{zh-CN,en}.json` 到 `web-dist/i18n/`（JS 与 Rust 共用
   同一份 catalog，见 `docs/i18n.md` §6）。

结果（2026-10-02）：

| 项 | 值 |
|---|---|
| 输出目录 | `web-dist/`（`index.html` `main.js` `style.css` `pkg/yacr.js` `pkg/yacr_bg.wasm` `i18n/*.json`） |
| `pkg/yacr_bg.wasm` 大小 | **14,792,865 bytes** |
| `pkg/yacr_bg.wasm` sha256 | `e742d1532d86528333ed92ef8410a915fb1ec05527fa8689c4e8cdd42f5ba6fd`（同一源码重跑两次一致） |
| `pkg/yacr.js` | 有；导出 `start_web`、`open_document_bytes`、`web_set_locale` 等 |
| profile | release（无 `wasm-opt` 步骤，故体积偏大） |

`index.html` 以 `<script type="module" src="main.js">` 加载 bundle；`main.js` 动态
`import("./pkg/yacr.js")` 并 `await init()`，因此模块加载失败可被捕获并本地化报错。

## 2. 静态服务

```bash
python3 scripts/serve-web.py --directory /tmp/opencode/worktrees/web-runtime/web-dist --port 8090
```

`serve-web.py` 显式注册 MIME（已用 `curl -I` 核对）：

```
/pkg/yacr_bg.wasm  -> Content-type: application/wasm
/pkg/yacr.js       -> Content-type: application/javascript
/i18n/en.json      -> Content-type: application/json
/                  -> Content-type: text/html
```

## 3. 浏览器冒烟（真实运行）

浏览器是用户本地预装的 Playwright Chromium（不是仓库依赖）：

- Playwright：`playwright-core 1.63.0`（`~/.local/share/headless-browser/node_modules`）。
- 浏览器：`Google Chrome for Testing 153.0.8010.12`，revision `chromium-1243`。
- `npm --prefix /tmp/opencode/webtest install playwright-core` → `added 1 package in 3s`。
- `node <playwright-core>/cli.js install chromium`（`PLAYWRIGHT_BROWSERS_PATH` 指向
  headless-browser 的 `ms-playwright`）→ 退出 0，检测到已安装的 Chromium，无需重新下载。
- 启动参数：`--no-sandbox --enable-unsafe-swiftshader --use-gl=angle --use-angle=swiftshader`
  （软件渲染；无真实 GPU）。

```bash
export PLAYWRIGHT_BROWSERS_PATH=$HOME/.local/share/headless-browser/ms-playwright
export LD_LIBRARY_PATH=$HOME/.local/share/headless-browser/lib
export PLAYWRIGHT_MODULE=$HOME/.local/share/headless-browser/node_modules/playwright-core/index.js
node scripts/check-web-ui.mjs http://127.0.0.1:8090/ /tmp/opencode/webtest/yacr-web.png
```

结果：`web UI check passed`（两次连续运行均通过）。关键断言与观测值：

| 断言 | 结果 |
|---|---|
| 启动失败与 winit handoff 区分 | 无 `window.yacrStartupError`；`window.yacrHandoff` 为真 |
| 渲染后端 | `chosen=WebGl2 adapter=Some(WebGl2) caps=Some((WebGl2, false, 8192)) error=None` |
| CAD 区域非空 | 裁剪 `x[0.32,0.68] y[0.10,0.42]`：`distinctColors=2`、`stddev=9.69`、`max-min=205`（细线图，两种颜色是正常的） |
| 导航后 CAD 确实变化 | 滚轮缩放后 `changedFraction=0.0029`（静态帧为 0） |
| 语言切换不重置文档 | 切换后 `entities=6` 不变、页面标记 `window.__yacrMarker==="keep"`、未重载 |
| HTML 同步 | `document.documentElement.lang` `zh-CN → en`；`document.title` `yacr — CAD 查看与批注 → yacr — CAD Viewer & Annotation` |
| 持久化恢复 | reload 后 `locale="en"`、`lang="en"`、`localStorage["yacr.cad.locale"]="en"` |
| 字体编排 | `window.yacr.load_fonts()` 返回 `ok`，`catalog=0 requested=0 planned=0 registered=0 failed=0`（演示图未引用 CAD 文本字体） |
| console / page 错误 | `consoleErrors=0`、`pageErrors=0` |

截图与报告：

- `/tmp/opencode/webtest/yacr-web.png`（启动后整页）
- `/tmp/opencode/webtest/yacr-web-navigation.png`（导航后）
- `/tmp/opencode/webtest/yacr-web.json`（结构化报告，含上面的字段）

console 仅有一条预期告警：`No available adapters.`（`Auto` 探测 WebGPU 适配器失败后
回退 WebGL2）。

### 3.1 修复的问题（否则浏览器路径不可运行）

1. **CAD 桥固定 WebGPU**：`app-web` 先按偏好选后端，却调用固定的
   `install_cad_bridge`（内部 `BackendPreference::WebGpu`），在无 WebGPU 的浏览器上
   `adapter=None`。改为 `install_with_preference(.., chosen)`，报告与渲染器使用同一后端。
2. **Slint winit 后端未编译 wgpu 渲染器**：`cad-ui-slint` 的非 Android `slint` 依赖
   只有 OpenGL femtovg，`require_wgpu_30` 被无声回退，`GraphicsAPI::WebGL` 无法提供
   wgpu 设备。补 `renderer-femtovg-wgpu` feature；`bridge.rs` 增加一条“非 WGPU30 的
   RenderingSetup”显式 console error，避免再次静默空白。
3. **wasm 轮询误判设备丢失**：`cad-render-wgpu::device_scope_outcome` 把
   `PollType::Wait` 超时一律当作 `DeviceLost`。浏览器主线程不能阻塞，WebGL2 后端
   会返回 `Timeout`，导致健康设备被判丢失。wasm 改为非阻塞 `PollType::Poll`，且
   不把超时当丢失。
4. **WebGL2 画布 MSAA 呈现失败**：`wgpu-hal` 的 GL present 在非 sRGB surface 上
   用 `glBlitFramebuffer`；默认 `antialias:true` 的画布是多重采样默认帧缓冲，该 blit
   在 WebGL2 非法（`INVALID_OPERATION`），画布保持空白。`main.js` 在 Slint 创建上下文
   前强制 `antialias:false`（CAD 帧是纹理合成，只损失画布级 MSAA）。
5. **wasm 时钟 panic（2026-10-02 回归修复）**：perf 提交
   （`feat(perf): wire real memory/timing measurement`）在
   `cad-render-wgpu::Renderer::upload` 与 `cad-import-acadrust` 引入了
   `std::time::Instant::now()`。该函数在 `wasm32-unknown-unknown` 上直接 panic
   （`time not implemented on this platform`）：首帧 `upload` 在 Slint
   `BeforeRendering` 通知内 panic，通知持有的 `BridgeState` 可变借用未被释放，
   随后 `main.js` 的 `renderer_state_report()` 轮询也以
   `already mutably borrowed` panic，页面停留在“正在初始化渲染器”。改用
   `web-time`（wasm 走 `Performance.now()`，native 仍为 `std::time::Instant`）
   后恢复。`bf031c2` 之后未再跑浏览器验证，故该回归此前未被发现；`web-smoke`
   仍是 capability-gated（`vars.WEB_SMOKE_ENABLED`），默认 SKIPPED。

### 3.2 i18n 宿主同步

- `index.html` 增加“简体中文 / English”选择器（`#language`）；选择器位于 HTML 宿主
  chrome（右下角），不是 Slint 设置抽屉内的控件。
- `main.js` 从 `web-dist/i18n/<tag>.json`（构建复制自唯一真相 `crates/cad-ui-slint/i18n`）
  本地化启动/文件/导出/导入/恢复/错误提示、`lang`、`title`、`noscript`；缺 key 显示
  `⟦key⟧`。
- 切换调用 wasm 导出 `web_set_locale(tag)` → `UiHandle::set_locale`（只重建 chrome，
  不动文档/相机/批注/撤销栈），并写 `localStorage["yacr.cad.locale"]`；启动时
  `cad_ui_slint::web::stored_locale()` 恢复。
- 未完成项（见 `docs/i18n.md` §6.2）：Slint 抽屉内选择器、Rust 宿主动态状态/诊断
  文案、数值/单位本地化、`check-i18n.py` 未扫描 web 宿主字面量。

### 3.3 复验（wasm 时钟修复后，2026-10-02）

用 `WITH_FONTS=0` 的临时 bundle（`/tmp/opencode/web-dist-fixed`，wasm
15,182,044 bytes）复跑 `scripts/check-web-ui.mjs`，退出 0（`web UI check passed`）：

| 断言 | 复验观测值 |
|---|---|
| 渲染后端 | `chosen=WebGl2 adapter=Some(WebGl2) caps=Some((WebGl2, false, 8192)) error=None entities=6` |
| 启动状态文案 | `渲染器就绪：WebGl2`（修复前停留在“正在初始化渲染器…”/“宿主尚未就绪”） |
| 导航后变化 | `changedFraction=0.00328`（静态帧为 0） |
| 语言切换 / 持久化 / 字体编排 | 均通过（与 §3 一致） |
| console / page 错误 | `consoleErrors=0`、`pageErrors=0`；仅 1–2 条预期 `No available adapters.` 告警 |

wasm 中仍存在 winit 及其 `std::sync` 通道单态化出来的 `std::time::Instant::now`
引用（`wasm-objdump -x` 可见），属于上游在 wasm 上未走 `web-time` 的**潜在**路径；
本轮所有已执行的启动/导航/切换/重载流程均未触发，记录为上游残余风险（不可用
Cargo patch 修改）。

## 4. 必跑检查（全部通过）

```bash
cargo test --workspace --exclude cad-ui-slint --exclude app-android --exclude app-web --locked   # ok
cargo fmt --all --check                                                                          # ok
cargo clippy --workspace --exclude cad-ui-slint --exclude app-android --exclude app-web --all-targets --locked  # ok
python3 scripts/check-architecture.py                                                            # ok: 23 packages
cargo check --workspace --lib --target wasm32-unknown-unknown --locked                           # ok
python3 scripts/check-i18n.py                                                                    # ok: 103 keys, 2 catalogs
```

## 5. NOT RUN（明确未验证，勿当作通过）

- **WebGPU 路径**：本机 Chromium 即使加 `--enable-unsafe-webgpu` 也没有
  `navigator.gpu`（探测为 `gpu:false`），只验证了 WebGL2 基础路径。
- **真实 GPU / 真机**：全部为 SwiftShader 软件渲染，未在真实 GPU、Android 或桌面
  GL 上核对 `Depth32Float`、纹理桥、sRGB/预乘、resize 时序。
- **Slint 渲染文案语言**：只断言了 `lang`/`title`/`current_locale` 与 Slint 后端的
  结构化状态，未对 Slint 画布内文案做 OCR 级核对。
- **CAD 文本字体 CDN 取字节**：演示图未引用 CAD 文本字体，`load_fonts()` 走的是
  “未引用”分支；真实 `fetch` jsDelivr 与跨域注册仍未运行（见
  `docs/font-host-loading.md`）。
- **wasm 体积优化**：无 `wasm-opt`，14.8 MB 为未优化产物。

## 6. 宿主/渲染桥拆分复验（2026-10-02）

职责划分见 `docs/code-structure.md`。Rust 宿主入口从 954 行降为约 245 行，JS
入口从 405 行降为 70 行，渲染桥从 1269 行降为 754 行；功能实现移动到具名模块，
不是改成占位或删除测试。渲染桥原有 13 项测试全部保留，另加场景 stamp/层显隐契约。

### 构建与检查

- 核心测试：**870 passed / 0 failed / 1 ignored**。
- 新增 JS 宿主模块契约：**5 passed / 0 failed**，使用 Node `22.22.1` 内置测试。
- `cargo check --workspace --lib --target wasm32-unknown-unknown --locked`：通过。
- `cargo check -p cad-ui-slint --tests --target wasm32-unknown-unknown --locked`：通过，
  只编译测试，**不表示执行测试**。
- 核心 clippy `-D warnings`、`app-web --no-deps` wasm clippy `-D warnings`：通过。
- fmt、架构（23 packages）、i18n（116 keys）、fixtures（9）、workflow 与 shell
  语法检查：通过。CI 增加非门控 `web-host-contracts`，不把它当作 GPU 验收。
- 初次 `scripts/build-web.sh` 在字体下载阶段耗时过长，终止该次抓取；两个缺失文件
  `simhei.woff` / `simsun.woff` 复用现有 Android 缓存（目录相同，23 个共同文件的
  SHA-256 全一致）。随后 `scripts/fetch-fonts.sh web-dist/fonts` 缓存复验通过，
  **99 个字体文件完整**。未更改字体源、字体内容或第三方许可要求。
- 最终源码另执行 release `cargo build -p app-web --target wasm32-unknown-unknown
  --profile release --locked`（`YACR_FONT_BASE_URL=fonts/`）与 `wasm-bindgen --target web`：
  均通过。发布 bundle 的字体同源，包含四个 `host/*.js` 模块。

最终 `pkg/yacr_bg.wasm`：**15,182,933 bytes**，SHA-256：
`ea5aca02ba8d09da811164fa85b945c56f90f226a2098ede4bb2e998a28b66a7`。

### Playwright（两次本地通过）

最终源码产物通过 `scripts/check-web-ui.mjs http://127.0.0.1:8096/`，Chromium
`153.0.8010.12`，WebGL2 + SwiftShader：

| 项目 | 结果 |
|---|---|
| 四个 JS 子模块加载 | 全部 HTTP 200 |
| 实际后端 / 错误 | `adapter=Some(WebGl2)` / `error=None` |
| 非空 CAD 画面 | 3 色，stddev 11.848 |
| 滚轮导航像素变化 | `changedFraction=0.003280573593073593` |
| 双语、文档保留、重载恢复 | 通过，`entities=6` 不变 |
| 字体编排 | 通过；演示图仍走未引用字体分支 |
| 批注导出/回导 | 实际下载 349-byte sidecar，经 File API 回导 0 条批注 |
| 轮询不覆盖导入消息 | 等待 2.2 秒后消息保持不变 |
| console / page errors | 0 / 0（仅预期 WebGPU 无适配器告警） |

证据：`/tmp/opencode/yacr-refactor-final-web.json`、
`/tmp/opencode/yacr-refactor-final-web.png`、
`/tmp/opencode/yacr-refactor-final-web-navigation.png`；核心/构建/检查日志为
`/tmp/opencode/yacr-refactor-*.log`。回导为空演示 sidecar，不能宣称验证了非空批注
完整业务或真实 DWG 兼容性。

### 环境阻塞与未验证项

- Linux 原生 `cad-ui-slint` 测试因缺 `pkg-config`/fontconfig 构建依赖失败；尝试
  `RUST_FONTCONFIG_DLOPEN=1` 后上游 `fontique` 符号导入不兼容，未绕过或修改上游。
  因此迁移后的 Rust 桥契约仅完成 wasm 编译检查，原生运行未完成。
- 额外尝试 UI wasm 严格 clippy 时，`adapter.rs` 的既有 `clone_on_copy` 与
  `responsive.rs` 的既有文档 lint 共 32 项阻塞；没有以关掉警告宣称该检查通过。
  本轮 app-web 和核心严格 clippy 已通过。
- 本轮未验证 Android 运行、WebGPU、真实 GPU、真实字体绘制或真机性能；先前章节的
  NOT RUN 限制继续有效。

## 7. UI 宿主连接器接线（源码接线；无头验收待补）

`apps/app-web` 由 `browser/state_push.rs::push_panel_state` 统一推送面板状态（命令、
打开、批注导入/导出/恢复后），并安装 `browser/pick.rs::WebCanvasPickMapper` 与选择拾取；
诊断抽屉经新增 wasm 导出 `diagnostics_report_json()` 与 `window.yacr.diagnostics_report`
暴露。空/多值文案来自目录（新增 `annotation.empty`）。

同一条漏斗现在也是 **overlay（选择高亮 + 工具预览）的唯一入口**：每次推送调用
`CadView::set_selection_highlight(controller.selection().clone())`、
`set_measurement_preview(controller.measurement_preview())`、
`set_annotation_preview(controller.annotation_preview())`。空选择集 → 显式空
`SelectionSet`（高亮消失），无活动工具 → `None`（预览消失），因此命令、拾取与
确认/取消都经同一路径清理，不会残留旧 overlay。此外在捕获工具活动且指针移动
（非导航拖动）时，`browser/input.rs` 用 `pick::map_canvas_point` 追出世界点，经
`SessionState::set_measurement_cursor`/`set_annotation_cursor` 写入并复用漏斗推送；
空闲/导航状态不触发该路径。

以上为源码接线与单元测试；**本机未重跑 wasm 构建、未跑浏览器脚本**。注意
`browser` 模块仅在 `target_arch = "wasm32"` 编译，故新增纯测试
（`derive_overlay_push` 的空/非空与预览、`preview_cursor_for` 的分类）**只能编译**，
执行需要 wasm 测试运行器或浏览器；本机缺原生 `cad-ui-slint` 构建依赖，无法原生执行。
后续无头验证至少应断言：`window.yacr.diagnostics_report()` 在未导入时返回空模型 JSON、
导入后出现 `objects`/`document` 行；点击画布后 `renderer_state_report()` 的选择面板计数
变化；启动测量工具后移动指针，`overlay_revision` 随预览变化递增而底图不重解析；
`annotation.empty` 在两种语言下都解析成功。

## 8. 异步可取消打开宿主接线（F01，源码接线；浏览器验收待补）

### 线程能力判定（诚实结论）

`cad-app::tasks::ImportManager` 用 `std::thread`；`wasm32-unknown-unknown` 无线程，
`thread::Builder::spawn` 失败会使 `ImportManager::start` 的 `.expect(...)` panic。
因此 **`HostController::begin_async_open` 在浏览器中不可调用**，`apps/app-web` 不调用
它。`browser/async_open.rs::worker_available()` 是唯一能力门，浏览器返回 `false`。

### 已接线（源码）

- 新 wasm 导出：`async_open_poll_json()`、`async_open_worker_available()`（既有导出未
  改名/signature 未变）。
- `open_document_bytes*`/`OPEN` → `start_or_apply`：有线程宿主走 worker 并至多发布一次
  （manager `TaskStamp` 门）经 `install_opened` 安装；浏览器走同步导入，把**真实**
  终态推入同一 `set_import_state` 面板（无伪造进度）。
- 取消：`cancel-open-requested` → `CommandId::CancelLoading` → `cancel_async_open`；
  单一漏斗 `push_panel_state` 每次重推面板，取消保留当前文档/未保存批注。
- JS：`web/host/renderer.js` 心跳调用 `async_open_poll_json`，running 时收紧到 250ms；
  `web/host/async-open.js` 纯解析；`window.yacrAsyncOpen` 暴露真实状态。

### 已执行 / 编译门

| 项 | 结果 |
|---|---|
| `cargo check -p app-web --lib --target wasm32-unknown-unknown --locked` | 通过 |
| `cargo check -p app-web --tests --target wasm32-unknown-unknown --locked` | 通过（含 `async_open` 纯 JSON 编码测试，**只编译不执行**） |
| `node --test scripts/test-web-host.mjs` | 通过，新增 3 项（解析防御性、导出调用、心跳收紧），共 12 项 |
| `cargo fmt --all` | 通过 |

`browser::async_open` 仅在 `target_arch = "wasm32"` 编译，且本机缺原生
`cad-ui-slint` 构建依赖，故其 Rust 单元测试本机无法执行；JS 侧由 Node 真实执行。

### NOT RUN（明确未验证，勿当作通过）

- **浏览器/GPU 运行**：本轮未重跑 `scripts/build-web.sh`、未启动 Playwright、未截图。
  因此「面板在真实浏览器可见/可取消」未被运行证据确认，仅有源码接线与编译门。
- **running/cancellable 面板**：浏览器平台无法提供（无线程），未、也不能伪造。
- **Android 宿主接线**：本轮范围外。
- 端到端真实 DWG 异步导入（worker 路径）未在浏览器运行，因该路径在浏览器不存在。

## 9. 绘制/编辑合并后主控复验（2026-10-03）

基于 `39077ec` 的合并结果修复快照发布、绘制/导航输入冲突及绘制错误返回，见
`drawing-edit.md` §6。此前编译通过没有发现“命令成功、渲染仍看旧 Arc”的集成缺口。

### 实际执行

- 核心串行：**963 passed / 0 failed / 1 ignored**；日志
  `/tmp/opencode/yacr-draw-core.log`。JS 契约 **27 passed**（含 PNG 像素解码契约），
  `/tmp/opencode/yacr-draw-js.log`。新增 Slint Rust 快照测试仅跨平台编译，未原生执行。
- fmt、核心/app-web 严格 clippy、架构、i18n（159 keys）、fixtures（9）与 workflows
  通过；workspace wasm lib、UI/Web wasm tests、UI/Android aarch64 tests 编译通过。
- 最终 release 测试 bundle：`/tmp/opencode/yacr-draw-web-dist/`，`WITH_FONTS=0`，
  不含 CAD 字体；wasm SHA-256
  `80e1c7b284719b23eed704a4ac399404448305c30b945180a68451b7ec50a987`。
- Chromium **153.0.8010.12**、SwiftShader/**WebGL2**；四种绘制工具独立进程串行通过。
  LINE/CIRCLE 创建、MOVE 与 LINE×LINE TRIM 改变解码后的 CAD 区域像素，撤销恢复
  与基线完全相同的像素 SHA-256；确认后工具行消失，排除把预览误当正式绘制。
  LINE/CIRCLE 确认前/取消不新增实体，重做恢复实体计数；曲线目标 TRIM 拒绝后工具
  保持捕获、计数不变，随后取消恢复空闲。无 page errors。
- 既有 `check-web-overlay.mjs` 通过（高亮变化 0.0012890625，清空差异 0）；
  `check-web-ribbon.mjs` 桌面 1280×800、390×844 DPR3、320×740 DPR2 通过，含导入/双指/平移。
  `check-web-ui.mjs` 独立进程通过（导航变化 0.004329004329004329、双语/持久化/空 sidecar
  往返，console/page errors 为 0）。这些不是硬件 GPU 或真实 DWG 兼容性证据。

复跑绘制（静态服务端口 8103）：

```bash
DIST=/tmp/opencode/yacr-draw-web-dist WITH_FONTS=0 CARGO_BUILD_JOBS=2 bash scripts/build-web.sh
python3 scripts/serve-web.py --directory /tmp/opencode/yacr-draw-web-dist --port 8103
# 另一个终端，使用本机已安装的 Playwright Chromium：
export PLAYWRIGHT_BROWSERS_PATH=$HOME/.local/share/headless-browser/ms-playwright
export LD_LIBRARY_PATH=$HOME/.local/share/headless-browser/lib
export PLAYWRIGHT_MODULE=$HOME/.local/share/headless-browser/node_modules/playwright-core/index.js
for kind in LINE CIRCLE MOVE TRIM; do
  node scripts/check-web-drawing.mjs http://127.0.0.1:8103/ "/tmp/opencode/yacr-drawing-final/$kind" "$kind" || exit
done
```

证据：`/tmp/opencode/yacr-drawing-final/{LINE,CIRCLE,MOVE,TRIM}/report.json` 与各阶段 PNG；
`/tmp/opencode/yacr-draw-final-overlay/`、`yacr-draw-final-ribbon/`、`yacr-draw-final-ui.json`。
截图需解码并仅比较 CAD 区域；`fromSurface:false` 的 CDP clip 在本环境可能被忽略，
整张 PNG 的焦点/历史控件不同不能作为底图不同的判断。

### 超时与未验收

- 连续多截图曾使 SwiftShader/CDP 停滞；四工具分独立进程复跑通过。新脚本为 CDP 截图
  设置 20 秒失败上限，不将超时当作通过。
- `check-web-mobile.mjs` 顺序回归超时（240 秒整条回归链预算），单独重试也超时
  （90 秒）。**本轮 mobile 脚本未通过**，没有以历史证据覆盖；Ribbon 的窄屏布局/
  导航验证是不同脚本，不等于完整 mobile 回归通过。
- 未运行 Android APK、真机/WebGPU/真实 GPU；未部署或验证生产 URL。未验证编辑 DWG
  写回、真实字体、手机绘制多指取消、重做像素以及所有错误工具保留路径。

## 10. 输入安全修复与 Pages 发布候选（2026-10-03）

在 §9 的基础上修复第二触点/touchcancel 不取消 shell 捕获、成功换图纸残留旧取点及
TRIM 固定世界容差；见 `drawing-edit.md` §6.1。

- 核心重跑 **963 passed / 0 failed / 1 ignored**；JS **28 passed**，PNG 解码测试加入 CI。
  日志 `/tmp/opencode/yacr-pages-core.log`、`yacr-pages-js.log`。fmt、严格核心/app-web clippy、
  workspace wasm lib、UI/Web wasm tests、UI/Android aarch64 tests 编译及各静态检查通过。
- 候选 `/tmp/opencode/yacr-pages-release/`，release wasm **15,505,725 bytes**，SHA-256
  `dfaf4af1a51ea71e1a61e5066d27e055722edd46a7c0f9e9fc8b02e5a2c54d58`。
  同源 `fonts/` 复用已有缓存，catalog 引用的 **99 个字体全部存在且非空**；不改字体内容/
  源/许可声明。本轮 UI 演示仍不引用 CAD 文本，不能以文件存在宣称字体渲染验收。
- 最终四工具独立进程通过，`/tmp/opencode/yacr-pages-drawing/`。新增输入安全脚本通过，
  `/tmp/opencode/yacr-pages-safety/report.json`：第二触点未移动、touchcancel、成功打开新
  DWG 后的旧确认均零提交。TRIM 圆目标错误保留捕获也通过。
- 最终桌面 UI 通过，`/tmp/opencode/yacr-pages-ui.json`。mobile **phone/small-phone/desktop**
  独立进程通过，`/tmp/opencode/yacr-pages-mobile/<scenario>/report.json`，包含高 DPI、语言
  UI-only 不增 CAD 帧、真实合成 DWG 导入、旋转保留文档及启动失败显示。
  测试新增 bringToFront、场景过滤与进度日志，未删断言。可复跑：

  ```bash
  node scripts/check-web-mobile.mjs http://127.0.0.1:8104/ /tmp/opencode/mobile-small small-phone
  ```

- §9 的超时仍保留。最终候选的 **Ribbon 触控回归仍出现超时**，包括独立 phone 场景；
  排查未定位确定根因，不能把它归结为已解决的环境问题或宣称全量浏览器回归通过。
  四工具批量运行也曾在 TRIM 截图处超时，独立重跑通过；记录真实限制，不隐去失败。
- 未新增 Android APK、真机/WebGPU/真实 GPU 证据。发布目标为现有 Cloudflare Pages 项目
  `yacr-examples`、生产分支 `main`；生产部署与线上复验结果将在下节追加。

## 11. Cloudflare Pages 生产部署与线上复验（2026-10-03）

- 修复代码提交 **`01f9f2b1892edbf70cb2d5fe42e09900e2d02f8c`** 已推送 `origin/main`。
- `DIST=/tmp/opencode/yacr-pages-release SKIP_BUILD=1 CF_PAGES_PROJECT=yacr-examples
  CF_PAGES_BRANCH=main bash scripts/deploy-cloudflare-pages.sh` 成功。Wrangler **4.147.0**，
  117 个文件（5 新上传、112 已有），未改变现有 DNS/自定义域名配置。
- Cloudflare API 确认部署 **`200b4aee-087f-4354-9285-5cbc59ec5507`**、
  `environment=production`、`latest_stage.status=success`，提交元数据与上述代码一致。
  部署完成时间 `2026-10-03T03:50:06.869629Z`。
- 生产地址：<https://yacr-examples.pages.dev/>；现有自定义域名：
  <https://yacr-examples.snakeheartgo.top/>。不可变部署 URL：
  <https://200b4aee.yacr-examples.pages.dev/>。
- 两生产域名的 wasm 实际下载 SHA-256 均为 §10 的
  `dfaf4af1a51ea71e1a61e5066d27e055722edd46a7c0f9e9fc8b02e5a2c54d58`，
  **15,505,725 bytes**；HTTP 200 / `application/wasm`，字体 catalog 均含 99 文件。
  初次经代理下载出现 403/传输超时，直连 `curl --noproxy '*'` 完整下载并核验通过。

线上真实无头 WebGL2/SwiftShader（Pages 生产 URL，不路由替换本地 wasm）：

| 检查 | 结果 |
|---|---|
| `check-web-ui.mjs` | 通过；渲染、导航像素改变、双语/重载、空 sidecar 往返，console/page errors 0 |
| `check-web-drawing-safety.mjs` | 通过；第二触点、touchcancel、成功换图纸后旧确认均不写库 |
| LINE/CIRCLE | 通过；正式绘制改变 CAD 像素，撤销逐像素回到基线，重做恢复计数 |
| MOVE | 两次截图阶段超时；本地发布候选通过，线上**未通过** |
| TRIM | 撤销后截图阶段超时；本地发布候选通过，线上**未通过** |

证据：`/tmp/opencode/yacr-pages-deploy.log`、`yacr-pages-production-assets.json`、
`yacr-pages-production-ui.json`、`yacr-pages-production-safety/report.json`、
`yacr-pages-production-drawing/{LINE,CIRCLE}/report.json`。不把资产一致或本地成功替代线上
超时验收；Ribbon 触控根因仍未定位。真机/硬件 GPU/WebGPU、Android APK 未在本轮运行。
