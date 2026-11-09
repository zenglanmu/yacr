//! `HATCHEDIT` / `-HATCHEDIT` (AutoCAD 2026): the declared typed-option subset.
//!
//! AutoCAD's HATCHEDIT opens a dialog (or `-HATCHEDIT` prompts) and edits one or
//! more hatch/fill objects. This build has no Hatch Edit dialog, so the option is
//! typed as part of the command line (`HATCHEDIT STYLE OUTER`,
//! `HATCHEDIT COLOR 30`, `HATCHEDIT RECREATEBOUNDARY`, …) and carried in
//! [`CommandPayload::HatchEdit`](crate::CommandPayload::HatchEdit). The current
//! selection is the rest of the input: **one or more** directly addressable
//! model-space `AcDbHatch` entities.
//!
//! Implemented options:
//!
//! * `STYLE IGNORE|OUTER|NORMAL` — the DXF group 75 island-detection style;
//! * `ANGLE` / `SCALE` / `PATTERNTYPE` — the pattern properties (pattern fill
//!   only for angle/scale);
//! * `SOLID` / `GRADIENT` — the fill variant;
//! * `COLOR <aci>` / `COLOR BYLAYER` — the entity display colour;
//! * `TRANSPARENCY <percent>` / `TRANSPARENCY BYLAYER` — the entity opacity
//!   (`0..=90` percent transparent, DXF/AutoCAD's entity bound);
//! * `LAYER <name>` — the owning drawing layer;
//! * `DRAWORDER BEHIND|FRONT|BOUNDARY` — model-space draw order
//!   (`BOUNDARY` is a named refusal: there is no boundary record);
//! * `RECREATEBOUNDARY` — one closed polyline per stored loop;
//! * `SEPARATE` — split a multi-loop hatch into one hatch per loop.
//!
//! Named refusals (global scope, before the selection is read): `ORIGIN`
//! (`hatchedit.missing_field`: no pattern-origin field), `ASSOCIATE` /
//! `DISASSOCIATE` (`hatchedit.no_association`) and `ANNOTATIVE`
//! (`hatchedit.annotative_unsupported`). `ADDBOUNDARIES` / `REMOVEBOUNDARIES`
//! are `hatchedit.boundaries_unsupported`: the domain cannot rebuild a loop set
//! from picks, so pretending to add/remove one would be a lie.
//!
//! The edit rebuilds hatch geometry through the domain constructor
//! (`HatchGeometry::from_import` / `with_style_and_fill`) from the stored loops,
//! plane, style, seeds and pixel size — the display is never hand-edited — and
//! writes every target in **one transaction / one undo step**. A run that
//! changes nothing is the named `hatchedit.unchanged` outcome with no
//! transaction.
//!
//! Full contract: [`crate::command_id::hatchedit`].

use super::*;

use super::hatch_parameters::default_gradient;
use super::hatch_utilities::loop_world_ring;
use crate::{HatchDrawOrder, HatchEditOption};
use cad_db::{DbEntity, DrawingDatabase, EntityColor, EntityRenderAttributes, EntityTransparency};

/// A `hatchedit.<code>: <detail>` refusal string (ASCII only).
fn refusal(code: &str, detail: impl std::fmt::Display) -> String {
    format!("hatchedit.{code}: {detail}")
}

/// A named, non-capability refusal (bad input for a supported shape).
fn invalid(code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::InvalidInput(refusal(code, detail))
}

/// A named capability-gap refusal (nothing is written).
fn unsupported(code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::Unsupported(refusal(code, detail))
}

/// The named no-op outcome (`hatchedit.unchanged`): no transaction, no revision
/// bump and no undo entry.
fn unchanged(option: &HatchEditOption) -> CommandOutcome {
    CommandOutcome {
        objects: Vec::new(),
        changes: None,
        diagnostics: vec![Diagnostic {
            object: None,
            code: "hatchedit.unchanged".into(),
            message: format!(
                "HATCHEDIT {}: every selected hatch already stores that value; nothing was \
                 written",
                option_label(option)
            ),
        }],
        measurement: None,
        point_report: None,
    }
}

