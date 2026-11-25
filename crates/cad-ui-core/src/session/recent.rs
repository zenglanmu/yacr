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
//!
//! The same open funnel also serves an **explicit path** the carrier received from
//! the OS (macOS `RunEvent::Opened`); that is not a recent row, so it skips the
//! list validation. See [`CadSession::open_path`].

use std::path::Path;

use cad_host::{PathOpenOutcome, RecentEntry};

use super::CadSession;

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

impl CadSession {
    /// The host's real recent-drawings list (newest first, possibly empty).
    ///
    /// Pure read of the durable list the host loaded/recorded; it never touches
    /// the drawing, the revision or the history.
    pub fn recent_files(&self) -> CadRecentFiles {
        CadRecentFiles::supported(self.core.recent_entries())
    }

    /// Reopen one recorded recent drawing by its opaque handle.
    ///
    /// The host validates the handle against its own list (an unknown/crafted
    /// handle is refused, not opened), then starts the ordinary
    /// `begin_open_target` funnel; a real platform failure (missing file, revoked
    /// grant) is returned unchanged. The document is not touched here.
    pub fn open_recent(&self, opaque: &str, display_name: &str) -> Result<(), String> {
        self.core
            .open_recent(opaque, display_name)
            .map_err(|error| super::cad_error_text(&error))
    }

    /// Open one explicit filesystem path at runtime (an OS "open with").
    ///
    /// This is the macOS `RunEvent::Opened` seam — the runtime counterpart of the
    /// startup `--open` drawing, on the **same** funnel
    /// ([`cad_host::HostCore::open_path`]): import worker or synchronous install,
    /// xref policy rooted at the drawing's directory, `openedSource` notice and
    /// recent/source bookkeeping. It is **not** a recent-row reopen, so there is no
    /// list validation: the caller owns the path. `Started` means the platform read
    /// began (the document still arrives through `cad_status`); `Busy` means a save
    /// or load already owns the slot and the carrier should retry rather than drop
    /// the user's file. A platform refusal is returned unchanged.
    pub fn open_path(&self, path: &Path) -> Result<PathOpenOutcome, String> {
        self.core
            .open_path(path)
            .map_err(|error| super::cad_error_text(&error))
    }
}
