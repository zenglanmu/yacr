# AGENTS.md

Rust CAD 查看、测量与批注系统，**开发搭建中**：契约框架 + 合成测试，没有可交付产品。
需求唯一权威是 `CAD_IMPLEMENTATION_SPEC.md`（v2.0，含 §21 桌面宿主增补）；`docs/handoff.md`
是逐轮交接入口；桌面 UI 决策见 `docs/adr/0010-react-linux-host-replaces-slint.md`。

> **2026-10-23 合成状态**：桌面 UI 是 **React + Tauri 原生宿主**（`apps/app-linux` 二进制 `yacr`、
> `apps/app-windows` `yacr-windows`），共享 React UI 在 `apps/desktop-ui`，Tauri 命令薄壳在
> `cad-desktop-shell`；`cad-ui-slint` 与所有 Slint 依赖已删除。
> **生产合成方向是 DOM/WebView 在上、wgpu CAD 原生表面在下**（[ADR 0023](docs/adr/0023-dom-on-top-cross-host-composition.md)）。
> Linux **原生 Wayland** 是唯一生产路径；**X11/XWayland 显式拒绝**（标记行 + 退出码 4）。
> **Windows 宿主已实现但视觉/运行时未验收**（上限 = Tier 0 编译 + Wine 拒绝合同；
> 真 Windows spike-1/4/5 PENDING 用户自测）。macOS/Android/Web 是**显式 NOT IMPLEMENTED 占位**，
> 不是可运行 GUI 产品。

## 铁律（违反即返工）

1. 不修改 acadrust 源码，不使用 Cargo patch（`check-architecture.py` 会拒绝）。
2. 未支持/未验证能力必须显式建模（`Unsupported`/`Partial`/诊断），占位与假数据不计完成，
   也**禁止**把失败改成空成功。
3. 业务层只操作数据库/命令/事务；GPU 提交封装在渲染器内。
4. 增量更新：ChangeSet → 依赖失效 → 显示表示重建，不重解析底图。
5. 每项改动同步补契约测试与文档。
6. 证据诚实：未运行就说未运行。README/handoff 中大量“已通过”注明仅为**合成测试**或
   特定环境（模拟器/软件 GPU/无头浏览器），不得推广为真机、真实 GPU、vendor 兼容结论。
   历史 Slint 宿主的证据不得冒充 React 宿主结论。**Windows 的 Tier 0 编译与 Wine 拒绝 smoke
   不是真 Windows 验收**；Wine 像素结论必须写 `NOT VERIFIED (Wine limitation, bug 55432/58921)`。
   Linux 合成结论只对 **GNOME/Mutter + P620 + `scale=1`** 成立；UI “DOM 测试”是源码结构/纯函数
   契约，不是运行时挂载或像素证据。
7. 不覆盖/删除陌生变更（并发实现常见）；先读源码和最新构建证据。
8. **DOM-canvas 宿主（`surfaceMode: dom-canvas`）**：wasm 侧 CAD 会话是**唯一 `cad-db` 权威**，
   native 壳**不得**实例化/镜像第二份文档状态，也**不得**把 native 显示表示当 wasm 数据源；
   能力缺失必须**显式拒绝**（标记行 `yacr: unsupported-runtime: webgl2` + i18n 诊断 + **退出码 6**），
   **禁止**空白 canvas、静默回退到 X11 native-child、静默软件渲染冒充硬件路径。
   触发条件与判据：只要某项改动让 `dom-canvas` 宿主上出现第二个 CAD 会话实例（或让 native 持有
   文档状态），或让 WebGL2 不可用变成空白窗口/静默回退，即返工。目标态与分阶段状态见
   `docs/adr/0024-dom-canvas-wasm-cad-path.md`（**代码未开始**）。

## 语言约定

