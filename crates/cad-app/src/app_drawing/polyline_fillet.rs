//! The AutoCAD **Polyline mode** of FILLET and CHAMFER: `FILLET POLYLINE` /
//! `CHAMFER POLYLINE` on one selected open LWPOLYLINE.
//!
//! ## Why a selector-based command
//!
//! AutoCAD reaches this mode through the `Polyline` sub-option of the FILLET /
//! CHAMFER object prompt. The fixed-step capture machine cannot express "pick the
//! first object **or** the Polyline option" (the option is not a world point), so
//! the mode is a separate selection-based command: the selector is the whole
//! input (`CommandPayload::None`), exactly like `JOIN`/`MATCHPROP`. The two-line
//! [`super::chamfer_fillet`] commands are untouched; `CHAMFER`/`FILLET` keep
//! their pick-pair capture.
//!
//! ## Supported shape (v1)
//!
//! One **open** polyline with **≥ 3** vertices whose **every segment is
//! straight** (`|bulge| <= tol`), lying in a plane parallel to world `XY` (`+Z`
//! planar, the convention this build's LWPOLYLINE data uses — the same predicate
//! [`cad_geometry::is_xy_parallel`] the tessellator/EXPLODE use to read a bulge in
//! the `+Z` frame). Every **interior** vertex is modified; the two end vertices
//! never move and no end cap is invented.
//!
//! ## The construction
//!
//! At an interior vertex with `u_in` the unit direction `previous → corner` and
//! `u_out` the unit direction `corner → next`, the **interior angle** between the
//! two arms leaving the corner is
//!
//! ```text
//! phi = acos((-u_in) · u_out)      (∈ (0, π])
//! ```
//!
//! which is exactly the `theta` of the two-line `plan_fillet` (its `dir1`/`dir2`
//! are the two arms). The corner vertex is replaced by two points:
//!
//! * **FILLET** — `d = radius / tan(phi/2)`, tangent points
//!   `t1 = corner - d·u_in` and `t2 = corner + d·u_out`; the joining segment
//!   `t1 → t2` is an **arc** of the session radius whose centre lies
//!   `radius / sin(phi/2)` from the corner along the inside bisector
//!   `normalize(-u_in + u_out)` — the same centre the two-line plan builds.
//! * **CHAMFER** — `t1 = corner - a·u_in` and `t2 = corner + b·u_out`; the joining
//!   segment `t1 → t2` is straight (`bulge = 0.0`).
//!
//! Our chamfer **orientation rule** (declared, not copied from AutoCAD): `A`
//! measures along the **incoming** arm (`previous → corner`), `B` along the
//! **outgoing** arm (`corner → next`). AutoCAD's prompt order for the polyline
//! sub-option is not reproduced here, so this rule is this build's contract and
//! is asserted by a test.
//!
//! ### The arc sign
//!
//! The sweep is derived from the **centre angles only**:
//! `start = atan2(t1 - centre)`, `end = atan2(t2 - centre)`,
//! `sweep = normalize_signed(end - start)` — the signed minor sweep in the `+Z`
//! plane, converted with [`cad_geometry::arc_sweep_to_bulge`] (the single
//! `sweep → bulge` authority, `bulge = tan(sweep/4)`). The sign is **never**
//! guessed from a cross product: the emitted bulge is fed back through
//! [`cad_geometry::bulge_arc_geometry`] and the recovered centre/radius/sweep must
//! equal the planned ones, or the whole plan is refused (`arc_mismatch`). A
//! 90° corner therefore yields the same centre, radius and sweep as
//! `plan_fillet` on the same two arms (the parity test pins this within 1e-12).
//!
//! ## Refusals (all named, all before any write)
//!
//! | code | meaning |
//! | --- | --- |
//! | `<c>.polyline_invalid_payload` | a payload was supplied (the selection is the input) |
//! | `<c>.polyline_selection` | the selection is not exactly one entity |
//! | `<c>.polyline_not_addressable` | the ref belongs to another document, or is a block-instance member / sub-element |
//! | `<c>.polyline_target_missing` | the referenced entity does not exist |
//! | `<c>.polyline_not_polyline` | the pick is not an LWPOLYLINE |
//! | `<c>.polyline_closed` | a closed polyline (no start/end; v1 is open only) |
//! | `<c>.polyline_too_few_vertices` | fewer than three vertices |
//! | `<c>.polyline_bulged_segment` | a segment is an arc (`\|bulge\| > tol`) |
//! | `<c>.polyline_not_planar` | the vertices are not `+Z` coplanar |
//! | `<c>.polyline_degenerate` | a non-finite or duplicated (zero-length) vertex pair |
//! | `<c>.polyline_collinear_vertex` | an interior vertex whose two arms are collinear (no corner) |
//! | `<c>.polyline_degenerate_join` | the joining segment of a corner has no length |
//! | `<c>.polyline_radius_not_positive` | FILLET radius `<= 0` (the two-line `R = 0` corner move has no polyline equivalent) |
//! | `<c>.polyline_distances_invalid` | CHAMFER `A < 0`, `B < 0`, or `A = B = 0` |
//! | `<c>.polyline_too_large` | a corner inset does not fit its segment(s) |
//! | `<c>.polyline_parameter_missing` | the session typed channel holds no finite number |
//! | `<c>.polyline_arc_mismatch` | the planned arc does not survive the bulge round-trip |
//!
//! `<c>` is `fillet` or `chamfer`. `polyline_*` refusals that report a
//! **capability gap** (not a polyline, closed, bulged, non-planar) are
//! [`CadError::Unsupported`]; a supported polyline whose numbers do not work is
//! [`CadError::InvalidInput`]. Viewer mode is refused by the command layer
//! ([`CommandId::requires_work_mode`]).
//!
//! ## Declared deviations from AutoCAD
//!
//! * **Closed polylines** are refused (AutoCAD rounds them too). v1 is open only,
//!   so there is no wrap-around corner to reason about.
//! * **Bulged (arc) segments** are refused: rounding a corner between an arc and a
//!   straight segment needs an arc/arc tangency construction this command does
//!   not implement. Nothing is straightened silently.
//! * **TRIMMODE** is not modelled: the trimmed source is always the polyline
//!   itself (there is nothing else to trim). No session parameter is read.
//! * **AutoCAD's radius auto-reduction** (it shrinks the radius to fit) is not
//!   reproduced: a radius/distances that do not fit is a named refusal.
//! * **Chamfer orientation** is our rule (`A` incoming, `B` outgoing), see above.
//! * The **two end segments** keep their full length and their own bulge; no end
//!   cap, extension or new entity is produced. The rewrite is one
//!   `update_entity` in one transaction, so the entity id, layer, space, draw
//!   order and style are preserved.

