//! The `PAINT <material>` typed grammar, split out of `arguments.rs` for the
//! code-size gate.
//!
//! The name is not resolved here: the command owns the document-first /
//! built-in-palette lookup, so a typed line and a direct payload caller see the
//! same refusals.

/// Parse `PAINT <material>`: the rest of the line, trimmed and non-empty.
pub fn parse_paint_argument(argument: &str) -> Option<String> {
    let name = argument.trim();
    (!name.is_empty()).then(|| name.to_string())
}

/// The stable rejection reason for a `PAINT` line with no material name.
pub fn invalid_paint_argument_reason(text: &str) -> String {
    format!(
        "unsupported CAD command: {} (PAINT needs one material name)",
        text.trim()
    )
}
