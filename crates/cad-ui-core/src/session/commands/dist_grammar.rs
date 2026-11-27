//! The `DIST GUIDE` grammar.
//!
//! The bare `DIST` selector starts the ordinary Distance measurement. The only
//! argument form is `GUIDE`, which starts the same capture with a guide mode; the
//! app writes the guide when the two points complete. Anything else is refused.

/// The `DIST` selector.
pub const DIST_SELECTOR: &str = "DIST";

/// Parse the one admitted `DIST` argument.
pub fn parse_dist_guide_argument(argument: &str) -> bool {
    argument.trim().eq_ignore_ascii_case("GUIDE")
}

/// The stable rejection reason for a `DIST` line outside the grammar.
pub fn invalid_dist_argument_reason(text: &str) -> String {
    format!(
        "unsupported CAD command: {} (DIST takes no argument to measure, or GUIDE to also create \
         a construction guide)",
        text.trim()
    )
}
