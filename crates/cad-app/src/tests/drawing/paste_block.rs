//! `PASTEBLOCK` contract tests.
//!
//! Pins the declared shape: the clipboard becomes one **auto-named** block
//! definition plus one model-space reference, member geometry stays bit-for-bit,
//! the reference translates by `T(insertion) · T(−clipboard base)`, the whole
//! thing is one transaction / one undo step, and every refusal leaves the
//! drawing, the block table, the history and the clipboard untouched.

use super::*;
use cad_db::{BlockDefinition, DrawingDatabaseBuilder, Layer};
use cad_domain::BlockId;

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
            identity: DocumentIdentity::Sha256([91u8; 32]),
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

/// Two model lines (draw orders 2 and 5, one on WALLS) plus an unselected line.
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

fn copyclip(app: &mut Application, session: &mut SessionState, ids: &[u128]) {
    execute(
        app,
        session,
        CommandId::Select,
        CommandPayload::Selection(ids.iter().map(|id| ref_of(*id)).collect()),
    )
    .unwrap();
    execute(
        app,
        session,
        CommandId::CopyToClipboard,
        CommandPayload::None,
    )
    .unwrap();
}

fn copybase(app: &mut Application, session: &mut SessionState, ids: &[u128], base: Point3) {
    execute(
        app,
        session,
        CommandId::CopyToClipboardBase,
        CommandPayload::CopyBase {
            refs: ids.iter().map(|id| ref_of(*id)).collect(),
            base,
        },
    )
    .unwrap();
}

fn paste_block(
    app: &mut Application,
    session: &mut SessionState,
    insertion: Point3,
) -> CadResult<CommandOutcome> {
    execute(
        app,
        session,
        CommandId::PasteBlock,
        CommandPayload::PasteBlock { insertion },
    )
}

fn assert_refused(
    app: &mut Application,
    session: &mut SessionState,
    insertion: Point3,
    code: &str,
) {
    let before_revision = revision(app);
    let before_depth = undo_depth(app);
    let before_clipboard = session.clipboard_len();
    let before_selection = session.selection.refs().to_vec();
    let error = paste_block(app, session, insertion).unwrap_err();
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
    assert_eq!(
        session.clipboard_len(),
        before_clipboard,
        "{code} must not clear the clipboard"
    );
    assert_eq!(
        session.selection.refs(),
        before_selection.as_slice(),
        "{code} must not change the selection"
    );
}

#[test]
fn pasteblock_creates_one_auto_named_block_and_reference() {
    let (mut app, mut session) = two_line_fixture();
    let start_revision = revision(&app);
    let before_1 = entity(&app, 1);
    let before_2 = entity(&app, 2);
    copyclip(&mut app, &mut session, &[1, 2]);
    assert_eq!(session.clipboard_len(), 2);
    assert!(
        session.clipboard_base.is_none(),
        "COPYCLIP records no base point"
    );

    let insertion = point(10.0, 5.0);
    let outcome = paste_block(&mut app, &mut session, insertion).expect("PASTEBLOCK should paste");

    // One transaction / one undo step / one revision bump.
    assert_eq!(revision(&app), Revision(start_revision.0 + 1));
    assert_eq!(undo_depth(&app), 1);

    let definition = drawing(&app)
        .block_by_name("PasteBlock1")
        .expect("the first auto name is PasteBlock1");
    let new_block = definition.id;
    assert_eq!(definition.entities.len(), 2);
    let metadata = drawing(&app)
        .block_metadata(new_block)
        .expect("the definition carries metadata");
    assert_eq!(metadata.name, "PasteBlock1");
    assert_eq!(
        metadata.base_point,
        point(0.0, 0.0),
        "a clipboard without a base uses the world origin"
    );

    // Two fresh member ids in `SpaceId::Block` with bit-identical geometry.
    let member_ids = definition.entities.clone();
    for (id, before) in member_ids.iter().zip([&before_1, &before_2]) {
        assert_ne!(*id, before.id, "members must use fresh ids");
        let after = entity(&app, id.0);
        assert_eq!(after.space, SpaceId::Block(new_block));
        assert_eq!(after.layer, before.layer);
        assert_eq!(after.draw_order, before.draw_order);
        assert_eq!(
            after.geometry, before.geometry,
            "member geometry must stay bit-for-bit in model-space coordinates"
        );
    }

    // The reference maps the clipboard origin onto the insertion point, and every
    // member's world position is the original translated by (10, 5).
    let reference_id = EntityId(outcome.objects[0].0);
    let reference = entity(&app, reference_id.0);
    assert_eq!(reference.space, SpaceId::Model);
    assert_eq!(reference.object.type_key, "AcDbBlockReference");
    assert_eq!(reference.layer, session.active_layer);
    let delta = point(10.0, 5.0);
    let transform = match &reference.geometry {
        SemanticGeometry::Insert { block, transform } => {
            assert_eq!(*block, new_block);
            *transform
        }
        other => panic!("expected an INSERT reference, got {other:?}"),
    };
    assert_eq!(transform.apply_point(point(0.0, 0.0)), delta);
    for before in [&before_1, &before_2] {
        let original = match &before.geometry {
            SemanticGeometry::Line { start, .. } => *start,
            other => panic!("expected a line, got {other:?}"),
        };
        assert_eq!(
            transform.apply_point(original),
            Point3 {
                x: original.x + delta.x,
                y: original.y + delta.y,
                z: original.z + delta.z,
            },
            "member world positions move by the insertion delta"
        );
    }

    // The clipboard survives untouched and the reference is exactly the selection.
    assert_eq!(session.clipboard_len(), 2);
    assert!(session.clipboard_base.is_none());
    assert_eq!(session.selection.refs(), &[ref_of(reference_id.0)]);
    assert_eq!(outcome.diagnostics[0].code, "drawing.pasteblock");
    assert!(outcome.diagnostics[0].message.contains("PasteBlock1"));
    assert!(outcome.diagnostics[0].message.contains("2 member"));
}

