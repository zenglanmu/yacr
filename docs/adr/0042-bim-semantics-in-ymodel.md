# ADR 0042：`.ymodel` 的 BIM 语义层（参照 iModel/BIS 的取舍）

- 状态：**Accepted（2026-11-21）**；**Phase 1 已实现**（格式与存储、IFC 语义导入、属性面板、端到端
  契约），**Phase 2 与 Phase 3 已实现（2026-11-21）**——IFC 导出回写、RVT/DGN 语义、DGN 标签 PSet、
  IFC 数量集与关系落盘、`cad-query` BIM 索引 + 单一 `cad_bim_query` wire（页与 facet `summary`
  一次返回）、React BIM 面板与 `cad_select_entities`。**三项明确边界**：① **IFC 关系只导出 `contains`/`nests`**（
  `IfcRelContainedInSpatialStructure`/`IfcRelNests`；`aggregates`/`group`/`connects`/`other:*` 因上游
  plan API 无法表达任意产品间关系而**不导出并逐类报告**，不写成容器/嵌套冒充）；②
  **Windows/macOS/Android 已装 BIM 查询 seam**（Windows 仅交叉编译，macOS/Android 仅编译/契约，
  **运行时 NOT VERIFIED**）；没有查询通道的载体仍按 `unsupported-runtime: bimQuery` 显式拒绝
  （不是空页）；③ `cad_db::BimValue::Text` 无 `ifc_type`，
  导出恒写 `Label`。RVT/DGN 语义仍 `Unverified`；**BIM 属性编辑已落地**（`c87e4046`：逐属性动态行 +
  `apply_bim_updates` + `BimPatch`，见 D4）；Aspect 落盘仍未开始，见 §4。
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
- **Phase 3 已落地（有界单表，2026-11-21）**：多对多关系落 `tables/bim-relations.json`，**不启用**
  `IdKind` 12/13、不占高水位、不碰 `header.rs`。行形状（`document_id` 与元素 `bim.source` 同一拼写）：

  ```json
  {"kind": "aggregates",
   "parent": {"kind": "source", "connector": "ifc", "document_id": "…", "id": "…"},
   "child":  {"kind": "entity", "entityId": 7},
   "source": {"kind": "source", "connector": "ifc", "document_id": "…", "id": "…"},
   "label": null}
  ```

  - `kind` ∈ `aggregates | nests | contains | group | connects | other:<token>`；未知 token 是坏文档，
    不是"新类型静默接受"。
  - `parent`/`child`/`source` 复用元素 `bim.source` 的 `BimKey` 语义：`{"kind":"source",connector,
    document_id,id}` 或 `{"kind":"entity",entityId}`（持久化 id，必须属于 entity 家族）。
  - `source`/`label` **恒写键**，无值即 `null`；省略键、额外键、错误类型都按坏文档拒绝。
  - 只在非空时写；预算 **100 000 行 / 32 MiB 编码体积**，超限 `ymodel.bim.budget_exceeded`
    **显式拒绝**，不静默截断。
  - 行键 `(kind, parent, child)`：重复行在写入（`BimRelations::insert`）与读取（表解码）两侧都
    **拒绝**，不静默合并（同端点不同 label 也算重复）。
  - 关系端点**不要求**解析到现存元素（IFC 的一个 product 可能没有几何行）；但形状与预算在读侧
    同样严格，防止恶意归档撑大边集。
  - 关系与元素/定义同属 **minor 7**；有 BIM 定义但无关系的文档也写 7（同一发布周期，不拆档）。

### D4 — 属性集内联；属性面板用动态逐属性行，可写

`PropertyRow` 已按 Phase 1 的预告升级（`c87e4046`）：
`key: Cow<'static, str>`、`label: Option<String>`（源自己的名字，永不进 i18n catalogue）、
`editable`/`kind`/`options`（读侧元数据，只声明「写路径可改」）。多选交集仍按 key 查找，所以契约是：
**key 在一次选择内唯一**；动态 key 一律带前缀（`bim.pset.{set}.{prop}`）；同一 `(set, prop)` 在不同
元素上名字不一致时整行标 `mixed`、`value=None`、不可编辑——不会静默丢行。

