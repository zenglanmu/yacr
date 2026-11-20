//! SAF byte transfer: the write ordering, the previous-copy decision and the
//! one notice this carrier owes the user.
//!
//! The Kotlin lane owns the `ContentResolver` calls; this module owns the
//! *policy* around them, so it is compiled (and tested) on the development host
//! with a fake [`SafIo`] instead of a device.
//!
//! # What SAF cannot do (said out loud)
//!
//! The Storage Access Framework has **no atomic replace**: a write to a
//! `content://` document truncates it, and a provider that fails mid-stream can
//! leave it short. The carrier therefore:
//!
//! 1. reads the target's current bytes first (a target that does not exist yet
//!    is an ordinary first save);
//! 2. tries to create a sibling `<name>.bak` document with those bytes through
//!    the provider (`DocumentsContract.createDocument`), which is best-effort:
//!    many providers refuse a sibling create, and that is the common case;
//! 3. writes the new bytes through a staged cache file (see the Kotlin
//!    `documentWriteUri`), so an encode/stream failure that the platform detects
//!    before opening the output stream never touches the target;
//! 4. when step 2 did **not** happen, latches [`SAVE_NO_BACKUP_STATUS`] so the
//!    host publishes `save.no_backup` on the successful overwrite. The carrier
//!    never implies a copy exists when it does not.
//!
//! "Atomic" is deliberately not claimed anywhere on this path: the function is
//! named after the trait method (`write_atomic`, the shared `PlatformFiles`
//! seam), not after a guarantee SAF does not provide.

#[cfg(not(test))]
use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(test)]
use std::cell::Cell;

/// The catalog key published after an overwrite that could not keep a copy.
///
/// The key (not a sentence) is what the host status carries; it exists in both
/// i18n catalogs. This is the same "one free-text notice channel" the shared
/// `cad_status.error` uses for `recovery.available` (see
/// `cad_ui_core::session::diagnostics::NOTICE_KEYS`); this carrier cannot add a
/// notice key to that closed list, so it latches the key into the mirrored
/// `cad_status` in [`crate::host::tick`].
pub const SAVE_NO_BACKUP_STATUS: &str = "save.no_backup";

/// Latched until the user acts again: a `true` here means "the last completed
/// save overwrote a document without keeping a previous copy".
///
/// The production carrier has exactly one CAD session and one SAF lane, so the
/// latch is process-global there; it is cleared when a command is admitted or a
/// document is opened, so a notice can never outlive the action that produced it
/// by more than that.
///
/// Under `cfg(test)` it is **per-thread** instead: the host tests in this module
/// run in parallel in one binary, and a shared latch makes their outcomes depend
/// on each other's interleaving. Each test performs its own note/read/clear on
/// its own thread, so a thread-local is the same single-latch model without the
/// cross-test coupling. Production code is unchanged (`#[cfg(not(test))]`).
#[cfg(not(test))]
static SAVE_WITHOUT_BACKUP: AtomicBool = AtomicBool::new(false);

#[cfg(test)]
thread_local! {
    /// Per-thread test double for the process-global latch above.
    static SAVE_WITHOUT_BACKUP: Cell<bool> = const { Cell::new(false) };
}

/// Record that an overwrite completed without a previous copy.
pub(crate) fn note_save_without_backup() {
    #[cfg(not(test))]
    SAVE_WITHOUT_BACKUP.store(true, Ordering::SeqCst);
    #[cfg(test)]
    SAVE_WITHOUT_BACKUP.with(|latch| latch.set(true));
}

/// Whether the notice is still owed to the user (does not consume it).
pub fn save_without_backup_notice() -> bool {
    #[cfg(not(test))]
    {
        SAVE_WITHOUT_BACKUP.load(Ordering::SeqCst)
    }
    #[cfg(test)]
    {
        SAVE_WITHOUT_BACKUP.with(Cell::get)
    }
}

/// Retire the notice (a new user action supersedes it).
pub fn clear_save_without_backup_notice() {
    #[cfg(not(test))]
    SAVE_WITHOUT_BACKUP.store(false, Ordering::SeqCst);
    #[cfg(test)]
    SAVE_WITHOUT_BACKUP.with(|latch| latch.set(false));
}

/// Whether an overwrite kept a previous copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverwriteSafety {
    /// The provider accepted the sibling `<name>.bak` document.
    Kept,
    /// No previous copy exists: the user must be told (`save.no_backup`).
    Lost,
}

/// The capability decision, in one place.
///
/// SAF has no atomic replace and no guaranteed sibling-create, so the **only**
/// input is whether the provider actually accepted a backup document for this
/// overwrite. Anything else (an assumed rename, an assumed provider behaviour)
/// would be a claim this carrier cannot verify, so there is no second branch.
pub fn overwrite_safety(backup_written: bool) -> OverwriteSafety {
    if backup_written {
        OverwriteSafety::Kept
    } else {
        OverwriteSafety::Lost
    }
}

