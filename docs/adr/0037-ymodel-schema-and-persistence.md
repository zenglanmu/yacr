# ADR 0037：ymodel — 唯一原生文档模型与持久化格式

- 状态：**Proposed（2026-11-18，rev.5：持久化章节二次评审后）**。本 ADR 只固定决策、边界与
  验收标准；**尚未实现**，文中不出现"已实现/已验证"结论。
- 日期：2026-11-18。
- 相关：`CAD_IMPLEMENTATION_SPEC.md` §4.3、§4.4、§4.6、§16.3 与新增 §16.5；
  [ADR-0033](0033-multi-format-import.md)、[ADR-0036](0036-3d-body-model-and-solid-service.md)、
  [ADR-0024](0024-dom-canvas-wasm-cad-path.md)、[ADR-0010](0010-react-linux-host.md)；
  `docs/compatibility.md`；`scripts/check-architecture.py`；`fixtures/manifest`。
- 影响：`cad-domain` 增加 schema 描述/身份/`DocumentHeader`/TypedView 契约；`cad-db` 增加
  `DocumentStore` 抽象与 `ChangeScope`；**新增 `cad-store-zip`（纯 Rust，只被 `apps/app-*`
  依赖）**；新增退出码 **8**（仅 CLI/native 映射）；新增连接器与兼容性报告契约。
  **不改变**：patch 禁令（仅 acadrust/rvt）、`cad-db` 唯一权威、载体文档所有权、
  wasm 不链接第三方 C 库。

---

## 0. 摘要

ymodel 是唯一原生文档模型与保存格式（`.ymodel` = **zip 容器 + 分片 JSON + 二进制大对象**）。
概念骨架取自 iModel/BIS（Element / Model / Relationship / Aspect / Code），子类使用 CAD 语义；
DWG/DXF/STEP/SKP/STL 等一律是连接器格式，每次转换都产出**对象级**兼容性报告。

保存模型：**显式保存 + 原子替换 + 恢复快照 + （Phase 2 起）脏分片 raw-copy**。
没有 write-behind、没有库内事务、没有 WAL。崩溃恢复粒度是"上次保存或上次恢复快照"。

与 iModel 的关键差别（需求更强，故有意超出）：

1. iModel 连接器单向，round-trip 只承诺 provenance；本项目要求 L1 语义不动点 + L0 源字节保留。
2. BIS 没有任意源对象的 opaque 通道；ymodel 有（`raw/source-*.bin` + `object_ref`）。
3. BIS 每属性只有一个持久化单位、没有源单位属性，annotation scale 只在 `JsonProperties`；
   ymodel 把单位来源与比例列为一等数据。
4. BIS 一个 Model 只能装 2D 或 3D；DWG model space 混装，ymodel 显式放宽并由几何派生维度。

rev.3–rev.5 的关键点：

- **容器是 zip**，但**不是"一个大 JSON"**：分片 + 逐 entry hash，使脏分片可按压缩字节原样拷贝。
- **磁盘格式由 descriptor 决定**：元素 payload 是 descriptor 字段集的 JSON 对象；
  `SemanticGeometry` 只是它的 Rust 视图。
- **存储句柄不进 `DrawingDatabase`**：它是 `Clone` + `Arc` 快照 + 指针身份模型。
- **id 是 i64 复合键**（8 位 kind + 45 位 seq，≤2^53），且**9 个高水位计数随文档持久化**。
- **无 tombstone**：id 永不复用，悬空引用显式 `Unresolved`。
- **L0 直通必须有源文件字节**，且"未修改"= `revision == manifest.open_revision`；
  `open_revision` 是导入基线，**永不随保存推进**。
- 未知 entry **原样保留**；更新的 minor 版本**拒绝打开**（不做静默降级）。
- **wasm 侧同格式**：Web 与 X11/XWayland 的 `dom-canvas` 都能打开/保存 `.ymodel`。

---

## 1. 问题

### 1.1 现状（事实，含锚点）

- **没有持久化**：`cad-db`/`cad-domain` 声明 `serde`/`serde_json` 但零使用；无 schema 版本、
  无迁移、无 save/load。`DrawingDatabase` 是 39 个 `pub(crate)` 字段的内存结构
  （`crates/cad-db/src/drawing/mod.rs:95`），`#[derive(Clone)]`，内含
  `Arc<RwLock<BoundsCache>>` 供场景线程共享同一 `Arc<DrawingDatabase>`（同文件 `:280`），
  编辑走 `Arc::make_mut`（`crates/cad-app/src/app_history.rs:33`）；
  渲染侧按 `Arc::ptr_eq` 识别文档（`crates/cad-engine/src/render/session_impl.rs:49`）。
