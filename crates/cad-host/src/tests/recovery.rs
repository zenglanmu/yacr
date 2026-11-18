//! Recovery-snapshot lifecycle contracts (ADR-0037 D12).
//!
//! The snapshot store is a carrier capability, so these contracts drive a
//! **fake** [`Persistence`] and a **fake** [`DocumentArchive`]: what is under
//! test is the host wiring — when a snapshot is written, when it is discarded,
//! when it is offered, and that a failure never becomes a command error or a
//! write to the document's own file.

use super::*;

use crate::RECOVERY_INTERVAL;
use cad_db::{FormatVersion, ManifestSummary};
use cad_domain::Revision;
use cad_platform::{
    ArchiveBytes, ArchiveError, ArchiveWrite, DocumentArchive, LoadedArchive, Persistence,
    RecoveryKey, RecoveryRecord, RecoverySnapshot,
};
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

const YMODEL_PATH: &str = "/tmp/plan.ymodel";
const RECOVERY_HINT: &str = "/home/user/.config/yacr/recovery/0123.ymodel";

/// What one `save_recovery` call saw.
#[derive(Debug, Clone)]
struct SavedSnapshot {
    key: String,
    revision: Revision,
    bytes: Vec<u8>,
    document_uuid: Option<String>,
}

/// A fake recovery store: records what the host asked for, answers `load` with a
/// canned snapshot.
#[derive(Default)]
struct FakePersistence {
    saved: Mutex<Vec<SavedSnapshot>>,
    discarded: Mutex<Vec<String>>,
    loaded: Mutex<Option<RecoverySnapshot>>,
    /// The key the last `load_recovery` was asked for.
    loaded_for: Mutex<Option<String>>,
    /// When set, every `save_recovery` refuses with this reason.
    save_error: Mutex<Option<String>>,
    hint: Option<String>,
}

impl FakePersistence {
    fn ready(loaded: Option<RecoverySnapshot>) -> Arc<FakePersistence> {
        Arc::new(FakePersistence {
            loaded: Mutex::new(loaded),
            hint: Some(RECOVERY_HINT.to_string()),
            ..FakePersistence::default()
        })
    }
}

impl Persistence for FakePersistence {
    fn save_recovery(&self, record: &RecoveryRecord, bytes: &[u8]) -> CadResult<()> {
        if let Some(reason) = self.save_error.lock().unwrap().clone() {
            return Err(CadError::Unsupported(reason));
        }
        self.saved.lock().unwrap().push(SavedSnapshot {
            key: record.key.clone(),
            revision: record.revision,
            bytes: bytes.to_vec(),
            document_uuid: record.document_uuid.clone(),
        });
        Ok(())
    }

    fn load_recovery(&self, key: &RecoveryKey) -> CadResult<Option<RecoverySnapshot>> {
        *self.loaded_for.lock().unwrap() = Some(key.as_str().to_string());
        Ok(self.loaded.lock().unwrap().clone())
    }

    fn recovery_hint(&self, _key: &RecoveryKey) -> Option<String> {
        self.hint.clone()
    }

    fn discard_recovery(&self, key: &RecoveryKey) -> CadResult<()> {
        self.discarded
            .lock()
            .unwrap()
            .push(key.as_str().to_string());
        Ok(())
    }
}

/// A fake carrier archive: one canned decode result and a configurable encoder.
struct SnapshotArchive {
    read: Result<LoadedArchive, ArchiveError>,
    fail_write: bool,
    writes: Mutex<Vec<Revision>>,
}

impl SnapshotArchive {
    fn new(loaded: LoadedArchive) -> Arc<SnapshotArchive> {
        Arc::new(SnapshotArchive {
            read: Ok(loaded),
            fail_write: false,
            writes: Mutex::new(Vec::new()),
        })
    }

    fn failing_write() -> Arc<SnapshotArchive> {
        Arc::new(SnapshotArchive {
            read: Err(ArchiveError::new("ymodel.invalid_document", "unused")),
            fail_write: true,
            writes: Mutex::new(Vec::new()),
        })
    }

