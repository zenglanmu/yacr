//! Per-face font load records.
//!
//! [`super::FontLoadReport`] keeps its historical `registered`/`failed`/
//! `unresolved` key lists for compatibility. Those are enough for diagnostics
//! but not for a UI that wants to list every face a load touched (what file,
//! where it came from, and whether it actually registered). A
//! [`FontFaceRecord`] carries that richer, plain-data view; `faces` is additive
//! and never replaces the key lists.

use cad_resources::FontKind;

/// Where a face's bytes came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontFaceSource {
    /// A face from the committed font package/catalog.
    Package,
    /// An explicit host-supplied `--font` entry.
    Explicit,
    /// The host's own system default face.
    System,
}

impl FontFaceSource {
    /// Stable machine label (ASCII) for a UI or diagnostic payload.
    pub fn as_str(self) -> &'static str {
        match self {
            FontFaceSource::Package => "package",
            FontFaceSource::Explicit => "explicit",
            FontFaceSource::System => "system",
        }
    }
}

/// What happened to a face during a load.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontFaceStatus {
    /// Fetched and registered with the shaping engine.
    Loaded,
    /// Resolved but fetch/parse/registration failed; [`FontFaceRecord::detail`]
    /// carries the reason.
    Failed,
    /// Requested by the drawing but absent from the catalog (and from every
    /// other source this load had).
    Unresolved,
    /// A catalog entry declares a font technology this build cannot decode.
    Unsupported,
}

impl FontFaceStatus {
    /// Stable machine label (ASCII) for a UI or diagnostic payload.
    pub fn as_str(self) -> &'static str {
        match self {
            FontFaceStatus::Loaded => "loaded",
            FontFaceStatus::Failed => "failed",
            FontFaceStatus::Unresolved => "unresolved",
            FontFaceStatus::Unsupported => "unsupported",
        }
    }
}

/// One face touched by a font load, with enough detail for a UI list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontFaceRecord {
    /// The lookup key/name this row was recorded under (a drawing request, a
    /// catalog name, a default-face name or [`super::DEFAULT_FALLBACK_KEY`]).
    pub key: String,
    /// File name when the face resolved to one (`None` for an unresolved name
    /// or a system default whose file name is unknown).
    pub file: Option<String>,
    /// Font technology, from the closed wire vocabulary `"shx"`, `"mesh"`,
    /// `"system"` or `"unknown"`. A catalog's arbitrary declared type is never
    /// passed through here; the row's status/detail carries why it was rejected.
    pub kind: String,
    pub source: FontFaceSource,
    pub status: FontFaceStatus,
    /// Failure reason, verbatim, when [`FontFaceStatus::Failed`].
    pub detail: Option<String>,
}

/// Map a catalog [`FontKind`] to the closed label vocabulary used in a record.
///
/// Only the two decodable technologies get their own label; every declared
/// `Other` type collapses to `"unknown"` so the UI contract stays a closed set
/// (the row's `Unsupported` status and `detail` carry the technology).
pub(crate) fn kind_label(kind: &FontKind) -> String {
    match kind {
        FontKind::Shx => "shx".to_string(),
        FontKind::Mesh => "mesh".to_string(),
        FontKind::Other(_) => "unknown".to_string(),
    }
}
