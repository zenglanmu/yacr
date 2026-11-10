# CI（GitHub Actions）分层工作流

本文件描述仓库**当前**实际提供的 CI 运行层。命令权威仍是 `AGENTS.md`「构建与门禁」与
`docs/build.md`；本文件只说明 workflow 如何镜像该门禁，以及哪些层次**未运行 / 仅出 artifact**。

工作流文件：`.github/workflows/core.yml`（核心质量层）与 `.github/workflows/build.yml`
（构建/校验层）。校验脚本：`scripts/check-workflows.py`（必需 job 片段 + release job 事件门禁契约 +
浏览器 build/smoke/deploy 入口契约）、`scripts/test-linux-workflow.py`（变异契约）、
`scripts/test-web-deploy-workflow.py`（浏览器流水线变异契约）、
`scripts/test-release-workflows.py`（4 个 release job 的事件门禁与入口契约）、
`scripts/test-ui-migration-guards.py`（迁移守卫契约）、
`scripts/check-command-docs.py`（§2/§2b/§3.7 的命令名与别名一致性，权威是
`crates/cad-ui-core/src/session/commands.rs`）与它的自测
`scripts/test-check-command-docs.py`（注入伪造名字/别名，断言检查器必然非零退出）。

> **2026-10-16 迁移状态（2026-10-23 增补）**：桌面已迁移到 React/Tauri（`apps/app-linux`、`yacr`）。
> 编译门禁已加入 Tauri/GTK 前置（`libwebkit2gtk-4.1-dev`）与 React 前端构建
> （`apps/desktop-ui`、`npm test`、`npm run build`）。**2026-10-23 起**：`windows-check` 编译的是
> **真实 Windows 宿主**（不再是 NOT IMPLEMENTED 桩），但**仍只是编译门禁**；X11/XWayland 在
> Linux 宿主中被显式拒绝（退出码 4），CI 不运行 GUI。**2026-11-01 起** 4 个发布 job
> （`linux-release`/`windows-release`/`macos-release`/`android-release`）已从
> `if: false` 重新启用为 **event-gated artifact builder**：仅 `v*` tag 或手工 `workflow_dispatch`
] 运行，仅上传 workflow artifact，不发布、不接触商店凭据；Windows/Android/macOS 用仓库自持
> 密钥自签名（Windows PFX、Android keystore、macOS ad-hoc）、Flatpak 在 GPG 凭据存在时签
> OSTree commit，缺失时显式产出 `unsigned` artifact
> （见 `docs/code-signing.md`）。见 ADR 0010 与 ADR 0023。

> **2026-10-25 增补（Web 流水线恢复）**：`apps/app-web` 的 wasm CAD 会话（ADR 0024）已在 CI 中
> 端到端接线：`web-build` 用**钉住 SHA-256 的 wasm-bindgen 预编译 CLI** 与**钉住
> version_120 的 binaryen `wasm-opt`** 构建（`[profile.wasm]` + `wasm-opt -Oz`，模块必须 ≤ 25 MiB
> 才能上传 Cloudflare Pages）`apps/app-web/dist` 并上传 artifact；`web-smoke` 下载该 artifact、起静态服务器、用 Playwright
> Chromium 打开页面并断言 typed `start: ok` 状态行；同一 job 还构建**共享 chrome bundle**
> （`scripts/build-web-ui.sh` → `apps/app-web/dist-ui`，上传为 `web-ui-dist`）。
> `web-deploy` 只在 **`v*` tag 或手工 `workflow_dispatch`** 时把 `web-ui-dist`
> （`BUNDLE_KIND=ui`）发布到 Cloudflare Pages（`yacr-examples`），唯一发布入口是项目自带的
> `https://yacr-examples.pages.dev`；自定义域名属 Cloudflare 项目侧配置，workflow 与
> `scripts/deploy-cloudflare-pages.sh` **不** attach/detach 任何自定义域名。
> 常规 push/PR **不会**发布。CI runner 无 GPU：`web-smoke` 走 **SwiftShader 软件渲染**，
> 因此它是 carrier/status 冒烟，**不是** GPU/像素/性能证据。CI 也未覆盖 WebKitGTK/其它浏览器
> 矩阵（本地证据见 `docs/validation-dom-canvas-wasm-2026-10-24.md`）。

> **本环境（无 GitHub runner、无 `gh` 权限）没有实际执行这些 job**；本文件描述的是 workflow 内容
> 与预期，**不是通过证据**。真实 job URL/commit/结果应写入 `docs/validation*.md`。

