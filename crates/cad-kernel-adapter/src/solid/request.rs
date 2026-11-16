//! The typed operations that cross the solid modelling seam, plus the pre-flight
//! validation every implementation must run before any kernel work.
//!
//! These types are plain data. Primitives and operands are borrowed from the
//! caller's [`BrepData`]; nothing here owns a kernel handle and nothing here
//! tessellates (the display mesh is built by `cad-kernel-adapter`'s existing
//! [`crate::BrepTessellator`] outside the modeler, ADR-0036 P1 design).

use cad_domain::{BrepData, Point3, TaskStamp, Transform3};

use crate::codes;

/// Which primitive family a [`PrimitiveSpec`] is, for capability probes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrimitiveKind {
    /// `BOX`: an axis-aligned cuboid.
    Box,
    /// `WEDGE`: a right triangular prism.
    Wedge,
    /// `PYRAMID`: a rectangular base tapering to a point.
    Pyramid,
    /// `CYLINDER`.
    Cylinder,
    /// `SPHERE`.
    Sphere,
    /// `CONE` (full cone, or a frustum when both radii are positive).
    Cone,
    /// `TORUS`.
    Torus,
}

impl PrimitiveKind {
    /// Stable tag for diagnostics and capability reporting (never localised).
    pub fn as_str(self) -> &'static str {
        match self {
            PrimitiveKind::Box => "box",
            PrimitiveKind::Wedge => "wedge",
            PrimitiveKind::Pyramid => "pyramid",
            PrimitiveKind::Cylinder => "cylinder",
            PrimitiveKind::Sphere => "sphere",
            PrimitiveKind::Cone => "cone",
            PrimitiveKind::Torus => "torus",
        }
    }
}

/// The primitive bodies a modeler can create from dimensions alone.
///
/// All lengths are model-space (drawing units); no unit conversion happens in
/// the seam (`cad-db` owns the unit context). `origin` is the primitive's
/// local anchor exactly as the corresponding command supplies it: the corner of
/// a box/wedge, the base centre of a pyramid/cone/cylinder, the centre of a
/// sphere/torus.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PrimitiveSpec {
    Box {
        origin: Point3,
        length: f64,
        width: f64,
        height: f64,
    },
    Wedge {
        origin: Point3,
        length: f64,
        width: f64,
        height: f64,
    },
    Pyramid {
        base_center: Point3,
        length: f64,
        height: f64,
    },
    Cylinder {
        origin: Point3,
        radius: f64,
        height: f64,
    },
    Sphere {
        origin: Point3,
        radius: f64,
    },
    /// A (possibly truncated) circular cone. `radius_top == 0.0` is a full cone
    /// whose apex is `height` above `origin`; both radii are never zero.
    Cone {
        origin: Point3,
        radius_bottom: f64,
        radius_top: f64,
        height: f64,
    },
    Torus {
        origin: Point3,
        major_radius: f64,
        minor_radius: f64,
    },
}

impl PrimitiveSpec {
    pub fn kind(&self) -> PrimitiveKind {
        match self {
            PrimitiveSpec::Box { .. } => PrimitiveKind::Box,
            PrimitiveSpec::Wedge { .. } => PrimitiveKind::Wedge,
            PrimitiveSpec::Pyramid { .. } => PrimitiveKind::Pyramid,
            PrimitiveSpec::Cylinder { .. } => PrimitiveKind::Cylinder,
            PrimitiveSpec::Sphere { .. } => PrimitiveKind::Sphere,
            PrimitiveSpec::Cone { .. } => PrimitiveKind::Cone,
            PrimitiveSpec::Torus { .. } => PrimitiveKind::Torus,
        }
    }

