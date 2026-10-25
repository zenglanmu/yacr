//! Host-side grip dragging: arming a press on a handle, tracking the drag,
//! committing one `GripStretch` on release, and the marker preview.
//!
//! This is the pointer half of the grip model in `cad_app::grips`. It owns **no**
//! geometry rule: which handles an entity exposes
//! (`grips_for_entity`), which one a plan-view point hits (`grip_hit_planar`)
//! and what a drag writes (`apply_grip`, through the `GRIPSTRETCH` command) all
//! stay in `cad_app`. Similarly there is no second grip enumeration and no
//! second hit authority here.
//!
//! ## One arbiter, at press time
//!
//! [`HostCore::pointer_down`] is the only place that holds the drawing, the
//! selection, the camera and the pickbox policy at the moment the gesture
//! starts, and it is already the single gesture arbiter (box / pan / orbit). A
//! press that lands on a grip of the selection therefore arms the drag **there**,
//! before any box/pan decision can claim the pointer, and every later phase
//! simply asks "is a drag armed?":
//!
//! * `pointer_move` services [`HostCore::track_grip_drag`] first, before its
//!   capture / modal-pan / box-gesture early returns and before the drag
//!   threshold gate — otherwise the drag stream would die on the first move;
//! * `pointer_up` commits **before** the box branch (a release that ends a grip
//!   drag is not a rubber band) and sets the click-suppression latch;
//! * `pointer_cancel`, the stale `Up`/`Cancel` queue path and `clear_gestures`
//!   drop the drag, which is a pure state drop because nothing was written yet.
//!
//! With no grip under the press nothing here is armed and pan/orbit/box-select
//! behave exactly as before.
//!
//! ## Arming gates
//!
//! A press arms only when **all** of these hold:
//!
//! * the view is 2D (`mode_is_3d` is false) — grip markers are not drawn in 3D
//!   and 3D grip editing is NOT IMPLEMENTED;
//! * no draw/measurement capture owns the pointer (`capturing`);
//! * the tool state is `Idle` or `Selecting` (the states a selection can be
//!   pointed at in; a modal pan or capture owns the pointer instead);
//! * the press button is the **primary** one, the same button the session
//!   requires for a click (`PointerPress::is_click`): a middle-button press on a
//!   grip must stay a pan, never a geometry edit;
//! * the selection yields at least one **visible**, directly addressable entity
//!   (no instance path, no sub-element) exposing a grip within the pick
//!   tolerance of the pointer.
//!
//! A selected **paper-space** entity still exposes its handles (the marker comes
//! from the selection, exactly like the highlight) but the commit is refused by
//! the command with `property.not_addressable`, which is an explicit refusal —
//! no write, no revision bump, no undo entry. Addressability is the command's
//! single authority; this module deliberately does not grow a second one.
//!
//! ## The two distances
//!
//! Hit-testing uses `grip_hit_planar` (x/y distance) with the **same**
//! `pick_tolerance` and the **same** selection/visibility predicates the click
//! and hover picks use, because a pointer resolves through
//! `Viewport::screen_to_world`, which always lands on the work plane. The drag
//! **target** keeps the dragged handle's own `z`, so a plan-view drag never
//! changes an elevation (3D grip editing is NOT IMPLEMENTED). Only a finite
//! target is ever stored.
//!
//! ## The preview
//!
//! `OverlayInputs::grip_markers` is fed by
//! [`HostCore::grip_marker_positions`]: one entry per grip of the visible
//! selected entities, with the dragged entry replaced by `to`. The moved marker
//! **is** the preview, so no extra overlay input or ghost marker is added and no
//! wire-protocol change is needed.

use super::*;
use crate::core::GripDrag;
use cad_app::grips::{grip_hit_planar, grips_for_entity, Grip, REASON_ZERO_DISPLACEMENT};
use cad_db::DbEntity;

