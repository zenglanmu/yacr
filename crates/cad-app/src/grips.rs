//! Grip model: which draggable handles an entity exposes, hit-testing one
//! handle, and the geometry a **single-handle drag** produces.
//!
//! This is the pure model only. There is deliberately **no** transaction, no
//! history entry, no renderer marker and no host arming here: the `GRIPSTRETCH`
//! **command** lives in `crate::app_drawing::grip` (it re-reads the addressed
//! entity, calls [`apply_grip`] and owns the single transaction / undo entry),
//! the marker is `crate::render_scene::grip_marker_overlay` (fed by the host),
//! and pointer routing / arming lives in `cad-host` (`input/grip_drag.rs`).
//! Everything in this module is a pure function over one entity's geometry (or,
//! for [`grip_hit`]/[`grip_hit_planar`], over a list of entities), so it can be
//! unit-tested without a drawing, a session or a GPU.
//!
//! ## A grip is not a snap point
//!
//! Object snap points (OSNAP, `cad-measure`'s
//! `SnapKind::{Endpoint, Midpoint, Center, …}`) answer "where may a **new**
//! point land?"; they are derived per geometry and only exist while a point is
//! being captured. A grip answers a different question: "which **existing**
//! handle of this entity may be dragged, and what geometry does dragging it
//! produce?". The two surfaces are unrelated and this module neither reuses nor
//! extends `SnapKind`. In particular, `Midpoint` is a snap point and is **not**
//! a grip (see the refusal list below).
//!
//! ## Supported handles (the whole implemented surface)
//!
//! | geometry | handles |
//! |---|---|
//! | `SemanticGeometry::Line` | two [`GripKind::Endpoint`] grips, `index` 0 = `start`, `index` 1 = `end` |
//! | `SemanticGeometry::Point` | one [`GripKind::Point`] grip, `index` 0 |
//!
//! That is all. [`grips_for_entity`] returns an **empty** vector for every
//! other geometry, and [`grip_hit`] therefore never hits one. No marker is
//! invented, no index is guessed and no geometry is silently translated instead:
//! an entity with no grips is simply not grip-editable in this build.
//!
//! ## Explicit refusal list (no marker, no hit)
//!
//! `ARC`, `CIRCLE`, `ELLIPSE`, `SPLINE`, `POLYLINE`/`LWPOLYLINE` (vertex grips
//! would have to decide the bulge semantics of the dragged segment),
//! `INSERT`/block-member geometry (an instance transform is not an entity
//! handle), `TEXT`/`MTEXT`/`MULTILEADER` (insertion-point and width grips),
//! `HATCH`, `DIMENSION` (definition-point grips; the dimension is
//! non-associative by design), `IMAGE`/`MASK`, and opaque/compound geometry
//! (`Opaque`, `Compound`, `Mesh`, `Shape`).
//!
//! Midpoint, centre, quadrant, insertion-point, vertex, width, rotation and
//! tangent grips are **NOT IMPLEMENTED**. They are not modelled as [`GripKind`]
//! variants either: a variant that always yields an empty vector or an error
//! would be a placeholder pretending to be a capability. A later lane that
//! implements one of them adds its variant **and** its drag rule together.
//!
//! ## Coordinates: 3D-exact, never projected
//!
//! A grip's [`Grip::position`] is the stored handle position verbatim — the
//! same `f64` bits, no rounding and no snap. A drag assigns the target point to
//! the dragged handle in **all three** coordinates; nothing is projected onto a
//! work plane or onto Z = 0. Because the supported kinds (`LINE` endpoint,
//! `POINT`) have an exact 3D handle, there is no off-plane input to refuse: the
//! caller resolves the pointer into the world point it means, and that
//! resolution belongs there, not here. The host (`cad-host`'s
//! `input/grip_drag.rs`) resolves the pointer in the plan view — x/y from
//! `screen_to_world`, and the dragged handle's own `z`, so a plan-view drag
//! never changes an elevation — and this module stores whatever finite target it
//! is given. A non-finite target is refused (see below) instead of stored.
//!
//! ## Hit-testing rule
//!
//! [`grip_hit`] returns the **closest** handle whose Euclidean world distance to
//! `world` is `<= tolerance`. Candidates are visited in a fixed order — the
//! entities in the order the caller supplied them, and within one entity its
//! grips in ascending `index` (for a `LINE`: `start` before `end`) — and a
//! candidate replaces the current winner only when it is **strictly** closer.
//!
//! [`grip_hit_planar`] is the **plan-view** variant the host uses: the same
//! ordering, tie-break and finiteness refusals, but the distance is measured in
//! x/y only. A pointer resolves through the viewport's `screen_to_world`, which
//! returns a point on the work plane (`z = work_plane.origin.z`); measuring 3D
//! distance against that point would miss every grip whose own `z` differs, so
//! a line at another elevation would be un-grippable in the very view that shows
//! it. Ignoring `z` is the plan-view rule, **not** 3D grip editing: moving a
//! handle in `z` is NOT IMPLEMENTED and the host keeps the dragged handle's own
//! `z` in the target it commits.
//!
//! **Tie-break rule:** on an exact distance tie the winner is the **first**
//! candidate in that iteration order, i.e. the earliest entity of the input and,
//! within one entity, the lower `index`. This is deterministic and is pinned by
//! a contract test (two handles at the same distance; the earlier one wins).
//! A non-finite `world`, a non-finite or negative `tolerance`, and a stored
//! position at a non-finite coordinate never hit (nothing is fabricated from a
//! `NaN` comparison).
//!
//! ## Refusal reasons (English, stable, machine-matched)
//!
//! [`apply_grip`] refuses with an existing [`CadError`] variant whose message
//! **starts with** one of these codes:
//!
//! * [`REASON_UNSUPPORTED_KIND`] (`CadError::Unsupported`) — the geometry kind
//!   is on the refusal list above and exposes no grip model at all. The message
//!   names the DXF class.
//! * [`REASON_STALE_HANDLE`] (`CadError::InvalidInput`) — the [`GripId`] does
//!   not name a handle of **this** geometry: a `LINE` with a [`GripKind::Point`]
//!   grip, a `POINT` with a [`GripKind::Endpoint`] grip, or an `index` outside
//!   the handle range (`LINE` > 1, `POINT` > 0). The caller is never trusted to
//!   have kept its id in sync with the geometry.
//! * [`REASON_ZERO_DISPLACEMENT`] (`CadError::InvalidInput`) — the target equals
//!   the dragged handle's current position, so there is nothing to write. It is
//!   an error, not a silent `Ok`: a future command must skip the transaction
//!   (and therefore the undo entry). Equality is IEEE `==` on all three
//!   coordinates (so `-0.0` equals `0.0`, and no tolerance is applied — a
//!   sub-tolerance drag is a real, tiny edit).
//! * [`REASON_NON_FINITE_TARGET`] (`CadError::InvalidInput`) — the target has a
//!   `NaN` or infinite coordinate. Storing it would corrupt the entity, and the
//!   database refuses it at commit anyway; refusing here names the cause.
//!
//! The checks run in a fixed order: geometry kind, then handle identity, then
//! target finiteness, then zero displacement. An unsupported kind is therefore
//! reported as [`REASON_UNSUPPORTED_KIND`] even when the id would also be stale
//! (there is no handle of that kind to be stale *against*), while a supported
//! kind with a mismatched id is always [`REASON_STALE_HANDLE`].
//!
//! ## Command-level reason (the one code this module does not raise itself)
//!
//! [`REASON_TARGET_MISSING`] belongs to the same `grip.*` vocabulary but is
//! raised by the command (`crate::app_drawing::grip`), never by this module: a
//! [`GripId`] names an entity, and this module only ever sees one entity's
//! geometry, so it cannot report that the entity is gone. It is declared here so
//! every `grip.*` code has one authority.

