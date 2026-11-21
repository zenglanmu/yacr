# BIM 语义 Phase 1 验证记录（2026-11-21）

范围：ADR-0042 **Phase 1** 的实际落地——`.ymodel` 格式与存储（`FORMAT_MINOR` **5 → 6**）、IFC 语义导入
（天花板 `Partial`）、属性面板固定行，以及库层/宿主端到端契约。环境：Linux x86_64，Rust 1.99.1
（`rust-toolchain.toml`），全部命令 `--locked`。

**这不是 BIM 能力验收**：IFC 语义映射只有**合成夹具**的契约证据，没有真实 IFC 文件、没有
Revit/Bentley/IfcOpenShell 等 vendor 交叉验证；RVT 的 BIM 语义、DGN 的 tags/Item Types 落库、
IFC4 导出回写都**未实现**（Phase 2/3）。本轮也不涉及 GPU、像素、真机或性能结论。

> **注（2026-11-21 追加）**：上面这句"未实现"是 Phase 1 当时的快照，保留原文不改写；Phase 2/3
> 已于同日落地，范围与证据见 §12–§14。

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

## 11. Phase 3 关系落盘（minor 6 → 7，2026-11-21）

本节只追加 Phase 3 的**关系落盘**，不改写 §1–§10 的 Phase 1 结论。

### 11.1 格式

- 新表 `tables/bim-relations.json`，**只在非空时写**（空表不产生归档条目、不升 minor）。
- 行形状（端点复用元素 `bim.source` 的键拼写）：

  ```json
  {"kind": "aggregates",
   "parent": {"kind": "source", "connector": "ifc", "document_id": "…", "id": "…"},
   "child":  {"kind": "entity", "entityId": 7},
   "source": {"kind": "source", "connector": "ifc", "document_id": "…", "id": "…"},
   "label": null}
  ```

- `kind` ∈ `aggregates | nests | contains | group | connects | other:<token>`；未知 token 是
  `ymodel.invalid_document`（detail 带 `ymodel.bim.invalid`）。
- `source`/`label` 是**必写键**，无值为 `null`；省略键、额外键（含 `native_id`）、错误类型都拒绝。
- `entityId` 必须是 entity 家族的持久化 id；裸序列或别的家族拒绝。
- `parent`/`child`/`source` **不要求**解析到现存元素（IFC 的一个 product 可能没有几何行）。

### 11.2 minor 与兼容

- `FORMAT_MINOR` **6 → 7**；写入按文档取 **7 / 5 / 4**：
  - 有 BIM（元素块、定义表、关系表任一）→ **7**（关系与元素/定义同属一次发布，
    "有定义无关系"的文档也写 7，理由见 ADR-0042 D6）；
  - 只有资源 / `embed_resources` → **5**；
  - 都没有 → **4**。
- **6 是只读旧档**：本构建能打开 minor-6 归档（无关系表），再保存写 7。
- 旧构建（≤6）读 7：`ymodel.unsupported_minor`，不静默丢关系。

### 11.3 预算与去重

- 关系预算 **100 000 行 / 32 MiB 编码体积**，写读两侧都强制；超限
  `ymodel.bim.budget_exceeded`（通过 `BimError` 的稳定 marker），**不截断**。
- 行键 `(kind, parent, child)`：完全相同的边插入第二次即拒绝（`ymodel.bim.invalid`），
  读取侧同样拒绝重复行；不静默合并。

### 11.4 改动位置

| 位置 | 作用 |
|---|---|
| `crates/cad-db/src/bim/relations.rs`（新） | `BimRelationKind`/`BimRelation`/`BimRelations`、预算、`validate_relation(s)`、`set_bim_relations` |
| `crates/cad-db/src/snapshot/relations.rs`（新） | `tables/bim-relations.json` 严格编解码 |
| `crates/cad-db/src/snapshot/{mod,build}.rs` | `table_entry`、`has_bim` 纳入关系表、非空才写、读回 hydrate |
| `crates/cad-db/src/{scope.rs,drawing/{mod,bim}.rs,builder.rs}` | `TableKind::BimRelations`、数据库侧表字段/访问器、构造器接线（`finish()` 一行改调 `validate_all`，未增加行数） |
| `crates/cad-store-zip/src/{minor.rs,read.rs}` | 写入四档（4/5/6 只读/7）与 15 条表项镜像 |
| `crates/cad-domain/src/ymodel/mod.rs` | `FORMAT_MINOR = 7` + 注释 |

