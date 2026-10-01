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

## Web（Wasm 静态产物 + 最小 JS 宿主）

```bash
scripts/build-web.sh                 # 产出 web-dist/（index.html、main.js、pkg/yacr.js、yacr_bg.wasm）
scripts/serve-web.py --directory web-dist --port 8090
# 浏览器打开 http://127.0.0.1:8090/
```

- 需要 wasm-bindgen-cli **0.2.129**（与 Cargo.lock 中的 wasm-bindgen 严格匹配）与
  `wasm32-unknown-unknown` target。
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