- **revision 不可恢复**：`DrawingDatabaseBuilder` 固定 `Revision(0)`（`builder.rs:43`），
  没有加载路径能设定初始 revision。
- **id 分配器从 1 开始且不持久**：`builder.rs:71-78` 的 `next_*_id` 全为 1；
  重开时由 `max(id)+1` 重建（如 `layer_patch.rs:124`），**删除最大 id 后重开会复用该 id**。
  `crates/cad-domain/src/lib.rs:47` 是 u128 newtype；DWG handle 只是调试字符串
  （`crates/cad-db/src/entity.rs:7`）；`DocumentId`/`DatabaseId` 写死 `1`；
  子对象（dimension/hatch/multileader/table/viewport/动态块状态）没有 id。
- **没有类型注册表**：`DbObject.type_key` 是未校验 `String`（`entity.rs:7`），
  语义集中在 `SemanticGeometry` 的 **20** 个变体（`crates/cad-domain/src/semantic.rs:105`）。
- **派生数据在权威行里**：Dimension/MultiLeader/Hatch/Table 内嵌 `display` 向量与冻结样式快照
  （`dimension.rs:444`、`multileader.rs:331`、`hatch_entity.rs:272`）。
- **写校验有"恢复旧行"例外**：`drawing/mod.rs:57-91` 的 `RestoreStored` 允许把数据库自己曾持有
  的行逐字节装回；`point_display_size` 保留原始非有限值（`:171-174`）。
- **变更集推不出脏范围**：`ObjectChange::{Insert,Update,Delete}` 只带 id + 5 bit mask
  （`change.rs:97-101`、`:8`），**不带值、不带表**；图层/布局/样式/线型/头变量写入发布的是
  **空 `changes` 列表 + 提升 revision**（`layer_edit.rs:27` 明确这样写）。
  所以 `revision != saved_revision` 为真时，"哪些行脏"无法从 ChangeSet 推出。
- **单位不进 `DrawingDatabase`**：`UnitContext`/`DocumentIdentity`/`resource_keys` 挂在
  `cad_app::Document`（`crates/cad-app/src/lib.rs:191-197`）。
  已知缺陷：`$INSUNITS=5`（厘米）映射为"米 + 0.01"（open B31）。
- **xref 只有元数据**；`ImportRequest::xref` 是 DWG-only。
- 已有 `cad-import-core` 与 11 个格式 crate，但派发是硬编码 `match`。
- 已有兼容性词汇可复用：`SupportStatus`/`EntityCapability`/`Completeness`、
  `ImportReport`、`ExportReport`；**判定不落盘**。
- `cad_platform::Persistence` 已声明（recovery 语义，含 `discard_recovery_after_confirmation`）
  但**从未实现**（`crates/cad-platform/src/lib.rs:86-93`）。
- spec §16.3 已要求版本化、迁移、未知字段不静默丢弃、单位与坐标随几何持久化。

### 1.2 需求

ymodel 为唯一权威数据源；是 DWG/DXF 的可验证超集；其它格式经 connector 双向转换并出对象级
兼容性报告（UI 可点击打开）；承载单位、比例、图层/块/样式/xref 等 schema；设计参考 iModel/BIS。

### 1.3 约束

`check-architecture.py` 分层（`cad-domain` 最低）；单文件 ≤1000 行；`wasm32-unknown-unknown`
必须可编译；只有 `acadrust`/`rvt` 可 patch；i18n 目录唯一；证据诚实。

---

## 2. 决策

### D1 分层、store 归属与对外契约

- `cad-domain`：纯数据（ID、单位、语义几何、class descriptor、`DocumentHeader`、TypedView、
  对象模型类型），不依赖持久化库。
- `cad-db`：`DrawingDatabase` facade、验证/事务/ChangeSet、**`ChangeScope`**、
  **`DocumentStore` trait**。不依赖 zip。
- **新增 `cad-store-zip`**：纯 Rust（`zip` + `serde_json` + `sha2`），实现 `DocumentStore`。
  只允许 `apps/app-*` 依赖；`cad-*` 共享层禁止依赖（`check-architecture.py` 增加断言）。
- **存储句柄不进入 `DrawingDatabase`**：句柄属于宿主 `Document`/session。契约测试加
  `fn _assert_send_sync<T: Send + Sync>()`。
- **`DocumentHeader` 在 `cad-domain`/`cad-db`**：承载 `UnitContext`、`DocumentIdentity`、
  `resource_keys` 与 9 个 id 高水位计数；`cad_app::Document` 持有它，store 只序列化它。
  （`cad-store-zip` 不得依赖 `cad-app`。）
