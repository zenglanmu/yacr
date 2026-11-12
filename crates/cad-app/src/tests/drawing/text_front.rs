//! TEXTTOFRONT (AutoCAD 2026) contract tests.
//!
//! The current selection is the input. Selected model-space
//! Text/MultiLeader/Dimension rows are moved above every non-eligible model-space
//! row, preserving their relative order; everything else produces a per-entity
//! `texttofront.skipped` diagnostic and never fails the command. Already-front
//! input is a no-op with no revision/undo entry; undo restores exact orders.

use super::*;

use super::hatch::undo_depth;

fn text_app(entities: Vec<(u128, SemanticGeometry, i64)>) -> (Application, SessionState) {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    for (id, geometry, order) in entities {
        b.insert_entity(ent(id, geometry, order)).unwrap();
    }
    let drawing = b.finish().unwrap();
    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([8u8; 32]),
            units: UnitContext::drawing_units(),
            resource_keys: Vec::new(),
        },
    );
    app.workspace.viewports.insert(
        ViewportId(1),
        Viewport::new(ViewportId(1), DocumentId(1), [800.0, 600.0]),
    );
    (app, SessionState::new(DocumentId(1), AppMode::Work))
}

fn line(start: Point3, end: Point3) -> SemanticGeometry {
    SemanticGeometry::Line { start, end }
}

fn text(id: u128) -> SemanticGeometry {
    SemanticGeometry::Text {
        text: format!("T{id}"),
        position: point(0.0, 0.0),
        style: StyleId(0),
        height: 2.5,
        rotation: 0.0,
        font: None,
        h_align: TextAlignH::Left,
        v_align: TextAlignV::Baseline,
        wrap_width: None,
        width_factor: 1.0,
    }
}

/// A straight-line multileader with a dogleg and no text content.
fn leader() -> SemanticGeometry {
    let (geometry, _) = MultiLeaderGeometry::from_import(MultiLeaderParts {
        style: MLeaderStyleValues {
            path_type: MLeaderPathType::StraightLineSegments,
            enable_landing: true,
            enable_dogleg: true,
            landing_distance: 2.0,
            landing_gap: 0.1,
            arrowhead_size: 0.5,
            text_height: 0.2,
            text_style: StyleId(0),
            text_style_name: "Standard".into(),
            text_font: None,
            scale_factor: 1.0,
        },
        style_name: "Standard".into(),
        override_mask: 0,
        content: MLeaderContent::None,
        text: MLeaderText {
            string: String::new(),
            location: point(0.0, 0.0),
            rotation: 0.0,
            height: 0.2,
            width: 0.0,
            attachment_point: 2,
            left_attachment: 2,
        },
        roots: vec![MLeaderRoot {
            leader_index: 0,
            landing_point: point(10.0, 5.0),
            dogleg_vector: point(1.0, 0.0),
            dogleg_length: 2.0,
        }],
        lines: vec![MLeaderLine {
            root_index: 0,
            index: 0,
            points: vec![point(0.0, 0.0), point(5.0, 0.0), point(8.0, 0.0)],
            path_type: MLeaderPathType::StraightLineSegments,
            override_flags: 0,
            arrowhead_size: 0.5,
            arrowhead_handle: None,
        }],
        reserved: MLeaderReserved::default(),
        plane: PlaneFrame::default(),
    });
    SemanticGeometry::MultiLeader(Box::new(geometry))
}

fn dimension() -> SemanticGeometry {
    let style = cad_db::DimStyle::standard_metric(DimStyleId(0)).resolved(
        "Standard".to_string(),
        StyleId(0),
        None,
    );
    let geometry = DimensionGeometry::synthesized(
        DimensionDefinition {
            kind: DimensionKind::Linear { rotation: 0.0 },
            points: DimensionPoints::Linear {
                first: point(0.0, 0.0),
                second: point(10.0, 0.0),
                dimension_line: point(5.0, -5.0),
            },
            text_middle: point(5.0, -5.0),
            text_rotation: 0.0,
            normal: p3(0.0, 0.0, 1.0),
        },
        "Standard".to_string(),
        style,
        None,
    )
    .expect("fixture dimension synthesizes");
    SemanticGeometry::Dimension(Box::new(geometry))
}

fn order_of(app: &Application, id: u128) -> i64 {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(id))
        .unwrap_or_else(|| panic!("entity {id} exists"))
        .draw_order
}

fn select(session: &mut SessionState, ids: &[u128]) {
    session.selection = SelectionSet::from_refs(ids.iter().copied().map(ref_of));
}

fn run_text_front(app: &mut Application, session: &mut SessionState) -> CadResult<CommandOutcome> {
    execute(app, session, CommandId::TextToFront, CommandPayload::None)
}

