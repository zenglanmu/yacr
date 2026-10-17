# ADR 0012：场景空间索引与 Pick / Snap / Selection / 局部重绘架构

- 状态：**已接受（Accepted，2026-10-17）**，范围**定性**：本 ADR 固化架构、术语与不变量，明确**否决项**
  与**延后项**。**不包含**任何实现完成声明：本 ADR 未运行任何测试；各项实现与证据在各自合入时另记
  （见 `docs/handoff.md`）。范围**有界**：Linux 宿主（React/Tauri + X11/原生 Wayland）的 2D 正交交互域；
  3D 透视拾取、跨平台、vendor、精度与性能结论均 **NOT RUN**。
- 相关：`CAD_IMPLEMENTATION_SPEC.md` §3.3、§4.4、§4.6、§4.7、§4.8、§8.1、§8.2、§8.4、§8.5、§16.1、
  §16.2、§18.2、§18.3；`docs/render-optimization-backlog.md`；`docs/picking-3d.md`；
  `docs/picking-gpu-id-pass.md`；`docs/architecture.md`；`docs/interaction.md`；`docs/handoff.md`。
- 前身/依赖：ADR 0007（entity-as-data 渲染器）、ADR 0008（SDF 圆/弧渲染）。ADR 0007 的静态
  `GpuBatch` + `commit_upload` 前缀复用是本 ADR 的硬前提。
- 裁决关系：本 ADR 取代一份 2026-10-17 提出的外部设计提案（46 节 + 追加修正）。该提案的**三段纪律**
  方向正确并被采纳；其 §16、§36、§47.11 三处被**改写**，其余多处被判定为**已存在**或**过度设计**。
  **冲突时以本 ADR 为准。**

## 背景

渲染侧的实体↔draw call 解耦已完成（ADR-0007）：draw 数 10,474 → 797、122,006 → 7,627；静态几何在
pan/zoom 全程零重传（见 `docs/validation-render-perf-2026-10-15.md`）。规模继续增长时，瓶颈预期从
GPU 提交转向**CPU 侧场景组织与空间查询**：每帧遍历、可见性判断、包围盒更新、脏区域、Pick/Hover/Snap
遍历、框选扫描、Block/大 Polyline/Hatch 层级查询、拖动期索引抖动、局部变化引发全场景重处理。

对现状的核查结论（决定本 ADR 的具体形态）：

- **已有**：`cad-spatial` 提供 `SpatialIndex` trait（`lib.rs:44`）与 `GridSpatialIndex`（`lib.rs:57`，均匀
  X/Y 网格；注意其 `ray_candidates` 实为对全部条目的 O(n) 线性扫描）；`cad-spatial/src/pick.rs` 已是
  **broad phase + CPU 精确命中**的分层实现（`pick.rs:3` 明示"the CPU precise-pick stage that follows the
  broad phase"；`pick.rs:439-441` "Cheap conservative rejection… Unknown/invalid bounds deliberately fall
  through"；`pick.rs:623` "deliberately conservative, low-cost local bound"）；`cad-app/src/picking.rs`
  提供 screen→ray→**世界空间**容差入口（`pick_tolerance` `:48`、`pick_at_screen` `:313`）；`cad-measure`
  的 snap 引擎完整（`snap.rs:26` `SnapKind{Endpoint,Midpoint,Center,Quadrant,Perpendicular,
  LocalIntersection}`、`:158` `collect_candidates`）；`cad-db` 具备 `Revision`、带 revision 的
  `BoundsCache`、`ChangeMask`/`ObjectChange`/`ChangeSet`（`change.rs:8/73/80`，含 `for_entity_update`
  `:48` 与 `follows()` `:91`）；`FIT/ZOOM EXTENTS` 已实现并复用 `drawing.bounds()`。
- **半接线**：`filter_by_index`（`picking.rs:351`）已写好却**只在测试使用**，生产 `pick_ray` 每次全量遍历
  `database.model_space()`（`picking.rs:109`）；`SceneCache::apply_changes`（`cache.rs:90`）**只在测试调用**，
  生产路径每次**全量 rebuild**；`CacheKey{identity,entity,revision,style_version,font_version,lod,space}`
  （`cad-scene/src/batch.rs:8`）**定义即废弃**（全仓无第二处引用）。
