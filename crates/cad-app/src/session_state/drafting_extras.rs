//! Session-only drafting flags and tokens for the extended command batches.
//!
//! `QTEXT`, `FILL`, `TFRAMES`, `REGENAUTO` and `ISODRAFT` (pilot) plus the Wave 2
//! names `DRAGMODE`, `HIGHLIGHTNEW`, `TRAYSETTINGS`, `TEXTSCR`/`GRAPHSCR`,
//! `FINDINCLUDE`, `FINDMATCH`, `ISOPLANE` and `APERTURE`/`REVCLOUDPROPERTIES`
//! each own one flag or token here, Wave 5 adds the `ATTDISP` token and Wave 6
//! adds the `DWGLOG`/`LOGFILEON`/`LOGFILEOFF` drawing-log token. Wave 8 adds the
//! `ELEV` elevation token and the `UCSICON` flag. This is **session state only**:
//! no database write, revision bump, `cad-history` entry or `ChangeSet`. Most
//! tokens have no renderer/behaviour consumer, so their handler reports that the
//! effect is not implemented rather than pretending the drawing changed. Two do:
//! `APERTURE` is the object-snap aperture
//! ([`crate::SessionState::interaction_tolerance`], once set explicitly) and
//! `REVCLOUDPROPERTIES` is the `REVCLOUD` chord token.

/// The session `WIPEOUTFRAME` state.
///
/// AutoCAD's three documented values apply to **all** wipeouts in the drawing:
/// `0` hides every frame (the mask still masks), `1` displays and plots them, and
/// `2` displays them without plotting. The display scene is built with the frame
/// enabled for `1` and `2`; the vector plot context is built with it enabled only
/// for `1`, which is what makes `2` display-only. Storing the token is session
/// state only: never a `cad-db` write, revision bump or undo entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WipeoutFrames {
    /// `WIPEOUTFRAME 0`: frames are neither displayed nor plotted.
    Off,
    /// `WIPEOUTFRAME 1` (the documented initial value): displayed and plotted.
    #[default]
    On,
    /// `WIPEOUTFRAME 2`: displayed but not plotted.
    DisplayNotPlot,
}

impl WipeoutFrames {
    /// The numeric `WIPEOUTFRAME` value.
    pub fn value(self) -> u8 {
        match self {
            Self::Off => 0,
            Self::On => 1,
            Self::DisplayNotPlot => 2,
        }
    }

    /// The stable uppercase label used in reports.
    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "OFF",
            Self::On => "ON",
            Self::DisplayNotPlot => "DISPLAY-NOT-PLOT",
        }
    }
}

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

/// The stored CHAMFER construction method (AutoCAD's `CHAMMODE`).
///
/// AutoCAD owns two CHAMFER methods: the distance/distances form and the
/// distance+angle form. Both are real construction rules, so the method is a
/// stored session mode (never an ignored flag). The value is read by
/// [`crate::CommandId::ChamferLines`] and `CHAMFER POLYLINE`; the interactive
/// `Method` sub-option that would set it is NOT WIRED yet (the command layer has
/// no in-command option prompt in this build).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChamferMethod {
    /// Two distances, applied in selection order (AutoCAD's default).
    Distance,
    /// Distance from the first object plus the chamfer angle from that object.
    Angle,
}

impl ChamferMethod {
    /// The stable uppercase token (`DISTANCE` / `ANGLE`).
    pub fn label(self) -> &'static str {
        match self {
            Self::Distance => "DISTANCE",
            Self::Angle => "ANGLE",
        }
    }
}

/// The stored CHAMFER/FILLET construction modes.
///
/// Session state only (AutoCAD keeps the same values as `TRIMMODE` and
/// `CHAMMODE` sysvars): no database write, revision bump, undo entry or
/// `ChangeSet`. Every field is a **consumed** mode — the two corner commands read
/// it for the actual geometry, so a stored value can never be a decorative flag:
///
/// * `trim` — `TRIMMODE` 1 (default) trims/extends the picked objects to the
///   corner result; `false` (`No Trim`, `TRIMMODE` 0) leaves them untouched and
///   only creates the joining line/arc.
/// * `chamfer_method` — see [`ChamferMethod`].
/// * `chamfer_angle_degrees` — the angle used when the method is
///   [`ChamferMethod::Angle`], in the prompt unit (degrees). `0` is AutoCAD's
///   initial `CHAMFERA`/`CHAMFERB`-era angle, and a zero distance with a zero
///   angle is the documented "extend/trim to intersect, create nothing" form.
///
/// The interactive `Trim` / `Method` / `Angle` sub-prompts that would write these
/// fields are NOT WIRED in this build; the setters are the single channel a
/// follow-up wiring lane must call.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CornerModes {
    trim: bool,
    chamfer_method: ChamferMethod,
    chamfer_angle_degrees: f64,
}

impl Default for CornerModes {
    fn default() -> Self {
        // AutoCAD's initial TRIMMODE is 1 and CHAMMODE is 0 (distance form).
        Self {
            trim: true,
            chamfer_method: ChamferMethod::Distance,
            chamfer_angle_degrees: 0.0,
        }
    }
}

