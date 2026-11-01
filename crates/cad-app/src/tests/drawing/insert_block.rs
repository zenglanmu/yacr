//! `INSERT` create-command contract tests (bounded v1: existing blocks only).
//!
//! The command creates **one** `SemanticGeometry::Insert` reference through the
//! shared typed create funnel. The block table is read-only: no definition is
//! added, removed or modified. The transform is the importer's own
//! `T(insertion) · T(−base_point)`, so the block's stored base point lands on the
//! picked insertion point.

use super::*;

/// A model-space block member (the fixture's block-space line).
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

fn fixture(blocks: Vec<BlockDefinition>, metadata: &[(BlockId, &str, Point3)]) -> DrawingDatabase {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    for block in blocks {
        b.insert_block(block).unwrap();
    }
    for (id, name, base) in metadata {
        b.set_block_metadata(*id, name, *base).unwrap();
    }
    b.finish().unwrap()
}

/// Block `B`: a block-space line (0,0)-(2,0), base point (1,0).
fn insert_app() -> (Application, SessionState) {
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
            identity: DocumentIdentity::Sha256([11u8; 32]),
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

fn entity_count(app: &Application) -> usize {
    drawing(app).entity_count()
}

fn block_count(app: &Application) -> usize {
    drawing(app).blocks().count()
}

fn insert(insertion: Point3, name: &str) -> CommandPayload {
    CommandPayload::InsertBlock {
        insertion,
        name: name.into(),
    }
}

#[test]
fn insert_places_a_reference_with_the_base_point_on_the_pick() {
    let (mut app, mut session) = insert_app();
    let before = revision(&app);
    let blocks_before = block_count(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::InsertBlock,
        insert(p3(10.0, 5.0, 0.0), "B"),
    )
    .unwrap();
    assert_eq!(outcome.objects.len(), 1, "one reference entity");
    let changes = outcome.changes.expect("change set");
    assert_eq!(changes.before, before);
    assert_eq!(changes.after.0, before.0 + 1, "exactly one revision bump");
    assert_eq!(changes.changes.len(), 1, "one transaction");

    let entity = drawing(&app)
        .entity(EntityId(outcome.objects[0].0))
        .expect("the stored reference");
    match &entity.geometry {
        SemanticGeometry::Insert { block, transform } => {
            assert_eq!(*block, BlockId(0));
            // The importer's convention: block point `base_point` lands on
            // `insertion`, so the member (0,0)-(2,0) lands at (9,5)-(11,5).
            assert_eq!(
                transform.apply_point(point(1.0, 0.0)),
                p3(10.0, 5.0, 0.0),
                "the base point must land exactly on the insertion point"
            );
            assert_eq!(transform.apply_point(point(0.0, 0.0)), p3(9.0, 5.0, 0.0));
            assert_eq!(transform.apply_point(point(2.0, 0.0)), p3(11.0, 5.0, 0.0));
        }
        other => panic!("expected an INSERT, got {other:?}"),
    }
    assert_eq!(entity.layer, LayerId(0), "active layer");
    assert_eq!(entity.space, SpaceId::Model, "model space");
    assert_eq!(
        entity.object.type_key, "AcDbBlockReference",
        "the typed create funnel stamps the block-reference type key"
    );
    assert!(
        outcome
            .diagnostics
            .iter()
            .any(|d| d.code == "drawing.insert" && d.message.contains("'B'")),
        "the command reports the created reference: {:?}",
        outcome.diagnostics
    );
    assert_eq!(block_count(&app), blocks_before, "no block-table write");
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 1, "one undo step");

    // Undo removes exactly the reference; the block table is untouched.
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert!(drawing(&app)
        .entity(EntityId(outcome.objects[0].0))
        .is_none());
    assert_eq!(block_count(&app), blocks_before);
}

#[test]
fn insert_resolves_trimmed_case_insensitive_names() {
    let (mut app, mut session) = insert_app();
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::InsertBlock,
        insert(p3(0.0, 0.0, 0.0), "  b  "),
    )
    .unwrap();
    assert_eq!(outcome.objects.len(), 1);
    assert!(matches!(
        &drawing(&app)
            .entity(EntityId(outcome.objects[0].0))
            .unwrap()
            .geometry,
        SemanticGeometry::Insert { block, .. } if *block == BlockId(0)
    ));
}

