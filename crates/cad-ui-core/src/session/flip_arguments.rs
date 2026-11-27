//! `FLIP X|Y|Z`: dispatch the in-place axis mirror.
//!
//! The app owns the reflection (selection bounds centre, one transaction and one
//! undo step); this module only validates the axis token and routes it.

use cad_app::{CommandId, CommandPayload, FlipAxis};

use super::commands::{
    invalid_flip_argument_reason, parse_flip_argument, FlipArgument, FLIP_SELECTOR,
};

impl super::CadSession {
    /// Apply one typed `FLIP <axis>` line.
    pub(super) fn flip_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        let Some(axis) = parse_flip_argument(argument) else {
            return self.refuse_argument(invalid_flip_argument_reason(text));
        };
        let axis = match axis {
            FlipArgument::X => FlipAxis::Red,
            FlipArgument::Y => FlipAxis::Green,
            FlipArgument::Z => FlipAxis::Blue,
        };
        self.dispatch_typed_argument(FLIP_SELECTOR, CommandId::Flip, CommandPayload::Flip(axis))
    }
}
