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
//! - The instance's backend set and adapter enumeration come from the caller's
//!   [`ProbeBackendPolicy`] (the renderer half of the durable user preference
//!   `wgpu_backend`, wire values exactly `auto`/`vulkan`/`dx12`):
//!   - `vulkan` = the historical contract: `Backends::VULKAN` only.
//!   - `dx12` = `Backends::DX12` only. On targets where wgpu does not compile the
//!     DX12 backend (anything but Windows) construction fails explicitly with
//!     [`ProbeError::UnsupportedBackendPolicy`] naming the policy — there is no
//!     fallback to Vulkan and no empty success.
//!   - `auto` (the default, also used when the preference is absent, empty or
//!     unknown) = `Backends::VULKAN | Backends::DX12`, with Vulkan preferred over
//!     DX12 inside a ranking tier. A backend not compiled for the target simply
//!     has no adapters; it is never substituted for the other one.
//! - Adapter selection rejects `cpu` (software/fallback) adapters. A preferred
//!   adapter *name* (default `P620`) is only a preference, never a filter: any
//!   eligible hardware adapter that can present to the surface is accepted. The
//!   full order is: hardware before CPU, then preferred name, then Vulkan before
//!   DX12, then discrete before integrated, then enumeration order.
//! - **Explicit, probe-only exception**: when the process environment contains
//!   `YACR_PROBE_ALLOW_CPU_ADAPTER` (presence-based; the value is irrelevant),
//!   CPU/software adapters become eligible too under every backend policy (for
//!   example lavapipe on Vulkan, or the Windows WARP DX12 adapter), so host
//!   bring-up and evidence runs work in software-rendering environments.
//!   Hardware adapters still rank strictly above CPU ones — a name-preferred CPU
//!   adapter never outranks hardware — and the preference order inside a
//!   device-type tier is unchanged. With the variable **absent** the contract is
//!   exactly as above (CPU adapters rejected). This is a bring-up/evidence knob,
//!   **not** a production relaxation: software rendering must never be reported
//!   as a hardware-GPU result.
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
//!
//! Windows hosts use a second `unsafe` entry point:
//! [`SurfaceProbePresenter::new_from_composition_visual`] builds the surface from
//! an `IDCompositionVisual` (`wgpu::SurfaceTargetUnsafe::CompositionVisual`), so
//! wgpu-hal creates a `CreateSwapChainForComposition` chain. That is the only
//! swapchain kind that composites a `WS_CHILD` under a transparent
//! (`WS_EX_NOREDIRECTIONBITMAP`) top-level window — an HWND flip chain binds to
//! the top-level DWM visual and its frames never appear. It reuses the same
//! factory, and the ready evidence line names the kind via
//! [`ProbeSurfaceKind::as_token`] (`surface=hwnd|composition`).

use crate::wgpu;
use futures_lite::future::block_on;

#[cfg(target_os = "windows")]
mod composition;
mod pipelines;
mod present_solid;
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
