//! The 2026-11-07 layout / viewport command implementations.
//!
//! The **read** commands (`LAYOUT LIST`, `LAYOUT SET`, `VPMAX`, `VPMIN`) are
//! real, and so are the table/viewport **writes** promoted now that `cad-db`
//! owns the sanctioned post-import layout write paths: `LAYOUT NEW|RENAME|DELETE`,
//! `LAYOUTMERGE`, `LAYOUTWIZARD` (including its optional authored page setup),
//! `MVIEW`, `VPORTS`/`-VPORTS`, `VPCLIP RECT`, `VPSCALE` and `LAYVPI` each
//! publish exactly one revision and one undo step. The typed
//! `PAGESETUP`/`PSETUPIN` writes live here too (`page_setup_write`). `LAYVPI`
//! writes real per-viewport layer overrides
//! ([`cad_db::PaperViewport::layer_overrides`]); because this build has no
//! viewport selection, it applies them to every viewport of the active layout
//! and never touches the global layer table. The full boundary is in
//! [`crate::command_id::layout`].

use super::*;

use cad_db::{DrawingDatabase, PaperViewport, PlotRotation};
use cad_history::{drawing_patch, layout_patch, DrawingUndoRecord};

use crate::app_drawing::{layout_row, next_transaction_id};
use crate::app_layout_geometry::{clip_center, grid_viewports, resolve_viewport_scale};
use crate::app_layout_names::{
    find_layout_named, resolve_layout_name, validate_layout_name, LayoutSummary,
};

