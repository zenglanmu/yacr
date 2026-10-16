//! Database-derived scene assembly, independent of the UI/GPU lifecycle.

use std::sync::Arc;

use crate::layers::LayerOverrideSet;
use cad_db::DrawingDatabase;
use cad_domain::{CadResult, TaskStamp, TolerancePolicy};
use cad_representation::layout::{build_paper_space, enumerate_layouts};
use cad_representation::{
    FontEngine, LayoutDescriptor, ProviderRegistry, RepresentationContext, SpaceSelection,
};
use cad_resources::DecodedImageCache;
use cad_scene::{RenderBatch, RenderTopology, SceneBudget, SceneCache, SceneDelta};

pub fn build_scene(database: &DrawingDatabase, stamp: TaskStamp) -> CadResult<SceneDelta> {
    build_scene_with_fonts(database, stamp, None)
}

/// Read-only counts describing how much the scene builder merged.
///
/// `fragments_in` is the display-fragment count the representation stage
/// produced (before merging); `batches_out` is the number of draw items the
/// scene emitted. Both come from the same build that produces the drawn
/// [`SceneDelta`], so the ratio is the real adjacency-merge yield, not an
/// estimate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SceneAssemblyStats {
    /// Display fragments the representation stage produced, before merging.
    pub fragments_in: usize,
    /// Draw items emitted: render batches plus image draw items (one GPU draw
    /// each).
    pub batches_out: usize,
    /// Adjacent emitted render batches that share every draw-relevant style
    /// attribute (alpha, colour, lineweight, linetype and their unresolved
    /// flags). Counted across the whole flat model-space stream, so it includes
    /// boundaries between entities that the per-entity merge cannot join; it is
    /// an upper bound on the extra adjacency a stronger (cross-representation)
    /// merge predicate could exploit.
    pub adjacent_same_style: usize,
    /// Adjacent style- and topology-class-equal render batches that differ only
    /// by paint order (`draw_order`). Zero until `draw_order` threading lands.
    pub adjacent_differing_by_draw_order: usize,
    /// Adjacent render batches that share style and paint order but are split by
    /// line-vs-mesh topology (one mesh-class, one line-class).
    pub adjacent_differing_by_topology: usize,
    /// Adjacent line batches that satisfy every gate the merge predicate applies
    /// (style, paint order, combined vertex cap, 8,192-unit local extent) yet
    /// stayed separate because they came from different representations — the
    /// realizable draw reduction a representation-spanning line merge would get.
    pub adjacent_mergeable_line_pairs: usize,
}

impl SceneAssemblyStats {
    /// Fragments per emitted draw item. `0.0` when nothing was emitted.
    pub fn merge_ratio(&self) -> f64 {
        if self.batches_out == 0 {
            0.0
        } else {
            self.fragments_in as f64 / self.batches_out as f64
        }
    }
}

/// Build scene batches, shaping text with `fonts` when supplied.
pub fn build_scene_with_fonts(
    database: &DrawingDatabase,
    stamp: TaskStamp,
    fonts: Option<Arc<FontEngine>>,
) -> CadResult<SceneDelta> {
    build_scene_with_overrides(database, stamp, fonts, &LayerOverrideSet::new())
}

/// Temporary layer visibility changes batches without reparsing the database.
pub fn build_scene_with_overrides(
    database: &DrawingDatabase,
    stamp: TaskStamp,
    fonts: Option<Arc<FontEngine>>,
    overrides: &LayerOverrideSet,
) -> CadResult<SceneDelta> {
    build_scene_with_space(database, stamp, fonts, overrides, SpaceSelection::Model)
}

/// Build model geometry or a layout's paper geometry and clipped viewports.
/// Inspect [`layout_descriptors`] for per-layout support diagnostics.
pub fn build_scene_with_space(
    database: &DrawingDatabase,
    stamp: TaskStamp,
    fonts: Option<Arc<FontEngine>>,
    overrides: &LayerOverrideSet,
    space: SpaceSelection,
) -> CadResult<SceneDelta> {
    build_scene_with_space_and_images(database, stamp, fonts, None, overrides, space)
}

/// Like [`build_scene_with_space`] but also feeds decoded raster images.
///
/// When `images` is `Some`, an `Image` whose key is present in the cache becomes
/// a texture primitive (`DisplayPrimitive::Image`) instead of an outline-only
/// frame; a key absent from the cache still draws the frame plus the provider's
/// honest `image.*` diagnostic. Passing `None` is exactly the previous
/// behaviour, so existing callers are unaffected.
#[allow(clippy::too_many_arguments)]
pub fn build_scene_with_space_and_images(
    database: &DrawingDatabase,
    stamp: TaskStamp,
    fonts: Option<Arc<FontEngine>>,
    images: Option<Arc<DecodedImageCache>>,
    overrides: &LayerOverrideSet,
    space: SpaceSelection,
) -> CadResult<SceneDelta> {
    build_scene_stats(database, stamp, fonts, images, overrides, space).map(|(delta, _stats)| delta)
}

