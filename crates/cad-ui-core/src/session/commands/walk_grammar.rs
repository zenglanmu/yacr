//! The `WALK` / `LOOKAROUND` typed grammars, split out of `arguments.rs` for the
//! code-size gate.
//!
//! The bare selectors start one-point navigation captures. Their typed forms are
//! numbers only: `WALK <distance>` (signed model units along the horizontal view
//! direction) and `LOOKAROUND <yaw> <pitch>` (degrees). Nothing else is admitted;
//! the command layer re-validates every value and refuses a plan/orthographic
//! view.

use cad_app::{LookAroundCommand, WalkCommand};

/// The `WALK` selector.
pub const WALK_SELECTOR: &str = "WALK";

/// The `LOOKAROUND` selector.
pub const LOOKAROUND_SELECTOR: &str = "LOOKAROUND";

/// Parse the argument of `WALK <distance>`.
pub fn parse_walk_argument(argument: &str) -> Option<WalkCommand> {
    let text = argument.trim();
    let distance: f64 = text.parse().ok()?;
    distance.is_finite().then_some(WalkCommand::By(distance))
}

/// Parse the argument of `LOOKAROUND <yaw> <pitch>`.
pub fn parse_lookaround_argument(argument: &str) -> Option<LookAroundCommand> {
    let mut words = argument.split_whitespace();
    let yaw: f64 = words.next()?.parse().ok()?;
    let pitch: f64 = words.next()?.parse().ok()?;
    if words.next().is_some() || !yaw.is_finite() || !pitch.is_finite() {
        return None;
    }
    Some(LookAroundCommand::By {
        yaw_degrees: yaw,
        pitch_degrees: pitch,
    })
}

/// The stable rejection reason for a `WALK` line outside the grammar.
pub fn invalid_walk_argument_reason(text: &str) -> String {
    format!(
        "unsupported CAD command: {} (WALK takes one finite distance, or no argument for the \
         destination capture)",
        text.trim()
    )
}

/// The stable rejection reason for a `LOOKAROUND` line outside the grammar.
pub fn invalid_lookaround_argument_reason(text: &str) -> String {
    format!(
        "unsupported CAD command: {} (LOOKAROUND takes <yaw> <pitch> in degrees, or no argument \
         for the look-at capture)",
        text.trim()
    )
}