- 导入仍经 Builder；写入仍走同一验证/变更路径。
- Tauri command/wire **不破坏**；`api/plugin-api.json` 只许追加字段。
- 复用 `cad_platform::Persistence` 做自动恢复快照。

### D2 保真契约：L0 / L1 / L3（L2 暂缓）

| 级别 | 含义 | 承诺 |
|---|---|---|
| **L0** | 源对象被保留：`raw/source-*.bin` + `object_ref` | `revision == manifest.open_revision && source_retained` 时同格式导出 = 原字节 |
| **L1** | 已建模语义往返相等 | **不动点**：`ymodel → 格式 → ymodel` 语义相等 |
| **L3** | 视觉/像素等同 | **永不承诺** |

- **两个 revision 必须分开**：`manifest.open_revision` = 导入时的基线，**永不推进**；
  `manifest.saved_revision` = 最后一次保存的状态。加载时 `revision := saved_revision`、
  `saved_revision := saved_revision`。`DrawingDatabaseBuilder` 必须能设定初始 revision。
  **没有这条，编辑后保存再打开会重新满足 L0 谓词并伪造"原字节还原"**。
- "未修改"是机械谓词（见上），任何 commit（改图层名、改样式、改头变量）都使直通失效。
- L0 依赖源文件字节（STORED）。`--no-retain-source` 时 `tables/sources.json` 记
  `source_retained: false`，直通退化为"路径 + sha256 可读"，不可用则 `export.l0_unavailable`。
- **明确非目标**：任意 DWG 对象修改后的完美回写。导不出的 L0 对象必须进不兼容清单。
- DXF 文本级拼接是 L0 增量的首选路径，依赖 acadrust 暴露源 span，Phase 3 验证后再承诺。

### D3 原生格式与另存为

- `.ymodel` 是唯一原生保存格式；其余全部是连接器格式。
- 现有 DWG/DXF 另存为保留（DWG 仍 experimental/lossy），由 ymodel 投影，产出对象级
  `ExportReport` 落入 `tables/compat.json`。
- 新导出器必须走同一契约与同一报告模型。
- 不允许静默降级。
- 打开 DWG/DXF **不会**在原目录自动生成 `.ymodel`；导入结果是未命名文档，由用户另存为。

### D4 对象模型：BIS 骨架 + CAD 子类

- 五基元：Element / Model / Relationship / Aspect / Code。
- `cad:GraphicElement` → Line/Polyline/Circle/Arc/Ellipse/Spline/Point/Text/MText/Dimension/
  Hatch/Leader/MultiLeader/Table/RasterImage/Shape/Mesh/Solid3d/Insert/Opaque；
  `cad:DefinitionElement` → TextStyle/DimensionStyle/MLeaderStyle/LineType/BlockDefinition/
  PlotStyle/TableStyle；`cad:InformationElement` → 文档头、命名视图、图层状态、组。
- **放宽 BIS 维度不变量**：`cad:ModelSpace`/`cad:BlockModel` 允许混合 2D/3D；
  `cad:PaperSpace` 仍是 2D。**元素维度由几何在写入时派生**，不落冗余标记。
- Aspects：`cad:RawSource`（multi）、`cad:UnitProvenance`（unique）。
  `cad:Xdata`/`cad:VendorRecord` 推迟。
- `SemanticGeometry` 的 20 个变体不删除：它是定义的 Rust 视图，不是磁盘格式。

### D5 身份与溯源

- **持久 id 是 i64 复合键**：`(kind << 45) | seq`。kind 8 位（≤255），seq 45 位，
  整体 ≤ 2^53 —— JSON 与 `jq`（f64 解析）也能精确显示；超出 2^53 的标识性整数写成字符串。
- **9 个 id 高水位计数随文档持久化**（`document.json` 的 `identity.next_*`），加载时原样恢复，
  并断言 `>= 1` 且 `<= 2^45`。**禁止**用 `max(id)+1` 重建分配器：删除最大 id 后重开再插入
  会复用已删除的 id，违反"永不复用"，并让分片规则失效。
  这 9 个是 entity / block / layer / layout / style / dim-style / mleader-style / linetype /
  scale。今天的代码只有 8 个分配器：**线型没有分配器**，`linetype_edit.rs:258-277` 每次创建都
  按 `max(现有行 id)+1` 现算，所以"创建→删除→再创建"今天就会复用 id。实现阶段必须补
  `next_linetype_id` 字段与迁移，不能把现算逻辑写进保存路径。
