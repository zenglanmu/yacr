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
`FontLoader` 实现，不复制计划/注册/回退逻辑（规范 §4.2 单一业务路径）。当图纸字体缺失
或拉取失败时它会注册一个**默认轮廓回退面**（`DEFAULT_FALLBACK_NAMES`：
`osifont` → `arial` → `simplex`），`FontLoadReport.default_face` 记录实际注册项。

Linux 桌面/CLI 宿主复用同一个 `load_font_engine`，只是 `FontLoader` 换成
`cad-platform::fonts::local::DirFontLoader`（读取可执行文件同级 `fonts/` 目录）；
随后把 `--font` 显式字体与系统默认字体（`register_default_face`）并入同一引擎。

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
- **不打包 mlightcad 字体**：许可原因（`docs/fonts.md`「授权」），未放 `assets/fonts/` 时返回
  `ResourceMissing`（`font asset not packaged …`），状态栏报告，绝不伪造。但
  `scripts/fetch-fonts.sh` 会把提交的 `fonts/`（QCAD osifont，GPL-3+例外）合并进
  `assets/fonts/`，保证 APK 至少自带给默认轮廓回退面。

## Linux 桌面 / CLI（`cad-platform::fonts::local`）

- 启动（桌面）或每次打开图纸后（两者都）重新收集 `requested_fonts`，按序组装引擎：
  1. 可执行文件同级的 `fonts/` 目录（`fonts.json` + 文件；`--fonts-dir` 可显式指定，
     显式目录不存在是错误，不静默回退）；
  2. `--font NAME=PATH` 显式注册；
  3. 系统默认字体（fontconfig `fc-match`，跳过 `.ttc` 等无法解析的候选，`__yacr_default__`
     为第一回退）。
- 空结果 → `clear_fonts()`；否则 `set_fonts`。没有可用字体时文本保持不可绘的 `Text`，
  绝不伪装成已渲染。
- 打包：`scripts/package-linux-release.sh` 把 `fonts/` 复制到发布包二进制同级。

## 验证

- 单元测试：`crates/cad-platform/src/fonts.rs`（`cargo test -p cad-platform`）覆盖
  - `requested_fonts` 收集 TEXT `font` 与样式 `resource_keys`；
  - `catalog_url` 规范化；
  - 目录/字体取字节可注入：注册失败被逐项报告而非计成功；
  - 未知名跳过（不伪造）；
  - 目录获取失败向上传播（`ResourceMissing`）。
- 编译：`cargo check --workspace --lib --target wasm32-unknown-unknown --locked`；
  `cargo check --target aarch64-linux-android -p app-android --locked`。
- **浏览器内编排运行**（web 工作流，2026-10-02，SwiftShader WebGL2）：在真实 headless
  Chromium 中调用 `window.yacr.load_fonts()` → `load_web_fonts()`；演示图未引用 CAD
  文本字体，返回 `catalog=0 requested=0 planned=0 registered=0 failed=0`，宿主路径可
  解析。明细见 `docs/validation-web.md`。
- 未做：浏览器内**真实** `fetch` 与 Android 真机资产读取的运行验证。上面的浏览器运行
  走的是“图纸未引用字体”分支，因此未真正下载 jsDelivr 目录/字体字节，也不构成 Android
  或 CDN 的验证。因此**不宣称** F10 已验收，只宣称代码路径已实现并编译、纯逻辑有测试、
  宿主编排在浏览器中可运行。

## 已知限制

1. Android 只实现 asset 路径，运行时 HTTP 未实现。
2. Web 依赖 CDN CORS 与网络可达；离线时报告取字节失败，文本保持不可绘（不会静默丢字，
   因为未注册时桥接不注入字体）。
3. 缓存：当前每次打开重新取字节；按 URL 的跨会话缓存/持久化（IndexedDB 等）未实现。
