//! Host-target contract tests for the browser platform adapters and the
//! bytes-based session entry.
//!
//! No GPU, no browser, no DOM: the session is built from the compiled-in
//! synthetic fixture's bytes with a `RenderSessionHandle` that has **no device
//! attached** (`RenderSessionHandle::new` only creates the owner-thread state; a
//! renderer is constructed on `attach_device`, which these tests never call). That
//! is enough to exercise the whole CAD half of the session — import, document,
//! command router, status, tick — which is exactly what phase 1 has to prove.

use std::cell::Cell;
use std::rc::Rc;

use cad_domain::CadError;
use cad_engine::render::{BackendPreference, RenderSessionHandle};
use cad_host::{HostOptions, RequestClock};
use cad_platform::{
    ConfigLoad, ConfigStore, FileEvent, FileTarget, HostCapabilities, PlatformFiles, SavePurpose,
    SurfaceSource,
};
use cad_ui_core::session::{BytesDocument, CadSession};

use super::*;
use crate::download::{BrowserFiles, DownloadRecord, RecordingSink};

/// The compiled-in synthetic fixture: four lines, **no text and no image**.
const SYNTHETIC: &[u8] = include_bytes!("../../../../fixtures/dwg/synthetic-four-lines.dwg");
const SYNTHETIC_LABEL: &str = "synthetic-four-lines.dwg";

/// A monotonic millisecond clock (the host-target stand-in for
/// `performance.now()`). Strictly increasing, so the pump's non-decreasing
/// validation is exercised for real rather than passing on a constant.
fn monotonic_clock() -> RequestClock {
    let millis = Cell::new(0.0_f64);
    Rc::new(move || {
        let now = millis.get();
        millis.set(now + 0.05);
        now
    })
}

fn options() -> HostOptions {
    HostOptions {
        fonts: Vec::new(),
        fonts_dir: None,
        images_dir: None,
        headless: false,
        // The document is the bytes; naming a path here would be a second source.
        drawing: None,
        size: [800.0, 600.0],
    }
}

/// Open the synthetic fixture through the browser adapters. `capabilities` is
/// what the carrier advertises, so a test can prove the projection is real.
fn open(capabilities: HostCapabilities) -> (BrowserPlatform, CadSession) {
    let platform = BrowserPlatform::new();
    let session = CadSession::from_bytes(
        options(),
        platform.host_platform(capabilities),
        RenderSessionHandle::new(BackendPreference::WebGl2),
        monotonic_clock(),
        BytesDocument {
            label: SYNTHETIC_LABEL,
            bytes: SYNTHETIC,
        },
    )
    .expect("the synthetic fixture opens from bytes through the browser adapters");
    (platform, session)
}

// ---------------------------------------------------------------- file contract

/// The browser file adapter over the host-testable sink.
fn browser_files() -> BrowserFiles<RecordingSink> {
    BrowserFiles::new(RecordingSink::new())
}

fn download_target(name: &str) -> FileTarget {
    FileTarget {
        display_name: name.to_string(),
        opaque: format!("download://{name}"),
    }
}

#[test]
fn begin_save_synthesizes_a_download_target_for_the_purpose() {
    let files = browser_files();
    files
        .begin_save(SavePurpose::SaveDxf, "drawing.dxf")
        .expect("the browser synthesizes the save target");
    files
        .begin_save(SavePurpose::Plot, "drawing.svg")
        .expect("the browser synthesizes the plot target");

    let events = files.poll();
    assert_eq!(events.len(), 2, "exactly one event per begin_save");
    match &events[0] {
        FileEvent::SaveTarget { purpose, target } => {
            assert_eq!(*purpose, SavePurpose::SaveDxf);
            assert_eq!(target.display_name, "drawing.dxf");
            assert_eq!(target.opaque, "download://drawing.dxf");
        }
        other => panic!("expected a SaveTarget, got {other:?}"),
    }
    match &events[1] {
        FileEvent::SaveTarget { purpose, target } => {
            assert_eq!(*purpose, SavePurpose::Plot);
            assert_eq!(target.display_name, "drawing.svg");
            assert_eq!(target.opaque, "download://drawing.svg");
        }
        other => panic!("expected a SaveTarget, got {other:?}"),
    }
}

