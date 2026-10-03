//! Web-host draw/edit command sink (F-EDIT).
//!
//! The shell captures points and emits a confirmed [`cad_app::DrawIntent`]; the
//! host maps it to exactly one application command through the shared
//! transaction/history path (`docs/drawing-edit.md` §2):
//!
//! * `Line { start, end }` → `CreateLine` + `Points([start, end])`
//! * `Circle { center, edge }` → `CreateCircle` + `Points([center, edge])`
//! * `Move { delta }` → `MoveEntities` with the session selection refs + delta
//! * `Trim { target_pick, boundary_pick }` → `TrimEntity` after resolving the
//!   two world picks to `SelectionRef`s
//!
//! `Move`/`Trim` need the session selection or a hit test, which the shell does
//! not hold; this is why the host — and not the adapter — owns the mapping. An
//! unresolved trim pick is an explicit error (never a payload-free success).
//! The same install forwards the live preview so the canvas draws the rubber
//! band through the existing overlay.

use std::cell::RefCell;
use std::rc::Rc;

use cad_app::host::HostController;
use cad_app::{DrawIntent, DrawPreview, PickOptions};
use cad_domain::{CadError, CadResult, Point3, Ray3, SelectionRef, TolerancePolicy, ViewportId};
use cad_ui_slint::{CadView, DrawCommandSink, DrawPreviewSink};

use super::SharedHandle;

/// The web host's single-slot CAD view holder (matches `browser.rs`).
type SharedView = Rc<RefCell<Option<CadView>>>;

/// Map a world point on the work plane to the closest model-space entity.
///
/// The capture delivers world points (the pick mapper already inverted the
/// screen mapping), so the hit test is a ray along the view direction through
/// the point. Trim is defined for plan geometry (`docs/drawing-edit.md` §3), so
/// a `-Z` ray through the work plane is the honest test; a miss is an explicit
/// error rather than an invented `SelectionRef`.
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
    let options = PickOptions::new(tolerance)?;
    let report = cad_app::pick_ray(drawing.as_ref(), controller.document_id, &ray, &options)?;
    report.hit.map(|hit| hit.source).ok_or_else(|| {
        CadError::InvalidInput(format!(
            "no entity under the trim pick ({:.3}, {:.3})",
            world.x, world.y
        ))
    })
}

/// The web host's draw/edit sink: one command per confirmed intent.
pub(super) struct WebDrawSink {
    controller: Rc<RefCell<HostController>>,
    handle: SharedHandle,
    view: SharedView,
    viewport: ViewportId,
}

impl WebDrawSink {
    /// Send one drawing command through the shared dispatch funnel so the
    /// status line, panels and overlays refresh exactly once.
    fn send(&self, id: cad_app::CommandId, payload: cad_app::CommandPayload) -> CadResult<()> {
        let document = self.controller.borrow().document_id;
        let command = cad_app::Command {
            schema_version: 1,
            id,
            document,
            viewport: self.viewport,
            payload,
        };
        super::input::dispatch(
            &self.controller,
            &self.handle,
            &self.view,
            &self.viewport,
            command,
        )
    }
}

impl DrawCommandSink for WebDrawSink {
    fn commit(&mut self, intent: DrawIntent) -> CadResult<()> {
        use cad_app::{CommandId, CommandPayload};
        match intent {
            DrawIntent::Line { start, end } => self.send(
                CommandId::CreateLine,
                CommandPayload::Points(vec![start, end]),
            ),
            DrawIntent::Circle { center, edge } => self.send(
                CommandId::CreateCircle,
                CommandPayload::Points(vec![center, edge]),
            ),
            DrawIntent::Move { delta } => {
                let refs: Vec<SelectionRef> = {
                    let c = self.controller.borrow();
                    c.selection().refs().to_vec()
                };
                if refs.is_empty() {
                    return Err(CadError::InvalidInput("move needs a selection".into()));
                }
                self.send(
                    CommandId::MoveEntities,
                    CommandPayload::Move { refs, delta },
                )
            }
            DrawIntent::Trim {
                target_pick,
                boundary_pick,
            } => {
                let (target, boundary) = {
                    let c = self.controller.borrow();
                    let viewport = c
                        .application
                        .workspace
                        .viewports
                        .get(&self.viewport)
                        .ok_or(CadError::Cancelled)?;
                    let tolerance = cad_app::picking::pick_tolerance(
                        &TolerancePolicy::default(),
                        &viewport.camera,
                        viewport.logical_size,
                    )?;
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
                )
            }
        }
    }
}

/// Forwards the shell's live draw preview into the CAD overlay.
pub(super) struct WebDrawPreviewSink {
    view: SharedView,
}

impl DrawPreviewSink for WebDrawPreviewSink {
    fn set_preview(&self, preview: Option<DrawPreview>) {
        if let Some(view) = self.view.borrow().as_ref() {
            view.set_draw_preview(preview);
        }
    }
}

/// Install the draw command and preview sinks on the adapter once the view
/// exists. Until this runs, a confirm reports "not wired" rather than faking a
/// success (see `crates/cad-ui-slint/src/draw.rs`).
pub(super) fn install(
    adapter: &cad_ui_slint::UiAdapter,
    controller: Rc<RefCell<HostController>>,
    handle: SharedHandle,
    view: SharedView,
    viewport: ViewportId,
) {
    adapter.set_draw_command_sink(Box::new(WebDrawSink {
        controller,
        handle,
        view: view.clone(),
        viewport,
    }));
    adapter.set_draw_preview_sink(Rc::new(WebDrawPreviewSink { view }));
}
