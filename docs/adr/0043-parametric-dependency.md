# ADR 0043：参数化依赖与求值（iTwin `ElementDrivesElement` + SolidWorks 取舍）

- 状态：**Proposed（2026-11-21）**，**未实现任何代码**。本文件是待审核的方案，不是已落地决策。
- 日期：2026-11-21。
- 相关：[ADR-0037](0037-ymodel-schema-and-persistence.md)（`.ymodel` 唯一原生格式、`(kind << 45) | seq`、
  Definition/Geometry 三段）、[ADR-0042](0042-bim-semantics-in-ymodel.md)（元素可选 `bim` 块、
  `tables/bim-relations.json`、`PropertyRow` 动态行、`cad_ui_core::host_read`）、
  [ADR-0017](0017-dimension-family.md)（`display = f(definition, style)` 的纯函数先例）、
  [ADR-0036](0036-3d-body-model-and-solid-service.md)（3D body 与 solid 服务边界）、
  [ADR-0031](0031-acadrust-vendored-patch-policy.md)/[ADR-0035](0035-rvt-vendored-patch-policy.md)
  （patch 禁令不变）。
- 影响：`cad-domain`（`param` 模块、ymodel descriptor/minor）、`cad-db`（参数表、快照、合并写入路径）、
  `cad-store-zip`（表项与 minor 分档）、`cad-history`（`DrawingHistoryStep` 新批次）、`cad-app`
  （`PARAM` 命令、`SETENTITYPROPERTY` 前置拦截、选择投影）、`cad-ui-core`/`apps/desktop-ui`（参数行）、
  `cad-cli-tools`（headless 求值验收）、`docs/compatibility.md`、`CAD_IMPLEMENTATION_SPEC.md` §2.3/§16.2。
- **不改变**：`cad-db` 是唯一权威；`ChangeMask` 保持 5 bit（规格 §16.5.3）；BIM 关系表格式与 IFC 导出
  基线；`cad-dependencies` 现有语义；patch 只允许 `acadrust`/`rvt` 的禁令；载体归属与 `native-below` /
  `dom-canvas` 的裁决。

---

## 0. 背景

需求是「改一个构件的尺寸或属性，依赖它的其它构件跟着调整」，并要求能承接 SolidWorks 的建模场景
（方程式、配置、自上而下）。今天仓库里没有这个能力：

- 规格 §2.3 把「参数化建模」列为非目标；§16.2（`CAD_IMPLEMENTATION_SPEC.md:650`）给了唯一合法口径：
  **「初期只实现依赖追踪，不实现完整参数化特征图或约束求解器。将来接入时必须沿用事务和错误边界，
  不能让求解器直接改 GPU 数据。」** 落地前必须改 §2.3 并写本 ADR（§16.4）。
- `cad-dependencies` 只有实体→实体的 BFS：边两端都是 `ObjectId`（就是 `EntityId` 的数值），
  只拒绝 `consumer == dependency` 自环、没有 DAG 校验，输出是「重建显示表示」而不是「重算」，
  且**生产未接线**（只有测试调用它）。
- 属性写路径只认 5 个显示键；`c87e4046` 刚给 BIM PSet 加了 `bim.pset.{set}.{prop}` 动态行与
  `apply_bim_updates` 合并写入。参数是同一形状的第二件事。
- `dimension.rs` 已有 `display = f(definition, style)` 的纯函数和单一 builder，但**只在创建/导入时调用**，
  样式改名不会重算已存尺寸——「有重算函数、无重算触发」的现成缺口。

外部参照（研究结论，证据边界见 §6）：

- **iTwin.js** 的 `ElementDrivesElement` 只提供「持久化有向边 + commit 边界拓扑调度 + 每节点一次回调 +
  fatal 环检测 + `status`/`priority` 记账」。它**不提供**参数、表达式、约束求解、特征历史；
  `ParametricGeometry`/`ParametricSolid` 在开源仓库中不存在。它的 `ElementAspect` 不是「派生数据容器」，
  官方立场是「能不持久化派生值就不持久化」。
- **SolidWorks** 是五层叠加：有序特征历史、引用 DAG、类型化参数/方程 DAG、配置稀疏覆盖、配合约束域。
  树序只是拓扑序的一个合法实例；配置只覆盖「抑制位 + 参数」；配合与方程**没有全局优先级**，冲突只报
  诊断；失败按节点建模（失败节点不产出新几何、保留 last-good、下游 blocked）。

