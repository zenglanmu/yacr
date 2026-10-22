# 构建与平台入口

工具链 1.99.0（`rust-toolchain.toml`）、`Cargo.lock` 已锁定。若 `PATH` 无 cargo，加入
`$HOME/.cargo/bin`。

> **2026-10-16 迁移状态（2026-10-23 增补）**：桌面 UI 已由 Slint 迁移为 **React + Tauri 原生宿主**
> （`apps/app-linux`，二进制 `yacr`）。`cad-ui-slint` 与所有 Slint 依赖已删除；macOS/Android/Web
> 当前是**显式 NOT IMPLEMENTED 占位**，**不是可运行 GUI 产品**；旧的平台发布/打包脚本已退役或禁用。
> **2026-10-23 起**：Linux 只支持**原生 Wayland**（X11/XWayland 显式 Unsupported，退出码 4）；
> 合成方向是 **DOM-on-top**（[ADR 0023](adr/0023-dom-on-top-cross-host-composition.md)）；
> **Windows 宿主已实现**（`apps/app-windows`，Tier 0 编译 + Wine 拒绝 smoke，视觉未验证）。
> 见 ADR 0010、`docs/architecture.md`、`docs/react-linux-app.md`、`docs/windows-app.md`。

## 验证范围（2026-10-15 用户决定，提效）

日常功能开发与修复**只验证自己的改动范围**（改动 crate 的编译、自己的契约测试、自己文件的
fmt/clippy）。全量门禁与全量测试只在两种情形运行：① 用户明确要求；② 发版/发布验收前。
证据诚实规则不变：未运行必须标注 NOT RUN，不得以旧证据冒充。

**注意**：这只是日常提效策略，**不是永久免测**。本次 Slint→React 迁移的最终验证
（编译、全量测试、门禁、真实 GPU）**尚未运行**，由编排者在 **P6** 统一执行并记录。

## 核心门禁（CI 同款；用户要求/发版前本地运行）

```bash
cargo fmt --all -- --check
cargo clippy --workspace --exclude app-android --exclude app-web --all-targets --locked -- -D warnings
python3 scripts/check-architecture.py
python3 scripts/check-fixture-manifest.py
python3 scripts/check-workflows.py
python3 scripts/check-i18n.py
python3 scripts/check-file-size.py   # 代码文件 ≤1000 行（.md 不计入）
cargo check --workspace --all-targets --locked
cargo check --workspace --lib --target wasm32-unknown-unknown --locked   # 全 workspace，含 UI/宿主
(cd apps/desktop-ui && npm ci && npm test && npm run build)
```

默认 Linux host workspace 编译包含 `app-android` 与 `app-web` 的 host target；这只是编译，
**不是**这些平台的运行验收。WASM 门禁验证平台隔离（非 Linux 目标上 `app-linux`/`app-windows`
编译为空库）。

单元/契约测试（改动范围或发版前）：

```bash
cargo test -p cad-ui-core -p cad-desktop-shell -p app-linux --lib --locked
cargo test -p cad-engine -p cad-host -p cad-app --lib --locked
(cd apps/desktop-ui && npm ci && npm test && npm run build)
```

## 退出码注册表（全宿主同源）

证据脚本以**稳定 stderr 标记行**为第一分类依据，码值仅辅助；**不得**用退出码 0 或非空截图单独
证明视觉正确。

| 码 | 含义 | 产点 | 标记行 |
|---|---|---|---|
| 0 | 成功（含正常退出、生命周期驱动自然结束） | `app-linux/entry.rs`、`app-windows/entry.rs` | — |
| 1 | 通用失败（Tauri 构建失败、启动合同拒绝、Windows 窗口/WebView 构建失败） | `entry.rs`、`main.rs` | — |
| 2 | 无效/不可用的显示后端策略 | `app-linux/src/entry.rs`（`BackendRefusal::Invalid`） | — |
| 3 | postload-pan 证据驱动失败 | `app-linux/src/host/postload.rs` | — |
| 4 | **不支持的后端（X11/XWayland）** | `app-linux/src/backend.rs` | `yacr: unsupported-backend: x11` |
| 5 | **不支持的运行时（WebView2 缺失）** | `app-windows/src/support.rs` | `yacr: unsupported-runtime: webview2` |

## Linux 桌面宿主（当前唯一可运行 GUI）

应用 `apps/app-linux`（二进制 `yacr`；Tauri 标识 `dev.yacr.app`）。

