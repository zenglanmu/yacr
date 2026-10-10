# 构建与平台入口

工具链 1.99.0（`rust-toolchain.toml`）、Cargo.lock 已锁定。
若 `PATH` 无 cargo，加入 `$HOME/.cargo/bin`。

## 纯核心

2026-10-04 起，默认提交门禁为 debug 编译与静态检查，包含 Linux App/Slint，
但不运行完整测试、Linux 离屏渲染或 release 编译；**编译不等于运行验收**：

```bash
cargo clippy --workspace --exclude app-android --exclude app-web --all-targets --locked -- -D warnings
cargo check --workspace --exclude app-android --exclude app-web --all-targets --locked
cargo check --workspace --lib --target wasm32-unknown-unknown --locked
```

静态门禁完整列表见 `AGENTS.md`。Linux 离屏/release 可选运行入口和限制见 `docs/linux-app.md`。
完整测试在明确要求时单独执行，未运行必须标注 NOT RUN。

```bash
cargo test --workspace --exclude cad-ui-slint --exclude app-android --exclude app-web --locked
python3 scripts/check-architecture.py
```

`cad-ui-slint` 需要 Linux 宿主的 fontconfig/freetype 开发头与 pkg-config 才能为本机
编译；2026-10-03 用户授权通过 sudo 安装 `pkgconf libfontconfig-dev libfreetype-dev`，
现在可以在 Linux 编译并执行 UI 契约与真实离屏 Slint 测试，无需窗口系统/Android 模拟器：

```bash
bash scripts/check-ui-native.sh
cargo test -p cad-ui-slint --lib --locked -- --test-threads=1
```

`YACR_UI_OUTPUT` 可指定新的截图目录。官方 FemtoVG/wgpu 离屏渲染共享 Slint 组件与 CAD
纹理，强制验证软件 Vulkan；这是合成内容/软件 GPU 证据，不等于桌面宿主产品或真机验收。

## Linux 发布包（Flatpak：GUI 主应用 + 无头 CLI）

CI 发布层：`build.yml` 的 `linux-release`（`workflow_dispatch` / `v*` tag）在
`ubuntu-latest` 安装 Slint 构建依赖与 `flatpak`/`flatpak-builder`、装好
`org.freedesktop.Platform//26.08` 运行时与 SDK、`cargo fetch --locked` 预热后运行同一脚本，
上传 `yacr-linux-release` artifact（`.flatpak` + sha256）。编译门禁仍是每次 push/PR 的
`linux-app`。

```bash
# 一次性准备（本机或 CI）：flatpak-builder + freedesktop 运行时/SDK
sudo apt-get install -y flatpak flatpak-builder
flatpak remote-add --if-not-exists flathub https://flathub.org/repo/flathub.flatpakrepo
sudo flatpak install -y --noninteractive flathub \
  org.freedesktop.Platform//26.08 org.freedesktop.Sdk//26.08

# 构建 + 打包（可选：对真实 DWG 用打包前 CLI 出图并校验 PNG 非空）
YACR_TEST_DWG=/path/input.dwg scripts/package-linux-release.sh
# 可选：在构建沙箱内启动一次 CLI，证明 Flatpak 能真正运行
YACR_FLATPAK_SMOKE=1 scripts/package-linux-release.sh
```

产出 `target/release/dist/yacr-<version>-linux-<arch>.flatpak`（及 `.sha256`），应用安装在
`/app`：`bin/yacr-linux`、`bin/cad-cli-tools`、`fonts/`（默认 `WITH_FONTS=1` 组装 mlightcad
全量 + 已提交 osifont；`WITH_FONTS=0` 仅带已提交字体）与图标/桌面项/AppStream 元数据。CLI
与 GUI 都从二进制同级 `fonts/` 自动加载。

Flatpak 由 `org.freedesktop.Platform` 运行时提供 glibc、fontconfig/freetype、GL/Vulkan 加载
器与窗口库，**不**打包宿主共享库（避免与运行时的 libc 世代耦合）；`runtime-version 26.08`
的 glibc 2.44 高于受支持构建主机的 glibc，故宿主构建的二进制可在沙箱内加载。manifest 在
`packaging/flatpak/dev.yacr.app.yml`，仅打包**预构建**二进制，沙箱内不编译。