面板行 = 静态摘要行（`bim_category`/`bim_name`/`bim_level`/`bim_type`/`bim_material`/`bim_source`/
`bim_evidence`）+ 每个已存非空 PSet 属性一行。**写**不经新命令：`SETENTITYPROPERTY` 在显示属性语法
之前认领 `bim.pset.*`，按元素自身存储的变体解析草稿（`Real` 保留 `ifc_type` 与单位 token），
全有或全无，一条事务/一个 revision/一条 undo；无变化时 `property.unchanged` 且不记历史。

落库是 `DrawingDatabase::apply_bim_updates`：先全量校验再改、replace-only（拒绝凭空造 `bim` 块）、
空批不 bump、自己构建 `ChangeScope.shards`，发布 `METADATA` changes。撤销走
`DrawingHistoryStep.bim_patches`（`BimPatch{id, before, after}`），`DrawingUndoRecord` 保持不变；
带 BIM patch 的 step **必须独占一步**（与实体 patch 走不同 db 路径，混用会跨两个 revision）。
Heuristic 元素可编辑：改的是文档已存的值，`evidence` 原样保留。契约测试见 §4。

### D5 — 标识：内部 `EntityId` 稳定 + 源标识并存

- 去重优先级：`(connector, document_id, id)` → `(connector, checksum, id)` → `EntityId`。
- 文档级 `checksum` 已在 `tables/sources.json`（`crates/cad-app/src/host/report_json.rs:98-130`），
  **不在元素上重复存 64 字节哈希**。
- RVT 的 `Unique Document GUID` 每次保存都会变（`vendor/rvt/src/ifc/mod.rs` 明示），IFC 的
  GlobalId 只在文档内唯一：**不承诺跨文件/改后重导入自动合并**；同一批次内按 `bim.source` 归组，
  重复的 `(document_id, id)` 保留两行并报 `import.bim.duplicate_source_id`（`Partial`）。

### D6 — `FORMAT_MINOR` 升到 7（6 保留为只读档），写入按文档取 4/5/7

- Phase 1 曾把 `cad-domain/src/ymodel/mod.rs` 的 `FORMAT_MINOR` 从 5 升到 6；Phase 3 再升到 **7**
  （关系表）。注释写明：6 = 元素 `bim` 块 + `tables/bim-definitions.json`，7 = `tables/bim-relations.json`。
- `document_minor()` 必须显式分档（现 `crates/cad-store-zip/src/minor.rs`），不能直接返回
  `FORMAT_MINOR`，否则"只有资源"的文档会被写成 7，老构建无谓拒绝：

  `has_bim`（元素块 / 定义表 / 关系表任一存在）→ 7；`embed_resources`/有资源条目 → 5；否则 → 4。

  **6 是只读旧档**：本构建能打开 minor-6 归档，但再保存一律写 7；"有 BIM 定义但无关系"也写 7，
  因为元素 `bim` 块本来就会让 minor ≤ 6 的构建拒绝，拆档没有收益。

  `has_bim` 落在 `cad-db`（`DocumentSnapshot` 方法，store 不认识 `"bim"` 字面量或关系表路径）。
- 兼容：旧读新 → `ymodel.unsupported_minor`（`crates/cad-store-zip/src/read.rs`），不静默丢
  `bim`/关系；新读旧 → 元素无 `bim` = `None`、无定义表 = 空定义、无关系表 = 空关系，正常打开。
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
- **关系导出（2026-11-21 补齐）**：`crates/cad-export-ifc/src/bim_relations.rs` 读
  `tables/bim-relations.json`（按 `(kind,parent,child)` 顺序）并只写固定版本 plan API 能表达的两类：
  `contains` → 覆盖元素的默认楼层容器（一条边多个父时取第一条可解析，其余计
  `export.ifc.bim.relation_not_written`）；`nests` → `NewObject::with_nest_in`，disposition 顺序保证宿主
  先写（环上丢一条边并计数）。`aggregates`/`group`/`connects`/`other:<token>` 无 plan 级表达
  （API 没有"两个产品之间的关系"）→ **不伪造**为容器/嵌套，按 kind 汇总报告
  `export.ifc.bim.relation_not_written`；端点不在本次导出（无几何行/未写出）→ 跳过并计
  `export.ifc.bim.relation_endpoint_missing`，不 panic、不造对象。无关系表的文档仍走同一路径且
  **逐字节不变**（钉住 2196/1374 继续生效）；关系表的端点可用 `BimKey::Source` 或 `Entity`。

