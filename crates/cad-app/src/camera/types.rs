use super::*;

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
    /// Every standard view in the order a UI selector presents them.
    ///
    /// A host builds its list and maps a chosen row index back through this
    /// array, exactly like `MeasurementToolKind::ALL`, so the UI ordering and
    /// the command payload cannot drift apart.
    pub const ALL: [StandardView; 7] = [
        StandardView::Top,
        StandardView::Bottom,
        StandardView::Front,
        StandardView::Back,
        StandardView::Left,
        StandardView::Right,
        StandardView::Isometric,
    ];

    /// Position of this view in [`StandardView::ALL`].
    pub fn index(self) -> usize {
        StandardView::ALL
            .iter()
            .position(|v| *v == self)
            .expect("every StandardView variant is in ALL")
    }

    /// Recover a standard view from a UI row index.
    pub fn from_index(index: i32) -> Option<StandardView> {
        usize::try_from(index)
            .ok()
            .and_then(|i| StandardView::ALL.get(i).copied())
    }

    /// Stable machine key for the i18n catalog (`view.standard.<key>`).
    pub fn key(self) -> &'static str {
        match self {
            StandardView::Top => "top",
            StandardView::Bottom => "bottom",
            StandardView::Front => "front",
            StandardView::Back => "back",
            StandardView::Left => "left",
            StandardView::Right => "right",
            StandardView::Isometric => "isometric",
        }
    }

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

/// Render-ready 2D camera parameters: the target and world units per pixel.
///
/// This is the host-independent data a renderer needs for the orthogonal plan
/// path (`cad_render_wgpu::Camera2d`). It carries no GPU type and is produced by
/// [`Camera::camera2d_params`], so the projection/observation split is validated
/// in `cad-app` before a host maps it to its renderer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera2dParams {
    /// World point at the centre of the viewport.
    pub center: Point3,
    /// World units per logical pixel (strictly positive).
    pub world_per_px: f64,
}

/// Render-ready perspective 3D camera parameters (a look-at + perspective block).
///
/// The near/far planes are derived here from the eye-target distance with the
/// exact same rule as [`Camera::projection_matrix`], so a renderer gets a
/// non-degenerate frustum rather than re-deriving (or guessing) it. Produced by
/// [`Camera::camera3d_params`], which refuses an orthographic or degenerate
/// camera; a host maps the fields straight onto its own 3D camera type.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera3dParams {
    pub eye: Point3,
    pub target: Point3,
    pub up: Point3,
    /// Full vertical field of view, in radians.
    pub fov_y: f64,
    /// Distance to the near plane (strictly positive).
    pub near: f64,
    /// Distance to the far plane (strictly greater than `near`).
    pub far: f64,
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
