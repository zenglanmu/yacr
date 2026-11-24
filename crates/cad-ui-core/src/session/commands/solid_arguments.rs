//! The typed `PYRAMID` command-line form: the regular n-gon base.
//!
//! `PYRAMID`'s payload already carries the explicit three-point square-base
//! form ([`cad_app::SolidPrimitiveCommand::Points`]), and the six other 3D
//! primitives have no typed line at all (their constructions are payload-only,
//! see `cad_app::SolidPrimitiveCommand`). This module adds the **only one**
//! typed solid line that has an argument grammar worth typing — AutoCAD's
//! `PYRAMID <sides> <centre> <radius> <height>` construction, whose base ring is
//! the `POLYGON` command's own:
//!
//! ```text
//! PYRAMID [INSCRIBED|I|CIRCUMSCRIBED|C] <sides> <cx> <cy> <radius> <height>
//! ```
//!
//! * the optional word is the same inscribed/circumscribed choice `POLYGON` and
//!   `POLYGON CIRCUMSCRIBED` offer; absent = inscribed (POLYGON's default);
//! * `<sides>` is an ASCII integer (the command layer owns the `3..=1024` bound
//!   and refuses an out-of-range count with `pyramid.sides`);
//! * `<cx> <cy>` is the base centre on the plan (its z is the session `ELEV`),
//!   `<radius>` the base radius and `<height>` the pyramid height.
//!
//! The parser only decides whether the line is inside that grammar; the
//! application validates the values and owns every refusal, so a malformed line
//! is `pyramid.argument` and an out-of-range one is the named
//! `pyramid.sides` / `pyramid.radius` / `pyramid.degenerate_extent`. A bare
//! `PYRAMID` never reaches here and keeps its "needs points" refusal.

use cad_app::{CommandId, CommandPayload, ExtendedCommandId, SolidPrimitiveCommand};

use crate::session::CadSession;

/// `PYRAMID [INSCRIBED|CIRCUMSCRIBED] <sides> <cx> <cy> <radius> <height>`.
pub const PYRAMID_SELECTOR: &str = "PYRAMID";

/// Parse the typed polygon-base `PYRAMID` line.
///
/// Returns `None` for anything outside the grammar: a missing or extra token, a
/// non-integer side count (a fractional/decimal count is refused rather than
/// rounded), and any non-decimal coordinate, radius or height. The
/// inscribed/circumscribed word may be `INSCRIBED`/`I` or `CIRCUMSCRIBED`/`C`
/// (case-insensitive); every other leading word is outside the grammar.
pub fn parse_pyramid_argument(argument: &str) -> Option<SolidPrimitiveCommand> {
    let mut tokens = argument.split_whitespace();
    let first = tokens.next()?;
    let (circumscribed, sides_token) =
        if first.eq_ignore_ascii_case("INSCRIBED") || first.eq_ignore_ascii_case("I") {
            (false, tokens.next()?)
        } else if first.eq_ignore_ascii_case("CIRCUMSCRIBED") || first.eq_ignore_ascii_case("C") {
            (true, tokens.next()?)
        } else {
            (false, first)
        };
    if sides_token.is_empty() || !sides_token.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let sides = sides_token.parse::<u32>().ok()?;
    let mut values = [0.0_f64; 4];
    for slot in &mut values {
        *slot = super::arguments::parse_signed_decimal(tokens.next()?)?;
    }
    if tokens.next().is_some() {
        return None;
    }
    Some(SolidPrimitiveCommand::PyramidPolygon {
        base: [values[0], values[1]],
        radius: values[2],
        sides,
        circumscribed,
        height: values[3],
    })
}

/// The stable rejection reason for a `PYRAMID` argument outside the grammar.
pub fn invalid_pyramid_argument_reason(text: &str) -> String {
    format!(
        "unsupported CAD command: {} (PYRAMID takes `[INSCRIBED|CIRCUMSCRIBED] <sides> <cx> <cy> \
         <radius> <height>` or a `Solid`/`Points` payload; nothing was written)",
        text.trim()
    )
}

impl CadSession {
    /// Apply one typed `PYRAMID …` line.
    ///
    /// It dispatches the ordinary [`ExtendedCommandId::Pyramid`] with the
    /// [`SolidPrimitiveCommand::PyramidPolygon`] payload, so the application's
    /// existing create funnel (one exact body, one transaction, one undo step)
    /// is the only write path. A malformed line is refused before IPC.
    pub(crate) fn pyramid_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        let Some(payload) = parse_pyramid_argument(argument) else {
            return self.refuse_argument(invalid_pyramid_argument_reason(text));
        };
        self.dispatch_typed_argument(
            PYRAMID_SELECTOR,
            CommandId::Extended(ExtendedCommandId::Pyramid),
            CommandPayload::Solid(payload),
        )
    }
}
