//! Contract tests for the Windows host file/platform seams.
//!
//! Split out of `platform.rs` to keep every code file under the repository's
//! 1000-line ceiling. No GUI: choosers are dependency-injected and the atomic
//! write runs against real temp directories, so these run on the development
//! host even though the crate's Win32 host layer is Windows-only.

use super::*;
use std::time::Duration;

fn temp_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "yacr-win-platform-test-{}-{name}",
        std::process::id()
    ))
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

/// A chooser that blocks until the test releases it, so a pending open can be
/// observed deterministically. Admission happens synchronously in
/// `begin_open`, so the request is already busy before the worker runs.
fn blocking_chooser() -> (FileChooser, std::sync::mpsc::Sender<()>) {
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    let release_rx = Mutex::new(release_rx);
    let chooser: FileChooser = Arc::new(move |_, _| {
        // A released or dropped sender both unblock; tests send exactly once.
        let _ = release_rx.lock().unwrap().recv();
        FileChooserOutcome::Cancelled
    });
    (chooser, release_tx)
}

#[test]
fn labels_come_from_the_i18n_catalogue() {
    let zh = WindowsOpenDialogLabels::from_locale("zh-CN").unwrap();
    let en = WindowsOpenDialogLabels::from_locale("en").unwrap();
    let other = WindowsOpenDialogLabels::from_locale("fr").unwrap();
    assert_ne!(zh.picker_title, en.picker_title);
    assert_eq!(other, en, "unknown locale defaults to English");
    assert!(zh.picker_title.contains('打') || zh.picker_title.contains('开'));
    assert!(en.picker_title.contains("Open"));
    assert!(zh.picker_failed("boom").contains("boom"));
}

#[test]
fn opened_target_reads_real_bytes() {
    let path = temp_path("opened.dwg");
    let payload = b"not-a-real-dwg-but-real-bytes";
    std::fs::write(&path, payload).unwrap();
    let files = files_with(Arc::new(|_, _| FileChooserOutcome::Cancelled));
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
    let files = files_with(Arc::new(|_, _| FileChooserOutcome::Cancelled));
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
    let files = files_with(Arc::new(|_, _| FileChooserOutcome::Cancelled));
    files.begin_open().unwrap();
    match wait_event(&files).as_slice() {
        [FileEvent::Cancelled { purpose }] => assert_eq!(*purpose, FilePurpose::Open),
        other => panic!("expected Cancelled, got {other:?}"),
    }
}

