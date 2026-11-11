//! `TEXTMASK` / `TEXTUNMASK` contract tests: the offset factor, the one modelled
//! mask kind, the per-text `*TEXTMASK-*` session group, re-run replacement and
//! the deletion rules (group first, estimated-bounds heuristic second).
//!
//! The mask entity is `SemanticGeometry::Mask { inverted: false }`
//! (`AcDbWipeout`). AutoCAD's `3dface` and `solid` mask types have **no entity
//! model** in this build (there is no 3DFACE / 2D SOLID entity), so they stay
//! named `Unsupported` refusals (`textmask.mask_type`) and are never aliased to a
//! wipeout.

use super::*;

fn masks(app: &Application) -> Vec<DbEntity> {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .entities()
        .filter(|entity| matches!(entity.geometry, SemanticGeometry::Mask { .. }))
        .cloned()
        .collect()
}

fn boundary_of(mask: &DbEntity) -> Vec<Point3> {
    match &mask.geometry {
        SemanticGeometry::Mask { boundary, .. } => boundary.clone(),
        other => panic!("expected a Mask, got {other:?}"),
    }
}

fn standalone_mask(id: u128, min: Point3, max: Point3, order: i64) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbWipeout".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Mask {
            boundary: vec![
                p3(min.x, min.y, min.z),
                p3(max.x, min.y, max.z),
                p3(max.x, max.y, max.z),
                p3(min.x, max.y, max.z),
            ],
            inverted: false,
        },
        draw_order: order,
    }
}

#[test]
fn textmask_zero_offset_is_the_exact_estimate_and_an_empty_string_refuses() {
    let (mut app, mut session) = app_with(vec![simple_text(1, "abcd", p3(0.0, 0.0, 0.0), 5)]);
    select(&mut session, &[1]);
    execute(
        &mut app,
        &mut session,
        CommandId::TextMask,
        textmask_payload(crate::command_payload::TextMaskOptions {
            offset_factor: 0.0,
            kind: crate::command_payload::TextMaskKind::Wipeout,
        }),
    )
    .expect("a zero offset is the estimated box itself");
    let mask = find_by_type(&app, "AcDbWipeout").expect("mask");
    assert_eq!(
        boundary_of(&mask),
        vec![
            p3(0.0, 0.0, 0.0),
            p3(8.0, 0.0, 0.0),
            p3(8.0, 2.0, 0.0),
            p3(0.0, 2.0, 0.0),
        ],
        "height 2 x 4 characters, height 2, no growth"
    );

    // An empty string has no estimated width, so there is nothing to mask.
    let (mut app, mut session) = app_with(vec![simple_text(1, "", p3(0.0, 0.0, 0.0), 5)]);
    select(&mut session, &[1]);
    let before = revision(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::TextMask,
        CommandPayload::None,
    )
    .expect_err("an empty text is refused");
    assert_invalid(error, "textmask.degenerate");
    assert_eq!(revision(&app), before, "the refusal writes nothing");
    assert_eq!(undo_depth(&app), 0);
    assert!(masks(&app).is_empty());
    assert!(session.groups.is_empty());
}

#[test]
fn textmask_groups_each_text_with_its_own_mask_on_the_texts_layer() {
    let mut second = simple_text(2, "abcd", p3(10.0, 10.0, 0.0), 3);
    second.layer = LayerId(0);
    let (mut app, mut session) = app_with(vec![simple_text(1, "ab", p3(0.0, 0.0, 0.0), 5), second]);
    select(&mut session, &[1, 2]);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::TextMask,
        CommandPayload::None,
    )
    .expect("TEXTMASK");
    assert_eq!(outcome.objects.len(), 2);
    let stored = masks(&app);
    assert_eq!(stored.len(), 2);
    assert_eq!(
        session.groups.len(),
        2,
        "one pair group per text, never a combined group"
    );
    for group in session.groups.groups() {
        assert!(group.name.starts_with("*TEXTMASK-"), "{}", group.name);
        assert_eq!(group.members.len(), 2, "text + its own mask");
        let text = group.members[0].entity;
        let mask = group.members[1].entity;
        let mask = stored
            .iter()
            .find(|entity| entity.id == mask)
            .expect("the grouped mask exists");
        let text = entity(&app, text.0).expect("the grouped text exists");
        assert_eq!(mask.layer, text.layer, "the mask is on the text's layer");
        assert_eq!(
            mask.draw_order,
            text.draw_order.saturating_sub(1),
            "the mask sits just below its text"
        );
    }
}