## 触发、权限与并发

- 触发：`push`、`pull_request`、`workflow_dispatch`。
- 权限：顶层 `permissions: contents: read`。发布凭据只在对应 job 使用：`web-deploy` 用
  `CLOUDFLARE_API_TOKEN` / `CLOUDFLARE_ACCOUNT_ID`；`windows-release`/`android-release` 在配置时
  用自签名 secrets/variables（`macos-release` 无密钥，ad-hoc）；`linux-release` 用 `FLATPAK_GPG_*`。它们都仅在 `v*` tag 或手工
  dispatch 时运行；其余 job 不接触发布凭据。
- 并发：`concurrency.group = <workflow>-<ref>`，`cancel-in-progress: true`。
- 字体缓存：`scripts/fetch-fonts.sh` 按文件缓存到 `${XDG_CACHE_HOME:-$HOME/.cache}/yacr/fonts`；
  `linux-release` / `windows-release` / `macos-release` / `android-release` / `web-build` /
  `android-check` 用 `actions/cache`（`key: <os>-yacr-fonts-<run_id>` + 前缀 restore）跨 run 复用，
  避免重复下载 mlightcad 全量目录。缓存只是传输层复用，未改变下载失败仍然硬失败。

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
| `macos-check` | build.yml | `macos-latest` | 先构建/测试 React 前端（`apps/desktop-ui`），再 `cargo check -p app-macos --all-targets --locked`（**真实 macOS 宿主**的编译门禁；不运行、不是视觉/Metal/DPI 验收） |
| `web-build` | build.yml | `ubuntu-latest` | 安装钉住版本的 wasm-bindgen CLI 与 binaryen `wasm-opt`（预编译 release + SHA-256）→ `bash scripts/build-web.sh`（`[profile.wasm]` + `wasm-opt -Oz`，≤ 25 MiB）→ 上传 `web-dist` artifact |
| `web-smoke` | build.yml | `ubuntu-latest` | 下载 `web-dist` → `scripts/serve-web.py` → Playwright Chromium 打开页面，断言 `start: ok` / `target` / `physical` / `batches` / `draws` / `presented`；**SwiftShader 软件渲染**，非 GPU/像素证据 |
| `web-deploy` | build.yml | `ubuntu-latest` | **仅 `v*` tag 或手工 dispatch**；`needs: web-build`，下载 `web-ui-dist`（`apps/app-web/dist-ui`）后 `scripts/deploy-cloudflare-pages.sh`（`SKIP_BUILD=1`、`BUNDLE_KIND=ui`）发布到 Cloudflare Pages |
| `android-check` | core.yml | 能力相关（默认 SKIP） | `bash scripts/build-android-ui.sh`（`YACR_RUNTIME=android` chrome bundle，因为 `tauri::generate_context!` 会内嵌 `frontendDist`；脚本目前仍一并产出 wasm 产物，`native-below` 载体不加载）→ `cargo check --target aarch64-linux-android -p app-android --locked`（**真实 Android 宿主 / 原生 `native-below` 载体**的编译门禁；不跑 Gradle、不建 APK、不跑设备） |
| `linux-release` | build.yml | `ubuntu-latest` | **仅 `v*` tag / 手工 dispatch**；安装 GTK/WebKitGTK + `flatpak-builder` + `org.gnome.Platform//51`/`Sdk//51` → `bash scripts/package-linux-release.sh` → 上传 `.flatpak` + `.sha256`（配置 `FLATPAK_GPG_*` 时对 OSTree commit 签名并附 `*.flatpak.gpg`；不发布 Flathub） |
| `windows-release` | build.yml | `windows-latest` | **仅 `v*` tag / 手工 dispatch**；先构建 React 前端 → `bash scripts/package-windows-release.sh`（MSVC 静态 CRT，PE/图标校验）→ 自签 PFX 存在时对两个 `.exe` 做 Authenticode 签名（附 `yacr-dev-codesign.cer`，exe/cer 均在包根目录）并上传 `yacr-windows-release`，否则上传 `yacr-windows-release-unsigned` |
| `macos-release` | build.yml | `macos-latest` | **仅 `v*` tag / 手工 dispatch**；先构建 React 前端 → `bash scripts/package-macos-release.sh`（universal + `lipo`，Mach-O 仅系统库）→ 默认 ad-hoc `codesign`（`MACOS_SIGN_IDENTITY` 可覆盖）后上传 `.tar.gz` + `.sha256`（未公证；`spctl` 仍会拒绝） |
| `android-release` | build.yml | `ubuntu-latest` | **仅 `v*` tag / 手工 dispatch**；同上但 `--release`，`zipalign` 后用仓库 keystore 签名（keystore 缺失时预检直接失败）并上传 `android-release`（APK + `SIGNING.txt` + `.sha256`），不产出 unsigned；无商店上传 |

