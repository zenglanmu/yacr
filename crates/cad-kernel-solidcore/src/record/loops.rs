//! Writing a face's boundary loops into the record: edges, vertices, pcurves.
//!
//! One use per oriented edge, in the record's canonical walk order (backwards
//! through a loop whose first side is reversed), with the edge and vertex records
//! created on first sight.
//!
//! The writer runs in two phases because a face's pcurves depend on what its
//! surface is:
//!
//! 1. [`write_loops`] writes the **geometry** — edges, vertices and the loop's
//!    oriented uses — and returns them as [`RawUse`]s. It needs no frame.
//! 2. the caller finishes the loops with [`plane_loops`] (the exact `(u, v)`
//!    image of a line segment, or the exact projection of an arc's own NURBS
//!    form) or [`surface_loops`] (every use's pcurve from
//!    `solidcore_ops::pcurve::pcurves_within`, which has exact closed forms only
//!    and fits nothing).
//!
//! A line use on a plane gets the exact degree-1 `(u, v)` spline of its segment;
//! an arc or a full circle gets the exact projection of its own NURBS form (see
//! [`super::arc`]). A curved face's uses get whatever the kernel's exact-pcurve
//! call proves; a miss is a named refusal ([`PCURVE_NOT_EXACT`]), never a fitted
//! or plane-assumed trim.

use std::collections::HashMap;

use cad_domain::{BrepCurve, BrepFace, Point3};
use solidcore_ops::pcurve::EdgeUse;
use solidcore_record::brep::{
    Brep as Record, Curve, Curve2, Edge, EdgeUseRef, Loop, OrientedEdge, Surface, Vertex,
};

use super::arc::{arc_pcurve, authored_segment};
use super::units::{bits, record_point};
use super::{dot, sub};
use crate::error::{LiftRefusal, PCURVE_NOT_EXACT};

/// A planar face's `(u, v)` frame, as the loop writer needs it: the plane's
/// origin and its two orthonormal in-plane axes.
#[derive(Debug, Clone, Copy)]
pub(super) struct PlaneFrame {
    pub origin: Point3,
    pub u_axis: Point3,
    pub v_axis: Point3,
}

/// One walked loop position before its pcurve is known: the record edge, how the
/// loop traverses it, and the authored curve's kind.
#[derive(Debug, Clone, Copy)]
pub(super) struct RawUse {
    pub(super) edge: u32,
    pub(super) forward: bool,
    pub(super) segment: Segment,
}

/// Write one face's boundary loops into `record`, without pcurves.
///
/// Returns the loops and the points the face's parameter domain must contain
/// (every line end and the arc bulges — a chord estimate would cut an arc).
pub(super) fn write_loops(
    record: &mut Record,
    vertex_of: &mut HashMap<[u64; 3], u32>,
    edge_of: &mut HashMap<([u64; 3], [u64; 3]), u32>,
    arcs: &mut Vec<ArcEdge>,
    circles: &mut Vec<CircleEdge>,
    face: &BrepFace,
) -> Result<(Vec<Vec<RawUse>>, Vec<Point3>), LiftRefusal> {
    let forward = !face.reversed;
    let mut corners = Vec::new();
    let mut loops = Vec::with_capacity(face.loops.len());
    for loop_ in &face.loops {
        if loop_.edges.is_empty() {
            return Err(LiftRefusal::unsupported(format!(
                "face {} has an empty boundary loop",
                face.id
            )));
        }
        let count = loop_.edges.len();
        let mut uses: Vec<Option<RawUse>> = vec![None; count];
        for k in 0..count {
            // The canonical edge walk runs backwards through a loop whose first
            // side is reversed, so edge indices match the record's
            // first-appearance rule.
            let position = if forward { k } else { count - 1 - k };
            let authored = authored_segment(
                record,
                vertex_of,
                edge_of,
                arcs,
                circles,
                &loop_.edges[position],
                face.id,
            )?;
            corners.extend(authored.corners());
            uses[position] = Some(RawUse {
                edge: authored.edge,
                forward: authored.forward,
                segment: authored.segment,
            });
        }
        loops.push(
            uses.into_iter()
                .map(|entry| entry.expect("every loop position is filled by the walk above"))
                .collect(),
        );
    }
    if loops.is_empty() {
        return Err(LiftRefusal::unsupported(format!(
            "face {} has no boundary loop",
            face.id
        )));
    }
    Ok((loops, corners))
}

