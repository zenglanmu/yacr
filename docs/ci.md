# CI（GitHub Actions）分层工作流

本文件描述仓库实际提供的 CI 运行层（审计 N02 §6）。命令权威仍是 `AGENTS.md`
“构建”一节与 `docs/build.md`；本文件只说明 workflow 如何镜像该门禁，以及哪些层次
**尚未运行**。

工作流文件：`.github/workflows/core.yml`（核心质量层）与
`.github/workflows/build.yml`（N02 构建/校验层）。校验脚本：`scripts/check-workflows.py`。

## Linux App debug 主门禁（2026-10-04 契约变更）

默认启用 `build.yml/linux-app`（无 capability guard、无 continue-on-error）：安装 pkgconf、
fontconfig/freetype 开发库，执行 `cargo check -p app-linux --all-targets --locked`，
上传编译日志（失败时也尝试保留）。`core-quality` 包含 Linux App/Slint 的严格 clippy
和全目标 debug 编译，不再默认运行完整 Rust 测试。
`check-workflows.py` 强制 debug 编译命令和日志产物，禁止给主 job 添加条件跳过；
`test-linux-workflow.py` 防止空成功与默认 Linux 门禁恢复离屏/release 执行。

Linux App 仍是优先宿主，但默认门禁只证明编译；Web/Android 保持独立构建/发布层，
其打包配置不代表默认本地提交要求。WASM 检查 Linux 空库仅验证平台隔离。
Linux 离屏与 release 验证保留为明确请求时运行的可选工具，不作为本轮运行证据。
本机执行与 GitHub Actions 实际 job URL/结果分开记录，本轮尚未取得远程 job 执行证据。

> 本环境（无 GitHub runner、无 `gh` 权限）**没有实际执行**这些 job；本文件描述的
> 是 workflow 内容与预期，不是通过证据。真实 job URL / commit / 结果应写入
> `docs/validation.md`（由控制器汇总，本文件不代填）。

## 触发、权限与并发

- 触发：`push`、`pull_request`、`workflow_dispatch`（可手动重跑单层）。
- 权限：顶层 `permissions: contents: read`；唯一写外部系统的是 **能力门控**
  的 `web-deploy`（Cloudflare Pages），它用仓库 secrets 而非 GitHub 写权限。
  其余 job 无发布/上传存储步骤（artifact 上传只需只读权限）。
- 并发：`concurrency.group = <workflow>-<ref>`，`cancel-in-progress: true`，
  同 ref 的旧运行被取消。
- 超时：`core-quality` 45 分钟、`wasm-check` 45 分钟、`android-check` 60 分钟。
- 缓存：`actions/cache@v4` 缓存 `~/.cargo/registry`、`~/.cargo/git`、`target`，
  key 为 `${{ runner.os }}-cargo-<job>-${{ hashFiles('Cargo.lock', 'rust-toolchain.toml') }}`
  （OS + `Cargo.lock` + `rust-toolchain.toml`），cache miss 仍可构建，不把陈旧
  target 当证据。

### Action 版本

| Action | Pin | 说明 |
|---|---|---|
| `actions/checkout` | `@v4` | 沿用仓库原有 pin，未改动 |
| `dtolnay/rust-toolchain` | `@1.99.0` | 与 `rust-toolchain.toml` 的 channel 一致，沿用仓库原有 pin |
| `actions/cache` | `@v4` | 本层新增的唯一缓存 action；与 `actions/checkout@v4` 相同的 major tag 风格 |
| `actions/upload-artifact` | `@v4` | `web-build` / `linux-release` / `android-apk` / `android-release` / `web-smoke` 上传产物，只读权限即可 |
| `actions/setup-node` | `@v4` | `web-host-contracts` / `web-smoke` / `web-deploy` 安装锁定 Node |
| `actions/download-artifact` | `@v4` | `web-deploy` 取回 `web-build` 已验证的 `web-dist`，避免重新构建未验证产物 |

