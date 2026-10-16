# ADR 0010：桌面 UI 以 React/Tauri 原生宿主替代 Slint

- 状态：**已接受（Accepted，2026-10-16）**——取代 ADR 0006 的“实验”定位，并使 ADR 0002/0003
  中“Slint 为桌面合成基线”的部分作废。
- 相关：`CAD_IMPLEMENTATION_SPEC.md` §21；`docs/architecture.md`；`docs/react-linux-app.md`；
  `docs/handoff.md`；报告 `.slim/deepwork/replace-slint-ui/`。

## 背景

原桌面路线（ADR 0002/0003）以 **Slint 作为唯一 UI 合成器**：Slint 拥有 surface/事件循环，
CAD 通过 `cad-ui-slint/src/bridge.rs` 与 wgpu 合成。2026-10-14 用户授权以 **React + Tauri** 作为
桌面替代实验（ADR 0006）。该实验随后被判定成熟到足以取代 Slint 桌面路线：P1（`cad-ui-core`
抽取）、P2（宿主合并）、P3（Slint 删除与脚本/CI 退役）、P4（AutoCAD 风格外壳重设计）已完成
源码级实施，P3b Oracle 源码评审 **ACCEPT**（仅源码，未运行）。

## 决策

**桌面（Linux 优先）UI 路线正式改为 React + Tauri 原生宿主；`cad-ui-slint` 与全部 Slint 依赖删除。**

关键点：

1. **宿主**：`apps/app-linux`，二进制 `yacr`，Tauri 标识 `dev.yacr.app`，产品名 `Yacr`。
   配置目录由 `~/.config/yacr-react` 迁移为 `~/.config/yacr`（**不做自动迁移**）。
2. **组合方式**：**GTK 中央 native CAD 分区** + **DOM chrome**，二者 co-visible。
   CAD 由原生 Rust（renderer-only wgpu，`cad-render-wgpu::SurfaceProbePresenter`）绘制在
   `GtkDrawingArea`/`GtkOverlay` 的 child surface 上；React（Vite）chrome 由 WebView 呈现。
   **不允许**跨 CAD 分区的 DOM 浮层；下拉/补全/历史**就地展开**，面板**停靠**（in-flow），
   从而不会压到原生 surface。
3. **共享 UI 逻辑**：平台无关部分下沉到新 crate `cad-ui-core`（protocol/layout/session/shell/
   i18n），位于 UI 无关的 `cad-app`/`cad-host`/`cad-engine`/`cad-platform` 契约之上、具体宿主之下。
   宿主通过 `state.shell.*` 显式访问共享状态，直接 `use cad_ui_core::…`，无过渡 re-export 别名。
4. **不变量保持**：`cad-db` 仍是唯一权威数据源；renderer-only wgpu（仅 `cad-render-wgpu` 依赖
   wgpu）；importer-only acadrust（仅 `cad-import-acadrust` 依赖 acadrust，禁止 patch）；ChangeSet →
   依赖失效 → 显示表示重建的增量语义不变；每条合成路径只有一个 CAD presenter，不做整幅 GPU→CPU
   回读。
5. **平台范围**：本轮**只有 Linux/X11 宿主是真实可运行 GUI**。非 Linux 目标上 `app-linux`
   编译为空库、二进制返回显式 NOT IMPLEMENTED；`app-windows`（`yacr-windows`）、`app-macos`
   （`yacr-macos`）、`app-android`、`app-web` 都是**显式 NOT IMPLEMENTED 占位**，不是可运行 GUI 产品。

## 现状范围（诚实）

- **仅 OPEN / FIT 已接线**到真实漏斗（打开文件、适应图纸）。其余 ribbon/QAT/命令/面板控件
  一律**显式禁用并标注原因**（`command.unsupported`、`command.badge.unwired`、
  `status.toggle_unsupported`、`layout.switch_unwired` 等）；**不新增**投机 IPC。
- 原生导航按实际源码能力支持（指针/滚轮缩放锚定光标、平移、fit）；`cad-ui-core` 的
  `COMMAND_TABLE` 覆盖 44 个 `CommandId`，需要 payload 的命令经真实漏斗被**显式拒绝**，绝不空成功。
