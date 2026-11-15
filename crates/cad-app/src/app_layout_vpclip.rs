//! `VPCLIP` polygon boundary: replace the first viewport's clip with an N-gon.
//!
//! [`cad_db::PaperViewport::clip`] is already a `Vec<Point3>` polygon, so a
//! polygonal boundary needs no new database field: the validated points are
//! stored verbatim through the one `set_layout_viewports` write (one revision,
//! one undo step).
//!
//! The boundary is a **real** viewport clip, not just stored data:
//! `cad-representation` carries it as a [`cad_representation::layout::PaperClip`]
//! and limits every primitive (lines, meshes, images, conics) to it, including
//! concave boundaries. There is no "written but not drawable" residual left for
//! this command.
//!
//! # Refusals (nothing written, no revision, no undo entry)
//!
//! * `vpclip.paper_space_required` — the active space is not a paper layout;
//! * `vpclip.degenerate_polygon` — fewer than three points, a non-finite
//!   coordinate, a repeated vertex, or a zero-area boundary;
//! * `vpclip.non_coplanar` — the points do not lie on one paper plane;
//! * `vpclip.self_intersecting` — two non-adjacent edges cross or touch;
//! * `vpclip.no_viewport` — the active layout has no viewport to clip;
//! * `vpclip.ambiguous_triangle` — a three-point boundary on a viewport whose
//!   transform carries no translation, which is exactly the legacy
//!   `[a, b, view_center]` rectangle record; writing it would store a clip that
//!   reads back as a rectangle, so the write is refused instead (a four-or-more
//!   point boundary, or a viewport whose transform is translated, is
//!   unambiguous).
//!
//! # Residual
//!
//! This build has no viewport **selection**, so `VPCLIP` clips the first
//! viewport (reported in `vpclip.applied_polygon`). Viewport **holes** are still
//! not modeled: one boundary per viewport.

use super::*;

use crate::app_drawing::{layout_row, next_transaction_id};
use crate::app_layout::{layout_viewports, require_paper_space};
use crate::app_layout_geometry::validate_clip_polygon;

impl Application {
    /// Handle `LayoutPayload::ClipPolygon`: one polygonal `VPCLIP` write.
    pub(super) fn clip_polygon(
        &mut self,
        session: &SessionState,
        command: &Command,
        points: &[Point3],
    ) -> CadResult<CommandOutcome> {
        let layout_id = require_paper_space(session, "vpclip")?;
        validate_clip_polygon(points, "vpclip")?;
        let document_id = command.document;
        let drawing = self.drawing(document_id)?.clone();
        let mut viewports = layout_viewports(&drawing, layout_id)?;
        let viewport = viewports.first_mut().ok_or_else(|| {
            CadError::Unsupported(format!(
                "vpclip.no_viewport: layout {} has no viewport to clip; this build has no \
                 viewport selection, so VPCLIP uses the first viewport",
                layout_id.0
            ))
        })?;
        // A three-point boundary is only distinguishable from the legacy
        // `[a, b, view_center]` rectangle record by the transform translation
        // (see `cad_representation::layout`); refuse rather than store a clip
        // that would read back as a rectangle.
        let translation = &viewport.model_to_paper.matrix;
        let translated = [translation[0][3], translation[1][3], translation[2][3]]
            .iter()
            .any(|value| value.abs() > 1e-9);
        if points.len() == 3 && !translated {
            return Err(CadError::InvalidInput(format!(
                "vpclip.ambiguous_triangle: viewport 0 of layout {} has a translation-free \
                 transform, so a three-point boundary would be read back as the legacy \
                 [a, b, view_center] rectangle; use four or more points",
                layout_id.0
            )));
        }
        viewport.clip = points.to_vec();
        let before = layout_row(&drawing, layout_id);
        let transaction = next_transaction_id();
        let mut candidate = drawing;
        let change_set =
            candidate.set_layout_viewports(layout_id, viewports, transaction, "vpclip")?;
        let after = layout_row(&candidate, layout_id);
        self.publish_layout_record(
            document_id,
            candidate,
            transaction,
            "vpclip",
            vec![layout_patch(layout_id, before, after)],
        )?;
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: Some(Arc::new(change_set)),
            diagnostics: vec![Diagnostic {
                object: None,
                code: "vpclip.applied_polygon".into(),
                message: format!(
                    "VPCLIP: replaced the clip of viewport 0 in layout {} with a {}-point \
                     polygonal boundary; the representation layer clips that viewport to the \
                     polygon; this build has no viewport selection, so the first viewport was \
                     clipped",
                    layout_id.0,
                    points.len()
                ),
            }],
            measurement: None,
            point_report: None,
        })
    }
}
