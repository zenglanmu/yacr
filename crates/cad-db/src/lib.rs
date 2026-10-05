//! Authoritative storage; import builder and transactions are the only write paths.
//!
//! Spec v2.0 §4.3, §4.6: the drawing database is read-only after a controlled
//! import. A transaction is either fully committed or leaves no partial state,
//! and every commit raises the database revision and publishes an ordered
//! [`ChangeSet`].

mod bounds;
mod builder;
mod change;
mod drawing;
mod entity;
mod math;
mod measurement;
mod tables;
mod transform;
mod validate;

pub use bounds::*;
pub use builder::*;
pub use change::*;
pub use drawing::*;
pub use entity::*;
pub use measurement::*;
pub use tables::*;
pub use transform::*;
pub use validate::*;

#[cfg(test)]
mod tests;
