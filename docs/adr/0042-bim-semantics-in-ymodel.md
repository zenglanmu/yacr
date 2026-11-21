# ADR 0042：`.ymodel` 的 BIM 语义层（参照 iModel/BIS 的取舍）

- 状态：**Accepted（2026-11-21）**；**Phase 1 已实现**（格式与存储、IFC 语义导入、属性面板、
  端到端契约），Phase 2（IFC 导出回写、RVT/DGN 语义）与 Phase 3（查询/过滤/明细表、
  关系与 Aspect 落盘）**未开始**，见 §4。
- 日期：2026-11-21。
- 相关：[ADR-0037](0037-ymodel-schema-and-persistence.md)（`.ymodel` 是唯一原生格式、BIS 骨架）、
  [ADR-0033](0033-multi-format-import.md)（连接器、各格式定位）、
  [ADR-0038](0038-mesh-base-color-texture.md)/[ADR-0039](0039-mesh-pbr-channels.md)/
  [ADR-0040](0040-ymodel-embedded-resources.md)（minor 升级与"按文档取值"先例）、
  [ADR-0035](0035-rvt-vendored-patch-policy.md)（`vendor/rvt` 受控补丁）、
  [ADR-0041](0041-save-export-split-and-format-provenance.md)（保存/导出分离）。
- 影响：`cad-domain`（ymodel 版本与 descriptor）、`cad-db`（BIM 旁表、快照编解码、builder）、
  `cad-store-zip`（表项、minor 三档）、`cad-import-core`、`cad-import-ifc`、`cad-import-rvt`、
  `cad-import-dgn`、`cad-export-ifc`、`cad-query`、`cad-app`（属性行）、`cad-ui-core`（i18n/wire）、
  `apps/desktop-ui`、`docs/compatibility.md`、`docs/dgn.md`。
- **不改变**：`.ymodel` 仍是唯一可直接再打开的格式；载体归属不变；patch 禁令不变（只允许
  `acadrust`/`rvt`，且 `rvt` 仅 `src/ifc/gltf.rs` 受控改动）。

---

## 0. 背景

三个导入器今天都只把 BIM 信息读成"几何 + 颜色"，构件语义在入库前就丢了：

- `cad-import-rvt` 调 `rvt::ifc::run_export` 拿到完整 `IfcModel`，随即只用
  `model_to_glb(&run.model)` 的 GLB，`IfcModel` 被丢弃（`crates/cad-import-rvt/src/decode.rs:194,225`）；
  GLB 节点其实带 `name` 与 `extras{entityIndex,ifcType}`（`vendor/rvt/src/ifc/gltf.rs:95-117`），
  但 `PlacedMesh.name` 读到后从未使用（`crates/cad-import-rvt/src/glb.rs:92-106`、
  `src/lib.rs:266-295`）。`rvt` 0.25.0 的 `IfcEntity::BuildingElement` 已有
  `ifc_type/name/type_guid(ElementId)/predefined_type/storey_index/material_index/property_set/host`。
- `cad-import-ifc` 只 `decode_at_with_id` 有几何的 product；`GlobalId`/`Name`/`Tag`、
  `IfcBuildingStorey`/`IfcRelAggregates`、`IfcTypeObject`/`IfcRelDefinesByType`、`IfcPropertySet`、
  `IfcMaterial`、`IfcSpace` 全部未读；`IFCSTYLEDITEM` 只计数。
- `cad-import-dgn` 只导 primary design model 的几何与 level 名；`cadkit` 已解码的 tags/Item Types
  在适配器里被丢弃（`import.dgn.tags_not_stored`），因为 `cad-db` 没有落库目标。

同时，`DbEntity`/`DbObject` 没有任何名称或属性字段（`crates/cad-db/src/entity.rs:5-21`），属性面板
只有 `entity/geometry/text/display` 四组（`crates/cad-app/src/selection.rs:36-76`），导出侧
`cad-export-ifc` 只会给每个 mesh 写 `IfcBuildingElementProxy`。BIM 场景（构件清单、按楼层/类别
统计、按属性过滤、回写 IFC）因此无从做起。

Bentley iModel/BIS 提供了成熟的参照：内部 Id 与 FederationGuid 分离、共享定义（Type/Category）
与实例分离、可选属性走 Aspect、来源可回溯（ExternalSourceAspect）、增量以 changeset/checkpoint
表达。本 ADR 只取其中能落进"zip 容器 + JSON 分片 + 显式保存"的部分，明确不引入 EC Schema/ECSQL
体系（见 §7）。

