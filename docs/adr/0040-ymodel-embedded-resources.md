# ADR 0040：ymodel 外部资源打包（自包含单文件）

- 状态：**Accepted（2026-11-20，rev.3）**。**2026-11-20 已实现**：格式核心（`cad-domain`/`cad-db`）、
  归档（`cad-store-zip`）、宿主采集与两条策略命令（`cad-host`/`cad-app`）、字体目录设置与目录
  选择器、React 保存对话框与选项行均已落地；门禁与测试证据见
  `docs/validation-resources-2026-11-20.md`（真实跨机、字体度量、portal 交互、真机/GPU 一律
  **NOT RUN**）。rev.3 把三态 `ResourcePolicy` 收敛为布尔
  `embed_resources`（见 D3）。rev.2 采纳独立架构复核的 9 条修正，其中 3 条为
  BLOCKER：`tables/resources.json` 必须是 JSON **数组**（原稿的对象外壳会让带资源的文档
  **完全打不开**）、`is_modelled` 前缀匹配方向搞反（会把未解码的条目当已建模，下次保存静默丢字节）、
  恢复快照并不存在 `with_previous`（原稿的"成本接近零"是错的）。
- 日期：2026-11-20。
- 相关：[ADR-0037](0037-ymodel-schema-and-persistence.md)（容器/分片/FORMAT_MINOR/`ChangeScope`）、
  [ADR-0038](0038-mesh-base-color-texture.md)（`textures/<key>` 二进制条目先例）、
  [ADR-0039](0039-mesh-pbr-channels.md)（minor 4）、
  `docs/xref.md`、`docs/fonts.md`、`docs/compatibility.md`。
- 影响：`cad-domain`（`resource_policy`、`BundledKind`、键校验）、`cad-db`（资源字节表、
  `tables/resources.json`、快照）、`cad-store-zip`（`resources/<kind>/<key>` 条目、raw-copy 资格、
  逐条校验与 per-document minor）、`cad-platform`（字体/图片解析优先取文档内嵌）、
  `cad-host`（保存时采集与缺失报告、恢复快照策略）、`apps/desktop-ui`（保存对话框开关与清单）、
  i18n。**`FORMAT_MINOR` 上限 4 → 5**（写入按文档取 4 或 5，见 D12）。
  **不改变**：`cad-db` 唯一权威、载体文档所有权、patch 禁令（仍只有 acadrust/rvt）、
  `raw/` 语义（仍是"不认识就原样保留"，本构建不写入）、载体接线范围（本 ADR 当时只有
  Linux 接线 `.ymodel`；后续载体接线见 ADR-0060 及 `docs/compatibility.md`）。

---

## 0. 摘要

现状（2026-11-20）：`.ymodel` 里**只有网格贴图**是真自包含的——字节在 `cad-db`，随归档落在
`textures/<key>`，键为 `<sha256 前 32 hex>.png|jpg`。其余三类资源都只有字符串：

| 资源 | 现在存什么 | 字节来源 |
|---|---|---|
| 外部参照 | `tables/xrefs.json` 的 `raw_path`/`resolved_path`；导入时 `splice_xref` 已把几何**烘焙**进块 | 无（源文件字节从不保留） |
| 字体 | `Style.resource_keys`、实体 `font`、`header.resource_keys` 里的**名字** | 无（宿主字体目录 / 系统字体 / 内置目录） |
| 栅格 `IMAGE` | `cad:Image.file` 逻辑键（可能来自源图纸的绝对路径） | 无（运行时按图纸目录/`images/` 解析磁盘） |

结果是：`.ymodel` 单文件跨机打开会丢字体、丢光栅图，外部参照也无法在换机后重新导入。
本 ADR 加一条**可选**通道：把这三类资源的字节按需塞进同一个 zip，使一个文件自包含。

设计对齐 docx/OPC 的分工，但**不新增顶层清单文件**——`manifest.json` 已经承担
`[Content_Types].xml` + `_rels` 的角色（每个 entry 的 `sha256`/`len`/`crc`、严格拒绝未知字段），
`document.json` 就是"主 JSON"：

```
manifest.json              目录 + 每 entry sha256/len/crc（format.minor 按文档取 4 或 5）
document.json              主 JSON：DocumentHeader（policy 非 None 时才写 resource_policy）
schema/classes.json
tables/*.json
tables/resources.json      新增：逻辑名 ↔ 归档键 映射（JSON 数组，D4）
elements/*.json
textures/<key>             既有：网格贴图字节，命名不动（minor ≤ 4 仍可读）
resources/font/<key>       新增
resources/xref/<key>       新增
resources/image/<key>      新增
raw/*                      外来条目直通，语义不变
```

