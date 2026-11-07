//! `LAYMCH` (AutoCAD 2026): move the selection to the first pick's layer.
//!
//! Declared pre-selection semantics: the first selected entity's stored layer is
//! the destination, every other selected model-space entity is moved there in one
//! transaction / one undo step. Refusals: empty selection, a single-object
//! selection, a payload, and a foreign/instance/sub-element/paper-space ref.

use super::*;

fn layers() -> Vec<(LayerId, &'static str, LayerAttributes)> {
    vec![
        (LayerId(0), "0", LayerAttributes::default()),
        (LayerId(3), "WALLS", LayerAttributes::default()),
        (LayerId(5), "DOORS", LayerAttributes::default()),
    ]
}

#[test]
fn laymch_moves_the_rest_of_the_selection_to_the_first_picks_layer() {
    let (mut app, mut session) = app_with(
        layers(),
        vec![
            line(1, LayerId(3), 0.0),
            line(2, LayerId(5), 5.0),
            line(3, LayerId(5), 10.0),
        ],
    );
    select(&mut app, &mut session, &[1, 2, 3]);
    let before = state(&app);

    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::LayMCh,
        CommandPayload::None,
    )
    .expect("LAYMCH moves the rest of the selection");

    assert_eq!(
        entity_layer(&app, 1),
        LayerId(3),
        "the first pick is the destination"
    );
    assert_eq!(entity_layer(&app, 2), LayerId(3));
    assert_eq!(entity_layer(&app, 3), LayerId(3));
    assert_eq!(
        revision(&app),
        Revision(before.0 .0 + 1),
        "one revision bump"
    );
    assert_eq!(undo_depth(&app), before.1 + 1, "one undo step");
    assert_eq!(
        outcome.objects.len(),
        2,
        "only the two moved entities are reported"
    );
    assert!(outcome.changes.is_some());
    assert_eq!(diagnostic_codes(&outcome), vec!["laymch.applied"]);
    assert!(outcome.diagnostics[0].message.contains("'WALLS'"));
}

#[test]
fn laymch_refuses_a_single_object_selection() {
    let (mut app, mut session) = app_with(layers(), vec![line(1, LayerId(5), 0.0)]);
    select(&mut app, &mut session, &[1]);
    let before = state(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::LayMCh,
        CommandPayload::None,
    )
    .expect_err("nothing can be moved");
    assert!(refusal(error).starts_with("laymch.single_selection:"));
    assert_untouched(&app, before);
}

#[test]
fn laymch_refuses_an_empty_selection_and_any_payload() {
    let (mut app, mut session) = app_with(layers(), vec![line(1, LayerId(5), 0.0)]);
    let before = state(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::LayMCh,
        CommandPayload::None,
    )
    .expect_err("empty selection is a refusal");
    assert!(refusal(error).starts_with("laymch.selection:"));
    assert_untouched(&app, before);

    select(&mut app, &mut session, &[1]);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::LayMCh,
        CommandPayload::LayerVisibilities(vec![(LayerId(3), false)]),
    )
    .expect_err("a payload is refused");
    assert!(refusal(error).starts_with("laymch.payload:"));
    assert_untouched(&app, before);
}

#[test]
fn laymch_refuses_foreign_instance_and_non_model_refs() {
    let (mut app, mut session) = app_with(
        layers(),
        vec![line(1, LayerId(3), 0.0), line(2, LayerId(5), 5.0)],
    );
    let foreign = SelectionRef {
        document: DocumentId(2),
        ..ref_of(1)
    };
    session.selection = SelectionSet::from_refs([foreign, ref_of(2)]);
    let before = state(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::LayMCh,
        CommandPayload::None,
    )
    .expect_err("another document's ref is refused");
    assert!(refusal(error).starts_with("laymch.not_addressable:"));
    assert_untouched(&app, before);

    let instance = SelectionRef {
        instance: InstancePath(vec![EntityId(1)]),
        ..ref_of(2)
    };
    session.selection = SelectionSet::from_refs([ref_of(1), instance]);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::LayMCh,
        CommandPayload::None,
    )
    .expect_err("a block-instance member is refused");
    assert!(refusal(error).starts_with("laymch.not_addressable:"));
    assert_untouched(&app, before);
}

#[test]
fn laymch_reports_a_noop_when_everything_is_already_on_the_destination() {
    let (mut app, mut session) = app_with(
        layers(),
        vec![line(1, LayerId(3), 0.0), line(2, LayerId(3), 5.0)],
    );
    select(&mut app, &mut session, &[1, 2]);
    let before = state(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::LayMCh,
        CommandPayload::None,
    )
    .expect("the no-op is reported, not an error");
    assert_eq!(state(&app), before, "no transaction, no undo entry");
    assert!(outcome.changes.is_none());
    assert_eq!(diagnostic_codes(&outcome), vec!["laymch.unchanged"]);
}