## 1. 决策

### D1 — 只读契约 + 显式状态，不做"看起来像 BIM"

任何 BIM 字段都不得来自猜测：源没给就是 `None` 并发稳定诊断；DGN 的启发式识别默认关闭，开启时
`evidence: "heuristic"` 且状态 `Unverified`。逐载体状态天花板见 §3。

### D2 — 逐元素 BIM 数据放在元素分片里，元素行加可选 `bim` 块

不放进 `tables/`：store 对 `tables/*` 是**整表重编码**（`crates/cad-store-zip/src/write.rs:416-422`，
`ChangeScope.tables` 只是信息性的，`crates/cad-db/src/scope.rs:119-131`），逐元素数据进表会让每次
保存丢掉增量语义（铁律 4）。元素分片才有 raw-copy（`write.rs:466-495`）。

形状（minor 6 起，只有带 BIM 的行才出现；`None` 时**整个键省略**，绝不写 `"bim": null`——旧构建
的元素解码是严格的，多一个字段即 `invalid_document`，见 `crates/cad-db/src/snapshot/elements.rs:355-364`）：

```json
"bim": {
  "category": "IfcWall",
  "name": "Basic Wall:Generic 200mm",
  "source": {"connector": "ifc", "document_id": "2O2Fr$t4…", "id": "2O2Fr$t4…", "native_id": "22"},
  "level": "level:ifc:<document_id>:<id>",
  "type": "type:ifc:<document_id>:<id>",
  "material": "material:ifc:<document_id>:<id>",
  "psets": [{"name": "Pset_WallCommon",
             "properties": [{"name": "IsExternal", "value": {"kind": "bool", "value": true}},
                            {"name": "Reference", "value": {"kind": "real", "value": 0.2,
                                                           "ifc_type": "IfcPositiveLengthMeasure",
                                                           "unit": "m"}}]}],
  "evidence": "declared"
}
```

- `BimValue` 只允许 `text|real|integer|bool|null` + 可选 `ifc_type`/`unit`；Complex/Enumerated/
  Bounded/Quantity 不压成字符串冒充，计数并报 `*.bim.property_value_unsupported`。
- 一个源构件被拆成多段几何时，每行保留自己的 `EntityId`，逻辑构件靠 `bim.source` 等价类归组；
  统计/明细按该等价类去重。
- `bim` 也可出现在非 mesh 元素上（DGN 矢量构件），形状相同。

### D3 — 共享定义放一张 `tables/bim-definitions.json`，键是可读复合字符串

```json
[{"id": "level:ifc:<doc>:<id>", "kind": "level", "name": "Level 1", "elevation": 0.0,
  "source": {"connector": "ifc", "document_id": "<doc>", "id": "<id>"}, "evidence": "declared"},
 {"id": "type:ifc:<doc>:<id>", "kind": "type", "name": "Basic Wall:Generic 200mm",
  "class": "IfcWallType", "category": "IfcWall", "psets": [], "evidence": "declared"},
 {"id": "material:ifc:<doc>:<id>", "kind": "material", "name": "Concrete",
  "evidence": "declared"}]
```

- 定义 id **不是** `PersistentId`：不进 `IdKind`、不需要高水位、不碰 `header.rs`（991 行，接近
  `check-file-size.py` 上限）。这与 `textures/<key>` / `resources/<kind>/<key>` 的文档局部目录键同类。
- **Category 不建表**：它是元素/类型上的开放 token（IFC 实体名规范拼写；RVT 用 `ifc_type`；
  DGN 只认文件自带的显式标签），类别列表由元素聚合。
- 定义表只在非空时写（照 `resources` 的 `build.rs` 条件写法），否则所有文档都会升 minor。
- 预算：`BIM_MAX_DEFINITIONS = 100_000`、编码体积 32 MiB；超预算丢弃超出部分、把相关元素引用置
  `None` 并报 `*.bim.budget_exceeded` + `Partial`，**不写悬空引用**。
- 多对多关系（group/system/connects）Phase 1 不落盘；Phase 3 以关系元素进分片或单表（上限
  100k 行 / 32 MiB，超出显式拒绝）。

### D4 — 属性集内联；属性面板用固定静态键 + 摘要值

