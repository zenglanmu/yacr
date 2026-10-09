//! macOS desktop binary (`yacr-macos`).
//!
//! A thin wrapper over the shared desktop host in `app_linux`: argument parsing,
//! the Slint/wgpu bridge and the native file chooser are identical to the Linux
//! `yacr-linux` and Windows `yacr` hosts, differing only in cfg-selected
//! platform details (NSOpenPanel chooser, `~/Library/Application Support`
//! config directory and the macOS system-font candidates).
//!
//! `scripts/package-macos-release.sh` stages this binary as the executable of an
//! `Yacr.app` bundle.

#[cfg(target_os = "macos")]
fn main() {
    if let Err(error) = app_linux::entry::run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("yacr-macos requires macOS");
    std::process::exit(1);
}
