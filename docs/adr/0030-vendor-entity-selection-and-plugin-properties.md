# ADR 0030：天正图元拾取、handle 语义与插件属性/命令提供者

- 状态：**Accepted（2026-11-05；2026-11-23 增补 D7–D9，P0 已实现）**。
  P0 覆盖：`tables/vendor.json`/minor 12、`YANE/YANS` 语义帧、墙/洞/文字/块插入的语义提取、
  `openProgress`/`documentReady`/`document-ready`、SAVE/EXPORT/PLOT 门禁。**P1**（其余已支持类的
  语义覆盖、copy hook、CLI 显式拒绝）与 **P2**（partial 行的 raw fallback、解码器扩展）**未实现**。
  验证边界：Linux 契约/单元测试；Windows/macOS/Android/Web 仍只到编译或契约级，
  真实 GPU/真机/像素 **NOT VERIFIED**。证据见
  `docs/validation-vendor-ymodel-2026-11-23.md`。
- 日期：2026-11-05；2026-11-23 增补 `ymodel` 语义持久化、两段式插件 ABI、打开完成事件与 SAVE 门禁。
- 相关：[ADR-0024](0024-dom-canvas-wasm-cad-path.md)（`dom-canvas` 载体）、
  [ADR-0027](0027-android-native-below-carrier.md)（`native-below` 载体）、
  [ADR-0037](0037-ymodel-schema-and-persistence.md)（ymodel schema/minor）、
  [ADR-0042](0042-bim-semantics-in-ymodel.md)/[ADR-0043](0043-parametric-dependency.md)
  （可选的 per-element 语义块与 host-owned 表的先例）、`docs/vendor-plugin.md`
  （现有 ABI v1 vendor pass）。
- 影响：`cad-app` 拾取目录、`cad-host` 点击/hover、`apps/desktop-ui` 属性 dock 与命令漏斗、
  `plugins/tch` 插件。**不**改变：`cad-db` 是唯一权威、vendor 派生表示不落库、单 React 树、
  `native-below`/`dom-canvas` 的文档所有权规则。

---

## 1. 问题

1. `TCH_*` / 无法解码的代理图元在 `cad-db` 里是 `SemanticGeometry::Opaque`。场景装配会把插件解出的
   派生 primitive 挂到源实体 id 上，所以它们**看得见**；但 CPU 拾取目录只读 `cad-db` 几何，
   `Opaque` 在精细拾取里返回 `PickOutcome::Unsupported("opaque")`，因此**点不中**。
2. 插件目前只有一个同步入口 `decode(records) -> result`，没有“启动时注册监听”的通道；属性命令
   `PROPERTIES` 是 chrome-local selector，从不经过 Rust 命令/事件，也没有插件拦截点。
3. 用户需要：天正图元可点击选中；选中后属性命令可由插件接管并显示天正自定义属性；插件可拒绝
   暂不支持的编辑操作（新增/移动/复制/删除等），未接管时继续走原属性面板。

## 2. 决策

### D1 可选中性以 `cad-db` 提交实体为准，不以 source handle 为准

- **可选中**：已经提交进 `cad-db` 的实体，身份是 `SelectionRef { document, entity: EntityId,
  instance, sub_element }`。`EntityId` 是唯一选择键。
- **不可选中**：transient overlay（绘制/测量预览、框选、捕捉提示、grip marker、光标）只携带
  `preview_source_id` 的合成哈希 id，**没有**真实 `EntityId`、**没有** handle；
  `DrawingTransaction` 里 staged 但未 commit 的实体不在 `cad-db`，也不可选中。
- `DbObject::source_handle: Option<String>` 是**来源出处**，不是选择身份：
  - 从 DWG/DXF 导入的实体有 handle（大写十六进制）；
  - API/编辑/COPY 创建并提交的实体有 `EntityId`，但 `source_handle = None`（属性面板显示
    `handle = none`，不伪造）；
  - 因此“带 handle 的都能选中”成立，但反过来“没有 handle 就不可选中”**不成立**——只要已提交
    到 `cad-db` 就可选中。这与 AutoCAD 的“已加入数据库即可选择、transient 不可选择”行为一致，
    只是本项目的 handle 字段额外承担“源文件出处”语义。