这些 action 未固定完整 commit SHA：编写时无法联网核验 SHA，禁止凭空编造。
仓库若采用 SHA 固定策略，应替换为已核验的完整 SHA（§6.3）。

## 已实现的 job

| Job | Workflow | Runner | 门禁 |
|---|---|---|---|
| `linux-app` | build.yml | `ubuntu-latest` | Linux App debug 全目标编译与日志，不运行渲染 |
| `linux-release` | build.yml | `ubuntu-latest`（按需/tag） | Linux GUI+CLI tar.gz + 全量字体，上传 artifact |
| `windows-check` | build.yml | `windows-latest` | 原生 MSVC 目标 `cargo check -p app-windows`，不运行 PE |
| `windows-release` | build.yml | `windows-latest`（按需/tag） | MSVC release 打包 + 全量字体 zip，上传 artifact |
| `macos-check` | build.yml | `macos-latest` | 原生 arm64 macOS 目标 `cargo check -p app-macos`，不运行 app |
| `macos-release` | build.yml | `macos-latest`（按需/tag） | universal `Yacr.app` tar.gz + 全量字体，上传 artifact |
| `android-release` | build.yml | 能力相关（默认 SKIP，按需/tag） | 含全量字体的 release APK，上传 artifact |
| `core-quality` | core.yml | `ubuntu-latest` | debug 全目标编译 + fmt + clippy + 架构边界，失败即红灯 |
| `wasm-check` | core.yml | `ubuntu-latest` | 完整 workspace wasm `--lib` 检查 + 单独 `app-web` 检查 |
| `i18n-contracts` | core.yml | `ubuntu-latest` | 双语 catalog / 缺 key / 硬编码白名单校验 |
| `shader-validation` | build.yml | `ubuntu-latest` | naga 离线 WGSL 解析/校验（无需 GPU），失败红灯 |
| `web-build` | build.yml | `ubuntu-latest` | `scripts/build-web.sh` 产出 `web-dist` 并上传，校验 wasm/JS 配对 |
| `web-deploy` | build.yml | `ubuntu-latest`（能力相关，默认 SKIP） | 将已验证的 `web-dist` 发布到 Cloudflare Pages（仅 main push） |
| `web-host-contracts` | build.yml | `ubuntu-latest` | Node 内置测试：模块边界、未保存决策、导出确认、本地化及轮询（无需 wasm/GPU） |
| `android-check` | core.yml | 能力相关（默认 SKIP） | aarch64 上 `cad-ui-slint` + `app-android` 的 `cargo check` |
| `android-apk` | build.yml | 能力相关（默认 SKIP） | 产出/上传 APK，记录真实 manifest facts（不宣称安装运行） |
| `web-smoke` | build.yml | 能力相关（默认 SKIP） | 软件 GPU 无头 Chromium 冒烟（非真机、非 WebGPU 验收） |

### 锁定工具版本

`build.yml` 顶部 `env:` 与 `docs/build.md`、`Cargo.lock` 严格对齐；每个 job 运行时
**再次核验**，漂移即失败（绝不静默改锁文件）：

| 工具 | 锁定值 | 对齐来源 |
|---|---|---|
| Rust | `1.99.0` | `rust-toolchain.toml` |
| `wasm-bindgen-cli` | `0.2.129` | `Cargo.lock` 的 `wasm-bindgen`（`web-build` 用脚本读取 lock 并比对，不一致直接失败） |
| `cargo-apk` | `0.10.0` | `docs/build.md` |
| Android NDK | `27.0.12077973` | `docs/build.md` |
| Android build-tools | `34.0.0` | `docs/build.md` |
| Android platform | `android-34` | `docs/build.md` |
| JDK | `17` | `docs/build.md`（运行时校验 `java -version` 主版本） |
| 预期 targetSdk / ABI / minSdk | `30` / `arm64-v8a` / `23` | 实测 manifest（`min_sdk_version` metadata 未生效，`docs/build.md` 已记录） |
| Node | `22.22.1` | `web-smoke`、`web-deploy` |
| Playwright | `1.55.1` | `web-smoke`（npm 锁定版本，不使用个人全局安装） |
| Wrangler | `4`（npm dist-tag） | `scripts/deploy-cloudflare-pages.sh` 默认值；`web-deploy` 不覆盖 |


