# 天正语义持久化与完整打开（ADR 0030 D7–D9，P0）

- 日期：2026-11-23。
- 范围：`plugins/tch` 的 `YANE/YANS` 语义帧与 P0 提取、`.ymodel` host-owned
  `tables/vendor.json`（minor 12）、打开管线 `openProgress`/`documentReady`/
  `document-ready`、SAVE/EXPORT/PLOT 门禁。
- 状态：**P0 已实现；P1/P2 未实现**（见文末）。
- 证据边界：Linux 契约/单测；Windows/macOS/Android/Web 只到编译或契约级。
  **没有真机、真实 GPU、像素、性能或 vendor 验收**；真实 DWG→ymodel→重开仍未运行。

## 1. 已接线

- **持久化**：`PersistenceState.vendor_rows` 是文档槽上的 host-owned 语义行；
  `snapshot_document` 仅在非空时写 `tables/vendor.json`（minor 12）；
  `install_snapshot` 读回并按 entity id 过滤 orphan。
- **插件两段式**：`extractSemantics(YAND) -> YANE`（import 一次）与
  `decode(YANS) -> YANR`（reopen）；`decode(YAND)` 帧路径不变。
  P0 类：`TCH_WALL` / `TCH_OPENING` / `TCH_TEXT` / `TCH_MTEXT` / `TCH_BLOCK_INSERT`。
  其余类显式 `status: unsupported` + `vendor.tch.semantics_not_implemented`。
- **宿主应用**：`YANE` 行在 owner tick 解析 `source.handle → EntityId` 后写入文档槽；
  语义应用与 primitive 显示独立（坏语义不阻断显示，不写半张表）。
- **打开管线**：`cad_status.openProgress`（import → vendor → representations）、
  `documentReady`、`openSummary`；`document-ready` 在 `vendor-settled` 之后发出。
- **门禁**：SAVE/EXPORT/PLOT 在 `documentReady = false` 时以
  `save.document_not_ready` 拒绝；`loading` 仍只管 picker/import，NEW/OPEN 可用；
  恢复快照在未 ready 时不写。
- **超时**：WebView pass 有 30 s JS 上限，失败以真实 unsupported result settle；
  前端异常也构造真实 unsupported 帧，不永远 pending。

## 2. 已运行

| 命令 / 套件 | 结果 |
|---|---|
| `cargo test -p cad-store-zip --locked` | 通过（含 `tests/vendor.rs`：minor 12、无行不写表、旧 reader 拒绝） |
| `cargo test -p cad-app --lib host::tests::vendor_rows` | 通过（行 round-trip、handle→entity、YANS 只取一次） |
| `cargo test -p cad-app --lib host::vendor_semantics` | 通过（schema/ref 诊断） |
| `cargo test -p cad-ui-core --locked` | 860 通过 / 1 ignored（含 YANE/YANS 校验、openProgress、document-ready 派生） |
| `cargo test -p cad-host --locked` | 465 通过 / 1 ignored（含 SAVE 门禁移动到 `tests/save_gate.rs`） |
| `cargo test -p cad-desktop-shell --locked` | 通过（含 vendor status overlay / `document-ready`） |
| `cargo test -p app-web --locked` | 148 通过 |
| `cargo clippy -p cad-app -p cad-ui-core -p cad-host -p cad-desktop-shell -p app-web --all-targets --locked -- -D warnings` | 通过 |
| `cargo check --workspace --all-targets --locked` | 通过 |
| `cargo check --workspace --lib --target wasm32-unknown-unknown --locked` | 通过（既有 dead-code warning 保留） |
| `cargo check -p app-android --locked` | 通过（宿主 target） |
| `(cd plugins/tch && npm run typecheck && npm run build)` | 通过 |
| `(cd apps/desktop-ui && npm test)` | 1965 通过 |
| `(cd apps/desktop-ui && npm run build)` | 通过 |
| `python3 scripts/check-file-size.py` / `check-i18n.py` / `check-architecture.py` | 通过 |
| `cargo fmt --all -- --check` | 通过 |

## 3. NOT RUN / 未验证

- **真实 DWG → .ymodel → 重开**：没有用 `test-tangent-real.dwg` 跑完整 UI 往返；
  P0 的 primitive 哈希一致性只有 TS 合成帧与 Rust 帧/表单测，不是真实样本证据。
- **GUI/像素/GPU/性能**：全部 NOT RUN。
- **Windows/macOS/Android/Web 运行时**：只有编译/契约级；真实 WebView/SAF/WKWebView/
  WebView2 未验证。
- **TCH base 9–14 / T20/T30 cipher / proxy retained data**：仍 NOT VERIFIED，解码器未扩展。
- **P1**：其余已支持类的语义、`attributes` 真实 property bag、copy/paste hook、
  CLI `unsupported-runtime: vendor` 未实现。
- **P2**：partial/unsupported 行的 `reimport_required` raw fallback、语义 schema 迁移、
  体积预算调优未实现。
- **全量 workspace 测试**：`cargo test --workspace --locked` 启动后 23 分钟未完成（无输出），
  已中止（SIGTERM）；**不作为通过证据**。本轮按 AGENTS 只验证改动范围的 crate 测试。

## 4. 平台/载体说明

- `native-below`（Linux）走 `cad-desktop-shell::vendor::tick`：优先 `YANS`，否则 records；
  应用 settled semantics/primitive 并投影 `documentReady`。
- `dom-canvas`（Linux X11/XWayland、Web）在 wasm 会话内同一 `VendorStore`/帧校验；
  `vendor_apply` 立即应用语义并 flush `vendor-settled` + `document-ready`。
- CLI/headless 无 WebView：不产生语义；不得声称 TCH 已支持。
