//! One bounded staging step shared by the upload pump.
//!
//! Extracted from `session.rs` verbatim so the runtime module stays under the
//! repository's per-file size limit; the behaviour and the documented invariants
//! are unchanged.

use super::*;

/// Stage one bounded batch slice and append it to the piece being built.
///
/// Group 0 (the persistent base) converts to the stream/chunk/arc classes; group
/// 1 (the transient overlay) is staged **legacy**. The base range is
/// materialised from [`cad_app::render_scene::ResidentScene::materialized`],
/// which clones the contents of exactly this bounded piece (at most 256 batches /
/// 262,144 vertices). That copy is O(piece), never O(scene): appending a piece is
/// one entity and this path never runs per frame, while a full re-stage (device
/// loss, declined append, refused overwrite) is already the slow path.
pub(super) fn stage_piece(
    group: usize,
    renderer: &mut Renderer,
    batches: &[cad_scene::RenderBatch],
    prepared: &mut Option<cad_render_wgpu::PreparedUpload>,
) -> CadResult<()> {
    let piece = if group == 0 {
        renderer.prepare_upload_batches(batches)?
    } else {
        renderer.prepare_upload_batches_legacy(batches)?
    };
    match prepared {
        Some(existing) => existing.append(piece),
        None => {
            *prepared = Some(piece);
            Ok(())
        }
    }
}