#[test]
fn textmask_rerun_with_a_new_offset_replaces_the_previous_mask_geometry() {
    let (mut app, mut session) = app_with(vec![simple_text(1, "abcd", p3(0.0, 0.0, 0.0), 5)]);
    select(&mut session, &[1]);
    execute(
        &mut app,
        &mut session,
        CommandId::TextMask,
        CommandPayload::None,
    )
    .expect("first TEXTMASK");
    let first = find_by_type(&app, "AcDbWipeout").expect("mask").id;
    execute(
        &mut app,
        &mut session,
        CommandId::TextMask,
        textmask_payload(crate::command_payload::TextMaskOptions {
            offset_factor: 1.0,
            kind: crate::command_payload::TextMaskKind::Wipeout,
        }),
    )
    .expect("second TEXTMASK");
    assert!(entity(&app, first.0).is_none(), "the old mask is erased");
    let stored = masks(&app);
    assert_eq!(stored.len(), 1, "replacement, never a second mask");
    assert_eq!(
        boundary_of(&stored[0])[0],
        p3(-2.0, -2.0, 0.0),
        "the new offset factor is applied"
    );
    assert_eq!(session.groups.len(), 1, "the group is replaced in place");
    assert_eq!(undo_depth(&app), 2, "each TEXTMASK is one undo step");
}

#[test]
fn textmask_refuses_a_full_group_store_and_a_non_text_target_without_writing() {
    // The session group store holds 64 groups; masking one more text must be a
    // named refusal rather than a mask without its pair group.
    let (mut app, mut session) = app_with(vec![simple_text(1, "abcd", p3(0.0, 0.0, 0.0), 5)]);
    for index in 0..64 {
        session
            .groups
            .create(&format!("FULLER-{index}"), Vec::new());
    }
    select(&mut session, &[1]);
    let before = revision(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::TextMask,
        CommandPayload::None,
    )
    .expect_err("a full group store is refused");
    assert_unsupported(error, "textmask.group_cap");
    assert_eq!(revision(&app), before, "the refusal writes nothing");
    assert!(masks(&app).is_empty());
    assert_eq!(session.groups.len(), 64);

    // A non-text target refuses the whole command.
    let (mut app, mut session) = app_with(vec![
        simple_text(1, "abcd", p3(0.0, 0.0, 0.0), 5),
        line_entity(2),
    ]);
    select(&mut session, &[1, 2]);
    let before = revision(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::TextMask,
        CommandPayload::None,
    )
    .expect_err("a non-text target is refused");
    assert_unsupported(error, "textmask.not_text");
    assert_eq!(revision(&app), before);
    assert!(masks(&app).is_empty());
    assert!(session.groups.is_empty());
}

