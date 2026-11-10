//! The published [`PreparedScene`] and the controller's base-version key.
//!
//! Split out of `mod.rs` so the controller file stays navigable (and under the
//! project's file-size ceiling); nothing here is controller logic.

use std::sync::Arc;

use cad_domain::{Completeness, Diagnostic, DocumentId, SceneIdentity, SelectionRef};
use cad_representation::SpaceSelection;
use cad_resources::DecodedImageCache;

use super::{BaseAppend, RenderBatch, ResidentScene, SceneDelta};

/// An appended delta may not grow the resident image/mask sets: the engine's
/// append path does not re-upload them.
///
/// `crates/cad-engine/src/render/runtime/session.rs` keeps the resident
/// textures/masks (and their earlier diagnostics) when a preparation is an
/// append and uploads images/masks only for a non-append base change. An append
/// whose per-entity delta carries raster draw items would therefore publish a
/// CPU base that the GPU never received — a silent partial success. The
/// controller falls back to the full rebuild instead.
pub(super) fn appendable_delta(delta: &SceneDelta) -> bool {
    delta.images.is_empty() && delta.masks.is_empty()
}

/// Everything that decides how the resident base is built.
///
/// A full `PartialEq` mismatch (including the scene identity's revision and
/// entity count) normally means a full rebuild; the retire and transform-update
/// paths compare the subset that must stay equal
/// (`retire::retire_keeps_base`).
#[derive(Clone, Debug, PartialEq)]
pub(super) struct SceneVersion {
    pub(super) identity: SceneIdentity,
    pub(super) document: DocumentId,
    pub(super) fonts: u64,
    pub(super) font_identity: usize,
    pub(super) layers: u64,
    pub(super) space: SpaceSelection,
    pub(super) document_epoch: u64,
    /// Identity of the decoded-image cache fed to the base build. A change is a
    /// new pointer, so an image cache installed after the base was built still
    /// rebuilds the image batches.
    pub(super) images: usize,
    /// Revision of the vendor derived-representation snapshot. A new plugin pass
    /// rebuilds the base so its primitives become visible without touching
    /// `cad-db`.
    pub(super) vendor: Option<u64>,
    pub(super) vendor_identity: usize,
    /// Display `WIPEOUTFRAME` state fed to the representation context. A change
    /// adds/removes the mask boundary frame, so it must rebuild the base.
    pub(super) wipeout_frames: bool,
}

/// One staged CPU publication: the resident base, its transient overlay and the
/// entity-level flags, plus any in-place batch replacements.
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
    /// The resident CPU base: a shared prefix plus the segments appended since
    /// it (see [`ResidentScene`]). An accepted append pushes one segment instead
    /// of re-materialising the whole base, so the append cost is O(changed).
    pub base: ResidentScene,
    pub base_append: Option<BaseAppend>,
    /// In-place batch replacements for a single-entity display update: one
    /// `(resident batch index, rebuilt batch)` per slot of the edited entity's
    /// range, in range order.
    ///
    /// The engine applies them with `Renderer::overwrite_batch` **after** the
    /// existing prepare steps, so the slot, the pick key and the batch/draw order
    /// of every replaced batch are unchanged. A single refusal must fall back to a
    /// full base re-stage (never keep the old geometry). Empty for every other
    /// publication.
    pub overwrites: Vec<(usize, Arc<RenderBatch>)>,
    /// Transient sub-element-highlight and tool-preview batches.
    pub highlight: Arc<SceneDelta>,
    /// Entity-level selection/hover/hidden flags for the renderer's in-stream
    /// flags path (see `cad_scene::FLAG_*` and `Renderer::set_entity_flags`).
    /// Empty when nothing is selected/hovered/hidden.
    pub entity_flags: Vec<(SelectionRef, u32)>,
    pub highlight_diagnostics: Vec<Diagnostic>,
    pub highlight_completeness: Completeness,
    /// Decoded-image cache the base was built with, carried to the renderer so
    /// its image quads can be resolved. `None` is the previous behaviour.
    pub images: Option<Arc<DecodedImageCache>>,
}
