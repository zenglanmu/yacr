# Android 模拟器安装与运行验证（x86_64）

> **历史执行证据（Slint 桌面已退役，2026-10-16）**：本文记录的是**迁移前** Slint 宿主/模拟器/浏览器
> 的运行与验收证据，**不代表**当前 React/Tauri 宿主的编译、测试或真实 GPU 结论；Slint→React 迁移后的
> 最终验证 **NOT RUN**（P6）。当前架构见 `docs/architecture.md` 与
> [ADR 0010](adr/0010-react-linux-host-replaces-slint.md)。下列事实仅作历史记录，不得据此宣称当前平台可用。

本文件记录 **实际执行过** 的 Android 构建、安装、启动与交互验证。所有结论均来自本机
模拟器的 logcat、截图与像素 diff；未观察到的能力一律标注 **NOT RUN** 或
**限制**，不作成功声明。

> **环境标注**：§1–§10 为**早前的软件 GPU（SwiftShader）环境**记录（`dev_api35`，API 35，
> `-gpu swiftshader`，KVM）；其中的数字/截图仅代表 SwiftShader，**不代表真实 GPU**。
> §11（2026-10-11）为**宿主真实 GPU（`-gpu host`）**环境记录。两轮都是 Android 模拟器，
> **不是真机**，不能据此声明真机兼容性。

关联文档：`docs/build.md`（Android 小节）、`docs/validation.md`（总表）、
`docs/render-backends.md`。

## 16. 真机 arm64 release 启动、SAF 与手势初测（2026-10-12；交互/性能仍待补测）

- 已在 §15 构建的同一 arm64 release 上重试安装：`adb -s ebfb4268 install -r -t
  target/release/apk/yacr.apk` → `Success`（约 7 s）。未卸载/清数据。以显式组件
  `dev.yacr.app/dev.yacr.app.YacrNativeActivity` 启动 → `am start -W` 状态 `ok`、冷启动
  385 ms；PID `18776` 存活。证据目录：`/tmp/opencode/yacr-android-device-20261012-010448/`。
- 设备：Xiaomi `23046RP50C`，Android 15/API 35，arm64-v8a，2880×1800 横屏截图；硬件
  Adreno 730。应用 PID 日志含 Qualcomm Vulkan 驱动 `/vendor/lib64/hw/vulkan.adreno.so`、
  `Application Name: wgpu` / `Engine Name: wgpu-hal`；应用诊断面板显示渲染后端 `vulkan`。
  启动画面确有内置演示几何输出，故本轮有真机硬件 Vulkan 启动/出图 smoke 证据；不据此作跨机型、
  稳定性或视觉正确性结论。日志同时有 `SURFACE_VIEW_FORMATS` downlevel 警告以及 32 次
  4×4 `GraphicBufferAllocator` format 探测失败记录，截图仍可取得；这些日志不等于已定位根因。
- 只使用 manifest 已授权合成 fixture `fixtures/dwg/synthetic-four-lines.dwg`（SHA-256
  `68637e120e71e471eccb5cfe593a2da156444b8ef2e8669bc4f521ce32c38135`；清单注明 4 个 LINE）。
  复制到本轮唯一目录 `/sdcard/Download/YacrTest-20261012-010448/`，通过应用“文件→打开图纸”
  的 SAF DocumentsUI 选中并返回。标题变为 `synthetic-four-lines.dwg`，当前空间标记
  `*Paper_Space`；诊断面板报告 warning `import.annotation_scale_unknown`。点“适应”后纸空间截图仍看不到
  图形。故 SAF 文件选择/回调有证据，**不能**把该次内容导入/几何显示/完整性判作通过，需在 model
  space 与诊断实体快照可见后复核。未打开其他用户文件。
- 手势初测（当前屏幕操作均为 `adb shell input`，不是物理触摸，也不是 pinch）：在内置 demo 上单指
  swipe `1050,900→1650,900` / 800 ms 后 canvas crop 差异 11,227/3,552,320 pixels，截图几何明显平移；
  另一次 `1200,900→1650,900` / 500 ms 的 crop 差异为 0。Zoom-in/out 工具按钮造成视图截图差异，
  其中 zoom-in 对比 433,064 canvas pixels；fit 返回近似基准。两秒等待后截屏仅是终态采样，没测帧间
  连续性、单次输入延迟、掉帧或流畅度。**双指缩放未测**：本轮 ADB `input` 的可用手势为单指
  `tap/swipe/motionevent`，不支持多指 pointer；中途设备自动休眠，唤醒后到锁屏，未绕过锁屏继续输入。
  用户另报“无双指缩放、拖动/缩放明显卡顿”，记为待复现实机问题，不是本轮量化结论。
- 性能数据边界：`dumpsys gfxinfo dev.yacr.app framestats` 显示 `Total frames rendered: 0`、
  `Pipeline=Skia (OpenGL)`，统计对象是 Java/HWUI；不覆盖 Slint NativeActivity 内的 wgpu Vulkan
  canvas，不能用它的 `Janky frames: 0` 声称画布流畅。当前 fixture manifest 给出 4 条线，实际 app
  entity/import-report 数量未取得。只读定位 lane 报告 `view.rs` 当前 move contacts=1 / canvas
  `TouchArea` 无第二指数据；这解释了双指能力缺口的代码现状，但本条仍是 lane 报告，不是设备 pinch 测试。
  其提出的 `HostCore::push` 为每次 Pan/Zoom 重复派生面板状态仅是性能候选，尚未确证。
- app PID logcat 截取在 `logcat-app-start.txt`、`logcat-app-interactions.txt`、
  `logcat-app-final.txt`。本轮 app-PID 日志未见 `FATAL EXCEPTION`、`ANR in dev.yacr` 或 panic；
  日志包含 vendor `MWDF parse error`（`No such file or directory` / `Timer expired`）及上述 gralloc
  探测失败。无崩溃不等于流畅/无缺陷。未完成 DWG 视觉验收、双指 pinch、连续性能 trace、SAF 保存、
  测量/批注交互；用户无需现在解锁前不再继续触摸输入。

## 17. pinch/navigation 修订版 arm64 release 部署（2026-10-12；等待解锁后交互验收）

- 构建前记录 dirty source：HEAD `96d631fe8dcaf6658ebee850008ad2f55034d39f`；production-source
  diff（Android view、cad-host/UI 实现与 Slint UI，排除测试）的 SHA-256 为
  `fd78297af07d00f64d6ff9aee3df2d9ace3608e6ef257c196eae4cba3a061ef3`，构建后相同；无 untracked
  production source。构建前后完整 tracked diff 和 untracked 指纹也未变化。因此本次构建期间没有
  production/test/docs 源码漂移；不是仅以 HEAD 指代 dirty source。
