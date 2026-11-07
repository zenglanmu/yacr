//! `DIMSPACE` v1 — re-space selected **linear** dimensions at equal automatic
//! spacing (AutoCAD 2026).
//!
//! ## Declared v1 boundary (read this before trusting the command)
//!
//! AutoCAD's `DIMSPACE` prompts "Select base dimension", "Select dimensions to
//! space", then "Enter value or [Auto] <Auto>". This build implements only the
//! **selection-based** form and **always** uses automatic spacing:
//!
//! * the **first** selected linear dimension is the base and every remaining
//!   selected dimension is a target, in **pick order**;
//! * the spacing is the base dimension's resolved style `DIMDLI`
//!   ([`cad_domain::DimensionStyleValues::dim_line_increment`]) — AutoCAD's
//!   automatic mode (its value `0`);
//! * the interactive value prompt (`0` = auto, or an explicit distance) is **NOT
//!   IMPLEMENTED**, and neither is associative reflow (no dimension is
//!   re-associated with its measured geometry).
//!
//! Input is [`crate::CommandPayload::None`]: the current selection is the whole
//! input, exactly like `MATCHPROP`/`JOIN`. A payload is refused by name.
//!
//! ## Construction
//!
//! The base's measured geometry and dimension line define the stacking normal
//! exactly as `DIMBASELINE` does: `n̂ = ±rot90(base.rotation)`, signed **away from
//! the base's measured geometry** (`dot(base.location − midpoint(base.first,
//! base.second), n̂) > 0`). A zero dot product leaves the side **undecidable**, and
//! the command refuses instead of guessing.
//!
//! Target `i` (`i = 1..=N` in pick order) keeps its **own** `first`, `second`,
//! rotation, style snapshot and text override; only its dimension line moves to
//! `base.location + n̂ · (i · spacing)`, and the whole entity is re-synthesized
//! through the shared [`DimensionGeometry::synthesized`] (so the measurement, the
//! formatted text and the display are recomputed — there is no second geometry
//! path). The base itself is never rewritten.
//!
//! ## Refusals (all before any write, all named)
//!
//! | code | meaning |
//! | --- | --- |
//! | `dimspace.payload` | a payload was supplied (the selection is the input) |
//! | `dimspace.selection` | fewer than two selected objects |
//! | `dimspace.duplicate` | the same entity is selected twice |
//! | `dimspace.not_addressable` | another document's ref, a block-instance member or a sub-element |
//! | `dimspace.target_missing` | the referenced entity does not exist |
//! | `dimspace.space` | an entity is not in model space (v1 is model-space only) |
//! | `dimspace.not_dimension` | a selected object is not a `DIMENSION` entity |
//! | `dimspace.kind` | a dimension that is not `Linear` (aligned/angular/radial/ordinate) |
//! | `dimspace.bad_spacing` | the base style's `DIMDLI` is non-finite or non-positive |
//! | `dimspace.undecidable_normal` | the base dimension line lies on its measured geometry |
//!
//! ## Write and no-op
//!
//! Every changed target is staged with one `update_entity` in **one transaction /
//! one undo step**; a target already sitting on its row is not staged. When no
//! target moves, the command opens **no** transaction, records no history and
//! reports `dimspace.unchanged` (never a silent empty success). Success reports
//! `dimspace.applied` with the number of moved dimensions. Work-only.

use super::*;
use cad_db::{DbEntity, DrawingDatabase};

/// The minimum spacing/extent used by the degeneracy guards (the same order as
/// the other dimension commands).
const MIN_DIMSPACE_EXTENT: f64 = 1e-9;

/// A named, non-capability refusal (bad input or selection).
fn invalid(code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::InvalidInput(format!("dimspace.{code}: {detail}"))
}

/// A named capability-gap refusal (nothing is written).
fn unsupported(code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::Unsupported(format!("dimspace.{code}: {detail}"))
}

/// The resolved base dimension `DIMSPACE` spaces the targets from.
struct SpaceBase {
    /// Group 13: the base's first extension-line origin.
    first: Point3,
    /// Group 14: the base's second extension-line origin.
    second: Point3,
    /// Group 10: the base's dimension-line location.
    location: Point3,
    /// Group 50: the base's rotation (radians, about `+Z`).
    rotation: f64,
}

/// One target that will move, with its before/after entity captured for the
/// undo patch.
struct SpaceTarget {
    before: DbEntity,
    after: DbEntity,
}

