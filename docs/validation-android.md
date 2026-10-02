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
| APK SHA-256 | `f10de4e2fcf90e9272375ac928f8bcd09b5cc7b3c3279a77c4b7c7f3afbeb44c`（提交源码重建；首轮验证 APK 为 `298b65b8…`，大小相同。APK 非字节可复现，zip/签名时间戳会改变哈希） |
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
