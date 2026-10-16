# 原生 Wayland 后端验证（2026-10-16，有界、分区）

状态：**有界源码/构建/门禁 + 有界真实原生 Wayland 运行已执行**：`--probe`、原生 CAD（`--open`，4 实体）、
9 个合成 DOM 输入 + 相机变化、resize 遥测、强制 overlay 生命周期释放/重建/恢复、几何修复后的**有界截图视觉 PASS**
（Observer 终态）与 X11 运行回归。**最终 Oracle W3 attempt3 终判：ACCEPT-WITH-LIMITATIONS（无 blocker）——有界本地
验收（GNOME/Mutter + Quadro P620 + `scale=1`），不是完整产品/跨平台/vendor/golden/性能/精度验收。** 本文件只记录
实际运行过的东西，未运行的一律标 NOT RUN。

需求与边界：`docs/adr/0011-native-wayland-subsurface.md`、`docs/react-linux-app.md`、
`docs/architecture.md`；逐轮交接见 `docs/handoff.md`。本轮**不改**历史 ADR 0006/0010 的初始范围。

- 证据根目录（本轮）：`/tmp/opencode/yacr-wayland-20261016T065457Z/`
  - `compile/remediation/`（scoped 编译/测试/clippy/wasm/门禁）
  - `compile/`（**首次失败**的编译日志，保留为负面证据）
  - `native-probe/`（原生 Wayland `--probe` 运行日志）
  - `cad-validation/`（原生 CAD/输入/生命周期/X11 用例；含**失败截图保留**）
  - `lifecycle-fix/`、`geometry-fix/`（源码修复的 scoped 检查、release build、原生运行与 Mutter 合成截图）
- 最终 release 二进制：`target/release/yacr`，
  **sha256 `7618d0d263498bac75c68eabc18ec6fcaba310a412a06d2467d83552b1393129`**（26,998,528 B）。
- 工具链：`cargo/rustc 1.99.0`（`rust-toolchain.toml`）；`CARGO_BUILD_JOBS=2`；全程 `--locked`，
  `Cargo.lock` 未改（其脏状态仅为已保留的迁移基线）。

## 1. 范围与授权

- 唯一被授权的 Linux 可运行宿主是 `apps/app-linux`（二进制 `yacr`）。本轮加入**原生 Wayland**
  后端，与既有 X11 后端并存；非 Linux 目标仍是显式 NOT IMPLEMENTED 占位（未变）。
- 源码阶段先经 Oracle 源码门禁（见 §4），再经 scoped 编译/契约，最后才允许**有界**原生 `--probe`
  硬件运行。**源码接受不是运行验收**。

## 2. 后端策略（选择意图 vs 实际后端）

选择意图与实际 GDK 后端/协议证据**分开记录**：

- 选择意图在 GTK 启动前由 `apps/app-linux/src/backend.rs` 解析 `GDK_BACKEND`：
  - `Auto`（未设）：`WAYLAND_DISPLAY` + `XDG_RUNTIME_DIR` 同时存在 → 原生 Wayland（优先）；否则
    仅有 `DISPLAY` → X11/XWayland；两者皆无 → 明确报错（**没有无头成功**），退出码 2。
  - `GDK_BACKEND=x11` / `GDK_BACKEND=wayland`：显式接受并**原样保留**；Wayland 需要
    `WAYLAND_DISPLAY` + `XDG_RUNTIME_DIR`，否则硬错（**不静默回退到 X11**）。
  - 其它值 / 大小写不符 / 空或含空白 / 多后端列表：硬错（引用原值），退出码 2。
  - 只有 `Auto` 会把**解析结果**导出为 `GDK_BACKEND`，从而 GDK 不能自行回退；用户显式值从不改写。
- **实际**后端/协议身份由运行时证据单独证明（§5）：`[probe] display backend … resolved=Wayland`、
  顶层 `wl_surface`/`wl_display` 对、app-owned registry bind（`wl_compositor`/`wl_subcompositor`）、
  `get_subsurface`、`set_desync`、`set_position`、`set_buffer_scale`、空 input region。
  选择意图**不等于**运行证据。

## 3. scoped 编译/门禁结果（实际运行）

最终证据：`compile/remediation/`（run `2026-10-16T07:16Z` 起）。

