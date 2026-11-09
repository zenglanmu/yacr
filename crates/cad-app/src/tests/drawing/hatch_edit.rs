//! HATCHEDIT v1 contract tests (AutoCAD 2026).
//!
//! The typed option subset is `ANGLE <degrees>`, `SCALE <factor>`, `SOLID` and
//! `GRADIENT` against exactly one directly addressable model-space hatch. These
//! tests read the **stored** fill back (pattern angle/scale, fill variant), pin
//! the preserved entity fields and the one-transaction/one-undo shape, and check
//! every refusal writes nothing.

use super::hatch::{
    circle, entity_count, hatch_app, run, set_flag, set_number, square, the_hatch, undo_depth,
};
use super::*;

use crate::HatchEditOption;
use cad_domain::{HatchFill, HatchGradient, HatchPatternType};

fn select(session: &mut SessionState, ids: &[u128]) {
    session
        .selection
        .replace(ids.iter().copied().map(ref_of).collect::<Vec<_>>());
}

fn edit(
    app: &mut Application,
    session: &mut SessionState,
    entity: u128,
    option: HatchEditOption,
) -> CadResult<CommandOutcome> {
    select(session, &[entity]);
    execute(
        app,
        session,
        CommandId::HatchEdit,
        CommandPayload::HatchEdit(option),
    )
}

/// Create a user-defined pattern hatch (spacing 2, angle 0, no double) on the
/// closed square entity 1 and return its object id.
fn pattern_hatch(app: &mut Application, session: &mut SessionState) -> u128 {
    set_number(
        session,
        ToolParameter::HatchPatternType,
        HATCH_PATTERN_TYPE_USER_DEFINED,
    );
    set_number(session, ToolParameter::HatchSpacing, 2.0);
    set_number(session, ToolParameter::HatchAngle, 0.0);
    set_flag(session, ToolParameter::HatchDouble, false);
    let out = run(app, session, &[1]).expect("HATCH succeeds");
    out.objects[0].0
}

fn pattern(hatch: &HatchGeometry) -> &HatchPattern {
    let HatchFill::Pattern(pattern) = hatch.fill() else {
        panic!("expected a pattern fill, got {:?}", hatch.fill());
    };
    pattern
}

#[test]
fn angle_edit_rebuilds_the_pattern_and_preserves_the_entity() {
    let (mut app, mut session) = hatch_app(vec![square(1, 0.0, 10.0, 0.0)]);
    let id = pattern_hatch(&mut app, &mut session);
    let before = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(id))
        .cloned()
        .unwrap();
    let before_hatch = before.geometry.clone();

    let out = edit(&mut app, &mut session, id, HatchEditOption::Angle(45.0))
        .expect("HATCHEDIT ANGLE succeeds");
    assert_eq!(out.objects, vec![ObjectId(id)]);
    assert_eq!(
        undo_depth(&app),
        2,
        "one transaction / one undo step on top of the HATCH"
    );
    assert_eq!(entity_count(&app), 2, "no entity was inserted or removed");

    let after = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(id))
        .cloned()
        .unwrap();
    assert_eq!(after.layer, before.layer);
    assert_eq!(after.space, before.space);
    assert_eq!(after.draw_order, before.draw_order);
    assert_eq!(after.object.type_key, "AcDbHatch");

    let SemanticGeometry::Hatch(after_hatch) = &after.geometry else {
        panic!("HATCHEDIT must keep the hatch geometry");
    };
    let SemanticGeometry::Hatch(before_hatch) = &before_hatch else {
        unreachable!()
    };
    assert_eq!(after_hatch.loops(), before_hatch.loops(), "loops preserved");
    assert_eq!(after_hatch.plane(), before_hatch.plane(), "plane preserved");
    assert_eq!(after_hatch.style(), before_hatch.style(), "style preserved");
    assert_eq!(after_hatch.seeds(), before_hatch.seeds(), "seeds preserved");
    assert_eq!(after_hatch.pixel_size(), before_hatch.pixel_size());
    assert_eq!(after_hatch.pattern_type(), HatchPatternType::UserDefined);
    let stored = pattern(after_hatch);
    assert!(
        (stored.angle - 45.0f64.to_radians()).abs() < 1e-12,
        "{}",
        stored.angle
    );
    assert_eq!(stored.scale, 2.0, "scale is untouched");
    assert_eq!(
        stored.lines,
        pattern(before_hatch).lines,
        "definition rows preserved"
    );
    assert!(!stored.double);
    assert!(
        !after_hatch.display().is_empty(),
        "the display is rebuilt by the domain constructor"
    );

    // Undo restores the exact stored angle.
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    let restored = the_hatch(&app);
    assert!((pattern(restored).angle - 0.0).abs() < 1e-12);
}

