//! `REFEDIT` / `-REFEDIT` / `REFSET` / `REFCLOSE` contracts: the bounded 2D
//! in-place block-reference edit session.
//!
//! Every test asserts the **actual database effect** (extracted clones, hidden
//! source reference, rewritten block definition), exactly one transaction / one
//! undo step per write, and that every named refusal writes nothing. The block
//! table stays the single authority throughout.

use super::*;
use cad_db::{
    BlockDefinition, DbEntity, DbObject, DrawingDatabaseBuilder, EntityColor,
    EntityRenderAttributes, Layer,
};

fn p3(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

fn line(start: Point3, end: Point3) -> SemanticGeometry {
    SemanticGeometry::Line { start, end }
}

fn circle(center: Point3, radius: f64) -> SemanticGeometry {
    SemanticGeometry::Circle {
        center,
        normal: p3(0.0, 0.0, 1.0),
        radius,
    }
}

fn insert(block: u128, transform: Transform3) -> SemanticGeometry {
    SemanticGeometry::Insert {
        block: BlockId(block),
        transform,
    }
}

fn part(
    id: u128,
    geometry: SemanticGeometry,
    layer: LayerId,
    space: SpaceId,
    type_key: &str,
    order: i64,
) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: type_key.into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer,
        space,
        geometry,
        draw_order: order,
    }
}

fn build(entities: Vec<DbEntity>, blocks: Vec<BlockDefinition>) -> (Application, SessionState) {
    let (mut app, session) = application_with_document();
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    for (id, name) in [(LayerId(0), "0"), (LayerId(3), "WALLS")] {
        b.insert_layer(Layer {
            id,
            name: name.into(),
            visible: true,
        })
        .unwrap();
    }
    for block in blocks {
        b.insert_block(block).unwrap();
    }
    for entity in entities {
        b.insert_entity(entity).unwrap();
    }
    app.workspace
        .documents
        .get_mut(&DocumentId(1))
        .unwrap()
        .drawing = Arc::new(b.finish().unwrap());
    (app, session)
}

fn drawing_of(app: &Application) -> &DrawingDatabase {
    &app.workspace.documents[&DocumentId(1)].drawing
}

fn revision(app: &Application) -> Revision {
    drawing_of(app).revision()
}

fn undo_depth(app: &Application) -> usize {
    app.history
        .get(&DocumentId(1))
        .map(|history| history.undo_depth())
        .unwrap_or(0)
}

fn model_count(app: &Application) -> usize {
    drawing_of(app).model_space().len()
}

fn select(session: &mut SessionState, ids: &[u128]) {
    session.selection = SelectionSet::from_refs(ids.iter().map(|id| SelectionRef {
        document: DocumentId(1),
        entity: EntityId(*id),
        instance: InstancePath::default(),
        sub_element: None,
    }));
}

fn execute(
    app: &mut Application,
    session: &mut SessionState,
    id: CommandId,
    payload: CommandPayload,
) -> CadResult<CommandOutcome> {
    app.execute(session, command(id, payload))
}

/// The placement used by the two-member fixture: `T(5, 5) · S(2)`.
fn placement() -> Transform3 {
    Transform3::translation(p3(5.0, 5.0, 0.0)).matrix_mul(&Transform3::scale(2.0))
}

/// Block 1 has a LINE member 100 `(0,0)-(1,0)` (layer 0) and a CIRCLE member 101
/// at `(0,0)` radius 1 (layer WALLS); entity 1 is an INSERT of it in model space.
fn two_member_app() -> (Application, SessionState) {
    build(
        vec![
            part(
                100,
                line(p3(0.0, 0.0, 0.0), p3(1.0, 0.0, 0.0)),
                LayerId(0),
                SpaceId::Block(BlockId(1)),
                "AcDbLine",
                0,
            ),
            part(
                101,
                circle(p3(0.0, 0.0, 0.0), 1.0),
                LayerId(3),
                SpaceId::Block(BlockId(1)),
                "AcDbCircle",
                1,
            ),
            part(
                1,
                insert(1, placement()),
                LayerId(0),
                SpaceId::Model,
                "AcDbBlockReference",
                0,
            ),
        ],
        vec![BlockDefinition {
            id: BlockId(1),
            entities: vec![EntityId(100), EntityId(101)],
            dynamic_visibility: None,
        }],
    )
}

