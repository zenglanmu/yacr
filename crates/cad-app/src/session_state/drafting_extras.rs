//! Session-only drafting flags and tokens for the extended command batches.
//!
//! `QTEXT`, `FILL`, `TFRAMES`, `REGENAUTO` and `ISODRAFT` (pilot) plus the Wave 2
//! names `DRAGMODE`, `HIGHLIGHTNEW`, `TRAYSETTINGS`, `TEXTSCR`/`GRAPHSCR`,
//! `FINDINCLUDE`, `FINDMATCH`, `ISOPLANE` and `APERTURE`/`REVCLOUDPROPERTIES`
//! each own one flag or token here, Wave 5 adds the `ATTDISP` token and Wave 6
//! adds the `DWGLOG`/`LOGFILEON`/`LOGFILEOFF` drawing-log token. Wave 8 adds the
//! `ELEV` elevation token and the `UCSICON` flag. This is **session state only**:
//! no database write, revision bump, `cad-history` entry or `ChangeSet`. This
//! lane wires no renderer/behaviour consumer, so the command handler reports
//! that the effect is not implemented rather than pretending the drawing
//! changed.

/// The current 2D isometric drafting plane token (`ISOPLANE`).
///
/// The token cycles `Left → Top → Right → Left`, exactly the AutoCAD order. This
/// build has no isometric cursor or snap geometry, so the token has no effect
/// beyond the report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IsoPlane {
    /// Left plane (AutoCAD's `Left`).
    Left,
    /// Top plane (AutoCAD's `Top`).
    Top,
    /// Right plane (AutoCAD's `Right`).
    Right,
}

impl IsoPlane {
    /// The stable uppercase token label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Left => "LEFT",
            Self::Top => "TOP",
            Self::Right => "RIGHT",
        }
    }

    /// The next plane in AutoCAD's `LEFT → TOP → RIGHT → LEFT` cycle.
    pub fn next(self) -> Self {
        match self {
            Self::Left => Self::Top,
            Self::Top => Self::Right,
            Self::Right => Self::Left,
        }
    }
}

/// The drafting display flags and session tokens the extended batches own.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DraftingExtras {
    qtext: bool,
    fill: bool,
    tframes: bool,
    regenauto: bool,
    isodraft: bool,
    dragmode: bool,
    aperture: u32,
    isoplane: IsoPlane,
    highlightnew: bool,
    traysettings: bool,
    text_screen: bool,
    find_include: bool,
    find_match_case: bool,
    revcloud_arc_length: f64,
    attdisp: bool,
    dwglog: bool,
    elev: f64,
    ucsicon: bool,
}

impl Default for DraftingExtras {
    fn default() -> Self {
        // AutoCAD's defaults where they are documented: `FILL`, `DRAGMODE`,
        // `HIGHLIGHTNEW`, `TRAYSETTINGS` and `FINDINCLUDE` start ON, the aperture
        // is 10, the isometric plane is Left and the revision-cloud arc length is
        // 15. `QTEXT`/`TFRAMES`/`REGENAUTO`/`ISODRAFT`, the text screen and
        // case-insensitive find start OFF; the Wave 5 `ATTDISP` token starts ON
        // (AutoCAD's default) and the Wave 6 `DWGLOG` token starts OFF. These are
        // session defaults only — nothing here is read from or written to the
        // drawing, and no log file exists. The Wave 8 `ELEV` token starts at 0.0
        // (AutoCAD's default) and the `UCSICON` flag starts OFF.
        Self {
            qtext: false,
            fill: true,
            tframes: false,
            regenauto: false,
            isodraft: false,
            dragmode: true,
            aperture: 10,
            isoplane: IsoPlane::Left,
            highlightnew: true,
            traysettings: true,
            text_screen: false,
            find_include: true,
            find_match_case: false,
            revcloud_arc_length: 15.0,
            attdisp: true,
            dwglog: false,
            elev: 0.0,
            ucsicon: false,
        }
    }
}

impl DraftingExtras {
    pub fn qtext(&self) -> bool {
        self.qtext
    }

