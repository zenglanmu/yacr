//! The paper-space clip **boundary** of a `VPCLIP`/`MVIEW` viewport.
//!
//! acadrust 0.6.8 carries one viewport-clip field — `clip_boundary_handle`
//! (DXF group 340, a hard pointer) plus the status bit `non_rectangular_clipping`
//! (group 90, bit `0x10000`). The vertices are not on the VIEWPORT record: the
//! handle points at a separate entity of the same drawing (an LWPOLYLINE or a 2D
//! POLYLINE), so the boundary is resolved through the document.
//!
//! A boundary this build can represent is stored verbatim as the file's own
//! polygon: a rectangle as its canonical four corners (byte-identical to the
//! rectangle-only reader) or an `N`-point simple polygon. A boundary it cannot
//! represent is a **named** refusal — never a squared-off rectangle, never
//! chorded arcs, and never a silent `Partial` a caller cannot see.

use acadrust::entities::PolylineFlags;

use super::*;

// ---- viewport clip boundary ----

/// Stable importer refusal codes for a VIEWPORT clip boundary that cannot be
/// stored as a paper polygon.
///
/// They are part of the import contract. A refusal is stored as the viewport's
/// [`Completeness::Partial`] reason with its code as the prefix, and
/// `ImporterBuilder::read_layout_viewports` lifts that prefix into the import
/// report's diagnostic code — so a boundary this build cannot represent is
/// always a **named** refusal, never a silently squared-off rectangle and never
/// a `Partial` a caller cannot see.
pub(crate) mod viewport_clip_reason {
    /// The boundary handle resolves to no entity in the drawing.
    pub const UNRESOLVED: &str = "import.viewport_clip_unresolved";
    /// The boundary entity is not a closed, straight-edge polyline.
    pub const UNSUPPORTED_ENTITY: &str = "import.viewport_clip_unsupported_entity";
    /// The boundary polyline is not closed, so it encloses no region.
    pub const NOT_CLOSED: &str = "import.viewport_clip_not_closed";
    /// A boundary segment is an arc (non-zero bulge).
    pub const CURVED: &str = "import.viewport_clip_curved";
    /// The boundary's OCS is not the paper plane.
    pub const NON_PLANAR: &str = "import.viewport_clip_non_planar";
    /// A boundary vertex is not finite.
    pub const NON_FINITE: &str = "import.viewport_clip_non_finite";
    /// Too few distinct vertices, or zero area.
    pub const DEGENERATE: &str = "import.viewport_clip_degenerate";
    /// The boundary's edges cross or touch: it is not a simple polygon.
    pub const SELF_INTERSECTING: &str = "import.viewport_clip_self_intersecting";
}

/// A refused clip boundary: a stable importer code plus the human detail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ClipBoundaryRefusal {
    code: &'static str,
    detail: String,
}

impl ClipBoundaryRefusal {
    fn new(code: &'static str, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: detail.into(),
        }
    }

    /// The stored [`Completeness::Partial`] reason, `"<code>: <detail>"`.
    ///
    /// The code prefix is deliberate: the stored viewport state is what the
    /// representation layer reads, and the import report lifts this prefix into
    /// its diagnostic code, so the refusal stays machine-identifiable in both.
    pub(crate) fn reason(&self) -> String {
        format!("{}: {}", self.code, self.detail)
    }
}

/// The import-report code for a stored viewport refusal reason.
///
/// [`ClipBoundaryRefusal::reason`] encodes the specific code as a `"<code>: "`
/// prefix; every other viewport reason (twist, perspective, an off-plane view
/// direction, a missing scale) keeps the generic code. A caller can therefore
/// branch on the report's diagnostic code alone.
pub(crate) fn viewport_reason_code(reason: &str) -> &str {
    match reason.split_once(": ") {
        Some((code, _)) if code.starts_with("import.") => code,
        _ => "import.viewport_unsupported",
    }
}

/// The paper-space clip boundary a paper VIEWPORT actually has.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ClipBoundary {
    /// No boundary handle: the viewport's own paper rectangle is the clip.
    ViewportRectangle,
    /// The boundary the file carries, straight edges only, in file vertex
    /// order: an axis-aligned rectangle as its canonical four corners (exactly
    /// the bytes the rectangle-only reader stored) or a general simple polygon
    /// (three or more points, no repeated closing vertex, on the paper plane).
    Area(Vec<Point3>),
    /// The handle is set, but the boundary cannot be represented.
    Refused(ClipBoundaryRefusal),
}