代码标识、注释、测试名、commit message 用英文；`docs/`、`AGENTS.md` 用中文。
用户可见文案只能来自**唯一权威目录** `crates/cad-ui-core/i18n/{zh-CN,en}.json`（键集必须一致，
见 `scripts/check-i18n.py`），不得在 React JSX/JS 或 Rust UI 里硬编码中文。
**该门禁的实际范围（2026-10-20 更正，勿按过度承诺理解）**：`check-i18n.py` 只扫
`crates/cad-ui-core/src`、`cad-desktop-shell/src`、`apps/app-linux/src`、`apps/app-windows/src`（Rust）与
`apps/desktop-ui/src`（JS/JSX）并拒绝其中的**硬编码 CJK**；
**`crates/cad-app` 仍含用户可见中文字面量**（如 `measure_tool.rs` 的提示文案），它们由
`crates/cad-app/src/tests/cjk_baseline.rs` **冻结**——只**禁止增长**，不是"已合规"。这是**已知债务**，
新代码**不得**沿用该做法。

**提交纪律（已发生两次，务必照做）**：后台 specialist lane 仍在写工作树时**禁止** `git add -A` / `git add .`；提交前必须 ①`git status --porcelain` **逐项**核对都属于本轮改动，②按**显式路径**暂存，③门禁命令**不得**用 `| tail -1` 之类的管道吞掉退出码（用 `${PIPESTATUS[0]}` 或 `set -o pipefail` 显式检查）。**已发生**：`git add -A` 两次扫入他人/lane 半成品；`clippy | tail -1` 一次把失败吞成"通过"。
**机械规则（已发生三次，务必照做）**：在 `crates/cad-app/src/selection.rs` 新增 `PropertyRow::new("<key>")` 时，**必须同时**补两个 i18n 目录的 `properties.row.<key>` **与** `apps/desktop-ui/src/cad.mjs` 的 `propertyRowLabel` switch 分支；`apps/desktop-ui/test/properties.test.mjs` 会扫源码字面量并要求两处齐全，否则失败。

## 仓库结构

Cargo workspace（`crates/cad-*` + `apps/app-linux`、`apps/app-android`、`apps/app-web`、
`apps/app-windows`、`apps/app-macos`）。分层示意，**不可反转**；实际依赖以 manifests 与
`scripts/check-architecture.py` 为准：

```
cad-domain
  ↑
cad-db ── cad-dependencies ── cad-history
  ↑
cad-geometry ─ cad-kernel-adapter ─ cad-representation
  ↑
cad-spatial ─ cad-scene ─ cad-render-wgpu
  ↑
cad-proxy ─ cad-import-acadrust ─ cad-resources
  ↑
cad-measure ─ cad-query ─ cad-app
  ↑
cad-engine ─ cad-platform ─ cad-diagnostics ─ cad-cli-tools
  ↑
cad-host（HostCore：命令路由、打开/保存/出图管线、状态推送、导航/绘制 sink；平台无关）
  ↑
cad-ui-core（protocol/layout/session/shell/popup/i18n；UI 框架与 GPU 无关）
  ↑                              ↑
cad-desktop-shell（唯一允许依赖   apps/desktop-ui（唯一桌面 React UI 包：单 React 树、
tauri 的共享层：命令注册、窗口    单一输入漏斗 `.native-input`、in-DOM 弹窗/菜单；
构建、threading、证据骨架）        构建产物被两个宿主嵌入）
  ↑                              ↑
apps/app-linux / apps/app-windows（Tauri 宿主：React chrome + 原生 CAD 分区 + renderer-only wgpu）
apps/app-macos|app-android|app-web（显式 NOT IMPLEMENTED 占位）
```

- `cad-host` 是平台无关的宿主编排层，**不依赖任何 UI 工具包**（不依赖 slint/tauri/gtk/wgpu/
  web-sys/Android 类型）；平台 I/O 一律走 `cad-platform` 契约（事件式、无 futures、无 `Send`）。
- **`cad-ui-core`** 承载平台无关的 UI 逻辑（wire protocol、逻辑布局、CAD 会话、宿主 admission/
  error 状态、popup 纯状态机、launch options、i18n 目录），位于 `cad-host` 之上、具体宿主之下。
  它依赖 `cad-host`（会话包装 `HostCore`），必须保持 UI 工具包/GPU 无关。宿主通过显式
  `state.shell.*` 访问共享状态、直接 `use cad_ui_core::…`，**不保留**过渡 re-export 别名。
  React 前端直接 import `cad-ui-core/i18n`（唯一文案目录）。
