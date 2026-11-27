//! The `INSERT <name>` / `BLOCK <name>` typed grammars, split out of
//! `arguments.rs` for the code-size gate.
//!
//! A bare `INSERT`/`BLOCK` is a point-then-name capture. The typed form only
//! carries the **name**: the session starts the same capture with that name
//! pending, so one click places/defines it and the name does not have to be
//! typed again. Nothing else is admitted; the command layer re-validates the
//! name (single line, non-empty, length limit, duplicate rules).

/// The `INSERT <name>` selector.
pub const INSERT_SELECTOR: &str = "INSERT";

/// The `BLOCK <name>` selector.
pub const BLOCK_SELECTOR: &str = "BLOCK";

/// Parse the block name of `INSERT <name>` / `BLOCK <name>`.
///
/// A trimmed, non-empty, single-line name; the caller passes it to the same
/// capture the bare selector starts, and the create command owns duplicates and
/// the length policy.
pub fn parse_block_name_argument(argument: &str) -> Option<String> {
    let name = argument.trim();
    if name.is_empty() || name.contains('\r') || name.contains('\n') {
        return None;
    }
    Some(name.to_string())
}

/// The stable rejection reason for an `INSERT`/`BLOCK` line outside the grammar.
pub fn invalid_block_name_argument_reason(selector: &str, text: &str) -> String {
    format!(
        "unsupported CAD command: {} ({selector} takes at most one trimmed block name, or no \
         argument for the point-then-name capture)",
        text.trim()
    )
}
