//! The typed argument forms of the 2026-11-07 2D text batch:
//! `JUSTIFYTEXT <H> <V>`, `TSCALE <factor>`, `TEXTALIGN <mode>` /
//! `TEXTALIGN DISTRIBUTE [X|Y]`, `TCOUNT <start> <step> [prefix] [suffix]` and
//! `TCIRCLE <shape>`.
//!
//! Each is an **argument form of its base selector**, not a command of its own:
//! there is no second IPC and no second write path. The selection is still the
//! input, so this module only parses the typed argument and dispatches the
//! matching [`cad_app::CommandPayload`].
//!
//! * `TJUST` is an alias of `JUSTIFYTEXT` ([`COMMAND_ALIASES`](super::COMMAND_ALIASES));
//!   the router normalizes it onto `JUSTIFYTEXT` **before** this module sees the
//!   selector, so the alias shares this grammar exactly.
//! * A bare `JUSTIFYTEXT`/`TSCALE`/`TEXTALIGN`/`TCOUNT`/`TCIRCLE` keeps the
//!   name-only funnel; the application refuses it by name
//!   (`*.needs_argument` / `tscale.needs_factor`), never with an invented
//!   default.
//! * An argument outside the grammar is refused with the stable
//!   [`super::invalid_justifytext_argument_reason`] family, recorded for
//!   `cad_status.error`, and changes nothing.
//!
//! `TEXTMASK`/`TEXTUNMASK` take no argument (the selection is the whole input),
//! so they are not in this module or in `ARGUMENT_SELECTORS`.

use cad_app::{
    CommandId, CommandPayload, JustifyTextCommand, TCircleCommand, TCircleSizeMode, TCountCommand,
    TextAlignAxis, TextAlignCommand, TextAlignEdge, TextAlignSpacing, TextAlignSpacingCommand,
    TextBoundaryShape, TextScaleCommand,
};
use cad_domain::{TextAlignH, TextAlignV};

use super::diagnostics::cad_error_text;
use super::CadSession;

/// `JUSTIFYTEXT <H> <V>`: set only the alignment pair of the selected text.
///
/// `TJUST` normalizes onto this canonical selector; there is no second grammar.
pub const JUSTIFYTEXT_SELECTOR: &str = "JUSTIFYTEXT";

/// `TSCALE <factor>`: scale the selected text glyph height / MTEXT wrap width.
pub const TSCALE_SELECTOR: &str = "TSCALE";

/// `TEXTALIGN <edge>`: move the selected text to one common edge/centre.
pub const TEXTALIGN_SELECTOR: &str = "TEXTALIGN";

/// `TCOUNT <start> <step> [prefix] [suffix]`: number the selected text.
pub const TCOUNT_SELECTOR: &str = "TCOUNT";

/// `TCIRCLE CIRCLE|RECTANGLE`: draw an estimated-bounds boundary per text.
pub const TCIRCLE_SELECTOR: &str = "TCIRCLE";

/// Parse `JUSTIFYTEXT <H> <V>`: exactly two whitespace-separated words.
///
/// H accepts `LEFT`, `CENTER`, `RIGHT`; V accepts `BASELINE`, `BOTTOM`, `MIDDLE`,
/// `TOP` (case-insensitive). Every other shape — a missing half, a third word, an
/// abbreviation — is refused with `None`; nothing is defaulted.
pub fn parse_justifytext_argument(argument: &str) -> Option<(TextAlignH, TextAlignV)> {
    let tokens: Vec<&str> = argument.split_whitespace().collect();
    if tokens.len() != 2 {
        return None;
    }
    let h_align = if tokens[0].eq_ignore_ascii_case("LEFT") {
        TextAlignH::Left
    } else if tokens[0].eq_ignore_ascii_case("CENTER") {
        TextAlignH::Center
    } else if tokens[0].eq_ignore_ascii_case("RIGHT") {
        TextAlignH::Right
    } else {
        return None;
    };
    let v_align = if tokens[1].eq_ignore_ascii_case("BASELINE") {
        TextAlignV::Baseline
    } else if tokens[1].eq_ignore_ascii_case("BOTTOM") {
        TextAlignV::Bottom
    } else if tokens[1].eq_ignore_ascii_case("MIDDLE") {
        TextAlignV::Middle
    } else if tokens[1].eq_ignore_ascii_case("TOP") {
        TextAlignV::Top
    } else {
        return None;
    };
    Some((h_align, v_align))
}

