//! The **discrete** view commands: framing (`FIT`), `ZOOM WINDOW` and
//! `ZOOM PREVIOUS`.
//!
//! Split out of `application.rs` to keep both files inside the code-size budget.
//! Nothing here is a second camera authority: every function mutates the
//! viewport of the [`crate::Command`] it is handed, through the same
//! [`crate::Camera`] value the rest of the application uses, and none of them
//! writes the drawing, the database revision or a `cad-history` entry — a view
//! command is session state only.
//!
//! ## One rect → (centre, scale) rule
//!
//! [`Application::framing_rect`] is the **single** mapping from a world
//! rectangle to the plan camera that frames it, used by `FIT` and by
//! `ZOOM WINDOW` with different margins:
//!
//! ```text
//! cx    = (min.x + max.x) / 2
//! cy    = (min.y + max.y) / 2
//! ex    = max(max.x - min.x, 1e-6)
//! ey    = max(max.y - min.y, 1e-6)
//! scale = max(ex / w, ey / h) * margin          (w, h = logical size, floored at 1)
//! ```
//!
//! `max(ex / w, ey / h)` is what makes the **tighter** axis decide: the
//! rectangle then fits horizontally *and* vertically, centred on the viewport.
//!
//! The margin is the one declared difference between the two commands:
//!
//! * `FIT` uses [`FIT_MARGIN`] (`1.05`), the pre-existing one-pixel-in-twenty
//!   breathing room around the drawing;
//! * `ZOOM WINDOW` uses [`WINDOW_MARGIN`] (`1.0`), i.e. the picked rectangle
//!   fills the viewport **exactly**. That is our choice, declared here: AutoCAD
//!   also fills the window, but any residual padding it applies is not
//!   documented, so claiming parity on a few-percent margin would be a guess.
//!   An exact fit is reproducible and contract-tested.

use super::*;

/// The `FIT` breathing-room factor around the framed bounds (see the module
/// docs). Kept at the historical value so existing framing is unchanged.
pub const FIT_MARGIN: f64 = 1.05;

/// The `ZOOM WINDOW` margin: the picked rectangle fills the viewport exactly
/// (see the module docs for why this is our declared choice).
pub const WINDOW_MARGIN: f64 = 1.0;

/// Canonicalise a two-corner **logical-pixel** rectangle to `[min, max]`.
///
/// One rule for every producer (the typed capture, the host's box gesture and a
/// programmatic caller): the corners may arrive in any order, and a non-finite
/// corner or a rectangle with no extent on either axis is an explicit
/// `InvalidInput` — never clamped, never reordered into a fabricated window.
/// The two refusal codes are stable so a host can report them by name.
pub fn canonical_logical_rect(rect_logical: [[f64; 2]; 2]) -> CadResult<[[f64; 2]; 2]> {
    let [first, second] = rect_logical;
    if !first
        .iter()
        .chain(second.iter())
        .all(|value| value.is_finite())
    {
        return Err(CadError::InvalidInput(
            "zoom.window.invalid_rect: the ZOOM WINDOW corners must be finite logical pixels"
                .into(),
        ));
    }
    let min = [first[0].min(second[0]), first[1].min(second[1])];
    let max = [first[0].max(second[0]), first[1].max(second[1])];
    if max[0] <= min[0] || max[1] <= min[1] {
        return Err(CadError::InvalidInput(
            "zoom.window.degenerate_rect: the ZOOM WINDOW rectangle must have an extent on both \
             axes"
                .into(),
        ));
    }
    Ok([min, max])
}

impl Application {
    /// The plan-view centre and orthographic scale that frame the world
    /// rectangle `[min, max]` in a viewport of `logical_size`.
    ///
    /// The one rect → (centre, scale) rule of this module (see the module docs
    /// for the formula and the margin choice). A degenerate extent is floored at
    /// `1e-6` and the scale at [`camera::MIN_ORTHO_SCALE`], so the result is
    /// always a renderable camera.
    pub(crate) fn framing_rect(
        min: Point3,
        max: Point3,
        logical_size: [f64; 2],
        margin: f64,
    ) -> ([f64; 2], f64) {
        let cx = (min.x + max.x) * 0.5;
        let cy = (min.y + max.y) * 0.5;
        let ex = (max.x - min.x).max(1e-6);
        let ey = (max.y - min.y).max(1e-6);
        let w = logical_size[0].max(1.0);
        let h = logical_size[1].max(1.0);
        let scale = ((ex / w).max(ey / h) * margin).max(camera::MIN_ORTHO_SCALE);
        ([cx, cy], scale)
    }

