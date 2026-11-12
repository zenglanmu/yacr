//! The 2026-11-07 2D text batch contract tests:
//! `JUSTIFYTEXT`/`TJUST`, `TSCALE`, `TEXTALIGN`, `TCOUNT`, `TCIRCLE`,
//! `TEXTMASK` and `TEXTUNMASK`.
//!
//! Pins the shared discipline — only the documented value changes, every refusal
//! writes nothing (no revision bump, no undo entry), a no-change result is a
//! `*.unchanged` diagnostic with no transaction, and a real change is exactly one
//! transaction / one undo step — plus the declared **estimated** extents and the
//! no-association mask boundary.

use super::*;
use cad_db::{DbEntity, DbObject, DrawingDatabaseBuilder, Layer};

fn p3(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

#[allow(clippy::too_many_arguments)]
fn text_entity(
    id: u128,
    text: &str,
    position: Point3,
    height: f64,
    h_align: TextAlignH,
    v_align: TextAlignV,
    order: i64,
    type_key: &str,
) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: type_key.into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(7),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Text {
            text: text.into(),
            position,
            style: StyleId(0),
            height,
            rotation: 0.0,
            font: Some("arial.ttf".into()),
            h_align,
            v_align,
            wrap_width: None,
        },
        draw_order: order,
    }
}

fn simple_text(id: u128, text: &str, position: Point3, order: i64) -> DbEntity {
    text_entity(
        id,
        text,
        position,
        2.0,
        TextAlignH::Left,
        TextAlignV::Baseline,
        order,
        "AcDbText",
    )
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
            start: point(0.0, 0.0),
            end: point(1.0, 0.0),
        },
        draw_order: id as i64,
    }
}

