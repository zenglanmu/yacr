//! Authoritative storage; import builder and transactions are the only write paths.
//!
//! Spec v2.0 §4.3, §4.6: the drawing database is read-only after a controlled
//! import. A transaction is either fully committed or leaves no partial state,
//! and every commit raises the database revision and publishes an ordered
//! [`ChangeSet`].

mod bim;
mod bounds;
mod builder;
mod change;
mod dim_style;
mod drawing;
mod entity;
mod feature;
mod identity;
mod math;
mod measurement;
mod mesh_validation;
mod mleader_style;
mod relation;
mod scope;
mod snapshot;
pub mod store;
mod tables;
mod transform;
mod validate;
mod xref;

pub use bim::*;
pub use bounds::*;
pub use builder::*;
pub use change::*;
pub use dim_style::*;
pub use drawing::*;
pub use entity::*;
pub use feature::*;
pub use measurement::*;
pub use mesh_validation::validate_texture_references;
pub use mleader_style::*;
pub use relation::*;
pub use scope::*;
pub use snapshot::*;
pub use store::*;
pub use tables::*;
pub use transform::*;
pub use validate::*;
pub use xref::*;

#[cfg(test)]
mod tests;