/// Parse `TSCALE <factor>`: one positive decimal factor.
///
/// The numeric shape is the same narrow positive decimal `SNAPUNIT`/`ZOOM` use
/// (`2`, `0.5`, `1.25`): unsigned ASCII digits with at most one `.` that has
/// digits on both sides. `0`, `-1`, `+2`, `1e3` and every non-number are refused
/// with `None`; the caller turns that into the stable refusal.
pub fn parse_tscale_argument(argument: &str) -> Option<f64> {
    super::commands::parse_positive_decimal(argument.trim())
}

/// Parse `TEXTALIGN <mode>`: the 15 full alignment words.
///
/// Single-axis: `LEFT`, `RIGHT`, `CENTER`, `TOP`, `MIDDLE`, `BOTTOM`. Combined
/// (AutoCAD 2026 `TEXTALIGNMODE`): `TOPLEFT`, `TOPCENTER`, `TOPRIGHT`,
/// `MIDDLELEFT`, `MIDDLECENTER`, `MIDDLERIGHT`, `BOTTOMLEFT`, `BOTTOMCENTER`,
/// `BOTTOMRIGHT`. No abbreviation is admitted: the host grammar does not guess
/// at a partial word, so `L`/`C`/`T`/`TOP-LEFT` are refused rather than mapped.
pub fn parse_textalign_argument(argument: &str) -> Option<TextAlignEdge> {
    let upper = argument.trim().to_ascii_uppercase();
    Some(match upper.as_str() {
        "LEFT" => TextAlignEdge::Left,
        "RIGHT" => TextAlignEdge::Right,
        "CENTER" => TextAlignEdge::Center,
        "TOP" => TextAlignEdge::Top,
        "MIDDLE" => TextAlignEdge::Middle,
        "BOTTOM" => TextAlignEdge::Bottom,
        "TOPLEFT" => TextAlignEdge::TopLeft,
        "TOPCENTER" => TextAlignEdge::TopCenter,
        "TOPRIGHT" => TextAlignEdge::TopRight,
        "MIDDLELEFT" => TextAlignEdge::MiddleLeft,
        "MIDDLECENTER" => TextAlignEdge::MiddleCenter,
        "MIDDLERIGHT" => TextAlignEdge::MiddleRight,
        "BOTTOMLEFT" => TextAlignEdge::BottomLeft,
        "BOTTOMCENTER" => TextAlignEdge::BottomCenter,
        "BOTTOMRIGHT" => TextAlignEdge::BottomRight,
        _ => return None,
    })
}

/// Parse `TEXTALIGN <spacing option>`.
///
/// * `DISTRIBUTE` (the default axis is Y) or `DISTRIBUTE X` / `DISTRIBUTE Y`;
/// * `SETSPACING [<distance> [X|Y]]` — the distance is a non-negative decimal
///   and the axis defaults to Y;
/// * `CURRENTVERTICAL`, `CURRENTHORIZONTAL` — whole-selection alignment using
///   each text's own stored justification.
///
/// A bare `SETSPACING` parses to `distance: None` so the application can answer
/// with `textalign.set_spacing_needs_value` rather than inventing a distance.
/// Any other shape is `None` and becomes the stable invalid-argument refusal.
pub fn parse_textalign_spacing_argument(argument: &str) -> Option<TextAlignSpacing> {
    let tokens: Vec<&str> = argument.split_whitespace().collect();
    if tokens.is_empty() {
        return None;
    }
    if tokens.len() == 1 {
        return match tokens[0].to_ascii_uppercase().as_str() {
            "DISTRIBUTE" => Some(TextAlignSpacing::Distribute(TextAlignAxis::Y)),
            "SETSPACING" => Some(TextAlignSpacing::SetSpacing {
                distance: None,
                axis: TextAlignAxis::Y,
            }),
            "CURRENTVERTICAL" => Some(TextAlignSpacing::CurrentVertical),
            "CURRENTHORIZONTAL" => Some(TextAlignSpacing::CurrentHorizontal),
            _ => None,
        };
    }
    if tokens[0].eq_ignore_ascii_case("DISTRIBUTE") && tokens.len() == 2 {
        return match tokens[1].to_ascii_uppercase().as_str() {
            "X" => Some(TextAlignSpacing::Distribute(TextAlignAxis::X)),
            "Y" => Some(TextAlignSpacing::Distribute(TextAlignAxis::Y)),
            _ => None,
        };
    }
    if tokens[0].eq_ignore_ascii_case("SETSPACING") && tokens.len() <= 3 {
        let distance = super::commands::parse_signed_decimal(tokens[1])?;
        if !distance.is_finite() || distance < 0.0 {
            return None;
        }
        let axis = match tokens.get(2) {
            None => TextAlignAxis::Y,
            Some(token) if token.eq_ignore_ascii_case("X") => TextAlignAxis::X,
            Some(token) if token.eq_ignore_ascii_case("Y") => TextAlignAxis::Y,
            Some(_) => return None,
        };
        return Some(TextAlignSpacing::SetSpacing {
            distance: Some(distance),
            axis,
        });
    }
    None
}