### D11 — 查询/过滤/统计：`cad-query` 内存索引，Phase 3 才上 wire/UI

`cad-query/src/bim.rs` 按 `(DatabaseId, Revision)` 懒重建：`by_category`/`by_level`/`by_type`/
`by_material`/`by_property`，每个集合成员是 `BimKey`（= `bim.source` 等价类，缺源 id 时 `EntityId`），
统计按 `BimKey` 去重。`BimFilter`（集合间 AND，最多 64 项）、`BimRow`、`BimSummary` 分页复用
`QueryRequest`/`QueryPage`。

**Wire 只有一个命令（2026-11-21 收口）**：`cad_bim_query`（照 `cad_layers` 先例），不进
`cad_command` 文本命令表（不欠 `check-command-docs.py` 的文档债）。早期设计里的第二个
`cad_bim_summary` 命令**已删除**：宿主一次绑定（同一 `QueryRequest` 的 document/revision）同时算
分页行与 facet 汇总，应答在 `databaseId/revision/total/offset/limit/stale/rows` 之外带 `summary`
块（形状与旧汇总应答一致，`summary.total == total`，即过滤后总数而不是当前页行数）。`limit: 0`
仍然只回总数——但 `summary` 照常计数（汇总是单个聚合行，不受页窗口影响）。命名同步收口为查询语义：
`BimQueryHost`/`BimQueryReads`/`BimQuerySeam`、`configure_bim_query_host`、
`Shared::cad_bim_query_host`；`unsupported-runtime: bimQuery` 标记不变。trait 留在 `cad-ui-core`
而不是 `cad-desktop-shell`，因为 registry 的共享行必须让 wasm 与 native 两个传输都看到它——分层约束，
不是遗漏。

**2026-11-21 追加（方案 B：通用 host-read seam）**：上面那组私有命名**已被通用契约吸收**。
请求携带型读取现在只有一份 seam：

- `cad_ui_core::host_read::HostReadRequest`（`#[serde(tag = "kind")]`，当前唯一 kind 为
  `bimQuery`，拼写只在 `BIM_QUERY_KIND` 定义并被单测钉住）与 `HostReadAnswer`（同 kind 标签）；
- `HostReadHost`（载体的平台侧）与 `HostReads`（wire 侧，`dom-canvas` 直读 session、
  desktop 经 `HostReadSeam` 转发）；
- 无 seam 的默认拒绝仍按 kind 取自 `no_seam_refusal`（BIM = `unsupported-runtime: bimQuery`）；
  未知 kind 是新的稳定标记 `unsupported-runtime: hostRead.kind`，**不得**被当作 BIM 静默处理。

`cad_bim_query` 的命令名、参数与 JSON 形状逐字节不变（冻结形状测试未改期望）；四个载体只剩一个
约 10–25 行的闭包注入（Linux/Windows/macOS 用 `on_main_thread`，Android 用有界
`run_on_main_thread_timeout`），装配点改为 `configure_host_read_host(HostReadFn::new(..))`。
**约束（新增 kind 时）**：只允许在 `host_read.rs` 加变体/拒绝词、在 `session::host_read` 加路由、
在 registry 加/改 wire 行；**不得**再新增每载体 seam 文件、struct 或 configure 函数。

### D12 — `IdKind` 预留位（11/12/13）Phase 1 不启用

