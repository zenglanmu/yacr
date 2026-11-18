//! The Linux carrier's recovery-snapshot store (ADR-0037 D12).
//!
//! A recovery snapshot is an **ordinary `.ymodel`** written to a carrier-private
//! directory, never to the document's own file: a failed snapshot can therefore
//! never damage the document, and "restore" is just opening the file through the
//! ordinary OPEN path (no new command, no new dialog).
//!
//! Layout, one pair of files per document key (`<sha256 hex>` of the opened
//! target, else of the document uuid — see `cad_host::HostCore::recovery_key`):
//!
//! ```text
//! <config dir>/yacr/recovery/<key>.ymodel   the archive bytes
//! <config dir>/yacr/recovery/<key>.json     the RecoveryRecord
//! ```
//!
//! The record is written **after** the bytes, and both through the crate's one
//! atomic replace. A crash between the two can only leave a record describing an
//! *older* revision than the bytes — the safe direction: the host compares
//! `record.revision` against the file's saved revision, so a torn pair is read as
//! "nothing newer to recover", never as "here is newer work than there is".
//!
//! The record is hand-mapped field by field onto `cad_platform::RecoveryRecord`
//! (no `Serialize`/`Deserialize` on the trait type); `serde_json` only provides
//! the strict lexical layer and rejects anything that is not a JSON object with
//! the expected field types.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use cad_domain::{CadError, CadResult, Revision};
use cad_platform::{Persistence, RecoveryKey, RecoveryRecord, RecoverySnapshot};

/// Ceiling on one snapshot, in bytes (512 MiB).
///
/// The snapshot is a full re-encode of the drawing, so its size tracks the
/// document, not the edit. 512 MiB is an order of magnitude above any fixture or
/// realistic drawing this build can encode, and it exists to stop one runaway
/// document from filling the disk with recovery copies: a snapshot that would
/// exceed it is refused with an explicit error (the host then reports that
/// recovery is unavailable for that document) instead of silently writing
/// gigabytes.
pub const RECOVERY_MAX_BYTES: usize = 512 * 1024 * 1024;

/// The stable marker every refusal from this store starts with.
///
/// Stable code, not prose: the host turns it into a status line, and a test can
/// assert the reason without matching a sentence.
pub const RECOVERY_STORE_PREFIX: &str = "recovery.store";

/// Filesystem-backed [`Persistence`] over one private directory.
#[derive(Debug, Clone)]
pub struct FsRecoveryStore {
    dir: PathBuf,
}

impl FsRecoveryStore {
    /// A store rooted at `dir` (the directory itself, e.g. `<config>/yacr/recovery`).
    ///
    /// Nothing is created here: a carrier that never writes a snapshot must not
    /// leave an empty directory behind, so the directory is created on the first
    /// [`Persistence::save_recovery`].
    pub fn new(dir: PathBuf) -> Arc<FsRecoveryStore> {
        Arc::new(FsRecoveryStore { dir })
    }

    /// The directory this store owns.
    pub fn directory(&self) -> &Path {
        &self.dir
    }

    /// The archive path for `key`. `key` is hex-only ([`RecoveryKey`]), so this
    /// can never name anything outside [`Self::directory`].
    fn bytes_path(&self, key: &RecoveryKey) -> PathBuf {
        self.dir.join(format!("{}.ymodel", key.as_str()))
    }

    /// The record path for `key`.
    fn record_path(&self, key: &RecoveryKey) -> PathBuf {
        self.dir.join(format!("{}.json", key.as_str()))
    }
}

impl Persistence for FsRecoveryStore {
    fn save_recovery(&self, record: &RecoveryRecord, bytes: &[u8]) -> CadResult<()> {
        // The record's own key is what names the files, and it is re-validated
        // here: a record built elsewhere must not be able to steer this store at
        // a path (`../`, `a/b`, a drive prefix) even though the host's own keys
        // are already hex.
        let key = RecoveryKey::from_hex(&record.key).ok_or_else(|| {
            CadError::InvalidInput(format!(
                "{RECOVERY_STORE_PREFIX}.invalid_key: {:?} is not a hex key",
                record.key
            ))
        })?;
        if bytes.len() > RECOVERY_MAX_BYTES {
            return Err(CadError::Unsupported(format!(
                "{RECOVERY_STORE_PREFIX}.too_large: {} bytes exceeds the {RECOVERY_MAX_BYTES}-byte limit",
                bytes.len()
            )));
        }
        std::fs::create_dir_all(&self.dir).map_err(|error| {
            CadError::Unsupported(format!(
                "{RECOVERY_STORE_PREFIX}.mkdir: {}: {error}",
                self.dir.display()
            ))
        })?;
        // Bytes first, record second (see the module doc): the pair can then only
        // under-claim, never over-claim, the revision it holds.
        crate::platform::atomic_replace(&self.bytes_path(&key), bytes, false)?;
        let text = encode_record(record);
        crate::platform::atomic_replace(&self.record_path(&key), text.as_bytes(), false)
    }

