//! Layout-table undo/redo plumbing.
//!
//! `cad_db::LayoutPatch` + `DrawingUndoRecord::layout_patches` commit a layout
//! row (and its plot settings) through the same validated combined write path as
//! an entity patch. This file pins that the ordinary `UNDO`/`REDO` commands
//! replay a layout-only record as exactly one revision per direction and restore
//! the recorded row with its page setup verbatim (a delete would remove both).
//! No command records one yet; the record below is built by hand, exactly like
//! `layer_undo.rs` and `block_undo.rs`.

use super::*;
use cad_history::{layout_patch, DrawingUndoRecord};

fn plot_settings(layout: u128) -> cad_db::PlotSettingsRecord {
    cad_db::PlotSettingsRecord {
        layout: LayoutId(layout),
        paper_size_name: "ISO_A4".into(),
        paper_width: 210.0,
        paper_height: 297.0,
        margins: cad_db::PlotMargins::uniform(5.0),
        rotation: cad_db::PlotRotation::None,
        scale_numerator: 1.0,
        scale_denominator: 1.0,
        plot_type: cad_db::PlotType::Layout,
        paper_units: cad_db::PlotPaperUnits::Millimeters,
        provenance: cad_db::PlotProvenance::Imported,
    }
}

/// Layer 0 and layout 1 (`Layout1`); the test creates layout 2 through the same
/// combined entry the replay uses.
fn layout_undo_app() -> (Application, SessionState) {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_layout(Layout {
        id: LayoutId(1),
        name: "Layout1".into(),
        viewports: Vec::new(),
    })
    .unwrap();
    b.set_plot_settings(plot_settings(1)).unwrap();
    let drawing = b.finish().unwrap();

    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([9u8; 32]),
            units: UnitContext::drawing_units(),
            resource_keys: Vec::new(),
        },
    );
    app.workspace.viewports.insert(
        ViewportId(1),
        Viewport::new(ViewportId(1), DocumentId(1), [800.0, 600.0]),
    );
    (app, SessionState::new(DocumentId(1), AppMode::Work))
}

#[test]
fn layout_patch_undo_and_redo_restore_row_and_plot_settings_in_one_revision_each() {
    let (mut app, mut session) = layout_undo_app();
    let document_id = DocumentId(1);
    let drawing = Arc::make_mut(
        &mut app
            .workspace
            .documents
            .get_mut(&document_id)
            .unwrap()
            .drawing,
    );
    let start_revision = drawing.revision();
    assert!(drawing.layout(LayoutId(2)).is_none());

    let created = cad_db::LayoutRow {
        layout: Layout {
            id: LayoutId(2),
            name: "Sheet".into(),
            viewports: Vec::new(),
        },
        plot: Some(plot_settings(2)),
    };

    // Put the authoritative drawing into the recorded `after` state through the
    // same combined entry the replay uses, so this test only exercises history.
    drawing
        .apply_drawing_changes_with_blocks_display_layers_and_layouts(
            "create layout",
            TransactionId(41),
            &[],
            Vec::new(),
            &[],
            &[],
            &[(LayoutId(2), Some(created.clone()))],
        )
        .expect("the create batch is valid");
    let applied_revision = drawing.revision();
    assert_eq!(applied_revision, Revision(start_revision.0 + 1));
    assert_eq!(drawing.plot_settings(LayoutId(2)), Some(&plot_settings(2)));

    app.history
        .entry(document_id)
        .or_default()
        .record_drawing(DrawingUndoRecord {
            transaction: TransactionId(41),
            label: "create layout".into(),
            patches: Vec::new(),
            block_patches: Vec::new(),
            layer_patches: Vec::new(),
            layout_patches: vec![layout_patch(LayoutId(2), None, Some(created.clone()))],
            merge_key: None,
        })
        .expect("one valid layout undo step");

    // Undo: exactly one revision, the layout and its plot row are removed.
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    let drawing = &app.workspace.documents[&document_id].drawing;
    assert_eq!(
        drawing.revision(),
        Revision(applied_revision.0 + 1),
        "undo is exactly one revision"
    );
    assert!(
        drawing.layout(LayoutId(2)).is_none(),
        "the created layout is gone"
    );
    assert!(
        drawing.plot_settings(LayoutId(2)).is_none(),
        "the plot settings row never outlives its layout"
    );
    assert!(drawing.layout(LayoutId(1)).is_some(), "other layouts stay");
    assert!(app.can_redo(&document_id));
    assert!(!app.can_undo(&document_id));

    // Redo: the row and its page setup come back in exactly one revision.
    execute(
        &mut app,
        &mut session,
        CommandId::Redo,
        CommandPayload::None,
    )
    .unwrap();
    let drawing = &app.workspace.documents[&document_id].drawing;
    assert_eq!(
        drawing.revision(),
        Revision(applied_revision.0 + 2),
        "redo is exactly one revision"
    );
    assert_eq!(
        drawing
            .layout(LayoutId(2))
            .map(|layout| layout.name.clone()),
        Some("Sheet".to_string())
    );
    assert_eq!(
        drawing.plot_settings(LayoutId(2)),
        Some(&plot_settings(2)),
        "the recorded page setup is restored verbatim"
    );
    assert_eq!(
        app.history
            .get(&document_id)
            .map(|history| history.undo_depth()),
        Some(1)
    );
}
