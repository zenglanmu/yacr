# AGENTS.md

Rust CAD **查看/测量**系统，**开发中，无可交付产品**。需求唯一权威是
`CAD_IMPLEMENTATION_SPEC.md`（v2.0）；逐轮交接入口 `docs/handoff.md`；桌面 UI 决策见
`docs/adr/0010-react-linux-host-replaces-slint.md`，合成方向见 ADR 0023/0024，Android 载体见
ADR 0027。

> 当前形态：**一棵 React 树，多种载体**。`apps/desktop-ui` 是唯一 React UI 包，Tauri 薄壳在
> `cad-desktop-shell`。Slint 已完全删除。
>
> - **`native-below`（native 会话持有 `cad-db`）**：Linux 原生 Wayland（`apps/app-linux`，`yacr`）、
>   Windows（`yacr-windows`）、macOS（`yacr-macos`）、Android（`apps/app-android`）。透明 WebView
>   在上，原生 CAD surface 在下。
> - **`dom-canvas`（wasm 会话持有 `cad-db`）**：Linux X11/XWayland（同一 `yacr`，按后端切换）、
>   Web（`apps/app-web`）。CAD 画在 DOM `<canvas>`。
>
> **大量“已通过”只是合成测试或有界环境证据**（模拟器、软件 GPU、无头浏览器），不是产品验收。

## 铁律（违反即返工）

1. 不改 acadrust 源码，不用 Cargo patch（`check-architecture.py` 拒绝）。
2. 未支持/未验证能力必须显式建模（`Unsupported`/`Partial`/诊断/`pending(...)`）；占位与假数据
   不计完成，**禁止**把失败改成空成功。
3. 业务层只操作数据库/命令/事务；GPU 提交封装在渲染器内。`cad-db` 是唯一权威，UI 与渲染均为派生。
4. 增量更新：ChangeSet → 依赖失效 → 显示表示重建，不重解析底图。
5. 每项改动同步补契约测试与文档。
6. 证据诚实：未运行就写 NOT RUN；合成测试、模拟器、软件渲染、无头浏览器结论不得推广为真机/
   真实 GPU/vendor/golden/性能结论；历史 Slint 证据不得冒充 React 宿主结论。
7. 不覆盖/删除陌生变更（并发实现常见）；先读源码与最新构建证据。
8. **载体权威不可混淆**：`dom-canvas`（X11/XWayland、Web）只有 wasm 侧 CAD 会话持有 `cad-db`；
   `native-below`（Wayland、Windows、macOS、Android）只有 native 会话持有。另一侧**不得**实例化
   第二份文档状态，也不得把显示表示当数据源。能力缺失必须显式拒绝
   （`yacr: unsupported-runtime: webgl2` + 退出码 6；浏览器侧是命名拒绝），**禁止**空白 canvas
   或静默回退。
9. **跨载体的能力按所有宿主建模**（天正/vendor 插件与任何用户可见通道都适用）：先在共享层
   （`cad-ui-core`/`cad-desktop-shell`/前端 adapter）定义一份载体无关的契约与测试，再按
   `surfaceMode`/`documentOwner` 接线；**不得**把 Linux（或任一单一宿主）的实现当成通用实现。
   某一载体暂时不能支持时必须显式拒绝（稳定的 `unsupported-runtime: ...` / `vendor.*` 原因），
   不得静默成功。当前只在 Linux 环境做运行/契约验证；Windows/macOS/Android/Web 的结论只能写
   “仅编译 / 仅契约 / NOT VERIFIED”，不得写成已验收。

## 分层与边界（`scripts/check-architecture.py` 强制）

唯一依赖：`cad-render-wgpu` → wgpu；`cad-import-acadrust` → acadrust（0.6.8，未修改）。
架构规则：`cad-desktop-shell` 是唯一允许依赖 `tauri` 的共享层（脚本强制的是 slint/iced 全局禁用
与逐 crate 的 ban 列表，不正向断言 tauri 唯一性——不要因此绕过该规则）。平台类型
（`web-sys`/`android-activity`/`jni`/`ndk`）不得进入 `cad-*`；`Cargo.toml` 根 `[patch]` 被拒绝。

```
cad-domain → cad-db/dependencies/history → geometry/kernel-adapter/representation
→ spatial/scene/render-wgpu → proxy/import-acadrust/resources → measure/query/app
→ engine/platform/diagnostics/cli-tools → cad-host → cad-ui-core
→ cad-desktop-shell + apps/desktop-ui（React）→ apps/app-{linux,windows,macos,android,web}
```

