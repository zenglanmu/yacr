//! `OVERKILL` — AutoCAD 2026's *Delete Duplicate Objects* (`-OVERKILL`).
//!
//! The dialog model is one comparison precision (**tolerance**) plus per-property
//! ignore toggles and four option groups. This module implements that model over
//! the modelled entity set; [`OverkillSettings`] is the typed payload and
//! [`Application::overkill`] the write path.
//!
//! ## What is compared
//!
//! Two selected entities are compared only when they are in the **same space**
//! and every property the settings do **not** ignore is equal:
//!
//! * layer id (unless `ignore_layer`),
//! * stored display colour / linetype *name* / lineweight / transparency (each
//!   unless its ignore toggle is set),
//! * the entity's own linetype scale, within the tolerance (unless
//!   `ignore_linetype_scale`).
//!
//! Thickness, plot style and material are **not modelled** on a `cad-db` entity.
//! The default settings ignore them (the only honest answer); asking to *compare*
//! one is the named `overkill.<property>_not_modelled` refusal rather than a
//! silently weaker comparison.
//!
//! ## What is removed / combined
//!
//! 1. **Duplicate objects** of the comparable kinds (LINE, LWPOLYLINE, CIRCLE,
//!    ARC, SPLINE) within the tolerance; the survivor is the lowest
//!    `(draw_order, id)`, so a delete can never pick the wrong copy.
//! 2. **Polyline segment optimisation** (when `optimize_segments` and not
//!    `do_not_break_polylines`): zero-length segments and immediately repeated
//!    (out-and-back) straight segments are removed from each polyline. A polyline
//!    that collapses below two vertices is deleted.
//! 3. **Collinear LINE combines**: partially overlapping segments
//!    (`combine_partial_overlap`) and segments that merely touch end to end
//!    (`combine_end_to_end`) are replaced by their union. The keeper is the
//!    lowest `(draw_order, id)` and is updated in place; the other LINE is
//!    deleted.
//! 4. **Arcs over circles**: an ARC that lies on a comparable CIRCLE's
//!    circumference is redundant and is deleted.
//! 5. **Duplicate segments across objects** (when `optimize_segments`): a
//!    standalone LINE or ARC that exactly reproduces one segment of a comparable
//!    LWPOLYLINE is deleted. The polyline is the keeper for this cross-object
//!    comparison and is never modified by it (`do_not_break_polylines` only
//!    controls step 2).
//!
//! `maintain_associative` has no association model to maintain; it is a **named
//! no-op** reported by `overkill.associative_noop`. Polyline segment *width* is
//! not modelled either; `ignore_polyline_segment_width` is reported as a no-op
//! (`overkill.segment_width_not_modelled`) and the other value changes nothing.
//!
//! Every write is **one transaction / one undo step**. A refused command opens no
//! transaction, bumps no revision and records no undo entry.

use super::*;
use cad_db::{EntityLineType, EntityRenderAttributes};

