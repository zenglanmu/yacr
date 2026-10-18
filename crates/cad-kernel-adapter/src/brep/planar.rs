//! Planar faces: loop projection, outer/hole classification, hole bridging
//! and ear-clipping triangulation.

use cad_domain::Point3;

use super::math::{cross, dot, frame_from, length, sub};
use super::tessellate::{face_normal, sample_loop, FaceFailure, FaceMesh};
use super::types::BrepFace;

pub(super) fn tessellate_planar(
    face: &BrepFace,
    origin: Point3,
    normal: Point3,
    u_dir: Point3,
    linear_deflection: f64,
    angular_deflection: f64,
) -> Result<FaceMesh, FaceFailure> {
    let n = face_normal(normal, face.reversed);
    let Some((u, v, _)) = frame_from(n, u_dir) else {
        return Err(FaceFailure::Degenerate("plane frame is degenerate".into()));
    };

    // Sample and project every loop.
    let mut loops2: Vec<Vec<[f64; 2]>> = Vec::new();
    let mut loops3: Vec<Vec<Point3>> = Vec::new();
    for loop_ in &face.loops {
        let ring = sample_loop(loop_, linear_deflection, angular_deflection)?;
        if ring.len() < 3 {
            continue;
        }
        let pts2: Vec<[f64; 2]> = ring
            .iter()
            .map(|p| {
                let d = sub(*p, origin);
                [dot(d, u), dot(d, v)]
            })
            .collect();
        loops2.push(pts2);
        loops3.push(ring);
    }
    if loops2.is_empty() {
        return Err(FaceFailure::Degenerate("face has no usable loop".into()));
    }

    // Classify: the loop with the largest absolute area is the outer boundary.
    let areas: Vec<f64> = loops2.iter().map(|p| ring_area(p)).collect();
    let mut order: Vec<usize> = (0..loops2.len()).collect();
    order.sort_by(|&a, &b| areas[b].abs().partial_cmp(&areas[a].abs()).unwrap());
    let outer = order[0];
    if areas[outer].abs() <= 1e-15 {
        return Err(FaceFailure::Degenerate("outer loop has no area".into()));
    }

    let mut poly2 = loops2[outer].clone();
    let mut poly3 = loops3[outer].clone();
    if ring_area(&poly2) < 0.0 {
        poly2.reverse();
        poly3.reverse();
    }
    let mut holes: Vec<(Vec<[f64; 2]>, Vec<Point3>)> = Vec::new();
    for &idx in &order[1..] {
        let mut h2 = loops2[idx].clone();
        let mut h3 = loops3[idx].clone();
        if ring_area(&h2) > 0.0 {
            h2.reverse();
            h3.reverse();
        }
        if h2.len() >= 3 && ring_area(&h2).abs() > 1e-15 {
            holes.push((h2, h3));
        }
    }
    // Process holes right-to-left so each bridge is visible.
    holes.sort_by(|a, b| {
        let ax = a.0.iter().map(|p| p[0]).fold(f64::NEG_INFINITY, f64::max);
        let bx = b.0.iter().map(|p| p[0]).fold(f64::NEG_INFINITY, f64::max);
        bx.partial_cmp(&ax).unwrap()
    });

    for (h2, h3) in holes {
        match bridge_hole(&mut poly2, &mut poly3, &h2, &h3) {
            Some(_) => {}
            None => return Err(FaceFailure::Degenerate("hole could not be bridged".into())),
        }
    }

    let tris = ear_clip(&poly2);
    if tris.is_empty() {
        return Err(FaceFailure::Degenerate(
            "planar loop did not triangulate".into(),
        ));
    }

    let mut out = FaceMesh::empty();
    out.vertices = poly3;
    out.normals = vec![n; out.vertices.len()];
    // Ear clipping returns CCW triangles in (u, v), whose normal is `n`.
    out.triangles = tris
        .into_iter()
        .filter(|t| {
            let a = out.vertices[t[0] as usize];
            let b = out.vertices[t[1] as usize];
            let c = out.vertices[t[2] as usize];
            length(cross(sub(b, a), sub(c, a))) > 1e-14
        })
        .collect();
    if out.triangles.is_empty() {
        return Err(FaceFailure::Degenerate(
            "planar loop produced only degenerate triangles".into(),
        ));
    }
    out.edges = face
        .loops
        .iter()
        .map(|l| sample_loop(l, linear_deflection, angular_deflection))
        .filter_map(Result::ok)
        .collect();
    Ok(out)
}
fn ring_area(p: &[[f64; 2]]) -> f64 {
    let mut acc = 0.0;
    for i in 0..p.len() {
        let a = p[i];
        let b = p[(i + 1) % p.len()];
        acc += a[0] * b[1] - b[0] * a[1];
    }
    acc / 2.0
}

