//! Android-host draw/edit command sink (F-EDIT).
//!
//! Mirrors `apps/app-web/src/browser/draw.rs`: the shell captures points and
//! emits a confirmed [`cad_app::DrawIntent`]; the host maps it to exactly one
//! application command through the shared transaction/history path
//! (`docs/drawing-edit.md` §2):
//!
//! * `Line { start, end }` → `CreateLine` + `Points([start, end])`
//! * `Circle { center, edge }` → `CreateCircle` + `Points([center, edge])`
//! * `Move { delta }` → `MoveEntities` with the session selection refs + delta
//! * `Trim { target_pick, boundary_pick }` → `TrimEntity` after resolving the
//!   two world picks to `SelectionRef`s
//!
//! `Move`/`Trim` need the session selection or a hit test the shell does not
//! hold, so the host owns the mapping. A miss is an explicit error, never an
//! invented [`SelectionRef`].
//!
//! The same install forwards the live preview into the CAD overlay so the
//! rubber band renders through the existing draw-preview path.

use super::*;

/// A world pick tolerance in drawing units, floored so a small drawing still
/// tolerates a few screen pixels (matches the web host).
fn pick_tolerance() -> f64 {
    TolerancePolicy::default().computation_world.max(0.5)
}

/// Map a world point on the work plane to the closest model-space entity.
///
/// Trim is planar (`docs/drawing-edit.md` §3), so the hit test is a `-Z` ray
/// through the point; a miss is an explicit error rather than a fabricated ref.
fn world_pick_ref(
    controller: &HostController,
    world: Point3,
    tolerance: f64,
) -> CadResult<SelectionRef> {
    let drawing = controller
        .drawing()
        .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
    let ray = Ray3 {
        origin: Point3 {
            x: world.x,
            y: world.y,
            z: world.z + 1.0,
        },
        direction: Point3 {
            x: 0.0,
            y: 0.0,
            z: -1.0,
        },
    };
    let options = cad_app::PickOptions::new(tolerance)?;
    let report =
        cad_app::picking::pick_ray(drawing.as_ref(), controller.document_id, &ray, &options)?;
    report.hit.map(|hit| hit.source).ok_or_else(|| {
        CadError::InvalidInput(format!(
            "no entity under the trim pick ({:.3}, {:.3})",
            world.x, world.y
        ))
    })
}

/// The Android host's draw/edit sink: one command per confirmed intent.
pub(crate) struct AndroidDrawSink {
    controller: Rc<RefCell<HostController>>,
    handle: SharedHandle,
    view: SharedView,
    viewport: ViewportId,
}

impl AndroidDrawSink {
    pub(crate) fn new(
        controller: Rc<RefCell<HostController>>,
        handle: SharedHandle,
        view: SharedView,
        viewport: ViewportId,
    ) -> Self {
        AndroidDrawSink {
            controller,
            handle,
            view,
            viewport,
        }
    }

    fn status(&self, text: impl Into<String>) {
        if let Some(handle) = self.handle.borrow().as_ref() {
            let _ = handle.set_status(text.into());
        }
    }

    /// Send one drawing command through the shared funnel so the status line,
    /// panels and overlays refresh exactly once.
    fn send(&self, id: CommandId, payload: CommandPayload) {
        let document = self.controller.borrow().document_id;
        let command = Command {
            schema_version: 1,
            id,
            document,
            viewport: self.viewport,
            payload,
        };
        let outcome = self.controller.borrow_mut().execute(command);
        sync_view_camera(&self.view, &self.controller, self.viewport);
        if let Some(handle) = self.handle.borrow().as_ref() {
            push_panel_state(&self.controller, handle, &self.view);
        }
        if let Err(e) = outcome {
            self.status(format!("命令失败：{e}"));
        }
    }
}

impl cad_ui_slint::DrawCommandSink for AndroidDrawSink {
    fn commit(&mut self, intent: cad_app::DrawIntent) -> CadResult<()> {
        match intent {
            cad_app::DrawIntent::Line { start, end } => {
                self.send(
                    CommandId::CreateLine,
                    CommandPayload::Points(vec![start, end]),
                );
                Ok(())
            }
            cad_app::DrawIntent::Circle { center, edge } => {
                self.send(
                    CommandId::CreateCircle,
                    CommandPayload::Points(vec![center, edge]),
                );
                Ok(())
            }
            cad_app::DrawIntent::Move { delta } => {
                let refs: Vec<SelectionRef> = self.controller.borrow().selection().refs().to_vec();
                if refs.is_empty() {
                    return Err(CadError::InvalidInput("move needs a selection".into()));
                }
                self.send(
                    CommandId::MoveEntities,
                    CommandPayload::Move { refs, delta },
                );
                Ok(())
            }
            cad_app::DrawIntent::Trim {
                target_pick,
                boundary_pick,
            } => {
                let tolerance = pick_tolerance();
                let (target, boundary) = {
                    let c = self.controller.borrow();
                    let target = world_pick_ref(&c, target_pick, tolerance)?;
                    let boundary = world_pick_ref(&c, boundary_pick, tolerance)?;
                    (target, boundary)
                };
                self.send(
                    CommandId::TrimEntity,
                    CommandPayload::Trim {
                        target,
                        boundary: vec![boundary],
                        pick_point: target_pick,
                    },
                );
                Ok(())
            }
        }
    }
}

/// Forwards the shell's live draw preview into the CAD overlay.
pub(crate) struct AndroidDrawPreviewSink {
    view: SharedView,
}

impl cad_ui_slint::DrawPreviewSink for AndroidDrawPreviewSink {
    fn set_preview(&self, preview: Option<cad_app::DrawPreview>) {
        if let Some(view) = self.view.borrow().as_ref() {
            view.set_draw_preview(preview);
        }
    }
}

/// Install the draw command and preview sinks on the adapter once the view
/// exists. Until this runs, a confirm reports "not wired" rather than faking a
/// success.
pub(crate) fn install_draw_sinks(
    adapter: &cad_ui_slint::UiAdapter,
    controller: Rc<RefCell<HostController>>,
    handle: SharedHandle,
    view: SharedView,
    viewport: ViewportId,
) {
    adapter.set_draw_command_sink(Box::new(AndroidDrawSink::new(
        controller,
        handle,
        view.clone(),
        viewport,
    )));
    adapter.set_draw_preview_sink(Rc::new(AndroidDrawPreviewSink { view }));
}