/// The one-member fixture (member 100 only), plus a plain host LINE 2.
fn one_member_app_with_host_line() -> (Application, SessionState) {
    build(
        vec![
            part(
                100,
                line(p3(0.0, 0.0, 0.0), p3(1.0, 0.0, 0.0)),
                LayerId(0),
                SpaceId::Block(BlockId(1)),
                "AcDbLine",
                0,
            ),
            part(
                1,
                insert(1, placement()),
                LayerId(0),
                SpaceId::Model,
                "AcDbBlockReference",
                0,
            ),
            part(
                2,
                line(p3(10.0, 10.0, 0.0), p3(11.0, 10.0, 0.0)),
                LayerId(0),
                SpaceId::Model,
                "AcDbLine",
                1,
            ),
        ],
        vec![BlockDefinition {
            id: BlockId(1),
            entities: vec![EntityId(100)],
            dynamic_visibility: None,
        }],
    )
}

fn begin(app: &mut Application, session: &mut SessionState) -> Vec<u128> {
    select(session, &[1]);
    let outcome = execute(app, session, CommandId::RefEdit, CommandPayload::None)
        .expect("REFEDIT extracts the members");
    outcome.objects.iter().map(|object| object.0).collect()
}

#[test]
fn refedit_extracts_members_hides_the_insert_and_selects_the_clones() {
    let (mut app, mut session) = two_member_app();
    let before = revision(&app);
    let clones = begin(&mut app, &mut session);

    assert_eq!(clones.len(), 2, "one clone per visible member");
    assert_eq!(revision(&app), Revision(before.0 + 1), "one revision");
    assert_eq!(undo_depth(&app), 1, "one undo step");
    assert_eq!(model_count(&app), 3, "the INSERT plus two clones");

    // The source INSERT is hidden by the existing session hidden set.
    assert!(session.hidden_entities.contains(&SelectionRef {
        document: DocumentId(1),
        entity: EntityId(1),
        instance: InstancePath::default(),
        sub_element: None,
    }));
    // ...and the selection is exactly the clones.
    let selected: Vec<u128> = session
        .selection
        .refs()
        .iter()
        .map(|reference| reference.entity.0)
        .collect();
    assert_eq!(selected, clones);

    // The clones carry the placed world geometry and the member's layer.
    let first = drawing_of(&app).entity(EntityId(clones[0])).unwrap();
    assert_eq!(first.space, SpaceId::Model);
    assert_eq!(first.layer, LayerId(0));
    assert_eq!(first.geometry, line(p3(5.0, 5.0, 0.0), p3(7.0, 5.0, 0.0)));
    let second = drawing_of(&app).entity(EntityId(clones[1])).unwrap();
    assert_eq!(second.layer, LayerId(3));
    assert_eq!(second.geometry, circle(p3(5.0, 5.0, 0.0), 2.0));

    // The block definition is untouched by the extraction.
    assert_eq!(
        drawing_of(&app).block(BlockId(1)).unwrap().entities,
        vec![EntityId(100), EntityId(101)]
    );
    assert_eq!(
        drawing_of(&app).entity(EntityId(100)).unwrap().geometry,
        line(p3(0.0, 0.0, 0.0), p3(1.0, 0.0, 0.0))
    );
    assert!(session.refedit.is_active());
}

