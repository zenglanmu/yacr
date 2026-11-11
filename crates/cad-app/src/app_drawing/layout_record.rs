//! The recording half of layout-table writes.
//!
//! A layout-table edit that a handler wants on the undo stack records one
//! [`cad_db::LayoutPatch`] per row (create = `before: None`, delete =
//! `after: None`) and publishes the candidate that produced it. Replay goes
//! through the database's validated combined path
//! ([`cad_db::DrawingDatabase::apply_drawing_changes_with_blocks_display_layers_and_layouts`]),
//! so the layout rows (and their plot settings) land in the same revision and
//! `ChangeSet` shape as every other edit, and a delete is applied after its
//! entity deletes.
//!
//! The layout command handlers ([`crate::app_layout`]) record through these
//! helpers, exactly like the layer writes use `layer_record`.

use super::*;

/// Read one layout table row (the [`cad_db::Layout`] plus its plot settings) as
/// the before/after unit a [`cad_db::LayoutPatch`] carries.
///
/// Returns `None` when the id names no layout row, so a create records
/// `before: None` and a delete records `after: None` without inventing a row.
/// The plot row travels with the layout because a delete cascades to it and an
/// undo must restore both.
pub(crate) fn layout_row(database: &DrawingDatabase, id: LayoutId) -> Option<cad_db::LayoutRow> {
    database.layout(id).map(|layout| cad_db::LayoutRow {
        layout: layout.clone(),
        plot: database.plot_settings(id).cloned(),
    })
}

impl Application {
    /// Record one layout-table undo step and publish the candidate that produced
    /// it.
    ///
    /// The record carries `layout_patches` and no entity/block/layer patches, so
    /// undo/redo replays it through the same validated combined write path
    /// (layout upserts first, layout deletes last, one revision and one
    /// `ChangeSet`). History is validated **before** the candidate replaces the
    /// authoritative drawing, so a rejected record leaves both the drawing and
    /// the history untouched.
    pub(crate) fn publish_layout_record(
        &mut self,
        document_id: DocumentId,
        candidate: DrawingDatabase,
        transaction: TransactionId,
        label: &str,
        layout_patches: Vec<cad_db::LayoutPatch>,
    ) -> CadResult<()> {
        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction,
                label: label.to_string(),
                patches: Vec::new(),
                block_patches: Vec::new(),
                layer_patches: Vec::new(),
                layout_patches,
                style_patches: Vec::new(),
                dim_style_patches: Vec::new(),
                mleader_style_patches: Vec::new(),
                merge_key: None,
            },
        )
    }
}
