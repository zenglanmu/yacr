# ADR-0059：`cad_set_view` 相机定位与 BIM 视图

- 状态：已接受（2026-11-25）
- 关联：ADR-0010（React/Linux 宿主）、ADR-0024（载体表面）、ADR-0042（BIM 语义）、
  ADR-0053（应用域）

## 背景

`cad_standard_view` 只能选七个命名方向，`cad_orbit` 只能增量环绕，`VPOINT` 只设视向，
`VIEW SAVE/RESTORE` 只能在**已有**相机上保存/恢复。BIM 界面需要两类此前没有的能力：

1. **绝对相机定位**：给定 `eye/target/up` 与投影，一次把相机放到调用方指定的位姿（脚本、
   测试、保存的视点、未来的插件/相机对话框）。这不能走 `cad_command`：名字漏斗不携带载荷。
2. **按模型边界拟合的 BIM 视图**：Revit 式的总览/平面/立面要框住**当前活动空间**，而 chrome
   从不接收模型 bounds。把拟合留在 UI 会复制相机约定；把它做成环视/标准视图会让“总览”变成
   “保持当前距离的等轴测”，在空图或小模型上不可用。

## 决策

### D1 一条 typed wire：`cad_set_view`

新增共享 typed 通道 `cad_set_view { preset?, camera? }`，**恰好一个**字段：

```json
{ "preset": "overview" }
```

```json
{
  "camera": {
    "eye": [0.0, -10.0, 10.0],
    "target": [0.0, 0.0, 0.0],
    "up": [0.0, 0.0, 1.0],
    "projection": { "kind": "perspective", "verticalFovRadians": 0.785398 },
    "mode": "3d"
  }
}
```

`projection` 是 internally tagged enum：

- `{ "kind": "perspective", "verticalFovRadians": … }`
- `{ "kind": "orthographic", "scale": … }`（世界单位 / 逻辑像素）

`mode` 只有 `"2d"` / `"3d"`。线协议、Tauri 参数和 wasm 参数是同一份 `serde` 形状。

### D2 预设由宿主拟合，绝对位姿原样应用

预设（`overview/plan/front/back/left/right`）解析为 `ViewPreset`，在 owner 线程用
`Application::active_space_rect` 取**与 `FIT` 相同的活动空间**：

- `plan`：`framing_rect` + `plan_camera` → 2D 正交俯视，`TwoD { saved: camera }`；
- 其余：`Camera::enter_3d` 得到 bounds 中心与拟合距离，再用该方向的标准 `eye_offset`/
  `up_hint` 放置相机 → 3D 透视。`overview` 是等轴测方向。

绝对相机用 `Camera::validate` 与 `Projection` 构造器校验；`2d` 必须正交、`3d` 必须透视
（当前 3D 渲染器只实现透视）。3D 从 2D 进入时保存**当前** 2D 相机；已在 3D 时沿用既有
`saved_2d`；2D 放置把相机同时作为 `saved`。**2D 是渲染器的平面路径**：`target.xy` 与正交
`scale` 是实际绘制状态，`eye/up` 仍进入相机/状态但不产生倾斜视图；需要任意朝向时用 `3d`。

### D3 校验在入队前，应用在 owner 线程

与 `cad_standard_view`/`cad_orbit` 同一条纪律：

- 共享 `parse_set_view` 先做内容校验（恰好一个字段、未知预设、非有限值、eye==target、
  投影范围、mode/projection 一致）；
- 然后过模态弹窗门、probe 门、有界队列（`CAD_SET_VIEW_CAP`）；
- 桌面把 `ViewPlacement` 推入队列，owner tick 调用 `CadSession::set_view`；
  `dom-canvas` 的 `DirectSink` 同步调用同一个 session 方法。两端拒绝字符串相同。

稳定拒绝：`view.set.request_invalid`、`view.set.unknown_preset`、`view.set.unknown_mode`、
`view.set.invalid_projection`、`view.set.invalid_camera`、
`view.set.mode_projection_mismatch`、`view.set.empty`，以及门禁自身的既有字符串。

### D4 视图历史：真实变化才记一步

一次成功且与当前相机/模式不同的放置记录**一个** `ZOOM PREVIOUS` 步骤；完全相同的放置是
no-op，不污染历史。它不写数据库、不升 revision、不产生 `ChangeSet`、不改工作平面。

### D5 载体：同一 API，能力由既有声明细分

- `native-below`（Wayland/Windows/macOS/Android）：3D/2D 预设都可用。
- `dom-canvas`（Linux X11/XWayland、Web）：`hostProfile.orbit === false` 已声明 2D-only。
  UI 用这个既有声明禁用五个 3D 预设（`view.orbit_unavailable`），**plan 仍可用**（2D 观测）。
  不新增第二套 carrier 能力表；API 本身不感知载体，UI 的共享准入负责不发送已声明不可用的观测。

### D6 BIM 界面：`BIM 视图`面板

BIM 的“视图”选项卡新增 `bim.views` 面板，六个真实 item 使用新的 `viewPreset` 派发族
（`RibbonItem` 没有 `command`/`standardView`/`action`）：总览、平面、前/后/左/右立面。
它们经 `runItem` → `shell.runSetView` → `cad_set_view`，与其它 item 共用禁用/拒绝/移动端抽屉逻辑。
没有命令行的 BIM 界面因此第一次有了“按模型拟合的视图”，而不是只能靠标准视图/ViewCube。

### D7 不进入插件 API

`cad_set_view` 是 IPC/宿主能力，不是 `cad-ui-core/api/plugin-api.json` 的操作。插件仍不能
读写相机；若未来开放，需要新的 descriptor 行与独立的能力声明，不能把本通道偷偷写成
`supported`。

## 后果

- 新增 `ViewPreset`/`ViewModeRequest`/`ViewPlacement`（`cad-app`）、`SetViewCameraArgs`
  （`cad-ui-core`）、`Channel::SetView` 队列与桌面/`dom-canvas` 两侧 drain。
- `COMMAND_TABLE` 增加 `SETVIEW → CommandId::SetView`：裸 `SETVIEW` 是**显式 payload 拒绝**，
  真正的入口只有 typed 通道（与 `STANDARDVIEW` 同一先例）。
- 文档、fixture、i18n 与现有 `FIT`/`ZOOM PREVIOUS`/标准视图契约继续各自独立。
- **未实现/边界**：Revit 式“视图对象”（图纸/视图树/视图样板/剖面框）、任意 FOV 的相机对话框、
  曲面/透视摄影机对象、每视图 V/G 覆盖都没有做；`plan` 是 2D 俯视，不是独立“楼层平面”对象。
  真实窗口/GPU/像素、真机与非 Linux 载体运行仍 **NOT VERIFIED**。
