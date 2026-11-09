//! `DIMSPACE` — re-space selected **linear/aligned or angular** dimensions at an
//! equal row pitch (AutoCAD 2026).
//!
//! ## Input and spacing rule
//!
//! AutoCAD's `DIMSPACE` prompts "Select base dimension", "Select dimensions to
//! space", then "Enter value or [Auto] <Auto>". This build implements the
//! **selection-based** form (the first selected dimension is the base, the rest
//! are targets in **pick order**) plus the two spacing rules as a typed payload:
//!
//! * [`crate::CommandPayload::None`] or [`crate::CommandPayload::DimSpace`] with
//!   `None` is **Auto**: the row pitch is **twice the base dimension's style
//!   text height** (`DimensionGeometry::style().text_height`, i.e. the resolved
//!   `DIMTXT` snapshot). It is **not** the base measurement. A base whose style
//!   text height is not finite/positive is refused by name
//!   (`dimspace.style_height`); no value is invented.
//! * [`crate::CommandPayload::DimSpace`] with `Some(0.0)` **aligns** the
//!   targets: every target is placed on the base dimension line (linear) or at
//!   the base arc radius (angular).
//! * [`crate::CommandPayload::DimSpace`] with `Some(distance > 0)` is an
//!   **explicit** spacing. A negative/non-finite value is `dimspace.bad_spacing`.
//!
//! Only linear/aligned dimensions (one family) or angular dimensions (the other
//! family) can be spaced together; mixing the families, or selecting a
//! radial/ordinate dimension, is `dimspace.kind`. Angular targets must be
//! **concentric** with the base (same vertex) or the command refuses by name
//! (`dimspace.not_concentric`).
//!
//! The interactive prompts themselves are **NOT IMPLEMENTED**. The spacing
//! payload is the command-line input channel; the current selection is the whole
//! object input, exactly like `MATCHPROP`/`JOIN`. Associative reflow is **NOT
//! IMPLEMENTED** (no dimension is re-associated with its measured geometry), and
//! the interactive live preview is absent.
//!
//! ## Construction
//!
//! ### Linear/aligned family
//!
//! The base's measured geometry and dimension line define the stacking normal
//! exactly as `DIMBASELINE` does: `n̂ = ±rot90(base.rotation)`, signed **away from
//! the base's measured geometry** (`dot(base.location − midpoint(base.first,
//! base.second), n̂) > 0`). A zero dot product leaves the side **undecidable**, and
//! the command refuses (`dimspace.undecidable_normal`) instead of guessing. For
//! an **aligned** base the rotation is derived from its measured segment
//! (`atan2(second − first)`, the same direction `DimensionGeometry::synthesized`
//! uses); a degenerate segment keeps the fixture's `+X` rotation.
//!
//! Target `i` (`i = 1..=N` in pick order) keeps its **own** `first`, `second`,
//! kind (rotated `Linear { rotation }` or `Aligned`), rotation/style snapshot and
//! text override; only its dimension line moves to
//! `base.location + n̂ · (i · spacing)` (or exactly `base.location` when aligning),
//! and the whole entity is re-synthesized through the shared
//! [`DimensionGeometry::synthesized`] (so the measurement, the formatted text and
//! the display are recomputed — there is no second geometry path). The base
//! itself is never rewritten.
//!
//! ### Angular family
//!
//! The base vertex is the explicit `Angular3Pt` vertex, or the intersection of
//! the two infinite lines for `Angular2Ln`. The base radius is
//! `|arc_point − vertex|`; the radial direction is `normalize(arc_point − vertex)`.
//! Target `i` keeps its own definition points and arc sector, and only its
//! `arc_point` moves to `vertex + direction · (base_radius + i · spacing)` (or to
//! `base_radius` when aligning). Concentricity is checked before anything is
//! staged.
//!
//! ## Refusals (all before any write, all named)
//!
//! | code | meaning |
//! | --- | --- |
//! | `dimspace.payload` | a payload that is not `None`/`DimSpace(Option<f64>)` |
//! | `dimspace.selection` | fewer than two selected objects |
//! | `dimspace.not_addressable` | another document's ref, a block-instance member or a sub-element |
//! | `dimspace.target_missing` | the referenced entity does not exist |
//! | `dimspace.space` | an entity is not in model space (paper-space layout is not addressable) |
//! | `dimspace.not_dimension` | a selected object is not a `DIMENSION` entity |
//! | `dimspace.kind` | a radial/ordinate dimension, mixed linear/angular families, or a degenerate/parallel angular base |
//! | `dimspace.style_height` | Auto cannot resolve a finite positive base style text height |
//! | `dimspace.not_concentric` | an angular target's vertex differs from the base's |
//! | `dimspace.bad_spacing` | the explicit spacing is non-finite/negative |
//! | `dimspace.undecidable_normal` | the base dimension line lies on its measured geometry |
//!
//! ## Write and no-op
//!
//! Every changed target is staged with one `update_entity` in **one transaction /
//! one undo step**; a target already sitting on its row is not staged. When no
//! target moves (including an all-rows-already-identity run), the command opens
//! **no** transaction, records no history and reports `dimspace.unchanged`
//! (never a silent empty success). Success reports `dimspace.applied` with the
//! number of moved dimensions. Work-only.

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

