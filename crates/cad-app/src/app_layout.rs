//! The 2026-11-07 layout / viewport command implementations.
//!
//! The **read** commands (`LAYOUT LIST`, `LAYOUT SET`, `VPMAX`, `VPMIN`) are
//! real, and so are the table/viewport **writes** promoted now that `cad-db`
//! owns the sanctioned post-import layout write paths: `LAYOUT NEW|RENAME|DELETE`,
//! `LAYOUTMERGE`, `LAYOUTWIZARD`, `VPORTS`/`-VPORTS`, `VPCLIP RECT` and
//! `VPSCALE` each publish exactly one revision and one undo step. `LAYVPI` stays
//! a named refusal because this build still has no per-viewport layer state; the
//! full boundary is in [`crate::command_id::layout`].

use super::*;

use cad_db::{DrawingDatabase, PaperViewport};
use cad_history::{drawing_patch, layout_patch, DrawingUndoRecord};

use crate::app_drawing::{layout_row, next_transaction_id};

/// One paper layout row the commands resolve against.
type LayoutSummary = (LayoutId, String, usize);

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
                if let Some(paper) = paper {
                    validate_paper_setup(paper)?;
                }
                let outcome =
                    self.create_layout_with_name(command, &name, "layoutwizard", "layout.created")?;
                let Some(paper) = paper else {
                    return Ok(outcome);
                };
                let mut outcome = outcome;
                outcome.diagnostics.push(Diagnostic {
                    object: None,
                    code: "layoutwizard.paper_not_applied".into(),
                    message: format!(
                        "LAYOUTWIZARD: created layout {:?} but did not apply page setup {} \
                         ({} x {}); page setup is still not modeled, so no plot settings were \
                         written",
                        name, paper.name, paper.width, paper.height
                    ),
                });
                Ok(outcome)
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
                require_paper_space(session, "layvpi")?;
                if layers.is_empty() {
                    return Err(CadError::InvalidInput(
                        "layvpi.invalid_argument: LAYVPI needs at least one layer name".into(),
                    ));
                }
                let drawing = self.drawing(command.document)?;
                for name in layers {
                    resolve_layer_name(drawing, name)?;
                }
                Err(CadError::Unsupported(
                    "layvpi.viewport_layer_state_unsupported: this build has no per-viewport \
                     layer state, so LAYVPI cannot freeze layers in other viewports; it never \
                     freezes them globally"
                        .into(),
                ))
            }
        }
    }

    /// Create one layout row and record it as one undo step.
    ///
    /// Shared by `LAYOUT NEW` and `LAYOUTWIZARD` so both name grammar checks,
    /// the duplicate check, the revision and the recorded patch shape exist once.
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
fn require_paper_space(session: &SessionState, prefix: &str) -> CadResult<LayoutId> {
    match &session.active_space {
        SpaceId::Paper(layout_id) => Ok(*layout_id),
        _ => Err(CadError::Unsupported(format!(
            "{prefix}.paper_space_required: the command needs an active paper layout"
        ))),
    }
}

