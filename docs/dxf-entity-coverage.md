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
| WIPEOUT | ⚠️ Partial | `wipeout_semantics` 发 `SemanticGeometry::Mask`（世界坐标边界 + `inverted`）；**掩码填充未渲染**（`inverted` 未被消费），显式 `Partial` |
| HELIX | ✅ | `convert` 复用 `spline_semantics(&h.spline)` |
| VIEWPORT | ✅ | `viewport_semantics`：纸空间视口边框矩形（模型内容由 plot 装配） |
| TOLERANCE | ⚠️ Partial | 公差框 + 文字；框高按 `text_lines()` 行数，**框宽仍按字符数估计**，显式 `Partial` |
| MLINE | ⚠️ Partial | `ImporterBuilder::mline_semantics`：从 `doc.objects` 的 `MLinestyle` 解析元素偏移/线型/`flags`，画 justification/scale 基准下的逐元素平行线、方/圆 caps 与 joins；精确 miter、`CLOSED`、cap 角度、`start_point`、`FILL_ON`、元素颜色/线型未应用，显式 `Partial` |
| MULTILEADER | ⚠️ Partial | `multileader_semantics`：`MLeaderStyle` 解析、箭头三角、`path_type`（Straight/Spline/Invisible）、dogleg/landing、文本附着枚举；颜色/线宽/背景填充/列/块属性/自定义箭头/transform 未应用，显式 `Partial` |
| RAY / XLINE | ✅ | `ray_semantics`：按运行中模型 bounds 裁剪到 XY 盒；无范围时默认盒并 `Partial` |
| SHAPE | ✅ | SHX 字形：`FontEngine::shape_glyph` 经 SHX `glyph_by_code` 生成折线；缺字体/按名引用显式 `Partial` |
| TABLE | ⚠️ Partial | 单元格网格 + 文字 + 合并单元格跨格；**按 `CellStyle`/`CellBorder` 逐边绘制**（`invisible` 跳过、`double_spacing` 双线、合并抑制内边）；逐边颜色/线宽、`override_flags`、`border_type`、`additional_borders` 未应用 |
| RASTERIMAGE | ✅/⚠️ | **新增端到端纹理显示**：importer 发 `SemanticGeometry::Image`；host 按相对路径解析 + PNG/JPEG 解码（`cad-resources`）；renderer 上传纹理（每键去重、UV 方向修正、裁剪多边形）；缺纹理→frame + `image.unresolved`；不支持编解码器、亮度/对比/淡出、outside 裁剪→`Partial` |
| DIMENSION | ✅/⚠️ | 线性/对齐/半径/直径/角度(2Ln/3Pt)/坐标/弧长 `Complete`（弧长画引线）；`LargeRadial` 用 `jog_angle` 定向仍 `Partial`；`is_partial` 弧标注 `Partial` |
| EXTENDED（RTEXT / ARCALIGNEDTEXT / GEOPOSITIONMARKER / SECTIONOBJECT / POINTCLOUD） | ⚠️ Partial | 基础表示：RTEXT→Text（flags/非 ±Z OCS 法向未处理则 `Partial`）、ARCALIGNEDTEXT→直线近似 Text、GEOPOSITIONMARKER→半径圆+注记+内嵌 MText、SECTIONOBJECT→折线、POINTCLOUD→范围盒（无点数据） |
| CAMERA / 动态块参数与夹点 / COORDINATION_MODEL / OLEFRAME / LAYOUTPRINTCONFIG / FORMAT / LEGACY / REGISTEREDCLASS | ⛔ 无显示 | 按设计无几何或为外部内容，保持 `Opaque`/`Unverified`（见 §4） |
| OLE2FRAME / UNDERLAY / VIEWBORDER / SECTIONSYMBOL / LIGHT | ✅/⚠️ | 外框/裁剪边界/符号折线/灯光字形；外部内容不加载，均 `Partial` |

