# 生产 CPU 拾取接入空间索引 验证（2026-10-17）

**目的**：记录一次**用户报告**的交互停顿（在 `fixtures/complex-test.dwg` 上点选一个实体停顿约一秒）的
**根因机制**、**渲染器/应用层改动**、**release harness 实测 before/after**、**随机差分测试的输入空间**、
**差分测试抓到的一个真实几何 bug**、**偏离项**与**未测边界**。改动落在 `crates/cad-spatial`
（新 `chunk.rs` 的 `ChunkGridIndex` + `pick.rs` 的保守包围盒）与 `crates/cad-app`（`picking.rs` 的
`PickIndex` 与索引化拾取路径），实现 commit `13f7808`，是 ADR-0012 §10 **S4 的拾取部分**。
**未**改宿主协议、**未**改 UI、**未**改 `Cargo.*`、**未**新增诊断码。

**这不是**性能验收、**不是** vendor/视觉结论，也**不是**宿主交互判定（见「未测 / 边界」）。单机、
单会话。口径与 `docs/validation-pick-table-incremental-2026-10-17.md`（提交路径 pick 表增量）、
`docs/validation-uniform-encode-skip-2026-10-17.md`（N1e 后续 uniform/encode）、
`docs/validation-plan-cost-2026-10-17.md`（N1e 本体）**不同**，各组数字**不得混算**。选择/pick 表与
GPU id pass 的上游契约见 `docs/picking-gpu-id-pass.md`；索引架构见
`docs/adr/0012-scene-spatial-index-and-interaction.md`。

## 环境与授权边界

| 项 | 值 |
|---|---|
| 机器/会话 | 本机桌面会话，同一 UID；**无** root/`sudo`、**无**屏录、**无**虚拟/无头显示 |
| 计算路径 | **纯 CPU 拾取**（`cad_app::picking::pick_at_screen`），**不触碰 GPU**（故无适配器/后端结论） |
| 夹具 | `fixtures/complex-test.dwg`（**44,591** 实体；展开 **723,587** 条目） |
| harness | `crates/cad-app/tests/pick_index_real_fixture.rs`（`#[ignore]`；需真实夹具） |
| 复现命令（harness 文档给出） | `YACR_PROFILE_PICK=1 cargo test -p cad-app --test pick_index_real_fixture --locked -- --ignored --nocapture`；对照腿加 `YACR_PICK_EXHAUSTIVE=1`。本轮数字取自 **release** 构建（**实测**） |
| 计时口径 | harness 逐次 `pick_at_screen` 墙钟（`cold_ms` / `warm_median_ms` / `warm_max_ms`）与 `YACR_PROFILE_PICK` 行 |
| 证据落盘 | 仅本地，**未上传** |

## 报告的症状（**实测**）

在 `fixtures/complex-test.dwg` 上**点选一个实体停顿约一秒**。同一函数（`pick_at_screen`）还在
**每次指针移动**上运行（hover，见 `docs/input.md` §1.1.2 与 backlog S2），故停顿不止于点选。

## 机制（改动前，`crates/cad-app/src/picking.rs` / `crates/cad-spatial`）

- **生产拾取是纯 CPU**：`SelectAt` → `pick_at_screen`（`crates/cad-app/src/picking.rs:605`）→ `pick_ray`。
- **改动前 `pick_ray` 全量展开**：`borrowed_pick_items`（`picking.rs:141`）把 `database.model_space()`
  的**每个**实体展开成 pick item（含**递归 INSERT** 展开与累积变换），再由
  `cad_spatial::pick_closest_borrowed` **逐项**测试，且**每次调用**新建一次 bounds 缓存。
- **索引存在但在拾取链里无调用方**：ADR-0012 §7 规定的两级空间索引已存在，但
  `filter_by_index`（旧签名 `picking.rs:351`，见 ADR-0012 §5 源码核查）**是死代码**——生产
  `pick_ray` 每次全量遍历 `model_space()`（ADR-0012 §5 记录的 `picking.rs:109`）。

## 改动（commit `13f7808`）

### 保守包围盒（`crates/cad-spatial/src/pick.rs`）

