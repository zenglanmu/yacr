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
    ConfigLoad, ConfigStore, FileTarget, HostCapabilities, PlatformFiles, SavePurpose,
    SurfaceSource,
};
use cad_ui_core::session::{BytesDocument, CadSession};

use super::*;

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

#[test]
fn every_file_operation_refuses_with_a_stable_reason() {
    let files = MemoryFiles::new();
    let target = FileTarget::from_path(SYNTHETIC_LABEL);
    let refusals = [
        files.begin_open().unwrap_err(),
        files.begin_open_target(&target).unwrap_err(),
        files
            .begin_save(SavePurpose::SaveDxf, SYNTHETIC_LABEL)
            .unwrap_err(),
        files.write_atomic(&target, b"x").unwrap_err(),
    ];
    for error in refusals {
        let CadError::Unsupported(reason) = error else {
            panic!("a browser file operation must be an explicit Unsupported refusal");
        };
        assert!(
            reason.starts_with(UNSUPPORTED_FILESYSTEM),
            "the reason must carry the stable prefix: {reason}"
        );
        assert!(
            !reason.contains('。') && reason.is_ascii(),
            "technical reasons stay ASCII: {reason}"
        );
    }
    // No interaction was ever admitted, so the drain is honestly empty (never a
    // pending event the host would wait for).
    assert!(files.poll().is_empty());
}

#[test]
fn the_source_overwrite_guard_answers_false_and_still_remembers_the_source() {
    // There is no device+inode identity in a browser, so the guard cannot be
    // armed; answering `true` would refuse every save instead.
    let files = MemoryFiles::new();
    let target = FileTarget::from_path(SYNTHETIC_LABEL);
    assert!(!files.refuses_source_overwrite(&target));
    files.set_source(Some(&target));
    assert_eq!(files.source(), Some(target.clone()));
    assert!(!files.refuses_source_overwrite(&target));
    files.set_source(None);
    assert_eq!(files.source(), None);
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
