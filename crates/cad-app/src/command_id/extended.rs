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
//! Every variant here stays Viewer-legal.

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
        }
    }

    /// Whether the command needs Work mode.
    ///
    /// Every variant in these batches is session-only, a read-only report or a
    /// named refusal, so it is Viewer-legal. This stays a method (rather than a
    /// constant) so a later write-capable extended command can declare its own
    /// authorization here.
    pub fn requires_work_mode(self) -> bool {
        false
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
    ];
}
