//! `BREAK`: remove the span between two projected picks and keep the
//! complementary piece(s).
//!
//! AutoCAD's two-point `BREAK` takes an entity pick (which is also the first
//! break point) and a second break point. This build supports **LINE**, **ARC**,
//! **CIRCLE**, **ELLIPSE** (full ellipse and elliptical arc), **SPLINE** and
//! **LWPOLYLINE** targets.
//! The **second** point may be off the object: it is projected to the nearest
//! point of the target curve before splitting (the first point is the entity
//! pick and must still project inside an open target's own span).
//!
//! The `F` first-point override is supported at the command layer: the payload
//! carries the resolved `target` ref separately from `first`, so `first` may be
//! any point on the target, not necessarily the entity-pick point. (The
//! interactive capture switches to that three-click form when the session
//! `breakFirstPoint` flag is set: entity pick, first break point, second break
//! point.)
//!
//! The picks are projected onto the target's own parameter space with the same
//! [`TargetCurve`] abstraction TRIM/EXTEND use, so there is one parameter model
//! and one tolerance helper ([`cut_tolerance`]) rather than a second break-only
//! rule:
//!
//! * an open analytic target (LINE/ARC/SPLINE) uses `u ∈ [0, 1]`; the first pick
//!   (the entity pick) must project inside that span, while the **second** pick
//!   may be off the object and is projected/clamped to the nearest point. The
//!   pieces `[0, u1]` and `[u2, 1]` are kept, each only when it is longer than
//!   the parameter tolerance;
//! * a periodic target (a full CIRCLE or a full ELLIPSE) normalises both
//!   parameters into `[0, 1)` and keeps the **one** complement starting at `u2`
//!   and wrapping through the seam. The stored entity therefore becomes an ARC
//!   or an elliptical arc;
//! * a **partial ELLIPSE** (an elliptical arc) is an open target with its own
//!   `u ∈ [0, 1]` along `start → start + sweep`; the two complementary
//!   elliptical arcs `[0, u1]` and `[u2, 1]` are kept. The **first** pick must
//!   project inside the arc's own sweep (`break.pick_outside_span`), the second
//!   is projected/clamped, exactly like an ARC;
//! * a LWPOLYLINE uses its own segment parameter `u ∈ [0, segments]` (each
//!   segment spans one unit). An **open** polyline keeps `[0, u1]` and
//!   `[u2, segments]`, preserving the bulge of every split segment. A **closed**
//!   polyline removes the forward span from the first pick to the second and
//!   becomes **one open polyline starting at the second break point** and
//!   wrapping through the seam back to the first pick; its bulges are preserved
//!   (and split) along that kept span.
//!
//! ## Refusals (all before the transaction)
//!
//! | code | meaning |
//! | --- | --- |
//! | `break.invalid_point` | `first`/`second` has a NaN/infinite coordinate |
//! | `break.not_addressable` | block-instance member, sub-element, or non-model-space target |
//! | `break.target_missing` | the target ref does not resolve (or is another document's) |
//! | `break.projection_failed` | a pick has no projection at all (degenerate target, pick on the ARC/CIRCLE axis) |
//! | `break.pick_outside_span` | the **first** pick projects outside `[0, 1]` on an open analytic target (the second is projected/clamped) |
//! | `break.zero_span` | the two parameters are the same (nothing to remove) |
//! | `break.whole_curve` | the span covers the whole curve (the object would vanish) |
//! | `break.degenerate_piece` | `with_span` cannot keep a well-formed piece (e.g. too few SPLINE control points) |
//! | `break.bulged_segment` | a non-planar polyline carries a bulge (no circular arc exists) |
//!
//! A refusal writes nothing: no revision bump, no `ChangeSet`, no undo entry.
//!
//! ## Transaction shape
//!
//! Everything is computed before the transaction is opened. On success the source
//! entity is **retained as the first piece** (same id, layer, space, draw order
//! and display sidecar; only its geometry and `type_key` change). When a second
//! piece exists it is inserted with a fresh id on the source's layer/space/draw
//! order and the source's display attributes. One transaction / one undo step;
//! undo restores the original geometry and removes the inserted piece.

