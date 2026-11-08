# 动态块可见性（读 + 切换）

`动态块可见性` 支持窗口内，**状态名读取 + 状态切换**已实现；参数/夹点编辑、
参数化求值、未知状态求值是 **Partial/未支持**，明确建模且不伪造几何。

> 状态：**契约 + 合成测试**。端到端 DWG 写入 → 回读的合成样本受 acadrust 写入器
> 能力限制（见 §6），因此本轮以“内存合成 acadrust 文档 + 纯映射函数”验证读取与
> 映射；真实导出样本仍是待补证据。

## 1. 数据来源（acadrust 0.6.8，只读）

- `document.rs`
  - `block_visibility_params: HashMap<Handle, BlockVisibilityParameter>`
  - `block_visibility_param_for_def(def_block) -> Option<&BlockVisibilityParameter>`
    （:4065，沿 `owner_chain_reaches` 找到该块定义可见性参数）
  - `dynamic_visibility_for_insert(insert) -> Option<(Handle, &BlockVisibilityParameter)>`
    （:4081，经 `dynamic_definition_for_insert` 解析匿名块）
  - `dynamic_definition_for_insert(insert)`（:4096）
  - `block_representations: HashMap<Handle, Handle>`（表示对象 → 动态定义块）
- `objects/block_visibility.rs`
  - `BlockVisibilityParameter { handle, owner, all_blocks: Vec<Handle>,
    states: Vec<BlockVisibilityState>, name, description, def_point, ... }`
  - `BlockVisibilityState { name, visible_blocks: Vec<Handle>,
    visible_params: Vec<Handle> }`
- `objects/dynamic_block.rs`：`DynamicBlockObject` / `DynamicBlockData`（含
  `VisibilityParameter(BlockVisibilityParameter)`）。

匿名（求值后）块把“其他状态”的成员标为不可见
（`EntityCommon::invisible`，`document.rs` / `entities/mod.rs:311`）。这是解析
“当前激活状态”的唯一公开证据。

## 2. 数据库模型（cad-db）

`BlockDefinition` 新增可选字段 `dynamic_visibility: Option<DynamicBlockVisibility>`；
非动态块为 `None`，行为不变。

```rust
pub struct DynamicBlockState {
    pub name: String,              // 状态名，原样
    pub entities: Vec<EntityId>,   // 该状态下可见的成员实体
}

pub struct DynamicBlockVisibility {
    pub member_entities: Vec<EntityId>, // 参数管辖成员（states 的并集，来自 all_blocks）
    pub states: Vec<DynamicBlockState>, // 按源顺序
    pub active_state: Option<String>,   // 能诚实确定时才有值
}
```

- `state_names()` / `state(name)` / `has_state(name)` / `active_entities()`。
- `is_visible(entity)`：非管辖实体恒可见；`active_state = None` 时所有成员可见
  （未确定状态绝不猜测隐藏）。这是**描述符自身**的判定；块成员的最终显示状态见下面的
  `block_member_visible` 优先级。
- `DrawingDatabase::block_dynamic_visibility(block)`、`block_visible_entities(block)`。
- `DrawingDatabase::block_entities(block)` 已按激活状态过滤：只返回该状态可见的
  成员（未管辖成员始终保留）。因此**表示层与包围盒自动只画激活状态**。

### 烘焙隐藏实体旁表（源文件 `invisible`，全局）

源文件的 `EntityCommon::invisible` 是烘焙好的**显示状态**，不是猜测。它覆盖两类实体：

- **顶层**模型空间 / 图纸空间实体（例如真实夹具 `*Model_Space` 中被标为 invisible 的实体）；
- **块成员**：求值后的匿名动态块（`*U…`）把**所有状态**的几何都放在同一个块里，并用该标志
  把非激活状态的成员标记为不可见。当可见性参数缺失或激活状态无法解析时，若仍绘制全部成员，
  就会把大量隐藏几何叠画在可见成员上。

`DrawingDatabase` 在实体表旁保存一个全局集合
`hidden_entities: BTreeSet<EntityId>`（源 `invisible=true` 的实体，顶层与块成员共用）；
`BlockDefinition` 与 `DbEntity` 均不新增字段。显示/出图入口用
`DrawingDatabase::entity_hidden(id)` 查询顶层实体。

可见性判定分两级：

- 顶层实体：`entity_hidden(id)` 为真即不绘制、不进包围盒、不进入 CLI `render`/`plot`。
- 块成员：`DrawingDatabase::block_member_visible(block, member)` 是唯一判定
  （`block_entities`、`block_visible_entities`、`bounds()` 共用）：
  1. **有解析出的激活状态** → 状态集合优先：受管辖成员由状态决定（状态可见即可见，即使源文件
     把它烘焙为 hidden；否则切换状态无法把它重新显示）；未受管辖成员按第 2 条。
  2. **没有描述符，或描述符的 `active_state` 未解析** → 按源 `invisible` 标志回退：hidden 成员
     不绘制、不进包围盒。没有烘焙标志的块仍返回全部成员（绝不猜测隐藏）。

