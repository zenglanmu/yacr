//! Shared entry point for the desktop hosts.
//!
//! `yacr-linux` (Linux) and `yacr` (Windows, `apps/app-windows`) both call this
//! so argument parsing, renderer selection and the headless acceptance path
//! cannot drift apart between the two binaries.

use crate::LinuxOptions;

/// Parse arguments, select the renderer and run the host.
pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let options = LinuxOptions::parse(std::env::args().skip(1))?;
    if options.headless {
        cad_ui_slint::offscreen::install()?;
    } else {
        cad_ui_slint::select_wgpu_backend_with(options.gpu)?;
    }
    let app = crate::LinuxApp::new(options.clone())?;
    if options.headless {
        app.acceptance(options.output.as_deref().expect("validated output"))?;
        Ok(())
    } else {
        app.run()?;
        Ok(())
    }
}
