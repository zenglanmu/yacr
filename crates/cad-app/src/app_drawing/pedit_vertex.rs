//! The `PEDIT Edit vertex` planner (next/previous, insert, move, straighten,
//! tangent/width report).
//!
//! Split out of `pedit.rs` (the code-size limit); it is a private helper module
//! of that file, not a second option surface. `pedit.rs` owns the selection
//! rules, the transaction and the undo record; everything here only resolves the
//! geometry a sub-menu action would write, or the named refusal/no-op.

use super::*;

/// The `PEDIT Edit vertex` planner.
///
/// Every action is resolved before the caller opens its transaction:
///
/// | action | write |
/// | --- | --- |
/// | `NEXT` / `PREVIOUS` | none — a report of the neighbouring vertex (the marker move is the whole effect) |
/// | `INSERT` | one vertex on the named segment at the typed point's projection; the curve's shape is preserved exactly |
/// | `MOVE` | the named vertex's coordinates; the segment bulges are kept |
/// | `STRAIGHTEN` | the vertices strictly between the two marks are removed and the merged segment is straight |
/// | `TANGENT` / `WIDTH` | named capability refusal (no stored fit tangent / segment width) |
///
/// A `NEXT`/`PREVIOUS` report is a named no-op (`pedit.vertex_report`): no
/// transaction, no revision bump, no undo entry.
pub(super) fn plan_vertex(
    geometry: SemanticGeometry,
    request: PeditVertexCommand,
) -> CadResult<Plan> {
    let SemanticGeometry::Polyline {
        mut points,
        bulges,
        closed,
        width,
    } = geometry
    else {
        return Err(unsupported(
            "not_polyline",
            "PEDIT Edit vertex needs a POLYLINE",
        ));
    };
    let count = points.len();
    let segments = if closed { count } else { count - 1 };
    let tol = join_tolerance();
    match request.action {
        PeditVertexAction::Next { from } => neighbour_report(&points, from, closed, 1, "NEXT"),
        PeditVertexAction::Previous { from } => {
            neighbour_report(&points, from, closed, -1, "PREVIOUS")
        }
        PeditVertexAction::Insert { after, point } => {
            if !is_finite_point(point) {
                return Err(invalid(
                    "vertex_invalid_point",
                    "the PEDIT EDIT INSERT point must be finite",
                ));
            }
            if after >= segments {
                return Err(invalid(
                    "vertex_out_of_range",
                    format!(
                        "PEDIT EDIT INSERT needs a segment index in 0..{segments}, got {after}"
                    ),
                ));
            }
            let curve = PolylineCurve::new(&points, &bulges, closed, "pedit")?;
            let Some(u) = curve.project_param(point, tol)? else {
                return Err(invalid(
                    "vertex_projection_failed",
                    "the PEDIT EDIT INSERT point has no projection onto the polyline",
                ));
            };
            let segment = (u.floor() as usize).min(segments.saturating_sub(1));
            let local = u - segment as f64;
            if segment != after {
                return Err(invalid(
                    "vertex_insert_off_segment",
                    format!(
                        "the PEDIT EDIT INSERT point projects onto segment {segment}, not the \
                         named segment {after}; only an on-curve insert is exact"
                    ),
                ));
            }
            let (inserted_points, inserted_bulges) = curve.insert_vertex_at(after, local)?;
            Ok(Plan::Write {
                geometry: SemanticGeometry::Polyline {
                    points: inserted_points,
                    bulges: inserted_bulges,
                    closed,
                    width,
                },
                label: format!(
                    "Edit vertex INSERT (one vertex on segment {after}, shape preserved)"
                ),
                deleted: Vec::new(),
            })
        }
        PeditVertexAction::Move { vertex, point } => {
            if !is_finite_point(point) {
                return Err(invalid(
                    "vertex_invalid_point",
                    "the PEDIT EDIT MOVE point must be finite",
                ));
            }
            if vertex >= count {
                return Err(invalid(
                    "vertex_out_of_range",
                    format!("PEDIT EDIT MOVE needs a vertex index in 0..{count}, got {vertex}"),
                ));
            }
            if distance(points[vertex], point) <= tol {
                return Ok(Plan::Noop {
                    code: "pedit.vertex_unchanged",
                    message: format!(
                        "PEDIT EDIT MOVE: vertex {vertex} is already at that point; nothing was \
                         written"
                    ),
                });
            }
            let mut neighbours = Vec::with_capacity(2);
            if vertex > 0 {
                neighbours.push(points[vertex - 1]);
            } else if closed {
                neighbours.push(points[count - 1]);
            }
            if vertex + 1 < count {
                neighbours.push(points[vertex + 1]);
            } else if closed {
                neighbours.push(points[0]);
            }
            if let Some(neighbour) = neighbours
                .into_iter()
                .find(|neighbour| distance(*neighbour, point) <= tol)
            {
                return Err(invalid(
                    "vertex_coincident",
                    format!(
                        "PEDIT EDIT MOVE would put vertex {vertex} on its neighbour \
                         ({}, {}); the segment would have no length",
                        neighbour.x, neighbour.y
                    ),
                ));
            }
            points[vertex] = point;
            if bulges.iter().any(|bulge| bulge.abs() > 1e-9)
                && !cad_geometry::is_xy_parallel(&points)
            {
                return Err(unsupported(
                    "vertex_plane",
                    "moving this vertex would put a bulged polyline off the +Z plane; its arc \
                     segments have no circular form there",
                ));
            }
            Ok(Plan::Write {
                geometry: SemanticGeometry::Polyline {
                    points,
                    bulges,
                    closed,
                    width,
                },
                label: format!("Edit vertex MOVE (vertex {vertex})"),
                deleted: Vec::new(),
            })
        }
        PeditVertexAction::Straighten { first, last } => {
            if first >= last || last >= count {
                return Err(invalid(
                    "vertex_range",
                    format!(
                        "PEDIT EDIT STRAIGHTEN needs 0 ≤ first < last < {count}, got {first} and \
                         {last}"
                    ),
                ));
            }
            let straight = bulges.get(first).copied().unwrap_or(0.0);
            if last == first + 1 && straight.abs() <= 1e-12 {
                return Ok(Plan::Noop {
                    code: "pedit.vertex_already_straight",
                    message: format!(
                        "PEDIT EDIT STRAIGHTEN: the segment {first} → {last} is already straight \
                         and no vertex lies between them; nothing was written"
                    ),
                });
            }
            // Keep the first and last vertices, drop the interior ones, and make
            // the merged segment straight.
            let mut kept_points: Vec<Point3> = points[..=first].to_vec();
            kept_points.extend_from_slice(&points[last..]);
            let mut kept_bulges: Vec<f64> = bulges.get(..first).unwrap_or(&[]).to_vec();
            kept_bulges.push(0.0);
            if let Some(tail) = bulges.get(last..) {
                kept_bulges.extend_from_slice(tail);
            }
            Ok(Plan::Write {
                geometry: SemanticGeometry::Polyline {
                    points: kept_points,
                    bulges: kept_bulges,
                    closed,
                    width,
                },
                label: format!(
                    "Edit vertex STRAIGHTEN ({first} → {last}: {} interior vertex/vertices \
                     removed)",
                    last - first - 1
                ),
                deleted: Vec::new(),
            })
        }
        PeditVertexAction::Tangent {
            vertex,
            angle_degrees,
        } => {
            if !angle_degrees.is_finite() {
                return Err(invalid(
                    "vertex_invalid_point",
                    "the PEDIT EDIT TANGENT angle must be finite",
                ));
            }
            if vertex >= count {
                return Err(invalid(
                    "vertex_out_of_range",
                    format!("PEDIT EDIT TANGENT needs a vertex index in 0..{count}, got {vertex}"),
                ));
            }
            Err(unsupported(
                "vertex_tangent_unsupported",
                "this build's polyline model has no stored fit-tangent field; PEDIT EDIT TANGENT \
                 is a named capability gap",
            ))
        }
        PeditVertexAction::Width => Err(unsupported(
            "vertex_width_unsupported",
            "this build stores one uniform polyline width; the per-vertex start/end taper modeled \
             by PEDIT EDIT WIDTH is not represented, so the action is a named capability gap",
        )),
    }
}

/// The read-only `NEXT`/`PREVIOUS` report of the neighbouring vertex.
fn neighbour_report(
    points: &[Point3],
    from: usize,
    closed: bool,
    step: isize,
    action: &str,
) -> CadResult<Plan> {
    let count = points.len();
    if from >= count {
        return Err(invalid(
            "vertex_out_of_range",
            format!("PEDIT EDIT {action} needs a vertex index in 0..{count}, got {from}"),
        ));
    }
    let target = from as isize + step;
    let target = if target < 0 {
        if closed {
            count - 1
        } else {
            return Ok(vertex_report(from, action, count, "it is the first vertex"));
        }
    } else if target as usize >= count {
        if closed {
            0
        } else {
            return Ok(vertex_report(from, action, count, "it is the last vertex"));
        }
    } else {
        target as usize
    };
    Ok(vertex_report(
        target,
        action,
        count,
        "the vertex marker moves there",
    ))
}

/// The `pedit.vertex_report` no-op for one vertex.
fn vertex_report(index: usize, action: &str, count: usize, note: &str) -> Plan {
    Plan::Noop {
        code: "pedit.vertex_report",
        message: format!(
            "PEDIT EDIT {action}: vertex {index} of {count}; {note}. No geometry was written."
        ),
    }
}
