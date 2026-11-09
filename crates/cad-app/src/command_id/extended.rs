//! The sustained "extended command" wrapper and its batches.
//!
//! [`ExtendedCommandId`] is deliberately **not** a second command funnel: each
//! variant is carried by the single [`crate::CommandId::Extended`] variant and
//! dispatched by `crate::app_dispatch`. That keeps the main `CommandId` enum
//! from growing one arm per payload-free helper while the vocabulary table still
//! names every selector.
//!
//! # The pilot batch (2026-11-09)
//!
//! All ten are **Viewer-legal session/report commands**: they open no
//! transaction, bump no revision and record no `cad-history` entry. Five are
//! session display toggles (`QTEXT`, `FILL`, `TFRAMES`, `REGENAUTO`,
//! `ISODRAFT`); their behaviour is not consumed by this build, so the handler
//! reports an honest partial instead of claiming a visual effect. Five are
//! read-only reports (`BCOUNT`, `COUNT`, `EDITTIME`, `JULIAN`, `XLIST`).
//!
//! # Wave 2 (2026-11-09)
//!
//! Thirty more payload-free names, grouped exactly as they are dispatched:
//!
//! * **Session tokens** (`DRAGMODE`, `APERTURE`, `ISOPLANE`, `HIGHLIGHTNEW`,
//!   `TRAYSETTINGS`, `TEXTSCR`, `GRAPHSCR`, `FINDINCLUDE`, `FINDMATCH`,
//!   `REVCLOUDPROPERTIES`): one session flag/token each, no database write and
//!   no renderer consumer; the two read-only forms (`APERTURE`,
//!   `REVCLOUDPROPERTIES`) report their token and name their setter as NOT
//!   IMPLEMENTED.
//! * **Read-only reports** (`SYSVARMONITOR`, `SYSVDLG`, `VTOPTIONS`,
//!   `ALIASEDIT`, `WHOHAS`, `ACADINFO`, `PTYPE`, `SCALELISTEDIT`): pure reads of
//!   data this build really has; anything absent is a named `not_tracked`/
//!   `write_unsupported` diagnostic, never an invented value.
//! * **Named refusals** (`HYPERLINKOPTIONS`, `INPUTSEARCHOPTIONS`,
//!   `-INPUTSEARCHOPTIONS`, `SIGVALIDATE`, `BASE`, `LIMITS`,
//!   `VIEWPLOTDETAILS`, `VPLAYER`, `MVIEW`, `PSBSCALE`, `PSTSCALE`,
//!   `MVSETUP`): `CadError::Unsupported` with a stable reason string, because
//!   the feature has no storage model or no API in this build. A refusal writes
//!   nothing, bumps no revision and records no history.
//!
//! # Wave 3 (2026-11-09)
//!
//! Thirty more payload-free names, again grouped by dispatch:
//!
//! * **Reports / session** (`MSPACE`, `PSPACE`, `STYLE`, `-STYLE`,
//!   `MLEADERSTYLE`, `BLOCK?`, `DIMREASSOC`, `DIMREASSOCIATE`): two space
//!   switches that reuse the existing `MODEL`/`SPACE` session path (never a
//!   second space authority), read-only style/block reports over the tables this
//!   build really stores, and two non-associative no-op reports mirroring
//!   `DIMREGEN`.
//! * **Named refusals** (`DIMJOGGED` through `CONSTRAINTSETTINGS`): features
//!   with no storage model or API in this build, refused with a stable
//!   `CadError::Unsupported` reason.
//!
//! Every variant in Waves 1–3 stays Viewer-legal: no database write, no
//! revision bump, no `cad-history` entry.
//!
//! # Wave 4 (2026-11-09)
//!
//! Thirty more names, and the first batch with **real entity writes**:
//!
//! * **Selection-based 2D edits** (`CENTERMARK`, `CENTERLINE`, `NCOPY`,
//!   `COPYTOLAYER`, `-COPYTOLAYER`, `CDORDER`, `FLATTEN`, `XPLODE`, `BTRIM`,
//!   `BEXTEND`, `CHANGE`, `CHPROP`): each reuses an existing planner/write path
//!   (creation funnel, `plan_trim`/`plan_extend`, `plan_entity`, EXPLODE). They
//!   are Work-only and commit exactly one transaction / one undo step.
//! * **Named refusals** (`BSCALE` through `CHSPACE`): features with no storage
//!   model or API in this build, refused with a stable
//!   `CadError::Unsupported` reason. Refusals stay Viewer-legal.
//!
//! # Wave 5 (2026-11-09)
//!
//! Thirty more names over the attribute and external-reference surfaces:
//!
//! * **Session token** (`ATTDISP`): one `attdisp` flag (default ON) in
//!   [`crate::session_state::DraftingExtras`]. This build has no block-attribute
//!   model, so nothing is rendered either way and the outcome says exactly that.
//! * **Read-only reports** (`BATTMAN`, `BATTORDER`, `XREF`, `-XREF`,
//!   `EXTERNALREFERENCES`, `EXTERNALREFERENCESCLOSE`): block definitions/member
//!   counts from `drawing.blocks()` and external-reference provenance from
//!   `drawing.external_references()`. Absent data is named
//!   (`*.attributes_not_modeled`, `externalreferences.palette_not_implemented`),
//!   never invented.
//! * **Named refusals** (`ATTDEF` through `CLASSICXREF`): no block-attribute
//!   model, no entity XDATA store, no xref bind/attach/edit/palette API. Each is
//!   `CadError::Unsupported` with a stable reason.
//!
//! Every Wave 5 variant stays Viewer-legal: no database write, no revision bump,
//! no `cad-history` entry.
//!
//! # Wave 6 (2026-11-09)
//!
//! Thirty more names over the file/plot/export surfaces:
//!
//! * **Session tokens** (`DWGLOG`, `LOGFILEON`, `LOGFILEOFF`): one `dwglog`
//!   flag (default OFF) in [`crate::session_state::DraftingExtras`]. This build
//!   writes no log file, so the handler reports the token and says exactly that.
//! * **Read-only reports** (`DWGPROPS`, `DWFFORMAT`, `EXPORTPDF`): document
//!   id/revision/entity counts, and two honest capability reports that name the
//!   absent export path instead of pretending to export. None is an error.
//! * **Named refusals** (`SAVEALL` through `AUTOPUBLISH`): no multi-document
//!   model, no reopen-to-source path, no plotter/plot-style/export-settings
//!   model, no plot preview/stamp, no batch publish/eTransmit/archive packaging.
//!   Each is `CadError::Unsupported` with a stable reason.
//!
//! Every Wave 6 variant stays Viewer-legal: no database write, no revision bump,
//! no `cad-history` entry, and no host-owned file I/O is re-implemented here.
//!
//! # Wave 7 (2026-11-09)
//!
//! Thirty more names over the dynamic-block and constraint surfaces:
//!
//! * **Read-only reports** (`BVSTATE`, `-BVSTATE`): the active dynamic-block
//!   visibility state of every directly selected INSERT, read through
//!   `DrawingDatabase::block_dynamic_visibility` (the read half of
//!   `set_block_visibility_state`). The report never sets a state; a selection
//!   with no INSERT is `*.no_selection`, and an INSERT whose block records no
//!   resolved state is `*.not_set` — nothing is invented.
//! * **Named refusals** (`BACTION` through `DCLINEAR`): no dynamic-block action,
//!   parameter, lookup-table, construction-geometry or visibility-hide/show
//!   model, no block authoring palette/editor, no parameter-reset API, and no
//!   geometric/dimensional constraint model. Each is `CadError::Unsupported`
//!   with a stable reason.
//!
//! Every Wave 7 variant stays Viewer-legal: no database write, no revision bump,
//! no `cad-history` entry.
//!
//! # Wave 8 (2026-11-09)
//!
//! `ELEV` (read-only session elevation report; setter NOT IMPLEMENTED) and
//! `UCSICON` (session flag, no icon) plus eight named refusals (`UCS`, `UCSMAN`,
//! `TABLET`, `TASKBAR`, `REINIT`, `SYSWINDOWS`, `MENU`, `RIBBON`) for the absent
//! UCS/tablet/menu models and host-owned chrome. All Viewer-legal.

