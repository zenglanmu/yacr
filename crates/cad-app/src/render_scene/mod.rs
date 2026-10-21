//! CPU scene preparation. No UI handles, GPU resources or render callbacks.
//!
//! The crate-private controller lives here; the unit tests are split into the
//! sibling `tests` module so this file stays navigable.
use crate::layers::LayerOverrideSet;
use cad_db::DrawingDatabase;
use cad_domain::{CadResult, DocumentId, Point3, SceneIdentity, SelectionRef, TaskStamp};
use cad_representation::{FontEngine, SpaceSelection};
use std::sync::Arc;
mod assembly;
pub use assembly::*;
mod overlay;
pub use overlay::*;

/// Decoded raster-image cache fed to the base scene build, re-exported so the UI
/// bridge can name the type carried by [`PreparedScene::images`].
pub use cad_resources::DecodedImageCache;
/// The prepared-scene delta type carried by [`PreparedScene::base`] /
/// [`PreparedScene::highlight`], re-exported so a consumer that must retain the
/// overlay `Arc` by identity (for example the engine's flags-only fast path) can
/// name it without a direct `cad-scene` dependency.
pub use cad_scene::SceneDelta;

#[derive(Clone, Debug, PartialEq)]
struct SceneVersion {
    identity: SceneIdentity,
    document: DocumentId,
    fonts: u64,
    layers: u64,
    space: SpaceSelection,
    document_epoch: u64,
    /// Identity of the decoded-image cache fed to the base build. A change is a
    /// new pointer, so an image cache installed after the base was built still
    /// rebuilds the image batches.
    images: usize,
}

#[derive(Clone)]
pub struct PreparedScene {
    pub revision: u64,
    pub base_revision: u64,
    /// Monotonic revision of the **entity-flag set** carried by this scene.
    ///
    /// Selection/hover/hidden are a separate publication axis: a flags-only
    /// change republishes `entity_flags` without advancing `revision` (the
    /// scene identity) or `base_revision`. The renderer uses it to apply the
    /// delta in place while keeping the committed scene and its overlay suffix
    /// intact. The engine overwrites this with its own publication revision on
    /// every publish, exactly as it owns `revision`/`base_revision`.
    pub flags_revision: u64,
    pub base: Arc<SceneDelta>,
    /// Transient sub-element-highlight and tool-preview batches.
    pub highlight: Arc<SceneDelta>,
    /// Entity-level selection/hover/hidden flags for the renderer's in-stream
    /// flags path (see `cad_scene::FLAG_*` and `Renderer::set_entity_flags`).
    /// Empty when nothing is selected/hovered/hidden.
    pub entity_flags: Vec<(SelectionRef, u32)>,
    pub highlight_diagnostics: Vec<cad_domain::Diagnostic>,
    pub highlight_completeness: cad_domain::Completeness,
    /// Decoded-image cache the base was built with, carried to the renderer so
    /// its image quads can be resolved. `None` is the previous behaviour.
    pub images: Option<Arc<DecodedImageCache>>,
}