`PropertyRow.key` 是 `&'static str`（`crates/cad-app/src/selection.rs:123-125`），多选交集用
`rows.iter().find(|row| row.key == key)`（`:406`），重复/动态 key 会静默丢行。因此 Phase 1 只发：

`bim_category`、`bim_name`、`bim_level`、`bim_type`、`bim_material`、`bim_source`、
`bim_properties`（单行摘要，例如 `Pset_WallCommon (3), Pset_Revit_Type (5)`）、`bim_evidence`。

逐属性行、动态属性名、属性编辑一律 Phase 3，且必须先改 wire 契约（`PropertyRow` 增加
`label`，`key` 变 `Cow<'static, str>`）——不在本轮偷偷改。

### D5 — 标识：内部 `EntityId` 稳定 + 源标识并存

- 去重优先级：`(connector, document_id, id)` → `(connector, checksum, id)` → `EntityId`。
- 文档级 `checksum` 已在 `tables/sources.json`（`crates/cad-app/src/host/report_json.rs:98-130`），
  **不在元素上重复存 64 字节哈希**。
- RVT 的 `Unique Document GUID` 每次保存都会变（`vendor/rvt/src/ifc/mod.rs` 明示），IFC 的
  GlobalId 只在文档内唯一：**不承诺跨文件/改后重导入自动合并**；同一批次内按 `bim.source` 归组，
  重复的 `(document_id, id)` 保留两行并报 `import.bim.duplicate_source_id`（`Partial`）。

### D6 — `FORMAT_MINOR` 升到 6，写入按文档取 4/5/6

- `cad-domain/src/ymodel/mod.rs:91` 改为 6，注释追加"6 = BIM 元素块 + `tables/bim-definitions.json`"。
- `document_minor()` 必须显式三档（现 `resources.rs:285-306` 直接返回 `FORMAT_MINOR`，不改会让
  "只有资源"的文档被写成 6，老构建无谓拒绝）：

  `has_bim` → 6；`embed_resources`/有资源条目 → 5；否则 → 4。

  `has_bim` 落在 `cad-db`（`DocumentSnapshot` 方法，store 不认识 `"bim"` 字面量）。
- 兼容：旧读新 → `ymodel.unsupported_minor`（`crates/cad-store-zip/src/read.rs:146-166`），不静默丢
  `bim`；新读旧 → 元素无 `bim` = `None`、无定义表 = 空定义，正常打开。
- `schema/classes.json` 多一个字段不影响旧读（读侧只做形状检查）。

### D7 — IFC 导入：第二遍语义扫描，天花板 `Partial`

几何路径不变；新增 `crates/cad-import-ifc/src/semantics.rs` 只扫关系/类型/属性集。

| 读什么 | 落到 | 诊断码 |
|---|---|---|
| `IfcRoot.GlobalId`/`Name`/`Tag` | `bim.source.id`/`native_id`、`bim.name` | `import.ifc.bim.source_id_missing` |
| `IfcBuildingStorey` + `IfcRelAggregates` / `IfcRelContainedInSpatialStructure` | `level` 定义 + `bim.level` | `import.ifc.bim.storey_unresolved` |
| `IfcTypeObject` 家族 + `IfcRelDefinesByType` | `type` 定义（含类型 PSet） | `import.ifc.bim.type_unresolved` |
| `IfcPropertySet` + `IfcRelDefinesByProperties` + `IfcPropertySingleValue` | `bim.psets` / 类型 `psets` | `import.ifc.bim.property_set_unsupported`、`property_value_unsupported`、`quantity_set_not_stored` |
| `IfcMaterial` + `IfcRelAssociatesMaterial` | `material` 定义 + `bim.material` | `import.ifc.bim.material_unresolved` |
| `IfcSpace` | 普通元素，`category="IfcSpace"` | — |
| `IfcStyledItem` 颜色 | 仍不导入 | 沿用 `import.ifc.style_not_imported` |

单元素上限：64 个 PSet、每 PSet 256 属性、字符串 4 KiB；读侧同样限流，超限
`ymodel.invalid_document`（detail 指明 bim）。

### D8 — RVT 导入：直接用 `IfcModel`，不解析 GLB extras，天花板 `Unverified`

`decode_to_glb` 改为返回 GLB + 本 crate 私有的 `RvtSemantics`（定义表 + 逐元素 `Option<BimElement>`），
并用 `node.extras().entityIndex` 把几何行对齐到 `model.entities[i]`。GLB extras 只有
`entityIndex/ifcType`，缺 storey/type/material/pset，从它猜等于重建劣化版。

