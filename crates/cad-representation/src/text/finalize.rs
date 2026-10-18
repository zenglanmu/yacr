use cad_domain::{Point3, TextAlignH, TextAlignV};

use super::layout::{ShapedLine, LINE_SPACING};
use super::model::TextFormatIssue;

/// Apply per-line horizontal alignment, vertical block placement, rotation and
/// translation.
///
/// Horizontal alignment is applied per line (so a multi-line block stays
/// flush against its anchor); vertical placement uses the accumulated line
/// heights with [`LINE_SPACING`]. Vertical alignment of a multi-line block is a
/// documented approximation (see the module docs).
#[cfg(test)]
pub(super) fn finalize_lines(
    lines: Vec<ShapedLine>,
    size: f64,
    origin: Point3,
    rotation: f64,
    h_align: TextAlignH,
    v_align: TextAlignV,
) -> Vec<Vec<Point3>> {
    finalize_text(lines, size, origin, rotation, h_align, v_align).outlines
}

pub(super) struct FinalizedText {
    pub(super) polylines: Vec<Vec<Point3>>,
    pub(super) outlines: Vec<Vec<Point3>>,
    pub(super) meshes: Vec<cad_domain::Mesh>,
    pub(super) fill_failures: Vec<String>,
}

#[derive(Clone, Copy)]
struct GlyphFillTransform {
    dx: f64,
    line_y: f64,
    origin: Point3,
    sin: f64,
    cos: f64,
}

/// Finalize text geometry while keeping each marked SFNT glyph's contours
/// together for filling. Empty polylines are internal glyph markers; SHX
/// contours are unmarked and stay as ordinary line geometry.
pub(super) fn finalize_text(
    lines: Vec<ShapedLine>,
    size: f64,
    origin: Point3,
    rotation: f64,
    h_align: TextAlignH,
    v_align: TextAlignV,
) -> FinalizedText {
    let vertical_shift = match v_align {
        TextAlignV::Baseline => 0.0,
        TextAlignV::Bottom => 0.2 * size,
        TextAlignV::Middle => 0.5 * size,
        TextAlignV::Top => size,
    };
    let (sin, cos) = rotation.sin_cos();
    let mut polylines = Vec::new();
    let mut outlines = Vec::new();
    let mut meshes = Vec::new();
    let mut failures = Vec::new();
    let mut baseline = 0.0;
    for line in lines {
        let dx = match h_align {
            TextAlignH::Left => 0.0,
            TextAlignH::Center => -line.width / 2.0,
            TextAlignH::Right => -line.width,
        };
        let line_y = baseline + vertical_shift;
        let map = |poly: &[[f64; 2]]| -> Vec<Point3> {
            poly.iter()
                .map(|p| {
                    let lx = p[0] + dx;
                    let ly = p[1] + line_y;
                    Point3 {
                        x: origin.x + cos * lx - sin * ly,
                        y: origin.y + sin * lx + cos * ly,
                        z: origin.z,
                    }
                })
                .collect()
        };
        let fill_transform = GlyphFillTransform {
            dx,
            line_y,
            origin,
            sin,
            cos,
        };
        let mut glyph_rings: Option<Vec<Vec<[f64; 2]>>> = None;
        for poly in line.polys {
            if poly.is_empty() {
                if let Some(rings) = glyph_rings.take() {
                    let old_failures = failures.len();
                    fill_sfnt_glyph(&rings, fill_transform, &mut meshes, &mut failures);
                    if failures.len() != old_failures {
                        polylines.extend(rings.iter().map(|ring| map(ring)));
                    }
                } else {
                    glyph_rings = Some(Vec::new());
                }
                continue;
            }
            let world = map(&poly);
            outlines.push(world.clone());
            if let Some(rings) = glyph_rings.as_mut() {
                rings.push(poly);
            } else {
                polylines.push(world);
            }
        }
        if let Some(rings) = glyph_rings {
            if !rings.is_empty() {
                let old_failures = failures.len();
                fill_sfnt_glyph(&rings, fill_transform, &mut meshes, &mut failures);
                if failures.len() != old_failures {
                    polylines.extend(rings.iter().map(|ring| map(ring)));
                }
            }
        }
        baseline -= LINE_SPACING * line.height;
    }
    FinalizedText {
        polylines,
        outlines,
        meshes,
        fill_failures: failures,
    }
}

fn fill_sfnt_glyph(
    rings: &[Vec<[f64; 2]>],
    transform: GlyphFillTransform,
    meshes: &mut Vec<cad_domain::Mesh>,
    failures: &mut Vec<String>,
) {
    if let Err(reason) = validate_nonzero_fill_equivalence(rings) {
        failures.push(format!("SFNT fill: {reason}"));
        return;
    }
    let fill = match cad_geometry::fill_rings(rings) {
        Ok(fill) => fill,
        Err(error) => {
            failures.push(format!("SFNT fill: {}", error.reason()));
            return;
        }
    };
    let vertices: Vec<Point3> = fill
        .vertices
        .iter()
        .map(|p| {
            let x = p[0] + transform.dx;
            let y = p[1] + transform.line_y;
            Point3 {
                x: transform.origin.x + transform.cos * x - transform.sin * y,
                y: transform.origin.y + transform.sin * x + transform.cos * y,
                z: transform.origin.z,
            }
        })
        .collect();
    meshes.push(cad_domain::Mesh {
        normals: vec![
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0
            };
            vertices.len()
        ],
        vertices,
        triangles: fill.triangles,
        face_sources: Vec::new(),
        colors: Vec::new(),
    });
}

