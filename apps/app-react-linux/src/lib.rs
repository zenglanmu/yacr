//! React (Tauri 2) native Linux probe host — experiment, see
//! `docs/adr/0006-react-native-linux-experiment.md`.
//!
//! This crate is a **probe**, not a CAD product and not the production CAD
//! presenter. It forces the X11 backend, drives
//! `cad-render-wgpu::SurfaceProbePresenter` on a transparent Tauri window from
//! the Tauri event loop, and exposes a small status/input protocol to the React
//! frontend.
//!
//! **Linux/X11 only.** On every other target this crate is deliberately empty:
//! no `tauri`/`wry`/`tao` code is compiled and the binary exits with an explicit
//! unsupported message. A workspace-wide wasm (or other-target) `--lib` check
//! therefore validates isolation instead of pretending the probe can run there.

#[cfg(target_os = "linux")]
pub mod backend;
#[cfg(target_os = "linux")]
pub mod cad;
#[cfg(target_os = "linux")]
pub mod evidence;
#[cfg(target_os = "linux")]
pub mod gtk_bridge;
#[cfg(target_os = "linux")]
pub mod host;
#[cfg(target_os = "linux")]
pub mod layout;
#[cfg(target_os = "linux")]
pub mod platform;
#[cfg(target_os = "linux")]
pub mod protocol;
#[cfg(target_os = "linux")]
pub mod redraw;

#[cfg(target_os = "linux")]
pub use backend::{check_gdk_backend, enforce_x11_backend, BackendDecision};
#[cfg(target_os = "linux")]
pub use protocol::{InputKind, InputTracker, ProbeInput, ProbeInputAck, ProbeStatus};
