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
//! extends `SnapKind`: `SnapKind::Midpoint` is a snap point and
//! [`GripKind::Midpoint`] is a handle, and they are two separate values on two
//! separate surfaces. Nothing here reads, wraps or re-derives an OSNAP point.
//!
//! ## Supported handles (the whole implemented surface)
//!
//! | geometry | handles, in list order |
//! |---|---|
//! | `SemanticGeometry::Line` | [`GripKind::Endpoint`] `0` = `start`, `1` = `end`, then [`GripKind::Midpoint`] `2` |
//! | `SemanticGeometry::Circle` | [`GripKind::Center`] `0`, then [`GripKind::Quadrant`] `0..=3` |
//! | `SemanticGeometry::Arc` | [`GripKind::Center`] `0` **only** |
//! | `SemanticGeometry::Point` | [`GripKind::Point`] `0` |
//!
//! [`grips_for_entity`] returns an **empty** vector for every other geometry, and
//! [`grip_hit`] therefore never hits one. No marker is invented, no index is
//! guessed and no geometry is silently translated instead: an entity with no
//! grips is simply not grip-editable in this build.
//!
//! ### Drag rules
//!
//! * **`LINE` endpoint** (`index` 0/1): the dragged endpoint takes the target and
//!   the other endpoint is returned **bit-identically**.
//! * **`LINE` midpoint** (`index` 2): the whole line **translates**. With
//!   `delta = to - midpoint(start, end)` both endpoints move by that same vector,
//!   so the line stays parallel to itself and its length is unchanged; nothing
//!   is rotated, scaled or snapped. The *reported* handle position is the exact
//!   f64 midpoint `(start + end) * 0.5` of the two stored endpoints (no rounding
//!   to a grid).
//! * **`CIRCLE` centre** (`index` 0): the centre becomes the target; `radius` and
//!   `normal` are returned bit-identically.
//! * **`CIRCLE` quadrant** (`index` `0..=3`, documented order: `0` = `+ax`,
//!   `1` = `+ay`, `2` = `-ax`, `3` = `-ay` of the circle's **own** plane frame):
//!   the **radius** becomes the distance from the centre to the target
//!   **projected into the circle's plane**, while the centre and the normal stay
//!   put (both bit-identically). It is therefore never a 3D distance: a target
//!   `h` off the plane gives the in-plane distance, not `sqrt(in_plane^2 + h^2)`,
//!   so a drag can never tilt a circle into an ellipse. A drag that would give a
//!   zero or non-finite radius is refused by name
//!   ([`REASON_DEGENERATE_RADIUS`]); it is **never clamped** to a minimum.
//! * **`ARC` centre** (`index` 0): the arc moves **rigidly** — `center` takes the
//!   target and `radius` / `normal` / `start` / `sweep` are bit-identical, so the
//!   radius and the angular extent are untouched.
//!   **Arc endpoint grips are NOT IMPLEMENTED** in this build: an `AcDbArc`
//!   exposes exactly one centre grip, and an `Endpoint` / `Quadrant` / `Midpoint`
//!   id against it is a [`REASON_STALE_HANDLE`] refusal rather than a silently
//!   reinterpreted centre drag.
//! * **`POINT`**: the position becomes the target.
//!
//! ## Explicit refusal list (no marker, no hit)
//!
//! `ELLIPSE`, `SPLINE`, `POLYLINE`/`LWPOLYLINE` (vertex grips would have to
//! decide the bulge semantics of the dragged segment), `INSERT`/block-member
//! geometry (an instance transform is not an entity handle),
//! `TEXT`/`MTEXT`/`MULTILEADER` (insertion-point and width grips), `HATCH`,
//! `DIMENSION` (definition-point grips; the dimension is non-associative by
//! design), `IMAGE`/`MASK`, and opaque/compound geometry (`Opaque`, `Compound`,
//! `Mesh`, `Shape`). **Arc endpoint grips** are on this list too, as are
//! **ellipse quadrant/axis** handles.
//!
//! Insertion-point, vertex, width, rotation and tangent grips are **NOT
//! IMPLEMENTED**. They are not modelled as [`GripKind`] variants either: a
//! variant that always yields an empty vector or an error would be a placeholder
//! pretending to be a capability. A later lane that implements one of them adds
//! its variant **and** its drag rule together.
//!
//! ## Coordinates: 3D-exact, never projected (except the circle radius)
//!
//! A handle's [`Grip::position`] is a stored or stored-derived point — the same
//! `f64` bits as the geometry (an endpoint, a point position, a circle/arc
//! centre) or the documented exact derivation of it (a line midpoint). Nothing
//! is rounded and nothing is snapped.
//!
//! A drag assigns the target point to the dragged handle in **all three**
//! coordinates; nothing is projected onto a work plane or onto Z = 0. The caller
//! resolves the pointer into the world point it means — the host (`cad-host`'s
//! `input/grip_drag.rs`) resolves it in the plan view (x/y from
//! `screen_to_world`, plus the dragged handle's own `z`, so a plan-view drag
//! never changes an elevation) — and this module stores whatever finite target it
//! is given. A non-finite target is refused (see below) instead of stored.
//!
//! The **one** projection this module performs is the circle-quadrant radius: the
//! distance is measured in the circle's own plane, precisely so that a
//! non-planar drag cannot deform a circle. That projection applies to the *drag
//! target of a quadrant handle* only; a centre drag, an arc drag, a line drag and
//! a point drag are stored 3D-exact, and no handle position is ever projected
//! back onto a plane.
//!
//! ## The plane frame is shared, not invented
//!
//! Quadrant handle positions and the in-plane radius use
//! `cad_geometry::arbitrary_axis` — the AutoCAD arbitrary-axis-algorithm frame
//! the tessellator, representation provider, intersect/transform code and the
//! importer already measure circle/arc angles in. It returns `(ax, ay, unit
//! normal)`, with `ax`/`ay` orthonormal and inside the circle plane, so
//! `dot(to - center, ax)` and `dot(to - center, ay)` are the exact in-plane
//! coordinates of a displacement. There is no second frame in this module.
//!
//! ## Hit-testing rule
//!
//! [`grip_hit`] returns the **closest** handle whose Euclidean world distance to
//! `world` is `<= tolerance`. Candidates are visited in a fixed order — the
//! entities in the order the caller supplied them, and within one entity its
//! grips in list order (see the table: for a `LINE` `start`, `end`, midpoint; for
//! a `CIRCLE` the centre and then quadrants 0..=3) — and a candidate replaces the
//! current winner only when it is **strictly** closer.
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
//! within one entity, the earlier entry of the list — which is the lower `index`
//! inside one kind, and the earlier kind in the table's order (`Endpoint` before
//! `Midpoint` for a `LINE`, `Center` before every `Quadrant` for a `CIRCLE`).
//! This is deterministic and is pinned by contract tests. Note that the line
//! midpoint is usually *nearer* than the endpoints, so a query at the midpoint is
//! a midpoint hit, not a tie: a tie between `start` and the midpoint needs a
//! query at a quarter of the line's length.
//!
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
//!   not name a handle of **this** geometry: a `LINE` with a
//!   [`GripKind::Center`]/[`GripKind::Quadrant`]/[`GripKind::Point`] grip, an
//!   `AcDbArc` with an [`GripKind::Endpoint`] grip (arc endpoints are not
//!   implemented), or an `index` outside the handle range (`LINE` endpoint > 1,
//!   `LINE` midpoint ≠ 2, `CIRCLE` centre > 0, `CIRCLE` quadrant > 3, `ARC`
//!   centre > 0, `POINT` > 0). The caller is never trusted to have kept its id in
//!   sync with the geometry.
//! * [`REASON_ZERO_DISPLACEMENT`] (`CadError::InvalidInput`) — the drag would not
//!   change the geometry, so there is nothing to write. That is the target
//!   equalling the dragged handle's current position (endpoint, point, midpoint,
//!   centre, quadrant), and additionally a quadrant target that already sits at
//!   the **stored** radius: its out-of-plane offset is ignored by the radius
//!   rule, so such a drag is equally a no-op. It is an error, not a silent `Ok`:
//!   the command must skip the transaction (and therefore the undo entry).
//!   Equality is IEEE `==` (so `-0.0` equals `0.0`, and no tolerance is applied —
//!   a sub-tolerance drag is a real, tiny edit).
//! * [`REASON_DEGENERATE_RADIUS`] (`CadError::InvalidInput`) — a circle-quadrant
//!   drag whose in-plane distance from the centre is zero or non-finite (the
//!   target is the centre, or the target lies on the plane normal through it). A
//!   circle cannot have radius `0` or `NaN`, and this model never clamps: it
//!   refuses by name.
//! * [`REASON_NON_FINITE_TARGET`] (`CadError::InvalidInput`) — the target has a
//!   `NaN` or infinite coordinate. Storing it would corrupt the entity, and the
//!   database refuses it at commit anyway; refusing here names the cause. The
//!   same code covers a midpoint translation that overflows `f64` (the inputs are
//!   finite but the translated endpoints are not), because the *result* would be
//!   an unstorable coordinate.
//!
//! The checks run in a fixed order: geometry kind, then handle identity, then
//! target finiteness, then the drag's own geometric refusals (for a quadrant
//! drag: "already at the handle" is checked before the radius rule, so a drag to
//! the handle itself reports [`REASON_ZERO_DISPLACEMENT`] rather than a
//! degenerate radius). An unsupported kind is therefore reported as
//! [`REASON_UNSUPPORTED_KIND`] even when the id would also be stale (there is no
//! handle of that kind to be stale *against*), while a supported kind with a
//! mismatched id is always [`REASON_STALE_HANDLE`].
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
use cad_geometry::{add, arbitrary_axis, dot, scale, sub};

