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
//! | `SemanticGeometry::Arc` | [`GripKind::Center`] `0`, then [`GripKind::Endpoint`] `0` = start point, `1` = end point |
//! | `SemanticGeometry::Ellipse` | [`GripKind::Center`] `0`, then [`GripKind::Quadrant`] `0..=3` — **full ellipses only** (see below) |
//! | `SemanticGeometry::Polyline` | [`GripKind::Vertex`] `index` = stored vertex index |
//! | `SemanticGeometry::Spline` | [`GripKind::ControlPoint`] `index` = stored control-point index |
//! | `SemanticGeometry::Point` | [`GripKind::Point`] `0` |
//!
//! The `ARC` order is `Center 0, Endpoint 0, Endpoint 1`: the centre comes
//! first because it was the only handle before the endpoints were added, and the
//! list order is the hit-test candidate order (see the tie-break rule), so
//! keeping it first preserves every tie the centre used to win.
//!
//! `POLYLINE` is the one semantic variant for **every** polyline class the
//! importer produces (LWPOLYLINE, 2D `AcDb2dPolyline`, legacy `AcDbPolyline` and
//! 3D `AcDb3dPolyline`): the class is not carried into [`SemanticGeometry`], so
//! this model cannot single out the 3D form and vertex grips therefore apply to
//! all of them. Distinguishing (and refusing) the 3D class is NOT IMPLEMENTED
//! and would need a domain change, not a guess here. A vertex drag only moves
//! x/y and keeps the vertex's own `z`, exactly like a `LINE` endpoint drag, so
//! it never edits a 3D polyline's elevation.
//!
//! `SPLINE` is a NURBS definition, and its control points are **not on the
//! curve**: a B-spline (rational or not) passes through its first and last control
//! point only when the end knots have full multiplicity, and never through the
//! interior ones. That is not a bug and not an approximation: the
//! stored `control_points` are the definition, the drawn curve is the
//! de-Boor/NURBS evaluation of `(degree, knots, control_points, weights)`. A
//! control-point grip therefore sits at the stored control point (a marker off
//! the drawn curve is correct), and a drag that redefines one of them **reshapes**
//! the curve — after such a drag the curve no longer passes through the points it
//! used to pass through. No handle names a point **on** the drawn curve: this
//! model exposes the definition points only, not the curve's endpoints (see the
//! drag rules). We do not claim vendor equivalence for the preview/reshape feel;
//! AutoCAD also drags control points without re-fitting, but that is a
//! resemblance, not a tested claim.
//!
//! Only control-point indices `0..=255` are addressable because [`GripId::index`]
//! is a `u8`: a spline with more than 256 stored control points exposes the first
//! 256 (exactly like a [`GripKind::Vertex`] polyline) and the rest are simply
//! unreachable in this build. Nothing is wrapped, truncated or renumbered.
//!
//! A spline whose stored definition is **internally inconsistent** —
//! `control_points.len() != weights.len()`, or
//! `knots.len() != control_points.len() + degree + 1` — exposes **no** grips at
//! all, and every [`GripKind::ControlPoint`] id against it is
//! [`REASON_STALE_HANDLE`]. We do not guess a repaired definition (padding a knot
//! vector or a weight list would silently change the curve), and we do not
//! validate the shape either: `cad-db`'s validator is the authority on what may be
//! stored. This model only declines to invent a handle for a definition it cannot
//! interpret.
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
//! * **`ARC` endpoint** (`index` 0 = start point, `1` = end point): the arc stays
//!   on the **same circle** — `center`, `radius` and `normal` are bit-identical —
//!   and only the dragged endpoint's angle changes. That angle becomes the
//!   in-plane direction of the target from the centre (the out-of-plane offset,
//!   including the target's `z`, is ignored: the arc's plane is fixed by its own
//!   normal). The **opposite endpoint keeps its angle**, so the end angle is the
//!   stored `start + sweep` when the start is dragged, and the stored `start`
//!   when the end is dragged. The sweep is then **recomputed in the stored
//!   direction** (positive for a counter-clockwise arc, negative for a
//!   clockwise/reflected one) as the in-plane extent between the two angles, and
//!   must stay in the **open** interval `(0, 2π)` in that direction.
//!
//!   A target that collapses the arc — a recomputed sweep within
//!   [`ARC_SWEEP_EPSILON`] (`1e-9` rad, the same slack the tessellator and the
//!   representation provider use to call a sweep "full") of `0` or `2π` — is
//!   refused by name ([`REASON_DEGENERATE_ARC`]) and **never clamped** into the
//!   open interval. So is a target with no in-plane direction at all (the centre
//!   itself, or a point on the normal through it). **This endpoint rule is our
//!   choice, not an AutoCAD claim**: it is what keeps a single-handle drag from
//!   tilting the arc, and no vendor behaviour is asserted by it.
//! * **`ELLIPSE` centre** (`index` 0): the ellipse moves **rigidly** — `center`
//!   takes the target and `normal` / `major_axis` / `ratio` / `start` / `sweep`
//!   are returned bit-identically.
//! * **`ELLIPSE` quadrant** (`index` `0..=3`, documented order in the ellipse's
//!   own stored frame: `0` = `+major`, `1` = `+minor`, `2` = `-major`,
//!   `3` = `-minor`, i.e. the same `+, +, -, -` ordering as the circle
//!   quadrants): the drag sets the **corresponding radius** to the distance from
//!   the centre to the target measured **in the ellipse's own plane** (never a
//!   3D distance and never an x/y distance). `center` and `normal` are
//!   bit-identical. Which stored field carries that radius depends on the
//!   handle: the ellipse stores its major semi-axis **as the `major_axis`
//!   vector** (`|major_axis|` is the major radius) and its minor radius as
//!   `|major_axis| * ratio`, so
//!   * a **major** quadrant (`0`/`2`) sets `|major_axis|` to the measured
//!     distance and keeps the stored **minor radius** (`major_len * |ratio|`)
//!     unchanged, which recomputes `ratio = minor_len / new_major_len`;
//!   * a **minor** quadrant (`1`/`3`) sets the minor radius to the measured
//!     distance, i.e. `ratio = distance / |major_axis|`, and leaves
//!     `major_axis` bit-identical.
//!
//!   **Axis direction never rotates** (our choice, **not an AutoCAD claim**): a
//!   major drag keeps the stored major direction and only changes its length,
//!   so `major_axis` is the stored vector scaled by
//!   `distance / |major_axis|`. This is the rule that cannot produce a
//!   degenerate ellipse (the direction is never re-derived from a target that
//!   might sit on the centre, and the two axes stay orthogonal by construction),
//!   and it is the same "one handle, one radius" shape as the circle quadrant
//!   rule. A target off the major axis therefore still sets the major **radius**
//!   from its full in-plane distance; it does not turn the ellipse.
//!
//!   The domain model has **no** `minor <= major` ordering: `cad-db` validation
//!   only requires a positive finite `ratio`, and the tessellator and
//!   representation provider draw `|major_axis| * |ratio|` as the minor
//!   semi-axis, so `ratio > 1` is a representable, correctly drawn ellipse whose
//!   stored axis roles are simply swapped. A drag that pushes one radius past
//!   the other is therefore **not** refused (there is no invariant to break) and
//!   is **not** silently re-normalized (swapping the axes would rotate the
//!   stored frame and change the parameterisation behind the user's back). Only
//!   the create path (`draw_tool::ellipse_parameters`) happens to keep
//!   `ratio <= 1`; that is a storage assumption of the axis-endpoint method, not
//!   a grip invariant.
//!
//!   A zero or non-finite in-plane distance is refused by name
//!   ([`REASON_DEGENERATE_RADIUS`]) and **never clamped**; so is a stored frame
//!   from which no in-plane radius can be derived (non-finite/zero major axis or
//!   a normal parallel to it). A drag that leaves the dragged radius at its
//!   stored length is [`REASON_ZERO_DISPLACEMENT`], exactly like the circle rule
//!   (a target off the plane at the same in-plane distance has no geometric
//!   effect).
//! * **`ELLIPSE` partial (elliptical arc)**: `SemanticGeometry::Ellipse` carries
//!   a `start`/`sweep` parameter range, and a range that is **not** a full turn
//!   is an elliptical arc. Such an entity exposes the **centre grip only**;
//!   quadrant drags are refused by name ([`REASON_PARTIAL_ELLIPSE`], **our
//!   choice, not an AutoCAD claim**). The four axis points are stored-parameter
//!   quadrant points, and on a partial ellipse some of them lie outside the
//!   drawn range — reshaping the arc from a handle that is not on the drawn curve
//!   would be a silent reinterpretation of the stored parameter range, so we do
//!   not offer it. The **centre** drag stays available because it changes no
//!   radius and no parameter, so the arc's `start`/`sweep` remain meaningful
//!   bit-for-bit. "Full turn" uses the same slack as the tessellator and the
//!   representation provider (`|sweep| >= 2π - 1e-9`), so the handle set and the
//!   drawn curve always agree about which ellipses are complete.
//! * **`POLYLINE` vertex** (`index` = stored vertex index): the dragged vertex
//!   takes `x`/`y` from the target and keeps its own stored `z`, so the drag
//!   never changes an elevation (3D vertex editing is NOT IMPLEMENTED). Every
//!   other vertex is returned **bit-identically**, and `closed` plus the whole
//!   `bulges` vector are copied verbatim. A bulge is therefore **never
//!   recomputed** for the moved vertex: the segment entering and the segment
//!   leaving it keep the bulge they were stored with (our reading of how the
//!   handle behaves — **not vendor-verified**). An `index` outside the stored
//!   vertices is [`REASON_STALE_HANDLE`].
//! * **`SPLINE` control point** (`index` = stored control-point index): the
//!   dragged entry of `control_points` takes the target **verbatim** (all three
//!   coordinates, no projection: a control point is a free 3D definition point,
//!   not a point on a plane) and `degree`, `knots` and `weights` are returned
//!   **bit-identically**. No knot and no weight is recomputed, renormalised or
//!   re-fitted: the result is a **different NURBS** that happens to share the
//!   parameterisation — that is our choice (AutoCAD also drags control points
//!   without re-fitting, but no vendor equivalence is claimed), and it is the
//!   reason a control-point drag changes the shape of the whole curve rather than
//!   moving a point along it. Every other control point is bit-identical. An
//!   `index` outside the stored control points — and any id against an
//!   internally inconsistent definition — is [`REASON_STALE_HANDLE`].
//! * **`POINT`**: the position becomes the target.
//!
//! ## Explicit refusal list (no marker, no hit)
//!
//! `INSERT`/block-member geometry (an instance transform is not an entity
//! handle), `TEXT`/`MTEXT`/`MULTILEADER` (insertion-point and width grips),
//! `HATCH`, `DIMENSION` (definition-point grips; the dimension is
//! non-associative by design), `IMAGE`/`MASK`, and opaque/compound geometry
//! (`Opaque`, `Compound`, `Mesh`, `Shape`). `ELLIPSE` is **no longer** on this
//! list: a full ellipse exposes centre + quadrant handles (see the drag rules),
//! and a partial (elliptical-arc) ellipse exposes the centre grip only, with
//! quadrant ids refused by [`REASON_PARTIAL_ELLIPSE`]. `SPLINE` is **no longer**
//! on this list either: it exposes one [`GripKind::ControlPoint`] per stored
//! control point (see above).
//!
//! Insertion-point, width, rotation and tangent grips are **NOT IMPLEMENTED**.
//! They are not modelled as [`GripKind`] variants either: a variant that always
//! yields an empty vector or an error would be a placeholder pretending to be a
//! capability. A later lane that implements one of them adds its variant **and**
//! its drag rule together (as this lane did for [`GripKind::Vertex`]).
//!
//! ## Coordinates: 3D-exact, never projected (except the three in-plane rules)
//!
//! A handle's [`Grip::position`] is a stored or stored-derived point — the same
//! `f64` bits as the geometry (an endpoint, a point position, a circle/arc/ellipse
//! centre, a polyline vertex, a spline control point) or the documented exact
//! derivation of it (a line
//! midpoint; an arc endpoint, i.e. the stored
//! `center + radius * (cos angle * ax + sin angle * ay)` in the arc's own plane
//! frame, the same expression the tessellator uses; an ellipse quadrant, i.e.
//! `center ± major_axis` and `center ± ` the stored minor semi-axis vector in the
//! ellipse's own frame, again the tessellator's own expression). Nothing is
//! rounded and nothing is snapped.
//!
//! A drag assigns the target point to the dragged handle in **all three**
//! coordinates; nothing is projected onto a work plane or onto Z = 0. The caller
//! resolves the pointer into the world point it means — the host (`cad-host`'s
//! `input/grip_drag.rs`) resolves it in the plan view (x/y from
//! `screen_to_world`, plus the dragged handle's own `z`, so a plan-view drag
//! never changes an elevation) — and this module stores whatever finite target it
//! is given. A non-finite target is refused (see below) instead of stored.
//!
//! The **three** projections this module performs are the circle-quadrant radius,
//! the arc-endpoint angle and the ellipse-quadrant radius: all are measured in
//! the entity's **own** plane, precisely so that a non-planar drag cannot deform a
//! circle, tilt an arc out of its plane, or turn an ellipse. Those projections
//! apply to the *drag target of a quadrant or an arc-endpoint handle* only; a
//! centre drag, an arc-rigid drag, a line drag, a polyline vertex drag, a spline
//! control-point drag and a point drag are stored 3D-exact (a polyline vertex
//! keeps its own `z`), and no handle position is ever projected back onto a plane.
//!
//! ## The plane frame is shared, not invented
//!
//! Quadrant handle positions of a **circle**, the circle in-plane radius, and the
//! arc endpoint positions and angles use `cad_geometry::arbitrary_axis` — the
//! AutoCAD arbitrary-axis-algorithm frame the tessellator, representation
//! provider, intersect/transform code and the importer already measure circle/arc
//! angles in. It returns `(ax, ay, unit normal)`, with `ax`/`ay` orthonormal and
//! inside the circle plane, so `dot(to - center, ax)` and `dot(to - center, ay)`
//! are the exact in-plane coordinates of a displacement, and
//! `atan2(dot(…, ay), dot(…, ax))` is the exact in-plane angle of a direction.
//!
//! An **ellipse** has no free choice of frame: its quadrant handles are its own
//! stored axes, so they use the ellipse frame the tessellator and the
//! representation provider draw with — `(unit(major_axis),
//! normalize(cross(unit(normal), unit(major_axis))))` — which is the expression
//! `cad_geometry`'s (crate-private) `ellipse_minor_dir` evaluates. Feeding
//! `arbitrary_axis` here would put the "quadrant" handles off the drawn curve
//! whenever the stored major axis is not aligned with the arbitrary frame, so the
//! ellipse reuses the stored-axis convention instead, and it does **not** invent a
//! second one: there is no new angle convention, no re-normalised axis and no
//! rotated frame in this module.
//!
//! ## Hit-testing rule
//!
//! [`grip_hit`] returns the **closest** handle whose Euclidean world distance to
//! `world` is `<= tolerance`. Candidates are visited in a fixed order — the
//! entities in the order the caller supplied them, and within one entity its
//! grips in list order (see the table: for a `LINE` `start`, `end`, midpoint; for
//! a `CIRCLE` the centre and then quadrants 0..=3; for an `ARC` the centre and
//! then endpoint 0 = start, endpoint 1 = end; for an `ELLIPSE` the centre and then
//! quadrants 0..=3 when it is a full ellipse; for a `POLYLINE` the vertices in
//! stored order; for a `SPLINE` the control points in stored order) — and a
//! candidate replaces the current winner only when it is
//! **strictly** closer.
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
//! `Midpoint` for a `LINE`, `Center` before every `Quadrant` for a `CIRCLE` or a
//! full `ELLIPSE`, `Center` before `Endpoint` `0` before `Endpoint` `1` for an
//! `ARC`). For a
//! `POLYLINE` the list is the stored vertex order, so the lower vertex index wins
//! a tie; for a `SPLINE` it is the stored control-point order, so the lower
//! control-point index wins a tie and a query exactly at a control point is that
//! control point's hit (it is strictly closer than every other one). This is
//! deterministic and is pinned by contract tests. Note that the
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
//! [`apply_grip`] refuses with an existing `CadError` variant whose message
//! **starts with** one of these codes:
//!
//! * [`REASON_UNSUPPORTED_KIND`] (`CadError::Unsupported`) — the geometry kind
//!   is on the refusal list above and exposes no grip model at all. The message
//!   names the DXF class.
//! * [`REASON_STALE_HANDLE`] (`CadError::InvalidInput`) — the [`GripId`] does
//!   not name a handle of **this** geometry: a `LINE` with a
//!   [`GripKind::Center`]/[`GripKind::Quadrant`]/[`GripKind::Point`]/[`GripKind::Vertex`]/[`GripKind::ControlPoint`]
//!   grip, an `AcDbCircle`/`AcDbArc` with a [`GripKind::Vertex`] grip, a
//!   `POLYLINE` with a non-vertex grip, a `SPLINE` with a non-control-point grip,
//!   or an `index` outside the handle range
//!   (`LINE` endpoint > 1, `LINE` midpoint ≠ 2, `CIRCLE`/`ELLIPSE` centre > 0,
//!   `CIRCLE`/`ELLIPSE` quadrant > 3, `ARC` centre > 0, `ARC` endpoint > 1,
//!   `POLYLINE` vertex ≥ the stored vertex count,
//!   `SPLINE` control point ≥ the stored control-point count, `POINT` > 0). It is
//!   also the refusal for **every** control-point id on an internally
//!   inconsistent spline definition, because such a spline exposes no handle at
//!   all. The caller is
//!   never trusted to have kept its id in sync with the geometry.
//! * [`REASON_ZERO_DISPLACEMENT`] (`CadError::InvalidInput`) — the drag would not
//!   change the geometry, so there is nothing to write. That is the target
//!   equalling the dragged handle's current position (endpoint, arc endpoint,
//!   point, midpoint, centre, quadrant, polyline vertex, spline control point),
//!   and additionally:
//!   a quadrant target that already sits at the **stored** radius (its
//!   out-of-plane offset is ignored by the radius rule), an arc endpoint drag
//!   whose recomputed start/sweep are identical to the stored ones (an
//!   off-plane target along the same ray, which the angle rule ignores), an
//!   ellipse-quadrant drag whose measured distance equals the stored radius of
//!   that handle (the axis direction is fixed, so such a target changes nothing),
//!   and a
//!   vertex target whose x/y equal the stored vertex (the drag keeps the stored
//!   `z`, so a z-only difference changes nothing). It is an error, not a silent
//!   `Ok`: the command must skip the transaction (and therefore the undo entry).
//!   Equality is IEEE `==` (so `-0.0` equals `0.0`, and no tolerance is applied —
//!   a sub-tolerance drag is a real, tiny edit).
//! * [`REASON_DEGENERATE_RADIUS`] (`CadError::InvalidInput`) — a circle- or
//!   ellipse-quadrant drag whose in-plane distance from the centre is zero or
//!   non-finite (the target is the centre, or the target lies on the plane normal
//!   through it), or an ellipse-quadrant drag on a stored frame no in-plane radius
//!   can be derived from (non-finite/zero major axis, or a normal parallel to it).
//!   A radius cannot be `0` or `NaN`, and this model never clamps: it refuses by
//!   name.
//! * [`REASON_DEGENERATE_ARC`] (`CadError::InvalidInput`) — an arc-endpoint drag
//!   that has no in-plane direction (target at the centre or on the normal
//!   through it), that produces a non-finite sweep, or whose sweep lands within
//!   [`ARC_SWEEP_EPSILON`] of `0` or `2π`. The arc must keep an extent strictly
//!   inside `(0, 2π)`; the sweep is **refused by name, never clamped**.
//! * [`REASON_PARTIAL_ELLIPSE`] (`CadError::InvalidInput`) — an ellipse-quadrant
//!   id on a **partial** ellipse (one whose stored `|sweep|` is not a full turn).
//!   Such an entity exposes the centre grip only; the refusal names the reason
//!   instead of pretending the handle is stale, and it is checked in the handle
//!   identity position (before the target's finiteness), because no quadrant
//!   handle of that geometry exists to be validated.
//! * [`REASON_NON_FINITE_TARGET`] (`CadError::InvalidInput`) — the target has a
//!   `NaN` or infinite coordinate. Storing it would corrupt the entity, and the
//!   database refuses it at commit anyway; refusing here names the cause. The
//!   same code covers a midpoint translation that overflows `f64` (the inputs are
//!   finite but the translated endpoints are not), because the *result* would be
//!   an unstorable coordinate.
//!
//! The checks run in a fixed order: geometry kind, then handle identity (which
//! includes "this kind has no handle on this geometry at all", such as a
//! quadrant id on a partial ellipse), then target finiteness, then the drag's own
//! geometric refusals. "Already at the
//! handle" is checked before a drag's geometric rule, so a drag onto the handle
//! itself reports [`REASON_ZERO_DISPLACEMENT`] rather than a degenerate radius or
//! arc. An unsupported kind is therefore reported as
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
use cad_domain::{EntityId, Point3, SemanticGeometry};
use cad_geometry::{add, arbitrary_axis, cross, dot, length, normalize, scale, sub};

