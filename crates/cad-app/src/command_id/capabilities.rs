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
                | Self::CreateCircle2P
                | Self::CreateCircle3P
                | Self::CreateCircleDiameter
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
                | Self::HatchPoint
                | Self::MoveEntities
                | Self::TrimEntity
                | Self::ExtendEntity
                | Self::Break
                // The single-curve edit pair (2026-11-01): both write entities.
                | Self::Divide
                | Self::Lengthen
                | Self::ChamferLines
                | Self::FilletLines
                | Self::ChamferPolyline
                | Self::FilletPolyline
                | Self::RevisionCloud
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
                // PASTEBLOCK writes the block table and inserts a reference.
                | Self::PasteBlock
                | Self::OffsetEntity
                | Self::SetActiveLayer
                | Self::CreateLayer
                | Self::RenameLayer
                | Self::DeleteLayer
                | Self::SetLayerProperty
                | Self::LayCur
                | Self::LayMCur
                | Self::LayFrz
                | Self::SetPointDisplayMode
                | Self::SetPointDisplaySize
                | Self::SetEntityProperty
                | Self::MatchProp
                | Self::GripStretch
                // INSERT writes one new block-reference entity; the block table
                // itself is read-only here (no BLOCK/WBLOCK).
                | Self::InsertBlock
                // SCALETEXT rewrites the selected text entities' glyph height.
                | Self::ScaleText
                // BLOCK writes the block table, moves the selected entities into
                // the new definition and inserts a reference.
                | Self::DefineBlock
                // XLINE/RAY write one finite clipped segment each.
                | Self::CreateXLine
                | Self::CreateRay
        )
    }
}