    /// Reject non-finite or out-of-range dimensions and non-finite anchors.
    ///
    /// Every length/radius must be finite and strictly positive (`Cone::radius_top`
    /// may be exactly zero for an apex). A non-finite origin/base centre is a bad
    /// input, not an unsupported operation. Degenerate-but-finite combinations
    /// that a kernel must judge (for example a torus whose minor radius exceeds
    /// its major radius) are left to the kernel: it reports
    /// [`crate::SolidOutcome::Failed`] rather than this validator inventing a rule.
    pub fn validate(&self) -> Result<(), SolidInputError> {
        match *self {
            PrimitiveSpec::Box {
                origin,
                length,
                width,
                height,
            }
            | PrimitiveSpec::Wedge {
                origin,
                length,
                width,
                height,
            } => {
                require_finite_point("origin", origin)?;
                require_positive("length", length)?;
                require_positive("width", width)?;
                require_positive("height", height)
            }
            PrimitiveSpec::Pyramid {
                base_center,
                length,
                height,
            } => {
                require_finite_point("base_center", base_center)?;
                require_positive("length", length)?;
                require_positive("height", height)
            }
            PrimitiveSpec::Cylinder {
                origin,
                radius,
                height,
            } => {
                require_finite_point("origin", origin)?;
                require_positive("radius", radius)?;
                require_positive("height", height)
            }
            PrimitiveSpec::Sphere { origin, radius } => {
                require_finite_point("origin", origin)?;
                require_positive("radius", radius)
            }
            PrimitiveSpec::Cone {
                origin,
                radius_bottom,
                radius_top,
                height,
            } => {
                require_finite_point("origin", origin)?;
                require_positive("radius_bottom", radius_bottom)?;
                if !radius_top.is_finite() || radius_top < 0.0 {
                    return Err(SolidInputError {
                        code: codes::SOLID_INVALID_DIMENSION,
                        message: format!(
                            "radius_top must be finite and non-negative, got {radius_top}"
                        ),
                    });
                }
                require_positive("height", height)
            }
            PrimitiveSpec::Torus {
                origin,
                major_radius,
                minor_radius,
            } => {
                require_finite_point("origin", origin)?;
                require_positive("major_radius", major_radius)?;
                require_positive("minor_radius", minor_radius)
            }
        }
    }
}

/// The boolean operation to apply to two closed bodies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BooleanOp {
    /// `UNION`.
    Union,
    /// `SUBTRACT`: `left` minus `right`.
    Subtract,
    /// `INTERSECT`.
    Intersect,
}

impl BooleanOp {
    /// Stable tag for diagnostics (never localised).
    pub fn as_str(self) -> &'static str {
        match self {
            BooleanOp::Union => "union",
            BooleanOp::Subtract => "subtract",
            BooleanOp::Intersect => "intersect",
        }
    }
}

/// The operation family, independent of its operands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SolidOperationFamily {
    Primitive,
    Boolean,
    Transform,
    MassProperties,
}

impl SolidOperationFamily {
    /// Stable tag for diagnostics and [`crate::SolidUnsupportedReason`].
    pub fn as_str(self) -> &'static str {
        match self {
            SolidOperationFamily::Primitive => "primitive",
            SolidOperationFamily::Boolean => "boolean",
            SolidOperationFamily::Transform => "transform",
            SolidOperationFamily::MassProperties => "mass_properties",
        }
    }
}

impl std::fmt::Display for SolidOperationFamily {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One modelling operation.
///
/// Operands are borrowed: the modeler reads a snapshot and returns a new body,
/// it never mutates the caller's B-rep and never keeps a handle after the call
/// (ADR-0036 D1).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SolidOperation<'a> {
    /// Create a primitive body.
    Primitive { spec: PrimitiveSpec },
    /// Combine two closed bodies.
    Boolean {
        op: BooleanOp,
        left: &'a BrepData,
        right: &'a BrepData,
    },
    /// Map a body through a transform.
    ///
    /// Only a **similarity** (uniform scale + rotation/reflection + translation)
    /// can be folded into [`cad_domain::BrepPlacement`]; a non-similarity is a
    /// capability refusal (`SolidOutcome::Unsupported`), never a silently
    /// non-uniform bake (ADR-0036 D4).
    Transform {
        brep: &'a BrepData,
        transform: &'a Transform3,
    },
    /// Compute mass properties of a body for the given density.
    MassProperties { brep: &'a BrepData, density: f64 },
}

impl SolidOperation<'_> {
    pub fn family(&self) -> SolidOperationFamily {
        match self {
            SolidOperation::Primitive { .. } => SolidOperationFamily::Primitive,
            SolidOperation::Boolean { .. } => SolidOperationFamily::Boolean,
            SolidOperation::Transform { .. } => SolidOperationFamily::Transform,
            SolidOperation::MassProperties { .. } => SolidOperationFamily::MassProperties,
        }
    }

    /// The primitive kind, when this is a `Primitive` operation.
    pub fn primitive_kind(&self) -> Option<PrimitiveKind> {
        match self {
            SolidOperation::Primitive { spec } => Some(spec.kind()),
            _ => None,
        }
    }
}

