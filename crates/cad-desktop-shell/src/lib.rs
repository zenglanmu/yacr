//! Desktop host shell shared by the Tauri apps.
//!
//! `cad-desktop-shell` is the **only** shared layer allowed to depend on
//! `tauri`. It sits above `cad-ui-core` and below the concrete `apps/app-*`
//! hosts, and owns:
//!
//! - [`state`]: the shell half of the host's shared state ([`state::Shared`]),
//!   the [`state::ShellTelemetry`] and [`state::ShellPopupHost`] platform seams,
//!   and the frozen lock-order invariant (`Shared` before render telemetry).
//! - [`commands`]: every `#[tauri::command]` IPC function plus the single
//!   `generate_handler!` registration (`commands::register_commands`).
//! - [`popup`]: the shared modal-popup lifecycle (gate mapping and the
//!   four-trigger clear contract); popups are in-DOM overlays, so it owns no
//!   window.
//! - [`threading`]: the shared main-thread marshalling (probe + dispatch seam),
//!   so every desktop host reuses one channel protocol instead of copying it.
//!
//! It is deliberately **toolkit-, portal- and GPU-free**: no
//! `gtk`/`gdk`/`gdkx11`/`ashpd`/`wgpu`/`cad-render-wgpu`/`cad-engine` (enforced
//! by `scripts/check-architecture.py`). Window creation, the GTK overlay
//! bridge and renderer telemetry stay in the concrete host.
//!
//! **Target gating**: `tauri` is a Linux/Windows-target dependency, and every
//! module is compiled out elsewhere. On wasm the crate is an empty shell, so
//! `cargo check --workspace --lib --target wasm32-unknown-unknown` keeps
//! validating isolation.

#[cfg(any(target_os = "linux", target_os = "windows"))]
pub mod commands;
#[cfg(any(target_os = "linux", target_os = "windows"))]
pub mod popup;
#[cfg(any(target_os = "linux", target_os = "windows"))]
pub mod state;
#[cfg(any(target_os = "linux", target_os = "windows"))]
pub mod threading;

#[cfg(any(target_os = "linux", target_os = "windows"))]
pub use commands::{register_commands, CadCommandAck, CadCommandAlias, COMMAND_NAMES};
#[cfg(any(target_os = "linux", target_os = "windows"))]
pub use popup::DesktopPopupTarget;
#[cfg(any(target_os = "linux", target_os = "windows"))]
pub use state::{lock, new_shared, ProbeShared, Shared, ShellPopupHost, ShellTelemetry};