/// The viewports of `layout_id`, cloned so a candidate can be edited independently.
fn layout_viewports(
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

/// The centre of a viewport clip's axis-aligned paper rectangle.
fn clip_center(clip: &[Point3]) -> Option<Point3> {
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for point in clip {
        if !point.x.is_finite() || !point.y.is_finite() {
            return None;
        }
        min_x = min_x.min(point.x);
        min_y = min_y.min(point.y);
        max_x = max_x.max(point.x);
        max_y = max_y.max(point.y);
    }
    if !min_x.is_finite() || !min_y.is_finite() || !max_x.is_finite() || !max_y.is_finite() {
        return None;
    }
    Some(Point3 {
        x: (min_x + max_x) * 0.5,
        y: (min_y + max_y) * 0.5,
        z: 0.0,
    })
}

/// Build a `rows x cols` grid of axis-aligned viewports covering the sheet.
///
/// The cells are equal, cover the sheet from its own origin, and each clip is the
/// four paper corners `(minx,miny)`, `(maxx,miny)`, `(maxx,maxy)`, `(minx,maxy)`.
/// The stored transform follows the importer's paper→model convention with
/// `model_per_paper = 1.0` and translation `anchor - paper_center`: the model
/// **origin** is the anchor, so `model_to_paper` maps model origin to each cell
/// centre. This is a declared simple layout, not an AutoCAD Standard
/// configuration.
fn grid_viewports(
    drawing: &DrawingDatabase,
    layout_id: LayoutId,
    rows: u32,
    cols: u32,
    total: u32,
) -> Vec<PaperViewport> {
    let (width, height) = {
        let (width, height) = drawing.plot_settings_for(layout_id).rotated_size();
        if width.is_finite() && height.is_finite() && width > 0.0 && height > 0.0 {
            (width, height)
        } else {
            (210.0, 297.0)
        }
    };
    let mut viewports = Vec::with_capacity(total as usize);
    for row in 0..rows {
        for col in 0..cols {
            let min_x = width * f64::from(col) / f64::from(cols);
            let max_x = width * f64::from(col + 1) / f64::from(cols);
            let min_y = height * f64::from(row) / f64::from(rows);
            let max_y = height * f64::from(row + 1) / f64::from(rows);
            let paper_center = Point3 {
                x: (min_x + max_x) * 0.5,
                y: (min_y + max_y) * 0.5,
                z: 0.0,
            };
            // The view centre is the model origin; the stored paper→model
            // translation is `anchor - paper_center` with a unit scale.
            let mut matrix = Transform3::identity().matrix;
            matrix[0][0] = 1.0;
            matrix[1][1] = 1.0;
            matrix[0][3] = -paper_center.x;
            matrix[1][3] = -paper_center.y;
            viewports.push(PaperViewport {
                clip: vec![
                    Point3 {
                        x: min_x,
                        y: min_y,
                        z: 0.0,
                    },
                    Point3 {
                        x: max_x,
                        y: min_y,
                        z: 0.0,
                    },
                    Point3 {
                        x: max_x,
                        y: max_y,
                        z: 0.0,
                    },
                    Point3 {
                        x: min_x,
                        y: max_y,
                        z: 0.0,
                    },
                ],
                model_to_paper: Transform3 { matrix },
                completeness: Completeness::Complete,
            });
        }
    }
    viewports
}

/// Validate a layout name and return the trimmed form.
fn validate_layout_name(name: &str) -> CadResult<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(CadError::InvalidInput(
            "layout.invalid_name: a layout name must not be empty".into(),
        ));
    }
    if name.contains('\r') || name.contains('\n') {
        return Err(CadError::InvalidInput(
            "layout.invalid_name: a layout name must be a single line".into(),
        ));
    }
    if name.chars().count() > 255 {
        return Err(CadError::InvalidInput(
            "layout.invalid_name: a layout name is limited to 255 characters".into(),
        ));
    }
    if name.chars().any(|ch| "<>/\\\":;?*|=`".contains(ch)) {
        return Err(CadError::InvalidInput(format!(
            "layout.invalid_name: layout name {:?} contains an AutoCAD-illegal character",
            name
        )));
    }
    Ok(name.to_string())
}

/// Find a layout by name (case-insensitive), excluding `exclude` when renaming.
fn find_layout_named(
    drawing: &DrawingDatabase,
    name: &str,
    exclude: Option<LayoutId>,
) -> Option<LayoutId> {
    drawing
        .layouts()
        .filter(|layout| Some(layout.id) != exclude)
        .find(|layout| layout.name.eq_ignore_ascii_case(name))
        .map(|layout| layout.id)
}

/// Resolve a layout name: exact first, then a unique case-insensitive match.
fn resolve_layout_name(drawing: &DrawingDatabase, name: &str) -> CadResult<LayoutSummary> {
    let rows: Vec<LayoutSummary> = drawing
        .layouts()
        .map(|layout| (layout.id, layout.name.clone(), layout.viewports.len()))
        .collect();
    let wanted = name.trim();
    if let Some(row) = rows
        .iter()
        .find(|(_, layout_name, _)| layout_name == wanted)
    {
        return Ok((*row).clone());
    }
    let matching: Vec<&LayoutSummary> = rows
        .iter()
        .filter(|(_, layout_name, _)| layout_name.eq_ignore_ascii_case(wanted))
        .collect();
    match matching.as_slice() {
        [row] => Ok((**row).clone()),
        [] => Err(CadError::InvalidInput(format!(
            "layout.unknown: no paper layout named {:?}",
            wanted
        ))),
        _ => Err(CadError::InvalidInput(format!(
            "layout.ambiguous: more than one layout matches {:?} case-insensitively",
            wanted
        ))),
    }
}

