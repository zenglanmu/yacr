//! Command normalization, status projection and lifecycle session tests.
//!
//! Split by theme across the submodules of this directory; every test is kept
//! verbatim and reaches the session items through `super::super::super::*` and
//! the shared fixtures through `super::super::test_support::*`.

mod command_vocabulary;
mod import_accounting;
mod input_and_lifecycle;
mod status_projection;

/// A committed synthetic, writer-produced AC1032 DWG with four LINE
/// entities (`fixtures/manifest`). It is not a vendor sample.
const SYNTHETIC_DWG: &[u8] = include_bytes!("../../../../../fixtures/dwg/synthetic-four-lines.dwg");
