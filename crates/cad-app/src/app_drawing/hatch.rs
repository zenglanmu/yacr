//! HATCH creation from the **pre-selection** (ADR 0018 decision D, P6b).
//!
//! The boundary is the selection snapshot: `DrawToolKind::Hatch` captures no
//! points (the zero-point exact complement, like ARRAYRECT) and takes no
//! in-command entity picks, so the refs arrive in `CommandPayload::Hatch`. Each
//! accepted selected entity becomes **one loop** in the hatch plane's OCS 2D
//! frame:
//!
//! * `Circle` → an **exact** arc loop: four quadrant vertices with a
//!   `tan(pi/8)` bulge on every segment, i.e. four quarter-circle arcs. The
//!   ADR's cheap win stores a circle as two bulge-1 segments; this build uses
//!   four because the creation-path validator (`HatchGeometry::new`) requires at
//!   least three vertices and measures a chord ring, so a two-vertex loop is a
//!   degenerate chord set there. Four quarter arcs are still an exact circle and
//!   still export as an exact bulged polyline boundary.
//! * full `Ellipse` → a **sampled** chord loop (`ELLIPSE_SEGMENTS` chords per
//!   turn). A true ellipse is not a circular arc, so a bulge loop would be a
//!   lie; this loss is declared here and in the command docs.
//! * closed `Polyline` → its own points and bulges, mapped into the hatch plane.
//!
//! Every selected entity must lie in one plane with a single consistent normal
//! (circles/ellipses declare their normal; a polyline's is implied by its
//! points); anything else is refused **by name** before any write. The geometry
//! goes through `HatchGeometry::new`, so the ADR A.3 creation-path rejections
//! (self-intersecting, tangent/shared vertex, duplicate, partially overlapping,
//! unclosed, degenerate, non-finite) fail the whole command with no transaction.
//! **Declared limitation**: that validator works on the loop's **chords**, so for
//! a bulged loop (our circle loop included) a tangency that falls between the
//! sampled chords of two loops is not detected. An exact-circle contact whose
//! inscribed squares touch *is* refused (pinned by the tests); the residual gap
//! is a chord-sampling blind spot, not a silent acceptance of a crossing.
//!
//! The selected boundary entities are **retained**: the hatch is a derived
//! annotation over them (no associativity, no `DELOBJ` gating).
//!
//! The typed parameters are read from the session (the POLYGON precedent): the
//! fill kind, spacing, angle (degrees on the channel, radians in the domain), the
//! `HPDOUBLE` flag and the style. A user-defined pattern is synthesized from
//! angle/spacing/double by the same one-row definition the import path generates
//! for a `76=0`/`78=0` hatch, so creation and import share one recipe; the
//! generated line geometry remains **our implementation** (ADR U-1), not a
//! documented AutoCAD formula.

use super::*;

use crate::draw_tool::{
    ToolParameter, HATCH_PATTERN_TYPE_SOLID, HATCH_PATTERN_TYPE_USER_DEFINED, HATCH_STYLE_IGNORE,
    HATCH_STYLE_NORMAL, HATCH_STYLE_OUTER,
};

/// Number of chords a full selected ellipse is sampled into.
///
/// A sampled ellipse boundary is the declared approximation (a true ellipse is
/// not a circular arc); 64 chords keep the visible boundary within about 0.1% of
/// the curve radius while staying small in the DXF file.
const ELLIPSE_SEGMENTS: usize = 64;

/// The pattern name written for a created user-defined pattern.
///
/// There is no pattern table and no `.pat` parser, so this is an opaque,
/// locale-neutral identity (ADR U-11: what AutoCAD prefers when a `76=0` hatch
/// carries both a name and definition rows is unverified). The definition rows
/// are the authoritative data.
const USER_DEFINED_PATTERN_NAME: &str = "USER";

/// The relative-plane tolerance: `1e-9` scaled by the boundary magnitude plus
/// one, so large drawings are not rejected for floating-point noise and exact
/// coincidences are still exact.
fn plane_tolerance(magnitude: f64) -> f64 {
    1e-9 * (1.0 + magnitude)
}

/// One accepted boundary entity, classified from its stored geometry.
enum BoundaryShape {
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
}

impl BoundaryShape {
    fn object(&self) -> EntityId {
        match self {
            Self::Circle { object, .. }
            | Self::Ellipse { object, .. }
            | Self::Polyline { object, .. } => *object,
        }
    }

    /// A point guaranteed to lie on the entity's plane.
    fn reference_point(&self) -> Point3 {
        match self {
            Self::Circle { center, .. } | Self::Ellipse { center, .. } => *center,
            Self::Polyline { points, .. } => points[0],
        }
    }

