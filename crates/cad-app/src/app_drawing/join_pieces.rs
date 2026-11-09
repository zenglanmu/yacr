//! The shared polyline chain pieces of JOIN and PEDIT JOIN.
//!
//! A piece is an open polyline (`bulges.len() == points.len() - 1`); LINEs and
//! ARCs become a one-segment piece, an open POLYLINE keeps its own segments.

use super::super::*;
use super::{angle_tolerance, invalid, join_tolerance, unsupported};
use cad_geometry::{add, arbitrary_axis, arc_sweep_to_bulge, is_xy_parallel, normalize, scale};

/// An open polyline chain piece: `bulges.len() == points.len() - 1`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PolylinePiece {
    pub points: Vec<Point3>,
    pub bulges: Vec<f64>,
}

impl PolylinePiece {
    fn reversed(self) -> Self {
        let mut points = self.points;
        let mut bulges = self.bulges;
        points.reverse();
        bulges.reverse();
        for bulge in &mut bulges {
            *bulge = -*bulge;
        }
        PolylinePiece { points, bulges }
    }

    /// Append `piece` after `self`; the shared junction point is dropped.
    fn append(&mut self, piece: PolylinePiece) {
        self.points.extend(piece.points.into_iter().skip(1));
        self.bulges.extend(piece.bulges);
    }

    /// Prepend `piece` before `self`; the junction point and its segment bulge
    /// are dropped.
    fn prepend(&mut self, piece: PolylinePiece) {
        let keep = piece.points.len().saturating_sub(1);
        let mut points = piece.points;
        points.truncate(keep);
        points.extend(std::mem::take(&mut self.points));
        // The dropped duplicate point closes `piece`'s last segment onto
        // `self`'s first point, so every piece bulge is still a real segment.
        let mut bulges = piece.bulges;
        bulges.extend(std::mem::take(&mut self.bulges));
        self.points = points;
        self.bulges = bulges;
    }
}

/// The point at `angle` on a circle/arc in its own plane.
fn arc_point(center: Point3, normal: Point3, radius: f64, angle: f64) -> Point3 {
    let (ax, ay, _) = arbitrary_axis(normal);
    add(
        center,
        add(
            scale(ax, radius * angle.cos()),
            scale(ay, radius * angle.sin()),
        ),
    )
}