---

## 1. 决策

### D1 — 三类统一为"文档资源"，键内容寻址

`resources/<kind>/<key>`，`key = <sha256 前 32 hex>.<ext>`，与 `textures/<key>` 同构：
同字节只存一份，键里不含任何路径成分，因此没有 zip-slip 面。

`kind` 与 `ext` 白名单（大小写不敏感地**拒绝**，不做补全）：

| kind | 允许扩展名 |
|---|---|
| `font` | `shx`, `ttf`, `otf`, `ttc` |
| `xref` | `dwg`, `dxf` |
| `image` | `png`, `jpg`, `jpeg`, `tif`, `tiff`, `bmp`, `webp`, `gif` |

不认识的扩展名 → 写入/采集侧 `resource.unsupported_kind`（显式拒绝，不猜媒体类型）。

**键必须与内容一致**：写入路径断言 `sha256(bytes)` 的前 32 hex（小写）**等于**键的前半段。
这条不变量不能下放到 `cad-db`（那里不引入哈希依赖），由 `cad-store-zip` 在已经算 `sha256`
的那一处断言。没有它，采集方算错键时 raw-copy 会**静默保留旧字节**——这才是真正的损坏路径。

### D2 — `BundledKind` 放 `cad-domain`，不复用 `cad-resources::ResourceKind`

```rust
// crates/cad-domain/src/resource.rs（新）
pub enum BundledKind { Font, Xref, Image }
pub fn is_valid_bundled_key(kind: BundledKind, key: &str) -> bool;
```

理由：`cad-db`/`cad-domain` 不能反向依赖 `cad-resources`（分层），而
`cad_resources::ResourceKind` 还带 `resolve/decode` 能力语义。`cad-platform`/`cad-host`
在边界上做匹配，不引入两套同义枚举的持久化。

### D3 — 是否自包含是文档属性：布尔 `embed_resources`，仅 `true` 时写

`DocumentHeader` 增加 `embed_resources: bool`，序列化时**仅当 `true` 才写**
`"embed_resources": true`；缺失 → `false`（唯一允许的缺省，供 minor ≤ 4 与普通文档使用）。
present-but-non-bool → 结构化错误（`field: "embed_resources"`）。

理由：`false` 是"没有自包含意图"，与"字段不存在"语义等价；不写字段才能让无资源的文档
保持 minor 4 字节级兼容（D12）。**缺省必须是 `false`**：本轮之前的文档全是 `false`。

刻意**不做**三态（`None`/`All`/`Selected`）：保存对话框就是一个"自包含"开关，逐条勾选
属于未来扩展；先造一个没人会写入的 `Selected` 变体等于制造死状态（AGENTS 铁律 2）。

### D4 — 逻辑映射表 `tables/resources.json` 是**数组**，不是对象

**这是硬约束**：`DocumentSnapshot.tables` 的行类型是 `Vec<serde_json::Value>`，
`cad-store-zip` 写 `Body::Table(&[Value])`、读侧硬要求 `Value::Array(rows)`。表外壳写成
`{version, entries}` 会让归档在读取时直接 `ymodel.invalid_document`。因此：

```json
[
  { "key": "9f3a…c1.ttf", "kind": "font", "name": "arial.ttf",
    "source": "C:/proj/fonts/arial.ttf" },
  { "key": "1b07…e4.dwg", "kind": "xref", "name": "STRUCT" }
]
```

- 无 `version` 字段：表版本由 `manifest.format.minor` 统一表达，不在行内再开一套。
- `source` **可选**：允许**缺失**与 `null` 两种写法，解码等价。
- `name` 是逻辑引用名，必须与被引用处逐字一致：字体取 `Style.resource_keys`/`Text.font`，
  `IMAGE` 取 `cad:Image.file`，外部参照取 `ExternalReference.name`。
- `source` 仅供诊断/重新解析提示，**不参与**运行时解析，不保证可读。
- 不重复存 `sha256`/`bytes`：完整性唯一来源是 `manifest.json`。
- 同名多键（同名字体两个不同字节）按 `(kind, name)` 拒绝：`resource.name_conflict`。
- 该表是**派生索引**：名字对不上引用时就永不命中（无害），不做读取期全量交叉校验。

