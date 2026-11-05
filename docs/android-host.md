# Android 宿主（`apps/app-android`）

> **2026-11-01（P5）起 Android 是原生 `native-below` 载体**：共享 React chrome 跑在 Tauri WebView 里，
> **原生 Rust `CadSession` 持有 `cad-db`**，wgpu（Vulkan）呈现到 WebView **之下**的 `SurfaceView`。
> wasm 会话在 Android 上**不再实例化**。此前 2026-10-28 的 `dom-canvas` 形态已退役（见 §10）。
>
> 本文记录契约、构建方式、**已运行的证据**与**未验证边界**。证据只到**编译级 / 开发机契约测试**；
> **设备运行、Vulkan adapter、像素、旋转/前后台、JNI 运行时链接、SAF 选择器/读写、外部「打开方式」
> 全部 NOT VERIFIED**。请勿把它读成「Android 已验收」。

## 1. 载体契约（不许改变的部分）

- **唯一 CAD 权威 = 原生 `CadSession`**：`cad_ui_core::session::CadSession` 持有 `cad-db`、命令/事务与
  显示表示；渲染由 `cad-render-wgpu` 在原生 `SurfaceView` 上完成。宿主上报
  `surfaceMode: "native-below"`、`documentOwner: "native"`（ADR 0027）。
- **wasm 会话不运行**：`apps/desktop-ui` 的 `dom-canvas` transport effect 以
  `surfaceMode === 'dom-canvas'` 为门，Android 不会加载 `/canvas/*`。native 壳**不**实例化第二份文档
  状态、**不**把 native 显示表示当作 wasm 数据源（铁律 8）。
- **React chrome 是同一棵树**：`apps/desktop-ui` 用 `YACR_RUNTIME=android` 构建，`native-below` 上渲染
  `.native-input`（单一输入漏斗）与**没有** DOM `<canvas>`。
- **命令集共享**：`cad_desktop_shell::commands::register_commands()`（共享 `generate_handler!` 注册表）
  加上本载体保留的两个 shell 报表 `canvas_session_status` / `canvas_runtime_failed`。原生命令语义 =
  `cad_ui_core::session::wire`，与桌面同一套解析/准入/错误码；没有 Android 专属命令表。
- **事件是 push-only**：唯一通道是 `cad_event`。每次 flush/帧推送一条携带**完整状态**的
  `status-changed`（含 `hostProfile`），语义事件另行推送（见 §2）。**前端不轮询 `cad_status`**。
- **`hostProfile` 恒在**：`cad_status` 与 `status-changed` 都带
  `hostProfile { surfaceMode, documentOwner, palettesAvailable, standardViews }`。载体由这个对象声明，
  不再靠「哪些键存在」推断。
- `app.windows` **必须**有一条窗口配置：`tauri::app::setup` 从 `app.windows` 创建移动 WebView，
  空数组合法但会**什么都不显示**（这是与桌面 `app.windows: []` + 手建窗口的区别）。
- CSP 为 `script-src 'self' 'wasm-unsafe-eval' blob:`（bundle 仍包含 wasm 产物；`blob:` 用于
  插件管理器导入用户安装/校验的 ES module）与
  `connect-src 'self' ipc: http://ipc.localhost`（Tauri IPC）。

## 2. 事件、状态与命令语义

| 项目 | 契约 |
|---|---|
| 状态推送 | 每 flush/帧一次 `status-changed`，携带完整 `CadStatus` + `hostProfile`（+ 可选 `canvasSession`）；`frameCount`/`cursorWorld` 搭这条事件 |
| 语义事件 | `command-admitted`、`command-rejected`、`selection-changed`、`tool-step`、`document-opened`/`document-closed`、`error-raised`、`popup-changed`、`events-dropped` |
| `command-rejected` 归因 | **仅批内相邻**：`Error` 必须直接跟在被准入的 `Command` 之后、且两者同批；跨 flush 的错误只经 `status.error` 到达 |
| 排序 | 同一 flush 里语义事件先、`status-changed` 后（命令结果先于包含它的快照） |
| `cad_status` | 只用于订阅/bootstrap 与丢事件（`events-dropped`）后的恢复；**不是**轮询源 |
| 命令语义 | `cad_ui_core::session::wire`；准入/上限/诊断字符串只在 `cad-ui-core` |
| tick 驱动 | `tauri::RunEvent::MainEventsCleared`，由 `RedrawPump` 唤醒；所有 GPU/`CadSession` 调用都在同一条 Tauri loop 线程（**源码阅读结论，未在设备上运行**） |

