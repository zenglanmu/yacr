# 宿主字体取字节（Web / Android）

本文记录 F10「宿主持有字体字节」的实现接线与验证边界。契约与字形来源见
`docs/fonts.md`；平台组装见 `docs/adr/0003-web-composition.md`。

## 分层

```
图纸 (DrawingDatabase)
  └─ requested_fonts()            cad-platform::fonts：只读收集引用键
       └─ plan_fonts()            cad-resources：名称 → 目录 face → URL
            └─ FontLoader          cad-platform：主机取字节
                 └─ FontEngine      cad-representation：按 encoding 注册
                      └─ set_fonts  cad-ui-slint：桥接重建场景并整形文字
```

`cad-platform::fonts::load_font_engine` 是 Web 与 Android 共用的编排；两个宿主只提供
`FontLoader` 实现，不复制计划/注册/回退逻辑（规范 §4.2 单一业务路径）。

## Web（`apps/app-web`）

- `WebFontLoader`：`window.fetch(url)` → 检查 `response.ok()` → `response.arrayBuffer()`
  → `Uint8Array::to_vec()`，转 `Arc<[u8]>`。CDN 跨域依赖其 CORS 头；非 2xx 记为
  `ResourceMissing`，绝不返回空字节。
- 目录：`cad_resources::DEFAULT_FONT_BASE_URL`（jsDelivr `mlightcad/cad-data`）。
- 触发：`open_document` 成功后 `spawn_font_load()`（`spawn_local`）异步加载；加载期间若
  又打开了别的图纸，用 `SceneIdentity` 比对丢弃过期结果（`StaleResult`）。
- 对外：`load_web_fonts()`（wasm-bindgen async，返回 `FontLoadReport::summary()`）与
  `font_load_report()`（最近一次结果）。`web/main.js` 暴露 `window.yacr.load_fonts()`
  与 `font_load_report()`，供无头验证真正等待下载完成。
- 注册为空 → `clear_fonts()`；否则 `set_fonts(engine)`。

## Android（`apps/app-android`）

- `android_main` 在任何 Slint 调用前 `app.asset_manager()`，存入线程局部；说明见
  android-activity 文档（返回的 manager 在进程内有效）。
- `AssetFontLoader`：`AssetManager::open(CStr)` → `read_to_end`。基址 `asset://fonts/`，
  目录 `asset://fonts/fonts.json`，即 APK 的 `assets/fonts/`。
- 触发：`open_drawing` 成功后同步加载（资产读取立即就绪，用 no-op waker 的执行器，
  不是通用 runtime）。
- **不打包字体**：许可原因（`docs/fonts.md`「授权」），未放 `assets/fonts/` 时返回
  `ResourceMissing`（`font asset not packaged …`），状态栏报告，绝不伪造。

## 验证

- 单元测试：`crates/cad-platform/src/fonts.rs`（`cargo test -p cad-platform`）覆盖
  - `requested_fonts` 收集 TEXT `font` 与样式 `resource_keys`；
  - `catalog_url` 规范化；
  - 目录/字体取字节可注入：注册失败被逐项报告而非计成功；
  - 未知名跳过（不伪造）；
  - 目录获取失败向上传播（`ResourceMissing`）。
- 编译：`cargo check --workspace --lib --target wasm32-unknown-unknown --locked`；
  `cargo check --target aarch64-linux-android -p app-android --locked`。
- 未做：浏览器内真实 `fetch` 与 Android 真机资产读取的运行验证（本环境无对应 harness /
  打包字体）。因此**不宣称** F10 已验收，只宣称代码路径已实现并编译、纯逻辑有测试。

## 已知限制

1. Android 只实现 asset 路径，运行时 HTTP 未实现。
2. Web 依赖 CDN CORS 与网络可达；离线时报告取字节失败，文本保持不可绘（不会静默丢字，
   因为未注册时桥接不注入字体）。
3. 缓存：当前每次打开重新取字节；按 URL 的跨会话缓存/持久化（IndexedDB 等）未实现。
