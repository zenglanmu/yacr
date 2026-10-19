//! ops module.
//!
//! Decomposed from a single oversized file into cohesive submodules. Every
//! pre-split `pub(crate)` item is re-exported here with the same `cfg` as its
//! definition, so each `crate::ops::X` path resolves under both the native host
//! and `wasm32` exactly as it did before the split.

use super::*;

#[cfg(not(target_arch = "wasm32"))]
mod benchmark;
mod inspect;
#[cfg(not(target_arch = "wasm32"))]
mod render;
mod resources;

#[cfg(not(target_arch = "wasm32"))]
pub(crate) use benchmark::run_benchmark;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use inspect::enforce_scene_budget;
pub(crate) use inspect::{run_build_representation, run_measure, run_proxy_report, run_scan};
// `representation_context` is unconditional but is only named from the
// native-only `render`/`benchmark` modules; the re-export is kept so
// `crate::ops::representation_context` still resolves on both targets.
#[allow(unused_imports)]
pub(crate) use inspect::representation_context;
// `run_plot_png` and `synthetic_plot_database` are `pub(crate)` items used only
// within the `render` module; re-exported to preserve the pre-split surface.
#[cfg(not(target_arch = "wasm32"))]
#[allow(unused_imports)]
pub(crate) use render::{run_plot, run_plot_png, run_render, synthetic_plot_database};
pub(crate) use resources::load_fonts;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use resources::{drawing_needs_text, fonts_requested, images_requested, load_images};
