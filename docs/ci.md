# CI（GitHub Actions）分层工作流

本文件描述仓库**当前**实际提供的 CI 运行层。命令权威仍是 `AGENTS.md`「构建与门禁」与
`docs/build.md`；本文件只说明 workflow 如何镜像该门禁，以及哪些层次**已退役/未运行**。

工作流文件：`.github/workflows/core.yml`（核心质量层）与 `.github/workflows/build.yml`
（构建/校验层）。校验脚本：`scripts/check-workflows.py`（必需 job 片段 + 退役 job 硬禁用契约）、
`scripts/test-linux-workflow.py`（变异契约）、`scripts/test-ui-migration-guards.py`（迁移守卫契约）。

> **2026-10-16 迁移状态**：桌面已迁移到 React/Tauri（`apps/app-linux`、`yacr`），Slint 退役。
> 编译门禁已加入 Tauri/GTK 前置（`libwebkit2gtk-4.1-dev`）与 React 前端构建（`apps/app-linux/ui`、
> `npm test`、`npm run build`）。8 个发布/Web job 以 `if: false` **硬禁用**并以 `RETIRED` 标记，
> **不可通过仓库变量重新启用**，也不含 `secrets.…` 发布路径。见 ADR 0010。

> **本环境（无 GitHub runner、无 `gh` 权限）没有实际执行这些 job**；本文件描述的是 workflow 内容
> 与预期，**不是通过证据**。真实 job URL/commit/结果应写入 `docs/validation*.md`。

## 触发、权限与并发

- 触发：`push`、`pull_request`、`workflow_dispatch`。
- 权限：顶层 `permissions: contents: read`；**没有 job 拥有外部发布 secrets**（退役的
  Cloudflare 发布路径已删除）。
- 并发：`concurrency.group = <workflow>-<ref>`，`cancel-in-progress: true`。

## 必需的 job（genuine gates）

`check-workflows.py` 要求下列 job 必须存在且包含各自命令片段，缺失即失败：

| Job | Workflow | Runner | 门禁 |
|---|---|---|---|
| `core-quality` | core.yml | `ubuntu-latest` | fmt + clippy(exclude android/web) + 架构/fixture/workflow/迁移契约 + **代码文件 ≤1000 行**（`check-file-size.py`）+ 全目标 debug 编译；并安装 GTK/WebKitGTK 前置 + 构建 React 前端（`npm test`/`npm run build`） |
| `wasm-check` | core.yml | `ubuntu-latest` | 全 workspace wasm `--lib` 检查 + 单独 `app-web` 检查 |
| `i18n-contracts` | core.yml | `ubuntu-latest` | `python3 scripts/check-i18n.py`（双语目录 / key / 占位符 / Rust + React 静态 key / 硬编码 CJK 扫描） |
| `shader-validation` | build.yml | `ubuntu-latest` | 离线 WGSL 解析/校验（naga，无需 GPU），失败红灯 |
| `linux-app` | build.yml | `ubuntu-latest` | 安装 GTK/WebKitGTK 前置 + 构建并测试 React 前端 + `cargo check -p app-linux --all-targets --locked`，上传日志 |
| `windows-check` | build.yml | `windows-latest` | 原生 MSVC `cargo check -p app-windows --all-targets --locked`（占位桩，不运行） |
| `macos-check` | build.yml | `macos-latest` | 原生 arm64 `cargo check -p app-macos --all-targets --locked`（占位桩，不运行） |
| `android-check` | core.yml | 能力相关（默认 SKIP） | `cargo check --target aarch64-linux-android -p app-android --locked`（占位桩） |

`android-check` 是**唯一** capability-gated 的编译门禁（`vars.ANDROID_CI_ENABLED`）；默认未启用时
GitHub 显示 **skipped（未运行）**，绝不显示为通过。`windows-check`/`macos-check` **只证明编译**，
不代表这些平台可运行。

## 已退役的 job（硬禁用，`if: false` + `RETIRED`）

下列 job **保留声明**、带无条件 `if: false` 与 `RETIRED` 注释，并含一个 `exit 1` 步骤；它们
**不含** `vars.…` 开关、`secrets.…` 引用或 `continue-on-error: true`。删除禁用守卫、改成仓库变量
或重新加入凭据都会被 `check-workflows.py` 判为硬错误：

`web-build`、`web-deploy`、`web-smoke`、`android-apk`、`android-release`、`linux-release`、
`windows-release`、`macos-release`。

含义：浏览器 UI / Android UI / Windows / macOS GUI 均已退役为显式 NOT IMPLEMENTED 占位。
`linux-release` 的**脚本**已于 2026-10-17 重启（本地可构建，见 `docs/flatpak.md`），但该 CI job 仍
**硬禁用**；它与其余 job 一样不会产出或发布任何产物。

## 锁定工具版本

`build.yml` 顶部 `env:` 与 `docs/build.md`、`Cargo.lock` 对齐：

| 工具 | 锁定值 | 来源 |
|---|---|---|
| Rust | `1.99.0` | `rust-toolchain.toml` |
| Node | 见 workflow `NODE_VERSION` | React 前端构建/测试 |

退役的 wasm-bindgen / Android / Playwright 环境钉住项已随对应 job 删除。

## 本层 NOT RUN（明确未运行）

- `core-quality` / `wasm-check` / `i18n-contracts` / `shader-validation` / `linux-app` /
  `windows-check` / `macos-check` / `android-check`：workflow 已定义，但**未在本环境的
  GitHub Actions 上执行**；请以真实 job 结果为准并写入 `docs/validation*.md`。
- 8 个退役 job：**永远不会**在受支持配置下运行（硬禁用）。
- **CI 上的真实 GPU / 黄金图 / 浏览器矩阵 / Android 真机**：未运行。GitHub 托管 runner 无 GPU，
  软件渲染不能作为真实 GPU 证据。
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
cargo check --workspace --all-targets --locked
cargo check --workspace --lib --target wasm32-unknown-unknown --locked
# React 前端
(cd apps/app-linux/ui && npm ci && npm test && npm run build)
# 工作流/打包/迁移契约
python3 scripts/test-linux-workflow.py
python3 scripts/test-ui-migration-guards.py
python3 scripts/test-web-deploy-workflow.py
python3 scripts/test-package-linux-release.py
python3 scripts/test-package-windows-release.py
python3 scripts/test-package-macos-release.py
python3 scripts/test-fetch-fonts.py
python3 scripts/test-android-icon.py
# 离线 WGSL 校验（无需 GPU）
cargo test -p cad-render-wgpu --test wgsl_validation --locked
```

退役脚本的负向断言（每个应非零退出）：

```bash
for s in build-web deploy-cloudflare-pages build-android \
         package-windows-release package-macos-release; do
  bash "scripts/$s.sh"; echo "$s exit=$? (expect 1)"
done
```

`package-linux-release.sh` **不在**该列表：它已于 2026-10-17 重启，按退出码 0 完成构建（本地产物，
无发布，见 `docs/flatpak.md`）。

`scripts/check-workflows.py` 有 PyYAML 时做真实解析，否则退化为结构化文本检查并如实说明；
变异契约（`test-linux-workflow.py`、`test-web-deploy-workflow.py`、`test-package-*.py`）故意删除
关键片段并断言检查**必然报错**，证明门禁不是摆设。它们都不触网、不部署、不执行 release。
