//! `FS` `FSMODE=ON` tangent-chain contract tests (split out of `inquiry` to keep
//! both files inside the code-size limit).
//!
//! Pinned here: the three-contact chain, the same input with `FSMODE=OFF`, and
//! every chain refusal (`fs.chain_nontangent`, `fs.chain_ambiguous`,
//! `fs.chain_bound`) — each refusing before the selection changes and without any
//! drawing write.

use super::inquiry::{app_from_builder, command, diagnostic, fs_fixture, line, point};
use super::*;
use crate::selection::model_ref;
use cad_db::{DrawingDatabaseBuilder, Layer};

// --- FS: FSMODE=ON tangent chain ------------------------------------------

/// Four collinear `LINE`s joined end-to-end: a three-contact tangent chain.
fn fs_chain_fixture() -> (Application, SessionState, DocumentId) {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    for index in 0..4u128 {
        let start = 10.0 * index as f64;
        builder
            .insert_entity(line(index + 1, point(start, 0.0), point(start + 10.0, 0.0)))
            .unwrap();
    }
    let (app, mut session, document) = app_from_builder(builder);
    session.selection = SelectionSet::from_refs([model_ref(document, EntityId(1))]);
    (app, session, document)
}

#[test]
fn fs_chain_mode_follows_three_tangent_contacts() {
    let (mut app, mut session, document) = fs_chain_fixture();
    session.drafting_extras.set_fs_mode(true);

    let outcome = app
        .execute(&mut session, command(CommandId::Fs, CommandPayload::None))
        .expect("FS chains the collinear tangent contacts");

    assert_eq!(
        session.selection.len(),
        4,
        "seed plus the three chained segments"
    );
    for id in 1..=4u128 {
        assert!(
            session
                .selection
                .contains(&model_ref(document, EntityId(id))),
            "entity {id} is in the chain"
        );
    }
    let applied = diagnostic(&outcome, "fs.applied");
    assert_eq!(applied.len(), 1);
    assert!(
        applied[0].contains("added 3 touching object(s)"),
        "{applied:?}"
    );

    let mode = diagnostic(&outcome, "fs.mode");
    assert_eq!(mode.len(), 1);
    assert!(mode[0].contains("FSMODE=ON"), "{mode:?}");
    assert!(mode[0].contains("tangent-chain"), "{mode:?}");
    assert!(
        mode[0].contains("4 visible model-space curve(s)"),
        "the chain length is named: {mode:?}"
    );
    assert!(mode[0].contains("3 tangent contact(s)"), "{mode:?}");

    // FS never writes: no revision bump and no undo entry.
    assert!(!app.can_undo(&document));
}

#[test]
fn fs_chain_mode_off_adds_only_the_direct_contact_of_the_same_input() {
    let (mut app, mut session, document) = fs_chain_fixture();
    assert!(!session.drafting_extras.fs_mode(), "OFF is the default");

    let outcome = app
        .execute(&mut session, command(CommandId::Fs, CommandPayload::None))
        .expect("FS adds the directly touching segment only");

    assert_eq!(session.selection.len(), 2, "seed plus one direct contact");
    assert!(session
        .selection
        .contains(&model_ref(document, EntityId(2))));
    assert!(
        !session
            .selection
            .contains(&model_ref(document, EntityId(3))),
        "the second contact is not transitive with FSMODE=OFF"
    );
    assert!(diagnostic(&outcome, "fs.applied")[0].contains("added 1 touching object(s)"));
    let mode = diagnostic(&outcome, "fs.mode");
    assert!(mode[0].contains("FSMODE=OFF"), "{mode:?}");
    assert!(mode[0].contains("direct-contact"), "{mode:?}");
    assert!(!app.can_undo(&document));
}

#[test]
fn fsmode_set_through_the_command_line_changes_what_fs_does() {
    // End to end: the typed/session `FSMODE` command is the only setter, and `FS`
    // reads exactly that token. The same input selects two or four entities
    // depending on the mode, and neither pass writes the drawing.
    let (mut app, mut session, document) = fs_chain_fixture();
    assert!(!session.drafting_extras.fs_mode(), "OFF is the default");

    app.execute(
        &mut session,
        command(
            CommandId::Extended(ExtendedCommandId::FsMode),
            CommandPayload::FsMode(true),
        ),
    )
    .expect("FSMODE ON is admitted");
    assert!(session.drafting_extras.fs_mode());

    let outcome = app
        .execute(&mut session, command(CommandId::Fs, CommandPayload::None))
        .expect("FS chains the collinear tangent contacts");
    assert_eq!(
        session.selection.len(),
        4,
        "the mode set through the command turns on the chain pass"
    );
    assert!(diagnostic(&outcome, "fs.mode")[0].contains("FSMODE=ON"));
    assert!(!app.can_undo(&document), "FSMODE + FS write nothing");

    // Back to OFF through the same command: the direct-contact pass again.
    session.selection = SelectionSet::from_refs([model_ref(document, EntityId(1))]);
    app.execute(
        &mut session,
        command(
            CommandId::Extended(ExtendedCommandId::FsMode),
            CommandPayload::FsMode(false),
        ),
    )
    .expect("FSMODE OFF is admitted");
    let outcome = app
        .execute(&mut session, command(CommandId::Fs, CommandPayload::None))
        .expect("FS adds the directly touching segment only");
    assert_eq!(session.selection.len(), 2);
    assert!(diagnostic(&outcome, "fs.mode")[0].contains("FSMODE=OFF"));
    assert!(!app.can_undo(&document), "still no drawing write");
}