use cad_db::DbEntity;
use cad_domain::{CadError, CadResult, EntityId, Point3, SemanticGeometry};

/// Stable reason code: the geometry kind exposes no grip model.
pub const REASON_UNSUPPORTED_KIND: &str = "grip.unsupported_kind";
/// Stable reason code: the [`GripId`] does not name a handle of this geometry.
pub const REASON_STALE_HANDLE: &str = "grip.stale_handle";
/// Stable reason code: the drag target equals the handle's current position.
pub const REASON_ZERO_DISPLACEMENT: &str = "grip.zero_displacement";
/// Stable reason code: the drag target is not a finite world point.
pub const REASON_NON_FINITE_TARGET: &str = "grip.non_finite_target";
/// Stable reason code: the addressed entity does not exist in the drawing.
///
/// Raised by the `GRIPSTRETCH` command (`crate::app_drawing::grip`), which is the
/// only place that can know an entity id stopped resolving; see the module doc.
pub const REASON_TARGET_MISSING: &str = "grip.target_missing";

/// A draggable handle of one entity.
///
/// Only the two kinds the model implements exist; see the module doc for the
/// deliberate absence of midpoint/centre/insertion/vertex kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GripKind {
    /// A straight-line endpoint (`LINE` only).
    Endpoint,
    /// The position of a point entity (`POINT` only).
    Point,
}