/// Stable reason code: the geometry kind exposes no grip model.
pub const REASON_UNSUPPORTED_KIND: &str = "grip.unsupported_kind";
/// Stable reason code: the [`GripId`] does not name a handle of this geometry.
pub const REASON_STALE_HANDLE: &str = "grip.stale_handle";
/// Stable reason code: the drag would not change the geometry.
pub const REASON_ZERO_DISPLACEMENT: &str = "grip.zero_displacement";
/// Stable reason code: the drag target is not a finite world point.
pub const REASON_NON_FINITE_TARGET: &str = "grip.non_finite_target";
/// Stable reason code: a quadrant drag yields a zero or non-finite radius.
pub const REASON_DEGENERATE_RADIUS: &str = "grip.degenerate_radius";
/// Stable reason code: the addressed entity does not exist in the drawing.
///
/// Raised by the `GRIPSTRETCH` command (`crate::app_drawing::grip`), which is the
/// only place that can know an entity id stopped resolving; see the module doc.
pub const REASON_TARGET_MISSING: &str = "grip.target_missing";

/// A draggable handle of one entity.
///
/// The variants are exactly the handles the drag rules above implement; see the
/// module doc for the deliberate absence of insertion/vertex/width/tangent
/// kinds and for the arc-endpoint refusal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GripKind {
    /// A straight-line endpoint (`LINE` only).
    Endpoint,
    /// The midpoint of a straight line (`LINE` only); dragging translates the
    /// whole line by `to - midpoint`.
    Midpoint,
    /// The centre of a circle or an arc; dragging moves the entity rigidly.
    Center,
    /// A quadrant point of a circle (`CIRCLE` only); dragging sets the radius to
    /// the in-plane distance from the centre to the target.
    Quadrant,
    /// The position of a point entity (`POINT` only).
    Point,
}