## 3. v1 的显式上限（必须照写，不许假装）

| 能力 | 状态 | 实际表现 |
|---|---|---|
| 打开 | **支持（SAF，设备 NOT RUN）** | `OPEN` → Kotlin `ACTION_OPEN_DOCUMENT`（`CATEGORY_OPENABLE`、`*/*`、读 + persistable 授权）；worker 线程读完整字节，经 `documentSnapshot` 以 `FileEvent::Opened` 交给共享 `PlatformFiles` 漏斗。`canOpen = true`。宽 MIME 是刻意的：`.dwg` 供应方常报 `application/octet-stream`，窄过滤器会把要打开的文件变灰 |
| 新建 | **不支持** | `canNew = false`；没有 New 漏斗，也不建第二份文档权威 |
| 保存 | **支持（DXF 导出，设备 NOT RUN）** | `SAVE` → Kotlin `ACTION_CREATE_DOCUMENT`（`application/octet-stream`、`EXTRA_TITLE`）；共享保存路径把 **DXF** 字节经 `write_atomic` → `ContentResolver.openOutputStream(uri, "wt")` 写入（SAF 实现是截断写，没有临时文件 + rename）。`canSave = true`。这是**导出一份副本**，不是原地覆盖：打开来源的同一 URI 被 `refuses_source_overwrite` 拒绝。**DWG 保存仍是共享路径的显式延后拒绝**（`save.dwg_deferred`） |
| 出图 | **不支持** | `canPlot = false`；没有 SVG/PDF 导出 |
| 文件身份 | **SAF URI，不伪造** | 身份 = `content://` opaque 句柄 + `OpenableColumns.DISPLAY_NAME`（缺列时回退 URI 末段）；没有 `--open` 启动参数。`content://` 目标**没有**文件系统目录，sidecar xref/栅格不可用：`xref_options_for` 对含 `://` 的句柄返回 `None`，引用按不可用报告，不派生假根目录 |
| 外部「打开方式」 | **已接（设备 NOT RUN）** | `bundle.fileAssociations`（`dwg`；mime 含 `application/octet-stream`）由 tauri-build 在构建期注入 manifest 的 VIEW intent-filter；`MainActivity` 冷启动（`onCreate`）与热启动（`onNewIntent` + `setIntent`，`singleTask`）都投递到同一个读槽 |
| 导入线程 | **同步（tick 线程）** | `importWorker = false`：DWG 在 tick 线程同步导入；极大图纸会阻塞渲染，属**已记录限制**，不是挂起或假成功 |
| 初始文档 | **共享核心内置 demo 图纸** | 与桌面宿主不带 `--open` 启动完全一致；外部打开到达后由共享打开漏斗替换 |
| 弹窗宿主 | **已接（in-DOM）** | `AndroidPopupHost`：弹窗是 React 树内的覆盖层，无原生窗口；`open`/`close` 经 `cad_desktop_shell::popup` 的状态机与模态门，主线程跳转有界（5 s）。Options / About / text-edit 可用；`textEdit` 载荷取当前 `CadSession` 选区（无选区 → 共享 `popup.no_text_selection` 拒绝）。设备运行 **NOT VERIFIED** |
| 持久配置/偏好 | **持久化到应用私有 `filesDir`**（`/data/user/0/io.github.zenglanmu.yacr/files`） | `AndroidConfigStore`：`config.json` / `preference.json`，读 `Absent`/`Text`/`Unreadable` 与桌面一致，写为「建目录 + 临时文件 + `sync_all` + rename」。路径来自 `nativeAssetsReady`（`filesDir` = 公告 `fontsDir` 的父目录）；握手前保持非持久（读 `Absent`、写 `Unsupported`），不发明路径。设备侧落盘 **NOT VERIFIED** |
| 插件管理器（2026-11-05） | **契约支持（`native-below`），设备 NOT VERIFIED** | Options → Plugins 的五个 `cad_plugin_*` 命令由共享 `cad-desktop-shell` 注册。策略/插件位于应用私有 `filesDir`：`filesDir/plugins.json` 与 `filesDir/plugins/<id>/{plugin.js,manifest.json}`；`AndroidPluginStore` 每次调用惰性解析 `filesDir`，`nativeAssetsReady` 握手前返回 `unsupported-runtime: plugin-store-unavailable`，不缓存启动时的空路径。安装/启用/禁用/卸载立即持久化、**下次启动才生效**（无热重载/热卸载）；卸载真删目录。外部解码器经 Blob URL 导入，故 CSP 含 `blob:`。设备/模拟器运行、目录落盘、插件执行与像素 **NOT VERIFIED** |
| 渲染后端 | **Vulkan 硬件 only** | 软件 Vulkan → `yacr: unsupported-gpu: software-vulkan`；**无** GLES/CPU 回退 |
| FIT / 视口 | **整窗取景** | `SurfaceView` 覆盖整个窗口；`fit_camera` 仍按全窗口取景，分区子矩形是 follow-up |
| 指针 | **分区原点平移** | tick 内把 DOM 分区局部坐标按 origin 换算成 presented-canvas 坐标；不是第二条输入通道。鼠标/触摸/笔按每事件 `device` 在共享 `cad-host` 仲裁（touch：单指平移、tap 选择、双指 pinch 缩放；mouse/pen：中键平移、2D 主键框选、滚轮缩放） |

