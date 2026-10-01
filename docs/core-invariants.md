# Core invariants (B23–B31 focus)

This document records which audit items the core crates
(`cad-domain`, `cad-geometry`, `cad-db`) close with pure value-level
invariants and unit tests, and which remain open because they need crates,
real DWG samples, devices or hosts that are outside this change.

It is a companion to `docs/code-audit-and-agent-handoff.md`; it does not
replace it. Every "closed" row names the test that would fail before the fix
and passes after.

Scope of this change: `crates/cad-domain/**`, `crates/cad-geometry/**`,
`crates/cad-db/**` and their `tests/**`. `cad-import-acadrust`,
`cad-representation`, `cad-app`, `cad-ui-slint`, `cad-measure`, `cad-scene`,
`cad-query`, `cad-history`, `cad-cli-tools` were **not** modified, so any item
whose fix must land in those crates stays open here.

## Closed items

| Item | What is locked | Test | Code |
|---|---|---|---|
| B23 (circle/arc under non-uniform transform) | A circle mapped by a non-uniform scale becomes an exact `Ellipse` (major axis + ratio), and a uniform similarity still yields a `Circle` with a scaled radius. | `non_uniform_transform_turns_circle_into_ellipse_not_circle`, `transformed_circle_points_lie_on_the_ellipse`, `uniform_transform_keeps_circle_and_scales_radius` | `DefaultGeometryEngine::transform` (`circle_to_ellipse`), `Transform3::is_uniform_scale` |
| B23 (bulge continuity) | A bulge arc preceded by a straight segment stays connected; a fully-bulged closed polyline closes on its first vertex and has no zero-length segment. | `bulge_arc_after_line_segment_is_continuous`, `closed_polyline_all_bulges_are_continuous` | `polyline_with_bulges`, `append_bulge_arc` |
| B23 (ellipse plane / ratio) | The minor axis is the documented `cross(world_z, major)` direction; a tilted ellipse stays in its own plane; ratio is recomputed under transform instead of being copied blindly. | `tilted_ellipse_stays_in_its_own_plane_when_tessellated`, `transformed_circle_points_lie_on_the_ellipse` | `ellipse_minor_dir`, `transform` Ellipse arm |
| B23 (spline knots/weights) | Source knots and weights are honoured; a quadratic Bézier with explicit clamped knots bows to its true apex; a rational weight changes the shape; a malformed knot vector falls back without panicking or fabricating geometry. | `spline_honours_source_knots_instead_of_uniformising`, `rational_spline_weighs_control_points`, `malformed_knots_fall_back_without_panicking` | `tessellate_spline`, `de_boor_rational` |
| B23 (display LOD vs semantics) | Curve intersection results do not depend on `TolerancePolicy::display_pixels`; a change of display budget cannot change measured/snapped geometry. | `intersection_is_independent_of_display_pixel_budget` | `GeometryEngine::tessellate_curve`, `intersect_local` |
| B23 (non-finite transform output) | A transform that maps a finite source to a non-finite point is reported as `InvalidInput`, never returned as poisoned geometry. | `transform_rejects_non_finite_result` | `geometry_is_finite`, `transform` |
| B23 (transform predicates) | `is_uniform_scale` accepts identity, uniform scale, rotation and mirror and rejects non-uniform scale, shear, singular and non-finite matrices; `determinant` reports sign/magnitude; composition matches sequential application; an inverse round-trips. | `cad-domain/tests/invariants.rs` (9 tests) | `Transform3::is_uniform_scale`, `max_scale`, `determinant`, `matrix_mul` |
| B24 (geometry-layer work plane / ray) | A skewed or non-orthogonal work plane is rejected; a degenerate or non-finite plane is rejected; a ray pointing away from the plane never hits even when unnormalised; the hit point is independent of the ray direction's magnitude. | `skewed_work_plane_is_rejected`, `degenerate_and_non_finite_work_planes_are_rejected`, `orthogonal_work_plane_is_accepted`, `ray_behind_the_origin_never_hits`, `ray_plane_result_is_independent_of_direction_magnitude` | `validate_work_plane`, `distance_to_plane`, `GeometryEngine::ray_plane` |
| B24 (area finiteness) | `measure_polygon_area` refuses non-finite coordinates and a non-orthogonal work plane instead of returning a plausible but meaningless value. | `area_rejects_non_finite_points_and_non_orthogonal_measurement_plane` | `area::measure_polygon_area` |
| B12 (annotation validation + atomicity) | A single invalid change (NaN geometry, key/id mismatch, zero-axis ellipse, negative style, empty leader, `modified < created`, non-finite/empty anchor, skewed measurement plane) rejects the whole transaction; no partial insert and **no revision bump**. | `invalid_annotation_rejects_whole_transaction_atomically`, `change_key_must_match_annotation_id`, `zero_axis_ellipse_annotation_is_rejected`, `negative_or_non_finite_style_is_rejected`, `modified_before_created_is_rejected`, `empty_leader_geometry_is_rejected`, `measurement_with_skewed_plane_is_rejected`, `non_finite_anchor_fallback_is_rejected`, `empty_anchor_handle_is_rejected` | `AnnotationDatabase::apply_annotation_changes`, `validate_annotation` |
| B12 (precise change mask) | A metadata/timestamp edit reports `ChangeMask::METADATA` and does not invalidate representations; a text/style edit reports `STYLE`, a geometry edit `GEOMETRY`, an anchor move `TRANSFORM`. | `metadata_only_update_reports_metadata_mask`, `text_payload_change_reports_style_mask_not_geometry`, `geometry_change_reports_geometry_mask`, `anchor_move_reports_transform_mask`, `committed_metadata_only_update_is_reported_as_metadata` | `ChangeMask::for_annotation_update` |
| B12 (database invariants) | Commits are ordered, `ChangeSet::follows` holds, revision is monotonic, an empty transaction does not advance revision, a dangling block/layout reference is refused by the builder, and a future export revision is refused while dirty state persists. | `commit_is_ordered_and_revision_is_monotonic`, `empty_transaction_does_not_advance_revision_or_claim_changes`, `builder_accepts_nested_blocks_and_rejects_dangling_block_entity`, `layout_entities_require_an_existing_layout`, `marking_a_future_revision_is_refused_and_dirty_stays`, `later_edit_after_mark_is_dirty_again` | `AnnotationDatabase`, `DrawingDatabaseBuilder::finish`, `ChangeSet::follows` |

