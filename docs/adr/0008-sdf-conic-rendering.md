# ADR 0008：SDF 圆/弧渲染（slice B：实例化解析弧 + 发射门翻转）

状态：**已实施**。本 ADR 是 slice B 的设计记录（由架构评审给出、本切片按此实施）。实施记录
见文末「实施记录」节（含与本文若干字面细节的**有意偏离**，均已显式记录）。slice A（`feat(representation): analytic Conic display primitive
(chord-neutral)`，ADR 中记为 “conic 中性载体”）已经把 Circle/Arc/Ellipse 变成
`DisplayPrimitive::Conic`（f64 世界空间、仿射保持参数角），但**所有下游消费者仍按弦展开**，
渲染输出与旧路径逐字节一致。slice B 把**可 SDF 化**的圆/弧改成**解析 SDF 实例**渲染，并翻转
slice A 预留的发射门。这是**语义像素变化**（SDF 抗锯齿比折线弦更平滑），已获用户视觉验收
授权；**不**做任何与弦的逐字节一致声明。

真实 GPU（默认 Vulkan ICD，未设 `VK_ICD_FILENAMES`）证据由运行适配器标注；软件后端产出的
数字/截图必须显式标注为软件渲染。本切片**未**做浏览器/WebGL2/Android/vendor/golden 验收。

> 后续：ADR-0009 把所有**线**类几何也改为与本文同语义的抗锯齿笔画（1.0 px 线性墨量）。
> 因此本文中「与硬抗锯齿 1 px 直线等墨量」的对照描述仅适用于 ADR-0009 之前的状态；线现已
> 同样 AA，墨量对照改为「跨类 AA 墨量一致」（见 `docs/render-order.md` 与 ADR-0009）。

## 背景

- 弧今天先离散为折线弦（`ConicSegment::tessellate` → `Lines` 批次 → instanced line 流）。
  弦近似在深缩放时可见折角，且占用大量 line 实例。
- slice A 已把 conic 作为**解析载体**带到表示层，并记录 SDF 资格事实（不透明、连续线型、
  非退化轴）到 `ConicStats`（`cad-representation`）。
- 待办项 P4（`docs/render-optimization-backlog.md`）明确本项取代 ADR-0007 deferred 的
  「解析弧」：SDF 是更强形态。

## 决策

### 1. 实例布局：`ArcInstance`（72 B，WebGL2 安全）

沿用 `LineInstance`（`plan.rs`）的**实例率顶点属性**先例（**不是** VS storage buffer，
故 WebGL2 安全）：

| 字段 | 布局 | 含义 |
|---|---|---|
| `center_r` | `vec4<f32>` | xyz = 圆心（世界 − chunk origin，f64→f32），w = 长半轴半径 `r` |
| `uvec` | `vec4<f32>` | xyz = 单位长轴方向，w = 起始角（rad，归一化到 `[0, τ)`） |
| `vvec` | `vec4<f32>` | xyz = 短半轴（共轭）半直径向量，w = 有符号扫掠角（`|≤τ|`） |
| `rgba` | `vec4<f32>` | 与 `LineInstance` 相同的净化（有限、钳制 `[0,1]`） |
| `id_flags` | `vec2<u32>` | [0] = pick key，[1] = `INSTANCE_FLAG_*` \| `ARC_FLAG_FULL_CIRCLE`（`1<<3`） |

参数化：`p(t) = center + (r·uvec)·cos t + vvec·sin t`。仿射变换保持 `t`（slice A 的
`DisplayPrimitive::Conic` / `Transform3::apply_vector` 已保证），故 INSERT 展开只改
`center/major/minor`，`start/sweep` 原样。

`ARC_FLAG_FULL_CIRCLE = 1 << 3` 复用 `INSTANCE_FLAG_*` 位域（`INSTANCE_FLAG_OPAQUE=1<<0`、
`HAS_SOURCE=1<<1`、`LINEWEIGHT=1<<2`）。

**退化**：`sweep == 0 && !full` → 打包时跳过并**计数**
（`Renderer::arc_degenerate_skips`，绝不伪造、也不静默）。该情形在发射侧已被排除（见 §6），
打包侧只作纵深防御。

### 2. 着色器（slice 1 = 圆 + 正交 u/v 的弧；椭圆是 slice 2，**不在本轮**）

