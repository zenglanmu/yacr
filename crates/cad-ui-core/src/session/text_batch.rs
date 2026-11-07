//! The typed argument forms of the 2026-11-07 2D text batch:
//! `JUSTIFYTEXT <H> <V>`, `TSCALE <factor>`, `TEXTALIGN <edge>`,
//! `TCOUNT <start> <step> [prefix] [suffix]` and `TCIRCLE <shape>`.
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
    CommandId, CommandPayload, JustifyTextCommand, TCircleCommand, TCountCommand, TextAlignCommand,
    TextAlignEdge, TextBoundaryShape, TextScaleCommand,
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

/// Parse `TEXTALIGN <edge>`: one of the six full edge words.
///
/// No abbreviation is admitted: the host grammar does not guess at a partial
/// word, so `L`/`C`/`T` are refused rather than mapped.
pub fn parse_textalign_argument(argument: &str) -> Option<TextAlignEdge> {
    let key = argument.trim();
    if key.eq_ignore_ascii_case("LEFT") {
        Some(TextAlignEdge::Left)
    } else if key.eq_ignore_ascii_case("RIGHT") {
        Some(TextAlignEdge::Right)
    } else if key.eq_ignore_ascii_case("CENTER") {
        Some(TextAlignEdge::Center)
    } else if key.eq_ignore_ascii_case("TOP") {
        Some(TextAlignEdge::Top)
    } else if key.eq_ignore_ascii_case("MIDDLE") {
        Some(TextAlignEdge::Middle)
    } else if key.eq_ignore_ascii_case("BOTTOM") {
        Some(TextAlignEdge::Bottom)
    } else {
        None
    }
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

/// Parse `TCIRCLE <shape>`: exactly `CIRCLE` or `RECTANGLE`.
pub fn parse_tcircle_argument(argument: &str) -> Option<TextBoundaryShape> {
    let key = argument.trim();
    if key.eq_ignore_ascii_case("CIRCLE") {
        Some(TextBoundaryShape::Circle)
    } else if key.eq_ignore_ascii_case("RECTANGLE") {
        Some(TextBoundaryShape::Rectangle)
    } else {
        None
    }
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
        "unsupported CAD command: {} (TEXTALIGN needs LEFT, RIGHT, CENTER, TOP, MIDDLE or BOTTOM)",
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
        "unsupported CAD command: {} (TCIRCLE needs CIRCLE or RECTANGLE)",
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
        let Some(edge) = super::parse_textalign_argument(argument) else {
            return self.refuse_argument(super::invalid_textalign_argument_reason(text));
        };
        self.text_batch_dispatch(
            TEXTALIGN_SELECTOR,
            CommandId::TextAlignEdges,
            CommandPayload::TextAlignEdges(TextAlignCommand { edge }),
        )
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
        let Some(shape) = super::parse_tcircle_argument(argument) else {
            return self.refuse_argument(super::invalid_tcircle_argument_reason(text));
        };
        self.text_batch_dispatch(
            TCIRCLE_SELECTOR,
            CommandId::TextCircle,
            CommandPayload::TextCircle(TCircleCommand { shape }),
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
