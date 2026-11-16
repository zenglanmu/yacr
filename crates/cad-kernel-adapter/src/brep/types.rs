//! Path-preserving re-exports of the neutral B-rep data model.
//!
//! The types themselves now live in [`cad_domain::brep`] — a body's exact
//! geometry is domain data, not a kernel-implementation detail. This module
//! keeps the historical `crate::brep::types::…` and `brep::…` paths resolving so
//! the crate's public surface is unchanged; the adapter owns only the evaluation
//! and tessellation seams that consume them.

pub use cad_domain::{
    BrepCurve, BrepData, BrepFace, BrepLoop, BrepPlacement, BrepShell, BrepSurface,
};