fn option_label(option: &HatchEditOption) -> String {
    match option {
        HatchEditOption::Angle(degrees) => format!("ANGLE {degrees}"),
        HatchEditOption::Scale(factor) => format!("SCALE {factor}"),
        HatchEditOption::PatternType(pattern_type) => {
            format!("PATTERNTYPE {}", pattern_type_label(*pattern_type))
        }
        HatchEditOption::Style(style) => format!("STYLE {}", style_label(*style)),
        HatchEditOption::DrawOrder(order) => format!("DRAWORDER {}", draw_order_label(*order)),
        HatchEditOption::Solid => "SOLID".into(),
        HatchEditOption::Gradient => "GRADIENT".into(),
        HatchEditOption::RecreateBoundary => "RECREATEBOUNDARY".into(),
        HatchEditOption::SeparateHatches => "SEPARATE".into(),
        HatchEditOption::Color(aci) => format!("COLOR {aci}"),
        HatchEditOption::ColorByLayer => "COLOR BYLAYER".into(),
        HatchEditOption::Transparency(percent) => format!("TRANSPARENCY {percent}"),
        HatchEditOption::TransparencyByLayer => "TRANSPARENCY BYLAYER".into(),
        HatchEditOption::Layer(name) => format!("LAYER {name}"),
        HatchEditOption::Origin => "ORIGIN".into(),
        HatchEditOption::Associate => "ASSOCIATE".into(),
        HatchEditOption::Disassociate => "DISASSOCIATE".into(),
        HatchEditOption::Annotative => "ANNOTATIVE".into(),
        HatchEditOption::AddBoundaries => "ADDBOUNDARIES".into(),
        HatchEditOption::RemoveBoundaries => "REMOVEBOUNDARIES".into(),
    }
}

fn pattern_type_label(pattern_type: HatchPatternType) -> &'static str {
    match pattern_type {
        HatchPatternType::UserDefined => "USER",
        HatchPatternType::Predefined => "PREDEFINED",
        HatchPatternType::Custom => "CUSTOM",
    }
}

fn style_label(style: HatchStyle) -> &'static str {
    match style {
        HatchStyle::Normal => "NORMAL",
        HatchStyle::Outer => "OUTER",
        HatchStyle::Ignore => "IGNORE",
    }
}

fn draw_order_label(order: HatchDrawOrder) -> &'static str {
    match order {
        HatchDrawOrder::BehindAll => "BEHIND",
        HatchDrawOrder::InFrontOfAll => "FRONT",
        HatchDrawOrder::BehindBoundary => "BOUNDARY",
    }
}

/// One stored hatch resolved from the selection.
struct Target {
    entity: DbEntity,
    hatch: HatchGeometry,
}

/// One updated entity plus an optional display-attribute write.
struct Change {
    before: DbEntity,
    after: DbEntity,
    display: Option<EntityRenderAttributes>,
}

/// One inserted entity plus an optional display-attribute write.
struct NewEntity {
    entity: DbEntity,
    display: Option<EntityRenderAttributes>,
}

/// Rebuild one hatch with a new fill and pattern type, keeping every other
/// stored field. `from_import` is the domain constructor that derives a fresh
/// display without rejecting an already-stored boundary.
fn rebuild(
    hatch: &HatchGeometry,
    fill: HatchFill,
    pattern_type: HatchPatternType,
) -> SemanticGeometry {
    let (rebuilt, _reasons) = HatchGeometry::from_import(
        hatch.loops().to_vec(),
        hatch.plane(),
        hatch.style(),
        fill,
        hatch.seeds().to_vec(),
        hatch.pixel_size(),
        pattern_type,
    );
    SemanticGeometry::Hatch(Box::new(rebuilt))
}

