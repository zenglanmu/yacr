# 宿主代码拆分

本文件记录宿主层（桌面 / Web / Android）的代码拆分。P1 轮（2026-10-11）把 `apps/app-web`
迁移到共享 `cad_host::HostCore`；P2 轮（2026-10-11）把 `apps/app-android` 迁移到同一核心。
两轮都**不改变**领域模型、crate 依赖方向、命令协议或平台导出名称。核心 CAD 算法的大文件仍可按
各自领域继续拆分，不把这两轮描述为全仓库完成。

## Web Rust 宿主

`apps/app-web/src/browser.rs` 是组合根：选择后端、用 Web 平台实现组装
`cad_host::HostCore`、安装 Slint 适配器与渲染桥，持有唯一运行时。命令路由、状态推送
（`push`/`sync_view`）、打开/出图/另存管线、画布拾取映射与绘制预览 sink 都在 `cad-host`；
`browser/` 只保留**浏览器特有**的适配与平台实现：

| 文件 | 职责 |
|---|---|
| `platform.rs` | `cad-platform` 契约的浏览器实现：`WebFiles`（`begin_open` 触发隐藏 `<input type=file>`；`open_document_bytes` 入队 `FileEvent::Opened`；`begin_save` 同步合成下载目标；`write_atomic` 本阶段显式 `Unsupported`）、`WebConfigStore`（localStorage）、`WebSurface`（CSS 表面 + 安全区） |
| `documents.rs` | 排空 Web 文件事件队列并经同步导入安装新文档（`install_opened` 唯一安装点） |
| `async_open.rs` | 同步回退心跳（`worker_available()==false`，不 spawn 线程）、真实终态快照与稳定 JSON 契约 |
| `fonts.rs` | Fetch 字体、注册、过期结果拒绝与诊断（复用 `WebFontLoader`） |
| `input.rs` | `WebSink`（拦截 `OpenDrawing`/`SwitchBackend`，其余委托 `HostCore::execute`）、`WebViewInput`（浏览器指针/轻触语义）与纯分类辅助 |
| `draw.rs` | `WebDrawSink`：把 Trim/Extend 的世界点解析为 `SelectionRef`（桌面核心显式拒绝）；预览 sink 复用共享核心 |
| `pick.rs` | 纯画布逻辑点 → 世界点映射与选择拾取辅助（拾取映射器为共享核心） |
| `state_push.rs` | 共享 `push` 之上的浏览器叠加：异步打开进度面板 + 资源抽屉字体摘要 + 诊断模型编码 |
| `persistence.rs` | 触发隐藏文件选择器 |
| `shell.rs` | 触控适配（几何/拾取/导航），导航命令经共享漏斗 |
| `config.rs` | wasm 配置导出（`viewer_*`），偏好读写经 localStorage |
| `messages.rs` | 目录镜像（面板空/多值文案） |

组合根通过 re-export 保留调用路径；子模块只在宿主内部共享运行时，不新增数据库副本。
文件打开仍为「平台事件 → 同步导入 → 安装」，保存/出图能力位本阶段保持 `false`（P3 接下载）。

## Web JavaScript

`apps/app-web/web/main.js` 只负责启动与模块组装，`host/` 包含：

- `i18n.js`：共享 catalog、DOM/状态本地化及语言持久化。
- `files.js`：File API 选择器、未保存提示、下载与导出确认。
- `renderer.js`：可见性感知轮询、退避及首次就绪消息。
- `runtime.js`：WebGL 单采样兼容及 winit 控制流异常识别。

文件/语言模块通过显式参数接收 wasm 与状态接口，不依赖入口的可变全局变量。
`window.yacr` 的既有诊断接口保留；`build-web.sh` 复制整个 `host/`，CI 与部署脚本
检查所有模块存在，防止只发布入口导致 HTTP 404。

## Android 宿主

