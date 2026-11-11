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

    /// The configured policy with the session `APERTURE` token applied.
    ///
    /// This is what the live cad-app snap/pick paths read: the session token is
    /// authoritative once `APERTURE <n>` set it, otherwise the configured
    /// `view.aperture.sizePixels` value is used unchanged
    /// ([`crate::SessionState::interaction_tolerance`]). The pickbox is
    /// deliberately untouched.
    pub(crate) fn session_tolerance(&self) -> TolerancePolicy {
        self.session.interaction_tolerance(self.tolerance)
    }
}