- **`cad-desktop-shell`** 是**唯一允许依赖 `tauri`** 的共享层：`#[tauri::command]` 薄壳与注册、
  主窗口构建/init scripts/`start_maximized`、popup 命令到 `cad-ui-core::popup` 的接线、
  工具包无关 `threading`、证据驱动骨架。**禁止** gtk/gdk/gdkx11/ashpd/wgpu/cad-render-wgpu/
  cad-engine/slint/iced/acadrust（脚本强制）；非 Linux/Windows 目标编译为空壳（保 wasm 门禁）。
- **`apps/desktop-ui`** 是**唯一桌面 React UI 包**：单 React 树、`.native-input` 单一漏斗、
  in-DOM 弹窗/菜单；两个宿主的 `tauri.conf.json` 都指向 `../desktop-ui/dist`。
  不得在 JSX/JS 里硬编码用户可见文案。
- `cad-engine` 是 UI 无关的渲染会话协调库（`cad-app`/`cad-db`/`cad-domain`/`cad-render-wgpu`/
  `cad-representation`）；`cad-app` 不依赖 `cad-engine`，`cad-engine` 不依赖 `cad-ui-core`/
  `cad-host`/任何 UI 工具包或直接依赖 wgpu。
- **合成方向（唯一）**：DOM/WebView 在上、CAD 原生表面在下且**不接收 OS 输入**。
  - **Wayland**：app-owned `wl_subsurface` **无条件** `place_below(parent)`（`apps/app-linux/native/`
    私有 C shim；`build.rs` 经 `cc`+`pkg-config wayland-client`，需 `libwayland-dev`）；
    GTK toplevel 必须透明（空 opaque region + `Operator::Source` 清屏），否则 CAD 不可见。
  - **Windows**：`WS_CHILD | WS_VISIBLE | WS_DISABLED | WS_CLIPSIBLINGS` 的 CAD child HWND 压在
    wry 的 `WRY_WEBVIEW` 容器紧邻之下（`ensure_order` 幂等重排）；WebView2 背景 alpha 必须恰为 0。
    **不使用** `WM_NCHITTEST`/`HTTRANSPARENT`，**不声称**原生点击穿透。
  - **输入**：唯一漏斗 `.native-input` → `probe_input` IPC → host 准入门/队列；半透明装饰面板
    `pointer-events: none` 穿透、面板内交互控件 `pointer-events: auto` 收回。
  - **弹窗**：锚定菜单与居中模态都是同一 React 树的 in-DOM 覆盖层；第二 Tauri 窗口机制已退役；
    host 侧 `ShellState.popup` 模态围栏保留，清除为四触发（close / reload / renderer crash / exit）。
  - 业务层/渲染器不因此改变：单 presenter、renderer-only wgpu、无整幅回读。
- **`apps/app-linux`** 是真实 Linux GUI（Tauri 2 WebView 中的 React chrome + GTK 中央原生 CAD
  分区 + renderer-only wgpu），支持**原生 Wayland**（native-below 生产路径）与 **X11/XWayland**
  （**DOM-canvas 载体**，ADR 0024）：后端由 `backend.rs` 在 GTK 启动前决议 `GDK_BACKEND` 与
  `surfaceMode`；X11 上 wasm/WebGL2 CAD 会话画进 WebView 内 `<canvas>`，native 壳不建 CAD 会话；
  WebGL2 不可用时输出稳定标记行 `yacr: unsupported-runtime: webgl2` + i18n 诊断 + **退出码 6**，
  **无静默回退**（`yacr: unsupported-backend: x11` + 退出码 4 为**历史**合同）。非 Linux 目标上
  它编译为空库、二进制返回显式 NOT IMPLEMENTED。
- **`apps/app-windows`** 是真实 Windows 宿主（Tauri 2 + 透明 WebView2 + Win32 child HWND +
  单一 DOM 漏斗），**已实现但视觉/运行时未验收**：WebView2 缺失 → 标记行
  `yacr: unsupported-runtime: webview2` + i18n + **退出码 5**。`app-macos`/`app-android`/`app-web`
  是显式 NOT IMPLEMENTED 占位。平台 I/O（文件、配置、表面度量）只存在于 `app-*`。
