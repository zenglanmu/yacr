# 渲染后端契约（未运行）

Auto/WebGPU/WebGL2 分开设置，UI 与 CAD 后端配置独立。Auto 必须实际尝试初始化并检查
features/limits；强制失败给用户原因/回退选项，不能只检测 navigator.gpu。
WebGL2 基础路径不依赖 compute/storage buffer/indirect draw；增强路径保持相同测量语义。

共享设备候选：Slint 作为唯一呈现协调者，CAD 同 Device/Queue 离屏纹理合成。
Web 受阻可验证双 Canvas 分区；不得每帧 GPU→CPU→GPU 整幅回读。
必须核查纹理 format/usage、MSAA resolve、alpha 预乘、sRGB、队列顺序、resize、device lost。

当前没有 Slint/wgpu 实际版本对齐、纹理桥、shader 编译/运行或平台验证。
Cargo 中未使用的候选版本不是兼容性证据；接入前用 ADR 锁定真实可编译组合。
切换后重建所有 GPU 派生资源，保持文档、相机与批注；失败重试有界，先走未保存保护。

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

- 没有真实适配器上的管线创建、深度精度、透明排序或镜像剔除的图像验证；
- WebGL2 下 `Depth32Float` 的可用性未在真实浏览器核对；
- 纹理桥、MSAA resolve、sRGB/预乘、resize 时序仍未实测；
- Auto 初始化失败的真实回退与 `BackendCapabilities` 运行时核对仍待真实设备。
