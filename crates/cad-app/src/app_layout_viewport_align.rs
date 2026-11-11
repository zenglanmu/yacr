//! `ALIGNSPACE` and `VPSYNC`: real layout-viewport alignment writes.
//!
//! Both commands were Wave 13 named refusals; they are now real
//! `set_layout_viewports` writes (one layout patch, one revision, one undo
//! step) because [`cad_db::PaperViewport::model_to_paper`] is already the stored
//! paper→model matrix: a uniform scale plus a translation is exactly what the
//! stored `[[mpp, 0, tx], [0, mpp, ty]]` diagonal encodes, and
//! [`crate::app_layout`]'s `VPSCALE` already writes it that way.
//!
//! # ALIGNSPACE
//!
//! Input is a [`CommandPayload::Points`] payload of **exactly four**
//! coordinates, read as two alignment points:
//!
//! ```text
//! [model_a, paper_a, model_b, paper_b]
//! ```
//!
//! The model point `model_a` is placed on the paper point `paper_a`, and the
//! pair distance ratio `|paper_b - paper_a| / |model_b - model_a|` is the
//! uniform paper-per-model scale. That is AutoCAD's pan+zoom alignment: a plan
//! viewport cannot rotate, so the second pair only fixes the scale. When the
//! two pair directions are not parallel the rotation the caller asked for
//! cannot be honoured; the pan+zoom part is still applied and the outcome says
//! `alignspace.rotation_ignored` (with the angle) instead of pretending the
//! second point landed exactly.
//!
//! Refusals: `alignspace.paper_space_required`, `alignspace.points_required`
//! (anything other than four coordinates), `alignspace.degenerate_alignment`
//! (non-finite points or a zero-length pair) and `alignspace.no_viewport`
//! (the active layout stores no viewport). This build has no viewport
//! selection, so the **first** viewport is aligned.
//!
//! # VPSYNC
//!
//! Copies the stored alignment (scale + translation) of the **master**
//! viewport — viewport 0, again because there is no viewport selection — onto
//! every other viewport of the active layout in one transaction. Refusals:
//! `vpsync.paper_space_required`, `vpsync.insufficient_viewports` (fewer than
//! two viewports, so there is nothing to synchronize), `vpsync.invalid_master`
//! (non-finite or non-uniform master scale) and the usual payload refusal. An
//! already-synchronized layout is a named no-op (`vpsync.unchanged`) with no
//! revision and no undo entry.
//!
//! # Mode
//!
//! Both commands write the drawing, so they need Work mode. The shared
//! capability registry (`command_id::extended::selectors`, outside this
//! module's file scope) still lists them as Viewer-legal, so each handler
//! enforces the gate itself and refuses with
//! `alignspace.work_mode_required` / `vpsync.work_mode_required` in Viewer
//! mode. A Viewer session therefore never writes.

use super::*;

use crate::app_drawing::{layout_row, next_transaction_id};
use crate::app_layout::{layout_viewports, require_paper_space};
use crate::app_utility::require_none_payload;

/// A plan-view alignment cannot rotate, so anything above this cross-product
/// magnitude (of two unit direction vectors) is a rotation the caller asked
/// for and this command cannot apply.
const PARALLEL_TOLERANCE: f64 = 1e-9;

/// Refuse a drawing write outside Work mode, with a stable reason.
fn require_work_mode(session: &SessionState, prefix: &str, name: &str) -> CadResult<()> {
    if session.app_mode() == AppMode::Work {
        return Ok(());
    }
    Err(CadError::Unsupported(format!(
        "{prefix}.work_mode_required: {name} writes the layout viewport alignment, so it needs \
         Work mode (the session is read-only)"
    )))
}

