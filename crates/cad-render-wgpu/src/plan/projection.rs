use super::*;

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
                ..
            } => [
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
                1.0,
                0.0,
                (sx * (origin[0] - center[0])) as f32,
                (sy * (origin[1] - center[1])) as f32,
                (origin[2] - center[2]) as f32 + z_plane,
                1.0,
            ],
            // Perspective narrows the origin to `f32` *before* applying the
            // (already `f32`) view-projection, so the `f64` camera-relative
            // subtraction claimed for the Ortho arm above does not apply here.
            FrameProjection::Perspective { vp, .. } => {
                translate_left(&vp, [origin[0] as f32, origin[1] as f32, origin[2] as f32])
            }
        }
    }
}
