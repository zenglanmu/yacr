//! The GPU half of an open: initialize the renderer on the presenter's device and
//! upload the display representation in **bounded chunks**.
//!
//! # Why chunked
//!
//! `Renderer::prepare_upload_batches` turns a slice of `RenderBatch`es into GPU
//! buffers, and for a large drawing the packed intermediates it builds per call
//! (instanced line streams, expanded anti-aliased quads, merged mesh chunks) are
//! comparable in size to the CPU scene itself. Staging every batch in one call is
//! therefore a second, near-scene-sized transient on top of a `SceneDelta` that is
//! already resident — precisely the wrong thing on a 4 GiB wasm heap.
//!
//! This module therefore does what `cad-engine` does when it pages uploads: stage
//! a bounded piece ([`budget::next_chunk_end`]), append it to the pending upload
//! (`PreparedUpload::append`, no GPU state published), and release the piece's CPU
//! geometry right away. Each chunk's worst-case transient is bounded by
//! `budget::UPLOAD_CHUNK_MAX_VERTICES`, not by the drawing.
//!
//! The bound is **deterministic** (batches *and* vertices), not a wall-clock page.
//! The engine's ~8 ms page exists to keep a foreground frame loop responsive
//! between slices; this open runs synchronously to its single commit, so a time
//! box would only make `upload_chunks` / `peak_batches_per_chunk` irreproducible
//! between runs — and those two numbers are the evidence.
//!
//! # Completeness is never traded for progress
//!
//! Staging is **not** publication: nothing is visible to the renderer until the
//! single `commit_upload(prepared, 0)` at the end, which atomically replaces the
//! scene. So the first frame the caller presents is always the whole drawing —
//! there is no "present what is uploaded so far" path that could look like a
//! successful open of a partial scene. A failure at any chunk publishes nothing.
//!
//! Chunk progress is reported as console lines with a stable marker
//! (`yacr: upload-progress:`) plus the measured `upload_chunks` /
//! `peak_batches_per_chunk` fields in `status()`/`document()`. Those are
//! measurements of this open, not performance claims.
//!
//! # The trade-off, stated
//!
//! A chunk boundary is also an instanced-line/merged-mesh **stream** boundary, so
//! a chunked upload can produce a few more resident streams than a whole-scene
//! one, and therefore a few more draw submissions per frame (bounded by the chunk
//! count, ~72 on the measured baseline against ~7.7 k draws). That is the same
//! trade `cad-engine` makes when it pages uploads, and it is taken deliberately:
//! a bounded staging peak is worth a small, bounded draw-plan overhead. The exact
//! draw-count/frame-time delta is evidence-lane work, not a claim made here.

use cad_render_wgpu::canvas::CanvasPresenter;
use cad_render_wgpu::{BackendPreference, PreparedUpload, Renderer};
use cad_scene::{RenderBatch, SceneDelta};
use wasm_bindgen::prelude::JsValue;

use super::import::Loaded;
use super::session_state::{
    wasm_heap_bytes, Failure, Metrics, Session, STEP_DEVICE, STEP_EMPTY_SCENE, STEP_UPLOAD,
};
use crate::budget;

/// Console lines per upload: the progress trace reports about this many steps, so
/// a 72-chunk baseline prints ~10 lines instead of one per chunk.
const PROGRESS_STEPS: usize = 10;

/// Stable prefix of the upload progress console lines. Evidence greps this; it is
/// a diagnostic trace, never user-visible copy.
const UPLOAD_PROGRESS_MARKER: &str = "yacr: upload-progress:";

/// What one scene upload measured. All fields are counts from this open.
pub(super) struct UploadOutcome {
    /// Batches resident after the commit.
    pub(super) batches: usize,
    /// `prepare_upload_batches` calls the upload used.
    pub(super) chunks: usize,
    /// Largest number of batches staged by one call.
    pub(super) peak_batches_per_chunk: usize,
}

/// GPU half of an open: initialize the renderer on `presenter`'s device, upload
/// the resident batches in bounded chunks, and build the session.
pub(super) fn attach(
    presenter: CanvasPresenter,
    metrics: Metrics,
    label: &str,
    mut loaded: Loaded,
) -> Result<Session, Failure> {
    // The renderer shares the presenter's device/queue: the frame texture it
    // renders is the one the presenter blits.
    let mut renderer = Renderer::new(BackendPreference::WebGl2);
    let (device, queue) = presenter.device_queue();
    renderer
        .initialize_with_device(device, queue)
        .map_err(|error| Failure::new(STEP_DEVICE, format!("{label}: {error}")))?;
    let upload_started = web_time::Instant::now();
    let outcome = upload_scene(&mut renderer, label, &mut loaded.delta)?;
    let upload_ms = upload_started.elapsed().as_secs_f64() * 1000.0;
    // Measured before the first frame, while the heap is at its open-time peak.
    // `open_bytes`/`start` refresh it after the first present, so the reported
    // value is the one that belongs to the completed open.
    let heap_after_open = wasm_heap_bytes();
    Ok(Session {
        presenter,
        renderer,
        camera: loaded.camera,
        metrics,
        label: label.to_string(),
        bytes: loaded.bytes,
        batches: outcome.batches,
        draws: 0,
        upload_chunks: outcome.chunks,
        peak_batches_per_chunk: outcome.peak_batches_per_chunk,
        heap_after_open,
        db_retained: budget::DB_RETAINED,
        last_error: None,
        import_ms: loaded.import_ms + upload_ms,
    })
}