fn app_with(entities: Vec<DbEntity>) -> (Application, SessionState) {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    for (id, name) in [(LayerId(0), "0"), (LayerId(7), "NOTES")] {
        builder
            .insert_layer(Layer {
                id,
                name: name.into(),
                visible: true,
            })
            .unwrap();
    }
    for entity in entities {
        builder.insert_entity(entity).unwrap();
    }
    let drawing = builder.finish().unwrap();
    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([93u8; 32]),
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

fn execute(
    app: &mut Application,
    session: &mut SessionState,
    id: CommandId,
    payload: CommandPayload,
) -> CadResult<CommandOutcome> {
    app.execute(
        session,
        Command {
            schema_version: 1,
            id,
            document: DocumentId(1),
            viewport: ViewportId(1),
            payload,
        },
    )
}

fn ref_of(entity: u128) -> SelectionRef {
    SelectionRef {
        document: DocumentId(1),
        entity: EntityId(entity),
        instance: InstancePath::default(),
        sub_element: None,
    }
}

fn select(session: &mut SessionState, entities: &[u128]) {
    session
        .selection
        .replace(entities.iter().map(|id| ref_of(*id)).collect::<Vec<_>>());
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

fn entity(app: &Application, id: u128) -> Option<DbEntity> {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(id))
        .cloned()
}

fn entity_count(app: &Application) -> usize {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity_count()
}

fn find_by_type(app: &Application, type_key: &str) -> Option<DbEntity> {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .entities()
        .find(|entity| entity.object.type_key == type_key)
        .cloned()
}

fn text_of(app: &Application, id: u128) -> String {
    match &entity(app, id).expect("entity exists").geometry {
        SemanticGeometry::Text { text, .. } => text.clone(),
        other => panic!("expected Text, got {other:?}"),
    }
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

fn assert_invalid(error: CadError, code: &str) {
    match error {
        CadError::InvalidInput(message) => assert!(
            message.starts_with(code),
            "expected refusal starting with {code}, got {message}"
        ),
        other => panic!("expected InvalidInput {code}, got {other:?}"),
    }
}

fn assert_unsupported(error: CadError, needle: &str) {
    match error {
        CadError::Unsupported(message) => assert!(
            message.contains(needle),
            "expected Unsupported containing {needle}, got {message}"
        ),
        other => panic!("expected Unsupported containing {needle}, got {other:?}"),
    }
}

// --- The shared estimate -----------------------------------------------------

#[test]
fn the_estimate_follows_the_stored_alignment_and_refuses_a_degenerate_height() {
    use crate::app_drawing::text_batch::estimate_text_bounds;

    let left = text_entity(
        1,
        "abcd",
        p3(10.0, 20.0, 0.0),
        2.0,
        TextAlignH::Left,
        TextAlignV::Baseline,
        0,
        "AcDbText",
    );
    let bounds = estimate_text_bounds(&left, "test").expect("finite");
    assert!(
        near(bounds.min_x, 10.0) && near(bounds.max_x, 18.0),
        "width = height × chars"
    );
    assert!(
        near(bounds.min_y, 20.0) && near(bounds.max_y, 22.0),
        "height = glyph height"
    );

    let centred = text_entity(
        2,
        "abcd",
        p3(10.0, 20.0, 0.0),
        2.0,
        TextAlignH::Center,
        TextAlignV::Top,
        0,
        "AcDbText",
    );
    let bounds = estimate_text_bounds(&centred, "test").expect("finite");
    assert!(
        near(bounds.min_x, 6.0) && near(bounds.max_x, 14.0),
        "centre anchor"
    );
    assert!(
        near(bounds.min_y, 18.0) && near(bounds.max_y, 20.0),
        "top anchor"
    );

    let degenerate = text_entity(
        3,
        "x",
        p3(0.0, 0.0, 0.0),
        0.0,
        TextAlignH::Left,
        TextAlignV::Baseline,
        0,
        "AcDbText",
    );
    let error = estimate_text_bounds(&degenerate, "tc").expect_err("zero height is refused");
    assert_invalid(error, "tc.degenerate");
}

// --- JUSTIFYTEXT -------------------------------------------------------------

#[test]
fn justifytext_changes_only_alignment_and_undo_restores_it() {
    let source = text_entity(
        1,
        "hello",
        p3(3.0, 4.0, 0.0),
        2.0,
        TextAlignH::Center,
        TextAlignV::Middle,
        4,
        "AcDbText",
    );
    let before = source.clone();
    let (mut app, mut session) = app_with(vec![source]);
    select(&mut session, &[1]);
    let before_revision = revision(&app);

    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::JustifyText,
        CommandPayload::JustifyText(JustifyTextCommand {
            h_align: TextAlignH::Left,
            v_align: TextAlignV::Baseline,
        }),
    )
    .expect("JUSTIFYTEXT");
    assert_eq!(outcome.objects, vec![ObjectId(1)]);

    let after = entity(&app, 1).unwrap();
    let SemanticGeometry::Text {
        text,
        position,
        height,
        rotation,
        h_align,
        v_align,
        ..
    } = &after.geometry
    else {
        panic!("expected Text");
    };
    assert_eq!(*h_align, TextAlignH::Left);
    assert_eq!(*v_align, TextAlignV::Baseline);
    // Position and every other field are untouched.
    let SemanticGeometry::Text {
        text: before_text,
        position: before_position,
        height: before_height,
        rotation: before_rotation,
        ..
    } = &before.geometry
    else {
        panic!("expected Text");
    };
    assert_eq!(text, before_text);
    assert_eq!(
        position, before_position,
        "JUSTIFYTEXT never moves the position"
    );
    assert_eq!(height, before_height);
    assert_eq!(rotation, before_rotation);
    assert_eq!(after.layer, LayerId(7));
    assert_eq!(after.draw_order, 4);
    assert_eq!(revision(&app).0, before_revision.0 + 1);
    assert_eq!(undo_depth(&app), 1);

    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .expect("undo");
    let SemanticGeometry::Text {
        h_align, v_align, ..
    } = &entity(&app, 1).unwrap().geometry
    else {
        panic!("expected Text");
    };
    assert_eq!(*h_align, TextAlignH::Center, "undo restores the alignment");
    assert_eq!(*v_align, TextAlignV::Middle);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn justifytext_skips_non_text_targets_and_an_empty_selection_is_refused() {
    let (mut app, mut session) = app_with(vec![
        simple_text(1, "a", p3(0.0, 0.0, 0.0), 0),
        line_entity(2),
    ]);
    select(&mut session, &[1, 2]);
    let before_revision = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::JustifyText,
        CommandPayload::JustifyText(JustifyTextCommand {
            h_align: TextAlignH::Right,
            v_align: TextAlignV::Top,
        }),
    )
    .expect("a mixed selection is a per-entity skip");
    assert!(
        outcome
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "justifytext.not_text"),
        "the LINE is reported, not silently rewritten"
    );
    assert_eq!(
        revision(&app).0,
        before_revision.0 + 1,
        "the text still changed"
    );

    select(&mut session, &[]);
    let before_revision = revision(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::JustifyText,
        CommandPayload::JustifyText(JustifyTextCommand {
            h_align: TextAlignH::Left,
            v_align: TextAlignV::Bottom,
        }),
    )
    .expect_err("an empty selection is refused");
    assert_invalid(error, "justifytext.needs_selection");
    assert_eq!(revision(&app), before_revision);
}

