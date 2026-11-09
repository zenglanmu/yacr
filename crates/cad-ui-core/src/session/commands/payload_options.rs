//! The typed-argument grammars of the 2026-11 P3 option batch:
//! `OVERKILL`, `REVCLOUD`, `WIPEOUT`, `TEXTMASK` and `TXT2MTXT`.
//!
//! These are **argument forms of canonical selectors**, never new commands and
//! never a second IPC. The bare selector keeps today's payload path (the
//! application's documented default); an argument only names the option the
//! typed line carries.
//!
//! The parsers here are pure: they decide whether the trimmed argument is inside
//! the selector's grammar and return the typed value the session dispatches.
//! Nothing is clamped, defaulted or silently repaired. An argument outside the
//! grammar returns `None`; the session turns that into the stable
//! `invalid_<selector>_argument_reason` and records nothing.
//!
//! Grammar summary (keywords ASCII case-insensitive):
//!
//! * `OVERKILL TOLERANCE <non-negative decimal>`;
//! * `OVERKILL IGNORE <COLOR|LAYER|LINETYPE|LTSCALE|LINEWEIGHT|TRANSPARENCY>`
//!   (several words accumulate; `THICKNESS`/`PLOTSTYLE`/`MATERIAL` are not
//!   modelled and are refused by name);
//! * `OVERKILL OPTIMIZE|NOOPTIMIZE`, `BREAK|NOBREAK`,
//!   `COLLINEAR|NOCOLLINEAR`, `ENDTOEND|NOENDTOEND`;
//! * `REVCLOUD ARC <positive decimal>` (chord length), `OBJECT`,
//!   `RECT|RECTANGULAR`, `POLY|POLYGONAL`, `FREEHAND`,
//!   `STYLE NORMAL|CALLIGRAPHY`, `MODIFY`;
//! * `WIPEOUT FRAMES ON|OFF|DISPLAY` (`DISPLAY` = display but not plot) and
//!   `WIPEOUT POLYLINE [ERASE YES|NO]`;
//! * `TEXTMASK OFFSET <non-negative decimal>` and
//!   `TEXTMASK MASK WIPEOUT|3DFACE|SOLID`;
//! * `TXT2MTXT COMBINE|SEPARATE`, `ORDER TOPDOWN|SELECT`, `WRAP`, `UNIFORM`.
//!
//! The `REVCLOUD` modes that AutoCAD captures interactively (`RECT`/`POLY`/
//! `FREEHAND`/`MODIFY`) are selection-based in this build: the current
//! selection must be exactly one `LWPOLYLINE`, whose vertices (or axis-aligned
//! bounding box for `RECT`) supply the path. A missing/unsupported selection is
//! a stable named refusal, never a fabricated point.

use cad_app::{OverkillSettings, RevCloudStyle, TextMaskKind, TextOrder, WipeoutFrames};

/// `OVERKILL <option>`: the Delete Duplicate Objects option line.
pub const OVERKILL_SELECTOR: &str = "OVERKILL";

/// `REVCLOUD <option>`: the revision-cloud mode/arc-length line.
pub const REVCLOUD_SELECTOR: &str = "REVCLOUD";

/// `WIPEOUT <option>`: the Frames / Polyline option line.
pub const WIPEOUT_SELECTOR: &str = "WIPEOUT";

/// `TEXTMASK <option>`: the offset / mask-type line.
pub const TEXTMASK_SELECTOR: &str = "TEXTMASK";

/// `TXT2MTXT <option>`: the settings line.
pub const TXT2MTXT_SELECTOR: &str = "TXT2MTXT";

/// One parsed `REVCLOUD` argument.
#[derive(Debug, Clone, PartialEq)]
pub enum RevCloudArgument {
    /// `OBJECT` or `ARC <chord>`: convert the one selected path.
    Object { arc_length: Option<f64> },
    /// `RECT` / `RECTANGULAR`: a cloud around the selection's bounding box.
    Rectangular,
    /// `POLY` / `POLYGONAL`: a cloud through the selected path's vertices.
    Polygonal,
    /// `FREEHAND`: a cloud through the selected path's vertices.
    Freehand,
    /// `STYLE NORMAL|CALLIGRAPHY`: the cloud style (Calligraphy is refused by
    /// the command layer, which has no style field).
    Style(RevCloudStyle),
    /// `MODIFY`: re-cloud the selected cloud and open it at its first vertex.
    Modify,
}