#[test]
fn begin_save_refuses_a_blank_name_instead_of_inventing_one() {
    let files = browser_files();
    let error = files
        .begin_save(SavePurpose::SaveDxf, "   ")
        .expect_err("a blank download name is refused");
    assert!(
        matches!(error, CadError::InvalidInput(ref reason) if reason.contains("default_name")),
        "a blank name is an explicit refusal: {error:?}"
    );
    assert!(
        files.poll().is_empty(),
        "a refused begin_save queues no fabricated target"
    );
}

#[test]
fn write_atomic_publishes_the_exact_bytes_and_records_the_download() {
    const BYTES: &[u8] = b"0\nSECTION\n0\nEOF\n";
    let files = browser_files();
    files
        .write_atomic(&download_target("drawing.dxf"), BYTES)
        .expect("the recording sink accepts the write");

    assert_eq!(
        files.sink().published(),
        vec![("drawing.dxf".to_string(), BYTES.to_vec())],
        "the sink receives the exact name and bytes"
    );
    assert_eq!(
        files.downloads(),
        vec![DownloadRecord {
            name: "drawing.dxf".to_string(),
            len: BYTES.len(),
        }]
    );
}

#[test]
fn poll_drains_each_event_exactly_once() {
    let files = browser_files();
    files.begin_save(SavePurpose::SaveDxf, "a.dxf").unwrap();
    files.begin_save(SavePurpose::Plot, "b.svg").unwrap();
    assert_eq!(files.poll().len(), 2);
    assert!(
        files.poll().is_empty(),
        "a drained event is never replayed to the next poll"
    );
}

#[test]
fn open_operations_still_refuse_with_the_filesystem_prefix() {
    let files = browser_files();
    let refusals = [
        files.begin_open().unwrap_err(),
        files
            .begin_open_target(&download_target("drawing.dwg"))
            .unwrap_err(),
    ];
    for error in refusals {
        let CadError::Unsupported(reason) = error else {
            panic!("a browser open must be an explicit Unsupported refusal");
        };
        assert!(
            reason.starts_with(UNSUPPORTED_FILESYSTEM),
            "the reason must carry the stable prefix: {reason}"
        );
        assert!(
            reason.contains("picker") && reason.contains("open_bytes"),
            "the refusal names the page picker and the bytes entry: {reason}"
        );
        assert!(
            !reason.contains('。') && reason.is_ascii(),
            "technical reasons stay ASCII: {reason}"
        );
    }
    // No open was ever admitted, so the drain is honestly empty (never a pending
    // event the host would wait for).
    assert!(files.poll().is_empty());
}

#[test]
fn the_source_overwrite_guard_answers_false_and_still_remembers_the_source() {
    // There is no device+inode identity in a browser, so the guard cannot be
    // armed; answering `true` would refuse every save instead.
    let files = browser_files();
    let target = download_target("drawing.dwg");
    assert!(!files.refuses_source_overwrite(&target));
    files.set_source(Some(&target));
    assert_eq!(files.source(), Some(target.clone()));
    assert!(!files.refuses_source_overwrite(&target));
    files.set_source(None);
    assert_eq!(files.source(), None);
}

#[test]
fn the_platform_exposes_the_downloads_it_published() {
    let platform = BrowserPlatform::new();
    platform
        .files()
        .write_atomic(&download_target("drawing.dxf"), b"dxf")
        .expect("the host carrier records the write");
    assert_eq!(
        platform.downloads(),
        vec![DownloadRecord {
            name: "drawing.dxf".to_string(),
            len: 3,
        }]
    );
}

