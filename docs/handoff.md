# 后续 agent 接手入口

## UI item 5 收尾轮（2026-10-09/10，本轮）

用户要求继续 handoff 收尾；问询确认范围为「全量 UI item 5」：布局面板（纸空间/视口
裁剪与比例）、资源/3D 抽屉、安全区/软键盘并入坐标映射，另含可收口小项（plot-style
显式建模、批注移除文档债）。四条并行工作流 + 主控集成验证。

**改动**

1. **纸空间视口裁剪闭环**（`cad-representation`）：mesh 逐三角形 Sutherland–Hodgman
   裁剪（z/法线/逐顶点 sRGB 颜色插值）、图像四边形 UV 插值裁剪
   （`DisplayPrimitive::Image` 新增可选 `clip`）；未成形 Text/坏 mesh/退化图像等
   不可裁剪情形**保留几何 + `Partial` + 稳定原因码**（`viewport.clip_*`）。
   15 项新契约；`docs/layouts.md` §2.4/§4/§5 同步。
2. **UI 抽屉与布局比例**（`cad-ui-slint`）：资源抽屉（字体/导入/代理/引用/图像真实
   分区，无数据显式空态，图像未建模显式说明）、3D 观察抽屉（复用既有
   `Switch2d3d`/`SwitchProjection`/`StandardView` 命令；环绕=手势说明；3D fit 显式
   禁用）、布局比例**显示级**（`LAYOUT_SCALE_CONTROL_WIRED=false`，命令面不存在
   不伪造）。i18n **202→248 keys**，含 `diagnostic.representation.viewport_clip_partial`、
   `diagnostic.import.plot_style_unsupported`、`diagnostic.import.shade_plot_unsupported`。
3. **plot-style 显式 Unsupported**（`cad-import-acadrust`，铁律 2 补齐）：
   `PlotSettings.current_style_sheet`/`Layout.plot_style_sheet`/`shade_plot_mode` 首次
   被读取并以 `import.plot_style_unsupported`/`import.shade_plot_unsupported` 显式
   建模（此前**完全未读取、静默忽略**，与文档"显式 Unsupported"声称不符）。5 项契约。
4. **批注移除文档债**：`CAD_IMPLEMENTATION_SPEC.md`、`docs/architecture.md`、
   `docs/core-invariants.md`、`docs/compatibility.md`、`docs/code-audit-and-agent-handoff.md`、
   `docs/panels.md`、`docs/ui-redesign.md`、本文 7 项清单的 F07/F08/F09 与
   `cad-annotations` 引用按「产品已移除（2026-10-05）」重写；历史证据段保留并标注。
5. **宿主接线**（`apps/app-linux|app-web|app-android`）：三宿主安装
   `LayoutSwitchSink`（走 `CommandId::SwitchSpace` 校验路径，未知布局显式拒绝）；
   `cad_app::input::inset_canvas_metrics` 把安全区折入 `CanvasMetrics`（退化输入
   显式拒绝；Web 经 `env(safe-area-inset-*)`、Android 提供 `set_surface_insets` 入口、
   Linux 显式零）；资源/3D 抽屉真实数据推送（桌面 `fonts: None`：加载器丢弃
   `FontLoadReport`，显示显式空态而非臆造摘要）。`docs/input.md`/`docs/responsive-ui.md`
   同步。
6. **主控修复潜伏测试缺陷**：`app-android` 的
   `android_view_input_scroll_zooms_the_camera` 断言方向与三宿主共享公式
   （`factor=1-dy*0.0015`）及 `Camera::zoom_at`（factor>1 zooms in）契约**相反**——
   该测试不在任何已记录门禁内、从未运行过，属潜伏缺陷非本轮回归。改为双向断言
   （scroll down=zoom out、scroll up=zoom in）并修正 `view.rs` 错误注释，属对齐
   契约而非弱化。

**门禁（本机 ThinkPad 桌面；GPU 测试 lavapipe 串行）**：fmt、clippy(`-D warnings`)、
architecture、fixture-manifest、workflows、i18n(248) 全绿；workspace all-targets
check、wasm32 全 workspace lib check、app-web wasm check 全绿；node 六套契约全绿；
`cargo test`：cad-app 263、cad-representation 157、cad-import-acadrust 111/0/1 ignored、
cad-ui-slint 131+全部集成（含 offscreen 外壳渲染）、app-linux 全部（含 verify_ui/
verify_ui_drawing）、app-web 全部、app-android **19/0**（修复后）。
**Android 门（本轮补装 SDK 后实跑）**：本机原缺 Android SDK/NDK；已按 `docs/build.md`
布局安装（`~/android-sdk`，build-tools 34.0.0 / platforms 34+30 / NDK 27.0.12077973，
`~/jdk17`，env 写入 `~/.bashrc`），随后
`cargo check --target aarch64-linux-android -p cad-ui-slint -p app-android --all-targets`
与同目标严格 clippy（`-D warnings`）**通过**（顺带清掉两处 app-android 遗留问题：
`poll_import_once`/`ImportPollOutcome` 再导出的 cfg 限定为 android+test，
`lib.rs` 两处 `clone_on_copy`）。

**未运行/限制**：真实 DWG/真实 GPU 像素验收/浏览器/真机 NOT RUN（lavapipe/合成为限）；
Web 生产部署仍待 Cloudflare token；Android Activity 侧 OS inset 回调**未转发**到
`set_surface_insets`（真机安全区未生效，显式未接线）；3D fit、视口比例命令面、
图像实体建模仍显式缺失（见 `docs/panels.md` §3.4）；`cargo-apk`/APK 打包与模拟器
不在本轮（SDK/NDK 已就绪，可按 `docs/build.md` 后续执行）。中途两次用户中断/恢复，
被取消的工作流残留改动已由主控就地核对合并，无回滚。

## acadrust 0.5.5 → 0.6.3 升级（2026-10-09，本轮）

用户要求把工作区 `acadrust` 从 `=0.5.5` 升到 `0.6.3`（features `serde` + `import`），
修 API 破坏并跑绿门禁，随后同步文档与契约测试。

**改动**

- `Cargo.toml:61`：`acadrust = { version = "0.6.3", features = ["serde", "import"] }`；
  `Cargo.lock` 由 `cargo update -p acadrust` 刷新（新增 `quick-xml 0.36.2`、`foldhash`、
  `serde`；`thiserror 1→2`；去掉 `ahash`）。
- 唯一编译破坏：`EntityType::Surface` 现为 `Box<Surface>`（0.6.0 同时 box 了
  Helix/MultiLeader/Table/Extended），改 `crates/cad-import-acadrust/src/tests.rs`。
- 未 patch/vendor acadrust；`cad-proxy` 不加依赖；Unsupported 建模未动。

**门禁（本机，无 GPU）**：fmt、clippy(`-D warnings`)、architecture、fixture-manifest、
workflows、i18n、workspace all-targets check、wasm32 lib check 全绿；
`cargo test -p cad-import-acadrust -p cad-proxy -p cad-db -p cad-domain` 212 passed / 0 failed / 1 ignored。
新增契约测试：LAYOUT `group 72/73` 被 acadrust 类型化（`acadrust_types_dxf_layout_group_72_and_73`、
`plot::layout_embedded_plot_fields_are_read_verbatim`），STYLE xdata 字体面类型化与
`read_styles` 优先级（`acadrust_types_the_style_xdata_face_and_the_scan_keeps_group_3`、
`importer_resolves_the_typed_true_type_face_without_the_scan`、
`importer_style_font_chain_is_typed_face_then_group_3_then_scan`）。

**能力增量（源码核对，详见 `docs/compatibility.md`）**

| 能力 | 0.6.3 | yacr |
|---|---|---|
| STYLE xdata 字体面 | 已类型化（`true_type_font`，DXF+DWG 经 `typeface_eed`） | 已接入；链=类型化 > group 3/4 > 字节扫描 |
| LAYOUT group 72/73 | 已暴露（0.5.5 亦然，旧文档有误） | 直接读取；纸名带单位时优先纸名 |
| plot-style 解析值 | 不支持 | 显式 Unsupported |
| 代理 opcode | 不支持（`proxy_graphics.rs` 字节一致） | 显式 Unsupported |
| DWG xdata | 部分（新增 crate-private `object_xdata`） | 显式 Partial |
| DXF 未知码 | 不支持（DWG 新增 `raw_record`/`layer_handle`） | 未使用 |

**已决策（2026-10-09）**

- **Path A：类型化 `true_type_font` 优先。** 解析链固定为
  `类型化 true_type_font > group 3/4 声明字体 > dxf_style_xdata_fonts 字节扫描`。
  理由：类型化字段是 acadrust 权威解码出的字体面，字节扫描只是回退；QCAD 类文件
  （group 3 为空）行为不变。DWG `true_type_font` 经 `read_styles` 统一消费（DWG 由
  `io/dwg/typeface_eed.rs` 填充），无需新代码。契约注释与文档已按此对齐，测试已固定该行为。

**未决项**

- `dxf_style_xdata_fonts` 字节扫描现与类型化路径基本重复，是否简化（本轮**不删**，仅记录）。
- `EntityCommon.raw_record`/`layer_handle` 未使用，保留为潜在能力。
- plot-style 解析值、代理 opcode、DXF 已知实体未知码仍显式 Unsupported。

## Web 生产部署 LinkError：跨部署 glue/wasm 缓存错配修复（2026-10-09，本轮）

用户报告线上 <https://yacr-examples.snakeheartgo.top/> 启动失败：
`LinkError: import object field '__wbg_adapterInfo_f9744bfff61e772c' is not a Function`。

- 实测**当前生产自洽**（干净 Playwright 浏览器 `chosen=WebGl2 cad_frames=1`，无错误）；
  上一版不可变部署 `200b4aee` 的旧 `yacr.js` **不含** `adapterInfo` 胶水 →
  定位为**旧 `pkg/yacr.js` + 新 `pkg/yacr_bg.wasm` 的跨部署缓存错配**（文件名无内容哈希、
  无共同版本），不是构建内 wasm-bindgen 版本问题。
- 修复：`scripts/build-web.sh` 以 `sha256(wasm)[:16]` 为 build stamp 注入 `index.html` 的
  `<meta name="yacr-build">`；`index.html` 按 stamp 载入 `main.js?v=`；`main.js` 按同一 stamp
  `import(\`./pkg/yacr.js?v=\`)` 并把 `yacr_bg.wasm?v=` URL 传给 `init()`，js/wasm 始终成对取用。
- 新增 `scripts/test-web-cache-busting.py`（静态+变异）接入 `core.yml`；`build-web.sh` 缺
  `__YACR_BUILD__` 占位符即中止。
- 本机 Playwright：三个请求均带同一 stamp 且应用正常；`check-web-ui.mjs` 通过。
- **未部署**：无 Cloudflare token，生产仍是旧 bundle，需下次 `web-deploy` 生效；已受影响客户端
  需硬刷新/清站点数据一次。详见 `docs/validation-web.md` §14。

## Web：真实 GPU Playwright 调试 + WebGPU 软适配器静默挂起修复（2026-10-09，本轮）

用户要求「本机开启 web wasm 测试，真机调试」，随后「改用 playwright 调试」。本机是**真实桌面**
（ThinkPad P15v，Wayland，Intel UHD + Quadro P620），非历史无头 LXC。

**实际执行**

- 安装 `wasm-bindgen-cli 0.2.129`（匹配 `Cargo.lock`），`DIST=/tmp/opencode/yacr-web-dev
  WITH_FONTS=1 scripts/build-web.sh` 成功；`scripts/serve-web.py` 起静态服务。
- 用已装的 flatpak Chrome 155（CDP 附加 + 仓库外 shim，**未改仓库脚本**）跑
  `check-web-ui.mjs` 真实 GPU 通过，证据记入 `docs/validation-web.md` §12。
- 改走 Playwright：`playwright-core 1.64.0` + 自带 Chromium 156.0.8078.4（headful 真实 GPU）。
  调试脚本在仓库外 `/tmp/opencode/webtest/{debug-web,webgpu-probe,webgpu-pref}.mjs`。
- **定位并修复**：`--enable-unsafe-webgpu` 下 `navigator.gpu` 给的是 SwiftShader 软适配器，
  JS 预探测选 `webgpu`，但 Rust 侧 Slint/wgpu 渲染 setup 永不完成 →
  `chosen=WebGpu adapter=None lifecycle=Detached cad_frames=0`，**无错误无回退的静默空白**。
  修复：`cad-ui-slint::web::select_backend` 强制 `WebGpu` 经真实 `webgpu_available()` 门；
  `webgpu_available()` 拒绝 `wgpu::DeviceType::Cpu` 软适配器并 `console_log` 探测结果，
  失败由 `app-web::start_with_preference` 既有分支回退 `WebGL2`。
- 重构建 + Playwright 复验：`--enable-unsafe-webgpu` 下显示探测 `device_type=Cpu`、
  `GpuFailure(...)`、最终 `chosen=WebGl2 … cad_frames=1 lifecycle=Ready`；默认路径不变；
  `check-web-ui.mjs` 通过。
- 新增 `scripts/test-web-backend-fallback.py`（静态+变异）并接入 `core.yml`
  workflow-contracts；本机通过 fmt、`cargo check -p cad-ui-slint -p app-web --target
  wasm32-unknown-unknown --locked`、`check-workflows.py`。

**未运行/限制**：真 WebGPU 硬件适配器仍未验证（本机只有 SwiftShader 软适配器，修复刻意不用它）；
非真实 DWG、非手机真机；证据与命令见 `docs/validation-web.md` §12/§13。

## CI 发布 job 首次运行失败与修复（2026-10-09）

`v0.1` tag 触发 `build.yml` 后，`windows-release` 与 `macos-release` 在打包步骤失败
（`linux-app`/`windows-check`/`macos-check`/`web-build` 等通过）。经配置 GitHub API 鉴权后拉取
真实日志，确认是**两个确定性脚本 bug**（非环境问题）：

1. **Windows**：`check-pe-imports.py` 在 Windows 文本模式下输出 CRLF，`package-windows-release.sh`
   的 `while read` 读到 `kernel32.dll\r`，与锚定正则 `^kernel32\.dll$` 不匹配，于是把**所有**
   导入误判为「非系统 DLL」并中止。修复：`check-pe-imports.py` 用
   `sys.stdout.reconfigure(newline="\n")` 输出 LF；脚本读取时 `name="${name%$'\r'}"` 兜底。
   本机已用真实 MSVC 产物验证：CR 剥离前 10 个导入全被误判，剥离后全部识别为系统库。
