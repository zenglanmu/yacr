//! HATCH boundary classification, stitching glue, plane resolution and loop
//! construction (ADR 0018 decision D).
//!
//! Split out of `hatch.rs` to keep every file under the 1000-line gate. The
//! command (`create_hatch`) lives in the parent module; this module owns the
//! boundary half: which kinds are accepted, how `LINE`/`ARC` edges are stitched
//! into closed loops ([`super::hatch_stitch`]), how one shared plane/normal is
//! resolved, and how each accepted loop is written as a closed bulged polyline in
//! the hatch plane's 2D frame.

use super::*;

use super::hatch_stitch::{
    stitch_edges, StitchEdge, StitchedLoop, DEGENERATE_SWEEP, STITCH_TOLERANCE,
};

/// Number of chords a full selected ellipse is sampled into.
///
/// A sampled ellipse boundary is the declared approximation (a true ellipse is
/// not a circular arc); 64 chords keep the visible boundary within about 0.1% of
/// the curve radius while staying small in the DXF file.
const ELLIPSE_SEGMENTS: usize = 64;

/// The relative-plane tolerance: `1e-9` scaled by the boundary magnitude plus
/// one, so large drawings are not rejected for floating-point noise and exact
/// coincidences are still exact.
fn plane_tolerance(magnitude: f64) -> f64 {
    1e-9 * (1.0 + magnitude)
}

/// One accepted boundary entity, classified from its stored geometry.
pub(super) enum BoundaryShape {
    Circle {
        object: EntityId,
        center: Point3,
        normal: Point3,
        radius: f64,
    },
    Ellipse {
        object: EntityId,
        center: Point3,
        normal: Point3,
        major_axis: Point3,
        ratio: f64,
        start: f64,
        sweep: f64,
    },
    Polyline {
        object: EntityId,
        points: Vec<Point3>,
        bulges: Vec<f64>,
    },
    /// One `LINE` boundary edge, directed `start -> end` as stored.
    Line {
        object: EntityId,
        start: Point3,
        end: Point3,
    },
    /// One `ARC` boundary edge, directed along its sweep (a full turn is split
    /// into four quarter edges, see [`boundary_shapes`]).
    ///
    /// `start_point`/`end_point` are the arc's own endpoints, computed with the
    /// same AutoCAD arbitrary-axis frame the tessellator draws it in.
    Arc {
        object: EntityId,
        center: Point3,
        normal: Point3,
        sweep: f64,
        start_point: Point3,
        end_point: Point3,
    },
}

impl BoundaryShape {
    fn object(&self) -> EntityId {
        match self {
            Self::Circle { object, .. }
            | Self::Ellipse { object, .. }
            | Self::Polyline { object, .. }
            | Self::Line { object, .. }
            | Self::Arc { object, .. } => *object,
        }
    }

    /// A point guaranteed to lie on the entity's plane.
    fn reference_point(&self) -> Point3 {
        match self {
            Self::Circle { center, .. } | Self::Ellipse { center, .. } => *center,
            Self::Polyline { points, .. } => points[0],
            Self::Line { start, .. } => *start,
            Self::Arc { center, .. } => *center,
        }
    }

    /// Every point that has to lie on the hatch plane.
    ///
    /// A circle/ellipse only needs its center (the normal is checked separately);
    /// an arc adds both endpoints; a line both endpoints; a polyline all vertices.
    fn plane_points(&self) -> Vec<Point3> {
        match self {
            Self::Circle { center, .. } | Self::Ellipse { center, .. } => vec![*center],
            Self::Polyline { points, .. } => points.clone(),
            Self::Line { start, end, .. } => vec![*start, *end],
            Self::Arc {
                center,
                start_point,
                end_point,
                ..
            } => vec![*center, *start_point, *end_point],
        }
    }

    /// The plane normal the entity itself declares, if any.
    ///
    /// A circle/ellipse/arc stores an extrusion normal; a line has none and a
    /// polyline has none either (its points imply the plane), so those never
    /// constrain the shared normal.
    fn declared_normal(&self) -> Option<Point3> {
        match self {
            Self::Circle { normal, .. }
            | Self::Ellipse { normal, .. }
            | Self::Arc { normal, .. } => Some(*normal),
            Self::Polyline { .. } | Self::Line { .. } => None,
        }
    }

