# Android 宿主（`apps/app-android`）

> 2026-10-28 起 Android 不再是 NOT IMPLEMENTED 占位：它是一个 **Tauri 2 WebView/资源载体**，
> 跑共享的 React chrome + **wasm `dom-canvas` CAD 会话**。本文记录契约、构建方式、**本轮实测证据**
> 与**未验证边界**。请勿把它读成「Android 已验收」。

## 1. 契约（不许改变的部分）

- **唯一 CAD 权威 = wasm 会话**（`AGENTS.md` 铁律 8、ADR 0024）。原生侧**不**实例化第二份文档状态、
  **不**持有显示表示、**不**注册 CAD 命令。`cad_status`/`cad_command`/`cad_commands`/
  `cad_command_aliases` 全部由 `apps/desktop-ui/src/host/web-adapter.ts` 走 wasm `shell_*` 导出回答。
- 原生侧**只**注册两个 shell 报表命令（`apps/desktop-ui/src/host/invoke.ts` 的
  `SHELL_ONLY_COMMANDS`，仅在 Tauri 桥存在时路由到 Tauri）：
  - `canvas_session_status(label, batches?, draws?) -> Result<(), String>`：登一行 `logcat`，`Ok(())`。
    **不存状态**——身份由 chrome 从 wasm 会话的 `cad_status.canvasSession` 读回。
  - `canvas_runtime_failed(step, detail) -> Result<(), String>`：先打稳定标记行
    `yacr: unsupported-runtime: <step>`，再打一行 `step=/detail=`；**当 `step == "webgl2"` 时**
    `std::process::exit(6)`。其余 step **只报告、不杀进程**（WebView 还活着，chrome 能显示自己的错误
    态；这与 Linux `dom-canvas` 宿主"每个 step 都退出"的差异是**有意的**，原因见代码内注释）。
    Android **没有 shell 退出通道**——判据是 `logcat` 里的标记行，退出码 6 只是尽力而为
    （进程死亡由平台自己记录）。
- CSP 必须保留 `script-src 'self' 'wasm-unsafe-eval'`（wasm 实例化）与
  `connect-src 'self' ipc: http://ipc.localhost`（Tauri IPC）。
- `app.windows` **必须**有一条窗口配置：`tauri::app::setup` 从 `app.windows` 创建移动 WebView，
  空数组合法但会**什么都不显示**（这是与桌面 `app.windows: []` + 手建窗口的区别）。
- **边缘到边缘（edge-to-edge）与手势条内边距**：`MainActivity` 调 `enableEdgeToEdge()`，因此系统手势条
  画在**页面之上**。这个高度是**宿主事实**（`WindowInsets.Type.navigationBars()`），WebView 里的
  `env(safe-area-inset-bottom)` **不报告它**（那是**显示缺口**安全区），所以宿主把它以 **CSS 像素**
  （物理 px ÷ `displayMetrics.density`）写成自定义属性 `--yacr-safe-bottom`，由
  `apps/desktop-ui/src/mobile.css` 的 `.mobile-commandbar` 唯一消费
  （`padding-bottom: calc(8px + max(env(...), var(--yacr-safe-bottom, 0px)))`）。
  **不**隐藏/改色系统栏，**不**在原生侧给内容视图加 padding（页面拥有自己的布局，加 padding 会连 CAD canvas
  一起缩小且清不掉「条内」重叠），**不**写任何设备专用常量。契约由
  `scripts/test-android-host.py::check_gesture_bar_inset` 双向固定（Kotlin 发布 + CSS 消费）。

## 2. 目录与布局

```
apps/app-android/
  package.json                 @tauri-apps/cli 2.12.3（devDep）+ android:* 脚本 + `tauri` 脚本
  src-tauri/
    Cargo.toml                 name "app-android", [lib] app_android, staticlib/cdylib/rlib
    build.rs                   仅在 target_os=android 时 tauri_build::build()
    tauri.conf.json
    icons/icon.png             来自仓库既有图标 assets/icons/yacr-1024.png（不新造美术）
    src/lib.rs                 refusal + report 模块；host 仅在 android target 编译
    src/host.rs                #[tauri::mobile_entry_point] run() + 两个命令
    src/refusal.rs             标记行 / step / 退出码 6（与平台无关，可在 host 上测试）
    src/report.rs              报表行的纯函数（一行、去控制字符、有界）
    gen/android/               **提交**：`tauri android init` 生成的 Gradle 工程
    dist/                      生成的 Android UI bundle（git 忽略）
```

