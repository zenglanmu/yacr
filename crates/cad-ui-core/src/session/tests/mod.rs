//! Session unit tests.
//!
//! The test gate lives on the single `mod tests;` declaration in the
//! parent `session` module, so this module and its children are test-only. Each
//! child reaches the session items through `super::super::*` and the shared
//! fixtures through `super::test_support::*`.

mod test_support;
mod tests_align;
mod tests_array;
mod tests_command_log;
mod tests_commands;
mod tests_explode;
mod tests_hatch;
mod tests_hover;
mod tests_input;
mod tests_settings;
mod tests_stretch;
mod tests_text;
mod tests_tools;