    /// Whether this shape is a single `LINE`/`ARC` edge that must be stitched.
    fn is_stitch_edge(&self) -> bool {
        matches!(self, Self::Line { .. } | Self::Arc { .. })
    }
}

/// The stitched result: the world-space loops plus the edge → shape mapping.
pub(super) struct StitchedBoundary {
    /// `edge_shape[i]` is the index in `BoundaryShape` order of stitcher edge `i`.
    edge_shape: Vec<usize>,
    loops: Vec<StitchedLoop>,
}

/// Feed every selected `LINE`/`ARC` shape to the stitcher, in selection order.
///
/// The stitcher itself knows nothing about entities or arcs: it chains endpoints
/// and reports which edge index each loop segment consumes, in which direction.
/// `edge_shape` maps those indices back to the classified shapes so the bulge of
/// each segment and its entity id stay available.
pub(super) fn stitch_boundary(shapes: &[BoundaryShape]) -> CadResult<StitchedBoundary> {
    let mut edge_shape = Vec::new();
    let mut edges = Vec::new();
    for (index, shape) in shapes.iter().enumerate() {
        if !shape.is_stitch_edge() {
            continue;
        }
        let (start, end) = match shape {
            BoundaryShape::Line { start, end, .. } => (*start, *end),
            BoundaryShape::Arc {
                start_point,
                end_point,
                ..
            } => (*start_point, *end_point),
            _ => unreachable!("is_stitch_edge only covers Line/Arc"),
        };
        edge_shape.push(index);
        edges.push(StitchEdge { start, end });
    }
    let loops = stitch_edges(&edges)?;
    Ok(StitchedBoundary { edge_shape, loops })
}

impl StitchedBoundary {
    /// How many `LINE`/`ARC` edges were chained (the summary diagnostic echoes it).
    pub(super) fn edge_count(&self) -> usize {
        self.edge_shape.len()
    }

    /// The entity a loop is reported under when a plane refusal must name one:
    /// the first consumed edge's source entity.
    fn object(&self, shapes: &[BoundaryShape], loop_: &StitchedLoop) -> EntityId {
        loop_
            .segments
            .first()
            .map(|segment| shapes[self.edge_shape[segment.edge]].object())
            .unwrap_or(EntityId(0))
    }
}

