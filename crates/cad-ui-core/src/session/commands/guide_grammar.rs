//! The `GUIDE [LIST|CLEAR]` typed grammar.
//!
//! Bare `GUIDE` reports the table; `GUIDE LIST` is the same report and
//! `GUIDE CLEAR` removes every guide in one write. Nothing else is admitted.

/// The `GUIDE` selector.
pub const GUIDE_SELECTOR: &str = "GUIDE";

/// One admitted `GUIDE` argument.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuideArgument {
    List,
    Clear,
}

/// Parse the argument of a `GUIDE <option>` line.
pub fn parse_guide_argument(argument: &str) -> Option<GuideArgument> {
    match argument.trim().to_ascii_uppercase().as_str() {
        "LIST" => Some(GuideArgument::List),
        "CLEAR" => Some(GuideArgument::Clear),
        _ => None,
    }
}

/// The stable rejection reason for a `GUIDE` line outside the grammar.
pub fn invalid_guide_argument_reason(text: &str) -> String {
    format!(
        "unsupported CAD command: {} (GUIDE takes LIST or CLEAR; bare GUIDE lists)",
        text.trim()
    )
}
