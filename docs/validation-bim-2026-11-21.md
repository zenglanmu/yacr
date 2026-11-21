# BIM 语义 Phase 1 验证记录（2026-11-21）

范围：ADR-0042 **Phase 1** 的实际落地——`.ymodel` 格式与存储（`FORMAT_MINOR` **5 → 6**）、IFC 语义导入
（天花板 `Partial`）、属性面板固定行，以及库层/宿主端到端契约。环境：Linux x86_64，Rust 1.99.1
（`rust-toolchain.toml`），全部命令 `--locked`。

**这不是 BIM 能力验收**：IFC 语义映射只有**合成夹具**的契约证据，没有真实 IFC 文件、没有
Revit/Bentley/IfcOpenShell 等 vendor 交叉验证；RVT 的 BIM 语义、DGN 的 tags/Item Types 落库、
IFC4 导出回写都**未实现**（Phase 2/3）。本轮也不涉及 GPU、像素、真机或性能结论。

**记录方式**：下表的退出码与计数来自**编排者的合并复跑**与各实现 lane 的单 crate 报告；本文档
lane 只做整理，**没有重跑任何 cargo / 前端命令**（构建门禁由编排者进程持有）。因此计数是实现 lane
的观测值，不是文档 lane 的复现。

## 1. 改动范围（实现位置）

| 位置 | 作用 |
|---|---|
| `crates/cad-domain`（ymodel 版本/descriptor） | `FORMAT_MINOR` 升到 6，注释写明 6 = BIM 元素块 + 定义表 |
| `crates/cad-db/src/bim.rs` | `BimElement`/`BimSource`/`BimValue`/PSet/定义类型；预算；`set_entity_bim`/`set_bim_definitions` |
| `crates/cad-db/src/snapshot/elements/bim.rs` | 元素行 `bim` 块编解码（`None` 时**省略整键**） |
| `crates/cad-db/src/snapshot/bim.rs` | `tables/bim-definitions.json` 编解码 |
| `crates/cad-db/src/drawing/bim.rs` | 只读访问（`bim_element`/`bim_definitions`/`bim_key`） |
| `crates/cad-store-zip/src/minor.rs` | 写入 minor 三档（6 / 5 / 4）与漂移测试 |
| `crates/cad-import-ifc/src/semantics.rs` + `semantics/property.rs` | 第二遍语义扫描（几何路径不变） |
| `crates/cad-app/src/selection_bim.rs` | 属性面板 BIM 行（8 个固定静态键，`PropertyGroup::Bim`） |
| `crates/cad-ui-core/i18n/{zh-CN,en}.json`、`apps/desktop-ui/src/properties/property-labels.ts` | 1 条分组 + 8 条行 + 2 条 evidence 值，双目录同键集 |
| `crates/cad-host/src/tests/ymodel_bim.rs` | 导入 → 保存 → 重开 的宿主级端到端 |

无新增 `[patch.crates-io]` 键；`vendor/rvt` 补丁不变。

## 2. 格式变更（minor 6）

### 2.1 结构

- 元素行新增**可选** `bim` 块。无 BIM 时**整个键省略**（不写 `"bim": null`）——旧构建的元素解码是
  严格的，多一个 `null` 字段即 `invalid_document`。
- `tables/bim-definitions.json` **仅在非空时**写；否则不产生该条目，文档不因「格式支持 BIM」而升 minor。
- `bim` 块字段：`category` / `name` / `source{connector,document_id,id,native_id}` /
  `level` / `type` / `material` / `psets[{name, properties[{name, value}]}]` / `evidence`；
  `value` 只允许 `text | real{ifc_type, unit} | integer | bool | null`。
- 定义行 id 是**可读复合字符串** `level:<connector>:<doc>:<key>` / `type:…` / `material:…`，
  **不是** `PersistentId`：不进 `IdKind`、不占高水位；`IdKind` 11/12/13 **未启用**。
- `category` 不建表，是元素/类型上的开放 token（IFC 实体名规范拼写）。

### 2.2 minor 三档与兼容矩阵

写入 minor 按**文档内容**取（`crates/cad-store-zip/src/minor.rs`）：有 BIM → **6**；
只有资源条目（或开了 `embed_resources`）→ **5**；否则 → **4**。顶层值必须等于 `FORMAT_MINOR` 的
漂移测试固定这条规则。

