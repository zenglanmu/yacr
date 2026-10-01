//! Application-side camera, projection and work-plane state (spec F13).
//!
//! This module owns the *model* of how the user observes a drawing: the
//! orthographic/perspective projection, the standard views, orbiting about a
//! target with a near-plane guard, zoom-to-cursor, and the right-handed work
//! plane that planar measurement consumes. It computes pure geometry only — no
//! GPU state and no picking — so hosts and tests can drive it without a device.
//!
//! # Conventions
//!
//! * **Handedness**: right-handed world coordinates `+X` right, `+Y` forward
//!   (plan "up"), `+Z` up. The default 2D view looks down `-Z` with screen
//!   `+right = +X` and screen `+up = +Y`.
//! * **Units**: world coordinates are in drawing units, screen coordinates are
//!   logical pixels with `y` growing **downwards** (the Slint/pointer
//!   convention). [`Camera::world_to_screen`] flips the sign accordingly.
//! * **Angles**: all angles are radians unless a function name says otherwise;
//!   a perspective field of view is the full *vertical* angle in radians.
//! * **View space**: `right`, `up`, `forward` are unit vectors with `forward`
//!   pointing from the eye toward the target. `(right, up, -forward)` is a
//!   right-handed basis (`right × up = -forward`); [`Camera::view_basis`]
//!   returns that ordered triple and guarantees orthonormality.

use cad_domain::*;

/// A projection of the camera frustum.
///
/// An orthographic projection is parameterised by world units per logical pixel
/// ([`Projection::Orthographic::scale`]); a perspective projection by a full
/// vertical field of view in radians. Both are rejected if non-finite or
/// non-positive ([`Projection::validate`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Projection {
    Orthographic { scale: f64 },
    Perspective { vertical_fov_radians: f64 },
}

/// Smallest orthographic scale we accept, in world units per logical pixel.
pub const MIN_ORTHO_SCALE: f64 = 1e-9;
/// The perspective FOV used when promoting a 2D view to 3D (full vertical angle).
pub const DEFAULT_PERSPECTIVE_FOV_RADIANS: f64 = std::f64::consts::FRAC_PI_4;
/// The orthographic scale used when a plan view has no scale to inherit.
pub const DEFAULT_ORTHO_SCALE: f64 = 1.0;
/// Smallest acceptable vertical field of view, in radians (~0.057°).
pub const MIN_FOV_RADIANS: f64 = 1e-3;
/// Largest acceptable vertical field of view, in radians (just under 180°).
pub const MAX_FOV_RADIANS: f64 = std::f64::consts::PI - 1e-3;

impl Projection {
    /// A validated orthographic projection.
    pub fn orthographic(scale: f64) -> CadResult<Self> {
        let projection = Projection::Orthographic { scale };
        projection.validate()?;
        Ok(projection)
    }

    /// A validated perspective projection from a full vertical FOV in radians.
    pub fn perspective(vertical_fov_radians: f64) -> CadResult<Self> {
        let projection = Projection::Perspective {
            vertical_fov_radians,
        };
        projection.validate()?;
        Ok(projection)
    }

    /// Whether this projection is orthographic.
    pub fn is_orthographic(&self) -> bool {
        matches!(self, Projection::Orthographic { .. })
    }

    /// Validate the projection parameters, rejecting non-finite or degenerate
    /// values rather than letting a NaN reach the matrix (audit F13).
    pub fn validate(&self) -> CadResult<()> {
        match *self {
            Projection::Orthographic { scale } => {
                if !scale.is_finite() || scale < MIN_ORTHO_SCALE {
                    return Err(CadError::InvalidInput(format!(
                        "orthographic scale must be finite and >= {MIN_ORTHO_SCALE}"
                    )));
                }
            }
            Projection::Perspective {
                vertical_fov_radians,
            } => {
                if !vertical_fov_radians.is_finite()
                    || vertical_fov_radians < MIN_FOV_RADIANS
                    || vertical_fov_radians > MAX_FOV_RADIANS
                {
                    return Err(CadError::InvalidInput(format!(
                        "vertical FOV must be in [{MIN_FOV_RADIANS}, {MAX_FOV_RADIANS}] radians"
                    )));
                }
            }
        }
        Ok(())
    }
}

/// Which of the two observation modes the viewport is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectionKind {
    TwoD,
    ThreeD,
}

