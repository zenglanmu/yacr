# ymodel Phase 2 验证记录（2026-11-19）

范围：ADR-0037 Phase 2 的**脏分片 raw-copy**、宿主 `ChangeScope` 累积、文档自身归档字节保留，
以及本轮全量回归中暴露并修复的两处既有漂移（`app-web` 的 tauri-only wire 列表、`.ymodel`
跨载体文件关联）。环境：Linux x86_64，Rust 1.99.1（`rust-toolchain.toml`），全部命令
`--locked`。**本文只记录实际执行过的命令**；未执行的项在末尾显式列出。

## 1. 全量测试

| 命令 | 结果 |
|---|---|
| `cargo test --workspace --locked -- --test-threads=1` | **exit 0**；**7968 passed / 0 failed / 39 ignored** |
| `cargo test -p cad-store-zip --locked` | 29 unit + 17 archive + 7 fixed-point + **6 raw-copy**，全绿 |
| `cargo test -p cad-db --locked` | 326 passed（其余二进制 7 / 326 等，全绿） |
| `cargo test -p cad-app --locked` | 3049 passed，3 ignored |
| `cargo test -p cad-host --locked` | 398 passed（含 `tests/ymodel.rs` 11 项） |
| `cargo test -p app-web --locked` | 132 passed |
| `(cd apps/desktop-ui && npm test)` | 1822 pass / 0 fail |

第一次全量运行在 `app-web` 失败：`the_wire_command_set_is_the_registrys_shared_set` 的硬编码
tauri-only 列表仍是 5 行，而注册表已因恢复快照增加 `cad_recovery_files` / `cad_open_recovery`
变为 7 行。此为本轮之前提交遗留的断言漂移，已更新断言；随后全量 0 失败。

## 2. 编译与 lint

| 命令 | 结果 |
|---|---|
| `cargo check --workspace --all-targets --locked` | exit 0 |
| `cargo check --workspace --lib --target wasm32-unknown-unknown --locked` | exit 0（`app-web` 1 条既有 dead_code 警告） |
| `cargo check -p app-linux --all-targets --locked` | exit 0 |
| `cargo check -p app-windows -p app-macos --all-targets --locked` | exit 0（Linux 上可编译；这两个载体的运行仍未验证） |
| `cargo clippy --workspace --exclude app-android --exclude app-web --all-targets --locked -- -D warnings` | exit 0 |
| `cargo clippy -p app-windows -p app-macos --all-targets --locked -- -D warnings` | exit 0（picker 断言更新后复跑） |
| `cargo fmt --all -- --check` | exit 0 |
| `rustfmt --edition 2021 --check apps/app-windows/src/platform.rs apps/app-macos/src/platform.rs` | exit 0 |

## 3. 静态门禁与契约脚本

| 命令 | 结果 |
|---|---|
| `python3 scripts/check-architecture.py` | exit 0（39 packages） |
| `python3 scripts/check-tauri-capabilities.py` | exit 0 |
| `python3 scripts/check-fixture-manifest.py` | exit 0（47 fixture） |
| `python3 scripts/check-file-size.py` | exit 0（2382 文件，≤1000 行） |
| `python3 scripts/check-i18n.py` | exit 0（2976 keys） |
| `python3 scripts/check-command-docs.py` | exit 0（0 mismatches） |
| `python3 scripts/test-check-command-docs.py` | 7 tests ok |
| `python3 scripts/check-workflows.py` | exit 0 |
| `python3 scripts/test-*.py`（全部 21 个） | 全部 exit 0 |

`test-file-associations.py` 第一次失败：`apps/app-linux/src/platform.rs OPEN_EXTENSIONS` 含
`ymodel`，但单一权威 `packaging/mime/formats.json` 没有登记，其它载体也未转录。此为该功能
早先提交遗留的漂移。已把 `.ymodel` 登记为 `application/x-yacr-ymodel`（authority + Linux
desktop/metainfo/mime XML + 三端 picker + Web picker + Android conf/committed manifest +
macOS Info.plist heredoc）；gate 现为 `OK: 0 mismatches`。同时更新了 `apps/app-macos` 与
`apps/app-windows` 中两处按旧顺序断言的 picker 单测（`ymodel` 现在是数组首项）；两 crate 的
`--all-targets` 编译通过，运行测试仍只在各自宿主/CI 上执行。

## 4. 本轮实现的证据边界

**已由测试证明（库层 + 宿主契约）**

- 未触及分片按压缩字节 raw-copy，脏分片重编码，`dirty.prev_shards` 抑制复制，删除分片不复活，
  首次保存无 previous 时全量编码（`crates/cad-store-zip/tests/raw_copy.rs` 6 项）。
- 复制条目的 manifest `sha256`/`len`/`crc` 来自源清单且重开校验通过；源清单被篡改时重开以
  `ymodel.invalid_document` 失败（不被盲信）。
- 宿主累积每个 commit 的 `ChangeScope`（命令、traced interaction、plugin commit、engine funnel），
  `mark_saved` / Open / New / Close 清空；第二次保存只携带第二次编辑的范围；layer rename（空
  `changes` 路径）标记 `TableKind::Layers` 并在保存/重开后仍在；Save As 保留源归档字节与范围。
- 保存的 `previous` 是文档**自身**的归档字节，不是保存目标（`PersistenceState::archive_bytes`）。

**NOT RUN / 未实现（不得据此宣称完成）**

- host `FakeArchive` → 真实 `cad-store-zip` 的端到端 narrowed-save（两侧分别有契约，组合未跑）。
- ADR-0037 D17 的性能门槛：100k 元素全量保存、200 MB raw-copy 保存、峰值磁盘、常驻内存。
- 真实 DWG/DXF → ymodel → 原格式的 **L1 不动点**；`tables/sources.json`、`tables/compat.json`
  报告落盘；`raw/source-*.bin` 与 **L0 源字节直通**（Phase 2 余项 / Phase 3）。
- 真实窗口内 open/save、kill -9 崩溃窗口、`.bak` 实际恢复、`.tmp` 孤儿清理。
- Windows / macOS / Android / Web 的 ymodel 归档打开/保存：仍无适配器，显式
  `unsupported-runtime: ymodel`；这些载体的运行、真机、GPU、像素、性能一律 **NOT VERIFIED**。
- 全量测试中 39 项 ignored 的逐项原因未审计（既有）。