### D5 — 字节进 `cad-db`，与 `textures` 同构

```rust
impl DrawingDatabase {
    pub fn resource(&self, kind: BundledKind, key: &str) -> Option<&Arc<[u8]>>;
    pub fn resources(&self) -> impl Iterator<Item = (BundledKind, &str, &Arc<[u8]>)>;
    pub fn resource_name(&self, kind: BundledKind, key: &str) -> Option<&str>;
    pub fn resource_key_for_name(&self, kind: BundledKind, name: &str) -> Option<&str>;
}
```

`DocumentSnapshot` 增加 `resources`/`resource_names`/`resource_sources`。归档是这个快照的
序列化，**不是**第二数据源：渲染与解析始终读 `cad-db`。

把字节放在归档而不放在 db 的替代方案（index-only + archive-as-byte-store）会违反
AGENTS 铁律 3（db 是唯一权威），因此不在候选集内。代价是常驻内存，用 D10 的预算封顶。

### D6 — 条目校验用单一 `resource_entry` 助手，不用前缀匹配

`is_modelled` **不是**拒绝名单，而是"接受并重新编码"的允许名单：`check_known_entries`
只在它返回 `false` 时报 `ymodel.unsupported_minor`。因此**不能**加 `resources/` 裸前缀匹配——
那会让未解码的条目被当成已建模、悄悄不进入快照、下次保存丢掉（正是要防的丢失）。

正确做法（与 `texture_key` 完全同构）：

```rust
fn resource_entry(path: &str) -> Option<(BundledKind, &str)>   // 校验 kind + 扩展名白名单 + 32 hex
```

同一助手被 `is_modelled`、`parse_resources`、`plan_entries` 三处共用；任一不合法即不匹配，
listed 条目随之被 `ymodel.unsupported_minor` 拒绝（拒绝打开优于静默丢字节）。

标记分工：**读侧**非法/不认识的资源条目一律 `ymodel.unsupported_minor`（门禁先触发），
`resource.unsupported_kind` 只用于**采集/写入**侧。

### D7 — 不需要 `ChangeScope.resources`

内容寻址使"字节变了"等价于"键变了"，于是永远是 insert（新路径），不存在"脏的既有键"。
快照决定存在什么，`previous` 的 `sha256` 在读取时逐条复核，所以漏记 scope 不会丢数据。

因此：**不新增** `ChangeScope.resources`。只加 `TableKind::Resources`（管
`tables/resources.json` 这张派生索引，走既有 `dirty.tables` 机制）。

原稿要的"漏写 scope 就会失败"的契约测试**无法真正失败**，属于 AGENTS 铁律 6 禁止的假证据，
不写。真正的安全网是 D1 的 `sha256(bytes)[..32] == key` 写入断言。

### D8 — 未改动的资源条目 raw-copy

`copyable` 判定：`previous.record(path)` 存在、`!foreign`、且 `resource_entry(path).is_some()`
→ `zip.raw_copy_file`。**不需要** `lists_resource` 这类平行结构（shard 需要它是因为路径还要
解析出 model/shard；资源路径由 `resource_entry` 校验）。

实现上把"可拷贝"从 `Planned.shard: Option<(ModelKey, Shard)>` 泛化为
`Planned.copy: Option<CopySource>`，保持写侧唯一不变量
`body.is_none() ⟺ copy_records.contains_key(path)`。

两种边界：

- **删除**资源：新快照不再规划该路径 → 既不写也不拷贝 → 条目自然消失（无需 tombstone）。
- **与外来条目撞路径**：若上版归档里存在同路径但**未列在 manifest** 的 `resources/...`，
  它会被判为 foreign 并与本次计划冲突（`previous::conflict`，保存失败）。内容寻址下这种碰撞
  几乎只来自手工构造的归档。规则：**比较 `sha256`，相同则由已建模条目接管（替换外来记录），
  不同则 `resource.conflict` 拒绝**——不静默覆盖不同字节。
  （`textures/` 有同样的潜在碰撞，本轮不动，记入 D14。）

`textures/<key>` 本轮**不**纳入 raw-copy，保持既有行为；差异写进代码注释。

### D9 — 解析优先级：内嵌 > 宿主 > 显式缺失

