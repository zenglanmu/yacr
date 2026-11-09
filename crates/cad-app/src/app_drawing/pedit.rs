//! `PEDIT` (AutoCAD 2026): the typed polyline-option surface.
//!
//! AutoCAD's `PEDIT` picks one polyline and then shows an option prompt. This
//! build has **no in-command option prompt**, so the option is typed as part of
//! the command line and carried in [`crate::CommandPayload::PeditEdit`] (or the
//! legacy [`crate::CommandPayload::PeditOption`] for `CLOSE`/`OPEN`/`REVERSE`).
//!
//! ## Implemented options
//!
//! | option | input | write |
//! | --- | --- | --- |
//! | `CLOSE` / `OPEN` | one polyline | `closed` flag |
//! | `REVERSE` | one polyline (or the spline produced by `SPLINE`) | reversed points/bulges or control frame |
//! | `JOIN` | one open base polyline + touching LINE/ARC/open POLYLINE picks | appended segments; the joined entities are deleted |
//! | `FIT` | one open or closed polyline | tangent-continuous arc segments through every vertex |
//! | `SPLINE` | one open or closed polyline | the vertices as a clamped B-spline control frame |
//! | `DECURVE` | one polyline or a PEDIT-generated SPLINE | removes bulges / restores the (open or closed) control frame |
//! | `UNDO` | no selection needed | applies the document undo step |
//!
//! ## `FIT`
//!
//! One circular arc per original segment, chosen so the end tangent of segment
//! `i` equals the start tangent of segment `i + 1` (the exact `bulge` rule:
//! start tangent `= chord − δ`, end tangent `= chord + δ`, `bulge = tan(δ/2)`).
//!
//! **Open**: the chain has one free end condition; this build declares it as
//! **δ₀ = 0** (the first tangent follows the first chord), so the fit is
//! deterministic and the first segment is straight.
//!
//! **Closed**: the ring has no free end, so the closing segment's turn closes the
//! cyclic tangent system `δᵢ + δᵢ₊₁ = turnᵢ` (indices modulo `n`) and the fit is
//! a tangent-continuous loop back to the start. For an odd vertex count the
//! cyclic system has exactly one solution; for an even count it is solvable only
//! when the alternating turn residual vanishes (a square or any equi-turning
//! ring), and the remaining one-parameter family is fixed by **minimising the
//! largest segment turn** (the most even arc radii); a ring whose turns cannot
//! close is refused by name (`pedit.fit_closure`), never emitted as a broken
//! loop.
//!
//! In both cases a vertex that would need `|δ| ≥ π/2` (a corner at or beyond a
//! half-turn arc) is refused by name (`pedit.fit_tangent`) rather than emitted
//! as a wrapped bulge. No fit vertices are inserted: the original vertices are
//! exactly the arc endpoints, so `DECURVE` restores the frame by clearing the
//! bulges (and keeps a closed ring closed).
//!
//! ## `SPLINE`
//!
//! The vertices become the **control polygon** of a clamped uniform polynomial
//! B-spline of degree `min(3, n − 1)` with unit weights. The curve passes
//! through the first and last vertex and is pulled toward, but not necessarily
//! through, the interior ones (an approximating, not interpolating, spline).
//!
//! **Closed**: the control frame repeats the first vertex at the end
//! (`P₀ … Pₙ₋₁ P₀`, degree `min(3, n − 1)` for `n` ring vertices) so the curve
//! starts and ends at `P₀` with a closed control polygon. The SPLINE model has
//! no closed flag, so exactly this repeated control point is the marker: `DECURVE`
//! reads a unit-weight, clamped-uniform frame whose ends coincide as the closed
//! form and restores the closed ring (`n` bulges). An open frame that happens to
//! end where it starts is therefore read as the closed form — a declared
//! ambiguity of a model without a periodic flag.
//!
//! The stored entity is an exact `AcDbSpline` (the LWPOLYLINE model has no
//! spline-fit flag); the id, layer, space and draw order are kept and the type
//! key becomes `AcDbSpline`. `DECURVE` on such a spline restores its control
//! frame as a polyline with no bulges. Original bulges are not recoverable
//! (there is no fit-data storage), which is a declared divergence.
//!
//! `SPLINE <precision>` accepts AutoCAD's 0–99 prompt. The value is validated
//! and echoed in the diagnostic but is geometrically inert: the result is an
//! exact B-spline, not a tessellation.
//!
//! ## `JOIN`
//!
//! The **first** selected entity is the base polyline; the remaining picks are
//! welded at either end. Endpoints must **touch** (within
//! [`super::join::join_tolerance`]); this build has no fuzz-distance prompt, so
//! there is no gap bridging. LINEs and ARCs become polyline segments (an ARC
//! becomes one bulge, so it must lie in the +Z plane), open POLYLINEs are
//! appended/reversed as needed. Closed sources, a closed base, another layer or
//! another space are named refusals.
//!
//! ## `PEDITACCEPT` conversion
//!
//! `PeditEditCommand::convert` is the typed answer to AutoCAD's
//! "Object selected is not a polyline. Do you want it to turn into one?"
//! prompt. `true` converts a single LINE/ARC/SPLINE to a polyline before the
//! option runs (LINE exact, ARC exact in the +Z plane, SPLINE adaptively
//! tessellated); `false` keeps the `pedit.not_polyline` refusal. There is no
//! dialog and no new UI parameter: the answer is part of the typed payload, so
//! the command line / host adapter still needs wiring to set it.
//!
//! ## Named refusals (nothing is written)
//!
//! `pedit.needs_option`, `pedit.payload`, `pedit.selection`,
//! `pedit.join.selection`, `pedit.not_addressable`, `pedit.not_polyline`,
//! `pedit.too_few_vertices`, `pedit.fit_plane`, `pedit.fit_degenerate`,
//! `pedit.fit_tangent`, `pedit.fit_closure`, `pedit.spline_precision`,
//! `pedit.decurve_spline`,
//! `pedit.width_invalid`, `pedit.width_unsupported`,
//! `pedit.ltype_gen_unsupported`, `pedit.edit_vertex_unsupported`,
//! `pedit.undo_empty`, plus the `pedit.join.*` chain refusals. `WIDTH` and
//! `LTYPE GEN` are capability gaps (the entity model has no width or
//! linetype-generation field); `Edit vertex` has no sub-menu channel.
//!
//! Every real change is **one transaction / one undo step**. A named no-op
//! (`pedit.already_closed`, `pedit.already_open`, `pedit.already_decurved`)
//! opens no transaction and bumps no revision.

