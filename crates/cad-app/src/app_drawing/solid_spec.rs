//! The typed solid-construction payload → [`PrimitiveSpec`] mapping.
//!
//! Split out of [`super::solids`] so that file stays inside the code-size gate.
//! Everything here is the **input** half of a primitive command: arity, finiteness,
//! extent and option-legality rules. Every refusal happens here, before the
//! modeler is called, so a bad payload is an input error rather than a kernel
//! answer, and nothing is ever written for it.
//!
//! # The two coordinate forms
//!
//! * [`SolidPrimitiveCommand::Points`] — the explicit 3D form (the pre-existing
//!   contract): every point keeps its own z, so the session `ELEV` token never
//!   translates it. A caller that wants a body at a specific z says so.
//! * Every other variant — the AutoCAD option constructions. Their base points
//!   are **plan** coordinates (`[x, y]`) and the base plane's z is the session
//!   `ELEV` token, exactly as a plan pick takes its z from the current elevation
//!   in AutoCAD. This is the one consumer of `ELEV` for explicit 3D solids; the
//!   create funnel's `Body3d` exemption (see `super::elevation`) means the z is
//!   applied **once**, here, and never translated again. The two forms whose
//!   AutoCAD prompt is explicitly 3D — `Center` with a corner and `SPHERE 2P` —
//!   carry `Point3` and keep their own z instead.
//!
//! # Declared limits
//!
//! * Every scalar extent is strictly positive. AutoCAD's scalar prompts also
//!   accept a negative length/height to build in the opposite direction; that
//!   direction is expressible through the explicit 3D [`SolidPrimitiveCommand::Points`]
//!   form (which normalises opposite corners), and the option form refuses a
//!   non-positive extent by name (`<kind>.degenerate_extent`) instead of
//!   guessing a sign.
//! * All seven families are +Z primitives: the base lies on the work plane and
//!   the axis is world +Z. The AutoCAD `Axis endpoint` height option (an axis
//!   in any 3D direction) and the `3P` / `Ttr` / `Elliptical` base options are
//!   **not** modelled, because the modeler seam has no oriented-axis or
//!   elliptical primitive. `PYRAMID`'s `Edge` / `Sides` / `Top radius` need a
//!   regular n-gon (possibly truncated) base, which the seam's square,
//!   apex-centred `PrimitiveSpec::Pyramid` cannot express — those constructions
//!   are refused as `solid.option_not_supported` and PYRAMID stays partial.

use super::*;
use crate::command_payload::{PlanCircle, PlanLength, SolidPrimitiveCommand};
use cad_kernel_adapter::{PrimitiveKind, PrimitiveSpec};

/// Map one typed construction onto the modeler seam.
///
/// `elevation` is the session `ELEV` token
/// ([`crate::SessionState::drafting_extras`] → `DraftingExtras::elev`); it is read
/// only by the plan forms (see the module docs).
pub(crate) fn solid_primitive_spec(
    kind: PrimitiveKind,
    input: &SolidPrimitiveCommand,
    elevation: f64,
) -> CadResult<PrimitiveSpec> {
    match kind {
        PrimitiveKind::Box => {
            let (origin, length, width, height) = basal_extents(kind, input, elevation)?;
            Ok(PrimitiveSpec::Box {
                origin,
                length,
                width,
                height,
            })
        }
        PrimitiveKind::Wedge => {
            let (origin, length, width, height) = basal_extents(kind, input, elevation)?;
            Ok(PrimitiveSpec::Wedge {
                origin,
                length,
                width,
                height,
            })
        }
        PrimitiveKind::Pyramid => pyramid_spec(input),
        PrimitiveKind::Cylinder => cylinder_spec(input, elevation),
        PrimitiveKind::Sphere => sphere_spec(input, elevation),
        PrimitiveKind::Cone => cone_spec(input, elevation),
        PrimitiveKind::Torus => torus_spec(input, elevation),
    }
}