/// The tolerance/ignore/option set of the AutoCAD 2026 OVERKILL dialog.
///
/// Autodesk does not publish the dialog's initial state, so these are the
/// **declared defaults** of this build:
///
/// | field | default | meaning |
/// | --- | --- | --- |
/// | `tolerance` | `0.0` | exact comparison (the documented "must match exactly") |
/// | `ignore_color` … `ignore_transparency` | `false` | compare the modelled properties |
/// | `ignore_thickness` / `ignore_plot_style` / `ignore_material` | `true` | not modelled; comparing them is refused |
/// | `optimize_segments` | `true` | run the polyline/cross-object segment passes |
/// | `ignore_polyline_segment_width` | `true` | width is not modelled (no-op either way) |
/// | `do_not_break_polylines` | `false` | segment optimisation may rewrite polylines |
/// | `combine_partial_overlap` / `combine_end_to_end` | `true` | both documented collinear combines |
/// | `maintain_associative` | `false` | named no-op (no association model) |
#[derive(Debug, Clone, PartialEq)]
pub struct OverkillSettings {
    /// Comparison precision in drawing units (and radians for angles). `0.0`
    /// means "must match exactly"; a negative/non-finite value is refused.
    pub tolerance: f64,
    /// Ignore the stored display colour when comparing.
    pub ignore_color: bool,
    /// Ignore the layer when comparing.
    pub ignore_layer: bool,
    /// Ignore the stored linetype name when comparing.
    pub ignore_linetype: bool,
    /// Ignore the entity's own linetype scale when comparing.
    pub ignore_linetype_scale: bool,
    /// Ignore the stored display lineweight when comparing.
    pub ignore_lineweight: bool,
    /// Ignore thickness. `false` is refused (`overkill.thickness_not_modelled`).
    pub ignore_thickness: bool,
    /// Ignore transparency when comparing.
    pub ignore_transparency: bool,
    /// Ignore plot style. `false` is refused (`overkill.plot_style_not_modelled`).
    pub ignore_plot_style: bool,
    /// Ignore material. `false` is refused (`overkill.material_not_modelled`).
    pub ignore_material: bool,
    /// `Optimize Segments Within Polylines`.
    pub optimize_segments: bool,
    /// The sub-option `Ignore Polyline Segment Width` (a reported no-op: the
    /// entity model stores no per-segment width).
    pub ignore_polyline_segment_width: bool,
    /// The sub-option `Do Not Break Polylines`: leave polyline objects unchanged.
    pub do_not_break_polylines: bool,
    /// `Combine Collinear Objects That Partially Overlap`.
    pub combine_partial_overlap: bool,
    /// `Combine Collinear Objects When Aligned End to End`.
    pub combine_end_to_end: bool,
    /// `Maintain Associative Objects` (named no-op: no association model).
    pub maintain_associative: bool,
}

impl Default for OverkillSettings {
    fn default() -> Self {
        OverkillSettings {
            tolerance: 0.0,
            ignore_color: false,
            ignore_layer: false,
            ignore_linetype: false,
            ignore_linetype_scale: false,
            ignore_lineweight: false,
            ignore_thickness: true,
            ignore_transparency: false,
            ignore_plot_style: true,
            ignore_material: true,
            optimize_segments: true,
            ignore_polyline_segment_width: true,
            do_not_break_polylines: false,
            combine_partial_overlap: true,
            combine_end_to_end: true,
            maintain_associative: false,
        }
    }
}

mod compare;

use compare::{
    points_equal, properties_match, same_duplicate_geometry, vdot, vnorm, vscale, vsub, within,
};

/// The result of one collinear LINE combine attempt.
enum Collinear {
    /// The intervals overlap over a positive length (containment included).
    Partial(SemanticGeometry),
    /// The intervals meet at one endpoint (within the tolerance).
    EndToEnd(SemanticGeometry),
}

/// Combine two collinear LINEs into their union.
///
/// `keeper` is the lower-draw-order LINE. Both other endpoints must lie within
/// `tol` of the keeper's infinite line; a gap larger than `tol` is not a combine.
fn collinear_merge(
    keeper: &SemanticGeometry,
    other: &SemanticGeometry,
    tol: f64,
) -> Option<Collinear> {
    let (
        SemanticGeometry::Line { start: k0, end: k1 },
        SemanticGeometry::Line { start: o0, end: o1 },
    ) = (keeper, other)
    else {
        return None;
    };
    let direction = vsub(*k1, *k0);
    let length = vnorm(direction);
    if length <= tol || !length.is_finite() {
        return None;
    }
    let unit = vscale(direction, 1.0 / length);
    // Perpendicular distance from the keeper's infinite line.
    let distance = |p: Point3| {
        let offset = vsub(p, *k0);
        let along = vdot(offset, unit);
        vnorm(vsub(offset, vscale(unit, along)))
    };
    if distance(*o0) > tol || distance(*o1) > tol {
        return None;
    }
    let along = |p: Point3| vdot(vsub(p, *k0), unit);
    let (k_min, k_max) = (0.0f64, length);
    let (o_a, o_b) = (along(*o0), along(*o1));
    let (o_min, o_max) = if o_a <= o_b { (o_a, o_b) } else { (o_b, o_a) };
    let overlap = k_max.min(o_max) - k_min.max(o_min);
    if overlap < -tol {
        return None;
    }
    let union_min = k_min.min(o_min);
    let union_max = k_max.max(o_max);
    if union_max - union_min <= tol {
        return None;
    }
    let extreme = |target: f64, prefer: Point3| {
        let mut best = prefer;
        let mut best_gap = (along(prefer) - target).abs();
        for candidate in [*o0, *o1] {
            let gap = (along(candidate) - target).abs();
            if gap < best_gap {
                best = candidate;
                best_gap = gap;
            }
        }
        best
    };
    // Prefer the keeper's own endpoint when it is already at the extreme.
    let start = extreme(union_min, if along(*k0) <= along(*k1) { *k0 } else { *k1 });
    let end = extreme(union_max, if along(*k0) <= along(*k1) { *k1 } else { *k0 });
    let union = SemanticGeometry::Line { start, end };
    if overlap.abs() <= tol {
        Some(Collinear::EndToEnd(union))
    } else {
        Some(Collinear::Partial(union))
    }
}

