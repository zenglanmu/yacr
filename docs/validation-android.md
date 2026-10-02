# Android 模拟器安装与运行验证（x86_64，SwiftShader）

本文件记录 **实际执行过** 的 Android 构建、安装、启动与交互验证。所有结论均来自本机
`emulator-5554` 的 logcat、截图与像素 diff；未观察到的能力一律标注 **NOT RUN** 或
**限制**，不作成功声明。

> 平台：Android 模拟器（`dev_api35`，API 35，`google_apis`，x86_64），KVM +
> **SwiftShader** 软件 GPU（无 3D GPU）。**这不是真机**，不能据此声明真机兼容性。

关联文档：`docs/build.md`（Android 小节）、`docs/validation.md`（总表）、
`docs/render-backends.md`。

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
| APK SHA-256 | `298b65b8b790e4555efaf81544aa69d43a3e05fd425af0f137c4ada365a3f01d` |
| native-code | `x86_64` |
| package / activity | `dev.yacr.app` / `android.app.NativeActivity` |
| 签名 | `CN=yacr dev, O=yacr`，SHA-256 `ae556b93ab916181980f09c45981331af65b2be5c0fc14aeb9fb5b921cae887f`（本地开发密钥，未入库） |
| 实测 manifest | `sdkVersion:'23'` / `targetSdkVersion:'30'`（metadata 里的 26/34 仍未生效，见 `docs/build.md`） |

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
- 批注安全：Android 未安装 `CanvasPickMapper`，且 `canvas-pick` 仅在量测/批注工具激活时
  才动作；因此画布手势不会误提交批注。批注面板保持空（`批注` 无行）。

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
- **surface 尺寸与安全区（audit U07）**：`UiConfiguration::safe_insets` 未被任何代码
  消费；Android 未在 surface 变化时回传逻辑尺寸、未重算 `apply_responsive`，也未对
  渲染目标做画布矩形偏移（`Image` 用完整窗口的帧）。导致顶部工具栏被状态栏遮挡、
  布局按配置值 [1080,1920] 而非真实 surface 推导，并可能被裁切。
- **量测/批注拾取**：Android（与 Web 相同）未安装 `CanvasPickMapper`；点击画布在工具
  激活时仅报告 `status.pick_unwired`，不产生世界点。
- **面板状态未推送**：Android 宿主未调用 `set_layer_state` / `set_layout_state` /
  `set_annotation_state` / `set_diagnostics_state`，图层/布局/批注/诊断面板显示显式
  空状态。
- **SAF/文件选择器**：未实现（沿用候选路径）。
- **打开 DWG**：NOT RUN（见 §6）。
- **`shader_model: Sm5` 的 downlevel 警告**：`SURFACE_VIEW_FORMATS` 缺失属模拟器
  Vulkan 能力提示，未观察到渲染失败。
- **helper 脚本**：见 §1 的 `find target` 退出码注意点。

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