#[test]
fn scale_edit_rebuilds_the_pattern_and_keeps_the_angle() {
    let (mut app, mut session) = hatch_app(vec![square(1, 0.0, 10.0, 0.0)]);
    let id = pattern_hatch(&mut app, &mut session);
    edit(&mut app, &mut session, id, HatchEditOption::Angle(30.0)).unwrap();

    let out = edit(&mut app, &mut session, id, HatchEditOption::Scale(3.5))
        .expect("HATCHEDIT SCALE succeeds");
    assert_eq!(out.objects, vec![ObjectId(id)]);
    let hatch = the_hatch(&app);
    let stored = pattern(hatch);
    assert_eq!(stored.scale, 3.5);
    assert!((stored.angle - 30.0f64.to_radians()).abs() < 1e-12);
    assert_eq!(stored.name, "USER");
    assert_eq!(hatch.pattern_type(), HatchPatternType::UserDefined);
}

#[test]
fn solid_to_gradient_and_gradient_to_solid_convert_the_fill() {
    let (mut app, mut session) = hatch_app(vec![square(1, 0.0, 10.0, 0.0)]);
    // Default HATCH pattern type is solid.
    let out = run(&mut app, &mut session, &[1]).expect("HATCH solid succeeds");
    let id = out.objects[0].0;
    assert!(matches!(the_hatch(&app).fill(), HatchFill::Solid));

    edit(&mut app, &mut session, id, HatchEditOption::Gradient).expect("SOLID -> GRADIENT");
    let hatch = the_hatch(&app);
    let HatchFill::Gradient(HatchGradient::TwoColor {
        name,
        angle_raw,
        stops,
        ..
    }) = hatch.fill()
    else {
        panic!("expected a two-colour gradient, got {:?}", hatch.fill());
    };
    assert_eq!(name, "LINEAR");
    assert_eq!(*angle_raw, 0.0);
    assert_eq!(stops[0].rgb, [255, 0, 0], "default stop A is ACI 1");
    assert_eq!(stops[1].rgb, [0, 0, 255], "default stop B is ACI 5");
    assert_eq!(hatch.pattern_type(), HatchPatternType::Predefined);

    edit(&mut app, &mut session, id, HatchEditOption::Solid).expect("GRADIENT -> SOLID");
    let hatch = the_hatch(&app);
    assert!(matches!(hatch.fill(), HatchFill::Solid));
    assert_eq!(hatch.pattern_type(), HatchPatternType::Predefined);
    assert_eq!(hatch.loops().len(), 1, "the boundary is preserved");
}

#[test]
fn a_gradient_hatch_can_be_turned_solid_and_back_to_the_default_gradient() {
    let (mut app, mut session) = hatch_app(vec![circle(1, 0.0, 0.0, 0.0, 5.0)]);
    set_number(
        &mut session,
        ToolParameter::HatchPatternType,
        HATCH_PATTERN_TYPE_GRADIENT,
    );
    set_number(
        &mut session,
        ToolParameter::HatchGradientKind,
        HATCH_GRADIENT_SPHERICAL,
    );
    let out = run(&mut app, &mut session, &[1]).expect("HATCH gradient succeeds");
    let id = out.objects[0].0;

    edit(&mut app, &mut session, id, HatchEditOption::Solid).expect("GRADIENT -> SOLID");
    assert!(matches!(the_hatch(&app).fill(), HatchFill::Solid));

    edit(&mut app, &mut session, id, HatchEditOption::Gradient).expect("SOLID -> GRADIENT");
    let HatchFill::Gradient(HatchGradient::TwoColor { name, .. }) = the_hatch(&app).fill() else {
        panic!("expected the default two-colour gradient");
    };
    // HATCHEDIT GRADIENT writes the *default* linear ramp, not the spherical
    // session value the source hatch happened to carry.
    assert_eq!(name, "LINEAR");
    assert_eq!(the_hatch(&app).loops()[0].points.len(), 4);
}

