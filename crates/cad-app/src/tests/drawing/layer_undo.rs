//! Layer-table undo/redo plumbing.
//!
//! `cad_db::LayerPatch` + `DrawingUndoRecord::layer_patches` commit a layer row
//! (and its attributes) through the same validated combined write path as an
//! entity patch. This file pins that the ordinary `UNDO`/`REDO` commands replay
//! a layer-only record as exactly one revision per direction and restore the
//! recorded [`cad_db::LayerAttributes`] verbatim. No command records one yet;
//! the record below is built by hand, exactly like `block_undo.rs`.

use super::*;
use cad_history::{layer_patch, DrawingUndoRecord};

/// Layer 0 and an unfrozen `WALLS` (id 5).
fn layer_undo_app() -> (Application, SessionState) {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    for (id, name) in [(LayerId(0), "0"), (LayerId(5), "WALLS")] {
        b.insert_layer(Layer {
            id,
            name: name.into(),
            visible: true,
        })
        .unwrap();
    }
    let drawing = b.finish().unwrap();

    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([8u8; 32]),
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
fn layer_patch_undo_and_redo_restore_attributes_in_one_revision_each() {
    let (mut app, mut session) = layer_undo_app();
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

    let before_attributes = drawing.layer_attributes(LayerId(5));
    assert!(!before_attributes.frozen, "the fixture layer is thawed");
    let before_row = cad_db::LayerRow {
        layer: drawing.layer(LayerId(5)).expect("fixture layer").clone(),
        attributes: before_attributes.clone(),
    };
    let mut frozen_attributes = before_attributes.clone();
    frozen_attributes.frozen = true;
    let after_row = cad_db::LayerRow {
        layer: drawing.layer(LayerId(5)).expect("fixture layer").clone(),
        attributes: frozen_attributes.clone(),
    };

    // Put the authoritative drawing into the recorded `after` state through the
    // same combined entry the replay uses, so this test only exercises history.
    drawing
        .apply_drawing_changes_with_blocks_display_and_layers(
            "freeze layer",
            TransactionId(31),
            &[],
            Vec::new(),
            &[],
            &[(LayerId(5), Some(after_row.clone()))],
        )
        .expect("the freeze batch is valid");
    let applied_revision = drawing.revision();
    assert_eq!(applied_revision, Revision(start_revision.0 + 1));
    assert_eq!(drawing.layer_attributes(LayerId(5)), frozen_attributes);

    app.history
        .entry(document_id)
        .or_default()
        .record_drawing(DrawingUndoRecord {
            transaction: TransactionId(31),
            label: "freeze layer".into(),
            patches: Vec::new(),
            block_patches: Vec::new(),
            layer_patches: vec![layer_patch(
                LayerId(5),
                Some(before_row.clone()),
                Some(after_row.clone()),
            )],
            layout_patches: Vec::new(),
            merge_key: None,
        })
        .expect("one valid layer undo step");

    // Undo: exactly one revision, the stored attributes and the derived
    // `visible` flag both return to the recorded before state.
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
    assert_eq!(
        drawing.layer_attributes(LayerId(5)),
        before_attributes,
        "the before attributes are restored verbatim"
    );
    assert_eq!(
        drawing.layer(LayerId(5)).map(|layer| layer.visible),
        Some(true),
        "visible is derived from the restored attributes"
    );
    assert!(app.can_redo(&document_id));
    assert!(!app.can_undo(&document_id));

    // Redo: the same row comes back in exactly one revision.
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
        drawing.layer_attributes(LayerId(5)),
        frozen_attributes,
        "the after attributes are restored verbatim"
    );
    assert_eq!(
        drawing.layer(LayerId(5)).map(|layer| layer.visible),
        Some(false),
        "a frozen row is not visible"
    );
    assert_eq!(
        app.history
            .get(&document_id)
            .map(|history| history.undo_depth()),
        Some(1)
    );
}