use super::polyline_curve::PolylineCurve;
use super::trim_curves::{cut_tolerance, norm_tau, TargetCurve};
use super::*;
use cad_db::DbObject;

const TAU: f64 = std::f64::consts::TAU;

/// A plan-level BREAK refusal: `InvalidInput` carrying its stable code.
fn refusal(code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::InvalidInput(format!("{code}: {detail}"))
}

impl Application {
    /// `BREAK` one LINE/ARC/CIRCLE/SPLINE/LWPOLYLINE between two picked points.
    ///
    /// See the module docs for the supported subset, the named refusals and the
    /// one-transaction/one-undo-step shape. The source is retained as the first
    /// piece; the kept complement is inserted when it is non-degenerate.
    pub(crate) fn break_curve(&mut self, command: &Command) -> CadResult<CommandOutcome> {
        let CommandPayload::Break {
            target,
            first,
            second,
        } = &command.payload
        else {
            return Err(CadError::InvalidInput(
                "BREAK needs a Break { target, first, second } payload".into(),
            ));
        };
        // 1. Finite picks: refused before the drawing is read at all.
        if !is_finite_point(*first) || !is_finite_point(*second) {
            return Err(refusal(
                "break.invalid_point",
                "BREAK break points must be finite",
            ));
        }
        // 2. Directly addressable, model-space entities only.
        if !target.instance.0.is_empty() || target.sub_element.is_some() {
            return Err(refusal(
                "break.not_addressable",
                format!(
                    "BREAK target {} is a block-instance member or sub-element; only \
                     directly addressable model-space entities can be broken",
                    target.entity.0
                ),
            ));
        }
        if target.document != command.document {
            return Err(refusal(
                "break.target_missing",
                "BREAK target belongs to another document",
            ));
        }

        let document_id = command.document;
        let tol = cut_tolerance();
        let document = self
            .workspace
            .documents
            .get_mut(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;

        // 3. Load the target.
        let before = document
            .drawing
            .entity(target.entity)
            .cloned()
            .ok_or_else(|| {
                refusal(
                    "break.target_missing",
                    format!("BREAK target {} does not exist", target.entity.0),
                )
            })?;
        if before.space != SpaceId::Model {
            return Err(refusal(
                "break.not_addressable",
                format!(
                    "BREAK target {} is not a model-space entity; only model space is \
                     supported",
                    before.id.0
                ),
            ));
        }

        // 4./5./6. Resolve the kept piece(s) as plain geometry before any write.
        let pieces = match &before.geometry {
            SemanticGeometry::Polyline {
                points,
                bulges,
                closed,
                ..
            } => {
                let curve = PolylineCurve::new(points, bulges, *closed, "break")?;
                let Some(u_a) = curve.project_param(*first, tol)? else {
                    return Err(refusal(
                        "break.projection_failed",
                        "the first BREAK pick has no projection onto the polyline",
                    ));
                };
                let Some(u_b) = curve.project_param(*second, tol)? else {
                    return Err(refusal(
                        "break.projection_failed",
                        "the second BREAK pick has no projection onto the polyline",
                    ));
                };
                let total = curve.total_length();
                if !total.is_finite() || total <= tol {
                    return Err(refusal(
                        "break.projection_failed",
                        "the BREAK target polyline is degenerate (zero length)",
                    ));
                }
                let param_tol = (tol / total).max(1e-12);
                if curve.is_closed() {
                    closed_polyline_pieces(&curve, u_a, u_b, param_tol)?
                } else {
                    open_polyline_pieces(&curve, u_a, u_b, param_tol)?
                }
            }
            SemanticGeometry::Ellipse {
                center,
                normal,
                major_axis,
                ratio,
                start,
                sweep,
            } => ellipse_pieces(
                *center,
                *normal,
                *major_axis,
                *ratio,
                *start,
                *sweep,
                *first,
                *second,
                tol,
            )?,
            _ => {
                let curve =
                    TargetCurve::new(&before.geometry, "BREAK", "LINE, ARC, CIRCLE or SPLINE")?;
                let Some(u_a) = curve.project_param(*first, tol) else {
                    return Err(refusal(
                        "break.projection_failed",
                        format!(
                            "the first BREAK pick has no projection onto the target {}",
                            curve.label()
                        ),
                    ));
                };
                let Some(u_b) = curve.project_param(*second, tol) else {
                    return Err(refusal(
                        "break.projection_failed",
                        format!(
                            "the second BREAK pick has no projection onto the target {}",
                            curve.label()
                        ),
                    ));
                };
                let span_len = curve.length();
                if !span_len.is_finite() || span_len <= tol {
                    return Err(refusal(
                        "break.projection_failed",
                        format!(
                            "the BREAK target {} is degenerate (zero length)",
                            curve.label()
                        ),
                    ));
                }
                let param_tol = (tol / span_len).max(1e-12);
                if curve.is_periodic() {
                    periodic_pieces(&curve, u_a, u_b, param_tol)?
                } else {
                    open_pieces(&curve, u_a, u_b, param_tol)?
                }
            }
        };
        debug_assert!(!pieces.is_empty(), "BREAK always keeps at least one piece");

        // 7. One transaction: update the source into the first piece, insert the
        // second when present, then read both back for the undo patches.
        let source_id = before.id;
        let source_display = document.drawing.entity_display_state(source_id);
        let mut candidate = document.drawing.as_ref().clone();
        let drawing = &mut candidate;

        // Allocate the fresh id before the transaction borrows the drawing. The
        // id is only consumed when a second piece exists.
        let new_id = pieces.get(1).map(|_| drawing.allocate_entity_id());
        // The new piece copies the source's **display attributes** (colour,
        // linetype, lineweight, opacity); it is a fresh model-space entity, so it
        // carries no dynamic-visibility/block membership.
        let new_display = new_id.map(|_| cad_db::EntityDisplayState {
            attributes: source_display.attributes.clone(),
            visibility: Default::default(),
        });
        let transaction = next_transaction_id();
        let change_set = {
            let mut staged = drawing.begin_drawing_transaction("break curve", transaction)?;
            let first_piece = pieces
                .first()
                .expect("a successful BREAK keeps at least one piece");
            let mut after = before.clone();
            after.geometry = first_piece.clone();
            after.object.type_key = class_name(first_piece).to_string();
            staged.update_entity(after)?;
            if let (Some(new_id), Some(geometry)) = (new_id, pieces.get(1)) {
                let entity = DbEntity {
                    object: DbObject {
                        id: ObjectId(new_id.0),
                        type_key: class_name(geometry).to_string(),
                        revision: Revision(0),
                        source_handle: None,
                    },
                    id: new_id,
                    layer: before.layer,
                    space: before.space.clone(),
                    geometry: geometry.clone(),
                    draw_order: before.draw_order,
                };
                staged.insert_entity(entity)?;
                // Display attributes live beside the entity row; the shared write
                // path applies them after the insert, in this same revision.
                if let Some(display) = &new_display {
                    staged.update_entity_display_state(new_id, display)?;
                }
            }
            staged.commit()?
        };

        // Read the committed rows back: the patches are the exact stored values.
        let stored_first = drawing
            .entity(source_id)
            .cloned()
            .ok_or_else(|| CadError::Invariant("broken entity vanished after commit".into()))?;
        let stored_second = match new_id {
            Some(id) => Some(drawing.entity(id).cloned().ok_or_else(|| {
                CadError::Invariant("inserted BREAK piece vanished after commit".into())
            })?),
            None => None,
        };

        let mut patches = vec![drawing_patch(
            source_id,
            Some(before.clone()),
            Some(stored_first),
        )];
        if let (Some(id), Some(stored)) = (new_id, &stored_second) {
            let mut patch = drawing_patch(id, None, Some(stored.clone()));
            // Record the copied display attributes so a redo re-applies them,
            // exactly like the shared create funnel does for a captured display.
            patch.after_state = new_display.clone();
            patches.push(patch);
        }

        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: "break curve".into(),
                patches,
                block_patches: Vec::new(),
                layer_patches: Vec::new(),
                style_patches: Vec::new(),
                dim_style_patches: Vec::new(),
                mleader_style_patches: Vec::new(),
                layout_patches: Vec::new(),
                merge_key: None,
            },
        )?;