/// Read the clip boundary a DXF VIEWPORT (group 340) refers to.
///
/// acadrust 0.6.8 carries exactly one viewport-clip field: `clip_boundary_handle`
/// (DXF group 340, a hard pointer) plus the status bit `non_rectangular_clipping`
/// (group 90, bit `0x10000`). The vertices are **not** on the VIEWPORT record —
/// there are no inline clip-vertex groups to read — the handle points at a
/// separate entity of the same drawing, so the boundary is resolved through the
/// document. Nothing here invents geometry the file does not carry.
pub(crate) fn read_clip_boundary(
    v: &acadrust::entities::Viewport,
    acad: &acadrust::CadDocument,
) -> ClipBoundary {
    if v.clip_boundary_handle.null_or_value_zero() {
        return ClipBoundary::ViewportRectangle;
    }
    let Some(entity) = acad.get_entity(v.clip_boundary_handle) else {
        return ClipBoundary::Refused(ClipBoundaryRefusal::new(
            viewport_clip_reason::UNRESOLVED,
            format!(
                "the clip boundary handle {:X} resolves to no entity in the drawing, so the \
                 clipped region is unknown",
                v.clip_boundary_handle.value()
            ),
        ));
    };
    match clip_boundary_area(entity) {
        Ok(points) => ClipBoundary::Area(points),
        Err(refusal) => ClipBoundary::Refused(refusal),
    }
}

/// The straight-edge paper area a clip-boundary entity encloses.
///
/// LWPOLYLINE is what VPCLIP writes and 2D POLYLINE is its heavy-polyline
/// equivalent; both carry OCS vertices with bulges. Every other class — CIRCLE,
/// ARC, ELLIPSE, SPLINE, 3D POLYLINE, REGION — would have to be chorded into
/// vertices the file does not have, so it is refused by name instead.
fn clip_boundary_area(entity: &EntityType) -> Result<Vec<Point3>, ClipBoundaryRefusal> {
    match entity {
        EntityType::LwPolyline(p) => clip_ring(
            p.is_closed,
            p3(p.normal),
            p.elevation,
            p.vertices
                .iter()
                .map(|v| (v.location.x, v.location.y, v.bulge))
                .collect(),
        ),
        EntityType::Polyline2D(p) => {
            // A curve-fit / spline-fit polyline draws a fitted curve; its
            // vertices are control points, not the boundary. Accepting them
            // would be the same mistake as chording an arc.
            if p.flags.bits() & (PolylineFlags::CURVE_FIT.bits() | PolylineFlags::SPLINE_FIT.bits())
                != 0
            {
                return Err(ClipBoundaryRefusal::new(
                    viewport_clip_reason::CURVED,
                    "the clip boundary polyline is curve/spline fitted: its vertices are control \
                     points, not the boundary that is drawn",
                ));
            }
            clip_ring(
                p.flags.is_closed(),
                p3(p.normal),
                p.elevation,
                p.vertices
                    .iter()
                    .map(|v| (v.location.x, v.location.y, v.bulge))
                    .collect(),
            )
        }
        other => Err(ClipBoundaryRefusal::new(
            viewport_clip_reason::UNSUPPORTED_ENTITY,
            format!(
                "the clip boundary is a {}, not a closed straight-edge polyline; an arc or circle \
                 boundary would have to be chorded into vertices the file does not carry, so it \
                 is not approximated",
                entity_class_name(other)
            ),
        )),
    }
}

