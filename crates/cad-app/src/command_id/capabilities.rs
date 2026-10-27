//! Command-level authorization attributes.
//!
//! Split out of the command catalogue to keep `command_id.rs` inside the
//! code-size limit; the `impl` block applies to the same type, so the public path
//! ([`crate::CommandId::requires_work_mode`]) is unchanged.

use super::CommandId;

impl CommandId {
    pub fn requires_work_mode(self) -> bool {
        matches!(
            self,
            Self::Measure
                | Self::ConfirmMeasurement
                | Self::AreaObject
                | Self::Undo
                | Self::Redo
                | Self::CreateLine
                | Self::CreateCircle
                | Self::CreateArc
                | Self::CreatePolyline
                | Self::CreateRectangle
                | Self::CreateEllipse
                | Self::CreatePolygon
                | Self::CreatePoint
                | Self::CreateSpline
                | Self::CreateText
                | Self::CreateMText
                | Self::MLeader
                | Self::TextEdit
                | Self::DimLinear
                | Self::DimAligned
                | Self::DimRadius
                | Self::DimDiameter
                | Self::DimCenter
                | Self::DimAngular
                | Self::DimOrdinate
                | Self::DimBaseline
                | Self::DimContinue
                | Self::Hatch
                | Self::MoveEntities
                | Self::TrimEntity
                | Self::ExtendEntity
                | Self::ChamferLines
                | Self::FilletLines
                | Self::ArrayRectangular
                | Self::ArrayPolar
                | Self::AlignEntities
                | Self::StretchEntities
                | Self::EraseEntities
                | Self::Join
                | Self::ExplodeEntities
                | Self::CopyEntities
                | Self::RotateEntities
                | Self::ScaleEntities
                | Self::MirrorEntities
                | Self::CutToClipboard
                // `COPYBASE` is deliberately **absent**: it is a read-only capture
                // (like `COPYCLIP`), so Viewer mode may use it. Its cutting sibling
                // and both pastes are work-only.
                | Self::CutToClipboardBase
                | Self::PasteFromClipboard
                | Self::PasteOriginalCoordinates
                | Self::OffsetEntity
                | Self::SetActiveLayer
                | Self::CreateLayer
                | Self::RenameLayer
                | Self::DeleteLayer
                | Self::SetLayerProperty
                | Self::SetPointDisplayMode
                | Self::SetPointDisplaySize
                | Self::SetEntityProperty
                | Self::MatchProp
                | Self::GripStretch
        )
    }
}