在 u/v 帧里 `q = p_local − center`：

```
x = dot(q, uvec); y = dot(q, normalize(vvec));
d_ring = length(vec2(x, y)) − r;
t = atan2(y, x); rel = wrap_tau(t − start);            // [0, τ)
inside = full || (sweep ≥ 0 ? rel ≤ sweep : τ − rel ≤ −sweep);
d = inside ? abs(d_ring) : min(dist to the two endpoints);
coverage = saturate(1.0 − d / max(w, 1e-8));           // w = 1 world-pixel
```

- `d` is the **unsigned** distance to the drawn curve (a true distance field with
  unit gradient). `w` is one screen pixel in world units (`world_per_px` from the
  camera uniform; `fwidth(d)` is only the fallback). The **triangle filter**
  `1 − d/w` reaches full intensity on the curve and ramps to zero one pixel out.
  This is the fix for the original `saturate(0.5 − d/fwidth(d))`: that is the
  filled-shape boundary formula and, applied to an unsigned distance field, caps
  the peak at **0.5** (a half-intensity hairline). Worse, `fwidth(abs(d_ring))`
  collapses to ~0 wherever the ring straddles a pixel pair symmetrically, which
  discarded the four axis points of a grid-aligned circle (the observed
  dashes/holes). Using the analytic 1-pixel width removes both.
- **墨量语义（width semantics）**：三角滤波的积分墨量恰为半宽 `w`：
  `∫ saturate(1 − |d|/w) dd = w`（三角形面积）。因此 `w = world_per_px` ⇒
  **墨量 = 1.0 px**，与硬别名 1 px 直线的单位长度墨量**逐值相等**（视觉重量对齐）。
  单位梯度 + 正交归一帧（`u` 单位、`normalize(v)` 单位且 `u ⟂ normalize(v)`）保证
  「1 world 单位的 `d` = 1 world 单位的屏幕距离」；两者缺一都会让墨量偏离 1.0。
  数值证明见 `crates/cad-render-wgpu/src/tests.rs` 的
  `sdf_conic_field_is_unit_gradient_and_triangle_ink_is_one_pixel`
  （打包帧正交归一 + 梯度模长 = 1 + 三角滤波墨量 = `w`），GPU 侧见 §8(a′)。
- **AA 取舍（已文档化，非缺陷）**：**峰值与墨量不可兼得**。直线是**硬别名 1 px**
  （峰值 255、墨量 1.0）；弧按**墨量对齐**（墨量 1.0），其峰值随相位在 **0.5–1.0** 之间
  （曲线正好落在两像素行/列之间时 0.5，正好穿过像素中心时 1.0），笔画像素均值 ≈0.5
  （≈185/255 sRGB）。这是**正确的抗锯齿**，**不**追峰值 = 255（那会牺牲墨量、把弧压得比
  直线更轻）。
- **sRGB 积分陷阱（取证教训）**：把**sRGB 编码通道**直接求和当作「墨量」会**高估**软边笔画
  ≈√2：理想 1 px 三角滤波在 sRGB 下积分 1.38–1.47，而硬别名 1 px 直线恒为 1.00。
  屏幕先把 sRGB 解码为线性光、人眼再积分，故**物理墨量必须在线性覆盖率上积分**
  （`tests/sdf_arcs.rs::rendered_coverage` / `coverage_at`）。图像取证报出的「弧 1.40–1.46 px、
  √2 偏宽」即此伪影，**不是**渲染缺陷：同一帧线性墨量为 1.000（见 §8(a′)）。
- `fwidth(d)` **必须在统一控制流中计算**（不能在 `discard` 之后）；`if coverage <= 0.0
  { discard; }` 是**强制**的：一个实例 = 一个包围四边形，若不 `discard`，四边形的**每一个**
  像素都会写深度并抹掉其后的几何。
- 端点距离在 u/v 帧里算：端点角 `start`、`start+sweep`，位置 `(r cos te, r sin te)`。
- `arc.wgsl`（颜色）+ `pick_arc.wgsl`（id pass）：pick **光栅化同一 SDF**，在
  `coverage < 0.5` 处 `discard`。因为峰值现在是 1.0，`coverage ≥ 0.5` 恰好等价于
  `d ≤ 0.5·w`，即**±0.5 px 拾取容差**（设计字面值，不再是偏离）。

