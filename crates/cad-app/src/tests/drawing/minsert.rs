//! `MINSERT` contract tests: a rectangular array of independent INSERT rows in
//! one transaction, with the importer's `T(insertion) · T(−base_point)` transform
//! at every cell.
//!
//! The block table is read-only; every cell is a real `SemanticGeometry::Insert`
//! reference to the same definition. Refusals are checked to write nothing
//! (no revision bump, no undo entry).

use super::*;
use crate::command_payload::MInsertCommand;

/// A block member (the block-space line) for the fixture.
fn block_member(id: u128, geometry: SemanticGeometry, block: BlockId) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbLine".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space: SpaceId::Block(block),
        geometry,
        draw_order: 0,
    }
}

/// Layer `0`, block `B` (line (0,0)-(2,0), base `(1,0)`), no model rows.
fn minsert_app() -> (Application, SessionState) {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_block(BlockDefinition {
        id: BlockId(0),
        entities: vec![EntityId(100)],
        dynamic_visibility: None,
    })
    .unwrap();
    b.insert_entity(block_member(
        100,
        SemanticGeometry::Line {
            start: point(0.0, 0.0),
            end: point(2.0, 0.0),
        },
        BlockId(0),
    ))
    .unwrap();
    b.set_block_metadata(BlockId(0), "B", point(1.0, 0.0))
        .unwrap();
    let drawing = b.finish().unwrap();
    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([21u8; 32]),
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

fn drawing(app: &Application) -> &DrawingDatabase {
    app.workspace.documents[&DocumentId(1)].drawing.as_ref()
}

fn request(
    block: &str,
    rows: u32,
    columns: u32,
    row_spacing: f64,
    column_spacing: f64,
) -> CommandPayload {
    CommandPayload::MInsert(Box::new(MInsertCommand {
        block: block.into(),
        rows,
        columns,
        row_spacing,
        column_spacing,
        insertion: p3(100.0, 200.0, 0.0),
    }))
}

#[test]
fn minsert_writes_rows_times_columns_independent_references_in_one_transaction() {
    let (mut app, mut session) = minsert_app();
    let before = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::MInsert),
        request("B", 2, 3, 10.0, 20.0),
    )
    .expect("MINSERT inserts six references");
    assert_eq!(outcome.objects.len(), 6);
    let changes = outcome.changes.expect("change set");
    assert_eq!(changes.before, before);
    assert_eq!(changes.after.0, before.0 + 1, "exactly one revision bump");
    assert_eq!(changes.changes.len(), 6, "one transaction, six inserts");

    let references: Vec<&DbEntity> = outcome
        .objects
        .iter()
        .map(|object| {
            drawing(&app)
                .entity(EntityId(object.0))
                .expect("the stored reference")
        })
        .collect();
    // Row-major: (0,0), (0,1), (0,2), (1,0), (1,1), (1,2).
    for (index, entity) in references.iter().enumerate() {
        let row = index / 3;
        let column = index % 3;
        match &entity.geometry {
            SemanticGeometry::Insert { block, transform } => {
                assert_eq!(*block, BlockId(0));
                let cell = p3(100.0 + column as f64 * 20.0, 200.0 + row as f64 * 10.0, 0.0);
                assert_eq!(
                    transform.apply_point(point(1.0, 0.0)),
                    cell,
                    "the block base point lands on cell ({row}, {column})"
                );
            }
            other => panic!("expected an INSERT, got {other:?}"),
        }
        assert_eq!(entity.layer, LayerId(0), "active layer");
        assert_eq!(entity.space, SpaceId::Model);
    }
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 1);
}

#[test]
fn a_one_by_one_minsert_still_writes_the_reference() {
    let (mut app, mut session) = minsert_app();
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::MInsert),
        request("B", 1, 1, 0.0, 0.0),
    )
    .expect("1 x 1 is one reference");
    assert_eq!(
        outcome.objects.len(),
        1,
        "the source cell is the block itself"
    );
}

#[test]
fn minsert_refusals_write_nothing() {
    let cases: Vec<(CommandPayload, &str)> = vec![
        (request("MISSING", 1, 1, 0.0, 0.0), "minsert.unknown_block"),
        (request("B", 0, 1, 0.0, 0.0), "minsert.invalid_count"),
        (request("B", 1, 0, 0.0, 0.0), "minsert.invalid_count"),
        (request("   ", 1, 1, 0.0, 0.0), "minsert.invalid_name"),
        (request("B", 1, 1, f64::NAN, 0.0), "minsert.invalid_spacing"),
        (request("B", 1001, 1000, 0.0, 0.0), "minsert.too_many"),
        (
            CommandPayload::MInsert(Box::new(MInsertCommand {
                block: "B".into(),
                rows: 1,
                columns: 1,
                row_spacing: 0.0,
                column_spacing: 0.0,
                insertion: p3(f64::INFINITY, 0.0, 0.0),
            })),
            "minsert.invalid_point",
        ),
        (CommandPayload::None, "minsert.payload"),
    ];
    for (payload, code) in cases {
        let (mut app, mut session) = minsert_app();
        let before = revision(&app);
        let error = execute(
            &mut app,
            &mut session,
            CommandId::Extended(ExtendedCommandId::MInsert),
            payload,
        )
        .expect_err("refused");
        assert!(error.to_string().contains(code), "{error}");
        assert_eq!(revision(&app), before, "{code} writes nothing");
        assert!(!app.can_undo(&DocumentId(1)), "{code} records no history");
    }
}
