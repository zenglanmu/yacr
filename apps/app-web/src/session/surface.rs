//! The one carrier the **viewer** session still opens: an `OffscreenCanvas`
//! transferred into a Worker.
//!
//! The page carrier moved to the production session ([`crate::page`],
//! `cad_ui_core::session::CadSession`): a viewer on the page's `<canvas>` would be
//! the second CAD session per canvas that ADR 0024 D2 forbids. What is left here is
//! the isolated carrier — the Worker owns it outright, and the page creates no
//! session at all on that path.
//!
//! # Single owner, enforced here
//!
//! [`Offscreen::resolve`] refuses an `OffscreenCanvas` when the module is **not**
//! running in a Worker ([`STEP_ISOLATION`]): a page-side `OffscreenCanvas` session
//! would compete with the page's own production session for the one document. The
//! rule is structural rather than advisory.

use cad_render_wgpu::canvas::CanvasPresenter;

use super::session_state::{Failure, STEP_ISOLATION, STEP_WEBGL2};
use crate::capability::{self, Context};

/// A canvas the page transferred into this Worker
/// (`transferControlToOffscreen()`, then `postMessage(.., [offscreen])`).
pub(crate) struct Offscreen(web_sys::OffscreenCanvas);

impl Offscreen {
    /// Accept the transferred canvas, or explain why it cannot be this session's.
    ///
    /// Called **before** the (expensive) geometry load, so a page-side attempt is a
    /// cheap, explicit `isolation` refusal that costs no parse.
    pub(super) fn resolve(canvas: web_sys::OffscreenCanvas) -> Result<Self, Failure> {
        if capability::current_context() != Context::Worker {
            return Err(Failure::new(
                STEP_ISOLATION,
                "start_offscreen is only valid inside a Worker: an OffscreenCanvas \
                 session opened from the page would be a second, competing owner of \
                 the single CAD session (the page carrier is the production session)",
            ));
        }
        Ok(Self(canvas))
    }

    /// Create the WebGL2 presenter for this carrier at `physical` pixels.
    ///
    /// This is the real capability check — instance → surface → adapter → device →
    /// configure — not a feature sniff, and it is where a worker-side
    /// `OffscreenCanvas` fails on WebKitGTK 2.52.6 (`getContext("webgl2")` returns
    /// `null`). Such a failure is [`STEP_WEBGL2`] with the `webgl2` refusal marker,
    /// never a blank canvas.
    pub(super) async fn presenter(self, physical: (u32, u32)) -> Result<CanvasPresenter, Failure> {
        CanvasPresenter::create_offscreen(self.0, physical)
            .await
            .map_err(|error| Failure::new(STEP_WEBGL2, error.to_string()))
    }
}