use super::helpers::{corner_tolerance, distance, normalize_signed};
use super::*;
use cad_geometry::{arc_sweep_to_bulge, bulge_arc_geometry, is_xy_parallel};

/// The `+Z` world normal every polyline bulge in this build is interpreted
/// against ([`cad_geometry::is_xy_parallel`] guarantees the input is `+Z` planar).
const POLYLINE_NORMAL: Point3 = Point3 {
    x: 0.0,
    y: 0.0,
    z: 1.0,
};

/// The corner operation a polyline-mode FILLET/CHAMFER applies at every interior
/// vertex.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum PolylineCornerSpec {
    /// FILLET with a **strictly positive** radius. The two-line `R = 0`
    /// "move both lines to the corner" form has no polyline equivalent (there is
    /// no corner to move to), so it is refused instead of being silently reused.
    Fillet { radius: f64 },
    /// CHAMFER with `a` measured along the **incoming** arm and `b` along the
    /// **outgoing** arm (our declared orientation rule).
    Chamfer { a: f64, b: f64 },
}

impl PolylineCornerSpec {
    /// The refusal-code stem (`fillet` / `chamfer`).
    fn stem(self) -> &'static str {
        match self {
            Self::Fillet { .. } => "fillet",
            Self::Chamfer { .. } => "chamfer",
        }
    }

    /// The user-facing command name used in diagnostic text.
    fn command_name(self) -> &'static str {
        match self {
            Self::Fillet { .. } => "FILLET POLYLINE",
            Self::Chamfer { .. } => "CHAMFER POLYLINE",
        }
    }

    /// The undo-record / transaction label.
    fn label(self) -> &'static str {
        match self {
            Self::Fillet { .. } => "fillet polyline",
            Self::Chamfer { .. } => "chamfer polyline",
        }
    }

    /// The diagnostic code of a successful edit.
    fn diagnostic_code(self) -> &'static str {
        match self {
            Self::Fillet { .. } => "drawing.fillet_polyline",
            Self::Chamfer { .. } => "drawing.chamfer_polyline",
        }
    }
}

