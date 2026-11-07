//! Typed-argument grammars for the command-line selectors.
//!
//! Split out of `commands.rs` (which is at the file-size ceiling). Every parser
//! here is pure: it decides whether the trimmed argument is inside its selector's
//! founded grammar and, when it is, returns the typed value the session
//! dispatches. Nothing is clamped, defaulted or silently repaired.
//!
//! `ARGUMENT_SELECTORS` in the parent module is the admission list; an argument
//! parser only ever sees text for a selector that list already admitted.

use cad_app::PeditOption;

/// AutoCAD's `LAYERSTATE <option>`: the bounded named layer-state store.
pub const LAYERSTATE_SELECTOR: &str = "LAYERSTATE";

/// AutoCAD's `LAYWALK <names>`: show only the named layers plus the current one.
pub const LAYWALK_SELECTOR: &str = "LAYWALK";

/// AutoCAD's `LTSCALE <value>`: the drawing-global linetype scale sysvar.
pub const LTSCALE_SELECTOR: &str = "LTSCALE";

/// AutoCAD's `LAYERPMODE [ON|OFF]`: the `LAYERP` snapshot-recording gate.
pub const LAYERPMODE_SELECTOR: &str = "LAYERPMODE";

/// One parsed argument of the `ZOOM` selector.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ZoomArgument {
    /// A positive finite **scale factor**: `> 1` zooms in, `< 1` zooms out —
    /// the same convention as `ZOOMIN`/`ZOOMOUT` and
    /// [`cad_app::CommandPayload::ZoomAt`]. The zoom is anchored at the
    /// **viewport centre**, exactly like those cursor-less button steps.
    Factor(f64),
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

/// Parse the argument of a `ZOOM` selector: a positive decimal factor, or
/// `ALL` / `EXTENTS` / `WINDOW` / `PREVIOUS` (case-insensitive, surrounding
/// whitespace ignored).
///
/// The numeric grammar is deliberately narrow — ASCII digits with at most one
/// `.` that has digits on **both** sides (`2`, `0.5`, `1.25`, `2.0`). Signs
/// (`-1`, `+2`), exponents (`1e3`), a bare `.`/`1.`, `NaN`/`inf` and zero are
/// all refused with `None`; the caller turns that into the stable refusal from
/// [`invalid_zoom_argument_reason`]. Nothing is clamped and no default factor is
/// invented.
///
/// The two word arguments are the **full** spellings only: no `W`/`P`
/// abbreviation is admitted (this router never guesses at a partial word), and
/// `ZOOM EXTENTS` is still resolved earlier by the pre-existing alias onto `FIT`.
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
    parse_positive_decimal(key).map(ZoomArgument::Factor)
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
        "unsupported CAD command: {} (ZOOM needs a positive factor, ALL, EXTENTS, WINDOW or \
         PREVIOUS)",
        text.trim()
    )
}

/// The stable rejection reason for a `PEDIT` argument outside the grammar.
///
/// The typed line is echoed and the implemented option set (with its single
/// letters) is named, so a typo or an unimplemented AutoCAD option is refused by
/// name. The NOT IMPLEMENTED options are listed too, so nobody reads the subset
/// as a promise that `JOIN`/`WIDTH`/`SPLINE` work.
pub fn invalid_pedit_argument_reason(text: &str) -> String {
    format!(
        "unsupported CAD command: {} (PEDIT needs CLOSE, OPEN or REVERSE, or C, O, R; Join, \
         Width, Halfwidth, Fit, Spline, Decurve, Ltype gen, Undo, Multiple and Edit vertex are \
         NOT implemented)",
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