/// The four circle quadrant handles, in documented `index` order.
///
/// Each entry picks one axis of the circle's own plane frame (`0` = `ax`,
/// `1` = `ay`) and a sign, i.e. `0` = `+ax`, `1` = `+ay`, `2` = `-ax`,
/// `3` = `-ay`. For a circle whose normal is `+Z` (the common case) that is
/// `+X`, `+Y`, `-X`, `-Y` in the circle's own plane.
const QUADRANT_DIRECTIONS: [(u8, f64); 4] = [(0, 1.0), (1, 1.0), (0, -1.0), (1, -1.0)];

/// The identity of one handle: which entity, which kind of handle, which one.
///
/// `index` is the handle's ordinal **within its kind**, in a documented order:
/// for a `LINE`, `0` is `start`, `1` is `end` and `2` is the midpoint; for a
/// `CIRCLE`, `0` is the centre and the quadrants are `0..=3`; for an `ARC`, `0`
/// is the centre; for a `POINT`, `0` is the position. The id is a caller-held
/// reference into geometry that may have changed since it was read, so
/// [`apply_grip`] re-validates it against the geometry it is handed instead of
/// trusting it.
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
    /// The stored handle position (a line midpoint is the exact f64 midpoint of
    /// the two stored endpoints).
    pub position: Point3,
}