2. **macOS**：`otool -L` 对 **universal（fat）** 二进制会为**每个架构**打印一行
   `path (architecture ARCH):` 头；脚本原来 `tail -n +2` 只跳过第一行，于是 `arm64:` 头被当作
   「非系统依赖」。修复：改为只取缩进行
   `otool -L "$binary" | awk '/^[[:space:]]/ {print $1}'`（thin/fat 均正确）。
3. **Windows（第二处）**：`fetch-fonts.sh` 的合并步骤用
   `pathlib.Path.read_text()/write_text()` 未指定编码，Windows 默认 `cp1252` 无法解码 UTF-8 的
   `fonts.json`（含非 ASCII 字体名）→ `UnicodeDecodeError`。修复：读写显式 `encoding="utf-8"`。

契约测试补了对应回归守卫（`test-package-windows-release.py` 要求 CR 剥离片段、
`test-package-macos-release.py` 要求 `awk` 过滤片段、新增 `scripts/test-fetch-fonts.py` 守卫
UTF-8 编码与「缺字体即失败」）；`test-package-macos-release.py` 与 `test-fetch-fonts.py` 一并
接入 `core-quality`。

**已验证（2026-10-09）**：`v0.1` tag 强制更新到修复提交后，run
`https://github.com/zenglanmu/yacr/actions/runs/37916376185` **全部通过**：
`windows-release`（11m39s，`yacr-windows-release` 57,385,763 B）、`macos-release`（6m14s，
`yacr-macos-release` 72,217,415 B，universal）、`linux-release`（3m51s，`yacr-linux-release`
61,648,871 B）均 success 并上传含字体产物；`android-apk`/`android-release`/`web-smoke`/
`web-deploy` 按能力门控 SKIPPED。这是 CI 打包证据，**不是**真机/真实 GPU/安装运行证据。

## CI 发布层补全：Linux / Android release job（2026-10-09，本轮）

用户要求：在 CI workflow 里也加上 Linux app、Android app（等）。经问询确认：新增
`linux-release` 与 `android-release`，与 `windows-release`/`macos-release` 一样在
`workflow_dispatch` 或 `v*` tag 触发并上传含字体产物。随后用户要求「加入 android 发布包，
指向 jdk17」，故 `android-release` 改为**自装 JDK 17 + 固定 SDK/NDK**、不再能力门控。

**改动**

1. `build.yml` 新增 `linux-release`（`ubuntu-latest`）：安装 Slint 依赖 →
   `cargo fetch --locked` 预热（`package-linux-release.sh` 以 `--offline` 构建）→
   `WITH_FONTS=1 scripts/package-linux-release.sh` → 上传 `yacr-linux-release`
   （tar.gz + sha256）。脚本在打包内实际跑 CLI `--help` 与 GUI `--headless` 解析错误，
   校验 RPATH/无缺失库；不跑窗口/GPU。
2. `build.yml` 新增 `android-release`（按需/tag，`ubuntu-latest`，自装工具链）：
   `actions/setup-java@v4`（temurin **JDK 17**）→ `sdkmanager` 安装
   `build-tools;34.0.0`/`platforms;android-34`/`platforms;android-30`/`ndk;27.0.12077973`
   并写入 `ANDROID_HOME`/`ANDROID_NDK_HOME` → 前置校验 → `cargo-apk 0.10.0` → 生成 gitignored
   开发 keystore → `scripts/fetch-fonts.sh apps/app-android/assets/fonts` →
   `scripts/build-android.sh --release` → 断言锁文件未改 → `aapt2 dump badging` 记录事实 →
   上传 `yacr-android-release` APK。`android-apk`（每次 push/PR，`ANDROID_CI_ENABLED` 门控）
   保留为构建+事实 job。
3. `scripts/check-workflows.py`：REQUIRED_JOBS 纳入两者；`android-release` **不**加入
   `GATED_JOBS`（已自装工具链，不再需要能力开关）；`docs/ci.md`（job 表、十七个必需 job、
   gated 列表、两节说明、NOT RUN、action 表新增 `actions/setup-java`）、`docs/build.md`、
   `docs/handoff.md`、`AGENTS.md` 同步。

**实际执行并通过（本机 Linux）**

- `python3 scripts/check-workflows.py`（PyYAML 解析 + 结构检查，含两个新 job）、
  `scripts/test-linux-workflow.py`、`check-architecture.py`、`check-i18n.py` 通过。

**已实跑验证**：`linux-release` 在 run 37916376185 通过；`android-release`（自装 JDK 17 +
固定 SDK/NDK）在 run 37920264484 通过（7m59s），产出 `yacr-android-release`（`dev.yacr.app` /
arm64-v8a / targetSdk 30 / 53,017,904 B）；`web-smoke`（`WEB_SMOKE_ENABLED=true`）在 run
37922705082 通过（修复：runner 上不存在 `/tmp/opencode`，`tee` 因 `pipefail` 失败——改用
`$RUNNER_TEMP`）。run 37924085023（`workflow_dispatch`）**全绿**：四个发布包 +
`web-build`/`web-smoke`/`web-deploy` 全部通过，`web-deploy` **真实发布**到
`https://yacr-examples.pages.dev`。**限制**：APK 打包**不等于安装/真机运行**（无设备/模拟器）；
`android-apk` 仍门控未启用。详见 `docs/ci.md`、`docs/validation.md`。

## macOS release 包：GitHub Actions（`macos-latest`）+ 含字体 `Yacr.app` tar.gz（2026-10-09，本轮）

用户要求：构建 macOS 下应用，最终类似 Linux/Windows 版，输出含字体的压缩包。经问询确认：
按仓库既有的 CI 模式实现（代码 cfg 分支 + `app-macos` + `package-macos-release.sh` +
CI `macos-latest` 出包 + 文档），产物形式为 **tar.gz：含 `Yacr.app` bundle + fonts/docs**。
本机是 Linux，**没有 Apple SDK/链接器，无法产出 Mach-O**，故本机不伪造 macOS 产物。

**改动**

1. **共享桌面宿主**：`apps/app-linux` 的宿主实现由
   `cfg(any(target_os = "linux", target_os = "windows"))` 放宽为
   `cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))`；平台差异按
   cfg 分派——文件选择（macOS 用 `rfd`/`NSOpenPanel`，`host/file_picker.rs`）、配置目录
   （`$HOME/Library/Application Support/yacr`，新增纯函数 `resolve_macos_config_dir`，
   `host/config_file.rs`）、系统默认字体（`/System/Library/Fonts` + `/Library/Fonts`，
   `cad-platform::fonts::local`，macOS 无 `fc-match`）。
2. **macOS 宿主**：新增 `apps/app-macos`（二进制 `yacr-macos`），薄封装共享入口
   `app_linux::entry::run()`。`--headless` 离屏验证平台强制软件 Vulkan（Linux/Windows），
   macOS 无 Vulkan，故 `entry.rs` 在 macOS 上把 `--headless` 显式报告为不支持，而非静默
   降级或编译失败。
3. **字体目录候选**：`cad_platform::fonts::local::font_dir_candidates` 增加
   `.app/Contents/Resources/fonts` 候选，使 `Yacr.app` 内 GUI 与同级 CLI 都能自动发现字体
   （含新增契约断言）。
4. **打包**：新增 `scripts/package-macos-release.sh`（**只能在 macOS 运行**，非 Darwin 直接
   退出）。默认 `MACOS_ARCH=universal`：构建 `aarch64-apple-darwin` + `x86_64-apple-darwin`
   并用 `lipo -create` 合成通用二进制；产出 `Yacr.app`（`Info.plist` + GUI + 
   `Contents/Resources/fonts/`）、`bin/cad-cli-tools`、`bin/yacr-macos` 与 `fonts` 符号链接、
   文档与脚本，打成 `yacr-<version>-macos-universal.tar.gz`（+ sha256）。`otool -L` 断言动态
   依赖只来自系统库，出现 `@rpath`/Homebrew 即中止；`WITH_FONTS=0` 离线仅 osifont；
   `YACR_MACOS_SMOKE=1` 跑 CLI `--help` 与 GUI 参数解析错误。
5. **CI**：`build.yml` 新增 `macos-check`（`macos-latest`，原生 arm64
   `cargo check -p app-macos --all-targets --locked`，每次 push/PR，不运行）与
   `macos-release`（`workflow_dispatch` / `v*` tag，universal 打包并上传 `yacr-macos-release`
   artifact）；`check-workflows.py` 的 REQUIRED_JOBS 纳入两者；`docs/ci.md` 同步。
6. **契约/文档**：新增 `scripts/test-package-macos-release.py`、`docs/macos-app.md`；更新
   `docs/build.md`、`docs/ci.md`、`docs/fonts.md`、`README.md`、`AGENTS.md`。

**实际执行并通过（本机 Linux，Rust 1.99.0）**

- 主机门禁：fmt、严格 clippy（`-D warnings`）、architecture/workflows/i18n、
  `test-package-macos-release.py`、`cargo check --workspace ... --all-targets` 通过；
  `app-macos` 在 Linux 上走 `cfg(not(macos))` 分支编译通过（仅证明新增 workspace 成员不破坏
  非 macOS 构建）。
- **macOS 门控代码的交叉类型检查**（本机 Linux，`rustup target add aarch64-apple-darwin`）：
  `cargo check -p app-macos --all-targets --target aarch64-apple-darwin --locked` 与
  `cargo clippy -p app-macos -p app-linux --all-targets --target aarch64-apple-darwin
  --locked -- -D warnings` 均通过（过程中修掉 `validation.rs`/`entry.rs` 对
  `cad_ui_slint::offscreen` 的无条件引用）。只类型检查，**不**链接、不产出 Mach-O、不运行。

**未运行/限制**：CI 的 `macos-check`/`macos-release` job **未在本环境运行**（无 macOS
runner）；Linux 主机**无法**产出 Mach-O，也没有 Apple SDK，不能本地复现；真实 Mac 的窗口、
Retina/DPI、Metal GPU 渲染与像素验收、原生文件对话框、配置目录写入、`--headless`（macOS 显式
不支持）、代码签名/notarization 全部 **NOT RUN**。详见 `docs/macos-app.md`。

## Windows release 包：GitHub Actions（MSVC）+ 含字体 zip（2026-10-09，本轮）

用户要求：构建 Windows 下 exe 应用，最终类似 Linux 版，输出含字体的压缩包；随后要求
**改走 GitHub Actions 的 Windows runner（MSVC）构建**。

**改动**

1. **共享桌面宿主**：`apps/app-linux` 的宿主实现由 `cfg(target_os = "linux")` 放宽为
   `cfg(any(target_os = "linux", target_os = "windows"))`，成为两个桌面宿主共享的实现
   层；平台差异按 cfg 分派——文件选择（Linux `ashpd` / Windows `rfd`，`host/file_picker.rs`）、
   配置目录（XDG / `%APPDATA%`，`host/config_file.rs`）、系统默认字体
   （`fc-match` / `%WINDIR%\Fonts`，`cad-platform::fonts::local`）。新增
   `app_linux::entry::run()` 供两个桌面二进制共用，避免解析/后端选择漂移。
2. **Windows 宿主**：新增 `apps/app-windows`（二进制 `yacr.exe`），薄封装共享入口；
   `cad-ui-slint::offscreen` 的 cfg 同步放宽到 Linux+Windows。
3. **打包（工具链无关）**：`scripts/package-windows-release.sh` 自动识别宿主 target：
   Windows 原生 MSVC 默认并自动 `+crt-static`（无需 VC++ redistributable）；Linux 可
   MinGW GNU 交叉或 `CARGO_XWIN=1` 走 MSVC。字体走 `fetch-fonts.sh`（该脚本也改为
   python3/python 自适应，便于 Git Bash）；非系统导入由新增 `scripts/check-pe-imports.py`
   （纯 Python PE 解析，替代 `objdump`，同时支持 Linux 与 Windows runner）检测，缺失即
   **中止**；zip + sha256 由 Python 生成（跨平台）。`WITH_FONTS=0` 离线仅 osifont。
4. **CI**：`build.yml` 新增 `windows-check`（`windows-latest`，原生 MSVC
   `cargo check -p app-windows --all-targets --locked`，每次 push/PR，不运行）与
   `windows-release`（`workflow_dispatch` / `v*` tag，打包并上传 `yacr-windows-release`
   artifact）；`check-workflows.py` 的 REQUIRED_JOBS 纳入两者；`docs/ci.md` 同步。
5. **契约/文档**：新增 `scripts/test-package-windows-release.py`、`docs/windows-app.md`；
   更新 `docs/build.md`、`docs/fonts.md`、`docs/validation.md`、`AGENTS.md`。

**实际执行并通过（本机 Linux，Rust 1.99.0）**

- 主机门禁：fmt、严格 clippy（`-D warnings`）0 警告、architecture/fixture/workflows/i18n、
  `test-package-windows-release.py` 通过；`cargo check --workspace ... --all-targets` 通过。
- GNU 交叉：`cargo check -p app-windows --target x86_64-pc-windows-gnu` 通过；MinGW
  release `cad-cli-tools.exe` 13,945,778 / `yacr.exe` 34,123,856 字节；
  `YACR_WINDOWS_SMOKE=1 scripts/package-windows-release.sh` exit 0。
- MSVC（`cargo xwin`，与 CI 同目标）：release 构建通过；`+crt-static` 下**无非系统导入**；
  `TARGET=x86_64-pc-windows-msvc CARGO_XWIN=1 WITH_FONTS=1 YACR_WINDOWS_SMOKE=1
  scripts/package-windows-release.sh` exit 0。产物
  `target/x86_64-pc-windows-msvc/release/dist/yacr-0.1.0-windows-x86_64.zip`，
  **57,364,212 字节**，sha256
  `deb9cc9ec3c31dddf1aeef9aa66bd7cd5a0d9184678c6b34e6929faa8c78abb7`（含构建时间，
  重跑会变）。gitignored `target/` 下，不入库。
- `check-pe-imports.py` 的 DLL 集合与 `x86_64-w64-mingw32-objdump -p` 一致。
- Wine（软件翻译）实际运行 GNU/MSVC 产物：`yacr.exe --bogus`（参数解析，exit 1）、
  `cad-cli-tools.exe --help`（exit 0）、`scan entities.dxf`（602 实体 / 75 块定义，exit 0）、
  `build-representation` 字体差分（带包内 `fonts/` 1464 primitives vs 无 1553）——证明打包
  字体被 Windows 二进制加载。

