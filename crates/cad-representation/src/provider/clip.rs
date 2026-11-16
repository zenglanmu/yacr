//! World-space clip boundaries for block-reference (`XCLIP`) boundaries.
//!
//! The stored boundary lives in the **block definition's local space**
//! (`cad_domain::BlockClip`). [`block_clip_boundary`] maps it through the
//! composed INSERT transform at expansion time and returns the world-space
//! region the reference keeps, as a [`WorldRectBoundary`]:
//!
//! * a rectangle ([`BlockClip::is_rect`]) or a stored polygon whose mapped ring
//!   is exactly an axis-aligned world-XY rectangle — **the rectangle path**, so
//!   the output is byte-for-byte the pre-polygon behaviour;
//! * a concave polygon, or a **rotated/sheared rectangle** — the ring its own
//!   corners span (a quad, never its axis-aligned envelope).
//!
//! [`block_clip_rect`] is kept unchanged as the **axis-aligned oracle**: it
//! answers "is this boundary an axis-aligned world rectangle, and which one"
//! (and refuses everything else with a stable reason), and
//! [`block_clip_boundary`] calls it first so the rectangle case cannot drift.
//!
//! Refusals are for states this build can genuinely not draw as a region, and
//! each one names itself so the caller keeps the geometry **unclipped** and
//! reports it — never a guessed, widened or partially cut region:
//!
//! * [`BlockClipRejection::Inverted`] — an inverted clip keeps the region
//!   *outside* its boundary, which is unbounded and has no region polygon;
//! * [`BlockClipRejection::NonXyPlane`] — the mapped boundary leaves a plane
//!   parallel to world XY. The shared clippers are two-dimensional (they cut in
//!   world XY and carry z through), so a tilted sheet has no region they can
//!   express; clipping it would need a 3D clip, which this build does not have;
//! * [`BlockClipRejection::Arity`], [`BlockClipRejection::Degenerate`],
//!   [`BlockClipRejection::SelfIntersecting`] — the stored ring is not a usable
//!   simple polygon (fewer than three distinct points, a repeated vertex, zero
//!   area, a singular/non-finite transform, or a ring that crosses itself).
//!
//! This module only builds the region; the clipping itself reuses the existing
//! `layout` helpers (Liang–Barsky polylines, analytic conic parameter intervals,
//! Sutherland–Hodgman / ear-clipped meshes, filled polygons and image quads) —
//! there is no second clipper.

use cad_domain::*;

use crate::layout::{viewport_reason, WorldRectBoundary};

/// Why a `BlockClip` could not be turned into a world-space clip region.
///
/// Each variant is reported by the caller with its own diagnostic code; the
/// geometry is always kept unclipped, never dropped or approximated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockClipRejection {
    /// The boundary is a polygon (three or more points), not a rectangle.
    ///
    /// **Produced only by the axis-aligned oracle** [`block_clip_rect`]: a
    /// polygon boundary has a real render path now, so
    /// [`block_clip_boundary`] never returns this and never refuses one.
    Polygon,
    /// The transformed rectangle is rotated by something other than a multiple
    /// of 90 degrees (or sheared/mirrored into a non-axis-aligned quad).
    ///
    /// **Produced only by the axis-aligned oracle** [`block_clip_rect`]: a
    /// rotated or sheared rectangle is rendered as the quad its corners span, so
    /// [`block_clip_boundary`] never returns this.
    Rotation,
    /// The transformed boundary does not lie on a plane parallel to world XY.
    NonXyPlane,
    /// The transform is singular/non-finite, or the ring has a repeated vertex
    /// or no extent.
    Degenerate,
    /// `inside = false` (an inverted clip): the kept region is the unbounded
    /// complement of the boundary, which this renderer cannot express.
    Inverted,
    /// A polygon ring with fewer than three points.
    Arity,
    /// The ring crosses or touches itself, so it has no well-defined interior.
    SelfIntersecting,
}

/// An axis-aligned world-space rectangle a block clip keeps.
///
/// The clippers take a `center`/`half` pair (the convention the paper-space
/// viewport path already uses), so [`Self::center_half`] is the only bridge.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorldRect {
    pub min: [f64; 2],
    pub max: [f64; 2],
}

impl WorldRect {
    /// Whether the point lies inside (boundary inclusive).
    #[cfg(test)]
    pub(crate) fn contains(&self, p: [f64; 2]) -> bool {
        p[0] >= self.min[0] && p[0] <= self.max[0] && p[1] >= self.min[1] && p[1] <= self.max[1]
    }
}

