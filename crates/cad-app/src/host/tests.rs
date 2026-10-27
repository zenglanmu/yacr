//! Host controller contract tests, grouped by theme.
//!
//! The shared imports below are re-globbed by each child module, so the theme
//! modules (`tools`, `document`, `status`, `snap`, `toggles`) all see the same
//! host types, `p(...)` point helper and snap-cache probe without duplication.

use super::databases::{p, snap_target_build_count};
use super::*;

mod align;
mod array;
mod chamfer_fillet;
mod clipboard;
mod dimension;
mod document;
mod draw_pick;
mod explode;
mod mtext;
mod ortho;
mod polygon;
mod properties;
mod snap;
mod snap_grid;
mod status;
mod stretch;
mod text_edit;
mod toggles;
mod tool_parameter;
mod tools;
