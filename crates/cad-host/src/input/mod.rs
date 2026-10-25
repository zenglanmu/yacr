//! UI-independent interaction dispatch and coordinate conversion.
//!
//! The implementation is partitioned into cohesive submodules; the path
//! `cad_host::input::…` is preserved (the submit-admission helper is re-exported
//! from here).
use super::*;

mod grip_drag;
mod hover;
mod interaction;
mod pointer;
mod queue;

pub(crate) use queue::admissible_for_submit;
