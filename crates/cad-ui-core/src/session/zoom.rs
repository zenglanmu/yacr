//! The zoom command path: the fixed `ZOOMIN`/`ZOOMOUT` steps, the typed
//! `ZOOM <argument>` line and its two capture/restore forms.
//!
//! Every form is one path: a scale factor applied about the **viewport centre**
//! through [`cad_app::CommandPayload::ZoomAt`]. A button supplies no cursor and a
//! typed `ZOOM 2` supplies no cursor either, so the centre anchoring is the
//! deliberate behaviour of a cursor-less zoom — never a fabricated no-op and
//! never a second zoom implementation.
//!
//! Two argument forms are **capture/restore** commands that carry their own
//! payload on the same [`CommandId::Zoom`] (no new command id, no new
//! vocabulary):
//!
//! * `ZOOM WINDOW` ([`ZoomArgument::Window`]) arms the host's two-corner capture
//!   ([`cad_host::HostCore::begin_zoom_window`]). The camera does **not** move at
//!   admission: the next 2D primary drag supplies the rectangle and its release
//!   dispatches one [`cad_app::CommandPayload::ZoomWindow`]. A 3D view is refused
//!   by name (`zoom.window.requires_2d`) at arming, so no dead capture exists;
//! * `ZOOM PREVIOUS` ([`ZoomArgument::Previous`]) dispatches one
//!   [`cad_app::CommandPayload::ZoomPrevious`] immediately — the per-viewport
//!   bounded discrete-view stack restores the previous view (or reports the
//!   `zoom.previous.empty` diagnostic when there is none).
//!
//! Kept in its own module only to keep `session/mod.rs` within the file-size
//! budget; nothing here is a second authority: the application validates the
//! factor/projection/rectangle and its refusal is returned verbatim.

use cad_app::{CommandId, CommandPayload};

use super::diagnostics::cad_error_text;
use super::{parse_zoom_argument, CadSession, ZoomArgument, ARGUMENT_SELECTOR};

impl CadSession {
    /// Apply one typed-argument command line (AutoCAD's `ZOOM n` / `ZOOM ALL` /
    /// `ZOOM WINDOW` / `ZOOM PREVIOUS`).
    ///
    /// The argument grammar lives in [`parse_zoom_argument`]:
    ///
    /// | argument | behaviour |
    /// | --- | --- |
    /// | positive decimal (`2`, `0.5`, `1.25`) | [`CadSession::zoom_step`] with that **scale factor** (`> 1` zooms in, `< 1` zooms out, the same convention as `ZOOMIN`/`ZOOMOUT`), anchored at the viewport centre |
    /// | `ALL` / `EXTENTS` | `CommandId::FitDrawing`, the ordinary `FIT` path |
    /// | `WINDOW` | arm the two-corner capture ([`CadSession::zoom_window_capture`]); the camera changes only when the rectangle is completed |
    /// | `PREVIOUS` | dispatch [`CommandPayload::ZoomPrevious`] now |
    /// | anything else (`0`, `-1`, `abc`, `2 3`) | the stable [`super::invalid_zoom_argument_reason`], recorded in [`CadSession`]'s error; nothing changes |
    ///
    /// No `CommandId` is added and no second zoom path exists: the factor form is
    /// exactly the button step with a caller-supplied factor, and the two word
    /// forms ride `CommandId::Zoom` with their own payloads.
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
            Some(ZoomArgument::Window) => {
                self.core.record_command(selector);
                self.zoom_window_capture()
            }
            Some(ZoomArgument::Previous) => {
                self.core.record_command(selector);
                self.dispatch_payload(CommandId::Zoom, CommandPayload::ZoomPrevious)
            }
            None => {
                let reason = super::invalid_zoom_argument_reason(text);
                *self.error.borrow_mut() = Some(reason.clone());
                Err(reason)
            }
        }
    }

    /// Arm the `ZOOM WINDOW` two-corner capture on the host.
    ///
    /// This is **not** a command dispatch: arming is transient host gesture state
    /// with no `CommandId` and no engine request, so it is a direct host call
    /// (like [`cad_host::HostCore::record_command`]). The host validates the
    /// precondition — a 3D view is refused by name
    /// (`zoom.window.requires_2d`), and so is a capture that already owns the
    /// pointer (`zoom.window.capture_busy`) — and its refusal is recorded for
    /// `cad_status.error` and returned verbatim. Nothing reaches the camera at
    /// this point.
    pub(super) fn zoom_window_capture(&self) -> Result<(), String> {
        match self.core.begin_zoom_window() {
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
