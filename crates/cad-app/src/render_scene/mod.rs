//! CPU scene preparation. No Slint handles, GPU resources or render callbacks.
use crate::layers::LayerOverrideSet;
use cad_db::DrawingDatabase;
use cad_domain::{CadResult, DocumentId, SceneIdentity, TaskStamp};
use cad_representation::{FontEngine, SpaceSelection};
use cad_scene::SceneDelta;
use std::sync::Arc;
mod assembly;
pub use assembly::*;
mod overlay;
pub use overlay::*;

#[derive(Clone, Debug, PartialEq)]
struct SceneVersion {
    identity: SceneIdentity,
    document: DocumentId,
    fonts: u64,
    layers: u64,
    space: SpaceSelection,
    document_epoch: u64,
}

#[derive(Clone)]
pub struct PreparedScene {
    pub revision: u64,
    pub base_revision: u64,
    pub base: Arc<SceneDelta>,
    /// Transient selection-highlight and tool-preview batches.
    pub highlight: Arc<SceneDelta>,
    pub highlight_diagnostics: Vec<cad_domain::Diagnostic>,
    pub highlight_completeness: cad_domain::Completeness,
}

#[derive(Clone, Default)]
pub struct CadSceneController {
    version: Option<SceneVersion>,
    /// `(base_revision, transient overlay fingerprint)`.
    visual_version: Option<(u64, u64)>,
    pub fonts_revision: u64,
    revision: u64,
    base_revision: u64,
    base: Option<Arc<SceneDelta>>,
    visual_overlay: Option<Arc<SceneDelta>>,
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
    ) -> CadResult<()> {
        self.prepare_shared_with_overlays(
            doc,
            document,
            fonts,
            layers,
            space,
            &OverlayInputs::default(),
        )
    }

    /// Like [`Self::prepare_shared`] but also rebuilds the transient selection
    /// highlight / tool-preview overlay.
    ///
    /// The base drawing `Arc` is reused unless its own inputs changed, so a
    /// selection or preview-cursor change only rebuilds the highlight batches.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare_shared_with_overlays(
        &mut self,
        doc: Option<Arc<DrawingDatabase>>,
        document: DocumentId,
        fonts: Option<Arc<FontEngine>>,
        layers: &LayerOverrideSet,
        space: SpaceSelection,
        overlays: &OverlayInputs,
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
        self.prepare_with_overlays_doc(doc.as_deref(), document, fonts, layers, space, overlays)
    }
    pub fn fonts_changed(&mut self) {
        self.fonts_revision += 1;
    }

    #[allow(clippy::too_many_arguments)]
    pub fn prepare(
        &mut self,
        doc: Option<&DrawingDatabase>,
        document: DocumentId,
        fonts: Option<Arc<FontEngine>>,
        layers: &LayerOverrideSet,
        space: SpaceSelection,
    ) -> CadResult<()> {
        self.prepare_with_overlays(
            doc,
            document,
            fonts,
            layers,
            space,
            &OverlayInputs::default(),
        )
    }

    /// [`Self::prepare`] plus transient highlight / preview overlay inputs.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare_with_overlays(
        &mut self,
        doc: Option<&DrawingDatabase>,
        document: DocumentId,
        fonts: Option<Arc<FontEngine>>,
        layers: &LayerOverrideSet,
        space: SpaceSelection,
        overlays: &OverlayInputs,
    ) -> CadResult<()> {
        self.prepare_with_overlays_doc(doc, document, fonts, layers, space, overlays)
    }

    #[allow(clippy::too_many_arguments)]
    fn prepare_with_overlays_doc(
        &mut self,
        doc: Option<&DrawingDatabase>,
        document: DocumentId,
        fonts: Option<Arc<FontEngine>>,
        layers: &LayerOverrideSet,
        space: SpaceSelection,
        overlays: &OverlayInputs,
    ) -> CadResult<()> {
        let result = self.prepare_inner(doc, document, fonts, layers, space, overlays);
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
        overlays: &OverlayInputs,
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
        let visual_version = (next_base, overlay_fingerprint(overlays));
        let visual_changed = base_changed || self.visual_version != Some(visual_version);
        if !base_changed && !visual_changed {
            return Ok(());
        }
        let mut stamp = TaskStamp::new(document, self.revision + 1);
        stamp.configuration_version = self.fonts_revision;
        if let Some(db) = doc {
            stamp.object_revision = db.revision();
        }
        // Stage every CPU result before publishing any version. A visual-overlay
        // edit keeps the base `Arc` (and its GPU revisions) unchanged.
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

        let (visual_delta, highlight_diagnostics, highlight_completeness) =
            if visual_changed || self.visual_overlay.is_none() {
                let mut overlay = VisualOverlay::default();
                if let Some(db) = doc {
                    // Reference overlays first, so the selection and previews
                    // draw above the axes/grid (they also carry a higher
                    // draw order, but merge order keeps the batch list readable).
                    if overlays.visibility.axes || overlays.visibility.grid {
                        match db.bounds() {
                            Some(bounds) => {
                                if overlays.visibility.grid {
                                    overlay.merge(grid_overlay(bounds, document));
                                }
                                if overlays.visibility.axes {
                                    overlay.merge(axes_overlay(bounds, document));
                                }
                            }
                            None => overlay.partial(
                                "overlay.bounds-unavailable",
                                "axes/grid requested but the drawing has no model-space bounds; \
                                 omitted"
                                    .into(),
                            ),
                        }
                    }
                    if overlays.visibility.selection_highlight {
                        overlay.merge(selection_highlight(
                            db,
                            &overlays.selection,
                            document,
                            stamp.clone(),
                            fonts.clone(),
                        ));
                    }
                    overlay.merge(preview_overlay(
                        overlays.measurement.as_ref(),
                        document,
                        overlays.visibility.snap_hints,
                        &PreviewOptions::default(),
                    ));
                    if let Some(preview) = overlays.draw_preview.as_ref() {
                        overlay.merge(draw_preview_overlay(
                            preview,
                            document,
                            overlays.visibility.snap_hints,
                            &PreviewOptions::default(),
                        ));
                    }
                    // Resolved object-snap hint markers are a separate concept
                    // from the pointer-cursor crosses above: distinct shapes
                    // encoding the snap kind, at their own draw order. Only
                    // built when the host fed hints in *and* the overlay flag is
                    // on; otherwise no batch, never an empty fabricated one.
                    if overlays.visibility.snap_hints && !overlays.snap_hints_input.is_empty() {
                        overlay.merge(snap_hint_overlay(
                            &overlays.snap_hints_input,
                            document,
                            &PreviewOptions::default(),
                        ));
                    }
                }
                let delta = Arc::new(SceneDelta {
                    stamp: stamp.clone(),
                    added: overlay.batches,
                    removed_chunks: vec![],
                });
                (delta, overlay.diagnostics, overlay.completeness)
            } else {
                self.visual_result()
            };

        self.revision += 1;
        self.base_revision = next_base;
        self.version = version;
        self.visual_version = Some(visual_version);
        self.base = Some(base.clone());
        self.visual_overlay = Some(visual_delta.clone());
        self.ready = Some(PreparedScene {
            revision: self.revision,
            base_revision: next_base,
            base,
            highlight: visual_delta,
            highlight_diagnostics,
            highlight_completeness,
        });
        Ok(())
    }

    /// The cached transient overlay plus its diagnostics/completeness.
    fn visual_result(
        &self,
    ) -> (
        Arc<SceneDelta>,
        Vec<cad_domain::Diagnostic>,
        cad_domain::Completeness,
    ) {
        match (&self.visual_overlay, &self.ready) {
            (Some(delta), Some(ready)) => (
                delta.clone(),
                ready.highlight_diagnostics.clone(),
                ready.highlight_completeness.clone(),
            ),
            (Some(delta), None) => (
                delta.clone(),
                Vec::new(),
                cad_domain::Completeness::Complete,
            ),
            (None, _) => (
                Arc::new(SceneDelta {
                    stamp: TaskStamp::new(DocumentId(0), 0),
                    added: Vec::new(),
                    removed_chunks: Vec::new(),
                }),
                Vec::new(),
                cad_domain::Completeness::Complete,
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::viewer_config::OverlayVisibility;
    use cad_db::DrawingDatabaseBuilder;
    use cad_domain::{DatabaseId, LayoutId};

    /// Overlay inputs with the reference axes/grid off, so a test isolates the
    /// selection/preview overlay from the world-space reference overlays.
    fn overlays_without_reference() -> OverlayInputs {
        OverlayInputs {
            visibility: OverlayVisibility {
                axes: false,
                grid: false,
                ..OverlayVisibility::default()
            },
            ..OverlayInputs::default()
        }
    }

    fn prepare(
        controller: &mut CadSceneController,
        db: Option<&DrawingDatabase>,
        fonts: Option<Arc<FontEngine>>,
    ) -> CadResult<()> {
        controller.prepare(
            db,
            DocumentId(73),
            fonts,
            &LayerOverrideSet::new(),
            SpaceSelection::Model,
        )
    }

    #[test]
    fn fonts_change_rebuilds_the_base_and_a_repeat_is_a_no_op() {
        let db = DrawingDatabaseBuilder::new(DatabaseId(9)).finish().unwrap();
        let mut controller = CadSceneController::default();
        let first_fonts = Arc::new(FontEngine::new());
        controller.fonts_changed();
        prepare(&mut controller, Some(&db), Some(first_fonts.clone())).unwrap();
        let first = controller.ready.as_ref().unwrap();
        assert_eq!(first.base.stamp.document, DocumentId(73));
        assert_eq!(first.base.stamp.object_revision, db.revision());
        let base = first.base.clone();
        let base_revision = first.base_revision;
        let revision = first.revision;
        prepare(&mut controller, Some(&db), Some(first_fonts)).unwrap();
        assert_eq!(controller.ready.as_ref().unwrap().revision, revision);
        assert!(Arc::ptr_eq(&base, &controller.ready.as_ref().unwrap().base));
        controller.fonts_changed();
        prepare(
            &mut controller,
            Some(&db),
            Some(Arc::new(FontEngine::new())),
        )
        .unwrap();
        assert!(controller.ready.as_ref().unwrap().base_revision > base_revision);
    }

    #[test]
    fn failed_build_keeps_previous_version_and_close_publishes_empty_scene() {
        let db = DrawingDatabaseBuilder::new(DatabaseId(9)).finish().unwrap();
        let mut controller = CadSceneController::default();
        prepare(&mut controller, Some(&db), None).unwrap();
        let previous = controller.ready.as_ref().unwrap().revision;
        assert!(controller
            .prepare(
                Some(&db),
                DocumentId(73),
                None,
                &LayerOverrideSet::new(),
                SpaceSelection::Paper(LayoutId(999)),
            )
            .is_err());
        assert_eq!(controller.ready.as_ref().unwrap().revision, previous);
        assert!(controller.diagnostic.is_some());
        prepare(&mut controller, None, None).unwrap();
        let closed = controller.ready.as_ref().unwrap();
        assert!(closed.base.added.is_empty() && closed.highlight.added.is_empty());
        assert!(closed.revision > previous);
        assert!(controller.diagnostic.is_none());
    }

    #[test]
    fn shared_adoption_distinguishes_opens_even_with_equal_database_identity() {
        let first = Arc::new(DrawingDatabaseBuilder::new(DatabaseId(8)).finish().unwrap());
        let second = Arc::new((*first).clone());
        let mut controller = CadSceneController::default();
        let layers = LayerOverrideSet::new();
        controller
            .prepare_shared(
                Some(first.clone()),
                DocumentId(1),
                None,
                &layers,
                SpaceSelection::Model,
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
            )
            .unwrap();
        assert!(controller.ready.as_ref().unwrap().base_revision > revision);
    }

    fn db_with_line() -> DrawingDatabase {
        use cad_db::{DbEntity, DbObject, Layer};
        use cad_domain::{
            EntityId, LayerId, ObjectId, Point3, Revision, SemanticGeometry, SpaceId,
        };
        let mut b = DrawingDatabaseBuilder::new(DatabaseId(9));
        b.insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
        b.insert_entity(DbEntity {
            object: DbObject {
                id: ObjectId(1),
                type_key: "AcDbLine".into(),
                revision: Revision(0),
                source_handle: None,
            },
            id: EntityId(1),
            layer: LayerId(0),
            space: SpaceId::Model,
            geometry: SemanticGeometry::Line {
                start: Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                end: Point3 {
                    x: 5.0,
                    y: 0.0,
                    z: 0.0,
                },
            },
            draw_order: 0,
        })
        .unwrap();
        b.finish().unwrap()
    }

    #[test]
    fn selection_change_rebuilds_only_the_highlight_overlay() {
        use crate::selection::{model_ref, SelectionSet};
        let db = db_with_line();
        let mut controller = CadSceneController::default();
        let layers = LayerOverrideSet::new();
        let empty = overlays_without_reference();
        controller
            .prepare_with_overlays(
                Some(&db),
                DocumentId(73),
                None,
                &layers,
                SpaceSelection::Model,
                &empty,
            )
            .unwrap();
        let first = controller.ready.as_ref().unwrap();
        let base = first.base.clone();
        let base_revision = first.base_revision;
        assert!(first.highlight.added.is_empty());
        assert_eq!(first.highlight_diagnostics.len(), 0);

        // Selecting the line rebuilds only the transient highlight overlay.
        let overlays = OverlayInputs {
            selection: SelectionSet::from_refs([model_ref(
                DocumentId(73),
                cad_domain::EntityId(1),
            )]),
            visibility: OverlayVisibility {
                axes: false,
                grid: false,
                ..OverlayVisibility::default()
            },
            ..OverlayInputs::default()
        };
        controller
            .prepare_with_overlays(
                Some(&db),
                DocumentId(73),
                None,
                &layers,
                SpaceSelection::Model,
                &overlays,
            )
            .unwrap();
        let after = controller.ready.as_ref().unwrap();
        assert!(
            Arc::ptr_eq(&base, &after.base),
            "the base drawing must be reused across a selection change"
        );
        assert_eq!(after.base_revision, base_revision);
        assert_eq!(after.highlight.added.len(), 1);
        assert!(after
            .highlight
            .added
            .iter()
            .all(|b| b.draw_order > 0 && b.draw_order < 1_000_000));
        assert!(after.highlight.added.iter().all(|b| !b.color_unresolved));

        // Rebuilding with the same selection is a no-op (revision unchanged).
        let revision = after.revision;
        let highlight = after.highlight.clone();
        controller
            .prepare_with_overlays(
                Some(&db),
                DocumentId(73),
                None,
                &layers,
                SpaceSelection::Model,
                &overlays,
            )
            .unwrap();
        assert_eq!(controller.ready.as_ref().unwrap().revision, revision);
        assert!(Arc::ptr_eq(
            &highlight,
            &controller.ready.as_ref().unwrap().highlight
        ));
    }

    #[test]
    fn preview_change_rebuilds_only_the_highlight_overlay_and_cancel_clears_it() {
        use crate::measure_tool::{MeasurementPreview, MeasurementToolKind};
        let db = db_with_line();
        let mut controller = CadSceneController::default();
        let layers = LayerOverrideSet::new();
        let overlays = overlays_without_reference();
        controller
            .prepare_with_overlays(
                Some(&db),
                DocumentId(73),
                None,
                &layers,
                SpaceSelection::Model,
                &overlays,
            )
            .unwrap();
        let base = controller.ready.as_ref().unwrap().base.clone();

        let active = OverlayInputs {
            measurement: Some(MeasurementPreview {
                kind: MeasurementToolKind::Distance,
                points: vec![cad_domain::Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                }],
                cursor: Some(cad_domain::Point3 {
                    x: 4.0,
                    y: 0.0,
                    z: 0.0,
                }),
                remaining: 1,
                ready: false,
            }),
            visibility: OverlayVisibility {
                axes: false,
                grid: false,
                ..OverlayVisibility::default()
            },
            ..OverlayInputs::default()
        };
        controller
            .prepare_with_overlays(
                Some(&db),
                DocumentId(73),
                None,
                &layers,
                SpaceSelection::Model,
                &active,
            )
            .unwrap();
        let with_preview = controller.ready.as_ref().unwrap();
        assert!(Arc::ptr_eq(&base, &with_preview.base));
        // One rubber-band chain (2 vertices) + one marker batch (4 vertices).
        assert_eq!(with_preview.highlight.added.len(), 2);

        // Cancelling (None) makes the preview overlay disappear.
        controller
            .prepare_with_overlays(
                Some(&db),
                DocumentId(73),
                None,
                &layers,
                SpaceSelection::Model,
                &overlays_without_reference(),
            )
            .unwrap();
        assert!(controller
            .ready
            .as_ref()
            .unwrap()
            .highlight
            .added
            .is_empty());
    }

    fn snap_hint(kind: cad_measure::SnapKind, point: cad_domain::Point3) -> SnapHint {
        cad_measure::SnapCandidate {
            kind,
            point,
            space: cad_domain::SpaceId::Model,
            source: cad_domain::SelectionRef {
                document: DocumentId(73),
                entity: cad_domain::EntityId(1),
                instance: cad_domain::InstancePath::default(),
                sub_element: None,
            },
            secondary: None,
            precision: cad_domain::Precision::Analytic,
            logical_pixel_distance: 0.0,
        }
    }

    #[test]
    fn snap_hints_draw_only_when_enabled_and_keep_the_base() {
        let db = db_with_line();
        let mut controller = CadSceneController::default();
        let layers = LayerOverrideSet::new();

        let off = OverlayInputs {
            snap_hints_input: vec![snap_hint(
                cad_measure::SnapKind::Endpoint,
                cad_domain::Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
            )],
            visibility: OverlayVisibility {
                axes: false,
                grid: false,
                snap_hints: false,
                selection_highlight: false,
            },
            ..OverlayInputs::default()
        };
        controller
            .prepare_with_overlays(
                Some(&db),
                DocumentId(73),
                None,
                &layers,
                SpaceSelection::Model,
                &off,
            )
            .unwrap();
        let base = controller.ready.as_ref().unwrap().base.clone();
        assert!(
            controller
                .ready
                .as_ref()
                .unwrap()
                .highlight
                .added
                .is_empty(),
            "snap_hints off must draw no snap-hint batch even when hints are fed"
        );

        let on = OverlayInputs {
            snap_hints_input: vec![
                snap_hint(
                    cad_measure::SnapKind::Endpoint,
                    cad_domain::Point3 {
                        x: 0.0,
                        y: 0.0,
                        z: 0.0,
                    },
                ),
                snap_hint(
                    cad_measure::SnapKind::Center,
                    cad_domain::Point3 {
                        x: 5.0,
                        y: 0.0,
                        z: 0.0,
                    },
                ),
            ],
            visibility: OverlayVisibility {
                axes: false,
                grid: false,
                snap_hints: true,
                selection_highlight: false,
            },
            ..OverlayInputs::default()
        };
        controller
            .prepare_with_overlays(
                Some(&db),
                DocumentId(73),
                None,
                &layers,
                SpaceSelection::Model,
                &on,
            )
            .unwrap();
        let after = controller.ready.as_ref().unwrap();
        assert!(
            Arc::ptr_eq(&base, &after.base),
            "snap hints must not rebuild the base drawing"
        );
        assert_eq!(
            after.highlight.added.len(),
            1,
            "one bounded snap-hint batch"
        );
        assert_eq!(
            after.highlight.added[0].draw_order,
            crate::render_scene::SNAP_HINT_DRAW_ORDER
        );
        assert!(
            after.highlight.added[0].draw_order > 900_000
                && after.highlight.added[0].draw_order < crate::render_scene::PREVIEW_DRAW_ORDER
        );

        // Clearing the hints removes the batch but keeps the base.
        controller
            .prepare_with_overlays(
                Some(&db),
                DocumentId(73),
                None,
                &layers,
                SpaceSelection::Model,
                &overlays_without_reference(),
            )
            .unwrap();
        let cleared = controller.ready.as_ref().unwrap();
        assert!(cleared.highlight.added.is_empty());
        assert!(Arc::ptr_eq(&base, &cleared.base));
    }

    #[test]
    fn disabling_the_selection_highlight_omits_the_selection_batch() {
        use crate::selection::{model_ref, SelectionSet};
        let db = db_with_line();
        let mut controller = CadSceneController::default();
        let layers = LayerOverrideSet::new();
        let hidden = OverlayInputs {
            selection: SelectionSet::from_refs([model_ref(
                DocumentId(73),
                cad_domain::EntityId(1),
            )]),
            visibility: OverlayVisibility {
                axes: false,
                grid: false,
                selection_highlight: false,
                ..OverlayVisibility::default()
            },
            ..OverlayInputs::default()
        };
        controller
            .prepare_with_overlays(
                Some(&db),
                DocumentId(73),
                None,
                &layers,
                SpaceSelection::Model,
                &hidden,
            )
            .unwrap();
        assert!(
            controller
                .ready
                .as_ref()
                .unwrap()
                .highlight
                .added
                .is_empty(),
            "a non-empty selection with selection_highlight off must draw nothing"
        );
    }

    #[test]
    fn axes_and_grid_appear_only_when_enabled_and_keep_the_base() {
        let db = db_with_line();
        let mut controller = CadSceneController::default();
        let layers = LayerOverrideSet::new();
        let off = overlays_without_reference();
        controller
            .prepare_with_overlays(
                Some(&db),
                DocumentId(73),
                None,
                &layers,
                SpaceSelection::Model,
                &off,
            )
            .unwrap();
        let base = controller.ready.as_ref().unwrap().base.clone();
        assert!(controller
            .ready
            .as_ref()
            .unwrap()
            .highlight
            .added
            .is_empty());

        let on = OverlayInputs {
            visibility: OverlayVisibility {
                selection_highlight: false,
                ..OverlayVisibility::default()
            },
            ..OverlayInputs::default()
        };
        controller
            .prepare_with_overlays(
                Some(&db),
                DocumentId(73),
                None,
                &layers,
                SpaceSelection::Model,
                &on,
            )
            .unwrap();
        let after = controller.ready.as_ref().unwrap();
        assert!(
            Arc::ptr_eq(&base, &after.base),
            "reference overlays must not rebuild the base drawing"
        );
        assert!(!after.highlight.added.is_empty());
        assert!(after
            .highlight
            .added
            .iter()
            .all(|b| b.draw_order > 0 && b.draw_order < 900_000));
        assert!(after
            .highlight
            .added
            .iter()
            .any(|b| b.draw_order == AXES_GRID_DRAW_ORDER));
    }

    #[test]
    fn axes_without_bounds_report_a_diagnostic_instead_of_pretending() {
        let db = DrawingDatabaseBuilder::new(DatabaseId(9)).finish().unwrap();
        let mut controller = CadSceneController::default();
        let layers = LayerOverrideSet::new();
        let on = OverlayInputs {
            visibility: OverlayVisibility {
                selection_highlight: false,
                ..OverlayVisibility::default()
            },
            ..OverlayInputs::default()
        };
        controller
            .prepare_with_overlays(
                Some(&db),
                DocumentId(73),
                None,
                &layers,
                SpaceSelection::Model,
                &on,
            )
            .unwrap();
        let ready = controller.ready.as_ref().unwrap();
        assert!(ready.highlight.added.is_empty());
        assert!(ready
            .highlight_diagnostics
            .iter()
            .any(|d| d.code == "overlay.bounds-unavailable"));
        assert!(matches!(
            ready.highlight_completeness,
            cad_domain::Completeness::Partial(_)
        ));
    }
}
