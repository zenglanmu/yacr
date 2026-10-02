# ADR 0002：Slint / wgpu 合成与 Android 宿主

状态：采用。范围：规范 §5.3 平台组合、§12.3 最小可编译证据。

## 问题

规范要求 CAD 用 wgpu 绘制、UI 用 Slint，并禁止每帧 GPU→CPU 回读。需要确定
Android 上「谁拥有 Surface / 谁提供 Device」、「如何把 CAD 纹理合成进 UI」，
并给出可编译证据。

## 证据

锁定版本：slint 1.18.1、wgpu 30.0.1、acadrust 0.5.5、cargo-apk 0.10.0、
NDK 27.0.12077973、JDK 17、Android SDK build-tools 34.0.0。

- Slint Android 后端（`i-slint-backend-android-activity`）固定使用 Skia 渲染器；
  打开 `unstable-wgpu-30` 时 `SkiaRenderer::default_wgpu_30` 使用 wgpu 作为底层，
  因而存在可共享的 wgpu Device/Queue。
- `slint::BackendSelector::require_wgpu_30(...)` 在 winit 与 Android 上生效。
- `slint::Window::set_rendering_notifier` 回调提供
  `GraphicsAPI::WGPU30 { device, queue, .. }`；`slint::Image::try_from(wgpu::Texture)`
  可把 CAD 离屏纹理交给 Slint 合成。
- 已用最小探针 crate（仅 Slint Android 后端 + `unstable-wgpu-30`）成功
  `cargo apk build` 产出可安装 APK（debug 258 MB，arm64-v8a）。

## 选择

Slint 作为唯一呈现协调者：Slint 拥有 Surface 与事件循环并提供共享
Device/Queue；`cad-render-wgpu` 仅创建派生资源（管线/buffer/离屏纹理），
在 `BeforeRendering` 中渲染到与 `Image::try_from` 兼容的纹理（
`Rgba8UnormSrgb`、`RENDER_ATTACHMENT|TEXTURE_BINDING`、alpha 使用预乘由 Slint
决定），由 Slint 合成。CAD 不创建第二套事件循环、不争抢 present、不做整幅回读。

## 影响

2026-10-02 复核：继续采用此组合，不迁移到独立 DOM UI / CAD Surface。
共享设备与回调合成不是场景控制耦合的理由；CPU scene controller、GPU runtime 和
Slint presenter 现已分离，职责、契约与剩余限制见 `../bridge-runtime.md`。

- 设备丢失：`Renderer::rebuild_device` 返回 GpuFailure，要求宿主提供新 Device；
  数据库 revision 不变（§18.1）。
- 纹理变化：以设备 epoch + texture revision + 尺寸决定重新导入 `Image`，
  不是只比较尺寸；内容更新不每帧创建 Image。
- 纹理格式/预乘/sRGB 已固定为上述组合，WebGL2 基础档另需 CPU 侧路径（未实现）。
- wgpu 30 描述符字段（`bind_group_layouts: &[Option<..>]`、`immediate_size`、
  `multiview_mask`、`buffers: &[Option<VertexBufferLayout>]`）与 wgpu 24 不同，
  升级需复验。

## 未决

- 真机/模拟器运行未执行（本环境无 adb/emulator），仅证明可编译并打包。
- 桌面宿主未编译：Linux 缺 fontconfig/freetype 开发头与 pkg-config，且无 sudo。
  规范允许桌面为预留宿主，故不提供桌面可执行产物。
- Web 双 Canvas/WebGL2 路径未实现。

## 回退

若 Skia+wgpu 组合在目标设备上失败：允许独立平台合成桥（Slint 渲染到纹理，
CAD 二次合成），但必须记录代价并按 §5.2 复测呈现顺序、alpha 与尺寸变化。