- 新增 `pick_world_bounds`（`pick.rs:837`）与局部盒 `index_local_bounds`（`pick.rs:870`）：为**能**被
  保守界定的每个家族产出一个世界 AABB（line/point/mask/text 取顶点盒；circle/arc/ellipse 取绕中心的
  球/盒；spline 取控制点凸包；mesh 取顶点盒 + 相对松弛；compound 取子节点并集）。**故意**不同于
  `local_pick_bounds`（后者对 mesh/spline/带凸度 polyline/text 返回 `None`，若用它建索引这些家族会
  落回精确测试、**永不**收窄扫描）。
- 无法界定者返回 `None`：opaque payload、未展开的 insert、含无界子节点的 compound —— 它们随后进入
  **恒候选**列表（见下），**不**丢失 `skipped` 记账与可选择性（**铁律 2**）。
- `pick_world_bounds` 再加一层**极小的相对松弛**（`1e-9` × 最大边），覆盖 mesh 三角形谓词的 `1e-12`
  重心松弛与末位浮点分歧，**只增候选**，不改 `PickReport`。

### 两级 chunk 网格（`crates/cad-spatial/src/chunk.rs`，新文件）

- `ChunkGridIndex`（`chunk.rs:56`）：顶层均匀 X/Y chunk 网格 + 每 chunk 的**扁平 payload 列表**
  （payload = 条目在展开顺序中的 ordinal）。`build`（`chunk.rs:102`）对每个条目校验盒有限且不反转。
- **超大图元策略**（ADR-0012 §7 规范 §8.1 强制）：AABB 跨 chunk 数超过 `MAX_CHUNK_SPAN`（**16**）的
  条目**不**复制进每个覆盖 chunk，而是进 **overflow 列表**，每次查询直接测试——避免「城市街区块填充
  跨上万格复制」。
- `ray_candidates`（`chunk.rs:209`）：把射线的 X/Y 投影裁剪到网格矩形并按**拾取容差**展开，访问该矩形
  触及的 chunk，对每个局部条目应用**精确阶段同一个** slab 谓词
  （`crate::pick::ray_intersects_expanded_bounds`，`pick.rs:1214`），overflow 条目直接测试。
  **保守性**：只有射线离条目 AABB **远于容差**才拒绝；非有限算术一律落为候选 ⇒ 被拒条目**不可能**
  曾被命中，broad phase **不可能**把命中变成未命中。`[BBox intersects query] ⟹ hit` **从不**假设
  （ADR-0012 §2）。

### 索引化拾取（`crates/cad-app/src/picking.rs`）

- 持久化 `PickIndex`（`picking.rs:311`）：缓存展开后的条目（**仅** `SelectionRef`，几何/累积变换在查询
  时对**少量**候选从数据库解析，故大 INSERT 展开**不**被深拷贝进缓存）+ `ChunkGridIndex` +
  `unbounded`（恒候选 ordinal 列表）。键为 `DrawingDatabase::scene_identity()`（内嵌 `Revision`）+
  document id，**复用既有单一失效真相**（ADR-0012 §5），**不**新增第二套失效真相。
- 缓存 `cached_pick_index`（`picking.rs:424`）：thread-local `Arc<PickIndex>`；命中（同 scene + 同
  document）直接复用，未命中则 `PickIndex::build`。
- 查询 `PickIndex::ray_candidates`：`ChunkGridIndex::ray_candidates` 结果 ∪ `unbounded`，排序去重后
  按 ordinal 映射回 `SelectionRef`，**保持展开顺序**（精确阶段稳定 tie 处理不变）。
- `pick_ray`（`picking.rs:453`）现走 `pick_ray_with_index`（`picking.rs:543`）；`pick_ray_exhaustive`
  （`picking.rs:468`）保留为**差分测试参考**与**回退主体**——它返回的 `PickReport` 就是索引路径必须
  **完全相等**的契约（hit、tie-break、`skipped` 列表及顺序）。候选几何/变换经 `item_transform` 解析
  （走 ref 自身 instance path 组合 insert 局部变换，**跳过** `resolve_pick_transform` 的成员重校验——
  索引与展开同源，重校验的 `block_entities` 每次分配正是宽候选集慢的原因）。
