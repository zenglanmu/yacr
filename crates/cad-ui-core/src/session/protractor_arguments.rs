//! `PROTRACTOR GUIDE`: the one argument form of the Protractor selector.
//!
//! The bare `PROTRACTOR` is an interaction-only measurement selector (no
//! `CommandId`) and starts `BeginMeasurement(Protractor)` in the session's typed
//! funnel. The `GUIDE` argument exists so the reference tool's second mode is a
//! **named refusal** rather than an unsupported line: this build has no guide
//! model, so it says so and writes nothing.

use super::commands::{invalid_protractor_argument_reason, parse_protractor_argument};

impl super::CadSession {
    /// Apply one typed `PROTRACTOR <argument>` line.
    pub(super) fn protractor_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        let Some(()) = parse_protractor_argument(argument) else {
            return self.refuse_argument(invalid_protractor_argument_reason(text));
        };
        self.refuse_argument(
            "protractor.guide_not_implemented: guide-line creation is not modeled; the protractor \
             measures the angle only"
                .into(),
        )
    }
}
