//! The typed-argument grammars of the paper-space/session/point-style batch:
//! `VPLAYER`, `MVSETUP`, `APERTURE`, `PTYPE` and `REVCLOUDPROPERTIES`.
//!
//! These are **argument forms of canonical selectors**, never new commands and
//! never a second IPC. The bare selector keeps today's name-only path (the
//! read-only `VPLAYER`/`PTYPE`/`APERTURE`/`REVCLOUDPROPERTIES` report, or the
//! application's documented payload refusal); a typed argument only names the
//! value the existing handler already models.
//!
//! The parsers here are pure: they decide whether the trimmed argument is inside
//! the selector's grammar and return the typed value the session dispatches.
//! Nothing is clamped, defaulted or silently repaired. Every malformed form
//! returns the selector's stable `*.` refusal sentence and changes nothing.
//!
//! Grammar summary (keywords ASCII case-insensitive):
//!
//! * `VPLAYER FREEZE|THAW <layer,...> [<viewport-index>]` — the tokens are kept
//!   verbatim (comma lists and the optional `[n]`/bare-index spelling) because
//!   the application's `app_layout_paper` handler parses and validates them;
//!   `VPLAYER ?` is the read-only report and takes no payload;
//! * `MVSETUP <rows> <cols> <scale>` — exactly three tokens; the positive grid
//!   counts and the viewport-scale name/ratio are resolved by the application;
//! * `APERTURE <n>` — one whole pixel value inside
//!   [`cad_app::viewer_config::ViewAperture::RANGE`] (AutoCAD's `1..=50`);
//! * `PTYPE <code> [size]` — the `PDMODE` integer and optional `PDSIZE` decimal
//!   through the **existing** point-display write path
//!   ([`super::parse_pdmode_argument`] / [`super::parse_pdsize_argument`]);
//! * `REVCLOUDPROPERTIES <arc-length>` — one finite positive decimal stored in
//!   the session token `REVCLOUD` already reads.

/// `VPLAYER FREEZE|THAW <layer,...> [<index>]`: the per-viewport layer line.
pub const VPLAYER_SELECTOR: &str = "VPLAYER";

/// `MVSETUP <rows> <cols> <scale>`: the viewport-grid line.
pub const MVSETUP_SELECTOR: &str = "MVSETUP";

/// `APERTURE <n>`: the session object-snap aperture token.
pub const APERTURE_SELECTOR: &str = "APERTURE";

/// `PTYPE <code> [size]`: the `$PDMODE`/`$PDSIZE` write form.
pub const PTYPE_SELECTOR: &str = "PTYPE";

/// `REVCLOUDPROPERTIES <arc-length>`: the session revision-cloud chord token.
pub const REVCLOUDPROPERTIES_SELECTOR: &str = "REVCLOUDPROPERTIES";

/// `SPACETRANS <distance>`: the model/paper distance conversion.
pub const SPACETRANS_SELECTOR: &str = "SPACETRANS";

/// One parsed `VPLAYER` argument.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VPlayerArgument {
    /// `?`: the read-only report, which carries no payload.
    Report,
    /// `FREEZE|THAW <layer,...> [<index>]` exactly as the application parses it.
    Tokens(Vec<String>),
}

/// One parsed `PTYPE` argument: the raw `$PDMODE` plus the optional `$PDSIZE`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PointTypeArgument {
    /// `$PDMODE` — the raw integer bit field, never rounded.
    pub mode: i16,
    /// `$PDSIZE` when the line carried a second token.
    pub size: Option<f64>,
}