mod drag;
pub use drag::apply_grip;
pub(crate) use drag::finite_target;

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
/// Stable reason code: an arc-endpoint drag would collapse the arc's sweep.
pub const REASON_DEGENERATE_ARC: &str = "grip.degenerate_arc";
/// Stable reason code: a quadrant id on a **partial** ellipse (an elliptical arc).
pub const REASON_PARTIAL_ELLIPSE: &str = "grip.partial_ellipse";
/// Stable reason code: the addressed entity does not exist in the drawing.
///
/// Raised by the `GRIPSTRETCH` command (`crate::app_drawing::grip`), which is the
/// only place that can know an entity id stopped resolving; see the module doc.
pub const REASON_TARGET_MISSING: &str = "grip.target_missing";

/// The angular slack that makes a recomputed arc sweep degenerate.
///
/// `1e-9` rad is the same slack the tessellator (`cad-geometry/src/tessellate.rs`)
/// and the representation provider use to call an existing sweep "full"; it is
/// reused here so "the arc vanished / became the whole circle" has one threshold
/// in the codebase. A sweep inside it is refused, never clamped.
pub const ARC_SWEEP_EPSILON: f64 = 1e-9;

/// A draggable handle of one entity.
///
/// The variants are exactly the handles the drag rules above implement; see the
/// module doc for the deliberate absence of insertion/width/rotation/tangent
/// kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GripKind {
    /// An endpoint: a `LINE`'s start/end, or an `ARC`'s start (`index` 0) and end
    /// (`index` 1) point.
    Endpoint,
    /// The midpoint of a straight line (`LINE` only); dragging translates the
    /// whole line by `to - midpoint`.
    Midpoint,
    /// The centre of a circle, an arc or an ellipse; dragging moves the entity
    /// rigidly.
    Center,
    /// A quadrant point of a circle, or an axis point of a **full** ellipse
    /// (`index` `0..=3`); dragging sets the corresponding radius to the in-plane
    /// distance from the centre to the target. A partial ellipse (an elliptical
    /// arc) exposes no quadrant handle.
    Quadrant,
    /// A stored vertex of a polyline (`POLYLINE`/`LWPOLYLINE` only, `index` = the
    /// vertex index in stored order); dragging moves that one vertex in x/y and
    /// keeps its own `z`, leaving the other vertices and every bulge untouched.
    Vertex,
    /// A stored control point of a NURBS spline (`SPLINE` only, `index` = the
    /// control-point index in stored order). A control point belongs to the curve
    /// **definition** and is generally **not on** the drawn curve; dragging it
    /// redefines that one entry and keeps `degree`/`knots`/`weights` bit-identical,
    /// so the drag reshapes the curve as a different NURBS — it never re-fits it,
    /// and the curve stops passing through the points it used to pass through.
    ControlPoint,
    /// The position of a point entity (`POINT` only).
    Point,
}