**未运行/限制**：CI 的 `windows-check`/`windows-release` job **未在本环境运行**（无 runner）；
真实 Windows 的窗口/DPI/原生文件对话框/配置写入、真实 GPU（DX12/Vulkan）与像素验收、
`--headless` 离屏、代码签名/SmartScreen、Windows 上 DWG `render`（无 wine+Vulkan 证据）、
完整 Linux `cargo test` 与离屏未在本轮重跑。Wine 只是软件翻译，不是 Windows 兼容性/真机
结论；GNU 与 MSVC 目标行为可能存在差异。详见 `docs/windows-app.md`。

## Linux release 包并入 GUI 主应用（2026-10-09，本轮）

用户指出 `scripts/package-linux-release.sh` 只打包无头 CLI，缺少 GUI 主应用。经问询确认
「并入现有 tar.gz」+「附带需打包的 `.so`」。

**改动**

1. **脚本**：同时 `cargo build -p cad-cli-tools` 与 `cargo rustc -p app-linux --bin
   yacr-linux`（后者带 `-C link-arg=-Wl,--disable-new-dtags,-rpath,$ORIGIN/../lib`）；staging
   增加 `bin/yacr-linux` 与 `lib/`；`copy_gui_libs` 用 `ldd` 收集非基础系统库（显式排除
   glibc/loader/libgcc/libstdc++）；打包内校验 `ldd` 无缺失、`readelf` 的 RPATH、库命中
   包内 `lib/`、`bin/yacr-linux --headless` 缺 `--output` 的既定错误加载成功；缺失即中止。
   新增 `BUNDLE_LIBS=0`（裸二进制）、`YACR_LINUX_SMOKE=1`（包内 GUI 离屏出图 + 报告断言）。
   `docs/` 纳入 `linux-app.md`、`fonts.md`；`PACKAGE.txt` 重写运行前提与限制。
2. **契约测试**：新增 `scripts/test-package-linux-release.py`（静态 + 变异），并接入
   `core.yml` 的 workflow-contracts 步骤与 `docs/ci.md`。只检查脚本文本，不跑 release 构建。
3. **文档**：`docs/build.md`、`docs/linux-app.md`、`docs/validation.md` 增补发布包布局、
   RPATH/库迁入、运行前提与边界。

**实际执行**：`VK_ICD_FILENAMES=.../lvp_icd.json YACR_LINUX_SMOKE=1 bash
scripts/package-linux-release.sh` 通过；迁入 8 个非基础库，包内 GUI 用 llvmpipe 离屏出图
`cadFrames=2`、`renderError=null`；产物 61732836 字节，sha256
`1473725b...af0094`（含构建时间，重跑会变）。release 构建命中缓存，非冷编译耗时。

**未运行/限制**：无真实 DWG（合成几何）、无窗口系统/真实 GPU/真机、无跨发行版/libc 兼容与
`xdg-desktop-portal` 桌面打开验证；包内库与构建发行版绑定，换发行版应 `BUNDLE_LIBS=0`
或从源码重建。

## 字体文件设计（2026-10-09，上一轮）

用户要求设计并**实现**字体文件方案（只做实现，不单独出设计文档），关键决策经问询确认：
字体来源 = QCAD + mlightcad；浏览器走 CDN，缺字体回退默认轮廓面；客户端输出带字体文件
（同级 `fonts/`）；客户端缺字体同样回退默认；QCAD `osifont.ttf` 提交进仓库，`.cxf` 不支持。

**改动**

1. **提交的字体包 `fonts/`**：`osifont.ttf`（QCAD 副本，GPL-3 + 字体例外；来源/哈希见
   `fonts/SOURCE.md`，许可证 `fonts/COPYING.GPL-3`）+ `fonts/fonts.json`（目录条目）。
   mlightcad 字体仍不入库。`THIRD_PARTY_NOTICES.md` 已记录。
2. **默认轮廓回退面（`cad-platform::fonts`）**：`load_font_engine` 在图纸字体缺失/拉取失败时
   保证注册一个默认面（`DEFAULT_FALLBACK_NAMES = ["osifont","arial","simplex"]`；
   `FontLoadReport.default_face` 记录）。新增 `register_default_face`（保留键
   `__yacr_default__`，置为第一回退）与 `fonts::local`（非 wasm）——可执行文件同级
   `fonts/` 目录的本地 `DirFontLoader`、目录候选/解析、fontconfig 系统默认候选与缓存、
   及组装引擎的 `load_engine`。
3. **Linux 桌面（`app-linux`）**：启动与每次打开图纸后重载字体：同级 `fonts/`（或
   `--fonts-dir`，显式目录必须存在）→ `--font` → 系统默认字体为第一回退。
4. **CLI（`cad-cli-tools`）**：`load_fonts` 增加 `requested` 入参，原生宿主复用
   `fonts::local`（同级 `fonts/` + 系统默认）；`run.rs` 先开图再取字体检清单。
5. **输出打包**：`scripts/package-linux-release.sh` 把 `fonts/` 复制到发布包
   `bin/` 同级；`scripts/fetch-fonts.sh`（平台无关唯一打包入口）把提交的
   `fonts/` 合并进目录清单与字节（web-dist/fonts、assets/fonts）。
6. **文档**：`docs/fonts.md`（来源/布局/回退/授权/未完成重写）、`docs/font-host-loading.md`
   （分层加桌面+默认面）、`docs/cli.md`、`docs/linux-app.md`、`docs/dxf-entity-coverage.md`。
   同轮评审后续：`fetch-web-fonts.sh`/`fetch-android-fonts.sh` 已并入**平台无关**的
   `scripts/fetch-fonts.sh <DEST> [FONTS...]` 并删除旧名——字体组装逻辑与目标平台解耦，
   各宿主（web/android/桌面）只传不同 `DEST`。见 `docs/fonts.md` `fonts/README.md`。

**实际执行并通过**：`cad-platform` 13 项（新增默认面/local 5 项）、`cad-cli-tools` lib 20、
`cli_contracts` 14（lavapipe 串行）、`app-linux` lib 1+3 ignored；fmt、严格 clippy
（`-D warnings`）、architecture/fixture/workflow/i18n、native 全 workspace check、
wasm 全 workspace lib check 均通过。CLI 差分 smoke：`entities.dxf` 无字体目录
lines=1443，有 `fonts/`（仓库根）lines=1497——缺字体时默认轮廓面成形，texts 均 0。
`fetch-fonts.sh` 合并 smoke：100 条目（99 mlightcad + osifont）成功。

**同轮后续（评审意见）**：Linux 发布包只带提交的 osifont 太单薄——用户指出 mlightcad
CDN 字库应随客户端输出内置（与 Web 一致）。`package-linux-release.sh` 已加 `WITH_FONTS=1`
（默认）：用平台无关的 `fetch-fonts.sh "$STAGE/fonts"` 组装 mlightcad 全量 + osifont，
`WITH_FONTS=0` 保留仅 osifont 的离线打包；PACKAGE.txt/文档已同步，mlightcad 再分发授权
仍由打包方负责。

**未运行/限制**：浏览器/真机下载与整形未验证；Android 未打包安装；无真实 GPU/窗口/
视觉验收；系统默认面依赖 `fc-match`，本机 `sans-serif` 命中 `.ttc`（引擎跳过）后取
`DejaVu Sans`，不同系统默认字形不同（预期）；Web 的「浏览器默认」是目录/CDN 轮廓面，
不是浏览器内建字体字节（wasm 无该能力）。

## 资源占用/显卡加速排查与三项修复（2026-10-08，本轮）

用户在真实 Wayland 桌面（Intel UHD 核显 + NVIDIA Quadro P620 混合显卡）上排查
`fixtures/complex-test.dwg`（4.2 MB，已在 `.gitignore`）的资源占用与「是否使用显卡加速」。
实测结论与修复如下；**本机为 ~30 GiB RAM 的桌面，非无头 LXC**。

**排查结论（均已实测）**

- **用了 GPU 加速，但排查时默认走 Intel 核显**：GUI（`yacr-linux`）与 CLI 在修复前
  默认都选中第一个 Vulkan 适配器 = Intel UHD Graphics（ANV）。DRM fdinfo 证据：进程在
  `renderD129`（Intel）上有 `drm-engine-render` 时间，`renderD128`（NVIDIA）为 0；
  `nvidia-smi` 看不到本应用。Slint 强制要求 GPU-backed 适配器
  （llvmpipe 需 `SLINT_WGPU_CPU=1`），能启动就意味着真在用 GPU。
- **空闲 CPU ~40%（单核）且与图纸无关**：启动无图纸（demo）也恒定 ~40%，
  原因是有 50 ms `Timer` 每 tick 无条件写 Slint 属性 + `CadView::request_redraw`，
  等价以 ~20fps 重绘。
- **大图 OOM 根因**：`complex-test.dwg` 只有 44,591 实体，但 1,081 个块定义嵌套
  展开成 **20,856,815 个 primitive / 46,064,001 顶点**；CLI `render` 之前
  `SceneCache::build`（不合批），`delta.added` 累积到 **~20 GB RSS（41 GB VM）被
  OOM-kill**（dmesg 已留证据）。

**改动**

1. **空闲重绘（`crates/cad-ui-slint/src/bridge/view.rs`、`apps/app-linux/src/host.rs`）**：
   `apply_view_snapshot` 对相同快照直接返回（不再 `set_view_state`+`request_redraw`）；
   `Runtime::metrics` 缓存 `(物理尺寸, config revision)`，未变化不调
   `refresh_window_layout`（不再每 tick 序列化有效配置 JSON）；`BridgeState` 新增
   `redraw_requests` 计数供测试/宿主观测。实测 release 空闲 CPU **~40% → ~0–2%**。
2. **显式 GPU 选择 + 双显卡默认独显（`cad-render-wgpu`/`cad-ui-slint`/`app-linux`/`cad-cli-tools`）**：
   新增稳定枚举 `GpuSelection`（`auto`/`high`/`low`）与
   `create_headless_gpu_with(preference, gpu)`（后端内按 `device_type` 优选，
   缺类型回退首个适配器，不跨后端）；`select_wgpu_backend_with` 映射到 wgpu
   `PowerPreference`；Linux App `--gpu`、CLI `render/plot --gpu`。**`auto` 与 `high`
   同义、在双显卡上默认优先独显**（用户要求），`low` 优先核显。实测修复后默认
   （`--gpu auto`）即选中 Quadro P620，`nvidia-smi` 显示进程 40–56 MiB / 1–4%。
   Slint 自动路径原生支持 `WGPU_ADAPTER_NAME`/`WGPU_POWER_PREF`（CLI 无头路径不读
   前者，文档已注明）。
3. **大图内存保护（`cad-cli-tools`）**：`render` 改用与桌面一致的打包虚线 +
   `SceneCache::build_compact`（97k 批次级合批），并新增 CLI 硬上限
   `--max-batches`/`--max-vertices`（默认 `4_000_000`/`128_000_000`，`0`=关闭），
   超限以 `invalid_input` **显式失败**（信息给出当前值与上限），绝不 OOM。
   `complex-test.dwg` 实测：**OOM ~20 GB → 峰值 RSS 3.85 GB、24 s、exit 0**
   （`scene.batches=97012`、`vertices=50251439`，帧按预算绘制部分批次——与桌面一致）。
   `plot` 路径同样套用预算守卫。

**测试与门禁（实际执行通过）**：`cargo test -p cad-render-wgpu`（40+12+13+3）、
`cad-ui-slint`（148，含新增 `tests/idle_redraw.rs`）、`app-linux`（8 passed /
3 ignored）、`cad-cli-tools` lib（20，含 `scene_budget_fails_explicitly_instead_of_oom`、
`gpu_selection_stable_names...`）与 `cli_contracts`（14，含 `--gpu`/`--max-batches`
解析契约）；fmt、严格 clippy（`-D warnings`）、architecture/fixture/workflow/i18n、
host 全 workspace 检查、wasm 全 workspace lib 检查均通过。

**未运行/限制**：Android/Web 未重编译运行（调用方签名未变）；无真实 GPU 像素矩阵、
真机、浏览器验收；`complex-test.dwg` 的数字是**本机软件 Vulkan（lavapipe）/Intel
核显**证据，不是性能或兼容性结论；NVIDIA 独显只验证到能出帧与占用（无视觉验收）；
GUI 空闲 CPU 数字来自本机 Wayland 桌面单次采样。

## Rust 工具链升级 1.98.1 → 1.99.0（2026-10-08，本轮）

用户要求把项目 Rust 工具链升级到最新 stable。以下为实际改动与**实际执行并通过**的门禁。

- **版本**：`rustup check` 报告最新 stable 为 **1.99.0**（`b940084d7`，2026-09-28）。
  `rust-toolchain.toml` 的 `channel` 由 `1.98.1` 改为 `1.99.0`，`components` 仍为
  `rustfmt`/`clippy`，target 仍为 `wasm32-unknown-unknown`。
- **同步镜像**：`.github/workflows/core.yml`（3 处 `uses` + 顶部注释）、
  `.github/workflows/build.yml`（5 处 `uses`）的 `dtolnay/rust-toolchain@1.98.1` →
  `@1.99.0`；`AGENTS.md`、`docs/build.md`、`docs/ci.md`（Action 表 + 锁定工具表）、
  `docs/validation.md`（环境小节）、`scripts/build-web.sh`、`scripts/build-android.sh`
  的版本说明同步更新。
- **未改动**：workspace `rust-version = "1.85"`（MSRV）保持不变——本次是工具链 channel
  升级，不是提高最低支持版本。
- **本机工具链**：`rustup toolchain install 1.99.0 --component rustfmt --component clippy
  --target wasm32-unknown-unknown`。
- **环境补装**：默认门禁包含 `cad-ui-slint`/`app-linux`，本机此前缺 fontconfig/freetype
  开发头，`yeslogic-fontconfig-sys` 构建脚本失败。经用户授权
  `sudo apt-get install pkgconf libfontconfig1-dev libfreetype6-dev` 后通过；此前
  `docs/validation-web.md` 记录的原生 Slint 构建阻塞在本机解除（该文档为历史证据，
  未回改）。