#[test]
fn pasteblock_undo_and_redo_are_one_step() {
    let (mut app, mut session) = two_line_fixture();
    let start_revision = revision(&app);
    copyclip(&mut app, &mut session, &[1, 2]);
    let outcome = paste_block(&mut app, &mut session, point(10.0, 5.0)).unwrap();
    let new_block = drawing(&app).block_by_name("PasteBlock1").unwrap().id;
    let reference_id = EntityId(outcome.objects[0].0);
    let member_ids = drawing(&app).block(new_block).unwrap().entities.clone();
    let after_definition = drawing(&app).block(new_block).cloned().unwrap();
    let after_metadata = drawing(&app).block_metadata(new_block).cloned().unwrap();
    let after_reference = entity(&app, reference_id.0);
    let after_members: Vec<DbEntity> = member_ids.iter().map(|id| entity(&app, id.0)).collect();

    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(revision(&app), Revision(start_revision.0 + 2));
    let after_undo = drawing(&app);
    assert_eq!(after_undo.entity(reference_id), None, "reference removed");
    for id in &member_ids {
        assert_eq!(after_undo.entity(*id), None, "member {id:?} removed");
    }
    assert!(after_undo.block(new_block).is_none());
    assert!(after_undo.block_metadata(new_block).is_none());
    assert!(after_undo.block_by_name("PasteBlock1").is_none());
    assert_eq!(undo_depth(&app), 0);
    assert_eq!(app.history.get(&DocumentId(1)).unwrap().redo_depth(), 1);

    execute(
        &mut app,
        &mut session,
        CommandId::Redo,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(revision(&app), Revision(start_revision.0 + 3));
    let after_redo = drawing(&app);
    assert_eq!(after_redo.block(new_block).cloned(), Some(after_definition));
    assert_eq!(
        after_redo.block_metadata(new_block).cloned(),
        Some(after_metadata)
    );
    assert_eq!(after_redo.entity(reference_id), Some(&after_reference));
    for (id, after) in member_ids.iter().zip(&after_members) {
        assert_eq!(after_redo.entity(*id), Some(after));
    }
    assert_eq!(undo_depth(&app), 1);
}

#[test]
fn pasteblock_translates_from_the_copybase_base_point() {
    let (mut app, mut session) = two_line_fixture();
    let before_1 = entity(&app, 1);
    let base = point(1.0, 0.0);
    copybase(&mut app, &mut session, &[1], base);
    assert_eq!(session.clipboard_base, Some(base));

    let insertion = point(10.0, 5.0);
    paste_block(&mut app, &mut session, insertion).expect("COPYBASE clipboard pastes as a block");

    let new_block = drawing(&app).block_by_name("PasteBlock1").unwrap().id;
    let metadata = drawing(&app).block_metadata(new_block).unwrap();
    assert_eq!(
        metadata.base_point, base,
        "the clipboard base is stored as the block base point"
    );
    let model_space = drawing(&app).model_space();
    let reference = model_space
        .iter()
        .find(|candidate| {
            matches!(
                &candidate.geometry,
                SemanticGeometry::Insert { block, .. } if *block == new_block
            )
        })
        .expect("the reference is a model-space INSERT");
    let transform = match &reference.geometry {
        SemanticGeometry::Insert { transform, .. } => *transform,
        other => panic!("expected an INSERT, got {other:?}"),
    };
    assert_eq!(
        transform.apply_point(base),
        insertion,
        "the captured base maps to the insertion point"
    );
    // Members are stored untranslated; the reference supplies the (10,5)-(1,0)
    // translation at display time.
    let delta = point(9.0, 5.0);
    let original = match &before_1.geometry {
        SemanticGeometry::Line { start, .. } => *start,
        other => panic!("expected a line, got {other:?}"),
    };
    assert_eq!(
        transform.apply_point(original),
        Point3 {
            x: original.x + delta.x,
            y: original.y + delta.y,
            z: original.z + delta.z,
        }
    );
}

#[test]
fn pasteblock_refuses_bad_input_without_writing() {
    // Empty clipboard.
    let (mut app, mut session) = two_line_fixture();
    assert_refused(
        &mut app,
        &mut session,
        point(0.0, 0.0),
        "pasteblock.empty_clipboard",
    );

    // Non-finite insertion point.
    copyclip(&mut app, &mut session, &[1]);
    assert_refused(
        &mut app,
        &mut session,
        point(f64::NAN, 0.0),
        "pasteblock.invalid_point",
    );

    // Non-finite clipboard base (possible only via session state, never capture).
    session.clipboard_base = Some(point(f64::INFINITY, 0.0));
    assert_refused(
        &mut app,
        &mut session,
        point(0.0, 0.0),
        "pasteblock.invalid_point",
    );
    session.clipboard_base = None;

    // A block-member entry (non-model space) is refused.
    let (mut app, mut session) = two_line_fixture();
    let block_member = line_entity(
        50,
        (0.0, 0.0),
        (1.0, 0.0),
        LayerId(0),
        SpaceId::Block(BlockId(0)),
        0,
    );
    session.clipboard = vec![ClipboardEntry {
        display: drawing(&app).entity_display_state(EntityId(50)),
        entity: block_member,
    }];
    assert_refused(
        &mut app,
        &mut session,
        point(0.0, 0.0),
        "pasteblock.not_model_space",
    );

    // A nested INSERT to a block that does not exist is refused.
    let (mut app, mut session) = two_line_fixture();
    let missing = DbEntity {
        object: DbObject {
            id: ObjectId(51),
            type_key: "AcDbBlockReference".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(51),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Insert {
            block: BlockId(999),
            transform: Transform3::identity(),
        },
        draw_order: 0,
    };
    session.clipboard = vec![ClipboardEntry {
        display: drawing(&app).entity_display_state(EntityId(51)),
        entity: missing,
    }];
    assert_refused(
        &mut app,
        &mut session,
        point(0.0, 0.0),
        "pasteblock.missing_block",
    );
}

#[test]
fn pasteblock_is_refused_in_viewer_mode() {
    let (mut app, mut session) = two_line_fixture();
    copyclip(&mut app, &mut session, &[1]);
    let before_revision = revision(&app);
    let before_depth = undo_depth(&app);
    let mut viewer = SessionState::new(DocumentId(1), AppMode::Viewer);
    viewer.clipboard = session.clipboard.clone();
    let error = paste_block(&mut app, &mut viewer, point(0.0, 0.0)).unwrap_err();
    assert_eq!(error, CadError::PermissionDenied);
    assert_eq!(revision(&app), before_revision);
    assert_eq!(undo_depth(&app), before_depth);
    assert_eq!(viewer.clipboard_len(), 1);
}

#[test]
fn pasteblock_allows_a_nested_insert_entry() {
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
            transform: Transform3::translation(point(3.5, -2.25)),
        },
        draw_order: 4,
    };
    b.insert_entity(insert.clone()).unwrap();
    let (mut app, mut session) = finish(b, AppMode::Work);
    copyclip(&mut app, &mut session, &[20]);

    paste_block(&mut app, &mut session, point(0.0, 0.0))
        .expect("a nested INSERT whose block exists is a legal member");

    let new_block = drawing(&app).block_by_name("PasteBlock1").unwrap().id;
    let member_id = drawing(&app).block(new_block).unwrap().entities[0];
    let member = entity(&app, member_id.0);
    assert_eq!(member.space, SpaceId::Block(new_block));
    assert_eq!(
        member.geometry, insert.geometry,
        "the nested reference's own transform is untouched"
    );
    // The inner block is not duplicated or rewritten.
    assert_eq!(drawing(&app).block_name(BlockId(0)), Some("Inner"));
}