UI 侧对能力布尔为 `false` 的项（New / Plot，以及共享路径判定为延后的 DWG 保存）必须**显式禁用/拒绝**
（带原因），不得空成功、不得伪造文件选择结果或路径。`canOpen`/`canSave` 由本载体的 tick 按
`policy.rs` 的常量上报，不在 React 侧按平台猜测。

## 4. 原生表面（`SurfaceView` + JNI）

Tauri 的窗口 `transparent` 在 Android 上**不支持**，因此透明按 view 处理：`MainActivity` 在 WebView
附着后把 `SurfaceView` 插到其父容器的 **index 0**（WebView **之下**），并清空 WebView 背景；平台正常的
`SurfaceView` 挖洞把原生表面透过透明页面合成出来。`setZOrderOnTop(false)` 是关键（on-top 会盖住
React chrome），`setZOrderMediaOverlay(false)` 一并固定。

**为什么必须在 attach 之后插入**：Wry 的 `onWebViewCreate(webView)` 在
`activity.setContentView(webview)` **之前**运行，那时 WebView 还没有父容器。因此插入动作挂在
`addOnAttachStateChangeListener` 上，而不是 `onWebViewCreate` 里。

冻结的 4 个 JNI 符号（包 / Kotlin namespace `io.github.zenglanmu.yacr`，实现于同一 `libapp_android.so`）：

| 符号 | 方向 | 职责 |
|---|---|---|
| `nativeSurfaceCreated(surface, width, height) -> Long` | Kotlin → Rust | 发布新的原生 `Surface` 与 buffer 尺寸，返回不透明 handle |
| `nativeSurfaceResized(handle, width, height)` | Kotlin → Rust | 记录新的 buffer 尺寸 |
| `nativeSurfaceDestroyed(handle)` | Kotlin → Rust | 标记表面消失；tick 停止呈现 |
| `nativeAssetsReady(fontsDir, imagesDir)` | Kotlin → Rust | 公告从 APK 解压到 `filesDir` 的字体/资源目录 |

- `Long` 只是不透明原生 handle；Kotlin **不**持有 CAD 状态、不做坐标/命令逻辑。

**反向（Rust → Kotlin）的 5 个实例方法**：`src/saf.rs` 按精确名字/描述符调用
`io.github.zenglanmu.yacr.MainActivity` 上的实例方法（`Java_io_github_zenglanmu_yacr_MainActivity_document*`），这也是
`proguard-rules.pro` 必须固定它们的原因。

| 方法 | 职责 |
|---|---|
| `documentLaunchOpen()` | 启动 `ACTION_OPEN_DOCUMENT`，清空并武装唯一交互槽 |
| `documentLaunchSave(String)` | 启动 `ACTION_CREATE_DOCUMENT`，`defaultName` 作为建议标题 |
| `documentReadUri(String)` | 启动 Kotlin worker 读；槽立即进入 `LOADING` |
| `documentWriteUri(String, byte[])` | 同步截断写入；失败抛 `IOException`，调用方报真实错误 |
| `documentSnapshot()` | 返回 `[Int state, String? uri, String? name, String? error, ByteArray? bytes]` |