| # | 命令 | 退出码 | 结果 |
| --- | --- | --- | --- |
| 1 | `cargo check -p app-linux --all-targets --locked` | 0 | 编译通过（含 build.rs 经 `cc` 编译的 C shim） |
| 2 | `cargo test -p app-linux --lib --locked` | 0 | **58 passed / 0 failed**（后续增量：lifecycle 修复 +1、几何修复 +1） |
| 2b | `cargo test -p cad-ui-core --lib --locked` | 0 | **61 passed / 0 failed**（几何数值回归 +1） |
| 3 | `cargo clippy -p app-linux --all-targets --locked -- -D warnings` | 0 | 0 warning |
| 4 | `cargo fmt -p app-linux -p cad-render-wgpu -- --check` | 1→0 | 首次 9 处格式差异；父级执行 `cargo fmt`（仅格式）后 scoped fmt check 退出 0（几何修复再 1 处，仅格式） |
| 5 | `cargo check -p app-linux --lib --target wasm32-unknown-unknown --locked` | 0 | cfg 隔离成立（原生模块在非 Linux 目标不编译） |
| 6a | `python3 scripts/check-architecture.py` | 0 | `27 packages, acyclic and core boundaries intact` |
| 6b | `python3 scripts/check-i18n.py` | 0 | 363 keys、2 catalogs、键/占位一致、无硬编码 CJK |

**未运行（NOT RUN）**：无全 workspace 测试/clippy 全扫、无 `check-fixture-manifest.py`、
`check-workflows.py`、无 renderer 独立测试、无 npm 重建（复用既有 `ui/dist`）。因此 §3 是
**scoped 结论**，**不是**全仓库门禁结论。

### 3.1 保留的首次失败（负面证据，不抹除）

首次 scoped 编译 **失败（exit 101）**，共 4 个错误，其中 lane A（`wayland_surface.rs`）3 个：
`E0282`（`display.to_glib_none()`／`window.to_glib_none()` 指针类型推断不足，2 处）与 `E0797`
（测试里 `matches!` 的第一个参数用了 `..` 结构表达式，1 处）；lane B（`host.rs`）1 个 `E0609`
（`WaylandWindowHandle` 无 `display` 字段）。日志保留于 `compile/`（负面证据，`exit 101`）。
源码门禁 attempt1 随后 **REJECT**（见 §4），修复后 attempt2 通过。**这两次都没有运行原生应用**。

## 4. 源码门禁（Oracle，仅源码）

- W1 attempt1：**ACCEPT-WITH-FIXES**；锁定：app-linux 私有小 C shim、GTK 独占分发、先做有界
  `--probe` 再谈 CAD 集成、presenter→subsurface→child 销毁顺序且早于 GTK parent unrealize。
- W2 attempt1：**REJECT**（缺 registry/discovery 完成、Ready 健康/父替换、父 commit 重绘、attach
  失败清理、诚实 panic payload、精确后端 token、checked physical size 等）。
- W2 attempt2：**ACCEPT-WITH-FIXES（无新增问题）**（确切判词；不是无条件的 ACCEPT），明确“源码接受 ≠
  运行证据；CAD/多平台不自动接受”。
- 细节见 `.slim/deepwork/native-wayland/gate-w1.md`、`gate-w2-attempt1.md`、`gate-w2-attempt2.md`、
  `w2a-remediation.md`、`w2b-remediation.md`。

## 5. 原生 Wayland `--probe` 硬件证据（实际运行）

来源：`.slim/deepwork/native-wayland/w2-native-probe.md`（run 日志在 `native-probe/`）。

- 生产 release 构建：`cargo build --release --locked -p app-linux --bin yacr --features
  tauri/custom-protocol` → **exit 0**（2m14s）；使用新二进制 `target/release/yacr`（sha256/`file`/`ldd`
  记录在 `01-binary-ldd.log`）。
- 进程环境（`/proc/<pid>/environ`，**逐字**）：
  `GDK_BACKEND=wayland`、`WAYLAND_DISPLAY=wayland-0`、`XDG_RUNTIME_DIR=/run/user/1000`、
  `DISPLAY`/`XAUTHORITY` **未设**；无 `VK_ICD_FILENAMES`/软件 ICD 选择。宿主为真实 GNOME/Mutter
  Wayland 会话（`/run/user/1000/wayland-0`）；`Xwayland :0` 存在但本进程从未使用。