/// One parsed `WIPEOUT` argument.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WipeoutArgument {
    /// `FRAMES ON|OFF|DISPLAY`: the session `WIPEOUTFRAME` state.
    Frames(WipeoutFrames),
    /// `POLYLINE [ERASE YES|NO]`: convert the one selected closed polyline.
    Polyline { erase: bool },
}

/// One parsed `TEXTMASK` argument.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TextMaskArgument {
    /// `OFFSET <non-negative decimal>`: the mask box growth factor.
    Offset(f64),
    /// `MASK WIPEOUT|3DFACE|SOLID`: the mask type (the last two are refused by
    /// the command layer, which has no such entity).
    Mask(TextMaskKind),
}

/// One parsed `TXT2MTXT` argument.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Txt2MTextArgument {
    /// `COMBINE` (`true`) / `SEPARATE` (`false`).
    Combine(bool),
    /// `ORDER TOPDOWN|SELECT`.
    Order(TextOrder),
    /// `WRAP`: enable word-wrap.
    WordWrap,
    /// `UNIFORM`: force uniform line spacing (refused by the command layer).
    Uniform,
}

/// Parse the `OVERKILL` argument into the dialog's typed settings.
///
/// The base is [`OverkillSettings::default`] (the same set the bare selector
/// applies), so a line only changes the options it names. `IGNORE` accepts one
/// or more property words and may be repeated; the whole set accumulates for
/// this invocation. `THICKNESS`, `PLOTSTYLE` and `MATERIAL` are **not modelled**
/// on a `cad-db` entity, so `IGNORE <one of them>` is outside the grammar and is
/// refused with [`invalid_overkill_argument_reason`] (which names them).
pub fn parse_overkill_argument(argument: &str) -> Option<OverkillSettings> {
    let tokens: Vec<&str> = argument.split_whitespace().collect();
    if tokens.is_empty() {
        return None;
    }
    let mut settings = OverkillSettings::default();
    let mut index = 0;
    while index < tokens.len() {
        match tokens[index].to_ascii_uppercase().as_str() {
            "TOLERANCE" => {
                let value = super::parse_signed_decimal(tokens.get(index + 1)?)?;
                if !value.is_finite() || value < 0.0 {
                    return None;
                }
                settings.tolerance = value;
                index += 2;
            }
            "IGNORE" => {
                index += 1;
                let mut matched_any = false;
                while let Some(token) = tokens.get(index) {
                    let matched = match token.to_ascii_uppercase().as_str() {
                        "COLOR" => {
                            settings.ignore_color = true;
                            true
                        }
                        "LAYER" => {
                            settings.ignore_layer = true;
                            true
                        }
                        "LINETYPE" => {
                            settings.ignore_linetype = true;
                            true
                        }
                        "LTSCALE" => {
                            settings.ignore_linetype_scale = true;
                            true
                        }
                        "LINEWEIGHT" => {
                            settings.ignore_lineweight = true;
                            true
                        }
                        "TRANSPARENCY" => {
                            settings.ignore_transparency = true;
                            true
                        }
                        // Not modelled: outside the grammar, refused by name.
                        "THICKNESS" | "PLOTSTYLE" | "MATERIAL" => return None,
                        _ => false,
                    };
                    if !matched {
                        break;
                    }
                    matched_any = true;
                    index += 1;
                }
                if !matched_any {
                    return None;
                }
            }
            "OPTIMIZE" => {
                settings.optimize_segments = true;
                index += 1;
            }
            "NOOPTIMIZE" => {
                settings.optimize_segments = false;
                index += 1;
            }
            "BREAK" => {
                settings.do_not_break_polylines = false;
                index += 1;
            }
            "NOBREAK" => {
                settings.do_not_break_polylines = true;
                index += 1;
            }
            "COLLINEAR" => {
                settings.combine_partial_overlap = true;
                index += 1;
            }
            "NOCOLLINEAR" => {
                settings.combine_partial_overlap = false;
                index += 1;
            }
            "ENDTOEND" => {
                settings.combine_end_to_end = true;
                index += 1;
            }
            "NOENDTOEND" => {
                settings.combine_end_to_end = false;
                index += 1;
            }
            _ => return None,
        }
    }
    Some(settings)
}