impl Application {
    /// Handle `ALIGNSPACE`: two-point pan+zoom alignment of the first viewport.
    pub(crate) fn align_space(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        require_work_mode(session, "alignspace", "ALIGNSPACE")?;
        let layout_id = require_paper_space(session, "alignspace")?;
        let CommandPayload::Points(points) = &command.payload else {
            return Err(CadError::InvalidInput(
                "alignspace.points_required: ALIGNSPACE needs two alignment point pairs, i.e. \
                 four coordinates: model_a paper_a model_b paper_b"
                    .into(),
            ));
        };
        let [model_a, paper_a, model_b, paper_b] = points.as_slice() else {
            return Err(CadError::InvalidInput(format!(
                "alignspace.points_required: ALIGNSPACE needs two alignment point pairs (four \
                 coordinates: model_a paper_a model_b paper_b); got {} point(s), and fewer than \
                 two alignment points cannot define an alignment",
                points.len()
            )));
        };
        for point in [model_a, paper_a, model_b, paper_b] {
            if !point.x.is_finite() || !point.y.is_finite() || !point.z.is_finite() {
                return Err(CadError::InvalidInput(
                    "alignspace.degenerate_alignment: an alignment point is not finite".into(),
                ));
            }
        }
        let model_span = ((model_b.x - model_a.x).powi(2) + (model_b.y - model_a.y).powi(2)).sqrt();
        let paper_span = ((paper_b.x - paper_a.x).powi(2) + (paper_b.y - paper_a.y).powi(2)).sqrt();
        if model_span <= 1e-12 || paper_span <= 1e-12 {
            return Err(CadError::InvalidInput(format!(
                "alignspace.degenerate_alignment: an alignment pair has zero length (model \
                 {model_span}, paper {paper_span}); two distinct points per space are required"
            )));
        }
        let paper_per_model = paper_span / model_span;
        let model_per_paper = 1.0 / paper_per_model;
        if !model_per_paper.is_finite() || model_per_paper <= 0.0 {
            return Err(CadError::InvalidInput(
                "alignspace.degenerate_alignment: the resolved scale is not finite and positive"
                    .into(),
            ));
        }
        let document_id = command.document;
        let drawing = self.drawing(document_id)?.clone();
        let mut viewports = layout_viewports(&drawing, layout_id)?;
        let viewport = viewports.first_mut().ok_or_else(|| {
            CadError::Unsupported(format!(
                "alignspace.no_viewport: layout {} has no viewport to align; this build has no \
                 viewport selection, so ALIGNSPACE uses the first viewport",
                layout_id.0
            ))
        })?;
        // paper = (model - t) * paper_per_model, so `model_a` lands on `paper_a`
        // when t = model_a - paper_a * model_per_paper.
        let translation_x = model_a.x - paper_a.x * model_per_paper;
        let translation_y = model_a.y - paper_a.y * model_per_paper;
        viewport.model_to_paper.matrix[0][0] = model_per_paper;
        viewport.model_to_paper.matrix[1][1] = model_per_paper;
        viewport.model_to_paper.matrix[0][3] = translation_x;
        viewport.model_to_paper.matrix[1][3] = translation_y;
        let before = layout_row(&drawing, layout_id);
        let transaction = next_transaction_id();
        let mut candidate = drawing;
        let change_set =
            candidate.set_layout_viewports(layout_id, viewports, transaction, "alignspace")?;
        let after = layout_row(&candidate, layout_id);
        self.publish_layout_record(
            document_id,
            candidate,
            transaction,
            "alignspace",
            vec![layout_patch(layout_id, before, after)],
        )?;
        let mut diagnostics = vec![Diagnostic {
            object: None,
            code: "alignspace.aligned".into(),
            message: format!(
                "ALIGNSPACE: aligned viewport 0 of layout {} so model point ({}, {}) sits on paper \
                 point ({}, {}) with uniform scale {paper_per_model} paper units per model unit \
                 (stored model units per paper unit {model_per_paper}); the second pair only fixed \
                 the scale",
                layout_id.0, model_a.x, model_a.y, paper_a.x, paper_a.y
            ),
        }];
        let model_direction = (
            (model_b.x - model_a.x) / model_span,
            (model_b.y - model_a.y) / model_span,
        );
        let paper_direction = (
            (paper_b.x - paper_a.x) / paper_span,
            (paper_b.y - paper_a.y) / paper_span,
        );
        let cross = model_direction.0 * paper_direction.1 - model_direction.1 * paper_direction.0;
        if cross.abs() > PARALLEL_TOLERANCE {
            let degrees = cross.asin().to_degrees();
            diagnostics.push(Diagnostic {
                object: None,
                code: "alignspace.rotation_ignored".into(),
                message: format!(
                    "ALIGNSPACE: the two alignment pairs differ by {degrees}°; a plan paper \
                     viewport can only pan and zoom, so the rotation is NOT applied and the second \
                     pair fixes the scale only"
                ),
            });
        }
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: Some(Arc::new(change_set)),
            diagnostics,
            measurement: None,
            point_report: None,
        })
    }

    /// Handle `VPSYNC`: copy the master viewport alignment to the others.
    pub(crate) fn vp_sync(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        require_work_mode(session, "vpsync", "VPSYNC")?;
        let layout_id = require_paper_space(session, "vpsync")?;
        require_none_payload(command, "VPSYNC")?;
        let document_id = command.document;
        let drawing = self.drawing(document_id)?.clone();
        let mut viewports = layout_viewports(&drawing, layout_id)?;
        if viewports.len() < 2 {
            return Err(CadError::Unsupported(format!(
                "vpsync.insufficient_viewports: layout {} has {} viewport(s); VPSYNC needs a \
                 master and at least one other viewport to synchronize",
                layout_id.0,
                viewports.len()
            )));
        }
        let master = viewports[0].model_to_paper;
        let matrix = master.matrix;
        let scale_x = matrix[0][0];
        let scale_y = matrix[1][1];
        let extent = scale_x.abs().max(scale_y.abs()).max(1.0);
        if !matrix.iter().flatten().all(|value| value.is_finite())
            || scale_x <= 0.0
            || scale_y <= 0.0
            || (scale_x - scale_y).abs() > 1e-9 * extent
        {
            return Err(CadError::Unsupported(format!(
                "vpsync.invalid_master: viewport 0 of layout {} stores a non-finite or non-uniform \
                 model_to_paper scale ({scale_x}, {scale_y}); only a uniform pan+zoom alignment can \
                 be copied",
                layout_id.0
            )));
        }
        let mut changed = 0usize;
        for viewport in viewports.iter_mut().skip(1) {
            if viewport.model_to_paper != master {
                viewport.model_to_paper = master;
                changed += 1;
            }
        }
        if changed == 0 {
            return Ok(CommandOutcome {
                objects: Vec::new(),
                changes: None,
                diagnostics: vec![Diagnostic {
                    object: None,
                    code: "vpsync.unchanged".into(),
                    message: format!(
                        "VPSYNC: every viewport of layout {} already stores the alignment of \
                         viewport 0; no transaction was opened",
                        layout_id.0
                    ),
                }],
                measurement: None,
                point_report: None,
            });
        }
        let before = layout_row(&drawing, layout_id);
        let transaction = next_transaction_id();
        let mut candidate = drawing;
        let viewport_count = viewports.len();
        let change_set =
            candidate.set_layout_viewports(layout_id, viewports, transaction, "vpsync")?;
        let after = layout_row(&candidate, layout_id);
        self.publish_layout_record(
            document_id,
            candidate,
            transaction,
            "vpsync",
            vec![layout_patch(layout_id, before, after)],
        )?;
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: Some(Arc::new(change_set)),
            diagnostics: vec![Diagnostic {
                object: None,
                code: "vpsync.applied".into(),
                message: format!(
                    "VPSYNC: copied the alignment of viewport 0 (model units per paper unit \
                     {scale_x}, translation {}, {}) to {changed} of the other {} viewport(s) of \
                     layout {}; this build has no viewport selection, so viewport 0 is the master",
                    matrix[0][3],
                    matrix[1][3],
                    viewport_count - 1,
                    layout_id.0
                ),
            }],
            measurement: None,
            point_report: None,
        })
    }
}