impl Application {
    /// Dispatch one `CommandPayload::Layout` input.
    pub(crate) fn layout_command(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::Layout(payload) = &command.payload else {
            return Err(CadError::InvalidInput(
                "layout.invalid_payload: the command needs a Layout payload".into(),
            ));
        };
        match payload.as_ref() {
            LayoutPayload::List => {
                let rows = self.layout_rows(command.document)?;
                Ok(CommandOutcome {
                    objects: Vec::new(),
                    changes: None,
                    diagnostics: if rows.is_empty() {
                        vec![Diagnostic {
                            object: None,
                            code: "layout.list.empty".into(),
                            message: "LAYOUT LIST: the drawing has no paper layouts".into(),
                        }]
                    } else {
                        rows.iter()
                            .map(|(id, name, viewports)| Diagnostic {
                                object: None,
                                code: "layout.list".into(),
                                message: format!(
                                    "layout {}: {} ({viewports} viewport(s))",
                                    id.0, name
                                ),
                            })
                            .collect()
                    },
                    measurement: None,
                    point_report: None,
                })
            }
            LayoutPayload::New { name } => {
                let name = validate_layout_name(name)?;
                self.create_layout_with_name(command, &name, "layout-new", "layout.created")
            }
            LayoutPayload::Rename { old, new } => {
                let document_id = command.document;
                let drawing = self.drawing(document_id)?.clone();
                let (id, resolved, _) = resolve_layout_name(&drawing, old)?;
                let new = validate_layout_name(new)?;
                if find_layout_named(&drawing, &new, Some(id)).is_some() {
                    return Err(CadError::InvalidInput(format!(
                        "layout.duplicate_name: a layout named {:?} already exists",
                        new
                    )));
                }
                let before = layout_row(&drawing, id);
                let transaction = next_transaction_id();
                let mut candidate = drawing;
                let change_set = candidate.rename_layout(id, &new, transaction, "rename layout")?;
                let after = layout_row(&candidate, id);
                self.publish_layout_record(
                    document_id,
                    candidate,
                    transaction,
                    "layout-rename",
                    vec![layout_patch(id, before, after)],
                )?;
                Ok(CommandOutcome {
                    objects: Vec::new(),
                    changes: Some(Arc::new(change_set)),
                    diagnostics: vec![Diagnostic {
                        object: None,
                        code: "layout.renamed".into(),
                        message: format!(
                            "LAYOUT RENAME: renamed layout {} from {:?} to {:?}",
                            id.0, resolved, new
                        ),
                    }],
                    measurement: None,
                    point_report: None,
                })
            }
            LayoutPayload::Delete { name } => {
                let document_id = command.document;
                let drawing = self.drawing(document_id)?.clone();
                let rows = crate::render_scene::layout_descriptors(&drawing);
                let (id, resolved, _) = resolve_layout_name(&drawing, name)?;
                if rows.len() <= 1 {
                    return Err(CadError::InvalidInput(format!(
                        "layout.last_layout: cannot delete layout {} ({:?}); it is the only \
                         paper layout",
                        id.0, resolved
                    )));
                }
                let before = layout_row(&drawing, id);
                let transaction = next_transaction_id();
                let mut candidate = drawing;
                let change_set = candidate.delete_layout(id, transaction, "delete layout")?;
                // The session may not stay on a space the delete removed. The
                // first remaining paper layout (id order is tab order) is the
                // fallback; model space is the fallback when none remains.
                let switch_target = if session.active_space == SpaceId::Paper(id) {
                    let remaining = candidate.layouts().map(|layout| layout.id).next();
                    Some(remaining.map(SpaceId::Paper).unwrap_or(SpaceId::Model))
                } else {
                    None
                };
                let cleared_maximized = session
                    .maximized_viewport()
                    .is_some_and(|viewport| viewport.layout == id);
                if cleared_maximized {
                    session.clear_maximized_viewport();
                }
                self.publish_layout_record(
                    document_id,
                    candidate,
                    transaction,
                    "layout-delete",
                    vec![layout_patch(id, before, None)],
                )?;
                let mut diagnostics = vec![Diagnostic {
                    object: None,
                    code: "layout.deleted".into(),
                    message: format!(
                        "LAYOUT DELETE: deleted layout {} ({:?}) and its plot settings",
                        id.0, resolved
                    ),
                }];
                if let Some(target) = switch_target {
                    let previous = session.active_space.clone();
                    if self.switch_space_command(session, command, target).is_err() {
                        // A remaining layout this build cannot draw must not
                        // leave the session on the deleted space: fall back to
                        // model space through the same validated switch path.
                        session.active_space = previous;
                        self.switch_space_command(session, command, SpaceId::Model)?;
                        diagnostics.push(Diagnostic {
                            object: None,
                            code: "layout.deleted.no_drawable_fallback".into(),
                            message: "LAYOUT DELETE: the remaining paper layout is not drawable \
                                      by this build; switched to model space"
                                .into(),
                        });
                    }
                }
                Ok(CommandOutcome {
                    objects: Vec::new(),
                    changes: Some(Arc::new(change_set)),
                    diagnostics,
                    measurement: None,
                    point_report: None,
                })
            }
            LayoutPayload::Set { name } => {
                let id = {
                    let drawing = self.drawing(command.document)?;
                    resolve_layout_name(drawing, name)?.0
                };
                self.switch_space_command(session, command, SpaceId::Paper(id))
            }
            LayoutPayload::Merge { source, target } => {
                let document_id = command.document;
                let drawing = self.drawing(document_id)?.clone();
                let (source_id, source_name, _) = resolve_layout_name(&drawing, source)?;
                let (target_id, target_name, _) = resolve_layout_name(&drawing, target)?;
                if source_id == target_id {
                    return Err(CadError::InvalidInput(format!(
                        "layoutmerge.same_layout: {:?} and {:?} are the same layout {}",
                        source_name, target_name, source_id.0
                    )));
                }
                let moved: Vec<cad_db::DbEntity> = drawing
                    .entities()
                    .filter(|entity| entity.space == SpaceId::Paper(source_id))
                    .cloned()
                    .collect();
                let before_row = layout_row(&drawing, source_id);
                let transaction = next_transaction_id();
                let mut candidate = drawing;
                let entity_changes: Vec<(EntityId, Option<cad_db::DbEntity>)> = moved
                    .iter()
                    .map(|before| {
                        let mut after = before.clone();
                        after.space = SpaceId::Paper(target_id);
                        (before.id, Some(after))
                    })
                    .collect();
                // One combined write: entity spaces move and the source layout
                // row is deleted in the same revision and `ChangeSet`; delete
                // safety counts the post-batch entity view, so moving the last
                // paper entity out and deleting the row is one legal batch.
                let change_set = candidate
                    .apply_drawing_changes_with_blocks_display_layers_and_layouts(
                        "layoutmerge",
                        transaction,
                        &[],
                        entity_changes,
                        &[],
                        &[],
                        &[(source_id, None)],
                    )?;
                let mut patches = Vec::with_capacity(moved.len());
                let mut objects = Vec::with_capacity(moved.len());
                for before in moved {
                    let id = before.id;
                    let after = candidate.entity(id).cloned().ok_or_else(|| {
                        CadError::Invariant("LAYOUTMERGE: moved entity vanished".into())
                    })?;
                    patches.push(drawing_patch(id, Some(before), Some(after)));
                    objects.push(ObjectId(id.0));
                }
                let moved_count = patches.len();
                if session.active_space == SpaceId::Paper(source_id) {
                    session.active_space = SpaceId::Paper(target_id);
                }
                if session
                    .maximized_viewport()
                    .is_some_and(|viewport| viewport.layout == source_id)
                {
                    session.clear_maximized_viewport();
                }
                self.publish_candidate(
                    document_id,
                    candidate,
                    DrawingUndoRecord {
                        transaction,
                        label: "layoutmerge".to_string(),
                        patches,
                        block_patches: Vec::new(),
                        layer_patches: Vec::new(),
                        layout_patches: vec![layout_patch(source_id, before_row, None)],
                        merge_key: None,
                    },
                )?;
                Ok(CommandOutcome {
                    objects,
                    changes: Some(Arc::new(change_set)),
                    diagnostics: vec![Diagnostic {
                        object: None,
                        code: "layoutmerge.applied".into(),
                        message: format!(
                            "LAYOUTMERGE: moved {moved_count} paper entities from layout {} \
                             ({:?}) into layout {} ({:?}) and deleted the source layout; \
                             one transaction / one undo step",
                            source_id.0, source_name, target_id.0, target_name
                        ),
                    }],
                    measurement: None,
                    point_report: None,
                })
            }
            LayoutPayload::Wizard { name, paper } => {
                let name = validate_layout_name(name)?;
                let document_id = command.document;
                let drawing = self.drawing(document_id)?.clone();
                if find_layout_named(&drawing, &name, None).is_some() {
                    return Err(CadError::InvalidInput(format!(
                        "layout.duplicate_name: a layout named {:?} already exists",
                        name
                    )));
                }
                let transaction = next_transaction_id();
                let mut candidate = drawing;
                // Allocate the same monotonic id `create_layout` would, then
                // install the row and its optional authored page setup in a
                // **single** revision through the validated batch entry, so a
                // wizard run with a paper setup is still one undo step.
                let id = LayoutId(candidate.next_layout_id().0.max(1));
                let plot = match &paper {
                    Some(paper) => Some(authored_plot_record(
                        id,
                        paper,
                        None,
                        PlotRotation::None,
                        "layoutwizard",
                    )?),
                    None => None,
                };
                let row = cad_db::LayoutRow {
                    layout: cad_db::Layout {
                        id,
                        name: name.clone(),
                        viewports: Vec::new(),
                    },
                    plot,
                };
                let change_set = candidate.apply_layout_changes(
                    "layoutwizard",
                    transaction,
                    &[(id, Some(row))],
                )?;
                let after = layout_row(&candidate, id);
                self.publish_layout_record(
                    document_id,
                    candidate,
                    transaction,
                    "layoutwizard",
                    vec![layout_patch(id, None, after)],
                )?;
                let mut diagnostics = vec![Diagnostic {
                    object: None,
                    code: "layout.created".into(),
                    message: format!("LAYOUT: created layout {} ({:?})", id.0, name),
                }];
                if let Some(paper) = &paper {
                    diagnostics.push(Diagnostic {
                        object: None,
                        code: "layoutwizard.paper_applied".into(),
                        message: format!(
                            "LAYOUTWIZARD: applied authored page setup {} ({} x {} mm, scale \
                             1:1, rotation 0) to layout {}",
                            paper.name, paper.width, paper.height, id.0
                        ),
                    });
                }
                Ok(CommandOutcome {
                    objects: Vec::new(),
                    changes: Some(Arc::new(change_set)),
                    diagnostics,
                    measurement: None,
                    point_report: None,
                })
            }
            LayoutPayload::Viewports { rows, cols } => {
                if *rows < 1 || *cols < 1 {
                    return Err(CadError::InvalidInput(
                        "vports.invalid_grid: VPORTS needs rows and columns >= 1".into(),
                    ));
                }
                let total = rows.checked_mul(*cols).ok_or_else(|| {
                    CadError::InvalidInput(
                        "vports.invalid_grid: VPORTS rows x columns overflows".into(),
                    )
                })?;
                let layout_id = require_paper_space(session, "vports")?;
                let document_id = command.document;
                let drawing = self.drawing(document_id)?.clone();
                let before = layout_row(&drawing, layout_id);
                let viewports = grid_viewports(&drawing, layout_id, *rows, *cols, total);
                let transaction = next_transaction_id();
                let mut candidate = drawing;
                let change_set =
                    candidate.set_layout_viewports(layout_id, viewports, transaction, "vports")?;
                let after = layout_row(&candidate, layout_id);
                self.publish_layout_record(
                    document_id,
                    candidate,
                    transaction,
                    "vports",
                    vec![layout_patch(layout_id, before, after)],
                )?;
                Ok(CommandOutcome {
                    objects: Vec::new(),
                    changes: Some(Arc::new(change_set)),
                    diagnostics: vec![Diagnostic {
                        object: None,
                        code: "vports.created".into(),
                        message: format!(
                            "VPORTS: created a {rows} x {cols} grid of {total} viewports in \
                             layout {} as a declared simple layout (equal cells, model origin at \
                             each cell centre); this is not an AutoCAD Standard viewport \
                             configuration",
                            layout_id.0
                        ),
                    }],
                    measurement: None,
                    point_report: None,
                })
            }
            LayoutPayload::ClipRect { min, max } => {
                let layout_id = require_paper_space(session, "vpclip")?;
                if !min.x.is_finite()
                    || !min.y.is_finite()
                    || !max.x.is_finite()
                    || !max.y.is_finite()
                    || min.x >= max.x
                    || min.y >= max.y
                {
                    return Err(CadError::InvalidInput(
                        "vpclip.degenerate_rect: VPCLIP RECT needs finite minx < maxx and \
                         miny < maxy"
                            .into(),
                    ));
                }
                let document_id = command.document;
                let drawing = self.drawing(document_id)?.clone();
                let mut viewports = layout_viewports(&drawing, layout_id)?;
                let viewport = viewports.first_mut().ok_or_else(|| {
                    CadError::Unsupported(format!(
                        "vpclip.no_viewport: layout {} has no viewport to clip; this build has \
                         no viewport selection, so VPCLIP uses the first viewport",
                        layout_id.0
                    ))
                })?;
                let clip = vec![
                    Point3 {
                        x: min.x,
                        y: min.y,
                        z: 0.0,
                    },
                    Point3 {
                        x: max.x,
                        y: min.y,
                        z: 0.0,
                    },
                    Point3 {
                        x: max.x,
                        y: max.y,
                        z: 0.0,
                    },
                    Point3 {
                        x: min.x,
                        y: max.y,
                        z: 0.0,
                    },
                ];
                viewport.clip = clip;
                let before = layout_row(&drawing, layout_id);
                let transaction = next_transaction_id();
                let mut candidate = drawing;
                let change_set =
                    candidate.set_layout_viewports(layout_id, viewports, transaction, "vpclip")?;
                let after = layout_row(&candidate, layout_id);
                self.publish_layout_record(
                    document_id,
                    candidate,
                    transaction,
                    "vpclip",
                    vec![layout_patch(layout_id, before, after)],
                )?;
                Ok(CommandOutcome {
                    objects: Vec::new(),
                    changes: Some(Arc::new(change_set)),
                    diagnostics: vec![Diagnostic {
                        object: None,
                        code: "vpclip.applied".into(),
                        message: format!(
                            "VPCLIP RECT: replaced the clip of viewport 0 in layout {} with the \
                             rectangle ({}, {})-({}, {}); this build has no viewport selection, so \
                             the first viewport was clipped (polygonal clips are NOT IMPLEMENTED)",
                            layout_id.0, min.x, min.y, max.x, max.y
                        ),
                    }],
                    measurement: None,
                    point_report: None,
                })
            }
            LayoutPayload::Scale { name } => {
                let layout_id = require_paper_space(session, "vpscale")?;
                let document_id = command.document;
                let drawing = self.drawing(document_id)?.clone();
                let (scale_name, factor) = resolve_viewport_scale(&drawing, name)?;
                if !factor.is_finite() || factor <= 0.0 {
                    return Err(CadError::InvalidInput(format!(
                        "vpscale.invalid_factor: resolved scale {:?} has a non-finite or \
                         non-positive factor {factor}",
                        scale_name
                    )));
                }
                let mut viewports = layout_viewports(&drawing, layout_id)?;
                let viewport = viewports.first_mut().ok_or_else(|| {
                    CadError::Unsupported(format!(
                        "vpscale.no_viewport: layout {} has no viewport to scale; this build has \
                         no viewport selection, so VPSCALE uses the first viewport",
                        layout_id.0
                    ))
                })?;
                // `factor` is paper/drawing, so the stored paper→model diagonal
                // is its reciprocal (model units per paper unit).
                let new_scale = 1.0 / factor;
                let center = clip_center(&viewport.clip).ok_or_else(|| {
                    CadError::InvalidInput(format!(
                        "vpscale.invalid_viewport: the clip of layout {} viewport 0 has no \
                         finite paper rectangle",
                        layout_id.0
                    ))
                })?;
                let old_scale = viewport.model_to_paper.matrix[0][0];
                let old_scale_y = viewport.model_to_paper.matrix[1][1];
                if !old_scale.is_finite()
                    || !old_scale_y.is_finite()
                    || old_scale <= 0.0
                    || old_scale_y <= 0.0
                {
                    return Err(CadError::InvalidInput(format!(
                        "vpscale.invalid_viewport: layout {} viewport 0 has a non-finite or \
                         non-positive model_to_paper scale",
                        layout_id.0
                    )));
                }
                // Preserve the model point shown at the clip centre:
                // anchor = translation + scale * paper_center.
                let old_tx = viewport.model_to_paper.matrix[0][3];
                let old_ty = viewport.model_to_paper.matrix[1][3];
                let anchor_x = old_tx + old_scale * center.x;
                let anchor_y = old_ty + old_scale_y * center.y;
                viewport.model_to_paper.matrix[0][0] = new_scale;
                viewport.model_to_paper.matrix[1][1] = new_scale;
                viewport.model_to_paper.matrix[0][3] = anchor_x - new_scale * center.x;
                viewport.model_to_paper.matrix[1][3] = anchor_y - new_scale * center.y;
                let before = layout_row(&drawing, layout_id);
                let transaction = next_transaction_id();
                let mut candidate = drawing;
                let change_set =
                    candidate.set_layout_viewports(layout_id, viewports, transaction, "vpscale")?;
                let after = layout_row(&candidate, layout_id);
                self.publish_layout_record(
                    document_id,
                    candidate,
                    transaction,
                    "vpscale",
                    vec![layout_patch(layout_id, before, after)],
                )?;
                Ok(CommandOutcome {
                    objects: Vec::new(),
                    changes: Some(Arc::new(change_set)),
                    diagnostics: vec![Diagnostic {
                        object: None,
                        code: "vpscale.applied".into(),
                        message: format!(
                            "VPSCALE: applied scale {:?} (paper/drawing factor {factor}) to \
                             viewport 0 of layout {}; stored model units per paper unit = \
                             {new_scale}",
                            scale_name, layout_id.0
                        ),
                    }],
                    measurement: None,
                    point_report: None,
                })
            }
            LayoutPayload::FreezeViewportLayers { layers } => {
                self.freeze_viewport_layers(session, command, layers)
            }
            // Page setup has its own handler (`page_setup_write`), routed by the
            // extended dispatch; these arms keep the enum match exhaustive
            // without inventing a second behaviour here.
            LayoutPayload::PageSetup { .. } | LayoutPayload::PageSetupIn { .. } => {
                self.page_setup_write(command, payload.as_ref())
            }
        }
    }

