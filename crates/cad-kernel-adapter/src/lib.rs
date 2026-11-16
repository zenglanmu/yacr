//! Isolated seam between imported solid/surface payloads and tessellatable
//! geometry (spec §10.2, requirement F15).
//!
//! This crate owns every kernel-facing type. Nothing here may leak an ACIS
//! kernel handle, a SAT/SAB parser or a surface object into the rest of the
//! workspace; the only things that cross the boundary are the plain-data
//! [`TessellationRequest`] and the structured [`TessellationResult`].
//!
//! # Honest status
//!
//! There is **no** ACIS kernel, no SAT/SAB parser and no licensed fixture
//! sample in this repository yet. A default [`SolidTessellator`],
//! [`NoKernelTessellator`], therefore never fabricates a mesh: it reports the
//! request as [`TessellationOutcome::Unsupported`] with a stable diagnostic
//! code. See `docs/kernel-acis.md` for the integration contract and the
//! acceptance criteria that must be met before this can be considered done.
//!
//! A second, independent seam lives in [`solid`]: [`SolidModeler`] creates and
//! combines **exact** bodies (primitives, booleans, similarity transforms, mass
//! properties) and returns neutral [`BrepData`]. Phase B1 (ADR-0036) defines only
//! that contract; this crate still links no modelling kernel and has no default
//! modeler.

use cad_domain::*;

pub mod brep;
pub use brep::{BrepCurve, BrepData, BrepFace, BrepLoop, BrepPlacement, BrepShell, BrepSurface};

pub mod codes;

pub mod solid;

mod brep_tessellator;
mod exchange;
mod no_kernel;
mod outcome;
mod request;
mod seam;

pub use brep_tessellator::*;
pub use exchange::*;
pub use no_kernel::*;
pub use outcome::*;
pub use request::*;
pub use seam::*;
pub use solid::*;

#[cfg(test)]
mod tests;
