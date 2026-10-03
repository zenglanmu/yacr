#![cfg(target_os = "linux")]
use app_linux::{LinuxApp, LinuxOptions};
use slint::ComponentHandle;
use std::path::PathBuf;

#[test]
fn options_refuse_unknown_missing_and_invalid_values() {
    for args in [
        vec!["--headless"],
        vec!["--open"],
        vec!["--size", "NaNx800"],
        vec!["--size", "0x800"],
        vec!["--locale", "fr"],
        vec!["--invented", "true"],
    ] {
        assert!(LinuxOptions::parse(args.into_iter().map(String::from)).is_err());
    }
    let options = LinuxOptions::parse(
        [
            "--headless",
            "--output",
            "/tmp/opencode/new-evidence",
            "--size",
            "390x844",
            "--locale",
            "en",
        ]
        .into_iter()
        .map(String::from),
    )
    .unwrap();
    assert!(options.headless);
    assert_eq!(options.size, [390.0, 844.0]);
    assert_eq!(options.locale, "en");
}

#[test]
fn real_linux_host_runs_commands_and_refuses_fake_file_success() {
    cad_ui_slint::offscreen::install().unwrap();
    let directory = PathBuf::from(format!(
        "/tmp/opencode/yacr-linux-test-{}",
        std::process::id()
    ));
    std::fs::create_dir(&directory).unwrap();
    let export = directory.join("annotations.json");
    let app = LinuxApp::new(LinuxOptions {
        headless: true,
        annotation_export: Some(export.clone()),
        annotation_import: Some(export.clone()),
        ..LinuxOptions::default()
    })
    .unwrap();
    app.adapter.component().show().unwrap();
    app.adapter.component().invoke_open_requested();
    assert!(app
        .adapter
        .component()
        .get_status_label()
        .contains("--open"));
    app.adapter.component().invoke_begin_draw_tool(
        cad_ui_slint::MessageSource::from_request("zh-CN")
            .text("draw.kind.line", &[])
            .into(),
    );
    app.adapter.component().invoke_canvas_pick(350.0, 210.0);
    app.adapter.component().invoke_canvas_pick(500.0, 260.0);
    app.adapter.component().invoke_confirm_draw_requested();
    assert!(
        app.adapter.component().get_can_undo(),
        "real command sink must commit drawing history"
    );
    app.adapter.component().invoke_undo_requested();
    assert!(app.adapter.component().get_can_redo());
    app.adapter.component().invoke_redo_requested();
    assert!(app.adapter.component().get_can_undo());
    app.adapter.component().invoke_measure_kind_selected(
        cad_ui_slint::MessageSource::from_request("zh-CN")
            .text("measure.kind.distance", &[])
            .into(),
    );
    app.adapter.component().invoke_canvas_pick(350.0, 210.0);
    app.adapter.component().invoke_canvas_pick(500.0, 260.0);
    app.adapter
        .component()
        .invoke_confirm_measurement_requested();
    assert!(app
        .adapter
        .component()
        .get_measurement_can_save_annotation());
    app.adapter.component().invoke_save_measurement_requested();
    app.adapter.component().invoke_export_requested();
    let text = std::fs::read_to_string(&export).unwrap();
    assert!(serde_json::from_str::<serde_json::Value>(&text).is_ok());
    app.adapter.component().invoke_import_requested();
    assert!(!app.adapter.component().get_status_label().contains("失败"));
    app.acceptance(&directory.join("evidence")).unwrap();
    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(directory.join("evidence/report.json")).unwrap())
            .unwrap();
    assert_eq!(report["host"], "app-linux");
    assert_eq!(report["navigationPixelsChanged"], true);
    assert!(
        app.acceptance(&directory.join("evidence")).is_err(),
        "old evidence must never be reused"
    );
    app.adapter
        .window()
        .set_size(slint::PhysicalSize::new(390, 844));
    app.adapter.handle().refresh_window_layout().unwrap();
    cad_ui_slint::offscreen::snapshot(app.adapter.window()).unwrap();
    assert!(
        app.adapter.component().get_phone_shell(),
        "native resize reclassifies the layout"
    );
    assert_eq!(app.adapter.handle().shell_geometry().unwrap().0[2], 390.0);
    let invalid = LinuxApp::new(LinuxOptions {
        drawing: Some(directory.join("missing.dwg")),
        ..LinuxOptions::default()
    });
    assert!(
        invalid.is_err(),
        "missing DWG must not fall back to synthetic success"
    );
}