/// Classify every selected entity, refusing unsupported/non-finite input by name.
///
/// An `ARC` covering a full turn is emitted as **four quarter edges** (with the
/// same sweep sign), for the same reason the circle loop uses four quarters: the
/// creation-path validator measures a chord ring and needs at least three
/// vertices, and an arc whose endpoints coincide would otherwise collapse into one
/// node. Each sub-edge keeps the source entity id, so a refusal still names it.
pub(super) fn boundary_shapes(sources: &[DbEntity]) -> CadResult<Vec<BoundaryShape>> {
    if sources.is_empty() {
        return Err(CadError::InvalidInput(
            "HATCH needs at least one selected boundary entity".into(),
        ));
    }
    let mut shapes = Vec::with_capacity(sources.len());
    for source in sources {
        let object = source.id;
        let produced = match &source.geometry {
            SemanticGeometry::Circle {
                center,
                normal,
                radius,
            } => {
                if !is_finite_point(*center) || !is_finite_point(*normal) {
                    return Err(non_finite_refusal(object, "circle"));
                }
                if !radius.is_finite() || *radius <= 0.0 {
                    return Err(CadError::InvalidInput(format!(
                        "HATCH boundary entity {} (circle) must have a positive finite radius, \
                         got {radius}",
                        object.0
                    )));
                }
                if unit(*normal).is_none() {
                    return Err(CadError::InvalidInput(format!(
                        "HATCH boundary entity {} (circle) has a degenerate normal",
                        object.0
                    )));
                }
                vec![BoundaryShape::Circle {
                    object,
                    center: *center,
                    normal: *normal,
                    radius: *radius,
                }]
            }
            SemanticGeometry::Ellipse {
                center,
                normal,
                major_axis,
                ratio,
                start,
                sweep,
            } => {
                if !is_finite_point(*center)
                    || !is_finite_point(*normal)
                    || !is_finite_point(*major_axis)
                    || !start.is_finite()
                    || !sweep.is_finite()
                    || !ratio.is_finite()
                {
                    return Err(non_finite_refusal(object, "ellipse"));
                }
                if unit(*normal).is_none() || unit(*major_axis).is_none() {
                    return Err(CadError::InvalidInput(format!(
                        "HATCH boundary entity {} (ellipse) has a degenerate normal or major axis",
                        object.0
                    )));
                }
                if sweep.abs() < std::f64::consts::TAU - 1e-9 {
                    return Err(CadError::InvalidInput(format!(
                        "HATCH boundary entity {} (ellipse) is an arc, not a closed loop; only a \
                         full ellipse can be a boundary",
                        object.0
                    )));
                }
                if ratio.abs() <= 0.0 {
                    return Err(CadError::InvalidInput(format!(
                        "HATCH boundary entity {} (ellipse) has a degenerate minor axis",
                        object.0
                    )));
                }
                vec![BoundaryShape::Ellipse {
                    object,
                    center: *center,
                    normal: *normal,
                    major_axis: *major_axis,
                    ratio: *ratio,
                    start: *start,
                    sweep: *sweep,
                }]
            }
            SemanticGeometry::Polyline {
                points,
                bulges,
                closed,
            } => {
                if !*closed {
                    return Err(CadError::InvalidInput(format!(
                        "HATCH boundary entity {} is an open polyline; only a closed polyline can \
                         be a boundary (an open polyline is not stitched — only LINE/ARC edges are)",
                        object.0
                    )));
                }
                if points.len() < 3 {
                    return Err(CadError::InvalidInput(format!(
                        "HATCH boundary entity {} (polyline) has fewer than three vertices",
                        object.0
                    )));
                }
                if !points.iter().all(|point| is_finite_point(*point))
                    || !bulges.iter().all(|bulge| bulge.is_finite())
                {
                    return Err(non_finite_refusal(object, "polyline"));
                }
                if !bulges.is_empty() && bulges.len() != points.len() {
                    return Err(CadError::InvalidInput(format!(
                        "HATCH boundary entity {} (polyline) has {} bulge(s) for {} vertices",
                        object.0,
                        bulges.len(),
                        points.len()
                    )));
                }
                vec![BoundaryShape::Polyline {
                    object,
                    points: points.clone(),
                    bulges: bulges.clone(),
                }]
            }
            SemanticGeometry::Line { start, end } => {
                if !is_finite_point(*start) || !is_finite_point(*end) {
                    return Err(non_finite_refusal(object, "line"));
                }
                if distance(*start, *end) <= STITCH_TOLERANCE {
                    return Err(degenerate_edge_refusal(
                        object,
                        "line",
                        &format!(
                            "zero length — shorter than the stitching tolerance \
                             {STITCH_TOLERANCE:e} world units — so it has no direction to chain"
                        ),
                    ));
                }
                vec![BoundaryShape::Line {
                    object,
                    start: *start,
                    end: *end,
                }]
            }
            SemanticGeometry::Arc {
                center,
                normal,
                radius,
                start,
                sweep,
            } => {
                if !is_finite_point(*center) || !is_finite_point(*normal) {
                    return Err(non_finite_refusal(object, "arc"));
                }
                if !start.is_finite() || !sweep.is_finite() {
                    return Err(non_finite_refusal(object, "arc"));
                }
                if !radius.is_finite() || *radius <= 0.0 {
                    return Err(CadError::InvalidInput(format!(
                        "HATCH boundary entity {} (arc) must have a positive finite radius, got \
                         {radius}",
                        object.0
                    )));
                }
                if unit(*normal).is_none() {
                    return Err(CadError::InvalidInput(format!(
                        "HATCH boundary entity {} (arc) has a degenerate normal",
                        object.0
                    )));
                }
                if sweep.abs() <= DEGENERATE_SWEEP {
                    return Err(degenerate_edge_refusal(
                        object,
                        "arc",
                        &format!(
                            "zero sweep — got {sweep} rad, at or below the degenerate threshold \
                             {DEGENERATE_SWEEP:e} rad — so there is no arc to chain"
                        ),
                    ));
                }
                // The endpoints use the tessellator's arbitrary-axis frame, so the
                // stitched vertices sit exactly where the arc is drawn.
                let (axis_x, axis_y, _) = cad_geometry::arbitrary_axis(*normal);
                let point_at = |angle: f64| {
                    add(
                        *center,
                        add(
                            scale(axis_x, *radius * angle.cos()),
                            scale(axis_y, *radius * angle.sin()),
                        ),
                    )
                };
                // A full turn — or any arc whose own endpoints are within the
                // stitching tolerance, i.e. a loop by the same endpoint rule — is
                // split into four quarter edges (see the doc above).
                let closes = sweep.abs() >= std::f64::consts::TAU - 1e-9
                    || distance(point_at(*start), point_at(*start + *sweep)) <= STITCH_TOLERANCE;
                let pieces = if closes { 4 } else { 1 };
                let step = *sweep / pieces as f64;
                (0..pieces)
                    .map(|index| {
                        let from = *start + step * index as f64;
                        let to = from + step;
                        BoundaryShape::Arc {
                            object,
                            center: *center,
                            normal: *normal,
                            sweep: step,
                            start_point: point_at(from),
                            end_point: point_at(to),
                        }
                    })
                    .collect()
            }
            other => {
                return Err(CadError::InvalidInput(format!(
                    "HATCH cannot use {} (entity {}) as a boundary; accepted boundaries are a \
                     circle, a full ellipse, a closed polyline, and LINE/ARC edges stitched into \
                     closed loops",
                    geometry_kind(other),
                    object.0
                )));
            }
        };
        shapes.extend(produced);
    }
    Ok(shapes)
}