#[derive(Clone, Default)]
pub struct CadSceneController {
    version: Option<SceneVersion>,
    /// `(base_revision, sub-element overlay fingerprint)`.
    visual_version: Option<(u64, u64)>,
    /// Fingerprint of the entity-flag inputs (entity-level selection/hover/hidden).
    flags_version: Option<u64>,
    /// Monotonic revision of the staged entity-flag set. Bumped only when the
    /// flags themselves are re-resolved; the engine overwrites the published
    /// value with its global publication revision.
    flags_revision: u64,
    pub fonts_revision: u64,
    revision: u64,
    base_revision: u64,
    base: Option<Arc<SceneDelta>>,
    visual_overlay: Option<Arc<SceneDelta>>,
    visual_diagnostics: Option<Vec<cad_domain::Diagnostic>>,
    visual_completeness: Option<cad_domain::Completeness>,
    entity_flags: Option<Vec<(SelectionRef, u32)>>,
    flags_diagnostics: Option<Vec<cad_domain::Diagnostic>>,
    flags_completeness: Option<cad_domain::Completeness>,
    shared_document: Option<(Arc<DrawingDatabase>, SceneIdentity)>,
    document_epoch: u64,
    /// Decoded raster-image cache fed to the base build, stored so a later
    /// [`PreparedScene`] carries it to the renderer. `None` = no image context.
    images: Option<Arc<DecodedImageCache>>,
    /// Visible POINT positions derived from the displayed drawing
    /// (ADR 0015 decision E). Cached because they can only change when the base
    /// changes; a camera-only overlay rebuild reuses them instead of rescanning
    /// the model space on every navigation step.
    point_markers: Vec<Point3>,
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
        self.prepare_shared_with_overlays_and_images(
            doc, document, fonts, layers, space, overlays, None,
        )
    }

    /// [`Self::prepare_shared_with_overlays`] plus the decoded raster-image
    /// cache fed to the base scene build.
    ///
    /// The cache is stored on the controller so the published [`PreparedScene`]
    /// carries it for the GPU image upload. `None` is exactly the previous
    /// behaviour (outline-only image frames), so existing callers are
    /// unaffected. A new cache pointer is part of the base version, so
    /// installing one after the base was built rebuilds the image batches.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare_shared_with_overlays_and_images(
        &mut self,
        doc: Option<Arc<DrawingDatabase>>,
        document: DocumentId,
        fonts: Option<Arc<FontEngine>>,
        layers: &LayerOverrideSet,
        space: SpaceSelection,
        overlays: &OverlayInputs,
        images: Option<Arc<DecodedImageCache>>,
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
        self.images = images;
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
            images: self
                .images
                .as_ref()
                .map_or(0, |cache| Arc::as_ptr(cache) as usize),
        });
        if let Some(db) = doc {
            crate::validate_space(db, space)?;
        }
        let base_changed = self.base.is_none() || self.version != version;
        // The visible POINT markers are drawing content: they can only change
        // when the base changes, so derive them once per base revision and reuse
        // the cache for every camera-only overlay rebuild (a full model-space
        // scan per navigation step would be wasted work). The derived positions
        // are injected into the effective inputs so they participate in the
        // overlay fingerprint below.
        if base_changed {
            self.point_markers = match doc {
                Some(db) => visible_point_marker_positions(db, layers, space),
                None => Vec::new(),
            };
        }
        let injected = (overlays.point_markers != self.point_markers).then(|| OverlayInputs {
            point_markers: self.point_markers.clone(),
            ..overlays.clone()
        });
        let overlays = injected.as_ref().unwrap_or(overlays);
        let next_base = self.base_revision + u64::from(base_changed);
        let visual_version = (next_base, overlay_fingerprint(overlays));
        let flags_version = entity_flags_fingerprint(overlays);
        let visual_changed = base_changed || self.visual_version != Some(visual_version);
        let flags_changed = base_changed || self.flags_version != Some(flags_version);
        if !base_changed && !visual_changed && !flags_changed {
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
                Some(db) => build_scene_with_space_and_images(
                    db,
                    stamp.clone(),
                    fonts.clone(),
                    self.images.clone(),
                    layers,
                    space,
                )?,
                None => SceneDelta {
                    stamp: stamp.clone(),
                    added: vec![],
                    removed_chunks: vec![],
                    images: vec![],
                    masks: vec![],
                },
            })
        } else {
            self.base.as_ref().expect("base prepared").clone()
        };

        let (visual_delta, visual_diagnostics, visual_completeness) =
            if visual_changed || self.visual_overlay.is_none() {
                // Screen-space marker sizing: every preview/snap marker's world
                // half-extent is a fixed pixel size times the current view scale,
                // so the per-kind glyphs stay legible under any zoom instead of
                // drifting with a fixed world size. A missing/degenerate scale
                // falls back to the documented world default (never zero/NaN).
                let preview_options = PreviewOptions::for_view_scale(overlays.view_scale);
                let mut overlay = VisualOverlay::default();
                if let Some(db) = doc {
                    // Reference overlays first, so the selection and previews
                    // draw above the axes/grid (they also carry a higher
                    // draw order, but merge order keeps the batch list readable).
                    if overlays.visibility.axes || overlays.visibility.grid {
                        match db.bounds() {
                            Some(bounds) => {
                                if overlays.visibility.grid {
                                    overlay.merge(grid_overlay(
                                        overlays.viewport_bounds.unwrap_or(bounds),
                                        document,
                                    ));
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
                    // Visible POINT markers (ADR 0015 decision E): real drawing
                    // content, drawn above the reference axes/grid and below all
                    // transient chrome. `viewport_bounds` is `None` in a 3D view,
                    // so no marker is emitted there.
                    overlay.merge(point_marker_overlay(
                        &overlays.point_markers,
                        overlays.viewport_bounds,
                        document,
                    ));
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
                        &preview_options,
                    ));
                    if let Some(preview) = overlays.draw_preview.as_ref() {
                        overlay.merge(draw_preview_overlay(
                            preview,
                            document,
                            overlays.visibility.snap_hints,
                            &preview_options,
                        ));
                    }
                    // The live box-selection rubber band is gesture feedback, not
                    // a selection tint: it is drawn whenever the host feeds it,
                    // independently of `visibility.selection_highlight`.
                    if let Some(selection_box) = overlays.selection_box.as_ref() {
                        overlay.merge(box_selection_overlay(selection_box, document));
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
                            &preview_options,
                        ));
                    }
                    // The pointer cursor crosshair + pickbox (renderer-only; the
                    // CAD partition has no DOM overlay). Drawn last, at the
                    // highest `draw_order`, so it is never occluded by geometry
                    // or the transient chrome above. An absent cursor or a 3D
                    // view yields no batch (an explicit empty overlay).
                    overlay.merge(cursor_overlay(
                        overlays.cursor,
                        overlays.viewport_bounds,
                        overlays.view_scale,
                        overlays.pickbox_logical_pixels,
                        overlays.crosshair_percent,
                        document,
                    ));
                }
                let delta = Arc::new(SceneDelta {
                    stamp: stamp.clone(),
                    added: overlay.batches,
                    removed_chunks: vec![],
                    images: vec![],
                    masks: vec![],
                });
                (delta, overlay.diagnostics, overlay.completeness)
            } else {
                self.visual_result()
            };

        // Entity-level flags are resolved separately from the overlay batches, so
        // an entity-only selection change republishes the flags without touching
        // the overlay delta (the same `Arc` is reused) or the base. The actual
        // resolution lives in `refresh_flags_only` — the **single** flags truth,
        // shared with the engine's synchronous flags publication and its
        // publication-boundary race guard. `base_changed` forces a re-resolution
        // because a new base can change what a ref resolves to (for example a
        // hidden layer).
        if flags_changed || self.entity_flags.is_none() {
            self.refresh_flags(doc, document, overlays, flags_changed);
        }
        let entity_flags = self.entity_flags.clone().unwrap_or_default();
        let flags_diagnostics = self.flags_diagnostics.clone().unwrap_or_default();
        let flags_completeness = self
            .flags_completeness
            .clone()
            .unwrap_or(cad_domain::Completeness::Complete);

        self.revision += 1;
        self.base_revision = next_base;
        self.version = version;
        self.visual_version = Some(visual_version);
        self.flags_version = Some(flags_version);
        self.base = Some(base.clone());
        self.visual_overlay = Some(visual_delta.clone());
        self.visual_diagnostics = Some(visual_diagnostics.clone());
        self.visual_completeness = Some(visual_completeness.clone());
        self.entity_flags = Some(entity_flags.clone());
        self.flags_diagnostics = Some(flags_diagnostics.clone());
        self.flags_completeness = Some(flags_completeness.clone());
        let mut highlight_diagnostics = visual_diagnostics;
        highlight_diagnostics.extend(flags_diagnostics);
        self.ready = Some(PreparedScene {
            revision: self.revision,
            base_revision: next_base,
            flags_revision: self.flags_revision,
            base,
            highlight: visual_delta,
            entity_flags,
            highlight_diagnostics,
            highlight_completeness: visual_completeness.combine(flags_completeness),
            images: self.images.clone(),
        });
        Ok(())
    }

    /// Re-resolve the entity-level flags from `overlays` and publish them into
    /// the staged (and, when present, the already published) scene.
    ///
    /// This is the **single** flags resolver: the full preparation and the
    /// engine's synchronous flags-only path both go through it, so the renderer
    /// flag table can never be fed by a second, divergent truth. It reuses
    /// `flags_version` as the change fingerprint and `selection_entity_flags`
    /// as the resolver.
    ///
    /// Returns whether the flags changed. Only the flags are touched:
    /// `revision`, `base_revision`, the base `Arc` and the overlay `Arc` are
    /// left exactly as they were, so a flags-only refresh never invalidates the
    /// committed scene or its overlay suffix. `flags_revision` advances when the
    /// flags change (the engine then stamps the global publication revision).
    pub fn refresh_flags_only(
        &mut self,
        doc: Option<&DrawingDatabase>,
        document: DocumentId,
        overlays: &OverlayInputs,
    ) -> bool {
        self.refresh_flags(doc, document, overlays, false)
    }

    /// [`Self::refresh_flags_only`] with an explicit re-resolution override.
    ///
    /// `force` re-resolves even when the fingerprint is unchanged; the full
    /// preparation passes its `flags_changed` (which includes `base_changed`)
    /// because a new base can change what an unchanged ref resolves to.
    fn refresh_flags(
        &mut self,
        doc: Option<&DrawingDatabase>,
        document: DocumentId,
        overlays: &OverlayInputs,
        force: bool,
    ) -> bool {
        let flags_version = entity_flags_fingerprint(overlays);
        if !force && self.flags_version == Some(flags_version) && self.entity_flags.is_some() {
            return false;
        }
        let (flags, diagnostics, completeness) = match doc {
            Some(db) => {
                let scene = selection_entity_flags(db, overlays, document);
                (scene.flags, scene.diagnostics, scene.completeness)
            }
            None => (Vec::new(), Vec::new(), cad_domain::Completeness::Complete),
        };
        self.flags_version = Some(flags_version);
        self.flags_revision = self
            .flags_revision
            .checked_add(1)
            .expect("flags revision exhausted");
        self.entity_flags = Some(flags.clone());
        self.flags_diagnostics = Some(diagnostics.clone());
        self.flags_completeness = Some(completeness.clone());
        if let Some(ready) = self.ready.as_mut() {
            // Refresh the published flags in place. `revision`/`base_revision`
            // and both geometry `Arc`s stay untouched: this is not a new scene.
            let mut highlight_diagnostics = self.visual_diagnostics.clone().unwrap_or_default();
            highlight_diagnostics.extend(diagnostics);
            ready.entity_flags = flags;
            ready.highlight_diagnostics = highlight_diagnostics;
            ready.highlight_completeness = self
                .visual_completeness
                .clone()
                .unwrap_or(cad_domain::Completeness::Complete)
                .combine(completeness);
            ready.flags_revision = self.flags_revision;
        }
        true
    }

    /// The cached transient overlay batches plus their diagnostics/completeness.
    fn visual_result(
        &self,
    ) -> (
        Arc<SceneDelta>,
        Vec<cad_domain::Diagnostic>,
        cad_domain::Completeness,
    ) {
        match (
            &self.visual_overlay,
            &self.visual_diagnostics,
            &self.visual_completeness,
        ) {
            (Some(delta), Some(diagnostics), Some(completeness)) => {
                (delta.clone(), diagnostics.clone(), completeness.clone())
            }
            (Some(delta), _, _) => (
                delta.clone(),
                Vec::new(),
                cad_domain::Completeness::Complete,
            ),
            (None, _, _) => (
                Arc::new(SceneDelta {
                    stamp: TaskStamp::new(DocumentId(0), 0),
                    added: Vec::new(),
                    removed_chunks: Vec::new(),
                    images: Vec::new(),
                    masks: Vec::new(),
                }),
                Vec::new(),
                cad_domain::Completeness::Complete,
            ),
        }
    }
}

#[cfg(test)]
mod tests;
