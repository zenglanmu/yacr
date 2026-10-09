# 字体来源与目录

本文记录 CAD 文本/字体资源从哪里来、如何编目、如何随客户端输出打包，以及当前的
实现边界。规范依据：§7.3（逻辑键，非平台路径）、§10（资源）、§16.2。

## 来源

字体来源有两个项目：

- **`mlightcad/cad-data`**（运行时/构建时获取，**不入库**）：默认字体集与
  `mlightcad/cad-viewer` 网页版一致，经 jsDelivr CDN 提供：
  - 默认根：`https://cdn.jsdelivr.net/gh/mlightcad/cad-data@main/fonts/`
    （对应 cad-viewer 的 `DEFAULT_BASE_URL = 'https://cdn.jsdelivr.net/gh/mlightcad/cad-data'`
    加 `fonts/`；见 `packages/cad-simple-viewer/src/app/AcApDocManager.ts`）。
  - 目录清单：`fonts/fonts.json`（截至 2026-10-01，
    sha256 `cb2344fa2f91648f3cb44b7f5a1e75342da5fa578d640de8e0500b12697246a0`，
    99 个字体：86 个 `.shx` + 13 个 outline/WOFF）。
  - outline 字体示例：`simsun.woff`、`simhei.woff`、`simkai.woff`、`msyh.woff`、
    `msgothic.woff`、`noto-sans-kr.woff`、`arial.woff`、`tahoma.woff`、`verdana.woff`、
    `gbgdt.woff`、`SJQY.woff`、`AIGDT.ttf`、`simsun.ttf`。
  - SHX 字体示例：`simplex.shx`、`txt.shx`、`romans.shx`、`isocp.shx`、`bigfont.shx`、
    `hztxt.shx`、`gbcbig.shx` 等。
- **QCAD（`fonts/` 目录，授权入库）**：ISO 3098 技术制图字体
  `osifont.ttf`（GPL-3 + 字体例外）已提交到仓库 `fonts/`（来源/哈希/授权见
  `fonts/SOURCE.md`），作为**默认轮廓回退面**随客户端输出分发。QCAD 的 `.cxf`
  编译字体当前引擎不支持，显式记为 Unsupported（见「字形渲染」）。

`fonts.json` 条目结构：

```json
{ "file": "simplex.shx", "name": ["simplex"], "type": "shx" }
{ "file": "@extfont2.shx", "name": ["@extfont2"], "type": "shx", "encoding": "shift-jis" }
{ "file": "simsun.woff", "name": ["SimSun", "宋体"], "type": "mesh" }
```

## 字体包与输出布局

仓库级 `fonts/` 是**唯一提交的 CAD 字体包**（`fonts.json` + `osifont.ttf` +
`COPYING.GPL-3` + `SOURCE.md`），各宿主把它复制到自己的输出旁：

- **Linux 桌面/CLI（发布包）**：`scripts/package-linux-release.sh` 默认
  `WITH_FONTS=1`，用同一个 `scripts/fetch-fonts.sh` 把 mlightcad 全量字库 + 提交的
  `fonts/` 组装到发布包 `fonts/`（可执行文件 `bin/` 同级）；`WITH_FONTS=0` 只打包
  提交的 `fonts/`（osifont）。宿主启动时自动扫描 `fonts/fonts.json` 并按需加载
  （见「主机接线」）。
- **Web**：`scripts/build-web.sh`/`scripts/fetch-fonts.sh` 把提交的 `fonts/` 合并进
  `web-dist/fonts/`（并与下载的 mlightcad 目录清单合并且去重），浏览器宿主优先用
  同源 `fonts/`，未发布本地包时回落到 CDN。
- **Android**：`scripts/fetch-fonts.sh` 把提交的 `fonts/` 合并进
  `assets/fonts/`（该目录本身仍 gitignore，不入库）。

