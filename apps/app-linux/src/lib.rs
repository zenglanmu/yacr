//! Shared desktop host: the Slint/wgpu GUI for Linux (`yacr-linux`) and Windows
//! (`yacr`, in `apps/app-windows`).
//!
//! On every other target this library is empty, so cross-platform `--lib`
//! checks still validate workspace isolation rather than claiming support.
#[cfg(any(target_os = "linux", target_os = "windows"))]
mod host;
#[cfg(any(target_os = "linux", target_os = "windows"))]
pub use host::{config_file, FilePicker, LinuxApp, LinuxOptions};

/// Platform-neutral aliases for the shared desktop host.
#[cfg(any(target_os = "linux", target_os = "windows"))]
pub use host::{LinuxApp as DesktopApp, LinuxOptions as DesktopOptions};

/// Shared command-line entry point used by both desktop binaries.
#[cfg(any(target_os = "linux", target_os = "windows"))]
pub mod entry;