/// One segment of a polyline as a comparable standalone geometry.
fn polyline_segments(
    points: &[Point3],
    bulges: &[f64],
    closed: bool,
) -> Option<Vec<SemanticGeometry>> {
    let n = points.len();
    if n < 2 {
        return Some(Vec::new());
    }
    let segments = if closed { n } else { n - 1 };
    // A non-empty bulge array must be either one entry per vertex or one per
    // segment. Anything else has no interpretation this build can trust.
    if !bulges.is_empty() && bulges.len() != n && bulges.len() != segments {
        return None;
    }
    let mut out = Vec::with_capacity(segments);
    for i in 0..segments {
        let a = points[i];
        let b = points[(i + 1) % n];
        let bulge = bulges.get(i).copied().unwrap_or(0.0);
        if bulge.abs() <= f64::EPSILON {
            out.push(SemanticGeometry::Line { start: a, end: b });
            continue;
        }
        // A bulged segment is an in-plane arc. The polyline stores no plane
        // normal; +Z is the documented 2D assumption and a segment whose
        // endpoints differ in Z is skipped rather than guessed.
        if !within(a.z, b.z, 1e-9) {
            continue;
        }
        let normal = Point3 {
            x: 0.0,
            y: 0.0,
            z: 1.0,
        };
        if let Some(arc) = cad_geometry::bulge_arc_geometry(a, b, bulge, normal) {
            out.push(arc);
        }
    }
    Some(out)
}

/// The outcome of the polyline segment optimiser.
enum PolylineOptimization {
    /// Nothing removable (or fewer than two vertices).
    Unchanged,
    /// The rewritten points and bulges.
    Changed(Vec<Point3>, Vec<f64>),
    /// The bulge array is neither empty nor one entry per vertex, so this build
    /// has no storage-layout interpretation it can trust.
    UnsupportedLayout,
}

