//! The [`DrawToolKind`] -> [`CommandId`] mapping of an interactive capture.
//!
//! Split out of `session_state.rs` to keep that module inside the code-size
//! limit; `SessionState::begin_draw` still calls `authorize(command)` and stores
//! the result, so nothing about the capture lifecycle lives here.

use super::*;

/// The command a capture belongs to.
///
/// A kind is not always one command: `COPYBASE` and `CUTBASE` share the
/// `CopyBase` capture shape but differ in authorization (Viewer-legal vs
/// work-only) and in the confirmed payload (capture only vs capture **and**
/// delete). The caller records the result on the capture so the host can map the
/// confirmed payload by the command, never by guessing from the kind.
pub(crate) fn command_for_draw_kind(kind: DrawToolKind) -> CommandId {
    match kind {
        DrawToolKind::Line => CommandId::CreateLine,
        // XLINE/RAY store one finite clipped segment each; the kind is what
        // distinguishes the two commands (and their stored `type_key`).
        DrawToolKind::XLine => CommandId::CreateXLine,
        DrawToolKind::Ray => CommandId::CreateRay,
        DrawToolKind::Circle => CommandId::CreateCircle,
        // The three CIRCLE construction options are their own commands (the
        // kind is what carries the construction choice).
        DrawToolKind::Circle2P => CommandId::CreateCircle2P,
        DrawToolKind::Circle3P => CommandId::CreateCircle3P,
        DrawToolKind::CircleDiameter => CommandId::CreateCircleDiameter,
        DrawToolKind::Arc => CommandId::CreateArc,
        DrawToolKind::Polyline => CommandId::CreatePolyline,
        DrawToolKind::Rectang => CommandId::CreateRectangle,
        DrawToolKind::Ellipse => CommandId::CreateEllipse,
        DrawToolKind::Polygon => CommandId::CreatePolygon,
        DrawToolKind::Point => CommandId::CreatePoint,
        DrawToolKind::Spline => CommandId::CreateSpline,
        DrawToolKind::Move => CommandId::MoveEntities,
        DrawToolKind::Trim => CommandId::TrimEntity,
        DrawToolKind::Copy => CommandId::CopyEntities,
        DrawToolKind::Extend => CommandId::ExtendEntity,
        DrawToolKind::Break => CommandId::Break,
        // BREAKATPOINT: entity pick, then a projected break point.
        DrawToolKind::BreakAtPoint => CommandId::BreakAtPoint,
        // The zero-point single-curve edit pair: the selection is the input and
        // the settings are the typed parameter channel.
        DrawToolKind::Divide => CommandId::Divide,
        DrawToolKind::Lengthen => CommandId::Lengthen,
        // SCALETEXT is the same zero-point pre-selection edit: the selection is
        // the input and the factor is the typed parameter channel.
        DrawToolKind::ScaleText => CommandId::ScaleText,
        // BLOCK: the selection snapshot plus one base point and the terminal
        // name define the new block definition and its reference.
        DrawToolKind::BlockDefine => CommandId::DefineBlock,
        DrawToolKind::Rotate => CommandId::RotateEntities,
        DrawToolKind::Scale => CommandId::ScaleEntities,
        DrawToolKind::Mirror => CommandId::MirrorEntities,
        DrawToolKind::Offset => CommandId::OffsetEntity,
        DrawToolKind::Chamfer => CommandId::ChamferLines,
        DrawToolKind::Fillet => CommandId::FilletLines,
        DrawToolKind::ArrayRect => CommandId::ArrayRectangular,
        DrawToolKind::ArrayPolar => CommandId::ArrayPolar,
        DrawToolKind::Align => CommandId::AlignEntities,
        DrawToolKind::Stretch => CommandId::StretchEntities,
        DrawToolKind::DimLinear => CommandId::DimLinear,
        DrawToolKind::DimAligned => CommandId::DimAligned,
        // DIMROTATED: the same three-pick capture as DIMLINEAR, with the
        // dimension-line angle from the session parameter channel.
        DrawToolKind::DimRotated => CommandId::DimRotated,
        DrawToolKind::DimRadius => CommandId::DimRadius,
        DrawToolKind::DimDiameter => CommandId::DimDiameter,
        DrawToolKind::DimCenter => CommandId::DimCenter,
        DrawToolKind::DimAngular => CommandId::DimAngular,
        DrawToolKind::DimOrdinate => CommandId::DimOrdinate,
        DrawToolKind::DimBaseline => CommandId::DimBaseline,
        DrawToolKind::DimContinue => CommandId::DimContinue,
        DrawToolKind::Hatch => CommandId::Hatch,
        DrawToolKind::HatchPoint => CommandId::HatchPoint,
        DrawToolKind::Text => CommandId::CreateText,
        // INSERT is point-then-name; the reference is created from existing
        // block metadata, never by writing the block table.
        DrawToolKind::InsertBlock => CommandId::InsertBlock,
        // MTEXT is the multiline create command (one `AcDbMText`).
        DrawToolKind::MText => CommandId::CreateMText,
        DrawToolKind::MLeader => CommandId::MLeader,
        DrawToolKind::TextEdit => CommandId::TextEdit,
        // COPYBASE is a read-only capture (Viewer-legal); CUTBASE is the same
        // capture for the work-only cutting command. The host reads the recorded
        // command to choose `CopyBase` vs `CutBase`.
        DrawToolKind::CopyBase => CommandId::CopyToClipboardBase,
        DrawToolKind::CutBase => CommandId::CutToClipboardBase,
        // PASTECLIP is work-only through its command (the capture itself needs
        // no selection).
        DrawToolKind::PasteClip => CommandId::PasteFromClipboard,
        // PASTEBLOCK writes a new block definition plus its reference; the
        // capture itself needs no selection (the clipboard is the input).
        DrawToolKind::PasteBlock => CommandId::PasteBlock,
    }
}
