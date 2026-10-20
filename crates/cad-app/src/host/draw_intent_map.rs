//! The confirmed-draw intent -> command mapping for [`HostController`].
//!
//! Split out of `host/mod.rs` to keep that module inside the code-size limit.
//! [`HostController::command_for_draw_intent`] is the one place a captured
//! [`crate::DrawIntent`] becomes exactly one [`crate::Command`]; it stays an
//! inherent method on [`HostController`] so the call sites in `host/mod.rs` are
//! unchanged.

use cad_domain::{CadError, CadResult, Point3, SelectionRef};

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
            // TEXT: the insertion point and the terminal text travel on the
            // payload; the height is the session typed channel
            // (`ToolParameter::TextHeight`), read here so the intent never carries
            // a session value of its own. The text was validated at capture time
            // (non-empty, at most `MAX_TOOL_TEXT_CHARS` characters) and is
            // re-validated by the command.
            DrawIntent::Text { position, text } => (
                crate::CommandId::CreateText,
                crate::CommandPayload::Text {
                    position,
                    text,
                    height: self
                        .session
                        .tool_parameter(crate::ToolParameter::TextHeight)
                        .number()
                        .unwrap_or(crate::draw_tool::TEXT_DEFAULT_HEIGHT),
                },
            ),
            // TEXTEDIT: the pick resolved at capture time plus the replacement
            // text. The command re-reads the entity at apply time (never a stale
            // retarget) and preserves every field but the string.
            DrawIntent::TextEdit { target, text } => (
                crate::CommandId::TextEdit,
                crate::CommandPayload::TextEdit { target, text },
            ),
            // MLEADER: the two captured points and the terminal text. The style
            // values are resolved by the command from the drawing's
            // `MLeaderStyle` table, so nothing style-shaped travels here.
            DrawIntent::MLeader {
                arrow,
                landing,
                text,
            } => (
                crate::CommandId::MLeader,
                crate::CommandPayload::MLeader {
                    arrow,
                    landing,
                    text,
                },
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
            // CHAMFER/FILLET: the two captured refs and pick points travel on the
            // payload; the distances/radius are the session typed channel the
            // command layer reads, so they never ride the payload.
            DrawIntent::Chamfer {
                first,
                second,
                pick_first,
                pick_second,
            } => (
                crate::CommandId::ChamferLines,
                crate::CommandPayload::Chamfer(Box::new(crate::TwoLineCommand {
                    first,
                    second,
                    pick_first,
                    pick_second,
                })),
            ),
            DrawIntent::Fillet {
                first,
                second,
                pick_first,
                pick_second,
            } => (
                crate::CommandId::FilletLines,
                crate::CommandPayload::Fillet(Box::new(crate::TwoLineCommand {
                    first,
                    second,
                    pick_first,
                    pick_second,
                })),
            ),
            // ARRAYRECT/ARRAYPOLAR: the selection snapshot travels as the refs;
            // the row/column/item parameters and the fill angle are the session
            // typed channel the command layer reads, so they never ride the
            // payload. ARRAYPOLAR additionally carries the one captured centre.
            DrawIntent::ArrayRect => (
                crate::CommandId::ArrayRectangular,
                crate::CommandPayload::ArrayRect {
                    refs: selection.to_vec(),
                },
            ),
            DrawIntent::ArrayPolar { center } => (
                crate::CommandId::ArrayPolar,
                crate::CommandPayload::ArrayPolar {
                    refs: selection.to_vec(),
                    center,
                },
            ),
            // ALIGN: the selection snapshot plus the four captured points travel
            // on the payload; the `AlignScale` flag is the session typed channel
            // the command layer reads, so it never rides the payload.
            DrawIntent::Align {
                source1,
                dest1,
                source2,
                dest2,
            } => (
                crate::CommandId::AlignEntities,
                crate::CommandPayload::Align {
                    refs: selection.to_vec(),
                    source1,
                    dest1,
                    source2,
                    dest2,
                },
            ),
            // STRETCH: the two captured window corners are mapped back to canvas
            // logical coordinates (the `BoxSelect` convention the payload uses),
            // and the base/displacement points travel as world coordinates. The
            // command maps the logical corners through the orthographic plan view
            // and recomputes the enclosure classification.
            DrawIntent::Stretch { rect, base, delta } => (
                crate::CommandId::StretchEntities,
                crate::CommandPayload::Stretch {
                    rect_logical: self.world_rect_to_logical(rect)?,
                    base,
                    delta,
                },
            ),
            // DIMLINEAR/DIMALIGNED: the two captured origins and the
            // dimension-line location travel in click order as `Points` (the
            // construction input the create path already reads, exactly like
            // ELLIPSE/RECTANG); the handler decides horizontal vs vertical
            // (DIMLINEAR) and builds the dimension.
            DrawIntent::DimLinear {
                first,
                second,
                location,
            } => (
                crate::CommandId::DimLinear,
                crate::CommandPayload::Points(vec![first, second, location]),
            ),
            DrawIntent::DimAligned {
                first,
                second,
                location,
            } => (
                crate::CommandId::DimAligned,
                crate::CommandPayload::Points(vec![first, second, location]),
            ),
            // DIMRADIUS/DIMDIAMETER: the picked ref and the location point travel
            // on the boxed `DimRadial` payload; the handler resolves the entity and
            // builds the dimension. The radius/diameter choice comes from the
            // command id, so the payload carries no kind.
            DrawIntent::DimRadius { target, location } => (
                crate::CommandId::DimRadius,
                crate::CommandPayload::DimRadial(Box::new(crate::RadialDimensionCommand {
                    target,
                    location,
                })),
            ),
            DrawIntent::DimDiameter { target, location } => (
                crate::CommandId::DimDiameter,
                crate::CommandPayload::DimRadial(Box::new(crate::RadialDimensionCommand {
                    target,
                    location,
                })),
            ),
            // DIMCENTER: the picked ref travels on the payload; the handler reads
            // the resolved `DIMCEN` and inserts plain geometry.
            DrawIntent::DimCenter { target } => (
                crate::CommandId::DimCenter,
                crate::CommandPayload::DimCenter(target),
            ),
            // DIMANGULAR: the two picked line refs and the arc-location point
            // travel on the boxed payload; the handler resolves the lines and
            // derives the vertex and the measured angle.
            DrawIntent::DimAngular {
                first,
                second,
                location,
            } => (
                crate::CommandId::DimAngular,
                crate::CommandPayload::DimAngular(Box::new(
                    crate::AngularDimensionInput::TwoLine {
                        first,
                        second,
                        location,
                    },
                )),
            ),
            // DIMORDINATE: the feature and leader points travel on the payload;
            // the interactive form never forces an axis (`axis = None`).
            DrawIntent::DimOrdinate { feature, leader } => (
                crate::CommandId::DimOrdinate,
                crate::CommandPayload::DimOrdinate(Box::new(crate::OrdinateDimensionCommand {
                    feature,
                    leader,
                    axis: None,
                })),
            ),
            // DIMBASELINE/DIMCONTINUE: the base ref resolved at pick time plus the
            // measured points travel on the boxed `DimChain` payload; the handler
            // resolves the base again and builds one dimension per point. The
            // baseline/continue choice comes from the command id, so the payload
            // carries no kind.
            DrawIntent::DimBaseline { base, points } => (
                crate::CommandId::DimBaseline,
                crate::CommandPayload::DimChain(Box::new(crate::DimensionChainCommand {
                    base,
                    points,
                })),
            ),
            DrawIntent::DimContinue { base, points } => (
                crate::CommandId::DimContinue,
                crate::CommandPayload::DimChain(Box::new(crate::DimensionChainCommand {
                    base,
                    points,
                })),
            ),
            // HATCH: the selection snapshot is the boundary (the tool captures no
            // points); the pattern type/spacing/angle/double/style are the session
            // typed channel the command layer reads, so they never ride the
            // payload.
            DrawIntent::Hatch => (
                crate::CommandId::Hatch,
                crate::CommandPayload::Hatch {
                    refs: selection.to_vec(),
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

    /// Map the two captured STRETCH window corners from world coordinates back to
    /// canvas logical pixels (the convention [`crate::CommandPayload::Stretch`]
    /// uses, matching `BoxSelect`/`ZoomAt`).
    ///
    /// The captured corners are world points, so they must be projected through
    /// the active viewport. A perspective/3D view has no plan mapping and is an
    /// explicit `Unsupported` (3D STRETCH is NOT IMPLEMENTED); a missing viewport
    /// is an explicit `InvalidInput`. This is a pure projection: no database
    /// write, no history.
    fn world_rect_to_logical(&self, rect: [Point3; 2]) -> CadResult<[[f64; 2]; 2]> {
        let viewport = self
            .application
            .workspace
            .viewports
            .get(&self.viewport_id)
            .ok_or_else(|| CadError::InvalidInput("unknown viewport".into()))?;
        let size = viewport.logical_size;
        let mut out = [[0.0; 2]; 2];
        for (slot, world) in out.iter_mut().zip(rect) {
            *slot = viewport
                .camera
                .world_to_screen(world, size)
                .ok_or_else(|| {
                    CadError::Unsupported("STRETCH requires the orthographic plan view".into())
                })?;
        }
        Ok(out)
    }
}