文本能力判定修正：Text（TEXT/MTEXT/ATTRIB/DIMENSION 文字）统一为 `Unverified`——
实体类型**已支持**，能否出字形取决于宿主是否提供字体（`--font name=path`）；不再把
字体名缺失当成 `Unsupported`。

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
ARC、CIRCLE、POINT、ELLIPSE、HATCH、LEADER、SOLID、VIEWPORT（边框）。`proxy-report`
对 9 个语料 + `flange` **零 `unsupported`**，由
`crates/cad-cli-tools/tests/dxf_samples.rs::committed_qcad_examples_have_no_unsupported_entity_types`
固定。remaining `partial` 只剩字体/子类等保真度问题，不再是“图元没实现”。

## 4. 已知缺口（未完成，不得当作通过）

| 图元 / 能力 | opencadstudio | 本项目 | 原因与状态 |
|---|---|---|---|
| MULTILEADER | ✅ | ⚠️ Partial | 引线折线+文字+箭头+样式/path type/dogleg/对齐已画；颜色/线宽/背景填充/列/块属性/自定义箭头未应用 |
| MLINE | ✅ | ⚠️ Partial | 逐元素偏移+caps+joins+justification/scale 已画；精确 miter、CLOSED、cap 角度、start_point、FILL_ON、元素颜色/线型未应用 |
| TOLERANCE | ✅ | ⚠️ Partial | 框+文字已画；框高按行数、框宽按字符数估计，符号字形依赖字体 |
| SHAPE | ✅ | ✅ | SHX 字形已接线；按名引用（无 shape code）仍 `Partial` |
| TABLE | ✅ | ⚠️ Partial | 网格+文字+合并+按 `CellStyle`/`CellBorder` 逐边已画；逐边颜色/线宽、override_flags、border_type、additional_borders 未应用 |
| RASTERIMAGE | ✅ | ✅/⚠️ | **端到端纹理显示已接**（host 解码 PNG/JPEG + GPU 纹理，每键去重/UV 修正/裁剪多边形）；不支持编解码器（TIFF/CCITT/EPS）、亮度/对比/淡出、outside 裁剪为 `Partial`；真实光栅文件未验收 |
| WIPEOUT 掩码 | ✅ | ⚠️ Partial | `Mask` 语义（边界+inverted）已发；**掩码填充未渲染**，透明覆盖顺序洞仍在 |
| EXTENDED 基础表示 | ✅ | ⚠️ Partial | RTEXT/ARCALIGNEDTEXT/GEOPOSITIONMARKER/SECTIONOBJECT/POINTCLOUD 基础表示已画（多为 `Partial`）；CAMERA/动态块参数夹点无几何、外部内容不加载 |
| RAY / XLINE | ✅ | ✅ | 裁剪到模型 bounds；无范围回退为默认盒 `Partial` |
| DIMENSION 子类 | ✅ | ✅/⚠️ | 线性/对齐/半径/直径/角度/坐标/圆弧长已画（弧长含引线）；大半径 jog 定向但仍 `Partial`，`is_partial` 弧标注 `Partial` |
| OLE2FRAME / UNDERLAY / VIEWBORDER / SECTIONSYMBOL / LIGHT | ✅ | ✅/⚠️ | 外框/裁剪边界/符号/灯光字形已画；外部内容不加载，均 `Partial` |
| 绘制顺序 `draw_order` | ✅ | ⛔ 未贯通 | `DbEntity::draw_order` 未经 `DisplayFragment` 传到 `RenderBatch`（本轮正式推迟）；绘制顺序=上传顺序 |
| 纸空间 `plot` | ✅ | ✅（视口合成近似） | 修复三处：①纸张单位优先从标准纸名解析（acadrust 实际会把 `group 72` 应用到 LAYOUT，但纸名带单位记号时更可靠）；②默认选有 viewport 的纸空间布局；③acadrust 亦应用 `group 73` 旋转，仅在文件未声明旋转时才按 viewport 范围把纸张轴交换为横向（标题栏保持正立）。纸张/边框/标题栏已出图；**模型视图仍有错位/多余图元**（视口合成保真未通过） |
| DXF STYLE XDATA 字体 | — | ✅（DXF/DWG） | acadrust 0.6.3 已把 STYLE 的 `1001 ACAD`/`1000` 字体面类型化为 `TextStyle.true_type_font`（DXF 读取；DWG 经 `io/dwg/typeface_eed.rs`）。解析链为 `类型化 true_type_font > group 3/4 声明字体 > dxf_style_xdata_fonts 字节扫描`；字节扫描自身仍不覆盖组件 group 3/4（不改 acadrust）。DXF/DWG 文本现可按宿主 `--font <face>=<path>` 解析 |

