# Measurement & snapping (`cad-measure`)

Spec v2.0 §3.3: a measurement result carries its input points, unit context,
algorithm and precision — never just a formatted string. This note records the
contracts closed under audit **B24** and what remains open.

## Spaces and algorithms

`MeasurementSpace` selects the space a measurement is evaluated in:

| Variant | Meaning | Planar algorithms allowed? |
|---|---|---|
| `Plane(WorkPlane)` | Explicit 2D work plane (u/v/out-of-plane in world units) | yes |
| `World3d` | 3D model space | no |
| `Paper(LayoutId)` | Paper space of a layout | no (unsupported until wired) |
| `ViewportModel { layout, inverse }` | Paper viewport showing model space | no (unsupported until wired) |

`Distance2d`, `PolylineLength` and `PlanarPolygonArea` require an explicit work
plane (`MeasurementSpace::is_planar`). Passing `World3d` to one of them is an
`InvalidInput` error, not a silent reinterpretation. `Distance3d` and
`Angle3Points` are spatial and are refused in paper/viewport space.

## Provenance and mixed spaces

`MeasurementRequest::tapped: Vec<MeasurementPoint>` carries each point's origin
`SpaceId`. When non-empty it takes precedence over the raw `points` vector:

- all tapped points must share one `SpaceId`, otherwise the measurement is
  refused (`"measurement mixes points from different spaces"`);
- a model-space measurement (`Plane`/`World3d`) may not consume paper-space
  points.

`MeasurementPoint::model` / `MeasurementPoint::paper` build the tagged form;
`MeasurementRequest::from_tapped` fills `points` from the tapped coordinates.

## Work-plane-aware area

`project_to_plane` validates the plane before projecting:

1. plane origin/u/v must be finite (`"measurement plane is not finite"`);
2. u and v must be non-degenerate (`"measurement plane is degenerate"`);
3. u and v must be orthogonal to within `1e-6` of unit scale
   (`"measurement plane is skewed (u and v are not orthogonal)"`);
4. when the basis is unit-scaled, every ring point must lie within
   `topology_world` of the plane (`"area ring is not coplanar with the
   measurement plane"`);
5. the projected coordinates must be finite
   (`"area projection is not finite"`).

The projected ring is then handed to `cad_geometry::measure_polygon_area`,
which rejects self-intersections, remaining non-coplanarity and degenerate
zero-area rings. Extreme finite coordinates that overflow the shoelace sum are
reported as an error rather than returning `inf`.

A skewed plane is refused even though the area business rule would tolerate one:
skew leaks a scale factor of `|u||v|sinθ` into the projected value.

## Pick-ray snapping

`MeasurementEngine::snap_to_points(candidates, ray, world_per_px, space_filter)`
enforces a **pick-ray** contract:

- `ray.origin` must be finite and `ray.direction` finite and non-zero; the
  direction is normalised internally;
- a candidate whose ray parameter `t = dot(p - origin, dir)` is negative is
  behind the origin and is skipped (`point_ray_distance` returns `None`);
- `world_per_px` must be finite and positive;
- when `space_filter` is supplied, only candidates in that space are considered.

The returned `logical_pixel_distance` is the perpendicular distance divided by
`world_per_px`; the kind and space are carried through.

## Open items

- `Paper` and `ViewportModel` measurement require a **verified inverse viewport
  transform**. The engine currently refuses all such measurements with
  `Unsupported`; the field exists so the wiring is a single, localised change in
  `check_space_policy`. This is intentional (spec §3.3 forbids guessing a model
  distance) and is tracked under F04.
- `MeasurementEngine::snap` (the `SelectionRef`-based contract entry) still
  returns `Unsupported` because candidate geometry must be resolved from the
  database + spatial index; `snap_to_points` is the real entry.