- **不是完整 CAD 产品**：真实 GPU/视觉验收已**有界执行**（硬件 Vulkan + 真实 GTK/XWayland 窗口，合成夹具），
  但 vendor DWG 兼容、精确视觉 golden、性能与跨平台运行**未验证**；证据见
  `docs/validation-react-linux-migration-2026-10-16.md`。

## 退役与后果

- 删除 `crates/cad-ui-slint`（含其重复 i18n 副本与 Slint 字体）与 workspace 的 `slint`/`slint-build`
  声明；`Cargo.lock` 按成员可达性剪枝（845 → 622 包）。
- `check-architecture.py` 现在**全 workspace** 禁止 `slint`/`slint-build`/`iced`（含 build/dev 与
  target 表、workspace 继承别名），保留 acadrust/wgpu 唯一依赖与分层规则。
- `check-i18n.py` 的**唯一权威目录**改为 `crates/cad-ui-core/i18n/{zh-CN,en}.json`，并新增对 React
  JSX/JS 的受限 CJK 硬编码扫描。
- 旧 Slint 验证/离屏/真实 GPU 测试套件、web shell、Flatpak 打包随之退役；六个公共
  构建/打包/部署脚本改为显式 NOT IMPLEMENTED（非零退出），8 个 CI job 以 `if: false` 硬禁用；
  **Linux release 暂时不可用，等待替代打包方案验收**。
- `docs/adr/0002`、`0003`、`0006` 与 `docs/validation*.md`、`docs/linux-app.md`、`docs/verify-ui.md`
  等历史记录**保留过去事实**，只加“已退役/被取代”横幅并指向本文与 `docs/architecture.md`。

## 后果与权衡

- 优点：桌面 chrome 用 Web 技术迭代更快；GTK child surface 明确解决“顶层透明叠加”历史 FAIL；
  `cad-ui-core` 让 UI 逻辑与工具包解耦，便于未来第二宿主。
- 代价：Tauri/GTK/WebKitGTK 系统依赖；Windows/macOS 需另行实现（本轮 NOT IMPLEMENTED）；
  旧 Slint 特性证据（选择/编辑/历史、文字渲染、API/事件等）不自动由 React 宿主持有，需在 P6
  用替代证据重新评估。

## 验证状态

**P6 最终验证已执行（有界、分区）；Gate P6 attempt2：ACCEPT-WITH-FIXES（文档限定已应用）。**

- 已运行：前端（`npm ci`/`npm test` 43/43/`npm run build`）、`cargo fmt --check`、clippy、四个 Python
  门禁与十个 Python 契约（含 18 项迁移守卫）、平台无关 PNG node 契约、`--locked` 的 metadata/clippy/
  workspace all-targets check/wasm lib check/workspace 测试（1825 passed/0 failed/19 ignored）/桩 lib 测试；
  真实硬件 GPU（Quadro P620 / Vulkan / 真实 GTK XWayland）的 open+渲染 smoke、原生分区生命周期
  （forced unrealize→新 XID+generation 2、自然 exit 0）、外部 WM minimize→restore 与真实 resize。
- 解析器一度被**手工锁不同步**阻断（`cargo … --locked` exit 101）；已按批准的最小离线同步修复
  （仅 `Cargo.lock`，622→595，纯删 27 孤儿包，无版本变动，acadrust 0.6.3 不变，无 `[patch]`），六条复跑全 exit 0。
- **仍未验证/未支持**：精确视觉 golden、性能、设备丢失、真实鼠标光标锚定像素验收、真实 portal picker、
  vendor DWG、Windows/macOS/Android/Web 运行、打包。**gate 结论**：Gate P6 attempt2 ACCEPT-WITH-FIXES
  （文档限定已应用）；observer 终判已观察截图**无重大视觉缺陷**；**有界本地 Linux 迁移验收通过**，
  **不是**完整 CAD 产品/跨平台/vendor/golden/性能验收。应用侧 `unminimize()`+强制 unrealize 序列**未解决**。
- 完整命令/退出码/日志与证据路径见 `docs/validation-react-linux-migration-2026-10-16.md`。
- **早期阶段**在其当时分支/代码树上另有有界执行（P1 core/app 契约 59+27→86 与 check/fmt；P2a 编译检查；
  P4 前端构建+43/43 测试）；这些历史证据**不**替代本次最终树验收，也未被抹除。
