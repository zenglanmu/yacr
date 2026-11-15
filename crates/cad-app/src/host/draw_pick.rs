//! Capture-time entity pick resolution for the TRIM/EXTEND draw steps.
//!
//! `docs/drawing-edit.md` §2–§4 make TRIM/EXTEND arguments object selections,
//! not free points. This module holds the **single** pick entry both the strict
//! capture resolution and the cursor-time candidate funnel through, so there is
//! no second tolerance, no second ranking, no second predicate and no second
//! identity: exactly the click-selection and hover path
//! ([`crate::picking::pick_at_screen_with_visibility`], the default
//! [`TolerancePolicy`] pickbox aperture, layer-override visibility).

use cad_domain::{CadError, CadResult, Point3, SelectionRef};

use crate::picking::pick_at_screen_with_visibility;
use crate::{BackFacePolicy, DrawToolKind};

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
            &self.session_tolerance(),
            BackFacePolicy::Cull,
            &self.session.layer_overrides,
            &self.session.hidden_entities,
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

    /// The entity ref a click resolves for a pending capture step that **accepts**
    /// a pick, or `None` when that step is DIMANGULAR's **mixed** first step
    /// ("select a line **or** specify a vertex") and the click hit nothing.
    ///
    /// A pick-only step keeps the strict refusal ([`Self::resolve_draw_pick`]), so
    /// an empty-space click can never be captured as a point where the step demands
    /// an entity. `None` therefore only ever means "the three-point form's vertex",
    /// and the caller stores the click as the free point. Callers gate this on
    /// `SessionState::draw_accepts_entity_pick`; a step that takes no pick at all is
    /// never routed here.
    pub(super) fn capture_draw_reference(&self, world: Point3) -> CadResult<Option<SelectionRef>> {
        if self.session.draw_requires_entity_pick() {
            return self.resolve_draw_pick(world).map(Some);
        }
        self.draw_pick_ref(world)
    }

    /// Route a click to the pending capture step when that step accepts a pick.
    ///
    /// Returns `true` when the click was consumed **entirely** here (the ref and
    /// the raw point are stored atomically, plus the per-kind pick validation), and
    /// `false` when the caller must still store it as a free point — either the
    /// step takes no pick, or it is DIMANGULAR's **mixed** first step ("select a
    /// line **or** specify a vertex") and the click hit nothing, which is that
    /// form's vertex.
    ///
    /// A **pick-only** step keeps the strict refusal
    /// ([`Self::resolve_draw_pick`]): an empty-space click can never be captured as
    /// a point where the step demands an entity, so the two failures stay
    /// distinguishable by name.
    pub(super) fn capture_tool_pick(&mut self, point: Point3) -> CadResult<bool> {
        if !self.session.draw_accepts_entity_pick() {
            return Ok(false);
        }
        let Some(reference) = self.capture_draw_reference(point)? else {
            return Ok(false);
        };
        // TEXTEDIT's pick must resolve to a single, directly addressable `Text`
        // **before** the step is consumed: a wrong object is refused by name at
        // capture time (never captured and refused later), and a valid one carries
        // its current string as the UI prefill.
        if self.session.draw_kind() == Some(DrawToolKind::TextEdit) {
            let prefill = self.text_edit_prefill(&reference)?;
            self.session.capture_draw_pick(point, reference)?;
            self.session.set_draw_text_prefill(Some(prefill));
        } else if matches!(
            self.session.draw_kind(),
            Some(DrawToolKind::DimBaseline | DrawToolKind::DimContinue)
        ) {
            // DIMBASELINE/DIMCONTINUE's pick must resolve to a directly addressable
            // **linear** dimension before the step is consumed: a wrong object is
            // refused by name at capture time (ADR 0017 decision C, P5f), so it can
            // never be captured and refused later.
            self.check_dimension_chain_base(reference.clone())?;
            self.session.capture_draw_pick(point, reference)?;
        } else {
            self.session.capture_draw_pick(point, reference)?;
        }
        Ok(true)
    }

    /// Strict capture-time validation for a DIMBASELINE/DIMCONTINUE base pick.
    ///
    /// The ref must resolve to a directly addressable `DimensionKind::Linear`
    /// dimension: a block-instance member / sub-element ref, every other dimension
    /// kind (aligned, angular, radial, ordinate) and every non-dimension geometry
    /// are refused **by name before the step is consumed**, so the capture can
    /// never be completed with a base the command would later reject. The command
    /// re-runs the same resolution at apply time, so a base deleted or replaced
    /// between the pick and the Confirm is still refused there.
    pub(super) fn check_dimension_chain_base(&self, reference: SelectionRef) -> CadResult<()> {
        let command = match self.session.draw_kind() {
            Some(DrawToolKind::DimContinue) => "DIMCONTINUE",
            _ => "DIMBASELINE",
        };
        let document = self
            .application
            .workspace
            .documents
            .get(&self.document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        crate::app_drawing::ensure_chain_base(document.drawing.as_ref(), &reference, command)
    }
}