未启用 `IdKind` 11/12/13；`header.rs`、`IdCounters` 未改；无新依赖、无 `[patch]`。

### 11.5 已运行（本轮 lane，逐条看退出码）

| 命令 | 结果 |
|---|---|
| `cargo test -p cad-db --locked` | **EXIT 0**（384 lib + 13 + 5） |
| `cargo test -p cad-store-zip --locked` | **EXIT 0**（lib 40；archive 18；bim_minor 8；bim_relations 1；document_fixed_point 8；raw_copy 6；resources 12；textures 6） |
| `cargo check -p cad-db --target wasm32-unknown-unknown --locked` | **EXIT 0**（仅编译） |
| `cargo clippy -p cad-db -p cad-store-zip --all-targets --locked -- -D warnings` | **EXIT 0** |
| `cargo fmt -p cad-db -p cad-store-zip -p cad-domain -- --check` | **EXIT 0** |
| `python3 scripts/check-file-size.py` | **EXIT 1**（唯一超限文件是并行 lane 的 `crates/cad-ui-core/src/session/wire.rs` 1057 行；本轮新增/修改文件均 ≤1000 行，未触碰该文件） |
| `python3 scripts/check-architecture.py` | **EXIT 0**（44 包） |

新增契约测试（实际名称）：

- `cad-db`（`bim::relations::tests`）：`duplicate_relation_is_rejected`、
  `relations_budget_excess_is_an_explicit_error`、`unknown_relation_kind_token_is_not_parsed`、
  `relation_shape_rejects_empty_endpoint_strings`、`an_empty_relations_table_is_not_written`、
  `relation_rows_round_trip_through_the_codec`。
- `cad-db`（`snapshot::relations::tests`）：`relation_row_round_trips_with_entity_endpoints_and_nulls`、
  `relation_row_with_unknown_kind_token_is_rejected`、`relation_row_with_unknown_endpoint_kind_is_rejected`、
  `a_relation_row_must_carry_every_key`、`a_relation_row_refuses_an_extra_key`、
  `a_source_endpoint_refuses_element_provenance_keys`、
  `an_entity_endpoint_must_carry_an_entity_persistent_id`、`duplicate_relation_rows_are_rejected_on_read`。
- `cad-store-zip`：`relation_rows_round_trip_through_an_archive`；`minor` 档位
  `a_document_with_relations_writes_minor_seven`、`a_bim_document_without_relations_still_writes_seven`、
  `a_plain_document_stays_minor_four`/`a_bim_free_document_stays_minor_four`、
  `a_resources_only_document_stays_minor_five`、`a_minor_six_archive_without_relations_opens`。

### 11.6 NOT RUN / NOT VERIFIED

- **关系导入未接线**：`cad-import-ifc`/`cad-import-rvt`/`cad-import-dgn` 本轮未改动，没有把源关系
  写进 `BimRelations` 的路径；本节的证据只有**格式/存储/编解码契约**与合成行。
- **真实 IFC/RVT/DGN 关系**、vendor 交叉验证、UI 明细表/查询：**NOT RUN / NOT VERIFIED**。
- **workspace 全量** `cargo test --workspace`、全量 clippy、`apps/**` 编译 **NOT RUN**。
- 宿主 `crates/cad-host/src/tests/ymodel_bim.rs` 仍断言 `FORMAT_MINOR == 6`（不在本轮 lane 范围），
  它需要由对应 lane 改为 7 才能在全量测试中通过；本轮未修改、未运行该 crate。
- wasm 仅编译检查，未运行；Android/GPU/像素/真机 **NOT VERIFIED**。

## 12. Phase 2/3 落地范围（2026-11-21）

本节只追加 Phase 2 与 Phase 3 的**落地范围**，不改写 §1–§11 的既有结论，也不把任何合成/单样本
观察升级为验收。

### 12.1 Phase 2 — IFC 导出回写（`crates/cad-export-ifc`）

| 位置 | 作用 |
|---|---|
| `src/bim.rs`（765 行） | BIM → IFC4 写出：`IfcBuildingStorey`、`IfcClass::Named(category)`、`<Category>Type`、`IfcMaterial`、PSet、源 `GlobalId` 复用 |
| `src/bim_value.rs`（286 行） | `BimValue` → IFC 属性值映射与「不可表示」计数 |
| `src/tests_bim.rs`（763 行，**14 条**） | 导出契约（含 GlobalId 向量与类别白名单） |