## Still open

| Item | Why it stays open |
|---|---|
| B23 (non-Z-plane circle under non-uniform transform, exactness flag) | The domain `Ellipse` has no extrusion normal, so a circle whose plane is not parallel to world Z, scaled non-uniformly, cannot be encoded exactly. It is currently approximated; a truthful `Partial`/`Unsupported` flag would need a representation/completeness channel or an `extrusion` field added across `cad-domain`, `cad-import-acadrust` and the annotation codec — out of this change's scope. |
| B23 (importer supplying source knots/OCS) | The importer must read the real DWG ellipse extrusion and spline knots/weights from acadrust; `cad-import-acadrust` is out of scope and needs real DWG samples to verify. |
| B24 (snap rejecting points behind the camera, measure space constraints) | The snap ray-parameter and paper/world space rules live in `cad-measure`, which is out of scope. The geometry-layer predicates (this change) are a prerequisite, not the fix. |
| B24 (area work-plane projection in `cad-measure`) | `PlanarPolygonArea` flattens points to z=0 in `cad-measure`; must call `validate_work_plane`/coplanarity there. Out of scope. |
| B12 (Builder robustness: text style existence, grid index, non-finite numeric fields) | `DrawingDatabaseBuilder` validates layer/layout/block references but not `Text.style` existence or per-geometry numeric finiteness. A focused follow-up can reuse `geometry_is_finite`-style checks inside the builder. |
| B28 (history stacks, budget, transaction id reuse) | The undo/redo stack, memory-budget accounting and shared transaction counters live in `cad-history`; out of scope. This change only locks the database-level atomicity/revision invariants that B28 depends on. |
| B25 (camera projection / standard views) | `cad-app`; out of scope. |
| B26 (query document/database/revision tracking) | `cad-query`; out of scope. |
| B27 (scene/GPU incremental + budget) | `cad-scene`, `cad-render-wgpu`; out of scope. |
| B29 (web polling/smoke test) | `apps/app-web`, `scripts/check-web-ui.mjs`; out of scope. |
| B30 (CLI atomic export/diagnostics) | `cad-cli-tools`; out of scope. |
| B31 (importer/proxy limits, source units) | `cad-import-acadrust`, `cad-proxy`; out of scope and needs real DWG samples. |

## Verification

See the commit message and the parent session report for the exact commands.
The invariant tests are:

- `crates/cad-domain/tests/invariants.rs`
- `crates/cad-geometry/tests/invariants.rs`
- `crates/cad-db/tests/invariants.rs`
