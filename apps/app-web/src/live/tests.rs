//! Host-target acceptance tests for the **production `dom-canvas` session**.
//!
//! No GPU, no DOM: the session is the real
//! [`CadSession`](cad_ui_core::session::CadSession) over [`BrowserPlatform`] with a
//! `RenderSessionHandle` that has **no device attached** (a renderer is constructed
//! on `attach_device`, which these tests never call). That is enough for everything
//! this lane claims:
//!
//! * the real session can be built and re-opened (a document replacement, not a
//!   rebuild);
//! * the ADR 0025 budgets refuse **before** anything is parsed or installed, with
//!   the named limits and the structured numbers;
//! * the status projections report the session's own entity count, tool, selection
//!   and command log;
//! * the popup path is the real state machine (`about` acknowledges, `textEdit`
//!   derives from the selection, the modal gate then refuses CAD input);
//! * `probe_input` (and the synthesized pan/zoom) move the document and the camera
//!   through the session's own input seam.
//!
//! What it does **not** prove: pixels. `render`/`present` need the device and the
//! canvas presenter (`crate::page`, wasm-only); that half is evidenced by the
//! browser run in `docs/validation-dom-canvas-flip-2026-10-28.md`.

use std::cell::Cell;
use std::rc::Rc;

use cad_engine::render::{BackendPreference, RenderSessionHandle};
use cad_host::RequestClock;
use cad_ui_core::protocol::InputKind;
use cad_ui_core::session::CadSession;

use super::*;
use crate::budget;

/// The compiled-in synthetic fixture: four `LINE`s, no text and no raster image.
pub(crate) const SYNTHETIC: &[u8] =
    include_bytes!("../../../../fixtures/dwg/synthetic-four-lines.dwg");

/// A monotonic millisecond clock (the host-target stand-in for
/// `performance.now()`), so the session's request pump sees a real progression.
pub(crate) fn monotonic_clock() -> RequestClock {
    let millis = Cell::new(0.0_f64);
    Rc::new(move || {
        let now = millis.get();
        millis.set(now + 0.05);
        now
    })
}

/// The heap gauge a host test injects: a small, fixed module heap.
fn small_heap() -> Option<f64> {
    Some(2.0 * 1024.0 * 1024.0)
}

/// The fixture's metrics (800×600 CSS at dpr 1).
pub(crate) fn metrics() -> Metrics {
    Metrics::new(800.0, 600.0, 1.0).expect("valid metrics")
}

/// The production session over the synthetic fixture, built exactly as `start`
/// builds it (minus the device and the presenter).
pub(crate) fn live() -> Live {
    Live::install(
        RenderSessionHandle::new(BackendPreference::WebGl2),
        monotonic_clock(),
        SYNTHETIC_LABEL,
        SYNTHETIC,
        metrics(),
        &small_heap,
    )
    .expect("the synthetic fixture installs through the production path")
}

/// The same session, without the wrapper: for tests that only need the `CadSession`.
pub(crate) fn fixture_session() -> CadSession {
    live().session
}

/// The presenter facts of a host test (no GPU: the numbers are injected, and the
/// adapter name says so).
fn facts() -> PresenterFacts {
    PresenterFacts {
        backend: "gl".to_string(),
        adapter: "host-test adapter (no device)".to_string(),
        format: "Bgra8Unorm".to_string(),
        target: "html-canvas",
        isolation: "main-thread",
        physical: (800, 600),
        presented: 3,
        configure_count: 1,
    }
}

/// One primary click (press + release in the same place) at a canvas position.
fn click(live: &mut Live, at: [f64; 2]) {
    for (kind, button) in [
        (InputKind::PointerDown, Some(0)),
        (InputKind::PointerUp, Some(0)),
    ] {
        let sequence = live.input.next_sequence();
        let input = input::pointer(sequence, kind, at, button);
        live.probe(&input).expect("a primary click is accepted");
    }
}

// ---------------------------------------------------------------------------
// Build + replace
// ---------------------------------------------------------------------------

