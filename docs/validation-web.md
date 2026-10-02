# Web 宿主验证（wasm 构建 + 浏览器冒烟 + i18n 宿主同步）

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
