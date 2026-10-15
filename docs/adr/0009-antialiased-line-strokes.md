# ADR 0009：抗锯齿线笔画统一（所有线类与 SDF 弧同语义）

状态：**已实施**（用户批准的视觉统一决策「1+3」：把所有线类几何改为抗锯齿笔画，与 SDF 弧
（ADR-0008）**同语义**——`w = world_per_px` 的线性覆盖三角滤波，1.0 px 线性墨量，相位相关峰值）。
本 ADR 是设计记录与实施记录；测试证据见 `crates/cad-render-wgpu/tests/sdf_arcs.rs`（新 fences
一节）与 `docs/render-order.md`。

真实 GPU（默认 Vulkan ICD，未设 `VK_ICD_FILENAMES`）证据由运行适配器标注。本切片**未**做
浏览器/WebGL2/Android/vendor/golden 验收。

## 背景

- ADR-0008 把圆/弧改成解析 SDF 实例，抗锯齿三角滤波，**1.0 px 线性墨量**；而**线**仍是
  硬抗锯齿（hard-aliased）1 px `LineList` 图元（相位吸附、无 AA）。实测线性墨量二者相同，
  但峰值/相位行为与锯齿观感不同（sRGB 通道积分会虚增软笔画 ~1.4×，是测量假象）。
- 用户决定统一：**线也走同一抗锯齿语义**，使峰值与墨量跨类一致。这是**已授权的语义像素变化**；
  与硬抗锯齿线的逐像素一致声明**作废**。

## 决策

### 1. 统一语义（到**线段**的距离场）

`line.wgsl`（legacy 逐批）/ `line_instanced.wgsl`（实例流）/ `line_pulled.wgsl`（加速档）三者
使用与 `arc.wgsl` 相同的滤波：

- 世界空间围绕 `p0→p1` 展开一个**四边形**（margin ≥ 2 世界像素，与弧的四边形 margin 同思路）；
- 片元阶段计算**到线段**的距离 `d`（含圆头端点）；
- `w = world_per_px`（`key_flags.z` / pulled 相机的 `c3.x`）；`fwidth(d)` 仅在未设置时回退；
- `coverage = saturate(1 − d / w)`（三角/线性滤波，积分恰为 `w` = **1.0 px 线性墨量**）；
- **强制 `coverage <= 0 → discard`**（四边形会过度绘制，不丢弃会抹掉其后几何）。

**平面内垂直方向**：垂直方向取**绘制平面（xy）内**的垂直（`vec2(-dir.y, dir.x)`），而非
`cross(dir, ref)`——后者对水平线段会给出屏幕法线（z）方向，四边形在正交俯视下退化为零面积
（实施中实测到的 bug，已修）。这与弧的**平面**世界展开假设一致，3D/透视沿用同一仿射
`world_pos` 构造（ADR-0008 已论证其透视正确性）。

### 2. 转换路径（"same AA segment instances"）

- **不透明 `Lines`**：实例流（`LineInstance` 56 B，加速档 64 B），每实例一个 AA 四边；
- **legacy 逐批线**（`line_instancing=false`、透明线、奇数顶点余量）：上传时展开为**三角列表
  四边**（`plan::expand_line_quads`，`LineQuadVertex` 32 B = `p0`+`p1`+corner，每线段 6 顶点），
  经 `line.wgsl` 同语义 AA——因此 `line_instancing` 开关不再改变 AA 语义，两条路径
  （实例 vs 逐批）应**等价**；
- **`MeshEdges`**：此前是**文档化的静默 no-op**（legacy `draw_batch` 读 `GpuBatch::indices`，
  而 `MeshEdges` 把 identity 线表放在 `edge_indices`）。现改为正常转换/绘制为 AA 线段
  （**backlog G3 关闭**）；sentinel 测试 `mesh_edges_batch_currently_renders_nothing` 被
  `mesh_edges_batch_renders_anti_aliased_segments` 取代；
- **网格叠加线框**：merged mesh 的叠加仍走实例流（AA）；非 merged mesh 的叠加走 legacy
  四边（AA）。`draw_batch` 的 mesh 叠加改由 `line_quads` 绘制（不再用共享网格顶点 + 线索引）；
- **弦展开 conic**（虚线/椭圆回退）：本身是 `Lines` 批次，自动统一；
- **加速档 pulled 线**：`line_pulled.wgsl` 同样 AA；`PulledLine` 记录未变（仍 56 B 内容 /
  64 B 存储），pulled 相机 uniform 增至 **4 列**（`c3.x` = `world_per_px`）。