`entity.rs` 的 `convert` 现覆盖所有可绘制 `EntityType`；仅结构性 `Block`/`BlockEnd`/
`Seqend` 不产生几何（本来就不应绘制）。`MultiLeader`/`MLine`/`Tolerance`/`Table`/
`RasterImage`/外部对象/大半径为 `Partial`。

## 5. 图元支持验证（ezdxf 对比已放弃）

按用户要求**放弃 ezdxf 像素对比**。改为图元级支持验证：

- `crates/cad-cli-tools/tests/dxf_samples.rs::committed_qcad_examples_have_no_unsupported_entity_types`
  对语料每个 DXF 跑 `proxy-report`，断言没有任何 `render: unsupported` 的图元类型；
  `committed_qcad_examples_import_and_build_without_failures` 保证解析/表示不失败。
- `scripts/check-qcad-examples.py` 批量跑同一套并打印 `unsupported entity types`，当前为
  `none`；`--no-render` 时无需 GPU。
- 运行：`cargo test -p cad-cli-tools --test dxf_samples --locked`。

（`--export-references` 的 ezdxf 导出保留为可选工具，但不再作为验收路径。）

QCAD 字体：`osifont.ttf`（GPL-3 + 字体例外）已提交到 `fonts/` 作为默认轮廓回退面；
`Standard/ltypeshp/qcadshp.cxf`（public domain）留在本机缓存（**不入库；CXF 当前引擎不支持**）。
文本仍因**宿主字体缺失时走回退**而标 `Partial`，但图元类型本身**已支持**，STYLE xdata
字体面也已由 acadrust 0.6.3 类型化（DXF+DWG）。

## 6. 结论

- QCAD examples 语料：**解析/表示全部通过，`proxy-report` 零 `unsupported`**；`partial`
  只剩字体/subclass 等保真度问题。
- 与 opencadstudio 图元清单相比：**所有可绘制 `EntityType` 均已接线**（外部/非绘制
  对象给出外框/符号/字形并标 `Partial`），仅结构性 `Block/BlockEnd/Seqend` 不产生几何。
- 2026-10-10 轮：**光栅图像（RASTERIMAGE）已端到端显示**（host 解码 PNG/JPEG + GPU 纹理
  管线，含每键纹理去重、UV 方向、裁剪多边形、缺失显式 `Partial`）；非 proxy `Extended`
  图元补齐基础表示；MLINE/TABLE/DIMENSION/MULTILEADER/HATCH 边界保真提升。
- 仍有保真度缺口（`Partial`）：WIPEOUT 掩码填充、图像不支持编解码器/亮度对比淡出/outside
  裁剪、TABLE 逐边样式、MLINE CLOSED/cap 角度、MULTILEADER 背景填充/列/块属性、大半径
  jog、外部内容加载、纸空间视口合成，见 §4。
- **显式不可实现**（记录、不伪造）：代理 opcode 厂商记录（本轮排除）、外部 PDF/DWF/DGN
  underlay 与 OLE2 嵌入、CoordinationModel/Navisworks NWD、ACIS 全内核、点云点数据、
  完全顺序正确的 WIPEOUT 透明覆盖掩码。STYLE xdata 字体名已由 acadrust 0.6.3 类型化
  （DXF+DWG）。
