//! Linux host. Non-Linux library checks validate workspace isolation, not platform support.
#[cfg(target_os = "linux")]
mod host;
#[cfg(target_os = "linux")]
pub use host::{LinuxApp, LinuxOptions};