use super::join::PolylinePiece;
use super::*;
use crate::command_payload::{PeditEditCommand, PeditEditOption};
use crate::PeditOption;
use cad_geometry::clamped_uniform_knots;

// The pure curve planners (conversion, fit, spline-form recognition) live in
// their own file so this one stays inside the code-size limit; they are a
// private helper of THIS module, not a second option surface.
#[path = "pedit_curves.rs"]
mod curves;
use curves::{
    convert_to_polyline, fit_bulges, fit_bulges_closed, pedit_spline_restore, reverse_spline,
};

/// A `pedit.<code>: <detail>` refusal string (ASCII only).
fn refusal(code: &str, detail: impl std::fmt::Display) -> String {
    format!("pedit.{code}: {detail}")
}

/// A named, non-capability refusal (bad input for a supported shape).
fn invalid(code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::InvalidInput(refusal(code, detail))
}

/// A named capability-gap refusal (nothing is written).
fn unsupported(code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::Unsupported(refusal(code, detail))
}

/// The named no-op outcome: no transaction, no revision bump, no undo entry.
fn unchanged(code: &str, message: String) -> CommandOutcome {
    CommandOutcome {
        objects: Vec::new(),
        changes: None,
        diagnostics: vec![Diagnostic {
            object: None,
            code: code.into(),
            message,
        }],
        measurement: None,
        point_report: None,
    }
}

/// The one planned result of an option.
enum Plan {
    Noop {
        code: &'static str,
        message: String,
    },
    Write {
        geometry: SemanticGeometry,
        label: String,
        deleted: Vec<DbEntity>,
    },
}

fn v1_option(option: PeditOption) -> PeditEditOption {
    match option {
        PeditOption::Close => PeditEditOption::Close,
        PeditOption::Open => PeditEditOption::Open,
        PeditOption::Reverse => PeditEditOption::Reverse,
    }
}

