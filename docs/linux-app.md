# Linux App：主验收宿主

2026-10-03 用户要求新增 Linux App，并将其设为优先验收标准，见规范 §1/§11.0。
不是复用 Android 宿主，也不是用 mock sink 的 UI 测试冒充应用。

2026-10-04 用户调整默认门禁为 `cargo check -p app-linux --all-targets --locked`。
下列 release 构建、桌面/离屏运行仅为可选验证入口，不再是提交或 CI `linux-app` 必需步骤。

## 构建与运行

```bash
sudo apt-get install -y pkgconf libfontconfig-dev libfreetype-dev mesa-vulkan-drivers
cargo build -p app-linux --bin yacr-linux --release --locked
target/release/yacr-linux                         # 已有桌面环境运行，不在无窗口 LXC 安装桌面
target/release/yacr-linux --open /absolute/file.dwg --locale en
target/release/yacr-linux --open /absolute/file.dxf --font times=/absolute/fonts/times.shx
bash scripts/check-linux-app.sh                   # 无窗口，release App + lavapipe
bash scripts/verify-ui.sh                         # 无头 UI 调试循环（第二层，见 docs/verify-ui.md）
```

`--headless --output <新目录>` 运行同一个 LinuxApp/controller/Slint/共享设备 CAD 桥，只改变
平台适配器。`--size WIDTHxHEIGHT` 限制 320–4096；`--locale zh-CN|en`；`--gpu auto|high|low`
设置桌面 wgpu 适配器偏好（`auto`/`high` 优先独显、`low` 优先核显，等价 wgpu
`PowerPreference`；还可直接用 `WGPU_ADAPTER_NAME`/`WGPU_POWER_PREF`）。
脚本自动寻找 Mesa lavapipe ICD，支持 `VK_ICD_FILENAMES`/`YACR_LINUX_OUTPUT` 覆盖；
`YACR_TEST_DWG` 可输入仓库外图纸。

混合显卡说明：桌面**默认（`--gpu auto`）即优先独显**（双显卡默认选独立 GPU），
此时 `nvidia-smi` 应能看到本应用进程；`--gpu low` 才切到核显。偏好只是尽力而为：
无可达独显时回退到所选后端的首个适配器。Slint 会拒绝纯 CPU 适配器（llvmpipe 需
`SLINT_WGPU_CPU=1` 才会启用），所以能启动就意味着在用真实 GPU 适配器。

PNG：`initial.png`、`navigation.png`；报告：`report.json`。目录必须新建，失败保留，禁止覆盖。
报告中出图 smoke、导航命令+像素变化是自动检查，`visualAcceptance` 始终等待独立人工审查，
软件 GPU 成功不能推广为窗口系统、真实 GPU 或 DWG vendor 兼容。

## 已接线范围与限制

宿主通过 HostController 的命令/事务更新图纸，统一推送层、属性、布局、测量、批注、诊断与
显示派生；鼠标平移/滚轮缩放/选择/测量取点、直线/圆/移动、撤销重做使用真实应用路径。
启动 fit 使用 CAD 内容区域；运行时 50ms 定时器只在**窗口尺寸/配置 revision/相机
快照真正变化**时才刷新布局与请求重绘（2026-10-08 修复：此前定时器每帧无条件写
Slint 属性并触发 `request_redraw`，空闲视图以 ~20fps 重绘、单核 CPU 占用 ~40%）。
`CadView::apply_view_snapshot` 对相同快照直接跳过 `set_view_state`/`request_redraw`；
`Runtime::metrics` 缓存 `(物理尺寸, config revision)`，未变化不重新应用布局或序列化配置。
异步打开仍由 `poll_open` 每 tick 检查，不会被跳过。

- 桌面打开按钮调用 `xdg-desktop-portal` 的系统文件选择器，筛选 DWG/DXF（含大小写扩展名）；
  在独立线程等待选择，Slint 事件循环继续运行。需要桌面 session D-Bus、`xdg-desktop-portal`
  及桌面对应的 portal backend（如 KDE/GNOME/GTK）；不在无窗口开发环境安装桌面。
  取消保留当前图纸，服务失败显式提示并保留当前图纸，不伪装为取消或成功。
  `--open PATH` 仍为启动入口；后续桌面打开重新选择文件，不反复打开启动路径。
  无窗口模式不调用 portal，只接受 `--open PATH`。
- 打开当前是同步读取/导入，未实现 Linux 后台进度/取消，不能宣称 F01 全闭环。
- `--export-annotations PATH`、`--import-annotations PATH` 指定侧车；导出先写临时文件并
  sync/rename，成功后才确认数据库 revision。不能写入时不能标记已保存；导入严格拒绝指纹不匹配。
- 未保存批注时打开/关闭被阻止，无隐式 discard。尚无保存/恢复/丢弃决策对话框及恢复缓存。
- Trim 点选显式 Unsupported 且按钮禁用；第三方 CAD 字体可通过重复 `--font NAME=PATH`
  显式加载，目录匹配/恢复、原生多触控均未闭环。
- Linux 可执行文件不是静态独立发行包：运行仍需要系统库；默认 CI 仅上传 debug 编译日志，不产出安装包。

## 配置与用户偏好持久化（2026-10-03）