/// Like [`build_scene_with_space_and_images`] but also returns read-only
/// [`SceneAssemblyStats`] (fragments in vs. draw items out). Purely additive
/// reporting: the returned [`SceneDelta`] is byte-for-byte the same as the
/// reporting-free call.
#[allow(clippy::too_many_arguments)]
pub fn build_scene_stats(
    database: &DrawingDatabase,
    stamp: TaskStamp,
    fonts: Option<Arc<FontEngine>>,
    images: Option<Arc<DecodedImageCache>>,
    overrides: &LayerOverrideSet,
    space: SpaceSelection,
) -> CadResult<(SceneDelta, SceneAssemblyStats)> {
    let registry = ProviderRegistry::with_default_provider();
    let mut context =
        RepresentationContext::new(stamp.document, TolerancePolicy::default(), stamp.clone())
            .with_packed_line_segments();
    if let Some(fonts) = fonts {
        context = context.with_fonts(fonts);
    }
    if let Some(images) = images {
        context = context.with_images(images);
    }
    let mut cache = SceneCache::new(SceneBudget::default());
    let mut combined = SceneDelta {
        stamp: stamp.clone(),
        added: Vec::new(),
        removed_chunks: Vec::new(),
        images: Vec::new(),
    };
    let mut stats = SceneAssemblyStats::default();
    match space {
        SpaceSelection::Model => {
            for entity in crate::layers::visible_model_entities(database, overrides) {
                let representation = registry.build_expanded(database, entity, &context)?;
                stats.fragments_in += representation.fragments.len();
                let delta = cache.build_compact(&representation, stamp.clone())?;
                stats.batches_out += delta.added.len() + delta.images.len();
                combined.added.extend(delta.added);
                combined.images.extend(delta.images);
            }
        }
        SpaceSelection::Paper(layout) => {
            let visible = |entity: &cad_db::DbEntity| overrides.is_entity_visible(database, entity);
            let representation =
                build_paper_space(&registry, database, layout, &context, &visible)?;
            stats.fragments_in += representation.fragments.len();
            let delta = cache.build_compact(&representation, stamp.clone())?;
            stats.batches_out += delta.added.len() + delta.images.len();
            combined.added.extend(delta.added);
            combined.images.extend(delta.images);
        }
    }
    tally_adjacent_merge_candidates(&combined.added, &mut stats);
    Ok((combined, stats))
}

/// Read-only merge-ratio measurement for the model-space scene of `database`.
///
/// Builds the model-space batches with a throwaway cache (no fonts, no layer
/// overrides, no decoded images) — the same compaction path the host open draws
/// — and returns the fragment/batch counters. It never touches live scene state
/// and does not change what the host draws.
pub fn model_scene_merge_stats(
    database: &DrawingDatabase,
    stamp: TaskStamp,
) -> CadResult<SceneAssemblyStats> {
    let (_delta, stats) = build_scene_stats(
        database,
        stamp,
        None,
        None,
        &LayerOverrideSet::new(),
        SpaceSelection::Model,
    )?;
    Ok(stats)
}

/// Whether two render batches share every draw-relevant style attribute, so an
/// order- and topology-agnostic merge predicate could in principle join them.
fn same_render_style(a: &RenderBatch, b: &RenderBatch) -> bool {
    a.alpha == b.alpha
        && a.color == b.color
        && a.color_unresolved == b.color_unresolved
        && a.lineweight == b.lineweight
        && a.lineweight_unresolved == b.lineweight_unresolved
        && a.linetype == b.linetype
        && a.linetype_unresolved == b.linetype_unresolved
}

/// Whether the successor line batch `b` could be absorbed into the adjacent line
/// batch `a` by a representation-spanning merge: the combined vertex count stays
/// under the builder's 65,536 cap and every `b` vertex lies within the builder's
/// 8,192-unit local extent of `a`'s origin.
///
/// This is exactly the gate the within-representation predicate applies to a
/// successor fragment, so [`SceneAssemblyStats::adjacent_mergeable_line_pairs`]
/// is a realizable reduction rather than a loose style-only bound.
fn line_merge_extent_ok(a: &RenderBatch, b: &RenderBatch) -> bool {
    a.topology == RenderTopology::Lines
        && b.topology == RenderTopology::Lines
        // The predicate only merges opaque line batches (a transparent batch
        // keeps its own centroid-based depth key).
        && a.alpha == 1.0
        && a.vertices.len().saturating_add(b.vertices.len()) <= 65_536
        && b.vertices.iter().all(|v| {
            [
                b.local_origin.x + v[0] as f64 - a.local_origin.x,
                b.local_origin.y + v[1] as f64 - a.local_origin.y,
                b.local_origin.z + v[2] as f64 - a.local_origin.z,
            ]
            .iter()
            .all(|d| d.is_finite() && d.abs() <= 8192.0)
        })
}

/// Tally the adjacency hints in [`SceneAssemblyStats`] over adjacent emitted
/// batches. Each mergeable pair a representation-spanning merge joins removes
/// exactly one draw item, so the mergeable count is the draw reduction.
fn tally_adjacent_merge_candidates(batches: &[RenderBatch], stats: &mut SceneAssemblyStats) {
    for pair in batches.windows(2) {
        let (a, b) = (&pair[0], &pair[1]);
        if !same_render_style(a, b) {
            continue;
        }
        stats.adjacent_same_style += 1;
        if a.draw_order == b.draw_order {
            if a.topology != b.topology {
                stats.adjacent_differing_by_topology += 1;
            } else if line_merge_extent_ok(a, b) {
                stats.adjacent_mergeable_line_pairs += 1;
            }
        } else if a.topology == b.topology {
            stats.adjacent_differing_by_draw_order += 1;
        }
    }
}

/// Database layouts with drawability verdicts; model space is always available.
pub fn layout_descriptors(database: &DrawingDatabase) -> Vec<LayoutDescriptor> {
    enumerate_layouts(database)
}
