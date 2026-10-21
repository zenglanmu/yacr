use super::*;

/// Camera used by the CAD viewport, in world units (2D plane view).
///
/// Kept here for the existing line path; the 3D orbit path uses [`Camera3d`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera2d {
    pub center: Point3,
    pub world_per_px: f64,
    /// World-z bias along the depth axis, in world units (positive = nearer; the
    /// 2D view looks down `-Z`). A frame's depth range is derived from the
    /// resident geometry's world-z extent, so geometry depth is otherwise
    /// absolute: panning `center.z` does not shift it.
    pub z_plane: f32,
}

impl Default for Camera2d {
    fn default() -> Self {
        Camera2d {
            center: Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            world_per_px: 1.0,
            z_plane: 0.0,
        }
    }
}

/// Horizontal key-light offset along the camera's own `right` axis, in units of
/// the camera radius (the eye-to-target distance). Negative = screen **left**.
///
/// [`KEY_LIGHT_UP`] and this constant are the deliberate **display convention**
/// of the 3D shading, not a physical light: the key sits a fixed amount
/// above-left *off* the view axis instead of exactly on it. A pure headlight
/// (`normalize(eye - target)`) makes every face of a solid equally inclined to
/// the view, so a symmetric view - above all the standard isometric view, where
/// the view axis is equally inclined to `+X`, `+Y` and `+Z` - shades all visible
/// faces identically and the solid reads as one flat silhouette. The offset is
/// constant in camera space, so it follows the orbit and every view keeps a
/// distinguishable face order. No shadows, no specular response, no energy
/// conservation and no physical light are claimed.
const KEY_LIGHT_RIGHT: f64 = -0.10;

/// Vertical key-light offset along the camera's own `up` axis, in units of the
/// camera radius: bigger than [`KEY_LIGHT_RIGHT`] so the key comes from
/// **above**-left rather than from the side, which is the CAD convention for
/// reading a solid.
const KEY_LIGHT_UP: f64 = 0.35;

/// A 3D orbit camera: an eye, a target and an up vector in world space.
///
/// Spec §5.2/§3.3 F13/F14: the 3D view needs a real projection before standard
/// views, orbit and 2D/3D switching are meaningful. This is an explicit
/// right-handed look-at + perspective projection; no hidden-line removal is
/// claimed (F14: "非完整隐藏线算法").
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera3d {
    pub eye: Point3,
    pub target: Point3,
    pub up: Point3,
    /// Vertical field of view in radians.
    pub fov_y: f64,
    /// Distance to the near plane, in world units.
    pub near: f64,
    /// Distance to the far plane, in world units.
    pub far: f64,
}

impl Camera3d {
    /// A camera looking straight down world `-Z` (the CAD plan orientation).
    pub fn plan_view(eye: Point3, target: Point3) -> Self {
        Camera3d {
            eye,
            target,
            up: Point3 {
                x: 0.0,
                y: 1.0,
                z: 0.0,
            },
            fov_y: std::f64::consts::FRAC_PI_4,
            near: 1.0,
            far: 1.0e7,
        }
    }

    /// Right-handed view-projection matrix, column-major (WGSL `mat4x4<f32>`).
    ///
    /// Returns `None` when the configuration is degenerate (eye == target, up
    /// parallel to the view direction, or a non-positive near/far span) rather
    /// than producing NaNs the GPU would render as garbage.
    pub fn view_projection(&self, aspect: f64) -> Option<[[f32; 4]; 4]> {
        if !self.is_usable(aspect) {
            return None;
        }
        let view = self.look_at()?;
        let proj = self.perspective(aspect);
        Some(mat4_mul(&proj, &view))
    }