    /// The plane normal the entity itself declares, if any.
    ///
    /// A circle/ellipse stores an extrusion normal; a polyline has none (its
    /// points imply the plane), so it never constrains the shared normal.
    fn declared_normal(&self) -> Option<Point3> {
        match self {
            Self::Circle { normal, .. } | Self::Ellipse { normal, .. } => Some(*normal),
            Self::Polyline { .. } => None,
        }
    }
}

/// The HATCH parameters read from the session typed channel.
struct HatchParameters {
    pattern_type: f64,
    spacing: f64,
    /// Degrees on the channel (the AutoCAD prompt unit); the domain stores radians.
    angle_degrees: f64,
    double: bool,
    style: f64,
}

impl Application {
    /// HATCH the pre-selected boundary entities into one entity (ADR 0018 D).
    ///
    /// One transaction / one undo step through the shared create funnel, on the
    /// active layer, with `type_key`/`class_name` = `AcDbHatch`. The selected
    /// boundary entities are **retained**: the hatch is a derived annotation over
    /// them, exactly like an AutoCAD "select objects" hatch (no associativity).
    ///
    /// Refusals (nothing written, no transaction):
    ///
    /// * an empty selection;
    /// * a selection entry referenced through a block instance or a sub-element
    ///   (the boundary must be a model-space entity);
    /// * any kind other than `Circle`, a full `Ellipse` or a closed `Polyline`
    ///   (`Line`/`Arc`/open polyline/spline/text/block/… — no boundary stitcher);
    /// * an ellipse that is not a full turn (an ellipse arc is not a closed loop);
    /// * a selection that is not coplanar with a single consistent normal;
    /// * every `HatchGeometry::new` creation-path rejection (self-intersecting,
    ///   tangent/shared vertex, duplicate, partially overlapping, unclosed,
    ///   degenerate, non-finite);
    /// * a missing active layer, an out-of-range typed parameter, and Viewer mode
    ///   (the command layer's mode check).
    pub(crate) fn create_hatch(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::Hatch { refs } = &command.payload else {
            return Err(CadError::InvalidInput(
                "Hatch needs a Hatch { refs } payload".into(),
            ));
        };
        let parameters = hatch_parameters(session)?;

        let document_id = command.document;
        let (loops, plane) = {
            let document = self
                .workspace
                .documents
                .get(&document_id)
                .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
            let drawing = document.drawing.as_ref();

            for reference in refs {
                if !reference.instance.0.is_empty() || reference.sub_element.is_some() {
                    return Err(CadError::InvalidInput(format!(
                        "HATCH boundary must be a model-space entity; entity {} is referenced \
                         through a block instance or a sub-element",
                        reference.entity.0
                    )));
                }
            }

            let (sources, _diagnostics) = Self::resolve_selection(
                drawing,
                document_id,
                refs,
                "hatch",
                "HATCH",
                "hatch",
                false,
            )?;
            let shapes = boundary_shapes(&sources)?;
            let plane = resolve_plane(&shapes)?;
            let loops = shapes
                .iter()
                .map(|shape| boundary_loop(shape, &plane))
                .collect::<CadResult<Vec<HatchLoop>>>()?;
            (loops, plane)
        };
        let loop_count = loops.len();
        let style = hatch_style(parameters.style)?;
        let (fill, pattern_type) = parameters.fill()?;

        // The unique creation-path constructor: every boundary defect (A.3) is a
        // whole-command refusal here, before a transaction exists.
        let hatch = HatchGeometry::new(loops, plane, style, fill, Vec::new(), None, pattern_type)
            .map_err(hatch_creation_refusal)?;
        let geometry = SemanticGeometry::Hatch(Box::new(hatch));

        let mut outcome = self.commit_new_entity(session, command, geometry, "create hatch")?;
        if let Some(object) = outcome.objects.first().copied() {
            outcome
                .diagnostics
                .push(hatch_diagnostic(object, loop_count, style, &parameters));
        }
        Ok(outcome)
    }
}