### `core-quality`（必需）

镜像 `AGENTS.md` 的纯核心门禁，步骤顺序：

1. `cargo fmt --all -- --check`
2. `cargo clippy --workspace --exclude app-android --exclude app-web --all-targets --locked -- -D warnings`
3. 架构、fixture、workflow 静态检查及 Python workflow 变异契约。
4. `cargo check --workspace --exclude app-android --exclude app-web --all-targets --locked`

Linux 宿主安装 fontconfig/freetype 开发头，编译包含 `cad-ui-slint` 与 app-linux。
Rust 测试代码通过 `--all-targets` 编译，但不执行测试。该 job 不设 `continue-on-error`，
失败必须红灯。

### `linux-release`（发布包，按需）

`runs-on: ubuntu-latest`，`if: github.event_name == 'workflow_dispatch' ||
startsWith(github.ref, 'refs/tags/v')`。安装 Slint 所需的 `pkgconf
libfontconfig1-dev libfreetype6-dev`，`cargo fetch --locked` 预热依赖
（`package-linux-release.sh` 以 `--offline` 构建，避免锁文件被静默重解析），随后
`WITH_FONTS=1 scripts/package-linux-release.sh`，上传 `yacr-linux-release` artifact
（tar.gz + sha256）。脚本在打包内实际运行 `bin/cad-cli-tools --help` 与
`bin/yacr-linux --headless`（解析错误路径）验证可加载，并检查 RPATH 生效、无缺失库；
**不**运行窗口/GPU 渲染（`YACR_LINUX_SMOKE` 需可达软件 Vulkan，默认关闭）。默认不在每个
PR 上跑以保持门禁轻量；编译门禁是 `linux-app`。本环境未在 GitHub Actions 上执行该 job，
属 NOT RUN；本机 Linux 可本地运行同一脚本复现。

### `windows-check`（MSVC 编译层，必需 job）

`runs-on: windows-latest`，命令 `cargo check -p app-windows --all-targets --locked`
（原生 `x86_64-pc-windows-msvc`，不需要 MinGW）。

验证 Linux/Windows 共享桌面宿主（`app-linux` + `app-windows`）在真实 MSVC 目标上的
编译。**只证明编译**：不运行 exe，不验证窗口、原生文件对话框、DX12/Vulkan 或真机。
本环境未在 GitHub Actions 上实际执行该 job，属 NOT RUN。

### `windows-release`（MSVC 发布包，按需）

`runs-on: windows-latest`，`if: github.event_name == 'workflow_dispatch' || tags/v*`。
`actions/setup-python` 提供 Python，随后 `bash scripts/package-windows-release.sh`
（MSVC 目标、`+crt-static` 静态链接 CRT、`fetch-fonts.sh` 组装全量字体），上传
`yacr-windows-release` artifact（zip + sha256）。默认不在每个 PR 上跑以保持门禁轻量；
编译门禁是 `windows-check`。本环境未在 GitHub Actions 上实际执行该 job，属 NOT RUN；
Linux 本机可分别以 `x86_64-pc-windows-gnu`（MinGW）或 `CARGO_XWIN=1` 的
`x86_64-pc-windows-msvc`（cargo-xwin）本地复现脚本逻辑。

### `macos-check`（原生 macOS 编译层，必需 job）

`runs-on: macos-latest`，命令 `cargo check -p app-macos --all-targets --locked`
（原生 arm64 macOS，不运行 app）。