所以本 ADR 只抄两件事：iTwin 的**调度与事务机制**，SolidWorks 的**语义分层**（且明确不抄它的全局阶段
顺序和无事务现实）。

---

## 1. 决策

### D1 — 三类边，只有 Value 类进入重算闭包

| 家族 | 例子 | 进重算闭包 | 权威 | 持久化 |
|---|---|---|---|---|
| **Ownership**（归属/身份） | `aggregates`、`contains`、`nests`、`group`、定义→实例 | 永不 | 结构本身 | 必须 |
| **Value**（值/定义） | 参数表达式引用、元素槽位绑定、特征输入 | **总是** | 表达式与绑定本身 | **不落盘**（可推导） |
| **Reference**（宿主/引用） | 墙↔洞↔门（`voids`/`fills`）、板↔孔特征、in-context 面/边引用 | **仅当该 `(host,hosted)` 类型对注册了规则** | 引用路径 + 状态 | 必须 |

理由：BIM 的「门窗—墙」是 Reference，不是参数化。把三类混成一个通用传播引擎，会让「墙厚变了门没动」
变成只能靠 dump 表才能发现的静默 bug；而「BIM 关系根本不是参数化更新」这个判断是对的，错的只是过去共用
了一个名字。

### D2 — 传播策略挂 **kind**（以及 host/hosted 类型对），不挂实例

- 每个 `RelKind` 有 `family`、`validation`（端点解析/无环/唯一父/同 model）与 `propagation`
  （`Never` / `Rule` / `Handler`）。
- Reference 类的规则按 **`(host_type_key, hosted_type_key)`** 注册；未注册 = 稳定诊断 `rel.no_rule` +
  不传播，**不得**有「默认按宿主尺寸重排」的通用缺省。
- 实例上不放开传播开关。iTwin 的 `0x80 Deferred` 只作为「临时不传播」的工具保留，不承担语义。
- 开放 token `other:<token>` **只能**是 Ownership：现有 IFC 导出已把它计入
  `export.ifc.bim.relation_not_written`，承载不了任何语义。

### D3 — 参数是文档级命名属性**节点**；只存表达式，不存当前值

```json
[{"seq": 7, "name": "板厚", "kind": "length", "unit": "mm",
  "expr": {"num": 12.5}},
 {"seq": 8, "name": "孔径", "kind": "length",
  "expr": {"op": "div", "lhs": {"ref": 7}, "rhs": {"num": 2}}}]
```

- 名称文档内唯一（重复 = `param.duplicate_name`）；引用用 `seq`，**改名不破坏引用**。
- 不存「当前值」：否则 P1 的配置覆盖需要一次格式迁移。
- 元素上的绑定是元素行末位可选块（照 `bim`/`pbr`：`None` 省略键，绝不写 `null`）：

  ```json
  "params": {"radius": {"ref": 8}, "height": {"num": 20}}
  ```

- 槽位名来自 `param_slots(type_key)` 的**封闭**注册表；未知槽位 = `param.unknown_slot`，不是任意用户键。
  这与 BIM PSet 的任意名字是有意的差异：槽位是「求值器知道怎么改的字段」，不是自由属性。

### D4 — 参数存 `tables/parameters.json`，不造「非图形元素」

- 身份 = 表内 `seq` + `name`，**不使用 `PersistentId`**：`IdKind` 12/13 仍保留（ADR-0042 D12），
  header 计数器不动，与 `tables/bim-relations.json` 的决定同构。
- 拒绝方案 B（参数内联到绑定元素、文档级参数挂一个容器元素）：仓库里没有「非图形元素」先例
  （BIM 是给已有实体挂语义，不是独立元素），造一个会让实体遍历、查询、DWG 导出、快照每处都要记得跳过它。

### D5 — 求值在**提交前**、拓扑序、每节点一次

- 图从权威行推导（表达式引用 + 槽位绑定 + D4 的 owner），打开后/每次提交后重建索引。
- 环在**静态校验期**检测：DFS 灰节点重入 = `param.cycle`，**事务拒绝**。持久化的图永远是 DAG。
- 平级用 `(seq, EntityId)` 稳定 tie-break，保证确定性。**不使用** iTwin 的 `priority`：它文档说
  「默认等于创建序」，实现里全写 0、顺序取决于 SQLite 扫描（见 §6），不可依赖。
- 单次求值有界：参数 ≤ 4096 行、表达式深度 ≤ 32、节点 ≤ 256、影响闭包 ≤ 10000 个元素；超限 =
  `param.budget_exceeded` 并**整单拒绝**（不降级成部分求值）。

