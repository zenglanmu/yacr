//! `SCALETEXT`: scale the **glyph height** of the selected Text/MText entities.
//!
//! The capture is the zero-point pre-selection shape
//! ([`crate::draw_tool::DrawToolKind::ScaleText`]): the selection snapshot is the
//! whole input and the factor is the session typed channel
//! ([`crate::draw_tool::ToolParameter::ScaleTextFactor`], finite and strictly
//! positive, default `1.0`). Nothing is picked inside the command.
//!
//! ## What changes (and what never does)
//!
//! `TEXT` and `MTEXT` share the [`SemanticGeometry::Text`] variant (their
//! `object.type_key` is what distinguishes them), so both are scaled the same way:
//!
//! * `height` becomes `old_height · factor`;
//! * an MTEXT `wrap_width: Some(w)` becomes `Some(w · factor)` (the wrap boundary
//!   is a world-unit distance, so it scales with the glyphs); `None` stays `None`.
//!
//! Every other field is preserved **bit-for-bit**: `text`, `position`, `style`,
//! `rotation`, `font`, `h_align`/`v_align` and the entity's own `type_key`, layer,
//! space and draw order are untouched. The entity id is the same id (an update,
//! not a replacement). This is not SCALE: there is no base point, no position
//! change and no uniform transform of the drawing.
//!
//! ## Refusals (all before any write)
//!
//! | code | meaning |
//! | --- | --- |
//! | `scaletext.needs_selection` | `refs` is empty |
//! | `scaletext.not_addressable` | block-instance member, sub-element, or non-model-space target |
//! | `scaletext.target_missing` | a ref does not resolve (or belongs to another document) |
//! | `scaletext.invalid_factor` | the session factor is not finite / not `> 0` |
//! | `scaletext.degenerate` | a stored height (or set wrap width) is not finite / not `> 0` |
//! | `scaletext.invalid_result` | the scaled height/width is non-finite, non-positive or above the declared bound |
//! | `scaletext.unchanged` | the factor is `1.0`, or no target would change — a **success** with no transaction |
//!
//! A non-text entity — or a mixed Text + other-kind selection — is the explicit
//! [`CadError::Unsupported`] `"SCALETEXT supports Text and MText only; <kind> is
//! not supported"` for the **whole** command; no entity is partially written.
//!
//! ## One transaction, one undo step
//!
//! Every refusal happens before `begin_drawing_transaction`, so a refused
//! SCALETEXT writes nothing: no revision bump, no `ChangeSet`, no undo entry. The
//! `1.0`/no-change case reports `scaletext.unchanged` and opens no transaction
//! (never an empty undo entry). On success every changed entity is staged in one
//! transaction / one undo step, and the committed rows are read back so the undo
//! patch carries the exact stored value. Work-only through
//! [`crate::CommandId::ScaleText`].

use super::*;

/// The declared upper bound on a scaled text dimension, in world units.
///
/// The drawing already refuses a non-finite/zero dimension; this second bound
/// keeps a large finite factor from producing an absurd (but technically finite)
/// glyph. It is stated rather than left to `f64::MAX`, so an oversized result is
/// a named `scaletext.invalid_result` rather than a silent success.
const MAX_TEXT_DIMENSION: f64 = 1.0e12;

/// A SCALETEXT refusal: `InvalidInput` carrying its stable code.
fn refusal(code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::InvalidInput(format!("{code}: {detail}"))
}

/// The name a refused target kind is reported under (ASCII, no class prefix).
fn kind_name(geometry: &SemanticGeometry) -> &'static str {
    match geometry {
        SemanticGeometry::Line { .. } => "LINE",
        SemanticGeometry::Arc { .. } => "ARC",
        SemanticGeometry::Circle { .. } => "CIRCLE",
        SemanticGeometry::Polyline { .. } => "LWPOLYLINE",
        SemanticGeometry::Ellipse { .. } => "ELLIPSE",
        SemanticGeometry::Spline { .. } => "SPLINE",
        SemanticGeometry::Point(_) => "POINT",
        SemanticGeometry::Shape { .. } => "SHAPE",
        SemanticGeometry::Insert { .. } => "INSERT",
        SemanticGeometry::Hatch(_) => "HATCH",
        SemanticGeometry::MultiLeader(_) => "MULTILEADER",
        SemanticGeometry::Text { .. } => "TEXT",
        _ => "that entity",
    }
}