/// A `<stem>.polyline_<code>: <detail>` refusal string (the planner's currency).
fn refusal(stem: &str, code: &str, detail: impl std::fmt::Display) -> String {
    format!("{stem}.polyline_{code}: {detail}")
}

/// One interior vertex's replacement: the two points, the bulge of the joining
/// segment, and how far each point sits from the corner (the overlap guard).
#[derive(Debug, Clone, Copy)]
struct Corner {
    t1: Point3,
    t2: Point3,
    /// Bulge of the segment `t1 → t2` (`0.0` for a chamfer).
    bulge: f64,
    /// Distance from the corner to `t1` along the incoming arm.
    inset_in: f64,
    /// Distance from the corner to `t2` along the outgoing arm.
    inset_out: f64,
}

/// Plan the polyline rewrite: replace every interior vertex of an **open**
/// straight-segment polyline with its fillet/chamfer corner.
///
/// Returns the new `(points, bulges)`; `bulges` keeps this build's one-bulge-per-
/// vertex convention (the trailing entry of an open polyline is unused, exactly
/// like `PLINE`'s own creation path). Untouched segments keep their stored bulge.
///
/// Every refusal is a `String` naming its code (see the module table) and the
/// function is **all-or-nothing**: a refusal means the caller writes nothing.
/// The plan is pure — no database, no session, no history.
pub(crate) fn plan_polyline_corners(
    points: &[Point3],
    bulges: &[f64],
    spec: PolylineCornerSpec,
    tol: f64,
) -> Result<(Vec<Point3>, Vec<f64>), String> {
    let stem = spec.stem();
    // 1. The spec's own numbers. The session channel validates the *shape*
    //    (a finite non-negative number); the *meaning* is per command, so the
    //    polyline mode's stricter rules live here.
    match spec {
        PolylineCornerSpec::Fillet { radius } => {
            if !radius.is_finite() || radius <= 0.0 {
                return Err(refusal(
                    stem,
                    "radius_not_positive",
                    format!(
                        "the polyline mode rounds every interior corner, so the FILLET radius must \
                         be finite and greater than zero, got {radius}; the two-line R=0 corner \
                         move has no polyline equivalent"
                    ),
                ));
            }
        }
        PolylineCornerSpec::Chamfer { a, b } => {
            if !a.is_finite() || !b.is_finite() || a < 0.0 || b < 0.0 || (a == 0.0 && b == 0.0) {
                return Err(refusal(
                    stem,
                    "distances_invalid",
                    format!(
                        "CHAMFER polyline distances must be finite and non-negative with at least \
                         one above zero, got A={a}, B={b}; the corner-only A=B=0 form has no \
                         polyline equivalent"
                    ),
                ));
            }
        }
    }
    // 2. Defensive shape guards. The command refuses these first and by name (as
    //    capability gaps), but a planner that silently straightened an arc
    //    segment, ignored a third dimension or indexed past a 2-vertex polyline
    //    would be exactly the silent wrong answer this build forbids.
    let count = points.len();
    if count < 3 {
        return Err(refusal(
            stem,
            "too_few_vertices",
            format!(
                "an open polyline needs at least three vertices to have an interior corner, got \
                 {count}"
            ),
        ));
    }
    if !points.iter().all(|p| is_finite_point(*p)) {
        return Err(refusal(
            stem,
            "degenerate",
            "the polyline has a non-finite vertex",
        ));
    }
    for (index, bulge) in bulges.iter().enumerate() {
        if !bulge.is_finite() || bulge.abs() > tol {
            return Err(refusal(
                stem,
                "bulged_segment",
                format!(
                    "LWPOLYLINE segment {index} is an arc (bulge {bulge}); only polylines whose \
                     every segment is straight can be modified"
                ),
            ));
        }
    }
    if !is_xy_parallel(points) {
        return Err(refusal(
            stem,
            "not_planar",
            "the polyline vertices are not coplanar in a plane parallel to world XY; only +Z \
             planar polylines can be modified",
        ));
    }

    // 3. Per interior vertex: the two replacement points and the joining bulge.
    let mut corners: Vec<Corner> = Vec::with_capacity(count - 2);
    for index in 1..count - 1 {
        let previous = points[index - 1];
        let corner = points[index];
        let next = points[index + 1];
        let u_in = unit_arm(previous, corner, tol, stem, index, "incoming")?;
        let u_out = unit_arm(corner, next, tol, stem, index, "outgoing")?;
        // Interior angle between the two arms leaving the corner. This is the
        // two-line `theta`: `dir1`/`dir2` there are `-u_in`/`u_out` here.
        let dot = (-u_in.0) * u_out.0 + (-u_in.1) * u_out.1;
        let phi = dot.clamp(-1.0, 1.0).acos();
        let half = phi * 0.5;
        let (sin_half, cos_half) = half.sin_cos();
        // Two collinear-apex shapes have no corner at all: the arms point the
        // same way (phi ≈ 0, a doubling-back spike) or the opposite way
        // (phi ≈ π, a straight-through vertex). Both are refused by name — the
        // first also guards `radius / sin(phi/2)`, which would be unbounded.
        if !sin_half.is_finite() || sin_half <= tol || (std::f64::consts::PI - phi) <= tol {
            return Err(refusal(
                stem,
                "collinear_vertex",
                format!(
                    "interior vertex {index} at ({:.6}, {:.6}) has (nearly) collinear arms \
                     (interior angle {phi} rad); there is no corner to modify",
                    corner.x, corner.y
                ),
            ));
        }
        match spec {
            PolylineCornerSpec::Fillet { radius } => {
                let tan_half = sin_half / cos_half;
                if !tan_half.is_finite() || tan_half <= tol {
                    return Err(refusal(
                        stem,
                        "collinear_vertex",
                        format!("interior vertex {index} has no usable corner angle"),
                    ));
                }
                let d = radius / tan_half;
                if !d.is_finite() || d <= tol {
                    // Unreachable after the collinear-apex guard above (a
                    // vanishing `d` needs `phi → π`), kept so the construction
                    // can never divide by a vanishing tangent.
                    return Err(refusal(
                        stem,
                        "degenerate_join",
                        format!("interior vertex {index} has a degenerate tangent distance"),
                    ));
                }
                let t1 = Point3 {
                    x: corner.x - d * u_in.0,
                    y: corner.y - d * u_in.1,
                    z: corner.z,
                };
                let t2 = Point3 {
                    x: corner.x + d * u_out.0,
                    y: corner.y + d * u_out.1,
                    z: corner.z,
                };
                // The inside bisector: the two-line centre direction.
                let bisector = (-u_in.0 + u_out.0, -u_in.1 + u_out.1);
                let bisector_length = bisector.0.hypot(bisector.1);
                if !bisector_length.is_finite() || bisector_length <= tol {
                    return Err(refusal(
                        stem,
                        "collinear_vertex",
                        format!("interior vertex {index} has no inside bisector"),
                    ));
                }
                let centre_distance = radius / sin_half;
                let centre = Point3 {
                    x: corner.x + bisector.0 / bisector_length * centre_distance,
                    y: corner.y + bisector.1 / bisector_length * centre_distance,
                    z: corner.z,
                };
                // The sweep comes from the centre angles only (never from a cross
                // product): the signed minor sweep in the +Z plane.
                let start = (t1.y - centre.y).atan2(t1.x - centre.x);
                let end = (t2.y - centre.y).atan2(t2.x - centre.x);
                let sweep = normalize_signed(end - start);
                if !sweep.is_finite() || sweep.abs() <= tol {
                    return Err(refusal(
                        stem,
                        "arc_mismatch",
                        format!("interior vertex {index} produced a degenerate arc sweep"),
                    ));
                }
                let bulge = arc_sweep_to_bulge(sweep).ok_or_else(|| {
                    refusal(
                        stem,
                        "arc_mismatch",
                        format!(
                            "interior vertex {index} produced a non-representable sweep {sweep}"
                        ),
                    )
                })?;
                require_recovered_arc(
                    stem,
                    index,
                    &ArcPlan {
                        t1,
                        t2,
                        centre,
                        radius,
                        sweep,
                        bulge,
                    },
                )?;
                corners.push(Corner {
                    t1,
                    t2,
                    bulge,
                    inset_in: d,
                    inset_out: d,
                });
            }
            PolylineCornerSpec::Chamfer { a, b } => {
                let t1 = Point3 {
                    x: corner.x - a * u_in.0,
                    y: corner.y - a * u_in.1,
                    z: corner.z,
                };
                let t2 = Point3 {
                    x: corner.x + b * u_out.0,
                    y: corner.y + b * u_out.1,
                    z: corner.z,
                };
                if distance(t1, t2) <= tol {
                    return Err(refusal(
                        stem,
                        "degenerate_join",
                        format!(
                            "interior vertex {index} at ({:.6}, {:.6}) produces a degenerate \
                             joining segment",
                            corner.x, corner.y
                        ),
                    ));
                }
                corners.push(Corner {
                    t1,
                    t2,
                    bulge: 0.0,
                    inset_in: a,
                    inset_out: b,
                });
            }
        }
    }

    // 4. No corner may eat into a neighbour. On every original segment the two
    //    insets it carries (the start vertex's outgoing inset and the end
    //    vertex's incoming inset) must sum to strictly less than its length minus
    //    `tol`. This is the task's "strictly less than the adjacent segment
    //    length minus tol" rule applied per segment, which is exactly the
    //    predicate that also stops two neighbouring corners from overlapping.
    for index in 0..count - 1 {
        let segment_length = distance(points[index], points[index + 1]);
        let start_inset = if index == 0 {
            0.0
        } else {
            corners[index - 1].inset_out
        };
        let end_inset = if index + 1 == count - 1 {
            0.0
        } else {
            corners[index].inset_in
        };
        let needed = start_inset + end_inset;
        if !segment_length.is_finite() || needed >= segment_length - tol {
            return Err(refusal(
                stem,
                "too_large",
                format!(
                    "polyline segment {index} is {segment_length} long but its corner insets need \
                     {needed}; the {} is too large for this polyline",
                    match spec {
                        PolylineCornerSpec::Fillet { .. } => "radius",
                        PolylineCornerSpec::Chamfer { .. } => "chamfer distance",
                    }
                ),
            ));
        }
    }

    // 5. Assemble: the vertex order is preserved, every interior vertex becomes
    //    `[t1, t2]`, and each untouched (sub-)segment keeps its stored bulge.
    let incoming = |index: usize| bulges.get(index).copied().unwrap_or(0.0);
    let mut out_points: Vec<Point3> = Vec::with_capacity(2 * count - 2);
    let mut out_bulges: Vec<f64> = Vec::with_capacity(2 * count - 2);
    out_points.push(points[0]);
    out_bulges.push(incoming(0));
    for (offset, corner_part) in corners.iter().enumerate() {
        out_points.push(corner_part.t1);
        out_bulges.push(corner_part.bulge);
        out_points.push(corner_part.t2);
        // The segment leaving `t2` is the rest of original segment `offset + 1`.
        out_bulges.push(incoming(offset + 1));
    }
    out_points.push(points[count - 1]);
    // The trailing entry of an open polyline's bulge list is unused (this
    // build's one-bulge-per-vertex convention: `PLINE` and OFFSET both do this).
    out_bulges.push(0.0);
    Ok((out_points, out_bulges))
}

