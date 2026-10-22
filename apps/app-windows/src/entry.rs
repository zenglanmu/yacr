//! Desktop entry point.
//!
//! [`run`] is the real Windows host. It parses the shared launch options, then
//! checks for a usable **WebView2 runtime before any window exists**. A missing
//! runtime is an explicit unsupported-runtime refusal: a stable marker line, the
//! localized catalog text and [`crate::support::UNSUPPORTED_WEBVIEW2_EXIT_CODE`]
//! — never a blank window and never a silent fallback.
//!
//! The single Tauri event loop is then driven exactly as on Linux: the host is
//! ticked on `MainEventsCleared`, and the CAD surface is released before the
//! window/loop teardown on `ExitRequested`/`Exit`.
//!
//! Every non-Windows target gets an explicit NOT IMPLEMENTED error instead of an
//! silently empty GUI: none of the `tauri`/WebView2/renderer dependencies are
//! active there, so this module is the only thing that compiles.

/// Run the desktop host.
///
/// Windows runs the real host and only returns after the event loop ends. A
/// non-Windows target returns an explicit NOT IMPLEMENTED error (the binary exits
/// non-zero).
#[cfg(target_os = "windows")]
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

    // The React chrome *is* a WebView2 document: no runtime = no UI at all.
    // Refuse before creating a window, with the stable marker first so evidence
    // collection can classify on that line, then the localized copy.
    match crate::support::ensure_runtime() {
        Ok(info) => eprintln!("{}", info.diagnostic()),
        Err(missing) => {
            eprintln!("{}", crate::support::UNSUPPORTED_WEBVIEW2_MARKER);
            eprintln!("[probe] webview2 runtime missing: {}", missing.detail);
            let (title, detail) =
                crate::support::missing_messages(cad_ui_core::i18n::host_locale_tag());
            eprintln!("{title}");
            eprintln!("{detail}");
            std::process::exit(crate::support::UNSUPPORTED_WEBVIEW2_EXIT_CODE);
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
    // is explicitly released before the window/loop teardown (surface before
    // window); `Destroyed` arrives too late and has no tick afterwards.
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

/// Non-Windows targets: the desktop host is Windows only. Return an explicit
/// error so the binary exits non-zero rather than pretending it can run.
#[cfg(not(target_os = "windows"))]
pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    // No `tauri`/WebView2/renderer dependency is active on this target.
    Err(
        "yacr-windows: the Windows desktop host is built only for windows targets; \
         this platform is NOT IMPLEMENTED"
            .into(),
    )
}

#[cfg(all(test, not(target_os = "windows")))]
mod tests {
    /// The non-Windows entry must stay an explicit, non-zero refusal instead of a
    /// silently empty GUI.
    #[test]
    fn a_non_windows_target_refuses_explicitly() {
        let error = super::run().expect_err("non-Windows run must fail");
        let text = error.to_string();
        assert!(text.contains("NOT IMPLEMENTED"), "{text}");
        assert!(text.starts_with("yacr-windows:"), "{text}");
    }
}
