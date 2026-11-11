//! Typed-argument grammars for the command-line selectors.
//!
//! Split out of `commands.rs` (which is at the file-size ceiling). Every parser
//! here is pure: it decides whether the trimmed argument is inside its selector's
//! founded grammar and, when it is, returns the typed value the session
//! dispatches. Nothing is clamped, defaulted or silently repaired.
//!
//! `ARGUMENT_SELECTORS` in the parent module is the admission list; an argument
//! parser only ever sees text for a selector that list already admitted.

use cad_app::{
    DashLayerOps, NamedObjectKind, PeditOption, PurgeTarget, RenameCommand, StandardView,
    SysVarMonitorAction,
};

/// AutoCAD's `LAYERSTATE <option>`: the bounded named layer-state store.
pub const LAYERSTATE_SELECTOR: &str = "LAYERSTATE";

/// AutoCAD's `LAYWALK <names>`: show only the named layers plus the current one.
pub const LAYWALK_SELECTOR: &str = "LAYWALK";

/// AutoCAD's `LTSCALE <value>`: the drawing-global linetype scale sysvar.
pub const LTSCALE_SELECTOR: &str = "LTSCALE";

/// AutoCAD's `LAYERPMODE [ON|OFF]`: the `LAYERP` snapshot-recording gate.
pub const LAYERPMODE_SELECTOR: &str = "LAYERPMODE";

/// AutoCAD's `VIEW <option>`: the session-only named-view store.
pub const VIEW_SELECTOR: &str = "VIEW";

/// AutoCAD's `VIEWGO <name>`: restore one saved named view.
pub const VIEWGO_SELECTOR: &str = "VIEWGO";

/// AutoCAD's `VPOINT <x> <y> <z>` or `TOP|BOTTOM|FRONT|BACK|LEFT|RIGHT`.
pub const VPOINT_SELECTOR: &str = "VPOINT";

/// One parsed argument of the `VIEW` selector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViewOption {
    /// Capture the current view under a name.
    Save(String),
    /// Apply a saved view.
    Restore(String),
    /// Report every saved name.
    List,
    /// Drop one saved view.
    Delete(String),
}

/// Parse `VIEW SAVE <name>` / `RESTORE <name>` / `LIST` / `DELETE <name>`.
///
/// The keyword is case-insensitive and the name is the trimmed remainder (it
/// may contain spaces) and must be non-empty. `LIST` takes no name. Nothing else
/// is admitted (`view.needs_option` for a bare selector, the invalid-argument
/// refusal for anything outside this grammar).
pub fn parse_view_argument(argument: &str) -> Option<ViewOption> {
    let text = argument.trim();
    let (keyword, rest) = match text.find([' ', '\t']) {
        Some(index) => (&text[..index], text[index..].trim()),
        None => (text, ""),
    };
    match keyword.to_ascii_uppercase().as_str() {
        "LIST" if rest.is_empty() => Some(ViewOption::List),
        "SAVE" if !rest.is_empty() => Some(ViewOption::Save(rest.to_string())),
        "RESTORE" if !rest.is_empty() => Some(ViewOption::Restore(rest.to_string())),
        "DELETE" if !rest.is_empty() => Some(ViewOption::Delete(rest.to_string())),
        _ => None,
    }
}

/// Parse `VIEWGO <name>`: one trimmed non-empty name.
pub fn parse_viewgo_argument(argument: &str) -> Option<String> {
    let name = argument.trim();
    (!name.is_empty()).then(|| name.to_string())
}

/// Parse `VPOINT <x> <y> <z>` or one of the six full standard-view words.
///
/// The words are case-insensitive and map onto the existing
/// [`cad_app::StandardView`] path (`TOP` is the 2D plan). A vector is three
/// finite plain decimals (sign allowed, no exponent) separated by whitespace;
/// zero length is **not** rejected here because it is a valid numeric triple —
/// the command refuses it by name (`vpoint.invalid_vector`) so direct payload
/// callers get the same refusal as the typed line.
pub fn parse_vpoint_argument(argument: &str) -> Option<Point3Input> {
    let text = argument.trim();
    let parts: Vec<&str> = text.split_whitespace().collect();
    if parts.len() == 1 {
        let view = match parts[0].to_ascii_uppercase().as_str() {
            "TOP" => StandardView::Top,
            "BOTTOM" => StandardView::Bottom,
            "FRONT" => StandardView::Front,
            "BACK" => StandardView::Back,
            "LEFT" => StandardView::Left,
            "RIGHT" => StandardView::Right,
            _ => return None,
        };
        return Some(Point3Input::Standard(view));
    }
    if parts.len() != 3 {
        return None;
    }
    let x = parse_signed_decimal(parts[0])?;
    let y = parse_signed_decimal(parts[1])?;
    let z = parse_signed_decimal(parts[2])?;
    Some(Point3Input::Vector([x, y, z]))
}

/// The machine form of a parsed `VPOINT` argument.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Point3Input {
    /// One of the six standard views.
    Standard(StandardView),
    /// An eye direction in world coordinates.
    Vector([f64; 3]),
}

