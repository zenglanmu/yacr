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
    codes, ModelerCapabilities, ModelerPrimitives, SolidModeler, SolidOperation, SolidOutcome,
    SolidRequest, SolidResult, SolidUnsupportedReason,
};

use crate::ops;

/// The pinned upstream revision this build links (the first 12 hex digits of
/// `55ae49ef919b606bf77901df626ed9bb543e5212`).
pub const UPSTREAM_REVISION: &str = "55ae49ef919b";

/// The pinned upstream version string, **unchanged by upstream at the pin**
/// (`1.0.1` is the crates.io release; the repository did not bump it), so the
/// revision suffix is what identifies this build.
pub const UPSTREAM_VERSION: &str = "1.0.1+55ae49ef919b";

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
/// `booleans`, `mass_properties` and `chamfer` are `true` for **planar**
/// operands; a curved operand is still refused at execution time with
/// [`crate::LIFT_REQUIRES_PLANAR`] (curved lift is P2.1). `chamfer` is the
/// straight-section, one-edge capability of P2.0 — it does not imply a fillet,
/// a shell or a NURBS body. `fillet`, `shell` and `nurbs` are `false`: this
/// build links no such operation, so they must not be claimed.
fn declared_capabilities() -> ModelerCapabilities {
    ModelerCapabilities {
        primitives: ModelerPrimitives::all(),
        booleans: true,
        mass_properties: true,
        chamfer: true,
        fillet: false,
        shell: false,
        nurbs: false,
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
                "mass_properties".into(),
                "transform.similarity".into(),
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
                SolidOperation::Transform { brep, transform } => {
                    ops::transform::transform(brep, transform)?
                }
                SolidOperation::MassProperties { brep, density } => {
                    ops::mass::mass(brep, *density)?
                }
            }
        } else {
            // Unreachable with the declaration above, but the contract says a
            // capability that is not claimed returns `Unsupported`, never a
            // silent attempt.
            SolidOutcome::Unsupported {
                reason: SolidUnsupportedReason {
                    code: codes::SOLID_UNSUPPORTED,
                    operation: request.operation.family(),
                    detail: format!("this build does not claim {}", request.operation.family()),
                },
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
