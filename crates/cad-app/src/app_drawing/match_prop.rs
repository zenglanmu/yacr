//! `MATCHPROP`: paint the **first selected** entity's display properties onto
//! every other selected entity (AutoCAD's property painter, selection form).
//!
//! ## Selection order is the input
//!
//! The payload is [`CommandPayload::None`] (all properties) or the typed
//! [`MatchPropSettings`] subset. The selection **is** the object input, read
//! from the session at apply time (a stale UI cannot retarget a different
//! object). The entity order is the one [`SelectionSet`] already preserves — the
//! order the user picked, first-seen first. The **first** selected entity is the
//! source; every other selected entity is a destination. The order is never
//! re-sorted by id/handle, so the same pick sequence always yields the same
//! source. Fewer than two selected entities is an explicit refusal
//! (`matchprop.needs_two`): there is no source-without-destination mode.
//!
//! ## Basic and Special property groups (AutoCAD 2026)
//!
//! The Basic group is the six modelled display properties plus linetype scale:
//! [`crate::property_value::SUPPORTED_KEYS`] — `layer`, `color`, `linetype`,
//! `lineweight`, `transparency` — and the entity's own **linetype scale**
//! ([`cad_db::EntityRenderAttributes::linetype_scale`], mirrored inside an
//! explicit linetype). The group has a master switch (`basic`) and each value
//! keeps its own toggle. Plot style and thickness are not modelled on an entity
//! in this build and are a declared remaining boundary.
//!
//! The three **Special** groups are restricted by object class:
//!
//! * **Text** (`text_style`): copy the TEXT/MTEXT style reference (and its font
//!   key). TEXT↔MTEXT is allowed because both use the same [`SemanticGeometry::
//!   Text`] model; the text **height** is never copied.
//! * **Dimension** (`dimension_style`): copy the DIMENSION style name and
//!   resolved snapshot. The dimension **overrides** (text override / text
//!   placement) stay with the destination because they are rebuilt from its own
//!   definition.
//! * **Hatch** (`hatch`): copy the hatch style, pattern/gradient definition and
//!   pattern type. The hatch **origin/elevation** (the plane) is explicitly not
//!   copied.
//!
//! A group applies only when the source is of its class. A destination of
//! another class skips that group (reported in the diagnostic) instead of
//! failing the whole command.
//!
//! [`MatchPropSettings`] toggles each Basic property and each Special group; a
//! disabled property is neither resolved nor written, so a source with an
//! unwritable value for an unticked property does not refuse the command. The
//! accepted subset is reported in the success diagnostic. There is **no second
//! grammar**: each enabled source value is projected with
//! [`crate::property_value::property_value`] and re-parsed with
//! [`crate::property_value::parse`] against the same drawing tables, so the
//! panel, `SETENTITYPROPERTY` and MATCHPROP accept and refuse the same values.
//!
//! Different source and destination entity kinds are allowed — that is the point
//! of a property painter — as long as every value is valid for the destination.
//!
//! ## Locked destination layers
//!
//! A destination that would change and is on a locked layer is refused with the
//! database's stable `layer.locked` reason before anything is staged, even when
//! only display attributes would change (the display path deliberately bypasses
//! the lock). Moving a destination onto a locked source layer is refused the
//! same way; an already-matching destination is a no-op and is not judged.
//!
//! ## All-or-nothing, one transaction, one undo step
//!
//! Every refusal happens before the transaction is opened, so nothing is written
//! and no history entry appears:
//!
//! * `matchprop.needs_two` — fewer than two selected entities;
//! * `matchprop.invalid_payload` — a payload other than `None` or this command's
//!   typed settings;
//! * `property.not_addressable` / `property.target_missing` — a source **or**
//!   destination ref carries an instance path (a block-instance member) or a
//!   sub-element, belongs to another document, or does not resolve: the exact
//!   rule [`super::properties`] enforces (reused, not re-implemented);
//! * `layer.locked` — a destination is on (or would move onto) a locked layer;
//! * `matchprop.source_value` — an **enabled** source value has no writable
//!   projection (`property_value` returns `None`) or the shared grammar refuses
//!   it. The refusal names the source entity **and** the property and carries
//!   the parser's own stable `property.*` code.
//!
//! A destination that already matches the source contributes **no** change and
//! is never staged (so undo restores exactly the entities that changed). When
//! every destination already matches, the command reports `matchprop.unchanged`
//! and opens **no** transaction — never an empty undo entry. Otherwise the
//! changed entities are staged in one transaction / one undo step, with patches
//! over both state layers (entity row + display attributes), exactly like
//! `SETENTITYPROPERTY`.

