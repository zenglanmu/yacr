# 三维拾取（screen ray → hit test，F14/F05）

规范 v2.0 §3.1 F14 / §8.5；审计 F05（INSERT 实例区分）与 F14（三维直接几何拾取）。
本文记录 **CPU 精确拾取** 的实现契约、容差来源，以及**明确未实现**的部分。GPU 侧
渲染边界见 `docs/render-3d.md`，相机/投影见 `docs/view-3d.md`。

拾取是**纯 CPU 几何**，不依赖 GPU 回读（规范 §8.5），因此在 WebGL2 后端与无设备的
宿主中行为一致。实现分为两层：

- `cad-spatial::pick`：与数据库无关的射线命中数学（线段距离、三角形求交、最近命中合并）；
- `cad-app::picking`：把相机像素变成射线的数据库侧装配（遍历 model space、展开 INSERT、
  累积 `InstancePath`、从 `TolerancePolicy` 推导世界容差）。

## 管线

```
逻辑像素 ──Camera::screen_to_ray──▶ 世界 Ray3
                                    │
      cad-app::drawing_pick_items ──┤ 每个 PickItem = SelectionRef + 几何 + 累积变换
                                    ▼
                        cad_spatial::pick_closest
                                    │
                        最近的 PickHit + skipped[]（原因）
```

`pick_at_screen` 是端到端入口：像素无效/视口退化 → `InvalidInput`（绝不返回空命中
冒充成功）；射线或容差退化 → `InvalidInput`；其余情况返回
`PickReport { hit: Option<PickHit>, skipped: Vec<SkippedGeometry> }`。

### 深度排序

按**沿射线的距离**取最近命中，而不是第一个找到的候选：

- `distance` 是命中点沿单位射线方向的参数（`>= 0`）；
- `offset` 是命中点到射线的**垂直距离**，仅用于**同深度**（同 `distance`）时的排序，
  使同一平面上的两个候选按射线真正接近程度决胜。

`PickHit.source` 是完整的 `SelectionRef`：`document + entity + instance + sub_element`。

### 实例区分（审计 F05）

`INSERT` 通过块定义展开，累积变换为 `父变换 × 插入变换`，插入链记为子实体的
`InstancePath`（由外到内的插入实体 id）。因此**同一块的两次放置**产生两个身份不同的
`SelectionRef`（`instance` 不同），命中携带对应路径，两个实例可分离。
展开受 `cad_db::MAX_INSTANCE_DEPTH` 限制并切断环，与 `DrawingDatabase::bounds` 同规；
超出深度或成环的分支**被跳过**，不产生命中。

## 容差

世界容差由 `cad_app::picking::pick_tolerance` 从 `TolerancePolicy` 推导：

| 投影 | 每逻辑像素的世界尺寸 | 世界容差 |
|---|---|---|
| 正交 | `scale`（世界单位/像素） | `interaction_logical_pixels × scale` |
| 透视 | `2·d·tan(fov/2) / height`，`d` 为相机到目标距离 | `interaction_logical_pixels × 该值` |

结果下限为 `TolerancePolicy::computation_world`（拾取绝不比数值谓词容差更紧），且必须
有限且为正；视口尺寸或投影退化 → `InvalidInput`。

- **线段/曲线**：命中条件为点到射线距离 `≤ tolerance`（`ray_segment_closest`）；
- **三角形**：Möller–Trumbore，命中点必在射线上，`offset = 0`。

## 背面策略（显式）

`BackFacePolicy` 必须由调用方显式给出：

- `Cull`（默认，匹配渲染器 `cull_mode = Back`）：世界外法向背向射线原点的三角形被忽略；
- `DoubleSided`：单面片/开放面两侧都算命中。

法向在**世界坐标**上用三角形绕序叉积计算，所以镜像变换（负行列式）导致的绕序翻转
不会把背面误判为正面。

## 支持与明确跳过

