//! Dispatch for the sustained extended-command wrapper.
//!
//! `crate::CommandId::Extended` is one enum arm; this module routes it to the
//! module that owns the authority: the pilot session toggles in `app_drafting`,
//! the pilot read-only reports in `app_reports_ext`, the Wave 2 handlers in
//! `app_extended_session2` (tokens), `app_extended_reports2` (reports) and
//! `app_extended_refusals` (named refusals), and the Wave 3 handlers in
//! `app_extended_reports3` (reports/session) and `app_extended_refusals3`
//! (named refusals), and the Wave 4 handlers in `app_extended_edit4` (real
//! edits) and `app_extended_refusals4` (named refusals). Wave 5 adds
//! `app_extended_attrs5` (ATTDISP token plus attribute/xref reports) and
//! `app_extended_refusals5` (named refusals); Wave 6 adds
//! `app_extended_file6` (drawing-log token plus file/plot/export reports) and
//! `app_extended_refusals6` (named refusals). Wave 7 adds `app_extended_dyn7`
//! (read-only dynamic-block visibility reports) and `app_extended_refusals7`
//! (named refusals). Wave 8 adds `app_extended_session8` (session tokens) and
//! `app_extended_refusals8` (named refusals). Keeping the routing here means
//! `application.rs` stays a one-line interception rather than growing a hundred
//! more arms.

use super::*;