/// A closed boundary polyline as a paper-space vertex ring.
///
/// `vertices` are `(x, y, bulge)` in the polyline's OCS. Consecutive duplicates
/// are collapsed and a repeated closing vertex is dropped (real files store
/// both), then the ring is classified:
///
/// * four unique corners, each on the envelope and each consecutive pair sharing
///   a coordinate ⇒ the canonical axis-aligned rectangle, i.e. exactly the bytes
///   the rectangle-only reader produced for the same file;
/// * otherwise a validated simple polygon, in file vertex order.
fn clip_ring(
    closed: bool,
    normal: Point3,
    elevation: f64,
    vertices: Vec<(f64, f64, f64)>,
) -> Result<Vec<Point3>, ClipBoundaryRefusal> {
    if !closed {
        return Err(ClipBoundaryRefusal::new(
            viewport_clip_reason::NOT_CLOSED,
            "the clip boundary polyline is not closed, so it encloses no region",
        ));
    }
    if vertices.is_empty() {
        return Err(ClipBoundaryRefusal::new(
            viewport_clip_reason::DEGENERATE,
            "the clip boundary polyline has no vertices",
        ));
    }
    // Any bulge is an arc segment: the boundary is not a straight-edged polygon,
    // even if its vertices happen to line up.
    if let Some(index) = vertices
        .iter()
        .position(|(_, _, bulge)| bulge.abs() > 1e-12)
    {
        return Err(ClipBoundaryRefusal::new(
            viewport_clip_reason::CURVED,
            format!(
                "the clip boundary segment after vertex {index} is an arc (bulge {}); chording \
                 it would invent geometry the file does not carry",
                vertices[index].2
            ),
        ));
    }
    let normal = polyline_normal(normal);
    if !is_world_z(normal) {
        // A tilted OCS projects onto paper as some other shape; do not guess.
        return Err(ClipBoundaryRefusal::new(
            viewport_clip_reason::NON_PLANAR,
            format!(
                "the clip boundary's extrusion ({}, {}, {}) is tilted off the paper plane, so its \
                 paper projection is not the polygon the file stores",
                normal.x, normal.y, normal.z
            ),
        ));
    }
    let points: Vec<Point3> = vertices
        .iter()
        .map(|(x, y, _)| ocs_to_wcs(normal, elevation, *x, *y))
        .collect();
    if let Some(point) = points
        .iter()
        .find(|p| !p.x.is_finite() || !p.y.is_finite() || !p.z.is_finite())
    {
        return Err(ClipBoundaryRefusal::new(
            viewport_clip_reason::NON_FINITE,
            format!("the clip boundary has non-finite coordinates {point:?}"),
        ));
    }
    let (min_x, max_x, min_y, max_y) = envelope(&points);
    let extent = (max_x - min_x).max(max_y - min_y);
    if extent <= 0.0 {
        return Err(ClipBoundaryRefusal::new(
            viewport_clip_reason::DEGENERATE,
            "every clip boundary vertex coincides with the first",
        ));
    }
    // Relative tolerance: a real paper boundary is many units across, so 1e-9 of
    // its extent separates true vertices from duplicate ones without merging a
    // visibly non-rectangular outline.
    let tolerance = 1e-9 * extent;
    let ring = collapse_ring(&points, tolerance);
    if let Some(rectangle) = canonical_rectangle(&ring, tolerance) {
        return Ok(rectangle);
    }
    validate_simple_ring(&ring, tolerance, extent)?;
    Ok(ring)
}

/// The four paper corners of an axis-aligned rectangle, counter-clockwise from
/// the minimum corner (the encoding every rectangular clip stores).
pub(crate) fn rectangle_corners(min_x: f64, min_y: f64, max_x: f64, max_y: f64) -> Vec<Point3> {
    vec![
        Point3 {
            x: min_x,
            y: min_y,
            z: 0.0,
        },
        Point3 {
            x: max_x,
            y: min_y,
            z: 0.0,
        },
        Point3 {
            x: max_x,
            y: max_y,
            z: 0.0,
        },
        Point3 {
            x: min_x,
            y: max_y,
            z: 0.0,
        },
    ]
}

/// The `(min_x, max_x, min_y, max_y)` envelope of a point set.
fn envelope(points: &[Point3]) -> (f64, f64, f64, f64) {
    let min_x = points.iter().map(|p| p.x).fold(f64::INFINITY, f64::min);
    let max_x = points.iter().map(|p| p.x).fold(f64::NEG_INFINITY, f64::max);
    let min_y = points.iter().map(|p| p.y).fold(f64::INFINITY, f64::min);
    let max_y = points.iter().map(|p| p.y).fold(f64::NEG_INFINITY, f64::max);
    (min_x, max_x, min_y, max_y)
}

/// Collapse consecutive duplicate vertices and drop a repeated closing vertex.
fn collapse_ring(points: &[Point3], tolerance: f64) -> Vec<Point3> {
    let mut unique: Vec<Point3> = Vec::with_capacity(points.len());
    for point in points {
        if unique.last().is_some_and(|last| {
            (last.x - point.x).abs() <= tolerance && (last.y - point.y).abs() <= tolerance
        }) {
            continue;
        }
        unique.push(*point);
    }
    if unique.len() > 1 {
        let first = unique[0];
        let last = *unique.last().expect("non-empty");
        if (first.x - last.x).abs() <= tolerance && (first.y - last.y).abs() <= tolerance {
            unique.pop();
        }
    }
    unique
}

