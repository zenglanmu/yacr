//! The `FLIP` typed grammar: one axis, nothing else.
//!
//! `FLIP X|Y|Z` (also `RED|GREEN|BLUE`) mirrors the selection in place about the
//! plane through its bounds centre perpendicular to that axis. A bare `FLIP` is
//! refused because SketchUp's Flip Along has no ambiguous default.

/// The `FLIP` selector.
pub const FLIP_SELECTOR: &str = "FLIP";

/// One axis a `FLIP` line accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlipArgument {
    X,
    Y,
    Z,
}

/// Parse the one axis token.
pub fn parse_flip_argument(argument: &str) -> Option<FlipArgument> {
    match argument.trim().to_ascii_uppercase().as_str() {
        "X" | "RED" => Some(FlipArgument::X),
        "Y" | "GREEN" => Some(FlipArgument::Y),
        "Z" | "BLUE" => Some(FlipArgument::Z),
        _ => None,
    }
}

/// The stable rejection reason for a `FLIP` line outside the grammar.
pub fn invalid_flip_argument_reason(text: &str) -> String {
    format!(
        "flip.axis_required: {} (FLIP takes X, Y or Z / RED, GREEN or BLUE)",
        text.trim()
    )
}