/// Validate optional `LAYOUTWIZARD` page-setup numbers.
fn validate_paper_setup(paper: &PaperSetup) -> CadResult<()> {
    if paper.name.trim().is_empty() {
        return Err(CadError::InvalidInput(
            "layoutwizard.invalid_paper: the paper name must not be empty".into(),
        ));
    }
    if !paper.width.is_finite()
        || !paper.height.is_finite()
        || paper.width <= 0.0
        || paper.height <= 0.0
    {
        return Err(CadError::InvalidInput(
            "layoutwizard.invalid_paper: paper width and height must be finite and > 0".into(),
        ));
    }
    Ok(())
}

/// Resolve a `VPSCALE` argument against the drawing's scale table.
///
/// A ratio (`a:b`, both finite, non-zero) is accepted even when the drawing does
/// not list it; a bare word must resolve through the scale table. Zero and
/// non-finite values are refused before the table is consulted.
fn resolve_viewport_scale(drawing: &DrawingDatabase, name: &str) -> CadResult<(String, f64)> {
    let name = name.trim();
    if name.is_empty() {
        return Err(CadError::InvalidInput(
            "vpscale.invalid_scale: VPSCALE needs a scale name or ratio".into(),
        ));
    }
    if let Some((paper, drawing_units)) = name.split_once(':') {
        let paper = paper.trim().parse::<f64>().map_err(|_| {
            CadError::InvalidInput(format!("vpscale.invalid_scale: {:?} is not a ratio", name))
        })?;
        let drawing_units = drawing_units.trim().parse::<f64>().map_err(|_| {
            CadError::InvalidInput(format!("vpscale.invalid_scale: {:?} is not a ratio", name))
        })?;
        if !paper.is_finite() || !drawing_units.is_finite() {
            return Err(CadError::InvalidInput(
                "vpscale.non_finite_scale: VPSCALE needs finite ratio components".into(),
            ));
        }
        if paper == 0.0 || drawing_units == 0.0 {
            return Err(CadError::InvalidInput(
                "vpscale.zero_scale: VPSCALE ratio components must be non-zero".into(),
            ));
        }
        if paper < 0.0 || drawing_units < 0.0 {
            return Err(CadError::InvalidInput(
                "vpscale.invalid_scale: VPSCALE ratio components must be positive".into(),
            ));
        }
        // Prefer the table's named factor when the drawing lists the same scale;
        // otherwise the parsed ratio is the standard scale the caller asked for.
        if let Some(scale) = drawing.scale_by_name(name) {
            return Ok((scale.name.clone(), scale.factor()));
        }
        return Ok((name.to_string(), paper / drawing_units));
    }
    if let Ok(value) = name.parse::<f64>() {
        if !value.is_finite() {
            return Err(CadError::InvalidInput(
                "vpscale.non_finite_scale: VPSCALE needs a finite scale".into(),
            ));
        }
        if value == 0.0 {
            return Err(CadError::InvalidInput(
                "vpscale.zero_scale: VPSCALE needs a non-zero scale".into(),
            ));
        }
        if value < 0.0 {
            return Err(CadError::InvalidInput(
                "vpscale.invalid_scale: VPSCALE needs a positive scale".into(),
            ));
        }
    }
    drawing
        .scale_by_name(name)
        .map(|scale| (scale.name.clone(), scale.factor()))
        .ok_or_else(|| {
            CadError::InvalidInput(format!(
                "vpscale.unknown_scale: {:?} is not in the drawing's scale table and is not a \
                 finite ratio",
                name
            ))
        })
}

/// Resolve a layer name exactly, then by a unique case-insensitive match.
fn resolve_layer_name(drawing: &DrawingDatabase, name: &str) -> CadResult<LayerId> {
    if let Some(id) = drawing.layer_id_by_name(name) {
        return Ok(id);
    }
    let matches: Vec<_> = drawing
        .layers()
        .filter(|layer| layer.name.eq_ignore_ascii_case(name))
        .map(|layer| layer.id)
        .collect();
    match matches.as_slice() {
        [id] => Ok(*id),
        [] => Err(CadError::InvalidInput(format!(
            "layvpi.unknown_layer: no layer named {:?}",
            name
        ))),
        _ => Err(CadError::InvalidInput(format!(
            "layvpi.ambiguous_layer: more than one layer matches {:?} case-insensitively",
            name
        ))),
    }
}
