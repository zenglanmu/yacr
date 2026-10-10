#![cfg(target_os = "linux")]
//! Desktop lossy Save As end to end.
//!
//! With an injected save path, open a committed fixture and save it as DXF on
//! the CPU path (no GPU); prove real DXF content, that `.dwg` is refused, that
//! saving over the opened source is refused, and that a cancelled dialog writes
//! nothing. Software Vulkan offscreen only; the offscreen Slint platform
//! installs once per process, so a single test owns the whole flow.
use app_linux::{LinuxApp, LinuxOptions, SavePathProvider};
use slint::ComponentHandle;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/dxf/qcad-flange/flange.dxf")
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
fn save_as_writes_dxf_and_refuses_dwg_source_and_cancel() {
    cad_ui_slint::offscreen::install().unwrap();
    let dir = PathBuf::from("/tmp/opencode").join(format!("yacr-save-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // Work on a copy so the committed fixture is never a write target.
    let source = dir.join("source.dxf");
    std::fs::copy(fixture(), &source).unwrap();
    let source_bytes = std::fs::read(&source).unwrap();
    let out_dxf = dir.join("out.dxf");
    let out_dwg = dir.join("out.dwg");
    let out_no_ext = dir.join("out");
    let out_txt = dir.join("out.txt");

    let action = Arc::new(Mutex::new(Action::Save(out_dxf.clone())));
    let injected = action.clone();
    let saver: SavePathProvider =
        Arc::new(move |_default: &str| match &*injected.lock().unwrap() {
            Action::Save(path) => Ok(path.clone()),
            Action::Cancel => Err(cad_domain::CadError::Cancelled),
        });

    let app = LinuxApp::new_with_providers(
        LinuxOptions {
            headless: true,
            drawing: Some(source.clone()),
            ..LinuxOptions::default()
        },
        Arc::new(|| Err(cad_domain::CadError::Cancelled)),
        saver,
    )
    .unwrap();
    app.adapter.component().show().unwrap();
    let ui = app.adapter.component();

    assert!(ui.get_can_save(), "desktop host must advertise Save As");

    // Save as DXF: real content, graded status (exact or partial per the report).
    ui.invoke_save_requested();
    assert!(pump_until(&app, || out_dxf.exists()), "DXF save timed out");
    let bytes = std::fs::read(&out_dxf).expect("DXF must be written");
    assert!(!bytes.is_empty(), "DXF must not be empty");
    assert!(
        contains_bytes(&bytes, b"SECTION"),
        "DXF must contain SECTION"
    );
    assert!(
        contains_bytes(&bytes, b"ENTITIES"),
        "DXF must contain ENTITIES"
    );
    let dxf_str = out_dxf.display().to_string();
    let clean = zh("save.exported", &[("path", &dxf_str)]);
    let status = ui.get_status_label().to_string();
    assert!(
        status.starts_with(&clean),
        "save status must report the file: {status}"
    );
    if status != clean {
        // Derive the partial wording from the catalog (never a hardcoded word).
        // Counts are unknown, so check the literal fragments around the
        // placeholders by rendering with sentinels.
        let partial = zh(
            "save.exported_partial",
            &[
                ("path", &dxf_str),
                ("converted", "\u{1}"),
                ("dropped", "\u{2}"),
            ],
        );
        for fragment in partial.split(['\u{1}', '\u{2}']) {
            assert!(
                status.contains(fragment),
                "a partial save must use the catalog wording: {status}"
            );
        }
    }

    // `.dwg` is refused explicitly and writes nothing.
    *action.lock().unwrap() = Action::Save(out_dwg.clone());
    ui.invoke_save_requested();
    let dwg_refused = zh("save.dwg_deferred", &[]);
    assert!(
        pump_until(&app, || ui.get_status_label().as_str()
            == dwg_refused.as_str()),
        "DWG refusal timed out: {}",
        ui.get_status_label()
    );
    assert!(
        !out_dwg.exists(),
        "a refused DWG target must not be written"
    );

    // No extension and an unknown extension are explicit failures, no file.
    *action.lock().unwrap() = Action::Save(out_no_ext.clone());
    ui.invoke_save_requested();
    let no_extension = zh("save.no_extension", &[]);
    assert!(
        pump_until(&app, || ui.get_status_label().contains(&no_extension)),
        "no-extension refusal timed out: {}",
        ui.get_status_label()
    );
    assert!(
        !out_no_ext.exists(),
        "a no-extension target must not be written"
    );

    *action.lock().unwrap() = Action::Save(out_txt.clone());
    ui.invoke_save_requested();
    let unknown_format = zh("save.unknown_format", &[("ext", "txt")]);
    assert!(
        pump_until(&app, || ui.get_status_label().contains(&unknown_format)),
        "unknown-format refusal timed out: {}",
        ui.get_status_label()
    );
    assert!(
        !out_txt.exists(),
        "an unknown-format target must not be written"
    );

    // Saving over the opened source file is refused and leaves it untouched.
    *action.lock().unwrap() = Action::Save(source.clone());
    ui.invoke_save_requested();
    let refused = zh("save.refused_source_path", &[]);
    assert!(
        pump_until(&app, || ui.get_status_label().as_str() == refused.as_str()),
        "source-path refusal timed out: {}",
        ui.get_status_label()
    );
    assert_eq!(
        std::fs::read(&source).unwrap(),
        source_bytes,
        "the opened source file must not be overwritten"
    );

    // A cancelled dialog writes nothing and is a cancel, not a failure. Compare
    // the whole directory listing before/after, and re-assert the source bytes.
    let listing = || {
        let mut names: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    };
    let before = listing();
    *action.lock().unwrap() = Action::Cancel;
    ui.invoke_save_requested();
    let cancelled = zh("save.cancelled", &[]);
    assert!(
        pump_until(&app, || ui.get_status_label().as_str()
            == cancelled.as_str()),
        "cancel status timed out: {}",
        ui.get_status_label()
    );
    assert_eq!(listing(), before, "a cancel must write nothing");
    assert_eq!(
        std::fs::read(&source).unwrap(),
        source_bytes,
        "a cancel must not touch the source file"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
