# 生产 CPU 拾取接入空间索引 验证（2026-10-17）

**目的**：记录一次**用户报告**的交互停顿（在 `fixtures/complex-test.dwg` 上点选一个实体停顿约一秒）的
**根因机制**、**渲染器/应用层改动**、**release harness 实测 before/after**、**随机差分测试的输入空间**、
**差分测试抓到的一个真实几何 bug**、**偏离项**与**未测边界**。改动落在 `crates/cad-spatial`
（新 `chunk.rs` 的 `ChunkGridIndex` + `pick.rs` 的保守包围盒）与 `crates/cad-app`（`picking.rs` 的
`PickIndex` 与索引化拾取路径），实现 commit `13f7808`，是 ADR-0012 §10 **S4 的拾取部分**。
**未**改宿主协议、**未**改 UI、**未**新增诊断码（`13f7808` 亦**未**改 `Cargo.*`）。后续 commit
`c4b66c6`（`perf(pick)`）追加了**每 chunk `rstar` 实验**与**粗网格落地**，commit `7031404`
（`feat(spatial)`）随后**按产品决定落地调优后的 per-chunk `rstar`**（并**改**了
`Cargo.toml`/`Cargo.lock` 与新增 `crates/cad-spatial/src/grid.rs`），见下文「每 chunk `rstar` 实验、粗
网格落地与 rstar 最终落地」；本页 `13f7808` 的历史数字保持原样，**不**被后续轮次改写。

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

## 每 chunk `rstar` 实验、粗网格落地与 rstar 最终落地（commit `c4b66c6` / `7031404`）

上面的数字与偏离项出自 commit `13f7808`（每 chunk 为**扁平 payload 列表**）。随后按 ADR-0012 §7 的
**字面**要求实现了**每 chunk `rstar`**（`bulk_load` 重建式 + 不可变快照；`rstar` **0.13.0** 仅接入
`cad-spatial`，§10.2 版本/来源/许可核查按解析后的 `Cargo.lock` 与已发布 crate 元数据完成：crates.io、
`MIT OR Apache-2.0`、`no_std`、默认 feature 为空、MSRV 1.85），并在**同一**夹具上对**旧扁平 per-chunk
列表**做了**单元尺寸扫描**。

复现命令（harness 文档给出）：
`YACR_PROFILE_PICK=1 cargo test -p cad-app --test pick_index_real_fixture --locked --release -- --ignored --nocapture`

### 单元尺寸扫描（**实测**，`fixtures/complex-test.dwg`，**723,587** 展开条目）

| entries/chunk | structure | build ms | RSS MB | warm med ms | warm max ms |
|---|---|---|---|---|---|
| 11 (fine) | flat / rstar | 520 / 652 | 128 / 157 | 0.447 / 0.561 | 36.8 / 41.2 |
| 256 | flat / rstar | 509 / 687 | 121 / 161 | 0.312 / 0.421 | 34.2 / 38.9 |
| 512 | flat / rstar | 508 / 685 | 118 / 160 | 0.310 / 0.435 | 34.2 / 38.8 |
| 4096 | flat / rstar | 511 / 717 | 123 / 163 | 0.318 / 0.449 | 33.9 / 39.6 |
| 65536 | flat / rstar | 503 / 686 | 118 / 162 | 0.310 / 0.446 | 36.1 / 38.7 |

每行给出 `flat / rstar` 两种结构；候选数在**每行都相同**（中位 **842**、最大 **21,762**）。per-chunk
R-tree 在**每个尺寸上都是净回归**：构建时间、热拾取中位/最差、RSS 全面变差，候选数不变。**原因**：
拾取容差与条目间距同量级，被访问 chunk 的查询包络覆盖其大部分，R-tree 几无可剪枝却仍付每棵树的开销。
**单独把网格变粗就有收益**：ADR 强制的单松弛格放置规则于是不再把跨格条目推进恒扫描的 oversized 列表。

### 落地结构（commit `c4b66c6`）