/// One parsed argument of the `ZOOM` selector.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ZoomArgument {
    /// A positive finite **scale factor**: `> 1` zooms in, `< 1` zooms out —
    /// the same convention as `ZOOMIN`/`ZOOMOUT` and
    /// [`cad_app::CommandPayload::ZoomAt`]. The zoom is anchored at the
    /// **viewport centre**, exactly like those cursor-less button steps.
    Factor(f64),
    /// `nX`: a positive finite **relative factor** (`2X`, `0.5X`). It is the
    /// same scale operation as [`ZoomArgument::Factor`] and rides the same
    /// [`cad_app::CommandPayload::ZoomAt`] payload; the suffix is only the
    /// AutoCAD spelling. `> 1` zooms in, `< 1` zooms out.
    Relative(f64),
    /// `nXP` / `a/bXP`: the **paper-space viewport scale**, written as a
    /// paper-to-drawing ratio. `2XP` becomes `VPSCALE 1:2` and `1/50XP` (or
    /// `1:50XP`) becomes `VPSCALE 1:50`; the ratio is routed through the
    /// existing Work-only `VPSCALE` write path, never through the Viewer-legal
    /// `ZOOM` command.
    PaperScale { paper: f64, drawing: f64 },
    /// `OBJECT`: frame the current **selection**. The selection is the whole
    /// input — there is no payload point and no interactive object pick.
    Object,
    /// `ALL` (the written-out spelling) or `EXTENTS`: frame the whole drawing,
    /// i.e. the [`cad_app::CommandId::FitDrawing`] path.
    Fit,
    /// `WINDOW`: start the **two-corner** capture. The next 2D primary drag
    /// supplies the rectangle (canvas logical pixels, through the host's
    /// box-gesture path) and its release dispatches one
    /// [`cad_app::CommandPayload::ZoomWindow`]. The camera does not move when the
    /// argument is admitted — only when the rectangle is completed — and a 3D
    /// view refuses by name (`zoom.window.requires_2d`).
    Window,
    /// `PREVIOUS`: restore this viewport's last **discrete** view immediately
    /// (no capture), i.e. [`cad_app::CommandPayload::ZoomPrevious`]. An empty
    /// history is a `zoom.previous.empty` diagnostic, not an error.
    Previous,
}

/// Parse the argument of a `ZOOM` selector: a positive decimal factor, `nX`,
/// `nXP`, or `ALL` / `EXTENTS` / `WINDOW` / `PREVIOUS` / `OBJECT`
/// (case-insensitive, surrounding whitespace ignored).
///
/// The numeric grammar is deliberately narrow — ASCII digits with at most one
/// `.` that has digits on **both** sides (`2`, `0.5`, `1.25`, `2.0`). Signs
/// (`-1`, `+2`), exponents (`1e3`), a bare `.`/`1.`, `NaN`/`inf` and zero are
/// all refused with `None`; the caller turns that into the stable refusal from
/// [`invalid_zoom_argument_reason`]. Nothing is clamped and no default factor is
/// invented.
///
/// The word arguments are the **full** spellings only: no `W`/`P`/`O`
/// abbreviation is admitted (this router never guesses at a partial word), and
/// `ZOOM EXTENTS` is still resolved earlier by the pre-existing alias onto `FIT`.
///
/// `ZOOM DYNAMIC` is deliberately **outside** this grammar. It is a named,
/// unimplemented gap (`zoom.dynamic_not_implemented`) and must never be parsed
/// into a different form.
pub fn parse_zoom_argument(argument: &str) -> Option<ZoomArgument> {
    let key = argument.trim();
    if key.eq_ignore_ascii_case("ALL") || key.eq_ignore_ascii_case("EXTENTS") {
        return Some(ZoomArgument::Fit);
    }
    if key.eq_ignore_ascii_case("WINDOW") {
        return Some(ZoomArgument::Window);
    }
    if key.eq_ignore_ascii_case("PREVIOUS") {
        return Some(ZoomArgument::Previous);
    }
    if key.eq_ignore_ascii_case("OBJECT") {
        return Some(ZoomArgument::Object);
    }
    let upper = key.to_ascii_uppercase();
    // `nXP` / `a/bXP` / `a:bXP`: the paper-to-drawing ratio.
    if upper.ends_with("XP") {
        return parse_paper_scale(&key[..key.len() - 2]);
    }
    // `nX`: the relative factor.
    if upper.ends_with('X') {
        return parse_positive_decimal(&key[..key.len() - 1]).map(ZoomArgument::Relative);
    }
    parse_positive_decimal(key).map(ZoomArgument::Factor)
}

/// Parse the body of an `nXP` argument into its paper-to-drawing ratio.
///
/// A bare factor (`2XP`) is `1:2` (one paper unit is two drawing units); a
/// ratio may use `/` or `:` (`1/50XP`, `1:50XP`). Every component is the same
/// positive plain decimal [`parse_positive_decimal`] accepts, so signs, zero,
/// exponents and embedded spaces are refused here.
fn parse_paper_scale(body: &str) -> Option<ZoomArgument> {
    let body = body.trim();
    if body.is_empty() {
        return None;
    }
    if let Some((paper, drawing)) = body.split_once(['/', ':']) {
        let paper = parse_positive_decimal(paper)?;
        let drawing = parse_positive_decimal(drawing)?;
        return Some(ZoomArgument::PaperScale { paper, drawing });
    }
    parse_positive_decimal(body).map(|drawing| ZoomArgument::PaperScale {
        paper: 1.0,
        drawing,
    })
}

