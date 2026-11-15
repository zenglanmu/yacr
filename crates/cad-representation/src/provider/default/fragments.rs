//! The chord/tessellation half of the default provider.
//!
//! Split out of `default.rs` for the file-size ceiling (`check-file-size.py`).
//! Everything here is the *derived display* of one entity's geometry: the chord
//! budget ([`chord_tessellation_params`]), the analytic conic a curve maps to
//! ([`conic_from_geometry`]) and the two fragment builders every fallback path
//! ends in. `DefaultRepresentationProvider` stays the only place that decides
//! *which* path an entity takes.

use crate::*;

/// The chord-tessellation tolerance a build uses for curve geometry.
///
/// Shared by the provider's chord fallback and by the conic chord expansion so
/// the two can never drift apart. `VIEWRES`
/// ([`RepresentationContext::curve_segments`]) is applied here as a **ceiling**
/// on segments per curve, so every chord path — splines, polyline bulge arcs,
/// the conic chord fallback and the wide-polyline band — reads one budget.
///
/// Two honest facts travel with that:
///
/// * the tolerance is still the display-pixel budget, so raising `VIEWRES`
///   above what the tolerance already asks for changes nothing (it is a
///   ceiling, not a smoothness setting);
/// * circles, arcs and ellipses reach the display as analytic conics and are
///   never chord-tessellated there, so `VIEWRES` cannot make them smoother or
///   coarser. Only the paths that really produce chords are bounded.
pub(crate) fn chord_tessellation_params(context: &RepresentationContext) -> TessellationParams {
    let mut params = TessellationParams {
        tolerance: context.tolerance.display_pixels.max(1e-12),
        ..TessellationParams::default()
    };
    if let Some(ceiling) = context.curve_segments {
        // `arc_segments_for_tolerance` clamps with `min.max(1)..max.max(1)`, so
        // the floor must never exceed the ceiling: a `VIEWRES` ceiling below the
        // default floor lowers the floor with it instead of panicking.
        params.max_segments = ceiling.max(1);
        params.min_segments = params.min_segments.min(params.max_segments);
    }
    params
}

/// Build the analytic conic for a curve entity, or `None` when the source is
/// degenerate or non-finite (the caller then keeps the legacy chord path).
///
/// The semi-diameter vectors mirror the frame [`tessellate_geometry`] uses, so
/// expanding the conic back through that same tessellation reproduces the
/// pre-conic chord output.
pub(super) fn conic_from_geometry(geometry: &SemanticGeometry) -> Option<ConicSegment> {
    match geometry {
        SemanticGeometry::Circle {
            center,
            normal,
            radius,
        } => {
            let (ax, ay, _) = cad_geometry::arbitrary_axis(*normal);
            let conic = ConicSegment {
                center: *center,
                major: cad_geometry::scale(ax, *radius),
                minor: cad_geometry::scale(ay, *radius),
                start: 0.0,
                sweep: std::f64::consts::TAU,
                full: true,
            };
            conic.axes_are_non_degenerate().then_some(conic)
        }
        SemanticGeometry::Arc {
            center,
            normal,
            radius,
            start,
            sweep,
        } => {
            let (ax, ay, _) = cad_geometry::arbitrary_axis(*normal);
            let conic = ConicSegment {
                center: *center,
                major: cad_geometry::scale(ax, *radius),
                minor: cad_geometry::scale(ay, *radius),
                start: *start,
                sweep: *sweep,
                full: sweep.abs() >= std::f64::consts::TAU - 1e-9,
            };
            conic.axes_are_non_degenerate().then_some(conic)
        }
        SemanticGeometry::Ellipse {
            center,
            normal,
            major_axis,
            ratio,
            start,
            sweep,
        } => {
            let major_len = cad_geometry::length(*major_axis);
            if !(major_len.is_finite() && major_len > 0.0) {
                return None;
            }
            let u = cad_geometry::scale(*major_axis, 1.0 / major_len);
            // The minor direction is the in-plane +90° rotation of the major
            // axis, matching `ellipse_minor_dir` in the tessellation frame.
            let minor_dir = {
                let c = cad_geometry::cross(*normal, u);
                let len = cad_geometry::length(c);
                if len >= 1e-12 {
                    cad_geometry::scale(c, 1.0 / len)
                } else {
                    return None;
                }
            };
            let conic = ConicSegment {
                center: *center,
                major: *major_axis,
                minor: cad_geometry::scale(minor_dir, major_len * ratio.abs()),
                start: *start,
                sweep: *sweep,
                full: sweep.abs() >= std::f64::consts::TAU - 1e-9,
            };
            conic.axes_are_non_degenerate().then_some(conic)
        }
        _ => None,
    }
}

