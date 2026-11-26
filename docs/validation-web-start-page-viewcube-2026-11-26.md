# Web 载体：起始页不再自动打开夹具 + `dom-canvas` 方位球可交互（2026-11-26）

范围：`apps/app-web`（wasm 页面会话）、`crates/cad-ui-core`（会话入口与载体 profile）、
`apps/desktop-ui`（canvas transport 与 ViewCube 接线）。证据为本机编译/契约测试；**浏览器运行时、
像素、真机与 GPU 一律 NOT VERIFIED**。

## 1. 结论

- **起始页**：React 页面载体不再调用“打开内置夹具”的 `start`，而是调用新的
  `start_empty(canvas_id, css_w, css_h, dpr)`。wasm 侧仍构造同一个真实 `CadSession`（presenter、
  platform adapters、`.ymodel` archive 都在），但**不投递任何文档**；`shell_status`/事件快照
  不发出 `canvasSession`，chrome 因此显示 `StartPanel`。第一次 `OPEN` 走 `open_bytes`，是同一个
  会话的原地文档替换，不是第二个会话。
- **方位球**：`HostProfile::dom_canvas()` 不再声明 `standardViews: []` / `orbit: false`。它现在
  声明 wasm 会话真实执行的七个标准视图键与 `orbit: true`；`cad_standard_view` / `cad_orbit`
  仍由 `dom-canvas` 的 web adapter 转发给 wasm 会话。2D 视图下的 orbit 仍是会话自己的真实拒绝，
  选中三维标准视图后可拖动。
- 独立 smoke harness（`apps/app-web/web/main.js`、`scripts/web-smoke.cjs`）继续使用
  `start`（内置夹具），不受生产 React 载体影响。

## 2. 实现位置

| 位置 | 改动 |
|---|---|
| `crates/cad-ui-core/src/session/bytes.rs` | 新增 `CadSession::from_empty`（无文档会话；要求 `StartDocument::Empty` 且 `drawing: None`，安装同一 clock/archive/fonts/images） |
| `apps/app-web/src/live.rs` | 新增 `Live::install_empty`；无文档时 `shell_status_json` 省略 `canvasSession` |
| `apps/app-web/src/page.rs` | 新增 `start_empty_impl`/`start_empty_session`；presenter 构造抽成 `presenter_for`；事件快照同样省略无文档描述符 |
| `apps/app-web/src/lib.rs` | 新增 wasm 导出 `start_empty`（native 侧保留显式 NOT_IMPLEMENTED 桩） |
| `apps/desktop-ui/src/cad/canvas-host.ts` | 优先调用 `start_empty`；旧 artifact 无该导出时回退 `start` |
| `crates/cad-ui-core/src/session/profile.rs` | `dom_canvas`: `standardViews` = 七个真实键，`orbit = true` |
| `apps/app-linux/src/host/dispatch.rs` | `dom-canvas` profile 契约期望同步 |
| `scripts/web-ui-command-case.cjs` | 起始页无文档后，命令用例先通过真实 picker 打开授权夹具，再断言命令 |

## 3. 已运行

| 命令 | 结果 |
|---|---|
| `cargo test -p cad-ui-core --locked empty_session` | 2 passed；0 failed |
| `cargo test -p app-web --locked` | 152 passed；0 failed |
| `cargo check -p app-web --target wasm32-unknown-unknown --locked` | ok |
| `cd apps/desktop-ui && npm test` | 2110 passed；0 failed（含 `tsc --noEmit`） |
| `cargo test -p app-linux the_dom_canvas_status_uses_the_typed_host_profile --locked` | 1 passed；0 failed |

## 4. NOT RUN

- 浏览器真实运行（`scripts/build-web-ui.sh` + Chromium）本轮 **NOT RUN**：
  起始页可见性、ViewCube 点击/拖动、真实 GPU/像素与性能均无运行时证据。
- Windows/macOS/Android 运行时 **NOT VERIFIED**；只保留了编译/契约层面的共享 profile 改动。

## 5. 边界

本轮的 `standardViews`/`orbit` 改动是**能力声明的更正**：会话侧 `cad_standard_view` /
`cad_orbit` 早已经同一 wire 漏斗执行（`app-web` 双 sink 等价矩阵）。它不新增 3D 渲染能力，
也不改变 native 载体的“打开即 3D”默认；`dom-canvas` 文档仍以 plan 视图打开，3D 由用户显式选择
标准视图/拖动方位球进入。真实视觉与交互手感仍需浏览器/真机证据。