**为什么 `gen/android` 提交**：它由 CLI 版本决定（AGP 9.3.1 / Gradle 9.6.1 / Kotlin 2.2.10），
重新生成会随 CLI 一起漂移；提交后 `scripts/build-android.sh` 与本地实机/模拟器运行都可复现。
其中的 `app/src/main/assets/tauri.conf.json`、`jniLibs/*.so` 等**生成物**由
`gen/android/app/.gitignore` 忽略。

`gen/android/app/src/main/java/dev/yacr/app/MainActivity.kt` 是**手写且提交**的宿主 Activity
（Tauri 模板只生成一次，`generated/` 下的 `.kt` 才是每次构建重写的）。它持有**唯一**的原生职责：
边缘到边缘 + 发布 `--yacr-safe-bottom`（见 §1）。**重新 `tauri android init` 会覆盖它**，
`scripts/test-android-host.py` 的 `check_gesture_bar_inset` 会在覆盖后判红。

**为什么 `tauri` 是 target-scoped 依赖**：`cargo check --workspace --all-targets`（host）与
`cargo check --workspace --lib --target wasm32-unknown-unknown` 都会编译本成员。Android 之外的
target 上这个 crate 是**有文档的空壳**（`lib.rs` 的 `start()` 返回显式错误），不是"假装能跑"。

## 3. UI 产物（`YACR_RUNTIME=android`）

```bash
bash scripts/build-android-ui.sh     # = build-canvas-wasm.sh + npm run build:android
```

`apps/desktop-ui/package.json` 的 `build:android` 写 **`../app-android/dist`**（`--emptyOutDir`），
绝不覆盖桌面宿主的 `apps/desktop-ui/dist`——两者用不同的运行时常量编译（`vite.config.mjs` 注入
`__YACR_RUNTIME__`）。`tauri.conf.json` 的 `beforeBuildCommand` 就调用这个脚本，Tauri CLI 的工作目录
是 `apps/app-android`（实测），脚本本身按 `BASH_SOURCE` 解析仓库根，不依赖 cwd。

## 4. 构建与安装

```bash
export ANDROID_HOME=$HOME/android-sdk ANDROID_SDK_ROOT=$HOME/android-sdk
export NDK_HOME=$HOME/android-sdk/ndk/29.0.13846066 JAVA_HOME=$HOME/jdk17
bash scripts/build-android.sh --release --target aarch64     # 真机构建（默认）
bash scripts/build-android.sh --debug   --target x86_64      # 模拟器（debug 已签名）
```

- `tauri android build` 会自动安装 Gradle wrapper / AGP / 需要的 build-tools（本轮它自动装了
  build-tools 36.0.0）。
- **release APK 默认未签名**（`app-universal-release-unsigned.apk`）。仓库里**没有**签名密钥
  （旧的 `dev-release.jks` 是 cargo-apk 时代的化石，已删除）；本地安装可用 `--debug` 或自行
  `apksigner sign`。
- `compileSdk = 37` 在本机 SDK 上**可以解析**：`platforms/android-37.0` 的
  `source.properties` 写的是 `AndroidVersion.ApiLevel=37.0`，AGP 9.3.1 接受，因此**没有**降级到 36。

## 5. 本轮（2026-10-28）实测证据

环境：本机 AVD `yacr-mobile`（API 35、x86_64、1080×2400 @420dpi ⇒ **412 CSS px** 宽，mobile 布局）、
WebView `com.google.android.webview 124.0.6367.219`、`-gpu swiftshader_indirect`
（**软件渲染，不是 GPU 证据**）。APK：x86_64 universal **release**，`apksigner` 用临时自签密钥签名，
15,296,759 字节。

