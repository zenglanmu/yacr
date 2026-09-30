# 构建与平台入口

当前可构建 Rust 契约 workspace 和 CLI，不能构建可用 CAD APK/网页。
工具链 1.98.1；Cargo.lock 固定已使用依赖。使用 README 命令。
若 PATH 无 cargo，添加 `$HOME/.cargo/bin`。

## Android（宿主骨架已接线，真机未验证）

已锁定的组合见 `docs/adr/0002-android-host-and-apk.md`：

| 项 | 值 |
|---|---|
| Rust target | `aarch64-linux-android`（唯一已验证 ABI） |
| JDK | 17（`JAVA_HOME`） |
| Android SDK | `build-tools;34.0.0`、`platforms;android-34`（`ANDROID_HOME`/`ANDROID_SDK_ROOT`） |
| Android NDK | 27.0.12077973（`ANDROID_NDK_ROOT`） |
| 打包器 | `cargo install cargo-apk`（本次 0.10.0） |

```bash
rustup target add aarch64-linux-android
export JAVA_HOME=/path/to/jdk17
export ANDROID_HOME=/path/to/android-sdk
export ANDROID_NDK_ROOT="$ANDROID_HOME/ndk/27.0.12077973"

# 只做 Rust 编译检查，不链接、不打包
cargo check --target aarch64-linux-android -p app-android --locked

# 打包 APK（debug，未签名发布；Skia/wgpu 全量链接，产物较大）
cargo apk build --target aarch64-linux-android --lib
# 产物：target/debug/apk/yacr.apk，内含 lib/arm64-v8a/libyacr.so
```

`apps/app-android` 现在提供 `android_main`，安装 Slint Android 后端并运行共享外壳；
命令由 `apps/app-android` 的 `CommandRouter` 转成 `cad-app` 命令。仍然未实现并显式标记为
`NotImplemented`/`Unsupported`（见该 crate 的 `CAPABILITIES`）：SAF content URI、
恢复存储、以及 CAD 渲染器与 Slint 之间共享 wgpu 纹理的桥。

`min_sdk_version`/`target_sdk_version` 仍是预留值；ABI 只声明 `aarch64-linux-android`，
其余 ABI 属候选，不得凭 metadata 宣称支持。发布签名密钥不入库，不申请不必要的全盘权限；
真机/模拟器运行属于单独验收项，不能由打包成功代替。

## Web（仅 Rust Wasm 编译路径）

```bash
cargo check --workspace --lib --target wasm32-unknown-unknown --locked
```

后续加 wasm-bindgen 导出、最小 JS、用户 File API、Worker、IndexedDB/下载导出和静态打包脚本。
部署设置 application/wasm MIME、HTTPS/安全上下文；COOP/COEP 仅可选共享内存路径需要。
无默认后端服务，无自动上传。不能把无 JS 导出的 cdylib 当作可运行网页。

iOS/desktop 当前只有 cad-platform 抽象，未建不可编译的虚假宿主。
