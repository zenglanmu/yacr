//! The Match Photo panel's wire row: `cad_match_photo` / `kind: "matchPhoto"`.
//!
//! Photo matching is not modeled, so the answer is a capability statement, not
//! an empty photo list: `modeled: false` plus the stable reason. The panel's
//! action dispatches `MATCHPHOTO`, which returns the same marker.

use serde::{Deserialize, Serialize};

/// The wire spelling of the Match Photo request kind.
pub const MATCH_PHOTO_KIND: &str = "matchPhoto";

/// Stable refusal for a carrier with no Match Photo lane.
pub const MATCH_PHOTO_UNSUPPORTED: &str = "unsupported-runtime: matchPhoto";

/// The Match Photo panel's answer.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MatchPhotoResponse {
    /// Whether this build models photo matching (always `false`).
    pub modeled: bool,
    /// The stable reason the panel shows.
    pub reason: String,
}

impl MatchPhotoResponse {
    /// Project the host's capability onto the wire row.
    pub fn from_host(status: cad_host::MatchPhotoStatus) -> Self {
        MatchPhotoResponse {
            modeled: status.modeled,
            reason: status.reason.to_string(),
        }
    }
}
