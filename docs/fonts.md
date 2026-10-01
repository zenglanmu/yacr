# 字体来源与目录

本文记录 CAD 文本/字体资源从哪里来、如何编目，以及当前的实现边界。
规范依据：§7.3（逻辑键，非平台路径）、§10（资源）、§16.2。

## 来源

本仓库不内置字体。默认字体集与 `mlightcad/cad-viewer` 网页版一致，来自
`mlightcad/cad-data` 仓库，经 jsDelivr CDN 提供：

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

`fonts.json` 条目结构：

```json
{ "file": "simplex.shx", "name": ["simplex"], "type": "shx" }
{ "file": "@extfont2.shx", "name": ["@extfont2"], "type": "shx", "encoding": "shift-jis" }
{ "file": "simsun.woff", "name": ["SimSun", "宋体"], "type": "mesh" }
```

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

- **不把任何字体文件提交进本仓库**；只在运行时/测试时按需下载。
- 使用前必须核实目标字体的授权，不能默认可再分发。

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
- 键匹配：注册键 + 文件主名。图纸引用 `arial.ttf`、库只有 `arial.woff` 时按主名 `arial` 命中。
- 排版：按字形 advance 前进（SHX 用 ink-width + cell 边距策略；TTF 应用 `kern` 字距），
  `\n`/`\P` 换行；`sanitize_text` 处理 `%%d/%%p/%%c`、`\P`、`\~`、花括号与 `\X...;`
  格式码（近似）。
- **对齐**：TEXT 的 `horizontal_alignment`/`vertical_alignment`（含 `alignment_point`）与
  MTEXT 的 `attachment_point` 映射为 `TextAlignH/TextAlignV`，在排版时按行宽/行高偏移；
  `Aligned/Fit` 按左对齐处理（不拉伸）。
- 主机通过 `RepresentationContext::with_fonts(Arc<FontEngine>)` 注入；CLI 用
  `--font <name=path>`（可重复），并自动把所有已注册字体设为回退。未注入字体时文本仍为
  不可绘的 `DisplayPrimitive::Text`。

## 主机接线

- **清单与计划**：`cad-resources::plan_fonts(catalog, requested, base)` 把图纸引用的字体名
  解析为 `PlannedFont { request, file, kind, encoding, url }`，按文件去重，未知名跳过。
- **加载契约**：`cad-platform::FontLoader::load_font(url) -> HostFuture<Arc<[u8]>>`。核心只
  生成 URL，主机负责网络/资源访问、授权与缓存。
- **显示**：主机把加载到的字节 `FontEngine::register(_with_encoding)` 后调用
  `CadView::set_fonts(Arc<FontEngine>)`；桥接下一帧重建场景并整形文字（字体到达后也会重建）。
- 典型流程：`fonts.json` → `plan_fonts` → 主机按 `url` 取字节 → `register` →
  `set_fonts`。Web 用 `fetch().arrayBuffer()`，Android 用 HTTP/asset；这些具体取字节代码
  尚未在本仓库实现（无真机/浏览器验证环境）。

实测（`docs/validation.md` 语料；注册 `simplex/txt/romans.shx` + `arial.woff` 并启用回退）：

| 样本 | 无字体 lines/texts | 有字体 lines/texts |
|---|---|---|
| AutoCAD_2000（SHX） | 19 / 11 | 55 / 0 |
| AutoCAD_2013 | 0 / 3 | 30 / 0 |
| baseline-sample | 86 / 60 | 2363 / 0 |
| lockers | 1796 / 33 | 2747 / 0 |
| map-of-uae | 87 / 32 | 459 / 0 |
| korean-DBCS-hangul | 2 / 3 | 96 / 0 |
| canteen（GOST，靠回退） | 41594 / 323 | 43809 / 0 |

## 未完成

1. **排版完备性**：复杂文字整形（bidi/上下文 shaping）、MTEXT 全格式码（堆叠、列、
   制表）与精确行距/垂直对齐；当前逐字、TTF `kern` 已应用、行距固定 1.2×、垂直对齐近似。
2. **平台取字节代码**：`FontLoader`/`plan_fonts`/`set_fonts` 契约已就绪，Web
   `fetch`/Android HTTP 的实际实现与真机/浏览器验证尚未完成。

含文本的图纸在导入报告里不会被报告为 `Complete`（导入阶段不知道宿主是否有字体）；
在 `build-representation` 注入字体后文本即可绘。