/// The resolved base/target values of one selected dimension.
enum DimensionBase {
    /// A rotated `Linear { rotation }` or `Aligned` dimension.
    Linear {
        /// Group 13: the first extension-line origin.
        first: Point3,
        /// Group 14: the second extension-line origin.
        second: Point3,
        /// Group 10: the dimension-line location.
        location: Point3,
        /// Group 50 (or the aligned measured direction), radians about `+Z`.
        rotation: f64,
        /// The resolved style text height (`DIMTXT` snapshot).
        style_height: f64,
    },
    /// An angular dimension with an explicit or intersected vertex.
    Angular {
        vertex: Point3,
        /// Unit radial direction from the vertex through `arc_point`.
        direction: Point3,
        /// `|arc_point − vertex|`.
        radius: f64,
        /// Group 10/16: the dimension-arc location (kept for the no-op check).
        arc_point: Point3,
        /// The resolved style text height (`DIMTXT` snapshot).
        style_height: f64,
    },
}

impl DimensionBase {
    fn family(&self) -> u8 {
        match self {
            DimensionBase::Linear { .. } => 0,
            DimensionBase::Angular { .. } => 1,
        }
    }

    fn style_height(&self) -> f64 {
        match self {
            DimensionBase::Linear { style_height, .. }
            | DimensionBase::Angular { style_height, .. } => *style_height,
        }
    }
}

/// One target that will move, with its before/after entity captured for the
/// undo patch.
struct SpaceTarget {
    before: DbEntity,
    after: DbEntity,
}