#[test]
fn the_production_session_holds_the_fixture_and_reports_its_own_facts() {
    let live = live();
    let status = live.cad_status(&facts());
    assert_eq!(status.document_label.as_deref(), Some(SYNTHETIC_LABEL));
    assert_eq!(status.entity_count, Some(4), "four LINEs in the fixture");
    assert_eq!(status.import_count, 1);
    assert_eq!(status.tool, None);
    assert_eq!(status.selection.count, 0);
    assert!(
        status.can_fit,
        "a delivered document can be refit (the session's own projection)"
    );
    // The carrier advertises no host-side open/save/plot/new, so none is claimed.
    assert!(!status.can_new && !status.can_open && !status.can_save && !status.can_plot);
    assert_eq!(status.error, None);
    // The database is retained: that is what makes the facts above real. The two
    // flags describe two carriers, so they are asserted as the constants they are.
    const { assert!(DB_RETAINED) };
    const {
        assert!(
            !budget::DB_RETAINED,
            "the viewer (worker carrier) still releases its database"
        )
    };
    let document: serde_json::Value =
        serde_json::from_str(&live.document_json(&facts(), small_heap())).expect("valid JSON");
    assert_eq!(document["label"], serde_json::json!(SYNTHETIC_LABEL));
    assert_eq!(document["entityCount"], serde_json::json!(4));
    assert_eq!(document["db_retained"], serde_json::json!(true));
    assert_eq!(document["ok"], serde_json::json!(true));
}

#[test]
fn open_bytes_replaces_the_document_in_place_and_keeps_the_session() {
    let mut live = live();
    let before = live.cad_status(&facts()).import_count;
    live.replace_document("second.dwg", SYNTHETIC, &small_heap)
        .expect("a valid drawing replaces the open document");
    let status = live.cad_status(&facts());
    assert_eq!(status.document_label.as_deref(), Some("second.dwg"));
    assert_eq!(
        status.import_count,
        before + 1,
        "a replacement is a second successful import (the device, presenter and session survive)"
    );
    assert_eq!(status.entity_count, Some(4), "same bytes, same document");
    assert_eq!(live.label, "second.dwg");
    assert_eq!(live.bytes, SYNTHETIC.len());
    assert_eq!(status.error, None);
}

#[test]
fn a_refused_replacement_leaves_the_open_document_and_its_status_intact() {
    let mut live = live();
    let error = live
        .replace_document("garbage.dwg", b"this is not a DWG file", &small_heap)
        .expect_err("an unparseable file is refused");
    assert_eq!(error.step, STEP_IMPORT);
    // The session still reports the previous document, and the failure is not
    // recorded as a session error (the open simply did not happen).
    let status = live.cad_status(&facts());
    assert_eq!(status.document_label.as_deref(), Some(SYNTHETIC_LABEL));
    assert_eq!(status.entity_count, Some(4));
    assert_eq!(status.import_count, 1);
}

// ---------------------------------------------------------------------------
// Budgets
// ---------------------------------------------------------------------------

#[test]
fn the_input_byte_budget_refuses_before_anything_is_parsed() {
    // One byte over the cap: the refusal must happen before the importer runs, so
    // the "file" can be bytes that would never parse.
    let oversized = vec![0_u8; budget::MAX_INPUT_BYTES + 1];
    let mut live = live();
    let before = live.cad_status(&facts());
    let error = live
        .replace_document("huge.dwg", &oversized, &small_heap)
        .expect_err("over MAX_INPUT_BYTES is refused");
    assert_eq!(error.step, STEP_BUDGET);
    let refusal = error.budget.expect("the structured numbers travel with it");
    assert_eq!(refusal.name, "MAX_INPUT_BYTES");
    assert_eq!(refusal.limit_bytes, budget::MAX_INPUT_BYTES as f64);
    assert_eq!(refusal.measured_bytes, (budget::MAX_INPUT_BYTES + 1) as f64);
    // The viewer's own message shape: the label, the budget, the measurement.
    assert!(refusal.message.contains("huge.dwg"), "{}", refusal.message);
    assert!(
        refusal.message.contains("MAX_INPUT_BYTES"),
        "{}",
        refusal.message
    );
    assert!(
        refusal.message.contains("not parsed"),
        "{}",
        refusal.message
    );
    assert!(refusal.message.contains("unchanged"), "{}", refusal.message);
    // Nothing changed.
    let after = live.cad_status(&facts());
    assert_eq!(after.document_label.as_deref(), Some(SYNTHETIC_LABEL));
    assert_eq!(after.import_count, before.import_count);
}

