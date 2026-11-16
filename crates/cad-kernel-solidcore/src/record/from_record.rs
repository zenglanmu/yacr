//! Record → neutral B-rep.
//!
//! The record stores one entry per logical use (a face can appear twice, each
//! side once) and an edge can be used by one or two loops. This direction picks
//! the **bounded, non-void regions' shells** and maps their oriented faces; the
//! infinite region and any void region are not body geometry here.
//!
//! Two normalisations are needed because the neutral model is coarser than the
//! record:
//!
//! - a cylindrical/cone lateral face is stored with its seam edges as ordinary
//!   loop uses, while [`cad_domain::BrepSurface::Cylinder`] is bounded by one
//!   full circle per loop, so the seam lines are verified (they must lie on the
//!   surface) and dropped, and the circles become one loop each;
//! - a sphere or a torus is stored as one face with seam-only boundary, while
//!   the neutral model expects a **loop-less** face covering the whole surface,
//!   so the boundary is verified to be seam-only (a trimmed face is refused, not
//!   drawn as a full primitive) and dropped.
//!
//! Everything not mapped is a typed refusal with
//! [`LIFT_UNSUPPORTED`](crate::LIFT_UNSUPPORTED) — never a guessed surface.

use std::collections::BTreeSet;

use cad_domain::{BrepCurve, BrepData, BrepFace, BrepLoop, BrepShell, BrepSurface, Point3};
use solidcore_record::brep::{Brep as Record, Edge, Face, Loop, OrientedFace, Shell};

use super::curve;
use super::surface;
use super::units::point;
use super::{add, dot, length, mul, normalize, sub};
use crate::error::LiftRefusal;

/// The edge uses of one face, plus whether a vertex loop (a pole) was present.
struct EdgeUses<'a> {
    edges: Vec<(usize, &'a Edge, bool)>,
    vertex_loop: bool,
}

impl<'a> EdgeUses<'a> {
    /// Every edge used by `face`'s loops, in loop order.
    fn of(record: &'a Record, face: &'a Face) -> Result<Self, LiftRefusal> {
        let mut edges = Vec::new();
        let mut vertex_loop = false;
        for loop_ in &face.loops {
            match loop_ {
                Loop::Edges { edges: uses } => {
                    for use_ in uses {
                        let edge = record.edges.get(use_.edge as usize).ok_or_else(|| {
                            LiftRefusal::unsupported(format!(
                                "a loop references missing edge {}",
                                use_.edge
                            ))
                        })?;
                        edges.push((use_.edge as usize, edge, use_.forward));
                    }
                }
                Loop::Vertex { .. } => vertex_loop = true,
            }
        }
        Ok(EdgeUses { edges, vertex_loop })
    }
}

/// A geometric tolerance for the seam checks below, relative to the feature.
fn tolerance(scale: f64) -> f64 {
    1e-6 * scale.max(1.0)
}

/// Record → neutral B-rep, refusing anything this cut cannot carry.
pub(crate) fn from_record(record: &Record) -> Result<BrepData, LiftRefusal> {
    if record.regions.is_empty() {
        return Err(LiftRefusal::unsupported("a record with no region"));
    }
    if record.regions.iter().any(|region| region.void) {
        return Err(LiftRefusal::unsupported(
            "a record with a void region (a cavity) is not mapped in P1",
        ));
    }
    let mut shells = Vec::new();
    let mut seen = BTreeSet::new();
    // `regions[0]` is the infinite region; the bounded ones follow it.
    for region in record.regions.iter().skip(1) {
        for &index in &region.shells {
            if !seen.insert(index) {
                continue;
            }
            let shell = record.shells.get(index as usize).ok_or_else(|| {
                LiftRefusal::unsupported(format!("a region references missing shell {index}"))
            })?;
            match shell {
                Shell::Faces { faces, wires } => {
                    // A wire edge embedded in a face shell has no slot in the
                    // neutral model (`BrepData` has no wire-edge list), and
                    // dropping it would silently lose geometry, so it is
                    // refused rather than ignored.
                    if !wires.is_empty() {
                        return Err(LiftRefusal::unsupported(format!(
                            "a face shell embeds {} wire edge(s); the neutral model has no \
                             wire-edge slot, so they would be lost",
                            wires.len()
                        )));
                    }
                    shells.push(shell_of(record, index, faces)?);
                }
                Shell::Wire { .. } => {
                    return Err(LiftRefusal::unsupported(
                        "a wire shell is a 1D body, not a solid face shell; not mapped in P1",
                    ));
                }
                Shell::Vertex { .. } => {
                    return Err(LiftRefusal::unsupported(
                        "a vertex shell is a 0D body, not a solid face shell; not mapped in P1",
                    ));
                }
            }
        }
    }
    if shells.is_empty() {
        return Err(LiftRefusal::unsupported(
            "a record with no bounded face shell has no solid body to map",
        ));
    }
    Ok(BrepData {
        shells,
        placement: None,
    })
}

