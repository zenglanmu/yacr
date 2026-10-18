//! Circular cone lateral faces (full cones and frusta) and the exact
//! boundary-ring stitcher that keeps them watertight.

use cad_domain::Point3;

use super::math::{add, dot, finite, frame_from, length, mul, normalize, sub};
use super::tessellate::{face_normal, sample_loop, FaceFailure, FaceMesh};
use super::types::{BrepCurve, BrepFace};

/// The exact boundary a cone side face was authored with: a full circle at an
/// axial level, sampled with the very same routine the planar cap uses, so the
/// two faces weld without a seam.
struct ConeRing {
    points: Vec<Point3>,
    level: f64,
}

/// Tessellate a circular cone's lateral face.
///
/// A full cone is bounded by one base circle and a degenerate apex loop; a
/// truncated cone (frustum) by two full circles. The lateral surface of a cone
/// is developable, so the *slope* direction needs no subdivision: sampling the
/// boundary circles (the only curved direction) and joining them along the
/// generators is enough. Boundary rings are taken from [`sample_loop`] verbatim
/// so they coincide exactly with the adjacent planar cap's ring, which is what
/// keeps the shell watertight.
#[allow(clippy::too_many_arguments)]
pub(super) fn tessellate_cone(
    face: &BrepFace,
    origin: Point3,
    axis: Point3,
    ref_dir: Point3,
    ref_radius: f64,
    sin_half_angle: f64,
    cos_half_angle: f64,
    linear_deflection: f64,
    angular_deflection: f64,
) -> Result<FaceMesh, FaceFailure> {
    if !ref_radius.is_finite() || ref_radius <= 0.0 {
        return Err(FaceFailure::Degenerate(
            "cone has no reference radius".into(),
        ));
    }
    if !sin_half_angle.is_finite() || !cos_half_angle.is_finite() {
        return Err(FaceFailure::Degenerate(
            "cone half-angle is not finite".into(),
        ));
    }
    // A cone with cos(half-angle) == 0 is a plane (flat disc), not a lateral
    // surface; it cannot be sampled from this parameterisation.
    if cos_half_angle.abs() < 1e-9 {
        return Err(FaceFailure::UnsupportedSurface(
            "cone-surface with a zero cosine half-angle".into(),
        ));
    }
    let slope = sin_half_angle / cos_half_angle;
    if !slope.is_finite() || slope.abs() < 1e-12 {
        // slope 0 is a cylinder; route it to the cylinder path instead of
        // silently accepting a degenerate cone.
        return Err(FaceFailure::UnsupportedSurface(
            "cone-surface with no slope".into(),
        ));
    }
    let Some((_, _, z)) = frame_from(axis, ref_dir) else {
        return Err(FaceFailure::Degenerate("cone frame is degenerate".into()));
    };

    // Partition the boundary loops: full circles become rings, the degenerate
    // apex singularity is remembered, and anything else is honestly refused.
    let mut rings: Vec<ConeRing> = Vec::new();
    let mut has_apex = false;
    for loop_ in &face.loops {
        if loop_.edges.len() == 1 {
            match &loop_.edges[0] {
                BrepCurve::Circle { center, radius, .. } => {
                    if !radius.is_finite() || *radius <= 0.0 {
                        return Err(FaceFailure::Degenerate(
                            "cone boundary has no radius".into(),
                        ));
                    }
                    let level = dot(sub(*center, origin), z);
                    if !level.is_finite() {
                        return Err(FaceFailure::Degenerate(
                            "cone boundary level is not finite".into(),
                        ));
                    }
                    // The boundary circle must lie on the cone surface, or the
                    // face is not a cone band and must not be guessed.
                    let expected = ref_radius + slope * level;
                    let scale = expected.abs().max(*radius).max(1e-12);
                    if (expected - *radius).abs() > 1e-6 * scale {
                        return Err(FaceFailure::UnsupportedSurface(
                            "cone boundary circle does not lie on the cone".into(),
                        ));
                    }
                    let points = sample_loop(loop_, linear_deflection, angular_deflection)?;
                    if points.len() < 3 {
                        return Err(FaceFailure::Degenerate(
                            "cone boundary sampled too few points".into(),
                        ));
                    }
                    rings.push(ConeRing { points, level });
                }
                BrepCurve::Unsupported { type_key } if type_key == "edge-without-curve" => {
                    has_apex = true;
                }
                BrepCurve::Line { start, end }
                    if dot(sub(*start, *end), sub(*start, *end)) <= 1e-18 =>
                {
                    has_apex = true;
                }
                BrepCurve::Unsupported { type_key } => {
                    return Err(FaceFailure::UnsupportedCurve(type_key.clone()));
                }
                BrepCurve::Line { .. } => {
                    return Err(FaceFailure::UnsupportedSurface(
                        "cone boundary is not a full circle".into(),
                    ));
                }
            }
        } else {
            return Err(FaceFailure::UnsupportedSurface(
                "cone face has a non-circular boundary loop".into(),
            ));
        }
    }

    if rings.is_empty() || rings.len() > 2 {
        return Err(FaceFailure::UnsupportedSurface(
            "cone face is not bounded by one or two full circles".into(),
        ));
    }
    // Apex is the axial level where the cross-section radius vanishes.
    let apex_level = -ref_radius / slope;
    if !apex_level.is_finite() {
        return Err(FaceFailure::Degenerate(
            "cone apex level is not finite".into(),
        ));
    }
    if rings.len() == 1 && !has_apex {
        return Err(FaceFailure::UnsupportedSurface(
            "cone face has a single circle but no apex boundary".into(),
        ));
    }
    if rings.len() == 2 && has_apex {
        return Err(FaceFailure::UnsupportedSurface(
            "cone face is bounded by two circles and an apex".into(),
        ));
    }

    let normal_at = |p: Point3| -> Point3 {
        let axial = dot(sub(p, origin), z);
        let radial = sub(sub(p, origin), mul(z, axial));
        let mut n = normalize(sub(radial, mul(z, slope)));
        if length(n) < 0.5 {
            // At the apex the radial part vanishes; the limit normal is axial.
            n = normalize(mul(z, -slope));
        }
        face_normal(n, face.reversed)
    };

    let mut out = FaceMesh::empty();
    let mut max_radius = 0.0f64;
    let mut max_segments = 0usize;
    for ring in &rings {
        max_segments = max_segments.max(ring.points.len());
        for p in &ring.points {
            let axial = dot(sub(*p, origin), z);
            max_radius = max_radius.max(length(sub(sub(*p, origin), mul(z, axial))));
        }
    }

    if rings.len() == 1 {
        // Full cone: fan the base ring to the single apex point.
        let ring = &rings[0];
        let base = out.vertices.len() as u32;
        for p in &ring.points {
            out.vertices.push(*p);
            out.normals.push(normal_at(*p));
        }
        let apex = add(origin, mul(z, apex_level));
        if !finite(apex) {
            return Err(FaceFailure::Degenerate("cone apex is not finite".into()));
        }
        let apex_index = out.vertices.len() as u32;
        out.vertices.push(apex);
        out.normals.push(normal_at(apex));
        let n = ring.points.len();
        for i in 0..n {
            let a = base + i as u32;
            let b = base + ((i + 1) % n) as u32;
            out.triangles.push([a, b, apex_index]);
        }
    } else {
        // Frustum: stitch the two exact boundary rings together.
        let (a, b) = (&rings[0], &rings[1]);
        if (a.level - b.level).abs() <= 1e-12 {
            return Err(FaceFailure::Degenerate(
                "cone boundaries share an axial level".into(),
            ));
        }
        let base_a = out.vertices.len() as u32;
        for p in &a.points {
            out.vertices.push(*p);
            out.normals.push(normal_at(*p));
        }
        let base_b = out.vertices.len() as u32;
        for p in &b.points {
            out.vertices.push(*p);
            out.normals.push(normal_at(*p));
        }
        let tris = stitch_rings(&a.points, &b.points, base_a, base_b, origin, z);
        out.triangles.extend(tris);
    }

    // Feature edges: the exact sampled boundary rings.
    out.edges = rings.iter().map(|r| r.points.clone()).collect();
    out.curved = true;
    // The slope direction is exact (a cone is developable); only the angular
    // chordal sagitta of the boundary circles contributes.
    let n = max_segments.max(3);
    out.error_bound = Some(max_radius * (1.0 - (std::f64::consts::PI / n as f64).cos()));
    Ok(out)
}
/// Stitch two closed rings that wind the same way around `axis` into a triangle
/// strip, matching vertices by their angular position so every triangle stays on
/// a cone generator. `base_a`/`base_b` are the vertex offsets of the two rings
/// in the destination mesh; the returned triangles index them directly.
fn stitch_rings(
    a: &[Point3],
    b: &[Point3],
    base_a: u32,
    base_b: u32,
    origin: Point3,
    axis: Point3,
) -> Vec<[u32; 3]> {
    let na = a.len();
    let nb = b.len();
    let mut out = Vec::with_capacity(na + nb);
    if na < 3 || nb < 3 {
        return out;
    }
    let seed = {
        let d = sub(a[0], origin);
        let axial = dot(d, axis);
        let radial = sub(d, mul(axis, axial));
        if length(radial) > 1e-9 {
            radial
        } else {
            sub(a[1], a[0])
        }
    };
    let Some((x, y, _)) = frame_from(axis, seed) else {
        return out;
    };
    let angle = |p: Point3| -> f64 {
        let d = sub(p, origin);
        let axial = dot(d, axis);
        let radial = sub(d, mul(axis, axial));
        dot(radial, y).atan2(dot(radial, x))
    };
    // The two rings may be authored with opposite winding (e.g. the straight
    // cap of a cone uses the opposite face normal). The merge below assumes a
    // common winding, so reverse `b` when its signed orientation about the axis
    // differs from `a`'s.
    let winding = |ring: &[Point3]| -> f64 {
        let mut acc = 0.0;
        for k in 0..ring.len() {
            let p0 = ring[k];
            let p1 = ring[(k + 1) % ring.len()];
            let d0 = sub(p0, origin);
            let d1 = sub(p1, origin);
            let a0 = dot(d0, mul(y, 1.0));
            let r0 = sub(d0, mul(y, a0));
            let a1 = dot(d1, mul(y, 1.0));
            let r1 = sub(d1, mul(y, a1));
            acc += dot(r0, x) * dot(r1, y) - dot(r1, x) * dot(r0, y);
        }
        acc
    };
    let b_reversed: Vec<Point3>;
    let b = if winding(a) * winding(b) < 0.0 {
        b_reversed = b.iter().rev().cloned().collect();
        b_reversed.as_slice()
    } else {
        b
    };
    let nb = b.len();
    // Rotate a ring so it starts at its smallest non-negative angle, leaving the
    // sequence monotone increasing over one full turn.
    let rotate = |ring: &[Point3]| -> (Vec<usize>, Vec<f64>) {
        let raw: Vec<f64> = ring
            .iter()
            .map(|p| {
                let t = angle(*p) % std::f64::consts::TAU;
                if t < 0.0 {
                    t + std::f64::consts::TAU
                } else {
                    t
                }
            })
            .collect();
        let start = raw
            .iter()
            .enumerate()
            .min_by(|(_, p), (_, q)| p.partial_cmp(q).unwrap())
            .map(|(i, _)| i)
            .unwrap_or(0);
        let mut order = Vec::with_capacity(ring.len());
        let mut angles = Vec::with_capacity(ring.len());
        let mut prev = f64::NEG_INFINITY;
        for k in 0..ring.len() {
            let i = (start + k) % ring.len();
            let mut t = raw[i];
            while t + 1e-12 < prev {
                t += std::f64::consts::TAU;
            }
            order.push(i);
            angles.push(t);
            prev = t;
        }
        (order, angles)
    };
    let (order_a, ang_a) = rotate(a);
    let (order_b, ang_b) = rotate(b);
    // Align b to a's start angle: find b's first vertex at/after angle_a[0].
    let a0 = ang_a[0];
    let mut offset_b = 0usize;
    let mut best = f64::INFINITY;
    for (k, t) in ang_b.iter().enumerate() {
        let mut rel = *t - a0;
        if rel < -1e-12 {
            rel += std::f64::consts::TAU;
        }
        if rel < best {
            best = rel;
            offset_b = k;
        }
    }
    // Absolute angle of every vertex over one full turn. Ring `a` starts at 0;
    // ring `b` starts at `best` (its first vertex at/after `a`'s), so its
    // closing sentinel sits at `best + TAU`, one full turn past its start.
    let abs_a = |k: usize| -> f64 {
        if k >= na {
            std::f64::consts::TAU
        } else {
            ang_a[k] - a0
        }
    };
    let abs_b = |k: usize| -> f64 {
        if k >= nb {
            best + std::f64::consts::TAU
        } else {
            let t = ang_b[(offset_b + k) % nb];
            let mut rel = t - a0;
            if rel < -1e-12 {
                rel += std::f64::consts::TAU;
            }
            rel
        }
    };
    let a_vertex = |k: usize| -> u32 { base_a + order_a[k % na] as u32 };
    let b_vertex = |k: usize| -> u32 { base_b + order_b[(offset_b + k) % nb] as u32 };

    let mut ia = 0usize;
    let mut ib = 0usize;
    let mut guard = (na + nb) * 2 + 8;
    while (ia < na || ib < nb) && guard > 0 {
        guard -= 1;
        let advance_a = if ia >= na {
            false
        } else if ib >= nb {
            true
        } else {
            abs_a(ia + 1) <= abs_b(ib + 1)
        };
        if advance_a {
            // `a_vertex`/`b_vertex` take the index modulo the ring length, so
            // once a ring is exhausted `ib`/`ia` wrap to its first vertex. That
            // closes the band against the start instead of clamping onto the
            // last vertex, which would fan a non-manifold seam.
            out.push([a_vertex(ia), b_vertex(ib), a_vertex(ia + 1)]);
            ia += 1;
        } else {
            out.push([a_vertex(ia), b_vertex(ib), b_vertex(ib + 1)]);
            ib += 1;
        }
    }
    out
}
