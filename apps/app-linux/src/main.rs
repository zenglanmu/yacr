#[cfg(target_os = "linux")]
fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

#[cfg(target_os = "linux")]
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let options = app_linux::LinuxOptions::parse(std::env::args().skip(1))?;
    if options.headless {
        cad_ui_slint::offscreen::install()?;
    } else {
        cad_ui_slint::select_wgpu_backend()?;
    }
    let app = app_linux::LinuxApp::new(options.clone())?;
    if options.headless {
        app.acceptance(options.output.as_deref().expect("validated output"))?;
        Ok(())
    } else {
        app.run()?;
        Ok(())
    }
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("yacr-linux requires Linux");
    std::process::exit(1);
}