#[test]
fn refedit_preserves_member_display_attributes() {
    let (mut app, mut session) = application_with_document();
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    for (id, name) in [(LayerId(0), "0"), (LayerId(3), "WALLS")] {
        b.insert_layer(Layer {
            id,
            name: name.into(),
            visible: true,
        })
        .unwrap();
    }
    let attributes = EntityRenderAttributes {
        color: EntityColor::Explicit([12, 34, 56]),
        ..Default::default()
    };
    b.insert_entity(part(
        100,
        line(p3(0.0, 0.0, 0.0), p3(1.0, 0.0, 0.0)),
        LayerId(0),
        SpaceId::Block(BlockId(1)),
        "AcDbLine",
        0,
    ))
    .unwrap();
    b.set_entity_render_attributes(EntityId(100), attributes.clone())
        .unwrap();
    b.insert_entity(part(
        1,
        insert(1, placement()),
        LayerId(0),
        SpaceId::Model,
        "AcDbBlockReference",
        0,
    ))
    .unwrap();
    b.insert_block(BlockDefinition {
        id: BlockId(1),
        entities: vec![EntityId(100)],
        dynamic_visibility: None,
    })
    .unwrap();
    app.workspace
        .documents
        .get_mut(&DocumentId(1))
        .unwrap()
        .drawing = Arc::new(b.finish().unwrap());

    let clones = begin(&mut app, &mut session);
    assert_eq!(
        drawing_of(&app).entity_render_attributes(EntityId(clones[0])),
        attributes,
        "the clone keeps the member's render attributes"
    );
    assert_eq!(
        drawing_of(&app).entity_render_attributes(EntityId(100)),
        attributes,
        "the block member is untouched by the extraction"
    );
}

#[test]
fn undo_of_refedit_drops_the_stale_session_and_unhides_the_source() {
    let (mut app, mut session) = two_member_app();
    let _clones = begin(&mut app, &mut session);
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .expect("undo the extraction");
    assert_eq!(model_count(&app), 1, "the clones are gone again");
    assert!(
        !session.refedit.is_active(),
        "a session whose clones vanished must not stay active"
    );
    assert!(
        session.hidden_entities.is_empty(),
        "the source reference is visible again"
    );
}

#[test]
fn refclose_save_rewrites_the_block_once_and_undo_restores_it() {
    let (mut app, mut session) = two_member_app();
    let before = revision(&app);
    let clones = begin(&mut app, &mut session);

    // Host edits on the clones: move both by (1, 1, 0) in model space.
    let refs = clones
        .iter()
        .map(|id| SelectionRef {
            document: DocumentId(1),
            entity: EntityId(*id),
            instance: InstancePath::default(),
            sub_element: None,
        })
        .collect();
    execute(
        &mut app,
        &mut session,
        CommandId::MoveEntities,
        CommandPayload::Move {
            refs,
            delta: p3(1.0, 1.0, 0.0),
        },
    )
    .expect("MOVE edits the clones");
    assert_eq!(undo_depth(&app), 2);

    let revision_before_save = revision(&app);
    execute(
        &mut app,
        &mut session,
        CommandId::RefClose,
        CommandPayload::RefClose(RefCloseOption::Save),
    )
    .expect("REFCLOSE SAVE writes the block");

    assert_eq!(
        revision(&app),
        Revision(revision_before_save.0 + 1),
        "SAVE is one revision"
    );
    assert_eq!(undo_depth(&app), 3, "SAVE is one undo step");
    assert_eq!(model_count(&app), 1, "the working clones are gone");

    // The block members keep their ids and are inverse-transformed exactly.
    let drawing = drawing_of(&app);
    assert_eq!(
        drawing.block(BlockId(1)).unwrap().entities,
        vec![EntityId(100), EntityId(101)]
    );
    assert_eq!(
        drawing.entity(EntityId(100)).unwrap().geometry,
        line(p3(0.5, 0.5, 0.0), p3(1.5, 0.5, 0.0))
    );
    assert_eq!(
        drawing.entity(EntityId(101)).unwrap().geometry,
        circle(p3(0.5, 0.5, 0.0), 1.0)
    );

    // The source reference is visible again and the session is closed.
    assert!(session.hidden_entities.is_empty());
    assert!(!session.refedit.is_active());
    assert!(session.selection.is_empty());
    assert_eq!(revision(&app), Revision(before.0 + 3), "edit + move + save");

    // Undo restores the pre-save block definition and the clones.
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .expect("undo the save");
    assert_eq!(
        drawing_of(&app).entity(EntityId(100)).unwrap().geometry,
        line(p3(0.0, 0.0, 0.0), p3(1.0, 0.0, 0.0))
    );
    assert_eq!(model_count(&app), 3, "the two clones come back");
}

