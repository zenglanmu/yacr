//! Neutral B-rep → record, **planar faces only**.
//!
//! This is the direction every boolean and mass-properties request needs: the
//! kernel only sees a record. It is deliberately narrow — a body whose faces are
//! all planes bounded by `Line` loops in one closed shell — because that is the
//! subset the neutral model and the record agree on *exactly*. Everything else
//! is refused with [`LIFT_REQUIRES_PLANAR`](crate::LIFT_REQUIRES_PLANAR) rather
//! than approximated; lifting curved shells needs the pcurves of the record and
//! the NURBS form of every face, which the neutral model does not carry until P2.
//!
//! # Canonical order
//!
//! The record requires entities in first-appearance order (upstream
//! `docs/native-brep.md`): shells, then faces walking regions → shells, then
//! edges walking faces → loops → oriented edges (backwards in a face whose first
//! side is reversed), then vertices walking edges start-then-end. This module
//! builds each list in exactly that walk, so the record validates without a
//! re-sorting pass. Edges are deduplicated by their **unordered** endpoint pair
//! and take their direction from the first use that created them; vertices are
//! deduplicated by exact bits.
//!
//! # pcurves
//!
//! `solidcore` stores every edge use's trim curve in the face's `(u, v)` and
//! integrates over that trim for area, centroid and inertia. Each use therefore
//! gets the **exact** `(u, v)` image of its line segment as a degree-1 spline
//! (exact for a straight segment on a plane), with the edge's interval `[0, 1]`
//! and running in the edge's own direction. Volume alone tolerates a placeholder
//! pcurve; the mass-properties area does not, and this was measured, not assumed.

use std::collections::HashMap;

use cad_domain::{BrepCurve, BrepData, BrepFace, BrepSurface, Point3};
use solidcore_record::brep::{
    Brep as Record, Curve, Curve2, Edge, EdgeUseRef, Face, Loop, OrientedEdge, OrientedFace,
    Region, Shell, Surface, Vertex, VERSION,
};

use super::surface;
use super::units::{bits, point, record_point};
use super::{dot, finite_point, is_axis, normalize, sub};
use crate::error::LiftRefusal;

/// Record tolerance (mm) used when a caller does not carry one: the value the
/// upstream record tests use and comfortably tighter than SMLib's default
/// intersection tolerance (`1e-5` mm).
pub(crate) const DEFAULT_TOLERANCE: f64 = 1e-7;

