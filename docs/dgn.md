# Bentley DGN 导入（V7 ISFF / V8 OLE-CFB）

> **状态（2026-11-14）：矢量文档导入已接线，但只有公开玩具样本的契约证据。**
> DGN 是继 DWG/DXF 之后第二条**矢量文档** lane，不是 OBJ/STEP/IFC/RVT/SKP 的网格预览 lane：
> 它把 level、线型、文字样式、线/弧/椭圆/样条/文字、共享单元（shared cell）与单位写进
> `cad-db`，而不是先三角化成网格。绘制/拾取对精确几何成立，测量对线/折线/圆/弧/椭圆/样条/
> 点成立，对文字、占位栅格与块参照保守降级（见下表）。

## 1. 依赖与许可

- `cadkit-dgn` **`=0.5.4`**（MIT OR Apache-2.0，纯 Rust，`unsafe` 禁止；上游自证 wasm
  可编译，本轮 wasm32 编译 **NOT RUN**），只由 `cad-import-dgn` 依赖。
- `default-features = false`：关闭 seed-based V8 writer 与可选 `cfb` 依赖；读取器自带 CFB
  解析，所以读取路径仍是纯 Rust。
- `cadkit-core =0.5.4` 是它的中性文档模型（`Document`/`EntityKind`），直接依赖以便映射类型。
- **未修改上游源码、无 Cargo patch、无 vendoring**。crate 很新（2026-10-01 首发，31 天内
  15 个版本，单一维护者），所以精确固定版本；0.x 允许破坏性变更，升级必须重新核对本文件
  与契约测试。**2026-11-21 由 0.5.2 升到 0.5.4**：唯一真实破坏点是 `Linetype::pattern`
  的单位（见 §3），其余是解码能力增强。
- 上游仍未覆盖的 V8 风险：椭圆 `ratio=0`、type-11 曲线切线、复合元素的其它嵌套形态；
  level 的 freeze/lock 仍不解码。0.5.4 已处理隐藏元素（`0x0080`）、`ff fe` + UTF-16LE 名称
  与超声明计数的复合链（新增上游 `dgn.complex_count` warning）。这些不是本仓库能修复的；
  遇到时走 `cadkit-dgn` 的 warning / 本适配器的显式拒绝，绝不静默改成“成功”。

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
| level | `Layer` | 只写名称与可见性；0.5.4 起上游可从 240 字节 level-entry 解出颜色索引（写 `Layer.color`）与 raw weight/style index（留在 `dgn.level_weight_raw`/`dgn.level_style_raw`），但**仍不解码 freeze/lock**，因此适配器**仍不写** `LayerAttributes`（默认行会伪装成真实数据），记 `import.dgn.level_symbology_undecoded` |
| linetype | `LineType` | `DGN_SOLID`/`DGN_DOT_DASH` 等的 pattern；**0.5.4 起 cadkit 的 `pattern` 是 drawing units**（内置装饰线型 1..=7 由纸面 mm 经 master unit 换算），适配器原样写入、**不再二次除 UOR**；文件无 master-unit 尺寸的内置纸面线型退 continuous 并记 `import.dgn.linetype_pattern_unscaled` |
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
  `import.dgn.text_encoding` 记录；0.5.4 另识别 `ff fe` BOM + UTF-16LE 的 linkage 名称
  （level/model/cell/tag set/单位 label），V8 无 stored-length 的文本由 range 推导，2D tag
  旋转改按 angle 读（兼容识别旧 quaternion）。`ImportRequest` 没有 codepage 通道，未声明
  GBK/Big5 的 V7 可能乱码，这是显式边界。
- **解压预算**：`ImportLimits.max_decompressed_bytes = None` 时保留 cadkit 自己的默认（4 GiB）；
  `Some(n)` 逐字透传。`max_file_bytes` 在 dispatcher 的 `Ole` 分支调用 `sniff` **之前**检查，
  避免对超大 CFB 先走目录解析。