impl Application {
    /// `DIMSPACE`: re-space the selected linear/aligned or angular dimensions
    /// from the first selected one, aligning them or using an explicit/Auto
    /// spacing.
    ///
    /// The selection is the object input; [`CommandPayload::DimSpace`] supplies
    /// the spacing rule (`None` = Auto). Every shape requirement is checked
    /// before the drawing transaction starts; a refusal changes no revision and
    /// records no history.
    pub(crate) fn dim_space(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let requested_spacing = match &command.payload {
            CommandPayload::None => None,
            CommandPayload::DimSpace(spacing) => *spacing,
            _ => {
                return Err(invalid(
                    "payload",
                    "DIMSPACE takes no payload (Auto) or DimSpace(Option<f64>); the current \
                     selection is the object input",
                ))
            }
        };
        let references = session.selection.refs();
        if references.len() < 2 {
            return Err(invalid(
                "selection",
                format!(
                    "DIMSPACE needs at least two selected dimensions, got {}",
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

        // Resolve every selected object before staging anything: one wrong
        // object refuses the whole command (no partial write).
        let mut bases: Vec<(DbEntity, DimensionBase)> = Vec::with_capacity(references.len());
        for reference in references {
            bases.push(resolve_dimension(drawing, command, reference)?);
        }
        // Mixing the linear and angular families is not a spacing operation;
        // AutoCAD spaces only dimensions of the same type together.
        let family = bases[0].1.family();
        if bases.iter().any(|(_, base)| base.family() != family) {
            return Err(unsupported(
                "kind",
                "every selected dimension must be in the same family (linear/aligned or \
                 angular); mixing families cannot be spaced together",
            ));
        }
        let base = &bases[0].1;

        // Auto = twice the base dimension's style text height (DIMTXT), never
        // the measurement. An explicit 0 aligns; any other value must be finite
        // and non-negative.
        let spacing = match requested_spacing {
            Some(distance) => {
                if !distance.is_finite() || distance < 0.0 {
                    return Err(invalid(
                        "bad_spacing",
                        format!(
                            "the explicit spacing must be finite and non-negative (0 aligns), \
                             got {distance}"
                        ),
                    ));
                }
                distance
            }
            None => {
                let height = base.style_height();
                if !height.is_finite() || height <= 0.0 {
                    return Err(invalid(
                        "style_height",
                        format!(
                            "Auto spacing needs the base dimension's style text height \
                             (DIMTXT) to be finite and positive, got {height}"
                        ),
                    ));
                }
                2.0 * height
            }
        };
        let align = spacing <= MIN_DIMSPACE_EXTENT;

        let mut targets: Vec<SpaceTarget> = Vec::with_capacity(bases.len() - 1);
        match base {
            DimensionBase::Linear {
                location, rotation, ..
            } => {
                let away = if align {
                    None
                } else {
                    Some(stacking_normal(*location, *rotation, &bases)?)
                };
                for (index, (entity, target)) in bases.iter().enumerate().skip(1) {
                    let DimensionBase::Linear {
                        first,
                        second,
                        location: target_location,
                        ..
                    } = target
                    else {
                        return Err(CadError::Invariant(
                            "dimspace: the family check admitted a non-linear target".into(),
                        ));
                    };
                    let target_line = match away {
                        Some(away) => Point3 {
                            x: location.x + away.x * index as f64 * spacing,
                            y: location.y + away.y * index as f64 * spacing,
                            z: location.z + away.z * index as f64 * spacing,
                        },
                        None => *location,
                    };
                    if points_equal(target_line, *target_location) {
                        continue;
                    }
                    let dimension = expect_dimension(entity)?;
                    let definition = DimensionDefinition {
                        // The target keeps its own subtype: a rotated `Linear`
                        // keeps its rotation and an `Aligned` dimension stays
                        // aligned (only the dimension line moves).
                        kind: dimension.kind(),
                        points: DimensionPoints::Linear {
                            first: *first,
                            second: *second,
                            dimension_line: target_line,
                        },
                        // Group 11/53 stay at the derive-from-the-line sentinel;
                        // the target's own text override, style snapshot and
                        // rotation are the only target-owned values carried
                        // across.
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
                    let next = DimensionGeometry::synthesized(
                        definition,
                        dimension.style_name().to_string(),
                        dimension.style().clone(),
                        dimension.text_override().map(str::to_string),
                    )?;
                    let mut after = entity.clone();
                    after.geometry = SemanticGeometry::Dimension(Box::new(next));
                    targets.push(SpaceTarget {
                        before: entity.clone(),
                        after,
                    });
                }
            }
            DimensionBase::Angular {
                vertex: base_vertex,
                radius: base_radius,
                ..
            } => {
                for (index, (entity, target)) in bases.iter().enumerate().skip(1) {
                    let DimensionBase::Angular {
                        vertex,
                        direction,
                        arc_point,
                        ..
                    } = target
                    else {
                        return Err(CadError::Invariant(
                            "dimspace: the family check admitted a non-angular target".into(),
                        ));
                    };
                    if !points_equal_xy(*vertex, *base_vertex) {
                        return Err(unsupported(
                            "not_concentric",
                            format!(
                                "entity {} is not concentric with the base angular dimension's \
                                 vertex",
                                entity.id.0
                            ),
                        ));
                    }
                    let radius = if align {
                        *base_radius
                    } else {
                        *base_radius + index as f64 * spacing
                    };
                    let new_arc = Point3 {
                        x: vertex.x + direction.x * radius,
                        y: vertex.y + direction.y * radius,
                        z: arc_point.z,
                    };
                    if points_equal(new_arc, *arc_point) {
                        continue;
                    }
                    let dimension = expect_dimension(entity)?;
                    let mut definition = dimension.definition().clone();
                    match &mut definition.points {
                        DimensionPoints::Angular2Ln { arc_point, .. }
                        | DimensionPoints::Angular3Pt { arc_point, .. } => *arc_point = new_arc,
                        other => {
                            return Err(CadError::Invariant(format!(
                                "dimspace: an angular dimension carried non-angular points \
                                 {other:?}"
                            )))
                        }
                    }
                    let next = DimensionGeometry::synthesized(
                        definition,
                        dimension.style_name().to_string(),
                        dimension.style().clone(),
                        dimension.text_override().map(str::to_string),
                    )?;
                    let mut after = entity.clone();
                    after.geometry = SemanticGeometry::Dimension(Box::new(next));
                    targets.push(SpaceTarget {
                        before: entity.clone(),
                        after,
                    });
                }
            }
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
                layer_patches: Vec::new(),
                layout_patches: Vec::new(),
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
                    "DIMSPACE: spaced {count} dimension(s) at {}",
                    match requested_spacing {
                        Some(_) if align => "the explicit 0 (aligned) spacing".to_string(),
                        Some(_) => format!("the explicit {spacing} spacing"),
                        None => format!("the Auto (2 x base style text height) {spacing} spacing"),
                    }
                ),
            }],
            measurement: None,
            point_report: None,
        })
    }
}

/// The stacking normal for a linear base: `±rot90(rotation)`, signed away from
/// the base's measured geometry. A base line on its measured points leaves the
/// side undecidable — the command never guesses.
fn stacking_normal(
    location: Point3,
    rotation: f64,
    bases: &[(DbEntity, DimensionBase)],
) -> CadResult<Point3> {
    let (first, second) = match &bases[0].1 {
        DimensionBase::Linear { first, second, .. } => (*first, *second),
        DimensionBase::Angular { .. } => {
            return Err(CadError::Invariant(
                "dimspace: a linear base resolved without linear points".into(),
            ))
        }
    };
    let direction = Point3 {
        x: rotation.cos(),
        y: rotation.sin(),
        z: 0.0,
    };
    let normal = Point3 {
        x: -direction.y,
        y: direction.x,
        z: 0.0,
    };
    let measured_mid = Point3 {
        x: (first.x + second.x) * 0.5,
        y: (first.y + second.y) * 0.5,
        z: (first.z + second.z) * 0.5,
    };
    let side = (location.x - measured_mid.x) * normal.x
        + (location.y - measured_mid.y) * normal.y
        + (location.z - measured_mid.z) * normal.z;
    if !side.is_finite() || side.abs() <= MIN_DIMSPACE_EXTENT {
        return Err(invalid(
            "undecidable_normal",
            "the base dimension line lies on its measured geometry, so the stacking side \
             cannot be decided",
        ));
    }
    Ok(if side > 0.0 {
        normal
    } else {
        Point3 {
            x: -normal.x,
            y: -normal.y,
            z: -normal.z,
        }
    })
}

/// Borrow a directly addressable `DIMENSION` payload.
fn expect_dimension(entity: &DbEntity) -> CadResult<&DimensionGeometry> {
    match &entity.geometry {
        SemanticGeometry::Dimension(dimension) => Ok(dimension),
        other => Err(CadError::Invariant(format!(
            "dimspace: the resolved entity became {other:?}"
        ))),
    }
}

/// Resolve one selected ref to a model-space linear/angular `DIMENSION` and its
/// base values, refusing every other shape by name.
fn resolve_dimension(
    drawing: &DrawingDatabase,
    command: &Command,
    reference: &SelectionRef,
) -> CadResult<(DbEntity, DimensionBase)> {
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
                "entity {} is not in model space; paper-space dimension layout is not \
                 addressable by this build, so DIMSPACE is model-space only",
                reference.entity.0
            ),
        ));
    }
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
    let style_height = dimension.style().text_height;
    let base = match dimension.kind() {
        DimensionKind::Linear { rotation } => {
            let (first, second, location) = linear_points(dimension, reference.entity)?;
            DimensionBase::Linear {
                first,
                second,
                location,
                rotation,
                style_height,
            }
        }
        DimensionKind::Aligned => {
            let (first, second, location) = linear_points(dimension, reference.entity)?;
            // The base direction: an `Aligned` dimension derives it from the
            // measured segment (the same direction the synthesis uses). A
            // degenerate segment has no direction, so `+X` is used.
            let dx = second.x - first.x;
            let dy = second.y - first.y;
            let rotation = if dx.hypot(dy) <= MIN_DIMSPACE_EXTENT {
                0.0
            } else {
                dy.atan2(dx)
            };
            DimensionBase::Linear {
                first,
                second,
                location,
                rotation,
                style_height,
            }
        }
        DimensionKind::Angular2Ln => {
            let DimensionPoints::Angular2Ln {
                line1_start,
                line1_end,
                line2_start,
                line2_end,
                arc_point,
            } = &dimension.definition().points
            else {
                return Err(CadError::Invariant(format!(
                    "dimspace: entity {} is Angular2Ln without two-line points",
                    reference.entity.0
                )));
            };
            let d1 = Point3 {
                x: line1_end.x - line1_start.x,
                y: line1_end.y - line1_start.y,
                z: 0.0,
            };
            let d2 = Point3 {
                x: line2_end.x - line2_start.x,
                y: line2_end.y - line2_start.y,
                z: 0.0,
            };
            if d1.x.hypot(d1.y) <= MIN_DIMSPACE_EXTENT || d2.x.hypot(d2.y) <= MIN_DIMSPACE_EXTENT {
                return Err(unsupported(
                    "kind",
                    format!(
                        "entity {} is an angular dimension with a degenerate line; no vertex",
                        reference.entity.0
                    ),
                ));
            }
            let vertex =
                line_intersection_xy(*line1_start, d1, *line2_start, d2).ok_or_else(|| {
                    unsupported(
                        "kind",
                        format!(
                            "entity {} is an angular dimension between parallel/collinear lines; \
                         no vertex to space around",
                            reference.entity.0
                        ),
                    )
                })?;
            angular_base(vertex, *arc_point, style_height, reference.entity)?
        }
        DimensionKind::Angular3Pt => {
            let DimensionPoints::Angular3Pt {
                vertex, arc_point, ..
            } = &dimension.definition().points
            else {
                return Err(CadError::Invariant(format!(
                    "dimspace: entity {} is Angular3Pt without three-point points",
                    reference.entity.0
                )));
            };
            angular_base(*vertex, *arc_point, style_height, reference.entity)?
        }
        other => {
            return Err(unsupported(
                "kind",
                format!(
                    "entity {} is {}, not a linear/aligned or angular dimension; DIMSPACE \
                     spaces those families only",
                    reference.entity.0,
                    dimension_kind_name(other)
                ),
            ))
        }
    };
    Ok((entity, base))
}