`scripts/fetch-fonts.sh <DEST> [FONTS...]` 是**平台无关的唯一打包入口**：下载 mlightcad
目录清单与指定面（无参数则为全部）、合并提交的 `fonts/`、写 `DEST/fonts.json`。Web /
Android / Linux 发布包都只是传不同的 `DEST`，本身不含任何平台逻辑（旧
`fetch-web-fonts.sh`、`fetch-android-fonts.sh` 已并入它并删除）。mlightcad 字体是第三方，
随发布包内置于任何平台时都由打包方负责其再分发授权。

## 实现

`cad-resources` 提供纯数据目录，不做任何网络/文件系统访问：

- `DEFAULT_FONT_BASE_URL`：上述 CDN 根。
- `FontKind::{Shx, Mesh, Other}`、`FontFace { file, names, kind, encoding }`。
- `FontCatalog::from_json`：解析 `fonts.json`，按**名称、文件名、文件主名**
  建立大小写不敏感索引（处理 `fonts/SIMPLEX.SHX` 之类引用）。
- `FontCatalog::get(name)`：命中返回 face，未命中返回 `None`（不伪造字体）。
- `face_url(base, face)` / `default_font_url(face)`：生成下载 URL，空格编码为 `%20`。

主机负责真正取字节：把字体下载/读取后经 `MapResolver::grant` 授权，再用现有
`ResolverChain`（用户包 → 文档映射 → 内置集）解析。安全策略不变：路径/URL 引用先经
`is_safe_reference` 与 `bare_name` 收敛为裸文件名。

## 授权

`mlightcad/cad-data` 未声明仓库级许可证，且其中的 `.shx`/`.ttf`/`.woff` 各自有字体
授权（多为 Autodesk/微软/开源字体的再分发）。因此：

- **本仓库不内置 mlightcad 字体**；只在运行时/构建时按需下载，再分发的授权由部署者负责。
- 使用前必须核实目标字体的授权，不能默认可再分发。
- **例外（用户指定）**：QCAD 的 `osifont.ttf` 以 GPL-3 + 字体例外授权，已提交到
  `fonts/`（`fonts/SOURCE.md` 记录来源、哈希与许可；本仓库为 AGPL-3，可与 GPL 组件
  组合）。QCAD 的 `.cxf` 未提交也未支持。

## 字形渲染（已实现：sfnt + SHX）

`cad-representation` 的 `FontEngine` 把文本转成世界坐标折线，`cad-scene` 按普通线段绘制：

- **SFNT**：TTF/OTF 原始 sfnt，以及 **WOFF1**（`woff_to_sfnt` 用 flate2 解压重建 sfnt）；
  拒收 **WOFF2**（记为 `Unsupported`，可被回退取代）。注册时校验可解析，坏字节报错。
- **SHX**：移植自 MIT 的 `@mlightcad/shx-parser`，支持编译 shape 字体的三种内容布局
  `shapes` / `unifont` / `bigfont`（含八分圆/分数/凸度圆弧、子形状、缩放与进退笔），
  按 `encoding`（gbk/shift-jis/…）把 Unicode 映射回字体码。
- **回退链**：`FontEngine::set_fallback(keys)`。图纸引用的字体未注册、或注册了但不可解码
  （如 WOFF2/不支持的 SHX 类型）时，按顺序改用可用的回退字体，**不会静默丢弃文本**；
  全部不可用才报告错误。`resolve_face` 可查询实际使用的字体。
- **默认轮廓回退面**：`cad-platform::fonts::load_font_engine` 在图纸字体缺失/拉取失败时
  保证注册一个默认轮廓面（`DEFAULT_FALLBACK_NAMES`，顺序 `osifont` → `arial` → `simplex`），
  并写入 `FontLoadReport.default_face`。桌面/CLI 宿主还可把**系统默认字体**（fontconfig
  `fc-match`）注册为第一回退（`register_default_face`，保留键 `__yacr_default__`）。
  浏览器无法读取系统字体字节，故 Web 用目录里的默认面（CDN 构建为 `arial`，自托管构建为
  已提交的 `osifont`）承担同一角色；QCAD `.cxf`、WOFF2 等不支持技术仍显式报告，不伪造。
