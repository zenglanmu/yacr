# ADR 0002：Android 宿主与 APK 打包组合

状态：采用（构建已验证，真机未验证）。范围：规范 §12.3 的首个跨端 GPU/UI 合成证据。

## 问题

`docs/validation.md` 与 `handoff.md` 一致记录：Android/Web 的 Slint + wgpu 组合未验证，
`cargo check --target aarch64-linux-android` 成功也不能等同于 APK 或真机验收。ADR 0001
把“优先补证并锁定版本”列为后续必须项。本次先把 Android 宿主的可编译、可打包证据固定下来。

## 证据（本次实测环境）

| 项 | 锁定值 |
|---|---|
| rustc/cargo | 1.98.1（`rust-toolchain.toml`） |
| Rust target | `aarch64-linux-android` |
| Slint | 1.18.1：`backend-android-activity-06` + `renderer-skia` + `unstable-wgpu-30` |
| android-activity / ndk | 0.6.1 / ndk 0.9（由 Slint 1.18.1 传递） |
| wgpu | 30.0.1 |
| JDK | Temurin 17.0.20.1（`JAVA_HOME`） |
| Android SDK | build-tools 34.0.0、platforms/android-34（`ANDROID_HOME`） |
| Android NDK | 27.0.12077973（`ANDROID_NDK_ROOT`） |
| 打包器 | cargo-apk 0.10.0 |
| Skia 预编译 | skia-bindings 0.153.3 的 android-aarch64 预编译（ganesh-gl-vulkan） |

## 选择

Android 渲染器是 Slint 的 Skia 后端，且在启用 `unstable-wgpu-30` 时由
`AndroidWindowAdapter` 固定选择 `SkiaRenderer::default_wgpu_30`（源码
`i-slint-backend-android-activity-1.18.1/androidwindowadapter.rs`）。因此 Android 上不存在
“选 wgpu 还是 skia”的分支：`unstable-wgpu-30` 即 Skia-on-wgpu。`app-android` 在
`slint::android::init` 前调用 `select_wgpu_backend()`，Android 分支下它只登记
`set_requested_graphics_api`，不自行安装平台。

APK 只打包 `aarch64-linux-android`：这是本次实际验证的 ABI，也是 Skia 预编译可用的 ABI。
`armv7-linux-androideabi`、`x86_64-linux-android` 仍是候选，未验证，不得凭 metadata 宣称支持。

宿主入口：`apps/app-android/src/lib.rs` 提供 `android_main`，安装平台后构建共享 Slint
外壳，并让 `CommandRouter` 把外壳回调转成 `cad-app` 命令。UI 只发命令，业务由应用执行。

## 候选与否决

- 用 `renderer-software` / femtovg：Android 后端只支持 Skia，选择器会直接报错；否决。
- 立即共享 Device/Queue：需要 `WGPUConfiguration::Manual` 和自有纹理桥，尚未实现；
  本次只记录请求，纹理桥在能力表中标记 `Unsupported`，不伪装完成。
- 全 ABI 打包：armv7/x86_64 的 Skia 预编译未验证，且在离线/受限网络下不可复现；本次否决。

## 影响与回退

- 影响：`docs/build.md` 增加已验证的 APK 打包命令；`apps/app-android` 不再是空占位。
- 回退：删除 `android_main`/`run`，恢复 `pending("host.android.activity_saf_composition")`
  即可回到 ADR 0001 状态；不影响核心 crate。
- 未验证项：真机/模拟器运行、GPU 纹理桥、SAF content URI、恢复存储；见
  `apps/app-android` 的 `CAPABILITIES`。

## 测试

- `cargo test -p app-android`：路由契约（Viewer 拒绝 Work 命令、OpenDrawing 归属宿主、
  图层覆盖不触库、能力表不宣称已真机验证）。
- 打包：`cargo apk build --target aarch64-linux-android --lib` 产出 aarch64 APK；
  构建证据记录在 `docs/validation.md`。
- 真机运行仍标记“未运行”。