字体在**宿主**上组装：`fetch-fonts.sh` 把目录写进 `target/flatpak/payload/fonts`，manifest 只
把这份本地目录复制进 `/app/fonts`，`flatpak-builder` 沙箱内**不联网**。若已在别处准备好字体
目录，用 `FONTS_DIR=<dir>`（须含 `fonts.json`）跳过下载直接打包，例如：

```bash
scripts/fetch-fonts.sh /tmp/yacr-fonts           # 一次性下载（可在有网的主机/CI cache）
FONTS_DIR=/tmp/yacr-fonts scripts/package-linux-release.sh
```

```bash
flatpak install --user target/release/dist/yacr-0.1.1-linux-x86_64.flatpak
flatpak run dev.yacr.app                                  # GUI（Wayland/X11 + GPU）
flatpak run --command=cad-cli-tools dev.yacr.app --help   # 无头 CLI
```

限制：glibc/内核、窗口系统、Vulkan 驱动与 `xdg-desktop-portal`（桌面打开对话框）由运行时/
宿主提供；包未签名。细节与验证证据见 `docs/flatpak.md`、`docs/linux-app.md` 与
`docs/validation.md`。

## Android APK（已验证编译、打包、安装与运行）

CI 发布层：`build.yml` 的 `android-release`（`workflow_dispatch` / `v*` tag）在
`ubuntu-latest` 上自装 **JDK 17**（`actions/setup-java`）与固定 SDK/NDK
（`build-tools;34.0.0`、`platforms;android-34/30`、`ndk;27.0.12077973`，经 `sdkmanager`），
生成开发签名 keystore、把全量 CAD 字体打进 `apps/app-android/assets/fonts/` 后运行
`scripts/build-android.sh --release`，上传 `yacr-android-release` APK artifact；`android-apk`
仍是每次 push/PR（`ANDROID_CI_ENABLED` 门控）的构建+事实记录 job。二者都**只产出 APK，
不等于安装运行**。

依赖：JDK 17、Android SDK（build-tools 34.0.0、platform android-34/30）、
NDK 27.0.12077973、`cargo-apk 0.10.0`。

```bash
export ANDROID_HOME="$HOME/android-sdk"
export ANDROID_NDK="$ANDROID_HOME/ndk/27.0.12077973"      # skia-bindings 需要
export ANDROID_NDK_HOME="$ANDROID_NDK"
export JAVA_HOME="$HOME/jdk17"
export PATH="$HOME/.cargo/bin:$JAVA_HOME/bin:$PATH"

rustup target add aarch64-linux-android
# 调试 APK（445 MB，调试签名）
cargo apk build -p app-android --target aarch64-linux-android --lib
# 发布 APK（11.6 MB）；需要本地开发签名密钥（见下）
cargo apk build -p app-android --target aarch64-linux-android --lib --release
# 产物：target/{debug,release}/apk/yacr.apk
```

发布签名：`[package.metadata.android.signing.release]` 指向 `dev-release.jks`，
该文件被 `.gitignore` 的 `*.jks` 排除，需本地生成：

```bash
keytool -genkeypair -keystore apps/app-android/dev-release.jks -alias yacr \
  -keyalg RSA -keysize 2048 -validity 10000 -storepass yacrdev -keypass yacrdev \
  -dname "CN=yacr dev, O=yacr"
```

生产密钥绝不进入仓库（规范 §9.1）。注意：`min_sdk_version`/`target_sdk_version`
metadata 未生效，实测 manifest 为 minSdk 23 / targetSdk 30；最低支持范围尚未在真机
验证，不得作为兼容性声明。

仅 Rust 检查（不需要链接）：

```bash
cargo check --target aarch64-linux-android -p cad-ui-slint --locked
cargo check --target aarch64-linux-android -p app-android --locked
```

`min/target SDK` 与 ABI 是预留值，未在真机实测最低支持范围。发布签名密钥不入库，
不申请不必要的全盘权限。**当前 SAF 文件选择器未实现**：宿主按候选路径尝试打开
DWG，找不到时显示内置演示几何并明确提示（见 `app-android`），不能宣称 F01 已验收。

### x86_64 模拟器安装与运行（已验证；SwiftShader，**非真机**）

