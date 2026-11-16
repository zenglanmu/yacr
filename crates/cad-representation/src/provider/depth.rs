//! `XCLIP` **clip depth**: the world z band a block reference keeps.
//!
//! `BlockClip` stores the DXF `SPATIAL_FILTER` front/back distances as a
//! block-local z band; [`super::clip::block_clip_depth`] maps it into world z.
//! This module is the display half: the z extent of one primitive and the
//! decision a band makes against it.
//!
//! # What the approximation is, exactly
//!
//! The clip plane normal is the block's `+Z`, so a depth band is a **z range**,
//! not a region. The display layer therefore filters primitives by their z
//! extent instead of cutting them at a depth plane:
//!
//! * a primitive whose whole extent is **outside** the band is omitted (exact:
//!   no point of it can be visible);
//! * a primitive that **straddles** a plane is kept **whole** and reported
//!   (`representation.xclip_depth_straddle`) — it is neither cut at the plane nor
//!   silently kept;
//! * a primitive whose z extent cannot be derived (a non-finite coordinate) is
//!   kept and reported (`representation.xclip_depth_unsupported`).
//!
//! Every primitive kind the display layer emits has a defined extent (see
//! [`primitive_depth_span`]), so there is no kind that quietly ignores the band.

use cad_db::DbEntity;

use crate::*;

use super::clip::ClipDepth;
use super::expand::report_xclip;

/// The z extent of one already-transformed primitive, or `None` when no finite
/// extent can be derived (a non-finite coordinate).
///
/// The clip plane normal is the block's `+Z`, so a depth band is a world z range
/// and the test is the primitive's z extent:
///
/// * **line / line bundle / mask** — the min/max z of the vertices;
/// * **mesh** — the min/max z of the vertices (for a body mesh, its stored edge
///   endpoints are included too);
/// * **conic** — `center.z ± (|major.z| + |minor.z|)`, an exact bound: a conic
///   whose bound is outside the band cannot have any point in it;
/// * **image** — the four z values of the unit square's corners under the
///   primitive's transform;
/// * **text** — its baseline z (a glyph lies in the block plane, so that one
///   value is the whole glyph's z).
pub(super) fn primitive_depth_span(primitive: &DisplayPrimitive) -> Option<(f64, f64)> {
    match primitive {
        DisplayPrimitive::Lines(points) | DisplayPrimitive::LineSegments(points) => {
            points_depth_span(points)
        }
        DisplayPrimitive::Conic(conic) => conic_depth_span(conic),
        DisplayPrimitive::Mesh(mesh) | DisplayPrimitive::MeshWithoutEdgeOverlay(mesh) => {
            points_depth_span(&mesh.vertices)
        }
        // The span covers the filled mesh **and** its stored wireframe: a body
        // whose edge set reaches outside the display triangles is still tested as
        // one primitive.
        DisplayPrimitive::MeshWithStoredEdges { mesh, edges } => {
            mesh_with_edges_depth_span(mesh, edges)
        }
        DisplayPrimitive::Text { origin, .. } => Some((origin.z, origin.z)),
        DisplayPrimitive::Image { transform, .. } => image_depth_span(transform),
        DisplayPrimitive::Mask { boundary, .. } => points_depth_span(boundary),
        // `Instance` is the recursion itself and never reaches this arm.
        DisplayPrimitive::Instance { .. } => None,
    }
}

/// The z extent of a world-space point list, `None` when empty or non-finite.
fn points_depth_span(points: &[Point3]) -> Option<(f64, f64)> {
    points_depth_span_of(std::iter::once(points))
}

/// The z extent of a mesh's vertices together with its stored edge endpoints.
fn mesh_with_edges_depth_span(mesh: &Mesh, edges: &[Point3]) -> Option<(f64, f64)> {
    points_depth_span_of([mesh.vertices.as_slice(), edges].into_iter())
}

/// The z extent over several world-space point lists, `None` when they are all
/// empty or any coordinate is non-finite.
fn points_depth_span_of<'a>(lists: impl Iterator<Item = &'a [Point3]>) -> Option<(f64, f64)> {
    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    for points in lists {
        for point in points {
            if !point.z.is_finite() {
                return None;
            }
            min = min.min(point.z);
            max = max.max(point.z);
        }
    }
    (min <= max).then_some((min, max))
}