`android-check` 是 capability-gated（`vars.ANDROID_CI_ENABLED`）；`web-deploy` 与上面 4 个
release job 是 **event-gated**（`github.event_name == 'workflow_dispatch' ||
startsWith(github.ref, 'refs/tags/v')`）；条件不满足时 GitHub 显示 **skipped（未运行）**，
绝不显示为通过。`windows-check`/`macos-check`
**只证明编译**，不代表这些平台可运行；`windows-check` 的 step summary 文案**仍是旧的
“NOT IMPLEMENTED placeholder”**，属**文案腐化**（实现已换成真实宿主），不得据此判断实现状态。

**CI 未覆盖但本地已执行的门禁**（2026-10-23，证据在 `$HOME/.cache/yacr-wine/logs/`）：
Windows MSVC `clippy`/`check`/release `build`、MinGW GNU release `build`、`app-windows` wasm 空壳
`check`——全部 rc=0。这些**不是** CI job 证据，也不代表 Windows 可运行。

## 发布打包 job（2026-11-01 重新启用，event-gated）

下列 job 不再是 `if: false` 退役占位；它们对**当前 React/Tauri 宿主**构建真实 artifact，并且只在
`v*` tag 或手工 `workflow_dispatch` 运行。发布 job 本身**不发布**；在自签名/GPG 凭据存在时
签名，缺失时显式产出 `unsigned` artifact，绝不写成「已签名」。任何一次运行都只看 artifact
上传结果，不公证、不发布到 Flathub/商店：

| Job | 产物 | 边界 |
|---|---|---|
| `linux-release` | `.flatpak` + `.sha256`（配置 `FLATPAK_GPG_*` 时另附 `.flatpak.gpg`） | 凭据缺失时未签名；未发布 Flathub；无窗口/GPU 验收 |
| `windows-release` | `yacr-windows-release`（`.zip` + `.sha256`）；无凭据时 `yacr-windows-release-unsigned` | 自签 PFX 配置时签名（非公共信任）；无运行/GPU 验收 |
| `macos-release` | `.tar.gz`（`Yacr.app`）+ `.sha256` | ad-hoc 签名（或配置的 identity）/未公证；无窗口/Metal/DPI 验收 |
| `android-release` | `android-release`（APK + `SIGNING.txt` + `.sha256`） | **必须有自持 keystore，缺失时 job 失败**；无商店上传；未安装运行；签名 ≠ 设备/GPU 验收 |

`scripts/check-workflows.py` 与 `scripts/test-release-workflows.py` 固定这些性质：job 必须存在、
必须调用真实入口脚本、必须保留 `refs/tags/v` / `workflow_dispatch` 门禁、不得出现 `if: false`、
`continue-on-error` 或发布命令；签名步骤只在自签名/GPG 凭据存在时运行，缺失时产出显式
`unsigned` artifact。`RETIRED_JOBS` 机制保留为空元组，供未来再次退役时使用。

Android：宿主自 2026-11-01 起是原生 `native-below` 载体（`apps/app-android` = Tauri 2 WebView +
原生 `CadSession`/wgpu Vulkan → `SurfaceView`，ADR 0027；wasm 会话不实例化）。`android-check` 仍是
capability-gated 的**编译门禁**；两个 APK job 只构建 artifact（release 在自签名 keystore 存在时签名），
**没有**设备/模拟器、Vulkan adapter、像素或 JNI 运行时链接证据。Windows/macOS 的 `*-release` 只证明
**能在对应 runner 上打包**，不证明真实窗口/GPU/对话框/视觉行为。所需 secrets/variables 与验证方式见
`docs/code-signing.md`。

## 公开发布（`publish-release.yml`，手工）

