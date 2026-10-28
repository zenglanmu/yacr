# CI（GitHub Actions）分层工作流

本文件描述仓库**当前**实际提供的 CI 运行层。命令权威仍是 `AGENTS.md`「构建与门禁」与
`docs/build.md`；本文件只说明 workflow 如何镜像该门禁，以及哪些层次**已退役/未运行**。

工作流文件：`.github/workflows/core.yml`（核心质量层）与 `.github/workflows/build.yml`
（构建/校验层）。校验脚本：`scripts/check-workflows.py`（必需 job 片段 + 退役 job 硬禁用契约 +
浏览器 build/smoke/deploy 入口契约）、`scripts/test-linux-workflow.py`（变异契约）、
`scripts/test-web-deploy-workflow.py`（浏览器流水线变异契约）、
`scripts/test-ui-migration-guards.py`（迁移守卫契约）、
`scripts/check-command-docs.py`（§2/§2b/§3.7 的命令名与别名一致性，权威是
`crates/cad-ui-core/src/session/commands.rs`）与它的自测
`scripts/test-check-command-docs.py`（注入伪造名字/别名，断言检查器必然非零退出）。

> **2026-10-16 迁移状态（2026-10-23 增补）**：桌面已迁移到 React/Tauri（`apps/app-linux`、`yacr`），
> Slint 退役。编译门禁已加入 Tauri/GTK 前置（`libwebkit2gtk-4.1-dev`）与 React 前端构建
> （`apps/desktop-ui`、`npm test`、`npm run build`）。**2026-10-23 起**：`windows-check` 编译的是
> **真实 Windows 宿主**（不再是 NOT IMPLEMENTED 桩），但**仍只是编译门禁**；X11/XWayland 在
> Linux 宿主中被显式拒绝（退出码 4），CI 不运行 GUI。5 个发布 job 以 `if: false`
> **硬禁用**并以 `RETIRED` 标记，**不可通过仓库变量重新启用**，也不含 `secrets.…` 发布路径。
> 见 ADR 0010 与 ADR 0023。

> **2026-10-25 增补（Web 流水线恢复）**：`apps/app-web` 的 wasm CAD 会话（ADR 0024）已在 CI 中
> 端到端接线：`web-build` 用**钉住 SHA-256 的 wasm-bindgen 预编译 CLI** 构建
> `apps/app-web/dist` 并上传 artifact；`web-smoke` 下载该 artifact、起静态服务器、用 Playwright
> Chromium 打开页面并断言 typed `start: ok` 状态行；`web-deploy` 只在 **`v*` tag 或手工
> `workflow_dispatch`** 时把**同一个** artifact 发布到 Cloudflare Pages（`yacr-examples`）。
> 常规 push/PR **不会**发布。CI runner 无 GPU：`web-smoke` 走 **SwiftShader 软件渲染**，
> 因此它是 carrier/status 冒烟，**不是** GPU/像素/性能证据。CI 也未覆盖 WebKitGTK/其它浏览器
> 矩阵（本地证据见 `docs/validation-dom-canvas-wasm-2026-10-24.md`）。

> **本环境（无 GitHub runner、无 `gh` 权限）没有实际执行这些 job**；本文件描述的是 workflow 内容
> 与预期，**不是通过证据**。真实 job URL/commit/结果应写入 `docs/validation*.md`。

## 触发、权限与并发

- 触发：`push`、`pull_request`、`workflow_dispatch`。
- 权限：顶层 `permissions: contents: read`。只有 `web-deploy` 使用
  `CLOUDFLARE_API_TOKEN` / `CLOUDFLARE_ACCOUNT_ID`（仓库 secrets），且仅在 `v*` tag 或手工
  dispatch 时运行；其余 job 不接触发布凭据。
- 并发：`concurrency.group = <workflow>-<ref>`，`cancel-in-progress: true`。

## 必需的 job（genuine gates）

`check-workflows.py` 要求下列 job 必须存在且包含各自命令片段，缺失即失败：