#[test]
fn unchanged_values_are_named_no_ops_without_a_transaction() {
    let (mut app, mut session) = hatch_app(vec![square(1, 0.0, 10.0, 0.0)]);
    let id = pattern_hatch(&mut app, &mut session);
    let revision_before = revision(&app);
    let undo_before = undo_depth(&app);

    for option in [HatchEditOption::Angle(0.0), HatchEditOption::Scale(2.0)] {
        let out =
            edit(&mut app, &mut session, id, option.clone()).expect("a no-op is not an error");
        assert!(out.objects.is_empty(), "{option:?}");
        assert!(out.changes.is_none(), "{option:?}");
        assert_eq!(out.diagnostics[0].code, "hatchedit.unchanged", "{option:?}");
    }
    assert_eq!(revision(&app), revision_before);
    assert_eq!(undo_depth(&app), undo_before);

    // The same fill variant is a no-op too, for both variants.
    set_number(
        &mut session,
        ToolParameter::HatchPatternType,
        HATCH_PATTERN_TYPE_SOLID,
    );
    let out = run(&mut app, &mut session, &[1]).expect("HATCH solid");
    let solid = out.objects[0].0;
    let out = edit(&mut app, &mut session, solid, HatchEditOption::Solid).unwrap();
    assert_eq!(out.diagnostics[0].code, "hatchedit.unchanged");
    edit(&mut app, &mut session, solid, HatchEditOption::Gradient).unwrap();
    let out = edit(&mut app, &mut session, solid, HatchEditOption::Gradient).unwrap();
    assert_eq!(out.diagnostics[0].code, "hatchedit.unchanged");
    assert_eq!(
        undo_depth(&app),
        undo_before + 2,
        "only the HATCH and the one real edit bumped"
    );
}

#[test]
fn bare_and_unknown_edits_are_refused_writing_nothing() {
    let (mut app, mut session) = hatch_app(vec![square(1, 0.0, 10.0, 0.0)]);
    let id = pattern_hatch(&mut app, &mut session);
    select(&mut session, &[id]);
    let revision_before = revision(&app);
    let undo_before = undo_depth(&app);

    let error = execute(
        &mut app,
        &mut session,
        CommandId::HatchEdit,
        CommandPayload::None,
    )
    .expect_err("bare HATCHEDIT must refuse");
    let text = format!("{error:?}");
    assert!(text.contains("hatchedit.needs_option"), "{text}");
    assert!(text.contains("ANGLE"), "{text}");
    assert!(text.contains("GRADIENT"), "{text}");
    assert_eq!(revision(&app), revision_before);
    assert_eq!(undo_depth(&app), undo_before);
}

#[test]
fn selection_and_addressability_refusals_write_nothing() {
    let (mut app, mut session) = hatch_app(vec![square(1, 0.0, 10.0, 0.0)]);
    let revision_before = revision(&app);

    let error = edit(&mut app, &mut session, 1, HatchEditOption::Solid)
        .expect_err("a non-hatch must refuse");
    assert!(
        format!("{error:?}").contains("hatchedit.not_hatch"),
        "{error:?}"
    );
    assert_eq!(revision(&app), revision_before);
    assert_eq!(undo_depth(&app), 0, "no transaction was opened");

    let id = run(&mut app, &mut session, &[1]).expect("HATCH").objects[0].0;
    let revision_after_hatch = revision(&app);

    session.selection.replace(Vec::new());
    let error = execute(
        &mut app,
        &mut session,
        CommandId::HatchEdit,
        CommandPayload::HatchEdit(HatchEditOption::Solid),
    )
    .expect_err("empty selection must refuse");
    assert!(
        format!("{error:?}").contains("hatchedit.selection"),
        "{error:?}"
    );

    session.selection.replace(vec![SelectionRef {
        document: DocumentId(1),
        entity: EntityId(id),
        instance: InstancePath(vec![EntityId(1)]),
        sub_element: None,
    }]);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::HatchEdit,
        CommandPayload::HatchEdit(HatchEditOption::Solid),
    )
    .expect_err("a block-instance member must refuse");
    assert!(
        format!("{error:?}").contains("hatchedit.not_addressable"),
        "{error:?}"
    );

    assert_eq!(
        revision(&app),
        revision_after_hatch,
        "refusals wrote nothing"
    );
    assert_eq!(undo_depth(&app), 1, "only the HATCH transaction exists");
}