- `cad-host` 不依赖任何 UI 工具包；平台 I/O 只存在于 `apps/app-*`（`cad-platform` 契约）。
- `cad-ui-core` 框架/GPU 无关（wire protocol、layout、`CadSession`、`ShellState`、popup、i18n）。
- `cad-app` 不依赖 `cad-engine`；`cad-engine` 不依赖 UI/宿主。
- `apps/desktop-ui` 是**唯一** React UI 包（非 Cargo 成员），Linux/Windows/macOS/Android 宿主共用；
  前三个的 `tauri.conf.json` 指向 `../desktop-ui/dist`，Android 指向 `../dist`。Web 用
  `scripts/build-web-ui.sh` 产出独立的 `apps/app-web/dist-ui`。
- `apps/app-web` 的 wasm 会话是真实实现；它的 **native host target** 是显式 NOT IMPLEMENTED
  空壳，不要为它补第二套 native 代码路径。

## 构建与门禁

工具链 **1.99.0**（`rust-toolchain.toml`），`Cargo.lock` 必须 `--locked`；`cargo` 不在 PATH 时
加 `$HOME/.cargo/bin`。

**日常只验证自己的改动范围**（改动 crate 的编译、自己的契约测试、自己文件的 fmt/clippy）。
全量门禁与全量测试只在用户明确要求或发版/发布验收前运行；未运行必须写 **NOT RUN**。

```bash
# 单 crate / 单测试（GPU/CLI 用例串行）
cargo test -p <crate> <test-name> --locked
cargo check -p app-linux --all-targets --locked
cargo test -p cad-render-wgpu --test wgsl_validation --locked   # 离线 WGSL，无需 GPU

# Web：host target 契约测试 + wasm-only 代码编译，两者不可互相替代
cargo test -p app-web --locked
cargo check -p app-web --target wasm32-unknown-unknown --locked

# 前端：npm test = tsc --noEmit + node --test test/*.test.ts
(cd apps/desktop-ui && npm ci && npm test && npm run build)
(cd apps/desktop-ui && node --test test/foo.test.ts)            # 单个前端测试

# 载体 bundle（各自产出，互不覆盖）
bash scripts/build-web-ui.sh        # -> apps/app-web/dist-ui
bash scripts/build-android-ui.sh    # -> apps/app-android/dist

# 全量测试（GPU/CLI 用例串行）
cargo test --workspace --locked -- --test-threads=1

# 全量门禁（CI core-quality + i18n-contracts 同款）
cargo fmt --all -- --check
cargo clippy --workspace --exclude app-android --exclude app-web --all-targets --locked -- -D warnings
python3 scripts/check-architecture.py
python3 scripts/check-tauri-capabilities.py    # Tauri v2 core-plugin ACL 能力文件
python3 scripts/check-fixture-manifest.py
python3 scripts/check-file-size.py                 # 代码文件 ≤1000 行
python3 scripts/check-command-docs.py && python3 scripts/test-check-command-docs.py
python3 scripts/check-i18n.py
cargo check --workspace --all-targets --locked
cargo check --workspace --lib --target wasm32-unknown-unknown --locked
```

CI 分层：`.github/workflows/core.yml`（core-quality/wasm-check/i18n-contracts/android-check）与
`.github/workflows/build.yml`（linux-app/windows-check/macos-check/shader-validation/web-build/
web-smoke/web-deploy + 5 个 event-gated release job）。`check-workflows.py` 及其 `scripts/test-*.py`
契约（linux/web-deploy/package/fetch-fonts/android-host/macos-host/ui-migration-guards）也是门禁的
一部分，清单以 workflow 与实际脚本为准。

**顺序陷阱**：`npm run build` 必须在 Rust 宿主编译之前，`tauri::generate_context!` 内嵌
`apps/desktop-ui/dist`；Android 的 `cargo check -p app-android --target aarch64-linux-android`
同理要先跑 `bash scripts/build-android-ui.sh`。门禁命令**不得**用 `| tail -1` 之类管道吞退出码
（用 `set -o pipefail` 或 `${PIPESTATUS[0]}`）。

## 读图与视觉证据（AI 会话必读）

- 看截图、渲染图、PDF、设计稿时**优先用模型自带的多模态能力**：直接 `read` 图片/PDF 文件观察。
  需要把大图或多图从主上下文隔离时交给 `@observer`。
- **不要**默认写 Python/Shell 把图像转 ASCII/ANSI、逐像素打印或降采样成文本再“猜图”。只有模型
  没有视觉能力、需要精确像素/颜色断言，或要生成 evidence 文件时才写脚本；脚本输出的数字**不能
  代替实际看图**。
