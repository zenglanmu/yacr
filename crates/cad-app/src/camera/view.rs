use super::*;

impl Camera {
    /// The default 2D top view looking down `-Z` from `z = 1000`.
    pub fn top_view_2d() -> Self {
        Camera {
            eye: Point3 {
                x: 0.0,
                y: 0.0,
                z: 1000.0,
            },
            target: Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            up: Point3 {
                x: 0.0,
                y: 1.0,
                z: 0.0,
            },
            projection: Projection::Orthographic { scale: 1.0 },
        }
    }

    /// Enter the perspective 3D view, framing `bounds`.
    ///
    /// Only the projection, the target and the eye distance change; the eye
    /// *direction* is preserved, so entering 3D from the plan view stays
    /// top-down (orbiting is what tilts it).
    ///
    /// The target becomes the bounds centre, `z` included, so a solid that does
    /// not straddle `z = 0` is framed too. The eye is pushed back to
    /// `radius / tan(fov_y / 2) * FIT_3D_MARGIN`, where `radius` is half the
    /// bounding-box diagonal. The vertical extent visible at the target is then
    /// `2 * distance * tan(fov_y / 2) = 2 * radius * FIT_3D_MARGIN`, i.e. the
    /// whole model diagonal fits with breathing room, and the model stays far
    /// beyond the near plane that [`perspective_near_far`] derives from the
    /// distance (it is a tiny fraction of the distance), so a solid is no longer
    /// clipped away.
    ///
    /// Fallback rule: absent, non-finite or zero-extent bounds fall back to a
    /// radius of [`EMPTY_MODEL_RADIUS`] (one world unit), and the distance is
    /// floored at [`MIN_3D_DISTANCE`], so the result is always a finite,
    /// non-degenerate camera — even for an empty document.
    pub fn enter_3d(
        &self,
        bounds: Option<(Point3, Point3)>,
        vertical_fov_radians: f64,
    ) -> CadResult<Camera> {
        let projection = Projection::perspective(vertical_fov_radians)?;
        let (target, radius) = match bounds {
            Some((min, max)) if is_finite_point(min) && is_finite_point(max) => {
                let centre = scale3(add(min, max), 0.5);
                (centre, length3(sub(max, min)) * 0.5)
            }
            _ => (self.target, 0.0),
        };
        let radius = if radius.is_finite() && radius > 0.0 {
            radius
        } else {
            EMPTY_MODEL_RADIUS
        };
        let tan_half = (vertical_fov_radians * 0.5).tan();
        let distance = if tan_half.is_finite() && tan_half > 0.0 {
            (radius / tan_half * FIT_3D_MARGIN).max(MIN_3D_DISTANCE)
        } else {
            // Not reachable through `Projection::perspective`'s validated FOV;
            // kept so a caller cannot get a NaN distance from a bad angle.
            MIN_3D_DISTANCE
        };
        // Preserve the view direction; a coincident eye/target pair (already an
        // invalid camera) falls back to the top-down direction.
        let direction = normalize3(sub(self.eye, self.target)).unwrap_or(Point3 {
            x: 0.0,
            y: 0.0,
            z: 1.0,
        });
        let camera = Camera {
            eye: add(target, scale3(direction, distance)),
            target,
            up: self.up,
            projection,
        };
        camera.validate()?;
        Ok(camera)
    }

    /// The right-handed, orthonormal view frame of this camera.
    pub fn view_basis(&self) -> CadResult<ViewBasis> {
        ViewBasis::from_forward_up(sub(self.target, self.eye), self.up)
    }

    /// The eye-to-target distance.
    pub fn distance(&self) -> f64 {
        length3(sub(self.target, self.eye))
    }

    /// Validate the camera, including its projection and a non-degenerate view
    /// direction (audit F13).
    pub fn validate(&self) -> CadResult<()> {
        for p in [self.eye, self.target, self.up] {
            if !is_finite_point(p) {
                return Err(CadError::InvalidInput("camera point is not finite".into()));
            }
        }
        if self.distance() < 1e-9 {
            return Err(CadError::InvalidInput(
                "camera eye and target coincide".into(),
            ));
        }
        self.projection.validate()
    }

