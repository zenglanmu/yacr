//! Neutral B-rep data and its tessellation.
//!
//! This module owns the *only* representation of imported solid geometry that
//! crosses the kernel seam. It is plain data: no ACIS handle, no parser and no
//! acadrust type appears here. The importer (`cad-import-acadrust`, the sole
//! acadrust consumer) lifts a SAT/SAB record graph into these types; this
//! module turns them into a [`cad_domain::Mesh`](crate::TessellationMesh).
//!
//! # Supported subset (honest)
//!
//! * Planar faces bounded by straight and/or full-circular loops, including
//!   inner loops (holes), are triangulated exactly.
//! * Sphere and torus surfaces with an unbounded (loop-less) face are sampled
//!   from their natural parameterisation.
//! * Cylindrical side faces bounded by two full circles are sampled as a strip.
//! * Every other supporting surface, every partial/elliptical curve and every
//!   degenerate loop is reported as an unsupported *face*, never replaced by a
//!   fabricated mesh.
//!
//! The default [`crate::NoKernelTessellator`] still refuses all payloads; a
//! caller opts into this subset explicitly with [`crate::BrepTessellator`].

use cad_domain::{Mesh, Point3};

/// A body placement in the ACIS convention `world = scale·(p·M) + t`.
///
/// `matrix` is row-major exactly as `SatDocument::placement` returns it, so the
/// same numbers survive the seam unchanged.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BrepPlacement {
    pub matrix: [[f64; 3]; 3],
    pub translation: Point3,
    pub scale: f64,
}

impl BrepPlacement {
    pub fn identity() -> Self {
        BrepPlacement {
            matrix: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            translation: Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            scale: 1.0,
        }
    }

    /// Whether this placement is the identity (no transform work needed).
    pub fn is_identity(&self) -> bool {
        self.scale == 1.0
            && self.translation.x == 0.0
            && self.translation.y == 0.0
            && self.translation.z == 0.0
            && self.matrix == [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
    }

    /// Apply the placement to a point (ACIS row-vector convention).
    pub fn apply(&self, p: Point3) -> Point3 {
        let m = &self.matrix;
        let t = self.translation;
        let s = self.scale;
        Point3 {
            x: s * (p.x * m[0][0] + p.y * m[1][0] + p.z * m[2][0]) + t.x,
            y: s * (p.x * m[0][1] + p.y * m[1][1] + p.z * m[2][1]) + t.y,
            z: s * (p.x * m[0][2] + p.y * m[1][2] + p.z * m[2][2]) + t.z,
        }
    }

    /// Rotate a direction by the placement's linear part and renormalise.
    pub fn apply_direction(&self, d: Point3) -> Point3 {
        let m = &self.matrix;
        normalize(Point3 {
            x: d.x * m[0][0] + d.y * m[1][0] + d.z * m[2][0],
            y: d.x * m[0][1] + d.y * m[1][1] + d.z * m[2][1],
            z: d.x * m[0][2] + d.y * m[1][2] + d.z * m[2][2],
        })
    }
}

/// The analytic supporting surface of a face.
#[derive(Debug, Clone, PartialEq)]
pub enum BrepSurface {
    /// A plane through `origin` with `normal`; `u_dir` fixes the in-plane frame.
    Plane {
        origin: Point3,
        normal: Point3,
        u_dir: Point3,
    },
    /// A sphere; loop-less faces cover the full parameter domain.
    Sphere {
        center: Point3,
        radius: f64,
        u_dir: Point3,
        pole: Point3,
    },
    /// A torus whose axis is `normal`; loop-less faces cover the full domain.
    Torus {
        center: Point3,
        normal: Point3,
        major_radius: f64,
        minor_radius: f64,
        u_dir: Point3,
    },
    /// A circular cylinder. Side faces are bounded by two full circles.
    Cylinder {
        origin: Point3,
        axis: Point3,
        ref_dir: Point3,
        radius: f64,
    },
    /// A surface this build cannot evaluate. Reported as a missing face.
    Unsupported { type_key: String },
}

/// A curve segment of a loop, already oriented in loop direction.
#[derive(Debug, Clone, PartialEq)]
pub enum BrepCurve {
    Line {
        start: Point3,
        end: Point3,
    },
    /// A full circle. `u_dir` is the radius direction at parameter zero.
    Circle {
        center: Point3,
        normal: Point3,
        u_dir: Point3,
        radius: f64,
    },
    /// A curve this build cannot evaluate; the owning face is unsupported.
    Unsupported {
        type_key: String,
    },
}

/// A connected, closed loop of curves in loop order.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BrepLoop {
    pub edges: Vec<BrepCurve>,
}