/// The unit XY direction of one arm, or a degeneracy refusal.
fn unit_arm(
    from: Point3,
    to: Point3,
    tol: f64,
    stem: &str,
    vertex: usize,
    name: &str,
) -> Result<(f64, f64), String> {
    let dx = to.x - from.x;
    let dy = to.y - from.y;
    let length = dx.hypot(dy);
    if !length.is_finite() || length <= tol {
        return Err(refusal(
            stem,
            "degenerate",
            format!(
                "the {name} segment of interior vertex {vertex} is degenerate (vertices \
                 ({:.6}, {:.6}) and ({:.6}, {:.6}) coincide)",
                from.x, from.y, to.x, to.y
            ),
        ));
    }
    Ok((dx / length, dy / length))
}

/// The arc planned at one interior vertex, before its bulge is verified.
struct ArcPlan {
    t1: Point3,
    t2: Point3,
    centre: Point3,
    radius: f64,
    sweep: f64,
    bulge: f64,
}

/// Verify the emitted bulge against the single `bulge → arc` authority.
///
/// The sweep was derived from the centre angles; feeding the resulting bulge back
/// through [`cad_geometry::bulge_arc_geometry`] must recover the **planned**
/// centre, radius and sweep. A wrong sign would move the centre by `2R`, so this
/// guard catches exactly the failure mode `arc_sweep_to_bulge`'s inverse could
/// hide.
fn require_recovered_arc(stem: &str, vertex: usize, plan: &ArcPlan) -> Result<(), String> {
    let mismatch = || {
        refusal(
            stem,
            "arc_mismatch",
            format!(
                "the arc planned at interior vertex {vertex} does not survive the bulge \
                 round-trip (bulge {})",
                plan.bulge
            ),
        )
    };
    let recovered =
        bulge_arc_geometry(plan.t1, plan.t2, plan.bulge, POLYLINE_NORMAL).ok_or_else(mismatch)?;
    let SemanticGeometry::Arc {
        center,
        radius: recovered_radius,
        sweep: recovered_sweep,
        ..
    } = recovered
    else {
        return Err(mismatch());
    };
    let tolerance = 1e-9 * (1.0 + plan.radius);
    if distance(center, plan.centre) > tolerance
        || (recovered_radius - plan.radius).abs() > tolerance
        || (recovered_sweep - plan.sweep).abs() > tolerance
    {
        return Err(mismatch());
    }
    Ok(())
}