- 键匹配：注册键 + 文件主名。图纸引用 `arial.ttf`、库只有 `arial.woff` 时按主名 `arial` 命中。
- 排版：按字形 advance 前进（SHX 用 ink-width + cell 边距策略；TTF 应用 `kern` 字距），
  `\n`/`\P` 换行。`cad-representation::text` 现在把 MTEXT/TEXT 控制内容解析为结构化 run
  列表（`parse_mtext`）并逐 run 排版（`FontEngine::shape`）：字体 `\f`/`\F`、绝对/相对字高
  `\H`、宽度因子 `\W`、倾斜 `\Q`、颜色 `\C`/`\c`、堆叠分数 `\S`、分组 `{}`、`\~`、`%%`
  特殊字符与 `\U+XXXX` 均被保留；无法忠实渲染的项（颜色/装饰/堆叠/`\A`/`\T` 等）会以
  `TextFormatIssue` 显式报告并计入 `Partial`，不静默忽略。详见 `docs/mtext.md`。
- **对齐**：TEXT 的 `horizontal_alignment`/`vertical_alignment`（含 `alignment_point`）与
  MTEXT 的 `attachment_point` 映射为 `TextAlignH/TextAlignV`，在排版时按行宽/行高偏移；
  `Aligned/Fit` 按左对齐处理（不拉伸）。
- 主机通过 `RepresentationContext::with_fonts(Arc<FontEngine>)` 注入；CLI 用
  `--font <name=path>`（可重复），并自动把所有已注册字体设为回退。未注入字体时文本仍为
  不可绘的 `DisplayPrimitive::Text`。

## 主机接线

- **清单与计划**：`cad-resources::plan_fonts(catalog, requested, base)` 把图纸引用的字体名
  解析为 `PlannedFont { request, file, kind, encoding, url }`，按文件去重，未知名跳过。
- **加载契约**：`cad-platform::FontLoader::load_font(url) -> HostFuture<Arc<[u8]>>`（另有
  默认实现 `load_catalog(url)`，可按字节流覆写目录获取）。核心只生成 URL，主机负责网络/资源访问、
  授权与缓存。
- **共用宿主编排**（`cad-platform::fonts`，Web 与 Android 共用）：
  - `requested_fonts(&DrawingDatabase)`：只读地收集图纸引用的字体键——`TEXT` 几何的 `font`
    字段与文本样式的 `Style::resource_keys`（导入器写入该样式的 SHX/BigFont/TTF 名）。只做
    名称收集，不查目录、不联网、不做路径解析。
  - `catalog_url(base)`：`{base}/fonts.json`。
  - `load_font_engine(loader, requested, base)`：取目录 JSON → `FontCatalog::from_json` →
    `plan_fonts` → 逐个 `load_font` → `FontEngine::register_with_encoding(file, bytes, encoding)`
    → `set_fallback(已注册键)`；返回 `(Arc<FontEngine>, FontLoadReport)`。
    `FontLoadReport` 逐项记录目录条目、请求、计划、注册、失败（含原因），不把失败折成成功。
- **显示**：主机把加载到的字节经上述流程注册后调用 `CadView::set_fonts(Arc<FontEngine>)`；
  注册为空时调用 `CadView::clear_fonts()`（而不是安装一个所有字形都会失败的空引擎）。
  桥接下一帧重建场景并整形文字（字体到达后也会重建）。
