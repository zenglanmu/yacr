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
                | Self::Wipeout
                | Self::CreateRectangle
                | Self::CreateEllipse
                | Self::CreatePolygon
                | Self::CreatePolygonCircumscribed
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
                | Self::DimRotated
                | Self::DimSpace
                // The 2026-11-07 dimension-edit batch: these four write entity
                // rows. DIMSTYLE's reports, DIMREGEN/DIMDISASSOCIATE no-ops and
                // the DIMJOGLINE/DIMBREAK capability refusals never write, so
                // they stay Viewer-legal.
                | Self::DimTextEdit
                | Self::DimEdit
                | Self::DimOverride
                | Self::QDim
                // The 2026-11-07 annotation batch: the leader/text/hatch forms
                // write entity rows; GROUP/UNGROUP/GROUPEDIT are session-only and
                // XDLIST is a read-only report, so they stay Viewer-legal.
                | Self::QLeader
                | Self::Leader
                | Self::TextFit
                | Self::HatchSetBoundary
                | Self::HatchSetOrigin
                | Self::Hatch
                | Self::HatchPoint
                // BOUNDARY writes new polylines from the discovered loops.
                | Self::Boundary
                | Self::HatchGenerateBoundary
                | Self::HatchToBack
                | Self::Gradient
                | Self::HatchEdit
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
                | Self::LayThw
                | Self::LayLck
                | Self::LayUlk
                | Self::SetByLayer
                // The 2026-11-07 layer batch: these write entity rows or the
                // layer table. LAYERPMODE is session-only and LAYWALK is a
                // session view operation, so neither is work-only.
                | Self::LayMCh
                | Self::LayMrg
                | Self::LayDel
                | Self::LayerState
                | Self::SetLinetypeScale
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
                | Self::Reverse
                | Self::Pedit
                | Self::Oops
                | Self::Overkill
                | Self::BreakAtPoint
                | Self::Torient
                | Self::TCase
                | Self::Txt2MText
                // The 2026-11-07 2D text batch: every one writes the drawing.
                | Self::JustifyText
                | Self::TextScale
                | Self::TextAlignEdges
                | Self::TextCount
                | Self::TextCircle
                | Self::TextMask
                | Self::TextUnmask
                // TEXTTOFRONT rewrites stored model-space draw order.
                | Self::TextToFront
                // FIND can replace text in one transaction, so the whole
                // selector is Work-only; the Viewer-legal report is the separate
                // `-FIND` selector. The selection/filter and hide-objects batch
                // is session state only, so it is absent too.
                | Self::FindReplace
                // The utility batch: PURGE/RENAME write named-object tables and
                // MREDO moves history; SETVAR/TIME/DBLIST/TREESTAT/MULTIPLE are
                // read-only or session-only and stay Viewer-legal.
                | Self::Purge
                | Self::Rename
                | Self::MRedo
                // The 2026-11-07 draw/modify batch. Every one writes drawing
                // entities except MLSTYLE, whose store is session state only.
                | Self::Donut
                | Self::Solid
                | Self::Helix
                | Self::Mline
                | Self::Trex
                | Self::Extrim
                | Self::Mpedit
                | Self::DrawOrder
                // The 2026-11-07 layout batch. `Layout` stays Viewer-legal for
                // LIST/SET; its write forms refuse by name without writing.
                // VPMAX/VPMIN are session-only and also stay Viewer-legal.
                | Self::LayoutMerge
                | Self::LayoutWizard
                | Self::Vports
                | Self::VpClip
                | Self::VpScale
                | Self::LayVpi
        )
    }
}
