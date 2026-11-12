# 字体来源、打包与加载

本文记录当前 React/Tauri/wasm 载体下的 CAD 文本字体：提交了哪些字体、各自授权、各宿主如何
取字节、回退顺序、诊断、支持的格式、产物布局与验证边界。规范依据：§7.3（逻辑键，非平台路径）、
§10（资源）、§16.2。宿主的逐条接线见 [font-host-loading.md](font-host-loading.md)，只读报告
`cad_fonts` 的 wire 形状见 [commands.md](commands.md)。

## 唯一提交的字体包：`fonts/`

仓库根 `fonts/` 是**唯一提交的 CAD 字体包**。各宿主把它复制/打包到自己的产物或站点字体目录，
不各自维护第二份字体目录。

| 文件 | 内容 |
|---|---|
| `NotoSansCJKsc-Regular.otf` | Noto Sans CJK SC Regular，CFF OTF，**16,437,364 B**，SHA-256 `2c76254f6fc379fddfce0a7e84fb5385bb135d3e399294f6eeb6680d0365b74b` |
| `osifont.ttf` | QCAD 的 ISO 3098 技术制图字体，**116,512 B** |
| `fonts.json` | 目录清单，按 `mlightcad/cad-data` 的 `{ file, name[], type }` 格式；当前 2 个 `mesh` 面 |
| `NotoSansCJK-OFL.txt` | Noto 的 SIL Open Font License 1.1 文本 |
| `COPYING.GPL-3` | osifont 的 GPLv3 文本 |
| `SOURCE.md` | 两个字体的上游 URL、固定提交、字节数与 SHA-256 |

### 来源与授权

- **Noto Sans CJK SC**：来自 `notofonts/noto-cjk`，固定提交
  `f8d157532fbfaeda587e826d4cd5b21a49186f7c`，路径
  `Sans/OTF/SimplifiedChinese/NotoSansCJKsc-Regular.otf`。以 **OFL-1.1** 原样分发、未修改；
  OFL 保留字体名（RFN）为 `Source`（继承自 Adobe Source Han Sans），本仓库不改字体，因此无需改名。
  它是所有宿主上的**统一默认面**（`default_face`）：同时覆盖拉丁、假名与 CJK，也是四个 CJK
  locale 优先面的最后一项。
- **osifont**：QCAD 仓库的 `fonts/osifont.ttf` 副本，原始项目为
  <https://github.com/hikikomori82/osifont>，授权为 **GPL-3 + 字体例外**（嵌入文档不传染 GPL；
  本仓库 AGPL-3 可与 GPL 组件组合）。它是 Noto 之后的**次选轮廓回退面**。
- **`mlightcad/cad-data` 字体不入库**：其目录与字体由 `scripts/fetch-fonts.sh` 在需要时下载，
  供发布打包、CDN 包组装与测试语料使用；再分发授权由部署者负责。下载按文件缓存到
  `FONT_CACHE_DIR`（默认 `${XDG_CACHE_HOME:-$HOME/.cache}/yacr/fonts`），重复打包与两个 Web
  bundle 复用同一份；`FORCE=1` 强制刷新，显式空值关闭缓存。Web 运行时的同源 `fonts/`
  就是 `scripts/deploy-cloudflare-pages.sh` 用该脚本组装并上传到项目 Pages CDN 的包。
  `cad-app` 的 SHX 契约测试（`TXTEXP`/`SHP2BLK`）**运行时**才从上述生成目录或缓存读取
  `complex.shx`（不再 `include_bytes!`，否则 clean clone 无法编译）；缺面时显式跳过并打印
  staging 命令。`core-quality` 只做编译、不跑 Rust 测试，因此不在 CI 下载字体；要在本地跑
  这两条 SHX 契约，先执行 `FONTS=complex.shx bash scripts/fetch-fonts.sh apps/app-android/font-assets`。
- QCAD 的编译字体 `.cxf` 未提交也不支持；引擎只处理 SHX 与 sfnt/WOFF1。

## 各宿主的加载路径

共同契约（`cad-platform::fonts`）：

1. `requested_fonts(drawing)` 只读收集图纸引用的字体键（TEXT 的 `font`、样式的
   `resource_keys`），并按大小写不敏感去重（1 万标签的图纸报 7 个名称，而不是 4,849 个）；
   `needs_text_shaping(drawing)` 判断图纸是否含文字；`code_page()` 经
   `TextLocale::from_code_page` 得到 `latin | zh-Hans | zh-Hant | ja | ko`。