    /// The 2D plan camera centred on `centre` with `scale`, keeping the given
    /// eye height (a plan fit never changes how far above the plane the eye sits).
    fn plan_camera(centre: [f64; 2], scale: f64, eye_z: f64) -> Camera {
        Camera {
            eye: Point3 {
                x: centre[0],
                y: centre[1],
                z: eye_z,
            },
            target: Point3 {
                x: centre[0],
                y: centre[1],
                z: 0.0,
            },
            up: Point3 {
                x: 0.0,
                y: 1.0,
                z: 0.0,
            },
            projection: Projection::Orthographic { scale },
        }
    }

    /// Fit a viewport to the current document bounds. Hosts call this after an
    /// import so the first frame is usable without a synthetic command.
    ///
    /// Fitting always returns the viewport to the 2D plan view (audit F13), so a
    /// fit is a deterministic reset-and-frame, not a 3D manipulation.
    ///
    /// Returns `false` when the drawing has no measurable extent: the camera is
    /// left untouched and nothing is framed, so a caller can report that
    /// explicitly instead of claiming success.
    ///
    /// A real fit is a **discrete** view change, so the pre-fit view is recorded
    /// as one `ZOOM PREVIOUS` step first ([`crate::zoom_history`]); a fit that
    /// frames nothing records no step at all.
    pub fn fit_viewport(
        &mut self,
        session: &mut SessionState,
        viewport_id: &ViewportId,
    ) -> CadResult<bool> {
        let document = self
            .workspace
            .documents
            .get(&session.document)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        let bounds = document.drawing.bounds();
        let viewport = self
            .workspace
            .viewports
            .get_mut(viewport_id)
            .ok_or_else(|| CadError::InvalidInput("unknown viewport".into()))?;
        let Some((min, max)) = bounds else {
            // An empty drawing has nothing to frame. Fitting is a documented
            // no-op (the camera and the session generation are left unchanged),
            // never an error and never a fabricated view. The `false` return lets
            // the caller report it explicitly.
            return Ok(false);
        };
        let (centre, scale) = Self::framing_rect(min, max, viewport.logical_size, FIT_MARGIN);
        let camera = Self::plan_camera(centre, scale, viewport.camera.eye.z);
        viewport.push_view_snapshot();
        viewport.camera = camera;
        viewport.view_mode = ViewMode2d3d::TwoD { saved: camera };
        session.generation += 1;
        Ok(true)
    }

