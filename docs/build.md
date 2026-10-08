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

## Linux release 包（含无头 render）

```bash
# 构建 + 打包（可选：对真实 DWG 出图并校验 PNG 非空）
VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json \
  YACR_TEST_DWG=/path/input.dwg scripts/package-linux-release.sh
```

产出 `target/release/dist/yacr-<version>-linux-<arch>.tar.gz`（及 `.sha256`），内含
release `bin/cad-cli-tools`、文档与 `scripts/{fetch-test-dwg,render-smoke}.sh`。
运行时的无头渲染用软件 Vulkan（Mesa lavapipe）时，先按发行版把 `VK_ICD_FILENAMES`
指向 `lvp_icd.json`；详见 `docs/headless-render.md` 与 `docs/validation.md`。

## Android APK（已验证编译、打包、安装与运行）

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

## 桌面 / iOS

仅 `cad-platform` 抽象与 ADR；不建不可编译的虚假宿主。
