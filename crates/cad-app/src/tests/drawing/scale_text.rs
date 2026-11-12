//! `SCALETEXT` edit-command contract tests.
//!
//! Pins the glyph-height scaling of Text/MText (with the MTEXT wrap width), the
//! in-place update of the **same** entity (identity / layer / space / draw order /
//! type key / every non-height field preserved), the one-transaction / one-undo
//! shape of a multi-selection, and every named refusal (nothing written, no
//! revision bump, no undo entry). A mixed selection is a whole-command refusal.

use super::explode::part;
use super::*;
use cad_db::{DrawingDatabaseBuilder, Layer};

fn text_geometry(height: f64, wrap_width: Option<f64>) -> SemanticGeometry {
    SemanticGeometry::Text {
        text: "hello".into(),
        position: p3(3.0, 4.0, 0.0),
        style: StyleId(0),
        height,
        rotation: 0.25,
        font: Some("arial.ttf".into()),
        h_align: TextAlignH::Center,
        v_align: TextAlignV::Middle,
        wrap_width,
        width_factor: 1.0,
    }
}

fn text_entity(id: u128, height: f64, wrap_width: Option<f64>, type_key: &str) -> DbEntity {
    part(
        id,
        text_geometry(height, wrap_width),
        LayerId(7),
        SpaceId::Model,
        type_key,
        id as i64,
    )
}

fn line_entity(id: u128) -> DbEntity {
    part(
        id,
        SemanticGeometry::Line {
            start: point(0.0, 0.0),
            end: point(1.0, 0.0),
        },
        LayerId(0),
        SpaceId::Model,
        "AcDbLine",
        id as i64,
    )
}

/// A work-mode app with layers 0/WALLS and the given model entities.
fn app_with(entities: Vec<DbEntity>) -> (Application, SessionState) {
    build(entities, AppMode::Work)
}

fn build(entities: Vec<DbEntity>, mode: AppMode) -> (Application, SessionState) {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    for (id, name) in [(LayerId(0), "0"), (LayerId(7), "WALLS")] {
        b.insert_layer(Layer {
            id,
            name: name.into(),
            visible: true,
        })
        .unwrap();
    }
    for entity in entities {
        b.insert_entity(entity).unwrap();
    }
    let drawing = b.finish().unwrap();
    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([61u8; 32]),
            units: UnitContext::drawing_units(),
            resource_keys: Vec::new(),
        },
    );
    app.workspace.viewports.insert(
        ViewportId(1),
        Viewport::new(ViewportId(1), DocumentId(1), [800.0, 600.0]),
    );
    (app, SessionState::new(DocumentId(1), mode))
}

fn undo_depth(app: &Application) -> usize {
    app.history
        .get(&DocumentId(1))
        .map(|history| history.undo_depth())
        .unwrap_or(0)
}

fn entity(app: &Application, id: u128) -> DbEntity {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(id))
        .cloned()
        .unwrap_or_else(|| panic!("entity {id} should exist"))
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

fn set_factor(session: &mut SessionState, factor: f64) {
    session
        .set_tool_parameter(ToolParameter::ScaleTextFactor, ToolValue::Number(factor))
        .unwrap_or_else(|error| panic!("scaleTextFactor {factor} refused: {error}"));
}