- 典型流程：`fonts.json` → `requested_fonts` → `plan_fonts` → 主机按 `url` 取字节 →
  `register_with_encoding` → `set_fonts`。
  - **Web**（`apps/app-web`）：`WebFontLoader` 用 `fetch().arrayBuffer()` 取字节，目录为
    `DEFAULT_FONT_BASE_URL`；`open_document` 后 `spawn_font_load()` 异步加载，过期结果
    （期间换了图纸）以 `StaleResult` 丢弃。导出 `load_web_fonts()` / `font_load_report()`
    供 JS 宿主与无头验证等待并读取 `FontLoadReport::summary()`。
  - **Android**（`apps/app-android`）：`AssetFontLoader` 从活动 `AssetManager` 读
    `assets/fonts/…`（基址 `asset://fonts/`），目录为 `asset://fonts/fonts.json`。
    走同一个 `load_font_engine`。mlightcad 字体**不入库**（授权，见上），未打包时报告
    `ResourceMissing`（`font asset not packaged …`），不伪造空字体集；但
    `scripts/fetch-fonts.sh` 会把提交的 `fonts/`（osifont）合并进资产集，保证 APK
    自带默认轮廓面。
  - **Linux / Windows 桌面与 CLI**（`app-linux`、`app-windows`、`cad-cli-tools`）：
    共用 `cad-platform::fonts::local`。启动（或每次打开图纸）时扫描可执行文件同级的
    `fonts/`（或 `--fonts-dir`）目录，用同一套 `load_font_engine` 从本地读字节；
    之后注册 `--font` 显式字体；最后把系统默认字体注册为第一回退（Linux 用
    `fc-match`，Windows 读 `%WINDIR%\Fonts` / `%LOCALAPPDATA%\Microsoft\Windows\Fonts`）。
    没有 `fonts/` 也没有系统字体时文本保持不可绘，绝不伪装。

## Web 自托管（发布包）

浏览器宿主在编译期读 `YACR_FONT_BASE_URL`（`apps/app-web/src/browser.rs`）决定目录/字体基址：
未设置时回退到 `DEFAULT_FONT_BASE_URL`（jsDelivr）。`scripts/build-web.sh` 默认
`YACR_FONT_BASE_URL=fonts/`（相对页面），并在打包时调用 `scripts/fetch-fonts.sh`
把 `mlightcad/cad-data` 的 `fonts.json` 与全部字体下载进 `web-dist/fonts/`，所以
**发布包自包含、不跨域**。

`fetch-fonts.sh` 默认先用 jsDelivr（`FONT_BASE_URL`）；单个文件失败时按
`FONT_RETRIES`（默认 3）退避重试，仍失败再回退到 `FONT_FALLBACK_BASE_URL`
（默认 `https://raw.githubusercontent.com/mlightcad/cad-data/main/fonts`，GitHub
Actions 的共享出口 IP 常被 jsDelivr 限流，但可访问 raw 源）。每个文件记录实际来源；
两个源都失败才整体退出 1（不把缺字体折成成功）。需要回到 CDN 时：

```bash
YACR_FONT_BASE_URL=https://cdn.jsdelivr.net/gh/mlightcad/cad-data@main/fonts/ \
  WITH_FONTS=0 scripts/build-web.sh
```

mlightcad 字体文件仍不提交仓库（`/web-dist/` 已 ignore）；下载与再分发的授权由部署者负责。
仓库提交的 `fonts/`（QCAD osifont）由 `fetch-fonts.sh` 在打包时合并进 `web-dist/fonts/`。
Cloudflare Pages 发布见 `scripts/deploy-cloudflare-pages.sh`。

## 主机取字节实现现状

| 宿主 | 目录来源 | 字体字节来源 | 状态 |
|---|---|---|---|
| Web (wasm) | `DEFAULT_FONT_BASE_URL/fonts.json`（jsDelivr，CORS）；自托管构建为同源 `fonts/` | `fetch().arrayBuffer()` | 代码完成；本环境**未跑浏览器验证** |
| Android | `asset://fonts/fonts.json`（APK assets，含提交的 osifont） | `AssetManager` | 代码完成并编译；本环境**无真机/无打包字体资产** |
| CLI | 发布包 `fonts/`（`fetch-fonts.sh` 组装 mlightcad 全量 + osifont）+ `--font name=path` | 本地文件 | 已实现（`cad-platform::fonts::local`；本机差分 smoke 见下） |
| Linux 桌面 | 可执行文件同级 `fonts/`（或 `--fonts-dir`）+ `--font` | 本地文件 + 系统默认字体（`fc-match`） | 已实现并编译；离屏/桌面运行见 `docs/linux-app.md` |
| Windows 桌面 | 可执行文件同级 `fonts/`（或 `--fonts-dir`）+ `--font` | 本地文件 + 系统默认字体（`%WINDIR%\Fonts`） | 交叉编译并 Wine 参数解析 smoke；真实 Windows/GPU 未验证，见 `docs/windows-app.md` |