| Job | Workflow | Runner | 门禁 |
|---|---|---|---|
| `core-quality` | core.yml | `ubuntu-latest` | fmt + clippy(exclude android/web) + 架构/fixture/workflow/迁移契约 + **代码文件 ≤1000 行**（`check-file-size.py`）+ **命令名/别名文档一致性**（`check-command-docs.py` + 自测 `test-check-command-docs.py`）+ 全目标 debug 编译；并安装 GTK/WebKitGTK 前置 + 构建 React 前端（`npm test`/`npm run build`） |
| `wasm-check` | core.yml | `ubuntu-latest` | 全 workspace wasm `--lib` 检查 + 单独 `app-web` 检查 |
| `i18n-contracts` | core.yml | `ubuntu-latest` | `python3 scripts/check-i18n.py`（双语目录 / key / 占位符 / Rust + React 静态 key / 硬编码 CJK 扫描） |
| `shader-validation` | build.yml | `ubuntu-latest` | 离线 WGSL 解析/校验（naga，无需 GPU），失败红灯 |
| `linux-app` | build.yml | `ubuntu-latest` | 安装 GTK/WebKitGTK 前置 + 构建并测试 React 前端 + `cargo check -p app-linux --all-targets --locked`，上传日志 |
| `windows-check` | build.yml | `windows-latest` | 原生 MSVC `cargo check -p app-windows --all-targets --locked`（**真实 Windows 宿主**的编译门禁；不运行 exe、不是视觉验收） |
| `macos-check` | build.yml | `macos-latest` | 原生 arm64 `cargo check -p app-macos --all-targets --locked`（占位桩，不运行） |
| `web-build` | build.yml | `ubuntu-latest` | 安装钉住版本的 wasm-bindgen CLI（预编译 release + SHA-256 + `Cargo.lock` 对照）→ `bash scripts/build-web.sh` → 上传 `web-dist` artifact |
| `web-smoke` | build.yml | `ubuntu-latest` | 下载 `web-dist` → `scripts/serve-web.py` → Playwright Chromium 打开页面，断言 `start: ok` / `target` / `physical` / `batches` / `draws` / `presented`；**SwiftShader 软件渲染**，非 GPU/像素证据 |
| `web-deploy` | build.yml | `ubuntu-latest` | **仅 `v*` tag 或手工 dispatch**；`needs: web-build`，下载同一 `web-dist` 后 `scripts/deploy-cloudflare-pages.sh`（`SKIP_BUILD=1`）发布到 Cloudflare Pages |
| `android-check` | core.yml | 能力相关（默认 SKIP） | `bash scripts/build-android-ui.sh`（wasm 会话 + `YACR_RUNTIME=android` chrome，因为 `tauri::generate_context!` 会内嵌 `frontendDist`）→ `cargo check --target aarch64-linux-android -p app-android --locked`（**真实 Android 宿主**的编译门禁；不跑 Gradle、不建 APK、不跑设备） |

`android-check` 是 capability-gated（`vars.ANDROID_CI_ENABLED`），`web-deploy` 是
**event-gated**（`github.event_name == 'workflow_dispatch' || startsWith(github.ref, 'refs/tags/v')`）；
条件不满足时 GitHub 显示 **skipped（未运行）**，绝不显示为通过。`windows-check`/`macos-check`
**只证明编译**，不代表这些平台可运行；`windows-check` 的 step summary 文案**仍是旧的
“NOT IMPLEMENTED placeholder”**，属**文案腐化**（实现已换成真实宿主），不得据此判断实现状态。

**CI 未覆盖但本地已执行的门禁**（2026-10-23，证据在 `$HOME/.cache/yacr-wine/logs/`）：
Windows MSVC `clippy`/`check`/release `build`、MinGW GNU release `build`、`app-windows` wasm 空壳
`check`——全部 rc=0。这些**不是** CI job 证据，也不代表 Windows 可运行。

## 已退役的 job（硬禁用，`if: false` + `RETIRED`）

