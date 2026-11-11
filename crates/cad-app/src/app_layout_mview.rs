//! `MVIEW`: append one paper viewport to the active layout.
//!
//! Two shapes are supported and both are one `set_layout_viewports` write, one
//! revision and one undo step:
//!
//! * **two corner points** — the axis-aligned `min`/`max` rectangle
//!   (`mview.created`), stored as the documented four paper corners;
//! * **three or more points** — a polygonal (non-rectangular) boundary
//!   (`mview.created_polygon`), stored verbatim as
//!   [`cad_db::PaperViewport::clip`].
//!
//! Both follow the importer's paper→model convention with `model_per_paper =
//! 1.0` and translation `-paper_center`, so the model origin sits at the clip
//! centre. Degenerate, non-coplanar and self-intersecting polygons are refused
//! before anything mutates.
//!
//! # Residual
//!
//! `cad-representation` resolves only three- and four-point clips, so a
//! polygonal viewport renders as `viewport_unsupported: clip_arity` until that
//! lane learns N-gons. The command says so in `mview.polygon_not_renderable`;
//! it never claims a draw it cannot produce. Viewport **holes** are still not
//! modeled (one boundary per viewport).

use super::*;

use cad_db::PaperViewport;

use crate::app_drawing::{layout_row, next_transaction_id};
use crate::app_layout::{layout_viewports, require_paper_space};
use crate::app_layout_geometry::{clip_center, validate_clip_polygon};

impl Application {
    /// Create one paper viewport from a typed two-corner rectangle **or** a
    /// polygonal boundary.
    ///
    /// The rectangle is a paper-space axis-aligned `min`/`max` pair (not
    /// canonicalised: the caller must give `minx < maxx` and `miny < maxy`).
    /// Three or more points are a closed polygon in the given order.
    pub(crate) fn mview(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let layout_id = require_paper_space(session, "mview")?;
        let CommandPayload::Points(points) = &command.payload else {
            return Err(CadError::InvalidInput(
                "mview.invalid_rect: MVIEW needs exactly two corner points or three or more \
                 polygon points"
                    .into(),
            ));
        };
        let (clip, description, created_code, residual) = match points.as_slice() {
            [min, max] => {
                if !min.x.is_finite()
                    || !min.y.is_finite()
                    || !max.x.is_finite()
                    || !max.y.is_finite()
                    || min.x >= max.x
                    || min.y >= max.y
                {
                    return Err(CadError::InvalidInput(
                        "mview.invalid_rect: MVIEW needs finite minx < maxx and miny < maxy".into(),
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
                let description = format!(
                    "the paper rectangle ({}, {})-({}, {})",
                    min.x, min.y, max.x, max.y
                );
                let residual = Diagnostic {
                    object: None,
                    code: "mview.no_holes".into(),
                    message: "MVIEW: the created viewport is one four-corner rectangle with no \
                              holes; viewport holes are NOT IMPLEMENTED"
                        .into(),
                };
                (clip, description, "mview.created", residual)
            }
            _ if points.len() >= 3 => {
                validate_clip_polygon(points, "mview")?;
                let description = format!("a {}-point polygonal boundary", points.len());
                let residual = Diagnostic {
                    object: None,
                    code: "mview.polygon_not_renderable".into(),
                    message: "MVIEW: stored the polygonal clip as real viewport geometry, but the \
                              representation lane resolves only three- and four-point clips, so \
                              this viewport renders as viewport_unsupported: clip_arity until \
                              that lane learns N-gons; no draw is claimed here"
                        .into(),
                };
                (
                    points.clone(),
                    description,
                    "mview.created_polygon",
                    residual,
                )
            }
            _ => {
                return Err(CadError::InvalidInput(
                    "mview.invalid_rect: MVIEW needs exactly two corner points or three or more \
                     polygon points"
                        .into(),
                ))
            }
        };
        let document_id = command.document;
        let drawing = self.drawing(document_id)?.clone();
        let mut viewports = layout_viewports(&drawing, layout_id)?;
        let paper_center = clip_center(&clip).ok_or_else(|| {
            CadError::InvalidInput(
                "mview.invalid_rect: the MVIEW boundary has no finite paper extent".into(),
            )
        })?;
        let mut matrix = Transform3::identity().matrix;
        matrix[0][0] = 1.0;
        matrix[1][1] = 1.0;
        matrix[0][3] = -paper_center.x;
        matrix[1][3] = -paper_center.y;
        viewports.push(PaperViewport {
            clip,
            model_to_paper: Transform3 { matrix },
            completeness: Completeness::Complete,
            layer_overrides: std::collections::BTreeMap::new(),
        });
        let index = viewports.len() - 1;
        let before = layout_row(&drawing, layout_id);
        let transaction = next_transaction_id();
        let mut candidate = drawing;
        let change_set =
            candidate.set_layout_viewports(layout_id, viewports, transaction, "mview")?;
        let after = layout_row(&candidate, layout_id);
        self.publish_layout_record(
            document_id,
            candidate,
            transaction,
            "mview",
            vec![layout_patch(layout_id, before, after)],
        )?;
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: Some(Arc::new(change_set)),
            diagnostics: vec![
                Diagnostic {
                    object: None,
                    code: created_code.into(),
                    message: format!(
                        "MVIEW: appended viewport {index} to layout {} with {description}",
                        layout_id.0
                    ),
                },
                residual,
            ],
            measurement: None,
            point_report: None,
        })
    }
}