- 不用 `SubCategory(11)` 表示 BIM 类别——它已是 DWG layer 的镜像（ADR-0037 D7），叠加会破坏不变量。
- 启用 `Relationship(12)`/`Aspect(13)` 需要：`IdCounters` 加字段、`from_json_value` 接受新文档缺省、
  `document_minor` 升档、只用 `header.mint()`（禁 `max(id)+1`）。**Phase 3 已落地但未启用它们**：
  多对多关系改走有界单表 `tables/bim-relations.json`（D3/D6）；12/13 仍留作将来把关系升级为关系元素，
  Aspect 落盘启动时再评估。

### D13 — 非目标（不照搬 BIS）

不做 EC Schema/EXPRESS/ECSQL；不做 briefcase/changeset 合并协议/锁；不做跨文件联邦（ADR-0037 D15）；
不建 Aspect 类层次（PSet 是扁平结构）；不做通用关系代数；不给普通 CAD 实体发 GUID；不把
DefinitionElement 迁移提前（样式/图层/线型/块定义仍在 `tables/*`，ADR-0037 D7）。

## 2. 兼容矩阵

| 文档 | 写入 minor | 旧构建（≤6） | 新构建 |
|---|---|---|---|
| 无 BIM、无资源 | 4 | 打开 | 打开，`bim=None` |
| 有资源、无 BIM | 5 | 拒绝（`unsupported_minor`，本就如此） | 打开 |
| 有 BIM 元素/定义、无关系 | 7 | 拒绝 `ymodel.unsupported_minor` | 打开，关系表为空 |
| 有关系（可带元素/定义） | 7 | 拒绝 `ymodel.unsupported_minor` | 打开 |
| 旧构建写的 minor 6（元素/定义，无关系） | 再保存变 7 | — | 接受并保留 |
| minor 5 但被塞了 `bim`（非合规） | 保存时变 7 | — | 接受并保留，不丢数据 |

## 3. 各载体状态天花板

状态列是**目标**天花板；"实现"列才是今天的事实（2026-11-21）。

| 来源 | 实现 | 目标 BIM 语义状态 | 说明 |
|---|---|---|---|
| IFC（`cad-import-ifc` / `cad-export-ifc`） | **已实现（Phase 1 + 3 导入，Phase 2 导出）** | `Partial` | 关系/类型/PSet 读取为合成契约证据；数量集已存为 PSet、五类 `IfcRel*` 已落 `tables/bim-relations.json`；Complex/枚举/样式不支持；导出仅自洽往返；关系只导出 `contains`/`nests`，`aggregates`/`group`/`connects`/`other:*` 不导出并报告 |
| RVT（`cad-import-rvt`） | **已实现（Phase 2）** | `Unverified` | `semantics.rs` 在 `model_to_glb` 前映射 `IfcModel`、用 GLB `entityIndex` 对齐几何；`type.class` 为 `None`（`IfcModel` 不暴露类型 class，派生就是猜）；`Aggregate` 不归组；真实 RVT 文件 **NOT VERIFIED**（单个样本计数只是契约观察），预览几何不变 |
| DGN（`cad-import-dgn`） | **已实现（Phase 3）** | `Unverified` | tags/Item Types 已落为 PSet（`evidence=Declared`）；只认文件自带的 `Category`/`IfcType`/`IFCEntity` 标签，level/cell/几何不推断；上游 tag 解码未在真实 V8i/CONNECT 文件验证 |

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

**Phase 2（已落地，2026-11-21）** — IFC 导出回写 + RVT 语义（证据与命令见
`docs/validation-bim-2026-11-21.md` §12–§13）：