- 无代理执行 `scripts/build-android.sh --release`（仅子 shell 清除全部大小写 proxy 与
  `ANDROID_BUILD_HTTPS_PROXY`；Android SDK、JDK 17、NDK 27，target `aarch64-linux-android`），退出码 0。
  APK `target/release/apk/yacr.apk`，mtime `2026-10-12 02:59:07`，15,599,082 bytes，SHA-256
  `8b23e45c984a1c47fa2650e6e12a56f1b50e4915cb1affebe5b3be87941e4061`；包 `dev.yacr.app`，
  arm64-v8a；验签通过，证书 SHA-256 `1a3a89ec7cf692806b5765e574e14c24d89fc2ab3957eb98d36555ddb88d1078`。
  `classes.dex` 含 `YacrNativeActivity.onActivityResult/readUri/writeUri`；manifest `hasCode=true`
  且 launcher activity 正确。完整构建/制品证据：`/tmp/opencode/yacr-android-acceptance-20261012-025646/`。
- 仅目标真机 `ebfb4268`（Xiaomi `23046RP50C`，Android 15/API 35）执行
  `adb -s ebfb4268 install -r -t target/release/apk/yacr.apk`，结果 `Success`，保留数据；未卸载、
  清数据、操作模拟器或读取用户文件。启动 intent 返回 `Status: ok`，PID `20719`。PID logcat 显示
  `yacr android host starting`，Qualcomm `/vendor/lib64/hw/vulkan.adreno.so`，`Application Name: wgpu`、
  `Engine Name: wgpu-hal`；可证硬件 Vulkan 初始化路径已调用，但不能替代可见帧/出图验收。
- 启动时设备已休眠：截图 `01-launch.png` 为黑屏；只发送 `KEYCODE_WAKEUP` 后为 Android 锁屏
  `02-wake-screen.png`，没有绕锁。应用仍有 PID，但当前无法确认处于前台，尚无新版 UI 截图、可见渲染帧、
  model-space fixture 内容、单指平移/按钮缩放或真实双指 pinch 的设备结果。**等待用户解锁并把 Yacr
  留在前台**；不要据合成 callback、软件 pinch 或 `gfxinfo` 代替真机手势证据。
- 已把仅有 manifest 授权的四条 LINE 合成样本复制到
  `/sdcard/Download/YacrTest-20261012-025646/synthetic-four-lines.dwg`；设备端与 repo fixture SHA-256
  都为 `68637e120e71e471eccb5cfe593a2da156444b8ef2e8669bc4f521ce32c38135`。后续可在 app 中经 SAF
  打开该唯一样本，并切至 Model/模型空间后核实实体确实可见，再进行单指 pan、现有 zoom control 与
  物理双指 pinch in/out。不要打开其他文件；操作期间保持 app 前台。ADB `input swipe` 是单指，不能声称
  覆盖 pinch；若无受支持的多点输入硬件/工具，改请用户在真机屏上操作并记录其动作。
- `dumpsys gfxinfo dev.yacr.app framestats` 为 `Total frames rendered: 0`，`Pipeline=Skia (OpenGL)`；
  是 Java/HWUI 统计，不覆盖 Slint NativeActivity 的 wgpu Vulkan canvas，0 帧/0 jank 不能证明流畅。
  本次没取得适用 canvas 的帧时间/性能 trace；性能与主观卡顿均待现场观察。设备 PID log、启动截图、
  fixture hash 和 APK 检查均在上述证据目录。

## 18. wheel/metrics/font 合并版验证与含全字体 APK（2026-10-12；设备画面仍待解锁）

- dirty-source 指纹：HEAD `96d631fe8dcaf6658ebee850008ad2f55034d39f`。包含 Android/UI/host/web
  生产实现的 tracked diff SHA-256 `51b59eeb92d2057d06b96212f3d2bedf80e1f808cab9fe70d5686698e718bc65`，
  构建前后完全相同；无 untracked production file。cad-host `zoom_anchor_tests.rs` 为未跟踪测试文件，
  与 tracked test-only diff 分开留指纹。因此 APK 可追溯到当时 dirty production source，而不是仅凭 HEAD。
- **门禁**：`cargo fmt --all -- --check` 未通过（Rustfmt 对 writer 的 Android tests、cad-host 新测试、
  UI implementation/tests 中 import 排序/断言折行等提出 diff）；未执行自动格式化或改源码。
  `cargo clippy --workspace --exclude app-android --exclude app-web --all-targets --locked -- -D warnings`
  未通过：`crates/cad-host/src/input.rs:150` 的 `wheel_anchor_tests` 位于后续 production impl 之前，触发
  `clippy::items_after_test_module`；未移动/改源码。Architecture、fixture-manifest、workflow、i18n
  检查通过；`cargo check --workspace --exclude app-android --exclude app-web --all-targets --locked`
  与 `cargo check --workspace --lib --target wasm32-unknown-unknown --locked` 通过。
- **契约测试**：`cargo test -p cad-host --locked`：22 passed，含 live zoom anchor metrics 与 desktop wheel
  anchor/sign tests。`cargo test -p app-android --lib --locked`：25 passed / 1 failed；唯一失败的
  `adapter_suppression_cancels_android_tail_and_restores_next_tap_and_tool_path` 是 Slint offscreen 检查，
  明确拒绝真实 NVIDIA Quadro P620 adapter（要求 software Vulkan）。依仓库真实 GPU 规则未切换软件渲染重跑，
  也不把该结果当真实设备失败。隔离相关用例通过：Android pinch 4/4、pan 1/1、scroll zoom 1/1；字体 APK
  校验器契约测试 8/8。未重复 UI 软件渲染/GPU lane。
- **全字体准备**：按 CI 无子集参数运行 `scripts/fetch-fonts.sh apps/app-android/assets/fonts`。
  无代理首次因 CDN 与 fallback TCP connection reset 失败；仅在 fetch 子进程使用先前用户明确指定的
  `http(s)_proxy=http://192.168.8.1:10809` 后成功，未修改调用者代理/User-Agent 配置。上游当次 99 项，合并
  仓库 QCAD `osifont.ttf` 后 `fonts.json` 为 100 个 unique entries。清单 SHA-256
  `48c10ddb5187e7b389c2e2ed5262a24692ea2a6d0a59872d4e816011d333b5f0`；字体总字节 `45,231,421`；
  osifont SHA-256 与 `fonts/SOURCE.md` 相同（`247ba237597538574a72e2ffe0c03087f1a35099bfff15a79a5fbe3549a258b9`）。
  第三方 mlightcad 字体逐项再分发许可仍由发布方核实，抓取成功不等于授权结论。资源保持 gitignored，未提交。
- **构建与包内门禁**：无代理子 shell 使用 SDK `$HOME/android-sdk`、JDK `$HOME/jdk17`、NDK
  `27.0.12077973`，执行 `scripts/build-android.sh --release`，退出 0。APK
  `target/release/apk/yacr.apk`，mtime `2026-10-12 08:18:50`，54,481,267 bytes，SHA-256
  `8bedbf51399fafac0df7d80bb21388dcc699186b474b8e4a8cac56f8d3593e5f`；package `dev.yacr.app`，
  `arm64-v8a`。签名验证通过（证书 SHA-256 `1a3a89ec7cf692806b5765e574e14c24d89fc2ab3957eb98d36555ddb88d1078`），
  `classes.dex` 的 SAF `readUri/writeUri/onActivityResult` 与 launcher manifest、`hasCode=true` 均核验通过。
  实际运行 `python3 scripts/check-android-apk-fonts.py target/release/apk/yacr.apk --fonts-dir apps/app-android/assets/fonts`，
  通过：100 项/45,231,421 bytes；包内 fonts.json 与准备清单逐字节一致，每字体非空且逐项 SHA-256 与准备资产一致，
  osifont 与仓库文件一致。它不是固定 100 项断言。APK 未因 Clippy/Rustfmt 告警而改写源码。