/// Resolve the extra JOIN picks into chain pieces and deleted entities.
fn join_pieces(
    drawing: &DrawingDatabase,
    command: &Command,
    base: &DbEntity,
    refs: &[SelectionRef],
) -> CadResult<(PolylinePiece, Vec<DbEntity>)> {
    let mut pieces = Vec::new();
    let mut deleted = Vec::new();
    let mut seen = vec![base.id];
    for reference in refs.iter().skip(1) {
        if reference.document != command.document {
            return Err(invalid(
                "join.not_addressable",
                "a JOIN pick belongs to another document",
            ));
        }
        if !reference.instance.0.is_empty() || reference.sub_element.is_some() {
            return Err(unsupported(
                "join.not_addressable",
                format!(
                    "entity {} is not directly addressable (block instance or sub-element)",
                    reference.entity.0
                ),
            ));
        }
        if seen.contains(&reference.entity) {
            continue;
        }
        seen.push(reference.entity);
        let entity = drawing.entity(reference.entity).cloned().ok_or_else(|| {
            invalid(
                "join.not_addressable",
                format!("entity {} does not exist", reference.entity.0),
            )
        })?;
        if entity.space != SpaceId::Model {
            return Err(unsupported(
                "join.not_addressable",
                format!("entity {} is not in model space", entity.id.0),
            ));
        }
        if entity.layer != base.layer {
            return Err(invalid(
                "join.layer",
                format!(
                    "entity {} is on layer {} but the base polyline is on layer {}",
                    entity.id.0, entity.layer.0, base.layer.0
                ),
            ));
        }
        if entity.space != base.space {
            return Err(unsupported(
                "join.space",
                format!("entity {} is in another space", entity.id.0),
            ));
        }
        pieces.push(super::join::piece_from_geometry(
            &entity.geometry,
            "pedit.join",
        )?);
        deleted.push(entity);
    }
    if deleted.is_empty() {
        return Err(invalid(
            "join.selection",
            "PEDIT JOIN needs at least one additional touching selection",
        ));
    }
    let base_piece = super::join::piece_from_geometry(&base.geometry, "pedit.join")?;
    let mut all = Vec::with_capacity(pieces.len() + 1);
    all.push(base_piece);
    all.extend(pieces);
    let chain = super::join::chain_pieces(all, "pedit.join")?;
    Ok((chain, deleted))
}