/// Turn one LINE/ARC/open POLYLINE into a chain piece, or refuse by name.
///
/// `prefix` is the calling command's refusal prefix (`"join"` or
/// `"pedit.join"`).
pub(crate) fn piece_from_geometry(
    geometry: &SemanticGeometry,
    prefix: &str,
) -> CadResult<PolylinePiece> {
    let tol = join_tolerance();
    match geometry {
        SemanticGeometry::Line { start, end } => {
            if !is_finite_point(*start) || !is_finite_point(*end) {
                return Err(invalid(
                    &format!("{prefix}.degenerate"),
                    "a LINE has a non-finite endpoint",
                ));
            }
            if distance(*start, *end) <= tol {
                return Err(invalid(
                    &format!("{prefix}.degenerate"),
                    "a LINE has no length to join",
                ));
            }
            Ok(PolylinePiece {
                points: vec![*start, *end],
                bulges: vec![0.0],
            })
        }
        SemanticGeometry::Arc {
            center,
            normal,
            radius,
            start,
            sweep,
        } => {
            let radius = radius.abs();
            if !is_finite_point(*center) || !is_finite_point(*normal) {
                return Err(invalid(
                    &format!("{prefix}.degenerate"),
                    "an ARC is not finite",
                ));
            }
            if !radius.is_finite() || radius <= tol {
                return Err(invalid(
                    &format!("{prefix}.degenerate"),
                    "an ARC has no radius",
                ));
            }
            if !sweep.is_finite() || sweep.abs() <= angle_tolerance(radius) {
                return Err(invalid(
                    &format!("{prefix}.degenerate"),
                    "an ARC has no sweep",
                ));
            }
            // A bulged polyline segment is defined against the +Z plane.
            let unit_normal = normalize(*normal);
            if unit_normal.z < 1.0 - 1e-9 {
                return Err(unsupported(
                    &format!("{prefix}.noncoplanar"),
                    "an ARC outside the +Z plane cannot become a bulged polyline segment",
                ));
            }
            let Some(bulge) = arc_sweep_to_bulge(*sweep) else {
                return Err(unsupported(
                    &format!("{prefix}.sweep_overflow"),
                    "an ARC sweep of one full turn or more cannot be one bulge segment",
                ));
            };
            Ok(PolylinePiece {
                points: vec![
                    arc_point(*center, *normal, radius, *start),
                    arc_point(*center, *normal, radius, start + sweep),
                ],
                bulges: vec![bulge],
            })
        }
        SemanticGeometry::Polyline {
            points,
            bulges,
            closed,
        } => {
            if *closed {
                return Err(invalid(
                    &format!("{prefix}.closed_object"),
                    "a closed polyline cannot be joined into another polyline",
                ));
            }
            if points.len() < 2 {
                return Err(invalid(
                    &format!("{prefix}.degenerate"),
                    "a polyline needs at least two vertices",
                ));
            }
            if !points.iter().all(|point| is_finite_point(*point)) {
                return Err(invalid(
                    &format!("{prefix}.degenerate"),
                    "a polyline has a non-finite vertex",
                ));
            }
            if !is_xy_parallel(points) {
                return Err(unsupported(
                    &format!("{prefix}.noncoplanar"),
                    "a non-coplanar 3D polyline is outside this build's POLYLINE join scope",
                ));
            }
            let segments = points.len() - 1;
            let mut bulges = bulges.clone();
            if bulges.len() < segments {
                if bulges.len() == segments - 1 {
                    bulges.push(0.0);
                } else {
                    return Err(unsupported(
                        &format!("{prefix}.unsupported"),
                        format!(
                            "a polyline with {} vertices and {} bulges is not addressable",
                            points.len(),
                            bulges.len()
                        ),
                    ));
                }
            }
            bulges.truncate(segments);
            if !bulges.iter().all(|bulge| bulge.is_finite()) {
                return Err(invalid(
                    &format!("{prefix}.degenerate"),
                    "a polyline has a non-finite bulge",
                ));
            }
            Ok(PolylinePiece {
                points: points.clone(),
                bulges,
            })
        }
        other => Err(unsupported(
            &format!("{prefix}.unsupported"),
            format!("{} cannot be joined into a polyline", class_name(other)),
        )),
    }
}

/// Chain touching open polyline pieces end-to-end (the shared JOIN/PEDIT JOIN
/// rule). Every piece must be used exactly once.
pub(crate) fn chain_pieces(
    mut pieces: Vec<PolylinePiece>,
    prefix: &str,
) -> CadResult<PolylinePiece> {
    let tol = join_tolerance();
    if pieces.len() < 2 {
        return Err(invalid(
            &format!("{prefix}.needs_two"),
            "a polyline chain needs at least two pieces",
        ));
    }
    let mut current = pieces.remove(0);
    while !pieces.is_empty() {
        let tail = *current.points.last().expect("a piece has points");
        let head = current.points[0];
        let mut matched = false;
        for index in 0..pieces.len() {
            let piece = &pieces[index];
            let piece_head = piece.points[0];
            let piece_tail = *piece.points.last().expect("a piece has points");
            // The piece's head is the free end only when it is appended forward;
            // a matching tail means the piece runs the other way.
            if distance(piece_head, tail) <= tol {
                let piece = pieces.remove(index);
                current.append(piece);
                matched = true;
                break;
            }
            if distance(piece_tail, tail) <= tol {
                let piece = pieces.remove(index).reversed();
                current.append(piece);
                matched = true;
                break;
            }
            if distance(piece_tail, head) <= tol {
                let piece = pieces.remove(index);
                current.prepend(piece);
                matched = true;
                break;
            }
            if distance(piece_head, head) <= tol {
                let piece = pieces.remove(index).reversed();
                current.prepend(piece);
                matched = true;
                break;
            }
        }
        if !matched {
            return Err(invalid(
                &format!("{prefix}.not_contiguous"),
                "a selected object does not touch the polyline chain; JOIN does not bridge gaps",
            ));
        }
    }
    Ok(current)
}