/// Remove zero-length and immediately repeated (out-and-back) straight segments
/// from one polyline.
///
/// Only the **per-vertex** bulge layout (`bulges.len() == points.len()`, the
/// layout every shipped constructor writes) and the empty array are rewritten.
/// A per-segment array (`points.len() - 1`) is reported as
/// [`PolylineOptimization::UnsupportedLayout`] instead of guessing which entry
/// belongs to which segment after a vertex is dropped.
fn optimize_polyline(
    points: &[Point3],
    bulges: &[f64],
    closed: bool,
    tol: f64,
) -> PolylineOptimization {
    let n = points.len();
    if n < 2 {
        return PolylineOptimization::Unchanged;
    }
    let per_vertex = !bulges.is_empty() && bulges.len() == n;
    if !bulges.is_empty() && !per_vertex {
        return PolylineOptimization::UnsupportedLayout;
    }
    let mut pts = points.to_vec();
    let mut bul = bulges.to_vec();
    let mut changed = false;

    // Remove point `j` (already proven to be a duplicate of a neighbour) and, in
    // the per-vertex layout, its bulge entry.
    let remove = |pts: &mut Vec<Point3>, bul: &mut Vec<f64>, j: usize| {
        if per_vertex && j < bul.len() {
            bul.remove(j);
        }
        pts.remove(j);
    };

    // The absolute bulge of segment `i` in the **working** array (indices shift
    // as vertices are dropped).
    let bulge_at = |bul: &[f64], i: usize| bul.get(i).copied().unwrap_or(0.0).abs();

    loop {
        let mut removed = false;
        let n = pts.len();
        if n < 2 {
            break;
        }
        let segments = if closed { n } else { n - 1 };
        // Adjacent duplicate points: a zero-length segment (including the closing
        // segment of a closed ring, where point n-1 duplicates point 0).
        for i in 0..segments {
            let a = pts[i];
            let b = pts[(i + 1) % n];
            if points_equal(a, b, tol) && bulge_at(&bul, i) <= tol {
                let victim = if i + 1 == n { n - 1 } else { i + 1 };
                remove(&mut pts, &mut bul, victim);
                changed = true;
                removed = true;
                break;
            }
        }
        if removed {
            continue;
        }
        // Out-and-back: p[i] and p[i+2] coincide, so the two straight segments
        // cover the same chord twice. Only for an open polyline: removing both
        // interior vertices is well defined there.
        if !closed && n >= 3 {
            for i in 0..n - 2 {
                if points_equal(pts[i], pts[i + 2], tol)
                    && bulge_at(&bul, i) <= tol
                    && bulge_at(&bul, i + 1) <= tol
                {
                    remove(&mut pts, &mut bul, i + 2);
                    remove(&mut pts, &mut bul, i + 1);
                    changed = true;
                    removed = true;
                    break;
                }
            }
        }
        if !removed {
            break;
        }
    }
    if changed {
        PolylineOptimization::Changed(pts, bul)
    } else {
        PolylineOptimization::Unchanged
    }
}

/// One entity as the plan sees it: the stored row, its working (possibly merged)
/// form and its fate.
struct Candidate {
    before: DbEntity,
    working: DbEntity,
    deleted: bool,
    updated: bool,
}