- **真机安装/启动**：仅 serial `ebfb4268`（Xiaomi `23046RP50C`，Android 15/API 35，arm64）执行
  `adb install -r -t`，成功；未卸载/清数据/读用户文件或操作模拟器。组件 `am start -W` 返回 `Status: ok`，
  app PID `24137`。但设备启动时休眠锁屏，黑屏截图；app logcat 明确记录
  `yacr android host failed: InvalidInput("canvas metrics are degenerate (size/DPI must be finite and positive)")`。
  唤醒只显示系统锁屏，未绕过安全设置；因此**不能**声称有可见 app frame、硬件 Vulkan/wgpu 初始化、字体运行时加载或
  渲染成功。logcat 仅观察到 Adreno GLES/HWUI 库初始化，不是 CAD canvas GPU 证据。`gfxinfo` 0 frame、Skia/OpenGL
  只覆盖 HWUI，不能用于评估 Vulkan canvas 或“流畅”。未取得可解释的 app frame 性能数据。
- **交互待现场完成**：唯一授权四条 LINE fixture 已复制到
  `/sdcard/Download/YacrFinal-20261012-081219/synthetic-four-lines.dwg`（hash
  `68637e120e71e471eccb5cfe593a2da156444b8ef2e8669bc4f521ce32c38135`），尚未 SAF 打开。解锁后请从 launcher
  打开 Yacr，在“文件→打开图纸”只选此 fixture，切到 Model/模型空间并确认几何可见，再试单指 pan 和 zoom controls。
  物理双指 pinch 与实体鼠标/触控板 wheel 需用户实际操作；ADB 单指 swipe 和合成 callback 均不能代替 raw gesture。
  尚未测 pinch、wheel、单指 pan/zoom 运行效果、fixture 几何、字体实际加载/可视性。用户报告的卡顿目前仅有
  减少重复 work 的实现，不等于已量化或解决；需分开记录主观响应与适用 canvas 帧时间。
- 证据目录：`/tmp/opencode/yacr-android-final-20261012-081219/`。此轮无提交。

## 19. wheel/metrics 统一门禁修复（2026-10-12；未重建/未操作设备）

- 先核对工作树；按 `cargo fmt --all -- --check` 的实际报告，仅对其中属于本轮 writer 变更的 Rust 文件运行
  `rustfmt --edition 2021`。只做了 import 排序/换行/断言排版，不格式化未报告的陌生 Rust 文件。
  `crates/cad-host/src/input.rs` 的 `#[cfg(test)] mod wheel_anchor_tests` 原样移至所有 production impl 之后的
  文件末尾，仅调整测试模块位置；未加 lint allow，也未改生产逻辑。
- 门禁复跑：`cargo fmt --all -- --check` 通过；
  `cargo clippy --workspace --exclude app-android --exclude app-web --all-targets --locked -- -D warnings` 通过；
  `cargo test -p cad-host --locked` 22/22 通过（含 zoom-anchor metrics 与 desktop wheel anchor）。
  `check-architecture.py`、`check-fixture-manifest.py`、`check-workflows.py`、`check-i18n.py` 均通过，字体包契约
  8/8 通过。上一轮 native/wasm `cargo check` 结果复用：此次仅为 Rustfmt 与 test module relocation。
- Android 全 suite 上一轮为 25/26：唯一失败的 Slint offscreen 用例因本机真实 Quadro P620 adapter 而拒绝运行，
  测试要求软件 Vulkan。没有把它记成通过，也未设置软件 ICD 重跑；上一轮 Android pinch/pan/scroll focused tests
  分别 4/4、1/1、1/1 通过。未重复该 UI/GPU lane。
- 当前 dirty production-source diff fingerprint 为
  `a0159bf87b4c92138bc6a1c1d9f05debfb58e835f65bfd12f8d9e257cc2f9922`；旧指纹
  `51b59eeb92d2057d06b96212f3d2bedf80e1f808cab9fe70d5686698e718bc65` 是上一版尚未格式化/测试模块尚未移动的 dirty
  source。diff 中这轮只包含 formatter 的纯排版以及 wheel 测试模块搬移。无 untracked production file；
  `crates/cad-host/src/zoom_anchor_tests.rs` 仍是独立 untracked 测试文件。
- 按用户指示本轮**不构建、不安装、不 adb 操作**。设备上已有含 100 字体项的 APK，SHA-256
  `8bedbf51399fafac0df7d80bb21388dcc699186b474b8e4a8cac56f8d3593e5f`，其构建对应的是旧 production fingerprint
  `51b59eeb92d2057d06b96212f3d2bedf80e1f808cab9fe70d5686698e718bc65`，**不是**当前新源码 fingerprint；不得将
  旧装机包声称为本次格式/测试布局后源码重新构建。字体清单 100 项及上轮包内逐字节校验事实保持不变。
- 本轮证据：`/tmp/opencode/yacr-gate-fix-20261012-083111/`。无提交。

## 20. Offscreen adapter 优先硬件并复验 Android UI suite（2026-10-12）

- 根因：上轮唯一 Android lib suite 失败用例
  `tests::adapter_suppression_cancels_android_tail_and_restores_next_tap_and_tool_path` 调用
  `cad_ui_slint::offscreen::install()`；offscreen helper 原本强制要求 `device_type == "cpu"`，因此在真实
  Quadro P620/Vulkan 上拒绝初始化。它错误地把 offscreen validation 限定为 software Vulkan。
- 限定修改 `crates/cad-ui-slint/src/offscreen.rs`：先用 `GpuSelection::HighPerformance` 选择 Vulkan
  discrete GPU；若结果不是 hardware 类，再探测 `LowPower`，优先 integrated GPU；只有两次选择都没有可用
  hardware 类型时接受 CPU adapter 作为 software fallback。仍强制 backend 为 Vulkan，继续创建实际的
  `FemtoVGWGPURenderer`；不以 adapter 缺失/错误折成空成功。初始化日志真实记录 `class/name/device_type/backend`；
  CPU 会明确打印 software fallback。没有设置 software ICD，也未改生产 renderer/pinch/zoom/font 逻辑。
- 当前主机可见 NVIDIA Quadro P620（`nvidia-smi`）；没有 `VK_*`/`WGPU_*` 环境覆盖。focused test 实际输出：
  `class=hardware, name=Quadro P620, device_type=discrete_gpu, backend=vulkan`。
  `cargo test -p app-android --lib adapter_suppression_cancels_android_tail_and_restores_next_tap_and_tool_path --locked -- --nocapture --test-threads=1`：1/1 passed。
