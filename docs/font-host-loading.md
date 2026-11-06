# 宿主字体加载（native / Android / Web）

本文只描述**当前** React/Tauri/wasm 载体如何把字体字节变成 `FontEngine`。字体来源、授权、
格式、回退顺序与 `cad_fonts` 字段见 [fonts.md](fonts.md)；逐宿主平台状态见
[android-host.md](android-host.md)、[windows-app.md](windows-app.md)、[macos-app.md](macos-app.md)。

## 共同契约

```
DrawingDatabase
  └─ requested_fonts() / needs_text_shaping() / code_page()
       └─ plan_fonts()          cad-resources：名称 → 目录 face → URL
            └─ FontLoader       cad-platform：宿主提供字节
                 └─ FontEngine  cad-representation：按 encoding 注册
                      └─ set_fonts(Some(engine)) / set_fonts(None)
```

`cad-platform::fonts::load_font_engine_for_locale` 是共同编排：读 `fonts.json`、计划、取字节、
注册、按 locale 建回退链，返回 `(Arc<FontEngine>, FontLoadReport)`。宿主只负责取字节；计划/
注册/回退不在各平台重写。**空引擎用 `set_fonts(None)`**，不安装一个所有字形都会失败的空引擎。

## 原生宿主（Linux / Windows / macOS 桌面与 CLI）

`cad-host` 的字体策略：

1. 从 `cad-db` 只读收集本图纸的 `requested_fonts`、`needs_text_shaping` 与
   `TextLocale::from_code_page(code_page())`。
2. 若当前 installed engine 的 locale 匹配、覆盖所有请求面且无 locale 缺口，直接复用；否则若新
   locale 的优先面已在 engine 中，就地重定向回退顺序；否则重建。
3. 重建时：
   - `cad-platform::fonts::local::resolve_font_dir(options.fonts_dir, current_exe())`：
     显式 `--fonts-dir` 必须存在；否则依次找可执行文件同级 `fonts/`、父级 `fonts/`、macOS
     `Contents/Resources/fonts`、CWD `fonts/`。
   - `DirFontLoader` 从该目录读字节；`load_engine_report` 再注册 `--font NAME=PATH`，最后注册
     第一个可解析的系统默认面（`fc-match`；Windows/macOS 从系统字体目录收集候选）。
   - 无字体目录时仍会按 locale 记录缺口（`font.locale_fallback_missing`），不把“没目录”伪装成
     已满足。
4. `FontLoadReport` 同时发布给 `cad_fonts`；失败面保留真实状态。

CLI（`cad-cli-tools`）复用同一 `local` 模块：`--font name=path` + 可执行文件同级 `fonts/`；
`build-representation` / `render` / `plot` 都注入同一个引擎。

## Android（`native-below`）

- Gradle 将仓库 `fonts/` 作为 APK asset 根；`MainActivity.kt` 在后台把
  `assets/fonts/…`（或 asset 根）复制到 `filesDir/fonts/`，完成后写
  `.yacr-assets-v2-complete`。标记存在且 `fonts.json` 存在时后续启动跳过复制；v2 标记确保旧安装
  在加入 Noto 后重新解压。
- 复制成功后通过冻结的 JNI `nativeAssetsReady(fontsDir, imagesDir)` 把**真实文件系统路径**交给
  Rust；`HostOptions.fonts_dir` 指向它。没有字体资产时只记录显式诊断，**不**调用 JNI 传入空包。
- Rust 侧与桌面相同：`cad-platform::fonts::local` 读该目录，按图纸 locale 构建引擎。Android 上
  **没有** wasm `CadSession`，也不会走 Web 的 `set_font_package`。

## Web（`dom-canvas`，wasm 会话）

Web **不在 bundle 里带字体，也不预加载**：包在文档打开**之后**按需从同源 `fonts/` 取。
`HostResources::install_fonts` 仍是**同步**的，所以页面必须先取字节再装包；懒加载把这一步移到
`open_bytes` 之后，再由 `apply_font_package` 在已打开的会话上重跑安装。

