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
| VIEWPORT | ✅ | `viewport_semantics`：纸空间视口边框矩形（模型内容由 plot 装配） |
| TOLERANCE | ⚠️ Partial | `tolerance_frame`+文本：公差框（框宽按字符数估计）+ 文字，显式 `Partial` |
| MLINE | ⚠️ Partial | `mline_semantics`：顶点中心线；**逐元素偏移/接合未实现**，显式 `Partial` |
| MULTILEADER | ⚠️ Partial | `multileader_semantics`：leader-root 折线 + 注解文字；块内容未展开，显式 `Partial` |
| RAY / XLINE | ✅ | `ray_semantics`：按运行中模型 bounds 裁剪到 XY 盒；无范围时默认盒并 `Partial` |
| SHAPE | ✅ | SHX 字形：`FontEngine::shape_glyph` 经 SHX `glyph_by_code` 生成折线；缺字体/按名引用显式 `Partial` |
| TABLE | ⚠️ Partial | 单元格网格 + 单元格文字；合并单元格/边框样式近似 |
| RASTERIMAGE | ⚠️ Partial | 图像边框（insertion/u/v/size）；像素纹理未解码 |
| DIMENSION 角度/坐标/圆弧长/大半径 | ✅/⚠️ | 角度(2Ln/3Pt)/坐标/弧长已画；大半径折线 jog 近似 `Partial` |

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
| MULTILEADER | ✅ | ⚠️ Partial | 引线折线+文字已画；样式/块内容/overrides 未完整 |
| MLINE | ✅ | ⚠️ Partial | 中心线已画；逐元素偏移/接合未实现 |
| TOLERANCE | ✅ | ⚠️ Partial | 框+文字已画；框宽按字符数估计，符号字形依赖字体 |
| SHAPE | ✅ | ✅ | SHX 字形已接线；按名引用（无 shape code）仍 `Partial` |
| TABLE | ✅ | ⚠️ Partial | 网格+文字已画；合并单元格/边框样式近似 |
| RASTERIMAGE | ✅ | ⚠️ Partial | 图像边框已画；**像素纹理未解码**（需图像解码+纹理管线） |
| RAY / XLINE | ✅ | ✅ | 裁剪到模型 bounds；无范围回退为默认盒 `Partial` |
| DIMENSION 子类 | ✅ | ✅/⚠️ | 线性/对齐/半径/直径/角度/坐标/圆弧长已画；大半径 jog 近似 `Partial` |
| UNDERLAY / OLE2FRAME | ✅ | ❌ | 外部参照/嵌入对象，无本地内容 |
| LIGHT / SECTION / VIEWBORDER / SEQEND | ✅ | ❌ | 非绘制或注释对象；保持显式未实现 |
| 纸空间 `plot` | ✅ | ✅（视口合成近似） | 修复两处：①纸张单位从标准纸名解析（锁定 acadrust 未把 `group 72` 应用到 LAYOUT，曾把 210mm 当 210in → 纹理超限/空白）；②默认选有 viewport 的纸空间布局（flange 的首个 `*Paper_Space` 为空）。纸张/边框/标题栏已出图；视口比例/位置仍为近似 |
| DXF/X2D 字体提示 | — | ❌ | QCAD 把真实 TTF（如 `Arial`）放在 STYLE 的 XDATA `1000`；锁定的 acadrust `TextStyle` 不暴露该字段。**不可修改 acadrust/不加 patch** |

`entity.rs` 的 `convert` **仍未覆盖**、会落 `Opaque`/`Unsupported` 的 `EntityType`
分支仅剩：`Underlay`、`Ole2Frame`（外部参照/嵌入对象，无本地内容）与
`Light`、`SectionSymbol`、`ViewBorder`、`Seqend`（非绘制/注释对象）。
其余可绘制图元均已接线（见 §2），其中 `MultiLeader`/`MLine`/`Tolerance`/`Table`/
`RasterImage`/大半径为 `Partial`。

## 5. 图元支持验证（ezdxf 对比已放弃）

按用户要求**放弃 ezdxf 像素对比**。改为图元级支持验证：

- `crates/cad-cli-tools/tests/dxf_samples.rs::committed_qcad_examples_have_no_unsupported_entity_types`
  对语料每个 DXF 跑 `proxy-report`，断言没有任何 `render: unsupported` 的图元类型；
  `committed_qcad_examples_import_and_build_without_failures` 保证解析/表示不失败。
- `scripts/check-qcad-examples.py` 批量跑同一套并打印 `unsupported entity types`，当前为
  `none`；`--no-render` 时无需 GPU。
- 运行：`cargo test -p cad-cli-tools --test dxf_samples --locked`。

（`--export-references` 的 ezdxf 导出保留为可选工具，但不再作为验收路径。）

QCAD 字体：`osifont.ttf`（GPLv3）与 `Standard/ltypeshp/qcadshp.cxf`（public domain）已下载
到本机缓存 `~/sources/cad-test-fonts/qcad/`（**不入库**；CXF 当前引擎不支持）。文本仍
受 §4 的 XDATA 字体提示限制，因此 `Partial`，但图元类型本身**已支持**。

## 6. 结论

- QCAD examples 语料：**解析/表示全部通过，`proxy-report` 零 `unsupported`**；`partial`
  只剩字体/subclass 等保真度问题。
- 与 opencadstudio 图元清单相比：`LEADER/Polyline/ATTRIB/Mesh/Polyface/Polygon/Wipeout/
  Helix/Viewport/Tolerance/MLine/MultiLeader/Ray/XLine/Shape/Table/RasterImage` 与全部
  DIMENSION 子类均已接线，其中 6 类为 `Partial`（表/大半径/多线/多引线/公差/光栅纹理）。
  仍**未覆盖** `Underlay/Ole2Frame/Light/SectionSymbol/ViewBorder/Seqend`（外部/非绘制）
  与真实材质图像纹理，见 §4。**接近但尚未 100% 覆盖 opencadstudio 的可绘制图元。**