- **桌面窗口策略**：Linux 宿主默认以**最大化窗口**启动（`apps/app-linux/src/host.rs` 的
  `start_maximized` 默认 `true`，`WindowBuilder::maximized(true)`），**保留标题栏/系统面板**——
  这是**窗口管理器最大化**，**不是**全屏/无边框 kiosk（源码未调用 `fullscreen`/`decorations(false)`）。
  仅验收/证据运行用 opt-out 环境变量 `YACR_PROBE_NO_MAXIMIZE`（**其存在**即退出最大化）；
  `tauri.conf.json` 无窗口配置（`"windows": []`），不覆盖此默认。跨窗口管理器/平台的行为不保证
  （见下方「构建与门禁」的 NOT VERIFIED 边界）。

`check-architecture.py` 强制的边界：`cad-domain` 不依赖任何 CAD 包；`cad-db` 只依赖
`cad-domain`；`cad-import-acadrust` 是**唯一**依赖 acadrust 的 crate；`cad-render-wgpu`
是唯一依赖 wgpu 的 crate；`cad-ui-core` 不得依赖 slint/wgpu/tauri/gtk/portal/平台类型；
**`cad-desktop-shell` 是唯一允许依赖 `tauri` 的 crate**，且不得依赖 gtk/gdk/gdkx11/ashpd/wgpu/
cad-render-wgpu/cad-engine/slint/iced/acadrust；`apps/desktop-ui` 不是 Cargo 成员（静态资源包）；
**全 workspace 禁止任何直接 `slint`/`slint-build`/`iced` 依赖**（含 build/dev 与 target 表、
workspace 继承别名）；平台类型（`web-sys`/`android-activity`/`jni`/`ndk`）不得进入 `cad-*`。
`cad-db` 是唯一权威数据源；UI 与渲染均为派生。详见 `docs/architecture.md`。

## 构建与门禁

工具链 1.99.0（`rust-toolchain.toml`）；若 `PATH` 无 cargo，加 `$HOME/.cargo/bin`。
**验证范围（2026-10-15 用户决定，提效）**：日常功能开发与修复**只验证自己的改动范围**
（改动 crate 的编译、自己的契约测试、自己文件的 fmt/clippy）。不要为小改动跑全量测试、
全量门禁或平台编译矩阵。全量测试与全量门禁（下方命令块，CI 同款，见 `docs/ci.md`）只在
两种情形运行：① 用户明确要求；② 发版/发布验收前。证据诚实规则不变：未运行必须标注
NOT RUN，不得以旧证据冒充。CI workflow 仍按自身节奏运行，不受此本地策略影响。

**这**是日常提效策略，**不是永久免测**：Slint→React 迁移的**最终验证**（P6）**已执行（有界、分区）**，
证据见 `docs/validation-react-linux-migration-2026-10-16.md`（编译/测试/门禁/硬件 GPU 完成；解析器
一度被手工锁阻断，已按批准最小离线同步修复，仅改 `Cargo.lock`）。**Gate P6 attempt2：ACCEPT-WITH-FIXES
（文档限定已应用）**，observer 终判已观察截图**无重大视觉缺陷**；**有界本地 Linux 迁移验收通过**，
但**不是**完整 CAD 产品/跨平台/vendor/golden/性能验收。
**DOM-on-top（WP1–WP5）沿用同一策略**：逐 WP 验证改动 crate 的编译/测试/门禁，**全 workspace 全量
门禁 NOT RUN**；证据与 NOT RUN 边界见 `docs/validation-dom-on-top-production-2026-10-23.md`。

```bash
cargo fmt --all -- --check
cargo clippy --workspace --exclude app-android --exclude app-web --all-targets --locked -- -D warnings
python3 scripts/check-architecture.py
python3 scripts/check-fixture-manifest.py
python3 scripts/check-workflows.py
python3 scripts/check-i18n.py
python3 scripts/check-file-size.py   # 代码文件 ≤1000 行（.rs/.js/.jsx/.mjs/.cjs/.ts/.tsx/.py；文档不计入）
cargo check --workspace --all-targets --locked
cargo check --workspace --lib --target wasm32-unknown-unknown --locked   # 全 workspace，含 UI/宿主
(cd apps/desktop-ui && npm ci && npm test && npm run build)
```

