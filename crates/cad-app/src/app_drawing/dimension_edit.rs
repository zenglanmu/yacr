//! The 2026-11-07 dimension-editing batch: `DIMSTYLE` (`-DIMSTYLE`),
//! `DIMTEDIT`, `DIMEDIT`, `DIMOVERRIDE`, `DIMREGEN`, `DIMDISASSOCIATE`, `QDIM`,
//! `DIMJOGLINE` and `DIMBREAK`.
//!
//! The full contract, including every declared boundary, lives in
//! [`crate::command_id::dimension_edit`]. In short:
//!
//! * `DIMSTYLE LIST`/`CURRENT` are read-only reports of the stored DIMSTYLE
//!   table; `DIMSTYLE SET` is a named refusal because `cad-db` has no
//!   post-import style-table write API.
//! * `DIMTEDIT`/`DIMEDIT`/`DIMOVERRIDE` reuse the existing
//!   [`DimensionGeometry`] entity: they rebuild it through the shared synthesis
//!   from a mutated definition / style snapshot / text override and write every
//!   changed entity in **one transaction / one undo step**.
//! * `QDIM` creates one `DimensionKind::Aligned` dimension per selected straight
//!   `LINE`, also in one transaction / one undo step.
//! * `DIMREGEN`/`DIMDISASSOCIATE` are explicit no-op diagnostics (this build is
//!   non-associative).
//! * `DIMJOGLINE`/`DIMBREAK` are unconditional named refusals: the domain has no
//!   jog field and no dimension-line break record. They never delete or trim the
//!   dimension.

use super::*;
use cad_db::{DbEntity, DrawingDatabase};
use cad_domain::DimensionStyleValues;

/// The minimum extent used by the degeneracy guards (the same order as the
/// other dimension commands).
const MIN_DIMENSION_EDIT_EXTENT: f64 = 1e-9;

/// A named input refusal for this batch.
fn invalid(code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::InvalidInput(format!("{code}: {detail}"))
}

/// A named capability-gap refusal for this batch.
fn unsupported(code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::Unsupported(format!("{code}: {detail}"))
}

/// The document's drawing, with the ordinary stale-document guard.
fn drawing_of(app: &Application, command: &Command) -> CadResult<Arc<DrawingDatabase>> {
    let document = app
        .workspace
        .documents
        .get(&command.document)
        .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
    if document.id != command.document {
        return Err(CadError::StaleResult);
    }
    Ok(document.drawing.clone())
}

/// One selected dimension resolved from the session selection.
struct SelectedDimension {
    entity: DbEntity,
    dimension: DimensionGeometry,
}

/// The definition-plane Z of a dimension (the Z the text must stay on).
fn plane_z(points: &DimensionPoints) -> f64 {
    match points {
        DimensionPoints::Linear { first, .. } => first.z,
        DimensionPoints::Radial { point_on_arc, .. } => point_on_arc.z,
        DimensionPoints::Angular2Ln { arc_point, .. } => arc_point.z,
        DimensionPoints::Angular3Pt { arc_point, .. } => arc_point.z,
        DimensionPoints::Ordinate { feature, .. } => feature.z,
    }
}

/// Resolve one selection ref to a directly addressable dimension entity.
fn resolve_dimension(
    drawing: &DrawingDatabase,
    command: &Command,
    reference: &SelectionRef,
    name: &str,
) -> CadResult<SelectedDimension> {
    if reference.document != command.document {
        return Err(invalid(
            &format!("{name}.not_addressable"),
            "the selected dimension belongs to another document",
        ));
    }
    if !reference.instance.0.is_empty() || reference.sub_element.is_some() {
        return Err(invalid(
            &format!("{name}.not_addressable"),
            "a block-instance member or sub-element cannot be edited; select the dimension \
             entity itself",
        ));
    }
    let entity = drawing.entity(reference.entity).cloned().ok_or_else(|| {
        invalid(
            &format!("{name}.target_missing"),
            format!(
                "the selected entity {} no longer exists",
                reference.entity.0
            ),
        )
    })?;
    match entity.geometry.clone() {
        SemanticGeometry::Dimension(dimension) => Ok(SelectedDimension {
            entity,
            dimension: *dimension,
        }),
        other => Err(invalid(
            &format!("{name}.not_dimension"),
            format!("{} is not a DIMENSION entity", geometry_kind(&other)),
        )),
    }
}