**实际执行并通过**（本机 1.99.0）：

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --exclude app-android --exclude app-web --all-targets
  --locked -- -D warnings`（含 `cad-ui-slint`、`app-linux`，0 警告）
- `cargo check --workspace --exclude app-android --exclude app-web --all-targets --locked`
- `cargo check --workspace --lib --target wasm32-unknown-unknown --locked`（完整
  workspace，含 UI/Host）
- `cargo check -p app-web --target wasm32-unknown-unknown --locked`
- `python3 scripts/check-architecture.py`、`check-fixture-manifest.py`、
  `check-workflows.py`、`check-i18n.py`

**未运行**：完整 `cargo test`、Linux 离屏/release、Android/Web 构建与真机（不属默认
提交门禁）；本文件下方与 `docs/validation.md` 中的详细证据仍标注其收集时的 1.98.1 环境。

## 完全移除用户批注（annotation）模块（2026-10-05）

按用户明确指示，整体移除用户批注（“批注/annotation”）功能，保留测量（F06）与 DXF
注释性缩放（annotative scaling，图纸实体特性）。范围与执行证据：

- **删除的 crate 与类型**：`crates/cad-annotations` 整包；`cad-db` 的 `Annotation`/
  `AnnotationDatabase`/`AnnotationGeometry`/`AnnotationStyle`/`AnchorStatus`/
  `EntityAnchor`/`validate_annotation`；`cad-domain::AnnotationId`。`MeasurementRecord`/
  `MeasurementAlgorithm` 从原 `annotation.rs` 迁至新的 `cad-db/src/measurement.rs` 保留。
- **历史/查询/场景**：`cad-history` 改为仅绘制实体撤销/重做（删除 `AnnotationPatch`/
  `UndoRecord`/`patch`/标注撤销 API/`MemoryJournal`/`RecoveryJournal` 与共享标记机制）；
  `cad-query` 删除 `AnnotationRow`/`annotations()`；`cad-scene` 删除 `annotations` 模块。
- **应用层**：删除 `annotation_tool`/`annotation_list`/`app_annotation` 与全部
  `CommandId`/`CommandPayload` 标注变体；`SessionState` 删除标注可见性/选择；`Document`
  删除 `annotations` 字段；`host_files`/unsaved 决策/恢复快照整体移除；`host` 删除
  标注导入/导出/恢复与 `save_measurement_as_annotation`；`recovery.rs` 仅保留后端结果模型。
- **绘制预览保留**：绘制/编辑工具的实时预览原先复用标注预览覆盖层，现改为
  `cad-app::render_scene::overlay::draw_preview_overlay`（`OverlayInputs.draw_preview`），
  行为等价（LINE/MOVE/TRIM 橡皮筋、CIRCLE 圆），不依赖已删除类型。
- **UI/宿主**：`cad-ui-slint` 删除标注面板/搜索/工具按钮/命令/预览桥接，`panels.slint`/
  `app.slint`/`ribbon.slint` 移除标注控件（ribbon 页签重编号为 文件/视图/测量/工具）；
  i18n 中英文各删除 19 个标注键（221→202，键集仍一致）；`app-linux`/`app-web`/`app-android`
  删除标注状态推送、sidecar 导入导出、恢复快照与未保存决策；Web JS/HTML 删除
  `annotation-input`、`export_annotations`、恢复快照入口；CLI 删除 `import-notes`/
  `export-notes` 与 `--notes`/`--allow-fingerprint-mismatch`。
- **测量**：`measure.save` 命令、`MeasurementUiState.can_save_annotation`、
  `StatusModel.unsaved` 等“存为批注/未保存”接口一并移除；测量本身（距离/折线/角度/面积、
  预览、确认/取消）保持。

**实际执行并通过的门禁**（本机、离线、排除 Android/Web 宿主编译）：
`cargo fmt --all -- --check`；`cargo clippy --workspace --exclude app-android
--exclude app-web --all-targets --offline -- -D warnings`；
`cargo check --workspace --exclude app-android --exclude app-web --all-targets --offline`
；`cargo check --workspace --lib --target wasm32-unknown-unknown --offline`（含 UI/Host）；
`python3 scripts/check-architecture.py`、`check-fixture-manifest.py`、`check-workflows.py`、
`check-i18n.py`；`node --test scripts/test-web-host.mjs scripts/test-host-panel-parity.mjs`。
`cargo check -p app-web --target wasm32-unknown-unknown --all-targets --offline` 通过。

**未运行/未闭环**：Android 目标（`aarch64-linux-android` 工具链与本机 target 不可用，
`--offline` 亦无法获取 `android-activity`）未编译；“各项测试通过”不包含 Android/Web
浏览器/真实 GPU/真机运行；文档 `CAD_IMPLEMENTATION_SPEC.md` §3.4/§16.3 与
`docs/architecture.md`、`docs/core-invariants.md`、`docs/compatibility.md`、
`docs/code-audit-and-agent-handoff.md` 中仍残留 F07/F08/F09 与 `cad-annotations` 引用，
尚未按“产品已移除该能力”重写，属已知文档债。真实 DWG 端到端见下一节。

1. 阅读规范、AGENTS.md、requirements.md、architecture.md 与最近 Git diff。
   此次恢复中发现并发实现与提交，结束时可能仍有其他实现工作；本文描述的是框架交接，
   请以当前源码和最新构建证据确认状态，不覆盖陌生变更。
2. 运行 README 的 check/test/architecture 命令；不要删除陌生的已有实现。
3. 用 `rg 'pending\(' crates apps` 查找显式占位；文件路径与字符串标识对应实施单元。
4. Android 的 Slint + wgpu 组合已验证到打包层（ADR 0002、docs/validation.md）；
   本轮进一步在无头模拟器（KVM + SwiftShader，x86_64）实际安装、启动、渲染并验证
   画布平移/适应（docs/validation-android.md）。**真机仍未运行**。Web 已可运行：
   wasm 静态产物在无头 Chromium 以 WebGL2 通过加载/导航/双语（docs/validation-web.md）；
   **WebGPU 与真实 GPU 未验证**。
5. 逐项实现 Builder/事务/历史 → 导入/基础语义 → 显示/索引/场景 → 应用/工具 → 宿主闭环；
   不把这个依赖顺序当作排期，也不绕过优先 GPU/UI 验证。
6. 每次移除占位同步补齐规范测试、能力表、构建说明、Git 提交。
7. DWG 打开/渲染回归遵循 `docs/testing-dwg.md`：原生 lavapipe 为主、WASM +
   Playwright 为第二层；出图 smoke 与参考图视觉验收分开记录。

## Linux 桌面加载响应与不完整出图（2026-10-04，最新一轮）

- 桌面读取/导入及 native CPU 准备移出 UI 线程，采用单 worker + latest pending 的内容键
  与过期拒绝，保留原数据库/命令与增量缓存；未保存决策不被重新解释。
- renderer staged upload 不再复制整份 SceneDelta；桥接按每帧原预算累积绘制全部有序
  场景，保留透明顺序与深度，修复 5025 万顶点样本只提交前 800 万顶点的问题。
- 桌面 post-start debug/release 回归实际通过：当前可见批次 97,013/97,013，加载事件间隔
  最大 391/309ms，完成约 27.99/19.97 秒；点击 1.92/1.49 秒，仍不是普遍流畅保证。
- 本轮新截图能看到多组平面图和表格；不是参考 CAD 全图视觉验收。当前输入实际完成才
  显示已打开，旧 demo 帧/预算前缀不算就绪。证据/限制见 `docs/linux-dwg-ui-freeze.md` 顶部。
- 不提交原图/截图；WASM CPU 准备仍同步，Android/Web 运行、重复第二张大图、完整视觉
  与长期稳定性未验证。保留之前 clippy 修复；本轮修改未提交。

## Linux 桌面真实 DWG 无响应修复（2026-10-04，前轮历史）

**后续 clippy 修复**：`cad-app/tests/select_all.rs` 的拒绝断言改为 `expect_err`，
不改变拒绝路径和状态不变契约。`cargo test -p cad-app --test select_all --locked`
实际执行 **17/17 合成契约通过**；全树严格 clippy（按主机门禁排除 Android/Web）与
`cargo fmt --all -- --check` 通过。以下 clippy 失败是前轮历史证据，不代表当前状态。

**后续纠正**：用户手工 release 仍卡死，窗口启动后打开新增回归复现长停顿；最新
部分优化和未闭环项见本文「Linux release 后续排查」与 `docs/linux-dwg-ui-freeze.md` 顶部，
不能将下面前轮短测试作为加载响应已解决的证据。

用户确认当前环境为有 GPU/显示的 Wayland 桌面，不是历史无头环境。本轮使用用户提供的
仓库外 DWG 复现：主线程在复合几何子图元处理中反复深拷贝父实体；消除复制后仍生成
20,842,742 个底图批次，GPU 资源分配造成严重阻塞。修复只读子几何遍历，宿主 model/paper
场景采用不透明同样式折线的局部 line-list 合批，保留选择来源/样式/透明边界与局部坐标
精度约束；不修改 acadrust、数据库/测量几何，不隐藏填充或虚线。

新增 6 项合成契约和 1 项默认忽略的真实桌面回归；representation **139/139**、scene
**66/66** 通过。最终 debug/release 桌面回归分别实际通过：20 事件回调、3 CAD frames、
滚轮相机与 CAD 像素变化。首帧仍需约 **25–27 秒**，CPU 准备/GPU 上传仍同步，不能宣称
加载期间持续响应、流畅交互或整图视觉正确；Slint 默认选中设备的 vendor 未单独记录。

主机/wasm debug 编译、定向严格 clippy、fmt、架构/fixture/workflow/i18n 及最终 Linux
debug/release 二进制构建通过。全树严格 clippy 在未修改的 `cad-app/tests/select_all.rs`
失败（`err_expect`）；`cad-app --lib` **293 passed / 2 failed**（两项 unknown-layer 契约），
不记为全树门禁/测试通过。未执行完整 workspace 测试、无头离屏、Android/Web 运行、
参考图视觉验收或长期压力测试。详见 `docs/linux-dwg-ui-freeze.md`；外部图纸与截图未提交。

## 功能参考契约修订（2026-10-04）

用户明确 OpenCADStudio 仅为功能规格参考，不为源码参考。已同步规范 §2/§10/§13–15、
UI 需求总目录与实施追踪：从 `docs/ui-requirements/` 对照用户可见功能，核心独立实现，
不要求源码抽取、移植、上游依赖或算法采用；参考清单不扩大只读/测量/批注范围。
`docs/migration-map.md` 保留历史调查事实，旧迁移待办不再有效；既有实现与历史证据不删除。
本轮仅文档与 Rust 文档注释修订，不宣称新的产品运行或兼容性证据。
已运行 `python3 scripts/test-feature-reference-contract.py`：3 项合成文档契约通过。
未运行 Rust 编译/测试、Linux 离屏或其他平台验收。

## 独立分支并行审查第五轮（2026-10-04）

3 个并行子代理分别在 `review/20261004-{query-updates,annotation-files,history-recovery}`
独立分支/worktree 审查查询增量状态、批注文件校验、历史恢复日志，仅修改代码、合成契约
及专属文档，不编译/执行测试/检查/格式化/提交/推送。主控另外修复诊断资源预算饱和计数。
主控已格式化、提交并无冲突合并三个分支，统一执行 debug 编译与快速静态检查。
完整 Rust 测试、Linux 离屏和 release 构建均不在本轮执行范围。

- query：保留真实文档身份，以数据库查询建立绑定；增量通知严格遵循 `ChangeSet::follows`，
  未知/外部数据库、重放、错误前后修订与溢出边界均拒绝且不污染已发布状态。
- annotations：编码拒绝重复 ID 和原始保留边带；旧/不完整快照不能把当前数据库标为已保存。
- history：日志追加校验结构，整批恢复成功才发布，失败不留下部分重放状态。
- diagnostics（主控）：分类与总预算计数改为检查加法，最大上限仍拒绝溢出且保持计费原子性。

新增 **19 项合成 Rust 契约**（query 9、annotations 4、history 4、diagnostics 2）。
本轮最终统一代码树主机 debug 全目标编译（含 Linux App/Slint 与测试代码）、严格 clippy、
wasm 全 workspace lib 检查、fmt、架构/fixture/workflow/i18n 和 diff 空白检查均实际通过。
**NOT RUN**：Rust 契约未执行；完整测试、Linux 离屏、release 构建、真实 DWG/真实 GPU/
桌面窗口/浏览器/真机与远程 CI 未运行。详见 `docs/review-query-updates.md`、
`docs/review-annotation-files.md`、`docs/review-history-recovery.md`、`docs/review-diagnostic-budget.md`。
单个查询服务仍只绑定最新数据库；恢复临时克隆增加内存开销，身份/冲突检测/幂等/持久化
仍未闭环；导出编码成功不代表实际磁盘持久化，宿主须在成功写入后确认 revision。
提交前发现其他会话的功能参考规范/文档与几何注释修改；不纳入本轮提交，不覆盖。
这些修改后，全树 diff 空白检查因规范文档尾随空白失败；本轮文件的定向空白检查通过。
上述编译/门禁证据不推广到后续并发修改；共享交接文档只暂存本轮新增小节。

## 独立分支并行审查第四轮（2026-10-04）

主控启动 3 个并行子代理，分别在
`review/20261004-{db-transactions,domain-transforms,resource-resolution}` 独立分支/worktree
审查数据库事务、领域变换与资源解析。子代理仅修改代码、合成契约与专属文档，
不编译、不执行测试/检查/格式化、不提交或推送；主控审查后提交分支并统一合并验证。

本轮沿用快速 debug 门禁与英文提交，不运行完整 Rust 测试、Linux 离屏或 release 构建。
三个分支已由主控格式化、提交并无冲突合并：

- 数据库：实体换空间时清理旧块及动态状态引用；可见性切换不再误报非受控成员。
- 领域：极端尺度相似变换判定、NaN 尺度传播、非法容差/精确奇异矩阵拒绝、单位换算校验。
- 资源：字体目录拒绝路径型文件名；URL 路径段完整编码，避免 query/fragment/百分号歧义。

新增 **17 项合成 Rust 契约**（db 5、domain 9、resources 3）。主控在最终统一代码树
实际完成：主机 debug `cargo check --workspace --exclude app-android --exclude app-web
--all-targets --locked`（含 Linux App/Slint 与测试代码）、严格 clippy、wasm 全 workspace
lib 检查、fmt、架构/fixture/workflow/i18n 及 diff 空白检查，均通过。

**NOT RUN**：所有 Rust 契约仅编译、未执行；无完整测试、Linux 离屏、release 构建、
真实字体/真实 DWG/真实 GPU/桌面窗口/浏览器/真机运行或远程 CI 证据。
详见 `docs/review-db-transactions.md`、`docs/review-domain-transforms.md`、
`docs/review-resource-resolution.md`。保留限制：极端原始行列式、变换算子范数上界、
空间迁移后动态成员恢复/下游缓存、字体 base URL/重定向/重复解码仍须独立设计审查。

## 独立分支并行审查第三轮（2026-10-04）

3 个子代理在 `review/20261004-{geometry-robustness,scene-consistency,query-contracts}`
独立分支/worktree 仅修改代码、合成契约测试与专属文档，不执行编译、测试、格式化、
提交或推送。主控审查后格式化、提交并合并三个分支，再统一编译与执行提交门禁。

- geometry：向量长度溢出/下溢、大有限向量归一化、平移后面积消减、工作平面正交校验。
- scene：线拓扑三角形误计、不可能缓存预留破坏状态、预留与预算/队列计数溢出。
- query：空选择重复矛盾属性、属性查询文档身份校验、零上限分页越界。
- 主控：SHAPE 实例包围盒重复应用缩放，补充放大/缩小/平移合成契约。
- 用户要求已将 `AGENTS.md` 的 commit message 语言改为英文；默认门禁改为 debug
  全目标编译与静态检查。Linux 离屏、release 编译及完整测试不再是默认提交要求。
  已同步 core/build CI、workflow 校验器与变异契约、规范、README 和构建文档。
  Web/Android 发布打包配置保留，不作为本地默认 debug 门禁。

新增 13 项测试并加强 1 项既有测试；详细缺陷与限制见
`docs/review-geometry-robustness.md`、`docs/review-scene-consistency.md`、
`docs/review-query-contracts.md`、`docs/review-db-shape-bounds.md`。

统一最终代码树 debug 主机全目标检查（含 Linux App/Slint 与测试代码）、严格 clippy、
wasm 全 workspace lib 检查、fmt、架构/fixture/workflow/i18n 和 diff 空白检查已通过。
Python workflow 合成契约 Linux **5/5**、Web deploy **4/4** 通过；无远程 CI 执行证据。

**NOT RUN**：Rust 回归测试仅编译，未执行；完整 Rust 测试曾按旧门禁启动，随后在
编译阶段收到用户门禁变更并由主控 SIGTERM 终止，不计通过也不计测试失败。
后台链路因此未执行后续 Linux release 离屏和 wasm 检查；wasm 已由主控另行实际重跑通过。
原中止输出：
`/home/zenglanmu/.local/share/opencode/shell/da739b3910d63162f4687d9e8c13d613a0959f68/sh_104b620bd001up8HASB09SkXvs.out`。
本轮没有执行 Linux 离屏、release 构建、真实 DWG/真实 GPU/桌面窗口/真机验证，不以旧证据代替本轮。

## 独立分支并行审查第二轮（2026-10-04）

4 个子代理分别在 `review/20261004-{history,dependencies,mapping,resource-limits}`
独立分支/worktree 只写代码与测试，无编译/检查/格式化/提交/推送；主控审查、
格式化、单 crate 验证后整合开发分支，无冲突，主控执行完整门禁后提交/推送。
修复：历史合并消去及 patch 身份/重复验证、依赖失效 revision 连续性和根/深度预算、
仿射方向向量的大平移精度与矩阵校验、图片像素计数整数溢出。
主控同步接入 `resource.size_overflow` 诊断双语文案（203 keys）。

新增 31 项合成测试；主控 worktree 单 crate 测试通过：history 23、dependencies 20、
annotations 24+15、resources 21；i18n、稳定代码双语描述、Node 六套契约 53/53 通过。
统一代码树最终 fmt、严格主机 clippy、架构/fixture/workflow/i18n、wasm 全 workspace
lib 检查均通过；完整主机测试（Linux App + Slint，lavapipe 串行）
**1229 passed / 0 failed / 1 ignored**。Linux release 实际离屏 smoke 通过，证据
`/tmp/opencode/yacr-linux-20261004-020914-462863`（合成图纸/软件 GPU，2 CAD frames，
导航相机与像素变化，无渲染错误；不是参考图视觉验收）。
详见 `docs/review-branches.md`。真实 GPU/桌面窗口/真实 DWG/真机未运行。

## Rust 审查第一轮（2026-10-04）

后续按用户要求开启两个并行子代理，分别只写空间索引输入校验和测量数值稳健性
代码/回归测试，不运行编译、测试或格式化；已交付，主控审查并统一格式化/验证。
空间索引拒绝非法 bounds/ray 且更新失败不改旧索引；角度归一化避免大有限臂的溢出。
新增 5+6 项合成测试，主控执行 spatial **25**、measure **51**、history **17**、
resources **17 passed / 0 failed**。详见 `docs/review-spatial-measure.md`。
当前统一代码树严格主机 clippy、wasm 全 workspace lib 检查通过；完整主机测试
**1198 passed / 0 failed / 1 ignored**（合成/软件 GPU，真实 DWG 条件用例未提供输入，
不是其已执行证据）。新一轮 Linux release lavapipe 离屏 smoke 实际通过：
`/tmp/opencode/yacr-linux-20261004-013052-436072`，输入为合成图纸，
2 CAD frames、导航相机/像素均改变、无渲染错误；参考图视觉验收仍未执行。
Node 六套无 wasm/GPU 模块契约 **53 passed / 0 failed**。

集中修复 `cad-history`：批注撤销/重做数据库失败不再丢栈顶；UUID 数值不再
参与字节估算；绘图标记/payload 计数对称并保留原事务标记；合并执行预算淘汰；
待提交绘图步骤拒绝重入写历史；批注不能使用保留绘图合并键。新增 8 项合成回归，
修复前全部失败、修复后历史 crate **17 passed / 0 failed**。
详细审查范围与尚存预算/API 限制见 `docs/review-history.md`。

本轮已通过 fmt、严格主机 clippy（排除 Android/Web）、架构/fixture/workflow/i18n、
wasm 全 workspace lib 检查以及 Linux release 编译。
完整主机测试首次在编译阶段超时（120 秒），取消时限重跑后通过；此运行包含历史
修复，不包含后续资源/并行轮改动。原输出保存于
`/home/zenglanmu/.local/share/opencode/shell/da739b3910d63162f4687d9e8c13d613a0959f68/sh_10471d8940017619qmCkjQzIQN.out`。

资源轮补修 `MapResolver` 替换的预算重复计费和 `ResolverChain` 非缺失错误被吞；
2 项回归修复前均失败，修复后 resources/history 各 **17 passed / 0 failed**。
Linux release 已实际通过 lavapipe 合成离屏 smoke（含资源修复），证据
`/tmp/opencode/yacr-linux-20261004-011217-419337`。详见 `docs/review-resources.md`。
真实 DWG/真实 GPU/桌面窗口/真机未运行；不沿用历史证据作为本轮通过结论。

## 上轮状态（compact，2026-10-03）

**迭代 6：宿主捕捉接线 + minimal 命令集收窄 + 紧凑抽屉布局列表**：

- **宿主捕捉接线（`ws-snap-host`）**：`cad_app::host::snap_candidates_near` 从
  `drawing_pick_items` 构造 `SnapTarget`（烘焙实例变换，无法精确变换的 Text/Spline/Shape/
  Opaque/Compound 显式跳过，不误捕），`HostController::snap_hints_near_cursor` 在活动工具
  有世界光标时返回候选；Linux/Web 状态漏斗调用 `view.set_snap_hints`，捕捉提示层首次由
  真实几何驱动。
- **minimal 命令集收窄（`ws-minimal-cmds`）**：`preset_command_visible` 显式策略——minimal
  保留查看/测量/批注/打开/图层/布局/诊断（32 个）并隐藏绘制/编辑/模式/后端（8 个）；新增
  分区与单调性测试，CanvasOnly ⊆ Minimal ⊆ Full。
- **紧凑抽屉布局列表（`ws-layout-panel`）**：手机抽屉（`tools-open`）新增 52px 布局条，
  复用真实 `layout-rows` 切换模型/图纸空间；桌面标签不变。

主控修复：紧凑布局条使手机抽屉上移 52px，更新 `concept_offscreen` 的 measure 点选坐标并
注明原因。验证：fmt、严格 clippy（含 app-web）、架构/i18n/fixture/workflows、node 六套契约、
`cargo test --workspace --exclude app-android`（含 app-web）、wasm 全 workspace lib check、
Android aarch64 check、Linux release 离屏 smoke 全通过。仍为软件 Vulkan/合成。

**迭代 5：浮动溢出弹层 + 状态栏覆盖开关 + 真实捕捉提示**：

- **浮动 ribbon 溢出（`ws-floating-overflow`）**：配置 ribbon 分组超过 6 个命令时，▾ 现弹出
  **真实锚定 `PopupWindow` 飞出层**（不再内联撑高行高），每项派发并关闭；`close-on-click-outside`
  与显式 × 关闭。
- **状态栏覆盖开关（`ws-statusbar-toggles`）**：桌面状态栏新增坐标轴/网格/捕捉提示三个可勾选
  开关，经 `overlay-toggled` → `update_config_json({"view":{"overlays":{...}}})` → 单一漏斗
  重推，`view.overlays.*` 首次可由用户直接切换；标签取自新目录键 `overlay.axes/grid/snap_hints`
  （中英同键集，202 keys）。
- **真实捕捉提示（`ws-snap-hints`）**：复用 `cad_measure::SnapCandidate/SnapKind`（`cad-app`
  已依赖 `cad-measure`，架构允许），`snap_hint_overlay` 按捕捉类型绘制可区分标记
  （draw order 1_500_000），`OverlayInputs.snap_hints_input` + `CadView::set_snap_hints`
  接线，`view.overlays.snapHints` 门控；`snapHints` 关闭时该层不绘制。宿主适配留待后续轮。

主控修复合并后一处 `app.slint` 注释/换行错位，并补齐 `overlay.*` 目录键与状态栏开关标签接线。
验证：fmt、严格 clippy（含 app-web）、架构/i18n/fixture/workflows、node 六套契约、
`cargo test --workspace --exclude app-android`（含 app-web）、wasm 全 workspace lib check、
Android aarch64 check、Linux release 离屏 smoke 全通过。仍为软件 Vulkan/合成。

**迭代 4：ribbon 显示模式/溢出 + minimal 语义 + 宿主面板一致性契约**：

- **Ribbon 显示模式与溢出（`ws-ribbon-modes`）**：`RibbonGroup.display`
  (`iconAndLabel`|`iconOnly`|`labelOnly`，默认 iconAndLabel) 经 `ResolvedRibbonCommand.display`
  进入 Slint 模型（int 0/1/2），按项图标/文字；组内命令超过 6 个时渲染真实内联溢出列表，
  全部命令仍可派发（不静默丢弃）。**未做浮动锚定弹层**（诚实标注为内联）。
- **minimal 预设显式语义（`ws-minimal-preset`）**：新增私有 `PresetComponents` 真值表，
  `resolve` 改为查表；新增「minimal 组件语义」与「CanvasOnly ≤ Minimal ≤ Full 单调性」
  测试，行为对既有预设逐位不变。
- **宿主面板一致性契约（`ws-android-parity`）**：新增 wasm-free `scripts/test-host-panel-parity.mjs`
  （8 项），断言 Linux/Web/Android 三宿主的 panel/overlay setter 集合与
  `effective_config()` overlay 门控一致，并以 `handle.rs` 的 `set_*` 集合做子集守卫；已加入
  CI `web-host-contracts`。

主控修复合并后两处编译问题（`json_display` move、测试内 `ViewerConfig` 导入与 `RibbonGroup.display`
初值）。验证：fmt、严格 clippy（含 app-web）、架构/i18n/fixture/workflows、node 六套契约、
`cargo test --workspace --exclude app-android`（含 app-web）、wasm 全 workspace lib check、
Android aarch64 check、Linux release 离屏 smoke 全通过。仍为软件 Vulkan/合成。

**迭代 3：可配置 ribbon 命令派发 + 响应式视口契约 + 能力钳制修复**：

- **剩余 ribbon 命令（`ws/ribbon-unwired`）**：`view.reset` 现派发真实 `ResetView`；其余 8 个
  白名单命令因缺目标/手势保持显式理由（`ribbon.command_needs_target`/
  `ribbon.command_needs_gesture`），不伪造。新增“每个白名单命令都有分类动作/理由”的漂移
  回归测试（覆盖全部 `COMMAND_IDS`）。
- **响应式视口契约（`ws/panels-mobile`）**：新增 wasm-free `scripts/test-web-responsive.mjs`
  （9 项）覆盖 360×800/800×360/800×1280/1280×800 × DPR1/2/3 × 安全区，并含对
  `viewer_config.rs` 阈值（720/1200/540）的漂移守卫；已加入 CI `web-host-contracts`。
  顺带澄清 `cad-ui-slint::responsive` 的 600/1024 为历史帮助函数、非 live 权威（live 为
  `cad_app::viewer_config`）。
- **能力钳制修复（主控）**：`ui.commandOverrides.<id>.visible` 加入 `is_clamped_path`，
  用户偏好不得重新启用被预设/能力禁用的命令；修复 `apply_preference` 新建 override 键
  时绕过钳制的漏洞，新增回归测试。

主控验证：fmt、严格 clippy（含 app-web）、架构/i18n/fixture/workflows、node 触控/宿主/
可访问性/像素/响应式契约通过；`cargo test --workspace --exclude app-android`（含 app-web）
通过；wasm 全 workspace lib check、Android aarch64 check 通过；Linux release 离屏 smoke
通过。仍为软件 Vulkan/合成，未做真实 GPU/真机验收。

**迭代 2：数据驱动 Ribbon + Web 触控门控**：主控派发两个隔离 worktree 子代理（ribbon 分组、
web 触控门控与契约），主控统一合并/编译/门禁。

- **数据驱动 Ribbon（`ws/ribbon-groups`）**：`ui.components.ribbon.tabs[].groups[].commands[]`
  经 `cad_app::resolve_ribbon` 解析为纯模型并按 `commandVisibility` 过滤，`chrome.rs` 映射为
  新 Slint `[RibbonTabModel]`（`ribbon-model.slint`），`ribbon.slint` 在
  `ribbon-config-driven` 为真时渲染配置选项卡/分组/命令按钮，`on_ribbon_command` 映射到真实
  命令（未支持的合法命令显式 `ribbon.command_unsupported`）。无自定义 tabs 时内置面板不变。
  新增 10 个 i18n 键（中英同键集，197 keys）。
- **Web 触控门控与契约（`ws/web-interaction`）**：新增 wasm 导出 `viewer_interaction_json`、
  JS 谓词 `interactionAllowsTouch`/`readInteractionAllowsTouch`；`touch.js` 在 `touch=false`
  时不再捕获指针或转发触摸，并在 `yacr-config-changed` 时重估。`scripts/test-web-touch.mjs`
  扩展到 12 项契约（含畸形 JSON 默认放行的文档化行为）。

主控修复合并后一处生成类型导入错误（`Ribbon*Model` 由 `include_modules!` 提供）。验证：
fmt、严格 clippy（工作区，含 app-web）、架构/i18n/fixture/workflows、node 触控契约通过；
`cargo test --workspace --exclude app-android`（**含 app-web**）**1151 passed / 0 failed /
1 ignored**；wasm 全 workspace lib check、Android aarch64 check 通过；Linux release 离屏
smoke 通过。仍为软件 Vulkan/合成，未做真实 GPU/真机验收。

**UI 配置门控三工作流集成（迭代 1）**：主控派发三个隔离 worktree 子代理，子代理只写代码与
单元测试，主控统一合并、编译、门禁。三个工作流文件不相交，合并零冲突：

- **overlay 门控（`ws/overlay-gating`）**：`view.overlays.*` 首次真正控制绘制。
  `OverlayInputs` 增加 `visibility`（默认全开，保持既有调用行为），并进入
  `overlay_fingerprint`；`prepare_inner` 据此门控选择高亮、已提交批注，并新增
  `axes_overlay`/`grid_overlay`（按 `Drawing::bounds` 生成、`GRID_MAX_LINES=400` 上限、
  无 bounds 时 `overlay.bounds-unavailable` 诊断）；预览十字受 `snapHints` 门控。
  `CadView::set_overlay_visibility` 由 Linux/Web/Android 状态漏斗按有效配置推送，切换只
  重建瞬态叠加层不动底图。
- **interaction 门控（`ws/interaction-gating`）**：共享 `UiAdapter` 提升配置文件存储后，
  在事件时按 `interaction.pointer` 早退指针/滚轮/画布拾取；命令别名仅在
  `keyboardShortcuts` 为真时展开（完整命令名仍可用）；Web 触控 wasm 入口按
  `interaction.touch` 空操作。新增 `crates/cad-ui-slint/tests/interaction_gating.rs`。
- **原生偏好持久化（`ws/native-prefs`）**：`apps/app-linux/src/host/config_file.rs` 从
  `$XDG_CONFIG_HOME/yacr/`（回退 `$HOME/.config/yacr/`）读取 `config.json`（完整
  `ViewerConfig`）与 `preferences.json`（`allowedPaths` 投影），新增
  `--config`/`--preferences`；`LinuxApp::apply_user_preference_json` 应用后原子持久化
  投影。无 XDG/HOME 时显式不持久化；解析失败保留默认并报告。

主控验证：`cargo fmt --check`、严格 clippy（工作区，排除 android/web）、架构/i18n/fixture/
workflows 全通过；`cad-app` 275、`cad-ui-slint` 82+1 离屏+1 门控、`app-linux`
`host_contracts` 4 + `host_config_disk` 1 + `verify_ui` 1 + `verify_ui_drawing` 1
（lavapipe 离屏）通过。仍为软件 Vulkan/合成，未做真实 GPU/真机验收。详见
`docs/ui-redesign.md`「仍未闭环」与 `docs/linux-app.md`。

**verify-ui 循环轮 E：minimal 预设语义契约（本轮）**：`verify_ui.rs` 场景加入
`minimal` 预设断言：保留应用框架/图层面板/导航工具栏/布局标签，隐藏 Ribbon、命令栏、
状态栏（`UiPresentationModel` 的既有语义，此前只有 canvas-only 被验证）。截图
`05b-minimal`。此前轮 D：`ui-drawing` 增加“切到每个图纸布局再切回模型后模型画面逐字节
恢复”的断言。轮 A/B/C 见下。


**verify-ui 循环轮 C：命令面 ESC/CONFIRM 作用于当前命令（本轮）**：按 AutoCAD 约定修复
命令输入区：`CANCEL`/`ESC` 此前只取消绘制捕获，现在按活动状态依次取消测量/批注/绘制并
退出平移；`CONFIRM` 也按活动状态作用于测量/批注/绘制，而不是只确认绘制。新增
`command_line_confirms_or_cancels_whatever_tool_is_active` 源码接线契约，并在
`verify_ui.rs` 场景加入“开启测量/批注/平移后 ESC 必须结束”的断言。`cad-ui-slint` 单元
80 passed、`verify_ui` 场景通过。


**verify-ui 循环轮 B：真实图纸操作 + 选择缺陷修复（本轮）**：新增
`apps/app-linux/tests/verify_ui_drawing.rs`（`scripts/verify-ui.sh` 的 `ui-drawing`
层），用已提交开源 fixture `fixtures/dxf/qcad-examples/entities.dxf` 真实打开后执行：
多图层显隐改变并恢复合成像素、模型/图纸布局切换、真实指针点选几何、清除后再次点选、
真实 MOVE 一次事务且撤销恢复、命令别名 `L`/`C`/`TR`。首次运行发现**真实缺陷**：
`CommandId::Select` 会把会话置为 `ToolState::Selecting`，而 `apps/app-linux` 的
`Navigation::select` 只在 `ToolState::Idle` 时拾取，因此第一次点选后无法再更改选择
（清除后点选同样失效）。已改为允许 `Idle | Selecting`，测量/批注/平移仍阻止拾取；
“清除后再次点选”保留为该场景的回归断言。对比只取 CAD 画布区域以排除状态栏文案；
大图纸场景重建较慢，用“连续两帧相同”作为收敛条件，并保留确定性自检。证据见
`docs/verify-ui.md`；仍为软件 Vulkan/合成/开源 fixture，不代表真实 GPU/真机。


**verify-ui 循环轮 A：AutoCAD 深色外观 + 命令面操作（本轮）**：按用户要求把循环焦点从
批注转到“UI 匹配 AutoCAD 风格 + 其它操作”。外观：`ui/theme.slint` 改为 AutoCAD 深色
（深灰 chrome/panel、浅色文字、蓝色选中、近黑模型空间），`ribbon`/`button`/`canvas`/
浮动工具栏的硬编码浅色同步；`YacrWindow` 设 `Palette.color-scheme = dark`，使
std-widgets 的命令栏输入、下拉、确认按钮、进度条也变深（此前为浅色，与深色 chrome 冲突）。
`concept_offscreen` 的“chrome 为浅色”像素断言改为“chrome 为深色”。场景新增：
标题/Ribbon/侧栏/画布**平均亮度深色检查**，以及命令输入区真实操作（`TOOLS`/`PANELS`
开关、`LINE`+`ESC`、`CIRCLE` 未取点 `CONFIRM` 显式报“参数无效”且保留捕获、`MOVE` 无选择
显式报“移动需要先选择对象”、未知命令显式报错、`FIT`）。迭代修复：`ColorScheme` 是内建
枚举不能从 `std-widgets.slint` 导入（首次编译失败已记录并改正）；场景初版把不完整
`CIRCLE` 的拒绝文案猜成“点”，实际为 `draw.error.invalid`（“参数无效”），按真实目录修正。
证据见 `docs/verify-ui.md`、`docs/ui-redesign.md` 顶部说明；仍为软件 Vulkan/合成图纸。


**verify-ui 无头 UI 循环（本轮）**：新增轻量入口 `scripts/verify-ui.sh` +
`scripts/verify-ui-summary.py`，对应测试计划第二层：用 Slint 官方 offscreen 平台
（`Platform`/`WindowAdapter` + FemtoVGWGPURenderer）+ Mesa lavapipe 软件 Vulkan，实际
运行真实 `LinuxApp`、自动操作控件、逐层超时、截图、panic 扫描、结构化证据包
（`verify-ui.json`/`environment.json`/`logs`/`screenshots`）。**不安装 X11/Wayland**，
offscreen 平台即虚拟窗口运行时；`environment.json` 如实记录 `xvfbAvailable=false` 与
`realDevice=not-run`。新增 `apps/app-linux/tests/verify_ui.rs` 固定场景：测量取消不写库、
距离自动完成并存批注、撤销重做、LINE 提交、文字批注需文字、矩形批注两点自动提交、
批注显隐/删除改变并恢复合成像素、**图层显隐改变且恢复合成像素**、布局/标准视图、
真实 `WindowEvent` 滚轮缩放与左键拖动平移、**2D→3D→2D 合成帧逐像素无损往返**、
canvas-only、中英切换、侧车导出回导、四尺寸矩阵。迭代中发现无字体时文字批注按设计
不绘制、Measurement 批注不进叠加层，显隐断言改用不依赖字体的矩形批注，未把设计限制
误判为缺陷。
首次运行发现脚本预创建证据目录导致 `--headless` 正确拒绝覆盖（exit 17），已修复为不预建；
场景初版误把“距离可确认”当契约，实际距离两点自动完成、`can_confirm` 只服务开放型工具，
已改为分别覆盖两条路径。`verify-ui.sh` 全层通过（6 层 + 可选真实图纸层），
`YACR_TEST_DWG=fixtures/dxf/qcad-examples/entities.dxf` 时 app-dwg 通过；证据
`/tmp/opencode/yacr-verify-ui-run2-*/`、`...-dwg-*/`、`...-iter-*/`。范围与限制见
`docs/verify-ui.md`。仍未做：真实 GPU、窗口系统、真实 DWG、真机。


**QCAD examples 语料与图元扩展（2026-10-03）**：下载并提交 QCAD `examples/` 其余 9 个
DXF 与 `flange.svg`（`fixtures/dxf/qcad-examples/`，来源/SHA/许可见 `SOURCE.md` 与
`fixtures/manifest`）。实现 LEADER（顶点折线+实心箭头）及通用 Polyline、ATTRIB/ATTDEF、
MESH/PolyfaceMesh/PolygonMesh、WIPEOUT、HELIX；带字体名的 Text 由 `Unsupported` 改判
`Unverified`。新增无需 GPU 的 `crates/cad-cli-tools/tests/dxf_samples.rs` 与
`scripts/check-qcad-examples.py`（lavapipe 批量出图+覆盖报告，可选 ezdxf 参考导出）。
实测 **9/9 通过**。随后补齐 `VIEWPORT`（边框）、`TOLERANCE`（框+文字）、`MLINE`（中心线）、
`MULTILEADER`（引线+文字）接线，并把 `Text` 统一为 `Unverified`（类型已支持，字体依赖宿主）；
新增 `dxf_samples` 的**零-`unsupported` 验证**（`proxy-report`），9 语料 + `flange` 全部通过。
按用户要求**放弃 ezdxf 像素对比**。随后按顺序补齐：`RAY/XLINE`（裁剪到模型范围）、
`SHAPE`（SHX 字形，新增 `SemanticGeometry::Shape`）、全部 DIMENSION 子类（角度/坐标/
圆弧长/大半径）、`TABLE`/`RASTERIMAGE`。另修复纸空间 `plot`：纸张单位从纸名解析、默认选有 viewport 的
布局、超限纹理返回结构化 `gpu_failure`；flange 纸空间边框/标题栏可出图（视口比例近似）。
另补回 DXF STYLE XDATA 字体名（`1001 ACAD`/`1000`，DXF-only 扫描，组件优先，不改
acadrust）。仍剩外部内容加载、光栅像素纹理、纸空间视口合成保真等 `Partial`；详见
`docs/dxf-entity-coverage.md`。

**QCAD flange 渲染回归样本（2026-10-03）**：按用户要求提交 QCAD 开源 `flange.dxf`
及 PNG/PDF 参考（`fixtures/dxf/qcad-flange/`，来源/许可/SHA 见其 `SOURCE.md` 与
`fixtures/manifest`），新增 `scripts/check-dxf-reference.py` 与无需 GPU 的
`crates/cad-cli-tools/tests/dxf_fixture.rs`。原生 release + lavapipe 实际执行：
419 entities / 223 model、0 build failure；实现无匿名块 DIMENSION 合成（线性/对齐/半径/
直径：线、实心箭头、按 DIMSTYLE 的测量文字）后表示 329 primitives（含 11 箭头 mesh + 6
标注文字），`dxf_fixture` 2/2 通过并新增箭头/文字回归断言。带宿主字体时 1024×768 非空帧
1.87%，四视图几何、剖面线、尺寸线与测量文字与参考一致；仍 `Partial`，因为文字绘制需要
宿主字体，且纸空间图框/标题栏不在 `render`（模型空间）范围内。证据
`docs/validation-dxf-flange.md`、`/tmp/opencode/yacr-dxf-reference-font/`。同时按用户要求
把“外部图纸/参考图不得入库”改为“授权可再分发即可入库并记 provenance”。

**Linux 文件选择/关闭崩溃/后端诊断修复（release 门禁进行中）**：桌面打开经 ashpd 调用系统
`xdg-desktop-portal` 文件选择器（独立线程等待，取消/失败区分，脏状态选择前后检查）；
`--open` 为启动路径，无窗口模式仍不调用选择器。Slint setup/teardown 不再写图片属性，
避免用户 Wayland 关闭回溯中的 RefCell 重入。共享设备实际 API 决定能力/诊断，软件
Vulkan/原生 Vulkan 不再误标 webgpu。契约与执行范围见 `docs/linux-app.md`；本轮未做
真实桌面 portal/Wayland/真实 GPU 验收，不沿用历史门禁作本轮证据。主机本轮合成回归
**1103/0/1 ignored**、针对性 Linux/UI 83 项、fmt/严格 clippy/四项 Python 门禁通过；
release 主验收与 WASM 门禁仍在执行（后台串行，日志 `/tmp/opencode/yacr-linux-picker-*.log`）。

**UI/DXF/文字修复（已提交推送及发布）**：用户指出图标/禁用/导航与规范不一致，要求支持 DXF、
网络 DWG 文字对比，只跑 Linux/Web 构建、不跑完整测试，并发布 Pages。范围及真实文字
14 倍放大缺陷证据见 `docs/ui-dxf-text-fixes.md`；缺原字体仍显式 Partial，不能宣称视觉全通过。
Linux/Web release 构建、4 项针对性契约、Linux DXF smoke、Web DXF File API 与真实四按钮
点击/平移抽查通过。大图 Linux App + TIMES 尝试 240s 超时，CLI 对比有效但不算宿主性能通过。
**未跑完整 workspace/clippy 门禁（用户要求）**，不用上一轮全量通过替代本轮证据。
实现提交 `d269140` 已推送；Pages 生产部署 `a6ea4bfe-9c00-40ae-8133-76535a242983`
API 确认成功，`https://yacr-examples.pages.dev/` 实际浏览器合成 DXF 重跑通过。