/// A face: a supporting surface plus its boundary loops.
#[derive(Debug, Clone, PartialEq)]
pub struct BrepFace {
    /// Source face record index (or a stable sequential id). Reported on failure.
    pub id: u32,
    pub surface: BrepSurface,
    /// `true` when the face sense is reversed relative to the surface normal.
    pub reversed: bool,
    /// Boundary loops; outer vs. inner is derived geometrically at tessellation.
    pub loops: Vec<BrepLoop>,
}

/// A connected shell of faces.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BrepShell {
    pub id: u32,
    pub faces: Vec<BrepFace>,
}

/// Neutral B-rep for one body.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BrepData {
    pub shells: Vec<BrepShell>,
    pub placement: Option<BrepPlacement>,
}

impl BrepData {
    /// Total face count across all shells.
    pub fn face_count(&self) -> usize {
        self.shells.iter().map(|s| s.faces.len()).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.face_count() == 0
    }
}

// ---------------------------------------------------------------------------
// Small vector helpers (kept local so the module is self-contained).
// ---------------------------------------------------------------------------

pub(crate) fn sub(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.x - b.x,
        y: a.y - b.y,
        z: a.z - b.z,
    }
}

pub(crate) fn add(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.x + b.x,
        y: a.y + b.y,
        z: a.z + b.z,
    }
}

pub(crate) fn mul(a: Point3, s: f64) -> Point3 {
    Point3 {
        x: a.x * s,
        y: a.y * s,
        z: a.z * s,
    }
}

pub(crate) fn dot(a: Point3, b: Point3) -> f64 {
    a.x * b.x + a.y * b.y + a.z * b.z
}

pub(crate) fn cross(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.y * b.z - a.z * b.y,
        y: a.z * b.x - a.x * b.z,
        z: a.x * b.y - a.y * b.x,
    }
}

pub(crate) fn length(a: Point3) -> f64 {
    dot(a, a).sqrt()
}

pub(crate) fn normalize(a: Point3) -> Point3 {
    let l = length(a);
    if l > 1e-12 {
        mul(a, 1.0 / l)
    } else {
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        }
    }
}

fn finite(p: Point3) -> bool {
    p.x.is_finite() && p.y.is_finite() && p.z.is_finite()
}

/// Build a right-handed orthonormal frame `(x, y, z)` with `z = axis`.
fn frame_from(axis: Point3, preferred_x: Point3) -> Option<(Point3, Point3, Point3)> {
    let z = normalize(axis);
    if length(z) < 0.5 {
        return None;
    }
    let mut x = sub(preferred_x, mul(z, dot(preferred_x, z)));
    if length(x) < 1e-9 {
        // Pick any axis not parallel to z.
        let alt = if z.x.abs() < 0.9 {
            Point3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            }
        } else {
            Point3 {
                x: 0.0,
                y: 1.0,
                z: 0.0,
            }
        };
        x = sub(alt, mul(z, dot(alt, z)));
    }
    let x = normalize(x);
    if length(x) < 0.5 {
        return None;
    }
    let y = cross(z, x);
    Some((x, y, z))
}

/// One face's local tessellation result, before global assembly.
#[derive(Debug, Clone)]
pub(crate) struct FaceMesh {
    pub vertices: Vec<Point3>,
    pub normals: Vec<Point3>,
    pub triangles: Vec<[u32; 3]>,
    /// Feature/boundary polylines for this face (source loops).
    pub edges: Vec<Vec<Point3>>,
    /// `true` if any part of the face is a curved approximation.
    pub curved: bool,
    /// Chordal error bound for curved faces.
    pub error_bound: Option<f64>,
}