/// Resolve every selection ref, refusing duplicates and non-dimensions before
/// anything is staged.
fn resolve_selected_dimensions(
    drawing: &DrawingDatabase,
    command: &Command,
    references: &[SelectionRef],
    name: &str,
) -> CadResult<Vec<SelectedDimension>> {
    let mut seen = std::collections::BTreeSet::new();
    let mut resolved = Vec::with_capacity(references.len());
    for reference in references {
        if !seen.insert(reference.entity) {
            return Err(invalid(
                &format!("{name}.duplicate"),
                format!("entity {} is selected twice", reference.entity.0),
            ));
        }
        resolved.push(resolve_dimension(drawing, command, reference, name)?);
    }
    Ok(resolved)
}

/// Rebuild one dimension from a mutated definition / style / override through
/// the single shared synthesis (never a second geometry path).
fn rebuild_dimension(
    dimension: &DimensionGeometry,
    definition: DimensionDefinition,
    style: Option<DimensionStyleValues>,
    text_override: Option<String>,
) -> CadResult<DimensionGeometry> {
    DimensionGeometry::synthesized(
        definition,
        dimension.style_name().to_string(),
        style.unwrap_or_else(|| dimension.style().clone()),
        text_override,
    )
}

/// Read a required batch payload.
fn batch_payload<'a>(command: &'a Command, name: &str) -> CadResult<&'a DimensionBatchPayload> {
    match &command.payload {
        CommandPayload::Dimension(payload) => Ok(payload.as_ref()),
        CommandPayload::None => Err(invalid(
            &format!("{name}.needs_option"),
            "the typed form needs an option on the same command line",
        )),
        _ => Err(invalid(
            &format!("{name}.payload"),
            "the typed form takes the documented dimension payload",
        )),
    }
}

impl Application {
    /// `DIMSTYLE` / `-DIMSTYLE`: report the stored DIMSTYLE table or refuse the
    /// style-table write this build does not have.
    pub(crate) fn dimstyle_command(&mut self, command: &Command) -> CadResult<CommandOutcome> {
        let request = match batch_payload(command, "dimstyle")? {
            DimensionBatchPayload::Style(style) => style.clone(),
            _ => {
                return Err(invalid(
                    "dimstyle.payload",
                    "DIMSTYLE takes LIST, CURRENT or SET <name>",
                ))
            }
        };
        let drawing = drawing_of(self, command)?;
        match request {
            DimStyleCommand::List => {
                let mut diagnostics = Vec::new();
                for style in drawing.dim_styles() {
                    diagnostics.push(Diagnostic {
                        object: None,
                        code: "dimstyle.entry".into(),
                        message: format!(
                            "DIMSTYLE: {} (id {}, text height {}, arrow size {}, DIMDLI {})",
                            style.name,
                            style.id.0,
                            style.text_height,
                            style.arrow_size,
                            style.dim_line_increment
                        ),
                    });
                }
                if diagnostics.is_empty() {
                    diagnostics.push(Diagnostic {
                        object: None,
                        code: "dimstyle.empty".into(),
                        message: "DIMSTYLE: the drawing stores no dimension styles".into(),
                    });
                }
                Ok(report_outcome(diagnostics))
            }
            DimStyleCommand::Current => {
                // Creation resolves `Standard` (ADR 0017 B.3); there is no
                // session "current style" variable in this build.
                let diagnostic = match drawing.dim_style_by_name("Standard") {
                    Some(style) => Diagnostic {
                        object: None,
                        code: "dimstyle.current".into(),
                        message: format!(
                            "DIMSTYLE CURRENT: Standard (id {}); dimension creation resolves this \
                             style (ADR 0017 B.3)",
                            style.id.0
                        ),
                    },
                    None => Diagnostic {
                        object: None,
                        code: "dimstyle.current_missing".into(),
                        message:
                            "DIMSTYLE CURRENT: the drawing has no 'Standard' dimension style; \
                                  creation would refuse (nothing was changed)"
                                .into(),
                    },
                };
                Ok(report_outcome(vec![diagnostic]))
            }
            DimStyleCommand::Set(name) => {
                if drawing.dim_style_by_name(&name).is_none() {
                    return Err(invalid(
                        "dimstyle.unknown",
                        format!("the drawing has no dimension style named '{name}'"),
                    ));
                }
                Err(unsupported(
                    "dimstyle.set_unavailable",
                    "this build has no post-import dim-style table write API \
                     (DrawingDatabase exposes read-only dim_styles()/dim_style_by_name(); only \
                     DrawingDatabaseBuilder::insert_dim_style can add a row), so DIMSTYLE SET \
                     changes nothing",
                ))
            }
        }
    }