/// Parse the argument of a `PEDIT` selector: one declared v1 polyline option.
///
/// Accepted (case-insensitive): `CLOSE`/`C`, `OPEN`/`O` and `REVERSE`/`R`. No
/// other word, partial spelling, empty argument, option stack (`PEDIT C O`) or
/// AutoCAD option beyond this subset (`JOIN`, `WIDTH`, `FIT`, `SPLINE`, …) is
/// admitted; the caller turns `None` into the stable refusal from
/// [`invalid_pedit_argument_reason`]. The bare selector never reaches this
/// parser: it keeps the name-only funnel and the application refuses it with
/// `pedit.needs_option`.
pub fn parse_pedit_argument(argument: &str) -> Option<PeditOption> {
    let key = argument.trim();
    if key.eq_ignore_ascii_case("CLOSE") || key.eq_ignore_ascii_case("C") {
        return Some(PeditOption::Close);
    }
    if key.eq_ignore_ascii_case("OPEN") || key.eq_ignore_ascii_case("O") {
        return Some(PeditOption::Open);
    }
    if key.eq_ignore_ascii_case("REVERSE") || key.eq_ignore_ascii_case("R") {
        return Some(PeditOption::Reverse);
    }
    None
}

/// Parse the argument of a `SNAPUNIT` selector: a positive decimal spacing.
///
/// The grammar is exactly the numeric shape `ZOOM` accepts
/// ([`parse_zoom_argument`]): ASCII digits with at most one `.` that has digits
/// on **both** sides (`25`, `0.5`, `2.5`). Signs (`-5`, `+5`), exponents
/// (`1e3`), a bare `.`/`5.`, `NaN`/`inf`, zero and `-0` are all refused with
/// `None`; the caller turns that into the stable refusal from
/// [`invalid_snap_unit_argument_reason`]. Nothing is clamped, no default spacing
/// is invented and `ALL`/`EXTENTS` mean nothing here.
pub fn parse_snap_unit_argument(argument: &str) -> Option<f64> {
    parse_positive_decimal(argument.trim())
}

/// Parse the argument of a `PDMODE` selector: one integer that fits `i16`.
///
/// The grammar is deliberately narrow — an optional `+`/`-` sign followed by
/// ASCII digits only (`0`, `3`, `35`, `-1`). A **fraction** (`3.5`) is refused:
/// `$PDMODE` is an integer bit field, so a decimal point can only mean the typist
/// has the wrong variable, and rounding it would silently draw a different marker
/// than the one asked for. Exponents (`1e1`), a bare sign, embedded spaces
/// (`3 5`) and every non-digit are refused too, as is any value outside
/// `i16::MIN..=i16::MAX` (`40000`) — the header field cannot hold it, and clamping
/// would store a value the user never typed. The caller turns `None` into the
/// stable refusal from [`invalid_pdmode_argument_reason`]; nothing is clamped and
/// no default mode is invented.
pub fn parse_pdmode_argument(argument: &str) -> Option<i16> {
    let text = argument.trim();
    let digits = text.strip_prefix(['+', '-']).unwrap_or(text);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

/// Parse the argument of a `PDSIZE` selector: one finite decimal size.
///
/// Unlike [`parse_snap_unit_argument`] the **sign and zero are meaningful** and
/// therefore accepted: `> 0` is an absolute drawing-unit size, `0` the
/// viewport-relative DWG default and `< 0` a percentage of the viewport height
/// (AutoCAD's own `PDSIZE` range). The numeric shape is the same narrow decimal
/// (`2`, `2.5`, `0`, `-20`, `+4`): ASCII digits with at most one `.` that has
/// digits on **both** sides. A bare `.`/`5.`, exponents (`1e3`), `NaN`/`inf`,
/// embedded spaces (`2 3`) and anything else are refused with `None`, as is a
/// magnitude that overflows to a non-finite `f64`. Nothing is clamped; the caller
/// turns `None` into the stable refusal from
/// [`invalid_pdsize_argument_reason`].
pub fn parse_pdsize_argument(argument: &str) -> Option<f64> {
    parse_signed_decimal(argument.trim())
}

/// One parsed argument of the `LAYERSTATE` selector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayerStateOption {
    /// Snapshot the current layers under a name.
    Save(String),
    /// Apply a saved snapshot.
    Restore(String),
    /// Report every saved name.
    List,
    /// Drop one saved snapshot.
    Delete(String),
}

/// Parse the argument of a `LAYERSTATE` selector:
/// `SAVE <name>`, `RESTORE <name>`, `LIST` or `DELETE <name>`.
///
/// The keyword is case-insensitive; the name is the trimmed remainder (it may
/// contain spaces) and must be non-empty. `LIST` takes no name; a name supplied
/// after `LIST`, a missing name after the other three keywords, and any other
/// keyword are refused with `None`, which the caller turns into
/// [`invalid_layerstate_argument_reason`]. Nothing is defaulted.
pub fn parse_layerstate_argument(argument: &str) -> Option<LayerStateOption> {
    let text = argument.trim();
    let (keyword, rest) = match text.find([' ', '\t']) {
        Some(index) => (&text[..index], text[index..].trim()),
        None => (text, ""),
    };
    match keyword.to_ascii_uppercase().as_str() {
        "LIST" if rest.is_empty() => Some(LayerStateOption::List),
        "SAVE" if !rest.is_empty() => Some(LayerStateOption::Save(rest.to_string())),
        "RESTORE" if !rest.is_empty() => Some(LayerStateOption::Restore(rest.to_string())),
        "DELETE" if !rest.is_empty() => Some(LayerStateOption::Delete(rest.to_string())),
        _ => None,
    }
}