/// `BOX` / `WEDGE`: the shared prompt set (corner/centre, `Cube`/`Length`,
/// height). Both kinds reduce to one axis-aligned `{origin, length, width,
/// height}`; only the modeler's body differs.
fn basal_extents(
    kind: PrimitiveKind,
    input: &SolidPrimitiveCommand,
    elevation: f64,
) -> CadResult<(Point3, f64, f64, f64)> {
    match input {
        SolidPrimitiveCommand::Points(points) => match kind {
            PrimitiveKind::Box => {
                require_arity(kind, points, 2)?;
                let (p0, p1) = (points[0], points[1]);
                require_finite(kind, &[p0, p1])?;
                let (low, high) = (min_point(p0, p1), max_point(p0, p1));
                Ok((
                    low,
                    require_extent(kind, "dx (|p1.x - p0.x|)", high.x - low.x)?,
                    require_extent(kind, "dy (|p1.y - p0.y|)", high.y - low.y)?,
                    require_extent(kind, "dz (|p1.z - p0.z|)", high.z - low.z)?,
                ))
            }
            PrimitiveKind::Wedge => wedge_points(points),
            // `solid_primitive_spec` routes every other kind away from here.
            _ => Err(option_not_supported(kind, input)),
        },
        SolidPrimitiveCommand::Cube { corner, edge } => {
            let (x, y) = require_plan_point(kind, "corner", *corner)?;
            let edge = require_extent(kind, "edge", *edge)?;
            Ok((point3(x, y, plan_z(elevation)?), edge, edge, edge))
        }
        SolidPrimitiveCommand::CenterPoints { center, corner } => {
            // AutoCAD's `Center` is centred on the point **in height too**:
            // `corner` is one corner and its mirror image about `center` is the
            // opposite corner, so the body spans `center ± (corner - center)`.
            require_finite(kind, &[*center, *corner])?;
            let opposite = point3(
                2.0 * center.x - corner.x,
                2.0 * center.y - corner.y,
                2.0 * center.z - corner.z,
            );
            require_finite(kind, &[opposite])?;
            let (low, high) = (min_point(opposite, *corner), max_point(opposite, *corner));
            Ok((
                low,
                require_extent(kind, "2 * |corner.x - center.x|", high.x - low.x)?,
                require_extent(kind, "2 * |corner.y - center.y|", high.y - low.y)?,
                require_extent(kind, "2 * |corner.z - center.z|", high.z - low.z)?,
            ))
        }
        SolidPrimitiveCommand::CenterCube { center, edge } => {
            let (x, y) = require_plan_point(kind, "center", *center)?;
            let edge = require_extent(kind, "edge", *edge)?;
            let half = edge / 2.0;
            let origin = point3(x - half, y - half, plan_z(elevation)? - half);
            require_finite(kind, &[origin])?;
            Ok((origin, edge, edge, edge))
        }
        SolidPrimitiveCommand::CenterLengths {
            center,
            length,
            width,
            height,
        } => {
            let (x, y) = require_plan_point(kind, "center", *center)?;
            let [length, width, height] = require_extents(
                kind,
                [("length", *length), ("width", *width), ("height", *height)],
            )?;
            let origin = point3(
                x - length / 2.0,
                y - width / 2.0,
                plan_z(elevation)? - height / 2.0,
            );
            require_finite(kind, &[origin])?;
            Ok((origin, length, width, height))
        }
        SolidPrimitiveCommand::Length {
            corner,
            length,
            width,
            height,
        } => {
            let (x, y) = require_plan_point(kind, "corner", *corner)?;
            let [length, width, height] = require_extents(
                kind,
                [("length", *length), ("width", *width), ("height", *height)],
            )?;
            Ok((point3(x, y, plan_z(elevation)?), length, width, height))
        }
        other => Err(option_not_supported(kind, other)),
    }
}