**代码文件行数上限**：任何**代码**文件不得超过 **1000 行**（`scripts/check-file-size.py`，含
`scripts/test-check-file-size.py` 自测）。`.md` **不计入**（`docs/handoff.md` 等按设计就是长日志/清单）。
文件接近上限时按模块拆分，**不要**加白名单豁免。

默认 Linux host workspace 编译包含 `app-android` 与 `app-web` 包的 host target；这不替代
下方独立的 WASM 与 Android 原生目标编译门禁，也不代表这些平台的运行验收。clippy 仍按
既有配置排除 `app-android` 与 `app-web`。

- **平台状态（2026-10-23）**：**Windows 宿主已实现**（Tauri 2 + 透明 WebView2 + Win32 child HWND +
  共享 `apps/desktop-ui`），但**视觉/运行时未验收**；CI `windows-check` 只做**编译门禁**
  （msvc/gnu/wasm 空壳），**不代表**可运行 GUI 或运行验收。`windows-release` 仍 `if: false`，
  `scripts/package-windows-release.sh` 仍退役非零。**macOS/Android/Web** 是显式 NOT IMPLEMENTED
  占位（Android/Web 仅保留编译门禁）。旧平台发布/打包脚本已退役或禁用；Linux release 暂时不可用，
  等待替代打包验收。**不要**把已退役脚本当作可用流程。
  **2026-10-23 立项 / 2026-10-24 第一批实现（[ADR 0024](docs/adr/0024-dom-canvas-wasm-cad-path.md)）**：
  X11/XWayland 走 **DOM-canvas 路径**（wasm/WebGL2 CAD 会话画进 DOM `<canvas>`，
  `surfaceMode: dom-canvas`），`apps/app-web` 的 wasm 会话共享**同一**路径；WebGL2 缺失 →
  标记行 `yacr: unsupported-runtime: webgl2` + **退出码 6**（已实现）。**已取得的证据**
  （`docs/validation-dom-canvas-wasm-2026-10-24.md`）：X11 宿主 `surfaceMode=dom-canvas` 启动、
  canvas 挂载 + backing `626×341`、合成 fixture 出图（ScreenCast 抓帧）；**X11 上加/缩放平移经用户
  手动确认**（用户报告，无 agent 像素差证据）；Chromium 141 与 WebKitGTK 2.52.6 本地出图与缩放。
  **仍禁止**写成「X11 已验收」或「web 已发布」：真 X11 服务器/WM、其它合成器、**硬件归属
  （NOT VERIFIED）**、DPR/多显示器、性能、vendor/golden、浏览器矩阵与 CI/发布均未验证；agent 侧
  输入注入在本机不可用。**阶段 ⑧ 首批已实现**（[ADR 0025](docs/adr/0025-wasm-memory-budget-and-worker-isolation.md)）：
  显式内存预算拒绝（`step:"budget"`，64 MiB 输入 / 3 GiB 堆）、分块但原子提交的上传、以及**按能力**
  的 Worker 隔离（Chromium 实测 `isolation=worker`；WebKitGTK 实测 Worker 内无 WebGL2，显式报
  `main-thread (no WebGL2 in worker)`）。`complex-test.dwg` 实测 heap **1.91 GB → 1.43 GB**、
  开图 **14.1 s → 11.2 s**；**峰值仍由导入/建场景阶段决定**，大图预算仍未达标。
  **GPU 侧同样有预算**（[ADR 0026](docs/adr/0026-gpu-memory-accounting-and-budget.md)）：渲染器按类目
  记账（current/peak，drop guard 精确退费），默认 **3 GiB** 应用预算（**估算，不是 VRAM 总量**；
  wgpu 无法查询），`--gpu-budget-mb` > `YACR_GPU_BUDGET_MB` > 默认；拒绝为
  `RenderError::GpuMemory` + CLI **退出码 7** + 不写 PNG。实测授权样本常驻 **2.11 GB**
  （驱动 allocator 交叉校验差 **0.07 %**），其中 **`stream` 1.70 GB** 为最大头（渲染器优化另立项）。
  `memory_budget_thresholds` **只在 D3D12** 启用：Vulkan 上实测会让同进程第二个 device 首次 72 B
  写入即 OOM，**不得**顺手打开。