下列 job **保留声明**、带无条件 `if: false` 与 `RETIRED` 注释，并含一个 `exit 1` 步骤；它们
**不含** `vars.…` 开关、`secrets.…` 引用或 `continue-on-error: true`。删除禁用守卫、改成仓库变量
或重新加入凭据都会被 `check-workflows.py` 判为硬错误：

`android-apk`、`android-release`、`linux-release`、`windows-release`、`macos-release`。

含义：**Android 宿主自 2026-10-28 起重新是真实宿主**（`apps/app-android` = Tauri 2 WebView 载体 +
wasm `dom-canvas` 会话），但 CI 仍**不**构建/签名/发布 APK——`android-apk`/`android-release` 保持硬禁用
（无签名密钥、无商店路径），APK 由本地 `scripts/build-android.sh` 产出；`android-check` 是
capability-gated 的编译门禁。macOS GUI 仍是显式 NOT IMPLEMENTED 占位；Windows 宿主 2026-10-23 起已实现，
`windows-check` 只是**编译门禁**（≠运行验收），`windows-release` 仍硬禁用。**Web job 已不在退役
列表**（2026-10-25）：`web-build`/`web-smoke` 是必需的验证 job，`web-deploy` 是 event-gated 的真实
发布 job（`secrets.…` + `v*` tag/dispatch）；`scripts/deploy-cloudflare-pages.sh` 也是真实发布入口，
发布前逐项校验 bundle 构件。`check-workflows.py` 仍会拒绝把 `web-deploy` 改成 `if: false` 或移除
tag/dispatch 门禁。
`linux-release` 的**脚本**已于 2026-10-17 重启（本地可构建，见 `docs/flatpak.md`），但该 CI job 仍
**硬禁用**；它与其余 job 一样不会产出或发布任何产物。

## 锁定工具版本

`build.yml` 顶部 `env:` 与 `docs/build.md`、`Cargo.lock` 对齐：

| 工具 | 锁定值 | 来源 |
|---|---|---|
| Rust | `1.99.0` | `rust-toolchain.toml` |
| Node | 见 workflow `NODE_VERSION` | React 前端构建/测试、Playwright、wrangler |
| wasm-bindgen CLI | `0.2.129`（预编译 musl release，SHA-256 固定） | `scripts/lib/wasm-toolchain.sh` 的 pin + `Cargo.lock` |
| Playwright | `1.64.1`（npm 钉住；不缓存浏览器二进制） | `web-smoke` job |

`web-build` 从 `wasm-bindgen` 官方 release 下载 `wasm-bindgen-0.2.129-x86_64-unknown-linux-musl`
并用固定 SHA-256 校验，再断言 `wasm-bindgen --version` 与 `Cargo.lock` 的 `0.2.129` 一致——不依赖
runner 上恰好存在的 CLI。**本地** bundle 构建（`scripts/build-web.sh`，与
`scripts/build-canvas-wasm.sh` 共用 `scripts/lib/wasm-toolchain.sh`）仍自己校验 CLI 与 `Cargo.toml`
的 `=0.2.129` 钉住**完全一致**，不一致即硬失败；`check-workflows.py` 断言该 pin 检查与
`cargo build -p app-web --target wasm32-unknown-unknown --release --locked` 不被删除。

## 本层 NOT RUN（明确未运行）

- `core-quality` / `wasm-check` / `i18n-contracts` / `shader-validation` / `linux-app` /
  `windows-check` / `macos-check` / `android-check` / `web-build` / `web-smoke` / `web-deploy`：
  workflow 已定义，结果以真实 job 为准并写入 `docs/validation*.md`。
- **Windows 视觉/运行时**：CI **没有**也不应有 Windows 运行验收（`windows-check` 只编译）；
  spike-1/4/5 由用户在真 Windows 自测（`docs/windows-app.md`），CI 不得冒充。
- **Linux X11/XWayland**：宿主显式拒绝（exit 4），CI 的 `linux-app` 只做编译/测试，不运行 GUI。
- 5 个退役 job：**永远不会**在受支持配置下运行（硬禁用）。
- **CI 上的真实 GPU / 黄金图 / 浏览器矩阵 / Android 真机**：未运行。GitHub 托管 runner 无 GPU，
  软件渲染不能作为真实 GPU 证据。