    /// The projection matrix for a viewport of logical size `size` (pixels).
    ///
    /// Returns a column-major `[[f64; 4]; 4]` mapping **view space** (right,
    /// up, -forward) to clip space with `x, y ∈ [-1, 1]` and `z ∈ [0, 1]`
    /// (WebGPU's depth range). The perspective matrix is built from a *full*
    /// vertical FOV in radians. Degenerate sizes or parameters are errors.
    pub fn projection_matrix(&self, size: [f64; 2]) -> CadResult<[[f64; 4]; 4]> {
        let (w, h) = (size[0], size[1]);
        if !w.is_finite() || !h.is_finite() || w <= 0.0 || h <= 0.0 {
            return Err(CadError::InvalidInput(
                "viewport logical size must be finite and positive".into(),
            ));
        }
        self.projection.validate()?;
        Ok(match self.projection {
            Projection::Orthographic { scale } => {
                // Half-extents in world units; scale is world units per pixel.
                let half_h = h * scale * 0.5;
                let half_w = w * scale * 0.5;
                orthographic_matrix(half_w, half_h, 0.0, 1.0)
            }
            Projection::Perspective {
                vertical_fov_radians,
            } => {
                let aspect = w / h;
                // WebGPU depth range [0, 1]; near/far derived from the camera
                // distance so the target always stays well inside the frustum.
                let (near, far) = perspective_near_far(self.distance());
                perspective_matrix(vertical_fov_radians, aspect, near, far)?
            }
        })
    }

    /// Render-ready parameters for the orthographic 2D plan path.
    ///
    /// Rejects a non-orthographic camera or a degenerate scale, so a host never
    /// renders the plan with a perspective projection (or a NaN scale) by
    /// accident.
    pub fn camera2d_params(&self) -> CadResult<Camera2dParams> {
        self.validate()?;
        let Projection::Orthographic { scale } = self.projection else {
            return Err(CadError::InvalidInput(
                "the 2D render path needs an orthographic projection".into(),
            ));
        };
        if !scale.is_finite() || scale < MIN_ORTHO_SCALE {
            return Err(CadError::InvalidInput(
                "orthographic scale is not renderable".into(),
            ));
        }
        Ok(Camera2dParams {
            center: self.target,
            world_per_px: scale,
        })
    }

    /// Render-ready parameters for the perspective 3D path.
    ///
    /// Refuses an orthographic camera (the 3D renderer only implements a
    /// perspective frustum here), a degenerate/coincident eye-target pair, a
    /// non-orthonormalisable basis, and an invalid projection. The near/far
    /// planes come from [`perspective_near_far`] — the same rule as
    /// [`Camera::projection_matrix`] — so the two paths cannot diverge.
    pub fn camera3d_params(&self) -> CadResult<Camera3dParams> {
        self.validate()?;
        let Projection::Perspective {
            vertical_fov_radians,
        } = self.projection
        else {
            return Err(CadError::InvalidInput(
                "the 3D render path needs a perspective projection".into(),
            ));
        };
        let distance = self.distance();
        if !distance.is_finite() || distance < 1e-6 {
            return Err(CadError::InvalidInput(
                "camera eye and target are too close to render in 3D".into(),
            ));
        }
        // A degenerate up hint (parallel to the view direction) must be refused
        // here as well: the renderer's look-at would have no stable basis, and
        // the app must not silently repair a camera the caller asked for.
        let forward = normalize3(sub(self.target, self.eye))
            .ok_or_else(|| CadError::InvalidInput("camera view direction is degenerate".into()))?;
        let up = normalize3(self.up)
            .ok_or_else(|| CadError::InvalidInput("camera up hint is degenerate".into()))?;
        if length3(cross(forward, up)) < 1e-6 {
            return Err(CadError::InvalidInput(
                "camera up hint is parallel to the view direction".into(),
            ));
        }
        let (near, far) = perspective_near_far(distance);
        Ok(Camera3dParams {
            eye: self.eye,
            target: self.target,
            up: self.up,
            fov_y: vertical_fov_radians,
            near,
            far,
        })
    }