- **渲染/端到端验证优先真实硬件 GPU**：只要环境存在真实显示与显卡（本机为 Wayland 桌面 +
  Intel UHD + Quadro P620，可用 `nvidia-smi`/DRM fdinfo 证明），验收**必须**走真实 GPU。
  当前桌面宿主（React/Tauri）的**唯一生产后端是原生 Wayland**（决策/子表面桥见
  `docs/adr/0011-native-wayland-subsurface.md` 与 [ADR 0023](docs/adr/0023-dom-on-top-cross-host-composition.md)；
  有界原生证据见 `docs/validation-native-wayland-2026-10-16.md` 与
  `docs/validation-dom-on-top-production-2026-10-23.md`）。**X11/XWayland 自 2026-10-24 起走
  DOM-canvas**（[ADR 0024](docs/adr/0024-dom-canvas-wasm-cad-path.md)）：有界运行成立（合成 fixture
  出图 + 用户手动确认缩放平移），但它**不是** native-below 的生产证据，且硬件归属、真 X11、性能均
  **NOT VERIFIED**；历史拒绝合同（`yacr: unsupported-backend: x11` + 退出码 4）**不得**与当前
  `webgl2` 码 6 混用。
  真实 GPU 路线是 env-gated 证据驱动（`apps/app-linux/src/evidence.rs`：
  `YACR_PROBE_EVIDENCE`/`YACR_PROBE_LIFECYCLE`）+ `scripts/collect-evidence.sh`
  （原生 Wayland 用同 UID Mutter `ScreenCast` 抓合成帧）。W3 曾执行有界原生运行（CAD/输入/生命周期/
  几何）并由 Observer 终态判定**有界截图视觉 PASS**；**最终 Oracle W3 attempt3 终判
  ACCEPT-WITH-LIMITATIONS（无 blocker）**：有界本地验收（GNOME/Mutter + P620 + `scale=1`），
  不得据此宣称完整原生支持或 vendor/精度/性能结论。该 W3 的 X11 运行回归属 **CAD-on-top 时期**；
  自 DOM-on-top 起 X11 被拒绝，`apps/app-linux/scripts/x11-window-grab.c` 是**历史工具**，
  **不得**冒充当前后端证据。
  **旧 Slint 离屏/无头与真实 GPU 测试套件（`scripts/check-linux-app.sh`、`verify-ui.sh`、
  `run-real-gpu-e2e.sh`）已随 Slint 退役，不再存在**。P6 已定义并**实际运行**替代的真实 GPU 程序
  （硬件 Vulkan + 真实 GTK/XWayland 窗口 + env-gated 证据驱动；证据见
  `docs/validation-react-linux-migration-2026-10-16.md` 与 `.slim/deepwork/replace-slint-ui/p6-real-gpu.md`）；
  旧套件结果不得复用，也不得用软件后端冒充真实 GPU。
  - **不得**用 SwiftShader/lavapipe 等软件后端产出"真实 GPU"结论；软件后端产出的数字/截图必须
    显式标注为软件渲染。只有**确无显示/GPU** 的环境才回退软件渲染。
  - **性能/CPU 测量同样适用真实 GPU 优先**；软件后端耗时不得用作性能结论。
- Rust 单测试：`cargo test -p <crate> <name> --locked`。GPU/CLI 用例串行跑。
- 需要真实 DWG 的测试设 `YACR_TEST_DWG=<绝对路径>`，否则跳过；跳过不是已运行证据。
- 离线 WGSL 校验（无需 GPU）：`cargo test -p cad-render-wgpu --test wgsl_validation --locked`。

## 平台状态：占位与“已实现但未验收”

