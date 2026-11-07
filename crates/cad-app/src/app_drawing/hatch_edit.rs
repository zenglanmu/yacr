//! `HATCHEDIT` (AutoCAD 2026), the declared v1 typed-option subset.
//!
//! AutoCAD's `HATCHEDIT` opens a dialog and then edits one hatch. This build has
//! no Hatch Edit dialog, so the option is typed as part of the command line
//! (`HATCHEDIT ANGLE <degrees>`, `HATCHEDIT SCALE <factor>`, `HATCHEDIT SOLID`,
//! `HATCHEDIT GRADIENT`) and carried in [`CommandPayload::HatchEdit`]. The
//! current selection is the rest of the input: **exactly one** directly
//! addressable model-space `AcDbHatch`.
//!
//! The edit rebuilds the hatch through the domain constructor
//! (`HatchGeometry::from_import`) from the stored loops, plane, style, seeds,
//! pixel size and (for ANGLE/SCALE) pattern type — the display is never
//! hand-edited. One `update_entity` in one transaction / one undo step preserves
//! the entity id, layer, space, draw order and `AcDbHatch` type key. A no-op is
//! the named `hatchedit.unchanged` outcome with no transaction.
//!
//! Full contract: [`crate::command_id::hatchedit`].

use super::*;

use super::hatch_parameters::default_gradient;
use crate::HatchEditOption;

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
fn unchanged(option: HatchEditOption, entity: EntityId) -> CommandOutcome {
    CommandOutcome {
        objects: Vec::new(),
        changes: None,
        diagnostics: vec![Diagnostic {
            object: Some(ObjectId(entity.0)),
            code: "hatchedit.unchanged".into(),
            message: format!(
                "HATCHEDIT {}: entity {} already stores that value; nothing was written",
                option_label(option),
                entity.0
            ),
        }],
        measurement: None,
        point_report: None,
    }
}

/// Rebuild one hatch with a new fill and pattern type, keeping every other
/// stored field. `from_import` is the domain constructor that derives a fresh
/// display without rejecting an already-stored boundary.
fn rebuild(
    hatch: &HatchGeometry,
    fill: HatchFill,
    pattern_type: HatchPatternType,
) -> CadResult<SemanticGeometry> {
    let (rebuilt, _reasons) = HatchGeometry::from_import(
        hatch.loops().to_vec(),
        hatch.plane(),
        hatch.style(),
        fill,
        hatch.seeds().to_vec(),
        hatch.pixel_size(),
        pattern_type,
    );
    Ok(SemanticGeometry::Hatch(Box::new(rebuilt)))
}

fn option_label(option: HatchEditOption) -> String {
    match option {
        HatchEditOption::Angle(degrees) => format!("ANGLE {degrees}"),
        HatchEditOption::Scale(factor) => format!("SCALE {factor}"),
        HatchEditOption::Solid => "SOLID".into(),
        HatchEditOption::Gradient => "GRADIENT".into(),
    }
}

