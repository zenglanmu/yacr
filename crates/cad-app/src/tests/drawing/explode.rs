//! EXPLODE edit-command contract tests (ADR 0016 decision G).
//!
//! Pins the one-level decomposition (polyline → segments + arcs, INSERT → members
//! with the placement baked in under the cycle/depth guard, Compound → every
//! child including the hatch fill), the explicit refusal set (nothing written),
//! the one-transaction/one-undo-step shape, and the declared member-layer and
//! dynamic-visibility rules.

use super::*;

/// A `DbEntity` with an explicit layer, space and type key.
fn part(
    id: u128,
    geometry: SemanticGeometry,
    layer: LayerId,
    space: SpaceId,
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
        layer,
        space,
        geometry,
        draw_order: order,
    }
}

fn line(x0: f64, y0: f64, x1: f64, y1: f64) -> SemanticGeometry {
    SemanticGeometry::Line {
        start: point(x0, y0),
        end: point(x1, y1),
    }
}

fn polyline(points: Vec<Point3>, bulges: Vec<f64>, closed: bool) -> SemanticGeometry {
    SemanticGeometry::Polyline {
        points,
        bulges,
        closed,
    }
}

fn insert(block: u128, transform: Transform3) -> SemanticGeometry {
    SemanticGeometry::Insert {
        block: cad_domain::BlockId(block),
        transform,
    }
}

/// A non-uniform scale transform (a shear-free but anisotropic linear part).
fn non_uniform(scale_x: f64, scale_y: f64) -> Transform3 {
    let mut matrix = Transform3::identity().matrix;
    matrix[0][0] = scale_x;
    matrix[1][1] = scale_y;
    Transform3 { matrix }
}

fn text_geometry() -> SemanticGeometry {
    SemanticGeometry::Text {
        text: "x".into(),
        position: point(0.0, 0.0),
        style: StyleId(0),
        height: 1.0,
        rotation: 0.0,
        font: None,
        h_align: TextAlignH::Left,
        v_align: TextAlignV::Baseline,
    }
}

fn image_geometry() -> SemanticGeometry {
    SemanticGeometry::Image {
        origin: point(0.0, 0.0),
        u: Point3 {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        },
        v: Point3 {
            x: 0.0,
            y: 1.0,
            z: 0.0,
        },
        pixels: [1.0, 1.0],
        file: None,
        clip: None,
        visible: true,
    }
}

fn mask_geometry() -> SemanticGeometry {
    SemanticGeometry::Mask {
        boundary: vec![point(0.0, 0.0), point(1.0, 0.0), point(1.0, 1.0)],
        inverted: false,
    }
}