### D6 — 两档失败，语义与落库分开

| 情形 | 结果 | 落库 |
|---|---|---|
| 未知引用 / 类型或量纲不符 / 环 / 槽位未知 / 预算超限 | 静态校验失败，**事务拒绝**，稳定 `param.*` 标记 | 不写 |
| 几何构造失败（退化、kernel 拒绝、可行性不成立） | 参数合法 → **提交参数**；该元素**不写新几何**，发诊断；下游标 `Blocked` | 写参数，几何保持旧值 |

第二档成立的原因：派生几何不落盘（D7）+ 失败不写几何 = 天然 last-good。重开文件重算仍失败会重现同一条
诊断（纯函数 → 确定性），不需要任何缓存字段，也不允许「失败改成空成功」。

### D7 — 派生几何不落盘

元素行里的 `geometry` 就是**上一次成功求值的结果**；打开时按 D5 重算，不一致即诊断。这与规格 §16.5.3
「Geometry 由 definition 生成、不落盘；Presentation v1 不持久化缓存」一致，也与 `dimension.rs` 的
单一 builder 纪律一致。

### D8 — 一条合并写入路径，并有意放宽 `DrawingHistoryStep` 的排他规则

参数命令必然同时写「参数表行」和「N 个重算实体」，所以：

- 新增 `DrawingDatabase::apply_parametric_update(reason, txn, param_patches, entity_updates)`：
  先全量校验（引用/类型/单位/环/预算/元素存在）→ 再改 → 一个 revision、一个 `ChangeSet`、一条 undo。
  空批不 bump、不伪造 changes（照 `apply_bim_updates`）。
- `ChangeScope` 必须同时填 `TableKind::Parameters` 与**每一个被重写元素的 shard**。
  `apply_bim_updates` 的注释已写明这一失败模式：漏一个 shard 只会在**第二次保存**暴露（旧字节被 raw-copy）。
- `DrawingHistoryStep` 增加 `param_patches`，并且**必须允许与 `record.patches` 同行**——这与
  `bim_patches` 的「必须独占一步」相反，因为参数与它产出的几何不是正交编辑。该放宽要写成校验规则 + 测试，
  不能靠调用方自觉。
- 不给 `DrawingUndoRecord` 加字段（102 个字面构造点），沿用 `c87e4046` 的 step 侧批先例。

### D9 — 编辑走现成动态行；结构变更走一个新命令

- **值编辑**：`SETENTITYPROPERTY` 在显示属性语法之前认领 `param.<name>`，与 `bim.pset.*` 同构：
  多选时「名字一致才可写、混合则整单拒绝（`param.mixed`）、全有或全无、无变化不记历史」。
  参数不做「按元素各自变体解析」，表达式语法全局一致。
- **结构变更**：`PARAM {op: create|delete|rename|bind|unbind}` 一个新命令（进 registry 与
  `check-command-docs.py` 文档债），沿用 `publish_candidate` 的单事务/单 undo 模式。
- 参数不是新面板：文档级列表在 P1 才需要；P0 只在属性面板显示**与当前选择绑定**的参数行。

### D10 — 载体读取走唯一 `host_read` seam，P0 不需要新 kind

P0 的参数行来自选择投影（`cad_select_entities` 路径），不需要新读取通道。P1 的文档级参数列表/求值状态
如果需要请求携带型读取，**必须**在 `cad_ui_core::host_read` 加一个变体 + 拒绝词 + `session::host_read`
路由；**不得**新增每载体文件（ADR-0042 D11 的约束）。未知 kind 仍是 `unsupported-runtime: hostRead.kind`。

### D11 — `FORMAT_MINOR` 升到 8，写入按文档取 4/5/7/8

- 8 = 元素 `params` 块 + `tables/parameters.json`。7 及以下保持可打开。
- `document_minor()` 显式分档：`has_params` → 8；`has_bim` → 7；有资源 → 5；否则 → 4。不得直接返回
  `FORMAT_MINOR`。`has_params` 落在 `cad-db`（store 不认识 `"params"` 字面量或表路径）。
- 兼容：旧读新 → `ymodel.unsupported_minor`，不静默丢；新读旧 → 无块 = 无绑定、无表 = 空参数。

### D12 — 不启用 `IdKind` 12/13

参数与参数边都不用 `PersistentId`。启用 `Relationship`/`Aspect` 需要的 header 计数器与格式迁移，留到
「关系升级为关系元素」或「Aspect 落盘」真正开始时再评估。

