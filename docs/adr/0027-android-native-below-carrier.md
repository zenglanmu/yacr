# ADR 0027：Android 原生 `native-below` 载体（P5：wasm DOM-canvas → 原生 `CadSession` + `SurfaceView`）

- 状态：**Accepted（2026-11-01）**；代码已落地（P5）。证据只到**编译级 + 开发机可运行契约测试 + Kotlin
  编译**，**设备/模拟器运行、Vulkan adapter、像素、旋转/前后台、JNI 运行时链接全部 NOT VERIFIED**
  （见第 7 节）。本文**不**宣称 Android 已验收。
- 日期：2026-11-01。
- 相关 ADR：[ADR-0024](0024-dom-canvas-wasm-cad-path.md)（DOM-canvas 载体；本文**收窄**其 D1/D2 在
  Android 上的适用，**不**改其对 X11/XWayland 与浏览器的结论）、
  [ADR-0023](0023-dom-on-top-cross-host-composition.md)（跨宿主合成权威；本文取代其 §5 的
  「Android 不在本轮 / NOT IMPLEMENTED 占位」一句，**不**改 D1 的输入漏斗、弹窗围栏与单 presenter
  不变量）、[ADR-0010](0010-react-linux-host-replaces-slint.md)（React/Tauri 宿主）。
- 影响：Android 的 `cad-db` 权威位置（wasm → native）、像素承载（WebView/WebGL2 → Vulkan/`SurfaceView`）、
  命令注册（两个专属命令 → 共享 registry + 两个 shell 报表）、事件模型（adopt 共享 push 通道）。
  **不**改变：单一 React 树、`.native-input` 单一输入漏斗、renderer-only wgpu、importer-only acadrust、
  单 presenter per drawable、无逐帧整幅回读、`cad-desktop-shell` 是唯一允许依赖 `tauri` 的共享层。

---

## 1. 问题

Android 此前按 ADR 0024 的 `dom-canvas` 形态交付：Tauri WebView 里跑 wasm CAD 会话与 WebGL2 渲染，
native 壳只保留两个 shell 报表命令。这条路径在 2026-10-28 的模拟器（SwiftShader）上出过图，但它把
Android 的产品路径绑在 **WebView 的 GL 实现**上：

- 渲染能力、adapter 归属与错误形态由 WebView/GPU 进程决定，宿主无法区分硬件与软件渲染；
- 文档权威在 wasm 里，Android 与桌面 `native-below` 走**两套**命令/状态实现，能力差异（标准视图、
  文件选择、弹窗）只能在 wasm 适配器里逐条解释；
- 仓库的共享桌面语义（`cad_ui_core::session::wire`、共享注册表、push 事件通道、`hostProfile`）已经在
  Linux/Windows/macOS 上定型，Android 继续留在 `dom-canvas` 会长期分叉。

问题因此不是「Android 能不能画」，而是：

> **怎样让 Android 复用桌面的 `native-below` 契约（native 持有 `cad-db`、wgpu 呈现、共享命令与事件），
> 同时不改 React chrome、不引入第二份文档权威、不静默接受软件渲染？**

## 2. 决策

### D1 Android 是 `native-below`：native `CadSession` 独占 `cad-db`

- `apps/app-android` 现在运行**原生 Rust** `cad_ui_core::session::CadSession`，它持有 `cad-db`；
  `cad-render-wgpu` 用 **Vulkan** 呈现到 Tauri WebView **之下**的 `SurfaceView`。
- 宿主上报 `surfaceMode: "native-below"`、`documentOwner: "native"`（`HostProfile::native_below`）。
- **wasm 会话在 Android 上从不实例化**。`apps/desktop-ui` 的 `dom-canvas` transport effect 以
  `surfaceMode === 'dom-canvas'` 为门，Android 上不会加载 `/canvas/*`。
- 单 presenter 不变量按路径成立：Android 只有一个原生 CAD presenter；没有任何 wasm presenter 画同一块
  像素。

### D2 React chrome 不变，载体差异只在数据标志

- 共享的 `apps/desktop-ui` 树不变：`native-below` 上渲染 `.native-input`（唯一输入漏斗），
  **没有** DOM `<canvas>`。