- 约束：不得为 transient/staged 实体伪造 handle 或 `EntityId`；不得把 GPU pick key 当持久身份。

### D2 vendor 派生几何进入拾取目录（替换 `Opaque`，不是写库）

- 当实体几何是 `SemanticGeometry::Opaque`，且 `VendorRepresentationStore` 按 `source_handle`
  能给出派生 primitive 时，拾取目录为这个实体提供一份**派生的 analytic 几何**
  （`SemanticGeometry::Compound`），`source` 仍是原实体的 `SelectionRef`，`transform` 仍是
  INSERT 展开的累计变换，`geometry_source = GeometrySource::ProxyCache`。
- 映射：`Lines` → `Polyline`；`Segments` → 多条 `Line`；`Arc`/`Circle` → 对应 analytic 变体；
  `Text` → 其 origin 的 `Point`（只保证锚点可点，不伪造字形轮廓）；`Block` 暂不展开
  （该 TCH 实体若只有 block primitive 则仍不可点，必须显式记录为限制，不得假装可点）。
- 没有 vendor 快照 / 没有派生 primitive 的 `Opaque` 保持原行为：精细拾取 `Unsupported("opaque")`，
  并在 `PickReport.skipped` 里保留原因。
- 图层可见性仍按源实体判断；`SELECTALL`、框选、点击、hover 使用**同一**目录。
- `PickIndex` 缓存键必须包含 vendor 快照 revision（当前只含 `SceneIdentity` + document，vendor
  快照变化不改 DB revision，必须补上），否则插件 settle 后仍会命中旧索引。

### D3 插件提供者注册（ABI v1 可选扩展，不改二进制帧）

- `plugins/tch` 的 `abiVersion` 保持 **1**；records/result 帧布局不变。
- 新增可选导出：
  ```ts
  export function register(host: VendorPluginHost): void;
  ```
  宿主在启动时加载模块并调用一次。`register` 抛错/缺失都只是“没有提供者”，不得阻塞应用。
- 宿主 API（插件侧只依赖这个结构，不导入宿主源码）：
  ```ts
  interface VendorPluginHost {
    on(event: 'properties', handler: (selection: PluginSelection) => PluginPropertyPanel | null): void;
    on(event: 'command', handler: (request: PluginCommandRequest) => PluginCommandDecision | null): void;
    on(event: 'cad-status', handler: (status: PluginCadStatus) => void): void;
  }
  ```
- 事件语义：
  - `cad-status`：每次 `cad_event` 的 `status-changed` 快照（含 `selection`）。插件可缓存；
  - `properties`：用户执行 `PROPERTIES` 时由 chrome 调用，传入**当前** Rust 状态里的
    `selection`；返回 `null` = 不接管，继续默认属性面板；返回 panel descriptor = 接管；
  - `command`：命令漏斗在准入后、发送 `cad_command` 前调用；返回 `null` = 放行；
    返回 `{ handled: true, messageKey }` = 拦截并显示模态说明。
- 多个提供者按注册顺序询问，第一个 `handled` 生效；插件异常被宿主捕获并视为“不处理”，
  不得静默变成成功。
- 当前限制：`cad_status.selection` 是多选**交集行**，不含逐个实体列表；因此 `properties` 与
  `command` 的插件判定只在 `selection.count == 1` 且 `handle` 行非 mixed 时可靠。多选一律
  pass-through（显式限制，后续如需多选接管要先扩展 wire，不在本 ADR 范围）。

### D4 属性命令：复用现有 `cad_event`/`status-changed`，不新增 Rust 命令