/// Parse the `REVCLOUD` argument: one typed mode/option.
pub fn parse_revcloud_argument(argument: &str) -> Option<RevCloudArgument> {
    let text = argument.trim();
    let (keyword, rest) = match text.find([' ', '\t']) {
        Some(index) => (&text[..index], text[index..].trim()),
        None => (text, ""),
    };
    match keyword.to_ascii_uppercase().as_str() {
        "OBJECT" if rest.is_empty() => Some(RevCloudArgument::Object { arc_length: None }),
        "ARC" => {
            let chord = super::parse_positive_decimal(rest)?;
            Some(RevCloudArgument::Object {
                arc_length: Some(chord),
            })
        }
        "RECT" | "RECTANGULAR" if rest.is_empty() => Some(RevCloudArgument::Rectangular),
        "POLY" | "POLYGONAL" if rest.is_empty() => Some(RevCloudArgument::Polygonal),
        "FREEHAND" if rest.is_empty() => Some(RevCloudArgument::Freehand),
        "STYLE" => match rest.to_ascii_uppercase().as_str() {
            "NORMAL" => Some(RevCloudArgument::Style(RevCloudStyle::Normal)),
            "CALLIGRAPHY" => Some(RevCloudArgument::Style(RevCloudStyle::Calligraphy)),
            _ => None,
        },
        "MODIFY" if rest.is_empty() => Some(RevCloudArgument::Modify),
        _ => None,
    }
}

/// Parse the `WIPEOUT` argument: Frames or Polyline.
pub fn parse_wipeout_argument(argument: &str) -> Option<WipeoutArgument> {
    let tokens: Vec<&str> = argument.split_whitespace().collect();
    if tokens.is_empty() {
        return None;
    }
    match tokens[0].to_ascii_uppercase().as_str() {
        "FRAMES" => match (
            tokens.len(),
            tokens.get(1).map(|word| word.to_ascii_uppercase()),
        ) {
            (2, Some(mode)) => match mode.as_str() {
                "ON" => Some(WipeoutArgument::Frames(WipeoutFrames::On)),
                "OFF" => Some(WipeoutArgument::Frames(WipeoutFrames::Off)),
                "DISPLAY" => Some(WipeoutArgument::Frames(WipeoutFrames::DisplayNotPlot)),
                _ => None,
            },
            _ => None,
        },
        "POLYLINE" => match tokens.len() {
            1 => Some(WipeoutArgument::Polyline { erase: false }),
            3 if tokens[1].eq_ignore_ascii_case("ERASE") => {
                let erase = match tokens[2].to_ascii_uppercase().as_str() {
                    "YES" | "Y" | "1" | "ON" => true,
                    "NO" | "N" | "0" | "OFF" => false,
                    _ => return None,
                };
                Some(WipeoutArgument::Polyline { erase })
            }
            _ => None,
        },
        _ => None,
    }
}

/// Parse the `TEXTMASK` argument: offset factor or mask type.
pub fn parse_textmask_argument(argument: &str) -> Option<TextMaskArgument> {
    let text = argument.trim();
    let (keyword, rest) = match text.find([' ', '\t']) {
        Some(index) => (&text[..index], text[index..].trim()),
        None => (text, ""),
    };
    match keyword.to_ascii_uppercase().as_str() {
        "OFFSET" => {
            let offset = super::parse_signed_decimal(rest)?;
            (offset >= 0.0).then_some(TextMaskArgument::Offset(offset))
        }
        "MASK" => match rest.to_ascii_uppercase().as_str() {
            "WIPEOUT" => Some(TextMaskArgument::Mask(TextMaskKind::Wipeout)),
            "3DFACE" => Some(TextMaskArgument::Mask(TextMaskKind::Face3d)),
            "SOLID" => Some(TextMaskArgument::Mask(TextMaskKind::Solid)),
            _ => None,
        },
        _ => None,
    }
}