#[test]
fn textunmask_keeps_another_texts_mask_and_group_intact() {
    let (mut app, mut session) = app_with(vec![
        simple_text(1, "ab", p3(0.0, 0.0, 0.0), 5),
        simple_text(2, "abcd", p3(10.0, 10.0, 0.0), 7),
    ]);
    select(&mut session, &[1, 2]);
    execute(
        &mut app,
        &mut session,
        CommandId::TextMask,
        CommandPayload::None,
    )
    .expect("mask both texts");
    assert_eq!(session.groups.len(), 2);
    let second_mask = session
        .groups
        .groups()
        .find(|group| group.members[0].entity == EntityId(2))
        .expect("the second text's group")
        .members[1]
        .entity;

    // Only the first text is selected: its grouped mask goes, the other pair is
    // untouched and stays removable through its own group.
    select(&mut session, &[1]);
    execute(
        &mut app,
        &mut session,
        CommandId::TextUnmask,
        CommandPayload::None,
    )
    .expect("TEXTUNMASK the first text");
    assert!(
        entity(&app, second_mask.0).is_some(),
        "the other mask stays"
    );
    assert_eq!(masks(&app).len(), 1);
    assert_eq!(session.groups.len(), 1, "only the first group is dropped");
    assert_eq!(
        session.groups.groups().next().unwrap().members[0].entity,
        EntityId(2)
    );

    select(&mut session, &[2]);
    execute(
        &mut app,
        &mut session,
        CommandId::TextUnmask,
        CommandPayload::None,
    )
    .expect("TEXTUNMASK the second text");
    assert!(masks(&app).is_empty(), "no orphan mask is left behind");
    assert!(session.groups.is_empty(), "no orphan group is left behind");
}

#[test]
fn textunmask_reports_a_named_no_match_and_writes_nothing() {
    // A text without any mask and without a group.
    let (mut app, mut session) = app_with(vec![simple_text(1, "abcd", p3(0.0, 0.0, 0.0), 5)]);
    select(&mut session, &[1]);
    let before = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::TextUnmask,
        CommandPayload::None,
    )
    .expect("nothing to unmask is not an error");
    assert!(outcome.changes.is_none(), "no ChangeSet");
    assert!(outcome.objects.is_empty());
    assert!(
        outcome
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "textunmask.unchanged"),
        "{:?}",
        outcome.diagnostics
    );
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 0);

    // A selected mask outside every selected text's estimated bounds is kept:
    // the heuristic cannot prove provenance and does not guess.
    let (mut app, mut session) = app_with(vec![
        simple_text(1, "abcd", p3(0.0, 0.0, 0.0), 5),
        standalone_mask(2, p3(100.0, 100.0, 0.0), p3(110.0, 110.0, 0.0), 6),
    ]);
    select(&mut session, &[1, 2]);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::TextUnmask,
        CommandPayload::None,
    )
    .expect("an out-of-bounds mask is not deleted");
    assert!(
        outcome
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "textunmask.unchanged"),
        "{:?}",
        outcome.diagnostics
    );
    assert_eq!(masks(&app).len(), 1, "the unrelated mask is kept");
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn textunmask_drops_a_stale_group_when_its_mask_is_already_gone() {
    let (mut app, mut session) = app_with(vec![simple_text(1, "abcd", p3(0.0, 0.0, 0.0), 5)]);
    select(&mut session, &[1]);
    execute(
        &mut app,
        &mut session,
        CommandId::TextMask,
        CommandPayload::None,
    )
    .expect("TEXTMASK");
    let mask = find_by_type(&app, "AcDbWipeout").expect("mask").id;
    assert_eq!(session.groups.len(), 1);

    // Erase the mask row directly, as any other erase path would.
    select(&mut session, &[mask.0]);
    execute(
        &mut app,
        &mut session,
        CommandId::EraseEntities,
        CommandPayload::None,
    )
    .expect("ERASE the mask");
    assert!(masks(&app).is_empty());
    assert_eq!(
        session.groups.len(),
        1,
        "ERASE leaves the session group (it is not drawing data)"
    );

    // ERASE clears the selection; select the surviving text again.
    select(&mut session, &[1]);
    let before = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::TextUnmask,
        CommandPayload::None,
    )
    .expect("a stale association is cleaned up, not an error");
    assert!(
        outcome
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "textunmask.stale_group"),
        "{:?}",
        outcome.diagnostics
    );
    assert!(
        outcome
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "textunmask.unchanged"),
        "{:?}",
        outcome.diagnostics
    );
    assert!(outcome.changes.is_none(), "no drawing transaction");
    assert!(session.groups.is_empty(), "the stale group is gone");
    assert_eq!(revision(&app), before, "no revision bump");
}