        let kinds: Vec<&str> = pieces.iter().map(class_name).collect();
        let mut objects = vec![ObjectId(source_id.0)];
        if let Some(id) = new_id {
            objects.push(ObjectId(id.0));
        }
        Ok(CommandOutcome {
            objects,
            changes: Some(Arc::new(change_set)),
            diagnostics: vec![Diagnostic {
                object: Some(ObjectId(source_id.0)),
                code: "drawing.break".into(),
                message: format!(
                    "BREAK: source {} kept as {} piece(s) ({})",
                    source_id.0,
                    pieces.len(),
                    kinds.join(", ")
                ),
            }],
            measurement: None,
            point_report: None,
        })
    }
}

/// Resolve the kept piece(s) of an **open** analytic target (LINE/ARC/SPLINE).
fn open_pieces(
    curve: &TargetCurve,
    u_a: f64,
    u_b: f64,
    param_tol: f64,
) -> CadResult<Vec<SemanticGeometry>> {
    if !u_a.is_finite() || u_a < -param_tol || u_a > 1.0 + param_tol {
        return Err(refusal(
            "break.pick_outside_span",
            format!(
                "the first BREAK pick projects outside the target {} (parameter {u_a:.9}, the \
                 target spans [0, 1])",
                curve.label()
            ),
        ));
    }
    if !u_b.is_finite() {
        return Err(refusal(
            "break.projection_failed",
            "the second BREAK pick projected to a non-finite parameter",
        ));
    }
    // Only a within-tolerance overshoot is folded onto the end; a pick from
    // another curve is still refused above, never silently clamped. The SECOND
    // pick may be off the object and is projected to the nearest point of the
    // curve, so its parameter is clamped onto the target's own span.
    let u1 = u_a.clamp(0.0, 1.0).min(u_b.clamp(0.0, 1.0));
    let u2 = u_a.clamp(0.0, 1.0).max(u_b.clamp(0.0, 1.0));
    if u2 - u1 <= param_tol {
        return Err(refusal(
            "break.zero_span",
            "the two BREAK picks project to the same point; there is no span to remove",
        ));
    }
    if u1 <= param_tol && u2 >= 1.0 - param_tol {
        return Err(refusal(
            "break.whole_curve",
            "the two BREAK picks span the whole target; BREAK does not delete the object \
             (use ERASE)",
        ));
    }

    let mut pieces = Vec::with_capacity(2);
    if u1 > param_tol {
        pieces.push(
            curve
                .with_span(0.0, u1)
                .map_err(|reason| refusal("break.degenerate_piece", reason))?,
        );
    }
    if u2 < 1.0 - param_tol {
        pieces.push(
            curve
                .with_span(u2, 1.0 - u2)
                .map_err(|reason| refusal("break.degenerate_piece", reason))?,
        );
    }
    Ok(pieces)
}

