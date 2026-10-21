//! Config-derived interaction tolerance (Options ▸ Selection / Drafting).

use super::*;

impl HostController {
    /// Install the configured interaction tolerance policy (pickbox/aperture).
    ///
    /// The host derives it from its effective config via
    /// [`crate::viewer_config::ViewConfig::tolerance_policy`] so picks, hovers and
    /// snaps all read one value. Cheap, plain data, and idempotent.
    pub fn set_tolerance_policy(&mut self, policy: TolerancePolicy) {
        self.tolerance = policy;
    }

    /// The interaction tolerance policy this session resolves with.
    pub fn tolerance(&self) -> TolerancePolicy {
        self.tolerance
    }
}