`apps/app-android`（P2 轮，2026-10-11）与 Web 一样退化为薄封装：`src/lib.rs` 是组合根，用
`src/platform.rs` 的 Android 平台实现构造 `cad_host::HostCore`，安装 Slint 适配器与渲染桥，
持有唯一运行时。命令路由、状态推送（`push`/`sync_view`）、打开/出图/另存管线、画布拾取映射
与绘制预览 sink 都在 `cad-host`；`src/` 只保留 **Android 特有**的适配与平台实现：

| 文件 | 职责 |
|---|---|
| `platform.rs` | `cad-platform` 契约的 Android 实现：`AndroidFiles`（Android 目标上 `begin_open`/`begin_save` 经 `saf` 启动 SAF 选择器，`poll` 读取结果并用 `ContentResolver` 读写文档 URI；宿主目标回退为样本路径探测，`begin_save`/`write_atomic` 显式 `Unsupported`；`refuses_source_overwrite` 比较 opaque 句柄=SAF content URI）、`AndroidConfigStore`（无持久化存储：加载恒 `Absent`，写入显式 `Unsupported`）、`AndroidSurface`（Activity 经表面入口转发的尺寸/DPI/安全区）、`AndroidResources`（`HostResources`：字体经 APK assets 装载，图像不建模） |
| `saf.rs` | Android-only 的 SAF JNI 桥：经 `ndk_context` 取得 `JavaVM`/Activity，用 Activity 的类加载器解析 `dev.yacr.app.YacrNativeActivity`（`FindClass` 在无 Java 帧的原生线程上取不到应用类），调用其静态 `begin`/`launch`/`readUri`/`writeUri` 并轮询静态结果槽；错误以未翻译的原始串返回，由 `platform.rs` 用目录键本地化 |
| `view.rs` | `AndroidViewInput`：保留共享 `InputPolicy` 轻触/拖拽状态机（一指点按选中、拖动平移），命令经共享 `HostCore::execute` 漏斗 |
| `draw.rs` | `AndroidDrawSink`：把 Trim/Extend 的世界点解析为 `SelectionRef`（桌面核心显式拒绝），空选择显式报错；预览 sink 复用共享核心 |
| `state_push.rs` | 共享 `push` 之上的 Android 叠加：异步打开进度面板 + 资源抽屉字体摘要（`images_modeled: false`）+ 目录文案 |
| `poll.rs` | Slint 定时器驱动 `core.tick()`（`import_worker=true` 的真实后台导入；打开与异步 SAF 另存选择器在途时保持轮询）与公开的 `set_surface_size`/`set_surface_insets` 入口 |
| `android_fonts.rs` | APK `assets/fonts/` 的 `FontLoader` 实现（`AndroidResources` 复用） |

能力位为 `open=true`、`new=true`、`save=true`、`plot=false`（P4 起 SAF 打开/另存 + 空白新建）。
Android 上打开/另存由 Java 垫片（`android/dev/yacr/app/YacrNativeActivity.java`）中介 SAF
`ACTION_OPEN_DOCUMENT`/`ACTION_CREATE_DOCUMENT`：该垫片是 `android.app.NativeActivity` 子类，
同时是 manifest 声明的启动 Activity，因此无需改写编译后的二进制 manifest。`scripts/build-android.sh`
在 `cargo apk build` 后 `javac`→`d8`→注入 `classes.dex`→`zipalign`→重签。
`android_main`/`slint::android::init`/Slint 定时器/两个表面入口不变。

`HostCore` 新增 `HostResources` 策略（`HostPlatform.resources`），使打开漏斗在不导入平台类型
的前提下装载平台资源：桌面为 `DesktopResources`（原 `cad-host` 字体/图像逻辑原样迁移），Web 为
`UnsupportedResources`（不被调用），Android 为 `AndroidResources`。`metrics` 现在允许 handle 尚未
安装（无窗口/测试核心仍应用表面映射，仅跳过窗口布局重分类）。

## 桌面宿主与宿主编排层（cad-host）