/// The handles `entity` exposes in plane space.
///
/// `LINE` ⇒ `start`, `end` endpoints then the midpoint; `CIRCLE` ⇒ the centre
/// then quadrants `0..=3`; `ARC` ⇒ the centre only (arc endpoint grips are NOT
/// IMPLEMENTED); `POINT` ⇒ one [`GripKind::Point`] grip. Every other geometry —
/// including the whole refusal list in the module doc — yields an empty vector:
/// no marker is invented for a kind this build cannot drag.
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
            Grip {
                id: id(GripKind::Midpoint, 2),
                position: midpoint(*start, *end),
            },
        ],
        SemanticGeometry::Circle {
            center,
            normal,
            radius,
        } => {
            let axes = plane_axes(*normal);
            let mut grips = Vec::with_capacity(1 + QUADRANT_DIRECTIONS.len());
            grips.push(Grip {
                id: id(GripKind::Center, 0),
                position: *center,
            });
            for (index, direction) in QUADRANT_DIRECTIONS.iter().enumerate() {
                // `QUADRANT_DIRECTIONS` has exactly four entries, so the index
                // always fits the documented `0..=3` range.
                grips.push(Grip {
                    id: id(GripKind::Quadrant, index as u8),
                    position: quadrant_point(*center, *radius, axes, *direction),
                });
            }
            grips
        }
        SemanticGeometry::Arc { center, .. } => vec![Grip {
            id: id(GripKind::Center, 0),
            position: *center,
        }],
        SemanticGeometry::Point(position) => vec![Grip {
            id: id(GripKind::Point, 0),
            position: *position,
        }],
        _ => Vec::new(),
    }
}

/// The circle/arc plane frame: the `(ax, ay)` of the shared arbitrary-axis frame.
///
/// `ax` and `ay` are orthonormal and lie in the plane whose normal is `normal`;
/// they are the same axes the tessellator, the representation provider and the
/// importer measure circle/arc angles in.
fn plane_axes(normal: Point3) -> (Point3, Point3) {
    let (ax, ay, _) = arbitrary_axis(normal);
    (ax, ay)
}

/// The exact f64 midpoint of two stored points: `(start + end) * 0.5`.
fn midpoint(start: Point3, end: Point3) -> Point3 {
    scale(add(start, end), 0.5)
}

/// One quadrant handle position: `centre + radius * (sign * axis)`.
fn quadrant_point(
    center: Point3,
    radius: f64,
    axes: (Point3, Point3),
    direction: (u8, f64),
) -> Point3 {
    let (which, sign) = direction;
    let axis = if which == 0 { axes.0 } else { axes.1 };
    add(center, scale(axis, sign * radius))
}

/// The distance from `center` to `to`, **projected into the plane** spanned by
/// the orthonormal in-plane axes `axes`.
///
/// This is the circle-quadrant radius rule: the out-of-plane component of
/// `to - center` is discarded, so a drag can never tilt the circle.
fn in_plane_distance(center: Point3, axes: (Point3, Point3), to: Point3) -> f64 {
    let offset = sub(to, center);
    dot(offset, axes.0).hypot(dot(offset, axes.1))
}