- **IFC 导出**：`cad-export-ifc` 新增 `src/bim.rs`（948 行）、`src/bim_relations.rs`（277 行，
  关系解析）、`src/bim_value.rs`（286 行）、`src/tests_bim.rs`（763 行，**14 条**测试）、
  `src/tests_relations.rs`（660 行，**8 条**测试）；入口
  `IfcExporter::with_bim_policy(BimExportPolicy{include_unverified:false})`。在
  `database.bim_elements()` 或 `database.bim_relations()` 非空时启用，否则走旧路径且**逐字节不变**
  （钉住：非空 2196 字节 / FNV `ec72ad…`，空 1374 / `c56f8c…`，只出现 `IFCBUILDINGELEMENTPROXY`）。写出 `IfcBuildingStorey`
  （按 `(elevation,name,id)` 排序、米）、`IfcClass::Named(category)`（显式白名单，逐项对照内置 IFC4
  registry，未知回退 Proxy）、`<Category>Type`（白名单；`TypeLink` 首现创建、后续引用；与源 class
  不一致则跳过不替换）、`IfcMaterial`（按名去重）、PSet、源 `GlobalId` 复用（自带 22 字符 IFC
  base64 → `u128` 解码，向量含 `u128::MAX` 与夹具 `2O2Fr$t4X7Zf8NOew3FLSJ` 自往返；非法/重复回退
  `stable_global_id` 并报告）。**已知限制**：`BimValue::Text` 无 `ifc_type` → 恒写 `Label`；未知单位
  不写出并计数；`Null` 跳过；**关系只导出 `contains`/`nests`**（`aggregates`/`group`/`connects`/
  `other:*` 因 plan API 不可表达而按 kind 报告，不伪造）。证据只有自洽往返（导出 → `cad-import-ifc` 读回
  GlobalId 一致），**无 vendor checker**。
- **RVT**：`cad-import-rvt` 新增 `src/semantics.rs`（820 行）+ `src/tests_semantics.rs`（407 行，
  **7 条**）；`RvtDecode` 增 `semantics`，在 `model_to_glb` 前 `map_model(&run.model)`；`glb.rs` 增
  `PlacedMesh.entity_index`（读 `node.extras().entityIndex`）；8 个 `import.rvt.bim.*` 诊断。映射见
  D8；`type.class = None`（`IfcModel` 不暴露类型 class，派生就是猜）；文档级 `global_ids.document`
  不用。**单个真实样本**（`~/下载/rmeadvancedsampleproject.rvt`）读到 `elements=6783, levels=4,
  types=90, materials=0, element_psets=44928, with_category=6783, with_level=6702, with_type=6650,
  with_material=0`（该样本材料表为空，材料路径只有手工 `IfcModel` 单测证据）——**代码契约观察，真实
  RVT BIM 语义仍 NOT VERIFIED**（`rvt` 自述输出不得用于测量/协调）。

**Phase 3（已落地，2026-11-21）** — DGN 标签、IFC 数量集与关系、查询/wire/UI/选择：

- **DGN**：`crates/cad-import-dgn/src/bim.rs`（339 行）+ `bim/tests.rs`（425 行，**8 条**），按 D9；
  tags/Item Types 按 `set` 合成 PSet（无 set → `DGN_TAGS`）；浮点保持 `f64`；仅 `Category`/`IfcType`/
  `IFCEntity` 的单个 token 写 `bim.category`；`import.dgn.bim.{category_from_tag,no_category,
  unverified,budget_exceeded}`，`import.dgn.tags_not_stored` 收窄为「确实无法解码/无行」。端到端走
  合成的 `cadkit_core::Document`，**上游 tag 解码本身未在真实 V8i/CONNECT 文件验证**。
- **数量**：`crates/cad-import-ifc/src/semantics/quantity.rs`（194 行）+
  `tests_semantics_quantity.rs`（282 行，**6 条**）；`IfcElementQuantity` 作为 PSet 存进 `bim.psets`
  并保留源名（`Qto_*`）；Length/Area/Volume/Weight/Time → `Real{value, ifc_type: 源实体名, unit}`，
  Count → `Integer`；**不换算**；单位解析不到 `unit: None`；`import.ifc.bim.quantity_set_stored`
  信息性、不降级，`quantity_set_not_stored` 收窄为「确实读不出」。
