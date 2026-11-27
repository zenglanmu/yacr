//! The Match Photo panel's read: the photo-matching model is **not** modeled.
//!
//! SketchUp's Match Photo imports a photograph and solves a camera from vanishing
//! points so the model can be drawn over it. This build has no photo storage, no
//! camera solve and no photo background, so the panel must say so rather than
//! show an empty list that would read as "no photos in this document".
//!
//! The read is a real projection of the build's capability: a constant
//! `modeled: false` plus the stable refusal the command path returns. The panel's
//! action dispatches `MATCHPHOTO`, whose handler returns the same marker.

use crate::HostCore;

/// The stable reason every Match Photo entry point reports.
pub const MATCH_PHOTO_NOT_MODELED: &str =
    "matchphoto.not_implemented: this build has no photo-matching model (no photo import, camera \
     solve or photo background)";

/// The Match Photo panel's state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchPhotoStatus {
    /// Always `false` in this build.
    pub modeled: bool,
    /// The stable reason the panel shows.
    pub reason: &'static str,
}

impl HostCore {
    /// The Match Photo panel's state (a constant: there is no photo model).
    pub fn match_photo(&self) -> MatchPhotoStatus {
        MatchPhotoStatus {
            modeled: false,
            reason: MATCH_PHOTO_NOT_MODELED,
        }
    }
}