`documentSnapshot()` 的状态码与 Kotlin `DocumentPicker.STATE_*` 一一对应：

| 码 | 名称 | 语义 |
|---|---|---|
| `0` | `NONE` | 没有进行中的交互 |
| `1` | `LOADING` | 读在途；**不消费**，下次仍可能返回 `LOADING` |
| `2` | `OK` | 读完成（带 `bytes`）或已选保存目标（`bytes == null`）；**消费** |
| `3` | `CANCELLED` | 用户取消选择器，不是失败；**消费** |
| `4` | `FAILED` | 交互失败，`error` 说明原因；**消费** |

- 终态（`OK`/`CANCELLED`/`FAILED`）**只交付一次**：`documentSnapshot` 在返回前把槽复位为 `NONE`，
  旧结果不能回答新请求。`bytes != null` 是唯一可用的 open/save 判别式（保存目标没有字节）；
  目的（`FilePurpose`）由 Rust 侧武装，Kotlin 只记 `pendingSave`，两侧不会互相串线。
- 线程：启动走 `runOnUiThread`（JNI 可能来自 tick 线程）；读在
  `yacr-document-read` worker 上完成（64 KiB 分块，不截断）；写按调用线程同步执行。槽访问加锁，
  并用单调 `generation` 丢弃过期的选择器/读取结果，不让旧结果覆盖新交互。
- Activity 对象来自冻结回调的隐式 `this`（`capture` 只存一次 `JavaVM` + global ref）；没有
  `FindClass`、没有第二个 Java 辅助类。每次交互一个 bounded local frame，JNI 失败先清 pending
  exception，再以 `Err(String)`/`Failed` 返回——**没有空成功**。稳定 logcat 前缀为 `yacr: saf`。
- 非 Android target 不编译 SAF 桥；`AndroidFiles` 的对应入口显式返回 `policy::file_refusal(...)`，
  开发机据此跑拒绝契约测试，而不是伪造选择器结果。
- 字体与资源从 APK assets 解压到 `filesDir` 后再公告；空字体目录显式拒绝（不伪造字体面）。资源公告
  可能晚于 Tauri `setup`，因此 tick 在会话建立前每轮重读。
- **同一个 `filesDir` 也是持久配置的根**：`config.json` / `preference.json` 写在它下面（`fontsDir` 的
  父目录），所以 JNI 契约不需改动；握手前 store 保持非持久而绝不猜路径。
- GPU 只接受**硬件 Vulkan**。软件 Vulkan（SwiftShader / lavapipe / CPU）显式拒绝并打稳定标记行；
  不静默换成 GLES，也不接受「空白 canvas」为成功。
- Android 没有 shell 退出码可判：**`logcat` 的稳定标记行是第一分类依据**。

## 5. React chrome 与输入

- 同一棵 React 树、同一套命令规则、同一套移动布局（`MOBILE_MAX_WIDTH = 1024`，按宽度切换）。没有
  Android 专属 UI。
- `native-below` 上输入仍走 `.native-input` → `probe_input` IPC → 宿主准入门/队列；`<canvas>` 不
  存在，也不注册监听器。
- **触摸输入走同一 DOM 漏斗**：鼠标/触摸/笔都由 DOM 的 `pointer*`/`wheel` 事件进入 `.native-input`；前端
  逐事件按 `PointerEvent.pointerType` 附可选 `device: "mouse" | "touch" | "pen"`（缺失/未知 ⇒ mouse），
  经 `probe_input` 交给共享 `cad-host` 仲裁。默认：**touch** 单指拖拽 = 平移、tap = 选择/取点、
  双指 `pinch` = 以中点缩放（含中点平移），**touch 从不框选**；mouse/pen 中键拖拽 = 平移、2D 主键拖拽 =
  框选、滚轮 = 缩放。`pinch` 契约（一次 `pointer_cancel` 进入、连续 `{x,y,dx,dy,scale}`、不逐指针转发）
  见 `docs/react-linux-app.md`。
