//! The `EDGECOLOR [DEFAULT|<r> <g> <b>]` typed grammar.
//!
//! The colour is session state only; the host clamps/validates bytes and reports
//! the current value for a bare selector.

/// The `EDGECOLOR` selector.
pub const EDGECOLOR_SELECTOR: &str = "EDGECOLOR";

/// One admitted `EDGECOLOR` argument.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeColorArgument {
    Set([u8; 3]),
    Default,
    Report,
}

/// Parse the argument of an `EDGECOLOR <option>` line.
pub fn parse_edge_color_argument(argument: &str) -> Option<EdgeColorArgument> {
    let tokens: Vec<&str> = argument.split_whitespace().collect();
    match tokens.as_slice() {
        [] => Some(EdgeColorArgument::Report),
        [word] if word.eq_ignore_ascii_case("DEFAULT") => Some(EdgeColorArgument::Default),
        [r, g, b] => Some(EdgeColorArgument::Set([
            r.parse::<u8>().ok()?,
            g.parse::<u8>().ok()?,
            b.parse::<u8>().ok()?,
        ])),
        _ => None,
    }
}

/// The stable rejection reason for an `EDGECOLOR` line outside the grammar.
pub fn invalid_edge_color_argument_reason(text: &str) -> String {
    format!(
        "unsupported CAD command: {} (EDGECOLOR takes DEFAULT or <r> <g> <b>)",
        text.trim()
    )
}