- Rust 已经在每次 flush 推送 `cad_event { type: 'status-changed', status: { selection, ... } }`。
  本决策把这条既有状态流作为插件的“cad-status 事件”来源。
- `PROPERTIES` 仍是 chrome-local selector（不产生 `cad_command`、不写 DB）。chrome 在打开属性 dock
  前询问已注册的 `properties` 提供者：
  - 返回 panel → 在属性 dock 中渲染**宿主渲染的**插件面板（插件只给分组只读 rows，不操作 DOM）；
  - 返回 `null`/插件未就绪 → 打开原有 `PropertiesPalette`。
- 属性 dock 的归属不变：`native-below` 由 `palettesAvailable` 门控；`dom-canvas` 无宿主文档状态时
  不出现。插件输出是 display-only，不进入 `cad-db`、不参与保存/导出。
- 面板 row key 使用稳定机器键（如 `wall.height`），宿主按 `properties.tch.<key>` 翻译；未知键显示
  原键（与默认面板的 fallback 一致）。插件不得硬编码中文文案。

### D5 编辑拦截：只在当前选择含天正实体时拒绝

- 命令漏斗在 `cad_command` 之前询问 `command` 提供者。天正插件对
  `MOVE/COPY/ERASE/ROTATE/SCALE/MIRROR/STRETCH/TRIM/EXTEND/OFFSET/EXPLODE` 等编辑命令，
  且当前单选实体在插件 records 缓存中命中时，返回
  `{ handled: true, messageKey: 'vendor.editing.unsupported' }`。
- 普通图元编辑、以及未选中天正实体时的命令**不受影响**。普通绘制命令（`LINE` 等）不拦截：
  本应用没有“创建 TCH 实体”的命令，不能把普通绘图误判为天正编辑。
- 拦截只发生在 chrome 漏斗；Rust 命令仍可被其他入口直接调用。该边界必须写入文档，不能声称
  插件是安全边界。

### D6 权威与隔离

- 插件仍是同权 module script（`docs/vendor-plugin.md` §6.3 的已知缺口），本 ADR 不宣称沙箱。
- `cad-db` 唯一权威；vendor primitive、插件属性行、插件面板都是派生显示。插件不能写 DB、
  不能改选择、不能伪造 handle/EntityId。
- 跨载体：所有 React 宿主共用同一 `apps/desktop-ui` 与 `cad_event`；`native-below`/`dom-canvas`
  都通过各自 Rust 会话的 `status.selection` 驱动，不按构建目标分叉。

## 3. 数据契约

```ts
// 宿主 → 插件（结构类型，插件不 import 宿主源码）
interface PluginSelectionRow {
  key: string; value: string | null; group: string;
  mixed: boolean; editable: boolean; kind: string; options: string[];
}
interface PluginSelection {
  count: number; empty: boolean; rows: PluginSelectionRow[];
  // handle 由 rows 里的 "handle" 行给出；多选时通常 mixed。
}

// 插件 → 宿主
interface PluginPropertyPanel {
  kind: string;      // TCH_WALL / TCH_COLUMN / ...
  handle: string;    // uppercase hex, source provenance
  groups: Array<{
    key: string;     // identity | geometry | text | ...
    rows: Array<{ key: string; value: string }>; // properties.tch.<key>
  }>;
}
interface PluginCommandRequest { command: string; selection: PluginSelection }
type PluginCommandDecision = { handled: true; messageKey: string } | null;
```

`PluginPropertyPanel` 由宿主渲染；插件不提供任意 HTML/脚本。`messageKey` 必须存在于
`crates/cad-ui-core/i18n/{zh-CN,en}.json`。

## 4. 失败闭合

- 插件未加载/无 `register`/handler 抛错：`properties` 返回 `null`，默认面板；`command` 放行。
  错误写诊断（chrome banner），不伪造“已处理”。
