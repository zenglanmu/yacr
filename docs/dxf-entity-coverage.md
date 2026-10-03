# DXF 图元覆盖矩阵（QCAD examples / opencadstudio 对照）

本轮用户要求：把 QCAD `examples/` 的 DXF 全部作为解析/渲染测试样本，覆盖 QCAD 所列
DXF 图元与 `../opencadstudio` 当前支持的图元，并要求“务必实现完整”。本文如实记录
**本轮实际完成的范围**，以及尚未覆盖的部分与原因；不得把本文当作完整兼容声明。

## 1. 样本与工具

- 语料：`fixtures/dxf/qcad-examples/`（9 个 DXF）+ `fixtures/dxf/qcad-flange/`（flange）。
  来源/SHA/许可见 `fixtures/dxf/qcad-examples/SOURCE.md`、`fixtures/manifest`。
- 解析/表示回归：`crates/cad-cli-tools/tests/dxf_samples.rs`（无需 GPU，进默认测试）。
- 出图与覆盖报告：`scripts/check-qcad-examples.py`（原生 lavapipe；`--export-references`
  时用可选 `ezdxf`+`matplotlib` 导出参考 PNG，见 §5）。
- 图元实现位置：`crates/cad-import-acadrust/src/{entity,dimension,primitives,geometry}.rs`。

## 2. 本轮新实现的图元

| 图元 | 实现 | 说明/契约测试 |
|---|---|---|
| LEADER | ✅ | `dimension.rs::leader_semantics`：顶点折线 + 顶点实心箭头（DIMSTYLE DIMASZ）；spline 路径按直线画并 `Partial`，hookline、离面法向显式 `Partial` |
| POLYLINE（通用） | ✅ | `primitives.rs::polyline_semantics`：world 顶点折线，closed 来自 flags |
| ATTRIB / ATTDEF | ✅ | `entity.rs::attribute_text`：按插入点/字高/旋转/样式生成 Text（ATTRIB 用 value，ATTDEF 用默认值或 tag） |
| MESH（subdivision） | ✅ | `primitives.rs::subd_mesh_semantics`：faces 扇形三角化 |
| POLYFACE MESH | ✅ | `polyface_mesh_semantics`：1-based/负索引边、四边面扇形三角化 |
| POLYGON MESH | ✅ | `polygon_mesh_semantics`：M×N 网格→四边形→三角 |
| WIPEOUT | ⚠️ Partial | `wipeout_semantics`：画裁剪边界闭合折线；**遮罩填充未实现**，显式 `Partial` |
| HELIX | ✅ | `convert` 复用 `spline_semantics(&h.spline)` |

另外修正文本能力判定：带字体名的 Text 现在是 `Unverified`（宿主可用 `--font name=path`
提供）而不是 `Unsupported`；只有完全没有字体名才是 `Unsupported`。

## 3. QCAD examples 语料覆盖

`scripts/check-qcad-examples.py` 实际执行结果（9/9 passed；lavapipe；文本字体按名注册）：

| 文件 | 导入/表示 | 完整性 |
|---|---|---|
| calibration.dxf | 通过 | complete（模型空间） |
| colors.dxf | 通过 | partial（MTEXT 字体，见 §4） |
| entities.dxf | 通过 | partial（BlockReference/Dimension/MTEXT；LEADER 已支持） |
| example00.dxf | 通过 | complete |
| example01.dxf | 通过 | partial（同上） |
| isometric_grid.dxf | 通过 | partial（Dimension 文字） |
| linetypes.dxf | 通过 | partial（MTEXT 字体） |
| lineweights.dxf | 通过 | partial（MTEXT 字体） |
| projection.dxf | 通过 | complete |

语料中出现且已实现的图元：LINE、LWPOLYLINE、MTEXT、INSERT、SPLINE、DIMENSION、
ARC、CIRCLE、POINT、ELLIPSE、HATCH、LEADER、SOLID。没有 `Unsupported` 的模型空间
几何（仅 calibration 的纸空间 `VIEWPORT` 仍 `Unsupported`，属布局/视口范围）。