/// Resolve the kept complement of a **periodic** analytic target (a full CIRCLE).
///
/// Both projected parameters are wrapped into `[0, 1)`, ordered, and the removed
/// span is `[u1, u2]`; the kept piece starts at `u2` and wraps through the seam
/// with fraction `1 - (u2 - u1)`. The result is an ARC, which is what the caller
/// stores in the original CIRCLE entity.
fn periodic_pieces(
    curve: &TargetCurve,
    u_a: f64,
    u_b: f64,
    param_tol: f64,
) -> CadResult<Vec<SemanticGeometry>> {
    if !u_a.is_finite() || !u_b.is_finite() {
        return Err(refusal(
            "break.projection_failed",
            "a BREAK pick projected to a non-finite parameter",
        ));
    }
    // `rem_euclid` folds both the seam and any wrap (a negative-sweep full turn)
    // into the target's own single period.
    let u_a = u_a.rem_euclid(1.0);
    let u_b = u_b.rem_euclid(1.0);
    let (u1, u2) = if u_a <= u_b { (u_a, u_b) } else { (u_b, u_a) };
    let removed = u2 - u1;
    if removed <= param_tol {
        return Err(refusal(
            "break.zero_span",
            "the two BREAK picks project to the same point; there is no span to remove",
        ));
    }
    let fraction = 1.0 - removed;
    if fraction <= param_tol {
        return Err(refusal(
            "break.whole_curve",
            "the two BREAK picks span the whole target; BREAK does not delete the object \
             (use ERASE)",
        ));
    }
    let geometry = curve
        .with_span(u2, fraction)
        .map_err(|reason| refusal("break.degenerate_piece", reason))?;
    Ok(vec![geometry])
}