- `filter_by_index`（`picking.rs:644`）现接受 `&PickIndex` 并返回 `Vec<SelectionRef>`（保持
  document + entity + instance path 的精确身份）。

### 回退与上报（无静默改变）

- `exhaustive_forced`（`picking.rs:42`）：`YACR_PICK_EXHAUSTIVE` 诊断门打开 ⇒ 强制穷举扫描。
- 索引不可用（构建失败）⇒ `cached_pick_index` 返回 `None` ⇒ 穷举扫描，并打印
  `pick_index_unavailable reason=…`。
- 机读模式：`last_pick_stats()`（`picking.rs:65`，thread-local）给出 `PickStats{indexed, items, candidates}`
  （`picking.rs:49`）。**索引路径**另打印 `pick_broad_phase mode=indexed …`；**穷举路径**打印
  `borrowed_pick_items …`。`indexed` 字段是两种模式的机读区分。

## 实测（release harness 驱动**同一**生产 `pick_at_screen`，同一夹具）

| metric | before（`YACR_PICK_EXHAUSTIVE=1`） | after（索引路径） |
|---|---|---|
| 展开条目 `items` | 723,587 | 723,587 |
| 热拾取中位 `warm_median_ms` | **~1055 ms** | **~0.32 ms**（**842** 个候选） |
| 最差热拾取 `warm_max_ms`（大半径） | 未记录 | **~20 ms**（**13,491** 个候选） |
| 一次性建索引 `pick_index_build` | — | **~535 ms** |

> **口径**：以上为**release harness**数字，**不是**宿主交互（见「未测 / 边界」）。逐次墙钟取
> harness 的 `cold_ms` / `warm_median_ms` / `warm_max_ms`；建索引成本取自 `pick_index_build` 行。

**`YACR_PROFILE_PICK` 行（源码逐字，`picking.rs`）**：

```text
YACR_PROFILE_PICK borrowed_pick_items count={} elapsed_ms={:.3}
YACR_PROFILE_PICK pick_index_build items={} bounded={} unbounded={} chunks={} overflow={} elapsed_ms={:.3}
YACR_PROFILE_PICK pick_broad_phase mode=indexed items={} candidates={} elapsed_ms={:.3}
YACR_PROFILE_PICK pick_closest_borrowed elapsed_ms={:.3}
YACR_PROFILE_PICK pick_index_unavailable reason={error:?}
```

**harness 行（源码逐字，`pick_index_real_fixture.rs`）**：

```text
YACR_PICK_HARNESS fixture={} entities={entity_count} exhaustive={}
YACR_PICK_HARNESS picks={} hits={hits} cold_ms={cold:.3} warm_median_ms={median:.3} warm_max_ms={max:.3}
```

## 随机差分测试的输入空间（**实测**）

`indexed_and_exhaustive_picks_agree_over_random_scenes`（`picking.rs:1480`）——
**16 种子 × 30 射线**（= **480** 次差分拾取；**源码为准**：commit message 记为「16 seeds × 60 rays」，
实际循环为每种子 30 次），每一步断言索引路径与穷举路径的 `PickReport` **完全相等**
（hit、tie-break、`skipped` 列表及顺序）。每个种子随机生成一个 model space：

- **两个 block**（其一嵌套：block 1 放置 block 0）+ 随机 INSERT 放置 + `24 + (seed % 5) × 16` 个实体；
- 覆盖**全部实体家族**：line / point / shape / polyline（含凸度、闭合）/ mask / circle / arc /
  ellipse / spline（偶发 NaN knot 与 0 权重 ⇒ 回退控制点路径）/ mesh（**一半带 face source**，一半不带）
  / text / image / opaque / compound；
- **稠密与稀疏**：`span ∈ {2.0, 50.0, 500.0}`；
- **容差**：`{1e-6, 1e-3, 0.5, span×0.01, span×0.2}` 中随机取；
- **射线**：随机非退化射线（方向长度 > 1e-3）。

配套的定向差分测试：

- `indexed_pick_agrees_at_the_predicate_tolerance_floor`（`picking.rs:1504`）：**40** 条射线，容差取
  策略下限 `computation_world`（强制重 tessellation 的样条/polyline/圆/mesh 混合场景）。
