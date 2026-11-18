//! The recovery-snapshot store (ADR-0037 D12).
//!
//! An explicit-save model loses everything since the last save if the process
//! dies. A recovery snapshot bounds that window: while a document is dirty the
//! host periodically writes the same archive it would have saved, to a separate
//! location, without touching the document's own file.
//!
//! # Why this is not the document store
//!
//! [`DocumentArchive`](crate::DocumentArchive) writes the user's file and owns
//! atomic replace, `.bak` and conflict detection. This trait writes a *copy* in a
//! carrier-chosen private directory, so its failure can never damage the
//! document. It is written by the same encoder, so a recovery file is a normal
//! `.ymodel` — inspectable, and openable by the ordinary open path. That is what
//! makes restore possible without a new command or a new dialog: the user opens
//! the recovery file.
//!
//! # Lifecycle (the rule that keeps it from rolling a document back)
//!
//! * written only while a document is dirty, at most once per interval;
//! * **discarded after a successful save** — otherwise next start would offer a
//!   stale snapshot and silently undo saved work;
//! * offered only when `record.revision > document.saved_revision`. A snapshot at
//!   or below the saved revision is stale by definition and must be discarded,
//!   not presented.
//!
//! # Keying
//!
//! The key is derived by the host, not by the store: it must be stable across
//! sessions for the same document (a hash of the opened target, else of the
//! document uuid). It is a hex string because it becomes a file name on every
//! native carrier.

use cad_domain::{CadResult, Revision};

/// Stable, file-name-safe key for one document's recovery snapshot.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RecoveryKey(String);

impl RecoveryKey {
    /// Wrap an already-computed hex key.
    ///
    /// Rejects anything that is not `[0-9a-f]`, so a key can never escape the
    /// carrier's recovery directory (`..`, `/`, a Windows drive prefix).
    pub fn from_hex(hex: &str) -> Option<RecoveryKey> {
        if hex.is_empty() || hex.len() > 128 {
            return None;
        }
        let bytes = hex.as_bytes();
        if bytes
            .iter()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
        {
            Some(RecoveryKey(hex.to_string()))
        } else {
            None
        }
    }

    /// The key as written (and as used as a file stem).
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// What a snapshot says about itself, without decoding the archive.
///
/// Kept beside the bytes so a start-up check is one small read, not a full
/// archive decode: deciding "is this stale?" must not cost more than the false
/// positive it prevents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryRecord {
    /// The key this record is stored under.
    pub key: String,
    /// The document identity at snapshot time, when known.
    pub document_uuid: Option<String>,
    /// The document revision the snapshot holds. This is the value compared
    /// against `saved_revision`; it is not a saved-revision marker.
    pub revision: Revision,
    /// The document's display name, for the message that points the user at it.
    pub display_name: Option<String>,
    /// Milliseconds since the Unix epoch when the snapshot was written, or 0
    /// when the carrier has no clock. Carrier-supplied; never invented here.
    pub saved_at_ms: u64,
}

/// One recovered snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoverySnapshot {
    pub record: RecoveryRecord,
    /// The archive bytes: a normal `.ymodel`.
    pub bytes: Vec<u8>,
}

/// Recovery-snapshot storage.
///
/// A carrier without a durable private directory returns an explicit error from
/// [`Persistence::save_recovery`] rather than pretending the snapshot exists;
/// the host then reports that recovery is unavailable instead of silently
/// leaving the user with an unbounded crash window.
pub trait Persistence {
    /// Write (or replace) the snapshot for `record.key`.
    ///
    /// Must be atomic enough that a crash during this write leaves either the
    /// previous snapshot or the new one, never a torn file.
    fn save_recovery(&self, record: &RecoveryRecord, bytes: &[u8]) -> CadResult<()>;

    /// Read the snapshot for `key`; `Ok(None)` when there is none.
    ///
    /// An unreadable snapshot is an error, not `None`: the difference is
    /// "nothing to recover" versus "there may be work that cannot be read".
    fn load_recovery(&self, key: &RecoveryKey) -> CadResult<Option<RecoverySnapshot>>;

