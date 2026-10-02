//! CPU scene preparation. No Slint handles, GPU resources or render callbacks.
use crate::layers::LayerOverrideSet;
use crate::AnnotationVisibilitySet;
use cad_db::{AnnotationDatabase, DrawingDatabase};
use cad_domain::{CadResult, DocumentId, SceneIdentity, TaskStamp};
use cad_representation::{FontEngine, SpaceSelection};
use std::sync::Arc;
mod assembly;
pub use assembly::*;
use cad_scene::{annotation_batches, AnnotationSceneOptions, FrameBudget, SceneBudget, SceneDelta};

#[derive(Clone, Debug, PartialEq)]
struct SceneVersion {
    identity: SceneIdentity,
    document: DocumentId,
    fonts: u64,
    layers: u64,
    space: SpaceSelection,
    document_epoch: u64,
}

pub struct PreparedScene {
    pub revision: u64,
    pub base_revision: u64,
    pub base: Arc<SceneDelta>,
    pub overlay: Arc<SceneDelta>,
    pub annotation_diagnostics: Vec<cad_domain::Diagnostic>,
    pub annotation_completeness: cad_domain::Completeness,
}

#[derive(Default)]
pub struct CadSceneController {
    version: Option<SceneVersion>,
    overlay_version: Option<(u64, u64, u64)>,
    annotations_revision: u64,
    pub fonts_revision: u64,
    revision: u64,
    base_revision: u64,
    base: Option<Arc<SceneDelta>>,
    shared_document: Option<(Arc<DrawingDatabase>, SceneIdentity)>,
    document_epoch: u64,
    pub ready: Option<PreparedScene>,
    pub diagnostic: Option<String>,
}

