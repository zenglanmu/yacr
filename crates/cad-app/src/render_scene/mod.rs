//! CPU scene preparation. No UI handles, GPU resources or render callbacks.
//!
//! The crate-private controller lives here; the unit tests are split into the
//! sibling `tests` module so this file stays navigable.
use crate::layers::LayerOverrideSet;
use crate::vendor::VendorRepresentationStore;
use cad_db::DrawingDatabase;
use cad_domain::{CadResult, DocumentId, EntityId, Point3, SceneIdentity, SelectionRef, TaskStamp};
use cad_representation::{FontEngine, SpaceSelection};
use std::collections::BTreeSet;
use std::ops::Range;
use std::sync::Arc;
mod assembly;
pub use assembly::*;
mod edit;
pub(crate) use edit::CommitDelta;
pub use edit::{BaseAppend, DrawingEditReceipt};
#[cfg(test)]
mod edit_tests;
mod entity_index;
mod overlay;
mod prepared;
pub use prepared::PreparedScene;
use prepared::{appendable_delta, SceneVersion};
mod resident;
mod retire;
#[cfg(test)]
mod retire_tests;
use retire::{
    flags_fingerprint, merge_retired_flags, retire_is_hideable, retire_keeps_base,
    RETIRED_COMPACTION_THRESHOLD,
};
mod update;
#[cfg(test)]
mod update_tests;
#[cfg(test)]
mod vendor_tests;
pub use overlay::*;
pub use resident::ResidentScene;

use entity_index::EntityBatchIndex;

/// Decoded raster-image cache fed to the base scene build, re-exported so the UI
/// bridge can name the type carried by [`PreparedScene::images`].
pub use cad_resources::DecodedImageCache;
/// The batch type carried by [`PreparedScene::overwrites`], re-exported so the
/// engine can name an in-place overwrite without a direct `cad-scene` dependency.
pub use cad_scene::RenderBatch;
/// The delta type a [`ResidentScene`] is built from and the type carried by
/// [`PreparedScene::highlight`], re-exported so a consumer that must retain the
/// overlay `Arc` by identity (for example the engine's flags-only fast path) can
/// name it without a direct `cad-scene` dependency.
pub use cad_scene::SceneDelta;

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
    /// The resident CPU base. A full build replaces it wholesale; an accepted
    /// append pushes one segment onto the retained predecessor (`ResidentScene`
    /// keeps the prefix `Arc` shared, so the append never copies the scene).
    base: Option<ResidentScene>,
    /// Exact shared snapshot which produced the successfully published base.
    /// Incoming/adopted documents are not evidence of successful preparation.
    published_drawing: Option<Arc<DrawingDatabase>>,
    // Retain resource identities, not just addresses: allocator reuse must not
    // make a replacement engine/snapshot look like the preceding resource.
    base_fonts: Option<Arc<FontEngine>>,
    base_vendor: Option<Arc<VendorRepresentationStore>>,
    base_append: Option<BaseAppend>,
    edit: Option<DrawingEditReceipt>,
    /// Resident batch ownership: for every visible model-space entity that
    /// emitted at least one render batch, the range of the resident base stream
    /// its own compacted delta occupies. Bookkeeping only (module `entity_index`).
    ///
    /// Invariants: replaced wholesale by a full base build, extended by an
    /// accepted append, empty off model space, and never left stale (an entity
    /// that produces no batches has no entry).
    entity_batches: EntityBatchIndex,
    /// Ids whose resident batches were deleted but are still physically present
    /// in the base, suppressed by `FLAG_HIDDEN` until a full rebuild compacts
    /// them. Reset by every full base build.
    retired: BTreeSet<EntityId>,
    model_max_order: Option<i64>,
    #[cfg(test)]
    fail_after_point_staging: bool,
    /// Test-only: treat the next transform-only update as not fitting, so the
    /// controller's full-rebuild fallback is exercised without a renderer.
    #[cfg(test)]
    fail_next_update_fit: bool,
    /// Test override for [`RETIRED_COMPACTION_THRESHOLD`]; `None` uses the
    /// documented default.
    #[cfg(test)]
    retired_compaction_threshold: Option<usize>,
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
    /// Derived vendor display snapshot (display-only; never `cad-db` state).
    vendor: Option<Arc<VendorRepresentationStore>>,
    /// Visible POINT positions derived from the displayed drawing
    /// (ADR 0015 decision E). Cached because they can only change when the base
    /// changes; a camera-only overlay rebuild reuses them instead of rescanning
    /// the model space on every navigation step.
    point_markers: Vec<Point3>,
    /// The displayed drawing's POINT display style (`$PDMODE` / `$PDSIZE`),
    /// cached beside [`Self::point_markers`] for the same reason: it belongs to
    /// the drawing, so it can only change when the base changes.
    point_display_style: PointDisplayStyle,
    pub ready: Option<PreparedScene>,
    pub diagnostic: Option<String>,
}