    /// Unit vector **toward** the 3D key light for this camera, in world space:
    ///
    /// ```text
    /// toward_eye = normalize(eye - target)
    /// right      = normalize(forward x up)      // forward = -toward_eye
    /// up         = right x forward
    /// light      = normalize(toward_eye + KEY_LIGHT_RIGHT * right
    ///                                    + KEY_LIGHT_UP * up)
    /// ```
    ///
    /// The basis is derived exactly as [`Camera3d::look_at`] builds the view
    /// matrix, so the offset is fixed *relative to the screen*: the key sits
    /// above-left of the line of sight at every orbit angle. That is the display
    /// convention described on [`KEY_LIGHT_RIGHT`] - a camera-following key, not
    /// a headlight and not a physical source - and it is what keeps a symmetric
    /// (e.g. isometric) view from shading all of its faces identically.
    ///
    /// `None` for a degenerate camera (eye == target), which
    /// [`Camera3d::view_projection`] already rejects, so the renderer only
    /// normalises a usable camera. An up hint parallel to the line of sight has
    /// no stable basis (also rejected by the projection); the light then falls
    /// back to the pure view direction instead of inventing an axis.
    pub fn headlight(&self) -> Option<[f32; 3]> {
        let span = sub(self.eye, self.target);
        let len = length(span);
        // `!(len > 1e-12)` written positively: a non-finite span has no
        // direction either (it is `None` exactly as before).
        if !len.is_finite() || len <= 1e-12 {
            return None;
        }
        let toward_eye = Point3 {
            x: span.x / len,
            y: span.y / len,
            z: span.z / len,
        };
        let light = normalize(self.key_light_direction(toward_eye));
        Some([light.x as f32, light.y as f32, light.z as f32])
    }

    /// The world-space key direction for a unit `toward_eye` (the reversed view
    /// direction), offset by [`KEY_LIGHT_RIGHT`]/[`KEY_LIGHT_UP`] along the
    /// camera basis. Falls back to `toward_eye` when the up hint leaves no
    /// stable basis.
    fn key_light_direction(&self, toward_eye: Point3) -> Point3 {
        let forward = Point3 {
            x: -toward_eye.x,
            y: -toward_eye.y,
            z: -toward_eye.z,
        };
        let side = cross(forward, self.up);
        let side_len = length(side);
        if side_len < 1e-12 {
            return toward_eye;
        }
        let right = Point3 {
            x: side.x / side_len,
            y: side.y / side_len,
            z: side.z / side_len,
        };
        // `forward` is unit and perpendicular to `right`, so this is unit too.
        let up = cross(right, forward);
        Point3 {
            x: toward_eye.x + KEY_LIGHT_RIGHT * right.x + KEY_LIGHT_UP * up.x,
            y: toward_eye.y + KEY_LIGHT_RIGHT * right.y + KEY_LIGHT_UP * up.y,
            z: toward_eye.z + KEY_LIGHT_RIGHT * right.z + KEY_LIGHT_UP * up.z,
        }
    }

    /// Whether the camera has a usable projection for the given aspect.
    pub fn is_usable(&self, aspect: f64) -> bool {
        if !point_is_finite(self.eye) || !point_is_finite(self.target) || !point_is_finite(self.up)
        {
            return false;
        }
        if !aspect.is_finite() || aspect <= 0.0 {
            return false;
        }
        if !self.fov_y.is_finite() || self.fov_y <= 0.0 || self.fov_y >= std::f64::consts::PI {
            return false;
        }
        if !self.near.is_finite()
            || !self.far.is_finite()
            || self.near <= 0.0
            || self.far <= self.near
        {
            return false;
        }
        let forward = normalize(sub(self.target, self.eye));
        if length(forward) < 1e-12 {
            return false;
        }
        let side = cross(forward, self.up);
        if length(side) < 1e-12 {
            // Up is parallel to the view direction: no stable basis.
            return false;
        }
        true
    }