1. **构建期**：`scripts/build-web-ui.sh`（`dist-ui`）与 `scripts/build-web.sh`（独立 harness
   `dist`）默认（`WITH_FONTS=1`）经共享 `scripts/lib/font-package.sh` 调用
   `scripts/fetch-fonts.sh`，把 mlightcad 全量 + 提交包 staged 到 bundle 的 `fonts/`；
   `WITH_FONTS=0` 只放提交包、`FONTS_DIR=<dir>` 复用预置目录（都不联网）。页面仍在 `open_bytes`
   之后才按需取面。`apps/desktop-ui/vite-font-package.mjs` 只在 `__YACR_RUNTIME__=tauri` 时把
   字体包复制到 `apps/desktop-ui/dist/fonts`（X11/XWayland `dom-canvas` 的离线包）；发布打包用
   `YACR_FONT_PACKAGE_DIR` 换成全量目录；web/android 的 UI bundle 都跳过。
2. **发布包**：`scripts/deploy-cloudflare-pages.sh` 在上传前调用 `scripts/fetch-fonts.sh` 把
   mlightcad 全量目录 + 提交的 osifont/Noto 组装到 `$DIST/fonts`，两种 bundle 都适用；
   `SKIP_FONTS=1` 复用已有目录、完全不下载。站点根因此始终有同源 `fonts/`。
3. **打开与懒加载**：页面先 `open_bytes`（此时无包；图纸需要文字时如实留下
   `unsupported-resources: fonts`），随后 `ensureFontPackage()`：
   - `invoke('cad_fonts')` 读 `requested`（已按大小写不敏感去重）、`textNeeded`、`locale`；
     无文字且无命名面时直接返回，不 fetch。
   - 从 `fonts/`（默认同源，`__YACR_FONT_BASE_URL__` 可覆盖）取 `fonts.json`，只保留本图需要的
     目录项（自身名称 + `DEFAULT_FALLBACK_NAMES` + locale 优先面），再取这些面的字节。
   - `set_font_package(catalogText, files)` → `app_web::fonts::install_package` 校验目录可解析、
     目录/文件非空、无重名后换入 `MemoryFontLoader`。**包可以只携带部分面**：目录里其余面
     没有字节不算错误，`plan_fonts` 选中缺失面时由 `load_package` 记为具名 `ResourceMissing`。
   - `apply_font_package()` → `CadSession::reinstall_fonts()` 重建引擎并呈现一帧；返回真实
     会话状态。拒绝按名：`font.package_missing` / `font.no_session` / `font.reinstall_failed`。
4. **失败**：fetch、校验或重装失败发布到 `__YACR_FONT_PACKAGE_ERROR__`，图纸保持打开、状态行
   保留，`cad_fonts` 保留 `fonts.failed`；下一次打开重试（失败不被记忆）。Worker 载体没有
   `CadSession`，对 `fonts` 请求返回 `unsupported-runtime: fonts`。

`MemoryFontLoader` 的 base 是 `memory://fonts/`，只作诊断标识，永不 fetch；查找按裸文件名
大小写不敏感。包不持久化：页面刷新后由下一次打开重新取。

## 验证

| 命令 | 覆盖 |
|---|---|
| `cargo test -p cad-platform fonts --locked` | `requested_fonts` 去重、`catalog_url`、计划/注册/失败逐项报告、默认面与 Noto 默认面、locale 链与缺口 |
| `cargo test -p cad-host fonts --locked` | 原生安装/复用/重定向、locale 缺口每文档一次 |
| `cargo test -p app-web fonts --locked` | `set_font_package` 校验、提交包/部分包安装、CJK 文本同时注册 osifont + Noto |
| `cargo test -p cad-desktop-shell --test font_commands --locked` | `cad_fonts` 命令注册、camelCase 序列化、无会话显式拒绝 |
| `(cd apps/desktop-ui && node --test test/fonts.test.ts test/vite-font-package.test.ts)` | 严格解析、只读行分组、`FONTS` 选择器、Vite 插件仅 tauri 复制、web/android 跳过 |
| `bash scripts/build-web-ui.sh` | exit 0；默认 staged mlightcad 全量 + 提交包到 `dist-ui/fonts`（`WITH_FONTS=0`/`FONTS_DIR` 为离线路径）。 |

**边界**：上述均为契约/编译/产物级证据。完整 mlightcad CDN 包的下载与浏览器运行时、真实
Cloudflare Pages 上传、真实 GPU 像素、Android 真机解压与 Noto 设备渲染、Windows/macOS
运行时均 **NOT RUN**。Web 的本地软件渲染证据见
[fonts.md](fonts.md#验证与证据边界)，不能升级为 GPU/像素结论。