- **Windows**（`apps/app-windows`）：**真实宿主，已实现**——Tauri 2 + 透明 WebView2 + Win32
  child HWND + 单一 DOM 输入漏斗；共享 UI 来自 `apps/desktop-ui`。**验收上限**：Tier 0 编译门禁
  （msvc/gnu/wasm 空壳）+ Wine **拒绝合同** smoke（WebView2 缺失 → 标记行 + 退出码 5）。
  视觉/运行时（spike-1/4/5、z-order、DPI、ProcessFailed、picker、runtime-present 出帧）**NOT RUN**。
  **禁止**写“Windows 已验收”；Wine 像素结论必须写
  `NOT VERIFIED (Wine limitation, bug 55432/58921)`。详见 `docs/windows-app.md`。
- **macOS**（`apps/app-macos`）：显式 NOT IMPLEMENTED 失败桩（`yacr-macos`），只随
  `--all-targets` 编译；不打可分发 GUI。
- **Android**（`apps/app-android`）：UI 已退役，是显式占位（原生入口打印退役诊断后 abort）。
  仅保留 `cargo check --target aarch64-linux-android -p app-android --locked` 编译检查。
  `build-android.sh` 非零失败；不再有 APK 流水线。**模拟器 ≠ 真机**，真机一律未验证。
- **Web**（`apps/app-web`）：UI 已退役，wasm 入口返回错误；`build-web.sh`/`deploy-cloudflare-pages.sh`
  非零失败。不再有 web 发布。

## 真实 DWG 回归

流程见 `docs/testing-dwg.md` 与 `docs/react-linux-app.md`：**有真实显示/GPU 时以真实硬件 GPU
为主**（React/Tauri 宿主经 `scripts/run-react-linux.sh` 启动，env-gated 证据驱动 +
硬件 Vulkan）；**无显示环境**才回退原生离屏 wgpu + Mesa lavapipe
（`cargo build -p cad-cli-tools --release`，`scripts/check-dwg-native.py`，用 lavapipe 时须显式
`--software`）。必须分别记录「打开 / 出图 smoke / 视觉验收」三个结论；退出码 0、非空截图、
`error=None` 都不能单独证明视觉正确。外部大样本默认放仓库外（默认 `~/sources/cad-test-files/`），
也可用 `scripts/fetch-test-dwg.sh` 拉到 `/tmp/opencode`。**授权明确、可再分发的图纸与参考图
可提交进 `fixtures/` 并记入 `fixtures/manifest`**，须在 `provenance` 记录来源 URL、下载
日期、许可与 SHA-256；未授权或来源不明的用户/厂商数据仍不得提交。每轮用新输出目录，
保留失败/超时记录，不用旧证据冒充本轮通过。