- 用例 1（`target/release/yacr --probe`，约 29s 后计划 SIGTERM）：`wait_exit=143`（**计划超时，非自然退出**）。
  遥测：`policy=Wayland resolved=Wayland source=from environment`；顶层
  `surface=0x… display=0x…`；`drawing_area_has_window=false`（Wayland 上**不** `ensure_native`）；
  `globals ready wl_compositor=v4 wl_subcompositor=v1`；app-owned child `pos=(326,273) scale=1
  phys=574x167`；presenter `backend=vulkan adapter=Quadro P620 device_type=discrete_gpu
  present_mode=Mailbox`；`frames=1686 ticks=1719 … error=None`。
  协议（`WAYLAND_DEBUG=client`，**仅 Default Queue**，GTK 独占分发）：app bind
  `wl_compositor`(v4) 与 `wl_subcompositor`(v1)、`create_surface`、`get_subsurface`、`set_desync`、
  `set_position(326,273)`、`set_buffer_scale(1)`、**空 input region**（`create_region` + 
  `set_input_region` 且**无 `wl_region.add`**）；child `#53` 走 dmabuf/`surface_feedback` 物理 GPU 呈现；
  **无** `wl_display.error`/协议错/角色错；`DISPLAY` 未设 + 真实 `wl_subcompositor` 交换 → **非 XWayland**。
  GPU 证明：活跃进程打开 `/dev/nvidia0`、`/dev/nvidiactl`、`/dev/dri/renderD128` 等；`nvidia-smi`
  报 `Quadro P620 00000000:01:00.0`，与适配器遥测一致。
- 用例 2（`YACR_PROBE_LIFECYCLE=exit … --probe`）：**自然退出 0**（非被杀）；同后端/适配器；
  resize grow/shrink 均 `ok` 并重配 surface、更新 `set_position`；退出顺序为
  `GPU released (generation 1; surface before native widgets)` →
  `wayland child surface released (before GTK parent unrealize)` → `run event: Exit`，无错误。

## 6. W3 原生 CAD / 输入 / 生命周期 / 几何 / 视觉 / X11 运行证据（实际运行）

来源：`.slim/deepwork/native-wayland/` 的 `w3-cad-x11-validation.md`、`w3-lifecycle-runtime.md`、
`w3-geometry-runtime.md`、`w3-native-visual-capture.md`；原始日志/图在
`/tmp/opencode/yacr-wayland-20261016T065457Z/{cad-validation,lifecycle-fix,geometry-fix}/`。

- **原生 CAD（`--open`）**：真实原生 Wayland 载入合成图纸，`entityCount:4`、`current_scene` 帧、
  `backend:vulkan`、`adapter:Quadro P620`、`error=None`（`cad-validation/case1`）。**9** 个合成 DOM 输入
  （`resize`/`pointer_down`/`move`/`up`/`wheel`）全部 `accepted=true` 且 **camera 实际改变**
  （`center [1.5,0.5]→[0.7789,-0.0548]`，`scale 0.005032→0.007862`）；WM resize 遥测同步更新
  （`cad-validation/case1b`）。**注意**：合成输入是 IPC 层证据，**不等于** OS 真实用户指针的像素精度验收。
- **生命周期**：`YACR_PROBE_LIFECYCLE=unrealize` 触发的是**overlay** unrealize（保留了 GTK toplevel
  父 `wl_surface#34`，两次 `get_subsurface` 复用同一父），因此是**子表面释放/重建/恢复**证明，
  **不是** GTK **toplevel** `wl_surface` 替换证明。分类修复后：旧二进制 `cb12c3fa` 的
  **120** 个 `cad-frame error`（case2 原始报告记 118 gap + 2 startup）+ **8** 个
  `"cad present failed: no GPU presenter (bridge not ready)"` 样本（**保留为历史原始证据，两处计数口径不同，
  按各自 case 引用**）→ **生命周期修复二进制 `6894e423`** 为 **0 error / 122 `cad-frame waiting`**，gap 内**无**
  presented 帧递增，gen2 重建**新 child** 且 `device_epoch=2` `current_scene` 恢复，**自然退出 0**
  （`lifecycle-fix/runtime`）。**限制**：该 `6894e423` **未**在最终几何二进制（`7618d0d2`）上复跑；展示门的
  分类逻辑（host admission）在几何轮**未改动**，故两轮行为等价但**未在同一二进制上复核**。
