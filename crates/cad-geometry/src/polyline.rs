//! Polylines, snapping candidates and length measurement (spec v2.0 §3.3).

use glam::DVec3;
use serde::{Deserialize, Serialize};

/// A tessellated, world-space polyline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Polyline {
    pub pts: Vec<DVec3>,
    pub closed: bool,
}

impl Polyline {
    pub fn open(pts: Vec<DVec3>) -> Self {
        Polyline { pts, closed: false }
    }

    pub fn closed(pts: Vec<DVec3>) -> Self {
        Polyline { pts, closed: true }
    }

    pub fn is_empty(&self) -> bool {
        self.pts.len() < 2
    }

    pub fn segments(&self) -> impl Iterator<Item = (DVec3, DVec3)> + '_ {
        let n = self.pts.len();
        let closed = self.closed;
        (0..n.saturating_sub(1))
            .map(move |i| (self.pts[i], self.pts[i + 1]))
            .chain(if closed && n > 2 { Some((self.pts[n - 1], self.pts[0])) } else { None })
    }
}

pub fn segments(p: &Polyline) -> impl Iterator<Item = (DVec3, DVec3)> + '_ {
    p.segments()
}

/// Axis-aligned bounds of a point set.
pub fn bounds(pts: &[DVec3]) -> Option<(DVec3, DVec3)> {
    let mut it = pts.iter();
    let first = *it.next()?;
    let mut min = first;
    let mut max = first;
    for p in it {
        min = min.min(*p);
        max = max.max(*p);
    }
    Some((min, max))
}

/// Total length of a polyline in world units.
pub fn polyline_length(p: &Polyline) -> f64 {
    p.segments().map(|(a, b)| (b - a).length()).sum()
}

/// Snap kind recognised by the initial snapping set (spec §3.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SnapKind {
    Endpoint,
    Midpoint,
    Center,
    Nearest,
    Intersection,
}

/// A candidate snap point returned by a local query.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SnapCandidate {
    pub point: DVec3,
    pub kind: SnapKind,
    pub distance: f64,
    pub segment: Option<usize>,
}

/// Closest candidate on a polyline to `query`.
pub fn closest_point_on_polyline(p: &Polyline, query: DVec3) -> Option<SnapCandidate> {
    let mut best: Option<SnapCandidate> = None;
    let mut consider = |point: DVec3, kind: SnapKind, seg: usize| {
        let d = (point - query).length();
        if best.map(|b| d < b.distance).unwrap_or(true) {
            best = Some(SnapCandidate { point, kind, distance: d, segment: Some(seg) });
        }
    };
    for (i, (a, b)) in p.segments().enumerate() {
        consider(a, SnapKind::Endpoint, i);
        consider(b, SnapKind::Endpoint, i);
        consider((a + b) * 0.5, SnapKind::Midpoint, i);
        consider(project_point_segment(query, a, b), SnapKind::Nearest, i);
    }
    best
}

/// Project `p` onto segment `a..b`, clamped to the segment.
pub fn project_point_segment(p: DVec3, a: DVec3, b: DVec3) -> DVec3 {
    let ab = b - a;
    let len2 = ab.length_squared();
    if len2 < 1e-24 {
        return a;
    }
    let t = ((p - a).dot(ab) / len2).clamp(0.0, 1.0);
    a + ab * t
}

/// Distance from a point to a segment.
pub fn point_segment_distance(p: DVec3, a: DVec3, b: DVec3) -> f64 {
    (p - project_point_segment(p, a, b)).length()
}

/// Intersection of two segments in the XY plane, if within both.
pub fn segment_intersection(a1: DVec3, a2: DVec3, b1: DVec3, b2: DVec3) -> Option<DVec3> {
    let r = a2 - a1;
    let s = b2 - b1;
    let denom = r.x * s.y - r.y * s.x;
    if denom.abs() < 1e-12 {
        return None;
    }
    let qp = b1 - a1;
    let t = (qp.x * s.y - qp.y * s.x) / denom;
    let u = (qp.x * r.y - qp.y * r.x) / denom;
    if (0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u) {
        Some(a1 + r * t)
    } else {
        None
    }
}