fn linear_points(
    dimension: &DimensionGeometry,
    entity: EntityId,
) -> CadResult<(Point3, Point3, Point3)> {
    let DimensionPoints::Linear {
        first,
        second,
        dimension_line,
    } = &dimension.definition().points
    else {
        return Err(CadError::Invariant(format!(
            "dimspace: entity {} is a linear-family dimension without linear points",
            entity.0
        )));
    };
    Ok((*first, *second, *dimension_line))
}

fn angular_base(
    vertex: Point3,
    arc_point: Point3,
    style_height: f64,
    entity: EntityId,
) -> CadResult<DimensionBase> {
    let dx = arc_point.x - vertex.x;
    let dy = arc_point.y - vertex.y;
    let radius = dx.hypot(dy);
    if !radius.is_finite() || radius <= MIN_DIMSPACE_EXTENT {
        return Err(unsupported(
            "kind",
            format!(
                "entity {} has an angular arc point at its vertex; there is no arc radius to \
                 space",
                entity.0
            ),
        ));
    }
    Ok(DimensionBase::Angular {
        vertex,
        direction: Point3 {
            x: dx / radius,
            y: dy / radius,
            z: 0.0,
        },
        radius,
        arc_point,
        style_height,
    })
}

/// Intersection of two infinite XY lines through `o1`/`o2` with directions
/// `d1`/`d2`; `None` for parallel/collinear directions.
fn line_intersection_xy(o1: Point3, d1: Point3, o2: Point3, d2: Point3) -> Option<Point3> {
    let cross = d1.x * d2.y - d1.y * d2.x;
    let scale = d1.x.hypot(d1.y) * d2.x.hypot(d2.y);
    if !cross.is_finite() || scale <= MIN_DIMSPACE_EXTENT || cross.abs() <= scale * 1e-12 {
        return None;
    }
    let offset_x = o2.x - o1.x;
    let offset_y = o2.y - o1.y;
    let t = (offset_x * d2.y - offset_y * d2.x) / cross;
    Some(Point3 {
        x: o1.x + d1.x * t,
        y: o1.y + d1.y * t,
        z: o1.z,
    })
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

/// Whether two points share the same XY position within the extent tolerance.
fn points_equal_xy(a: Point3, b: Point3) -> bool {
    (a.x - b.x).abs() <= MIN_DIMSPACE_EXTENT && (a.y - b.y).abs() <= MIN_DIMSPACE_EXTENT
}
