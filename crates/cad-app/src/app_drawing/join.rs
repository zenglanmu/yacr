//! JOIN edit command: merge collinear LINEs / concentric ARCs into one entity.
//!
//! AutoCAD's `JOIN` welds a selection into one object. This build implements the
//! **two honest subsets** the semantic model can express exactly:
//!
//! * **all `Line`** — every picked line must be **collinear** with the longest one
//!   (within [`join_tolerance`]), share its layer and its space, and their spans
//!   along the common direction must **touch or overlap** (no gap, no fold-back).
//!   The result is one LINE spanning the union `min → max` with a **fresh entity
//!   id**; the sources are deleted.
//! * **all `Arc`** — every picked arc must share the reference arc's centre,
//!   normal **direction** and radius (within [`join_tolerance`] / the derived
//!   angular tolerance), share its layer and its space, and their sweeps must be
//!   **contiguous in the same rotational direction**. The result is one ARC whose
//!   sweep is the **summed** sweep; a total above a full turn is refused
//!   (`join.sweep_overflow`) rather than wrapped, and a total of exactly one turn
//!   is stored as a `2π` ARC (this build keeps the arc kind; it is not silently
//!   turned into a `Circle`).
//!
//! Everything else is an **explicit refusal that writes nothing**:
//!
//! | code | meaning |
//! | --- | --- |
//! | `join.needs_two` | fewer than two entities are selected |
//! | `join.mixed_kinds` | the selection mixes LINEs and ARCs |
//! | `join.not_collinear` | a line is not on the common line |
//! | `join.not_contiguous` | a gap, opposite directions, or arcs with a different centre/normal/radius |
//! | `join.layer_mismatch` | the picks are not all on one layer |
//! | `join.space_mismatch` | the picks are not all in one space |
//! | `join.sweep_overflow` | the summed arc sweep exceeds one full turn |
//! | `join.degenerate` | a zero-length LINE or zero-sweep ARC |
//!
//! A non-LINE/non-ARC pick (polyline, circle, spline, text, …) is
//! [`CadError::Unsupported`] — a capability gap, never blamed on the input and
//! never silently dropped from the merge.
//!
//! ## Transaction shape
//!
//! The whole plan is computed **before any write**; on success the sources are
//! deleted and the merged entity inserted in **one transaction with exactly one
//! undo step** (the `explode`/`erase` shape), so an undo restores every source
//! and removes the merged entity.
//!
//! The merged entity keeps the **lowest draw order of its sources**, so the weld
//! replaces the group where it was painted instead of jumping to the top of the
//! drawing. Its layer, space and `type_key` come from the sources/geometry — no
//! style is invented.

use super::helpers::normalize_signed;
use super::*;
use cad_geometry::{dot, sub};

const TAU: f64 = std::f64::consts::TAU;

/// The world tolerance the JOIN predicates use: the computation predicate
/// tolerance, widened to the topology join tolerance (the same shape
/// [`super::trim_curves::cut_tolerance`] and the OFFSET helper use).
pub(super) fn join_tolerance() -> f64 {
    let policy = TolerancePolicy::default();
    policy
        .computation_world
        .max(1e-12)
        .max(policy.topology_world)
}

/// The angular tolerance for two arcs' continuity at one radius.
///
/// Derived from the **world** tolerance at the arc's own circumference
/// (`angle = arc_length / radius`), so one policy number governs both the linear
/// and the angular predicates instead of a second, invented angle epsilon.
fn angle_tolerance(radius: f64) -> f64 {
    let radius = radius.abs().max(join_tolerance());
    (join_tolerance() / radius).max(1e-12)
}

/// A plan-level JOIN refusal: `InvalidInput` carrying its stable code.
fn refusal(code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::InvalidInput(format!("{code}: {detail}"))
}

/// One inclusive span along the common line (world units from the reference
/// origin).
#[derive(Debug, Clone, Copy, PartialEq)]
struct Span {
    start: f64,
    end: f64,
}

/// The merged geometry a JOIN will insert, resolved before any write.
enum Joined {
    Line {
        start: Point3,
        end: Point3,
    },
    Arc {
        center: Point3,
        normal: Point3,
        radius: f64,
        start: f64,
        sweep: f64,
    },
}

impl Joined {
    fn geometry(self) -> SemanticGeometry {
        match self {
            Joined::Line { start, end } => SemanticGeometry::Line { start, end },
            Joined::Arc {
                center,
                normal,
                radius,
                start,
                sweep,
            } => SemanticGeometry::Arc {
                center,
                normal,
                radius,
                start,
                sweep,
            },
        }
    }
}