/// Resolve the kept piece(s) of an **open** LWPOLYLINE.
fn open_polyline_pieces(
    curve: &PolylineCurve,
    u_a: f64,
    u_b: f64,
    param_tol: f64,
) -> CadResult<Vec<SemanticGeometry>> {
    let n = curve.segment_count() as f64;
    if !u_a.is_finite() || u_a < -param_tol || u_a > n + param_tol {
        return Err(refusal(
            "break.pick_outside_span",
            format!(
                "the first BREAK pick projects outside the target polyline (parameter {u_a:.9}, \
                 the target spans [0, {n}])"
            ),
        ));
    }
    if !u_b.is_finite() {
        return Err(refusal(
            "break.projection_failed",
            "the second BREAK pick projected to a non-finite polyline parameter",
        ));
    }
    // The second pick may be off the polyline: it is projected to the nearest
    // point and clamped onto the polyline's own span.
    let u1 = u_a.clamp(0.0, n).min(u_b.clamp(0.0, n));
    let u2 = u_a.clamp(0.0, n).max(u_b.clamp(0.0, n));
    if u2 - u1 <= param_tol {
        return Err(refusal(
            "break.zero_span",
            "the two BREAK picks project to the same point; there is no span to remove",
        ));
    }
    if u1 <= param_tol && u2 >= n - param_tol {
        return Err(refusal(
            "break.whole_curve",
            "the two BREAK picks span the whole polyline; BREAK does not delete the object \
             (use ERASE)",
        ));
    }
    let mut pieces = Vec::with_capacity(2);
    if u1 > param_tol {
        pieces.push(curve.sub_curve(0.0, u1)?);
    }
    if u2 < n - param_tol {
        pieces.push(curve.sub_curve(u2, n)?);
    }
    Ok(pieces)
}

/// Resolve the kept complement of a **closed** LWPOLYLINE.
///
/// The removed span is the forward span from the first pick to the second; the
/// kept piece is the forward span from the second pick back to the first,
/// wrapping through the seam. The result is one **open** polyline whose first
/// vertex is the second break point.
fn closed_polyline_pieces(
    curve: &PolylineCurve,
    u_a: f64,
    u_b: f64,
    param_tol: f64,
) -> CadResult<Vec<SemanticGeometry>> {
    let n = curve.segment_count() as f64;
    if !u_a.is_finite() || !u_b.is_finite() {
        return Err(refusal(
            "break.projection_failed",
            "a BREAK pick projected to a non-finite polyline parameter",
        ));
    }
    let first = u_a.rem_euclid(n);
    let second = u_b.rem_euclid(n);
    let removed = (second - first).rem_euclid(n);
    if removed <= param_tol {
        return Err(refusal(
            "break.zero_span",
            "the two BREAK picks project to the same point; there is no span to remove",
        ));
    }
    if n - removed <= param_tol {
        return Err(refusal(
            "break.whole_curve",
            "the two BREAK picks span the whole polyline; BREAK does not delete the object \
             (use ERASE)",
        ));
    }
    let kept_end = if first > second { first } else { first + n };
    Ok(vec![curve.sub_curve(second, kept_end)?])
}