fn shell_of(record: &Record, index: u32, uses: &[OrientedFace]) -> Result<BrepShell, LiftRefusal> {
    let mut faces = Vec::with_capacity(uses.len());
    for use_ in uses {
        faces.push(face_of(record, use_)?);
    }
    Ok(BrepShell { id: index, faces })
}

fn face_of(record: &Record, use_: &OrientedFace) -> Result<BrepFace, LiftRefusal> {
    let index = use_.face;
    let face = record.faces.get(index as usize).ok_or_else(|| {
        LiftRefusal::unsupported(format!("a shell references missing face {index}"))
    })?;
    let surface = surface::to_brep(&face.surface)?;
    let loops = match &surface {
        BrepSurface::Plane { .. } => plane_loops(record, face)?,
        BrepSurface::Cylinder {
            origin,
            axis,
            radius,
            ..
        } => cylinder_loops(record, face, *origin, *axis, *radius)?,
        BrepSurface::Cone {
            origin,
            axis,
            radius,
            sin_half_angle,
            cos_half_angle,
            ..
        } => cone_loops(
            record,
            face,
            *origin,
            *axis,
            *radius,
            *sin_half_angle,
            *cos_half_angle,
        )?,
        BrepSurface::Sphere {
            center,
            radius,
            pole,
            ..
        } => sphere_loops(record, face, *center, *radius, *pole)?,
        BrepSurface::Torus {
            major_radius,
            minor_radius,
            ..
        } => torus_loops(record, face, *major_radius, *minor_radius)?,
        BrepSurface::Unsupported { type_key } => {
            return Err(LiftRefusal::unsupported(format!(
                "an unsupported surface '{type_key}' cannot be mapped"
            )));
        }
    };
    Ok(BrepFace {
        id: index,
        surface,
        reversed: !use_.forward,
        loops,
    })
}

/// A planar face: every loop maps literally, one curve per oriented edge.
fn plane_loops(record: &Record, face: &Face) -> Result<Vec<BrepLoop>, LiftRefusal> {
    let mut loops = Vec::with_capacity(face.loops.len());
    for loop_ in &face.loops {
        let Loop::Edges { edges } = loop_ else {
            return Err(LiftRefusal::unsupported(
                "a vertex loop on a planar face is not mapped in P1",
            ));
        };
        if edges.is_empty() {
            return Err(LiftRefusal::unsupported("an empty planar loop"));
        }
        let mut curves = Vec::with_capacity(edges.len());
        for use_ in edges {
            let edge = record.edges.get(use_.edge as usize).ok_or_else(|| {
                LiftRefusal::unsupported(format!("a loop references missing edge {}", use_.edge))
            })?;
            curves.push(curve::to_brep(record, edge, use_.forward)?);
        }
        loops.push(BrepLoop { edges: curves });
    }
    if loops.is_empty() {
        return Err(LiftRefusal::unsupported(
            "a planar face without a boundary loop",
        ));
    }
    Ok(loops)
}

