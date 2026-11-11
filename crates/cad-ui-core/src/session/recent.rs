//! The `cad_recent_files` / `cad_open_recent` wire surface.
//!
//! Recent drawings are **host file data**, not CAD document data and not a
//! drawing command: the browser carriers cannot reopen a past file without a new
//! user gesture, so their adapter refuses by name, and the two names are
//! deliberately absent from `cad_commands` / the typed command table.
//!
//! The answer shape is exactly what the start panel parses:
//! `{ supported, entries, reason }` with `camelCase` rows. A native host answers
//! `supported: true` with its real list (possibly empty); a carrier without the
//! capability answers `supported: false`, an empty list and the stable
//! `unsupported-runtime: <command>` reason. A malformed/absent preference file
//! is an empty list, never a fabricated row.

use cad_host::RecentEntry;

/// The recent-list command name (also the browser adapter's refusal subject).
pub const RECENT_FILES_COMMAND: &str = "cad_recent_files";

/// The reopen command name.
pub const OPEN_RECENT_COMMAND: &str = "cad_open_recent";

/// One `cad_recent_files` answer.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CadRecentFiles {
    /// Whether this carrier tracks recents at all.
    pub supported: bool,
    /// The stored rows, newest first (empty on an unsupported carrier).
    pub entries: Vec<RecentEntry>,
    /// The stable reason when `supported` is false, otherwise `None`.
    pub reason: Option<String>,
}

impl CadRecentFiles {
    /// A native carrier's real list (possibly empty; never invented).
    pub fn supported(entries: Vec<RecentEntry>) -> Self {
        Self {
            supported: true,
            entries,
            reason: None,
        }
    }

    /// A carrier that cannot track recents.
    ///
    /// The reason is the one stable marker the chrome already understands for a
    /// missing capability (`unsupported-runtime: <command>`), so a caller never
    /// needs a second vocabulary.
    pub fn unsupported(command: &str) -> Self {
        Self {
            supported: false,
            entries: Vec::new(),
            reason: Some(format!("unsupported-runtime: {command}")),
        }
    }
}
