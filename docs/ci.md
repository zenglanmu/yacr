# CI（GitHub Actions）分层工作流

本文件描述仓库实际提供的 CI 运行层（审计 N02 §6）。命令权威仍是 `AGENTS.md`
“构建”一节与 `docs/build.md`；本文件只说明 workflow 如何镜像该门禁，以及哪些层次
**尚未运行**。

工作流文件：`.github/workflows/core.yml`（核心质量层）与
`.github/workflows/build.yml`（N02 构建/校验层）。校验脚本：`scripts/check-workflows.py`。

> 本环境（无 GitHub runner、无 `gh` 权限）**没有实际执行**这些 job；本文件描述的
> 是 workflow 内容与预期，不是通过证据。真实 job URL / commit / 结果应写入
> `docs/validation.md`（由控制器汇总，本文件不代填）。

## 触发、权限与并发

- 触发：`push`、`pull_request`、`workflow_dispatch`（可手动重跑单层）。
- 权限：顶层 `permissions: contents: read`；本工作流无发布/上传步骤，不需要任何
  write 权限或 secrets。
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
| `dtolnay/rust-toolchain` | `@1.98.1` | 与 `rust-toolchain.toml` 的 channel 一致，沿用仓库原有 pin |
| `actions/cache` | `@v4` | 本层新增的唯一缓存 action；与 `actions/checkout@v4` 相同的 major tag 风格 |
| `actions/upload-artifact` | `@v4` | `web-build` / `android-apk` / `web-smoke` 上传产物，只读权限即可 |
| `actions/setup-node` | `@v4` | `web-smoke` 安装锁定 Node |

这些 action 未固定完整 commit SHA：编写时无法联网核验 SHA，禁止凭空编造。
仓库若采用 SHA 固定策略，应替换为已核验的完整 SHA（§6.3）。

## 已实现的 job

| Job | Workflow | Runner | 门禁 |
|---|---|---|---|
| `core-quality` | core.yml | `ubuntu-latest` | 核心测试 + fmt + clippy + 架构边界，失败即红灯 |
| `wasm-check` | core.yml | `ubuntu-latest` | 完整 workspace wasm `--lib` 检查 + 单独 `app-web` 检查 |
| `i18n-contracts` | core.yml | `ubuntu-latest` | 双语 catalog / 缺 key / 硬编码白名单校验 |
| `shader-validation` | build.yml | `ubuntu-latest` | naga 离线 WGSL 解析/校验（无需 GPU），失败红灯 |
| `web-build` | build.yml | `ubuntu-latest` | `scripts/build-web.sh` 产出 `web-dist` 并上传，校验 wasm/JS 配对 |
| `web-host-contracts` | build.yml | `ubuntu-latest` | Node 内置测试：模块边界、未保存决策、导出确认、本地化及轮询（无需 wasm/GPU） |
| `android-check` | core.yml | 能力相关（默认 SKIP） | aarch64 上 `cad-ui-slint` + `app-android` 的 `cargo check` |
| `android-apk` | build.yml | 能力相关（默认 SKIP） | 产出/上传 APK，记录真实 manifest facts（不宣称安装运行） |
| `web-smoke` | build.yml | 能力相关（默认 SKIP） | 软件 GPU 无头 Chromium 冒烟（非真机、非 WebGPU 验收） |

### 锁定工具版本

`build.yml` 顶部 `env:` 与 `docs/build.md`、`Cargo.lock` 严格对齐；每个 job 运行时
**再次核验**，漂移即失败（绝不静默改锁文件）：

| 工具 | 锁定值 | 对齐来源 |
|---|---|---|
| Rust | `1.98.1` | `rust-toolchain.toml` |
| `wasm-bindgen-cli` | `0.2.129` | `Cargo.lock` 的 `wasm-bindgen`（`web-build` 用脚本读取 lock 并比对，不一致直接失败） |
| `cargo-apk` | `0.10.0` | `docs/build.md` |
| Android NDK | `27.0.12077973` | `docs/build.md` |
| Android build-tools | `34.0.0` | `docs/build.md` |
| Android platform | `android-34` | `docs/build.md` |
| JDK | `17` | `docs/build.md`（运行时校验 `java -version` 主版本） |
| 预期 targetSdk / ABI / minSdk | `30` / `arm64-v8a` / `23` | 实测 manifest（`min_sdk_version` metadata 未生效，`docs/build.md` 已记录） |
| Node | `22.22.1` | `web-smoke` |
| Playwright | `1.55.1` | `web-smoke`（npm 锁定版本，不使用个人全局安装） |


### `core-quality`（必需）

镜像 `AGENTS.md` 的纯核心门禁，步骤顺序：

1. `cargo fmt --all -- --check`
2. `cargo clippy --workspace --exclude cad-ui-slint --exclude app-android --exclude app-web --all-targets --locked -- -D warnings`
3. `python3 scripts/check-architecture.py`
4. `cargo test --workspace --exclude cad-ui-slint --exclude app-android --exclude app-web --locked --no-fail-fast`