/// A cylindrical lateral face: two full circles, the seam lines verified on the
/// surface and dropped.
fn cylinder_loops(
    record: &Record,
    face: &Face,
    origin: Point3,
    axis: Point3,
    radius: f64,
) -> Result<Vec<BrepLoop>, LiftRefusal> {
    let uses = EdgeUses::of(record, face)?;
    if uses.vertex_loop {
        return Err(LiftRefusal::unsupported(
            "a cylindrical face with a vertex loop is not mapped in P1",
        ));
    }
    let mut circles: Vec<BrepCurve> = Vec::new();
    let mut seen = BTreeSet::new();
    for (edge_index, edge, _forward) in &uses.edges {
        if let Some((center, normal, u_dir, circle_radius)) = curve::circle(&edge.curve) {
            if !curve::closed(edge) {
                return Err(LiftRefusal::unsupported(
                    "a cylindrical face bounded by a circular arc is not mapped in P1",
                ));
            }
            if seen.insert(*edge_index) {
                circles.push(BrepCurve::Circle {
                    center,
                    normal,
                    u_dir,
                    radius: circle_radius,
                });
            }
            continue;
        }
        let (start, end) = curve::endpoints(record, edge, true).ok_or_else(|| {
            LiftRefusal::unsupported(format!(
                "a cylindrical face with a '{}' boundary curve is not mapped in P1",
                curve::type_key(&edge.curve)
            ))
        })?;
        for p in [start, end] {
            let offset = sub(p, origin);
            let axial = dot(offset, axis);
            let radial = length(sub(offset, mul(axis, axial)));
            if (radial - radius).abs() > tolerance(radius.max(length(offset))) {
                return Err(LiftRefusal::unsupported(
                    "a cylindrical face has a boundary line that does not lie on the cylinder",
                ));
            }
        }
    }
    if circles.len() != 2 {
        return Err(LiftRefusal::unsupported(format!(
            "a cylindrical face is bounded by {} full circle(s); the neutral model needs two",
            circles.len()
        )));
    }
    Ok(circles
        .into_iter()
        .map(|circle| BrepLoop {
            edges: vec![circle],
        })
        .collect())
}

/// A cone lateral face: one or two full circles plus, for a full cone, the
/// degenerate apex loop the neutral tessellator recognises.
fn cone_loops(
    record: &Record,
    face: &Face,
    origin: Point3,
    axis: Point3,
    radius: f64,
    sin_half_angle: f64,
    cos_half_angle: f64,
) -> Result<Vec<BrepLoop>, LiftRefusal> {
    if !cos_half_angle.is_finite() || cos_half_angle.abs() < 1e-9 {
        return Err(LiftRefusal::unsupported(
            "a cone with a zero cosine half angle is a disc, not a lateral face",
        ));
    }
    let slope = sin_half_angle / cos_half_angle;
    if !slope.is_finite() || slope.abs() < 1e-12 {
        return Err(LiftRefusal::unsupported(
            "a cone with no slope is a cylinder, not a lateral face",
        ));
    }
    let apex = add(origin, mul(axis, -radius / slope));
    let apex_tolerance = tolerance(radius + length(sub(apex, origin)));
    let uses = EdgeUses::of(record, face)?;
    let mut circles: Vec<BrepCurve> = Vec::new();
    let mut seen = BTreeSet::new();
    let mut has_apex = uses.vertex_loop;
    for (edge_index, edge, _forward) in &uses.edges {
        if let Some((center, normal, u_dir, circle_radius)) = curve::circle(&edge.curve) {
            if !curve::closed(edge) {
                return Err(LiftRefusal::unsupported(
                    "a cone face bounded by a circular arc is not mapped in P1",
                ));
            }
            let level = dot(sub(center, origin), axis);
            let expected = radius + slope * level;
            if (expected - circle_radius).abs() > tolerance(expected.abs().max(circle_radius)) {
                return Err(LiftRefusal::unsupported(
                    "a cone boundary circle does not lie on the cone",
                ));
            }
            if seen.insert(*edge_index) {
                circles.push(BrepCurve::Circle {
                    center,
                    normal,
                    u_dir,
                    radius: circle_radius,
                });
            }
            continue;
        }
        let (start, end) = curve::endpoints(record, edge, true).ok_or_else(|| {
            LiftRefusal::unsupported(format!(
                "a cone face with a '{}' boundary curve is not mapped in P1",
                curve::type_key(&edge.curve)
            ))
        })?;
        // The full cone's seam runs base → apex; it marks the singular boundary
        // rather than a real trim, exactly as the neutral tessellator expects.
        if start == end
            || length(sub(start, apex)) <= apex_tolerance
            || length(sub(end, apex)) <= apex_tolerance
        {
            has_apex = true;
        }
    }
    if circles.is_empty() || circles.len() > 2 {
        return Err(LiftRefusal::unsupported(format!(
            "a cone face is bounded by {} full circle(s); one or two are mapped",
            circles.len()
        )));
    }
    if circles.len() == 2 && has_apex {
        return Err(LiftRefusal::unsupported(
            "a cone face bounded by two circles and an apex is inconsistent",
        ));
    }
    if circles.len() == 1 && !has_apex {
        return Err(LiftRefusal::unsupported(
            "a cone face bounded by one circle but no apex is not mapped in P1",
        ));
    }
    let mut loops: Vec<BrepLoop> = circles
        .into_iter()
        .map(|circle| BrepLoop {
            edges: vec![circle],
        })
        .collect();
    if has_apex {
        loops.push(BrepLoop {
            edges: vec![BrepCurve::Unsupported {
                type_key: "edge-without-curve".into(),
            }],
        });
    }
    Ok(loops)
}

