//! `XEDGES` command contracts (strict version).
//!
//! The command is a **pure extraction** of the already-stored
//! [`BodyDisplay::edges`](cad_domain::BodyDisplay::edges): no modeler and no
//! geometry kernel are involved. These tests pin the one-transaction/one-undo
//! write, the source body surviving, the per-segment LINE mapping, and every
//! refusal (`selection_empty`, `not_a_body`, `sub_element_not_supported`,
//! `no_display_edges`) with **zero** database/revision/undo change.

use super::*;
use crate::tests::inquiry::app_from_builder;
use cad_db::{DrawingDatabaseBuilder, Layer};

const BODY: u128 = 700;
const OTHER_BODY: u128 = 701;
const LINE: u128 = 702;

/// A sheet body with a stored display-edge set and no exact geometry: the
/// smallest row that carries real edges (the command never needs a solid).
fn body_entity(id: u128, edges: Vec<Vec<Point3>>) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbBody".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Body3d(Box::new(Body3d {
            kind: BodyKind::Sheet,
            exact: BodyExact::Unavailable {
                reason: "display-only test body".into(),
            },
            display: BodyDisplay {
                mesh: Mesh::default(),
                edges,
                tolerance: None,
                precision: Precision::Approximate { error_bound: None },
                completeness: Completeness::Partial(vec!["display-only test body".into()]),
            },
        })),
        draw_order: id as i64,
    }
}

fn line_entity(id: u128) -> DbEntity {
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
        geometry: SemanticGeometry::Line {
            start: p3(0.0, 0.0, 0.0),
            end: p3(1.0, 0.0, 0.0),
        },
        draw_order: id as i64,
    }
}

fn app_with(entities: Vec<DbEntity>) -> (Application, SessionState) {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    for entity in entities {
        builder.insert_entity(entity).unwrap();
    }
    let (app, session, _) = app_from_builder(builder);
    (app, session)
}

fn select(session: &mut SessionState, ids: &[EntityId]) {
    session.selection = SelectionSet::from_refs(ids.iter().map(|id| SelectionRef {
        document: DocumentId(1),
        entity: *id,
        instance: InstancePath::default(),
        sub_element: None,
    }));
}

fn xedges() -> Command {
    command(
        CommandId::Extended(ExtendedCommandId::Xedges),
        CommandPayload::None,
    )
}

fn revision(app: &Application) -> Revision {
    app.workspace.documents[&DocumentId(1)].drawing.revision()
}

fn undo_depth(app: &Application) -> usize {
    app.history
        .get(&DocumentId(1))
        .map_or(0, |history| history.undo_depth())
}

fn invalid_text(error: CadError) -> String {
    match error {
        CadError::InvalidInput(text) => text,
        other => panic!("expected InvalidInput, got {other:?}"),
    }
}

fn lines(app: &Application) -> Vec<DbEntity> {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .entities()
        .filter(|entity| entity.object.type_key == "AcDbLine")
        .cloned()
        .collect()
}

/// Two stored polylines: one with two real segments, one whose only segment is
/// zero length (the seam case every tessellated body can contain).
fn stored_edges() -> Vec<Vec<Point3>> {
    vec![
        vec![p3(0.0, 0.0, 0.0), p3(1.0, 0.0, 0.0), p3(1.0, 1.0, 0.5)],
        vec![p3(2.0, 2.0, 2.0), p3(2.0, 2.0, 2.0)],
    ]
}

#[test]
fn xedges_extracts_every_stored_segment_in_one_transaction_and_one_undo_step() {
    let (mut app, mut session) = app_with(vec![body_entity(BODY, stored_edges())]);
    select(&mut session, &[EntityId(BODY)]);
    let before_revision = revision(&app);
    let before_undo = undo_depth(&app);

    let outcome = app
        .execute(&mut session, xedges())
        .expect("XEDGES extracts");
    assert_eq!(
        outcome.objects.len(),
        2,
        "two real segments, the seam skipped"
    );
    assert_eq!(
        revision(&app),
        Revision(before_revision.0 + 1),
        "exactly one revision bump"
    );
    assert_eq!(undo_depth(&app), before_undo + 1, "exactly one undo step");
    assert!(
        outcome
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "drawing.xedges"),
        "{:?}",
        outcome.diagnostics
    );

    // The body stays; the two new rows are real LINEs inheriting its layer/space
    // and carry the stored endpoints.
    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    assert!(
        drawing.entity(EntityId(BODY)).is_some(),
        "the body survives"
    );
    let extracted = lines(&app);
    assert_eq!(extracted.len(), 2);
    let mut endpoints: Vec<(Point3, Point3)> = extracted
        .iter()
        .map(|entity| match entity.geometry {
            SemanticGeometry::Line { start, end } => (start, end),
            ref other => panic!("not a LINE: {other:?}"),
        })
        .collect();
    endpoints.sort_by(|a, b| a.0.x.partial_cmp(&b.0.x).unwrap());
    assert_eq!(endpoints[0], (p3(0.0, 0.0, 0.0), p3(1.0, 0.0, 0.0)));
    assert_eq!(endpoints[1], (p3(1.0, 0.0, 0.0), p3(1.0, 1.0, 0.5)));
    for entity in &extracted {
        assert_eq!(entity.layer, LayerId(0));
        assert_eq!(entity.space, SpaceId::Model);
    }

    // One undo removes the whole wireframe and keeps the source body.
    app.execute(&mut session, command(CommandId::Undo, CommandPayload::None))
        .expect("undo");
    assert!(
        lines(&app).is_empty(),
        "one undo removes every extracted LINE"
    );
    assert!(app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(BODY))
        .is_some());
}