## 4. 已知缺口（未完成，不得当作通过）

| 图元 / 能力 | opencadstudio | 本项目 | 原因与状态 |
|---|---|---|---|
| MULTILEADER | ✅ | ❌ `Partial`/Opaque | 需要 mleader 样式、内容块、多引线；未实现 |
| MLINE | ✅ | ❌ | 多线偏移/接合未实现 |
| SHAPE | ✅ | ❌ | 需要 SHX shape 引用与字体；未接线 |
| TABLE | ✅ | ❌ | 表格栅格/文字未实现 |
| TOLERANCE | ✅ | ❌ | 形位公差框/符号未实现 |
| RASTERIMAGE | ✅ | ❌ | 图像定义/贴图未接线（数据库有 `images` 通道） |
| UNDERLAY / OLE2FRAME | ✅ | ❌ | 外部参照/嵌入对象，无本地内容 |
| RAY / XLINE | ✅ | ❌ | 无限直线需要视口裁剪；未实现 |
| VIEWPORT（纸空间） | ✅ | ❌（`plot` 空白） | 纸空间出图本轮为空白，未定位，独立缺口 |
| LIGHT / SECTION / VIEWBORDER / SEQEND | ✅ | ❌ | 非绘制或注释对象；未实现 |
| DIMENSION 子类 | ✅ | ⚠️ Partial | 仅线性/对齐/半径/直径；角度/坐标/圆弧长/大半径未实现 |
| DXF/X2D 字体提示 | — | ❌ | QCAD 把真实 TTF（如 `Arial`）放在 STYLE 的 XDATA `1000`；锁定的 acadrust `TextStyle` 不暴露该字段。**不可修改 acadrust/不加 patch**，故 colors/linetypes/lineweights 等 MTEXT 无字体名可解析 |

`entity.rs` 的 `convert` 未覆盖的 `EntityType` 分支（→ `Opaque`/`Unsupported`，按
上面矩阵）：MultiLeader、MLine、Table、Tolerance、Shape、Ray、XLine、RasterImage、
Underlay、Ole2Frame、Light、SectionSymbol、ViewBorder、Viewport、Seqend。

## 5. ezdxf 参考导出与宿主字体

`scripts/check-qcad-examples.py --export-references` 在本机安装 `ezdxf`+`matplotlib`
时，用 `ezdxf.addons.drawing.matplotlib` 为每个 DXF 导出 `*-ezdxf.png` 供人工/粗粒度
对照；缺包时该项记为 **NOT RUN**，不伪造参考图。本轮代理网络多次中断，`uv pip install
ezdxf matplotlib` 未成功，故参考导出 **NOT RUN**；`flange` 有上游 PNG/SVG/PDF 可直接用
（见 `docs/validation-dxf-flange.md`）。

QCAD 字体：`osifont.ttf`（GPLv3，含 `osifont_license.txt`）与 `Standard/ltypeshp/
qcadshp.cxf`（QCAD LICENSE.txt 声明为 public domain）已下载到本机字体缓存
`~/sources/cad-test-fonts/qcad/`（**不入库**；CXF 当前字体引擎不支持，仅 osifont.ttf 可
直接当 TTF 回退）。语料文本真正缺的是 §4 的 XDATA 字体提示：多数 QCAD STYLE 的 group 3
为空、真实 TTF 名在 XDATA `1000`，锁定的 acadrust `TextStyle` 不暴露该字段，因此
colors/linetypes/lineweights 的 MTEXT 无字体名可解析，仍 `Partial`。

## 6. 结论

- QCAD examples 语料：**解析与表示全部通过，模型空间无 `Unsupported` 几何**；文本受
  §4 的 XDATA 字体限制，许多 MTEXT 仍 `Partial`。
- 与 opencadstudio 的图元清单相比：本轮补齐了其中一批（LEADER/Polyline/ATTRIB/
  Mesh/Polyface/Polygon/Wipeout/Helix），但**尚未达到“完全覆盖”**；剩余项见 §4，
  不得声称已完整实现。
