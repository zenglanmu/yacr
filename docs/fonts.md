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

## 未完成

字体目录只是接线。**字形渲染尚未实现**：`cad-scene` 仍跳过 `Text` 基元，文字不会
出现在画布上。后续需要：

1. SHX（编译 shape 字体）与 outline（TTF/WOFF，WOFF 需解压）的字形轮廓提取；
2. 文本排版（字距、行距、MTEXT 格式码、旋转、对齐）；
3. 轮廓 → 折线/网格，接入 `cad-scene` 批次；
4. 缺失字体与替代字体的显式诊断。

在此之前，含文本的图纸不会被报告为 `Complete`（见 `docs/validation.md`）。
