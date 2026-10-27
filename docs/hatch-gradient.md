# Gradient HATCH rendering

Status: implemented subset. This document is the authority for which gradient
HATCH kinds render, how they are tessellated, and what remains an explicit gap.
It complements `docs/entity-style.md` (per-entity colour) and
`docs/render-order.md` (draw order); it does **not** modify them.

## 1. What changed

Before this work a HATCH whose `gradient_color` was enabled was imported as
`Completeness::Partial("gradient hatch renders its boundary only")`: only the
boundary loops were drawn and the fill was dropped. Gradient hatches now render
as a real colour ramp clipped to the hatch boundary.

The chosen approach is the **lower-risk per-vertex colour bake**:

1. The existing even-odd `fill_rings` triangulation produces the interior
   triangles (unchanged; this is what clips the fill to the boundary, including
   holes and islands).
2. A pure function, `cad_geometry::gradient_vertex_colors`, samples the gradient
   ramp once per fill vertex and returns one sRGB triple per vertex.
3. The colours travel as an optional attribute on the domain `Mesh`
   (`Mesh::colors`), then on `cad_scene::RenderBatch::colors`, and finally as a
   third mesh vertex buffer read at `@location(2)` by `mesh.wgsl`.

No 1D gradient texture and no fragment-shader gradient math were introduced, so
the existing mesh pipeline keeps its shape. A non-gradient mesh supplies white
vertices, so `tint * vertex_color` reduces to the batch tint and solid meshes
render exactly as before.

## 2. Data model

`cad_geometry::GradientDef` (in `crates/cad-domain/src/hatch/gradient.rs`):

| field | meaning | DXF source |
| --- | --- | --- |
| `kind` | `Linear` or `Spherical` | group 470 name (450 marks the gradient fill) |
| `angle` | ramp direction, **radians** | group 460 |
| `shift` | offset along the ramp, clamped at the ends | group 461 |
| `single_color` | DXF single-colour flag | group 452 |
| `tint` | single-colour tint in `-1..=1` | group 462 |
| `stops` | `{ value: 0..=1, rgb }` entries | groups 463 / 421 |

**The angle is in radians, and must stay so.** Group 460 carries the gradient
angle in radians (`acadrust-0.6.5/src/entities/hatch.rs` documents the field as
"Gradient angle in radians"; the DXF reference agrees, and an earlier version of
this table had the group numbers swapped). `cad-domain`'s
`gradient_angle_is_stored_in_radians_not_degrees` test pins it end to end: a
stored `FRAC_PI_6` bakes a 30° ramp. Do **not** "correct" it into degrees: the
raw value is passed through and interpreted as radians everywhere.

For the record, the neighbouring gradient codes are 451 (reserved), 453 (colour
count), 470 (name) and 463/63/421 (stop value / ACI / true colour).

A similarity transform rotates `angle_raw` by the same in-plane `delta` the
pattern angle uses (`angle_raw += delta`), so a rotated gradient hatch turns its
ramp with the geometry instead of keeping the pre-rotation direction. Group 460
stays a plain radian value; nothing here converts it to degrees.

`GradientDef::is_usable()` requires a finite angle/shift and at least one stop
with a finite `value` in `0..=1`. Unusable definitions are reported, never drawn
solid.

`gradient_vertex_colors(fill, def)`:

* **Linear**: projects each vertex onto `(cos angle, sin angle)`, normalises
  across the boundary extent, adds `shift` and clamps to `0..=1`.
* **Spherical**: `value = |p - centre| / max_radius` using the centre of the
  boundary's bounding box.
* Samples the (sorted) stops with linear sRGB interpolation; a single stop is a
  constant.
* A single-colour gradient is expanded to a two-stop ramp from the listed colour
  to white (`tint > 0`) or black (`tint < 0`).

## 2b. 创建（2026-10-27）：`HATCH` 类型码 `3`

同一套域类型与烘焙也被**创建**路径复用：没有新 `CommandId`、没有新载荷、没有渲染器或写出改动。
能力表（`docs/compatibility.md` 的 HATCH（创建）行）是这里语义的权威；本节只记**创建出来的实体长什么样**。