| 读什么 | 落到 |
|---|---|
| `BuildingElement{ifc_type,name}` | `bim.category`（规范化为 IFC 拼写）、`bim.name` |
| `global_ids.elements[i]`（缺则 `type_guid`→`native_id`） | `bim.source` |
| `building_storeys[storey_index]`（`elevation_feet*0.3048`） | `level` 定义 |
| `element_type_ids` + `Family:Type` 名 | `type` 定义 + 类型 PSet |
| `materials[material_index]` | `material` 定义 |
| `property_set` | `bim.psets` |

`IfcEntity::Aggregate` 的 parts Phase 2 不归组（`import.rvt.bim.aggregate_parts_not_grouped`）。
占位楼层（rvt 只在缺楼层时合成）不得当真实楼层导入。真实 RVT 文件的 BIM 语义 **NOT VERIFIED**。

### D9 — DGN 导入：只认文件自带标签，天花板 `Unverified`

- `cadkit` 已解码的 tags/Item Types 写成一个 `BimPropertySet`（`evidence: "declared"`，因为值是
  文件作者写的），替代今天的 `import.dgn.tags_not_stored`（旧码只保留给"确实无法解码/超预算"）。
- 只有标签名明确是 `Category`/`IfcType`/`IFCEntity`（大小写不敏感）且值是类别 token 时才写
  `bim.category`；否则 `None` + `import.dgn.bim.no_category`。
- **level 不是楼层**、cell 名不是类别、几何形状不推断类别；启发式若将来开启必须是显式导入选项 +
  `evidence: "heuristic"` + `Unverified`。

### D10 — IFC4 导出回写：无 BIM 路径逐字节不变

- 无 `bim` 的数据库仍走现有 `build_plan`（`IfcBuildingElementProxy` + 既有 GlobalId 派生），
  既有确定性/黄金测试不动。
- 有 `bim` 时新增 `crates/cad-export-ifc/src/bim.rs`：`IfcBuildingStorey`（按 `(elevation,name)`
  排序）、`IfcClass::Named(category)`（非法类别回退 Proxy + `export.ifc.bim.category_not_written`）、
  `TypeLink`（白名单类别才派生 `<Category>Type`，否则跳过并报）、`MaterialLink`（按名去重）、
  `with_psets`（`BimValue` → `PropertyValue`；不可表示的值跳过并汇总一条诊断）。
- GlobalId：源里有且合法就复用（先在导出端实现 22 字符 IFC base64 → `u128` 解码并加已知向量测试；
  `ifc-lite-core` 只提供 encode），冲突/非法回退 `stable_global_id` 并报
  `export.ifc.bim.global_id_invalid|global_id_collision`。IFC→ymodel→IFC 的 GlobalId 因此稳定。
- `evidence: "unverified"` 的 PSet 默认不导出（`include_unverified=false` + `unverified_skipped`）。

### D11 — 查询/过滤/统计：`cad-query` 内存索引，Phase 3 才上 wire/UI

`cad-query/src/bim.rs` 按 `(DatabaseId, Revision)` 懒重建：`by_category`/`by_level`/`by_type`/
`by_material`/`by_property`，每个集合成员是 `BimKey`（= `bim.source` 等价类，缺源 id 时 `EntityId`），
统计按 `BimKey` 去重。`BimFilter`（集合间 AND，最多 64 项）、`BimRow`、`BimSummary` 分页复用
`QueryRequest`/`QueryPage`。Phase 3 才加 `cad_bim_query`/`cad_bim_summary`（照 `cad_layers` 先例），
且不进 `cad_command` 文本命令表（不欠 `check-command-docs.py` 的文档债）。

### D12 — `IdKind` 预留位（11/12/13）Phase 1 不启用

- 不用 `SubCategory(11)` 表示 BIM 类别——它已是 DWG layer 的镜像（ADR-0037 D7），叠加会破坏不变量。
- 启用 `Relationship(12)`/`Aspect(13)` 需要：`IdCounters` 加字段、`from_json_value` 接受新文档缺省、
  `document_minor` 升 7、只用 `header.mint()`（禁 `max(id)+1`）。Phase 3 再做。

### D13 — 非目标（不照搬 BIS）

