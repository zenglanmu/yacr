# Linux App：主验收宿主

2026-10-03 用户要求新增 Linux App，并将其设为优先验收标准，见规范 §1/§11.0。
不是复用 Android 宿主，也不是用 mock sink 的 UI 测试冒充应用。

## 构建与运行

```bash
sudo apt-get install -y pkgconf libfontconfig-dev libfreetype-dev mesa-vulkan-drivers
cargo build -p app-linux --bin yacr-linux --release --locked
target/release/yacr-linux                         # 已有桌面环境运行，不在无窗口 LXC 安装桌面
target/release/yacr-linux --open /absolute/file.dwg --locale en
bash scripts/check-linux-app.sh                   # 无窗口，release App + lavapipe
```

`--headless --output <新目录>` 运行同一个 LinuxApp/controller/Slint/共享设备 CAD 桥，只改变
平台适配器。`--size WIDTHxHEIGHT` 限制 320–4096；`--locale zh-CN|en`。脚本自动寻找 Mesa
lavapipe ICD，支持 `VK_ICD_FILENAMES`/`YACR_LINUX_OUTPUT` 覆盖；`YACR_TEST_DWG` 可输入仓库外图纸。

PNG：`initial.png`、`navigation.png`；报告：`report.json`。目录必须新建，失败保留，禁止覆盖。
报告中出图 smoke、导航命令+像素变化是自动检查，`visualAcceptance` 始终等待独立人工审查，
软件 GPU 成功不能推广为窗口系统、真实 GPU 或 DWG vendor 兼容。

## 已接线范围与限制

宿主通过 HostController 的命令/事务更新图纸，统一推送层、属性、布局、测量、批注、诊断与
显示派生；鼠标平移/滚轮缩放/选择/测量取点、直线/圆/移动、撤销重做使用真实应用路径。
启动 fit 使用 CAD 内容区域；运行时 50ms 定时器更新尺寸/相机，不重新解析底图或自动 fit。

- `--open PATH` 指定打开源；没有原生文件选择器，未指定路径的打开按钮显式 Unsupported。
- 打开当前是同步读取/导入，未实现 Linux 后台进度/取消，不能宣称 F01 全闭环。
- `--export-annotations PATH`、`--import-annotations PATH` 指定侧车；导出先写临时文件并
  sync/rename，成功后才确认数据库 revision。不能写入时不能标记已保存；导入严格拒绝指纹不匹配。
- 未保存批注时打开/关闭被阻止，无隐式 discard。尚无保存/恢复/丢弃决策对话框及恢复缓存。
- Trim 点选显式 Unsupported；第三方 CAD 字体加载、完整 ViewerConfig、原生多触控均未闭环。
- Linux 可执行文件不是静态独立发行包：运行仍需要系统库，当前 CI 上传二进制和证据，不宣称完整安装包。

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
