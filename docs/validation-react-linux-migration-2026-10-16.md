# 验证：Slint → React/Tauri 桌面迁移（2026-10-16）

状态：**已执行（有界、分区）**；**Gate P6 attempt2：ACCEPT-WITH-FIXES（仅文档限定，已应用）**，observer
对已观察截图终判**无重大视觉缺陷**。**有界本地 Linux 迁移验收通过**；**不是**完整 CAD 产品/跨平台/
vendor DWG/golden/性能验收。本文记录**实际运行**的证据（命令、退出码、日志路径、限制）。

- 代码树：HEAD `ca69374d46c93cd1f8a80cedbc9ae173cfe062ec`，分支 `experiment/react-linux-ui`（含未提交迁移改动）。
- 证据根：`/tmp/opencode/yacr-p6-20261016-030149Z/`
  （`results.tsv` 汇总；`logs/` 初始 sweep；`lock-recovery/logs/` 锁复跑；`gpu/` 真实 GPU；`gpu-targeted/` 定向复检）。
- 环境：rustc/cargo 1.99.0；node v24.22.0 / npm 11.19.1；pkg-config 2.5.1；gtk+-3.0 3.24.52；
  webkit2gtk-4.1 2.52.6；12 核 / 30Gi RAM。GPU：**NVIDIA Quadro P620**（driver 580.178.04，离散，硬件
  Vulkan），真实 GNOME Wayland + XWayland `:0`；`VK_ICD_FILENAMES`/`VK_DRIVER_FILES` **未设置**（无软件 ICD）。
- 未改代码/清单/脚本；无 `git restore/clean`；未安装系统包。

## 1. 前端（apps/app-linux/ui）

| 命令 | 退出 | 结果 / 日志 |
|---|---|---|
| `npm ci` | 0 | 19 packages，`logs/npm-ci.log` |
| `npm test` | 0 | **43 passed / 0 failed**（`node --test test/*.test.mjs`），`logs/npm-test.log` |
| `npm run build` | 0 | 40 modules → `dist/`（`index.html` + assets），`logs/npm-build.log` |

`ui/dist` 在 cargo 编译前生成（Tauri `frontendDist` 依赖）。

## 2. 静态门禁与契约（锁无关；全通过）

| 命令 | 退出 | 结果 / 日志 |
|---|---|---|
| `cargo fmt --all -- --check` | 0 | 无输出，`logs/cargo-fmt-check.log` |
| `python3 scripts/check-architecture.py` | 0 | `Architecture OK: 27 packages, acyclic and core boundaries intact` |
| `python3 scripts/check-fixture-manifest.py` | 0 | `manifest OK: schema_version=1, 22 fixture(s)` |
| `python3 scripts/check-workflows.py` | 0 | required jobs 完整、retired jobs 硬禁用 |
| `python3 scripts/check-i18n.py` | 0 | `363 keys, 2 catalogs, … no hardcoded CJK … Rust/React` |
| `python3 scripts/test-ui-migration-guards.py` | 0 | **Ran 18 tests … OK**（新增迁移守卫） |
| `python3 scripts/test-linux-workflow.py` | 0 | 变异契约 |
| `python3 scripts/test-web-deploy-workflow.py` | 0 | 退役契约 |
| `python3 scripts/test-package-{linux,windows,macos}-release.py` | 0 | 退役打包契约 |
| `python3 scripts/test-fetch-fonts.py` | 0 | 字体脚本契约 |
| `python3 scripts/test-android-icon.py` | 0 | 图标契约 |
| `python3 scripts/test-check-android-apk-fonts.py` | 0 | APK 字体契约 |
| `python3 scripts/test-feature-reference-contract.py` | 0 | 特性参考契约 |
| `node --test scripts/test-web-pixels.mjs` | 0 | 1 passed（平台无关 PNG 契约） |

退役脚本**预期非零**（6/6，显式 NOT IMPLEMENTED，退出码 1）：`build-web`、`deploy-cloudflare-pages`、
`build-android`、`package-linux-release`、`package-windows-release`、`package-macos-release`
（`logs/retired-*.log`）。

## 3. Cargo 锁：初始失败 → 批准的最小恢复 → 复跑全绿

### 3.1 初始失败（保留，未抹除）