impl HostCore {
    /// Arm a grip drag if `logical` lands on a handle of the current selection.
    ///
    /// Returns whether a drag was armed; the gates are listed in the module doc.
    /// On success the drag starts with `to == from`, so nothing has moved yet and
    /// the marker preview is identical to the static handle.
    pub(super) fn arm_grip_at(&self, logical: [f64; 2]) -> bool {
        if self.mode_is_3d() || self.capturing() || self.grip_drag_active() {
            return false;
        }
        let tool_allows_selection = {
            let c = self.controller.borrow();
            matches!(&c.session.tool, ToolState::Idle | ToolState::Selecting)
        };
        if !tool_allows_selection {
            return false;
        }
        let Some(grip) = self.grip_at(logical) else {
            return false;
        };
        *self.navigation.grip_drag.borrow_mut() = Some(GripDrag {
            id: grip.id,
            from: grip.position,
            to: grip.position,
        });
        // The dragged marker is the preview: an idle hover tint must not compete
        // with it (the press already cleared hover, this keeps the invariant
        // local to the arming decision).
        self.clear_hover();
        true
    }

    /// Whether a grip drag is currently armed.
    ///
    /// Visible crate-wide so the interaction dispatcher can demote a grip-drag
    /// frame to an overlay-only change and the synthetic contracts can observe
    /// the armed state. It is **public** because `cad-ui-core` projects it onto
    /// `cad_status.gripDrag`: the desktop UI has to know a drag owns the pointer
    /// to route Escape to `CANCEL`, and the host is the only authority for that
    /// fact (`cad-ui-core` must not keep a second copy of a gesture).
    pub fn grip_drag_active(&self) -> bool {
        self.navigation.grip_drag.borrow().is_some()
    }

    /// Track the armed drag to `logical`.
    ///
    /// Returns whether a drag is armed, so the pointer path can stop before its
    /// capture / modal-pan / box / threshold branches. An armed drag whose target
    /// cannot be resolved (a degenerate projection, a non-finite point) keeps its
    /// last finite target and still reports `true`: the pointer belongs to the
    /// drag either way, and a fabricated target is never stored.
    pub(super) fn track_grip_drag(&self, logical: [f64; 2]) -> bool {
        let Some(target) = self.drag_target(logical) else {
            return self.grip_drag_active();
        };
        let mut drag = self.navigation.grip_drag.borrow_mut();
        match drag.as_mut() {
            Some(drag) => {
                drag.to = target;
                true
            }
            None => false,
        }
    }

    /// Commit the armed drag as **one** `GripStretch`.
    ///
    /// The command is the same immediate-command funnel every other host gesture
    /// uses, so it re-reads the entity, re-validates the handle against the
    /// current geometry and owns the single transaction / undo entry.
    ///
    /// A release that did not move the handle (or a model-level
    /// `grip.zero_displacement`) is an explicit **no-op**: no command is
    /// dispatched, so no transaction, no undo entry and no status error exists to
    /// surface. Every other refusal is propagated normally.
    pub(super) fn commit_grip_drag(&self) -> CadResult<()> {
        let Some(drag) = self.navigation.grip_drag.borrow_mut().take() else {
            return Ok(());
        };
        if drag.to == drag.from {
            // Nothing moved: the model would refuse this with
            // `grip.zero_displacement`, and a drag in place is not an edit.
            return Ok(());
        }
        let result = self.immediate_command(
            CommandId::GripStretch,
            CommandPayload::GripStretch(Box::new(cad_app::GripStretchCommand {
                id: drag.id,
                to: drag.to,
            })),
        );
        match result {
            // The model's zero-displacement refusal is a documented no-op here,
            // never a user-visible `grip.*` error: it can only arise when a
            // concurrent edit already put the handle where the drag points.
            Err(CadError::InvalidInput(message))
                if message.starts_with(REASON_ZERO_DISPLACEMENT) =>
            {
                Ok(())
            }
            other => other,
        }
    }

    /// Drop an armed drag without writing anything.
    ///
    /// Nothing is stored before the release commit, so this is a pure state drop
    /// (used by a cancel, a stale `Up`/`Cancel`, a document/metrics/config
    /// change, and a second contact taking the pointer).
    pub(crate) fn cancel_grip_drag(&self) {
        *self.navigation.grip_drag.borrow_mut() = None;
    }

    /// Drop the whole grip gesture: the armed drag **and** any pending click
    /// swallow.
    ///
    /// Every terminal path that is not a release commit uses this one entry, so
    /// a dropped drag can never leave a stale latch behind to swallow a later
    /// click. Pure state: nothing was written before the commit.
    pub(crate) fn reset_grip_gesture(&self) {
        self.cancel_grip_drag();
        self.navigation.suppress_click.set(false);
    }