use super::properties::{ensure_addressable, plan_entity, StagedChange};
use super::*;
use crate::property_value::{self, PropertyValue};
use cad_db::{
    sanitize_linetype_scale, DbEntity, DrawingDatabase, EntityLineType, EntityRenderAttributes,
};

impl Application {
    /// Paint the enabled subset of the first selected entity's display
    /// properties onto the rest of the selection in one transaction.
    pub(crate) fn match_prop(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let settings = match_prop_settings(command)?;

        let document_id = command.document;
        let drawing = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?
            .drawing
            .clone();

        // The selection's own order is the contract: first picked = source.
        let references = session.selection.refs();
        if references.len() < 2 {
            return Err(CadError::InvalidInput(format!(
                "matchprop.needs_two: MATCHPROP needs a source (the first selected entity) and \
                 at least one destination, got {} selected",
                references.len()
            )));
        }

        // Preflight **every** ref (source and destinations) before resolving a
        // value or staging anything: one non-addressable ref refuses the whole
        // command, exactly like the single-property write.
        for reference in references {
            ensure_addressable(drawing.as_ref(), command, reference)?;
        }

        let source = drawing
            .entity(references[0].entity)
            .expect("the preflight verified the source resolves")
            .clone();

        // Resolve only the enabled source values, once, against the drawing's
        // own tables.
        let targets = source_targets(&drawing, &source, &settings)?;
        let planned: Vec<&PropertyValue> = targets.iter().map(|(_, target)| target).collect();
        let source_scale = (settings.basic && settings.linetype_scale)
            .then(|| source_linetype_scale(&drawing.entity_render_attributes(source.id)));

        let mut special = SpecialReport::default();
        let mut staged_changes: Vec<StagedChange> = Vec::new();
        for reference in &references[1..] {
            let before = drawing
                .entity(reference.entity)
                .expect("the preflight verified every destination resolves")
                .clone();
            let change = plan_entity(&drawing, before.clone(), &planned);
            let change = apply_linetype_scale(&drawing, before.clone(), change, source_scale);
            if let Some(change) =
                apply_special_groups(&source, &before, change, &settings, &mut special)?
            {
                staged_changes.push(change);
            }
        }

        // Only entities that would actually change are judged by the lock rule;
        // an already-matching destination on a locked layer is a no-op. The
        // display path would accept the write, so this command owns the refusal.
        refuse_locked_changes(&drawing, &staged_changes, source.layer, &settings)?;

        let destinations = references.len() - 1;
        if staged_changes.is_empty() {
            // Nothing to write: no transaction, no ChangeSet, no history entry.
            return Ok(CommandOutcome {
                objects: Vec::new(),
                changes: None,
                diagnostics: vec![Diagnostic {
                    object: None,
                    code: "matchprop.unchanged".into(),
                    message: format!(
                        "MATCHPROP: all {destinations} destination entities already match source \
                         entity {} on {}{}",
                        source.id.0,
                        settings_label(&settings),
                        special.note()
                    ),
                }],
                measurement: None,
                point_report: None,
            });
        }

        let transaction = next_transaction_id();
        let mut candidate = drawing.as_ref().clone();
        let change_set = {
            let mut staged =
                candidate.begin_drawing_transaction("match entity properties", transaction)?;
            for change in &staged_changes {
                if change.after != change.before {
                    staged.update_entity(change.after.clone())?;
                }
                if let Some(attributes) = &change.after_attributes {
                    staged.update_entity_display(change.id, attributes.clone())?;
                }
            }
            staged.commit()?
        };

        // Read the committed state back so patches carry the exact stored value,
        // never a re-derivation.
        for change in &mut staged_changes {
            change.after = candidate
                .entity(change.id)
                .cloned()
                .ok_or_else(|| CadError::Invariant("edited entity vanished after commit".into()))?;
            if change.after_attributes.is_some() {
                change.after_attributes = Some(candidate.entity_render_attributes(change.id));
            }
        }

        let patches: Vec<DrawingPatch> = staged_changes
            .iter()
            .map(|change| DrawingPatch {
                id: change.id,
                before: Some(change.before.clone()),
                after: Some(change.after.clone()),
                before_state: Some(drawing.entity_display_state(change.id)),
                after_state: Some(candidate.entity_display_state(change.id)),
            })
            .collect();
        let objects: Vec<ObjectId> = staged_changes
            .iter()
            .map(|change| ObjectId(change.id.0))
            .collect();
        let count = objects.len();
        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: format!("match properties from {}", source.id.0),
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
                code: "drawing.matchprop".into(),
                message: format!(
                    "MATCHPROP: {count} of {destinations} destination entities now match source \
                     entity {} on {}{}",
                    source.id.0,
                    settings_label(&settings),
                    special.note()
                ),
            }],
            measurement: None,
            point_report: None,
        })
    }
}