/// A single solid modelling request.
///
/// The stamp identifies the originating task so a late result can be rejected
/// as stale before it is committed. The seam does not compare it: the caller
/// checks it against the current stamp with [`TaskStamp::validate`] at commit
/// time, because the modeler deliberately has no database access.
#[derive(Debug, Clone, PartialEq)]
pub struct SolidRequest<'a> {
    pub stamp: TaskStamp,
    pub operation: SolidOperation<'a>,
}

impl SolidRequest<'_> {
    /// Pre-flight validation, run before any kernel work.
    ///
    /// Rejects malformed primitives (non-finite/non-positive dimensions),
    /// empty boolean/transform/mass operands, a non-finite, non-affine or
    /// singular transform, and a non-positive mass density. It deliberately does
    /// **not** reject a non-similarity transform (that is a capability refusal,
    /// `Unsupported`), a non-identity [`cad_domain::BrepPlacement`] (also a
    /// capability refusal in phase B1), or a stale stamp (only the caller knows
    /// the current stamp; there is no independent "invalid stamp" predicate in
    /// `cad-domain`, and inventing one would reject legitimate zero-generation
    /// stamps).
    pub fn validate(&self) -> Result<(), SolidInputError> {
        match &self.operation {
            SolidOperation::Primitive { spec } => spec.validate(),
            SolidOperation::Boolean { left, right, .. } => {
                require_operand("left", left)?;
                require_operand("right", right)
            }
            SolidOperation::Transform { brep, transform } => {
                require_operand("brep", brep)?;
                validate_transform(transform)
            }
            SolidOperation::MassProperties { brep, density } => {
                require_operand("brep", brep)?;
                if !density.is_finite() || *density <= 0.0 {
                    return Err(SolidInputError {
                        code: codes::SOLID_INVALID_DENSITY,
                        message: format!("density must be finite and positive, got {density}"),
                    });
                }
                Ok(())
            }
        }
    }
}

/// A validated-input failure raised before any kernel work starts.
///
/// Mirrors [`crate::TessellationInputError`]: the `code` is a stable
/// [`codes`] value and `message` is for diagnostics only, never parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SolidInputError {
    /// Stable [`codes`] value (`solid.invalid_*` / `solid.empty_operand`).
    pub code: &'static str,
    pub message: String,
}

impl std::fmt::Display for SolidInputError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for SolidInputError {}

fn require_positive(name: &str, value: f64) -> Result<(), SolidInputError> {
    if !value.is_finite() || value <= 0.0 {
        return Err(SolidInputError {
            code: codes::SOLID_INVALID_DIMENSION,
            message: format!("{name} must be finite and positive, got {value}"),
        });
    }
    Ok(())
}

fn require_finite_point(name: &str, point: Point3) -> Result<(), SolidInputError> {
    for (axis, value) in [("x", point.x), ("y", point.y), ("z", point.z)] {
        if !value.is_finite() {
            return Err(SolidInputError {
                code: codes::SOLID_INVALID_POINT,
                message: format!("{name}.{axis} must be finite, got {value}"),
            });
        }
    }
    Ok(())
}

fn require_operand(name: &str, brep: &BrepData) -> Result<(), SolidInputError> {
    if brep.is_empty() {
        return Err(SolidInputError {
            code: codes::SOLID_EMPTY_OPERAND,
            message: format!("{name} operand carries no B-rep face"),
        });
    }
    Ok(())
}

/// Reject a transform the affine placement cannot represent.
///
/// Only finiteness, the affine bottom row and a non-zero determinant are input
/// rules. Whether the linear part is a *similarity* is a capability question and
/// is answered during execution (`Unsupported`, ADR-0036 D4).
fn validate_transform(transform: &Transform3) -> Result<(), SolidInputError> {
    let matrix = &transform.matrix;
    if let Some(value) = matrix.iter().flatten().find(|v| !v.is_finite()) {
        return Err(SolidInputError {
            code: codes::SOLID_INVALID_TRANSFORM,
            message: format!("transform matrix contains a non-finite coefficient {value}"),
        });
    }
    if matrix[3] != [0.0, 0.0, 0.0, 1.0] {
        return Err(SolidInputError {
            code: codes::SOLID_INVALID_TRANSFORM,
            message: "transform matrix is not affine (last row must be [0, 0, 0, 1])".into(),
        });
    }
    let determinant = transform.determinant();
    if !determinant.is_finite() || determinant == 0.0 {
        return Err(SolidInputError {
            code: codes::SOLID_INVALID_TRANSFORM,
            message: format!("transform must be non-singular, determinant {determinant}"),
        });
    }
    Ok(())
}