    fn writes(&self) -> Vec<Revision> {
        self.writes.lock().unwrap().clone()
    }
}

impl DocumentArchive for SnapshotArchive {
    fn read(&self, _bytes: &[u8]) -> Result<LoadedArchive, ArchiveError> {
        self.read.clone()
    }

    fn write(&self, request: &ArchiveWrite<'_>) -> Result<ArchiveBytes, ArchiveError> {
        if self.fail_write {
            return Err(ArchiveError::new(
                "ymodel.invalid_document",
                "the encoder refused",
            ));
        }
        self.writes.lock().unwrap().push(request.snapshot.revision);
        Ok(ArchiveBytes {
            bytes: b"RECOVERY-ARCHIVE-BYTES".to_vec(),
            saved_revision: request.snapshot.revision,
        })
    }
}

/// A platform with a tiny in-memory disk and a synchronous save target.
#[derive(Default)]
struct RecoveryFiles {
    events: RefCell<Vec<FileEvent>>,
    disk: RefCell<BTreeMap<String, Vec<u8>>>,
    written: RefCell<Vec<String>>,
    source: RefCell<Option<FileTarget>>,
}

impl PlatformFiles for RecoveryFiles {
    fn begin_open(&self) -> CadResult<()> {
        Ok(())
    }
    fn begin_open_target(&self, _target: &FileTarget) -> CadResult<()> {
        Ok(())
    }
    fn begin_save(&self, purpose: SavePurpose, default_name: &str) -> CadResult<()> {
        let target = match self.source.borrow().as_ref() {
            Some(source) => source.clone(),
            None => FileTarget {
                display_name: default_name.to_string(),
                opaque: format!("/tmp/{default_name}"),
            },
        };
        self.events
            .borrow_mut()
            .push(FileEvent::SaveTarget { purpose, target });
        Ok(())
    }
    fn write_atomic(&self, target: &FileTarget, bytes: &[u8]) -> CadResult<()> {
        self.written.borrow_mut().push(target.display_name.clone());
        self.disk
            .borrow_mut()
            .insert(target.opaque.clone(), bytes.to_vec());
        Ok(())
    }
    fn refuses_source_overwrite(&self, _target: &FileTarget) -> bool {
        false
    }
    fn set_source(&self, target: Option<&FileTarget>) {
        *self.source.borrow_mut() = target.cloned();
    }
    fn read_target(&self, target: &FileTarget) -> Result<Vec<u8>, String> {
        self.disk
            .borrow()
            .get(&target.opaque)
            .cloned()
            .ok_or_else(|| "read-target: not found".to_string())
    }
    fn poll(&self) -> Vec<FileEvent> {
        std::mem::take(&mut *self.events.borrow_mut())
    }
}

fn target() -> FileTarget {
    FileTarget {
        display_name: "plan.ymodel".to_string(),
        opaque: YMODEL_PATH.to_string(),
    }
}

fn manifest(saved_revision: Revision) -> ManifestSummary {
    ManifestSummary {
        format: FormatVersion {
            read: 1,
            write: 1,
            minor: 0,
        },
        document_uuid: "doc-uuid-1".to_string(),
        open_revision: Revision(0),
        saved_revision,
        foreign_entries: Vec::new(),
    }
}

/// A real snapshot of a real one-line document, produced by the same
/// `snapshot_document` the save path uses, plus the manifest that would have
/// produced it (so the installed document is not dirty).
fn loaded_archive() -> (Revision, LoadedArchive) {
    let mut source =
        cad_app::host::HostController::with_blank_document([800.0, 600.0]).expect("blank session");
    source.create_line(p(0.0, 0.0), p(10.0, 0.0)).expect("line");
    let snapshot = source.snapshot_document().expect("snapshot");
    let revision = snapshot.revision;
    let manifest = manifest(revision);
    (revision, LoadedArchive { snapshot, manifest })
}