impl Application {
    /// `HATCHEDIT`: apply one typed option to every selected hatch.
    ///
    /// Refusals write nothing and open no transaction: a bare payload
    /// (`hatchedit.needs_option`), any other payload (`hatchedit.payload`), an
    /// empty selection (`hatchedit.selection`), a duplicate ref
    /// (`hatchedit.duplicate`), a ref that is not directly addressable
    /// model-space (`hatchedit.not_addressable`), another geometry kind
    /// (`hatchedit.not_hatch`), ANGLE/SCALE on a Solid/Gradient fill
    /// (`hatchedit.not_pattern`), a bad angle/scale/colour/transparency/layer
    /// (`hatchedit.angle`/`scale`/`color`/`transparency`/`unknown_layer`) and
    /// the global capability refusals listed in the module docs.
    pub(crate) fn hatch_edit(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let option = match &command.payload {
            CommandPayload::HatchEdit(option) => option.clone(),
            CommandPayload::None => {
                return Err(invalid(
                    "needs_option",
                    "HATCHEDIT needs an option: STYLE, ANGLE, SCALE, PATTERNTYPE, SOLID, \
                     GRADIENT, COLOR, TRANSPARENCY, LAYER, DRAWORDER, RECREATEBOUNDARY or \
                     SEPARATE",
                ));
            }
            _ => {
                return Err(invalid(
                    "payload",
                    "HATCHEDIT takes a HatchEdit payload; the selection is the input",
                ));
            }
        };
        // Global refusals: the same answer for every selection (and before it is
        // read), because the missing model is global.
        match &option {
            HatchEditOption::Origin => {
                return Err(unsupported(
                    "missing_field",
                    "the hatch model stores no pattern origin, so HATCHEDIT ORIGIN cannot set one",
                ))
            }
            HatchEditOption::Associate | HatchEditOption::Disassociate => {
                return Err(unsupported(
                    "no_association",
                    "this build stores non-associative hatches; there is no association record to \
                     add or remove",
                ))
            }
            HatchEditOption::Annotative => {
                return Err(unsupported(
                    "annotative_unsupported",
                    "this build has no annotative-scale model for hatches",
                ))
            }
            HatchEditOption::AddBoundaries | HatchEditOption::RemoveBoundaries => {
                return Err(unsupported(
                    "boundaries_unsupported",
                    "the domain cannot rebuild a hatch loop set safely from added/removed \
                     boundary picks, so HATCHEDIT ADDBOUNDARIES/REMOVEBOUNDARIES changes nothing",
                ))
            }
            HatchEditOption::DrawOrder(HatchDrawOrder::BehindBoundary) => {
                return Err(unsupported(
                    "draw_order_boundary",
                    "DRAWORDER BOUNDARY needs a stored boundary/associativity record, which this \
                     build does not have; use BEHIND or FRONT",
                ))
            }
            _ => {}
        }

        let references = session.selection.refs();
        if references.is_empty() {
            return Err(invalid(
                "selection",
                "HATCHEDIT needs at least one selected hatch",
            ));
        }
        let drawing = {
            let document = self
                .workspace
                .documents
                .get(&command.document)
                .ok_or_else(|| invalid("not_addressable", "document not open"))?;
            document.drawing.clone()
        };
        let mut candidate = drawing.as_ref().clone();
        let targets = resolve_targets(&candidate, command, references)?;

        let mut changes: Vec<Change> = Vec::new();
        let mut inserts: Vec<NewEntity> = Vec::new();
        match &option {
            HatchEditOption::Angle(degrees) => {
                if !degrees.is_finite() {
                    return Err(invalid(
                        "angle",
                        format!("HATCHEDIT ANGLE needs a finite angle, got {degrees}"),
                    ));
                }
                for target in &targets {
                    let HatchFill::Pattern(pattern) = target.hatch.fill() else {
                        return Err(unsupported(
                            "not_pattern",
                            format!(
                                "HATCHEDIT ANGLE applies to a pattern fill only; entity {} \
                                 stores {}",
                                target.entity.id.0,
                                fill_kind(target.hatch.fill())
                            ),
                        ));
                    };
                    if pattern.angle == degrees.to_radians() {
                        continue;
                    }
                    let mut changed = pattern.clone();
                    changed.angle = degrees.to_radians();
                    push_geometry_change(
                        &mut changes,
                        target,
                        rebuild(
                            &target.hatch,
                            HatchFill::Pattern(changed),
                            target.hatch.pattern_type(),
                        ),
                    );
                }
            }
            HatchEditOption::Scale(factor) => {
                if !factor.is_finite() || *factor <= 0.0 {
                    return Err(invalid(
                        "scale",
                        format!("HATCHEDIT SCALE needs a finite factor > 0, got {factor}"),
                    ));
                }
                for target in &targets {
                    let HatchFill::Pattern(pattern) = target.hatch.fill() else {
                        return Err(unsupported(
                            "not_pattern",
                            format!(
                                "HATCHEDIT SCALE applies to a pattern fill only; entity {} \
                                 stores {}",
                                target.entity.id.0,
                                fill_kind(target.hatch.fill())
                            ),
                        ));
                    };
                    if pattern.scale == *factor {
                        continue;
                    }
                    let mut changed = pattern.clone();
                    changed.scale = *factor;
                    push_geometry_change(
                        &mut changes,
                        target,
                        rebuild(
                            &target.hatch,
                            HatchFill::Pattern(changed),
                            target.hatch.pattern_type(),
                        ),
                    );
                }
            }
            HatchEditOption::PatternType(pattern_type) => {
                for target in &targets {
                    if target.hatch.pattern_type() == *pattern_type {
                        continue;
                    }
                    let rebuilt = target.hatch.with_style_and_fill(
                        target.hatch.style(),
                        target.hatch.fill().clone(),
                        *pattern_type,
                    );
                    push_geometry_change(
                        &mut changes,
                        target,
                        SemanticGeometry::Hatch(Box::new(rebuilt)),
                    );
                }
            }
            HatchEditOption::Style(style) => {
                for target in &targets {
                    if target.hatch.style() == *style {
                        continue;
                    }
                    let rebuilt = target.hatch.with_style_and_fill(
                        *style,
                        target.hatch.fill().clone(),
                        target.hatch.pattern_type(),
                    );
                    push_geometry_change(
                        &mut changes,
                        target,
                        SemanticGeometry::Hatch(Box::new(rebuilt)),
                    );
                }
            }
            HatchEditOption::Solid => {
                for target in &targets {
                    if matches!(target.hatch.fill(), HatchFill::Solid) {
                        continue;
                    }
                    push_geometry_change(
                        &mut changes,
                        target,
                        rebuild(
                            &target.hatch,
                            HatchFill::Solid,
                            HatchPatternType::Predefined,
                        ),
                    );
                }
            }
            HatchEditOption::Gradient => {
                for target in &targets {
                    if matches!(target.hatch.fill(), HatchFill::Gradient(_)) {
                        continue;
                    }
                    push_geometry_change(
                        &mut changes,
                        target,
                        rebuild(
                            &target.hatch,
                            HatchFill::Gradient(default_gradient()),
                            HatchPatternType::Predefined,
                        ),
                    );
                }
            }
            HatchEditOption::Color(aci) => {
                let rgb = crate::property_value::rgb_for_aci(*aci as u16).ok_or_else(|| {
                    invalid(
                        "color",
                        format!(
                            "HATCHEDIT COLOR needs an ACI index 1..=255, got {aci} (0 and 256 \
                             are symbolic; use COLOR BYLAYER)"
                        ),
                    )
                })?;
                for target in &targets {
                    let mut attributes = candidate.entity_render_attributes(target.entity.id);
                    if attributes.color == EntityColor::Explicit(rgb) {
                        continue;
                    }
                    attributes.color = EntityColor::Explicit(rgb);
                    push_display_change(&mut changes, target, attributes);
                }
            }
            HatchEditOption::ColorByLayer => {
                for target in &targets {
                    let mut attributes = candidate.entity_render_attributes(target.entity.id);
                    if attributes.color == EntityColor::ByLayer {
                        continue;
                    }
                    attributes.color = EntityColor::ByLayer;
                    push_display_change(&mut changes, target, attributes);
                }
            }
            HatchEditOption::Transparency(percent) => {
                if *percent > 90 {
                    return Err(invalid(
                        "transparency",
                        format!(
                            "HATCHEDIT TRANSPARENCY needs 0..=90 percent transparent, got \
                             {percent}"
                        ),
                    ));
                }
                let opacity = 1.0 - f32::from(*percent) / 100.0;
                for target in &targets {
                    let mut attributes = candidate.entity_render_attributes(target.entity.id);
                    if attributes.transparency == EntityTransparency::Explicit(opacity) {
                        continue;
                    }
                    attributes.transparency = EntityTransparency::Explicit(opacity);
                    push_display_change(&mut changes, target, attributes);
                }
            }
            HatchEditOption::TransparencyByLayer => {
                for target in &targets {
                    let mut attributes = candidate.entity_render_attributes(target.entity.id);
                    if attributes.transparency == EntityTransparency::ByLayer {
                        continue;
                    }
                    attributes.transparency = EntityTransparency::ByLayer;
                    push_display_change(&mut changes, target, attributes);
                }
            }
            HatchEditOption::Layer(name) => {
                let layer = resolve_layer(&candidate, name)?;
                for target in &targets {
                    if target.entity.layer == layer {
                        continue;
                    }
                    let mut after = target.entity.clone();
                    after.layer = layer;
                    changes.push(Change {
                        before: target.entity.clone(),
                        after,
                        display: None,
                    });
                }
            }
            HatchEditOption::DrawOrder(order) => {
                let selected: std::collections::BTreeSet<EntityId> =
                    targets.iter().map(|target| target.entity.id).collect();
                let moves = match order {
                    HatchDrawOrder::BehindAll => behind_all_orders(&candidate, &selected)?,
                    HatchDrawOrder::InFrontOfAll => front_all_orders(&candidate, &selected)?,
                    HatchDrawOrder::BehindBoundary => unreachable!("refused above"),
                };
                for (id, target_order) in moves {
                    let Some(target) = targets.iter().find(|target| target.entity.id == id) else {
                        continue;
                    };
                    if target.entity.draw_order == target_order {
                        continue;
                    }
                    let mut after = target.entity.clone();
                    after.draw_order = target_order;
                    changes.push(Change {
                        before: target.entity.clone(),
                        after,
                        display: None,
                    });
                }
            }
            HatchEditOption::RecreateBoundary => {
                let mut base_order = next_model_order(&candidate);
                for target in &targets {
                    if target.hatch.loops().is_empty() {
                        return Err(invalid(
                            "empty",
                            format!(
                                "hatch {} stores no boundary loops to recreate",
                                target.entity.id.0
                            ),
                        ));
                    }
                    for loop_ in target.hatch.loops() {
                        let (points, bulges) = loop_world_ring(loop_, target.hatch.plane())
                            .map_err(|detail| {
                                invalid(
                                    "invalid_loop",
                                    format!("hatch {} {detail}", target.entity.id.0),
                                )
                            })?;
                        let entity_id = candidate.allocate_entity_id();
                        inserts.push(NewEntity {
                            entity: DbEntity {
                                object: cad_db::DbObject {
                                    id: ObjectId(entity_id.0),
                                    type_key: "AcDbPolyline".into(),
                                    revision: Revision(0),
                                    source_handle: None,
                                },
                                id: entity_id,
                                layer: target.entity.layer,
                                space: target.entity.space.clone(),
                                geometry: SemanticGeometry::Polyline {
                                    points,
                                    bulges,
                                    closed: true,
                                },
                                draw_order: base_order,
                            },
                            display: None,
                        });
                        base_order += 1;
                    }
                }
            }
            HatchEditOption::SeparateHatches => {
                let mut base_order = next_model_order(&candidate);
                for target in &targets {
                    let loops = target.hatch.loops();
                    if loops.len() <= 1 {
                        continue;
                    }
                    // The original keeps the first loop; the others become their own
                    // hatches (nested-island semantics are not preserved — declared).
                    let (first, rest) = loops.split_first().expect("len > 1");
                    let (rebuilt, _reasons) = HatchGeometry::from_import(
                        vec![first.clone()],
                        target.hatch.plane(),
                        target.hatch.style(),
                        target.hatch.fill().clone(),
                        target.hatch.seeds().to_vec(),
                        target.hatch.pixel_size(),
                        target.hatch.pattern_type(),
                    );
                    push_geometry_change(
                        &mut changes,
                        target,
                        SemanticGeometry::Hatch(Box::new(rebuilt)),
                    );
                    let display = candidate.entity_render_attributes(target.entity.id);
                    for loop_ in rest {
                        let (rebuilt, _reasons) = HatchGeometry::from_import(
                            vec![loop_.clone()],
                            target.hatch.plane(),
                            target.hatch.style(),
                            target.hatch.fill().clone(),
                            target.hatch.seeds().to_vec(),
                            target.hatch.pixel_size(),
                            target.hatch.pattern_type(),
                        );
                        let entity_id = candidate.allocate_entity_id();
                        inserts.push(NewEntity {
                            entity: DbEntity {
                                object: cad_db::DbObject {
                                    id: ObjectId(entity_id.0),
                                    type_key: "AcDbHatch".into(),
                                    revision: Revision(0),
                                    source_handle: None,
                                },
                                id: entity_id,
                                layer: target.entity.layer,
                                space: target.entity.space.clone(),
                                geometry: SemanticGeometry::Hatch(Box::new(rebuilt)),
                                draw_order: base_order,
                            },
                            display: Some(display.clone()),
                        });
                        base_order += 1;
                    }
                }
            }
            HatchEditOption::Origin
            | HatchEditOption::Associate
            | HatchEditOption::Disassociate
            | HatchEditOption::Annotative
            | HatchEditOption::AddBoundaries
            | HatchEditOption::RemoveBoundaries => unreachable!("refused above"),
        }

        if changes.is_empty() && inserts.is_empty() {
            return Ok(unchanged(&option));
        }
        self.commit_hatch_edits(
            command,
            drawing.as_ref(),
            candidate,
            changes,
            inserts,
            &option,
        )
    }