#[test]
fn angle_on_a_solid_fill_and_bad_scale_are_refused() {
    let (mut app, mut session) = hatch_app(vec![square(1, 0.0, 10.0, 0.0)]);
    let id = run(&mut app, &mut session, &[1])
        .expect("solid HATCH")
        .objects[0]
        .0;

    let error = edit(&mut app, &mut session, id, HatchEditOption::Angle(45.0))
        .expect_err("ANGLE on a solid fill must refuse");
    assert!(
        format!("{error:?}").contains("hatchedit.not_pattern"),
        "{error:?}"
    );

    let error = edit(&mut app, &mut session, id, HatchEditOption::Scale(2.0))
        .expect_err("SCALE on a solid fill must refuse");
    assert!(
        format!("{error:?}").contains("hatchedit.not_pattern"),
        "{error:?}"
    );

    // A pattern fill is required before the scale value itself is examined.
    set_number(
        &mut session,
        ToolParameter::HatchPatternType,
        HATCH_PATTERN_TYPE_USER_DEFINED,
    );
    set_number(&mut session, ToolParameter::HatchSpacing, 1.0);
    let id = run(&mut app, &mut session, &[1])
        .expect("pattern HATCH")
        .objects[0]
        .0;
    for factor in [0.0, -1.0, f64::INFINITY, f64::NAN] {
        let error = edit(&mut app, &mut session, id, HatchEditOption::Scale(factor))
            .expect_err("an invalid scale must refuse");
        assert!(
            format!("{error:?}").contains("hatchedit.scale"),
            "{factor}: {error:?}"
        );
    }
    let error = edit(&mut app, &mut session, id, HatchEditOption::Angle(f64::NAN))
        .expect_err("a non-finite angle must refuse");
    assert!(
        format!("{error:?}").contains("hatchedit.angle"),
        "{error:?}"
    );

    assert_eq!(undo_depth(&app), 2, "only the two HATCH transactions exist");
}

#[test]
fn viewer_mode_refuses_hatchedit_and_writes_nothing() {
    let (mut app, mut session) = hatch_app(vec![square(1, 0.0, 10.0, 0.0)]);
    let id = pattern_hatch(&mut app, &mut session);
    session.switch_mode(AppMode::Viewer).unwrap();
    select(&mut session, &[id]);
    let revision_before = revision(&app);
    let undo_before = undo_depth(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::HatchEdit,
        CommandPayload::HatchEdit(HatchEditOption::Angle(45.0)),
    )
    .expect_err("Viewer must refuse");
    assert!(matches!(error, CadError::PermissionDenied), "{error:?}");
    assert_eq!(revision(&app), revision_before);
    assert_eq!(undo_depth(&app), undo_before);
}

/// The stored display is derived, not a stale copy: after an edit the hatch's
/// display geometry is non-empty and the boundary loop count is unchanged.
#[test]
fn the_rebuilt_display_is_fresh_not_a_stale_copy() {
    let (mut app, mut session) = hatch_app(vec![square(1, 0.0, 10.0, 0.0)]);
    let id = pattern_hatch(&mut app, &mut session);
    edit(&mut app, &mut session, id, HatchEditOption::Angle(60.0)).unwrap();
    let hatch = the_hatch(&app);
    assert_eq!(hatch.loops().len(), 1);
    assert!(
        hatch
            .display()
            .iter()
            .any(|geometry| matches!(geometry, SemanticGeometry::Polyline { closed: false, .. })),
        "a user-defined pattern has display polylines"
    );
}

// ---------------------------------------------------------------------------
// 2026-11 AutoCAD gap-closing options: style, pattern type, colour,
// transparency, layer, draw order, boundary recreation and separation.
// ---------------------------------------------------------------------------

use crate::HatchDrawOrder;
use cad_db::EntityColor;
use cad_domain::HatchStyle;

fn stored_hatch(app: &Application, id: u128) -> HatchGeometry {
    let entity = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(id))
        .expect("the hatch entity");
    match &entity.geometry {
        SemanticGeometry::Hatch(hatch) => (**hatch).clone(),
        other => panic!("expected a hatch, got {other:?}"),
    }
}