- 看图前先确认渲染来源：真实 GPU / SwiftShader / lavapipe / Wine / 模拟器 / 无头浏览器。软件渲染
  与模拟器截图只能作布局/DOM 证据，不得写成 GPU、真机、像素或性能结论。

## 运行与证据

- Linux 唯一可运行 GUI：`bash scripts/run-react-linux.sh --open <dwg>`（不强制 `GDK_BACKEND`）。
  直跑先构建前端，再
  `cargo run --release --locked -p app-linux --bin yacr --features tauri/custom-protocol -- --open <dwg>`。
- 后端在 GTK 启动前由 `apps/app-linux/src/backend.rs` 决议，**无静默回退**：Wayland =
  `native-below`（生产路径）；X11/XWayland = `dom-canvas`。退出码 4 是历史 X11 拒绝，**不再产生**。
- Android 宿主：`bash scripts/build-android.sh --release --target aarch64`（需要 `ANDROID_HOME`、
  `NDK_HOME`、JDK 17）；UI bundle 由 `build-android-ui.sh` 生成。APK 成功只是打包 artifact。
- 一键证据：`bash scripts/collect-evidence.sh`（Wayland/X11 用同 UID Mutter ScreenCast 抓合成帧）。
  `--probe` 是假打开，**不得**当作视觉/像素证据；证据运行必须显式前台目标窗口并每轮用新目录。
- 桌面窗口默认**最大化但保留标题栏**（不是全屏/kiosk）；`YACR_PROBE_NO_MAXIMIZE` 仅用于证据运行。
- Web 本地验收（手工，未接线 CI）：`bash scripts/build-web-ui.sh` →
  `python3 scripts/serve-web.py --directory apps/app-web/dist-ui` →
  `node scripts/web-ui-smoke.cjs --url http://127.0.0.1:8090/ --verbose --evidence <dir>`
  （Chromium + SwiftShader ⇒ 软件渲染，只作布局/DOM 证据）。
- 真实 DWG：`YACR_TEST_DWG=<绝对路径>`，否则用例跳过——**跳过不是已运行证据**。外部大样本放仓库外。
- 有真实显示/GPU 时必须用真实 GPU。软件渲染（SwiftShader/lavapipe）产物必须显式标注，且不得
  用作性能结论。
- 退出码：`0` 成功、`1` 通用失败、`2` 后端策略错误、`3` postload-pan 驱动失败、`4` **历史** X11
  拒绝（不再产生）、`5` Windows 缺 WebView2、`6` dom-canvas 缺 WebGL2、`7` GPU 预算拒绝。
  分类以稳定标记行为第一依据。

## 前端约定（`apps/desktop-ui`）

- TypeScript/TSX，**只用可剥离语法**（禁 `enum`/`namespace`/参数属性），类型导入用 `import type`；
  相对导入带显式 `.ts`/`.tsx`。`test/*.ts` **不参与** `tsc`（有意，见 `test/helpers.ts` 头注释）。
- 纯逻辑按功能放 `src/{cad,commands,host,layout,layers,options,popup,properties,references,ribbon,tools,color}/`，
  区域组件留在 `src/components/`；不新增桶式导出。职责表见 `apps/desktop-ui/README.md`。
- 构建期 `__YACR_RUNTIME__` ∈ `tauri | web | android`（Vite `define` 注入并校验，默认 `tauri`）。
- `src/host/invoke.ts` 是**唯一** import `@tauri-apps/api` 的模块；`web-adapter.ts` 是薄转发层，
  **只许拒绝、不许伪造答案**（拒绝来自会话，如 `unsupported-runtime: <name>`）。
- 路由按宿主上报的 `cad_status.hostProfile.surfaceMode`，**不按构建目标**：`native-below` 的 CAD
  数据命令走 Tauri/native；`dom-canvas` 走 wasm adapter。Android 是 `native-below`，不要按
  `__YACR_RUNTIME__=android` 把 CAD 命令送到 wasm。
- 移动端不是第二套 UI：同一 React 树，只按宽度切换。单源是
  `src/layout/layout-mode.ts` 的 `MOBILE_MAX_WIDTH = 1024`（含端点）。
- 三份产物**不得互相覆盖**：桌面 `apps/desktop-ui/dist`、Web `apps/app-web/dist-ui`、
  Android `apps/app-android/dist`（分别由 `npm run build`、`scripts/build-web-ui.sh`、
  `scripts/build-android-ui.sh` 生成）。`scripts/build-web.sh` 写的 `apps/app-web/dist` 是另一份
  独立 harness 产物，也不要覆盖。

