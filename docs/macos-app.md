# macOS 宿主（`Yacr.app` / `yacr-macos`）

> 状态：**构建路径为 GitHub Actions `macos-latest` 原生构建**（`macos-check` /
> `macos-release`）。本仓库的开发主机是 Linux，**没有** Apple SDK/链接器，无法在本机
> 产出 Mach-O 可执行文件（Slint 的 Cocoa/Metal 后端也无法真正跨平台链接），因此本机
> 不产出 macOS 包。真实 Mac 上的窗口、Metal GPU、原生文件对话框与配置目录写入
> **未验证**；CI job 也未在本环境运行（见「证据与限制」）。

## 组成

- `apps/app-macos`：macOS 二进制 `yacr-macos`。它是**薄封装**，直接调用
  `apps/app-linux` 暴露的共享桌面入口 `app_linux::entry::run()`；控制器、Slint 桥、
  测量/绘制/字体安装与 Linux/Windows 宿主编译同一份实现。
- `apps/app-linux`：crate 名保留 `app-linux`，但宿主实现按
  `cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))` 编译，
  成为三个桌面宿主共享的实现层。平台差异只在 `src/host/` 内按 cfg 选择：
  - 文件选择：Linux 走 XDG portal（`ashpd`）；Windows 与 macOS 走系统通用文件
    对话框（`rfd`，Windows 用 Win32 通用对话框，macOS 用 `NSOpenPanel`）。
  - 配置目录：Linux `$XDG_CONFIG_HOME/yacr` 或 `$HOME/.config/yacr`；
    Windows `%APPDATA%\yacr`，回退 `%LOCALAPPDATA%\yacr`；macOS
    `$HOME/Library/Application Support/yacr`（**不**读取 `XDG_CONFIG_HOME`，
    避免 shell 变量重定向存储）。
  - 系统默认字体：macOS 读取 `/System/Library/Fonts`（含 `Supplemental/`）与
    `/Library/Fonts` 的常见 UI 面
    （`cad-platform::fonts::local::system_default_candidates`）；没有 `fc-match`。
- 渲染后端：Slint + 共享 CAD wgpu 渲染器（`renderer-femtovg-wgpu`）；macOS 上 wgpu
  走 Metal。窗口适配器由 Slint 的 Cocoa 后端提供。
- 无头离屏（`--headless --output DIR`）：离屏验证平台**强制软件 Vulkan**，macOS 没有
  Vulkan，因此 `--headless` 在 macOS 上被显式报告为不支持（解析后立即返回错误），
  不会静默降级或伪造成功。

## 打包

`scripts/package-macos-release.sh`（**只能在 macOS 上运行**）产出：

```
target/macos/<arch>/dist/yacr-<version>-macos-<arch>.tar.gz   (+ .sha256)
```

`<arch>` 默认为 `universal`（同时构建 `aarch64-apple-darwin` 与
`x86_64-apple-darwin`，用 `lipo -create` 合成通用二进制）；`MACOS_ARCH=arm64` 或
`x86_64` 可只构建单一架构。压缩包内容：

- `Yacr.app/`：自包含 GUI 应用包。`Contents/MacOS/yacr-macos` 是通用 Mach-O 可执行
  文件，`Contents/Info.plist` 声明 `CFBundleExecutable=yacr-macos`、`LSMinimumSystemVersion=11.0`
  与 dwg/dxf Viewer 文档类型，`Contents/Resources/fonts/` 是 CAD 字体包；
- `bin/cad-cli-tools`：release 无头 CLI（含 `render`）；
- `bin/yacr-macos`：指向应用包内 GUI 可执行文件的符号链接（便于从终端运行）；
- `fonts`：指向 `Yacr.app/Contents/Resources/fonts` 的符号链接，使同级 CLI 也能自动
  加载同一份字体包（避免在压缩包里重复约 55 MiB 字体）；
- `docs/`、`scripts/{fetch-test-dwg,render-smoke}.sh`、`LICENSE`、`README.md`、
  `THIRD_PARTY_NOTICES.md`、`PACKAGE.txt`。

打包脚本在打包内做以下**校验**（任一失败即中止，不产出半成品）：

- 两个可执行文件必须是 Mach-O（`file`），且 `lipo -archs` 打印目标架构；
- `otool -L` 列出的每个动态依赖必须来自系统（`/usr/lib/`、`/System/Library/`、
  `/System/iOSSupport/`）；出现 `@rpath`/Homebrew 等非系统库即报错，避免产出“只在构建机
  能跑”的包；
