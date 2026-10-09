# Windows 宿主（`yacr.exe`）

> 状态：**构建路径为 GitHub Actions `windows-latest` 原生 MSVC**（`windows-check` /
> `windows-release`）；本机已用 `cargo xwin` 复现 MSVC release 与 MinGW 交叉，并用 Wine
> 做参数解析 smoke。窗口、设备选择、原生文件对话框与真实 GPU 渲染 **未在真实 Windows 上
> 验证**，CI job 也未在本环境运行（见「证据与限制」）。

## 组成

- `apps/app-windows`：Windows 二进制 `yacr`。它是**薄封装**，直接调用
  `apps/app-linux` 暴露的共享桌面入口 `app_linux::entry::run()`；控制器、Slint 桥、
  测量/绘制/字体安装与 Linux 宿主编译同一份实现。
- `apps/app-linux`：crate 名保留 `app-linux`，但宿主实现按
  `cfg(any(target_os = "linux", target_os = "windows"))` 编译，成为两个桌面宿主
  共享的实现层。平台差异只在 `src/host/` 内按 cfg 选择：
  - 文件选择：Linux 走 XDG portal（`ashpd`，`file_picker.rs`）；Windows 走系统
    通用文件对话框（`rfd`，无额外 feature）。
  - 配置目录：Linux `$XDG_CONFIG_HOME/yacr` 或 `$HOME/.config/yacr`；
    Windows `%APPDATA%\yacr`，回退 `%LOCALAPPDATA%\yacr`（`config_file.rs`）。
  - 系统默认字体：Linux 用 `fc-match` 与常见路径；Windows 读取
    `%WINDIR%\Fonts` 与 `%LOCALAPPDATA%\Microsoft\Windows\Fonts` 的常见 UI 面
    （`cad-platform::fonts::local::system_default_candidates`）。
- 渲染后端与 Linux 相同：Slint + 共享 CAD wgpu 渲染器
  （`renderer-femtovg-wgpu`）；Windows 上 wgpu 走 DX12/Vulkan。
- 无头离屏（`--headless --output DIR`）复用 `cad-ui-slint::offscreen`，仍强制
  软件 Vulkan（需宿主自备 lavapipe 等），不随包提供。

## 构建

**主路径：GitHub Actions `windows-latest` 原生 MSVC。** `.github/workflows/build.yml`：

- `windows-check`（每次 push/PR）：`cargo check -p app-windows --all-targets --locked`
  原生 `x86_64-pc-windows-msvc` 编译门禁（不运行 exe）。
- `windows-release`（`workflow_dispatch` 或 `v*` tag）：`bash
  scripts/package-windows-release.sh` 构建 release 并上传 `yacr-windows-release`
  artifact（zip + sha256）。MSVC 目标自动 `+crt-static` 静态链接 CRT，无需 VC++
  redistributable。

本机（无需 Windows）复现脚本逻辑：

```bash
# 方式 A：Linux 交叉编译 GNU 目标（需 MinGW-w64）
rustup target add x86_64-pc-windows-gnu
sudo apt-get install -y gcc-mingw-w64-x86-64
scripts/package-windows-release.sh                      # TARGET 默认 x86_64-pc-windows-gnu

# 方式 B：Linux 上用 cargo-xwin 构建 MSVC 目标（需 clang/lld + cargo-xwin）
cargo install cargo-xwin --locked
CARGO_XWIN=1 scripts/package-windows-release.sh         # TARGET 默认 x86_64-pc-windows-msvc

# 通用开关
WITH_FONTS=0 scripts/package-windows-release.sh         # 离线：仅已提交 osifont
YACR_WINDOWS_SMOKE=1 scripts/package-windows-release.sh # 仅 Linux + wine：参数解析 smoke
```

产出 `target/<TARGET>/release/dist/yacr-<version>-windows-x86_64.zip`（及 `.sha256`），
内含：

- `bin/yacr.exe`：release GUI 宿主；
- `bin/cad-cli-tools.exe`：release 无头 CLI（含 `render`）；
- `bin/*.dll`：仅当 exe 导入非系统 MinGW 运行库时打包（Rust `windows-gnu`
  标准库通常已静态链接，实际多为零个）；
- `fonts/`：CAD 字体包（与 Linux 包相同的 `fetch-fonts.sh` 组装；`WITH_FONTS=0`
  仅带已提交 `fonts/`）；
- `docs/`、`scripts/`、`LICENSE`、`README.md`、`THIRD_PARTY_NOTICES.md`、
  `PACKAGE.txt`。

## 平台行为

- 双击 `bin\yacr.exe`（或 `yacr.exe --open C:\path\drawing.dwg`）启动 GUI；无
  `--open` 时显示内置演示几何。
- 「打开」按钮调用系统文件对话框，过滤 `*.dwg`/`*.dxf`；取消返回取消诊断，
  不伪造成功。
- 字体：先读可执行文件同级 `fonts/`（或 `--fonts-dir`），再合并 `--font NAME=PATH`，
  最后以系统默认面作为第一回退；缺字体回退到默认轮廓面，不静默丢字。
- 配置与偏好持久化到 `%APPDATA%\yacr\{config.json,preferences.json}`。

## 证据与限制

**已实测（本仓库，Linux 宿主，交叉/工具链模拟）**

- GNU 交叉：`cargo check -p app-windows --target x86_64-pc-windows-gnu` 通过；
  `YACR_WINDOWS_SMOKE=1 scripts/package-windows-release.sh`（MinGW）exit 0。
- MSVC（`cargo xwin`，与 CI `windows-latest` 同目标 `x86_64-pc-windows-msvc`）：
  `RUSTFLAGS=-C target-feature=+crt-static cargo xwin build --release ...` 通过；
  `scripts/check-pe-imports.py` 显示 `+crt-static` 下**无任何非系统导入**（未加该开关时
  导入 `vcruntime140.dll`，故该开关必要）。
- `scripts/check-pe-imports.py` 的 DLL 集合与 `x86_64-w64-mingw32-objdump -p` 一致
  （对 GNU 产物做差分验证）。
- Wine 下实际运行（软件翻译，**非真机**）：`yacr.exe --bogus value` 报
  `unknown option or invalid value: --bogus` 退出 1；`cad-cli-tools.exe --help` exit 0；
  `scan entities.dxf` 解析 602 实体 / 75 块定义；`build-representation` 字体差分
  （带包内 `fonts/` vs 无）证明字体被读取。
- MSVC 与 GNU 的 release 打包均产出 PE32+ x86-64、零随包 DLL。
- 本机 MSVC 产物（含全量字体）：
  `target/x86_64-pc-windows-msvc/release/dist/yacr-0.1.0-windows-x86_64.zip`，
  **57364212 字节**，sha256
  `deb9cc9ec3c31dddf1aeef9aa66bd7cd5a0d9184678c6b34e6929faa8c78abb7`；`bin/` 仅
  `yacr.exe` 与 `cad-cli-tools.exe`，`fonts/` 101 文件。

**NOT RUN / 未验证**

- GitHub Actions 的 `windows-check` / `windows-release` job 本身**未运行**（本环境无
  runner）：CI 的 MSVC 原生构建与 artifact 上传尚未取得 job 证据。
- 真实 Windows 上的窗口、DPI、原生文件对话框、配置目录写入。
- 真实 GPU（DX12/Vulkan）渲染与像素/视觉验收；Wine 软件翻译不等于真机。
- `--headless` 离屏出图（需 Windows 侧软件 Vulkan）。
- 未做代码签名；SmartScreen 可能对未签名 exe 给出警告。

以上限制不得推广为 Windows 兼容性或性能结论。
