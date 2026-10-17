# 块级视锥剔除（chunk-level view culling，P1）验证（2026-10-17）

**目的**：记录 **backlog §P1 —— chunk 级视锥剔除**落地后的**本机真实宿主实测**与**测试矩阵**。
代码面：新纯谓词模块 `crates/cad-render-wgpu/src/cull.rs`（`WorldAabb` / `ChunkCull` / `build` /
`ortho_visible` / `perspective_visible` / `class_visible`，10 个纯单测）+ 三个 chunk 构建器
**保留**其本已计算的 world-space `f64` AABB 到上传结构（`InstanceStream` / `GpuMeshChunk` /
`ArcStream`）+ 每帧一次对 `FrameProjection` 的可见性测试 + 在全部 encode/合并点**跳过被剔除的
run** + `FrameStats` / `[probe] actual-render` 的**追加式**上报字段 `cull_chunks`。

**这不是**性能验收、**不是** GPU 时间测量、**不是**跨平台/vendor/发布版结论、**不是**视觉判定。
单机、单适配器、单会话、同 UID。口径与 `docs/validation-frame-pacing-2026-10-17.md`（**culling
之前**的 pacing 轮运行）、`docs/validation-realhost-frame-cost-2026-10-17.md`（更早的运行）**不同**，
三组数字**不得混算**。

## 环境与授权边界

| 项 | 值 |
|---|---|
| 机器/会话 | 本机桌面会话，**原生 Wayland**，同一 UID；**无** root/`sudo`、**无**屏录、**无**虚拟/无头显示（**实测**） |
| 适配器 / 后端 | **Vulkan**，`Quadro P620`（**实测**；驱动版本本轮**未记录**） |
| 缩放 | 本轮**未记录** `scale`（不得假定 `scale=1`） |
| 宿主进程 | `target/release/yacr`（本轮**未记录**二进制哈希，故不声称具体 commit 对应关系） |
| 退出/卫生 | 两次运行均自然 **exit 0**、**无** panic、**无** invalid-buffer 行、**无** validation 行、**无** device-lost 行（**实测**） |
| 计时口径 | 宿主逐帧 `[probe] actual-render …`。**`gpu_ms=None`**（无 `TIMESTAMP_QUERY`）；`wait_ms` 是**阻塞 poll / 呈现的代理，不是 GPU 时间** |
| 证据落盘 | 仅本地（`${YACR_EVIDENCE_ROOT:-/tmp/opencode}` 下本轮输出目录），**未上传** |

## 本轮改了什么

### 剔除单元与谓词（纯逻辑，无 GPU 依赖）

- **剔除单元是一个 chunk**：`InstanceStream`（instanced line 段）、`GpuMeshChunk`（合并 mesh 三角形）
  或 `ArcStream`（SDF conic 实例）。**每一条被测量的 draw 都是限定在单个 chunk 内的合并 run**，故
  剔除一个 chunk 会整条消掉其 run（见 `docs/render-optimization-backlog.md` §P1）。
- **AABB 来自构建器既有计算**：三个构建器（`plan.rs`）在打包 chunk 时**本已**累计一个 world-space
  `f64` AABB（用于各自的切分启发式）。本轮把它**保留**在发出的 chunk 上、并带到上传结构
  （`InstanceStream.aabb` / `GpuMeshChunk.aabb` / `ArcStream.aabb`，`lib.rs:843`/`:896`/`:925`）。
  **不新增任何逐实体逐帧计算**——剔除是 **O(chunks)**。
- **每帧测试一次**：`cull::build(projection, wpp_line, wpp_arc, instance, mesh, arc)` 对该帧的
  `FrameProjection` 产出 per-class 可见性掩码（`ChunkCull`）。

### 保守且单侧（只会保留，绝不会误剔除）

- **非有限 / 空 / 反向 AABB ⇒ 恒可见**（谓词无法推理的 box 一律不剔除）；
- **跨 chunk 的超大源**：每个分片携带**整批**的 AABB（保守——只要该批任一可见，每个分片都保留）；
- **混合了远处批次的 chunk**：box 很大、几乎与任何视图相交 ⇒ **从不剔除**。
- 以上均为**刻意**设计（单侧谓词：仅当**已加 pad 的** box 完全在视图外才剔除，故任何可能上屏的
  chunk 恒被保留）。

### 测试期 pad（**绝不**烘焙进存储的 box），逐类

