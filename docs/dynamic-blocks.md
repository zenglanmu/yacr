# 动态块可见性（读 + 切换）

`动态块可见性` 支持窗口内，**状态名读取 + 状态切换**已实现；参数/夹点编辑、
参数化求值、未知状态求值是 **Partial/未支持**，明确建模且不伪造几何。

> 状态：**契约 + 合成测试**。端到端 DWG 写入 → 回读的合成样本受 acadrust 写入器
> 能力限制（见 §6），因此本轮以“内存合成 acadrust 文档 + 纯映射函数”验证读取与
> 映射；真实导出样本仍是待补证据。

## 1. 数据来源（acadrust 0.5.5，只读）

- `document.rs`
  - `block_visibility_params: HashMap<Handle, BlockVisibilityParameter>`
  - `block_visibility_param_for_def(def_block) -> Option<&BlockVisibilityParameter>`
    （:3573，沿 `owner_chain_reaches` 找到该块定义可见性参数）
  - `dynamic_visibility_for_insert(insert) -> Option<(Handle, &BlockVisibilityParameter)>`
    （:3589，经 `dynamic_definition_for_insert` 解析匿名块）
  - `dynamic_definition_for_insert(insert)`（:3604）
  - `block_representations: HashMap<Handle, Handle>`（表示对象 → 动态定义块）
- `objects/block_visibility.rs`
  - `BlockVisibilityParameter { handle, owner, all_blocks: Vec<Handle>,
    states: Vec<BlockVisibilityState>, name, description, def_point, ... }`
  - `BlockVisibilityState { name, visible_blocks: Vec<Handle>,
    visible_params: Vec<Handle> }`
- `objects/dynamic_block.rs`：`DynamicBlockObject` / `DynamicBlockData`（含
  `VisibilityParameter(BlockVisibilityParameter)`）。

匿名（求值后）块把“其他状态”的成员标为不可见
（`EntityCommon::invisible`，`document.rs` / `entities/mod.rs:295`）。这是解析
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
  （未确定状态绝不猜测隐藏）。
- `DrawingDatabase::block_dynamic_visibility(block)`、`block_visible_entities(block)`。
- `DrawingDatabase::block_entities(block)` 已按激活状态过滤：只返回该状态可见的
  成员（未管辖成员始终保留）。因此**表示层与包围盒自动只画激活状态**。

### 写入与校验

`DrawingDatabaseBuilder::set_block_dynamic_visibility(block, vis)` 附加描述符；
`finish()` 校验并拒绝：

- 状态名为空或重复；
- 状态可见实体不是该块成员；
- `member_entities` 含非成员实体；
- `active_state` 不是已定义状态。

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

`cad-scene::SceneCache::apply_changes` 按 `sources[].entity` 命中，因此仅该块受影响
的成员批次失效；宿主重建该块显示表示，**不重新导入底图**（符合 §4.6 / 增量更新）。
表示层从同一数据库重新求值：`ProviderRegistry::build_expanded` 经
`block_entities` 只发出激活状态成员，切换后自动改变派生几何。

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
  - 切换产生正确新增/移除实体集合并递增 revision；共有实体不出现。
  - 未知状态拒绝；无参数块拒绝；切到已激活状态为空变更。
  - builder 拒绝非成员实体、重复状态名、未定义激活状态。
- cad-import-acadrust
  - 合成 acadrust 文档（程序化 `BlockRecord` + 成员实体 + `BlockVisibilityParameter`）
    经 `record_dynamic_visibility` 记录状态并解析出激活状态 A / B。
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
- **未知状态的求值**：当激活状态无法确定时保留全部管辖成员（`Partial`），不做几何
  伪造；UI/CLI 可读 `block_visible_entities` 与诊断原因码。
- **端到端 DWG 写入 → 回读**：本轮读取映射用内存合成 `CadDocument` 验证。
  acadrust 的 DWG 写入器是否完整回写 `block_visibility_params` 与
  `block_representations` 的 INSERT 扩展字典链，尚未用真实导出样本核实，因此
  “真实文件端到端”为**待补证据缺口**（见 §7.1 需样本）。
- **状态名唯一性**：以精确字符串比较；源中同名状态会被 builder 拒绝而非合并。
- **不可见实体仍导入**：为支持切换，`all_blocks` 全部成员都进入数据库（即使导入时
  不可见）；激活状态决定绘制，切换无需重新解析底图。

## 7. 后续工作

1. 真实动态块样本（含可见性参数、多个状态、匿名块求值）验证读取与激活状态解析。
2. 参数/夹点编辑与动作求值（需独立求值器与样本证据）。
3. DWG 写入回流样本，闭合“导入器 ↔ 真实文件”证据链。