/// Finish a **planar** face's loops: the exact `(u, v)` image of each use.
///
/// A line use is the exact degree-1 spline of its segment (the projection is
/// affine); an arc or a full circle is the exact projection of the edge's own
/// NURBS form, on the edge's interval and in the edge's direction. A pcurve that
/// does not match the loop would report a wrong trimmed area even when the volume
/// integral agreed, which is why neither is a placeholder.
pub(super) fn plane_loops(
    record: &Record,
    raw: &[Vec<RawUse>],
    frame: PlaneFrame,
) -> Result<Vec<Loop>, LiftRefusal> {
    let PlaneFrame {
        origin,
        u_axis,
        v_axis,
    } = frame;
    let mut loops = Vec::with_capacity(raw.len());
    for uses in raw {
        let mut edges = Vec::with_capacity(uses.len());
        for use_ in uses {
            // The pcurve runs the edge's own direction (min → max), not the
            // loop's: `forward` already says which way the loop traverses it.
            let pcurve = match use_.segment {
                Segment::Line { start, end } => {
                    let (start, end) = if use_.forward {
                        (start, end)
                    } else {
                        (end, start)
                    };
                    uv_line(origin, u_axis, v_axis, start, end)
                }
                Segment::Arc | Segment::Circle => {
                    arc_pcurve(record, use_.edge, origin, u_axis, v_axis)?
                }
            };
            edges.push(OrientedEdge {
                edge: use_.edge,
                forward: use_.forward,
                pcurve,
            });
        }
        loops.push(Loop::Edges { edges });
    }
    Ok(loops)
}

/// Finish a **non-planar** face's loops: every use's pcurve from the kernel's
/// exact-pcurve call, in the face's surface parameters.
///
/// `domain` is the face's box **in the same parameters** (the form's, for a
/// labelled curved face), which is what lets the call place a seam use a whole
/// period from its twin. Nothing is fitted: a use with no exact closed form is
/// [`PCURVE_NOT_EXACT`], and the face is refused by name rather than trimmed with
/// an approximation.
pub(super) fn surface_loops(
    record: &Record,
    raw: &[Vec<RawUse>],
    face: u32,
    surface: &Surface,
    parameters: &Surface,
    domain: [[f64; 2]; 2],
    tolerance: f64,
) -> Result<Vec<Loop>, LiftRefusal> {
    let mut uses = Vec::with_capacity(raw.len());
    for (loop_index, loop_) in raw.iter().enumerate() {
        let mut list = Vec::with_capacity(loop_.len());
        for use_ in loop_ {
            let edge = record.edges.get(use_.edge as usize).ok_or_else(|| {
                LiftRefusal::unsupported(format!(
                    "loop {loop_index} references missing edge {}",
                    use_.edge
                ))
            })?;
            list.push(EdgeUse {
                edge: use_.edge,
                curve: &edge.curve,
                interval: edge.interval,
                forward: use_.forward,
                tolerance: edge.tolerance,
                given: None,
            });
        }
        uses.push(list);
    }
    let pcurves = solidcore_ops::pcurve::pcurves_within(surface, &uses, tolerance, Some(domain))
        .map_err(|error| {
            LiftRefusal::new(
                PCURVE_NOT_EXACT,
                format!("no exact pcurve for this face's boundary in its surface: {error}"),
            )
        })?;
    verify_pcurves(face, record, raw, parameters, &pcurves)?;
    if pcurves.len() != raw.len() {
        return Err(LiftRefusal::new(
            PCURVE_NOT_EXACT,
            format!(
                "the exact-pcurve call answered {} loop(s) for {}",
                pcurves.len(),
                raw.len()
            ),
        ));
    }
    let mut loops = Vec::with_capacity(raw.len());
    for (loop_, pcurves) in raw.iter().zip(pcurves) {
        if pcurves.len() != loop_.len() {
            return Err(LiftRefusal::unsupported(format!(
                "the exact-pcurve call answered {} use(s) for a loop of {}",
                pcurves.len(),
                loop_.len()
            )));
        }
        loops.push(Loop::Edges {
            edges: loop_
                .iter()
                .zip(pcurves)
                .map(|(use_, pcurve)| OrientedEdge {
                    edge: use_.edge,
                    forward: use_.forward,
                    pcurve,
                })
                .collect(),
        });
    }
    Ok(loops)
}