impl Application {
    /// `PEDIT`: apply one typed polyline option (one transaction / one undo step
    /// for a real change).
    pub(crate) fn pedit(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let request = match &command.payload {
            CommandPayload::PeditOption(option) => PeditEditCommand {
                option: v1_option(*option),
                convert: false,
            },
            CommandPayload::PeditEdit(request) => *request,
            CommandPayload::None => {
                return Err(invalid(
                    "needs_option",
                    "PEDIT needs an option: CLOSE, OPEN, JOIN, WIDTH, FIT, SPLINE <0-99>, \
                     DECURVE, LTYPE GEN, REVERSE or UNDO (e.g. `PEDIT FIT`). `Edit vertex` \
                     has no sub-menu channel and is a named refusal",
                ));
            }
            _ => {
                return Err(invalid(
                    "payload",
                    "PEDIT takes a PeditEdit or PeditOption payload; the selection is the input",
                ));
            }
        };

        // `UNDO` is the document undo step; it needs no selection and writes no
        // transaction of its own.
        if matches!(request.option, PeditEditOption::Undo) {
            return self.pedit_undo(command);
        }

        let references = session.selection.refs().to_vec();
        let joining = matches!(request.option, PeditEditOption::Join);
        if joining {
            if references.len() < 2 {
                return Err(invalid(
                    "join.selection",
                    format!(
                        "PEDIT JOIN needs the base polyline plus at least one touching pick, \
                         got {}",
                        references.len()
                    ),
                ));
            }
        } else if references.len() != 1 {
            return Err(invalid(
                "selection",
                format!(
                    "PEDIT edits exactly one polyline, got {} selected (the interactive \
                     Select polyline / Multiple prompt is not implemented)",
                    references.len()
                ),
            ));
        }
        let reference = &references[0];
        if reference.document != command.document {
            return Err(invalid(
                "not_addressable",
                format!(
                    "the selected ref for entity {} belongs to another document",
                    reference.entity.0
                ),
            ));
        }
        if !reference.instance.0.is_empty() || reference.sub_element.is_some() {
            return Err(unsupported(
                "not_addressable",
                format!(
                    "PEDIT needs a directly addressable model-space polyline; entity {} is \
                     referenced through a block instance or a sub-element",
                    reference.entity.0
                ),
            ));
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
                "not_addressable",
                format!(
                    "entity {} does not exist in this drawing",
                    reference.entity.0
                ),
            )
        })?;
        if entity.space != SpaceId::Model {
            return Err(unsupported(
                "not_addressable",
                format!(
                    "PEDIT edits model-space polylines only; entity {} is in another space",
                    reference.entity.0
                ),
            ));
        }

        // The `PEDITACCEPT` conversion answer applies to the base entity only;
        // a JOIN pick is converted by the piece builder (LINE/ARC are segments).
        let mut geometry = entity.geometry.clone();
        if !matches!(geometry, SemanticGeometry::Polyline { .. }) {
            let decurve_spline = matches!(request.option, PeditEditOption::Decurve)
                && matches!(geometry, SemanticGeometry::Spline { .. });
            let reverse_spline = matches!(request.option, PeditEditOption::Reverse)
                && matches!(geometry, SemanticGeometry::Spline { .. });
            if request.convert {
                geometry = convert_to_polyline(&geometry)?;
            } else if !decurve_spline && !reverse_spline {
                return Err(unsupported(
                    "not_polyline",
                    format!(
                        "PEDIT needs a POLYLINE selection; {} is not supported (set \
                         `PEDITACCEPT` / pass `convert = true` to turn it into one)",
                        class_name(&geometry)
                    ),
                ));
            }
        }

        // JOIN reads the base geometry through the same `PEDITACCEPT` conversion
        // as every other option (a converted LINE base is a valid polyline base).
        let join_base = DbEntity {
            geometry: geometry.clone(),
            ..entity.clone()
        };
        let plan = match request.option {
            PeditEditOption::Close | PeditEditOption::Open => plan_close_open(
                geometry,
                matches!(request.option, PeditEditOption::Close),
                entity.id,
            ),
            PeditEditOption::Reverse => plan_reverse(geometry, entity.id),
            PeditEditOption::Join => plan_join(drawing, command, &join_base, &references),
            PeditEditOption::Fit => plan_fit(geometry),
            PeditEditOption::Spline { precision } => plan_spline(geometry, precision),
            PeditEditOption::Decurve => plan_decurve(geometry),
            PeditEditOption::Width(width) => {
                if !width.is_finite() || width < 0.0 {
                    return Err(invalid(
                        "width_invalid",
                        "PEDIT WIDTH needs a finite non-negative width",
                    ));
                }
                return Err(unsupported(
                    "width_unsupported",
                    "this build has no per-polyline (or per-segment) width field; PEDIT WIDTH \
                     is a named capability gap",
                ));
            }
            PeditEditOption::LtypeGen { .. } => {
                return Err(unsupported(
                    "ltype_gen_unsupported",
                    "this build's entity model has no linetype-generation (`plinegen`) flag",
                ));
            }
            PeditEditOption::EditVertex => {
                return Err(unsupported(
                    "edit_vertex_unsupported",
                    "PEDIT Edit vertex needs an interactive Next/Previous/Break sub-menu; this \
                     build has no in-command sub-menu channel",
                ));
            }
            PeditEditOption::Undo => unreachable!("UNDO is handled before the drawing borrow"),
        }?;

        let (new_geometry, label, deleted) = match plan {
            Plan::Noop { code, message } => return Ok(unchanged(code, message)),
            Plan::Write {
                geometry,
                label,
                deleted,
            } => (geometry, label, deleted),
        };

        let entity_id = entity.id;
        let mut after = entity.clone();
        after.geometry = new_geometry;
        after.object.type_key = class_name(&after.geometry).to_string();
        let display_states: Vec<_> = deleted
            .iter()
            .map(|extra| drawing.entity_display_state(extra.id))
            .collect();
        let transaction = next_transaction_id();
        let change_set = {
            let mut staged = drawing.begin_drawing_transaction("pedit", transaction)?;
            staged.update_entity(after)?;
            for extra in &deleted {
                staged.delete_entity(extra.id)?;
            }
            staged.commit()?
        };
        // Read the committed entity back so the undo patch's `after` is the exact
        // stored value, never a re-derivation.
        let stored = drawing.entity(entity_id).cloned().ok_or_else(|| {
            CadError::Invariant("PEDIT: the polyline vanished after commit".into())
        })?;
        let mut patches = vec![drawing_patch(entity_id, Some(entity), Some(stored))];
        for (extra, state) in deleted.into_iter().zip(display_states) {
            patches.push(drawing_patch_deleted(extra.id, extra, state));
        }
        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: "pedit".to_string(),
                patches,
                block_patches: Vec::new(),
                merge_key: None,
            },
        )?;

        Ok(CommandOutcome {
            objects: vec![ObjectId(entity_id.0)],
            changes: Some(Arc::new(change_set)),
            diagnostics: vec![Diagnostic {
                object: Some(ObjectId(entity_id.0)),
                code: "pedit.applied".into(),
                // Locale-neutral technical diagnostic: English ASCII only.
                message: format!(
                    "PEDIT {}: updated entity {} in one transaction",
                    label, entity_id.0
                ),
            }],
            measurement: None,
            point_report: None,
        })
    }

    /// `PEDIT UNDO`: apply the document undo step.
    fn pedit_undo(&mut self, command: &Command) -> CadResult<CommandOutcome> {
        let can_undo = self
            .history
            .get(&command.document)
            .map(|history| history.can_undo())
            .unwrap_or(false);
        if !can_undo {
            return Err(invalid(
                "undo_empty",
                "PEDIT UNDO: the document has no undo step to apply",
            ));
        }
        let mut outcome = self.undo(command)?;
        outcome.diagnostics.push(Diagnostic {
            object: None,
            code: "pedit.undo".into(),
            message: "PEDIT UNDO: applied the document undo step; this build has no in-command \
                      PEDIT option stack"
                .into(),
        });
        Ok(outcome)
    }
}

