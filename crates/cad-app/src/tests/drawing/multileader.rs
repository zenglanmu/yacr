//! P7b-1 MULTILEADER contract tests: explicit match-point arms.
//!
//! Covers ADR 0019 decision G — EXPLODE refuses by type key (top level and
//! in-block), STRETCH refuses the whole command, pick/bounds/snap match the
//! pre-P7b `Compound`, and the stored `class_name` is `AcDbMultiLeader`.

use super::*;
use cad_measure::snap::SnapTarget;

/// A straight-line multileader with an arrowhead, a dogleg and no text.
fn multileader_geometry() -> MultiLeaderGeometry {
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
    geometry
}

fn multileader() -> SemanticGeometry {
    SemanticGeometry::MultiLeader(Box::new(multileader_geometry()))
}

fn equivalent_compound() -> SemanticGeometry {
    SemanticGeometry::Compound(multileader_geometry().display().to_vec())
}

fn ent_with(
    id: u128,
    space: SpaceId,
    geometry: SemanticGeometry,
    type_key: &str,
    order: i64,
) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: type_key.into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space,
        geometry,
        draw_order: order,
    }
}

fn app_with(entities: Vec<DbEntity>, blocks: Vec<BlockDefinition>) -> (Application, SessionState) {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    for block in blocks {
        builder.insert_block(block).unwrap();
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
            identity: DocumentIdentity::Sha256([7u8; 32]),
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

fn near(a: Point3, b: Point3) -> bool {
    (a.x - b.x).abs() < 1e-9 && (a.y - b.y).abs() < 1e-9 && (a.z - b.z).abs() < 1e-9
}

fn entity_count(app: &Application) -> usize {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity_count()
}

fn undo_depth(app: &Application) -> usize {
    app.history
        .get(&DocumentId(1))
        .map(|history| history.undo_depth())
        .unwrap_or(0)
}

fn select_one(app: &mut Application, session: &mut SessionState, entity: u128) {
    execute(
        app,
        session,
        CommandId::Select,
        CommandPayload::Selection(vec![ref_of(entity)]),
    )
    .unwrap();
}

/// The two logical canvas corners of a world-space window (default plan camera).
fn window(a: Point3, b: Point3) -> [[f64; 2]; 2] {
    let vp = Viewport::new(ViewportId(1), DocumentId(1), [800.0, 600.0]);
    let size = vp.logical_size;
    let to_logical = |world: Point3| {
        vp.camera
            .world_to_screen(world, size)
            .expect("finite world point")
    };
    [to_logical(a), to_logical(b)]
}

#[test]
fn explode_refuses_a_top_level_multileader_by_type_key_and_writes_nothing() {
    let (mut app, mut session) = app_with(
        vec![ent_with(
            1,
            SpaceId::Model,
            multileader(),
            "AcDbMultiLeader",
            0,
        )],
        Vec::new(),
    );
    let before = app.workspace.documents[&DocumentId(1)].drawing.revision();
    select_one(&mut app, &mut session, 1);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::ExplodeEntities,
        CommandPayload::None,
    )
    .expect_err("a multileader is refused");
    match error {
        CadError::Unsupported(reason) => assert!(
            reason.contains("AcDbMultiLeader") && reason.contains("type_key"),
            "the refusal must name the type key: {reason}"
        ),
        other => panic!("expected an Unsupported type_key refusal, got {other:?}"),
    }
    assert_eq!(
        app.workspace.documents[&DocumentId(1)].drawing.revision(),
        before,
        "a refusal changes nothing"
    );
    assert_eq!(entity_count(&app), 1);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn explode_refuses_an_in_block_multileader_by_type_key_and_writes_nothing() {
    let block = BlockDefinition {
        id: BlockId(0),
        entities: vec![EntityId(100)],
        dynamic_visibility: None,
    };
    let (mut app, mut session) = app_with(
        vec![
            ent_with(
                100,
                SpaceId::Block(BlockId(0)),
                multileader(),
                "AcDbMultiLeader",
                0,
            ),
            ent_with(
                1,
                SpaceId::Model,
                SemanticGeometry::Insert {
                    block: BlockId(0),
                    transform: Transform3::identity(),
                },
                "AcDbBlockReference",
                1,
            ),
        ],
        vec![block],
    );
    let before = app.workspace.documents[&DocumentId(1)].drawing.revision();
    select_one(&mut app, &mut session, 1);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::ExplodeEntities,
        CommandPayload::None,
    )
    .expect_err("an in-block multileader is refused");
    match error {
        CadError::Unsupported(reason) => assert!(
            reason.contains("AcDbMultiLeader") && reason.contains("type_key"),
            "the in-block refusal must name the type key, not a transform failure: {reason}"
        ),
        other => panic!("expected an Unsupported type_key refusal, got {other:?}"),
    }
    assert_eq!(
        app.workspace.documents[&DocumentId(1)].drawing.revision(),
        before
    );
    assert_eq!(entity_count(&app), 2);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn stretch_refuses_a_partially_enclosed_multileader_as_a_whole_command() {
    let (mut app, mut session) = app_with(
        vec![ent_with(
            1,
            SpaceId::Model,
            multileader(),
            "AcDbMultiLeader",
            0,
        )],
        Vec::new(),
    );
    let before = app.workspace.documents[&DocumentId(1)].drawing.revision();
    // A window around the leader's head only: the entity is partially enclosed,
    // which is the case a `Compound` used to fail on.
    let error = execute(
        &mut app,
        &mut session,
        CommandId::StretchEntities,
        CommandPayload::Stretch {
            rect_logical: window(point(-1.0, -1.0), point(2.0, 1.0)),
            base: point(0.0, 0.0),
            delta: point(1.0, 0.0),
        },
    )
    .expect_err("STRETCH refuses a multileader");
    match error {
        CadError::Unsupported(reason) => {
            assert!(reason.contains("MULTILEADER"), "{reason}");
        }
        other => panic!("expected an Unsupported refusal, got {other:?}"),
    }
    assert_eq!(
        app.workspace.documents[&DocumentId(1)].drawing.revision(),
        before
    );
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn multileader_pick_and_bounds_match_the_equivalent_compound() {
    let multileader = multileader();
    let compound = equivalent_compound();
    let options = cad_spatial::pick::PickOptions::new(0.25).expect("valid options");
    let identity = Transform3::identity();
    for point_on_geometry in [point(2.0, 0.0), point(8.0, 0.0), point(8.9, 2.0)] {
        let ray = Ray3 {
            origin: Point3 {
                x: point_on_geometry.x,
                y: point_on_geometry.y,
                z: 10.0,
            },
            direction: Point3 {
                x: 0.0,
                y: 0.0,
                z: -1.0,
            },
        };
        let from_multileader =
            cad_spatial::pick::hit_geometry(&ray, &multileader, &identity, &options)
                .expect("pick runs");
        let from_compound = cad_spatial::pick::hit_geometry(&ray, &compound, &identity, &options)
            .expect("pick runs");
        assert_eq!(from_multileader, from_compound, "at {point_on_geometry:?}");
    }
    // A miss must also match.
    let miss = Ray3 {
        origin: Point3 {
            x: 100.0,
            y: 100.0,
            z: 10.0,
        },
        direction: Point3 {
            x: 0.0,
            y: 0.0,
            z: -1.0,
        },
    };
    assert_eq!(
        cad_spatial::pick::hit_geometry(&miss, &multileader, &identity, &options).unwrap(),
        cad_spatial::pick::hit_geometry(&miss, &compound, &identity, &options).unwrap()
    );

    assert_eq!(
        cad_spatial::pick::pick_world_bounds(&multileader, &identity),
        cad_spatial::pick::pick_world_bounds(&compound, &identity)
    );
}

#[test]
fn multileader_snap_candidates_match_the_equivalent_compound() {
    let source = SelectionRef {
        document: DocumentId(1),
        entity: EntityId(1),
        instance: InstancePath::default(),
        sub_element: None,
    };
    let multileader = SnapTarget::new(
        multileader(),
        source.clone(),
        SpaceId::Model,
        Precision::Analytic,
    );
    let compound = SnapTarget::new(
        equivalent_compound(),
        source,
        SpaceId::Model,
        Precision::Analytic,
    );
    let ray = Ray3 {
        origin: Point3 {
            x: 2.0,
            y: 0.0,
            z: 10.0,
        },
        direction: Point3 {
            x: 0.0,
            y: 0.0,
            z: -1.0,
        },
    };
    let plane = WorkPlane {
        origin: point(0.0, 0.0),
        u: point(1.0, 0.0),
        v: point(0.0, 1.0),
    };
    let policy = TolerancePolicy::default();
    let from_multileader = cad_measure::snap::collect_candidates(
        std::slice::from_ref(&multileader),
        &ray,
        Some(&plane),
        1.0,
        Some(&SpaceId::Model),
        &policy,
    )
    .expect("snap runs");
    let from_compound = cad_measure::snap::collect_candidates(
        std::slice::from_ref(&compound),
        &ray,
        Some(&plane),
        1.0,
        Some(&SpaceId::Model),
        &policy,
    )
    .expect("snap runs");
    assert!(!from_compound.is_empty());
    assert_eq!(from_multileader, from_compound);
}

#[test]
fn multileader_class_name_is_acdb_multileader() {
    assert_eq!(
        crate::app_drawing::class_name(&multileader()),
        "AcDbMultiLeader"
    );
}

#[test]
fn rotate_multileader_moves_geometry_and_rebuilds_the_display() {
    let (mut app, mut session) = app_with(
        vec![ent_with(
            1,
            SpaceId::Model,
            multileader(),
            "AcDbMultiLeader",
            0,
        )],
        Vec::new(),
    );
    execute(
        &mut app,
        &mut session,
        CommandId::RotateEntities,
        CommandPayload::Rotate {
            refs: vec![ref_of(1)],
            base: point(0.0, 0.0),
            angle: std::f64::consts::FRAC_PI_2,
        },
    )
    .expect("a rigid rotation is accepted");
    let geometry = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(1))
        .unwrap()
        .geometry
        .clone();
    let SemanticGeometry::MultiLeader(multileader) = geometry else {
        panic!("expected the multileader variant");
    };
    assert!(near(multileader.lines()[0].points[1], point(0.0, 5.0)));
    // The display was rebuilt from the transformed definition.
    match &multileader.display()[0] {
        SemanticGeometry::Polyline { points, .. } => {
            assert!(near(points[1], point(0.0, 5.0)), "{points:?}");
        }
        other => panic!("expected the rotated polyline, got {other:?}"),
    }
}

#[test]
fn scale_multileader_is_refused_without_change() {
    let (mut app, mut session) = app_with(
        vec![ent_with(
            1,
            SpaceId::Model,
            multileader(),
            "AcDbMultiLeader",
            0,
        )],
        Vec::new(),
    );
    let before = app.workspace.documents[&DocumentId(1)].drawing.revision();
    let error = execute(
        &mut app,
        &mut session,
        CommandId::ScaleEntities,
        CommandPayload::Scale {
            refs: vec![ref_of(1)],
            base: point(0.0, 0.0),
            factor: 2.0,
        },
    )
    .expect_err("SCALE refuses a multileader");
    match error {
        CadError::Unsupported(reason) => {
            assert!(reason.contains("multileader"), "{reason}");
        }
        other => panic!("expected an Unsupported refusal, got {other:?}"),
    }
    assert_eq!(
        app.workspace.documents[&DocumentId(1)].drawing.revision(),
        before
    );
    assert_eq!(entity_count(&app), 1);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn mirror_multileader_is_refused_and_adds_no_copy() {
    let (mut app, mut session) = app_with(
        vec![ent_with(
            1,
            SpaceId::Model,
            multileader(),
            "AcDbMultiLeader",
            0,
        )],
        Vec::new(),
    );
    let before = app.workspace.documents[&DocumentId(1)].drawing.revision();
    let error = execute(
        &mut app,
        &mut session,
        CommandId::MirrorEntities,
        CommandPayload::Mirror {
            refs: vec![ref_of(1)],
            axis_start: point(0.0, 0.0),
            axis_end: point(1.0, 1.0),
        },
    )
    .expect_err("MIRROR refuses a multileader");
    match error {
        CadError::Unsupported(reason) => {
            assert!(reason.contains("multileader"), "{reason}");
        }
        other => panic!("expected an Unsupported refusal, got {other:?}"),
    }
    assert_eq!(
        app.workspace.documents[&DocumentId(1)].drawing.revision(),
        before
    );
    assert_eq!(entity_count(&app), 1);
    assert_eq!(undo_depth(&app), 0);
}