/// A named standard view direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StandardView {
    Top,
    Bottom,
    Front,
    Back,
    Left,
    Right,
    Isometric,
}

impl StandardView {
    /// Unit vector from the target toward the eye for this view.
    ///
    /// This is the direction the camera looks *from*, so the eye is
    /// `target + offset * distance`. All offsets are unit vectors so a caller
    /// picks the distance independently (e.g. from the drawing bounds).
    pub fn eye_offset(self) -> Point3 {
        match self {
            // Looking straight down: screen up is world +Y.
            StandardView::Top => Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
            // Looking straight up: the "up" hint is world +Y but the basis
            // builder rotates it to keep the frame right-handed and non-
            // degenerate (a top-down view has `forward` parallel to `world_up`).
            StandardView::Bottom => Point3 {
                x: 0.0,
                y: 0.0,
                z: -1.0,
            },
            StandardView::Front => Point3 {
                x: 0.0,
                y: -1.0,
                z: 0.0,
            },
            StandardView::Back => Point3 {
                x: 0.0,
                y: 1.0,
                z: 0.0,
            },
            StandardView::Left => Point3 {
                x: -1.0,
                y: 0.0,
                z: 0.0,
            },
            StandardView::Right => Point3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
            StandardView::Isometric => {
                // Classic 45°/35.26° isometric; normalised so the offset is unit.
                let k = 1.0 / 3.0f64.sqrt();
                Point3 { x: k, y: -k, z: k }
            }
        }
    }

    /// The preferred world up hint for this view.
    pub fn up_hint(self) -> Point3 {
        match self {
            // Top/bottom look along ±Z, so world +Y is the screen-up hint.
            StandardView::Top | StandardView::Bottom => Point3 {
                x: 0.0,
                y: 1.0,
                z: 0.0,
            },
            _ => Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
        }
    }

    /// Whether this standard view is the canonical 2D plan (top) view.
    pub fn is_plan(self) -> bool {
        matches!(self, StandardView::Top)
    }
}

/// A right-handed, orthonormal camera frame.
///
/// `right`, `up` and `forward` are unit vectors; `forward` points from the eye
/// toward the target; `(right, up, -forward)` is a right-handed basis
/// (`right × up = -forward`). Screen space maps `+right` to increasing logical
/// `x` and `+up` to *decreasing* logical `y` (screen `y` grows downward).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewBasis {
    pub right: Point3,
    pub up: Point3,
    pub forward: Point3,
}

impl ViewBasis {
    /// Build a right-handed basis from a forward direction and an up hint.
    ///
    /// If the up hint is (nearly) parallel to forward — e.g. a top-down view
    /// with a world-`+Y` hint — a deterministic fallback axis is used so the
    /// basis is never degenerate (audit F13 near-plane/degenerate guard).
    pub fn from_forward_up(forward: Point3, up_hint: Point3) -> CadResult<Self> {
        let forward = normalize3(forward).ok_or_else(|| {
            CadError::InvalidInput("camera forward direction is degenerate".into())
        })?;
        let mut up_hint = normalize3(up_hint)
            .ok_or_else(|| CadError::InvalidInput("camera up hint is degenerate".into()))?;
        // Reject a hint parallel to forward; pick a stable perpendicular axis.
        if length3(cross(forward, up_hint)) < 1e-6 {
            up_hint = alternate_up(forward);
        }
        let right = normalize3(cross(forward, up_hint))
            .ok_or_else(|| CadError::InvalidInput("camera right direction is degenerate".into()))?;
        let up = normalize3(cross(right, forward))
            .ok_or_else(|| CadError::InvalidInput("camera up direction is degenerate".into()))?;
        Ok(ViewBasis { right, up, forward })
    }

    /// Whether the basis is orthonormal within `tol`.
    pub fn is_orthonormal(&self, tol: f64) -> bool {
        let unit = |v: Point3| (length3(v) - 1.0).abs() <= tol;
        let ortho = |a: Point3, b: Point3| dot3(a, b).abs() <= tol;
        unit(self.right)
            && unit(self.up)
            && unit(self.forward)
            && ortho(self.right, self.up)
            && ortho(self.right, self.forward)
            && ortho(self.up, self.forward)
    }
}