/// Prove that every use's pcurve is the exact image of its edge, **inside** the
/// span, in the face's own parameters.
///
/// `solidcore`'s record check samples the ends and the middle; on a quarter-aligned
/// span the middle lands on a grid half-integer, where two different
/// parameterisations can agree by construction, so that check alone cannot tell an
/// exact trim from one that is a rounding-wedge off. The closed-form pcurves this
/// module writes come out of the kernel in the **label's** parameters while the
/// record keeps them in the face's (the form's); the two coincide when the
/// boundary lands on the shared quarter grid and diverge between the grid lines
/// otherwise. This samples four interior points as well, and a miss is
/// [`PCURVE_NOT_EXACT`] by name rather than the record's structural "geometry
/// disagrees with its topology".
fn verify_pcurves(
    face: u32,
    record: &Record,
    raw: &[Vec<RawUse>],
    parameters: &Surface,
    pcurves: &[Vec<Curve2>],
) -> Result<(), LiftRefusal> {
    let kernel = match solidcore_convert::surface(parameters) {
        Ok(kernel) => kernel,
        // A surface this crate cannot evaluate is not something to certify here;
        // the kernel's own check owns it.
        Err(_) => return Ok(()),
    };
    let Ok(surface) = solidcore::eval::SurfaceEval::new(&kernel) else {
        return Ok(());
    };
    // The same relative bound `solidcore` uses for "exact": rounding of the
    // geometry's own size, not the record's tolerance.
    const EXACT: f64 = 1e-9;
    for (loop_index, (uses, pcurves)) in raw.iter().zip(pcurves).enumerate() {
        for (position, (use_, pcurve)) in uses.iter().zip(pcurves).enumerate() {
            let Some(edge) = record.edges.get(use_.edge as usize) else {
                continue;
            };
            let Ok(kernel) = solidcore_convert::curve(edge.parameters()) else {
                continue;
            };
            let Ok(curve) = solidcore::eval::CurveEval::new(&kernel) else {
                continue;
            };
            let [t0, t1] = edge.interval;
            for step in [0.0, 0.125, 0.375, 0.625, 0.875, 1.0] {
                let t = t0 + (t1 - t0) * step;
                let (Some(uv), Some(on_edge)) = (
                    solidcore_ops::pcurve::eval(pcurve, t),
                    solidcore::eval::CurveEval::point(&curve, t),
                ) else {
                    continue;
                };
                let Some(on_surface) = solidcore::eval::SurfaceEval::point(&surface, uv) else {
                    return Err(LiftRefusal::new(
                        PCURVE_NOT_EXACT,
                        format!(
                            "face {face} loop {loop_index} use {position}: the pcurve leaves its \
                             surface at {uv:?}"
                        ),
                    ));
                };
                let gap = ((on_surface.x - on_edge.x).powi(2)
                    + (on_surface.y - on_edge.y).powi(2)
                    + (on_surface.z - on_edge.z).powi(2))
                .sqrt();
                let scale = 1.0 + on_edge.x.abs().max(on_edge.y.abs()).max(on_edge.z.abs());
                if gap > EXACT * scale {
                    return Err(LiftRefusal::new(
                        PCURVE_NOT_EXACT,
                        format!(
                            "face {face} loop {loop_index} use {position}: the exact pcurve of this \
                             boundary in the face's parameters is {gap} off its edge at t={t} \
                             (bound {}) — an exact closed form exists only where the boundary \
                             shares the face's quarter grid",
                            EXACT * scale
                        ),
                    ));
                }
            }
        }
    }
    Ok(())
}

