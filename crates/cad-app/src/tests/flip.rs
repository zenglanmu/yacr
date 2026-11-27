//! `FLIP X|Y|Z`: the in-place SketchUp Flip Along.
//!
//! The reflection plane is through the selection bounds centre; the source is
//! erased and the mirrored copy inserted in one transaction/undo step, and the
//! selection follows the new row. No document format changes.

use super::*;

fn line(app: &mut Application, session: &mut SessionState) -> EntityId {
    app.execute(
        session,
        command(
            CommandId::CreateLine,
            CommandPayload::Points(vec![point(0.0, 0.0), point(2.0, 1.0)]),
        ),
    )
    .expect("the line");
    let id = app.workspace.documents[&DocumentId(1)]
        .drawing
        .model_space()
        .last()
        .expect("the line row")
        .id;
    app.execute(
        session,
        command(
            CommandId::Select,
            CommandPayload::Selection(vec![SelectionRef {
                document: DocumentId(1),
                entity: id,
                instance: InstancePath::default(),
                sub_element: None,
            }]),
        ),
    )
    .expect("select");
    id
}

fn flip(
    app: &mut Application,
    session: &mut SessionState,
    axis: FlipAxis,
) -> CadResult<CommandOutcome> {
    app.execute(
        session,
        command(CommandId::Flip, CommandPayload::Flip(axis)),
    )
}

fn line_geometry(app: &Application, id: EntityId) -> (Point3, Point3) {
    match &app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(id)
        .expect("the entity")
        .geometry
    {
        SemanticGeometry::Line { start, end } => (*start, *end),
        other => panic!("expected a line, got {other:?}"),
    }
}

#[test]
fn flip_x_reflects_the_selection_in_place_and_keeps_it_selected() {
    let (mut app, mut session) = application_with_document();
    let before_id = line(&mut app, &mut session);
    let before_revision = app.workspace.documents[&DocumentId(1)].drawing.revision();
    let outcome = flip(&mut app, &mut session, FlipAxis::Red).expect("flip");
    assert_eq!(
        app.workspace.documents[&DocumentId(1)].drawing.revision(),
        Revision(before_revision.0 + 1),
        "one transaction"
    );
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 2, "create + flip");
    assert!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity(before_id)
            .is_none(),
        "the source row is erased"
    );
    let selected = session.selection.refs();
    assert_eq!(selected.len(), 1, "the new row stays selected");
    let (start, end) = line_geometry(&app, selected[0].entity);
    assert_eq!(start, point(2.0, 0.0), "x reflected about x = 1");
    assert_eq!(end, point(0.0, 1.0));
    assert!(outcome
        .diagnostics
        .iter()
        .any(|row| row.code == "flip.applied"));

    app.execute(&mut session, command(CommandId::Undo, CommandPayload::None))
        .expect("undo");
    let (start, end) = line_geometry(&app, before_id);
    assert_eq!((start, end), (point(0.0, 0.0), point(2.0, 1.0)));
    assert_eq!(
        session.selection.refs().len(),
        0,
        "undo clears the stale selection"
    );
}

#[test]
fn flip_y_and_z_use_the_selection_centre_on_their_own_axis() {
    let (mut app, mut session) = application_with_document();
    line(&mut app, &mut session);
    flip(&mut app, &mut session, FlipAxis::Green).expect("flip Y");
    let (start, end) = line_geometry(&app, session.selection.refs()[0].entity);
    assert!((start.y - 1.0).abs() < 1e-9 && start.x == 0.0, "{start:?}");
    assert!((end.y).abs() < 1e-9 && end.x == 2.0, "{end:?}");
    assert_eq!(start.z, 0.0);
    flip(&mut app, &mut session, FlipAxis::Blue).expect("flip Z");
    // A planar line at z = 0 has its centre on z = 0, so a Z flip is an identity.
    let (start, end) = line_geometry(&app, session.selection.refs()[0].entity);
    assert!((start.y - 1.0).abs() < 1e-9 && start.x == 0.0, "{start:?}");
    assert!((end.y).abs() < 1e-9 && end.x == 2.0, "{end:?}");
}

#[test]
fn flip_refuses_an_empty_selection_and_viewer_mode() {
    let (mut app, mut session) = application_with_document();
    let error = flip(&mut app, &mut session, FlipAxis::Red).expect_err("empty");
    assert!(
        matches!(error, CadError::InvalidInput(ref text) if text.contains("flip.selection_required")),
        "{error:?}"
    );
    line(&mut app, &mut session);
    session.switch_mode(AppMode::Viewer).expect("Viewer");
    assert!(matches!(
        flip(&mut app, &mut session, FlipAxis::Red).unwrap_err(),
        CadError::PermissionDenied
    ));
}