/// A stable axis perpendicular to `forward` for degenerate up hints.
fn alternate_up(forward: Point3) -> Point3 {
    // Pick the world axis least aligned with `forward` so the cross is stable.
    let ax = Point3 {
        x: 1.0,
        y: 0.0,
        z: 0.0,
    };
    let ay = Point3 {
        x: 0.0,
        y: 1.0,
        z: 0.0,
    };
    let az = Point3 {
        x: 0.0,
        y: 0.0,
        z: 1.0,
    };
    let candidates = [ax, ay, az];
    let mut best = ax;
    let mut best_abs = f64::INFINITY;
    for c in candidates {
        let a = dot3(forward, c).abs();
        if a < best_abs {
            best_abs = a;
            best = c;
        }
    }
    best
}

/// The camera of one viewport.
///
/// `eye` and `target` define the view ray; `up` is an up *hint* (it need not be
/// orthogonal to the view direction; [`Camera::view_basis`] orthonormalises it).
/// The projection is validated on construction and by every transition.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    pub eye: Point3,
    pub target: Point3,
    pub up: Point3,
    pub projection: Projection,
}

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
                let far = (self.distance() * 1e4).max(1.0);
                let near = (self.distance() * 1e-4).max(1e-6);
                perspective_matrix(vertical_fov_radians, aspect, near, far)?
            }
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
        // Pitch about the horizontal right axis (world +Z cross offset).
        let horizontal = Point3 {
            x: offset.x,
            y: offset.y,
            z: 0.0,
        };
        let right = normalize3(cross(
            horizontal,
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
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
    fn cursor_on_target_plane(&self, cursor: [f64; 2], size: [f64; 2]) -> Option<Point3> {
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

fn orthographic_matrix(half_w: f64, half_h: f64, near: f64, far: f64) -> [[f64; 4]; 4] {
    // Column-major; maps x∈[-half_w,half_w], y∈[-half_h,half_h], z∈[near,far]
    // to clip x,y∈[-1,1], z∈[0,1] (WebGPU).
    [
        [1.0 / half_w, 0.0, 0.0, 0.0],
        [0.0, 1.0 / half_h, 0.0, 0.0],
        [0.0, 0.0, 1.0 / (far - near), 0.0],
        [0.0, 0.0, -near / (far - near), 1.0],
    ]
}

fn perspective_matrix(
    vertical_fov_radians: f64,
    aspect: f64,
    near: f64,
    far: f64,
) -> CadResult<[[f64; 4]; 4]> {
    if !aspect.is_finite() || aspect <= 0.0 {
        return Err(CadError::InvalidInput(
            "perspective aspect ratio must be positive and finite".into(),
        ));
    }
    if !near.is_finite() || !far.is_finite() || near <= 0.0 || far <= near {
        return Err(CadError::InvalidInput(
            "perspective near/far must be finite with 0 < near < far".into(),
        ));
    }
    let f = 1.0 / (vertical_fov_radians * 0.5).tan();
    let nf = 1.0 / (near - far);
    Ok([
        [f / aspect, 0.0, 0.0, 0.0],
        [0.0, f, 0.0, 0.0],
        [0.0, 0.0, far * nf, -1.0],
        [0.0, 0.0, near * far * nf, 0.0],
    ])
}

/// Whether a point has only finite coordinates.
pub fn is_finite_point(p: Point3) -> bool {
    p.x.is_finite() && p.y.is_finite() && p.z.is_finite()
}

/// Whether a screen-space `[f64; 2]` is finite.
fn finite2(v: [f64; 2]) -> bool {
    v[0].is_finite() && v[1].is_finite()
}

/// A right-handed orthonormal work plane used by planar measurement.
///
/// The plane is `origin + u*a + v*b` with a **right-handed** basis: the outward
/// normal `n = normalize(u × v)` points toward the viewer in the 2D top view
/// (i.e. `+Z`). `u` and `v` are unit vectors. Planar measurement
/// (`cad-measure`) consumes this exact `WorkPlane`, so the app must supply a
/// non-degenerate, orthogonal, unit-scaled basis (audit B24).
pub fn orthonormal_work_plane(origin: Point3, u: Point3, v: Point3) -> CadResult<WorkPlane> {
    let u = normalize3(u).ok_or_else(|| {
        CadError::InvalidInput("work plane u must be a finite non-zero vector".into())
    })?;
    let v = normalize3(v).ok_or_else(|| {
        CadError::InvalidInput("work plane v must be a finite non-zero vector".into())
    })?;
    let n = cross(u, v);
    if length3(n) < 1e-9 {
        return Err(CadError::InvalidInput(
            "work plane basis is degenerate (u and v are parallel)".into(),
        ));
    }
    // Re-orthogonalise v against u so the normalised basis is exactly orthogonal.
    let n_unit = normalize3(n).expect("checked non-degenerate");
    let v_ortho = normalize3(cross(n_unit, u)).expect("cross of orthonormal pair");
    if !is_finite_point(origin) {
        return Err(CadError::InvalidInput(
            "work plane origin is not finite".into(),
        ));
    }
    Ok(WorkPlane {
        origin,
        u,
        v: v_ortho,
    })
}

/// The default XY work plane at `z = 0` with a right-handed `u = +X`, `v = +Y`.
pub fn xy_work_plane(z: f64) -> WorkPlane {
    orthonormal_work_plane(
        Point3 { x: 0.0, y: 0.0, z },
        Point3 {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        },
        Point3 {
            x: 0.0,
            y: 1.0,
            z: 0.0,
        },
    )
    .expect("the XY basis is always valid")
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

// --- small vector helpers (cad-domain Point3 doubles as a vector) ---

/// Squared-free 3D dot product.
pub fn dot3(a: Point3, b: Point3) -> f64 {
    a.x * b.x + a.y * b.y + a.z * b.z
}

/// 3D cross product.
pub fn cross(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.y * b.z - a.z * b.y,
        y: a.z * b.x - a.x * b.z,
        z: a.x * b.y - a.y * b.x,
    }
}

/// 3D Euclidean length.
pub fn length3(v: Point3) -> f64 {
    dot3(v, v).sqrt()
}

/// Normalise a vector, returning `None` for a non-finite or zero vector.
pub fn normalize3(v: Point3) -> Option<Point3> {
    let l = length3(v);
    if !l.is_finite() || l < 1e-300 {
        return None;
    }
    Some(scale3(v, 1.0 / l))
}

/// Scale a vector.
pub fn scale3(v: Point3, s: f64) -> Point3 {
    Point3 {
        x: v.x * s,
        y: v.y * s,
        z: v.z * s,
    }
}

/// Add two vectors.
pub fn add(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.x + b.x,
        y: a.y + b.y,
        z: a.z + b.z,
    }
}

/// Subtract `b` from `a`.
pub fn sub(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.x - b.x,
        y: a.y - b.y,
        z: a.z - b.z,
    }
}

fn rotate_about_z(v: Point3, angle: f64) -> Point3 {
    rotate_about_axis(
        v,
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 1.0,
        },
        angle,
    )
}

