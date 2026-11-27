//! The `SOFTENEDGES [<angle>]` typed grammar, split out of `arguments.rs` for
//! the code-size gate.
//!
//! A bare `SOFTENEDGES` is admitted as the selector with no argument and takes
//! the command's default angle; a typed line accepts one plain decimal in
//! `0..=180` degrees. Nothing else is admitted, and the command re-validates
//! the value so a direct payload caller gets the same refusal.

/// The SketchUp `SOFTENEDGES` selector.
pub const SOFTENEDGES_SELECTOR: &str = "SOFTENEDGES";

/// Parse the optional angle of `SOFTENEDGES <angle>`.
///
/// `None` for an empty argument (the bare selector's default) and for anything
/// outside one finite decimal in `0..=180`.
pub fn parse_softenedges_argument(argument: &str) -> Option<Option<f64>> {
    let text = argument.trim();
    if text.is_empty() {
        return Some(None);
    }
    let angle: f64 = text.parse().ok()?;
    (angle.is_finite() && (0.0..=180.0).contains(&angle)).then_some(Some(angle))
}

/// The stable rejection reason for a `SOFTENEDGES` line outside the grammar.
pub fn invalid_softenedges_argument_reason(text: &str) -> String {
    format!(
        "unsupported CAD command: {} (SOFTENEDGES takes at most one angle in 0..=180 degrees)",
        text.trim()
    )
}