宿主启动时若存在配置文件即读取，随后按有效配置渲染；三个 overlay/interaction 门控由
状态漏斗在事件时读取有效配置执行（见 `docs/ui-redesign.md`）。

- 目录：`$XDG_CONFIG_HOME/yacr/`，未设置时回退 `$HOME/.config/yacr/`；两者都没有时
  完全不持久化（显式，不猜测路径）。
- `config.json`：完整宿主 `ViewerConfig`（`schemaVersion:1`，camelCase，未知字段拒绝），经
  `UiHandle::set_config_json` 应用；解析/校验失败保留默认并报告，不伪装成功。
- `preferences.json`：仅 `allowedPaths` 投影的用户偏好补丁；应用后把投影原子重写
  （与 Web `localStorage` 语义一致）。非法文件保留默认且不重写。
- 覆盖：`--config PATH`、`--preferences PATH`（优先级高于 XDG 默认）。
- `LinuxApp::apply_user_preference_json` 应用后原子持久化投影；原生 UI 尚无交互式偏好
  改写入囗（`UiHandle` 层已有），因此本轮只闭环“启动读取 + API 应用持久化 + 往返测试”。
  证据：`apps/app-linux/tests/host_config_disk.rs`（lavapipe 离屏，单测试进程）。


## 文件选择与关闭崩溃修复（2026-10-03）

- 用户 Wayland 桌面回溯表明关闭窗口在 `RenderingTeardown` 内调用 `set_cad_frame`，
  图片属性变更触发 `request_redraw`，与 winit 窗口的内部可变借用冲突。
  setup/teardown 现在只更新渲染器和绑定标记，不修改 Slint 图片属性；旧纹理由 Slint
  图片引用保活，下一次 `BeforeRendering` 按设备 epoch 重新绑定。未修改上游 Slint。
- 后端能力与诊断从共享 `wgpu::Device::adapter_info().backend` 获取，不再从请求的
  WebGPU 偏好推断。Linux Vulkan 实际显示 `vulkan`，原生 OpenGL 显示 `opengl`；
  不强行把所有 Linux 设备称为 Vulkan。定时状态同步刷新初始化后的诊断标签。
- 新增无需窗口的生命周期绑定失效契约，真实软件 Vulkan UI 断言，以及注入选择器的
  Linux 宿主 DXF 选择/取消/文件失败契约。注入选择器不是实际桌面 portal 验收。
- 本轮 Linux/Slint 针对性合成契约 **83 passed / 0 failed**（Linux 单元 1 + 宿主契约 2 +
  UI 单元 79 + 软件 Vulkan UI 集成 1），日志 `/tmp/opencode/yacr-linux-picker-tests.log`；
  fmt、严格 workspace clippy 与四项 Python 门禁通过（i18n 187 keys）。
  主机全量合成回归 **1103 passed / 0 failed / 1 ignored**，日志
  `/tmp/opencode/yacr-linux-picker-workspace-tests.log`；未设置 `YACR_TEST_DWG`，依赖该变量
  的真实 DWG 分支未执行，不能用总数当真实图纸证据。release/WASM 门禁仍在执行。
  首次编译发现旧 reset 调用与 portal title 参数不匹配，已修正；首次 clippy 发现测试模块
  位置不符合规则，已移至文件末尾并重跑，失败日志保留。
- 用户真实图纸路径在当前环境不存在。Wayland 关闭窗口、真实 GPU 与实际 portal
  交互未在无窗口环境运行，不能用合成/offscreen 测试代替。

## 执行证据

本轮 `cargo check -p app-linux` 已通过；Linux 宿主合成契约重跑 **2 passed**，
真实 UI 回调完成直线事务/撤销重做、测量与保存为批注、侧车导出回导，缺失路径不会空成功。
全 workspace WASM lib 隔离检查通过。首次测试编译发现回调名称不匹配；第一次全量回归
发现成功后残留旧错误状态，修正后重跑，失败记录未改为通过。
最终主机门禁（包含 Slint/Linux App）**1048 passed / 0 failed / 1 ignored**；fmt、严格
clippy、四项 Python 门禁、3 项工作流变异契约与全 workspace WASM lib 检查通过。
日志：`/tmp/opencode/yacr-linux-final-tests.log`。增加 resize 契约后首次读取尚未布局的矩形
失败，补充实际 snapshot 让 Slint 完成布局后重跑通过；不是删掉尺寸断言。

release 主验收：`YACR_LINUX_OUTPUT=/tmp/opencode/yacr-linux-app-final-retry bash scripts/check-linux-app.sh`
实际通过，软件 Vulkan llvmpipe / Mesa 26.0.8 / LLVM 21.1.8；1280×800，CAD 矩形
`[240,180,1040,500]`，2 个 CAD 帧，无渲染错误，导航改变相机及 4553 个画布像素。
截图抽查确认本合成样本完整出图；自动报告仍保留 `manual-review-required`，不推广为真实图纸
视觉验收。初次 release 冷编译 20m15s；最终重建一次 120s 超时，换新目录延长预算后
6m20s 编译并实际运行通过，旧记录不冒充最终证据。

GitHub workflow 已声明并通过本机结构/变异校验，远程 job 结果尚未取得。
桌面窗口、真实 GPU、本轮真实 DWG、Android/移动浏览器均 **NOT RUN**。
