//! Windows desktop binary (`yacr`).
//!
//! A thin wrapper over the shared desktop host in `app_linux`: argument parsing,
//! the Slint/wgpu bridge and the native file chooser are identical to the Linux
//! `yacr-linux` host, differing only in cfg-selected platform details.

#[cfg(target_os = "windows")]
fn main() {
    if let Err(error) = app_linux::entry::run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("yacr requires Windows");
    std::process::exit(1);
}
