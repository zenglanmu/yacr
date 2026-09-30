//! Curve discretisation with tolerance-driven LOD (spec v2.0 §8.1, §10.2).
//!
//! Segment counts come from a chord-error budget, so LOD is tied to visible
//! quality rather than a fixed count. The budget comes from the domain
//! [`TolerancePolicy`], never from a global constant.

use cad_domain::entity::{EntityGeometry, PolyVertex};
use cad_domain::TolerancePolicy;
use glam::{DVec2, DVec3};

use crate::polyline::Polyline;

/// Tessellation controls.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TessellationParams {
    /// Maximum allowed chord deviation in world units.
    pub tolerance: f64,
    pub max_segments: usize,
    pub min_segments: usize,
}

impl Default for TessellationParams {
    fn default() -> Self {
        TessellationParams { tolerance: 0.01, max_segments: 4096, min_segments: 8 }
    }
}

impl TessellationParams {
    /// Derive a world-space tolerance from a screen-space error and zoom.
    pub fn from_screen(screen_error_px: f64, world_per_px: f64) -> Self {
        let tol = (screen_error_px * world_per_px).max(1e-12);
        TessellationParams { tolerance: tol, ..Self::default() }
    }
}

/// Build tessellation parameters from the document tolerance policy.
pub fn params_from_policy(policy: &TolerancePolicy, world_per_px: f64) -> TessellationParams {
    TessellationParams {
        tolerance: policy.display_chord_at(world_per_px),
        ..TessellationParams::default()
    }
}

/// Segments needed so an arc of `radius` and `sweep` keeps its sagitta bounded.
pub fn arc_segments_for_tolerance(radius: f64, sweep: f64, params: TessellationParams) -> usize {
    let sweep = sweep.abs();
    if sweep <= 1e-12 {
        return 1;
    }
    let radius = radius.abs().max(1e-12);
    let ratio = 1.0 - (params.tolerance / radius);
    let step = if ratio <= -1.0 {
        std::f64::consts::FRAC_PI_2
    } else {
        2.0 * ratio.clamp(-1.0, 1.0).acos()
    };
    let step = step.max(1e-6);
    let n = (sweep / step).ceil() as usize;
    n.clamp(params.min_segments.max(1), params.max_segments.max(1))
}

fn normalize_ccw_sweep(sweep: f64) -> f64 {
    let tau = std::f64::consts::TAU;
    let mut s = sweep % tau;
    if s <= 0.0 {
        s += tau;
    }
    s
}

/// Append an arc (angles in the XY plane) as world-space points.
pub fn tessellate_arc(
    center: DVec3,
    radius: f64,
    start_angle: f64,
    end_angle: f64,
    params: TessellationParams,
    out: &mut Vec<DVec3>,
    include_first: bool,
) {
    let sweep = normalize_ccw_sweep(end_angle - start_angle);
    let n = arc_segments_for_tolerance(radius, sweep, params);
    out.reserve(n + 1);
    for i in 0..=n {
        if i == 0 && !include_first {
            continue;
        }
        let t = start_angle + sweep * (i as f64) / (n as f64);
        out.push(center + DVec3::new(t.cos() * radius, t.sin() * radius, 0.0));
    }
}

pub fn tessellate_circle(center: DVec3, radius: f64, params: TessellationParams, out: &mut Vec<DVec3>) {
    tessellate_arc(center, radius, 0.0, std::f64::consts::TAU, params, out, true);
}

/// Tessellate an ellipse arc into XY-plane points (before OCS transform).
pub fn tessellate_ellipse(
    center: DVec3,
    major_axis: DVec3,
    ratio: f64,
    start_param: f64,
    end_param: f64,
    params: TessellationParams,
    out: &mut Vec<DVec3>,
) {
    let major_len = major_axis.length();
    if major_len <= 1e-12 {
        out.push(center);
        return;
    }
    let minor = major_axis.normalize().cross(DVec3::Z) * major_len * ratio;
    let raw = end_param - start_param;
    let full = raw.abs() >= std::f64::consts::TAU - 1e-9;
    let sweep = if full { std::f64::consts::TAU } else { normalize_ccw_sweep(raw) };
    let r = major_len.max(major_len * ratio.abs());
    let n = arc_segments_for_tolerance(r, sweep, params);
    out.reserve(n + 1);
    for i in 0..=n {
        let t = start_param + sweep * (i as f64) / (n as f64);
        out.push(center + major_axis * t.cos() + minor * t.sin());
    }
}