fn plan_close_open(geometry: SemanticGeometry, close: bool, id: EntityId) -> CadResult<Plan> {
    let SemanticGeometry::Polyline {
        points,
        bulges,
        closed,
    } = geometry
    else {
        return Err(unsupported("not_polyline", "CLOSE/OPEN needs a POLYLINE"));
    };
    if close {
        if closed {
            return Ok(Plan::Noop {
                code: "pedit.already_closed",
                message: format!(
                    "PEDIT CLOSE: entity {} is already closed; nothing was written",
                    id.0
                ),
            });
        }
        if points.len() < 3 {
            return Err(invalid(
                "too_few_vertices",
                format!(
                    "PEDIT CLOSE needs at least three vertices to form a closed ring, got {}",
                    points.len()
                ),
            ));
        }
        Ok(Plan::Write {
            geometry: SemanticGeometry::Polyline {
                points,
                bulges,
                closed: true,
            },
            label: "CLOSE (closed = true)".into(),
            deleted: Vec::new(),
        })
    } else {
        if !closed {
            return Ok(Plan::Noop {
                code: "pedit.already_open",
                message: format!(
                    "PEDIT OPEN: entity {} is already open; nothing was written",
                    id.0
                ),
            });
        }
        Ok(Plan::Write {
            geometry: SemanticGeometry::Polyline {
                points,
                bulges,
                closed: false,
            },
            label: "OPEN (closed = false)".into(),
            deleted: Vec::new(),
        })
    }
}

fn plan_reverse(geometry: SemanticGeometry, _id: EntityId) -> CadResult<Plan> {
    if let SemanticGeometry::Spline {
        degree,
        knots,
        control_points,
        weights,
    } = &geometry
    {
        return Ok(Plan::Write {
            geometry: reverse_spline(*degree, knots, control_points, weights),
            label: "REVERSE (SPLINE control frame)".into(),
            deleted: Vec::new(),
        });
    }
    let reversed =
        super::modify_extra::reverse_geometry(&geometry).map_err(|error| match error {
            CadError::InvalidInput(message) => CadError::InvalidInput(refusal("reverse", message)),
            other => other,
        })?;
    Ok(Plan::Write {
        geometry: reversed,
        label: "REVERSE (vertex order and bulges reversed)".into(),
        deleted: Vec::new(),
    })
}

fn plan_join(
    drawing: &DrawingDatabase,
    command: &Command,
    base: &DbEntity,
    refs: &[SelectionRef],
) -> CadResult<Plan> {
    let SemanticGeometry::Polyline { closed, .. } = &base.geometry else {
        return Err(unsupported(
            "not_polyline",
            "PEDIT JOIN needs the base selection to be a POLYLINE",
        ));
    };
    if *closed {
        return Err(invalid(
            "join.closed",
            "PEDIT JOIN needs an open base polyline; a closed polyline has no free end",
        ));
    }
    let (chain, deleted) = join_pieces(drawing, command, base, refs)?;
    Ok(Plan::Write {
        geometry: SemanticGeometry::Polyline {
            points: chain.points,
            bulges: chain.bulges,
            closed: false,
        },
        label: format!("JOIN ({} objects welded)", deleted.len()),
        deleted,
    })
}

