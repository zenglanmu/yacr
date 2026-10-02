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

- The importer still does not emit a usable viewport transform (adjacent clip
  corners, scalar-only transform, missing direction/target/twist); see
  `docs/layouts.md` §3.1. Until then no real imported viewport yields a
  `ViewportModel` inverse. The engine-side contract is complete and tested with
  constructed transforms.
- `MeasurementEngine::snap` (the `SelectionRef`-based contract entry) still
  returns `Unsupported` because candidate geometry must be resolved from the
  database + spatial index; `snap_to_points` is the real entry.