/// A core over the fake files, with a controllable monotonic clock.
fn recovery_core(files: Rc<RecoveryFiles>) -> (HostCore, Rc<Cell<u64>>) {
    let mut core = core_with_caps(files, capabilities(false));
    let now_ms = Rc::new(Cell::new(0_u64));
    let clock = now_ms.clone();
    let base = std::time::Instant::now();
    core.set_recovery_clock_for_test(Rc::new(move || {
        base + std::time::Duration::from_millis(clock.get())
    }));
    (core, now_ms)
}

/// The key the host derives for `opaque`, computed independently of
/// `HostCore::recovery_key`.
fn key_for(opaque: &str) -> String {
    RecoveryKey::from_hex(&crate::sha256_hex(opaque.as_bytes()))
        .expect("hex")
        .as_str()
        .to_string()
}

fn offer(record_revision: Revision) -> RecoverySnapshot {
    RecoverySnapshot {
        record: RecoveryRecord {
            // The fake store answers any key; the host must still ask for the
            // derived one (asserted separately).
            key: key_for(YMODEL_PATH),
            document_uuid: Some("doc-uuid-1".to_string()),
            revision: record_revision,
            display_name: Some("plan.ymodel".to_string()),
            saved_at_ms: 0,
        },
        bytes: b"RECOVERY-ARCHIVE-BYTES".to_vec(),
    }
}

#[test]
fn a_dirty_document_snapshots_once_per_interval() {
    let files = Rc::new(RecoveryFiles::default());
    let (core, now_ms) = recovery_core(files.clone());
    let archive = SnapshotArchive::new(loaded_archive().1);
    let store = FakePersistence::ready(None);
    core.set_document_archive(Some(archive.clone()));
    core.set_persistence(Some(store.clone()));

    // One real edit: the document is dirty at revision 1.
    core.controller
        .borrow_mut()
        .create_line(p(0.0, 5.0), p(5.0, 5.0))
        .expect("edit");
    let revision = core
        .controller
        .borrow()
        .drawing()
        .expect("drawing")
        .revision();
    assert!(core.controller.borrow().is_dirty());
    let key = core.recovery_key().expect("a dirty document has a key");

    // The first dirty tick only arms the interval.
    core.tick().expect("tick");
    assert!(
        store.saved.lock().unwrap().is_empty(),
        "no snapshot before the interval elapses"
    );

    now_ms.set(59_999);
    core.tick().expect("tick");
    assert!(
        store.saved.lock().unwrap().is_empty(),
        "59.999 s is still inside the interval"
    );

    now_ms.set(RECOVERY_INTERVAL.as_millis() as u64);
    core.tick().expect("tick");
    let saved = store.saved.lock().unwrap().clone();
    assert_eq!(saved.len(), 1, "exactly one snapshot at the deadline");
    assert_eq!(
        saved[0].revision, revision,
        "the snapshot is the current revision, from the save path's own encode"
    );
    assert_eq!(saved[0].key, key.as_str(), "filed under the document's key");
    assert_eq!(saved[0].bytes, b"RECOVERY-ARCHIVE-BYTES");
    assert_eq!(
        saved[0].document_uuid,
        core.controller
            .borrow()
            .persistence()
            .and_then(|state| state.document_uuid.clone()),
        "the snapshot records the document identity, not an invented one"
    );
    assert_eq!(
        archive.writes(),
        vec![revision],
        "the encode went through the carrier archive"
    );

    // A second tick inside the next interval writes nothing more.
    now_ms.set(RECOVERY_INTERVAL.as_millis() as u64 + 1);
    core.tick().expect("tick");
    assert_eq!(store.saved.lock().unwrap().len(), 1);
}

