//! Linux desktop binary (`yacr-linux`). Windows uses `yacr` in `app-windows`
//! and macOS uses `yacr-macos` in `app-macos`; all three delegate to the shared
//! `app_linux::entry::run`.

#[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
fn main() {
    if let Err(error) = app_linux::entry::run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
fn main() {
    eprintln!("yacr-linux requires Linux, Windows or macOS");
    std::process::exit(1);
}
