#![cfg(target_os = "linux")]
use app_linux::{config_file, LinuxApp, LinuxOptions};
use slint::ComponentHandle;
use std::path::PathBuf;

/// Install the process-global offscreen platform once; every test in this
/// binary reuses it (`set_platform` rejects a second install).
fn install_offscreen() {
    use std::sync::OnceLock;
    static PLATFORM: OnceLock<Result<(), String>> = OnceLock::new();
    let result = PLATFORM
        .get_or_init(|| cad_ui_slint::offscreen::install().map_err(|error| error.to_string()));
    if let Err(error) = result {
        panic!("offscreen platform install failed: {error}");
    }
}

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
    install_offscreen();
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
    cad_ui_slint::offscreen::snapshot(app.adapter.window()).unwrap();
    let (rect, _) = app.adapter.handle().shell_geometry().unwrap();
    for y in [44.0, 92.0, 140.0, 188.0] {
        assert!(!app
            .adapter
            .handle()
            .canvas_hit_test([rect[0] + rect[2] - 40.0, rect[1] + y])
            .unwrap());
    }
    assert!(!app.adapter.component().get_can_open());
    assert!(!app.adapter.component().get_can_trim());
    assert!(!app.adapter.component().get_can_switch_backend());
    assert!(app.adapter.component().get_can_export());
    app.adapter.component().invoke_zoom_requested(1.25);
    app.adapter.component().invoke_zoom_requested(0.8);
    app.adapter.component().invoke_pan_requested();
    assert!(app.adapter.component().get_pan_active());
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
    assert!(
        !app.adapter.component().get_pan_active(),
        "starting a capture exits pan mode"
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
    // Desktop selection runs on a worker. These are injected chooser contracts,
    // not evidence of an actual desktop portal or Wayland session.
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    let picked = directory.join("picked.dxf");
    std::fs::write(&picked, b"0\nSECTION\n2\nHEADER\n9\n$ACADVER\n1\nAC1015\n0\nENDSEC\n0\nSECTION\n2\nENTITIES\n0\nLINE\n5\n10\n8\n0\n10\n1\n20\n2\n30\n0\n11\n4\n21\n6\n31\n0\n0\nENDSEC\n0\nEOF\n").unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let missing = directory.join("picker-missing.dwg");
    let desktop = LinuxApp::new_with_file_picker(
        LinuxOptions::default(),
        Arc::new(move || match count.fetch_add(1, Ordering::SeqCst) {
            0 => Ok(picked.clone()),
            1 => Err(cad_domain::CadError::Cancelled),
            _ => Ok(missing.clone()),
        }),
    )
    .unwrap();
    desktop.adapter.component().show().unwrap();
    for expected in ["已打开", "已取消", "失败"] {
        assert!(desktop.adapter.component().get_can_open());
        desktop.adapter.component().invoke_open_requested();
        assert!(!desktop.adapter.component().get_can_open());
        for _ in 0..100 {
            std::thread::sleep(std::time::Duration::from_millis(10));
            cad_ui_slint::offscreen::snapshot(desktop.adapter.window()).unwrap();
            if desktop.adapter.component().get_can_open() {
                break;
            }
        }
        assert!(desktop
            .adapter
            .component()
            .get_status_label()
            .contains(expected));
        assert!(desktop
            .adapter
            .component()
            .get_application_title()
            .contains("picked.dxf"));
        assert!(desktop
            .adapter
            .component()
            .get_diagnostics_backend_label()
            .contains("vulkan"));
    }
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    let invalid = LinuxApp::new(LinuxOptions {
        drawing: Some(directory.join("missing.dwg")),
        ..LinuxOptions::default()
    });
    assert!(
        invalid.is_err(),
        "missing DWG must not fall back to synthetic success"
    );
}

#[test]
fn options_parse_config_and_preference_paths() {
    let options = LinuxOptions::parse(
        [
            "--config",
            "/tmp/config.json",
            "--preferences",
            "/tmp/preferences.json",
        ]
        .into_iter()
        .map(String::from),
    )
    .unwrap();
    assert_eq!(options.config, Some(PathBuf::from("/tmp/config.json")));
    assert_eq!(
        options.preferences,
        Some(PathBuf::from("/tmp/preferences.json"))
    );
    for args in [vec!["--config"], vec!["--preferences"]] {
        assert!(LinuxOptions::parse(args.into_iter().map(String::from)).is_err());
    }
    assert!(LinuxOptions::parse(["--invented", "value"].into_iter().map(String::from)).is_err());
}

#[test]
fn config_dir_prefers_xdg_and_falls_back_to_home() {
    let resolve = config_file::resolve_config_dir;
    assert_eq!(
        resolve(Some("/xdg"), Some("/home/user")),
        Some(PathBuf::from("/xdg/yacr"))
    );
    assert_eq!(
        resolve(None, Some("/home/user")),
        Some(PathBuf::from("/home/user/.config/yacr"))
    );
    assert_eq!(
        resolve(Some(""), Some("/home/user")),
        Some(PathBuf::from("/home/user/.config/yacr"))
    );
    assert_eq!(
        resolve(Some("/xdg"), None),
        Some(PathBuf::from("/xdg/yacr"))
    );
    assert_eq!(resolve(Some(""), Some("")), None);
    assert_eq!(resolve(None, None), None);
}
