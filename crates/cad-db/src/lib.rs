//! Authoritative storage; import builder and transactions are the only write paths.
//!
//! Spec v2.0 §4.3, §4.6: the drawing database is read-only after a controlled
//! import. A transaction is either fully committed or leaves no partial state,
//! and every commit raises the database revision and publishes an ordered
//! [`ChangeSet`].

mod bounds;
mod builder;
mod change;
mod dim_style;
mod drawing;
mod entity;
mod identity;
mod math;
mod measurement;
mod mleader_style;
mod scope;
mod snapshot;
pub mod store;
mod tables;
mod transform;
mod validate;
mod xref;

pub use bounds::*;
pub use builder::*;
pub use change::*;
pub use dim_style::*;
pub use drawing::*;
pub use entity::*;
pub use measurement::*;
pub use mleader_style::*;
pub use scope::*;
pub use snapshot::*;
pub use store::*;
pub use tables::*;
pub use transform::*;
pub use validate::*;
pub use xref::*;

#[cfg(test)]
mod tests;