验证共享桌面宿主（`app-linux` + `app-macos`）在真实 macOS 目标上的编译。**只证明编译**：
不验证窗口、Metal GPU、原生文件对话框或真机。本环境（Linux，无 macOS runner 与 Apple
SDK）未执行该 job，属 NOT RUN。

### `macos-release`（universal macOS 发布包，按需）

`runs-on: macos-latest`，`if: github.event_name == 'workflow_dispatch' || tags/v*`。
安装 `aarch64-apple-darwin` 与 `x86_64-apple-darwin` target 后执行
`MACOS_ARCH=universal WITH_FONTS=1 bash scripts/package-macos-release.sh`：用 `lipo`
合成通用 Mach-O、组装自包含 `Yacr.app`（`Info.plist` + 字体包）、`otool -L` 断言依赖只来自
系统库，上传 `yacr-macos-release` artifact（tar.gz + sha256）。默认不在每个 PR 上跑以保持
门禁轻量；编译门禁是 `macos-check`。本环境未在 GitHub Actions 上执行该 job，属 NOT RUN；
Linux 主机无法产出 Mach-O，不能本地复现。见 `docs/macos-app.md`。

### `wasm-check`（必需）

1. `cargo check --workspace --lib --target wasm32-unknown-unknown --locked`
   —— **完整 workspace，不排除 UI/宿主**；这正是暴露宿主误耦合（审计 B03）的门禁，
   不能沿用“排除 UI/宿主假装两端覆盖”。
2. `cargo check -p app-web --target wasm32-unknown-unknown --locked`
   —— 单独检查 `app-web`，便于把 web 宿主回归与其它成员区分定位。

依赖 `wasm32-unknown-unknown` target（`rust-toolchain.toml` 已声明）。不需要 secrets。

### `shader-validation`（必需，无需 GPU）

命令：`cargo test -p cad-render-wgpu --test wgsl_validation --locked`。

`crates/cad-render-wgpu/tests/wgsl_validation.rs` 用 `naga`（`wgpu 30.0.1` 内嵌的
同版本，`dev-dependencies` 固定 `=30.0.1`）离线解析并校验 `line.wgsl` /
`mesh.wgsl`，并断言故意写坏的 shader **校验失败**，防止测试退化为 no-op。该测试
**不需要 GPU/显示**，故为真实必需 job，失败即红灯。

它只覆盖**静态 WGSL 校验**：不代表管线可编译、纹理/GPU 集成或真实初始化证据
（那仍属“NOT RUN”，见下）。

### `web-build`（必需）

1. 校验 `wasm-bindgen-cli` pin 与本仓库 `Cargo.lock` 的 `wasm-bindgen` 严格一致
   （运行 `python3` 读 lock 比对，漂移直接失败）；
2. `cargo install wasm-bindgen-cli --version 0.2.129 --locked` 并核对
   `wasm-bindgen --version`；版本不可得即失败；
3. `DIST=$GITHUB_WORKSPACE/web-dist PROFILE=release scripts/build-web.sh`
   （脚本内部 `cargo build ... --locked`，不会改锁文件）。脚本默认
   `WITH_FONTS=1`，经 `fetch-fonts.sh` 把第三方字体下载进 `web-dist/fonts/`
   （jsDelivr 优先；单文件失败按 `FONT_RETRIES` 退避重试，再回退到
   `raw.githubusercontent.com`——GitHub Actions 共享出口 IP 常被 jsDelivr 限流；
   两个源都失败才退出 1。字体不入库，见 `docs/fonts.md`）；
4. 校验 `index.html`/`main.js`/`style.css`/`pkg/yacr.js`/`pkg/yacr_bg.wasm` 及
   `host/{files,i18n,renderer,runtime}.js` 均非空，
   且 JS glue 引用 `yacr_bg.wasm`（wasm/JS 配对）；
