//! Contract tests for the React Linux host platform contracts.
//!
//! Split out of `platform.rs` to keep every code file under the repository's
//! 1000-line ceiling. No GUI: choosers are dependency-injected and the atomic
//! write runs against real temp directories.

use super::*;
use std::time::Duration;

fn temp_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("yacr-platform-test-{}-{name}", std::process::id()))
}

/// Poll until at least one event arrives (bounded), never unbounded.
fn wait_event(files: &ReactFiles) -> Vec<FileEvent> {
    for _ in 0..300 {
        let events = files.poll();
        if !events.is_empty() {
            return events;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("no file event within the bounded wait");
}

fn files_with(chooser: FileChooser) -> ReactFiles {
    ReactFiles::with_chooser("zh-CN", chooser).expect("labels load")
}

fn files_with_save(chooser: FileChooser, save_chooser: SaveChooser) -> ReactFiles {
    ReactFiles::with_choosers("zh-CN", chooser, save_chooser).expect("labels load")
}

fn cancelled_open() -> FileChooser {
    Arc::new(|_| FileChooserOutcome::Cancelled)
}

/// What an injected save chooser saw: the purpose and suggested file name.
type SaveCall = Arc<Mutex<Option<(SavePurpose, String)>>>;

/// A save chooser that returns `path` and records the purpose + suggested name it
/// was called with, so the test can assert the default name reached the dialog.
fn recording_save_path(path: PathBuf) -> (SaveChooser, SaveCall) {
    let seen = Arc::new(Mutex::new(None));
    let recorded = seen.clone();
    let chosen = path.clone();
    let chooser: SaveChooser = Arc::new(move |purpose, _, default_name| {
        *recorded.lock().unwrap() = Some((purpose, default_name.to_string()));
        FileChooserOutcome::Path(chosen.clone())
    });
    (chooser, seen)
}

/// A chooser that blocks until the test releases it, so a pending open can be
/// observed deterministically. Admission happens synchronously in
/// `begin_open`, so the request is already busy before the worker runs.
fn blocking_chooser() -> (FileChooser, std::sync::mpsc::Sender<()>) {
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    let release_rx = Mutex::new(release_rx);
    let chooser: FileChooser = Arc::new(move |_| {
        // A released or dropped sender both unblock; tests send exactly once.
        let _ = release_rx.lock().unwrap().recv();
        FileChooserOutcome::Cancelled
    });
    (chooser, release_tx)
}

/// The save counterpart of [`blocking_chooser`]: keeps a save in flight so the
/// shared single-flight admission is observable.
fn blocking_save_chooser() -> (SaveChooser, std::sync::mpsc::Sender<()>) {
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    let release_rx = Mutex::new(release_rx);
    let chooser: SaveChooser = Arc::new(move |_, _, _| {
        let _ = release_rx.lock().unwrap().recv();
        FileChooserOutcome::Cancelled
    });
    (chooser, release_tx)
}

#[test]
fn labels_come_from_the_i18n_catalogue() {
    let zh = UseOpenDialogLabels::from_locale("zh-CN").unwrap();
    let en = UseOpenDialogLabels::from_locale("en").unwrap();
    let other = UseOpenDialogLabels::from_locale("fr").unwrap();
    assert_ne!(zh.picker_title, en.picker_title);
    assert_eq!(other, en, "unknown locale defaults to English");
    assert!(zh.picker_title.contains('打') || zh.picker_title.contains('开'));
    assert!(en.picker_title.contains("Open"));
    assert!(zh.picker_failed("boom").contains("boom"));
    assert!(!zh.open_path.is_empty());
}

#[test]
fn save_labels_come_from_the_i18n_catalogue() {
    let zh = UseSaveDialogLabels::from_locale("zh-CN").unwrap();
    let en = UseSaveDialogLabels::from_locale("en").unwrap();
    assert_eq!(UseSaveDialogLabels::from_locale("fr").unwrap(), en);
    // Both purposes have their own localized title and filter, and no key fell
    // back to an empty string.
    for labels in [&zh, &en] {
        for purpose in [SavePurpose::SaveDxf, SavePurpose::Plot] {
            assert!(!labels.title(purpose).is_empty());
            assert!(!labels.filter(purpose).is_empty());
            assert!(!labels.picker_empty.is_empty());
            assert!(!labels.picker_local.is_empty());
        }
        assert_ne!(
            labels.title(SavePurpose::SaveDxf),
            labels.title(SavePurpose::Plot)
        );
        assert_ne!(
            labels.filter(SavePurpose::SaveDxf),
            labels.filter(SavePurpose::Plot)
        );
    }
    assert_ne!(zh.title_dxf, en.title_dxf);
    assert_eq!(en.title_dxf, "Save as DXF");
    assert!(zh.picker_failed("boom").contains("boom"));
}

#[test]
fn opened_target_reads_real_bytes() {
    let path = temp_path("opened.dwg");
    let payload = b"not-a-real-dwg-but-real-bytes";
    std::fs::write(&path, payload).unwrap();
    let files = files_with(cancelled_open());
    files
        .begin_open_target(&FileTarget::from_path(&path.to_string_lossy()))
        .unwrap();
    let events = wait_event(&files);
    let _ = std::fs::remove_file(&path);
    match events.as_slice() {
        [FileEvent::Opened { target, bytes }] => {
            assert_eq!(&bytes[..], payload);
            assert_eq!(target.opaque, path.to_string_lossy().into_owned());
        }
        other => panic!("expected Opened, got {other:?}"),
    }
}

#[test]
fn missing_target_reports_an_error_event() {
    let path = temp_path("missing.dwg");
    let _ = std::fs::remove_file(&path);
    let files = files_with(cancelled_open());
    files
        .begin_open_target(&FileTarget::from_path(&path.to_string_lossy()))
        .unwrap();
    match files.poll().as_slice() {
        [FileEvent::Failed { purpose, reason }] => {
            assert_eq!(*purpose, FilePurpose::Open);
            assert!(reason.contains("does not exist"), "reason: {reason}");
        }
        other => panic!("expected Failed, got {other:?}"),
    }
}

#[test]
fn cancelled_chooser_is_a_cancel_not_a_success() {
    let files = files_with(cancelled_open());
    files.begin_open().unwrap();
    match wait_event(&files).as_slice() {
        [FileEvent::Cancelled { purpose }] => assert_eq!(*purpose, FilePurpose::Open),
        other => panic!("expected Cancelled, got {other:?}"),
    }
}

#[test]
fn failed_chooser_reports_its_localized_reason() {
    let files = files_with(Arc::new(|labels| {
        FileChooserOutcome::Failed(labels.picker_failed("portal down"))
    }));
    files.begin_open().unwrap();
    match wait_event(&files).as_slice() {
        [FileEvent::Failed { reason, .. }] => assert!(reason.contains("portal down")),
        other => panic!("expected Failed, got {other:?}"),
    }
}

#[test]
fn second_open_is_rejected_until_the_queued_result_is_polled() {
    // A missing target is an accepted request whose terminal event is queued
    // synchronously, so "busy but result queued" is observable without a race.
    let missing = temp_path("queued-missing.dwg");
    let _ = std::fs::remove_file(&missing);
    let target = FileTarget::from_path(&missing.to_string_lossy());
    let files = files_with(cancelled_open());

    files.begin_open_target(&target).unwrap();
    // The terminal event is queued but not polled: the request is still busy.
    assert!(
        files.begin_open().is_err(),
        "a queued, unpolled result must still be single-flight"
    );
    assert!(files.begin_open_target(&target).is_err());

    // Draining the accepted terminal releases the single flight.
    match files.poll().as_slice() {
        [FileEvent::Failed { purpose, .. }] => assert_eq!(*purpose, FilePurpose::Open),
        other => panic!("expected Failed, got {other:?}"),
    }
    files.begin_open_target(&target).unwrap();
    let _ = files.poll();
}

#[test]
fn consumed_terminal_does_not_unlock_a_new_pending_open() {
    // Consume an old terminal first.
    let missing = temp_path("old-terminal.dwg");
    let _ = std::fs::remove_file(&missing);
    let target = FileTarget::from_path(&missing.to_string_lossy());
    let (chooser, release) = blocking_chooser();
    let files = files_with(chooser);
    files.begin_open_target(&target).unwrap();
    let _ = files.poll();

    // A new request is admitted and its worker is still running.
    files.begin_open().unwrap();
    // An empty poll must not reset `busy`: the old terminal cannot unlock it.
    assert!(files.poll().is_empty());
    assert!(
        files.begin_open().is_err(),
        "an empty poll must not unlock the new pending open"
    );

    release.send(()).unwrap();
    match wait_event(&files).as_slice() {
        [FileEvent::Cancelled { .. }] => {}
        other => panic!("expected Cancelled, got {other:?}"),
    }
}

#[test]
fn missing_target_is_rejected_while_an_open_is_pending() {
    let (chooser, release) = blocking_chooser();
    let files = files_with(chooser);
    files.begin_open().unwrap();

    let missing = temp_path("missing-during-pending.dwg");
    let _ = std::fs::remove_file(&missing);
    let target = FileTarget::from_path(&missing.to_string_lossy());
    assert!(
        files.begin_open_target(&target).is_err(),
        "a pending open must reject a new target, missing or not"
    );

    release.send(()).unwrap();
    // The rejected call queued nothing: only the pending open's terminal arrives.
    match wait_event(&files).as_slice() {
        [FileEvent::Cancelled { purpose }] => assert_eq!(*purpose, FilePurpose::Open),
        other => panic!("expected the pending open's Cancelled, got {other:?}"),
    }
}

#[test]
fn terminal_slot_capacity_is_bounded_to_one() {
    let missing = temp_path("bounded.dwg");
    let _ = std::fs::remove_file(&missing);
    let target = FileTarget::from_path(&missing.to_string_lossy());
    let files = files_with(cancelled_open());

    files.begin_open_target(&target).unwrap();
    for _ in 0..16 {
        assert!(
            files.begin_open_target(&target).is_err(),
            "extra requests must be rejected, never queued"
        );
    }
    assert_eq!(files.poll().len(), 1, "capacity 1: no unbounded queue");
    assert!(files.poll().is_empty(), "the slot is drained exactly once");
}

#[test]
fn source_guard_uses_filesystem_identity() {
    let source = temp_path("source.dwg");
    let other = temp_path("other.dwg");
    std::fs::write(&source, b"a").unwrap();
    std::fs::write(&other, b"b").unwrap();
    let files = files_with(cancelled_open());
    assert!(!files.refuses_source_overwrite(&FileTarget::from_path(&source.to_string_lossy())));
    files.set_source(Some(&FileTarget::from_path(&source.to_string_lossy())));
    assert!(files.refuses_source_overwrite(&FileTarget::from_path(&source.to_string_lossy())));
    assert!(!files.refuses_source_overwrite(&FileTarget::from_path(&other.to_string_lossy())));
    files.set_source(None);
    assert!(!files.refuses_source_overwrite(&FileTarget::from_path(&source.to_string_lossy())));
    let _ = std::fs::remove_file(&source);
    let _ = std::fs::remove_file(&other);
}

#[test]
fn dxf_save_chooser_returns_the_chosen_target() {
    let path = temp_path("chosen.dxf");
    let _ = std::fs::remove_file(&path);
    let (chooser, seen) = recording_save_path(path.clone());
    let files = files_with_save(cancelled_open(), chooser);

    files.begin_save(SavePurpose::SaveDxf, "draft.dxf").unwrap();
    let events = wait_event(&files);
    match events.as_slice() {
        [FileEvent::SaveTarget { purpose, target }] => {
            assert_eq!(*purpose, SavePurpose::SaveDxf);
            assert_eq!(target.opaque, path.to_string_lossy().into_owned());
            assert_eq!(target.display_name, path.to_string_lossy().into_owned());
        }
        other => panic!("expected SaveTarget, got {other:?}"),
    }
    // The suggested name reached the dialog and the worker thread got its name.
    assert_eq!(
        *seen.lock().unwrap(),
        Some((SavePurpose::SaveDxf, "draft.dxf".to_string()))
    );
}

#[test]
fn plot_save_chooser_returns_the_chosen_target() {
    let path = temp_path("chosen.svg");
    let _ = std::fs::remove_file(&path);
    let (chooser, seen) = recording_save_path(path.clone());
    let files = files_with_save(cancelled_open(), chooser);

    files.begin_save(SavePurpose::Plot, "out.svg").unwrap();
    match wait_event(&files).as_slice() {
        [FileEvent::SaveTarget { purpose, target }] => {
            assert_eq!(*purpose, SavePurpose::Plot);
            assert_eq!(target.opaque, path.to_string_lossy().into_owned());
        }
        other => panic!("expected SaveTarget, got {other:?}"),
    }
    assert_eq!(
        *seen.lock().unwrap(),
        Some((SavePurpose::Plot, "out.svg".to_string()))
    );
}

#[test]
fn cancelled_save_is_a_cancel_with_the_save_purpose() {
    let files = files_with_save(
        cancelled_open(),
        Arc::new(|_, _, _| FileChooserOutcome::Cancelled),
    );
    files.begin_save(SavePurpose::Plot, "out.svg").unwrap();
    match wait_event(&files).as_slice() {
        [FileEvent::Cancelled { purpose }] => {
            assert_eq!(*purpose, FilePurpose::Save(SavePurpose::Plot));
        }
        other => panic!("expected Cancelled, got {other:?}"),
    }
}

#[test]
fn failed_save_reports_the_chooser_reason() {
    let files = files_with_save(
        cancelled_open(),
        Arc::new(|_, _, _| FileChooserOutcome::Failed("portal down".into())),
    );
    files.begin_save(SavePurpose::SaveDxf, "draft.dxf").unwrap();
    match wait_event(&files).as_slice() {
        [FileEvent::Failed { purpose, reason }] => {
            assert_eq!(*purpose, FilePurpose::Save(SavePurpose::SaveDxf));
            assert!(reason.contains("portal down"), "reason: {reason}");
        }
        other => panic!("expected Failed, got {other:?}"),
    }
}

#[test]
fn save_without_an_injected_chooser_fails_explicitly() {
    // `with_chooser` wires only the open chooser: a save must publish a failure,
    // never a fabricated target.
    let files = files_with(cancelled_open());
    files.begin_save(SavePurpose::Plot, "out.svg").unwrap();
    match wait_event(&files).as_slice() {
        [FileEvent::Failed { purpose, reason }] => {
            assert_eq!(*purpose, FilePurpose::Save(SavePurpose::Plot));
            assert!(!reason.is_empty());
            assert!(
                reason.contains("no save chooser is configured"),
                "reason: {reason}"
            );
        }
        other => panic!("expected Failed, got {other:?}"),
    }
}

#[test]
fn second_save_is_rejected_while_one_is_in_flight() {
    let (save_chooser, release) = blocking_save_chooser();
    let files = files_with_save(cancelled_open(), save_chooser);

    files.begin_save(SavePurpose::SaveDxf, "draft.dxf").unwrap();
    match files.begin_save(SavePurpose::Plot, "out.svg") {
        Err(CadError::InvalidInput(_)) => {}
        other => panic!("expected InvalidInput, got {other:?}"),
    }

    release.send(()).unwrap();
    match wait_event(&files).as_slice() {
        [FileEvent::Cancelled { purpose }] => {
            assert_eq!(*purpose, FilePurpose::Save(SavePurpose::SaveDxf));
        }
        other => panic!("expected the pending save's Cancelled, got {other:?}"),
    }
}

#[test]
fn open_is_rejected_while_a_save_is_in_flight() {
    let (save_chooser, release) = blocking_save_chooser();
    let files = files_with_save(cancelled_open(), save_chooser);

    files.begin_save(SavePurpose::Plot, "out.svg").unwrap();
    match files.begin_open() {
        Err(CadError::InvalidInput(_)) => {}
        other => panic!("expected InvalidInput, got {other:?}"),
    }
    let missing = temp_path("open-during-save.dwg");
    let _ = std::fs::remove_file(&missing);
    assert!(files
        .begin_open_target(&FileTarget::from_path(&missing.to_string_lossy()))
        .is_err());

    release.send(()).unwrap();
    match wait_event(&files).as_slice() {
        [FileEvent::Cancelled { purpose }] => {
            assert_eq!(*purpose, FilePurpose::Save(SavePurpose::Plot));
        }
        other => panic!("expected the pending save's Cancelled, got {other:?}"),
    }
}

/// No `.tmp` sibling may survive a successful atomic write.
fn temp_leftovers(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .expect("read the write directory")
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.contains(".tmp"))
        .collect()
}

#[test]
fn write_atomic_writes_exact_bytes_and_leaves_no_temp_file() {
    let dir = temp_path("write-dir");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let files = files_with(cancelled_open());

    // Over a non-existent file.
    let fresh = dir.join("fresh.dxf");
    files
        .write_atomic(
            &FileTarget::from_path(&fresh.to_string_lossy()),
            b"fresh-bytes",
        )
        .unwrap();
    assert_eq!(std::fs::read(&fresh).unwrap(), b"fresh-bytes");
    assert!(temp_leftovers(&dir).is_empty());

    // Over an existing (longer) file: exact replacement, no truncation residue.
    let existing = dir.join("existing.dxf");
    std::fs::write(&existing, b"old-and-much-longer").unwrap();
    files
        .write_atomic(&FileTarget::from_path(&existing.to_string_lossy()), b"new")
        .unwrap();
    assert_eq!(std::fs::read(&existing).unwrap(), b"new");
    assert!(temp_leftovers(&dir).is_empty());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn write_atomic_into_a_missing_directory_is_an_explicit_error() {
    let dir = temp_path("missing-write-dir");
    let _ = std::fs::remove_dir_all(&dir);
    let path = dir.join("out.dxf");
    let files = files_with(cancelled_open());

    match files.write_atomic(&FileTarget::from_path(&path.to_string_lossy()), b"x") {
        Err(CadError::InvalidInput(message)) => {
            assert!(
                message.contains(&dir.to_string_lossy().into_owned()),
                "the error must name the path: {message}"
            );
        }
        other => panic!("expected InvalidInput, got {other:?}"),
    }
    assert!(!path.exists(), "a failed write must not create the target");
}

/// The source-overwrite guard is a separate query the host enforces before the
/// write; [`PlatformFiles::write_atomic`] itself never refuses a target.
#[test]
fn write_atomic_does_not_enforce_the_source_guard_itself() {
    let dir = temp_path("source-guard-write-dir");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let source = dir.join("source.dwg");
    std::fs::write(&source, b"original").unwrap();
    let files = files_with(cancelled_open());
    files.set_source(Some(&FileTarget::from_path(&source.to_string_lossy())));

    assert!(files.refuses_source_overwrite(&FileTarget::from_path(&source.to_string_lossy())));
    files
        .write_atomic(
            &FileTarget::from_path(&source.to_string_lossy()),
            b"replaced",
        )
        .unwrap();
    assert_eq!(std::fs::read(&source).unwrap(), b"replaced");

    // The guard itself is unchanged by the write.
    assert!(files.refuses_source_overwrite(&FileTarget::from_path(&source.to_string_lossy())));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn surface_metrics_set_and_clear() {
    let surface = ReactSurface::new();
    assert_eq!(surface.surface(), None);
    let metrics = SurfaceMetrics {
        logical: [960.0, 439.0],
        scale: 1.0,
        safe_insets: [0.0; 4],
    };
    surface.set_metrics(metrics);
    assert_eq!(surface.surface(), Some(metrics));
    surface.clear();
    assert_eq!(surface.surface(), None);
}

#[test]
fn config_absent_unreadable_and_persist_roundtrip() {
    // Absent path is normal, not a failure.
    let absent = temp_path("absent-config.json");
    let _ = std::fs::remove_file(&absent);
    let store = ReactConfigStore::new(absent.clone(), temp_path("absent-pref.json"));
    assert_eq!(store.load_config(), ConfigLoad::Absent);

    // A directory is unreadable, reported as a diagnostic.
    let dir = temp_path("a-directory");
    std::fs::create_dir_all(&dir).unwrap();
    let store = ReactConfigStore::new(dir.clone(), dir.clone());
    assert!(matches!(store.load_config(), ConfigLoad::Unreadable { .. }));
    assert!(matches!(
        store.load_preference(),
        ConfigLoad::Unreadable { .. }
    ));

    // A real path persists atomically and reads back.
    let pref = temp_path("pref.json");
    let store = ReactConfigStore::new(absent, pref.clone());
    store.persist_preference("{\"k\":1}").unwrap();
    match store.load_preference() {
        ConfigLoad::Text { text, .. } => assert_eq!(text, "{\"k\":1}"),
        other => panic!("expected Text, got {other:?}"),
    }
    let _ = std::fs::remove_file(&pref);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn non_persistent_store_refuses_persistence_explicitly() {
    let store = ReactConfigStore::non_persistent();
    assert_eq!(store.load_config(), ConfigLoad::Absent);
    assert!(matches!(
        store.persist_preference("{}"),
        Err(CadError::Unsupported(_))
    ));
}

/// The renderer backend preference is read through the same store every
/// other user preference uses, and every failure mode is a silent `None`
/// (the presenter then uses `auto`) rather than an invented value.
#[test]
fn wgpu_backend_preference_reads_the_durable_store() {
    let absent = temp_path("absent-wgpu-pref.json");
    let _ = std::fs::remove_file(&absent);
    let store = ReactConfigStore::new(temp_path("unused-config.json"), absent);
    assert_eq!(wgpu_backend_preference(&store), None);

    let preferences = [
        (r#"{"wgpu_backend":"dx12"}"#, Some("dx12")),
        (r#"{"wgpu_backend":"vulkan"}"#, Some("vulkan")),
        (r#"{"wgpu_backend":"auto"}"#, Some("auto")),
        // A nested projection is tolerated, the root key is canonical.
        (r#"{"ui":{"wgpu_backend":"dx12"}}"#, Some("dx12")),
        // Unknown tokens are returned verbatim; the renderer resolves them
        // to `auto` (one parser owns the wire contract).
        (r#"{"wgpu_backend":"gl"}"#, Some("gl")),
        // Absent key, non-string value and malformed JSON are all `None`.
        (r#"{"interaction":{"snapEnabled":true}}"#, None),
        (r#"{"wgpu_backend":1}"#, None),
        (r#"not json"#, None),
    ];
    for (text, expected) in preferences {
        assert_eq!(
            backend_preference_from_json(text),
            expected.map(str::to_string),
            "{text} must extract {expected:?}"
        );
    }

    // End-to-end through the durable store (the path the host reads).
    let path = temp_path("wgpu-pref.json");
    let store = ReactConfigStore::new(temp_path("unused-config.json"), path.clone());
    store
        .persist_preference(r#"{"wgpu_backend":"dx12"}"#)
        .unwrap();
    assert_eq!(wgpu_backend_preference(&store), Some("dx12".to_string()));
    let _ = std::fs::remove_file(&path);
}
