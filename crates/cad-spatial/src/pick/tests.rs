//! Precise-pick tests, split by theme.
//!
//! Shared fixtures live in [`common`]; each theme module owns one cohesive
//! group of tests (bounds/culling, low-level primitives, whole-query reports,
//! and sub-element resolution). The split keeps every file well under the
//! 1000-line code-file gate.

mod bounds;
mod common;
mod primitives;
mod reports;
mod sub_elements;