/// The four circle/ellipse quadrant handles, in documented `index` order.
///
/// Each entry picks one axis of the entity's own plane frame (`0` = the first
/// axis, `1` = the second axis) and a sign, i.e. `0` = `+axis0`, `1` = `+axis1`,
/// `2` = `-axis0`, `3` = `-axis1`. For a circle the frame is
/// `cad_geometry::arbitrary_axis`, so a `+Z` circle reads `+X`, `+Y`, `-X`, `-Y`;
/// for an ellipse the frame is its stored `(unit major, unit minor)` axes, so the
/// same order reads `+major`, `+minor`, `-major`, `-minor`.
const QUADRANT_DIRECTIONS: [(u8, f64); 4] = [(0, 1.0), (1, 1.0), (0, -1.0), (1, -1.0)];

/// The identity of one handle: which entity, which kind of handle, which one.
///
/// `index` is the handle's ordinal **within its kind**, in a documented order:
/// for a `LINE`, `0` is `start`, `1` is `end` and `2` is the midpoint; for a
/// `CIRCLE`/`ELLIPSE`, `0` is the centre and the quadrants are `0..=3`; for an
/// `ARC`, `0` is the centre, `Endpoint` `0` is the start point and `Endpoint` `1`
/// the end point; for a `POLYLINE`, a `Vertex` `index` is the **stored** vertex
/// index (`u8`, so only vertices `0..=255` can be addressed); for a `SPLINE`, a
/// `ControlPoint` `index` is the **stored** control-point index (`u8`, so only
/// control points `0..=255` can be addressed); for a `POINT`, `0`
/// is the position. The id is a caller-held reference into geometry that may have
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
    /// The stored handle position (a line midpoint is the exact f64 midpoint of
    /// the two stored endpoints).
    pub position: Point3,
}