/// A `LINE`/`ARC` edge that cannot take part in stitching is refused by name
/// **before** any node clustering happens.
fn degenerate_edge_refusal(object: EntityId, kind: &str, detail: &str) -> CadError {
    CadError::InvalidInput(format!(
        "HATCH boundary entity {} ({kind}) is degenerate: hatch.boundary_degenerate ({detail})",
        object.0
    ))
}

fn non_finite_refusal(object: EntityId, kind: &str) -> CadError {
    CadError::InvalidInput(format!(
        "HATCH boundary entity {} ({kind}) has a non-finite coordinate",
        object.0
    ))
}

/// Resolve the single hatch plane from the classified shapes.
///
/// A circle/ellipse/arc declares its normal; every declared normal must be the
/// **same** direction (not merely parallel with the opposite sign), because the
/// ADR requires one consistent normal. When nothing declares a normal the plane
/// comes from the first closed ring in the selection: a polyline's points, or
/// (when the selection is only `LINE`/`ARC` edges) the first stitched loop —
/// whose vertices are the chain's node positions, so the Newell normal is the
/// plane the edges actually share. A polyline's/loop's winding may be either way
/// round; only coplanarity is required. All centers/vertices/arc endpoints must
/// then sit on one plane within [`plane_tolerance`].
pub(super) fn resolve_plane(
    shapes: &[BoundaryShape],
    stitched: &StitchedBoundary,
) -> CadResult<HatchPlane> {
    let Some(first) = shapes.first() else {
        return Err(CadError::InvalidInput(
            "HATCH needs at least one selected boundary entity".into(),
        ));
    };
    let mut magnitude = 0.0f64;
    for point in shapes.iter().flat_map(BoundaryShape::plane_points).chain(
        stitched
            .loops
            .iter()
            .flat_map(|loop_| loop_.points.iter().copied()),
    ) {
        magnitude = magnitude
            .max(point.x.abs())
            .max(point.y.abs())
            .max(point.z.abs());
    }
    let tolerance = plane_tolerance(magnitude);

    let mut normal: Option<Point3> = None;
    for shape in shapes {
        let Some(declared) = shape.declared_normal() else {
            continue;
        };
        let Some(unit) = unit(declared) else {
            return Err(CadError::InvalidInput(format!(
                "HATCH boundary entity {} has a degenerate plane normal",
                shape.object().0
            )));
        };
        match normal {
            None => normal = Some(unit),
            Some(existing) => {
                if dot(existing, unit) < 1.0 - 1e-9 {
                    return Err(CadError::InvalidInput(format!(
                        "HATCH boundaries must share a single consistent normal; entity {} \
                         disagrees",
                        shape.object().0
                    )));
                }
            }
        }
    }
    let normal = match normal {
        Some(normal) => normal,
        None => {
            let mut found = None;
            for shape in shapes {
                if let BoundaryShape::Polyline { points, object, .. } = shape {
                    match newell_normal(points) {
                        Some(candidate) => {
                            found = Some(candidate);
                            break;
                        }
                        None => {
                            return Err(CadError::InvalidInput(format!(
                                "HATCH boundary entity {} (polyline) is degenerate: its points \
                                 define no plane",
                                object.0
                            )));
                        }
                    }
                }
            }
            if found.is_none() {
                // Every stitched loop lies on the same plane, so the first one
                // decides — exactly like the first polyline above.
                if let Some(loop_) = stitched.loops.first() {
                    match newell_normal(&loop_.points) {
                        Some(candidate) => found = Some(candidate),
                        None => {
                            return Err(CadError::InvalidInput(format!(
                                "HATCH stitched boundary loop is degenerate: \
                                 hatch.boundary_degenerate (its {} edge(s) enclose no area, so they \
                                 define no boundary plane)",
                                loop_.segments.len()
                            )));
                        }
                    }
                }
            }
            found.ok_or_else(|| {
                CadError::InvalidInput(
                    "HATCH cannot determine a boundary plane from the selection".into(),
                )
            })?
        }
    };

    let elevation = dot(first.reference_point(), normal);
    if !elevation.is_finite() {
        return Err(CadError::InvalidInput(
            "HATCH boundary plane elevation must be finite".into(),
        ));
    }
    let on_plane = |point: Point3| (dot(point, normal) - elevation).abs() <= tolerance;
    for shape in shapes {
        if !shape.plane_points().into_iter().all(on_plane) {
            return Err(coplanar_refusal(shape.object()));
        }
    }
    for loop_ in &stitched.loops {
        if !loop_.points.iter().all(|point| on_plane(*point)) {
            return Err(coplanar_refusal(stitched.object(shapes, loop_)));
        }
    }
    Ok(HatchPlane::new(normal, elevation))
}