    /// Approximate world units per pixel at the camera **target** plane for a
    /// viewport `viewport_height` pixels tall.
    ///
    /// Used only to floor the SDF conic quad margin so a small on-screen circle's
    /// anti-aliased fringe is not clipped on the 3D path. A perspective view has
    /// no single world-per-pixel; the target-plane value is the honest estimate
    /// (the same depth the view is framed around). Degenerate cameras fall back
    /// to `0.0`, which disables the floor.
    pub fn world_per_px_at_target(&self, viewport_height: u32) -> f64 {
        let height = viewport_height.max(1) as f64;
        let distance = length(sub(self.target, self.eye));
        let span = 2.0 * (self.fov_y * 0.5).tan() * distance;
        if span.is_finite() && span > 0.0 {
            span / height
        } else {
            0.0
        }
    }

    pub(super) fn look_at(&self) -> Option<[[f64; 4]; 4]> {
        let forward = normalize(sub(self.target, self.eye));
        if length(forward) < 1e-12 {
            return None;
        }
        let side = normalize(cross(forward, self.up));
        if length(side) < 1e-12 {
            return None;
        }
        let up = cross(side, forward);
        // Right-handed, looking down -Z in view space (wgpu convention).
        let z = Point3 {
            x: -forward.x,
            y: -forward.y,
            z: -forward.z,
        };
        Some([
            [side.x, up.x, z.x, 0.0],
            [side.y, up.y, z.y, 0.0],
            [side.z, up.z, z.z, 0.0],
            [
                -dot(side, self.eye),
                -dot(up, self.eye),
                -dot(z, self.eye),
                1.0,
            ],
        ])
    }

    pub(super) fn perspective(&self, aspect: f64) -> [[f64; 4]; 4] {
        let f = 1.0 / (self.fov_y / 2.0).tan();
        let nf = 1.0 / (self.near - self.far);
        // wgpu clip space: z in [0, 1].
        [
            [f / aspect, 0.0, 0.0, 0.0],
            [0.0, f, 0.0, 0.0],
            [0.0, 0.0, self.far * nf, -1.0],
            [0.0, 0.0, self.near * self.far * nf, 0.0],
        ]
    }
}

/// Accumulate a frame's vertex/triangle/byte usage against a budget.
///
/// Returns the list of batches that fit and a single explicit over-budget
/// reason, or `None`, when a limit was crossed. The offending batch and every
/// batch after it are *not* silently dropped: the caller receives the reason and
/// can surface it. The counts are the same ones the GPU will submit, and the
/// byte charge is [`RenderBatch::upload_size_bytes`] — the exact packed size —
/// so [`FrameBudget::upload_bytes_per_frame`] cannot be bypassed by a small
/// vertex/triangle count.
pub fn plan_frame<'a>(
    batches: &'a [RenderBatch],
    budget: &FrameBudget,
) -> (Vec<&'a RenderBatch>, FrameUsage, Option<OverBudget>) {
    let mut usage = FrameUsage::default();
    let mut accepted = Vec::with_capacity(batches.len());
    for batch in batches {
        let vertices = if batch.topology == RenderTopology::Conics {
            batch.conics.len() * 6
        } else {
            batch.vertices.len()
        };
        let triangles = match batch.topology {
            RenderTopology::Conics => batch.conics.len() * 2,
            RenderTopology::Mesh => batch.triangle_count(),
            _ => 0,
        };
        let bytes = batch.upload_size_bytes();
        // Charge bytes first so a batch that crosses several categories reports
        // the size dimension consistently; vertices/triangles follow.
        let outcome = budget
            .charge_bytes(&mut usage, bytes)
            .and_then(|()| budget.charge(&mut usage, vertices, triangles));
        match outcome {
            Ok(()) => accepted.push(batch),
            Err(exceeded) => {
                let skipped = batches.len() - accepted.len();
                return (
                    accepted,
                    usage,
                    Some(OverBudget {
                        category: exceeded.category,
                        requested: exceeded.requested,
                        limit: exceeded.limit,
                        skipped_batches: skipped,
                    }),
                );
            }
        }
    }
    (accepted, usage, None)
}