fn plan_fit(geometry: SemanticGeometry) -> CadResult<Plan> {
    let SemanticGeometry::Polyline {
        points,
        closed,
        bulges,
    } = geometry
    else {
        return Err(unsupported("not_polyline", "FIT needs a POLYLINE"));
    };
    let fitted = if closed {
        fit_bulges_closed(&points)?
    } else {
        fit_bulges(&points)?
    };
    // A fit whose bulges are all zero is the already-straight frame.
    if fitted.iter().all(|bulge| bulge.abs() <= 1e-12)
        && bulges.iter().all(|bulge| bulge.abs() <= 1e-12)
    {
        return Ok(Plan::Noop {
            code: "pedit.already_fit",
            message: "PEDIT FIT: the polyline is already a straight frame; nothing was written"
                .into(),
        });
    }
    Ok(Plan::Write {
        geometry: SemanticGeometry::Polyline {
            points,
            bulges: fitted,
            closed,
        },
        label: if closed {
            "FIT (closed tangent-continuous arc ring)".into()
        } else {
            "FIT (tangent-continuous arc chain)".into()
        },
        deleted: Vec::new(),
    })
}

fn plan_spline(geometry: SemanticGeometry, precision: u8) -> CadResult<Plan> {
    if precision > 99 {
        return Err(invalid(
            "spline_precision",
            format!("PEDIT SPLINE precision must be 0..=99, got {precision}"),
        ));
    }
    let SemanticGeometry::Polyline {
        points,
        closed,
        bulges: _,
    } = geometry
    else {
        return Err(unsupported("not_polyline", "SPLINE needs a POLYLINE"));
    };
    let minimum = if closed { 3 } else { 2 };
    if points.len() < minimum {
        return Err(invalid(
            "too_few_vertices",
            format!(
                "PEDIT SPLINE needs at least {minimum} vertices on a{} polyline, got {}",
                if closed { " closed" } else { "n open" },
                points.len()
            ),
        ));
    }
    // The degree is fixed by the ring's own vertex count; the closed frame then
    // repeats the first vertex so the control polygon closes on itself.
    let degree = 3.min(points.len() - 1) as u32;
    let mut control_points = points;
    if closed {
        let first = control_points[0];
        control_points.push(first);
    }
    let n = control_points.len();
    Ok(Plan::Write {
        geometry: SemanticGeometry::Spline {
            degree,
            knots: clamped_uniform_knots(n, degree as usize),
            control_points,
            weights: vec![1.0; n],
        },
        label: if closed {
            format!("SPLINE (closed control frame, precision {precision})")
        } else {
            format!("SPLINE (control frame, precision {precision})")
        },
        deleted: Vec::new(),
    })
}

fn plan_decurve(geometry: SemanticGeometry) -> CadResult<Plan> {
    match geometry {
        SemanticGeometry::Polyline {
            points,
            mut bulges,
            closed,
        } => {
            if bulges.iter().all(|bulge| bulge.abs() <= 1e-12) {
                return Ok(Plan::Noop {
                    code: "pedit.already_decurved",
                    message: "PEDIT DECURVE: the polyline has no fit/spline data; nothing was \
                              written"
                        .into(),
                });
            }
            bulges.fill(0.0);
            Ok(Plan::Write {
                geometry: SemanticGeometry::Polyline {
                    points,
                    bulges,
                    closed,
                },
                label: "DECURVE (bulges cleared, frame restored)".into(),
                deleted: Vec::new(),
            })
        }
        SemanticGeometry::Spline {
            degree,
            knots,
            control_points,
            weights,
        } => {
            let Some((points, bulges, closed)) =
                pedit_spline_restore(&degree, &knots, &control_points, &weights)
            else {
                return Err(unsupported(
                    "decurve_spline",
                    "PEDIT DECURVE only restores a SPLINE written by `PEDIT SPLINE` (clamped \
                     uniform degree min(3, n-1), unit weights, closed frames repeated at the \
                     start vertex); this spline is not that form",
                ));
            };
            Ok(Plan::Write {
                geometry: SemanticGeometry::Polyline {
                    points,
                    bulges,
                    closed,
                },
                label: "DECURVE (SPLINE control frame restored)".into(),
                deleted: Vec::new(),
            })
        }
        other => Err(unsupported(
            "not_polyline",
            format!(
                "DECURVE needs a POLYLINE or a PEDIT SPLINE, got {}",
                class_name(&other)
            ),
        )),
    }
}