fn coplanar_refusal(object: EntityId) -> CadError {
    CadError::InvalidInput(format!(
        "HATCH boundaries must be coplanar with a single consistent normal; entity {} is off the \
         boundary plane",
        object.0
    ))
}

/// The Newell normal of a 3D polygon, or `None` when it encloses no area.
///
/// The Newell vector has units of area, so the degeneracy test is relative to the
/// point extent squared (an absolute epsilon would reject a tiny but valid loop).
fn newell_normal(points: &[Point3]) -> Option<Point3> {
    if points.len() < 3 {
        return None;
    }
    let mut normal = Point3 {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };
    for index in 0..points.len() {
        let a = points[index];
        let b = points[(index + 1) % points.len()];
        normal.x += (a.y - b.y) * (a.z + b.z);
        normal.y += (a.z - b.z) * (a.x + b.x);
        normal.z += (a.x - b.x) * (a.y + b.y);
    }
    let mut min = points[0];
    let mut max = points[0];
    for point in points {
        min.x = min.x.min(point.x);
        min.y = min.y.min(point.y);
        min.z = min.z.min(point.z);
        max.x = max.x.max(point.x);
        max.y = max.y.max(point.y);
        max.z = max.z.max(point.z);
    }
    let span = (max.x - min.x).hypot(max.y - min.y).hypot(max.z - min.z);
    let length = normal.x.hypot(normal.y).hypot(normal.z);
    if !length.is_finite() || !span.is_finite() || length <= 1e-12 * (span * span).max(1e-24) {
        return None;
    }
    // A polyline in a plane parallel to world XY is 2D data: its bulges follow
    // the DXF OCS convention with a `+Z` normal, not the vertex winding (the
    // display and the fill sampler must agree for either winding).
    if cad_geometry::is_xy_parallel(points) {
        return Some(Point3 {
            x: 0.0,
            y: 0.0,
            z: 1.0,
        });
    }
    unit(normal)
}

