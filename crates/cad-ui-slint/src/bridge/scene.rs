//! Database-derived scene assembly, independent of the Slint/GPU lifecycle.

use std::sync::Arc;

use cad_app::layers::LayerOverrideSet;
use cad_app::AnnotationVisibilitySet;
use cad_db::{AnnotationDatabase, DrawingDatabase};
use cad_domain::{CadResult, DocumentId, TaskStamp, TolerancePolicy};
use cad_representation::layout::{build_paper_space, enumerate_layouts};
use cad_representation::{
    FontEngine, LayoutDescriptor, ProviderRegistry, RepresentationContext, SpaceSelection,
};
use cad_scene::{
    annotation_batches, AnnotationScene, AnnotationSceneOptions, FrameBudget, SceneBudget,
    SceneCache, SceneDelta,
};

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
        RepresentationContext::new(DocumentId(0), TolerancePolicy::default(), stamp.clone());
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
            for entity in cad_app::layers::visible_model_entities(database, overrides) {
                let representation = registry.build_expanded(database, entity, &context)?;
                let delta = cache.build(&representation, stamp.clone())?;
                combined.added.extend(delta.added);
            }
        }
        SpaceSelection::Paper(layout) => {
            let visible = |entity: &cad_db::DbEntity| overrides.is_entity_visible(database, entity);
            let representation =
                build_paper_space(&registry, database, layout, &context, &visible)?;
            let delta = cache.build(&representation, stamp.clone())?;
            combined.added.extend(delta.added);
        }
    }
    Ok(combined)
}

/// Database layouts with drawability verdicts; model space is always available.
pub fn layout_descriptors(database: &DrawingDatabase) -> Vec<LayoutDescriptor> {
    enumerate_layouts(database)
}

/// Drawing and annotation overlay share one delta and one renderer upload.
/// An absent sidecar contributes an empty overlay, not fake annotation data.
pub fn build_scene_with_annotations(
    database: &DrawingDatabase,
    stamp: TaskStamp,
    fonts: Option<Arc<FontEngine>>,
    overrides: &LayerOverrideSet,
    annotations: Option<&AnnotationDatabase>,
    visibility: &AnnotationVisibilitySet,
    annotation_document: DocumentId,
) -> CadResult<(SceneDelta, AnnotationScene)> {
    build_scene_with_annotations_in_space(
        database,
        stamp,
        fonts,
        overrides,
        SpaceSelection::Model,
        annotations,
        visibility,
        annotation_document,
    )
}

/// Explicit model/paper space plus annotation overlay and conversion diagnostics.
#[allow(clippy::too_many_arguments)]
pub fn build_scene_with_annotations_in_space(
    database: &DrawingDatabase,
    stamp: TaskStamp,
    fonts: Option<Arc<FontEngine>>,
    overrides: &LayerOverrideSet,
    space: SpaceSelection,
    annotations: Option<&AnnotationDatabase>,
    visibility: &AnnotationVisibilitySet,
    annotation_document: DocumentId,
) -> CadResult<(SceneDelta, AnnotationScene)> {
    let mut delta =
        build_scene_with_space(database, stamp.clone(), fonts.clone(), overrides, space)?;
    let budget = FrameBudget::from_scene(&SceneBudget::default());
    let options = AnnotationSceneOptions {
        document: annotation_document,
        fonts: fonts.as_deref(),
        budget,
        ..AnnotationSceneOptions::default()
    };
    let annotation_scene = match annotations {
        Some(db) => annotation_batches(db.annotations(), |id| visibility.effective(id), &options),
        None => annotation_batches(std::iter::empty(), |_| false, &options),
    };
    delta.added.extend(annotation_scene.batches.iter().cloned());
    Ok((delta, annotation_scene))
}

/// Revision and visibility both invalidate the derived annotation batches.
pub fn annotation_fingerprint(
    annotations: Option<&AnnotationDatabase>,
    visibility: &AnnotationVisibilitySet,
) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    match annotations {
        Some(db) => {
            db.id().0.hash(&mut hasher);
            db.revision().0.hash(&mut hasher);
            db.len().hash(&mut hasher);
        }
        None => 0u8.hash(&mut hasher),
    }
    for (id, visible) in visibility.iter() {
        id.0.hash(&mut hasher);
        visible.hash(&mut hasher);
    }
    hasher.finish()
}
