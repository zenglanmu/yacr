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
//! * `PATTERN <name>` — replace the fill with the named built-in `.pat` library
//!   pattern, keeping the hatch's stored pattern angle/scale/`double` and
//!   recording DXF group 76 as `Predefined` (`1`, what the HATCH creation path
//!   writes for the same library name). Pattern fill only: a solid/gradient fill
//!   is `hatchedit.pattern_requires_pattern`. The name set is the one
//!   [`super::hatch_pattern::PatternLibrary`] table (the `.pat` loader pinned to
//!   `cad_domain::hatch::predefined_patterns`), matched ASCII
//!   case-insensitively; an unknown name is `hatchedit.unknown_pattern` and a
//!   missing name is `hatchedit.needs_pattern_name`;
//! * `PATTERNFILE <path.pat> [<name>]` — the same edit from a **user** `.pat`
//!   file, parsed by that same [`super::hatch_pattern::PatternLibrary`] reader and
//!   recording group 76 as `Custom` (`2`, the DXF classification of a user-file
//!   pattern). The host read the text (the payload carries it, see
//!   [`crate::PatternFileText`]), so this command never touches the filesystem; a
//!   payload without the text is `hatchedit.pattern_file_not_read`. With no name
//!   the file must hold exactly one pattern (`hatchedit.pattern_file_ambiguous`
//!   otherwise); with a name the file is searched first and the built-in table
//!   second, and a name in neither is `hatchedit.unknown_pattern` naming both
//!   sources;
//! * `SOLID` / `GRADIENT` — the fill variant;
//! * `COLOR <aci>` / `COLOR BYLAYER` — the entity display colour;
//! * `TRANSPARENCY <percent>` / `TRANSPARENCY BYLAYER` — the entity opacity
//!   (`0..=90` percent transparent, DXF/AutoCAD's entity bound);
//! * `LAYER <name>` — the owning drawing layer;
//! * `ORIGIN <x> <y>` — move the pattern origin (pattern fill only). The value is
//!   a hatch-plane displacement of the pattern's definition rows, because
//!   neither the DXF nor the DWG HATCH record has a dedicated origin group: both
//!   store the phase in each row's base point (groups 43/44), which export
//!   rewrites. `ORIGIN 0 0` is therefore the file's own origin, and the display
//!   (pattern lines and dash phase) is rebuilt from the domain constructor;
//! * `DRAWORDER BEHIND|FRONT|BOUNDARY` — model-space draw order
//!   (`BOUNDARY` is a named refusal: there is no boundary record);
//! * `RECREATEBOUNDARY` — one closed polyline per stored loop;
//! * `SEPARATE` — split a multi-loop hatch into one hatch per loop;
//! * `ADDBOUNDARIES` / `REMOVEBOUNDARIES` — edit the stored loop set from the
//!   selected closed entities (see below).
//!
//! Named refusals (global scope, before the selection is read): `ASSOCIATE` /
//! `DISASSOCIATE` (`hatchedit.no_association`) and `ANNOTATIVE`
//! (`hatchedit.annotative_unsupported`). `PATTERN <name>` resolves its name
//! against the built-in `.pat` library there too: an empty name is
//! `hatchedit.needs_pattern_name`, an unknown name is
//! `hatchedit.unknown_pattern` (with the accepted names), and both write
//! nothing. A `.pat` file that does not parse is `hatchedit.pattern_file_invalid`
//! (the parser's own refusal and the path in the detail). A Solid/Gradient fill
//! is the per-target `hatchedit.pattern_requires_pattern` refusal for
//! `PATTERN`/`PATTERNFILE`, and `hatchedit.not_pattern` for
//! `ANGLE`/`SCALE`/`ORIGIN`.
//!
//! `ADDBOUNDARIES` / `REMOVEBOUNDARIES` use the pre-selection model: the
//! selection must hold exactly **one** hatch (the target) plus one or more
//! closed boundary entities (`Polyline`/`Circle`/full `Ellipse`) on that
//! hatch's plane. Every non-hatch selection is converted with the same
//! `HATCHSETBOUNDARY` projection (`annot_batch::hatch::boundary_loop`), so a
//! non-coplanar or unsupported shape is refused by that code. `ADDBOUNDARIES`
//! appends the converted loops; `REMOVEBOUNDARIES` removes the stored loops that
//! match a converted loop by **points + bulges** (`1e-9` tolerance, flags and
//! source edges ignored) and refuses when nothing matches or when the last loop
//! would go. The rebuilt set goes through `HatchGeometry::new` (constructor
//! validation: duplicate/crossing rings are `hatchedit.*_boundaries_invalid`),
//! keeping the hatch plane, style, fill, seeds, pixel size and pattern type.
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

