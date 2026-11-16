//! The clip **boundary**: a simple polygon, its fill rule and a triangulation.
//!
//! A clip is not restricted to an axis-aligned rectangle. A stored ring is
//! validated here as one **simple** (non self-intersecting) polygon and carried
//! as [`PaperClip`]; the clippers in [`super::clip_polygon`] then limit every
//! primitive to that region. Two consumers share this module:
//!
//! * the **paper-space viewport** boundary (`MVIEW`/`VPCLIP`), built by
//!   [`super::build`] from a stored `Vec<Point3>`;
//! * the **world-space block-reference** boundary (`XCLIP`), built by
//!   [`crate::provider::block_clip_boundary`] by mapping the stored block-local
//!   ring through the composed INSERT transform. It is carried as
//!   [`WorldRectBoundary`], the rectangle-or-simple-polygon region a block clip
//!   keeps (nested references can intersect into several disjoint pieces).
//!
//! Two shapes are distinguished, and the distinction is behavioural, not
//! cosmetic:
//!
//! * an **axis-aligned rectangle** keeps the dedicated half-plane clippers
//!   (`super::clip`), so a rectangular clip produces exactly the geometry and
//!   diagnostics it produced before polygons were supported;
//! * anything else is clipped with a general even-odd / crossing algorithm.
//!
//! A boundary that cannot be used is refused with a stable
//! [`super::viewport_reason`] code — never squared off, boxed or dropped.

use cad_domain::Point3;

use super::selection::ViewportUnsupported;
use super::viewport_reason;

/// Relative tolerance for every boundary test in this module; it is scaled by
/// the polygon's own extent so a millimetre clip and a metre clip behave alike.
const RELATIVE_TOLERANCE: f64 = 1e-9;

/// One validated, simple paper-space clip boundary.
///
/// `boundary` is counter-clockwise, does not repeat its first vertex, and has at
/// least three vertices. `min`/`max` are its axis-aligned envelope: a cheap
/// rejection box that also lets a rectangular clip keep its exact old path.
#[derive(Debug, Clone, PartialEq)]
pub struct PaperClip {
    boundary: Vec<[f64; 2]>,
    min: [f64; 2],
    max: [f64; 2],
    /// `Some` only when the boundary is exactly an axis-aligned rectangle.
    rect: Option<([f64; 2], [f64; 2])>,
    /// Whether every turn is a left turn (collinear vertices allowed).
    convex: bool,
}

impl PaperClip {
    /// Wrap an already-validated counter-clockwise boundary.
    ///
    /// `rect` must be `Some` when (and only when) the boundary is exactly the
    /// four corners of that axis-aligned rectangle.
    pub(super) fn new(boundary: Vec<[f64; 2]>, rect: Option<([f64; 2], [f64; 2])>) -> Self {
        let (min, max) = envelope(&boundary);
        let convex = is_convex(&boundary);
        PaperClip {
            boundary,
            min,
            max,
            rect,
            convex,
        }
    }

    /// The clip that is exactly the axis-aligned rectangle `[min, max]`.
    pub fn rect(min: [f64; 2], max: [f64; 2]) -> Self {
        let boundary = vec![
            [min[0], min[1]],
            [max[0], min[1]],
            [max[0], max[1]],
            [min[0], max[1]],
        ];
        PaperClip {
            boundary,
            min,
            max,
            rect: Some((min, max)),
            convex: true,
        }
    }

    /// The boundary vertices, counter-clockwise, without a repeated close.
    pub fn boundary(&self) -> &[[f64; 2]] {
        &self.boundary
    }

    /// The axis-aligned envelope as `(min, max)`.
    pub fn bounds(&self) -> ([f64; 2], [f64; 2]) {
        (self.min, self.max)
    }

    /// The envelope centre, used as the viewport's paper centre.
    pub fn center(&self) -> [f64; 2] {
        [
            (self.min[0] + self.max[0]) * 0.5,
            (self.min[1] + self.max[1]) * 0.5,
        ]
    }