/// Parse the argument of a `LAYWALK` selector: comma-separated layer names.
///
/// Surrounding whitespace is trimmed from every name; an empty list, an empty
/// entry (`WALLS,,DIMS`), a bare comma and a blank argument are refused with
/// `None` ([`invalid_laywalk_argument_reason`]). Duplicates are allowed — the
/// command dedupes by resolved layer id. Nothing is resolved here: the names are
/// matched against the authoritative drawing by the application.
pub fn parse_laywalk_argument(argument: &str) -> Option<Vec<String>> {
    let mut names = Vec::new();
    for part in argument.split(',') {
        let name = part.trim();
        if name.is_empty() {
            return None;
        }
        names.push(name.to_string());
    }
    (!names.is_empty()).then_some(names)
}

/// Parse the argument of a `LTSCALE` selector: one positive finite decimal.
///
/// The numeric shape is [`parse_snap_unit_argument`]'s (unsigned plain decimal):
/// signs, exponents, zero, `NaN`/`inf` and embedded spaces are refused with
/// `None` ([`invalid_ltscale_argument_reason`]). The database repeats the
/// positivity/finiteness check, so a value cannot slip past a second caller.
pub fn parse_ltscale_argument(argument: &str) -> Option<f64> {
    parse_positive_decimal(argument.trim())
}

/// Parse the argument of a `LAYERPMODE` selector: `ON` or `OFF`.
///
/// Case-insensitive, surrounding whitespace ignored. Nothing else is admitted;
/// the caller turns `None` into [`invalid_layerpmode_argument_reason`]. The bare
/// selector is admitted separately and reports the current setting.
pub fn parse_layerpmode_argument(argument: &str) -> Option<bool> {
    let key = argument.trim();
    if key.eq_ignore_ascii_case("ON") {
        return Some(true);
    }
    if key.eq_ignore_ascii_case("OFF") {
        return Some(false);
    }
    None
}

/// The stable rejection reason for a `ZOOM` argument outside the grammar.
///
/// One wording for one fact: the typed line is echoed verbatim and the accepted
/// grammar is spelled out, so a refusal is never a silent ignore or a clamp.
pub fn invalid_zoom_argument_reason(text: &str) -> String {
    format!(
        "unsupported CAD command: {} (ZOOM needs a positive factor, ALL, EXTENTS, WINDOW, \
         PREVIOUS, OBJECT, nX or nXP; ZOOM DYNAMIC is not implemented)",
        text.trim()
    )
}

/// The stable rejection reason for a `PEDIT` argument outside the legacy subset.
///
/// The typed line is echoed and the two grammars are named: the legacy
/// `CLOSE`/`OPEN`/`REVERSE` subset (which keeps `PeditOption`) and the full
/// option line (`PeditEdit`), so a typo is refused by name and nobody reads the
/// legacy subset as the whole surface.
pub fn invalid_pedit_argument_reason(text: &str) -> String {
    format!(
        "unsupported CAD command: {} (the legacy PEDIT subset is CLOSE, OPEN or REVERSE, or C, \
         O, R; the full option line is JOIN, WIDTH <w>, EDIT, FIT, SPLINE [0-99], DECURVE, \
         LTYPE [GEN|ON|OFF] and UNDO, optionally prefixed by CONVERT Y|N)",
        text.trim()
    )
}

/// The stable rejection reason for a `SNAPUNIT` argument outside the grammar.
///
/// Same shape as [`invalid_zoom_argument_reason`]: the typed line is echoed and
/// the accepted grammar (one positive decimal spacing) is named.
pub fn invalid_snap_unit_argument_reason(text: &str) -> String {
    format!(
        "unsupported CAD command: {} (SNAPUNIT needs a positive decimal spacing)",
        text.trim()
    )
}

/// The stable rejection reason for a `PDMODE` argument outside the grammar.
///
/// The typed line is echoed and the accepted grammar is named, so a fractional
/// (`PDMODE 3.5`) or out-of-`i16` (`PDMODE 40000`) value is refused **by name**
/// instead of being rounded, truncated or clamped into a different marker.
pub fn invalid_pdmode_argument_reason(text: &str) -> String {
    format!(
        "unsupported CAD command: {} (PDMODE needs one whole number in the i16 range, e.g. \
         `PDMODE 3`)",
        text.trim()
    )
}

/// The stable rejection reason for a `PDSIZE` argument outside the grammar.
///
/// The typed line is echoed and the accepted grammar (one finite decimal size,
/// sign and zero allowed) is named.
pub fn invalid_pdsize_argument_reason(text: &str) -> String {
    format!(
        "unsupported CAD command: {} (PDSIZE needs one finite decimal size, e.g. `PDSIZE 2.5`)",
        text.trim()
    )
}