2. `plan_fonts` 把名称解析为目录条目/URL；未知名跳过、不伪造。
3. 宿主 `FontLoader` 提供字节，`FontEngine::register_with_encoding` 注册；随后
   `render.set_fonts(engine)`（空引擎则 `set_fonts(None)`，不安装“所有字形都会失败”的空引擎）。
4. `FontLoadReport` 逐项记录目录条目、请求、计划、注册、失败与每个 `FontFaceRecord`；失败不折成
   成功。

### Linux / Windows / macOS 桌面与 CLI（`native-below` 与 CLI）

- 共用 `cad-platform::fonts::local`。`resolve_font_dir` 先看显式 `--fonts-dir`（目录不存在是
  错误，不静默回退），否则按顺序找：可执行文件同级 `fonts/`、其父级 `fonts/`、macOS
  `.app` 的 `Contents/Resources/fonts`、当前工作目录 `fonts/`。
- `DirFontLoader` 从该目录读 `fonts.json` 与面字节；`load_engine_report` 再注册 `--font
  NAME=PATH` 显式字体，最后把系统默认字体（`fc-match` / `%WINDIR%\Fonts` /
  `/System/Library/Fonts` 等候选）作为首回退。`.ttc` 等无法解析的候选被跳过，不伪造。
- 宿主在 `cad-host` 里按图纸 locale 复用/重定向已安装引擎；同一语言且面齐全时不重复读包。
- CLI 的 `--font <name=path>` 与同级 `fonts/` 自动加载同样走这条路径；`render` / `plot` 也会注入。

### Android（`native-below`）

- Gradle 把仓库 `fonts/` 作为 APK asset 根（`assets.srcDir(.../fonts)`）。
- `MainActivity.kt` 在后台把 APK assets 解压到应用私有 `filesDir/fonts/`，完整复制后写标记
  **`.yacr-assets-v2-complete`**（v2 加入 Noto；升级包后旧安装会重新解压）。没有字体资产时打印
  显式诊断，**不**用“看起来有效但为空”的路径调用 JNI。
- 解压完成后经冻结的 `nativeAssetsReady(fontsDir, imagesDir)` 把真实文件系统路径交给 Rust；
  `HostOptions.fonts_dir` 指向它，之后走与桌面相同的 `cad-platform::fonts::local`。
- wasm 会话在 Android 上**不实例化**；字体由 native 会话持有。

### Web（`dom-canvas`，wasm 会话）

- **构建期就把字体包 staged 进 bundle，但仍然懒加载**。`scripts/build-web-ui.sh`（`dist-ui`）与
  `scripts/build-web.sh`（独立 harness `dist`）默认（`WITH_FONTS=1`）经 `scripts/lib/font-package.sh`
  调用 `scripts/fetch-fonts.sh`，把 mlightcad 全量目录 + 提交包放进 bundle 的 `fonts/`；
  `WITH_FONTS=0` 只放提交包、`FONTS_DIR=<dir>` 复用预置目录（两者都不联网）。页面仍只在
  `open_bytes` 之后按需从同源 `fonts/` 取面，不预加载。
  `apps/desktop-ui/vite-font-package.mjs` 只在 `__YACR_RUNTIME__=tauri` 时把字体包复制到
  `apps/desktop-ui/dist/fonts`（X11/XWayland `dom-canvas` 的离线包）；发布打包用
  `YACR_FONT_PACKAGE_DIR` 换成全量目录；web/android 都跳过。
- **发布包**：`scripts/deploy-cloudflare-pages.sh` 在上传前用 `scripts/fetch-fonts.sh` 把
  mlightcad 全量目录 + 提交的 osifont/Noto 组装到 `$DIST/fonts`（两种 bundle 都适用），站点根
  因此始终提供同源 `fonts/`；`SKIP_FONTS=1` 复用已有目录而不下载。该脚本同时把
  `NotoSansCJK-OFL.txt`、`COPYING.GPL-3`、`SOURCE.md` 复制到同一目录；OFL/GPL 文本不是可选
  payload，缺少时部署直接失败。Windows/macOS 的 `WITH_FONTS=1` 打包也走同一合并路径。
- **懒加载发生在 `open_bytes` 之后**：`bootstrap.js`（共享 canvas 引导）与
  `apps/app-web/web/main.js` 的 `ensureFontPackage()` 先读 `cad_fonts` 的 `requested`、
  `textNeeded`、`locale`；无文字且无命名面时不取任何字节。需要时从同源 `fonts/`（Web 为项目
  Pages CDN，X11/XWayland 为 tauri 包内 `dist/fonts`；`__YACR_FONT_BASE_URL__` 可覆盖基址）
  只取 `fonts.json` 与本图需要的目录项，调用
  `set_font_package`，再调用 `apply_font_package()` → `CadSession::reinstall_fonts()` + 一帧。
  `ready` / `start` **从不等待**字体。
