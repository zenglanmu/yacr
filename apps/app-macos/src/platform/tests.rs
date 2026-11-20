use super::*;
use std::time::Duration;

fn temp_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "yacr-macos-platform-test-{}-{name}",
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
    let zh = MacosOpenDialogLabels::from_locale("zh-CN").unwrap();
    let en = MacosOpenDialogLabels::from_locale("en").unwrap();
    let other = MacosOpenDialogLabels::from_locale("fr").unwrap();
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
    // ADR-0041 D4: no carrier refuses a save that names the opened source;
    // the previous bytes are protected by the platform's atomic write. This
    // carrier's `set_source` therefore only records (and accepts clearing)
    // the handle, and never gates a write.
    let source = temp_path("source.dwg");
    std::fs::write(&source, b"a").unwrap();
    let files = files_with(Arc::new(|_, _| FileChooserOutcome::Cancelled));
    files.set_source(Some(&FileTarget::from_path(&source.to_string_lossy())));
    files.set_source(None);
    let _ = std::fs::remove_file(&source);
}

/// The carrier's save plumbing exists (see `platform/save.rs` for the save
/// funnel); a `.ymodel` open is the native archive lane, never a connector
/// probe, and the carrier delivers the bytes unchanged.
#[test]
fn ymodel_open_is_the_archive_lane_not_a_connector() {
    let path = temp_path("plan.ymodel");
    let payload = b"PK\x03\x04 not-a-real-archive-but-real-bytes";
    std::fs::write(&path, payload).unwrap();
    let target = FileTarget::from_path(&path.to_string_lossy());
    // The shared host routes by this predicate before any importer probe.
    assert!(cad_host::policy::is_ymodel_target(&target));
    assert!(!cad_host::policy::is_ymodel_target(&FileTarget::from_path(
        "/tmp/plan.dxf"
    )));

    let files = files_with(Arc::new(|_, _| FileChooserOutcome::Cancelled));
    files.begin_open_target(&target).unwrap();
    match wait_event(&files).as_slice() {
        [FileEvent::Opened {
            target: opened,
            bytes,
        }] => {
            assert_eq!(opened, &target, "the archive lane keeps the real handle");
            assert_eq!(&bytes[..], payload, "bytes are handed over unchanged");
        }
        other => panic!("expected Opened, got {other:?}"),
    }
    let _ = std::fs::remove_file(&path);
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
        (r#"{"wgpu_backend":"metal"}"#, Some("metal")),
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

/// `NSOpenPanel` must offer every extension the dispatcher can read.
///
/// `.rfa`/`.rft` are listed on purpose (the importer refuses families by
/// name), and `.skp` is the SketchUp preview lane; this test asserts presence
/// only, never "supported".
#[test]
fn allowed_file_types_cover_the_importer_set() {
    for extension in [
        "ymodel", "dwg", "dxf", "obj", "stl", "ply", "off", "coff", "step", "stp", "ifc", "rvt",
        "rte", "rfa", "rft", "skp", "dgn",
    ] {
        assert!(
            OPEN_EXTENSIONS.contains(&extension),
            "{extension} must be selectable"
        );
    }
    assert_eq!(&OPEN_EXTENSIONS[..2], &["ymodel", "dwg"]);
}