不做 EC Schema/EXPRESS/ECSQL；不做 briefcase/changeset 合并协议/锁；不做跨文件联邦（ADR-0037 D15）；
不建 Aspect 类层次（PSet 是扁平结构）；不做通用关系代数；不给普通 CAD 实体发 GUID；不把
DefinitionElement 迁移提前（样式/图层/线型/块定义仍在 `tables/*`，ADR-0037 D7）。

## 2. 兼容矩阵

| 文档 | 写入 minor | 旧构建（≤5） | 新构建 |
|---|---|---|---|
| 无 BIM、无资源 | 4 | 打开 | 打开，`bim=None` |
| 有资源、无 BIM | 5 | 拒绝（`unsupported_minor`，本就如此） | 打开 |
| 有 BIM | 6 | 拒绝 `ymodel.unsupported_minor` | 打开 |
| minor 5 但被塞了 `bim`（非合规） | 保存时变 6 | — | 接受并保留，不丢数据 |

## 3. 各载体状态天花板

状态列是**目标**天花板；"实现"列才是今天的事实（2026-11-21）。

| 来源 | 实现 | 目标 BIM 语义状态 | 说明 |
|---|---|---|---|
| IFC（`cad-import-ifc`） | **已实现（Phase 1）** | `Partial` | 关系/类型/PSet 读取为合成契约证据；Complex/枚举/数量/样式不支持 |
| RVT（`cad-import-rvt`） | **未实现（Phase 2）** | `Unverified` | 今天的 `cad-import-rvt` 仍丢弃 `IfcModel`、不用 GLB extras，元素没有 `bim` 块；目标源是 `rvt` 的 `IfcModel`，真实 RVT 文件未验证，预览几何不变 |
| DGN（`cad-import-dgn`） | **未实现（Phase 3）** | `Unverified` | 今天的 tags/Item Types 仍无落库目标（`import.dgn.tags_not_stored`）；目标只认文件自带 Item Type/tag，level/cell/几何不推断 |

## 4. 分期与验收

**Phase 1（已落地，2026-11-21）** — 提交 A：格式与存储（`cad-domain` minor 6 + descriptor；新 `cad-db/src/bim.rs`、
`snapshot/elements/bim.rs`、`snapshot/bim.rs`；`TableKind::BimDefinitions`；`cad-store-zip` 表项 +
三档 `minor.rs`）。提交 B：IFC 语义导入 + 属性面板（`cad-import-core` 钩子、`cad-import-ifc/src/semantics.rs`、
合成 IFC fixture、`cad-app/src/selection_bim.rs`、i18n、`property-labels.ts`）。

**已落地的契约测试**（2026-11-21；证据与命令见
`docs/validation-bim-2026-11-21.md`）：元素 `bim` 往返 + 缺省省略
（`element_bim_round_trips_and_is_omitted_when_absent`）；minor≤5 行解成 `None`
（`absent_bim_on_a_minor_five_row_decodes_as_none`）；`null`/未知字段被拒
（`a_row_with_bim_null_is_rejected`、`a_unknown_bim_field_is_rejected`）；定义表严格解码 + 拒悬空引用
（`bim_definitions_table_decodes_strictly_and_rejects_dangling_refs`）；定义行必须带 kind 前缀
（`a_definition_row_must_carry_its_kind_prefix`）；预算超限显式报错
（`bim_budget_exceeded_is_an_explicit_error`、`an_oversized_bim_string_in_an_archive_is_refused`）；
BIM 文档写 6 并能重开、无 BIM 仍是 4、只有资源仍是 5
（`crates/cad-store-zip/tests/bim_minor.rs`：`a_bim_document_writes_minor_six_and_reopens`、
`a_bim_free_document_stays_minor_four`、`a_resources_only_document_stays_minor_five`；
minor-5 无 BIM 可开、minor-5 带 BIM 保存时升 6：`a_minor_five_archive_without_bim_opens`、
`a_minor_five_archive_that_holds_bim_is_accepted_and_resaved_at_six`）；
IFC 墙的 GlobalId/楼层/类型/材质/PSet 落库
（`ifc_wall_globalid_storey_type_material_pset_are_stored`）；一个 IFC product 只算一个构件
（`one_ifc_product_stays_one_bim_element`）；Complex 属性只报不造
（`complex_property_is_reported_not_fabricated`）；无关系 product 只有 source+category
（`a_product_without_relations_has_source_and_category_only`）；`IfcSpace` 是普通元素
（`an_ifc_space_is_an_ordinary_element_with_its_category`）；预算不制造悬空引用
（`bim_budget_exceeded_keeps_no_dangling_reference`）；没有 BIM 关系的导入结果不变
（`import_without_bim_relations_stays_unchanged`）；面板行用静态键 + PSet 摘要
（`cad-app`：`bim_panel_rows_use_static_keys_and_a_pset_summary`）；非 BIM 元素没有 BIM 行
（`a_non_bim_element_has_no_bim_rows`）；宿主端到端五条
（`crates/cad-host/src/tests/ymodel_bim.rs`：`ifc_bim_survives_a_ymodel_save_and_reopen`、
`a_bim_document_is_written_at_minor_six`、`a_bim_free_document_stays_at_minor_four`、
`reopening_a_bim_document_exposes_bim_property_rows`、`a_plain_mesh_element_has_no_bim_rows`）。