/// The stable rejection reason for a `LAYERSTATE` argument outside the grammar.
pub fn invalid_layerstate_argument_reason(text: &str) -> String {
    format!(
        "unsupported CAD command: {} (LAYERSTATE needs SAVE <name>, RESTORE <name>, LIST or \
         DELETE <name>)",
        text.trim()
    )
}

/// The stable rejection reason for a `LAYWALK` argument outside the grammar.
pub fn invalid_laywalk_argument_reason(text: &str) -> String {
    format!(
        "unsupported CAD command: {} (LAYWALK needs one or more layer names separated by commas, \
         e.g. `LAYWALK WALLS,DIMS`)",
        text.trim()
    )
}

/// The stable rejection reason for a `LTSCALE` argument outside the grammar.
pub fn invalid_ltscale_argument_reason(text: &str) -> String {
    format!(
        "unsupported CAD command: {} (LTSCALE needs one positive finite decimal, e.g. `LTSCALE \
         2.5`)",
        text.trim()
    )
}

/// The stable rejection reason for a `LAYERPMODE` argument outside the grammar.
pub fn invalid_layerpmode_argument_reason(text: &str) -> String {
    format!(
        "unsupported CAD command: {} (LAYERPMODE needs ON or OFF; a bare LAYERPMODE reports the \
         current setting)",
        text.trim()
    )
}

/// The stable rejection reason for a `VIEW` argument outside the grammar.
pub fn invalid_view_argument_reason(text: &str) -> String {
    format!(
        "unsupported CAD command: {} (VIEW needs SAVE <name>, RESTORE <name>, LIST or DELETE \
         <name>)",
        text.trim()
    )
}

/// The stable rejection reason for a `VIEWGO` argument outside the grammar.
pub fn invalid_viewgo_argument_reason(text: &str) -> String {
    format!(
        "unsupported CAD command: {} (VIEWGO needs one saved view name)",
        text.trim()
    )
}

/// The stable rejection reason for a `VPOINT` argument outside the grammar.
pub fn invalid_vpoint_argument_reason(text: &str) -> String {
    format!(
        "unsupported CAD command: {} (VPOINT needs <x> <y> <z> or TOP, BOTTOM, FRONT, BACK, LEFT, \
         RIGHT)",
        text.trim()
    )
}

/// Whether `text` is a plain positive decimal number (`digits[.digits]`).
///
/// A leading sign is refused here even though
/// [`parse_signed_decimal`] accepts one: `ZOOM`/`SNAPUNIT`/`LTSCALE` have always
/// refused `+5`/`-5` (a spacing/scale is written unsigned), and widening their
/// grammar would change an existing refusal contract.
pub(crate) fn parse_positive_decimal(text: &str) -> Option<f64> {
    if text.starts_with(['+', '-']) {
        return None;
    }
    parse_signed_decimal(text).filter(|value| *value > 0.0)
}

/// Whether `text` is a plain decimal number with an optional sign
/// (`[+-]digits[.digits]`).
///
/// No exponent, no bare `.`/`5.`, no embedded spaces: the same narrow shape
/// `ZOOM`/`SNAPUNIT` accept, widened only by the sign (which `PDSIZE` needs:
/// a negative size is a percentage of the viewport height). A magnitude that
/// overflows to a non-finite `f64` is refused rather than stored.
pub(crate) fn parse_signed_decimal(text: &str) -> Option<f64> {
    let digits = text.strip_prefix(['+', '-']).unwrap_or(text);
    let (whole, fraction) = match digits.split_once('.') {
        Some((whole, fraction)) => (whole, Some(fraction)),
        None => (digits, None),
    };
    let digits_only = |part: &str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
    if !digits_only(whole) {
        return None;
    }
    if let Some(fraction) = fraction {
        if !digits_only(fraction) {
            return None;
        }
    }
    let value: f64 = text.parse().ok()?;
    value.is_finite().then_some(value)
}

// ---------------------------------------------------------------------------
// The 2026-11-07 utility/named-object batch
// ---------------------------------------------------------------------------

/// `PURGE [LAYER|BLOCK|STYLE|LINETYPE]`: remove unused named objects.
pub const PURGE_SELECTOR: &str = "PURGE";

/// `RENAME <kind> <old> <new>`: rename one named object.
pub const RENAME_SELECTOR: &str = "RENAME";

/// `SETVAR <name> [<value>]`: report or write one modelled variable.
pub const SETVAR_SELECTOR: &str = "SETVAR";

/// `SYSVARMONITOR ADD|REMOVE|LIST <name>`: the session watch set.
pub const SYSVARMONITOR_SELECTOR: &str = "SYSVARMONITOR";

/// `MREDO [n]`: apply up to `n` existing redo records.
pub const MREDO_SELECTOR: &str = "MREDO";

/// `MULTIPLE [command]`: arm the session repeat flag.
pub const MULTIPLE_SELECTOR: &str = "MULTIPLE";

/// Parse the argument of a `PURGE` selector: one full kind word, or none.
///
/// `LAYER` is the only kind with a database delete API in this build; `BLOCK`,
/// `STYLE` and `LINETYPE` are admitted so the application can refuse them **by
/// name** (never as an unknown word). A partial spelling is refused.
pub fn parse_purge_argument(argument: &str) -> Option<PurgeTarget> {
    match argument.trim().to_ascii_uppercase().as_str() {
        "LAYER" => Some(PurgeTarget::Layer),
        "BLOCK" => Some(PurgeTarget::Block),
        "STYLE" => Some(PurgeTarget::Style),
        "LINETYPE" => Some(PurgeTarget::Linetype),
        _ => None,
    }
}

