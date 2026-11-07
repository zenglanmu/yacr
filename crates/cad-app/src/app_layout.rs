//! The 2026-11-07 layout / viewport command implementations.
//!
//! The **read** commands (`LAYOUT LIST`, `LAYOUT SET`, `VPMAX`, `VPMIN`) are
//! real. Every layout/viewport **write** is a named `Unsupported` refusal:
//! `cad-db` has no post-import layout or `PaperViewport` write API, so mutating
//! a clone would publish nothing. The full boundary is in
//! [`crate::command_id::layout`].

use super::*;

use cad_db::DrawingDatabase;

/// One paper layout row the commands resolve against.
type LayoutRow = (LayoutId, String, usize);

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
                let drawing = self.drawing(command.document)?;
                if find_layout_named(drawing, &name, None).is_some() {
                    return Err(CadError::InvalidInput(format!(
                        "layout.duplicate_name: a layout named {:?} already exists",
                        name
                    )));
                }
                Err(CadError::Unsupported(format!(
                    "layout.write_unsupported: cannot create layout {:?}; cad-db has no \
                     post-import layout write API",
                    name
                )))
            }
            LayoutPayload::Rename { old, new } => {
                let drawing = self.drawing(command.document)?;
                let (id, resolved, _) = resolve_layout_name(drawing, old)?;
                let new = validate_layout_name(new)?;
                if find_layout_named(drawing, &new, Some(id)).is_some() {
                    return Err(CadError::InvalidInput(format!(
                        "layout.duplicate_name: a layout named {:?} already exists",
                        new
                    )));
                }
                Err(CadError::Unsupported(format!(
                    "layout.write_unsupported: cannot rename layout {} ({:?}) to {:?}; cad-db \
                     has no post-import layout write API",
                    id.0, resolved, new
                )))
            }
            LayoutPayload::Delete { name } => {
                let drawing = self.drawing(command.document)?;
                let rows = self.layout_rows(command.document)?;
                let (id, resolved, _) = resolve_layout_name(drawing, name)?;
                if rows.len() <= 1 {
                    return Err(CadError::InvalidInput(format!(
                        "layout.last_layout: cannot delete layout {} ({:?}); it is the only \
                         paper layout",
                        id.0, resolved
                    )));
                }
                Err(CadError::Unsupported(format!(
                    "layout.write_unsupported: cannot delete layout {} ({:?}); cad-db has no \
                     post-import layout write API",
                    id.0, resolved
                )))
            }
            LayoutPayload::Set { name } => {
                let id = {
                    let drawing = self.drawing(command.document)?;
                    resolve_layout_name(drawing, name)?.0
                };
                self.switch_space_command(session, command, SpaceId::Paper(id))
            }
            LayoutPayload::Merge { source, target } => {
                let drawing = self.drawing(command.document)?;
                let (source_id, source_name, _) = resolve_layout_name(drawing, source)?;
                let (target_id, target_name, _) = resolve_layout_name(drawing, target)?;
                if source_id == target_id {
                    return Err(CadError::InvalidInput(format!(
                        "layoutmerge.same_layout: {:?} and {:?} are the same layout {}",
                        source_name, target_name, source_id.0
                    )));
                }
                Err(CadError::Unsupported(format!(
                    "layoutmerge.write_unsupported: cannot move entities from layout {} ({:?}) \
                     into layout {} ({:?}); cad-db has no entity-space move API and no layout \
                     delete API",
                    source_id.0, source_name, target_id.0, target_name
                )))
            }
            LayoutPayload::Wizard { name, paper } => {
                let name = validate_layout_name(name)?;
                let drawing = self.drawing(command.document)?;
                if find_layout_named(drawing, &name, None).is_some() {
                    return Err(CadError::InvalidInput(format!(
                        "layout.duplicate_name: a layout named {:?} already exists",
                        name
                    )));
                }
                if let Some(paper) = paper {
                    validate_paper_setup(paper)?;
                    return Err(CadError::Unsupported(format!(
                        "layoutwizard.write_unsupported: cannot create layout {:?} with page \
                         setup {} ({} x {}); cad-db has no layout creation or page-setup write \
                         API, so the paper size is not applied",
                        name, paper.name, paper.width, paper.height
                    )));
                }
                Err(CadError::Unsupported(format!(
                    "layoutwizard.write_unsupported: cannot create layout {:?}; cad-db has no \
                     post-import layout write API and no page-setup write API",
                    name
                )))
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
                require_paper_space(session, "vports")?;
                Err(CadError::Unsupported(format!(
                    "vports.write_unsupported: cannot create a {rows} x {cols} grid of {total} \
                     PaperViewport(s); cad-db has no post-import viewport write API"
                )))
            }
            LayoutPayload::ClipRect { min, max } => {
                require_paper_space(session, "vpclip")?;
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
                Err(CadError::Unsupported(
                    "vpclip.viewport_selection_unsupported: this build has no selected-viewport \
                     state and cad-db has no viewport clip write API; polygonal clips are NOT \
                     IMPLEMENTED"
                        .into(),
                ))
            }
            LayoutPayload::Scale { name } => {
                require_paper_space(session, "vpscale")?;
                let drawing = self.drawing(command.document)?;
                let (scale_name, factor) = resolve_viewport_scale(drawing, name)?;
                Err(CadError::Unsupported(format!(
                    "vpscale.write_unsupported: resolved scale {:?} (factor {factor}); cad-db \
                     has no viewport scale write API, so no viewport was changed",
                    scale_name
                )))
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

    /// `VPMAX`: set the session-only maximized-viewport marker.
    pub(crate) fn vp_max(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        if !matches!(command.payload, CommandPayload::None) {
            return Err(CadError::InvalidInput(
                "vpmax.invalid_payload: VPMAX takes no payload".into(),
            ));
        }
        let SpaceId::Paper(layout_id) = &session.active_space else {
            return Err(CadError::Unsupported(
                "vpmax.paper_space_required: VPMAX needs an active paper layout".into(),
            ));
        };
        let layout_id = *layout_id;
        let viewports = self
            .drawing(command.document)?
            .layout(layout_id)
            .ok_or_else(|| {
                CadError::InvalidInput(format!("vpmax.unknown_layout: layout {}", layout_id.0))
            })?
            .viewports
            .len();
        if viewports == 0 {
            return Err(CadError::Unsupported(format!(
                "vpmax.no_viewports: layout {} has no viewports to maximize",
                layout_id.0
            )));
        }
        let target = LayoutViewportRef {
            layout: layout_id,
            index: 0,
        };
        if session.maximized_viewport() == Some(target) {
            return Ok(CommandOutcome {
                objects: Vec::new(),
                changes: None,
                diagnostics: vec![Diagnostic {
                    object: None,
                    code: "vpmax.already_maximized".into(),
                    message: format!(
                        "VPMAX: layout {} viewport {} is already maximized",
                        layout_id.0, target.index
                    ),
                }],
                measurement: None,
                point_report: None,
            });
        }
        session.set_maximized_viewport(target);
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: None,
            diagnostics: vec![Diagnostic {
                object: None,
                code: "vpmax.maximized".into(),
                message: format!(
                    "VPMAX: maximized layout {} viewport {} (session display state only; the \
                     first viewport is used because this build has no viewport selection)",
                    layout_id.0, target.index
                ),
            }],
            measurement: None,
            point_report: None,
        })
    }

    /// `VPMIN`: clear the session-only maximized-viewport marker.
    pub(crate) fn vp_min(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        if !matches!(command.payload, CommandPayload::None) {
            return Err(CadError::InvalidInput(
                "vpmin.invalid_payload: VPMIN takes no payload".into(),
            ));
        }
        match session.maximized_viewport() {
            Some(viewport) => {
                session.clear_maximized_viewport();
                Ok(CommandOutcome {
                    objects: Vec::new(),
                    changes: None,
                    diagnostics: vec![Diagnostic {
                        object: None,
                        code: "vpmin.restored".into(),
                        message: format!(
                            "VPMIN: restored the layout viewport (layout {} viewport {})",
                            viewport.layout.0, viewport.index
                        ),
                    }],
                    measurement: None,
                    point_report: None,
                })
            }
            None => Ok(CommandOutcome {
                objects: Vec::new(),
                changes: None,
                diagnostics: vec![Diagnostic {
                    object: None,
                    code: "vpmin.nothing_to_restore".into(),
                    message: "VPMIN: no viewport is maximized; nothing changed".into(),
                }],
                measurement: None,
                point_report: None,
            }),
        }
    }

    fn drawing(&self, document: DocumentId) -> CadResult<&DrawingDatabase> {
        self.workspace
            .documents
            .get(&document)
            .map(|document| document.drawing.as_ref())
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))
    }

    /// The authoritative paper-layout rows, in `LayoutId` order.
    fn layout_rows(&self, document: DocumentId) -> CadResult<Vec<LayoutRow>> {
        Ok(
            crate::render_scene::layout_descriptors(self.drawing(document)?)
                .into_iter()
                .map(|descriptor| (descriptor.id, descriptor.name, descriptor.viewport_count))
                .collect(),
        )
    }
}

/// Require an active paper layout for a viewport-scoped command.
fn require_paper_space(session: &SessionState, prefix: &str) -> CadResult<()> {
    if matches!(&session.active_space, SpaceId::Paper(_)) {
        Ok(())
    } else {
        Err(CadError::Unsupported(format!(
            "{prefix}.paper_space_required: the command needs an active paper layout"
        )))
    }
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
fn resolve_layout_name(drawing: &DrawingDatabase, name: &str) -> CadResult<LayoutRow> {
    let rows: Vec<LayoutRow> = drawing
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
    let matching: Vec<&LayoutRow> = rows
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