/// The handles `entity` exposes in plane space.
///
/// `LINE` ⇒ `start`, `end` endpoints then the midpoint; `CIRCLE` ⇒ the centre
/// then quadrants `0..=3`; `ARC` ⇒ the centre, then the start (`Endpoint` `0`)
/// and end (`Endpoint` `1`) points; `ELLIPSE` ⇒ the centre, then quadrants
/// `0..=3` **when the stored parameter range is a full turn** (a partial ellipse
/// exposes the centre grip only, and its quadrant ids are
/// [`REASON_PARTIAL_ELLIPSE`]); `POLYLINE` ⇒ one [`GripKind::Vertex`] per
/// stored vertex, in stored order (`0..=255`; a longer polyline exposes the first
/// 256 vertices only, because [`GripId::index`] is a `u8` — the rest are simply
/// not addressable in this build); `SPLINE` ⇒ one [`GripKind::ControlPoint`] per
/// stored control point, in stored order (the same `0..=255` `u8` cap: a spline
/// with more than 256 control points exposes the first 256 and the rest are not
/// addressable), and **no** grip at all when the stored definition is internally
/// inconsistent (`control_points.len() != weights.len()`, or
/// `knots.len() != control_points.len() + degree + 1`) — no marker is invented for
/// a definition this model cannot interpret; `POINT` ⇒ one [`GripKind::Point`]
/// grip. Every
/// other geometry — including the whole refusal list in the module doc — yields
/// an empty vector: no marker is invented for a kind this build cannot drag.
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
        SemanticGeometry::Arc {
            center,
            normal,
            radius,
            start,
            sweep,
        } => {
            let axes = plane_axes(*normal);
            vec![
                Grip {
                    id: id(GripKind::Center, 0),
                    position: *center,
                },
                Grip {
                    id: id(GripKind::Endpoint, 0),
                    position: arc_point(*center, *radius, axes, *start),
                },
                Grip {
                    id: id(GripKind::Endpoint, 1),
                    position: arc_point(*center, *radius, axes, *start + *sweep),
                },
            ]
        }
        SemanticGeometry::Ellipse {
            center,
            normal,
            major_axis,
            ratio,
            sweep,
            ..
        } => {
            let mut grips = Vec::with_capacity(1 + QUADRANT_DIRECTIONS.len());
            grips.push(Grip {
                id: id(GripKind::Center, 0),
                position: *center,
            });
            // A partial ellipse (an elliptical arc) stores a parameter range, and
            // its axis points need not lie on the drawn range: the centre is the
            // only handle offered. A full ellipse whose stored frame is
            // degenerate (`ellipse_axes` returns `None`) also offers the centre
            // only — a quadrant marker with an underivable position is never
            // fabricated.
            if is_full_ellipse(*sweep) {
                if let Some(axes) = ellipse_axes(*normal, *major_axis) {
                    for (index, direction) in QUADRANT_DIRECTIONS.iter().enumerate() {
                        grips.push(Grip {
                            id: id(GripKind::Quadrant, index as u8),
                            position: ellipse_quadrant_point(
                                *center,
                                *major_axis,
                                *ratio,
                                axes,
                                *direction,
                            ),
                        });
                    }
                }
            }
            grips
        }
        SemanticGeometry::Polyline { points, .. } => points
            .iter()
            .take(MAX_GRIP_INDEX + 1)
            .enumerate()
            .map(|(index, position)| Grip {
                // `index < MAX_GRIP_INDEX + 1`, so the `u8` conversion is
                // exact (see the module doc: vertices past 255 are not
                // addressable and are therefore not offered, never wrapped).
                id: id(GripKind::Vertex, index as u8),
                position: *position,
            })
            .collect(),
        SemanticGeometry::Spline {
            degree,
            knots,
            control_points,
            weights,
        } => {
            // A definition this model cannot interpret exposes no handle at all:
            // padding a knot/weight vector would silently rewrite the curve, and
            // `cad-db`'s validator is the authority on shape validity.
            if !spline_is_consistent(*degree, knots, control_points, weights) {
                return Vec::new();
            }
            control_points
                .iter()
                .take(MAX_GRIP_INDEX + 1)
                .enumerate()
                .map(|(index, position)| Grip {
                    // Same `u8` cap as the polyline vertices: control points past
                    // 255 are not addressable and are therefore not offered.
                    id: id(GripKind::ControlPoint, index as u8),
                    position: *position,
                })
                .collect()
        }
        SemanticGeometry::Point(position) => vec![Grip {
            id: id(GripKind::Point, 0),
            position: *position,
        }],
        _ => Vec::new(),
    }
}

