//! `BREAKATPOINT`: split one object into two at a single point.
//!
//! Split out of `break_curve.rs` (the code-size limit); it is a private helper
//! module of that file, not a second command surface. Supported targets are
//! LINE, ARC and an **open** LWPOLYLINE; a CIRCLE or a closed polyline is
//! `breakatpoint.closed`, a SPLINE is `breakatpoint.spline_unsupported` and
//! every other kind is `breakatpoint.unsupported`. The break point is projected
//! onto the target, the source keeps the first piece and the second piece is a
//! fresh id in one transaction / one undo step.

use super::super::polyline_curve::PolylineCurve;
use super::super::trim_curves::{cut_tolerance, TargetCurve};
use super::super::*;
use cad_db::DbObject;

/// A `BREAKATPOINT` refusal.
fn at_point_refusal(code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::InvalidInput(format!("{code}: {detail}"))
}

impl Application {
    /// `BREAKATPOINT`: split one object into two at a single point.
    ///
    /// Supported targets are LINE, ARC and an **open** LWPOLYLINE. AutoCAD's
    /// BREAKATPOINT refuses closed objects, and so does this build: a CIRCLE or a
    /// closed polyline is `breakatpoint.closed`; a SPLINE is
    /// `breakatpoint.spline_unsupported`; every other kind is
    /// `breakatpoint.unsupported`. The break point is **projected** onto the
    /// target (the host resolved the entity with its pickbox), so a click a few
    /// pixels off the curve is still valid.
    ///
    /// The source entity is retained as the first piece (same id/layer/space/
    /// draw order/display state); the second piece is a fresh-id entity copying
    /// the source's display attributes. One transaction / one undo step.
    pub(crate) fn break_at_point(&mut self, command: &Command) -> CadResult<CommandOutcome> {
        let CommandPayload::BreakAtPoint { target, point } = &command.payload else {
            return Err(at_point_refusal(
                "breakatpoint.payload",
                "BREAKATPOINT needs a BreakAtPoint { target, point } payload",
            ));
        };
        if !is_finite_point(*point) {
            return Err(at_point_refusal(
                "breakatpoint.invalid_point",
                "the break point must be finite",
            ));
        }
        if !target.instance.0.is_empty() || target.sub_element.is_some() {
            return Err(at_point_refusal(
                "breakatpoint.not_addressable",
                format!(
                    "target {} is a block-instance member or sub-element; only directly \
                     addressable model-space entities can be split",
                    target.entity.0
                ),
            ));
        }
        if target.document != command.document {
            return Err(at_point_refusal(
                "breakatpoint.target_missing",
                "the target belongs to another document",
            ));
        }

        let document_id = command.document;
        let tol = cut_tolerance();
        let document = self
            .workspace
            .documents
            .get_mut(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        let before = document
            .drawing
            .entity(target.entity)
            .cloned()
            .ok_or_else(|| {
                at_point_refusal(
                    "breakatpoint.target_missing",
                    format!("target {} does not exist", target.entity.0),
                )
            })?;
        if before.space != SpaceId::Model {
            return Err(at_point_refusal(
                "breakatpoint.not_addressable",
                format!(
                    "target {} is not a model-space entity; only model space is supported",
                    before.id.0
                ),
            ));
        }

        let (first, second) = match &before.geometry {
            SemanticGeometry::Circle { .. } => {
                return Err(at_point_refusal(
                    "breakatpoint.closed",
                    "BREAKATPOINT does not split a closed object such as a CIRCLE",
                ));
            }
            SemanticGeometry::Polyline {
                points,
                bulges,
                closed,
                ..
            } => {
                if *closed {
                    return Err(at_point_refusal(
                        "breakatpoint.closed",
                        "BREAKATPOINT does not split a closed polyline",
                    ));
                }
                let curve = PolylineCurve::new(points, bulges, false, "breakatpoint")?;
                curve.split_at_point(*point, tol)?
            }
            SemanticGeometry::Spline { .. } => {
                return Err(CadError::Unsupported(
                    "breakatpoint.spline_unsupported: BREAKATPOINT does not split a SPLINE; \
                     only LINE, ARC and an open LWPOLYLINE are supported"
                        .into(),
                ));
            }
            SemanticGeometry::Line { .. } | SemanticGeometry::Arc { .. } => {
                curve_split_at_point(&before.geometry, *point, tol)?
            }
            other => {
                return Err(CadError::Unsupported(format!(
                    "breakatpoint.unsupported: {} is not LINE, ARC or an open LWPOLYLINE; \
                     BREAKATPOINT changes nothing",
                    class_name(other)
                )));
            }
        };

        let source_id = before.id;
        let source_display = document.drawing.entity_display_state(source_id);
        let mut candidate = document.drawing.as_ref().clone();
        let drawing = &mut candidate;
        let new_id = drawing.allocate_entity_id();
        let new_display = cad_db::EntityDisplayState {
            attributes: source_display.attributes.clone(),
            visibility: Default::default(),
        };
        let transaction = next_transaction_id();
        let change_set = {
            let mut staged = drawing.begin_drawing_transaction("break at point", transaction)?;
            let mut after = before.clone();
            after.geometry = first;
            after.object.type_key = class_name(&after.geometry).to_string();
            staged.update_entity(after)?;
            let entity = DbEntity {
                object: DbObject {
                    id: ObjectId(new_id.0),
                    type_key: class_name(&second).to_string(),
                    revision: Revision(0),
                    source_handle: None,
                },
                id: new_id,
                layer: before.layer,
                space: before.space.clone(),
                geometry: second,
                draw_order: before.draw_order,
            };
            staged.insert_entity(entity)?;
            staged.update_entity_display_state(new_id, &new_display)?;
            staged.commit()?
        };

        let stored_first = drawing
            .entity(source_id)
            .cloned()
            .ok_or_else(|| CadError::Invariant("split entity vanished after commit".into()))?;
        let stored_second = drawing.entity(new_id).cloned().ok_or_else(|| {
            CadError::Invariant("inserted split piece vanished after commit".into())
        })?;
        let mut second_patch = drawing_patch(new_id, None, Some(stored_second));
        second_patch.after_state = Some(new_display);
        let patches = vec![
            drawing_patch(source_id, Some(before), Some(stored_first)),
            second_patch,
        ];
        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: "break at point".into(),
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

        Ok(CommandOutcome {
            objects: vec![ObjectId(source_id.0), ObjectId(new_id.0)],
            changes: Some(Arc::new(change_set)),
            diagnostics: vec![Diagnostic {
                object: Some(ObjectId(source_id.0)),
                code: "breakatpoint.applied".into(),
                message: format!(
                    "BREAKATPOINT: split entity {} into two pieces at the projected point",
                    source_id.0
                ),
            }],
            measurement: None,
            point_report: None,
        })
    }
}

/// Split a LINE or ARC at the projection of `point` (one parameter on the
/// target's own span).
fn curve_split_at_point(
    geometry: &SemanticGeometry,
    point: Point3,
    tol: f64,
) -> CadResult<(SemanticGeometry, SemanticGeometry)> {
    let curve = TargetCurve::new(geometry, "BREAKATPOINT", "LINE, ARC or open LWPOLYLINE")?;
    let Some(u) = curve.project_param(point, tol) else {
        return Err(at_point_refusal(
            "breakatpoint.projection_failed",
            format!("the point has no projection onto the {}", curve.label()),
        ));
    };
    let span = curve.length();
    if !span.is_finite() || span <= tol {
        return Err(at_point_refusal(
            "breakatpoint.projection_failed",
            format!("the {} is degenerate (zero length)", curve.label()),
        ));
    }
    let param_tol = (tol / span).max(1e-12);
    if !u.is_finite() || u < -param_tol || u > 1.0 + param_tol {
        return Err(at_point_refusal(
            "breakatpoint.point_outside_span",
            format!(
                "the point projects outside the {} (parameter {u:.9}, the target spans [0, 1])",
                curve.label()
            ),
        ));
    }
    let u = u.clamp(0.0, 1.0);
    if u <= param_tol || u >= 1.0 - param_tol {
        return Err(at_point_refusal(
            "breakatpoint.point_at_endpoint",
            format!(
                "the break point coincides with an endpoint of the {}; there is no interior \
                 split to make",
                curve.label()
            ),
        ));
    }
    let first = curve
        .with_span(0.0, u)
        .map_err(|reason| at_point_refusal("breakatpoint.degenerate_piece", reason))?;
    let second = curve
        .with_span(u, 1.0 - u)
        .map_err(|reason| at_point_refusal("breakatpoint.degenerate_piece", reason))?;
    Ok((first, second))
}