GitHub Actions 的 workflow artifact 需要登录且会过期，**不能**作为公开下载入口。公开分发由单独的
`.github/workflows/publish-release.yml` 完成：**仅 `workflow_dispatch`**，是唯一带
`permissions: contents: write` 的 workflow（并显式 `actions: read` 下载 artifact——一旦声明
`permissions:`，未列出的 scope 就是 `none`）。它**不构建/不打包/不签名**，只从指定的
`CI build layers` run 拉取四个平台 artifact：`android-release` 直接取签名 APK；Windows 优先
`yacr-windows-release`，缺失时回退 `yacr-windows-release-unsigned`；Flatpak artifact 若含 `*.flatpak.gpg` 则一并分发。校验 `.sha256` 后
附加到该 tag 的 GitHub Release；release notes 来自仓库内文件（如 `docs/release-notes/v0.1.4.md`），
按 `docs/release-notes/TEMPLATE.md` 写：只列用户可见变化，证据边界文末一句带过。
**签名只发生在 build job（自签名/GPG），publisher 从不签名。**

职责因此是分开的：`build.yml` 的 release job 构建 + 上传 artifact（在凭据存在时签名；
`contents: read`，`test-release-workflows.py` 禁止它们出现 `gh release`）；公开上传只发生在手工
dispatch 的 publisher 里。publisher 全绿只代表 **artifact 上传成功**，不是签名/运行/GPU/设备/
视觉验收。

## 锁定工具版本

`build.yml` 顶部 `env:` 与 `docs/build.md`、`Cargo.lock` 对齐：

| 工具 | 锁定值 | 来源 |
|---|---|---|
| Rust | `1.99.0` | `rust-toolchain.toml` |
| Node | 见 workflow `NODE_VERSION` | React 前端构建/测试、Playwright、wrangler |
| wasm-bindgen CLI | `0.2.129`（预编译 musl release，SHA-256 固定） | `scripts/lib/wasm-toolchain.sh` 的 pin + `Cargo.lock` |
| binaryen (`wasm-opt`) | `version_120`（GitHub release，SHA-256 固定） | `scripts/lib/wasm-toolchain.sh` 的 pin；Cloudflare Pages 单文件 25 MiB 上限 |
| Playwright | `1.64.1`（npm 钉住；不缓存浏览器二进制） | `web-smoke` job |

`web-build` 从 `wasm-bindgen` 官方 release 下载 `wasm-bindgen-0.2.129-x86_64-unknown-linux-musl`、
从 binaryen 官方 release 下载 `binaryen-version_120-x86_64-linux`，都用固定 SHA-256 校验，再断言
CLI 版本与 pin 一致——不依赖 runner 上恰好存在的工具。**本地** bundle 构建（`scripts/build-web.sh`，
与 `scripts/build-canvas-wasm.sh` 共用 `scripts/lib/wasm-toolchain.sh`）自己校验 wasm-bindgen 与
`Cargo.toml` 的 `=0.2.129`、`wasm-opt` 与 binaryen `120` **完全一致**，不一致即硬失败；产物用
`wasm-opt -Oz --skip-pass=merge-similar-functions --strip-debug --strip-producers` 压缩后仍 > 26,214,400 字节（Cloudflare Pages 单文件
25 MiB 上限）也是硬失败。`check-workflows.py` 断言这些 pin 检查、
`cargo build -p app-web --target wasm32-unknown-unknown --profile wasm --locked`、`wasm-opt` 与
25 MiB 上限不被删除。

## 子模块推送契约（pre-push hook）

`plugins/tch` 是 git submodule，superproject 的 gitlink 必须指向 `tangent-ts` 远端**已发布**的
commit。若只在本地提交插件改动而没推子模块，superproject 推送后**每个** job 都会在
`actions/checkout` 直接失败（不是门禁失败）：

```
Submodule 'plugins/tch' registered for path 'plugins/tch'
fatal: remote error: upload-pack: not our ref <sha>
fatal: Fetched in submodule path 'plugins/tch', but it did not contain <sha>
```

2026-11-07 的 `42338811245` / `42338811233` 就是这一原因（gitlink `0ce4dbb`，远端仍在 `7ecb6d8`）。

本地保护（一次性安装，`core.hooksPath` 是仓库本地配置，不随提交分发）：

```bash
git config core.hooksPath .githooks
```

`.githooks/pre-push` → `scripts/pre-push-submodules.py`：读取 pre-push 协议的所有出站 ref，用
`git ls-tree -r <pushed-sha>` 取出 gitlink，对每个 `(submodule, sha)` 执行 fetch 后检查该 commit 是否
已被 `refs/remotes/<remote>/` 包含；未被包含时把**包含它的本地分支**推到其 upstream（缺 upstream 则推到
`refs/heads/<branch>`），推完再校验一次可达性。

