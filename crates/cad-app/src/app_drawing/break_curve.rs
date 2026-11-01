//! `BREAK`: remove the span between two projected picks and keep the
//! complementary piece(s).
//!
//! AutoCAD's two-point `BREAK` takes an entity pick (which is also the first
//! break point) and a second break point. This build implements exactly that
//! shape for **LINE**, **ARC**, **CIRCLE** and **SPLINE** targets. The `F`
//! first-point override and multiple-break mode are deliberately **NOT
//! IMPLEMENTED**.
//!
//! The picks are projected onto the target's own parameter space with the same
//! [`TargetCurve`] abstraction TRIM/EXTEND use, so there is one parameter model
//! and one tolerance helper ([`cut_tolerance`]) rather than a second break-only
//! rule:
//!
//! * an open target (LINE/ARC/SPLINE) uses `u ∈ [0, 1]`; both picks must project
//!   inside that span (no clamping from another curve). The pieces `[0, u1]` and
//!   `[u2, 1]` are kept, each only when it is longer than the parameter tolerance;
//! * a periodic target (a full CIRCLE) normalises both parameters into `[0, 1)`
//!   and keeps the **one** complement starting at `u2` and wrapping through the
//!   seam. The stored entity therefore becomes an ARC.
//!
//! ## Refusals (all before the transaction)
//!
//! | code | meaning |
//! | --- | --- |
//! | `break.invalid_point` | `first`/`second` has a NaN/infinite coordinate |
//! | `break.not_addressable` | block-instance member, sub-element, or non-model-space target |
//! | `break.target_missing` | the target ref does not resolve (or is another document's) |
//! | `break.projection_failed` | a pick has no projection at all (degenerate target, pick on the ARC/CIRCLE axis) |
//! | `break.pick_outside_span` | a pick projects outside `[0, 1]` on an open target |
//! | `break.zero_span` | the two parameters are the same (nothing to remove) |
//! | `break.whole_curve` | the span covers the whole curve (the object would vanish) |
//! | `break.degenerate_piece` | `with_span` cannot keep a well-formed piece (e.g. too few SPLINE control points) |
//!
//! A LWPOLYLINE (or any other kind) is the explicit `Unsupported` from
//! [`TargetCurve::new`], whose `supported` name is `LINE, ARC, CIRCLE or SPLINE`.
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

use super::trim_curves::{cut_tolerance, TargetCurve};
use super::*;
use cad_db::DbObject;

/// A plan-level BREAK refusal: `InvalidInput` carrying its stable code.
fn refusal(code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::InvalidInput(format!("{code}: {detail}"))
}

impl Application {
    /// `BREAK` one LINE/ARC/CIRCLE/SPLINE between two picked points.
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

        // 3. Load the target and build the shared parameter model.
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
        let curve = TargetCurve::new(&before.geometry, "BREAK", "LINE, ARC, CIRCLE or SPLINE")?;

        // 4. Project both picks onto the target's own parameter space.
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

        // 5./6. Resolve the kept piece(s) as plain geometry before any write.
        let pieces = if curve.is_periodic() {
            periodic_pieces(&curve, u_a, u_b, param_tol)?
        } else {
            open_pieces(&curve, u_a, u_b, param_tol)?
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
            }
            staged.commit()?
        };
        // Display attributes live beside the entity row, so they are restored on
        // the candidate **after** the insert committed (the transaction cannot
        // stage display attributes for a row that does not exist yet).
        if let (Some(id), Some(display)) = (new_id, &new_display) {
            drawing.restore_entity_display_state(id, display);
        }

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

/// Resolve the kept piece(s) of an **open** target (LINE/ARC/SPLINE).
fn open_pieces(
    curve: &TargetCurve,
    u_a: f64,
    u_b: f64,
    param_tol: f64,
) -> CadResult<Vec<SemanticGeometry>> {
    for u in [u_a, u_b] {
        if !u.is_finite() || u < -param_tol || u > 1.0 + param_tol {
            return Err(refusal(
                "break.pick_outside_span",
                format!(
                    "a BREAK pick projects outside the target {} (parameter {u:.9}, the \
                     target spans [0, 1])",
                    curve.label()
                ),
            ));
        }
    }
    // Only a within-tolerance overshoot is folded onto the end; a pick from
    // another curve is refused above, never silently clamped.
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

/// Resolve the kept complement of a **periodic** target (a full CIRCLE).
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