### 3. 管线 / 抗锯齿决策（选项 (b)，已定）

- 深度 `LessEqual` + `depth_write = true` + `ALPHA_BLENDING`（复用既有混合状态）；**无 MSAA**
  （渲染器 `sample_count == 1`）。
- **已知、文档化限制**：部分覆盖的边缘片元仍写**满深度** → 在**深度不同（透视）**的域里，
  约 1 px 的 halo 可能遮挡其后几何；该域已被「per-chunk f64 origin 限定于 Ortho」的既有
  fence 隔离，**不外推为等价**。
- **软回退**：翻成「硬边」变体（相同的布局，把 coverage 阈值化成 0/1 discard）。
- **不**把弧移入透明类（会破坏跨类绘制顺序语义）。

### 4. 线宽：hairline

携带 `w`，**从不光栅化**（渲染器今天不画任何线宽）。`render.lineweight_not_drawn` 上报必须
覆盖弧：当 `w > 0` 时置 `INSTANCE_FLAG_LINEWEIGHT`。粗线形随全局线宽特性一起推迟。

### 5. 精度：与线相同的 chunk 纪律

- 与 `build_instance_chunks` 同款：上传时对圆心做 `(world − chunk origin)` 的 **f64 约简**后
  再窄化到 f32；chunk 按**范围 AABB**（`center ± |r·u| ± |v|`）切分，受 `MAX_STREAM_RADIUS` /
  `MAX_INSTANCES_PER_STREAM` 约束；超大 conic 自成一块，origin = 范围中点。
- 相机 uniform 路径**不变**。
- f32 半径/角度的 ulp 与今天的弦顶点**同量级**（记于本文；不宣称与弦逐字节一致）。

### 6. 发射（翻转 slice A 的预留门）

- `provider.rs` 的 `expand_entity` Conic 分支：对 **eligible** conic（不透明 + 连续线型 +
  轴非退化 **+ slice-1 圆形/弧**：长短半轴等长且正交，且非 `sweep==0 && !full`）**推入变换后
  的 `DisplayPrimitive::Conic` 片段**，不再 `emit_polyline`；**非 eligible 仍弦展开**，并经
  `ConicStats` **计数**（`record(geometric, eligible)`：几何资格与风格/钩子门分开，无静默丢弃）。
  - **椭圆**（长短半轴不等）是 slice 2，本轮**继续弦展开**：slice 1 的 SDF 把 `vvec`
    归一化后按圆半径 `r` 计算环距离，对椭圆不是正确的 SDF，故不发射。
- `cad-scene`：
  - 新增 `RenderTopology::Conics` 与 `RenderBatch.conics: Vec<RenderConic>`（**f64 语义
    记录**：`center/major/minor/start/sweep/full`）。batch 仍保留
    colour/alpha/lineweight/sources/draw_order（与 line 批次一致）。
  - `cache.rs` 的 conic 分支：由 extent 求 centroid/bounds；`build_compact` 对**相邻同风格**
    conic 片段做合并（复用 line 合并谓词的风格比较）。
- `plot_vector.rs` / `layout.rs` / `highlight.rs` **保持弦展开**（SDF 是光栅绘制，矢量出图/
  版面/子元素高亮不受影响）。slice A 已让 `plot_vector` 走弦。

### 7. 集成

- **flags**：弧 shader 复用**同一个 group-1 flags bind group**（`FLAG_SELECTED`/`FLAG_HOVERED`
  tint，`FLAG_HIDDEN` discard）。
- **id pass**：`PickDraw::ArcRun` 镜像 `LineRun`，**包括** `base + run.offset` 的重定基约定
  （`pick.rs` 刚修的 bug，不得回归），**不合并**。
- **coalescing**：帧遍历里新增**第三个 pending-run 槽**；弧 run **只与同一 stream 内相邻的弧
  run** 合并；遇到管线切换（legacy/mesh/line）时按计划顺序 flush pending run。加速档的**残留
  路径**同样遵守此规则：`draw_batch`（legacy）之前先 `flush_pending_arc`，因此一个 legacy
  batch 不会把弧重排到其后，也不会让两侧的弧 run 跨过它合并。
- **字节预算**：`upload_size_bytes` 计入 **72 × instance_count**；progressive 逐实例计
  `vertices += 6·n, triangles += 2·n`（单位变化记于 `docs/render-order.md` 预算节）。