impl Application {
    /// Dispatch an [`ExtendedCommandId`], or `None` when the command is not one.
    ///
    /// The caller has already authorized the command and checked the stale
    /// document, so every handler can assume a live session.
    pub(crate) fn dispatch_extended(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<Option<CommandOutcome>> {
        let CommandId::Extended(ext) = command.id else {
            return Ok(None);
        };
        let outcome = match ext {
            ExtendedCommandId::QText
            | ExtendedCommandId::Fill
            | ExtendedCommandId::TFrames
            | ExtendedCommandId::RegenAuto
            | ExtendedCommandId::IsoDraft => self.drafting_toggle(session, command, ext)?,
            ExtendedCommandId::BlockCount
            | ExtendedCommandId::CountObjects
            | ExtendedCommandId::EditTime
            | ExtendedCommandId::JulianDate
            | ExtendedCommandId::XList => self.extended_report(session, command, ext)?,
            // Wave 2 session tokens: each flips or reports one session token.
            ExtendedCommandId::DragMode
            | ExtendedCommandId::Aperture
            | ExtendedCommandId::IsoPlane
            | ExtendedCommandId::HighlightNew
            | ExtendedCommandId::TraySettings
            | ExtendedCommandId::TextScreen
            | ExtendedCommandId::GraphScreen
            | ExtendedCommandId::FindInclude
            | ExtendedCommandId::FindMatch
            | ExtendedCommandId::RevCloudProperties => {
                self.extended_session2_toggle(session, command, ext)?
            }
            // Wave 2 read-only reports: no transaction, no revision, no history.
            ExtendedCommandId::SysVarMonitor
            | ExtendedCommandId::SysVarDialog
            | ExtendedCommandId::ViewTransitionOptions
            | ExtendedCommandId::AliasEdit
            | ExtendedCommandId::WhoHas
            | ExtendedCommandId::AcadInfo
            | ExtendedCommandId::PointType
            | ExtendedCommandId::ScaleListEdit => self.extended_report2(session, command, ext)?,
            // Wave 2 named refusals: a stable `Unsupported` reason per name.
            ExtendedCommandId::HyperlinkOptions
            | ExtendedCommandId::InputSearchOptions
            | ExtendedCommandId::DashInputSearchOptions
            | ExtendedCommandId::SigValidate
            | ExtendedCommandId::BasePoint
            | ExtendedCommandId::DrawingLimits
            | ExtendedCommandId::ViewPlotDetails
            | ExtendedCommandId::VPlayer
            | ExtendedCommandId::PsbScale
            | ExtendedCommandId::PstScale
            | ExtendedCommandId::MvSetup => self.extended_refusal(command, ext)?,
            // Wave 3 reports/session: two space switches plus read-only reports.
            ExtendedCommandId::Mspace
            | ExtendedCommandId::Pspace
            | ExtendedCommandId::TextStyle
            | ExtendedCommandId::DashStyle
            | ExtendedCommandId::MLeaderStyleList
            | ExtendedCommandId::BlockList
            | ExtendedCommandId::DimReassoc
            | ExtendedCommandId::DimReassociate => self.extended_report3(session, command, ext)?,
            // Wave 3 named refusals: a stable `Unsupported` reason per name.
            ExtendedCommandId::DimJogged
            | ExtendedCommandId::DimInspect
            | ExtendedCommandId::DashDimInspect
            | ExtendedCommandId::AnnoReset
            | ExtendedCommandId::AnnoUpdate
            | ExtendedCommandId::ObjectScale
            | ExtendedCommandId::DashObjectScale
            | ExtendedCommandId::QlAttach
            | ExtendedCommandId::QlAttachSet
            | ExtendedCommandId::QlDetachSet
            | ExtendedCommandId::ArrayClose
            | ExtendedCommandId::ArrayEdit
            | ExtendedCommandId::BlockEditor
            | ExtendedCommandId::DashBlockEditor
            | ExtendedCommandId::BlockEditorClose
            | ExtendedCommandId::BCycleOrder
            | ExtendedCommandId::BGripSet
            | ExtendedCommandId::BEditorSettings
            | ExtendedCommandId::BTestBlock
            | ExtendedCommandId::LayerPalette
            | ExtendedCommandId::ConstraintBar
            | ExtendedCommandId::ConstraintSettings => self.extended_refusal3(command, ext)?,
            // Wave 4 real selection-based 2D edits: each writes entity rows and
            // is Work-only; one transaction / one undo step per command.
            ExtendedCommandId::CenterMark
            | ExtendedCommandId::CenterLine
            | ExtendedCommandId::NCopy
            | ExtendedCommandId::CopyToLayer
            | ExtendedCommandId::DashCopyToLayer
            | ExtendedCommandId::CDOrder
            | ExtendedCommandId::Flatten
            | ExtendedCommandId::XPlode
            | ExtendedCommandId::BTrim
            | ExtendedCommandId::BExtend
            | ExtendedCommandId::Change
            | ExtendedCommandId::ChProp => self.extended_edit4(session, command, ext)?,
            // Wave 4 named refusals: a stable `Unsupported` reason per name.
            ExtendedCommandId::BScale
            | ExtendedCommandId::MInsert
            | ExtendedCommandId::BlockReplace
            | ExtendedCommandId::BReplace
            | ExtendedCommandId::DashBReplace
            | ExtendedCommandId::SplineEdit
            | ExtendedCommandId::MlEdit
            | ExtendedCommandId::DashMlEdit
            | ExtendedCommandId::Burst
            | ExtendedCommandId::Clip
            | ExtendedCommandId::ClipIt
            | ExtendedCommandId::Region
            | ExtendedCommandId::AddSelected
            | ExtendedCommandId::WBlock
            | ExtendedCommandId::DashWBlock
            | ExtendedCommandId::BSave
            | ExtendedCommandId::BSaveAs
            | ExtendedCommandId::ChSpace => self.extended_refusal4(command, ext)?,
            // Wave 5 session token + attribute/xref reports: ATTDISP flips one
            // session flag; the rest read the block and xref tables this build
            // really stores. All are Viewer-legal.
            ExtendedCommandId::AttDisp
            | ExtendedCommandId::BattMan
            | ExtendedCommandId::BattOrder
            | ExtendedCommandId::XRef
            | ExtendedCommandId::DashXRef
            | ExtendedCommandId::ExternalReferences
            | ExtendedCommandId::ExternalReferencesClose => {
                self.extended_attrs5(session, command, ext)?
            }
            // Wave 5 named refusals: a stable `Unsupported` reason per name.
            ExtendedCommandId::AttDef
            | ExtendedCommandId::DashAttDef
            | ExtendedCommandId::AttEdit
            | ExtendedCommandId::DashAttEdit
            | ExtendedCommandId::AttIpEdit
            | ExtendedCommandId::AttIn
            | ExtendedCommandId::AttOut
            | ExtendedCommandId::AttExt
            | ExtendedCommandId::DashAttExt
            | ExtendedCommandId::EAttEdit
            | ExtendedCommandId::EAttExt
            | ExtendedCommandId::DashEAttExt
            | ExtendedCommandId::AttRedef
            | ExtendedCommandId::AttSync
            | ExtendedCommandId::Gatte
            | ExtendedCommandId::XData
            | ExtendedCommandId::XBind
            | ExtendedCommandId::DashXBind
            | ExtendedCommandId::XAttach
            | ExtendedCommandId::Attach
            | ExtendedCommandId::DashAttach
            | ExtendedCommandId::XOpen
            | ExtendedCommandId::ClassicXRef => self.extended_refusal5(command, ext)?,
            // Wave 6 session tokens + file/plot/export reports: DWGLOG/LOGFILEON/
            // LOGFILEOFF flip one session flag (no log file is written); the
            // three reports read the document and name the absent export models.
            // All are Viewer-legal.
            ExtendedCommandId::DwgLog
            | ExtendedCommandId::LogFileOn
            | ExtendedCommandId::LogFileOff
            | ExtendedCommandId::DwgProps
            | ExtendedCommandId::DwfFormat
            | ExtendedCommandId::ExportPdf => self.extended_file6(session, command, ext)?,
            // Wave 6 named refusals: a stable `Unsupported` reason per name.
            ExtendedCommandId::SaveAll
            | ExtendedCommandId::QNew
            | ExtendedCommandId::Revert
            | ExtendedCommandId::CloseAll
            | ExtendedCommandId::CloseAllOther
            | ExtendedCommandId::QQuit
            | ExtendedCommandId::Quit
            | ExtendedCommandId::Propulate
            | ExtendedCommandId::PlotterManager
            | ExtendedCommandId::StylesManager
            | ExtendedCommandId::ExportSettings
            | ExtendedCommandId::Preview
            | ExtendedCommandId::ExportLayout
            | ExtendedCommandId::PlotStamp
            | ExtendedCommandId::DashPlotStamp
            | ExtendedCommandId::DashPlot
            | ExtendedCommandId::DashExport
            | ExtendedCommandId::ETransmit
            | ExtendedCommandId::DashETransmit
            | ExtendedCommandId::Archive
            | ExtendedCommandId::DashArchive
            | ExtendedCommandId::Publish
            | ExtendedCommandId::DashPublish
            | ExtendedCommandId::AutoPublish => self.extended_refusal6(command, ext)?,
            // Wave 7 read-only dynamic-block visibility reports: BVSTATE and
            // -BVSTATE read the active state of each selected INSERT through
            // the existing read accessor; they never set a state.
            ExtendedCommandId::BvState | ExtendedCommandId::DashBvState => {
                self.extended_dyn7(session, command, ext)?
            }
            // Wave 7 named refusals: a stable `Unsupported` reason per name.
            ExtendedCommandId::BAction
            | ExtendedCommandId::BActionBar
            | ExtendedCommandId::BActionSet
            | ExtendedCommandId::BActionTool
            | ExtendedCommandId::BAssociate
            | ExtendedCommandId::BAuthorPalette
            | ExtendedCommandId::BAuthorPaletteClose
            | ExtendedCommandId::BConstruction
            | ExtendedCommandId::BCParameter
            | ExtendedCommandId::DashBCParameter
            | ExtendedCommandId::BParameter
            | ExtendedCommandId::BLookupTable
            | ExtendedCommandId::BTable
            | ExtendedCommandId::BvHide
            | ExtendedCommandId::BvShow
            | ExtendedCommandId::ResetBlock
            | ExtendedCommandId::AutoConstrain
            | ExtendedCommandId::DelConstraint
            | ExtendedCommandId::DimConstraint
            | ExtendedCommandId::GeomConstraint
            | ExtendedCommandId::DcAligned
            | ExtendedCommandId::DcAngular
            | ExtendedCommandId::DcConvert
            | ExtendedCommandId::DcDiameter
            | ExtendedCommandId::DcDisplay
            | ExtendedCommandId::DcForm
            | ExtendedCommandId::DcHorizontal
            | ExtendedCommandId::DcLinear => self.extended_refusal7(command, ext)?,
            // Wave 8 session tokens: ELEV reports one token, UCSICON flips one.
            // Both are session-only and never write the drawing.
            ExtendedCommandId::Elev | ExtendedCommandId::UcsIcon => {
                self.extended_session8(session, command, ext)?
            }
            // Wave 8 named refusals: a stable `Unsupported` reason per name.
            ExtendedCommandId::Ucs
            | ExtendedCommandId::UcsMan
            | ExtendedCommandId::Tablet
            | ExtendedCommandId::TaskBar
            | ExtendedCommandId::Reinit
            | ExtendedCommandId::SysWindows
            | ExtendedCommandId::Menu
            | ExtendedCommandId::Ribbon => self.extended_refusal8(command, ext)?,
            // Wave 9: 2D draw/modify, real edits, aliases and reports.
            ExtendedCommandId::Array
            | ExtendedCommandId::DashArray
            | ExtendedCommandId::ArrayClassic
            | ExtendedCommandId::ArcText
            | ExtendedCommandId::Blend
            | ExtendedCommandId::BreakLine
            | ExtendedCommandId::MLeaderAlign
            | ExtendedCommandId::MLeaderEdit
            | ExtendedCommandId::MLeaderCollect
            | ExtendedCommandId::Tolerance
            | ExtendedCommandId::DimRouter
            | ExtendedCommandId::DimArc
            | ExtendedCommandId::Calc => self.extended_w9_draw(session, command, ext)?,
            ExtendedCommandId::DashBlock
            | ExtendedCommandId::DashBoundary
            | ExtendedCommandId::DashInsert
            | ExtendedCommandId::DashHatch
            | ExtendedCommandId::DashHatchEdit
            | ExtendedCommandId::ExOffset => self.extended_w9_alias(session, command, ext)?,
            ExtendedCommandId::DimExport
            | ExtendedCommandId::DimImport
            | ExtendedCommandId::SpaceTrans
            | ExtendedCommandId::TextExplode
            | ExtendedCommandId::UpdateField
            | ExtendedCommandId::Units => self.extended_w9_reports(session, command, ext)?,
            // Wave 10: layer/view/session reports and named refusals.
            ExtendedCommandId::DashLayer
            | ExtendedCommandId::LayerStateSave
            | ExtendedCommandId::NewView
            | ExtendedCommandId::DashScaleListEdit
            | ExtendedCommandId::CopyHist => self.extended_w10_layer(session, command, ext)?,
            ExtendedCommandId::DashUnits
            | ExtendedCommandId::LineWeight
            | ExtendedCommandId::DashLineWeight
            | ExtendedCommandId::Color
            | ExtendedCommandId::DashColor => self.extended_w10_session(session, command, ext)?,
            ExtendedCommandId::Help
            | ExtendedCommandId::ClassicInsert
            | ExtendedCommandId::ClassicGroup
            | ExtendedCommandId::FileTab
            | ExtendedCommandId::FileTabClose
            | ExtendedCommandId::HidePalettes
            | ExtendedCommandId::ShowPalettes
            | ExtendedCommandId::RibbonClose
            | ExtendedCommandId::GoToStart
            | ExtendedCommandId::Transparency => {
                self.extended_w10_refusals(session, command, ext)?
            }
            // Wave 11: stable named refusals for 3D/mesh/surface commands.
            ExtendedCommandId::ThreeDAlign
            | ExtendedCommandId::ThreeDArray
            | ExtendedCommandId::ThreeDClip
            | ExtendedCommandId::ThreeDCorbit
            | ExtendedCommandId::ThreeDDistance
            | ExtendedCommandId::ThreeDDwf
            | ExtendedCommandId::ThreeDEditBar
            | ExtendedCommandId::ThreeDFace
            | ExtendedCommandId::ThreeDFly
            | ExtendedCommandId::ThreeDForbit
            | ExtendedCommandId::ThreeDMesh
            | ExtendedCommandId::ThreeDMove
            | ExtendedCommandId::ThreeDOrbit
            | ExtendedCommandId::ThreeDOrbitCtr
            | ExtendedCommandId::Dash3DOsnap
            | ExtendedCommandId::ThreeDObjectSnap
            | ExtendedCommandId::ThreeDPan
            | ExtendedCommandId::ThreeDPoly
            | ExtendedCommandId::ThreeDPrint
            | ExtendedCommandId::ThreeDPrintService
            | ExtendedCommandId::ThreeDRotate
            | ExtendedCommandId::ThreeDScale
            | ExtendedCommandId::ThreeDSwivel
            | ExtendedCommandId::ThreeDWalk
            | ExtendedCommandId::ThreeDZoom => self.extended_w11_refusals(command, ext)?,
            // Wave 12: file/import/export/extension named refusals.
            ExtendedCommandId::ThreeDsIn
            | ExtendedCommandId::AcisIn
            | ExtendedCommandId::AcisOut
            | ExtendedCommandId::BmpOut
            | ExtendedCommandId::JpgOut
            | ExtendedCommandId::PngOut
            | ExtendedCommandId::TifOut
            | ExtendedCommandId::WmfIn
            | ExtendedCommandId::WmfOut
            | ExtendedCommandId::IgesImport
            | ExtendedCommandId::IgesExport
            | ExtendedCommandId::StlOut
            | ExtendedCommandId::ImportFile
            | ExtendedCommandId::ExportToAutoCad
            | ExtendedCommandId::DwgConvert
            | ExtendedCommandId::Plt2Dwg
            | ExtendedCommandId::DxbIn
            | ExtendedCommandId::AppLoad
            | ExtendedCommandId::Arx
            | ExtendedCommandId::NetLoad
            | ExtendedCommandId::VbaIde
            | ExtendedCommandId::VbaLoad
            | ExtendedCommandId::VbaRun
            | ExtendedCommandId::VbaUnload
            | ExtendedCommandId::Script => self.extended_w12_refusals(command, ext)?,
            // Wave 13: page-setup writes/reports, the MVIEW viewport write, and
            // read-only conversion reports.
            ExtendedCommandId::MView => self.mview(session, command)?,
            ExtendedCommandId::PageSetup
            | ExtendedCommandId::DashPageSetup
            | ExtendedCommandId::PageSetupIn
            | ExtendedCommandId::DashPageSetupIn
            | ExtendedCommandId::ConvertPoly => self.extended_w13_reports(session, command, ext)?,
            // TABLE / -TABLE create one real table; TABLEDIT edits one selected
            // table's cell text. Each is one transaction / one undo step.
            ExtendedCommandId::TableCreate
            | ExtendedCommandId::DashTable
            | ExtendedCommandId::TableEdit => self.table_edit(session, command, ext)?,
            ExtendedCommandId::PlotStyleList
            | ExtendedCommandId::DashPlotStyleList
            | ExtendedCommandId::ConvertPlotStyles
            | ExtendedCommandId::ConvertCtb
            | ExtendedCommandId::ConvertGeneric
            | ExtendedCommandId::TableExport
            | ExtendedCommandId::TableStyleList
            | ExtendedCommandId::TableInsert
            | ExtendedCommandId::MatchCell
            | ExtendedCommandId::Field
            | ExtendedCommandId::RText
            | ExtendedCommandId::RTextEdit
            | ExtendedCommandId::Spell
            | ExtendedCommandId::SuperHatch
            | ExtendedCommandId::AlignSpace
            | ExtendedCommandId::VpSync
            | ExtendedCommandId::LayTrans => self.extended_w13_refusals(session, command, ext)?,
            // Wave 14: stable named refusals.
            ExtendedCommandId::Box
            | ExtendedCommandId::Cone
            | ExtendedCommandId::Cylinder
            | ExtendedCommandId::Sphere
            | ExtendedCommandId::Torus
            | ExtendedCommandId::Wedge
            | ExtendedCommandId::Pyramid
            | ExtendedCommandId::PolySolid
            | ExtendedCommandId::Extrude
            | ExtendedCommandId::Revolve
            | ExtendedCommandId::Sweep
            | ExtendedCommandId::Loft
            | ExtendedCommandId::Intersect
            | ExtendedCommandId::Union
            | ExtendedCommandId::Subtract
            | ExtendedCommandId::Slice
            | ExtendedCommandId::Thicken
            | ExtendedCommandId::PressPull
            | ExtendedCommandId::Imprint
            | ExtendedCommandId::OffsetEdge
            | ExtendedCommandId::ChamferEdge
            | ExtendedCommandId::FilletEdge
            | ExtendedCommandId::Edge
            | ExtendedCommandId::Mirror3D
            | ExtendedCommandId::Rotate3D => self.extended_w14_refusals(command, ext)?,
            // Wave 15: stable named refusals.
            ExtendedCommandId::SurfBlend
            | ExtendedCommandId::SurfExtend
            | ExtendedCommandId::SurfExtractCurve
            | ExtendedCommandId::SurfFillet
            | ExtendedCommandId::SurfNetwork
            | ExtendedCommandId::SurfOffset
            | ExtendedCommandId::SurfPatch
            | ExtendedCommandId::SurfSculpt
            | ExtendedCommandId::SurfTrim
            | ExtendedCommandId::SurfUntrim
            | ExtendedCommandId::MeshCreate
            | ExtendedCommandId::MeshCap
            | ExtendedCommandId::MeshCollapse
            | ExtendedCommandId::MeshCrease
            | ExtendedCommandId::MeshExtrude
            | ExtendedCommandId::MeshMerge
            | ExtendedCommandId::MeshOptions
            | ExtendedCommandId::MeshPrimitiveOptions
            | ExtendedCommandId::MeshRefine
            | ExtendedCommandId::MeshSmooth
            | ExtendedCommandId::MeshSmoothLess
            | ExtendedCommandId::MeshSmoothMore
            | ExtendedCommandId::MeshSpin
            | ExtendedCommandId::MeshSplit
            | ExtendedCommandId::MeshUncrease => self.extended_w15_refusals(command, ext)?,
            // Wave 16: the raster-image report and its three real authoring
            // commands; the remaining names are stable refusals.
            ExtendedCommandId::Image | ExtendedCommandId::DashImage => {
                self.extended_w16_reports(session, command, ext)?
            }
            ExtendedCommandId::ImageAttach
            | ExtendedCommandId::ImageClip
            | ExtendedCommandId::ImageAdjust
            | ExtendedCommandId::DashImageAdjust => self.image_edit(session, command, ext)?,
            ExtendedCommandId::ImageQuality
            | ExtendedCommandId::ImageApp
            | ExtendedCommandId::ImageAsyncWait
            | ExtendedCommandId::ImageEdit
            | ExtendedCommandId::ClassicImage
            | ExtendedCommandId::PdfAttach
            | ExtendedCommandId::DashPdfAttach
            | ExtendedCommandId::PdfClip
            | ExtendedCommandId::PdfLayers
            | ExtendedCommandId::PdfAdjust
            | ExtendedCommandId::PdfImport
            | ExtendedCommandId::DashPdfImport
            | ExtendedCommandId::PdfShxText
            | ExtendedCommandId::DashPdfShxText
            | ExtendedCommandId::DgnAttach
            | ExtendedCommandId::DashDgnAttach
            | ExtendedCommandId::DgnClip
            | ExtendedCommandId::DgnLayers
            | ExtendedCommandId::DgnAdjust => self.extended_w16_refusals(command, ext)?,
            // Waves 17-24: the data-driven named refusals (see `RefusalId`).
            ExtendedCommandId::Refusal(name) => self.extended_refusal_named(command, name)?,
        };
        Ok(Some(outcome))
    }
}