impl Application {
    /// `OVERKILL`: remove duplicates / combine collinear objects / optimise
    /// polyline segments over the current selection, in **one** transaction.
    ///
    /// The selection is the whole input (AutoCAD's select-objects prompt is the
    /// declared divergence). Every selected ref must be directly addressable and
    /// resolve to a stored entity; the first violation refuses the whole command
    /// before a write. See the module docs for the comparison and repair rules.
    pub(crate) fn overkill(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let settings = overkill_settings(command)?;
        if !settings.tolerance.is_finite() || settings.tolerance < 0.0 {
            return Err(CadError::InvalidInput(
                "overkill.tolerance: the comparison tolerance must be finite and non-negative \
                 (0 means exact)"
                    .into(),
            ));
        }
        if !settings.ignore_thickness {
            return Err(CadError::Unsupported(
                "overkill.thickness_not_modelled: entities in this build have no thickness \
                 field, so thickness cannot be compared"
                    .into(),
            ));
        }
        if !settings.ignore_plot_style {
            return Err(CadError::Unsupported(
                "overkill.plot_style_not_modelled: entities in this build have no plot-style \
                 field, so plot style cannot be compared"
                    .into(),
            ));
        }
        if !settings.ignore_material {
            return Err(CadError::Unsupported(
                "overkill.material_not_modelled: entities in this build have no material field, \
                 so material cannot be compared"
                    .into(),
            ));
        }
        let references = session.selection.refs();
        if references.is_empty() {
            return Err(CadError::InvalidInput(
                "overkill.selection: OVERKILL needs at least one selected entity".into(),
            ));
        }
        let document_id = command.document;
        let drawing = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?
            .drawing
            .as_ref();

        // Resolve the directly addressable rows first; a foreign or
        // block-instance/sub-element ref is a named refusal (never silently
        // skipped, because the user asked to compare that object).
        let mut seen: Vec<EntityId> = Vec::new();
        let mut candidates: Vec<Candidate> = Vec::new();
        for reference in references {
            if reference.document != document_id {
                return Err(CadError::InvalidInput(format!(
                    "overkill.not_addressable: entity {} belongs to another document",
                    reference.entity.0
                )));
            }
            if !reference.instance.0.is_empty() || reference.sub_element.is_some() {
                return Err(CadError::InvalidInput(format!(
                    "overkill.not_addressable: entity {} is referenced through a block instance \
                     or a sub-element; only directly addressable entities can be compared",
                    reference.entity.0
                )));
            }
            if seen.contains(&reference.entity) {
                continue;
            }
            seen.push(reference.entity);
            let entity = drawing.entity(reference.entity).cloned().ok_or_else(|| {
                CadError::InvalidInput(format!(
                    "overkill.target_missing: entity {} does not exist in this drawing",
                    reference.entity.0
                ))
            })?;
            candidates.push(Candidate {
                before: entity.clone(),
                working: entity,
                deleted: false,
                updated: false,
            });
        }
        // Deterministic keeper order: the lowest draw order, ties on the id.
        candidates.sort_by_key(|candidate| (candidate.before.draw_order, candidate.before.id.0));
        let mut diagnostics = Vec::new();
        if settings.maintain_associative {
            diagnostics.push(Diagnostic {
                object: None,
                code: "overkill.associative_noop".into(),
                message: "OVERKILL: 'maintain associative objects' is a no-op; this build has no \
                          association model"
                    .into(),
            });
        }
        if settings.optimize_segments && settings.ignore_polyline_segment_width {
            diagnostics.push(Diagnostic {
                object: None,
                code: "overkill.segment_width_not_modelled".into(),
                message: "OVERKILL: polyline segment width is not modelled; the 'ignore polyline \
                          segment width' sub-option changes nothing"
                    .into(),
            });
        }

        let mut counts = Counts::default();

        // 1. Polyline segment optimisation (before the duplicate pass so two
        //    polylines that differ only by zero-length segments still compare
        //    equal).
        if settings.optimize_segments && !settings.do_not_break_polylines {
            for candidate in &mut candidates {
                if candidate.deleted {
                    continue;
                }
                let SemanticGeometry::Polyline {
                    points,
                    bulges,
                    closed,
                } = &candidate.working.geometry
                else {
                    continue;
                };
                let optimized = optimize_polyline(points, bulges, *closed, settings.tolerance);
                let (new_points, new_bulges) = match optimized {
                    PolylineOptimization::Unchanged => continue,
                    PolylineOptimization::UnsupportedLayout => {
                        counts.polyline_layout_unsupported += 1;
                        continue;
                    }
                    PolylineOptimization::Changed(points, bulges) => (points, bulges),
                };
                if new_points.len() < 2 {
                    candidate.deleted = true;
                    candidate.updated = false;
                    counts.polyline_collapsed += 1;
                    continue;
                }
                candidate.working.geometry = SemanticGeometry::Polyline {
                    points: new_points,
                    bulges: new_bulges,
                    closed: *closed,
                };
                candidate.updated = true;
                counts.polylines_optimized += 1;
            }
        }

        // 2. Duplicate objects.
        for i in 0..candidates.len() {
            if candidates[i].deleted {
                continue;
            }
            for j in 0..i {
                if candidates[j].deleted {
                    continue;
                }
                if properties_match(
                    drawing,
                    &candidates[j].before,
                    &candidates[i].before,
                    &settings,
                ) && same_duplicate_geometry(
                    &candidates[j].working.geometry,
                    &candidates[i].working.geometry,
                    settings.tolerance,
                ) {
                    candidates[i].deleted = true;
                    candidates[i].updated = false;
                    counts.duplicates += 1;
                    break;
                }
            }
        }

        // 3. Collinear LINE combines, repeated until no pair merges (a chain of
        //    overlapping segments can merge several times into one union).
        if settings.combine_partial_overlap || settings.combine_end_to_end {
            loop {
                let mut merged = false;
                'outer: for i in 0..candidates.len() {
                    if candidates[i].deleted {
                        continue;
                    }
                    for j in 0..i {
                        if candidates[j].deleted {
                            continue;
                        }
                        if !properties_match(
                            drawing,
                            &candidates[j].before,
                            &candidates[i].before,
                            &settings,
                        ) {
                            continue;
                        }
                        let Some(combined) = collinear_merge(
                            &candidates[j].working.geometry,
                            &candidates[i].working.geometry,
                            settings.tolerance,
                        ) else {
                            continue;
                        };
                        let allowed = match combined {
                            Collinear::Partial(_) => settings.combine_partial_overlap,
                            Collinear::EndToEnd(_) => settings.combine_end_to_end,
                        };
                        if !allowed {
                            continue;
                        }
                        let union = match combined {
                            Collinear::Partial(union) | Collinear::EndToEnd(union) => union,
                        };
                        candidates[i].deleted = true;
                        candidates[i].updated = false;
                        counts.merged += 1;
                        if !same_duplicate_geometry(
                            &candidates[j].working.geometry,
                            &union,
                            settings.tolerance,
                        ) {
                            candidates[j].working.geometry = union;
                            candidates[j].updated = true;
                        }
                        merged = true;
                        break 'outer;
                    }
                }
                if !merged {
                    break;
                }
            }
        }