`block_visible_entities` 的 `Option` 语义不变：`None` 仍表示“该块没有可见性描述符”；
有描述符时返回按上述优先级过滤后的成员。

拾取/选择与导出**不**消费 `entity_hidden`：顶层隐藏实体仍可被选中/导出（见 §6 的既有权衡）。

### 写入与校验

`DrawingDatabaseBuilder::set_block_dynamic_visibility(block, vis)` 附加描述符；
`DrawingDatabaseBuilder::set_hidden_entities(entities)` **整体替换**全局烘焙隐藏集合
（去重；空集合清空；不存在的实体在 `finish()` 统一拒绝）。删除实体时该 id 会从集合中移除，
避免删除后残留隐藏行。`finish()` 校验并拒绝：

- 状态名为空或重复；
- 状态可见实体不是该块成员；
- `member_entities` 含非成员实体；
- `active_state` 不是已定义状态；
- 烘焙隐藏 id 没有对应的实体行。

## 3. 状态切换（ChangeSet，增量更新）

```rust
pub fn DrawingDatabase::set_block_visibility_state(
    &mut self,
    block: BlockId,
    state: &str,
    transaction: TransactionId,
    reason: &str,
) -> CadResult<ChangeSet>
```

- 未定义该状态 → `CadError::InvalidInput`，revision 不变（**拒绝未知状态**）。
- 已是激活状态 → 空 `ChangeSet`，不递增 revision，不伪造变更。
- 否则：更新激活状态、`revision += 1`，`ChangeSet.changes` 只含**进入或离开可见集**
  的成员实体，`ChangeMask::GEOMETRY`；两状态共有实体不产生变更。

`ChangeSet` 由宿主消费：受影响的成员实体从**同一数据库快照**重新求值，宿主按
`cad-app` 的增量路径（append / update / retire）或整基重建发布新的 `SceneDelta`。
`cad-scene` 本身没有按变更集失效或删除批次的通道（`SceneCache::publish` 只追加），
所以这里不存在“只有该块的成员批次失效”的缓存机制。重建的仍然只是显示表示：
`ProviderRegistry::build_expanded` 经 `block_entities` 只发出激活状态成员，切换后自动
改变派生几何，**不重新导入底图**（符合 §4.6 / 增量更新）。

## 4. 导入器映射（cad-import-acadrust）

`read_entities` 处理块定义时收集：源 handle 值 → `EntityId`、源 handle 值 →
`!invisible`。两条解析路径：

1. **直接定义路径**：`ImporterBuilder::record_dynamic_visibility` 对每个块调用
   `block_visibility_param_for_def(block_record.handle)`；参数 owner 链直达该块时命中。
2. **INSERT / 匿名求值块路径**：遇 `EntityType::Insert` 时调用
   `record_insert_dynamic_visibility` → `dynamic_visibility_for_insert(insert)`（经
   `AcDbBlockRepresentationData` 解析动态定义块），映射到 INSERT 实际引用的块记录
   （求值后的匿名块）的成员上。这是真实动态块求值文件的主路径。

命中参数时经 `dynamic.rs::map_visibility` 映射为 DB 描述符。

块循环里会把 `member_visible == false` 的源 handle（经 `member_ids` 映射为真正导入成功的
`EntityId`）收集起来；模型空间与图纸空间循环里，源实体 `EntityCommon::invisible` 为真且
`push_entity` 成功时也收集同一 `EntityId`。三个循环结束后**只调用一次**
`DrawingDatabaseBuilder::set_hidden_entities`：这就是源文件的全局烘焙显示状态。它**不改变**
`record_dynamic_visibility` 的行为——能解析出激活状态时描述符依旧是切换状态的权威；解析不到
时，数据库才用这份烘焙集合回退隐藏。

### 激活状态解析（诚实，绝不猜）

对每个状态检查：对所有管辖成员，`状态成员关系 == 实际可见标志`。**恰好一个**状态
一致时取为激活状态；成员/参数缺失、零个或多个状态一致时 → `active_state = None`。

### 稳定原因码

| 原因码 | 含义 |
| --- | --- |
| `dynamic_block_active_state_unknown` | 无法由成员可见标志确定激活状态 |
| `dynamic_block_member_unresolved` | 参数中某 handle 没有对应导入实体 |

出现原因码时映射结果为 `Completeness::Partial(reasons)`，并产生诊断
`import.dynamic_block_visibility`（进入导入完整性聚合）。不可映射为 `Complete`。

## 5. 测试覆盖（本轮）