fn stored_layer(app: &Application, id: u128) -> LayerId {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(id))
        .expect("the entity")
        .layer
}

fn stored_order(app: &Application, id: u128) -> i64 {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(id))
        .expect("the entity")
        .draw_order
}

/// A drawing with layer 0 and a second layer "Walls" (id 7).
fn layered_hatch_app() -> (Application, SessionState) {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    for (id, name) in [(LayerId(0), "0"), (LayerId(7), "Walls")] {
        b.insert_layer(Layer {
            id,
            name: name.into(),
            visible: true,
        })
        .unwrap();
    }
    b.insert_entity(ent(1, square_geometry(0.0, 10.0), 0))
        .unwrap();
    let drawing = b.finish().unwrap();
    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([4u8; 32]),
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

fn square_geometry(min: f64, max: f64) -> SemanticGeometry {
    SemanticGeometry::Polyline {
        points: vec![
            p3(min, min, 0.0),
            p3(max, min, 0.0),
            p3(max, max, 0.0),
            p3(min, max, 0.0),
        ],
        bulges: vec![0.0; 4],
        closed: true,
    }
}

#[test]
fn hatchedit_style_and_pattern_type_edits_change_the_stored_properties() {
    let (mut app, mut session) = hatch_app(vec![square(1, 0.0, 10.0, 0.0)]);
    let id = pattern_hatch(&mut app, &mut session);

    let before = the_hatch(&app).style();
    let out = edit(
        &mut app,
        &mut session,
        id,
        HatchEditOption::Style(HatchStyle::Outer),
    )
    .expect("STYLE OUTER");
    assert_eq!(out.objects, vec![ObjectId(id)]);
    assert_eq!(the_hatch(&app).style(), HatchStyle::Outer);
    assert_ne!(before, HatchStyle::Outer);
    let undo_before = undo_depth(&app);
    let out = edit(
        &mut app,
        &mut session,
        id,
        HatchEditOption::Style(HatchStyle::Outer),
    )
    .unwrap();
    assert_eq!(out.diagnostics[0].code, "hatchedit.unchanged");
    assert_eq!(undo_depth(&app), undo_before);

    assert_eq!(
        the_hatch(&app).pattern_type(),
        HatchPatternType::UserDefined
    );
    edit(
        &mut app,
        &mut session,
        id,
        HatchEditOption::PatternType(HatchPatternType::Predefined),
    )
    .expect("PATTERNTYPE PREDEFINED");
    assert_eq!(the_hatch(&app).pattern_type(), HatchPatternType::Predefined);
    assert_eq!(
        pattern(the_hatch(&app)).scale,
        2.0,
        "the fill itself is preserved"
    );
}

#[test]
fn hatchedit_color_and_transparency_edits_write_the_entity_display_attributes() {
    let (mut app, mut session) = hatch_app(vec![square(1, 0.0, 10.0, 0.0)]);
    let id = pattern_hatch(&mut app, &mut session);
    fn attributes(app: &Application, id: u128) -> cad_db::EntityRenderAttributes {
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity_render_attributes(EntityId(id))
    }
    assert_eq!(attributes(&app, id).color, EntityColor::ByLayer);

    edit(&mut app, &mut session, id, HatchEditOption::Color(30)).expect("COLOR 30");
    assert_eq!(
        attributes(&app, id).color,
        EntityColor::Explicit(
            crate::property_value::rgb_for_aci(30).expect("ACI 30 is a real colour")
        )
    );
    edit(&mut app, &mut session, id, HatchEditOption::ColorByLayer).expect("COLOR BYLAYER");
    assert_eq!(attributes(&app, id).color, EntityColor::ByLayer);

    edit(
        &mut app,
        &mut session,
        id,
        HatchEditOption::Transparency(50),
    )
    .expect("TRANSPARENCY 50");
    assert_eq!(
        attributes(&app, id).transparency,
        cad_db::EntityTransparency::Explicit(0.5)
    );
    edit(
        &mut app,
        &mut session,
        id,
        HatchEditOption::TransparencyByLayer,
    )
    .expect("TRANSPARENCY BYLAYER");
    assert_eq!(
        attributes(&app, id).transparency,
        cad_db::EntityTransparency::ByLayer
    );

    // An ACI index outside 1..=255 (a hand-built payload) is a named refusal.
    let undo_before = undo_depth(&app);
    let error =
        edit(&mut app, &mut session, id, HatchEditOption::Color(0)).expect_err("ACI 0 is symbolic");
    assert!(
        format!("{error:?}").contains("hatchedit.color"),
        "{error:?}"
    );
    let error = edit(
        &mut app,
        &mut session,
        id,
        HatchEditOption::Transparency(91),
    )
    .expect_err("91% is out of the entity bound");
    assert!(
        format!("{error:?}").contains("hatchedit.transparency"),
        "{error:?}"
    );
    assert_eq!(undo_depth(&app), undo_before);
}

#[test]
fn hatchedit_layer_edits_move_every_target_and_refuse_an_unknown_layer() {
    let (mut app, mut session) = layered_hatch_app();
    let id = run(&mut app, &mut session, &[1]).expect("HATCH").objects[0].0;
    assert_eq!(stored_layer(&app, id), LayerId(0));

    edit(
        &mut app,
        &mut session,
        id,
        HatchEditOption::Layer("Walls".into()),
    )
    .expect("LAYER Walls");
    assert_eq!(stored_layer(&app, id), LayerId(7));
    let out = edit(
        &mut app,
        &mut session,
        id,
        HatchEditOption::Layer("walls".into()),
    )
    .unwrap();
    assert_eq!(
        out.diagnostics[0].code, "hatchedit.unchanged",
        "a case-insensitive match is the same layer"
    );
    let undo_before = undo_depth(&app);
    let error = edit(
        &mut app,
        &mut session,
        id,
        HatchEditOption::Layer("Missing".into()),
    )
    .expect_err("unknown layer");
    assert!(
        format!("{error:?}").contains("hatchedit.unknown_layer"),
        "{error:?}"
    );
    assert_eq!(undo_depth(&app), undo_before);
    assert_eq!(stored_layer(&app, id), LayerId(7));
}

#[test]
fn hatchedit_draw_order_moves_the_selected_hatch_behind_and_in_front() {
    let (mut app, mut session) =
        hatch_app(vec![square(1, 0.0, 10.0, 0.0), square(2, 20.0, 30.0, 0.0)]);
    let id = run(&mut app, &mut session, &[1]).expect("HATCH").objects[0].0;
    let other_order = stored_order(&app, 2);
    assert!(
        stored_order(&app, id) > other_order,
        "the hatch starts on top"
    );

    // FRONT is already satisfied: named no-op, no transaction.
    let undo_before = undo_depth(&app);
    let out = edit(
        &mut app,
        &mut session,
        id,
        HatchEditOption::DrawOrder(HatchDrawOrder::InFrontOfAll),
    )
    .unwrap();
    assert_eq!(out.diagnostics[0].code, "hatchedit.unchanged");
    assert_eq!(undo_depth(&app), undo_before);

    edit(
        &mut app,
        &mut session,
        id,
        HatchEditOption::DrawOrder(HatchDrawOrder::BehindAll),
    )
    .expect("DRAWORDER BEHIND");
    assert!(stored_order(&app, id) < other_order);

    edit(
        &mut app,
        &mut session,
        id,
        HatchEditOption::DrawOrder(HatchDrawOrder::InFrontOfAll),
    )
    .expect("DRAWORDER FRONT");
    assert!(stored_order(&app, id) > other_order);

    let error = edit(
        &mut app,
        &mut session,
        id,
        HatchEditOption::DrawOrder(HatchDrawOrder::BehindBoundary),
    )
    .expect_err("no boundary record");
    assert!(
        format!("{error:?}").contains("hatchedit.draw_order_boundary"),
        "{error:?}"
    );
}

#[test]
fn hatchedit_recreate_boundary_inserts_one_closed_polyline_per_loop() {
    let (mut app, mut session) = hatch_app(vec![square(1, 0.0, 10.0, 0.0)]);
    let id = run(&mut app, &mut session, &[1]).expect("HATCH").objects[0].0;
    let before = entity_count(&app);
    let out = edit(
        &mut app,
        &mut session,
        id,
        HatchEditOption::RecreateBoundary,
    )
    .expect("RECREATEBOUNDARY");
    assert_eq!(entity_count(&app), before + 1);
    assert_eq!(out.objects.len(), 1);
    let created = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(out.objects[0].0))
        .expect("the boundary polyline");
    assert_eq!(created.layer, LayerId(0));
    match &created.geometry {
        SemanticGeometry::Polyline {
            points,
            bulges,
            closed,
        } => {
            assert!(*closed);
            assert_eq!(points.len(), 4);
            assert_eq!(bulges.len(), 4);
            assert_eq!(points[0], p3(0.0, 0.0, 0.0));
        }
        other => panic!("expected a polyline, got {other:?}"),
    }
}