排除 `cad-ui-slint` 的原因见 `docs/build.md`：Linux 宿主缺 fontconfig/freetype
开发头；其编译由 Android target 检查覆盖。该 job 不设 `continue-on-error`，
失败必须红灯。

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
   `WITH_FONTS=1`，经 `fetch-web-fonts.sh` 把第三方字体下载进 `web-dist/fonts/`
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

### `web-smoke`（能力相关，默认 SKIP；仅软件 GPU）

由仓库变量 `WEB_SMOKE_ENABLED == 'true'` 门控。启用后：安装锁定
Node `22.22.1` 与 Playwright `1.55.1` Chromium，构建 `web-dist`，起
`scripts/serve-web.py`，再跑 `scripts/check-web-ui.mjs`，上传截图/JSON/日志。

- 走 SWIFTSHADER 软件 WebGL2 路径，**明确不代表**桌面/移动真机或 WebGPU 验收；
- WebGPU 能力缺失时应如实报告未运行，不得记为通过；
- 依赖审计 B29 的修复，未修复前其结论视为不完整。

## 需要的 secrets / 权限

- 本工作流：无 secrets、无写权限（artifact 上传用 `actions/upload-artifact`，
  普通 `permissions: contents: read` 即可）。发布/部署均不在本层。
- 如需启用 Android 层：只需仓库变量 `ANDROID_CI_ENABLED`（`true`）与
  `ANDROID_RUNNER_LABEL`；web 冒烟只需 `WEB_SMOKE_ENABLED`。这些是 **variable**，
  不是 secret。签名密钥一律不注入到普通构建 job（CI APK 用临时开发签名）。

## 本环境 / 本层 NOT RUN（明确未运行）

以下内容**未实现或未运行**，不得据本工作流宣称通过；需要维护者先确认
runner、标签、secrets 与设备：

- `android-check` / `android-apk`：默认关闭，无 Android SDK/NDK 的 runner 上未运行；
  启用后仍**只**产出 APK，**安装与真机运行未执行**。
- `web-smoke`：默认关闭。即便启用，也只是软件 GPU 无头 Chromium，**非真机、非
  WebGPU 验收**；真机浏览器矩阵未运行。
- **真实 GPU / WebGPU 初始化与黄金图**：未运行（`shader-validation` 只是离线解析）。
- **Android 真机 / 模拟器交互、移动性能报告**：未运行。
- **自托管 GPU / Android 真机 runner**：本轮未注册、未购买、未验证标签与权限。
- `core-quality` / `wasm-check` / `i18n-contracts` / `shader-validation` /
  `web-build`：workflow 已定义；`shader-validation` 与 `web-build` 的命令已在本环境
  本地实跑（见下），但**未在 GitHub Actions 上执行**，请以真实 job 结果为准并写入
  `docs/validation.md`。

维护者待确认项：是否启用 Android/web-smoke 层（`ANDROID_CI_ENABLED` /
`ANDROID_RUNNER_LABEL` / `WEB_SMOKE_ENABLED`）、是否注册自托管 GPU/真机 runner
（标签、镜像、费用、权限）、是否启用发布/部署、CI APK 的临时签名策略。

## 本地复现

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo fmt --all -- --check
cargo clippy --workspace --exclude cad-ui-slint --exclude app-android --exclude app-web \
  --all-targets --locked -- -D warnings
python3 scripts/check-architecture.py
cargo test --workspace --exclude cad-ui-slint --exclude app-android --exclude app-web \
  --locked --no-fail-fast
cargo check --workspace --lib --target wasm32-unknown-unknown --locked
cargo check -p app-web --target wasm32-unknown-unknown --locked
# 离线 WGSL 校验（无需 GPU）
cargo test -p cad-render-wgpu --test wgsl_validation --locked
# Web 静态产物（需 wasm-bindgen-cli 0.2.129）
cargo install wasm-bindgen-cli --version 0.2.129 --locked
DIST="$PWD/web-dist" PROFILE=release scripts/build-web.sh
# Android（需 SDK/JDK/NDK，见 docs/build.md）
cargo check --target aarch64-linux-android -p cad-ui-slint -p app-android --locked
scripts/build-android.sh --release
```

工作流结构自检（纯 Python 标准库）：

```bash
python3 scripts/check-workflows.py
```

它校验：workflow 文件存在且非空；八个必需 job（`core-quality`、`wasm-check`、
`i18n-contracts`、`shader-validation`、`web-build`、`android-check`、`android-apk`、
`web-smoke`）均已声明；必需 job 内没有 `continue-on-error: true`；每个 job 保留其
命令片段；且被门控的 job（`android-check`/`android-apk`/`web-smoke`）保留其
`if:` 能力开关（缺开关即失败，防止门控被误当成静默通过）。有 PyYAML 时做真实解析，
否则退化为结构化文本检查并如实说明（不假装 YAML 已解析）。失败退出码非零。