- 插件返回未知 `messageKey`：宿主显示稳定的通用 key，并保留原 key 作为诊断；不静默吞掉。
- 插件对多选返回 panel：宿主拒绝并在诊断中记录 `vendor.plugin.multi_selection_unsupported`。
- vendor 派生几何为空/只有 block primitive：`Opaque` 仍不可点，`skipped` 保留 `opaque`；
  `SELECTALL`/框选不得把不可点实体算作“已支持”。
- 任何情况下都不得把插件结果写入 `cad-db`、不得修改 `source_handle`、不得生成第二份文档状态。

## 5. 测试与证据

- Rust：`cad-app` 拾取测试覆盖
  - 已提交实体可选中；staged 事务实体不在 DB/目录；API 创建实体 `source_handle=None`；
  - `Opaque` + vendor store 可点，无 store 时 `skipped = opaque`；
  - vendor revision 变化使 `PickIndex` 失效；
  - `SELECTALL`/框选包含 vendor 派生实体。
- 前端：插件 `register`/`properties`/`command` 的宿主单测；无插件/异常/多选 pass-through；
  `cad-command` 的 `blocked` 结果；面板 row 翻译；i18n 键集一致。
- 插件：合成 records 帧 → `decode` → `properties` 断言墙/柱/文字/洞口行；真实样本只在有
  `YACR_TEST_DWG` 时运行，跳过不算证据。
- 手工：Linux 真实 GUI 打开天正样本，点击墙/柱/文字，执行 `PROPERTIES`，确认插件面板；
  选中天正后执行 `MOVE`/`ERASE`，确认模态拒绝；未选中天正时普通编辑不受影响。
  **本 ADR 不把 Windows/macOS/Android/Web 的编译或契约结论写成运行验收。**

---

## 6. 2026-11-23 增补：ymodel 语义持久化（D7）

### 6.1 问题与证据

- **vendor records 是 import 会话态**：保存只序列化 `DocumentSnapshot` + `Compat`/`Sources`，
  `install_snapshot` 明确 `last_vendor_records = Vec::new(); vendor_records_pending = true`
  （`crates/cad-app/src/host/snapshot.rs:118-122`）；打开 `.ymodel` 时下一 tick 用 `Some([])`
  清掉 vendor pass。
- `cad-db` 里 TCH 实体只剩 `Opaque{payload: []}` 或代理缓存展开的 `Compound`
  （`crates/cad-connector-acadrust/src/entity/convert.rs:526-547`）。