- **缺失**：任何 R-tree/BVH/第三方空间库；`GeometryBounds` 与 `RenderBounds` 的区分（全仓只有一种
  `Bounds3`，`cad-domain/src/lib.rs:75`）；视口/视锥剔除（每帧遍历 `Renderer.batches` 全量，`lib.rs:1146`，
  渲染器构建期算出的 AABB 用完即弃）；dirty region / tile / scissor / damage（全仓零命中，只有整帧
  `FrameInvalidation`）；`Window`/`Crossing`/`Lasso` 框选（零类型）；实时宿主的鼠标 pick/hover/snap
  （DOM 路径只产出 `Pointer`/`Scroll` → pan/orbit，`SelectAt` 从 DOM 不可达）。

## 决策

### 1. 五级空间过滤模型（统一术语）

`L0 Scene/Viewport`（排除视口外）→ `L1 Chunk`（粗粒度裁剪）→ `L2 Entity Bounds / R-tree`（**broad phase**，
只产出"可能命中"）→ `L3 PickProxy / Primitive Bounds`（局部 **narrow phase**，排除实体盒内大量空白）→
`L4 Exact Geometry`（精确命中 / 最近点）→ `Ranking`。

此前提案中"三级/四级/五级"三种说法作废，一律用本五级。Render、Pick、Snap、Selection 共用同一模型。

### 2. 三段纪律（规范性不变量）

**BBox 负责找候选；Geometry 负责决定命中；screen-space distance 负责决定最终选谁。**

`[BBox intersects query] ⟹ 命中` 在任何路径下**都不成立**，永远只是候选条件。当前 2D 正交域下"世界射线
容差"与"屏幕距离"等价（线性缩放），故排序语义不变（见 §9）。

### 3. 早判不对称性（本轮最重要的修正）

三类查询的 bounds 早判规则**不同**，必须分别实现并写测试：

- **Pick（射线）**：`ray 与 (bounds ⊕ tol)` **不相交 ⟹ 必不命中**（可靠早退，因为 `geometry ⊆ bounds`）。
  **未知/无效/缺失 bounds 必须落回精确判定，不得早退** —— 这是既有 `pick.rs:439-441` 的语义，**保持**。
- **Window Selection**：`rect ⊇ bounds ⟹ 完整包含 geometry`（**可靠早接受**，即使 bounds 松弛保守，
  因为"装得下超集就装得下子集"；零面积、零长度、geometry==bbox 等退化情形同样成立）。
  **`¬(rect ⊇ bounds)` 不得判 reject**：本仓 bounds 是**保守**的（曲线走保守球扩张；`local_pick_bounds`
  对 text / 带凸度 polyline / mesh 返回 `None`），紧贴图元的选框可能装不下保守盒却装得下真实几何 →
  会**误拒已完全选中的图元**。正确写：`contains → ACCEPT`，**否则落回精确判定**。
- **Crossing Selection**：`rect ∩ bounds = ∅ ⟹ 不相交`（**可靠早退**）；`rect ∩ bounds ≠ ∅` **仅作候选**，
  必须再走精确几何相交。

**前置条件**：任何 bounds 早判都必须基于**当前** geometry 版本（§4.6 / §8.4）。过期 bounds 会破坏上述
不变量，因此早判点必须能证明 bounds 与几何版本一致。

### 4. Bounds：只保留一套权威

- 结构上**只有 `Bounds3`**（f64）。**不新增** `Aabb2`，**不**把 f32/`glam` 引入 CPU 查询层（§3.3、§8.2
  要求 CPU f64；`glam` 仅存在于 `cad-render-wgpu`）。
- 几何包围盒的**权威**是 `cad-db::compute_bounds`（解析图元 + 曲线保守球扩张，`drawing.rs:801`，按
  `Revision` 缓存）——**非** GPU/顶点派生。