/// Test-only parity reference: the plan the **two-line** `plan_fillet` builds for
/// the same two arms.
///
/// The polyline mode replaces a corner between two straight segments, which is
/// exactly the two-line geometry, so its planner must produce the same centre,
/// radius, sweep and tangent points. This wrapper exists (instead of re-exporting
/// the private two-line planner) so the two-line implementation and its module
/// surface stay untouched, while the crate's test tree can still compare against
/// it. Returns `(first, second, inserted)` — the two retargeted lines and the
/// optional arc, exactly the two-line edit's fields.
#[cfg(test)]
pub(crate) type TwoLineFilletPlan = ((Point3, Point3), (Point3, Point3), Option<SemanticGeometry>);

#[cfg(test)]
pub(crate) fn two_line_fillet_plan(
    first: (Point3, Point3),
    pick_first: Point3,
    second: (Point3, Point3),
    pick_second: Point3,
    radius: f64,
    tol: f64,
) -> Option<TwoLineFilletPlan> {
    let input = super::helpers::TwoLineInput {
        first,
        pick_first,
        second,
        pick_second,
    };
    super::helpers::plan_fillet(&input, radius, tol)
        .ok()
        .map(|edit| (edit.first, edit.second, edit.inserted))
}

/// Test-only: the runtime corner tolerance the command hands the planner, so a
/// contract test compares against the real predicate instead of a copy.
#[cfg(test)]
pub(crate) fn polyline_corner_tolerance() -> f64 {
    corner_tolerance()
}