/// The canonical four corners of `ring` when it is exactly an axis-aligned
/// rectangle, otherwise `None`.
///
/// The corners are returned counter-clockwise from the minimum corner, which is
/// the exact encoding the rectangle-only reader stored, so a rectangular VPCLIP
/// boundary stays byte-identical. Consecutive corners must share a coordinate: a
/// bow tie that merely *uses* the four corners of the box is not a rectangle and
/// falls through to the polygon validator, which names it self-intersecting
/// instead of squaring it off.
fn canonical_rectangle(ring: &[Point3], tolerance: f64) -> Option<Vec<Point3>> {
    if ring.len() != 4 {
        return None;
    }
    let (min_x, max_x, min_y, max_y) = envelope(ring);
    if max_x - min_x <= tolerance || max_y - min_y <= tolerance {
        return None;
    }
    for point in ring {
        let on_x = (point.x - min_x).abs() <= tolerance || (point.x - max_x).abs() <= tolerance;
        let on_y = (point.y - min_y).abs() <= tolerance || (point.y - max_y).abs() <= tolerance;
        if !on_x || !on_y {
            return None;
        }
    }
    for index in 0..ring.len() {
        let a = ring[index];
        let b = ring[(index + 1) % ring.len()];
        if (a.x - b.x).abs() > tolerance && (a.y - b.y).abs() > tolerance {
            return None;
        }
    }
    Some(rectangle_corners(min_x, min_y, max_x, max_y))
}

/// Validate a ring as one simple paper polygon, naming every refusal.
///
/// This mirrors the representation layer's `validate_paper_polygon` (private to
/// that crate): the stored viewport claims `Complete`, so the importer must not
/// store a ring the representation would refuse without naming why here. The
/// tolerance is `1e-9 × extent`, the same relative tolerance.
fn validate_simple_ring(
    ring: &[Point3],
    tolerance: f64,
    extent: f64,
) -> Result<(), ClipBoundaryRefusal> {
    if ring.len() < 3 {
        return Err(ClipBoundaryRefusal::new(
            viewport_clip_reason::DEGENERATE,
            format!(
                "the clip boundary has {} distinct vertices; a polygon needs at least three",
                ring.len()
            ),
        ));
    }
    let count = ring.len();
    for index in 0..count {
        let next = (index + 1) % count;
        let (a, b) = (ring[index], ring[next]);
        if (a.x - b.x).abs() <= tolerance && (a.y - b.y).abs() <= tolerance {
            return Err(ClipBoundaryRefusal::new(
                viewport_clip_reason::DEGENERATE,
                format!("clip boundary vertices {index} and {next} coincide"),
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
            if ring_edges_meet(
                ring[first],
                ring[first_next],
                ring[second],
                ring[second_next],
                tolerance,
            ) {
                return Err(ClipBoundaryRefusal::new(
                    viewport_clip_reason::SELF_INTERSECTING,
                    format!(
                        "clip boundary edges {first}-{first_next} and {second}-{second_next} \
                         cross or touch; the boundary is not a simple polygon"
                    ),
                ));
            }
        }
    }
    let area = cad_geometry::signed_area(ring);
    if area.abs() <= tolerance * extent {
        return Err(ClipBoundaryRefusal::new(
            viewport_clip_reason::DEGENERATE,
            "clip boundary has zero area (collinear vertices)",
        ));
    }
    Ok(())
}

/// The signed cross product `(b - a) × (c - a)` on paper; positive is a left turn.
fn turn(a: Point3, b: Point3, c: Point3) -> f64 {
    (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)
}

/// Whether `point` lies on the closed segment `a`–`b`, within `tolerance`.
fn point_on_segment(point: Point3, a: Point3, b: Point3, tolerance: f64) -> bool {
    if turn(a, b, point).abs() > tolerance * (b.x - a.x).hypot(b.y - a.y).max(tolerance) {
        return false;
    }
    point.x >= a.x.min(b.x) - tolerance
        && point.x <= a.x.max(b.x) + tolerance
        && point.y >= a.y.min(b.y) - tolerance
        && point.y <= a.y.max(b.y) + tolerance
}

/// Whether two paper segments cross or touch, within `tolerance`.
///
/// Collinear overlap and endpoint touching count as meeting: a boundary that
/// merely grazes itself is not a simple polygon.
fn ring_edges_meet(a1: Point3, a2: Point3, b1: Point3, b2: Point3, tolerance: f64) -> bool {
    let limit = tolerance
        * (a2.x - a1.x)
            .abs()
            .max((a2.y - a1.y).abs())
            .max((b2.x - b1.x).abs())
            .max((b2.y - b1.y).abs())
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