/// `fill_rings` uses even-odd semantics. SFNT uses nonzero winding, so accept
/// it only when simple, disjoint contours with alternating nested winding make
/// the two rules equivalent.
pub(super) fn validate_nonzero_fill_equivalence(
    rings: &[Vec<[f64; 2]>],
) -> Result<(), &'static str> {
    let rings: Vec<Vec<[f64; 2]>> = rings
        .iter()
        .map(|ring| {
            let mut points = ring.clone();
            if points.len() > 1 && points.first() == points.last() {
                points.pop();
            }
            points
        })
        .collect();
    if rings.is_empty() || rings.iter().any(|ring| ring.len() < 3) {
        return Err("contour is degenerate");
    }
    for ring in &rings {
        if ring.iter().flatten().any(|v| !v.is_finite()) || ring_area(ring).abs() <= 1e-12 {
            return Err("contour is non-finite or zero-area");
        }
        if ring_self_intersects(ring) {
            return Err("self-intersecting contour");
        }
    }
    for i in 0..rings.len() {
        for j in i + 1..rings.len() {
            if rings_intersect(&rings[i], &rings[j]) {
                return Err("intersecting or overlapping contours");
            }
        }
    }
    let depths: Vec<usize> = rings
        .iter()
        .enumerate()
        .map(|(i, ring)| {
            rings
                .iter()
                .enumerate()
                .filter(|(j, other)| *j != i && point_in_ring(ring[0], other))
                .count()
        })
        .collect();
    let areas: Vec<f64> = rings.iter().map(|ring| ring_area(ring).abs()).collect();
    for i in 0..rings.len() {
        if depths[i] == 0 {
            continue;
        }
        let parent = (0..rings.len())
            .filter(|&j| {
                j != i && depths[j] + 1 == depths[i] && point_in_ring(rings[i][0], &rings[j])
            })
            .min_by(|&a, &b| areas[a].total_cmp(&areas[b]));
        let Some(parent) = parent else {
            return Err("nested contour hierarchy is ambiguous");
        };
        if ring_area(&rings[i]).is_sign_positive() == ring_area(&rings[parent]).is_sign_positive() {
            return Err("nested contour winding does not alternate");
        }
    }
    Ok(())
}

fn ring_area(ring: &[[f64; 2]]) -> f64 {
    (0..ring.len())
        .map(|i| {
            let a = ring[i];
            let b = ring[(i + 1) % ring.len()];
            a[0] * b[1] - b[0] * a[1]
        })
        .sum::<f64>()
        * 0.5
}

fn point_in_ring(point: [f64; 2], ring: &[[f64; 2]]) -> bool {
    let mut inside = false;
    for i in 0..ring.len() {
        let a = ring[i];
        let b = ring[(i + 1) % ring.len()];
        if (a[1] > point[1]) != (b[1] > point[1])
            && point[0] < a[0] + (point[1] - a[1]) * (b[0] - a[0]) / (b[1] - a[1])
        {
            inside = !inside;
        }
    }
    inside
}

fn ring_self_intersects(ring: &[[f64; 2]]) -> bool {
    let n = ring.len();
    for i in 0..n {
        for j in i + 1..n {
            if j == i + 1 || (i == 0 && j == n - 1) {
                continue;
            }
            if segments_intersect(ring[i], ring[(i + 1) % n], ring[j], ring[(j + 1) % n]) {
                return true;
            }
        }
    }
    false
}

fn rings_intersect(a: &[[f64; 2]], b: &[[f64; 2]]) -> bool {
    (0..a.len()).any(|i| {
        (0..b.len())
            .any(|j| segments_intersect(a[i], a[(i + 1) % a.len()], b[j], b[(j + 1) % b.len()]))
    })
}

fn segments_intersect(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2]) -> bool {
    let orient = |p: [f64; 2], q: [f64; 2], r: [f64; 2]| {
        (q[0] - p[0]) * (r[1] - p[1]) - (q[1] - p[1]) * (r[0] - p[0])
    };
    let scale = [a, b, c, d]
        .iter()
        .flatten()
        .fold(1.0_f64, |scale, value| scale.max(value.abs()));
    let eps = scale * scale * 1e-12;
    let (o1, o2, o3, o4) = (
        orient(a, b, c),
        orient(a, b, d),
        orient(c, d, a),
        orient(c, d, b),
    );
    let on_segment = |p: [f64; 2], q: [f64; 2], r: [f64; 2]| {
        r[0] >= p[0].min(q[0]) - eps
            && r[0] <= p[0].max(q[0]) + eps
            && r[1] >= p[1].min(q[1]) - eps
            && r[1] <= p[1].max(q[1]) + eps
    };
    ((o1 > eps && o2 < -eps) || (o1 < -eps && o2 > eps))
        && ((o3 > eps && o4 < -eps) || (o3 < -eps && o4 > eps))
        || (o1.abs() <= eps && on_segment(a, b, c))
        || (o2.abs() <= eps && on_segment(a, b, d))
        || (o3.abs() <= eps && on_segment(c, d, a))
        || (o4.abs() <= eps && on_segment(c, d, b))
}

/// Drop repeated issue codes, keeping the first message for each.
pub(super) fn dedupe_issues(issues: Vec<TextFormatIssue>) -> Vec<TextFormatIssue> {
    let mut out: Vec<TextFormatIssue> = Vec::new();
    for issue in issues {
        if !out.iter().any(|existing| existing.code == issue.code) {
            out.push(issue);
        }
    }
    out
}