评测对象：已启动的无头模拟器 `emulator-5554`（`dev_api35`，API 35，`google_apis`
x86_64，`-gpu swiftshader`）。APK 必须针对 **x86_64-linux-android**；aarch64 在
x86_64 镜像上经 NDK translation 安装但运行 abort。

```bash
# 1) 从当前 checkout 构建 x86_64 release APK（约 12.8 MB，release 签名）
YACR_REPO="$PWD" PROFILE=release ~/android-dev/build-apk.sh
#    等价直接命令（CARGO_TARGET_DIR 指向仓库外时更稳）：
#    cargo apk build -p app-android --target x86_64-linux-android --lib --release
#    产物：$CARGO_TARGET_DIR/release/apk/yacr.apk

# 2) 安装并启动 NativeActivity
adb install -r -t target/release/apk/yacr.apk
adb shell am start -n dev.yacr.app/android.app.NativeActivity
adb shell pidof dev.yacr.app          # 进程存活即启动成功

# 3) 抓日志与截图
adb logcat -d > /tmp/yacr-logcat.txt
adb exec-out screencap -p > /tmp/yacr.png
```

实测结果（完整证据见 `docs/validation-android.md`）：

- 进程启动、Slint + wgpu 初始化，内置演示几何（虚线圆/弧）真实渲染；无崩溃/ANR。
- 后端（设备日志原文）：`CAD renderer initialized: preference=WebGpu actual=WebGpu
  ... max_texture_dimension=8192`；底层 wgpu 走 Android Vulkan（模拟器 SwiftShader），
  Slint 合成器为 Skia。
- 画布拖动平移、点按「适应」重定中心均已生效（像素 diff 非零；修复前为 0）。
- **限制**：`safe_insets` 未被消费、surface 尺寸/旋转未回传（audit U07），顶部工具栏被
  系统状态栏遮挡，Open/DWG 选择在本次运行中不可达（打开图纸 **NOT RUN**）；量测拾取未
  接线；图层/布局/批注面板状态未推送。真机仍未验证。

## Web（Wasm 静态产物 + 最小 JS 宿主）

```bash
scripts/build-web.sh                 # 产出 web-dist/（index.html、main.js、pkg/yacr.js、yacr_bg.wasm）
scripts/serve-web.py --directory web-dist --port 8090
# 浏览器打开 http://127.0.0.1:8090/
```

- 需要 wasm-bindgen-cli **0.2.129**（与 Cargo.lock 中的 wasm-bindgen 严格匹配）与
  `wasm32-unknown-unknown` target。
- **时钟**：`std::time::Instant::now()` / `SystemTime::now()` 在
  `wasm32-unknown-unknown` 上会 panic（`time not implemented on this platform`），
  且 `panic=abort` 会把 panic 点所在的 `RefCell` 借用永久卡住。wasm 运行时可达的
  计时必须用 `web-time`（本仓 `cad-render-wgpu`、`cad-import-acadrust` 已在 wasm
  目标下改为 `web_time::Instant`；见 `docs/validation-web.md` §3.1）。
- 服务必须发送 `application/wasm`；`scripts/serve-web.py` 已设置，部署到静态托管时同样配置。
  HTTPS/安全上下文为 File API 与持久化所需；COOP/COEP 仅可选共享内存路径需要
  （`--coi` 打开）。
- 宿主为最小 JS：`apps/app-web/web/main.js` 只负责加载 wasm、连接 File API 选择器、
  下载导出批注，并吞掉 winit 在 wasm 上用于移交事件循环的已记录异常。
- 后端选择：UI 工具栏 “Auto / WebGPU / WebGL2”。Auto 会真实请求 WebGPU adapter，
  失败才回退 WebGL2；强制选择失败时记录原因并回退 WebGL2（写入 localStorage 后重建会话，
  规范 §6 允许重启渲染会话）。`?backend=webgl2` 可覆盖。
- 无头浏览器验证：

```bash
scripts/serve-web.py --directory web-dist &
node scripts/check-web-ui.mjs http://127.0.0.1:8090/ /tmp/opencode/yacr-web.png
```

脚本检查 wasm 启动、渲染后端就绪、截图非空白（内置 PNG 解码统计）并收集控制台错误。
它记录“浏览器（无头 Chromium）通过”，不等于桌面/移动浏览器矩阵或真机通过。

仅 Rust 检查（不需要链接）：

