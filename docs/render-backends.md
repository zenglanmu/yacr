# 渲染后端契约

## 2026-10-03 共享设备诊断修正

`BackendCapabilities.actual` 与新增 `api` 来自共享设备的 `adapter_info().backend`，不来自
`BackendPreference`。原生 Vulkan/Metal/DX12/OpenGL 归入 `ActiveBackend::Native`，但诊断
分别显示 `vulkan`/`metal`/`dx12`/`opengl`；浏览器分别为 `webgpu`/`webgl2`，未知为
`unknown`。请求 WebGPU 不意味着原生 wgpu 实际使用浏览器 WebGPU。

Slint setup/teardown 回调只更新内部 GPU 状态与绑定标记，不写 UI 图片属性，不在窗口内部
借用期间触发重绘。`BeforeRendering` 按设备 epoch/纹理 revision 绑定当前图片。
本轮软件 Vulkan Slint 集成已核对原生类别及 `vulkan` 标签；用户 Wayland 关闭窗口/真实 GPU
未运行。最新宿主证据见 `docs/linux-app.md`、`docs/validation-web.md`；下文早期无宿主状态
是历史设计记录，不能覆盖这些分平台限定证据。

## 初期契约与历史状态

Auto/WebGPU/WebGL2 分开设置，UI 与 CAD 后端配置独立。Auto 必须实际尝试初始化并检查
features/limits；强制失败给用户原因/回退选项，不能只检测 navigator.gpu。
WebGL2 基础路径不依赖 compute/storage buffer/indirect draw；增强路径保持相同测量语义。

共享设备候选：Slint 作为唯一呈现协调者，CAD 同 Device/Queue 离屏纹理合成。
Web 受阻可验证双 Canvas 分区；不得每帧 GPU→CPU→GPU 整幅回读。
必须核查纹理 format/usage、MSAA resolve、alpha 预乘、sRGB、队列顺序、resize、device lost。

当前没有 Slint 宿主、纹理桥或 Android/浏览器平台验证。渲染器本身的管线创建、shader
编译与真实提交已在**原生无头软件适配器**（Mesa lavapipe，Vulkan）上执行，见
`docs/headless-render.md` 与 `docs/validation.md`；这**不**等于 Slint 共享设备、WebGL2
或真机 GPU 的验证。Cargo 中未使用的候选版本不是兼容性证据；接入前用 ADR 锁定真实
可编译组合。切换后重建所有 GPU 派生资源，保持文档、相机与批注；失败重试有界，先走
未保存保护。

## 管线与帧预算（F14，静态实现）

`cad-render-wgpu` 现在声明两条真实管线，见 `docs/render-3d.md`：

- `cad-lines-pipeline`：`LineList`，用于折线与网格线框叠加；
- `cad-mesh-pipeline` / `cad-mesh-pipeline-mirrored`：`TriangleList`，带深度缓冲、
  背面剔除；镜像（负行列式）批次切换到 `front_face = Cw` 的变体。

WGSL 以 `include_str!` 从 `crates/cad-render-wgpu/shaders/{line,mesh}.wgsl` 载入，
单源可校验。离线 naga 校验测试见 `crates/cad-render-wgpu/tests/wgsl_validation.rs`。

帧预算（`cad_scene::SceneBudget::max_vertices_per_frame` / `max_triangles_per_frame`）
在提交前由 `plan_from_gpu` 逐批累计，超限返回显式 `OverBudgetReason`（诊断码
`render.frame_over_budget`），不静默丢批。

## 设备丢失（F12，渲染器侧）

`Renderer::render` / `render_3d` 返回 `Result<FrameStats, RenderError>`，调用方可据
`RenderError::is_device_loss()` 区分：

- `DeviceLost`：设备丢失或后端失败；派生 GPU 资源已释放，宿主须重新提供设备并重传场景；
- `Frame`：单帧校验/参数错误，设备与批次仍有效；
- `NotInitialized`：未初始化或缺少派生资源。

宿主（Slint）观察到 `DeviceLostReason` 时调用 `Renderer::note_device_lost(...)`；
`Renderer::rebuild_device` 只拆除并置位，绝不带着丢失的批次静默重建。

## 仍缺失（未验证，禁止当作已完成）

- 无头软件适配器上的管线创建/深度/透明排序/镜像剔除已有像素级测试（`docs/headless-render.md`）；
  但这些**未**在真实 GPU、Android 或浏览器后端核对；
- WebGL2 下 `Depth32Float` 的可用性未在真实浏览器核对；
- 纹理桥、MSAA resolve、sRGB/预乘、resize 时序仍未实测；
- Auto 初始化失败的真实回退与 `BackendCapabilities` 运行时核对仍待真实设备及宿主接线；
- 无头提交的 `poll` 等待默认 1 秒面向交互式宿主，软件大帧需显式
  `Renderer::set_poll_timeout`（见 `docs/headless-render.md` §4.5）。