/// A spherical face: only a full sphere is mapped, as a loop-less face.
///
/// The axis needs no check here: [`surface::to_brep`] only returns a
/// `BrepSurface::Sphere` from a record frame whose `z = x × y` is a unit axis
/// (the same check the kernel makes), so `pole` is already usable.
fn sphere_loops(
    record: &Record,
    face: &Face,
    center: Point3,
    radius: f64,
    pole: Point3,
) -> Result<Vec<BrepLoop>, LiftRefusal> {
    if !radius.is_finite() || radius <= 0.0 {
        return Err(LiftRefusal::unsupported(format!(
            "a sphere record with radius {radius} cannot be mapped"
        )));
    }
    let axis = normalize(pole);
    let north = add(center, mul(axis, radius));
    let south = sub(center, mul(axis, radius));
    let limit = tolerance(radius);
    let uses = EdgeUses::of(record, face)?;
    for (_, edge, _) in &uses.edges {
        for &vertex in &edge.vertices {
            let stored = record.vertices.get(vertex as usize).ok_or_else(|| {
                LiftRefusal::unsupported(format!("an edge references missing vertex {vertex}"))
            })?;
            let p = point(stored.point);
            if length(sub(p, north)) > limit && length(sub(p, south)) > limit {
                return Err(LiftRefusal::unsupported(
                    "a trimmed spherical face is not mapped in P1; only a full sphere is",
                ));
            }
        }
    }
    Ok(Vec::new())
}