- **不存 tombstone**：删除只让 id 消失；悬空引用在读取时是显式 `Unresolved`。
  （避免 tombstone 表 O(deletes) 增长，也避免每次删除都重写一个 entry。）
- `FederationGuid`：每文档唯一，导入时分配；另存为保留元素 GUID，生成新的 `document_uuid`。
  v1 无联邦消费者（用户决策保留）。
- `SourceRef` multi-aspect：`source_id`、`identifier`、`kind`、`checksum`；
  唯一索引 `(source_id, identifier)`；映射表是连接器必需输出。
- `EntityId`/`ObjectId` 继续是同一数值空间。

### D6 关系与 Code

- v1 关系：`ModelContainsElement`、`ElementOwnsChildElements`、`ElementRefersToElement`、
  `GeometricElementIsInSubCategory`。
- layer/style/linetype 引用用**属性引用**，不升格为关系。
- `Code` 唯一性是"带诊断的唯一"：真实 DWG 存在大小写折叠后重名，保留数据、按 `Model`
  作用域消歧并记 `code.duplicate`，绝不自动改名。

### D7 Model / Space / Category / Block

- Model 是独立实体；`ModelModelsElement` 指向它描述的元素。
- model space → `cad:ModelSpace`；每个 paper layout → `cad:PaperSpace`；
  block definition → `cad:BlockDefinitionElement`（住文档级 `cad:DefinitionModel`）
  + 一个 `cad:BlockModel`；样式/Category/SubCategory 也住 `DefinitionModel`。
  `INSERT` 引用定义，不复制成员。
- 每元素恰属一个 Model；父子同 Model 且无环；图形元素恰属一个 SubCategory。
- 每文档一个 `cad:LayerCategory`；每个 DWG layer = 一个 SubCategory 元素（1:1）；
  每条 xref 一个带前缀的子 Category。
- **v1 存储偏差（显式记录，不是静默降级）**：对象模型里样式/图层/线型/比例/布局都是
  DefinitionElement（语义上住 `DefinitionModel`），但 v1 把它们的**行**存进
  `tables/*.json`（含块定义行：`tables/blocks.json`），而不是 `elements/<model>-<shard>.json`。理由：① 改一行只需重写一个小
  entry，而放进 `Definitions` 分片会把整片（含全部块定义）一起重写，raw-copy 粒度更差；
  ② 这些行的 payload 编解码是 Phase 2+ 的工作量，先做会把首个可用的保存路径推后。
  `ChangeScope::TableKind` 是这份偏差的显式载体——新增行表必须同时加进该枚举与 store 的
  表分派，否则该表不会被保存。迁移到 DefinitionElement 需要升 minor 并写迁移器；在此之前，
  SubCategory 归属由元素的 `layer` 字段派生，`LayerCategory` 只作为对象模型概念存在、不落盘。

### D8 几何三点，以及变更范围

- Definition：权威（参数、样式引用、layer 属性），存元素 payload。
- Geometry：由 definition 生成，不落盘；打开时重建即正确性测试。
- Presentation：概念保留，v1 不持久化缓存。
- 现有内嵌 `display` 字段：Phase 1 保持原样，只有替代缓存能给出逐字节一致的 accessor 输出时
  才降级（Phase 2+）。
- **`ChangeScope` 是内存对象，不落盘**（与 rev.2 的"落盘"不同）：
  `ChangeScope { shards: BTreeSet<(ModelId, Shard)>, prev_shards: BTreeSet<(ModelId, Shard)>,
  tables: TableSet, header: bool }`。由**每一个写入路径**填充（包括发布空 `changes` 的
  图层/布局/样式/线型/头变量写入），session 累积。它是"哪些 entry 脏"的唯一依据。
  **不放进 `DrawingDatabase`**（它 `Clone`、`Arc` 共享、`Arc::make_mut`，批次会在克隆时分叉）。
- `ChangeMask` v1 保持 5 bit（内存通知用）；文件不存 mask。

### D9 单位、比例、坐标与数值

- 规范单位：毫米 + 弧度；`KindOfQuantity` 覆盖 length/angle/area/volume/scale/unitless。
- 每源 `UnitProvenance`：源单位 id、精确换算、可得的原始数值文本。
- **先修 B31** 再允许写 `.ymodel`；未声明单位保持"图纸单位"。
- 比例一等：annotation scale、每 layout 的 plot scale/paper units、model/paper 划分。
- 地理配准独立以米 + CRS 存。
- **数值编码**：JSON；f64 最短往返（逐位往返属性测试）。JSON 无 NaN/Inf，用文档化转义
  `{"$f64":"nan"|"inf"|"-inf"}`；读取端拒绝未知转义值。新写入拒绝非有限几何；
  **恢复旧行**（`RestoreStored`）按原值转义保存，绝不因文件里有 NaN 而拒绝保存。