#[test]
fn refclose_discard_deletes_the_clones_and_keeps_the_block() {
    let (mut app, mut session) = two_member_app();
    let before = revision(&app);
    let clones = begin(&mut app, &mut session);

    execute(
        &mut app,
        &mut session,
        CommandId::RefClose,
        CommandPayload::RefClose(RefCloseOption::Discard),
    )
    .expect("REFCLOSE DISCARD deletes the clones");

    assert_eq!(revision(&app), Revision(before.0 + 2), "extract + discard");
    assert_eq!(undo_depth(&app), 2);
    assert_eq!(model_count(&app), 1, "only the source INSERT remains");
    for clone in clones {
        assert!(
            drawing_of(&app).entity(EntityId(clone)).is_none(),
            "clone {clone} must be deleted"
        );
    }
    assert_eq!(
        drawing_of(&app).block(BlockId(1)).unwrap().entities,
        vec![EntityId(100), EntityId(101)],
        "DISCARD leaves the block definition unchanged"
    );
    assert_eq!(
        drawing_of(&app).entity(EntityId(100)).unwrap().geometry,
        line(p3(0.0, 0.0, 0.0), p3(1.0, 0.0, 0.0))
    );
    assert!(session.hidden_entities.is_empty());
    assert!(!session.refedit.is_active());
}

#[test]
fn refset_add_moves_a_host_object_into_the_block_on_save() {
    let (mut app, mut session) = one_member_app_with_host_line();
    let clones = begin(&mut app, &mut session);
    let revision_after_begin = revision(&app);

    // Bare REFSET is the ADD default; the current selection is the host LINE.
    select(&mut session, &[2]);
    execute(
        &mut app,
        &mut session,
        CommandId::RefSet,
        CommandPayload::None,
    )
    .expect("REFSET adds the host line");
    assert_eq!(
        revision(&app),
        revision_after_begin,
        "REFSET is session state only"
    );
    assert_eq!(undo_depth(&app), 1);
    assert!(session.refedit.working().unwrap().contains(&EntityId(2)));

    execute(
        &mut app,
        &mut session,
        CommandId::RefClose,
        CommandPayload::RefClose(RefCloseOption::Save),
    )
    .expect("REFCLOSE SAVE moves the host line into the block");

    let drawing = drawing_of(&app);
    assert_eq!(
        drawing.block(BlockId(1)).unwrap().entities,
        vec![EntityId(100), EntityId(2)],
        "the host line is a block member now"
    );
    let moved = drawing.entity(EntityId(2)).unwrap();
    assert_eq!(moved.space, SpaceId::Block(BlockId(1)));
    assert_eq!(
        moved.geometry,
        line(p3(2.5, 2.5, 0.0), p3(3.0, 2.5, 0.0)),
        "the host geometry is inverse-transformed into block coordinates"
    );
    assert!(drawing.entity(EntityId(clones[0])).is_none());
    assert!(session.hidden_entities.is_empty());
}