| 项目 | 结果 |
|---|---|
| `adb install -r` | **成功**（增量安装，包 `dev.yacr.app`） |
| 启动 `dev.yacr.app/.MainActivity` | **成功**（`Status: ok`，冷启动 798 ms / 复测 507 ms） |
| 设备 CSS 视口 | `wm size` 1080×2400、`wm density` 420 ⇒ DPR **2.625**。**页面自报**（`logcat` 的 `page layout:` 行）：竖屏 **`vw=412, vh=915`**（mobile 布局）；横屏 **`vw=915, vh=412`**（mobile 布局） |
| 日志 `I yacr` | `app-android: Tauri carrier starting …` 与 `canvas_session_status label="synthetic-four-lines.dwg" batches=4 draws=1` —— **wasm 会话起来了**，且 `canvas_session_status` 经 Tauri IPC 到达原生侧并被记录 |
| `<canvas>`（DOM 类名 `.cad-canvas`） | 存在，backing store 非零。**本轮修复后**：竖屏 CAD 分区为 y **244..2184 物理**（= 93..832 CSS，高 739 CSS），横屏 `canvas` rect = **`[0,52,915,277.4]`**（`logcat` 的 `page layout:` 行）。上一轮（命令栏还没让开手势条）记录为 CSS 412×765 / backing 1082×2007 —— 高度差正是本轮命令栏变高的那部分 |
| 会话状态（**上一轮** CDP 读取，非本轮） | `adapter = Android Emulator OpenGL ES Translator (Google SwiftShader) (html-canvas)`、`backend=gl`、`presented=4`、`error=null` |
| 移动布局 | 顶部栏显示图纸名、汉堡抽屉、底部命令栏（截图见下） |
| 旋转到横屏（`user_rotation=1`） | **仍是 mobile 布局**（`data-layout=mobile`，断点 1024 之后）。**2026-10-28 本轮已重新在模拟器采集**：`vw=915 ≤ 1024`；`.mobile-topbar` 高 **52 CSS**、`.cad-canvas` **915×277.4 CSS**、`.ribbon` **不存在**、`.mobile-commandbar` 内有 742 CSS 宽命令框；chrome 占 **134.6/412 = 32.7 %**（此前横屏掉到桌面 chrome 时约 62 %，"62% chrome on a phone"）。见 `landscape.png` |
| **CAD 像素** | **通过（有界）**：黑底 + 四条竖向白色合成线可见。竖屏 CAD 分区：众数色 `#000000`（占比 0.9977）、6 种颜色、亮像素 536 个采样点、**4 个亮列组**（x 344/475/606/737）；横屏同样 **4 个亮列组**（x 1004/1135/1266/1397）。修复前后对比见 `docs/validation-android-present-2026-10-28.md`。**软件渲染模拟器结果，不是 GPU/像素验收** |
| **手势条内边距（本轮新增）** | 见下方「手势条重叠：修复与前后实测」 |

### 手势条重叠：修复与前后实测（2026-10-28）

**症状（用户报告 + 本轮复现）**：竖屏时系统手势条画在**命令输入框内部**。用本轮修复前的 APK
（`yacr-android-present-20261028T045524Z/app-universal-release-signed.apk`，同一模拟器）复测：

| 竖屏（1080×2400 物理 px） | 修复前 | 修复后 |
|---|---|---|
| 命令框（`--field` `#2c3540`） | y **2273..2383** | y **2205..2315** |
| 手势条（系统绘制，`#eeeff0`） | y **2364..2373** | y **2364..2373**（系统位置不变） |
| 关系 | 手势条上沿在命令框内 **19 px**（重叠） | 命令框下沿距手势条上沿 **49 px**（= 18.7 CSS px 空档） |

**机制**：`MainActivity` 用 `WindowInsetsCompat.Type.navigationBars().bottom` 读原生内边距
（本机 = **63 物理 px**，÷ density **2.625** = **24 CSS px**），通过 `evaluateJavascript` 写成
`document.documentElement.style.setProperty('--yacr-safe-bottom', '24px')`；`mobile.css` 的
`.mobile-commandbar` 用 `max(env(safe-area-inset-bottom), var(--yacr-safe-bottom))` 让底部 padding
从 8 CSS px 变成 **32 CSS px**（= 84 物理 px），因此命令框整体上移 **68 物理 px**。