/// Map `clip` (block-local) through `transform` into an axis-aligned world rect.
///
/// This is the **axis-aligned oracle** the provider keeps so a rectangular clip
/// is resolved by exactly the arithmetic it always was:
///
/// * `Ok(None)` — the clip is switched **off** (`XCLIP OFF`): no cut applies;
/// * `Ok(Some(rect))` — the boundary maps to that world rectangle;
/// * `Err(Rotation)`/`Err(Polygon)` — a real stored boundary that is not an
///   axis-aligned rectangle (a rotated quad or a polygon); renderable through
///   [`block_clip_boundary`], not an error there;
/// * `Err(NonXyPlane)`/`Err(Degenerate)`/`Err(Inverted)` — genuinely
///   unrenderable as a region, refused by both entry points.
pub fn block_clip_rect(
    clip: &BlockClip,
    transform: &Transform3,
) -> Result<Option<WorldRect>, BlockClipRejection> {
    if !clip.visible {
        return Ok(None);
    }
    if !clip.inside {
        return Err(BlockClipRejection::Inverted);
    }
    if !clip.is_rect() {
        return Err(BlockClipRejection::Polygon);
    }
    let [minx, miny] = clip.vertices[0];
    let [maxx, maxy] = clip.vertices[1];
    // The transform must be a real affine map; a singular one is refused rather
    // than mapped into an invented rectangle.
    if transform.inverse().is_none() {
        return Err(BlockClipRejection::Degenerate);
    }
    let local = [[minx, miny], [maxx, miny], [maxx, maxy], [minx, maxy]];
    let mut world = [[0.0f64; 3]; 4];
    for (out, corner) in world.iter_mut().zip(local) {
        let p = transform.apply_point(Point3 {
            x: corner[0],
            y: corner[1],
            z: 0.0,
        });
        if !p.x.is_finite() || !p.y.is_finite() || !p.z.is_finite() {
            return Err(BlockClipRejection::Degenerate);
        }
        *out = [p.x, p.y, p.z];
    }
    let extent = world
        .iter()
        .fold(0.0f64, |acc, p| acc.max(p[0].abs()).max(p[1].abs()))
        .max(1.0);
    let tolerance = 1e-9 * extent;
    // Parallel to XY: every corner shares the same z.
    let z0 = world[0][2];
    if world.iter().any(|p| (p[2] - z0).abs() > tolerance) {
        return Err(BlockClipRejection::NonXyPlane);
    }
    // Axis-aligned: the x-edge is along X or Y and the y-edge along the other
    // axis, with no shear (the fourth corner is the parallelogram sum).
    let ex = [world[1][0] - world[0][0], world[1][1] - world[0][1]];
    let ey = [world[3][0] - world[0][0], world[3][1] - world[0][1]];
    let along_x = |v: [f64; 2]| v[1].abs() <= tolerance && v[0].abs() > tolerance;
    let along_y = |v: [f64; 2]| v[0].abs() <= tolerance && v[1].abs() > tolerance;
    let axis_aligned = (along_x(ex) && along_y(ey)) || (along_y(ex) && along_x(ey));
    let expected = [
        world[1][0] + world[3][0] - world[0][0],
        world[1][1] + world[3][1] - world[0][1],
    ];
    let no_shear = (world[2][0] - expected[0]).abs() <= tolerance
        && (world[2][1] - expected[1]).abs() <= tolerance;
    if !axis_aligned || !no_shear {
        return Err(BlockClipRejection::Rotation);
    }
    let mut min = [f64::INFINITY; 2];
    let mut max = [f64::NEG_INFINITY; 2];
    for p in &world {
        min[0] = min[0].min(p[0]);
        min[1] = min[1].min(p[1]);
        max[0] = max[0].max(p[0]);
        max[1] = max[1].max(p[1]);
    }
    if max[0] - min[0] <= tolerance || max[1] - min[1] <= tolerance {
        return Err(BlockClipRejection::Degenerate);
    }
    Ok(Some(WorldRect { min, max }))
}