- **多模型**：只导入 `Document::model_space()`；其它模型跳过并记诊断 + Partial。仓库没有
  多模型/图纸切换，把 sheet 模型叠进模型空间会是假内容。
- **隐藏元素**：`Entity.visible=false` 写入 `builder.set_hidden_entities`，不删除；0.5.4 起
  V8 property bit `0x0080` 由上游解释为不可见（此前上游总报 visible）。level 的
  display/freeze/lock 标志（V7/V8）上游尚未解码，因此所有 level 都按可见导入，并记
  `import.dgn.level_symbology_undecoded` + **每个成功导入都是 Partial**（与 RVT/SKP 的 preview
  诚实通道同型）。
- **标签/Item Types/XAttributes（ADR-0042 D9，2026-11-21）**：`cadkit` 已把 DGN tag / Item Type
  解码进 `Entity.attributes`（每项有 `tag`、已解码类型的 `value`、所属 `set`）。适配器把它们写成该
  元素的 BIM 块（`cad_db::BimElement`，随元素分片保存）：
  - 同一 `set` 合成一个 `BimPropertySet`，`set` 缺失时用固定名 `DGN_TAGS`；属性名是 tag 名，
    值按解码类型映射 `BimValue::{Text,Integer,Real,Bool,Null}`（浮点保留数值，**不**格式化成字符串）。
  - **只有** tag 名（ASCII 大小写不敏感）是 `Category`/`IfcType`/`IFCEntity`、且值是单个 token
    （非空、无空白、≤128 字节）时才写 `bim.category`；否则 `category=None` +
    `import.dgn.bim.no_category`。**level 名不是楼层、cell 名不是类别、几何形状不推断类别**，
    没有启发式路径（`evidence` 恒为 `declared`）。
  - `bim.source` 的 `id`/`native_id` 复用实体行既有的 `source_handle` 大写十六进制拼写；
    `document_id` 为 `None`（DGN 无文档 GUID，checksum 在 `tables/sources.json`）。
  - 预算用 `cad_db::BIM_*`：单元素 ≤64 PSet、每 PSet ≤256 属性、单串 ≤4 KiB。超限**丢弃该部分**
    并记 `import.dgn.bim.budget_exceeded`，不写不完整块。
  - `import.dgn.tags_not_stored` **保留**，但只用于：读者无法命名的 tag（缺 tag-set 定义时的
    `tag_<n>` 回退）、无 `BimValue` 形态的值（binary/list、非有限浮点），以及没有数据库行的元素
    （跳过类型、未知元素、被扁平化的 group 自身）。
  - 整体 BIM 解释恒记一条 `import.dgn.bim.unverified`：值来自文件作者，但**没有真实 V8i/CONNECT
    样本**，语义仍 **NOT VERIFIED**。XAttributes 仍不建模。
- **颜色**：DGN palette 已由 cadkit 解析成 sRGB；>255 的扩展色上游仍回退 `ByLayer`（0.5.4
  起在 `Document::warnings` 里按文档聚合 `dgn.color_extended_unresolved`），本适配器无法区分、
  也不转发该上游 warning，按文档边界声明，不声称 true color 完整。
- **复合链/形**：0.5.4 在 header 声明计数多于实际成员时提前关闭父链（上游 issue #27），避免
  后续图元被误吃进父链；上游每文档给 `dgn.complex_count` warning。适配器仍把成员扁平化为
  独立实体（`import.dgn.group_flattened`），不保留 group 编辑语义。两个上游 warning 都不是
  本适配器的 `import.dgn.*` 稳定码，也不进入 `ImportReport`（适配器当前不转发
  `Document::warnings`）。

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

BIM 语义（ADR-0042 D9）：`import.dgn.bim.tags_stored_as_properties`、
`import.dgn.bim.category_from_tag`、`import.dgn.bim.no_category`、
`import.dgn.bim.unverified`、`import.dgn.bim.budget_exceeded`。

