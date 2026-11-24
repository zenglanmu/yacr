//! Exact planar B-rep data for the polyhedral primitives.
//!
//! `solidcore` has constructors for the cylinder, sphere, cone and torus but
//! **none for the wedge**, and its pyramid takes a square base only, so the
//! wedge and every pyramid (square, regular `n`-gon, and the truncated `n`-gon)
//! are authored here as exact neutral B-rep data: every face is a plane, every
//! boundary curve a straight line, so the body lifts to a kernel record without
//! loss (and the modeler lifts it once to prove the kernel accepts it before
//! handing it out).
//!
//! Every face is wound counter-clockwise seen from outside the material, which is
//! what makes the record's shell normals point out of the material and the
//! resulting solid volume positive. Face ids are sequential in authoring order.
//!
//! Volumes are the closed forms the unit tests assert against: `l·w·h` (box),
//! `l·w·h/2` (wedge, a right triangular prism), `s²·h/3` (square pyramid),
//! `(n/2)·r²·sin(2π/n)·h/3` (regular `n`-gon pyramid) and
//! `(n/2)·sin(2π/n)·h·(r² + r·r_top + r_top²)/3` (truncated `n`-gon pyramid).

use cad_domain::{BrepCurve, BrepData, BrepFace, BrepLoop, BrepShell, BrepSurface, Point3};