// --- TSCALE ------------------------------------------------------------------

#[test]
fn tscale_uses_the_scaletext_rule_and_is_one_undo_step() {
    let tscale_source = text_entity(
        1,
        "hello",
        p3(3.0, 4.0, 0.0),
        2.5,
        TextAlignH::Center,
        TextAlignV::Middle,
        1,
        "AcDbMText",
    );
    let mut mtext = tscale_source.clone();
    if let SemanticGeometry::Text { wrap_width, .. } = &mut mtext.geometry {
        *wrap_width = Some(10.0);
    }
    let mut scaletext_source = mtext.clone();
    scaletext_source.id = EntityId(1);
    scaletext_source.object.id = ObjectId(1);

    // The SCALETEXT twin, driven by the session factor.
    let (mut reference_app, mut reference_session) = app_with(vec![scaletext_source.clone()]);
    reference_session
        .set_tool_parameter(ToolParameter::ScaleTextFactor, ToolValue::Number(2.0))
        .expect("factor 2");
    execute(
        &mut reference_app,
        &mut reference_session,
        CommandId::ScaleText,
        CommandPayload::ScaleText {
            refs: vec![ref_of(1)],
        },
    )
    .expect("SCALETEXT");

    // The TSCALE command, driven by the typed factor.
    let (mut app, mut session) = app_with(vec![mtext]);
    select(&mut session, &[1]);
    let before_revision = revision(&app);
    execute(
        &mut app,
        &mut session,
        CommandId::TextScale,
        CommandPayload::TextScale(TextScaleCommand { factor: 2.0 }),
    )
    .expect("TSCALE");

    assert_eq!(
        entity(&app, 1).unwrap().geometry,
        entity(&reference_app, 1).unwrap().geometry,
        "TSCALE and SCALETEXT share the one scaled_text rule"
    );
    let SemanticGeometry::Text {
        height,
        wrap_width,
        position,
        ..
    } = &entity(&app, 1).unwrap().geometry
    else {
        panic!("expected Text");
    };
    assert!(near(*height, 5.0), "2.5 × 2 = 5");
    assert_eq!(*wrap_width, Some(20.0), "the MTEXT wrap width scales too");
    assert_eq!(*position, p3(3.0, 4.0, 0.0), "position is untouched");
    assert_eq!(revision(&app).0, before_revision.0 + 1);
    assert_eq!(undo_depth(&app), 1);

    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .expect("undo");
    let SemanticGeometry::Text { height, .. } = &entity(&app, 1).unwrap().geometry else {
        panic!("expected Text");
    };
    assert!(near(*height, 2.5));
}

#[test]
fn tscale_refuses_a_non_positive_factor_and_a_mixed_selection() {
    let (mut app, mut session) = app_with(vec![
        simple_text(1, "a", p3(0.0, 0.0, 0.0), 0),
        line_entity(2),
    ]);
    select(&mut session, &[1]);
    let before_revision = revision(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::TextScale,
        CommandPayload::TextScale(TextScaleCommand { factor: 0.0 }),
    )
    .expect_err("zero is refused");
    assert_invalid(error, "tscale.invalid_factor");
    assert_eq!(revision(&app), before_revision);

    select(&mut session, &[1, 2]);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::TextScale,
        CommandPayload::TextScale(TextScaleCommand { factor: 2.0 }),
    )
    .expect_err("a mixed selection is refused whole");
    assert_unsupported(error, "Text and MText only");
    assert_eq!(undo_depth(&app), 0);
}