诊断消息是英文内部文本；用户可见的状态/标签仍走 i18n 目录。**每个成功导入恒为 `Partial`**
（level 显示标志未解码是结构性损失），reason 聚合到 ≤5 条稳定原因；**没有几何或只有未知元素时
是 `Unsupported`，不是空成功**。上游 `cadkit` 0.5.4 的 `Document::warnings`（如
`dgn.complex_count`、`dgn.color_extended_unresolved`）不是本适配器的稳定码，也不进入
`ImportReport`。

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
- 31 条 crate 契约测试（`cargo test -p cad-import-dgn`）覆盖：V7 的 text/circle/polyline/line
  与单位；V8 的 metres、3 个 level、shared cell 定义/Insert 位置与 10000 缩放、group 扁平化、
  2 个 Unknown 类型、tags、恒 Partial；纸面线型换算为 0.0045 m（0.5.2 时代夹具值 0.00045 m
  是把纸面毫米当 UOR 再除 UOR 因子的错误结果），无 master-unit 尺寸的纸面线型退 continuous
  并记 `import.dgn.linetype_pattern_unscaled`；layer 不写伪造属性；
  **每个导入实体经 `apply_drawing_changes` 写回成功**；非 DGN/超文件/超实体/取消拒绝；
  未知-only、无几何、fit-point spline、缺块 Insert、循环/超深 shared cell 引用（不发出递归
  `Insert`）、栅格 per-pixel u/v 与未知像素尺寸、ratio=0 椭圆、zero-sweep 弧、完整椭圆；
  **BIM（ADR-0042 D9，合成 `cadkit_core::Document`）**：tag→PSet 分组与类型映射（浮点不被字符串化）、
  显式 `Category`/`IfcType`/`IFCEntity`（大小写混写）设类别、无类别 tag 报 `no_category`、
  level/cell 名与散文值永不成为类别、预算超限整块丢弃并报 `budget_exceeded`（不写不完整块）、
  无 tag 元素没有 `bim` 块、读者 `tag_<n>` 回退名不成为属性名、以及夹具里唯一的 tag 仍只报
  `tags_not_stored`。**不证明** V8i/CONNECT/ORD、真实 MicroStation 文件、
  level 显示标志、references、真实 Item Types、尺寸/多线/网格/实体、栅格像素或字体度量。
- **2026-11-21 依赖升级**（`cadkit` 0.5.2 → 0.5.4）：只重跑了上述 crate 契约测试（23 passed）、
  `cargo check -p cad-import-dgn --all-targets --locked`、该 crate 的
  `clippy --all-targets -- -D warnings` 与 `cargo fmt --all -- --check`；没有用真实 DGN 样本
  复跑，wasm32 编译与全量门禁 **NOT RUN**（详见 `docs/validation-deps-2026-11-21.md`）。
- **真实 GPU 视觉运行（2026-11-14）**：Linux GNOME/Mutter Wayland 会话内启动嵌套 Weston 14，
  `target/debug/yacr --open fixtures/dgn/{smalltest,test_dgnv8}.dgn` 以
  `GDK_BACKEND=wayland` + `WGPU_BACKEND=vulkan` + NVIDIA ICD + `DRI_PRIME=1` 运行；probe 报告
  `backend=vulkan adapter=Quadro P620 device_type=discrete_gpu`，`bridge wayland`（native-below）
  且 `frames` 持续增长。V7/V8 截图、probe 与命令见 `docs/evidence/dgn-linux-gpu/`。这是**视觉/present 证据**，
  不是 golden/像素/性能或 Bentley 兼容性验收；debug 构建 + 嵌套合成器是两条边界。该轮运行在
  `cadkit-dgn` 0.5.2 上，**0.5.4 未重跑真实样本或视觉证据**。
- 真实文件只通过 `YACR_TEST_DGN=<绝对路径>` 本地验证；未设置时的跳过 **不是已运行证据**。
- `cargo check -p app-web --target wasm32-unknown-unknown` 只证明可编译；没有浏览器/像素验收。
  该条是历史结论，本轮（0.5.4）wasm32 编译 **NOT RUN**。
  Windows/macOS/Android 的 DGN 运行证据一律 **NOT VERIFIED**。
