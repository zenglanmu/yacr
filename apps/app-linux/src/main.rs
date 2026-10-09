//! Linux desktop binary (`yacr-linux`). Windows uses `yacr` in `app-windows`;
//! both delegate to the shared `app_linux::entry::run`.

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn main() {
    if let Err(error) = app_linux::entry::run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
fn main() {
    eprintln!("yacr-linux requires Linux or Windows");
    std::process::exit(1);
}