    /// The envelope half-extent, used as the viewport's paper half-extent.
    pub fn half(&self) -> [f64; 2] {
        [
            (self.max[0] - self.min[0]) * 0.5,
            (self.max[1] - self.min[1]) * 0.5,
        ]
    }

    /// The rectangle this clip is, if it is exactly an axis-aligned rectangle.
    pub fn as_rect(&self) -> Option<([f64; 2], [f64; 2])> {
        self.rect
    }

    /// Whether every turn is convex, so a half-plane clipper is exact for it.
    pub fn is_convex(&self) -> bool {
        self.convex
    }

    /// Whether `point` is inside the clip.
    ///
    /// A rectangular clip keeps the inclusive half-open-free test the
    /// half-plane clippers use (`min <= p <= max`), so both paths agree on the
    /// boundary. A general polygon uses the even-odd crossing rule and counts a
    /// point lying exactly **on** the boundary as inside: the viewport edge
    /// belongs to the visible region.
    pub fn contains(&self, point: [f64; 2]) -> bool {
        if let Some((min, max)) = self.rect {
            return point[0] >= min[0]
                && point[0] <= max[0]
                && point[1] >= min[1]
                && point[1] <= max[1];
        }
        if point[0] < self.min[0]
            || point[0] > self.max[0]
            || point[1] < self.min[1]
            || point[1] > self.max[1]
        {
            return false;
        }
        let n = self.boundary.len();
        let mut inside = false;
        for index in 0..n {
            let a = self.boundary[index];
            let b = self.boundary[(index + 1) % n];
            if point_on_segment(point, a, b, self.tolerance()) {
                return true;
            }
            if (a[1] > point[1]) != (b[1] > point[1]) {
                let t = (point[1] - a[1]) / (b[1] - a[1]);
                if a[0] + (b[0] - a[0]) * t > point[0] {
                    inside = !inside;
                }
            }
        }
        inside
    }

    /// Whether every point lies inside this clip (boundary inclusive).
    ///
    /// Used as the "nothing to cut" fast path for a primitive wholly inside the
    /// region. Point containment is exact for a rectangle and for a convex
    /// polygon's hull; a concave clip is still accepted here only when every
    /// point is inside it (the region test the caller asked for).
    pub fn holds_all_points<'a>(&self, points: impl IntoIterator<Item = &'a Point3>) -> bool {
        points.into_iter().all(|p| self.contains([p.x, p.y]))
    }

    /// Whether a whole convex-or-concave subject boundary lies inside the clip.
    ///
    /// True only when every subject vertex is inside **and** no subject edge
    /// crosses a clip edge: with a concave clip, four corners inside do not
    /// imply the quad is inside (a notch can cut through its interior while
    /// leaving every corner inside a different lobe).
    pub(super) fn holds(&self, subject: &[[f64; 2]]) -> bool {
        if subject.len() < 3 {
            return false;
        }
        if !subject.iter().all(|p| self.contains(*p)) {
            return false;
        }
        let tolerance = self.tolerance();
        let n = self.boundary.len();
        for pair in subject.windows(2) {
            for index in 0..n {
                let a = self.boundary[index];
                let b = self.boundary[(index + 1) % n];
                if properly_crosses(pair[0], pair[1], a, b, tolerance) {
                    return false;
                }
            }
        }
        true
    }

    /// Triangulate the clip into counter-clockwise convex windows.
    ///
    /// A concave clip has no half-plane representation, so a subject triangle is
    /// intersected with these windows one by one; because the windows partition
    /// the clip's interior, the union of the pairwise intersections is exactly
    /// the intersection with the clip (no area is lost or double counted).
    pub fn triangles(&self) -> Vec<[[f64; 2]; 3]> {
        triangulate(&self.boundary)
    }

    /// Wrap a **validated**, counter-clockwise boundary, re-detecting the
    /// axis-aligned rectangle case so [`Self::as_rect`] keeps the half-plane
    /// clippers in use for it.
    fn from_validated_boundary(boundary: Vec<[f64; 2]>) -> Self {
        let rect = rect_of_boundary(&boundary);
        PaperClip::new(boundary, rect)
    }

    /// The tolerance this clip's own extent justifies.
    fn tolerance(&self) -> f64 {
        RELATIVE_TOLERANCE
            * (self.max[0] - self.min[0])
                .max(self.max[1] - self.min[1])
                .max(f64::MIN_POSITIVE)
    }
}

