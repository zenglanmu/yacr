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
//! # Scope of this cut (P1 + P2.1b part 2, honest)
//!
//! **Fully supported** (create, edit, mass, display):
//!
//! - `BOX`, `WEDGE`, `PYRAMID`: authored locally as exact planar B-rep data
//!   ([`polyhedra`]), lifted to a record and validated by the kernel
//!   (`Body::from_record(.., false)`), so a body handed out is one the kernel
//!   accepted;
//! - `UNION`/`SUBTRACT`/`INTERSECT` of bodies whose faces are planes with `Line`,
//!   `Circle` or `Arc` boundaries (holes included), and of bodies with a curved
//!   face;
//! - a straight **chamfer on one `Line` edge** (P2.0,
//!   [`cad_kernel_adapter::SolidOperation::Chamfer`]): the product re-lifts, also
//!   when the body it cuts is curved; a curved *edge* (a cylinder rim) needs the
//!   curve-edge selector and stays P2.2. Fillets, shells and the `SURFACE` family
//!   remain unimplemented and are refused, not approximated;
//! - mass properties of those bodies, including the closed forms of a cylinder,
//!   sphere, cone and torus;
//! - similarity transforms of **any** body this module can describe, including
//!   the curved primitives below: the map is baked into the neutral B-rep
//!   arithmetically and never re-enters the kernel.
//!
//! **Curved bodies lift (P2.1b part 2)**: `CYLINDER`, `SPHERE`, `CONE` and
//! `TORUS` are built by the upstream constructors
//! (`solidcore_ops::{make_cylinder, make_sphere, make_cone, make_torus}`), read
//! back through `Body::to_record`, mapped to analytic
//! [`cad_domain::BrepSurface`]/[`cad_domain::BrepCurve`] variants — and written
//! **back** with the exact rational NURBS form `solidcore`'s `eval::form` gives
//! each analytic label, the form's `(u, v)` box and every use's exact pcurve. So
//! a boolean, mass-properties or chamfer request over a curved body reaches the
//! kernel: the closed forms of a cylinder, sphere, cone and torus are measured,
//! and booleans on them are answered.
//!
//! The one refusal that survives at the lift is a **boundary family** this build
//! does not write (a spline or ellipse curve, an `Unsupported` surface) —
//! [`LIFT_REQUIRES_PLANAR`] — plus the structural ones. A curved face with no
//! stated `(u, v)` domain is [`LIFT_UNSUPPORTED`] by name, and a use whose pcurve
//! has no exact closed form in its surface parameters is `solid.pcurve_not_exact`:
//! nothing is fitted and no box is invented.
//!
//! Display follows `cad-db`, not this crate: a **trimmed** curved face lifts but
//! tessellates as a partial body naming that face (the neutral tessellator draws a
//! cylinder strip only from two full circles). A **NURBS** face has a mesher in
//! this build: [`tess::SolidCoreTessellator`] is the `solidcore-tess` display
//! mesher of the pinned revision, behind the same [`cad_kernel_adapter::SolidTessellator`]
//! seam the neutral tessellator implements (P2.1c part 1). It is **injected by a
//! carrier**, exactly like the modeler: a session without it keeps the neutral
//! path, which still reports a NURBS face as a missing face by name.
//!
//! # Family-level capabilities, per-operand refusals
//!
//! [`cad_kernel_adapter::ModelerCapabilities::booleans`], `mass_properties` and
//! `chamfer` being `true` means "this family is implemented", **not** "every
//! operand can be evaluated": each execution still re-lifts its operands and may
//! refuse. The lift accepts exactly **one closed, consistently oriented shell**
//! (`[LIFT_UNSUPPORTED]` otherwise):
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
//!   region or a second shell is refused instead of summing two volumes;
//! - a **curved** operand needs `ModelerCapabilities::nurbs`; the pinned
//!   modeler declares it, and each execution still writes the exact form — an
//!   operand it cannot express (no `(u, v)` domain, no exact pcurve) is refused
//!   by name, never approximated.
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
//! [`cad_kernel_adapter::ModelerCapabilities::nurbs`] is `true` and is
//! **load-bearing**: a boolean, mass-properties or chamfer operand with a
//! non-planar face needs it, and a modeler that declares `false` refuses such an
//! operand with `solid.nurbs_not_supported` before any work.
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
pub mod tess;

pub use error::{
    CHAMFER_FAILED, EDGE_NOT_FOUND, EMPTY_RESULT, KERNEL_FAILED, KERNEL_PANIC,
    LIFT_REQUIRES_PLANAR, LIFT_UNSUPPORTED, NONUNIFORM_TRANSFORM, PCURVE_NOT_EXACT,
    PLACEMENT_NOT_SUPPORTED, UNSURE_REGION,
};
pub use modeler::{SolidCoreModeler, TYPE_KEY, UPSTREAM_REVISION, UPSTREAM_VERSION};
pub use polyhedra::{box_brep, pyramid_brep, wedge_brep};
pub use tess::SolidCoreTessellator;