#[test]
fn the_browser_capabilities_advertise_save_and_plot_only() {
    // The constant's own values (const-evaluated: a `true` here is a claim the
    // carrier keeps — the session exports and the download adapter publishes).
    const { assert!(BROWSER_CAPABILITIES.save, "the session performs SAVE") };
    const { assert!(BROWSER_CAPABILITIES.plot, "the session performs PLOT") };
    const {
        assert!(
            !BROWSER_CAPABILITIES.open,
            "the page's picker owns selection; the session does not open"
        )
    };
    const { assert!(!BROWSER_CAPABILITIES.new, "a browser has no New path") };
    const {
        assert!(
            !BROWSER_CAPABILITIES.import_worker,
            "an open runs on the calling thread in wasm"
        )
    };

    // The projection the chrome reads follows the same constant.
    let (_platform, session) = open(BROWSER_CAPABILITIES);
    let status = session.snapshot(None, None, 0);
    assert!(status.can_save && status.can_plot);
    assert!(!status.can_open && !status.can_new);
}

#[test]
fn a_real_save_command_publishes_one_dxf_download() {
    // The whole browser save path, end to end on the host: SAVE is admitted by
    // the real session, the export funnel runs inside the command (the
    // synchronous-target rule), and the finished DXF reaches the download sink.
    let platform = BrowserPlatform::new();
    platform.surface().report([800.0, 600.0], 1.0);
    let session = CadSession::from_bytes(
        options(),
        platform.host_platform(BROWSER_CAPABILITIES),
        RenderSessionHandle::new(BackendPreference::WebGl2),
        monotonic_clock(),
        BytesDocument {
            label: SYNTHETIC_LABEL,
            bytes: SYNTHETIC,
        },
    )
    .expect("the synthetic fixture opens through the browser adapters");
    session
        .set_canvas_metrics([800.0, 600.0], 1.0)
        .expect("the reported surface accepts the canvas metrics");
    session.command("SAVE").expect("SAVE is admitted");
    let downloads = platform.downloads();
    assert_eq!(
        downloads.len(),
        1,
        "exactly one save publishes one download"
    );
    assert!(
        downloads[0].name.ends_with(".dxf"),
        "the download keeps the session's name: {}",
        downloads[0].name
    );
    let published = platform.files().sink().published();
    assert_eq!(published.len(), 1);
    assert!(
        published[0].1.windows(7).any(|window| window == b"SECTION"),
        "the download is a real ASCII DXF document"
    );
}

#[test]
fn a_real_plot_command_publishes_one_svg_download() {
    let fixture =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/plot/a4-layout.dwg");
    let bytes = std::fs::read(&fixture).expect("committed plot fixture");
    let platform = BrowserPlatform::new();
    platform.surface().report([800.0, 600.0], 1.0);
    let session = CadSession::from_bytes(
        options(),
        platform.host_platform(BROWSER_CAPABILITIES),
        RenderSessionHandle::new(BackendPreference::WebGl2),
        monotonic_clock(),
        BytesDocument {
            label: "a4-layout.dwg",
            bytes: &bytes,
        },
    )
    .expect("the plot fixture opens through the browser adapters");
    session.command("PLOT").expect("PLOT is admitted");

    let published = platform.files().sink().published();
    assert_eq!(
        published.len(),
        1,
        "exactly one plot publishes one download"
    );
    assert!(
        published[0].0.ends_with(".svg"),
        "the default plot format is svg: {}",
        published[0].0
    );
    assert!(
        published[0].1.windows(4).any(|window| window == b"<svg"),
        "the download is a real SVG document, not an empty file"
    );
}

// -------------------------------------------------------------- config contract