#[test]
fn hatchedit_separate_hatches_splits_a_multi_loop_hatch_into_one_hatch_per_loop() {
    let (mut app, mut session) =
        hatch_app(vec![square(1, 0.0, 10.0, 0.0), square(2, 20.0, 30.0, 0.0)]);
    let id = run(&mut app, &mut session, &[1, 2])
        .expect("HATCH from two boundaries")
        .objects[0]
        .0;
    assert_eq!(stored_hatch(&app, id).loops().len(), 2);
    let before = entity_count(&app);
    let out = edit(&mut app, &mut session, id, HatchEditOption::SeparateHatches).expect("SEPARATE");
    assert_eq!(entity_count(&app), before + 1);
    assert_eq!(stored_hatch(&app, id).loops().len(), 1);
    assert_eq!(out.objects.len(), 2, "the original and the new hatch");
    let created = out
        .objects
        .iter()
        .find(|object| object.0 != id)
        .expect("the split-off hatch");
    assert_eq!(stored_hatch(&app, created.0).loops().len(), 1);
    let undo_before = undo_depth(&app);
    let out = edit(&mut app, &mut session, id, HatchEditOption::SeparateHatches).unwrap();
    assert_eq!(
        out.diagnostics[0].code, "hatchedit.unchanged",
        "a one-loop hatch cannot be separated"
    );
    assert_eq!(undo_depth(&app), undo_before);
}