#[test]
fn the_heap_budget_refuses_before_another_document_is_installed() {
    // A module heap already above MAX_WASM_HEAP_BYTES: another document could only
    // push the module toward the 4 GiB abort, so the install is refused and nothing
    // is touched.
    let over = budget::MAX_WASM_HEAP_BYTES + 4096.0;
    let gauge = move || Some(over);
    let mut live = live();
    let error = live
        .replace_document("another.dwg", SYNTHETIC, &gauge)
        .expect_err("an over-budget heap refuses the install");
    assert_eq!(error.step, STEP_BUDGET);
    let refusal = error.budget.expect("structured numbers");
    assert_eq!(refusal.name, "MAX_WASM_HEAP_BYTES");
    assert_eq!(refusal.measured_bytes, over);
    assert!(
        refusal.message.contains("before any GPU upload"),
        "{}",
        refusal.message
    );
    assert_eq!(
        live.cad_status(&facts()).import_count,
        1,
        "no document was installed"
    );

    // The same heap also refuses the very first install (a fresh module already over
    // the cap has no honest path to a complete frame).
    let refused = Live::install(
        RenderSessionHandle::new(BackendPreference::WebGl2),
        monotonic_clock(),
        SYNTHETIC_LABEL,
        SYNTHETIC,
        metrics(),
        &gauge,
    )
    .err()
    .expect("over budget");
    assert_eq!(refused.step, STEP_BUDGET);
}

#[test]
fn a_post_install_measurement_over_the_cap_is_reported_not_ignored() {
    // The carrier cannot undo a delivered open, so an over-budget *measurement* after
    // the install is published (with the limit and the number) instead of being
    // swallowed or quietly turning the cap up.
    let reads = Cell::new(0_u32);
    let gauge = move || {
        reads.set(reads.get() + 1);
        // The pre-install checks see a small heap; the post-install read sees the
        // retained session's real (over-cap) footprint.
        if reads.get() <= 1 {
            Some(1024.0)
        } else {
            Some(budget::MAX_WASM_HEAP_BYTES + 1.0)
        }
    };
    let live = Live::install(
        RenderSessionHandle::new(BackendPreference::WebGl2),
        monotonic_clock(),
        SYNTHETIC_LABEL,
        SYNTHETIC,
        metrics(),
        &gauge,
    )
    .expect("the pre-install heap admitted the file");
    let refusal = live
        .over_budget
        .as_ref()
        .expect("the measurement is published");
    assert_eq!(refusal.name, "MAX_WASM_HEAP_BYTES");
    let status: serde_json::Value =
        serde_json::from_str(&live.canvas_status_json(&facts(), Some(refusal.measured_bytes)))
            .expect("valid JSON");
    assert_eq!(
        status["over_budget"]["name"],
        serde_json::json!("MAX_WASM_HEAP_BYTES")
    );
    assert_eq!(
        status["over_budget"]["measured_bytes"],
        serde_json::json!(budget::MAX_WASM_HEAP_BYTES + 1.0)
    );
}

// ---------------------------------------------------------------------------
// The wire surface
// ---------------------------------------------------------------------------

#[test]
fn a_command_reaches_the_real_router_and_moves_the_status() {
    let mut live = live();
    live.command("LINE")
        .expect("LINE is admitted by the router");
    let status = live.cad_status(&facts());
    let tool = status.tool.expect("LINE begins a draw capture");
    assert!(tool.key.starts_with("draw."), "{}", tool.key);
    assert!(
        !status.command_log.is_empty(),
        "the router echo is the session's command log"
    );
    live.command("CANCEL").expect("CANCEL ends the capture");
    assert!(live.cad_status(&facts()).tool.is_none());
}

#[test]
fn the_vocabulary_the_carrier_advertises_is_what_the_router_executes() {
    // A name the old carrier refused is now executed by the same session: `PAN` is
    // an admitted selector, and the router's own refusal for an unknown name is the
    // desktop one.
    let mut live = live();
    live.command("PAN").expect("PAN is admitted (modal pan)");
    live.command("CANCEL").expect("CANCEL ends the modal pan");
    let error = live
        .command("FLOOPY")
        .expect_err("an unknown name is refused");
    assert!(error.message.contains("FLOOPY"), "{}", error.message);
}

// ---------------------------------------------------------------------------
// Popups
// ---------------------------------------------------------------------------