    /// Create one layout row and record it as one undo step.
    ///
    /// The `LAYOUT NEW` create path (the wizard uses its own combined
    /// create-plus-page-setup batch so both land in one revision).
    fn create_layout_with_name(
        &mut self,
        command: &Command,
        name: &str,
        label: &str,
        diagnostic_code: &str,
    ) -> CadResult<CommandOutcome> {
        let document_id = command.document;
        let drawing = self.drawing(document_id)?.clone();
        if find_layout_named(&drawing, name, None).is_some() {
            return Err(CadError::InvalidInput(format!(
                "layout.duplicate_name: a layout named {:?} already exists",
                name
            )));
        }
        let transaction = next_transaction_id();
        let mut candidate = drawing;
        let (id, change_set) = candidate.create_layout(name, transaction, label)?;
        let after = layout_row(&candidate, id);
        self.publish_layout_record(
            document_id,
            candidate,
            transaction,
            label,
            vec![layout_patch(id, None, after)],
        )?;
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: Some(Arc::new(change_set)),
            diagnostics: vec![Diagnostic {
                object: None,
                code: diagnostic_code.into(),
                message: format!("LAYOUT: created layout {} ({:?})", id.0, name),
            }],
            measurement: None,
            point_report: None,
        })
    }

    /// Apply one typed `PAGESETUP`/`PSETUPIN` write.
    ///
    /// Both forms store the result through
    /// [`cad_db::DrawingDatabase::set_plot_settings`] (one revision) and record
    /// exactly one [`cad_db::LayoutPatch`], so the previous plot row is restored
    /// exactly by UNDO and re-applied by REDO.
    pub(crate) fn page_setup_write(
        &mut self,
        command: &Command,
        payload: &LayoutPayload,
    ) -> CadResult<CommandOutcome> {
        match payload {
            LayoutPayload::PageSetup {
                layout,
                paper,
                scale,
                rotation,
            } => {
                let document_id = command.document;
                let drawing = self.drawing(document_id)?.clone();
                let (id, resolved, _) = resolve_layout_name(&drawing, layout)?;
                let record = authored_plot_record(id, paper, *scale, *rotation, "pagesetup")?;
                let before = layout_row(&drawing, id);
                let transaction = next_transaction_id();
                let mut candidate = drawing;
                let change_set =
                    candidate.set_plot_settings(id, record.clone(), transaction, "pagesetup")?;
                let after = layout_row(&candidate, id);
                self.publish_layout_record(
                    document_id,
                    candidate,
                    transaction,
                    "pagesetup",
                    vec![layout_patch(id, before, after)],
                )?;
                let (width, height) = record.rotated_size();
                Ok(CommandOutcome {
                    objects: Vec::new(),
                    changes: Some(Arc::new(change_set)),
                    diagnostics: vec![Diagnostic {
                        object: None,
                        code: "pagesetup.applied".into(),
                        message: format!(
                            "PAGESETUP: applied authored page setup '{}' ({} x {} {:?} after \
                             rotation) scale {}:{} rotation {}° to layout {} ({:?})",
                            record.paper_size_name,
                            width,
                            height,
                            record.paper_units,
                            record.scale_numerator,
                            record.scale_denominator,
                            record.rotation.to_degrees(),
                            id.0,
                            resolved
                        ),
                    }],
                    measurement: None,
                    point_report: None,
                })
            }
            LayoutPayload::PageSetupIn { source, target } => {
                let document_id = command.document;
                let drawing = self.drawing(document_id)?.clone();
                let (source_id, source_name, _) = resolve_layout_name(&drawing, source)?;
                let (target_id, target_name, _) = resolve_layout_name(&drawing, target)?;
                // Only a stored record is importable. The A4 fallback is a
                // documented convenience for readers and is never promoted to a
                // real page setup.
                let mut record = drawing.plot_settings(source_id).cloned().ok_or_else(|| {
                    CadError::InvalidInput(format!(
                        "psetupin.no_source: layout {} ({:?}) stores no PLOTSETTINGS record to \
                         import; the default page is not synthesized",
                        source_id.0, source_name
                    ))
                })?;
                record.layout = target_id;
                record.provenance = cad_db::PlotProvenance::Authored;
                let before = layout_row(&drawing, target_id);
                let transaction = next_transaction_id();
                let mut candidate = drawing;
                let change_set =
                    candidate.set_plot_settings(target_id, record, transaction, "psetupin")?;
                let after = layout_row(&candidate, target_id);
                self.publish_layout_record(
                    document_id,
                    candidate,
                    transaction,
                    "psetupin",
                    vec![layout_patch(target_id, before, after)],
                )?;
                Ok(CommandOutcome {
                    objects: Vec::new(),
                    changes: Some(Arc::new(change_set)),
                    diagnostics: vec![Diagnostic {
                        object: None,
                        code: "psetupin.applied".into(),
                        message: format!(
                            "PSETUPIN: copied the stored page setup of layout {} ({:?}) to \
                             layout {} ({:?}) as an authored record",
                            source_id.0, source_name, target_id.0, target_name
                        ),
                    }],
                    measurement: None,
                    point_report: None,
                })
            }
            _ => Err(CadError::InvalidInput(
                "pagesetup.invalid_payload: PAGESETUP/PSETUPIN need a \
                 Layout(PageSetup|PageSetupIn) payload"
                    .into(),
            )),
        }
    }

    /// Create one paper viewport from a typed two-corner rectangle.
    ///
    /// The rectangle is a paper-space axis-aligned `min`/`max` pair (not
    /// canonicalised: the caller must give `minx < maxx` and `miny < maxy`). The
    /// new viewport is appended to the active layout's existing viewports with a
    /// four-corner clip, `Completeness::Complete`, unit scale and the same
    /// `anchor - paper_center` translation convention as `VPORTS`.
    pub(crate) fn mview(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let layout_id = require_paper_space(session, "mview")?;
        let CommandPayload::Points(points) = &command.payload else {
            return Err(CadError::InvalidInput(
                "mview.invalid_rect: MVIEW needs exactly two corner points".into(),
            ));
        };
        let [min, max] = points.as_slice() else {
            return Err(CadError::InvalidInput(
                "mview.invalid_rect: MVIEW needs exactly two corner points".into(),
            ));
        };
        if !min.x.is_finite()
            || !min.y.is_finite()
            || !max.x.is_finite()
            || !max.y.is_finite()
            || min.x >= max.x
            || min.y >= max.y
        {
            return Err(CadError::InvalidInput(
                "mview.invalid_rect: MVIEW needs finite minx < maxx and miny < maxy".into(),
            ));
        }
        let document_id = command.document;
        let drawing = self.drawing(document_id)?.clone();
        let mut viewports = layout_viewports(&drawing, layout_id)?;
        let paper_center = Point3 {
            x: (min.x + max.x) * 0.5,
            y: (min.y + max.y) * 0.5,
            z: 0.0,
        };
        let mut matrix = Transform3::identity().matrix;
        matrix[0][0] = 1.0;
        matrix[1][1] = 1.0;
        matrix[0][3] = -paper_center.x;
        matrix[1][3] = -paper_center.y;
        let clip = vec![
            Point3 {
                x: min.x,
                y: min.y,
                z: 0.0,
            },
            Point3 {
                x: max.x,
                y: min.y,
                z: 0.0,
            },
            Point3 {
                x: max.x,
                y: max.y,
                z: 0.0,
            },
            Point3 {
                x: min.x,
                y: max.y,
                z: 0.0,
            },
        ];
        viewports.push(PaperViewport {
            clip,
            model_to_paper: Transform3 { matrix },
            completeness: Completeness::Complete,
            layer_overrides: std::collections::BTreeMap::new(),
        });
        let index = viewports.len() - 1;
        let before = layout_row(&drawing, layout_id);
        let transaction = next_transaction_id();
        let mut candidate = drawing;
        let change_set =
            candidate.set_layout_viewports(layout_id, viewports, transaction, "mview")?;
        let after = layout_row(&candidate, layout_id);
        self.publish_layout_record(
            document_id,
            candidate,
            transaction,
            "mview",
            vec![layout_patch(layout_id, before, after)],
        )?;
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: Some(Arc::new(change_set)),
            diagnostics: vec![
                Diagnostic {
                    object: None,
                    code: "mview.created".into(),
                    message: format!(
                        "MVIEW: appended viewport {index} to layout {} with the paper rectangle \
                         ({}, {})-({}, {})",
                        layout_id.0, min.x, min.y, max.x, max.y
                    ),
                },
                Diagnostic {
                    object: None,
                    code: "mview.no_holes".into(),
                    message: "MVIEW: the created viewport is one four-corner rectangle with no \
                              holes; polygonal clips and viewport holes are NOT IMPLEMENTED"
                        .into(),
                },
            ],
            measurement: None,
            point_report: None,
        })
    }

    pub(crate) fn drawing(&self, document: DocumentId) -> CadResult<&DrawingDatabase> {
        self.workspace
            .documents
            .get(&document)
            .map(|document| document.drawing.as_ref())
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))
    }

    /// The authoritative paper-layout rows, in `LayoutId` order.
    fn layout_rows(&self, document: DocumentId) -> CadResult<Vec<LayoutSummary>> {
        Ok(
            crate::render_scene::layout_descriptors(self.drawing(document)?)
                .into_iter()
                .map(|descriptor| (descriptor.id, descriptor.name, descriptor.viewport_count))
                .collect(),
        )
    }
}