验证命令（逐条看退出码，不用 `| tail -1`）：

```bash
cargo test -p cad-domain --locked descriptor
cargo test -p cad-db --locked bim
cargo test -p cad-store-zip --locked --test archive
cargo test -p cad-store-zip --locked --test document_fixed_point
cargo test -p cad-import-core --locked && cargo test -p cad-import-ifc --locked
cargo test -p cad-app --locked selection
python3 scripts/check-fixture-manifest.py
python3 scripts/check-i18n.py && python3 scripts/check-file-size.py && python3 scripts/check-architecture.py
(cd apps/desktop-ui && npm ci && npm test)
cargo check -p cad-db --target wasm32-unknown-unknown --locked
cargo clippy -p cad-db -p cad-store-zip -p cad-import-core -p cad-import-ifc -p cad-app --all-targets --locked -- -D warnings
```

**Phase 2** — IFC 导出回写（storey/type/pset/material/GlobalId 往返）；RVT 语义（`IfcModel` +
GLB `entityIndex`）；来源对账 helper。证据上限：IFC 只做自洽往返，无 vendor checker；RVT 真实文件
**NOT VERIFIED**。

**Phase 3** — DGN tags/Item Types → PSet + 显式 Category 标签；`cad-query` BIM 索引/过滤/统计 +
`cad_bim_query` wire + React 过滤/明细表；关系/Aspect 落盘（`IdKind` 12/13 + 高水位 + minor 7）；
数量/`IfcElementQuantity`。证据上限：DGN BIM 解释 `Unverified`；UI 数据来自合成/契约样本。

## 5. 风险与规避

1. **`tables/` 整表重写**：定义表每次保存全量重编码。规避：只放共享定义 + 预算；Phase 2 实测保存
   耗时；超标则 Phase 3 迁到 DefinitionModel 分片元素（minor 7）。
2. **`document_minor` 复用 `FORMAT_MINOR`**：会把 resources-only 文档写成 6。规避：显式三档 +
   单测；定义表只在非空时写。
3. **`PropertyRow.key` 静态契约**：动态键会在多选 `find` 里静默丢行。规避：Phase 1 固定键 + 摘要。
4. **`bim: null` 破坏旧读**：规避：None 省略键，并有 minor-4/5 字节兼容测试。
5. **把源标识当身份**：RVT 文档 GUID 每次保存都变。规避：`document_id+id` 优先、checksum 只作
   provenance、跨保存不自动合并。
6. **文件规模与并发**：`cad-db/src/snapshot/tables.rs`、`ymodel/header.rs`、`cad-store-zip/src/write.rs`、
   `cad-import-rvt/src/glb.rs` 都接近 1000 行上限。规避：新代码全部放新模块，既有文件只做小接线。

## 6. 证据边界

本 ADR 的所有映射结论都是**代码契约**，不是真实模型验收：

- IFC 语义导入只有合成 fixture 证据，**无 vendor 工具交叉验证**；
- RVT 的 BIM 语义建立在 `rvt` 0.25.0 的 `IfcModel` 上，其上游自身声明输出不得用于测量/协调；
  真实 RVT 文件 **NOT VERIFIED**；
- DGN 的 Item Type/tag 语义无真实 V8i/Connect 样本，**NOT VERIFIED**；
- IFC 导出回写只承诺自洽往返，**不承诺**被 Revit/Bentley/IfcOpenShell 接受；
- 所有平台结论仍按 `docs/compatibility.md` 与 AGENTS.md 平台表：只在 Linux 做运行/契约验证，
  Windows/macOS/Android/Web 只能写"仅编译 / 仅契约 / NOT VERIFIED"。