impl Application {
    /// `FILLET POLYLINE`: round every interior corner of the one selected open
    /// straight-segment polyline with the session `filletRadius`.
    ///
    /// The radius comes from the same typed channel the two-line `FILLET` reads
    /// ([`ToolParameter::FilletRadius`]); `R <= 0` is refused **by name** here
    /// because the two-line `R = 0` corner move has no polyline equivalent.
    pub(crate) fn fillet_polyline(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let spec = PolylineCornerSpec::Fillet {
            radius: polyline_parameter(session, ToolParameter::FilletRadius, "fillet")?,
        };
        self.polyline_corner_edit(session, command, spec)
    }

    /// `CHAMFER POLYLINE`: chamfer every interior corner of the one selected open
    /// straight-segment polyline with the session `chamferDistanceA`/`B`.
    ///
    /// `A` is measured along the **incoming** arm and `B` along the **outgoing**
    /// arm (our declared orientation rule); `A = B = 0` is refused because the
    /// two-line corner-only form has no polyline equivalent.
    pub(crate) fn chamfer_polyline(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let spec = PolylineCornerSpec::Chamfer {
            a: polyline_parameter(session, ToolParameter::ChamferDistanceA, "chamfer")?,
            b: polyline_parameter(session, ToolParameter::ChamferDistanceB, "chamfer")?,
        };
        self.polyline_corner_edit(session, command, spec)
    }