#[test]
fn the_config_store_is_in_memory_and_never_persistent() {
    let store = MemoryConfigStore::new();
    // The contract this store advertises about itself.
    let persistent = MemoryConfigStore::PERSISTENT;
    assert!(!persistent, "the store never claims durability");
    // A first run: nothing stored, nothing invented.
    assert_eq!(store.load_config(), ConfigLoad::Absent);
    assert_eq!(store.load_preference(), ConfigLoad::Absent);

    store.store_config("{}");
    store
        .persist_preference(r#"{"interaction":{"snapEnabled":false}}"#)
        .unwrap();
    assert!(
        matches!(store.load_config(), ConfigLoad::Text { path, .. } if path == MEMORY_CONFIG_PATH)
    );
    assert!(matches!(
        store.load_preference(),
        ConfigLoad::Text { path, .. } if path == MEMORY_PREFERENCE_PATH
    ));

    // A reload (a fresh store) starts empty: the value never claimed durability.
    let reloaded = MemoryConfigStore::new();
    assert_eq!(reloaded.load_config(), ConfigLoad::Absent);
    assert_eq!(reloaded.load_preference(), ConfigLoad::Absent);
}

// ------------------------------------------------------------- surface contract

#[test]
fn the_surface_is_none_before_the_first_report() {
    let surface = ReportedSurface::new();
    assert!(surface.surface().is_none());
    surface.report([640.0, 480.0], 1.5);
    let metrics = surface.surface().expect("a reported surface is mapped");
    assert_eq!(metrics.logical, [640.0, 480.0]);
    assert_eq!(metrics.scale, 1.5);
    assert_eq!(metrics.safe_insets, [0.0; 4]);
    // A clone shares the cell (one value, not a copy that can drift).
    assert_eq!(surface.clone().surface(), Some(metrics));
    surface.clear();
    assert!(surface.surface().is_none());
}

// ----------------------------------------------------------- resource contract

#[test]
fn resources_refuse_only_when_the_drawing_asks_for_something() {
    // The synthetic fixture's case: no faces, no shaping, no images — a real
    // no-op, not a fabricated install.
    BrowserResources::fonts_decision(&[], false).expect("nothing requested is a no-op");
    BrowserResources::images_decision(&[]).expect("no images is a no-op");

    let font_error = BrowserResources::fonts_decision(&["simplex".into()], false).unwrap_err();
    let CadError::Unsupported(reason) = font_error else {
        panic!("an unwired font package must refuse explicitly");
    };
    assert!(reason.starts_with(UNSUPPORTED_RESOURCES), "{reason}");
    assert!(
        reason.contains('1'),
        "the requested face count is named: {reason}"
    );

    let shaping_error = BrowserResources::fonts_decision(&[], true).unwrap_err();
    assert!(shaping_error.to_string().contains("text_shaping=true"));

    let image_error = BrowserResources::images_decision(&["logo.png".into()]).unwrap_err();
    let CadError::Unsupported(reason) = image_error else {
        panic!("an unwired image decoder must refuse explicitly");
    };
    assert!(reason.starts_with(UNSUPPORTED_RESOURCES), "{reason}");
    assert!(reason.contains("logo") || reason.contains('1'), "{reason}");
}

// ------------------------------------------------------------ the session entry

#[test]
fn the_bytes_entry_opens_the_fixture_and_reports_truthful_status() {
    // A carrier that advertises no session-side open (the page's own picker is a
    // DOM capability) must report `canOpen: false` even with a document open.
    let (_platform, session) = open(HostCapabilities::default());
    let status = session.snapshot(None, None, 0);
    assert_eq!(status.document_label.as_deref(), Some(SYNTHETIC_LABEL));
    assert!(
        status.entity_count.unwrap_or(0) > 0,
        "the imported fixture has entities: {:?}",
        status.entity_count
    );
    assert!(status.can_fit, "a delivered document can be refit");
    assert!(!status.can_open, "no advertised open => canOpen is false");
    assert!(!status.can_new && !status.can_save && !status.can_plot);
    assert!(!status.loading, "a synchronous install is not 'loading'");
    assert_eq!(status.error, None, "a clean open reports no error");
}

#[test]
fn an_advertised_open_is_reported_as_can_open() {
    let capabilities = HostCapabilities {
        open: true,
        ..HostCapabilities::default()
    };
    let (_platform, session) = open(capabilities);
    assert!(
        session.snapshot(None, None, 0).can_open,
        "canOpen follows the advertised capability, not a guess"
    );
}

#[test]
fn line_reaches_the_command_router() {
    // The whole point of this phase: the desktop command vocabulary must be
    // admitted by a session built inside the browser carrier.
    let (_platform, session) = open(HostCapabilities::default());
    session.command("LINE").expect("LINE must be admitted");
    let status = session.snapshot(None, None, 0);
    assert_eq!(status.error, None, "LINE started no error");
    let tool = status.tool.expect("LINE begins a draw capture");
    assert!(
        tool.key.starts_with("draw."),
        "the active tool is a draw capture: {}",
        tool.key
    );
    // The capture really owns the session: a second LINE is refused by the host
    // (one capture at a time), not by this layer.
    session
        .command("CANCEL")
        .expect("an active capture cancels");
    assert!(session.snapshot(None, None, 0).tool.is_none());
}

#[test]
fn an_unknown_command_is_refused_by_name_and_changes_nothing() {
    let (_platform, session) = open(HostCapabilities::default());
    let before = session.snapshot(None, None, 0);
    let error = session.command("FLOOPY").unwrap_err();
    assert!(
        error.starts_with("unsupported CAD command") && error.contains("FLOOPY"),
        "the refusal names the command: {error}"
    );
    let after = session.snapshot(None, None, 0);
    assert_eq!(after.document_label, before.document_label);
    assert_eq!(after.error, None, "a refused name is not a session error");
    assert!(after.tool.is_none(), "a refused name starts no tool");
}

#[test]
fn tick_and_canvas_metrics_are_callable_once_a_surface_is_reported() {
    let (platform, session) = open(HostCapabilities::default());
    // The host's own contract: an unmapped surface is an explicit refusal, never
    // a canvas of invented size.
    let refused = session.tick().unwrap_err();
    assert!(!refused.is_empty(), "the refusal carries a reason");

    platform.surface().report([800.0, 600.0], 1.0);
    session
        .set_canvas_metrics([800.0, 600.0], 1.0)
        .expect("valid logical metrics are accepted");
    session.tick().expect("a mapped surface ticks");
    // A clear/restore cycle is the real resize path and must stay tickable.
    platform.surface().clear();
    assert!(session.tick().is_err());
    platform.surface().report([1024.0, 768.0], 2.0);
    session.set_canvas_metrics([1024.0, 768.0], 2.0).unwrap();
    session.tick().expect("the resized surface ticks");
}

#[test]
fn the_bytes_entry_refuses_a_contradictory_or_unnamed_document() {
    let platform = BrowserPlatform::new();
    let mut with_path = options();
    with_path.drawing = Some(std::path::PathBuf::from("/tmp/other.dwg"));
    let refused = CadSession::from_bytes(
        with_path,
        platform.host_platform(HostCapabilities::default()),
        RenderSessionHandle::new(BackendPreference::WebGl2),
        monotonic_clock(),
        BytesDocument {
            label: SYNTHETIC_LABEL,
            bytes: SYNTHETIC,
        },
    );
    assert!(
        matches!(refused, Err(CadError::InvalidInput(_))),
        "a path plus bytes is a contradictory document source"
    );

    let refused = CadSession::from_bytes(
        options(),
        platform.host_platform(HostCapabilities::default()),
        RenderSessionHandle::new(BackendPreference::WebGl2),
        monotonic_clock(),
        BytesDocument {
            label: "   ",
            bytes: SYNTHETIC,
        },
    );
    assert!(
        matches!(refused, Err(CadError::InvalidInput(_))),
        "a blank label is refused"
    );
}

#[test]
fn an_unparseable_document_fails_the_bytes_entry_explicitly() {
    // Never a fake success: garbage bytes fail the construction (the previous
    // document is untouched, which is what the caller's live session relies on).
    let platform = BrowserPlatform::new();
    let refused = CadSession::from_bytes(
        options(),
        platform.host_platform(HostCapabilities::default()),
        RenderSessionHandle::new(BackendPreference::WebGl2),
        monotonic_clock(),
        BytesDocument {
            label: "garbage.dwg",
            bytes: b"this is not a DWG file",
        },
    );
    assert!(
        refused.is_err(),
        "an unparseable file must not report success"
    );
}