| 类 | pad | 依据 |
|---|---|---|
| line（instanced） | `3·wpp_line + 1e-6` | AA line 四边在端点外展开 `half = wpp + max(2·wpp, 1e-6)`，最多 `3·wpp`（`shaders/line_instanced.wgsl:95-97`、`line.wgsl:105-109`、`line_pulled.wgsl:131-133`） |
| mesh（merged） | `1e-6` | mesh 三角形按精确顶点光栅化，**无** stroke/quad 展开 |
| arc（SDF conic） | `2·wpp_arc + max(0.02·R, 0) + 1e-6` | SDF conic 四边形绕中心按 `grow = 1 + max(0.02, 2·wpp_arc/r)` 扩展（`shaders/arc.wgsl:122-123`）；`R` = 该 chunk AABB 的**最大半extent** |

- `wpp_line = line_ramp_override.unwrap_or(projection.world_per_px())`（测试钩子覆盖值优先）；
  `wpp_arc = projection.world_per_px()`（SDF conic 类**从不**用覆盖值）。
- **`lineweight` 不产生 pad 项**：它被**携带但未光栅化**（`shaders/line_instanced.wgsl:123-124`，
  `GpuBatch.lineweight`），故**不是**遗漏项——不得读作缺了一个 pad 项。

### 正交与透视

- **Ortho**：精确 `f64` 2 轴 NDC 测试；当加 pad 后的 box 的 x-range 或 y-range 与 `[-1,1]` 无重叠时
  剔除。**z 刻意不测**——正交 clip `z` 是 `(z − center.z) + z_plane`，深度映射**不是**可见性判据
  （测它可能误剔除投影在视图内的几何）。
- **Perspective**：从 `vp` 的行做 6 平面 **p-vertex** 测试，带 **f32 相对 margin**——因为透视原点在
  送入 view-projection 前被窄化为 `f32`（`FrameProjection::batch_matrix_f64`），渲染出的世界位置
  可与存储的 `f64` box 差约 `ulp(origin)`；`4×f32::EPSILON×max_abs` 是安全上界。

### 记账与 `use_pulled` 门

- **记账**：被剔除的 run 在**每个** encode/合并点被跳过（**从不重排、从不跨空隙合并**），并计**零**
  顶点/三角形。**字节记账仍按批粒度**（`max_bytes` 在产品里是 `usize::MAX`，建模的是上传驻留而非
  draw 工作量）。**透明 legacy draw 与图像不被剔除**。
- **`use_pulled` 门**：加速 pulled 路径在**任何**剔除发生时被禁用（`&& !cull.any_culled()`，
  `renderer_impl.rs:4620`）——因为一次 pulled draw 会重绘**整条 stream/chunk**，会重绘被剔除的几何。
  **不做 pulled 子集 draw**——那仍是 **N1d**（见下）。
- **pick 从不被剔除**：`pick.rs:495` 传 `ChunkCull::all_visible()` 恒等掩码，故命中测试与视图剔除**独立**。

### 上报（追加式，不改既有字段含义）

`FrameStats` 新增 `cull_chunks_total` / `cull_chunks_kept`（`lib.rs:460`/`:463`）；
`[probe] actual-render` 行在 `pacing={}` **之后**追加 `cull_chunks={kept}/{total}`
（格式串 `apps/app-linux/src/host.rs:1981`）。**未新增**诊断码。

## 逐帧证据（字段与观测区间）

**说明（诚实边界）**：本轮**未保留**可逐字引用的捕获行，故**不**伪造任何 verbatim 行。以下是
**字段布局**（取自源码格式串，**非**某次运行的捕获行）与**观测区间**：

- 字段布局（行尾，`pacing={}` 之后）：`… pacing=<active|at-ceiling|disabled-no-completion-signal> cull_chunks=<kept>/<total>`。
- 观测区间（**实测**）：

`fixtures/complex-test.dwg`：

- **681** 个 render-probe 页；其中 **161** 页 `kept < total`；
- `cull_chunks` 区间 **`1/9362` … `9362/9362`**；
- `draw_calls` 区间 **`1` … `3027`**（剔除前基线 **3,027** 在 `cull_chunks=9362/9362` 处复现）；
- 最深的剔除页：`draw_calls=1 verts=12 tris=4 instanced=1`；
- `accelerated=false`；`pulled=active`；`budget_scale` **0.050 … 1.000**；`pacing=active`；
- `complete=true` 出现在 **50** 个 probe；`over_budget=true` 出现在 **653** 页。