#[test]
fn open_popup_answers_a_real_acknowledgement_and_gates_the_input() {
    let mut live = live();
    let ack = {
        let mut popups = LivePopups::new(&live);
        popups.open("about").expect("about is allowlisted")
    };
    assert!(ack.accepted);
    assert_eq!(ack.kind, "about");
    assert!(!ack.existing, "a fresh open");
    assert_eq!(ack.popup_label, "popup-1");
    let payload = ack.payload.expect("about carries the build facts");
    match payload {
        cad_ui_core::popup::PopupPayload::About { version, build } => {
            assert!(!version.is_empty(), "the workspace version");
            assert!(build == "debug" || build == "release", "{build}");
        }
        other => panic!("about must carry the About payload, got {other:?}"),
    }
    // The gate is projected into the status, and it refuses CAD input by name.
    assert!(live.popups.modal_open());
    let status = live.cad_status(&facts());
    let popup = status.popup.expect("the open popup is projected");
    assert_eq!(popup.kind, "about");
    let refusal = live
        .probe(&input::pointer(
            99,
            InputKind::PointerDown,
            [10.0, 10.0],
            Some(0),
        ))
        .expect_err("the modal gate refuses pointer input");
    assert_eq!(refusal.message, POPUP_MODAL_OPEN);

    // A second open reports the existing popup instead of opening another one.
    let again = {
        let mut popups = LivePopups::new(&live);
        popups
            .open("options")
            .expect("an open popup is reported, not replaced")
    };
    assert!(again.existing);
    assert_eq!(
        again.kind, "about",
        "the open popup is the one that answers"
    );

    // Close is idempotent and clears the gate.
    {
        let mut popups = LivePopups::new(&live);
        popups.close().expect("close");
        popups.close().expect("close is idempotent");
    }
    assert!(!live.popups.modal_open());
    assert!(live.cad_status(&facts()).popup.is_none());
    live.probe(&input::pointer(
        100,
        InputKind::PointerMove,
        [10.0, 10.0],
        None,
    ))
    .expect("input flows again once the gate is clear");
}

#[test]
fn the_text_edit_popup_derives_its_payload_from_the_selection_or_refuses() {
    // Nothing selected: the host refusal, and no gate is set.
    let live = live();
    let refusal = {
        let mut popups = LivePopups::new(&live);
        popups.open("textEdit").expect_err("no text is selected")
    };
    assert!(
        refusal.starts_with("popup.no_text_selection"),
        "the stable refusal names the requirement: {refusal}"
    );
    assert!(
        !live.popups.modal_open(),
        "a refused open leaves no gate behind"
    );
    // An unknown kind is refused by the allowlist before anything is touched.
    let unknown = {
        let mut popups = LivePopups::new(&live);
        popups
            .open("settings")
            .expect_err("not an allowlisted kind")
    };
    assert!(unknown.starts_with("popup.unknown_kind"), "{unknown}");
}

// ---------------------------------------------------------------------------
// Input
// ---------------------------------------------------------------------------

#[test]
fn probe_input_draws_a_line_through_the_real_input_seam() {
    let mut live = live();
    live.command("LINE").expect("start the line tool");
    assert_eq!(live.cad_status(&facts()).entity_count, Some(4));
    // Two clicks in the drawing area: inside a capture each click is a point.
    click(&mut live, [200.0, 200.0]);
    click(&mut live, [500.0, 350.0]);
    live.command("CONFIRM").expect("confirm the two points");
    let status = live.cad_status(&facts());
    assert_eq!(
        status.entity_count,
        Some(5),
        "the confirmed LINE is a real entity in the session's database: {:?}",
        status.error
    );
    assert_eq!(status.tool, None, "the capture closed on confirm");
    assert_eq!(status.error, None);
}

#[test]
fn probe_input_refuses_a_position_it_cannot_use_instead_of_guessing() {
    let mut live = live();
    let mut input = input::pointer(1, InputKind::PointerDown, [0.0, 0.0], None);
    input.x = None;
    let refusal = live.probe(&input).expect_err("a pointer needs a position");
    assert_eq!(refusal.step, STEP_INPUT);
    let mut input = input::pointer(2, InputKind::PointerDown, [0.0, 0.0], Some(-1));
    input.button = Some(-1);
    let refusal = live
        .probe(&input)
        .expect_err("an undefined button is refused");
    assert!(
        refusal.message.contains("unsupported pointer button"),
        "{}",
        refusal.message
    );
}

#[test]
fn the_synthesized_pan_is_the_middle_drag_the_chrome_sends() {
    let mut live = live();
    let before = live.cad_status(&facts()).camera.expect("a 2D view");
    // A gesture is a press plus moves; the anchor is the viewport centre and the
    // delta accumulates, so nothing is lost to the drag threshold.
    for _ in 0..4 {
        live.pan_by([50.0, 0.0]).expect("a pan step");
    }
    assert!(
        live.input.pan_open(),
        "the press is retained across the calls"
    );
    live.close_pan_inputs();
    assert!(!live.input.pan_open(), "the gesture is closed");
    let after = live.cad_status(&facts()).camera.expect("still a 2D view");
    // 200 CSS px to the right moves the camera *left* by 200 × world_per_px.
    let expected = before.center[0] - 200.0 * after.scale;
    assert!(
        (after.center[0] - expected).abs() < 1e-6,
        "pan moved the camera to {:?}, expected x≈{expected} (before {:?})",
        after.center,
        before.center
    );
    assert!(
        (after.center[1] - before.center[1]).abs() < 1e-9,
        "no vertical pan"
    );
    assert!(
        (after.scale - before.scale).abs() < 1e-12,
        "a pan never zooms"
    );
}