- **部分包是正常情况**：页面只取本次需要的面，目录里其余面没有字节不算错误；`plan_fonts`
  选中缺失面时由 `load_package` 记为具名 `ResourceMissing` 行，不静默空面。
- `set_font_package` 先校验：目录可解析、目录与文件非空、无重名、每个实际携带的文件字节非空。
  校验通过才把 `MemoryFontLoader` 换入线程局部槽；坏包不会替换已安装的好包。返回
  `{ok, catalog_entries, files, bytes}`（`catalog_entries` 是整份目录，`files` 是本次携带的面）。
- 打开时若还没有包且图纸需要字体/文字整形，`BrowserResources::install_fonts` 显式留下
  `unsupported-resources: fonts`（打开漏斗记为 `fonts.failed`，图纸仍打开、文字诚实不整形）；
  包到达后 `apply_font_package` 在**已打开**的会话上重跑安装并重绘。
- 加载失败发布到 `__YACR_FONT_PACKAGE_ERROR__`，不伪造成功；图纸与状态行保持。
- 隔离 Worker 载体没有 `CadSession`，对字体请求显式拒绝
  `unsupported-runtime: fonts`，不假装加载。

## 回退顺序与诊断

### 默认面与 locale 面

- `DEFAULT_FALLBACK_NAMES`（`cad-platform::fonts`）当前为：

  ```
  noto sans cjk sc → osifont → arial → simplex
  ```

  只在需要默认面时（图纸有文字但没注册到任何面，或有未解析/失败的面）按顺序尝试，
  **注册第一个可用者**为 `default_face` 并停止。Noto Sans CJK SC 是统一默认面：它同时覆盖
  拉丁、假名与 CJK，因此缺字图纸不会静默丢失中文；代价是首次需要默认面时会加载 16.4 MB
  的 Noto 文件。`osifont` 仍在其后，供没有 Noto 的包使用。
- `TextLocale::preferred_faces()` 为 CJK locale 附加 locale 面；四个 CJK locale 的**最后一项**
  都是 `noto sans cjk sc`：

  | locale | 优先面（最后统一为 Noto） |
  |---|---|
  | `zh-Hans` | `gbcbig` → `hztxt` → `chineset` → `bigfont` → `simhei` → `simsun` → `noto sans cjk sc` |
  | `zh-Hant` | `chineset` → `bigfont` → `noto sans cjk sc` |
  | `ja` | `bigfont` → `noto sans cjk sc` |
  | `ko` | `bigfont` → `noto sans cjk sc` |

  加载器只注册**第一个可用**的 locale 面。最终回退链由
  `TextLocale::ordered_fallback(registered)` 生成：CJK 把已注册的 locale 优先面前移，其余面保持
  注册顺序，未注册的名称不会凭空出现。默认面循环已先注册 Noto 时，locale 循环会复用它，
  `default_face` 与 locale 面都是 Noto；包内没有 Noto 时才继续尝试 SHX/locale 面。
- 显式 `--font` 或系统默认面可补足 locale 面；引擎会按 locale 重新排序回退链。

### 稳定诊断

| 代码 / 状态 | 含义 |
|---|---|
| `font.locale_fallback_missing` | 非拉丁 locale 且有文字，但没有任何 locale 优先面注册成功。每次加载/打开只报一次，消息是 ASCII 机器细节（locale + 尝试过的候选），UI 按 code 本地化。 |
| `text.glyph_missing` | 整形时实际缺字（字体已注册但该 glyph 不存在）。空白 glyph / 仅空格 run 不产生伪缺图。 |
| `text.sfnt_fill_partial` | SFNT 轮廓无法证明 nonzero winding 与现有 even-odd 填充器等价，或填充器失败；保留边界线并记 `Partial`，不静默错误填洞。 |
| `fonts.failed` | 宿主字体资源安装失败（如 Web 无 staged package 的 `unsupported-resources: fonts`）；是会话状态/诊断，不把图纸打开折成失败。 |

`cad_fonts` 的 `faces[].status` 另用稳定小写标签 `loaded | failed | unresolved | unsupported`，
`source` 为 `package | explicit | system`；`detail` 是宿主原始失败原因。失败面保留在列表中，
读取不重试、不编造。