## i18n（文案唯一权威）

- 唯一目录 `crates/cad-ui-core/i18n/{zh-CN,en}.json`，键集必须一致；React TS/TSX 与 Rust UI
  不得硬编码中文（`scripts/check-i18n.py`）。
- **机械规则**：在 `crates/cad-app/src/selection.rs` 新增 `PropertyRow::new("<key>")` 时，必须同时补
  两个 i18n 文件的 `properties.row.<key>` **和**
  `apps/desktop-ui/src/properties/property-labels.ts` 的 `propertyRowLabel` 分支；否则
  `apps/desktop-ui/test/properties.test.ts` 失败。
- `crates/cad-app` 仍有**冻结的用户可见中文债务**（`crates/cad-app/src/tests/cjk_baseline.rs`）：
  只禁止增长，不是已合规；新代码不得沿用。

## 平台状态（勿过度承诺）

| 载体 / 平台 | 实现与证据边界 |
|---|---|
| Linux `app-linux`（`yacr`） | **唯一可运行 GUI**。原生 Wayland = `native-below` 生产路径；X11/XWayland = `dom-canvas`。只有 GNOME/Mutter + P620 + `scale=1` 的有界运行证据 |
| Windows `app-windows`（`yacr-windows`） | 真实 Tauri/WebView2 + Win32 child HWND，`native-below`；**仅**编译门禁 + Wine 拒绝合同（缺 WebView2 → 退出码 5）；视觉/运行时/真 Windows **NOT VERIFIED**（PENDING user real-Windows spike） |
| macOS `app-macos`（`yacr-macos`） | 真实 Tauri/WKWebView + CAD 子 `NSView`（Metal），`native-below`；**仅** CI 编译；窗口/Metal/层序/DPI/对话框/像素一律 **NOT VERIFIED** |
| Android `app-android` | 真实 Tauri WebView + 原生 `CadSession` + Vulkan `SurfaceView`，`native-below`，wasm 会话**不实例化**。SAF 打开 + DXF 导出保存已接线（DWG 保存显式延后），但设备/真实 GPU **NOT VERIFIED**；仅编译/契约级证据 |
| Web `app-web` | wasm `CadSession` + DOM canvas（`dom-canvas`）。OPEN 走页面上传，SAVE/PLOT 走浏览器下载（DXF/SVG/PDF；**不回写 DWG**）；CI `web-smoke` 是 SwiftShader 冒烟，非 GPU/像素；native host target 是显式 NOT IMPLEMENTED 空壳 |
| CLI `cad-cli-tools` | 保留，headless 开图/出图（PNG/SVG/PDF） |

平台打包脚本与 5 个 release job 自 2026-11-01 起按 React/Tauri 宿主重新启用：仅 `v*` tag / 手工
`workflow_dispatch` 运行，只上传 workflow artifact，**不签名、不公证、不接触商店凭据**。
`publish-release.yml` 是 GitHub Release 的唯一发布入口（手工把指定 run 的 5 个 artifact 附到
tag 的 Release）；`web-deploy` 也只在该事件下把 `apps/app-web/dist-ui` 发布到
`https://yacr-examples.pages.dev`。
打包或发布成功都不是运行、GPU、像素或真机验收，也不要写“某平台已验收”。

`README.md` 的平台段仍有历史残留（例如 Windows/Web 的旧状态），平台判断以本表、`docs/*-app.md`、
`docs/handoff.md` 与实际代码为准。

## 提交纪律（已发生两次）

后台 specialist lane 仍在写工作树时**禁止** `git add -A` / `git add .`。提交前：①
`git status --porcelain` 逐项核对都属于本轮改动；②按**显式路径**暂存；③门禁命令显式检查退出码。
已发生：`git add -A` 扫入他人半成品；`clippy | tail -1` 把失败吞成“通过”。

## 文档索引（按需读）

`docs/architecture.md`（边界/不变量）、`docs/core-invariants.md`、`docs/handoff.md`（逐轮交接，
最新事实优先）、`docs/compatibility.md`（能力表）、`docs/testing-dwg.md`、`docs/react-linux-app.md`、
`docs/android-host.md`、`docs/windows-app.md`、`docs/macos-app.md`、`docs/xref.md`、`docs/ci.md`
（部分历史条目已过期，以 workflow 与实际代码为准）、`docs/adr/`（0010、0017–0027）。
`docs/linux-app.md`、`docs/verify-ui.md` 与旧 Slint 验证记录是历史材料，不得作为当前证据。
