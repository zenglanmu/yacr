//! `BLOCK` define-command contract tests.
//!
//! Pins the block-space convention (members keep their geometry bit-for-bit and
//! only change `space`), the reference's identity `T(base) · T(−base)`
//! composition, the one-transaction / one-undo shape (definition + member
//! changes + reference), the undo/redo round trip, and every named refusal.

use super::*;
use cad_db::{BlockDefinition, DrawingDatabaseBuilder, Layer, LayerAttributes, Layout};

/// A layer 0 + WALLS builder, so the lock and paper-space tests have real rows.
fn builder() -> DrawingDatabaseBuilder {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    for (id, name) in [(LayerId(0), "0"), (LayerId(7), "WALLS")] {
        b.insert_layer(Layer {
            id,
            name: name.into(),
            visible: true,
        })
        .unwrap();
    }
    b
}

fn finish(b: DrawingDatabaseBuilder, mode: AppMode) -> (Application, SessionState) {
    let drawing = b.finish().unwrap();
    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([71u8; 32]),
            units: UnitContext::drawing_units(),
            resource_keys: Vec::new(),
        },
    );
    app.workspace.viewports.insert(
        ViewportId(1),
        Viewport::new(ViewportId(1), DocumentId(1), [800.0, 600.0]),
    );
    (app, SessionState::new(DocumentId(1), mode))
}

fn line_entity(
    id: u128,
    start: (f64, f64),
    end: (f64, f64),
    layer: LayerId,
    space: SpaceId,
    order: i64,
) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbLine".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer,
        space,
        geometry: SemanticGeometry::Line {
            start: point(start.0, start.1),
            end: point(end.0, end.1),
        },
        draw_order: order,
    }
}

fn entity(app: &Application, id: u128) -> DbEntity {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(id))
        .cloned()
        .unwrap_or_else(|| panic!("entity {id} should exist"))
}

fn drawing(app: &Application) -> &DrawingDatabase {
    app.workspace.documents[&DocumentId(1)].drawing.as_ref()
}

fn undo_depth(app: &Application) -> usize {
    app.history
        .get(&DocumentId(1))
        .map(|history| history.undo_depth())
        .unwrap_or(0)
}

/// Two non-round model lines on different layers/draw orders, plus an
/// unselected higher-draw-order line that must stay above the new reference.
fn two_line_fixture() -> (Application, SessionState) {
    let mut b = builder();
    b.insert_entity(line_entity(
        1,
        (1.25, 2.5),
        (4.75, 6.125),
        LayerId(0),
        SpaceId::Model,
        2,
    ))
    .unwrap();
    b.insert_entity(line_entity(
        2,
        (10.5, -3.25),
        (12.0, 0.75),
        LayerId(7),
        SpaceId::Model,
        5,
    ))
    .unwrap();
    b.insert_entity(line_entity(
        3,
        (20.0, 20.0),
        (21.0, 21.0),
        LayerId(0),
        SpaceId::Model,
        9,
    ))
    .unwrap();
    finish(b, AppMode::Work)
}

fn define(
    app: &mut Application,
    session: &mut SessionState,
    refs: Vec<SelectionRef>,
    base: Point3,
    name: &str,
) -> CadResult<CommandOutcome> {
    execute(
        app,
        session,
        CommandId::DefineBlock,
        CommandPayload::DefineBlock {
            refs,
            base,
            name: name.to_string(),
        },
    )
}

fn assert_refused(
    app: &mut Application,
    session: &mut SessionState,
    refs: Vec<SelectionRef>,
    base: Point3,
    name: &str,
    code: &str,
) {
    let before_revision = revision(app);
    let before_depth = undo_depth(app);
    let error = define(app, session, refs, base, name).unwrap_err();
    let message = format!("{error}");
    assert!(
        message.contains(code),
        "expected refusal {code}, got {message}"
    );
    assert_eq!(
        revision(app),
        before_revision,
        "{code} must not bump the revision"
    );
    assert_eq!(undo_depth(app), before_depth, "{code} must not add history");
}

