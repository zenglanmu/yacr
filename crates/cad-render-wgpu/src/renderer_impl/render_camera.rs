use super::*;

impl Renderer {
    /// Render one frame into the offscreen target using a 2D camera.
    ///
    /// The error is explicit: callers can tell a device loss from a rejected
    /// frame via [`RenderError::is_device_loss`].
    pub fn render(
        &mut self,
        camera: Camera2d,
        target: &RenderTarget,
    ) -> Result<FrameStats, RenderError> {
        self.render_with_camera(camera, target, false)
    }

    /// Composite the transient overlay over the retained base without advancing
    /// the progressive page walk when a page is already retained.
    ///
    /// The engine uses this for cursor-only invalidation while navigation is
    /// active. A camera/target change still restarts the walk and draws page 1
    /// (the renderer decides from `cursor == 0`), so this can never show a stale
    /// camera.
    pub fn render_retained_overlay(
        &mut self,
        camera: Camera2d,
        target: &RenderTarget,
    ) -> Result<FrameStats, RenderError> {
        self.render_with_camera(camera, target, true)
    }

    /// Dispatch a navigation frame: a paused gesture uses the retained-base
    /// variant; a normal frame advances the progressive walk.
    pub fn render_navigation(
        &mut self,
        camera: Camera2d,
        target: &RenderTarget,
        retain_progressive_page: bool,
    ) -> Result<FrameStats, RenderError> {
        if retain_progressive_page {
            self.render_retained_overlay(camera, target)
        } else {
            self.render(camera, target)
        }
    }

    fn render_with_camera(
        &mut self,
        camera: Camera2d,
        target: &RenderTarget,
        retain_progressive_page: bool,
    ) -> Result<FrameStats, RenderError> {
        let w = target.width.max(1) as f64;
        let h = target.height.max(1) as f64;
        let sx = 2.0 / (w * camera.world_per_px);
        // CAD and clip space are both Y-up. wgpu's viewport maps positive
        // clip Y to the top of the image; negating here mirrors the drawing
        // and disagrees with the host's screen-to-world picking transform.
        let sy = 2.0 / (h * camera.world_per_px);
        // The depth band comes from the resident geometry's world-z extent, so a
        // solid's whole extent lands inside the [0, 1] clip range instead of being
        // clipped by a one-unit slab (see `OrthoDepth`).
        let projection = FrameProjection::Ortho {
            sx,
            sy,
            center: [camera.center.x, camera.center.y, camera.center.z],
            z_plane: camera.z_plane,
            depth: self.ortho_depth(),
            world_per_px: camera.world_per_px,
        };
        // Camera-only world→clip matrix, column-major (WGSL `mat4x4<f32>`). It
        // is the per-batch transform with the batch origin removed, so image
        // vertices (already in world space) can share the same camera uniform;
        // its z row is read back from the projection so the two can never drift.
        let (z_scale, z_offset) = projection
            .ortho_camera_z()
            .expect("an Ortho projection has a depth row");
        let camera_matrix = [
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
            z_scale,
            0.0,
            (sx * -camera.center.x) as f32,
            (sy * -camera.center.y) as f32,
            z_offset,
            1.0,
        ];
        self.render_with_projection(
            projection,
            camera_matrix,
            [
                camera.center.x as f32,
                camera.center.y as f32,
                camera.center.z as f32,
            ],
            // 2D keeps its world-Z headlight, so the diffuse term stays `n.z`.
            [0.0, 0.0, 1.0, 0.0],
            target,
            retain_progressive_page,
        )
    }

    /// World-z band covering every resident batch, mapped onto the depth range.
    ///
    /// The union is taken over the batch `z_range`s captured at upload, so the
    /// band always contains the geometry the frame can draw (a culled batch only
    /// widens it, which is conservative). An empty scene falls back to a unit
    /// band around the origin ([`OrthoDepth::default`]).
    fn ortho_depth(&self) -> OrthoDepth {
        let mut lo = f64::INFINITY;
        let mut hi = f64::NEG_INFINITY;
        for batch in &self.batches {
            lo = lo.min(f64::from(batch.z_range[0]));
            hi = hi.max(f64::from(batch.z_range[1]));
        }
        OrthoDepth::covering(lo, hi)
    }

    /// Render one frame into the offscreen target using a 3D camera.
    ///
    /// Returns [`RenderError::Frame`] (no GPU work submitted) when the camera
    /// projection is degenerate.
    pub fn render_3d(
        &mut self,
        camera: Camera3d,
        target: &RenderTarget,
    ) -> Result<FrameStats, RenderError> {
        self.render_3d_with_camera(camera, target, false)
    }

    /// 3D counterpart of [`Renderer::render_retained_overlay`].
    pub fn render_3d_retained_overlay(
        &mut self,
        camera: Camera3d,
        target: &RenderTarget,
    ) -> Result<FrameStats, RenderError> {
        self.render_3d_with_camera(camera, target, true)
    }

    /// 3D counterpart of [`Renderer::render_navigation`].
    pub fn render_3d_navigation(
        &mut self,
        camera: Camera3d,
        target: &RenderTarget,
        retain_progressive_page: bool,
    ) -> Result<FrameStats, RenderError> {
        if retain_progressive_page {
            self.render_3d_retained_overlay(camera, target)
        } else {
            self.render_3d(camera, target)
        }
    }

    fn render_3d_with_camera(
        &mut self,
        camera: Camera3d,
        target: &RenderTarget,
        retain_progressive_page: bool,
    ) -> Result<FrameStats, RenderError> {
        let aspect = target.width.max(1) as f64 / target.height.max(1) as f64;
        let Some(vp) = camera.view_projection(aspect) else {
            return Err(RenderError::Frame(
                "camera has no usable 3D projection".into(),
            ));
        };
        // Per-batch uniform = VP * T(batch origin).
        // Camera-only VP, column-major flat, for image vertices that are already
        // in world space.
        let mut camera_matrix = [0.0f32; 16];
        for col in 0..4 {
            for row in 0..4 {
                camera_matrix[col * 4 + row] = vp[col][row];
            }
        }
        let eye = [
            camera.eye.x as f32,
            camera.eye.y as f32,
            camera.eye.z as f32,
        ];
        // Camera-relative key light for 3D (see `Camera3d::headlight`): the
        // light follows the orbit but sits a fixed offset above-left of the view
        // axis, so a solid's faces (including a symmetric isometric view's
        // three) no longer all collapse to one shade. The fallback is
        // unreachable after the projection check above; it is world-`+Z` like
        // the 2D light.
        let light = camera
            .headlight()
            .map_or([0.0, 0.0, 1.0, 0.0], |[r, g, b]| [r, g, b, 0.0]);
        self.render_with_projection(
            FrameProjection::Perspective {
                vp,
                world_per_px: camera.world_per_px_at_target(target.height),
            },
            camera_matrix,
            eye,
            light,
            target,
            retain_progressive_page,
        )
    }
}