**并行三工作流（本轮新增，2026-10-03）**：主控派发三个隔离 worktree 子代理，分别闭环
三处显式缺口，主控统一合并/构建/门禁；子代理只写代码与单元测试，所有构建与运行由主控执行。

- **批注 sidecar 保真（`ws/annotation-fidelity`）**：`cad-annotations` 现在保留注解对象及
  嵌套 `geometry`/`style`/`precision` 的未知字段（私有顶层边带
  `yacr.nested_extensions`，`AnnotationFile::nested_extensions` 为类型化视图），编码时与
  已知字段冲突或命名缺失注解则拒绝；时间戳解码同时接受整数 Unix 毫秒与 RFC 3339
  （`Z`/偏移/小数秒），非法或 `modified < created` 拒绝；新增
  `MIN_SCHEMA_VERSION`/`migrate_file` 显式版本策略（更高 `Unsupported`、更低且无确定性
  迁移则 `CorruptData`）。`RecoverySnapshot` 保留未知顶层字段并拒绝更新版本（不降级）。
  新增 6 项注解契约测试 + 3 项恢复快照测试。详见 `docs/annotations.md`、`docs/recovery.md`。
- **矢量出图（`ws/vector-plot`）**：`cad-representation::plot_vector` 新增纯 CPU 路径文档 +
  自包含 SVG/PDF writer（无第三方 crate、不建 GPU 设备）；PDF 透明度用真实
  `ExtGState`（`/ca`/`CA`，页面 `/Resources` 引用、`gs` 选择后恢复不透明）。CLI 新增
  `--plot-format png|svg|pdf`（默认 png）。无法表达为路径的图元逐项 `vector.*` 诊断并降级
  `completeness`，不静默丢图。176 项相关单测通过（含 SVG/PDF 结构、透明度资源、诊断）。
  详见 `docs/plot.md` §9、`docs/cli.md`。
