//! The 2026-11-07 dimension-editing batch: `DIMSTYLE` (`-DIMSTYLE`),
//! `DIMTEDIT`, `DIMEDIT`, `DIMOVERRIDE`, `DIMREGEN`, `DIMDISASSOCIATE`, `QDIM`,
//! `DIMJOGLINE` and `DIMBREAK`.
//!
//! The full contract, including every declared boundary, lives in
//! [`crate::command_id::dimension_edit`]. In short:
//!
//! * `DIMSTYLE LIST`/`CURRENT` are read-only reports of the stored DIMSTYLE
//!   table; `DIMSTYLE SET` writes the session's current style and the write forms
//!   are post-import table writes (one transaction / one undo step);
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
use cad_domain::{DimensionStyleValues, TextAlignH};

/// The minimum extent used by the degeneracy guards (the same order as the
/// other dimension commands).
const MIN_DIMENSION_EDIT_EXTENT: f64 = 1e-9;

/// A named input refusal for this batch.
pub(super) fn invalid(code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::InvalidInput(format!("{code}: {detail}"))
}

/// A named capability-gap refusal for this batch.
pub(super) fn unsupported(code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::Unsupported(format!("{code}: {detail}"))
}

/// The document's drawing, with the ordinary stale-document guard.
pub(super) fn drawing_of(app: &Application, command: &Command) -> CadResult<Arc<DrawingDatabase>> {
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
pub(super) struct SelectedDimension {
    pub(super) entity: DbEntity,
    pub(super) dimension: DimensionGeometry,
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

/// The text anchor for `DIMTEDIT LEFT|RIGHT|CENTER`.
///
/// The domain stores an **anchor** (group 11) and, separately, the glyph-level
/// horizontal alignment ([`DimensionGeometry::text_h_align`]). Justification
/// moves this anchor to the start, end or midpoint of the dimension's own
/// "line", and the caller sets the matching glyph alignment:
///
/// * linear/aligned — the two extension-line feet on the dimension line;
/// * radius/diameter — the centre-side and arc-side ends of the radial leader;
/// * angular — the start/end/mid angle of the dimension arc (the arc is the
///   angular subtype's dimension line), read from the stored display so the
///   anchor is exactly on the drawn arc;
/// * ordinate — the leader, i.e. the feature end and the leader end, with the
///   midpoint between them.
///
/// The only refused case is a dimension whose stored display carries no arc for
/// an angular kind (an imported picture-branch dimension): there is nothing to
/// justify along, so it is a named refusal rather than a guessed anchor.
fn justify_text_anchor(
    dimension: &DimensionGeometry,
    request: DimTextEditCommand,
) -> CadResult<Point3> {
    let select = |start: Point3, end: Point3| match request {
        DimTextEditCommand::Left => start,
        DimTextEditCommand::Right => end,
        _ => Point3 {
            x: (start.x + end.x) * 0.5,
            y: (start.y + end.y) * 0.5,
            z: (start.z + end.z) * 0.5,
        },
    };
    match (dimension.kind(), &dimension.definition().points) {
        (
            DimensionKind::Linear { rotation },
            DimensionPoints::Linear {
                first,
                second,
                dimension_line,
            },
        ) => {
            let direction = direction_of(rotation);
            let t1 = dot(sub(*first, *dimension_line), direction);
            let t2 = dot(sub(*second, *dimension_line), direction);
            let (low, high) = if t1 <= t2 { (t1, t2) } else { (t2, t1) };
            let t = match request {
                DimTextEditCommand::Left => low,
                DimTextEditCommand::Right => high,
                _ => (low + high) * 0.5,
            };
            Ok(add(*dimension_line, scale_point(direction, t)))
        }
        (
            DimensionKind::Aligned,
            DimensionPoints::Linear {
                first,
                second,
                dimension_line,
            },
        ) => {
            let raw = sub(*second, *first);
            let length = dot(raw, raw).sqrt();
            if !length.is_finite() || length <= MIN_DIMENSION_EDIT_EXTENT {
                return Err(invalid(
                    "dimtedit.degenerate",
                    "the aligned dimension has no measurable direction to justify along",
                ));
            }
            let direction = scale_point(raw, 1.0 / length);
            let t1 = dot(sub(*first, *dimension_line), direction);
            let t2 = dot(sub(*second, *dimension_line), direction);
            let (low, high) = if t1 <= t2 { (t1, t2) } else { (t2, t1) };
            let t = match request {
                DimTextEditCommand::Left => low,
                DimTextEditCommand::Right => high,
                _ => (low + high) * 0.5,
            };
            Ok(add(*dimension_line, scale_point(direction, t)))
        }
        (
            DimensionKind::Radius | DimensionKind::Diameter,
            DimensionPoints::Radial {
                center,
                point_on_arc,
                ..
            },
        ) => Ok(select(*center, *point_on_arc)),
        (DimensionKind::Angular2Ln | DimensionKind::Angular3Pt, _) => {
            angular_justify_anchor(dimension, request)
        }
        (
            DimensionKind::Ordinate { .. },
            DimensionPoints::Ordinate {
                feature,
                leader_end,
                ..
            },
        ) => Ok(select(*feature, *leader_end)),
        (kind, _) => Err(unsupported(
            "dimtedit.justify_unsupported",
            format!(
                "DIMTEDIT LEFT/RIGHT/CENTER has no dimension line to justify along on {}",
                describe_dimension_kind(kind)
            ),
        )),
    }
}

/// The anchor of an angular dimension's justification, taken from the **drawn**
/// arc (the angular subtype's dimension line).
///
/// The arc's `start` angle is the first bounding ray and `start + sweep` the
/// second, so LEFT is the arc's start end, RIGHT its end and CENTER the mid
/// angle; the anchor stays on the arc radius in the dimension plane, exactly
/// where the synthesis draws the arc. An angular dimension whose display has no
/// arc (the imported picture branch) is a named refusal.
fn angular_justify_anchor(
    dimension: &DimensionGeometry,
    request: DimTextEditCommand,
) -> CadResult<Point3> {
    let arc = dimension.display().iter().find_map(|child| match child {
        SemanticGeometry::Arc {
            center,
            radius,
            start,
            sweep,
            ..
        } => Some((*center, *radius, *start, *sweep)),
        _ => None,
    });
    let Some((center, radius, start, sweep)) = arc else {
        return Err(unsupported(
            "dimtedit.justify_unsupported",
            format!(
                "DIMTEDIT LEFT/RIGHT/CENTER has no dimension arc to justify along on {}: its \
                 stored display is an imported picture, not our synthesis",
                describe_dimension_kind(dimension.kind())
            ),
        ));
    };
    let angle = match request {
        DimTextEditCommand::Left => start,
        DimTextEditCommand::Right => start + sweep,
        _ => start + sweep * 0.5,
    };
    Ok(Point3 {
        x: center.x + radius * angle.cos(),
        y: center.y + radius * angle.sin(),
        z: center.z,
    })
}

/// The stable kind name used by the DIMTEDIT justification refusal.
fn describe_dimension_kind(kind: DimensionKind) -> &'static str {
    match kind {
        DimensionKind::Linear { .. } => "a linear dimension",
        DimensionKind::Aligned => "an aligned dimension",
        DimensionKind::Angular2Ln | DimensionKind::Angular3Pt => "an angular dimension",
        DimensionKind::Radius => "a radius dimension",
        DimensionKind::Diameter => "a diameter dimension",
        DimensionKind::Ordinate { .. } => "an ordinate dimension",
    }
}

/// The XY unit direction of a linear dimension's group-50 rotation.
fn direction_of(rotation: f64) -> Point3 {
    Point3 {
        x: rotation.cos(),
        y: rotation.sin(),
        z: 0.0,
    }
}

fn add(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.x + b.x,
        y: a.y + b.y,
        z: a.z + b.z,
    }
}

