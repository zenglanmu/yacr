//! The one record funnel and its per-lane wrappers.
//!
//! Each wrapper names exactly the lane a command wrote (linetype, BIM,
//! parameters/bindings, groups, views, materials, scenes) and forwards to
//! [`History::record_drawing_step`], which builds the step, validates it as a
//! whole and merges/evicts under the memory budget. Split out of `lib.rs` for the
//! code-size gate; the step type and its validator stay in the crate root.

use super::*;

impl History {
    /// Record a committed drawing-entity change. Clears the redo stack.
    ///
    /// Coalescing keeps the earliest before state and the latest after state.
    /// Patches that return to their original state are removed; if all patches
    /// cancel out, the merged entry is removed without touching earlier entries.
    /// Even a fully canceled merge clears redo, since new edits were committed.
    /// An empty patch list is rejected, so a command can never claim success
    /// while recording nothing.
    pub fn record_drawing(&mut self, record: DrawingUndoRecord) -> CadResult<()> {
        self.record_drawing_step(
            record,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
    }

    /// Record a committed drawing change plus the linetype-table rows it wrote.
    ///
    /// [`Self::record_drawing`] is the common shape (no linetype rows). The
    /// linetype batch is carried beside the record (see [`DrawingHistoryStep`]),
    /// is never coalesced, and counts toward "this step records something", so a
    /// linetype-only rename is still one real undo step.
    pub fn record_drawing_with_linetypes(
        &mut self,
        record: DrawingUndoRecord,
        linetype_patches: Vec<cad_db::LinetypePatch>,
    ) -> CadResult<()> {
        self.record_drawing_step(
            record,
            linetype_patches,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
    }

    /// Record a committed BIM property edit plus its element patches.
    ///
    /// The patches are carried beside the record like the linetype batch (see
    /// [`DrawingHistoryStep`]), are never coalesced, and count toward "this step
    /// records something", so a BIM-only edit is one real undo step. The record
    /// must be empty: the BIM batch is applied by its own database path and a
    /// mixed step would make one undo span two revisions (see
    /// `DrawingHistoryStep::validate`).
    pub fn record_drawing_with_bim(
        &mut self,
        record: DrawingUndoRecord,
        bim_patches: Vec<cad_db::BimPatch>,
    ) -> CadResult<()> {
        self.record_drawing_step(
            record,
            Vec::new(),
            bim_patches,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
    }

    /// Record a committed parametric edit plus the parameter and binding rows it
    /// wrote (ADR-0043 D8).
    ///
    /// The patches are carried beside the record like the linetype/BIM batches
    /// (see [`DrawingHistoryStep`]), are never coalesced, and count toward "this
    /// step records something". A parametric step carries **no** record entity or
    /// table rows: the derived geometry travels on
    /// [`cad_db::ParamBindingPatch::before_geometry`]/`after_geometry` instead,
    /// and `validate()` refuses any non-empty record batch. `validate()` runs
    /// before the step is recorded, so a refused mix leaves the retained history
    /// untouched.
    pub fn record_drawing_with_params(
        &mut self,
        record: DrawingUndoRecord,
        param_patches: Vec<cad_db::ParamPatch>,
        param_binding_patches: Vec<cad_db::ParamBindingPatch>,
    ) -> CadResult<()> {
        self.record_drawing_step(
            record,
            Vec::new(),
            Vec::new(),
            param_patches,
            param_binding_patches,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
    }

    /// Record a committed group edit plus the rows it wrote.
    pub fn record_drawing_with_groups(
        &mut self,
        record: DrawingUndoRecord,
        group_patches: Vec<cad_db::GroupPatch>,
    ) -> CadResult<()> {
        self.record_drawing_step(
            record,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            group_patches,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
    }

    /// Record a committed named-view table write (`VIEW SAVE`/`DELETE`).
    pub fn record_drawing_with_views(
        &mut self,
        record: DrawingUndoRecord,
        view_patches: Vec<cad_db::ViewPatch>,
    ) -> CadResult<()> {
        self.record_drawing_step(
            record,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            view_patches,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
    }

    /// Record a committed material write (`PAINT`).
    pub fn record_drawing_with_materials(
        &mut self,
        record: DrawingUndoRecord,
        material_patches: Vec<cad_db::MaterialPatch>,
    ) -> CadResult<()> {
        self.record_drawing_step(
            record,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            material_patches,
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
    }

    /// Record a committed scene write (`SCENE SAVE`/`DELETE`).
    pub fn record_drawing_with_scenes(
        &mut self,
        record: DrawingUndoRecord,
        scene_patches: Vec<cad_db::ScenePatch>,
    ) -> CadResult<()> {
        self.record_drawing_step(
            record,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            scene_patches,
            Vec::new(),
            Vec::new(),
        )
    }

    /// Record a committed display-settings write (`SHADOWS`/`FOG`).
    pub fn record_drawing_with_display(
        &mut self,
        record: DrawingUndoRecord,
        display_patches: Vec<cad_db::DisplayPatch>,
    ) -> CadResult<()> {
        self.record_drawing_step(
            record,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            display_patches,
            Vec::new(),
        )
    }

    /// Record a committed construction-guide write (`GUIDE`/measurement guide).
    pub fn record_drawing_with_guides(
        &mut self,
        record: DrawingUndoRecord,
        guide_patches: Vec<cad_db::GuidePatch>,
    ) -> CadResult<()> {
        self.record_drawing_step(
            record,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            guide_patches,
        )
    }

    // The one record funnel: each table lane adds one patch vector, and the
    // wrappers above keep every caller explicit about what it wrote (the same
    // reason the record struct carries them all). A struct here would only move
    // the list, not shorten it.
    #[allow(clippy::too_many_arguments)]
    fn record_drawing_step(
        &mut self,
        record: DrawingUndoRecord,
        linetype_patches: Vec<cad_db::LinetypePatch>,
        bim_patches: Vec<cad_db::BimPatch>,
        param_patches: Vec<cad_db::ParamPatch>,
        param_binding_patches: Vec<cad_db::ParamBindingPatch>,
        group_patches: Vec<cad_db::GroupPatch>,
        view_patches: Vec<cad_db::ViewPatch>,
        material_patches: Vec<cad_db::MaterialPatch>,
        scene_patches: Vec<cad_db::ScenePatch>,
        display_patches: Vec<cad_db::DisplayPatch>,
        guide_patches: Vec<cad_db::GuidePatch>,
    ) -> CadResult<()> {
        self.ensure_no_pending_drawing()?;
        let mut step = DrawingHistoryStep {
            record,
            linetype_patches,
            bim_patches,
            param_patches,
            param_binding_patches,
            group_patches,
            view_patches,
            material_patches,
            scene_patches,
            display_patches,
            guide_patches,
        };
        step.validate()?;
        if let Some(key) = step.record.merge_key.clone() {
            if let Some(last) = self.undo.back_mut() {
                if last.record.merge_key.as_deref() == Some(key.as_str()) {
                    for patch in std::mem::take(&mut step.record.patches) {
                        if let Some(existing) =
                            last.record.patches.iter_mut().find(|p| p.id == patch.id)
                        {
                            existing.after = patch.after;
                        } else {
                            last.record.patches.push(patch);
                        }
                    }
                    last.record
                        .patches
                        .retain(|patch| patch.before != patch.after);
                    if last.record.patches.is_empty() {
                        self.undo.pop_back();
                    }
                    self.recompute_used_bytes();
                    self.redo.clear();
                    self.enforce_budget();
                    return Ok(());
                }
            }
        }
        self.used_bytes += step.approx_bytes();
        self.undo.push_back(step);
        self.redo.clear();
        self.enforce_budget();
        Ok(())
    }
}