#[test]
fn failed_chooser_reports_its_localized_reason() {
    let files = files_with(Arc::new(|labels, _| {
        FileChooserOutcome::Failed(labels.picker_failed("dialog down"))
    }));
    files.begin_open().unwrap();
    match wait_event(&files).as_slice() {
        [FileEvent::Failed { reason, .. }] => assert!(reason.contains("dialog down")),
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
    let files = files_with(Arc::new(|_, _| FileChooserOutcome::Cancelled));

    files.begin_open_target(&target).unwrap();
    assert!(
        files.begin_open().is_err(),
        "a queued, unpolled result must still be single-flight"
    );

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
    let missing = temp_path("old-terminal.dwg");
    let _ = std::fs::remove_file(&missing);
    let target = FileTarget::from_path(&missing.to_string_lossy());
    let (chooser, release) = blocking_chooser();
    let files = files_with(chooser);
    files.begin_open_target(&target).unwrap();
    let _ = files.poll();

    files.begin_open().unwrap();
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
fn terminal_slot_capacity_is_bounded_to_one() {
    let missing = temp_path("bounded.dwg");
    let _ = std::fs::remove_file(&missing);
    let target = FileTarget::from_path(&missing.to_string_lossy());
    let files = files_with(Arc::new(|_, _| FileChooserOutcome::Cancelled));

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
fn set_source_is_bookkeeping_since_overwriting_is_allowed() {
    // ADR-0041 D4: no carrier refuses a save that names the opened source; the
    // previous bytes are protected by the atomic write's `.bak`. This carrier's
    // `set_source` therefore only records (and accepts clearing) the handle, and
    // never gates a write.
    let source = temp_path("source.dwg");
    let other = temp_path("other.dwg");
    std::fs::write(&source, b"a").unwrap();
    std::fs::write(&other, b"b").unwrap();
    let files = files_with(cancelled_open());
    files.set_source(Some(&FileTarget::from_path(&source.to_string_lossy())));
    files.set_source(Some(&FileTarget::from_path(&other.to_string_lossy())));
    files.set_source(None);
    let _ = std::fs::remove_file(&source);
    let _ = std::fs::remove_file(&other);
}

#[test]
fn surface_metrics_set_and_clear() {
    let surface = ReactSurface::new();
    assert_eq!(surface.surface(), None);
    let metrics = SurfaceMetrics {
        logical: [960.0, 439.0],
        origin: [0.0, 0.0],
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
    let absent = temp_path("absent-config.json");
    let _ = std::fs::remove_file(&absent);
    let store = ReactConfigStore::new(absent.clone(), temp_path("absent-pref.json"));
    assert_eq!(store.load_config(), ConfigLoad::Absent);

    // A directory is unreadable, reported as a diagnostic.
    let dir = temp_path("a-directory");
    std::fs::create_dir_all(&dir).unwrap();
    let store = ReactConfigStore::new(dir.clone(), dir.clone());
    assert!(matches!(store.load_config(), ConfigLoad::Unreadable { .. }));

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

/// The COM filter must offer every extension the dispatcher can read.
///
/// `.rfa`/`.rft` are listed on purpose (the importer refuses families by
/// name), and `.skp` is the SketchUp preview lane; this test asserts presence
/// only, never "supported".
#[test]
fn open_filter_spec_lists_the_importer_set() {
    let spec = open_filter_spec();
    let patterns: Vec<&str> = spec.split(';').collect();
    for extension in [
        "ymodel", "dwg", "dxf", "obj", "stl", "ply", "off", "coff", "step", "stp", "ifc", "rvt",
        "rte", "rfa", "rft", "skp",
    ] {
        let pattern = format!("*.{extension}");
        assert!(
            patterns.contains(&pattern.as_str()),
            "{pattern} missing from {spec}"
        );
    }
    assert!(spec.starts_with("*.ymodel;*.dwg;*.dxf;"), "{spec}");
    assert!(!spec.contains(" "), "{spec}");
}

fn cancelled_open() -> FileChooser {
    Arc::new(|_, _| FileChooserOutcome::Cancelled)
}

fn files_with_save(chooser: FileChooser, save_chooser: SaveChooser) -> ReactFiles {
    ReactFiles::with_choosers("zh-CN", chooser, save_chooser).expect("labels load")
}

/// What an injected save chooser saw: the purpose and suggested file name.
type SaveCall = Arc<Mutex<Option<(SavePurpose, String)>>>;

/// A save chooser that returns `path` and records the purpose + suggested name it
/// was called with, so the test can assert the prefill reached the dialog.
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

/// A fresh, empty directory under the system temp dir.
fn fresh_dir(name: &str) -> PathBuf {
    let dir = temp_path(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Temp files the atomic write may leave behind.
fn temp_leftovers(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with('.') && name.ends_with(".tmp"))
        .collect()
}

#[test]
fn save_ymodel_prefills_the_default_name_and_offers_only_ymodel() {
    let chosen = temp_path("prefilled.ymodel");
    let (chooser, seen) = recording_save_path(chosen.clone());
    let files = files_with_save(cancelled_open(), chooser);

    files
        .begin_save(SavePurpose::SaveYmodel, "plan.ymodel")
        .unwrap();
    match wait_event(&files).as_slice() {
        [FileEvent::SaveTarget { purpose, target }] => {
            assert_eq!(*purpose, SavePurpose::SaveYmodel);
            assert_eq!(target.opaque, chosen.to_string_lossy().into_owned());
        }
        other => panic!("expected SaveTarget, got {other:?}"),
    }
    assert_eq!(
        *seen.lock().unwrap(),
        Some((SavePurpose::SaveYmodel, "plan.ymodel".to_string())),
        "the suggested name reaches the chooser unchanged"
    );

    // SAVE offers exactly the native archive, and the shell appends `.ymodel`
    // when the user types a bare name.
    assert_eq!(save_filter_spec(SavePurpose::SaveYmodel), "*.ymodel");
    assert_eq!(
        save_default_extension(SavePurpose::SaveYmodel),
        Some("ymodel")
    );

    // EXPORT offers every writer extension in one filter and does not default an
    // extension (the extension *is* the writer choice).
    let export = save_filter_spec(SavePurpose::SaveExport);
    let patterns: Vec<&str> = export.split(';').collect();
    for extension in [
        "dxf", "dwg", "stl", "obj", "ply", "off", "coff", "ifc", "skp", "glb", "gltf",
    ] {
        let pattern = format!("*.{extension}");
        assert!(
            patterns.contains(&pattern.as_str()),
            "{pattern} missing from {export}"
        );
    }
    assert_eq!(save_default_extension(SavePurpose::SaveExport), None);

    // PLOT is vector output only.
    assert_eq!(save_filter_spec(SavePurpose::Plot), "*.svg;*.pdf");
    assert_eq!(save_default_extension(SavePurpose::Plot), Some("svg"));
}

#[test]
fn a_cancelled_save_reports_cancelled_and_writes_nothing() {
    let dir = fresh_dir("save-cancel-dir");
    let target = dir.join("never.ymodel");
    let chooser: SaveChooser = Arc::new(|_, _, _| FileChooserOutcome::Cancelled);
    let files = files_with_save(cancelled_open(), chooser);

    files
        .begin_save(SavePurpose::SaveYmodel, "plan.ymodel")
        .unwrap();
    match wait_event(&files).as_slice() {
        [FileEvent::Cancelled { purpose }] => {
            assert_eq!(*purpose, FilePurpose::Save(SavePurpose::SaveYmodel));
        }
        other => panic!("expected Cancelled, got {other:?}"),
    }
    assert!(!target.exists(), "a cancelled save writes no file");
    assert!(
        std::fs::read_dir(&dir).unwrap().next().is_none(),
        "a cancelled save leaves the directory empty"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_failed_save_reports_the_reason_and_writes_nothing() {
    let dir = fresh_dir("save-fail-dir");
    let target = dir.join("never.ymodel");
    let chooser: SaveChooser =
        Arc::new(|_, labels, _| FileChooserOutcome::Failed(labels.picker_failed("dialog down")));
    let files = files_with_save(cancelled_open(), chooser);

    files
        .begin_save(SavePurpose::SaveYmodel, "plan.ymodel")
        .unwrap();
    match wait_event(&files).as_slice() {
        [FileEvent::Failed { purpose, reason }] => {
            assert_eq!(*purpose, FilePurpose::Save(SavePurpose::SaveYmodel));
            assert!(reason.contains("dialog down"), "reason: {reason}");
        }
        other => panic!("expected Failed, got {other:?}"),
    }
    assert!(!target.exists(), "a failed chooser writes no file");
    assert!(std::fs::read_dir(&dir).unwrap().next().is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_save_without_a_chooser_fails_explicitly_instead_of_inventing_a_path() {
    let files = files_with(cancelled_open());
    files
        .begin_save(SavePurpose::SaveYmodel, "plan.ymodel")
        .unwrap();
    match wait_event(&files).as_slice() {
        [FileEvent::Failed { purpose, reason }] => {
            assert_eq!(*purpose, FilePurpose::Save(SavePurpose::SaveYmodel));
            assert!(!reason.is_empty());
        }
        other => panic!("expected Failed, got {other:?}"),
    }
}

#[test]
fn write_atomic_keeps_the_previous_bytes_in_a_bak_and_replaces_the_target() {
    let dir = fresh_dir("write-bak-dir");
    let target = dir.join("drawing.ymodel");
    let backup = dir.join("drawing.ymodel.bak");
    let files = files_with(cancelled_open());

    // A first save has nothing to back up, so it must not invent one.
    files
        .write_atomic(&FileTarget::from_path(&target.to_string_lossy()), b"v1")
        .unwrap();
    assert_eq!(std::fs::read(&target).unwrap(), b"v1");
    assert!(!backup.exists(), "no .bak for a file that did not exist");
    assert!(temp_leftovers(&dir).is_empty());

    // The second save keeps the previous bytes, byte for byte.
    files
        .write_atomic(&FileTarget::from_path(&target.to_string_lossy()), b"v2")
        .unwrap();
    assert_eq!(std::fs::read(&target).unwrap(), b"v2");
    assert_eq!(std::fs::read(&backup).unwrap(), b"v1");

    // A third save rotates the backup; the stale one must not survive, and the
    // target is replaced (not renamed away and recreated) with exact bytes.
    files
        .write_atomic(&FileTarget::from_path(&target.to_string_lossy()), b"v3")
        .unwrap();
    assert_eq!(std::fs::read(&target).unwrap(), b"v3");
    assert_eq!(std::fs::read(&backup).unwrap(), b"v2");
    assert!(
        temp_leftovers(&dir).is_empty(),
        "a save leaves no temp files behind, backup or not"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_failed_write_leaves_the_target_untouched() {
    let files = files_with(cancelled_open());

    // The target directory does not exist: the temp file cannot be created, so
    // the write fails explicitly and no target appears.
    let dir = temp_path("missing-write-dir");
    let _ = std::fs::remove_dir_all(&dir);
    let path = dir.join("out.dxf");
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

    // An existing target that cannot be replaced: the temp file is written and
    // synced, then the replace fails. The target's previous content (here: the
    // directory itself and its child) must be intact and the temp file removed.
    let dir = fresh_dir("unreplaceable-target-dir");
    let target = dir.join("occupied.dxf");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::write(target.join("keep.txt"), b"keep").unwrap();
    let error = files
        .write_atomic(&FileTarget::from_path(&target.to_string_lossy()), b"new")
        .expect_err("replacing a directory with a file must fail");
    assert!(matches!(error, CadError::InvalidInput(_)), "{error:?}");
    assert!(target.is_dir(), "the old target survives a failed write");
    assert_eq!(std::fs::read(target.join("keep.txt")).unwrap(), b"keep");
    assert!(
        temp_leftovers(&dir).is_empty(),
        "no partial temp file remains"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
