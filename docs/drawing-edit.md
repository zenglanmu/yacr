# Drawing 写事务与绘制/编辑命令（设计契约，F-EDIT）

> **历史文档（Slint 桌面已退役，2026-10-16）**：本文描述的是**迁移前**的 Slint 桌面/web 宿主；
> 其中 `cad-ui-slint`、`yacr-linux`、旧脚本命令均已退役（部分脚本改为非零 NOT IMPLEMENTED）。
> 当前架构与构建见 `docs/architecture.md`、`docs/react-linux-app.md`、`docs/build.md`、
> [ADR 0010](adr/0010-react-linux-host-replaces-slint.md)。下文仅作历史记录。

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
7. **样条必须是内部自洽的 NURBS**（2026-10-26 增补，`validate_geometry` 的 `Spline` 分支）：
   **非空** `knots` 必须恰有 `control_points.len() + degree + 1` 个（`spline.knot_count`）；
   **非空** `weights` 必须每个控制点一项（`spline.weight_count`）；非空 knot 向量必须**非递减**
   （`spline.knot_order`，消息点名相邻两个索引与数值）。既有的 degree ≥ 1、控制点数 ≥ degree+1、
   有限性、正权重检查保持在前面，不被新码遮蔽。

   **两条被认可的简写**（各自独立判定，互不遮蔽）：`knots` 为空 = "clamped uniform"，
   `weights` 为空 = "全 1"。它们不是宽松容错，而是本仓库真实产生的形态：
   `cad-domain/src/multileader.rs::path_geometry` 与
   `cad-import-acadrust/src/dimension/multileader.rs::multileader_path` 都用
   `knots: Vec::new(), weights: Vec::new()` 构造 MULTILEADER 引线的显示样条；
   `cad-geometry::tessellate_spline` 会在空 knot 时回退到 `clamped_uniform_knots`，
   `NurbsCurve::new` 会把空权重展开成全 1。**不做任何补齐/均匀化/修复**，拒绝先于任何变更。

   该规则对**派生显示几何**递归生效（`Compound`/`Dimension`/`Hatch`/`MultiLeader`），
   这正是必须接受这两种简写的原因：拒绝它会让导入的 Spline 引线 MULTILEADER 变得不可编辑。
   注意导入构建器走的是宽松的 `DrawingDatabaseBuilder`（记录来源、报告完整性），
   因此该不变式只约束**受校验的写路径**，不拒绝导入本身。
   `cad-app` 的夹点模型（`crates/cad-app/src/grips/mod.rs::spline_is_consistent`）对无法索引的
   定义给出**零夹点**，它是纵深防御（覆盖未经受校验写入的几何与逐次拖拽），不是第二套校验。

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

## 3. TRIM / EXTEND 的诚实边界

通用解析裁剪（任意曲线对任意边界）仍是大型计算几何；当前实现一个**明确子集**并用
`Unsupported`/`InvalidInput` 与诊断标注，绝不假装完整：

- 目标：**LINE / ARC / CIRCLE**（EXTEND 只接受 LINE / ARC——圆没有端点）。
- 边界：**LINE / ARC / CIRCLE / ELLIPSE / LWPOLYLINE**（直线段或带 bulge 的弧段），可任意混用。
  LWPOLYLINE 的 bulge 段经 `cad_geometry::bulge_arc_geometry` 还原为解析 ARC 后求交，
  切点来自 `GeometryEngine::intersect_local`（与对象捕捉同一个解析圆锥曲线求交）；只有
  ARC×ELLIPSE、ELLIPSE×ELLIPSE 这类引擎不解析的组合退回其采样近似。
- 裁剪语义：切点按目标自身参数排序（LINE 用 `t ∈ [0,1]`，ARC/CIRCLE 用自身 sweep 上的
  `u ∈ [0,1]`，一个周期参数域一套算法），保留含拾取点**投影**的那一段（见下方拾取语义）。ARC 复用原
  圆心/法向/半径并重算 `start`/`sweep`，**保留原 sweep 符号**；CIRCLE 变成一段 ARC，
  需要至少两个切点（周期参数域上“恰好一个切点”仍拒绝）；完全落在闭合边界内的目标
  整体删除（含 CIRCLE）。**内侧判定按边界自身平面**：闭合折线先做平面距离检查，再在
  该折线自己的平面坐标系里做射线判定，且要求严格内缩超过容差（删除是破坏性操作，
  边界附近一律不删）；圆/椭圆走解析内侧判定。
- 拾取语义：`pick_point` **先投影到已解析的目标上**，再用投影点定段（LINE 取垂足；
  ARC/CIRCLE 取 `center + radius * normalize(pick − center)` 在目标自身平面内的径向投影）。
  宿主是用**拾取框**解析出目标的，点击本来就可能离曲线几个像素（只有对象捕捉开启时才有捕捉点），
  所以“点在实体附近”必须能工作；按曲线距离拒绝反而会否掉宿主已经解析出的这次拾取。
