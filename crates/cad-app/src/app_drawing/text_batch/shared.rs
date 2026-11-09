//! Shared helpers of the 2026-11-07 2D text batch: the one extents estimate,
//! the model-space target resolver and the transaction/publish funnels.
//!
//! Nothing here is a second write authority: `commit_text_updates` and
//! `commit_new_rows` both open exactly one `begin_drawing_transaction`, stage the
//! documented op per row, commit, read the committed rows back for the undo
//! patch and publish through [`Application::publish_candidate`].

use super::*;

/// A `<command>.<code>` refusal: `InvalidInput` carrying its stable code.
pub(super) fn refusal(code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::InvalidInput(format!("{code}: {detail}"))
}

/// The channel error's inner message (a named refusal keeps its own text).
pub(super) fn error_detail(error: CadError) -> String {
    match error {
        CadError::InvalidInput(message) | CadError::Unsupported(message) => message,
        other => format!("{other:?}"),
    }
}

/// The ASCII name a refused target kind is reported under (no class prefix).
pub(super) fn kind_name(geometry: &SemanticGeometry) -> &'static str {
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
        SemanticGeometry::Mask { .. } => "WIPEOUT",
        SemanticGeometry::Text { .. } => "TEXT",
        _ => "that entity",
    }
}

/// The **estimated** world-space box of one text entity.
///
/// This is the single extents authority of the batch: width is
/// `height × text.chars().count()` and height is `height` (world units); the
/// anchor follows `h_align`/`v_align`; `z` is the insertion `z`. The box is
/// axis-aligned and **ignores rotation** — a declared estimate, never a glyph
/// metric.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct EstimatedTextBounds {
    pub min_x: f64,
    pub min_y: f64,
    pub max_x: f64,
    pub max_y: f64,
    pub z: f64,
}

impl EstimatedTextBounds {
    pub(crate) fn width(&self) -> f64 {
        self.max_x - self.min_x
    }

    pub(crate) fn height(&self) -> f64 {
        self.max_y - self.min_y
    }

    pub(crate) fn center(&self) -> (f64, f64) {
        (
            (self.min_x + self.max_x) * 0.5,
            (self.min_y + self.max_y) * 0.5,
        )
    }

    /// The four corners in a counter-clockwise outline starting at the
    /// lower-left corner (the `RECTANG`/mask boundary order).
    pub(crate) fn corners(&self) -> Vec<Point3> {
        vec![
            Point3 {
                x: self.min_x,
                y: self.min_y,
                z: self.z,
            },
            Point3 {
                x: self.max_x,
                y: self.min_y,
                z: self.z,
            },
            Point3 {
                x: self.max_x,
                y: self.max_y,
                z: self.z,
            },
            Point3 {
                x: self.min_x,
                y: self.max_y,
                z: self.z,
            },
        ]
    }
}

/// The estimated bounds of one `Text` entity, or a named refusal.
///
/// `prefix` is the command's diagnostic prefix (`tcircle`, `textmask`, …), so the
/// degenerate-value refusal keeps the command's own stable code.
pub(crate) fn estimate_text_bounds(
    entity: &DbEntity,
    prefix: &str,
) -> CadResult<EstimatedTextBounds> {
    let SemanticGeometry::Text {
        text,
        position,
        height,
        h_align,
        v_align,
        ..
    } = &entity.geometry
    else {
        return Err(CadError::Unsupported(format!(
            "{prefix}.not_text: {} is not a Text/MText entity",
            kind_name(&entity.geometry)
        )));
    };
    if !is_finite_point(*position) {
        return Err(refusal(
            &format!("{prefix}.degenerate"),
            format!("the text entity {} has a non-finite position", entity.id.0),
        ));
    }
    if !height.is_finite() || *height <= 0.0 {
        return Err(refusal(
            &format!("{prefix}.degenerate"),
            format!(
                "the text entity {} has the degenerate stored height {height}",
                entity.id.0
            ),
        ));
    }
    let width = *height * text.chars().count() as f64;
    let (left, right) = match h_align {
        TextAlignH::Left => (position.x, position.x + width),
        TextAlignH::Center => (position.x - width * 0.5, position.x + width * 0.5),
        TextAlignH::Right => (position.x - width, position.x),
    };
    let (bottom, top) = match v_align {
        TextAlignV::Baseline | TextAlignV::Bottom => (position.y, position.y + height),
        TextAlignV::Middle => (position.y - height * 0.5, position.y + height * 0.5),
        TextAlignV::Top => (position.y - height, position.y),
    };
    if ![left, right, bottom, top]
        .iter()
        .all(|value| value.is_finite())
    {
        return Err(refusal(
            &format!("{prefix}.degenerate"),
            format!(
                "the text entity {} has non-finite estimated extents",
                entity.id.0
            ),
        ));
    }
    Ok(EstimatedTextBounds {
        min_x: left,
        min_y: bottom,
        max_x: right,
        max_y: top,
        z: position.z,
    })
}