- **几何修复**：旧二进制（`cb12c3fa`）原生子缓冲比 DOM CAD 矩形小 **52×99**（视觉失败：灰色条带），
  最终二进制 **sha256 `7618d0d263498bac75c68eabc18ec6fcaba310a412a06d2467d83552b1393129`** 后：
  baseline `DOM 926x266 → child 926x266`（旧 874x167）、resize `1166x513 → child 1166x513`（旧 1114x414），
  容器 `960x553`/`1200x800`，margins `end=34 bottom=84`，无溢出（`geometry-fix/runtime`）。
- **原生视觉（有界截图视觉 PASS）**：最终二进制下由**自身同 UID 的 Mutter `org.gnome.Mutter.ScreenCast`**
  （+GStreamer `pipewiresrc→pngenc`）捕获 baseline/resized 两帧真实合成输出
  （`geometry-fix/runtime/capA-native-baseline.png`、`capB-native-resized.png`）；**无安全策略变更、
  无安装、无 sudo，用户已授权**，会话均已 `Stop()`。Observer 终态裁决：旧 A/B 的**右侧 ~52px / 底部 ~100px
  灰带消失**，CAD `926x266`/`1166x513` 贴合 nav/modelstrip，无覆盖/间隙/溢出/未翻译错误，非空几何可见
  → **有界截图视觉 PASS**（**无** golden/精度/vendor 声明）。
- **执行退出码（按 case 精确，勿概括为“全部自然 0”）**：原生 baseline `capA` 捕获后**强制 SIGTERM，
  exit 143**；原生 resized `capB` **自然 0**；X11 smoke **自然 0**。
- **X11 运行时回归**：`GDK_BACKEND=x11` 真实 XID/Xlib 原生子窗口、`entityCount:4`/`current_scene`/`error:null`、
  `present_mode=Immediate`、同一坐标规则下 settled native = DOM、**自然退出 0**（`geometry-fix/runtime`）。
  X11 的 PNG（`capX11-toplevel.png`/`capX11-cad-child.png`）**不是**原生证据，且本机 reader **无法解码**，
  Observer **不判** X11 视觉——X11 只作**运行证据 PASS**。

## 7. 明确 NOT RUN / 待办（不得当成已支持）

- **最终 Oracle 终判：ACCEPT-WITH-LIMITATIONS（W3 attempt3，无 blocker）** —— 有界本地验收：本机
  GNOME/Mutter + Quadro P620 + `scale=1`；**不是**完整产品/跨平台/vendor/golden/性能验收。判词见
  `.slim/deepwork/native-wayland/gate-w3-final.md`。历史 attempt/report 中的 PENDING 叙述保留。
- **X11 视觉**：X11 的 PNG **未**被 Observer 判读（reader 无法解码），故**不是** observer 视觉 PASS；
  X11 仅为**运行证据 PASS**。
- **DPR>1 / 分数缩放**：整数 DPR 只测 `scale=1`；**不支持、未验证**（显式）。
- **GTK toplevel `wl_surface` 替换**、**GPU 设备丢失/热插拔**、P620 之外的 **vendor/adapter 广度**、
  **几何/像素精度验收**（合成输入≠真实用户指针精度）、**性能/60fps**、**golden**、**打包/跨平台**
  均**未验证**。
- 原生路径**只**在物理 P620 上验证，**未**在软件 ICD 下复跑（软件结果若产出必须显式标注）。
- 无全 workspace 测试全扫、无 npm 重建、无打包。

## 8. 诚实结论

- **已执行**：scoped 编译/契约（`app-linux` 58、`cad-ui-core` 61）/clippy/fmt/wasm 隔离/架构/i18n（§3）、
  源码门禁 attempt2（§4）、有界原生 `--probe`（§5）、原生 CAD+合成输入+生命周期+几何+视觉+**X11 运行证据**（§6）。
- **最终 Oracle 终判（W3 attempt3）：ACCEPT-WITH-LIMITATIONS，无 blocker** = 有界本地验收
  （GNOME/Mutter + P620 + `scale=1`），**不是**完整产品/跨平台/vendor/golden/性能验收。
- **未执行 / 未支持**：DPR>1、toplevel 替换、设备丢失、vendor 广度、精度/性能/golden/打包/跨平台（§7）。
- 仍**不能**把上述结果推广为“完整原生 Wayland CAD 支持”；当前 release 二进制为
  `target/release/yacr`（sha256 `7618d0d2…`），**用户手动本地复测可选自行进行**；非 Linux 占位未变。
