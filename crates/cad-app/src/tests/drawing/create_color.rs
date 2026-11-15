//! `COLOR` is a **creation** input: the shared create funnel
//! (`app_drawing::trim_extend`) stamps the session current-colour ACI on every
//! entity it inserts, and `ByLayer` (the default token) writes no per-entity
//! override at all.
//!
//! The assertions read the **stored** row (`entity_render_attributes`), never the
//! command ack, because that is what the report, the renderer and the DXF/DWG
//! exporter consume.

use super::*;

/// `COLOR 1` then LINE / POLYLINE / CIRCLE / RECTANG / POINT: every stored row
/// really carries ACI 1. POINT is the batch path (`commit_new_entities`), so one
/// token covers every row of a multi-entity command.
#[test]
fn color_aci_is_stamped_on_every_created_entity() {
    let (mut app, mut session) = application_with_document();
    set_session_color(&mut app, &mut session, 1);

    let rgb = crate::property_value::rgb_for_aci(1).expect("ACI 1 is a real colour");
    let red = cad_db::EntityColor::Explicit(rgb);

    let created = [
        execute(
            &mut app,
            &mut session,
            CommandId::CreateLine,
            CommandPayload::Points(vec![point(0.0, 0.0), point(10.0, 0.0)]),
        ),
        execute(
            &mut app,
            &mut session,
            CommandId::CreatePolyline,
            CommandPayload::Points(vec![point(0.0, 0.0), point(4.0, 0.0), point(4.0, 3.0)]),
        ),
        execute(
            &mut app,
            &mut session,
            CommandId::CreateCircle,
            CommandPayload::Points(vec![point(0.0, 0.0), point(5.0, 0.0)]),
        ),
        execute(
            &mut app,
            &mut session,
            CommandId::CreateRectangle,
            CommandPayload::Points(vec![point(0.0, 0.0), point(6.0, 4.0)]),
        ),
        // Two picks in one command: the batch funnel must colour **both** rows.
        execute(
            &mut app,
            &mut session,
            CommandId::CreatePoint,
            CommandPayload::Points(vec![point(1.0, 1.0), point(2.0, 2.0)]),
        ),
    ];

    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    let mut checked = 0;
    for outcome in created {
        let outcome = outcome.expect("creation succeeds");
        assert!(!outcome.objects.is_empty(), "a creation inserts something");
        for object in &outcome.objects {
            let id = EntityId(object.0);
            let attributes = drawing.entity_render_attributes(id);
            assert_eq!(
                attributes.color, red,
                "entity {} stores the session colour, not just a symbolic default",
                id.0
            );
            assert_eq!(
                crate::property_value::aci_for_rgb(rgb),
                Some(1),
                "the stored colour is exactly ACI 1's palette entry"
            );
            assert_eq!(
                attributes.transparency,
                cad_db::EntityTransparency::ByLayer,
                "colour is the only display attribute the token writes"
            );
            assert_eq!(
                attributes.lineweight,
                cad_db::EntityLineWeight::ByLayer,
                "colour is the only display attribute the token writes"
            );
            assert_eq!(
                crate::property_value::property_value(
                    drawing,
                    drawing.entity(id).expect("the inserted row"),
                    "color"
                )
                .as_deref(),
                Some("aci:1"),
                "the entity report projects the stored colour back as ACI 1"
            );
            assert_eq!(
                drawing.entity(id).expect("the inserted row").layer,
                LayerId(0),
                "the token colours the row; the active layer is unchanged"
            );
            checked += 1;
        }
    }
    assert_eq!(checked, 6, "LINE + POLYLINE + CIRCLE + RECTANG + 2 POINTs");
}

/// The default token is ByLayer and an explicit `COLOR BYLAYER` keeps it: the
/// created row is the **default** render-attribute set, i.e. no per-entity
/// override is written (exactly the pre-`COLOR` behaviour).
#[test]
fn color_bylayer_writes_no_per_entity_override() {
    let (mut app, mut session) = application_with_document();
    assert_eq!(
        session.drafting_extras.current_color(),
        256,
        "ByLayer is the documented default token"
    );
    // Walk the token away and back: the reset must be a real ByLayer, not a
    // "never set" coincidence.
    set_session_color(&mut app, &mut session, 4);
    set_session_color(&mut app, &mut session, 256);

    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::CreateLine,
        CommandPayload::Points(vec![point(0.0, 0.0), point(3.0, 0.0)]),
    )
    .expect("a ByLayer creation succeeds");
    let id = EntityId(outcome.objects[0].0);
    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    let attributes = drawing.entity_render_attributes(id);
    assert_eq!(attributes.color, cad_db::EntityColor::ByLayer);
    assert_eq!(
        attributes,
        cad_db::EntityRenderAttributes::default(),
        "ByLayer stages no display state, so the row keeps every default"
    );
}

/// `ByBlock` is the token's other symbolic value: ACI 0 stores the symbolic
/// variant and still resolves at the model root like any other ByBlock row.
#[test]
fn color_byblock_stays_symbolic() {
    let (mut app, mut session) = application_with_document();
    set_session_color(&mut app, &mut session, 0);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::CreateCircle,
        CommandPayload::Points(vec![point(0.0, 0.0), point(2.0, 0.0)]),
    )
    .expect("a ByBlock creation succeeds");
    let id = EntityId(outcome.objects[0].0);
    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    assert_eq!(
        drawing.entity_render_attributes(id).color,
        cad_db::EntityColor::ByBlock
    );
}

