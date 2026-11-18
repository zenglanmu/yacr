//! The `cad_recovery_files` / `cad_open_recovery` wire surface.
//!
//! A recovery snapshot is a carrier-private copy of the document (ADR-0037 D12),
//! not CAD document data and not a drawing command: a browser or SAF carrier has
//! no durable private directory, so its adapter refuses by name, and the two
//! names are deliberately absent from `cad_commands` / the typed command table.
//!
//! The answer shape is exactly what the start panel parses:
//! `{ supported, entries, reason }` with `camelCase` rows. A native host answers
//! `supported: true` with its real list (possibly empty); a carrier without the
//! capability answers `supported: false`, an empty list and the stable
//! `unsupported-runtime: <command>` reason.
//!
//! # Key only, never a path
//!
//! [`RecoveryEntry::key`] is the store's hex key. It is the **whole** open
//! request: the host validates it ([`cad_platform::RecoveryKey`]) and resolves
//! the location from its own store, so no path ever travels to or from the UI.
//! A row the store cannot decode or cannot pair with its archive is not listed at
//! all, so every visible row is openable.

use cad_platform::RecoveryRecord;

use super::CadSession;

/// The recovery-list command name (also the browser adapter's refusal subject).
pub const RECOVERY_FILES_COMMAND: &str = "cad_recovery_files";

/// The open-recovery command name.
pub const OPEN_RECOVERY_COMMAND: &str = "cad_open_recovery";

/// One listed snapshot, exactly the wire row the start panel consumes.
///
/// The key is opaque to the UI and safe in a file name (hex only). `displayName`
/// is the document name recorded at snapshot time (`None` when the carrier had
/// none), `revision` is the revision the bytes hold, and `savedAtMs` is the
/// carrier's own timestamp (`0` when it has no clock) — never invented here.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryEntry {
    pub key: String,
    pub display_name: Option<String>,
    pub revision: u64,
    pub saved_at_ms: u64,
}

impl From<RecoveryRecord> for RecoveryEntry {
    fn from(record: RecoveryRecord) -> Self {
        Self {
            key: record.key,
            display_name: record.display_name,
            revision: record.revision.0,
            saved_at_ms: record.saved_at_ms,
        }
    }
}

/// One `cad_recovery_files` answer.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CadRecoveryFiles {
    /// Whether this carrier can offer recovery snapshots at all.
    pub supported: bool,
    /// The real rows, newest first (empty on an unsupported carrier).
    pub entries: Vec<RecoveryEntry>,
    /// The stable reason when `supported` is false, otherwise `None`.
    pub reason: Option<String>,
}

impl CadRecoveryFiles {
    /// A native carrier's real list (possibly empty; never invented).
    pub fn supported(entries: Vec<RecoveryEntry>) -> Self {
        Self {
            supported: true,
            entries,
            reason: None,
        }
    }

    /// A carrier that cannot offer recovery snapshots.
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
    /// The recovery snapshots this carrier can offer, newest first.
    ///
    /// A read of the carrier's own store (never the drawing, revision or
    /// history). An unreadable store is a real error: "there may be work that
    /// cannot be seen" must not be shown as "no snapshots".
    pub fn recovery_files(&self) -> Result<CadRecoveryFiles, String> {
        let records = self
            .core
            .list_recovery()
            .map_err(|error| super::cad_error_text(&error))?;
        Ok(CadRecoveryFiles::supported(
            records.into_iter().map(RecoveryEntry::from).collect(),
        ))
    }

    /// Start opening the recovery snapshot filed under `key`.
    ///
    /// `key` is the whole input — the host validates it and resolves the path
    /// from its own store, so the UI never names a file. A key the store does not
    /// list is refused, and a real platform failure is returned unchanged.
    pub fn open_recovery(&self, key: &str) -> Result<(), String> {
        self.core
            .open_recovery(key)
            .map_err(|error| super::cad_error_text(&error))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cad_domain::Revision;

    #[test]
    fn a_record_becomes_the_camel_case_row_the_panel_parses() {
        let entry = RecoveryEntry::from(RecoveryRecord {
            key: "0123456789abcdef".to_string(),
            document_uuid: Some("doc-uuid-1".to_string()),
            revision: Revision(12),
            display_name: Some("plan.ymodel".to_string()),
            saved_at_ms: 1_700_000_000_000,
        });
        assert_eq!(
            entry,
            RecoveryEntry {
                key: "0123456789abcdef".to_string(),
                display_name: Some("plan.ymodel".to_string()),
                revision: 12,
                saved_at_ms: 1_700_000_000_000,
            }
        );
        let json = serde_json::to_value(&entry).expect("serializable");
        assert_eq!(
            json,
            serde_json::json!({
                "key": "0123456789abcdef",
                "displayName": "plan.ymodel",
                "revision": 12,
                "savedAtMs": 1_700_000_000_000_u64,
            }),
            "the wire spelling is camelCase"
        );
    }

    #[test]
    fn the_unsupported_answer_carries_the_stable_command_marker() {
        let answer = CadRecoveryFiles::unsupported(RECOVERY_FILES_COMMAND);
        assert!(!answer.supported);
        assert!(answer.entries.is_empty());
        assert_eq!(
            answer.reason.as_deref(),
            Some("unsupported-runtime: cad_recovery_files")
        );
        assert_eq!(RECOVERY_FILES_COMMAND, "cad_recovery_files");
        assert_eq!(OPEN_RECOVERY_COMMAND, "cad_open_recovery");
    }
}