- **TIER**：弧走**通用 per-run 路径**（`PulledArc` 推迟；本轮仅通用档）。加速档激活时，
  弧属于「**pulled 类先于残留**」的既有等价例外：pulled 类先画、残留弧后画，因此
  **弧-vs-已拉取类**的等深并列**恒由弧胜**（与计划顺序无关），不同于通用档（计划顺序裁决）。
  此差异已记入本 ADR 并 fence（§8(e) 的 `accelerated_sdf_arc_vs_pulled_class_tie_is_the_
  documented_exception`）。**不**把弧塞进拉取结构（非平凡，且会扩大已知例外面）。残留路径
  本身（弧 vs legacy batch 的等深并列）**仍按计划顺序**，由
  `accelerated_sdf_arc_residual_path_and_legacy_tie_match_universal` fence。

### 8. Fences（全部真实 GPU，`--test-threads=1`）

- **(a) coverage-vs-analytic**：在 ~1e5–1e6 世界坐标上布网格圆心/半径/扫掠（full、近 τ、
  顺时针）/平面；采样落在 **f64 解析曲线**上的像素 → coverage **≥ 0.9**（三角形滤波的设计
  峰值）；偏离 **≥1.5 px** → **0**；±1 px 带容忍；含 chunk 边界用例。
- **(a′) 墨量宽度对齐（forensics-grade）**：同帧 SDF 弧 + 硬别名 1 px 直线；沿 48 个角度对
  **线性覆盖率**积分得笔画墨量（px），断言弧墨量 ∈ **[0.85, 1.15]**、且与直线墨量相差 ≤15%。
  真正的宽度尺度回归（`w = √2·world_per_px`、或非正交归一帧）在此失败；sRGB 伪影不会。
  （数值：弧 mean 0.987 [0.897, 1.059]，直线 1.000。）
- **(b′) 逐相位峰值边界（forensics-grade）**：360 个相位取样环上最近像素的峰值覆盖率，
  断言 **min ≥ 0.45**（几何下界 0.5 留 8-bit 余量）、**max ≤ 1.0**、且 max ≥ 0.9、min < 0.9
  （峰值确实随相位变化，不是硬边）。（数值：mean 0.865 [0.723, 1.000]。）
- **(c′) 墨量连续性（forensics-grade）**：360 个相位的积分墨量，断言**无任何样本低于中位数的
  50%**（B1 的轴点空洞、半强度段都会失败）。（数值：median 0.997，min 0.892。）
- **(b) chord-baseline containment**：当前弦细分的**每个顶点**都落在 SDF ≥0.5 覆盖带内
  （关节无缝隙）。
- **(c) id-pass parity**：可拾取 ⇔ coverage ≥ 0.5，**双向扫描**（跨越 0.5 阈值两侧各取样）。
- **(d) flags**：弧上的 tint/hidden。
- **(e) draw order**：弧-vs-线 等深 tie（计划顺序裁决）在**两个交点、两种顺序**下取样；
  弧-vs-mesh（弧胜 tie，如线）。**加速档**的 tie 例外单独 fence（`accelerated_tier.rs`：
  pulled 类先于残留 → 弧恒胜；残留 vs legacy 按计划顺序且与通用档逐字节一致）。
- **(f) determinism + pan/zoom 零弧流重传**。
- **(g) wgsl_validation + 弧 shader 的 GLSL-ES-300 回译**（`pick_arc` 是 native-only，像
  `pick_line` 一样排除出 GLSL 列表）。
- **(h) 视觉验收**：把真实**混合弧+线** fixture 渲进**新目录** `/tmp/opencode/yacr-sdf-arcs-v3/`
  （before=弦，after=SDF；直线两帧相同，用于比对权重）——这是本切片**授权的语义变化证据**（弧
  更平滑、不透明、连续）；断言弧环连续（无缺口）、核心达到满不透明、**墨量**与直线相差 ≤15%、
  且每个采样角度墨量 ∈ [0.85, 1.15] px；**任何地方都不得**做与弦的逐字节一致声明。

### 9. 停止 / 回退