平台无关的宿主编排抽取为 `crates/cad-host`，一个 `HostCore`（`Clone` 句柄）实现
`cad-ui-slint` 的 sink trait（`UiCommandSink`/`LayoutSwitchSink`/`ViewInput`/
`CanvasPickMapper`/`DrawCommandSink`/`DrawPreviewSink`），并按职责拆分：

| 文件 | 职责 |
|---|---|
| `lib.rs` | `HostCore`/`HostOptions`/`HostPlatform`、构造与访问器、`execute` 命令路由、`metrics`、配置装载、`tick` |
| `open.rs` | 打开管线：平台事件 → `begin_async_open`（或同步）→ 轮询 → 安装（fit/取消绘制捕获/字体/图像/推送） |
| `save.rs` | 出图（SVG/PDF）与有损 DXF 另存：扩展名策略、源文件覆盖保护、分级状态 |
| `state.rs` | `push`/`sync_view` 状态漏斗 |
| `input.rs` | 鼠标导航、画布拾取映射、绘制命令/预览 sink |
| `fonts.rs` | 原生字体/图像装载与出图字体引擎（wasm 为显式 `Unsupported`） |
| `policy.rs` | 纯策略：状态归属、扩展名分级、默认文件名（可无 Slint 单测） |

平台 I/O 通过 `cad-platform` 契约表达（`PlatformFiles` 事件式、无 futures/`Send`；
`ConfigStore`；`SurfaceSource`；`HostCapabilities`）。`apps/app-linux` 退化为薄封装：
`src/host.rs` 保留 `LinuxApp`/`LinuxOptions` 公共 API 与事件循环 + 50ms 定时器（调用
`core.tick()`），`src/host/platform.rs` 提供桌面实现（ashpd/rfd 选择器 + 读取 + 原子写 +
dev/inode 源保护；XDG/APPDATA/Application-Support 配置；窗口表面）。`app-windows`/
`app-macos` 仍只调用 `app-linux::entry::run`，未改动。

`apps/app-web` 同样退化为薄封装：`browser/platform.rs` 提供 `WebFiles`/`WebConfigStore`/
`WebSurface`，组合根用它们构造 `HostCore`（能力位 `open=true`、`new/save/plot=false`、
`import_worker=false`），命令路由 / 状态推送 / 拾取映射 / 绘制预览由共享核心承担。Web 无
Rust 定时器：JS 心跳经 `async_open_poll_json` 驱动打开进度；核心的 `tick()` 不用于 Web。

## Slint 渲染桥

`crates/cad-ui-slint/src/bridge.rs` 仅装配 Slint 回调；`bridge/view.rs` 接收完整视图
快照并合并准备请求，`bridge/runtime.rs` 管理 GPU 生命周期、成功发布和按需绘制，
`bridge/presenter.rs` 管理纹理导入，`bridge/camera.rs` 负责相机适应与参数映射。
CPU 场景构建与版本控制位于 `cad-app/src/render_scene/`，不依赖 Slint 或 wgpu；
原场景辅助函数仍由 `bridge` re-export。分散相机/空间 setter 收敛为
`apply_view_snapshot` / `sync_session`。契约与剩余限制见 `bridge-runtime.md`。

## 回归入口

```bash
node --test scripts/test-web-host.mjs scripts/test-web-touch.mjs scripts/test-host-panel-parity.mjs scripts/test-web-responsive.mjs
DIST=/tmp/opencode/yacr-web-p1 WITH_FONTS=0 scripts/build-web.sh
node scripts/check-web-ui.mjs http://127.0.0.1:8090/ /tmp/opencode/yacr-refactor-web.png
```

模块契约测试使用 DOM/wasm stub，只验证模块边界，不算真实浏览器或 GPU 验收。
Playwright 使用真实 wasm、WebGL2/SwiftShader：检查模块加载、非空画面、缩放变化、
双语切换/重载持久化、真实批注下载与 sidecar 回导，以及轮询不覆盖操作状态。
运行证据与明确未验证项见 `validation-web.md`。