/// Build the OCS-2D loops for the whole selection.
///
/// Closed shapes keep their one-loop-per-entity order; the stitched `LINE`/`ARC`
/// loops are emitted as a block at the position of the first edge in the
/// selection, in chain-discovery order. The `HatchGeometry::new` validator only
/// cares about the loop set, not its order, but a stable order keeps the
/// diagnostics and the tests deterministic.
pub(super) fn boundary_loops(
    shapes: &[BoundaryShape],
    stitched: &StitchedBoundary,
    plane: &HatchPlane,
) -> CadResult<Vec<HatchLoop>> {
    let mut loops = Vec::new();
    let mut stitched_pushed = false;
    for shape in shapes {
        if shape.is_stitch_edge() {
            if !stitched_pushed {
                for loop_ in &stitched.loops {
                    loops.push(stitched_loop(loop_, shapes, stitched, plane)?);
                }
                stitched_pushed = true;
            }
            continue;
        }
        loops.push(boundary_loop(shape, plane)?);
    }
    Ok(loops)
}

/// Convert one stitched edge loop into a closed bulged `HatchLoop`.
///
/// Every segment keeps its edge **exact**: a `LINE` contributes bulge 0 and an
/// `ARC` contributes `tan(sweep / 4)` for the sweep it is traversed with — the
/// stored sweep when the segment runs `start → end`, the negated one when the
/// stitcher had to consume the edge backwards. That is exactly the convention the
/// circle loop uses (a quarter arc is `tan(pi/8)`), so a stitched arc is the same
/// geometry a source bulge loop would carry; nothing is sampled.
fn stitched_loop(
    loop_: &StitchedLoop,
    shapes: &[BoundaryShape],
    stitched: &StitchedBoundary,
    plane: &HatchPlane,
) -> CadResult<HatchLoop> {
    let to_2d = plane_projection(plane);
    let mut points = Vec::with_capacity(loop_.segments.len());
    let mut bulges = Vec::with_capacity(loop_.segments.len());
    for (index, segment) in loop_.segments.iter().enumerate() {
        points.push(to_2d(loop_.points[index]));
        let shape = &shapes[stitched.edge_shape[segment.edge]];
        bulges.push(segment_bulge(shape, segment.reversed, plane)?);
    }
    Ok(HatchLoop::new(
        points,
        bulges,
        HatchLoopFlags(HatchLoopFlags::POLYLINE),
    ))
}

/// The bulge one stitched segment contributes.
fn segment_bulge(shape: &BoundaryShape, reversed: bool, plane: &HatchPlane) -> CadResult<f64> {
    match shape {
        BoundaryShape::Line { .. } => Ok(0.0),
        BoundaryShape::Arc {
            object,
            normal,
            sweep,
            ..
        } => {
            let Some(normal) = unit(*normal) else {
                return Err(CadError::InvalidInput(format!(
                    "HATCH boundary entity {} (arc) has a degenerate normal",
                    object.0
                )));
            };
            if dot(normal, plane.normal) <= 0.0 {
                return Err(CadError::InvalidInput(format!(
                    "HATCH boundary entity {} (arc) sweeps against the boundary plane normal; a \
                     mirrored arc cannot be stitched into this hatch plane",
                    object.0
                )));
            }
            let sweep = if reversed { -*sweep } else { *sweep };
            Ok((sweep / 4.0).tan())
        }
        _ => Err(CadError::InvalidInput(
            "HATCH internal: a stitched boundary segment is not a LINE or an ARC".into(),
        )),
    }
}

/// The hatch plane's own OCS 2D projection.
///
/// Recovered through `HatchPlane::to_world`, so the loop frame can never disagree
/// with the plane the fill and the display use.
fn plane_projection(plane: &HatchPlane) -> impl Fn(Point3) -> [f64; 2] {
    let origin = plane.to_world([0.0, 0.0]);
    let axis_x = sub(plane.to_world([1.0, 0.0]), origin);
    let axis_y = sub(plane.to_world([0.0, 1.0]), origin);
    move |point: Point3| {
        let relative = sub(point, origin);
        [dot(relative, axis_x), dot(relative, axis_y)]
    }
}