/// The channel error's inner message (a named refusal keeps its own text).
fn error_detail(error: CadError) -> String {
    match error {
        CadError::InvalidInput(message) | CadError::Unsupported(message) => message,
        other => format!("{other:?}"),
    }
}

impl Application {
    /// Scale the selected Text/MText glyph height by the session factor.
    ///
    /// One transaction / one undo step; every refusal leaves the drawing
    /// untouched and creates no history entry.
    pub(crate) fn scale_text(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::ScaleText { refs } = &command.payload else {
            return Err(CadError::InvalidInput(
                "SCALETEXT needs a ScaleText { refs } payload".into(),
            ));
        };
        // 1. At least one selected entity.
        if refs.is_empty() {
            return Err(refusal(
                "scaletext.needs_selection",
                "SCALETEXT needs at least one selected Text/MText entity",
            ));
        }
        // 2. Directly addressable entities only. The check runs before the drawing
        //    is read so a block-instance member or a sub-element is never mistaken
        //    for a whole selectable text entity.
        for reference in refs {
            if !reference.instance.0.is_empty() || reference.sub_element.is_some() {
                return Err(refusal(
                    "scaletext.not_addressable",
                    format!(
                        "SCALETEXT target {} is a block-instance member or sub-element; only \
                         directly addressable model-space entities can be scaled",
                        reference.entity.0
                    ),
                ));
            }
        }
        let factor = scale_text_factor(session)?;

        let document_id = command.document;
        // 3. Resolve every ref and validate the whole target set as plain data,
        //    before any transaction exists.
        let sources = {
            let document = self
                .workspace
                .documents
                .get(&document_id)
                .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
            let drawing = document.drawing.as_ref();
            let (sources, _diagnostics) = Self::resolve_selection(
                drawing,
                document_id,
                refs,
                "scaletext",
                "SCALETEXT",
                "scaletext",
                false,
            )
            .map_err(|error| {
                refusal(
                    "scaletext.target_missing",
                    format!(
                        "a SCALETEXT target does not resolve: {}",
                        error_detail(error)
                    ),
                )
            })?;
            for source in &sources {
                if source.space != SpaceId::Model {
                    return Err(refusal(
                        "scaletext.not_addressable",
                        format!(
                            "SCALETEXT target {} is not a model-space entity; only model space \
                             is supported",
                            source.id.0
                        ),
                    ));
                }
                if !matches!(source.geometry, SemanticGeometry::Text { .. }) {
                    return Err(CadError::Unsupported(format!(
                        "SCALETEXT supports Text and MText only; {} is not supported",
                        kind_name(&source.geometry)
                    )));
                }
            }
            sources
        };
        let count = sources.len();

        // 4. A factor of exactly 1.0 changes nothing: report the named no-op and
        //    open no transaction (never an empty undo entry).
        if factor == 1.0 {
            return Ok(unchanged(count, factor));
        }

        // 5. Build every candidate as plain data; a degenerate stored value or an
        //    invalid result refuses the whole command before the transaction.
        let mut planned: Vec<(DbEntity, DbEntity)> = Vec::with_capacity(sources.len());
        for before in sources {
            let after = scaled_text(&before, factor)?;
            if after.geometry != before.geometry {
                planned.push((before, after));
            }
        }
        if planned.is_empty() {
            return Ok(unchanged(count, factor));
        }

        // 6. One transaction for every changed entity.
        let mut candidate = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?
            .drawing
            .as_ref()
            .clone();
        let transaction = next_transaction_id();
        let change_set = {
            let mut staged = candidate.begin_drawing_transaction("scale text", transaction)?;
            for (_, after) in &planned {
                staged.update_entity(after.clone())?;
            }
            staged.commit()?
        };

        // Read every committed row back so the undo patch carries the exact stored
        // value, never a re-derivation.
        let mut patches = Vec::with_capacity(planned.len());
        let mut objects = Vec::with_capacity(planned.len());
        for (before, _) in &planned {
            let after = candidate
                .entity(before.id)
                .cloned()
                .ok_or_else(|| CadError::Invariant("scaled text vanished after commit".into()))?;
            patches.push(drawing_patch(before.id, Some(before.clone()), Some(after)));
            objects.push(ObjectId(before.id.0));
        }
        let changed = objects.len();
        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: "scale text".into(),
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
                code: "drawing.scaletext".into(),
                message: format!(
                    "SCALETEXT: scaled {changed} text entities by factor {factor} \
                     ({count} target(s) verified)"
                ),
            }],
            measurement: None,
            point_report: None,
        })
    }
}