| 归档 | 写入 minor | 旧构建（≤5） | 新构建 |
|---|---|---|---|
| 无 BIM、无资源 | 4 | 打开 | 打开，`bim = None` |
| 只有资源、无 BIM | 5 | 拒绝 `ymodel.unsupported_minor`（既有行为） | 打开 |
| 有 BIM | 6 | 拒绝 `ymodel.unsupported_minor` | 打开 |
| minor 5 但被塞了 `bim`（非合规） | 保存时升 **6** | — | 接受、保留，不丢数据 |

- 旧读新：minor > 6 按 `ymodel.unsupported_minor` 拒绝，不静默丢 `bim`。
- 新读旧：元素无 `bim` 键 = `None`；无定义表 = 空定义；正常打开。

### 2.3 预算与稳定标记

预算在**写读两侧都强制**（`cad-db` 与归档解码共用同一套限制）：定义 **100 000** 行、定义表编码
**32 MiB**、单元素 **64** 个 PSet、每 PSet **256** 个属性、字符串 **4 KiB**。

超限是**显式拒绝**，不是截断、不是丢弃超出部分。稳定前缀：
`ymodel.bim.invalid`、`ymodel.bim.budget_exceeded`、`ymodel.bim.dangling_reference`。
（ADR-0042 D3 草案写的是「丢弃超出部分并报 `*.bim.budget_exceeded`」；实际落地选择了显式拒绝，
本记录以代码为准。）

## 3. IFC 导入（状态 `Partial`）

### 3.1 读什么、落到哪里

| IFC 源 | 落到 | 备注 |
|---|---|---|
| `IfcRoot.GlobalId` / `Name` | `bim.source.id` / `bim.name` | 缺则 `import.ifc.bim.source_id_missing` |
| `IfcElement.Tag` | `bim.source.native_id` | spatial 元素的 slot 7 是 `LongName`，**不读** |
| `IfcProject.GlobalId` | `bim.source.document_id` | 文档级标识 |
| `IfcBuildingStorey` | `level` 定义 + `bim.level` | elevation 按项目长度单位换算到米；单位不可解析 → `None` + `import.ifc.bim.elevation_unresolved` |
| `IfcRelAggregates` / `IfcRelContainedInSpatialStructure` | 构件→楼层的归属 | 向上最多 **16** 条边、防环；无法解析 → `import.ifc.bim.storey_unresolved` |
| `IfcTypeObject` 家族 + `IfcRelDefinesByType` | `type` 定义 + `bim.type` | 含类型级 PSet；未解析 → `import.ifc.bim.type_unresolved` |
| `IfcPropertySet` + `IfcRelDefinesByProperties` + `IfcPropertySingleValue` | `bim.psets` | 见 3.2 |
| `IfcMaterial` + `IfcRelAssociatesMaterial` | `material` 定义 + `bim.material` | 未解析 → `import.ifc.bim.material_unresolved` |
| `IfcSpace` | 普通元素，`category = "IfcSpace"` | 不建独立类别表 |

### 3.2 明确不存

- `IfcComplexProperty` / 枚举 / 有界 / Table：**只计数，不压成字符串**
  （`import.ifc.bim.property_value_unsupported`）。
- `IfcElementQuantity`：只计数（`import.ifc.bim.quantity_set_not_stored`）。
- `IfcStyledItem` 颜色：沿用既有 `import.ifc.style_not_imported`，不因 BIM 通道而重开。
- 无法承载的 PSet 形状：`import.ifc.bim.property_set_unsupported`。
- 预算超限：`import.ifc.bim.budget_exceeded`；单个实体被拒：`import.bim.entity_refused`。

### 3.3 完整性

- **只有几何、没有关系级 IFC** 的文档仍 `Complete`——只是多了 `source`/`category`/`name`。
- 一旦出现关系级 BIM 数据（楼层/类型/PSet/材质）即 `Partial`，理由写明映射只是**代码契约证据**，
  没有 vendor 工具交叉验证（源码注释：`contract evidence; no vendor tool cross-validation exists`）。
- 一个 IFC product 只算**一个**构件；被拆成多段几何时每行保留自己的 `EntityId`，靠 `bim.source`
  等价类归组。

## 4. 属性面板

- `crates/cad-app/src/selection_bim.rs` 只发 **8 个固定静态键**：`bim_category`、`bim_name`、
  `bim_level`、`bim_type`、`bim_material`、`bim_source`、`bim_properties`、`bim_evidence`；
  分组 `PropertyGroup::Bim`。
