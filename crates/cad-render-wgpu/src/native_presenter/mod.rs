//! Native-only **surface probe** presenter for host bring-up.
//!
//! This is deliberately **not** the production CAD presenter. It proves that a
//! host (a Tauri window, or an X11 child handle) can hand a window to the
//! renderer crate, get a real swapchain back, render onto it with wgpu, and
//! present every tick on a real hardware GPU. To make that unambiguous the probe
//! shader draws only a recognisable grid, a translating colour block and two
//! corner markers — never anything that could be mistaken for CAD geometry.
//!
//! Contract (spec §5.2/§5.3; `docs/adr/0002-gpu-ui-composition.md`):
//! - Native only (`#[cfg(not(target_arch = "wasm32"))]`); the browser path gets
//!   its device from the host canvas instead.
//! - All `wgpu` instance/device/pass/submit/present work lives in this module.
//!   `wgpu` handle objects never escape it; only plain data (adapter
//!   description, texture format, size, present mode) is exposed.
//! - The instance forces `Backends::VULKAN`, and adapter selection rejects
//!   `cpu` (software/fallback) adapters. A preferred adapter *name* (default
//!   `P620`) is only a preference, never a filter: any Vulkan hardware adapter
//!   that can present to the surface is accepted.
//! - No GPU→CPU readback is ever performed.
//! - The renderer never owns an event loop; the host owns the single loop and
//!   calls [`SurfaceProbePresenter::frame`] once per tick.
//! - Lifecycle is destroy-and-rebuild: drop the presenter to release the
//!   surface, construct a new one to rebuild. Every failure is an explicit
//!   [`ProbeError`], never a silent success.
//!
//! The constructor takes an *owning* window handle (`W: 'static`). The surface
//! is `Surface<'static>` and keeps that handle alive, so the window outlives the
//! surface by construction (see `wgpu::Surface::_handle_source`).
//!
//! For hosts that own a non-`Send` native widget (for example a GTK child), the
//! safe owning constructors cannot be used.
//! [`SurfaceProbePresenter::new_from_raw_handles`] is an explicit `unsafe`
//! alternative whose lifetime contract is entirely the caller's (see its
//! `# Safety`); it reuses the same factory as the safe path.

use crate::wgpu;
use futures_lite::future::block_on;

mod pipelines;
mod presenter;
mod shaders;
mod types;

/// Real-GPU evidence: opens an Xlib window on the host display, drives the probe
/// and checks it presents on a hardware Vulkan adapter.
///
/// Ignored by default (needs a real X11/XWayland window + a hardware GPU).
/// Run once, serially:
/// `cargo test -p cad-render-wgpu --lib probe_presents_grid_and_rebuilds -- --ignored --test-threads=1 --nocapture`
#[cfg(all(test, target_os = "linux"))]
mod linux_gpu_tests;
#[cfg(test)]
mod tests;

pub(crate) use pipelines::*;
pub use presenter::*;
pub(crate) use shaders::*;
pub use types::*;