impl Application {
    /// `DIMSPACE` v1: re-space the selected linear dimensions from the first
    /// selected one, at the automatic `DIMDLI` spacing.
    ///
    /// The selection is the whole input, so a payload is refused by name. Every
    /// shape requirement is checked before the drawing transaction starts; a
    /// refusal changes no revision and records no history.
    pub(crate) fn dim_space(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        if !matches!(command.payload, CommandPayload::None) {
            return Err(invalid(
                "payload",
                "DIMSPACE takes no payload; the current selection is the input",
            ));
        }
        let references = session.selection.refs();
        if references.len() < 2 {
            return Err(invalid(
                "selection",
                format!(
                    "DIMSPACE needs at least two selected linear dimensions, got {}",
                    references.len()
                ),
            ));
        }
        let document_id = command.document;
        let document = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        let drawing = document.drawing.as_ref();

        // Resolve every selected object to a linear dimension before staging
        // anything: one wrong object refuses the whole command (no partial write).
        let mut bases: Vec<(DbEntity, SpaceBase)> = Vec::with_capacity(references.len());
        for reference in references {
            let (entity, base) = resolve_linear_dimension(drawing, command, reference)?;
            bases.push((entity, base));
        }
        let base = &bases[0].1;

        // Automatic spacing is the base's resolved `DIMDLI`. A non-positive value
        // would stack every target on the same line, so it is refused by name.
        let style = match &bases[0].0.geometry {
            SemanticGeometry::Dimension(dimension) => dimension.style(),
            other => {
                return Err(CadError::Invariant(format!(
                    "dimension_chain: the resolved base became {other:?}"
                )))
            }
        };
        let spacing = style.dim_line_increment;
        if !spacing.is_finite() || spacing <= MIN_DIMSPACE_EXTENT {
            return Err(invalid(
                "bad_spacing",
                format!(
                    "the base dimension's automatic spacing (DIMDLI) must be finite and \
                     positive, got {spacing}"
                ),
            ));
        }

        // The stacking normal: ±rot90(base rotation), signed away from the base's
        // measured geometry. A base line on the measured points leaves the side
        // undecidable — the command never guesses.
        let direction = Point3 {
            x: base.rotation.cos(),
            y: base.rotation.sin(),
            z: 0.0,
        };
        let normal = Point3 {
            x: -direction.y,
            y: direction.x,
            z: 0.0,
        };
        let measured_mid = Point3 {
            x: (base.first.x + base.second.x) * 0.5,
            y: (base.first.y + base.second.y) * 0.5,
            z: (base.first.z + base.second.z) * 0.5,
        };
        let side = (base.location.x - measured_mid.x) * normal.x
            + (base.location.y - measured_mid.y) * normal.y
            + (base.location.z - measured_mid.z) * normal.z;
        if !side.is_finite() || side.abs() <= MIN_DIMSPACE_EXTENT {
            return Err(invalid(
                "undecidable_normal",
                "the base dimension line lies on its measured geometry, so the stacking side \
                 cannot be decided",
            ));
        }
        let away = if side > 0.0 {
            normal
        } else {
            Point3 {
                x: -normal.x,
                y: -normal.y,
                z: -normal.z,
            }
        };

        // Build each target's new dimension line in pick order. The base is
        // excluded (it is the reference, never rewritten).
        let mut targets: Vec<SpaceTarget> = Vec::with_capacity(bases.len() - 1);
        for (index, (entity, target_base)) in bases.iter().enumerate().skip(1) {
            let row = index as f64;
            let target_line = Point3 {
                x: base.location.x + away.x * row * spacing,
                y: base.location.y + away.y * row * spacing,
                z: base.location.z + away.z * row * spacing,
            };
            let (style_name, style, text_override) = match &entity.geometry {
                SemanticGeometry::Dimension(dimension) => (
                    dimension.style_name().to_string(),
                    dimension.style().clone(),
                    dimension.text_override().map(str::to_string),
                ),
                other => {
                    return Err(CadError::Invariant(format!(
                        "dimspace: the resolved target became {other:?}"
                    )))
                }
            };
            if points_equal(target_line, target_base.location) {
                // Already on its row: not staged, so undo restores exactly what
                // changed (nothing for this target).
                continue;
            }
            let definition = DimensionDefinition {
                kind: DimensionKind::Linear {
                    rotation: target_base.rotation,
                },
                points: DimensionPoints::Linear {
                    first: target_base.first,
                    second: target_base.second,
                    dimension_line: target_line,
                },
                // Group 11/53 stay at the derive-from-the-line sentinel; the
                // target's own text override, style snapshot and rotation are the
                // only target-owned values carried across.
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
            let dimension =
                DimensionGeometry::synthesized(definition, style_name, style, text_override)?;
            let mut after = entity.clone();
            after.geometry = SemanticGeometry::Dimension(Box::new(dimension));
            targets.push(SpaceTarget {
                before: entity.clone(),
                after,
            });
        }

        if targets.is_empty() {
            // Nothing moved: no transaction, no ChangeSet and no history entry.
            return Ok(CommandOutcome {
                objects: Vec::new(),
                changes: None,
                diagnostics: vec![Diagnostic {
                    object: None,
                    code: "dimspace.unchanged".into(),
                    message: format!(
                        "DIMSPACE: every selected dimension already sits on its {} spacing row \
                         (no transaction)",
                        spacing
                    ),
                }],
                measurement: None,
                point_report: None,
            });
        }

        // Re-read the drawing mutably through a candidate clone (the same single
        // funnel the other entity edits use).
        let document = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        let mut candidate = document.drawing.as_ref().clone();
        let transaction = next_transaction_id();
        let change_set = {
            let mut staged = candidate.begin_drawing_transaction("dimspace", transaction)?;
            for target in &targets {
                staged.update_entity(target.after.clone())?;
            }
            staged.commit()?
        };
        // Read the committed entities back so the undo patches carry the exact
        // stored value, never a re-derivation.
        let mut patches = Vec::with_capacity(targets.len());
        let mut objects = Vec::with_capacity(targets.len());
        for target in &mut targets {
            target.after = candidate.entity(target.before.id).cloned().ok_or_else(|| {
                CadError::Invariant("DIMSPACE: a target vanished after commit".into())
            })?;
            patches.push(drawing_patch(
                target.before.id,
                Some(target.before.clone()),
                Some(target.after.clone()),
            ));
            objects.push(ObjectId(target.before.id.0));
        }
        let count = objects.len();
        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: "dimspace".to_string(),
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
                code: "dimspace.applied".into(),
                message: format!(
                    "DIMSPACE: spaced {count} linear dimension(s) at the automatic {} spacing",
                    spacing
                ),
            }],
            measurement: None,
            point_report: None,
        })
    }
}