- **fail-closed**：子模块未初始化、commit 本地不存在、commit 不在任何本地分支（detached/unreferenced）、
  子模块 push 被拒，都会以非零码**阻断** superproject 推送并打印 `pre-push: BLOCKED ...`。
- 只校验**被推送 ref 的 tip commit**（CI checkout 构建的就是 tip）；中间历史里的旧 gitlink 不追溯。
- 只处理**直接**子模块，不递归推送嵌套子模块；若子模块自身带子模块，需手工先推。
- `--dry-run` 只报告 `would push`，`--no-push` 只校验并阻断；`SKIP_SUBMODULE_PUSH_CHECK=1` 是逃生口，
  会打印警告且不校验（此时 CI 失败风险由使用者承担）。
- 契约测试：`python3 scripts/test-pre-push-submodules.py`（临时仓库夹具，7 个用例，无网络）。

## 本层 NOT RUN（明确未运行）

- `core-quality` / `wasm-check` / `i18n-contracts` / `shader-validation` / `linux-app` /
  `windows-check` / `macos-check` / `android-check` / `web-build` / `web-smoke` / `web-deploy`：
  workflow 已定义，结果以真实 job 为准并写入 `docs/validation*.md`。
- **Windows 视觉/运行时**：CI **没有**也不应有 Windows 运行验收（`windows-check` 只编译）；
  spike-1/4/5 由用户在真 Windows 自测（`docs/windows-app.md`），CI 不得冒充。
- **Linux X11/XWayland**：宿主显式拒绝（exit 4），CI 的 `linux-app` 只做编译/测试，不运行 GUI。
- 5 个 release job：已在 GitHub Actions 真实运行两次（`v0.1.3` run `41304110619`：
  Linux/Windows/macOS/web ✅、Android 两个 job 因 `-lnativewindow` ❌；`v0.1.4` run `41307807752`：
  五个 release job + `web-deploy` 全部 ✅）。它们只产出 workflow artifact，不发布商店；签名在
  自签名/GPG 凭据存在时进行，缺失时显式产出 `unsigned` artifact（真实签名请求本轮 **NOT RUN**）。
  公开下载由手工 dispatch 的 `publish-release.yml` 负责（见上节）。
- **CI 上的真实 GPU / 黄金图 / 浏览器矩阵 / Android 真机**：未运行。GitHub 托管 runner 无 GPU，
  软件渲染不能作为真实 GPU 证据。
- **Android 设备**：CI **没有**设备/模拟器 job（`android-check` 只编译）。**2026-11-01 的
  `native-below` 载体（P5）只有编译级证据**（`cargo check`/clippy/开发机契约测试/Kotlin 编译/`javap`
  JNI 符号核对），**没有**安装、启动、Vulkan adapter、像素、旋转/前后台或 JNI 运行时链接记录。
  2026-10-28 的模拟器（x86_64 + `-gpu swiftshader_indirect`）出图记录属于**已退役的 `dom-canvas`
  载体**，不得作为 native 载体的证据；见 `docs/android-host.md` §8/§10。
- **响应式 chrome：构建/发布已接线，*验收* 仍未接线**：`web-build` 现在同时构建
  `scripts/build-web-ui.sh`（→ `apps/app-web/dist-ui`），`web-deploy` 发布该 artifact；
  `scripts/build-android-ui.sh` 由 `android-check` 与两个 APK job 调用。
  `scripts/web-ui-smoke.cjs`（Chromium + SwiftShader 的布局/DOM 套件）仍是**本地/手工**入口，
  **没有**对应 workflow。CI 的 `web-smoke` 只冒烟旧的 `apps/app-web/dist`（见下一条「浏览器
  bundle」），不得据本地结果宣称 CI 通过。本地有界记录见
  `docs/validation-mobile-responsive-2026-10-28.md`。
- **浏览器 bundle**：`web-build` 在 CI 构建并上传 artifact；`web-smoke` 用 Playwright Chromium
  （SwiftShader 软件渲染）打开并断言 typed 状态行，因此**有** CI bundle 产物与 carrier/status 冒烟，
  但**没有**真实 GPU、像素/golden、性能或 WebKitGTK/其它浏览器矩阵证据；本地真实浏览器读数
  （Chromium 141 / WebKitGTK 2.52.6，合成夹具）记录在
  `docs/validation-dom-canvas-wasm-2026-10-24.md`，属**本地**证据，不是 CI 证据。