/// The stable refusal for a `PURGE` argument outside the kind grammar.
pub fn invalid_purge_argument_reason(text: &str) -> String {
    format!("invalid PURGE argument: '{text}' (expected LAYER, BLOCK, STYLE or LINETYPE)")
}

/// Parse the argument of a `RENAME` selector: `<kind> <old> <new>`.
///
/// Exactly three tokens after quote-aware splitting. A name containing
/// whitespace must be double-quoted (`RENAME LAYER "Old Walls" "New Walls"`);
/// an unquoted extra token is refused rather than guessed, because the old/new
/// boundary would be ambiguous.
pub fn parse_rename_argument(argument: &str) -> Option<RenameCommand> {
    let tokens = split_quoted_tokens(argument)?;
    if tokens.len() != 3 {
        return None;
    }
    let kind = match tokens[0].to_ascii_uppercase().as_str() {
        "LAYER" => NamedObjectKind::Layer,
        "BLOCK" => NamedObjectKind::Block,
        "STYLE" => NamedObjectKind::Style,
        "LINETYPE" => NamedObjectKind::Linetype,
        "DIMSTYLE" => NamedObjectKind::DimStyle,
        "MLEADERSTYLE" => NamedObjectKind::MLeaderStyle,
        _ => return None,
    };
    if tokens[1].is_empty() || tokens[2].is_empty() {
        return None;
    }
    Some(RenameCommand {
        kind,
        old: tokens[1].clone(),
        new: tokens[2].clone(),
    })
}

/// The stable refusal for a `RENAME` argument outside its grammar.
pub fn invalid_rename_argument_reason(text: &str) -> String {
    format!(
        "invalid RENAME argument: '{text}' (expected <kind> <old> <new> with LAYER, BLOCK, STYLE, \
         LINETYPE, DIMSTYLE or MLEADERSTYLE; quote names that contain spaces)"
    )
}

/// Parse the argument of a `SETVAR` selector: `<name>` or `<name> <value>`.
///
/// The name is one token; the value is the trimmed remainder (a boolean word or
/// a number, so embedded spaces are refused later by the value grammar). An
/// empty name is refused. The variable-name set and value grammar belong to the
/// caller: this parser only splits the two halves.
pub fn parse_setvar_argument(argument: &str) -> Option<(String, Option<String>)> {
    let text = argument.trim();
    let split = text.find([' ', '\t']);
    let (name, value) = match split {
        Some(index) => (&text[..index], text[index..].trim()),
        None => (text, ""),
    };
    if name.is_empty() {
        return None;
    }
    Some((
        name.to_string(),
        (!value.is_empty()).then(|| value.to_string()),
    ))
}

/// Parse the argument of a `MREDO` selector: one positive integer.
///
/// ASCII digits only; zero, a sign, a fraction, an exponent and anything outside
/// `u32` are refused rather than clamped.
pub fn parse_mredo_argument(argument: &str) -> Option<u32> {
    let text = argument.trim();
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse::<u32>().ok().filter(|count| *count > 0)
}

/// The stable refusal for a `MREDO` argument outside its grammar.
pub fn invalid_mredo_argument_reason(text: &str) -> String {
    format!("invalid MREDO argument: '{text}' (expected one positive integer)")
}

/// Parse the argument of a `MULTIPLE` selector: one non-empty command name.
///
/// The name is returned verbatim; the caller resolves it against the command
/// vocabulary so an unknown name is a named refusal, not a stored phantom.
pub fn parse_multiple_argument(argument: &str) -> Option<String> {
    let text = argument.trim();
    (!text.is_empty()).then(|| text.to_string())
}

/// Parse the argument of a `SYSVARMONITOR` selector.
///
/// The grammar is `ADD <name>`, `REMOVE <name>` or `LIST [<name>]`; the bare
/// selector is the `LIST` form and carries no payload, so it never reaches this
/// parser. The name is one non-empty token. The **modelled** name set and every
/// semantic refusal belong to the application, which owns the closed set.
pub fn parse_sysvarmonitor_argument(argument: &str) -> Option<SysVarMonitorAction> {
    let mut parts = argument.split_whitespace();
    let verb = parts.next()?.to_ascii_uppercase();
    let name = parts.next().map(str::to_string);
    if parts.next().is_some() {
        return None;
    }
    match verb.as_str() {
        "ADD" => name.map(SysVarMonitorAction::Add),
        "REMOVE" => name.map(SysVarMonitorAction::Remove),
        "LIST" => Some(SysVarMonitorAction::List(name)),
        _ => None,
    }
}

/// The stable refusal for a `SYSVARMONITOR` argument outside its grammar.
pub fn invalid_sysvarmonitor_argument_reason(text: &str) -> String {
    format!(
        "invalid SYSVARMONITOR argument: '{text}' (expected `ADD <name>`, `REMOVE <name>` or \
         `LIST [<name>]`)"
    )
}