- 标识性整数 ≤2^53；超出写字符串。

### D10 schema 与 class 注册表

- 仓库内版本化 descriptor（JSON，`include_str!`）+ 编译期校验 + 保存时物化进
  `schema/classes.json`。不做 EC 解释器/反射。
- 磁盘格式由 descriptor 决定：元素 payload 是 descriptor 字段集的 JSON 对象；
  Rust 类型是视图。加字段 = 升 minor；改类型/删字段 = 升版本 + 迁移。
  descriptor 与 payload 字段集由测试逐项比对。
- 长尾对象用 descriptor + `TypedView`；v1 手写 TypedView。
- dynamic class 机制保留、**注册推迟到 Phase 4**；Phase 1–3 未知对象走 `cad:Opaque` +
  `object_ref` + `class.undeclared`。
- 未声明 class → 通用属性 + raw 引用 + 报告；不拒绝打开、不猜语义。

### D11 容器：zip + 分片 JSON + 二进制大对象

```text
manifest.json                 -- format{read,write,minor}, generator, document_uuid,
                                 open_revision, saved_revision, saved_at,
                                 shard_map: {model_id: [entry...]},
                                 entries[{path, sha256, sha256_c, len, crc, foreign}]
document.json                 -- DocumentHeader: 单位/身份/resource_keys + identity.next_*（9 个）
schema/classes.json
tables/models.json            -- Model 元数据（kind、所属 layout/block、名称）
tables/layers.json | blocks.json | layouts.json | styles.json |
       dimstyles.json | mleaderstyles.json | linetypes.json | scales.json |
       plot-settings.json | xrefs.json      -- v1 行表（见 D7 存储偏差）
tables/sources.json | object-refs.json | compat.json   -- Phase 2+
tables/relationships.json | aspects.json               -- Phase 3+
elements/<model>-<shard>.json -- 每分片 2048 元素，按 id 排序
raw/source-<id>.bin           -- 保留的原始源文件，STORED
```

- **分片规则是格式契约**：`shard = seq / 2048`（同一 Model 内），元素按 id 排序写入。
  元素 id 永不复用，所以只有"换 Model"会让元素换分片（旧分片删除 + 新分片插入）。
- **entry 集合由 live 数据决定，不是"旧包里没改的都留"**：
  `new_entries = live_shards ∪ dirty_shards ∪ tables_present ∪ foreign_entries`。
  raw-copy 只复制 `旧 manifest.shard_map` 中**仍存在**的 entry。被删除的 layout/block/model
  的旧分片不会被复制回来（否则重开时"复活"）。
- **manifest 自身不在 `entries` 里**（自引用无定义）；它的完整性由 zip CRC 保护。
- **两个 hash**：`sha256` 是解压后字节（格式契约）；`sha256_c` 是压缩字节。
  **实测约束（rev.8）**：`zip` 9.5.0 不公开压缩字节（`take_raw_reader` 是 crate-private），
  所以本仓库既算不出也验不了 `sha256_c`。Phase 1 对**每个** entry 校验解压后的 `sha256`
  + `len` + zip 中央目录 CRC（对内容完整性严格更强，且覆盖 foreign entry）；`sha256_c` 只作为
  拷贝 entry 的原始记录从旧 manifest 透传，读侧标注"未校验"。Phase 2 要么换 zip API/自写
  容器，要么按 minor 迁移删掉该字段——**不得**把它写成已验证。
- **压缩**：JSON entry 用 deflate；`raw/*.bin` STORED。
- **未知 entry**：`foreign: true`，保存时按压缩字节 raw-copy，永不解析、永不丢弃；
  **全量重写/compaction 也必须复制它们**，不可读则 `ymodel.foreign_entries_unreadable`。
  foreign entry 与保留路径冲突 → 显式失败，不覆盖。
- **更新的 minor 版本拒绝打开**（`ymodel.unsupported_minor`）：本写入器无法在重写分片时
  保留未知字段，读进来再存出去就是静默丢数据。只读模式作为后续选项，不在 v1。
- 一个文件一个文档；id/GUID 唯一性按文件计。
- `document.json` 放文档数据；`manifest.json` 只放格式元数据。

### D12 保存语义：显式保存 + 原子替换 + 恢复快照

- **显式保存**（Ctrl+S / 另存为）。没有 write-behind，没有库内事务。
  `dirty = revision != saved_revision`；dirty 的**范围**来自 `ChangeScope`，不是 ChangeSet。