/// Validate a stored clip ring as one simple polygon in the clip plane.
///
/// This is the **one** rule that decides whether a stored ring can be used,
/// shared by the paper-space viewport boundary ([`validate_paper_polygon`]) and
/// by the world-space block-reference (`XCLIP`) boundary built in
/// [`crate::provider::block_clip_boundary`]. A ring that fails is refused by
/// name — never squared off, widened or drawn partially.
///
/// Stable refusals, all in the `viewport.` family:
///
/// * [`viewport_reason::CLIP_ARITY`] — fewer than three points;
/// * [`viewport_reason::NON_FINITE`] — a non-finite coordinate;
/// * [`viewport_reason::POLYGON_DEGENERATE`] — a repeated vertex or zero area;
/// * [`viewport_reason::POLYGON_SELF_INTERSECTING`] — two non-adjacent edges
///   cross or touch.
///
/// The returned ring is counter-clockwise, has no repeated closing vertex and is
/// ready for [`PaperClip::new`] / [`WorldRectBoundary::ring`].
pub fn validate_simple_polygon(points: &[[f64; 2]]) -> Result<Vec<[f64; 2]>, ViewportUnsupported> {
    if points.len() < 3 {
        return Err(ViewportUnsupported::new(
            viewport_reason::CLIP_ARITY,
            format!(
                "a clip polygon needs at least three points; found {}",
                points.len()
            ),
        ));
    }
    if !points
        .iter()
        .all(|point| point[0].is_finite() && point[1].is_finite())
    {
        return Err(ViewportUnsupported::new(
            viewport_reason::NON_FINITE,
            "a clip polygon has non-finite coordinates",
        ));
    }
    let (min, max) = envelope(points);
    let extent = (max[0] - min[0]).max(max[1] - min[1]);
    if extent <= 0.0 {
        return Err(ViewportUnsupported::new(
            viewport_reason::POLYGON_DEGENERATE,
            "clip polygon has no extent; every vertex coincides",
        ));
    }
    let tolerance = RELATIVE_TOLERANCE * extent;
    let count = points.len();
    for index in 0..count {
        let next = (index + 1) % count;
        let (a, b) = (points[index], points[next]);
        if (a[0] - b[0]).abs() <= tolerance && (a[1] - b[1]).abs() <= tolerance {
            return Err(ViewportUnsupported::new(
                viewport_reason::POLYGON_DEGENERATE,
                format!("clip vertices {index} and {next} coincide"),
            ));
        }
    }
    for first in 0..count {
        let first_next = (first + 1) % count;
        for second in (first + 1)..count {
            let second_next = (second + 1) % count;
            // Adjacent edges share a vertex by construction; only an edge that
            // meets an edge it shares no vertex with is a self-intersection.
            if second == first_next || second_next == first {
                continue;
            }
            if properly_crosses(
                points[first],
                points[first_next],
                points[second],
                points[second_next],
                tolerance,
            ) {
                return Err(ViewportUnsupported::new(
                    viewport_reason::POLYGON_SELF_INTERSECTING,
                    format!(
                        "clip edges {first}-{first_next} and {second}-{second_next} cross or \
                         touch; the boundary is not a simple polygon"
                    ),
                ));
            }
        }
    }
    // A symmetric bow tie already failed the self-intersection test above, so
    // only a genuinely collinear boundary reaches this check.
    let area = signed_area(points);
    if area.abs() <= tolerance * extent {
        return Err(ViewportUnsupported::new(
            viewport_reason::POLYGON_DEGENERATE,
            "clip polygon has zero area (collinear vertices)",
        ));
    }
    let mut boundary = points.to_vec();
    if area < 0.0 {
        boundary.reverse();
    }
    Ok(boundary)
}