    /// The marker positions reaching `OverlayInputs::grip_markers`.
    ///
    /// One entry per grip of the visible selected entities, in selection order
    /// and then handle-index order, with the **dragged** handle's entry replaced
    /// by the current drag target — the moved marker is the drag preview. It is
    /// empty when nothing is selected, when the selected entities expose no grip
    /// (see the refusal list in `cad_app::grips`), and when a selected entity is
    /// hidden under the session's layer overrides.
    pub(crate) fn grip_marker_positions(&self) -> Vec<Point3> {
        let entities = self.grip_candidates();
        if entities.is_empty() {
            return Vec::new();
        }
        let drag = *self.navigation.grip_drag.borrow();
        let mut positions = Vec::new();
        for entity in &entities {
            for grip in grips_for_entity(entity) {
                positions.push(match drag {
                    Some(drag) if drag.id == grip.id => drag.to,
                    _ => grip.position,
                });
            }
        }
        positions
    }

    /// The world point the armed drag currently names.
    ///
    /// The pointer resolves on the work plane (`screen_to_world`), but the target
    /// keeps the dragged handle's own `z`: a plan-view drag moves the handle in
    /// x/y only. A non-finite result is refused rather than stored.
    fn drag_target(&self, logical: [f64; 2]) -> Option<Point3> {
        let from = self.navigation.grip_drag.borrow().as_ref()?.from;
        let c = self.controller.borrow();
        let viewport = c.application.workspace.viewports.get(&c.viewport_id)?;
        let world = viewport.screen_to_world(logical, viewport.logical_size)?;
        let target = Point3 {
            x: world.x,
            y: world.y,
            z: from.z,
        };
        is_finite_point(target).then_some(target)
    }

    /// The grip under `logical`, through the model's plan-view hit rule.
    ///
    /// The tolerance is the configured pickbox in world units
    /// (`cad_app::picking::pick_tolerance`) — the **same** radius a click and the
    /// hover pick use, so a press can never aim at a handle the indicator would
    /// not show. No second tolerance and no second projection exist.
    fn grip_at(&self, logical: [f64; 2]) -> Option<Grip> {
        // No `metrics()` call here: the logical position was already validated
        // against the current metrics revision by `pointer_logical`, and
        // `metrics()` may drop in-flight gestures on a metrics change — it must
        // never run between arming and the first move.
        let entities = self.grip_candidates();
        if entities.is_empty() {
            return None;
        }
        let (world, tolerance) = {
            let c = self.controller.borrow();
            let viewport = c.application.workspace.viewports.get(&c.viewport_id)?;
            let world = viewport.screen_to_world(logical, viewport.logical_size)?;
            let tolerance = cad_app::picking::pick_tolerance(
                &self.tolerance_policy(),
                &viewport.camera,
                viewport.logical_size,
            )
            .ok()?;
            (world, tolerance)
        };
        grip_hit_planar(entities.iter(), world, tolerance)
    }

    /// The selected entities grips may be drawn for and armed from.
    ///
    /// Only refs of the **current document** that address an entity directly are
    /// candidates: a `GripId` carries no instance path and no sub-element, so a
    /// block-member instance (reachable only through an `INSERT`) and an
    /// entity-level sub-element can never be expressed as a grip. Visibility
    /// reuses `cad_app::picking::ref_is_visible` — the exact predicate the click
    /// and hover picks apply — so a grip on a hidden layer neither appears nor
    /// arms.
    fn grip_candidates(&self) -> Vec<DbEntity> {
        let c = self.controller.borrow();
        let Some(drawing) = c.drawing() else {
            return Vec::new();
        };
        let document = c.document_id;
        c.session
            .selection
            .refs()
            .iter()
            .filter(|reference| {
                reference.document == document
                    && reference.instance.0.is_empty()
                    && reference.sub_element.is_none()
                    && cad_app::picking::ref_is_visible(
                        &drawing,
                        &c.session.layer_overrides,
                        reference,
                    )
            })
            .filter_map(|reference| drawing.entity(reference.entity).cloned())
            .collect()
    }
}

/// Whether a world point is storable (no `NaN`, no infinity).
fn is_finite_point(point: Point3) -> bool {
    point.x.is_finite() && point.y.is_finite() && point.z.is_finite()
}
