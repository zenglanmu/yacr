//! Web-host draw/edit command sink (F-EDIT) — retained compatibility facade.
//!
//! With P2-B the engine owns draw/edit: the Slint `EngineAdapter` routes a
//! confirmed capture through `InteractionRequest::Confirm`, and the shared
//! `HostCore` maps the session's [`cad_app::DrawIntent`] to exactly one command
//! (including Trim/Extend pick resolution, now in `cad-app`). The adapter no
//! longer reads the legacy draw-command sink once a client is attached, so the
//! mapping below is kept only as a source-compatible facade and is never
//! double-dispatched.
//!
//! * `Line { start, end }` → `CreateLine` + `Points([start, end])`
//! * `Circle { center, edge }` → `CreateCircle` + `Points([center, edge])`
//! * `Move { delta }` → `MoveEntities` with the session selection refs + delta
//! * `Copy { delta }` → `CopyEntities` with the session selection refs + delta
//! * `Trim { target_pick, boundary_pick }` → `TrimEntity` after resolving the
//!   two world picks to `SelectionRef`s
//!
//! An unresolved trim pick is an explicit error (never a payload-free success).

use cad_app::host::HostController;
use cad_app::{Command, DrawIntent, PickOptions};
use cad_domain::{CadError, CadResult, Point3, Ray3, SelectionRef, TolerancePolicy, ViewportId};
use cad_host::HostCore;
use cad_ui_slint::DrawCommandSink;

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
    core: HostCore,
    viewport: ViewportId,
}

impl WebDrawSink {
    /// Send one drawing command through the shared command funnel so the
    /// status line, panels and overlays refresh exactly once.
    fn send(&self, id: cad_app::CommandId, payload: cad_app::CommandPayload) -> CadResult<()> {
        let document = self.core.controller.borrow().document_id;
        let command = Command {
            schema_version: 1,
            id,
            document,
            viewport: self.viewport,
            payload,
        };
        super::run_command(&self.core, command)
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
            DrawIntent::Arc { points } => self.send(
                CommandId::CreateArc,
                CommandPayload::Points(points.to_vec()),
            ),
            DrawIntent::Move { delta } => {
                let refs: Vec<SelectionRef> = {
                    let c = self.core.controller.borrow();
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
            DrawIntent::Copy { delta } => {
                let refs: Vec<SelectionRef> = {
                    let c = self.core.controller.borrow();
                    c.selection().refs().to_vec()
                };
                if refs.is_empty() {
                    return Err(CadError::InvalidInput("copy needs a selection".into()));
                }
                self.send(
                    CommandId::CopyEntities,
                    CommandPayload::Copy { refs, delta },
                )
            }
            DrawIntent::Rotate { base, angle } => {
                let refs: Vec<SelectionRef> = {
                    let c = self.core.controller.borrow();
                    c.selection().refs().to_vec()
                };
                if refs.is_empty() {
                    return Err(CadError::InvalidInput("rotate needs a selection".into()));
                }
                self.send(
                    CommandId::RotateEntities,
                    CommandPayload::Rotate { refs, base, angle },
                )
            }
            DrawIntent::Scale { base, factor } => {
                let refs: Vec<SelectionRef> = {
                    let c = self.core.controller.borrow();
                    c.selection().refs().to_vec()
                };
                if refs.is_empty() {
                    return Err(CadError::InvalidInput("scale needs a selection".into()));
                }
                self.send(
                    CommandId::ScaleEntities,
                    CommandPayload::Scale { refs, base, factor },
                )
            }
            DrawIntent::Mirror { p1, p2 } => {
                let refs: Vec<SelectionRef> = {
                    let c = self.core.controller.borrow();
                    c.selection().refs().to_vec()
                };
                if refs.is_empty() {
                    return Err(CadError::InvalidInput("mirror needs a selection".into()));
                }
                self.send(
                    CommandId::MirrorEntities,
                    CommandPayload::Mirror {
                        refs,
                        axis_start: p1,
                        axis_end: p2,
                    },
                )
            }
            DrawIntent::Offset { side_point } => {
                let refs: Vec<SelectionRef> = {
                    let c = self.core.controller.borrow();
                    c.selection().refs().to_vec()
                };
                if refs.is_empty() {
                    return Err(CadError::InvalidInput("offset needs a selection".into()));
                }
                self.send(
                    CommandId::OffsetEntity,
                    CommandPayload::Offset { refs, side_point },
                )
            }
            DrawIntent::Trim {
                target_pick,
                boundary_pick,
            } => {
                let (target, boundary) = {
                    let c = self.core.controller.borrow();
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
            DrawIntent::Extend {
                target_pick,
                boundary_pick,
            } => {
                let (target, boundary) = {
                    let c = self.core.controller.borrow();
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
                    CommandId::ExtendEntity,
                    CommandPayload::Extend {
                        target,
                        boundary: vec![boundary],
                        pick_point: target_pick,
                    },
                )
            }
        }
    }
}

/// Install the legacy draw command sink on the adapter.
///
/// Once a client is attached the adapter routes a confirmed capture through the
/// engine (`InteractionRequest::Confirm`), so this sink is never called; it is
/// retained as a source-compatible facade. The live rubber-band preview travels
/// through `HostUpdates` (engine-owned), not a host preview sink.
pub(super) fn install(adapter: &cad_ui_slint::UiAdapter, core: HostCore, viewport: ViewportId) {
    adapter.set_draw_command_sink(Box::new(WebDrawSink { core, viewport }));
}
