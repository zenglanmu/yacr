//! Database-derived scene assembly, independent of the Slint/GPU lifecycle.

use std::sync::Arc;

use crate::layers::LayerOverrideSet;
use cad_db::DrawingDatabase;
use cad_domain::{CadResult, TaskStamp, TolerancePolicy};
use cad_representation::layout::{build_paper_space, enumerate_layouts};
use cad_representation::{
    FontEngine, LayoutDescriptor, ProviderRegistry, RepresentationContext, SpaceSelection,
};
use cad_scene::{SceneBudget, SceneCache, SceneDelta};

pub fn build_scene(database: &DrawingDatabase, stamp: TaskStamp) -> CadResult<SceneDelta> {
    build_scene_with_fonts(database, stamp, None)
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
    let registry = ProviderRegistry::with_default_provider();
    let mut context =
        RepresentationContext::new(stamp.document, TolerancePolicy::default(), stamp.clone())
            .with_packed_line_segments();
    if let Some(fonts) = fonts {
        context = context.with_fonts(fonts);
    }
    let mut cache = SceneCache::new(SceneBudget::default());
    let mut combined = SceneDelta {
        stamp: stamp.clone(),
        added: Vec::new(),
        removed_chunks: Vec::new(),
    };
    match space {
        SpaceSelection::Model => {
            for entity in crate::layers::visible_model_entities(database, overrides) {
                let representation = registry.build_expanded(database, entity, &context)?;
                let delta = cache.build_compact(&representation, stamp.clone())?;
                combined.added.extend(delta.added);
            }
        }
        SpaceSelection::Paper(layout) => {
            let visible = |entity: &cad_db::DbEntity| overrides.is_entity_visible(database, entity);
            let representation =
                build_paper_space(&registry, database, layout, &context, &visible)?;
            let delta = cache.build_compact(&representation, stamp.clone())?;
            combined.added.extend(delta.added);
        }
    }
    Ok(combined)
}

/// Database layouts with drawability verdicts; model space is always available.
pub fn layout_descriptors(database: &DrawingDatabase) -> Vec<LayoutDescriptor> {
    enumerate_layouts(database)
}
