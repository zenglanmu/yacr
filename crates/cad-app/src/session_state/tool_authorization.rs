//! The Work-only command each session tool parameter is authorized against.
//!
//! Split out of `session_state.rs` to keep that module inside the code-size limit;
//! the single mapping function keeps its original body and is re-exported by the
//! parent so the parameter channel's authorization is unchanged.

use super::*;

/// The Work-only command a [`ToolParameter`] is authorized against.
///
/// `PolygonSides` keeps its original authorization (`CreatePolygon`) exactly as
/// before the migration. `FilletRadius` and the two `ChamferDistance`s belong to
/// the FILLET/CHAMFER edit commands (ADR 0016 decisions B/C); the array
/// parameters belong to the ARRAYRECT/ARRAYPOLAR edit commands (ADR 0016 decision
/// D). Each parameter is authorized against its own command, so a Viewer-mode set
/// is refused exactly like the create commands.
pub(crate) fn authorizing_command(parameter: ToolParameter) -> CommandId {
    match parameter {
        ToolParameter::PolygonSides => CommandId::CreatePolygon,
        ToolParameter::FilletRadius => CommandId::FilletLines,
        ToolParameter::ChamferDistanceA | ToolParameter::ChamferDistanceB => {
            CommandId::ChamferLines
        }
        ToolParameter::ArrayRows
        | ToolParameter::ArrayColumns
        | ToolParameter::ArrayRowSpacing
        | ToolParameter::ArrayColumnSpacing => CommandId::ArrayRectangular,
        ToolParameter::ArrayItems
        | ToolParameter::ArrayFillAngle
        | ToolParameter::ArrayRotateItems => CommandId::ArrayPolar,
        // ALIGN (P4d): `AlignScale` is authorized against the ALIGN edit command,
        // so a Viewer-mode set is refused exactly like the other edit parameters.
        ToolParameter::AlignScale => CommandId::AlignEntities,
        // HATCH (P6b): every hatch parameter is authorized against the HATCH
        // create command, so a Viewer-mode set is refused exactly like the create
        // commands.
        ToolParameter::HatchPatternType
        | ToolParameter::HatchPattern
        | ToolParameter::HatchSpacing
        | ToolParameter::HatchAngle
        | ToolParameter::HatchDouble
        | ToolParameter::HatchStyle
        | ToolParameter::HatchGradientKind
        | ToolParameter::HatchGradientAngle
        | ToolParameter::HatchGradientColorA
        | ToolParameter::HatchGradientColorB => CommandId::Hatch,
        // TEXT (P7a): the glyph height is authorized against the TEXT create
        // command, so a Viewer-mode set is refused exactly like the other create
        // parameters. MTEXT captures with the **same** parameter (its only height
        // channel) and `CreateMText` is Work-only too, so this one authorization is
        // exactly the gate MTEXT needs — there is no second height parameter and no
        // per-kind authorization to drift.
        ToolParameter::TextHeight => CommandId::CreateText,
        // TEXT creation also owns the rotation: the `Text` payload has no rotation
        // field, so `textRotation` is read straight from this channel by
        // `create_text`. MTEXT consumes the same parameter (it has no rotation
        // parameter of its own), and `CreateMText` is Work-only too, so the one
        // authorization is exactly the gate both create commands need.
        ToolParameter::TextRotation => CommandId::CreateText,
        // MTEXT attachment is MTEXT-only, so it is authorized against the MTEXT
        // create command (`CreateMText` is Work-only, like every other parameter
        // command here).
        ToolParameter::MTextAttachment => CommandId::CreateMText,
    }
}