/// The highest handle index a [`GripId`] can address: `GripId::index` is a `u8`.
///
/// Shared by every index-based handle family (polyline vertices, spline control
/// points) so the `0..=255` cap has exactly one value in this module.
const MAX_GRIP_INDEX: usize = u8::MAX as usize;

/// Whether a stored spline definition is internally consistent enough to expose
/// control-point handles for.
///
/// Two structural invariants, both pure length arithmetic on the stored vectors:
///
/// * `control_points.len() == weights.len()` — every control point carries its
///   own weight (a non-rational B-spline stores all `1.0`, but the vector is still
///   per control point);
/// * `knots.len() == control_points.len() + degree + 1` — a clamped B-spline of
///   degree `d` with `n + 1` control points has `n + d + 2` knots.
///
/// This is **not** shape validation: `cad-db`'s validator is the authority on what
/// may be stored, and this model only declines to invent a handle for a definition
/// it cannot index into. A mismatch is not repaired here (padding either vector
/// would change the curve behind the user's back); the spline simply exposes no
/// grip, and every control-point id against it is [`REASON_STALE_HANDLE`].
fn spline_is_consistent(
    degree: u32,
    knots: &[f64],
    control_points: &[Point3],
    weights: &[f64],
) -> bool {
    if control_points.len() != weights.len() {
        return false;
    }
    let expected_knots = control_points.len() + degree as usize + 1;
    knots.len() == expected_knots
}