## Windows release 包（MSVC，GitHub Actions `windows-latest`）

Windows 宿主是 `apps/app-windows`（二进制 `yacr.exe`），薄封装共享桌面实现
`apps/app-linux`；平台差异（文件对话框、配置目录、系统字体）按 cfg 选择。**主构建路径
是 GitHub Actions 的 Windows runner 原生 MSVC**（`x86_64-pc-windows-msvc`）：

- `.github/workflows/build.yml` `windows-check`（每次 push/PR）：
  `cargo check -p app-windows --all-targets --locked`（编译门禁，不运行）。
- `windows-release`（`workflow_dispatch` 或 `v*` tag）：
  `bash scripts/package-windows-release.sh`，上传 `yacr-windows-release` artifact。
- MSVC 目标自动 `+crt-static` 静态链接 CRT，无需 VC++ redistributable；
  `scripts/check-pe-imports.py` 断言无非系统导入（否则**中止**）。

本机（Linux）复现同一脚本：

```bash
# GNU 交叉（MinGW-w64）
rustup target add x86_64-pc-windows-gnu
sudo apt-get install -y gcc-mingw-w64-x86-64
scripts/package-windows-release.sh                         # TARGET 默认 x86_64-pc-windows-gnu

# MSVC（cargo-xwin；需 clang/lld）
cargo install cargo-xwin --locked
CARGO_XWIN=1 scripts/package-windows-release.sh            # TARGET 默认 MSVC

WITH_FONTS=0 scripts/package-windows-release.sh            # 离线：仅已提交 osifont
YACR_WINDOWS_SMOKE=1 scripts/package-windows-release.sh    # Linux + wine 参数解析 smoke
```

产出 `target/<TARGET>/release/dist/yacr-<version>-windows-x86_64.zip`（及 `.sha256`），
内含 `bin/yacr.exe`、`bin/cad-cli-tools.exe`、同级 `fonts/`、`docs/` 与脚本
（详见 `docs/windows-app.md`）。

**边界**：CI 的 MSVC job 与真实 Windows 窗口/文件对话框/真实 GPU 与像素验收 **未运行**；
Linux 上已用 cargo-xwin 复现 MSVC release 构建与 Wine 参数解析，但 Wine 不是真机。该包未签名。

## macOS release 包（`macos-latest` 原生构建，`Yacr.app`）

macOS 宿主是 `apps/app-macos`（二进制 `yacr-macos`），同样是共享桌面实现
`apps/app-linux` 的薄封装；平台差异（`NSOpenPanel` 文件对话框、
`~/Library/Application Support` 配置目录、`/System/Library/Fonts` 系统字体）按 cfg 选择。
**主构建路径是 GitHub Actions 的 macOS runner 原生构建**（无法从 Linux 交叉链接）：

- `.github/workflows/build.yml` `macos-check`（每次 push/PR）：
  `cargo check -p app-macos --all-targets --locked`（编译门禁，不运行）。
- `macos-release`（`workflow_dispatch` 或 `v*` tag）：
  `MACOS_ARCH=universal WITH_FONTS=1 bash scripts/package-macos-release.sh`，上传
  `yacr-macos-release` artifact。

产出 `target/macos/universal/dist/yacr-<version>-macos-universal.tar.gz`（及 `.sha256`），
内含自包含 `Yacr.app`（`Contents/MacOS/yacr-macos` + `Info.plist` +
`Contents/Resources/fonts/`）、`bin/cad-cli-tools`、指向字体的 `fonts` 符号链接与文档
（详见 `docs/macos-app.md`）。脚本用 `lipo` 合成 arm64 + x86_64 通用二进制，并用
`otool -L` 断言依赖只来自系统库（否则中止）。

**边界**：本机是 Linux，**无法**产出 Mach-O；CI 的 macOS job 与真实 Mac 窗口/Metal
GPU/文件对话框/像素验收 **未运行**。`--headless` 离屏验证在 macOS 上显式不支持（无
Vulkan）。该包未签名、未 notarize。

## 桌面 / iOS

`cad-platform` 抽象与 ADR；未建不可编译的虚假宿主。macOS 桌面宿主已按上述 cfg 模式接入
（`apps/app-macos` + 共享 `app-linux`），但真实 Mac 上的运行/渲染仍属 NOT RUN；iOS 仍为
保留槽位。