- 拾取拒绝只剩两种真正歧义的情形：①投影落到目标自身参数域**之外**（LINE `t ∉ [0,1]`，
  ARC 超出自身 sweep）——**显式拒绝并点名“投影”**，绝不夹到最近端点（夹紧等于替用户决定保留
  哪一段）；②ARC/CIRCLE 拾取点的**径向方向为零**（点落在目标轴线上，例如圆心）——没有角度可投影，
  显式拒绝。CIRCLE 的参数域是周期的，任何角度都在域内。这两条都发生在**解析之后、提交之前**，
  拒绝时零事务、零层改动。
- 延伸语义：LINE 沿自身无限直线、ARC 沿自身圆，到拾取端**前方最近**的交点；只增不减，
  “延伸哪一端”由拾取点**在曲线上的投影落点**判定（不是点到曲线的距离），
  已接触/已穿过/前方没有交点都是显式拒绝。ARC 的延伸量**上限半圈**：更远的交点属于
  “只能绕远路到达”，显式拒绝而不是把弧展开成近一整圈。`|sweep| > 一周` 的 ARC 目标
  在入口就拒绝（没有唯一周期，切点会折错位置）。
- 近似诚实性：边界对在引擎里没有解析解时（ARC×ELLIPSE、ELLIPSE×ELLIPSE、非共面的
  圆族）仍走引擎的**采样**回退，但成功结果会**额外**产出一条 `drawing.trim` /
  `drawing.extend` 诊断声明该切点是近似值；只有真正贡献了切点的边界对才会被点名。
- 目标或边界含 SPLINE / INSERT / Opaque / Mesh → `CadError::Unsupported`，**不改库**。
- 交点容差用 `TolerancePolicy`；退化/共线（重叠）线/无交点 → 显式报告，且**先解析后提交**，
  所以拒绝时一次事务都不开。

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

## 6. 合并后主控验收（2026-10-03）

基于 `39077ec`，修复三个宿主/UI 集成缺口：

1. 命令修改数据库后，桥的 `IncomingDocument` 仍指向旧 Arc。新增
   `CadView::sync_drawing`，Web/Android 的状态漏斗发布权威快照，包含撤销重做；
   同 Arc 推送不更新快照。桥新增快照替换/复用/关闭契约测试（本机仅 wasm/Android 编译）。
2. 绘制取点仍进入宿主选择/导航，MOVE 会丢失选择、取消后可能残留选择高亮。
   活动绘制工具独占主指针，保留滚轮及非主按钮的既有导航路径。
3. 绘制 sink 吞掉执行错误并返回成功。两宿主现在向 shell 返回真实 `CadResult`，
   错误不会结束捕获或伪造成功。

`scripts/check-web-drawing.mjs` 在真实 Chromium 153、SwiftShader/WebGL2 下分别验收
LINE、CIRCLE、MOVE、TRIM（直线交直线）。全部操作来自 Slint 命令输入与画布取点，
不是直接调用测试 API。确认后工具行关闭，底图 CAD 区域像素改变；撤销后的解码像素
SHA-256 与各场景基线完全相同。MOVE 锚点为空白区域，验证选择不丢失；LINE/CIRCLE
确认前及取消保持实体计数，重做恢复计数；TRIM 圆目标拒绝后捕获保留、实体计数不变，
取消恢复空闲。重做像素、多点触控绘制、复杂裁剪和 Android
运行未在本轮新增验收。证据 `/tmp/opencode/yacr-drawing-final/{LINE,CIRCLE,MOVE,TRIM}/`。

核心串行 963 passed / 0 failed / 1 ignored；JS 27 passed；workspace wasm lib、
UI/Web wasm 测试编译、UI/Android aarch64 测试编译、严格核心/app-web clippy、fmt、
架构/i18n/fixtures/workflows 通过。浏览器环境限制及复跑命令见 `validation-web.md` §9。

范围仍为受控内存编辑：不写回 DWG、不提供圆弧/样条/多段线创建，也不把 TRIM 子集
宣称为完整 CAD 编辑器。Android 本轮只有编译证据；真机/WebGPU/生产 URL 未重验。

### 6.1 发布前输入安全修复

Web 原生 TouchEvent 路径现在在第二触点落下及 touchcancel 时显式取消 shell 绘制，
不是仅取消 `SessionState` 中另一个工具。成功换图纸时 Web/Android 取消旧捕获，失败/
取消打开则仍保留原文档。TRIM 使用 `cad_app::picking::pick_tolerance` 的逻辑像素半径，
随相机缩放派生世界容差；此前固定 0.5 单位只接受极其精确的屏幕点。

新增 Node 触点取消契约与 `scripts/check-web-drawing-safety.mjs` 无头端到端，确认第二触点
尚未移动即取消、touchcancel 清空、换图纸后旧确认不插入实体。四工具与安全脚本在完整
99 字体发布候选中通过；JS 28 passed。发布与最终回归边界见 `validation-web.md` §10。