- 代理缓存不是可靠兜底：公开样本的 TCH proxy 常见只有 `PUSH_MATRIX`/`POP_MATRIX`、零 primitive
  （[ACadSharp #1131](https://github.com/DomCR/ACadSharp/issues/1131)）；TCH 也没有公开格式规范，
  base 1–5 与 9–14（T20/T30 cipher）都靠逆向（MiniCAD `TchCipher.h`/`TchWall.h`）。
- 结论：`.ymodel` 重开后没有任何路径能重建 TCH 派生表示；必须显式持久化**版本无关语义**。

### D7 持久化语义，不持久化原始 class body

三条版本轴必须分开，只有语义模型跨版本：

| 层 | 例子 | 是否可持久化 |
|---|---|---|
| DWG 容器 | merged record 拆分、skip bits、pre-R2007 内联文本、handle stream | **否**，只在 import 解码 |
| TCH 对象编码 | base/class version、property bag 布局、84 字节保护几何 + XOR key、T20/T30 cipher | **否**，只在 import 解码 |
| 语义模型 | 墙起终点/厚/高、洞口宽高/所属墙、文字、块引用、属性集 | **是** |
| 显示派生 | kind 0–5 primitives | 默认**不**持久化；由插件重算 |

**载体**：新增 host-owned `tables/vendor.json`（`TableKind::Vendor`），**按 entity id 索引**，
文档 `minor = 12`。理由：

- TCH 实体的 `geometry` 不统一（`Opaque` 或 `Compound`），不存在一个统一的 per-element 挂点；
- 插件在 `publish_imported` **之后**才回答，per-element 写入会为 1 万级实体引入 revision/undo pass；
- `Sources`/`Compat` 已是 host-owned metadata 先例，`cad-db` 不必知道 vendor 语义；
  运行时行由 document slot 持有（与 `compat`/`sources` 同型），`snapshot_document` 写入表、
  `install_snapshot` 读回 slot；插件 primitives 仍是派生显示，`cad-db` 唯一权威与
  "primitives 不落库"不变。
- 不选 `cad:Opaque.payload`：`Opaque` 的语义是"未解释的代理字节"，且实体几何可能不是 `Opaque`；
  不选 foreign entry：foreign 只从上一份 archive raw-copy，且明确"不建模"。

**行形状**（`schema` 是 vendor 语义版本，不是 ymodel minor）：

```json
{
  "entity": 123456,          // host-added persistent id (raw i64 JSON number)
  "handle": "1A2F",
  "kind": "tch",
  "class": "TCH_OPENING",
  "schema": 1,
  "status": "ok",
  "source": { "handle": "1A2F", "dwg_version": 1027, "tch_base": 6,
              "class_version": 3, "decoder": 1, "raw_sha256": "…（可选）" },
  "units": { "scale": 100, "unit": "mm" },
  "attributes": [
    { "name": "MATERIAL", "type": "i32", "value": 60 },
    { "name": "DN", "type": "f64", "value": 100.0 }
  ],
  "params": {
    "position": [1000, 2000, 0], "width": 900, "height": 2100,
    "thickness": 240, "sill": 0, "angle": 0, "mirror": 0, "kind": 0,
    "segments": [], "label": "M0921", "labelHeight": 300
  },
  "refs": [
    { "role": "wall", "kind": "entity", "handle": "1A2B", "entity": 123457 },
    { "role": "block2d", "kind": "block", "handle": "B10" }
  ],
  "diagnostics": []
}
```

规则：

- `status ∈ ok|partial|unsupported` 与 import 时的稳定 reason codes **必须原样持久化**；
  reopen 不得把 `unsupported` 洗成 `ok`。
- `attributes` 是 decoded property bag + `TDBXDATAMAP` 的 typed `name → value`；未知键/类型原样保留
  （int32/f64/string/point3/handle）。文字样式的 `_TCH` XData 也按同一规则存。
- 值一律存 **world space**，并带 import 时的 unit/scale 快照；**不存**"已经切好洞的墙"：
  墙与洞的关系、切口在插件 render 时重算。
- `refs` 存 `{kind: entity|block, id?, handle}`；宿主在保存时写 `id`，插件不解释持久 id。
  解析顺序 `id → handle`，失败报 `vendor.ref.unresolved` / `vendor.ref.block_missing`，不得静默丢。
- import 时插件只给 `source_handle`；宿主用 `source_handle → EntityId` 映射写行
  （导入器已在 `push` 时写 `source_handle`；没有现成索引就由宿主构建一次，
  绝不让插件猜持久 id）。block ref 用 `block_source_handles` 解析。
- 删除实体后的 orphan row 在 save/load 过滤并报 `vendor.orphan_rows`；copy/paste 走 copy hook
  （P1；未接线时明确写"pasted TCH 重开无派生显示"）。
- 行数/总字节有显式上限，超限截断并写诊断。raw class-body **默认不落盘**；
  仅 `partial/unsupported` 行可选的 `reimport_required` fallback（P2），永不优先于语义。

`docs/vendor-plugin.md` §5.4 维护逐类可持久化字段清单；本 ADR 只固定边界。

---

## 7. 两段式插件契约（D8）

- **import 阶段**保持 `decode(YAND) -> YANR` 不变；新增
  `extractSemantics(YAND) -> YANE`：
  - 同一 token/generation；独立 32 MiB 上限；与 `decode` 共用同一套 parse/几何 builder；
  - 只输出语义行（handle/class/status/attributes/params/refs/diagnostics），不输出 primitives；
  - 不能把 semantics 追加进 4 MiB 的 result frame：一条大记录会连带打挂显示 pass。
- **reopen 阶段**：host 从 `tables/vendor.json` 生成 `YANS` 输入帧，`decode` 按 magic 分派；
  result frame 仍是 ABI v1，`applied == primitive_count` 等规则不变。
  result 帧的 `u64` 是 **pass-local key = source handle**（import 与 reopen 相同）：
  表按 entity id 索引，但 YANS 编码时用行的 `source.handle` 作为 pass key，因此现有
  `VendorRepresentationStore`（按 source handle）不变。重复 `source_handle`（copy/paste）
  是 P1 的已知限制；升级为 entity key 时必须同时改存储与场景查找，不得让一组 primitive
  覆盖另一组。
- `cad_vendor_apply` 增加可选 `semantics`（base64 YANE）：primitive 与 semantics 独立校验；
  semantics 非法只记 `vendor.semantics.*`，显示仍可 settle，**不得写半张表**。
- **schema 规则**：插件遇 `schema > max` 显式拒绝 `vendor.semantics.unsupported_schema`；
  同 major 只允许"加字段、缺省有值"，改名/删除必须升 major；未知 class 显式 unsupported。
- reopen 后属性面板/命令拦截从语义缓存回答，不再依赖 records cache；多选限制不变（D1/D5）。
- 跨载体：native 与 wasm 共用 `cad-ui-core` 的帧/校验；**不新增 per-carrier 文件**。
- 原始 class body 只在 import 存在；`YANE/YANS` 是第二个 ABI 命名空间，不是 ABI v1 的破坏性修改。

---

## 8. 打开进度、`document-ready` 与 SAVE 门禁（D9）

### 8.1 单一打开管线与状态

`CadStatus` 增加（纯 additive）：

- `open_progress: Option<CadOpenProgress>`：
  `stage ∈ import|vendor|representations`、`phase`、`done`、`total`、
  `unit ∈ entities|vendorRecords`、`cancellable`；
- `document_ready: bool`；
- `open_summary: Option<CadOpenSummary>`：最近一次打开终态
  （label/entities/vendor status/applied/diagnostics）。

```json
{
  "openProgress": {
    "stage": "vendor", "phase": "decode",
    "done": 0, "total": 9899, "unit": "vendorRecords", "cancellable": false
  },
  "documentReady": false,
  "openSummary": null
}
```

ready 后 `openProgress: null`、`documentReady: true`，`openSummary` 保留最近一次
`{label, entities, vendor: {status, applied, diagnostics}}`。

`importProgress` 保留为 import 子阶段的兼容投影；React 改用 `openProgress`，**不再在 import
terminal 时隐藏进度**。阶段：

1. **import**：现有 `ImportPhase` + 实体计数（唯一有真实 `done/total` 的阶段）；
2. **vendor**：无 records → 跳过；有 records → `phase ∈ records|decode|apply`，
   计数单位是 vendor records；单次同步 decode 内没有逐条 tick 时用 indeterminate，不伪造百分比；
3. **representations**：`phase = rebuild`，派生快照已装入 render session（indeterminate）。
   **P0 在 vendor settle 的同一 owner tick 内完成**，不单独发一帧；阶段键保留给未来的
   异步重建。
4. **ready**：`open_progress = null`，`document_ready = true`。

UI 文案用稳定键：`open.stage.import|vendor|representations`、`open.vendor.records|decode|apply`；
`import.phase.*` 继续服务 import 子阶段。没有真实 `done/total` 的阶段只渲染 indeterminate，
不把上一阶段的计数沿用到新单位。

vendor 不支持/失败/超时仍然是"打开完成"：`document_ready = true`，
`open_summary.vendor.status ∈ ok|unsupported|failed|timeout`，诊断保留；
宿主看门狗在（例如）30 s 后产生 `vendor.timeout`，**绝不永远 pending**。

`document_ready` 属于**当前权威文档**：失败的 OPEN/CANCEL 不改变仍然打开的上一份文档的
ready 状态；没有文档时 `document_ready = false`。`OPEN` 取消只取消新打开。
`document-ready` 事件每次成功安装文档最多发一次；晚连接者从 `cad_status.document_ready`
与 `open_summary` 读取，不依赖事件重放（事件环有界）。

### 8.2 `document-ready` 事件

- 新增 `CadEventKind::DocumentReady`（wire `document-ready`），在 `vendor-settled` 之后、
  同一 flush 发出；无 vendor 的文档在 `document-opened` 同批发出。
- payload：`{ label, entities, vendor: { status, applied, diagnostics } }`；诊断有界。
- 由 `cad-ui-core` 的纯函数从 `(prev.document_ready, next.document_ready, next.open_summary)`
  派生，native/wasm 都调用它，禁止各写一份。
- 语义：`document-opened` = 文档已发布（label 安装）；`document-ready` = 打开管线完成
  （vendor 已 settle、派生快照已装入）。**不冒充 GPU 首帧/像素验收**。

### 8.3 SAVE 门禁

- `can_save`/`can_plot` 在现有条件上加 `document_ready`；
  `request_save`/`request_export`/`request_plot` 的 `require_save_ready` 在未 ready 时返回稳定
  `save.document_not_ready`（`open_progress.stage` 给出原因）。
- **不**把 `loading` 扩展成 vendor pending：`loading` 继续只管 import/picker；
  `can_new`/`can_open` 保持可用，用户可取消或换文件。
- 恢复快照在 `!document_ready` 时不写，避免把缺少 vendor 行的文档当作可恢复基线。
- 门禁是 host 侧强制；UI 的 `can_save` 只是投影，直接调用 `cad_command SAVE` 同样被拒。

---

## 9. 测试与证据（D7–D9）

- **落地/重开**：`test-tangent-real.dwg` 导入 → 保存 `.ymodel` → 重开；
  `primitives(import)` 与 `primitives(reopen semantics)` 的 result frame（去 token/generation）
  逐字节相同，`applied` 相等；语义行的 status/reason 集合与 import 时一致。
- **minor/拒绝**：含 vendor 行的文档写 `minor 12`；`minor 11` reader 读它 →
  `ymodel.unsupported_minor`；无 vendor 行的文档仍按 4/5/7/… 写；篡改 `schema: 99` →
  `vendor.semantics.unsupported_schema`。
- **增量**：只改无关图层后保存，TCH 实体所在 shard 与上一份 archive 逐字节相同（raw-copy），
  `tables/vendor.json` 内容不变；删除一个洞口后保存，表行消失且墙切口按"无该洞口"重算。
- **引用/orphan**：删除墙后 opening 的 `refs.wall` 解析失败 → `vendor.ref.unresolved`；
  删除实体 → save/load 过滤 orphan 并报 `vendor.orphan_rows`；copy/paste 走 hook 或显式声明缺口。
- **打开管线**：import 中 `open_progress.stage = import`；publish 后未 settle 为 `vendor`；
  settle 后 `representations`；最终 `document_ready = true` 且只发一次 `document-ready`；
  无 records 的文档直接 ready；插件缺失/异常/超时仍 ready 且带诊断。
- **SAVE 门禁**：vendor pending 时 `can_save = false` 且 `request_save` 返回
  `save.document_not_ready`；settle 后（即使 unsupported/failed）允许保存；恢复快照在未 ready 时
  不写。
- **证据边界**：Linux 只做契约/运行验证；Windows/macOS/Android/Web 只到编译/契约级。
  TCH base 9–14、cipher、proxy retained-data 仍 **NOT VERIFIED**；不因语义持久化而宣称解码覆盖扩大。