- 随后未设置 `VK_ICD_FILENAMES`，串行运行
  `cargo test -p app-android --lib --locked -- --test-threads=1 --nocapture`：**26/26 passed**，该用例再次记录同一硬件
  adapter。该 headless test 证明 Vulkan adapter/Slint renderer 初始化及 adapter callback 契约，不是有窗口的桌面、
  Android 真机 GPU、真实帧视觉验收或性能流畅度结论。
- `cargo fmt --all -- --check` 与 workspace Clippy `-D warnings` 通过；无 allow 抑制、无新依赖。
  未构建 APK、未使用 adb/模拟器、未安装或打断用户设备。前述旧 APK / 字体事实不变。
- 证据目录：`/tmp/opencode/yacr-offscreen-hardware-20261012-084758/`；无提交。

## 21. Linux HostCore 轻量导航真实窗口 bridge acceptance（2026-10-12）

- 为核实新增的 ignored real-window 测试 `host::validation::tests::real_gpu_host_navigation_push_updates_bridge_and_preserves_full_push_semantics`，在本机 Wayland/X11 桌面及 NVIDIA Quadro P620 上以独立测试进程运行；未设置 `VK_*`/`WGPU_*` GPU 选择覆盖。命令：
  `YACR_NAVIGATION_OUTPUT=/tmp/opencode/yacr-git-push-20261012-091936/navigation cargo test -p app-linux --lib real_gpu_host_navigation_push_updates_bridge_and_preserves_full_push_semantics --locked -- --ignored --test-threads=1 --nocapture`，外层 150 秒 timeout；1/1 passed，运行 15.70 秒。
- `renderer.txt` 记录 `backend=Some(Native)`、`api=Some("vulkan")`、`frames_rendered=1`、`nvidia_device_fd=true`；`host-navigation.txt` 记录测试断言的 navigation push、selection/measurement/snap hints 与 full push 保持语义。Slint timer 每 50ms 检查 renderer readiness 并于 90 秒 deadline 失败或成功时退出；另有外层 timeout。证据：`/tmp/opencode/yacr-git-push-20261012-091936/`。
- 这是 Linux 真窗口下 HostCore/bridge 及活动 renderer 的测试证据，不是 CAD 视觉正确性、桌面流畅度、Android 真机渲染或性能结论。未构建 APK、adb 或操作手机。

## 15. arm64 release 真机构建与安装尝试（2026-10-12；等待设备授权）

- 在 checkout `96d631fe8dcaf6658ebee850008ad2f55034d39f` 上由本轮实际执行
  `scripts/build-android.sh --release`；显式清除 `ANDROID_BUILD_HTTPS_PROXY` 及大小写全部
  http/https/all/no proxy 变量。两个 Android `cargo check` 通过，release 编译/链接约 1m55s，
  SAF `classes.dex` 注入、zipalign、apksigner 与 manifest 检查完成，脚本退出 0。
- APK `target/release/apk/yacr.apk`，mtime `2026-10-12 00:59:15`，15,586,794 bytes，SHA-256
  `f019be1e74885136de79e26867149737def3df94828349039f1603c0230e1121`；包名
  `dev.yacr.app`，native-code `arm64-v8a`，min/target SDK 23/30。签名验证通过：
  `CN=yacr dev, O=yacr`，证书 SHA-256 `1a3a89ec7cf692806b5765e574e14c24d89fc2ab3957eb98d36555ddb88d1078`。
  ZIP 含 `classes.dex` 与 `lib/arm64-v8a/libapp_android.so`；manifest 有
  `YacrNativeActivity`、`hasCode=true`。本轮 build 与工作树信息见证据目录。
- 目标仍为 Xiaomi `23046RP50C` / serial `ebfb4268` / Android 15/API 35 / arm64-v8a。
  `adb -s ebfb4268 install -r -t .../target/release/apk/yacr.apk` 返回
  `INSTALL_FAILED_USER_RESTRICTED: Install canceled by user`。安装后复查 `pm path` 与 `pidof`
  均无 `dev.yacr.app`；未启动、未卸载、未清数据，未读写用户数据。
- 因设备限制，启动、UI/触摸、DWG/缩放、应用 logcat、截图、SAF 与视觉验收均未执行；
  SurfaceFlinger 的 Adreno/Vulkan 系统能力不能代替应用 GPU 使用证据。请解锁设备并在系统/开发者选项
  允许 USB 安装（若出现确认对话框请点允许/安装），再重试保留数据的 `adb install -r`。
- 证据目录：`/tmp/opencode/yacr-android-device-20261012-005711/`，含 build/install 日志、APK
  元数据与安装后状态。

## 14. 用户报告构建成功后的 APK 核验（2026-10-12；安装被 release 制品阻塞）

- 用户报告手动运行 `build-android.sh` 成功；本轮未执行构建。checkout `HEAD` 为
  `96d631fe8dcaf6658ebee850008ad2f55034d39f`，APK 元数据不能证明产物对应该 checkout。
- adb 目标仍是唯一在线设备 `ebfb4268`：Xiaomi `23046RP50C`，Android 15/API 35，arm64-v8a；
  SurfaceFlinger 报 Adreno 730 / OpenGL ES 3.2，系统声明 Vulkan 能力。设备上未安装
  `dev.yacr.app`。GPU 能力枚举不是应用渲染证据。
- `target/release/apk/yacr.apk` mtime `2026-10-11 17:41`，SHA-256
  `63161e362b40bfc47d692aa15e1c24727e3f0f3e271509361b9ca838a890368d`，native-code 为
  `x86_64`，与设备 ABI 不符且早于当前 checkout；不安装。
- `target/debug/apk/yacr.apk` mtime `2026-10-12 00:52`、SHA-256
  `4755b255ab1f1ac426f9d8e858137e7766d677646ffba8cbdf4820849a88a3d2`，native-code 为
  `arm64-v8a`，有 `classes.dex`、启动 Activity `YacrNativeActivity`、`hasCode=true`，签名可验；
  但这是 741,991,914 字节的 debug APK，不符合本轮要求使用 release（项目记录 debug `.so` 体积
  可能导致 Activity attach 超时），故未安装。
- `/tmp/opencode/release-assets/yacr-android-release/release/apk/yacr.apk` 是 2026-10-10 的旧
  arm64 artifact，但 `classes.dex` 缺失且 manifest `hasCode=false`，不能作为本轮可启动制品。
- 因无可接受的 arm64 release APK，本轮未安装/启动，也没有应用 logcat、截图、交互、SAF 或视觉验收；
  未卸载、清数据或触碰用户文件。所有 APK 检查、设备状态和路径见
  `/tmp/opencode/yacr-android-device-20261012-005421/`。后续需提供/生成当前目标的 arm64 release
  APK，再继续 `adb -s ebfb4268 install -r` 与启动 smoke。

## 13. USB 真机重试（2026-10-11/12；指定 HTTPS 代理，构建超时）

- 设备仍为 Xiaomi `23046RP50C`（serial `ebfb4268`），Android 15/API 35，arm64-v8a。
- 代理探测：通过 `https_proxy=http://192.168.8.1:10809` 对 Skia GitHub release URL 做单字节
  Range 请求，HTTP 206、1 byte、4.43 s；对 `skia.googlesource.com` 的 DEPS 请求在 5 s
  connect timeout 失败。探测证据在本节目录。