impl Application {
    /// `HATCHEDIT`: apply one typed v1 option to the one selected hatch.
    ///
    /// Refusals (nothing written, no transaction): a bare payload
    /// (`hatchedit.needs_option`, listing the four implemented forms), any other
    /// payload (`hatchedit.payload`), a selection count other than one
    /// (`hatchedit.selection`), a ref that is not directly addressable
    /// model-space (`hatchedit.not_addressable`), another geometry kind
    /// (`hatchedit.not_hatch`), ANGLE/SCALE on a Solid/Gradient fill
    /// (`hatchedit.not_pattern`), a non-finite angle (`hatchedit.angle`) and a
    /// non-finite/non-positive scale (`hatchedit.scale`).
    pub(crate) fn hatch_edit(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let option = match command.payload {
            CommandPayload::HatchEdit(option) => option,
            CommandPayload::None => {
                return Err(invalid(
                    "needs_option",
                    "HATCHEDIT needs an option: `HATCHEDIT ANGLE <degrees>`, \
                     `HATCHEDIT SCALE <factor>`, `HATCHEDIT SOLID` or `HATCHEDIT GRADIENT`. \
                     Pattern name, Color, Properties, Draw order, Add/Remove boundaries and \
                     associativity are NOT implemented",
                ));
            }
            _ => {
                return Err(invalid(
                    "payload",
                    "HATCHEDIT takes a HatchEdit payload; the selection is the input",
                ));
            }
        };

        let references = session.selection.refs();
        if references.len() != 1 {
            return Err(invalid(
                "selection",
                format!(
                    "HATCHEDIT edits exactly one hatch, got {} selected (the interactive Hatch \
                     Edit dialog is not implemented)",
                    references.len()
                ),
            ));
        }
        let reference = &references[0];
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
                    "HATCHEDIT needs a directly addressable model-space hatch; entity {} is \
                     referenced through a block instance or a sub-element",
                    reference.entity.0
                ),
            ));
        }

        let document_id = command.document;
        let document = self
            .workspace
            .documents
            .get_mut(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        let mut candidate = document.drawing.as_ref().clone();
        let drawing = &mut candidate;
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
                    "HATCHEDIT v1 edits model-space hatches only; entity {} is in another space",
                    reference.entity.0
                ),
            ));
        }
        let SemanticGeometry::Hatch(hatch) = &entity.geometry else {
            return Err(unsupported(
                "not_hatch",
                format!(
                    "HATCHEDIT needs a HATCH selection; entity {} is {}",
                    reference.entity.0,
                    class_name(&entity.geometry)
                ),
            ));
        };

        let entity_id = entity.id;
        let after_geometry = match option {
            HatchEditOption::Angle(degrees) => {
                if !degrees.is_finite() {
                    return Err(invalid(
                        "angle",
                        format!("HATCHEDIT ANGLE needs a finite angle, got {degrees}"),
                    ));
                }
                let HatchFill::Pattern(pattern) = hatch.fill() else {
                    return Err(unsupported(
                        "not_pattern",
                        format!(
                            "HATCHEDIT ANGLE applies to a pattern fill only; entity {} stores {} \
                             (use HATCHEDIT SOLID or HATCHEDIT GRADIENT to replace the fill)",
                            entity_id.0,
                            fill_kind(hatch.fill())
                        ),
                    ));
                };
                let radians = degrees.to_radians();
                if pattern.angle == radians {
                    return Ok(unchanged(option, entity_id));
                }
                let mut changed = pattern.clone();
                changed.angle = radians;
                rebuild(hatch, HatchFill::Pattern(changed), hatch.pattern_type())?
            }
            HatchEditOption::Scale(factor) => {
                if !factor.is_finite() || factor <= 0.0 {
                    return Err(invalid(
                        "scale",
                        format!("HATCHEDIT SCALE needs a finite factor > 0, got {factor}"),
                    ));
                }
                let HatchFill::Pattern(pattern) = hatch.fill() else {
                    return Err(unsupported(
                        "not_pattern",
                        format!(
                            "HATCHEDIT SCALE applies to a pattern fill only; entity {} stores {} \
                             (use HATCHEDIT SOLID or HATCHEDIT GRADIENT to replace the fill)",
                            entity_id.0,
                            fill_kind(hatch.fill())
                        ),
                    ));
                };
                if pattern.scale == factor {
                    return Ok(unchanged(option, entity_id));
                }
                let mut changed = pattern.clone();
                changed.scale = factor;
                rebuild(hatch, HatchFill::Pattern(changed), hatch.pattern_type())?
            }
            HatchEditOption::Solid => {
                if matches!(hatch.fill(), HatchFill::Solid) {
                    return Ok(unchanged(option, entity_id));
                }
                rebuild(hatch, HatchFill::Solid, HatchPatternType::Predefined)?
            }
            HatchEditOption::Gradient => {
                if matches!(hatch.fill(), HatchFill::Gradient(_)) {
                    return Ok(unchanged(option, entity_id));
                }
                rebuild(
                    hatch,
                    HatchFill::Gradient(default_gradient()),
                    HatchPatternType::Predefined,
                )?
            }
        };

        let mut after = entity.clone();
        after.geometry = after_geometry;
        let transaction = next_transaction_id();
        let change_set = {
            let mut staged = drawing.begin_drawing_transaction("hatchedit", transaction)?;
            staged.update_entity(after)?;
            staged.commit()?
        };
        // Read the committed entity back so the undo patch's `after` is the exact
        // stored value, never a re-derivation.
        let stored = drawing.entity(entity_id).cloned().ok_or_else(|| {
            CadError::Invariant("HATCHEDIT: the hatch vanished after commit".into())
        })?;
        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: "hatchedit".to_string(),
                patches: vec![drawing_patch(entity_id, Some(entity), Some(stored))],
                block_patches: Vec::new(),
                merge_key: None,
            },
        )?;

        Ok(CommandOutcome {
            objects: vec![ObjectId(entity_id.0)],
            changes: Some(Arc::new(change_set)),
            diagnostics: vec![Diagnostic {
                object: Some(ObjectId(entity_id.0)),
                code: "hatchedit.applied".into(),
                message: format!(
                    "HATCHEDIT {}: updated hatch entity {} in one transaction",
                    option_label(option),
                    entity_id.0
                ),
            }],
            measurement: None,
            point_report: None,
        })
    }
}

/// The human half of a fill refusal (the stored variant, never a pattern name).
fn fill_kind(fill: &HatchFill) -> &'static str {
    match fill {
        HatchFill::Solid => "a solid fill",
        HatchFill::Pattern(_) => "a pattern fill",
        HatchFill::Gradient(_) => "a gradient fill",
    }
}