/// Read the optional typed settings; `None` means all six properties on.
fn match_prop_settings(command: &Command) -> CadResult<MatchPropSettings> {
    match &command.payload {
        CommandPayload::None => Ok(MatchPropSettings::default()),
        CommandPayload::SelectionTool(payload) => match payload.as_ref() {
            SelectionToolPayload::MatchProp(settings) => Ok(*settings),
            _ => Err(CadError::InvalidInput(
                "matchprop.invalid_payload: MATCHPROP accepts only its own settings".into(),
            )),
        },
        _ => Err(CadError::InvalidInput(
            "matchprop.invalid_payload: MATCHPROP takes no payload or its typed settings".into(),
        )),
    }
}

/// Refuse a staged destination that is on, or would move onto, a locked layer.
fn refuse_locked_changes(
    drawing: &DrawingDatabase,
    changes: &[StagedChange],
    source_layer: LayerId,
    settings: &MatchPropSettings,
) -> CadResult<()> {
    for change in changes {
        if drawing.layer_locked(change.before.layer) {
            return Err(locked_layer_refusal(
                drawing,
                change.before.layer,
                change.id,
                "cannot modify entity",
            ));
        }
        if settings.layer
            && change.before.layer != source_layer
            && change.after.layer == source_layer
            && drawing.layer_locked(source_layer)
        {
            return Err(locked_layer_refusal(
                drawing,
                source_layer,
                change.id,
                "cannot move entity onto",
            ));
        }
    }
    Ok(())
}

fn locked_layer_refusal(
    drawing: &DrawingDatabase,
    layer: LayerId,
    entity: EntityId,
    action: &str,
) -> CadError {
    let name = drawing
        .layer(layer)
        .map(|row| row.name.as_str())
        .unwrap_or("<unknown>");
    CadError::InvalidInput(format!(
        "layer.locked: {action} {} on locked layer '{name}' ({}); unlock the layer first",
        entity.0, layer.0
    ))
}

/// Resolve the source's **enabled** writable properties into write targets.
///
/// One resolution per property, against the drawing's own tables, before any
/// entity is staged — so a refusal cannot depend on which destination is
/// processed first (the same ordering guarantee `SETENTITYPROPERTY` documents).
fn source_targets(
    drawing: &DrawingDatabase,
    source: &DbEntity,
    settings: &MatchPropSettings,
) -> CadResult<Vec<(&'static str, PropertyValue)>> {
    let mut targets = Vec::new();
    for key in property_value::SUPPORTED_KEYS {
        if !enabled(settings, key) {
            continue;
        }
        let Some(value) = property_value::property_value(drawing, source, key) else {
            return Err(CadError::Unsupported(format!(
                "matchprop.source_value: source entity {} has no writable {key} value to paint",
                source.id.0
            )));
        };
        let target = property_value::parse(drawing, key, &value)
            .map_err(|error| name_refusal(error, source.id, key, &value))?;
        targets.push((key, target));
    }
    Ok(targets)
}

fn enabled(settings: &MatchPropSettings, key: &str) -> bool {
    if !settings.basic {
        return false;
    }
    match key {
        "layer" => settings.layer,
        "color" => settings.color,
        "linetype" => settings.linetype,
        "lineweight" => settings.lineweight,
        "transparency" => settings.transparency,
        _ => false,
    }
}