- 执行正式命令 `ANDROID_BUILD_HTTPS_PROXY=http://192.168.8.1:10809 TARGET=aarch64-linux-android
  bash scripts/build-android.sh --release`。前置 `cargo check` 成功；链接阶段输出
  `Compiling skia-bindings v0.153.3` 后 30 分钟无进一步输出，达到明确的有界等待上限后以
  timeout 退出码 124 终止；事后未发现构建进程残留。此次没有获得 APK。
- 未安装/启动应用，未读写用户数据。原有 APK 仍为旧 `x86_64` 制品，不适用于本设备且未安装；
  无本轮应用 logcat、截图或交互证据。设备 GPU 能力不代表应用渲染验收。
- 本轮证据目录：`/tmp/opencode/yacr-android-device-20261011-235628/`，包括设备/源码信息、
  有界探测、完整构建日志、超时记录和进程检查。

## 12. USB 真机验证尝试（2026-10-11；构建阻塞，未安装）

- 设备：serial `ebfb4268`，小米 `23046RP50C`，Android 15 / API 35，ABI `arm64-v8a`
  （另有 armeabi-v7a/armeabi）；分辨率 1800×2880，density 400。设备在线、boot 完成。
- 设备 SurfaceFlinger 报 `Qualcomm, Adreno (TM) 730, OpenGL ES 3.2`，并声明 Vulkan
  compute/level 1/version 4198400。此处仅为设备能力枚举；**应用未启动，未验证应用使用该 GPU**。
- 当前源码 `96d631fe8dcaf6658ebee850008ad2f55034d39f`。正式入口
  `TARGET=aarch64-linux-android bash scripts/build-android.sh --release` 的 release 构建先后尝试
  Skia 预编译包下载及 Skia 源码依赖下载，均被网络阻断：未代理时 GitHub 下载超时，
  `chromium.googlesource.com` / `skia.googlesource.com` 连接超时，build script 失败退出。
  本轮未重试无限等待；无遗留构建进程。
- 仓库原有 `target/release/apk/yacr.apk` 时间为 2026-10-11 17:41，SHA-256
  `63161e362b40bfc47d692aa15e1c24727e3f0f3e271509361b9ca838a890368d`，native-code 为
  `x86_64`；早于当前源码提交，且 ABI 不匹配，故未安装、未启动、未用作真机证据。
- 安装前设备未安装 `dev.yacr.app`。本轮无应用 logcat、截图、交互或 SAF 验证；没有操作
  用户数据。构建日志、APK 元数据、设备信息与网络失败细节：
  `/tmp/opencode/yacr-android-device-20261011-231245/`。
- 同轮 `scripts/build-android.sh` 代理隔离契约测试及 `bash -n` 已通过；详见 `docs/build.md`
  与 `scripts/test-android-build-proxy.py`。这不改变本次构建因外网下载失败而未完成的事实。

## 1. 被测设备与制品

| 项 | 值 |
|---|---|
| 设备 | `emulator-5554`，`sdk_gphone64_x86_64` |
| SDK / ABI | API 35，`x86_64` |
| GPU（系统 GLES） | `Android Emulator OpenGL ES Translator (Google SwiftShader)`, OpenGL ES 3.0 (SwiftShader 4.0.0.1) |
| 模拟器启动参数 | `-gpu swiftshader`（`emulator @dev_api35 -no-window ...`） |
| 显示 | `adb shell wm size` = 1080x2400；density 420 |
| APK 路径 | `target/release/apk/yacr.apk`（`CARGO_TARGET_DIR` 外部时由构建脚本打印的路径） |
| APK 大小 | 12,816,804 字节（约 12.8 MB，release，压缩后） |
| APK SHA-256 | `f10de4e2fcf90e9272375ac928f8bcd09b5cc7b3c3279a77c4b7c7f3afbeb44c`（提交源码重建；首轮验证 APK 为 `298b65b8…`，大小相同。APK 非字节可复现，zip/签名时间戳会改变哈希） |
| native-code | `x86_64` |
| package / activity | `dev.yacr.app` / `android.app.NativeActivity` |
| 签名 | `CN=yacr dev, O=yacr`，SHA-256 `ae556b93ab916181980f09c45981331af65b2be5c0fc14aeb9fb5b921cae887f`（本地开发密钥，未入库） |
| 实测 manifest | `sdkVersion:'23'` / `targetSdkVersion:'30'`（顶层 metadata 不生效；Cargo.toml 已填实际值，见 `docs/build.md`） |

构建（x86_64 必须，aarch64 在 x86_64 镜像上会运行 abort）：

```bash
export PATH="$HOME/.cargo/bin:$PATH"
export CARGO_TARGET_DIR=/home/zenglanmu/sources/yacr/target
export CARGO_BUILD_JOBS=2
source ~/.config/android-env.sh
YACR_REPO=<worktree> PROFILE=release ~/android-dev/build-apk.sh
```

实测耗时约 4–5 分钟（增量）。**注意**：helper 脚本在末尾用相对路径 `find target`
定位产物；当 `CARGO_TARGET_DIR` 指向仓库外时该 `find` 会报 `No such file or directory`
并使脚本退出码为 1，但 APK 已由 `cargo apk` 成功产出。等价的直接命令：

```bash
cargo apk build -p app-android --target x86_64-linux-android --lib --release
# 产物：$CARGO_TARGET_DIR/release/apk/yacr.apk
```

## 2. 安装与启动

```bash
adb logcat -c
adb install -r -t target/release/apk/yacr.apk      # Success，~1–2s
adb shell am start -n dev.yacr.app/android.app.NativeActivity
adb shell pidof dev.yacr.app                        # 4111（新增修复版）
```

- 进程存活（`pidof` 返回 pid），无 `FATAL EXCEPTION` / `AndroidRuntime` 崩溃，
  无 `ANR in dev.yacr`（`grep -c` 为 0）。
- 启动约 1.5–2.5 s 后首次渲染完成。

## 3. 渲染后端（来自 logcat，非推测）

```
I app_android: yacr android host starting
D vulkan  : searching for layers in '.../lib/x86_64'
W wgpu_core::instance: Missing downlevel flags: DownlevelFlags(SURFACE_VIEW_FORMATS)
W wgpu_core::instance:     shader_model: Sm5,
I cad_ui_slint::bridge: CAD renderer initialized: preference=WebGpu actual=WebGpu \
    compute=true storage_buffers=true indirect_draw=true max_texture_dimension=8192
I MESA    : exportSyncFdForQSRILocked: call for image ...
```

- 应用上报的后端是 `WebGpu`（`BackendPreference::WebGpu`，即 wgpu 的 WebGPU API 层）；
  `actual` 由 `cad-render-wgpu::caps_for` 按 preference 映射，因此它是 **API 层标签**，
  不是底层图形 API 名。
- 底层实际路径：wgpu 通过 Android Vulkan loader 初始化（`vulkan` + `wgpu_core` +
  `MESA`/SwiftShader 日志）。模拟器为 `-gpu swiftshader`，系统 GLES 亦为 SwiftShader。