- 前端路由：`android` 构建从 carrier `unknown` 起步；第一份 `cad_status` 经 Tauri 到达后按
  `hostProfile.surfaceMode` 定为 `native-below`；`web` 构建从 `dom-canvas` 起步。
- `invoke.ts` 的所有权判定是
  `adapterOwnsData = carrier === 'dom-canvas' || !tauriPresent()`：Android 上 Tauri 存在且 carrier 为
  `native-below`，因此**每条 CAD 命令都走 Tauri**，wasm 适配器不接管数据。
- 不按平台分叉：没有 `if (android)` 的 UI 分支，路由只读宿主上报的 carrier。

### D3 命令集与语义完全共享

- Android 的 IPC handler = `cad_desktop_shell::commands::register_commands()`（共享注册表，
  `generate_handler!` 只有一份来源）**加上**本载体继承的两个 shell 报表
  `canvas_session_status` / `canvas_runtime_failed`。报表仍有效（chrome 的 shell 上报通道），但在
  `native-below` 上 chrome 不会发送 wasm 运行时失败。
- 原生命令语义 = `cad_ui_core::session::wire`（与桌面同一套参数解析、准入、错误码），不存在 Android
  专属命令表。
- Android 没有 shell 退出码可判：证据的第一分类依据仍是 `logcat` 稳定标记行。

### D4 事件是 push-only，`cad_status` 不轮询

- 共享 `cad_event` 通道是**推送**：每帧/每次 flush 发送一次 `status-changed`，携带**完整**状态
  （含 `hostProfile`），不做扩散门控；`frameCount`/`cursorWorld` 都搭这条事件，因此没有独立的
  `frame-advanced`/`cursor-moved`。
- 语义事件：`command-admitted`、`command-rejected`（仅批内相邻归因——`Error` 必须直接跟在被准入的
  `Command` 之后且两者同批）、`selection-changed`、`tool-step`、`document-opened`/`document-closed`、
  `error-raised`、`popup-changed`、`events-dropped`。
- **前端不再轮询 `cad_status`**；`cad_status` 只用于订阅/bootstrap 与丢事件后的恢复。
- `status-changed` 在每次 flush 里排在语义事件**之后**，消费者先看到命令/选择变化，再看到已包含其
  结果的快照。

### D5 wire：`hostProfile` 恒在

每条 `cad_status`（以及 `status-changed` 里的状态）都带**始终存在**的 `hostProfile`：

```json
"hostProfile": {
  "surfaceMode": "native-below",
  "documentOwner": "native",
  "palettesAvailable": true,
  "standardViews": null
}
```

`standardViews: null` = **未声明**（`native-below` 的既有语义：不限制）；`[]` = **已声明、没有可用项**
（`dom-canvas` 的语义）。这取代了此前「按键是否存在来猜载体」的隐式约定。

### D6 像素与 JNI：`SurfaceView` 在 WebView 之下，Vulkan only

- Kotlin 侧在附着后把 `SurfaceView` 插到 WebView 父容器的 **index 0**（即 WebView **之下**），因为
  Wry 的 `onWebViewCreate` 在 `setContentView` **之前**运行；`setZOrderOnTop(false)` /
  `setZOrderMediaOverlay(false)`，WebView 背景清成透明。
- 冻结的 4 个 JNI 符号（包 `dev.yacr.app`）：
  `nativeSurfaceCreated` / `nativeSurfaceResized` / `nativeSurfaceDestroyed` / `nativeAssetsReady`。
  `Long` 只是不透明原生 handle，Kotlin 不持有 CAD 状态。
- 字体/资源从 APK 解压到 `filesDir` 后由 `nativeAssetsReady(fontsDir, imagesDir)` 公告；空字体目录
  显式拒绝，不伪造字体面。
- **只接受 Vulkan 硬件 adapter**。软件 Vulkan（SwiftShader / lavapipe / CPU）显式拒绝：稳定标记行
  `yacr: unsupported-gpu: software-vulkan`；**没有 GLES 回退**，也没有 CPU 路径。空白/黑屏不是成功。

### D7 指针映射与视口（当前边界）

- 指针/滚轮坐标在 tick 里按 DOM 分区原点平移（分区局部 → presented canvas），纯函数、可在开发机测试。
- `fit_camera` 仍按**整个窗口**取景（`SurfaceView` 是全窗口）；CAD 分区小于窗口时，FIT 的取景范围
  与分区不一致。这是**已记录的限制**，不是新的正确行为（见第 8 节 follow-up）。

