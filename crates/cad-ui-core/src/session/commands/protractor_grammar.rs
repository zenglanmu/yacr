//! The `PROTRACTOR` typed grammar, split out of `arguments.rs` for the
//! code-size gate.
//!
//! The bare selector starts the three-point Protractor measurement (vertex, then
//! one point on each ray). The only admitted argument is `GUIDE`, and it is
//! admitted **to be refused by name**: guide-line creation is not modeled, so
//! `PROTRACTOR GUIDE` answers `protractor.guide_not_implemented` instead of
//! silently measuring without the guide the user asked for.

/// The SketchUp `PROTRACTOR` selector.
pub const PROTRACTOR_SELECTOR: &str = "PROTRACTOR";

/// Parse the optional `GUIDE` argument of `PROTRACTOR`.
///
/// `Some(())` only for the single word `GUIDE` (case-insensitive); `None` for
/// any other text, which is the invalid-argument refusal. The caller turns the
/// admitted `GUIDE` into the named not-implemented refusal.
pub fn parse_protractor_argument(argument: &str) -> Option<()> {
    argument.trim().eq_ignore_ascii_case("GUIDE").then_some(())
}

/// The stable rejection reason for a `PROTRACTOR` line outside the grammar.
pub fn invalid_protractor_argument_reason(text: &str) -> String {
    format!(
        "unsupported CAD command: {} (PROTRACTOR takes no option; PROTRACTOR GUIDE is refused by \
         name because guide-line creation is not modeled)",
        text.trim()
    )
}