/// Tessellate a clamped uniform B-spline from control points.
///
/// The domain keeps only control points and degree (no knots/weights), so the
/// result is an approximation and callers must label it as such.
pub fn tessellate_spline(
    control_points: &[DVec3],
    degree: u32,
    closed: bool,
    params: TessellationParams,
) -> Vec<DVec3> {
    let n = control_points.len();
    let k = (degree as usize).max(1).min(n.saturating_sub(1));
    if n == 0 {
        return Vec::new();
    }
    if n <= k {
        let mut v = control_points.to_vec();
        if closed && !v.is_empty() {
            v.push(v[0]);
        }
        return v;
    }
    let knots = clamped_uniform_knots(n, k);
    let spans = n - k;
    let per_span = (params.max_segments / spans.max(1)).clamp(8, 64);
    let mut out = Vec::with_capacity(spans * per_span + 1);
    let t_max = knots[n];
    for s in 0..=(spans * per_span) {
        let t = t_max * (s as f64) / ((spans * per_span) as f64);
        out.push(de_boor(control_points, &knots, k, t));
    }
    if closed {
        if let Some(first) = out.first().copied() {
            out.push(first);
        }
    }
    out
}

fn clamped_uniform_knots(n: usize, k: usize) -> Vec<f64> {
    let m = n + k + 1;
    let mut knots = vec![0.0; m];
    for i in 0..=k {
        knots[n + i] = 1.0;
    }
    let inner = n.saturating_sub(k + 1);
    if inner > 0 {
        for i in 1..=inner {
            knots[k + i] = i as f64 / (inner + 1) as f64;
        }
    }
    knots
}

fn de_boor(pts: &[DVec3], knots: &[f64], k: usize, t: f64) -> DVec3 {
    let n = pts.len();
    let mut span = k;
    while span < n - 1 && t >= knots[span + 1] {
        span += 1;
    }
    let mut d: Vec<DVec3> = (0..=k).map(|j| pts[span - k + j]).collect();
    for r in 1..=k {
        for j in (r..=k).rev() {
            let i = span - k + j;
            let denom = knots[i + k + 1 - r] - knots[i];
            let alpha = if denom.abs() > 1e-12 { (t - knots[i]) / denom } else { 0.0 };
            d[j] = d[j - 1] * (1.0 - alpha) + d[j] * alpha;
        }
    }
    d[k]
}

/// Tessellate any drawable entity into world-space polylines.
///
/// Meshes and inserts yield no polylines here (they are handled by the
/// representation layer as triangles / instances).
pub fn tessellate_entity(geometry: &EntityGeometry, params: TessellationParams) -> Vec<Polyline> {
    use EntityGeometry as G;
    match geometry {
        G::Line { start, end } => vec![Polyline::open(vec![*start, *end])],
        G::Polyline { vertices, closed, elevation } => {
            vec![polyline_with_bulges(vertices, *closed, *elevation, params)]
        }
        G::Polyline3D { points, closed } => vec![Polyline { pts: points.clone(), closed: *closed }],
        G::Circle { center, normal, radius } => {
            let (ax, ay, _az) = crate::ocs::ocs_to_wcs(*normal);
            let mut local = Vec::new();
            tessellate_circle(DVec3::ZERO, *radius, params, &mut local);
            let pts = local.into_iter().map(|p| *center + ax * p.x + ay * p.y).collect();
            vec![Polyline::closed(pts)]
        }
        G::Arc { center, normal, radius, start_angle, end_angle } => {
            let (ax, ay, _az) = crate::ocs::ocs_to_wcs(*normal);
            let mut local = Vec::new();
            tessellate_arc(DVec3::ZERO, *radius, *start_angle, *end_angle, params, &mut local, true);
            let pts = local.into_iter().map(|p| *center + ax * p.x + ay * p.y).collect();
            vec![Polyline::open(pts)]
        }
        G::Ellipse { center, normal, major_axis, ratio, start_param, end_param } => {
            let (ax, ay, _az) = crate::ocs::ocs_to_wcs(*normal);
            let mut local = Vec::new();
            tessellate_ellipse(DVec3::ZERO, *major_axis, *ratio, *start_param, *end_param, params, &mut local);
            let pts = local.into_iter().map(|p| *center + ax * p.x + ay * p.y).collect();
            vec![Polyline::open(pts)]
        }
        G::Spline { control_points, degree, closed } => vec![Polyline {
            pts: tessellate_spline(control_points, *degree, *closed, params),
            closed: false,
        }],
        G::Point { position } => vec![Polyline::open(vec![*position, *position])],
        G::Solid { corners } => {
            let c = corners;
            vec![Polyline::closed(vec![c[0], c[1], c[3], c[2]])]
        }
        G::Hatch { loops, .. } => loops
            .iter()
            .map(|l| Polyline {
                pts: l.vertices.iter().map(|p| DVec3::new(p.x, p.y, 0.0)).collect(),
                closed: true,
            })
            .collect(),
        G::ProxyStrokes { strokes, .. } => strokes
            .iter()
            .map(|s| Polyline { pts: s.points.clone(), closed: s.closed })
            .collect(),
        G::Mesh(_)
        | G::Text(_)
        | G::Insert { .. }
        | G::Dimension { .. }
        | G::ProxyPlaceholder { .. }
        | G::Unknown { .. } => Vec::new(),
    }
}