- 入口 `IfcExporter::with_bim_policy(BimExportPolicy{include_unverified:false})`；**触发条件**是
  `database.bim_elements()` 非空，否则走旧路径且**逐字节不变**：钉住值非空 **2196 字节** / FNV
  `ec72ad…`、空 **1374 字节** / `c56f8c…`，输出只出现 `IFCBUILDINGELEMENTPROXY`。
- storey 按 `(elevation,name,id)` 排序、单位米；类别走显式白名单并逐项对照内置 IFC4 registry，
  未知类别回退 `IfcBuildingElementProxy`；`TypeLink` 首现创建、后续引用，与源 class 不一致则跳过不
  替换；material 按名去重。
- GlobalId：自带 22 字符 IFC base64 → `u128` 解码（首字符 2 bit），向量含 `u128::MAX` 与夹具
  `2O2Fr$t4X7Zf8NOew3FLSJ` 自往返；非法/重复回退 `stable_global_id` 并报告。
- **已知限制**：`cad_db::BimValue::Text` 无 `ifc_type`，因此一律写 `Label`（不按 token 映射）；未知
  单位（`kN`/`psi` 等）不写出并计数；`Null` 跳过；**关系不导出**。
- 诊断前缀 `export.ifc.bim.*`（`storey_not_written`/`category_not_written`/`type_not_written`/
  `property_not_written`/`unverified_skipped`/`global_id_invalid`/`global_id_collision`）。
- 证据：自洽往返（导出 → `cad-import-ifc` 读回 GlobalId 一致）；**无 vendor checker**。

### 12.2 Phase 2 — RVT 语义导入（`crates/cad-import-rvt`）

| 位置 | 作用 |
|---|---|
| `src/semantics.rs`（820 行） | `IfcModel` → `RvtSemantics`（定义表 + 逐元素 `Option<BimElement>`） |
| `src/tests_semantics.rs`（407 行，**7 条**） | 手工 `IfcModel` 的映射契约（含材料路径） |
| `src/decode.rs` / `src/glb.rs` / `src/lib.rs` | `RvtDecode` 增 `semantics`；`model_to_glb` 前 `map_model`；`PlacedMesh.entity_index` 读 `node.extras().entityIndex`；8 个 `import.rvt.bim.*` 诊断 |

- 映射：`BuildingElement{ifc_type,name,type_guid,storey_index,material_index,property_set}` →
  category/name/source/level/type/material/psets；`global_ids.elements[i]` → `bim.source.id`；
  `type_guid`（Revit ElementId）→ `native_id`；文档级 `global_ids.document` **不用**（上游说每次保存
  都变）；楼层 `elevation_feet×0.3048`；`element_type_ids` → `type:<ElementId>`，名字取
  `Family:Type` 的 Type 段；`type_property_sets` → 类型 PSet；`IfcEntity::ElementPropertySet` 追加到
  同一元素的 psets；`Aggregate` 不归组并计数。
- `type.class = None`：`IfcModel` 不暴露类型 class，派生就是猜。

### 12.3 Phase 3 — DGN 标签、IFC 数量集、关系、查询/wire/UI/选择

| 位置 | 作用 |
|---|---|
| `crates/cad-import-dgn/src/bim.rs`（339 行）+ `bim/tests.rs`（425 行，**8 条**） | tags/Item Types → PSet；显式 Category 标签；`import.dgn.bim.*` |
| `crates/cad-import-ifc/src/semantics/quantity.rs`（194 行）+ `tests_semantics_quantity.rs`（282 行，**6 条**） | `IfcElementQuantity` → PSet（保留 `Qto_*` 源名） |
| `crates/cad-import-ifc`（关系导入）+ `cad-import-core::MeshImport`（17 行转发） | 五类 `IfcRel*` → `BimRelations` |
| `crates/cad-query/src/bim.rs`（865 行）+ `bim/tests.rs`（776 行，**10 条**） | 按 `(DatabaseId, Revision)` 懒建索引、过滤、汇总 |
| `crates/cad-ui-core/src/bim.rs`（709 行）+ `bim/tests.rs`（**18 条**） | 冻结 camelCase wire 契约 |
| `crates/cad-host/src/bim.rs` + `tests/bim_query.rs`（**9 条**） | 宿主查询 seam |
| `apps/desktop-ui/src/bim/` + `components/BimPanel.tsx` | BIM 面板（浮窗/抽屉）与选择联动 |
| `apps/app-web`（`DirectReads`，**4 条**测试） | wasm 侧 BIM 查询接线 |

