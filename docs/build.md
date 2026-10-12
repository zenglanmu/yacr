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

`scripts/build-android.sh` 在自身隔离的子 shell 中清除大小写两种 HTTP/HTTPS/ALL/NO
proxy 变量，避免意外继承代理；需要指定 HTTPS 代理时可在调用前设置
`ANDROID_BUILD_HTTPS_PROXY=http://host:port`，脚本仅在子进程中将其设为小写 `https_proxy`
（`http_proxy` 等仍保持清除）。不论直接执行或 `source`，均不会改写调用者 shell 的代理环境；
脚本其余 Android 环境变量也只在该子 shell 内生效。默认未设置时仍为无代理。

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

### 本地 release APK（与 CI 一样完整打包字体）

`android-release` workflow 的顺序是先运行 `scripts/fetch-fonts.sh
apps/app-android/assets/fonts`（**不传子集参数**，下载当次上游 `fonts.json` 列出的全部面并合并
仓库 `fonts/osifont.ttf`），再运行 `scripts/build-android.sh --release`。本地发布也必须按同一顺序，
否则 APK 可能没有 fonts 资源：

```bash
# 在网络可访问的主机运行；按需隔离网络代理，不改调用 shell 的代理配置
scripts/fetch-fonts.sh apps/app-android/assets/fonts
bash scripts/build-android.sh --release
python3 scripts/check-android-apk-fonts.py target/release/apk/yacr.apk \
  --fonts-dir apps/app-android/assets/fonts
```

校验器从本次准备的 `fonts.json` 动态读取所需文件（不固定远程字体数量/版本），核对 APK 内
`assets/fonts/fonts.json` 字节一致、每项字体非空且与准备目录 SHA-256 一致，并验证合并项
`osifont.ttf` 与仓库授权版本一致。契约测试：
`python3 scripts/test-check-android-apk-fonts.py`。仅构建/签名成功不代表字体已进入 APK；发布前
必须让上述包内验证通过。CI 流程与本地一致，但 CI artifact 也不等于安装/运行验收。

`apps/app-android/assets/fonts/` 的下载字体被 `.gitignore` 忽略，不提交二进制字体；准备出的清单
和每份字节应作为本轮构建的动态输入记录（例如 SHA-256、下载日志、校验输出），不要仅引用历史
清单 hash 或旧字体数量。`mlightcad/cad-data` 是第三方且未声明仓库级许可证，字体逐项授权和 APK
再分发许可由发布方负责核实；抓取成功或加入 CI artifact 不构成许可结论。仓库合并的 QCAD
`osifont.ttf` 为 GPL-3 + font exception，来源、哈希与许可见 `fonts/SOURCE.md`。
本轮已执行的无代理 release 构建、动态清单 hash、逐项 APK 字体验证与设备运行边界见
`docs/validation-android.md` §18；该记录也注明了尚待处理的代码门禁告警。

发布签名：`[package.metadata.android.signing.release]` 指向 `dev-release.jks`，
该文件被 `.gitignore` 的 `*.jks` 排除，需本地生成：

```bash
keytool -genkeypair -keystore apps/app-android/dev-release.jks -alias yacr \
  -keyalg RSA -keysize 2048 -validity 10000 -storepass yacrdev -keypass yacrdev \
  -dname "CN=yacr dev, O=yacr"
```

生产密钥绝不进入仓库（规范 §9.1）。注意：`min_sdk_version`/`target_sdk_version`
（`[package.metadata.android]` 顶层键，cargo-apk 0.10 只认嵌套的
`[package.metadata.android.sdk]` 表）**不生效**；Cargo.toml 已填实际生效值
minSdk 23 / targetSdk 30，与 `scripts/build-android.sh` 的 `d8 --min-api 23` 一致。
最低支持范围尚未在真机验证，不得作为兼容性声明。

仅 Rust 检查（不需要链接）：

```bash
cargo check --target aarch64-linux-android -p cad-ui-slint --locked
cargo check --target aarch64-linux-android -p app-android --locked
```

`min/target SDK` 与 ABI 是预留值，未在真机实测最低支持范围。发布签名密钥不入库，
不申请不必要的全盘权限。

