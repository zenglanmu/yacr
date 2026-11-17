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
/// `chamfer` is the P2.0 straight-chamfer capability; `nurbs` is P2.1b's curved
/// B-rep capability: it says the modeler can lift a body with a **non-planar**
/// face back into its kernel (a labelled cylinder, cone, sphere or torus with
/// its exact NURBS form and pcurves, or a form surface). `fillet` and `shell`
/// are the rest of phase P2 (ADR-0036 D7). A `false` flag means
/// [`crate::SolidOutcome::Unsupported`] today, never a degraded approximation.
/// `mass_properties` covers volume, surface area, centroid and inertia at a
/// **requested** relative accuracy (see [`crate::MassAccuracy`]): the estimate is
/// not a proved error bound.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ModelerCapabilities {
    pub primitives: ModelerPrimitives,
    pub booleans: bool,
    pub mass_properties: bool,
    pub chamfer: bool,
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
    /// a non-similarity transform, a curved body at a chamfer) happens in
    /// [`crate::SolidModeler::execute`] and is reported as
    /// [`crate::SolidOutcome::Unsupported`] with a stable reason code.
    ///
    /// One operand property **is** part of the probe, because it is a declared
    /// capability: a boolean, mass-properties or chamfer request whose operand
    /// B-rep has a non-planar face needs [`Self::nurbs`]. With `nurbs: false` the
    /// request is refused here, before any work, instead of being attempted and
    /// failing on the way into the kernel.
    ///
    /// [`SolidOperation::Transform`] is always `true`: a similarity transform is
    /// arithmetic on the neutral B-rep and part of the seam's baseline, while
    /// refusing a non-similarity is an execution-time decision.
    pub fn supports(&self, operation: &SolidOperation<'_>) -> bool {
        self.family(operation) && (self.nurbs || !requires_nurbs(operation))
    }

    /// Whether the **family** flag of `operation` is claimed, ignoring the
    /// per-operand curved-geometry gate. `supports` and `refusal` both read this
    /// so the two cannot disagree about which cause is being reported.
    fn family(&self, operation: &SolidOperation<'_>) -> bool {
        match operation {
            SolidOperation::Primitive { spec } => self.primitives.supports(spec.kind()),
            SolidOperation::Boolean { .. } => self.booleans,
            SolidOperation::Chamfer { .. } => self.chamfer,
            SolidOperation::Transform { .. } => true,
            SolidOperation::MassProperties { .. } => self.mass_properties,
        }
    }

    /// The structured `Unsupported` reason for an operation [`Self::supports`]
    /// refuses.
    ///
    /// Two causes, two codes: a curved operand this modeler cannot carry is
    /// [`codes::SOLID_NURBS_NOT_SUPPORTED`](crate::codes::SOLID_NURBS_NOT_SUPPORTED)
    /// (the family is implemented; the geometry is beyond this build), anything
    /// else is [`codes::SOLID_UNSUPPORTED`](crate::codes::SOLID_UNSUPPORTED) (the
    /// family itself is not claimed).
    pub fn refusal(&self, operation: &SolidOperation<'_>) -> super::SolidUnsupportedReason {
        let operation_family = operation.family();
        if !self.family(operation) {
            // The family itself is not claimed: say so, whatever the operand is.
            // Reporting the curved-operand gate here would name a cause that
            // fixing `nurbs` would not help.
            return super::SolidUnsupportedReason {
                code: crate::codes::SOLID_UNSUPPORTED,
                operation: operation_family,
                detail: format!("this build does not claim {operation_family}"),
            };
        }
        if requires_nurbs(operation) && !self.nurbs {
            return super::SolidUnsupportedReason {
                code: crate::codes::SOLID_NURBS_NOT_SUPPORTED,
                operation: operation_family,
                detail: "the operand has a curved (non-planar) face; this modeler does not \
                         declare the `nurbs` capability, so the B-rep cannot be lifted into its \
                         kernel"
                    .into(),
            };
        }
        // `supports` accepted it; a caller asking for a refusal anyway gets the
        // generic family answer rather than an invented cause.
        super::SolidUnsupportedReason {
            code: crate::codes::SOLID_UNSUPPORTED,
            operation: operation_family,
            detail: format!("this build does not claim {operation_family}"),
        }
    }
}

/// Whether a B-rep operand of `operation` has a face this seam calls **curved**:
/// any supporting surface other than a plane.
///
/// The check is on the neutral surface kind, not on the loops: a plane bounded by
/// an arc is still planar, while a cylinder is curved even when its boundary is
/// two full circles. `Transform` and `Primitive` never read an operand.
pub fn requires_nurbs(operation: &SolidOperation<'_>) -> bool {
    fn curved(brep: &cad_domain::BrepData) -> bool {
        brep.shells
            .iter()
            .flat_map(|shell| &shell.faces)
            .any(|face| !matches!(face.surface, cad_domain::BrepSurface::Plane { .. }))
    }
    match operation {
        SolidOperation::Primitive { .. } | SolidOperation::Transform { .. } => false,
        SolidOperation::Boolean { left, right, .. } => curved(left) || curved(right),
        SolidOperation::Chamfer { brep, .. } | SolidOperation::MassProperties { brep, .. } => {
            curved(brep)
        }
    }
}