/// Resolve the kept piece(s) of an ELLIPSE or an elliptical arc.
///
/// AutoCAD's BREAK removes the forward (sweep-direction) span from the first
/// pick to the second and keeps the complement:
///
/// * a **full** ELLIPSE has no ends, so the complement is one elliptical arc in
///   the original entity (type key `AcDbEllipse`, `start = start + sweep·u₂`,
///   `sweep = sweep·(1 − (u₂ − u₁))`);
/// * a **partial ELLIPSE** (an elliptical arc) is an open target: the kept
///   pieces are `[0, u₁]` and `[u₂, 1]` along its own sweep, exactly the ARC
///   rule. The first pick must project inside the arc's own span; the second is
///   projected/clamped onto it.
#[allow(clippy::too_many_arguments)]
fn ellipse_pieces(
    center: Point3,
    normal: Point3,
    major_axis: Point3,
    ratio: f64,
    start: f64,
    sweep: f64,
    first: Point3,
    second: Point3,
    tol: f64,
) -> CadResult<Vec<SemanticGeometry>> {
    let (major, minor) = ellipse_axes(center, normal, major_axis, ratio, tol)?;
    if !sweep.is_finite() || sweep.abs() <= tol {
        return Err(refusal(
            "break.projection_failed",
            "the BREAK target ELLIPSE is degenerate (zero sweep)",
        ));
    }
    if sweep.abs() > TAU + 1e-9 {
        return Err(refusal(
            "break.projection_failed",
            format!(
                "the BREAK target ELLIPSE sweeps more than one full turn ({sweep}); its \
                 parameter has no single period"
            ),
        ));
    }
    // The angular pick tolerance is expressed in the target's own `u` units, so
    // it is the world tolerance over one unit of parameter (the arc length of the
    // full sweep at the major radius).
    let param_tol = (tol / (sweep.abs() * cad_geometry::length(major).max(tol))).max(1e-12);
    let angle_a = ellipse_pick_param(center, major, minor, first, tol).ok_or_else(|| {
        refusal(
            "break.projection_failed",
            "the first BREAK pick has no projection onto the ellipse",
        )
    })?;
    let angle_b = ellipse_pick_param(center, major, minor, second, tol).ok_or_else(|| {
        refusal(
            "break.projection_failed",
            "the second BREAK pick has no projection onto the ellipse",
        )
    })?;
    // Fold each raw eccentric angle onto the ellipse's own parameter period in
    // its sweep direction (the stored `start` and sweep sign matter).
    let normalised = |angle: f64| {
        let delta = if sweep >= 0.0 {
            norm_tau(angle - start)
        } else {
            norm_tau(start - angle)
        };
        delta / sweep.abs()
    };
    let u_a = normalised(angle_a);
    let u_b = normalised(angle_b);
    if sweep.abs() < TAU - 1e-9 {
        // A partial ELLIPSE is an open elliptical arc: the first pick must lie on
        // the arc's own sweep, the second is projected/clamped onto it.
        if !u_a.is_finite() || u_a < -param_tol || u_a > 1.0 + param_tol {
            return Err(refusal(
                "break.pick_outside_span",
                format!(
                    "the first BREAK pick projects outside the elliptical arc (parameter \
                     {u_a:.9}, the target spans [0, 1])"
                ),
            ));
        }
        if !u_b.is_finite() {
            return Err(refusal(
                "break.projection_failed",
                "the second BREAK pick projected to a non-finite parameter",
            ));
        }
        let u1 = u_a.clamp(0.0, 1.0).min(u_b.clamp(0.0, 1.0));
        let u2 = u_a.clamp(0.0, 1.0).max(u_b.clamp(0.0, 1.0));
        if u2 - u1 <= param_tol {
            return Err(refusal(
                "break.zero_span",
                "the two BREAK picks project to the same point; there is no span to remove",
            ));
        }
        if u1 <= param_tol && u2 >= 1.0 - param_tol {
            return Err(refusal(
                "break.whole_curve",
                "the two BREAK picks span the whole target; BREAK does not delete the object \
                 (use ERASE)",
            ));
        }
        let mut pieces = Vec::with_capacity(2);
        if u1 > param_tol {
            pieces.push(SemanticGeometry::Ellipse {
                center,
                normal,
                major_axis,
                ratio,
                start,
                sweep: sweep * u1,
            });
        }
        if u2 < 1.0 - param_tol {
            pieces.push(SemanticGeometry::Ellipse {
                center,
                normal,
                major_axis,
                ratio,
                start: start + sweep * u2,
                sweep: sweep * (1.0 - u2),
            });
        }
        return Ok(pieces);
    }
    // A full ELLIPSE is periodic: both parameters wrap into one period and the
    // kept piece is the complement wrapping through the seam.
    let u_a = u_a.rem_euclid(1.0);
    let u_b = u_b.rem_euclid(1.0);
    let (u1, u2) = if u_a <= u_b { (u_a, u_b) } else { (u_b, u_a) };
    let removed = u2 - u1;
    if removed <= param_tol {
        return Err(refusal(
            "break.zero_span",
            "the two BREAK picks project to the same point; there is no span to remove",
        ));
    }
    if 1.0 - removed <= param_tol {
        return Err(refusal(
            "break.whole_curve",
            "the two BREAK picks span the whole target; BREAK does not delete the object \
             (use ERASE)",
        ));
    }
    Ok(vec![SemanticGeometry::Ellipse {
        center,
        normal,
        major_axis,
        ratio,
        start: start + sweep * u2,
        sweep: sweep * (1.0 - removed),
    }])
}

