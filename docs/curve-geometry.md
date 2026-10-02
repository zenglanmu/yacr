# 曲线几何：NURBS、椭圆与 OCS、仿射锥线、解析交点

审计 B23（数值/变换/样条/椭圆语义）与规范 §16.1 的落地说明。权威仍是
`CAD_IMPLEMENTATION_SPEC.md` v2.0；本文记录**现在精确到什么程度**、近似/不支持在哪里，
以及验证入口。纯 f64，不引入外部数学库。

## 1. 有理 B 样条（NURBS）

`cad_geometry::NurbsCurve`（`crates/cad-geometry/src/nurbs.rs`）持有：

- `degree`、`control_points`、`knots`（长度 `n + degree + 1`）、`weights`（长度 `n`，缺省全 1）；
- `evaluate(t)`：齐次 de Boor + 透视除法；
- `tangent(t)`：齐次导数曲线（对 `A(t)/w(t)` 用商法则）得到一阶导；
- `discretize(tolerance, params)`：按**弦高**在每个非空 knot 区间内递归二分，采样随几何
  而非固定参数步长；`max_segments` 为硬上限，端点始终保留。

`NurbsCurve::new` 拒绝非有限控制点/knot、非单调 knot、非正或非有限权重、空控制多边形、
长度不匹配与空 knot 域——显式报错，不生成假曲线。

`SemanticGeometry::Spline` 的 tessellation 现在构造 `NurbsCurve` 并做自适应离散：

- 源 knot 向量与权重**权威**，不再统一化或忽略；
- 输入畸形（knot 长度/单调性、权重、degree）回退到 clamped-uniform，并不静默声称精确；
- 导入器 `spline_semantics` 对忠实可表示者报 `Complete`；仅当缺控制点（fit-point-only）
  或 knot/权重向量与控制多边形长度不符时报 `Partial` 并给出原因。

已知值测试（`cad-geometry/src/nurbs.rs`）：有理二次四分之一圆（圆心原点、半径 1，
权重 `1, 1/√2, 1`）逐点精确在圆上；导数与有限差分一致；clamped 三次 B 样条端点插值。
固定容差下更细的 tolerance 产生更多采样。

**精确 vs 近似**：多项式/有理 NURBS 的求值与导数精确（浮点舍入内）；离散是**近似**，
近似误差受 `tolerance` 控制（测量/交点不消费离散，见 §5）。

## 2. 椭圆与 OCS

`SemanticGeometry::Ellipse` 新增 `normal` 字段（ADR 0004），约定
`minor = cross(normal, major_axis)`。世界 Z 椭圆即 `normal = +Z`，与旧 `cross(world_z, major)`
约定逐位一致。

- 反序列化/tessellation 在**椭圆自身平面**内取 minor 轴，任意倾斜 OCS 平面不再被压到世界 XY。
- 导入器 `ellipse_semantics` 保留 `normal`；倾斜椭圆现在是 `Complete`，不再是
  “压平但声称 Partial”的半成品；退化法向回退到 `+Z`，零/非有限轴或比例报 `Partial`。

OCS 顶点/中心：

- LWPOLYLINE / 2D POLYLINE：`polyline_ocs_points` 经 AutoCAD arbitrary-axis 算法映射到 WCS
  （世界 Z 走精确恒等路径）。
- **CIRCLE / ARC**：acadrust 的 center 存于 OCS，导入改用 `center_wcs()`（等价于
  `ocs_to_wcs`）；`normal` 与角度在 `arbitrary_axis` 重建的同一 OCS 框架中解释，因此倾斜圆/弧
  落在自身平面且方向正确。

## 3. 非均匀仿射变换

`DefaultGeometryEngine::transform`：

- Circle/Arc 在相似变换（`is_uniform_scale`）下仍是圆/弧；**镜像**（负行列式）翻转参数方向，
  `oriented_after_reflection` 取反 `start`/`sweep`，避免画到补弧。
- 非均匀变换走一般仿射锥线路径：
  - `circle_to_geometry` / `arc_to_geometry` → `affine_ellipse_arc`；
  - `affine_ellipse_arc` 对 `M = [A·u, A·v]` 做 `MᵀM` 的 2×2 特征分解，得到主轴
    （`s1 ≥ s2`）、比例 `s2/s1`、法向 `w1×w2` 和参数平移 `−θ`。任意仿射像都保持为**精确椭圆**，
    并保留部分弧的 `sweep`。
  - 秩退化（平面塌缩）返回 `Point`（ratio→0 的退化椭圆/点），不伪造圆。
- Ellipse 在相似变换下直接缩放轴、旋转法向、保留比例；镜像同样翻转参数方向。
- **Bulge 在非均匀缩放下不再被清零**：`transform_polyline_affine` 把每个 bulge 段还原成
  源平面内的精确圆弧（`bulge_arc_geometry`），再经上面的锥线路径变换，结果是一个
  `Compound`（直线段 + 椭圆弧段）。相似变换下 bulge 保留，镜像取反 bulge 符号。
  两顶点倾斜 bulge 没有唯一平面，导入器仍报 `Partial`。

**精确 vs 近似**：小线段/圆弧在仿射下精确（浮点内）。只有平面塌缩时才出现退化点。

## 4. 解析交点

`intersect_local` 对两个锥线图元先走解析路径（`analytic_intersections`）：

- segment/segment（任意 3D，最近点法，非 XY 投影）；
- segment/circle、segment/arc（解析 + sweep 过滤）；
- circle/circle、circle/arc、arc/arc（同平面；不同平面回退采样）；
- segment/ellipse（投影到椭圆平面解二次，再校验共面与 sweep）。

容差来自 `TolerancePolicy::computation_world` / `topology_world`，**与 `display_pixels` 无关**。
对样条或不支持组合，回退到以 world 计算容差的自适应采样（同样不看显示 LOD）。

## 5. 显示 LOD 与测量语义分离

- `tessellate_curve` / `intersect_local` 只用 `computation_world` 决定弦高与接受容差；
- `display_pixels` 只影响显示用密度（`TessellationParams::from_policy`）；
- 测试 `intersection_is_independent_of_display_lod_for_curves`（`tests/curves.rs`）与
  `intersection_is_independent_of_display_pixel_budget`（`tests/invariants.rs`）锁定：
  改变显示像素预算不改变测量/捕捉结果。

## 6. 验证入口

```bash
cargo test -p cad-geometry --locked
cargo test -p cad-import-acadrust --locked
cargo test --workspace --exclude cad-ui-slint --exclude app-android --exclude app-web --locked
```

参考实现（仅行为，独立重写，未逐字拷贝，GPL-3.0）：

- `/tmp/opencode/ocs` commit `02c470aac7af1f912349ee7fd5ad612863e8b7da`
- `src/entities/curve.rs`：`ocs_plane` / `ocs_axes` 的 OCS→WCS 约定、`ellipse_curve`
  的 `minor = cross(normal, major)` 与 center/major 的投影、`circle_curve`/`arc_curve`
  用 `center.z` 作为 elevation、`spline_curve` 的平面检查。
- `acadrust 0.5.5`：`entities/circle.rs`、`entities/arc.rs` 的 `center_wcs`/`arbitrary_axis`
  框架，`entities/ellipse.rs` 的字段，`entities/transform.rs` 的镜像翻转 bulge 约定。
