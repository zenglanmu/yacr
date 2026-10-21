use super::*;

/// Depth the nearest end of an orthographic frame's world-z band maps to.
///
/// Deliberately inside `(0, 1)`: a fragment that lands exactly on a clip plane
/// (0 = near, 1 = far) can be clipped or -- for the mesh pass, which compares
/// with `Less` against a depth buffer cleared to 1.0 -- silently rejected.
pub(crate) const ORTHO_DEPTH_NEAR: f64 = 0.05;

/// Depth the farthest end of an orthographic frame's world-z band maps to.
pub(crate) const ORTHO_DEPTH_FAR: f64 = 0.95;

/// Smallest world-z span an orthographic frame maps onto the depth range.
///
/// A flat drawing has no z extent at all; giving it a unit-deep band keeps every
/// coplanar fragment at the *same* interior depth (exactly as before), so ties
/// between coplanar fills, lines and overlays are decided by draw order alone.
const MIN_ORTHO_DEPTH_SPAN: f64 = 1.0;

/// Relative headroom added to each end of a non-flat z band, so no face of a
/// model sits exactly on a clip plane or on the cleared far value.
const ORTHO_DEPTH_PADDING: f64 = 0.01;

/// The world-z band an orthographic frame maps onto the WebGPU depth range.
///
/// The 2D camera looks **down `-Z`** (the world convention fixed by
/// `docs/view-3d.md`), and the renderer keeps wall-clock front faces (outward
/// CCW: `front_face = Ccw`, back faces culled), so a **larger world z is
/// nearer** and must map to a **smaller depth**. Mapping world z one-to-one onto
/// clip z (the pre-fix behaviour) had two defects for anything with real depth
/// extent:
///
/// * WebGPU clip space is `0 <= z <= w`, so a one-to-one map only contained the
///   1-unit slab `[0, 1]` above the drawing plane: everything below the plane,
///   and anything above it by more than one unit, was clipped away;
/// * the visible `+Z` face of a solid spanning `[-1, 1]` landed at clip z = 1,
///   i.e. exactly at the cleared far value, where the mesh pass's `Less` compare
///   rejects every fill fragment. The `LessEqual` line overlay of the same
///   triangles kept drawing, which is why a tessellated solid looked like its
///   own wireframe.
///
/// The band therefore comes from the resident geometry (`z_range` per batch) and
/// is padded strictly inside `(0, 1)`, so every surface is rasterized with a
/// depth the mesh pipeline can beat and no surface is clipped by the mapping.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct OrthoDepth {
    /// World z that maps to [`ORTHO_DEPTH_NEAR`] (the highest z of the band).
    pub(crate) near_z: f64,
    /// World z that maps to [`ORTHO_DEPTH_FAR`] (the lowest z of the band).
    pub(crate) far_z: f64,
}

impl Default for OrthoDepth {
    /// A unit-deep band around the origin, used by fixtures that build a
    /// projection directly instead of through `Renderer::render`.
    fn default() -> Self {
        OrthoDepth::covering(0.0, 0.0)
    }
}

impl OrthoDepth {
    /// The band covering the resident geometry's world-z range `[z_min, z_max]`.
    ///
    /// A flat range (or a non-finite one) becomes [`MIN_ORTHO_DEPTH_SPAN`] deep
    /// around its midpoint, and a real range is padded by
    /// [`ORTHO_DEPTH_PADDING`] on both ends, so no surface lies exactly on a
    /// clip plane or on the cleared far value.
    pub(crate) fn covering(z_min: f64, z_max: f64) -> Self {
        let (mut lo, mut hi) = if z_min.is_finite() && z_max.is_finite() && z_min <= z_max {
            (z_min, z_max)
        } else {
            (0.0, 0.0)
        };
        let span = hi - lo;
        if span < MIN_ORTHO_DEPTH_SPAN {
            let mid = lo + span * 0.5;
            lo = mid - MIN_ORTHO_DEPTH_SPAN * 0.5;
            hi = mid + MIN_ORTHO_DEPTH_SPAN * 0.5;
        } else {
            let pad = span * ORTHO_DEPTH_PADDING;
            lo -= pad;
            hi += pad;
        }
        OrthoDepth {
            near_z: hi,
            far_z: lo,
        }
    }

