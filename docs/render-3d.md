# 3D 网格渲染路径（实现契约，GPU 未运行）

规范 §3.1 F14 / §5.2 / §8；审计 F14 与 F12。本文记录 GPU 侧三维网格路径的**实现
边界**，区分已实现、显式未实现和只能在真实设备上验证的部分。

## 已实现（静态可验证）

### 场景侧的拓扑与数据（`cad-scene`）

`RenderBatch` 现在携带：

- `topology: RenderTopology`（`Lines` / `Mesh` / `MeshEdges`），审计指出的“Scene 无
  拓扑标记”由此闭合；
- `normals`：每个网格顶点一个法向，长度与 `vertices` 一致；
- `indices`：三角形拓扑索引，网格顶点**去重**（不再按三角形角点重复）；
- `edges`：线框叠加用的边顶点位置；
- `mirrored`：源变换是否负行列式（镜像绕序）；
- `alpha`：逐对象常量透明度，当前恒为 1.0（见“显式未实现”）。

`SceneCache::build` 会丢弃越界索引构成的三角形，而不是上传非法拓扑。

### HATCH 多环填充（`cad-geometry` → `cad-import-acadrust` → `cad-representation`）

`cad_geometry::hatch::fill_rings` 按**奇偶规则**填充多环 HATCH：环按包含深度分层，偶数深度为实体、
奇数深度为孔、更深偶数深度为岛。实现是 y 波段梯形分解（波段只取顶点纵坐标，段内边单调，交点按序两两配对），
每对生成两个一致的逆时针三角形，因此孔和岛都无需脆弱的桥接。结果用从嵌套关系推导的奇偶面积做交叉校验；
不匹配即返回 `FillError::Degenerate`，绝不返回错误填充。

- 预算：合并顶点数超过 `MAX_FILL_POINTS` 返回 `Budget`；三角形数超过 `MAX_FILL_TRIANGLES` 返回 `TooComplex`。
- 线圈简化按 HATCH 自身范围缩放；模式填充（`pattern_polylines`）同样按奇偶规则排除孔与岛。
- importer 只在 `fill_rings` 成功时追加 `SemanticGeometry::Mesh`，失败（预算/退化/非有限/退化平面法向）
  一律追加边界并报告 `Completeness::Partial`（boundary only），不伪造填充。
- `cad-representation` 把该 compound 的 Mesh 子几何走既有 `DisplayPrimitive::Mesh` 路径，
  边界环同时保留为 `Lines`（契约测试 `multi_ring_hatch_fill_flows_through_the_mesh_path`）。

**未完成**：非平面边界不可由 OCS 边的 2D 表达表示，故此处只报告退化平面法向为 Partial；真正 3D 非共面
HATCH 的检测需要上游提供 3D 边界。渐变 HATCH 仍只绘制边界。

### 顶点/索引预算（`SceneBudget`）

新增 `max_vertices_per_frame`、`max_triangles_per_frame`。`cad_scene::FrameBudget`
与 `FrameUsage` 提供逐批累计，超限返回带 category/requested/limit 的
`BudgetExceeded`。渲染器提交前调用 `plan_from_gpu`，超限产生诊断码
`render.frame_over_budget`（`render.frame_over_budget` 属于结构化诊断，不是散文）。
**超限时保留已接受的批次并明确报告被跳过的批次数，绝不静默丢弃。**

### GPU 管线（`cad-render-wgpu`）

- **线条管线** `cad-lines-pipeline`：`LineList`，`cull_mode = None`。
- **网格管线** `cad-mesh-pipeline`：`TriangleList`，`Depth32Float` 深度附件，
  `depth_compare = Less`，`cull_mode = Back`。
- **镜像变体** `cad-mesh-pipeline-mirrored`：与网格管线同 shader，仅
  `front_face = Cw`。当批次的 `mirrored` 为真时选用它，使背面剔除仍剔除真正不可见的
  一侧（审计“镜像绕序”）。
- **线框叠加**：对网格批次，用共享的网格顶点缓冲 + `sorted_edge_indices` 生成的
  `LineList` 索引缓冲绘制边线，不重复顶点数据。
- **深度缓冲**：`ensure_target` 随颜色附件一同创建并清空深度纹理。

### 法向修复（显式规则）

`geometry::repaired_normals`：

1. 法向长度与顶点数一致且全部有限、非退化 → 归一化后直接使用；
2. 否则（缺失、数量不符、NaN、零向量）→ 用 `cad_geometry::compute_vertex_normals`
   从索引三角形做面积加权平均重算；
3. 重算仍得到零向量（只被退化三角形触及）→ 使用固定单位法向兜底，shader 永远看不到
   NaN 或零长度向量。

`normals_need_repair` 暴露是否需要修复，便于上层报告。

### 3D 相机（`geometry::Camera3d`）

右手系 look-at + 透视投影，裁剪空间 z 在 `[0, 1]`（wgpu 约定），列主序与 WGSL
`mat4x4<f32>` 一致。退化配置（eye==target、up 平行视线、非正 near/far、非法 aspect）
返回 `None`，`render_3d` 返回 `RenderError::Frame` 且不提交任何 GPU 工作。