/// Resolve one selection ref to a directly addressable **model-space** entity.
///
/// The caller decides what to do with a non-text geometry (the whole-command
/// `Unsupported` for most commands, a per-entity diagnostic for JUSTIFYTEXT and
/// TEXTUNMASK). `prefix` supplies the command's stable refusal codes.
pub(crate) fn resolve_model_entity(
    drawing: &DrawingDatabase,
    document_id: DocumentId,
    reference: &SelectionRef,
    prefix: &str,
) -> CadResult<DbEntity> {
    if reference.document != document_id {
        return Err(refusal(
            &format!("{prefix}.not_addressable"),
            format!("target {} belongs to another document", reference.entity.0),
        ));
    }
    if !reference.instance.0.is_empty() || reference.sub_element.is_some() {
        return Err(refusal(
            &format!("{prefix}.not_addressable"),
            format!(
                "target {} is a block-instance member or sub-element; only directly \
                 addressable model-space entities are supported",
                reference.entity.0
            ),
        ));
    }
    let entity = drawing.entity(reference.entity).cloned().ok_or_else(|| {
        refusal(
            &format!("{prefix}.target_missing"),
            format!("target {} does not exist", reference.entity.0),
        )
    })?;
    if entity.space != SpaceId::Model {
        return Err(refusal(
            &format!("{prefix}.not_addressable"),
            format!(
                "target {} is not a model-space entity; only model space is supported",
                entity.id.0
            ),
        ));
    }
    Ok(entity)
}

/// Refuse a non-text target for the **whole** command (the `SCALETEXT` rule).
pub(super) fn require_text(prefix: &str, entity: &DbEntity) -> CadResult<()> {
    if matches!(entity.geometry, SemanticGeometry::Text { .. }) {
        Ok(())
    } else {
        Err(CadError::Unsupported(format!(
            "{prefix}.not_text: {prefix} supports Text and MText only; {} is not supported",
            kind_name(&entity.geometry)
        )))
    }
}

/// The named no-op outcome: no transaction, no `ChangeSet`, no undo entry.
pub(super) fn unchanged_outcome(
    code: &str,
    message: String,
    diagnostics: Vec<Diagnostic>,
) -> CommandOutcome {
    let mut diagnostics = diagnostics;
    diagnostics.push(Diagnostic {
        object: None,
        code: code.into(),
        message,
    });
    CommandOutcome {
        objects: Vec::new(),
        changes: None,
        diagnostics,
        measurement: None,
        point_report: None,
    }
}

/// One per-entity skip diagnostic (used by JUSTIFYTEXT and TEXTUNMASK).
pub(super) fn skipped(code: &str, entity: EntityId, detail: &str) -> Diagnostic {
    Diagnostic {
        object: Some(ObjectId(entity.0)),
        code: code.into(),
        message: format!("{code}: entity {} was left unchanged: {detail}", entity.0),
    }
}