    /// `DIMTEDIT`: move/rotate/reset the text of exactly one selected dimension.
    pub(crate) fn dim_text_edit(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let request = match batch_payload(command, "dimtedit")? {
            DimensionBatchPayload::TextEdit(request) => *request,
            _ => {
                return Err(invalid(
                    "dimtedit.payload",
                    "DIMTEDIT takes MOVE <x> <y>, ROTATE <degrees> or RESET",
                ))
            }
        };
        let references = session.selection.refs();
        if references.len() != 1 {
            return Err(invalid(
                "dimtedit.selection",
                format!(
                    "DIMTEDIT needs exactly one selected dimension, got {}",
                    references.len()
                ),
            ));
        }
        let drawing = drawing_of(self, command)?;
        let selected = resolve_selected_dimensions(&drawing, command, references, "dimtedit")?;
        let target = &selected[0];
        let mut definition = target.dimension.definition().clone();
        match request {
            DimTextEditCommand::Move { x, y } => {
                if !x.is_finite() || !y.is_finite() {
                    return Err(invalid(
                        "dimtedit.bad_position",
                        "the text position must be finite",
                    ));
                }
                definition.text_middle = Point3 {
                    x,
                    y,
                    z: plane_z(&definition.points),
                };
            }
            DimTextEditCommand::Rotate { degrees } => {
                if !degrees.is_finite() {
                    return Err(invalid(
                        "dimtedit.bad_rotation",
                        "the text rotation must be finite",
                    ));
                }
                definition.text_rotation = degrees.to_radians();
            }
            DimTextEditCommand::Reset => {
                definition.text_middle = Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                };
                definition.text_rotation = 0.0;
            }
        }
        let after = rebuild_dimension(
            &target.dimension,
            definition,
            None,
            target.dimension.text_override().map(str::to_string),
        )
        .map_err(|error| {
            invalid(
                "dimtedit.degenerate",
                format!("the mutated dimension has no synthesizable display: {error}"),
            )
        })?;
        let mut entity = target.entity.clone();
        entity.geometry = SemanticGeometry::Dimension(Box::new(after));
        if entity.geometry == target.entity.geometry {
            return Ok(unchanged_outcome(
                "dimtedit.unchanged",
                "DIMTEDIT: the text is already at the requested placement/rotation \
                 (no transaction)",
            ));
        }
        self.commit_dimension_changes(
            command,
            vec![(target.entity.clone(), entity)],
            "dimtedit",
            "dimtedit.applied",
            "DIMTEDIT: updated the text placement/rotation of one dimension",
        )
    }

    /// `DIMEDIT`: set/reset the text override of every selected dimension.
    pub(crate) fn dim_edit(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let request = match batch_payload(command, "dimedit")? {
            DimensionBatchPayload::Edit(request) => request.clone(),
            _ => {
                return Err(invalid(
                    "dimedit.payload",
                    "DIMEDIT takes TEXT <override> or RESET",
                ))
            }
        };
        let references = session.selection.refs();
        if references.is_empty() {
            return Err(invalid(
                "dimedit.selection",
                "DIMEDIT needs at least one selected dimension",
            ));
        }
        if let DimEditCommand::Text(text) = &request {
            if text.trim().is_empty() {
                return Err(invalid(
                    "dimedit.bad_text",
                    "the dimension text override must be non-empty; use DIMEDIT RESET to drop it",
                ));
            }
            let characters = text.chars().count();
            if characters > MAX_TOOL_TEXT_CHARS {
                return Err(invalid(
                    "dimedit.bad_text",
                    format!(
                        "the text override allows at most {MAX_TOOL_TEXT_CHARS} characters, got \
                         {characters}"
                    ),
                ));
            }
        }
        let drawing = drawing_of(self, command)?;
        let selected = resolve_selected_dimensions(&drawing, command, references, "dimedit")?;
        let override_text = match &request {
            DimEditCommand::Text(text) => Some(text.clone()),
            DimEditCommand::Reset => None,
        };
        let mut changes = Vec::new();
        for target in selected {
            let after = rebuild_dimension(
                &target.dimension,
                target.dimension.definition().clone(),
                None,
                override_text.clone(),
            )
            .map_err(|error| {
                invalid(
                    "dimedit.degenerate",
                    format!("the dimension has no synthesizable display: {error}"),
                )
            })?;
            let mut entity = target.entity.clone();
            entity.geometry = SemanticGeometry::Dimension(Box::new(after));
            if entity.geometry != target.entity.geometry {
                changes.push((target.entity, entity));
            }
        }
        if changes.is_empty() {
            return Ok(unchanged_outcome(
                "dimedit.unchanged",
                "DIMEDIT: every selected dimension already has the requested text override \
                 (no transaction)",
            ));
        }
        let count = changes.len();
        self.commit_dimension_changes(
            command,
            changes,
            "dimedit",
            "dimedit.applied",
            &format!("DIMEDIT: updated the text override of {count} dimension(s)"),
        )
    }

    /// `DIMOVERRIDE`: apply one closed variable to every selected dimension's
    /// style snapshot.
    pub(crate) fn dim_override(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let request = match batch_payload(command, "dimoverride")? {
            DimensionBatchPayload::Override(request) => request.clone(),
            _ => {
                return Err(invalid(
                    "dimoverride.payload",
                    "DIMOVERRIDE takes <variable> <value>",
                ))
            }
        };
        let variable = request.variable.trim().to_ascii_uppercase();
        let references = session.selection.refs();
        if references.is_empty() {
            return Err(invalid(
                "dimoverride.selection",
                "DIMOVERRIDE needs at least one selected dimension",
            ));
        }
        let drawing = drawing_of(self, command)?;
        let selected = resolve_selected_dimensions(&drawing, command, references, "dimoverride")?;
        let mut changes = Vec::new();
        for target in selected {
            let mut style = target.dimension.style().clone();
            apply_dimension_override(&mut style, &variable, &request.value)?;
            let after = rebuild_dimension(
                &target.dimension,
                target.dimension.definition().clone(),
                Some(style),
                target.dimension.text_override().map(str::to_string),
            )
            .map_err(|error| {
                invalid(
                    "dimoverride.degenerate",
                    format!("the dimension has no synthesizable display: {error}"),
                )
            })?;
            let mut entity = target.entity.clone();
            entity.geometry = SemanticGeometry::Dimension(Box::new(after));
            if entity.geometry != target.entity.geometry {
                changes.push((target.entity, entity));
            }
        }
        if changes.is_empty() {
            return Ok(unchanged_outcome(
                "dimoverride.unchanged",
                &format!(
                    "DIMOVERRIDE: every selected dimension already stores {variable} at this value \
                     (no transaction)"
                ),
            ));
        }
        let count = changes.len();
        self.commit_dimension_changes(
            command,
            changes,
            "dimoverride",
            "dimoverride.applied",
            &format!(
                "DIMOVERRIDE: applied {variable} to the entity style snapshot of {count} \
                 dimension(s); the DIMSTYLE table was not changed"
            ),
        )
    }

    /// `DIMREGEN`: explicit non-associative no-op.
    pub(crate) fn dim_regen(&self, command: &Command) -> CadResult<CommandOutcome> {
        if !matches!(command.payload, CommandPayload::None) {
            return Err(invalid(
                "dimregen.payload",
                "DIMREGEN takes no payload; the drawing's dimensions are the whole input",
            ));
        }
        drawing_of(self, command)?;
        Ok(report_outcome(vec![Diagnostic {
            object: None,
            code: "dimregen.non_associative".into(),
            message: "DIMREGEN: this build stores non-associative dimensions; there is no \
                      association to regenerate and nothing changed"
                .into(),
        }]))
    }

    /// `DIMDISASSOCIATE`: explicit already-non-associative no-op.
    pub(crate) fn dim_disassociate(&self, command: &Command) -> CadResult<CommandOutcome> {
        if !matches!(command.payload, CommandPayload::None) {
            return Err(invalid(
                "dimdisassociate.payload",
                "DIMDISASSOCIATE takes no payload; the selection is not its input in this build",
            ));
        }
        drawing_of(self, command)?;
        Ok(report_outcome(vec![Diagnostic {
            object: None,
            code: "dimdisassociate.already_non_associative".into(),
            message: "DIMDISASSOCIATE: every dimension in this build is already \
                      non-associative; nothing changed"
                .into(),
        }]))
    }

    /// `QDIM` v1: one aligned dimension per selected straight LINE.
    pub(crate) fn qdim(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        if !matches!(command.payload, CommandPayload::None) {
            return Err(invalid(
                "qdim.payload",
                "QDIM takes no payload; the current selection is the input",
            ));
        }
        let references = session.selection.refs();
        if references.is_empty() {
            return Err(invalid(
                "qdim.selection",
                "QDIM needs at least one selected straight LINE",
            ));
        }
        let drawing = drawing_of(self, command)?;
        // Every object is resolved and validated before the first line is built.
        let mut seen = std::collections::BTreeSet::new();
        let mut segments = Vec::with_capacity(references.len());
        for reference in references {
            if !seen.insert(reference.entity) {
                return Err(invalid(
                    "qdim.duplicate",
                    format!("entity {} is selected twice", reference.entity.0),
                ));
            }
            if reference.document != command.document {
                return Err(invalid(
                    "qdim.not_addressable",
                    "the selected object belongs to another document",
                ));
            }
            if !reference.instance.0.is_empty() || reference.sub_element.is_some() {
                return Err(invalid(
                    "qdim.not_addressable",
                    "a block-instance member or sub-element cannot be measured; select the \
                     line entity itself",
                ));
            }
            let entity = drawing.entity(reference.entity).cloned().ok_or_else(|| {
                invalid(
                    "qdim.target_missing",
                    format!(
                        "the selected entity {} no longer exists",
                        reference.entity.0
                    ),
                )
            })?;
            if entity.space != SpaceId::Model {
                return Err(invalid(
                    "qdim.space",
                    "QDIM v1 creates dimensions in model space only",
                ));
            }
            match &entity.geometry {
                SemanticGeometry::Line { start, end } => {
                    let length = distance(*start, *end);
                    if !length.is_finite() || length <= MIN_DIMENSION_EDIT_EXTENT {
                        return Err(invalid(
                            "qdim.degenerate",
                            format!(
                                "the selected LINE {} has no measurable length",
                                reference.entity.0
                            ),
                        ));
                    }
                    segments.push((*start, *end, length));
                }
                SemanticGeometry::Circle { .. } => {
                    return Err(unsupported(
                        "qdim.circle_unsupported",
                        "QDIM v1 produces linear/aligned dimensions from straight LINEs only; a \
                         circle would need a radial/diameter QDIM, which is NOT IMPLEMENTED",
                    ))
                }
                SemanticGeometry::Arc { .. } => {
                    return Err(unsupported(
                        "qdim.arc_unsupported",
                        "QDIM v1 produces linear/aligned dimensions from straight LINEs only; an \
                         arc would need a radial QDIM, which is NOT IMPLEMENTED",
                    ))
                }
                SemanticGeometry::Point(_) => {
                    return Err(unsupported(
                        "qdim.point_unsupported",
                        "a lone POINT has no measurable extent and this build has no point \
                         ordering/reference primitive for QDIM",
                    ))
                }
                other => {
                    return Err(unsupported(
                        "qdim.unsupported_kind",
                        format!(
                            "QDIM v1 supports straight LINEs only; {} is not supported",
                            geometry_kind(other)
                        ),
                    ))
                }
            }
        }
        let style = drawing.resolve_dim_style("Standard").ok_or_else(|| {
            invalid(
                "qdim.no_style",
                "the drawing has no 'Standard' dimension style to create with",
            )
        })?;
        let offset = style.dim_line_increment;
        if !offset.is_finite() || offset <= MIN_DIMENSION_EDIT_EXTENT {
            return Err(invalid(
                "qdim.bad_spacing",
                format!("the Standard style's DIMDLI must be finite and positive, got {offset}"),
            ));
        }
        let mut geometries = Vec::with_capacity(segments.len());
        for (start, end, length) in segments {
            let direction = Point3 {
                x: (end.x - start.x) / length,
                y: (end.y - start.y) / length,
                z: 0.0,
            };
            let normal = Point3 {
                x: -direction.y,
                y: direction.x,
                z: 0.0,
            };
            let midpoint = Point3 {
                x: (start.x + end.x) * 0.5,
                y: (start.y + end.y) * 0.5,
                z: (start.z + end.z) * 0.5,
            };
            let location = Point3 {
                x: midpoint.x + normal.x * offset,
                y: midpoint.y + normal.y * offset,
                z: midpoint.z,
            };
            let definition = DimensionDefinition {
                kind: DimensionKind::Aligned,
                points: DimensionPoints::Linear {
                    first: start,
                    second: end,
                    dimension_line: location,
                },
                text_middle: Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                text_rotation: 0.0,
                normal: Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 1.0,
                },
            };
            geometries.push(SemanticGeometry::Dimension(Box::new(
                DimensionGeometry::synthesized(
                    definition,
                    "Standard".to_string(),
                    style.clone(),
                    None,
                )?,
            )));
        }
        self.commit_new_entities(session, command, geometries, "qdim")
    }

    /// `DIMJOGLINE`: unconditional capability refusal.
    pub(crate) fn dim_jogline(&mut self, command: &Command) -> CadResult<CommandOutcome> {
        match batch_payload(command, "dimjogline")? {
            DimensionBatchPayload::JogLine(_) => {}
            _ => {
                return Err(invalid(
                    "dimjogline.payload",
                    "DIMJOGLINE takes ADD <x> <y> or REMOVE",
                ))
            }
        }
        Err(unsupported(
            "dimjogline.unsupported",
            "cad_domain::DimensionDefinition has no jog field and DimensionGeometry cannot store \
             a jog, so DIMJOGLINE changes nothing",
        ))
    }

    /// `DIMBREAK`: unconditional capability refusal; never touches the entity.
    pub(crate) fn dim_break(&mut self, command: &Command) -> CadResult<CommandOutcome> {
        if !matches!(command.payload, CommandPayload::None) {
            return Err(invalid(
                "dimbreak.payload",
                "DIMBREAK takes no payload; the selected dimension and crossing objects are the \
                 input",
            ));
        }
        Err(unsupported(
            "dimbreak.unsupported",
            "cad_domain::DimensionGeometry stores no dimension-line break and this build has no \
             break record, so DIMBREAK changes nothing (the dimension is never deleted or trimmed)",
        ))
    }

    /// Commit a batch of dimension entity replacements in one transaction / one
    /// undo step and read every stored value back for the undo patches.
    fn commit_dimension_changes(
        &mut self,
        command: &Command,
        changes: Vec<(DbEntity, DbEntity)>,
        label: &str,
        code: &str,
        message: &str,
    ) -> CadResult<CommandOutcome> {
        let document_id = command.document;
        let document = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        let mut candidate = document.drawing.as_ref().clone();
        let transaction = next_transaction_id();
        let change_set = {
            let mut staged = candidate.begin_drawing_transaction(label, transaction)?;
            for (_, after) in &changes {
                staged.update_entity(after.clone())?;
            }
            staged.commit()?
        };
        let mut patches = Vec::with_capacity(changes.len());
        let mut objects = Vec::with_capacity(changes.len());
        for (before, _) in &changes {
            let stored = candidate.entity(before.id).cloned().ok_or_else(|| {
                CadError::Invariant(format!(
                    "{label}: an updated dimension vanished after commit"
                ))
            })?;
            patches.push(drawing_patch(before.id, Some(before.clone()), Some(stored)));
            objects.push(ObjectId(before.id.0));
        }
        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: label.to_string(),
                patches,
                block_patches: Vec::new(),
                merge_key: None,
            },
        )?;
        Ok(CommandOutcome {
            objects,
            changes: Some(Arc::new(change_set)),
            diagnostics: vec![Diagnostic {
                object: None,
                code: code.into(),
                message: message.to_string(),
            }],
            measurement: None,
            point_report: None,
        })
    }
}