/// The identity of one handle: which entity, which kind of handle, which one.
///
/// `index` is the handle's ordinal **within its kind**, in a documented order:
/// for a `LINE`, `0` is `start` and `1` is `end`; for a `POINT`, `0` is the
/// position. The id is a caller-held reference into geometry that may have
/// changed since it was read, so [`apply_grip`] re-validates it against the
/// geometry it is handed instead of trusting it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GripId {
    pub entity: EntityId,
    pub kind: GripKind,
    pub index: u8,
}

/// A handle plus the world position it names.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Grip {
    pub id: GripId,
    /// The stored handle position, bit-identical to the geometry's own value.
    pub position: Point3,
}

/// The handles `entity` exposes in plane space.
///
/// `LINE` ⇒ two [`GripKind::Endpoint`] grips in `start`, `end` order;
/// `POINT` ⇒ one [`GripKind::Point`] grip. Every other geometry — including the
/// whole refusal list in the module doc — yields an empty vector: no marker is
/// invented for a kind this build cannot drag.
#[must_use]
pub fn grips_for_entity(entity: &DbEntity) -> Vec<Grip> {
    let id = |kind, index| GripId {
        entity: entity.id,
        kind,
        index,
    };
    match &entity.geometry {
        SemanticGeometry::Line { start, end } => vec![
            Grip {
                id: id(GripKind::Endpoint, 0),
                position: *start,
            },
            Grip {
                id: id(GripKind::Endpoint, 1),
                position: *end,
            },
        ],
        SemanticGeometry::Point(position) => vec![Grip {
            id: id(GripKind::Point, 0),
            position: *position,
        }],
        _ => Vec::new(),
    }
}