5. 断言 `Cargo.lock` 未被改写（`git diff --exit-code`）；
6. 上传 artifact `web-dist`（retention 14 天），并把 wasm 字节数与
   `wasm-bindgen-cli` 版本写入 job summary。

`DIST` 固定为工作区内一次性生成目录（脚本 `rm -rf` 只作用于该目录，见审计 §8 提醒）。

### `web-deploy`（能力相关，默认 SKIP；真实发布）

由仓库变量 `CF_PAGES_DEPLOY_ENABLED == 'true'` 门控，且**仅当** `push` 到 `main`
（`github.event_name == 'push' && github.ref == 'refs/heads/main'`）才运行；PR/fork 永不接触
Cloudflare secrets。未启用或非 main push 时 GitHub 显示 **skipped**，不是绿灯。

`needs: web-build`，只有构建成功才发布（即“构建成功自动发布”）。步骤：

1. `actions/download-artifact@v4` 取回 `web-build` 的 `web-dist` artifact；
2. 读取 `CLOUDFLARE_API_TOKEN`（权限含 *Cloudflare Pages: Edit*）与
   `CLOUDFLARE_ACCOUNT_ID` secrets，缺失则步骤**显式失败**（job 断言与
   `scripts/deploy-cloudflare-pages.sh` 各查一次）；
3. `DIST=$GITHUB_WORKSPACE/web-dist SKIP_BUILD=1 CF_PAGES_PROJECT=yacr-examples
   CF_PAGES_BRANCH=main scripts/deploy-cloudflare-pages.sh`——**不重新构建**，
   发布的就是 `web-build` 已校验的产物；
4. 脚本按需创建/复用 Pages 项目，`npx wrangler@4 pages deploy` 直传并输出生产 URL。

对接的现有生产项目为 `yacr-examples`、生产分支 `main`（`docs/validation-web.md` §11）。
这是审计 §6.3 所说的“构建与发布分开、自动部署需另行确认”的后续，现在由**仓库变量
显式启用**，符合“未支持能力显式建模、不静默成功”。本仓库**尚未在 GitHub Actions 实跑**
该 job（见下 NOT RUN）。

### `web-host-contracts`（必需，无需 wasm/GPU）

`web-host-contracts` 独立使用 Node `22.22.1` 执行
`node --test scripts/test-web-host.mjs`，不受 `WEB_SMOKE_ENABLED` 门控。
它使用宿主 stub，不代替真实 wasm/Playwright 冒烟；模块划分见 `code-structure.md`。

### `android-check`（能力相关，默认 SKIP）

命令：`cargo check --target aarch64-linux-android -p cad-ui-slint -p app-android --locked`。

GitHub 托管 runner **不保证**提供 `docs/build.md` 固定的 JDK 17 /
Android SDK / NDK 27.0.12077973。因此该 job 默认被门控：

- 仅当仓库变量 `ANDROID_CI_ENABLED == 'true'` 时运行；
- `runs-on: ${{ vars.ANDROID_RUNNER_LABEL || 'ubuntu-latest' }}`，指向已配置的
  runner/标签（如 `android-ci`）；
- 运行时会先执行 “Require pinned Android toolchain” 步骤，缺 `ANDROID_HOME` 或
  `JAVA_HOME` 时**显式失败**并指向 `docs/build.md`，不会静默通过。

默认未启用时，GitHub 将该 job 显示为 **skipped（未运行）**，绝不显示为通过。
维护者须先确认 runner/标签/SDK/NDK 版本，再打开该变量。

### `android-apk`（能力相关，默认 SKIP；打包 ≠ 安装运行）

与 `android-check` 同一 `ANDROID_CI_ENABLED` 开关。启用后：

1. 校验 `ANDROID_HOME` / `JAVA_HOME` 存在，且 `java -version` 主版本为 **17**、
   NDK 目录与 build-tools `34.0.0/aapt2` 存在，否则**失败**；