### D13 — BIM 关系表不改名、不并表

`tables/bim-relations.json` 是**源保真的 IFC/连接器词汇表**，`c87e4046` 已给它加了导出契约
（只导出 `contains`/`nests`，其余逐 kind 报告，无关系导出逐字节一致 2196/1374）。所以：

- 不改名（会牵动 ADR-0042 基线、导出计数与逐字节契约，收益只是名字）。
- `voids`/`fills` **加为具名 kind**（它们本来就是 `IfcRelVoidsElement`/`IfcRelFillsElement`）。
- 统一发生在**内存 `RelationGraph` + kind 策略注册表**，不发生在持久化词汇上。
- 只有「我方自造、无源词汇」的边（Phase 3 的 in-context 面/边引用）才需要新表，届时叫
  `tables/relations.json`。

### D14 — 非目标

- 不做约束求解器、不做配合/装配求解、不做「全局优先级表」（SolidWorks 自己也没有）。
- 不做特征历史 rollback/freeze、不做跨文档外部参考（P3 才评估）。
- 不引入脚本语言；表达式是封闭语法，不是求值器插件。
- 不把参数当通用属性系统（不做任意 key→value；槽位必须由类型注册）。
- 不改变 `cad-dependencies`：参数图另立，P2 再评估显示失效是否接入统一调度。

### D15 — 沿用的现有边界

- 业务层只写数据库/命令/事务；求值器是纯函数（无 db、无 GPU、无时间、无 IO）。
- 未支持能力显式建模：`param.no_evaluator`（类型无求值器）、`rel.no_rule`、`param.blocked`。
- 一切都是 `cad-db` 派生：参数与绑定是权威，几何与显示是派生。

---

## 2. 数据模型与求值

**槽位注册表（`cad-domain::param`，P0）**

| `type_key` | 槽位 | kind | 说明 |
|---|---|---|---|
| `cad:Circle` | `radius` | length | 单标量 |
| `cad:Polyline` | `width` | length | 单标量（不改拓扑） |
| `cad:Text` | `height` | length | 单标量 |
| `cad:Insert` | `scale` | number | P1：需要拆分 transform |

P0 刻意只做「不改变拓扑」的槽位；端点驱动（矩形四线、阵列展开）需要 P1 的槽位语义，
`param.no_evaluator` 先显式拒绝而不是猜。

**表达式（P0 封闭语法）**

```json
{"num": 1.0}
{"ref": 7}
{"op": "add|sub|mul|div", "lhs": <expr>, "rhs": <expr>}
{"const": "pi"}
```

P1 追加 `pow`/`neg`/一元函数/`{"if":…}`/文本模板（`cad:Text.text` 的 `{name}` 占位）。量纲在解析期检查，
统一到 mm/rad；量纲不符 = `param.unit_mismatch`，不做静默换算。

**求值结果**：每个受影响元素产出一个新的 `SemanticGeometry`（复用现有构造器；`Text` 的高度同时影响
显示重建），以普通 `Update(entity, GEOMETRY)` 发布——**`ChangeMask` 不扩位**。

---

## 3. 兼容矩阵

| 文档 | 写入 minor | 旧构建（≤7） | 新构建 |
|---|---|---|---|
| 无参数、无 BIM、无资源 | 4 | 打开 | 打开，无绑定 |
| 有资源 | 5 | 拒绝 | 打开 |
| 有 BIM（元素/定义/关系） | 7 | 拒绝（本就如此） | 打开 |
| 有参数表 / 元素 `params` 块 | **8** | 拒绝 `ymodel.unsupported_minor` | 打开 |
| 旧构建写的 minor 7 | 再保存仍 7 | — | 接受并保留 |

---

## 4. 分期与验收（计划，尚未运行）

**P0 — 垂直切片**：参数表 + 元素 `params` 块 + `num/ref/四则/pi` + 三个槽位（`Circle.radius`、
`Polyline.width`、`Text.height`）+ `PARAM` 命令 + `SETENTITYPROPERTY` 的 `param.*` 拦截 +
合并写入路径 + 参数行。

计划中的契约测试（命名先定，实现时逐条落地）：

- 格式：元素 `params` 往返 + 缺省省略；`"params": null` 与未知槽位被拒；参数表严格解码、拒重复名与
  悬空 `ref`；预算超限显式报错。
- 图：`ref` 成环 = `param.cycle` 且事务拒绝；改一个参数只重算闭包内元素且每元素恰好一次；
  多元素绑定同一参数同时更新。