**为什么用 CSS 变量而不是给 `android.R.id.content` 加 padding**：Tauri 2.12 的
`WryActivity.onWebViewCreate(webView)` 把 `WebView` 交给 Activity，`evaluateJavascript` 可用
（不需要新依赖，也不需要反射）；而给内容视图加 padding 会**连 CAD canvas 一起缩小**，且页面自己在
`100dvh` 里排布，原生 padding 无法只清掉「条压在框内」这一段。原生侧**不**改系统栏样式。

**为什么需要「写入后确认」**：Activity 观察不到页面加载完成（`WebViewClient` 属于 `WryActivity`，
既不可取回也不可替换，`onPageFinished` 拿不到），而 WebView 的第一份文档是 `about:`——写入会成功
但随导航消失。**实测**：只写一次的中间版本仍然重叠（命令框下沿 2378 vs 手势条上沿 2364，仍重叠
14 px）。最终实现：脚本在 `location.protocol === 'about:'` 时返回 `wait`，否则写属性并**读回**；
宿主最多重试 40 次（每 100 ms，自停），成功时打
`safe-bottom CSS var confirmed on the document: 24px (attempt 2)`，失败时打 `NOT applied` 警告
（**没有静默成功**）。实测确认发生在第 2 次尝试（页面首帧之前）。

**`logcat` 自证几何**：确认成功与每次旋转后各打一行 `page layout: {…}`（`layout`、`vw`/`vh`/`dpr`、
`safeBottom`、`topbar`/`menu`/`field`/`canvas`/`ribbon` 的 `getBoundingClientRect()`），因此本节的数字
不是从截图反推的：

```
10-28 14:05:37.405 I yacr : page layout: {"layout":"mobile","vw":412,"vh":915,"dpr":2.625,"safeBottom":"24px","topbar":[0,0,412.2,93.8],"menu":[8,49,44,44],"field":[28,839,280.2,44],"canvas":null,"ribbon":null}
10-28 14:05:46.777 I yacr : page layout: {"layout":"mobile","vw":915,"vh":412,"dpr":2.625,"safeBottom":"24px","topbar":[0,0,915,52],"menu":[49,3.6,44,44],"field":[69,336.2,742,44],"canvas":[0,52,915,277.4],"ribbon":null}
```

（竖屏那行 `canvas:null` 是**如实记录**：该行在会话挂上 canvas 之前打印；横屏那行是旋转后 600 ms 的
快照，`canvas` 已存在。）

> 顺带记录两个**设备事实**（不是设计）：①竖屏下 WebView 里 `env(safe-area-inset-top)` 报的是
> **41.8 CSS px**（显示缺口/状态栏区，`viewport-fit=cover`），所以 `.mobile-topbar` 实际高
> **93.8 CSS** 而不是 52；横屏没有该缺口，顶栏是 52 CSS。②系统栏（状态栏）在本宿主上是**隐藏**的，
> 截图顶部全是页面自己的顶栏。自动化点击要按 `logcat` 里的 `menu` 矩形换算（竖屏 CSS
> `[8,49,44,44]` ⇒ 物理 (21..137, 129..244)），按「顶栏 52 CSS」猜坐标会点空。

证据文件（本轮，`/tmp/opencode` 下，**不上传**）：