- **保存算法**：
  1. 从 `ChangeScope` 取脏分片/脏表/header；计算 live entry 集合（D11）。
  2. 在同目录写**唯一**临时文件 `foo.ymodel.<pid>-<nonce>.tmp`：未修改且仍存在的 entry
     raw-copy（Phase 2 起；Phase 1 全量重写）；脏 entry 重写；`manifest.json` 最后写。
  3. fsync；重开校验 manifest 与每个 entry 的 `sha256` + `len` + CRC。被拷贝的 foreign entry
     同样走解压后校验（`sha256_c` 在 `zip` 9.5.0 上不可验，见 D11）。
  4. 原子替换：先 `link(target → target.bak)`（无硬链接则复制），再
     `rename(tmp → target)`；Linux/macOS 加目录 fsync；Windows `ReplaceFileW`。
     **绝不**先 `rename(target → bak)`（会留下"目标不存在"的窗口）。
  5. 成功后 `saved_revision := revision`；清 dirty；**清掉或替换恢复快照**（见下）。
  6. 清理孤儿 `.tmp`（打开时扫描本目录）。
- **恢复快照生命周期**：成功保存后清掉旧快照；启动时只有
  `snapshot.revision > file.saved_revision` 才提示恢复。否则会用旧快照把文件"回滚"。
- **崩溃窗口**：上次保存/恢复快照之后的编辑。
- **撤销**：会话内；undo/redo 照常提升 revision（回到保存点仍是 dirty，安全方向）。
  **`File > Revert` = 从磁盘重新加载**，不是反向应用撤销栈（后者清不干净 dirty）。
- **多实例（无锁文件）**：打开时记下目标 `(sha256, mtime)`；保存前重算，不一致 →
  显式拒绝并建议"另存为"。Web/SAF 本来就不允许旁路文件，所以不引入 lockfile。
- **Web 的"已保存"**：FSA `createWritable` 必须 `await writer.close()` 后才清 dirty；
  下载降级时清 dirty 但 UI 显示"已下载副本"。多标签 FSA：保存前重读
  `lastModified + sha256` 比对。
- **X11/XWayland 的 wasm 文档**：宿主写完并回 ACK `{saved_revision, target_sha256,
  target_len}` 后才清 dirty；禁止 fire-and-forget。

### D13 版本与迁移

- `manifest.format{read,write,minor}`；语义 `Read.Write.Minor`。
- 更高 read → 拒绝（`ymodel.unsupported_version`）；更高 minor → 拒绝
  （`ymodel.unsupported_minor`）。**退出码 8 只是 CLI / `native-below` 的映射**；
  wasm/浏览器没有退出码，稳定标记才是契约。
- 迁移是确定性链 + 历史 `.ymodel` 样本；未知 entry 原样保留。
- 另存为 = 新 `document_uuid`、保留元素 GUID、新 `database_id`。
- 打开视为不可信输入：版本 + manifest/schema 校验 + entry hash/尺寸上限。

### D14 连接器契约与兼容性报告

- `Connector` descriptor（id、格式、方向、版本范围、能力、限额、`declared_classes`）；
  派发改为注册表，`FormatImporter` 保留为薄封装。
- 复用 `SupportStatus`/`EntityCapability`/`Completeness`，不造第二套。
- 报告对象级、双向（import/export/roundtrip），`level ∈ {L0,L1}`，落 `tables/compat.json`，
  可导 JSON 与 wire。
- 新 code 与新属性走 i18n 目录与现有 `PropertyRow` 路径。
- UI：可点击条目 → 报告弹窗。
- 顺带修：`import.unknown_entity` 从不发射；`completeness_label()` 硬编码中文。

### D15 xref

- 默认外链（路径 + hash + 状态）；`--embed-xref` 嵌入独立 Model。v1 不做跨文件联邦。

### D16 载体

文档所有权与字节源/汇分开：store 是纯 Rust，**谁持有 `cad-db` 谁就能持久化**。

| 载体 | 文档所有者 | 打开 | 保存 |
|---|---|---|---|
| Linux Wayland / Windows / macOS | native | 宿主文件 | 临时文件 + 原子替换 + `.bak` |
| Android | native | **SAF 流读入应用私有缓存**（zip 读需要 Seek，不能直接流式打开） | 临时文档 + `DocumentsContract` 替换 + 私有 `.bak`（非原子） |
| Linux X11/XWayland | wasm | Tauri 对话框，宿主读字节 | 宿主写临时文件 + 替换 + 回 ACK |
| Web（FSA 可用） | wasm | 文件选择器，整份读入 | FSA 原地写回，`close()` 后清 dirty |
| Web（无 FSA） | wasm | 同上 | 下载副本；UI 明示，不冒充"已保存" |