#[test]
fn mixed_text_leader_and_dimension_move_above_every_non_text_object() {
    let (mut app, mut session) = text_app(vec![
        (1, line(point(0.0, 0.0), point(1.0, 0.0)), 0),
        (2, text(2), 1),
        (3, line(point(0.0, 1.0), point(1.0, 1.0)), 2),
        (4, leader(), 3),
        (5, dimension(), 4),
        (6, line(point(0.0, 2.0), point(1.0, 2.0)), 5),
    ]);
    select(&mut session, &[2, 4, 5]);
    let before_revision = revision(&app);

    let outcome = run_text_front(&mut app, &mut session).expect("TEXTTOFRONT succeeds");
    assert_eq!(outcome.objects.len(), 3);
    // Non-eligible max is 5 -> the eligible rows receive 6, 7, 8 in their old
    // relative order (2, then 4, then 5).
    assert_eq!(order_of(&app, 2), 6);
    assert_eq!(order_of(&app, 4), 7);
    assert_eq!(order_of(&app, 5), 8);
    assert_eq!(order_of(&app, 1), 0, "non-eligible rows do not move");
    assert_eq!(order_of(&app, 3), 2);
    assert_eq!(order_of(&app, 6), 5);
    assert_ne!(revision(&app), before_revision);
    assert_eq!(undo_depth(&app), 1, "one transaction / one undo step");

    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(order_of(&app, 2), 1, "undo restores the exact prior order");
    assert_eq!(order_of(&app, 4), 3);
    assert_eq!(order_of(&app, 5), 4);
}

#[test]
fn non_eligible_and_non_addressable_refs_are_skipped_not_failed() {
    let (mut app, mut session) = text_app(vec![
        (1, line(point(0.0, 0.0), point(1.0, 0.0)), 0),
        (2, text(2), 1),
        // Not selected: it only supplies the non-eligible baseline (5), so the
        // text is not already in front and must move.
        (3, line(point(0.0, 5.0), point(1.0, 5.0)), 5),
    ]);
    let mut refs = vec![ref_of(1), ref_of(2)];
    // A ref for an entity that does not exist is reported and skipped too.
    refs.push(ref_of(999));
    session.selection = SelectionSet::from_refs(refs);

    let outcome = run_text_front(&mut app, &mut session).expect("TEXTTOFRONT still succeeds");
    assert_eq!(
        order_of(&app, 2),
        6,
        "the text moved above the highest non-eligible row"
    );
    let skipped: Vec<_> = outcome
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "texttofront.skipped")
        .collect();
    assert_eq!(skipped.len(), 2, "one line + one missing ref");
    assert!(skipped
        .iter()
        .any(|diagnostic| diagnostic.object == Some(ObjectId(1))));
    assert!(skipped
        .iter()
        .any(|diagnostic| diagnostic.object == Some(ObjectId(999))));
}

#[test]
fn already_front_selection_is_a_no_op_with_no_revision_or_undo() {
    let (mut app, mut session) = text_app(vec![
        (1, line(point(0.0, 0.0), point(1.0, 0.0)), 0),
        (2, text(2), 10),
    ]);
    select(&mut session, &[2]);
    let before_revision = revision(&app);
    let before_depth = undo_depth(&app);

    let outcome = run_text_front(&mut app, &mut session).expect("already front is not an error");
    assert_eq!(order_of(&app, 2), 10);
    assert!(outcome.changes.is_none());
    assert!(outcome.objects.is_empty());
    assert!(outcome
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "texttofront.unchanged"));
    assert_eq!(revision(&app), before_revision);
    assert_eq!(undo_depth(&app), before_depth);
}

#[test]
fn an_empty_selection_is_the_named_refusal() {
    let (mut app, mut session) = text_app(vec![(1, text(1), 0)]);
    let before_revision = revision(&app);
    let error = run_text_front(&mut app, &mut session).expect_err("empty selection refuses");
    assert!(format!("{error:?}").contains("texttofront.selection"));
    assert_eq!(revision(&app), before_revision);
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn viewer_mode_refuses_texttofront_without_writing() {
    let (mut app, _work) = text_app(vec![
        (1, line(point(0.0, 0.0), point(1.0, 0.0)), 0),
        (2, text(2), 1),
    ]);
    let mut viewer = SessionState::new(DocumentId(1), AppMode::Viewer);
    select(&mut viewer, &[2]);
    let before_revision = revision(&app);
    let error = run_text_front(&mut app, &mut viewer).expect_err("Viewer refuses");
    assert_eq!(error, CadError::PermissionDenied);
    assert_eq!(revision(&app), before_revision);
    assert!(!app.can_undo(&DocumentId(1)));
}