#[test]
fn fs_chain_mode_refuses_a_non_tangent_contact_and_writes_nothing() {
    let (mut app, mut session, document) = fs_fixture();
    session.drafting_extras.set_fs_mode(true);
    session.selection = SelectionSet::from_refs([model_ref(document, EntityId(1))]);
    let before = app
        .workspace
        .documents
        .get(&document)
        .unwrap()
        .drawing
        .clone();

    let error = app
        .execute(&mut session, command(CommandId::Fs, CommandPayload::None))
        .expect_err("a transversal crossing is not a tangent contact");
    assert!(error.to_string().contains("fs.chain_nontangent"), "{error}");
    assert_eq!(session.selection.len(), 1, "the selection is unchanged");
    let after = app
        .workspace
        .documents
        .get(&document)
        .unwrap()
        .drawing
        .clone();
    assert_eq!(after.revision(), before.revision());
    assert!(!app.can_undo(&document));
}

#[test]
fn fs_chain_mode_refuses_an_ambiguous_branch_and_writes_nothing() {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder
        .insert_entity(line(1, point(0.0, 0.0), point(10.0, 0.0)))
        .unwrap();
    // Two distinct entities tangent to the seed at the same contact point: the
    // chain cannot say which one continues.
    builder
        .insert_entity(line(2, point(10.0, 0.0), point(20.0, 0.0)))
        .unwrap();
    builder
        .insert_entity(line(3, point(10.0, 0.0), point(20.0, 0.0)))
        .unwrap();
    let (mut app, mut session, document) = app_from_builder(builder);
    session.drafting_extras.set_fs_mode(true);
    session.selection = SelectionSet::from_refs([model_ref(document, EntityId(1))]);

    let error = app
        .execute(&mut session, command(CommandId::Fs, CommandPayload::None))
        .expect_err("two tangent continuations at one point are ambiguous");
    assert!(error.to_string().contains("fs.chain_ambiguous"), "{error}");
    assert_eq!(session.selection.len(), 1);
    assert!(!app.can_undo(&document));
}

#[test]
fn fs_chain_mode_refuses_past_the_documented_iteration_bound() {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    // 70 segments: a 71-entity chain, past the documented bound of 64.
    for index in 0..70u128 {
        let start = 10.0 * index as f64;
        builder
            .insert_entity(line(index + 1, point(start, 0.0), point(start + 10.0, 0.0)))
            .unwrap();
    }
    let (mut app, mut session, document) = app_from_builder(builder);
    session.drafting_extras.set_fs_mode(true);
    session.selection = SelectionSet::from_refs([model_ref(document, EntityId(1))]);

    let error = app
        .execute(&mut session, command(CommandId::Fs, CommandPayload::None))
        .expect_err("a chain past the bound is refused, never truncated");
    assert!(error.to_string().contains("fs.chain_bound"), "{error}");
    assert_eq!(
        session.selection.len(),
        1,
        "the refusal selects nothing beyond the seed"
    );
    assert!(!app.can_undo(&document));
}

#[test]
fn fs_chain_mode_reports_no_touch_without_changing_the_selection() {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder
        .insert_entity(line(1, point(0.0, 0.0), point(1.0, 0.0)))
        .unwrap();
    builder
        .insert_entity(line(2, point(50.0, 50.0), point(51.0, 50.0)))
        .unwrap();
    let (mut app, mut session, document) = app_from_builder(builder);
    session.drafting_extras.set_fs_mode(true);
    session.selection = SelectionSet::from_refs([model_ref(document, EntityId(1))]);

    let outcome = app
        .execute(&mut session, command(CommandId::Fs, CommandPayload::None))
        .expect("FS reports no touch as a diagnostic, not an error");
    let none = diagnostic(&outcome, "fs.none");
    assert_eq!(none.len(), 1);
    assert!(none[0].contains("FSMODE=ON"), "{none:?}");
    assert_eq!(session.selection.len(), 1);
}
