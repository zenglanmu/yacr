# AGENTS.md

Rust CAD 查看、测量与批注系统，**开发搭建中**：契约框架 + 合成测试，没有可交付产品。
需求唯一权威是 `CAD_IMPLEMENTATION_SPEC.md`（v2.0，含 §21 桌面宿主增补）；`docs/handoff.md`
是逐轮交接入口；桌面 UI 决策见 `docs/adr/0010-react-linux-host-replaces-slint.md`。

> **2026-10-16 迁移状态**：桌面 UI 已由 Slint 迁移为 **React + Tauri 原生宿主**
> （`apps/app-linux`，二进制 `yacr`）；`cad-ui-slint` 与所有 Slint 依赖已删除。
> Windows/macOS/Android/Web 是**显式 NOT IMPLEMENTED 占位**，不是可运行 GUI 产品。

## 铁律（违反即返工）

1. 不修改 acadrust 源码，不使用 Cargo patch（`check-architecture.py` 会拒绝）。
2. 未支持/未验证能力必须显式建模（`Unsupported`/`Partial`/诊断），占位与假数据不计完成，
   也**禁止**把失败改成空成功。
3. 业务层只操作数据库/命令/事务；GPU 提交封装在渲染器内。
4. 增量更新：ChangeSet → 依赖失效 → 显示表示重建，不重解析底图。
5. 每项改动同步补契约测试与文档。
6. 证据诚实：未运行就说未运行。README/handoff 中大量“已通过”注明仅为**合成测试**或
   特定环境（模拟器/软件 GPU/无头浏览器），不得推广为真机、真实 GPU、vendor 兼容结论。
   历史 Slint 宿主的证据不得冒充 React 宿主结论。
7. 不覆盖/删除陌生变更（并发实现常见）；先读源码和最新构建证据。

## 语言约定

代码标识、注释、测试名、commit message 用英文；`docs/`、`AGENTS.md` 用中文。
用户可见文案只能来自**唯一权威目录** `crates/cad-ui-core/i18n/{zh-CN,en}.json`（键集必须一致，
见 `scripts/check-i18n.py`），不得在 React JSX/JS 或 Rust UI 里硬编码中文。
**该门禁的实际范围（2026-10-20 更正，勿按过度承诺理解）**：`check-i18n.py` 只扫
`crates/cad-ui-core/src`、`apps/app-linux/src`（Rust）与 `apps/app-linux/ui/src`（JS/JSX）并拒绝其中的**硬编码 CJK**；
**`crates/cad-app` 仍含用户可见中文字面量**（如 `measure_tool.rs` 的提示文案），它们由
`crates/cad-app/src/tests/cjk_baseline.rs` **冻结**——只**禁止增长**，不是"已合规"。这是**已知债务**，
新代码**不得**沿用该做法。
**机械规则（已发生三次，务必照做）**：在 `crates/cad-app/src/selection.rs` 新增 `PropertyRow::new("<key>")` 时，**必须同时**补两个 i18n 目录的 `properties.row.<key>` **与** `apps/app-linux/ui/src/cad.mjs` 的 `propertyRowLabel` switch 分支；`apps/app-linux/ui/test/properties.test.mjs` 会扫源码字面量并要求两处齐全，否则失败。

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
cad-ui-core（protocol/layout/session/shell/i18n；UI 框架与 GPU 无关）
  ↑