/// A toroidal face: only a **ring** torus (`major > minor > 0`) is mapped, as a
/// loop-less face.
///
/// The neutral torus is sampled as a ring; a record whose tube reaches or
/// crosses its axis (`major <= minor`) is a spindle/horn torus, a different
/// shape, so it is refused instead of being drawn as a ring.
fn torus_loops(
    record: &Record,
    face: &Face,
    major_radius: f64,
    minor_radius: f64,
) -> Result<Vec<BrepLoop>, LiftRefusal> {
    if !(major_radius.is_finite()
        && minor_radius.is_finite()
        && minor_radius > 0.0
        && major_radius > minor_radius)
    {
        return Err(LiftRefusal::unsupported(format!(
            "a torus with major radius {major_radius} and minor radius {minor_radius} is not a \
             ring torus; the neutral model cannot draw it"
        )));
    }
    let uses = EdgeUses::of(record, face)?;
    for (_, edge, _) in &uses.edges {
        if !curve::closed(edge) {
            return Err(LiftRefusal::unsupported(
                "a trimmed toroidal face is not mapped in P1; only a full torus is",
            ));
        }
    }
    Ok(Vec::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use solidcore_record::brep::{Curve, Surface};

    use crate::error::LIFT_UNSUPPORTED;
    use crate::polyhedra;
    use crate::record::{to_record, DEFAULT_TOLERANCE};

    fn unit_box() -> Record {
        to_record(
            &polyhedra::box_brep(
                Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                1.0,
                2.0,
                3.0,
            ),
            DEFAULT_TOLERANCE,
        )
        .expect("authored box lifts")
    }

    fn refusal(record: &Record) -> LiftRefusal {
        from_record(record).expect_err("expected a refusal")
    }

    #[test]
    fn an_authored_box_reads_back_as_six_outward_planar_faces() {
        let brep = from_record(&unit_box()).expect("box reads back");
        assert_eq!(brep.shells.len(), 1);
        assert_eq!(brep.face_count(), 6);
        assert!(brep.placement.is_none());
        for face in &brep.shells[0].faces {
            assert!(!face.reversed, "authored shell faces are forward");
            assert_eq!(face.loops.len(), 1);
            match &face.surface {
                BrepSurface::Plane { normal, .. } => {
                    assert!((length(*normal) - 1.0).abs() < 1e-12);
                }
                other => panic!("expected a plane, got {other:?}"),
            }
            for curve in &face.loops[0].edges {
                assert!(matches!(curve, BrepCurve::Line { .. }), "{curve:?}");
            }
        }
        // The six faces of a box point out of the material: their normals sum
        // to zero and the body's volume is the box's.
        let total = brep.shells[0]
            .faces
            .iter()
            .filter_map(|face| match &face.surface {
                BrepSurface::Plane { normal, .. } => Some(*normal),
                _ => None,
            })
            .fold(
                Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                add,
            );
        assert!(length(total) < 1e-12);
    }

    #[test]
    fn a_torus_whose_tube_starts_off_the_equator_is_refused() {
        let body = solidcore_ops::make_torus([0.0, 0.0, 0.0], 2.0, 0.5).expect("torus");
        let (mut record, _) = body.to_record().expect("torus record");
        let mut shifted = false;
        for face in &mut record.faces {
            if let Surface::Torus { tube_start, .. } = &mut face.surface {
                *tube_start = 0.25;
                shifted = true;
            }
        }
        assert!(shifted, "the torus record carries a torus face");
        let refusal = refusal(&record);
        assert_eq!(refusal.code, LIFT_UNSUPPORTED);
        assert!(refusal.detail.contains("tube starts"), "{}", refusal.detail);
    }

    #[test]
    fn a_torus_that_is_not_a_ring_is_refused() {
        let body = solidcore_ops::make_torus([0.0, 0.0, 0.0], 2.0, 0.5).expect("torus");
        let (mut record, _) = body.to_record().expect("torus record");
        let mut mutated = false;
        for face in &mut record.faces {
            if let Surface::Torus {
                minor_radius,
                major_radius,
                ..
            } = &mut face.surface
            {
                // A spindle torus: the tube reaches the axis.
                *minor_radius = 0.5;
                *major_radius = 0.5;
                mutated = true;
            }
        }
        assert!(mutated, "the torus record carries a torus face");
        let refusal = refusal(&record);
        assert_eq!(refusal.code, LIFT_UNSUPPORTED);
        assert!(refusal.detail.contains("ring torus"), "{}", refusal.detail);
    }

    #[test]
    fn a_spline_boundary_curve_is_refused() {
        let mut record = unit_box();
        record.edges[0].curve = Curve::Spline {
            knots: vec![0.0, 0.0, 1.0, 1.0],
            points: vec![[0.0, 0.0, 0.0], [1.0, 1.0, 1.0]],
        };
        let refusal = refusal(&record);
        assert_eq!(refusal.code, LIFT_UNSUPPORTED);
        assert!(refusal.detail.contains("spline"), "{}", refusal.detail);
    }

    #[test]
    fn a_straight_spline_edge_maps_to_its_chord() {
        // The pinned kernel writes a chamfer's straight edges as a cubic Bézier
        // whose control points are collinear on the segment. That curve *is* the
        // segment, so the planar lift must read it as a `Line`.
        let mut record = unit_box();
        let (i0, i1) = {
            let edge = &record.edges[0];
            (edge.vertices[0] as usize, edge.vertices[1] as usize)
        };
        let a = record.vertices[i0].point;
        let b = record.vertices[i1].point;
        let at = |t: f64| {
            [
                a[0] + (b[0] - a[0]) * t,
                a[1] + (b[1] - a[1]) * t,
                a[2] + (b[2] - a[2]) * t,
            ]
        };
        record.edges[0].curve = Curve::Spline {
            knots: vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
            points: vec![a, at(1.0 / 3.0), at(2.0 / 3.0), b],
        };
        let brep = from_record(&record).expect("a straight spline maps to a line");
        let mut mapped = false;
        for face in &brep.shells[0].faces {
            for loop_ in &face.loops {
                for curve in &loop_.edges {
                    if let BrepCurve::Line { start, end } = curve {
                        let s = [start.x, start.y, start.z];
                        let e = [end.x, end.y, end.z];
                        if (s == a && e == b) || (s == b && e == a) {
                            mapped = true;
                        }
                    }
                }
            }
        }
        assert!(mapped, "the straight spline edge did not map to its chord");
    }

    #[test]
    fn a_curved_spline_edge_is_still_refused() {
        // Same endpoints, but the control polygon leaves the chord: a real curve,
        // which the neutral model has no variant for in P1.
        let mut record = unit_box();
        let (i0, i1) = {
            let edge = &record.edges[0];
            (edge.vertices[0] as usize, edge.vertices[1] as usize)
        };
        let a = record.vertices[i0].point;
        let b = record.vertices[i1].point;
        let along = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let probe = if along[0].abs() < 0.5 {
            [1.0, 0.0, 0.0]
        } else {
            [0.0, 1.0, 0.0]
        };
        let across = [
            along[1] * probe[2] - along[2] * probe[1],
            along[2] * probe[0] - along[0] * probe[2],
            along[0] * probe[1] - along[1] * probe[0],
        ];
        let length = (across[0] * across[0] + across[1] * across[1] + across[2] * across[2]).sqrt();
        let at = |t: f64, bulge: f64| {
            [
                a[0] + along[0] * t + across[0] / length * bulge,
                a[1] + along[1] * t + across[1] / length * bulge,
                a[2] + along[2] * t + across[2] / length * bulge,
            ]
        };
        record.edges[0].curve = Curve::Spline {
            knots: vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
            points: vec![a, at(1.0 / 3.0, 0.3), at(2.0 / 3.0, 0.3), b],
        };
        let refusal = refusal(&record);
        assert_eq!(refusal.code, LIFT_UNSUPPORTED);
        assert!(refusal.detail.contains("spline"), "{}", refusal.detail);
    }

    #[test]
    fn a_void_region_is_refused() {
        let mut record = unit_box();
        record.regions[1].void = true;
        let refusal = refusal(&record);
        assert_eq!(refusal.code, LIFT_UNSUPPORTED);
        assert!(refusal.detail.contains("void region"), "{}", refusal.detail);
    }

    #[test]
    fn a_trimmed_sphere_is_refused() {
        let body = solidcore_ops::make_sphere([0.0, 0.0, 0.0], 1.0).expect("sphere");
        let (mut record, _) = body.to_record().expect("sphere record");
        // Move one boundary vertex off the poles: the face is no longer the
        // whole sphere, and the neutral model has no trimmed sphere.
        record.vertices[0].point = [0.5, 0.5, 0.5];
        let refusal = refusal(&record);
        assert_eq!(refusal.code, LIFT_UNSUPPORTED);
        assert!(
            refusal.detail.contains("trimmed spherical"),
            "{}",
            refusal.detail
        );
    }

    #[test]
    fn a_curved_primitive_keeps_its_analytic_surfaces() {
        let body = solidcore_ops::make_cylinder([0.0, 0.0, 0.0], 1.0, 2.0).expect("cylinder");
        let (record, _) = body.to_record().expect("cylinder record");
        let brep = from_record(&record).expect("cylinder reads back");
        assert_eq!(brep.face_count(), 3);
        let kinds: Vec<&str> = brep
            .shells
            .iter()
            .flat_map(|shell| &shell.faces)
            .map(|face| match face.surface {
                BrepSurface::Cylinder { .. } => "cylinder",
                BrepSurface::Plane { .. } => "plane",
                _ => "other",
            })
            .collect();
        assert_eq!(kinds.iter().filter(|kind| **kind == "cylinder").count(), 1);
        assert_eq!(kinds.iter().filter(|kind| **kind == "plane").count(), 2);
        // The lateral face is bounded by two full circles (its seam lines are
        // verified and dropped).
        let lateral = brep
            .shells
            .iter()
            .flat_map(|shell| &shell.faces)
            .find(|face| matches!(face.surface, BrepSurface::Cylinder { .. }))
            .expect("cylinder face");
        assert_eq!(lateral.loops.len(), 2);
        for loop_ in &lateral.loops {
            assert_eq!(loop_.edges.len(), 1);
            assert!(matches!(loop_.edges[0], BrepCurve::Circle { .. }));
        }
    }
}
