//! What a [`SolidModeler`](super::SolidModeler) declares it can do.
//!
//! The declaration is a contract, not a promise: a capability reported `true`
//! must produce a [`crate::SolidOutcome::Success`] for representable input, and
//! a capability reported `false` must produce
//! [`crate::SolidOutcome::Unsupported`] instead of attempting the call. Phase B1
//! has no kernel, so there is no default instance claiming anything.

use super::{PrimitiveKind, SolidOperation};

/// The primitive families a modeler can create.
///
/// One flag per family so a host can offer `BOX` without claiming `TORUS`.
/// [`ModelerPrimitives::supports`] is the lookup used by
/// [`ModelerCapabilities::supports`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ModelerPrimitives {
    /// `BOX` (cuboid). Named `box_primitive` because `box` is a Rust keyword.
    pub box_primitive: bool,
    /// `WEDGE`.
    pub wedge: bool,
    /// `PYRAMID`.
    pub pyramid: bool,
    /// `CYLINDER`.
    pub cylinder: bool,
    /// `SPHERE`.
    pub sphere: bool,
    /// `CONE` (full cone and frustum).
    pub cone: bool,
    /// `TORUS`.
    pub torus: bool,
}

impl ModelerPrimitives {
    /// Every primitive family.
    pub const fn all() -> Self {
        ModelerPrimitives {
            box_primitive: true,
            wedge: true,
            pyramid: true,
            cylinder: true,
            sphere: true,
            cone: true,
            torus: true,
        }
    }

    /// Whether this family is declared.
    pub fn supports(self, kind: PrimitiveKind) -> bool {
        match kind {
            PrimitiveKind::Box => self.box_primitive,
            PrimitiveKind::Wedge => self.wedge,
            PrimitiveKind::Pyramid => self.pyramid,
            PrimitiveKind::Cylinder => self.cylinder,
            PrimitiveKind::Sphere => self.sphere,
            PrimitiveKind::Cone => self.cone,
            PrimitiveKind::Torus => self.torus,
        }
    }
}

/// The full capability declaration of a modeler.
///
/// `fillet`, `shell` and `nurbs` are phase P2 (ADR-0036 D7); a `false` flag
/// there means [`crate::SolidOutcome::Unsupported`] today, never a degraded
/// approximation. `mass_properties` covers volume, surface area, centroid and
/// inertia at a **requested** relative accuracy (see
/// [`crate::MassAccuracy`]): the estimate is not a proved error bound.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ModelerCapabilities {
    pub primitives: ModelerPrimitives,
    pub booleans: bool,
    pub mass_properties: bool,
    pub fillet: bool,
    pub shell: bool,
    pub nurbs: bool,
}

impl ModelerCapabilities {
    /// Whether this modeler claims the family `operation` belongs to.
    ///
    /// This is a cheap probe: it answers per operation *family* and per
    /// primitive kind, not whether a specific operand is representable. Refusing
    /// an individual operand (an unsupported surface, a non-identity placement,
    /// a non-similarity transform) happens in
    /// [`crate::SolidModeler::execute`] and is reported as
    /// [`crate::SolidOutcome::Unsupported`] with a stable reason code.
    ///
    /// [`SolidOperation::Transform`] is always `true`: a similarity transform is
    /// arithmetic on the neutral B-rep and part of the seam's baseline, while
    /// refusing a non-similarity is an execution-time decision.
    pub fn supports(&self, operation: &SolidOperation<'_>) -> bool {
        match operation {
            SolidOperation::Primitive { spec } => self.primitives.supports(spec.kind()),
            SolidOperation::Boolean { .. } => self.booleans,
            SolidOperation::Transform { .. } => true,
            SolidOperation::MassProperties { .. } => self.mass_properties,
        }
    }
}