## 3. 否决的备选方案

- **保留 Android 的 wasm `dom-canvas` 载体**。否决理由：①渲染能力与 adapter 归属由 WebView 决定，
  无法执行「软件 Vulkan 必须拒绝」这条诚实性规则；②Android 与桌面继续维护两套命令/状态实现，
  共享注册表与 `hostProfile` 的价值在移动端归零；③wasm 会话的堆预算与 WebView GL 限制成为 Android
  的产品上限，而不是平台事实。历史证据（2026-10-28 模拟器 SwiftShader 出图）保留，但**不**作为本载体
  的证据。
- **native + wasm 双权威 / 把 native 显示表示流进 wasm**。否决理由与 ADR 0024 §3 的 M1 相同：
  双数据路径、增量 wire 语义要重新编码一遍、直接违反铁律 8 的「唯一 `cad-db` 权威」。Android 上
  native 会话已经存在，任何「顺手把状态镜像给页面」的便利代码都会把双权威变成事实。
- **软件 Vulkan 回退 / GLES 回退**。否决理由：静默降级会把「模拟器能出图」冒充成设备能力；
  能力缺失必须显式 `Unsupported`/标记行，不得空成功。

## 4. 后果

**正面**：

- Android 与 Linux/Windows/macOS 共用一套命令语义、注册表、事件通道与 `hostProfile`；移动端不再是
  第二套 CAD 实现。
- `cad-db` 权威位置由 carrier 显式声明（`documentOwner`），不再靠「哪个宿主多了哪些键」推断。
- 渲染路径走仓库唯一的生产渲染器（wgpu）；Android 上后端固定 **Vulkan**，adapter 归属可判定，
  软件 adapter 可拒绝。
- React chrome 零分叉：同一棵树、同一输入漏斗、同一套 DOM 弹窗/菜单渲染（Android v1 未接弹窗宿主，
  相关命令显式拒绝）。

**代价与风险**：

- **Android 变成有状态的原生宿主**：tick 线程、`SurfaceHolder` 生命周期、JNI 交接、资源解压都进入
  Android lane；这些正是本轮**完全没有设备证据**的部分。
- **Vulkan 是硬门槛**：没有 Vulkan 硬件 adapter 的设备/模拟器直接不可用；这比 `dom-canvas` 的
  WebGL2 门槛更窄，必须如实告知，不能悄悄退回 GLES。
- **v1 能力明显小于桌面**（第 5 节）：没有文件、没有弹窗、没有持久配置。UI 必须显式禁用/拒绝，
  不得把「桌面有」当成「Android 有」。
- **测试矩阵**：编译级契约测试只能钉住接线与窗口常量；像素/输入/生命周期仍需要设备证据。
- **旧的 Android 证据不可迁移**：2026-10-28 的模拟器 DOM-canvas 像素/命令证据属于**已退役载体**，
  不能拿来证明 native carrier。

## 5. Android v1 的显式上限（写死）

| 能力 | v1 状态 | 表现形式 |
|---|---|---|
| 打开图纸 / 新建 / 保存 / 出图 | **不支持** | 全部 `CadError::Unsupported`；`canOpen`/`canNew`/`canSave`/`canPlot` 为 `false`；不伪造文件身份 |
| 弹窗宿主 | **未接** | `state.popup_host = None`；共享弹窗命令**显式拒绝**（`no popup host registered`） |
| 持久配置/偏好 | **不支持** | `NonPersistentConfigStore`：读 `Absent`，写入 `Unsupported` |
| 文件身份（drawing label / path / URI） | **不伪造** | 启动无 `--open`，`drawing: None` |
| 初始文档 | **共享核心内置 demo 图纸** | 与桌面宿主不带 `--open` 启动时完全相同；不是导入结果 |
| 渲染后端 | **Vulkan 硬件 only** | 软件 Vulkan → `yacr: unsupported-gpu: software-vulkan`；无 GLES/CPU 回退 |
| 视口/FIT | **整窗取景** | `SurfaceView` 是全窗口；分区子矩形取景未实现（follow-up） |
| 指针 | **分区原点平移** | tick 内按 DOM 分区 origin 换算；不是独立输入通道 |

