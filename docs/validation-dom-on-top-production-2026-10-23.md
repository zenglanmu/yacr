# DOM-on-top 生产合成验证记录（2026-10-23）

## 结论与范围

**生产合成方向已反转为 DOM 在 wgpu CAD 原生表面之上**：Linux 原生 Wayland 是唯一生产路径，
并有**有界** Observer 判读；X11/XWayland 被**显式拒绝**（退出码 4）；Windows 宿主**完整实现**
并通过 **Tier 0 编译门禁 + Wine 拒绝合同 smoke**，视觉/运行时验收 **PENDING 用户真 Windows
spike-1/4/5**。

| 路径 | 本轮结论 | 边界（必须同时引用） |
| --- | --- | --- |
| Linux 原生 Wayland（生产） | **有界 PASS（带 caveat）**：X11 拒绝合同、生产帧层序、回归夹具 4/4 帧、WP3 About 帧由 Observer 判读 | 仅 **GNOME/Mutter + Quadro P620/Vulkan + `scale=1`**；**透明修复后的像素复采 = PENDING-RECAPTURE**；不是完整产品/跨合成器/分数缩放/多显示器/性能/vendor/精度结论 |
| X11 / XWayland | **Unsupported**：GTK 前拒绝，标记行 + i18n 诊断 + **退出码 4**，无窗口、无截图 | 依据是 2026-10-22 探针**合成 FAIL**与机制未闭环；**不是“X11 不可能”**；web+canvas 回退路线显式延后 |
| Windows | **实现 + Tier 0 编译门禁（全部 rc=0）+ Wine 拒绝合同 smoke（三次 rc=5）** | **Tier 0 ≠ 可运行 GUI**；Wine ≠ 真 Windows；z-order/DPI/ProcessFailed 活体/picker/runtime-present 出帧全部 **NOT RUN** |
| macOS / Android / Web | 显式 NOT IMPLEMENTED 占位 | 不是可运行 GUI 产品；不得用本记录推导 |

**历史区分（不得翻案）**：更早的**整窗顶层透明 alpha 全屏叠加**是历史 FAIL（ADR 0006 / SPEC §21）。
本记录 PASS 的是**分区子表面 `place_below`**（CAD 表面仍只占 CAD 矩形），二者机制与证据链不同。
本记录也**不**替代此前的迁移/产品验收，不宣称 CAD 视觉正确性、精度、vendor 兼容或性能。

## 实现快照与环境

- 分支 `dom-on-top-layering`；提交序列（本记录覆盖范围）：
  `dece71c`（UI 归并 `apps/desktop-ui`）、`10a61e5`（新 crate `cad-desktop-shell`）、
  `0093dae`（WP1 门禁修复）、`359d58a`（WP2：Wayland 反转生产化 + X11 拒绝）、
  `9ecce95`（WP3：弹窗迁 in-DOM 覆盖层）、`6e4bf06`（生产透明修复）、
  `dd23bea`（WP4：Windows 宿主）、`9fba811`（WP4 门禁修复：布局反转守卫、z-order 失败去重、
  退出码注册表文档）。
- 架构决策与失效条款见 [ADR 0023](adr/0023-dom-on-top-cross-host-composition.md)；
  宿主使用说明见 [react-linux-app.md](react-linux-app.md) 与 [windows-app.md](windows-app.md)。
- 运行环境（Linux）：同 UID 的现有 GNOME/Mutter **Wayland** 会话；`scale=1`；CAD presenter 为
  **Quadro P620 / Vulkan / discrete_gpu**（`nvidia-smi` 580.178.04 与
  `[probe] status backend="vulkan" adapter="Quadro P620"` 双证）；证据运行设置
  `YACR_PROBE_NO_MAXIMIZE=1`（仅为取景，不改变产品默认最大化策略）。
- 启动方式：`scripts/collect-evidence.sh` 真实构建 + 真实 `--open fixtures/dwg/synthetic-four-lines.dwg
  --fonts-dir fonts`（**不用** `--probe` 假打开），原生 Wayland 帧经同 UID
  `org.gnome.Mutter.ScreenCast` 抓合成画面；X11 行只采集**拒绝合同**。