/// Commit every planned text update in **one** transaction / undo step.
pub(super) fn commit_text_updates(
    app: &mut Application,
    document_id: DocumentId,
    mut candidate: DrawingDatabase,
    planned: &[(DbEntity, DbEntity)],
    label: &str,
    transaction_label: &str,
) -> CadResult<(Vec<ObjectId>, Arc<ChangeSet>)> {
    let transaction = next_transaction_id();
    let change_set = {
        let mut staged = candidate.begin_drawing_transaction(transaction_label, transaction)?;
        for (_, after) in planned {
            staged.update_entity(after.clone())?;
        }
        staged.commit()?
    };
    let mut patches = Vec::with_capacity(planned.len());
    let mut objects = Vec::with_capacity(planned.len());
    for (before, _) in planned {
        let after = candidate.entity(before.id).cloned().ok_or_else(|| {
            CadError::Invariant(format!("{label}: an updated text vanished after commit"))
        })?;
        patches.push(drawing_patch(before.id, Some(before.clone()), Some(after)));
        objects.push(ObjectId(before.id.0));
    }
    app.publish_candidate(
        document_id,
        candidate,
        DrawingUndoRecord {
            transaction: change_set.transaction,
            label: label.into(),
            patches,
            block_patches: Vec::new(),
            layer_patches: Vec::new(),
            layout_patches: Vec::new(),
            merge_key: None,
        },
    )?;
    Ok((objects, Arc::new(change_set)))
}

/// Insert every planned new entity in **one** transaction / undo step.
pub(super) fn commit_new_rows(
    app: &mut Application,
    document_id: DocumentId,
    mut candidate: DrawingDatabase,
    entities: Vec<DbEntity>,
    label: &str,
    transaction_label: &str,
) -> CadResult<(Vec<ObjectId>, Arc<ChangeSet>)> {
    let transaction = next_transaction_id();
    let change_set = {
        let mut staged = candidate.begin_drawing_transaction(transaction_label, transaction)?;
        for entity in &entities {
            staged.insert_entity(entity.clone())?;
        }
        staged.commit()?
    };
    let mut patches = Vec::with_capacity(entities.len());
    let mut objects = Vec::with_capacity(entities.len());
    for before in entities {
        let after = candidate.entity(before.id).cloned().ok_or_else(|| {
            CadError::Invariant(format!("{label}: an inserted entity vanished after commit"))
        })?;
        patches.push(drawing_patch(before.id, None, Some(after)));
        objects.push(ObjectId(before.id.0));
    }
    app.publish_candidate(
        document_id,
        candidate,
        DrawingUndoRecord {
            transaction: change_set.transaction,
            label: label.into(),
            patches,
            block_patches: Vec::new(),
            layer_patches: Vec::new(),
            layout_patches: Vec::new(),
            merge_key: None,
        },
    )?;
    Ok((objects, Arc::new(change_set)))
}

/// The model-space draw order one past the current maximum (the create funnel's
/// rule), used to place TCIRCLE boundaries above existing geometry.
pub(super) fn next_model_draw_order(drawing: &DrawingDatabase) -> i64 {
    drawing
        .entities()
        .filter(|entity| entity.space == SpaceId::Model)
        .map(|entity| entity.draw_order)
        .max()
        .map(|order| order.saturating_add(1))
        .unwrap_or(0)
}

/// Shift one text entity's `position` on one axis so `current` becomes `target`.
///
/// `axis == 0.0` moves X, `axis == 1.0` moves Y. Returns whether the position
/// really changed (a zero delta is a no-op, never a staged write).
pub(super) fn shift_text(entity: &mut DbEntity, current: f64, target: f64, axis: f64) -> bool {
    let delta = target - current;
    if !delta.is_finite() || delta == 0.0 {
        return false;
    }
    let SemanticGeometry::Text { position, .. } = &mut entity.geometry else {
        return false;
    };
    if axis == 0.0 {
        position.x += delta;
    } else {
        position.y += delta;
    }
    true
}