#[test]
fn refset_remove_deletes_the_original_and_keeps_the_host_copy() {
    let (mut app, mut session) = two_member_app();
    let clones = begin(&mut app, &mut session);
    // The clone order follows the stored member order (100, then 101).
    let removed_clone = clones[0];

    select(&mut session, &[removed_clone]);
    execute(
        &mut app,
        &mut session,
        CommandId::RefSet,
        CommandPayload::RefSet(RefSetOption::Remove),
    )
    .expect("REFSET removes the clone from the working set");
    assert!(!session
        .refedit
        .working()
        .unwrap()
        .contains(&EntityId(removed_clone)));

    execute(
        &mut app,
        &mut session,
        CommandId::RefClose,
        CommandPayload::RefClose(RefCloseOption::Save),
    )
    .expect("REFCLOSE SAVE deletes the removed member from the block");

    let drawing = drawing_of(&app);
    assert_eq!(
        drawing.block(BlockId(1)).unwrap().entities,
        vec![EntityId(101)],
        "the removed member is deleted from the block"
    );
    assert!(drawing.entity(EntityId(100)).is_none());
    // The removed clone stays in model space as an ordinary host copy.
    let host_copy = drawing.entity(EntityId(removed_clone)).unwrap();
    assert_eq!(host_copy.space, SpaceId::Model);
    // The remaining clone is gone (it was still in the working set).
    assert!(drawing.entity(EntityId(clones[1])).is_none());
}

#[test]
fn refedit_refuses_a_bad_selection_and_a_non_insert_without_writing() {
    // No selection at all.
    let (mut app, mut session) = two_member_app();
    let before = revision(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::RefEdit,
        CommandPayload::None,
    )
    .unwrap_err();
    assert!(error.to_string().contains("refedit.selection"), "{error}");
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 0);
    assert!(!session.refedit.is_active());

    // A directly addressable LINE is not an INSERT.
    let (mut app, mut session) = build(
        vec![part(
            1,
            line(p3(0.0, 0.0, 0.0), p3(1.0, 0.0, 0.0)),
            LayerId(0),
            SpaceId::Model,
            "AcDbLine",
            0,
        )],
        Vec::new(),
    );
    select(&mut session, &[1]);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::RefEdit,
        CommandPayload::None,
    )
    .unwrap_err();
    assert!(error.to_string().contains("refedit.not_insert"), "{error}");
    assert_eq!(revision(&app), Revision(0));
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn refedit_refuses_a_non_uniform_placement_without_writing() {
    let mut non_uniform = Transform3::identity();
    non_uniform.matrix[0][0] = 2.0;
    non_uniform.matrix[1][1] = 3.0;
    let (mut app, mut session) = build(
        vec![
            part(
                100,
                line(p3(0.0, 0.0, 0.0), p3(1.0, 0.0, 0.0)),
                LayerId(0),
                SpaceId::Block(BlockId(1)),
                "AcDbLine",
                0,
            ),
            part(
                1,
                insert(1, non_uniform),
                LayerId(0),
                SpaceId::Model,
                "AcDbBlockReference",
                0,
            ),
        ],
        vec![BlockDefinition {
            id: BlockId(1),
            entities: vec![EntityId(100)],
            dynamic_visibility: None,
        }],
    );
    select(&mut session, &[1]);
    let before = revision(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::RefEdit,
        CommandPayload::None,
    )
    .unwrap_err();
    assert!(error.to_string().contains("refedit.non_uniform"), "{error}");
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 0);
    assert!(session.hidden_entities.is_empty());
    assert!(!session.refedit.is_active());
}

