//! Panel actions whose model is **not implemented** (Match Photo, Overlays).
//!
//! The panels themselves are host-read-backed (`cad_match_photo` /
//! `cad_overlays`); these commands are the stable refusal a panel action
//! dispatches when there is nothing to write. They write nothing, bump no
//! revision and record no history — the same shape as every other named
//! refusal.

use super::*;
use crate::app_utility::require_none_payload;

/// The Match Photo refusal every entry point shares (also on the host read).
pub const MATCH_PHOTO_NOT_MODELED: &str =
    "matchphoto.not_implemented: this build has no photo-matching model (no photo import, camera \
     solve or photo background)";

/// The Overlays management refusal.
pub const OVERLAYS_NOT_MODELED: &str =
    "overlays.not_implemented: this build has no user-managed overlay definition model; the listed \
     channels are renderer-owned and read-only";

impl Application {
    /// `MATCHPHOTO`: named refusal (photo matching is not modeled).
    pub(crate) fn match_photo(
        &mut self,
        _session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        require_none_payload(command, "MATCHPHOTO")?;
        Err(CadError::Unsupported(MATCH_PHOTO_NOT_MODELED.into()))
    }

    /// `OVERLAYS`: named refusal (no user-managed overlay model).
    pub(crate) fn overlays(
        &mut self,
        _session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        require_none_payload(command, "OVERLAYS")?;
        Err(CadError::Unsupported(OVERLAYS_NOT_MODELED.into()))
    }
}