- 发射谓词是**唯一门**（测试有 force-chord 钩子）。
- **软停**：硬边变体。
- **硬回退**：关掉 Conic 发射 → 弦，渲染器零清理。
- 任何精度 fence 在大坐标失败 = **回退**，**绝不**放低精度底线。

### 10. 不做（显式）

- 从弦反推弧参数；GPU 曲面细分半吊子；通用档顶点级 storage buffer；弧像素的逐字节一致声明；
  在线上仍 hairline 时给弧光栅化宽度；把虚线/非连续 conic 路由进 SDF（弦+虚线路径保留并
  计数）；静默丢弃退化情形；`rebuild_pulled` 的改动超过 tier 集成的最小面。

## 后果

- 圆/弧不再占用 line 实例、不再有弦折角；SDF 抗锯齿分辨率无关。
- **语义像素变化**（更平滑）已获授权；**不**声称与弦逐字节一致。
- slice 2（椭圆）与 `PulledArc` 推迟；加速档的弧类 tie 例外已文档化并 fence。

## 证据边界（诚实）

- 真实 GPU（默认 Vulkan ICD，运行适配器标注）逐用例运行；软件后端（lavapipe/SwiftShader）
  产出的数字/截图显式标注软件渲染。
- **未**做浏览器/WebGL2/Android/vendor/golden 验收；**未**做椭圆 SDF。
- 视觉验收目录 `/tmp/opencode/yacr-sdf-arcs-v3/`（before 弦 / after SDF）是语义变化证据，
  非逐字节一致证据。

## 实施记录（2026-10-15，真实 GPU）

实现落在 `crates/cad-representation`（发射门）、`crates/cad-scene`（`RenderTopology::Conics`
+ `RenderBatch.conics` + `RenderConic` + cache conic 分支/compact 合并）、
`crates/cad-render-wgpu`（`ArcInstance` 72 B、`build_arc_chunks`、`shaders/arc.wgsl` /
`pick_arc.wgsl`、arc 管线、帧遍历第三个 pending 槽、id pass `PickDraw::ArcRun`、预算/统计）。
`cad-app` / `cad-cli-tools` 只做了加字段/补 match 臂的机械改动（`RenderBatch` 新字段与
`RenderTopology` 新变体的编译涟漪）。

### 实施中确认/细化的点

- **slice-1 资格 = 圆形**：`ConicSegment::is_sdf_circle_arc()` 要求 `|major| == |minor|` 且
  `major ⟂ minor` 且非 `sweep==0 && !full`。**椭圆（长短半轴不等）不发射 SDF**（slice 2）。
  资格在**变换后**的 conic 上判定：非均匀 INSERT 会把圆变成椭圆，必须走弦。
- **发射门翻转**：`provider.rs` 的 `expand_entity` Conic 分支对 eligible conic 推入**变换后的
  `DisplayPrimitive::Conic`**；其余仍 `emit_polyline` 弦展开。`ConicStats::record(geometric,
  eligible)` 把**几何资格**（`is_sdf_circle_arc`）与**风格/钩子门**分开计数：`sdf_geometric()`
  记几何事实，`sdf_eligible()` 记实际发射决定（无静默丢弃）。
- **force-chord 钩子**：`RepresentationContext::with_force_chord_conics()`（软回退杠杆），
  测试覆盖：钩子把 eligible 圆路由回弦，且与旧弦路径逐点一致。

### 与本文设计字面的**有意偏离**（诚实记录）

1. **四边形扩展在 world 空间（相对 2%，且下限为 ~2 world-px）而非 clip 空间**。设计原文的
   `clip.xy += corner * MARGIN * clip.w` 会**拉伸插值出的 `world_pos` 场**：顶点 world 位置
   不变而屏幕边界外移，环随之外移（实测半径 80 px → 87 px，且中带丢像素）。改为
   `world = center + corner.x*(grow*r*u) + corner.y*(grow*v)`，其中
   `grow = 1 + max(0.02, 2·world_per_px / r)`，保持仿射一致；SDF 丢弃溢出部分。
   **相对下限**：纯 2% 边距在屏幕半径 ≲ 50 px 时不足以容纳 ~1 px 的 AA 外沿，会在四个轴点
   （四边形边距环最近处）削掉外沿；`2·world_per_px` 的绝对下限修正之（fence：
   `sdf_arc_small_circle_fringe_survives_zoom_out`，半径 40→3 px 扫描）。