**粗网格**（目标 **512** 条目/chunk ⇒ **1,419** chunk，而非 65,637）+ **扁平 per-chunk 列表**；
`SMALL_CHUNK_MAX` 已移除（扁平唯一结构下无意义）。编排者**独立复测**：构建 **504.8 ms**、热拾取中位
**0.290 ms**、最差 **33.1 ms**、索引 RSS 增量 **115,812 kB（~113 MB）**、oversized **598**（原 2,066）、
chunk **1,419**、候选不变。该结果在**每个指标**上都优于旧的细扁平网格**与** rstar 变体。**`rstar` 依赖
从未提交、也不需要**——最终树 `Cargo.toml`/`Cargo.lock` 未动。§7 其余要求全部保留：松弛 2× 边界 +
单格放置、独立 oversized 列表、build-once 不可变快照、`Revision` 键控版本、保守超集语义、
`YACR_PICK_EXHAUSTIVE` 穷举回退；随机差分测试原样通过。

**测试（实测，commit `c4b66c6`）**：`cad-spatial` **47**、`cad-app` **358**、`cad-host` **102**、
`check-architecture.py` OK、wasm lib check OK、release build OK。

### 节点尺寸调优扫描（**实测**，同一夹具）

rstar 库默认节点尺寸（`MIN_SIZE = 3` / `MAX_SIZE = 6`）对这些 chunk 尺寸是**差拟合**。把 `MIN/MAX` 从
3/6 扫到 512/1024：**更大节点在每个轴上都更好**，直到约 512 后趋平；**`(256, 1024)`** 是扫描中的最佳
平衡点。调优把未调优版本的代价**大致减半**：

| structure | build ms | warm median ms | warm max ms | index RSS |
|---|---|---|---|---|
| untuned rstar (3,6) | 691 | 0.407 | 39.5 | ~146 MB |
| **tuned rstar (256,1024)** | **554** | **0.319** | **33.1** | **~136 MB** |

**小 chunk 扁平路径**在 512- 与 128-条目阈值上测量，**均被否决**（两种阈值的 warm 中位都差于纯树）。

### rstar 最终落地（commit `7031404`）

**产品决定**：**产品负责人明确选择 ADR 一致性、接受实测代价**（"全面落地 rstar"），故每 chunk 的
`rstar` **现已落地**：每非空 chunk 一棵 `RTree`（`bulk_load_with_params` 重建式 + 不可变快照；空 chunk
不分配），查询先按查询区域包络取候选、再走既有精确 slab 谓词。粗网格保留（目标 **512** 条目/chunk ⇒
**1,419** chunk），故单松弛格放置规则不把跨格条目推进恒扫描的 oversized 列表（**17,903 → 598**）。
候选数与 `PickReport`、保守超集语义**不变**。

**最终实测权衡（相对扁平 per-chunk 列表，同夹具 release 拾取 harness）**：

| structure | build ms | warm median ms | warm max ms | index RSS |
|---|---|---|---|---|
| flat + coarse（上一版） | ~520（简报：504.8） | ~0.309（简报：0.290） | ~34.6（简报：33.1） | ~126 MB（简报：115,812 kB） |
| **落地的 rstar（256,1024）** | **554**（独立复测：553.6） | **0.319**（独立复测：0.328） | **33.1**（独立复测：33.05） | **~136 MB**（独立复测：138,984 kB） |
| 未调优 rstar（3,6） | 691 | 0.407 | 39.5 | ~146 MB |

候选数在**每一行都相同**（中位 **842**、最大 **21,762**）。⇒ 构建与热拾取中位**变差**、warm 最差
**略好**；这是**为 ADR 一致性接受代价**，不是性能改进。

**依赖/锁事实**：`rstar` **0.13.0**、`default-features = false`、**仅** `cad-spatial`（`cad-app`/
`cad-scene`/`cad-measure` 不依赖）。§10.2 式核查（解析后的 `Cargo.lock` + 已发布 crate 元数据）
**无失配**（crates.io、`MIT OR Apache-2.0`、`no_std`、默认 feature 为空、MSRV 1.85）。`Cargo.lock` 差为
**纯新增（+31/-0）**；**诚实注记**：一次普通 `cargo check` 会重解析并顶起约八个无关包，故锁被还原、
**恰好**插入三个解析器条目（`rstar`/`heapless`/`hash32`），再以 `cargo check --locked` 与锁定 release
构建验证。

**网格算术复用（残余缺口已闭合）**：共享 helper `cad-spatial/src/grid.rs` 现由 `GridSpatialIndex` **与**
拾取 `ChunkGridIndex` **共用**（每个调用方保留自己的尺寸输入；新公式经核对与旧内联算术**逐位相同**，
故无既有尺寸改变）。