#[test]
fn hatchedit_one_run_edits_every_selected_hatch_in_one_undo() {
    let (mut app, mut session) =
        hatch_app(vec![square(1, 0.0, 10.0, 0.0), square(2, 20.0, 30.0, 0.0)]);
    let first = pattern_hatch(&mut app, &mut session);
    // A second pattern hatch from the other square.
    set_number(&mut session, ToolParameter::HatchSpacing, 1.0);
    let second = run(&mut app, &mut session, &[2]).expect("HATCH 2").objects[0].0;
    assert_ne!(first, second);
    let undo_before = undo_depth(&app);

    select(&mut session, &[first, second]);
    let out = execute(
        &mut app,
        &mut session,
        CommandId::HatchEdit,
        CommandPayload::HatchEdit(HatchEditOption::Angle(45.0)),
    )
    .expect("multi-target HATCHEDIT");
    assert_eq!(out.objects.len(), 2);
    assert_eq!(
        undo_depth(&app),
        undo_before + 1,
        "one transaction / one undo"
    );
    for id in [first, second] {
        assert!(
            (pattern(&stored_hatch(&app, id)).angle - 45.0f64.to_radians()).abs() < 1e-12,
            "entity {id} was edited"
        );
    }
}

#[test]
fn hatchedit_global_capability_refusals_are_named_and_write_nothing() {
    let (mut app, mut session) = hatch_app(vec![square(1, 0.0, 10.0, 0.0)]);
    let id = run(&mut app, &mut session, &[1]).expect("HATCH").objects[0].0;
    select(&mut session, &[id]);
    let revision_before = revision(&app);
    let undo_before = undo_depth(&app);
    for (option, code) in [
        (HatchEditOption::Origin, "hatchedit.missing_field"),
        (HatchEditOption::Associate, "hatchedit.no_association"),
        (HatchEditOption::Disassociate, "hatchedit.no_association"),
        (
            HatchEditOption::Annotative,
            "hatchedit.annotative_unsupported",
        ),
    ] {
        let error = execute(
            &mut app,
            &mut session,
            CommandId::HatchEdit,
            CommandPayload::HatchEdit(option),
        )
        .expect_err("the missing model must refuse");
        assert!(format!("{error:?}").contains(code), "{code}: {error:?}");
    }
    assert_eq!(revision(&app), revision_before);
    assert_eq!(undo_depth(&app), undo_before);
}
