#![cfg(target_os = "linux")]
//! Desktop vector plot/export end to end.
//!
//! With an injected save path, open a committed synthetic fixture and export
//! SVG and PDF on the CPU vector path (no GPU); prove real vector content, an
//! unknown extension fails explicitly, and a cancelled dialog writes nothing and
//! reports the cancel. Software Vulkan offscreen only; the offscreen Slint
//! platform installs once per process, so a single test owns the whole flow.
use app_linux::{LinuxApp, LinuxOptions, SavePathProvider};
use slint::ComponentHandle;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

fn fixture() -> PathBuf {
    // Committed synthetic fixture: one paper-space layout with a line and A4/A3
    // plot settings, so the vector plot has real stroke geometry to assert.
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/plot/a4-layout.dwg")
}

/// The localized text for a key, so the test never hardcodes a language.
fn zh(key: &str, values: &[(&str, &str)]) -> String {
    cad_ui_slint::MessageSource::from_request("zh-CN").text(key, values)
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

enum Action {
    Save(PathBuf),
    Cancel,
}

/// Pump the shared renderer until `done` or a bounded timeout.
fn pump_until(app: &LinuxApp, mut done: impl FnMut() -> bool) -> bool {
    for _ in 0..200 {
        std::thread::sleep(std::time::Duration::from_millis(10));
        let _ = cad_ui_slint::offscreen::snapshot(app.adapter.window());
        if done() {
            return true;
        }
    }
    false
}

#[test]
fn plot_exports_real_vector_content_and_reports_cancel_and_bad_extension() {
    cad_ui_slint::offscreen::install().unwrap();
    let dir = PathBuf::from("/tmp/opencode").join(format!("yacr-plot-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let svg = dir.join("out.svg");
    let pdf = dir.join("out.pdf");
    let txt = dir.join("out.txt");

    // The injected save chooser reads the current action so the test can switch
    // target/cancel between invocations without a native dialog.
    let action = Arc::new(Mutex::new(Action::Save(svg.clone())));
    let injected = action.clone();
    let saver: SavePathProvider =
        Arc::new(move |_default: &str| match &*injected.lock().unwrap() {
            Action::Save(path) => Ok(path.clone()),
            Action::Cancel => Err(cad_domain::CadError::Cancelled),
        });

    let app = LinuxApp::new_with_providers(
        LinuxOptions {
            headless: true,
            drawing: Some(fixture()),
            ..LinuxOptions::default()
        },
        Arc::new(|| Err(cad_domain::CadError::Cancelled)),
        saver,
    )
    .unwrap();
    app.adapter.component().show().unwrap();
    let ui = app.adapter.component();

    assert!(ui.get_can_plot(), "desktop host must advertise plot");

    // SVG: real vector content, not just a prolog. The success status is either
    // the clean export or the partial one (never a dropped-diagnostics clean
    // success).
    ui.invoke_plot_requested();
    assert!(pump_until(&app, || svg.exists()), "SVG export timed out");
    let svg_bytes = std::fs::read(&svg).expect("SVG must be written");
    assert!(
        contains_bytes(&svg_bytes, b"<svg"),
        "SVG must contain the <svg> root element"
    );
    assert!(
        contains_bytes(&svg_bytes, b"<polyline")
            || contains_bytes(&svg_bytes, b"<polygon")
            || contains_bytes(&svg_bytes, b"<path"),
        "SVG must contain stroke geometry, not just a prolog"
    );
    let svg_str = svg.display().to_string();
    let clean = zh("plot.exported", &[("path", &svg_str)]);
    let status = ui.get_status_label().to_string();
    assert!(
        status.starts_with(&clean),
        "SVG status must report the export: {status}"
    );
    if status != clean {
        assert!(
            status.contains("部分内容未表达"),
            "a partial export must name the shortfall: {status}"
        );
    }

    // PDF: real content markers.
    *action.lock().unwrap() = Action::Save(pdf.clone());
    ui.invoke_plot_requested();
    assert!(pump_until(&app, || pdf.exists()), "PDF export timed out");
    let pdf_bytes = std::fs::read(&pdf).expect("PDF must be written");
    assert!(pdf_bytes.starts_with(b"%PDF"), "PDF must start with %PDF");
    assert!(
        contains_bytes(&pdf_bytes, b"stream") && contains_bytes(&pdf_bytes, b"xref"),
        "PDF must contain a content stream and an xref table"
    );

    // Unknown extension: explicit failure, no file.
    *action.lock().unwrap() = Action::Save(txt.clone());
    ui.invoke_plot_requested();
    let unknown = zh("plot.unknown_format", &[("ext", "txt")]);
    assert!(
        pump_until(&app, || ui.get_status_label().contains(&unknown)),
        "unknown-extension refusal timed out: {}",
        ui.get_status_label()
    );
    assert!(!txt.exists(), "an unknown extension must not be written");

    // Cancelled dialog: no write, explicit cancel (not a failure).
    *action.lock().unwrap() = Action::Cancel;
    ui.invoke_plot_requested();
    let cancelled = zh("plot.cancelled", &[]);
    assert!(
        pump_until(&app, || {
            ui.get_status_label().as_str() == cancelled.as_str()
        }),
        "cancel status timed out: {}",
        ui.get_status_label()
    );
    assert_ne!(
        ui.get_status_label().to_string(),
        zh("plot.export_failed", &[("reason", "")]),
        "a cancel must not be reported as a failure"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