首次 sweep 中六条 `cargo … --locked` 均在解析阶段失败，`exit 101`：
`cannot update the lock file … because --locked was passed`（`logs/cargo-metadata-locked.log`、
`logs/clippy.log`、`logs/cargo-check-workspace.log`、`logs/cargo-check-wasm.log`、
`logs/cargo-test-workspace.log`、`logs/cargo-test-stubs.log`）。根因：手工剪枝的 `Cargo.lock` 与
manifests 不同步（仓库锁 622 包 vs cargo 正确 resolve 595 包）。

### 3.2 批准的最小离线同步（仅 `Cargo.lock`）

`cargo metadata --format-version 1 --offline`（**对真实仓库**，非 generate-lockfile）→ exit 0
（`lock-recovery/logs/metadata-sync.json`）。`Cargo.lock` 622 → **595** 包；sha256 `4ec63239…` →
`955603a2…`。差异（`lock-recovery/logs/lock.diff`，163 insertions / 3139 deletions）为**纯删除**：

- 删除 **27** 个孤儿包、**0** 新增；**无版本升级/降级**（同名多版本仅删去不再被依赖者：`ctor`→0.8.0、
  `dtor`→0.3.0、`drm` 全删、`linux-raw-sys`→0.12.1、`quick-xml`→0.36.2/0.42.0、`redox_syscall`→0.5.18、
  `rustix`→1.1.5 保留）。
- 保留 `(name,version)` 的 checksum/source **零变化**；`acadrust` 前置/后置均 **0.6.3**；根 `Cargo.toml`
  **无 `[patch]`**。其余差异为依赖列表版本限定符规范化（如 `foldhash 0.2.0`→`foldhash`）。
- **源码改动范围：仅 `Cargo.lock`**（`git diff --stat` 3302 行）。

### 3.3 六条复跑（全 `exit 0`；`lock-recovery/logs/`）

| 命令 | 退出 | 用时 |
|---|---|---|
| `cargo metadata --format-version 1 --locked` | 0 | 0s |
| `cargo clippy --workspace --exclude app-android --exclude app-web --all-targets --locked -- -D warnings` | 0 | 12s |
| `cargo check --workspace --all-targets --locked` | 0 | 7s |
| `cargo check --workspace --lib --target wasm32-unknown-unknown --locked` | 0 | 41s |
| `cargo test --workspace --exclude app-android --exclude app-web --locked -- --test-threads=1` | 0 | 154s |
| `cargo test -p app-android -p app-web --lib --locked` | 0 | 0s |

- **workspace 测试**：`1825 passed / 0 failed / 19 ignored`，76 个 test 二进制；无 panic/FAILED。
  `YACR_TEST_DWG=fixtures/dwg/synthetic-four-lines.dwg`；`--test-threads=1`；真实 X11/XWayland 环境，
  未钉软件 ICD。19 ignored 为显式 `#[ignore]`（需真实窗口/GPU/资料），**非失败**，不计入验收。
- **桩包**：`app-android`/`app-web` host lib 各 **0 测试**（源码无 `#[cfg(test)]` 用例），exit 0；
  **不代表**这些平台可运行。

## 4. 真实 GPU / 真实窗口（Quadro P620，硬件 Vulkan）

详见 `.slim/deepwork/replace-slint-ui/p6-real-gpu.md` 与 `p6-targeted-gpu.md`；产物 `gpu/`、`gpu-targeted/`。

- **环境证明**：`nvidia-smi` → `Quadro P620, 580.178.04, 4096 MiB`；`/proc/<pid>/fd` →
  `/dev/nvidia0`、`/dev/nvidiactl`、`/dev/nvidia-modeset`、`/dev/dri/renderD128,D129`；
  `/proc/<pid>/maps` 映射 `libnvidia-glcore…`；presenter `backend=vulkan adapter=Quadro P620
  device_type=discrete_gpu present_mode=Immediate`；`nvidia-smi --query-compute-apps` 映射到 `yacr`。
- **Open + 渲染 smoke（合成夹具）**：`--open fixtures/dwg/synthetic-four-lines.dwg`（`entityCount: 4`）；
  `frames` 递增，所有 presented 帧 `complete=current_scene`（`base_only=0`、`error=Some=0`）。
  合成输入（`YACR_PROBE_EVIDENCE=1`）产生**真实相机/视口增量**（非真实鼠标）。
- **原生分区生命周期**：顶层 XID `0x600003`，原生 GTK CAD `GtkDrawingArea` **child** surface
  `0x600032`（与顶层不同）；`forced overlay unrealize` 后重建为**新 XID `0x600061` + generation 2**，
  帧恢复 `current_scene`。`YACR_PROBE_LIFECYCLE=unrealize|full|exit` 三次 **自然退出 exit 0**。