- 无 BIM 的元素**零行**（不写空占位）。
- `bim_properties` 是**单行摘要**，例如 `Pset_WallCommon (2)`；逐属性行与动态属性名属 Phase 3。
- 引用解析失败时显示**原始 id**（不隐藏、不猜名字）。
- `editable` 恒为 `false`：属性编辑是 Phase 3。
- i18n：`properties.group.bim` + 8 条 `properties.row.bim_*` + 2 条 `properties.value.bim_evidence.*`，
  两个目录键集一致；`property-labels.ts` 同步；`apps/desktop-ui/test/properties.test.ts` 的键扫描
  已扩到 `selection_bim.rs`。
- **已知小缺口**：`PropertiesPalette.tsx` 尚未对 `bim_evidence` 的**值**走 `t()`，当前显示
  `declared` / `heuristic` 机器 token；行的 label 已本地化。

## 5. 端到端契约（`crates/cad-host/src/tests/ymodel_bim.rs`，5 条）

| 测试 | 断言 |
|---|---|
| `ifc_bim_survives_a_ymodel_save_and_reopen` | 夹具 IFC 导入 → 原生保存 → 重开，BIM 全等 |
| `a_bim_document_is_written_at_minor_six` | 两次保存 minor 恒 6 |
| `a_bim_free_document_stays_at_minor_four` | 无 BIM 文档 minor 4 |
| `reopening_a_bim_document_exposes_bim_property_rows` | 重开后属性行存在 |
| `a_plain_mesh_element_has_no_bim_rows` | 无 BIM 实体不出 BIM 行 |

观察到的数据（合成夹具）：`elements=2, levels=2, types=1, materials=1`，minor **6**；
面板值 `IfcWall` / `Level 0` / `Basic Wall 200mm` / `Concrete` / `Pset_WallCommon (2)`。

**minor 证据分工**：宿主侧 minor 是「把快照交给载体、由载体选 6/4」的投影（`cad-store-zip` 不允许被
其它 `cad-*` 依赖，宿主测试用 fake `DocumentArchive`）；**真实落盘数字**由
`crates/cad-store-zip/tests/bim_minor.rs` 的 5 条测试钉住（4 / 5 / 6、minor-5 无 BIM 可开、
minor-5 带 BIM 被接受并在保存时升 6）。

## 6. 已运行

| 命令 | 结果 |
|---|---|
| `cargo test -p cad-domain -p cad-db -p cad-store-zip -p cad-import-core -p cad-import-ifc -p cad-import-dgn -p cad-export-ifc -p cad-app -p cad-host --locked` | **EXIT 0**（编排者合并复跑） |
| `cargo clippy`（同批 + `cad-ui-core`，`--all-targets -- -D warnings`） | **EXIT 0** |
| `cargo fmt --all -- --check` | **EXIT 0** |
| `cargo check --target wasm32-unknown-unknown`：`cad-db` / `cad-import-ifc` / `cad-store-zip` | **EXIT 0**（仅编译，未运行 wasm） |
| `python3 scripts/check-file-size.py` | **EXIT 0**（2497 文件） |
| `python3 scripts/check-architecture.py` | **EXIT 0**（44 包） |
| `python3 scripts/check-i18n.py` | **EXIT 0**（3035 键 / 2 目录） |
| `python3 scripts/check-fixture-manifest.py` | **EXIT 0**（48） |
| `python3 scripts/check-tauri-capabilities.py` | **EXIT 0** |
| `python3 scripts/check-command-docs.py` + `scripts/test-check-command-docs.py` | **EXIT 0** |
| `(cd apps/desktop-ui && npm test)` | **1860 passed** |

各 lane 的单 crate 计数（报告值，非文档 lane 复现）：

| crate | 计数 |
|---|---|
| `cad-domain` | 219 lib |
| `cad-db` | 370 lib（含 8 条 BIM 测试） |
| `cad-store-zip` | `archive` 18 / `document_fixed_point` 8 / `bim_minor` 5 / `resources` 18 |
| `cad-import-core` | 41 |
| `cad-import-ifc` | 27（含 7 条新语义测试） |
| `cad-import-dgn` | 23 |
| `cad-export-ifc` | 13 |
| `cad-app` | 3073 passed / 3 ignored |
| `cad-host` | 429 passed / 1 ignored（含 5 条新端到端） |

新增契约测试（实际名称）：

- 存储（`cad-db`）：`element_bim_round_trips_and_is_omitted_when_absent`、
  `absent_bim_on_a_minor_five_row_decodes_as_none`、`a_row_with_bim_null_is_rejected`、
  `a_unknown_bim_field_is_rejected`、`bim_definitions_table_decodes_strictly_and_rejects_dangling_refs`、
  `a_definition_row_must_carry_its_kind_prefix`、`bim_budget_exceeded_is_an_explicit_error`、
  `an_oversized_bim_string_in_an_archive_is_refused`。
