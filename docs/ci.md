# CI（GitHub Actions）分层工作流

本文件描述仓库实际提供的 CI 运行层（审计 N02 §6）。命令权威仍是 `AGENTS.md`
“构建”一节与 `docs/build.md`；本文件只说明 workflow 如何镜像该门禁，以及哪些层次
**尚未运行**。

工作流文件：`.github/workflows/core.yml`。校验脚本：`scripts/check-workflows.py`。

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
| `actions/cache` | `@v4` | 本层新增的唯一 action；与 `actions/checkout@v4` 相同的 major tag 风格 |

`actions/cache` 未固定完整 commit SHA：编写时无法联网核验 SHA，禁止凭空编造。
仓库若采用 SHA 固定策略，应替换为已核验的完整 SHA（§6.3）。

## 已实现的 job

| Job | Runner | 门禁 |
|---|---|---|
| `core-quality` | `ubuntu-latest` | 核心测试 + fmt + clippy + 架构边界，失败即红灯 |
| `wasm-check` | `ubuntu-latest` | 完整 workspace wasm `--lib` 检查 + 单独 `app-web` 检查 |
| `android-check` | 默认关闭（能力相关） | aarch64 上 `cad-ui-slint` + `app-android` 的 `cargo check` |

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

### `android-check`（能力相关，默认关闭）

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

## 需要的 secrets / 权限

- 本工作流：无 secrets、无写权限。构建 APK / 发布 / 部署均不在本层。
- 如需启用 `android-check`：只需仓库变量 `ANDROID_CI_ENABLED`（`true`）与
  `ANDROID_RUNNER_LABEL`；这些是 **variable**，不是 secret。签名密钥一律
  不注入到普通构建 job。

## 本环境 / 本层 NOT RUN（明确未运行）

以下 N02 §6.2 层次**未实现或未运行**，不得据本工作流宣称通过；需要维护者先确认
runner、标签、secrets 与发布目标：

- `android-check`（默认关闭，见上；无 Android SDK/NDK 的 runner 上未运行）。
- `android-apk`：不产出/上传 APK；打包与安装运行均未执行，需要 CI 临时开发签名策略。
- `web-build`：不运行 `scripts/build-web.sh`，不上传 `web-dist`；需要锁定
  `wasm-bindgen-cli`（`docs/build.md` 固定 0.2.129）与产物目录预算。
- `web-smoke`：不运行浏览器/Playwright；需要锁定 Node/Playwright/浏览器版本与软件 GPU，
  且依赖审计 B29 的修复。
- `i18n-contracts`：双语 catalog / 缺 key / 硬编码白名单校验未实现（N01 未完成，
  当前仅 `zh-CN`），不能以“构建通过”代替。
- `shader-validation`：WGSL 离线校验与真实 GPU 初始化证据均未运行。
- **自托管 GPU / Android 真机 runner**：本轮未注册、未购买、未验证标签与权限；
  真机安装、真实 GPU/WebGPU 黄金图与性能验收全部**未运行**。
- `core-quality` / `wasm-check`：workflow 已定义，但本环境无 GitHub runner，
  **未在 Actions 上实际执行**；请以真实 job 结果为准并写入 `docs/validation.md`。

维护者待确认项：是否启用自托管/Android runner（标签、镜像、费用、权限）、
是否启用发布/部署、CI APK 的临时签名方式、Node/Playwright 锁定版本。

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
# Android（需 SDK/JDK/NDK，见 docs/build.md）
cargo check --target aarch64-linux-android -p cad-ui-slint -p app-android --locked
```

工作流结构自检（本层新增，纯 Python 标准库）：

```bash
python3 scripts/check-workflows.py
```

它校验：workflow 文件存在且非空、三个必需 job 均已声明、必需 job 内没有
`continue-on-error: true`、并保留各自的命令片段。有 PyYAML 时做真实解析，否则
退化为结构化文本检查并如实说明（不假装 YAML 已解析）。失败退出码非零。