- 本记录所有证据目录位于本机 `/tmp/opencode/`，**仅本地保存、不上传**；Wayland 帧是**整屏合成**，
  可能含桌面上其它窗口。

## WP2：合成反转生产化 + X11 显式 Unsupported

### 1. 证据运行（`yacr-evidence-wp2-final6`）

`results.tsv` 三行，每行对应一次独立真实启动（独立 `app.pid` / `app.rc` / `run.log`）：

| backend | case | verdict | exit | screenshot | 判读 |
| --- | --- | --- | --- | --- | --- |
| x11 | production | `UNSUPPORTED_BACKEND` | 4 | missing | 拒绝合同成立 |
| wayland | production | `NATURAL_EXIT` | 0 | captured | 生产 chrome/层序帧 |
| wayland | stacking-regression | `NATURAL_EXIT` | 0 | captured | `classification=STACKING_REGRESSION`；4/4 帧 |

X11 `run.log` 逐字节为：

```
yacr: unsupported-backend: x11
yacr 无法在 X11/XWayland 上启动
本版本将 DOM/WebView 叠加在原生 CAD 表面之上，该合成仅实现于原生 Wayland。请在 Wayland 会话中运行（GDK_BACKEND=wayland）；X11 的 web+canvas 路线已显式延后。
```

- 分类以**稳定标记行**为第一依据，退出码 4 仅辅助；拒绝发生在 GTK/Tauri 初始化**之前**，
  因此**不存在窗口与截图**（`screenshot=missing` 是正确结果，不是采集失败）。
- Wayland 生产行为 `NATURAL_EXIT`（应用自行退出，非 SIGTERM）；CAD 帧持续呈现
  （`cad-frame presented … complete=current_scene`，4 实体、`error=null`）。
- 回归夹具是**单次启动采样四次**（`initial`、`popup-open`、`after-synthetic-input`、
  `after-synthetic-input-later`），触发契约三件套：`YACR_PROBE_STACKING=1`（严格 `"1"`）+
  init-script `window.__YACR_STACKING_REGRESSION__ === true` + 真实 `--open` 会话；
  `--probe` / `YACR_PROBE_ONLY` 同现启动即拒绝。夹具**不是产品开关**。

### 2. Observer 判读（5 帧，WP2）

| 判据 | 结论 | 观察 |
| --- | --- | --- |
| A 不透明 chrome 遮挡 CAD | **PASS** | crosshair 在不透明按钮矩形处**精确中断** |
| B 半透明面板混合 | **PASS** | 实测 α≈0.549，与源码 `.55` 一致 |
| C 弹窗跨 chrome/CAD 边界 | **PASS** | 分隔线中断、crosshair 被弹窗裁切 |
| D 标记运动 | **PASS** | 第 3→5 帧单调右移；第 2 帧灰十字（非橙）判 INDETERMINATE，不影响结论 |
| E 生产层序 | **PASS（带覆盖限制）** | 限制：陈旧第二窗口、CAD 区底部出屏、canvas 无特征 |

以上是**有界截图视觉判读**：合成帧只证明本轮合成方向在该环境成立，不证明视觉正确性、精度或
vendor 行为。**夹具层序 ≠ 生产透明度**——WP2 之后的手工测试恰好抓到这一点（见下节）。

## WP3：弹窗迁入同一 React 树的 DOM 覆盖层

### 1. 证据运行

| 目录 | 内容 | 判读 |
| --- | --- | --- |
| `yacr-evidence-wp3-final1` | Wayland 生产启动（`NATURAL_EXIT` 0、`captured`），帧含 About 覆盖层 | **About 帧 PASS** |
| `yacr-evidence-wp3-textedit4` | `popup_textedit_pass=1`；`EVIDENCE\PEDIT` 往返（`te_payload=EVIDENCE\nEDIT`、`te_text=EVIDENCE\PEDIT`、`te_gate_released=1`、reopen 两行）；2 张 textEdit 帧 | **INVALID wrong-window**（帧内容是 Chrome/终端，不是 `yacr`） |