/// `WEDGE`'s explicit 3D form: two base corners and a height point.
///
/// The base must be one plane (a tilted base has no single height), and the tall
/// face is at the **minimum-x** corner (`polyhedra::wedge_brep`'s convention), so
/// reversing `p0`/`p1` never moves it.
fn wedge_points(points: &[Point3]) -> CadResult<(Point3, f64, f64, f64)> {
    let kind = PrimitiveKind::Wedge;
    require_arity(kind, points, 3)?;
    let (p0, p1, p2) = (points[0], points[1], points[2]);
    require_finite(kind, &[p0, p1, p2])?;
    let dx = (p1.x - p0.x).abs();
    let dy = (p1.y - p0.y).abs();
    let length = require_extent(kind, "dx (|p1.x - p0.x|)", dx)?;
    let width = require_extent(kind, "dy (|p1.y - p0.y|)", dy)?;
    let scale = length.max(width).max(1.0);
    if (p0.z - p1.z).abs() > 1e-9 * scale {
        return Err(CadError::InvalidInput(format!(
            "wedge.base_not_coplanar: the two base corners must share z, got {} and {}",
            p0.z, p1.z
        )));
    }
    let origin = Point3 {
        x: p0.x.min(p1.x),
        y: p0.y.min(p1.y),
        z: p0.z.min(p1.z),
    };
    let height = require_extent(kind, "height (p2.z - base z)", p2.z - origin.z)?;
    Ok((origin, length, width, height))
}

/// `PYRAMID`: only the explicit square-base form is modelled (see the module
/// docs for why the `Edge`/`Sides`/`Top radius` constructions are absent).
fn pyramid_spec(input: &SolidPrimitiveCommand) -> CadResult<PrimitiveSpec> {
    let kind = PrimitiveKind::Pyramid;
    let SolidPrimitiveCommand::Points(points) = input else {
        return Err(option_not_supported(kind, input));
    };
    require_arity(kind, points, 3)?;
    let (p0, p1, apex) = (points[0], points[1], points[2]);
    require_finite(kind, &[p0, p1, apex])?;
    let dx = (p1.x - p0.x).abs();
    let dy = (p1.y - p0.y).abs();
    let length = require_extent(kind, "dx (|p1.x - p0.x|)", dx)?;
    let width = require_extent(kind, "dy (|p1.y - p0.y|)", dy)?;
    // A rectangular base would need a different apex rule than the seam's
    // square-base `Pyramid`; refuse rather than silently square it.
    let scale = length.max(width).max(1.0);
    if (dx - dy).abs() > 1e-9 * scale {
        return Err(CadError::InvalidInput(format!(
            "pyramid.invalid_base: the base must be square, got {dx} x {dy}"
        )));
    }
    // The base is one horizontal plane (`z = p0.z = p1.z`), exactly like WEDGE:
    // a tilted base has no single height, and silently taking the lower z would
    // drop the other corner's z.
    if (p0.z - p1.z).abs() > 1e-9 * scale {
        return Err(CadError::InvalidInput(format!(
            "pyramid.base_not_coplanar: the two base corners must share z, got {} and {}",
            p0.z, p1.z
        )));
    }
    let base_z = p0.z.min(p1.z);
    let centre = Point3 {
        x: (p0.x + p1.x) / 2.0,
        y: (p0.y + p1.y) / 2.0,
        z: base_z,
    };
    let off_centre = ((apex.x - centre.x).powi(2) + (apex.y - centre.y).powi(2)).sqrt();
    if off_centre > 1e-9 * scale {
        return Err(CadError::InvalidInput(format!(
            "pyramid.invalid_base: the apex must be over the base centre, it is \
             {off_centre} away in xy"
        )));
    }
    Ok(PrimitiveSpec::Pyramid {
        base_center: centre,
        length,
        height: require_extent(kind, "height (apex.z - base z)", apex.z - base_z)?,
    })
}

fn cylinder_spec(input: &SolidPrimitiveCommand, elevation: f64) -> CadResult<PrimitiveSpec> {
    let kind = PrimitiveKind::Cylinder;
    match input {
        SolidPrimitiveCommand::Points(points) => {
            require_arity(kind, points, 3)?;
            let (centre, radius_point, top) = (points[0], points[1], points[2]);
            require_finite(kind, &[centre, radius_point, top])?;
            Ok(PrimitiveSpec::Cylinder {
                origin: centre,
                radius: require_extent(
                    kind,
                    "radius (|r - c| in xy)",
                    planar_distance(centre, radius_point),
                )?,
                height: require_extent(kind, "height (t.z - c.z)", top.z - centre.z)?,
            })
        }
        SolidPrimitiveCommand::CylinderPlan { base, height } => {
            let (origin, radius) = plan_circle(kind, *base, elevation)?;
            Ok(PrimitiveSpec::Cylinder {
                origin,
                radius,
                height: require_extent(kind, "height", *height)?,
            })
        }
        other => Err(option_not_supported(kind, other)),
    }
}

