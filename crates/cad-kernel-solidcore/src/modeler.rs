//! [`SolidCoreModeler`]: the [`SolidModeler`] implementation over the pinned
//! `solidcore` revision.
//!
//! The modeler is a zero-sized, stateless service. Thread safety comes from the
//! upstream session: `solidcore_ops` serialises every call on one process kernel
//! behind a mutex, so two `execute` calls from two threads queue up and both
//! succeed without this crate adding a lock of its own (proved by
//! `tests/chain.rs::two_threads_share_one_modeler`). A modeler therefore never
//! needs `&mut self` and is usable as `Arc<dyn SolidModeler>`.

use cad_domain::{CadResult, Registration};
use cad_kernel_adapter::{
    ModelerCapabilities, ModelerPrimitives, SolidModeler, SolidOperation, SolidOutcome,
    SolidRequest, SolidResult,
};

use crate::ops;

/// The pinned upstream revision this build links (the first 12 hex digits of
/// `f17599cbfa99b420ea9bc24531ad2a808dbda34c`).
pub const UPSTREAM_REVISION: &str = "f17599cbfa99";

/// The pinned upstream version string, **unchanged by upstream at the pin**
/// (`1.0.1` is the `solidcore` crate's version in the repository, not just the
/// crates.io release), so the revision suffix is what identifies this build.
pub const UPSTREAM_VERSION: &str = "1.0.1+f17599cbfa99";

/// The seam's registration type key: this is the `solidcore` modeler, by name.
pub const TYPE_KEY: &str = "yacr.kernel.solidcore";

/// The solid modeler backed by the pinned `solidcore` kernel.
///
/// Zero-sized: one instance per process is enough, and sharing it costs nothing.
/// All state lives in the upstream session (the kernel, its rollback and its
/// cancellation token), never here.
#[derive(Debug, Default, Clone, Copy)]
pub struct SolidCoreModeler;

impl SolidCoreModeler {
    /// A new (zero-sized) modeler.
    pub fn new() -> Self {
        SolidCoreModeler
    }
}

/// The capability declaration of this build.
///
/// `booleans`, `mass_properties` and `chamfer` are `true` for bodies whose faces
/// are planes **and** for bodies with a curved face (P2.1b): a curved shell is
/// lifted back into the kernel with its exact NURBS form and exact pcurves, so
/// the same gate serves both. `nurbs: true` is the flag that gate reads (see
/// [`cad_kernel_adapter::requires_nurbs`]); it does not imply a fillet, a shell
/// or a NURBS *edit*, which stay out of this build. `fillet` and `shell` remain
/// `false`.
fn declared_capabilities() -> ModelerCapabilities {
    ModelerCapabilities {
        primitives: ModelerPrimitives::all(),
        booleans: true,
        mass_properties: true,
        chamfer: true,
        offset_face: true,
        extrude_profile: true,
        fillet: false,
        shell: false,
        nurbs: true,
    }
}

impl SolidModeler for SolidCoreModeler {
    fn registration(&self) -> Registration {
        Registration {
            type_key: TYPE_KEY.into(),
            // The seam's own contract version: `Registration::version` is a
            // `u32`, so the upstream `1.0.1+<rev>` string is advertised in
            // `capabilities` below instead of being squeezed into a number.
            version: 1,
            priority: 10,
            // Only the classes this modeler can create and consume. `REGION`
            // and `SURFACE` are `BodyKind::Sheet` bodies (zero-thickness), which
            // this cut neither creates nor accepts as boolean/mass operands (a
            // sheet has one use per edge and is refused), so advertising them
            // would over-claim; they come with P2's SURFACE family.
            entity_types: vec!["3DSOLID".into(), "BODY".into()],
            capabilities: vec![
                "primitives.box".into(),
                "primitives.wedge".into(),
                "primitives.pyramid".into(),
                "primitives.cylinder".into(),
                "primitives.sphere".into(),
                "primitives.cone".into(),
                "primitives.torus".into(),
                "boolean.union".into(),
                "boolean.subtract".into(),
                "boolean.intersect".into(),
                "chamfer.sides".into(),
                // The signed one-face move (Push/Pull's atomic operation).
                "offset.face".into(),
                // One closed planar profile extruded into a capped solid.
                "extrude.profile".into(),
                "mass_properties".into(),
                "transform.similarity".into(),
                // The curved-lift capability the `nurbs` flag stands for: a
                // non-planar face is written back with its exact form and exact
                // pcurves. Advertised by name so a host can tell which face
                // kinds ride on the flag.
                "nurbs.faces".into(),
                format!("solidcore.version={UPSTREAM_VERSION}"),
            ],
        }
    }

    fn capabilities(&self) -> ModelerCapabilities {
        declared_capabilities()
    }

    fn execute(
        &self,
        request: &SolidRequest<'_>,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> CadResult<SolidResult> {
        // Cancellation before any work, and before input validation: a request
        // nobody waits for is never worth a diagnostic.
        if cancelled() {
            return Err(cad_domain::CadError::Cancelled);
        }
        request
            .validate()
            .map_err(|error| cad_domain::CadError::InvalidInput(error.to_string()))?;

        let capabilities = self.capabilities();
        let outcome = if capabilities.supports(&request.operation) {
            match &request.operation {
                SolidOperation::Primitive { spec } => ops::primitive::create(spec)?,
                SolidOperation::Boolean { op, left, right } => {
                    ops::boolean::boolean(*op, left, right, cancelled)?
                }
                SolidOperation::Chamfer {
                    brep,
                    selector,
                    distance,
                    second,
                } => ops::chamfer::chamfer(brep, *selector, *distance, *second, cancelled)?,
                SolidOperation::OffsetFace {
                    brep,
                    selector,
                    distance,
                } => ops::offset_face::offset_face(brep, *selector, *distance, cancelled)?,
                SolidOperation::ExtrudeProfile {
                    profile,
                    direction,
                    distance,
                } => ops::extrude_profile::extrude_profile(
                    profile, *direction, *distance, cancelled,
                )?,
                SolidOperation::Transform { brep, transform } => {
                    ops::transform::transform(brep, transform)?
                }
                SolidOperation::MassProperties { brep, density } => {
                    ops::mass::mass(brep, *density)?
                }
            }
        } else {
            // The declaration above refuses a curved operand when `nurbs` is
            // false, and an unknown family otherwise; the reason says which.
            SolidOutcome::Unsupported {
                reason: capabilities.refusal(&request.operation),
            }
        };

        // A product that arrives after cancellation is dropped: never committed,
        // never returned as an outcome.
        if cancelled() {
            return Err(cad_domain::CadError::Cancelled);
        }
        Ok(SolidResult {
            stamp: request.stamp.clone(),
            outcome,
        })
    }
}
