//! `yacr-react-probe` entry point (Linux/X11 only).
//!
//! Requires `GDK_BACKEND=x11` (enforced or set before GTK starts). The probe is
//! driven from the single Tauri event loop; no second event loop is created.

#[cfg(target_os = "linux")]
fn main() {
    match app_react_linux::enforce_x11_backend() {
        Ok(app_react_linux::BackendDecision::AlreadyX11) => {
            eprintln!("[probe] GDK_BACKEND=x11 already set");
        }
        Ok(app_react_linux::BackendDecision::EnforcedX11) => {
            eprintln!("[probe] GDK_BACKEND was unset; enforced GDK_BACKEND=x11");
        }
        Err(message) => {
            eprintln!("error: {message}");
            std::process::exit(2);
        }
    }

    let mut launch = app_react_linux::host::LaunchOptions::from_args(std::env::args().skip(1));
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
    let shared = app_react_linux::host::new_shared(launch);
    let setup_shared = shared.clone();
    let run_shared = shared.clone();

    let app = tauri::Builder::default()
        .manage(app_react_linux::host::ProbeShared(shared))
        .invoke_handler(tauri::generate_handler![
            app_react_linux::host::probe_status,
            app_react_linux::host::probe_input,
            app_react_linux::host::cad_command,
            app_react_linux::host::cad_status
        ])
        .setup(move |app| {
            app_react_linux::host::init(app, &setup_shared)?;
            Ok(())
        })
        .build(tauri::generate_context!())
        .unwrap_or_else(|error| {
            eprintln!("error: failed to build the tauri app: {error}");
            std::process::exit(1);
        });

    // Single event loop: the probe is ticked on `MainEventsCleared`. The surface
    // is explicitly released before the window/loop teardown (surface before
    // window); `Destroyed` arrives too late and has no tick afterwards.
    app.run(move |_app_handle, event| match event {
        tauri::RunEvent::MainEventsCleared => app_react_linux::host::tick(&run_shared),
        tauri::RunEvent::ExitRequested { .. } => {
            eprintln!("[probe] run event: ExitRequested");
            app_react_linux::host::close_presenter();
            app_react_linux::host::request_stop(&run_shared);
        }
        tauri::RunEvent::Exit => {
            eprintln!("[probe] run event: Exit");
            app_react_linux::host::close_presenter();
            app_react_linux::host::request_stop(&run_shared);
        }
        _ => {}
    });
}

/// Non-Linux targets: the probe is a Linux/X11 experiment. Exit explicitly
/// rather than pretend it can run.
#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("yacr-react-probe is a Linux/X11 experiment; this platform is not supported");
    std::process::exit(1);
}