| 资源 | 内嵌命中时 | 未内嵌时（现状） | 都失败 |
|---|---|---|---|
| 字体 | 以 `name` 注册给 shaper，优先于目录/系统字体 | `FontCatalog` + 宿主 `fonts/`/`--font` | `font.unresolved`（既有）+ `text.glyph_missing` |
| `IMAGE` | 字节进 `DecodedImageCache`（按逻辑键，见下） | 图纸目录 → `images/` | `image.unresolved`（既有） |
| 外部参照 | 由未来 `XREF RELOAD` 消费；渲染**不**读 | 不可用（无活链接） | 保持 `ExternalReferenceStatus` |

**内嵌必须优先**，否则不叫自包含；宿主文件更新时不"取较新者"——文档即真相。

两处接缝必须按现状代码走，不能靠改接口糊过去：

- **字体**：`FontLoader` 是 URL 取字节器，不是入场券。`plan_fonts` 只产出 URL、`FontCatalog`
  只接受裸文件名，所以内嵌的 `resources/font/<hash>.ttf` 永远不可能成为 catalog 条目。
  正确接缝是 `load_font_engine_for_locale` **之前**过滤：把已内嵌的名字从 `requested` 移除
  （否则它们会进 `report.unresolved` 并触发 arial/simplex 回退链与 `font.unresolved`），
  先用 `register_with_encoding` 注册内嵌字节，再记一行 `FontFaceSource::Document`。
  形状对齐既有图片先例 `document_supplies`。
- **`IMAGE`**：缓存键必须是**逻辑键** `ResourceKey(image_resource_key(name))`（必要时回退裸
  `name`），**不是**归档键；否则 mesh/图像批次带的是逻辑键，查不到，仍然画空帧。
  另外 `cad:Image.file` 可能是源绝对路径而 `resolve_image_file` 会直接拒绝绝对引用，
  所以内嵌查找按原始 `name` 走、**绕过** `is_safe_reference`。归档键只是存储，永不当身份。

### D10 — 预算与拒绝

- 采集期：单条 ≤ **32 MiB**、总量 ≤ **64 MiB**。超出 → `resource.budget.entry` /
  `resource.budget.total`，逐条记录并**跳过该条**（不是整篇失败）。
  总量取 64 MiB（不是原稿的 256 MiB）：字节常驻 db，且写盘路径存在多份临时副本（D15），
  256 MiB 会让单次保存的峰值超过半 GB。
- 写入期：沿用 `ZipDocumentStore::max_bytes`（`PERSISTENCE_TOO_LARGE`）作最后兜底。
  注意 native 载体的 `max_bytes` 是 `None`，所以**不能**指望它拦 D10 的超限。
- 读取期：`len` 超限或 `sha256` 不符 → 既有 `ymodel.invalid_document`；单条超限 →
  `resource.budget.entry`。

### D11 — 恢复快照一律 `ResourcePolicy::None`

`crates/cad-host/src/recovery.rs` 当前**硬编码 `previous: None`**（"恢复文件是独立的"），
不存在原稿假设的 `with_previous`。若恢复沿用文档策略，就是每 60 s 全量重编码 + 两次整档复制，
而 native 侧没有任何上限拦得住。

因此：恢复快照**不打包资源**，并在该恢复文档里记 `recovery.resources_omitted`（显式，不静默）。
恢复后需要重新从宿主解析资源——这与"恢复不等于回滚"一致，且恢复文件体积可控。

（若将来要让恢复也自包含，前提是给 `Persistence` 加一条"上次恢复字节"的读侧，再传
`previous`；不在本轮。）

### D12 — `FORMAT_MINOR` 上限升到 5，写入按文档取 4 或 5

- `cad-domain::ymodel::FORMAT_MINOR = 5` 表示**可读上限**（`gate` 拒绝 `minor > 5`）。
- 写入时**按文档计算**：`resources` 为空且 `policy == None` → 写 `4`；否则写 `5`。
  理由：`gate` 在解码任何 entry 之前就触发，无条件写 5 会让老构建连
  "policy=None、零资源"的普通图都打不开，与 D3 的兼容承诺矛盾。
- 新读旧：minor ≤ 4 无 `resource_policy`（→ `None`）、无 `resources/` 条目 → 正常打开。
- 旧读新：带资源（minor 5）的文档被 `ymodel.unsupported_minor` 拒绝——**这是设计意图**，
  旧构建无法理解 listed 资源条目，必须拒绝而不是丢字节。