- **wasm 内存预算**：sink 必须**分块且可中止**（`begin/write_chunk/abort/commit`），
  上限按**峰值**（累计写入 + 源文件长度 + 文档内存）计，超过 **256 MB**（Phase 3 实测后
  调整）以 `web.persistence_too_large` 中途 abort；不允许先分配整块缓冲再检查。
- **未保存保护**：wasm-owned 文档在 `beforeunload` 拦截 dirty 关闭/刷新；
  v1 不做浏览器恢复快照（IndexedDB/OPFS 后续另立 ADR）。
- 编译门禁：`cargo check -p app-web --target wasm32-unknown-unknown --locked`。

### D17 迁移路线与验收门槛

1. **Phase 0（文档）**：本 ADR + spec §16.5。
2. **Phase 1**：descriptor + 身份（含 9 个高水位持久化）+ `DocumentHeader` + `ChangeScope` +
   `DocumentStore` + `cad-store-zip`（**全量重写**、manifest 校验、原子替换、`.bak`、
   revision 种子、恢复快照）。交付物：**"39 + 5 个字段 + 20 个变体 → entry/字段"清单**
   （`docs/ymodel-phase1-inventory.md`）；
   `Send + Sync` 断言；打开/保存/重开契约测试。
3. **Phase 2**：DXF 导入；DXF 导出 + L1 不动点；报告落盘；**脏分片 raw-copy +
   `sha256_c` 校验 + live entry 集合**；撤销/恢复快照生命周期；冲突检测。
4. **Phase 3**：DWG 导入 + `raw/source` + L0 直通；单位/比例；兼容性弹窗；
   acadrust 源 span 可行性验证；Web 保存实测。
5. **Phase 4**：dynamic class 注册 + STEP/SKP mesh 连接器；RVT 不变。

性能门槛：open ≤ 2× 今天 parse-to-db；Phase 1 全量保存 100k 元素 ≤ 3s；
Phase 2 raw-copy 保存 200 MB 文件 ≤ 1s（SSD）；保存峰值磁盘 ≤ 2× 文件大小
（**不含恢复快照**；`.bak` 用硬链接计 0 额外字节）；`.ymodel` ≤ 1.5× 等价 DXF 文本；
常驻内存 ≤ 2× 今天 `DrawingDatabase`。数字先测量后收紧。

---

## 3. 备选与权衡

### 3.1 决策历史与最终选择（JSON+zip）

1. **rev.1**：JSON+zip（纯文本可查看）。
2. **rev.2**：改 SQLite（事务、WAL、增量提交）。
3. **rev.3–rev.5（当前）**：回到 JSON+zip，前提变为**显式保存 + 原子替换 + 恢复快照**，
   并引入（Phase 2）脏分片 raw-copy。

| 判准 | JSON+zip（本方案） | SQLite |
|---|---|---|
| 保存粒度 | 显式；崩溃丢"上次保存/快照之后" | write-behind；丢"一条命令" |
| 保存代价 | 全量 O(doc)；raw-copy 后 CPU O(脏分片)、I/O O(文件) | 增量 O(change) |
| 网络盘 | 单文件替换（rename 原子性不保证，靠 `.bak`+hash） | WAL 明确不可用 |
| Android SAF | 读需 seekable（要 copy-in 缓存），写可流式 | 需 copy-in + 侧车 |
| 可检查性 | `unzip -p` + `jq` | `sqlite3` |
| wasm | 纯 Rust，可编译 | C 依赖，不可 |
| 依赖/构建 | 纯 Rust | C 工具链、NDK 交叉编译 |
| 事务语义 | 无（靠原子替换） | 有 |
| 未知数据保留 | entry 级 raw-copy | 需"未知行永不重写"的纪律 |

结论：在**显式保存**这个产品模型下，SQLite 的增量提交优势失去使用场景，而 C 依赖、网络盘限制、
SAF copy-in 仍在。JSON+zip 的 O(doc) 保存被 raw-copy 压到"O(脏分片) CPU + O(文件) I/O"。
**若将来要求"每条命令持久"**，需要回到变更日志/数据库方案，是另一次 ADR 与迁移。

### 3.2 其它备选（拒绝）

- 单个大 JSON + zip：失去分片 raw-copy 与懒加载。
- append-only zip：I/O O(脏)，但产生未引用字节、需要 compaction、崩溃恢复复杂；不进 v1。
- 自定义 append-only 二进制：工具链/迁移/调试全自担。
- SQLite 作为 working set：摧毁借用/快照语义与 357 个 representation 测试。
- 通用 EC 解释器：工程量数倍，与编译期校验冲突。
- 照搬 BIS 分类：`PhysicalElement` 表示 2D 直线是错误映射。

