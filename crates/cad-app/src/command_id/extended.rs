//! The sustained "extended command" wrapper and its first batch.
//!
//! [`ExtendedCommandId`] is deliberately **not** a second command funnel: each
//! variant is carried by the single [`crate::CommandId::Extended`] variant and
//! dispatched by `crate::app_dispatch`. That keeps the main `CommandId` enum
//! from growing one arm per payload-free helper while the vocabulary table still
//! names every selector.
//!
//! # The pilot batch
//!
//! All ten are **Viewer-legal session/report commands**: they open no
//! transaction, bump no revision and record no `cad-history` entry. Five are
//! session display toggles (`QTEXT`, `FILL`, `TFRAMES`, `REGENAUTO`,
//! `ISODRAFT`); their behaviour is not consumed by this build, so the handler
//! reports an honest partial instead of claiming a visual effect. Five are
//! read-only reports (`BCOUNT`, `COUNT`, `EDITTIME`, `JULIAN`, `XLIST`).

/// One payload-free command in the extended wrapper batch.
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
        }
    }

    /// Whether the command needs Work mode.
    ///
    /// Every variant in this batch is session-only or a read-only report, so it
    /// is Viewer-legal. This stays a method (rather than a constant) so a later
    /// write-capable extended command can declare its own authorization here.
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
    ];
}
