//! Recent drawings: the durable root `recentFiles` array and its wire row.
//!
//! `recentFiles` is **user data**, not an Options setting: it is a root-level
//! `preference.json` leaf exactly like [`crate::settings_root`]'s appearance
//! leaves, but it is deliberately not in
//! [`crate::settings_root::ROOT_PREFERENCE_KEYS`], [`crate::HostSettings`] or
//! [`crate::HOST_ALLOWED_PATHS_JSON`]. The host owns the list, records one row
//! after a **successful** open, and re-emits the array on every preference write
//! ([`crate::HostCore::persist_preference`]) so an unrelated setting write can
//! never drop it.
//!
//! # Trust rule for the persisted array
//!
//! A malformed array entry makes the **whole list** untrusted and the host
//! answers an empty list: there is no partial salvage. A recent row is a file
//! handle the host will later reopen, so a row that does not round-trip exactly
//! (missing/blank name or handle, wrong field type) is not "one bad row" — the
//! document is not the shape this build writes, and guessing which rows are real
//! is how a fabricated entry appears. The caller records an empty list, never an
//! invented one. An absent file, an absent key or a non-array value is normal
//! and also yields the empty list.
//!
//! The host never parses an opaque handle here: `location` is supplied by the
//! platform ([`cad_platform::PlatformFiles::recent_location`]), and the handle
//! itself is passed back to the platform unchanged when a row is reopened.

use super::*;

use cad_platform::FileTarget;

/// The root-level durable preference key holding the recent-drawings array.
pub const RECENT_FILES_PREFERENCE_KEY: &str = "recentFiles";

/// The most rows the host keeps (and the UI's defensive cap).
pub const RECENT_FILES_CAP: usize = 10;

/// One recent drawing, exactly the wire row the start panel consumes.
///
/// `camelCase` is the one shape shared by the durable array, the
/// `cad_recent_files` answer and the UI's parser — there is no second spelling.
/// `opaque` is the platform's own handle (an absolute path, a SAF URI, …); the
/// UI never decodes it. `location` is the platform hint
/// ([`cad_platform::PlatformFiles::recent_location`]) or `None`, never an
/// invented path.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentEntry {
    pub display_name: String,
    pub opaque: String,
    pub location: Option<String>,
    pub opened_at_ms: u64,
}

impl RecentEntry {
    /// The platform target this row reopens (the handle plus its display name).
    pub fn target(&self) -> FileTarget {
        FileTarget {
            display_name: self.display_name.clone(),
            opaque: self.opaque.clone(),
        }
    }
}

/// Deduplicate by `opaque` (first occurrence wins, so the input must be newest
/// first) and cap at [`RECENT_FILES_CAP`].
fn normalized(entries: Vec<RecentEntry>) -> Vec<RecentEntry> {
    let mut seen = std::collections::HashSet::new();
    let mut kept = Vec::with_capacity(entries.len().min(RECENT_FILES_CAP));
    for entry in entries {
        if kept.len() == RECENT_FILES_CAP {
            break;
        }
        if seen.insert(entry.opaque.clone()) {
            kept.push(entry);
        }
    }
    kept
}

/// Extract the durable recent-drawings array from a preference document.
///
/// Absent/malformed documents and a malformed row yield an empty list (see the
/// module docs); a valid array is deduplicated by handle and capped.
pub fn recent_files_from_preference(text: &str) -> Vec<RecentEntry> {
    let Ok(document) = serde_json::from_str::<serde_json::Value>(text) else {
        return Vec::new();
    };
    let Some(value) = document.get(RECENT_FILES_PREFERENCE_KEY) else {
        return Vec::new();
    };
    let Some(rows) = value.as_array() else {
        return Vec::new();
    };
    let mut entries = Vec::with_capacity(rows.len().min(RECENT_FILES_CAP));
    for row in rows {
        let Ok(entry) = serde_json::from_value::<RecentEntry>(row.clone()) else {
            return Vec::new();
        };
        if entry.display_name.trim().is_empty() || entry.opaque.trim().is_empty() {
            return Vec::new();
        }
        entries.push(entry);
    }
    normalized(entries)
}

/// Remove the recent-drawings leaf from a durable preference document.
///
/// The store's allowed-path validation is strict, so the array is captured
/// separately ([`recent_files_from_preference`]) and stripped here before the
/// rest of the document is applied. Text that is not valid JSON is returned
/// as-is so the store reports its own parse error.
pub(crate) fn without_recent_files(text: &str) -> String {
    let Ok(mut document) = serde_json::from_str::<serde_json::Value>(text) else {
        return text.to_string();
    };
    if let Some(object) = document.as_object_mut() {
        object.remove(RECENT_FILES_PREFERENCE_KEY);
    }
    serde_json::to_string(&document).unwrap_or_else(|_| text.to_string())
}