- **ViewerConfig 协议（`ws/viewer-config`）**：`cad-app::viewer_config` 补齐
  `features`/`view.overlays`/`interaction`/`commandOverrides`/`userCustomization.allowedPaths`
  /Ribbon tabs/groups/commands/panel placement/initiallyOpen；解析顺序
  默认→预设→宿主→宿主允许的用户偏好，用户偏好 clamp（不得重新启用被禁止项），
  数组整体替换、显式 false 生效、失败原子保留旧值与 revision；`ViewerConfigStore` 提供
  revision + 数据驱动 observer + 有效配置查询。Web 暴露
  `window.yacr.setConfig/updateConfig/applyUserPreference/clear/config` 与
  `yacr-config-changed` 事件，localStorage 仅投影 allowedPaths；Slint 按新的
  `UiPresentationModel` 分面板/overlays/features 门控。详见 `docs/ui-redesign.md`。

主控集成：合并三支后修复 3 处子代理遗漏（app-web 格式、`web.rs` 构造 CustomEvent、
`run.rs` wasm 下 `run_plot` 引用、`chrome.rs` 未用 import、app-web 未用 re-export）；
门禁与运行证据见下方“三工作流集成轮”。

**Linux App 主验收（本轮完成）**：`apps/app-linux` 提供桌面与无窗口共用 LinuxApp，
真实 HostController 命令与数据库/共享 Slint/wgpu 桥，release smoke 入口
`bash scripts/check-linux-app.sh`。规范 §11.0、AGENTS/build/ci 已改为 Linux 主验收，
GitHub `linux-app` 默认启用；原生质量层包含 Slint/Linux App 串行测试。执行证据与未接线
范围见 `docs/linux-app.md`；未运行远程 workflow/桌面窗口不计通过。
最终主机合成测试 1048 passed / 0 failed / 1 ignored，fmt/clippy/Python/WASM 门禁通过。
release 实际运行证据 `/tmp/opencode/yacr-linux-app-final-retry/`：软件 Vulkan，2 CAD 帧、
4553 画布像素导航变化；已抽查合成截图，无真实 DWG/桌面窗口/真实 GPU 验收结论。