- DGN：tags/Item Types 按 `set` 合成 PSet（无 set → `DGN_TAGS`）；浮点保持 `f64` 不格式化；仅当
  标签名为 `Category`/`IfcType`/`IFCEntity`（大小写不敏感）且值是单个 token 时写 `bim.category`；
  level/cell/几何**不推断**；`evidence=Declared`、整体 `Unverified`；诊断
  `import.dgn.bim.{category_from_tag,no_category,unverified,budget_exceeded}`，
  `import.dgn.tags_not_stored` 收窄为「确实无法解码/无行」。已提交夹具 `test_dgnv8.dgn` 只有一个只能
  叫 `tag_0` 的标签 → 仍按 `tags_not_stored` 报告；端到端走合成的 `cadkit_core::Document`。
- 数量：Length/Area/Volume/Weight/Time → `Real{value, ifc_type: 源实体名, unit}`，Count → `Integer`；
  **不换算**（3000 mm 存 3000 + `unit: "mm"`）；单位来自条目 `Unit` 或项目单位，解析不到
  `unit: None`（不猜米）；`IfcPhysicalComplexQuantity` 等不压平；新
  `import.ifc.bim.quantity_set_stored`（信息性、不降级），`quantity_set_not_stored` 收窄。
- 关系落盘：`tables/bim-relations.json`（仅非空时写）；行 `{kind, parent, child, source, label}`，
  端点复用元素 `bim.source` 键；`kind ∈ aggregates|nests|contains|group|connects|other:<token>`；
  预算 100 000 行 / 32 MiB，超限显式拒绝；重复 `(kind,parent,child)` 写读双向拒绝；`FORMAT_MINOR`
  升到 **7**（元素 6 起、关系 7；minor 6 仍可读但再保存写 7），写入三档 4/5/7、6 为只读档。
- 查询：facet 间 AND、facet 内 OR；属性算子 `Eq/Ne/Contains`（数值 Real/Integer 互比）；term 上限
  64（超出 `bim.filter.too_many_terms`）；行排序确定（category,name,source id,最低 EntityId,BimKey）。
- UI：非模态浮窗（>1024px）/ 全宽底部抽屉（≤1024px）；facet chip 点击即筛选、150ms 防抖、
  generation 丢弃过期响应；区分「无 BIM 数据/过滤无匹配/无图纸/载体不支持/会话未就绪/stale」；
  两语言各 +61 键。
- `cad_select_entities`：payload `{entityIds:number[]}`（camelCase 已用测试钉住）；空数组 →
  `selection.empty`（不清空）；未知 id → 整单拒绝 `selection.unknown_entity: <id>`（不部分选中）；
  无文档 → `selection.no_document`；无选择缝 → `unsupported-runtime: cad_select_entities`。

## 13. 最终合并验证（编排者复跑，逐条 EXIT 0）

下表是编排者在本轮合并后的实际命令与结果；**文档 lane 没有重跑**，只整理报告值。

