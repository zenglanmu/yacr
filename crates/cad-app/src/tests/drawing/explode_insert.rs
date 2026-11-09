//! EXPLODE INSERT/nested-block contract tests (ADR 0016 decision G).
//!
//! Split out of `explode.rs` to keep that file under the file-size limit. These
//! cover the one-level INSERT decomposition, the placement bake, the nested and
//! self-referential block references, and the refusal set for INSERTs.

use super::explode::*;
use super::*;

/// A drawing with block 1 (one LINE member on layer 7) and a model INSERT (id 1)
/// with `transform`, on layer 0.
fn insert_app(transform: Transform3) -> (Application, SessionState) {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    for (id, name) in [(LayerId(0), "0"), (LayerId(7), "MEMBERS")] {
        b.insert_layer(Layer {
            id,
            name: name.into(),
            visible: true,
        })
        .unwrap();
    }
    b.insert_block(BlockDefinition {
        id: cad_domain::BlockId(1),
        entities: vec![EntityId(100)],
        dynamic_visibility: None,
    })
    .unwrap();
    b.insert_entity(part(
        100,
        line(0.0, 0.0, 1.0, 0.0),
        LayerId(7),
        SpaceId::Block(cad_domain::BlockId(1)),
        "AcDbLine",
        0,
    ))
    .unwrap();
    b.insert_entity(part(
        1,
        insert(1, transform),
        LayerId(0),
        SpaceId::Model,
        "AcDbBlockReference",
        1,
    ))
    .unwrap();
    app_from(b.finish().unwrap())
}
#[test]
fn explode_refuses_a_minsert_compound_by_name() {
    // A MINSERT imports as `Compound(Insert…)`; AutoCAD cannot explode a
    // MINSERT, so this is a named refusal (nothing written), not a fan-out into
    // block references.
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_block(BlockDefinition {
        id: cad_domain::BlockId(1),
        entities: vec![EntityId(100)],
        dynamic_visibility: None,
    })
    .unwrap();
    b.insert_entity(part(
        100,
        line(0.0, 0.0, 1.0, 0.0),
        LayerId(0),
        SpaceId::Block(cad_domain::BlockId(1)),
        "AcDbLine",
        0,
    ))
    .unwrap();
    let minsert = SemanticGeometry::Compound(vec![
        insert(1, Transform3::translation(p3(10.0, 0.0, 0.0))),
        insert(1, Transform3::translation(p3(20.0, 0.0, 0.0))),
    ]);
    b.insert_entity(part(
        1,
        minsert,
        LayerId(0),
        SpaceId::Model,
        "AcDbBlockReference",
        1,
    ))
    .unwrap();
    let (mut app, mut session) = app_from(b.finish().unwrap());
    let before = revision(&app);
    select_one(&mut app, &mut session, 1);
    let error = explode(&mut app, &mut session).expect_err("a MINSERT is refused");
    match error {
        CadError::Unsupported(reason) => assert!(
            reason.contains("MINSERT"),
            "the refusal must name the MINSERT: {reason}"
        ),
        other => panic!("expected an Unsupported MINSERT refusal, got {other:?}"),
    }
    assert_eq!(revision(&app), before);
    assert_eq!(entity_count(&app), 2);
    assert_eq!(undo_depth(&app), 0);
}
#[test]
fn explode_uniformly_scaled_insert_bakes_the_placement_and_keeps_member_layers() {
    // The member LINE is (0,0)-(1,0) on block layer 7; the INSERT scales it by 2
    // and translates by (10,0), so the exploded LINE is (10,0)-(12,0) on layer 7.
    let transform = Transform3::translation(p3(10.0, 0.0, 0.0)).matrix_mul(&Transform3::scale(2.0));
    let (mut app, mut session) = insert_app(transform);
    select_one(&mut app, &mut session, 1);
    explode(&mut app, &mut session).unwrap();

    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    let model = drawing.model_space();
    assert_eq!(model.len(), 1, "the INSERT becomes its one member");
    let member = model[0];
    assert_eq!(
        member.layer,
        LayerId(7),
        "members keep their own block-definition layer"
    );
    assert_eq!(member.space, SpaceId::Model);
    match &member.geometry {
        SemanticGeometry::Line { start, end } => {
            assert!(
                near(*start, p3(10.0, 0.0, 0.0)) && near(*end, p3(12.0, 0.0, 0.0)),
                "{start:?} .. {end:?}"
            );
        }
        other => panic!("expected a LINE, got {other:?}"),
    }
}
#[test]
fn explode_non_uniform_insert_places_the_member_exactly() {
    // A non-uniform scale is the deterministic subset: a LINE has an exact
    // affine image, so it is placed instead of refusing the whole INSERT.
    let (mut app, mut session) = insert_app(non_uniform(2.0, 1.0));
    select_one(&mut app, &mut session, 1);
    explode(&mut app, &mut session).unwrap();
    let lines = model_lines(&app);
    assert_eq!(lines.len(), 1);
    assert!(
        near(lines[0].0, p3(0.0, 0.0, 0.0)) && near(lines[0].1, p3(2.0, 0.0, 0.0)),
        "{:?}",
        lines[0]
    );
    assert_eq!(undo_depth(&app), 1);
}

