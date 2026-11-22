# Web slim bundle 验证（2026-11-22）

目标：`web-build` 连续失败在 Cloudflare Pages 单文件 25 MiB 上限（`wasm-opt -Oz` 后 28,236,131 B）。
本轮先保留 `[profile.wasm] opt-level = "z"`，再把**部署到浏览器的** wasm 连接器集收敛到 DWG/DXF，
并把余量固化成构建门禁。

## 体积（同一棵工作树，`wasm-opt -Oz --skip-pass=merge-similar-functions --strip-debug --strip-producers`）

| 构建 | 产物 | 结果 |
|---|---|---|
| `opt-level = "s"`（旧） | 28,236,131 B（CI run 45021048920）/ 28,234,659 B（本地） | 超 26,214,400 B 上限 → `web-build` 失败 |
| `opt-level = "s"` + `-Oz --converge`（实验） | 28,220,150 B | 收益 < 15 KB，放弃 |
| `opt-level = "z"`（全连接器） | 24,166,298 B（CI run 45027532154）/ 24,164,727 B（本地） | 合规，余量 ~2.0 MiB |
| `opt-level = "z"` + slim（DWG/DXF） | **11,680,561 B**（本地 `WITH_FONTS=0 bash scripts/build-web.sh`） | 距 25 MiB 发布上限 ~13.9 MiB，距新增 20 MiB 预算 ~8.3 MiB |

slim 的 bindgen 输出（未优化）为 22,793,732 B，`wasm-opt` 后 11,680,561 B。

## 改了什么

- `cad-app`：格式适配器依赖改为 `optional`，新增 `formats-all`/`formats-mesh`/`formats-ifc`/
  `formats-step`/`formats-dgn`/`formats-rvt`/`formats-skp`/`formats-gltf`（`default = ["formats-all"]`）；
  `connector.rs` 对 descriptor/static/probe/exporter/lane/refusal 逐项 `cfg`，新增
  `absent_import_reason`（对 `DrawingFormat` 穷尽 match）、`absent_export_reason`、`KNOWN_EXPORT_KEYS`、
  `unclaimed_input()`、`ALL_FORMATS_COMPILED`。被编译掉的格式答 `unsupported-runtime: import.<id>`
  （OLE 归 `import.ole`）/ `export.<format_id>`；`step`/`dgn`/`rvt` 不再退化成 `export.unknown_format`。
- `app-web`：对 `cad-app` 的边改 `default-features = false`（本 workspace 是 edition 2021，
  `workspace = true` 边上的 `default-features` 会被 Cargo 忽略，故写成显式 path 边）；新增
  `full-formats = ["cad-app/formats-all"]`。`cad-engine`/`cad-ui-core`/`cad-host` 的边同步关默认，
  `cad-host` 增加 `[dev-dependencies] cad-app` 让夹具测试仍有全格式 lane。
- 门禁：`scripts/lib/wasm-toolchain.sh` 在 `cargo build` 后跑两条 `cargo tree` 泄漏守卫（`formats-*`
  泄漏、以及 `rvt`/`cadmpeg-*`/`ifc-lite-*`/`skpwright`/`cadkit-*`/`gltf`/`image` 被链入），在
  `wasm-opt` 后检查 20 MiB 预算；`scripts/test-file-associations.py` + `packaging/mime/formats.json`
  把 web picker 的证据收窄成 `webOpenExtensions = ["ymodel","dwg","dxf"]`，并静态断言 `app-web` 的
  `cad-app` 边没有 `formats-*`。
- 载体面：`apps/desktop-ui/src/host/web-adapter.ts` 的 `WEB_OPEN_EXTENSIONS`、`apps/app-web/web/index.html`
  的独立 harness picker；原生载体清单与 i18n 未改。

## 跑过的门禁

- `cargo test -p cad-app --locked`：3134 passed；`cargo test -p app-web --locked`：148 passed
  （含 `connector_tests::the_browser_registry_keeps_the_dwg_and_dxf_lanes`）；
  `cargo test -p cad-host --locked`：457 passed。
- `cargo check --workspace --all-targets --locked`、`cargo check -p app-web --target
  wasm32-unknown-unknown --locked`、`cargo check -p app-web --target wasm32-unknown-unknown
  --profile wasm --features full-formats --locked`：通过。
- `cargo clippy -p cad-app --all-targets --locked -- -D warnings`、`cargo fmt --all -- --check`：通过。
- `python3 scripts/check-architecture.py`、`check-file-size.py`、`check-i18n.py`、`check-workflows.py`、
  `check-fixture-manifest.py`、`test-file-associations.py`：0 mismatch。
- `(cd apps/desktop-ui && npm test)`：1949 passed / 0 failed。
- `WITH_FONTS=0 bash scripts/build-web.sh`：slim 产物 11,680,561 B，两条泄漏守卫与 20 MiB 预算通过。
- `cargo tree -p app-web --target wasm32-unknown-unknown --edges features,normal,build -i cad-app
  --locked --offline`：无 `formats-*`。

## NOT RUN

- 浏览器本地加载 slim bundle（`scripts/web-smoke.cjs` 本轮未跑）；由 CI `web-smoke`（SwiftShader，
  软件渲染，不是 GPU/像素证据）在推送后验证。
- `web-deploy`（仅 `v*` tag / 手工 dispatch）。
- `cargo test --workspace`（本轮只跑改动范围的 crate + workspace `--all-targets` check）。
- 桌面 `dom-canvas` full 变体的完整 `scripts/build-canvas-wasm.sh`（只做了 `cargo check --features
  full-formats`）；真实 X11/GPU 运行未重跑。
- `opt-level = "z"` 相对 `"s"` 的运行时性能差异（未测量；web 端不作为性能证据）。

## 边界

- 裁剪只作用于**部署的浏览器 bundle**；原生载体与 Linux X11/XWayland 的 `dom-canvas`
  （`scripts/build-canvas-wasm.sh` → `full-formats`）仍编译全部格式。
- 拒绝是稳定标记，不是空成功；`descriptors()` 不广告未编译的 lane；页面 picker 与
  `formats.json` 的 `webOpenExtensions` 保持一致。