| 几何 | 处理 | `precision` |
|---|---|---|
| `Line`、`Point` | 解析线段/点距离 | `Analytic` |
| `Polyline`（含 bulge） | 离散为折线后逐段测距 | `Approximate` |
| `Circle`、`Arc`、`Ellipse`、`Spline` | `cad_geometry::tessellate_geometry` 离散后逐段测距 | `Approximate { error_bound = tolerance }` |
| `Text` | 同上（占位框边，非真实字形轮廓） | `Approximate` |
| `Mesh` | 逐三角形 Möller–Trumbore，取最近 | `Analytic` |
| `Compound` | 递归取最近子命中；至少一个子几何可测即视为可测 | 取子命中精度 |
| `Insert` | **本层不测**，由 `cad-app` 展开后逐子几何测试 | — |
| `Opaque` | **跳过**，`skipped.reason = "opaque"` | — |

> `Compound` 中不可测试的子几何（如 `Opaque`）若同组另有可测子几何，则只报告命中，
> 不单列该子几何的跳过原因（`pick_closest` 的 `skipped` 是条目级）。这是已知的信息
> 粒度限制，不是伪造命中。

`skipped` 中的条目带机器可读 `reason`（永不是翻译散文），与 `Miss` 严格区分：跳过
表示"无法精确测试"，`Miss` 表示"测试了但没命中"。**任何情况下都不会为不支持的几何
伪造命中。**

## 空间索引

`cad_spatial::GridSpatialIndex` 只做 AABB 粗筛（`ray_candidates`）。
`cad_app::picking::filter_by_index` 按**完整身份**（entity + instance + sub_element）
把候选精确映射回 `PickItem`，不会用同块的另一个实例顶替；精确阶段随后在候选上运行。
`sensor` 级精度由精确阶段保证，索引不改变结果。

## 明确未实现（不是空成功）

- **GPU 深度测试对照**：拾取是 CPU 侧、基于语义几何的结果，**未**与 GPU 深度缓冲/
  深度回读比对。二者在理想情况下应一致，但本环境**无法运行 GPU**，不宣称一致。
- **曲面精确求交**：圆/弧/椭圆/样条按弦高容差离散后做线段测距，属于**近似**命中；
  报告 `Precision::Approximate`，并在 `error_bound` 中给出离散容差。真正的解析曲线
  求交未实现。
- **文字字形轮廓**：`Text` 使用占位框边，不是真实排版轮廓。
- **子元素（mesh 面/edge）选择**：命中始终 `sub_element = None`；`Mesh::face_sources`
  已存在但尚未映射到 `SubElementId`。
- **mesh 几何来源**：数据库不携带网格来源（Direct/Kernel/Proxy），`PickItem` 对 `Mesh`
  统一标 `GeometrySource::DirectMesh`，其余标 `Analytic`；这是**保守标签**而非实证来源。
- **OBB / 变换后紧包围盒索引**：索引按世界坐标 AABB，未做实例级 OBB 剔除。
- **宿主接线**：把画布点击接到 `pick_at_screen`（并去掉逻辑/物理像素与安全区的歧义）
  仍属宿主（`apps/**`）与交互工作流，未在本轮完成。

## 测试

`cad-spatial`：
`segment_hit_and_miss_with_tolerance`、`tolerance_scales_the_segment_hit_window`、
`segment_behind_the_origin_is_a_miss`、`mesh_triangle_hit_respects_back_face_policy`、
`mesh_pick_returns_the_closest_triangle`、`closest_of_many_is_returned_not_the_first`、
`instance_path_is_carried_on_the_hit`、`opaque_geometry_is_skipped_with_a_reason`、
`insert_is_not_silently_claimed_as_a_hit`、`degenerate_ray_and_tolerance_are_refused`、
`transform_is_applied_before_testing`、`circle_is_tessellated_and_marked_approximate`。

`cad-app`：
`insert_expansion_carries_distinct_instance_paths`、`two_insert_instances_are_separable_by_the_hit`、
`screen_pick_uses_camera_ray_and_returns_closest`、`perspective_pick_hits_3d_geometry_along_the_view_ray`、
`screen_pick_misses_empty_space_without_a_wrong_hit`、
`pick_tolerance_scales_with_orthographic_zoom`、
`pick_tolerance_scales_inversely_with_perspective_height`、
`pick_tolerance_never_drops_below_the_predicate_tolerance`、
`degenerate_viewport_has_no_pick_tolerance`、`invalid_screen_pixel_is_rejected_not_empty`、
`filter_by_index_keeps_identity_exact`；集成契约
`screen_pick_produces_a_selection_ref_the_select_command_accepts`（`tests/contracts.rs`）。
