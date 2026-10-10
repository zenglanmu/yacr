#![cfg(target_os = "linux")]
//! Desktop "New blank drawing" end to end: open a committed synthetic fixture
//! with the production [`LinuxApp`], then run New and assert the session is a
//! valid empty drawing with no panic.
//!
//! Software Vulkan offscreen only; the offscreen Slint platform installs a
//! process-global rendering notifier, so a single test owns the whole flow.
use app_linux::{LinuxApp, LinuxOptions};
use slint::{ComponentHandle, Model as _};
use std::path::PathBuf;

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/dxf/qcad-examples/entities.dxf")
}

#[test]
fn new_blank_drawing_after_open_is_empty_and_valid() {
    cad_ui_slint::offscreen::install().unwrap();
    let app = LinuxApp::new(LinuxOptions {
        headless: true,
        drawing: Some(fixture()),
        ..LinuxOptions::default()
    })
    .unwrap();
    app.adapter.component().show().unwrap();
    let ui = app.adapter.component();

    // The committed fixture opened with real layers and a non-empty title.
    assert!(ui.get_can_new(), "desktop host must advertise New");
    assert!(
        ui.get_layer_rows().row_count() > 1,
        "the fixture should expose multiple layers before New"
    );
    assert!(!ui.get_application_title().is_empty());

    // Populate the session: a real selection plus an active draw capture, so New
    // has something to clear/cancel.
    ui.invoke_select_all_requested();
    assert!(
        ui.get_selection_count() > 0,
        "the fixture should yield a non-empty selection"
    );
    ui.invoke_begin_draw_tool("line".into());
    assert!(
        ui.get_draw_tool_active(),
        "a LINE capture must be active before New"
    );

    // New replaces the document with a valid, empty drawing.
    ui.invoke_new_requested();
    // Pump one frame so the push/bridge settles; a panic here is a failure.
    let _ = cad_ui_slint::offscreen::snapshot(app.adapter.window()).unwrap();

    let rows = ui.get_layer_rows();
    assert_eq!(rows.row_count(), 1, "New keeps exactly the default layer");
    assert_eq!(rows.row_data(0).unwrap().name.to_string(), "0");
    assert_eq!(
        ui.get_application_title().to_string(),
        "untitled",
        "New names the fresh document"
    );
    assert_eq!(ui.get_selection_count(), 0, "New clears the selection");
    assert_eq!(ui.get_property_rows().row_count(), 0);
    assert!(!ui.get_draw_tool_active(), "New cancels the draw capture");
    assert!(ui.get_can_new(), "New stays available after New");

    // FIT on the empty drawing must report the explicit empty-fit message, not
    // a generic "completed".
    ui.invoke_fit_requested();
    assert_eq!(
        ui.get_status_label().to_string(),
        cad_ui_slint::MessageSource::from_request("zh-CN").text("fit.empty", &[])
    );
}