    fn load_recovery(&self, key: &RecoveryKey) -> CadResult<Option<RecoverySnapshot>> {
        let record_path = self.record_path(key);
        let text = match std::fs::read_to_string(&record_path) {
            Ok(text) => text,
            // No record at all: there is nothing to recover (the record is the
            // index; orphan bytes without one are not a snapshot).
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(CadError::Unsupported(format!(
                    "{RECOVERY_STORE_PREFIX}.record_unreadable: {}: {error}",
                    record_path.display()
                )))
            }
        };
        let record = decode_record(&text).map_err(|reason| {
            CadError::CorruptData(format!(
                "{RECOVERY_STORE_PREFIX}.record_invalid: {}: {reason}",
                record_path.display()
            ))
        })?;
        // A record whose key is not the key it was found under cannot describe
        // this document; refuse it rather than reading a neighbour's bytes.
        if record.key != key.as_str() {
            return Err(CadError::CorruptData(format!(
                "{RECOVERY_STORE_PREFIX}.record_key_mismatch: {:?} found under {:?}",
                record.key,
                key.as_str()
            )));
        }
        // A record *is* a claim that bytes exist: a missing/unreadable archive is
        // an error, not "nothing to recover" (the trait's contract).
        let bytes_path = self.bytes_path(key);
        let metadata = std::fs::metadata(&bytes_path).map_err(|error| {
            CadError::CorruptData(format!(
                "{RECOVERY_STORE_PREFIX}.bytes_missing: {}: {error}",
                bytes_path.display()
            ))
        })?;
        if metadata.len() > RECOVERY_MAX_BYTES as u64 {
            return Err(CadError::CorruptData(format!(
                "{RECOVERY_STORE_PREFIX}.bytes_too_large: {} is {} bytes",
                bytes_path.display(),
                metadata.len()
            )));
        }
        let bytes = std::fs::read(&bytes_path).map_err(|error| {
            CadError::CorruptData(format!(
                "{RECOVERY_STORE_PREFIX}.bytes_unreadable: {}: {error}",
                bytes_path.display()
            ))
        })?;
        Ok(Some(RecoverySnapshot { record, bytes }))
    }

    fn recovery_hint(&self, key: &RecoveryKey) -> Option<String> {
        Some(self.bytes_path(key).to_string_lossy().into_owned())
    }

    fn discard_recovery(&self, key: &RecoveryKey) -> CadResult<()> {
        // Absent is success: the caller discards after a save and after an
        // explicit stale decision, and neither has written a snapshot every time.
        for path in [self.bytes_path(key), self.record_path(key)] {
            match std::fs::remove_file(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(CadError::Unsupported(format!(
                        "{RECOVERY_STORE_PREFIX}.discard_failed: {}: {error}",
                        path.display()
                    )))
                }
            }
        }
        Ok(())
    }
}

/// Serialize a record as a strict JSON object.
///
/// Hand-mapped on purpose: `RecoveryRecord` is a trait type and carries no serde
/// derive, so the wire shape is written and read here, next to the store that
/// owns it.
fn encode_record(record: &RecoveryRecord) -> String {
    serde_json::json!({
        "key": record.key,
        "document_uuid": record.document_uuid,
        "revision": record.revision.0,
        "display_name": record.display_name,
        "saved_at_ms": record.saved_at_ms,
    })
    .to_string()
}

/// Parse [`encode_record`]'s output, refusing anything unexpected.
fn decode_record(text: &str) -> Result<RecoveryRecord, String> {
    let value: serde_json::Value = serde_json::from_str(text).map_err(|error| error.to_string())?;
    let object = value
        .as_object()
        .ok_or_else(|| "the record is not a JSON object".to_string())?;
    let key = object
        .get("key")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "the record has no string key".to_string())?
        .to_string();
    let revision = object
        .get("revision")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| "the record has no unsigned revision".to_string())?;
    let saved_at_ms = object
        .get("saved_at_ms")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    Ok(RecoveryRecord {
        key,
        document_uuid: optional_string(object.get("document_uuid"))?,
        revision: Revision(revision),
        display_name: optional_string(object.get("display_name"))?,
        saved_at_ms,
    })
}

