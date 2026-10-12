# Android 画布双指缩放

共享 Slint 画布由 `ScaleRotateGestureHandler` 包住同几何尺寸的 `TouchArea`，使已 grab 的原始指针沿父链到达手势识别器。手势中心是画布局部 logical 坐标；Android backend 已完成 DPI 换算，调用链不再重复缩放坐标。旋转量被有意忽略，当前只实现缩放和中心锚定，不实现双指平移或 3D orbit。

Slint 的 `scale` 是每次手势从 1 开始的累计值。适配器把相邻更新换算为 `current / previous`，经既有 `ZoomAt` 命令漏斗提交。无效、非正数更新不提交。该公开 scale 值不承诺等于原始触点距离比，不能声称精确往返比例。

`ViewInput` 的 contact id 是平台中立整数，0 表示鼠标；手势生命周期同样由共享 trait 提供，非 Android 实现可忽略。共享 `CanvasTouchGuard` 在 pointer callback 中先于 `canvas-pick` 更新。它按 Slint 1.18.1 输入顺序记录首个 Down(A)；不同 ID 的合成 Up(B)（此序列没有 Down(B)）立即结束单指跟踪并进入等待状态。进入等待后，adapter 不会把被拒绝的 touch Down/Move/Up 原样送给 host，而是送 cancel；ID 0 的 mouse 输入不走此转换。这样即使禁用后重启配置但没有新的 gesture-started，host 也不会把尾指 Down/Up 解释为单指 tap。TouchArea 的 hover-loss/child Exit 单独通知为 `pointer-exited`，不会盲目清空状态：若 parent gesture handler 正在接管或等待 tail，就延迟处理；普通单指离开则取消该序列，匹配的后续 Up 仍不能 pick。残留指针的合成 Down/Up、cancel（包括 ID 0）或已归属的 gesture lifecycle 才结束等待。Android host 使用独立实例执行相同转移，抑制 tap-selection 并取消单指 drag bookkeeping。

纯契约测试覆盖真实 Down(A)/synthetic Up(B)（无 Down(B)）、不同 tail ID、child Exit 与 started/ended/tail 的交错顺序、轻点多指、ID 0 cancel、orphan Up 和下一次点击；Android 的端到端合成接线测试使用实际 `UiAdapter` + `AndroidViewInput` + `AndroidSink`，覆盖配置变更后无新 started 的残留尾指不会选中对象或提交测量点，且后续 touch tap 和 mouse 行为恢复。另有 adapter 测试覆盖 `interaction.pointer` 门控及禁用后的重启要求。Android 测试覆盖增量比例、中心锚定、非法 scale、无 start 更新、真实 metrics 提前失败后的 retry、失败 send 后 retry、真实 camera zoom 路径和测量点保留。Synthetic callback 测试不能模拟设备原生触摸分发或证明 TouchArea 原始事件的 parent bubbling，故仍不能替代真机验收。测量/绘制捕获仍只会在 `canvas-pick` 通过后接收点，导航不会清空既有捕获点或确认/提交工具。

Slint 接线/合成契约测试及编译不能证明 Android 原生触摸事件顺序、DPI 或设备体验。真机验证尚未执行，需由设备验收单独确认；尤其需验证第二指进入、残留指释放和取消后的下一次完整点击。

## 滚轮缩放锚点

滚轮回调现在将滚轮增量与 `TouchArea` 的 `mouse-x` / `mouse-y` 一起传递；坐标是画布局部 logical pixels，不拼接窗口偏移，也不再做 DPI 换算。Android/Web 保留既有 `1 - dy * 0.0015` 因子，桌面保留 `1 + dy * 0.0015` 的 winit 符号约定；三条路径均用该 cursor 发 `ZoomAt`。偏离画布中心的锚点有 Android、desktop HostCore 真实相机路径测试，UI adapter 测试验证 scroll callback 将 cursor 原值交给 `ViewInput`，Web 有 wheel payload 契约测试。

这些是合成接线与软件/合成相机验证，不是 Android 真机或桌面实体滚轮验收。Pinch 坐标与 renderer viewport 尺寸的匹配由另一 lane 只读核验/修复；本节不对 pinch 的实际视口锚点作额外结论。