// --- TEXTALIGN ---------------------------------------------------------------

#[test]
fn textalign_moves_to_the_common_left_edge_and_undo_restores_the_positions() {
    let (mut app, mut session) = app_with(vec![
        simple_text(1, "ab", p3(0.0, 0.0, 0.0), 0),
        simple_text(2, "abcd", p3(10.0, 5.0, 0.0), 1),
    ]);
    select(&mut session, &[1, 2]);
    let before_revision = revision(&app);

    execute(
        &mut app,
        &mut session,
        CommandId::TextAlignEdges,
        CommandPayload::TextAlignEdges(TextAlignCommand {
            edge: TextAlignEdge::Left,
        }),
    )
    .expect("TEXTALIGN LEFT");

    let first = entity(&app, 1).unwrap();
    let second = entity(&app, 2).unwrap();
    let SemanticGeometry::Text { position: a, .. } = &first.geometry else {
        panic!("expected Text");
    };
    let SemanticGeometry::Text { position: b, .. } = &second.geometry else {
        panic!("expected Text");
    };
    assert!(near(a.x, 0.0), "the left-most text does not move");
    assert!(
        near(b.x, 0.0),
        "the second text moves to the common left edge"
    );
    assert!(near(b.y, 5.0), "the other axis is untouched");
    assert_eq!(revision(&app).0, before_revision.0 + 1);
    assert_eq!(undo_depth(&app), 1);

    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .expect("undo");
    let SemanticGeometry::Text { position, .. } = &entity(&app, 2).unwrap().geometry else {
        panic!("expected Text");
    };
    assert!(near(position.x, 10.0), "undo restores the position");
}

#[test]
fn textalign_refuses_fewer_than_two_targets() {
    let (mut app, mut session) = app_with(vec![simple_text(1, "a", p3(0.0, 0.0, 0.0), 0)]);
    select(&mut session, &[1]);
    let before_revision = revision(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::TextAlignEdges,
        CommandPayload::TextAlignEdges(TextAlignCommand {
            edge: TextAlignEdge::Top,
        }),
    )
    .expect_err("one target cannot align");
    assert_invalid(error, "textalign.needs_two");
    assert_eq!(revision(&app), before_revision);
    assert_eq!(undo_depth(&app), 0);
}

// --- TCOUNT ------------------------------------------------------------------

#[test]
fn tcount_numbers_in_reading_order_with_prefix_and_suffix() {
    let (mut app, mut session) = app_with(vec![
        simple_text(1, "x", p3(5.0, 10.0, 0.0), 0),
        simple_text(2, "x", p3(1.0, 10.0, 0.0), 1),
        simple_text(3, "x", p3(0.0, 0.0, 0.0), 2),
    ]);
    select(&mut session, &[1, 2, 3]);
    let before_revision = revision(&app);

    execute(
        &mut app,
        &mut session,
        CommandId::TextCount,
        CommandPayload::TextCount(TCountCommand {
            start: 10,
            step: 5,
            prefix: "A".into(),
            suffix: "-".into(),
        }),
    )
    .expect("TCOUNT");

    // Top-to-bottom, then left-to-right: (1,10) is #1, (5,10) is #2, (0,0) is #3.
    assert_eq!(text_of(&app, 2), "A10-");
    assert_eq!(text_of(&app, 1), "A15-");
    assert_eq!(text_of(&app, 3), "A20-");
    assert_eq!(revision(&app).0, before_revision.0 + 1);
    assert_eq!(undo_depth(&app), 1);

    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .expect("undo");
    assert_eq!(text_of(&app, 1), "x");
    assert_eq!(text_of(&app, 2), "x");
    assert_eq!(text_of(&app, 3), "x");
}