**自动证据采集（一键：构建 + 启动 + 截图 + 汇总）**：`scripts/collect-evidence.sh` 串起
「前端构建 → `cargo build --release` 宿主 → 按后端启动真实 `yacr`（`--open` + `--fonts-dir`，
**不用** `--probe` 假打开）→ 截图 → 汇总日志」。原生 Wayland 后端用 `scripts/screencast-capture.py`
经同 UID `org.gnome.Mutter.ScreenCast` 抓合成帧；**X11 自 2026-10-24 起是真实运行**
（`surfaceMode=dom-canvas`），同样用 ScreenCast 抓 `x11/x11-frame.png`；**拒绝合同**改为
`webgl2` 能力缺失：稳定标记行 `yacr: unsupported-runtime: webgl2` + **退出码 6** → `UNSUPPORTED_RUNTIME`
（标记行第一依据，退出码辅助）。历史 `UNSUPPORTED_BACKEND`（`yacr: unsupported-backend: x11` +
退出码 4）解析保留，仅用于旧构建。X11 抓屏**不得**用 `ffmpeg -f x11grab`/`gnome-screenshot`
（本会话在 Wayland 合成下全黑）；`apps/app-linux/scripts/x11-window-grab.c` 仍是**历史工具**，
不得冒充证据。输出落
`${YACR_EVIDENCE_ROOT:-/tmp/opencode}/yacr-evidence-<UTC>Z/`：`results.tsv` 逐后端记录
`NATURAL_EXIT`/`PLANNED_TIMEOUT`/`SKIPPED`/`NO_DISPLAY`/`UNSUPPORTED_BACKEND`、真实退出码、
截图 `captured`/`missing`；`README.md` 附授权边界。契约测试
`python3 scripts/test-collect-evidence.py`。**DOM-on-top 回归夹具**只在真实 `--open` 会话 +
`YACR_PROBE_STACKING=1`（严格 `"1"`）+ init-script `window.__YACR_STACKING_REGRESSION__ === true`
时启用；`--probe`/`YACR_PROBE_ONLY` 同现拒绝；夹具不是产品开关，其帧**不是**视觉 PASS。
**证据取景规则**：必须包含可见几何、捕获时确认目标窗口在前台（否则得到 wrong-window 帧）、
夹具层序 ≠ 生产透明度、每轮新目录；**生产透明修复后的像素复采已执行（有界 PASS：几何满亮度、
无蒙层；About 覆盖层 PASS）**，但 textEdit 编辑器内容与光标抑制确定性仍未闭环（见验证记录）。
**授权边界（2026-10-23 用户授权放宽）**：允许 `sudo`/以 root 运行、允许解锁或切换会话、允许安装
软件（浏览器 / Playwright / 测试与构建工具）、允许在**真实用户会话**中运行有显示的程序（含
X11/XWayland 实例下的 DOM-canvas 探针）。**仍然禁止**：① 起虚拟/无头显示（Xvfb、虚拟合成器等）并把
其产物冒充真机/真实会话证据；② 用软件渲染（llvmpipe/SwiftShader/lavapipe 等）冒充真实 GPU——
软件渲染产物必须显式标注，且不得用作性能结论；③ 把无头或 **XWayland** 结论推广到真机、真 X11、
其它窗口管理器/合成器、其它 vendor 或跨平台。无真实显示时仍**显式非零退出**而非假装成功；
截图仅本地保存、不上传。Wayland 走用户**自身会话内**的 ScreenCast（有录制指示、不抑制），
**不是**绕过授权；**Wayland 帧为整屏合成**（同 UID `RecordMonitor`），**会包含桌面上其它窗口**——
仅供本地读判、不上传。截图只是窗口/合成像素，**不等于**视觉正确性、精度或 vendor 结论。
> **历史（2026-10-23 之前的边界，保留不删）**：以当前会话同 UID 用户运行，**不提权、不以 root
> 运行、不 `sudo`、不解锁/修改会话、不改 portal/ScreenCast 权限、不装包、不起虚拟/无头显示**。
> 该限制**只描述旧授权**；`scripts/collect-evidence.sh` 内嵌的授权说明与 `docs/handoff.md` 的
> 历史条目仍按旧边界书写，属**同步债务**（脚本不在本轮改动范围），阅读时以本段为准。

## 交接点

用 `rg 'pending\(' crates apps` 查找显式占位（文件路径与标识对应实施单元）。替换 `pending`
必须补验收测试，禁止改成空成功。已定义但仍需设计审查或未闭环的事项集中在
`docs/handoff.md`「已定义，但尚需设计审查」与各 `docs/*.md` 的“未完成”小节。
**注意**：ADR 0023 §5 **显式延期**「平台合成 ADR（RenderTarget/HostTexture token 所有权）」——
它属 renderer 资源契约，与层序正交；handoff 中该条目**原样保留**，不得当作已被 DOM-on-top ADR 覆盖。

## 文档索引

`docs/architecture.md`（边界与不变量）、`docs/react-linux-app.md`（当前 Linux 宿主与 DOM-on-top）、
`docs/windows-app.md`（Windows 宿主与 spike 判据）、`docs/build.md`（构建与退役入口）、
`docs/ci.md`（CI 分层与 NOT RUN）、`docs/validation*.md`（历史执行证据，含 NOT RUN 边界；
DOM-on-top 生产记录 = `validation-dom-on-top-production-2026-10-23.md`）、
`docs/compatibility.md`（能力表）、`docs/handoff.md`（逐轮交接）、
`docs/adr/`（含 ADR 0010、0017–0025；**0023 = DOM-on-top 跨宿主合成**、
**0024 = DOM-canvas / wasm CAD 路径（已实现首批）**、**0025 = wasm 内存预算/分块上传/Worker 隔离**、
0021 = 弹窗层（其第二窗口机制已被 0023 取代）、0022 = WIPEOUT 遮罩渲染）、
`docs/testing-dwg.md`、`docs/app-icon.md`、`fixtures/manifest`。
