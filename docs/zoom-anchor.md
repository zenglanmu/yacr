# Pinch 缩放锚点与画布尺寸

## 尺寸契约

有 UI handle 时，`HostCore::metrics` 使用 `UiHandle::cad_surface_size()` 提供的 CAD
区域 logical 尺寸和窗口实际 scale。渲染 bridge 也以同一 CAD 区域尺寸/scale 渲染。工具栏、手机
chrome、安全区已经由 UI 布局从 CAD 区域排除，不能再次从该尺寸扣除 inset。CAD 输入命令的
光标坐标是 canvas-local，因此对应 `CanvasMetrics` 的 origin 固定为 `[0, 0]`；窗口中的 CAD
区域位置不应再次参与锚点坐标换算。

没有 UI handle 的 headless 路径仍从平台 surface 读取尺寸、scale 和 safe inset，并通过共享
`inset_canvas_metrics` 计算画布区域。平台 surface 不存在仍返回 `Cancelled`；handle 存在但 CAD
尺寸不可用也取消，不回退为整窗尺寸。CAD 尺寸无效由 `apply_canvas_metrics` 显式拒绝。

即使 handle 提供了 CAD 区域，平台 surface 仍须通过 `inset_canvas_metrics` 的完整校验：逻辑尺寸
和 scale 必须有效，insets 必须有限、非负且不能耗尽区域。校验结果只用于判断平台数据有效，
不使用其 inset-adjusted 尺寸替代 live CAD 尺寸。此校验不能假定由 `SurfaceSource` 实现保证：
`AndroidSurface`/`WebSurface` 只筛选正逻辑尺寸，scale 与 insets 原样返回；`DesktopSurface` 也
直接转发 handle CAD 尺寸、scale 并填零 insets。

## 验证边界

`cad-host::zoom_anchor_tests` 是合成契约测试：构造包含非零窗口/CAD 偏移语义、高 DPI 与
phone chrome/inset 的 metrics，验证 live CAD logical 尺寸和渲染 physical 尺寸一致，并用明显
偏离中心的 canvas-local 点执行 ZoomAt，验证其下方世界点前后不变。另有 headless inset fallback、
无效 CAD 尺寸拒绝及 live CAD 路径中的平台 surface 零/NaN 尺寸和无效 scale 拒绝检查。

这些测试不创建真实 Slint UI、不依赖 GPU，也不是 Android 真机或设备 pinch 验收。它们证明
HostCore metrics 选择契约和相机锚点的合成行为；真机尺寸/触控事件及 GPU 渲染仍需独立验收。
