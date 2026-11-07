//! Command normalization, status projection and lifecycle session tests.
//!
//! Split by theme across the submodules of this directory; every test is kept
//! verbatim and reaches the session items through `super::super::super::*` and
//! the shared fixtures through `super::super::test_support::*`.

mod alias_table;
mod annot_arguments;
mod command_vocabulary;
mod command_vocabulary_clipboard;
mod command_vocabulary_inquiry;
mod dimension_arguments;

mod draw2_arguments;
mod gradient;
mod hatchedit;
mod import_accounting;
mod input_and_lifecycle;
mod layer_arguments;
mod layout_arguments;
mod layout_status;
mod pedit;
mod point_style;
mod select_filter;
mod snap_grid;
mod space_argument;
mod standard_view;
mod status_projection;
mod text_batch;
mod utility_arguments;
mod view_arguments;
mod xref_status;
mod zoom_argument;
mod zoom_window_previous;

/// A committed synthetic, writer-produced AC1032 DWG with four LINE
/// entities (`fixtures/manifest`). It is not a vendor sample.
const SYNTHETIC_DWG: &[u8] = include_bytes!("../../../../../fixtures/dwg/synthetic-four-lines.dwg");