#[test]
fn explode_non_uniform_insert_turns_a_circle_member_into_an_exact_ellipse() {
    // A circle under (x×2, y×1) is the ellipse with major 2·r and ratio 1/2.
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_block(BlockDefinition {
        id: cad_domain::BlockId(1),
        entities: vec![EntityId(100)],
        dynamic_visibility: None,
    })
    .unwrap();
    b.insert_entity(part(
        100,
        SemanticGeometry::Circle {
            center: point(0.0, 0.0),
            normal: p3(0.0, 0.0, 1.0),
            radius: 1.0,
        },
        LayerId(0),
        SpaceId::Block(cad_domain::BlockId(1)),
        "AcDbCircle",
        0,
    ))
    .unwrap();
    b.insert_entity(part(
        1,
        insert(1, non_uniform(2.0, 1.0)),
        LayerId(0),
        SpaceId::Model,
        "AcDbBlockReference",
        1,
    ))
    .unwrap();
    let (mut app, mut session) = app_from(b.finish().unwrap());
    select_one(&mut app, &mut session, 1);
    explode(&mut app, &mut session).unwrap();
    let produced = model_geometry(&app);
    assert_eq!(produced.len(), 1);
    match &produced[0].1 {
        SemanticGeometry::Ellipse {
            major_axis,
            ratio,
            start,
            ..
        } => {
            assert!((cad_geometry::length(*major_axis) - 2.0).abs() < 1e-9);
            assert!((*ratio - 0.5).abs() < 1e-9);
            assert!(start.is_finite());
        }
        other => panic!("expected an ELLIPSE, got {other:?}"),
    }
}

