//! Authoritative storage; import builder and transactions are the only write paths.
//!
//! Spec v2.0 §4.3, §4.6: the drawing database is read-only after a controlled
//! import; the annotation database is only mutated through transactions. A
//! transaction is either fully committed or leaves no partial state, and every
//! commit raises the database revision and publishes an ordered [`ChangeSet`].

mod annotation;
mod annotation_db;
mod bounds;
mod builder;
mod change;
mod drawing;
mod entity;
mod math;
mod tables;

pub use annotation::*;
pub use annotation_db::*;
pub use bounds::*;
pub use builder::*;
pub use change::*;
pub use drawing::*;
pub use entity::*;
pub use tables::*;

#[cfg(test)]
mod tests;