/// Validate a stored clip as one simple, planar paper polygon.
///
/// The planarity half is paper-specific (a viewport clip must lie on its own
/// sheet); the ring itself is validated by [`validate_simple_polygon`], so a
/// viewport clip and a block clip accept exactly the same simple rings. Stable
/// refusals are that function's codes plus
/// [`viewport_reason::POLYGON_NON_PLANAR`].
///
/// The returned boundary is counter-clockwise, has no repeated closing vertex
/// and is ready for [`PaperClip::new`].
pub(super) fn validate_paper_polygon(
    points: &[Point3],
) -> Result<Vec<[f64; 2]>, ViewportUnsupported> {
    if points.len() < 3 {
        return Err(ViewportUnsupported::new(
            viewport_reason::CLIP_ARITY,
            format!(
                "a viewport clip polygon needs at least three points; found {}",
                points.len()
            ),
        ));
    }
    if !points.iter().all(super::view::finite_point) {
        return Err(ViewportUnsupported::new(
            viewport_reason::NON_FINITE,
            "viewport clip has non-finite coordinates",
        ));
    }
    let mut min = [f64::INFINITY; 3];
    let mut max = [f64::NEG_INFINITY; 3];
    for point in points {
        for (axis, value) in [point.x, point.y, point.z].into_iter().enumerate() {
            min[axis] = min[axis].min(value);
            max[axis] = max[axis].max(value);
        }
    }
    let extent = (max[0] - min[0]).max(max[1] - min[1]);
    if extent <= 0.0 {
        return Err(ViewportUnsupported::new(
            viewport_reason::POLYGON_DEGENERATE,
            "viewport clip polygon has no paper extent; every vertex coincides",
        ));
    }
    let tolerance = RELATIVE_TOLERANCE * extent;
    if max[2] - min[2] > tolerance {
        return Err(ViewportUnsupported::new(
            viewport_reason::POLYGON_NON_PLANAR,
            format!(
                "viewport clip spans z {}..{}; a paper clip boundary must lie on one plane",
                min[2], max[2]
            ),
        ));
    }
    let boundary: Vec<[f64; 2]> = points.iter().map(|p| [p.x, p.y]).collect();
    validate_simple_polygon(&boundary)
}

/// The axis-aligned envelope of a boundary.
fn envelope(boundary: &[[f64; 2]]) -> ([f64; 2], [f64; 2]) {
    let mut min = [f64::INFINITY; 2];
    let mut max = [f64::NEG_INFINITY; 2];
    for point in boundary {
        for axis in 0..2 {
            min[axis] = min[axis].min(point[axis]);
            max[axis] = max[axis].max(point[axis]);
        }
    }
    (min, max)
}

/// Twice the signed area of the boundary (positive when counter-clockwise).
fn signed_area(boundary: &[[f64; 2]]) -> f64 {
    let mut sum = 0.0;
    for index in 0..boundary.len() {
        let a = boundary[index];
        let b = boundary[(index + 1) % boundary.len()];
        sum += a[0] * b[1] - b[0] * a[1];
    }
    sum * 0.5
}

/// Whether every turn of a counter-clockwise boundary is a left turn.
fn is_convex(boundary: &[[f64; 2]]) -> bool {
    let n = boundary.len();
    if n < 3 {
        return true;
    }
    for index in 0..n {
        let a = boundary[index];
        let b = boundary[(index + 1) % n];
        let c = boundary[(index + 2) % n];
        if turn(a, b, c) < 0.0 {
            return false;
        }
    }
    true
}