- **入口**：`hatchPatternType` 闭集在 `{0 solid, 1 user-defined, 2 predefined}` 之外新增 **`3` 渐变**
  （`crates/cad-app/src/draw_tool/parameter.rs`）。四个 `Number` 参数（`cad_set_tool_parameter`）：

  | 键 | 默认 | 校验 |
  | --- | --- | --- |
  | `hatchGradientKind` | `0` LINEAR | 闭集 `{0 LINEAR, 1 SPHERICAL}`；**从不**写 `CYLINDER`（域把它烘焙成球面，一个行为不给两个名字） |
  | `hatchGradientAngle` | `0`（度） | 任意**有限**值；**通道是度**，实体存**弧度** |
  | `hatchGradientColorA` | `1`（红） | **整数 ACI `1..=255`**；`0`＝ByBlock 占位、无 sRGB ⇒ 按名拒绝 |
  | `hatchGradientColorB` | `5`（蓝） | 同上 |

- **写出的实体**：`HatchGradient::TwoColor { reserved: 0, angle_raw: 弧度, shift: 0.0,
  stops: [0.0 → A 的 rgb, 1.0 → B 的 rgb] }`；`pattern_type` 是 **`Predefined`（`76=1`）**，
  `HatchStyle` 按所选写。渐变**不读**图案的间距/角度/`double`：陈旧的非法间距不得拒绝一个渐变，
  反之陈旧的非法渐变颜色也不得拒绝图案。
- **度 → 弧度只转一次**：`app_drawing/hatch_parameters.rs` 的 `.to_radians()` 是本路径唯一的换算；
  **导入**侧按组 `460` **逐字**存弧度、从不换算。两个约定在
  `crates/cad-app/src/tests/drawing/hatch_gradient.rs` 的
  `the_degrees_to_radians_conversion_survives_export_and_reimport`（create → export → reimport）上被钉住
  ——改任何一侧都会让它失败。
- **只有 ACI**：颜色是整数索引，经 `crate::property_value::rgb_for_aci` 解析一次；没有真彩、没有色名/色簿。
  `0` 没有 sRGB 三元组（`rgb_for_aci(0)` 返回 `None`），所以命令**按名拒绝**它，绝不退化成黑色。stop 颜色下游仍会被
  实体/图层 tint 相乘，即渐变颜色不是绝对值。
- **`SPHERICAL` 在矩形/圆上读成平色**（已声明，见 §6 第 3 条）：ramp 按三角化顶点取值，而矩形四角的
  归一化半径相同 ⇒ 常数颜色；三角形或曲线边界才会真正径向。按值细分内部三角形是未来工作。
- **`76=1` 是未查证的 vendor 约定**：渐变按「实体填充 + 450–470 块」写、没有图案定义行，所以
  `pattern_type` 记 `Predefined`；这是**声明**的映射，**没有**用 vendor 生成的文件比对过。
  也因此没有单色/tint 变体（`single_color=false`、`tint=0`）、没有逐站命名。

证据上限：创建路径的证据是 `cargo test -p cad-app` 的契约测试（含上面那条往返钉）；UI 侧的四个控件
只有源码结构/纯函数契约，**没有**挂载后的 DOM、像素或真窗口证据。

## 3. Importer behaviour and reason codes

`cad_import_acadrust::translate_gradient` maps `HatchGradientPattern`:

| gradient name | result | completeness |
| --- | --- | --- |
| `LINEAR` | linear mesh with baked colours | `Complete` |
| `SPHERICAL`, `CYLINDER` | radial mesh with baked colours | `Complete` |
| (empty name, with stops) | treated as `LINEAR` | `Complete` |
| `HEMISPHERICAL`, `CURVED`, `INVSPHERICAL`, `INVCYLINDER` | boundary only | `Partial("gradient_kind_not_supported")` |
| unknown non-empty name | boundary only | `Partial("gradient_kind_not_supported")` |
| empty name, no stops | boundary only | `Partial("gradient_name_missing")` |
| enabled but definition unusable (NaN angle, no/invalid stops) | boundary only | `Partial("gradient_definition_unusable")` |

