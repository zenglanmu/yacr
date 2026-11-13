//! Command normalization, status projection and lifecycle session tests.
//!
//! Split by theme across the submodules of this directory; every test is kept
//! verbatim and reaches the session items through `super::super::super::*` and
//! the shared fixtures through `super::super::test_support::*`.

mod alias_table;
mod annot_arguments;
mod classic_status;
mod command_vocabulary;
mod command_vocabulary_clipboard;
mod command_vocabulary_inquiry;
mod command_vocabulary_text;
mod dash_layer_arguments;
mod dimension_arguments;
mod draw2_arguments;
mod extended_vocabulary;
mod extras_arguments;
mod gradient;
mod hatchedit;
mod image_arguments;
mod import_accounting;
mod input_and_lifecycle;
mod layer_arguments;
mod layout_arguments;
mod layout_status;
mod mleader_arguments;
mod modify_arguments;
mod orbit;
mod paper_arguments;
mod payload_options;
mod pedit;
mod point_style;
mod refedit_arguments;
mod select_filter;
mod shape_block_arguments;
mod shell_settings;
mod snap_grid;
mod space_argument;
mod standard_view;
mod status_projection;
mod style_arguments;
mod table_arguments;
mod text_arguments;
mod text_batch;
mod utility_arguments;
mod view_arguments;
mod view_status;
mod vocabulary_control;
mod xclip;
mod xref_status;
mod zoom_argument;
mod zoom_object_xp;
mod zoom_window_previous;

/// A committed synthetic, writer-produced AC1032 DWG with four LINE
/// entities (`fixtures/manifest`). It is not a vendor sample.
const SYNTHETIC_DWG: &[u8] = include_bytes!("../../../../../fixtures/dwg/synthetic-four-lines.dwg");