**系统前置**（脚本不自动安装）：WebKitGTK 4.1 开发包（`libwebkit2gtk-4.1-dev`）、
GTK3 开发包（`libgtk-3-dev`）、**Wayland 客户端开发包（`libwayland-dev`，提供
`wayland-client.pc`，用于编译原生 Wayland 私有 C shim）**、Node + npm、Vulkan 驱动；
`pkg-config --exists webkit2gtk-4.1 gtk+-3.0 wayland-client` 应通过。

```bash
sudo apt-get install -y pkgconf libgtk-3-dev libwebkit2gtk-4.1-dev libwayland-dev \
  libfontconfig-dev libfreetype-dev mesa-vulkan-drivers

# 推荐入口：构建前端并以 custom-protocol 运行（Auto 解析到原生 Wayland）
bash scripts/run-react-linux.sh --open /absolute/drawing.dwg

# 或直接跑（先自行 npm run build 出 apps/desktop-ui/dist）
cargo run --release --locked -p app-linux --bin yacr \
  --features tauri/custom-protocol -- --open /absolute/drawing.dwg
```

**后端**：由宿主 `apps/app-linux/src/backend.rs` 在 GTK 启动前显式决议 `GDK_BACKEND`：
未设时 `WAYLAND_DISPLAY` + `XDG_RUNTIME_DIR` → 原生 Wayland；只有 `DISPLAY` → 解析为 X11 并
**显式拒绝**（标记 `yacr: unsupported-backend: x11` + 退出码 4）；两者皆无 → 退出 2。
显式 `wayland` 保留；显式 `x11` → **拒绝（exit 4）**；非法/歧义值硬错。**无静默回退**：

```bash
env -u DISPLAY -u XAUTHORITY GDK_BACKEND=wayland WAYLAND_DISPLAY=wayland-0 \
    XDG_RUNTIME_DIR=/run/user/1000 ./target/release/yacr --open /absolute/drawing.dwg

# X11/XWayland（预期拒绝，不是替代路径）
GDK_BACKEND=x11 DISPLAY=:0 ./target/release/yacr --open /absolute/drawing.dwg
# stderr: yacr: unsupported-backend: x11   → exit 4
```

宿主参数：`--open <path>`、`--fonts-dir <path>`、`--images-dir <path>`、`--probe`。完整说明、
后端策略/原生 Wayland 子表面、DOM-on-top 合成、输入漏斗、弹窗层、支持范围（**仅 OPEN/FIT**）与
限制见 `docs/react-linux-app.md`、`docs/adr/0011-native-wayland-subsurface.md` 与
`docs/adr/0023-dom-on-top-cross-host-composition.md`。

**配置目录**：`~/.config/yacr/`（由旧的 `~/.config/yacr-react/` 迁移，**不自动迁移**）。

**前端**：`apps/desktop-ui`（所有桌面宿主共用的唯一 React UI 包；Vite + React 19）：

```bash
cd apps/desktop-ui
npm ci          # 或 npm install
npm test        # node:test 源码结构/纯函数契约（不是运行时 DOM 挂载）
npm run build   # 产出 dist/（被 .gitignore 忽略；两个宿主的 tauri.conf.json 都指向它）
```

**无头/发布**：本宿主需要真实显示（**原生 Wayland**）；**无显示的 SSH/无头**不支持。
原生 Wayland 的私有 C shim 由 `apps/app-linux/build.rs` 经 `cc` 编译并链接系统
`wayland-client`（`pkg-config`）。该 shim **只在 Linux 宿主编译**：从**非 Linux 宿主交叉构建到
Linux** 时原生 Wayland 路径**不支持**（`build.rs` 会显式 `cargo:warning`，不会静默省略）。
真实 GPU 证据路线见 `docs/react-linux-app.md`「证据机制」（`YACR_PROBE_EVIDENCE`/
`YACR_PROBE_LIFECYCLE`/`YACR_PROBE_STACKING` + `scripts/collect-evidence.sh`；X11 行只用于采集
**拒绝合同**）；**旧 Slint 离屏/无头 `--headless` 路径已随 Slint 退役**，不再存在。

## 无头 CLI（仍可用）

```bash
cargo build -p cad-cli-tools --release --locked
./target/release/cad-cli-tools render /absolute/drawing.dwg --png /tmp/opencode/out.png
./target/release/cad-cli-tools --help
```

成功时 stdout 只输出一份 JSON 结果文档，失败为非零退出码 + 结构化错误。操作与选项见
`docs/cli.md`。离线 WGSL 校验（无需 GPU）：
`cargo test -p cad-render-wgpu --test wgsl_validation --locked`。