- **采纳 RenderBounds 概念，但否决其尺度参数化形式**：RenderBounds = **样式派生的像素边距常量**
  （线宽/AA/highlight），在**查询时**按 `world_per_px` 展开，与既有 `pick_tolerance`（`picking.rs:48`）和
  `conservative_world_bounds`（`pick.rs:772`）同构。**否决** `render_bounds(&self, view_scale)` 这类
  尺度参数化实体方法：它使渲染包围盒随 zoom 变化，破坏按实体的版本缓存，并迫使 pan/zoom 重建索引，
  与本 ADR §38 语义（相机变化不重建）自相矛盾。
- 渲染器构建期的**顶点派生 AABB**（`plan.rs` 的 `batch_world_aabb`/`piece_world_aabb`）保持**渲染器内部**
  用途（流分块、剔除），**不得**外泄到 pick / selection / zoom-extents / 空间索引。两套 AABB 允许数值
  不同（db 侧保守、渲染侧精确），但**角色必须分离**。

### 5. 不新增第二套失效真相

**否决** GEOMETRY_DIRTY/BOUNDS_DIRTY/RENDER_DIRTY 新脏标记体系，**否决** `EntityChange` 逐实体事件类型。
增量失效**唯一**由 `ChangeSet` + `ChangeMask` + `Revision` 驱动（§4.6、§8.4，铁律 4）；§4.6 明令批量
（"十万个对象变化不产生十万个 UI 回调"），逐实体事件即便聚合后**就是** ChangeSet。

补齐**已定义但未使用**的 `CacheKey`（`cad-scene/src/batch.rs:8`，须含文档身份、实体/定义版本、样式、
字体、细分等级、布局上下文，§8.4），并把 `SceneCache::apply_changes`（`cache.rs:90`）接入**生产**路径。

### 6. 标识不回退

所有拾取/选择/快照结果类型必须携带 `SelectionRef`（`DocumentId` + `EntityId` + `InstancePath` +
可选 `SubElementId`，§4.4、§4.2），**不得**退化为裸 `EntityId`。既有 `PickHit.source: SelectionRef`
（`cad-spatial/src/lib.rs:25`）保持；`Vec<EntityId>` 形式的中间候选不得跨模块边界传播。

### 7. 空间索引：chunk 网格 + local rstar

- **结构**：顶层为固定/松弛（loose）网格 chunk（复用/扩展 `GridSpatialIndex` 的网格算术）；每 chunk 内以
  **`rstar`** 作 local 索引，`bulk_load` 重建式 + 不可变快照。查询按"顶层网格 → 命中少数 chunk →
  各 chunk 局部索引 → 候选实体"两级进行。**否决**"全局单棵 R-tree 先行、以后再分块"的中间迭代。
- **依赖落点**：唯一允许落点 `cad-spatial`（只依赖 `cad-domain`/`cad-geometry`）。`rstar` **0.13.0**
  （2026-05-24；`MIT OR Apache-2.0`；MSRV 1.85 ≤ 工具链 1.99；`no_std`；依赖仅 `num-traits`/`heapless`/
  `smallvec`；默认 feature 为空）。引入须更新 `Cargo.lock`（`--locked` 流程）并按 §10.2 完成版本/来源/
  许可独立核查。`check-architecture.py` 不禁该依赖；`cad-scene`/`cad-app`/`cad-measure` 通过
  `cad-spatial` 消费，不得各自直接依赖。
- **超大图元策略（规范 §8.1 强制要求）**：`超长线、大填充和大型插入应有单独策略，避免跨块复制引起
  数据膨胀`。实现为**松弛边界（2×，使尺寸 ≤ 单元尺寸的图元只落一格）** + **超大图元单独列表 / 子 AABB
  拆分**；**禁止**"插入每个相交格"的做法（一个城市街区块填充会跨上万格复制）。