    /// The shared polyline corner edit: resolve the single selection, validate the
    /// polyline and plan the rewrite **before any write**, then commit exactly one
    /// `update_entity` in one transaction / one undo record.
    fn polyline_corner_edit(
        &mut self,
        session: &SessionState,
        command: &Command,
        spec: PolylineCornerSpec,
    ) -> CadResult<CommandOutcome> {
        let stem = spec.stem();
        let command_name = spec.command_name();
        if !matches!(command.payload, CommandPayload::None) {
            return Err(invalid(
                stem,
                "invalid_payload",
                format!("{command_name} takes no payload; the selected polyline is the input"),
            ));
        }
        let references = session.selection.refs();
        match references {
            [] => {
                return Err(invalid(
                    stem,
                    "selection",
                    format!("{command_name} modifies exactly one polyline, got none selected"),
                ));
            }
            [_] => {}
            many => {
                return Err(invalid(
                    stem,
                    "selection",
                    format!(
                        "{command_name} modifies exactly one polyline, got {} selected",
                        many.len()
                    ),
                ));
            }
        }
        let reference = &references[0];
        if reference.document != command.document {
            return Err(invalid(
                stem,
                "not_addressable",
                format!(
                    "the selected ref for entity {} belongs to another document",
                    reference.entity.0
                ),
            ));
        }
        if !reference.instance.0.is_empty() || reference.sub_element.is_some() {
            return Err(CadError::Unsupported(refusal(
                stem,
                "not_addressable",
                format!(
                    "{command_name} needs a model-space polyline with an addressable entity \
                         identity; entity {} is referenced through a block instance or a \
                         sub-element",
                    reference.entity.0
                ),
            )));
        }

        let document_id = command.document;
        let document = self
            .workspace
            .documents
            .get_mut(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        let mut candidate = document.drawing.as_ref().clone();
        let drawing = &mut candidate;
        let entity = drawing.entity(reference.entity).cloned().ok_or_else(|| {
            invalid(
                stem,
                "target_missing",
                format!(
                    "entity {} does not exist in this drawing",
                    reference.entity.0
                ),
            )
        })?;
        let (points, bulges, closed) = match &entity.geometry {
            SemanticGeometry::Polyline {
                points,
                bulges,
                closed,
            } => (points.clone(), bulges.clone(), *closed),
            other => {
                return Err(CadError::Unsupported(refusal(
                    stem,
                    "not_polyline",
                    format!(
                        "{command_name} needs an LWPOLYLINE pick; {} is not supported",
                        class_name(other)
                    ),
                )));
            }
        };
        let tol = corner_tolerance();
        validate_polyline(stem, command_name, &points, &bulges, closed, tol)?;
        let (out_points, out_bulges) =
            plan_polyline_corners(&points, &bulges, spec, tol).map_err(CadError::InvalidInput)?;
        let corners = points.len() - 2;
        let vertices = out_points.len();

        let entity_id = entity.id;
        let mut after = entity.clone();
        after.geometry = SemanticGeometry::Polyline {
            points: out_points,
            bulges: out_bulges,
            closed: false,
        };

        let label = spec.label();
        let transaction = next_transaction_id();
        let change_set = {
            let mut staged = drawing.begin_drawing_transaction(label, transaction)?;
            staged.update_entity(after.clone())?;
            staged.commit()?
        };
        // Read the committed entity back so the undo patch's `after` is the exact
        // stored value, never a re-derivation.
        let stored = drawing.entity(entity_id).cloned().ok_or_else(|| {
            CadError::Invariant(format!(
                "{command_name}: the polyline vanished after commit"
            ))
        })?;
        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: label.to_string(),
                patches: vec![drawing_patch(entity_id, Some(entity), Some(stored))],
                merge_key: None,
            },
        )?;