**测试（实测，commit `7031404`）**：`cad-spatial` **47**、`cad-app` **358**（+4/+11/+17 集成）、
`cad-host` **102**，全 0 failed；随机差分与边界测试**原样通过**；`check-architecture.py` OK、
wasm lib check OK、`clippy -D warnings` 干净、`fmt --all --check` 干净、锁定 release 构建 OK。

### 口径与边界（不得省略）

- **单个已提交夹具、纯 CPU 拾取**；**不给** GPU/视觉/vendor/跨平台或通用性能结论。
- **RSS 口径**：索引 RSS 为**构建前后的进程 RSS 增量**，是**上界**、依赖分配器、逐次运行波动
  **±~10 MB**，且**含**共享的 `items: Vec<SelectionRef>`（**不是**索引独占）。
- 扫描表中 pre-rstar「**细扁平**」基线行（含 `11 (fine)` 行）来自**上一轮** harness 运行，**本轮未重测**；
  其「flat」列与 `13f7808` 历史数字同源，**不得**与 `c4b66c6` 的粗网格数字混算。
- **「flat + coarse」行的来源（不得混算）**：其**简报**数字（构建 **504.8 ms**、中位 **0.290 ms**、最差
  **33.1 ms**、RSS **115,812 kB**）来自**上一轮**（`c4b66c6` 的编排者复测），**本轮未在该 SHA 重跑**；
  **实现 lane** 在**当前树内**重测了 flat+coarse（构建 **~520 ms**、中位 **~0.309 ms**、最差 **~34.6 ms**、
  RSS **~126 MB**），**编排者**则**独立复跑**了 **rstar 腿**（**553.6 ms / 0.328 ms / 33.05 ms /
  138,984 kB**）。三组数字**分别来自不同运行**，**不得**互相代入或合并。
- 热拾取**中位**由**稀疏区**拾取主导、**最差**由**稠密区**拾取的**精确阶段**主导，**都不单独隔离**
  broad phase（结构对比仍**同口径**：同网格、同放置、同候选）。

## 偏离项（记录，不隐藏）

- **ADR-0012 §7 命名的是每 chunk 的 `rstar`**（不可变快照 + `bulk_load` 重建式）。它先被实现并实测为
  对旧扁平列表**在每个单元尺寸上都净回归**（见上「单元尺寸扫描」表，**这是回归的证据基础**）；**随后
  产品负责人明确选择 ADR 一致性、接受实测代价**（"全面落地 rstar"），故 **rstar 现已落地**（调优
  `(256,1024)`，在粗网格之上，commit `7031404`，见上「rstar 最终落地」）。⇒ 这是**一次被记录的产品
  权衡，不是疏忽**：构建与热拾取中位相对扁平列表**变差**、warm 最差**略好**，候选数**不变**。
  `chunk.rs` 头部注释明确写出该结构、代价与退化情形（点状查询访问一个 chunk，长对角射线退化为跨 chunk
  的线性扫描；该退化**被记录、不隐藏**）。
- **网格算术复用（残余缺口已闭合）**：§7 要求「复用/扩展 `GridSpatialIndex` 的网格算术」。早先拾取
  chunk 布局**复制**了该算术而非共享；现已抽出共享 helper `cad-spatial/src/grid.rs`，由
  `GridSpatialIndex` **与** `ChunkGridIndex` **共用**，每个调用方保留自己的尺寸输入，新公式经核对与旧
  内联算术**逐位相同**（无既有尺寸改变）。`GridSpatialIndex`/`SpatialIndex`/`PendingSpatialIndex` 在
  `cad-spatial` 之外**无消费者**（已 grep 全 workspace 确认）。
- **内存足迹**：落地后的索引在 `fixtures/complex-test.dwg` 上占 **~136 MB**（**实测**，独立复测
  **138,984 kB**）的**进程 RSS 增量**（**上界**，依赖分配器，逐次波动 **±~10 MB**，且**含**共享的
  `items: Vec<SelectionRef>`）。扁平列表版为 **~126 MB**（简报 **115,812 kB**）。上一轮（细扁平，
  commit `13f7808`）粗记为 **~90–100 MB**，**本轮未重测**。本轮**未**测量跨图纸的足迹分布。

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
