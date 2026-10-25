//! The zoom command path: the fixed `ZOOMIN`/`ZOOMOUT` steps and the typed
//! `ZOOM <argument>` line.
//!
//! Both forms are one path: a scale factor applied about the **viewport centre**
//! through [`cad_app::CommandPayload::ZoomAt`]. A button supplies no cursor and a
//! typed `ZOOM 2` supplies no cursor either, so the centre anchoring is the
//! deliberate behaviour of a cursor-less zoom — never a fabricated no-op and
//! never a second zoom implementation.
//!
//! Kept in its own module only to keep `session/mod.rs` within the file-size
//! budget; nothing here is a second authority: the application validates the
//! factor and projection and its refusal is returned verbatim.

use cad_app::{CommandId, CommandPayload};

use super::diagnostics::cad_error_text;
use super::{parse_zoom_argument, CadSession, ZoomArgument, ARGUMENT_SELECTOR};

impl CadSession {
    /// Apply one typed-argument command line (AutoCAD's `ZOOM n` / `ZOOM ALL`).
    ///
    /// The argument grammar lives in [`parse_zoom_argument`]:
    ///
    /// | argument | behaviour |
    /// | --- | --- |
    /// | positive decimal (`2`, `0.5`, `1.25`) | [`CadSession::zoom_step`] with that **scale factor** (`> 1` zooms in, `< 1` zooms out, the same convention as `ZOOMIN`/`ZOOMOUT`), anchored at the viewport centre |
    /// | `ALL` / `EXTENTS` | `CommandId::FitDrawing`, the ordinary `FIT` path |
    /// | anything else (`0`, `-1`, `abc`, `2 3`) | the stable [`super::invalid_zoom_argument_reason`], recorded in [`CadSession`]'s error; nothing changes |
    ///
    /// No `CommandId` is added and no second zoom path exists: the factor form is
    /// exactly the button step with a caller-supplied factor.
    pub(super) fn typed_argument_command(
        &self,
        selector: &'static str,
        argument: &str,
        text: &str,
    ) -> Result<(), String> {
        debug_assert_eq!(selector, ARGUMENT_SELECTOR);
        match parse_zoom_argument(argument) {
            Some(ZoomArgument::Factor(factor)) => {
                // Echo the admitted selector name (never the typed value) in the
                // command log, exactly like the name-only path.
                self.core.record_command(selector);
                self.zoom_step(factor)
            }
            Some(ZoomArgument::Fit) => {
                self.core.record_command(selector);
                self.dispatch_command(CommandId::FitDrawing)
            }
            None => {
                let reason = super::invalid_zoom_argument_reason(text);
                *self.error.borrow_mut() = Some(reason.clone());
                Err(reason)
            }
        }
    }

    /// Apply a fixed zoom step about the **viewport centre**.
    ///
    /// A `ZOOMIN`/`ZOOMOUT` button supplies no cursor, so the step is anchored
    /// at the logical centre of the active viewport — the deliberate behaviour
    /// of a cursor-less zoom control (a real zoom about the viewport centre that
    /// keeps the centre fixed), never a fabricated no-op. The
    /// [`CommandPayload::ZoomAt`] payload is synthesized here because the
    /// router's payload-bearing `ZOOM` selector otherwise requires a
    /// caller-supplied factor. The application validates the factor and
    /// projection and refuses explicitly on failure; the outcome is never turned
    /// into an empty success.
    pub(super) fn zoom_step(&self, factor: f64) -> Result<(), String> {
        let centre = {
            let c = self.core.controller.borrow();
            c.application
                .workspace
                .viewports
                .get(&c.viewport_id)
                .map(|viewport| {
                    [
                        viewport.logical_size[0] * 0.5,
                        viewport.logical_size[1] * 0.5,
                    ]
                })
        };
        let Some(cursor) = centre else {
            return Err("zoom step needs an active viewport".into());
        };
        match self
            .core
            .command(CommandId::Zoom, CommandPayload::ZoomAt { factor, cursor })
        {
            Ok(()) => {
                *self.error.borrow_mut() = None;
                Ok(())
            }
            Err(error) => {
                let text = cad_error_text(&error);
                *self.error.borrow_mut() = Some(text.clone());
                Err(text)
            }
        }
    }
}