        let detail = match spec {
            PolylineCornerSpec::Fillet { radius } => {
                format!(
                    "R={radius} at {corners} interior corner(s) of entity {}",
                    entity_id.0
                )
            }
            PolylineCornerSpec::Chamfer { a, b } => format!(
                "A={a}, B={b} at {corners} interior corner(s) of entity {}",
                entity_id.0
            ),
        };
        Ok(CommandOutcome {
            objects: vec![ObjectId(entity_id.0)],
            changes: Some(Arc::new(change_set)),
            diagnostics: vec![Diagnostic {
                object: Some(ObjectId(entity_id.0)),
                code: spec.diagnostic_code().into(),
                // Locale-neutral technical diagnostic: English ASCII only.
                message: format!(
                    "{command_name}: {detail} (polyline rewritten in place, {vertices} vertices)"
                ),
            }],
            measurement: None,
            point_report: None,
        })
    }
}

/// The command-level shape requirements of the v1 polyline mode.
///
/// Capability gaps (closed ring, arc segment, 3D polyline) are
/// [`CadError::Unsupported`]; a supported shape whose vertices are degenerate is
/// [`CadError::InvalidInput`]. Nothing is written by either.
fn validate_polyline(
    stem: &str,
    command_name: &str,
    points: &[Point3],
    bulges: &[f64],
    closed: bool,
    tol: f64,
) -> CadResult<()> {
    if closed {
        return Err(CadError::Unsupported(refusal(
            stem,
            "closed",
            format!(
                "{command_name} rounds the interior corners of an open polyline; a closed polyline \
                 has no start or end vertex and is not supported in this version"
            ),
        )));
    }
    if points.len() < 3 {
        return Err(invalid(
            stem,
            "too_few_vertices",
            format!(
                "an open polyline needs at least three vertices to have an interior corner, got {}",
                points.len()
            ),
        ));
    }
    if !points.iter().all(|p| is_finite_point(*p)) {
        return Err(invalid(
            stem,
            "degenerate",
            "the polyline has a non-finite vertex",
        ));
    }
    for (index, bulge) in bulges.iter().enumerate() {
        if !bulge.is_finite() || bulge.abs() > tol {
            return Err(CadError::Unsupported(refusal(
                stem,
                "bulged_segment",
                format!(
                    "LWPOLYLINE segment {index} is an arc (bulge {bulge}); only polylines whose \
                     every segment is straight can be modified"
                ),
            )));
        }
    }
    if !is_xy_parallel(points) {
        return Err(CadError::Unsupported(refusal(
            stem,
            "not_planar",
            "the polyline vertices are not coplanar in a plane parallel to world XY; only +Z \
             planar polylines can be modified",
        )));
    }
    for index in 0..points.len() - 1 {
        if distance(points[index], points[index + 1]) <= tol {
            return Err(invalid(
                stem,
                "degenerate",
                format!(
                    "vertices {index} and {} coincide (a zero-length segment has no direction)",
                    index + 1
                ),
            ));
        }
    }
    Ok(())
}

/// Read one session tool parameter as a finite number, or refuse by name.
///
/// The typed channel stores a default for every parameter, so "missing" here means
/// "the channel holds no usable finite number" (`ToolValue::Flag` or a non-finite
/// value) — never a silently invented `0.0`.
fn polyline_parameter(
    session: &SessionState,
    parameter: ToolParameter,
    stem: &str,
) -> CadResult<f64> {
    match session.tool_parameter(parameter) {
        ToolValue::Number(value) if value.is_finite() => Ok(value),
        other => Err(invalid(
            stem,
            "parameter_missing",
            format!(
                "session parameter '{}' holds no finite number ({:?})",
                parameter.key(),
                other
            ),
        )),
    }
}

/// An `<stem>.polyline_<code>: <detail>` [`CadError::InvalidInput`].
fn invalid(stem: &str, code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::InvalidInput(refusal(stem, code, detail))
}
