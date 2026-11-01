//! Block-aware undo/redo plumbing.
//!
//! Lane A's [`DrawingDatabase::apply_drawing_changes_with_blocks`] commits a
//! definition row and its member/reference entity batch as one revision. This
//! file pins that the history record carries the matching [`cad_db::BlockPatch`]
//! and that the ordinary undo/redo commands replay both batches, so a future
//! `BLOCK` command is one atomic undo step (no command is added here).

use super::*;
use cad_history::{drawing_patch, DrawingUndoRecord};

fn line_entity(id: u128, start: Point3, end: Point3, order: i64) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbLine".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Line { start, end },
        draw_order: order,
    }
}

/// Layer 0 and one model-space line (entity 1) at `(0,0) -> (4,0)`.
fn block_undo_app() -> (Application, SessionState) {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_entity(line_entity(1, point(0.0, 0.0), point(4.0, 0.0), 0))
        .unwrap();
    let drawing = b.finish().unwrap();

    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([7u8; 32]),
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

/// A `BLOCK`-shaped combined edit: create the definition + metadata, move the
/// member into `SpaceId::Block` translated by `T(-base)`, insert the reference
/// carrying `T(base)`, then record exactly the patches that reverse it.
#[test]
fn block_create_undo_and_redo_apply_all_patches_in_one_revision_each() {
    let (mut app, mut session) = block_undo_app();
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
    let block = drawing.allocate_block_id();
    let base = point(2.0, 0.0);

    let before_member = drawing.entity(EntityId(1)).expect("fixture line").clone();
    let mut after_member = before_member.clone();
    after_member.geometry = cad_db::transform_geometry(
        &before_member.geometry,
        &Transform3::translation(point(-base.x, -base.y)),
    )
    .unwrap();
    after_member.space = SpaceId::Block(block);
    let reference = DbEntity {
        object: DbObject {
            id: ObjectId(2),
            type_key: "AcDbBlockReference".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(2),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Insert {
            block,
            transform: Transform3::translation(base),
        },
        draw_order: 1,
    };
    let after_row = cad_db::BlockRow {
        definition: cad_db::BlockDefinition {
            id: block,
            entities: vec![EntityId(1)],
            dynamic_visibility: None,
        },
        metadata: Some(cad_db::BlockMetadata {
            name: "B1".into(),
            base_point: base,
        }),
    };

    let changes = drawing
        .apply_drawing_changes_with_blocks(
            "create block",
            TransactionId(41),
            &[(block, Some(after_row.clone()))],
            vec![
                (EntityId(1), Some(after_member.clone())),
                (EntityId(2), Some(reference.clone())),
            ],
        )
        .expect("Lane A combined create");
    let created_revision = changes.after;
    assert_eq!(changes.before, start_revision);
    assert_eq!(created_revision, Revision(start_revision.0 + 1));
    assert_eq!(drawing.revision(), created_revision);
    assert_eq!(drawing.block_name(block), Some("B1"));
    assert_eq!(drawing.entity(EntityId(1)), Some(&after_member));
    assert_eq!(drawing.entity(EntityId(2)), Some(&reference));

    app.history
        .entry(document_id)
        .or_default()
        .record_drawing(DrawingUndoRecord {
            transaction: changes.transaction,
            label: "create block".into(),
            patches: vec![
                drawing_patch(
                    EntityId(1),
                    Some(before_member.clone()),
                    Some(after_member.clone()),
                ),
                drawing_patch(EntityId(2), None, Some(reference.clone())),
            ],
            block_patches: vec![cad_db::BlockPatch {
                id: block,
                before: None,
                after: Some(after_row.clone()),
            }],
            merge_key: None,
        })
        .expect("one valid block undo step");

    // Undo: members return before the definition is removed, so the block
    // delete is legal. Exactly one revision, one history step.
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
        Revision(created_revision.0 + 1),
        "undo is exactly one revision"
    );
    assert_eq!(drawing.entity(EntityId(1)), Some(&before_member));
    assert_eq!(
        drawing.entity(EntityId(2)),
        None,
        "the reference is removed"
    );
    assert_eq!(drawing.entity_count(), 1);
    assert!(drawing.block(block).is_none(), "definition row removed");
    assert!(
        drawing.block_metadata(block).is_none(),
        "metadata row removed"
    );
    assert!(drawing.block_name(block).is_none());
    assert!(
        drawing.block_by_name("B1").is_none(),
        "name index entry removed"
    );
    assert!(app.can_redo(&document_id));
    assert!(!app.can_undo(&document_id));

    // Redo: the definition, metadata, member move and reference come back as
    // the exact recorded rows in one revision.
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
        Revision(created_revision.0 + 2),
        "redo is exactly one revision"
    );
    assert_eq!(
        drawing.block(block).cloned(),
        Some(after_row.definition.clone()),
        "the definition row is restored verbatim"
    );
    assert_eq!(
        drawing.block_metadata(block).cloned(),
        after_row.metadata.clone(),
        "the metadata row is restored verbatim"
    );
    assert_eq!(drawing.block_name(block), Some("B1"));
    assert_eq!(drawing.block_base_point(block), Some(base));
    assert_eq!(
        drawing.block_by_name("B1").map(|definition| definition.id),
        Some(block),
        "the name index resolves the restored row again"
    );
    assert_eq!(drawing.entity(EntityId(1)), Some(&after_member));
    assert_eq!(drawing.entity(EntityId(2)), Some(&reference));
    assert_eq!(
        app.history
            .get(&document_id)
            .map(|history| history.undo_depth()),
        Some(1),
        "undo and redo are still one step"
    );
    assert!(app.can_undo(&document_id));
    assert!(!app.can_redo(&document_id));
}
