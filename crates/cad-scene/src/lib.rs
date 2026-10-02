//! CPU scene cache, separate from authoritative data and device resources.
//!
//! Spec v2.0 §4.8, §8.1, §8.4: display representations become chunked render
//! batches. Batches are keyed so a change invalidates only the affected chunks,
//! and vertices are stored relative to a per-batch local origin so `f32` keeps
//! precision at large coordinates.

use cad_db::{ChangeSet, ObjectChange};
use cad_domain::*;
use cad_representation::{DisplayPrimitive, DisplayRepresentation};
use std::collections::BTreeMap;

/// Re-exported so a host (and the renderer's tests) can construct a batch's
/// linetype without depending on `cad-db` directly.
pub use cad_db::LinetypePattern;

pub mod annotations;
pub mod highlight;

pub use annotations::{
    all_visible, annotation_batches, annotation_geometry_source, conversion_supported,
    tessellate_ellipse, AnnotationScene, AnnotationSceneOptions, DEFAULT_ANNOTATION_FONT,
};
pub use highlight::{
    highlight_batches, HighlightOptions, HighlightScene, DEFAULT_HIGHLIGHT_ALPHA,
    DEFAULT_HIGHLIGHT_COLOR, HIGHLIGHT_DRAW_ORDER,
};

mod batch;
mod budget;
mod cache;

pub use batch::*;
pub use budget::*;
pub use cache::*;

#[cfg(test)]
mod tests;