- textEdit 的运行日志是**行为证据**（宿主派生 payload、admission accepted、应用后回读一致、
  模态门释放与重开）；但**帧无效**，因此视觉结论为 **NOT RUN**，不得用日志代替像素。
- 弹窗机制的契约证据（非像素）：
  - `D2` host 模态围栏语义不变：`cad_status` wire golden 夹具未改且 4/4 绿；`popup.modal_open`
    对 `cad_command` 与 `probe_input` 双漏斗拒绝。
  - 清除触发器**四触发**：显式 close / reload-navigation / renderer 进程崩溃（`web-process-crashed`、
    `ProcessFailed`）/ 进程退出；Esc 与关闭按钮在正文 error-boundary 之外。
  - **renderer 崩溃活体注入 NOT RUN**，只有契约测试。
- UI 侧 “DOM 测试” 是 `node:test` 的**源码结构/纯函数契约**，**不是** jsdom 挂载或运行时像素证据。

## 生产透明缺陷与修复（WP2/WP3 之间）

- **症状（人工复测发现）**：迁移后 CAD 层在生产 UI 中不可见，但夹具帧看起来正常。
- **根因**：`.shell` / `.workspace` / `.workspace__canvas` 在 CAD 视口区画了**不透明底色**。
  夹具验证的是层序，不能覆盖生产 DOM 的绘制不透明度。
- **修复**：提交 `6e4bf06` 将三者改为 `background: transparent`；回归测试在
  `apps/desktop-ui/test/metrics.test.mjs`（钉住透明不变量）。
- **复采尝试 `yacr-evidence-wp3-fix1`**：X11 仍为拒绝（exit 4、无截图）；Wayland 为
  `NATURAL_EXIT` 0 但 **`screenshot=missing`**——屏保抑制了 ScreenCast 会话，采集未越权解锁会话。
  因此该目录只有构建/运行日志（Vulkan/P620、`entityCount=4`），**没有可用像素**。

### PENDING-RECAPTURE（不预写结论）

**生产透明修复后的像素复采 + Observer 重判 = PENDING-RECAPTURE。** 本记录**不**声称透明视觉已
验收。复采必须满足：

1. 取景包含**可见几何**（不能只拍 featureless canvas）；
2. 捕获时确认目标窗口**在前台**并核对窗口/标题（避免再次得到 wrong-window 帧）；
3. 新输出目录，不覆盖 `wp2-final6` / `wp3-*` 的既有证据；
4. 结论仍限定 **GNOME/Mutter + P620 + `scale=1`**。

参考命令（同 UID 会话；若会话仍被屏保抑制，需用户自行激活会话，**不得**越权解锁或改权限）：

```bash
scripts/collect-evidence.sh --backend wayland \
  --dwg fixtures/dwg/synthetic-four-lines.dwg \
  --fonts-dir fonts \
  --out /tmp/opencode/yacr-evidence-postfix-$(date -u +%Y%m%dT%H%M%SZ)
```

## WP4：Windows 宿主（实现 + Tier 0 + Wine smoke）

### 1. 实现形态

`apps/app-windows` 已是真实 Tauri 2 + WebView2 + Win32 宿主（非占位）：
`src/{entry,support,win32_surface,redraw,platform,main_thread}.rs` 与 `src/host/{init,tick,popup}.rs`；
透明 WebView2（背景 alpha 恰为 0）+ `WS_CHILD | WS_VISIBLE | WS_DISABLED | WS_CLIPSIBLINGS` 的
CAD child HWND 压在 `WRY_WEBVIEW` 容器紧邻之下，`ensure_order` 幂等重排；输入仍走单一 DOM 漏斗，
**不使用** `WM_NCHITTEST`/`HTTRANSPARENT`，也**不声称**具备原生点击穿透能力。
`WS_EX_NOREDIRECTIONBITMAP` 为 best-effort（Tauri 2.11.6 无 builder flag）。

### 2. Tier 0 编译与契约门禁（全部 rc=0）

环境：Linux 主机 + `TMPDIR=$HOME/.cache/yacr-tmp` + `cargo-xwin 0.23.1` +
`llvm-rc`（`/usr/lib/llvm-21/bin` 需显式加入 `PATH`）。日志在 `$HOME/.cache/yacr-wine/logs/`。