/// The ellipse's major-axis and minor-axis vectors.
fn ellipse_axes(
    center: Point3,
    normal: Point3,
    major_axis: Point3,
    ratio: f64,
    tol: f64,
) -> CadResult<(Point3, Point3)> {
    let major_len = cad_geometry::length(major_axis);
    let normal_len = cad_geometry::length(normal);
    let ratio = ratio.abs();
    if !major_len.is_finite()
        || major_len <= tol
        || !normal_len.is_finite()
        || normal_len <= tol
        || !ratio.is_finite()
        || ratio <= tol
        || !is_finite_point(center)
    {
        return Err(refusal(
            "break.projection_failed",
            "the BREAK target ELLIPSE is degenerate",
        ));
    }
    let unit_major = cad_geometry::scale(major_axis, 1.0 / major_len);
    let cross = cad_geometry::cross(normal, unit_major);
    let cross_len = cad_geometry::length(cross);
    if !cross_len.is_finite() || cross_len <= tol {
        return Err(refusal(
            "break.projection_failed",
            "the BREAK target ELLIPSE has a degenerate minor axis",
        ));
    }
    let unit_minor = cad_geometry::scale(cross, 1.0 / cross_len);
    let minor = cad_geometry::scale(unit_minor, major_len * ratio);
    Ok((major_axis, minor))
}

/// The ellipse parameter (raw eccentric angle) nearest the picked point.
fn ellipse_pick_param(
    center: Point3,
    major: Point3,
    minor: Point3,
    pick: Point3,
    tol: f64,
) -> Option<f64> {
    let offset = cad_geometry::sub(pick, center);
    let major_len = cad_geometry::length(major);
    let minor_len = cad_geometry::length(minor);
    if major_len <= tol || minor_len <= tol {
        return None;
    }
    let qx = cad_geometry::dot(offset, major) / (major_len * major_len);
    let qy = cad_geometry::dot(offset, minor) / (minor_len * minor_len);
    if qx.hypot(qy) <= 1e-12 {
        return None;
    }
    Some(qy.atan2(qx))
}

// The `BREAKATPOINT` single-point split lives in its own file (code-size limit);
// it is a private helper module of this file, not a second command surface.
#[path = "break_at_point.rs"]
mod at_point;