fn sphere_spec(input: &SolidPrimitiveCommand, elevation: f64) -> CadResult<PrimitiveSpec> {
    let kind = PrimitiveKind::Sphere;
    match input {
        SolidPrimitiveCommand::Points(points) => {
            require_arity(kind, points, 2)?;
            let (centre, radius_point) = (points[0], points[1]);
            require_finite(kind, &[centre, radius_point])?;
            Ok(PrimitiveSpec::Sphere {
                origin: centre,
                radius: require_extent(kind, "radius (|p1 - p0|)", distance(centre, radius_point))?,
            })
        }
        SolidPrimitiveCommand::SphereDiameter { center, diameter } => {
            let (x, y) = require_plan_point(kind, "center", *center)?;
            Ok(PrimitiveSpec::Sphere {
                origin: point3(x, y, plan_z(elevation)?),
                radius: require_extent(kind, "diameter", *diameter)? / 2.0,
            })
        }
        SolidPrimitiveCommand::SphereTwoPoint { first, second } => {
            // `2P` is the one option form whose points are documented as
            // anywhere in 3D space, so it carries explicit coordinates.
            require_finite(kind, &[*first, *second])?;
            let radius = require_extent(
                kind,
                "2P diameter (|second - first|)",
                distance(*first, *second),
            )? / 2.0;
            Ok(PrimitiveSpec::Sphere {
                origin: Point3 {
                    x: (first.x + second.x) / 2.0,
                    y: (first.y + second.y) / 2.0,
                    z: (first.z + second.z) / 2.0,
                },
                radius,
            })
        }
        other => Err(option_not_supported(kind, other)),
    }
}

fn cone_spec(input: &SolidPrimitiveCommand, elevation: f64) -> CadResult<PrimitiveSpec> {
    let kind = PrimitiveKind::Cone;
    match input {
        SolidPrimitiveCommand::Points(points) => {
            require_arity(kind, points, 3)?;
            let (centre, radius_point, top) = (points[0], points[1], points[2]);
            require_finite(kind, &[centre, radius_point, top])?;
            let radius_bottom = require_extent(
                kind,
                "radius_bottom (|r - c| in xy)",
                planar_distance(centre, radius_point),
            )?;
            // `0` is the seam's full-cone apex; any other value is a frustum top.
            let radius_top = planar_distance(centre, top);
            if !radius_top.is_finite() {
                return Err(CadError::InvalidInput(
                    "cone.degenerate_extent: radius_top must be finite".into(),
                ));
            }
            require_distinct_radii(radius_bottom, radius_top)?;
            Ok(PrimitiveSpec::Cone {
                origin: centre,
                radius_bottom,
                radius_top,
                height: require_extent(kind, "height (t.z - c.z)", top.z - centre.z)?,
            })
        }
        SolidPrimitiveCommand::ConePlan {
            base,
            top_radius,
            height,
        } => {
            let (origin, radius_bottom) = plan_circle(kind, *base, elevation)?;
            let radius_top = require_non_negative(kind, "top_radius", *top_radius)?;
            require_distinct_radii(radius_bottom, radius_top)?;
            Ok(PrimitiveSpec::Cone {
                origin,
                radius_bottom,
                radius_top,
                height: require_extent(kind, "height", *height)?,
            })
        }
        other => Err(option_not_supported(kind, other)),
    }
}