- **画布比例取 WebView 的 `devicePixelRatio`**：tick 用 `crate::input::canvas_scale` 从已接受的布局
  矩形取 `dpr`，而不是 `tao` 的 `window.scale_factor()`；两者不一致会把整个输入空间按错误比例缩放，
  pinch 锚点会落在手指之外。首个带 DOM rect 的 tick 打一行
  `[probe] canvas scale: window.scale_factor=… webview.dpr=… using=…`。开发机纯函数测试覆盖取值/回退；
  **设备上的 pinch 与比例确认 NOT RUN**。
- **Options / 持久化**：弹窗宿主已接（`AndroidPopupHost`，in-DOM 覆盖层），因此 Options / About /
  text-edit 可用；`textEdit` 载荷由 loop 线程上的原生 `CadSession` 选区派生。`interaction.leftDragPans`
  （Options → Drafting「左键拖动平移」）等偏好写入 `filesDir` 的 `preference.json`（`config.json` 为完整
  viewer config），下次启动经同一 store 读回。**设备侧点击/落盘/重启后读回 NOT RUN**（§8）；源码与开发机
  契约测试只证明接线与文件语义。
- 指针/滚轮坐标由 tick 按 DOM 分区原点平移后交给共享输入管线；纯函数在开发机有单测。
- `invoke.ts` 的 `adapterOwnsData = carrier === 'dom-canvas' || !tauriPresent()`：Android 上 Tauri 存在
  且 carrier 为 `native-below`，所有 CAD 命令走 Tauri；wasm 适配器不接管数据。`android` 构建从
  carrier `unknown` 起步，第一份 `cad_status` 到达后定为 `native-below`。

## 6. 目录与构建

```
apps/app-android/
  package.json                 @tauri-apps/cli 2.12.3（devDep）+ android:* 脚本
  src-tauri/
    Cargo.toml                 name "app-android", [lib] app_android, staticlib/cdylib/rlib
    build.rs                   仅在 target_os=android 时 tauri_build::build()
    tauri.conf.json            frontendDist "../dist"; beforeBuildCommand = build-android-ui.sh
    icons/icon.png             仓库既有图标 assets/icons/yacr-1024.png
    src/lib.rs                 模块清单；非 Android target 的显式 NOT IMPLEMENTED 回答
    src/host.rs                Tauri entry + 共享命令 handler + 两个报表 + Shared/telemetry
    src/host/tick.rs           MainEventsCleared tick（输入映射、呈现、事件 flush）
    src/host/pump.rs           RedrawPump（只唤醒 loop，不做渲染工作）
    src/popup.rs               弹窗接缝：PopupOps payload + AndroidPopupHost 主线程跳转
    src/surface.rs             JNI 表面回调 + SurfaceProbePresenter（Vulkan only）
    src/saf.rs                 Android-only SAF JNI 桥：5 个 document* 实例方法；非 Android 目标不编译
    src/policy.rs              纯策略：标记行、能力布尔（open/save=true，new/plot=false）、JNI 符号名（可在开发机测试）
    src/platform.rs            SAF 文件契约（AndroidFiles）、DOM surface、filesDir 持久 config store
    src/plugin_store.rs        插件管理器持久化：惰性解析 filesDir 的 AndroidPluginStore（委托 FsPluginStore）
    src/input.rs               分区局部 → canvas 坐标（纯函数，开发机单测）
    src/refusal.rs             稳定标记行 / step / 历史 exit 6 词汇
    src/report.rs              报表行纯函数（一行、去控制字符、有界）
    src/contract.rs            源码结构契约（开发机运行）
    gen/android/               **提交**的 Gradle 工程
    dist/                      生成的 Android UI bundle（git 忽略）
```

- `gen/android` **提交**：它由 CLI 版本决定；重新生成会随 CLI 漂移，提交后构建可复现。
  `app/src/main/assets/tauri.conf.json`、`jniLibs/*.so` 等生成物由 `gen/android/app/.gitignore` 忽略。
- `gen/android/app/src/main/java/io/github/zenglanmu/yacr/MainActivity.kt` 是**手写且提交**的宿主 Activity。
  重新 `tauri android init` 会覆盖它；`scripts/test-android-host.py` 会判红。它只把五个 `document*`
  实例方法委派给 `DocumentPicker`，自身不持有 CAD 文档。