/// The stable refusal for a `MULTIPLE` argument outside its grammar.
pub fn invalid_multiple_argument_reason(text: &str) -> String {
    format!("invalid MULTIPLE argument: '{text}' (expected one command name)")
}

/// AutoCAD's `-LAYER <option> …`: the command-line layer editor's write forms.
///
/// The bare selector and `-LAYER ?` keep the **read-only report** (admitted as
/// the extended `DashLayer` command and Viewer-legal), so `?` never reaches this
/// parser; every write option does.
pub const DASH_LAYER_SELECTOR: &str = "-LAYER";

/// Parse the argument of a `-LAYER` selector: `? [pattern]` (the read-only
/// report, optional AutoCAD name wildcard) or one write option.
///
/// One option per typed line. Keywords are case-insensitive and the documented
/// short aliases are `N` (NEW), `M` (MAKE), `S` (SET), `R` (RENAME), `D`
/// (DELETE), `C` (COLOR), `L` (LTYPE), `LW` (LWEIGHT), `P` (PLOT) plus AutoCAD's
/// `F` (FREEZE), `T` (THAW), `LO` (LOCK) and `U` (UNLOCK). `ON` and `OFF` are
/// the full spellings (AutoCAD's own `-LAYER` prompt words).
///
/// Shape:
///
/// * name lists (`NEW`, `DELETE`, `ON`, `OFF`, `FREEZE`, `THAW`, `LOCK`,
///   `UNLOCK`) are **comma-separated**; each entry is trimmed and must be
///   non-empty. A space is **not** a list separator: a layer name may contain
///   spaces, so `NEW A B` is one layer named `A B`. A layer name may not contain
///   a comma (the database refuses one), which is what makes the list
///   unambiguous;
/// * `MAKE` / `SET` take the trimmed remainder as the name (a single fully
///   quoted token is unquoted);
/// * `RENAME` takes the first token as the old name and the remainder as the
///   new one; either side may be quoted (`RENAME "OLD NAME" "NEW NAME"`);
/// * `COLOR` / `LTYPE` / `LWEIGHT` / `PLOT` take one value token followed by the
///   name list; the value may be quoted (`LTYPE "My Style" WALLS`).
///
/// `COLOR`'s value token is passed through **verbatim**: the command layer
/// rewrites a bare AutoCAD ACI number (`3`) to the layer grammar's `aci:3` form
/// (`cad_app::layer_property::normalize_color_input`) and then parses every colour
/// with that one grammar, which owns the ranges, the `BYLAYER` refusal and the
/// linetype-free `#RRGGBB` / `r,g,b` forms.
///
/// `PLOT` accepts `Y`/`N` (the command-line spelling) and the layer grammar's
/// flag words (`TRUE`/`FALSE`/`1`/`0`, case-insensitive).
///
/// A missing operand, an empty list entry, an unknown keyword, a second
/// whitespace token where one value is expected and everything else return
/// `None`; the caller turns that into
/// [`invalid_dash_layer_argument_reason`]. Nothing is defaulted.
pub fn parse_dash_layer_argument(argument: &str) -> Option<DashLayerOps> {
    let text = argument.trim();
    // `?` is the read-only table report, optionally followed by one layer-name
    // wildcard filter (`*` any run, `?` one character, case-insensitive).
    if let Some(rest) = text.strip_prefix('?') {
        let pattern = rest.trim();
        return Some(DashLayerOps::List {
            pattern: (!pattern.is_empty()).then(|| pattern.to_string()),
        });
    }
    let (keyword, rest) = split_keyword(text);
    match keyword.to_ascii_uppercase().as_str() {
        "NEW" | "N" => Some(DashLayerOps::New(parse_layer_name_list(rest)?)),
        "MAKE" | "M" => Some(DashLayerOps::Make(single_name(rest)?)),
        "SET" | "S" => Some(DashLayerOps::Set(single_name(rest)?)),
        "RENAME" | "R" => {
            let tokens = split_quoted_tokens(rest)?;
            if tokens.len() < 2 || tokens[0].is_empty() {
                return None;
            }
            let old = tokens[0].clone();
            let new = tokens[1..].join(" ");
            (!new.is_empty()).then_some(DashLayerOps::Rename { old, new })
        }
        "DELETE" | "D" => Some(DashLayerOps::Delete(parse_layer_name_list(rest)?)),
        "ON" => Some(DashLayerOps::On(parse_layer_name_list(rest)?)),
        "OFF" => Some(DashLayerOps::Off(parse_layer_name_list(rest)?)),
        "FREEZE" | "F" => Some(DashLayerOps::Freeze(parse_layer_name_list(rest)?)),
        "THAW" | "T" => Some(DashLayerOps::Thaw(parse_layer_name_list(rest)?)),
        "LOCK" | "LO" => Some(DashLayerOps::Lock(parse_layer_name_list(rest)?)),
        "UNLOCK" | "U" => Some(DashLayerOps::Unlock(parse_layer_name_list(rest)?)),
        "COLOR" | "C" => {
            let (value, names) = split_quoted_value_and_rest(rest)?;
            Some(DashLayerOps::Color {
                value,
                layers: parse_layer_name_list(&names)?,
            })
        }
        "LTYPE" | "L" => {
            let (name, names) = split_quoted_value_and_rest(rest)?;
            Some(DashLayerOps::Linetype {
                name,
                layers: parse_layer_name_list(&names)?,
            })
        }
        "LWEIGHT" | "LW" => {
            let (value, names) = split_quoted_value_and_rest(rest)?;
            Some(DashLayerOps::Lineweight {
                value,
                layers: parse_layer_name_list(&names)?,
            })
        }
        "PLOT" | "P" => {
            let (value, names) = split_quoted_value_and_rest(rest)?;
            Some(DashLayerOps::Plot {
                plottable: parse_plottable_value(&value)?,
                layers: parse_layer_name_list(&names)?,
            })
        }
        _ => None,
    }
}