/// A JSON field that is a string or null; any other type is malformed.
fn optional_string(value: Option<&serde_json::Value>) -> Result<Option<String>, String> {
    match value {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::String(text)) => Ok(Some(text.clone())),
        Some(_) => Err("an optional text field is neither a string nor null".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// A fresh directory under the system temp dir (no `tempfile` dependency in
    /// this crate); removed again when the test ends.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> TempDir {
            static SEQUENCE: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "yacr-recovery-{label}-{}-{}",
                std::process::id(),
                SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = std::fs::remove_dir_all(&path);
            TempDir(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn key() -> RecoveryKey {
        RecoveryKey::from_hex("0123456789abcdef").expect("hex key")
    }

    fn record(key: &RecoveryKey, revision: u64) -> RecoveryRecord {
        RecoveryRecord {
            key: key.as_str().to_string(),
            document_uuid: Some("doc-uuid-1".to_string()),
            revision: Revision(revision),
            display_name: Some("plan.ymodel".to_string()),
            saved_at_ms: 7,
        }
    }

    #[test]
    fn a_saved_snapshot_loads_back_byte_for_byte() {
        let temp = TempDir::new("roundtrip");
        let store = FsRecoveryStore::new(temp.0.join("recovery"));
        let key = key();
        let bytes = b"PK\x03\x04a real archive body".to_vec();

        store
            .save_recovery(&record(&key, 9), &bytes)
            .expect("the snapshot is written");
        let loaded = store
            .load_recovery(&key)
            .expect("readable")
            .expect("the snapshot exists");

        assert_eq!(loaded.bytes, bytes);
        assert_eq!(loaded.record, record(&key, 9));
        assert_eq!(
            store.recovery_hint(&key).as_deref(),
            Some(
                store
                    .directory()
                    .join("0123456789abcdef.ymodel")
                    .to_string_lossy()
                    .as_ref()
            )
        );
        // Both halves of the pair are on disk, under the key's own name.
        assert!(store.directory().join("0123456789abcdef.ymodel").is_file());
        assert!(store.directory().join("0123456789abcdef.json").is_file());
    }

    #[test]
    fn an_absent_record_is_none_but_a_missing_archive_is_an_error() {
        let temp = TempDir::new("missing");
        let store = FsRecoveryStore::new(temp.0.join("recovery"));
        let key = key();
        assert_eq!(
            store.load_recovery(&key).expect("absent is not an error"),
            None,
            "nothing was ever written"
        );

        store
            .save_recovery(&record(&key, 4), b"bytes")
            .expect("written");
        std::fs::remove_file(store.directory().join("0123456789abcdef.ymodel"))
            .expect("remove the bytes, keep the record");
        let error = store
            .load_recovery(&key)
            .expect_err("a record without bytes may hide work: it is an error");
        assert!(
            error.to_string().contains("recovery.store.bytes_missing"),
            "{error}"
        );
    }

    #[test]
    fn discard_removes_both_halves_and_absent_is_success() {
        let temp = TempDir::new("discard");
        let store = FsRecoveryStore::new(temp.0.join("recovery"));
        let key = key();
        store
            .save_recovery(&record(&key, 2), b"bytes")
            .expect("written");

        store.discard_recovery(&key).expect("discarded");
        assert!(!store.directory().join("0123456789abcdef.ymodel").exists());
        assert!(!store.directory().join("0123456789abcdef.json").exists());
        assert_eq!(store.load_recovery(&key).expect("still readable"), None);
        // Never written again: a second discard is still success.
        store
            .discard_recovery(&key)
            .expect("absent is success, not an error");
    }

    #[test]
    fn a_key_can_never_escape_the_recovery_directory() {
        let temp = TempDir::new("escape");
        let directory = temp.0.join("recovery");
        let store = FsRecoveryStore::new(directory.clone());
        let key = key();

        // A hostile record key is refused before any path is built.
        for hostile in ["../escape", "/etc/passwd", "a/b", "AB12", ""] {
            let mut hostile_record = record(&key, 1);
            hostile_record.key = hostile.to_string();
            let error = store
                .save_recovery(&hostile_record, b"bytes")
                .expect_err("a non-hex key must be refused");
            assert!(
                error.to_string().contains("recovery.store.invalid_key"),
                "{hostile:?}: {error}"
            );
        }
        // The same rule holds for the key type itself.
        for hostile in ["../escape", "a/b", "c:", "é"] {
            assert!(RecoveryKey::from_hex(hostile).is_none(), "{hostile:?}");
        }

        // A record file whose body claims another key is refused by the store,
        // even though it sits at a legitimate name.
        std::fs::create_dir_all(&directory).expect("dir");
        std::fs::write(
            directory.join("0123456789abcdef.json"),
            r#"{"key":"../escape","revision":3,"saved_at_ms":0}"#,
        )
        .expect("hostile record");
        std::fs::write(directory.join("0123456789abcdef.ymodel"), b"bytes").expect("bytes");
        let error = store
            .load_recovery(&key)
            .expect_err("a mismatched record must be refused");
        assert!(
            error
                .to_string()
                .contains("recovery.store.record_key_mismatch"),
            "{error}"
        );
        // Nothing was written outside the directory by any of the above.
        assert!(!temp.0.join("escape.ymodel").exists());
    }

    #[test]
    fn a_snapshot_over_the_byte_ceiling_is_refused_explicitly() {
        let temp = TempDir::new("bounded");
        let directory = temp.0.join("recovery");
        let store = FsRecoveryStore::new(directory.clone());
        let key = key();
        // The ceiling itself is not allocated: the refusal happens before the
        // write, so asserting the constant and an over-limit length is enough.
        assert_eq!(RECOVERY_MAX_BYTES, 512 * 1024 * 1024);
        let oversized = vec![0_u8; RECOVERY_MAX_BYTES + 1];
        let error = store
            .save_recovery(&record(&key, 1), &oversized)
            .expect_err("refused");
        assert!(
            error.to_string().contains("recovery.store.too_large"),
            "{error}"
        );
        assert!(
            !directory.exists(),
            "a refused snapshot must not even create the directory"
        );
    }
}
