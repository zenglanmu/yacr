#![cfg(not(target_arch = "wasm32"))]
//! `solidcore` implementation of the kernel-independent [`SolidModeler`] seam
//! (ADR-0036, phase B2).
//!
//! This crate is the **only** place in the workspace allowed to name
//! `solidcore` (the same single-owner rule `cad-import-acadrust` has for
//! `acadrust`; `scripts/check-architecture.py` enforces both). It owns three
//! things and nothing else:
//!
//! 1. the two conversions between the neutral [`cad_domain::BrepData`] and the
//!    upstream portable record (`solidcore_record::brep::Brep`);
//! 2. the typed operations ([`cad_kernel_adapter::SolidOperation`]) as calls on
//!    `solidcore_ops::Body`;
//! 3. the honest capability report of the pinned revision.
//!
//! It holds no document state, no body handle and no cache: content in, content
//! out (ADR-0036 D1, [`cad_kernel_adapter::SolidModeler`]).
//!
//! # Scope of this cut (P1, honest)
//!
//! **Fully supported** (create, edit, mass, display):
//!
//! - `BOX`, `WEDGE`, `PYRAMID`: authored locally as exact planar B-rep data
//!   ([`polyhedra`]), lifted to a record and validated by the kernel
//!   (`Body::from_record(.., false)`), so a body handed out is one the kernel
//!   accepted;
//! - `UNION`/`SUBTRACT`/`INTERSECT` of bodies whose faces are all planes with
//!   `Line` boundary curves;
//! - a straight **chamfer on one edge** of such a planar body (P2.0,
//!   [`cad_kernel_adapter::SolidOperation::Chamfer`]): the product is still
//!   all-planar, so it re-lifts; NURBS, fillets, shells and the `SURFACE` family
//!   remain unimplemented and are refused, not approximated;
//! - mass properties of the same planar bodies;
//! - similarity transforms of **any** body this module can describe, including
//!   the curved primitives below: the map is baked into the neutral B-rep
//!   arithmetically and never re-enters the kernel.
//!
//! **Created, displayable and exportable, but not editable in P1**:
//! `CYLINDER`, `SPHERE`, `CONE` and `TORUS` are built by the upstream
//! constructors (`solidcore_ops::{make_cylinder, make_sphere, make_cone,
//! make_torus}`), read back through `Body::to_record` and mapped to analytic
//! [`cad_domain::BrepSurface`]/[`cad_domain::BrepCurve`] variants.
//!
//! **Refused, never approximated**: any operation that has to *lift* a
//! `BrepData` back into the kernel — a boolean, mass-properties or chamfer
//! request — on a body with a non-planar face or a non-`Line` boundary curve.
//! Those return [`cad_kernel_adapter::SolidOutcome::Unsupported`] with the stable
//! reason [`LIFT_REQUIRES_PLANAR`]. Lifting curved shells losslessly needs NURBS
//! surfaces *and* their pcurves in the neutral model, which is phase P2.1.
//!
//! # Family-level capabilities, per-operand refusals
//!
//! [`cad_kernel_adapter::ModelerCapabilities::booleans`], `mass_properties` and
//! `chamfer` being `true` means "this family is implemented", **not** "every
//! operand can be evaluated": each execution still re-lifts its operands and may
//! refuse. The planar lift accepts exactly **one closed, consistently oriented
//! shell** (`[LIFT_UNSUPPORTED]` otherwise):
//!
//! - a boolean result with a **void region (a cavity)** is refused **at the
//!   boolean itself** (the kernel stores a cavity as a void region, which this
//!   lift does not carry); a **multi-shell** result (a disjoint `UNION`) is
//!   *returned as a body* — `BrepData` holds both shells — and refused only when
//!   it is **lifted again** by a later mass-properties or boolean request, so
//!   mass properties need exactly one closed shell;
//! - an open sheet (edges used once), a reversed face, a duplicated face, a gap
//!   in a loop, or a record whose geometry disagrees with its topology is
//!   refused rather than measured: the measured symptoms of letting them through
//!   were a plausible area of 40 instead of 52 and a volume of 36 instead of 24;
//! - mass properties additionally require **exactly one closed shell**: a void
//!   region or a second shell is refused instead of summing two volumes.
//!
//! Callers must therefore handle a per-call `Unsupported` from a family the
//! modeler declares; they must not cache "booleans are supported" as a
//! per-operand guarantee.
//!
//! Also refused with stable reasons: void/`wire`/`vertex` shells, multiple
//! shells, vertex loops, trimmed spherical/toroidal faces, a torus whose tube
//! starts off its outer equator, a non-identity placement that cannot be baked,
//! and a mirroring transform (see [`LIFT_UNSUPPORTED`] and
//! [`PLACEMENT_NOT_SUPPORTED`]).
//!
//! # Units
//!
//! The upstream record is millimetres, radians, right-handed Z-up. The neutral
//! B-rep carries *drawing units* with no declared scale, so this seam performs
//! no unit conversion: one drawing unit is handed to the kernel as one record
//! unit. Volumes and areas therefore come back in the document's units cubed and
//! squared, and this module never re-labels them as mm³/mm². Convert at the
//! command layer when a document's units are known.
//!
//! # Cancellation
//!
//! The seam's `cancelled` callback is checked before and after every operation.
//! Booleans additionally run under `solidcore_ops::cancel` with a watcher thread
//! that polls the callback every ~50 ms (the **detection granularity**) and
//! cancels the kernel's `CancelToken`, so a long boolean stops at the kernel's
//! next poll; the watcher is joined before the call returns, and is woken with
//! `unpark` as soon as the call ends (and from a `Drop` guard if the call
//! unwinds), so a completed or panicking call never waits for a poll. That is
//! cooperative, poll-based cancellation of the *running* call, not preemption,
//! and a cancelled call yields [`cad_domain::CadError::Cancelled`] with no
//! product.
//!
//! # Registration
//!
//! [`SolidCoreModeler::registration`] reports `type_key = "yacr.kernel.solidcore"`
//! and `entity_types = ["3DSOLID", "BODY"]` — the two classes this modeler can
//! create and consume. `REGION`/`SURFACE` are zero-thickness sheet bodies, which
//! this cut neither creates nor accepts, so they are not advertised.
//! [`cad_domain::Registration::version`] is a `u32`, so the pinned upstream
//! version string is carried in [`SolidCoreModeler::UPSTREAM_VERSION`]
//! (`1.0.1+55ae49ef919b`) and advertised in `Registration::capabilities`;
//! `version` stays `1`, the contract version of this adapter. The upstream
//! `1.0.1` string was **not** bumped at the pin, so it does not identify the
//! revision on its own — the suffix does.
//!
//! # wasm
//!
//! The whole crate is `#![cfg(not(target_arch = "wasm32"))]` and the upstream
//! dependencies are target-gated: browsers have no threads for the watcher and
//! the carriers where solidcore is not linked must report
//! `unsupported-runtime: solid-modeler` instead of a silent failure.

mod error;
mod modeler;
mod ops;
pub mod polyhedra;
mod record;

pub use error::{
    CHAMFER_FAILED, EDGE_NOT_FOUND, EMPTY_RESULT, KERNEL_FAILED, KERNEL_PANIC,
    LIFT_REQUIRES_PLANAR, LIFT_UNSUPPORTED, NONUNIFORM_TRANSFORM, PLACEMENT_NOT_SUPPORTED,
    UNSURE_REGION,
};
pub use modeler::{SolidCoreModeler, TYPE_KEY, UPSTREAM_REVISION, UPSTREAM_VERSION};
pub use polyhedra::{box_brep, pyramid_brep, wedge_brep};
