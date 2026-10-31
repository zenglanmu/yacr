//! Desktop entry point.
//!
//! [`run`] is the real macOS host: it parses the shared launch options, builds the
//! single Tauri application (transparent window + transparent child WKWebView +
//! the native CAD `NSView`) and drives one event loop.
//!
//! Unlike the Windows host there is **no runtime pre-check**: WKWebView and Metal
//! ship with the OS, so there is no "missing runtime" refusal to make. What stays
//! identical is the loop contract — the host is ticked on
//! `RunEvent::MainEventsCleared`, and the CAD surface is stopped and released
//! (surface before window) on `ExitRequested`/`Exit`.
//!
//! Every non-macOS target gets an explicit NOT IMPLEMENTED error instead of a
//! silently empty GUI: none of the `tauri`/WKWebView/renderer dependencies are
//! active there, so the refusal is the only thing that compiles.

/// Run the desktop host.
///
/// macOS runs the real host and only returns after the event loop ends. A
/// non-macOS target returns an explicit NOT IMPLEMENTED error (the binary exits
/// non-zero).
#[cfg(target_os = "macos")]
pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut launch = cad_ui_core::launch::LaunchOptions::from_args(std::env::args().skip(1));
    if std::env::var_os("YACR_PROBE_ONLY").is_some() {
        launch.probe_only = true;
    }
    // Env-only fallback for real-DWG regression runs (never a fixture trigger).
    if launch.drawing.is_none() {
        if let Some(path) = std::env::var_os("YACR_TEST_DWG") {
            launch.drawing = Some(std::path::PathBuf::from(path));
        }
    }
    if let Some(path) = &launch.drawing {
        eprintln!("[probe] startup drawing: {}", path.display());
    }

    let shared = crate::host::new_shared(launch);
    let telemetry = crate::host::new_render_telemetry();
    let setup_shared = shared.clone();
    let setup_telemetry = telemetry.clone();
    let run_shared = shared.clone();
    let run_telemetry = telemetry.clone();

    let app = tauri::Builder::default()
        .manage(crate::host::ProbeShared(shared.clone()))
        .invoke_handler(cad_desktop_shell::commands::register_commands())
        .setup(move |app| {
            crate::host::init(app, &setup_shared, &setup_telemetry)?;
            Ok(())
        })
        .build(tauri::generate_context!())
        .unwrap_or_else(|error| {
            eprintln!("error: failed to build the tauri app: {error}");
            std::process::exit(1);
        });

    // Single event loop: the host is ticked on `MainEventsCleared`. The surface
    // is explicitly stopped and released before the window/loop teardown (surface
    // before view); `Destroyed` arrives too late and has no tick afterwards.
    app.run(move |_app_handle, event| match event {
        tauri::RunEvent::MainEventsCleared => crate::host::tick(&run_shared, &run_telemetry),
        tauri::RunEvent::ExitRequested { .. } => {
            eprintln!("[probe] run event: ExitRequested");
            // Trigger ④ of the popup lifecycle: the process is exiting, so no
            // modal gate may stay set.
            cad_desktop_shell::popup::on_process_exit(&run_shared);
            crate::host::close_presenter();
            crate::host::request_stop(&run_shared, &run_telemetry);
        }
        tauri::RunEvent::Exit => {
            eprintln!("[probe] run event: Exit");
            cad_desktop_shell::popup::on_process_exit(&run_shared);
            crate::host::close_presenter();
            crate::host::request_stop(&run_shared, &run_telemetry);
        }
        _ => {}
    });
    Ok(())
}

/// Non-macOS targets: the desktop host is macOS only. Return an explicit error so
/// the binary exits non-zero rather than pretending it can run.
#[cfg(not(target_os = "macos"))]
pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    // No `tauri`/WKWebView/renderer dependency is active on this target.
    Err(
        "yacr-macos: the macOS desktop host is built only for macOS targets; \
         this platform is NOT IMPLEMENTED"
            .into(),
    )
}

#[cfg(all(test, not(target_os = "macos")))]
mod tests {
    /// The non-macOS entry must stay an explicit, non-zero refusal instead of a
    /// silently empty GUI.
    #[test]
    fn a_non_macos_target_refuses_explicitly() {
        let error = super::run().expect_err("non-macOS run must fail");
        let text = error.to_string();
        assert!(text.contains("NOT IMPLEMENTED"), "{text}");
        assert!(text.starts_with("yacr-macos:"), "{text}");
    }
}