## 支持的字体格式

| 格式 | 状态 |
|---|---|
| SHX（`shapes` / `unifont` / `bigfont`） | 支持，含八分圆/分数/凸度圆弧、子形状、缩放与进退笔；按 encoding（gbk/shift-jis/…）映射回 Unicode。输出线稿，不做面填充。 |
| 原始 SFNT：TTF / OTF（含 **CFF** 轮廓） | 支持。CFF 由 `ttf-parser` 解析；提交的 Noto CFF OTF 有专门契约测试验证注册与中文轮廓输出。 |
| WOFF1 | 支持：`woff_to_sfnt` 用 flate2 解压重建 sfnt。 |
| WOFF2 | **不支持**，显式记为 `Unsupported`（可被回退面替代），不伪造。 |
| TTC / 字体集合 | 不是一等格式：注册只解析集合的 **index 0**，集合内其他面不可寻址；系统候选中的 `.ttc` 若解析失败会被跳过，不当作已加载。 |
| QCAD `.cxf` | **不支持**，未提交。 |

SFNT glyph 的填充仍走安全路径：只有证明 nonzero winding 与 even-odd 在轮廓简单、互不相交/
重叠且严格嵌套绕向交替时等价，才生成 mesh；否则保留边界线并报 `text.sfnt_fill_partial`。
成功时关闭该 glyph 的 mesh edge overlay，字腔保留且不暴露三角内部边。

## 打包产物

同一提交包按载体落到不同位置，互不覆盖：

| 载体 | 产物位置 | 来源 |
|---|---|---|
| 桌面（Tauri，含 X11/XWayland `dom-canvas`） | `apps/desktop-ui/dist/fonts/` | Vite 插件 `vite-font-package.mjs`：默认提交包；发布打包导出 `YACR_FONT_PACKAGE_DIR` 时改用 `scripts/fetch-fonts.sh` 组装的全量包（X11/XWayland 页面懒读同源 `fonts/`） |
| Web `dist-ui` | bundle 内 `fonts/`（默认全量）＋站点根 `fonts/` | `scripts/build-web-ui.sh` 经 `scripts/lib/font-package.sh` staged（`WITH_FONTS=1` 默认走 `scripts/fetch-fonts.sh`；`0`/`FONTS_DIR` 离线）；页面打开图纸后懒加载 |
| Web 独立 harness `dist` | bundle 内 `fonts/`（默认全量）＋站点根 `fonts/` | `scripts/build-web.sh` 同上；`web-deploy` 上传前再用 `scripts/fetch-fonts.sh` 合并/校验站点根包 |
| Android | APK `assets/fonts/` → 运行时 `filesDir/fonts/` | `scripts/build-android-ui.sh` 经共享 `scripts/lib/font-package.sh` 组装 `apps/app-android/font-assets`（默认 mlightcad 全量 + 提交包；`WITH_FONTS=0`/`FONTS_DIR` 离线）+ Gradle `assets.srcDir(font-assets)` + Kotlin 按 `fonts.json` 解压（标记 `.yacr-assets-v3-complete`） |
| Linux Flatpak | payload `fonts/`（`bin/` 同级） | `scripts/package-linux-release.sh`：默认 `WITH_FONTS=1` 用 `scripts/fetch-fonts.sh` 组装 mlightcad 全量并合并提交包；`WITH_FONTS=0` 只放提交包；`FONTS_DIR=<dir>` 用预置目录、打包步骤免联网 |
| Windows / macOS 发布包 | `fonts/` | `WITH_FONTS=1` 时 `scripts/fetch-fonts.sh` 组装 mlightcad 全量并合并提交包；`0` 只放提交包 |
| CLI | 可执行文件同级 `fonts/` 或 `--font` | `cad-platform::fonts::local` |

本地 Web 构建默认（`WITH_FONTS=1`）就把 mlightcad 全量目录 staged 到 bundle 的 `fonts/`；
`WITH_FONTS=0` 只放提交包，`FONTS_DIR=<dir>` 复用已组装目录（两者都不联网）。页面仍然只在打开
图纸后从同源 `fonts/` 懒加载需要的面，不预加载、不阻塞 `ready`/`start`。