- Slint 合成器：Skia（`D skia: [SkFontMgr Android Parser] ...`），位于 wgpu 之上。
- 结论：**模拟器 SwiftShader 上的 Vulkan（wgpu）/ Skia 合成**。未观察到 fallback 到
  其他后端；不声明真机 GPU 行为。

## 4. 渲染内容（截图解码，非空判定）

截图由纯 stdlib PNG 解码器分析（`/tmp/opencode/android-runtime/png_analyze.py`），
1080x2400 RGBA：

| 区域 | 颜色数 | 非主色像素比例 | 说明 |
|---|---|---|---|
| top_bar | 382 | 0.0458 | 系统状态栏 |
| canvas_upper | 332 | 0.0043 | 画布上段，虚线圆/弧 |
| canvas_mid | 689 | 0.0909 | 画布中段，几何 |
| canvas_lower | 1157 | 0.6232 | 面板/工具条区域 |
| bottom_status | 13 | 0.3556 | 底部状态栏 |

（基线截图 `01_baseline_fit.png`；`canvas_upper`/`canvas_mid` 的数百种颜色来自虚线几何
与抗锯齿边缘。）

内置演示几何（`cad-app::demo_database`：房间矩形 + 半径 800 圆 + 带 bulge 的多段线）
以虚线真实绘制。**画布非空白**。

## 5. 交互验证（像素 diff，关键回归）

未修复版本与修复版本的同一操作对比（`png_diff.py`，同尺寸逐像素）：

| 操作 | 构建 | 变化像素 | 结论 |
|---|---|---|---|
| 画布拖动 `input swipe 700 400 300 400 300` | **未修复** | 0 / 2,592,000（0.00%） | 画布完全无响应（缺陷） |
| 画布拖动 | 修复后 | 21,415 / 2,592,000（0.83%） | 相机/视图真实改变 |
| 点按浮动导航「适应」重定中心 | 修复后 | 21,415 / 2,592,000（0.83%） | 视图恢复到拖动前 |
| 画布点按（无活动工具）`input tap 540 500` | 修复后 | 363（仅顶部时钟） | 无副作用，不提交批注 |
| `input keyevent KEYCODE_BACK` | 修复后 | — | 退到 Launcher，进程存活，无崩溃 |

- 修复前 swipe 0 像素变化，证明 Android 宿主从未把 `pointer-input`/`scroll-input`
  接到 `ViewInput`（Web 宿主已接；Android 遗漏），画布无法平移/缩放。
- 修复后截图可见 `01_baseline_fit.png` / `02_after_pan.png` / `03_after_fit.png`：
  虚线几何随拖动位移，点「适应」后回到基准。
- 批注安全：本次改动后 Android 已安装 `CanvasPickMapper`（`screen_to_world`），量测/
  批注工具激活时画布点按会产生真实世界点；无活动工具时点按走选择拾取。画布拖动手势
  仍不会误提交批注/量测。以上仅在宿主本机做类型检查与纯逻辑单测（见 §8），**未在
  模拟器上重新点击验证**；§5 的像素 diff 仍是未接线版本的历史证据。

截图与日志已随本文件提交（`docs/evidence/android-runtime/`）：

| 文件 | 内容 |
|---|---|
| `01_baseline_fit.png` | 修复版基线（已点「适应」），状态栏显示正确就绪文案 |
| `02_after_pan.png` | 同上，执行画布拖动后（几何位移） |
| `03_after_fit.png` | 再次点「适应」后，与 `01` 逐像素一致 |
| `unfixed_before_swipe.png` / `unfixed_after_swipe.png` | 未修复版：拖动前后完全一致（0 像素变化） |
| `logcat-backend.txt` | 启动与后端初始化 logcat（本次运行） |
| `png_analyze.py` / `png_diff.py` | 纯 stdlib PNG 解码/区域统计与逐像素 diff 脚本 |

会话内原始文件亦在 `/tmp/opencode/android-runtime/`。

## 6. 打开图纸（步骤 6）：**NOT RUN**

- 已把样本推送到宿主候选路径：`/sdcard/Download/yacr-sample.dwg`（81,659 字节，
  来自 `/tmp/opencode/dwg-samples/baseline-sample.dwg`）。
- **未能触发**：本 build 的 `safe_insets` 未被消费、surface 尺寸/旋转未回传
  （audit U07），顶部工具栏（Open 按钮所在）被系统状态栏遮挡/出屏。实测在顶部点按
  无任何变化（diff 0，焦点仍在本 Activity），无法到达 Open。故 F01 的“打开图纸”在
  模拟器上 **未执行**，不能宣称已验收。
- **本 workstream 的异步接线未改变该结论**：`open_drawing` 现在在 Android（有
  `std::thread`）走 `begin_async_open` + `poll_async_open` + 进度面板，并有宿主单测与
  Android 目标类型检查；但**未在模拟器上点击 Open 或观察进度面板**，F01 仍为
  **NOT RUN**。

## 7. 本次修复的代码缺陷

1. **画布输入未接线（阻塞级）** — `apps/app-android/src/lib.rs`
   新增 `AndroidViewInput` 并在 `start()` 中 `adapter.set_view_input(...)`，把
   `TouchArea` 的逻辑像素事件翻译为 `Pan`/`Zoom` 命令，与 Web 宿主一致；相机由
   `HostController` 的权威 `Viewport` 统一维护。未接线时画布拖动/缩放完全无效。
2. **状态栏误导** — shell 初始状态 `status.scaffold`（“框架占位：尚未接入 CAD 画布”）
   在 `install_cad_bridge` 之后仍然显示。现改为：无恢复快照时显示
   `就绪（内置演示几何，非兼容性声明）`。
3. **后端不可观测** — 在 `crates/cad-ui-slint/src/bridge.rs` 的 `RenderingSetup`
   分支新增一行 `log::info!`（仅新增日志，不改逻辑），在设备日志中如实上报真实
   初始化结果；`BeforeRendering`/`install` 逻辑不变。此为本 workstream 之外 crate 的
   最小附加改动，已在报告中标注。

回归测试：`apps/app-android` 新增 `android_view_input_drag_pans_the_authoritative_camera`
与 `android_view_input_scroll_zooms_the_camera`（纯宿主逻辑）。宿主本机缺少
pkg-config/fontconfig 开发头，`app-android` 被主机测试排除，**这两个测试在本环境
NOT RUN**；仅做了 Android target 的类型检查。

## 8. 仍未完成 / 限制（全部显式记录）

