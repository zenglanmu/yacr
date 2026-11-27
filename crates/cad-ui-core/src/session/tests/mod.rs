//! Session unit tests.
//!
//! The test gate lives on the single `mod tests;` declaration in the
//! parent `session` module, so this module and its children are test-only. Each
//! child reaches the session items through `super::super::*` and the shared
//! fixtures through `super::test_support::*`.

mod test_support;
mod tests_acadinfo;
mod tests_action_recorder;
mod tests_admission;
mod tests_align;
mod tests_api_object;
mod tests_api_prompt;
mod tests_api_submit;
mod tests_array;
mod tests_array_arguments;
mod tests_batch;
mod tests_bim;
mod tests_command_log;
mod tests_commands;
mod tests_explode;
mod tests_fonts;
mod tests_hatch;
mod tests_hatch_arguments;
mod tests_hover;
mod tests_input;
mod tests_layers;
mod tests_mechanical;
mod tests_mesh;
mod tests_native_pinch;
mod tests_param;
mod tests_polyline_width;
mod tests_properties;
mod tests_scenes;
mod tests_session_surfaces;
mod tests_settings;
mod tests_stretch;
mod tests_sysvdlg;
mod tests_text;
mod tests_tools;
mod tests_tools_routing;
mod tests_typed_dimensions;
mod tests_wire;