/// Hit-test a world position against the handles of `entities`.
///
/// Returns the closest handle within `tolerance` world units, or `None`. A
/// candidate must satisfy `distance <= tolerance` (so a handle exactly at the
/// tolerance is hit, and `tolerance == 0.0` only hits an exactly coincident
/// handle). Entities that expose no grips are skipped. Ties are broken by the
/// documented iteration order: the **earliest entity of the input** wins, and
/// within one entity the **lower `index`** wins (a candidate must be strictly
/// closer to replace the current winner).
///
/// A non-finite `world`, a non-finite or negative `tolerance`, and a stored
/// handle position at a non-finite coordinate are all non-hits.
#[must_use]
pub fn grip_hit<'a>(
    entities: impl IntoIterator<Item = &'a DbEntity>,
    world: Point3,
    tolerance: f64,
) -> Option<Grip> {
    if !is_finite(world) || !tolerance.is_finite() || tolerance < 0.0 {
        return None;
    }
    let mut best: Option<(f64, Grip)> = None;
    for entity in entities {
        for grip in grips_for_entity(entity) {
            let distance = distance(world, grip.position);
            // Written as an explicit NaN guard so a `NaN` distance (a
            // non-finite stored coordinate) cannot pass the filter.
            if distance.is_nan() || distance > tolerance {
                continue;
            }
            match best {
                // Strictly closer only: the earlier candidate keeps an exact
                // tie, which is the documented tie-break.
                Some((best_distance, _)) if distance >= best_distance => {}
                _ => best = Some((distance, grip)),
            }
        }
    }
    best.map(|(_, grip)| grip)
}

/// Hit-test a **plan-view** world position against the handles of `entities`.
///
/// Identical to [`grip_hit`] — same candidate iteration order, same
/// "strictly closer wins" replacement rule (so an exact tie still goes to the
/// earliest entity of the input and, within one entity, to the lower `index`),
/// same `distance <= tolerance` predicate and same finiteness refusals — with
/// **one** difference: the distance is measured in the x/y plane only, so the
/// `z` coordinates of `world` and of the handle are ignored.
///
/// This is the documented **plan-view** hit rule for a pointer: the host
/// resolves a pointer through the viewport's `screen_to_world`, which always
/// returns a point on the work plane (`z = work_plane.origin.z`). A 3D distance
/// against that point would miss every grip at another elevation (a `LINE` drawn
/// at `z = 5` would be un-grippable in the plan view that displays it), so the
/// plan-view rule deliberately measures what the user is aiming at on screen.
///
/// It is **not** 3D grip editing: dragging a handle in `z` is NOT IMPLEMENTED.
/// The host keeps the dragged handle's own `z` in the target it commits, so a
/// plan-view drag never changes an entity's elevation.
///
/// A non-finite `world`, a non-finite or negative `tolerance`, and a stored
/// handle position at a non-finite coordinate (any of `x`, `y`, `z`) are
/// non-hits. The explicit stored-position check is required here because an
/// ignored `NaN` `z` would otherwise not poison the planar distance.
#[must_use]
pub fn grip_hit_planar<'a>(
    entities: impl IntoIterator<Item = &'a DbEntity>,
    world: Point3,
    tolerance: f64,
) -> Option<Grip> {
    if !is_finite(world) || !tolerance.is_finite() || tolerance < 0.0 {
        return None;
    }
    let mut best: Option<(f64, Grip)> = None;
    for entity in entities {
        for grip in grips_for_entity(entity) {
            if !is_finite(grip.position) {
                continue;
            }
            let distance = planar_distance(world, grip.position);
            if distance.is_nan() || distance > tolerance {
                continue;
            }
            match best {
                // Strictly closer only: an exact tie keeps the earlier candidate,
                // exactly like `grip_hit`.
                Some((best_distance, _)) if distance >= best_distance => {}
                _ => best = Some((distance, grip)),
            }
        }
    }
    best.map(|(_, grip)| grip)
}