#[test]
fn pasteblock_names_are_incremented_and_addressable() {
    let (mut app, mut session) = two_line_fixture();
    copyclip(&mut app, &mut session, &[1]);

    paste_block(&mut app, &mut session, point(0.0, 0.0)).unwrap();
    paste_block(&mut app, &mut session, point(100.0, 0.0)).unwrap();

    assert_eq!(undo_depth(&app), 2, "each paste is one undo step");
    for name in ["PasteBlock1", "PasteBlock2"] {
        assert!(
            drawing(&app).block_by_name(name).is_some(),
            "{name} must resolve"
        );
    }
    assert!(drawing(&app).block_by_name("PasteBlock3").is_none());
    // The second paste's reference is selected and the clipboard is unchanged.
    assert_eq!(session.clipboard_len(), 1);
    assert_eq!(session.selection.refs().len(), 1);
}

#[test]
fn pasteblock_restores_member_display_and_reference_draw_order() {
    let attributes = non_default_attributes();
    let mut b = builder();
    b.insert_entity(line_entity(
        1,
        (0.0, 0.0),
        (10.0, 0.0),
        LayerId(0),
        SpaceId::Model,
        3,
    ))
    .unwrap();
    b.insert_entity(line_entity(
        2,
        (0.0, 1.0),
        (10.0, 1.0),
        LayerId(0),
        SpaceId::Model,
        7,
    ))
    .unwrap();
    b.set_entity_render_attributes(EntityId(1), attributes.clone())
        .unwrap();
    let (mut app, mut session) = finish(b, AppMode::Work);
    copyclip(&mut app, &mut session, &[1, 2]);

    paste_block(&mut app, &mut session, point(5.0, 5.0)).unwrap();

    let new_block = drawing(&app).block_by_name("PasteBlock1").unwrap().id;
    let member_ids = drawing(&app).block(new_block).unwrap().entities.clone();
    // The stored member order is ascending draw order.
    let orders: Vec<i64> = member_ids
        .iter()
        .map(|id| entity(&app, id.0).draw_order)
        .collect();
    assert_eq!(orders, vec![3, 7]);
    // The captured colour survives on the member.
    let restored = drawing(&app).entity_display_state(member_ids[0]).attributes;
    assert_eq!(
        restored.color, attributes.color,
        "member display attributes are restored after the commit"
    );
    // The reference draws with the highest member order.
    let model_space = drawing(&app).model_space();
    let reference = model_space
        .iter()
        .find(|candidate| {
            matches!(
                &candidate.geometry,
                SemanticGeometry::Insert { block, .. } if *block == new_block
            )
        })
        .expect("the reference is a model-space INSERT");
    assert_eq!(reference.draw_order, 7, "max member draw order");
}

#[test]
fn pasteblock_history_record_is_one_block_patch_plus_entity_patches() {
    let (mut app, mut session) = two_line_fixture();
    copyclip(&mut app, &mut session, &[1, 2]);
    paste_block(&mut app, &mut session, point(0.0, 0.0)).unwrap();
    let new_block = drawing(&app).block_by_name("PasteBlock1").unwrap().id;

    let record = app
        .history
        .get_mut(&DocumentId(1))
        .unwrap()
        .begin_drawing_undo()
        .unwrap();
    assert_eq!(record.label, "paste block");
    assert_eq!(record.block_patches.len(), 1);
    assert_eq!(record.block_patches[0].id, new_block);
    assert!(record.block_patches[0].before.is_none());
    assert!(record.block_patches[0].after.is_some());
    assert_eq!(
        record.patches.len(),
        3,
        "two member patches plus the reference patch"
    );
    assert!(record.merge_key.is_none());
    app.history
        .get_mut(&DocumentId(1))
        .unwrap()
        .cancel_pending_drawing();
    assert_eq!(undo_depth(&app), 1);
}
