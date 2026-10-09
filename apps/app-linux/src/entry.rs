//! Shared entry point for the desktop hosts.
//!
//! `yacr-linux` (Linux), `yacr` (Windows, `apps/app-windows`) and `yacr-macos`
//! (macOS, `apps/app-macos`) all call this so argument parsing, renderer
//! selection and the headless acceptance path cannot drift apart between the
//! desktop binaries.

use crate::LinuxOptions;

/// Parse arguments, select the renderer and run the host.
pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let options = LinuxOptions::parse(std::env::args().skip(1))?;
    if options.headless {
        // The offscreen validation platform drives a software Vulkan (lavapipe)
        // device; it is a Linux/Windows acceptance path. macOS has no Vulkan, so
        // `--headless` is an explicit unsupported capability there rather than a
        // silently broken build.
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        {
            cad_ui_slint::offscreen::install()?;
            let app = crate::LinuxApp::new(options.clone())?;
            app.acceptance(options.output.as_deref().expect("validated output"))?;
            return Ok(());
        }
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        {
            return Err("--headless offscreen validation requires software Vulkan \
                 (Linux/Windows) and is not supported on macOS"
                .into());
        }
    }
    cad_ui_slint::select_wgpu_backend_with(options.gpu)?;
    let app = crate::LinuxApp::new(options.clone())?;
    app.run()?;
    Ok(())
}