/// Apply a single-handle drag to `geometry`, returning the geometry the entity
/// must be updated to.
///
/// * `LINE`: the dragged endpoint takes `to`; the other endpoint is returned
///   **bit-identically** (the very same `f64` values, not a recomputed pair);
/// * `POINT`: the position becomes `to`;
/// * everything else, a [`GripId`] that does not name a handle of this geometry,
///   a non-finite target and a zero displacement are refused with the stable
///   reason codes documented in the module doc (see [`REASON_UNSUPPORTED_KIND`]).
///
/// The function is pure: it writes nothing, opens no transaction and records no
/// history. The `GRIPSTRETCH` command calls it, and on `Err` skips the
/// transaction entirely — that is exactly why a zero displacement is an error
/// instead of an `Ok` with unchanged geometry (no empty undo entry).
pub fn apply_grip(
    geometry: &SemanticGeometry,
    id: &GripId,
    to: Point3,
) -> CadResult<SemanticGeometry> {
    match geometry {
        SemanticGeometry::Line { start, end } => {
            let (current, dragged_start) = match (id.kind, id.index) {
                (GripKind::Endpoint, 0) => (*start, true),
                (GripKind::Endpoint, 1) => (*end, false),
                (GripKind::Endpoint, index) => {
                    return Err(stale_handle(format!(
                        "AcDbLine has no Endpoint grip at index {index} (0 = start, 1 = end)"
                    )));
                }
                (GripKind::Point, _) => {
                    return Err(stale_handle(
                        "AcDbLine exposes Endpoint grips only, not a Point grip".to_string(),
                    ));
                }
            };
            finite_target(to)?;
            if to == current {
                return Err(zero_displacement("AcDbLine endpoint"));
            }
            Ok(if dragged_start {
                SemanticGeometry::Line {
                    start: to,
                    end: *end,
                }
            } else {
                SemanticGeometry::Line {
                    start: *start,
                    end: to,
                }
            })
        }
        SemanticGeometry::Point(position) => {
            match (id.kind, id.index) {
                (GripKind::Point, 0) => {}
                (GripKind::Point, index) => {
                    return Err(stale_handle(format!(
                        "AcDbPoint has no Point grip at index {index} (only 0)"
                    )));
                }
                (GripKind::Endpoint, _) => {
                    return Err(stale_handle(
                        "AcDbPoint exposes a Point grip only, not an Endpoint grip".to_string(),
                    ));
                }
            }
            finite_target(to)?;
            if to == *position {
                return Err(zero_displacement("AcDbPoint position"));
            }
            Ok(SemanticGeometry::Point(to))
        }
        other => Err(unsupported_kind(crate::app_drawing::class_name(other))),
    }
}

/// A `grip.unsupported_kind` refusal naming the DXF class.
fn unsupported_kind(class: &str) -> CadError {
    CadError::Unsupported(format!(
        "{REASON_UNSUPPORTED_KIND}: {class} exposes no grippable handle"
    ))
}

/// A `grip.stale_handle` refusal with the specific mismatch.
fn stale_handle(detail: String) -> CadError {
    CadError::InvalidInput(format!("{REASON_STALE_HANDLE}: {detail}"))
}

/// A `grip.zero_displacement` refusal naming the handle that did not move.
fn zero_displacement(handle: &str) -> CadError {
    CadError::InvalidInput(format!(
        "{REASON_ZERO_DISPLACEMENT}: the {handle} is already at the drag target"
    ))
}

/// Refuse a target that could not be stored as a world point.
///
/// Exposed to the `GRIPSTRETCH` command so it can refuse a non-finite target
/// **before** it even reads the drawing, with the same code and message the drag
/// rule would produce.
pub(crate) fn finite_target(to: Point3) -> CadResult<()> {
    if is_finite(to) {
        Ok(())
    } else {
        Err(CadError::InvalidInput(format!(
            "{REASON_NON_FINITE_TARGET}: ({}, {}, {})",
            to.x, to.y, to.z
        )))
    }
}

fn is_finite(point: Point3) -> bool {
    point.x.is_finite() && point.y.is_finite() && point.z.is_finite()
}

fn distance(a: Point3, b: Point3) -> f64 {
    let dx = a.x - b.x;
    let dy = a.y - b.y;
    let dz = a.z - b.z;
    (dx * dx + dy * dy + dz * dz).sqrt()
}

/// x/y distance, ignoring `z` (the plan-view rule of [`grip_hit_planar`]).
fn planar_distance(a: Point3, b: Point3) -> f64 {
    let dx = a.x - b.x;
    let dy = a.y - b.y;
    (dx * dx + dy * dy).sqrt()
}