- **真机**：未执行。本文件全部为模拟器 SwiftShader 结果。
- **surface 尺寸与安全区（audit U07）**：`UiConfiguration::safe_insets` 仍未被任何代码
  消费。纯函数 `apply_surface_size(controller, size_logical, dpi_scale)`
  （用 `apply_canvas_metrics` 更新 `Viewport.logical_size`/`dpi_scale` 且**不动相机**，
  见 `apps/app-android/src/state_push.rs`）在 `start()` 用配置逻辑尺寸调用一次。
  本 workstream 新增导出入口 `pub fn set_surface_size(width, height, scale)`
  （`apps/app-android/src/lib.rs` 重导出 `poll::set_surface_size`）：它经线程内注册的
  `Runtime` 调用 `apply_surface_resize`（纯 `apply_surface_size` + 重同步相机 + 重推
  布局/叠加层）。**Activity 侧尚未把 `SurfaceHolder` 的尺寸/旋转回调转发到该入口**
  （NativeActivity 胶水在本仓库外），因此真实 surface 变化仍未生效；未拍现场景不做假
  接线。剩余挂钩位置即 `set_surface_size`（见本节末“宿主侧待办”）。单测：
  `android_surface_resize_preserves_the_camera_target`、
  `android_surface_entry_point_applies_after_runtime_install`（**未设备复测**）。
- **量测/批注拾取**：本次改动已在 Android 安装 `CanvasPickMapper`
  （`AndroidCanvasPickMapper`，逻辑像素→`Viewport::screen_to_world`），量测/批注工具
  激活时画布点按产生真实世界点，不再只报 `status.pick_unwired`。退化输入（无 surface、
  非有限坐标）返回 `None`，不伪造点。**未在设备上复测**。
- **面板状态已推送**：Android 宿主新增 `push_panel_state`，在启动、打开图纸成功、
  命令执行与画布交互后推送历史可用性、测量、图层（+有序 id）、属性、批注（+有序 id）、
  布局、诊断；无数据时是显式空态。渲染桥同时接收 `session.layer_overrides`，图层开关
  会真正影响显示。**未在设备上复测**（面板可见性/命中等需截图验证）。
- **SAF/文件选择器**：未实现（沿用候选路径）。
- **打开 DWG**：NOT RUN（见 §6）。
- **`shader_model: Sm5` 的 downlevel 警告**：`SURFACE_VIEW_FORMATS` 缺失属模拟器
  Vulkan 能力提示，未观察到渲染失败。
- **helper 脚本**：见 §1 的 `find target` 退出码注意点。

## 8b. 宿主接线状态（未设备复测）

`apps/app-android/src/state_push.rs`（宿主→外壳连接器）接入
`start()`、打开图纸成功、命令执行与画布交互；本 workstream 再新增异步打开与 surface
入口：

| 连接器 | 位置 | 状态 |
|---|---|---|
| 历史可用性（undo+redo） | `push_panel_state` → `set_history_availability` | 已接线，替换了原来的 `set_can_undo` 单独调用（redo 不再陈旧） |
| 测量面板 | `MeasurementUiState::from_preview` | 已接线 |
| 图层面板 + 有序 `LayerId` | `LayerPanelState::from_rows` + `layer_ids` | 已接线；并把 `session.layer_overrides` 推入渲染桥 |
| 属性面板 | `PropertyPanelState::from_properties` | 已接线（空选择为显式空态） |
| 批注面板 + 有序 `AnnotationId` | `AnnotationPanelState::from_rows` | 已接线 |
| 布局面板 + 有序 `LayoutId` | `layout_descriptors` + `from_descriptors` | 已接线（真实布局表；无布局为显式空态） |
| 诊断抽屉 | `last_import_report.diagnostics` | 已接线；无报告时为空且摘要为“未验证”，不显示为完整 |
| 选择高亮 | `push_panel_state` → `CadView::set_selection_highlight` | 已接线；选择来自 `HostController::selection()`，空选择为显式空高亮 |
| 测量/批注预览 | `push_panel_state` → `CadView::set_measurement_preview` / `set_annotation_preview` | 已接线；无工具时传 `None`（取消叠加层） |
| 布局切换 | 适配器默认路径 → `SwitchSpace` → `HostSink::send` → `execute` | 已接线；**刻意未安装 `LayoutSwitchSink`**（安装会取代命令路径）。相机与布局面板经 `sync_view_camera`/`push_panel_state` 重同步 |
| 画布→世界映射 | `AndroidCanvasPickMapper` → `set_canvas_pick_mapper` | 已接线；退化输入返回 `None` |
| 点按选择 | `AndroidViewInput`（`InputPolicy` 区分 tap/drag） | 无捕获工具时点按 `pick_at_screen`，命中派发 `Select`+`Selection`，未命中清空选择并显示显式状态；拖动仍平移且不选择 |
| 异步打开进度 | `host.rs::open_drawing` → `begin_async_open`；`poll.rs` 定时轮询 → `push_import_state` → `set_import_state` | 已接线（本 workstream）；发布由核心 stamp 守卫完成，宿主只镜像一次 |
| 打开取消 | `cancel-open-requested` → `CancelLoading` → `cancel_async_open` | 已接线（命令路径，不丢文档/批注） |
| surface 尺寸 | 导出 `set_surface_size` → `apply_surface_resize` → 纯 `apply_surface_size` | 纯函数 + 启动调用 + 导出入口；**Activity resize 回调未转发**（见上） |

**宿主侧待办（Activity）**：`android-activity` 的 `SurfaceHolder` 尺寸/旋转变化时需要
调用 `set_surface_size(logical_w, logical_h, dpi_scale)`（它内部完成
`apply_surface_size` + `sync_view_camera` + `request_redraw` + `push_panel_state`）。
当前 `apps/app-android` 的 `start()` 拿不到该回调，因此只落地导出入口与其单测，
**未伪造 resize 行为**；入口在 UI 线程之外或 `start()` 之前调用会返回显式错误，不静默
成功。

**测试（本机 `#[cfg(test)]`，Android 目标 `cargo check --tests` 可编译；本机缺
fontconfig 无法原生运行，标注 NOT RUN）**：`state_push` 派生（图层/布局/空面板/诊断
行）、`apply_surface_size`（更新尺寸与 DPI 且保持相机 target 不变、退化输入报错）、
`android_surface_resize_preserves_the_camera_target` 与
`android_surface_entry_point_applies_after_runtime_install`（导出入口）、异步打开
（`android_async_open_publishes_once_and_leaves_no_running_job`、
`android_cancel_command_keeps_the_document_and_reports_cancelled`、
`android_failed_async_open_keeps_the_demo_document`、
`android_import_panel_maps_running_and_terminal_snapshots`）、选中点按/空白清除/拖动不
选择、无 surface 时 pick mapper 返回 `None`、选择高亮（空选择 vs 真实选择，预览为
`None`）、布局切换命令路径
（`android_layout_selection_routes_through_switch_space`：模型→图纸→模型、
未知布局被拒并保持原空间）。

## 9. 必跑检查结果（全部通过）

```bash
cargo test --workspace --exclude cad-ui-slint --exclude app-android --exclude app-web --locked
#   EXIT 0；53 个 test result: ok，无 FAILED
cargo fmt --all --check                       # FMT_OK
cargo clippy --workspace --exclude cad-ui-slint --exclude app-android --exclude app-web --all-targets --locked
#   EXIT 0，无 warning
python3 scripts/check-architecture.py         # Architecture OK: 23 packages
cargo check --workspace --lib --target wasm32-unknown-unknown --locked   # EXIT 0
cargo check --target aarch64-linux-android -p cad-ui-slint -p app-android --locked   # EXIT 0
# 附加：类型检查 Android 目标下的新增测试
cargo check --target aarch64-linux-android -p app-android --tests --locked   # EXIT 0
```