impl CornerModes {
    /// Whether the picked objects are trimmed/extended (`TRIMMODE`).
    pub fn trim(&self) -> bool {
        self.trim
    }

    /// The stored CHAMFER method (`CHAMMODE`).
    pub fn chamfer_method(&self) -> ChamferMethod {
        self.chamfer_method
    }

    /// The stored CHAMFER angle in the prompt unit (degrees).
    pub fn chamfer_angle_degrees(&self) -> f64 {
        self.chamfer_angle_degrees
    }

    /// The stored CHAMFER angle in radians (the geometry unit).
    pub fn chamfer_angle_radians(&self) -> f64 {
        self.chamfer_angle_degrees.to_radians()
    }

    /// Set the `TRIMMODE` switch. Session state only.
    pub fn set_trim(&mut self, trim: bool) {
        self.trim = trim;
    }

    /// Set the `CHAMMODE` method. Session state only.
    pub fn set_chamfer_method(&mut self, method: ChamferMethod) {
        self.chamfer_method = method;
    }

    /// Store the CHAMFER angle (degrees).
    ///
    /// Only a **finite** angle in `0..180` is stored: a negative angle is not
    /// documented by AutoCAD for the angle method and is refused by name rather
    /// than guessed, and 180 leaves no wedge to chamfer. Session state only.
    pub fn set_chamfer_angle_degrees(&mut self, degrees: f64) -> crate::CadResult<()> {
        if !degrees.is_finite() || !(0.0..180.0).contains(&degrees) {
            return Err(crate::CadError::InvalidInput(format!(
                "chamfer.angle_out_of_range: the CHAMFER angle must be finite and in 0..180 \
                 degrees, got {degrees}"
            )));
        }
        self.chamfer_angle_degrees = degrees;
        Ok(())
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
    /// Whether `APERTURE <n>` set the token in this session. Until it is set, the
    /// host keeps reading the configured `view.aperture.sizePixels` value
    /// ([`SessionState::interaction_tolerance`]), so the default snap aperture
    /// stays bit-identical to the pre-command behaviour.
    aperture_explicit: bool,
    isoplane: IsoPlane,
    highlightnew: bool,
    traysettings: bool,
    text_screen: bool,
    find_include: bool,
    find_match_case: bool,
    revcloud_arc_length: f64,
    wipeout_frames: WipeoutFrames,
    attdisp: bool,
    dwglog: bool,
    elev: f64,
    ucsicon: bool,
    corner_modes: CornerModes,
    /// `LWEIGHT`: whether lineweights are displayed (AutoCAD `LWDISPLAY`).
    /// This build stores the token but has no renderer consumer.
    lweight_display: bool,
    /// `COLOR`: the session current-colour ACI (256 = ByLayer). This build
    /// stores the token but no creation path consumes it.
    current_color: i16,
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
            aperture_explicit: false,
            isoplane: IsoPlane::Left,
            highlightnew: true,
            traysettings: true,
            text_screen: false,
            find_include: true,
            find_match_case: false,
            revcloud_arc_length: 15.0,
            wipeout_frames: WipeoutFrames::On,
            attdisp: true,
            dwglog: false,
            elev: 0.0,
            ucsicon: false,
            corner_modes: CornerModes::default(),
            lweight_display: true,
            current_color: 256,
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

    /// Whether `APERTURE <n>` set the token (see the field docs).
    pub fn aperture_explicit(&self) -> bool {
        self.aperture_explicit
    }

    /// Set the session aperture token from a validated `APERTURE <n>` line.
    ///
    /// Validation (whole pixels inside `ViewAperture::RANGE`) is the command
    /// layer's job; this records that the token is now explicit, which is what
    /// makes it authoritative over `view.aperture.sizePixels`.
    pub fn set_aperture(&mut self, value: u32) {
        self.aperture = value;
        self.aperture_explicit = true;
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

    /// The session `WIPEOUTFRAME` state (default ON).
    pub fn wipeout_frames(&self) -> WipeoutFrames {
        self.wipeout_frames
    }

    /// Store the session `WIPEOUTFRAME` state.
    pub fn set_wipeout_frames(&mut self, value: WipeoutFrames) {
        self.wipeout_frames = value;
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

    /// Set the session revision-cloud arc-length token.
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

    /// The session `LWEIGHT` lineweight-display token (default ON).
    ///
    /// The renderer does not consume this flag in this build; the handler says
    /// exactly that instead of claiming a visual change.
    pub fn lweight_display(&self) -> bool {
        self.lweight_display
    }

    /// The session `COLOR` current-colour ACI (default 256 = ByLayer).
    ///
    /// No creation path reads this token yet; the handler reports the stored
    /// value and names the missing colour dialog.
    pub fn current_color(&self) -> i16 {
        self.current_color
    }

    /// The stored CHAMFER/FILLET construction modes (session state only).
    ///
    /// See [`CornerModes`] for the exact meaning of each consumed field.
    pub fn corner_modes(&self) -> CornerModes {
        self.corner_modes
    }

    /// Mutable access to the stored corner modes (the single setter channel a
    /// wiring lane uses; every value is validated by the `CornerModes` setters).
    pub fn corner_modes_mut(&mut self) -> &mut CornerModes {
        &mut self.corner_modes
    }
}
