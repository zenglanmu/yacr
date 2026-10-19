//! The confirmed-draw intent -> command mapping for [`HostController`].
//!
//! Split out of `host/mod.rs` to keep that module inside the code-size limit.
//! [`HostController::command_for_draw_intent`] is the one place a captured
//! [`crate::DrawIntent`] becomes exactly one [`crate::Command`]; it stays an
//! inherent method on [`HostController`] so the call sites in `host/mod.rs` are
//! unchanged.

use cad_domain::{CadResult, SelectionRef};

use crate::{Command, DrawIntent};

use super::HostController;

impl HostController {
    pub(super) fn command_for_draw_intent(
        &self,
        intent: DrawIntent,
        selection: &[SelectionRef],
    ) -> CadResult<Command> {
        let (id, payload) = match intent {
            DrawIntent::Line { start, end } => (
                crate::CommandId::CreateLine,
                crate::CommandPayload::Points(vec![start, end]),
            ),
            DrawIntent::Circle { center, edge } => (
                crate::CommandId::CreateCircle,
                crate::CommandPayload::Points(vec![center, edge]),
            ),
            DrawIntent::Arc { points } => (
                crate::CommandId::CreateArc,
                crate::CommandPayload::Points(points.to_vec()),
            ),
            // PLINE: the captured vertices travel as `Points` (the payload the
            // create path already reads), so the handler builds the
            // `SemanticGeometry::Polyline` from exactly the points the user saw.
            DrawIntent::Polyline { points } => (
                crate::CommandId::CreatePolyline,
                crate::CommandPayload::Points(points),
            ),
            // RECTANG: the two captured corners travel in click order as `Points`
            // (the payload the create path already reads); the handler canonicalises
            // the winding and builds the closed four-vertex polyline.
            DrawIntent::Rect { corner1, corner2 } => (
                crate::CommandId::CreateRectangle,
                crate::CommandPayload::Points(vec![corner1, corner2]),
            ),
            // ELLIPSE: the three captured axis-endpoint picks travel in click
            // order as `Points`; the handler builds the ellipse frame.
            DrawIntent::Ellipse { p1, p2, p3 } => (
                crate::CommandId::CreateEllipse,
                crate::CommandPayload::Points(vec![p1, p2, p3]),
            ),
            // POLYGON: the centre and radius point travel as `Points` (the
            // construction input the create path already reads); the side count
            // is the session sysvar the handler reads, so it never rides the
            // payload.
            DrawIntent::Polygon {
                center,
                radius_point,
            } => (
                crate::CommandId::CreatePolygon,
                crate::CommandPayload::Points(vec![center, radius_point]),
            ),
            // POINT: the single captured position travels as `Points`.
            DrawIntent::Point { position } => (
                crate::CommandId::CreatePoint,
                crate::CommandPayload::Points(vec![position]),
            ),
            // SPLINE: the captured fit points travel as `Points`; the handler runs
            // the interpolation and stores one Spline entity.
            DrawIntent::Spline { fit_points } => (
                crate::CommandId::CreateSpline,
                crate::CommandPayload::Points(fit_points),
            ),
            DrawIntent::Move { delta } => (
                crate::CommandId::MoveEntities,
                crate::CommandPayload::Move {
                    refs: selection.to_vec(),
                    delta,
                },
            ),
            DrawIntent::Copy { delta } => (
                crate::CommandId::CopyEntities,
                crate::CommandPayload::Copy {
                    refs: selection.to_vec(),
                    delta,
                },
            ),
            DrawIntent::Trim {
                target,
                boundary,
                pick_point,
            } => (
                crate::CommandId::TrimEntity,
                crate::CommandPayload::Trim {
                    target,
                    boundary: vec![boundary],
                    pick_point,
                },
            ),
            DrawIntent::Extend {
                target,
                boundary,
                pick_point,
            } => (
                crate::CommandId::ExtendEntity,
                crate::CommandPayload::Extend {
                    target,
                    boundary: vec![boundary],
                    pick_point,
                },
            ),
            DrawIntent::Rotate { base, angle } => (
                crate::CommandId::RotateEntities,
                crate::CommandPayload::Rotate {
                    refs: selection.to_vec(),
                    base,
                    angle,
                },
            ),
            DrawIntent::Scale { base, factor } => (
                crate::CommandId::ScaleEntities,
                crate::CommandPayload::Scale {
                    refs: selection.to_vec(),
                    base,
                    factor,
                },
            ),
            DrawIntent::Mirror { p1, p2 } => (
                crate::CommandId::MirrorEntities,
                crate::CommandPayload::Mirror {
                    refs: selection.to_vec(),
                    axis_start: p1,
                    axis_end: p2,
                },
            ),
            DrawIntent::Offset { side_point } => (
                crate::CommandId::OffsetEntity,
                crate::CommandPayload::Offset {
                    refs: selection.to_vec(),
                    side_point,
                },
            ),
        };
        Ok(Command {
            schema_version: 1,
            id,
            document: self.document_id,
            viewport: self.viewport_id,
            payload,
        })
    }
}