    /// `(scale, offset)` of the affine map `depth = scale·z + offset`.
    ///
    /// Linear on purpose: the accelerated (pulled) pipelines rebuild the same
    /// transform from one shared origin-independent column plus a per-chunk
    /// translation, so a non-linear depth map could not be expressed there.
    /// `scale` is negative because a larger world z is nearer (a smaller depth).
    pub(crate) fn scale_offset(self) -> (f64, f64) {
        let span = self.near_z - self.far_z;
        if !(span.is_finite() && span > 0.0) {
            return (0.0, 0.5);
        }
        let scale = -(ORTHO_DEPTH_FAR - ORTHO_DEPTH_NEAR) / span;
        let offset = ORTHO_DEPTH_NEAR + (ORTHO_DEPTH_FAR - ORTHO_DEPTH_NEAR) * self.near_z / span;
        (scale, offset)
    }

    /// Clip `z` (depth) for world z `z`, with `z_plane` applied as a world-space
    /// bias toward the viewer (positive = nearer).
    pub(crate) fn clip_z(self, z: f64, z_plane: f32) -> f64 {
        let (scale, offset) = self.scale_offset();
        scale * (z + f64::from(z_plane)) + offset
    }
}

/// Per-batch world→clip projection for one frame.
///
/// The 2D variant reproduces, bit for bit, the f64 camera-centre subtraction the
/// large-coordinate precision test fences before narrowing to `f32`; the 3D
/// variant multiplies the view-projection by the batch origin (matching the
/// previous `translate_left(&vp, origin)`). Building the matrix inline per
/// accepted batch avoids materialising and cloning a full `Vec<[f32; 16]>` every
/// frame.
#[derive(Clone, Copy)]
pub(crate) enum FrameProjection {
    Ortho {
        sx: f64,
        sy: f64,
        /// Camera centre, kept in f64 so the subtraction stays precise.
        center: [f64; 3],
        z_plane: f32,
        /// World-z band mapped onto the depth range (see [`OrthoDepth`]).
        depth: OrthoDepth,
        /// World units per pixel, passed to the SDF conic shader so its quad
        /// margin can be floored at ~2 px (see `shaders/arc.wgsl`).
        world_per_px: f64,
    },
    Perspective {
        vp: [[f32; 4]; 4],
        /// Approximate world units per pixel at the camera target; the SDF
        /// conic margin floor uses it so a small on-screen circle's fringe is
        /// not clipped on the 3D path either.
        world_per_px: f64,
    },
}

impl FrameProjection {
    /// World units per pixel, for the SDF conic quad-margin floor.
    pub(crate) fn world_per_px(self) -> f64 {
        match self {
            FrameProjection::Ortho { world_per_px, .. }
            | FrameProjection::Perspective { world_per_px, .. } => world_per_px,
        }
    }

    pub(crate) fn batch_matrix(self, origin: [f32; 3]) -> [f32; 16] {
        self.batch_matrix_f64([
            f64::from(origin[0]),
            f64::from(origin[1]),
            f64::from(origin[2]),
        ])
    }

    /// Per-batch matrix for an `f64` origin.
    ///
    /// The **Ortho** variant keeps the origin and the camera centre in `f64`
    /// through the subtraction before narrowing — the precision contract fenced
    /// by `large_coordinate_batch_keeps_precision`. The instanced line path
    /// passes its stream's exact `f64` fixed-point origin here, so stream
    /// positions stay small even at large drawing coordinates. The **Perspective**
    /// variant narrows the origin to `f32` before `translate_left`, so this `f64`
    /// camera-relative subtraction does not apply on the 3D path.
    pub(crate) fn batch_matrix_f64(self, origin: [f64; 3]) -> [f32; 16] {
        match self {
            FrameProjection::Ortho {
                sx,
                sy,
                center,
                z_plane,
                depth,
                ..
            } => {
                let (z_scale, _) = depth.scale_offset();
                [
                    sx as f32,
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                    sy as f32,
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                    // The z row is the depth map's affine scale; it does not depend
                    // on the origin, so the pulled camera uniform can carry it.
                    z_scale as f32,
                    0.0,
                    (sx * (origin[0] - center[0])) as f32,
                    (sy * (origin[1] - center[1])) as f32,
                    depth.clip_z(origin[2], z_plane) as f32,
                    1.0,
                ]
            }
            // Perspective narrows the origin to `f32` *before* applying the
            // (already `f32`) view-projection, so the `f64` camera-relative
            // subtraction claimed for the Ortho arm above does not apply here.
            FrameProjection::Perspective { vp, .. } => {
                translate_left(&vp, [origin[0] as f32, origin[1] as f32, origin[2] as f32])
            }
        }
    }

