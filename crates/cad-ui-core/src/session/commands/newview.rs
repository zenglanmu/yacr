//! `NEWVIEW <name>` / `NEWVIEW WINDOW <name> <x1> <y1> <x2> <y2>` grammar.
//!
//! Split out of `arguments.rs` for the file-size gate. The parser is pure; the
//! command layer resolves the viewport, validates the rectangle and writes the
//! persisted named-view table.

use super::arguments::parse_signed_decimal;

/// `NEWVIEW <name>` / `NEWVIEW WINDOW <name> <x1> <y1> <x2> <y2>`: save one
/// named view from the current display or from a rectangular window.
pub const NEWVIEW_SELECTOR: &str = "NEWVIEW";

/// One parsed `NEWVIEW` argument.
#[derive(Debug, Clone, PartialEq)]
pub enum NewViewArgument {
    /// `NEWVIEW <name>`: store the current display under `name`.
    Current(String),
    /// `NEWVIEW WINDOW <name> <x1> <y1> <x2> <y2>`: store a view framed by the
    /// world-space rectangle.
    Window { name: String, rect: [f64; 4] },
}

/// Parse `NEWVIEW <name>` or `NEWVIEW WINDOW <name> <x1> <y1> <x2> <y2>`.
///
/// Both forms accept a name with spaces: the current-view form takes the whole
/// trimmed line, and the window form takes everything between the `WINDOW`
/// keyword and the final four numeric tokens. A window form missing a name or
/// carrying a non-numeric corner is `None`; the command layer owns extent and
/// 2D validation by name.
pub fn parse_newview_argument(argument: &str) -> Option<NewViewArgument> {
    let text = argument.trim();
    if text.is_empty() {
        return None;
    }
    let parts: Vec<&str> = text.split_whitespace().collect();
    if !parts
        .first()
        .is_some_and(|word| word.eq_ignore_ascii_case("WINDOW"))
    {
        return Some(NewViewArgument::Current(text.to_string()));
    }
    if parts.len() < 6 {
        return None;
    }
    let numbers: Vec<f64> = parts[parts.len() - 4..]
        .iter()
        .map(|token| parse_signed_decimal(token))
        .collect::<Option<Vec<_>>>()?;
    let name = parts[1..parts.len() - 4].join(" ");
    if name.is_empty() {
        return None;
    }
    Some(NewViewArgument::Window {
        name,
        rect: [numbers[0], numbers[1], numbers[2], numbers[3]],
    })
}

/// The stable rejection reason for a `NEWVIEW` argument outside the grammar.
pub fn invalid_newview_argument_reason(text: &str) -> String {
    format!(
        "unsupported CAD command: {} (NEWVIEW needs <name> or WINDOW <name> <x1> <y1> <x2> \
         <y2>)",
        text.trim()
    )
}
