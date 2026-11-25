# Web/`dom-canvas` `.ymodel` 打开与保存（2026-11-25）

本轮范围：让 `apps/app-web` 的 wasm CAD 会话安装真实的 `.ymodel` 归档适配器，使浏览器
（以及同一 wasm 会话的 Linux X11/XWayland `dom-canvas`）可以打开页面上传的 `.ymodel`，并让
`SAVE` 把编码后的归档作为浏览器下载落盘。决策见
[ADR-0060](adr/0060-web-ymodel-archive.md)；平台边界见 `docs/compatibility.md` 与
`AGENTS.md` 平台表。

## 改动范围

- `crates/cad-ui-core/src/session/bytes.rs`：`BytesDocument` 增加 `archive` 字段与
  `new`/`with_archive`，`from_bytes` 在投递首个文档**之前**安装归档。
- `apps/app-web/src/ymodel_archive.rs`：`ZipArchive`（`cad-store-zip` 的
  `MemorySource`/`MemorySink`），与 Linux 载体同容器实现，无文件/临时文件/`.bak`。
- `apps/app-web/src/live.rs`：生产 wasm 会话构建时安装该适配器。
- `apps/app-web/src/download.rs`：`begin_save` 每次生成新的
  `download://<seq>/<name>` 句柄（浏览器保存永不原地覆盖）；`.ymodel` 下载 MIME 为
  `application/x-yacr-ymodel`。
- `apps/app-web/src/direct.rs`：`cad_ymodel_resource_preview` /
  `cad_set_embed_resources` 改为 `unsupported-runtime: embed_resources`（归档已支持；页面仍
  读不到本地外部资源）。
- `apps/app-web/Cargo.toml`：增加 `cad-store-zip`（仍只由 `apps/app-*` 载体链接）。

## 已运行（主机目标 / 契约）

| 命令 | 结果 |
|---|---|
| `cargo test -p app-web --locked -- --test-threads=1` | **PASS**：151 passed, 0 failed |
| `cargo check -p app-web --target wasm32-unknown-unknown --locked` | **PASS**（仅有既存 `dead_code` 警告） |
| `cargo clippy -p app-web -p cad-ui-core --all-targets --locked -- -D warnings` | **PASS** |
| `cargo fmt -p app-web -p cad-ui-core -- --check` | **PASS** |
| `python3 scripts/check-architecture.py` | **PASS**（38 packages） |
| `python3 scripts/check-file-size.py` | **PASS** |
| `python3 scripts/test-file-associations.py` | **PASS**（0 mismatches） |
| `(cd apps/desktop-ui && npm test)` | **PASS**：2096 tests, 0 failed |
| `bash scripts/build-canvas-wasm.sh web-ui` | **PASS**：slim 产物 `app_web_bg.wasm` **13,281,659 字节**（wasm-opt -Oz；旧基线 11,680,561），仍低于 20 MiB；仅为构建产物证据 |

关键契约断言（`apps/app-web/src/platform/tests.rs`）：

- `save_without_an_archive_still_refuses_by_name_and_writes_nothing`：没有归档的字节会话仍以
  `unsupported-runtime: ymodel` 拒绝且不产生下载。
- `save_with_the_archive_downloads_a_reopenable_ymodel`：`SAVE` 产生
  `synthetic-four-lines.ymodel` 下载，字节以 `PK\x03\x04` 开头；第二次 `SAVE` 仍产生新下载；
  下载字节作为**新会话的首个文档**打开，保留 4 个 fixture 实体。

## NOT RUN / NOT VERIFIED

- **浏览器内**上传 `.ymodel` 后打开、`SAVE` 下载真正落到用户下载目录、重复保存、刷新后行为：
  **NOT RUN**。本轮只有主机目标契约与 wasm 编译证据。
- WebKitGTK/Tauri WebView（Linux X11 `dom-canvas`）的下载落盘路径：**NOT VERIFIED**。
  `DomDownload` 只证明 `Blob` + `<a download>` 的 DOM 手势成功，不证明平台保存了文件。
- 浏览器内大归档的内存/时间预算、真实 GPU、像素、性能：**NOT RUN**。
- 外部资源嵌入（font/xref/image）在 Web/X11 不提供；已内嵌资源随文档保留，但未做浏览器级
  往返验证。
- 没有新增 `.bak`、原子替换或恢复快照；不得把浏览器下载描述成原子保存。
