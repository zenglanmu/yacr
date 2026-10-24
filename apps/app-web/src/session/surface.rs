//! The two carriers a CAD session can be opened on.
//!
//! One session implementation, two carriers: the pipeline downstream of this module
//! ([`super::import`] → [`super::upload`] → the frame path in
//! [`super::session_state`]) is identical for both, and this module is the only
//! place that knows which web canvas a [`cad_render_wgpu::canvas::CanvasPresenter`]
//! was created from.
//!
//! | carrier | canvas handle | isolation |
//! |---|---|---|
//! | [`Surface::Page`] | `HtmlCanvasElement` resolved from the host page by id | main thread |
//! | [`Surface::Offscreen`] | `OffscreenCanvas` transferred into this Worker | worker |
//!
//! # Single owner, enforced here
//!
//! [`Surface::Offscreen`] is refused when the module is **not** running in a Worker
//! ([`STEP_ISOLATION`]). Without that refusal a page could open a session on a
//! canvas it also transferred to a worker (or vice versa) and there would be two
//! CAD documents for one canvas — the exact situation ADR 0024 D2 forbids. The rule
//! is structural rather than advisory: the page carrier is a **page** `<canvas>`
//! and the worker carrier is an **`OffscreenCanvas`**, and each is refused in the
//! context where it cannot be the only owner.

use cad_render_wgpu::canvas::CanvasPresenter;

use super::session_state::{canvas_element, Failure, STEP_CANVAS, STEP_ISOLATION, STEP_WEBGL2};
use crate::capability::{self, Context};

/// The carrier a host asks for.
pub(crate) enum Surface {
    /// The host page's `<canvas id=...>` (main-thread session).
    Page(String),
    /// A canvas the page transferred into this Worker
    /// (`transferControlToOffscreen()`, then `postMessage(.., [offscreen])`).
    Offscreen(web_sys::OffscreenCanvas),
}

/// A carrier resolved to a concrete canvas handle, before any GPU object exists.
///
/// Resolution is deliberately fallible and separate: the caller resolves **before**
/// the (expensive) geometry load, so a missing element, a wrong element type or an
/// offscreen canvas in the wrong context is reported without parsing anything.
pub(crate) enum Carrier {
    /// The page element, created with
    /// [`CanvasPresenter::create`](cad_render_wgpu::canvas::CanvasPresenter::create).
    Page(web_sys::HtmlCanvasElement),
    /// The transferred canvas, created with
    /// [`CanvasPresenter::create_offscreen`](cad_render_wgpu::canvas::CanvasPresenter::create_offscreen).
    Offscreen(web_sys::OffscreenCanvas),
}

impl Surface {
    /// Resolve the requested carrier, or explain why it cannot be the session's.
    ///
    /// `Page` errors are [`STEP_CANVAS`] (no such element / not a canvas); a
    /// non-worker `Offscreen` request is [`STEP_ISOLATION`].
    pub(super) fn resolve(self) -> Result<Carrier, Failure> {
        match self {
            Self::Page(canvas_id) => canvas_element(&canvas_id)
                .map(Carrier::Page)
                .map_err(|message| Failure::new(STEP_CANVAS, message)),
            Self::Offscreen(canvas) => {
                if capability::current_context() != Context::Worker {
                    return Err(Failure::new(
                        STEP_ISOLATION,
                        "start_offscreen is only valid inside a Worker: an OffscreenCanvas \
                         session opened from the page would be a second, competing owner of \
                         the single CAD session (use start() with the page's <canvas>)",
                    ));
                }
                Ok(Carrier::Offscreen(canvas))
            }
        }
    }
}

impl Carrier {
    /// Create the WebGL2 presenter for this carrier at `physical` pixels.
    ///
    /// This is the real capability check — instance → surface → adapter → device →
    /// configure — not a feature sniff, and it is where a worker-side
    /// `OffscreenCanvas` fails on WebKitGTK 2.52.6 (`getContext("webgl2")` returns
    /// `null`). Such a failure is [`STEP_WEBGL2`] with the `webgl2` refusal marker,
    /// never a blank canvas.
    pub(super) async fn presenter(self, physical: (u32, u32)) -> Result<CanvasPresenter, Failure> {
        let created = match self {
            Self::Page(canvas) => CanvasPresenter::create(canvas, physical).await,
            Self::Offscreen(canvas) => CanvasPresenter::create_offscreen(canvas, physical).await,
        };
        created.map_err(|error| Failure::new(STEP_WEBGL2, error.to_string()))
    }
}