- 写入：一次命令 = 一个 revision = 一条 undo；**保存两次**字节稳定（覆盖 ChangeScope 漏 shard 的失败模式）；
  undo 同时回滚参数行与重算几何；redo 一致。
- 失败：几何构造失败时参数已提交、几何保持旧值、诊断重现于重开；未知引用不落库。
- minor：有参数写 8、无参数保持 4、有 BIM 无参数仍是 7；minor-7 档案可打开。
- 面板：`param.<name>` 动态行可编辑；多选名字不一致 → 不可编辑 + `param.mixed`；非绑定元素无参数行。

建议验证命令（逐条看退出码，不用管道吞退出码）：

```bash
cargo test -p cad-domain --locked param
cargo test -p cad-db --locked param
cargo test -p cad-store-zip --locked --test archive
cargo test -p cad-history --locked
cargo test -p cad-app --locked param
cargo check -p cad-db --target wasm32-unknown-unknown --locked
python3 scripts/check-architecture.py && python3 scripts/check-file-size.py
python3 scripts/check-fixture-manifest.py && python3 scripts/check-command-docs.py
(cd apps/desktop-ui && npm test)
```

**P1 — 表达式与配置**：一元函数/`if`/文本模板；`tables/configurations.json` = 参数 `seq` 的稀疏覆盖 +
抑制集合；派生配置 = 父 + 显式 override，必须区分 inherited/overridden；文档级参数列表（`host_read`
新 kind）。`Insert.scale` 槽位。

**P2 — 3D 特征**：`order` 字段 + 特征定义元素（参数 + 输入引用 + 操作枚举）→ evaluator 调用
`cad-kernel-solidcore`（content in / content out）。`voids`/`fills` 具名 kind + `(host,hosted)` 规则；
未注册 = `rel.no_rule`。fillet/shell 仍显式拒绝。

**P3 — 自上而下**：显式 typed 边 + 状态机 `InContext/OutOfContext/Locked/Broken/Dangling`，
自有边表 `tables/relations.json`；评估 `cad-dependencies` 与统一调度的关系。

---

## 5. 风险与规避

1. **第二次保存的脏分片**：BIM 写入路径已经踩过。规避：`apply_parametric_update` 自建 scope.shards，
   「保存两次」成为必测项。
2. **放宽 `DrawingHistoryStep` 排他规则**：可能让一条 undo 跨两个 db 路径。规避：只在参数路径放宽，
   校验器显式写规则，BIM 的独占规则不变。
3. **表达式/单位蔓延**：P0 封闭语法 + 预算；文本模板、函数库进 P1。量纲不符拒绝，不静默换算。
4. **`PropertyRow` 动态键冲突**：`param.<name>` 前缀 + 选择内唯一性检查；多选交集按名字。
5. **求值器不确定**：纯函数约束 + 「同一输入求值两次结果相同」测试；禁止 db/时间/GPU 访问。
6. **打开即重算的耗时**：闭包与参数数量都有硬上限；P0 实测后再调。
7. **文件规模**：`cad-domain`/`cad-db` 已有接近 1000 行的文件。规避：新代码全部新模块，既有文件只接线。
8. **文档漂移**：ADR-0042 的 `PropertyRow`/属性编辑段曾在 `c87e4046` 后过期，已同步修正
   （2026-11-21）；本 ADR 以代码为准，不回写 ADR-0042 的历史决策。

---

## 6. 证据边界

- 本 ADR 的 iTwin.js 结论来自 `iTwin/itwinjs-core` `21f2140` 与 `itwin/imodel-native` 源码/文档；
  SolidWorks 结论来自 `help.solidworks.com` 2021–2026 与 API Help。两者都只是**设计输入**，
  不是本项目的能力声明。
- 记录三条「文档与实现不一致」，避免以后照着误传设计：① EDE 的 `priority` 默认 = 创建序只写在头文件，
  实现里全 0、顺序取决于 SQLite 扫描；② 环上「所有边 status=1」只写在 JSDoc，代码只置检测点那一条；
  ③ `ElementAspect` 不是「派生数据容器」，官方只把它定位为非 essential 属性的容器。
- 本 ADR **没有任何实现、没有运行任何测试**；§4 的测试名与命令是计划。所有平台结论（Windows/macOS/
  Android/Web）为 **NOT VERIFIED**，P0 只在 Linux 做契约与运行验证。
- 更正一条已被推翻的既有结论：`de9b02b3` 时「BIM 只有导入写路径、没有命令写路径」成立；
  `c87e4046` 之后不成立。