#[test]
fn block_define_keeps_member_geometry_and_stores_the_picked_base() {
    let (mut app, mut session) = two_line_fixture();
    let start_revision = revision(&app);
    let before_1 = entity(&app, 1);
    let before_2 = entity(&app, 2);
    let base = point(1.0, 0.0);
    // A live selection is cleared by the command: those ids become members.
    session.selection = SelectionSet::from_refs(vec![ref_of(1), ref_of(2), ref_of(3)]);

    let outcome = define(
        &mut app,
        &mut session,
        vec![ref_of(1), ref_of(2)],
        base,
        "  Widget  ",
    )
    .expect("BLOCK should define the two selected lines");

    // One transaction / one undo step and exactly one revision bump.
    assert_eq!(revision(&app), Revision(start_revision.0 + 1));
    assert_eq!(undo_depth(&app), 1);

    let drawing = drawing(&app);
    let definition = drawing
        .block_by_name("Widget")
        .expect("the trimmed name resolves");
    let new_block = definition.id;
    assert_eq!(definition.entities, vec![EntityId(1), EntityId(2)]);
    let metadata = drawing
        .block_metadata(new_block)
        .expect("the definition carries metadata");
    assert_eq!(metadata.name, "Widget");
    assert_eq!(metadata.base_point, base);

    // Members: identity/layer/order preserved, space changed, geometry identical.
    let after_1 = entity(&app, 1);
    let after_2 = entity(&app, 2);
    for (before, after) in [(&before_1, &after_1), (&before_2, &after_2)] {
        assert_eq!(after.id, before.id);
        assert_eq!(after.layer, before.layer);
        assert_eq!(after.draw_order, before.draw_order);
        assert_eq!(after.space, SpaceId::Block(new_block));
        assert_eq!(
            after.geometry, before.geometry,
            "member geometry must stay bit-for-bit in model-space coordinates"
        );
        assert_eq!(after.object.type_key, before.object.type_key);
    }

    // The reference sits on the active layer in model space with the identity
    // transform (INSERT's own composition at insertion == base).
    let reference_id = EntityId(outcome.objects[0].0);
    let reference = entity(&app, reference_id.0);
    assert_eq!(reference.object.type_key, "AcDbBlockReference");
    assert_eq!(reference.space, SpaceId::Model);
    assert_eq!(reference.layer, session.active_layer);
    assert_eq!(reference.draw_order, 5, "max member draw order");
    match &reference.geometry {
        SemanticGeometry::Insert { block, transform } => {
            assert_eq!(*block, new_block);
            assert_eq!(
                transform.apply_point(base),
                base,
                "the picked base must map to itself (identity reference)"
            );
            assert_eq!(
                transform.apply_point(point(7.5, -1.25)),
                point(7.5, -1.25),
                "no translation may leak into the reference"
            );
        }
        other => panic!("expected an INSERT reference, got {other:?}"),
    }

    // An unselected entity with a higher draw order stays above the reference.
    let unselected = entity(&app, 3);
    assert_eq!(unselected.space, SpaceId::Model);
    assert_eq!(unselected.draw_order, 9);
    assert!(unselected.draw_order > reference.draw_order);

    assert!(
        session.selection.refs().is_empty(),
        "the selected ids are block members now, so the session selection is cleared"
    );
    assert_eq!(outcome.diagnostics[0].code, "drawing.block");
    assert!(outcome.diagnostics[0].message.contains("Widget"));
    assert!(outcome.diagnostics[0].message.contains("2 member"));
}