fn torus_spec(input: &SolidPrimitiveCommand, elevation: f64) -> CadResult<PrimitiveSpec> {
    let kind = PrimitiveKind::Torus;
    match input {
        SolidPrimitiveCommand::Points(points) => {
            require_arity(kind, points, 3)?;
            let (centre, major_point, minor_point) = (points[0], points[1], points[2]);
            require_finite(kind, &[centre, major_point, minor_point])?;
            let major = require_extent(
                kind,
                "major_radius (|m - c| in xy)",
                planar_distance(centre, major_point),
            )?;
            let minor = require_extent(
                kind,
                "minor_radius (|n - m|)",
                distance(major_point, minor_point),
            )?;
            require_tube_fits(major, minor)?;
            Ok(PrimitiveSpec::Torus {
                origin: centre,
                major_radius: major,
                minor_radius: minor,
            })
        }
        SolidPrimitiveCommand::TorusPlan { ring, tube } => {
            let (origin, major) = plan_circle(kind, *ring, elevation)?;
            let minor = plan_length(kind, *tube)?;
            require_tube_fits(major, minor)?;
            Ok(PrimitiveSpec::Torus {
                origin,
                major_radius: major,
                minor_radius: minor,
            })
        }
        other => Err(option_not_supported(kind, other)),
    }
}

/// Resolve one plan circle to its centre (base plane z = `ELEV`) and radius.
fn plan_circle(
    kind: PrimitiveKind,
    circle: PlanCircle,
    elevation: f64,
) -> CadResult<(Point3, f64)> {
    let z = plan_z(elevation)?;
    match circle {
        PlanCircle::Radius { center, radius } => {
            let (x, y) = require_plan_point(kind, "center", center)?;
            Ok((point3(x, y, z), require_extent(kind, "radius", radius)?))
        }
        PlanCircle::Diameter { center, diameter } => {
            let (x, y) = require_plan_point(kind, "center", center)?;
            let radius = require_extent(kind, "diameter", diameter)? / 2.0;
            Ok((point3(x, y, z), radius))
        }
        PlanCircle::TwoPoint { first, second } => {
            let (x0, y0) = require_plan_point(kind, "first diameter point", first)?;
            let (x1, y1) = require_plan_point(kind, "second diameter point", second)?;
            let diameter = ((x1 - x0).powi(2) + (y1 - y0).powi(2)).sqrt();
            let radius =
                require_extent(kind, "2P diameter (|second - first| in xy)", diameter)? / 2.0;
            Ok((point3((x0 + x1) / 2.0, (y0 + y1) / 2.0, z), radius))
        }
    }
}

/// Resolve TORUS's tube input to a radius.
fn plan_length(kind: PrimitiveKind, length: PlanLength) -> CadResult<f64> {
    match length {
        PlanLength::Radius(radius) => require_extent(kind, "tube radius", radius),
        PlanLength::Diameter(diameter) => {
            Ok(require_extent(kind, "tube diameter", diameter)? / 2.0)
        }
        PlanLength::TwoPoint { first, second } => {
            let (x0, y0) = require_plan_point(kind, "first tube point", first)?;
            let (x1, y1) = require_plan_point(kind, "second tube point", second)?;
            let radius = ((x1 - x0).powi(2) + (y1 - y0).powi(2)).sqrt();
            require_extent(kind, "tube radius (|second - first| in xy)", radius)
        }
    }
}

/// The stable refusal for a construction the kind does not take.
fn option_not_supported(kind: PrimitiveKind, input: &SolidPrimitiveCommand) -> CadError {
    CadError::InvalidInput(format!(
        "solid.option_not_supported: {} does not take the {} construction",
        kind.as_str().to_ascii_uppercase(),
        option_name(input)
    ))
}

/// The name a refusal uses for one payload variant.
fn option_name(input: &SolidPrimitiveCommand) -> &'static str {
    match input {
        SolidPrimitiveCommand::Points(_) => "explicit points",
        SolidPrimitiveCommand::Cube { .. } => "Cube",
        SolidPrimitiveCommand::CenterPoints { .. } => "Center (corner)",
        SolidPrimitiveCommand::CenterCube { .. } => "Center (Cube)",
        SolidPrimitiveCommand::CenterLengths { .. } => "Center (Length)",
        SolidPrimitiveCommand::Length { .. } => "Length",
        SolidPrimitiveCommand::CylinderPlan { .. } => "plan base circle",
        SolidPrimitiveCommand::ConePlan { .. } => "plan base circle with Top radius",
        SolidPrimitiveCommand::SphereDiameter { .. } => "Diameter",
        SolidPrimitiveCommand::SphereTwoPoint { .. } => "2P",
        SolidPrimitiveCommand::TorusPlan { .. } => "plan ring and tube",
    }
}

