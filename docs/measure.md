# Measurement & snapping (`cad-measure`)

Spec v2.0 §3.3: a measurement result carries its input points, unit context,
algorithm and precision — never just a formatted string. This note records the
contracts closed under audit **B24** and the paper/viewport space work under
**F04/F06**, plus what remains open.

## Spaces and algorithms

`MeasurementSpace` selects the space a measurement is evaluated in:

| Variant | Meaning | Planar algorithms allowed? |
|---|---|---|
| `Plane(WorkPlane)` | Explicit 2D work plane (u/v/out-of-plane in world units) | yes |
| `World3d` | 3D model space | no |
| `Paper(LayoutId)` | Paper space of a layout (the sheet itself) | yes |
| `ViewportModel { layout, inverse }` | Paper viewport showing model space; `inverse` maps paper → model | yes |

`Distance2d`, `PolylineLength` and `PlanarPolygonArea` require a space with a
measurement plane (`MeasurementSpace::is_planar`). Passing `World3d` to one of
them is an `InvalidInput` error, not a silent reinterpretation. `Distance3d` and
`Angle3Points` are spatial: `Distance3d` is refused on the 2D paper sheet;
`Angle3Points` is refused on the sheet but allowed in a viewport (measured on
the model plane the viewport maps onto).

### Paper vs viewport-model distance

The engine first brings the picked points into the *measurement space*:

- `Paper`: the picks **are** paper coordinates; distance/length/area are measured
  directly on the sheet, in paper units. `Distance3d` is not defined here.
- `ViewportModel`: the picks are paper coordinates; the viewport's verified
  `paper → model` inverse maps them into model space before any distance is
  computed, so the result is a genuine **model** distance, not a paper-pixel
  distance. The recorded `inputs` are the transformed model points and `plane`
  is the model plane the sheet maps onto.

A 1:100 viewport therefore turns a 1-paper-unit pick span into 100 model units;
the two records differ in `value`, in `inputs` and in `plane`, so a caller can
tell them apart.

### When the inverse is missing

`ViewportModel` requires a **valid inverse** transformer: finite and invertible
(determinant magnitude above the scale-relative threshold). Without one the
engine returns `Unsupported` with an explicit reason — model measurement is
disabled, never guessed from paper pixels (spec §3.3, F04). `cad-app`'s
`viewport_measurement_space(db, layout, viewport_index)` builds the space from
`cad-representation`'s `ViewportTransform::paper_to_model` and returns
`Unsupported` with the representation layer's stable viewport reason when the
viewport cannot be represented.

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
4. every ring point must lie within `topology_world` of the plane, measured along
   the **normalised** normal, so the test is unit-correct whatever the basis
   scale (a viewport's model plane is scaled by the drawing ratio)
   (`"area ring is not coplanar with the measurement plane"`);
5. the projected coordinates (onto the orthonormal basis, so the area is in
   world/model units) must be finite (`"area projection is not finite"`).

The projected ring is then handed to `cad_geometry::measure_polygon_area`,
which rejects self-intersections, remaining non-coplanarity and degenerate
zero-area rings. Extreme finite coordinates that overflow the shoelace sum are
reported as an error rather than returning `inf`.

A skewed plane is refused even though the area business rule would tolerate one:
skew leaks a scale factor of `|u||v|sinθ` into the projected value.

## Units

The record carries the request's `UnitContext`, including `source` and `display`.
Unknown units display **"drawing units"** (`UnitContext::label`) and are never
assumed to be millimetres; a known display unit keeps its source unit on the
record (`UnitContext { source: Millimeter, display: Inch, .. }` reports `"in"`
and still records `Millimeter` as the source).

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

- Paper/`ViewportModel` measurement are implemented, but the importer still does
  not emit a usable viewport transform (adjacent clip corners, scalar-only
  transform, missing direction/target/twist); see `docs/layouts.md` §3.1. Until
  then no real imported viewport yields a `ViewportModel` inverse. The
  engine-side contract is complete and tested with constructed transforms.
- `MeasurementEngine::snap` (the `SelectionRef`-based contract entry) still
  returns `Unsupported` because candidate geometry must be resolved from the
  database + spatial index; `snap_targets`/`snap_best`/`measure_snapped` are the
  real entry points.
- `MeasurementRecord` lives in `cad-db` and has no snap field; provenance is
  returned beside the record as `SnappedMeasurement` rather than mutating the
  database type.
- Snaps are analytic but not yet threaded through the app command path
  (`evaluate_measurement` still takes raw points); the engine and provenance are
  ready for that wiring.