/// Parse `TCOUNT <start> <step> [prefix] [suffix]`.
///
/// `start` is a signed whole number and `step` a whole number; 2, 3 or 4 tokens
/// are admitted (2 = bare run, 3 = prefix, 4 = prefix + suffix). Token 3 is
/// always the prefix, so a suffix cannot be given without one.
///
/// The **step sign** is accepted here and refused by the application with the
/// stable `tcount.invalid_step` code (zero/negative), so the two refusal sites
/// say the same thing; the application is the authority.
pub fn parse_tcount_argument(argument: &str) -> Option<TCountCommand> {
    let tokens: Vec<&str> = argument.split_whitespace().collect();
    if tokens.len() < 2 || tokens.len() > 4 {
        return None;
    }
    let start: i64 = tokens[0].parse().ok()?;
    let step: i64 = tokens[1].parse().ok()?;
    Some(TCountCommand {
        start,
        step,
        prefix: tokens
            .get(2)
            .map(|token| (*token).to_string())
            .unwrap_or_default(),
        suffix: tokens
            .get(3)
            .map(|token| (*token).to_string())
            .unwrap_or_default(),
    })
}

/// Parse `TCIRCLE <shape> [OFFSET <factor>] [VARIABLE|CONSTANT [BOTH|WIDTH|HEIGHT]]`.
///
/// The shape is `CIRCLE`, `SLOT` or `RECTANGLE`. The offset factor defaults to
/// **0.35** (AutoCAD's initial value) and must be finite and non-negative. The
/// size mode defaults to `Variable`; `CONSTANT` alone means `BOTH`. Any other
/// shape, a repeated option or an unknown token is `None`.
pub fn parse_tcircle_argument(argument: &str) -> Option<TCircleCommand> {
    let mut tokens = argument.split_whitespace();
    let shape = match tokens.next()?.to_ascii_uppercase().as_str() {
        "CIRCLE" => TextBoundaryShape::Circle,
        "SLOT" => TextBoundaryShape::Slot,
        "RECTANGLE" => TextBoundaryShape::Rectangle,
        _ => return None,
    };
    let mut offset_factor = 0.35f64;
    let mut offset_seen = false;
    let mut size_mode = TCircleSizeMode::Variable;
    let mut size_seen = false;
    while let Some(token) = tokens.next() {
        match token.to_ascii_uppercase().as_str() {
            "OFFSET" if !offset_seen => {
                let value = super::commands::parse_signed_decimal(tokens.next()?)?;
                if !value.is_finite() || value < 0.0 {
                    return None;
                }
                offset_factor = value;
                offset_seen = true;
            }
            "VARIABLE" if !size_seen => {
                size_mode = TCircleSizeMode::Variable;
                size_seen = true;
            }
            "CONSTANT" if !size_seen => {
                let mode = match tokens.next() {
                    None => TCircleSizeMode::ConstantBoth,
                    Some(word) => match word.to_ascii_uppercase().as_str() {
                        "BOTH" => TCircleSizeMode::ConstantBoth,
                        "WIDTH" => TCircleSizeMode::ConstantWidth,
                        "HEIGHT" => TCircleSizeMode::ConstantHeight,
                        _ => return None,
                    },
                };
                size_mode = mode;
                size_seen = true;
            }
            _ => return None,
        }
    }
    Some(TCircleCommand {
        shape,
        offset_factor,
        size_mode,
    })
}

/// Stable refusal for a `JUSTIFYTEXT` argument outside the grammar.
pub fn invalid_justifytext_argument_reason(text: &str) -> String {
    format!(
        "unsupported CAD command: {} (JUSTIFYTEXT needs <H> <V> with H = LEFT, CENTER or RIGHT \
         and V = BASELINE, BOTTOM, MIDDLE or TOP, e.g. `JUSTIFYTEXT LEFT TOP`)",
        text.trim()
    )
}

/// Stable refusal for a `TSCALE` argument outside the grammar.
pub fn invalid_tscale_argument_reason(text: &str) -> String {
    format!(
        "unsupported CAD command: {} (TSCALE needs one positive decimal factor, e.g. `TSCALE 2`)",
        text.trim()
    )
}