    pub fn fill(&self) -> bool {
        self.fill
    }

    pub fn tframes(&self) -> bool {
        self.tframes
    }

    pub fn regenauto(&self) -> bool {
        self.regenauto
    }

    pub fn isodraft(&self) -> bool {
        self.isodraft
    }

    /// The session `DRAGMODE` flag (default ON).
    pub fn dragmode(&self) -> bool {
        self.dragmode
    }

    /// The session object-snap aperture token (default 10).
    pub fn aperture(&self) -> u32 {
        self.aperture
    }

    /// The session isometric-plane token (default `Left`).
    pub fn isoplane(&self) -> IsoPlane {
        self.isoplane
    }

    /// The session `HIGHLIGHTNEW` flag (default ON).
    pub fn highlightnew(&self) -> bool {
        self.highlightnew
    }

    /// The session `TRAYSETTINGS` flag (default ON).
    pub fn traysettings(&self) -> bool {
        self.traysettings
    }

    /// Whether the session text screen token is set (`TEXTSCR`/`GRAPHSCR`).
    pub fn text_screen(&self) -> bool {
        self.text_screen
    }

    /// Whether a find includes block/xref text (`FINDINCLUDE`).
    pub fn find_include(&self) -> bool {
        self.find_include
    }

    /// Whether a find compares case-sensitively (`FINDMATCH`).
    pub fn find_match_case(&self) -> bool {
        self.find_match_case
    }

    /// The session revision-cloud arc-length token (default 15).
    pub fn revcloud_arc_length(&self) -> f64 {
        self.revcloud_arc_length
    }

    /// The session `ATTDISP` token (default ON).
    ///
    /// This build has no block-attribute model, so nothing renders either way;
    /// the token exists so a caller can see that the command was received.
    pub fn attdisp(&self) -> bool {
        self.attdisp
    }

    /// The session drawing-log token (default OFF, shared by `DWGLOG`,
    /// `LOGFILEON` and `LOGFILEOFF`).
    ///
    /// This build writes no log file, so the token exists only so a caller can
    /// see that the commands were received; the handler says exactly that.
    pub fn dwglog(&self) -> bool {
        self.dwglog
    }

    pub fn set_qtext(&mut self, value: bool) {
        self.qtext = value;
    }

    pub fn set_fill(&mut self, value: bool) {
        self.fill = value;
    }

    pub fn set_tframes(&mut self, value: bool) {
        self.tframes = value;
    }

    pub fn set_regenauto(&mut self, value: bool) {
        self.regenauto = value;
    }

    pub fn set_isodraft(&mut self, value: bool) {
        self.isodraft = value;
    }

    /// Set the session `DRAGMODE` flag.
    pub fn set_dragmode(&mut self, value: bool) {
        self.dragmode = value;
    }

    /// Set the session aperture token (only the report path uses this today; the
    /// typed `APERTURE <n>` setter is NOT IMPLEMENTED).
    pub fn set_aperture(&mut self, value: u32) {
        self.aperture = value;
    }

    /// Set the session isometric-plane token.
    pub fn set_isoplane(&mut self, value: IsoPlane) {
        self.isoplane = value;
    }

    /// Set the session `HIGHLIGHTNEW` flag.
    pub fn set_highlightnew(&mut self, value: bool) {
        self.highlightnew = value;
    }

    /// Set the session `TRAYSETTINGS` flag.
    pub fn set_traysettings(&mut self, value: bool) {
        self.traysettings = value;
    }

    /// Set the session text-screen token (`TEXTSCR` = true, `GRAPHSCR` = false).
    pub fn set_text_screen(&mut self, value: bool) {
        self.text_screen = value;
    }

    /// Set the session `FINDINCLUDE` flag.
    pub fn set_find_include(&mut self, value: bool) {
        self.find_include = value;
    }

    /// Set the session `FINDMATCH` case-sensitivity flag.
    pub fn set_find_match_case(&mut self, value: bool) {
        self.find_match_case = value;
    }