- cad-db
  - 激活状态只发出该状态成员；未确定状态发出全部成员。
  - 无描述符 + 烘焙隐藏成员：`block_entities` 排除隐藏成员，`bounds()` 不含其几何。
  - 顶层（`SpaceId::Model`）烘焙隐藏实体：`entity_hidden` 为真，`bounds()` 不含其几何。
  - 有解析状态 + 烘焙隐藏：状态集合优先（烘焙 hidden 但状态可见即绘制），切换后
    `ChangeSet` 正确。
  - builder 拒绝不存在的隐藏 id；`set_hidden_entities` 是整体替换（去重、空集清空）；
    删除隐藏实体后 `entity_hidden` 不再为真。
  - 切换产生正确新增/移除实体集合并递增 revision；共有实体不出现。
  - 未知状态拒绝；无参数块拒绝；切到已激活状态为空变更。
  - builder 拒绝非成员实体、重复状态名、未定义激活状态。
- cad-app
  - `visible_model_entities` 同时排除烘焙隐藏实体与隐藏图层实体（override 无法复活烘焙隐藏）。
  - `build_scene_stats_with_vendor` 的模型/图纸空间装配都不为烘焙隐藏实体产生批次。
- cad-import-acadrust
  - 合成 acadrust 文档（程序化 `BlockRecord` + 成员实体 + `BlockVisibilityParameter`）
    经 `record_dynamic_visibility` 记录状态并解析出激活状态 A / B。
  - 合成文档无可见性参数、3 成员中 1 个 `invisible`：导入后 `block_entities` 只含 2 个，
    `block_visible_entities` 仍为 `None`；模型空间 invisible 实体 `entity_hidden` 为真。
  - 真实夹具（`YACR_TEST_DWG`，未设置即跳过）：`*U231` / `*U247` 只绘制 1 个烘焙可见成员，
    且导入的隐藏集合覆盖源 `*Model_Space` 的全部 invisible 实体（按源 handle 精确比较）。
  - 模糊可见标志 → `Partial` + `REASON_ACTIVE_UNKNOWN`，成员不隐藏。
  - 悬空成员 handle → `Partial` + `REASON_MEMBER_UNRESOLVED`。
- cad-representation
  - `build_expanded` 只展开激活状态成员；切换后从同一 DB 重新派生。
  - 未确定激活状态展开全部成员。
- cad-scene
  - 切换 `ChangeSet` 只失效增量成员批次，共有成员保留。

## 6. 明确 Partial / 未支持 / 缺口

- **参数 / 夹点编辑**：线性/极轴/翻转/查表/旋转等参数与动作求值、grip 拖动均未实现；
  仅可见性状态切换。未支持能力显式建模，无占位假数据。
- **未知状态的求值**：当激活状态无法确定时，**按源文件的 `invisible` 标志回退**——烘焙为
  hidden 的成员不绘制、不进包围盒（`Partial` 诊断 `dynamic_block_active_state_unknown` 仍然
  报告）；没有烘焙标志的块保留全部成员。激活状态能解析时**状态集合优先**，状态切换仍然可用
  （即使某成员在源文件中烘焙 hidden，只要新状态包含它就会重新显示）。不做几何伪造；
  UI/CLI 可读 `block_visible_entities` 与诊断原因码。
- **顶层烘焙隐藏实体的既有权衡**：模型/图纸空间的源 `invisible` 实体不再绘制、不进包围盒、
  不进入 CLI `render`/`plot`；但**拾取/选择与 DXF 导出仍然保留它们**（实体行未删除，导出也
  不写 `invisible` 标志）。若将来要“导出保留隐藏标志”或“隐藏实体不可选”，需要独立通道，
  不能把这里当成已支持。
- **端到端 DWG 写入 → 回读**：本轮读取映射用内存合成 `CadDocument` 验证。
  acadrust 的 DWG 写入器是否完整回写 `block_visibility_params` 与
  `block_representations` 的 INSERT 扩展字典链，尚未用真实导出样本核实，因此
  “真实文件端到端”为**待补证据缺口**（见 §7.1 需样本）。
- **状态名唯一性**：以精确字符串比较；源中同名状态会被 builder 拒绝而非合并。
- **不可见实体仍导入**：为支持切换，`all_blocks` 全部成员都进入数据库（即使导入时
  不可见）；激活状态决定绘制，切换无需重新解析底图。源 `invisible` 只作为**显示回退**
  （见 §2 旁表），不删除成员数据。

## 7. 后续工作

1. 真实动态块样本（含可见性参数、多个状态、匿名块求值）验证读取与激活状态解析。
2. 参数/夹点编辑与动作求值（需独立求值器与样本证据）。
3. DWG 写入回流样本，闭合“导入器 ↔ 真实文件”证据链。
