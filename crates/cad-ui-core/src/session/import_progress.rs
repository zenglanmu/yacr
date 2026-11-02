//! The live import-progress block of the frozen wire.
//!
//! [`CadImportProgress`] projects only a genuinely running import; a terminal
//! snapshot is never turned into a fabricated 100% bar.

use serde::Serialize;

/// Live import progress for the frontend progress affordance.
///
/// Present **only while an import is genuinely running**: a terminal snapshot
/// (opened/cancelled/failed) is never projected here, so an idle or finished
/// document carries `importProgress: null` and the chrome shows no bar. Every
/// field is a real observed value — `entities_total`/`bytes` stay `null` when
/// the importer did not expose them, which the UI renders as an indeterminate
/// bar without a byte count; nothing is extrapolated to 100%.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CadImportProgress {
    /// Stable machine key of the latest phase (e.g. `"entities"`); `null` before
    /// the first progress tick. The UI maps it to a catalog label.
    pub phase: Option<String>,
    /// Real entities normalised/inserted so far (`0` before the first tick).
    pub entities_done: u64,
    /// Total entities expected, or `null` when the format does not expose it.
    pub entities_total: Option<u64>,
    /// Source bytes measured so far, only when actually known.
    pub bytes: Option<u64>,
    /// Whether a cancel request can still be issued for this import.
    pub cancellable: bool,
}

impl CadImportProgress {
    /// Project a running [`cad_app::ImportProgressSnapshot`]; a terminal snapshot
    /// maps to `None`, so the affordance is only ever driven by a live import.
    pub(super) fn from_snapshot(snapshot: &cad_app::ImportProgressSnapshot) -> Option<Self> {
        if !snapshot.running {
            return None;
        }
        Some(CadImportProgress {
            phase: snapshot.phase_key().map(str::to_string),
            entities_done: snapshot.entities_done as u64,
            entities_total: snapshot.entities_total.map(|total| total as u64),
            bytes: snapshot.bytes,
            cancellable: snapshot.cancellable,
        })
    }
}