- **并发**：单写多读；每 chunk 一个 owned 快照（`Arc`）发布 + 原子替换；**不**做 `RwLock<Node>` 节点级
  细粒度锁。快照版本**取自已构建的数据库 `Revision`**，陈旧判定复用 `ChangeSet::follows()`（`change.rs:91`）
  与 `TaskStamp`（`SceneCache::publish` `cache.rs:474`）。**否决** camera_version / spatial_version /
  visibility_version 之类并行版本计数器：任何能与 `Revision` 失同步的版本都会导致 §4.6 禁止的"长期
  保持错误缓存"。
- **`ViewportSpatialContext`**：本 ADR **不**引入该结构。视口过滤初期实现为**纯函数/谓词**（视口 bounds ∩
  可见性过滤），当出现**两个以上真实消费者**时再升格为结构。可见集**不得**每帧物化为 `Vec<EntityId>`
  （§8.1"不逐帧全图扫描"）——用惰性/流式候选查询，而非缓存 chunk id 列表。
- **多核**：本轮**不**引入。渲染会话属主线程、`cad-platform` 契约无 futures/无 `Send`，rayon 进渲染路径
  与 ADR-0007 及平台契约冲突。串行先行，按实测再议（§8.5 要求 profiling 先行）。

- **实施状态（2026-10-18，记录，不改本决策）**：本节的**顶层 chunk 网格 + 超大图元策略**已接入**生产
  拾取路径**（`cad-app::picking` 的 `PickIndex` + `cad-spatial` 的 `ChunkGridIndex`，commit `13f7808`）。
  §7 字面要求的**每 chunk `rstar`** 随后**已实现并实测**（`rstar` **0.13.0** 仅接入 `cad-spatial`，
  `bulk_load` 重建式 + 不可变快照；§10.2 版本/来源/许可核查按解析后的 `Cargo.lock` 与已发布 crate
  元数据完成：crates.io、`MIT OR Apache-2.0`、`no_std`、默认 feature 为空、MSRV 1.85），并在
  `fixtures/complex-test.dwg`（**723,587** 展开条目）上以 release 拾取 harness 对**旧扁平 per-chunk
  列表**做**单元尺寸扫描**（约 11 → 65536 条目/chunk）。结论：per-chunk R-tree **在每个尺寸上都是净
  回归**（构建时间、热拾取中位/最差、RSS 全面变差，候选数**完全相同**）。**原因**：拾取容差与条目间距
  同量级，被访问 chunk 的查询包络覆盖其大部分，R-tree 几无可剪枝却仍付每棵树的开销。
- **落地结构（commit `c4b66c6`）**：**粗网格**（目标 **512** 条目/chunk ⇒ **1,419** chunk，而非
  65,637）+ **扁平 per-chunk 列表**；`SMALL_CHUNK_MAX` 已移除。编排者**独立复测**：构建 **504.8 ms**、
  热拾取中位 **0.290 ms**、最差 **33.1 ms**、索引 RSS 增量 **115,812 kB（~113 MB）**、oversized
  **598**（原 2,066）、候选不变。**`rstar` 依赖未落地**（最终树 `Cargo.toml`/`Cargo.lock` 未动），
  §7 其余要求全部保留：松弛 2× 边界 + 单格放置（不跨格复制）、独立 oversized 列表、build-once 不可变
  快照、`Revision` 键控版本（无并行计数器）、保守超集语义、`YACR_PICK_EXHAUSTIVE` 穷举回退；随机差分
  测试原样通过。
- **这是一次由实测驱动的偏离**，本 ADR 自身的门允许（§10 S6「按实测再评估」、§8.5 profiling 先行）。
- **残余缺口（如实记录）**：§7 要求「复用/扩展 `GridSpatialIndex` 的网格算术」，但拾取 chunk 布局
  **复制**了该算术而非共享；`GridSpatialIndex`/`SpatialIndex`/`PendingSpatialIndex` 在 `cad-spatial`
  之外**无消费者**（已 grep 全 workspace 确认），故本轮只改了 `chunk.rs` 的副本，`lib.rs`/`pick.rs`/
  `tests.rs` 未动。抽公共 helper 被**有意**略过，以免改动他人正在调整的尺寸。证据见
  `docs/validation-pick-index-2026-10-17.md`（含扫描表、偏离项与内存足迹）。

### 8. 交互语义

