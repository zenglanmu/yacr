//! EXPLODE edit-command contract tests (ADR 0016 decision G).
//!
//! Pins the one-level decomposition (polyline → segments + arcs, INSERT → members
//! with the placement baked in under the cycle/depth guard, Compound → every
//! child including the hatch fill), the explicit refusal set (nothing written),
//! the one-transaction/one-undo-step shape, and the declared member-layer and
//! dynamic-visibility rules.

use super::*;

/// A `DbEntity` with an explicit layer, space and type key.
pub(super) fn part(
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

pub(super) fn line(x0: f64, y0: f64, x1: f64, y1: f64) -> SemanticGeometry {
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

pub(super) fn insert(block: u128, transform: Transform3) -> SemanticGeometry {
    SemanticGeometry::Insert {
        block: cad_domain::BlockId(block),
        transform,
    }
}

/// A non-uniform scale transform (a shear-free but anisotropic linear part).
pub(super) fn non_uniform(scale_x: f64, scale_y: f64) -> Transform3 {
    let mut matrix = Transform3::identity().matrix;
    matrix[0][0] = scale_x;
    matrix[1][1] = scale_y;
    Transform3 { matrix }
}

pub(super) fn text_geometry() -> SemanticGeometry {
    SemanticGeometry::Text {
        text: "x".into(),
        position: point(0.0, 0.0),
        style: StyleId(0),
        height: 1.0,
        rotation: 0.0,
        font: None,
        h_align: TextAlignH::Left,
        v_align: TextAlignV::Baseline,
        wrap_width: None,
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
pub(super) fn app_from(drawing: cad_db::DrawingDatabase) -> (Application, SessionState) {
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

pub(super) fn select_one(app: &mut Application, session: &mut SessionState, entity: u128) {
    execute(
        app,
        session,
        CommandId::Select,
        CommandPayload::Selection(vec![ref_of(entity)]),
    )
    .unwrap();
}

pub(super) fn explode(
    app: &mut Application,
    session: &mut SessionState,
) -> CadResult<CommandOutcome> {
    execute(
        app,
        session,
        CommandId::ExplodeEntities,
        CommandPayload::None,
    )
}

pub(super) fn model_geometry(app: &Application) -> Vec<(EntityId, SemanticGeometry)> {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .model_space()
        .iter()
        .map(|entity| (entity.id, entity.geometry.clone()))
        .collect()
}

pub(super) fn model_lines(app: &Application) -> Vec<(Point3, Point3)> {
    model_geometry(app)
        .into_iter()
        .filter_map(|(_, geometry)| match geometry {
            SemanticGeometry::Line { start, end } => Some((start, end)),
            _ => None,
        })
        .collect()
}

pub(super) fn entity_count(app: &Application) -> usize {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity_count()
}

pub(super) fn undo_depth(app: &Application) -> usize {
    app.history
        .get(&DocumentId(1))
        .map(|history| history.undo_depth())
        .unwrap_or(0)
}

pub(super) fn near(a: Point3, b: Point3) -> bool {
    (a.x - b.x).abs() < 1e-9 && (a.y - b.y).abs() < 1e-9 && (a.z - b.z).abs() < 1e-9
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
fn explode_refuses_a_non_decomposable_child_explicitly_by_kind() {
    // A Compound whose child is Text (only reachable via a compound in this
    // build). ADR 0019 decision B made Text transformable, so the refusal is now
    // its **own named reason** (`push_produced`/`child_refusal`), not a transform
    // side effect — and the behaviour (whole command fails) is unchanged.
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
        let kind = crate::app_drawing::class_name(&geometry);
        let compound = SemanticGeometry::Compound(vec![line(0.0, 0.0, 1.0, 0.0), geometry]);
        let (mut app, mut session) = single_app(compound, "AcDbCompound");
        let before = revision(&app);
        select_one(&mut app, &mut session, 1);
        let error = explode(&mut app, &mut session).expect_err("non-decomposable child");
        match error {
            CadError::Unsupported(reason) => {
                assert!(
                    reason.contains(kind),
                    "the refusal must name {kind}: {reason}"
                );
                assert!(
                    reason.contains("EXPLODE does not decompose"),
                    "the refusal must be the explicit kind refusal: {reason}"
                );
            }
            other => panic!("expected an Unsupported refusal, got {other:?}"),
        }
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
fn explode_refuses_a_dimension_entity_by_type_key() {
    // ADR 0017 A.8: the new `SemanticGeometry::Dimension` arm must refuse rather
    // than decompose. The existing test only covered a `Compound` carrying the
    // `"AcDbDimension"` string; this exercises the real variant.
    let dimension = DimensionGeometry::synthesized(
        dimension_definition(),
        "Standard".into(),
        dimension_style(),
        None,
    )
    .expect("synthesized");
    let (mut app, mut session) = single_app(
        SemanticGeometry::Dimension(Box::new(dimension)),
        "AcDbDimension",
    );
    let before = revision(&app);
    select_one(&mut app, &mut session, 1);
    let error = explode(&mut app, &mut session).expect_err("a dimension is refused");
    match error {
        CadError::Unsupported(reason) => assert!(
            reason.contains("type_key"),
            "the refusal must name the type_key reason: {reason}"
        ),
        other => panic!("expected an Unsupported type_key refusal, got {other:?}"),
    }
    assert_eq!(revision(&app), before, "a refusal changes nothing");
    assert_eq!(entity_count(&app), 1);
    assert_eq!(undo_depth(&app), 0);
}

/// A synthesized dimension's style snapshot for the EXPLODE tests.
fn dimension_style() -> DimensionStyleValues {
    DimensionStyleValues {
        arrow_size: 2.5,
        ext_offset: 0.625,
        ext_beyond: 1.25,
        text_height: 2.5,
        text_gap: 0.625,
        center_mark: 0.0,
        dim_line_increment: 3.75,
        text_vertical: 1,
        text_vertical_position: 0.0,
        suppress_ext1: false,
        suppress_ext2: false,
        suppress_dim1: false,
        suppress_dim2: false,
        linear_factor: 1.0,
        decimals: 4,
        rounding: 0.0,
        postfix: String::new(),
        zero_suppression: 0,
        linear_unit_format: 2,
        decimal_separator: 46,
        text_style_name: "Standard".into(),
        text_style: StyleId(0),
        text_font: None,
    }
}

fn dimension_definition() -> DimensionDefinition {
    DimensionDefinition {
        kind: DimensionKind::Aligned,
        points: DimensionPoints::Linear {
            first: point(0.0, 0.0),
            second: point(10.0, 0.0),
            dimension_line: point(0.0, -5.0),
        },
        text_middle: point(0.0, 0.0),
        text_rotation: 0.0,
        normal: p3(0.0, 0.0, 1.0),
    }
}

/// A first-class solid HATCH over a 4x4 square (boundary + fill mesh).
fn hatch_geometry() -> SemanticGeometry {
    let hatch = HatchGeometry::new(
        vec![HatchLoop::new(
            vec![[0.0, 0.0], [4.0, 0.0], [4.0, 4.0], [0.0, 4.0]],
            vec![0.0; 4],
            HatchLoopFlags::default(),
        )],
        HatchPlane::default(),
        HatchStyle::Normal,
        HatchFill::Solid,
        Vec::new(),
        None,
        HatchPatternType::UserDefined,
    )
    .expect("valid hatch");
    SemanticGeometry::Hatch(Box::new(hatch))
}

#[test]
fn explode_first_class_hatch_keeps_boundary_and_fill() {
    // ADR 0018 E: EXPLODE keeps today's decomposition (boundary polylines plus
    // the fill mesh), exactly what the old `Compound` hatch produced.
    let (mut app, mut session) = single_app(hatch_geometry(), "AcDbHatch");
    assert_eq!(
        crate::app_drawing::class_name(&hatch_geometry()),
        "AcDbHatch"
    );
    select_one(&mut app, &mut session, 1);
    let outcome = explode(&mut app, &mut session).unwrap();
    assert_eq!(outcome.objects.len(), 2, "boundary + fill mesh");
    let produced = model_geometry(&app);
    assert_eq!(produced.len(), 2);
    assert_eq!(
        produced
            .iter()
            .filter(|(_, geometry)| matches!(geometry, SemanticGeometry::Polyline { .. }))
            .count(),
        1,
        "the boundary loop is produced"
    );
    assert_eq!(
        produced
            .iter()
            .filter(|(_, geometry)| matches!(geometry, SemanticGeometry::Mesh(_)))
            .count(),
        1,
        "the fill mesh is produced"
    );
}

#[test]
fn explode_insert_with_a_hatch_member_keeps_the_in_block_refusal() {
    // Before P6a a hatch inside a block was a `Compound` and `push_produced`
    // refused it; the first-class variant must keep that refusal.
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
        hatch_geometry(),
        LayerId(0),
        SpaceId::Block(cad_domain::BlockId(1)),
        "AcDbHatch",
        0,
    ))
    .unwrap();
    b.insert_entity(part(
        1,
        insert(1, Transform3::translation(p3(5.0, 0.0, 0.0))),
        LayerId(0),
        SpaceId::Model,
        "AcDbBlockReference",
        1,
    ))
    .unwrap();
    let (mut app, mut session) = app_from(b.finish().unwrap());
    let before = revision(&app);
    select_one(&mut app, &mut session, 1);
    let error = explode(&mut app, &mut session).expect_err("the hatch member is refused");
    assert!(matches!(error, CadError::Unsupported(_)), "{error:?}");
    assert_eq!(revision(&app), before, "nothing is written");
    assert_eq!(entity_count(&app), 2);
    assert_eq!(undo_depth(&app), 0);
}