- `geometry_exactly_at_the_tolerance_boundary_agrees`（`picking.rs:1583`）：容差**恰好**处命中、
  `tol + 1e-6` 处不命中。
- `candidates_exactly_on_a_chunk_boundary_agree`（`picking.rs:1620`）：几何落在 chunk 边界、
  射线原点**恰在**网格边界上。
- `indexed_pick_agrees_on_a_ray_that_hits_nothing`（`picking.rs:1565`）：无命中射线。
- `the_fallback_scan_is_used_and_correct_without_an_index`（`picking.rs:1718`）与
  `the_cached_indexed_pick_matches_the_exhaustive_reference`（`picking.rs:1735`）。
- **计数断言**：`the_index_examines_fewer_items_on_a_large_scene`（`picking.rs:1663`）——5,000 条线的
  规则网格上，`last_pick_stats().candidates < items / 10`，且索引结果与穷举相等。

## 差分测试抓到的一个真实几何 bug（记录）

带**凸度**的 polyline，其圆弧在 polyline 于 3D 中**非共面**时可**远超**弦盒（超过矢高）。tessellator
在 polyline 的**最佳拟合平面**（Newell 法）里构造圆弧，故弦盒对非共面 polyline 不再包含弧。修复：
凸度弧现由**绕 tessellator 弧心的球**来界定（`bulge_arc_bounds`，`pick.rs:1142`；平面法向
`polyline_plane_normal`，`pick.rs:1106`，与 tessellator 同一 Newell 法、含闭合边与退化回退），
对**任意平面**都包含整段弧。

## 偏离项（记录，不隐藏）

- **ADR-0012 §7 命名的是每 chunk 的 `rstar`**（不可变快照 + `bulk_load` 重建式）；本轮 `Cargo.*`
  超出任务范围，故每个 chunk 持**扁平 payload 列表**。`rstar` **仍未实现**。`chunk.rs` 头部注释
  明确写出该偏离与退化情形（点状查询访问一个 chunk，长对角射线退化为跨 chunk 的线性扫描；该退化
  **被记录、不隐藏**）。
- **内存足迹**：该索引在 `fixtures/complex-test.dwg` 上占 **~90–100 MB**（**真实**足迹，**未**削减）。
  本轮**未**测量跨图纸的足迹分布。

## 未测 / 边界（不得省略）

- **不得从 harness 得出人工交互判定**：after 数字来自 **release harness**，它直接调用**生产**
  `pick_at_screen`，但**不是**宿主交互——**没有任何探针钩子能 hover 或 select**
  （`YACR_PROBE_POSTLOAD_PAN` 以按下开始，故无 hover；baseline 那次唯一的宿主 hover 来自一次
  **不确定**的合成器移动）。**人工判定归用户给出**。
- **不给** GPU/视觉/vendor/性能结论：harness 是**纯 CPU**，**不触碰 GPU**，**无**像素读回、
  **无**视觉判定；单机、单会话，**不外推**到其它图纸/更大图纸/其它 vendor。
- **单适配器无关**：本路径不用 GPU，故与适配器无关（这不构成「跨平台一致」结论）。
- **wasm / 浏览器真实运行时、Windows/macOS/Android**：未测。
- **`SceneCache::apply_changes` / `CacheKey`（ADR-0012 §10 S4 的另一半）**：**未**在本轮接入生产；
  仍只在测试使用（见 `docs/render-optimization-backlog.md` §S4）。

## 交叉引用

- 索引架构、五级过滤、早判不对称性、超大图元策略：`docs/adr/0012-scene-spatial-index-and-interaction.md`
  （§2、§3、§7、§10 S4）。
- 生产拾取走 CPU 路径、GPU id pass 无生产调用方：`docs/picking-gpu-id-pass.md`（§0b）。
- 优先级与 landed 记录：`docs/render-optimization-backlog.md` §S4。
- 提交路径 pick 表增量（同轮相邻的 pick 缺陷，口径不同）：`docs/validation-pick-table-incremental-2026-10-17.md`。
- 相机-only 导航保持选择高亮（同轮第二项修复）：`docs/validation-selection-survives-camera-2026-10-17.md`。
