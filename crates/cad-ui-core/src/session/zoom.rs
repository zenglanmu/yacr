//! The zoom command path: the fixed `ZOOMIN`/`ZOOMOUT` steps, the typed
//! `ZOOM <argument>` line and its capture/restore/selection forms.
//!
//! Every view form is one path: a scale factor applied about the **viewport
//! centre** through [`cad_app::CommandPayload::ZoomAt`]. A button supplies no
//! cursor and a typed `ZOOM 2` / `ZOOM 2X` supplies no cursor either, so the
//! centre anchoring is the deliberate behaviour of a cursor-less zoom — never a
//! fabricated no-op and never a second zoom implementation.
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
//! `ZOOM OBJECT` ([`ZoomArgument::Object`]) dispatches
//! [`cad_app::CommandPayload::ZoomObject`] and lets the application read the
//! **current selection**, while `ZOOM nXP` ([`ZoomArgument::PaperScale`]) is a
//! drawing write routed through the already Work-only `VPSCALE` path. `ZOOM
//! DYNAMIC` remains an explicit named gap (`zoom.dynamic_not_implemented`).
//!
//! Kept in its own module only to keep `session/mod.rs` within the file-size
//! budget; nothing here is a second authority: the application validates the
//! factor/projection/rectangle/selection and its refusal is returned verbatim.

use cad_app::{CommandId, CommandPayload, LayoutPayload};
use cad_domain::SpaceId;

use super::diagnostics::cad_error_text;
use super::{parse_zoom_argument, CadSession, ZoomArgument, ARGUMENT_SELECTOR};

impl CadSession {
    /// Apply one typed-argument command line (AutoCAD's `ZOOM n` / `ZOOM ALL` /
    /// `ZOOM WINDOW` / `ZOOM PREVIOUS` / `ZOOM OBJECT` / `ZOOM nX` / `ZOOM nXP`).
    ///
    /// The argument grammar lives in [`parse_zoom_argument`]:
    ///
    /// | argument | behaviour |
    /// | --- | --- |
    /// | positive decimal (`2`, `0.5`, `1.25`) | [`CadSession::zoom_step`] with that **scale factor** (`> 1` zooms in, `< 1` zooms out, the same convention as `ZOOMIN`/`ZOOMOUT`), anchored at the viewport centre |
    /// | `nX` (`2X`, `0.5X`) | the same cursor-less [`CadSession::zoom_step`] with the relative factor |
    /// | `ALL` / `EXTENTS` | `CommandId::FitDrawing`, the ordinary `FIT` path |
    /// | `WINDOW` | arm the two-corner capture ([`CadSession::zoom_window_capture`]); the camera changes only when the rectangle is completed |
    /// | `PREVIOUS` | dispatch [`CommandPayload::ZoomPrevious`] now |
    /// | `OBJECT` | dispatch [`CommandPayload::ZoomObject`]; the application reads the current selection |
    /// | `nXP` / `a/bXP` | require an active paper layout (`zoom.xp.requires_paper`), then dispatch the Work-only `VPSCALE` write as `a:b` |
    /// | `DYNAMIC` | the named gap `zoom.dynamic_not_implemented`; nothing changes |
    /// | anything else (`0`, `-1`, `abc`, `2 3`) | the stable [`super::invalid_zoom_argument_reason`], recorded in [`CadSession`]'s error; nothing changes |
    ///
    /// No `CommandId` is added: the factor forms are exactly the button step
    /// with a caller-supplied factor, the word forms ride `CommandId::Zoom`
    /// with their own payloads, and `nXP` reuses the existing Work-only
    /// [`CommandId::VpScale`] path rather than writing the drawing here.
    pub(super) fn typed_argument_command(
        &self,
        selector: &'static str,
        argument: &str,
        text: &str,
    ) -> Result<(), String> {
        debug_assert_eq!(selector, ARGUMENT_SELECTOR);
        // `ZOOM DYNAMIC` is a **named gap**: it is outside the parser grammar
        // and is refused here with its own stable reason instead of falling
        // into the generic invalid-argument sentence. It is never approximated
        // by another zoom form.
        if argument.trim().eq_ignore_ascii_case("DYNAMIC") {
            return self.refuse_argument(
                "zoom.dynamic_not_implemented: ZOOM DYNAMIC is not implemented in this build; \
                 nothing changed"
                    .into(),
            );
        }
        match parse_zoom_argument(argument) {
            Some(ZoomArgument::Factor(factor) | ZoomArgument::Relative(factor)) => {
                // Echo the admitted selector name (never the typed value) in the
                // command log, exactly like the name-only path. `2X` is the same
                // scale operation as `2`, anchored at the viewport centre.
                self.core.record_command(selector);
                self.zoom_step(factor)
            }
            Some(ZoomArgument::PaperScale { paper, drawing }) => {
                // `nXP` writes the drawing through the existing Work-only
                // `VPSCALE` path, not through the Viewer-legal `ZOOM` command.
                // Only an active paper layout has a viewport scale to write.
                let space = self.core.controller.borrow().session.active_space.clone();
                if !matches!(space, SpaceId::Paper(_)) {
                    return self.refuse_argument(
                        "zoom.xp.requires_paper: ZOOM nXP sets a paper-space viewport scale and \
                         needs an active paper layout; nothing changed"
                            .into(),
                    );
                }
                self.core.record_command(selector);
                self.dispatch_payload(
                    CommandId::VpScale,
                    CommandPayload::Layout(Box::new(LayoutPayload::Scale {
                        name: format!("{paper}:{drawing}"),
                    })),
                )
            }
            Some(ZoomArgument::Object) => {
                self.core.record_command(selector);
                self.dispatch_payload(CommandId::Zoom, CommandPayload::ZoomObject)
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