    /// Map a world point to logical screen pixels for the given viewport size.
    ///
    /// Screen `x` grows right and `y` grows **down** (pointer convention). A
    /// point behind the camera (a negative view-space depth) or one that maps to
    /// a non-finite coordinate returns `None` rather than a fabricated pixel.
    pub fn world_to_screen(&self, world: Point3, size: [f64; 2]) -> Option<[f64; 2]> {
        if !is_finite_point(world) || size[0] <= 0.0 || size[1] <= 0.0 {
            return None;
        }
        let basis = self.view_basis().ok()?;
        let rel = sub(world, self.eye);
        let x = dot3(rel, basis.right);
        let y = dot3(rel, basis.up);
        // Depth along the view direction, positive in front of the camera.
        let depth = dot3(rel, basis.forward);
        let (nx, ny) = match self.projection {
            Projection::Orthographic { scale } => {
                let half_h = size[1] * scale * 0.5;
                let half_w = size[0] * scale * 0.5;
                (x / half_w, y / half_h)
            }
            Projection::Perspective {
                vertical_fov_radians,
            } => {
                if depth <= 1e-9 {
                    return None;
                }
                let t = (vertical_fov_radians * 0.5).tan();
                let half_h = depth * t;
                let half_w = half_h * (size[0] / size[1]);
                (x / half_w, y / half_h)
            }
        };
        if !nx.is_finite() || !ny.is_finite() {
            return None;
        }
        Some([
            (nx * 0.5 + 0.5) * size[0],
            // Screen y grows downward: flip the clip-space +y (up).
            (0.5 - ny * 0.5) * size[1],
        ])
    }

    /// Build a world-space pick ray through a logical screen pixel.
    ///
    /// This is the exact inverse direction of [`Camera::world_to_screen`] for
    /// both projections: the ray origin is the eye and the direction passes
    /// through the pixel's plane. It performs no depth test or scene picking
    /// (that lives in the render workstream); callers intersect it themselves.
    pub fn screen_to_ray(&self, logical: [f64; 2], size: [f64; 2]) -> Option<Ray3> {
        if !logical[0].is_finite() || !logical[1].is_finite() || size[0] <= 0.0 || size[1] <= 0.0 {
            return None;
        }
        let basis = self.view_basis().ok()?;
        // Normalised device coordinates: +x right, +y up, in [-1, 1].
        let nx = (logical[0] / size[0]) * 2.0 - 1.0;
        let ny = 1.0 - (logical[1] / size[1]) * 2.0;
        let direction = match self.projection {
            Projection::Orthographic { .. } => {
                // A parallel ray: the direction is the view direction.
                basis.forward
            }
            Projection::Perspective {
                vertical_fov_radians,
            } => {
                let t = (vertical_fov_radians * 0.5).tan();
                let half_h = t;
                let half_w = t * (size[0] / size[1]);
                add(
                    basis.forward,
                    add(
                        scale3(basis.right, nx * half_w),
                        scale3(basis.up, ny * half_h),
                    ),
                )
            }
        };
        let direction = normalize3(direction)?;
        // Orthographic rays start at the pixel's own position on the eye plane,
        // so the ray actually passes through that pixel's world column.
        let origin = match self.projection {
            Projection::Orthographic { scale } => {
                let half_h = size[1] * scale * 0.5;
                let half_w = size[0] * scale * 0.5;
                add(
                    self.eye,
                    add(
                        scale3(basis.right, nx * half_w),
                        scale3(basis.up, ny * half_h),
                    ),
                )
            }
            Projection::Perspective { .. } => self.eye,
        };
        Some(Ray3 { origin, direction })
    }