> **2026-11-01 修订（追加，不改写上面的历史决定）**：表中「弹窗宿主 / 持久配置」两项已被后续实现
> 取代。弹窗是 in-DOM 覆盖层，`AndroidPopupHost` 把共享生命周期跳到 loop 线程执行（有界 5 s），
> Options / About / text-edit 可用；`AndroidConfigStore` 把 `config.json` / `preference.json` 写入
> 应用私有 `filesDir`（公告 `fontsDir` 的父目录，JNI 契约未改），握手前保持非持久。其余各行
> （文件/伪造身份/Vulkan 硬门槛/整窗 FIT）**仍然有效**。设备运行与落盘仍 **NOT VERIFIED**。
>
> **2026-11-02 更新（追加，不改写上面的历史决定）**：表中「打开 / 保存」已由 SAF 实现，不再是不支持：
> `canOpen`/`canSave` 为 `true`，打开走 `ACTION_OPEN_DOCUMENT`，另存走 `ACTION_CREATE_DOCUMENT` 并导出
> **DXF**（DWG 保存仍显式延后 `save.dwg_deferred`，且不原地覆盖来源 URI）。**新建 / 出图仍不支持**
> （`canNew`/`canPlot` 为 `false`）；「伪造文件身份」与「Vulkan 硬门槛 / 整窗 FIT」两行仍有效。
> 设备/模拟器上的选择器、读写与外部「打开方式」**NOT RUN**。

## 6. 失效条款清单（逐条对照）

| 文档 | 位置 | 原表述（摘要） | 处置 |
|---|---|---|---|
| ADR 0024 | D1 两模式表（Android 此前沿用的框架）/ D2「`dom-canvas` 宿主由 wasm 独占权威」 | Android 按 `dom-canvas` 交付（wasm 持有文档与像素） | 本文**收窄**：Android 不属于 `dom-canvas` 集合；D1 的输入漏斗/单 presenter 与 D2 对 X11/浏览器的结论**不变** |
| ADR 0023 | §5 非目标 `:211` | 「macOS / Android / iOS 不在本轮；Android/Web 仍是 NOT IMPLEMENTED 占位」 | Android 已由本文承接为 `native-below`；该句对 macOS/Web 的历史状态不改写，但**不再适用于 Android** |
| `docs/android-host.md` | 全文 | 「native 不持有 CAD 状态；只有两个命令；wasm 持有文档」 | **本轮重写**为新契约 |
| `docs/architecture.md` | 载体/平台范围 `app-android` 行 | Android 与 Web 并列「Tauri/浏览器载体，wasm `dom-canvas` 会话」；平台范围写 Android 是占位 | **本轮更新**（载体表、`cad-db` 权威、平台范围） |
| `docs/build.md` | 顶部迁移注记、入口表、Android 段 | Android 描述的 wasm 会话/`dom-canvas` | **本轮更新**；构建事实（`scripts/build-android.sh`、release 未签名、CI 只编译）保留 |
| `docs/ci.md` | `android-check` 行、退役 job 含义段、NOT RUN 设备段 | Android = WebView + wasm `dom-canvas`；证据是模拟器运行 | **本轮更新**为 native 载体 + 编译级证据；「CI 无设备 job」原样保留 |
| `README.md` | 顶部状态、crate 树、Other platforms、平台表 Android 行 | Android 是 NOT IMPLEMENTED 占位 | **本轮更新** |
| `docs/compatibility.md` | 交互/TRIM 段末句、平台表 Android 行 | Android 是 NOT IMPLEMENTED 占位 | **本轮更新** |
| `AGENTS.md` | 铁律 8（及平台状态表） | 仍把 Android 列为 `dom-canvas` 宿主 | **本轮不改**（不属于本文 lane）。铁律 8 的修正必须由当前 `AGENTS.md` 的持有者完成（该文件在本轮工作树里已处于修改状态）；在那之前，本文与代码是 Android 事实的权威。 |
| `CAD_IMPLEMENTATION_SPEC.md` | §9.1、§21 增补、§9 表 | 仍写 Android NOT IMPLEMENTED 占位 | **本轮未同步** → **同步债务**（SPEC 是需求权威，必须由 SPEC 持有者修订；本文不代替它） |