/// Classify every selected entity, refusing unsupported/non-finite input by name.
fn boundary_shapes(sources: &[DbEntity]) -> CadResult<Vec<BoundaryShape>> {
    if sources.is_empty() {
        return Err(CadError::InvalidInput(
            "HATCH needs at least one selected boundary entity".into(),
        ));
    }
    let mut shapes = Vec::with_capacity(sources.len());
    for source in sources {
        let object = source.id;
        let shape = match &source.geometry {
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
                BoundaryShape::Circle {
                    object,
                    center: *center,
                    normal: *normal,
                    radius: *radius,
                }
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
                BoundaryShape::Ellipse {
                    object,
                    center: *center,
                    normal: *normal,
                    major_axis: *major_axis,
                    ratio: *ratio,
                    start: *start,
                    sweep: *sweep,
                }
            }
            SemanticGeometry::Polyline {
                points,
                bulges,
                closed,
            } => {
                if !*closed {
                    return Err(CadError::InvalidInput(format!(
                        "HATCH boundary entity {} is an open polyline; only a closed polyline can \
                         be a boundary (LINE/ARC boundary stitching is NOT IMPLEMENTED)",
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
                BoundaryShape::Polyline {
                    object,
                    points: points.clone(),
                    bulges: bulges.clone(),
                }
            }
            other => {
                return Err(CadError::InvalidInput(format!(
                    "HATCH cannot use {} (entity {}) as a boundary; accepted boundaries are a \
                     circle, a full ellipse and a closed polyline",
                    geometry_kind(other),
                    object.0
                )));
            }
        };
        shapes.push(shape);
    }
    Ok(shapes)
}

fn non_finite_refusal(object: EntityId, kind: &str) -> CadError {
    CadError::InvalidInput(format!(
        "HATCH boundary entity {} ({kind}) has a non-finite coordinate",
        object.0
    ))
}

/// Resolve the single hatch plane from the classified shapes.
///
/// A circle/ellipse declares its normal; every declared normal must be the
/// **same** direction (not merely parallel with the opposite sign), because the
/// ADR requires one consistent normal. When every shape is a polyline the normal
/// is the first non-degenerate Newell normal, and only coplanarity is required
/// (a polyline's winding may be either way round). All centers/vertices must then
/// sit on one plane within [`plane_tolerance`].
fn resolve_plane(shapes: &[BoundaryShape]) -> CadResult<HatchPlane> {
    let Some(first) = shapes.first() else {
        return Err(CadError::InvalidInput(
            "HATCH needs at least one selected boundary entity".into(),
        ));
    };
    let magnitude = shapes.iter().fold(0.0f64, |magnitude, shape| {
        let points: &[Point3] = match shape {
            BoundaryShape::Circle { center, .. } | BoundaryShape::Ellipse { center, .. } => {
                std::slice::from_ref(center)
            }
            BoundaryShape::Polyline { points, .. } => points,
        };
        points.iter().fold(magnitude, |magnitude, point| {
            magnitude
                .max(point.x.abs())
                .max(point.y.abs())
                .max(point.z.abs())
        })
    });
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
    for shape in shapes {
        let on_plane = |point: Point3| (dot(point, normal) - elevation).abs() <= tolerance;
        match shape {
            BoundaryShape::Circle { center, object, .. }
            | BoundaryShape::Ellipse { center, object, .. } => {
                if !on_plane(*center) {
                    return Err(coplanar_refusal(*object));
                }
            }
            BoundaryShape::Polyline { points, object, .. } => {
                if !points.iter().all(|point| on_plane(*point)) {
                    return Err(coplanar_refusal(*object));
                }
            }
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

/// Build one OCS-2D loop from a classified boundary entity.
fn boundary_loop(shape: &BoundaryShape, plane: &HatchPlane) -> CadResult<HatchLoop> {
    // The plane's own OCS basis, recovered through its public mapping so the loop
    // frame can never disagree with `HatchPlane::to_world`.
    let origin = plane.to_world([0.0, 0.0]);
    let axis_x = sub(plane.to_world([1.0, 0.0]), origin);
    let axis_y = sub(plane.to_world([0.0, 1.0]), origin);
    let to_2d = |point: Point3| {
        let relative = sub(point, origin);
        [dot(relative, axis_x), dot(relative, axis_y)]
    };
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
    }
}

/// Read and defensively re-validate the HATCH typed parameters.
fn hatch_parameters(session: &SessionState) -> CadResult<HatchParameters> {
    let parameters = [
        ToolParameter::HatchPatternType,
        ToolParameter::HatchSpacing,
        ToolParameter::HatchAngle,
        ToolParameter::HatchDouble,
        ToolParameter::HatchStyle,
    ];
    for parameter in parameters {
        parameter
            .validate(session.tool_parameter(parameter))
            .map_err(|error| match error {
                CadError::InvalidInput(message) => {
                    CadError::InvalidInput(format!("HATCH: {message}"))
                }
                other => other,
            })?;
    }
    let number = |parameter: ToolParameter| {
        session
            .tool_parameter(parameter)
            .number()
            .unwrap_or(f64::NAN)
    };
    Ok(HatchParameters {
        pattern_type: number(ToolParameter::HatchPatternType),
        spacing: number(ToolParameter::HatchSpacing),
        angle_degrees: number(ToolParameter::HatchAngle),
        double: session
            .tool_parameter(ToolParameter::HatchDouble)
            .flag()
            .unwrap_or(false),
        style: number(ToolParameter::HatchStyle),
    })
}

impl HatchParameters {
    /// The fill definition and the DXF pattern type it must record.
    ///
    /// A solid fill records `Predefined` (`76=1`), matching the committed SOLID
    /// fixtures (a solid pattern has no user-defined definition rows). A
    /// user-defined fill records `UserDefined` (`76=0`) and carries the
    /// synthesized definition rows.
    fn fill(&self) -> CadResult<(HatchFill, HatchPatternType)> {
        if self.pattern_type == HATCH_PATTERN_TYPE_SOLID {
            return Ok((HatchFill::Solid, HatchPatternType::Predefined));
        }
        if self.pattern_type != HATCH_PATTERN_TYPE_USER_DEFINED {
            return Err(CadError::InvalidInput(format!(
                "HATCH pattern type must be {} (solid) or {} (user-defined), got {}",
                HATCH_PATTERN_TYPE_SOLID, HATCH_PATTERN_TYPE_USER_DEFINED, self.pattern_type
            )));
        }
        if !self.spacing.is_finite() || self.spacing <= 0.0 {
            return Err(CadError::InvalidInput(format!(
                "HATCH spacing must be finite and > 0, got {}",
                self.spacing
            )));
        }
        if !self.angle_degrees.is_finite() {
            return Err(CadError::InvalidInput(format!(
                "HATCH pattern angle must be finite, got {}",
                self.angle_degrees
            )));
        }
        let pattern = HatchPattern {
            name: USER_DEFINED_PATTERN_NAME.to_string(),
            angle: self.angle_degrees.to_radians(),
            scale: self.spacing,
            double: self.double,
            lines: vec![synthesized_user_defined_line()],
        };
        Ok((HatchFill::Pattern(pattern), HatchPatternType::UserDefined))
    }
}

/// One pattern-definition row at offset `(0, 1)` in pattern space.
///
/// This mirrors `cad_import_acadrust::hatch::import_pattern`'s `76=0`/`78=0`
/// synthesis exactly: the hatch-level scale is then the line spacing, the
/// hatch-level angle rotates the family and the double flag adds the
/// perpendicular family in [`HatchPattern::families`]. The generated geometry is
/// **our implementation** (ADR U-1), not a documented AutoCAD formula.
fn synthesized_user_defined_line() -> HatchPatternLine {
    HatchPatternLine {
        angle: 0.0,
        base_x: 0.0,
        base_y: 0.0,
        offset_x: 0.0,
        offset_y: 1.0,
        dashes: Vec::new(),
    }
}

/// Map the closed-set style code to the domain style.
fn hatch_style(code: f64) -> CadResult<HatchStyle> {
    if code == HATCH_STYLE_NORMAL {
        Ok(HatchStyle::Normal)
    } else if code == HATCH_STYLE_OUTER {
        Ok(HatchStyle::Outer)
    } else if code == HATCH_STYLE_IGNORE {
        Ok(HatchStyle::Ignore)
    } else {
        Err(CadError::InvalidInput(format!(
            "HATCH style must be {HATCH_STYLE_NORMAL} (Normal), {HATCH_STYLE_OUTER} (Outer) or \
             {HATCH_STYLE_IGNORE} (Ignore), got {code}"
        )))
    }
}

/// Relabel a creation-path rejection with the command context (the reason itself
/// is the domain's, e.g. "self-intersecting").
fn hatch_creation_refusal(error: CadError) -> CadError {
    match error {
        CadError::InvalidInput(reason) => {
            CadError::InvalidInput(format!("HATCH boundary rejected: {reason}"))
        }
        other => other,
    }
}

/// The explicit summary diagnostic (locale-neutral, like the ARRAY commands).
fn hatch_diagnostic(
    object: ObjectId,
    loop_count: usize,
    style: HatchStyle,
    parameters: &HatchParameters,
) -> Diagnostic {
    let fill = if parameters.pattern_type == HATCH_PATTERN_TYPE_SOLID {
        "solid".to_string()
    } else {
        format!(
            "user-defined pattern (spacing {}, angle {} deg, double {})",
            parameters.spacing, parameters.angle_degrees, parameters.double
        )
    };
    Diagnostic {
        object: Some(object),
        code: "drawing.hatch".into(),
        message: format!(
            "HATCH: one entity with {loop_count} boundary loop(s) inserted (style {style:?}, \
             {fill}; associative hatch NOT IMPLEMENTED)"
        ),
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