        // 4. Arcs drawn over circles.
        for i in 0..candidates.len() {
            if candidates[i].deleted {
                continue;
            }
            let SemanticGeometry::Arc {
                center,
                normal,
                radius,
                ..
            } = &candidates[i].working.geometry
            else {
                continue;
            };
            for j in 0..candidates.len() {
                if i == j || candidates[j].deleted {
                    continue;
                }
                let SemanticGeometry::Circle {
                    center: circle_center,
                    normal: circle_normal,
                    radius: circle_radius,
                } = &candidates[j].working.geometry
                else {
                    continue;
                };
                if properties_match(
                    drawing,
                    &candidates[j].before,
                    &candidates[i].before,
                    &settings,
                ) && points_equal(*center, *circle_center, settings.tolerance)
                    && points_equal(*normal, *circle_normal, settings.tolerance)
                    && within(*radius, *circle_radius, settings.tolerance)
                {
                    candidates[i].deleted = true;
                    candidates[i].updated = false;
                    counts.arcs_over_circles += 1;
                    break;
                }
            }
        }

        // 5. Standalone LINE/ARC segments duplicated by a polyline segment.
        if settings.optimize_segments {
            for i in 0..candidates.len() {
                if candidates[i].deleted {
                    continue;
                }
                if !matches!(
                    candidates[i].working.geometry,
                    SemanticGeometry::Line { .. } | SemanticGeometry::Arc { .. }
                ) {
                    continue;
                }
                let duplicate = candidates.iter().enumerate().any(|(j, polyline)| {
                    if j == i || polyline.deleted {
                        return false;
                    }
                    let SemanticGeometry::Polyline {
                        points,
                        bulges,
                        closed,
                    } = &polyline.working.geometry
                    else {
                        return false;
                    };
                    if !properties_match(
                        drawing,
                        &polyline.before,
                        &candidates[i].before,
                        &settings,
                    ) {
                        return false;
                    }
                    let Some(segments) = polyline_segments(points, bulges, *closed) else {
                        return false;
                    };
                    segments.iter().any(|segment| {
                        same_duplicate_geometry(
                            segment,
                            &candidates[i].working.geometry,
                            settings.tolerance,
                        )
                    })
                });
                if duplicate {
                    candidates[i].deleted = true;
                    candidates[i].updated = false;
                    counts.duplicate_segments += 1;
                }
            }
        }

        // Nothing to do: no transaction, no revision bump, no undo entry.
        if counts.polyline_layout_unsupported > 0 {
            diagnostics.push(Diagnostic {
                object: None,
                code: "overkill.polyline_bulge_layout".into(),
                message: format!(
                    "OVERKILL: {} polyline(s) store a per-segment bulge array; segment \
                     optimisation left them unchanged",
                    counts.polyline_layout_unsupported
                ),
            });
        }
        let deletes: Vec<EntityId> = candidates
            .iter()
            .filter(|candidate| candidate.deleted)
            .map(|candidate| candidate.before.id)
            .collect();
        let updates: Vec<(EntityId, DbEntity)> = candidates
            .iter()
            .filter(|candidate| candidate.updated && !candidate.deleted)
            .map(|candidate| (candidate.before.id, candidate.working.clone()))
            .collect();
        if deletes.is_empty() && updates.is_empty() {
            diagnostics.push(Diagnostic {
                object: None,
                code: "overkill.unchanged".into(),
                message: "OVERKILL: nothing to delete or combine among the selected entities"
                    .into(),
            });
            return Ok(CommandOutcome {
                objects: Vec::new(),
                changes: None,
                diagnostics,
                measurement: None,
                point_report: None,
            });
        }

