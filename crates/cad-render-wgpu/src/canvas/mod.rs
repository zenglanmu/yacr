//! Wasm-only **WebGL2 canvas presenter**: the browser sibling of
//! [`crate::native_presenter`].
//!
//! The `dom-canvas` runtime path (`surfaceMode: dom-canvas`, ADR 0024) draws the
//! CAD frame with wgpu into an offscreen texture and blits it onto a **web
//! canvas**. Neither target runtime exposes WebGPU (`navigator.gpu` is absent in
//! WebKitGTK 2.52.6 and Chromium 141), so the only possible wasm backend is
//! **WebGL2** — `wgpu/webgl` — and the instance requests `Backends::GL` only.
//!
//! # Two carriers, one presenter
//!
//! The presenter draws into **either** web canvas kind, selected by constructor:
//!
//! | constructor | canvas | where the session runs |
//! |---|---|---|
//! | [`CanvasPresenter::create`] | page `<canvas>` (`HtmlCanvasElement`) | main thread |
//! | [`CanvasPresenter::create_offscreen`] | `OffscreenCanvas` (transferred into a Worker with `transferControlToOffscreen()`) | Worker |
//!
//! Everything else is shared: one instance descriptor, one WebGL2 device floor,
//! one present-mode/format/alpha policy, one [`crate::blit`] fullscreen blit, one
//! acquire/status policy. The carrier is recorded as
//! [`CanvasAdapterInfo::target`] / [`CanvasPresenter::target_kind`]
//! ([`CanvasTargetKind`]), which is where a host reads whether it may report
//! `isolation: worker`.
//!
//! # The measured engine split (2026-10-24, this lane)
//!
//! Worker-side rendering is a **browser capability**, and the two runtimes this
//! project targets disagree:
//!
//! - **Chromium 141**: WebGL2 **works** inside a dedicated Worker on an
//!   `OffscreenCanvas` transferred with `transferControlToOffscreen()` (verified:
//!   ANGLE/Intel adapter, clear + `readPixels` correct).
//! - **WebKitGTK 2.52.6**: `OffscreenCanvas.getContext("webgl2")` in a Worker
//!   returns **`null`** (verified). Worker-side rendering is therefore impossible
//!   on that engine, and it must be reported as an explicit limitation — never
//!   silently attempted, never papered over with a blank canvas.
//!
//! So [`CanvasPresenter::create_offscreen`] on WebKitGTK fails early with
//! [`CanvasError::SurfaceCreate`] (the text names `offscreen-canvas`), and the
//! host is expected to keep the main-thread carrier and say so in its status line.
//!
//! Contracts:
//! - The host passes a canvas handle and a **physical** size (CSS px ×
//!   `devicePixelRatio`); it never handles a wgpu surface. `wgpu` stays confined
//!   to this crate, exactly like the native presenter.
//! - Presentation is a fullscreen blit of the renderer's offscreen frame texture
//!   (the same shader the native path uses, see [`crate::blit`]): no CPU readback,
//!   no blocking poll (WebGL2 has none), no second CAD session.
//! - Every failure is explicit. A zero/out-of-limit physical size, an
//!   unreachable adapter/device, a stale or lost surface, and a frame that cannot
//!   be blitted (size/format/usage mismatch) all produce a [`CanvasError`] that
//!   names the failed step; a blank or stretched canvas is never presented
//!   silently.
//!
//! Build/enable boundary: [`CanvasPresenter`] itself is
//! `#[cfg(target_arch = "wasm32")]`, because it owns a `web_sys` canvas handle
//! and a real WebGL2 surface. The framework-free policy around it
//! ([`CanvasError`], [`SurfaceStatus`], [`CanvasTargetKind`], the physical-size and
//! format rules) is compiled on every target so its unit tests actually run in the
//! ordinary `cargo test -p cad-render-wgpu` (native) invocation instead of being
//! skipped with the browser-only code.
//!
//! The per-target split mirrors `native_presenter`, which exists only natively.

mod policy;

pub use policy::{
    check_physical_size, check_present_format, check_present_size, check_present_usage,
    preferred_present_format, CanvasError, CanvasTargetKind, SurfaceStatus,
};

#[cfg(target_arch = "wasm32")]
mod presenter;

#[cfg(target_arch = "wasm32")]
pub use presenter::{CanvasAdapterInfo, CanvasPresenter};

#[cfg(test)]
mod tests;