2. **ramp 宽度用解析的 1 world-pixel（`world_per_px`）而非 `fwidth(d)`**。设计字面的
   `coverage = saturate(1 − d/fwidth(d))` 有两个实测问题：(i) `fwidth(abs(d_ring))` 在环
   **对称跨越像素对**时塌缩到 ~0（`abs` 的 V 形有限差分相消），使 `d/max(w,1e-8)` 爆掉、
   把网格对齐圆的**四个轴点整段丢弃**——正是复核与图像取证看到的虚线/空洞；(ii) `fwidth` 是
   `|∂d/∂x|+|∂d/∂y|` 之和，在弧端点 medial axis 处可高估到 ~1.6 px。距离场是单位梯度 ⇒
   1 屏像素 = `world_per_px` world 单位，用它作 ramp 宽度既准确又稳定；`fwidth` 仅作 uniform
   未设时的回退。
3. **id pass 的 discard 阈值回到字面的 `coverage < 0.5`**。峰值现在是 **1.0**（不再是 0.5），
   因此 `coverage ≥ 0.5 ⇔ d ≤ 0.5·w` 即 **±0.5 px**，与设计声明一致；旧「ink 阈值」偏离取消。
   同理，**弧-vs-线等深并列现在是不透明替换**（按计划顺序），旧「混合/hairline」记录作废。
4. **墨量语义的数值闭环（本轮新增）**。图像取证一度报「弧墨量 1.40–1.46 px vs 直线 1.00 px、
   √2 偏宽」，并据此怀疑 `world_per_px` 用了 `hypot(sx, sy)`、或 `uvec`/`normalize(vvec)` 帧
   非正交归一、或三角半宽语义错位。逐项排除：
   - `renderer_impl.rs` 的 `key_flags.z` 写的是 `projection.world_per_px()`（正交档即
     `camera.world_per_px`），**不是** `hypot`；且正交档 `sx = 2/(w·wpp)`、`sy = 2/(h·wpp)` ⇒
     **无论宽高比**，1 像素 = `wpp` world 单位（各轴一致），标量宽度正确；
   - 打包帧正交归一：`u` 单位、`v` 为完整半直径向量（`|v| = r`）、`normalize(v)` 单位且
     `u ⟂ normalize(v)`，故 `d` 是**单位梯度**距离场；
   - 三角半宽语义正确：墨量 = `w`，`w = world_per_px` ⇒ 墨量 = **1.0 px**。
   结论：**渲染正确**；1.40–1.46 是把 **sRGB 编码通道**直接积分的伪影（理想 1 px 三角滤波在
   sRGB 下积分 1.38–1.47，硬别名 1 px 直线恒 1.00）。数值证明：
   `src/tests.rs::sdf_conic_field_is_unit_gradient_and_triangle_ink_is_one_pixel`（打包帧正交
   归一 + 64 角梯度模长 = 1 + 三角滤波墨量 = `w`）与 §8(a′) 的真实 GPU 线性墨量对齐
   （弧 mean 0.987 / 直线 1.000）。**未改任何渲染常数**（默认目标语义 = 现状）。

### 实测（真实 NVIDIA Quadro P620 / Vulkan，未设 `VK_ICD_FILENAMES`，运行适配器
`AdapterInfo { backend: "vulkan", name: "Quadro P620", device_type: "discrete_gpu" }`）

`crates/cad-render-wgpu/tests/sdf_arcs.rs`（`--test-threads=1`，**13/13 + 1 ignored** 全绿）：

- **(a) coverage-vs-analytic**：~1e5–1e6 世界坐标上的圆/弧（full、near-τ、顺时针）逐像素与
  **f64 解析曲线**比对；`d_px ≤ 0.05` 的像素 coverage **≥ 0.9**，`d_px ≥ 1.5` 严格为背景
  （≤0.02），峰值 ≥ 0.9。
- **(b) chord-baseline containment**：当前弦细分的每个顶点在 5×5 邻域内都有 ink（关节无缝隙）。
- **(c) id-pass parity**：沿一条穿越环的水平扫描线**双向**扫描；`coverage ≥ 0.55` 的像素可
  拾取、`coverage ≤ 0.45` 的像素不可拾取，阈值两侧都有样本；背景像素不可拾取。
