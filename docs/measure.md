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

## Object snapping

`crates/cad-measure/src/snap.rs` implements the object-snap engine (spec §3.3,
F06). The database + spatial index pick the **local** target set near the cursor
and pass it as `SnapTarget`s; the engine never scans the drawing.

Entry points on `MeasurementEngine`:

- `snap_targets(targets, ray, plane, world_per_px, space_filter)` — all
  candidates, nearest first;
- `snap_best(...)` — the single nearest candidate;
- `measure_snapped(request, snaps)` — a `MeasurementRecord` plus per-input
  `SnapProvenance` (kind, source entity, sub-element, precision).

Snap kinds: `Endpoint`, `Midpoint`, `Center` (circle/arc/ellipse/bulge arc),
`Quadrant`, `Perpendicular` (foot on a segment or arc, never past an end) and
`LocalIntersection` (pairs within the supplied local set; the quadratic pass is
skipped above `MAX_INTERSECTION_TARGETS = 64`).

Contracts:

- `plane = Some(w)` maps the pick ray to the cursor point `ray ∩ w`; the
  tolerance is `interaction_logical_pixels × world_per_px` in world units, so a
  change of zoom or DPI scales the aperture exactly. `plane = None` falls back
  to the perpendicular distance to the ray.
- The ray origin is a **pick ray**: a candidate with `t = dot(p − origin, dir)
  < 0` is behind the origin and is never returned.
- Non-finite ray/`world_per_px`/tolerance and non-positive
  `interaction_logical_pixels` are rejected (`InvalidInput`); a poisoned target
  contributes no candidate.
- Every candidate carries its `SnapKind`, world point, `SpaceId`,
  `SelectionRef` (with a `SubElementId::source_key` such as `edge:3`,
  `quadrant:2`), `Precision` and logical-pixel distance. Intersection
  candidates also carry the second entity in `secondary`.
- Curves are resolved analytically (bulge arcs from the bulge, ellipses from
  their axes, splines from their source knots); the display LOD never enters a
  snap, so changing `display_pixels` cannot move a snap or a measurement.

`MeasurementEngine::snap` (the old `SelectionRef`-based contract entry) still
returns `Unsupported`: candidate geometry must be resolved from the database,
so `snap_targets` is the real entry.

## Open items

- `Paper` and `ViewportModel` measurement require a **verified inverse viewport
  transform**. The engine currently refuses all such measurements with
  `Unsupported`; the field exists so the wiring is a single, localised change in
  `check_space_policy`. This is intentional (spec §3.3 forbids guessing a model
  distance) and is tracked under F04.
- `MeasurementRecord` lives in `cad-db` and has no snap field; provenance is
  returned beside the record as `SnappedMeasurement` rather than mutating the
  database type.
- Snaps are analytic and do not yet thread through the app command path
  (`evaluate_measurement` still takes raw points); the engine and provenance are
  ready for that wiring.