/// Whether a stored ellipse parameter range covers the whole curve.
///
/// The slack is [`ARC_SWEEP_EPSILON`], the same one the tessellator and the
/// representation provider use to call a sweep "full", so the handle set and the
/// drawn curve agree about which ellipses are complete.
fn is_full_ellipse(sweep: f64) -> bool {
    sweep.abs() >= std::f64::consts::TAU - ARC_SWEEP_EPSILON
}

/// The ellipse's own plane frame `(unit major, unit minor)`.
///
/// `unit_minor = normalize(cross(unit(normal), unit(major_axis)))` is the
/// expression `cad_geometry`'s crate-private `ellipse_minor_dir` evaluates, i.e.
/// the frame the tessellator and the representation provider draw the ellipse
/// with. The axes are the **stored** ones; `arbitrary_axis` is only used for the
/// radially symmetric circle/arc, where every in-plane direction is equivalent.
///
/// `None` when no frame can be derived: a non-finite or zero-length major axis, a
/// non-finite normal, or a normal parallel to the major axis (`cad-db` validation
/// refuses to store such an ellipse, so this is the corrupt case). The caller then
/// offers no quadrant handle rather than a fabricated position.
fn ellipse_axes(normal: Point3, major_axis: Point3) -> Option<(Point3, Point3)> {
    let major_len = length(major_axis);
    if !major_len.is_finite() || major_len <= 0.0 {
        return None;
    }
    let unit_major = scale(major_axis, 1.0 / major_len);
    let minor = cross(normalize(normal), unit_major);
    let minor_len = length(minor);
    // `1e-9` is the orthogonality slack `ellipse_minor_dir` uses before its
    // fallback, and `cad-db` validation refuses a stored ellipse below it.
    if !minor_len.is_finite() || minor_len < 1e-9 {
        return None;
    }
    Some((unit_major, scale(minor, 1.0 / minor_len)))
}

/// One ellipse axis handle position: the tessellator's
/// `center + major_axis * cos t + minor * sin t` at `t = 0, π/2, π, 3π/2`, term
/// for term — `centre ± major_axis` for the major axis and
/// `centre ± (|major_axis| * |ratio|) * unit_minor` for the minor axis.
fn ellipse_quadrant_point(
    center: Point3,
    major_axis: Point3,
    ratio: f64,
    axes: (Point3, Point3),
    direction: (u8, f64),
) -> Point3 {
    let (which, sign) = direction;
    let vector = if which == 0 {
        major_axis
    } else {
        scale(axes.1, length(major_axis) * ratio.abs())
    };
    add(center, scale(vector, sign))
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

/// One point on the arc: `centre + radius * (cos(angle) * ax + sin(angle) * ay)`.
///
/// The expression is the tessellator's, term for term, so an arc endpoint grip
/// sits on exactly the vertex the display draws for the same stored `start` /
/// `sweep` (no second angle convention, no rounding).
fn arc_point(center: Point3, radius: f64, axes: (Point3, Point3), angle: f64) -> Point3 {
    add(
        center,
        add(
            scale(axes.0, angle.cos() * radius),
            scale(axes.1, angle.sin() * radius),
        ),
    )
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