| # | 命令 | rc |
| --- | --- | --- |
| 1 | `cargo fmt -p app-windows -- --check` | 0 |
| 2 | `cargo clippy -p app-windows --all-targets --locked -- -D warnings` | 0 |
| 3 | `cargo xwin clippy -p app-windows --target x86_64-pc-windows-msvc --locked -- -D warnings` | 0 |
| 4 | `cargo test -p app-windows --lib --locked`（6 passed） | 0 |
| 5 | `cargo xwin check -p app-windows --target x86_64-pc-windows-msvc --locked` | 0 |
| 6 | `cargo xwin build -p app-windows --release --target x86_64-pc-windows-msvc --locked` | 0 |
| 7 | `cargo build -p app-windows --release --target x86_64-pc-windows-gnu --locked` | 0 |
| 8 | `cargo check -p app-windows --lib --target wasm32-unknown-unknown --locked`（空壳隔离） | 0 |
| 9 | `python3 scripts/check-i18n.py`（716 keys / 2 catalogs） | 0 |
| 10 | `python3 scripts/check-file-size.py`（851 files，上限 1000） | 0 |
| 11 | `python3 scripts/check-architecture.py`（28 packages） | 0 |
| 12 | `python3 scripts/test-ui-migration-guards.py`（18 tests） | 0 |

**编译门禁 ≠ 可运行 GUI**：`cargo xwin build` 只证明 MSVC 目标可链接（产物
`target/x86_64-pc-windows-msvc/release/yacr-windows.exe`）。

### 3. Wine 拒绝合同 smoke（**不是** Windows 运行证据）

统一环境：`WINEPREFIX=$HOME/.cache/yacr-wine/prefix`、`WINEARCH=win64`（**迁址缘由**：`/tmp/opencode` 遇 per-user `EDQUOT` 配额不足，前缀与日志改落 `$HOME/.cache/yacr-wine/`，未做任何系统级变更）、
`WINEDLLOVERRIDES="mscoree,mshtml="`；prefix 内无 WebView2 运行时。

| # | 产物 / 场景 | rc | 观察 |
| --- | --- | --- | --- |
| W1 | GNU release，Evergreen/Fixed 均缺失 | **5** | 第一行 `yacr: unsupported-runtime: webview2` → 细节 → zh-CN 标题/详情；无窗口 |
| W2 | GNU release，`WEBVIEW2_BROWSER_EXECUTABLE_FOLDER=C:\no-such-webview2` | **5** | 细节点名坏 Fixed-Version 路径，证明该分支独立 |
| W3 | MSVC release（交付目标），无运行时 | **5** | 与 W1 同形态；MSVC 产物静态链接 loader，拒绝路径在无运行时机器上可达 |

GNU 产物动态导入 `WebView2Loader.dll`；W1/W2 仅在 smoke 目录旁放置 crate 自带 DLL（未入库）。

### 4. Windows NOT RUN 清单

| # | 未运行项 | 闭环方式 |
| --- | --- | --- |
| N1 | spike-1 视觉透明（透明 DOM 下可见 wgpu CAD 帧、resize 无残影） | 用户真 Windows `--open` + 截图/ex-style dump/alpha 读回 |
| N2 | spike-4 z-order 稳定性（show/hide、最大化/还原、DPI 切换、右键菜单、DevTools、多显示器） | 逐步 `GetWindow(container, GW_HWNDNEXT)==cad` + re-assert 计数 |
| N3 | spike-5 Runtime 版本敏感性（决定 Evergreen+记录 vs pin Fixed Version） | 两版本各跑一次 alpha 对照 |
| N4 | DPI / 多显示器缩放（100↔150%、跨屏） | N2 第 3 步 + `geometry … scale=` |
| N5 | ProcessFailed 实触发（渲染进程崩溃 → 清 modal gate + 诊断） | `taskkill /f /im msedgewebview2.exe` 后核对 `cad_status` |
| N6 | 真实 COM 文件选择器（取消/选择/失败三态） | 真 Windows 打开 picker |
| N7 | runtime-present 启动路径（窗口/WebView2 创建、alpha 读回、CAD child install、出帧） | 装 Evergreen 后启动核对 probe 行 |
| N8 | Wine 透明/合成视觉 | **结构性不可能**（DComp stub bug 58921；`WS_EX_LAYERED|NOREDIRECTIONBITMAP` 黑屏 bug 55432） |
| N9 | `WS_EX_NOREDIRECTIONBITMAP` 实际生效（创建闪屏） | 真 Windows ex-style dump；或升级 Tauri ≥2.12.3 |
| N10 | Windows 硬件 Vulkan 出帧/精度/性能 | N7 + `YACR_PROBE_GPU_TIME`（测量专属） |