- **定向外部 WM 复检**（`gpu-targeted/`）：
  - **minimize → 外部 WM restore**：`XIconifyWindow` → host `paused(minimized=true)`，frames 冻结在 57；
    外部恢复（`WM_STATE=Normal`、`_NET_WM_STATE_HIDDEN` 清除、`focused=true`）→ **`resumed`**，
    frames **57 → 963**，全部 `current_scene`，`error=None`。客户端 `map_state` 全程 `Viewable`，
    **故不声称“unmapped”**。
  - **真实 resize**：`YACR_PROBE_NO_MAXIMIZE=1` 下直接 `XMoveResizeWindow` 达 **1200x800** 与 **900x600**
    两个实际几何，传播到 DOM rect（`dom=1166x513` / `866x313`）、原生 CAD surface 与 presented 帧
    （`626→926→1166→866`），`current_scene=505 base_only=0 error=0`。EWMH `_NET_MOVERESIZE_WINDOW` 在本
    WM 为 **no-op**；`native_surface` 相对 geometry 行**滞后一次 revalidation**（分阶段重排）。
- **保留的初始失败 / 未解决项（未抹除）**：首轮 `evidence` 抓图因子进程缺 `DISPLAY` 失败（**harness 抓图
  失败，非 app 失败**，修复后以 `evidence-r2/r3` 新材料重跑）；初始 `lifecycle-full` 的应用侧
  `unminimize()` + 强制 unrealize 路径未产生恢复。**该序列仍未解决**——它**既不证明宿主恢复缺陷，也不
  确立“仅 WM 原因”**。已证明的仅限：**定向外部 WM restore 路径成功恢复渲染**（无更宽的“无缺陷”结论）。

## 5. 视觉 smoke（observer；已终判）

- **首轮观察**：可读的本地化 chrome（标题/QAT、功能区选项卡 文件/常用/绘图/测量/批注/视图、面板、
  状态栏、语言 `zh-CN`）+ **非空 CAD 分区**，无显著遮挡/报错。
- **后续（终判）**：post-restore 与 1200x800/900x600 的**未增强**截图中，chrome 与 CAD 均清楚可读、
  原生分区**适配**、无重大重叠或裁剪；**仅在 900x600 时画布几何触及边界**；**无重大视觉缺陷**。
- **不据此推断**：无 golden/参考图、无像素精度/性能/GPU 结论；**网格与彩色坐标轴未被证明**。
- 截图（绝对路径，未增强为证）：`gpu-targeted/case1-minrestore/shot-post-final-top.png`、
  `.../case2-resize/shot-A-1200x800-top.png`、`.../shot-B-900x600-top.png` 及对应 `-cad` 子图；
  增强副本（`-bright`，标注 enhanced）仅供可读性复核。黑帧
  `gpu/lifecycle-full/shot-t30-top.png` 为**最小化状态**（非崩溃），仅文档。

## 6. 支持范围与限制（诚实）

- 真实支持的 GUI 操作：**OPEN / FIT + 原生导航**（缩放/平移）；其余控件显式禁用。**合成夹具**，**非**
  vendor DWG 兼容证据。
- **未运行/未声称**：真实鼠标精确光标锚定像素验收（仅合成 DOM 输入/间接）；真实 portal picker（需交互）；
  golden/精确视觉；性能预算；设备丢失/重建压力；Windows/macOS/Android/Web 运行；保存/编辑/多平台产品能力。
- **不等于**已退役的 Slint 7 用例真实 GPU 套件结果；旧 Slint 证据不继承。
- 本次 GPU 验收**未回填** CI（本环境无 GitHub runner）。
- **验收范围**：Gate P6 attempt2 **ACCEPT-WITH-FIXES**（文档限定已应用）；**有界本地 Linux 迁移验收通过**，
  但**不**构成完整 CAD 产品/跨平台/vendor/golden/性能验收。应用侧 `unminimize()`+强制 unrealize 序列**未解决**；
  真实 portal picker、真实鼠标像素精度、vendor/golden/性能/设备丢失/打包/非 Linux 仍**未支持/未验证**（见 §6）。

## 7. Ready for GPU 复跑：是

锁已最小同步（仅 `Cargo.lock`，纯删 27 孤儿、无版本变动），六条 `--locked` 命令全绿；GPU 定向复检已终判。