#[test]
fn the_synthesized_zoom_is_one_wheel_event_about_the_cursor() {
    let mut live = live();
    let before = live.cad_status(&facts()).camera.expect("a 2D view");
    live.zoom_about(1.25, [400.0, 300.0])
        .expect("a wheel notch in");
    let after = live.cad_status(&facts()).camera.expect("a 2D view");
    assert!(
        (after.scale - before.scale / 1.25).abs() < 1e-9,
        "one 1.25× notch: {} -> {}",
        before.scale,
        after.scale
    );
    live.zoom_about(0.8, [400.0, 300.0])
        .expect("a wheel notch out");
    let back = live.cad_status(&facts()).camera.expect("a 2D view");
    assert!(
        (back.scale - before.scale).abs() < 1e-9,
        "the reciprocal step returns to the framing: {} vs {}",
        back.scale,
        before.scale
    );
}

#[test]
fn a_zoom_factor_the_wheel_would_clamp_is_refused_not_approximated() {
    let mut live = live();
    for factor in [0.1, 10.0, 0.0, -2.0, f64::NAN] {
        let error = live
            .zoom_about(factor, [400.0, 300.0])
            .expect_err("outside the host's wheel range");
        assert_eq!(error.step, STEP_NAVIGATION, "{factor}");
    }
    // …and nothing moved.
    assert_eq!(live.cad_status(&facts()).error, None);
}

// ---------------------------------------------------------------------------
// Status / document projections
// ---------------------------------------------------------------------------

#[test]
fn the_canvas_status_reports_the_session_and_never_the_viewer_fields() {
    let live = live();
    let status: serde_json::Value =
        serde_json::from_str(&live.canvas_status_json(&facts(), small_heap())).expect("valid JSON");
    assert_eq!(status["ok"], serde_json::json!(true));
    assert_eq!(status["label"], serde_json::json!(SYNTHETIC_LABEL));
    assert_eq!(status["entityCount"], serde_json::json!(4));
    assert_eq!(status["target"], serde_json::json!("html-canvas"));
    assert_eq!(status["isolation"], serde_json::json!("main-thread"));
    assert_eq!(status["presented"], serde_json::json!(3));
    assert_eq!(status["db_retained"], serde_json::json!(true));
    // No draw count and no chunked upload on this carrier: absent or null, never a
    // number standing in for something nobody measured.
    assert_eq!(status["draws"], serde_json::json!(null));
    assert!(status.get("upload_chunks").is_none());
}

#[test]
fn a_failed_start_reports_the_step_and_the_marker() {
    let failure = Failure::new(STEP_WEBGL2, "request_adapter(html-canvas): no adapter");
    assert_eq!(failure.marker(), Some(WEBGL2_MARKER));
    let status: serde_json::Value =
        serde_json::from_str(&failure_status_json(&failure, small_heap())).expect("valid JSON");
    assert_eq!(status["ok"], serde_json::json!(false));
    assert_eq!(status["step"], serde_json::json!(STEP_WEBGL2));
    assert_eq!(status["marker"], serde_json::json!(WEBGL2_MARKER));
    assert_eq!(status["label"], serde_json::json!(null));
    // Only `webgl2` carries the marker.
    assert_eq!(Failure::new(STEP_IMPORT, "bad file").marker(), None);
}

#[test]
fn the_metrics_are_validated_like_the_viewer_does() {
    assert!(Metrics::new(f64::NAN, 600.0, 1.0).is_err());
    assert!(Metrics::new(0.0, 600.0, 1.0).is_err());
    assert!(Metrics::new(800.0, 600.0, 0.0).is_err());
    assert!(Metrics::new(800.0, 600.0, 1.0).is_ok());
    // A CSS size that rounds to nothing in physical pixels is refused.
    let tiny = Metrics::new(0.2, 0.2, 1.0).expect("positive");
    assert!(tiny.physical().is_err());
    assert_eq!(metrics().physical().expect("800x600"), (800, 600));
    assert_eq!(metrics().centre(), [400.0, 300.0]);
}