#[test]
fn a_body_with_no_stored_edges_is_refused_by_name_without_writing() {
    let (mut app, mut session) = app_with(vec![body_entity(BODY, Vec::new())]);
    select(&mut session, &[EntityId(BODY)]);
    let before_revision = revision(&app);

    let text = invalid_text(app.execute(&mut session, xedges()).expect_err("no edges"));
    assert!(text.starts_with("xedges.no_display_edges:"), "{text}");
    assert_eq!(revision(&app), before_revision);
    assert_eq!(undo_depth(&app), 0, "a refusal records no undo step");
    assert!(lines(&app).is_empty());
}

#[test]
fn a_selection_of_only_zero_length_segments_is_the_same_no_edges_refusal() {
    let (mut app, mut session) = app_with(vec![body_entity(
        BODY,
        vec![vec![p3(1.0, 1.0, 1.0), p3(1.0, 1.0, 1.0)]],
    )]);
    select(&mut session, &[EntityId(BODY)]);
    let text = invalid_text(
        app.execute(&mut session, xedges())
            .expect_err("a zero-length seam is not extractable"),
    );
    assert!(text.starts_with("xedges.no_display_edges:"), "{text}");
    assert!(lines(&app).is_empty());
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn an_empty_selection_or_a_non_body_row_is_refused_before_any_write() {
    let (mut app, mut session) =
        app_with(vec![body_entity(BODY, stored_edges()), line_entity(LINE)]);
    let before = revision(&app);

    let text = invalid_text(
        app.execute(&mut session, xedges())
            .expect_err("empty selection"),
    );
    assert!(text.starts_with("xedges.selection_empty:"), "{text}");

    select(&mut session, &[EntityId(LINE)]);
    let text = invalid_text(
        app.execute(&mut session, xedges())
            .expect_err("a LINE is not a body"),
    );
    assert!(text.starts_with("xedges.not_a_body:"), "{text}");
    assert!(text.contains("AcDbLine"), "{text}");

    // A mixed selection is all-or-nothing: the body's edges are not extracted.
    select(&mut session, &[EntityId(BODY), EntityId(LINE)]);
    let text = invalid_text(
        app.execute(&mut session, xedges())
            .expect_err("one non-body refuses the batch"),
    );
    assert!(text.starts_with("xedges.not_a_body:"), "{text}");

    assert_eq!(revision(&app), before);
    assert!(!app.can_undo(&DocumentId(1)));
    assert!(lines(&app).len() == 1, "only the fixture LINE exists");
}

#[test]
fn a_sub_element_reference_is_refused_by_name() {
    let (mut app, mut session) = app_with(vec![body_entity(BODY, stored_edges())]);
    session.selection = SelectionSet::from_refs([SelectionRef {
        document: DocumentId(1),
        entity: EntityId(BODY),
        instance: InstancePath::default(),
        sub_element: Some(SubElementId {
            source_key: "brep:face:0/0".into(),
            topology_revision: Revision(0),
        }),
    }]);
    let before = revision(&app);
    let text = invalid_text(
        app.execute(&mut session, xedges())
            .expect_err("a face is not a whole body"),
    );
    assert!(
        text.starts_with("xedges.sub_element_not_supported:"),
        "{text}"
    );
    assert_eq!(revision(&app), before);
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn xedges_takes_no_payload_and_refuses_one_before_touching_the_drawing() {
    let (mut app, mut session) = app_with(vec![body_entity(BODY, stored_edges())]);
    select(&mut session, &[EntityId(BODY)]);
    let before = revision(&app);
    let error = app
        .execute(
            &mut session,
            command(
                CommandId::Extended(ExtendedCommandId::Xedges),
                CommandPayload::Points(vec![p3(0.0, 0.0, 0.0)]),
            ),
        )
        .expect_err("XEDGES has no payload");
    assert!(matches!(error, CadError::InvalidInput(_)), "{error:?}");
    assert_eq!(revision(&app), before);
    assert!(lines(&app).is_empty());
}

#[test]
fn several_selected_bodies_contribute_their_edges_in_one_batch() {
    let (mut app, mut session) = app_with(vec![
        body_entity(BODY, stored_edges()),
        body_entity(OTHER_BODY, vec![vec![p3(5.0, 0.0, 0.0), p3(6.0, 0.0, 0.0)]]),
    ]);
    select(&mut session, &[EntityId(BODY), EntityId(OTHER_BODY)]);
    let outcome = app.execute(&mut session, xedges()).expect("both bodies");
    assert_eq!(outcome.objects.len(), 3, "two + one segments");
    assert_eq!(undo_depth(&app), 1, "still one undo step");
    assert_eq!(lines(&app).len(), 3);
}