/// The signed cross product `(b - a) x (c - a)`; positive is a left turn.
fn turn(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

/// Whether `point` lies on the closed segment `a`–`b`, within `tolerance`.
fn point_on_segment(point: [f64; 2], a: [f64; 2], b: [f64; 2], tolerance: f64) -> bool {
    if turn(a, b, point).abs() > tolerance * (b[0] - a[0]).hypot(b[1] - a[1]).max(tolerance) {
        return false;
    }
    point[0] >= a[0].min(b[0]) - tolerance
        && point[0] <= a[0].max(b[0]) + tolerance
        && point[1] >= a[1].min(b[1]) - tolerance
        && point[1] <= a[1].max(b[1]) + tolerance
}

/// Whether two segments cross or touch, within `tolerance`.
///
/// Collinear overlap and endpoint touching count as a crossing: a boundary that
/// merely grazes itself is not a simple polygon, and a subject edge that grazes
/// a clip edge is not safely classifiable as "inside" for a split decision.
pub(super) fn properly_crosses(
    a1: [f64; 2],
    a2: [f64; 2],
    b1: [f64; 2],
    b2: [f64; 2],
    tolerance: f64,
) -> bool {
    let limit = tolerance
        * (a2[0] - a1[0])
            .abs()
            .max((a2[1] - a1[1]).abs())
            .max((b2[0] - b1[0]).abs())
            .max((b2[1] - b1[1]).abs())
            .max(1.0);
    let d1 = turn(b1, b2, a1);
    let d2 = turn(b1, b2, a2);
    let d3 = turn(a1, a2, b1);
    let d4 = turn(a1, a2, b2);
    let opposite = |first: f64, second: f64| {
        first.abs() > limit && second.abs() > limit && first.signum() != second.signum()
    };
    if opposite(d1, d2) && opposite(d3, d4) {
        return true;
    }
    point_on_segment(a1, b1, b2, tolerance)
        || point_on_segment(a2, b1, b2, tolerance)
        || point_on_segment(b1, a1, a2, tolerance)
        || point_on_segment(b2, a1, a2, tolerance)
}

/// Whether `point` is inside the triangle `a`,`b`,`c` (either winding), boundary
/// included.
fn point_in_triangle(point: [f64; 2], a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> bool {
    let area = turn(a, b, c);
    if area == 0.0 {
        return false;
    }
    let sign = if area > 0.0 { 1.0 } else { -1.0 };
    sign * turn(a, b, point) >= 0.0
        && sign * turn(b, c, point) >= 0.0
        && sign * turn(c, a, point) >= 0.0
}

/// Ear-clip a counter-clockwise simple polygon into triangles.
///
/// A validated simple polygon always has an ear, so the loop terminates. The
/// last-resort branch exists only so a numerically degenerate input can never
/// hang the layout build: it drops the flattest vertex, which loses no area for
/// a truly collinear vertex and degrades a near-degenerate one by that sliver.
fn triangulate(boundary: &[[f64; 2]]) -> Vec<[[f64; 2]; 3]> {
    let n = boundary.len();
    if n < 3 {
        return Vec::new();
    }
    if n == 3 {
        return vec![[boundary[0], boundary[1], boundary[2]]];
    }
    let mut indices: Vec<usize> = (0..n).collect();
    let mut triangles = Vec::with_capacity(n - 2);
    while indices.len() > 3 {
        let count = indices.len();
        let mut ear = None;
        for k in 0..count {
            let (previous, current, next) = (
                indices[(k + count - 1) % count],
                indices[k],
                indices[(k + 1) % count],
            );
            let (a, b, c) = (boundary[previous], boundary[current], boundary[next]);
            if turn(a, b, c) <= 0.0 {
                continue;
            }
            let blocked = indices.iter().any(|index| {
                *index != previous
                    && *index != current
                    && *index != next
                    && point_in_triangle(boundary[*index], a, b, c)
            });
            if blocked {
                continue;
            }
            ear = Some((k, [a, b, c]));
            break;
        }
        let Some((k, triangle)) = ear else {
            // No ear found: numerically stuck. Drop the flattest vertex so the
            // loop always makes progress.
            let flattest = (0..count)
                .min_by(|left, right| {
                    let flatten = |k: usize| {
                        let (previous, current, next) = (
                            indices[(k + count - 1) % count],
                            indices[k],
                            indices[(k + 1) % count],
                        );
                        turn(boundary[previous], boundary[current], boundary[next]).abs()
                    };
                    flatten(*left)
                        .partial_cmp(&flatten(*right))
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .unwrap_or(0);
            indices.remove(flattest);
            continue;
        };
        triangles.push(triangle);
        indices.remove(k);
    }
    triangles.push([
        boundary[indices[0]],
        boundary[indices[1]],
        boundary[indices[2]],
    ]);
    triangles
}

/// The axis-aligned rectangle a validated ring is, if it is exactly one.
///
/// A simple four-vertex ring that uses all four corners of its own envelope can
/// only be that rectangle (any other ordering either leaves a corner unused or
/// crosses itself, and a crossing ring never reaches this function). The test is
/// therefore structural, not a comparison against an assumed corner order.
fn rect_of_boundary(boundary: &[[f64; 2]]) -> Option<([f64; 2], [f64; 2])> {
    if boundary.len() != 4 {
        return None;
    }
    let (min, max) = envelope(boundary);
    let width = max[0] - min[0];
    let height = max[1] - min[1];
    if width <= 0.0 || height <= 0.0 {
        return None;
    }
    let tolerance = RELATIVE_TOLERANCE * width.max(height);
    let mut seen = [false; 4];
    for point in boundary {
        let x = if (point[0] - min[0]).abs() <= tolerance {
            0
        } else if (point[0] - max[0]).abs() <= tolerance {
            1
        } else {
            return None;
        };
        let y = if (point[1] - min[1]).abs() <= tolerance {
            0
        } else if (point[1] - max[1]).abs() <= tolerance {
            1
        } else {
            return None;
        };
        let corner = x * 2 + y;
        if seen[corner] {
            return None;
        }
        seen[corner] = true;
    }
    seen.iter().all(|used| *used).then_some((min, max))
}

/// The world-space region a block reference's `XCLIP` boundary keeps.
///
/// The region is **one or more simple polygons** whose union is exactly what the
/// stored boundary maps to:
///
/// * a boundary that maps to an axis-aligned world-XY rectangle is stored as a
///   single rectangle piece, so the dedicated half-plane clippers
///   (`super::clip`) run and the output is byte-for-byte the pre-polygon
///   behaviour;
/// * anything else — a concave polygon, a **rotated or sheared rectangle** — is
///   the ring its own points span, never its axis-aligned envelope;
/// * more than one piece appears only when a nested reference's boundary is
///   intersected with its parent's (`rect ∩ concave ring` can leave two
///   disjoint lobes). The pieces are disjoint and their union is the exact
///   intersection, so a primitive is clipped piece by piece and the union of the
///   results is `primitive ∩ clip`.
///
/// Every clipper that produces geometry goes through [`Self::pieces`] and the
/// shared dispatchers in [`super::clip_polygon`] — there is no second clipper.
/// [`Self::contains`] is only the point-classification view of the same region
/// (tests and reports use it; the clippers never need it).
#[derive(Debug, Clone, PartialEq)]
pub struct WorldRectBoundary {
    pieces: Vec<PaperClip>,
}

impl WorldRectBoundary {
    /// The region that is exactly the axis-aligned rectangle `[min, max]`.
    ///
    /// `min`/`max` must already be ordered and non-degenerate; the provider's
    /// axis-aligned oracle ([`crate::provider::block_clip_rect`]) guarantees both.
    pub fn rect(min: [f64; 2], max: [f64; 2]) -> Self {
        WorldRectBoundary {
            pieces: vec![PaperClip::rect(min, max)],
        }
    }

    /// Validate and wrap a world-XY ring (any winding) as the kept region.
    ///
    /// The ring is validated by [`validate_simple_polygon`], so `<3` points, a
    /// repeated vertex, zero area or a self-intersection is refused by name while
    /// the caller's geometry stays unclipped. An axis-aligned rectangle is
    /// recognized here and keeps the half-plane clippers.
    pub fn ring(points: &[[f64; 2]]) -> Result<Self, ViewportUnsupported> {
        let boundary = validate_simple_polygon(points)?;
        Ok(WorldRectBoundary {
            pieces: vec![PaperClip::from_validated_boundary(boundary)],
        })
    }

    /// The disjoint simple polygons whose union is the region.
    pub fn pieces(&self) -> &[PaperClip] {
        &self.pieces
    }

    /// The rectangle this region is, when it is exactly one.
    pub fn as_rect(&self) -> Option<([f64; 2], [f64; 2])> {
        match self.pieces.as_slice() {
            [only] => only.as_rect(),
            _ => None,
        }
    }

    /// Whether `point` is inside the region (boundary inclusive).
    pub fn contains(&self, point: [f64; 2]) -> bool {
        self.pieces.iter().any(|piece| piece.contains(point))
    }

    /// Whether every point lies inside the region (boundary inclusive).
    ///
    /// This is the "nothing can be cut" fast path for a primitive wholly inside
    /// the region: every mesh vertex and stored edge endpoint is tested against
    /// the union of the region's pieces. Any point outside keeps the exact
    /// clipper path.
    pub fn holds_all_points<'a>(&self, points: impl IntoIterator<Item = &'a Point3>) -> bool {
        points.into_iter().all(|p| self.contains([p.x, p.y]))
    }

    /// The exact intersection of two regions, or `None` when they are disjoint.
    ///
    /// Rectangle ∩ rectangle is computed as a rectangle (so a nested pair of
    /// rectangular clips stays on the byte-identical half-plane path);
    /// everything else is the existing filled-polygon clipper
    /// ([`super::clip_polygon::clip_filled_polygon`]) applied to one ring at a
    /// time, whose convex pieces are the exact intersection. A
    /// rectangle ∩ polygon pair is therefore refined into its real pieces rather
    /// than being widened to the rectangle or dropped.
    pub fn intersect(&self, other: &WorldRectBoundary) -> Option<WorldRectBoundary> {
        let mut pieces = Vec::new();
        for left in &self.pieces {
            for right in &other.pieces {
                pieces.extend(intersect_pieces(left, right));
            }
        }
        if pieces.is_empty() {
            None
        } else {
            Some(WorldRectBoundary { pieces })
        }
    }
}

/// One piece ∩ one piece, as disjoint simple polygon pieces.
fn intersect_pieces(left: &PaperClip, right: &PaperClip) -> Vec<PaperClip> {
    if let (Some((left_min, left_max)), Some((right_min, right_max))) =
        (left.as_rect(), right.as_rect())
    {
        let min = [left_min[0].max(right_min[0]), left_min[1].max(right_min[1])];
        let max = [left_max[0].min(right_max[0]), left_max[1].min(right_max[1])];
        if max[0] <= min[0] || max[1] <= min[1] {
            return Vec::new();
        }
        return vec![PaperClip::rect(min, max)];
    }
    super::clip_polygon::clip_filled_polygon(left.boundary(), right)
        .into_iter()
        .filter(|piece| piece.len() >= 3 && signed_area(piece) != 0.0)
        .map(PaperClip::from_validated_boundary)
        .collect()
}
