# Bentley DGN 导入（V7 ISFF / V8 OLE-CFB）

> **状态（2026-11-14）：矢量文档导入已接线，但只有公开玩具样本的契约证据。**
> DGN 是继 DWG/DXF 之后第二条**矢量文档** lane，不是 OBJ/STEP/IFC/RVT/SKP 的网格预览 lane：
> 它把 level、线型、文字样式、线/弧/椭圆/样条/文字、共享单元（shared cell）与单位写进
> `cad-db`，而不是先三角化成网格。绘制/拾取对精确几何成立，测量对线/折线/圆/弧/椭圆/样条/
> 点成立，对文字、占位栅格与块参照保守降级（见下表）。

## 1. 依赖与许可

- `cadkit-dgn` **`=0.5.2`**（MIT OR Apache-2.0，纯 Rust，`unsafe` 禁止，wasm 可编译），
  只由 `cad-import-dgn` 依赖。
- `default-features = false`：关闭 seed-based V8 writer 与可选 `cfb` 依赖；读取器自带 CFB
  解析，所以读取路径仍是纯 Rust。
- `cadkit-core =0.5.2` 是它的中性文档模型（`Document`/`EntityKind`），直接依赖以便映射类型。
- **未修改上游源码、无 Cargo patch、无 vendoring**。crate 很新（2026-10-01 首发，31 天内
  15 个版本，单一维护者），所以精确固定版本；0.x 允许破坏性变更，升级必须重新核对本文件
  与契约测试。
- 上游已知的 V8 风险（2026-11-12 仍有 open issue）：椭圆 `ratio=0`、隐藏元素被判可见、
  UTF-16LE+BOM level 名、复合元素嵌套、type-11 曲线切线。这些不是本仓库能修复的；遇到时
  走 `cadkit-dgn` 的 warning / 本适配器的显式拒绝，绝不静默改成“成功”。

## 2. 检测与路由

`cad-import-core::detect_format` 仍只做保守嗅探：

- **DGN V7**：固定魔数 `08 05 17 00`，或第二字节 `09 FE 02` 且首字节为 `08`/`C8`。
- **DGN V8**：OLE/CFB 魔数 `D0 CF 11 E0 A1 B1 1A E1` 且根存储含 `Dgn~H`。
- CFB 无法只凭魔数区分 Revit 与 DGN V8，因此检测层返回 `DrawingFormat::Ole`；
  `cad_app::import::FormatImporter` 是唯一派发点，先问 `DgnImporter::sniff`（`cadkit_dgn::sniff`
  会解析 CFB 目录），是 DGN 就走 DGN，否则走 RVT。**不按扩展名猜格式**；扩展名只用于文件
  选择器与 OS 关联。
- 其它 carrier（Web wasm、Android SAF、CLI）走同一个 `FormatImporter`，所以打开路径只有一条。

## 3. 映射

| DGN | `cad-db` | 说明 |
|---|---|---|
| level | `Layer` | 只写名称与可见性；cadkit **不解码** level 颜色/线宽/线型，因此不写 `LayerAttributes`（默认行会伪装成真实数据），记 `import.dgn.level_symbology_undecoded` |
| linetype | `LineType` | `DGN_SOLID`/`DGN_DOT_DASH` 等的 dash 段；**cadkit 的 pattern 是 UOR**，按 `dgn.uor_per_master` 缩放到文档主单位；缺因子时退 continuous + 诊断 |
| font/text style | `Style` + `SemanticGeometry::Text.style/font` | 字体名作为 resource key；`DGN_FONT_n` 没有字体名 |
| shared cell definition | `BlockDefinition` + `BlockMetadata` | 名称与基点；成员是 `SpaceId::Block(id)` 的实体行 |
| shared cell instance | `SemanticGeometry::Insert` | `T(position) · R(normal,rotation) · S(scale) · T(-base)` |
| Line / LineString / Shape | `Line` / `Polyline` | bulge 精确保留；闭合标志保留 |
| Circle / Arc / Ellipse | 同名语义 | 法向、半径/长短轴、起角/扫掠精确保留 |
| B-spline | `Spline`（NURBS） | 控制点/节点/权重；只有 fit point 时降为折线并记 Partial |
| Text / MText | `Text` | 对齐、字高、旋转、宽度因子；MText 的格式码不保留 |
| Point / PointString | `Point` / 逐点实体 | point string 的成员按父级样式继承 |
| Complex chain/shape、text node、cell group | **成员扁平化** | `cad-db` 没有 group 容器；几何精确，group 编辑语义不保留 |
| Raster frame | `SemanticGeometry::Image`（`size_px` 已知时）或 `Opaque`（未知时） | cadkit 的 `u_vector`/`v_vector` 是整幅宽高，domain 的 `u`/`v` 是**每像素**，所以除 `size_px`；像素在外部 TIF，不加载；`size_px` 未知时不伪造 1×1 |
| Unknown element | 跳过 + 诊断 | 全部元素都未知 ⇒ `Unsupported`，不是空成功 |

