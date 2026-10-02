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

`cad_geometry::GradientDef` (in `crates/cad-geometry/src/hatch.rs`):

| field | meaning | DXF source |
| --- | --- | --- |
| `kind` | `Linear` or `Spherical` | group 450 name |
| `angle` | ramp direction (radians) | group 452 |
| `shift` | offset along the ramp, clamped at the ends | group 453 |
| `single_color` | DXF single-colour flag | group 451 |
| `tint` | single-colour tint in `-1..=1` | group 460 |
| `stops` | `{ value: 0..=1, rgb }` entries | groups 463 / 421 |

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