/// One payload-free command in the extended wrapper batches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtendedCommandId {
    /// `QTEXT`: toggle the session quick-text display flag.
    QText,
    /// `FILL`: toggle the session solid-fill display flag.
    Fill,
    /// `TFRAMES`: toggle the session frame-display flag.
    TFrames,
    /// `REGENAUTO`: toggle the session automatic-regeneration flag.
    RegenAuto,
    /// `ISODRAFT`: toggle the session isometric-plane drafting flag.
    IsoDraft,
    /// `BCOUNT`: read-only block-instance count over model space.
    BlockCount,
    /// `COUNT`: read-only count of the selection or all model-space entities.
    CountObjects,
    /// `EDITTIME`: read-only clock/document report.
    EditTime,
    /// `JULIAN`: read-only Julian day number for the current UTC instant.
    JulianDate,
    /// `XLIST`: read-only block-definition list.
    XList,

    // -- Wave 2: session tokens (app_extended_session2) ----------------------
    /// `DRAGMODE`: toggle the session drag-preview flag (default ON).
    DragMode,
    /// `APERTURE`: read-only report of the session aperture token.
    Aperture,
    /// `ISOPLANE`: cycle the session isometric-plane token LEFT→TOP→RIGHT.
    IsoPlane,
    /// `HIGHLIGHTNEW`: toggle the session "highlight new features" flag.
    HighlightNew,
    /// `TRAYSETTINGS`: toggle the session status-tray settings flag.
    TraySettings,
    /// `TEXTSCR`: set the session `text_screen` token to ON.
    TextScreen,
    /// `GRAPHSCR`: set the session `text_screen` token to OFF.
    GraphScreen,
    /// `FINDINCLUDE`: toggle the session `find_include` token.
    FindInclude,
    /// `FINDMATCH`: toggle the session `find_match_case` token.
    FindMatch,
    /// `REVCLOUDPROPERTIES`: read-only report of the session revision-cloud arc
    /// length token.
    RevCloudProperties,

    // -- Wave 2: read-only reports (app_extended_reports2) -------------------
    /// `SYSVARMONITOR`: report the modelled `SETVAR` closed set.
    SysVarMonitor,
    /// `SYSVDLG`: the same closed set, plus dialog_not_implemented.
    SysVarDialog,
    /// `VTOPTIONS`: view-transition settings are not tracked.
    ViewTransitionOptions,
    /// `ALIASEDIT`: read-only list of the canonical alias table.
    AliasEdit,
    /// `WHOHAS`: single-document build report.
    WhoHas,
    /// `ACADINFO`: product/version/build + document revision/entity counts.
    AcadInfo,
    /// `PTYPE`: read-only `$PDMODE`/`$PDSIZE` report.
    PointType,
    /// `SCALELISTEDIT`: read-only paper-size report; the write is unsupported.
    ScaleListEdit,

    // -- Wave 2: named refusals (app_extended_refusals) ----------------------
    /// `HYPERLINKOPTIONS`: no hyperlink storage model.
    HyperlinkOptions,
    /// `INPUTSEARCHOPTIONS`: input-search settings are not modeled.
    InputSearchOptions,
    /// `-INPUTSEARCHOPTIONS`: the command-line spelling of the same refusal.
    DashInputSearchOptions,
    /// `SIGVALIDATE`: no digital-signature model.
    SigValidate,
    /// `BASE`: the drawing stores no insertion base point.
    BasePoint,
    /// `LIMITS`: no drawing-limits model.
    DrawingLimits,
    /// `VIEWPLOTDETAILS`: no plot-details model.
    ViewPlotDetails,
    /// `VPLAYER`: no per-viewport layer state.
    VPlayer,
    /// `MVIEW`: no viewport creation API.
    MView,
    /// `PSBSCALE`: no viewport scale write API.
    PsbScale,
    /// `PSTSCALE`: no viewport scale write API.
    PstScale,
    /// `MVSETUP`: no viewport-group model.
    MvSetup,

    // -- Wave 3: reports / session (app_extended_reports3) -------------------
    /// `MSPACE`: switch to model space through the existing `MODEL` path.
    Mspace,
    /// `PSPACE`: switch to the first paper layout through the existing `SPACE`
    /// path; a drawing with no paper layout is a named no-op.
    Pspace,
    /// `STYLE`: read-only list of the drawing's text styles.
    TextStyle,
    /// `-STYLE`: the same text-style list under its own diagnostic codes.
    DashStyle,
    /// `MLEADERSTYLE`: read-only list of the drawing's multileader styles.
    MLeaderStyleList,
    /// `BLOCK?`: read-only list of block definitions and their member counts.
    BlockList,
    /// `DIMREASSOC`: non-associative no-op report.
    DimReassoc,
    /// `DIMREASSOCIATE`: the same no-op report under its own code.
    DimReassociate,

    // -- Wave 3: named refusals (app_extended_refusals3) ---------------------
    /// `DIMJOGGED`: the dimension domain stores no jog field.
    DimJogged,
    /// `DIMINSPECT`: the dimension domain stores no inspection data.
    DimInspect,
    /// `-DIMINSPECT`: the command-line spelling of the same refusal.
    DashDimInspect,
    /// `ANNORESET`: no annotative-object model.
    AnnoReset,
    /// `ANNOUPDATE`: no annotative-object model.
    AnnoUpdate,
    /// `OBJECTSCALE`: no annotative-object model.
    ObjectScale,
    /// `-OBJECTSCALE`: the command-line spelling of the same refusal.
    DashObjectScale,
    /// `QLATTACH`: no QLEADER annotation-attachment model.
    QlAttach,
    /// `QLATTACHSET`: no QLEADER annotation-attachment model.
    QlAttachSet,
    /// `QLDETACHSET`: no QLEADER annotation-attachment model.
    QlDetachSet,
    /// `ARRAYCLOSE`: no associative-array edit state.
    ArrayClose,
    /// `ARRAYEDIT`: no associative-array edit state.
    ArrayEdit,
    /// `BEDIT`: no block editor.
    BlockEditor,
    /// `-BEDIT`: the command-line spelling of the same refusal.
    DashBlockEditor,
    /// `BCLOSE`: no block editor.
    BlockEditorClose,
    /// `BCYCLEORDER`: no dynamic-block grip model.
    BCycleOrder,
    /// `BGRIPSET`: no dynamic-block parameter model.
    BGripSet,
    /// `BESETTINGS`: no block editor.
    BEditorSettings,
    /// `BTESTBLOCK`: no block editor.
    BTestBlock,
    /// `LAYERPALETTE`: no separate layer palette (use `LAYER`).
    LayerPalette,
    /// `CONSTRAINTBAR`: no constraint model.
    ConstraintBar,
    /// `CONSTRAINTSETTINGS`: no constraint model.
    ConstraintSettings,

    // -- Wave 4: real selection-based 2D edits (app_extended_edit4) ----------
    /// `CENTERMARK`: crossing LINEs at the centre of each selected CIRCLE/ARC.
    CenterMark,
    /// `CENTERLINE`: one midway LINE between two parallel selected LINEs.
    CenterLine,
    /// `NCOPY`: copy block-instance member geometry into model space.
    NCopy,
    /// `COPYTOLAYER`: copy the selection onto the first selected entity's layer.
    CopyToLayer,
    /// `-COPYTOLAYER`: the command-line spelling of the same copy.
    DashCopyToLayer,
    /// `CDORDER`: rewrite the selection's draw order by colour index.
    CDOrder,
    /// `FLATTEN`: set the supported selected geometry's z to 0.
    Flatten,
    /// `XPLODE`: EXPLODE INSERTs with an attributes-not-implemented report.
    XPlode,
    /// `BTRIM`: trim the selection against the first selected INSERT's members.
    BTrim,
    /// `BEXTEND`: extend the selection to the first selected INSERT's members.
    BExtend,
    /// `CHANGE`: move the selection to the current layer and reset to ByLayer.
    Change,
    /// `CHPROP`: the same reset under its own selector.
    ChProp,

    // -- Wave 4: named refusals (app_extended_refusals4) --------------------
    /// `BSCALE`: needs a factor plus insertion reference.
    BScale,
    /// `MINSERT`: needs a block name plus row/column counts.
    MInsert,
    /// `BLOCKREPLACE`: needs two block definitions.
    BlockReplace,
    /// `BREPLACE`: needs two block definitions.
    BReplace,
    /// `-BREPLACE`: the command-line spelling of the same refusal.
    DashBReplace,
    /// `SPLINEDIT`: no NURBS edit surface.
    SplineEdit,
    /// `MLEDIT`: no MLINE join/cap edit model.
    MlEdit,
    /// `-MLEDIT`: the command-line spelling of the same refusal.
    DashMlEdit,
    /// `BURST`: no block-attribute model.
    Burst,
    /// `CLIP`: no image/xref clipping model.
    Clip,
    /// `CLIPIT`: no image/xref clipping model.
    ClipIt,
    /// `REGION`: no Region geometry model.
    Region,
    /// `ADDSELECTED`: no per-type creation prompts.
    AddSelected,
    /// `WBLOCK`: no separate drawing-file writer.
    WBlock,
    /// `-WBLOCK`: the command-line spelling of the same refusal.
    DashWBlock,
    /// `BSAVE`: no block editor.
    BSave,
    /// `BSAVEAS`: no block editor.
    BSaveAs,
    /// `CHSPACE`: no model/paper-space entity move.
    ChSpace,

    // -- Wave 5: session token / reports (app_extended_attrs5) --------------
    /// `ATTDISP`: toggle the session attribute-display token (default ON).
    AttDisp,
    /// `BATTMAN`: read-only block definition/member list plus the attribute gap.
    BattMan,
    /// `BATTORDER`: read-only block definition list plus the attribute gap.
    BattOrder,
    /// `XREF`: read-only report of the drawing's external references.
    XRef,
    /// `-XREF`: the same report under its own diagnostic codes.
    DashXRef,
    /// `EXTERNALREFERENCES`: the same report; this build has no palette.
    ExternalReferences,
    /// `EXTERNALREFERENCESCLOSE`: named no-op (there is no palette to close).
    ExternalReferencesClose,

    // -- Wave 5: named refusals (app_extended_refusals5) --------------------
    /// `ATTDEF`: no block-attribute model.
    AttDef,
    /// `-ATTDEF`: the command-line spelling of the same refusal.
    DashAttDef,
    /// `ATTEDIT`: no block-attribute model.
    AttEdit,
    /// `-ATTEDIT`: the command-line spelling of the same refusal.
    DashAttEdit,
    /// `ATTIPEDIT`: no block-attribute model.
    AttIpEdit,
    /// `ATTIN`: no block-attribute model.
    AttIn,
    /// `ATTOUT`: no block-attribute model.
    AttOut,
    /// `ATTEXT`: no block-attribute model.
    AttExt,
    /// `-ATTEXT`: the command-line spelling of the same refusal.
    DashAttExt,
    /// `EATTEDIT`: no block-attribute model.
    EAttEdit,
    /// `EATTEXT`: no block-attribute model.
    EAttExt,
    /// `-EATTEXT`: the command-line spelling of the same refusal.
    DashEAttExt,
    /// `ATTREDEF`: no block-attribute model.
    AttRedef,
    /// `ATTSYNC`: no block-attribute model.
    AttSync,
    /// `GATTE`: no block-attribute model.
    Gatte,
    /// `XDATA`: entity XDATA is not retained.
    XData,
    /// `XBIND`: no xref bind/insert model.
    XBind,
    /// `-XBIND`: the command-line spelling of the same refusal.
    DashXBind,
    /// `XATTACH`: external drawings are not attached as xrefs.
    XAttach,
    /// `ATTACH`: no external-reference/underlay attachment API.
    Attach,
    /// `-ATTACH`: the command-line spelling of the same refusal.
    DashAttach,
    /// `XOPEN`: no xref editing session.
    XOpen,
    /// `CLASSICXREF`: no separate xref palette (use the RESOURCES panel).
    ClassicXRef,

    // -- Wave 6: session tokens / reports (app_extended_file6) --------------
    /// `DWGLOG`: toggle the session drawing-log token (default OFF).
    DwgLog,
    /// `LOGFILEON`: set the session drawing-log token ON.
    LogFileOn,
    /// `LOGFILEOFF`: set the session drawing-log token OFF.
    LogFileOff,
    /// `DWGPROPS`: read-only document/entity report plus the metadata gap.
    DwgProps,
    /// `DWFFORMAT`: report that DWF is not an export format here.
    DwfFormat,
    /// `EXPORTPDF`: report that PDF export is host-owned through PLOT.
    ExportPdf,

    // -- Wave 6: named refusals (app_extended_refusals6) --------------------
    /// `SAVEALL`: this build has one document; use SAVE.
    SaveAll,
    /// `QNEW`: this build has one document; use NEW.
    QNew,
    /// `REVERT`: no reopen-to-source path.
    Revert,
    /// `CLOSEALL`: this build has one document; use CLOSE.
    CloseAll,
    /// `CLOSEALLOTHER`: only one document exists.
    CloseAllOther,
    /// `QQUIT`: application exit is host-owned.
    QQuit,
    /// `QUIT`: application exit is host-owned.
    Quit,
    /// `PROPULATE`: no property-propagation model.
    Propulate,
    /// `PLOTTERMANAGER`: no plotter configuration model.
    PlotterManager,
    /// `STYLESMANAGER`: no CTB/STB plot-style model.
    StylesManager,
    /// `EXPORTSETTINGS`: no export-settings model.
    ExportSettings,
    /// `PREVIEW`: no plot-preview window.
    Preview,
    /// `EXPORTLAYOUT`: use PLOT (one layout, SVG/PDF).
    ExportLayout,
    /// `PLOTSTAMP`: no plot-stamp model.
    PlotStamp,
    /// `-PLOTSTAMP`: the command-line spelling of the same refusal.
    DashPlotStamp,
    /// `-PLOT`: use PLOT; this build has one vector plot path.
    DashPlot,
    /// `-EXPORT`: use PLOT/EXPORT; this build has one vector export path.
    DashExport,
    /// `ETRANSMIT`: no eTransmit/zip packaging model.
    ETransmit,
    /// `-ETRANSMIT`: the command-line spelling of the same refusal.
    DashETransmit,
    /// `ARCHIVE`: no sheet-set archive packaging model.
    Archive,
    /// `-ARCHIVE`: the command-line spelling of the same refusal.
    DashArchive,
    /// `PUBLISH`: no batch publish model.
    Publish,
    /// `-PUBLISH`: the command-line spelling of the same refusal.
    DashPublish,
    /// `AUTOPUBLISH`: no automatic publish model.
    AutoPublish,

    // -- Wave 7: read-only dynamic-block visibility reports (dyn7) ----------
    /// `BVSTATE`: read-only active visibility state of each selected INSERT.
    BvState,
    /// `-BVSTATE`: the same report under its own diagnostic codes.
    DashBvState,

    // -- Wave 7: named refusals (app_extended_refusals7) --------------------
    /// `BACTION`: no dynamic-block action model.
    BAction,
    /// `BACTIONBAR`: no dynamic-block action model.
    BActionBar,
    /// `BACTIONSET`: no dynamic-block action model.
    BActionSet,
    /// `BACTIONTOOL`: no dynamic-block action model.
    BActionTool,
    /// `BASSOCIATE`: no dynamic-block action model.
    BAssociate,
    /// `BAUTHORPALETTE`: no block editor.
    BAuthorPalette,
    /// `BAUTHORPALETTECLOSE`: no block editor.
    BAuthorPaletteClose,
    /// `BCONSTRUCTION`: no dynamic-block construction geometry.
    BConstruction,
    /// `BCPARAMETER`: no dynamic-block parameter model.
    BCParameter,
    /// `-BCPARAMETER`: the command-line spelling of the same refusal.
    DashBCParameter,
    /// `BPARAMETER`: no dynamic-block parameter model.
    BParameter,
    /// `BLOOKUPTABLE`: no dynamic-block lookup table.
    BLookupTable,
    /// `BTABLE`: no dynamic-block lookup table.
    BTable,
    /// `BVHIDE`: no per-visibility-state entity hide API.
    BvHide,
    /// `BVSHOW`: no per-visibility-state entity show API.
    BvShow,
    /// `RESETBLOCK`: no dynamic-block parameter reset API.
    ResetBlock,
    /// `AUTOCONSTRAIN`: no geometric-constraint model.
    AutoConstrain,
    /// `DELCONSTRAINT`: no geometric-constraint model.
    DelConstraint,
    /// `DIMCONSTRAINT`: no dimensional-constraint model.
    DimConstraint,
    /// `GEOMCONSTRAINT`: no geometric-constraint model.
    GeomConstraint,
    /// `DCALIGNED`: no dimensional-constraint model.
    DcAligned,
    /// `DCANGULAR`: no dimensional-constraint model.
    DcAngular,
    /// `DCCONVERT`: no dimensional-constraint model.
    DcConvert,
    /// `DCDIAMETER`: no dimensional-constraint model.
    DcDiameter,
    /// `DCDISPLAY`: no dimensional-constraint model.
    DcDisplay,
    /// `DCFORM`: no dimensional-constraint model.
    DcForm,
    /// `DCHORIZONTAL`: no dimensional-constraint model.
    DcHorizontal,
    /// `DCLINEAR`: no dimensional-constraint model.
    DcLinear,

    // Wave 8: two session tokens and eight named refusals (all Viewer-legal).
    Elev,
    UcsIcon,
    Ucs,
    UcsMan,
    Tablet,
    TaskBar,
    Reinit,
    SysWindows,
    Menu,
    Ribbon,
}

