//! The raster-image half of the default provider.
//!
//! Split out of `default.rs` for the file-size ceiling (`check-file-size.py`).
//! Everything here is one IMAGE build: texture resolution, the entity clip cut
//! into convex pieces (one fragment per piece), and the frame-only refusal path
//! that names its reason instead of drawing a placeholder image. A hidden image
//! still draws nothing at all. The [`ImageGeometry`] struct is exactly the
//! fields `default.rs`'s dispatch already matched, so the arm body moved
//! verbatim.

use crate::*;

use super::super::image::{
    image_basis_is_degenerate, image_clip_pieces, image_clip_reason, image_frame_corners,
    resolve_image_texture,
};

/// The matched fields of one `SemanticGeometry::Image`, borrowed from the entity.
pub(super) struct ImageGeometry<'a> {
    pub(super) origin: &'a Point3,
    pub(super) u: &'a Point3,
    pub(super) v: &'a Point3,
    pub(super) pixels: &'a [f64; 2],
    pub(super) file: &'a Option<String>,
    pub(super) clip: &'a Option<ImageClip>,
    pub(super) visible: &'a bool,
    pub(super) adjust: &'a ImageAdjust,
}

/// Build a raster image's fragments, or its frame plus an honest refusal.
///
/// A texture that resolves draws the placed quad (one fragment per convex clip
/// piece); anything else draws the frame and reports the reason, so a missing
/// texture or an unusable clip never becomes a fabricated image.
pub(super) fn image_fragment(
    entity: &DbEntity,
    context: &RepresentationContext,
    geometry: ImageGeometry<'_>,
    source: SelectionRef,
    geometry_source: GeometrySource,
    representation: &mut DisplayRepresentation,
) {
    let ImageGeometry {
        origin,
        u,
        v,
        pixels,
        file,
        clip,
        visible,
        adjust,
    } = geometry;
    // A hidden image draws nothing; never substitute a fake frame.
    if *visible {
        let (resource, decoded) = resolve_image_texture(file.as_deref(), context.images.as_deref());
        // Map the entity-level clip once: a convex clip stays one
        // piece (byte-identical to the behaviour before concave clips
        // were supported), a concave clip becomes one convex piece per
        // ear of its triangulation. A clip that is not a usable region
        // is refused by name below, never silently widened.
        let clip = clip
            .as_ref()
            .map(|clip| image_clip_pieces(clip, *origin, *u, *v, *pixels));
        // The clip's own verdict, independent of texture resolution.
        let clip_refusal: Option<(&'static str, String)> = match &clip {
            None | Some(Ok(Some(_))) => None,
            Some(Ok(None)) => Some((
                image_clip_reason::UNSUPPORTED,
                "the image clip has no area inside the image rectangle; only the \
                 frame is drawn"
                    .to_string(),
            )),
            Some(Err(refusal)) => Some((refusal.code, refusal.message.clone())),
        };
        match (&resource, decoded) {
            (Some(key), Some(_image))
                if clip_refusal.is_none() && !image_basis_is_degenerate(*u, *v, *pixels) =>
            {
                // The texture's unit square maps to the placed quad.
                let transform = Transform3::from_basis(
                    cad_geometry::scale(*u, pixels[0]),
                    cad_geometry::scale(*v, pixels[1]),
                    *origin,
                );
                // No clip draws the whole unit square; a clip draws one
                // fragment per convex piece, so the union of the
                // fragments is exactly the visible texture region (also
                // for a concave clip). The refusal cases are already
                // handled below, so they contribute nothing here.
                let pieces = match clip {
                    None => vec![None],
                    Some(Ok(Some(pieces))) => pieces.into_iter().map(Some).collect(),
                    Some(Ok(None) | Err(_)) => Vec::new(),
                };
                for piece in pieces {
                    representation.fragments.push(DisplayFragment {
                        source: source.clone(),
                        geometry_source: geometry_source.clone(),
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
                        primitive: DisplayPrimitive::Image {
                            resource: key.clone(),
                            transform,
                            clip: piece.map(|vertices| Arc::from(vertices.into_boxed_slice())),
                            adjust: *adjust,
                        },
                    });
                }
            }
            (resource, decoded) => {
                // No usable texture, or a clip that is not a usable
                // region: draw the frame only and report the honest
                // reason instead of a placeholder image.
                let (code, message) = match (resource, &clip_refusal, decoded) {
                    (None, _, _) => (
                        "image.unresolved",
                        "image has no resolvable resource key; only the frame is drawn".to_string(),
                    ),
                    // An outside/mask clip keeps the region *outside* its
                    // boundary, which a visible-region polygon cannot
                    // express; a self-intersecting ring or too few /
                    // degenerate points has no interior to draw. Each
                    // refusal names itself and draws no region at all.
                    (Some(_), Some((code, message)), _) => (*code, message.clone()),
                    // A cache miss is a missing texture, not a
                    // host-confirmed decode failure.
                    (Some(key), None, None) => (
                        "image.unresolved",
                        format!(
                            "no decoded texture for '{}' was available; only the frame is drawn",
                            key.0
                        ),
                    ),
                    (Some(_), None, Some(_)) => (
                        "image.degenerate_frame",
                        "image frame has a degenerate basis; only the frame is drawn".to_string(),
                    ),
                };
                representation.completeness = representation
                    .completeness
                    .clone()
                    .combine(Completeness::Partial(vec![message.clone()]));
                representation.diagnostics.push(Diagnostic {
                    object: Some(ObjectId(entity.id.0)),
                    code: code.into(),
                    message,
                });
                let corners = image_frame_corners(*origin, *u, *v, *pixels);
                if corners.len() >= 2 {
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
                        primitive: DisplayPrimitive::Lines(Arc::from(corners.into_boxed_slice())),
                    });
                }
            }
        }
    }
}
