//! `SELECTSIMILAR` contract tests (type-based v1).
//!
//! Pinned here: the union with the existing selection, the geometry-kind match
//! (a selected LINE adds the other LINE, never a CIRCLE), the payload-free
//! contract, the non-empty-selection requirement, the model-space-only refusal,
//! and the fact that the command writes nothing (no revision bump, no undo
//! entry). Attribute similarity (layer/colour/linetype/lineweight/transparency)
//! is explicitly **NOT IMPLEMENTED** in v1 and is not asserted here.

use super::*;

fn execute(
    app: &mut Application,
    session: &mut SessionState,
    id: CommandId,
    payload: CommandPayload,
) -> CadResult<CommandOutcome> {
    app.execute(session, command(id, payload))
}

fn model_ref(entity: EntityId) -> SelectionRef {
    SelectionRef {
        document: DocumentId(1),
        entity,
        instance: InstancePath::default(),
        sub_element: None,
    }
}

fn revision(app: &Application) -> Revision {
    app.workspace.documents[&DocumentId(1)].drawing.revision()
}

fn undo_depth(app: &Application) -> usize {
    app.history
        .get(&DocumentId(1))
        .map(|history| history.undo_depth())
        .unwrap_or(0)
}

fn model_ids(app: &Application) -> Vec<(EntityId, &'static str)> {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .model_space()
        .iter()
        .map(|entity| {
            let kind = match &entity.geometry {
                SemanticGeometry::Line { .. } => "line",
                SemanticGeometry::Circle { .. } => "circle",
                _ => "other",
            };
            (entity.id, kind)
        })
        .collect()
}

fn two_lines_and_a_circle() -> (Application, SessionState, Vec<EntityId>, EntityId) {
    let (mut app, mut session) = application_with_document();
    for (start, end) in [((0.0, 0.0), (1.0, 0.0)), ((0.0, 1.0), (1.0, 1.0))] {
        execute(
            &mut app,
            &mut session,
            CommandId::CreateLine,
            CommandPayload::Points(vec![point(start.0, start.1), point(end.0, end.1)]),
        )
        .expect("LINE is created");
    }
    execute(
        &mut app,
        &mut session,
        CommandId::CreateCircle,
        CommandPayload::Points(vec![point(5.0, 5.0), point(7.0, 5.0)]),
    )
    .expect("CIRCLE is created");

    let ids = model_ids(&app);
    let lines: Vec<EntityId> = ids
        .iter()
        .filter(|(_, kind)| *kind == "line")
        .map(|(id, _)| *id)
        .collect();
    let circle = ids
        .iter()
        .find(|(_, kind)| *kind == "circle")
        .map(|(id, _)| *id)
        .expect("one circle");
    assert_eq!(lines.len(), 2, "two model-space LINE entities");
    (app, session, lines, circle)
}

#[test]
fn adding_similar_objects_keeps_the_selection_and_is_read_only() {
    let (mut app, mut session, lines, circle) = two_lines_and_a_circle();
    execute(
        &mut app,
        &mut session,
        CommandId::Select,
        CommandPayload::Selection(vec![model_ref(lines[0])]),
    )
    .expect("SELECT accepts one line");

    let before_revision = revision(&app);
    let before_undo = undo_depth(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::SelectSimilar,
        CommandPayload::None,
    )
    .expect("SELECTSIMILAR runs");

    assert_eq!(session.selection.len(), 2, "the other LINE is added");
    assert!(session.selection.contains(&model_ref(lines[0])));
    assert!(session.selection.contains(&model_ref(lines[1])));
    assert!(
        !session.selection.contains(&model_ref(circle)),
        "a CIRCLE is not similar to a LINE"
    );
    assert!(matches!(session.tool, ToolState::Selecting));
    assert!(outcome.objects.is_empty());
    assert!(outcome.changes.is_none());
    assert_eq!(outcome.diagnostics.len(), 1);
    let diagnostic = &outcome.diagnostics[0];
    assert_eq!(diagnostic.code, "selectsimilar.result");
    assert!(
        diagnostic.message.is_ascii(),
        "the diagnostic must be ASCII: {}",
        diagnostic.message
    );
    assert!(
        diagnostic.message.contains('1'),
        "the diagnostic names how many objects were added: {}",
        diagnostic.message
    );
    assert_eq!(undo_depth(&app), before_undo, "no undo entry");
    assert_eq!(revision(&app), before_revision, "no database revision bump");
}

#[test]
fn empty_selection_is_refused_by_name() {
    let (mut app, mut session, _, _) = two_lines_and_a_circle();
    let before = session.selection.clone();
    let error = execute(
        &mut app,
        &mut session,
        CommandId::SelectSimilar,
        CommandPayload::None,
    )
    .expect_err("an empty selection cannot produce a similarity match");
    match error {
        CadError::InvalidInput(message) => assert!(
            message.starts_with("selectsimilar.selection_required"),
            "stable refusal prefix, got {message:?}"
        ),
        other => panic!("expected InvalidInput, got {other:?}"),
    }
    assert_eq!(session.selection, before);
    assert!(matches!(session.tool, ToolState::Idle));
}

#[test]
fn a_non_none_payload_is_refused() {
    let (mut app, mut session, lines, _) = two_lines_and_a_circle();
    execute(
        &mut app,
        &mut session,
        CommandId::Select,
        CommandPayload::Selection(vec![model_ref(lines[0])]),
    )
    .expect("SELECT accepts one line");
    let before = session.selection.clone();
    let error = execute(
        &mut app,
        &mut session,
        CommandId::SelectSimilar,
        CommandPayload::Selection(Vec::new()),
    )
    .expect_err("SELECTSIMILAR takes no payload");
    assert!(matches!(error, CadError::InvalidInput(_)));
    assert_eq!(session.selection, before);
}

#[test]
fn paper_space_active_space_is_unsupported() {
    let (mut app, mut session, lines, _) = two_lines_and_a_circle();
    execute(
        &mut app,
        &mut session,
        CommandId::Select,
        CommandPayload::Selection(vec![model_ref(lines[0])]),
    )
    .expect("SELECT accepts one line");
    session.active_space = SpaceId::Paper(LayoutId(1));
    let before = session.selection.clone();
    let error = execute(
        &mut app,
        &mut session,
        CommandId::SelectSimilar,
        CommandPayload::None,
    )
    .expect_err("model space is the only supported space");
    assert!(matches!(error, CadError::Unsupported(_)));
    assert_eq!(session.selection, before);
}

#[test]
fn a_selected_ref_outside_the_catalogue_is_stale() {
    let (mut app, mut session, _, _) = two_lines_and_a_circle();
    let stale = EntityId(9_999);
    execute(
        &mut app,
        &mut session,
        CommandId::Select,
        CommandPayload::Selection(vec![model_ref(stale)]),
    )
    .expect("SELECT stores the ref");
    let error = execute(
        &mut app,
        &mut session,
        CommandId::SelectSimilar,
        CommandPayload::None,
    )
    .expect_err("a ref with no pick item cannot be matched");
    assert!(matches!(error, CadError::StaleResult));
}