/// Map `clip` (block-local) through `transform` into the world region it keeps.
///
/// `Ok(None)` means the clip is switched **off** (`XCLIP OFF`): no cut applies.
/// `Ok(Some(region))` is exact: a rectangle, or the ring the boundary's own
/// corners span (concave and rotated/sheared quads included).
/// `Err(reason)` is a boundary that has no region representation in this build;
/// the caller keeps the geometry unclipped and reports `reason`.
///
/// The axis-aligned oracle runs first, so an axis-aligned rectangle is resolved
/// by `block_clip_rect`'s arithmetic and the rectangle half-plane clippers then
/// produce exactly the output they produced before polygons were supported.
pub fn block_clip_boundary(
    clip: &BlockClip,
    transform: &Transform3,
) -> Result<Option<WorldRectBoundary>, BlockClipRejection> {
    match block_clip_rect(clip, transform) {
        Ok(None) => return Ok(None),
        Ok(Some(rect)) => return Ok(Some(WorldRectBoundary::rect(rect.min, rect.max))),
        // The oracle's two "not a rectangle, but a real boundary" answers go on
        // to the general path; every other answer is a genuine refusal.
        Err(BlockClipRejection::Polygon) | Err(BlockClipRejection::Rotation) => {}
        Err(rejection) => return Err(rejection),
    }
    if !clip.visible {
        return Ok(None);
    }
    if !clip.inside {
        return Err(BlockClipRejection::Inverted);
    }
    let local: Vec<[f64; 2]> = if clip.is_rect() {
        let [minx, miny] = clip.vertices[0];
        let [maxx, maxy] = clip.vertices[1];
        vec![[minx, miny], [maxx, miny], [maxx, maxy], [minx, maxy]]
    } else {
        clip.vertices.clone()
    };
    if local.len() < 3 {
        return Err(BlockClipRejection::Arity);
    }
    if transform.inverse().is_none() {
        return Err(BlockClipRejection::Degenerate);
    }
    let mut ring = Vec::with_capacity(local.len());
    let mut min = [f64::INFINITY; 3];
    let mut max = [f64::NEG_INFINITY; 3];
    for point in &local {
        let mapped = transform.apply_point(Point3 {
            x: point[0],
            y: point[1],
            z: 0.0,
        });
        if !mapped.x.is_finite() || !mapped.y.is_finite() || !mapped.z.is_finite() {
            return Err(BlockClipRejection::Degenerate);
        }
        for (axis, value) in [mapped.x, mapped.y, mapped.z].into_iter().enumerate() {
            min[axis] = min[axis].min(value);
            max[axis] = max[axis].max(value);
        }
        ring.push([mapped.x, mapped.y]);
    }
    // A clip region is two-dimensional: a boundary whose corners do not share one
    // z-plane is a tilted sheet in world space and has no world-XY region.
    let extent = (max[0] - min[0]).max(max[1] - min[1]).max(1.0);
    if max[2] - min[2] > 1e-9 * extent {
        return Err(BlockClipRejection::NonXyPlane);
    }
    match WorldRectBoundary::ring(&ring) {
        Ok(region) => Ok(Some(region)),
        Err(refusal) => Err(match refusal.code {
            viewport_reason::CLIP_ARITY => BlockClipRejection::Arity,
            viewport_reason::POLYGON_SELF_INTERSECTING => BlockClipRejection::SelfIntersecting,
            // A non-finite coordinate, a repeated vertex, zero area or a
            // non-planar ring: all "this ring has no interior".
            _ => BlockClipRejection::Degenerate,
        }),
    }
}

/// The stable diagnostic code for one refused boundary.
pub(crate) fn rejection_code(rejection: BlockClipRejection) -> &'static str {
    match rejection {
        // Retired from the render path: a polygon boundary is drawn now. The
        // code is still defined because `block_clip_rect` is public and still
        // answers with these two variants; no caller emits them as an XCLIP
        // degradation any more.
        BlockClipRejection::Polygon => "representation.xclip_polygon_not_renderable",
        BlockClipRejection::Rotation => "representation.xclip_rotation_unsupported",
        BlockClipRejection::NonXyPlane => "representation.xclip_plane_unsupported",
        BlockClipRejection::Degenerate => "representation.xclip_degenerate",
        BlockClipRejection::Inverted => "representation.xclip_invert_unsupported",
        BlockClipRejection::Arity => "representation.xclip_boundary_arity",
        BlockClipRejection::SelfIntersecting => "representation.xclip_self_intersecting",
    }
}

/// The human-readable half of the diagnostic for one refused boundary.
pub(crate) fn rejection_message(rejection: BlockClipRejection) -> &'static str {
    match rejection {
        BlockClipRejection::Polygon => {
            "an XCLIP polygonal boundary is stored verbatim; a stored polygon is rendered as a \
             clip region, so this oracle answer is not a degradation"
        }
        BlockClipRejection::Rotation => {
            "the XCLIP rectangle maps to a rotated or sheared world quad; it is rendered as the \
             quad its corners span, so this oracle answer is not a degradation"
        }
        BlockClipRejection::NonXyPlane => {
            "the XCLIP boundary does not map onto a plane parallel to world XY; the shared \
             clippers cut in world XY, so a tilted boundary has no region they can express and \
             the reference is drawn unclipped"
        }
        BlockClipRejection::Degenerate => {
            "the XCLIP boundary or its INSERT transform is degenerate (singular, non-finite, a \
             repeated vertex or no area), so the reference is drawn unclipped"
        }
        BlockClipRejection::Inverted => {
            "an inverted (XCLIP INVERT) boundary keeps the region outside it, which is unbounded \
             and has no region polygon in this build, so the reference is drawn unclipped"
        }
        BlockClipRejection::Arity => {
            "the XCLIP polygon boundary has fewer than three points, so it is not a region and the \
             reference is drawn unclipped"
        }
        BlockClipRejection::SelfIntersecting => {
            "the XCLIP boundary crosses or touches itself, so it has no well-defined interior and \
             the reference is drawn unclipped"
        }
    }
}