    /// The z row's `(scale, offset)` for the orthographic depth map, so the
    /// camera-only matrix used by images agrees with [`Self::batch_matrix_f64`].
    ///
    /// `offset` is the depth of world z = 0 after the `z_plane` bias.
    pub(crate) fn ortho_camera_z(self) -> Option<(f32, f32)> {
        match self {
            FrameProjection::Ortho { z_plane, depth, .. } => {
                let (scale, _) = depth.scale_offset();
                Some((scale as f32, depth.clip_z(0.0, z_plane) as f32))
            }
            FrameProjection::Perspective { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A solid spanning `[-1, 1]` in z maps entirely inside the clip range: no
    /// surface is clipped by the mapping and none lands on the cleared far value
    /// (1.0), where the mesh pass's `Less` compare would reject it.
    #[test]
    fn solid_z_extent_lands_inside_the_ortho_depth_range() {
        let depth = OrthoDepth::covering(-1.0, 1.0);
        assert!(
            depth.near_z > 1.0 && depth.far_z < -1.0,
            "the band must contain the geometry: {depth:?}"
        );
        let top = depth.clip_z(1.0, 0.0);
        let bottom = depth.clip_z(-1.0, 0.0);
        for value in [top, bottom] {
            assert!(
                value > ORTHO_DEPTH_NEAR && value < ORTHO_DEPTH_FAR,
                "depth {value} must be strictly inside the clip range"
            );
        }
        assert!(
            top < bottom,
            "the camera looks down -Z, so the +Z face is nearest and must have the smaller depth"
        );
        assert!(top < 1.0, "the visible face must not sit on the far plane");
    }

    /// A flat drawing has no z extent: every fragment must keep the *same*
    /// depth, so coplanar fills, lines and overlays still tie exactly.
    #[test]
    fn flat_scene_keeps_coplanar_ties() {
        let depth = OrthoDepth::covering(0.0, 0.0);
        let a = depth.clip_z(0.0, 0.0);
        let b = depth.clip_z(0.0, 0.0);
        assert_eq!(a, b);
        assert!(a > 0.0 && a < 1.0, "coplanar content is at interior depth");
        // An empty scene falls back to the same unit band around the origin.
        let empty = OrthoDepth::covering(f64::INFINITY, f64::NEG_INFINITY);
        assert_eq!(empty, OrthoDepth::default());
    }

    /// The affine map the pulled pipelines rebuild must match the per-batch
    /// matrix exactly, including at a large world z.
    #[test]
    fn camera_z_row_matches_the_batch_matrix() {
        let projection = FrameProjection::Ortho {
            sx: 0.5,
            sy: 0.5,
            center: [0.0, 0.0, 0.0],
            z_plane: 0.0,
            depth: OrthoDepth::covering(-1.0, 1.0),
            world_per_px: 2.0,
        };
        let (scale, offset) = projection.ortho_camera_z().expect("ortho");
        let camera = projection.batch_matrix_f64([0.0, 0.0, 0.0]);
        assert_eq!(camera[10], scale);
        assert_eq!(camera[14], offset);
        let origin = [0.0, 0.0, 0.75];
        let batch = projection.batch_matrix_f64(origin);
        assert_eq!(batch[10], scale);
        let expected = scale * origin[2] as f32 + offset;
        assert!((batch[14] - expected).abs() < 1e-6);
        let direct = projection
            .ortho_camera_z()
            .map(|(s, o)| s * origin[2] as f32 + o)
            .unwrap();
        assert!((batch[14] - direct).abs() < 1e-6);
    }
}