**Concept UI 重设计进行中**：设计依据 `docs/ui-spec/`，配置/布局纯契约已新增到
`cad-app::viewer_config`（3 项合成测试已执行通过）。用户授权解除原生 Slint 编译限制，
后续使用 Linux Slint/wgpu/lavapipe 离屏主测试，不跑 Android 模拟器，最后 WASM 浏览器抽查。
完整范围与配置未闭环项见 `docs/ui-redesign.md`。
Slint 已改为浅银/蓝选中 concept：桌面左侧面板+布局/状态；手机标题+底部四组和工具/图层抽屉。
新增 Linux 官方 FemtoVG/wgpu 离屏平台；原生 UI 合成单元 76 项、真实离屏集成 1 项通过。
截图 `/tmp/opencode/yacr-concept-round4/`。浮动控件的浏览器触控排除命中已补契约。
本轮三个里程碑已完成限定范围：配置布局子集、共享 Slint concept 重排、原生主回归及
桌面 WASM 第二层抽查。最终 Linux 合成 1045 passed / 0 failed / 1 ignored，严格 clippy
（含 UI 和 Web wasm）及全部门禁通过。原生截图 `/tmp/opencode/yacr-concept-final-native/`；
浏览器 `/tmp/opencode/yacr-concept-final-web/integration.png` 人工抽查确认初始合成图不再裁切，
导航/双语/偏好/空批注往返通过。完整配置协议、真实 DWG/真实 GPU/真机仍未验证。

**真实 DWG 回归（最新）**：以用户外部 `anteen.dwg` 与去色参考图，优先 Linux
原生 wgpu/lavapipe。修复 2D 投影重复 Y 翻转、CLI render/plot 忽略 `--font`；
核心串行 **965/0/1 ignored**。原生导入/出图通过，但填充/字体保真未验收。
WASM/Playwright 发现打开后旧演示纹理停滞，增加事件循环唤醒后真实文件
CAD 帧 **1 → 4**，最终 WebGL2 打开/出图 smoke 通过。修复前 120 秒超时证据保留；
目录 99 个字体但本图注册 0 个，Web 缺文字，**视觉保真仍未通过**。
新增可复用 `check-dwg-native.py` / `check-web-dwg.mjs`；证据与命令见
`docs/validation-dwg.md`，产物 `/tmp/opencode/yacr-canteen-validation/`。

**修复与 Pages 发布轮（最新，2026-10-03）**：继续修复 Web 第二触点/touchcancel 不取消
shell 绘制捕获、Web/Android 成功换图纸后残留旧取点、TRIM 使用固定世界拾取容差的问题。
新增 `check-web-drawing-safety.mjs`，真实浏览器证明第二触点（未移动）、touchcancel、换图纸
之后确认不写库；TRIM 复用共享屏幕像素拾取容差。核心 **963/0/1 ignored**、JS **28 passed**，
跨平台编译/严格 clippy/架构/i18n/fixtures/workflows 通过。

发布候选 `/tmp/opencode/yacr-pages-release/` 含完整 99 个同源 CAD 字体；四种工具、输入安全、
桌面 UI 通过。mobile 三场景独立进程复跑通过（`/tmp/opencode/yacr-pages-mobile/`），
此前超时记录不删除；最终 Ribbon 触控复验仍超时，**不宣称最终 Ribbon 回归通过**。
修复提交 `01f9f2b` 已推送 main 并部署至已有 `yacr-examples` 的生产分支。
生产地址 `https://yacr-examples.pages.dev/`（自定义域名 `yacr-examples.snakeheartgo.top`）；
两地址 wasm 哈希匹配验证包。线上桌面 UI、LINE/CIRCLE、输入取消安全通过；MOVE/TRIM
线上截图超时，本地像素通过，**不能宣称所有线上绘制回归通过**。详情见 validation-web §11。
Android 本轮仍只有编译门，未新增 APK/真机/WebGPU 证据。

**绘制/编辑合并后主控验收（历史）**：基于 `39077ec` 的数据库写事务、应用命令、
UI 工具及 Web/Android sink 合并结果，完成核心与无头 Web 验证；没有另启 subagent。

- 修复命令后渲染仍持有旧底图 Arc：两宿主状态漏斗通过 `CadView::sync_drawing`
  发布当前数据库快照，创建/编辑/撤销/重做可见；导航与叠加保持原 Arc，不重导入。
- 修复绘制主指针同时进入宿主导航/选择：MOVE 取空白锚点不再清空选择，取消不残留
  意外选择高亮。绘制 sink 返回真实命令错误，失败保留捕获参数，不冒充提交成功。
- 新增 `scripts/check-web-drawing.mjs`：真实 Slint 命令栏取点/确认，经宿主与事务到
  WebGL2；LINE/CIRCLE/MOVE/TRIM 四场景均改变底图像素，撤销恢复相同 CAD 像素哈希；
  LINE/CIRCLE 确认前与取消不新增实体，重做恢复实体计数。每种工具独立进程串行运行。
- 核心串行 **963 passed / 0 failed / 1 ignored**；JS **27 passed**；fmt、核心与 app-web
  严格 clippy、架构、i18n（159 keys）、fixture/workflow、wasm workspace lib、UI/Web wasm
  测试编译、Android aarch64 测试编译通过。UI/宿主 Rust 测试为**编译而非执行**。
- 本地无头 Chromium/SwiftShader：四工具、overlay、ribbon（1280/390/320px）、桌面 UI
  通过；证据 `/tmp/opencode/yacr-drawing-final/`、`/tmp/opencode/yacr-draw-final-*`。
  `check-web-mobile.mjs` 两次超时，本轮**未通过**；连续截图停滞记录在 validation-web §9。
  本轮未部署生产、未运行 Android APK/真机/WebGPU，测试 bundle 不含 CAD 字体。

详情：`docs/drawing-edit.md` §6、`docs/validation-web.md` §9。绘制/编辑是受控内存库
子集，不支持保存修改后的 DWG；不能据此宣布规范的“未来底图编辑”完整产品验收。

**UI 宿主接线轮 2（历史，iterations 2–4）**：Ribbon 文档的宿主接线与渲染叠加全部闭环，
并继续补齐审计项。四个迭代的顺序合入均通过主控验证：

- **叠加层宿主接线**：`CadView::set_selection_highlight/set_measurement_preview/
  set_annotation_preview` 已由 Web 与 Android 的状态漏斗在每次命令/拾取/打开/确认取消后
  推送；选择变化复用底图与批注 Arc。
- **选择高亮端到端修复**：修复 Web 画布点击永不可选（`!was_dragging` 门把每次点击都当
  拖动拒绝）；现在单击命中即派发 `Select`，无头截图证明圆被高亮且清空回到基线。
- **测量存为批注（F06/F07）**：`CommandId::SaveMeasurementAsAnnotation`，一次事务/一次
  撤销，无记录时 `InvalidInput`；UI 按钮「存为批注」仅在确认记录存在时可用。
- **查看/工作模式（U02）**：`CommandId::SetMode` + 命令栏开关，权限仍在命令层强制。
- **窄屏命令栏**：手机上模式开关移入可展开行，320px 不再挤压命令输入框。
- **异步导入（F01）**：核心快照 + `ImportProgressUiState`；Web 在 wasm（无线程）只推真实
  终态并诚实说明，Android 走真实 `std::thread` worker + 100ms 轮询 + 取消；面板无伪造进度。
- **可访问性（U12）/ 状态重叠（U08）**：`aria-live=polite` 人类摘要区域、`role=application`
  可聚焦画布、`host-state` 就绪时隐藏且不再双重播报。
- **U07**：Android 暴露 `set_surface_size` 入口（Activity 回调仍为显式未接钩子）。

证据：核心串行 **830 passed / 0 failed / 1 ignored**；JS 契约 **26 passed**；i18n
**151 keys**；架构、fixture manifest、workflows、Android aarch64 检查全通过；无头
Chromium（SwiftShader/WebGL2）ribbon/UI/mobile/overlay 四套脚本在 1280×800、390×844
DPR3、320×740 DPR2 通过，并直接对 **Cloudflare Pages 生产 URL 重跑通过**。新增
`scripts/check-web-overlay.mjs` 端到端证明选择高亮改变像素并清空回基线。仍开放：真机、
WebGPU/真实 GPU、Android Activity resize/SAF。（绘制/编辑后续进展见上方主控验收。）

**UI 宿主接线轮（iteration 1）**：Ribbon 文档里"宿主连接器（本轮范围外）"与"预览几何尚未
接线"两项已落地并端到端验证。三个并行 workstream 已合入 main：

- **Web 宿主**：`browser/state_push.rs::push_panel_state` 成为唯一状态漏斗，在每次
  命令/打开/批注变更/启动恢复后推送历史可用性（undo+redo 独立）、测量、图层+有序 id、
  属性、批注+有序 id、布局、诊断抽屉（来自真实 `ImportReport`）；安装
  `WebCanvasPickMapper`（逻辑像素→世界点，退化输入返回 None）；无工具时的轻触经
  `pick_at_screen` 派发 `Select`（空命中＝清空选择），不再静默。新增
  `diagnostics_report_json`。空态/多值文案全部来自目录。
- **渲染叠加**：`cad-app::render_scene::overlay` 新增选择高亮（按 `drawing_pick_items`
  展开 INSERT，未解析引用显式诊断不伪造）与工具预览（测量/批注 rubber-band + 捕捉点
  + 矩形/椭圆）批次；`CadView::set_selection_highlight/set_measurement_preview/
  set_annotation_preview` 独立 `overlay_revision`，选择变化复用底图与批注 Arc。
  `prepare_shared_with_overlays` 为向后兼容的新入口。高亮 `draw_order` 900_000，
  预览 2_000_000，位于底图与批注之间/之上。
- **Android + Web 手势**：Android 接入同一状态漏斗 + `AndroidCanvasPickMapper` +
  tap/drag 判定（`InputPolicy`）+ `apply_surface_size`（横竖屏不改相机中心；Activity
  resize 回调仍为显式未接钩子）；修复 `web/host/touch.js` 手指数变化时重定基准会跳变
  的缺陷，`node --test scripts/test-web-touch.mjs` 6 passed。