- 可选 `YACR_MACOS_SMOKE=1`：运行打包内 CLI `--help`，并断言 GUI 对 `--bogus value`
  报告文档化的参数解析错误后退出 1（只验证参数解析，不打开窗口/GPU）。

字体包默认 `WITH_FONTS=1`，经 `scripts/fetch-fonts.sh` 下载 mlightcad 全量目录并合并已
提交的 QCAD `osifont.ttf`；`WITH_FONTS=0` 为离线构建，只带已提交字体（见 `docs/fonts.md`）。

## 构建

**主路径：GitHub Actions `macos-latest` 原生构建。** `.github/workflows/build.yml`：

- `macos-check`（每次 push/PR）：`cargo check -p app-macos --all-targets --locked`
  原生 arm64 macOS 编译门禁（不运行 app）。
- `macos-release`（`workflow_dispatch` 或 `v*` tag）：安装
  `aarch64-apple-darwin` + `x86_64-apple-darwin`，执行
  `MACOS_ARCH=universal WITH_FONTS=1 bash scripts/package-macos-release.sh`，上传
  `yacr-macos-release` artifact（tar.gz + sha256）。

本机（Linux）无法复现该构建；但可以做**静态**交叉检查（不链接、不产出 Mach-O，仅类型
检查 macOS 门控代码，需联网下载该 target 的 std 与 macOS 依赖 crate）：

```bash
rustup target add aarch64-apple-darwin
cargo check -p app-macos --all-targets --target aarch64-apple-darwin --locked
cargo clippy -p app-macos -p app-linux --all-targets --target aarch64-apple-darwin --locked -- -D warnings
```

这只能证明 macOS 门控代码在类型层面成立，**不**产出 Mach-O，也不代表任何运行/渲染结果。

## 平台行为

- 双击 `Yacr.app`（或 `./bin/yacr-macos --open /path/drawing.dwg`）启动 GUI；无
  `--open` 时显示内置演示几何。
- 「打开」按钮调用 `NSOpenPanel`，过滤 `*.dwg`/`*.dxf`；取消返回取消诊断，不伪造成功。
- 字体：先读应用包内 `Contents/Resources/fonts/`（或同级 `fonts/`、`--fonts-dir`），
  再合并 `--font NAME=PATH`，最后以系统默认面作为第一回退；缺字体回退到默认轮廓面，
  不静默丢字。
- 配置与偏好持久化到 `~/Library/Application Support/yacr/{config.json,preferences.json}`。

## 证据与限制

**已实测（本仓库，Linux 主机）**

- `cargo check -p app-macos` 在 **Linux 主机**（`cfg(not(macos))` 分支）通过；这只是
  证明新增 workspace 成员在非 macOS 上仍可编译。
- **macOS 门控代码路径已用交叉 `cargo check` 类型检查**（Rust 1.99.0，本机 Linux，
  `rustup target add aarch64-apple-darwin`）：`cargo check -p app-macos --all-targets
  --target aarch64-apple-darwin --locked` 与
  `cargo clippy -p app-macos -p app-linux --all-targets --target aarch64-apple-darwin
  --locked -- -D warnings` 均通过。这只证明 macOS 分支（`NSOpenPanel`、`Application
  Support`、`/System/Library/Fonts`、`entry.rs` 的 macOS 分支）在类型层面成立，**不**
  链接、不产出 Mach-O、不运行。
- `python3 scripts/check-architecture.py`、`scripts/check-workflows.py`、
  `scripts/test-package-macos-release.py` 在 Linux 上通过：workspace DAG/边界、CI job
  声明与打包脚本的静态/变异契约成立。

**NOT RUN / 未验证**

- `macos-check` / `macos-release` GitHub Actions job 本身**未运行**（本环境无 macOS
  runner）：真实 arm64 编译、universal `lipo`、`otool` 依赖校验与 artifact 上传尚无
  job 证据。
- macOS 门控代码路径**未链接、未在 macOS 上运行**（交叉 `cargo check` 只做类型检查，
  不代替链接/运行）。
- 真实 Mac 上的窗口、DPI/Retina、Metal 渲染与像素/视觉验收；原生文件对话框；
  配置目录写入。
- `--headless` 离屏出图：macOS 上显式不支持（无 Vulkan）。
- 代码签名与 notarization 未做；Gatekeeper 可能对未签名应用给出警告。

以上限制不得推广为 macOS 兼容性或性能结论。