/// Parse the `VPLAYER` argument: the report marker or the handler's tokens.
///
/// The operation must be `FREEZE`/`F` or `THAW`/`T` and at least one layer
/// token must precede the optional `[n]` index; everything else is outside the
/// grammar. A bare whole number is deliberately **kept as a token** rather than
/// read as an index here: the application resolves layer names first, so
/// `FREEZE 2` addresses the layer named `2` (the existing `split_viewport_index`
/// rule), while `FREEZE WALLS 2` is the convenience index form.
pub fn parse_vplayer_argument(argument: &str) -> Option<VPlayerArgument> {
    let trimmed = argument.trim();
    if trimmed == "?" {
        return Some(VPlayerArgument::Report);
    }
    let tokens: Vec<String> = trimmed.split_whitespace().map(str::to_string).collect();
    let operation = tokens.first()?.to_ascii_uppercase();
    if !matches!(operation.as_str(), "FREEZE" | "F" | "THAW" | "T") {
        return None;
    }
    if tokens.len() < 2 {
        return None;
    }
    let last = tokens.last()?;
    if last.starts_with('[') || last.ends_with(']') {
        let inner = last.strip_prefix('[')?.strip_suffix(']')?;
        inner.trim().parse::<usize>().ok()?;
        // `FREEZE [2]` has an index but no layer.
        if tokens.len() == 2 {
            return None;
        }
    }
    Some(VPlayerArgument::Tokens(tokens))
}

/// Parse the `MVSETUP` argument: exactly `<rows> <cols> <scale>`.
///
/// The two grid counts must be positive whole numbers; `0`, a sign, a fraction
/// or an overflow is outside the grammar (the application repeats the check and
/// owns the total-viewport bound). The scale token is kept verbatim for the
/// application's viewport-scale resolver (`resolve_viewport_scale`), which names
/// a `VPSCALE` scale, a ratio (`1:100`) or a number and refuses an unusable one.
pub fn parse_mvsetup_argument(argument: &str) -> Option<Vec<String>> {
    let tokens: Vec<String> = argument.split_whitespace().map(str::to_string).collect();
    if tokens.len() != 3 {
        return None;
    }
    for count in &tokens[..2] {
        let value: u32 = count.parse().ok()?;
        if value == 0 {
            return None;
        }
    }
    Some(tokens)
}

/// Parse the `APERTURE` argument: one whole pixel value in AutoCAD's range.
///
/// Only ASCII digits are admitted (no sign, fraction or exponent). A numeric
/// value outside [`cad_app::viewer_config::ViewAperture::RANGE`] gets the
/// distinct `aperture.out_of_range` refusal instead of being clamped; a
/// non-numeric shape gets `aperture.invalid_argument`. Nothing is stored.
pub fn parse_aperture_argument(argument: &str) -> Result<u32, String> {
    let text = argument.trim();
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return Err(invalid_aperture_argument_reason(argument));
    }
    let value: u32 = text
        .parse()
        .map_err(|_| invalid_aperture_argument_reason(argument))?;
    if !cad_app::viewer_config::ViewAperture::RANGE.contains(&value) {
        return Err(aperture_out_of_range_reason(argument));
    }
    Ok(value)
}

/// Parse the `PTYPE` argument: `$PDMODE` and an optional `$PDSIZE`.
///
/// The two numeric grammars are the existing `PDMODE`/`PDSIZE` ones, so a
/// fractional mode (`PTYPE 3.5`), an out-of-`i16` mode (`PTYPE 40000`) or a
/// non-finite size is refused with the same rule as the dedicated commands.
/// Anything other than one or two tokens is `ptype.invalid_argument`.
pub fn parse_ptype_argument(argument: &str) -> Result<PointTypeArgument, String> {
    let trimmed = argument.trim();
    let tokens: Vec<&str> = trimmed.split_whitespace().collect();
    match tokens.as_slice() {
        [code] => Ok(PointTypeArgument {
            mode: super::parse_pdmode_argument(code)
                .ok_or_else(|| invalid_ptype_argument_reason(trimmed))?,
            size: None,
        }),
        [code, size] => Ok(PointTypeArgument {
            mode: super::parse_pdmode_argument(code)
                .ok_or_else(|| invalid_ptype_argument_reason(trimmed))?,
            size: Some(
                super::parse_pdsize_argument(size)
                    .ok_or_else(|| invalid_ptype_argument_reason(trimmed))?,
            ),
        }),
        _ => Err(invalid_ptype_argument_reason(trimmed)),
    }
}