**SAF 文件选择器（P4，2026-10-11）**：Android 打开/另存经 Storage Access Framework
（`ACTION_OPEN_DOCUMENT`/`ACTION_CREATE_DOCUMENT`）实现。因 `android-activity` 的原生
activity 栈收不到 `onActivityResult`，manifest 声明的启动 Activity 改为一个 Java
`NativeActivity` 子类 `dev.yacr.app.YacrNativeActivity`
（`apps/app-android/android/dev/yacr/app/YacrNativeActivity.java`，经
`[package.metadata.android.application.activity]` 指定，故**无需**改写编译后的二进制
manifest）；Rust 侧 `apps/app-android/src/saf.rs` 经 JNI 驱动它并用 `ContentResolver`
读写文档 URI。`scripts/build-android.sh` 在 `cargo apk build` 之后把该 Java 编译成
`classes.dex` 注入 APK 并重新对齐/签名（`javac`→`d8`→`zip`→`zipalign`→`apksigner`，
任一步失败即中止）。该后处理**强制**执行：manifest 声明了 `hasCode=true` 与
`YacrNativeActivity`，缺 `classes.dex` 的 APK 启动即 `ClassNotFoundException`，故没有
跳过开关。宿主目标（非 Android）仍回退到候选路径探测，`begin_save`/`write_atomic` 显式
`Unsupported`。真机 SAF 仍未验证（见 `docs/validation-android.md`）。

### x86_64 模拟器安装与运行（**宿主有 GPU 时用 `-gpu host`**；非真机）

评测对象：无头模拟器（`yacr35`，API 35，`google_apis`，x86_64）。**宿主存在真实显示/显卡时
必须用 `-gpu host`**（把宿主显卡透传给模拟器，其 GLES/Vulkan 走真实 GPU，见 AGENTS.md 契约）；
只有**确无 GPU** 的环境才回退 `-gpu swiftshader`（软件渲染，证据须显式标注）。APK 必须针对
**x86_64-linux-android**；aarch64 在 x86_64 镜像上经 NDK translation 安装但运行 abort。

```bash
# 0) 宿主有 GPU/显示时启动（无头 + 真实显卡；-read-only 便于多开）
emulator -avd yacr35 -no-window -gpu host -no-audio -no-boot-anim -no-snapshot -read-only &
adb wait-for-device
until [ "$(adb shell getprop sys.boot_completed | tr -d '\r')" = 1 ]; do sleep 2; done

# 1) 从当前 checkout 构建 x86_64 release APK（含 SAF 垫片后处理）
TARGET=x86_64-linux-android bash scripts/build-android.sh --release
#    等价直接命令（CARGO_TARGET_DIR 指向仓库外时更稳；不含 SAF 垫片后处理）：
#    cargo apk build -p app-android --target x86_64-linux-android --lib --release
#    产物：$CARGO_TARGET_DIR/release/apk/yacr.apk

# 2) 安装并启动（P4 起启动 Activity 是 SAF 垫片子类）
adb install -r -t target/release/apk/yacr.apk
adb shell am start -n dev.yacr.app/dev.yacr.app.YacrNativeActivity
adb shell pidof dev.yacr.app          # 进程存活即启动成功

# 3) 抓日志、截图与渲染器（确认是否走宿主 GPU）
adb shell dumpsys SurfaceFlinger | grep -m1 'GLES:'
adb logcat -d > /tmp/yacr-logcat.txt
adb exec-out screencap -p > /tmp/yacr.png
```

实测结果（完整证据见 `docs/validation-android.md`）：

- **宿主 GPU 运行（2026-10-11，`-gpu host`，本轮）**：`dumpsys SurfaceFlinger` 报
  `GLES: Google (Intel), Android Emulator OpenGL ES Translator (Mesa Intel(R) UHD Graphics
  (CML GT2)), OpenGL ES 3.0 (4.6 (Core Profile) Mesa 26.0.8-1ubuntu0.3)`（**宿主显卡，非
  SwiftShader**）；Slint + wgpu 初始化（Android Vulkan，guest 经 goldfish 转发到宿主 Mesa），
  内置演示几何渲染；SAF 打开/另存/New 三项 e2e 通过。证据
  `/tmp/opencode/yacr-android-host-gpu/`。
- **早前 SwiftShader 运行（软件 GPU，保留为历史记录）**：`-gpu swiftshader`；进程启动、Slint
  + wgpu 初始化，内置演示几何渲染；无崩溃/ANR。后端（设备日志原文）：
  `CAD renderer initialized: preference=WebGpu actual=WebGpu ... max_texture_dimension=8192`；
  底层 wgpu 走模拟器 SwiftShader；画布拖动平移、点按「适应」重定中心生效（像素 diff 非零）。
  **该轮的软件渲染数字/截图仅代表 SwiftShader，不代表真实 GPU**。
- **限制**：`safe_insets` 未被消费、surface 尺寸/旋转未回传（audit U07），顶部工具栏被系统
  状态栏遮挡；量测拾取未接线；图层/布局/批注面板状态未推送。**模拟器 ≠ 真机**，真机仍未验证
  （早前 SwiftShader 轮次中打开图纸为 NOT RUN；P4 后宿主 GPU 轮次已完成 SAF 打开/另存/New）。

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