/// Rodrigues rotation of `v` about a unit `axis` by `angle` radians.
fn rotate_about_axis(v: Point3, axis: Point3, angle: f64) -> Point3 {
    let axis = match normalize3(axis) {
        Some(a) => a,
        None => return v,
    };
    let (s, c) = angle.sin_cos();
    let k = dot3(axis, v) * (1.0 - c);
    let cr = cross(axis, v);
    Point3 {
        x: v.x * c + cr.x * s + axis.x * k,
        y: v.y * c + cr.y * s + axis.y * k,
        z: v.z * c + cr.z * s + axis.z * k,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: f64, y: f64, z: f64) -> Point3 {
        Point3 { x, y, z }
    }

    #[test]
    fn projection_rejects_degenerate_fov_and_scale() {
        assert!(Projection::perspective(0.0).is_err());
        assert!(Projection::perspective(-1.0).is_err());
        assert!(Projection::perspective(f64::NAN).is_err());
        assert!(Projection::perspective(std::f64::consts::PI).is_err());
        assert!(Projection::perspective(f64::INFINITY).is_err());
        assert!(Projection::orthographic(0.0).is_err());
        assert!(Projection::orthographic(-1.0).is_err());
        assert!(Projection::orthographic(f64::NAN).is_err());

        // A valid FOV round-trips through validation.
        assert!(Projection::perspective(std::f64::consts::FRAC_PI_4).is_ok());
        assert!(Projection::orthographic(1.0).is_ok());
    }

    #[test]
    fn orthographic_projection_matrix_maps_extents_to_clip() {
        let camera = Camera {
            eye: p(0.0, 0.0, 10.0),
            target: p(0.0, 0.0, 0.0),
            up: p(0.0, 1.0, 0.0),
            projection: Projection::Orthographic { scale: 1.0 },
        };
        let m = camera.projection_matrix([800.0, 600.0]).unwrap();
        // World x = ±400 (half width in pixels * scale) maps to clip ±1.
        let clip_x = m[0][0] * 400.0;
        assert!((clip_x - 1.0).abs() < 1e-12);
        // World y = ±300 maps to clip ±1.
        let clip_y = m[1][1] * 300.0;
        assert!((clip_y - 1.0).abs() < 1e-12);
    }

    #[test]
    fn projection_matrix_rejects_degenerate_viewport_size() {
        let camera = Camera::top_view_2d();
        assert!(camera.projection_matrix([0.0, 600.0]).is_err());
        assert!(camera.projection_matrix([800.0, f64::NAN]).is_err());
    }

    #[test]
    fn standard_view_bases_are_right_handed_and_orthonormal() {
        for view in [
            StandardView::Top,
            StandardView::Bottom,
            StandardView::Front,
            StandardView::Back,
            StandardView::Left,
            StandardView::Right,
            StandardView::Isometric,
        ] {
            let offset = view.eye_offset();
            let camera = Camera {
                eye: add(p(0.0, 0.0, 0.0), scale3(offset, 1000.0)),
                target: p(0.0, 0.0, 0.0),
                up: view.up_hint(),
                projection: Projection::orthographic(1.0).unwrap(),
            };
            camera.validate().unwrap();
            let basis = camera.view_basis().unwrap();
            assert!(
                basis.is_orthonormal(1e-9),
                "view {view:?} basis is not orthonormal: {basis:?}"
            );
            // Right-handed: right × up = -forward.
            let expected = scale3(basis.forward, -1.0);
            let n = cross(basis.right, basis.up);
            assert!(
                length3(sub(n, expected)) < 1e-9,
                "view {view:?} basis is not right-handed"
            );
        }
    }

    #[test]
    fn orbit_preserves_distance_and_clamps_pitch_at_poles() {
        let mut camera = Camera {
            eye: p(0.0, 0.0, 1000.0),
            target: p(0.0, 0.0, 0.0),
            up: p(0.0, 1.0, 0.0),
            projection: Projection::perspective(std::f64::consts::FRAC_PI_4).unwrap(),
        };
        let distance = camera.distance();
        camera.orbit(0.4, 0.3).unwrap();
        assert!((camera.distance() - distance).abs() < 1e-6);
        // A huge pitch must not collapse the view; the basis stays valid.
        camera.orbit(0.0, std::f64::consts::PI).unwrap();
        let basis = camera.view_basis().unwrap();
        assert!(basis.is_orthonormal(1e-9));
        // A near-plane guard keeps the eye off the target.
        assert!(camera.distance() > 1e-9);
    }

    #[test]
    fn orbit_rejects_non_finite_angles() {
        let mut camera = Camera::top_view_2d();
        assert!(camera.orbit(f64::NAN, 0.0).is_err());
        assert!(camera.orbit(0.0, f64::INFINITY).is_err());
    }

    #[test]
    fn world_screen_round_trip_orthographic() {
        let camera = Camera {
            eye: p(0.0, 0.0, 1000.0),
            target: p(0.0, 0.0, 0.0),
            up: p(0.0, 1.0, 0.0),
            projection: Projection::Orthographic { scale: 2.0 },
        };
        let size = [800.0, 600.0];
        let world = p(120.0, -80.0, 0.0);
        let screen = camera.world_to_screen(world, size).unwrap();
        let ray = camera.screen_to_ray(screen, size).unwrap();
        // In orthographic, the ray through the pixel must pass through the world
        // point: project the world point onto the ray.
        let rel = sub(world, ray.origin);
        let t = dot3(rel, ray.direction);
        let closest = add(ray.origin, scale3(ray.direction, t));
        assert!(length3(sub(closest, world)) < 1e-9);
    }

    #[test]
    fn world_screen_round_trip_perspective() {
        let camera = Camera {
            eye: p(0.0, -500.0, 200.0),
            target: p(0.0, 0.0, 0.0),
            up: p(0.0, 0.0, 1.0),
            projection: Projection::perspective(std::f64::consts::FRAC_PI_4).unwrap(),
        };
        let size = [1024.0, 768.0];
        let world = p(50.0, 30.0, 10.0);
        let screen = camera.world_to_screen(world, size).unwrap();
        let ray = camera.screen_to_ray(screen, size).unwrap();
        // The world point lies on the ray through its own pixel.
        let rel = sub(world, ray.origin);
        let t = dot3(rel, ray.direction);
        let closest = add(ray.origin, scale3(ray.direction, t));
        assert!(length3(sub(closest, world)) < 1e-9);
    }

    #[test]
    fn world_to_screen_rejects_points_behind_a_perspective_camera() {
        let camera = Camera {
            eye: p(0.0, 0.0, 0.0),
            target: p(0.0, 1.0, 0.0),
            up: p(0.0, 0.0, 1.0),
            projection: Projection::perspective(std::f64::consts::FRAC_PI_4).unwrap(),
        };
        // A point behind the eye (negative depth) is not projectable.
        assert!(camera
            .world_to_screen(p(0.0, -10.0, 0.0), [800.0, 600.0])
            .is_none());
    }

    #[test]
    fn zoom_to_cursor_anchors_in_both_projections() {
        // Orthographic: the world point under the cursor is unchanged.
        let mut ortho = Camera {
            eye: p(0.0, 0.0, 1000.0),
            target: p(0.0, 0.0, 0.0),
            up: p(0.0, 1.0, 0.0),
            projection: Projection::Orthographic { scale: 1.0 },
        };
        let cursor = [200.0, 150.0];
        let size = [800.0, 600.0];
        let anchor = ortho.screen_to_plan_world(cursor, size).unwrap();
        ortho.zoom_at(2.0, cursor, size).unwrap();
        let after = ortho.screen_to_plan_world(cursor, size).unwrap();
        assert!(length3(sub(anchor, after)) < 1e-9);

        // Perspective: the world point under the cursor stays on the same ray.
        let mut persp = Camera {
            eye: p(0.0, -500.0, 200.0),
            target: p(0.0, 0.0, 0.0),
            up: p(0.0, 0.0, 1.0),
            projection: Projection::perspective(std::f64::consts::FRAC_PI_4).unwrap(),
        };
        let ray_before = persp.screen_to_ray(cursor, size).unwrap();
        let focus_before = persp.cursor_on_target_plane(cursor, size).unwrap();
        persp.zoom_at(1.5, cursor, size).unwrap();
        let ray_after = persp.screen_to_ray(cursor, size).unwrap();
        // Same direction and the focus point still lies on the pixel ray.
        assert!(length3(sub(ray_before.direction, ray_after.direction)) < 1e-9);
        let rel = sub(focus_before, ray_after.origin);
        let t = dot3(rel, ray_after.direction);
        let closest = add(ray_after.origin, scale3(ray_after.direction, t));
        assert!(length3(sub(closest, focus_before)) < 1e-9);
    }

    #[test]
    fn zoom_rejects_non_positive_or_non_finite_factor() {
        let mut camera = Camera::top_view_2d();
        assert!(camera.zoom_at(0.0, [0.0, 0.0], [800.0, 600.0]).is_err());
        assert!(camera.zoom_at(-2.0, [0.0, 0.0], [800.0, 600.0]).is_err());
        assert!(camera
            .zoom_at(f64::NAN, [0.0, 0.0], [800.0, 600.0])
            .is_err());
    }

    #[test]
    fn work_plane_basis_is_orthonormal_and_right_handed() {
        let plane =
            orthonormal_work_plane(p(1.0, 2.0, 3.0), p(1.0, 1.0, 0.0), p(-1.0, 1.0, 0.0)).unwrap();
        assert!((length3(plane.u) - 1.0).abs() < 1e-12);
        assert!((length3(plane.v) - 1.0).abs() < 1e-12);
        assert!(dot3(plane.u, plane.v).abs() < 1e-12);
        // Right-handed normal points +Z for an in-plane basis.
        let n = cross(plane.u, plane.v);
        assert!(n.z > 0.0);
        // A parallel basis is degenerate and rejected.
        assert!(
            orthonormal_work_plane(p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0), p(2.0, 0.0, 0.0)).is_err()
        );
    }

    #[test]
    fn camera_validate_rejects_coincident_eye_and_target() {
        let camera = Camera {
            eye: p(0.0, 0.0, 0.0),
            target: p(0.0, 0.0, 0.0),
            up: p(0.0, 1.0, 0.0),
            projection: Projection::orthographic(1.0).unwrap(),
        };
        assert!(camera.validate().is_err());
    }
}