/// Require an active paper layout for a viewport-scoped command, returning it.
pub(super) fn require_paper_space(session: &SessionState, prefix: &str) -> CadResult<LayoutId> {
    match &session.active_space {
        SpaceId::Paper(layout_id) => Ok(*layout_id),
        _ => Err(CadError::Unsupported(format!(
            "{prefix}.paper_space_required: the command needs an active paper layout"
        ))),
    }
}

/// The viewports of `layout_id`, cloned so a candidate can be edited independently.
pub(super) fn layout_viewports(
    drawing: &DrawingDatabase,
    layout_id: LayoutId,
) -> CadResult<Vec<PaperViewport>> {
    drawing
        .layout(layout_id)
        .map(|layout| layout.viewports.clone())
        .ok_or_else(|| {
            CadError::InvalidInput(format!(
                "layout.unknown_id: this drawing has no layout with id {}",
                layout_id.0
            ))
        })
}

/// Build the authored `PLOTSETTINGS` row the page-setup writes store.
///
/// `prefix` is the stable diagnostic namespace of the caller (`pagesetup` or
/// `layoutwizard`). Every field not carried by the grammar uses the documented
/// authored default: zero margins, `PlotType::Layout`, millimetre paper units and
/// `PlotProvenance::Authored` (never `Imported`/`DefaultPage`).
fn authored_plot_record(
    layout: LayoutId,
    paper: &PaperSetup,
    scale: Option<(f64, f64)>,
    rotation: PlotRotation,
    prefix: &str,
) -> CadResult<cad_db::PlotSettingsRecord> {
    validate_paper_setup(paper, prefix)?;
    let (numerator, denominator) = scale.unwrap_or((1.0, 1.0));
    if !numerator.is_finite() || !denominator.is_finite() || numerator <= 0.0 || denominator <= 0.0
    {
        return Err(CadError::InvalidInput(format!(
            "{prefix}.invalid_scale: the page-setup scale must be finite and > 0"
        )));
    }
    Ok(cad_db::PlotSettingsRecord {
        layout,
        paper_size_name: paper.name.trim().to_string(),
        paper_width: paper.width,
        paper_height: paper.height,
        margins: cad_db::PlotMargins::default(),
        rotation,
        scale_numerator: numerator,
        scale_denominator: denominator,
        plot_type: cad_db::PlotType::Layout,
        paper_units: cad_db::PlotPaperUnits::Millimeters,
        provenance: cad_db::PlotProvenance::Authored,
    })
}

/// Validate page-setup name/numbers for one command namespace.
fn validate_paper_setup(paper: &PaperSetup, prefix: &str) -> CadResult<()> {
    if paper.name.trim().is_empty() {
        return Err(CadError::InvalidInput(format!(
            "{prefix}.invalid_paper: the paper name must not be empty"
        )));
    }
    if !paper.width.is_finite()
        || !paper.height.is_finite()
        || paper.width <= 0.0
        || paper.height <= 0.0
    {
        return Err(CadError::InvalidInput(format!(
            "{prefix}.invalid_paper: paper width and height must be finite and > 0"
        )));
    }
    Ok(())
}