/// Merge the recent-drawings array into the projected preference document.
///
/// Every preference write goes through this, so the leaf survives an unrelated
/// settings change. An empty list removes the leaf (a real clear drops it). A
/// projection that is not a JSON object is an explicit error, never a silently
/// rewritten file.
pub(crate) fn merge_recent_files_into_preference(
    projected: &str,
    entries: &[RecentEntry],
) -> CadResult<String> {
    let mut document: serde_json::Value = serde_json::from_str(projected).map_err(|error| {
        CadError::InvalidInput(format!("preference projection is not JSON: {error}"))
    })?;
    let object = document.as_object_mut().ok_or_else(|| {
        CadError::InvalidInput("preference projection is not a JSON object".into())
    })?;
    if entries.is_empty() {
        object.remove(RECENT_FILES_PREFERENCE_KEY);
    } else {
        let rows = serde_json::to_value(normalized(entries.to_vec())).map_err(|error| {
            CadError::InvalidInput(format!("recent drawings are not JSON: {error}"))
        })?;
        object.insert(RECENT_FILES_PREFERENCE_KEY.to_string(), rows);
    }
    serde_json::to_string(&document)
        .map_err(|error| CadError::InvalidInput(format!("preference write is not JSON: {error}")))
}

/// Insert one successful open at the front, deduplicated by handle.
///
/// The newest row wins: an existing row with the same `opaque` is removed and
/// re-inserted with the new display name/location/time. The list is capped at
/// [`RECENT_FILES_CAP`], so the oldest row falls off. A failed or cancelled
/// interaction never calls this.
pub fn record_success(
    entries: &mut Vec<RecentEntry>,
    target: &FileTarget,
    location: Option<String>,
    now_ms: u64,
) {
    entries.retain(|entry| entry.opaque != target.opaque);
    entries.insert(
        0,
        RecentEntry {
            display_name: target.display_name.clone(),
            opaque: target.opaque.clone(),
            location,
            opened_at_ms: now_ms,
        },
    );
    entries.truncate(RECENT_FILES_CAP);
}

/// The wall-clock time one record is stamped with, in milliseconds since the
/// Unix epoch.
///
/// `web_time::SystemTime` is `std`'s off wasm and the browser clock on wasm (the
/// host crate is compiled for both). A clock before the epoch is not a real
/// timestamp; it yields `0` rather than a wrapped huge value.
pub fn now_ms() -> u64 {
    web_time::SystemTime::now()
        .duration_since(web_time::UNIX_EPOCH)
        .map(|duration| u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

impl HostCore {
    /// The real recent-drawings list, newest first (possibly empty).
    pub fn recent_entries(&self) -> Vec<RecentEntry> {
        self.recent_files.borrow().clone()
    }

    /// Record one successful open and persist it.
    ///
    /// The platform hint is captured first, then the platform is told to start
    /// the next chooser from this target. A persistence failure rolls the list
    /// back and returns it: a row that the next start would lose must not look
    /// recorded (`[crate::settings::preference_persist_refusal]`). Never called
    /// for a failed or cancelled delivery.
    pub(crate) fn record_recent_success(&self, target: &FileTarget) -> CadResult<()> {
        let location = self.files.recent_location(target);
        self.files.set_recent_target(Some(target));
        let previous = self.recent_files.borrow().clone();
        record_success(
            &mut self.recent_files.borrow_mut(),
            target,
            location,
            now_ms(),
        );
        if let Err(error) = self.persist_preference() {
            *self.recent_files.borrow_mut() = previous;
            return Err(crate::settings::preference_persist_refusal(&error));
        }
        Ok(())
    }

    /// Seed the next open chooser with the newest durable recent entry.
    ///
    /// Called once after the preference document is loaded. An empty list leaves
    /// the platform hint unset (no invented directory).
    pub(crate) fn seed_recent_target(&self) {
        if let Some(first) = self.recent_files.borrow().first() {
            self.files.set_recent_target(Some(&first.target()));
        }
    }

    /// Reopen one recorded recent drawing by its opaque handle.
    ///
    /// The handle must be one of this host's own rows: the durable list is the
    /// only source of a target, so a crafted `opaque` cannot make the host open
    /// an arbitrary path. The stored display name is used (the request's copy is
    /// only a label and is never trusted over the record). The open then follows
    /// the ordinary `begin_open_target` funnel, so a missing file / revoked grant
    /// / platform failure propagates unchanged.
    pub fn open_recent(&self, opaque: &str, display_name: &str) -> CadResult<()> {
        if opaque.trim().is_empty() {
            return Err(CadError::InvalidInput(
                "recent.invalid_payload: an empty file handle cannot be reopened".into(),
            ));
        }
        let Some(target) = self
            .recent_files
            .borrow()
            .iter()
            .find(|entry| entry.opaque == opaque)
            .map(RecentEntry::target)
        else {
            return Err(CadError::InvalidInput(format!(
                "recent.unknown: {display_name:?} is not in this host's recent drawings"
            )));
        };
        // Same admission rules as the chooser funnel: an open never races a save
        // and never overlaps another load.
        if self.save_pending() {
            return Err(CadError::Unsupported(
                "file operation already in progress".into(),
            ));
        }
        if !self.options.headless && self.loading() {
            return Ok(());
        }
        self.begin_open_attempt();
        self.open_pending.set(true);
        if let Err(error) = self.files.begin_open_target(&target) {
            self.open_pending.set(false);
            return Err(error);
        }
        self.drain_files()?;
        if self.open_pending.get() {
            if !self.options.headless {
                self.set_catalog_status("linux.picker_waiting", &[])?;
            }
        } else {
            self.grade_pending_open()?;
        }
        Ok(())
    }
}