证据：核心串行 **819 passed / 0 failed / 1 ignored**；wasm lib check、clippy、
架构、i18n（132 keys）、fixture manifest、Android aarch64 check 全通过；无头 Chromium
（SwiftShader/WebGL2）ribbon/UI/mobile 三套脚本在 1280×800、390×844 DPR3、320×740
DPR2 通过，并直接对 **Cloudflare Pages 生产 URL 重跑通过**。截图
`/tmp/opencode/yacr-ribbon-final/`、`/tmp/opencode/yacr-ui-final.png`。仍开放：真机、
WebGPU/真实 GPU、Android Activity resize/SAF、INSERT 根选择高亮（仅叶子高亮）。
详见 `docs/ribbon-ui.md`、`docs/ui.md`、`docs/panels.md`。

**Ribbon UI 布局轮**：保留 Slint/shared-wgpu，界面拆成 app/ribbon/button/command-bar/
canvas/panels，桌面 Ribbon 与命令栏可收展；手机默认仅底部命令栏，TOOLS 打开工具。
手机 48px 按钮，真实 Slint 文件选择器导入 DWG、双指缩放与拖动已接应用命令。
未接绘制/编辑菜单禁用且显式标注，交接详见 `docs/ribbon-ui.md`。独立无 Slint CAD
显示 wasm 尚未提取，当前 app-web.wasm 不算独立引擎，由下一阶段实施。

**渲染桥职责重构**：继续采用 Slint/shared Device/Queue。CPU controller 移至
`cad-app/render_scene`，GPU runtime 与 Slint presenter 分离；字体 revision、
上传成功后发布、底图/批注组更新、画面脏标记、纹理身份和设备生命周期已接线。
`IncomingDocument=None` 明确表示关闭文档。详见 `docs/bridge-runtime.md`。
最终核心串行测试 875 passed / 0 failed / 1 ignored；新增 GPU 契约实际运行。
桌面及 320/390px 高 DPI 无头 WebGL2 回归通过，UI-only 语言切换不增加 CAD 绘制数。
原生 Slint 测试仍受环境阻塞；wasm 测试仅编译。后台 CPU 准备、分批上传预算和真实
设备丢失恢复尚未验收。并行核心 GPU 测试一次驱动 SIGSEGV，串行重跑通过，不能隐去。

**宿主结构拆分**：Web Rust 宿主拆为 documents/annotations/fonts/input/persistence，
JS 宿主拆为 i18n/files/renderer/runtime；Slint 桥分离 scene/camera/tests，既有接口和
13 项桥测试保留。核心 870 passed、新增 JS 契约 5 passed、Web release 与两次本地
Playwright 通过（含模块 HTTP 200、批注下载/空 sidecar 回导与状态保留）。CI 与部署
检查包含所有 JS 模块。结构与环境阻塞见 `docs/code-structure.md`、
`docs/validation-web.md` §6；原生 Slint 测试未执行完成，不能当作通过。

四个并行 workstream 已合入 main 并验证：

- **Android 运行闭环**：x86_64 release APK 在无头模拟器安装/启动/渲染；修复画布输入
  未接线（现在单指拖动平移、滚轮/捏合缩放）、初始状态文案、后端日志；像素 diff 证明
  平移生效。证据 `docs/validation-android.md`（含截图/日志）。
- **Web 运行闭环**：修复桥固定 WebGPU、Slint 缺 `renderer-femtovg-wgpu`、wasm 轮询
  误判设备丢失、WebGL2 MSAA present 失败；`web-dist/` 在无头 Chromium 以 WebGL2 通过
  加载/导航/双语，语言偏好持久化；B29 冒烟脚本修复。证据 `docs/validation-web.md`。
- **3D / 纸空间宿主接线**：bridge 按 `SpaceSelection` 与视图模式分派 `render`/`render_3d`，
  UI 提供 2D/3D、投影、标准视图、拖动轨道与布局选择；退化/不支持视图显式诊断。
  宿主编排者已补 `CadView::sync_session`（空间+相机+模式）。`docs/view-3d.md`。
- **透明度与代理**：acadrust `Transparency`（ByLayer/ByObject/ByBlock）经
  `DisplayFragment.alpha` 进入透明管线；代理保留全部片段与真实来源/精度。软件 Vulkan
  透明合成测试通过。`docs/render-order.md`、`docs/proxy-support.md`。

核心测试 **573 passed / 0 failed**；Wasm、Android target、fmt、clippy(0)、架构、
`check-i18n.py`（116 keys）、`check-fixture-manifest.py`、`check-workflows.py` 全通过；
`cad-render-wgpu` 在 lavapipe 下 47 passed（含透明合成）。

**集成轮 2（核心 CAD，Linux 构建）**：曲线升级为真实 NURBS + 椭圆 OCS 法向 +
仿射真椭圆 + 解析交点（`docs/curve-geometry.md`）；F15 ACIS 打通
acadrust SAT/SAB → 中性 B-rep → 平面/球/柱/环面子集离散（`docs/kernel-acis.md`，
合成夹具入 manifest）；纸空间 4 角视口/正确比例 + 空间感知测量（`docs/layouts.md`、
`docs/measure.md`）；对象捕捉六类 + HATCH 多环含孔洞填充。核心测试 **667 passed /
0 failed**。

**集成轮 3（核心 CAD，Linux 构建）**：实体颜色（ByObject/ByLayer/ByBlock，ACI/RGB）与
线宽（mm）进入渲染（`docs/entity-style.md`，线宽显式不绘制）；MTEXT 格式 run 解析与
整形（`docs/mtext.md`，堆叠分数/颜色/装饰为显式 Partial）；导入 4 角纸空间视口/完整变换/
INSERT/OCS/SOLID（闭合 round-2 `docs/layouts.md` §3.1）；网格面 `SubElementId` 拾取 +
选择高亮叠加（`docs/picking-3d.md`）。核心测试 **735 passed / 0 failed**，软件 Vulkan
**50 passed**；fmt/clippy/架构/i18n/fixtures/wasm `--lib` 全通过（仍只构建 Linux 核心，
未构建 Android/Web）。仍开放：ACIS 真实授权样本与锥面/带环球面、LINETYPE 虚线、渐变
HATCH、动态块求值、注释性缩放、打印/出图、异步可取消导入与进度、性能基准与预算实测。

**集成轮 4（核心 CAD，Linux 构建）**：LINETYPE 虚线端到端（`docs/entity-style.md`）；
纸空间布局出图到 PNG + `PLOTSETTINGS` 导入（`docs/plot.md`）；性能预算
（`cpu_bytes`/`queued_tasks`/`upload_bytes_per_frame`）真实计费 + 可复现 benchmark
（`docs/performance.md`）；渐变 HATCH 逐顶点颜色烘焙（`docs/hatch-gradient.md`）；
异步可取消导入（进度 + 过期结果丢弃，F01）（`docs/import-async.md`）。集成测试
**824 passed / 0 failed**（lavapipe），fmt/clippy/架构/i18n/fixtures/wasm `--lib` 全通过。
仍开放：矢量出图（PDF/HPGL/CTB）、复杂/嵌入形状线型、宿主异步导入面板接线、
手机内存/FPS 实测。

**集成轮 5（核心 CAD，Linux 构建）**：动态块可见性状态读取与切换（GEOMETRY 增量，
`docs/dynamic-blocks.md`）；ACIS 锥面（完整圆锥 + 截头圆锥）与环形圆环面离散，修复
截头圆锥母线缝合在环长不等时夹紧末点导致的非流形开边（`docs/kernel-acis.md`）；
注释性缩放：`Scale` 表 + `CANNOSCALE` 导入、TEXT/MTEXT 按活动比例缩放并支持
按比例位置覆盖，非文本/非法比例显式 `Partial`（`docs/annotative-scaling.md`）。集成测试
**870 passed / 0 failed**（lavapipe），fmt/clippy(0)/架构/i18n/fixtures/wasm `--lib` 全通过。
仍开放：真实授权 ACIS 样本、带环球面、非圆椭圆/样条面、矢量出图、复杂线型形状、
宿主比例切换与异步导入面板接线、真机内存/FPS 实测。

仍开放（受环境/外部依赖限制）：**F15 真实 ACIS 离散**（需内核 + SAT/SAB 解析器 + 授权样本）；
**真机**（模拟器 SwiftShader 不等同真机）；**WebGPU / 真实 GPU / 移动与桌面浏览器矩阵**；
Android surface 尺寸/安全区（U07）、SAF、量测/批注拾取与面板状态推送；自托管 GPU/Android
runner；**大型授权 DWG/字体与跨后端黄金图矩阵**（`fixtures/manifest` 现有开源 QCAD
flange 样本，但仅 `Partial`，不构成兼容性或黄金图验收）；MultiLeader；
复杂文字整形；自动保存/崩溃恢复保留策略。详见各功能 `docs/*.md` 的"未完成"。

## 下一轮优先：UI 界面未实现功能（用户指定）

用户明确要求本集成轮结束后优先补齐 UI 未实现功能。下表按源码现状刷新为**真实状态**，
不再保留过期待办。`docs/ui-redesign.md`「仍未闭环」与 `docs/panels.md` /
`docs/responsive-ui.md` 的缺口同步更新。

| # | 项目 | 状态 | 证据 / 位置 |
|---|---|---|---|
| 1 | Ribbon 自定义分组渲染 | **已实现** | `ui.components.ribbon.tabs[].groups[].commands[]` 真正渲染并按 `commandVisibility` 过滤、`display` 模式与内联溢出；未声明自定义 tabs 时内置面板不变。`crates/cad-ui-slint/ui/ribbon.slint:98–156,205` |
| 2 | overlay 开关驱动合成层 | **已实现** | `view.overlays.{axes,grid,selectionHighlight,snapHints}` 端到端门控绘制（axes/grid 真实世界参考几何，捕捉标记按 `SnapKind`；`annotations` 开关随批注子系统移除）。`crates/cad-app/src/render_scene/mod.rs:213–265` |
| 3 | interaction 门控 | **已实现** | `interaction.{pointer,touch,keyboardShortcuts}` 在共享 Slint 适配器逐项门控输入（指针/滚轮/拾取/别名展开）。`crates/cad-ui-slint/src/adapter.rs:160–167`、`tests/interaction_gating.rs` |
| 4 | 精简预设语义 | **已实现** | `minimal` 有显式组件真值表与命令白/黑名单（`canvasOnly ⊆ minimal ⊆ full`）。`crates/cad-app/src/viewer_config.rs:202–311` |
| 5 | 面板细节（布局面板；资源/3D/诊断抽屉；`responsive-ui.md` §6 缺口；软键盘/安全区） | **已完成（本轮，边界显式）** | 诊断抽屉 + 资源/3D 抽屉 + 布局比例（显示级）已接线（`docs/panels.md` §3.4）；纸空间 mesh/图像逐三角裁剪闭环（`docs/layouts.md` §4）；`safe_insets` 已折入 `CanvasMetrics`（`docs/input.md` §5）。显式缺失：3D fit、视口比例命令面、图像实体建模、Android OS inset 回调转发 |
| 6 | 原生宿主偏好持久化 | **已实现** | 原生宿主读取 `$XDG_CONFIG_HOME/yacr/config.json` 与 `preferences.json`，支持 `--config`/`--preferences` 覆盖，无 XDG/HOME 时显式不持久化。`apps/app-linux/src/host/config_file.rs:103–175` |
| 7 | 真机/浏览器移动矩阵 | **环境/证据缺口** | 需真机与移动浏览器矩阵实测；属验收环境缺口，不是代码待办。 |

注：用户批注子系统已于 2026-10-05 整体移除（见 `CAD_IMPLEMENTATION_SPEC.md` 顶部变更
记录），`docs/ui-redesign.md` 中涉及批注门控/批注面板的旧描述按“已移除”理解。

## 已定义，但尚需设计审查

- RenderTarget/HostTexture 当前仅为自有数字 token；共享 Device/Queue 的拥有者、
  token 生命周期与安全访问方式须在平台合成 ADR 中固定，不可凭 token 猜测 GPU 对象。
- AnnotationId(u128) 只表达 UUID 位宽；生成/碰撞防护由宿主服务和数据库校验完成。
  JSON UUID 编码、时间格式、extension 原始 JSON 校验尚未实现。
- FingerprintPolicy 的显式策略不意味着导入已经能自动建立锚点；fingerprint 计算需分段/异步。
- CommandPayload 的类型校验、防止 ID 与 payload 不匹配、事务权限与撤销能力仍需接线。
  当前 Application 提前检查模式与文档身份，但 Work 模式执行仍返回 NotImplemented。
- RepresentationProvider 的字体/样式上下文目前只提供最小配置，需要扩充自有只读 views。
- DB 中已有基础实现应保留并复查几何边界/块实例/验证完整性，不代表工业语义已验收。
- 数据库/几何已有基础真实实现与合成测试，代理也已有有界 framing/记录回放代码。
  这些是在恢复过程中保留的新增实现；需审查实际支持范围，不能据此宣称真实 DWG/工业兼容。
- 扩展点包含 Importer/Tool/CommandHandler 等；注册/优先级/冲突策略需统一实现。
- PropertyProvider、字体 shaping、缓存容量、Worker 二进制传输、Journal codec、schema migrations
  都需要完整接线和验收，不允许以现有结构体宣称闭环。

## 接线验收

业务不调用 draw；批注确认一次事务，取消零事务；UI 回填不触发重复命令；
revision 不连续重建；GPU 重建不改数据库；设备失败和退出保护未保存批注；
局部失败保留对象级报告；未知源单位显示图纸单位。Android/Web 真机与浏览器分开记录。
## 2026-10-04 Linux release 后续排查（部分优化，问题未闭环）

- 用户手工 release 仍卡死；补窗口启动后打开和实际 pointer/click 探针，基线 release
  重现 17 秒事件循环停顿。前轮 `--open` 回归不覆盖加载响应。
- 增加不透明虚线 `LineSegments` packed 表示及弧长索引，所有 consuming boundary 识别
  独立端点对；默认 provider 与透明 run 顺序不变。CPU 场景约 17.6→14.0 秒。
- 拾取改为借用几何，约 72 万 items 的收集 1.85→0.20 秒；debug 点击仍约 2.05 秒超阈值。
- release `--open` 加 pointer/click 短回归通过：17.68 秒首次帧，1.58 秒点击，max gap 1.60 秒；
  **不是窗口启动后打开/加载响应通过，也不是手工或视觉验收**。
- 尚未实现 native 异步准备、空间 broad phase、GPU 上传背压；详细证据/限制见
  `docs/linux-dwg-ui-freeze.md` 最新小节。修改未提交；不覆盖前轮与并发工作。