/// Wrap a finished builder into a Work-mode app/session pair.
fn app_from(drawing: cad_db::DrawingDatabase) -> (Application, SessionState) {
    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([21u8; 32]),
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

/// A drawing with layer 0 (and optionally layer 7) plus the given model entities.
fn app_with_layers(extra_layer: bool, entities: Vec<DbEntity>) -> (Application, SessionState) {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    if extra_layer {
        b.insert_layer(Layer {
            id: LayerId(7),
            name: "MEMBERS".into(),
            visible: true,
        })
        .unwrap();
    }
    for entity in entities {
        b.insert_entity(entity).unwrap();
    }
    app_from(b.finish().unwrap())
}

/// An app with one model-space entity of the given geometry/type key.
fn single_app(geometry: SemanticGeometry, type_key: &str) -> (Application, SessionState) {
    app_with_layers(
        false,
        vec![part(1, geometry, LayerId(0), SpaceId::Model, type_key, 0)],
    )
}

/// An app with one model-space polyline (entity 1).
fn polyline_app(
    points: Vec<Point3>,
    bulges: Vec<f64>,
    closed: bool,
) -> (Application, SessionState) {
    single_app(polyline(points, bulges, closed), "AcDbPolyline")
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

fn explode(app: &mut Application, session: &mut SessionState) -> CadResult<CommandOutcome> {
    execute(
        app,
        session,
        CommandId::ExplodeEntities,
        CommandPayload::None,
    )
}

fn model_geometry(app: &Application) -> Vec<(EntityId, SemanticGeometry)> {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .model_space()
        .iter()
        .map(|entity| (entity.id, entity.geometry.clone()))
        .collect()
}

fn model_lines(app: &Application) -> Vec<(Point3, Point3)> {
    model_geometry(app)
        .into_iter()
        .filter_map(|(_, geometry)| match geometry {
            SemanticGeometry::Line { start, end } => Some((start, end)),
            _ => None,
        })
        .collect()
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

fn near(a: Point3, b: Point3) -> bool {
    (a.x - b.x).abs() < 1e-9 && (a.y - b.y).abs() < 1e-9 && (a.z - b.z).abs() < 1e-9
}

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
fn explode_compound_keeps_every_child_including_hatch_fill() {
    // A hatch imports as a Compound of boundary loops PLUS the fill mesh and
    // pattern polylines; decomposing to loops only would be a silent loss, so
    // every child is produced.
    let boundary = polyline(
        vec![
            point(0.0, 0.0),
            point(4.0, 0.0),
            point(4.0, 4.0),
            point(0.0, 4.0),
        ],
        vec![0.0; 4],
        true,
    );
    let fill = SemanticGeometry::Mesh(Mesh {
        vertices: vec![point(0.0, 0.0), point(4.0, 0.0), point(4.0, 4.0)],
        triangles: vec![[0, 1, 2]],
        normals: vec![p3(0.0, 0.0, 1.0); 3],
        face_sources: Vec::new(),
        colors: Vec::new(),
    });
    let pattern = polyline(vec![point(0.0, 1.0), point(4.0, 1.0)], vec![0.0; 2], false);
    let compound = SemanticGeometry::Compound(vec![boundary, fill, pattern]);
    let (mut app, mut session) = single_app(compound, "AcDbHatch");

    select_one(&mut app, &mut session, 1);
    let outcome = explode(&mut app, &mut session).unwrap();
    assert_eq!(
        outcome.objects.len(),
        3,
        "all three children are produced, including the fill mesh"
    );
    let produced = model_geometry(&app);
    assert_eq!(produced.len(), 3, "no child is silently dropped");
    assert_eq!(
        produced
            .iter()
            .filter(|(_, geometry)| matches!(geometry, SemanticGeometry::Mesh(_)))
            .count(),
        1,
        "the hatch fill mesh is produced"
    );
    assert_eq!(
        produced
            .iter()
            .filter(|(_, geometry)| matches!(geometry, SemanticGeometry::Polyline { .. }))
            .count(),
        2,
        "the boundary loop and the pattern polyline are both produced"
    );
}

#[test]
fn explode_closed_polyline_with_bulge_yields_segments_and_arcs() {
    // An open two-vertex polyline with bulge 1: one ARC (a unit-radius
    // semicircle over the 2-unit chord). The recovered arc must span the chord.
    let (mut app, mut session) =
        polyline_app(vec![point(0.0, 0.0), point(2.0, 0.0)], vec![1.0], false);
    select_one(&mut app, &mut session, 1);
    explode(&mut app, &mut session).unwrap();

    let produced = model_geometry(&app);
    assert_eq!(produced.len(), 1, "a bulge segment becomes one ARC");
    match &produced[0].1 {
        SemanticGeometry::Arc {
            center,
            radius,
            start,
            sweep,
            normal,
        } => {
            assert!((radius - 1.0).abs() < 1e-9, "radius = {radius}");
            assert!(near(*center, p3(1.0, 0.0, 0.0)), "centre = {center:?}");
            assert!(
                (sweep.abs() - std::f64::consts::PI).abs() < 1e-9,
                "sweep = {sweep}"
            );
            assert!((normal.z - 1.0).abs() < 1e-9, "plane normal = {normal:?}");
            let at = |angle: f64| Point3 {
                x: center.x + radius * angle.cos(),
                y: center.y + radius * angle.sin(),
                z: center.z,
            };
            assert!(
                near(at(*start), p3(0.0, 0.0, 0.0)) && near(at(start + sweep), p3(2.0, 0.0, 0.0)),
                "the arc must span the chord"
            );
        }
        other => panic!("expected an ARC, got {other:?}"),
    }
}

#[test]
fn explode_straight_closed_polyline_yields_only_lines_including_the_closing_segment() {
    let (mut app, mut session) = polyline_app(
        vec![point(0.0, 0.0), point(4.0, 0.0), point(4.0, 3.0)],
        vec![0.0, 0.0, 0.0],
        true,
    );
    select_one(&mut app, &mut session, 1);
    explode(&mut app, &mut session).unwrap();

    let lines = model_lines(&app);
    assert_eq!(lines.len(), 3, "a closed triangle has three segments");
    let has = |a: Point3, b: Point3| {
        lines
            .iter()
            .any(|(s, e)| (near(*s, a) && near(*e, b)) || (near(*s, b) && near(*e, a)))
    };
    assert!(has(point(0.0, 0.0), point(4.0, 0.0)));
    assert!(has(point(4.0, 0.0), point(4.0, 3.0)));
    assert!(
        has(point(4.0, 3.0), point(0.0, 0.0)),
        "the closing segment is produced"
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
fn explode_is_one_transaction_one_undo_step_and_undo_restores_the_source_exactly() {
    let (mut app, mut session) = polyline_app(
        vec![point(0.0, 0.0), point(4.0, 0.0), point(4.0, 3.0)],
        vec![0.0, 0.0, 0.0],
        true,
    );
    let before = revision(&app);
    select_one(&mut app, &mut session, 1);
    explode(&mut app, &mut session).unwrap();
    assert_eq!(revision(&app).0, before.0 + 1, "one transaction");
    assert_eq!(undo_depth(&app), 1, "one undo step for the whole explode");
    assert_eq!(
        entity_count(&app),
        3,
        "source deleted, three segments inserted"
    );

    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(entity_count(&app), 1, "undo removes every child");
    assert_eq!(undo_depth(&app), 0);
    let restored = model_geometry(&app);
    assert!(
        matches!(
            &restored[0].1,
            SemanticGeometry::Polyline { closed: true, .. }
        ),
        "the source polyline is restored exactly: {:?}",
        restored[0].1
    );
}

#[test]
fn explode_refuses_a_selection_with_nothing_decomposable_and_writes_nothing() {
    for geometry in [
        line(0.0, 0.0, 1.0, 0.0),
        text_geometry(),
        SemanticGeometry::Spline {
            degree: 3,
            knots: vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
            control_points: vec![
                point(0.0, 0.0),
                point(1.0, 0.0),
                point(1.0, 1.0),
                point(2.0, 1.0),
            ],
            weights: vec![1.0; 4],
        },
        SemanticGeometry::Shape {
            shape_name: "x".into(),
            code: 1,
            position: point(0.0, 0.0),
            size: 1.0,
            rotation: 0.0,
            font: None,
        },
        SemanticGeometry::Opaque {
            type_key: "ACAD_PROXY_ENTITY".into(),
            version: 1,
            payload: vec![1, 2, 3],
        },
    ] {
        let (mut app, mut session) = single_app(geometry, "AcDbEntity");
        let before = revision(&app);
        select_one(&mut app, &mut session, 1);
        let error = explode(&mut app, &mut session).expect_err("not decomposable");
        assert!(matches!(error, CadError::Unsupported(_)), "{error:?}");
        assert_eq!(revision(&app), before, "a refusal changes nothing");
        assert_eq!(entity_count(&app), 1);
        assert_eq!(undo_depth(&app), 0);
    }

    // An empty selection is an explicit refusal.
    let (mut app, mut session) =
        polyline_app(vec![point(0.0, 0.0), point(2.0, 0.0)], vec![0.0], false);
    let before = revision(&app);
    let error = explode(&mut app, &mut session).expect_err("an empty selection is refused");
    assert!(matches!(error, CadError::InvalidInput(_)), "{error:?}");
    assert_eq!(revision(&app), before);
    assert_eq!(entity_count(&app), 1);
}

#[test]
fn explode_refuses_an_untransformable_child_through_the_transform_surface() {
    // A Compound whose child is Text (only reachable via a compound in this
    // build): the transform surface refuses it, so the whole command fails.
    for geometry in [
        text_geometry(),
        SemanticGeometry::Spline {
            degree: 3,
            knots: vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
            control_points: vec![
                point(0.0, 0.0),
                point(1.0, 0.0),
                point(1.0, 1.0),
                point(2.0, 1.0),
            ],
            weights: vec![1.0; 4],
        },
        SemanticGeometry::Shape {
            shape_name: "x".into(),
            code: 1,
            position: point(0.0, 0.0),
            size: 1.0,
            rotation: 0.0,
            font: None,
        },
        SemanticGeometry::Opaque {
            type_key: "ACAD_PROXY_ENTITY".into(),
            version: 1,
            payload: vec![1, 2, 3],
        },
    ] {
        let compound = SemanticGeometry::Compound(vec![line(0.0, 0.0, 1.0, 0.0), geometry]);
        let (mut app, mut session) = single_app(compound, "AcDbCompound");
        let before = revision(&app);
        select_one(&mut app, &mut session, 1);
        let error = explode(&mut app, &mut session).expect_err("untransformable child");
        assert!(matches!(error, CadError::Unsupported(_)), "{error:?}");
        assert_eq!(revision(&app), before, "a refusal changes nothing");
        assert_eq!(entity_count(&app), 1);
        assert_eq!(undo_depth(&app), 0);
    }
}

#[test]
fn explode_refuses_image_and_mask_by_policy_not_by_the_transform_layer() {
    // IMAGE/MASK are transformable, so their refusal is a stated policy choice.
    for geometry in [image_geometry(), mask_geometry()] {
        // Top-level: the policy reason.
        let (mut app, mut session) = single_app(geometry.clone(), "AcDbRasterImage");
        select_one(&mut app, &mut session, 1);
        let error = explode(&mut app, &mut session).expect_err("image/mask refused");
        match error {
            CadError::Unsupported(reason) => assert!(
                reason.contains("policy"),
                "the refusal must state the policy reason: {reason}"
            ),
            other => panic!("expected an Unsupported policy refusal, got {other:?}"),
        }

        // As a Compound child the same policy reason applies.
        let compound = SemanticGeometry::Compound(vec![geometry]);
        let (mut app, mut session) = single_app(compound, "AcDbCompound");
        select_one(&mut app, &mut session, 1);
        let error = explode(&mut app, &mut session).expect_err("image/mask child refused");
        match error {
            CadError::Unsupported(reason) => assert!(
                reason.contains("policy"),
                "the refusal must state the policy reason: {reason}"
            ),
            other => panic!("expected an Unsupported policy refusal, got {other:?}"),
        }
    }
}

#[test]
fn explode_refuses_a_dimension_like_compound_by_type_key() {
    for type_key in ["AcDbDimension", "AcDbTable", "AcDbMultiLeader"] {
        let compound = SemanticGeometry::Compound(vec![line(0.0, 0.0, 1.0, 0.0)]);
        let (mut app, mut session) = single_app(compound, type_key);
        let before = revision(&app);
        select_one(&mut app, &mut session, 1);
        let error = explode(&mut app, &mut session).expect_err("dimension-like compound refused");
        match error {
            CadError::Unsupported(reason) => assert!(
                reason.contains("type_key"),
                "the refusal must name the type_key reason: {reason}"
            ),
            other => panic!("expected an Unsupported type_key refusal, got {other:?}"),
        }
        assert_eq!(revision(&app), before);
        assert_eq!(entity_count(&app), 1);
        assert_eq!(undo_depth(&app), 0);
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
