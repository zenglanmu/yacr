//! `PROTRACTOR GUIDE`: start the angle capture with guide mode.
//!
//! The bare `PROTRACTOR` is an interaction-only measurement selector (no
//! `CommandId`) and starts `BeginMeasurement(Protractor)` in the session's typed
//! funnel. `PROTRACTOR GUIDE` starts the same capture and writes a guide line
//! from the vertex along the second ray when the three points complete.

use cad_app::{GuideMode, MeasurementToolKind};

use super::commands::{invalid_protractor_argument_reason, parse_protractor_argument};

impl super::CadSession {
    /// Apply one typed `PROTRACTOR <argument>` line.
    pub(super) fn protractor_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        let Some(()) = parse_protractor_argument(argument) else {
            return self.refuse_argument(invalid_protractor_argument_reason(text));
        };
        self.start_guide_measurement(GuideMode::ProtractorLine, MeasurementToolKind::Protractor)
    }
}
