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
fn explode_minsert_compound_yields_insert_instances_then_members_on_the_next_explode() {
    // A MINSERT imports as `Compound(Insert…)`; the one-level rule means the first
    // EXPLODE yields the Insert instances and the next yields their members.
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

    select_one(&mut app, &mut session, 1);
    explode(&mut app, &mut session).unwrap();
    // The source Compound is deleted; two Insert instances remain.
    assert!(app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(1))
        .is_none());
    assert!(
        model_lines(&app).is_empty(),
        "no member is expanded at the first level"
    );
    let inserts: Vec<(EntityId, Transform3)> = model_geometry(&app)
        .into_iter()
        .filter_map(|(id, geometry)| match geometry {
            SemanticGeometry::Insert { transform, .. } => Some((id, transform)),
            _ => None,
        })
        .collect();
    assert_eq!(
        inserts.len(),
        2,
        "the MINSERT yields its two Insert instances"
    );
    assert_eq!(undo_depth(&app), 1, "one undo step");

    // The next EXPLODE on the (10,0) instance expands its member LINE.
    let instance = inserts
        .iter()
        .find(|(_, transform)| (transform.matrix[0][3] - 10.0).abs() < 1e-9)
        .map(|(id, _)| *id)
        .expect("the instance translated by (10,0)");
    session.selection = SelectionSet::from_refs(vec![SelectionRef {
        document: DocumentId(1),
        entity: instance,
        instance: InstancePath::default(),
        sub_element: None,
    }]);
    explode(&mut app, &mut session).unwrap();
    let lines = model_lines(&app);
    assert_eq!(lines.len(), 1, "the second level yields the member LINE");
    assert!(
        (near(lines[0].0, p3(10.0, 0.0, 0.0)) && near(lines[0].1, p3(11.0, 0.0, 0.0)))
            || (near(lines[0].0, p3(11.0, 0.0, 0.0)) && near(lines[0].1, p3(10.0, 0.0, 0.0))),
        "{:?}",
        lines[0]
    );
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
fn explode_refuses_a_non_uniformly_scaled_insert() {
    let (mut app, mut session) = insert_app(non_uniform(2.0, 1.0));
    let before = revision(&app);
    select_one(&mut app, &mut session, 1);
    let error = explode(&mut app, &mut session).expect_err("a non-uniform INSERT is refused");
    assert!(matches!(error, CadError::Unsupported(_)), "{error:?}");
    assert_eq!(revision(&app), before, "a refusal changes nothing");
    assert_eq!(
        entity_count(&app),
        2,
        "the block member and the INSERT remain"
    );
    assert_eq!(undo_depth(&app), 0);
}
#[test]
fn explode_insert_with_an_untransformable_member_fails_the_whole_command() {
    // Block 1 has a LINE member and a TEXT member; the TEXT member cannot have the
    // placement baked in, so the whole command fails and nothing is written.
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
    let error = explode(&mut app, &mut session).expect_err("an untransformable member fails");
    assert!(matches!(error, CadError::Unsupported(_)), "{error:?}");
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