- **(d) flags**：`FLAG_SELECTED` 换色、`FLAG_HIDDEN` discard 在弧上生效。
- **(e) draw order**：弧-vs-线 等深并列由**计划顺序**裁决，在**两个交点、两种顺序**下取样
  （线后画→蓝胜；弧后画→**红胜的不透明替换**）；弧-vs-mesh 等深并列弧胜。
- **(f) determinism + pan/zoom**：同一场景逐像素确定；5 次相机移动 `arc_stream_uploads` 不变。
- **(g) wgsl_validation**：`arc.wgsl` 通过 naga 校验并成功回译 GLSL ES 3.00（WebGL2 安全声明
  被测试，不是假设）；`pick_arc.wgsl` 是 native-only，像 `pick_line` 一样排除出 GLSL 列表。
- **(h) 视觉验收**（`#[ignore]`，`YACR_SDF_ARC_OUT`，默认 `/tmp/opencode/yacr-sdf-arcs-v3/`）：
  **混合弧+线**的 before=弦 / after=SDF 两帧写入**新目录**；断言弧环连续（144 角无缺口）、
  核心满不透明（ring peak=1.000，与直线 core=1.000 一致，≤20% 差）、**线性墨量对齐**
  （ring ink mean=1.000 [0.888, 1.134] px vs 直线 ink=1.000 px，每个采样角 ∈ [0.85, 1.15]）、
  灰阶数 ≥ 弦（实测 chord=118 / sdf=137）。
- **新增 forensics-grade (a′) 墨量宽度对齐**：48 角线性墨量（弧 mean 0.987 [0.897, 1.059] vs
  直线 1.000，差 ≤15%）。
- **新增 forensics-grade (b′) 逐相位峰值边界**：360 相位峰值 mean 0.865 [0.723, 1.000]，
  min ≥ 0.45、max ≤ 1.0、max ≥ 0.9、min < 0.9。
- **新增 forensics-grade (c′) 墨量连续性**：360 相位墨量 median 0.997、min 0.892，无样本
  低于中位数 50%。
- **新增 cross-class 强度一致性**（捕获 B1 的 fence）：同色圆（SDF）+ 椭圆（弦）+ 直线（弦）
  同帧，三类核心峰值均 1.000，最轻类 ≥ 0.8×最重类；并逐角检查圆无缺口。
- **新增 small-circle margin/zoom-out 扫描**（S3）：屏幕半径 40→3 px，四个轴点的外沿范围与
  45° 参考一致（无削顶）。
- **新增** `non_opaque_conic_batch_is_rejected_at_upload`（S4）与
  `degenerate_conic_skip_is_counted`（N2）。

`crates/cad-render-wgpu/tests/accelerated_tier.rs`（**23/23**）含新增两条加速档 fence：
`accelerated_sdf_arc_residual_path_and_legacy_tie_match_universal`（残留弧路径 + legacy batch
等深并列按计划顺序，且与通用档逐字节一致；`arc_draws > 0`）与
`accelerated_sdf_arc_vs_pulled_class_tie_is_the_documented_exception`（pulled 先于残留 ⇒
弧恒胜，与通用档的计划顺序裁决不同——即文档化的 tie 例外）。

既有渲染器套件（lib 120 / accelerated 23 / headless 19 / pick_id 32 / render_effects 44 /
wgsl 10）在本切片改动后**全部保持通过**；`cad-representation` 186 通过。
scoped rustfmt 通过。

### 诚实边界

- **未**做浏览器/WebGL2/Android/vendor/golden 验收；**未**做椭圆 SDF（slice 2）、
  `PulledArc`、透视域精度。
- 加速档下弧走通用 per-run（残留）；「弧 vs **已拉取类**」等深并列**恒由弧胜**（pulled 先于
  残留），与通用档的计划顺序裁决不同——已 fence（§8(e)）。残留路径内部（弧 vs legacy batch）
  仍按计划顺序，由 accelerated fence 覆盖。
- **弧-vs-线等深并列现在是不透明替换**（峰值 1.0，按计划顺序），已 fence（§8(e)）。
- (h) 的 before 用**渲染器层的弦批次**（`ConicSegment::chord_points()` → `Lines`），不是
  provider 的 force-chord 钩子（`cad-render-wgpu` 不依赖 `cad-db`）；钩子本身由
  `cad-representation` 的单测覆盖。