    /// Apply the planned hatch changes in one transaction / one undo step.
    fn commit_hatch_edits(
        &mut self,
        command: &Command,
        before_drawing: &DrawingDatabase,
        mut candidate: DrawingDatabase,
        changes: Vec<Change>,
        inserts: Vec<NewEntity>,
        option: &HatchEditOption,
    ) -> CadResult<CommandOutcome> {
        let document_id = command.document;
        let transaction = next_transaction_id();
        let change_set = {
            let mut staged = candidate.begin_drawing_transaction("hatchedit", transaction)?;
            for new in &inserts {
                staged.insert_entity(new.entity.clone())?;
                if let Some(attributes) = &new.display {
                    staged.update_entity_display(new.entity.id, attributes.clone())?;
                }
            }
            for change in &changes {
                if change.after != change.before {
                    staged.update_entity(change.after.clone())?;
                }
                if let Some(attributes) = &change.display {
                    staged.update_entity_display(change.after.id, attributes.clone())?;
                }
            }
            staged.commit()?
        };
        let mut patches = Vec::with_capacity(changes.len() + inserts.len());
        let mut objects = Vec::with_capacity(changes.len() + inserts.len());
        for change in &changes {
            let stored = candidate.entity(change.before.id).cloned().ok_or_else(|| {
                CadError::Invariant("HATCHEDIT: an updated hatch vanished after commit".into())
            })?;
            patches.push(DrawingPatch {
                id: change.before.id,
                before: Some(change.before.clone()),
                after: Some(stored),
                before_state: Some(before_drawing.entity_display_state(change.before.id)),
                after_state: Some(candidate.entity_display_state(change.before.id)),
            });
            objects.push(ObjectId(change.before.id.0));
        }
        for new in &inserts {
            let stored = candidate.entity(new.entity.id).cloned().ok_or_else(|| {
                CadError::Invariant("HATCHEDIT: a created entity vanished after commit".into())
            })?;
            patches.push(DrawingPatch {
                id: new.entity.id,
                before: None,
                after: Some(stored),
                before_state: None,
                after_state: new
                    .display
                    .as_ref()
                    .map(|_| candidate.entity_display_state(new.entity.id)),
            });
            objects.push(ObjectId(new.entity.id.0));
        }
        let count = objects.len();
        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: "hatchedit".to_string(),
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
                code: "hatchedit.applied".into(),
                message: format!(
                    "HATCHEDIT {}: applied to {count} object(s) in one transaction",
                    option_label(option)
                ),
            }],
            measurement: None,
            point_report: None,
        })
    }
}