/// Neutral B-rep → record. Refuses a body this cut cannot express exactly.
pub(crate) fn to_record(brep: &BrepData, tolerance: f64) -> Result<Record, LiftRefusal> {
    if !tolerance.is_finite() || tolerance <= 0.0 || tolerance > 1e3 {
        return Err(LiftRefusal::unsupported(format!(
            "record tolerance must be finite and in (0, 1e3], got {tolerance}"
        )));
    }
    let placement = brep
        .placement
        .unwrap_or_else(cad_domain::BrepPlacement::identity);
    let baked = super::bake(brep, placement)?;
    let brep = &baked;

    if brep.shells.len() != 1 {
        return Err(LiftRefusal::unsupported(format!(
            "a body with {} shells cannot be lifted; P1 lifts exactly one closed face shell",
            brep.shells.len()
        )));
    }
    let shell = &brep.shells[0];
    if shell.faces.is_empty() {
        return Err(LiftRefusal::unsupported(
            "a shell without a face cannot be lifted",
        ));
    }

    let mut record = Record {
        version: VERSION,
        tolerance,
        regions: Vec::new(),
        shells: Vec::new(),
        faces: Vec::new(),
        edges: Vec::new(),
        vertices: Vec::new(),
    };
    let mut vertex_of: HashMap<[u64; 3], u32> = HashMap::new();
    let mut edge_of: HashMap<([u64; 3], [u64; 3]), u32> = HashMap::new();
    let mut oriented = Vec::with_capacity(shell.faces.len());

    for (index, face) in shell.faces.iter().enumerate() {
        let face_index = u32::try_from(index)
            .map_err(|_| LiftRefusal::unsupported("more faces than a record index can name"))?;
        let (surface, u_axis, v_axis, origin) = record_face_surface(face)?;
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
            let mut uses = vec![
                (
                    0u32,
                    false,
                    Point3 {
                        x: 0.0,
                        y: 0.0,
                        z: 0.0
                    },
                    Point3 {
                        x: 0.0,
                        y: 0.0,
                        z: 0.0
                    }
                );
                count
            ];
            for k in 0..count {
                // The canonical edge walk runs backwards through a loop whose
                // first side is reversed, so edge indices match the record's
                // first-appearance rule.
                let position = if forward { k } else { count - 1 - k };
                let Some(BrepCurve::Line { start, end }) = loop_.edges.get(position) else {
                    return Err(LiftRefusal::non_planar(format!(
                        "face {} has a boundary curve that is not a line ({}); only planar, \
                         line-bounded bodies are lifted",
                        face.id,
                        boundary_kind(&loop_.edges[position])
                    )));
                };
                if !finite_point(*start) || !finite_point(*end) {
                    return Err(LiftRefusal::unsupported(format!(
                        "face {} has a boundary point that is not finite",
                        face.id
                    )));
                }
                corners.push(*start);
                corners.push(*end);
                let (edge, edge_forward) = edge_use(
                    &mut record,
                    &mut vertex_of,
                    &mut edge_of,
                    (*start, *end),
                    face.id,
                )?;
                uses[position] = (edge, edge_forward, *start, *end);
            }
            let edges = uses
                .into_iter()
                .map(|(edge, forward, start, end)| {
                    // The pcurve runs the edge's own direction (min → max), not
                    // the loop's: `forward` already says which way the loop
                    // traverses it. For a planar face bounded by lines this is
                    // the exact `(u, v)` image of the 3D segment.
                    let (start, end) = if forward { (start, end) } else { (end, start) };
                    OrientedEdge {
                        edge,
                        forward,
                        pcurve: uv_line(origin, u_axis, v_axis, start, end),
                    }
                })
                .collect();
            loops.push(Loop::Edges { edges });
        }
        if loops.is_empty() {
            return Err(LiftRefusal::unsupported(format!(
                "face {} has no boundary loop",
                face.id
            )));
        }
        let domain = domain(origin, u_axis, v_axis, &corners);
        record.faces.push(Face {
            surface,
            nurbs: None,
            domain,
            loops,
        });
        oriented.push(OrientedFace {
            face: face_index,
            forward,
        });
    }

    record.shells.push(Shell::Faces {
        faces: oriented,
        wires: Vec::new(),
    });
    // A closed face shell separates two regions: it is the outer shell of the
    // bounded region inside it and listed by the infinite region around it.
    record.regions = vec![
        Region {
            shells: vec![0],
            void: false,
        },
        Region {
            shells: vec![0],
            void: false,
        },
    ];
    list_uses(&mut record);
    record.validate().map_err(|error| {
        LiftRefusal::unsupported(format!(
            "the lifted record is structurally invalid: {error}"
        ))
    })?;
    check_closed_shell(&record)?;
    check_loop_chain(&record)?;
    // Last: the kernel's own geometric agreement check (edge/vertex, pcurve
    // lifting, loop turn), which assumes a structurally valid record. This
    // catches a body whose topology passes `validate` but whose geometry does
    // not agree with it, before the kernel is asked to build a body from it.
    solidcore_ops::record_check::agreement(&record).map_err(|mismatch| {
        LiftRefusal::unsupported(format!(
            "the lifted record's geometry disagrees with its topology: {mismatch}"
        ))
    })?;
    Ok(record)
}