## 7. 证据状态（写死，防误读）

**已运行（编译级 / 开发机）**：

- `cargo check -p app-android --target aarch64-linux-android --locked`；
- Android lane 的 clippy；
- host-runnable 契约测试（`policy`/`platform`/`input`/源码结构；`cargo test -p app-android --locked`
  在开发机运行）；
- Kotlin 编译（Gradle 工程）；
- `javap` 核对 4 个 JNI 符号的导出名。

**NOT VERIFIED（全部未运行）**：

- **没有**设备或模拟器运行（未安装、未启动 APK）；
- **没有** Vulkan adapter（未枚举、未创建 device/queue）；
- **没有**像素（无帧、无截图、无 golden、无色域）；
- **没有**旋转 / `onPause`/`onResume` / 进程重建 / `SurfaceView` 重建；
- **没有** JNI 运行时链接（`System.loadLibrary` → 符号解析 → 回调）证据；
- **没有**性能、功耗、触摸/手势、软键盘、分屏/多窗口证据。

因此本文的最高诚实表述是：**「native `native-below` 载体已实现并接线，编译级契约通过；设备运行与
视觉证据均为 NOT RUN」**。`scripts/test-android-host.py` 的断言是**静态接线**，不是设备证据。

## 8. 后续（follow-up，不在本轮）

1. **FIT/视口子矩形**：`fit_camera` 按 CAD 分区（surface 减 chrome 占用）取景；需要
   `cad-engine` 的 viewport/inset 概念，而不是在 Android lane 里做坐标补丁。
2. **弹窗能力标志**：在 `hostProfile` 或能力布尔里显式声明「本载体无弹窗宿主」，让 UI 在发 IPC
   **之前**禁用相关控件（当前是命令到达后拒绝）。
3. **SAF 文件访问**：用 Android Storage Access Framework 接入打开/保存；在此之前 `canOpen`/`canSave`
   必须保持 `false`，不得用私有路径伪造文件身份。**（已实现 2026-11-02：`canOpen`/`canSave` 现为
   `true`；实现与 NOT RUN 边界见下方补记。）**
4. **设备 CI job**：真实设备或硬件加速模拟器上的安装/启动/像素/Vulkan adapter 断言；在它存在之前，
   CI 的 `android-check` 仍只是编译门禁。
5. **构建产物清理**：`scripts/build-android-ui.sh` 目前仍会产出 wasm 会话产物（本载体不加载）；
   是否删除属构建 lane 的独立改动，本文只记录事实。

> **2026-11-02 补记（追加）**：第 3 项 SAF 已落地——新的 Android-only `src/saf.rs` + 手写提交的
> `DocumentPicker.kt` / `MainActivity` 五个 `document*` 实例方法实现了打开（`ACTION_OPEN_DOCUMENT`，
> `*/*`，worker 读后以 `FileEvent::Opened` 交给共享漏斗）与 DXF 另存（`ACTION_CREATE_DOCUMENT`，
> `ContentResolver.openOutputStream(uri, "wt")`）；`bundle.fileAssociations` 提供外部「打开方式」。
> `canNew`/`canPlot` 仍为 `false`，DWG 保存仍是 `save.dwg_deferred`。**设备/模拟器上的选择器、
> 实际读写、外部 intent 与 JNI 运行时均 NOT RUN**，本轮证据只到开发机契约测试与编译级。

## 9. 历史保留（不抹、不改写）

- 2026-10-28 的 Android `dom-canvas` 证据（模拟器、`swiftshader_indirect`、四线合成夹具、手势条内边距、
  DocumentsUI 选择器）**作为历史记录保留**，其对象是**已退役载体**。它**不得**被引用来证明 native
  carrier 的像素、Vulkan adapter、输入或生命周期。
- `scripts/test-android-host.py` 的旧断言（wasm 会话、两个命令、`surfaceMode: dom-canvas`）已被本轮
  新增的 native-carrier 断言取代；旧断言不是「曾经通过所以现在仍通过」。
- 本文不翻案 ADR 0024 对 X11/XWayland 与浏览器的结论：`dom-canvas` 仍是它们的载体，Android 只是
  **退出该集合**。