/// Push one prepared triangle mesh (for example a wide polyline stroke band)
/// with the same pre-expansion style placeholders every provider fragment
/// carries.
///
/// The placeholders are resolved by `expand_entity` (ByLayer/ByBlock and layer
/// style), so the stroke follows the entity's resolved colour, lineweight,
/// linetype and alpha exactly like the centreline path. The mesh reaches the
/// clip stage as [`DisplayPrimitive::Mesh`], which `emit_clipped` cuts with
/// `clip_mesh_to_rect`, so an `XCLIP` rectangle clips the stroke too.
pub(super) fn push_mesh_fragment(
    entity: &DbEntity,
    mesh: Mesh,
    source: SelectionRef,
    geometry_source: GeometrySource,
    representation: &mut DisplayRepresentation,
) {
    if mesh.triangles.is_empty() {
        representation.completeness = weaker_completeness(
            &representation.completeness,
            &Completeness::Partial(vec!["the wide polyline stroke produced no triangles".into()]),
        );
        representation.diagnostics.push(Diagnostic {
            object: Some(ObjectId(entity.id.0)),
            code: "representation.polyline_stroke_empty".into(),
            message: "the wide polyline stroke produced no triangles".into(),
        });
        return;
    }
    representation.fragments.push(DisplayFragment {
        source,
        geometry_source,
        precision: Precision::Analytic,
        alpha: 1.0,
        draw_order: entity.draw_order,
        color: DEFAULT_RENDER_COLOR,
        color_unresolved: true,
        lineweight: DEFAULT_LINEWEIGHT_MM,
        lineweight_unresolved: true,
        linetype: LinetypePattern::continuous(),
        linetype_unresolved: true,
        linetype_scale: 1.0,
        primitive: DisplayPrimitive::Mesh(Arc::new(mesh)),
    });
}

/// Tessellate a curve-like entity into a chord polyline and push it as a
/// `Lines` fragment (the pre-conic display path).
///
/// A shape that yields fewer than two points becomes an explicit `Missing`
/// result with a diagnostic, never an empty success.
pub(super) fn push_chord_fragment(
    entity: &DbEntity,
    geometry: &SemanticGeometry,
    context: &RepresentationContext,
    source: SelectionRef,
    geometry_source: GeometrySource,
    representation: &mut DisplayRepresentation,
) {
    let points = tessellate_geometry(geometry, chord_tessellation_params(context));
    if points.len() >= 2 {
        representation.fragments.push(DisplayFragment {
            source,
            geometry_source,
            precision: Precision::Analytic,
            alpha: 1.0,
            draw_order: entity.draw_order,
            color: DEFAULT_RENDER_COLOR,
            color_unresolved: true,
            lineweight: DEFAULT_LINEWEIGHT_MM,
            lineweight_unresolved: true,
            linetype: LinetypePattern::continuous(),
            linetype_unresolved: true,
            linetype_scale: 1.0,
            primitive: DisplayPrimitive::Lines(Arc::from(points.into_boxed_slice())),
        });
    } else {
        representation.completeness =
            Completeness::Missing(vec!["no display representation in this build".into()]);
        representation.diagnostics.push(Diagnostic {
            object: Some(ObjectId(entity.id.0)),
            code: "representation.empty".into(),
            message: format!("{} produced no drawable points", entity.object.type_key),
        });
    }
}