| 文件 | 内容 |
|---|---|
| `/tmp/opencode/yacr-android-inset-20261028T054145Z/portrait.png` | **修复后竖屏**（mobile）：顶栏 + 命令栏，CAD 黑底 + 四条竖线；命令框在手势条上方 49 px |
| `/tmp/opencode/yacr-android-inset-20261028T054145Z/landscape.png` | **修复后横屏**：`data-layout=mobile`，顶栏 52 CSS + 命令栏，**无 ribbon / 无状态栏**，CAD 915×277.4 CSS 出图 |
| `/tmp/opencode/yacr-android-inset-20261028T054145Z/landscape-drawer.png` | 横屏抽屉打开（点顶栏菜单按钮）：`.mobile-drawer` 面板 y 137..733 物理（= 52..279 CSS，高 227 CSS，命中 `--mobile-drawer-h` 的短视口上限），下方为遮罩压暗的图纸与命令栏 |
| `/tmp/opencode/yacr-android-inset-20261028T054145Z/before-portrait.png` | **修复前**竖屏（同一模拟器、上一轮 APK）：手势条压在命令框内，供上表前后对比 |
| `/tmp/opencode/yacr-android-inset-20261028T054145Z/logcat-yacr.txt` | tag `yacr` 的 `logcat`：两条会话行 + 内边距两行 + 两行 `page layout:` |
| `/tmp/opencode/yacr-android-inset-20261028T054145Z/measure-portrait.txt` / `measure-landscape.txt` / `measure-landscape-drawer.txt` | 像素测量输出（手势条 bbox、命令框 bbox、空档、CAD 分区色统计、四条线的亮列组） |
| `/tmp/opencode/yacr-android-inset-20261028T054145Z/app-universal-release-signed.apk` | 被测 APK（x86_64 universal release，**临时自签密钥**，密钥只在 `/tmp/opencode`，未提交） |
| `/tmp/opencode/yacr-android-20261028T042324Z/*` | 上一轮（透明/全黑像素定位）证据，保留 |

> **CDP 读数的来源与边界**：上表的 canvas 上下文/会话状态是**上一轮**（同一天更早）经 **WebView DevTools
> （CDP）** 读的。release APK 默认**关闭** WebView 调试，那一轮临时在 `MainActivity.kt` 里加了
> `WebView.setWebContentsDebuggingEnabled(true)`，取数后**已还原**（最终 APK 由还原后的源码重建）。
> **本轮（手势条内边距）没有用 CDP，也没有开 WebView 调试**：几何数字来自宿主自己注入页面的
> `page layout:` 日志 + 截图像素测量。

- **日志摘录**（`adb logcat -d`，tag `yacr`）：

  ```
  10-28 12:24:08.560  5332  5349 I yacr    : app_android::host: app-android: Tauri carrier starting (wasm dom-canvas CAD session owns the document)
  10-28 12:24:10.844  5332  5424 I yacr    : app_android::host: canvas_session_status label="synthetic-four-lines.dwg" batches=4 draws=1
  ```

  第二行是**关键证据**：wasm 会话起来了，并且它发出的 `canvas_session_status` 经 **Tauri IPC** 到达
  原生侧（`android_logger` → `logcat`）。

- **像素失败已定位并修复（不是载体/打包问题）**

同一个 `YACR_RUNTIME=android` bundle 用**桌面 Chromium 141（SwiftShader）**打开时**正常出图**
（黑色画布 + 可见线），所以 bundle、CSP、移动布局与 wasm 会话都没问题。在 Android WebView 内：

- 手动 `gl.clear(red)` 到同一个 canvas **能**显示 ⇒ canvas 被合成、尺寸正确、上下文可用；
- 会话自己的 `present` 提交了完整的一轮 GL 调用（`viewport 1082×2007`、`bindFramebuffer(DRAW,null)`、
  `drawBuffers([BACK])`、`drawArrays(TRIANGLES,0,3)`、program 已链接、`getError()=0`），
  但**一个片元都没有写出去**：先把 canvas clear 成绿色再 present，绿色**一字未改**。

设备内最小对照定位到**顶点阶段的动态数组下标**：`P[gl_VertexID]` 在该实现上恒为 0，
三角形退化 ⇒ present 画不出任何像素且不报错。这正是 wgpu-hal sRGB present 全屏三角形的写法，
也命中本仓库 `blit.rs` 的全屏 blit。修复（线性 present 目标 = 纯 `blitFramebuffer` 拷贝 +
我们自己的 blit 做 sRGB 编码 + 顶点阶段改为算术取角）**已在本轮完成并验证出图**，
完整记录见 `docs/validation-android-present-2026-10-28.md`。

### 命令接线缺陷修复：标准视图与 `OPEN` 选择器（2026-10-28 第二轮）

