# 构建与平台入口

工具链 1.98.1（`rust-toolchain.toml`）、Cargo.lock 已锁定。
若 `PATH` 无 cargo，加入 `$HOME/.cargo/bin`。

## 纯核心

```bash
cargo test --workspace --exclude cad-ui-slint --exclude app-android --exclude app-web --locked
python3 scripts/check-architecture.py
```

`cad-ui-slint` 需要 Linux 宿主的 fontconfig/freetype 开发头与 pkg-config 才能为本机
编译；本环境无 sudo，故主机测试排除它，其编译由 Android target 覆盖。

## Android APK（已验证可编译打包）

依赖：JDK 17、Android SDK（build-tools 34.0.0、platform android-34/30）、
NDK 27.0.12077973、`cargo-apk 0.10.0`。

```bash
export ANDROID_HOME="$HOME/android-sdk"
export ANDROID_NDK_HOME="$ANDROID_SDK/ndk/27.0.12077973"
export JAVA_HOME="$HOME/jdk17"
export PATH="$HOME/.cargo/bin:$JAVA_HOME/bin:$PATH"

rustup target add aarch64-linux-android
cargo apk build -p app-android --target aarch64-linux-android --lib
# 产物：target/<profile>/apk/yacr.apk
```

仅 Rust 检查（不需要链接）：

```bash
cargo check --target aarch64-linux-android -p cad-ui-slint --locked
cargo check --target aarch64-linux-android -p app-android --locked
```

`min/target SDK` 与 ABI 是预留值，未在真机实测最低支持范围。发布签名密钥不入库，
不申请不必要的全盘权限。**当前 SAF 文件选择器未实现**：宿主按候选路径尝试打开
DWG，找不到时显示内置演示几何并明确提示（见 `app-android`），不能宣称 F01 已验收。

## Web（仅 Rust 编译路径）

```bash
cargo check --workspace --lib --target wasm32-unknown-unknown --locked
```

**无 Wasm 导出与最小 JS 宿主**，因此尚无浏览器可运行产物。后续需加 wasm-bindgen
导出、File API、Worker、IndexedDB/下载导出与静态打包脚本；部署需 application/wasm
MIME、HTTPS/安全上下文。COOP/COEP 仅可选共享内存路径需要。

## 桌面 / iOS

仅 `cad-platform` 抽象与 ADR；不建不可编译的虚假宿主。