## 已退役 / 禁用的平台入口（勿当可用）

以下入口**不是可运行的构建路径**；脚本显式非零失败或在 CI 中 `if: false` 硬禁用。**不要**把它们
当作可用的发布流程（唯一例外是 2026-10-17 重启的 `scripts/package-linux-release.sh`，见下）：

| 入口 | 状态 |
|---|---|
| `scripts/build-web.sh`、`scripts/deploy-cloudflare-pages.sh` | **RETIRED**——浏览器 UI 未实现；非零退出，不发布任何东西 |
| `scripts/build-android.sh` | **RETIRED**——Android UI 未实现（`apps/app-android` 为占位）；非零退出 |
| `scripts/package-windows-release.sh` | **RETIRED**——无 Windows 发布流水线；非零退出（注意：`app-windows` **已是真实宿主**，退役的是**打包脚本**，不是 GUI 实现） |
| `scripts/package-macos-release.sh` | **RETIRED**——macOS GUI 未实现（`apps/app-macos` 占位 `yacr-macos`）；非零退出 |

**Linux 打包（2026-10-17 重启，非 RETIRED）**：`scripts/package-linux-release.sh` 现在为
React/Tauri 宿主构建 **Flatpak** bundle（`bin/yacr` + committed `fonts/`，运行时
`org.gnome.Platform//51`），产出 `target/release/dist/yacr-<version>-linux-<arch>.flatpak` + `.sha256`。
它**不是**分发/发布流程：**未发布 Flathub**，CI 的 `linux-release` job（`.github/workflows/build.yml`）
**仍是 `if: false` 硬禁用**（本轮**未**改 CI），因此没有可下载的发布包。见 `docs/flatpak.md`、
`docs/validation-flatpak-linux-2026-10-17.md`。

平台包状态（源码级）：

- **Windows**：`apps/app-windows`（`yacr-windows`）**已是真实 Tauri/WebView2/Win32 宿主**
  （DOM-on-top；WebView2 缺失 → 标记行 + exit 5）。CI `windows-check` 在 `windows-latest` 上做
  **原生 MSVC 编译门禁**（不运行 exe）；`windows-release` 仍 `if: false` 退役，**无发布包**。
  视觉/运行时验收 = **PENDING user real-Windows spike**（判据见 `docs/windows-app.md`）；
  Wine 只做拒绝合同 smoke。见 `docs/windows-app.md`。
- **macOS**：`apps/app-macos`（`yacr-macos`）仍是**显式 NOT IMPLEMENTED 失败桩**（打印诊断并
  `exit 1`）；CI `macos-check` 只做编译门禁，**不运行**，也不产出可分发 GUI。
- **Android / Web**：`apps/app-android` / `apps/app-web` 是**显式 NOT IMPLEMENTED 占位**
  （Android 打印退役诊断后 abort；Web wasm 入口返回错误）。旧的 wasm/APK 构建与部署全部退役。
- **Linux release**：`dev.yacr.app` Flatpak bundle 可**本地构建并安装**（`scripts/package-linux-release.sh`，
  2026-10-17 重启，见上）；但**没有受支持的发布通道**——未提交 Flathub、无分发产物，CI `linux-release`
  job 仍硬禁用。本机实测细节见 `docs/flatpak.md`。

CI 分层、被禁用的 job 清单与 NOT RUN 边界见 `docs/ci.md`。

**通用工具（保留）**：`scripts/serve-web.py` 是**通用静态文件服务器**（浏览器 UI 退役后**没有默认
bundle**，必须显式 `--directory <bundle-dir>`，且目录须含 `index.html`）。它不再属于任何可用构建路径。

## 平台无关摘要

- **renderer-only wgpu**：只有 `cad-render-wgpu` 依赖 wgpu。
- **importer-only acadrust**：只有 `cad-import-acadrust` 依赖 acadrust；**禁止 Cargo patch**。
- **UI 工具包中立**：`cad-host` 不依赖任何 UI 工具包；平台无关 UI 逻辑在 `cad-ui-core`。
  `check-architecture.py` 会拒绝任何 workspace 直接 `slint`/`slint-build`/`iced` 依赖。
- `cad-db` 是唯一权威数据源；UI 与渲染均为派生。

## 桌面 / iOS

`cad-platform` 抽象保留；本轮不建不可编译的虚假宿主。iOS 仍为保留槽位（未实现）。