#[test]
fn insert_refusals_change_nothing() {
    let (mut app, mut session) = insert_app();
    let rev = revision(&app);
    let entities = entity_count(&app);
    let long_name = "a".repeat(MAX_TOOL_TEXT_CHARS + 1);

    let cases = [
        (insert(p3(0.0, 0.0, 0.0), "Nope"), "insert.unknown_block"),
        (insert(p3(0.0, 0.0, 0.0), "   "), "insert.invalid_name"),
        (insert(p3(0.0, 0.0, 0.0), "B\nX"), "insert.invalid_name"),
        (insert(p3(0.0, 0.0, 0.0), &long_name), "insert.invalid_name"),
        (insert(p3(f64::NAN, 0.0, 0.0), "B"), "insert.invalid_point"),
    ];
    for (payload, code) in cases {
        let error = execute(&mut app, &mut session, CommandId::InsertBlock, payload).unwrap_err();
        match error {
            CadError::InvalidInput(message) => {
                assert!(message.contains(code), "expected {code}, got {message}")
            }
            other => panic!("expected InvalidInput({code}), got {other:?}"),
        }
        assert_eq!(revision(&app), rev, "{code} must not write");
        assert_eq!(entity_count(&app), entities, "{code} must not write");
        assert!(!app.can_undo(&DocumentId(1)), "{code} must not record undo");
    }
}

#[test]
fn insert_refuses_missing_block_metadata_without_inventing_a_base() {
    // A block with **no** stored name/base point is not addressable by name at
    // all, so the reachable refusal is `insert.unknown_block`. The command's
    // `insert.block_metadata_missing` branch is the defensive second check for a
    // name that resolves but whose metadata is gone; this fixture proves the
    // unnamed block is never silently given a zero base.
    let drawing = fixture(
        vec![BlockDefinition {
            id: BlockId(0),
            entities: Vec::new(),
            dynamic_visibility: None,
        }],
        &[],
    );
    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([12u8; 32]),
            units: UnitContext::drawing_units(),
            resource_keys: Vec::new(),
        },
    );
    app.workspace.viewports.insert(
        ViewportId(1),
        Viewport::new(ViewportId(1), DocumentId(1), [800.0, 600.0]),
    );
    let mut session = SessionState::new(DocumentId(1), AppMode::Work);
    let rev = revision(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::InsertBlock,
        insert(p3(1.0, 2.0, 0.0), "B"),
    )
    .unwrap_err();
    match error {
        CadError::InvalidInput(message) => assert!(
            message.contains("insert.unknown_block"),
            "unnamed blocks are not insertable by name: {message}"
        ),
        other => panic!("expected InvalidInput, got {other:?}"),
    }
    assert_eq!(revision(&app), rev);
}

#[test]
fn insert_accepts_a_block_whose_definition_contains_a_nested_insert() {
    // Block "A" contains a reference to block "Inner". Inserting "A" creates one
    // reference and composes at display time; nothing in either definition is
    // rewritten and no new block row appears.
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
    b.insert_block(BlockDefinition {
        id: BlockId(1),
        entities: vec![EntityId(101)],
        dynamic_visibility: None,
    })
    .unwrap();
    b.insert_entity(block_member(
        100,
        SemanticGeometry::Insert {
            block: BlockId(1),
            transform: Transform3::identity(),
        },
        BlockId(0),
    ))
    .unwrap();
    b.insert_entity(block_member(
        101,
        SemanticGeometry::Line {
            start: point(0.0, 0.0),
            end: point(1.0, 0.0),
        },
        BlockId(1),
    ))
    .unwrap();
    b.set_block_metadata(BlockId(0), "A", point(0.0, 0.0))
        .unwrap();
    b.set_block_metadata(BlockId(1), "Inner", point(0.0, 0.0))
        .unwrap();
    let nested = b.finish().unwrap();

    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(nested),
            identity: DocumentIdentity::Sha256([13u8; 32]),
            units: UnitContext::drawing_units(),
            resource_keys: Vec::new(),
        },
    );
    app.workspace.viewports.insert(
        ViewportId(1),
        Viewport::new(ViewportId(1), DocumentId(1), [800.0, 600.0]),
    );
    let mut session = SessionState::new(DocumentId(1), AppMode::Work);
    let blocks_before = block_count(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::InsertBlock,
        insert(p3(4.0, 4.0, 0.0), "A"),
    )
    .unwrap();
    assert_eq!(outcome.objects.len(), 1);
    assert!(matches!(
        &drawing(&app)
            .entity(EntityId(outcome.objects[0].0))
            .unwrap()
            .geometry,
        SemanticGeometry::Insert { block, .. } if *block == BlockId(0)
    ));
    assert_eq!(block_count(&app), blocks_before, "no definition write");
}

#[test]
fn insert_is_work_only_and_viewer_refuses_before_any_write() {
    let (mut app, mut session) = insert_app();
    let rev = revision(&app);
    session.switch_mode(AppMode::Viewer).unwrap();
    let error = execute(
        &mut app,
        &mut session,
        CommandId::InsertBlock,
        insert(p3(1.0, 2.0, 0.0), "B"),
    )
    .unwrap_err();
    assert!(matches!(error, CadError::PermissionDenied), "{error:?}");
    assert_eq!(revision(&app), rev);
    assert!(!app.can_undo(&DocumentId(1)));
}
