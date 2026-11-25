# ADR-0060：Web/`dom-canvas` 载体的 `.ymodel` 打开与保存

- 状态：已接受（2026-11-25）
- 关联：ADR-0037（`.ymodel` 容器与自包含）、ADR-0040（内嵌外部资源）、ADR-0041（SAVE/EXPORT
  分离）、ADR-0024（`dom-canvas` 载体）

## 背景

ADR-0037 D16 把 `.ymodel` 的编解码放在 `cad_platform::DocumentArchive` 后面，由载体链接
`cad-store-zip`；Linux/Windows/macOS/Android 各自实现并安装它。Web（`apps/app-web` 的 wasm
会话，同时服务浏览器与 Linux X11/XWayland 的 `dom-canvas`）此前没有实现，`SAVE` 与
`.ymodel` 打开都以 `unsupported-runtime: ymodel` 拒绝，写文件只能走 `EXPORT`。

这把 Yacr 自己的文档格式排除在 Web 之外，也与 ADR-0037 的“wasm 侧同格式”目标不一致。
浏览器确实没有文件系统，但它有**上传**与**下载**两个真实能力：打开时页面已经把文件字节交给
`open_bytes`，保存时 `BrowserFiles`/`DomDownload` 已经把导出字节发布成 `Blob` +
`<a download>`。缺少的只是容器编解码与安装时机。

## 决策

### D1 载体实现同一个 `DocumentArchive`

新增 `apps/app-web/src/ymodel_archive.rs` 的 `ZipArchive`：与 Linux 载体相同的
`ZipDocumentStore` + `MemorySource`/`MemorySink` 实现，不引入文件、临时文件、`fsync`、
`.bak` 或重命名。`cad-store-zip` 只由 `apps/app-*` 载体链接（`check-architecture.py`），
本改动不改变该边界。

### D2 归档在**首个文档投递之前**安装

`BytesDocument` 增加 `archive: Option<Arc<dyn DocumentArchive>>`，`CadSession::from_bytes`
在 `deliver_external_open` 之前调用 `core.set_document_archive(...)`。否则页面先上传
`.ymodel` 再 `start` 时，会话尚未拿到适配器，首个文档会被当成“无归档”拒绝。`None` 保持
原来的具名拒绝，供没有归档的字节载体使用。

### D3 保存是浏览器下载，不是原地写

`SAVE` 仍只在会话内编码 `.ymodel`，然后经现有 `BrowserFiles` 发布下载。`begin_save` 为每次
保存生成**新的** opaque 句柄 `download://<seq>/<name>`：

- 浏览器没有可写回的文件，也没有 `.bak`/恢复快照；每次保存都应是新下载（浏览器自身可能
  改成 `plan (1).ymodel`），因此让宿主的 in-place 规则永不命中；
- 若沿用固定 `download://<name>`，第二次 `SAVE` 会与上次保存记录的句柄相同，宿主会尝试
  `read_target`（该载体默认拒绝），把一次正常保存变成 `unsupported-runtime: read_target`。

### D4 外部资源嵌入问题仍不提供

Web/X11 `dom-canvas` 读不到本地目录，无法解析待打包的 font/xref/image。`cad_ymodel_resource_preview`
与 `cad_set_embed_resources` 都以 `unsupported-runtime: embed_resources` 拒绝（不再是
`unsupported-runtime: ymodel`，因为归档本身已支持）；保存对话框因此跳过该问题。已打开归档里
**已经内嵌**的资源仍随文档存在，保存不会丢失它们。

### D5 证据边界

- 本轮只证明主机目标契约：`cargo test -p app-web` 覆盖归档读写、`SAVE` 产生 `.ymodel`
  下载、第二次 `SAVE` 仍是新下载、下载字节作为新会话的首个文档重新打开并保留实体；
  `cargo check -p app-web --target wasm32-unknown-unknown` 证明 wasm 可编译。
- **浏览器内**的上传→打开、下载真正落盘、重复保存、真实 GPU/像素一律 **NOT RUN**；
  WebKitGTK/Tauri WebView 的下载目标行为也 **NOT VERIFIED**。
- 不新增 `.bak`、原子替换或恢复快照；不得把浏览器下载写成“原子保存”。

## 后果

- `SAVE` 在 Web 与 X11 `dom-canvas` 上可用，写出的 `.ymodel` 与原生载体同容器、可互相打开；
- `EXPORT`/`PLOT` 的下载通道不变；无归档的字节载体仍以 `unsupported-runtime: ymodel` 明确拒绝；
- 归档使 wasm bundle 增加 `cad-store-zip` 的编解码代码，仍受 slim 产物的 20 MiB 预算与
  发布 25 MiB 上限约束；浏览器运行时与下载落盘需要单独取证。