    /// `ZOOM WINDOW`: frame the picked two-corner rectangle **exactly**.
    ///
    /// Two-corner capture semantics: the payload carries the two opposite
    /// corners in **canvas logical pixels** (the same convention as
    /// [`CommandPayload::BoxSelect`] and [`CommandPayload::ZoomAt`]), so no
    /// drafting constraint (object snap, grid snap, `ORTHO`) can distort the
    /// rectangle, and the corners are canonicalised to `[min, max]` here — the
    /// command is the single validation point, whatever produced the payload.
    ///
    /// **2D orthographic only.** A window is a statement about a plan
    /// rectangle, which has no meaning in a perspective observation, so a 3D
    /// view is refused **by name** (`zoom.window.requires_2d`) instead of being
    /// turned into a perspective fit the user did not ask for. `view_mode` stays
    /// whatever the 2D mode was (`TwoD`, with its saved camera tracking the new
    /// one, exactly like [`Application::zoom_at`]).
    ///
    /// A real window records one `ZOOM PREVIOUS` step first
    /// ([`crate::zoom_history`]); a refused or degenerate rectangle records
    /// nothing and moves nothing. No database write, no revision bump, no
    /// `cad-history` entry, and `session.generation` is **not** bumped: that
    /// stamp is for document/task work, and a plain zoom is not one.
    pub(crate) fn zoom_window(&mut self, command: &Command) -> CadResult<CommandOutcome> {
        let CommandPayload::ZoomWindow { rect_logical } = &command.payload else {
            return Err(CadError::InvalidInput(
                "ZoomWindow needs a { rect_logical } payload".into(),
            ));
        };
        let rect = canonical_logical_rect(*rect_logical)?;
        let viewport = self.viewport_mut(command)?;
        if viewport.view_mode.kind() != ProjectionKind::TwoD
            || !viewport.camera.projection.is_orthographic()
        {
            return Err(CadError::InvalidInput(
                "zoom.window.requires_2d: ZOOM WINDOW is only defined in the 2D plan view; a \
                 perspective view has no window to fit (switch back to 2D first)"
                    .into(),
            ));
        }
        let size = viewport.logical_size;
        let camera = viewport.camera;
        // The screen-to-plan mapping is the one measurement picks and the box
        // rubber band use, so the window is the rectangle the user dragged.
        let first = camera.screen_to_plan_world(rect[0], size).ok_or_else(|| {
            CadError::InvalidInput(
                "zoom.window.degenerate_rect: the first corner cannot be mapped to the plan".into(),
            )
        })?;
        let second = camera.screen_to_plan_world(rect[1], size).ok_or_else(|| {
            CadError::InvalidInput(
                "zoom.window.degenerate_rect: the second corner cannot be mapped to the plan"
                    .into(),
            )
        })?;
        let min = Point3 {
            x: first.x.min(second.x),
            y: first.y.min(second.y),
            z: first.z,
        };
        let max = Point3 {
            x: first.x.max(second.x),
            y: first.y.max(second.y),
            z: first.z,
        };
        if max.x <= min.x || max.y <= min.y {
            return Err(CadError::InvalidInput(
                "zoom.window.degenerate_rect: the ZOOM WINDOW rectangle must have an extent on \
                 both axes"
                    .into(),
            ));
        }
        let (centre, scale) = Self::framing_rect(min, max, size, WINDOW_MARGIN);
        let framed = Self::plan_camera(centre, scale, camera.eye.z);
        viewport.push_view_snapshot();
        viewport.camera = framed;
        viewport.view_mode = ViewMode2d3d::TwoD { saved: framed };
        Ok(CommandOutcome::none())
    }

    /// `ZOOM PREVIOUS`: restore this viewport's last **discrete** view.
    ///
    /// Pops the newest [`crate::ViewSnapshot`] and applies it exactly (camera,
    /// 2D/3D mode, plan work plane) — no re-framing, no clamp. It never pushes,
    /// so repeated presses walk back monotonically; pans, wheel zooms and orbit
    /// drags never recorded a step (the declared divergence documented in
    /// [`crate::zoom_history`]).
    ///
    /// An empty history is **not an error** and changes nothing: it reports the
    /// `zoom.previous.empty` diagnostic, the same non-error shape `LAYERP` uses
    /// for "no previous layer state". No database write, no revision bump and no
    /// `cad-history` entry.
    pub(crate) fn zoom_previous(&mut self, command: &Command) -> CadResult<CommandOutcome> {
        let viewport = self.viewport_mut(command)?;
        let Some(snapshot) = viewport.zoom_history.pop() else {
            return Ok(CommandOutcome {
                objects: Vec::new(),
                changes: None,
                diagnostics: vec![Diagnostic {
                    object: None,
                    code: "zoom.previous.empty".into(),
                    message: "ZOOM PREVIOUS: there is no previous view in this viewport; nothing \
                              changed"
                        .into(),
                }],
                measurement: None,
                point_report: None,
            });
        };
        snapshot.restore(viewport);
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: None,
            diagnostics: vec![Diagnostic {
                object: None,
                code: "zoom.previous".into(),
                message: "ZOOM PREVIOUS: restored the previous view (session view state only)"
                    .into(),
            }],
            measurement: None,
            point_report: None,
        })
    }
}