fn cross2(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

fn point_in_tri(p: [f64; 2], a: [f64; 2], b: [f64; 2], c: [f64; 2], eps: f64) -> bool {
    cross2(a, b, p) >= -eps && cross2(b, c, p) >= -eps && cross2(c, a, p) >= -eps
}

fn same2(a: [f64; 2], b: [f64; 2]) -> bool {
    (a[0] - b[0]).abs() <= 1e-12 && (a[1] - b[1]).abs() <= 1e-12
}

/// Ear-clipping triangulation of a simple CCW polygon (holes already bridged).
fn ear_clip(poly: &[[f64; 2]]) -> Vec<[u32; 3]> {
    let n = poly.len();
    if !(3..=4096).contains(&n) {
        return Vec::new();
    }
    let mut idx: Vec<usize> = (0..n).collect();
    let ccw = ring_area(poly) > 0.0;
    if !ccw {
        idx.reverse();
    }
    let mut out = Vec::new();
    let mut guard = n * n + 8;
    while idx.len() > 3 {
        guard = guard.saturating_sub(1);
        if guard == 0 {
            return Vec::new();
        }
        let m = idx.len();
        let mut clipped = false;
        for i in 0..m {
            let a = idx[(i + m - 1) % m];
            let b = idx[i];
            let c = idx[(i + 1) % m];
            if cross2(poly[a], poly[b], poly[c]) <= 1e-14 {
                continue;
            }
            let mut contains = false;
            for &j in &idx {
                if j == a || j == b || j == c {
                    continue;
                }
                // A hole-bridge duplicate sitting exactly on an ear vertex
                // must not veto the ear.
                if same2(poly[j], poly[a]) || same2(poly[j], poly[b]) || same2(poly[j], poly[c]) {
                    continue;
                }
                if point_in_tri(poly[j], poly[a], poly[b], poly[c], 1e-14) {
                    contains = true;
                    break;
                }
            }
            if contains {
                continue;
            }
            out.push([a as u32, b as u32, c as u32]);
            idx.remove(i);
            clipped = true;
            break;
        }
        if !clipped {
            return Vec::new();
        }
    }
    out.push([idx[0] as u32, idx[1] as u32, idx[2] as u32]);
    out
}

/// Bridge one hole into the outer polygon so the shape becomes a simple CCW
/// polygon. Returns `None` when the hole's rightmost vertex has no visible
/// boundary edge to its right (reported as a degenerate face, never guessed).
fn bridge_hole(
    outer2: &mut Vec<[f64; 2]>,
    outer3: &mut Vec<Point3>,
    hole2: &[[f64; 2]],
    hole3: &[Point3],
) -> Option<()> {
    if hole2.len() < 3 {
        return Some(());
    }
    // Rightmost hole vertex (tie-break highest y).
    let (mi, _) = hole2.iter().enumerate().max_by(|(_, a), (_, b)| {
        a[0].partial_cmp(&b[0])
            .unwrap()
            .then(a[1].partial_cmp(&b[1]).unwrap())
    })?;
    let m = hole2[mi];

    // First boundary edge to the right of m at y = m.y.
    let mut best: Option<(usize, f64)> = None;
    for i in 0..outer2.len() {
        let a = outer2[i];
        let b = outer2[(i + 1) % outer2.len()];
        let straddles = (a[1] <= m[1]) != (b[1] <= m[1]);
        if !straddles {
            continue;
        }
        let t = (m[1] - a[1]) / (b[1] - a[1]);
        let x = a[0] + (b[0] - a[0]) * t;
        if x >= m[0] - 1e-12 {
            match best {
                Some((_, bx)) if x >= bx => {}
                _ => best = Some((i, x)),
            }
        }
    }
    let (edge, _) = best?;

    // Bridge to an existing outer vertex, never a freshly inserted edge point:
    // inserting one would leave a T-junction with the adjacent face, so the
    // welded result could not close. Prefer the rightmost endpoint, then the
    // highest, which keeps the bridge outside the hole for the supported cases.
    let a_idx = edge;
    let b_idx = (edge + 1) % outer2.len();
    let (a, b) = (outer2[a_idx], outer2[b_idx]);
    let p_idx = if a[0] > b[0] + 1e-12 {
        a_idx
    } else if b[0] > a[0] + 1e-12 {
        b_idx
    } else if a[1] >= b[1] {
        a_idx
    } else {
        b_idx
    };

    // Rotate the hole to start at its rightmost vertex.
    let mut h2: Vec<[f64; 2]> = Vec::with_capacity(hole2.len());
    let mut h3: Vec<Point3> = Vec::with_capacity(hole3.len());
    for k in 0..hole2.len() {
        let s = (mi + k) % hole2.len();
        h2.push(hole2[s]);
        h3.push(hole3[s]);
    }

    // outer[0..=p] + hole(M..) + M + P + outer[p+1..]
    let mut merged2: Vec<[f64; 2]> = Vec::with_capacity(outer2.len() + h2.len() + 2);
    let mut merged3: Vec<Point3> = Vec::with_capacity(outer3.len() + h3.len() + 2);
    for k in 0..=p_idx {
        merged2.push(outer2[k]);
        merged3.push(outer3[k]);
    }
    for k in 0..h2.len() {
        merged2.push(h2[k]);
        merged3.push(h3[k]);
    }
    merged2.push(m);
    merged3.push(hole3[mi]);
    merged2.push(outer2[p_idx]);
    merged3.push(outer3[p_idx]);
    for k in p_idx + 1..outer2.len() {
        merged2.push(outer2[k]);
        merged3.push(outer3[k]);
    }
    *outer2 = merged2;
    *outer3 = merged3;
    Some(())
}