/// Build one OCS-2D loop from a classified **closed** boundary entity.
fn boundary_loop(shape: &BoundaryShape, plane: &HatchPlane) -> CadResult<HatchLoop> {
    let to_2d = plane_projection(plane);
    let flags = HatchLoopFlags(HatchLoopFlags::POLYLINE);
    match shape {
        BoundaryShape::Circle { center, radius, .. } => {
            // Four quarter arcs: every vertex sits on the circle and every bulge
            // is `tan(pi/8)`, so the boundary is exactly the circle.
            let center = to_2d(*center);
            let bulge = std::f64::consts::FRAC_PI_8.tan();
            let points = (0..4)
                .map(|index| {
                    let angle = index as f64 * std::f64::consts::FRAC_PI_2;
                    let (sin, cos) = angle.sin_cos();
                    [center[0] + radius * cos, center[1] + radius * sin]
                })
                .collect();
            Ok(HatchLoop::new(points, vec![bulge; 4], flags))
        }
        BoundaryShape::Ellipse {
            center,
            normal,
            major_axis,
            ratio,
            start,
            sweep,
            ..
        } => {
            let Some(major_unit) = unit(*major_axis) else {
                return Err(CadError::InvalidInput(
                    "HATCH ellipse boundary has a degenerate major axis".into(),
                ));
            };
            let Some(normal_unit) = unit(*normal) else {
                return Err(CadError::InvalidInput(
                    "HATCH ellipse boundary has a degenerate normal".into(),
                ));
            };
            let major_length = length(*major_axis);
            // `minor = cross(normal, major_unit) * major_length * |ratio|`, exactly
            // the renderer's ellipse frame.
            let minor = scale(cross(normal_unit, major_unit), major_length * ratio.abs());
            let points = (0..ELLIPSE_SEGMENTS)
                .map(|index| {
                    let t = start + sweep * index as f64 / ELLIPSE_SEGMENTS as f64;
                    let (sin, cos) = t.sin_cos();
                    to_2d(add(
                        *center,
                        add(scale(*major_axis, cos), scale(minor, sin)),
                    ))
                })
                .collect();
            Ok(HatchLoop::new(points, vec![0.0; ELLIPSE_SEGMENTS], flags))
        }
        BoundaryShape::Polyline { points, bulges, .. } => {
            let points = points.iter().map(|point| to_2d(*point)).collect::<Vec<_>>();
            let bulges = if bulges.is_empty() {
                vec![0.0; points.len()]
            } else {
                bulges.clone()
            };
            Ok(HatchLoop::new(points, bulges, flags))
        }
        BoundaryShape::Line { .. } | BoundaryShape::Arc { .. } => Err(CadError::InvalidInput(
            "HATCH internal: a LINE/ARC edge must be stitched, not converted on its own".into(),
        )),
    }
}

// ---------------------------------------------------------------------------
// Small vector algebra (the domain's helpers are private; `cad-app` only needs
// the few operations below).
// ---------------------------------------------------------------------------

fn add(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.x + b.x,
        y: a.y + b.y,
        z: a.z + b.z,
    }
}

fn sub(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.x - b.x,
        y: a.y - b.y,
        z: a.z - b.z,
    }
}

fn scale(v: Point3, factor: f64) -> Point3 {
    Point3 {
        x: v.x * factor,
        y: v.y * factor,
        z: v.z * factor,
    }
}

fn dot(a: Point3, b: Point3) -> f64 {
    a.x * b.x + a.y * b.y + a.z * b.z
}

fn cross(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.y * b.z - a.z * b.y,
        y: a.z * b.x - a.x * b.z,
        z: a.x * b.y - a.y * b.x,
    }
}

fn length(v: Point3) -> f64 {
    v.x.hypot(v.y).hypot(v.z)
}

/// The unit vector, or `None` for a zero/non-finite vector.
fn unit(v: Point3) -> Option<Point3> {
    let length = length(v);
    if !length.is_finite() || length <= 1e-12 {
        None
    } else {
        Some(scale(v, 1.0 / length))
    }
}