2. `cargo install cargo-apk --version 0.10.0 --locked`；
3. `scripts/build-android.sh --release`（脚本内两个 `cargo check` 已带 `--locked`；
   `cargo apk build` 无 `--locked` 参数，故随后单独断言 `git diff --exit-code Cargo.lock`
   未变，锁文件被改写即失败）；
4. 用 `aapt2 dump badging` 读取**真实** manifest，记录 package / minSdk / targetSdk /
   ABI / 签名类型，写入 job summary；与锁定期望不符即失败；
5. 上传 artifact `android-apk`。

**实测事实**（与 `docs/build.md` 一致）：`package=dev.yacr.app`、`minSdk=23`
（metadata 写的 26 **未生效**）、`targetSdk=30`（metadata 写的 34 **未生效**）、
ABI `arm64-v8a`、v1 debug/dev 密钥 JAR 签名。**打包等于产出可安装 APK 文件，
不等于安装运行**；无设备/模拟器，安装与启动一律未验证。

### `android-release`（能力相关，默认 SKIP；按需/tag 含字体 APK）

与 `android-apk` 同一 `ANDROID_CI_ENABLED` 开关，但仅在 `workflow_dispatch` 或 `v*` tag 触发
（`android-apk` 是每次 push/PR 的构建+事实记录，`android-release` 是发布打包）。启用后：
先执行与 `android-apk` 相同的固定工具链前置校验；`cargo install cargo-apk 0.10.0`；生成
**gitignored** 的开发签名 keystore（CI 专用，与 `docs/build.md` 一致）；`scripts/fetch-fonts.sh
apps/app-android/assets/fonts` 把 mlightcad 全量目录 + 已提交 osifont 打进 `assets/fonts/`；
`scripts/build-android.sh --release`；断言 `Cargo.lock` 未被改写；用 `aapt2 dump badging`
记录真实 manifest，上传 `yacr-android-release` artifact（APK）。**打包不等于安装运行**；默认
未启用时 GitHub 显示 **skipped**，绝不显示为通过。本环境未在 GitHub Actions 上执行该 job，
属 NOT RUN。

### `web-smoke`（能力相关，默认 SKIP；仅软件 GPU）

由仓库变量 `WEB_SMOKE_ENABLED == 'true'` 门控。启用后：安装锁定
Node `22.22.1` 与 Playwright `1.55.1` Chromium，构建 `web-dist`，起
`scripts/serve-web.py`，再跑 `scripts/check-web-ui.mjs`，上传截图/JSON/日志。

- 走 SWIFTSHADER 软件 WebGL2 路径，**明确不代表**桌面/移动真机或 WebGPU 验收；
- WebGPU 能力缺失时应如实报告未运行，不得记为通过；
- 依赖审计 B29 的修复，未修复前其结论视为不完整。

## 需要的 secrets / 权限

- 除 `web-deploy` 外：无 secrets、无写权限（artifact 上传用 `actions/upload-artifact`，
  普通 `permissions: contents: read` 即可）。
- `web-deploy`（真实发布，需显式启用）：两个 **secret**
  - `CLOUDFLARE_API_TOKEN`：至少含 *Cloudflare Pages: Edit*；如需自动挂自定义域名再含
    *Zone: DNS: Edit*；
  - `CLOUDFLARE_ACCOUNT_ID`：Cloudflare account id。
  - 另需仓库 **variable** `CF_PAGES_DEPLOY_ENABLED=true` 才会运行；缺 secret 时 job
    显式失败，绝不静默成功。
- 如需启用 Android 层：只需仓库变量 `ANDROID_CI_ENABLED`（`true`）与
  `ANDROID_RUNNER_LABEL`；web 冒烟只需 `WEB_SMOKE_ENABLED`。这些是 **variable**，
  不是 secret。签名密钥一律不注入到普通构建 job（CI APK 用临时开发签名）。

## 本环境 / 本层 NOT RUN（明确未运行）