fn sub(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.x - b.x,
        y: a.y - b.y,
        z: a.z - b.z,
    }
}

fn scale_point(point: Point3, factor: f64) -> Point3 {
    Point3 {
        x: point.x * factor,
        y: point.y * factor,
        z: point.z * factor,
    }
}

fn dot(a: Point3, b: Point3) -> f64 {
    a.x * b.x + a.y * b.y + a.z * b.z
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
pub(super) fn resolve_selected_dimensions(
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
///
/// The synthesis places the text centred, so the caller's glyph alignment is
/// applied afterwards with [`DimensionGeometry::with_text_h_align`]; that only
/// rewrites the existing `Text` child's `h_align` and never rebuilds the display.
pub(super) fn rebuild_dimension(
    dimension: &DimensionGeometry,
    definition: DimensionDefinition,
    style: Option<DimensionStyleValues>,
    text_override: Option<String>,
    h_align: TextAlignH,
) -> CadResult<DimensionGeometry> {
    Ok(DimensionGeometry::synthesized(
        definition,
        dimension.style_name().to_string(),
        style.unwrap_or_else(|| dimension.style().clone()),
        text_override,
    )?
    .with_text_h_align(h_align))
}

/// The glyph alignment a `DIMTEDIT` request selects; everything except
/// LEFT/RIGHT/CENTER preserves the dimension's current alignment.
fn dimtedit_alignment(dimension: &DimensionGeometry, request: DimTextEditCommand) -> TextAlignH {
    match request {
        DimTextEditCommand::Left => TextAlignH::Left,
        DimTextEditCommand::Right => TextAlignH::Right,
        DimTextEditCommand::Center => TextAlignH::Center,
        _ => dimension.text_h_align(),
    }
}

/// Read a required batch payload.
pub(super) fn batch_payload<'a>(
    command: &'a Command,
    name: &str,
) -> CadResult<&'a DimensionBatchPayload> {
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
    /// `DIMTEDIT`: move/justify/rotate/home the text of exactly one selected
    /// dimension.
    ///
    /// The documented set is `MOVE <x> <y>`, `LEFT`, `RIGHT`, `CENTER`, `HOME`
    /// and `ANGLE <degrees>`, plus `OBLIQUE <degrees>` as a named capability
    /// refusal. Justification applies to every dimension kind: linear/aligned
    /// along the dimension line, radius/diameter along the radial leader,
    /// angular along the dimension arc and ordinate along the leader.
    /// `LEFT`/`RIGHT`/`CENTER` move the anchor and set the glyph-level
    /// horizontal alignment; `MOVE`/`HOME`/`ANGLE` preserve the alignment.
    /// `ANGLE` follows AutoCAD's UCS-X-axis convention and keeps the text centre
    /// fixed; `0` restores the derived orientation. `HOME` restores the derived
    /// position only.
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
                    "DIMTEDIT takes MOVE <x> <y>, LEFT, RIGHT, CENTER, HOME or ANGLE <degrees>",
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
            DimTextEditCommand::Left | DimTextEditCommand::Right | DimTextEditCommand::Center => {
                definition.text_middle = justify_text_anchor(&target.dimension, request)?;
            }
            DimTextEditCommand::Home => {
                definition.text_middle = Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                };
            }
            DimTextEditCommand::Angle { degrees } => {
                if !degrees.is_finite() {
                    return Err(invalid(
                        "dimtedit.bad_rotation",
                        "the text rotation must be finite",
                    ));
                }
                // Group 53 stores radians from the UCS X axis; `0` is the
                // "derive from the dimension line" sentinel.
                definition.text_rotation = degrees.to_radians();
            }
            DimTextEditCommand::Oblique { .. } => {
                return Err(unsupported(
                    "dimtedit.oblique_unsupported",
                    "DIMTEDIT OBLIQUE needs a glyph-level oblique (slant) angle, and this build's \
                     text model stores none for any text (cad_domain::SemanticGeometry::Text has \
                     no oblique field); nothing was changed",
                ))
            }
        }
        let after = rebuild_dimension(
            &target.dimension,
            definition,
            None,
            target.dimension.text_override().map(str::to_string),
            dimtedit_alignment(&target.dimension, request),
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

    /// `DIMEDIT`: set/reset the text override, restore the derived text
    /// placement, or set the text rotation of every selected dimension.
    ///
    /// The documented options are `TEXT <override>`, `RESET`, `HOME`,
    /// `ROTATE <degrees>` and `OBLIQUE <degrees>`. `TEXT`/`RESET` write group 1;
    /// `HOME` restores the group-11 sentinel (the derived position) and `ROTATE`
    /// writes group 53 (radians from the UCS X axis, `0` = derive). `OBLIQUE`
    /// (the extension-line oblique angle) is a named capability refusal: the
    /// dimension entity stores no oblique angle, so nothing is written.
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
                    "DIMEDIT takes TEXT <override>, RESET, HOME, ROTATE <degrees> or OBLIQUE \
                     <degrees>",
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
        match &request {
            DimEditCommand::Oblique { .. } => {
                return Err(unsupported(
                    "dimedit.oblique_unsupported",
                    "DIMEDIT OBLIQUE needs an extension-line oblique angle, and \
                     cad_domain::DimensionGeometry stores none (the synthesis draws every \
                     extension line perpendicular to the dimension line); nothing was changed",
                ))
            }
            DimEditCommand::Rotate { degrees } if !degrees.is_finite() => {
                return Err(invalid(
                    "dimedit.bad_rotation",
                    "the text rotation must be finite",
                ))
            }
            _ => {}
        }
        let drawing = drawing_of(self, command)?;
        let selected = resolve_selected_dimensions(&drawing, command, references, "dimedit")?;
        let override_text = match &request {
            DimEditCommand::Text(text) => Some(text.clone()),
            DimEditCommand::Reset => None,
            // HOME/ROTATE keep the stored override.
            _ => None,
        };
        let mut changes = Vec::new();
        for target in selected {
            let mut definition = target.dimension.definition().clone();
            match &request {
                DimEditCommand::Home => {
                    definition.text_middle = Point3 {
                        x: 0.0,
                        y: 0.0,
                        z: 0.0,
                    };
                }
                DimEditCommand::Rotate { degrees } => {
                    definition.text_rotation = degrees.to_radians();
                }
                _ => {}
            }
            let text_override = match &request {
                DimEditCommand::Text(_) | DimEditCommand::Reset => override_text.clone(),
                _ => target.dimension.text_override().map(str::to_string),
            };
            let after = rebuild_dimension(
                &target.dimension,
                definition,
                None,
                text_override,
                target.dimension.text_h_align(),
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
                "DIMEDIT: every selected dimension already has the requested text override/\
                 placement (no transaction)",
            ));
        }
        let count = changes.len();
        self.commit_dimension_changes(
            command,
            changes,
            "dimedit",
            "dimedit.applied",
            &format!("DIMEDIT: updated {count} dimension(s)"),
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
    pub(super) fn commit_dimension_changes(
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
                layer_patches: Vec::new(),
                style_patches: Vec::new(),
                dim_style_patches: Vec::new(),
                mleader_style_patches: Vec::new(),
                layout_patches: Vec::new(),
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
pub(super) fn unchanged_outcome(code: &str, message: &str) -> CommandOutcome {
    report_outcome(vec![Diagnostic {
        object: None,
        code: code.into(),
        message: message.into(),
    }])
}