/// A point from plain coordinates.
fn p(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

fn add(a: Point3, b: Point3) -> Point3 {
    p(a.x + b.x, a.y + b.y, a.z + b.z)
}

fn sub(a: Point3, b: Point3) -> Point3 {
    p(a.x - b.x, a.y - b.y, a.z - b.z)
}

fn cross(a: Point3, b: Point3) -> Point3 {
    p(
        a.y * b.z - a.z * b.y,
        a.z * b.x - a.x * b.z,
        a.x * b.y - a.y * b.x,
    )
}

fn is_zero(v: Point3) -> bool {
    v.x == 0.0 && v.y == 0.0 && v.z == 0.0
}

fn normalize(v: Point3) -> Point3 {
    let length = (v.x * v.x + v.y * v.y + v.z * v.z).sqrt();
    if length > 1e-12 {
        p(v.x / length, v.y / length, v.z / length)
    } else {
        p(0.0, 0.0, 0.0)
    }
}

/// One planar face from its outward-wound vertex cycle.
///
/// The plane frame is built from the first edge (`u`) and the cycle's own normal
/// (`v = n × u`), so the record's `u_axis`/`v_axis` are orthonormal by
/// construction — the kernel refuses a skewed plane frame, and deriving `v` from
/// a second edge would skew it on a slightly non-planar cycle.
fn face(id: u32, cycle: &[Point3]) -> BrepFace {
    let origin = cycle[0];
    let u_dir = cycle
        .iter()
        .map(|corner| normalize(sub(*corner, origin)))
        .find(|direction| !is_zero(*direction))
        .unwrap_or(p(1.0, 0.0, 0.0));
    let mut normal = p(0.0, 0.0, 0.0);
    for i in 1..cycle.len() {
        for j in (i + 1)..cycle.len() {
            let candidate = normalize(cross(sub(cycle[i], origin), sub(cycle[j], origin)));
            if !is_zero(candidate) {
                normal = candidate;
                break;
            }
        }
        if !is_zero(normal) {
            break;
        }
    }
    let edges = (0..cycle.len())
        .map(|i| BrepCurve::Line {
            start: cycle[i],
            end: cycle[(i + 1) % cycle.len()],
        })
        .collect();
    BrepFace {
        id,
        surface: BrepSurface::Plane {
            origin,
            normal,
            u_dir,
        },
        reversed: false,
        loops: vec![BrepLoop { edges }],
        domain: None,
        tolerance: None,
    }
}

fn body(cycles: &[&[usize]], corners: &[Point3]) -> BrepData {
    let faces = cycles
        .iter()
        .enumerate()
        .map(|(id, cycle)| {
            let points: Vec<Point3> = cycle.iter().map(|&index| corners[index]).collect();
            face(id as u32, &points)
        })
        .collect();
    BrepData {
        shells: vec![BrepShell { id: 0, faces }],
        placement: None,
        tolerance: None,
    }
}

/// An axis-aligned box: `origin` is the lower corner, positive dimensions.
///
/// Volume `length · width · height` (`length` along +X, `width` along +Y,
/// `height` along +Z).
pub fn box_brep(origin: Point3, length: f64, width: f64, height: f64) -> BrepData {
    let o = origin;
    let corners = [
        o,
        add(o, p(length, 0.0, 0.0)),
        add(o, p(length, width, 0.0)),
        add(o, p(0.0, width, 0.0)),
        add(o, p(0.0, 0.0, height)),
        add(o, p(length, 0.0, height)),
        add(o, p(length, width, height)),
        add(o, p(0.0, width, height)),
    ];
    // Outward-wound cycles: -Z, +Z, -Y, +X, +Y, -X.
    let cycles: [&[usize]; 6] = [
        &[0, 3, 2, 1],
        &[4, 5, 6, 7],
        &[0, 1, 5, 4],
        &[1, 2, 6, 5],
        &[2, 3, 7, 6],
        &[3, 0, 4, 7],
    ];
    body(&cycles, &corners)
}

/// A right triangular prism: the base rectangle `length × width` in the XY
/// plane, full `height` on the edge at `x = origin.x`, tapering to zero at
/// `x = origin.x + length`.
///
/// Volume `length · width · height / 2`.
pub fn wedge_brep(origin: Point3, length: f64, width: f64, height: f64) -> BrepData {
    let o = origin;
    // Cross-section in XZ (a right triangle), extruded along +Y.
    let corners = [
        o,                             // a
        add(o, p(length, 0.0, 0.0)),   // b
        add(o, p(0.0, 0.0, height)),   // c
        add(o, p(0.0, width, 0.0)),    // d
        add(o, p(length, width, 0.0)), // e
        add(o, p(0.0, width, height)), // f
    ];
    // Outward: -Y triangle, +Y triangle, bottom (-Z), back (-X), slope.
    let cycles: [&[usize]; 5] = [
        &[0, 1, 2],
        &[3, 5, 4],
        &[0, 3, 4, 1],
        &[0, 2, 5, 3],
        &[1, 4, 5, 2],
    ];
    body(&cycles, &corners)
}

/// A square pyramid: base side `side` centred on `base_center` in the plane
/// `z = base_center.z`, apex `height` above the centre.
///
/// Volume `side² · height / 3`.
pub fn pyramid_brep(base_center: Point3, side: f64, height: f64) -> BrepData {
    let half = side / 2.0;
    let c = base_center;
    let corners = [
        p(c.x - half, c.y - half, c.z),
        p(c.x + half, c.y - half, c.z),
        p(c.x + half, c.y + half, c.z),
        p(c.x - half, c.y + half, c.z),
        p(c.x, c.y, c.z + height),
    ];
    // Outward: base (-Z), then the four side triangles.
    let cycles: [&[usize]; 5] = [
        &[0, 3, 2, 1],
        &[0, 1, 4],
        &[1, 2, 4],
        &[2, 3, 4],
        &[3, 0, 4],
    ];
    body(&cycles, &corners)
}

/// A regular `sides`-gon pyramid: the base ring centred on `base_center` in the
/// plane `z = base_center.z` with vertex radius `radius`, its first vertex at
/// `rotation` radians about `+Z`, and the apex `height` above the centre.
///
/// `sides == 4` with `rotation = π/4` and `radius = side / √2` is exactly
/// [`pyramid_brep`]'s square base, rotated so its vertices sit at the corners;
/// the ring order matches the `POLYGON` command's own construction.
///
/// Volume `(n / 2) · r² · sin(2π/n) · height / 3`.
pub fn pyramid_ngon_brep(
    base_center: Point3,
    radius: f64,
    sides: u32,
    rotation: f64,
    height: f64,
) -> BrepData {
    let n = sides as usize;
    let mut corners = ngon_ring(base_center, radius, sides, rotation);
    corners.push(p(base_center.x, base_center.y, base_center.z + height));
    // Outward: the base's reversed ring (-Z), then one triangle per side.
    let mut cycles: Vec<Vec<usize>> = Vec::with_capacity(n + 1);
    cycles.push((0..n).rev().collect());
    for index in 0..n {
        cycles.push(vec![index, (index + 1) % n, n]);
    }
    let refs: Vec<&[usize]> = cycles.iter().map(Vec::as_slice).collect();
    body(&refs, &corners)
}

/// A regular `sides`-gon **truncated** pyramid: the same bottom ring as
/// [`pyramid_ngon_brep`] and a parallel top ring of vertex radius `top_radius`,
/// `height` above it, at the same `rotation` and vertex count.
///
/// Every side face is a planar trapezoid in exactly the plane the apex form's
/// side triangle lies in, so this is the same authored planar B-rep path: no
/// curved surface and no second construction.
///
/// Volume `(n / 2) · sin(2π/n) · height · (r² + r·r_top + r_top²) / 3`.
pub fn pyramid_ngon_frustum_brep(
    base_center: Point3,
    radius: f64,
    top_radius: f64,
    sides: u32,
    rotation: f64,
    height: f64,
) -> BrepData {
    let n = sides as usize;
    let bottom = ngon_ring(base_center, radius, sides, rotation);
    let top_center = p(base_center.x, base_center.y, base_center.z + height);
    let top = ngon_ring(top_center, top_radius, sides, rotation);
    let mut corners = Vec::with_capacity(2 * n);
    corners.extend_from_slice(&bottom);
    corners.extend_from_slice(&top);
    // Outward: the bottom's reversed ring (-Z), the top ring (+Z), then one
    // trapezoid per side, wound like the apex form's triangle.
    let mut cycles: Vec<Vec<usize>> = Vec::with_capacity(n + 2);
    cycles.push((0..n).rev().collect());
    cycles.push((n..2 * n).collect());
    for index in 0..n {
        let next = (index + 1) % n;
        cycles.push(vec![index, next, n + next, n + index]);
    }
    let refs: Vec<&[usize]> = cycles.iter().map(Vec::as_slice).collect();
    body(&refs, &corners)
}

/// One regular `sides`-gon ring of vertex radius `radius`, centred on `center`
/// in the plane `z = center.z`, first vertex at `rotation` about `+Z`.
fn ngon_ring(center: Point3, radius: f64, sides: u32, rotation: f64) -> Vec<Point3> {
    let step = std::f64::consts::TAU / f64::from(sides);
    (0..sides as usize)
        .map(|index| {
            let (sin, cos) = (rotation + step * index as f64).sin_cos();
            p(center.x + radius * cos, center.y + radius * sin, center.z)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record;

    fn volume(brep: &BrepData) -> f64 {
        let record = record::to_record(brep, record::DEFAULT_TOLERANCE)
            .unwrap_or_else(|refusal| panic!("{refusal:?}"));
        let (body, _) = solidcore_ops::Body::from_record(&record, false)
            .unwrap_or_else(|error| panic!("kernel refused the authored body: {error}"));
        body.volume(1e-10)
            .unwrap_or_else(|error| panic!("volume: {error}"))
    }

    fn counts(brep: &BrepData) -> (usize, usize, usize) {
        let record = record::to_record(brep, record::DEFAULT_TOLERANCE).expect("record");
        (brep.face_count(), record.edges.len(), record.vertices.len())
    }

    #[test]
    fn box_has_six_faces_twelve_edges_eight_vertices_and_unit_volume() {
        let brep = box_brep(p(0.0, 0.0, 0.0), 1.0, 1.0, 1.0);
        assert_eq!(counts(&brep), (6, 12, 8));
        assert!((volume(&brep) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn box_volume_is_length_times_width_times_height() {
        let brep = box_brep(p(-1.0, 2.0, 0.5), 3.0, 2.0, 4.0);
        assert!((volume(&brep) - 24.0).abs() < 1e-9);
    }

    #[test]
    fn wedge_is_a_right_triangular_prism() {
        let brep = wedge_brep(p(0.0, 0.0, 0.0), 2.0, 3.0, 4.0);
        // 5 faces (2 triangles + 3 quads), 9 edges, 6 vertices.
        assert_eq!(counts(&brep), (5, 9, 6));
        assert!((volume(&brep) - 12.0).abs() < 1e-9);
    }

    #[test]
    fn pyramid_is_a_square_base_pyramid() {
        let brep = pyramid_brep(p(0.0, 0.0, 0.0), 2.0, 3.0);
        // 5 faces (1 base + 4 triangles), 8 edges, 5 vertices.
        assert_eq!(counts(&brep), (5, 8, 5));
        assert!((volume(&brep) - 4.0).abs() < 1e-9);
    }

    #[test]
    fn an_ngon_pyramid_has_the_closed_form_volume_and_ring_counts() {
        // A regular hexagon of vertex radius 2: area = 3/2 · √3 · r².
        let sides = 6u32;
        let radius = 2.0;
        let height = 3.0;
        let brep = pyramid_ngon_brep(p(1.0, -2.0, 0.5), radius, sides, 0.0, height);
        // n side triangles + 1 base face, 2n edges, n + 1 vertices.
        assert_eq!(counts(&brep), (7, 12, 7));
        let side = std::f64::consts::TAU / f64::from(sides);
        let expected = 0.5 * f64::from(sides) * radius * radius * side.sin() * height / 3.0;
        assert!((volume(&brep) - expected).abs() < 1e-9);
        // Every base vertex really sits `radius` from the centre in the base plane.
        for corner in &brep.shells[0].faces[0].loops[0].edges {
            let cad_domain::BrepCurve::Line { start, .. } = corner else {
                panic!("the authored base is straight-edged");
            };
            let dx = start.x - 1.0;
            let dy = start.y + 2.0;
            assert!((dx.hypot(dy) - radius).abs() < 1e-9);
            assert!((start.z - 0.5).abs() < 1e-12);
        }
    }

    #[test]
    fn an_ngon_frustum_has_the_closed_form_volume_and_two_rings() {
        // The same hexagon of vertex radius 2 truncated to a top radius of 1:
        // n trapezoids + 2 rings, 3n edges, 2n vertices.
        let sides = 6u32;
        let (radius, top_radius, height) = (2.0, 1.0, 3.0);
        let brep =
            pyramid_ngon_frustum_brep(p(1.0, -2.0, 0.5), radius, top_radius, sides, 0.0, height);
        assert_eq!(counts(&brep), (8, 18, 12));
        let step = std::f64::consts::TAU / f64::from(sides);
        let expected = 0.5
            * f64::from(sides)
            * step.sin()
            * height
            * (radius * radius + radius * top_radius + top_radius * top_radius)
            / 3.0;
        assert!((volume(&brep) - expected).abs() < 1e-9);
        // The top ring is parallel, concentric and `height` above the base.
        let top = &brep.shells[0].faces[1];
        let BrepSurface::Plane { origin, normal, .. } = &top.surface else {
            panic!("the authored top is a plane");
        };
        assert!((origin.z - 3.5).abs() < 1e-12);
        assert!(normal.z > 0.0);
        for edge in &top.loops[0].edges {
            let cad_domain::BrepCurve::Line { start, .. } = edge else {
                panic!("the authored top is straight-edged");
            };
            let dx = start.x - 1.0;
            let dy = start.y + 2.0;
            assert!((dx.hypot(dy) - top_radius).abs() < 1e-9);
            assert!((start.z - 3.5).abs() < 1e-12);
        }
    }

    #[test]
    fn a_four_sided_ngon_frustum_is_the_truncated_square_pyramid() {
        // Sides 4 at rotation π/4 with vertex radius side/√2 is the rotated
        // square base; the same scale applies to both rings, so the truncated
        // square pyramid's volume follows the closed form of the four-sided ring.
        let sides = 4u32;
        let (radius, top_radius, height) = (2.0 / 2.0_f64.sqrt(), 1.0 / 2.0_f64.sqrt(), 3.0);
        let brep = pyramid_ngon_frustum_brep(
            p(0.0, 0.0, 0.0),
            radius,
            top_radius,
            sides,
            std::f64::consts::FRAC_PI_4,
            height,
        );
        // The square base has side 2 and the square top side 1; the frustum of a
        // pyramid with those parallel bases is h/3·(4 + 1 + 2) = 3·7/3 = 7.
        assert_eq!(brep.face_count(), 6, "4 sides + 2 rings");
        assert!((volume(&brep) - 7.0).abs() < 1e-9);
    }

    #[test]
    fn a_four_sided_ngon_pyramid_is_the_rotated_square_pyramid() {
        // Sides 4 at rotation π/4 with vertex radius side/√2 places the corners
        // exactly where `pyramid_brep` does, so the volumes must agree.
        let square = pyramid_brep(p(0.0, 0.0, 0.0), 2.0, 3.0);
        let ngon = pyramid_ngon_brep(
            p(0.0, 0.0, 0.0),
            2.0 / 2.0_f64.sqrt(),
            4,
            std::f64::consts::FRAC_PI_4,
            3.0,
        );
        assert!((volume(&ngon) - volume(&square)).abs() < 1e-9);
    }

    #[test]
    fn every_authored_normal_points_out_of_the_material() {
        // The centroid of the solid lies behind every face plane: a point on the
        // face, offset by the outward normal, points away from the interior.
        let centroid = p(1.0, 2.0, 3.0);
        let brep = box_brep(p(0.0, 0.0, 0.0), 2.0, 4.0, 6.0);
        for face in &brep.shells[0].faces {
            let BrepSurface::Plane {
                origin,
                normal,
                u_dir,
            } = &face.surface
            else {
                panic!("authored face is not a plane");
            };
            // Outward: the face origin is on the far side of the centroid.
            assert!(
                (origin.x - centroid.x) * normal.x
                    + (origin.y - centroid.y) * normal.y
                    + (origin.z - centroid.z) * normal.z
                    > 0.0,
                "face {} points into the box",
                face.id
            );
            // Orthonormal frame: unit axes, perpendicular to each other.
            let n = (normal.x * normal.x + normal.y * normal.y + normal.z * normal.z).sqrt();
            let u = (u_dir.x * u_dir.x + u_dir.y * u_dir.y + u_dir.z * u_dir.z).sqrt();
            assert!((n - 1.0).abs() < 1e-12 && (u - 1.0).abs() < 1e-12);
            assert!((normal.x * u_dir.x + normal.y * u_dir.y + normal.z * u_dir.z).abs() < 1e-12);
        }

        let pyramid = pyramid_brep(p(0.0, 0.0, 0.0), 2.0, 2.0);
        let interior = p(0.0, 0.0, 0.25);
        for face in &pyramid.shells[0].faces {
            let BrepSurface::Plane { origin, normal, .. } = &face.surface else {
                panic!("authored face is not a plane");
            };
            assert!(
                (origin.x - interior.x) * normal.x
                    + (origin.y - interior.y) * normal.y
                    + (origin.z - interior.z) * normal.z
                    > 0.0,
                "face {} points into the pyramid",
                face.id
            );
        }
    }
}