/// The conservative z extent of a conic: its center plus both semi-diameters.
pub(super) fn conic_depth_span(conic: &ConicSegment) -> Option<(f64, f64)> {
    let reach = conic.major.z.abs() + conic.minor.z.abs();
    if !conic.center.z.is_finite() || !reach.is_finite() {
        return None;
    }
    Some((conic.center.z - reach, conic.center.z + reach))
}

/// The z extent of an image's unit square under its (already composed) transform.
fn image_depth_span(transform: &Transform3) -> Option<(f64, f64)> {
    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    for (x, y) in [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)] {
        let z = transform.apply_point(Point3 { x, y, z: 0.0 }).z;
        if !z.is_finite() {
            return None;
        }
        min = min.min(z);
        max = max.max(z);
    }
    Some((min, max))
}

/// The name one primitive kind is reported by when its z extent is unavailable.
pub(super) fn primitive_depth_kind(primitive: &DisplayPrimitive) -> &'static str {
    match primitive {
        DisplayPrimitive::Lines(_) | DisplayPrimitive::LineSegments(_) => "line",
        DisplayPrimitive::Conic(_) => "conic",
        DisplayPrimitive::Mesh(_) | DisplayPrimitive::MeshWithoutEdgeOverlay(_) => "mesh",
        DisplayPrimitive::MeshWithStoredEdges { .. } => "mesh",
        DisplayPrimitive::Text { .. } => "text",
        DisplayPrimitive::Image { .. } => "image",
        DisplayPrimitive::Mask { .. } => "mask",
        DisplayPrimitive::Instance { .. } => "instance",
    }
}

/// The exact statement of what happens to a primitive that crosses the band.
pub(super) fn depth_straddle_message(band: ClipDepth) -> String {
    format!(
        "the primitive crosses the XCLIP clip depth band (world z {}..{}); a primitive that \
         straddles the band is kept whole — this build filters primitives by their z extent, it \
         does not cut one at a depth plane",
        band.min, band.max
    )
}

/// Drop the runs that lie wholly outside a clip depth band.
///
/// A run is tested on its own z extent (in packed mode each independent segment
/// pair is tested separately, so a fully-outside dash disappears): a run outside
/// the band is omitted exactly, and a run that crosses the band is **kept whole**
/// and reported — this build filters by z extent, it does not cut a polyline at a
/// depth plane.
pub(super) fn filter_runs_by_depth(
    entity: &DbEntity,
    runs: Vec<Vec<Point3>>,
    band: ClipDepth,
    packed: bool,
    out: &mut DisplayRepresentation,
) -> Vec<Vec<Point3>> {
    let mut kept = Vec::new();
    for run in runs {
        if packed {
            // Independent endpoint pairs: drop a pair only when both its points
            // are outside the band, so the surviving pairs stay a pair list.
            let mut pairs: Vec<Point3> = Vec::with_capacity(run.len());
            let mut straddled = false;
            let mut unsupported = false;
            for pair in run.as_chunks::<2>().0 {
                match points_depth_span(pair) {
                    Some((min, max)) if band.excludes(min, max) => {}
                    Some((min, max)) if !band.contains(min, max) => {
                        pairs.extend_from_slice(pair);
                        straddled = true;
                    }
                    Some(_) => pairs.extend_from_slice(pair),
                    None => {
                        pairs.extend_from_slice(pair);
                        unsupported = true;
                    }
                }
            }
            if !pairs.is_empty() {
                if straddled {
                    report_xclip(
                        out,
                        entity,
                        "representation.xclip_depth_straddle",
                        &depth_straddle_message(band),
                    );
                }
                if unsupported {
                    report_xclip(
                        out,
                        entity,
                        "representation.xclip_depth_unsupported",
                        "a line segment's z extent is not finite, so the clip depth band is not \
                         applied to it and it is kept",
                    );
                }
                kept.push(pairs);
            }
        } else {
            match points_depth_span(&run) {
                Some((min, max)) if band.excludes(min, max) => {}
                Some((min, max)) if !band.contains(min, max) => {
                    report_xclip(
                        out,
                        entity,
                        "representation.xclip_depth_straddle",
                        &depth_straddle_message(band),
                    );
                    kept.push(run);
                }
                Some(_) => kept.push(run),
                None => {
                    report_xclip(
                        out,
                        entity,
                        "representation.xclip_depth_unsupported",
                        "the polyline's z extent is not finite, so the clip depth band is not \
                         applied to it and it is kept",
                    );
                    kept.push(run);
                }
            }
        }
    }
    kept
}