fn scale_text(
    app: &mut Application,
    session: &mut SessionState,
    refs: Vec<SelectionRef>,
) -> CadResult<CommandOutcome> {
    execute(
        app,
        session,
        CommandId::ScaleText,
        CommandPayload::ScaleText { refs },
    )
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

/// The stored (height, wrap width) pair of a Text/MTEXT entity.
fn stored_text(app: &Application, id: u128) -> (f64, Option<f64>) {
    match &entity(app, id).geometry {
        SemanticGeometry::Text {
            height, wrap_width, ..
        } => (*height, *wrap_width),
        other => panic!("expected a Text geometry, got {other:?}"),
    }
}

#[test]
fn text_height_scales_and_every_other_field_is_preserved() {
    let source = text_entity(1, 2.5, None, "AcDbText");
    let before = source.clone();
    let (mut app, mut session) = app_with(vec![source]);
    set_factor(&mut session, 2.0);
    let before_revision = revision(&app);

    let outcome = scale_text(&mut app, &mut session, vec![ref_of(1)]).expect("2.5 × 2 = 5");
    assert_eq!(
        outcome.objects,
        vec![ObjectId(1)],
        "the SAME entity is updated"
    );
    assert_eq!(outcome.diagnostics.len(), 1);
    assert_eq!(outcome.diagnostics[0].code, "drawing.scaletext");
    assert!(outcome.diagnostics[0].message.contains("factor 2"));

    let after = entity(&app, 1);
    let (height, wrap_width) = stored_text(&app, 1);
    assert!(near(height, 5.0), "2.5 × 2 = 5, got {height}");
    assert_eq!(wrap_width, None, "no wrap boundary stays none");
    // Everything except the height is bit-for-bit preserved.
    let SemanticGeometry::Text {
        text,
        position,
        style,
        rotation,
        font,
        h_align,
        v_align,
        ..
    } = &after.geometry
    else {
        panic!("expected a Text geometry");
    };
    let SemanticGeometry::Text {
        text: before_text,
        position: before_position,
        style: before_style,
        rotation: before_rotation,
        font: before_font,
        h_align: before_h,
        v_align: before_v,
        ..
    } = &before.geometry
    else {
        panic!("expected a Text geometry");
    };
    assert_eq!(text, before_text);
    assert_eq!(position, before_position, "position is untouched");
    assert_eq!(style, before_style, "style is untouched");
    assert_eq!(rotation, before_rotation, "rotation is untouched");
    assert_eq!(font, before_font);
    assert_eq!(h_align, before_h);
    assert_eq!(v_align, before_v);

    // Identity, layer, space, draw order and type key are preserved.
    assert_eq!(after.id, EntityId(1));
    assert_eq!(after.layer, LayerId(7));
    assert_eq!(after.space, SpaceId::Model);
    assert_eq!(after.draw_order, 1);
    assert_eq!(after.object.type_key, "AcDbText");
    assert_eq!(revision(&app).0, before_revision.0 + 1, "one transaction");
    assert_eq!(undo_depth(&app), 1, "exactly one undo step");
}

#[test]
fn mtext_scales_height_and_wrap_width_and_none_stays_none() {
    let wrapped = text_entity(1, 3.0, Some(10.0), "AcDbMText");
    let unwrapped = text_entity(2, 4.0, None, "AcDbMText");
    let (mut app, mut session) = app_with(vec![wrapped, unwrapped]);
    set_factor(&mut session, 2.0);

    scale_text(&mut app, &mut session, vec![ref_of(1), ref_of(2)])
        .expect("both MTEXT entities scale");

    assert_eq!(stored_text(&app, 1), (6.0, Some(20.0)));
    assert_eq!(
        stored_text(&app, 2),
        (8.0, None),
        "a `None` wrap boundary stays `None`"
    );
    assert_eq!(entity(&app, 1).object.type_key, "AcDbMText");
    assert_eq!(undo_depth(&app), 1);
}

#[test]
fn multiple_targets_share_one_transaction_and_undo_restores_all() {
    let (mut app, mut session) = app_with(vec![
        text_entity(1, 2.0, None, "AcDbText"),
        text_entity(2, 4.0, Some(8.0), "AcDbMText"),
        text_entity(3, 6.0, None, "AcDbMText"),
    ]);
    set_factor(&mut session, 1.5);
    let before_revision = revision(&app);

    let outcome = scale_text(
        &mut app,
        &mut session,
        vec![ref_of(1), ref_of(2), ref_of(3)],
    )
    .expect("all three texts scale");
    assert_eq!(outcome.objects, vec![ObjectId(1), ObjectId(2), ObjectId(3)]);
    assert_eq!(stored_text(&app, 1), (3.0, None));
    assert_eq!(stored_text(&app, 2), (6.0, Some(12.0)));
    assert_eq!(stored_text(&app, 3), (9.0, None));
    assert_eq!(revision(&app).0, before_revision.0 + 1, "one transaction");
    assert_eq!(undo_depth(&app), 1, "one undo step for the whole selection");

    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .expect("the SCALETEXT is undoable");
    assert_eq!(stored_text(&app, 1), (2.0, None));
    assert_eq!(stored_text(&app, 2), (4.0, Some(8.0)));
    assert_eq!(stored_text(&app, 3), (6.0, None));
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn a_mixed_selection_refuses_the_whole_command() {
    let (mut app, mut session) =
        app_with(vec![text_entity(1, 2.5, None, "AcDbText"), line_entity(2)]);
    set_factor(&mut session, 2.0);
    let before_revision = revision(&app);

    let error = scale_text(&mut app, &mut session, vec![ref_of(1), ref_of(2)])
        .expect_err("a mixed Text + LINE selection is refused");
    assert_unsupported(error, "Text and MText only");
    assert_unsupported(
        scale_text(&mut app, &mut session, vec![ref_of(1), ref_of(2)])
            .expect_err("the refusal repeats"),
        "LINE is not supported",
    );

    assert_eq!(revision(&app), before_revision, "no revision bump");
    assert_eq!(undo_depth(&app), 0, "no transaction");
    assert_eq!(
        stored_text(&app, 1),
        (2.5, None),
        "the text is not partially written"
    );
}

#[test]
fn an_empty_selection_is_refused() {
    let (mut app, mut session) = app_with(vec![text_entity(1, 2.5, None, "AcDbText")]);
    set_factor(&mut session, 2.0);
    let before_revision = revision(&app);

    let error =
        scale_text(&mut app, &mut session, Vec::new()).expect_err("an empty selection is refused");
    assert_invalid(error, "scaletext.needs_selection");
    assert_eq!(revision(&app), before_revision, "no revision bump");
    assert_eq!(undo_depth(&app), 0, "no transaction");
    assert_eq!(stored_text(&app, 1), (2.5, None));
}

#[test]
fn block_member_and_sub_element_refs_are_refused() {
    let (mut app, mut session) = app_with(vec![text_entity(1, 2.5, None, "AcDbText")]);
    set_factor(&mut session, 2.0);

    let instance_ref = SelectionRef {
        document: DocumentId(1),
        entity: EntityId(1),
        instance: InstancePath(vec![EntityId(9)]),
        sub_element: None,
    };
    let error = scale_text(&mut app, &mut session, vec![instance_ref])
        .expect_err("a block-instance member is refused");
    assert_invalid(error, "scaletext.not_addressable");

    let sub_element_ref = SelectionRef {
        document: DocumentId(1),
        entity: EntityId(1),
        instance: InstancePath::default(),
        sub_element: Some(SubElementId {
            source_key: "face".into(),
            topology_revision: Revision(0),
        }),
    };
    let error = scale_text(&mut app, &mut session, vec![sub_element_ref])
        .expect_err("a sub-element ref is refused");
    assert_invalid(error, "scaletext.not_addressable");

    assert_eq!(undo_depth(&app), 0, "no transaction");
    assert_eq!(stored_text(&app, 1), (2.5, None));
}

#[test]
fn a_missing_target_is_refused() {
    let (mut app, mut session) = app_with(vec![text_entity(1, 2.5, None, "AcDbText")]);
    set_factor(&mut session, 2.0);
    let before_revision = revision(&app);

    let error = scale_text(&mut app, &mut session, vec![ref_of(999)])
        .expect_err("a missing target is refused");
    assert_invalid(error, "scaletext.target_missing");
    assert_eq!(revision(&app), before_revision);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn invalid_factor_values_are_refused_by_name() {
    for (label, factor) in [
        ("zero", 0.0),
        ("negative", -2.0),
        ("NaN", f64::NAN),
        ("infinity", f64::INFINITY),
    ] {
        let (mut app, mut session) = app_with(vec![text_entity(1, 2.5, None, "AcDbText")]);
        // Injected past the channel, exactly like the other tool-parameter
        // defenses: the command must refuse it by name.
        session.force_tool_parameter(ToolParameter::ScaleTextFactor, ToolValue::Number(factor));
        let before_revision = revision(&app);
        let error = scale_text(&mut app, &mut session, vec![ref_of(1)])
            .expect_err("an invalid factor is refused");
        assert_invalid(error, "scaletext.invalid_factor");
        assert_eq!(revision(&app), before_revision, "{label}: no revision bump");
        assert_eq!(undo_depth(&app), 0, "{label}: no transaction");
        assert_eq!(
            stored_text(&app, 1),
            (2.5, None),
            "{label}: nothing written"
        );
    }
}

#[test]
fn the_channel_refuses_a_non_positive_or_non_finite_factor() {
    let mut session = SessionState::new(DocumentId(1), AppMode::Work);
    for factor in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert!(
            session
                .set_tool_parameter(ToolParameter::ScaleTextFactor, ToolValue::Number(factor))
                .is_err(),
            "{factor} must be refused by the channel"
        );
    }
    assert_eq!(
        session.tool_parameter(ToolParameter::ScaleTextFactor),
        ToolValue::Number(1.0),
        "a refused set leaves the default intact"
    );
}

#[test]
fn factor_one_is_a_named_no_op_with_no_transaction() {
    let (mut app, mut session) = app_with(vec![
        text_entity(1, 2.5, None, "AcDbText"),
        text_entity(2, 4.0, Some(8.0), "AcDbMText"),
    ]);
    // The documented default is 1.0; do not call `set_factor`.
    let before_revision = revision(&app);

    let outcome = scale_text(&mut app, &mut session, vec![ref_of(1), ref_of(2)])
        .expect("a factor of 1.0 is a successful no-op");
    assert!(outcome.objects.is_empty(), "nothing was updated");
    assert!(outcome.changes.is_none(), "no ChangeSet");
    assert_eq!(outcome.diagnostics.len(), 1);
    assert_eq!(outcome.diagnostics[0].code, "scaletext.unchanged");
    assert_eq!(revision(&app), before_revision, "no revision bump");
    assert_eq!(undo_depth(&app), 0, "no empty undo entry");
    assert_eq!(stored_text(&app, 1), (2.5, None));
    assert_eq!(stored_text(&app, 2), (4.0, Some(8.0)));
}

#[test]
fn a_degenerate_stored_height_is_refused() {
    // The builder stores raw geometry, so this pins the command's own defense
    // against a height the validated write path would never produce.
    let degenerate = text_entity(1, 0.0, None, "AcDbText");
    let (mut app, mut session) = app_with(vec![degenerate]);
    set_factor(&mut session, 2.0);
    let before_revision = revision(&app);

    let error = scale_text(&mut app, &mut session, vec![ref_of(1)])
        .expect_err("a degenerate stored height is refused");
    assert_invalid(error, "scaletext.degenerate");
    assert_eq!(revision(&app), before_revision, "no revision bump");
    assert_eq!(undo_depth(&app), 0, "no transaction");
}

#[test]
fn an_oversized_result_is_refused() {
    let (mut app, mut session) = app_with(vec![text_entity(1, 2.5, Some(1.0), "AcDbMText")]);
    set_factor(&mut session, 1.0e13);
    let before_revision = revision(&app);

    let error = scale_text(&mut app, &mut session, vec![ref_of(1)])
        .expect_err("an oversized result is refused");
    assert_invalid(error, "scaletext.invalid_result");
    assert_eq!(revision(&app), before_revision);
    assert_eq!(undo_depth(&app), 0);
    assert_eq!(stored_text(&app, 1), (2.5, Some(1.0)));
}

#[test]
fn a_viewer_session_is_refused() {
    let (mut app, mut session) =
        build(vec![text_entity(1, 2.5, None, "AcDbText")], AppMode::Viewer);
    let before_revision = revision(&app);

    let error = scale_text(&mut app, &mut session, vec![ref_of(1)])
        .expect_err("Viewer mode cannot scale text");
    assert!(
        matches!(error, CadError::PermissionDenied),
        "expected PermissionDenied, got {error:?}"
    );
    assert_eq!(revision(&app), before_revision, "no revision bump");
    assert_eq!(undo_depth(&app), 0);
    assert_eq!(stored_text(&app, 1), (2.5, None));
}