#[test]
fn explode_non_uniform_insert_refuses_an_unrepresentable_member_by_name() {
    // A non-uniformly scaled TEXT has no exact flat-text representation (its
    // glyphs would be sheared), so the whole command refuses and names the
    // member.
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_block(BlockDefinition {
        id: cad_domain::BlockId(1),
        entities: vec![EntityId(101)],
        dynamic_visibility: None,
    })
    .unwrap();
    b.insert_entity(part(
        101,
        text_geometry(),
        LayerId(0),
        SpaceId::Block(cad_domain::BlockId(1)),
        "AcDbText",
        0,
    ))
    .unwrap();
    b.insert_entity(part(
        1,
        insert(1, non_uniform(2.0, 1.0)),
        LayerId(0),
        SpaceId::Model,
        "AcDbBlockReference",
        1,
    ))
    .unwrap();
    let (mut app, mut session) = app_from(b.finish().unwrap());
    let before = revision(&app);
    select_one(&mut app, &mut session, 1);
    let error = explode(&mut app, &mut session).expect_err("a sheared TEXT is refused");
    match error {
        CadError::Unsupported(reason) => {
            assert!(
                reason.contains("101"),
                "the refusal names the member: {reason}"
            );
            assert!(reason.contains("AcDbText"), "{reason}");
        }
        other => panic!("expected an Unsupported refusal, got {other:?}"),
    }
    assert_eq!(revision(&app), before, "nothing is written");
    assert_eq!(entity_count(&app), 2);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn explode_refuses_an_xref_block_reference_by_name() {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_block(BlockDefinition {
        id: cad_domain::BlockId(3),
        entities: Vec::new(),
        dynamic_visibility: None,
    })
    .unwrap();
    b.insert_external_reference(cad_db::ExternalReference {
        name: "XREF-A".into(),
        raw_path: r".\xref\base.dwg".into(),
        resolved_path: Some("/work/xref/base.dwg".into()),
        block: Some(cad_domain::BlockId(3)),
        overlay: false,
        depth: 0,
        status: cad_db::ExternalReferenceStatus::Loaded,
        reason: None,
    })
    .unwrap();
    b.insert_entity(part(
        1,
        insert(3, Transform3::identity()),
        LayerId(0),
        SpaceId::Model,
        "AcDbBlockReference",
        0,
    ))
    .unwrap();
    let (mut app, mut session) = app_from(b.finish().unwrap());
    let before = revision(&app);
    select_one(&mut app, &mut session, 1);
    let error = explode(&mut app, &mut session).expect_err("an XREF is refused");
    match error {
        CadError::Unsupported(reason) => assert!(
            reason.contains("XREF"),
            "the refusal must name the XREF: {reason}"
        ),
        other => panic!("expected an Unsupported XREF refusal, got {other:?}"),
    }
    assert_eq!(revision(&app), before);
    assert_eq!(entity_count(&app), 1);
    assert_eq!(undo_depth(&app), 0);
}
#[test]
fn explode_insert_with_a_text_member_still_fails_the_whole_command() {
    // Block 1 has a LINE member and a TEXT member. ADR 0019 decision B made Text
    // transformable, so the placement bakes fine; the EXPLODE of the member is
    // then refused by its **explicit** kind reason (`child_refusal`) instead of a
    // transform side effect. The behaviour is unchanged: the whole command fails
    // and nothing is written.
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_block(BlockDefinition {
        id: cad_domain::BlockId(1),
        entities: vec![EntityId(100), EntityId(101)],
        dynamic_visibility: None,
    })
    .unwrap();
    b.insert_entity(part(
        100,
        line(0.0, 0.0, 1.0, 0.0),
        LayerId(0),
        SpaceId::Block(cad_domain::BlockId(1)),
        "AcDbLine",
        0,
    ))
    .unwrap();
    b.insert_entity(part(
        101,
        text_geometry(),
        LayerId(0),
        SpaceId::Block(cad_domain::BlockId(1)),
        "AcDbText",
        1,
    ))
    .unwrap();
    b.insert_entity(part(
        1,
        insert(1, Transform3::translation(p3(5.0, 0.0, 0.0))),
        LayerId(0),
        SpaceId::Model,
        "AcDbBlockReference",
        2,
    ))
    .unwrap();
    let (mut app, mut session) = app_from(b.finish().unwrap());
    let before = revision(&app);
    select_one(&mut app, &mut session, 1);
    let error = explode(&mut app, &mut session).expect_err("an undecomposable member fails");
    match error {
        CadError::Unsupported(reason) => {
            assert!(
                reason.contains("EXPLODE does not decompose TEXT"),
                "the TEXT member is refused explicitly: {reason}"
            );
        }
        other => panic!("expected an Unsupported refusal, got {other:?}"),
    }
    assert_eq!(revision(&app), before, "nothing is written");
    assert_eq!(entity_count(&app), 3, "both members and the INSERT remain");
    assert_eq!(undo_depth(&app), 0);
}
#[test]
fn explode_insert_with_a_mirrored_arc_member_fails_the_whole_command() {
    // A mirrored INSERT (negative determinant) cannot bake its placement into an
    // Arc member: the transform surface refuses the reflection, so the whole
    // command fails (the ADR's "mirrored Arc/Ellipse children" refusal).
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_block(BlockDefinition {
        id: cad_domain::BlockId(1),
        entities: vec![EntityId(100)],
        dynamic_visibility: None,
    })
    .unwrap();
    b.insert_entity(part(
        100,
        SemanticGeometry::Arc {
            center: point(0.0, 0.0),
            normal: p3(0.0, 0.0, 1.0),
            radius: 1.0,
            start: 0.0,
            sweep: std::f64::consts::PI,
        },
        LayerId(0),
        SpaceId::Block(cad_domain::BlockId(1)),
        "AcDbArc",
        0,
    ))
    .unwrap();
    b.insert_entity(part(
        1,
        insert(1, non_uniform(-1.0, 1.0)),
        LayerId(0),
        SpaceId::Model,
        "AcDbBlockReference",
        1,
    ))
    .unwrap();
    let (mut app, mut session) = app_from(b.finish().unwrap());
    let before = revision(&app);
    select_one(&mut app, &mut session, 1);
    let error = explode(&mut app, &mut session).expect_err("a mirrored ARC member fails");
    assert!(matches!(error, CadError::Unsupported(_)), "{error:?}");
    assert_eq!(revision(&app), before, "nothing is written");
    assert_eq!(entity_count(&app), 2);
    assert_eq!(undo_depth(&app), 0);
}
/// Block 1's member is an INSERT of block 2 (transform `(1,1,0)`, on layer 7);
/// block 2's member is a LINE `(0,0)-(2,0)`; model entity 1 is an INSERT of
/// block 1 at `(10,0,0)`.
fn nested_block_app() -> (Application, SessionState) {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    for (id, name) in [(LayerId(0), "0"), (LayerId(7), "MEMBERS")] {
        b.insert_layer(Layer {
            id,
            name: name.into(),
            visible: true,
        })
        .unwrap();
    }
    b.insert_block(BlockDefinition {
        id: cad_domain::BlockId(1),
        entities: vec![EntityId(100)],
        dynamic_visibility: None,
    })
    .unwrap();
    b.insert_block(BlockDefinition {
        id: cad_domain::BlockId(2),
        entities: vec![EntityId(101)],
        dynamic_visibility: None,
    })
    .unwrap();
    b.insert_entity(part(
        100,
        insert(2, Transform3::translation(p3(1.0, 1.0, 0.0))),
        LayerId(7),
        SpaceId::Block(cad_domain::BlockId(1)),
        "AcDbBlockReference",
        0,
    ))
    .unwrap();
    b.insert_entity(part(
        101,
        line(0.0, 0.0, 2.0, 0.0),
        LayerId(0),
        SpaceId::Block(cad_domain::BlockId(2)),
        "AcDbLine",
        0,
    ))
    .unwrap();
    b.insert_entity(part(
        1,
        insert(1, Transform3::translation(p3(10.0, 0.0, 0.0))),
        LayerId(0),
        SpaceId::Model,
        "AcDbBlockReference",
        1,
    ))
    .unwrap();
    app_from(b.finish().unwrap())
}
#[test]
fn explode_nested_block_reference_member_stays_a_block_reference() {
    // One level: the outer INSERT yields its direct member, which is itself a
    // block reference and therefore stays one (only the placement is composed
    // in) — its contents are NOT produced in this call.
    let (mut app, mut session) = nested_block_app();
    select_one(&mut app, &mut session, 1);
    explode(&mut app, &mut session).unwrap();

    let produced = model_geometry(&app);
    assert_eq!(produced.len(), 1, "the INSERT yields its one direct member");
    match &produced[0].1 {
        SemanticGeometry::Insert { block, transform } => {
            assert_eq!(
                *block,
                cad_domain::BlockId(2),
                "the member stays a reference to block 2"
            );
            // The placement (10,0) composed with the member's own (1,1): a
            // translation to (11,1), no rotation or scale.
            assert!(
                (transform.matrix[0][3] - 11.0).abs() < 1e-9,
                "{transform:?}"
            );
            assert!((transform.matrix[1][3] - 1.0).abs() < 1e-9, "{transform:?}");
            assert!((transform.matrix[0][0] - 1.0).abs() < 1e-9, "{transform:?}");
            assert!((transform.matrix[1][1] - 1.0).abs() < 1e-9, "{transform:?}");
        }
        other => panic!("the nested member must stay an INSERT, got {other:?}"),
    }
    assert!(
        model_lines(&app).is_empty(),
        "block 2's geometry is not produced until it is exploded again"
    );
    assert_eq!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .model_space()[0]
            .layer,
        LayerId(7),
        "the member keeps its own block-definition layer"
    );
}
#[test]
fn explode_of_the_produced_nested_reference_yields_its_members_on_the_next_explode() {
    // The two-step behaviour: explode the outer block, then explode the produced
    // nested reference to get its members with the composed placement baked in.
    let (mut app, mut session) = nested_block_app();
    select_one(&mut app, &mut session, 1);
    explode(&mut app, &mut session).unwrap();

    let reference_id = model_geometry(&app)[0].0;
    session.selection = SelectionSet::from_refs(vec![SelectionRef {
        document: DocumentId(1),
        entity: reference_id,
        instance: InstancePath::default(),
        sub_element: None,
    }]);
    explode(&mut app, &mut session).unwrap();

    let lines = model_lines(&app);
    assert_eq!(
        lines.len(),
        1,
        "the second EXPLODE yields block 2's LINE member"
    );
    let (start, end) = lines[0];
    assert!(
        (near(start, p3(11.0, 1.0, 0.0)) && near(end, p3(13.0, 1.0, 0.0)))
            || (near(start, p3(13.0, 1.0, 0.0)) && near(end, p3(11.0, 1.0, 0.0))),
        "{start:?} .. {end:?}"
    );
}
#[test]
fn explode_a_self_referential_block_once_yields_the_self_reference_without_recursion() {
    // Block 1's only member is an INSERT of block 1. Exploding once yields the
    // self-reference again (with the composed placement) and terminates, so no
    // cycle guard and no recursion is needed.
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_block(BlockDefinition {
        id: cad_domain::BlockId(1),
        entities: vec![EntityId(100)],
        dynamic_visibility: None,
    })
    .unwrap();
    b.insert_entity(part(
        100,
        insert(1, Transform3::translation(p3(0.0, 5.0, 0.0))),
        LayerId(0),
        SpaceId::Block(cad_domain::BlockId(1)),
        "AcDbBlockReference",
        0,
    ))
    .unwrap();
    b.insert_entity(part(
        1,
        insert(1, Transform3::translation(p3(10.0, 0.0, 0.0))),
        LayerId(0),
        SpaceId::Model,
        "AcDbBlockReference",
        1,
    ))
    .unwrap();
    let (mut app, mut session) = app_from(b.finish().unwrap());

    select_one(&mut app, &mut session, 1);
    let outcome = explode(&mut app, &mut session).expect("a self-reference explodes once");
    assert_eq!(
        outcome.objects.len(),
        1,
        "exactly the self-reference is produced"
    );
    let produced = model_geometry(&app);
    assert_eq!(produced.len(), 1);
    match &produced[0].1 {
        SemanticGeometry::Insert { block, transform } => {
            assert_eq!(*block, cad_domain::BlockId(1));
            assert!(
                (transform.matrix[0][3] - 10.0).abs() < 1e-9,
                "{transform:?}"
            );
            assert!((transform.matrix[1][3] - 5.0).abs() < 1e-9, "{transform:?}");
        }
        other => panic!("expected the self-reference INSERT, got {other:?}"),
    }
    assert_eq!(undo_depth(&app), 1, "one transaction / one undo step");
}

