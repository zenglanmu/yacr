//! Native Linux desktop host for the CAD viewer/measure/annotation app.
//!
//! The host is a **React (Vite) chrome in a Tauri 2 webview** composited over a
//! native GTK/wgpu CAD surface: the single Tauri event loop drives
//! `cad-render-wgpu::SurfaceProbePresenter` on a windowed `GtkDrawingArea` inside
//! a `GtkOverlay` (the webview is a transparent sibling **underneath** the CAD
//! area, which is the topmost overlay child; input passes through the CAD rect), and exposes a
//! small status/input protocol to the frontend.
//!
//! The platform-independent half (wire protocol, logical layout, CAD session,
//! shell admission/error state, i18n catalogs) lives in `cad-ui-core` and is
//! imported directly. `host::*` exposes only thin convenience re-exports
//! (`host::LaunchOptions`, `host::cad_status`, …) consumed inside this crate —
//! aliases for app-internal call sites, not a compatibility shim layer.
//!
//! **Linux only** (GTK X11/XWayland **or** native Wayland; the native CAD
//! partition is an X11 child window on X11 and an app-owned Wayland
//! `wl_subsurface` on Wayland). On every other target this crate's GUI modules
//! are empty (no `tauri`/`wry`/`tao`, no GTK, no renderer), so the binary is an
//! explicit NOT IMPLEMENTED stub ([`entry::run`] returns an error). A
//! workspace-wide wasm (or other-target) `--lib` check therefore validates
//! isolation instead of pretending the host can run there.

#[cfg(target_os = "linux")]
pub mod backend;
#[cfg(target_os = "linux")]
pub mod evidence;
#[cfg(target_os = "linux")]
pub mod gtk_bridge;
#[cfg(target_os = "linux")]
pub mod host;
#[cfg(target_os = "linux")]
pub mod platform;
#[cfg(target_os = "linux")]
pub mod redraw;
#[cfg(target_os = "linux")]
mod stacking_regression;
#[cfg(target_os = "linux")]
pub mod wayland_surface;

#[cfg(target_os = "linux")]
pub use backend::{
    parse_backend_policy, prepare_backend, BackendPlan, BackendPolicy, ResolvedBackend,
};

/// The desktop entry point: Linux runs the real host, every other target is an
/// explicit NOT IMPLEMENTED error (see [`entry::run`]).
pub mod entry;
