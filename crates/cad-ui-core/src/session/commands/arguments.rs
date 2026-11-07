//! Typed command-line arguments (`ZOOM 2`, `SNAPUNIT 0.5`, `PDMODE 3`, …).
//!
//! Split out of [`super`] to keep the command vocabulary module inside the
//! code-size limit; every item is re-exported by [`super`], so the public paths
//! (`cad_ui_core::session::parse_zoom_argument`, `ARGUMENT_SELECTORS`, …) are
//! unchanged. The grammar is deliberately narrow and refuses (never clamps) a
//! value outside it; the caller turns `None` into the stable refusal from the
//! matching `invalid_*_reason`.

use super::{canonical_command_name, ARGUMENT_SELECTORS};

/// AutoCAD's `ZOOM n` / `ZOOM ALL`: the first argument-taking selector.
///
/// A positive factor reuses the `ZOOMIN`/`ZOOMOUT` step path (anchored at the
/// viewport centre); `ALL`/`EXTENTS` dispatches the ordinary `FIT`; `WINDOW`
/// starts the two-corner capture and `PREVIOUS` restores the last discrete view
/// (see [`ZoomArgument`]). The last two still carry **no** `CommandId` of their
/// own: they ride [`CommandId::Zoom`] with their own payloads.
pub const ARGUMENT_SELECTOR: &str = "ZOOM";

/// AutoCAD's `SNAPUNIT <value>`: the session grid-snap spacing sysvar.
///
/// The value is a positive decimal in drawing units and is stored as session
/// state only (no database, history or preference write). It is the spacing the
/// `SNAP` (F9) quantiser uses.
pub const SNAPUNIT_SELECTOR: &str = "SNAPUNIT";

/// AutoCAD's `PDMODE <n>`: the drawing-global POINT display **mode** sysvar.
///
/// The value is an integer stored **raw** in the drawing header (`$PDMODE`), so
/// it is a document write (one revision bump, no undo entry), not session state.
/// The display layer interprets the bits.
pub const PDMODE_SELECTOR: &str = "PDMODE";

/// AutoCAD's `PDSIZE <n>`: the drawing-global POINT display **size** sysvar.
///
/// The value is a finite decimal stored **raw** in the drawing header
/// (`$PDSIZE`): `> 0` absolute drawing units, `0` viewport-relative, `< 0` a
/// percentage of the viewport height. A document write like
/// [`PDMODE_SELECTOR`], not session state.
pub const PDSIZE_SELECTOR: &str = "PDSIZE";

/// The `SPACE <argument>` selector: switch the active drawing space.
///
/// This is the same [`CommandId::SwitchSpace`] command the IPC/panel path uses;
/// the typed argument only saves a second IPC for a layout-tab click. `MODEL`
/// selects model space, `#<u128>` names a `LayoutId` directly, and any other
/// whole argument is resolved as a layout **display name** (exact match first,
/// then a unique case-insensitive one). A bare `SPACE` keeps the existing
/// explicit "needs a space payload" refusal.
pub const SPACE_SELECTOR: &str = "SPACE";

/// One parsed argument of the `ZOOM` selector.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ZoomArgument {
    /// A positive finite **scale factor**: `> 1` zooms in, `< 1` zooms out —
    /// the same convention as `ZOOMIN`/`ZOOMOUT` and
    /// [`cad_app::CommandPayload::ZoomAt`]. The zoom is anchored at the
    /// **viewport centre**, exactly like those cursor-less button steps.
    Factor(f64),
    /// `ALL` (the written-out spelling) or `EXTENTS`: frame the whole drawing,
    /// i.e. the [`CommandId::FitDrawing`] path.
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

/// Split a typed line into an argument-taking selector and its argument.
///
/// `Some((selector, argument))` only when `text` starts with one of
/// [`ARGUMENT_SELECTORS`] followed by one or more spaces/tabs and a non-empty
/// argument (`ZOOM 2`, `  snapunit\t0.5 `); a name-only line (`ZOOM`,
/// `SNAPUNIT`) returns `None` so it keeps the name-only funnel and its existing
/// payload refusal. The argument keeps its internal text: `ZOOM 2 3` reaches
/// [`parse_zoom_argument`] as `2 3` and is refused there rather than being split
/// a second time.
pub fn typed_command_argument(text: &str) -> Option<(&'static str, &str)> {
    let trimmed = text.trim();
    let split = trimmed.find([' ', '\t'])?;
    let (name, rest) = trimmed.split_at(split);
    let argument = rest.trim_start_matches([' ', '\t']).trim_end();
    if argument.is_empty() {
        return None;
    }
    let selector = canonical_command_name(name)?;
    ARGUMENT_SELECTORS
        .contains(&selector)
        .then_some((selector, argument))
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

/// Whether `text` is a plain positive decimal number (`digits[.digits]`).
///
/// A leading sign is refused here even though
/// [`parse_signed_decimal`] accepts one: `ZOOM`/`SNAPUNIT` have always refused
/// `+5`/`-5` (a spacing is written unsigned), and widening their grammar would
/// change an existing refusal contract.
fn parse_positive_decimal(text: &str) -> Option<f64> {
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
fn parse_signed_decimal(text: &str) -> Option<f64> {
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