    /// Where the snapshot lives, for a message that tells the user what to open.
    ///
    /// A carrier with no filesystem path (a browser, an Android SAF URI) returns
    /// `None`, and the host then names the recovery without a path rather than
    /// inventing one.
    fn recovery_hint(&self, _key: &RecoveryKey) -> Option<String> {
        None
    }

    /// Delete the snapshot for `key`; absent is success.
    ///
    /// Called after a successful save and after an explicit discard, never
    /// speculatively.
    fn discard_recovery(&self, key: &RecoveryKey) -> CadResult<()>;

    /// Every snapshot this carrier can offer, newest first (possibly empty).
    ///
    /// The default is `Ok(Vec::new())` rather than a refusal, unlike the other
    /// methods: the caller is a **list** offered on the start page, and "this
    /// carrier has no snapshots" is a true answer, not a missing capability. A
    /// carrier whose store lives somewhere (a filesystem directory) overrides it;
    /// [`NoRecovery`] therefore needs no implementation.
    ///
    /// An entry that cannot be opened must **not** appear here: a row the user
    /// could click and only then learn it is broken is worse than no row. A store
    /// that can name the snapshots but not read the whole directory is an error,
    /// not an empty list (`Ok(empty)` would mean "nothing to recover").
    fn list_recovery(&self) -> CadResult<Vec<RecoveryRecord>> {
        Ok(Vec::new())
    }
}

/// A [`Persistence`] for a carrier with no durable private directory.
///
/// Every method refuses by name, so a host that installs it can still report
/// "recovery is unavailable here" instead of silently doing nothing.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoRecovery;

const NO_RECOVERY: &str = "unsupported-runtime: recovery";

impl Persistence for NoRecovery {
    fn save_recovery(&self, _record: &RecoveryRecord, _bytes: &[u8]) -> CadResult<()> {
        Err(cad_domain::CadError::Unsupported(NO_RECOVERY.into()))
    }

    fn load_recovery(&self, _key: &RecoveryKey) -> CadResult<Option<RecoverySnapshot>> {
        Err(cad_domain::CadError::Unsupported(NO_RECOVERY.into()))
    }

    fn discard_recovery(&self, _key: &RecoveryKey) -> CadResult<()> {
        Err(cad_domain::CadError::Unsupported(NO_RECOVERY.into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_file_name_safe_or_rejected() {
        assert!(RecoveryKey::from_hex("0123456789abcdef").is_some());
        for bad in ["", "../escape", "ABCDEF", "a/b", "a\\b", "c:", "é"] {
            assert!(
                RecoveryKey::from_hex(bad).is_none(),
                "{bad:?} must not become a path"
            );
        }
        assert!(RecoveryKey::from_hex(&"a".repeat(129)).is_none());
    }

    #[test]
    fn the_null_store_refuses_by_name() {
        let store = NoRecovery;
        let key = RecoveryKey::from_hex("00").expect("key");
        let record = RecoveryRecord {
            key: "00".into(),
            document_uuid: None,
            revision: Revision(3),
            display_name: None,
            saved_at_ms: 0,
        };
        let error = store.save_recovery(&record, b"bytes").expect_err("refused");
        assert!(error.to_string().contains("unsupported-runtime: recovery"));
        assert!(store.load_recovery(&key).is_err());
        assert!(store.discard_recovery(&key).is_err());
    }

    #[test]
    fn the_default_snapshot_list_is_empty_and_not_an_error() {
        // A carrier with no durable private directory has nothing to offer. That
        // is a true answer to a list, so it is `Ok(empty)` rather than the
        // refusal every other method returns.
        let store = NoRecovery;
        assert_eq!(
            store
                .list_recovery()
                .expect("an empty list is not an error"),
            Vec::new()
        );
    }
}