### 着色（诚实声明）

`mesh.wgsl` 仅做：固定 -Z 方向平行光（headlight）的 `N·L` 漫反射 + 常量环境项。
**不声称** PBR、环境光照、镜面高光或任何基于物理的正确性。法向按世界法向插值处理，
仅适用于当前平面/轨道视角路径。

### 设备丢失（F12-adjacent）

`render` / `render_3d` 返回 `Result<FrameStats, RenderError>`：

- `RenderError::DeviceLost` — 设备丢失/后端失败；派生资源已清除；
- `RenderError::Frame` — 单帧校验或参数错误，设备与批次仍有效；
- `RenderError::NotInitialized` — 未初始化或缺少派生资源。

宿主观测到设备丢失时调用 `Renderer::note_device_lost(detail)`，它清空派生 GPU 资源并
置位。`rebuild_device` 同样只拆除并返回 Err，绝不带着丢失的批次静默重建。

## 显式未实现

- **透明/半透明排序**：场景侧尚无实体透明度数据通路，`alpha` 恒为 1.0。管线使用
  `ALPHA_BLENDING` 并把 `alpha` 传入 shader，但**没有**按深度排序透明批次，也未实现
  正确的透明合成顺序。审计要求的“透明”因此标注为未实现，而非假成功。
- **完整隐藏线算法**：仅背面剔除 + 深度测试，不是 HLR（规范明确允许）。
- **边线偏移（line offset）**：线框叠加与实体面共面时可能出现 z-fighting；未实现
  深度偏移。
- **3D 拾取**：GPU 侧无拾取，F14 的“受支持几何可拾取”仍待上层。
- **Mesh/Polyface 完整适配**：本路径只消费 `DisplayPrimitive::Mesh`；importer 的类型
  适配不在本工作范围内。

## 验证状态

- 纯逻辑单元测试（绕序翻转决策、法向重算/兜底、预算累计、3D 投影、边索引去重）：
  已运行并通过（`cargo test`）。
- WGSL 校验：用与 `wgpu 30.0.1` 同版本的 `naga` 在离线测试中解析并校验两个 shader
  文件，另有故意错误 shader 的对照测试。仅静态校验，**不编译管线、不执行 draw**。
- **GPU 提交无法在本环境验证**：没有可用适配器/设备初始化路径，管线创建、深度精度、
  镜像剔除和透明合成的真实行为均未运行验证。不得据此宣称渲染正确。

## 分页预算与每帧活动预算（2026-10-17）

逐条证据见 `docs/validation-frame-pacing-2026-10-17.md`；决策与基线见 `docs/render-optimization-backlog.md`。

- **两个预算**：`Renderer::frame_budget` 是**未 pacing 的上限**（静态默认 8,000,000 顶点 /
  2,000,000 三角形）；`active_frame_budget()` 是分页规划点**实际使用**的**活动预算**——上限乘上
  自适应 `scale` 的结果。`max_bytes` **不**随 scale 缩放。
- **三种 `pacing=` 状态**（`FrameStats::pacing`，逐帧上报）：
  `active`（渐进渲染 + 有完成信号：活动预算是缩放预算，每帧喂控制器）、
  `at-ceiling`（非渐进帧：整场景按上限规划，无可 pace）、
  `disabled-no-completion-signal`（无完成信号，如 wasm 非阻塞 poll：用静态上限，预算与 pacing
  之前**逐位一致**）。**pick/snap 不 pacing**（用上限）。
- **控制器**（`crates/cad-render-wgpu/src/pacing.rs`，纯状态机）：输入是每帧实测
  `cpu_ms + wait_ms`，目标默认 13 ms，±5% 死区，连续 2 次超目标 `×0.75`、连续 8 次低于目标
  `×1.08`，`scale ∈ [0.05, 4.0]`。**不得**改用 tick 间隔（被 vsync 量化，会形成极限环）。
- **清屏页图像先预留**：图像只在清屏页绘制（续页 `LoadOp::Load`，避免 alpha 累积）。清屏页现在**先**
  按活动预算预留**整幅图像记账**（paced 预算装不下全部图像时回落**上限**），几何再填剩余 ⇒ 页切分
  **不会**把图像丢在续页上。绘制顺序不变（图像仍最后）。
- **`over_budget` 不再意味着"慢"**：它的字面含义始终是"本帧**计划**越过了**活动**预算"，诊断码仍是
  `render.frame_over_budget`。`budget_scale` / `budget_target_ms` / `budget_cost_ema_ms` /
  `budget_verts` / `budget_tris` / `pacing` 字段（追加在 `pulled={}` 之后）负责**分类**：撞上按页
  顶点/三角形上限（如饱和在 ≈2.0M 三角形）会报超预算，**但该帧可以完全在 60 fps 预算之内**
  （`debug-default-fonts.dwg`：`over_budget=true` 473/479 帧，`wait_ms > 16.6 ms` 仅 0.4%，
  每帧 ≈5 ms，**实测**）。被**进度下限**强制而越界的一页带原因 `pacing-floor`。