宿主缺少 pkg-config/fontconfig 开发头，`app-android` 被主机测试排除，因此 §7 的两个
回归测试在本环境无法运行（**NOT RUN**）；已用 Android target 类型检查确认可编译，
并在设备上以像素 diff 做了端到端替代验证（§5）。

提交源码重建（build5）后再次安装/启动/复测：`pid=4456` 存活，
`CAD renderer initialized: preference=WebGpu actual=WebGpu ...` 日志一致，
拖动像素 diff = 21,534/2,592,000（0.83%），与 build4 相同。

## 10. P4：SAF 打开 / 另存 / New（2026-10-11，本轮）

平台：无头 x86_64 模拟器 **`yacr35`（API 35，`google_apis`，x86_64，KVM，
`-gpu swiftshader`）**，`wm size 1080x1920` / `wm density 420`。**不是真机**。
证据目录：`/tmp/opencode/yacr-android-p4/`（截图、logcat、写出的 DXF）。

被测制品：`TARGET=x86_64-linux-android bash scripts/build-android.sh --release` 产出
`target/release/apk/yacr.apk`，含注入的 `classes.dex`；`aapt2 dump xmltree` 实测
`android:hasCode=true`、启动 Activity = `dev.yacr.app.YacrNativeActivity`
（`extends android.app.NativeActivity` 的 SAF 垫片）。

流程与结论（截图逐张）：

| 步骤 | 操作 | 证据 | 结论 |
|---|---|---|---|
| 打开 | 点顶栏“打开”图标 → DocumentsUI `ACTION_OPEN_DOCUMENT` → Downloads → `flange.dxf` | `12-open-loaded.png`；`PickerActionHandler onFinished content://…raw%3A%2Fstorage%2Femulated%2F0%2FDownload%2Fflange.dxf` | 标题变 `flange.dxf`，画布渲染法兰几何（弧/尺寸线/红色中心线） |
| 另存 | `input keycombination 113 47`（Ctrl+S）→ `ACTION_CREATE_DOCUMENT`（默认名 `yacr-demo.dxf`）→ SAVE | `14-save-attempt.png`、`15-save-done.png`、`yacr-demo.dxf` | `/sdcard/Download/yacr-demo.dxf` 写出 **48333 字节**，首 `0/SECTION/2/HEADER`、末 `EOF` |
| New | `input keycombination 113 42`（Ctrl+N） | `16-new-done.png` | 标题变 `untitled`，画布空白 |

- 全程 logcat 无 `FATAL EXCEPTION` / `ANR in dev.yacr` / panic（计数为 0）。
- **打开/另存走 SAF 内容 URI**（`ContentResolver` 读写），不使用全盘存储权限。

**环境阻塞（非本轮改动，诚实记录）**：任务指定的 AVD **`yacr`（API 30）无法渲染**：
其 goldfish/SwiftShader Vulkan 驱动上报 `maxBufferSize=0`，wgpu `request_device` 失败
（`Limit 'max_buffer_size' value 268435456 is better than allowed 0`）；此外适配器选择还需
`WGPU_ALLOW_UNDERLYING_NONCOMPLIANT_ADAPTER`（本机 `android_main` 已设置，对合规真机为
no-op）。API 35 镜像无此问题，故 P4 e2e 在 `yacr35` 上完成。API 30 上的 SAF 本身未验证。

**NOT RUN**：真机；OEM provider（Google Drive / 厂商云盘）；多 API 矩阵；
`ACTION_CREATE_DOCUMENT` 覆盖已打开源文件同 URI 的守卫场景（该选择器产生新 URI，无法稳定
复现；守卫逻辑由 `cad-host` 单测 `save_source_guard` 覆盖）；Android 离屏渲染。

## 11. 宿主真实 GPU 运行（`-gpu host`，2026-10-11，本轮）

平台：无头 Android 模拟器 **`yacr35`（API 35，`google_apis`，x86_64，KVM）**，
`-gpu host`（宿主显卡透传），`-no-window -no-snapshot -read-only`。宿主为 Wayland 桌面 +
Intel UHD + Quadro P620（`nvidia-smi` 可见）。**这不是真机**，且 GPU 是宿主显卡而非设备
SoC。证据目录：`/tmp/opencode/yacr-android-host-gpu/`。

渲染器（`adb shell dumpsys SurfaceFlinger`，设备原文）：

```
GLES: Google (Intel), Android Emulator OpenGL ES Translator (Mesa Intel(R) UHD Graphics (CML GT2)), OpenGL ES 3.0 (4.6 (Core Profile) Mesa 26.0.8-1ubuntu0.3)
```

即模拟器的 GLES 渲染器是**宿主 Intel UHD（Mesa 26.0.8）**，**不是 SwiftShader**（对比 §1 的
`Google SwiftShader`）。应用侧：`ro.hardware.vulkan=ranchu`，wgpu 走 Android Vulkan
（guest goldfish 转发到宿主 Mesa；应用进程 logcat 有 `MESA: exportSyncFdForQSRILocked` 行）；
`app_android` 启动、wgpu 初始化成功，无 `FATAL EXCEPTION`/`ANR`/panic。

SAF e2e（与 P4 相同流程；证据 `h1-launch.png`/`h2-open-loaded.png`/`h3-new.png`/
`flange-out.dxf`/`renderer-evidence.txt`/`logcat-host-gpu.txt`）：

| 步骤 | 操作 | 证据 | 结论 |
|---|---|---|---|
| 打开 | Ctrl+O → DocumentsUI `ACTION_OPEN_DOCUMENT` → Downloads → `flange.dxf` | `h2-open-loaded.png`；`PickerActionHandler onFinished …raw%3A…%2FDownload%2Fflange.dxf` | 标题 `flange.dxf`，法兰几何渲染 |
| 另存 | 打开后立即 Ctrl+S → `ACTION_CREATE_DOCUMENT` → 改名 `flange-out.dxf` → SAVE | `flange-out.dxf` | `/sdcard/Download/flange-out.dxf` = **145282 字节**，首 `0/SECTION/2/HEADER`、末 `EOF` |
| New | Ctrl+N | `h3-new.png` | 标题 `untitled`，画布空白 |

- 全程无 `FATAL EXCEPTION` / `ANR in dev.yacr` / panic（计数 0）。
- 打开/另存走 SAF 内容 URI（`ContentResolver`），不使用全盘存储权限。
- **wgpu 与宿主 Vulkan**：本机 API 35 镜像上 wgpu 设备创建成功并渲染；未见 §10 记录的
  API 30 goldfish Vulkan `maxBufferSize=0` 问题（那是 API 30 镜像的驱动缺陷，与 `-gpu host`
  无关）。

**NOT RUN / 边界**：真机；真实设备 SoC GPU（本机 GPU 是宿主显卡）；OEM provider（Google
Drive / 厂商云盘）；多 API 矩阵；软件渲染与宿主 GPU 的逐像素对比；Android 离屏渲染。