        let document = self
            .workspace
            .documents
            .get_mut(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        let mut candidate_drawing = document.drawing.as_ref().clone();
        let drawing = &mut candidate_drawing;
        // Capture the display state each delete prunes, so undo restores it.
        let mut display_states = Vec::with_capacity(deletes.len());
        for id in &deletes {
            display_states.push(drawing.entity_display_state(*id));
        }
        let transaction = next_transaction_id();
        let change_set = {
            let mut staged = drawing.begin_drawing_transaction("overkill", transaction)?;
            for (_, after) in &updates {
                staged.update_entity(after.clone())?;
            }
            for id in &deletes {
                staged.delete_entity(*id)?;
            }
            staged.commit()?
        };
        let mut patches = Vec::with_capacity(updates.len() + deletes.len());
        for (id, after) in &updates {
            let stored = drawing.entity(*id).cloned().ok_or_else(|| {
                CadError::Invariant(format!("OVERKILL: entity {} vanished after commit", id.0))
            })?;
            let before = candidates
                .iter()
                .find(|candidate| candidate.before.id == *id)
                .map(|candidate| candidate.before.clone())
                .ok_or_else(|| {
                    CadError::Invariant(format!("OVERKILL: entity {} lost its before state", id.0))
                })?;
            let _ = after;
            patches.push(drawing_patch(*id, Some(before), Some(stored)));
        }
        for (id, state) in deletes.iter().zip(display_states) {
            let before = candidates
                .iter()
                .find(|candidate| candidate.before.id == *id)
                .map(|candidate| candidate.before.clone())
                .ok_or_else(|| {
                    CadError::Invariant(format!("OVERKILL: entity {} lost its before state", id.0))
                })?;
            patches.push(drawing_patch_deleted(*id, before, state));
        }
        self.publish_candidate(
            document_id,
            candidate_drawing,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: "overkill".into(),
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

        let mut objects: Vec<ObjectId> = updates.iter().map(|(id, _)| ObjectId(id.0)).collect();
        objects.extend(deletes.iter().map(|id| ObjectId(id.0)));
        diagnostics.push(Diagnostic {
            object: None,
            code: "overkill.applied".into(),
            message: format!(
                "OVERKILL: deleted {} object(s) ({} duplicate(s), {} collinear merge(s), {} arc(s) \
                 over circles, {} duplicate segment object(s), {} collapsed polyline(s)); \
                 optimized {} polyline(s); tolerance {}",
                deletes.len(),
                counts.duplicates,
                counts.merged,
                counts.arcs_over_circles,
                counts.duplicate_segments,
                counts.polyline_collapsed,
                counts.polylines_optimized,
                settings.tolerance
            ),
        });
        Ok(CommandOutcome {
            objects,
            changes: Some(Arc::new(change_set)),
            diagnostics,
            measurement: None,
            point_report: None,
        })
    }
}

/// The documented counters behind the `overkill.applied` report.
#[derive(Default)]
struct Counts {
    duplicates: usize,
    merged: usize,
    polylines_optimized: usize,
    polyline_collapsed: usize,
    polyline_layout_unsupported: usize,
    arcs_over_circles: usize,
    duplicate_segments: usize,
}

/// Read the optional typed settings; `None` means the declared dialog defaults.
fn overkill_settings(command: &Command) -> CadResult<OverkillSettings> {
    match &command.payload {
        CommandPayload::None => Ok(OverkillSettings::default()),
        CommandPayload::SelectionTool(payload) => match payload.as_ref() {
            SelectionToolPayload::Overkill(settings) => Ok((**settings).clone()),
            _ => Err(CadError::InvalidInput(
                "overkill.invalid_payload: OVERKILL accepts only its own settings".into(),
            )),
        },
        _ => Err(CadError::InvalidInput(
            "overkill.invalid_payload: OVERKILL takes no payload or its typed settings".into(),
        )),
    }
}