/// Hit-test a world position against the handles of `entities`.
///
/// Returns the closest handle within `tolerance` world units, or `None`. A
/// candidate must satisfy `distance <= tolerance` (so a handle exactly at the
/// tolerance is hit, and `tolerance == 0.0` only hits an exactly coincident
/// handle). Entities that expose no grips are skipped. Ties are broken by the
/// documented iteration order: the **earliest entity of the input** wins, and
/// within one entity the **earlier grip of the list** wins (a candidate must be
/// strictly closer to replace the current winner).
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
/// earliest entity of the input and, within one entity, to the earlier grip of
/// the list), same `distance <= tolerance` predicate and same finiteness
/// refusals — with **one** difference: the distance is measured in the x/y plane
/// only, so the `z` coordinates of `world` and of the handle are ignored.
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
/// * `LINE` endpoint: the dragged endpoint takes `to`, the other endpoint is
///   returned **bit-identically** (the very same `f64` values);
/// * `LINE` midpoint: both endpoints move by `to - midpoint`, so the line
///   translates rigidly (same direction, same length);
/// * `CIRCLE` centre: the centre takes `to`; `radius`/`normal` are bit-identical;
/// * `CIRCLE` quadrant: `radius` becomes the in-plane distance from the centre to
///   `to`; `center`/`normal` are bit-identical;
/// * `ARC` centre: the arc translates rigidly; `radius`/`normal`/`start`/`sweep`
///   are bit-identical (arc endpoint grips are NOT IMPLEMENTED);
/// * `POINT`: the position becomes `to`;
/// * everything else, a [`GripId`] that does not name a handle of this geometry,
///   a non-finite target and a drag with no geometric effect are refused with the
///   stable reason codes documented in the module doc.
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
        SemanticGeometry::Line { start, end } => apply_line(*start, *end, id, to),
        SemanticGeometry::Circle {
            center,
            normal,
            radius,
        } => apply_circle(*center, *normal, *radius, id, to),
        SemanticGeometry::Arc {
            center,
            normal,
            radius,
            start,
            sweep,
        } => apply_arc(*center, *normal, *radius, *start, *sweep, id, to),
        SemanticGeometry::Point(position) => {
            match (id.kind, id.index) {
                (GripKind::Point, 0) => {}
                (GripKind::Point, index) => {
                    return Err(stale_handle(format!(
                        "AcDbPoint has no Point grip at index {index} (only 0)"
                    )));
                }
                (
                    GripKind::Endpoint | GripKind::Midpoint | GripKind::Center | GripKind::Quadrant,
                    _,
                ) => {
                    return Err(stale_handle(
                        "AcDbPoint exposes a Point grip (0) only".to_string(),
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

/// The `LINE` drag rules (two endpoints, then the translating midpoint).
fn apply_line(start: Point3, end: Point3, id: &GripId, to: Point3) -> CadResult<SemanticGeometry> {
    match (id.kind, id.index) {
        (GripKind::Endpoint, 0) => {
            finite_target(to)?;
            if to == start {
                return Err(zero_displacement("AcDbLine start endpoint"));
            }
            Ok(SemanticGeometry::Line { start: to, end })
        }
        (GripKind::Endpoint, 1) => {
            finite_target(to)?;
            if to == end {
                return Err(zero_displacement("AcDbLine end endpoint"));
            }
            Ok(SemanticGeometry::Line { start, end: to })
        }
        (GripKind::Endpoint, index) => Err(stale_handle(format!(
            "AcDbLine has no Endpoint grip at index {index} (0 = start, 1 = end)"
        ))),
        (GripKind::Midpoint, 2) => {
            finite_target(to)?;
            let from = midpoint(start, end);
            if to == from {
                return Err(zero_displacement("AcDbLine midpoint"));
            }
            // One delta for both endpoints: the line translates, so it stays
            // parallel to itself and keeps its length.
            let delta = sub(to, from);
            let new_start = add(start, delta);
            let new_end = add(end, delta);
            if !is_finite(new_start) || !is_finite(new_end) {
                // Finite inputs can still overflow to infinity; storing that
                // would corrupt the entity, so it is refused like any other
                // unstorable coordinate.
                return Err(overflowed("AcDbLine midpoint"));
            }
            Ok(SemanticGeometry::Line {
                start: new_start,
                end: new_end,
            })
        }
        (GripKind::Midpoint, index) => Err(stale_handle(format!(
            "AcDbLine has no Midpoint grip at index {index} (only 2)"
        ))),
        (GripKind::Center | GripKind::Quadrant | GripKind::Point, _) => Err(stale_handle(
            "AcDbLine exposes Endpoint (0 = start, 1 = end) and Midpoint (2) grips only"
                .to_string(),
        )),
    }
}

/// The `CIRCLE` drag rules (centre, then the four in-plane quadrants).
fn apply_circle(
    center: Point3,
    normal: Point3,
    radius: f64,
    id: &GripId,
    to: Point3,
) -> CadResult<SemanticGeometry> {
    match (id.kind, id.index) {
        (GripKind::Center, 0) => {
            finite_target(to)?;
            if to == center {
                return Err(zero_displacement("AcDbCircle centre"));
            }
            Ok(SemanticGeometry::Circle {
                center: to,
                normal,
                radius,
            })
        }
        (GripKind::Center, index) => Err(stale_handle(format!(
            "AcDbCircle has no Center grip at index {index} (only 0)"
        ))),
        (GripKind::Quadrant, index @ 0..=3) => {
            finite_target(to)?;
            let axes = plane_axes(normal);
            let direction = QUADRANT_DIRECTIONS[usize::from(index)];
            if to == quadrant_point(center, radius, axes, direction) {
                return Err(zero_displacement("AcDbCircle quadrant"));
            }
            let new_radius = in_plane_distance(center, axes, to);
            if !new_radius.is_finite() || new_radius <= 0.0 {
                return Err(degenerate_radius(new_radius));
            }
            if new_radius == radius {
                // A target off the plane at exactly the stored radius has no
                // geometric effect (the radius rule ignores the out-of-plane
                // offset), so this is a zero-displacement refusal too: never an
                // `Ok` that would stage an unchanged entity.
                return Err(unchanged_radius(new_radius));
            }
            Ok(SemanticGeometry::Circle {
                center,
                normal,
                radius: new_radius,
            })
        }
        (GripKind::Quadrant, index) => Err(stale_handle(format!(
            "AcDbCircle has no Quadrant grip at index {index} \
             (0 = +ax, 1 = +ay, 2 = -ax, 3 = -ay)"
        ))),
        (GripKind::Endpoint | GripKind::Midpoint | GripKind::Point, _) => Err(stale_handle(
            "AcDbCircle exposes a Center grip (0) and Quadrant grips (0..=3) only".to_string(),
        )),
    }
}

/// The `ARC` drag rule: the centre grip only (arc endpoints are NOT IMPLEMENTED).
fn apply_arc(
    center: Point3,
    normal: Point3,
    radius: f64,
    start: f64,
    sweep: f64,
    id: &GripId,
    to: Point3,
) -> CadResult<SemanticGeometry> {
    match (id.kind, id.index) {
        (GripKind::Center, 0) => {
            finite_target(to)?;
            if to == center {
                return Err(zero_displacement("AcDbArc centre"));
            }
            Ok(SemanticGeometry::Arc {
                center: to,
                normal,
                radius,
                start,
                sweep,
            })
        }
        (GripKind::Center, index) => Err(stale_handle(format!(
            "AcDbArc has no Center grip at index {index} (only 0)"
        ))),
        (GripKind::Endpoint, _) => Err(stale_handle(
            "AcDbArc endpoint grips are NOT IMPLEMENTED; an AcDbArc exposes a Center grip (0) only"
                .to_string(),
        )),
        (GripKind::Quadrant, _) => Err(stale_handle(
            "AcDbArc exposes a Center grip (0) only, not a Quadrant grip".to_string(),
        )),
        (GripKind::Midpoint | GripKind::Point, _) => Err(stale_handle(
            "AcDbArc exposes a Center grip (0) only".to_string(),
        )),
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

/// A `grip.zero_displacement` refusal for a quadrant drag whose radius is
/// already stored (an off-plane target at the current radius).
fn unchanged_radius(radius: f64) -> CadError {
    CadError::InvalidInput(format!(
        "{REASON_ZERO_DISPLACEMENT}: the AcDbCircle quadrant drag leaves the radius at {radius}"
    ))
}

/// A `grip.degenerate_radius` refusal naming the rejected radius.
fn degenerate_radius(radius: f64) -> CadError {
    CadError::InvalidInput(format!(
        "{REASON_DEGENERATE_RADIUS}: the AcDbCircle quadrant drag yields radius {radius}; \
         a circle radius must be positive and finite (the centre is not moved by this handle)"
    ))
}

/// A `grip.non_finite_target` refusal for a finite input whose result overflows.
fn overflowed(handle: &str) -> CadError {
    CadError::InvalidInput(format!(
        "{REASON_NON_FINITE_TARGET}: translating the {handle} produces a non-finite coordinate"
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