- **单位**：`cadkit` 已把 UOR 转成首个模型的主单位。按 `units.meters_per_unit` **精确匹配**
  `1.0`（m）/`0.001`（mm）/`0.0254`（in）/`0.3048`（ft），作为 `UnitContext` 的 source/display
  （ratio 1.0）；`Unitless` 用 `UnitContext::drawing_units()`；其它值（含 US survey foot、`Custom`）
  回退 drawing units 并记 `import.dgn.units_unsupported`。**绝不假设毫米，也不把 survey foot
  当国际英尺。**
- **编码**：8-bit 文本由 cadkit 按文件声明或 windows-1252 回退解码，`source.codepage` 作为
  `import.dgn.text_encoding` 记录；`ImportRequest` 没有 codepage 通道，未声明 GBK/Big5 的 V7
  可能乱码，这是显式边界。
- **解压预算**：`ImportLimits.max_decompressed_bytes = None` 时保留 cadkit 自己的默认（4 GiB）；
  `Some(n)` 逐字透传。`max_file_bytes` 在 dispatcher 的 `Ole` 分支调用 `sniff` **之前**检查，
  避免对超大 CFB 先走目录解析。
- **多模型**：只导入 `Document::model_space()`；其它模型跳过并记诊断 + Partial。仓库没有
  多模型/图纸切换，把 sheet 模型叠进模型空间会是假内容。
- **隐藏元素**：`Entity.visible=false` 写入 `builder.set_hidden_entities`，不删除；level 的
  display/freeze/lock 标志（V7/V8）上游尚未解码，因此所有 level 都按可见导入，并记
  `import.dgn.level_symbology_undecoded` + **每个成功导入都是 Partial**（与 RVT/SKP 的 preview
  诚实通道同型）。
- **标签/Item Types/XAttributes**：`cad-db` 没有通用属性表；标签存在时记
  `import.dgn.tags_not_stored` + Partial，**不**塞进几何或伪造属性行。
- **颜色**：DGN palette 已由 cadkit 解析成 sRGB；>255 的扩展色上游回退 `ByLayer`，本适配器
  无法区分，按文档边界声明，不声称 true color 完整。

## 4. 能力表

| 源元素 | 读取 | 语义 | 绘制 | 拾取 | 测量 |
|---|---|---|---|---|---|
| Line / Polyline / Circle / Arc / Ellipse / Spline / Point | 实现 | 实现（有退化时 Partial） | 实现 | 实现 | 实现 |
| Text / MText | 实现 | 实现（MText 格式码丢失则 Partial） | 未验证（依赖宿主字体） | 未验证 | 不支持 |
| Insert（shared cell） | 实现 | 实现（缺块时跳过 + Partial，不伪造空块） | 未验证（成员能力取最弱） | 未验证 | 不支持 |
| Raster frame | 实现（仅放置） | Partial（外部像素未加载） | 不支持（Image 只引用外部文件；`size_px` 未知时为 Opaque） | 不支持 | 不支持 |
| Unknown element | 实现（仅识别） | 不支持 | 不支持 | 不支持 | 不支持 |

`read` 恒为 `Verified` 只表示“字节被读取器接受”，不表示所有元素都被建模。

## 5. 稳定诊断