/// The byte-level SAF operations the write path needs.
///
/// Implemented by the JNI bridge on the Android target ([`crate::saf`]) and by a
/// fake in the tests here; the write ordering and the notice latch are tested on
/// the development host against the fake.
pub(crate) trait SafIo {
    /// Read the document's current bytes; `Ok(None)` means it does not exist.
    fn read(&self, uri: &str) -> Result<Option<Vec<u8>>, String>;

    /// Write `bytes` to the document (staged through a cache file by the Kotlin
    /// lane, then streamed into the provider).
    fn write(&self, uri: &str, bytes: &[u8]) -> Result<(), String>;

    /// Create a sibling `<display name>.bak` document holding `bytes`.
    ///
    /// `Err` is the common, expected answer (the provider does not support a
    /// sibling create); it is not retried and never fails the save.
    fn write_backup(&self, uri: &str, bytes: &[u8]) -> Result<(), String>;
}

/// Write `bytes` to `uri` in the one order SAF allows.
///
/// See the module docs: read the previous bytes, try the sibling backup, then
/// write. A failed target write is an error (and leaves the latch untouched, so
/// a failed save never publishes a success notice); a successful overwrite
/// without a backup latches [`SAVE_NO_BACKUP_STATUS`].
pub(crate) fn write_via_saf(io: &dyn SafIo, uri: &str, bytes: &[u8]) -> Result<(), String> {
    let previous = io.read(uri)?;
    let lost_copy = match previous {
        // The document did not exist: there is nothing to lose.
        None => false,
        // `ACTION_CREATE_DOCUMENT` creates an empty document, so a brand-new save
        // target exists with zero bytes. There is nothing to keep, and no notice is
        // owed: publishing "cannot keep a previous copy" on every first save would
        // be noise, not information.
        Some(previous) if previous.is_empty() => false,
        Some(previous) => {
            let backup_written = io.write_backup(uri, &previous).is_ok();
            overwrite_safety(backup_written) == OverwriteSafety::Lost
        }
    };
    io.write(uri, bytes)?;
    // Only a completed write answers "what is on disk now": a failed write leaves
    // any earlier notice in place (the last *successful* save still lacked a copy).
    clear_save_without_backup_notice();
    if lost_copy {
        note_save_without_backup();
    }
    Ok(())
}

/// Read a target's current bytes for an in-place save's conflict check.
///
/// An absent document is an error, never empty bytes: the trait's contract says
/// the save funnel only treats an error as "no previous archive" for a target it
/// was not opened from, and an in-place save it cannot re-read must refuse.
pub(crate) fn read_via_saf(io: &dyn SafIo, uri: &str) -> Result<Vec<u8>, String> {
    match io.read(uri)? {
        Some(bytes) => Ok(bytes),
        None => Err(format!("the document at {uri} does not exist")),
    }
}

/// The real SAF byte operations, through the JNI bridge (Android only).
///
/// The fake the host tests use lives in this module's test block; this type is
/// the only production implementation of [`SafIo`], so the write ordering above
/// is the same code the device runs.
#[cfg(target_os = "android")]
pub(crate) struct JniSaf;

#[cfg(target_os = "android")]
impl SafIo for JniSaf {
    fn read(&self, uri: &str) -> Result<Option<Vec<u8>>, String> {
        crate::saf::read_uri_sync(uri)
    }

    fn write(&self, uri: &str, bytes: &[u8]) -> Result<(), String> {
        crate::saf::write_uri(uri, bytes)
    }

