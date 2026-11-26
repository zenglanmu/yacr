//! UI-independent interaction dispatch and coordinate conversion.
//!
//! The implementation is partitioned into cohesive submodules; the path
//! `cad_host::input::…` is preserved (the submit-admission helper is re-exported
//! from here).
use super::*;

mod face_tool;
mod grip_drag;
mod hover;
mod interaction;
mod lasso;
mod mesh_target;
mod pointer;
mod queue;

pub(crate) use face_tool::solid_face_tool;
pub(crate) use mesh_target::mesh_face_edit_target;
pub(crate) use queue::admissible_for_submit;