`scripts/fetch-fonts.sh <DEST> [FONTS...]` 是平台无关的“下载第三方目录 + 合并提交包”打包
入口，`scripts/lib/font-package.sh` 是各宿主共用的选择策略（`WITH_FONTS`/`FONTS_DIR`）。调用方：
Linux Flatpak、Windows/macOS 发布包、Android APK（`build-android-ui.sh`）、Web 本地 bundle
（`build-web.sh`/`build-web-ui.sh`）与 Web CDN 包（`deploy-cloudflare-pages.sh`）。它本身不产生
Web 构建产物。mlightcad 字体仍不提交，再分发授权由部署者负责。

## 验证与证据边界

### 本轮实际运行（2026-11-03）

下列契约测试/构建数字是本轮**早先**的运行；lazy Web 打包与 `requested`/`textNeeded` 改动后
没有全部重跑（见 NOT RUN），重跑数字可能变化。

| 命令 | 结果 |
|---|---|
| `cargo test -p cad-representation committed_noto_cjk_cff_face_outlines_chinese_glyphs --locked` | 1 passed（CFF OTF 注册 + `中文` 轮廓/mesh 输出） |
| `cargo test -p cad-platform fonts --locked` | 34 passed（`requested_fonts` 去重、默认面顺序、Noto 默认面、locale 链、缺口诊断、本地目录/系统面） |
| `cargo test -p app-web fonts --locked` | 7 passed（提交包/部分包安装、坏包拒绝、CJK 文本注册 osifont + Noto） |
| `cargo test -p cad-host fonts --locked` | 8 passed（原生安装/复用/重定向/locale 缺口） |
| `cargo test -p cad-ui-core fonts --locked` | 3 passed（`cad_fonts` wire 投影） |
| `cargo test -p cad-desktop-shell --test font_commands --locked` | 6 passed（`cad_fonts` 命令注册/序列化/无会话拒绝） |
| `(cd apps/desktop-ui && node --test test/fonts.test.ts test/vite-font-package.test.ts)` | 14 pass / 0 fail（严格解析、行分组、`FONTS` 选择器、Vite 插件仅 tauri 复制、web/android 跳过） |
| `bash scripts/build-web-ui.sh` | exit 0；**2026-11-03 记录**：当时默认不 staging 字体（`fonts/*` 报 `not bundled (lazy CDN fonts/)`），`YACR_WEB_FONTS=1` 才放提交子集。当前默认（`WITH_FONTS=1`）已改为 staged 全量，未重跑。 |

### Web 运行时证据（软件渲染，非 GPU/像素验收）

lazy 改动后，本地对 `apps/app-web/dist-ui` 做无头浏览器检查（**软件渲染**），字体由 override
base 提供：

- 打开前 **0 个字体请求**：`ready` / `start` 不等待字体。
- 打开 `fixtures/debug-default-fonts.dwg` 后恰好 **3 个**请求：`fonts.json`、`osifont.ttf`、
  `NotoSansCJKsc-Regular.otf`（只取本图需要的目录项）。
- `cad_fonts`：`defaultFace=NotoSansCJKsc-Regular.otf`、`localeMissing=null`、`requested` 7 个
  去重名称；`shell_status().error=null`；页面无错误。
- 这是软件渲染路径，只能作布局/DOM/加载路径证据；**不是**真实 GPU、像素、视觉 golden
  或浏览器产品验收。

**未覆盖**：完整 mlightcad CDN 包的实际下载/运行时（需要网络）与真实 Cloudflare Pages 上传
（需要凭据）均 **NOT RUN**。上一轮 W2 的 `web-ui-smoke` 是 SwiftShader，也不构成本轮 CDN
运行时证据；真实 Noto 字形在这些截图中的视觉正确性未单独判定。

### NOT RUN / 未验证

- 完整 mlightcad CDN 包的下载与浏览器运行时、真实 Cloudflare Pages 上传：**NOT RUN**。
- 真实 GPU（硬件 Vulkan/Metal/D3D）与真实浏览器像素/视觉验收：**NOT RUN**。
- Android 真机/模拟器上的 Noto 解压、注册与实际字形：**NOT RUN**；只有编译/契约级证据。
- Windows/macOS 运行时、字体目录发现、系统面注册：**NOT RUN**（仅编译门禁/历史 Wine 参数
  smoke，见平台文档）。
- Linux 桌面真实窗口的字体面板/Noto 渲染截图：**NOT RUN**。
- 性能、内存与包体积影响未做基准；默认面是 Noto，只有真需要默认面/命名字体的图纸才取字节，
  但“不需要时零成本”只有代码路径依据，没有测量结论。

历史材料（旧宿主、2026-10 的 CLI/CJK 专项）保留在 `docs/handoff.md` 与历史 validation
文档中，不作为本轮 React/Tauri/wasm 载体的证据。
