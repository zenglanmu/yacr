#![cfg(target_os = "linux")]
//! Source-path corruption guard across a *failed* open attempt.
//!
//! The guard must point at the successfully-opened file, never at a file whose
//! open failed. This drives the real picker + async-read path so a failing open
//! cannot silently move the guard.
use app_linux::{LinuxApp, LinuxOptions, SavePathProvider};
use slint::ComponentHandle;
use std::path::PathBuf;
use std::sync::Arc;

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/dxf/qcad-flange/flange.dxf")
}

fn zh(key: &str, values: &[(&str, &str)]) -> String {
    cad_ui_slint::MessageSource::from_request("zh-CN").text(key, values)
}

fn pump_until(app: &LinuxApp, mut done: impl FnMut() -> bool) -> bool {
    for _ in 0..400 {
        std::thread::sleep(std::time::Duration::from_millis(10));
        let _ = cad_ui_slint::offscreen::snapshot(app.adapter.window());
        if done() {
            return true;
        }
    }
    false
}

#[test]
fn failed_open_does_not_move_the_source_path_guard() {
    cad_ui_slint::offscreen::install().unwrap();
    let dir =
        PathBuf::from("/tmp/opencode").join(format!("yacr-save-guard-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let good = dir.join("good.dxf");
    std::fs::copy(fixture(), &good).unwrap();
    let good_bytes = std::fs::read(&good).unwrap();
    let bad = dir.join("bad.dxf");
    std::fs::write(&bad, b"this is not a drawing at all").unwrap();

    // Non-headless so the open goes through the picker + async-read path.
    let bad_for_picker = bad.clone();
    let picker = Arc::new(move || Ok(bad_for_picker.clone()));
    let saver: SavePathProvider = Arc::new({
        let good = good.clone();
        move |_default: &str| Ok(good.clone())
    });

    let app = LinuxApp::new_with_providers(
        LinuxOptions {
            headless: false,
            drawing: Some(good.clone()),
            ..LinuxOptions::default()
        },
        picker,
        saver,
    )
    .unwrap();
    app.adapter.component().show().unwrap();
    let ui = app.adapter.component();

    // Wait for the good drawing to open and settle.
    assert!(
        pump_until(&app, || ui.get_can_open()),
        "good drawing did not open: {}",
        ui.get_status_label()
    );
    assert!(ui.get_can_save(), "save must be available after open");

    // Attempt to open the bad file; it must fail and must not move the guard.
    let failed_prefix = zh("linux.failed", &[("error", "")]);
    ui.invoke_open_requested();
    assert!(
        pump_until(&app, || ui.get_status_label().contains(&failed_prefix)),
        "bad open did not report failure: {}",
        ui.get_status_label()
    );
    // Let the controller settle back to idle.
    assert!(
        pump_until(&app, || ui.get_can_open()),
        "host did not settle"
    );

    // Save As over the *good* (still-open) source path must be refused.
    ui.invoke_save_requested();
    let refused = zh("save.refused_source_path", &[]);
    assert!(
        pump_until(&app, || ui.get_status_label().as_str() == refused.as_str()),
        "source-path refusal timed out: {}",
        ui.get_status_label()
    );
    assert_eq!(
        std::fs::read(&good).unwrap(),
        good_bytes,
        "the opened source file must not be overwritten"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