use super::hatch_edit_draw_order::{behind_all_orders, front_all_orders, next_model_order};
use super::hatch_edit_labels::option_label;
use super::hatch_edit_pattern_file::{resolve_file_pattern, PatternChoice};
use super::hatch_edit_pattern_name::resolve_builtin_pattern;
use super::hatch_parameters::default_gradient;
use super::hatch_utilities::loop_world_ring;
use crate::{HatchDrawOrder, HatchEditOption};
use cad_db::{DbEntity, DrawingDatabase, EntityColor, EntityRenderAttributes, EntityTransparency};

/// A `hatchedit.<code>: <detail>` refusal string (ASCII only).
fn refusal(code: &str, detail: impl std::fmt::Display) -> String {
    format!("hatchedit.{code}: {detail}")
}

/// A named, non-capability refusal (bad input for a supported shape).
pub(super) fn invalid(code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::InvalidInput(refusal(code, detail))
}

/// A named capability-gap refusal (nothing is written).
pub(super) fn unsupported(code: &str, detail: impl std::fmt::Display) -> CadError {
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

/// One stored hatch resolved from the selection.
pub(super) struct Target {
    pub(super) entity: DbEntity,
    pub(super) hatch: HatchGeometry,
}

/// One updated entity plus an optional display-attribute write.
pub(super) struct Change {
    pub(super) before: DbEntity,
    pub(super) after: DbEntity,
    pub(super) display: Option<EntityRenderAttributes>,
}

/// One inserted entity plus an optional display-attribute write.
pub(super) struct NewEntity {
    entity: DbEntity,
    display: Option<EntityRenderAttributes>,
}

/// Rebuild one hatch with a new fill and pattern type, keeping every other
/// stored field (the pattern origin included). `from_import` is the domain
/// constructor that derives a fresh display without rejecting an already-stored
/// boundary; it starts at the zero origin, so the stored displacement is applied
/// back explicitly.
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
    let rebuilt = rebuilt.with_pattern_origin(hatch.pattern_origin());
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
    /// (`hatchedit.not_hatch`), ANGLE/SCALE/ORIGIN on a Solid/Gradient fill
    /// (`hatchedit.not_pattern`), PATTERN/PATTERNFILE on a Solid/Gradient fill
    /// (`hatchedit.pattern_requires_pattern`), an empty/unknown pattern name
    /// (`hatchedit.needs_pattern_name`/`unknown_pattern`), an unusable pattern
    /// file (`hatchedit.pattern_file_*`), a bad
    /// angle/scale/origin/colour/transparency/layer
    /// (`hatchedit.angle`/`scale`/`origin`/`color`/`transparency`/`unknown_layer`)
    /// and the global capability refusals listed in the module docs.
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
                    "HATCHEDIT needs an option: STYLE, ANGLE, SCALE, ORIGIN, PATTERN, \
                     PATTERNFILE, PATTERNTYPE, SOLID, GRADIENT, COLOR, TRANSPARENCY, LAYER, \
                     DRAWORDER, RECREATEBOUNDARY or SEPARATE",
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
                // Handled after the drawing is loaded, before the generic
                // per-hatch target resolution: the selection mixes the one target
                // hatch with plain boundary entities.
            }
            HatchEditOption::DrawOrder(HatchDrawOrder::BehindBoundary) => {
                return Err(unsupported(
                    "draw_order_boundary",
                    "DRAWORDER BOUNDARY needs the hatch's boundary entities, and this build \
                     stores none: the HATCH record's boundary handles (group 97/330) are read by \
                     the importer but not kept in the domain hatch (only the derived loops are), \
                     and an entity-id record would need the invalidation/reactor model this build \
                     refuses. Use BEHIND or FRONT",
                ))
            }
            _ => {}
        }
        // The pattern-library lookup is global: an unknown (or missing) name, an
        // unreadable/unparsable `.pat` file and an ambiguous file are all refused
        // before the selection is read, and open no transaction.
        let pattern_choice = match &option {
            HatchEditOption::Pattern(name) => {
                Some(PatternChoice::Builtin(resolve_builtin_pattern(name)?))
            }
            HatchEditOption::PatternFile { path, name, text } => Some(resolve_file_pattern(
                path,
                name.as_deref(),
                text.as_ref().map(crate::PatternFileText::as_str),
            )?),
            _ => None,
        };

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
        // ADDBOUNDARIES/REMOVEBOUNDARIES take exactly one target hatch plus
        // plain boundary entities, so they cannot go through `resolve_targets`
        // (which requires every selected entity to be a hatch).
        if matches!(
            option,
            HatchEditOption::AddBoundaries | HatchEditOption::RemoveBoundaries
        ) {
            return self.hatch_edit_boundaries(
                command,
                references,
                drawing.as_ref(),
                candidate,
                &option,
            );
        }
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
            HatchEditOption::Origin { x, y } => {
                if !x.is_finite() || !y.is_finite() {
                    return Err(invalid(
                        "origin",
                        format!(
                            "HATCHEDIT ORIGIN needs finite hatch-plane coordinates, got ({x}, {y})"
                        ),
                    ));
                }
                let origin = [*x, *y];
                for target in &targets {
                    if !matches!(target.hatch.fill(), HatchFill::Pattern(_)) {
                        return Err(unsupported(
                            "not_pattern",
                            format!(
                                "HATCHEDIT ORIGIN applies to a pattern fill only; entity {} \
                                 stores {}",
                                target.entity.id.0,
                                fill_kind(target.hatch.fill())
                            ),
                        ));
                    }
                    if target.hatch.pattern_origin() == origin {
                        continue;
                    }
                    push_geometry_change(
                        &mut changes,
                        target,
                        SemanticGeometry::Hatch(Box::new(target.hatch.with_pattern_origin(origin))),
                    );
                }
            }
            HatchEditOption::Pattern(_) | HatchEditOption::PatternFile { .. } => {
                let choice = pattern_choice.expect("the pattern was resolved before the selection");
                let definition = choice.definition();
                for target in &targets {
                    let HatchFill::Pattern(pattern) = target.hatch.fill() else {
                        return Err(unsupported(
                            "pattern_requires_pattern",
                            format!(
                                "HATCHEDIT {} applies to a pattern fill only; entity {} stores {}",
                                option_label(&option),
                                target.entity.id.0,
                                fill_kind(target.hatch.fill())
                            ),
                        ));
                    };
                    // The library entry supplies the name and the definition rows
                    // only: the hatch keeps its stored angle (already radians),
                    // scale and `HPDOUBLE` flag exactly.
                    let replaced = definition.to_hatch_pattern_radians(
                        pattern.scale,
                        pattern.angle,
                        pattern.double,
                    );
                    if pattern == &replaced {
                        continue;
                    }
                    push_geometry_change(
                        &mut changes,
                        target,
                        rebuild(
                            &target.hatch,
                            HatchFill::Pattern(replaced),
                            choice.pattern_type(),
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
                                    width: 0.0,
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
                    let rebuilt = rebuilt.with_pattern_origin(target.hatch.pattern_origin());
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
                        // Every split part keeps the pattern phase of the original.
                        let rebuilt = rebuilt.with_pattern_origin(target.hatch.pattern_origin());
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
            HatchEditOption::Associate
            | HatchEditOption::Disassociate
            | HatchEditOption::Annotative
            | HatchEditOption::AddBoundaries
            | HatchEditOption::RemoveBoundaries => unreachable!("handled above"),
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
    pub(super) fn commit_hatch_edits(
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

/// The human half of a fill refusal (the stored variant, never a pattern name).
pub(super) fn fill_kind(fill: &HatchFill) -> &'static str {
    match fill {
        HatchFill::Solid => "a solid fill",
        HatchFill::Pattern(_) => "a pattern fill",
        HatchFill::Gradient(_) => "a gradient fill",
    }
}