- `gen/android/app/src/main/java/io/github/zenglanmu/yacr/DocumentPicker.kt` 是**手写且提交**的 SAF 中介：选择器、
  worker 读、单槽快照；只保存一次交互的结果，不保存文档（铁律 8）。
- `gen/android/app/proguard-rules.pro` 的 `-keepclassmembers class io.github.zenglanmu.yacr.MainActivity` 固定
  五个 `document*` 方法的名字与描述符。**这不是预防性规则**：没有它时 release R8 会把
  `documentLaunchOpen` 从 `classes.dex` 移除（Rust 是唯一调用方），运行期必抛 `NoSuchMethodError`；
  该缺陷已在验证中实际捕获。
- `tauri.conf.json` 的 `bundle.fileAssociations` 声明 `.dwg`（含 `image/vnd.dwg`、`application/acad`、
  `application/dwg`、`application/x-dwg`、`application/autocad_dwg`、`application/octet-stream`）。
  `AndroidManifest.xml` 里的 `tauri-file-associations. AUTO-GENERATED` 块由 tauri-build 在构建期注入；
  **改关联请改配置**，不要手写 manifest。
- Gradle `ndkVersion = "29.0.13846066"`（脚本断言）。仓库 `fonts/` 作为 assets `srcDir` 打包
  （`fonts.json` + 字体文件）。
- **NDK 链接下限**：`ndk` crate 的 `nativewindow` feature 会无条件链接 `-lnativewindow`，
  而 NDK 只在 **API ≥ 26** 提供该 stub；本应用 `minSdkVersion = 24`，所以该 feature 必须保持
  **关闭**（`scripts/test-android-host.py` 断言）。实际使用的 `NativeWindow::from_surface`、
  `ptr`/`width`/`height`、`Clone`/`Drop` 在 API 24 由 `libandroid.so` 导出；只有 API 26+ 的
  `set_buffers_*`/`set_frame_rate*`/`try_allocate_buffers` 需要该库。API 24/25 真机运行时
  **NOT VERIFIED**。
- `scripts/build-android-ui.sh` 产出 `apps/app-android/dist`（`YACR_RUNTIME=android`，不覆盖桌面
  `apps/desktop-ui/dist`）。它目前仍会一并产出 wasm 会话产物；native 载体不加载它们（构建 lane 的
  独立清理项，不影响本契约）。

```bash
export ANDROID_HOME=$HOME/android-sdk ANDROID_SDK_ROOT=$HOME/android-sdk
export NDK_HOME=$HOME/android-sdk/ndk/29.0.13846066 JAVA_HOME=$HOME/jdk17
bash scripts/build-android.sh --release --target aarch64     # 真机构建（默认）
bash scripts/build-android.sh --debug   --target x86_64      # 模拟器（debug 已签名）
```

- **本地 release APK 默认未签名**（`app-universal-release-unsigned.apk`）；仓库不含签名密钥。本地安装可用
  `--debug` 或自行 `apksigner sign`。CI `android-release` 用仓库 keystore 对
  `zipalign` 后的 APK 签名并上传 `android-release`（APK + `SIGNING.txt` + `.sha256`）；keystore
  缺失时 job 预检直接失败，不产出 unsigned；
  **签名 ≠ 设备/GPU/像素验收**，见 `docs/code-signing.md`。
- `scripts/build-android.sh` 先跑 `build-android-ui.sh`，再 `tauri android build --apk`。

## 7. 运行与观测

- **`logcat` tag 是 `yacr`**：`adb logcat -s yacr`。宿主启动行、表面生命周期、资源公告、页面几何、
  SAF 选择/读写（失败带 `yacr: saf` 前缀）都打在这个 tag 上。
- **证据运行必须显式记录标记行**。Android 没有 shell 退出状态；`canvas_runtime_failed` 的 `webgl2`
  step 仍保留退出码 6 的**历史**契约，但当前载体不产生该失败。