/// The stable refusal for a `-LAYER` write argument outside the grammar.
///
/// The typed line is echoed and the accepted options plus the comma-separated
/// list rule are spelled out, so a refusal is never a silent ignore or a
/// defaulted option.
pub fn invalid_dash_layer_argument_reason(text: &str) -> String {
    format!(
        "unsupported CAD command: {} (-LAYER takes ? [wildcard] or one option: NEW|N, MAKE|M, \
         SET|S, RENAME|R <old> <new>, DELETE|D, ON, OFF, FREEZE|F, THAW|T, LOCK|LO, UNLOCK|U, \
         COLOR|C <value>, LTYPE|L <name>, LWEIGHT|LW <value> or PLOT|P Y|N, followed by \
         comma-separated layer names; a bare -LAYER or -LAYER ? reports the table)",
        text.trim()
    )
}

/// Split the keyword from the trimmed remainder of a line.
fn split_keyword(text: &str) -> (&str, &str) {
    match text.find([' ', '\t']) {
        Some(index) => (&text[..index], text[index..].trim()),
        None => (text, ""),
    }
}

/// One trimmed non-empty name (the whole remainder, spaces allowed).
///
/// A single fully quoted token (`SET "A B"`) is unquoted here; an unquoted
/// remainder (`SET A B`) is taken verbatim as the one name, exactly as before.
fn single_name(rest: &str) -> Option<String> {
    let name = rest.trim();
    if name.is_empty() {
        return None;
    }
    if name.starts_with('"') {
        let tokens = split_quoted_tokens(name)?;
        if tokens.len() == 1 {
            return (!tokens[0].is_empty()).then(|| tokens[0].clone());
        }
    }
    Some(name.to_string())
}

/// Split one (optionally quoted) value token from the non-empty name list.
///
/// `LTYPE "My Style" WALLS` works because the quoted value keeps its space;
/// unquoted values keep the historical one-whitespace-token rule. The remainder
/// is rejoined with single spaces and handed to the comma-list parser.
fn split_quoted_value_and_rest(rest: &str) -> Option<(String, String)> {
    let tokens = split_quoted_tokens(rest.trim())?;
    if tokens.len() < 2 || tokens[0].is_empty() {
        return None;
    }
    let value = tokens[0].clone();
    let names = tokens[1..].join(" ");
    (!names.is_empty()).then_some((value, names))
}

/// One comma-separated layer-name list; entries trimmed, empty entries refused.
fn parse_layer_name_list(text: &str) -> Option<Vec<String>> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let mut names = Vec::new();
    for part in text.split(',') {
        let name = part.trim();
        if name.is_empty() {
            return None;
        }
        names.push(name.to_string());
    }
    Some(names)
}

/// The `PLOT` flag: `Y`/`N` plus the layer grammar's flag words.
fn parse_plottable_value(value: &str) -> Option<bool> {
    match value.to_ascii_uppercase().as_str() {
        "Y" | "YES" | "TRUE" | "1" => Some(true),
        "N" | "NO" | "FALSE" | "0" => Some(false),
        _ => None,
    }
}

/// Parse a boolean sysvar value used by `SETVAR GRID|SNAP|ORTHO`: `ON`/`OFF`,
/// `1`/`0` (case-insensitive).
pub fn parse_sysvar_toggle(argument: &str) -> Option<bool> {
    match argument.trim().to_ascii_uppercase().as_str() {
        "ON" | "1" => Some(true),
        "OFF" | "0" => Some(false),
        _ => None,
    }
}

/// The stable refusal for a `SETVAR` value outside its variable's grammar.
pub fn invalid_setvar_value_reason(name: &str, value: &str, expected: &str) -> String {
    format!("invalid SETVAR {name} value '{value}' (expected {expected})")
}

/// Split `text` into whitespace-separated tokens, removing double quotes and
/// keeping quoted whitespace inside one token.
///
/// An unbalanced quote is refused (`None`). Empty unquoted runs are ignored; an
/// explicitly quoted empty token (`""`) is preserved as an empty string so the
/// caller can refuse it by name.
pub(crate) fn split_quoted_tokens(text: &str) -> Option<Vec<String>> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut token_started = false;
    for ch in text.chars() {
        match ch {
            '"' => {
                quoted = !quoted;
                token_started = true;
            }
            c if c.is_whitespace() && !quoted => {
                if token_started || !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                    token_started = false;
                }
            }
            c => {
                current.push(c);
                token_started = true;
            }
        }
    }
    if quoted {
        return None;
    }
    if token_started || !current.is_empty() {
        tokens.push(current);
    }
    Some(tokens)
}