#[test]
fn a_clean_document_never_snapshots() {
    let files = Rc::new(RecoveryFiles::default());
    let (core, now_ms) = recovery_core(files);
    let archive = SnapshotArchive::new(loaded_archive().1);
    let store = FakePersistence::ready(None);
    core.set_document_archive(Some(archive.clone()));
    core.set_persistence(Some(store.clone()));
    assert!(!core.controller.borrow().is_dirty(), "nothing was edited");

    for millis in [0, 60_000, 600_000] {
        now_ms.set(millis);
        core.tick().expect("tick");
    }

    assert!(
        store.saved.lock().unwrap().is_empty(),
        "a clean document has nothing to recover"
    );
    assert!(archive.writes().is_empty(), "nothing is even encoded");
}

#[test]
fn a_successful_native_save_discards_the_snapshot() {
    let files = Rc::new(RecoveryFiles::default());
    let (core, _now_ms) = recovery_core(files.clone());
    let store = FakePersistence::ready(None);
    core.set_document_archive(Some(SnapshotArchive::new(loaded_archive().1)));
    core.set_persistence(Some(store.clone()));
    // The document was opened from this target, so the snapshot is filed under
    // the target's key. The fake disk holds the file we are saving over.
    files
        .disk
        .borrow_mut()
        .insert(YMODEL_PATH.to_string(), b"ORIGINAL-BYTES".to_vec());
    *core.source_target.borrow_mut() = Some(target());
    core.controller
        .borrow_mut()
        .create_line(p(0.0, 1.0), p(1.0, 1.0))
        .expect("edit");

    core.request_save()
        .expect("the save funnel grades a status");

    assert_eq!(
        files.written.borrow().len(),
        1,
        "the document's own file was written"
    );
    assert_eq!(
        store.discarded.lock().unwrap().clone(),
        vec![key_for(YMODEL_PATH)],
        "the snapshot for the current key is discarded after the write"
    );
    let status = core.last_status().expect("status");
    let (key, _) = catalog(&status);
    assert_eq!(
        key, "save.exported_ymodel",
        "a clean save keeps its ordinary status"
    );
}

#[test]
fn a_newer_snapshot_is_offered_when_the_document_installs() {
    let files = Rc::new(RecoveryFiles::default());
    let (core, _now_ms) = recovery_core(files);
    let (saved_revision, loaded) = loaded_archive();
    let newer = Revision(saved_revision.0 + 6);
    let store = FakePersistence::ready(Some(offer(newer)));
    core.set_document_archive(Some(SnapshotArchive::new(loaded)));
    // Installed before the open, exactly like the carrier bootstrap: the install
    // anchor itself must find the record.
    core.set_persistence(Some(store.clone()));

    core.deliver_external_open(target(), Arc::from(b"archive bytes".to_vec()))
        .expect("the archive installs");

    assert_eq!(
        store.loaded_for.lock().unwrap().as_deref(),
        Some(key_for(YMODEL_PATH).as_str()),
        "the record is looked up under the opened target's key"
    );
    let status = core.last_status().expect("a status");
    let (key, args) = catalog(&status);
    assert_eq!(key, "recovery.available");
    assert_eq!(
        args.get("path").map(String::as_str),
        Some(RECOVERY_HINT),
        "the status names the file to open"
    );
    let expected_revision = newer.0.to_string();
    assert_eq!(
        args.get("revision").map(String::as_str),
        Some(expected_revision.as_str())
    );
    assert!(
        store.discarded.lock().unwrap().is_empty(),
        "a newer snapshot is offered, never discarded"
    );
}

#[test]
fn a_stale_snapshot_is_discarded_instead_of_offered() {
    let files = Rc::new(RecoveryFiles::default());
    let (core, _now_ms) = recovery_core(files);
    let (saved_revision, loaded) = loaded_archive();
    // At the saved revision: equal is stale, not "recoverable" (D12 forbids
    // presenting anything that could roll the file back).
    let store = FakePersistence::ready(Some(offer(saved_revision)));
    core.set_document_archive(Some(SnapshotArchive::new(loaded)));
    core.set_persistence(Some(store.clone()));

    core.deliver_external_open(target(), Arc::from(b"archive bytes".to_vec()))
        .expect("the archive installs");

    assert_eq!(
        store.discarded.lock().unwrap().clone(),
        vec![key_for(YMODEL_PATH)],
        "a stale snapshot is discarded, not offered"
    );
    // The install's own status is untouched: a stale discard is silent.
    let install_status = core.last_status().expect("the open status");
    let (key, _) = catalog(&install_status);
    assert_ne!(key, "recovery.available");
    assert_ne!(key, "recovery.available_no_hint");
}