/// Every edge is used exactly twice, by the two faces traversing it in opposite
/// directions **in the shell's outward orientation**: the signature of a closed,
/// consistently oriented shell.
///
/// `Brep::validate` proves the `uses` list matches the walk; it cannot prove the
/// walk visits each edge twice. A sheet mapped as a solid (one use per edge) or a
/// face whose orientation was flipped (both uses traversing the same way) passes
/// structural validation and then produces a *plausible but wrong* body — the
/// measured symptom was an area of 40 instead of 52 — so it is refused here.
///
/// The direction compared is the shell's, not the stored loop's: a face used
/// backwards (`OrientedFace::forward == false`) stores its loop relative to its
/// natural normal, so its stored traverse is the *reverse* of the shell's. The
/// kernel's own record of a box minus a corner has exactly such faces, and their
/// shared edges carry equal `upper` flags while the body is perfectly valid — so
/// `upper` equality alone is **not** the invariant here. Traversal in the shell's
/// orientation is, and it is also what a negative volume would reflect.
fn check_closed_shell(record: &Record) -> Result<(), LiftRefusal> {
    let uses_of_shell = match record.shells.first() {
        Some(Shell::Faces { faces, .. }) => faces.as_slice(),
        _ => {
            return Err(LiftRefusal::unsupported(
                "the lifted record has no single face shell",
            ));
        }
    };
    for (index, edge) in record.edges.iter().enumerate() {
        let [first, second] = edge.uses.as_slice() else {
            return Err(LiftRefusal::unsupported(format!(
                "edge {index} is used {} time(s); a closed solid's edge is used exactly twice \
                 (an open sheet, or a face listed twice)",
                edge.uses.len()
            )));
        };
        match (
            shell_traverse(record, uses_of_shell, first),
            shell_traverse(record, uses_of_shell, second),
        ) {
            (Some(a), Some(b)) if a != b => {}
            (Some(_), Some(_)) => {
                return Err(LiftRefusal::unsupported(format!(
                    "edge {index} is traversed the same way by both faces: the shell is not \
                     consistently oriented (a reversed face?)"
                )));
            }
            _ => {
                return Err(LiftRefusal::unsupported(format!(
                    "edge {index} has a use whose face is missing from the shell or listed \
                     twice, so its shell orientation cannot be resolved"
                )));
            }
        }
    }
    Ok(())
}