/// Resolve every selected ref to a directly addressable model-space hatch.
fn resolve_targets(
    drawing: &DrawingDatabase,
    command: &Command,
    references: &[SelectionRef],
) -> CadResult<Vec<Target>> {
    let mut seen = std::collections::BTreeSet::new();
    let mut targets = Vec::with_capacity(references.len());
    for reference in references {
        if !seen.insert(reference.entity) {
            return Err(invalid(
                "duplicate",
                format!("entity {} is selected twice", reference.entity.0),
            ));
        }
        if reference.document != command.document {
            return Err(invalid(
                "not_addressable",
                format!(
                    "the selected ref for entity {} belongs to another document",
                    reference.entity.0
                ),
            ));
        }
        if !reference.instance.0.is_empty() || reference.sub_element.is_some() {
            return Err(unsupported(
                "not_addressable",
                format!(
                    "HATCHEDIT needs directly addressable model-space hatches; entity {} is \
                     referenced through a block instance or a sub-element",
                    reference.entity.0
                ),
            ));
        }
        let entity = drawing.entity(reference.entity).cloned().ok_or_else(|| {
            invalid(
                "not_addressable",
                format!(
                    "entity {} does not exist in this drawing",
                    reference.entity.0
                ),
            )
        })?;
        if entity.space != SpaceId::Model {
            return Err(unsupported(
                "not_addressable",
                format!(
                    "HATCHEDIT edits model-space hatches only; entity {} is in another space",
                    reference.entity.0
                ),
            ));
        }
        let SemanticGeometry::Hatch(hatch) = entity.geometry.clone() else {
            return Err(unsupported(
                "not_hatch",
                format!(
                    "HATCHEDIT needs HATCH selections; entity {} is {}",
                    reference.entity.0,
                    class_name(&entity.geometry)
                ),
            ));
        };
        targets.push(Target {
            entity,
            hatch: *hatch,
        });
    }
    Ok(targets)
}