/// Stable refusal for a `TEXTALIGN` argument outside the grammar.
pub fn invalid_textalign_argument_reason(text: &str) -> String {
    format!(
        "unsupported CAD command: {} (TEXTALIGN needs LEFT|RIGHT|CENTER|TOP|MIDDLE|BOTTOM|\
         TOPLEFT|TOPCENTER|TOPRIGHT|MIDDLELEFT|MIDDLECENTER|MIDDLERIGHT|BOTTOMLEFT|\
         BOTTOMCENTER|BOTTOMRIGHT, or DISTRIBUTE [X|Y])",
        text.trim()
    )
}

/// Stable refusal for a `TCOUNT` argument outside the grammar.
pub fn invalid_tcount_argument_reason(text: &str) -> String {
    format!(
        "unsupported CAD command: {} (TCOUNT needs <start> <step> [prefix] [suffix], e.g. \
         `TCOUNT 1 1 A -`)",
        text.trim()
    )
}

/// Stable refusal for a `TCIRCLE` argument outside the grammar.
pub fn invalid_tcircle_argument_reason(text: &str) -> String {
    format!(
        "unsupported CAD command: {} (TCIRCLE needs CIRCLE|SLOT|RECTANGLE, optionally \
         `OFFSET <factor>` and `VARIABLE|CONSTANT [BOTH|WIDTH|HEIGHT]`)",
        text.trim()
    )
}

impl CadSession {
    /// The `argument_command` arms for the batch's five typed selectors.
    pub(super) fn text_batch_argument(
        &self,
        selector: &'static str,
        argument: &str,
        text: &str,
    ) -> Result<(), String> {
        match selector {
            JUSTIFYTEXT_SELECTOR => self.justifytext_argument(argument, text),
            TSCALE_SELECTOR => self.tscale_argument(argument, text),
            TEXTALIGN_SELECTOR => self.textalign_argument(argument, text),
            TCOUNT_SELECTOR => self.tcount_argument(argument, text),
            _ => {
                debug_assert_eq!(selector, TCIRCLE_SELECTOR);
                self.tcircle_argument(argument, text)
            }
        }
    }

    fn justifytext_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        let Some((h_align, v_align)) = super::parse_justifytext_argument(argument) else {
            return self.refuse_argument(super::invalid_justifytext_argument_reason(text));
        };
        self.text_batch_dispatch(
            JUSTIFYTEXT_SELECTOR,
            CommandId::JustifyText,
            CommandPayload::JustifyText(JustifyTextCommand { h_align, v_align }),
        )
    }

    fn tscale_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        let Some(factor) = super::parse_tscale_argument(argument) else {
            return self.refuse_argument(super::invalid_tscale_argument_reason(text));
        };
        self.text_batch_dispatch(
            TSCALE_SELECTOR,
            CommandId::TextScale,
            CommandPayload::TextScale(TextScaleCommand { factor }),
        )
    }

    fn textalign_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        if let Some(edge) = super::parse_textalign_argument(argument) {
            return self.text_batch_dispatch(
                TEXTALIGN_SELECTOR,
                CommandId::TextAlignEdges,
                CommandPayload::TextAlignEdges(TextAlignCommand { edge }),
            );
        }
        if let Some(spacing) = super::parse_textalign_spacing_argument(argument) {
            return self.text_batch_dispatch(
                TEXTALIGN_SELECTOR,
                CommandId::TextAlignEdges,
                CommandPayload::TextAlignSpacing(TextAlignSpacingCommand { spacing }),
            );
        }
        self.refuse_argument(super::invalid_textalign_argument_reason(text))
    }

    fn tcount_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        let Some(request) = super::parse_tcount_argument(argument) else {
            return self.refuse_argument(super::invalid_tcount_argument_reason(text));
        };
        self.text_batch_dispatch(
            TCOUNT_SELECTOR,
            CommandId::TextCount,
            CommandPayload::TextCount(request),
        )
    }

    fn tcircle_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        let Some(request) = super::parse_tcircle_argument(argument) else {
            return self.refuse_argument(super::invalid_tcircle_argument_reason(text));
        };
        self.text_batch_dispatch(
            TCIRCLE_SELECTOR,
            CommandId::TextCircle,
            CommandPayload::TextCircle(request),
        )
    }

    /// Echo the admitted selector (never the typed value) and dispatch.
    fn text_batch_dispatch(
        &self,
        selector: &'static str,
        id: CommandId,
        payload: CommandPayload,
    ) -> Result<(), String> {
        self.core.record_command(selector);
        match self.core.command(id, payload) {
            Ok(()) => {
                *self.error.borrow_mut() = None;
                Ok(())
            }
            Err(error) => {
                let text = cad_error_text(&error);
                *self.error.borrow_mut() = Some(text.clone());
                Err(text)
            }
        }
    }
}