impl FaceMesh {
    fn empty() -> Self {
        FaceMesh {
            vertices: Vec::new(),
            normals: Vec::new(),
            triangles: Vec::new(),
            edges: Vec::new(),
            curved: false,
            error_bound: None,
        }
    }
}

/// Why a face could not be tessellated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FaceFailure {
    UnsupportedSurface(String),
    UnsupportedCurve(String),
    Degenerate(String),
}

impl FaceFailure {
    pub(crate) fn reason(&self) -> String {
        match self {
            FaceFailure::UnsupportedSurface(t) => format!("unsupported surface '{t}'"),
            FaceFailure::UnsupportedCurve(t) => format!("unsupported curve '{t}'"),
            FaceFailure::Degenerate(m) => format!("degenerate face: {m}"),
        }
    }

    pub(crate) fn is_unsupported(&self) -> bool {
        matches!(
            self,
            FaceFailure::UnsupportedSurface(_) | FaceFailure::UnsupportedCurve(_)
        )
    }
}

/// Number of segments for an arc of `sweep` radians on a circle of `radius`.
///
/// Both the chordal deflection and the angular deflection are respected; the
/// result is monotone in `linear_deflection` (smaller deflection → no fewer
/// segments), which is exactly the tolerance-monotonicity contract.
pub(crate) fn arc_segments(
    radius: f64,
    sweep: f64,
    linear_deflection: f64,
    angular_deflection: f64,
) -> usize {
    let sweep = sweep.abs();
    if !sweep.is_finite() || sweep <= 0.0 || !radius.is_finite() || radius <= 0.0 {
        return 3;
    }
    let chord = if linear_deflection < radius {
        let ratio = (1.0 - linear_deflection / radius).clamp(-1.0, 1.0);
        2.0 * ratio.acos()
    } else {
        std::f64::consts::PI
    };
    let n_chord = (sweep / chord.max(1e-9)).ceil();
    let n_ang = (sweep / angular_deflection.max(1e-9)).ceil();
    let n = n_chord.max(n_ang).max(3.0);
    (n as usize).clamp(3, 8192)
}

/// Tessellate one neutral face. Unsupported geometry is an error, never a mesh.
pub(crate) fn tessellate_face(
    face: &BrepFace,
    linear_deflection: f64,
    angular_deflection: f64,
) -> Result<FaceMesh, FaceFailure> {
    match &face.surface {
        BrepSurface::Plane {
            origin,
            normal,
            u_dir,
        } => tessellate_planar(
            face,
            *origin,
            *normal,
            *u_dir,
            linear_deflection,
            angular_deflection,
        ),
        BrepSurface::Sphere {
            center,
            radius,
            u_dir,
            pole,
        } => tessellate_sphere(
            face,
            *center,
            *radius,
            *u_dir,
            *pole,
            linear_deflection,
            angular_deflection,
        ),
        BrepSurface::Torus {
            center,
            normal,
            major_radius,
            minor_radius,
            u_dir,
        } => tessellate_torus(
            face,
            *center,
            *normal,
            *major_radius,
            *minor_radius,
            *u_dir,
            linear_deflection,
            angular_deflection,
        ),
        BrepSurface::Cylinder {
            origin,
            axis,
            ref_dir,
            radius,
        } => tessellate_cylinder(
            face,
            *origin,
            *axis,
            *ref_dir,
            *radius,
            linear_deflection,
            angular_deflection,
        ),
        BrepSurface::Unsupported { type_key } => {
            Err(FaceFailure::UnsupportedSurface(type_key.clone()))
        }
    }
}

/// Effective outward normal of a face.
fn face_normal(surface_normal: Point3, reversed: bool) -> Point3 {
    if reversed {
        mul(surface_normal, -1.0)
    } else {
        surface_normal
    }
}