/// Parse the `TXT2MTXT` argument: one settings toggle.
pub fn parse_txt2mtext_argument(argument: &str) -> Option<Txt2MTextArgument> {
    let tokens: Vec<&str> = argument.split_whitespace().collect();
    if tokens.is_empty() {
        return None;
    }
    match tokens[0].to_ascii_uppercase().as_str() {
        "COMBINE" if tokens.len() == 1 => Some(Txt2MTextArgument::Combine(true)),
        "SEPARATE" if tokens.len() == 1 => Some(Txt2MTextArgument::Combine(false)),
        "ORDER" if tokens.len() == 2 => match tokens[1].to_ascii_uppercase().as_str() {
            "TOPDOWN" => Some(Txt2MTextArgument::Order(TextOrder::TopDown)),
            "SELECT" => Some(Txt2MTextArgument::Order(TextOrder::Selection)),
            _ => None,
        },
        "WRAP" if tokens.len() == 1 => Some(Txt2MTextArgument::WordWrap),
        "UNIFORM" if tokens.len() == 1 => Some(Txt2MTextArgument::Uniform),
        _ => None,
    }
}

/// The stable refusal for an `OVERKILL` argument outside the grammar.
pub fn invalid_overkill_argument_reason(text: &str) -> String {
    format!(
        "unsupported CAD command: {} (OVERKILL needs TOLERANCE <non-negative decimal>, \
         IGNORE <COLOR|LAYER|LINETYPE|LTSCALE|LINEWEIGHT|TRANSPARENCY>, OPTIMIZE|NOOPTIMIZE, \
         BREAK|NOBREAK, COLLINEAR|NOCOLLINEAR or ENDTOEND|NOENDTOEND; THICKNESS, PLOTSTYLE and \
         MATERIAL are not modelled)",
        text.trim()
    )
}

/// The stable refusal for a `REVCLOUD` argument outside the grammar.
pub fn invalid_revcloud_argument_reason(text: &str) -> String {
    format!(
        "unsupported CAD command: {} (REVCLOUD needs ARC <positive decimal>, OBJECT, \
         RECT|RECTANGULAR, POLY|POLYGONAL, FREEHAND, STYLE NORMAL|CALLIGRAPHY or MODIFY; \
         RECT/POLY/FREEHAND/MODIFY take the one selected LWPOLYLINE as the path)",
        text.trim()
    )
}

/// The stable refusal for a `WIPEOUT` argument outside the grammar.
pub fn invalid_wipeout_argument_reason(text: &str) -> String {
    format!(
        "unsupported CAD command: {} (WIPEOUT needs FRAMES ON|OFF|DISPLAY or POLYLINE \
         [ERASE YES|NO])",
        text.trim()
    )
}

/// The stable refusal for a `TEXTMASK` argument outside the grammar.
pub fn invalid_textmask_argument_reason(text: &str) -> String {
    format!(
        "unsupported CAD command: {} (TEXTMASK needs OFFSET <non-negative decimal> or \
         MASK WIPEOUT|3DFACE|SOLID; 3DFACE and SOLID are refused by the command)",
        text.trim()
    )
}

/// The stable refusal for a `TXT2MTXT` argument outside the grammar.
pub fn invalid_txt2mtext_argument_reason(text: &str) -> String {
    format!(
        "unsupported CAD command: {} (TXT2MTXT needs COMBINE, SEPARATE, ORDER TOPDOWN|SELECT, \
         WRAP or UNIFORM; UNIFORM is refused by the command)",
        text.trim()
    )
}
