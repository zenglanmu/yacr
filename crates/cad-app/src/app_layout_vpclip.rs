//! `VPCLIP`: replace the clip boundary of the targeted layout viewport.
//!
//! Both the rectangular and the polygonal form are **real** viewport clips, not
//! just stored data: `cad-representation` carries the boundary as a
//! [`cad_representation::layout::PaperClip`] and limits every primitive (lines,
//! meshes, images, conics) to it, including concave boundaries. There is no
//! "written but not drawable" residual left for these commands.
//!
//! # Targeting
//!
//! `parse_vpclip_argument` (`VPCLIP RECT …`) produces `viewport: None`, and so
//! does every other producer that has no explicit selection: the **session
//! `CVPORT` current viewport of the active paper layout** is clipped. A payload
//! with `Some(index)` — the explicit-index form — always wins over the marker.
//! The marker itself is session state ([`crate::command_id::extras`]) and is
//! never written by this command; the clip write below is one revision and one
//! undo step.
//!
//! # Refusals (nothing written, no revision, no undo entry)
//!
//! * `vpclip.paper_space_required` — the active space is not a paper layout;
//! * `vpclip.degenerate_rect` — a non-finite or inverted rectangle;
//! * `vpclip.degenerate_polygon` — fewer than three points, a non-finite
//!   coordinate, a repeated vertex, or a zero-area boundary;
//! * `vpclip.non_coplanar` — the points do not lie on one paper plane;
//! * `vpclip.self_intersecting` — two non-adjacent edges cross or touch;
//! * `vpclip.no_viewport` — the active layout has no viewport to clip;
//! * `vpclip.unknown_viewport` — an explicit index outside the viewport count;
//! * `vpclip.current_viewport_out_of_range` — a stale `CVPORT` marker (never
//!   clamped to the first viewport);
//! * `vpclip.ambiguous_triangle` — a three-point boundary on a viewport whose
//!   transform carries no translation, which is exactly the legacy
//!   `[a, b, view_center]` rectangle record; writing it would store a clip that
//!   reads back as a rectangle, so the write is refused instead (a four-or-more
//!   point boundary, or a viewport whose transform is translated, is
//!   unambiguous).
//!
//! # Residual
//!
//! Viewport **holes** are still not modeled: one boundary per viewport.

use super::*;

use crate::app_drawing::{layout_row, next_transaction_id};
use crate::app_layout::{layout_viewports, require_paper_space};
use crate::app_layout_geometry::validate_clip_polygon;
use crate::session_state::target_viewport;

impl Application {
    /// Handle `LayoutPayload::ClipRect`: one rectangular `VPCLIP` write.
    pub(super) fn clip_rect(
        &mut self,
        session: &SessionState,
        command: &Command,
        min: Point3,
        max: Point3,
        explicit: Option<usize>,
    ) -> CadResult<CommandOutcome> {
        // The paper-space refusal comes first, so a degenerate rectangle in
        // model space reports the space, not the numbers.
        require_paper_space(session, "vpclip")?;
        if !min.x.is_finite()
            || !min.y.is_finite()
            || !max.x.is_finite()
            || !max.y.is_finite()
            || min.x >= max.x
            || min.y >= max.y
        {
            return Err(CadError::InvalidInput(
                "vpclip.degenerate_rect: VPCLIP RECT needs finite minx < maxx and miny < maxy"
                    .into(),
            ));
        }
        let clip = vec![
            Point3 {
                x: min.x,
                y: min.y,
                z: 0.0,
            },
            Point3 {
                x: max.x,
                y: min.y,
                z: 0.0,
            },
            Point3 {
                x: max.x,
                y: max.y,
                z: 0.0,
            },
            Point3 {
                x: min.x,
                y: max.y,
                z: 0.0,
            },
        ];
        self.write_viewport_clip(
            session,
            command,
            explicit,
            clip,
            ClipReport {
                code: "vpclip.applied",
                spelling: "VPCLIP RECT",
                boundary: format!(
                    "the rectangle ({}, {})-({}, {})",
                    min.x, min.y, max.x, max.y
                ),
            },
        )
    }

    /// Handle `LayoutPayload::ClipPolygon`: one polygonal `VPCLIP` write.
    pub(super) fn clip_polygon(
        &mut self,
        session: &SessionState,
        command: &Command,
        points: &[Point3],
        explicit: Option<usize>,
    ) -> CadResult<CommandOutcome> {
        validate_clip_polygon(points, "vpclip")?;
        self.write_viewport_clip(
            session,
            command,
            explicit,
            points.to_vec(),
            ClipReport {
                code: "vpclip.applied_polygon",
                spelling: "VPCLIP",
                boundary: format!("a {}-point polygonal boundary", points.len()),
            },
        )
    }

    /// The one clip write both `VPCLIP` forms share.
    ///
    /// `explicit` is the payload's zero-based viewport index (`None` = the
    /// session `CVPORT` current viewport). Everything that can refuse is checked
    /// before the candidate drawing is touched, so a refusal records no revision
    /// and no undo entry.
    fn write_viewport_clip(
        &mut self,
        session: &SessionState,
        command: &Command,
        explicit: Option<usize>,
        clip: Vec<Point3>,
        report: ClipReport<'_>,
    ) -> CadResult<CommandOutcome> {
        let layout_id = require_paper_space(session, "vpclip")?;
        let document_id = command.document;
        let drawing = self.drawing(document_id)?.clone();
        let mut viewports = layout_viewports(&drawing, layout_id)?;
        if viewports.is_empty() {
            return Err(CadError::Unsupported(format!(
                "vpclip.no_viewport: layout {} has no viewport to clip",
                layout_id.0
            )));
        }
        let index = target_viewport(
            layout_id,
            explicit,
            session.viewport_marker(layout_id),
            viewports.len(),
            "vpclip",
        )?;
        let viewport = &mut viewports[index];
        // A three-point boundary is only distinguishable from the legacy
        // `[a, b, view_center]` rectangle record by the transform translation
        // (see `cad_representation::layout`); refuse rather than store a clip
        // that would read back as a rectangle.
        let translation = &viewport.model_to_paper.matrix;
        let translated = [translation[0][3], translation[1][3], translation[2][3]]
            .iter()
            .any(|value| value.abs() > 1e-9);
        if clip.len() == 3 && !translated {
            return Err(CadError::InvalidInput(format!(
                "vpclip.ambiguous_triangle: viewport {index} of layout {} has a translation-free \
                 transform, so a three-point boundary would be read back as the legacy \
                 [a, b, view_center] rectangle; use four or more points",
                layout_id.0
            )));
        }
        viewport.clip = clip;
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
                code: report.code.into(),
                message: format!(
                    "{}: replaced the clip of viewport {index} of layout {} with {}; the \
                     representation layer clips that viewport to the polygon; the target came \
                     from {} and the CVPORT marker itself was not written",
                    report.spelling,
                    layout_id.0,
                    report.boundary,
                    if explicit.is_some() {
                        "the explicit payload index, which overrides CVPORT"
                    } else {
                        "the session CVPORT current viewport"
                    }
                ),
            }],
            measurement: None,
            point_report: None,
        })
    }
}

/// What one clip write reports: its stable code, the selector spelling and the
/// boundary description (`VPCLIP RECT` and the polygonal form share the write).
struct ClipReport<'a> {
    code: &'a str,
    spelling: &'a str,
    boundary: String,
}