- **Android 设备**：CI **没有**设备/模拟器 job（`android-check` 只编译）。本地模拟器
  （x86_64 + `-gpu swiftshader_indirect`）的一次有界运行记录在 `docs/android-host.md`；
  它**不是**真机、真实 GPU 或像素验收。
- **响应式 chrome 验收未接线 CI**：`scripts/build-web-ui.sh`（→ `apps/app-web/dist-ui`）与
  `scripts/web-ui-smoke.cjs`（Chromium + SwiftShader 的布局/DOM 套件）、以及
  `scripts/build-android-ui.sh` / `scripts/build-android.sh` 都是**本地/手工**入口，**没有**对应
  workflow；CI 的 `web-smoke` 只冒烟旧的 `apps/app-web/dist`（见下一条「浏览器 bundle」），不得据本地
  结果宣称 CI 通过。本地有界记录见 `docs/validation-mobile-responsive-2026-10-28.md`。
- **浏览器 bundle**：`web-build` 在 CI 构建并上传 artifact；`web-smoke` 用 Playwright Chromium
  （SwiftShader 软件渲染）打开并断言 typed 状态行，因此**有** CI bundle 产物与 carrier/status 冒烟，
  但**没有**真实 GPU、像素/golden、性能或 WebKitGTK/其它浏览器矩阵证据；本地真实浏览器读数
  （Chromium 141 / WebKitGTK 2.52.6，合成夹具）记录在
  `docs/validation-dom-canvas-wasm-2026-10-24.md`，属**本地**证据，不是 CI 证据。
- **Web 发布**：`web-deploy` 只在 `v*` tag 或手工 dispatch 时运行，把该次 `web-build` 的 artifact
  发布到 Cloudflare Pages（`yacr-examples`）；上传成功只是**发布证据**，不是运行/GPU/像素验收。
  常规 push/PR 没有发布路径。
- **Linux 发布包**：CI 上仍没有受支持的流水线（`linux-release` 硬禁用）。`package-linux-release.sh`
  已于 2026-10-17 重启，本地可构建并安装 `.flatpak`（证据见
  `docs/validation-flatpak-linux-2026-10-17.md`）；**未**发布到 Flathub，也未在 CI 运行。

## 本地复现（当前门禁）

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo fmt --all -- --check
cargo clippy --workspace --exclude app-android --exclude app-web --all-targets --locked -- -D warnings
python3 scripts/check-architecture.py
python3 scripts/check-fixture-manifest.py
python3 scripts/check-workflows.py
python3 scripts/check-i18n.py
python3 scripts/check-file-size.py   # 代码文件 ≤1000 行；文档不计入；自测 scripts/test-check-file-size.py
python3 scripts/check-command-docs.py     # docs/shortcuts.md §2/§2b + autocad-command-coverage.md §3.7
python3 scripts/test-check-command-docs.py  # 自测：注入伪造 canonical + 伪造别名，必须退出 1
cargo check --workspace --all-targets --locked
cargo check --workspace --lib --target wasm32-unknown-unknown --locked
# React 前端
(cd apps/desktop-ui && npm ci && npm test && npm run build)
# 工作流/打包/迁移契约
python3 scripts/test-linux-workflow.py
python3 scripts/test-ui-migration-guards.py
python3 scripts/test-web-deploy-workflow.py
python3 scripts/test-package-linux-release.py
python3 scripts/test-package-windows-release.py
python3 scripts/test-package-macos-release.py
python3 scripts/test-fetch-fonts.py
python3 scripts/test-android-host.py
# 离线 WGSL 校验（无需 GPU）
cargo test -p cad-render-wgpu --test wgsl_validation --locked
# 全量 workspace 测试必须拆两步：先排除 cad-render-wgpu，再单独串行跑该 crate。
# 默认并行的整包命令会在 accelerated_tier 因 wgpu device 争用而“挂起”（不是失败），
# 见 docs/testing-dwg.md §3「全量 workspace 测试：必须排除 cad-render-wgpu 并单独串行」。
cargo test --workspace --exclude app-android --exclude app-web --exclude cad-render-wgpu --locked
cargo test -p cad-render-wgpu --locked -- --test-threads=1
```

浏览器 bundle（与 CI 的 `web-build` 同一条命令；**本地**运行，产物 git 忽略）：

```bash
bash scripts/build-web.sh    # → apps/app-web/dist/{index.html,main.js,worker.js,style.css,pkg/app_web.js,pkg/app_web_bg.wasm}
python3 scripts/serve-web.py --directory apps/app-web/dist --port 8090
# 可选：与 CI 相同的 headless 冒烟（需 `npm i playwright` 且已装 Chromium）
node scripts/web-smoke.cjs --url http://127.0.0.1:8090/
```

产物目录（`apps/app-web/dist/`、`apps/app-web/web/pkg/`）都被 git 忽略。构建不会产出半成品：
构件缺失/为空、wasm-bindgen pin 不符、工具链缺失都硬失败。`web-smoke` 在 CI 上走 SwiftShader
软件渲染，只能证明 carrier/状态行成立，**不是** GPU/像素证据；截图只是调试产物。

退役脚本的负向断言（每个应非零退出）：

```bash
for s in package-windows-release package-macos-release; do
  bash "scripts/$s.sh"; echo "$s exit=$? (expect 1)"
