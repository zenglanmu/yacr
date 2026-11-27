//! `DIST GUIDE`: start the Distance capture with guide mode.

use cad_app::interaction::InteractionRequest;
use cad_app::{AppMode, GuideMode, MeasurementToolKind};

use super::commands::{invalid_dist_argument_reason, parse_dist_guide_argument};

impl super::CadSession {
    /// Apply one typed `DIST <argument>` line.
    pub(super) fn dist_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        if !parse_dist_guide_argument(argument) {
            return self.refuse_argument(invalid_dist_argument_reason(text));
        }
        self.start_guide_measurement(GuideMode::DistanceLine, MeasurementToolKind::Distance)
    }

    /// Start a guide-mode measurement after checking the carrier can write.
    pub(super) fn start_guide_measurement(
        &self,
        mode: GuideMode,
        kind: MeasurementToolKind,
    ) -> Result<(), String> {
        {
            let controller = self.core.controller.borrow();
            if controller.session.app_mode() == AppMode::Viewer {
                return Err(
                    "guide.requires_edit: a construction guide is a document write; switch out of \
                     Viewer mode first"
                        .into(),
                );
            }
        }
        self.submit_interactions(vec![InteractionRequest::BeginMeasurement(kind)])?;
        self.core
            .controller
            .borrow_mut()
            .session
            .set_guide_mode(mode);
        Ok(())
    }
}