| 命令 | 结果 |
|---|---|
| `cargo test` 13 个 crate：`cad-domain`/`cad-db`/`cad-store-zip`/`cad-import-core`/`cad-import-ifc`/`cad-import-dgn`/`cad-import-rvt`/`cad-export-ifc`/`cad-query`/`cad-app`/`cad-host`/`cad-ui-core`/`cad-desktop-shell` + `-p app-web` | **EXIT 0** |
| `cargo clippy`（同批 13 crate + `app-web`，`--all-targets --locked -- -D warnings`） | **EXIT 0** |
| `cargo fmt --all -- --check` | **EXIT 0** |
| wasm32 `cargo check --target wasm32-unknown-unknown --locked`：`cad-db`/`cad-store-zip`/`cad-import-ifc`/`cad-import-rvt`/`cad-import-dgn`/`cad-export-ifc`/`cad-query`/`cad-ui-core`/`app-web` | **全 EXIT 0**（`cad-host` 与 `app-web` 各有 1 条既有 `dead_code` 警告，非门禁项） |
| `cargo check -p app-linux --all-targets --locked` | **EXIT 0** |
| `cargo check -p app-windows --target x86_64-pc-windows-msvc --locked` | **EXIT 0**（新增 tick 代码通过 Windows 类型检查） |
| `cargo check -p app-macos --target aarch64-apple-darwin --locked` | **NOT RUN（非代码失败）**：`cc-rs` 编 `objc2-exception-helper` 的 `try_catch.m` 失败——本机无 macOS SDK |
| `python3 scripts/check-file-size.py` | **EXIT 0**（2510 文件） |
| `python3 scripts/check-architecture.py` | **EXIT 0**（44 包） |
| `python3 scripts/check-i18n.py` | **EXIT 0**（3096 键 / 2 目录） |
| `python3 scripts/check-fixture-manifest.py` | **EXIT 0**（48） |
| `python3 scripts/check-tauri-capabilities.py` | **EXIT 0** |
| `python3 scripts/check-command-docs.py` + `python3 scripts/test-check-command-docs.py` | **EXIT 0** |
| `(cd apps/desktop-ui && npm test)` | **1917 通过 / 0 失败**（BIM 新增 57） |

各 lane 计数（报告值，非文档 lane 复现）：

| crate / 包 | 计数 |
|---|---|
| `cad-query` | 26（含 10 条 BIM） |
| `cad-import-ifc` | 41（含数量 6、关系 8） |
| `cad-import-dgn` | 31（含 8 条 BIM） |
| `cad-import-rvt` | 48（含 7 条 BIM） |
| `cad-export-ifc` | 27（含 14 条 BIM） |
| `cad-host` | 437+（含 `bim_query` 9、`select_entities` 4、`ymodel_bim` 5） |
| `cad-ui-core` | 827+（wire BIM 18、select 4） |
| `cad-desktop-shell` | 全绿 |
| `app-web` | 145 |

## 14. NOT RUN / 证据边界（Phase 2/3）

- **真实 IFC 文件**：`YACR_TEST_IFC` 未设置；导出回写只有自洽往返（导出 → `cad-import-ifc` 读回），
  **不做**「真实 IFC 往返并保持全部语义」的结论。
- **vendor 交叉验证**：Revit / Bentley / IfcOpenShell / buildingSMART 校验一律**未运行**。
- **RVT**：**单个真实样本**（`~/下载/rmeadvancedsampleproject.rvt`）的观察计数为
  `elements=6783, levels=4, types=90, materials=0, element_psets=44928, with_category=6783,
  with_level=6702, with_type=6650, with_material=0`。该样本材料表为空，`materials=0` 不等于材料
  路径被验证（材料映射只有手工 `IfcModel` 单测证据）。**这是代码契约观察，不是验收**：真实 RVT BIM
  语义 **NOT VERIFIED**，且 `rvt` 自述输出不得用于测量/协调。
- **DGN**：已提交夹具 `test_dgnv8.dgn` 的标签只能叫 `tag_0`；端到端走合成 `cadkit_core::Document`。
  **上游 tag 解码本身未在真实 V8i/CONNECT 文件验证**。
- **IFC 关系导出**：**未实现**（只做落盘 + 导入；导出仍不写 `IfcRelAggregates`/`IfcRelNests`）。
- **IFC `Text` 值**：`BimValue::Text` 无 `ifc_type`，导出恒 `Label`。
- **BIM 查询/选择按载体**：`cad_bim_query`/`cad_bim_summary` 在 Linux native（`cad-host`）与 Web
  （`DirectReads`）已接线，Windows/macOS/Android **无 seam → `unsupported-runtime: bimQuery`**
  （显式拒绝，不是空页）；`cad_select_entities` 各载体 tick 已接线，但 Windows/macOS/Android 只有
  源码一致性。**全部载体运行时 NOT VERIFIED**。
- **`app-macos` 编译**：`cc-rs` 编 `objc2-exception-helper` 需要 macOS SDK，本机不可用 → **NOT RUN**
  （环境缺失，不是代码失败）；**Android 编译 NOT RUN**。
- **运行时/真机/GPU/像素/性能**：Web 运行时、Windows/macOS/Android 点击、真机、GPU、像素、性能、
  跨机自包含一律 **NOT RUN / NOT VERIFIED**。合成夹具与单文件观察不得推广为验收。