`import.dgn.not_a_dgn_file`、`import.dgn.no_geometry`、
`import.dgn.unknown_elements_only`、`import.dgn.unknown_element`、
`import.dgn.element_not_modelled`、`import.dgn.tags_not_stored`、
`import.dgn.group_flattened`、`import.dgn.model_skipped`、
`import.dgn.level_symbology_undecoded`、`import.dgn.raster_not_loaded`、`import.dgn.spline_fit_points_polyline`、
`import.dgn.text_formatting_flattened`、`import.dgn.text.align_unsupported`、
`import.dgn.text.plane_not_supported`、`import.dgn.text.width_factor`、
`import.dgn.text_encoding`、`import.dgn.geometry_rejected`、
`import.dgn.block_missing`、`import.dgn.cell_mirrored_z`、`import.dgn.depth_limit`、
`import.dgn.linetype_unknown`、`import.dgn.linetype_pattern_unscaled`、`import.dgn.layer_unknown`、
`import.dgn.units_unsupported`、`import.dgn.units_unitless`、`import.dgn.units_<name>`、
`import.dgn.limits_exceeded`。

诊断消息是英文内部文本；用户可见的状态/标签仍走 i18n 目录。**每个成功导入恒为 `Partial`**
（level 显示标志未解码是结构性损失），reason 聚合到 ≤5 条稳定原因；**没有几何或只有未知元素时
是 `Unsupported`，不是空成功**。

## 6. 与 DGNATTACH / DGNIMPORT 的区别

- 打开 `.dgn` **文档**现在可用（本 lane）。
- `DGNIMPORT`/`-DGNIMPORT`（把 DGN 合并进当前 DWG 的映射模型）、`DGNATTACH` 底图、
  `DGNCLIP`/`DGNLAYERS`/`DGNADJUST`、`DGNEXPORT`/`-DGNEXPORT` **仍是具名拒绝**。它们需要
  另一套“导入到当前文档”的映射/事务语义，不能因为 OPEN 支持了就宣称它们可用。
- 同理，DGN 也**不是**可写格式：没有 DGN 导出。

## 7. 证据边界

- 提交的夹具只有 GDAL 的 `smalltest.dgn`（V7，4 个图形）与 `test_dgnv8.dgn`（V8，27 KB，
  **ODA Teigha 4.02 写出，不是 MicroStation**）；来源、SHA-256 与许可见
  `fixtures/dgn/SOURCE.md` 与 `fixtures/manifest`。
- 22 条 crate 契约测试（`cargo test -p cad-import-dgn`）覆盖：V7 的 text/circle/polyline/line
  与单位；V8 的 metres、3 个 level、shared cell 定义/Insert 位置与 10000 缩放、group 扁平化、
  2 个 Unknown 类型、tags、恒 Partial；UOR 线型缩放为 0.00045 m；layer 不写伪造属性；
  **每个导入实体经 `apply_drawing_changes` 写回成功**；非 DGN/超文件/超实体/取消拒绝；
  未知-only、无几何、fit-point spline、缺块 Insert、循环/超深 shared cell 引用（不发出递归
  `Insert`）、栅格 per-pixel u/v 与未知像素尺寸、ratio=0 椭圆、zero-sweep 弧、完整椭圆。**不证明** V8i/CONNECT/ORD、真实 MicroStation 文件、
  level 显示标志、references、Item Types、尺寸/多线/网格/实体、栅格像素或字体度量。
- **真实 GPU 视觉运行（2026-11-14）**：Linux GNOME/Mutter Wayland 会话内启动嵌套 Weston 14，
  `target/debug/yacr --open fixtures/dgn/{smalltest,test_dgnv8}.dgn` 以
  `GDK_BACKEND=wayland` + `WGPU_BACKEND=vulkan` + NVIDIA ICD + `DRI_PRIME=1` 运行；probe 报告
  `backend=vulkan adapter=Quadro P620 device_type=discrete_gpu`，`bridge wayland`（native-below）
  且 `frames` 持续增长。V7/V8 截图、probe 与命令见 `docs/evidence/dgn-linux-gpu/`。这是**视觉/present 证据**，
  不是 golden/像素/性能或 Bentley 兼容性验收；debug 构建 + 嵌套合成器是两条边界。
- 真实文件只通过 `YACR_TEST_DGN=<绝对路径>` 本地验证；未设置时的跳过 **不是已运行证据**。
- `cargo check -p app-web --target wasm32-unknown-unknown` 只证明可编译；没有浏览器/像素验收。
  Windows/macOS/Android 的 DGN 运行证据一律 **NOT VERIFIED**。