- **圆/弧拾取按 AutoCAD 惯例：仅圆周可选**（`|d − r| ≤ tol`）；"内部可选"必须**显式**开产品选项并配测试。
  合成 conic 若走解析命中，须与既有离散路径做 id parity / 像素 fence 对齐（ADR-0008）。
- **窗口/穿越框选**：新功能，严格按 §3 不对称性实现；第一版**串行**（不做 chunk 并行）。
- **`trait Pickable`**：采纳（采纳方决定引入），**但签名必须三态**：
  `fn hit_test(&self, q: &PickQuery) -> PickVerdict`，其中
  `PickVerdict::{ Hit(PickHit), Miss, Unsupported(&'static str) }`。**禁止**裸 `Option`——它会把"未支持"
  静默成"未命中"，削弱既有 `PickOutcome::Unsupported` / `SkippedGeometry` 的机读能力记账（铁律 2）。
  **不**引入独立的 `PickProxy` 类型层级：`PickItem{source, geometry, transform, geometry_source}` +
  `local_pick_bounds` **即** pick proxy 与 proxy bounds，文档中正式采用该命名即可。
- **Hover 缓存**（Pick Cell / LastCandidates）延后，不在本轮。

### 9. 容差与排序

- 容差一律由 `TolerancePolicy` 派生（`pick_tolerance` / snap 派生），**禁止**在查询类型里自造
  `screen_tolerance`/`world_tolerance` 字段绕开策略对象。权威值为 `interaction_logical_pixels = 12.0`
  （`cad-domain/src/lib.rs:421`）。
- **排序保持既有语义**：射线**深度优先** + 垂直偏移 tie-break（`PickHit` `lib.rs:25`）。理由：3D 下
  "投影最近"会让远处但投影压在光标下的图元压过真正最近的表面。**暂不**切换为屏幕距离判定/排序 ——
  正交 2D 下两者等价，切换是纯返工且会改变既有 pick parity 语义；待 3D 透视拾取真正落地时重新评估
  （届时**判定**可切为深度缩放容差，**排序**仍保持深度优先 + 屏幕距离 tie-break）。

### 10. 实施顺序（垂直切片优先）

以**用户可见价值**而非基础设施完整度排序；每片必须自带契约测试与证据。

1. **S1 点选闭环**：DOM → `SelectAt` → `pick_at_screen` → 高亮 + 属性面板。机制已九成就绪
   （`pick_at_screen`、`InteractionRequest::SelectAt`、`select_all`、`fit_*`、GPU id-pass 与 parity 测试
   均在）。**最高性价比，且是后续一切交互的前提。**
2. **S2 Hover**：`MoveCursor` → 节流拾取 → hover 高亮。**这是对全量拾取路径第一次真实压力测量**，也是
   启动 S4 的诚实触发条件。
3. **S3 Snap 接线**：db 解析 `SnapTarget` → `collect_candidates`/`best_candidate` → 标记（引擎已存在，
   宿主未接）。
4. **S4 索引与增量失效接入热路径**：落地 §7 的 chunk + local rstar，把 `filter_by_index` 接入生产的
   `pick_ray`，把 `SceneCache::apply_changes` 接入生产，补齐 `CacheKey`。由 S2 实测驱动。
5. **S5 框选**：Window/Crossing（串行，按 §3）。
6. **S6 按实测再评估**：多核、子元素（分段/字形）索引、Block 局部索引、超大图元子 AABB 深度、
   Pick Cell / Snap cache。
7. **Dirty Region / Tile：排最后**。`docs/render-optimization-backlog.md` 的 P3 自述"最低优先级；先测量"，
   且当前整帧 cpu 仅 1.88 / 5.91 ms，**没有**实测证据支持做 tile；tile 还会与选择/悬停标志色区域
   （必须重绘，不能复用旧像素）和 progressive 帧缓存交互，并引入第二套 damage 真相。

## 备选方案（否决）

