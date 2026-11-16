//! Shape-relative tessellation policy for solid bodies.
//!
//! This is the **one** tolerance rule shared by the two paths that produce a
//! body's display cache:
//!
//! * import-time ACIS lift (`cad-import-acadrust` → `tessellate_exchange`);
//! * command-time primitive creation (`cad-app` → the injected `SolidModeler`
//!   → `tessellate_exchange`).
//!
//! The deflection is **relative to the body's size**, not a drawing-unit
//! constant: `1e-3` (0.1 %) of the bounding-box diagonal. A 2 mm part and a
//! 2000 mm part therefore get the same relative facet density, and — this is
//! the point of the rule — an absurdly large primitive (a sphere of radius
//! `1e6`) does not ask for a mesh with hundreds of millions of triangles: its
//! chord budget grows with the radius, so the primitive's triangle count stays
//! ~constant. An absolute default (0.01) instead makes the tessellator refine
//! until the budget check fires (or, worse, materialises the mesh first).
//!
//! The angular deflection keeps the tessellator default (0.35 rad): it bounds
//! the facet-normal error rather than a length, so it is not derived from size.
//!
//! A body with no measurable extent falls back to a **unit diagonal** (`1.0`),
//! so the policy stays finite for degenerate/unsupported B-reps — which the
//! tessellator/modeler then refuses by name instead of producing a bogus mesh.

use crate::request::TessellationTolerance;
use cad_domain::{BrepCurve, BrepData, BrepPlacement, BrepSurface, Point3};

/// Relative chordal deflection applied to every solid body.
pub const SHAPE_RELATIVE_LINEAR_DEFLECTION: f64 = 1.0e-3;

/// The shape-relative tolerance for `brep`, plus the diagonal it came from.
pub fn shape_relative_tolerance(brep: &BrepData) -> (TessellationTolerance, f64) {
    let diagonal = brep_diagonal(brep).unwrap_or(1.0);
    let linear_deflection = (diagonal * SHAPE_RELATIVE_LINEAR_DEFLECTION).clamp(1e-9, f64::MAX);
    (
        TessellationTolerance {
            linear_deflection,
            angular_deflection: TessellationTolerance::default().angular_deflection,
        },
        diagonal,
    )
}

/// The bounding-box diagonal of `brep`, when it has a measurable extent.
pub fn brep_diagonal(brep: &BrepData) -> Option<f64> {
    brep_bounds(brep)
        .map(|(min, max)| {
            let d = (max.x - min.x, max.y - min.y, max.z - min.z);
            (d.0 * d.0 + d.1 * d.1 + d.2 * d.2).sqrt()
        })
        .filter(|diagonal| diagonal.is_finite() && *diagonal > 0.0)
}

/// Conservative axis-aligned bounds of a neutral B-rep, in world coordinates.
///
/// Every analytic surface contributes its centre ± its radius on all three
/// axes (a superset of the true extent), and every loop contributes its line
/// endpoints. The placement is applied to each sample. This is a *size* estimate
/// for the tolerance rule, not a display bound, so a conservative over-estimate
/// is preferable to missing a curved face.
pub fn brep_bounds(brep: &BrepData) -> Option<(Point3, Point3)> {
    let placement = brep.placement.unwrap_or_else(BrepPlacement::identity);
    let mut samples: Vec<Point3> = Vec::new();
    for shell in &brep.shells {
        for face in &shell.faces {
            surface_samples(&face.surface, &mut samples);
            for loop_ in &face.loops {
                for curve in &loop_.edges {
                    match curve {
                        BrepCurve::Line { start, end } => {
                            samples.push(*start);
                            samples.push(*end);
                        }
                        BrepCurve::Circle { center, radius, .. } => {
                            axis_box(*center, radius.abs(), &mut samples)
                        }
                        BrepCurve::Unsupported { .. } => {}
                    }
                }
            }
        }
    }
    let mut iter = samples
        .into_iter()
        .filter(|p| p.x.is_finite() && p.y.is_finite() && p.z.is_finite())
        .map(|p| placement.apply(p));
    let first = iter.next()?;
    let mut min = first;
    let mut max = first;
    for p in iter {
        min.x = min.x.min(p.x);
        min.y = min.y.min(p.y);
        min.z = min.z.min(p.z);
        max.x = max.x.max(p.x);
        max.y = max.y.max(p.y);
        max.z = max.z.max(p.z);
    }
    Some((min, max))
}

/// The shared completeness reason for a body drawn as facets.
///
/// Both the import path (an ACIS body) and the create path (a created solid)
/// use this exact sentence, so a caller cannot see two different explanations
/// for the same situation: the exact B-rep is retained, but the **display** is
/// a discrete mesh at the given chord tolerance and is not refined when the
/// view zooms in.
pub fn discrete_mesh_reason(linear_deflection: f64) -> String {
    format!(
        "solid drawn as a discrete mesh at tolerance {linear_deflection:.6e}; it does not refine \
         when you zoom"
    )
}

fn surface_samples(surface: &BrepSurface, out: &mut Vec<Point3>) {
    match surface {
        BrepSurface::Plane { .. } | BrepSurface::Unsupported { .. } => {}
        BrepSurface::Sphere { center, radius, .. } => axis_box(*center, radius.abs(), out),
        BrepSurface::Torus {
            center,
            major_radius,
            minor_radius,
            ..
        } => axis_box(*center, major_radius.abs() + minor_radius.abs(), out),
        BrepSurface::Cylinder { origin, radius, .. } => axis_box(*origin, radius.abs(), out),
        BrepSurface::Cone { origin, radius, .. } => axis_box(*origin, radius.abs(), out),
    }
}

fn axis_box(center: Point3, radius: f64, out: &mut Vec<Point3>) {
    if !radius.is_finite() || radius <= 0.0 {
        return;
    }
    for (dx, dy, dz) in [
        (-1.0, 0.0, 0.0),
        (1.0, 0.0, 0.0),
        (0.0, -1.0, 0.0),
        (0.0, 1.0, 0.0),
        (0.0, 0.0, -1.0),
        (0.0, 0.0, 1.0),
    ] {
        out.push(Point3 {
            x: center.x + radius * dx,
            y: center.y + radius * dy,
            z: center.z + radius * dz,
        });
    }
}
