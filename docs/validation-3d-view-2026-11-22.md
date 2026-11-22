# 验证记录：3D 默认观察 / FIT / 透视缩放 / 方位球（2026-11-22）

## 范围

- 打开含 3D 几何的文档后默认进入等轴测 3D（native 载体的共享打开漏斗）。
- 3D 下 `FIT` 保持当前 eye 方向与 `up`，target 移到包围盒中心（含 `z`），只重算距离；不再回 2D。
- 透视 `zoom_at` 固定 target 做 dolly，再在屏幕平面平移 eye+target 保持光标下世界点的像素；
  后续 orbit 围绕模型 target，不再被缩放推过模型。
- 导航方位球：ViewCube 从 56px 导航列移到绘图区独立浮层，改为不透明球面 + 六个方向帽；
  导航列宽度不变，点击标准视图 / 拖动 orbit / Home / 无障碍键 / 门禁不变。

## 实现落点

- `crates/cad-app/src/camera/view.rs` — 透视 `zoom_at` 的 dolly + 横向平移锚定。
- `crates/cad-app/src/app_zoom.rs` — `fit_active_space` 的 3D 分支（`Camera::enter_3d`）。
- `crates/cad-app/src/application_view.rs` — `apply_standard_view` 从 2D 进入时同时取拟合 target；
  `document_is_3d`（`Body3d` 或显著 z 范围）。
- `crates/cad-app/src/host/documents.rs` — `fit_opened_document`；`crates/cad-host/src/open.rs`
  四条成功打开路径改用它。
- `apps/desktop-ui/src/cad/viewcube-canvas.ts`、`components/ViewCube.tsx`、
  `components/Workspace.tsx`、`components/NavigationDock.tsx`、`viewcube.css`、
  `style.css`、`mobile.css` — 球面投影/拾取/绘制与独立浮层。

## 已运行

| 命令 | 结果 |
|---|---|
| `cargo test -p cad-app --locked` | 通过（3127 lib + 全部集成测试，0 failed） |
| `cargo test -p cad-host --locked` | 通过（451 passed，0 failed） |
| `cargo check -p app-linux --all-targets --locked` | 通过 |
| `cargo fmt --all -- --check` | 通过（改动文件已格式化） |
| `cargo clippy -p cad-app -p cad-host --all-targets --locked -- -D warnings` | 通过 |
| `python3 scripts/check-file-size.py` | 通过（2614 个代码文件，上限 1000 行） |
| `python3 scripts/check-architecture.py` | 通过（44 packages，无环、边界完整） |
| `python3 scripts/check-i18n.py` | 通过（3120 keys，两目录一致） |
| `cargo check -p cad-app -p cad-host -p cad-ui-core --target wasm32-unknown-unknown --locked` | 通过（1 个既有 `explicit_font_dir` dead-code warning） |
| `(cd apps/desktop-ui && npm test)` | 通过（tsc clean，1931/1931 node tests） |
| `(cd apps/desktop-ui && npm run build)` | 通过（仅既有 >500 kB chunk 警告） |
| `bash scripts/build-web-ui.sh` | **未完成**：`wasm-bindgen` CLI `0.2.129` 与 workspace 固定的 `0.2.130` 不一致；脚本在 wasm 产物阶段失败，本轮**未产出** web UI bundle。不是本轮改动导致。 |

## 未运行 / 未验证

- 全 workspace 测试、其它载体 crate、GPU/真实渲染、性能：**NOT RUN**。
- 真实 GUI 目视、真实 3D DWG 打开、真实鼠标/触控手感、像素：**NOT VERIFIED**。
- Windows / macOS / Android / Web 运行时：**NOT VERIFIED**；本轮只有 Linux 的
  `app-linux` 编译检查与共享层契约测试。Web/`dom-canvas` 仍是 2D-only。
- 自动进入 3D 后 `ZOOM PREVIOUS` 的跨文档历史行为未改变、未新增验证（沿用既有
  `fit_viewport` 记录一步的规则）。

## 证据边界

以上均为契约/单元/编译/前端测试证据，不是真机、真实 GPU、像素或性能验收。文档不把
“测试通过”写成“产品已验收”。