- **关系**：`tables/bim-relations.json`（仅非空时写；`kind ∈ aggregates|nests|contains|group|
  connects|other:<token>`；行 `{kind,parent,child,source,label}`；端点复用元素 `bim.source` 键）。
  `cad-import-ifc` 导入五类 `IfcRel*`（端点 = 项目 GlobalId + 对象 GlobalId；关系 GlobalId/Name →
  `source`/`label`）；端点缺 GlobalId 跳过 + `import.ifc.bim.relation_endpoint_missing`；行超预算
  **整表拒绝** `relation_budget_exceeded`；**仅存在关系不降级**；`cad-import-core::MeshImport` 增
  17 行 `set_bim_relations` 转发。**采用有界单表（`FORMAT_MINOR` 7），未启用 `IdKind` 12/13**——
  仍留作将来把关系升级为关系元素；不占高水位、不碰 `header.rs`。**导出**见 D10：只写
  `contains`/`nests`，其余 kind 报告不写出。
- **查询**：`crates/cad-query/src/bim.rs`（865 行）+ `bim/tests.rs`（776 行，**10 条**）；按
  `(DatabaseId, Revision)` 懒建 category/level/type/material/property 索引，成员 `BimKey`，统计按
  `BimKey` 去重；facet 间 AND、facet 内 OR；属性算子 `Eq/Ne/Contains`（Real/Integer 互比）；term
  上限 64（`bim.filter.too_many_terms`）；行排序确定。
- **wire/UI/选择**：`crates/cad-ui-core/src/bim.rs`（冻结 camelCase 契约，`summary` 块随查询应答）
  + **18 条**测试（含「不得出现 snake_case 键」递归断言、`object.kind` 二态、`value.kind` 四态）；
  `session/bim.rs` + `session/host_read.rs`（原 `session/bim_reads.rs` 已删除）；registry **只有**
  `cad_bim_query` 一行（`cad_bim_summary`
  已删除，fixture 同步）；`cad-host/src/bim.rs` + `tests/bim_query.rs`（**11 条**，含
  `summary.total == total` 与过滤后 facet 计数）；`cad-desktop-shell` 命令 + 注册表 fixture；
  `apps/app-web` 已接线（`DirectReads`，**5 条**，含「一次调用同时拿 rows 与 summary」）；
  `apps/desktop-ui/src/bim/`（`bim-query.ts`/`bim-view.ts`/`bim-store.ts`/`selection.ts`/
  `panel.ts`/`use-bim-panel.ts`）+ `components/BimPanel.tsx` + ribbon 入口（两语言各 +61 键）；
  `bim-store` 每次动作（打开/刷新/筛选/翻页）只发一个 `invoke`；`cad_select_entities`
  （payload `{entityIds:number[]}`）。
- **读取 seam（`676b9d4f` 收口）**：请求携带型读取只剩 `cad_ui_core::host_read`
  （`HostReadRequest{kind:"bimQuery"}` → `HostReadAnswer`）。四个载体各只有一个 `host_read.rs`
  闭包（原 `bim_host.rs` 已删除）；无 seam 的载体按 `unsupported-runtime: bimQuery` 显式拒绝，
  未知 kind 按 `unsupported-runtime: hostRead.kind` 拒绝；**新增 kind 不得再新增每载体文件**。
  Linux 有运行证据，Windows 仅交叉编译，macOS/Android 仅编译/契约（运行时 **NOT VERIFIED**）；
  `cad_select_entities` 各载体 tick 已接线（Windows/macOS/Android 仅源码一致性，**NOT VERIFIED**）。
- **属性编辑（`c87e4046`）**：`crates/cad-app/src/app_drawing/bim_properties.rs`
  （`SETENTITYPROPERTY` 前置拦截 + 逐元素变体解析 + 全有或全无），代表测试
  `set_bim_key_writes_the_stored_variant_and_advances_the_revision`、
  `set_bim_key_refuses_a_conflicting_multi_selection`、
  `set_bim_key_reports_unchanged_without_an_undo_entry`、`undo_and_redo_restore_the_bim_value`、
  `set_bim_key_on_a_heuristic_element_is_allowed_and_keeps_the_evidence`；
  `crates/cad-db/src/tests/bim_edit.rs`（`apply_bim_updates_bumps_revision_and_dirties_the_element_shard`、
  `apply_bim_updates_is_all_or_nothing`、`apply_bim_updates_with_an_empty_batch_is_a_no_op` 等）；
  `cad-history` 的 `BimPatch` 往返。
