//! `BOUNDARY` (AutoCAD 2026) and the shared internal-point boundary discovery it
//! uses with `HATCHPOINT`.
//!
//! `BOUNDARY` (`BO`/`BPOLY` in AutoCAD) creates closed polylines or regions from
//! the objects surrounding a picked internal point. This build implements the
//! **polyline-only, non-associative** v1 described here; the alias spellings are
//! deliberately **not** admitted (`COMMAND_ALIASES` has no `BO`/`BPOLY` row), so
//! only the canonical `BOUNDARY` selector is typed.
//!
//! # Input
//!
//! Payload is [`crate::CommandPayload::BoundaryPoint`] `{ seed }`: the one
//! captured internal point. The **current selection is not the input** (the
//! boundary is discovered from the point, exactly like `HATCHPOINT`), and the
//! capture takes no entity pick and no typed text.
//!
//! # Discovery (shared with `HATCHPOINT`)
//!
//! [`crate::app_drawing::hatch_internal_point`] owns the one discovery pipeline:
//! the **smallest-area visible closed boundary** containing the seed is the outer
//! loop and every loop nested inside it (all depths) is included. `BOUNDARY`
//! consumes only the **whole-entity** half of that pipeline — the same source
//! set its sibling documents — and never duplicates the geometry code.
//!
//! Declared source-entity limits (the code does not claim more):
//!
//! * only **whole visible** `Circle`, full `Ellipse` and closed `Polyline`
//!   entities participate; `LINE`/`ARC` edge chains, `SPLINE`/ellipse-arc edges
//!   and block-instance edges are **not** auto-discovered;
//! * only boundaries in the seed's `+Z` plane (same elevation) are considered;
//!   other planes, hidden layers and unresolved refs are skipped, never
//!   projected or invented;
//! * the seed must be clearly inside or outside: on-edge, no-boundary and
//!   equal-area-ambiguous picks are explicit refusals, never guesses;
//! * self-intersecting input is whatever the shared loop builder already
//!   accepted; `BOUNDARY` adds no repair pass.
//!
//! # Output
//!
//! Every discovered loop becomes one closed [`cad_domain::SemanticGeometry::Polyline`]
//! (`type_key`/`class_name` = `AcDbPolyline`):
//!
//! * the loop's hatch-plane 2D points and bulges are mapped to world `+Z`
//!   coordinates through the same plane frame the fill uses, preserving every
//!   bulge;
//! * a repeated last point is dropped (with its degenerate closing bulge), because
//!   a closed polyline already implies the closing segment;
//! * straight rings get one zero bulge per vertex (the storage convention the
//!   importer and `PEDIT` use).
//!
//! All created polylines go on the **active layer**, in model space, through the
//! shared create funnel in **one transaction / one undo step**. The source
//! boundary entities are retained: there is **no associativity** — editing them
//! afterwards never updates the created polylines.
//!
//! # HATCHPOINT's fill-specific limit
//!
//! `HATCHPOINT` fills the same discovered loops through `HatchGeometry::new`,
//! whose existing **chord/bulge validation blind spot** still applies to bulged
//! loops (that creation-path validator measures the loop's chords). `BOUNDARY`
//! writes the loops as polylines instead and never runs the hatch fill validator;
//! its only loop conversion rule is the shared `loop_world_ring`, which preserves
//! every bulge exactly.
//!
//! # Refusals
//!
//! Every refusal below happens before any write and opens no transaction:
//!
//! * `boundary.payload` — the payload is not `BoundaryPoint { seed }`;
//! * `boundary.not_planar` — the seed is non-finite, or the active space is not
//!   model space (v1 is a model-space `+Z` construction);
//! * `boundary.boundary_on_edge` — the seed lies on a candidate boundary;
//! * `boundary.boundary_not_found` — no visible closed boundary contains the seed;
//! * `boundary.boundary_ambiguous` — two containing boundaries have the same area;
//! * `boundary.not_addressable` — the active layer does not exist or a discovered
//!   loop cannot be converted to a valid closed polyline.
//!
//! # Not implemented
//!
//! The `-BOUNDARY` command-line options — a named **boundary set**, **island
//! detection off**, **region** output and the **object type** choice — are **NOT
//! IMPLEMENTED**. This build always discovers the boundary set from the whole
//! drawing, always includes islands, always writes polylines, and has no region
//! model. None of them has a payload field, so a caller cannot smuggle one in.
//! The surface is named in the catalog hints and in
//! `docs/autocad-2026-command-coverage.md`.
//!
//! Work-only: Viewer mode refuses the command at the command layer.