/// The named no-op outcome: no transaction, no `ChangeSet`, no undo entry.
fn unchanged(count: usize, factor: f64) -> CommandOutcome {
    CommandOutcome {
        objects: Vec::new(),
        changes: None,
        diagnostics: vec![Diagnostic {
            object: None,
            code: "scaletext.unchanged".into(),
            message: format!(
                "SCALETEXT: factor {factor} leaves all {count} selected text entities unchanged"
            ),
        }],
        measurement: None,
        point_report: None,
    }
}

/// Read and defensively re-validate the SCALETEXT factor.
///
/// The channel refuses a non-finite/`<= 0` factor at set time; this is the
/// defense in depth for a value injected past the channel. `1.0` is a valid
/// channel value and is handled by the command as the named no-op.
fn scale_text_factor(session: &SessionState) -> CadResult<f64> {
    let parameter = ToolParameter::ScaleTextFactor;
    let value = session.tool_parameter(parameter);
    parameter.validate(value).map_err(|_| {
        refusal(
            "scaletext.invalid_factor",
            format!("SCALETEXT factor must be finite and > 0, got {value:?}"),
        )
    })?;
    match value {
        ToolValue::Number(factor) if factor.is_finite() && factor > 0.0 => Ok(factor),
        other => Err(refusal(
            "scaletext.invalid_factor",
            format!("SCALETEXT needs a finite, strictly positive factor, got {other:?}"),
        )),
    }
}

/// The scaled copy of one Text/MTEXT entity, preserving every other field.
///
/// The **single** scale rule: `TSCALE` (typed factor) and `SCALETEXT` (session
/// tool parameter) both call this, so the two can never diverge.
pub(super) fn scaled_text(before: &DbEntity, factor: f64) -> CadResult<DbEntity> {
    let SemanticGeometry::Text {
        height, wrap_width, ..
    } = &before.geometry
    else {
        // The caller validated the whole target set; refuse instead of guessing.
        return Err(CadError::Unsupported(format!(
            "SCALETEXT supports Text and MText only; {} is not supported",
            kind_name(&before.geometry)
        )));
    };
    let old_height = *height;
    let old_width = *wrap_width;
    if !old_height.is_finite() || old_height <= 0.0 {
        return Err(refusal(
            "scaletext.degenerate",
            format!(
                "the SCALETEXT target {} has the degenerate stored height {old_height}",
                before.id.0
            ),
        ));
    }
    if let Some(width) = old_width {
        if !width.is_finite() || width <= 0.0 {
            return Err(refusal(
                "scaletext.degenerate",
                format!(
                    "the SCALETEXT target {} has the degenerate stored wrap width {width}",
                    before.id.0
                ),
            ));
        }
    }
    let new_height = old_height * factor;
    if !valid_dimension(new_height) {
        return Err(refusal(
            "scaletext.invalid_result",
            format!(
                "the SCALETEXT height {} × {factor} = {new_height} is not a usable dimension",
                old_height
            ),
        ));
    }
    let new_width = match old_width {
        Some(width) => {
            let scaled = width * factor;
            if !valid_dimension(scaled) {
                return Err(refusal(
                    "scaletext.invalid_result",
                    format!(
                        "the SCALETEXT wrap width {} × {factor} = {scaled} is not a usable \
                         dimension",
                        width
                    ),
                ));
            }
            Some(scaled)
        }
        None => None,
    };
    let mut after = before.clone();
    if let SemanticGeometry::Text {
        height, wrap_width, ..
    } = &mut after.geometry
    {
        *height = new_height;
        *wrap_width = new_width;
    }
    Ok(after)
}

/// Whether a scaled height/width is finite, strictly positive and within the
/// declared bound.
fn valid_dimension(value: f64) -> bool {
    value.is_finite() && value > 0.0 && value <= MAX_TEXT_DIMENSION
}