**症状（设备实测，上一轮）**：View 面板的七个标准视图在浏览器/Android 载体上**显示为可用**，点按却抛
`unsupported-runtime: cad_standard_view`；横幅**永不清除**（后续成功的 `FIT` 也清不掉，只能重启 App）。
`OPEN` 的 `<input type="file" accept=".dwg,.dxf">` 在 Android/Chromium 上把 `.dwg` 解析成
`image/vnd.dwg`，意图被路由到**媒体 Photo Picker**（"No photos or videos"），`/sdcard/Download`
里的 DWG 根本选不到。

**修复 1 — 能力显式化**：`cad_status` 增加**可选**字段 `standardViews?: readonly string[]`。
**缺省** = 旧行为（老宿主、桌面 `native-below`）；**存在** = 该载体的权威列表，键不在列表里的视图在
发出 IPC **之前**就被拒绝。wasm 会话报 `[]`（它是 2D presenter，没有 3D 相机、没有
`cad_standard_view` 映射）。原因文案走新目录键 `view.standard_unavailable`（无 `{command}`
占位，整面板共用一句，抽屉按面板说明只打一次）。

**设备实测（本轮，同一 AVD/APK 重建后）**：经 WebView devtools socket 读取渲染后的 DOM ——
`View` 抽屉的面板说明为 `Standard views are not available on this carrier: it has no 3D camera.`，
七个单元（Top/Bottom/Front/Back/Left/Right/Isometric）**全部 `disabled=true`**；同一时刻
`shell_status().standardViews` = **`[]`**、`surfaceMode` = `dom-canvas`、`documentLabel` =
`synthetic-four-lines.dwg`。截图 `portrait-drawer-view.png`。

**修复 2 — 选择器改为文档选择器**：`<input type="file">` **不再带 `accept`**（不再按媒体 MIME
过滤），`multiple=false`；选中后按**扩展名**校验（`.dwg`/`.dxf`，大小写不敏感），类型不符则
`web-adapter: cad_command: OPEN refused "<name>": only .dwg / .dxf files …` 显式拒绝，**不会**把任意
字节交给导入器。

**设备实测（本轮）**：点 `Open drawing` 后前台 Activity =
`com.google.android.documentsui/com.android.documentsui.picker.PickActivity`
（文档选择器，带 Images/Audio/Videos/Documents 类型页签与 "Browse files in other apps"），
**不是** `com.google.android.providers.media.module/…photopicker.PhotoPickerGetContentActivity`
（后者只作为**上一轮**的 `mLastPausedActivity` 留在历史里）。截图 `picker-documents.png`，
`activities-after-picker.txt` 为 `dumpsys` 原始记录。返回键取消后命令栏无横幅、`Open drawing`
重新可用（取消不是失败）。

**本轮没做也不许当作已做**：

- **真实选择文件仍未验证**：`/sdcard/Download/probe-open-check.dwg` 已推入并触发媒体扫描，但
  模拟器里**无法无头驱动一次真实的文件选中**（选择器是系统 UI，需真实触摸；本轮点击由 CDP
  `userGesture` 合成）。因此「选中 DWG 后真的打开」只在 **web** 侧
  （`scripts/web-ui-smoke.cjs` 的 OPEN 用例）与本机单测里成立，**Android 上 NOT VERIFIED**。
- **错误类型的设备内拒绝未验证**：只在本机单测里覆盖（`apps/desktop-ui/test/host-adapter.test.ts`
  的 `OPEN refused "holiday.png"`），设备上未选过非图纸文件。
- **其它 Android 版本/厂商 ROM/picker 实现**：只测本 AVD（API 35、WebView 124）。OEM 的
  `ACTION_GET_CONTENT` 提供者、旧版 DocumentsUI、以及「最近」列表的媒体扫描行为**均未验证**。
- **标准视图的真实 3D**：`[]` 是**能力缺失**的诚实答案，不是"以后会实现"的占位；本轮没有、也不
  应该让 wasm 会话支持标准视图。

## 6. NOT VERIFIED（必须照写）