/// A read-only report outcome (no ChangeSet, no history).
fn report_outcome(diagnostics: Vec<Diagnostic>) -> CommandOutcome {
    CommandOutcome {
        objects: Vec::new(),
        changes: None,
        diagnostics,
        measurement: None,
        point_report: None,
    }
}

/// A no-op outcome with one named diagnostic.
fn unchanged_outcome(code: &str, message: &str) -> CommandOutcome {
    report_outcome(vec![Diagnostic {
        object: None,
        code: code.into(),
        message: message.into(),
    }])
}

/// The closed `DIMOVERRIDE` variable set, applied to the entity style snapshot.
///
/// Every value is validated (never clamped); a variable the snapshot cannot
/// carry is a named capability refusal. See
/// [`crate::command_id::dimension_edit`] for the set and its boundaries.
fn apply_dimension_override(
    style: &mut DimensionStyleValues,
    variable: &str,
    value: &str,
) -> CadResult<()> {
    let value = value.trim();
    match variable {
        "DIMTXT" => style.text_height = positive(variable, value)?,
        "DIMASZ" => style.arrow_size = positive(variable, value)?,
        "DIMEXO" => style.ext_offset = non_negative(variable, value)?,
        "DIMEXE" => style.ext_beyond = non_negative(variable, value)?,
        "DIMGAP" => style.text_gap = non_negative(variable, value)?,
        "DIMCEN" => style.center_mark = finite(variable, value)?,
        "DIMDLI" => style.dim_line_increment = positive(variable, value)?,
        "DIMTAD" => {
            let code = integer(variable, value)?;
            if !(0..=4).contains(&code) {
                return Err(invalid(
                    "dimoverride.bad_value",
                    format!("DIMTAD must be 0..=4, got {code}"),
                ));
            }
            style.text_vertical = code;
        }
        "DIMTVP" => style.text_vertical_position = finite(variable, value)?,
        "DIMLFAC" => style.linear_factor = positive(variable, value)?,
        "DIMDEC" => {
            let decimals = integer(variable, value)?;
            if !(0..=8).contains(&decimals) {
                return Err(invalid(
                    "dimoverride.bad_value",
                    format!("DIMDEC must be 0..=8, got {decimals}"),
                ));
            }
            style.decimals = decimals as usize;
        }
        "DIMRND" => style.rounding = non_negative(variable, value)?,
        "DIMPOST" => {
            if value.is_empty() {
                return Err(invalid(
                    "dimoverride.bad_value",
                    "DIMPOST needs a non-empty template; `<>` renders the bare measurement",
                ));
            }
            style.postfix = value.to_string();
        }
        "DIMZIN" => {
            let bits = integer(variable, value)?;
            // Bits 1/2 (suppress 0 feet / 0 inches) are declared unimplemented
            // by the formatter, so only the implementable subset is admitted.
            if !matches!(bits, 0 | 4 | 8 | 12) {
                return Err(unsupported(
                    "dimoverride.unsupported_value",
                    format!(
                        "DIMZIN {bits} uses feet/inch zero-suppression bits, which this build \
                         does not implement; the admitted set is 0, 4, 8, 12"
                    ),
                ));
            }
            style.zero_suppression = bits;
        }
        "DIMSE1" => style.suppress_ext1 = boolean(variable, value)?,
        "DIMSE2" => style.suppress_ext2 = boolean(variable, value)?,
        "DIMSD1" => style.suppress_dim1 = boolean(variable, value)?,
        "DIMSD2" => style.suppress_dim2 = boolean(variable, value)?,
        "DIMDSEP" => {
            let code = integer(variable, value)?;
            let admitted = code == 0 || code == 46 || (32..=126).contains(&code);
            if !admitted {
                return Err(invalid(
                    "dimoverride.bad_value",
                    format!(
                        "DIMDSEP must be 0, 46 or a printable ASCII code (32..=126), got {code}"
                    ),
                ));
            }
            style.decimal_separator = code;
        }
        "DIMLUNIT" => {
            let format = integer(variable, value)?;
            if format != 2 {
                return Err(unsupported(
                    "dimoverride.unsupported_value",
                    format!(
                        "DIMLUNIT {format} is a non-decimal unit format; this build renders \
                         decimal (DIMLUNIT 2) only"
                    ),
                ));
            }
            style.linear_unit_format = 2;
        }
        // Recognised DIMSTYLE variables with no entity-snapshot slot.
        "DIMSCALE" | "DIMCLRD" | "DIMCLRE" | "DIMCLRT" | "DIMTXSTY" | "DIMJUST" | "DIMATFIT"
        | "DIMTIX" | "DIMTOFL" | "DIMTIH" | "DIMTOH" | "DIMDLE" | "DIMCONTINUEMODE" => {
            return Err(unsupported(
                "dimoverride.unsupported_variable",
                format!(
                    "{variable} is not carried by cad_domain::DimensionStyleValues (the entity \
                     snapshot the synthesis consumes), so it cannot be overridden per dimension"
                ),
            ))
        }
        _ => {
            return Err(invalid(
                "dimoverride.unknown_variable",
                format!(
                    "DIMOVERRIDE does not model '{variable}'; the admitted set is DIMTXT, DIMASZ, \
                     DIMEXO, DIMEXE, DIMGAP, DIMCEN, DIMDLI, DIMTAD, DIMTVP, DIMLFAC, DIMDEC, \
                     DIMRND, DIMPOST, DIMZIN, DIMSE1, DIMSE2, DIMSD1, DIMSD2, DIMDSEP, DIMLUNIT"
                ),
            ))
        }
    }
    Ok(())
}