/// Convert a 2D polyline with bulges into a tessellated 3D polyline.
pub fn polyline_with_bulges(
    vertices: &[PolyVertex],
    closed: bool,
    elevation: f64,
    params: TessellationParams,
) -> Polyline {
    let mut out = Vec::with_capacity(vertices.len() * 4);
    if vertices.is_empty() {
        return Polyline::open(out);
    }
    let count = vertices.len();
    let last = if closed { count } else { count.saturating_sub(1) };
    for i in 0..last {
        let a = vertices[i];
        let b = vertices[(i + 1) % count];
        let pa = DVec3::new(a.pos.x, a.pos.y, elevation);
        if out.is_empty() {
            out.push(pa);
        }
        if a.bulge.abs() < 1e-12 {
            out.push(DVec3::new(b.pos.x, b.pos.y, elevation));
        } else {
            append_bulge_arc(pa, b.pos, a.bulge, elevation, params, &mut out);
        }
    }
    Polyline { pts: out, closed }
}

/// Append the arc described by a bulge from `a` to `b`.
///
/// `bulge = tan(theta/4)`; positive means a counter-clockwise arc.
fn append_bulge_arc(
    a: DVec3,
    b: DVec2,
    bulge: f64,
    z: f64,
    params: TessellationParams,
    out: &mut Vec<DVec3>,
) {
    let b3 = DVec3::new(b.x, b.y, z);
    let chord = b3 - a;
    let chord_len = chord.length();
    if chord_len < 1e-12 {
        out.push(b3);
        return;
    }
    let theta = 4.0 * bulge.atan();
    let half = theta * 0.5;
    let half_chord = chord_len * 0.5;
    let sagitta = bulge * half_chord;
    if sagitta.abs() < 1e-12 {
        out.push(b3);
        return;
    }
    let radius_abs = (half_chord * half_chord + sagitta * sagitta) / (2.0 * sagitta.abs());
    let mid = (a + b3) * 0.5;
    let dir = DVec3::new(chord.x, chord.y, 0.0) / chord_len;
    let left = DVec3::new(-dir.y, dir.x, 0.0);
    let u = if sagitta >= 0.0 { left } else { -left };
    let center = mid - u * (radius_abs * half.cos());
    let start_angle = (a - center).y.atan2((a - center).x);
    let n = arc_segments_for_tolerance(radius_abs, theta, params);
    for i in 1..=n {
        let t = start_angle + theta * (i as f64) / (n as f64);
        out.push(center + DVec3::new(t.cos() * radius_abs, t.sin() * radius_abs, 0.0));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bulge_semicircle_hits_lower_apex_for_ccw() {
        let a = DVec3::new(0.0, 0.0, 0.0);
        let b = DVec2::new(2.0, 0.0);
        let mut out = vec![a];
        append_bulge_arc(a, b, 1.0, 0.0, TessellationParams::default(), &mut out);
        let apex = out.iter().find(|p| (p.x - 1.0).abs() < 1e-6).unwrap();
        assert!((apex.y + 1.0).abs() < 1e-6, "apex y = {}", apex.y);
    }

    #[test]
    fn negative_bulge_mirrors_the_arc() {
        let a = DVec3::new(0.0, 0.0, 0.0);
        let b = DVec2::new(2.0, 0.0);
        let mut out = vec![a];
        append_bulge_arc(a, b, -1.0, 0.0, TessellationParams::default(), &mut out);
        let apex = out.iter().find(|p| (p.x - 1.0).abs() < 1e-6).unwrap();
        assert!((apex.y - 1.0).abs() < 1e-6, "apex y = {}", apex.y);
    }

    #[test]
    fn spline_endpoints_are_clamped() {
        let pts = vec![
            DVec3::new(0.0, 0.0, 0.0),
            DVec3::new(1.0, 2.0, 0.0),
            DVec3::new(2.0, 0.0, 0.0),
        ];
        let s = tessellate_spline(&pts, 2, false, TessellationParams::default());
        assert!((s.first().unwrap() - pts[0]).length() < 1e-9);
        assert!((s.last().unwrap() - pts[2]).length() < 1e-3);
    }

    #[test]
    fn policy_drives_chord_error() {
        let policy = TolerancePolicy::default();
        let coarse = params_from_policy(&policy, 1.0).tolerance;
        let fine = params_from_policy(&policy, 0.001).tolerance;
        assert!(fine < coarse);
    }
}