/// Apply the enabled Special groups to one destination.
///
/// A group applies only when the **source** is of its class; a destination of
/// another class skips that group and increments the report counter. `None`
/// keeps the existing planned change untouched; a group that changes geometry
/// creates a change when the Basic group had none.
fn apply_special_groups(
    source: &DbEntity,
    before: &DbEntity,
    change: Option<StagedChange>,
    settings: &MatchPropSettings,
    report: &mut SpecialReport,
) -> CadResult<Option<StagedChange>> {
    if !settings.text_style && !settings.dimension_style && !settings.hatch {
        return Ok(change);
    }
    let mut after = change
        .as_ref()
        .map(|change| change.after.clone())
        .unwrap_or_else(|| before.clone());
    let mut geometry_changed = false;

    if settings.text_style {
        if let Some(source_text) = text_style_ref(&source.geometry) {
            if let Some(target_text) = text_style_ref(&before.geometry) {
                if target_text != source_text {
                    if let SemanticGeometry::Text { style, font, .. } = &mut after.geometry {
                        *style = source_text.style;
                        *font = source_text.font.clone();
                        geometry_changed = true;
                    }
                }
            } else {
                report.text_skipped += 1;
            }
        }
    }

    if settings.dimension_style {
        if let Some(source_dimension) = dimension_ref(&source.geometry) {
            if let Some(target_dimension) = dimension_ref(&before.geometry) {
                if target_dimension.style_name() != source_dimension.style_name()
                    || target_dimension.style() != source_dimension.style()
                {
                    let next = target_dimension
                        .with_style(
                            source_dimension.style_name().to_string(),
                            source_dimension.style().clone(),
                        )
                        .map_err(|error| {
                            CadError::Unsupported(format!(
                                "matchprop.special: source entity {} cannot paint its dimension \
                                 style onto entity {}: {error}",
                                source.id.0, before.id.0
                            ))
                        })?;
                    after.geometry = SemanticGeometry::Dimension(Box::new(next));
                    geometry_changed = true;
                }
            } else {
                report.dimension_skipped += 1;
            }
        }
    }

    if settings.hatch {
        if let Some(source_hatch) = hatch_ref(&source.geometry) {
            if let Some(target_hatch) = hatch_ref(&before.geometry) {
                if target_hatch.style() != source_hatch.style()
                    || target_hatch.fill() != source_hatch.fill()
                    || target_hatch.pattern_type() != source_hatch.pattern_type()
                {
                    // The target's own loops/plane (origin and elevation), seeds
                    // and pixel size are kept; only the hatch properties move.
                    let next = target_hatch.with_style_and_fill(
                        source_hatch.style(),
                        source_hatch.fill().clone(),
                        source_hatch.pattern_type(),
                    );
                    after.geometry = SemanticGeometry::Hatch(Box::new(next));
                    geometry_changed = true;
                }
            } else {
                report.hatch_skipped += 1;
            }
        }
    }

    match change {
        Some(mut change) => {
            if geometry_changed {
                change.after = after;
            }
            Ok(Some(change))
        }
        None if geometry_changed => {
            debug_assert!(after != *before, "a special group claims a geometry change");
            Ok(Some(StagedChange {
                id: before.id,
                before: before.clone(),
                after,
                after_attributes: None,
            }))
        }
        None => Ok(None),
    }
}

/// The TEXT/MTEXT style reference and its font key.
#[derive(PartialEq)]
struct TextStyleRef {
    style: StyleId,
    font: Option<String>,
}

fn text_style_ref(geometry: &SemanticGeometry) -> Option<TextStyleRef> {
    match geometry {
        SemanticGeometry::Text { style, font, .. } => Some(TextStyleRef {
            style: *style,
            font: font.clone(),
        }),
        _ => None,
    }
}

fn dimension_ref(geometry: &SemanticGeometry) -> Option<&DimensionGeometry> {
    match geometry {
        SemanticGeometry::Dimension(dimension) => Some(dimension),
        _ => None,
    }
}

fn hatch_ref(geometry: &SemanticGeometry) -> Option<&HatchGeometry> {
    match geometry {
        SemanticGeometry::Hatch(hatch) => Some(hatch),
        _ => None,
    }
}

/// Counts of destinations that skipped a Special group because they are not of
/// that group's class.
#[derive(Default)]
struct SpecialReport {
    text_skipped: usize,
    dimension_skipped: usize,
    hatch_skipped: usize,
}