**任何 Wine 像素/截图结论必须写 `NOT VERIFIED (Wine limitation, bug 55432/58921)`。**
Windows 结论只能限定“该 Runtime 版本 + 该 GPU/驱动 + 该 Windows 版本 + 该缩放”；**禁止**写
“Windows 已验收”或无限定的“透明支持”。

## 本轮实际运行的门禁与 NOT RUN

- **WP2 门禁复核（实跑）**：`cargo test -p app-linux` 138/138、`cargo test -p cad-ui-core` 212/212、
  `(cd apps/desktop-ui && npm test)` 403/403、`clippy -p app-linux --all-targets -- -D warnings`、
  `cargo fmt --all -- --check`，以及 `check-architecture` / `check-workflows` /
  `test-linux-workflow` / `test-ui-migration-guards` / `check-fixture-manifest` / `check-i18n` /
  `test-collect-evidence.py`，全部 rc=0。
- **WP3 门禁复核（实跑）**：`cargo test -p cad-ui-core` 215/215、`-p cad-desktop-shell` 11/11 +
  `wire_goldens` 4/4、`-p app-linux` 134/134、`(cd apps/desktop-ui && npm test)` 411/411、
  `fmt --check`、`clippy`（三 crate，`-D warnings`）、`check-file-size` 840 files、
  `check-architecture` 28 packages、`check-i18n` 709 keys，全部 rc=0。
- **WP4 门禁（实跑）**：见上表 1–12。
- **NOT RUN（如实）**：全 workspace 测试与全量 CI 门禁矩阵（按日常有界策略，本轮只验证改动范围）；
  Android/wasm 平台矩阵（仅 `app-windows` wasm 空壳 check 已跑）；N>1 次 surface generation 的
  **运行期**重放（仅代码路径与代际计数验证）；renderer 崩溃活体注入；Windows 全部视觉/运行时项；
  生产透明修复后的像素复采。

## 证据目录、授权与复核方式

| 目录（`/tmp/opencode/`） | 用途 |
| --- | --- |
| `yacr-evidence-wp2-final6` | X11 拒绝行、Wayland 生产帧、stacking 夹具 4/4 帧、`results.tsv`、`README.md` |
| `yacr-evidence-wp3-final1` | Wayland 生产帧（About 覆盖层） |
| `yacr-evidence-wp3-textedit4` | textEdit 行为日志 + 2 帧（wrong-window，作废） |
| `yacr-evidence-wp3-fix1` | 透明修复后构建/运行日志；截图缺失 |
| `$HOME/.cache/yacr-wine/{logs,smoke,smoke-msvc}` | WP4 编译门禁与 Wine rc 记录 |

**授权边界**：全部以同 UID 用户态运行；**不提权、不以 root 运行、不 `sudo`、不解锁或修改会话、
不改变 portal/ScreenCast 权限、不安装系统包、不启动虚拟/无头显示**。截图仅本地保存、不上传。
无真实显示时脚本显式非零退出，而不是假装成功。`x11-window-grab` 是历史 X11 工具；当前 X11 后端
被拒绝，**不得**用其产物冒充当前后端证据。

**复核**：`cat` 各目录 `results.tsv` 即可核对退出码/截图列；X11 拒绝以 `x11/run.log` 首行标记行为准；
Wayland 帧须由独立 Observer 重新判读，不得仅凭“文件存在/非空”推断视觉正确。
