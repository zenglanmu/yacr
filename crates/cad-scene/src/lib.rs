//! CPU scene cache, separate from authoritative data and device resources.
//!
//! Spec v2.0 §4.8, §8.1, §8.4: display representations become render batches.
//! A build produces one [`SceneDelta`]; [`SceneCache::publish`] only **appends**
//! its batches (plus images/masks) under a fresh chunk key. There is no per-chunk
//! invalidation or removal channel: removing content is the host's job — the
//! retire path suppresses a deleted entity's resident batches with `FLAG_HIDDEN`
//! and a full rebuild compacts them (`cad-app/src/render_scene/retire.rs`), while
//! this cache evicts oldest chunks to stay within `SceneBudget::cpu_bytes`.
//! Vertices are stored relative to a per-batch local origin so `f32` keeps
//! precision at large coordinates.

use cad_domain::*;
use cad_representation::{DisplayPrimitive, DisplayRepresentation, ImageVertex};
use cad_resources::ResourceKey;
use std::collections::BTreeMap;

/// Re-exported so a host (and the renderer's tests) can construct a batch's
/// linetype without depending on `cad-db` directly.
pub use cad_db::LinetypePattern;

pub mod highlight;

pub use highlight::{
    highlight_batches, HighlightOptions, HighlightScene, DEFAULT_HIGHLIGHT_ALPHA,
    DEFAULT_HIGHLIGHT_COLOR, FLAG_HIDDEN, FLAG_HOVERED, FLAG_SELECTED, HIGHLIGHT_DRAW_ORDER,
};

mod batch;
mod budget;
mod cache;
mod transient;

pub use batch::*;
pub use budget::*;
pub use cache::*;
pub use transient::*;

#[cfg(test)]
mod tests;