/// Stage `delta.added` in bounded chunks, publish it with one atomic commit, then
/// upload the wipeout masks.
///
/// The CPU geometry of each chunk is released immediately after that chunk's
/// buffers are on the device, so the upload never holds the whole
/// import footprint *and* the whole scene *and* a chunk's worth of staging packed
/// intermediates at once.
fn upload_scene(
    renderer: &mut Renderer,
    label: &str,
    delta: &mut SceneDelta,
) -> Result<UploadOutcome, Failure> {
    let total = delta.added.len();
    // Plan once so progress can name a real denominator instead of a guess.
    let planned_chunks = budget::chunk_count(total, |index| delta.added[index].vertices.len());
    let progress_stride = planned_chunks.div_ceil(PROGRESS_STEPS).max(1);

    let mut staged: Option<PreparedUpload> = None;
    let mut start = 0usize;
    let mut chunks = 0usize;
    let mut peak_batches_per_chunk = 0usize;
    while start < total {
        // A chunk never splits a batch, so `end > start` while `start < total`
        // and the loop always terminates.
        let end = budget::next_chunk_end(total, start, |index| delta.added[index].vertices.len());
        let piece = renderer
            .prepare_upload_batches(&delta.added[start..end])
            .map_err(|error| Failure::new(STEP_UPLOAD, format!("{label}: {error}")))?;
        // Every buffer of this chunk has been written to the device; the prepared
        // GPU batches keep no CPU copy of the geometry (assembled streams are
        // packed into their own buffers, not retained). Releasing the source
        // geometry now is what makes the chunking pay off.
        release_uploaded_geometry(&mut delta.added[start..end]);
        match &mut staged {
            // `append` only merges staged pieces; no GPU state changes yet.
            Some(prepared) => prepared
                .append(piece)
                .map_err(|error| Failure::new(STEP_UPLOAD, format!("{label}: {error}")))?,
            None => staged = Some(piece),
        }
        chunks += 1;
        peak_batches_per_chunk = peak_batches_per_chunk.max(end - start);
        start = end;
        if chunks.is_multiple_of(progress_stride) || start == total {
            log_progress(label, chunks, planned_chunks, start, total);
        }
    }

    let prepared = match staged {
        Some(prepared) => prepared,
        // `load` already refused an empty delta; stage an empty upload so the
        // commit below reports zero batches instead of being skipped silently.
        None => renderer
            .prepare_upload_batches(&[])
            .map_err(|error| Failure::new(STEP_UPLOAD, format!("{label}: {error}")))?,
    };
    // One atomic publication of the whole scene: `keep_prefix = 0` replaces the
    // renderer's scene wholesale, so no partially staged scene can ever be drawn.
    renderer
        .commit_upload(prepared, 0)
        .map_err(|error| Failure::new(STEP_UPLOAD, format!("{label}: {error}")))?;
    // Wipeout masks are a separate resident set, not geometry batches (ADR 0022).
    renderer
        .upload_masks(&delta.masks)
        .map_err(|error| Failure::new(STEP_UPLOAD, format!("{label}: {error}")))?;

    let batches = renderer.batch_count();
    if batches == 0 {
        // `load` already refused an empty delta; this catches a renderer that
        // admits every batch and keeps none, so a blank canvas is never success.
        return Err(Failure::new(
            STEP_EMPTY_SCENE,
            format!("{label}: the uploaded scene holds no draw batches"),
        ));
    }
    if batches != total {
        // A complete first frame is the contract: a commit that holds fewer
        // batches than were staged must fail loudly instead of presenting a
        // partial drawing as a successful open.
        return Err(Failure::new(
            STEP_UPLOAD,
            format!(
                "{label}: the commit holds {batches} of {total} staged batches; \
                 refusing to present a partial scene"
            ),
        ));
    }
    Ok(UploadOutcome {
        batches,
        chunks,
        peak_batches_per_chunk,
    })
}

/// Drop the CPU geometry of batches whose buffers are already on the device.
///
/// `prepare_upload_batches` writes every vertex/index/instance/conic buffer to the
/// device synchronously and the staged GPU batches keep only scalars, run indices
/// and the `SelectionRef` list (already cloned by the staging call) — never the
/// source vertices. So the source geometry of a prepared chunk is dead weight and
/// dropping it keeps the upload peak down instead of holding a full second copy of
/// the drawing until the commit.
///
/// What is deliberately **not** dropped: every batch shell stays in `added` (in
/// order), so the staged chunks are appended in the drawing's own order.
///
/// Honest limit of this release: `prepare_upload_batches` hands the bytes to
/// `queue.write_buffer`, and a GL backend may stage that payload in its **own**
/// internal buffer until the first submit. The release above therefore removes
/// *this crate's* copy of the geometry; the backend's staging is not observable
/// from here, and `heap_after_open` — not this reasoning — is the authority on
/// whether the peak actually moved.
fn release_uploaded_geometry(batches: &mut [RenderBatch]) {
    for batch in batches {
        batch.vertices = Vec::new();
        batch.normals = Vec::new();
        batch.colors = Vec::new();
        batch.indices = Vec::new();
        batch.edges = Vec::new();
        batch.conics = Vec::new();
        batch.sources = Vec::new();
    }
}

/// Report real chunk progress on the console (diagnostic trace, not user copy).
fn log_progress(label: &str, chunk: usize, chunks: usize, batches: usize, total: usize) {
    let text = format!(
        "{UPLOAD_PROGRESS_MARKER} {label}: chunk {chunk}/{chunks}, batches {batches}/{total}"
    );
    web_sys::console::log_1(&JsValue::from_str(&text));
}