/// Sample a loop into a closed 3D ring.
fn sample_loop(
    loop_: &BrepLoop,
    linear_deflection: f64,
    angular_deflection: f64,
) -> Result<Vec<Point3>, FaceFailure> {
    let mut pts = Vec::new();
    for curve in &loop_.edges {
        match curve {
            BrepCurve::Line { start, .. } => pts.push(*start),
            BrepCurve::Circle {
                center,
                normal,
                u_dir,
                radius,
            } => {
                if !radius.is_finite() || *radius <= 0.0 {
                    return Err(FaceFailure::Degenerate("circle has no radius".into()));
                }
                let Some((u, v, _)) = frame_from(*normal, *u_dir) else {
                    return Err(FaceFailure::Degenerate("circle frame is degenerate".into()));
                };
                let n = arc_segments(
                    *radius,
                    std::f64::consts::TAU,
                    linear_deflection,
                    angular_deflection,
                );
                for i in 0..n {
                    let t = std::f64::consts::TAU * (i as f64) / (n as f64);
                    pts.push(add(
                        *center,
                        add(mul(u, *radius * t.cos()), mul(v, *radius * t.sin())),
                    ));
                }
            }
            BrepCurve::Unsupported { type_key } => {
                return Err(FaceFailure::UnsupportedCurve(type_key.clone()))
            }
        }
    }
    dedupe_ring(&mut pts);
    Ok(pts)
}

/// Drop consecutive (and wrap-around) duplicate points from a ring.
fn dedupe_ring(pts: &mut Vec<Point3>) {
    pts.dedup_by(|a, b| {
        let d = sub(*a, *b);
        dot(d, d) <= 1e-18
    });
    while pts.len() >= 2 {
        let d = sub(pts[0], pts[pts.len() - 1]);
        if dot(d, d) <= 1e-18 {
            pts.pop();
        } else {
            break;
        }
    }
}