impl Application {
    /// JOIN the current selection into one LINE or one ARC.
    ///
    /// The selection is the whole input (payload must be `None`). The plan is
    /// resolved before any write, so every refusal leaves the drawing, the id
    /// allocator and history untouched.
    pub(crate) fn join_entities(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        if !matches!(command.payload, CommandPayload::None) {
            return Err(CadError::InvalidInput("JOIN takes no payload".into()));
        }
        let refs = session.selection.refs().to_vec();
        if refs.len() < 2 {
            return Err(refusal(
                "join.needs_two",
                format!(
                    "JOIN needs at least two selected entities, got {}",
                    refs.len()
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

        let (sources, mut diagnostics) =
            Self::resolve_selection(drawing, document_id, &refs, "join", "JOIN", "join", false)?;
        if sources.len() < 2 {
            // Duplicate refs collapse to one entity: that is one object, not two.
            return Err(refusal(
                "join.needs_two",
                "JOIN needs at least two distinct selected entities",
            ));
        }

        let joined = plan_join(&sources)?;

        // One layer and one space for the whole weld: a merge across either would
        // have to pick a winner, which is a silent loss.
        let layer = sources[0].layer;
        let space = sources[0].space.clone();
        for source in &sources[1..] {
            if source.layer != layer {
                return Err(refusal(
                    "join.layer_mismatch",
                    format!(
                        "entity {} is on layer {} but the first pick is on layer {}",
                        source.id.0, source.layer.0, layer.0
                    ),
                ));
            }
            if source.space != space {
                return Err(refusal(
                    "join.space_mismatch",
                    format!(
                        "entity {} is in another space than the first pick; a join never \
                         crosses model/paper space",
                        source.id.0
                    ),
                ));
            }
        }

        // Capture the display state each delete prunes, so undo restores it.
        let display_states: Vec<_> = sources
            .iter()
            .map(|source| drawing.entity_display_state(source.id))
            .collect();

        let geometry = joined.geometry();
        let new_id = drawing.allocate_entity_id();
        let entity = DbEntity {
            object: cad_db::DbObject {
                id: ObjectId(new_id.0),
                type_key: class_name(&geometry).to_string(),
                revision: Revision(0),
                source_handle: None,
            },
            id: new_id,
            layer,
            space,
            geometry,
            // The weld replaces the group in place: it inherits the lowest draw
            // order of its sources instead of jumping above every entity.
            draw_order: sources
                .iter()
                .map(|source| source.draw_order)
                .min()
                .unwrap_or(0),
        };

        let transaction = next_transaction_id();
        let change_set = {
            let mut staged = drawing.begin_drawing_transaction("join entities", transaction)?;
            for source in &sources {
                staged.delete_entity(source.id)?;
            }
            staged.insert_entity(entity.clone())?;
            staged.commit()?
        };

        let mut patches: Vec<DrawingPatch> = sources
            .iter()
            .zip(display_states)
            .map(|(source, state)| drawing_patch_deleted(source.id, source.clone(), state))
            .collect();
        patches.push(drawing_patch(new_id, None, Some(entity.clone())));

        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: "join entities".into(),
                patches,
                merge_key: None,
            },
        )?;

        diagnostics.push(Diagnostic {
            object: Some(ObjectId(new_id.0)),
            code: "drawing.join".into(),
            message: format!(
                "JOIN: {} entities merged into {} {}",
                sources.len(),
                class_name(&entity.geometry),
                new_id.0
            ),
        });

        Ok(CommandOutcome {
            objects: vec![ObjectId(new_id.0)],
            changes: Some(Arc::new(change_set)),
            diagnostics,
            measurement: None,
            point_report: None,
        })
    }
}

/// Resolve the one merged geometry from the sources, or refuse by name.
fn plan_join(sources: &[DbEntity]) -> CadResult<Joined> {
    let lines: Vec<(Point3, Point3)> = sources
        .iter()
        .filter_map(|source| match &source.geometry {
            SemanticGeometry::Line { start, end } => Some((*start, *end)),
            _ => None,
        })
        .collect();
    let arcs: Vec<(Point3, Point3, f64, f64, f64)> = sources
        .iter()
        .filter_map(|source| match &source.geometry {
            SemanticGeometry::Arc {
                center,
                normal,
                radius,
                start,
                sweep,
            } => Some((*center, *normal, *radius, *start, *sweep)),
            _ => None,
        })
        .collect();

    if lines.len() == sources.len() {
        return plan_lines(&lines);
    }
    if arcs.len() == sources.len() {
        return plan_arcs(&arcs);
    }
    let unsupported: Vec<&str> = sources
        .iter()
        .filter(|source| {
            !matches!(
                source.geometry,
                SemanticGeometry::Line { .. } | SemanticGeometry::Arc { .. }
            )
        })
        .map(|source| class_name(&source.geometry))
        .collect();
    if !unsupported.is_empty() {
        return Err(CadError::Unsupported(format!(
            "JOIN merges only LINEs (collinear) and ARCs (concentric); {} is not supported",
            unsupported.join(", ")
        )));
    }
    Err(refusal(
        "join.mixed_kinds",
        "JOIN cannot merge LINEs with ARCs; select one kind",
    ))
}

/// Plan a LINE merge (see the module docs for the rule).
fn plan_lines(lines: &[(Point3, Point3)]) -> CadResult<Joined> {
    let tol = join_tolerance();
    // The longest line defines the common direction; a longer baseline keeps the
    // unit vector and the projections well conditioned.
    let reference = lines
        .iter()
        .max_by(|a, b| {
            let la = cad_geometry::length(sub(a.1, a.0));
            let lb = cad_geometry::length(sub(b.1, b.0));
            la.partial_cmp(&lb).unwrap_or(std::cmp::Ordering::Equal)
        })
        .copied()
        .expect("plan_lines is called with a non-empty selection");
    let direction = sub(reference.1, reference.0);
    let reference_length = cad_geometry::length(direction);
    if !reference_length.is_finite() || reference_length <= tol {
        return Err(refusal(
            "join.degenerate",
            "the longest selected LINE has no length to merge along",
        ));
    }
    let unit = Point3 {
        x: direction.x / reference_length,
        y: direction.y / reference_length,
        z: direction.z / reference_length,
    };
    let base = reference.0;

    let mut spans: Vec<Span> = Vec::with_capacity(lines.len());
    for (index, (start, end)) in lines.iter().enumerate() {
        for point in [start, end] {
            if !is_finite_point(*point) {
                return Err(refusal(
                    "join.degenerate",
                    format!("LINE {index} has a non-finite endpoint"),
                ));
            }
        }
        let chord = sub(*end, *start);
        let length = cad_geometry::length(chord);
        if !length.is_finite() || length <= tol {
            return Err(refusal(
                "join.degenerate",
                format!("LINE {index} has no length"),
            ));
        }
        // Parallel: the unit chord's component across the common direction is
        // within the (dimensionless) angular predicate derived from the world
        // tolerance over the line's own length.
        let along = Point3 {
            x: chord.x / length,
            y: chord.y / length,
            z: chord.z / length,
        };
        let across = sub(
            along,
            Point3 {
                x: unit.x * dot(along, unit),
                y: unit.y * dot(along, unit),
                z: unit.z * dot(along, unit),
            },
        );
        let angular = (tol / length).max(1e-12);
        if cad_geometry::length(across) > angular {
            return Err(refusal(
                "join.not_collinear",
                format!("LINE {index} is not parallel to the merge direction"),
            ));
        }
        // On the common line: both endpoints' perpendicular offset is inside the
        // world tolerance.
        let mut projected = [0.0_f64; 2];
        for (slot, point) in [start, end].iter().enumerate() {
            let offset = sub(**point, base);
            let along = dot(offset, unit);
            let residual = sub(
                offset,
                Point3 {
                    x: unit.x * along,
                    y: unit.y * along,
                    z: unit.z * along,
                },
            );
            if cad_geometry::length(residual) > tol {
                return Err(refusal(
                    "join.not_collinear",
                    format!("LINE {index} does not lie on the first pick's line"),
                ));
            }
            projected[slot] = along;
        }
        spans.push(Span {
            start: projected[0].min(projected[1]),
            end: projected[0].max(projected[1]),
        });
    }

    // Touching/overlapping union along the common direction (no gap).
    spans.sort_by(|a, b| {
        a.start
            .partial_cmp(&b.start)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut union_end = spans[0].end;
    for span in &spans[1..] {
        if span.start > union_end + tol {
            return Err(refusal(
                "join.not_contiguous",
                format!(
                    "a {} gap separates the selected LINEs; JOIN does not bridge a gap",
                    span.start - union_end
                ),
            ));
        }
        union_end = union_end.max(span.end);
    }
    let union_start = spans[0].start;
    if union_end - union_start <= tol {
        return Err(refusal(
            "join.degenerate",
            "the selected LINEs have no extent to merge",
        ));
    }
    let start = Point3 {
        x: base.x + unit.x * union_start,
        y: base.y + unit.y * union_start,
        z: base.z + unit.z * union_start,
    };
    let end = Point3 {
        x: base.x + unit.x * union_end,
        y: base.y + unit.y * union_end,
        z: base.z + unit.z * union_end,
    };
    // A LINE is an unordered segment, so the merged endpoint order would otherwise
    // follow the pick order (the reference is the longest line). Order the pair
    // ascending by `(x, y, z)` so the same three lines always produce byte-identical
    // geometry — a declared choice, not a geometric claim.
    let (start, end) = if (start.x, start.y, start.z) <= (end.x, end.y, end.z) {
        (start, end)
    } else {
        (end, start)
    };
    Ok(Joined::Line { start, end })
}

/// Plan an ARC merge (see the module docs for the rule).
fn plan_arcs(arcs: &[(Point3, Point3, f64, f64, f64)]) -> CadResult<Joined> {
    let tol = join_tolerance();
    let (center, normal, radius, _start, sweep) = arcs[0];
    if !is_finite_point(center) || !is_finite_point(normal) {
        return Err(refusal("join.degenerate", "the first ARC is not finite"));
    }
    let radius = radius.abs();
    if !radius.is_finite() || radius <= tol {
        return Err(refusal("join.degenerate", "the first ARC has no radius"));
    }
    let angle_tol = angle_tolerance(radius);
    if !sweep.is_finite() || sweep.abs() <= angle_tol {
        return Err(refusal("join.degenerate", "the first ARC has no sweep"));
    }
    let sign = if sweep > 0.0 { 1.0 } else { -1.0 };

    let mut spans: Vec<(f64, f64)> = Vec::with_capacity(arcs.len());
    for (index, (other_center, other_normal, other_radius, other_start, other_sweep)) in
        arcs.iter().enumerate()
    {
        if !is_finite_point(*other_center) || !is_finite_point(*other_normal) {
            return Err(refusal(
                "join.degenerate",
                format!("ARC {index} is not finite"),
            ));
        }
        if distance(center, *other_center) > tol {
            return Err(refusal(
                "join.not_contiguous",
                format!("ARC {index} has a different centre; only concentric ARCs merge"),
            ));
        }
        // Same plane orientation: parallel normals **in the same direction**. A
        // reversed normal describes the same circle with a negated sweep, which
        // this build refuses instead of re-deriving the direction.
        let normal_dot = dot(normal, *other_normal);
        let cross = Point3 {
            x: normal.y * other_normal.z - normal.z * other_normal.y,
            y: normal.z * other_normal.x - normal.x * other_normal.z,
            z: normal.x * other_normal.y - normal.y * other_normal.x,
        };
        if normal_dot <= 0.0 || cad_geometry::length(cross) > (tol / radius).max(1e-12) {
            return Err(refusal(
                "join.not_contiguous",
                format!("ARC {index} has a different normal; only co-planar ARCs merge"),
            ));
        }
        let other_radius = other_radius.abs();
        if !other_radius.is_finite() || (other_radius - radius).abs() > tol {
            return Err(refusal(
                "join.not_contiguous",
                format!("ARC {index} has a different radius; only equal radii merge"),
            ));
        }
        if !other_sweep.is_finite() || other_sweep.abs() <= angle_tol {
            return Err(refusal(
                "join.degenerate",
                format!("ARC {index} has no sweep"),
            ));
        }
        if other_sweep * sign <= 0.0 {
            return Err(refusal(
                "join.not_contiguous",
                format!("ARC {index} runs the other way; JOIN needs one direction"),
            ));
        }
        // Work in the rotational frame where every sweep is positive.
        spans.push((normalize_positive(other_start * sign), other_sweep.abs()));
    }

    // Unroll: an arc chain may cross the 0/2π seam, so every arc is tried as the
    // chain's start. The first cut that yields a gap-free union wins.
    for cut in 0..spans.len() {
        let base = spans[cut].0;
        let mut unrolled: Vec<(f64, f64)> = spans
            .iter()
            .map(|(start, length)| {
                let mut start = *start;
                while start < base - angle_tol {
                    start += TAU;
                }
                (start, start + *length)
            })
            .collect();
        unrolled.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        let mut union_end = unrolled[0].1;
        let mut contiguous = true;
        for span in &unrolled[1..] {
            if span.0 > union_end + angle_tol {
                contiguous = false;
                break;
            }
            union_end = union_end.max(span.1);
        }
        if !contiguous {
            continue;
        }
        let total = union_end - base;
        if total > TAU + angle_tol {
            return Err(refusal(
                "join.sweep_overflow",
                format!("the summed sweep {total} exceeds one full turn; JOIN never wraps an ARC"),
            ));
        }
        return Ok(Joined::Arc {
            center,
            normal,
            radius,
            start: normalize_signed(sign * base),
            sweep: sign * total,
        });
    }
    Err(refusal(
        "join.not_contiguous",
        "the selected ARCs do not form one continuous sweep",
    ))
}

/// Normalise an angle into `[0, 2π)`.
fn normalize_positive(angle: f64) -> f64 {
    let value = angle % TAU;
    if value < 0.0 {
        value + TAU
    } else {
        value
    }
}