fn push_geometry_change(changes: &mut Vec<Change>, target: &Target, geometry: SemanticGeometry) {
    let mut after = target.entity.clone();
    after.geometry = geometry;
    if after.geometry != target.entity.geometry {
        changes.push(Change {
            before: target.entity.clone(),
            after,
            display: None,
        });
    }
}

fn push_display_change(
    changes: &mut Vec<Change>,
    target: &Target,
    attributes: EntityRenderAttributes,
) {
    changes.push(Change {
        before: target.entity.clone(),
        after: target.entity.clone(),
        display: Some(attributes),
    });
}

/// The layer id for a typed layer name (exact first, then a unique
/// case-insensitive match), or the named refusal.
fn resolve_layer(drawing: &DrawingDatabase, name: &str) -> CadResult<LayerId> {
    if let Some(layer) = drawing.layer_id_by_name(name) {
        return Ok(layer);
    }
    let matches: Vec<LayerId> = drawing
        .layers()
        .filter(|layer| layer.name.eq_ignore_ascii_case(name))
        .map(|layer| layer.id)
        .collect();
    match matches.as_slice() {
        [] => Err(invalid(
            "unknown_layer",
            format!("this drawing has no layer named '{name}'"),
        )),
        [only] => Ok(*only),
        _ => Err(invalid(
            "ambiguous_layer",
            format!(
                "'{name}' matches {} layers case-insensitively; use the exact name",
                matches.len()
            ),
        )),
    }
}