**未变的账**：线段存储格式与拾取 key 打包不变；`lineweight` 仍「携带不绘制」
（`lineweight_not_drawn` 仍覆盖所有类）；透明线仍按原透明 pass 混合 + AA；**不**光栅化线宽。

### 3. 拾取对齐（±0.5 px）

`pick_line.wgsl` / `pick_line_instanced.wgsl` 栅格化**同一**线段 SDF，`coverage < 0.5` 丢弃
（±0.5 px 容差，与 `pick_arc` 一致）。因此线像素的**核心**可拾取、AA 边缘外沿不可拾取——这是
既有拾取容差的延续，不是回归。

### 4. 预算诚实（图元单位）

每线段计 `vertices += 6`、`triangles += 2`（实例流与 legacy 四边一致）；字节：实例 56/64 B、
legacy 四边 32 B × 6、弧 72 B、merged mesh 44 B/顶点 + 4 B/索引。`batch_budget_*` 与
`docs/render-order.md` 预算节同步更新。

### 5. 加速档类绘制顺序

合并 mesh 类**先于**线类绘制（此前相反）。原因：AA 线在亚单位覆盖处的边缘会与「其后」几何
混合，而通用逐 run 路径按计划顺序先画 mesh 后画线；先画 mesh 使线在穿越处按 mesh 之上混合，
两档帧**逐字节一致**（`accelerated_overlapping_ties_match_universal_pixels`）。等深并列本身
与顺序无关（线的 `LessEqual` 胜 mesh 的 `Less`），仅边缘混合背景受顺序影响。

## Fence 重划（显式、诚实）

- 确定性 fence 保留；相机移动零重传保留（`instance_stream_uploads` 不变）。
- 「legacy vs instanced 逐字节一致」→ **转换路径等价**（两侧均 AA）：f32 可表示的共享 origin
  下逐字节一致；非共享/per-chunk origin 下 f32 归约有 ~1 ulp 差异，AA 边缘暴露为**有界**的
  边缘像素差（`instanced_edge_overlay_matches_legacy_pixels`、`merged_meshes_lines_and_overlays_match_legacy_pixels`
  现按有界像素数断言；`instanced_deep_zoom_*` 改断言**同一像素列**——精度语义量）。
- 大坐标精度 fence 保留其解析检查；弧弦包含基线按 AA 弦重测。
- **新增统一 fences**（`tests/sdf_arcs.rs`）：
  (i) 跨类线性墨量一致——弧/线/`MeshEdges`/透明线/弦展开 conic 同帧，均 ∈ [0.85, 1.15] px；
  (ii) 线类逐相位峰值包络（16 相位一帧，min ≥ 0.45、max ≥ 0.9、相位相关）；
  (iii) 折线连续性（共线与折角接头无孔）；
  (iv) 接头 alpha 并集（接头不暗于单笔、不超过满强度）；
  (v) 确定性 + 相机移动零重传。
- 视觉验收：`/tmp/opencode/yacr-sdf-arcs-v4/{before-hard-lines,after-unified-aa}.png`，
  混合场景（弧 + 直线 + 折线接头）；「before」用测试钩子 `set_line_ramp_override` 把线 ramp
  压到亚像素以复现统一前的硬抗锯齿外观（真实 GPU 输出，非合成）。

## 实施记录（真实 GPU，Quadro P620 / Vulkan）

- `cargo test -p cad-render-wgpu`（`--test-threads=1`）：lib 121 passed/3 ignored、
  accelerated_tier 23、headless_render 19、pick_id 32、render_effects 44、sdf_arcs 17/2 ignored、
  wgsl_validation 10，全绿。
- 线性墨量（同帧，`unified_line_ink_parity_across_classes`）：arc 1.000、line 1.000、
  mesh_edges 1.000、transparent 0.994、chord 1.004 px。
- 相位峰值：mean 0.748、[0.497, 1.000]。
- 折线接头峰值全 1.0（无孔、无双重覆盖）。
- 视觉验收：after 线墨量 1.001 px；AA 灰阶 hard=24 vs unified=34。
- 拾取对齐：`pick_id` / `accelerated_tier` 拾取用例按 ±0.5 px 容差重划后全绿。

**未验证**：浏览器/WebGL2/WebGPU、Android 设备、vendor DWG、视觉 golden；真机 GPU 以外的
后端未测。**未**光栅化线宽（backlog）。**未**做与硬抗锯齿线的逐像素一致声明（本 ADR 即其作废）。
