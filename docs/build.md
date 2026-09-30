# 构建与平台入口

当前可构建 Rust 契约 workspace 和 CLI，不能构建可用 CAD APK/网页。
工具链 1.98.1；Cargo.lock 固定已使用依赖。使用 README 命令。
若 PATH 无 cargo，添加 `$HOME/.cargo/bin`。

## Android（宿主尚未实现）

```bash
rustup target add aarch64-linux-android
cargo check --target aarch64-linux-android -p cad-ui-slint --locked
```

这是 Rust 编译检查，不需要链接 APK。后续先固定 JDK/SDK/NDK/cargo-apk 和 Slint/wgpu，
补 android_main/Activity、SAF content URI、生命周期、GPU 合成、恢复存储再增加打包入口。
当前 metadata 的 min/target SDK 与 ABI 是预留值，不是实测最低支持范围。
发布签名密钥不入库，不申请不必要的全盘权限。

## Web（仅 Rust Wasm 编译路径）

```bash
cargo check --workspace --lib --target wasm32-unknown-unknown --locked
```

后续加 wasm-bindgen 导出、最小 JS、用户 File API、Worker、IndexedDB/下载导出和静态打包脚本。
部署设置 application/wasm MIME、HTTPS/安全上下文；COOP/COEP 仅可选共享内存路径需要。
无默认后端服务，无自动上传。不能把无 JS 导出的 cdylib 当作可运行网页。

iOS/desktop 当前只有 cad-platform 抽象，未建不可编译的虚假宿主。