- **IFC 关系导出（`c87e4046`）**：`crates/cad-export-ifc/src/bim_relations.rs` + `tests_relations.rs`：
  只写 `contains`/`nests`，其余 kind 计 `relation_not_written`，端点缺失计
  `relation_endpoint_missing`；无关系导出保持逐字节一致（2196/1374 基线）。
- 证据上限：DGN BIM 解释 `Unverified`；UI 数据来自合成/契约样本；各载体运行时 **NOT VERIFIED**。
- **仍未开始**：Aspect 落盘；`IdKind` 12/13 的启用（D12）。

## 5. 风险与规避

1. **`tables/` 整表重写**：定义表每次保存全量重编码。规避：只放共享定义 + 预算；Phase 2 实测保存
   耗时；超标则 Phase 3 迁到 DefinitionModel 分片元素（minor 7）。
2. **`document_minor` 复用 `FORMAT_MINOR`**：会把 resources-only 文档写成 6。规避：显式三档 +
   单测；定义表只在非空时写。
3. **`PropertyRow` 动态键**：多选交集仍按 key `find`，重复键会静默丢行。规避：key 在一次选择内唯一、
   动态键带 `bim.pset.*` 前缀、名字不一致时整行 `mixed` 且不可编辑（`c87e4046` 已落地并有测试）。
4. **`bim: null` 破坏旧读**：规避：None 省略键，并有 minor-4/5 字节兼容测试。
5. **把源标识当身份**：RVT 文档 GUID 每次保存都变。规避：`document_id+id` 优先、checksum 只作
   provenance、跨保存不自动合并。
6. **文件规模与并发**：`cad-db/src/snapshot/tables.rs`、`ymodel/header.rs`、`cad-store-zip/src/write.rs`、
   `cad-import-rvt/src/glb.rs` 都接近 1000 行上限。规避：新代码全部放新模块，既有文件只做小接线。
7. **第二次保存才暴露的脏分片**：`apply_bim_updates` 自建 `ChangeScope.shards`，漏标一个元素分片时
   第一次保存看起来正常（走 live 分片），第二次保存会把旧字节 raw-copy 回去。规避：写路径必须同时
   标记元素分片，并保留「保存两次字节稳定」的测试。

## 6. 证据边界

本 ADR 的所有映射结论都是**代码契约**，不是真实模型验收：

- IFC 语义导入/数量集/关系只有合成 fixture 与自洽往返证据，**无 vendor 工具交叉验证**；
- RVT 的 BIM 语义建立在 `rvt` 0.25.0 的 `IfcModel` 上，其上游自身声明输出不得用于测量/协调；
  单个真实样本（`rmeadvancedsampleproject.rvt`）的元素/楼层/类型/PSet 计数是**代码契约观察**，
  真实 RVT BIM 语义仍 **NOT VERIFIED**；
- DGN 的 Item Type/tag 语义只有合成 `cadkit_core::Document` 证据，无真实 V8i/Connect 样本的 tag
  解码验证，**NOT VERIFIED**；
- IFC 导出回写只承诺自洽往返，**不承诺**被 Revit/Bentley/IfcOpenShell 接受；**IFC 关系只导出
  `contains`/`nests`**（`IfcRelContainedInSpatialStructure`/`IfcRelNests`），`aggregates`/`group`/
  `connects`/`other:*` 不导出并逐 kind 报告；`BimValue::Text` 导出恒 `Label`；
- BIM 查询/面板与属性编辑：Linux 有运行证据；Windows 仅交叉编译，macOS/Android 仅编译/契约，
  `cad_select_entities` 在这些载体只有源码一致性，**运行时 NOT VERIFIED**；没有查询通道的载体仍按
  `unsupported-runtime: bimQuery` 显式拒绝；
- 所有平台结论仍按 `docs/compatibility.md` 与 AGENTS.md 平台表：只在 Linux 做运行/契约验证，
  Windows/macOS/Android/Web 只能写"仅编译 / 仅契约 / NOT VERIFIED"。