- 归档（`cad-store-zip/tests/bim_minor.rs`）：`a_bim_free_document_stays_minor_four`、
  `a_resources_only_document_stays_minor_five`、`a_bim_document_writes_minor_six_and_reopens`、
  `a_minor_five_archive_without_bim_opens`、
  `a_minor_five_archive_that_holds_bim_is_accepted_and_resaved_at_six`。
- IFC 语义（`cad-import-ifc/src/tests_semantics.rs`）：
  `ifc_wall_globalid_storey_type_material_pset_are_stored`、`one_ifc_product_stays_one_bim_element`、
  `complex_property_is_reported_not_fabricated`、
  `a_product_without_relations_has_source_and_category_only`、
  `bim_budget_exceeded_keeps_no_dangling_reference`、
  `an_ifc_space_is_an_ordinary_element_with_its_category`、
  `import_without_bim_relations_stays_unchanged`。
- 属性面板（`cad-app/src/tests/selection_bim_tests.rs`）：
  `bim_panel_rows_use_static_keys_and_a_pset_summary`、`a_non_bim_element_has_no_bim_rows`、
  `bim_rows_do_not_duplicate_keys_under_multi_select`、`bim_evidence_is_localized_via_catalog`。

## 7. NOT RUN / NOT VERIFIED

- **真实 IFC 文件**：`YACR_TEST_IFC` 未设置，真实 IFC 的语义读取、楼层/类型/材质/PSet 映射与几何
  一致性 **NOT RUN / NOT VERIFIED**。
- **vendor 交叉验证**：Revit / Bentley / IfcOpenShell / buildingSMART 校验一律**未运行**；
  IFC 语义导入只有合成夹具契约证据。
- **RVT**：真实 RVT 文件的 BIM 语义 **NOT VERIFIED**（且本轮根本没接线，见 §8）。
- **DGN**：真实 V8i/Connect 样本的 tags/Item Types **NOT VERIFIED**（且本轮没有落库目标）。
- **IFC 导出回写**：**未实现**，因此没有「导出→再导入 BIM 全等」的证据。
- **workspace 全量**：`cargo test --workspace`、全量 clippy、`apps/app-*` 编译 **NOT RUN**。
- **wasm / Android 运行时**：只对三个 crate 做了 `wasm32-unknown-unknown` 编译检查，**未运行**；
  Android/设备/GPU/像素 **NOT VERIFIED**。
- **跨机自包含**：与 BIM 一并保存的资源打包没有跨机验证。

## 8. 仍不存在的（Phase 2/3）

- **IFC4 导出回写**：`cad-export-ifc` 仍只写 `IfcBuildingElementProxy`，**不写** storey/type/pset/
  material，GlobalId 也不从 `bim.source` 复用。Phase 2。
- **RVT 的 BIM 语义**：`rvt` 的 `IfcModel` 仍被 `decode_to_glb` 丢弃，GLB extras 的
  `entityIndex`/`ifcType` 也未使用。Phase 2。
- **DGN 的 tags/Item Types 落库**：仍是 `import.dgn.tags_not_stored`，没有 BIM 目标。Phase 3。
- **查询/过滤/明细表**：`cad-query` 的 BIM 索引与 `cad_bim_query`/`cad_bim_summary` 未实现。Phase 3。
- **关系与 Aspect 落盘**：`IdKind` 12/13、关系元素、minor 7 未启动。Phase 3。
- **属性编辑与逐属性行**：面板 `editable` 恒 `false`，`PropertyRow.key` 仍是 `&'static str`。Phase 3。
- **`bim_evidence` 值本地化**：`PropertiesPalette.tsx` 仍显示机器 token（见 §4）。

## 9. 依赖

`ifc-lite-*` **28.3.0**、`cadkit-dgn` / `cadkit-core` **0.5.4**、`rvt` **0.25.0**（`vendor/rvt`
受控补丁不变，只改 `src/ifc/gltf.rs` 的透明度选择）。没有新增 `[patch.crates-io]` 键。

## 10. 边界结论

- 本轮证明的是：**格式能写能读、IFC 合成夹具的 BIM 数据能落库并在保存/重开后全等、面板能显示**。
- 不证明：任何真实 IFC 模型被正确映射、任何 vendor 工具接受这些数据、IFC 导出能回写、RVT/DGN
  的 BIM 语义可用、任何平台/GPU/像素/性能结论。
- 证据边界同时写进 `docs/compatibility.md` 与 ADR-0042 §4/§6；`.ymodel` 仍是唯一可直接再打开的格式。