fn tessellate_planar(
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

fn tessellate_sphere(
    face: &BrepFace,
    center: Point3,
    radius: f64,
    u_dir: Point3,
    pole: Point3,
    linear_deflection: f64,
    angular_deflection: f64,
) -> Result<FaceMesh, FaceFailure> {
    if !face.loops.is_empty() {
        return Err(FaceFailure::UnsupportedSurface(
            "sphere with trimming loops".into(),
        ));
    }
    if !radius.is_finite() || radius <= 0.0 {
        return Err(FaceFailure::Degenerate("sphere has no radius".into()));
    }
    let Some((x, y, z)) = frame_from(pole, u_dir) else {
        return Err(FaceFailure::Degenerate("sphere frame is degenerate".into()));
    };
    let n_lon = arc_segments(
        radius,
        std::f64::consts::TAU,
        linear_deflection,
        angular_deflection,
    );
    let n_lat = arc_segments(
        radius,
        std::f64::consts::PI,
        linear_deflection,
        angular_deflection,
    );
    let n_lat = n_lat.max(2);

    let point = |lon: f64, lat: f64| {
        let (sl, cl) = lat.sin_cos();
        add(
            center,
            mul(
                add(
                    mul(x, cl * lon.cos()),
                    add(mul(y, cl * lon.sin()), mul(z, sl)),
                ),
                radius,
            ),
        )
    };
    let normal_at = |p: Point3| normalize(sub(p, center));

    let mut out = FaceMesh::empty();
    let south = point(0.0, -std::f64::consts::FRAC_PI_2);
    let north = point(0.0, std::f64::consts::FRAC_PI_2);
    out.vertices.push(south);
    out.normals
        .push(face_normal(normal_at(south), face.reversed));
    let south_i = 0u32;
    // Interior latitude rings.
    let mut rings: Vec<Vec<u32>> = Vec::new();
    for j in 1..n_lat {
        let lat = -std::f64::consts::FRAC_PI_2 + std::f64::consts::PI * (j as f64) / (n_lat as f64);
        let mut ring = Vec::with_capacity(n_lon);
        for i in 0..n_lon {
            let lon = std::f64::consts::TAU * (i as f64) / (n_lon as f64);
            let p = point(lon, lat);
            ring.push(out.vertices.len() as u32);
            out.vertices.push(p);
            out.normals.push(face_normal(normal_at(p), face.reversed));
        }
        rings.push(ring);
    }
    out.vertices.push(north);
    out.normals
        .push(face_normal(normal_at(north), face.reversed));
    let north_i = (out.vertices.len() - 1) as u32;

    // South fan.
    for i in 0..n_lon {
        let a = rings[0][i];
        let b = rings[0][(i + 1) % n_lon];
        out.triangles.push([south_i, b, a]);
    }
    // Middle bands.
    for j in 0..rings.len() - 1 {
        for i in 0..n_lon {
            let a = rings[j][i];
            let b = rings[j][(i + 1) % n_lon];
            let c = rings[j + 1][(i + 1) % n_lon];
            let d = rings[j + 1][i];
            out.triangles.push([a, b, c]);
            out.triangles.push([a, c, d]);
        }
    }
    // North fan.
    let last = rings.last().unwrap();
    for i in 0..n_lon {
        let a = last[i];
        let b = last[(i + 1) % n_lon];
        out.triangles.push([a, b, north_i]);
    }

    out.curved = true;
    out.error_bound = Some(radius * (1.0 - (std::f64::consts::PI / n_lat as f64).cos()));
    Ok(out)
}

#[allow(clippy::too_many_arguments)]
fn tessellate_torus(
    face: &BrepFace,
    center: Point3,
    normal: Point3,
    major_radius: f64,
    minor_radius: f64,
    u_dir: Point3,
    linear_deflection: f64,
    angular_deflection: f64,
) -> Result<FaceMesh, FaceFailure> {
    if !face.loops.is_empty() {
        return Err(FaceFailure::UnsupportedSurface(
            "torus with trimming loops".into(),
        ));
    }
    if major_radius <= 0.0 || minor_radius <= 0.0 {
        return Err(FaceFailure::Degenerate("torus has no radii".into()));
    }
    let Some((x, y, z)) = frame_from(normal, u_dir) else {
        return Err(FaceFailure::Degenerate("torus frame is degenerate".into()));
    };
    let n_u = arc_segments(
        major_radius + minor_radius,
        std::f64::consts::TAU,
        linear_deflection,
        angular_deflection,
    );
    let n_v = arc_segments(
        minor_radius,
        std::f64::consts::TAU,
        linear_deflection,
        angular_deflection,
    );

    let point = |u: f64, v: f64| {
        let (su, cu) = u.sin_cos();
        let (sv, cv) = v.sin_cos();
        add(
            center,
            add(
                mul(
                    add(mul(x, cu), mul(y, su)),
                    major_radius + minor_radius * cv,
                ),
                mul(z, minor_radius * sv),
            ),
        )
    };
    let normal_at = |u: f64, v: f64| {
        let (su, cu) = u.sin_cos();
        let (sv, cv) = v.sin_cos();
        normalize(add(mul(add(mul(x, cu), mul(y, su)), cv), mul(z, sv)))
    };

    let mut out = FaceMesh::empty();
    for i in 0..n_u {
        for j in 0..n_v {
            let u = std::f64::consts::TAU * (i as f64) / (n_u as f64);
            let v = std::f64::consts::TAU * (j as f64) / (n_v as f64);
            let p = point(u, v);
            out.vertices.push(p);
            out.normals
                .push(face_normal(normal_at(u, v), face.reversed));
        }
    }
    let idx = |i: usize, j: usize| ((i % n_u) * n_v + (j % n_v)) as u32;
    for i in 0..n_u {
        for j in 0..n_v {
            let a = idx(i, j);
            let b = idx(i + 1, j);
            let c = idx(i + 1, j + 1);
            let d = idx(i, j + 1);
            out.triangles.push([a, b, c]);
            out.triangles.push([a, c, d]);
        }
    }
    out.curved = true;
    let err_u = (major_radius + minor_radius) * (1.0 - (std::f64::consts::PI / n_u as f64).cos());
    let err_v = minor_radius * (1.0 - (std::f64::consts::PI / n_v as f64).cos());
    out.error_bound = Some(err_u.max(err_v));
    Ok(out)
}

#[allow(clippy::too_many_arguments)]
fn tessellate_cylinder(
    face: &BrepFace,
    origin: Point3,
    axis: Point3,
    ref_dir: Point3,
    radius: f64,
    linear_deflection: f64,
    angular_deflection: f64,
) -> Result<FaceMesh, FaceFailure> {
    if face.loops.len() != 2 {
        return Err(FaceFailure::UnsupportedSurface(
            "cylinder face without two circular boundaries".into(),
        ));
    }
    if !radius.is_finite() || radius <= 0.0 {
        return Err(FaceFailure::Degenerate("cylinder has no radius".into()));
    }
    let Some((x, y, z)) = frame_from(axis, ref_dir) else {
        return Err(FaceFailure::Degenerate(
            "cylinder frame is degenerate".into(),
        ));
    };
    // Each loop must be a single full circle; its centre gives the axial level.
    let mut levels = Vec::new();
    for loop_ in &face.loops {
        if loop_.edges.len() != 1 {
            return Err(FaceFailure::UnsupportedSurface(
                "cylinder boundary is not a single full circle".into(),
            ));
        }
        match &loop_.edges[0] {
            BrepCurve::Circle { center, .. } => {
                if (dot(sub(*center, origin), z)).is_finite() {
                    levels.push(dot(sub(*center, origin), z));
                } else {
                    return Err(FaceFailure::Degenerate(
                        "cylinder level is not finite".into(),
                    ));
                }
            }
            _ => {
                return Err(FaceFailure::UnsupportedSurface(
                    "cylinder boundary is not a full circle".into(),
                ))
            }
        }
    }
    if (levels[0] - levels[1]).abs() <= 1e-12 {
        return Err(FaceFailure::Degenerate(
            "cylinder boundaries share an axial level".into(),
        ));
    }
    let v0 = levels[0];
    let v1 = levels[1];
    let n = arc_segments(
        radius,
        std::f64::consts::TAU,
        linear_deflection,
        angular_deflection,
    );

    let mut out = FaceMesh::empty();
    for level in [v0, v1] {
        for i in 0..n {
            let t = std::f64::consts::TAU * (i as f64) / (n as f64);
            let radial = add(mul(x, t.cos()), mul(y, t.sin()));
            let p = add(add(origin, mul(z, level)), mul(radial, radius));
            out.vertices.push(p);
            out.normals.push(face_normal(radial, face.reversed));
        }
    }
    let idx = |ring: usize, i: usize| (ring * n + (i % n)) as u32;
    for i in 0..n {
        let a = idx(0, i);
        let b = idx(0, i + 1);
        let c = idx(1, i + 1);
        let d = idx(1, i);
        out.triangles.push([a, b, c]);
        out.triangles.push([a, c, d]);
    }
    out.curved = true;
    out.error_bound = Some(radius * (1.0 - (std::f64::consts::PI / n as f64).cos()));
    Ok(out)
}

// ---------------------------------------------------------------------------
// 2D polygon handling (projection, hole bridging, ear clipping).
// ---------------------------------------------------------------------------

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

/// Triangle area of the assembled mesh, for tests and reporting.
pub fn mesh_area(mesh: &Mesh) -> f64 {
    let mut area = 0.0;
    for t in &mesh.triangles {
        let a = mesh.vertices[t[0] as usize];
        let b = mesh.vertices[t[1] as usize];
        let c = mesh.vertices[t[2] as usize];
        area += length(cross(sub(b, a), sub(c, a))) * 0.5;
    }
    area
}

/// Whether every finite coordinate is present (defensive, for diagnostics).
pub fn mesh_is_finite(mesh: &Mesh) -> bool {
    mesh.vertices.iter().all(|p| finite(*p)) && mesh.normals.iter().all(|p| finite(*p))
}

fn finite2(p: [f64; 2]) -> bool {
    p[0].is_finite() && p[1].is_finite()
}

/// Kept for symmetry with `mesh_is_finite`; the 2D path is internal.
#[allow(dead_code)]
pub(crate) fn polygon_is_finite(p: &[[f64; 2]]) -> bool {
    p.iter().all(|q| finite2(*q))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: f64, y: f64, z: f64) -> Point3 {
        Point3 { x, y, z }
    }

    fn square_loop(z: f64, ccw: bool) -> BrepLoop {
        let pts = if ccw {
            vec![[0.0, 0.0], [2.0, 0.0], [2.0, 2.0], [0.0, 2.0]]
        } else {
            vec![[0.0, 0.0], [0.0, 2.0], [2.0, 2.0], [2.0, 0.0]]
        };
        BrepLoop {
            edges: pts
                .iter()
                .map(|q| BrepCurve::Line {
                    start: p(q[0], q[1], z),
                    end: p(q[0], q[1], z),
                })
                .collect(),
        }
    }

    #[test]
    fn planar_square_triangulates_to_two_triangles() {
        let face = BrepFace {
            id: 0,
            surface: BrepSurface::Plane {
                origin: p(0.0, 0.0, 0.0),
                normal: p(0.0, 0.0, 1.0),
                u_dir: p(1.0, 0.0, 0.0),
            },
            reversed: false,
            loops: vec![square_loop(0.0, true)],
        };
        let m = tessellate_face(&face, 0.01, 0.35).unwrap();
        assert_eq!(m.triangles.len(), 2);
        let area: f64 = m
            .triangles
            .iter()
            .map(|t| {
                let a = m.vertices[t[0] as usize];
                let b = m.vertices[t[1] as usize];
                let c = m.vertices[t[2] as usize];
                length(cross(sub(b, a), sub(c, a))) * 0.5
            })
            .sum();
        assert!((area - 4.0).abs() < 1e-9, "area {area}");
        for n in &m.normals {
            assert!((n.z - 1.0).abs() < 1e-9);
        }
    }

    #[test]
    fn reversed_planar_face_flips_the_normal() {
        let face = BrepFace {
            id: 0,
            surface: BrepSurface::Plane {
                origin: p(0.0, 0.0, 0.0),
                normal: p(0.0, 0.0, 1.0),
                u_dir: p(1.0, 0.0, 0.0),
            },
            reversed: true,
            loops: vec![square_loop(0.0, true)],
        };
        let m = tessellate_face(&face, 0.01, 0.35).unwrap();
        assert!(m.normals.iter().all(|n| n.z < -0.9));
    }

    #[test]
    fn planar_face_with_square_hole_keeps_the_hole_open() {
        // A 4x4 outer square with a 2x2 inner hole: area 16 - 4 = 12.
        let outer = BrepLoop {
            edges: [[0.0, 0.0], [4.0, 0.0], [4.0, 4.0], [0.0, 4.0]]
                .iter()
                .map(|q| BrepCurve::Line {
                    start: p(q[0], q[1], 0.0),
                    end: p(q[0], q[1], 0.0),
                })
                .collect(),
        };
        // Hole wound opposite to the outer boundary.
        let hole = BrepLoop {
            edges: [[1.0, 1.0], [1.0, 3.0], [3.0, 3.0], [3.0, 1.0]]
                .iter()
                .map(|q| BrepCurve::Line {
                    start: p(q[0], q[1], 0.0),
                    end: p(q[0], q[1], 0.0),
                })
                .collect(),
        };
        let face = BrepFace {
            id: 0,
            surface: BrepSurface::Plane {
                origin: p(0.0, 0.0, 0.0),
                normal: p(0.0, 0.0, 1.0),
                u_dir: p(1.0, 0.0, 0.0),
            },
            reversed: false,
            loops: vec![outer, hole],
        };
        let m = tessellate_face(&face, 0.01, 0.35).unwrap();
        let area: f64 = m
            .triangles
            .iter()
            .map(|t| {
                let a = m.vertices[t[0] as usize];
                let b = m.vertices[t[1] as usize];
                let c = m.vertices[t[2] as usize];
                length(cross(sub(b, a), sub(c, a))) * 0.5
            })
            .sum();
        assert!((area - 12.0).abs() < 1e-9, "area {area}");
    }

    #[test]
    fn arc_segments_are_monotone_in_tolerance() {
        let coarse = arc_segments(1.0, std::f64::consts::TAU, 0.5, 0.35);
        let fine = arc_segments(1.0, std::f64::consts::TAU, 0.01, 0.35);
        assert!(fine >= coarse, "{fine} < {coarse}");
    }

    #[test]
    fn unsupported_surface_is_an_error_not_a_mesh() {
        let face = BrepFace {
            id: 3,
            surface: BrepSurface::Unsupported {
                type_key: "nurbs-surface".into(),
            },
            reversed: false,
            loops: Vec::new(),
        };
        let err = tessellate_face(&face, 0.01, 0.35).unwrap_err();
        assert!(err.is_unsupported());
        assert!(err.reason().contains("nurbs-surface"));
    }
}