- **边缘到边缘（edge-to-edge）与手势条内边距**：`MainActivity` 调 `enableEdgeToEdge()`，系统手势条
  画在**页面之上**。这个高度是**宿主事实**（`WindowInsets.Type.navigationBars()`），WebView 里的
  `env(safe-area-inset-bottom)` **不报告它**（那是显示缺口安全区），所以宿主把它以 **CSS 像素**
  （物理 px ÷ `displayMetrics.density`）写成自定义属性 `--yacr-safe-bottom`，由
  `apps/desktop-ui/src/mobile.css` 的 `.mobile-commandbar` 唯一消费。
  **不**隐藏/改色系统栏，**不**在原生侧给内容视图加 padding（页面拥有自己的布局；加 padding 会连
  CAD 分区一起缩小），**不**写设备专用常量。`scripts/test-android-host.py::check_gesture_bar_inset`
  双向固定（Kotlin 发布 + CSS 消费）。
- 写入 `--yacr-safe-bottom` 后必须**读回确认**：Activity 观察不到页面加载完成（`WebViewClient` 属于
  `WryActivity`），第一份文档是 `about:`，写入会随导航消失。实现会重试并在成功/失败时各打一行
  （**没有静默成功**）。
- 每次旋转后宿主打一行 `page layout: {…}`（`layout`、`vw`/`vh`/`dpr`、`safeBottom`、顶栏/菜单/命令框
  等矩形），几何判据优先读这行，不要从截图反推。
- **旧文档里的 `dom-canvas` 观测（`.cad-canvas`、wasm `presented=`、WebView GL adapter）不再适用于本
  载体**；当前分区是 `.native-input`，像素来自原生 `SurfaceView`。

## 8. 证据状态

**已运行（编译级 / 开发机）**：

- `cargo test -p app-android --locked`：**48 passed**（`policy`/`platform`（含 SAF 事件解码与来源覆盖
  守卫）/`input`/`report`/`refusal`/`contract`/源码结构；**不是**设备证据）；
- `cargo check -p app-android --target aarch64-linux-android --locked`（需要 `apps/app-android/dist`
  存在，`tauri::generate_context!` 会内嵌 `frontendDist`）；
- Android lane clippy（host target）；
- Kotlin 编译（提交的 Gradle 工程）；
- `javap` 核对 4 个 Kotlin→Rust JNI 导出符号；
- `python3 scripts/test-android-host.py`：新增断言 `DocumentPicker.kt` 的 SAF 接线（open/save 动作、
  `ContentResolver`、`"wt"`、64 KiB 读、`STATE_* = 0..4`、终态消费）、`MainActivity` 的五个委派方法、
  `onNewIntent` + `setIntent`、`fileAssociations` 的 dwg mime 集，以及 ProGuard 的五行 keep；
- `python3 scripts/check-file-size.py`、`check-architecture.py`、`check-tauri-capabilities.py`、
  `check-i18n.py` 通过；
- **release APK 构建并本地签名**（签名仅供本地验证，不是发布流程；**未安装运行**）；R8 release
  `classes.dex` 已核对五个 `document*` 方法存在（ProGuard keep 规则就是这次修复）。

**NOT VERIFIED（全部未运行）**：

- **设备/模拟器**：未安装、未启动 APK（本轮没有任何 `adb` 运行）。
- **文件选择器 / SAF 读写**：DocumentsUI 打开与创建、provider 授权、persistable grant、worker 读、
  `write_atomic` 写入 `content://`、取消/失败路径、大文件与超时均**未在设备上运行**。
- **外部「打开方式」**：VIEW intent 是否被系统匹配、冷启动 `onCreate` 与热启动 `onNewIntent` 的投递
  均未在设备上验证。
- **Vulkan adapter**：未枚举，未创建 device/queue，未验证硬件/软件归属。
- **像素**：无帧、无截图、无 golden、无色域/色彩空间结论。
- **旋转 / `onPause`/`onResume` / 进程重建 / `SurfaceView` 重建**：未验证。
- **JNI 运行时链接**：`System.loadLibrary` → 符号解析 → 回调、以及 Rust→Kotlin 的五个 `document*`
  调用均未在设备上跑过；静态符号/dex 核对不等于运行时链接。
- **输入**：触摸、手势、软键盘、长按、双击、滚轮（如果有）均未验证；pinch 与
  `devicePixelRatio` 比例只在开发机纯函数测试覆盖，**设备确认 NOT RUN**。