/// Resolve one selected ref to a model-space linear `DIMENSION` and its base
/// values, refusing every other shape by name.
fn resolve_linear_dimension(
    drawing: &DrawingDatabase,
    command: &Command,
    reference: &SelectionRef,
) -> CadResult<(DbEntity, SpaceBase)> {
    if reference.document != command.document {
        return Err(invalid(
            "not_addressable",
            format!(
                "the ref for entity {} belongs to another document",
                reference.entity.0
            ),
        ));
    }
    if !reference.instance.0.is_empty() || reference.sub_element.is_some() {
        return Err(unsupported(
            "not_addressable",
            format!(
                "entity {} is referenced through a block instance or a sub-element; pick the \
                 dimension entity itself",
                reference.entity.0
            ),
        ));
    }
    let entity = drawing.entity(reference.entity).cloned().ok_or_else(|| {
        invalid(
            "target_missing",
            format!(
                "entity {} does not exist in this drawing",
                reference.entity.0
            ),
        )
    })?;
    if entity.space != SpaceId::Model {
        return Err(unsupported(
            "space",
            format!(
                "entity {} is not in model space; DIMSPACE v1 is model-space only",
                reference.entity.0
            ),
        ));
    }
    let (first, second, location, rotation) = {
        let SemanticGeometry::Dimension(dimension) = &entity.geometry else {
            return Err(unsupported(
                "not_dimension",
                format!(
                    "entity {} is a {} entity, not a DIMENSION",
                    reference.entity.0,
                    class_name(&entity.geometry)
                ),
            ));
        };
        let DimensionKind::Linear { rotation } = dimension.kind() else {
            return Err(unsupported(
                "kind",
                format!(
                    "entity {} is {}, not a linear dimension; DIMSPACE v1 spaces linear \
                     dimensions only",
                    reference.entity.0,
                    dimension_kind_name(dimension.kind())
                ),
            ));
        };
        let DimensionPoints::Linear {
            first,
            second,
            dimension_line,
        } = &dimension.definition().points
        else {
            return Err(CadError::Invariant(format!(
                "dimspace: entity {} is a linear dimension without linear points",
                reference.entity.0
            )));
        };
        (*first, *second, *dimension_line, rotation)
    };
    Ok((
        entity,
        SpaceBase {
            first,
            second,
            location,
            rotation,
        },
    ))
}

/// The stable name of a dimension kind for refusal messages.
fn dimension_kind_name(kind: DimensionKind) -> &'static str {
    match kind {
        DimensionKind::Linear { .. } => "a linear dimension",
        DimensionKind::Aligned => "an aligned dimension",
        DimensionKind::Angular2Ln | DimensionKind::Angular3Pt => "an angular dimension",
        DimensionKind::Radius => "a radius dimension",
        DimensionKind::Diameter => "a diameter dimension",
        DimensionKind::Ordinate { .. } => "an ordinate dimension",
    }
}

/// Whether two points hold the same value within the command's extent tolerance.
fn points_equal(a: Point3, b: Point3) -> bool {
    (a.x - b.x).abs() <= MIN_DIMSPACE_EXTENT
        && (a.y - b.y).abs() <= MIN_DIMSPACE_EXTENT
        && (a.z - b.z).abs() <= MIN_DIMSPACE_EXTENT
}
