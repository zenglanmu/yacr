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
mod tests_block_names;
mod tests_camera;
mod tests_command_log;
mod tests_commands;
mod tests_display;
mod tests_explode;
mod tests_flip;
mod tests_fonts;
mod tests_guides;
mod tests_hatch;
mod tests_hatch_arguments;
mod tests_hover;
mod tests_input;
mod tests_layers;
mod tests_materials;
mod tests_mechanical;
mod tests_mesh;
mod tests_native_pinch;
mod tests_outershell;
mod tests_outliner;
mod tests_panels;
mod tests_param;
mod tests_polyline_width;
mod tests_properties;
mod tests_protractor;
mod tests_scenes;
mod tests_session_surfaces;
mod tests_settings;
mod tests_soften;
mod tests_splitsolid;
mod tests_stretch;
mod tests_styles;
mod tests_sysvdlg;
mod tests_text;
mod tests_text3d;
mod tests_tools;
mod tests_tools_routing;
mod tests_trimsolid;
mod tests_typed_dimensions;
mod tests_walk;
mod tests_wire;