- **弹窗**：接缝已接（`AndroidPopupHost` + `AndroidPopupOps`），源码/开发机契约测试覆盖「注册、
  路由到共享生命周期、无选区返回共享拒绝、主线程跳转有界」；**设备上打开 Options/About/text-edit、
  输入文本、模态门** 未运行。
- **偏好持久化**：`AndroidConfigStore` 的开发机测试覆盖 absent/unreadable/round-trip/原子覆盖/非持久
  回退；**设备上的 `filesDir` 落盘、进程重建后读回** 未运行。
- **多窗口/分屏/折叠屏/三键导航/侧边导航栏**：未验证。
- **架构产物**：aarch64 release APK 已构建但**未安装运行**；armv7/i686/x86_64 未构建、未安装。
- **性能**：同步导入（`importWorker = false`）在超大 DWG 上的渲染停顿未测量。
- **release 签名/分发**：本地签名仅供验证；CI 必须使用自持 keystore 签名，缺失时 job
  直接失败；签名产物以 CI artifact 为准，设备安装/分发均 **NOT RUN**。签名不是设备/GPU/
  像素验收，见 `docs/code-signing.md`。

## 9. 门禁

- `python3 scripts/test-android-host.py`：静态契约。除既有的 workspace/manifest/config/UI 产物/
  `.gitignore` 检查外，`check_native_carrier` 断言：4 个冻结 JNI 签名、`SurfaceView` 在 WebView 父容器
  index 0、`setZOrderOnTop(false)`/`setZOrderMediaOverlay(false)`、holder 回调、
  `nativeAssetsReady(...)`、`ndkVersion = "29.0.13846066"`、`fonts/` 作为 assets `srcDir`、透明主题、
  `configChanges` 保留 `orientation|screenSize|smallestScreenSize|uiMode`；
  `check_document_picker_wiring` 另外断言 SAF 打开/另存、五个 `document*` 实例方法、外部
  `onNewIntent`、`.dwg` 关联 mime 集与 ProGuard keep 行。**无变异即失败**：把表面改成 on-top、去掉一个
  JNI 符号、换掉 NDK 版本、改回 `ACTION_GET_CONTENT` 或删掉 keep 规则都会判红。**注意：这些是静态
  接线，不是设备/像素/SAF 运行证据。**
- `cargo test -p app-android --locked`：纯函数与源码结构契约（host target 上运行）。
- `cargo check -p app-android --target aarch64-linux-android --locked`：真实编译门禁。
- `bash scripts/build-android.sh`：真实 APK 构建，失败即非零。
- CI：`android-check` 是 capability-gated（`vars.ANDROID_CI_ENABLED`）的 **aarch64 编译门禁**
  （不跑 Gradle、不建 APK、不跑设备）。`android-release`（release；必须使用自持 keystore 签名，
  缺失时 job 失败）自 2026-11-01 起重新启用，但只在
  `v*` tag / 手工 dispatch 运行，且只构建并上传 APK artifact；发布 job 本身不发布；签名缺失时
  job 失败，不产出 unsigned；不安装运行。**签名 ≠ 设备/GPU/像素验收**，见 `docs/code-signing.md`。
  **CI 没有设备/模拟器 job。** 见 `docs/ci.md`。

## 10. 历史：已退役的 `dom-canvas` 载体（2026-10-28）

在 P5 之前，Android 是 `surfaceMode: "dom-canvas"`：wasm 会话持有 `cad-db` 并用 WebView 的 WebGL2
画进 `<canvas>`，native 侧只注册两个 shell 报表。那一轮的模拟器（API 35、x86_64、
`-gpu swiftshader_indirect`）记录（出图、手势条内边距、DocumentsUI 选择器、CDP 读数）作为**历史**
保留在 `docs/validation-android-present-2026-10-28.md` 与本文件的 git 历史里。

**不得复用**：那些像素/命令/选择器结论属于**已退役载体**，不能证明 native `SurfaceView`、Vulkan
adapter、JNI 链接、旋转/生命周期或 native 命令语义。当前 Android 的最高诚实表述是
**「native `native-below` 载体已实现并接线，文件打开/DXF 另存与外部「打开方式」已接，编译级契约通过；
设备运行、SAF 实际读写、外部 intent 投递与视觉证据均为 NOT RUN」**。