/// The first free draw order above every model-space row.
fn next_model_order(drawing: &DrawingDatabase) -> i64 {
    drawing
        .entities()
        .filter(|entity| entity.space == SpaceId::Model)
        .map(|entity| entity.draw_order)
        .max()
        .map(|order| order.saturating_add(1))
        .unwrap_or(0)
}

/// Target orders that put the selected hatches behind every other model-space
/// object, keeping their relative order.
fn behind_all_orders(
    drawing: &DrawingDatabase,
    selected: &std::collections::BTreeSet<EntityId>,
) -> CadResult<Vec<(EntityId, i64)>> {
    let min_other = drawing
        .entities()
        .filter(|entity| entity.space == SpaceId::Model && !selected.contains(&entity.id))
        .map(|entity| entity.draw_order)
        .min();
    let Some(min_other) = min_other else {
        return Ok(Vec::new());
    };
    let mut ordered: Vec<&DbEntity> = drawing
        .entities()
        .filter(|entity| selected.contains(&entity.id))
        .collect();
    ordered.sort_by_key(|entity| (entity.draw_order, entity.id.0));
    if ordered.iter().all(|entity| entity.draw_order < min_other) {
        return Ok(Vec::new());
    }
    let count = i64::try_from(ordered.len())
        .map_err(|_| invalid("order_exhausted", "too many selected hatches to reorder"))?;
    let first = min_other.checked_sub(count).ok_or_else(|| {
        invalid(
            "order_exhausted",
            "the target draw orders would underflow i64",
        )
    })?;
    Ok(ordered
        .iter()
        .enumerate()
        .map(|(index, entity)| (entity.id, first + index as i64))
        .collect())
}

