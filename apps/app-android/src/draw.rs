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
//! invented [`cad_domain::SelectionRef`]. The generic preview sink is the shared
//! `HostCore`; only the command mapping is host-specific here because the shell
//! enables Trim/Extend, which the desktop core refuses.

use super::*;
use cad_app::{Command, CommandId, CommandPayload};
use cad_host::HostCore;
use cad_ui_slint::DrawCommandSink;

/// Map a world point on the work plane to the closest model-space entity.
///
/// Trim is planar (`docs/drawing-edit.md` §3), so the hit test is a `-Z` ray
/// through the point; a miss is an explicit error rather than a fabricated ref.
fn world_pick_ref(
    controller: &cad_app::host::HostController,
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
    core: HostCore,
    viewport: ViewportId,
}

impl AndroidDrawSink {
    pub(crate) fn new(core: HostCore, viewport: ViewportId) -> Self {
        Self { core, viewport }
    }

    /// Send one drawing command through the shared funnel so the status line,
    /// panels and overlays refresh exactly once.
    fn send(&self, id: CommandId, payload: CommandPayload) -> CadResult<()> {
        let document = self.core.controller.borrow().document_id;
        let command = Command {
            schema_version: 1,
            id,
            document,
            viewport: self.viewport,
            payload,
        };
        state_push::run_command(&self.core, command)
    }

    /// The current session selection refs, or an explicit error when empty.
    fn selection_refs(&self, what: &str) -> CadResult<Vec<SelectionRef>> {
        let refs: Vec<SelectionRef> = self.core.controller.borrow().selection().refs().to_vec();
        if refs.is_empty() {
            return Err(CadError::InvalidInput(format!("{what} needs a selection")));
        }
        Ok(refs)
    }
}

impl DrawCommandSink for AndroidDrawSink {
    fn commit(&mut self, intent: cad_app::DrawIntent) -> CadResult<()> {
        match intent {
            cad_app::DrawIntent::Line { start, end } => self.send(
                CommandId::CreateLine,
                CommandPayload::Points(vec![start, end]),
            ),
            cad_app::DrawIntent::Circle { center, edge } => self.send(
                CommandId::CreateCircle,
                CommandPayload::Points(vec![center, edge]),
            ),
            cad_app::DrawIntent::Arc { points } => self.send(
                CommandId::CreateArc,
                CommandPayload::Points(points.to_vec()),
            ),
            cad_app::DrawIntent::Move { delta } => {
                let refs = self.selection_refs("move")?;
                self.send(
                    CommandId::MoveEntities,
                    CommandPayload::Move { refs, delta },
                )
            }
            cad_app::DrawIntent::Copy { delta } => {
                let refs = self.selection_refs("copy")?;
                self.send(
                    CommandId::CopyEntities,
                    CommandPayload::Copy { refs, delta },
                )
            }
            cad_app::DrawIntent::Rotate { base, angle } => {
                let refs = self.selection_refs("rotate")?;
                self.send(
                    CommandId::RotateEntities,
                    CommandPayload::Rotate { refs, base, angle },
                )
            }
            cad_app::DrawIntent::Scale { base, factor } => {
                let refs = self.selection_refs("scale")?;
                self.send(
                    CommandId::ScaleEntities,
                    CommandPayload::Scale { refs, base, factor },
                )
            }
            cad_app::DrawIntent::Mirror { p1, p2 } => {
                let refs = self.selection_refs("mirror")?;
                self.send(
                    CommandId::MirrorEntities,
                    CommandPayload::Mirror {
                        refs,
                        axis_start: p1,
                        axis_end: p2,
                    },
                )
            }
            cad_app::DrawIntent::Offset { side_point } => {
                let refs = self.selection_refs("offset")?;
                self.send(
                    CommandId::OffsetEntity,
                    CommandPayload::Offset { refs, side_point },
                )
            }
            cad_app::DrawIntent::Trim {
                target_pick,
                boundary_pick,
            } => {
                let (target, boundary) = self.resolve_trim(target_pick, boundary_pick)?;
                self.send(
                    CommandId::TrimEntity,
                    CommandPayload::Trim {
                        target,
                        boundary: vec![boundary],
                        pick_point: target_pick,
                    },
                )
            }
            cad_app::DrawIntent::Extend {
                target_pick,
                boundary_pick,
            } => {
                let (target, boundary) = self.resolve_trim(target_pick, boundary_pick)?;
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

impl AndroidDrawSink {
    /// Resolve a trim/extend pair of world picks to `SelectionRef`s.
    ///
    /// The tolerance comes from the shared default policy and the real viewport;
    /// either unresolved pick is an explicit error, never a fabricated ref.
    fn resolve_trim(
        &self,
        target_pick: Point3,
        boundary_pick: Point3,
    ) -> CadResult<(SelectionRef, SelectionRef)> {
        let c = self.core.controller.borrow();
        let viewport = c
            .application
            .workspace
            .viewports
            .get(&self.viewport)
            .ok_or(CadError::Cancelled)?;
        let tolerance = cad_app::picking::pick_tolerance(
            &cad_domain::TolerancePolicy::default(),
            &viewport.camera,
            viewport.logical_size,
        )?;
        let target = world_pick_ref(&c, target_pick, tolerance)?;
        let boundary = world_pick_ref(&c, boundary_pick, tolerance)?;
        Ok((target, boundary))
    }
}

/// Install the draw command and preview sinks on the adapter once the view
/// exists. Until this runs, a confirm reports "not wired" rather than faking a
/// success. The preview sink is the shared `HostCore`.
pub(crate) fn install_draw_sinks(
    adapter: &cad_ui_slint::UiAdapter,
    core: HostCore,
    viewport: ViewportId,
) {
    adapter.set_draw_command_sink(Box::new(AndroidDrawSink::new(core.clone(), viewport)));
    adapter.set_draw_preview_sink(Rc::new(core));
}
