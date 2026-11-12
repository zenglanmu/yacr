//! World-space rectangle clipping for block-reference (`XCLIP`) boundaries.
//!
//! The stored boundary lives in the **block definition's local space**
//! (`cad_domain::BlockClip`). [`block_clip_rect`] maps it through the composed
//! INSERT transform at expansion time and accepts it only when the transformed
//! four corners are still an **axis-aligned world-XY rectangle**. That covers
//! the 0/90/180/270 degree rotations of a rectangular boundary; any other
//! rotation, a non-XY plane or a degenerate transform is refused with a stable
//! reason so the caller can keep the geometry unclipped and report honestly
//! instead of drawing a guessed region.
//!
//! A polygon boundary is stored (the command writes and reports it) but has no
//! exact render path here, so it is refused the same way `VPCLIP` refuses an
//! N-gon clip: the geometry stays unclipped and the report names the boundary.
//!
//! This module only builds the rectangle; the actual clipping reuses the
//! existing `layout` helpers (Liang–Barsky polylines, analytic conic parameter
//! intervals, Sutherland–Hodgman meshes and image quads) — there is no second
//! clipper.

use cad_domain::*;

/// Why a `BlockClip` could not be turned into a world rectangle.
///
/// Each variant is reported by the caller with its own diagnostic code; the
/// geometry is always kept unclipped, never dropped or approximated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockClipRejection {
    /// The boundary is a polygon (three or more points), not a rectangle.
    Polygon,
    /// The transformed rectangle is rotated by something other than a multiple
    /// of 90 degrees (or sheared/mirrored into a non-axis-aligned quad).
    Rotation,
    /// The transformed boundary does not lie on a plane parallel to world XY.
    NonXyPlane,
    /// The transform is singular/non-finite, or the rectangle has no extent.
    Degenerate,
    /// `inside = false` (an inverted clip): this build refuses to render it.
    Inverted,
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
    /// The rectangle as the `(center, half-extent)` pair the shared clippers
    /// (`clip_polyline_to_rect`, `clip_conic_to_rect`, `clip_mesh_to_rect`,
    /// `clip_image_quad_to_rect`) consume.
    pub(crate) fn center_half(&self) -> ([f64; 2], [f64; 2]) {
        (
            [
                (self.min[0] + self.max[0]) * 0.5,
                (self.min[1] + self.max[1]) * 0.5,
            ],
            [
                (self.max[0] - self.min[0]) * 0.5,
                (self.max[1] - self.min[1]) * 0.5,
            ],
        )
    }

    /// Intersection of two rectangle clips, or `None` when they do not overlap
    /// (a nested `INSTANCE` clip can make the outer clip empty).
    pub(crate) fn intersect(&self, other: &WorldRect) -> Option<WorldRect> {
        let min = [self.min[0].max(other.min[0]), self.min[1].max(other.min[1])];
        let max = [self.max[0].min(other.max[0]), self.max[1].min(other.max[1])];
        if max[0] <= min[0] || max[1] <= min[1] {
            return None;
        }
        Some(WorldRect { min, max })
    }

    /// Whether the point lies inside (boundary inclusive).
    #[cfg(test)]
    pub(crate) fn contains(&self, p: [f64; 2]) -> bool {
        p[0] >= self.min[0] && p[0] <= self.max[0] && p[1] >= self.min[1] && p[1] <= self.max[1]
    }
}

/// Map `clip` (block-local) through `transform` into an axis-aligned world rect.
///
/// `Ok(None)` means the clip is switched **off** (`XCLIP OFF`): no cut applies.
/// `Err(reason)` means the boundary is real stored data this build cannot render
/// exactly; the caller keeps the geometry unclipped and reports `reason`.
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

/// The stable diagnostic code for one refused boundary.
pub(crate) fn rejection_code(rejection: BlockClipRejection) -> &'static str {
    match rejection {
        BlockClipRejection::Polygon => "representation.xclip_polygon_not_renderable",
        BlockClipRejection::Rotation => "representation.xclip_rotation_unsupported",
        BlockClipRejection::NonXyPlane => "representation.xclip_plane_unsupported",
        BlockClipRejection::Degenerate => "representation.xclip_degenerate",
        BlockClipRejection::Inverted => "representation.xclip_invert_unsupported",
    }
}

/// The human-readable half of the diagnostic for one refused boundary.
pub(crate) fn rejection_message(rejection: BlockClipRejection) -> &'static str {
    match rejection {
        BlockClipRejection::Polygon => {
            "an XCLIP polygonal boundary is stored verbatim but this build renders only \
             rectangular clips, so the reference is drawn unclipped"
        }
        BlockClipRejection::Rotation => {
            "the XCLIP rectangle maps to a rotated or sheared world quad (only 0/90/180/270 \
             degree rotations are renderable), so the reference is drawn unclipped"
        }
        BlockClipRejection::NonXyPlane => {
            "the XCLIP rectangle does not map onto a world-XY plane, so the reference is drawn \
             unclipped"
        }
        BlockClipRejection::Degenerate => {
            "the XCLIP rectangle or its INSERT transform is degenerate, so the reference is \
             drawn unclipped"
        }
        BlockClipRejection::Inverted => {
            "an inverted (XCLIP INVERT) boundary is not stored, so the reference is drawn \
             unclipped"
        }
    }
}