- 新标记：`resource.unsupported_kind`、`resource.name_conflict`、`resource.budget.entry`、
  `resource.budget.total`、`resource.conflict`、
  `resource.unavailable`、`recovery.resources_omitted`。前缀统一 `resource.`，
  与既有 `font.*`/`image.*`/`xref.path.*` 并列，不替换它们。

### D13 — 采集在宿主保存路径；结果走本轮状态通道，不写进本轮归档

`cad-db` 只持久化已决定的字节；采集（读磁盘/字体目录/`resolved_path`）是 I/O，属
`cad-platform`/`cad-host`：

1. `cad-host/src/save.rs::export_ymodel` 在 `snapshot_document()` **之前**按策略采集；
2. 采集结果是纯输入 `Vec<(BundledKind, name, source, bytes)>`，由 session 写入 db（内容寻址
   幂等，因此只会在第一次保存推进一次 revision），再走既有保存流程；
3. 缺失/超预算逐条进**本轮的保存结果/状态通道**。

**不写进 `tables/compat.json`**：`record_export_report` 发生在文件落盘之后，而 `.ymodel` 的
报告得写进它自己正在生成的归档，时序不成立；且该表上限 32 条（`MAX_EXPORT_RECORDS`），
会丢掉超预算行。兼容表的历史语义保持"最多 32 条既往记录"，若将来要持久化资源结果，
需另立一张无淘汰表（不在本轮）。

不做"保存时重新读底图/重新导入外部参照"——只复制字节。

### D14 — 不做的事（明确非目标）

- **不解压到磁盘**：字节只在内存里流转，不引入解压目录与 zip-slip 写面。
- **不把外部参照变成活链接**：不实现 `XREF RELOAD`；内嵌源只保证重新导入/导出保真。
- **不做 L0 源字节直通**（`raw/source-*.bin` 仍未实现）。
- **不改 `textures/<key>` 命名**，不把 `IMAGE` 与网格贴图合并成一张表，不改其碰撞规则。
- **不做字体子集化、不做字体授权校验**：授权由用户承担，UI 提示即可。
- **不做加密/签名/口令**。
- **不做跨载体接线**：Windows/macOS/Android/Web 打开/保存 `.ymodel` 仍以
  `unsupported-runtime: ymodel` 拒绝。**（2026-11-25 更新）** 后续 ADR-0060 已把 Web/`dom-canvas`
  的归档打开/保存接线；Web/X11 仍**不提供外部资源嵌入问题**（`unsupported-runtime:
  embed_resources`），本 ADR 的内嵌资源语义不变。

### D15 — 写盘路径不做整块临时副本

`Body::encode` 当前对 `Body::Binary` 返回 `bytes.to_vec()`，随后 CRC 与 sha256 各扫一遍；
`apps/app-linux/src/ymodel_archive.rs` 还会把 `request.previous` 与产出字节各复制一份。
资源字节最大 64 MiB 时，这些临时副本叠加的峰值是数倍。

要求：资源字节**直接**写入 zip（不建中间 `Vec`），`sha256`/`len`/`crc` 在**一趟**扫描里同时
算完；`previous` 以借用/`Arc` 传递，去掉 `app-linux` 的 `to_vec()`。

### D16 — 快照比较不得比较字节

`DocumentSnapshot` 派生了 `PartialEq`。资源字节进快照后，任何快照比较都会变成整块内容比较，
`snapshot/build.rs` 的不动点测试也随之变成 O(bytes)。要求：`PartialEq` 只比较
`(kind, key)` 集合与长度（纹理同理），字节相等另设 `bytes_equal` 助手供往返测试使用。

### D17 — 保存对话框的开关就是"自包含"，必须讲清得失

保存路径上的开关**不是**技术选项，而是"这个文件能不能单独带走"。因此 UI 必须当场说明：

- 标题：**嵌入外部资源（自包含文件）**
- 收益句：*把字体、外部参照原文件和图片一并打包，拷贝到别的电脑打开时不会缺字体、缺图、缺参照。*
- 代价句：*文件体积会变大（最多增加 64 MB），保存会稍慢。*
- 默认：**关闭**（与现有行为一致，不改变任何人当前产物）。
- 现场反馈：列出探测到的资源（名称 / 类型 / 实际大小 / 未找到），并给出合计大小，
  让用户在勾选前就看到"会增加多少"。未找到的资源单独成组并标注不会被嵌入——不给"已打包"
  的错觉。
- 不写营销词，不写"完美""无损"；只说打包了什么、变大多少。