#[test]
fn block_define_undo_and_redo_apply_all_patches_atomically() {
    let (mut app, mut session) = two_line_fixture();
    let start_revision = revision(&app);
    let before_1 = entity(&app, 1);
    let before_2 = entity(&app, 2);
    let base = point(1.0, 0.0);
    let outcome = define(
        &mut app,
        &mut session,
        vec![ref_of(1), ref_of(2)],
        base,
        "Widget",
    )
    .unwrap();
    let new_block = drawing(&app).block_by_name("Widget").unwrap().id;
    let reference_id = EntityId(outcome.objects[0].0);
    let after_definition = drawing(&app).block(new_block).cloned().unwrap();
    let after_metadata = drawing(&app).block_metadata(new_block).cloned().unwrap();
    let after_1 = entity(&app, 1);
    let after_2 = entity(&app, 2);
    let after_reference = entity(&app, reference_id.0);

    // Undo: members return to model space with identical geometry; the
    // reference and the block row disappear; exactly one revision.
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(revision(&app), Revision(start_revision.0 + 2));
    let drawing_after_undo = drawing(&app);
    assert_eq!(drawing_after_undo.entity(EntityId(1)), Some(&before_1));
    assert_eq!(drawing_after_undo.entity(EntityId(2)), Some(&before_2));
    assert_eq!(drawing_after_undo.entity(reference_id), None);
    assert!(drawing_after_undo.block(new_block).is_none());
    assert!(drawing_after_undo.block_metadata(new_block).is_none());
    assert!(drawing_after_undo.block_by_name("Widget").is_none());
    assert_eq!(undo_depth(&app), 0);
    assert_eq!(app.history.get(&DocumentId(1)).unwrap().redo_depth(), 1);

    // Redo reproduces the exact post-state in one revision.
    execute(
        &mut app,
        &mut session,
        CommandId::Redo,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(revision(&app), Revision(start_revision.0 + 3));
    let drawing_after_redo = drawing(&app);
    assert_eq!(
        drawing_after_redo.block(new_block).cloned(),
        Some(after_definition)
    );
    assert_eq!(
        drawing_after_redo.block_metadata(new_block).cloned(),
        Some(after_metadata)
    );
    assert_eq!(drawing_after_redo.entity(EntityId(1)), Some(&after_1));
    assert_eq!(drawing_after_redo.entity(EntityId(2)), Some(&after_2));
    assert_eq!(
        drawing_after_redo.entity(reference_id),
        Some(&after_reference)
    );
    assert_eq!(undo_depth(&app), 1);

    // Two more cycles are still exactly one step each way.
    for _ in 0..2 {
        let undo_revision = revision(&app).0 + 1;
        execute(
            &mut app,
            &mut session,
            CommandId::Undo,
            CommandPayload::None,
        )
        .unwrap();
        assert_eq!(revision(&app), Revision(undo_revision));
        assert_eq!(drawing(&app).entity(EntityId(1)), Some(&before_1));
        assert!(drawing(&app).block_by_name("Widget").is_none());
        execute(
            &mut app,
            &mut session,
            CommandId::Redo,
            CommandPayload::None,
        )
        .unwrap();
        assert_eq!(revision(&app), Revision(undo_revision + 1));
        assert_eq!(drawing(&app).entity(EntityId(1)), Some(&after_1));
        assert!(drawing(&app).block_by_name("Widget").is_some());
    }
    assert_eq!(undo_depth(&app), 1);
}

#[test]
fn block_define_history_record_is_one_block_patch_plus_entity_patches() {
    let (mut app, mut session) = two_line_fixture();
    define(
        &mut app,
        &mut session,
        vec![ref_of(1), ref_of(2)],
        point(1.0, 0.0),
        "Widget",
    )
    .unwrap();
    let new_block = drawing(&app).block_by_name("Widget").unwrap().id;

    // `begin_drawing_undo` exposes the exact retained record; cancel puts it back
    // without touching the drawing. A record with block patches cannot carry a
    // merge key (history validation refuses that combination), so a successful
    // publish already proves `merge_key == None`; this pins the shape anyway.
    let record = app
        .history
        .get_mut(&DocumentId(1))
        .unwrap()
        .begin_drawing_undo()
        .unwrap();
    assert_eq!(record.label, "define block");
    assert_eq!(record.block_patches.len(), 1);
    assert_eq!(record.block_patches[0].id, new_block);
    assert!(record.block_patches[0].before.is_none());
    assert!(record.block_patches[0].after.is_some());
    assert_eq!(
        record.patches.len(),
        3,
        "two member patches plus one reference creation"
    );
    assert!(record.merge_key.is_none());
    app.history
        .get_mut(&DocumentId(1))
        .unwrap()
        .cancel_pending_drawing();
    assert_eq!(undo_depth(&app), 1);
}

#[test]
fn block_define_allows_a_nested_insert_member_untouched() {
    let mut b = builder();
    // The existing block `Inner` keeps its block-space child.
    b.insert_block(BlockDefinition {
        id: BlockId(0),
        entities: vec![EntityId(10)],
        dynamic_visibility: None,
    })
    .unwrap();
    b.set_block_metadata(BlockId(0), "Inner", point(0.0, 0.0))
        .unwrap();
    b.insert_entity(line_entity(
        10,
        (0.0, 0.0),
        (2.0, 0.0),
        LayerId(0),
        SpaceId::Block(BlockId(0)),
        0,
    ))
    .unwrap();
    // A model-space reference to `Inner`, with a non-identity transform.
    let transform = Transform3::translation(point(3.5, -2.25))
        .matrix_mul(&Transform3::translation(point(0.5, 1.0)));
    let insert = DbEntity {
        object: DbObject {
            id: ObjectId(20),
            type_key: "AcDbBlockReference".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(20),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Insert {
            block: BlockId(0),
            transform,
        },
        draw_order: 4,
    };
    b.insert_entity(insert.clone()).unwrap();
    let (mut app, mut session) = finish(b, AppMode::Work);

    define(
        &mut app,
        &mut session,
        vec![ref_of(20)],
        point(1.0, 0.0),
        "Nested",
    )
    .expect("a nested INSERT is a legal member");

    let new_block = drawing(&app).block_by_name("Nested").unwrap().id;
    let after = entity(&app, 20);
    assert_eq!(after.space, SpaceId::Block(new_block));
    assert_eq!(
        after.geometry, insert.geometry,
        "the nested reference's own transform is untouched"
    );
    match &after.geometry {
        SemanticGeometry::Insert { block, transform } => {
            assert_eq!(*block, BlockId(0));
            let original = match &insert.geometry {
                SemanticGeometry::Insert { transform, .. } => transform,
                other => panic!("expected the original INSERT, got {other:?}"),
            };
            assert_eq!(
                transform.apply_point(point(1.0, 2.0)),
                original.apply_point(point(1.0, 2.0)),
                "world positions still go through the same nested transform"
            );
        }
        other => panic!("expected an INSERT, got {other:?}"),
    }
}

#[test]
fn block_define_refuses_invalid_input_without_writing() {
    let (mut app, mut session) = two_line_fixture();
    let base = point(1.0, 0.0);
    // Empty selection.
    assert_refused(
        &mut app,
        &mut session,
        Vec::new(),
        base,
        "Widget",
        "block.selection_empty",
    );
    // Instance path and sub-element are never reachable as whole members.
    let instance = SelectionRef {
        instance: InstancePath(vec![EntityId(1)]),
        ..ref_of(1)
    };
    assert_refused(
        &mut app,
        &mut session,
        vec![instance],
        base,
        "Widget",
        "block.not_addressable",
    );
    let sub_element = SelectionRef {
        sub_element: Some(SubElementId {
            source_key: "edge".into(),
            topology_revision: Revision(0),
        }),
        ..ref_of(1)
    };
    assert_refused(
        &mut app,
        &mut session,
        vec![sub_element],
        base,
        "Widget",
        "block.not_addressable",
    );
    // Missing target and another document's id.
    assert_refused(
        &mut app,
        &mut session,
        vec![ref_of(999)],
        base,
        "Widget",
        "block.target_missing",
    );
    let other_document = SelectionRef {
        document: DocumentId(2),
        ..ref_of(1)
    };
    assert_refused(
        &mut app,
        &mut session,
        vec![other_document],
        base,
        "Widget",
        "block.target_missing",
    );
    // Name grammar: empty/whitespace, newline, over-long and every illegal char.
    for name in ["", "   ", "A\nB", "A\rB"] {
        assert_refused(
            &mut app,
            &mut session,
            vec![ref_of(1)],
            base,
            name,
            "block.invalid_name",
        );
    }
    let too_long = "x".repeat(MAX_TOOL_TEXT_CHARS + 1);
    assert_refused(
        &mut app,
        &mut session,
        vec![ref_of(1)],
        base,
        &too_long,
        "block.invalid_name",
    );
    for illegal in [
        '<', '>', '/', '\\', '"', ':', ';', '?', '*', '|', '=', '`', '\'',
    ] {
        assert_refused(
            &mut app,
            &mut session,
            vec![ref_of(1)],
            base,
            &format!("a{illegal}b"),
            "block.invalid_name",
        );
    }
    // Non-finite base points.
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert_refused(
            &mut app,
            &mut session,
            vec![ref_of(1)],
            point(bad, 0.0),
            "Widget",
            "block.invalid_point",
        );
    }
}

#[test]
fn block_define_refuses_paper_space_and_block_members() {
    // Paper space needs a real layout row for the builder.
    let mut b = builder();
    b.insert_layout(Layout {
        id: LayoutId(0),
        name: "Layout1".into(),
        viewports: Vec::new(),
    })
    .unwrap();
    b.insert_entity(line_entity(
        5,
        (0.0, 0.0),
        (1.0, 1.0),
        LayerId(0),
        SpaceId::Paper(LayoutId(0)),
        0,
    ))
    .unwrap();
    let (mut app, mut session) = finish(b, AppMode::Work);
    assert_refused(
        &mut app,
        &mut session,
        vec![ref_of(5)],
        point(1.0, 0.0),
        "Widget",
        "block.not_model_space",
    );

    // An existing block's member is already in block space.
    let mut b = builder();
    b.insert_block(BlockDefinition {
        id: BlockId(0),
        entities: vec![EntityId(10)],
        dynamic_visibility: None,
    })
    .unwrap();
    b.set_block_metadata(BlockId(0), "Inner", point(0.0, 0.0))
        .unwrap();
    b.insert_entity(line_entity(
        10,
        (0.0, 0.0),
        (2.0, 0.0),
        LayerId(0),
        SpaceId::Block(BlockId(0)),
        0,
    ))
    .unwrap();
    let (mut app, mut session) = finish(b, AppMode::Work);
    assert_refused(
        &mut app,
        &mut session,
        vec![ref_of(10)],
        point(1.0, 0.0),
        "Widget",
        "block.not_model_space",
    );
}

#[test]
fn block_define_refuses_an_exact_or_case_insensitive_duplicate_name() {
    let mut b = builder();
    b.insert_block(BlockDefinition {
        id: BlockId(0),
        entities: Vec::new(),
        dynamic_visibility: None,
    })
    .unwrap();
    b.set_block_metadata(BlockId(0), "Dup", point(0.0, 0.0))
        .unwrap();
    b.insert_entity(line_entity(
        1,
        (0.0, 0.0),
        (1.0, 0.0),
        LayerId(0),
        SpaceId::Model,
        0,
    ))
    .unwrap();
    let (mut app, mut session) = finish(b, AppMode::Work);
    for name in ["Dup", "dUp", " DUP "] {
        assert_refused(
            &mut app,
            &mut session,
            vec![ref_of(1)],
            point(1.0, 0.0),
            name,
            "block.duplicate_name",
        );
    }
}

#[test]
fn block_define_refuses_locked_member_and_locked_active_layers() {
    // A member on a locked layer is refused by the entity stager.
    let mut b = builder();
    b.set_layer_attributes(
        LayerId(7),
        LayerAttributes {
            locked: true,
            ..LayerAttributes::default()
        },
    )
    .unwrap();
    b.insert_entity(line_entity(
        1,
        (0.0, 0.0),
        (1.0, 0.0),
        LayerId(7),
        SpaceId::Model,
        0,
    ))
    .unwrap();
    let (mut app, mut session) = finish(b, AppMode::Work);
    assert_refused(
        &mut app,
        &mut session,
        vec![ref_of(1)],
        point(1.0, 0.0),
        "Widget",
        "layer.locked",
    );

    // The member is writable, but the reference insert targets the locked active
    // layer.
    let mut b = builder();
    b.set_layer_attributes(
        LayerId(7),
        LayerAttributes {
            locked: true,
            ..LayerAttributes::default()
        },
    )
    .unwrap();
    b.insert_entity(line_entity(
        1,
        (0.0, 0.0),
        (1.0, 0.0),
        LayerId(0),
        SpaceId::Model,
        0,
    ))
    .unwrap();
    let (mut app, mut session) = finish(b, AppMode::Work);
    session.active_layer = LayerId(7);
    assert_refused(
        &mut app,
        &mut session,
        vec![ref_of(1)],
        point(1.0, 0.0),
        "Widget",
        "layer.locked",
    );
}

#[test]
fn block_define_is_work_only() {
    let (mut app, _) = two_line_fixture();
    let mut session = SessionState::new(DocumentId(1), AppMode::Viewer);
    let before_revision = revision(&app);
    let before_depth = undo_depth(&app);
    let error = define(
        &mut app,
        &mut session,
        vec![ref_of(1)],
        point(1.0, 0.0),
        "Widget",
    )
    .unwrap_err();
    assert!(
        matches!(error, CadError::PermissionDenied),
        "Viewer must refuse BLOCK, got {error:?}"
    );
    assert_eq!(revision(&app), before_revision);
    assert_eq!(undo_depth(&app), before_depth);
}