#[test]
fn tcount_refuses_a_non_positive_step() {
    let (mut app, mut session) = app_with(vec![simple_text(1, "x", p3(0.0, 0.0, 0.0), 0)]);
    select(&mut session, &[1]);
    let before_revision = revision(&app);
    for step in [0, -1] {
        let error = execute(
            &mut app,
            &mut session,
            CommandId::TextCount,
            CommandPayload::TextCount(TCountCommand {
                start: 1,
                step,
                prefix: String::new(),
                suffix: String::new(),
            }),
        )
        .expect_err("a non-positive step is refused");
        assert_invalid(error, "tcount.invalid_step");
    }
    assert_eq!(revision(&app), before_revision);
    assert_eq!(undo_depth(&app), 0);
}

// --- TCIRCLE -----------------------------------------------------------------

#[test]
fn tcircle_draws_a_circle_on_the_text_layer_and_keeps_the_text() {
    let (mut app, mut session) = app_with(vec![simple_text(1, "abcd", p3(0.0, 0.0, 0.0), 5)]);
    select(&mut session, &[1]);
    let before_count = entity_count(&app);

    execute(
        &mut app,
        &mut session,
        CommandId::TextCircle,
        CommandPayload::TextCircle(TCircleCommand {
            shape: TextBoundaryShape::Circle,
        }),
    )
    .expect("TCIRCLE CIRCLE");

    assert_eq!(entity_count(&app), before_count + 1);
    let circle = find_by_type(&app, "AcDbCircle").expect("a circle boundary");
    assert_eq!(circle.layer, LayerId(7), "the text's layer");
    let SemanticGeometry::Circle {
        center,
        radius,
        normal,
    } = &circle.geometry
    else {
        panic!("expected a Circle");
    };
    // Estimated bounds (0,0)-(8,2): centre (4,1), radius half the diagonal.
    assert!(near(center.x, 4.0) && near(center.y, 1.0));
    assert!(near(*radius, 0.5 * 8.0_f64.hypot(2.0)));
    assert!(near(normal.z, 1.0));
    assert!(entity(&app, 1).is_some(), "the text is retained");
    assert_eq!(undo_depth(&app), 1);

    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .expect("undo");
    assert!(find_by_type(&app, "AcDbCircle").is_none());
    assert_eq!(entity_count(&app), before_count);
}

#[test]
fn tcircle_rectangle_is_a_closed_four_vertex_polyline() {
    let (mut app, mut session) = app_with(vec![simple_text(1, "abcd", p3(0.0, 0.0, 0.0), 0)]);
    select(&mut session, &[1]);
    execute(
        &mut app,
        &mut session,
        CommandId::TextCircle,
        CommandPayload::TextCircle(TCircleCommand {
            shape: TextBoundaryShape::Rectangle,
        }),
    )
    .expect("TCIRCLE RECTANGLE");
    let boundary = find_by_type(&app, "AcDbPolyline").expect("a polyline boundary");
    let SemanticGeometry::Polyline {
        points,
        bulges,
        closed,
    } = &boundary.geometry
    else {
        panic!("expected a Polyline");
    };
    assert_eq!(points.len(), 4);
    assert_eq!(bulges.len(), 4);
    assert!(*closed);
    assert!(near(points[2].x, 8.0) && near(points[2].y, 2.0));
}

// --- TEXTMASK / TEXTUNMASK ---------------------------------------------------

