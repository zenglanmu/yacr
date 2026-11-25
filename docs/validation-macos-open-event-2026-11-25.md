# macOS「打开方式」接线 + Android 文件关联产物复核（2026-11-25）

本轮范围：补上 macOS 的 OS 打开事件半程（`RunEvent::Opened` → 路径 → 同一条打开漏斗），并
复核 Android/其它载体的 OS 文件关联。权威仍是 `packaging/mime/formats.json` 与
`scripts/test-file-associations.py`；边界见 `AGENTS.md` 平台表与 `docs/macos-app.md`。

## 改动范围

- `crates/cad-host/src/open.rs`：新增 `HostCore::open_path`（`PathOpenOutcome::{Started,Busy}`），
  `open_configured_drawing` 改用它，`--open` 与运行时“打开方式”共用同一条漏斗。
- `crates/cad-ui-core/src/session/recent.rs`：`CadSession::open_path` 转发到 host core
  （与 `open_recent` 共用同一打开入口；放在该模块是为了不越过 `session/mod.rs` 的 1000 行门禁）。
- `apps/app-macos/src/open_event.rs`（新，**任意宿主可编译/单测**）：`file_url_to_path`
  （本地 `file:` URL、空 authority 或 `localhost`、percent 解码、拒绝远程/其它 scheme/畸形转义/
  非 UTF-8）与 `enqueue_bounded`（有界 FIFO，溢出丢最旧）。
- `apps/app-macos/src/host/open_event.rs`（新）：待开队列 + `dispatch`/`drain`；`host/tick.rs`
  每 tick drain（双击启动时 Opened 先于会话创建）。
- `apps/app-macos/src/entry.rs`：处理 `tauri::RunEvent::Opened { urls }`，非本地 URL 打
  `[probe] open event refused`，队列溢出打 `[probe] open event queue full`。
- `scripts/test-macos-host.py`：新增契约 (g)（事件处理 / URL 转换 / 模块声明 / tick drain）与三个
  自测变异。
- `docs/android-host.md`：关联计数从过期的「24 条 / 16 个扩展」改为现状（27 条 MIME 关联 +
  1 条 octet-stream 通配 = 28，19 个扩展，含 `ymodel`/`gltf`/`glb`）。

## 已运行（主机目标）

| 命令 | 结果 |
|---|---|
| `cargo test -p app-macos --locked` | **PASS**：43 passed, 0 failed（含 `open_event` 9 条） |
| `cargo test -p cad-host an_explicit_path --locked` | **PASS**：2 passed（Started + Busy） |
| `cargo test -p cad-host --locked` | **PASS**：477 passed, 0 failed（`open_configured_drawing` 重构回归） |
| `cargo test -p cad-ui-core --locked` | **PASS**：938 + 1 + 6 passed, 0 failed |
| `cargo check -p app-macos --all-targets --locked` | **PASS**（Linux 目标） |
| `CC_aarch64_apple_darwin=/bin/true AR_aarch64_apple_darwin=/bin/true cargo check -p app-macos --target aarch64-apple-darwin --lib --locked` | **PASS**（exit 0；C 工具链为桩，只影响 `objc2-exception-helper` 的目标文件，Rust 类型检查真实） |
| `cargo check -p cad-ui-core -p cad-host --lib --target wasm32-unknown-unknown --locked` | **PASS**（exit 0，仅既存 `dead_code` 警告） |
| `cargo clippy -p cad-host -p cad-ui-core -p app-macos --all-targets --locked -- -D warnings` | **PASS** |
| `cargo fmt --all -- --check` | **PASS** |
| `python3 scripts/test-macos-host.py` | **PASS**（含新 (g) 与自测变异） |
| `python3 scripts/test-file-associations.py` | **PASS**：`OK: 0 mismatches`（19 extensions / 27 MIME / 12 shared-mime-info） |

契约断言：

- `open_event::tests::*`（9 条）：纯路径转换（空 authority/`localhost`/percent/`+`/query/远程
  authority/畸形转义/非 UTF-8）与有界队列“溢出丢最旧”。
- `cad-host` `an_explicit_path_opens_through_the_platform_open_funnel`：`begin_open_target`
  收到调用方自己的路径；`an_explicit_path_while_a_load_runs_is_busy_and_is_not_started`：
  已有加载时返回 `Busy` 且不启动第二个平台打开。

## Android 复核（未重建产物）

源码正确：`apps/app-android/src-tauri/tauri.conf.json` 的 `bundle.fileAssociations` 有 28 条
（含 `application/x-revit` → `rvt`，octet-stream 通配列全 19 个扩展），已提交的
`gen/android/app/src/main/AndroidManifest.xml` 有 `application/x-revit` + `.*\.rvt`。

旧的 **release** APK（仍在 `build/outputs/apk/universal/release/` 下，未重建；2026-11-13 19:52
构建，早于关联扩容提交 `1530921b` 22:26）只注册了 DWG：

```
strings <旧 apk> AndroidManifest.xml -> image/vnd.dwg, application/{acad,dwg,x-dwg,autocad_dwg},
  application/octet-stream, .*\.dwg        （无 revit / 无其它扩展）
```

本轮重建（`WITH_FONTS=1` 首次因字体 CDN 无网络失败，改用离线字体包）：

| 命令 | 结果 |
|---|---|
| `WITH_FONTS=0 bash scripts/build-android.sh --debug --target aarch64` | **PASS**（exit 0）；`app-universal-debug.apk` 1,189,061,366 B，2026-11-25 22:51 |
| `aapt2 dump xmltree --file AndroidManifest.xml <apk>` | **28 个 `mimeType` / 48 个 `pathPattern`**；`application/x-revit` → `.*\.rvt`（另有 rte/rfa/rft、skp、dgn、step/stp 的 `application/octet-stream` 通配） |
| `git status --porcelain …/src/main/AndroidManifest.xml` | 空（tauri-build 重新生成与提交版本逐字一致，无漂移） |

`WITH_FONTS=0` 只影响打包的字体目录，与文件关联无关。APK 未安装、未在设备上打开任何文件。

## NOT RUN / NOT VERIFIED

- macOS 真机双击、“打开方式”菜单、LaunchServices 注册、`RunEvent::Opened` 的真实到达时机与
  队列行为：**NOT VERIFIED**（本机无 macOS；darwin 仅桩工具链类型检查）。
- Android：APK 已重建（debug、离线字体包）但**未安装**；设备 DocumentsUI 的“打开方式”是否
  列出 RVT、`ACTION_VIEW` 是否命中并读到文件仍 **NOT VERIFIED**。
- 未跑全量 workspace 测试、未跑 GUI/GPU/像素证据。