- **真机**：未运行（没有 Android 设备）。模拟器 ≠ 真机（CPU 架构、GPU、驱动、WebView 版本全不同）。
- **真实 GPU**：未验证。本轮是 `swiftshader_indirect` 软件渲染，**不得**用作 GPU/性能结论。
- **像素正确性**：**有界成立**：本轮修复后 CAD 分区不再是纯色，黑底 + 四条白线可见（见 §5 与
  `docs/validation-android-present-2026-10-28.md`）。但这是**软件渲染模拟器 + 一份合成夹具**，
  在任何平台上都**没有** Android 的 golden/像素/色彩空间证据（夹具是纯白线 + 纯黑底，对 sRGB
  编码不变，因此中灰等价性**未**用像素证明）。
- **其它 WebView 版本 / 系统 WebView**：只测了 124.0.6367.219（模拟器自带）。
- **手势条内边距**：只在**手势导航**（gesture navigation，pill）模式下测过，`navigationBars()` 报
  63 物理 px / 24 CSS px；**三键导航**（按键栏更高）、**导航栏在侧边**（平板/折叠屏横屏）、
  `navigationBars` 与 `ime` 同时出现（软键盘弹出时的内边距）**均未验证**。旋转后也没有单独验证
  「内边距从 63 变为 0」的分支（本机横屏手势条仍在底部，值未变，日志因此没有第二次
  `safe-bottom inset` 行——这是**未覆盖**，不是「已覆盖」）。
- **输入**：只验证了两次点按（横屏点顶栏菜单按钮 → 抽屉打开并截图；竖屏点命令框 → 软键盘弹出，
  `mInputShown=true`）。抽屉内的标签切换/滚动、手势缩放/平移、长按菜单、软键盘弹出时的布局让位
  **未验证**。
- **前后台/生命周期**（`onPause`/`onResume`、进程重建）、旋转时的 canvas 重建、分屏/多窗口：未验证。
- **aarch64/armv7/i686 构建产物**：只构建并安装了 x86_64。
- **release 签名**：仓库不含密钥；本轮用临时自签密钥安装，**不是**发布流程。
- **`OPEN` 的真实选择**：只验证"点击 `Open drawing` 打开了**文档**选择器（DocumentsUI）"，**没有**
  也能**不能**无头驱动一次真实的文件选中（见上文「命令接线缺陷修复」小节）：
  "选中 Download 里的 DWG 并打开" 在 Android 上 **NOT VERIFIED**；
  非图纸文件的设备内拒绝同样 NOT VERIFIED（只有单测）。
- **标准视图**：`cad_status.standardViews = []` 是本载体的真实答案，七个单元在设备上**全部禁用**
  并有面板说明；「点按它们会不会发 IPC」只在 **web** 侧验证（禁用按钮不挂 handler），
  设备侧为同一份代码、但**未**逐键验证。
- **标准视图与 picker 的其它环境**：只测本 AVD（API 35 / WebView 124 / DocumentsUI 自带）。
- **Gradle/AGP 的可复现性**：本轮构建留下的 `gen/android` 是 CLI 生成的版本（AGP 9.3.1 /
  Gradle 9.6.1）；换 CLI 版本需要重新 `tauri android init` 并复核本文。

## 7. 门禁

- `python3 scripts/test-android-host.py`：静态契约（布局、Android-only `tauri`、两个命令、
  标记行/退出码 6、UI 产物路径、`gen/android` 已提交、`.gitignore`、**手势条内边距的双向对接**：
  `MainActivity.kt` 必须 `enableEdgeToEdge()` + 用 `WindowInsets.Type.navigationBars()` +
  发布 `--yacr-safe-bottom`，`mobile.css` 必须消费 `var(--yacr-safe-bottom, 0px)`）。**无变异即失败**：
  删掉 CSP 的 `wasm-unsafe-eval`、换掉 `generate_handler!` 名单、或把 `navigationBars()` 换成
  `systemBars()` 都会被判红（后者已实测）。
- `cargo test -p app-android --locked`：`refusal`/`report` 的纯函数契约（host target 上运行）。
- `cargo check -p app-android --target aarch64-linux-android --locked`：**需要**
  `apps/app-android/dist` 存在（`tauri::generate_context!` 内嵌 `frontendDist`）。
- `bash scripts/build-android-ui.sh` / `bash scripts/build-android.sh`：真实构建，失败即非零。