    fn write_backup(&self, uri: &str, bytes: &[u8]) -> Result<(), String> {
        crate::saf::write_sibling_backup(uri, bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    /// A fake SAF: an in-memory document set, one switch for a provider that
    /// refuses the sibling backup, one for a failed target write.
    #[derive(Default)]
    struct FakeSaf {
        documents: RefCell<BTreeMap<String, Vec<u8>>>,
        backups: RefCell<BTreeMap<String, Vec<u8>>>,
        backup_supported: bool,
        write_fails: bool,
    }

    impl FakeSaf {
        fn with_document(uri: &str, bytes: &[u8]) -> FakeSaf {
            let saf = FakeSaf::default();
            saf.documents
                .borrow_mut()
                .insert(uri.to_string(), bytes.to_vec());
            saf
        }

        fn document(&self, uri: &str) -> Option<Vec<u8>> {
            self.documents.borrow().get(uri).cloned()
        }

        fn backup(&self, uri: &str) -> Option<Vec<u8>> {
            self.backups.borrow().get(uri).cloned()
        }
    }

    impl SafIo for FakeSaf {
        fn read(&self, uri: &str) -> Result<Option<Vec<u8>>, String> {
            Ok(self.document(uri))
        }

        fn write(&self, uri: &str, bytes: &[u8]) -> Result<(), String> {
            if self.write_fails {
                // Models the platform refusing before the output stream opens:
                // the provider's document is untouched.
                return Err("the provider refused the output stream".to_string());
            }
            self.documents
                .borrow_mut()
                .insert(uri.to_string(), bytes.to_vec());
            Ok(())
        }

        fn write_backup(&self, uri: &str, bytes: &[u8]) -> Result<(), String> {
            if !self.backup_supported {
                return Err("the provider has no sibling document create".to_string());
            }
            self.backups
                .borrow_mut()
                .insert(uri.to_string(), bytes.to_vec());
            Ok(())
        }
    }

    #[test]
    fn a_first_save_writes_without_a_notice_and_without_a_backup() {
        let saf = FakeSaf::default();
        write_via_saf(&saf, "content://d/1", b"new document").expect("written");
        assert_eq!(
            saf.document("content://d/1").as_deref(),
            Some(&b"new document"[..])
        );
        assert_eq!(saf.backup("content://d/1"), None, "nothing to back up");
        assert!(
            !save_without_backup_notice(),
            "a new document overwrites nothing, so no notice is owed"
        );
    }

    #[test]
    fn an_overwrite_with_a_sibling_backup_keeps_the_previous_bytes() {
        let saf = FakeSaf::with_document("content://d/2", b"previous");
        let mut saf = saf;
        saf.backup_supported = true;
        write_via_saf(&saf, "content://d/2", b"next").expect("written");
        assert_eq!(saf.document("content://d/2").as_deref(), Some(&b"next"[..]));
        assert_eq!(
            saf.backup("content://d/2").as_deref(),
            Some(&b"previous"[..]),
            "the previous bytes survive in the sibling .bak document"
        );
        assert!(!save_without_backup_notice(), "a copy exists");
        clear_save_without_backup_notice();
    }

    #[test]
    fn an_overwrite_without_a_backup_latches_save_no_backup() {
        // (d) The common case: the provider refuses a sibling create. The save
        // succeeds, and the carrier says out loud that no copy was kept.
        clear_save_without_backup_notice();
        let saf = FakeSaf::with_document("content://d/3", b"previous");
        write_via_saf(&saf, "content://d/3", b"next").expect("written");
        assert_eq!(saf.document("content://d/3").as_deref(), Some(&b"next"[..]));
        assert_eq!(saf.backup("content://d/3"), None);
        assert!(
            save_without_backup_notice(),
            "an overwrite without a copy must latch {SAVE_NO_BACKUP_STATUS}"
        );
        assert_eq!(SAVE_NO_BACKUP_STATUS, "save.no_backup");
        clear_save_without_backup_notice();
        assert!(!save_without_backup_notice(), "a user action retires it");
    }

    #[test]
    fn a_failed_write_leaves_the_document_untouched_and_latches_nothing() {
        // (e) The staged-cache path exists so a failure the platform detects
        // before opening the output stream never touches the target. (SAF has no
        // atomic replace: a mid-stream provider failure can still truncate, which
        // is exactly why the .bak attempt above exists.)
        clear_save_without_backup_notice();
        let mut saf = FakeSaf::with_document("content://d/4", b"previous");
        saf.write_fails = true;
        let error = write_via_saf(&saf, "content://d/4", b"next").expect_err("refused");
        assert!(error.contains("output stream"), "{error}");
        assert_eq!(
            saf.document("content://d/4").as_deref(),
            Some(&b"previous"[..]),
            "the existing document bytes are untouched"
        );
        assert!(!save_without_backup_notice(), "a failed save is not a save");
    }

    #[test]
    fn a_created_empty_save_target_is_a_first_save_not_an_overwrite() {
        // `ACTION_CREATE_DOCUMENT` creates the target, so it exists with zero
        // bytes. Nothing is lost, no `.bak` is attempted and no notice is owed.
        clear_save_without_backup_notice();
        let saf = FakeSaf::with_document("content://d/8", b"");
        write_via_saf(&saf, "content://d/8", b"first content").expect("written");
        assert_eq!(
            saf.document("content://d/8").as_deref(),
            Some(&b"first content"[..])
        );
        assert_eq!(saf.backup("content://d/8"), None);
        assert!(
            !save_without_backup_notice(),
            "an empty target loses nothing"
        );
    }

    #[test]
    fn a_successful_write_clears_a_stale_notice() {
        note_save_without_backup();
        let mut saf = FakeSaf::with_document("content://d/5", b"previous");
        saf.backup_supported = true;
        write_via_saf(&saf, "content://d/5", b"next").expect("written");
        assert!(
            !save_without_backup_notice(),
            "the newest completed write's own answer replaces the previous notice"
        );
    }

    #[test]
    fn a_failed_write_keeps_the_earlier_notice() {
        // The earlier notice described a real completed overwrite; a failed attempt
        // has no answer of its own, so it must not erase that fact.
        note_save_without_backup();
        let mut saf = FakeSaf::with_document("content://d/9", b"previous");
        saf.write_fails = true;
        let _ = write_via_saf(&saf, "content://d/9", b"next").expect_err("refused");
        assert!(save_without_backup_notice());
        clear_save_without_backup_notice();
    }

    #[test]
    fn read_target_refuses_an_absent_document_instead_of_returning_empty_bytes() {
        let saf = FakeSaf::default();
        let error = read_via_saf(&saf, "content://d/6").expect_err("absent is an error");
        assert!(error.contains("does not exist"), "{error}");
        let saf = FakeSaf::with_document("content://d/7", b"bytes");
        assert_eq!(read_via_saf(&saf, "content://d/7").unwrap(), b"bytes");
    }
}