#[test]
fn without_a_recovery_store_nothing_is_written() {
    let files = Rc::new(RecoveryFiles::default());
    let (core, now_ms) = recovery_core(files.clone());
    let archive = SnapshotArchive::new(loaded_archive().1);
    core.set_document_archive(Some(archive.clone()));
    // No `set_persistence`: this carrier has no durable private directory.
    assert!(core.persistence_store().is_none());
    core.controller
        .borrow_mut()
        .create_line(p(0.0, 2.0), p(2.0, 2.0))
        .expect("edit");
    let status_before = core.last_status();

    now_ms.set(0);
    core.tick().expect("tick");
    now_ms.set(RECOVERY_INTERVAL.as_millis() as u64 * 2);
    core.tick().expect("tick");

    assert!(archive.writes().is_empty(), "nothing is encoded");
    assert_eq!(
        core.last_status(),
        status_before,
        "no per-tick status spam for a carrier without recovery"
    );
    // The open anchor is equally silent instead of failing.
    core.observe_recovery_on_install();
    assert_eq!(core.last_status(), status_before);
}

#[test]
fn a_failed_save_keeps_the_snapshot() {
    let files = Rc::new(RecoveryFiles::default());
    let (core, _now_ms) = recovery_core(files.clone());
    let store = FakePersistence::ready(None);
    // The encoder refuses: the save fails after the conflict check but before any
    // bytes reach the document's file.
    core.set_document_archive(Some(SnapshotArchive::failing_write()));
    core.set_persistence(Some(store.clone()));
    files
        .disk
        .borrow_mut()
        .insert(YMODEL_PATH.to_string(), b"ORIGINAL-BYTES".to_vec());
    *core.source_target.borrow_mut() = Some(target());
    core.controller
        .borrow_mut()
        .create_line(p(0.0, 3.0), p(3.0, 3.0))
        .expect("edit");

    core.request_save()
        .expect("the save funnel grades a status");

    assert!(files.written.borrow().is_empty(), "nothing was written");
    assert!(
        store.discarded.lock().unwrap().is_empty(),
        "a failed save must not discard the snapshot (that would delete the only copy)"
    );
    let status = core.last_status().expect("status");
    let (key, _) = catalog(&status);
    assert_eq!(key, "save.export_failed");
    assert!(
        core.controller.borrow().is_dirty(),
        "a refused save leaves the document dirty"
    );
}

#[test]
fn a_refusing_store_is_reported_once_per_document() {
    let files = Rc::new(RecoveryFiles::default());
    let (core, now_ms) = recovery_core(files);
    let archive = SnapshotArchive::new(loaded_archive().1);
    let store = FakePersistence::ready(None);
    *store.save_error.lock().unwrap() = Some("recovery.store.mkdir: denied".to_string());
    core.set_document_archive(Some(archive));
    core.set_persistence(Some(store));
    core.controller
        .borrow_mut()
        .create_line(p(0.0, 4.0), p(4.0, 4.0))
        .expect("edit");

    now_ms.set(0);
    core.tick().expect("tick");
    now_ms.set(RECOVERY_INTERVAL.as_millis() as u64);
    core.tick()
        .expect("a failed snapshot must not fail the tick");
    let warning = core.last_status().expect("an explicit warning");
    let (key, args) = catalog(&warning);
    assert_eq!(key, "recovery.failed");
    assert!(
        args.get("error")
            .is_some_and(|error| error.contains("recovery.store.mkdir")),
        "{args:?}"
    );

    // The next interval fails again but the document already reported it.
    core.set_catalog_status("command.completed", &[])
        .expect("status");
    now_ms.set(RECOVERY_INTERVAL.as_millis() as u64 * 2);
    core.tick().expect("tick");
    let status = core.last_status().expect("status");
    let (key, _) = catalog(&status);
    assert_eq!(
        key, "command.completed",
        "a latched failure does not rewrite the status every interval"
    );
}

