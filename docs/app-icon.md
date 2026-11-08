# 应用图标（App icon）

> **历史文档（早期桌面 UI 已退役，2026-10-16）**：本文描述的是**迁移前**的桌面/web 宿主；
> 其中旧 UI crate、`yacr-linux`、旧脚本命令均已退役（部分脚本改为非零 NOT IMPLEMENTED）。
> 当前架构与构建见 `docs/architecture.md`、`docs/react-linux-app.md`、`docs/build.md`、
> [ADR 0010](adr/0010-react-linux-host.md)。下文仅作历史记录。

单一来源：**`assets/yacr-icon.svg`**（仓库根 `assets/`，用户提供的 YACR “Y” 蓝图图标，
自包含，无外部引用）。所有平台派生资产都由它生成，不得各自维护副本。

## 派生资产

`python3 scripts/generate-icons.py` 从 SVG 生成全部派生资产；`--check` 逐字节比对已提交
资产，作为漂移守卫（需要 `rsvg-convert` 与 Pillow，属于一次性/本地校验工具，默认提交门禁
不运行）。生成结果（均已提交，构建时无需光栅化工具）：

| 路径 | 用途 |
| --- | --- |
| `assets/yacr-icon.ico` | Windows 多分辨率图标（16–256） |
| `assets/yacr-icon.icns` | macOS 多分辨率图标（16–1024） |
| `assets/icons/yacr-<size>.png` | Linux hicolor、Web manifest、apple-touch |
| `apps/app-android/src-tauri/icons/icon.png` | Android 宿主（Tauri 2）的 `bundle.icon` 源；`tauri android init` 由它生成 `gen/android` 的 mipmap。**不是** `generate-icons.py` 的输出：它是 `assets/icons/yacr-1024.png` 的副本，由 `scripts/test-android-host.py` 断言存在 |

## 各端接入

- **桌面（Linux / Windows / macOS）**：早期桌面 UI（已移除）的窗口定义曾设置
  `icon: @image-url("../../../assets/yacr-icon.svg")`，图标在编译期嵌入，
  运行时二进制自包含；winit 后端据此设置窗口与任务栏图标（X11 生效；Wayland 通常改从
  `.desktop`/app_id 取图标，见下）。该 UI 的 app-icon 契约测试已随其删除。
- **macOS 包**：`scripts/package-macos-release.sh` 复制
  `Contents/Resources/Yacr.icns` 并在 `Info.plist` 写 `CFBundleIconFile=Yacr`。
- **Linux 包**：`scripts/package-linux-release.sh` 舞台化
  `share/icons/hicolor/scalable/apps/yacr.svg`、`256x256/apps/yacr.png` 与
  `share/applications/yacr.desktop`（`Icon=yacr`）。
- **Windows 包**：`apps/app-windows/build.rs` 对 Windows 目标调用 `tauri_build::build()`，
  由 `tauri-build` 的 `tauri_winres` 把 `assets/yacr-icon.ico` 编进 `yacr-windows.exe` 的 PE
  资源段（缺失图标即构建失败，**不**静默产出无图标可执行文件）；`CARGO_CFG_TARGET_OS` 门禁
  保证非 Windows 目标不生成 Tauri context、不跑资源编译器。`apps/app-windows/icons/icon.ico`
  是同一资产的逐字节副本（`tauri.conf.json` 的 `bundle.icon` 指向它；契约测试
  `scripts/test-package-windows-release.py` 断言两者一致）。打包脚本与 `windows-release` job
  自 2026-11-01 起重新启用（event-gated），构建时用 `check-pe-imports.py --has-icon` 校验图标；
  本环境**未运行**该 CI 打包，MSVC 产物也仍不构成运行/外观证据。
- **Android**：`apps/app-android/Cargo.toml` 的 `[package.metadata.android] resources = "res"`
  与 `[package.metadata.android.application] icon/label`；`res/` 由上面的生成资产填充。
- **Web**：当前的 `apps/app-web/web/index.html` **没有** icon/favicon 链接（也没有
  `manifest.webmanifest`），浏览器标签页因此用默认图标——这是**未实现**，不是已接入；构建脚本
  `scripts/build-web.sh` 只组装 shell 与 wasm 产物（`index.html`/`main.js`/`worker.js`/
  `style.css`/`pkg/*`），不拷贝 `assets/`。若恢复 web 图标支持，需同时补 shell 链接、构建拷贝与
  契约测试。

## 验证状态（诚实记录）

- **已运行（合成/本地）**：`node --test scripts/test-web-icon.mjs`；
  `python3 scripts/test-android-host.py`（Android 宿主图标路径 + `bundle.icon` 断言）；
  三个 `test-package-*-release.py`；`python3 scripts/generate-icons.py --check`。
- **未运行**：真实 GPU/窗口管理器图标显示、Android 真机启动图标外观、macOS `.app`
  图标。Windows `.exe` 内嵌图标经本机 GNU 交叉构建 + `check-pe-imports.py --has-icon`
  验证；MSVC 路径由 windows-release CI 构建时校验，均不证明真机外观。
  Android 侧只验证了 APK 构建与模拟器安装（`docs/android-host.md`），**未**验证启动器图标外观。
- **本地已运行（补充）**：`cargo build -p app-windows --target x86_64-pc-windows-gnu` +
  `python3 scripts/check-pe-imports.py --has-icon target/x86_64-pc-windows-gnu/debug/yacr.exe`。

## 平台工具

派生资产需要 `rsvg-convert`（`librsvg2-bin`）与 Pillow；这是**本地生成工具**，非运行时或
构建依赖。Android 启动图标通常也可只提交生成后的 `res/`，CI 不需重跑生成。
