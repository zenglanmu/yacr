//! Desktop entry point.
//!
//! [`run`] is the real Linux host: it prepares the display backend policy
//! (`GDK_BACKEND`: explicit X11/Wayland, or an environment-derived `Auto`
//! choice) **before any GTK/Tauri initialization**, then drives the single Tauri
//! event loop (no second loop is created). The process exit codes are preserved
//! exactly (`2` for an unsupported or unavailable display backend, `1` for a
//! failed Tauri build) by resolving those cases here, where the original `main`
//! did.
//!
//! Every non-Linux target gets an explicit NOT IMPLEMENTED error instead of an
//! silently empty GUI: none of the `tauri`/GTK/renderer dependencies are active
//! there, so this module is the only thing that compiles.

/// Run the desktop host.
///
/// Linux runs the real host and only returns after the event loop ends. A
/// non-Linux target returns an explicit NOT IMPLEMENTED error (the binary exits
/// non-zero).
#[cfg(target_os = "linux")]
pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    use crate::backend::prepare_backend;

    // Must run before any GTK/Tauri initialization so the resolved backend is
    // fixed and GTK never falls back on its own.
    match prepare_backend() {
        Ok(plan) => {
            let source = if plan.exports_gdk_backend {
                "exported by host"
            } else {
                "from environment"
            };
            eprintln!(
                "[probe] display backend policy={:?} resolved={:?} source={source}",
                plan.policy, plan.resolved
            );
        }
        Err(message) => {
            eprintln!("error: {message}");
            std::process::exit(2);
        }
    }

    let mut launch = crate::host::LaunchOptions::from_args(std::env::args().skip(1));
    if launch.drawing.is_none() {
        if let Some(path) = std::env::var_os("YACR_TEST_DWG") {
            launch.drawing = Some(std::path::PathBuf::from(path));
        }
    }
    if std::env::var_os("YACR_PROBE_ONLY").is_some() {
        launch.probe_only = true;
    }
    if let Some(path) = &launch.drawing {
        eprintln!("[probe] startup drawing: {}", path.display());
    }
    let shared = crate::host::new_shared(launch);
    let setup_shared = shared.clone();
    let run_shared = shared.clone();

    let app = tauri::Builder::default()
        .manage(crate::host::ProbeShared(shared.clone()))
        .invoke_handler(tauri::generate_handler![
            crate::host::probe_status,
            crate::host::probe_input,
            crate::host::cad_command,
            crate::host::cad_status
        ])
        .setup(move |app| {
            crate::host::init(app, &setup_shared)?;
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
        tauri::RunEvent::MainEventsCleared => crate::host::tick(&run_shared),
        tauri::RunEvent::ExitRequested { .. } => {
            eprintln!("[probe] run event: ExitRequested");
            crate::host::close_presenter();
            crate::host::request_stop(&run_shared);
        }
        tauri::RunEvent::Exit => {
            eprintln!("[probe] run event: Exit");
            crate::host::close_presenter();
            crate::host::request_stop(&run_shared);
        }
        _ => {}
    });
    // Propagate the bounded post-load driver's requested status to the OS. The
    // event loop's `AppHandle::exit(code)` was observed to leave the OS status 0,
    // so the recorded code is applied here, after the loop and teardown. A
    // default/manual run (no request) or a passing driver is `0` and returns
    // normally.
    let exit = crate::host::resolve_process_exit(crate::host::requested_exit_code(&shared));
    if exit != 0 {
        std::process::exit(exit);
    }
    Ok(())
}

/// Non-Linux targets: the desktop host is Linux only (X11/XWayland or native
/// Wayland). Return an explicit error so the binary exits non-zero rather than
/// pretending it can run.
#[cfg(not(target_os = "linux"))]
pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    // No `tauri`/GTK/renderer dependency is active on this target.
    Err("yacr: the desktop host is Linux only; this platform is NOT IMPLEMENTED".into())
}