/// Whether a use walks its edge from interval min to max **in the shell's
/// orientation**.
///
/// The stored traversal is flipped when the shell uses the face backwards, i.e.
/// the result is `stored XNOR facing` (`stored == facing`); the invariant checked
/// by [`check_closed_shell`] is that the two uses of an edge disagree (`a != b`).
fn shell_traverse(
    record: &Record,
    uses_of_shell: &[OrientedFace],
    use_: &EdgeUseRef,
) -> Option<bool> {
    let facing = shell_facing(uses_of_shell, use_.face)?;
    let stored = match record
        .faces
        .get(use_.face as usize)?
        .loops
        .get(use_.r#loop as usize)?
    {
        Loop::Edges { edges } => edges.get(use_.position as usize)?.forward,
        Loop::Vertex { .. } => return None,
    };
    Some(stored == facing)
}

/// The shell's facing for a face, looked up by the face's **record index**.
///
/// This module pushes one oriented face per body face in order, so the face index
/// also works as a position — but a second producer must not be able to rely on
/// that and silently read another face's facing, so the lookup matches the index
/// and refuses (returns `None`) when the face is missing or listed twice.
fn shell_facing(uses_of_shell: &[OrientedFace], face: u32) -> Option<bool> {
    let mut facing = None;
    for oriented in uses_of_shell
        .iter()
        .filter(|oriented| oriented.face == face)
    {
        if facing.replace(oriented.forward).is_some() {
            // Listed twice in one shell: a use cannot say which side it is on.
            return None;
        }
    }
    facing
}

/// Every loop chains end to end: edge `k`'s traversed end is edge `k + 1`'s
/// traversed start, and the last end is the first start — compared by **exact
/// bits**, so the loop is one closed cycle of the same points the curves carry.
///
/// The record's own validation already chains each loop by vertex index; this
/// repeats the requirement on the record this module just built, so a builder
/// mistake is refused with a geometric detail (which vertex ends where) instead
/// of only the structural message.
fn check_loop_chain(record: &Record) -> Result<(), LiftRefusal> {
    for (face_index, face) in record.faces.iter().enumerate() {
        for (loop_index, loop_) in face.loops.iter().enumerate() {
            let Loop::Edges { edges } = loop_ else {
                continue;
            };
            let mut previous: Option<u32> = None;
            let mut first_start: Option<u32> = None;
            for use_ in edges {
                let Some(edge) = record.edges.get(use_.edge as usize) else {
                    return Err(LiftRefusal::unsupported(format!(
                        "face {face_index} loop {loop_index} references missing edge {}",
                        use_.edge
                    )));
                };
                let (start, end) = if use_.forward {
                    (edge.vertices[0], edge.vertices[1])
                } else {
                    (edge.vertices[1], edge.vertices[0])
                };
                if let Some(previous) = previous {
                    if !same_vertex_point(record, previous, start) {
                        return Err(LiftRefusal::unsupported(format!(
                            "face {face_index} loop {loop_index} is not chained: vertex \
                             {previous} ends a use but the next starts at vertex {start}"
                        )));
                    }
                } else {
                    first_start = Some(start);
                }
                previous = Some(end);
            }
            match (previous, first_start) {
                (Some(last), Some(first)) if same_vertex_point(record, last, first) => {}
                (Some(last), Some(first)) => {
                    return Err(LiftRefusal::unsupported(format!(
                        "face {face_index} loop {loop_index} is open: it ends at vertex {last} \
                         and starts at vertex {first}"
                    )));
                }
                _ => {}
            }
        }
    }
    Ok(())
}

/// Two vertices carry bit-identical geometry (the record's exact-equality rule).
fn same_vertex_point(record: &Record, a: u32, b: u32) -> bool {
    match (
        record.vertices.get(a as usize),
        record.vertices.get(b as usize),
    ) {
        (Some(a), Some(b)) => bits(a.point) == bits(b.point),
        _ => false,
    }
}

/// The record plane plus the frame its domain is measured in.
fn record_face_surface(face: &BrepFace) -> Result<(Surface, Point3, Point3, Point3), LiftRefusal> {
    let BrepSurface::Plane {
        origin,
        normal,
        u_dir,
    } = &face.surface
    else {
        return Err(LiftRefusal::non_planar(format!(
            "face {} is a {} surface; only planar faces are lifted in P1",
            face.id,
            surface_kind(&face.surface)
        )));
    };
    let normal = normalize(*normal);
    if !is_axis(normal) || !finite_point(*origin) {
        return Err(LiftRefusal::placement(format!(
            "face {} has a degenerate plane frame",
            face.id
        )));
    }
    let surface = surface::record_plane(*origin, normal, *u_dir)?;
    let (u_axis, v_axis) = match &surface {
        Surface::Plane { u_axis, v_axis, .. } => (*u_axis, *v_axis),
        _ => {
            return Err(LiftRefusal::unsupported(
                "an internal plane record lost its kind",
            ))
        }
    };
    Ok((surface, point(u_axis), point(v_axis), *origin))
}

/// One oriented edge use, creating the edge and its vertices on first sight.
fn edge_use(
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
fn vertex(
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
fn domain(origin: Point3, u_axis: Point3, v_axis: Point3, corners: &[Point3]) -> [[f64; 2]; 2] {
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
fn list_uses(record: &mut Record) {
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

fn surface_kind(surface: &BrepSurface) -> &'static str {
    match surface {
        BrepSurface::Plane { .. } => "plane",
        BrepSurface::Sphere { .. } => "sphere",
        BrepSurface::Torus { .. } => "torus",
        BrepSurface::Cylinder { .. } => "cylinder",
        BrepSurface::Cone { .. } => "cone",
        BrepSurface::Unsupported { .. } => "unsupported",
    }
}

fn boundary_kind(curve: &BrepCurve) -> &'static str {
    match curve {
        BrepCurve::Line { .. } => "line",
        BrepCurve::Circle { .. } => "circle",
        BrepCurve::Unsupported { .. } => "unsupported",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::{LIFT_REQUIRES_PLANAR, LIFT_UNSUPPORTED, PLACEMENT_NOT_SUPPORTED};
    use crate::polyhedra;
    use crate::record::from_record;

    fn p(x: f64, y: f64, z: f64) -> Point3 {
        Point3 { x, y, z }
    }

    fn unit_box() -> BrepData {
        polyhedra::box_brep(p(0.0, 0.0, 0.0), 1.0, 1.0, 1.0)
    }

    #[test]
    fn a_planar_box_record_is_canonical_and_ordered() {
        let record = to_record(&unit_box(), DEFAULT_TOLERANCE).expect("record");
        assert_eq!(record.version, VERSION);
        assert_eq!(record.faces.len(), 6);
        assert_eq!(record.edges.len(), 12);
        assert_eq!(record.vertices.len(), 8);
        assert_eq!(record.regions.len(), 2);
        assert!(!record.regions[0].void && !record.regions[1].void);
        assert_eq!(record.regions[0].shells, vec![0]);
        assert_eq!(record.regions[1].shells, vec![0]);
        // Every edge is used twice, once per adjacent face, and its uses carry
        // the radial `upper = !forward` flag.
        for (index, edge) in record.edges.iter().enumerate() {
            assert_eq!(edge.uses.len(), 2, "closed shell edge used twice");
            assert_eq!(edge.interval, [0.0, 1.0]);
            for use_ in &edge.uses {
                let Loop::Edges { edges } =
                    &record.faces[use_.face as usize].loops[use_.r#loop as usize]
                else {
                    panic!("a vertex loop in an authored box");
                };
                let oriented = &edges[use_.position as usize];
                assert_eq!(oriented.edge as usize, index);
                assert_eq!(use_.upper, !oriented.forward);
            }
        }
        // No signed zeros survive: the record's exact-bit dedup would see two
        // vertices otherwise.
        for vertex in &record.vertices {
            for value in vertex.point {
                assert!(!value.is_sign_negative() || value != 0.0);
            }
        }
    }

    #[test]
    fn pcurves_are_the_exact_uv_image_of_each_edge() {
        let record = to_record(&unit_box(), DEFAULT_TOLERANCE).expect("record");
        for face in &record.faces {
            let Surface::Plane {
                origin,
                u_axis,
                v_axis,
            } = &face.surface
            else {
                panic!("authored box face is a plane");
            };
            let (origin, u, v) = (point(*origin), point(*u_axis), point(*v_axis));
            for loop_ in &face.loops {
                let Loop::Edges { edges } = loop_ else {
                    panic!("edge loop");
                };
                for use_ in edges {
                    let edge = &record.edges[use_.edge as usize];
                    let Curve::Line { start, end } = &edge.curve else {
                        panic!("authored box edge is a line");
                    };
                    let expected = [
                        [
                            dot(sub(point(*start), origin), u),
                            dot(sub(point(*start), origin), v),
                        ],
                        [
                            dot(sub(point(*end), origin), u),
                            dot(sub(point(*end), origin), v),
                        ],
                    ];
                    let Curve2::Spline { points, .. } = &use_.pcurve else {
                        panic!("a degree-1 spline pcurve");
                    };
                    for (got, want) in points.iter().zip(&expected) {
                        assert!((got[0] - want[0]).abs() < 1e-12);
                        assert!((got[1] - want[1]).abs() < 1e-12);
                    }
                }
            }
        }
    }

    #[test]
    fn a_round_trip_keeps_the_body() {
        let brep = unit_box();
        let record = to_record(&brep, DEFAULT_TOLERANCE).expect("record");
        let back = from_record(&record).expect("neutral body");
        assert_eq!(back.face_count(), brep.face_count());
        let again = to_record(&back, DEFAULT_TOLERANCE).expect("record again");
        assert_eq!(again.faces.len(), record.faces.len());
        assert_eq!(again.edges.len(), record.edges.len());
        assert_eq!(again.vertices.len(), record.vertices.len());
    }

    #[test]
    fn a_curved_face_is_refused_with_the_stable_planar_reason() {
        let body = solidcore_ops::make_cylinder([0.0, 0.0, 0.0], 1.0, 2.0).expect("cylinder");
        let (record, _) = body.to_record().expect("cylinder record");
        let brep = from_record(&record).expect("neutral cylinder");
        let refusal = to_record(&brep, DEFAULT_TOLERANCE).expect_err("curved lift refused");
        assert_eq!(refusal.code, LIFT_REQUIRES_PLANAR);
        assert!(refusal.detail.contains("cylinder"), "{}", refusal.detail);
    }

    #[test]
    fn a_circle_boundary_is_refused() {
        let mut brep = unit_box();
        brep.shells[0].faces[0].loops[0].edges[0] = cad_domain::BrepCurve::Circle {
            center: p(0.5, 0.5, 0.0),
            normal: p(0.0, 0.0, 1.0),
            u_dir: p(1.0, 0.0, 0.0),
            radius: 0.5,
        };
        let refusal = to_record(&brep, DEFAULT_TOLERANCE).expect_err("circle refused");
        assert_eq!(refusal.code, LIFT_REQUIRES_PLANAR);
        assert!(refusal.detail.contains("circle"), "{}", refusal.detail);
    }

    #[test]
    fn a_second_shell_is_refused() {
        let mut brep = unit_box();
        let shell = brep.shells[0].clone();
        brep.shells.push(shell);
        let refusal = to_record(&brep, DEFAULT_TOLERANCE).expect_err("two shells refused");
        assert_eq!(refusal.code, LIFT_UNSUPPORTED);
        assert!(refusal.detail.contains("2 shells"), "{}", refusal.detail);
    }

    #[test]
    fn a_placed_body_is_baked_into_the_record() {
        let mut brep = unit_box();
        brep.placement = Some(cad_domain::BrepPlacement {
            matrix: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            translation: p(10.0, 0.0, 0.0),
            scale: 2.0,
        });
        let record = to_record(&brep, DEFAULT_TOLERANCE).expect("placed body lifts");
        let min = record
            .vertices
            .iter()
            .map(|vertex| vertex.point[0])
            .fold(f64::INFINITY, f64::min);
        let max = record
            .vertices
            .iter()
            .map(|vertex| vertex.point[0])
            .fold(f64::NEG_INFINITY, f64::max);
        assert!((min - 10.0).abs() < 1e-12, "min {min}");
        assert!((max - 12.0).abs() < 1e-12, "max {max}");
    }

    #[test]
    fn a_negative_tolerance_is_refused() {
        let refusal = to_record(&unit_box(), 0.0).expect_err("zero tolerance refused");
        assert_eq!(refusal.code, LIFT_UNSUPPORTED);
        assert!(refusal.detail.contains("tolerance"), "{}", refusal.detail);
    }

    /// One table: each refusal site, and the one code it must report. This is the
    /// guard against code conflation — `solid.lift_requires_planar` is only for
    /// curved geometry, structural problems are `solid.lift_unsupported`, and a
    /// bad frame is `solid.placement_not_supported`.
    #[test]
    fn refusal_codes_are_not_conflated() {
        let mut cases: Vec<(&str, BrepData, &'static str)> = Vec::new();

        // (1) non-plane face: the two allowed `lift_requires_planar` sites.
        let body = solidcore_ops::make_cylinder([0.0, 0.0, 0.0], 1.0, 2.0).expect("cylinder");
        let (record, _) = body.to_record().expect("cylinder record");
        let cylinder = from_record(&record).expect("neutral cylinder");
        cases.push(("non-plane face", cylinder, LIFT_REQUIRES_PLANAR));

        let mut circle = unit_box();
        circle.shells[0].faces[0].loops[0].edges[0] = cad_domain::BrepCurve::Circle {
            center: p(0.5, 0.5, 0.0),
            normal: p(0.0, 0.0, 1.0),
            u_dir: p(1.0, 0.0, 0.0),
            radius: 0.5,
        };
        cases.push(("non-line boundary", circle, LIFT_REQUIRES_PLANAR));

        // (2) structural refusals.
        let mut reversed = unit_box();
        reversed.shells[0].faces[0].reversed = true;
        cases.push(("reversed face", reversed, LIFT_UNSUPPORTED));

        let mut duplicated = unit_box();
        let duplicate = duplicated.shells[0].faces[0].clone();
        duplicated.shells[0].faces.push(duplicate);
        cases.push(("duplicated face", duplicated, LIFT_UNSUPPORTED));

        // A loop written backwards (order reversed *and* each segment flipped):
        // still a closed cycle, but it winds the other way round the face.
        let mut reversed_loop = unit_box();
        let edges = &mut reversed_loop.shells[0].faces[0].loops[0].edges;
        *edges = edges
            .iter()
            .rev()
            .map(|curve| match curve {
                cad_domain::BrepCurve::Line { start, end } => cad_domain::BrepCurve::Line {
                    start: *end,
                    end: *start,
                },
                other => other.clone(),
            })
            .collect();
        cases.push(("reversed loop", reversed_loop, LIFT_UNSUPPORTED));

        let mut two_shells = unit_box();
        let shell = two_shells.shells[0].clone();
        two_shells.shells.push(shell);
        cases.push(("two shells", two_shells, LIFT_UNSUPPORTED));

        let mut sheet = unit_box();
        sheet.shells[0].faces.truncate(1);
        cases.push(("open sheet", sheet, LIFT_UNSUPPORTED));

        // (3) frame refusals: a plane normal cannot be zero.
        let mut degenerate = unit_box();
        degenerate.shells[0].faces[0].surface = BrepSurface::Plane {
            origin: p(0.0, 0.0, 0.0),
            normal: p(0.0, 0.0, 0.0),
            u_dir: p(1.0, 0.0, 0.0),
        };
        cases.push(("degenerate frame", degenerate, PLACEMENT_NOT_SUPPORTED));

        for (name, body, expected) in cases {
            let refusal = to_record(&body, DEFAULT_TOLERANCE).expect_err(name);
            assert_eq!(refusal.code, expected, "{name}: {refusal:?}");
            assert!(!refusal.detail.is_empty(), "{name}: empty detail");
        }

        // Tolerance is structural, not curved.
        let refusal = to_record(&unit_box(), 0.0).expect_err("tolerance");
        assert_eq!(refusal.code, LIFT_UNSUPPORTED);
    }
}