#[test]
fn refset_and_refclose_refuse_without_a_session_and_write_nothing() {
    let (mut app, mut session) = two_member_app();
    select(&mut session, &[1]);
    let before = revision(&app);

    let error = execute(
        &mut app,
        &mut session,
        CommandId::RefSet,
        CommandPayload::None,
    )
    .unwrap_err();
    assert!(error.to_string().contains("refedit.no_session"), "{error}");

    let error = execute(
        &mut app,
        &mut session,
        CommandId::RefClose,
        CommandPayload::RefClose(RefCloseOption::Save),
    )
    .unwrap_err();
    assert!(error.to_string().contains("refedit.no_session"), "{error}");

    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn refset_refuses_a_reference_to_the_edited_block_itself() {
    let (mut app, mut session) = two_member_app();
    let clones = begin(&mut app, &mut session);
    let revision_after_begin = revision(&app);

    // The source INSERT is an instance of block 1; a save would make block 1
    // contain an instance of itself.
    select(&mut session, &[1]);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::RefSet,
        CommandPayload::None,
    )
    .unwrap_err();
    assert!(error.to_string().contains("refedit.selection"), "{error}");
    assert_eq!(revision(&app), revision_after_begin);
    assert_eq!(undo_depth(&app), 1);
    assert_eq!(session.refedit.working().unwrap().len(), clones.len());
}

#[test]
fn refset_needs_a_non_empty_selection() {
    let (mut app, mut session) = two_member_app();
    let clones = begin(&mut app, &mut session);
    let revision_after_begin = revision(&app);
    session.selection = SelectionSet::new();

    let error = execute(
        &mut app,
        &mut session,
        CommandId::RefSet,
        CommandPayload::None,
    )
    .unwrap_err();
    assert!(
        error.to_string().contains("refedit.selection_empty"),
        "{error}"
    );
    assert_eq!(revision(&app), revision_after_begin);
    assert_eq!(undo_depth(&app), 1);
    assert!(session
        .refedit
        .working()
        .unwrap()
        .contains(&EntityId(clones[0])));
}

#[test]
fn a_second_refedit_is_refused_while_a_session_is_active() {
    let (mut app, mut session) = two_member_app();
    let _clones = begin(&mut app, &mut session);
    let revision_after_begin = revision(&app);

    // Select the source INSERT again (it is hidden but the request is explicit).
    select(&mut session, &[1]);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::RefEdit,
        CommandPayload::None,
    )
    .unwrap_err();
    assert!(error.to_string().contains("refedit.active"), "{error}");
    assert_eq!(revision(&app), revision_after_begin);
    assert_eq!(undo_depth(&app), 1);
    assert!(session.refedit.is_active());
}

#[test]
fn bare_refclose_needs_an_option_and_keeps_the_session_open() {
    let (mut app, mut session) = two_member_app();
    let _clones = begin(&mut app, &mut session);
    let revision_after_begin = revision(&app);

    let error = execute(
        &mut app,
        &mut session,
        CommandId::RefClose,
        CommandPayload::None,
    )
    .unwrap_err();
    assert!(
        error.to_string().contains("refclose.needs_option"),
        "{error}"
    );
    assert_eq!(revision(&app), revision_after_begin);
    assert_eq!(undo_depth(&app), 1);
    assert!(session.refedit.is_active());
    assert!(!session.hidden_entities.is_empty());
}

#[test]
fn viewer_mode_refuses_all_four_commands() {
    for (id, payload) in [
        (CommandId::RefEdit, CommandPayload::None),
        (CommandId::DashRefEdit, CommandPayload::None),
        (CommandId::RefSet, CommandPayload::RefSet(RefSetOption::Add)),
        (
            CommandId::RefClose,
            CommandPayload::RefClose(RefCloseOption::Save),
        ),
    ] {
        let (mut app, mut session) = two_member_app();
        session
            .switch_mode(AppMode::Viewer)
            .expect("Viewer is legal");
        select(&mut session, &[1]);
        let before = revision(&app);
        let error = execute(&mut app, &mut session, id, payload).unwrap_err();
        assert!(
            matches!(error, CadError::PermissionDenied),
            "{id:?} must be PermissionDenied, got {error:?}"
        );
        assert_eq!(revision(&app), before, "{id:?} must not write");
        assert_eq!(undo_depth(&app), 0, "{id:?} must not record history");
        assert!(session.hidden_entities.is_empty());
        assert!(!session.refedit.is_active());
    }
}
