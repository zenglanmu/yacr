//! The Wave 17-24 data-driven named refusals.
//!
//! Every name in [`cad_app::RefusalId`] is a feature with no model or API in
//! this build. The registry lives in `command_id/extended/refusal_names.rs`;
//! this module is the single handler, so adding a name never adds a dispatch
//! arm or a hand-written reason.
//!
//! A refusal writes nothing, bumps no revision and records no `cad-history`
//! entry, so every one of these names is Viewer-legal. A caller payload is
//! refused first with `require_none_payload`, so a malformed request never
//! reaches the feature-specific reason.

use super::*;
use crate::app_utility::require_none_payload;

impl Application {
    /// Refuse one Wave 17-24 command with its stable reason string.
    pub(crate) fn extended_refusal_named(
        &mut self,
        command: &Command,
        name: RefusalId,
    ) -> CadResult<CommandOutcome> {
        require_none_payload(command, name.selector())?;
        Err(CadError::Unsupported(name.reason().to_string()))
    }
}