/// Target orders that put the selected hatches in front of every other
/// model-space object, keeping their relative order.
fn front_all_orders(
    drawing: &DrawingDatabase,
    selected: &std::collections::BTreeSet<EntityId>,
) -> CadResult<Vec<(EntityId, i64)>> {
    let base = drawing
        .entities()
        .filter(|entity| entity.space == SpaceId::Model && !selected.contains(&entity.id))
        .map(|entity| entity.draw_order)
        .max()
        .unwrap_or(0);
    let mut ordered: Vec<&DbEntity> = drawing
        .entities()
        .filter(|entity| selected.contains(&entity.id))
        .collect();
    ordered.sort_by_key(|entity| (entity.draw_order, entity.id.0));
    if ordered.iter().all(|entity| entity.draw_order > base) {
        return Ok(Vec::new());
    }
    let mut out = Vec::with_capacity(ordered.len());
    for (index, entity) in ordered.iter().enumerate() {
        let target = base.checked_add(index as i64 + 1).ok_or_else(|| {
            invalid(
                "order_exhausted",
                "the target draw orders would overflow i64",
            )
        })?;
        out.push((entity.id, target));
    }
    Ok(out)
}

/// The human half of a fill refusal (the stored variant, never a pattern name).
fn fill_kind(fill: &HatchFill) -> &'static str {
    match fill {
        HatchFill::Solid => "a solid fill",
        HatchFill::Pattern(_) => "a pattern fill",
        HatchFill::Gradient(_) => "a gradient fill",
    }
}
