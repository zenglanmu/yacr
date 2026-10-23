//! Desktop entry point.
//!
//! [`run`] is the real Linux host: it prepares the display backend policy
//! (`GDK_BACKEND`: explicit X11/Wayland, or an environment-derived `Auto`
//! choice) **before any GTK/Tauri initialization** and installs the pixel carrier
//! that policy selects (`native-below` for native Wayland, `dom-canvas` for
//! X11/XWayland, ADR 0024), then drives the single Tauri event loop (no second
//! loop is created). The process exit codes are preserved exactly (`2` for an
//! invalid or unavailable display backend policy, `1` for a failed Tauri build or
//! a refused startup contract) by resolving those cases here, where the original
//! `main` did. The `dom-canvas` runtime refusal (exit code `6`) is emitted by
//! [`crate::canvas_runtime`] once the WebView exists — it cannot be decided here.
//! Exit code `4` (X11/XWayland refused) is a **historical** contract kept in
//! [`crate::backend`]; nothing produces it any more.
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
    use crate::backend::{
        install_surface_mode, prepare_backend, BackendRefusal, SurfaceMode,
        UNSUPPORTED_BACKEND_EXIT_CODE,
    };

    // Parse the real launch options first: the stacking regression contract needs
    // the **explicit** drawing (`--open`/positional), and the `YACR_TEST_DWG`
    // fallback below must not be able to satisfy it.
    let mut launch = crate::host::LaunchOptions::from_args(std::env::args().skip(1));
    if std::env::var_os("YACR_PROBE_ONLY").is_some() {
        launch.probe_only = true;
    }
    // Pinned regression trigger: `YACR_PROBE_STACKING=1` + the init-script flag +
    // a real `--open` drawing. `--probe` (a fake open) is explicitly refused.
    crate::stacking_regression::validate(
        crate::stacking_regression::enabled(),
        launch.probe_only,
        launch.drawing.is_some(),
    )?;
    // Env-only fallback for real-DWG regression runs (never a fixture trigger).
    if launch.drawing.is_none() {
        if let Some(path) = std::env::var_os("YACR_TEST_DWG") {
            launch.drawing = Some(std::path::PathBuf::from(path));
        }
    }

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
            // The carrier is fixed here, once, before the webview exists (ADR
            // 0024 D4: no hot switching). Wayland keeps the native subsurface
            // path unchanged; X11/XWayland runs the wasm `dom-canvas` carrier.
            let surface_mode = plan.surface_mode();
            install_surface_mode(surface_mode);
            if surface_mode == SurfaceMode::DomCanvas {
                eprintln!(
                    "[probe] surfaceMode={} (wasm CAD session in the WebView owns the pixels; \
                     no native CAD surface)",
                    surface_mode.wire_name()
                );
                // The stacking-regression fixture is a **native-below** fixture: it
                // exists to prove the DOM sits above the native CAD surface. On this
                // carrier there is no such surface, so a run that asks for it is
                // refused instead of producing a frame that proves nothing.
                if crate::stacking_regression::enabled() {
                    eprintln!(
                        "error: YACR_PROBE_STACKING=1 is a native-below compositing fixture and \
                         is not available on surfaceMode=dom-canvas"
                    );
                    std::process::exit(1);
                }
            }
        }
        Err(BackendRefusal::Invalid(message)) => {
            eprintln!("error: {message}");
            std::process::exit(2);
        }
        Err(refusal @ BackendRefusal::Unsupported(_)) => {
            // Historical (ADR 0023 D3): nothing produces this refusal since ADR
            // 0024 stage ④ — X11 starts on `dom-canvas` instead. Kept so the
            // recorded exit-code-4 contract (marker line first) stays executable
            // and is never silently repurposed.
            if let Some(marker) = refusal.marker() {
                eprintln!("{marker}");
            }
            let messages = cad_ui_core::i18n::MessageSource::from_request(
                cad_ui_core::i18n::host_locale_tag(),
            );
            eprintln!("{}", messages.text("startup.unsupported_backend", &[]));
            eprintln!(
                "{}",
                messages.text("startup.unsupported_backend.detail", &[])
            );
            std::process::exit(UNSUPPORTED_BACKEND_EXIT_CODE);
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
        // Host-owned registration: the shared command list plus the `dom-canvas`
        // seam (carrier report + runtime refusal). On `native-below` it forwards
        // every command to the shared handler unchanged.
        .invoke_handler(crate::host::invoke_handler())
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
            // Trigger ④ of the popup lifecycle: the host process is exiting, so
            // no modal gate may stay set.
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
    // Propagate the bounded post-load driver's requested status to the OS. The
    // event loop's `AppHandle::exit(code)` was observed to leave the OS status 0,
    // so the recorded code is applied here, after the loop and teardown. A
    // default/manual run (no request) or a passing driver is `0` and returns
    // normally.
    let exit = crate::host::resolve_process_exit(crate::host::requested_exit_code(&telemetry));
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