fn parse_number(variable: &str, value: &str) -> CadResult<f64> {
    value.parse::<f64>().map_err(|_| {
        invalid(
            "dimoverride.bad_value",
            format!("{variable} needs a finite number, got '{value}'"),
        )
    })
}

fn finite(variable: &str, value: &str) -> CadResult<f64> {
    let number = parse_number(variable, value)?;
    if !number.is_finite() {
        return Err(invalid(
            "dimoverride.bad_value",
            format!("{variable} needs a finite number, got '{value}'"),
        ));
    }
    Ok(number)
}

fn positive(variable: &str, value: &str) -> CadResult<f64> {
    let number = finite(variable, value)?;
    if number <= 0.0 {
        return Err(invalid(
            "dimoverride.bad_value",
            format!("{variable} must be strictly positive, got {number}"),
        ));
    }
    Ok(number)
}

fn non_negative(variable: &str, value: &str) -> CadResult<f64> {
    let number = finite(variable, value)?;
    if number < 0.0 {
        return Err(invalid(
            "dimoverride.bad_value",
            format!("{variable} must be non-negative, got {number}"),
        ));
    }
    Ok(number)
}

fn integer(variable: &str, value: &str) -> CadResult<i16> {
    value.parse::<i16>().map_err(|_| {
        invalid(
            "dimoverride.bad_value",
            format!("{variable} needs a whole number in i16, got '{value}'"),
        )
    })
}

fn boolean(variable: &str, value: &str) -> CadResult<bool> {
    match value.to_ascii_uppercase().as_str() {
        "ON" | "1" | "TRUE" => Ok(true),
        "OFF" | "0" | "FALSE" => Ok(false),
        _ => Err(invalid(
            "dimoverride.bad_value",
            format!("{variable} needs ON/OFF (or 1/0), got '{value}'"),
        )),
    }
}
