//! `LAYOUT COPY`: duplicate one paper layout — its viewports with their clips,
//! scales and layer overrides, its paper-space entities and its stored page
//! setup — in one transaction / one undo step.
//!
//! The row shape is [`cad_db::LayoutRow`] (layout + plot configuration), so the
//! copy goes through the sanctioned `cad-db` batch write
//! ([`cad_db::DrawingDatabase::apply_drawing_changes_with_blocks_display_layers_and_layouts`])
//! and never edits the tables directly. `cad-db` already owns create/rename/
//! delete; this module adds the **duplicate** form without a second write path.
//!
//! `LAYOUT TEMPLATE` (import a layout from a DWT/DXF template file) is not
//! modelled by this build and is refused by name in the command parser
//! (`layout.template_not_implemented`), never mapped onto `COPY`.
//!
//! Declared from `app_layout` with an explicit `#[path]` so this module keeps
//! that file inside the 1000-line code gate; it is the same `impl Application`
//! and uses the same layout patches as the other layout writes.

use super::*;

impl Application {
    /// `LAYOUT COPY <source> <new>`: duplicate one layout in one undo step.
    ///
    /// The new row is the source's [`cad_db::Layout`] — the same viewport vector,
    /// hence the same clips, `model_to_paper` scales and `layer_overrides` — under
    /// the next allocated id and the validated new name, plus the source's stored
    /// `PLOTSETTINGS` row re-keyed to the new id (marked
    /// [`cad_db::PlotProvenance::Authored`], exactly like `PSETUPIN`: this build
    /// wrote the copy).
    ///
    /// The source's paper-space **entities** are duplicated into the new layout
    /// with fresh ids and their source draw order. The one exception is an
    /// `AcDbViewport` frame entity: that *is* the viewport machinery (the
    /// viewports themselves are copied as layout rows), so duplicating the frame
    /// would draw a second boundary for a viewport that has no stored state. Those
    /// frames are skipped and reported by name (`layout.copy.viewport_frame`),
    /// never silently dropped.
    ///
    /// One validated combined write (entity inserts + the layout upsert) raises
    /// the revision once and records one undo step, so UNDO removes the row and
    /// the copied entities together. Refusals (unknown/ambiguous source number,
    /// invalid name, duplicate name, locked layer) write nothing anywhere.
    pub(super) fn copy_layout(
        &mut self,
        command: &Command,
        source: &str,
        name: &str,
    ) -> CadResult<CommandOutcome> {
        let document_id = command.document;
        let drawing = self.drawing(document_id)?.clone();
        let (source_id, source_name, _) = resolve_layout_name(&drawing, source)?;
        let name = validate_layout_name(name)?;
        if find_layout_named(&drawing, &name, Some(source_id)).is_some() {
            return Err(CadError::InvalidInput(format!(
                "layout.duplicate_name: a layout named {:?} already exists",
                name
            )));
        }
        let source_layout = drawing.layout(source_id).cloned().ok_or_else(|| {
            CadError::Invariant(format!("LAYOUT COPY: layout {source_id:?} vanished"))
        })?;
        // The id comes from the same monotonic allocator `create_layout` uses, so
        // a deleted id is never handed out again.
        let id = LayoutId(drawing.next_layout_id().0.max(1));
        let plot = drawing.plot_settings(source_id).cloned().map(|mut record| {
            record.layout = id;
            record.provenance = cad_db::PlotProvenance::Authored;
            record
        });
        let mut candidate = drawing;
        let sources: Vec<cad_db::DbEntity> = candidate
            .entities()
            .filter(|entity| entity.space == SpaceId::Paper(source_id))
            .cloned()
            .collect();
        let mut entity_changes: Vec<(EntityId, Option<cad_db::DbEntity>)> =
            Vec::with_capacity(sources.len());
        let mut inserted: Vec<cad_db::DbEntity> = Vec::with_capacity(sources.len());
        let mut skipped_viewport_frames = 0usize;
        for before in sources {
            if before.object.type_key == "AcDbViewport" {
                skipped_viewport_frames += 1;
                continue;
            }
            let new_id = candidate.allocate_entity_id();
            let mut after = before;
            after.id = new_id;
            after.object.id = ObjectId(new_id.0);
            after.object.revision = Revision(0);
            after.object.source_handle = None;
            after.space = SpaceId::Paper(id);
            inserted.push(after.clone());
            entity_changes.push((new_id, Some(after)));
        }
        let viewport_count = source_layout.viewports.len();
        let row = cad_db::LayoutRow {
            layout: cad_db::Layout {
                id,
                name: name.clone(),
                viewports: source_layout.viewports,
            },
            plot,
        };
        let transaction = next_transaction_id();
        let change_set = candidate.apply_drawing_changes_with_blocks_display_layers_and_layouts(
            "layout-copy",
            transaction,
            &[],
            entity_changes,
            &[],
            &[],
            &[(id, Some(row))],
        )?;
        let after_row = layout_row(&candidate, id);
        let entity_count = inserted.len();
        let objects: Vec<ObjectId> = inserted
            .iter()
            .map(|entity| ObjectId(entity.id.0))
            .collect();
        // Every copied entity is an **insertion** with a fresh id, so its undo
        // patch has no before state; the layout row is an insertion too.
        let patches = inserted
            .into_iter()
            .map(|after| drawing_patch(after.id, None, Some(after)))
            .collect();
        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction,
                label: "layout-copy".to_string(),
                patches,
                block_patches: Vec::new(),
                layer_patches: Vec::new(),
                style_patches: Vec::new(),
                dim_style_patches: Vec::new(),
                mleader_style_patches: Vec::new(),
                layout_patches: vec![layout_patch(id, None, after_row)],
                merge_key: None,
            },
        )?;
        let mut diagnostics = vec![Diagnostic {
            object: None,
            code: "layout.copied".into(),
            message: format!(
                "LAYOUT COPY: created layout {} ({:?}) as a copy of layout {} ({:?}) — {viewport_count} \
                 viewport(s) with their clips, scales and layer overrides, {entity_count} paper-space \
                 entity/entities with fresh ids and the stored page setup (as an authored record); \
                 one transaction / one undo step",
                id.0, name, source_id.0, source_name
            ),
        }];
        if skipped_viewport_frames > 0 {
            diagnostics.push(Diagnostic {
                object: None,
                code: "layout.copy.viewport_frame".into(),
                message: format!(
                    "LAYOUT COPY: skipped {skipped_viewport_frames} AcDbViewport frame \
                     entity/entities of layout {} — the viewport definitions themselves were \
                     copied as layout viewports, so a duplicated frame would be a boundary \
                     without a stored viewport",
                    source_id.0
                ),
            });
        }
        Ok(CommandOutcome {
            objects,
            changes: Some(Arc::new(change_set)),
            diagnostics,
            measurement: None,
            point_report: None,
        })
    }
}