/// One oriented edge use, creating the edge and its vertices on first sight.
pub(super) fn edge_use(
    record: &mut Record,
    vertex_of: &mut HashMap<[u64; 3], u32>,
    edge_of: &mut HashMap<([u64; 3], [u64; 3]), u32>,
    (start, end): (Point3, Point3),
    face: u32,
) -> Result<(u32, bool), LiftRefusal> {
    let a = bits(record_point(start));
    let z = bits(record_point(end));
    if a == z {
        return Err(LiftRefusal::unsupported(format!(
            "face {face} has a boundary edge with coincident endpoints; the record has no \
             zero-length edges"
        )));
    }
    if let Some(&edge) = edge_of.get(&(a, z)) {
        return Ok((edge, true));
    }
    if let Some(&edge) = edge_of.get(&(z, a)) {
        return Ok((edge, false));
    }
    let first = vertex(record, vertex_of, a, start);
    let second = vertex(record, vertex_of, z, end);
    let tolerance = record.tolerance;
    record.edges.push(Edge {
        curve: Curve::Line {
            start: record_point(start),
            end: record_point(end),
        },
        nurbs: None,
        interval: [0.0, 1.0],
        vertices: [first, second],
        tolerance,
        uses: Vec::new(),
    });
    let edge = u32::try_from(record.edges.len())
        .map_err(|_| LiftRefusal::unsupported("more edges than a record index can name"))?
        - 1;
    edge_of.insert((a, z), edge);
    Ok((edge, true))
}

/// The record vertex for `point`, created on first sight (exact-bit dedup).
pub(super) fn vertex(
    record: &mut Record,
    vertex_of: &mut HashMap<[u64; 3], u32>,
    key: [u64; 3],
    point: Point3,
) -> u32 {
    if let Some(&index) = vertex_of.get(&key) {
        return index;
    }
    let tolerance = record.tolerance;
    record.vertices.push(Vertex {
        point: record_point(point),
        tolerance,
    });
    // A body's faces are bounded by the record item budget, so the index fits.
    let index = (record.vertices.len() as u32).saturating_sub(1);
    vertex_of.insert(key, index);
    index
}

/// The reader's marker for a **full cone's degenerate apex loop**: the whole
/// boundary is one point, so there is no curve to name. The writer turns it back
/// into the apex vertex a cone band ends at (see `record::curved`), because the
/// record has no way to write "a loop of no edge".
pub(super) const APEX_WITHOUT_CURVE: &str = "edge-without-curve";

/// One authored boundary curve, as the loop writer needs it after the record
/// edge exists.
#[derive(Debug, Clone, Copy)]
pub(super) enum Segment {
    Line {
        start: Point3,
        end: Point3,
    },
    /// A circular arc; its pcurve is read back from the record edge (whose frame
    /// fixes the parameterisation both uses share).
    Arc,
    /// A full circle; its pcurve is read back from the record edge the same way.
    Circle,
}

/// One walked loop position: the record edge, how the loop traverses it, the
/// authored curve, and the points its face domain must contain.
pub(super) struct Authored {
    pub(super) edge: u32,
    pub(super) forward: bool,
    pub(super) segment: Segment,
    pub(super) domain_samples: Vec<Point3>,
}

impl Authored {
    fn corners(&self) -> impl Iterator<Item = Point3> + '_ {
        self.domain_samples.iter().copied()
    }
}

/// One record edge that carries an arc, identified by the arc's **point set**.
///
/// Two uses of one arc (the two faces sharing it) must become one record edge, or
/// the closed-shell check reports each use once. The point set is the right
/// identity because a reversed traversal of the same arc reproduces its centre,
/// radius and midpoint exactly — the midpoint is the same geometric point
/// whatever direction the arc is walked in.
pub(super) struct ArcEdge {
    pub(super) center: Point3,
    pub(super) radius: f64,
    pub(super) midpoint: Point3,
    pub(super) start: Point3,
    pub(super) end: Point3,
    pub(super) edge: u32,
}

