//! `VPCLIP` polygon boundary: replace the first viewport's clip with an N-gon.
//!
//! [`cad_db::PaperViewport::clip`] is already a `Vec<Point3>` polygon, so a
//! polygonal boundary needs no new database field: the validated points are
//! stored verbatim through the one `set_layout_viewports` write (one revision,
//! one undo step).
//!
//! # Refusals (nothing written, no revision, no undo entry)
//!
//! * `vpclip.paper_space_required` — the active space is not a paper layout;
//! * `vpclip.degenerate_polygon` — fewer than three points, a non-finite
//!   coordinate, a repeated vertex, or a zero-area boundary;
//! * `vpclip.non_coplanar` — the points do not lie on one paper plane;
//! * `vpclip.self_intersecting` — two non-adjacent edges cross or touch;
//! * `vpclip.no_viewport` — the active layout has no viewport to clip.
//!
//! # Residual
//!
//! `cad-representation::viewport_transform` resolves only three- and
//! four-point clips, so the written viewport reports
//! `representation.viewport_unsupported: clip_arity` (a `Partial` layout) until
//! that lane learns N-gons. The command reports this as
//! `vpclip.polygon_not_renderable`; it never claims the new boundary is drawn.

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
            diagnostics: vec![
                Diagnostic {
                    object: None,
                    code: "vpclip.applied_polygon".into(),
                    message: format!(
                        "VPCLIP: replaced the clip of viewport 0 in layout {} with a {}-point \
                         polygonal boundary; this build has no viewport selection, so the first \
                         viewport was clipped",
                        layout_id.0,
                        points.len()
                    ),
                },
                Diagnostic {
                    object: None,
                    code: "vpclip.polygon_not_renderable".into(),
                    message:
                        "VPCLIP: the polygonal boundary is real stored viewport geometry, but \
                              the representation lane resolves only three- and four-point clips, \
                              so this viewport renders as viewport_unsupported: clip_arity until \
                              that lane learns N-gons; no draw is claimed here"
                            .into(),
                },
            ],
            measurement: None,
            point_report: None,
        })
    }
}
