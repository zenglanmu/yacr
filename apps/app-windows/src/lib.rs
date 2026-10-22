//! Native Windows desktop host for the CAD viewer/measure/annotation app.
//!
//! The host is a **transparent WebView2 (React chrome) above an in-process native
//! CAD child HWND**: the single Tauri event loop drives
//! `cad-render-wgpu::SurfaceProbePresenter` on a `WS_CHILD` window created with
//! hand-rolled Win32, and the renderer surface is built from a
//! `Win32WindowHandle` / `WindowsDisplayHandle` pair (no renderer changes).
//!
//! Composition contract (the same model as the Linux host, DOM-on-top):
//! - the DOM/WebView layer is above in **both** pixels and input;
//! - the CAD child is a **display-only** surface (`WS_DISABLED`), sized from the
//!   React-reported CAD rect through `cad_ui_core::layout::LayoutTracker`;
//! - input has one funnel: `.native-input` → `probe_input` IPC → host queue. The
//!   host never asks Windows for click-through and never uses `WS_EX_LAYERED`.
//!
//! The platform-independent half (wire protocol, logical layout, CAD session,
//! shell admission/error state, i18n catalogs) lives in `cad-ui-core`; the Tauri
//! command/state layer lives in `cad-desktop-shell`. Both are imported directly,
//! with no compatibility shim.
//!
//! **Windows only**: `tauri`/`wry`/`tao`, WebView2 and the Win32 APIs are
//! Windows-target dependencies, so this crate is an empty shell on every other
//! target and a workspace wasm (or other-target) `--lib` check validates
//! isolation rather than pretending the host can run there. The binary returns an
//! explicit NOT IMPLEMENTED error elsewhere ([`entry::run`]).
//!
//! [`support`] is deliberately target-independent: it holds the WebView2
//! missing-runtime refusal contract (marker, exit code, localized copy) and the
//! pinned transparent-background alpha, so its unit tests run on any host.

pub mod entry;
pub mod support;

#[cfg(target_os = "windows")]
pub mod host;
#[cfg(target_os = "windows")]
mod main_thread;
#[cfg(target_os = "windows")]
pub mod platform;
#[cfg(target_os = "windows")]
pub mod redraw;
#[cfg(target_os = "windows")]
pub mod win32_surface;