/// One record edge that carries a **full circle**, identified by its circle.
///
/// A closed circle edge has one vertex (its seam point), so a reversed traversal
/// is not distinguishable by endpoints; the two faces that share it meet it at
/// that one point. The seam point is the circle's own `u_dir` radius direction,
/// which is also the point a cap's loop starts from, so both uses name it.
pub(super) struct CircleEdge {
    pub(super) center: Point3,
    pub(super) normal: Point3,
    pub(super) radius: f64,
    pub(super) edge: u32,
}

/// A line segment in the face's `(u, v)`, as the exact degree-1 spline the
/// record asks for.
///
/// This is not a placeholder: the modeler measures area/centroid/inertia by
/// integrating over trimmed faces in surface parameters, so a pcurve that does
/// not match the loop would report a zero-area (or wrongly-trimmed) face even
/// though the volume integral still agreed.
fn uv_line(origin: Point3, u_axis: Point3, v_axis: Point3, start: Point3, end: Point3) -> Curve2 {
    let uv = |p: Point3| {
        let offset = sub(p, origin);
        [dot(offset, u_axis), dot(offset, v_axis)]
    };
    Curve2::Spline {
        knots: vec![0.0, 0.0, 1.0, 1.0],
        points: vec![uv(start), uv(end)],
    }
}

/// The modelled rectangle in the plane's `(u, v)`, around the loop with padding.
pub(super) fn domain(
    origin: Point3,
    u_axis: Point3,
    v_axis: Point3,
    corners: &[Point3],
) -> [[f64; 2]; 2] {
    let (mut u_min, mut u_max) = (f64::INFINITY, f64::NEG_INFINITY);
    let (mut v_min, mut v_max) = (f64::INFINITY, f64::NEG_INFINITY);
    for corner in corners {
        let offset = sub(*corner, origin);
        let u = dot(offset, u_axis);
        let v = dot(offset, v_axis);
        u_min = u_min.min(u);
        u_max = u_max.max(u);
        v_min = v_min.min(v);
        v_max = v_max.max(v);
    }
    if !u_min.is_finite() || !u_max.is_finite() || !v_min.is_finite() || !v_max.is_finite() {
        // No usable corner (unreachable: loops are non-empty and finite); a unit
        // square keeps the record structurally valid rather than emitting NaN.
        return [[-1.0, 1.0], [-1.0, 1.0]];
    }
    let extent = (u_max - u_min).max(v_max - v_min).max(1e-9);
    let pad = (extent * 0.05).max(1e-6);
    [[u_min - pad, u_max + pad], [v_min - pad, v_max + pad]]
}

/// Fill each edge's list of uses exactly as the record's radial cycle expects:
/// every use of every loop, in walk order, with `upper = !forward`.
pub(super) fn list_uses(record: &mut Record) {
    for edge in &mut record.edges {
        edge.uses.clear();
    }
    for (face_index, face) in record.faces.iter().enumerate() {
        for (loop_index, loop_) in face.loops.iter().enumerate() {
            if let Loop::Edges { edges } = loop_ {
                for (position, use_) in edges.iter().enumerate() {
                    let target = &mut record.edges[use_.edge as usize];
                    target.uses.push(EdgeUseRef {
                        face: face_index as u32,
                        r#loop: loop_index as u32,
                        position: position as u32,
                        upper: !use_.forward,
                    });
                }
            }
        }
    }
}

pub(super) fn boundary_kind(curve: &BrepCurve) -> &'static str {
    match curve {
        BrepCurve::Line { .. } => "line",
        BrepCurve::Circle { .. } => "circle",
        BrepCurve::Arc { .. } => "arc",
        BrepCurve::Unsupported { .. } => "unsupported",
    }
}