impl ExtendedCommandId {
    /// The stable uppercase selector (the name in `cad-ui-core`'s
    /// `COMMAND_TABLE`).
    pub fn selector(self) -> &'static str {
        match self {
            Self::QText => "QTEXT",
            Self::Fill => "FILL",
            Self::TFrames => "TFRAMES",
            Self::RegenAuto => "REGENAUTO",
            Self::IsoDraft => "ISODRAFT",
            Self::BlockCount => "BCOUNT",
            Self::CountObjects => "COUNT",
            Self::EditTime => "EDITTIME",
            Self::JulianDate => "JULIAN",
            Self::XList => "XLIST",
            Self::DragMode => "DRAGMODE",
            Self::Aperture => "APERTURE",
            Self::IsoPlane => "ISOPLANE",
            Self::HighlightNew => "HIGHLIGHTNEW",
            Self::TraySettings => "TRAYSETTINGS",
            Self::TextScreen => "TEXTSCR",
            Self::GraphScreen => "GRAPHSCR",
            Self::FindInclude => "FINDINCLUDE",
            Self::FindMatch => "FINDMATCH",
            Self::RevCloudProperties => "REVCLOUDPROPERTIES",
            Self::SysVarMonitor => "SYSVARMONITOR",
            Self::SysVarDialog => "SYSVDLG",
            Self::ViewTransitionOptions => "VTOPTIONS",
            Self::AliasEdit => "ALIASEDIT",
            Self::WhoHas => "WHOHAS",
            Self::AcadInfo => "ACADINFO",
            Self::PointType => "PTYPE",
            Self::ScaleListEdit => "SCALELISTEDIT",
            Self::HyperlinkOptions => "HYPERLINKOPTIONS",
            Self::InputSearchOptions => "INPUTSEARCHOPTIONS",
            Self::DashInputSearchOptions => "-INPUTSEARCHOPTIONS",
            Self::SigValidate => "SIGVALIDATE",
            Self::BasePoint => "BASE",
            Self::DrawingLimits => "LIMITS",
            Self::ViewPlotDetails => "VIEWPLOTDETAILS",
            Self::VPlayer => "VPLAYER",
            Self::MView => "MVIEW",
            Self::PsbScale => "PSBSCALE",
            Self::PstScale => "PSTSCALE",
            Self::MvSetup => "MVSETUP",
            Self::Mspace => "MSPACE",
            Self::Pspace => "PSPACE",
            Self::TextStyle => "STYLE",
            Self::DashStyle => "-STYLE",
            Self::MLeaderStyleList => "MLEADERSTYLE",
            Self::BlockList => "BLOCK?",
            Self::DimReassoc => "DIMREASSOC",
            Self::DimReassociate => "DIMREASSOCIATE",
            Self::DimJogged => "DIMJOGGED",
            Self::DimInspect => "DIMINSPECT",
            Self::DashDimInspect => "-DIMINSPECT",
            Self::AnnoReset => "ANNORESET",
            Self::AnnoUpdate => "ANNOUPDATE",
            Self::ObjectScale => "OBJECTSCALE",
            Self::DashObjectScale => "-OBJECTSCALE",
            Self::QlAttach => "QLATTACH",
            Self::QlAttachSet => "QLATTACHSET",
            Self::QlDetachSet => "QLDETACHSET",
            Self::ArrayClose => "ARRAYCLOSE",
            Self::ArrayEdit => "ARRAYEDIT",
            Self::BlockEditor => "BEDIT",
            Self::DashBlockEditor => "-BEDIT",
            Self::BlockEditorClose => "BCLOSE",
            Self::BCycleOrder => "BCYCLEORDER",
            Self::BGripSet => "BGRIPSET",
            Self::BEditorSettings => "BESETTINGS",
            Self::BTestBlock => "BTESTBLOCK",
            Self::LayerPalette => "LAYERPALETTE",
            Self::ConstraintBar => "CONSTRAINTBAR",
            Self::ConstraintSettings => "CONSTRAINTSETTINGS",
            Self::CenterMark => "CENTERMARK",
            Self::CenterLine => "CENTERLINE",
            Self::NCopy => "NCOPY",
            Self::CopyToLayer => "COPYTOLAYER",
            Self::DashCopyToLayer => "-COPYTOLAYER",
            Self::CDOrder => "CDORDER",
            Self::Flatten => "FLATTEN",
            Self::XPlode => "XPLODE",
            Self::BTrim => "BTRIM",
            Self::BExtend => "BEXTEND",
            Self::Change => "CHANGE",
            Self::ChProp => "CHPROP",
            Self::BScale => "BSCALE",
            Self::MInsert => "MINSERT",
            Self::BlockReplace => "BLOCKREPLACE",
            Self::BReplace => "BREPLACE",
            Self::DashBReplace => "-BREPLACE",
            Self::SplineEdit => "SPLINEDIT",
            Self::MlEdit => "MLEDIT",
            Self::DashMlEdit => "-MLEDIT",
            Self::Burst => "BURST",
            Self::Clip => "CLIP",
            Self::ClipIt => "CLIPIT",
            Self::Region => "REGION",
            Self::AddSelected => "ADDSELECTED",
            Self::WBlock => "WBLOCK",
            Self::DashWBlock => "-WBLOCK",
            Self::BSave => "BSAVE",
            Self::BSaveAs => "BSAVEAS",
            Self::ChSpace => "CHSPACE",
            Self::AttDisp => "ATTDISP",
            Self::BattMan => "BATTMAN",
            Self::BattOrder => "BATTORDER",
            Self::XRef => "XREF",
            Self::DashXRef => "-XREF",
            Self::ExternalReferences => "EXTERNALREFERENCES",
            Self::ExternalReferencesClose => "EXTERNALREFERENCESCLOSE",
            Self::AttDef => "ATTDEF",
            Self::DashAttDef => "-ATTDEF",
            Self::AttEdit => "ATTEDIT",
            Self::DashAttEdit => "-ATTEDIT",
            Self::AttIpEdit => "ATTIPEDIT",
            Self::AttIn => "ATTIN",
            Self::AttOut => "ATTOUT",
            Self::AttExt => "ATTEXT",
            Self::DashAttExt => "-ATTEXT",
            Self::EAttEdit => "EATTEDIT",
            Self::EAttExt => "EATTEXT",
            Self::DashEAttExt => "-EATTEXT",
            Self::AttRedef => "ATTREDEF",
            Self::AttSync => "ATTSYNC",
            Self::Gatte => "GATTE",
            Self::XData => "XDATA",
            Self::XBind => "XBIND",
            Self::DashXBind => "-XBIND",
            Self::XAttach => "XATTACH",
            Self::Attach => "ATTACH",
            Self::DashAttach => "-ATTACH",
            Self::XOpen => "XOPEN",
            Self::ClassicXRef => "CLASSICXREF",
            Self::DwgLog => "DWGLOG",
            Self::LogFileOn => "LOGFILEON",
            Self::LogFileOff => "LOGFILEOFF",
            Self::DwgProps => "DWGPROPS",
            Self::DwfFormat => "DWFFORMAT",
            Self::ExportPdf => "EXPORTPDF",
            Self::SaveAll => "SAVEALL",
            Self::QNew => "QNEW",
            Self::Revert => "REVERT",
            Self::CloseAll => "CLOSEALL",
            Self::CloseAllOther => "CLOSEALLOTHER",
            Self::QQuit => "QQUIT",
            Self::Quit => "QUIT",
            Self::Propulate => "PROPULATE",
            Self::PlotterManager => "PLOTTERMANAGER",
            Self::StylesManager => "STYLESMANAGER",
            Self::ExportSettings => "EXPORTSETTINGS",
            Self::Preview => "PREVIEW",
            Self::ExportLayout => "EXPORTLAYOUT",
            Self::PlotStamp => "PLOTSTAMP",
            Self::DashPlotStamp => "-PLOTSTAMP",
            Self::DashPlot => "-PLOT",
            Self::DashExport => "-EXPORT",
            Self::ETransmit => "ETRANSMIT",
            Self::DashETransmit => "-ETRANSMIT",
            Self::Archive => "ARCHIVE",
            Self::DashArchive => "-ARCHIVE",
            Self::Publish => "PUBLISH",
            Self::DashPublish => "-PUBLISH",
            Self::AutoPublish => "AUTOPUBLISH",
            Self::BvState => "BVSTATE",
            Self::DashBvState => "-BVSTATE",
            Self::BAction => "BACTION",
            Self::BActionBar => "BACTIONBAR",
            Self::BActionSet => "BACTIONSET",
            Self::BActionTool => "BACTIONTOOL",
            Self::BAssociate => "BASSOCIATE",
            Self::BAuthorPalette => "BAUTHORPALETTE",
            Self::BAuthorPaletteClose => "BAUTHORPALETTECLOSE",
            Self::BConstruction => "BCONSTRUCTION",
            Self::BCParameter => "BCPARAMETER",
            Self::DashBCParameter => "-BCPARAMETER",
            Self::BParameter => "BPARAMETER",
            Self::BLookupTable => "BLOOKUPTABLE",
            Self::BTable => "BTABLE",
            Self::BvHide => "BVHIDE",
            Self::BvShow => "BVSHOW",
            Self::ResetBlock => "RESETBLOCK",
            Self::AutoConstrain => "AUTOCONSTRAIN",
            Self::DelConstraint => "DELCONSTRAINT",
            Self::DimConstraint => "DIMCONSTRAINT",
            Self::GeomConstraint => "GEOMCONSTRAINT",
            Self::DcAligned => "DCALIGNED",
            Self::DcAngular => "DCANGULAR",
            Self::DcConvert => "DCCONVERT",
            Self::DcDiameter => "DCDIAMETER",
            Self::DcDisplay => "DCDISPLAY",
            Self::DcForm => "DCFORM",
            Self::DcHorizontal => "DCHORIZONTAL",
            Self::DcLinear => "DCLINEAR",
            Self::Elev => "ELEV",
            Self::UcsIcon => "UCSICON",
            Self::Ucs => "UCS",
            Self::UcsMan => "UCSMAN",
            Self::Tablet => "TABLET",
            Self::TaskBar => "TASKBAR",
            Self::Reinit => "REINIT",
            Self::SysWindows => "SYSWINDOWS",
            Self::Menu => "MENU",
            Self::Ribbon => "RIBBON",
        }
    }

    /// Whether the command needs Work mode.
    ///
    /// Every variant in these batches is session-only, a read-only report or a
    /// named refusal, so it is Viewer-legal. This stays a method (rather than a
    /// constant) so a later write-capable extended command can declare its own
    /// authorization here.
    pub fn requires_work_mode(self) -> bool {
        matches!(
            self,
            Self::CenterMark
                | Self::CenterLine
                | Self::NCopy
                | Self::CopyToLayer
                | Self::DashCopyToLayer
                | Self::CDOrder
                | Self::Flatten
                | Self::XPlode
                | Self::BTrim
                | Self::BExtend
                | Self::Change
                | Self::ChProp
        )
    }

    /// Every variant in declaration order; the vocabulary contract test uses it
    /// to prove the table and the enum cannot drift.
    pub const ALL: &'static [ExtendedCommandId] = &[
        Self::QText,
        Self::Fill,
        Self::TFrames,
        Self::RegenAuto,
        Self::IsoDraft,
        Self::BlockCount,
        Self::CountObjects,
        Self::EditTime,
        Self::JulianDate,
        Self::XList,
        Self::DragMode,
        Self::Aperture,
        Self::IsoPlane,
        Self::HighlightNew,
        Self::TraySettings,
        Self::TextScreen,
        Self::GraphScreen,
        Self::FindInclude,
        Self::FindMatch,
        Self::RevCloudProperties,
        Self::SysVarMonitor,
        Self::SysVarDialog,
        Self::ViewTransitionOptions,
        Self::AliasEdit,
        Self::WhoHas,
        Self::AcadInfo,
        Self::PointType,
        Self::ScaleListEdit,
        Self::HyperlinkOptions,
        Self::InputSearchOptions,
        Self::DashInputSearchOptions,
        Self::SigValidate,
        Self::BasePoint,
        Self::DrawingLimits,
        Self::ViewPlotDetails,
        Self::VPlayer,
        Self::MView,
        Self::PsbScale,
        Self::PstScale,
        Self::MvSetup,
        Self::Mspace,
        Self::Pspace,
        Self::TextStyle,
        Self::DashStyle,
        Self::MLeaderStyleList,
        Self::BlockList,
        Self::DimReassoc,
        Self::DimReassociate,
        Self::DimJogged,
        Self::DimInspect,
        Self::DashDimInspect,
        Self::AnnoReset,
        Self::AnnoUpdate,
        Self::ObjectScale,
        Self::DashObjectScale,
        Self::QlAttach,
        Self::QlAttachSet,
        Self::QlDetachSet,
        Self::ArrayClose,
        Self::ArrayEdit,
        Self::BlockEditor,
        Self::DashBlockEditor,
        Self::BlockEditorClose,
        Self::BCycleOrder,
        Self::BGripSet,
        Self::BEditorSettings,
        Self::BTestBlock,
        Self::LayerPalette,
        Self::ConstraintBar,
        Self::ConstraintSettings,
        Self::CenterMark,
        Self::CenterLine,
        Self::NCopy,
        Self::CopyToLayer,
        Self::DashCopyToLayer,
        Self::CDOrder,
        Self::Flatten,
        Self::XPlode,
        Self::BTrim,
        Self::BExtend,
        Self::Change,
        Self::ChProp,
        Self::BScale,
        Self::MInsert,
        Self::BlockReplace,
        Self::BReplace,
        Self::DashBReplace,
        Self::SplineEdit,
        Self::MlEdit,
        Self::DashMlEdit,
        Self::Burst,
        Self::Clip,
        Self::ClipIt,
        Self::Region,
        Self::AddSelected,
        Self::WBlock,
        Self::DashWBlock,
        Self::BSave,
        Self::BSaveAs,
        Self::ChSpace,
        Self::AttDisp,
        Self::BattMan,
        Self::BattOrder,
        Self::XRef,
        Self::DashXRef,
        Self::ExternalReferences,
        Self::ExternalReferencesClose,
        Self::AttDef,
        Self::DashAttDef,
        Self::AttEdit,
        Self::DashAttEdit,
        Self::AttIpEdit,
        Self::AttIn,
        Self::AttOut,
        Self::AttExt,
        Self::DashAttExt,
        Self::EAttEdit,
        Self::EAttExt,
        Self::DashEAttExt,
        Self::AttRedef,
        Self::AttSync,
        Self::Gatte,
        Self::XData,
        Self::XBind,
        Self::DashXBind,
        Self::XAttach,
        Self::Attach,
        Self::DashAttach,
        Self::XOpen,
        Self::ClassicXRef,
        Self::DwgLog,
        Self::LogFileOn,
        Self::LogFileOff,
        Self::DwgProps,
        Self::DwfFormat,
        Self::ExportPdf,
        Self::SaveAll,
        Self::QNew,
        Self::Revert,
        Self::CloseAll,
        Self::CloseAllOther,
        Self::QQuit,
        Self::Quit,
        Self::Propulate,
        Self::PlotterManager,
        Self::StylesManager,
        Self::ExportSettings,
        Self::Preview,
        Self::ExportLayout,
        Self::PlotStamp,
        Self::DashPlotStamp,
        Self::DashPlot,
        Self::DashExport,
        Self::ETransmit,
        Self::DashETransmit,
        Self::Archive,
        Self::DashArchive,
        Self::Publish,
        Self::DashPublish,
        Self::AutoPublish,
        Self::BvState,
        Self::DashBvState,
        Self::BAction,
        Self::BActionBar,
        Self::BActionSet,
        Self::BActionTool,
        Self::BAssociate,
        Self::BAuthorPalette,
        Self::BAuthorPaletteClose,
        Self::BConstruction,
        Self::BCParameter,
        Self::DashBCParameter,
        Self::BParameter,
        Self::BLookupTable,
        Self::BTable,
        Self::BvHide,
        Self::BvShow,
        Self::ResetBlock,
        Self::AutoConstrain,
        Self::DelConstraint,
        Self::DimConstraint,
        Self::GeomConstraint,
        Self::DcAligned,
        Self::DcAngular,
        Self::DcConvert,
        Self::DcDiameter,
        Self::DcDisplay,
        Self::DcForm,
        Self::DcHorizontal,
        Self::DcLinear,
        Self::Elev,
        Self::UcsIcon,
        Self::Ucs,
        Self::UcsMan,
        Self::Tablet,
        Self::TaskBar,
        Self::Reinit,
        Self::SysWindows,
        Self::Menu,
        Self::Ribbon,
    ];
}