apps/app-linux（React/Tauri 真实 Linux GUI）  apps/app-windows|app-macos|app-android|app-web（NOT IMPLEMENTED 占位）
```

- `cad-host` 是平台无关的宿主编排层，**不依赖任何 UI 工具包**（不依赖 slint/tauri/gtk/wgpu/
  web-sys/Android 类型）；平台 I/O 一律走 `cad-platform` 契约（事件式、无 futures、无 `Send`）。
- **`cad-ui-core`** 承载平台无关的 UI 逻辑（wire protocol、逻辑布局、CAD 会话、宿主 admission/
  error 状态、i18n 目录），位于 `cad-host` 之上、具体宿主之下。它依赖 `cad-host`（会话包装
  `HostCore`），必须保持 UI 工具包/GPU 无关。宿主通过显式 `state.shell.*` 访问共享状态、
  直接 `use cad_ui_core::…`，**不保留**过渡 re-export 别名。
- `cad-engine` 是 UI 无关的渲染会话协调库（`cad-app`/`cad-db`/`cad-domain`/`cad-render-wgpu`/
  `cad-representation`）；`cad-app` 不依赖 `cad-engine`，`cad-engine` 不依赖 `cad-ui-core`/
  `cad-host`/任何 UI 工具包或直接依赖 wgpu。
- `apps/app-linux` 是 React/Tauri 真实 Linux GUI（Tauri 2 WebView 中的 React chrome + GTK 中央
  原生 CAD 分区 + renderer-only wgpu），支持 **X11/XWayland** 与**原生 Wayland** 两种后端：
  X11 用原生 X child 窗口，Wayland 用 app-owned `wl_subsurface`（`apps/app-linux/native/` 私有 C
  shim；`build.rs` 经 `cc`+`pkg-config wayland-client` 编译，需 `libwayland-dev`）；后端由
  `backend.rs` 在 GTK 启动前显式决议 `GDK_BACKEND`，**无静默回退**。非 Linux 目标上它编译为空库、
  二进制返回显式 NOT IMPLEMENTED。`apps/app-windows|app-macos|app-android|app-web` 是显式 NOT
  IMPLEMENTED 占位。平台 I/O（文件、配置、表面度量）只存在于 `app-*`。
- **桌面窗口策略**：Linux 宿主默认以**最大化窗口**启动（`apps/app-linux/src/host.rs` 的
  `start_maximized` 默认 `true`，`WindowBuilder::maximized(true)`），**保留标题栏/系统面板**——
  这是**窗口管理器最大化**，**不是**全屏/无边框 kiosk（源码未调用 `fullscreen`/`decorations(false)`）。
  仅验收/证据运行用 opt-out 环境变量 `YACR_PROBE_NO_MAXIMIZE`（**其存在**即退出最大化）；
  `tauri.conf.json` 无窗口配置（`"windows": []`），不覆盖此默认。跨窗口管理器/平台的行为不保证
  （见下方「构建与门禁」的 NOT VERIFIED 边界）。

`check-architecture.py` 强制的边界：`cad-domain` 不依赖任何 CAD 包；`cad-db` 只依赖
`cad-domain`；`cad-import-acadrust` 是**唯一**依赖 acadrust 的 crate；`cad-render-wgpu`
是唯一依赖 wgpu 的 crate；`cad-ui-core` 不得依赖 slint/wgpu/tauri/gtk/portal/平台类型；
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
(cd apps/app-linux/ui && npm ci && npm test && npm run build)
```

**代码文件行数上限**：任何**代码**文件不得超过 **1000 行**（`scripts/check-file-size.py`，含
`scripts/test-check-file-size.py` 自测）。`.md` **不计入**（`docs/handoff.md` 等按设计就是长日志/清单）。
文件接近上限时按模块拆分，**不要**加白名单豁免。

默认 Linux host workspace 编译包含 `app-android` 与 `app-web` 包的 host target；这不替代
下方独立的 WASM 与 Android 原生目标编译门禁，也不代表这些平台的运行验收。clippy 仍按
既有配置排除 `app-android` 与 `app-web`。

- **Windows/macOS/Android/Web**：本轮为 **NOT IMPLEMENTED 占位**。CI 只做**编译门禁**
  （`windows-check`/`macos-check`/`android-check`），**不代表**可运行 GUI 或运行验收。旧的
  平台发布/打包脚本已退役或禁用（非零退出）；Linux release 暂时不可用，等待替代打包验收。
  **不要**把已退役脚本当作可用流程。