/// `ui.recovery.intervalSeconds = 0` writes nothing even while dirty, and turns
/// "off" into a state that leaves no snapshot behind.
#[test]
fn a_zero_interval_disables_the_writes_and_discards_the_snapshot() {
    let files = Rc::new(RecoveryFiles::default());
    let (core, now_ms) = recovery_core(files);
    let archive = SnapshotArchive::new(loaded_archive().1);
    let store = FakePersistence::ready(None);
    core.set_document_archive(Some(archive.clone()));
    core.set_persistence(Some(store.clone()));
    core.controller
        .borrow_mut()
        .create_line(p(0.0, 6.0), p(6.0, 6.0))
        .expect("edit");
    assert!(core.controller.borrow().is_dirty());
    let key = core.recovery_key().expect("a dirty document has a key");

    // Turning the snapshot writer off also discards the current snapshot: a
    // stale file would otherwise still be offered at the next open.
    core.apply_setting(CadSettingKey::RecoveryInterval, CadSettingValue::Number(0))
        .unwrap();
    assert_eq!(
        store.discarded.lock().unwrap().clone(),
        vec![key.as_str().to_string()],
        "\"off\" leaves nothing behind"
    );

    for millis in [0, 60_000, 600_000] {
        now_ms.set(millis);
        core.tick().expect("tick");
    }
    assert!(
        store.saved.lock().unwrap().is_empty(),
        "a zero interval writes no snapshot even while dirty"
    );
    assert!(
        archive.writes().is_empty(),
        "nothing is even encoded while the writer is off"
    );
}

/// A changed interval is honoured on the next tick: one second writes a second
/// after the arming tick, and a later 120 s setting moves the deadline.
#[test]
fn a_changed_interval_is_honoured_per_tick() {
    let files = Rc::new(RecoveryFiles::default());
    let (core, now_ms) = recovery_core(files);
    let archive = SnapshotArchive::new(loaded_archive().1);
    let store = FakePersistence::ready(None);
    core.set_document_archive(Some(archive.clone()));
    core.set_persistence(Some(store.clone()));
    core.apply_setting(CadSettingKey::RecoveryInterval, CadSettingValue::Number(1))
        .unwrap();
    core.controller
        .borrow_mut()
        .create_line(p(0.0, 7.0), p(7.0, 7.0))
        .expect("edit");

    // The first dirty tick arms the 1 s interval.
    now_ms.set(0);
    core.tick().expect("tick");
    assert!(store.saved.lock().unwrap().is_empty());
    now_ms.set(999);
    core.tick().expect("tick");
    assert!(
        store.saved.lock().unwrap().is_empty(),
        "999 ms is still inside a 1 s interval"
    );
    now_ms.set(1_000);
    core.tick().expect("tick");
    assert_eq!(
        store.saved.lock().unwrap().len(),
        1,
        "a 1 s interval writes at the 1 s deadline, not at the 60 s default"
    );

    // A later, longer interval is read per tick and moves the next deadline.
    core.apply_setting(
        CadSettingKey::RecoveryInterval,
        CadSettingValue::Number(120),
    )
    .unwrap();
    now_ms.set(1_001);
    core.tick().expect("tick");
    now_ms.set(60_000);
    core.tick().expect("tick");
    assert_eq!(
        store.saved.lock().unwrap().len(),
        1,
        "60 s after the last attempt is inside the new 120 s interval"
    );
    now_ms.set(121_000);
    core.tick().expect("tick");
    assert_eq!(
        store.saved.lock().unwrap().len(),
        2,
        "the 120 s interval really is honoured"
    );
}
