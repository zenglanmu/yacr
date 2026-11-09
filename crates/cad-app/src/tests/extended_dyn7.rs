//! Wave 7 dynamic-block visibility contracts: `BVSTATE`, `-BVSTATE`.
//!
//! The two reports are pure reads of the existing
//! `DrawingDatabase::block_dynamic_visibility` accessor (the read half of
//! `set_block_visibility_state`). They must report the active state of each
//! directly selected INSERT, name a missing state as `*.not_set`, name a
//! selection with no INSERT as `*.no_selection`, and never write the drawing.
//! Each test pins the diagnostic codes so a report can never invent a state.

use super::*;
use cad_db::{
    BlockDefinition, DbEntity, DbObject, DrawingDatabaseBuilder, DynamicBlockState,
    DynamicBlockVisibility, Layer,
};

fn p3(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

fn line(id: u128, space: SpaceId) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbLine".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space,
        geometry: SemanticGeometry::Line {
            start: p3(0.0, 0.0, 0.0),
            end: p3(1.0, 0.0, 0.0),
        },
        draw_order: 0,
    }
}

fn insert_entity(id: u128, block: u128) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbBlockReference".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Insert {
            block: BlockId(block),
            transform: Transform3::identity(),
        },
        draw_order: 0,
    }
}

fn visibility(active: Option<&str>) -> DynamicBlockVisibility {
    DynamicBlockVisibility {
        member_entities: vec![EntityId(10), EntityId(11)],
        states: vec![
            DynamicBlockState {
                name: "A".into(),
                entities: vec![EntityId(10)],
            },
            DynamicBlockState {
                name: "B".into(),
                entities: vec![EntityId(11)],
            },
        ],
        active_state: active.map(str::to_string),
    }
}

/// A drawing with three blocks and two model-space INSERTs:
///
/// * block 1 (`DOOR`) — visibility states A/B, active A;
/// * block 2 (`PLAIN`) — no visibility descriptor;
/// * block 3 (`UNRESOLVED`) — states A/B, no resolved active state.
fn application_with_dynamic_blocks() -> (Application, SessionState) {
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
    b.insert_entity(line(10, SpaceId::Block(BlockId(1))))
        .unwrap();
    b.insert_entity(line(11, SpaceId::Block(BlockId(1))))
        .unwrap();
    b.insert_entity(line(12, SpaceId::Block(BlockId(3))))
        .unwrap();
    b.insert_block(BlockDefinition {
        id: BlockId(1),
        entities: vec![EntityId(10), EntityId(11)],
        dynamic_visibility: None,
    })
    .unwrap();
    b.set_block_metadata(BlockId(1), "DOOR", p3(0.0, 0.0, 0.0))
        .unwrap();
    b.set_block_dynamic_visibility(BlockId(1), visibility(Some("A")))
        .unwrap();
    b.insert_block(BlockDefinition {
        id: BlockId(2),
        entities: Vec::new(),
        dynamic_visibility: None,
    })
    .unwrap();
    b.set_block_metadata(BlockId(2), "PLAIN", p3(0.0, 0.0, 0.0))
        .unwrap();
    b.insert_block(BlockDefinition {
        id: BlockId(3),
        entities: vec![EntityId(12)],
        dynamic_visibility: None,
    })
    .unwrap();
    b.set_block_metadata(BlockId(3), "UNRESOLVED", p3(0.0, 0.0, 0.0))
        .unwrap();
    b.set_block_dynamic_visibility(
        BlockId(3),
        DynamicBlockVisibility {
            member_entities: vec![EntityId(12)],
            states: vec![DynamicBlockState {
                name: "A".into(),
                entities: vec![EntityId(12)],
            }],
            active_state: None,
        },
    )
    .unwrap();
    b.insert_entity(insert_entity(20, 1)).unwrap();
    b.insert_entity(insert_entity(21, 2)).unwrap();
    b.insert_entity(insert_entity(22, 3)).unwrap();
    app.workspace
        .documents
        .get_mut(&DocumentId(1))
        .unwrap()
        .drawing = Arc::new(b.finish().unwrap());
    (app, session)
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
    ext: ExtendedCommandId,
) -> CadResult<CommandOutcome> {
    app.execute(
        session,
        command(CommandId::Extended(ext), CommandPayload::None),
    )
}

fn revision(app: &Application) -> Revision {
    app.workspace.documents[&DocumentId(1)].drawing.revision()
}

fn diagnostic<'a>(outcome: &'a CommandOutcome, code: &str) -> &'a Diagnostic {
    outcome
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == code)
        .unwrap_or_else(|| panic!("missing {code}"))
}

fn count_code(outcome: &CommandOutcome, code: &str) -> usize {
    outcome
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == code)
        .count()
}

