//! The typed `HATCHEDIT <option>` line (AutoCAD 2026).
//!
//! `HATCHEDIT` is an argument form of its base selector, not a command of its
//! own: there is no second [`cad_app::CommandId`] and no second IPC. The current
//! selection is still the input (one or more directly addressable model-space
//! hatches), so this module only parses the typed option and dispatches
//! [`cad_app::CommandPayload::HatchEdit`].
//!
//! Grammar (case-insensitive):
//!
//! * `ANGLE <degrees>` — one finite signed decimal, in degrees;
//! * `SCALE <factor>` — one finite decimal strictly greater than 0;
//! * `PATTERNTYPE USER|PREDEFINED|CUSTOM` — DXF group 76;
//! * `STYLE IGNORE|OUTER|NORMAL` — the island-detection style (group 75);
//! * `SOLID`, `GRADIENT` — the fill variant;
//! * `COLOR <aci>` or `COLOR BYLAYER`;
//! * `TRANSPARENCY <percent>` (`0..=90`) or `TRANSPARENCY BYLAYER`;
//! * `LAYER <name>` — the drawing layer name (spaces allowed);
//! * `DRAWORDER BEHIND|FRONT|BOUNDARY`;
//! * `RECREATEBOUNDARY`, `SEPARATE`;
//! * `ADDBOUNDARIES`, `REMOVEBOUNDARIES` — the pre-selection boundary edit:
//!   exactly one hatch plus the other selected closed entities, rewritten as one
//!   transaction / one undo step;
//! * the named-refusal forms `ORIGIN`, `ASSOCIATE`, `DISASSOCIATE`,
//!   `ANNOTATIVE` (they parse so the application can name the missing model
//!   instead of pretending the option word is invalid).
//!
//! A bare `HATCHEDIT` never reaches here (it keeps the name-only funnel and the
//! application refuses it with `hatchedit.needs_option`). An argument outside
//! the grammar is refused with the stable
//! [`super::invalid_hatchedit_argument_reason`], recorded for `cad_status.error`,
//! and changes nothing.

use cad_app::{CommandId, CommandPayload, HatchDrawOrder, HatchEditOption};
use cad_domain::{HatchPatternType, HatchStyle};

use super::commands::parse_signed_decimal;
use super::diagnostics::cad_error_text;
use super::{CadSession, HATCHEDIT_SELECTOR};

/// Parse the argument of a `HATCHEDIT` selector: one declared option.
///
/// No partial spelling, option stack or AutoCAD dialog field is admitted; the
/// caller turns `None` into the stable refusal from
/// [`invalid_hatchedit_argument_reason`]. The bare selector never reaches this
/// parser: it keeps the name-only funnel and the application refuses it with
/// `hatchedit.needs_option`.
pub fn parse_hatchedit_argument(argument: &str) -> Option<HatchEditOption> {
    let trimmed = argument.trim();
    let (word, rest) = match trimmed.find([' ', '\t']) {
        Some(index) => (&trimmed[..index], trimmed[index..].trim()),
        None => (trimmed, ""),
    };
    if word.eq_ignore_ascii_case("SOLID") {
        return rest.is_empty().then_some(HatchEditOption::Solid);
    }
    if word.eq_ignore_ascii_case("GRADIENT") {
        return rest.is_empty().then_some(HatchEditOption::Gradient);
    }
    if word.eq_ignore_ascii_case("RECREATEBOUNDARY") {
        return rest.is_empty().then_some(HatchEditOption::RecreateBoundary);
    }
    if word.eq_ignore_ascii_case("SEPARATE") {
        return rest.is_empty().then_some(HatchEditOption::SeparateHatches);
    }
    if word.eq_ignore_ascii_case("ORIGIN") {
        return rest.is_empty().then_some(HatchEditOption::Origin);
    }
    if word.eq_ignore_ascii_case("ASSOCIATE") {
        return rest.is_empty().then_some(HatchEditOption::Associate);
    }
    if word.eq_ignore_ascii_case("DISASSOCIATE") {
        return rest.is_empty().then_some(HatchEditOption::Disassociate);
    }
    if word.eq_ignore_ascii_case("ANNOTATIVE") {
        return rest.is_empty().then_some(HatchEditOption::Annotative);
    }
    if word.eq_ignore_ascii_case("ADDBOUNDARIES") {
        return rest.is_empty().then_some(HatchEditOption::AddBoundaries);
    }
    if word.eq_ignore_ascii_case("REMOVEBOUNDARIES") {
        return rest.is_empty().then_some(HatchEditOption::RemoveBoundaries);
    }
    if word.eq_ignore_ascii_case("ANGLE") {
        // `parse_signed_decimal` already refuses an empty value, exponents, a
        // bare sign, embedded spaces and every non-finite magnitude.
        return parse_signed_decimal(rest).map(HatchEditOption::Angle);
    }
    if word.eq_ignore_ascii_case("SCALE") {
        return parse_signed_decimal(rest)
            .filter(|factor| *factor > 0.0)
            .map(HatchEditOption::Scale);
    }
    if word.eq_ignore_ascii_case("PATTERNTYPE") {
        return parse_pattern_type(rest).map(HatchEditOption::PatternType);
    }
    if word.eq_ignore_ascii_case("STYLE") {
        return parse_style(rest).map(HatchEditOption::Style);
    }
    if word.eq_ignore_ascii_case("DRAWORDER") {
        return parse_draw_order(rest).map(HatchEditOption::DrawOrder);
    }
    if word.eq_ignore_ascii_case("COLOR") {
        if rest.eq_ignore_ascii_case("BYLAYER") {
            return Some(HatchEditOption::ColorByLayer);
        }
        return parse_aci(rest).map(HatchEditOption::Color);
    }
    if word.eq_ignore_ascii_case("TRANSPARENCY") {
        if rest.eq_ignore_ascii_case("BYLAYER") {
            return Some(HatchEditOption::TransparencyByLayer);
        }
        return rest
            .parse::<u8>()
            .ok()
            .filter(|percent| *percent <= 90)
            .map(HatchEditOption::Transparency);
    }
    if word.eq_ignore_ascii_case("LAYER") {
        return (!rest.is_empty()).then(|| HatchEditOption::Layer(rest.to_string()));
    }
    None
}