#[test]
fn explode_non_uniform_insert_turns_an_arc_member_into_an_exact_elliptical_arc() {
    // An ARC under (x×2, y×1) is an elliptical arc with the same angular sweep;
    // the parameter offset is re-read from the transformed start point.
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_block(BlockDefinition {
        id: cad_domain::BlockId(1),
        entities: vec![EntityId(100)],
        dynamic_visibility: None,
    })
    .unwrap();
    b.insert_entity(part(
        100,
        SemanticGeometry::Arc {
            center: point(0.0, 0.0),
            normal: p3(0.0, 0.0, 1.0),
            radius: 1.0,
            start: 0.0,
            sweep: std::f64::consts::FRAC_PI_2,
        },
        LayerId(0),
        SpaceId::Block(cad_domain::BlockId(1)),
        "AcDbArc",
        0,
    ))
    .unwrap();
    b.insert_entity(part(
        1,
        insert(1, non_uniform(2.0, 1.0)),
        LayerId(0),
        SpaceId::Model,
        "AcDbBlockReference",
        1,
    ))
    .unwrap();
    let (mut app, mut session) = app_from(b.finish().unwrap());
    select_one(&mut app, &mut session, 1);
    explode(&mut app, &mut session).unwrap();
    let produced = model_geometry(&app);
    assert_eq!(produced.len(), 1);
    match &produced[0].1 {
        SemanticGeometry::Ellipse {
            major_axis,
            ratio,
            sweep,
            ..
        } => {
            assert!((cad_geometry::length(*major_axis) - 2.0).abs() < 1e-9);
            assert!((*ratio - 0.5).abs() < 1e-9);
            assert!(
                (*sweep - std::f64::consts::FRAC_PI_2).abs() < 1e-9,
                "the sweep is preserved: {sweep}"
            );
        }
        other => panic!("expected an elliptical ARC as ELLIPSE, got {other:?}"),
    }
}