以下内容**未实现或未运行**，不得据本工作流宣称通过；需要维护者先确认
runner、标签、secrets 与设备：

- `android-check` / `android-apk` / `android-release`：默认关闭，无 Android SDK/NDK 的
  runner 上未运行；启用后仍**只**产出 APK，**安装与真机运行未执行**。
- `web-smoke`：默认关闭。即便启用，也只是软件 GPU 无头 Chromium，**非真机、非
  WebGPU 验收**；真机浏览器矩阵未运行。
- `web-deploy`：默认关闭（`CF_PAGES_DEPLOY_ENABLED` 未设即 SKIP），**未在本仓库的
  GitHub Actions 上跑过**；`docs/validation-web.md` §11 的记录是**本机手工**执行
  `scripts/deploy-cloudflare-pages.sh`，不构成本 workflow 的通过证据。首次启用后须以真实
  job URL / 部署 id 回填 `docs/validation.md`。
- **真实 GPU / WebGPU 初始化与黄金图**：未运行（`shader-validation` 只是离线解析）。
- **Android 真机 / 模拟器交互、移动性能报告**：未运行。
- **自托管 GPU / Android 真机 runner**：本轮未注册、未购买、未验证标签与权限。
- `core-quality` / `wasm-check` / `i18n-contracts` / `shader-validation` /
  `web-build` / `web-host-contracts`：workflow 已定义；`shader-validation` 与
  `web-build` 的命令已在本环境本地实跑（见下），但**未在 GitHub Actions 上执行**，
  请以真实 job 结果为准并写入 `docs/validation.md`。
- `linux-release` / `windows-check` / `windows-release` / `macos-check` / `macos-release`：
  workflow 已定义，但**未在本环境的 GitHub Actions 上执行**；`linux-release` 可在本机 Linux
  运行同一脚本复现，`windows-*` 可交叉复现，`macos-*` 无法（Linux 主机无 Apple SDK，
  产不出 Mach-O），只能由 macOS runner/真机产出。

维护者待确认项：是否启用 web 自动发布 `web-deploy`（设置 `CF_PAGES_DEPLOY_ENABLED`
与 `CLOUDFLARE_API_TOKEN` / `CLOUDFLARE_ACCOUNT_ID`）、是否启用 Android/web-smoke 层
（`ANDROID_CI_ENABLED` / `ANDROID_RUNNER_LABEL` / `WEB_SMOKE_ENABLED`）、是否注册自托管
GPU/真机 runner（标签、镜像、费用、权限）、CI APK 的临时签名策略。

## 本地复现

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo fmt --all -- --check
cargo clippy --workspace --exclude app-android --exclude app-web \
  --all-targets --locked -- -D warnings
python3 scripts/check-architecture.py
cargo check --workspace --exclude app-android --exclude app-web --all-targets --locked
cargo check --workspace --lib --target wasm32-unknown-unknown --locked
cargo check -p app-web --target wasm32-unknown-unknown --locked
# 以下是附加 CI/发布层，不属于默认本地 debug 提交门禁。
# 离线 WGSL 校验（无需 GPU）
cargo test -p cad-render-wgpu --test wgsl_validation --locked
# Web 静态产物（需 wasm-bindgen-cli 0.2.129）
cargo install wasm-bindgen-cli --version 0.2.129 --locked
DIST="$PWD/web-dist" PROFILE=release scripts/build-web.sh
# Web 自动发布到 Cloudflare Pages（需 CLOUDFLARE_API_TOKEN / CLOUDFLARE_ACCOUNT_ID；
# SKIP_BUILD=1 表示发布上面已构建/校验的产物，不重建）
DIST="$PWD/web-dist" SKIP_BUILD=1 CF_PAGES_PROJECT=yacr-examples CF_PAGES_BRANCH=main \
  scripts/deploy-cloudflare-pages.sh