/// The session `ELEV` token as a base-plane z.
///
/// The session setter already refuses a non-finite token; a caller that wrote
/// the field directly still gets a named refusal instead of a non-finite origin.
fn plan_z(elevation: f64) -> CadResult<f64> {
    if elevation.is_finite() {
        return Ok(elevation);
    }
    Err(CadError::InvalidInput(format!(
        "elev.invalid_value: the session ELEV token {elevation} is not finite, so no solid can be \
         created at that elevation"
    )))
}

/// A cone whose top radius equals its base radius is a cylinder.
fn require_distinct_radii(radius_bottom: f64, radius_top: f64) -> CadResult<()> {
    if (radius_top - radius_bottom).abs() > 1e-12 * radius_bottom.max(1.0) {
        return Ok(());
    }
    Err(CadError::InvalidInput(format!(
        "cone.equal_radii: radius_top {radius_top} equals radius_bottom {radius_bottom}; use \
         CYLINDER for a straight tube"
    )))
}

/// A torus tube must be thinner than the ring.
fn require_tube_fits(major: f64, minor: f64) -> CadResult<()> {
    if minor < major {
        return Ok(());
    }
    Err(CadError::InvalidInput(format!(
        "torus.degenerate_extent: the minor radius {minor} must be smaller than the major radius \
         {major}"
    )))
}

/// Enforce the explicit point count of one primitive family.
fn require_arity(kind: PrimitiveKind, points: &[Point3], expected: usize) -> CadResult<()> {
    if points.len() == expected {
        return Ok(());
    }
    let word = if expected == 2 { "two" } else { "three" };
    Err(CadError::InvalidInput(format!(
        "{}.requires_{word}_points: {} takes exactly {expected} point(s), got {}",
        kind.as_str(),
        kind.as_str().to_ascii_uppercase(),
        points.len()
    )))
}

fn require_finite(kind: PrimitiveKind, points: &[Point3]) -> CadResult<()> {
    if points.iter().all(|point| is_finite_point(*point)) {
        return Ok(());
    }
    Err(CadError::InvalidInput(format!(
        "{}.non_finite_point: every point must have finite x, y and z",
        kind.as_str()
    )))
}

/// Validate one plan point and return its `(x, y)`.
fn require_plan_point(kind: PrimitiveKind, name: &str, [x, y]: [f64; 2]) -> CadResult<(f64, f64)> {
    if x.is_finite() && y.is_finite() {
        return Ok((x, y));
    }
    Err(CadError::InvalidInput(format!(
        "{}.non_finite_point: {name} must have finite x and y, got ({x}, {y})",
        kind.as_str()
    )))
}

fn require_extent(kind: PrimitiveKind, name: &str, value: f64) -> CadResult<f64> {
    if value.is_finite() && value > 0.0 {
        return Ok(value);
    }
    Err(CadError::InvalidInput(format!(
        "{}.degenerate_extent: {name} must be finite and positive, got {value}",
        kind.as_str()
    )))
}

/// Validate several named extents, reporting the first offender by name.
fn require_extents<const N: usize>(
    kind: PrimitiveKind,
    values: [(&str, f64); N],
) -> CadResult<[f64; N]> {
    let mut out = [0.0; N];
    for (slot, (name, value)) in out.iter_mut().zip(values) {
        *slot = require_extent(kind, name, value)?;
    }
    Ok(out)
}

/// A cone's top radius may be zero (a pointed cone) but never negative.
fn require_non_negative(kind: PrimitiveKind, name: &str, value: f64) -> CadResult<f64> {
    if value.is_finite() && value >= 0.0 {
        return Ok(value);
    }
    Err(CadError::InvalidInput(format!(
        "{}.degenerate_extent: {name} must be finite and non-negative, got {value}",
        kind.as_str()
    )))
}

fn point3(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

fn planar_distance(a: Point3, b: Point3) -> f64 {
    ((b.x - a.x).powi(2) + (b.y - a.y).powi(2)).sqrt()
}

fn min_point(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.x.min(b.x),
        y: a.y.min(b.y),
        z: a.z.min(b.z),
    }
}

fn max_point(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.x.max(b.x),
        y: a.y.max(b.y),
        z: a.z.max(b.z),
    }
}