---

## 4. 依赖与许可

- `zip`（MIT）、`serde_json`（MIT/Apache-2.0）、`sha2`（MIT/Apache-2.0）、
  `flate2`/`miniz_oxide`（纯 Rust 路径）。全部纯 Rust，可编 wasm。
  新增依赖记入 `THIRD_PARTY_NOTICES.md`。
- 不新增 `[patch.crates-io]` 键；无 C 工具链依赖。

---

## 5. 测试与证据

- **revision / L0**：导入 → 改图层名 → 保存 → 重开 → 断言 L0 不可用（B1 回归）。
- **id 高水位**：删除最大 id → 保存 → 重开 → 插入 → 断言不复用；分片不串。
- **脏范围**：建图层 + 改名（发布空 `changes`）→ 保存 → 重开 → 名称仍在（B3 回归）。
- **entry 集合**：删 layout/block/model → 保存 → 重开 → 旧分片不复活（B4 回归）。
- **unknown**：更高 minor 拒绝打开；foreign entry 在首次保存与 compaction 后仍在；
  foreign 路径冲突显式失败（B5 回归）。
- **raw-copy**（Phase 2）：未修改 entry 的 `sha256_c` 逐字节不变；脏分片重写；
  `sha256_c` 校验能发现短拷贝；manifest 不在自身 entries。
- **数值**：f64 逐位往返；`$f64` 转义往返；新写入拒绝非有限值；恢复旧行（含非有限 header）
  能保存并读回；id ≤2^53 在 `jq` 中精确。
- **崩溃/替换**：临时文件写完前 kill -9 → 旧文件完好；替换前 kill -9 → 旧或新；
  `.bak` 可恢复；孤儿 `.tmp` 被清理。
- **快照生命周期**：保存后旧快照清除；`snapshot.revision <= saved_revision` 不提示恢复。
- **冲突**：保存前文件被外部改动 → 拒绝并提示另存为。
- **载体**：wasm 编译通过；Web 下载路径产出可独立打开的 `.ymodel`；chunked sink 在超限时
  中止（mock sink）；`beforeunload` 拦截；FSA `close()` 后才清 dirty（adapter 契约测试）；
  X11 宿主 ACK 前不清 dirty。浏览器运行时、真实写盘、跨浏览器差异 **NOT RUN**。
- **NOT RUN**：真实 DWG 不动点、性能数字、真机/GPU/像素、Windows/macOS/Android 运行。

---

## 6. 后果、风险与跟进

正面：格式可检查；纯 Rust、无 C 工具链；网络盘与 SAF 可用；未知数据 entry 级保留；
raw-copy 把保存压到接近增量；Web 与 X11 的 `dom-canvas` 都能原生保存。

成本与风险：

- **显式保存**：崩溃丢"上次保存/快照之后"的编辑；必须写进用户文档。
- 首次保存与 compaction 是 O(doc)；大文件峰值磁盘 2×（`.bak` 用硬链接可降到 2× 以内）。
- 原子替换依赖平台 API；Android SAF 明确非原子；Web 无 FSA 时只能下载副本。
- 浏览器：256 MB 峰值上限、FSA 仅 Chromium、v1 无恢复快照。
- `ChangeScope` 必须覆盖**每一个**写入路径；漏一条就是"保存成功但没写"。
- 更新的 minor 版本拒绝打开（牺牲前向兼容换"绝不静默丢字段"）。
- 超集受写入器限制；RVT/SKP/STEP 几何上限不变（ADR-0033）。
- 默认 xref 外链 → 单文件不总自包含；全量 hydration 内存未测。
- **三处保留的用户决策，代价已知**：`FederationGuid`；layer 1:1 SubCategory；
  dynamic class 机制（Phase 4）。

跟进项：

1. spec §16.5 与 §4.3/§4.4 交叉引用（同步完成）。
2. 实现阶段更新 `AGENTS.md`：ymodel 原生格式、退出码 8（仅 CLI/native）、Web 平台行、
   保存语义。
3. `docs/compatibility.md` 增加 ymodel 保真等级与按格式的 L0/L1 清单。
4. 修 B31、`import.unknown_entity` 死代码、`completeness_label()`；复用
   `cad_platform::Persistence` 作恢复快照。
5. append-only zip / 增量日志 / Web 恢复快照 / 跨文件联邦：各自独立 ADR。