#[test]
fn bvstate_reports_the_active_visibility_state_of_a_selected_insert() {
    let (mut app, mut session) = application_with_dynamic_blocks();
    let before = revision(&app);
    select(&mut session, &[20]);

    let outcome = execute(&mut app, &mut session, ExtendedCommandId::BvState)
        .expect("BVSTATE is a report, not an error");
    assert!(outcome.changes.is_none());
    let report = diagnostic(&outcome, "bvstate.report");
    assert!(report.message.contains("'DOOR'"), "{}", report.message);
    assert!(
        report.message.contains("active visibility state 'A'"),
        "{}",
        report.message
    );
    assert!(
        report.message.contains("available: A, B"),
        "{}",
        report.message
    );
    assert!(report.message.contains("not changed"), "{}", report.message);
    assert_eq!(count_code(&outcome, "bvstate.report"), 1);
    assert_eq!(count_code(&outcome, "bvstate.not_set"), 0);
    assert_eq!(count_code(&outcome, "bvstate.no_selection"), 0);
    assert_eq!(revision(&app), before, "BVSTATE writes no drawing");
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn bvstate_with_no_insert_selected_is_a_named_no_selection_report() {
    let (mut app, mut session) = application_with_dynamic_blocks();
    let before = revision(&app);

    let outcome = execute(&mut app, &mut session, ExtendedCommandId::BvState)
        .expect("BVSTATE is a report, not an error");
    assert!(outcome.changes.is_none());
    assert_eq!(count_code(&outcome, "bvstate.no_selection"), 1);
    assert_eq!(count_code(&outcome, "bvstate.report"), 0);
    assert_eq!(count_code(&outcome, "bvstate.not_set"), 0);
    assert!(diagnostic(&outcome, "bvstate.no_selection")
        .message
        .contains("no INSERT is selected"));
    assert_eq!(revision(&app), before);
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn an_insert_whose_block_has_no_visibility_state_reports_not_set() {
    let (mut app, mut session) = application_with_dynamic_blocks();
    let before = revision(&app);
    select(&mut session, &[21]);

    let outcome = execute(&mut app, &mut session, ExtendedCommandId::BvState)
        .expect("BVSTATE is a report, not an error");
    assert!(outcome.changes.is_none());
    let report = diagnostic(&outcome, "bvstate.not_set");
    assert!(report.message.contains("'PLAIN'"), "{}", report.message);
    assert!(
        report
            .message
            .contains("no dynamic-block visibility descriptor"),
        "{}",
        report.message
    );
    assert_eq!(count_code(&outcome, "bvstate.report"), 0);
    assert_eq!(revision(&app), before);
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn an_unresolved_active_state_reports_not_set_instead_of_guessing() {
    let (mut app, mut session) = application_with_dynamic_blocks();
    let before = revision(&app);
    select(&mut session, &[22]);

    let outcome = execute(&mut app, &mut session, ExtendedCommandId::BvState)
        .expect("BVSTATE is a report, not an error");
    let report = diagnostic(&outcome, "bvstate.not_set");
    assert!(
        report
            .message
            .contains("no active visibility state was resolved"),
        "{}",
        report.message
    );
    assert_eq!(count_code(&outcome, "bvstate.report"), 0);
    assert_eq!(revision(&app), before);
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn bvstate_reports_every_selected_insert_and_ignores_non_inserts() {
    let (mut app, mut session) = application_with_dynamic_blocks();
    let before = revision(&app);
    select(&mut session, &[10, 20, 22]);

    let outcome = execute(&mut app, &mut session, ExtendedCommandId::BvState)
        .expect("BVSTATE is a report, not an error");
    assert_eq!(count_code(&outcome, "bvstate.report"), 1);
    assert_eq!(count_code(&outcome, "bvstate.not_set"), 1);
    assert_eq!(count_code(&outcome, "bvstate.no_selection"), 0);
    assert_eq!(revision(&app), before);
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn dash_bvstate_reports_the_same_state_under_its_own_codes() {
    let (mut app, mut session) = application_with_dynamic_blocks();
    let before = revision(&app);
    select(&mut session, &[20]);

    let outcome = execute(&mut app, &mut session, ExtendedCommandId::DashBvState)
        .expect("-BVSTATE is a report, not an error");
    assert!(diagnostic(&outcome, "-bvstate.report")
        .message
        .contains("active visibility state 'A'"));
    assert_eq!(count_code(&outcome, "bvstate.report"), 0);

    // The same command with nothing selected uses the dash no-selection code.
    session.selection = SelectionSet::new();
    let empty = execute(&mut app, &mut session, ExtendedCommandId::DashBvState)
        .expect("-BVSTATE is a report, not an error");
    assert_eq!(count_code(&empty, "-bvstate.no_selection"), 1);
    assert_eq!(count_code(&empty, "bvstate.no_selection"), 0);

    assert_eq!(revision(&app), before);
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn a_visibility_report_payload_is_refused_and_changes_nothing() {
    let (mut app, mut session) = application_with_dynamic_blocks();
    let before = revision(&app);
    let refused = app.execute(
        &mut session,
        command(
            CommandId::Extended(ExtendedCommandId::BvState),
            CommandPayload::Points(vec![point(1.0, 2.0)]),
        ),
    );
    assert!(refused.is_err(), "BVSTATE takes no payload");
    assert_eq!(revision(&app), before);
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn the_visibility_reports_stay_viewer_legal() {
    let (mut app, mut session) = application_with_dynamic_blocks();
    session.switch_mode(AppMode::Viewer).unwrap();
    select(&mut session, &[20]);

    for ext in [ExtendedCommandId::BvState, ExtendedCommandId::DashBvState] {
        assert!(
            !ext.requires_work_mode(),
            "{} is Viewer-legal",
            ext.selector()
        );
        let outcome = execute(&mut app, &mut session, ext)
            .unwrap_or_else(|error| panic!("{} failed: {error}", ext.selector()));
        assert!(outcome.changes.is_none());
    }
    assert!(!app.can_undo(&DocumentId(1)));
}
