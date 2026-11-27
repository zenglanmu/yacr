//! SketchUp construction shapes: rotated rectangle and pie.
//!
//! Both are three-point captures that produce **one closed straight
//! `AcDbPolyline`** on the active layer; neither needs a kernel, a schema change
//! or a parameter.
//!
//! * `RECTANGROTATED` (Rotated Rectangle): the first two clicks are one full
//!   edge (corner and its far end); the third click sets the width on the plane
//!   the three points span. The four corners are emitted counter-clockwise about
//!   that plane normal, so the row is a usable closed profile (Push/Pull,
//!   Follow Me, `AREA`).
//! * `PIE`: the centre, a point on the circle (radius), then a point that sets
//!   the end angle. The arc is sampled into straight segments
//!   ([`PIE_SEGMENTS_PER_TURN`]) and closed through the centre, so the result is
//!   a straight closed polyline (a SketchUp arc is faceted too).
//!
//! # Refusals
//!
//! `rectangrotated.requires_points` / `rectangrotated.degenerate` and
//! `pie.requires_points` / `pie.degenerate` (a zero edge, collinear clicks, a
//! zero radius or a zero sweep). Every refusal happens before the insert, so a
//! failed run bumps no revision and records no undo entry.

use super::*;
use crate::app_extrude::invalid;

/// How many arc segments one full turn of `PIE` is sampled into.
///
/// A SketchUp arc is itself a run of segments; 24 per turn keeps a quarter-pie
/// readable without filling the row with vertices.
pub const PIE_SEGMENTS_PER_TURN: usize = 24;

impl Application {
    /// `RECTANGROTATED`: one closed four-vertex rectangle on an arbitrary plane.
    ///
    /// Payload: `Points([corner, edge_end, width_point])`, the three clicks in
    /// capture order. The first two define the first edge; the third decides the
    /// plane (through all three points) and the width.
    pub(crate) fn rotated_rectangle(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let points = three_points(command, "rectangrotated.requires_points", "RECTANGROTATED")?;
        let [corner, edge_end, width_point] = points;
        let edge = sub(edge_end, corner);
        let length = length(edge);
        if length <= 1e-9 {
            return Err(invalid(
                "rectangrotated.degenerate",
                "the first click and the second click must define a non-zero edge",
            ));
        }
        let direction = mul(edge, 1.0 / length);
        let normal = normalize(cross(edge, sub(width_point, corner))).ok_or_else(|| {
            invalid(
                "rectangrotated.degenerate",
                "the three rotated-rectangle clicks are collinear, so they define no plane",
            )
        })?;
        let width_direction = cross(normal, direction);
        let width = dot(sub(width_point, corner), width_direction);
        if width.abs() <= 1e-9 {
            return Err(invalid(
                "rectangrotated.degenerate",
                "the third click lies on the first edge, so the rectangle has no width",
            ));
        }
        let corners = vec![
            corner,
            edge_end,
            add(edge_end, mul(width_direction, width)),
            add(corner, mul(width_direction, width)),
        ];
        let geometry = closed_polyline(corners);
        self.commit_new_entity(session, command, geometry, "create rotated rectangle")
    }

    /// `PIE`: one closed polyline of the centre plus a sampled arc.
    ///
    /// Payload: `Points([center, start, end])`. The radius is `|start - center|`;
    /// `end` only sets the end angle, and the sweep follows the shorter signed
    /// turn (a clockwise pair sweeps clockwise).
    pub(crate) fn pie(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let points = three_points(command, "pie.requires_points", "PIE")?;
        let [center, start, end] = points;
        let radius_vector = sub(start, center);
        let radius = length(radius_vector);
        if radius <= 1e-9 {
            return Err(invalid(
                "pie.degenerate",
                "the first click is the centre and the second must be on the arc, at a non-zero \
                 radius",
            ));
        }
        let u = mul(radius_vector, 1.0 / radius);
        let end_vector = sub(end, center);
        let normal = normalize(cross(radius_vector, end_vector)).ok_or_else(|| {
            invalid(
                "pie.degenerate",
                "the end click lies on the start radius, so the pie sweep is zero",
            )
        })?;
        let v = cross(normal, u);
        let sweep = dot(end_vector, v).atan2(dot(end_vector, u));
        if sweep.abs() <= 1e-9 {
            return Err(invalid("pie.degenerate", "the pie sweep is zero"));
        }
        let segments = ((sweep.abs() / std::f64::consts::TAU * PIE_SEGMENTS_PER_TURN as f64).ceil()
            as usize)
            .max(2);
        let mut ring = Vec::with_capacity(segments + 2);
        ring.push(center);
        for index in 0..=segments {
            let angle = sweep * index as f64 / segments as f64;
            ring.push(add(
                center,
                add(mul(u, radius * angle.cos()), mul(v, radius * angle.sin())),
            ));
        }
        // A full turn's last sample coincides with the first; a closed polyline
        // would then carry a duplicate vertex.
        if let (Some(first), Some(last)) = (ring.get(1), ring.last()) {
            if same_point(*first, *last) {
                ring.pop();
            }
        }
        let geometry = closed_polyline(ring);
        self.commit_new_entity(session, command, geometry, "create pie")
    }
}

/// The three captured points of a construction shape, or its named refusal.
fn three_points(command: &Command, code: &str, name: &str) -> CadResult<[Point3; 3]> {
    let CommandPayload::Points(points) = &command.payload else {
        return Err(invalid(
            code,
            format!("{name} takes a Points([first, second, third]) payload"),
        ));
    };
    if points.len() != 3 {
        return Err(invalid(
            code,
            format!("{name} takes exactly three clicks, got {}", points.len()),
        ));
    }
    Ok([points[0], points[1], points[2]])
}

/// One closed straight polyline (no bulges) from `points`.
fn closed_polyline(points: Vec<Point3>) -> SemanticGeometry {
    SemanticGeometry::Polyline {
        bulges: vec![0.0; points.len()],
        points,
        closed: true,
        width: 0.0,
    }
}

fn same_point(a: Point3, b: Point3) -> bool {
    let scale = 1.0_f64.max(length(a)).max(length(b));
    length(sub(a, b)) <= 1e-9 * scale
}

fn dot(a: Point3, b: Point3) -> f64 {
    a.x * b.x + a.y * b.y + a.z * b.z
}

fn cross(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.y * b.z - a.z * b.y,
        y: a.z * b.x - a.x * b.z,
        z: a.x * b.y - a.y * b.x,
    }
}

fn sub(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.x - b.x,
        y: a.y - b.y,
        z: a.z - b.z,
    }
}

fn add(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.x + b.x,
        y: a.y + b.y,
        z: a.z + b.z,
    }
}

fn mul(v: Point3, scale: f64) -> Point3 {
    Point3 {
        x: v.x * scale,
        y: v.y * scale,
        z: v.z * scale,
    }
}

fn length(v: Point3) -> f64 {
    v.x.hypot(v.y).hypot(v.z)
}

fn normalize(v: Point3) -> Option<Point3> {
    let size = length(v);
    (size.is_finite() && size > 1e-12).then(|| mul(v, 1.0 / size))
}