# Android（需 SDK/JDK/NDK，见 docs/build.md）
cargo check --target aarch64-linux-android -p cad-ui-slint -p app-android --locked
scripts/build-android.sh --release
```

工作流结构自检（纯 Python 标准库）：

```bash
python3 scripts/check-workflows.py
python3 scripts/test-linux-workflow.py
python3 scripts/test-web-deploy-workflow.py
python3 scripts/test-package-linux-release.py
python3 scripts/test-package-macos-release.py
python3 scripts/test-fetch-fonts.py
```

它校验：workflow 文件存在且非空；十七个必需 job（`core-quality`、`wasm-check`、
`i18n-contracts`、`shader-validation`、`linux-app`、`linux-release`、`windows-check`、
`windows-release`、`macos-check`、`macos-release`、`android-release`、`web-build`、
`web-host-contracts`、`web-deploy`、`android-check`、`android-apk`、`web-smoke`）均已声明；
必需 job 内没有 `continue-on-error: true`；每个 job 保留其命令片段；且被门控的 job
（`web-deploy`/`android-check`/`android-apk`/`android-release`/`web-smoke`）保留其 `if:`
能力开关（缺开关即失败，防止门控被误当成静默通过）。有 PyYAML 时做真实解析，
否则退化为结构化文本检查并如实说明（不假装 YAML 已解析）。失败退出码非零。

`scripts/test-linux-workflow.py` 与 `scripts/test-web-deploy-workflow.py` 是**变异契约**
（mutation contracts）：它们故意删除主 Linux job 的产物/失败策略、或删除 `web-deploy`
的能力开关 / `download-artifact` / 真实部署命令，断言结构检查**必然报错**，从而证明
上面的门禁不是摆设。两者都不触网、不部署。

`scripts/test-package-linux-release.py` 是发布打包脚本的静态 + 变异契约：断言
`scripts/package-linux-release.sh` 仍构建并打包 CLI 与 GUI 两个二进制、把 GUI 非基础系统库
复制进 `lib/` 并带 `$ORIGIN/../lib` RPATH、且保留「无缺失库 / RPATH 生效 / GUI 可加载」的
打包内校验；逐条删除任一保证都会报错。它只检查脚本文本，**不**执行 release 构建，也不构成
已打包或已渲染的证据。

`scripts/test-package-macos-release.py` 同理，断言 `scripts/package-macos-release.sh` 仍拒绝
在非 macOS 运行、构建 CLI 与 GUI 两个 Mach-O、用 `lipo` 合成 arm64+x86_64、拒绝非系统
动态依赖、组装自包含 `Yacr.app` 与字体包并产出 tar.gz + sha256；它同样只检查脚本文本，
**不**执行构建，也不构成已打包或在真实 Mac 上可运行的证据。

`scripts/test-fetch-fonts.py` 是字体打包脚本的静态 + 变异契约：断言 `fetch-fonts.sh` 以 UTF-8
读写 `fonts.json`（此前 Windows 文本模式默认 cp1252 会导致 `UnicodeDecodeError`）、字体下载
失败仍 `sys.exit(1)`、保留回退源与已提交默认字体合并；只检查脚本文本，不联网、不下载。

## 启用 web 自动发布（维护者操作）

1. 仓库 Secrets 增加 `CLOUDFLARE_API_TOKEN`（Pages: Edit；需自动挂域名再加 DNS: Edit）
   与 `CLOUDFLARE_ACCOUNT_ID`；
2. 仓库 Variables 设 `CF_PAGES_DEPLOY_ENABLED = true`；
3. 此后每次 push 到 `main` 且 `web-build` 成功，`web-deploy` 会把该次构建的
   `web-dist` artifact 发布到 `yacr-examples`；Cloudflare 与 job summary 均给出
   不可变部署 URL 与生产 URL。

关闭自动发布只需把变量改回非 `true`（job 回到 SKIP，不删任何配置）。