The gradient is checked **before** the solid flag, because a real gradient
HATCH is stored as a solid hatch with gradient metadata. A gradient is never
substituted with a flat solid fill.

A fill failure (over-budget boundary, degenerate or self-intersecting rings)
reports `Partial` with the underlying `FillError::reason()`; it is never faked.

## 4. Tessellation and attribute approach

* Boundaries and holes come from the same `fill_rings` even-odd triangulation as
  solid hatches. The gradient is applied per extracted vertex, so it is clipped
  to the boundary by construction.
* `Mesh::colors` is empty for every non-gradient mesh (the default), meaning "no
  vertex colour". When present it must be exactly `vertices.len()` long; the
  scene layer sanitises each channel and falls back to the uniform batch colour
  if the length does not match.
* `mesh.wgsl` computes `tint.rgb * vertex_color * (ambient + diffuse)`. For a
  planar hatch facing the camera the diffuse term is 1.0, so the sampled stop
  colours are shown directly. A hatch whose plane faces away is dimmed to the
  ambient floor, exactly like a solid hatch.
* The per-vertex colour is modulated by the entity/batch tint. In practice a
  HATCH resolves `ByLayer` to the default white, so the gradient shows through
  unchanged; a non-white entity colour remaps the ramp's hue. This is documented
  as a limitation below.

## 5. Verification

* Pure-function unit tests in `crates/cad-geometry/src/hatch.rs` cover known
  endpoints, angle rotation, single-colour tinting, stop ordering, `shift`, and
  spherical radius.
* Importer tests in `crates/cad-import-acadrust/src/tests.rs` assert `Complete`
  plus graded vertices for `LINEAR`/`SPHERICAL`/single-colour, and the explicit
  `Partial` reason codes for curve kinds and unusable definitions.
* Real software-Vulkan (lavapipe) frames in
  `crates/cad-render-wgpu/tests/headless_render.rs` prove a red→blue per-vertex
  ramp rasterizes red-dominant on one end and blue-dominant on the other, and
  that reversing the ramp changes the readback.
* `fill_rings` was found to drop bands that taper to an apex (any non-rectangular
  boundary, e.g. a triangle, failed the area cross-check). This is fixed and
  covered by `fill_rings_handles_a_polygon_with_apex_bands`; without it gradient
  and solid fills of arbitrary boundaries could not be produced.

## 6. Gaps (explicit, not silently approximated)

1. **Curved / inversion gradient kinds** (`HEMISPHERICAL`, `CURVED`,
   `INVSPHERICAL`, `INVCYLINDER`): reported `Partial`, boundary drawn only.
2. **Multi-stop and curved ramps**: multiple stops are supported as a piecewise
   linear ramp, but AutoCAD's "curved" blend between stops is not reproduced;
   interpolation is linear in sRGB.
3. **Radial fidelity on coarse boundaries**: because the ramp is baked per
   triangulation vertex, a `SPHERICAL` gradient on a shape whose boundary
   vertices all share one radius (for example an axis-aligned rectangle) is
   constant. A triangle or curved boundary has varied radii and does ramp. A
   future improvement would subdivide interior triangles by value.
4. **`shift` wrapping**: the offset is clamped at the ends rather than wrapping
   around, so the two boundary endpoints keep their exact stop colours.
5. **Pattern + gradient combination**: a HATCH is either solid, gradient, or a
   line pattern. A hatch that carries both a line pattern and an enabled gradient
   is currently resolved as a gradient (gradient checked before the solid flag);
   combining pattern lines with a gradient background is not implemented.
6. **Tint modulation**: gradient stop colours are multiplied by the entity/batch
   colour, so a non-white entity colour shifts the ramp. Gradient colours are not
   treated as absolute.
7. **No colour management**: interpolation is in sRGB byte space, matching the
   renderer's existing no-colour-management path.