impl CadSceneController {
    /// Immutable database adoption avoids an O(n) bounds/identity walk on camera
    /// navigation. Retaining the Arc prevents pointer reuse from hiding a new
    /// open, even when database id/revision/bounds happen to match.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare_shared(
        &mut self,
        doc: Option<Arc<DrawingDatabase>>,
        document: DocumentId,
        fonts: Option<Arc<FontEngine>>,
        layers: &LayerOverrideSet,
        space: SpaceSelection,
        annotations: Option<&AnnotationDatabase>,
        visibility: &AnnotationVisibilitySet,
    ) -> CadResult<()> {
        let same = match (&self.shared_document, &doc) {
            (Some((old, _)), Some(new)) => Arc::ptr_eq(old, new),
            (None, None) => true,
            _ => false,
        };
        if !same {
            self.document_epoch += 1;
            self.shared_document = doc.as_ref().map(|db| (db.clone(), db.scene_identity()));
        }
        self.prepare(
            doc.as_deref(),
            document,
            fonts,
            layers,
            space,
            annotations,
            visibility,
        )
    }
    pub fn fonts_changed(&mut self) {
        self.fonts_revision += 1;
    }
    pub fn annotations_changed(&mut self) {
        self.annotations_revision += 1;
    }

    #[allow(clippy::too_many_arguments)]
    pub fn prepare(
        &mut self,
        doc: Option<&DrawingDatabase>,
        document: DocumentId,
        fonts: Option<Arc<FontEngine>>,
        layers: &LayerOverrideSet,
        space: SpaceSelection,
        annotations: Option<&AnnotationDatabase>,
        visibility: &AnnotationVisibilitySet,
    ) -> CadResult<()> {
        let result =
            self.prepare_inner(doc, document, fonts, layers, space, annotations, visibility);
        self.diagnostic = result.as_ref().err().map(ToString::to_string);
        result
    }

    #[allow(clippy::too_many_arguments)]
    fn prepare_inner(
        &mut self,
        doc: Option<&DrawingDatabase>,
        document: DocumentId,
        fonts: Option<Arc<FontEngine>>,
        layers: &LayerOverrideSet,
        space: SpaceSelection,
        annotations: Option<&AnnotationDatabase>,
        visibility: &AnnotationVisibilitySet,
    ) -> CadResult<()> {
        let version = doc.map(|db| SceneVersion {
            identity: self
                .shared_document
                .as_ref()
                .filter(|(cached, _)| std::ptr::eq(cached.as_ref(), db))
                .map(|(_, identity)| *identity)
                .unwrap_or_else(|| db.scene_identity()),
            document,
            fonts: self.fonts_revision,
            layers: layers.fingerprint(),
            space,
            document_epoch: self.document_epoch,
        });
        if let Some(db) = doc {
            crate::validate_space(db, space)?;
        }
        let base_changed = self.base.is_none() || self.version != version;
        let next_base = self.base_revision + u64::from(base_changed);
        let overlay_version = (
            next_base,
            annotation_fingerprint(annotations, visibility),
            self.annotations_revision,
        );
        if !base_changed && self.overlay_version == Some(overlay_version) {
            return Ok(());
        }
        let mut stamp = TaskStamp::new(document, self.revision + 1);
        stamp.configuration_version = self.fonts_revision;
        if let Some(db) = doc {
            stamp.object_revision = db.revision();
        }
        // Stage both CPU results before publishing either version. An overlay
        // edit keeps the base Arc and its GPU revision unchanged.
        let base = if base_changed {
            Arc::new(match doc {
                Some(db) => {
                    build_scene_with_space(db, stamp.clone(), fonts.clone(), layers, space)?
                }
                None => SceneDelta {
                    stamp: stamp.clone(),
                    added: vec![],
                    removed_chunks: vec![],
                },
            })
        } else {
            self.base.as_ref().expect("base prepared").clone()
        };
        let options = AnnotationSceneOptions {
            document,
            fonts: fonts.as_deref(),
            budget: FrameBudget::from_scene(&SceneBudget::default()),
            ..AnnotationSceneOptions::default()
        };
        let overlay = match (doc, annotations) {
            (Some(_), Some(db)) => {
                annotation_batches(db.annotations(), |id| visibility.effective(id), &options)
            }
            _ => annotation_batches(std::iter::empty(), |_| false, &options),
        };
        // Conversion gaps remain explicit; do not claim an empty successful overlay.
        let delta = Arc::new(SceneDelta {
            stamp,
            added: overlay.batches,
            removed_chunks: vec![],
        });
        self.revision += 1;
        self.base_revision = next_base;
        self.version = version;
        self.overlay_version = Some(overlay_version);
        self.base = Some(base.clone());
        self.ready = Some(PreparedScene {
            revision: self.revision,
            base_revision: next_base,
            base,
            overlay: delta,
            annotation_diagnostics: overlay.diagnostics,
            annotation_completeness: overlay.completeness,
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cad_db::DrawingDatabaseBuilder;
    use cad_domain::{DatabaseId, LayoutId};

    fn prepare(
        controller: &mut CadSceneController,
        db: Option<&DrawingDatabase>,
        fonts: Option<Arc<FontEngine>>,
        annotations: Option<&AnnotationDatabase>,
    ) -> CadResult<()> {
        controller.prepare(
            db,
            DocumentId(73),
            fonts,
            &LayerOverrideSet::new(),
            SpaceSelection::Model,
            annotations,
            &AnnotationVisibilitySet::new(),
        )
    }

    #[test]
    fn fonts_some_to_some_invalidates_and_overlay_does_not_rebuild_base() {
        let db = DrawingDatabaseBuilder::new(DatabaseId(9)).finish().unwrap();
        let mut controller = CadSceneController::default();
        let first_fonts = Arc::new(FontEngine::new());
        controller.fonts_changed();
        prepare(&mut controller, Some(&db), Some(first_fonts.clone()), None).unwrap();
        let first = controller.ready.as_ref().unwrap();
        assert_eq!(first.base.stamp.document, DocumentId(73));
        assert_eq!(first.base.stamp.object_revision, db.revision());
        let base = first.base.clone();
        let base_revision = first.base_revision;
        let revision = first.revision;
        prepare(&mut controller, Some(&db), Some(first_fonts.clone()), None).unwrap();
        assert_eq!(controller.ready.as_ref().unwrap().revision, revision);
        let annotations = AnnotationDatabase::new(DatabaseId(3));
        prepare(
            &mut controller,
            Some(&db),
            Some(first_fonts),
            Some(&annotations),
        )
        .unwrap();
        assert!(Arc::ptr_eq(&base, &controller.ready.as_ref().unwrap().base));
        assert_eq!(
            controller.ready.as_ref().unwrap().base_revision,
            base_revision
        );
        controller.fonts_changed();
        prepare(
            &mut controller,
            Some(&db),
            Some(Arc::new(FontEngine::new())),
            Some(&annotations),
        )
        .unwrap();
        assert!(controller.ready.as_ref().unwrap().base_revision > base_revision);
    }

    #[test]
    fn failed_build_keeps_previous_version_and_close_publishes_empty_scene() {
        let db = DrawingDatabaseBuilder::new(DatabaseId(9)).finish().unwrap();
        let mut controller = CadSceneController::default();
        prepare(&mut controller, Some(&db), None, None).unwrap();
        let previous = controller.ready.as_ref().unwrap().revision;
        assert!(controller
            .prepare(
                Some(&db),
                DocumentId(73),
                None,
                &LayerOverrideSet::new(),
                SpaceSelection::Paper(LayoutId(999)),
                None,
                &AnnotationVisibilitySet::new()
            )
            .is_err());
        assert_eq!(controller.ready.as_ref().unwrap().revision, previous);
        assert!(controller.diagnostic.is_some());
        prepare(&mut controller, None, None, None).unwrap();
        let closed = controller.ready.as_ref().unwrap();
        assert!(closed.base.added.is_empty() && closed.overlay.added.is_empty());
        assert!(closed.revision > previous);
        assert!(controller.diagnostic.is_none());
    }

    #[test]
    fn shared_adoption_distinguishes_opens_even_with_equal_database_identity() {
        let first = Arc::new(DrawingDatabaseBuilder::new(DatabaseId(8)).finish().unwrap());
        let second = Arc::new((*first).clone());
        let mut controller = CadSceneController::default();
        let layers = LayerOverrideSet::new();
        let visibility = AnnotationVisibilitySet::new();
        controller
            .prepare_shared(
                Some(first.clone()),
                DocumentId(1),
                None,
                &layers,
                SpaceSelection::Model,
                None,
                &visibility,
            )
            .unwrap();
        let revision = controller.ready.as_ref().unwrap().base_revision;
        controller
            .prepare_shared(
                Some(first),
                DocumentId(1),
                None,
                &layers,
                SpaceSelection::Model,
                None,
                &visibility,
            )
            .unwrap();
        assert_eq!(controller.ready.as_ref().unwrap().base_revision, revision);
        controller
            .prepare_shared(
                Some(second),
                DocumentId(1),
                None,
                &layers,
                SpaceSelection::Model,
                None,
                &visibility,
            )
            .unwrap();
        assert!(controller.ready.as_ref().unwrap().base_revision > revision);
    }
}