fn parse_pattern_type(text: &str) -> Option<HatchPatternType> {
    if text.eq_ignore_ascii_case("USER") {
        Some(HatchPatternType::UserDefined)
    } else if text.eq_ignore_ascii_case("PREDEFINED") {
        Some(HatchPatternType::Predefined)
    } else if text.eq_ignore_ascii_case("CUSTOM") {
        Some(HatchPatternType::Custom)
    } else {
        None
    }
}

fn parse_style(text: &str) -> Option<HatchStyle> {
    if text.eq_ignore_ascii_case("IGNORE") {
        Some(HatchStyle::Ignore)
    } else if text.eq_ignore_ascii_case("OUTER") {
        Some(HatchStyle::Outer)
    } else if text.eq_ignore_ascii_case("NORMAL") {
        Some(HatchStyle::Normal)
    } else {
        None
    }
}

fn parse_draw_order(text: &str) -> Option<HatchDrawOrder> {
    if text.eq_ignore_ascii_case("BEHIND") {
        Some(HatchDrawOrder::BehindAll)
    } else if text.eq_ignore_ascii_case("FRONT") {
        Some(HatchDrawOrder::InFrontOfAll)
    } else if text.eq_ignore_ascii_case("BOUNDARY") {
        Some(HatchDrawOrder::BehindBoundary)
    } else {
        None
    }
}

fn parse_aci(text: &str) -> Option<u8> {
    text.parse::<u8>()
        .ok()
        .filter(|index| (1..=255).contains(index))
}

/// The stable rejection reason for a `HATCHEDIT` argument outside the grammar.
///
/// The typed line is echoed and the implemented options are named, together with
/// the documented AutoCAD options that are **not** implemented, so nobody reads
/// the subset as a promise that the whole dialog works.
pub fn invalid_hatchedit_argument_reason(text: &str) -> String {
    format!(
        "unsupported CAD command: {} (HATCHEDIT needs STYLE, ANGLE, SCALE, PATTERNTYPE, SOLID, \
         GRADIENT, COLOR, TRANSPARENCY, LAYER, DRAWORDER, RECREATEBOUNDARY, SEPARATE, \
         ADDBOUNDARIES or REMOVEBOUNDARIES; ORIGIN/ASSOCIATE/DISASSOCIATE/ANNOTATIVE are \
         named refusals)",
        text.trim()
    )
}

impl CadSession {
    /// Apply one typed `HATCHEDIT <option>` line.
    pub(super) fn hatchedit_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        let Some(option) = parse_hatchedit_argument(argument) else {
            return self.refuse_argument(super::invalid_hatchedit_argument_reason(text));
        };
        // Echo the admitted selector (never the typed argument).
        self.core.record_command(HATCHEDIT_SELECTOR);
        match self
            .core
            .command(CommandId::HatchEdit, CommandPayload::HatchEdit(option))
        {
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