/// Parse the `REVCLOUDPROPERTIES` argument: one finite positive arc length.
///
/// The shape is the narrow plain decimal `ZOOM`/`SNAPUNIT` use; zero, a negative
/// value, an exponent and `NaN`/`inf` are outside the grammar. The value is
/// stored only after the application repeats the check.
pub fn parse_revcloud_properties_argument(argument: &str) -> Result<f64, String> {
    super::parse_positive_decimal(argument.trim())
        .ok_or_else(|| invalid_revcloudproperties_argument_reason(argument))
}

/// The stable `vplayer.invalid_argument` refusal sentence.
pub fn invalid_vplayer_argument_reason(text: &str) -> String {
    format!(
        "vplayer.invalid_argument: VPLAYER needs `FREEZE|THAW <layer,...> [<viewport-index>]` \
         (for example `VPLAYER FREEZE WALLS,DIMS [0]`) or `?` for the read-only report; nothing \
         was changed: {}",
        text.trim()
    )
}

/// The stable `mvsetup.invalid_argument` refusal sentence.
pub fn invalid_mvsetup_argument_reason(text: &str) -> String {
    format!(
        "mvsetup.invalid_argument: MVSETUP needs `<rows> <cols> <scale>` (for example \
         `MVSETUP 2 2 1:100`) with positive whole grid counts; nothing was changed: {}",
        text.trim()
    )
}

/// The stable `aperture.invalid_argument` refusal sentence.
pub fn invalid_aperture_argument_reason(text: &str) -> String {
    format!(
        "aperture.invalid_argument: APERTURE needs one whole pixel value \
         (for example `APERTURE 10`); nothing was changed: {}",
        text.trim()
    )
}

/// The stable `aperture.out_of_range` refusal sentence.
pub fn aperture_out_of_range_reason(text: &str) -> String {
    format!(
        "aperture.out_of_range: APERTURE must be {}..={} logical pixels (AutoCAD's range); \
         nothing was changed: {}",
        cad_app::viewer_config::ViewAperture::RANGE.start(),
        cad_app::viewer_config::ViewAperture::RANGE.end(),
        text.trim()
    )
}

/// The stable `ptype.invalid_argument` refusal sentence.
pub fn invalid_ptype_argument_reason(text: &str) -> String {
    format!(
        "ptype.invalid_argument: PTYPE needs `<code> [size]` where the code is one whole \
         `$PDMODE` value and the optional size is one finite `$PDSIZE` decimal; nothing was \
         changed: {}",
        text.trim()
    )
}

/// The stable `revcloudproperties.invalid_argument` refusal sentence.
pub fn invalid_revcloudproperties_argument_reason(text: &str) -> String {
    format!(
        "revcloudproperties.invalid_argument: REVCLOUDPROPERTIES needs one finite positive \
         arc length (for example `REVCLOUDPROPERTIES 15`); nothing was changed: {}",
        text.trim()
    )
}

/// Parse the `SPACETRANS` argument: one finite positive plain decimal distance.
///
/// The host converts it as a **model-space** distance for the active layout
/// viewport and reports the inverse direction too. A zero/negative, non-finite,
/// exponent or extra-token form is outside the grammar and is refused by name;
/// whether the session is in paper space and whether the viewport resolves are
/// the application's semantic checks.
pub fn parse_spacetrans_argument(argument: &str) -> Option<f64> {
    let text = argument.trim();
    if text.is_empty()
        || !text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b'.')
    {
        return None;
    }
    if text.matches('.').count() > 1 {
        return None;
    }
    let value: f64 = text.parse().ok()?;
    (value.is_finite() && value > 0.0).then_some(value)
}

/// The stable `spacetrans.invalid_argument` refusal sentence.
pub fn invalid_spacetrans_argument_reason(text: &str) -> String {
    format!(
        "spacetrans.invalid_argument: SPACETRANS needs one finite positive distance (for example \
         `SPACETRANS 100`); nothing was changed: {}",
        text.trim()
    )
}