#[test]
fn textmask_inserts_an_independent_wipeout_behind_the_text() {
    let (mut app, mut session) = app_with(vec![simple_text(1, "abcd", p3(0.0, 0.0, 0.0), 5)]);
    select(&mut session, &[1]);
    let before_count = entity_count(&app);

    execute(
        &mut app,
        &mut session,
        CommandId::TextMask,
        CommandPayload::None,
    )
    .expect("TEXTMASK");

    assert_eq!(entity_count(&app), before_count + 1);
    let mask = find_by_type(&app, "AcDbWipeout").expect("the AcDbWipeout mask");
    assert_eq!(mask.layer, LayerId(7), "the text's layer");
    assert_eq!(mask.draw_order, 4, "just below the text's draw order");
    let SemanticGeometry::Mask { boundary, inverted } = &mask.geometry else {
        panic!("expected a Mask");
    };
    assert_eq!(boundary.len(), 4, "the estimated bounds rectangle");
    assert!(!*inverted);
    // Height 2.0, offset 0.35 -> 0.7 world units on every side.
    assert_eq!(boundary[0], p3(-0.7, -0.7, 0.0));
    assert_eq!(boundary[1], p3(8.7, -0.7, 0.0));
    assert_eq!(boundary[2], p3(8.7, 2.7, 0.0));
    assert_eq!(boundary[3], p3(-0.7, 2.7, 0.0));
    assert!(entity(&app, 1).is_some(), "the text is retained");
    assert_eq!(session.groups.len(), 1, "text and mask are grouped");
    let group = session.groups.groups().next().unwrap();
    assert!(group.name.starts_with("*TEXTMASK-"));
    assert_eq!(group.members.len(), 2);
    assert_eq!(group.members[0].entity, EntityId(1));
    assert_eq!(group.members[1].entity, mask.id);
    assert_eq!(undo_depth(&app), 1);
}

#[test]
fn textunmask_deletes_the_selected_mask_inside_the_text_bounds() {
    let (mut app, mut session) = app_with(vec![simple_text(1, "abcd", p3(0.0, 0.0, 0.0), 5)]);
    select(&mut session, &[1]);
    execute(
        &mut app,
        &mut session,
        CommandId::TextMask,
        CommandPayload::None,
    )
    .expect("TEXTMASK");
    let mask = find_by_type(&app, "AcDbWipeout").expect("mask");
    select(&mut session, &[1, mask.id.0]);

    execute(
        &mut app,
        &mut session,
        CommandId::TextUnmask,
        CommandPayload::None,
    )
    .expect("TEXTUNMASK");

    assert!(
        find_by_type(&app, "AcDbWipeout").is_none(),
        "the mask is deleted"
    );
    assert!(entity(&app, 1).is_some(), "the text is kept");
    assert_eq!(undo_depth(&app), 2, "mask create + unmask");

    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .expect("undo");
    assert!(
        find_by_type(&app, "AcDbWipeout").is_some(),
        "undo restores the mask"
    );
}

#[test]
fn textunmask_with_only_masks_selected_deletes_them() {
    let (mut app, mut session) = app_with(vec![simple_text(1, "abcd", p3(0.0, 0.0, 0.0), 5)]);
    select(&mut session, &[1]);
    execute(
        &mut app,
        &mut session,
        CommandId::TextMask,
        CommandPayload::None,
    )
    .expect("TEXTMASK");
    let mask = find_by_type(&app, "AcDbWipeout").expect("mask");
    select(&mut session, &[mask.id.0]);

    execute(
        &mut app,
        &mut session,
        CommandId::TextUnmask,
        CommandPayload::None,
    )
    .expect("TEXTUNMASK with only a mask selected");
    assert!(find_by_type(&app, "AcDbWipeout").is_none());
}

#[test]
fn textunmask_with_only_text_selected_removes_its_grouped_mask() {
    let (mut app, mut session) = app_with(vec![simple_text(1, "abcd", p3(0.0, 0.0, 0.0), 5)]);
    select(&mut session, &[1]);
    execute(
        &mut app,
        &mut session,
        CommandId::TextMask,
        CommandPayload::None,
    )
    .expect("TEXTMASK");
    assert_eq!(session.groups.len(), 1);
    let depth = undo_depth(&app);
    // The group names the exact mask, so selecting only the text removes it.
    execute(
        &mut app,
        &mut session,
        CommandId::TextUnmask,
        CommandPayload::None,
    )
    .expect("TEXTUNMASK follows the group");
    assert!(
        find_by_type(&app, "AcDbWipeout").is_none(),
        "the grouped mask is removed without scanning unrelated masks"
    );
    assert!(
        session.groups.is_empty(),
        "the TEXTMASK group is cleaned up"
    );
    assert_eq!(undo_depth(&app), depth + 1);
}

fn textmask_payload(options: crate::command_payload::TextMaskOptions) -> CommandPayload {
    CommandPayload::Annotation(Box::new(AnnotationBatchPayload::Utility(Box::new(
        crate::command_payload::UtilityPayload::TextMask(options),
    ))))
}