Web 的 `fetch` 依赖 CDN 的 CORS 头；非 2xx 响应记为 `ResourceMissing`，不当作空字节。
Android 若未把字体目录放进 `assets/fonts/`（含 `fonts.json`），打开带文本的图纸会以缺资源
报告，属预期而非缺陷。取字节的**浏览器/真机行为尚未在本仓库验证**。


实测（`docs/validation.md` 语料；注册 `simplex/txt/romans.shx` + `arial.woff` 并启用回退）：

| 样本 | 无字体 lines/texts | 有字体 lines/texts |
|---|---|---|
| AutoCAD_2000（SHX） | 19 / 11 | 55 / 0 |
| AutoCAD_2013 | 0 / 3 | 30 / 0 |
| baseline-sample | 1021 / 60 | 3298 / 0 |
| lockers | 1796 / 33 | 2747 / 0 |
| map-of-uae | 173 / 32 | 545 / 0 |
| korean-DBCS-hangul | 2 / 3 | 96 / 0 |
| canteen（GOST，靠回退） | 42749 / 323 | 44964 / 0 |

2026-10-09 CLI 差分 smoke（本机，软件环境，非视觉验收）：`entities.dxf`（94 个
TEXT/MTEXT）在可执行文件同级的 `fonts/` 不可见时 `build-representation` 为
`lines=1443`；在仓库根（`./fonts` 命中）或仅 osifont 的发布包（`bin/../fonts` 命中）
运行时为 `lines=1497`、`texts=0`——缺字体时默认轮廓面（osifont + 系统 DejaVu）把
文字整形成线段，而不是丢弃。加入 mlightcad 全量字库的发布包（`WITH_FONTS=1`，
101 个文件）从无关 CWD 运行同样 `texts=0`、`lines=1395`——图纸引用的目录字体在本机
解析成形，不再依赖回退或网络。

## 未完成

1. **排版完备性**：复杂文字整形（bidi/上下文 shaping）与列/真正制表位仍缺；MTEXT 全格式码
   现由 `parse_mtext` 解析为 run 列表（见 `docs/mtext.md`），但颜色、下划线/上划线/删除线、
   堆叠分数、`\A`/`\T`/`\p` 等只能近似或仅解析，`shape` 以 `TextFormatIssue` 显式报告；
   精确行距/垂直对齐仍为近似（行距 = 1.2×该行最高 run 字高）。
2. **平台验证**：`FontLoader`/`plan_fonts`/`set_fonts` 契约与 Web/Android 取字节代码均已
   实现并编译（见上表），但**浏览器与真机上的实际下载/整形/重绘尚未在本仓库验证**；
   Android 也仍未真正随 APK 打包并安装验证。
3. **Android 网络路径**：当前只实现 asset 路径；运行时 HTTP 下载（含授权与缓存策略）未实现。
4. **桌面/CLI 的系统默认面**：`fc-match` 结果随系统而异（本机 `sans-serif` 返回 `.ttc`，
   会被引擎跳过后再试 `DejaVu Sans`）；不同桌面得到不同的默认字形，属预期但不是确定值。
   浏览器端无法读取系统字体字节，Web 的「默认」是目录/CDN 里的轮廓面，不等价于浏览器
   内建字体。

含文本的图纸在导入报告里不会被报告为 `Complete`（导入阶段不知道宿主是否有字体）；
在 `build-representation` 注入字体后文本即可绘。

2026-10-03 真实图纸回归修复：CLI `render` / `plot` 也将 `--font` 注入表示上下文，
不再仅在 `build-representation` / `benchmark` 生效。无字体时仍保留不可绘 Text，
不把文字缺失当作完整渲染。canteen 原生/浏览器的差异见 `validation-dwg.md`。
