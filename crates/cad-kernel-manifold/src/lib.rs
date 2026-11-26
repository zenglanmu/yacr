#![forbid(unsafe_code)]
//! `manifold-rust` implementation of the kernel-independent [`MeshModeler`]
//! seam (ADR-0061, P-MESH-0).
//!
//! This crate is the **only** place in the workspace allowed to name
//! `manifold_rust`, exactly as `cad-kernel-solidcore` is the only `solidcore`
//! consumer. It owns two things and nothing else:
//!
//! 1. the conversion between the neutral [`cad_kernel_adapter::MeshData`] and
//!    the kernel's `MeshGL64`;
//! 2. the typed operations ([`cad_kernel_adapter::MeshOperation`]) as calls on
//!    `manifold_rust::manifold::Manifold`, plus the honest status mapping.
//!
//! It holds no document state, no mesh handle and no cache: content in, content
//! out. A mesh result is never exact B-rep geometry.

mod modeler;

pub use modeler::{declared_capabilities, ManifoldMeshModeler, TYPE_KEY, UPSTREAM_VERSION};

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(crate) use modeler::mesh_from_kernel;

#[cfg(test)]
pub(crate) use modeler::volume_of;