#[test]
fn textmask_offset_factor_and_mask_types_are_explicit() {
    let (mut app, mut session) = app_with(vec![simple_text(1, "abcd", p3(0.0, 0.0, 0.0), 5)]);
    select(&mut session, &[1]);
    execute(
        &mut app,
        &mut session,
        CommandId::TextMask,
        textmask_payload(crate::command_payload::TextMaskOptions {
            offset_factor: 1.0,
            kind: crate::command_payload::TextMaskKind::Wipeout,
        }),
    )
    .expect("a custom offset factor");
    let mask = find_by_type(&app, "AcDbWipeout").expect("mask");
    let SemanticGeometry::Mask { boundary, .. } = &mask.geometry else {
        panic!("expected a Mask");
    };
    assert_eq!(boundary[0], p3(-2.0, -2.0, 0.0));
    assert_eq!(boundary[2], p3(10.0, 4.0, 0.0));

    let before = revision(&app);
    for kind in [
        crate::command_payload::TextMaskKind::Face3d,
        crate::command_payload::TextMaskKind::Solid,
    ] {
        let error = execute(
            &mut app,
            &mut session,
            CommandId::TextMask,
            textmask_payload(crate::command_payload::TextMaskOptions {
                offset_factor: 0.35,
                kind,
            }),
        )
        .expect_err("unmodelled mask types refuse");
        assert!(
            matches!(error, CadError::Unsupported(ref message) if message.contains("textmask.mask_type")),
            "{error:?}"
        );
        assert_eq!(revision(&app), before, "a refusal writes nothing");
    }

    let error = execute(
        &mut app,
        &mut session,
        CommandId::TextMask,
        textmask_payload(crate::command_payload::TextMaskOptions {
            offset_factor: -0.1,
            kind: crate::command_payload::TextMaskKind::Wipeout,
        }),
    )
    .expect_err("a negative offset refuses");
    assert!(
        matches!(error, CadError::InvalidInput(ref message) if message.contains("textmask.invalid_offset")),
        "{error:?}"
    );
}

#[test]
fn textmask_rerun_replaces_the_previous_mask_and_does_not_duplicate_groups() {
    let (mut app, mut session) = app_with(vec![simple_text(1, "abcd", p3(0.0, 0.0, 0.0), 5)]);
    select(&mut session, &[1]);
    execute(
        &mut app,
        &mut session,
        CommandId::TextMask,
        CommandPayload::None,
    )
    .expect("first TEXTMASK");
    let first_mask = find_by_type(&app, "AcDbWipeout").expect("first mask").id;
    assert_eq!(session.groups.len(), 1);
    execute(
        &mut app,
        &mut session,
        CommandId::TextMask,
        CommandPayload::None,
    )
    .expect("second TEXTMASK");
    assert!(
        entity(&app, first_mask.0).is_none(),
        "the previous mask is erased"
    );
    let masks: Vec<_> = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entities()
        .filter(|entity| matches!(&entity.geometry, SemanticGeometry::Mask { .. }))
        .collect();
    assert_eq!(masks.len(), 1, "exactly one mask remains");
    assert_eq!(
        session.groups.len(),
        1,
        "the group is replaced, not duplicated"
    );
    assert_eq!(undo_depth(&app), 2, "each TEXTMASK is one undo step");
}

// --- Catalogue facts ---------------------------------------------------------

#[test]
fn the_batch_is_work_only_and_has_no_second_path() {
    for id in [
        CommandId::JustifyText,
        CommandId::TextScale,
        CommandId::TextAlignEdges,
        CommandId::TextCount,
        CommandId::TextCircle,
        CommandId::TextMask,
        CommandId::TextUnmask,
    ] {
        assert!(id.requires_work_mode(), "{id:?} writes the drawing");
    }
}

// TEXTALIGN's combined modes / DISTRIBUTE and the TEXTMASK/TEXTUNMASK mask
// contracts, split out so every file in this batch stays inside the code-size
// limit.
mod align;
mod mask;