impl CadSceneController {
    /// Immutable database adoption lets a camera-only prepare reuse the resident
    /// base without re-deriving scene state. Retaining the Arc prevents pointer
    /// reuse from hiding a new open, even when the database's id/revision happen
    /// to match.
    #[allow(clippy::too_many_arguments)]
    /// Install the derived vendor snapshot that the next base build must merge.
    ///
    /// The caller replaces the whole snapshot; the controller compares its
    /// revision in [`SceneVersion`] so an unchanged store is free.
    pub fn set_vendor_representations(&mut self, vendor: Option<Arc<VendorRepresentationStore>>) {
        self.vendor = vendor;
    }

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
        self.prepare_shared_with_overlays_and_images_and_edit(
            doc, document, fonts, layers, space, overlays, images, None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn prepare_shared_with_overlays_and_images_and_edit(
        &mut self,
        doc: Option<Arc<DrawingDatabase>>,
        document: DocumentId,
        fonts: Option<Arc<FontEngine>>,
        layers: &LayerOverrideSet,
        space: SpaceSelection,
        overlays: &OverlayInputs,
        images: Option<Arc<DecodedImageCache>>,
        edit: Option<&DrawingEditReceipt>,
    ) -> CadResult<()> {
        self.edit = edit
            .filter(|edit| {
                self.published_drawing
                    .as_ref()
                    .is_some_and(|old| Arc::ptr_eq(old, &edit.before))
                    && doc
                        .as_ref()
                        .is_some_and(|new| Arc::ptr_eq(new, &edit.after))
            })
            .cloned();
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
        // A borrowed preparation cannot establish shared-snapshot provenance.
        self.published_drawing = None;
        self.edit = None;
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
            font_identity: fonts
                .as_ref()
                .map_or(0, |fonts| Arc::as_ptr(fonts) as usize),
            layers: layers.fingerprint(),
            space,
            document_epoch: self.document_epoch,
            images: self
                .images
                .as_ref()
                .map_or(0, |cache| Arc::as_ptr(cache) as usize),
            vendor: self.vendor.as_ref().map(|store| store.revision()),
            vendor_identity: self
                .vendor
                .as_ref()
                .map_or(0, |store| Arc::as_ptr(store) as usize),
        });
        if let Some(db) = doc {
            crate::validate_space(db, space)?;
        }
        let version_changed = self.base.is_none() || self.version != version;
        // A delete-only commit keeps the resident base: the deleted ids are
        // accumulated as retired and hidden through the flags path. All the
        // preconditions (document/space/fonts/layers/images/vendor unchanged,
        // every deleted class hideable by a whole-entity flag) must hold, and the
        // accumulation must not have crossed the compaction threshold, otherwise
        // this is today's full rebuild and the retired set is cleared.
        let retire_candidate = self.edit.as_ref().and_then(|edit| {
            let CommitDelta::Retire(ids) = &edit.delta else {
                return None;
            };
            if self.base.is_none() || !std::ptr::eq(doc?, edit.after.as_ref()) {
                return None;
            }
            let old = self.version.as_ref()?;
            let new = version.as_ref()?;
            (retire_keeps_base(old, new)
                && ids.iter().all(|id| {
                    edit.before
                        .entity(*id)
                        .is_none_or(|entity| retire_is_hideable(&entity.geometry))
                }))
            .then(|| ids.clone())
        });
        let mut next_retired = match retire_candidate.as_ref() {
            Some(ids) => {
                let mut merged = self.retired.clone();
                merged.extend(ids.iter().copied());
                merged
            }
            None => self.retired.clone(),
        };
        let retire_taken =
            retire_candidate.is_some() && next_retired.len() <= self.retired_compaction_limit();
        if !retire_taken {
            next_retired = self.retired.clone();
        }
        let mut stamp = TaskStamp::new(document, self.revision + 1);
        stamp.configuration_version = self.fonts_revision;
        if let Some(db) = doc {
            stamp.object_revision = db.revision();
        }
        // P4: a transform-only update keeps the resident base and replaces the
        // moved entity's own batches in place. The staging is provisional: the
        // rebuilt delta must have exactly the entity's resident batch count (a
        // GEOMETRY change can alter it), so a mismatch declines to the full
        // rebuild below instead of publishing a partial range.
        let update_candidate = update::update_candidate(
            self.edit.as_ref(),
            doc,
            space,
            self.base.as_ref(),
            &self.entity_batches,
            &self.retired,
            self.version.as_ref(),
            version.as_ref(),
        );
        let update_rebuild = update_candidate.and_then(|(id, range)| {
            #[cfg(test)]
            if std::mem::take(&mut self.fail_next_update_fit) {
                return None;
            }
            let doc = doc.expect("an update candidate has a shared snapshot");
            update::stage_update(
                doc,
                id,
                &range,
                stamp.clone(),
                fonts.clone(),
                self.images.clone(),
                layers,
                self.vendor.as_deref(),
            )
            .map(|batches| (id, range, batches))
        });
        let update_taken = update_rebuild.is_some();
        let base_changed = version_changed && !retire_taken && !update_taken;
        let appended_entity = self.edit.as_ref().and_then(|edit| {
            let inserted = match &edit.delta {
                CommitDelta::Append(inserted) => *inserted,
                _ => return None,
            };
            // Undo trap: re-inserting a retired id must never append. The old
            // resident batches for that id are still in the base (hidden), so the
            // new ones would either double-draw once un-hidden or stay invisible;
            // the only correct answer is the full rebuild that compacts them.
            if self.retired.contains(&inserted) {
                return None;
            }
            if !std::ptr::eq(doc?, edit.after.as_ref()) {
                return None;
            }
            let old = self.version.as_ref()?;
            let new = version.as_ref()?;
            let entity = edit.after.entity(inserted)?;
            (base_changed
                && space == SpaceSelection::Model
                && self.base.is_some()
                && old.document == new.document
                && old.space == new.space
                && old.fonts == new.fonts
                && old.font_identity == new.font_identity
                && old.layers == new.layers
                && old.images == new.images
                && old.vendor == new.vendor
                && old.vendor_identity == new.vendor_identity
                && self
                    .model_max_order
                    .is_none_or(|order| entity.draw_order > order))
            .then(|| entity.clone())
        });
        // Stage every CPU result before publishing any version. A visual-overlay
        // edit keeps the base `Arc` (and its GPU revisions) unchanged.
        let next_append = if appended_entity.is_some() {
            self.base.as_ref().map(|base| BaseAppend {
                predecessor: base.clone(),
                keep_batches: base.batch_len(),
            })
        } else if base_changed || retire_taken {
            None
        } else {
            self.base_append.clone()
        };
        // The one full base build: the `base_changed` arm and the append-decline
        // fallback must produce exactly the same CPU scene. It also reports the
        // per-entity ownership of `added`, which becomes the new index wholesale
        // (a rebuild never patches the previous one).
        let full_base =
            |doc: Option<&DrawingDatabase>| -> CadResult<(SceneDelta, EntityBatchIndex)> {
                match doc {
                    Some(db) => {
                        let (delta, ranges) =
                            assembly::build_scene_with_space_and_images_and_vendor_indexed(
                                db,
                                stamp.clone(),
                                fonts.clone(),
                                self.images.clone(),
                                layers,
                                space,
                                self.vendor.as_deref(),
                            )?;
                        Ok((delta, EntityBatchIndex::from_ranges(ranges)))
                    }
                    None => Ok((
                        SceneDelta {
                            stamp: stamp.clone(),
                            added: vec![],
                            images: vec![],
                            masks: vec![],
                        },
                        EntityBatchIndex::default(),
                    )),
                }
            };
        // An append is provisional until the freshly built per-entity delta is
        // known to be appendable; a declined append rebuilds the whole base and
        // publishes no append proof.
        let mut append_declined = false;
        let mut rebuilt = false;
        let (base, next_entity_batches, overwrites) =
            if let Some((_id, range, batches)) = update_rebuild {
                // O(changed): swap one per-batch `Arc` per resident slot of the moved
                // entity. The range was validated against the resident base, so a
                // refused replacement is an invariant violation, never a silent
                // partial update.
                let mut resident = self.base.as_ref().expect("update base").clone();
                let mut overwrites = Vec::with_capacity(batches.len());
                for (offset, batch) in batches.into_iter().enumerate() {
                    let index = range.start + offset;
                    assert!(
                        resident.replace_batch(index, batch.clone()),
                        "the staged update range was validated against the resident base"
                    );
                    overwrites.push((index, batch));
                }
                // The per-entity index is unchanged: the range and its length are.
                (resident, self.entity_batches.clone(), overwrites)
            } else if let Some(entity) = appended_entity.as_ref() {
                let delta = assembly::build_model_entity_scene(
                    doc.expect("append drawing"),
                    entity,
                    stamp.clone(),
                    fonts.clone(),
                    self.images.clone(),
                    layers,
                    self.vendor.as_deref(),
                )?;
                if appendable_delta(&delta) {
                    // O(changed): keep the predecessor's prefix `Arc` and push the
                    // freshly built per-entity delta as one appended segment. The
                    // predecessor is retained (never re-built), so a large resident
                    // scene is not copied.
                    let mut resident = self.base.as_ref().expect("append predecessor").clone();
                    let inserted_start = resident.batch_len();
                    resident.push_appended(Arc::new(delta));
                    // The inserted entity's batches are one contiguous suffix (the
                    // appended segment); an empty delta (hidden entity) records
                    // nothing and the append is declined downstream anyway.
                    let mut index = self.entity_batches.clone();
                    index.record(entity.id, inserted_start..resident.batch_len());
                    (resident, index, Vec::new())
                } else {
                    append_declined = true;
                    rebuilt = true;
                    let (delta, index) = full_base(doc)?;
                    (ResidentScene::from_owned(delta), index, Vec::new())
                }
            } else if base_changed {
                // This also covers the undo trap (a re-inserted retired id declines
                // the append) and a retire that crossed the compaction threshold.
                rebuilt = true;
                let (delta, index) = full_base(doc)?;
                (ResidentScene::from_owned(delta), index, Vec::new())
            } else {
                // Nothing structural changed, a delete-only commit kept the base, or
                // a transform-only update replaced one entity's batches in place: the
                // resident scene and its per-entity index are otherwise untouched.
                (
                    self.base.as_ref().expect("base prepared").clone(),
                    self.entity_batches.clone(),
                    Vec::new(),
                )
            };
        if rebuilt {
            // A full rebuild is the accepted epoch reset: it retires every pick
            // key and allocates fresh ones, so no retired id can still name a
            // resident batch.
            next_retired.clear();
        }
        // The index is model-space only: paper-space batches are not owned by a
        // single entity, so the previous (model) entries must not survive a
        // switch to a layout.
        let mut next_entity_batches = next_entity_batches;
        if space != SpaceSelection::Model {
            next_entity_batches.clear();
        }
        let next_append = if append_declined { None } else { next_append };
        // A declined append is a full rebuild, so it must rescan the markers
        // exactly like one; only an append that was actually taken may reuse the
        // previous marker set (the inserted entity is never a POINT).
        let append_taken = appended_entity.is_some() && !append_declined;
        // The visible POINT markers are drawing content: they can only change
        // when the base changes, so derive them once per base revision and reuse
        // the cache for every camera-only overlay rebuild (a full model-space
        // scan per navigation step would be wasted work). The derived positions
        // are injected into the effective inputs so they participate in the
        // overlay fingerprint below.
        let staged_points = if base_changed && !append_taken {
            let positions = match doc {
                Some(db) => visible_point_marker_positions(db, layers, space),
                None => Vec::new(),
            };
            let style = match doc {
                Some(db) => PointDisplayStyle {
                    mode: db.point_display_mode(),
                    size: db.point_display_size(),
                },
                None => PointDisplayStyle::default(),
            };
            // The test hook lives **inside** the staging arm so it fires exactly
            // when the drawing-wide scan actually ran (a taken append skips it).
            #[cfg(test)]
            if std::mem::take(&mut self.fail_after_point_staging) {
                return Err(cad_domain::CadError::Invariant(
                    "test: failure after POINT staging".into(),
                ));
            }
            Some((positions, style))
        } else {
            None
        };
        let (point_markers, point_display_style) = staged_points
            .as_ref()
            .map(|(positions, style)| (positions, *style))
            .unwrap_or((&self.point_markers, self.point_display_style));
        let injected = (overlays.point_markers != *point_markers
            || overlays.point_display_style != point_display_style)
            .then(|| OverlayInputs {
                point_markers: point_markers.clone(),
                point_display_style,
                ..overlays.clone()
            });
        let overlays = injected.as_ref().unwrap_or(overlays);
        let next_base = self.base_revision + u64::from(base_changed);
        let visual_version = (next_base, overlay_fingerprint(overlays));
        let flags_version = flags_fingerprint(overlays, &next_retired);
        let visual_changed = base_changed || self.visual_version != Some(visual_version);
        let flags_changed = base_changed || self.flags_version != Some(flags_version);
        // A transform-only update changes geometry without changing the base
        // revision, the overlay fingerprint or the flags: it must still publish so
        // the engine applies the in-place overwrites.
        if !base_changed && !visual_changed && !flags_changed && !update_taken {
            return Ok(());
        }

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
                    // transient chrome, with the drawing's `$PDMODE`/`$PDSIZE`
                    // glyph and size. `viewport_bounds` is `None` in a 3D view,
                    // so no marker is emitted there.
                    overlay.merge(point_marker_overlay(
                        &overlays.point_markers,
                        overlays.viewport_bounds,
                        overlays.point_display_style,
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
                        // Grip handles are the draggable affordance of the
                        // entities just resolved above, so they share the
                        // selection-highlight gate instead of a visibility leaf
                        // of their own. The list is host-fed (one entry per
                        // grip); `viewport_bounds` is `None` in a 3D view, so no
                        // marker is emitted there.
                        overlay.merge(grip_marker_overlay(
                            &overlays.grip_markers,
                            overlays.viewport_bounds,
                            document,
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
                    // from the renderer-owned pointer cursor: distinct shapes
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
                }
                let delta = Arc::new(SceneDelta {
                    stamp: stamp.clone(),
                    added: overlay.batches,
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
            self.refresh_flags(doc, document, overlays, &next_retired, flags_changed);
        }
        let entity_flags = self.entity_flags.clone().unwrap_or_default();
        let flags_diagnostics = self.flags_diagnostics.clone().unwrap_or_default();
        let flags_completeness = self
            .flags_completeness
            .clone()
            .unwrap_or(cad_domain::Completeness::Complete);

        self.revision += 1;
        if let Some((positions, style)) = staged_points {
            self.point_markers = positions;
            self.point_display_style = style;
        }
        self.base_revision = next_base;
        if base_changed {
            self.base_fonts = fonts.clone();
            self.base_vendor = self.vendor.clone();
            self.model_max_order = match appended_entity.as_ref() {
                Some(entity) => Some(entity.draw_order),
                None => doc.and_then(|db| {
                    db.entities()
                        .filter(|entity| entity.space == cad_domain::SpaceId::Model)
                        .map(|entity| entity.draw_order)
                        .max()
                }),
            };
        }
        if base_changed || retire_taken || update_taken {
            // A retire or an in-place update does not rebuild the base, but the
            // published snapshot must still advance: the next receipt is bound to
            // this new `Arc`.
            self.published_drawing = self
                .shared_document
                .as_ref()
                .filter(|(shared, _)| doc.is_some_and(|db| std::ptr::eq(shared.as_ref(), db)))
                .map(|(shared, _)| shared.clone());
        }
        self.base_append = next_append.clone();
        self.retired = next_retired;
        self.version = version;
        self.visual_version = Some(visual_version);
        self.flags_version = Some(flags_version);
        self.base = Some(base.clone());
        self.entity_batches = next_entity_batches;
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
            base_append: next_append,
            overwrites,
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
        let retired = self.retired.clone();
        self.refresh_flags(doc, document, overlays, &retired, false)
    }

    /// [`Self::refresh_flags_only`] with an explicit re-resolution override.
    ///
    /// `force` re-resolves even when the fingerprint is unchanged; the full
    /// preparation passes its `flags_changed` (which includes `base_changed`)
    /// because a new base can change what an unchanged ref resolves to.
    ///
    /// `retired` is the id set this resolution must suppress with `FLAG_HIDDEN`;
    /// the caller passes the **staged** set during a preparation and the
    /// controller's current set on the engine's sync path.
    fn refresh_flags(
        &mut self,
        doc: Option<&DrawingDatabase>,
        document: DocumentId,
        overlays: &OverlayInputs,
        retired: &BTreeSet<EntityId>,
        force: bool,
    ) -> bool {
        let flags_version = flags_fingerprint(overlays, retired);
        if !force && self.flags_version == Some(flags_version) && self.entity_flags.is_some() {
            return false;
        }
        let (mut flags, diagnostics, completeness) = match doc {
            Some(db) => {
                let scene = selection_entity_flags(db, overlays, document);
                (scene.flags, scene.diagnostics, scene.completeness)
            }
            None => (Vec::new(), Vec::new(), cad_domain::Completeness::Complete),
        };
        // A retired entity is no longer in the database, so it cannot go through
        // the resolver (which would report it as `highlight.unresolved`). Merge
        // its whole-entity hidden bit directly into the resolved set.
        merge_retired_flags(&mut flags, document, retired);
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

    /// The range of the resident model-space base (`ready.base`) owned by `id`,
    /// if that entity contributed at least one render batch.
    ///
    /// An entity that produces no batches — hidden layer, non-drawable geometry,
    /// a POINT (whose glyph is a drawing-wide overlay, not a base batch) — has
    /// **no** entry, as does every entity of a paper-space base. Crate-internal
    /// bookkeeping for incremental update/delete; deliberately not published on
    /// the wire or in the scene status.
    // Consumed by the upcoming incremental update/delete phases; the index is
    // maintained (and tested) now, so this getter is intentionally unused by
    // production code until then.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn entity_batch_range(&self, id: EntityId) -> Option<Range<usize>> {
        self.entity_batches.get(id)
    }

    /// Number of entities tracked by [`Self::entity_batch_range`]. Diagnostics
    /// and tests only; not part of the published scene status.
    #[cfg(test)]
    pub(crate) fn entity_batch_index_len(&self) -> usize {
        self.entity_batches.len()
    }

    /// Number of retired ids still physically present in the resident base.
    /// Diagnostics and tests only; not part of the published scene status.
    #[cfg(test)]
    pub(crate) fn retired_count(&self) -> usize {
        self.retired.len()
    }

    /// Lower the retire-compaction threshold for a test.
    ///
    /// A production controller never calls this and keeps
    /// [`RETIRED_COMPACTION_THRESHOLD`].
    #[cfg(test)]
    pub(crate) fn set_retired_compaction_threshold(&mut self, threshold: usize) {
        self.retired_compaction_threshold = Some(threshold);
    }

    /// Force the next transform-only update to decline its in-place fit, so the
    /// full-rebuild fallback is exercised without a renderer. Test only.
    #[cfg(test)]
    pub(crate) fn set_fail_next_update_fit(&mut self, fail: bool) {
        self.fail_next_update_fit = fail;
    }

    fn retired_compaction_limit(&self) -> usize {
        #[cfg(test)]
        if let Some(threshold) = self.retired_compaction_threshold {
            return threshold;
        }
        RETIRED_COMPACTION_THRESHOLD
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