- **渲染/端到端验证优先真实硬件 GPU**：只要环境存在真实显示与显卡（本机为 Wayland 桌面 +
  Intel UHD + Quadro P620，可用 `nvidia-smi`/DRM fdinfo 证明），验收**必须**走真实 GPU。
  当前桌面宿主（React/Tauri）的后端支持 **X11/XWayland** 与**原生 Wayland**（原生 Wayland 决策/
  子表面桥见 `docs/adr/0011-native-wayland-subsurface.md`；本轮有界源码/门禁/原生 `--probe` 证据见
  `docs/validation-native-wayland-2026-10-16.md`）。真实 GPU 路线是 env-gated 证据驱动
  （`apps/app-linux/src/evidence.rs`：`YACR_PROBE_EVIDENCE`/`YACR_PROBE_LIFECYCLE`）+
  `apps/app-linux/scripts/x11-window-grab.c` 窗口捕获（**X11**），由 `scripts/run-react-linux.sh`
  启动（**不强制** `GDK_BACKEND`）。W3 已执行有界原生运行（CAD/输入/生命周期/几何/X11）并由 Observer
  终态判定**有界截图视觉 PASS**（同 UID Mutter `ScreenCast` 捕获，无安全策略变更）；**最终 Oracle W3
  attempt3 终判 ACCEPT-WITH-LIMITATIONS（无 blocker）**：有界本地验收（GNOME/Mutter + P620 + `scale=1`），
  不得据此宣称完整原生支持或 vendor/精度/性能结论；X11 截图**不是**原生证据（且视觉未判读）。
  证据见 `docs/validation-native-wayland-2026-10-16.md`。
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

## 平台占位（NOT IMPLEMENTED，勿当可运行）

- **Android**（`apps/app-android`）：UI 已退役，是显式占位（原生入口打印退役诊断后 abort）。
  仅保留 `cargo check --target aarch64-linux-android -p app-android --locked` 编译检查。
  `build-android.sh` 非零失败；不再有 APK 流水线。**模拟器 ≠ 真机**，真机一律未验证。
- **Web**（`apps/app-web`）：UI 已退役，wasm 入口返回错误；`build-web.sh`/`deploy-cloudflare-pages.sh`
  非零失败。不再有 web 发布。
- **Windows/macOS**：显式 NOT IMPLEMENTED 失败桩（`yacr-windows`/`yacr-macos`），只随
  `--all-targets` 编译；不打可分发 GUI。

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
**不用** `--probe` 假打开）→ 截图 → 汇总日志」。X11 后端用 `apps/app-linux/scripts/x11-window-grab.c`
抓窗口**自身 drawable**（`shot-top`/`shot-cad`）；原生 Wayland 后端用 `scripts/screencast-capture.py`
经同 UID `org.gnome.Mutter.ScreenCast` 抓合成帧（`wayland-frame`）。输出落
`${YACR_EVIDENCE_ROOT:-/tmp/opencode}/yacr-evidence-<UTC>Z/`：`results.tsv` 逐后端记录
`NATURAL_EXIT`/`PLANNED_TIMEOUT`/`SKIPPED`/`NO_DISPLAY`、真实退出码、截图 `captured`/`missing`；
`README.md` 附授权边界。契约测试 `python3 scripts/test-collect-evidence.py`。
**授权边界**：以当前会话同 UID 用户运行，**不提权、不以 root 运行、不 `sudo`、不解锁/修改会话、不改
portal/ScreenCast 权限、不装包、不起虚拟/无头显示**；无真实显示时**显式非零退出**而非假装成功；
截图仅本地保存、不上传。X11 自窗口抓取不需额外授权；Wayland 走用户**自身会话内**的 ScreenCast
（有录制指示、不抑制），**不是**绕过授权。X11 截图只含应用窗口本身；**Wayland 帧为整屏合成**（同 UID
`RecordMonitor`），**会包含桌面上其它窗口**——仅供本地读判、不上传。截图只是窗口/合成像素，
**不等于**视觉正确性、精度或 vendor 结论。

## 交接点

用 `rg 'pending\(' crates apps` 查找显式占位（文件路径与标识对应实施单元）。替换 `pending`
必须补验收测试，禁止改成空成功。已定义但仍需设计审查或未闭环的事项集中在
`docs/handoff.md`「已定义，但尚需设计审查」与各 `docs/*.md` 的“未完成”小节。

## 文档索引

`docs/architecture.md`（边界与不变量）、`docs/react-linux-app.md`（当前桌面宿主）、
`docs/build.md`（构建与退役入口）、`docs/ci.md`（CI 分层与 NOT RUN）、`docs/validation*.md`
（历史执行证据，含 NOT RUN 边界）、`docs/compatibility.md`（能力表）、`docs/handoff.md`
（逐轮交接）、`docs/adr/`（含 ADR 0010、0019–0021；0021 = 弹窗层：模态窗口 + 原生锚定菜单）、`docs/testing-dwg.md`、`docs/app-icon.md`、
`fixtures/manifest`。