CAD 命令行 `SETVAR` 不新增键：策略是保存流程的输入（D3 的文档字段），不是绘图变量。

### D18 — `read()` 保留上版字节的条件要扩到资源

`cad-store-zip::read` 目前只在存在外来条目时保留上版字节。若某载体依赖这条自动捕获来做
增量保存，则带资源但无外来条目的归档会失去 raw-copy 依据。要求：谓词扩为
"有外来条目**或**有已建模资源条目"。（Linux 宿主自己传 `previous`，不受影响，但文档不能再
声称"读取即自动让下次保存增量"。）

---

## 2. 对现有实现的改动点（文件级）

| crate | 改动 |
|---|---|
| `cad-domain` | 新 `src/resource.rs`（`BundledKind`/`ResourcePolicy`/键校验）；`ymodel/header.rs` 加 `embed_resources`（仅 true 时写）；`ymodel/mod.rs` `FORMAT_MINOR = 5` |
| `cad-db` | `DrawingDatabase` 资源表 + `insert_resource`/`drop_resource`/`resource_name`/`resource_key_for_name`；`snapshot/*` 新表与字段；`scope.rs` 加 `TableKind::Resources`（**不加** `ChangeScope.resources`）；快照 `PartialEq` 去字节 |
| `cad-store-zip` | `resource_entry` 助手 + `plan_entries`/`copyable`/`Planned.copy`；`parse_resources`；per-document minor；`sha256` 前缀断言；`read` 保留条件扩展；`Body::Binary` 直写 |
| `cad-platform` | 字体：`requested` 前置过滤 + `FontFaceSource::Document`；图片：内嵌按逻辑键入缓存、绕过 `is_safe_reference` |
| `cad-host` | `save.rs::export_ymodel` 采集 + 预算 + 状态报告；`recovery.rs` 固定 `ResourcePolicy::None` + `recovery.resources_omitted` |
| `apps/app-linux` | `ymodel_archive.rs` 去掉 `previous`/产出的 `to_vec()` |
| `apps/desktop-ui` | 保存对话框：`打包外部资源` 开关 + 资源清单（名称/类型/大小/缺失），设计交 @designer |
| `crates/cad-ui-core/i18n/{zh-CN,en}.json` | 新增文案键 |
| `docs/` | `compatibility.md`、平台表、`docs/xref.md`/`docs/fonts.md` 各一句指向本 ADR |

---

## 3. 验证计划

| 层 | 证据 |
|---|---|
| 契约（必做） | `round_trip`：insert_resource → snapshot → encode → decode → 字节/`name` 一致，含 minor 4/5 两种写入口 |
| 表形状（必做） | `tables/resources.json` 读回后仍是数组；对象外壳被拒（钉住 BLOCKER 1） |
| 键不变量（必做） | 键与字节 sha256 前缀不符时写入**失败**（钉住 D1；这是唯一的静默损坏防线） |
| 增量（必做） | 二次保存未改资源 → raw-copy 命中且该 entry 字节不变 |
| 拒绝（必做） | 非法键（`../`、绝对路径、大写扩展名、未知扩展名）在写入与读取两侧都拒绝；listed 但仍不合法 → `ymodel.unsupported_minor` |
| 碰撞 | 上版未列出的同路径资源条目：字节相同由已建模接管，不同则 `resource.conflict` |
| 预算 | 单条 > 32 MiB / 总量 > 64 MiB → `resource.budget.*`，其余条目仍写入 |
| 快照比较 | `PartialEq` 不比较字节：仅字节不同、键相同的两个快照判等（钉住 D16） |
| 宿主 | 保存契约：缺失字体/图像进本轮状态报告，不静默；恢复快照不含 `resources/` 且带 `recovery.resources_omitted` |
| 未运行 | 真实样本跨机打开、字体渲染一致性、真机/GPU/像素一律 **NOT RUN**，不得写成已验收 |

---

## 4. 证据边界

本轮只做**库层 + 宿主契约**。真实 DWG 跨机自包含、字体度量一致性、外部参照重新导入保真
都需要用户提供样本，未运行即标 `NOT RUN`。跨载体（Windows/macOS/Android/Web）仍是
`unsupported-runtime: ymodel`，不因本 ADR 改变。**（2026-11-25 更新）** Web/`dom-canvas` 的
归档打开/保存由 ADR-0060 接线；其外部资源嵌入问题仍不提供。