    /// Zoom by `factor` (>1 zooms in) about a screen cursor.
    ///
    /// Orthographic: multiplies the world-per-pixel scale and shifts the target
    /// so the world point under the cursor stays put. Perspective: moves the eye
    /// toward/away from the target along the view ray by the same factor so the
    /// cursor's world direction is preserved. Rejects non-positive factors.
    pub fn zoom_at(&mut self, factor: f64, cursor: [f64; 2], size: [f64; 2]) -> CadResult<()> {
        if !factor.is_finite() || factor <= 0.0 {
            return Err(CadError::InvalidInput(
                "zoom factor must be positive and finite".into(),
            ));
        }
        match self.projection {
            Projection::Orthographic { scale } => {
                let new_scale = (scale / factor).max(MIN_ORTHO_SCALE);
                // Anchor the world point under the cursor: capture it at the old
                // scale, then translate the camera so it maps back to the cursor.
                let anchor = self.screen_to_plan_world(cursor, size);
                self.projection = Projection::Orthographic { scale: new_scale };
                if let Some(anchor) = anchor {
                    if let Some(after) = self.screen_to_plan_world(cursor, size) {
                        let shift = sub(anchor, after);
                        self.target = add(self.target, shift);
                        self.eye = add(self.eye, shift);
                    }
                }
            }
            Projection::Perspective { .. } => {
                let distance = self.distance();
                if distance < 1e-9 {
                    return Err(CadError::InvalidInput(
                        "camera eye and target coincide".into(),
                    ));
                }
                // Anchor the cursor with a dolly: shift both the eye and the
                // target along the cursor ray so every pixel ray line is
                // preserved (the world point under the cursor stays put).
                let focus = self
                    .cursor_on_target_plane(cursor, size)
                    .unwrap_or(self.target);
                let to_focus = sub(focus, self.eye);
                let focus_distance = length3(to_focus);
                if !focus_distance.is_finite() || focus_distance < 1e-9 {
                    return Err(CadError::InvalidInput(
                        "zoom focus point is degenerate".into(),
                    ));
                }
                let new_focus_distance = (focus_distance / factor).max(MIN_ORTHO_SCALE);
                let dir = scale3(to_focus, 1.0 / focus_distance);
                let delta = scale3(dir, focus_distance - new_focus_distance);
                self.eye = add(self.eye, delta);
                self.target = add(self.target, delta);
            }
        }
        self.validate()
    }

