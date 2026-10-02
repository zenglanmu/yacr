# Drawing 写事务与绘制/编辑命令（设计契约，F-EDIT）

规范 v2.0 §4.3/§4.6/§4.7：底图导入后只读；**未来底图编辑**必须经同一验证与变更追踪
路径，不得为预留而暴露 unrestricted mutable access。本文件固定本轮实现的接口，供并行
workstream 对齐。权威仍以源码与 `CAD_IMPLEMENTATION_SPEC.md` 为准。

## 1. 核心写事务（`cad-db`，workstream A 落地）

`DrawingDatabase` 新增受控写入口，语义对齐既有
`AnnotationDatabase::apply_annotation_changes` 与 `DrawingDatabase::set_block_visibility_state`：

```rust
impl DrawingDatabase {
    /// 原子、全有或全无。空变更不推进 revision、不伪造 ChangeSet。
    pub fn begin_drawing_transaction(
        &mut self,
        reason: &str,
        id: TransactionId,
    ) -> CadResult<DrawingTransaction<'_>>;

    /// 下一个不冲突的 EntityId/ ObjectId；删除后不复用（带 generation 的语义）。
    pub fn allocate_entity_id(&mut self) -> EntityId;

    /// 直接应用一批已验证变更（事务与历史共用，类比 annotation 路径）。
    pub fn apply_drawing_changes(
        &mut self,
        reason: &str,
        transaction: TransactionId,
        changes: Vec<(EntityId, Option<DbEntity>)>, // None = delete
    ) -> CadResult<ChangeSet>;
}

pub struct DrawingTransaction<'a> { /* staged */ }
impl<'a> DrawingTransaction<'a> {
    pub fn insert_entity(&mut self, entity: DbEntity) -> CadResult<EntityId>;
    pub fn update_entity(&mut self, entity: DbEntity) -> CadResult<()>;
    pub fn delete_entity(&mut self, id: EntityId) -> CadResult<()>;
    /// Move = 对已有实体替换几何（保留 id/object/space/layer/draw_order 除非显式改动）
    pub fn transform_entity(&mut self, id: EntityId, transform: &Transform3) -> CadResult<()>;
    pub fn staged_len(&self) -> usize;
    pub fn commit(self) -> CadResult<ChangeSet>;
    pub fn rollback(self);
}
```

**不变式（必须有测试）**
1. 验证先行：id/object 一致、几何有限且合法、layer/space 引用存在；任一失败则**不改动**任何状态。
2. 提交原子：成功则 revision +1 并返回按插入顺序排列的 `ChangeSet`；失败返回 `Err` 且数据库不变。
3. 空变更：返回 `before == after` 的 ChangeSet，不推进 revision。
4. `ChangeMask`：新增=`GEOMETRY|STYLE`；几何变化=`GEOMETRY`；仅样式=`STYLE`；
   变换=`TRANSFORM`（用于 MOVE）；删除=`Delete`。
5. `EntityId` 分配单调递增，删除后不复用；`scene_identity()` 随之变化使缓存失效。
6. 删除不存在的实体、更新/删除 id 不匹配 → `CadError::Invariant`/`InvalidInput`，非静默成功。

## 2. 应用命令（`cad-app`，workstream B 落地）

在 `Application::execute` 内实现（无 `NotImplemented`），全部 Work-only
（`CommandId::requires_work_mode`），全部经写事务 + 历史（一次撤销一步）：

| CommandId | Payload | 语义 |
|---|---|---|
| `CreateLine` | `Geometry(SemanticGeometry)` 或 `Points([start,end])` | 插入 LINE（模型空间、活动图层） |
| `CreateCircle` | `Points([center, edge])` | 半径 = 中心到 edge 的距离；>0 否则 `InvalidInput` |
| `MoveEntities` | `Move { refs: Vec<SelectionRef>, delta: Point3 }` | 对选中实体做平移，一步事务 |
| `TrimEntity` | `Trim { target: SelectionRef, boundary: [SelectionRef], pick_point: Point3 }` | 以边界裁剪目标的可见段（见 §3） |

- 图层：新增实体进入会话活动图层（`SessionState` 新增 `active_layer: LayerId`，默认 `LayerId(0)`；不存在的图层 → 显式错误）。
- draw_order：新实体取当前模型空间最大 + 1；不改动既有顺序。
- 权限：Viewer 模式返回 `PermissionDenied`（在命令层，不靠隐藏按钮）。
- 撤销：`UndoRecord` 保存 before/after 实体；`Redo` 等价重放。历史可用性随之更新。
- 发布：命令成功后 `ChangeSet` 交给宿主（场景重派生，不重解析底图）。

## 3. TRIM 的诚实边界

TRIM 的通用解析裁剪（任意曲线对任意边界）是大型计算几何。本轮实现**明确子集**并用
`Partial`/诊断标注，绝不假装完整：

- 支持 **LINE 被 LINE/LWPOLYLINE 直线段** 裁剪：用 2D 参数解算与边界的交点，选择
  `pick_point` 所在的一侧保留，裁剪掉另一段（一条 LINE 完全落在边界内则整体删除）。
- 目标/边界含曲线圆弧、SPLINE、INSERT 实例、Opaque 等 → 返回 `CadError::Unsupported`
  或 `Partial` 诊断，**不改库**。
- 交点容差用 `TolerancePolicy`；退化/共线/无交点 → 显式报告。

## 4. UI 与宿主（workstream C 落地）

- Ribbon 绘制/编辑组由禁用占位变为真实工具：LINE（两点）、CIRCLE（中心+半径点）、
  MOVE（先选择再拖动/输入位移）、TRIM（选目标 + 选边界）。
- 输入状态机复用 `cad-app::input`：单指绘制、双指不提交；预览走既有
  `CadView::set_*_preview` 叠加层；确认恰好一次事务，取消零事务。
- 删除 `pending("ui.command.editing")` 与 `pending("ui.ribbon.draw_modify")` 占位注释，
  在 `docs/ribbon-ui.md` 记录真实可用性与仍未支持的图元。
- 目录新增文案（两语言）：工具名、步骤提示、错误（未知图元/无交点/只读模式）。

## 5. 验收

- `cad-db`/`cad-app` 单测覆盖 §1 不变式与 §2 每条命令（含 Viewer 拒绝、空事务、
  撤销重做、TRIM 子集与显式 unsupported）。
- `cargo check --workspace --lib --target wasm32-unknown-unknown` 通过（UI 编译门）。
- 主控做无头浏览器端到端：创建一条线/圆后画布像素变化，撤销后回到基线。
