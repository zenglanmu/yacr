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
//!   `VIEWPLOTDETAILS`, `VPLAYER`, `PSBSCALE`, `PSTSCALE`,
//!   `MVSETUP`): `CadError::Unsupported` with a stable reason string, because
//!   the feature has no storage model or no API in this build. A refusal writes
//!   nothing, bumps no revision and records no history. (`MVIEW` graduated to a
//!   real Wave 13 viewport write and is no longer a refusal.)
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

mod refusal_names;
mod selectors;

pub use refusal_names::RefusalId;

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
    /// `SYSVARMONITOR`: add/remove/list the session watch set.
    SysVarMonitor,
    /// `SYSVDLG`: the same closed set, plus dialog_not_implemented.
    SysVarDialog,
    /// `VTOPTIONS`: view-transition settings are not tracked.
    ViewTransitionOptions,
    /// `ALIASEDIT`: read-only list of the build's canonical alias table (runtime
    /// editing is the session-only `ALIASEDIT ADD|REMOVE|LIST` command line).
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
    /// `MVIEW`: append one rectangular paper viewport to the active layout from
    /// a two-corner `Points` payload (Work-only write).
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
    /// `MINSERT`: insert an existing block as a rectangular array of independent
    /// INSERT references (`app_drawing::minsert`).
    MInsert,
    /// `BLOCKREPLACE`: needs two block definitions.
    BlockReplace,
    /// `BREPLACE`: needs two block definitions.
    BReplace,
    /// `-BREPLACE`: the command-line spelling of the same refusal.
    DashBReplace,
    /// `SPLINEDIT`: no NURBS edit surface.
    SplineEdit,
    /// `MLEDIT`: the two-LINE subset of multiline editing
    /// (`app_drawing::mledit`).
    MlEdit,
    /// `-MLEDIT`: the command-line spelling of the same edit.
    DashMlEdit,
    /// `BURST`: no block-attribute model.
    Burst,
    /// `CLIP`: no image/xref clipping model.
    Clip,
    /// `CLIPIT`: no image/xref clipping model.
    ClipIt,
    /// `REGION`: no Region geometry model.
    Region,
    /// `ADDSELECTED`: create a new entity of the single selected entity's type
    /// and general properties at the typed point (`app_drawing::addselected`).
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
    /// `CLASSICXREF`: read-only xref report plus the chrome-local classic panel.
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

    // Wave 9: 2D draw/modify routes, real edits, and named refusals.
    /// `ARRAY`: route a typed array payload to the existing ARRAYRECT/ARRAYPOLAR planner; no legacy dialog.
    Array,
    /// `-ARRAY`: the command-line spelling of the same array router.
    DashArray,
    /// `ARRAYCLASSIC`: the legacy dialog is NOT IMPLEMENTED; routes to the ARRAY planner when a typed payload is supplied.
    ArrayClassic,
    /// `ARCTEXT`: text-along-arc is NOT IMPLEMENTED (no combined arc+text payload model).
    ArcText,
    /// `BLEND`: one cubic SPLINE bridging two open LINE/POLYLINE/ARC ends; tangents from the source curves.
    Blend,
    /// `BREAKLINE`: one open polyline with a break symbol between two picked points.
    BreakLine,
    /// `MLEADERALIGN`: align/space the selected MULTILEADERs' text
    /// (`app_drawing::mleader_edit`).
    MLeaderAlign,
    /// `MLEADEREDIT`: add/remove one leader line on the selected MULTILEADERs
    /// (`app_drawing::mleader_edit`).
    MLeaderEdit,
    /// `MLEADERCOLLECT`: collect the selected MULTILEADERs into one
    /// (`app_drawing::mleader_edit`).
    MLeaderCollect,
    /// `TOLERANCE`: one feature-control frame as separate border/divider/TEXT
    /// rows (`app_drawing::tolerance`); no single `AcDbFcf` object is stored.
    Tolerance,
    /// `DIM`: read-only router report listing the implemented DIM* creation commands; no dimension is created.
    DimRouter,
    /// `DIMARC`: arc-length dimensions are NOT IMPLEMENTED (the dimension domain has no arc-length kind).
    DimArc,
    /// `CAL`: expression evaluation is NOT IMPLEMENTED; reports the missing evaluator and writes nothing.
    Calc,
    /// `-BLOCK`: the command-line spelling of `BLOCK`; re-dispatches to the existing block-definition path.
    DashBlock,
    /// `-BOUNDARY`: the command-line spelling of `BOUNDARY`; re-dispatches to the existing boundary path.
    DashBoundary,
    /// `-INSERT`: the command-line spelling of `INSERT`; re-dispatches to the existing block-insert path.
    DashInsert,
    /// `-HATCH`: the command-line spelling of `HATCH`; re-dispatches to the existing hatch path.
    DashHatch,
    /// `-HATCHEDIT`: the command-line spelling of `HATCHEDIT`; re-dispatches to the existing hatch-edit path.
    DashHatchEdit,
    /// `EXOFFSET`: the Express offset name; re-dispatches to the existing OFFSET planner.
    ExOffset,
    /// `DIMEX`: dimension-style export to an external file is NOT IMPLEMENTED (no file format model).
    DimExport,
    /// `DIMIM`: dimension-style import from an external file is NOT IMPLEMENTED (no file format model).
    DimImport,
    /// `SPACETRANS`: real model/paper distance conversion for the active viewport.
    SpaceTrans,
    /// `TXTEXP`: exploding text into polylines is NOT IMPLEMENTED (no font-outline-to-geometry path).
    TextExplode,
    /// `UPDATEFIELD`: fields are NOT IMPLEMENTED (no field model).
    UpdateField,
    /// `UNITS`: read-only report of the session display-format token; the full UNITS dialog is NOT IMPLEMENTED.
    Units,

    // Wave 10: layer/view/session reports and named refusals.
    /// `-UNITS`: the command-line spelling of the read-only UNITS report.
    DashUnits,
    /// `-LAYER`: read-only layer-table report (name/on/off/frozen/locked/current); mutations are NOT IMPLEMENTED.
    DashLayer,
    /// `LAYERSTATESAVE`: with a `LayerState::Save` payload it routes to the existing layer-state store; otherwise it reports the missing dialog.
    LayerStateSave,
    /// `NEWVIEW`: with a `View::Save` payload it routes to the existing named-view store; otherwise it reports the missing window-name prompt.
    NewView,
    /// `-SCALELISTEDIT`: read-only scale-list report; the write is NOT IMPLEMENTED.
    DashScaleListEdit,
    /// `LWEIGHT`: read-only report of the session lineweight-display token; the LWEIGHT dialog is NOT IMPLEMENTED.
    LineWeight,
    /// `-LWEIGHT`: the command-line spelling of the same lineweight-display report.
    DashLineWeight,
    /// `COLOR`: read-only report of the session current-colour token; the colour dialog is NOT IMPLEMENTED.
    Color,
    /// `-COLOR`: the command-line spelling of the same current-colour report.
    DashColor,
    /// `HELP`: the help system is host-owned and NOT IMPLEMENTED in this build.
    Help,
    /// `CLASSICINSERT`: the classic insert dialog is NOT IMPLEMENTED; use INSERT.
    ClassicInsert,
    /// `CLASSICGROUP`: the classic group dialog is NOT IMPLEMENTED; use GROUP.
    ClassicGroup,
    /// `FILETAB`: file-tab chrome is host-owned; this build has no file-tab model.
    FileTab,
    /// `FILETABCLOSE`: file-tab chrome is host-owned; this build has no file-tab model.
    FileTabClose,
    /// `HIDEPALETTES`: palette chrome is host-owned; no global palette-visibility model.
    HidePalettes,
    /// `SHOWPALETTES`: palette chrome is host-owned; no global palette-visibility model.
    ShowPalettes,
    /// `RIBBONCLOSE`: ribbon chrome is host-owned; use RIBBON or the host affordance.
    RibbonClose,
    /// `GOTOSTART`: switching to the start tab is host-owned and NOT IMPLEMENTED by the router.
    GoToStart,
    /// `COPYHIST`: the session stores no command-history text; the UI command log owns it, so nothing is copied.
    CopyHist,
    /// `TRANSPARENCY`: image transparency is NOT IMPLEMENTED (no image-transparency model).
    Transparency,

    // Wave 11: 3D/mesh/surface commands. 3D is lower priority; every name is a
    // stable named refusal (Viewer-legal).
    /// `3DALIGN`: 3D align: 3D alignment is not modeled; the 2D ALIGN path is the supported one.
    ThreeDAlign,
    /// `3DARRAY`: 3D array: 3D arrays are not modeled; use the 2D ARRAY/ARRAYRECT/ARRAYPOLAR paths.
    ThreeDArray,
    /// `3DCLIP`: 3D clip: no 3D clipping-plane model.
    ThreeDClip,
    /// `3DCORBIT`: continuous 3D orbit: no orbit controller is implemented.
    ThreeDCorbit,
    /// `3DDISTANCE`: interactive 3D distance: no 3D distance navigation is implemented.
    ThreeDDistance,
    /// `3DDWF`: DWF export: DWF/DWFx export is not implemented; use PLOT for SVG/PDF.
    ThreeDDwf,
    /// `3DEDITBAR`: 3D edit bar: no NURBS/spline edit bar.
    ThreeDEditBar,
    /// `3DFACE`: 3D face: no 3DFACE entity model.
    ThreeDFace,
    /// `3DFLY`: 3D fly: no fly navigation is implemented.
    ThreeDFly,
    /// `3DFORBIT`: free 3D orbit: no orbit controller is implemented.
    ThreeDForbit,
    /// `3DMESH`: 3D mesh: no free-form mesh creation model.
    ThreeDMesh,
    /// `3DMOVE`: 3D move: no 3D move gizmo.
    ThreeDMove,
    /// `3DORBIT`: 3D orbit: no interactive orbit controller; use ORBIT for the implemented camera path.
    ThreeDOrbit,
    /// `3DORBITCTR`: 3D orbit center: no orbit-center model.
    ThreeDOrbitCtr,
    /// `-3DOSNAP`: 3D object snap: no 3D object-snap model.
    Dash3DOsnap,
    /// `3DOSNAP`: 3D object snap: no 3D object-snap model.
    ThreeDObjectSnap,
    /// `3DPAN`: 3D pan: no 3D pan navigation is implemented.
    ThreeDPan,
    /// `3DPOLY`: 3D polyline: no 3D polyline entity model.
    ThreeDPoly,
    /// `3DPRINT`: 3D print: no 3D print model.
    ThreeDPrint,
    /// `3DPRINTSERVICE`: 3D print service: no STL export or print service.
    ThreeDPrintService,
    /// `3DROTATE`: 3D rotate: no 3D rotate gizmo.
    ThreeDRotate,
    /// `3DSCALE`: 3D scale: no 3D scale gizmo.
    ThreeDScale,
    /// `3DSWIVEL`: 3D swivel: no swivel navigation is implemented.
    ThreeDSwivel,
    /// `3DWALK`: 3D walk: no walk navigation is implemented.
    ThreeDWalk,
    /// `3DZOOM`: 3D zoom: no perspective 3D zoom navigation.
    ThreeDZoom,

    // Wave 12: file/import/export/extension commands. Every name is a stable
    // named refusal (Viewer-legal).
    /// `3DSIN`: 3DS import is not implemented.
    ThreeDsIn,
    /// `ACISIN`: ACIS (SAT) import is not implemented.
    AcisIn,
    /// `ACISOUT`: ACIS (SAT) export is not implemented.
    AcisOut,
    /// `BMPOUT`: raster BMP export is not implemented; use PLOT for vector SVG/PDF.
    BmpOut,
    /// `JPGOUT`: raster JPEG export is not implemented; use PLOT for vector SVG/PDF.
    JpgOut,
    /// `PNGOUT`: raster PNG export is not implemented; use PLOT for vector SVG/PDF.
    PngOut,
    /// `TIFOUT`: raster TIFF export is not implemented; use PLOT for vector SVG/PDF.
    TifOut,
    /// `WMFIN`: Windows metafile import is not implemented.
    WmfIn,
    /// `WMFOUT`: Windows metafile export is not implemented.
    WmfOut,
    /// `IGESIMPORT`: IGES import is not implemented.
    IgesImport,
    /// `IGESEXPORT`: IGES export is not implemented.
    IgesExport,
    /// `STLOUT`: STL export is not implemented.
    StlOut,
    /// `IMPORT`: the generic IMPORT picker is not implemented.
    ImportFile,
    /// `EXPORTTOAUTOCAD`: exporting to another AutoCAD version is not implemented.
    ExportToAutoCad,
    /// `DWGCONVERT`: batch DWG conversion is not implemented.
    DwgConvert,
    /// `PLT2DWG`: HPGL (PLT) import is not implemented.
    Plt2Dwg,
    /// `DXBIN`: DXB import is not implemented.
    DxbIn,
    /// `APPLOAD`: no plug-in/application loader model.
    AppLoad,
    /// `ARX`: no ObjectARX loader model.
    Arx,
    /// `NETLOAD`: no .NET application loader model.
    NetLoad,
    /// `VBAIDE`: no VBA IDE model.
    VbaIde,
    /// `VBALOAD`: no VBA loader model.
    VbaLoad,
    /// `VBARUN`: no VBA runtime.
    VbaRun,
    /// `VBAUNLOAD`: no VBA runtime.
    VbaUnload,
    /// `SCRIPT`: no script-file runner.
    Script,

    // Wave 13: 2D/annotation/table/page-setup surfaces.
    /// `PAGESETUP`: with no payload, report each layout's stored plot settings;
    /// with a typed `Layout(PageSetup)` payload, write one authored page setup
    /// (Work-only).
    PageSetup,
    /// `-PAGESETUP`: the command-line spelling of the same report/write.
    DashPageSetup,
    /// `PSETUPIN`: with a typed `Layout(PageSetupIn)` payload, copy a stored page
    /// setup between layouts of this drawing (Work-only); there is no
    /// cross-drawing transfer in this build.
    PageSetupIn,
    /// `-PSETUPIN`: the command-line spelling of the same page-setup copy.
    DashPageSetupIn,
    /// `PLOTSTYLE`: named plot styles are NOT IMPLEMENTED (no CTB/STB table).
    PlotStyleList,
    /// `-PLOTSTYLE`: the command-line spelling of the unsupported plot-style list.
    DashPlotStyleList,
    /// `CONVERTPSTYLES`: plot-style conversion is NOT IMPLEMENTED (no CTB/STB model).
    ConvertPlotStyles,
    /// `CONVERTCTB`: CTB conversion is NOT IMPLEMENTED (no CTB/STB model).
    ConvertCtb,
    /// `CONVERTPOLY`: read-only report that this build stores one LWPOLYLINE representation; no legacy polyline conversion is performed.
    ConvertPoly,
    /// `CONVERT`: generic object conversion is NOT IMPLEMENTED.
    ConvertGeneric,
    /// `TABLE`: writes one empty table grid (Work-only).
    TableCreate,
    /// `-TABLE`: the command-line spelling of the same table creation (Work-only).
    DashTable,
    /// `TABLEDIT`: rewrites one selected table's cell text (Work-only).
    TableEdit,
    /// `TABLEEXPORT`: CSV/file table export is NOT IMPLEMENTED (no CSV/file writer).
    TableExport,
    /// `TABLESTYLE`: named table styles are NOT IMPLEMENTED (no table-style table).
    TableStyleList,
    /// `TINSERT`: inserting a block in a table cell is NOT IMPLEMENTED (no block-in-cell model).
    TableInsert,
    /// `MATCHCELL`: copying a cell style is NOT IMPLEMENTED (no cell-style copy surface).
    MatchCell,
    /// `FIELD`: fields are NOT IMPLEMENTED (no field model).
    Field,
    /// `RTEXT`: remote text is NOT IMPLEMENTED (no RTEXT model).
    RText,
    /// `RTEDIT`: remote text is NOT IMPLEMENTED (no RTEXT model).
    RTextEdit,
    /// `SPELL`: no spell-check dictionary or model is implemented.
    Spell,
    /// `SUPERHATCH`: image/block-based super-hatching is NOT IMPLEMENTED.
    SuperHatch,
    /// `ALIGNSPACE`: viewport alignment is NOT IMPLEMENTED (no viewport alignment API).
    AlignSpace,
    /// `VPSYNC`: viewport synchronization is NOT IMPLEMENTED (no viewport group model).
    VpSync,
    /// `LAYTRANS`: layer translation standards are NOT IMPLEMENTED.
    LayTrans,

    // Wave 14: stable named refusals (Viewer-legal).
    /// `BOX`: 3D solid primitives are not implemented.
    Box,
    /// `CONE`: 3D solid primitives are not implemented.
    Cone,
    /// `CYLINDER`: 3D solid primitives are not implemented.
    Cylinder,
    /// `SPHERE`: 3D solid primitives are not implemented.
    Sphere,
    /// `TORUS`: 3D solid primitives are not implemented.
    Torus,
    /// `WEDGE`: 3D solid primitives are not implemented.
    Wedge,
    /// `PYRAMID`: 3D solid primitives are not implemented.
    Pyramid,
    /// `POLYSOLID`: 3D solid primitives are not implemented.
    PolySolid,
    /// `EXTRUDE`: 3D extrusion is not implemented.
    Extrude,
    /// `REVOLVE`: 3D revolution is not implemented.
    Revolve,
    /// `SWEEP`: 3D sweep is not implemented.
    Sweep,
    /// `LOFT`: 3D loft is not implemented.
    Loft,
    /// `INTERSECT`: no 3D boolean model.
    Intersect,
    /// `UNION`: no 3D boolean model.
    Union,
    /// `SUBTRACT`: no 3D boolean model.
    Subtract,
    /// `SLICE`: no 3D slicing model.
    Slice,
    /// `THICKEN`: no surface-to-solid model.
    Thicken,
    /// `PRESSPULL`: no press/pull model.
    PressPull,
    /// `IMPRINT`: no 3D imprint model.
    Imprint,
    /// `OFFSETEDGE`: no 3D face/edge offset model.
    OffsetEdge,
    /// `CHAMFEREDGE`: no 3D edge chamfer model.
    ChamferEdge,
    /// `FILLETEDGE`: no 3D edge fillet model.
    FilletEdge,
    /// `EDGE`: no 3D face-edge visibility model.
    Edge,
    /// `MIRROR3D`: no 3D mirror-plane model.
    Mirror3D,
    /// `ROTATE3D`: no 3D rotate-axis model.
    Rotate3D,

    // Wave 15: stable named refusals (Viewer-legal).
    /// `SURFBLEND`: surface blending is not implemented.
    SurfBlend,
    /// `SURFEXTEND`: surface extension is not implemented.
    SurfExtend,
    /// `SURFEXTRACTCURVE`: surface curve extraction is not implemented.
    SurfExtractCurve,
    /// `SURFFILLET`: surface filleting is not implemented.
    SurfFillet,
    /// `SURFNETWORK`: surface network creation is not implemented.
    SurfNetwork,
    /// `SURFOFFSET`: surface offset is not implemented.
    SurfOffset,
    /// `SURFPATCH`: surface patching is not implemented.
    SurfPatch,
    /// `SURFSCULPT`: surface sculpting is not implemented.
    SurfSculpt,
    /// `SURFTRIM`: surface trimming is not implemented.
    SurfTrim,
    /// `SURFUNTRIM`: surface untrimming is not implemented.
    SurfUntrim,
    /// `MESH`: free-form mesh creation is not implemented.
    MeshCreate,
    /// `MESHCAP`: mesh capping is not implemented.
    MeshCap,
    /// `MESHCOLLAPSE`: mesh collapse is not implemented.
    MeshCollapse,
    /// `MESHCREASE`: mesh creasing is not implemented.
    MeshCrease,
    /// `MESHEXTRUDE`: mesh extrusion is not implemented.
    MeshExtrude,
    /// `MESHMERGE`: mesh merge is not implemented.
    MeshMerge,
    /// `MESHOPTIONS`: no mesh tessellation options model.
    MeshOptions,
    /// `MESHPRIMITIVEOPTIONS`: no mesh primitive options model.
    MeshPrimitiveOptions,
    /// `MESHREFINE`: mesh refinement is not implemented.
    MeshRefine,
    /// `MESHSMOOTH`: mesh smoothing conversion is not implemented.
    MeshSmooth,
    /// `MESHSMOOTHLESS`: no mesh smoothness level model.
    MeshSmoothLess,
    /// `MESHSMOOTHMORE`: no mesh smoothness level model.
    MeshSmoothMore,
    /// `MESHSPIN`: mesh edge spin is not implemented.
    MeshSpin,
    /// `MESHSPLIT`: mesh face split is not implemented.
    MeshSplit,
    /// `MESHUNCREASE`: mesh uncreasing is not implemented.
    MeshUncrease,

    // Wave 16: image report/write (see app_extended_w16_reports /
    // app_drawing::image_edit) plus its named refusals (Viewer-legal).
    /// `IMAGE`: read-only report of every stored raster image (writes nothing).
    Image,
    /// `-IMAGE`: the same read-only raster-image report with dash codes.
    DashImage,
    /// `IMAGEATTACH`: writes one raster image entity (Work-only).
    ImageAttach,
    /// `IMAGECLIP`: writes the selected image's pixel-space clip (Work-only).
    ImageClip,
    /// `IMAGEADJUST`: writes the selected image's brightness/contrast/fade/
    /// monochrome display transfer (Work-only).
    ImageAdjust,
    /// `-IMAGEADJUST`: the same display-transfer write with dash codes
    /// (Work-only).
    DashImageAdjust,
    /// `IMAGEQUALITY`: no image display-quality model.
    ImageQuality,
    /// `IMAGEAPP`: no external image-editor association model.
    ImageApp,
    /// `IMAGEASYNCWAIT`: no asynchronous image loading model.
    ImageAsyncWait,
    /// `IMAGEEDIT`: no external image-editor model.
    ImageEdit,
    /// `CLASSICIMAGE`: no image manager; use IMAGE.
    ClassicImage,
    /// `PDFATTACH`: PDF underlays are not implemented.
    PdfAttach,
    /// `-PDFATTACH`: PDF underlays are not implemented.
    DashPdfAttach,
    /// `PDFCLIP`: no PDF underlay clip model.
    PdfClip,
    /// `PDFLAYERS`: no PDF underlay layer model.
    PdfLayers,
    /// `PDFADJUST`: no PDF underlay adjustment model.
    PdfAdjust,
    /// `PDFIMPORT`: PDF geometry import is not implemented.
    PdfImport,
    /// `-PDFIMPORT`: PDF geometry import is not implemented.
    DashPdfImport,
    /// `PDFSHXTEXT`: PDF SHX text conversion is not implemented.
    PdfShxText,
    /// `-PDFSHXTEXT`: PDF SHX text conversion is not implemented.
    DashPdfShxText,
    /// `DGNATTACH`: DGN underlays are not implemented.
    DgnAttach,
    /// `-DGNATTACH`: DGN underlays are not implemented.
    DashDgnAttach,
    /// `DGNCLIP`: no DGN underlay clip model.
    DgnClip,
    /// `DGNLAYERS`: no DGN underlay layer model.
    DgnLayers,
    /// `DGNADJUST`: no DGN underlay adjustment model.
    DgnAdjust,

    /// `SHP2BLK` (Work-only): selected SHAPE rows → per-glyph block definitions
    /// of their outline polylines plus one INSERT per source, sources deleted in
    /// one transaction. Contract: `crate::app_drawing::shape`.
    Shp2Blk,

    // Waves 17-24: 197 named refusals (action macros, palettes, OLE/hyperlinks,
    // LISP, lights/materials/constraints, data extraction, DWF/DGN underlays,
    // markup, GIS, compare, sheet sets). The selectors, reasons and variant set
    // live in `refusal_names.rs`; every one is Viewer-legal and writes nothing.
    /// `Refusal(name)`: one Wave 17-24 feature with no model in this build.
    Refusal(RefusalId),
}

impl ExtendedCommandId {}