`fixtures/debug-default-fonts.dwg`：

- **466** 页；其中 **215** 页 `kept < total`；
- `cull_chunks` 区间 **`1/806` … `806/806`**；
- `draw_calls` 区间 **`1` … `1299`**；
- 最深的剔除页：`draw_calls=1 verts=12 tris=4 instanced=1`；
- `accelerated` **混合**（**163** true / **303** false）；`pulled=active`；`budget_scale` **0.562 … 4.000**；
- `complete=true` 出现在 **381** 个 probe。

- post-load 的**平移（pan）**已足以驱动深剔除（`kept` 最低到 **1**）；**无需**单独的 zoom 钩子。

### 两次运行（**实测**）

| fixture | probe 页 | `kept<total` 页 | `cull_chunks` 区间 | `draw_calls` 区间 | 最深剔除页 | `accelerated` | `pulled` | `budget_scale` | `complete=true` | `over_budget=true` |
|---|---:|---:|---|---|---|---|---|---|---:|---:|
| `complex-test.dwg` | 681 | 161 | `1/9362` … `9362/9362` | `1` … `3027` | `draw_calls=1 verts=12 tris=4 instanced=1` | `false`（全部） | `active` | 0.050…1.000 | 50 | 653 |
| `debug-default-fonts.dwg` | 466 | 215 | `1/806` … `806/806` | `1` … `1299` | `draw_calls=1 verts=12 tris=4 instanced=1` | 混合（163 true / 303 false） | `active` | 0.562…4.000 | 381 | （未单列） |

## 测试矩阵（**实测**）

| 命令 | 结果 |
|---|---|
| `cargo test -p cad-render-wgpu --lib` | **177 passed / 0 failed**（含 **10** 个新纯 cull 单测） |
| `cargo test -p cad-render-wgpu --test headless_render` | **32 passed**（含 **6** 个新 GPU fence） |
| `cargo test -p cad-render-wgpu --test accelerated_tier` | **23** |
| `cargo test -p cad-render-wgpu --test pulled_prefix_equivalence` | **9** |
| `cargo test -p cad-render-wgpu --test render_effects` | **45** |
| `cargo test -p cad-render-wgpu --test pick_id` | **32** |
| `cargo test -p cad-render-wgpu --test pick_hidden_flags` | **8** |
| `cargo test -p cad-render-wgpu --test sdf_arcs` | **17** |

`headless_render` 新增的 6 个 fence（名称即契约）：

| fence | 断言 |
|---|---|
| `culled_frame_is_byte_identical_and_drops_exactly_the_culled_runs` | 剔除帧与未剔除帧**逐字节一致**，且**恰好**丢掉被剔除的 run |
| `large_lineweight_at_the_viewport_edge_is_pixel_identical` | 视口边缘的大 `lineweight` 逐像素一致（`lineweight` 无 pad 项成立） |
| `pick_is_invariant_under_chunk_culling` | pick 在 chunk 剔除下不变（pick 用恒等掩码） |
| `culling_does_not_pop_across_a_chunk_border` | 剔除在 chunk 边界**不弹出** |
| `culling_is_byte_identical_on_the_3d_path` | 3D 透视路径上剔除**逐字节一致** |
| `culling_does_not_break_the_progressive_cursor` | 剔除**不破坏**渐进 cursor |

门禁（**实测**）：`fmt` / `check --all-targets` / `clippy -D warnings` /
`check --lib --target wasm32-unknown-unknown` 全 clean；release 构建
（`--features tauri/custom-protocol`）OK。

## 未测 / 边界（不得省略）

- **逐字节 parity fence 跑在 headless 软件 Vulkan 适配器上，不是硬件 GPU**；真实宿主两次运行只产出
  **日志证据**（**无**像素读回）。**剔除比**不能推出任何 vendor / 精度 / 性能结论。
- **未解释观测（记为开放问题）**：debug 的最大 `draw_calls` 为 **1299**，**高于**更早的 **1,195** 基线；
  实现 lane **未**对账它（**可能**是不同的相机/页构成）。**逐字记录为开放问题，不作为解释**。
- **未测（NOT RUN）**：全量 workspace 门禁矩阵、打包路径、X11 沙箱、其它 GPU/vendor、**视觉判定**。
- **N1d 仍开放**：pulled **子集** draw **未**实现；本轮的 `use_pulled` 合取项使剔除**强制**走通用
  per-run 组织（见 `docs/render-optimization-backlog.md` §P1 / §N1）。
