//! Capture-time entity pick resolution for the TRIM/EXTEND draw steps.
//!
//! `docs/drawing-edit.md` §2–§4 make TRIM/EXTEND arguments object selections,
//! not free points. This module holds the **single** pick entry both the strict
//! capture resolution and the cursor-time candidate funnel through, so there is
//! no second tolerance, no second ranking, no second predicate and no second
//! identity: exactly the click-selection and hover path
//! ([`crate::picking::pick_at_screen_with_visibility`], the default
//! [`TolerancePolicy`] pickbox aperture, layer-override visibility).

use cad_domain::{CadError, CadResult, Point3, SelectionRef, TolerancePolicy};

use crate::picking::pick_at_screen_with_visibility;
use crate::BackFacePolicy;

use super::HostController;

impl HostController {
    /// The entity under `world`, or `None` when nothing is there.
    ///
    /// Reuses the click-selection/hover pick funnel (`pick_at_screen_with_visibility`)
    /// with the default pickbox aperture ([`TolerancePolicy::pickbox_logical_pixels`])
    /// and the session's temporary layer overrides, so a pick can never target an
    /// entity a hidden layer keeps out of the rendered base. `BackFacePolicy::Cull`
    /// matches click-selection and hover exactly.
    ///
    /// A cursor with no projectable screen point has nothing under it (`Ok(None)`).
    /// A missing document or viewport, or a genuine pick failure, is an `Err` —
    /// never a fabricated ref.
    pub(super) fn draw_pick_ref(&self, world: Point3) -> CadResult<Option<SelectionRef>> {
        let document = self
            .application
            .workspace
            .documents
            .get(&self.document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        let viewport = self
            .application
            .workspace
            .viewports
            .get(&self.viewport_id)
            .ok_or_else(|| CadError::InvalidInput("unknown viewport".into()))?;
        let Some(logical) = viewport
            .camera
            .world_to_screen(world, viewport.logical_size)
        else {
            // Outside the projection (or a non-finite world point): there is
            // nothing under the cursor, which is an absence, not an error.
            return Ok(None);
        };
        let report = pick_at_screen_with_visibility(
            &document.drawing,
            self.document_id,
            &viewport.camera,
            logical,
            viewport.logical_size,
            &TolerancePolicy::default(),
            BackFacePolicy::Cull,
            &self.session.layer_overrides,
        )?;
        Ok(report.hit.map(|hit| hit.source))
    }

    /// Strict capture resolution for a TRIM/EXTEND step.
    ///
    /// A miss reuses the existing refusal text
    /// (`"draw pick did not hit supported geometry"`) so the user-facing reason
    /// is unchanged from the previous confirm-time path.
    pub(super) fn resolve_draw_pick(&self, world: Point3) -> CadResult<SelectionRef> {
        self.draw_pick_ref(world)?.ok_or_else(|| {
            CadError::InvalidInput("draw pick did not hit supported geometry".into())
        })
    }
}
