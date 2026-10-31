//! Native macOS desktop host for the CAD viewer/measure/annotation app.
//!
//! The host is a **transparent WKWebView (React chrome) above an in-process
//! native CAD child `NSView`**, the same composition direction as the Linux and
//! Windows hosts (ADR 0023): the DOM/WebView layer is above in **both** pixels and
//! input, and the CAD view is a **display-only** `NSView` subview (added
//! `positioned: .below`) whose renderer surface is a Metal swapchain driven by
//! [`cad_render_wgpu::SurfaceProbePresenter`]. The child view is sized from the
//! React-reported CAD rect through `cad_ui_core::layout`, never from the window
//! and never from a readback.
//!
//! Composition contract:
//! - the DOM/WebView layer owns every pixel above the CAD view and is the **single
//!   input funnel** (`.native-input` → `probe_input` IPC → host queue); the CAD
//!   `NSView` installs no responder overrides and no gesture recognizers;
//! - the CAD view is created **hidden**, laid out from the accepted DOM rect and
//!   shown only once the rect is confirmed;
//! - there is no second window, no second webview and no document state in the
//!   native shell beyond the single [`cad_ui_core::session::CadSession`].
//!
//! The platform-independent half (wire protocol, logical layout, CAD session,
//! shell admission/error state, i18n catalogs) lives in `cad-ui-core`; the Tauri
//! command/state layer lives in `cad-desktop-shell`. Both are imported directly,
//! with no compatibility shim.
//!
//! **macOS only**: `tauri`/`wry`/`tao`, the Objective-C bindings and the Metal
//! presenter are `target_os = "macos"` dependencies, so every module except
//! [`entry`] is gated. [`entry`] is target-independent on purpose: its `run` body
//! is `cfg`-gated and the non-macOS branch is an explicit NOT IMPLEMENTED refusal,
//! so a Linux/Windows workspace check type-checks the refusal instead of skipping
//! it (and its unit test runs on those hosts).
//!
//! Verification reality: only compilation is claimed (`cargo check -p app-macos
//! --target aarch64-apple-darwin --all-targets` and the CI `macos-check` gate).
//! Runtime, visual, Metal, DPI and native-dialog behaviour are **NOT VERIFIED**.

pub mod entry;

#[cfg(target_os = "macos")]
pub mod host;
#[cfg(target_os = "macos")]
pub mod macos_surface;
#[cfg(target_os = "macos")]
pub mod main_thread;
#[cfg(target_os = "macos")]
pub mod platform;
#[cfg(target_os = "macos")]
pub mod redraw;

pub use entry::run;