impl SpecialReport {
    /// A diagnostic suffix naming every skipped group, or the empty string.
    fn note(&self) -> String {
        let mut parts = Vec::new();
        if self.text_skipped > 0 {
            parts.push(format!(
                "text style skipped on {} non-text destination(s)",
                self.text_skipped
            ));
        }
        if self.dimension_skipped > 0 {
            parts.push(format!(
                "dimension style skipped on {} non-dimension destination(s)",
                self.dimension_skipped
            ));
        }
        if self.hatch_skipped > 0 {
            parts.push(format!(
                "hatch properties skipped on {} non-hatch destination(s)",
                self.hatch_skipped
            ));
        }
        if parts.is_empty() {
            String::new()
        } else {
            format!("; {}", parts.join("; "))
        }
    }
}

/// The authoritative source linetype scale: an explicit linetype's own factor
/// is the one the explicit render path resolves, otherwise the attribute field.
fn source_linetype_scale(attributes: &EntityRenderAttributes) -> f64 {
    match &attributes.linetype {
        EntityLineType::Explicit { scale, .. } => sanitize_linetype_scale(*scale),
        EntityLineType::ByBlock | EntityLineType::ByLayer => {
            sanitize_linetype_scale(attributes.linetype_scale)
        }
    }
}

/// Apply the source linetype scale when that Settings toggle is on.
///
/// `plan_entity` cannot express it (the property grammar does not expose the
/// scale), so this adds the sixth property to the same staged change. Both the
/// attribute field and an explicit linetype's inner copy are written, keeping
/// the two authorities the importer keeps in sync.
fn apply_linetype_scale(
    drawing: &DrawingDatabase,
    before: DbEntity,
    change: Option<StagedChange>,
    source_scale: Option<f64>,
) -> Option<StagedChange> {
    let Some(scale) = source_scale else {
        return change;
    };
    let mut attributes = change
        .as_ref()
        .and_then(|change| change.after_attributes.clone())
        .unwrap_or_else(|| drawing.entity_render_attributes(before.id));
    let mut changed = false;
    if sanitize_linetype_scale(attributes.linetype_scale) != scale {
        attributes.linetype_scale = scale;
        changed = true;
    }
    if let EntityLineType::Explicit { scale: inner, .. } = &mut attributes.linetype {
        if sanitize_linetype_scale(*inner) != scale {
            *inner = scale;
            changed = true;
        }
    }
    match change {
        Some(mut change) => {
            if changed {
                change.after_attributes = Some(attributes);
            }
            Some(change)
        }
        None if changed => Some(StagedChange {
            id: before.id,
            before: before.clone(),
            after: before,
            after_attributes: Some(attributes),
        }),
        None => None,
    }
}

fn settings_label(settings: &MatchPropSettings) -> String {
    let mut parts: Vec<&str> = Vec::new();
    if settings.basic {
        if settings.layer {
            parts.push("layer");
        }
        if settings.color {
            parts.push("color");
        }
        if settings.linetype {
            parts.push("linetype");
        }
        if settings.linetype_scale {
            parts.push("linetype scale");
        }
        if settings.lineweight {
            parts.push("lineweight");
        }
        if settings.transparency {
            parts.push("transparency");
        }
    }
    if settings.text_style {
        parts.push("text style");
    }
    if settings.dimension_style {
        parts.push("dimension style");
    }
    if settings.hatch {
        parts.push("hatch");
    }
    if parts.is_empty() {
        "no properties".to_string()
    } else {
        parts.join(", ")
    }
}

/// Name the source entity and the property in the shared parser's refusal.
///
/// The [`CadError`] variant is preserved so the parser's stable `property.*`
/// code and the command-layer classification (a refused input vs. an unsupported
/// value) survive; only the message gains the MATCHPROP context.
fn name_refusal(error: CadError, entity: EntityId, key: &str, value: &str) -> CadError {
    let context = format!(
        "matchprop.source_value: source entity {} cannot paint {key} = '{value}': ",
        entity.0
    );
    match error {
        CadError::InvalidInput(message) => CadError::InvalidInput(format!("{context}{message}")),
        CadError::Unsupported(message) => CadError::Unsupported(format!("{context}{message}")),
        other => other,
    }
}