- **Web 发布**：`web-deploy` 只在 `v*` tag 或手工 dispatch 时运行，把该次 `web-build` 产出的
  `web-ui-dist`（共享 chrome bundle，`BUNDLE_KIND=ui`）发布到 Cloudflare Pages（`yacr-examples`）；
  发布入口固定为项目的 `https://yacr-examples.pages.dev`（脚本只上传并打印该 URL，不管理自定义域名）。
  上传成功只是**发布证据**，不是运行/GPU/像素验收。常规 push/PR 没有发布路径。
- **Linux 发布包**：`linux-release` 已重新启用（event-gated），CI 会构建并上传 `.flatpak` artifact
  （配置 `FLATPAK_GPG_*` 时对 OSTree commit 签名并附 `*.flatpak.gpg`，否则未签名）；**未**发布到
  Flathub。本轮**没有**在 GitHub runner 上实际运行该 job；本地 2026-10-17 的构建/安装证据仍见
  `docs/validation-flatpak-linux-2026-10-17.md`（历史证据，不代表 CI 已通过）。

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
python3 scripts/check-command-docs.py     # docs/shortcuts.md §2/§2b + autocad-2026-command-coverage.md 的「测量选择子」一节
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
python3 scripts/test-release-workflows.py
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

发布打包入口现在都是真实流水线，**没有**「退役桩」负向列表：
`scripts/package-linux-release.sh`、`scripts/package-windows-release.sh`、
`scripts/package-macos-release.sh` 分别构建 Flatpak / MSVC zip / macOS tarball；
`scripts/build-android.sh` 构建真实 APK（需要 SDK/NDK/JDK 17，release 默认未签名）；
`scripts/build-web.sh`、`scripts/web-smoke.cjs`、`scripts/deploy-cloudflare-pages.sh` 构建/驱动/
发布经过校验的 bundle。任何一步失败都必须非零退出，**不允许**伪造空成功。
Linux Flatpak 的本地运行方式见 `docs/flatpak.md`；Windows/macOS 打包脚本只能在对应操作系统上
真正执行（或按脚本支持的方式交叉构建），本环境未运行。

`scripts/check-workflows.py` 有 PyYAML 时做真实解析，否则退化为结构化文本检查并如实说明；
变异契约（`test-linux-workflow.py`、`test-web-deploy-workflow.py`、`test-release-workflows.py`、
`test-package-*.py`）故意删除关键片段并断言检查**必然报错**，证明门禁不是摆设。它们都不触网、
不部署、不执行 release。
`test-web-deploy-workflow.py` 既证明 bundle/smoke/deploy 入口契约（pin 检查、wasm32 构建、输出目录、
`start: ok` 断言、构件校验先于上传）会因回归而失败，也证明 `web-deploy` 的 tag/dispatch 门禁
不能被改成「每次 push 都发布」。

`check-command-docs.py` 每次运行都从 `crates/cad-ui-core/src/session/commands.rs` 重新解析
`COMMAND_TABLE` / `MEASUREMENT_SELECTORS` / `ZOOM_STEP_SELECTORS` / `COMMAND_ALIASES`，然后检查
`docs/shortcuts.md` §2（canonical 列必须是准入名、别名单元必须是真实别名行且映射回本行）、§2b
（整张别名表必须与 `COMMAND_ALIASES` **完全一致**）、`docs/autocad-2026-command-coverage.md` 的「测量选择子」一节（第一列
必须是准入名），以及两篇文档里每一处 ``别名 `x` `` 提法（先剥离围栏代码块）。退出码 **0** = 一致
（`OK: 0 mismatches`）、**1** = 有 mismatch（逐条打印）、**2** = 文档/源码形状读不出来（缺章节或
缺常量表，**不会**当成通过）。**守门范围只有名字**：§1/§3/§4/§5 与 §3 的语义边界仍是人工核对。
本条目的证据是**本地**退出码（`scripts/test-check-command-docs.py` 注入伪造 canonical 与伪造别名
并断言退出 1），**不是** GitHub runner 上的 job 结果（本环境 NOT RUN）。

## 宣传页部署（Landing site）

静态宣传页 `apps/landing/` 部署到 Cloudflare Pages（`https://yacr.pages.dev/`），安装包镜像到
R2 桶 `yacr-downloads`。`.github/workflows/site.yml` 是**手动 dispatch**（发布完成后再跑），
先 R2 后 Pages；契约测试 `python3 scripts/test-landing-site.py` 已接入 `core-quality`。
入口/凭据/诚实边界见 [`docs/landing-site.md`](landing-site.md)。