done
```

`scripts/build-android.sh` **不在**负向列表：2026-10-28 起它构建真实 APK（`tauri android build --apk`，
需要 SDK/NDK/JDK 17，release 默认未签名），失败即非零退出，绝不产出桩。
`scripts/build-web.sh`、`scripts/web-smoke.cjs` 与 `scripts/deploy-cloudflare-pages.sh` **不在**该
列表：它们构建真实 bundle、驱动真实浏览器、发布经过校验的 bundle（后者需要 Cloudflare 凭据，
且只在 tag/dispatch 由 CI 触发）；**不允许**用「退役桩」冒充成功。`package-linux-release.sh`
**不在**负向列表：它已于 2026-10-17 重启，按退出码 0 完成构建（本地产物，无发布，见
`docs/flatpak.md`）。

`scripts/check-workflows.py` 有 PyYAML 时做真实解析，否则退化为结构化文本检查并如实说明；
变异契约（`test-linux-workflow.py`、`test-web-deploy-workflow.py`、`test-package-*.py`）故意删除
关键片段并断言检查**必然报错**，证明门禁不是摆设。它们都不触网、不部署、不执行 release。
`test-web-deploy-workflow.py` 既证明 bundle/smoke/deploy 入口契约（pin 检查、wasm32 构建、输出目录、
`start: ok` 断言、构件校验先于上传）会因回归而失败，也证明 `web-deploy` 的 tag/dispatch 门禁
不能被改成「每次 push 都发布」。

`check-command-docs.py` 每次运行都从 `crates/cad-ui-core/src/session/commands.rs` 重新解析
`COMMAND_TABLE` / `MEASUREMENT_SELECTORS` / `ZOOM_STEP_SELECTORS` / `COMMAND_ALIASES`，然后检查
`docs/shortcuts.md` §2（canonical 列必须是准入名、别名单元必须是真实别名行且映射回本行）、§2b
（整张别名表必须与 `COMMAND_ALIASES` **完全一致**）、`docs/autocad-command-coverage.md` §3.7（第一列
必须是准入名），以及两篇文档里每一处 ``别名 `x` `` 提法（先剥离围栏代码块）。退出码 **0** = 一致
（`OK: 0 mismatches`）、**1** = 有 mismatch（逐条打印）、**2** = 文档/源码形状读不出来（缺章节或
缺常量表，**不会**当成通过）。**守门范围只有名字**：§1/§3/§4/§5 与 §3 的语义边界仍是人工核对。
本条目的证据是**本地**退出码（`scripts/test-check-command-docs.py` 注入伪造 canonical 与伪造别名
并断言退出 1），**不是** GitHub runner 上的 job 结果（本环境 NOT RUN）。