- **全局单棵 rstar 先行、以后再分块**：否决（采纳方决定直接做两级结构）。
- **`DynamicEntitySet` 处理拖拽**：否决。§4.7 已规定拖拽只改 `PreviewState`、确认时一次提交；拖动中
  图元由工具自身持有，无需从索引重新拾取；`SpatialIndex::update`（`lib.rs:46`）本身已具增量插入/删除
  能力。每鼠标事件改索引是自找的，且在每事件改几何时违反 §4.7。
- **chunk 级并行渲染 / 框选**：否决（见 §7 多核）。
- **`SpatialQueryService` trait 层**：否决。对已有 `SpatialIndex` 再包一层且只有一个实现的 service trait
  与 §16.4 反对的 service-locator 式分层同质；改用 `cad-spatial` 自由函数 + `cad-app` 包装。
- **每帧重新收集 Primitive / 合批（提案 §16）**：否决。与 ADR-0007 的静态 `GpuBatch` + `commit_upload`
  前缀复用、§8.1"相机变化不重传全部顶点"以及铁律 4 冲突。相机变化只允许重算**已存在 batch 的绘制子集**。
- **`render_bounds(view_scale)`**：否决（见 §4）。
- **现在就做 Dirty Tile**：否决（见 §10.7）。
- **独立 `PickProxy` 类型层级**：否决（见 §8）。
- **现在切换为屏幕距离判定/排序**：否决（见 §9）。
- **超出规范范围的 snap 类型（Tangent / NearestPoint）与字形级文字拾取**：不作为本轮设计内容；实现前
  必须显式建模为 `Unsupported`（铁律 2），不得静默降级。
- **新脏标记体系 / `EntityChange`**：否决（见 §5）。

## 后果与权衡

- 优点：术语与不变量统一；明确的早判不对称性可防止 Window Selection 误拒；索引结构一次到位且带规范
  强制的超大图元策略；不引入并行失效真相；实施顺序先产出用户可见价值并以其驱动基础设施。
- 代价/限制：本 ADR 显式**改写**了原提案的 §16、§36、§47.11，并延后了大量提案内容；被延后项在实测
  证据出现前**不得**当作已设计完成。新增 `rstar` 依赖需 `Cargo.lock` 更新与许可核查。
- 证据边界：本 ADR **未运行**任何测试、门禁或渲染；"已有/半接线/缺失"均为源码核查（含 `pick.rs:3`、
  `pick.rs:439`、`pick.rs:623`、`picking.rs:109/351` 等具体位置）。任何实现完成声明必须另附本轮证据，
  不得引用本 ADR 或历史轮次证据冒充。

## 显式延后（NOT NOW）

- Dirty Region / Dirty Tile / scissor 局部重绘（backlog P3；需先有 P1 剔除与实测帧成本分布）。
- `ViewportSpatialContext` 结构体（待 ≥2 真实消费者）。
- Pick Cell / Snap 候选缓存、Break 型 hover 节流。
- 多核（chunk 级并行、`par_iter` 精确框选）。
- 子元素索引阈值（含 `segment_count < 64` 线性 / `≥ 64` 建索引一类判定的具体阈值）、`MAX_PICK_SEGMENTS`
  （`pick.rs:53`，当前 8192）的调整。
- Block 层级局部索引（当前 INSERT 展开 + `MAX_INSTANCE_DEPTH=32` + 每 insert `InstancePath` 已满足
  §18.2 的 per-insert 标识要求）。
- 3D 透视拾取下的容差判定/排序切换。
- GPU ID 拾取作为补充路径接线（保持 CPU 主路径，§8.5）。
- GPU 侧可见性/剔除（backlog P6/P7）保持为**能力门控的、渲染器内部的 bonus**——**永不**是架构依赖、
  **永不**是 pick 权威、**永不**是第二套失效真相；且在 **GPU 帧时间测量存在之前**不得构建。

## 未决（需要产品/证据输入）

- 圆"内部可选"是否作为产品选项（默认仅圆周）。
- 超大图元"单独列表"与"子 AABB 拆分"的取舍阈值。
- `rstar` 引入时的 `Cargo.lock` 更新与许可核查结论。
- S2 hover 实测帧成本分布 → 决定 S4 是否立刻落地两级索引。