    /// Orbit the eye about the target by yaw/pitch deltas (radians).
    ///
    /// Yaw rotates about the world `+Z` axis and pitch about the current
    /// horizontal (`right`) axis. The eye-target distance is preserved. Clamps
    /// pitch to avoid passing through the poles (`forward` parallel to `up`) and
    /// enforces a minimum eye-to-target distance so a near-plane guard survives
    /// the orbit (audit F13).
    pub fn orbit(&mut self, yaw: f64, pitch: f64) -> CadResult<()> {
        if !yaw.is_finite() || !pitch.is_finite() {
            return Err(CadError::InvalidInput("orbit angles must be finite".into()));
        }
        let distance = self.distance();
        if distance < 1e-9 {
            return Err(CadError::InvalidInput(
                "camera eye and target coincide".into(),
            ));
        }
        // Current offset, expressed in a Z-up world frame.
        let mut offset = sub(self.eye, self.target);
        // Yaw about world +Z.
        offset = rotate_about_z(offset, yaw);
        // Pitch about the horizontal right axis: `up × horizontal`, i.e. the
        // camera's screen right for a Z-up world.
        //
        // The axis must be frame-consistent, so its sign cannot depend on which
        // way the current offset already leans. Deriving it the other way round
        // (`horizontal × up`) evaluates to the *negative* of screen right, so
        // the first increment tilted the eye and every following one tilted it
        // back: an incremental orbit (one command per pointer move, the host's
        // real drag path) cancelled itself and left the camera a few
        // milliradians off top-down instead of the pitched view the drag asked
        // for. A top-down `[-1, 1]³` cube then rendered as its `+Z` face - a
        // rotated square with no internal edges (fenced by
        // `incremental_orbit_accumulates_the_pitch`).
        let horizontal = Point3 {
            x: offset.x,
            y: offset.y,
            z: 0.0,
        };
        let right = normalize3(cross(
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
            horizontal,
        ))
        .unwrap_or(Point3 {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        });
        offset = rotate_about_axis(offset, right, pitch);
        // Clamp the polar angle away from the poles so the basis stays valid.
        let len = length3(offset);
        let max_polar = std::f64::consts::PI - 1e-3;
        let min_polar = 1e-3;
        let polar = (offset.z / len).clamp(-1.0, 1.0).acos();
        let clamped = polar.clamp(min_polar, max_polar);
        if (clamped - polar).abs() > 0.0 {
            let sin = clamped.sin();
            let cos = clamped.cos();
            let horiz = normalize3(horizontal).unwrap_or(Point3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            });
            offset = add(
                scale3(horiz, len * sin),
                Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: len * cos,
                },
            );
        }
        // Near-plane guard: never let the orbit collapse the eye onto the target.
        let new_distance = len.max(MIN_ORTHO_SCALE);
        offset = scale3(offset, new_distance / len.max(MIN_ORTHO_SCALE));
        self.eye = add(self.target, offset);
        // Keep the up hint consistent with the new orbit.
        self.up = self.compute_up_hint();
        self.validate()
    }

    /// A stable up hint for the current eye direction, avoiding polar degeneracy.
    fn compute_up_hint(&self) -> Point3 {
        let forward = normalize3(sub(self.target, self.eye)).unwrap_or(Point3 {
            x: 0.0,
            y: 0.0,
            z: -1.0,
        });
        let z = Point3 {
            x: 0.0,
            y: 0.0,
            z: 1.0,
        };
        if length3(cross(forward, z)) < 1e-6 {
            // Looking along ±Z: use world +Y as the screen-up hint.
            Point3 {
                x: 0.0,
                y: 1.0,
                z: 0.0,
            }
        } else {
            z
        }
    }

    /// The world point under the cursor on the plane through the target that is
    /// perpendicular to the view direction. Used internally by zoom-to-cursor.
    pub(super) fn cursor_on_target_plane(
        &self,
        cursor: [f64; 2],
        size: [f64; 2],
    ) -> Option<Point3> {
        let ray = self.screen_to_ray(cursor, size)?;
        let basis = self.view_basis().ok()?;
        let denom = dot3(ray.direction, basis.forward);
        if denom.abs() < 1e-12 {
            return None;
        }
        // Plane through target with normal = forward.
        let t = dot3(sub(self.target, ray.origin), basis.forward) / denom;
        if !t.is_finite() {
            return None;
        }
        Some(add(ray.origin, scale3(ray.direction, t)))
    }
}

/// World↔screen mapping helpers that are projection-specific 2D conventions.
///
/// Kept separate from [`Camera`] so existing 2D callers (measurement picking)
/// keep working while the same camera object also serves 3D.
impl Camera {
    /// The world point on the horizontal target plane (constant `target.z`) under
    /// a screen cursor in the **top-down 2D** convention.
    ///
    /// This is the legacy 2D mapping used by measurement: it assumes the camera
    /// looks down `-Z`, so it ignores `up` and only uses the projection scale.
    /// For a general view use [`Camera::screen_to_ray`].
    pub fn screen_to_plan_world(
        &self,
        logical: [f64; 2],
        canvas_logical_size: [f64; 2],
    ) -> Option<Point3> {
        if !finite2(logical)
            || !finite2(canvas_logical_size)
            || canvas_logical_size[0] <= 0.0
            || canvas_logical_size[1] <= 0.0
        {
            return None;
        }
        if !self.projection.is_orthographic() {
            // The plan mapping is only defined for the orthographic top view.
            return None;
        }
        let scale = match self.projection {
            Projection::Orthographic { scale } => scale.max(MIN_ORTHO_SCALE),
            Projection::Perspective { .. } => return None,
        };
        let dx = (logical[0] - canvas_logical_size[0] * 0.5) * scale;
        let dy = (logical[1] - canvas_logical_size[1] * 0.5) * scale;
        Some(Point3 {
            x: self.target.x + dx,
            y: self.target.y - dy,
            z: self.target.z,
        })
    }
}
