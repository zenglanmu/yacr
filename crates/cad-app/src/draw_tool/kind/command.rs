//! The one `DrawToolKind → CommandId` mapping.
//!
//! Split out of the catalogue (`kind.rs`) and the capture state machine
//! (`session_state.rs`) so both readers share one table: the capture records the
//! command it belongs to ([`crate::SessionState::draw_command`]) and the
//! `MULTIPLE` repeat loop compares a completed capture against its armed command.
//! A kind is not always one command — `COPYBASE`/`CUTBASE` share the
//! `CopyBase` shape, `CIRCLE` has three construction variants — which is why the
//! command, not the kind, is the stored identity.

use super::DrawToolKind;
use crate::{CommandId, ExtendedCommandId};

impl DrawToolKind {
    /// The command this capture commits for.
    ///
    /// Every capture kind maps to exactly one admitted command; the command is
    /// recorded on the capture and is what authorizes it and what the repeat
    /// loop matches.
    pub fn command(self) -> CommandId {
        match self {
            Self::Line => CommandId::CreateLine,
            // XLINE/RAY store one finite clipped segment each; the kind is what
            // distinguishes the two commands (and their stored `type_key`).
            Self::XLine => CommandId::CreateXLine,
            Self::Ray => CommandId::CreateRay,
            Self::Circle => CommandId::CreateCircle,
            // The three CIRCLE options are their own commands.
            Self::Circle2P => CommandId::CreateCircle2P,
            Self::Circle3P => CommandId::CreateCircle3P,
            Self::CircleDiameter => CommandId::CreateCircleDiameter,
            Self::Arc => CommandId::CreateArc,
            Self::Polyline => CommandId::CreatePolyline,
            // FREEHAND commits through the existing `SKETCH` write: one open
            // polyline from the sampled hand path.
            Self::Freehand => CommandId::Sketch,
            // WIPEOUT stores one `Mask` entity (`AcDbWipeout`).
            Self::Wipeout => CommandId::Wipeout,
            Self::Rectang => CommandId::CreateRectangle,
            Self::Ellipse => CommandId::CreateEllipse,
            Self::Polygon => CommandId::CreatePolygon,
            // POLYGON CIRCUMSCRIBED is the tangent variant.
            Self::PolygonCircumscribed => CommandId::CreatePolygonCircumscribed,
            Self::Point => CommandId::CreatePoint,
            Self::Spline => CommandId::CreateSpline,
            Self::Move => CommandId::MoveEntities,
            Self::Trim => CommandId::TrimEntity,
            Self::Copy => CommandId::CopyEntities,
            Self::Extend => CommandId::ExtendEntity,
            Self::Break => CommandId::Break,
            // BREAKATPOINT: entity pick, then a projected break point.
            Self::BreakAtPoint => CommandId::BreakAtPoint,
            // The zero-point single-curve edit pair: the selection is the input and
            // the settings are the typed parameter channel.
            Self::Divide => CommandId::Divide,
            Self::Lengthen => CommandId::Lengthen,
            // SCALETEXT is the same zero-point pre-selection edit: the selection is
            // the input and the factor is the typed parameter channel.
            Self::ScaleText => CommandId::ScaleText,
            // BLOCK: the selection snapshot plus one base point and the terminal
            // name define the new block definition and its reference.
            Self::BlockDefine => CommandId::DefineBlock,
            Self::Rotate => CommandId::RotateEntities,
            Self::Scale => CommandId::ScaleEntities,
            Self::Mirror => CommandId::MirrorEntities,
            Self::Offset => CommandId::OffsetEntity,
            Self::Chamfer => CommandId::ChamferLines,
            Self::Fillet => CommandId::FilletLines,
            Self::ArrayRect => CommandId::ArrayRectangular,
            Self::ArrayPolar => CommandId::ArrayPolar,
            Self::Align => CommandId::AlignEntities,
            Self::Stretch => CommandId::StretchEntities,
            Self::DimLinear => CommandId::DimLinear,
            Self::DimAligned => CommandId::DimAligned,
            Self::DimRadius => CommandId::DimRadius,
            Self::DimDiameter => CommandId::DimDiameter,
            Self::DimCenter => CommandId::DimCenter,
            Self::DimAngular => CommandId::DimAngular,
            Self::DimOrdinate => CommandId::DimOrdinate,
            Self::DimBaseline => CommandId::DimBaseline,
            Self::DimContinue => CommandId::DimContinue,
            Self::DimRotated => CommandId::DimRotated,
            Self::Hatch => CommandId::Hatch,
            Self::HatchPoint => CommandId::HatchPoint,
            // BOUNDARY: one internal point, shared discovery, new polylines.
            Self::Boundary => CommandId::Boundary,
            Self::ChamferEdge => CommandId::Extended(ExtendedCommandId::ChamferEdge),
            Self::PressPull => CommandId::Extended(ExtendedCommandId::PressPull),
            Self::Extrude => CommandId::Extended(ExtendedCommandId::Extrude),
            Self::FollowMe => CommandId::Extended(ExtendedCommandId::FollowMe),
            Self::RectangRotated => CommandId::Extended(ExtendedCommandId::RectangRotated),
            Self::Pie => CommandId::Extended(ExtendedCommandId::Pie),
            Self::Text => CommandId::CreateText,
            // INSERT is point-then-name; the reference is created from existing
            // block metadata, never by writing the block table.
            Self::InsertBlock => CommandId::InsertBlock,
            // MTEXT is the multiline create command (one `AcDbMText`).
            Self::MText => CommandId::CreateMText,
            Self::MLeader => CommandId::MLeader,
            Self::TextEdit => CommandId::TextEdit,
            // COPYBASE is a read-only capture (Viewer-legal); CUTBASE is the same
            // capture for the work-only cutting command. The host reads the recorded
            // command to choose `CopyBase` vs `CutBase`.
            Self::CopyBase => CommandId::CopyToClipboardBase,
            Self::CutBase => CommandId::CutToClipboardBase,
            // PASTECLIP is work-only through its command (the capture itself needs
            // no selection).
            Self::PasteClip => CommandId::PasteFromClipboard,
            // PASTEBLOCK writes a new block definition plus its reference; the
            // capture itself needs no selection (the clipboard is the input).
            Self::PasteBlock => CommandId::PasteBlock,
        }
    }

    /// Whether `command` starts a repeatable interactive capture.
    ///
    /// `MULTIPLE` can only repeat a command that re-opens a capture after each
    /// confirm; one-shot and measurement commands have no capture to repeat and
    /// are refused by name.
    pub fn is_capture_command(command: CommandId) -> bool {
        Self::ALL.iter().any(|kind| kind.command() == command)
    }
}