/// One undo step removes the created entity with its colour; redo restores both.
/// The patch records the display state per row, so the redo cannot come back
/// ByLayer while the geometry returns.
#[test]
fn created_colour_survives_undo_and_redo_symmetrically() {
    let (mut app, mut session) = application_with_document();
    set_session_color(&mut app, &mut session, 5);
    let rgb = crate::property_value::rgb_for_aci(5).expect("ACI 5 is a real colour");

    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::CreateLine,
        CommandPayload::Points(vec![point(0.0, 0.0), point(7.0, 0.0)]),
    )
    .expect("creation succeeds");
    let id = EntityId(outcome.objects[0].0);
    assert_eq!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity_render_attributes(id)
            .color,
        cad_db::EntityColor::Explicit(rgb)
    );

    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .expect("one undo step reverts the creation");
    assert!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity(id)
            .is_none(),
        "undo removes the entity row"
    );

    execute(
        &mut app,
        &mut session,
        CommandId::Redo,
        CommandPayload::None,
    )
    .expect("redo re-applies the creation");
    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    assert!(drawing.entity(id).is_some(), "redo restores the entity row");
    assert_eq!(
        drawing.entity_render_attributes(id).color,
        cad_db::EntityColor::Explicit(rgb),
        "redo restores the colour with the row, not a re-derived default"
    );
}

/// OFFSET does **not** go through the shared create funnel for its display state:
/// it copies the source row's, so the session token must not recolour a copy (the
/// copy's properties follow its source, not the session).
#[test]
fn offset_copies_the_source_colour_instead_of_the_session_token() {
    let (mut app, mut session) = application_with_document();
    set_session_color(&mut app, &mut session, 5);
    let source = execute(
        &mut app,
        &mut session,
        CommandId::CreateLine,
        CommandPayload::Points(vec![point(0.0, 0.0), point(10.0, 0.0)]),
    )
    .expect("source creation succeeds");
    let source_id = EntityId(source.objects[0].0);

    // Move the session token somewhere else: the copy must ignore it.
    set_session_color(&mut app, &mut session, 1);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::OffsetEntity,
        CommandPayload::Offset {
            refs: vec![ref_of(source_id.0)],
            side_point: point(0.5, 1.0),
        },
    )
    .expect("offset succeeds");
    let copy_id = EntityId(outcome.objects[0].0);
    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    assert_eq!(
        drawing.entity_render_attributes(copy_id).color,
        drawing.entity_render_attributes(source_id).color,
        "OFFSET preserves the source display state"
    );
    assert_eq!(
        drawing.entity_render_attributes(copy_id).color,
        cad_db::EntityColor::Explicit(
            crate::property_value::rgb_for_aci(5).expect("ACI 5 is a real colour")
        ),
        "the copy is ACI 5 (its source), not the session's ACI 1"
    );
}

/// A Viewer session may not steer what later creations inherit, but the bare
/// `COLOR` report stays Viewer-legal (it is a read-only report, not a write).
#[test]
fn viewer_refuses_the_colour_write_and_keeps_the_report() {
    let (mut app, mut session) = application_with_document();
    session.switch_mode(AppMode::Viewer).expect("Viewer mode");

    let error = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::Color),
        CommandPayload::CurrentColor(1),
    )
    .expect_err("a Viewer session cannot write the colour token");
    assert!(
        matches!(error, CadError::PermissionDenied),
        "the refusal is the shared permission refusal, got {error:?}"
    );
    assert_eq!(
        session.drafting_extras.current_color(),
        256,
        "the refused write leaves the token untouched"
    );

    let report = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::Color),
        CommandPayload::None,
    )
    .expect("the report is still Viewer-legal");
    assert!(report.changes.is_none(), "the report writes nothing");
    assert!(
        report
            .diagnostics
            .iter()
            .any(|row| row.code == "color.report" && row.message.contains("ByLayer")),
        "the report still names the current token"
    );
}

/// A token outside the closed ACI domain is refused by name, never clamped, and
/// the stored token keeps its previous value.
#[test]
fn color_outside_the_aci_domain_is_refused_by_name() {
    for bad in [-1, 257, 1000] {
        let (mut app, mut session) = application_with_document();
        let error = execute(
            &mut app,
            &mut session,
            CommandId::Extended(ExtendedCommandId::Color),
            CommandPayload::CurrentColor(bad),
        )
        .expect_err("an out-of-domain ACI is refused");
        match error {
            CadError::InvalidInput(reason) => assert!(
                reason.starts_with("color.invalid_value:"),
                "{bad} must name the stable refusal, got {reason}"
            ),
            other => panic!("{bad} must be InvalidInput, got {other:?}"),
        }
        assert_eq!(session.drafting_extras.current_color(), 256);
    }
}

/// The write path changes the session token only: no drawing row, no revision
/// bump and no undo entry. The entity a later command creates is the only write.
#[test]
fn the_color_write_itself_touches_no_drawing_state() {
    let (mut app, mut session) = application_with_document();
    let before = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::Color),
        CommandPayload::CurrentColor(3),
    )
    .expect("the token write succeeds");
    assert!(
        outcome.changes.is_none(),
        "the token write publishes no change"
    );
    assert!(
        outcome.objects.is_empty(),
        "the token write names no object"
    );
    assert_eq!(revision(&app), before, "no revision bump");
    assert!(!app.can_undo(&DocumentId(1)), "no undo entry");
    assert_eq!(session.drafting_extras.current_color(), 3);
}

/// Write the session `COLOR` token the way a command line does (one ACI payload).
fn set_session_color(app: &mut Application, session: &mut SessionState, aci: i16) {
    execute(
        app,
        session,
        CommandId::Extended(ExtendedCommandId::Color),
        CommandPayload::CurrentColor(aci),
    )
    .unwrap_or_else(|error| panic!("COLOR {aci} must store: {error:?}"));
}
