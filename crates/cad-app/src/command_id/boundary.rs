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
//! Payload is [`crate::CommandPayload::BoundaryPoint`] — a [`BoundaryInput`] with
//! the one captured internal point and the `-BOUNDARY` [`BoundaryOptions`]. The
//! **current selection is not the input** (the boundary is discovered from the
//! point, exactly like `HATCHPOINT`), and the capture takes no entity pick and no
//! typed text. The interactive capture sends the documented default option set
//! (whole drawing, island detection on), so it behaves exactly like the pre-option
//! v1.
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
//! * `boundary.payload` — the payload is not a `BoundaryPoint(BoundaryInput)`;
//! * `boundary.not_planar` — the seed is non-finite, or the active space is not
//!   model space (v1 is a model-space `+Z` construction);
//! * `boundary.boundary_on_edge` — the seed lies on a candidate boundary;
//! * `boundary.boundary_not_found` — no visible closed boundary contains the seed;
//! * `boundary.boundary_ambiguous` — two containing boundaries have the same area;
//! * `boundary.not_addressable` — the active layer does not exist or a discovered
//!   loop cannot be converted to a valid closed polyline;
//! * `boundary.set_empty` — the `-BOUNDARY` boundary set is `Some` but empty;
//! * `boundary.set_not_candidate` — a boundary-set ref is not a visible, directly
//!   addressable, model-space whole closed candidate in the seed plane.
//!
//! # `-BOUNDARY` options
//!
//! The two options this build models travel on [`BoundaryOptions`] (the interactive
//! form prompts nothing: the payload is the whole input, exactly like every other
//! option-carrying command):
//!
//! * **Boundary set** — `options.boundary_set = Some(refs)` searches exactly those
//!   entities instead of every visible closed entity. A ref that is not a visible,
//!   directly addressable model-space closed candidate in the seed's `+Z` plane is
//!   the named `boundary.set_not_candidate` refusal (nothing is written), and an
//!   empty set is `boundary.set_empty` — the set is never widened back to the whole
//!   drawing. Duplicate refs collapse (a set has one entry per entity).
//! * **Island detection** — `options.island_detection = false` creates only the
//!   outer loop and names every skipped nested loop in
//!   `boundary.island_detection_off`. Detection on (the default) emits
//!   `boundary.islands`.
//!
//! # Hole representation (declared)
//!
//! A closed [`cad_domain::SemanticGeometry::Polyline`] carries **no** hole list, so
//! islands are written as **separate closed polylines** — the outer loop first,
//! then the nested loops — and the relation between them exists only in the
//! `boundary.islands` diagnostic. It is never silently dropped: a caller that needs
//! holes must see the diagnostic, and island detection off names the omitted loops.
//!
//! # Not modelled (build-wide)
//!
//! **Region** output and the **object type** choice (polyline vs region) have no
//! model in this build: `cad-domain` has no region object, so `-BOUNDARY`'s
//! `Region` answer and the `Object` option are outside the support domain and have
//! no payload field — a caller cannot request them at all. Everything else in the
//! row is implemented.
//!
//! Work-only: Viewer mode refuses the command at the command layer.

use cad_domain::SelectionRef;

/// The `-BOUNDARY` option set: AutoCAD's `Boundary set` and `Island detection`.
///
/// The derived default is AutoCAD's own: the whole drawing is the boundary set and
/// island detection is **on**, so an empty payload option set behaves exactly like
/// the pre-option v1. Session, database and history state are all untouched.
#[derive(Debug, Clone, PartialEq)]
pub struct BoundaryOptions {
    /// `Boundary set`: `None` (the default) considers every visible closed entity;
    /// `Some(refs)` considers exactly those entities and nothing else.
    pub boundary_set: Option<Vec<SelectionRef>>,
    /// `Island detection`: `true` (the default) also creates the nested loops.
    ///
    /// `false` creates only the outer loop and names the skipped nested loops in
    /// `boundary.island_detection_off` — a closed polyline cannot carry holes, so
    /// the choice is between separate loops and no loops, never an implicit drop.
    pub island_detection: bool,
}

impl BoundaryOptions {
    /// The options with island detection **on**, as AutoCAD's initial value is.
    ///
    /// `#[derive(Default)]` would give `island_detection: false`, which is not the
    /// documented default; this constructor is the one every `Default` path uses.
    pub fn new() -> Self {
        Self {
            boundary_set: None,
            island_detection: true,
        }
    }
}

impl Default for BoundaryOptions {
    fn default() -> Self {
        Self::new()
    }
}

/// The whole `BOUNDARY` input: the picked internal point plus the option set.
///
/// One type so the payload stays a single variant and the option defaults have one
/// home (`BoundaryOptions::default`).
#[derive(Debug, Clone, PartialEq)]
pub struct BoundaryInput {
    /// The picked internal point (finite, `+Z` model plane).
    pub seed: cad_domain::Point3,
    /// The `-BOUNDARY` options.
    pub options: BoundaryOptions,
}