    /// Set the session revision-cloud arc-length token (the typed setter is NOT
    /// IMPLEMENTED).
    pub fn set_revcloud_arc_length(&mut self, value: f64) {
        self.revcloud_arc_length = value;
    }

    /// Flip `QTEXT` and return the new state.
    pub fn toggle_qtext(&mut self) -> bool {
        self.qtext = !self.qtext;
        self.qtext
    }

    /// Flip `FILL` and return the new state.
    pub fn toggle_fill(&mut self) -> bool {
        self.fill = !self.fill;
        self.fill
    }

    /// Flip `TFRAMES` and return the new state.
    pub fn toggle_tframes(&mut self) -> bool {
        self.tframes = !self.tframes;
        self.tframes
    }

    /// Flip `REGENAUTO` and return the new state.
    pub fn toggle_regenauto(&mut self) -> bool {
        self.regenauto = !self.regenauto;
        self.regenauto
    }

    /// Flip `ISODRAFT` and return the new state.
    pub fn toggle_isodraft(&mut self) -> bool {
        self.isodraft = !self.isodraft;
        self.isodraft
    }

    /// Flip `DRAGMODE` and return the new state.
    pub fn toggle_dragmode(&mut self) -> bool {
        self.dragmode = !self.dragmode;
        self.dragmode
    }

    /// Flip `HIGHLIGHTNEW` and return the new state.
    pub fn toggle_highlightnew(&mut self) -> bool {
        self.highlightnew = !self.highlightnew;
        self.highlightnew
    }

    /// Flip `TRAYSETTINGS` and return the new state.
    pub fn toggle_traysettings(&mut self) -> bool {
        self.traysettings = !self.traysettings;
        self.traysettings
    }

    /// Flip `FINDINCLUDE` and return the new state.
    pub fn toggle_find_include(&mut self) -> bool {
        self.find_include = !self.find_include;
        self.find_include
    }

    /// Flip `FINDMATCH` and return the new state.
    pub fn toggle_find_match_case(&mut self) -> bool {
        self.find_match_case = !self.find_match_case;
        self.find_match_case
    }

    /// Advance the isometric plane token and return the new plane.
    pub fn cycle_isoplane(&mut self) -> IsoPlane {
        self.isoplane = self.isoplane.next();
        self.isoplane
    }

    /// Flip the `ATTDISP` token and return the new state.
    pub fn toggle_attdisp(&mut self) -> bool {
        self.attdisp = !self.attdisp;
        self.attdisp
    }

    /// Set the session drawing-log token (`LOGFILEON` = true, `LOGFILEOFF` =
    /// false).
    pub fn set_dwglog(&mut self, value: bool) {
        self.dwglog = value;
    }

    /// Flip the `DWGLOG` token and return the new state.
    pub fn toggle_dwglog(&mut self) -> bool {
        self.dwglog = !self.dwglog;
        self.dwglog
    }

    /// The session `ELEV` elevation token (default 0.0).
    ///
    /// This build stores the value but has no elevation consumer: the typed
    /// setter form `ELEV <height>` is NOT IMPLEMENTED and no new object is
    /// created at an elevation. The handler says exactly that.
    pub fn elev(&self) -> f64 {
        self.elev
    }

    /// Set the session `ELEV` token (only the report path would use this today;
    /// the typed setter is NOT IMPLEMENTED).
    pub fn set_elev(&mut self, value: f64) {
        self.elev = value;
    }

    /// The session `UCSICON` flag (default OFF, i.e. no icon).
    ///
    /// This build renders no UCS icon, so the flag has no renderer consumer; the
    /// handler says exactly that.
    pub fn ucsicon(&self) -> bool {
        self.ucsicon
    }

    /// Set the session `UCSICON` flag.
    pub fn set_ucsicon(&mut self, value: bool) {
        self.ucsicon = value;
    }

    /// Flip the `UCSICON` flag and return the new state.
    pub fn toggle_ucsicon(&mut self) -> bool {
        self.ucsicon = !self.ucsicon;
        self.ucsicon
    }
}
