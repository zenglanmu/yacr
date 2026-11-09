//! QDIM contract tests (AutoCAD 2026): every documented generation mode plus the
//! Datum Point / Edit / Settings options.
//!
//! These tests read the **stored** dimensions back (kind, measurement, definition
//! points) and pin the one-transaction/one-undo shape, the per-item `qdim.skipped`
//! policy and every whole-run refusal.

use super::*;

const CHAIN_A: u128 = 901;
const CHAIN_B: u128 = 902;
const CIRCLE: u128 = 903;
const ARC: u128 = 904;
const CROSS_V: u128 = 905;
const CROSS_H1: u128 = 906;
const CROSS_H2: u128 = 907;
const DIM_LINEAR: u128 = 908;

fn standard_style() -> DimensionStyleValues {
    cad_db::DimStyle::standard_metric(DimStyleId(0)).resolved(
        "Standard".to_string(),
        StyleId(0),
        None,
    )
}

fn synthesized(kind: DimensionKind, points: DimensionPoints) -> SemanticGeometry {
    SemanticGeometry::Dimension(Box::new(
        DimensionGeometry::synthesized(
            DimensionDefinition {
                kind,
                points,
                text_middle: point(0.0, 0.0),
                text_rotation: 0.0,
                normal: p3(0.0, 0.0, 1.0),
            },
            "Standard".to_string(),
            standard_style(),
            None,
        )
        .expect("the fixture dimension synthesizes"),
    ))
}

fn qdim_app() -> (Application, SessionState) {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_dim_style(cad_db::DimStyle::standard_metric(DimStyleId(0)))
        .unwrap();
    b.insert_entity(ent(
        CHAIN_A,
        SemanticGeometry::Line {
            start: point(0.0, 300.0),
            end: point(10.0, 300.0),
        },
        0,
    ))
    .unwrap();
    b.insert_entity(ent(
        CHAIN_B,
        SemanticGeometry::Line {
            start: point(10.0, 300.0),
            end: point(25.0, 300.0),
        },
        1,
    ))
    .unwrap();
    b.insert_entity(ent(
        CIRCLE,
        SemanticGeometry::Circle {
            center: point(100.0, 100.0),
            normal: p3(0.0, 0.0, 1.0),
            radius: 5.0,
        },
        2,
    ))
    .unwrap();
    b.insert_entity(ent(
        ARC,
        SemanticGeometry::Arc {
            center: point(200.0, 100.0),
            normal: p3(0.0, 0.0, 1.0),
            radius: 5.0,
            start: 0.0,
            sweep: 1.0,
        },
        3,
    ))
    .unwrap();
    b.insert_entity(ent(
        CROSS_V,
        SemanticGeometry::Line {
            start: point(400.0, -5.0),
            end: point(400.0, 5.0),
        },
        4,
    ))
    .unwrap();
    b.insert_entity(ent(
        CROSS_H1,
        SemanticGeometry::Line {
            start: point(395.0, -2.0),
            end: point(405.0, -2.0),
        },
        5,
    ))
    .unwrap();
    b.insert_entity(ent(
        CROSS_H2,
        SemanticGeometry::Line {
            start: point(395.0, 2.0),
            end: point(405.0, 2.0),
        },
        6,
    ))
    .unwrap();
    // An existing dimension is a legal QDIM input.
    b.insert_entity(ent(
        DIM_LINEAR,
        synthesized(
            DimensionKind::Linear { rotation: 0.0 },
            DimensionPoints::Linear {
                first: point(500.0, 0.0),
                second: point(510.0, 0.0),
                dimension_line: point(505.0, -5.0),
            },
        ),
        7,
    ))
    .unwrap();
    let drawing = b.finish().unwrap();
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

fn select(session: &mut SessionState, ids: &[u128]) {
    session.selection = SelectionSet::from_refs(ids.iter().copied().map(ref_of));
}

fn stored(app: &Application, id: u128) -> DimensionGeometry {
    let entity = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(id))
        .expect("the dimension entity");
    match &entity.geometry {
        SemanticGeometry::Dimension(dimension) => (**dimension).clone(),
        other => panic!("expected a dimension, got {other:?}"),
    }
}

fn undo_depth(app: &Application) -> usize {
    app.history
        .get(&DocumentId(1))
        .map(|history| history.undo_depth())
        .unwrap_or(0)
}

fn approx(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

fn qdim_payload(request: QDimCommand) -> CommandPayload {
    CommandPayload::Dimension(Box::new(DimensionBatchPayload::QDim(request)))
}

fn qdim_request(mode: QDimMode) -> QDimCommand {
    QDimCommand {
        mode,
        ..QDimCommand::default()
    }
}

#[test]
fn qdim_continuous_is_the_bare_default_and_chains_collinear_lines_once() {
    let (mut app, mut session) = qdim_app();
    select(&mut session, &[CHAIN_A, CHAIN_B]);
    let before = revision(&app);

    let out = execute(
        &mut app,
        &mut session,
        CommandId::QDim,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(out.objects.len(), 2);
    assert_eq!(revision(&app).0, before.0 + 1);
    assert_eq!(undo_depth(&app), 1);

    let first = stored(&app, out.objects[0].0);
    assert!(matches!(first.kind(), DimensionKind::Linear { .. }));
    assert!(approx(first.measurement(), 10.0));
    match &first.definition().points {
        DimensionPoints::Linear {
            first,
            second,
            dimension_line,
        } => {
            assert_eq!(*first, point(0.0, 300.0));
            assert_eq!(*second, point(10.0, 300.0));
            // +X chain direction, left normal +Y, Standard DIMDLI 3.75.
            assert_eq!(*dimension_line, point(0.0, 303.75));
        }
        other => panic!("expected Linear points, got {other:?}"),
    }
    let second = stored(&app, out.objects[1].0);
    assert!(approx(second.measurement(), 15.0));
    match &second.definition().points {
        DimensionPoints::Linear { first, second, .. } => {
            assert_eq!(*first, point(10.0, 300.0));
            assert_eq!(*second, point(25.0, 300.0));
        }
        other => panic!("expected Linear points, got {other:?}"),
    }
}

#[test]
fn qdim_baseline_and_staggered_stack_rows_and_honour_the_datum() {
    let (mut app, mut session) = qdim_app();
    select(&mut session, &[CHAIN_A, CHAIN_B]);

    let out = execute(
        &mut app,
        &mut session,
        CommandId::QDim,
        qdim_payload(qdim_request(QDimMode::Baseline)),
    )
    .unwrap();
    assert_eq!(out.objects.len(), 2);
    let mut rows: Vec<f64> = out
        .objects
        .iter()
        .map(|id| match &stored(&app, id.0).definition().points {
            DimensionPoints::Linear { dimension_line, .. } => dimension_line.y,
            other => panic!("expected Linear points, got {other:?}"),
        })
        .collect();
    rows.sort_by(f64::total_cmp);
    assert_eq!(rows, vec![303.75, 307.5], "two stacked baseline rows");
    for id in &out.objects {
        match &stored(&app, id.0).definition().points {
            DimensionPoints::Linear { first, .. } => assert_eq!(*first, point(0.0, 300.0)),
            other => panic!("expected Linear points, got {other:?}"),
        }
    }

    // A datum replaces the common extension origin.
    let mut request = qdim_request(QDimMode::Baseline);
    request.datum = Some(point(-5.0, 0.0));
    let out = execute(
        &mut app,
        &mut session,
        CommandId::QDim,
        qdim_payload(request),
    )
    .unwrap();
    assert_eq!(out.objects.len(), 3);
    let mut datum_rows: Vec<f64> = out
        .objects
        .iter()
        .map(|id| match &stored(&app, id.0).definition().points {
            DimensionPoints::Linear {
                first,
                dimension_line,
                ..
            } => {
                assert_eq!(*first, point(-5.0, 0.0));
                dimension_line.y
            }
            other => panic!("expected Linear points, got {other:?}"),
        })
        .collect();
    datum_rows.sort_by(f64::total_cmp);
    assert_eq!(datum_rows, vec![3.75, 7.5, 11.25]);

    // STAGGERED steps the increment (declared default DIMDLI; override here).
    let mut request = qdim_request(QDimMode::Staggered);
    request.increment = Some(2.0);
    let out = execute(
        &mut app,
        &mut session,
        CommandId::QDim,
        qdim_payload(request),
    )
    .unwrap();
    let mut staggered: Vec<f64> = out
        .objects
        .iter()
        .map(|id| match &stored(&app, id.0).definition().points {
            DimensionPoints::Linear { dimension_line, .. } => dimension_line.y,
            other => panic!("expected Linear points, got {other:?}"),
        })
        .collect();
    staggered.sort_by(f64::total_cmp);
    assert_eq!(staggered, vec![302.0, 304.0]);
}

#[test]
fn qdim_ordinate_measures_from_the_datum_on_the_dominant_axis() {
    let (mut app, mut session) = qdim_app();
    select(&mut session, &[CHAIN_A, CHAIN_B]);
    let mut request = qdim_request(QDimMode::Ordinate);
    request.datum = Some(point(0.0, 290.0));
    let out = execute(
        &mut app,
        &mut session,
        CommandId::QDim,
        qdim_payload(request),
    )
    .unwrap();
    assert_eq!(out.objects.len(), 3);
    let axes: Vec<bool> = out
        .objects
        .iter()
        .map(|id| match stored(&app, id.0).kind() {
            DimensionKind::Ordinate { x_ordinate } => x_ordinate,
            other => panic!("QDIM ORDINATE must create ordinate dimensions, got {other:?}"),
        })
        .collect();
    assert_eq!(
        axes,
        vec![false, true, true],
        "dominant axis of origin-datum"
    );
    match &stored(&app, out.objects[0].0).definition().points {
        DimensionPoints::Ordinate {
            base, leader_end, ..
        } => {
            assert_eq!(*base, point(0.0, 290.0));
            assert_eq!(*leader_end, point(0.0, 303.75));
        }
        other => panic!("expected Ordinate points, got {other:?}"),
    }
}

#[test]
fn qdim_radius_and_diameter_consume_circles_and_arcs() {
    let (mut app, mut session) = qdim_app();
    for (mode, id, expected) in [
        (QDimMode::Radius, CIRCLE, 5.0),
        (QDimMode::Diameter, CIRCLE, 10.0),
        (QDimMode::Radius, ARC, 5.0),
        (QDimMode::Diameter, ARC, 10.0),
    ] {
        select(&mut session, &[id]);
        let out = execute(
            &mut app,
            &mut session,
            CommandId::QDim,
            qdim_payload(qdim_request(mode)),
        )
        .unwrap();
        assert_eq!(out.objects.len(), 1);
        let dimension = stored(&app, out.objects[0].0);
        assert!(approx(dimension.measurement(), expected), "{mode:?}");
        match (mode, dimension.kind()) {
            (QDimMode::Radius, DimensionKind::Radius) => {}
            (QDimMode::Diameter, DimensionKind::Diameter) => {}
            (_, other) => panic!("{mode:?} created {other:?}"),
        }
    }
}

#[test]
fn qdim_edit_and_settings_are_typed_options_with_named_refusals() {
    let (mut app, mut session) = qdim_app();
    select(&mut session, &[CHAIN_A, CHAIN_B]);
    let mut request = qdim_request(QDimMode::Continuous);
    request.remove = vec![1];
    let out = execute(
        &mut app,
        &mut session,
        CommandId::QDim,
        qdim_payload(request),
    )
    .unwrap();
    assert_eq!(out.objects.len(), 1);
    assert!(approx(stored(&app, out.objects[0].0).measurement(), 25.0));

    let mut request = qdim_request(QDimMode::Continuous);
    request.remove = vec![9];
    let error = execute(
        &mut app,
        &mut session,
        CommandId::QDim,
        qdim_payload(request),
    )
    .unwrap_err();
    assert!(format!("{error}").contains("qdim.edit_range"), "{error}");

    // INTERSECTIONS uses the two crossings of the vertical with the horizontals.
    select(&mut session, &[CROSS_V, CROSS_H1, CROSS_H2]);
    let mut request = qdim_request(QDimMode::Continuous);
    request.snap = QDimSnap::Intersections;
    let out = execute(
        &mut app,
        &mut session,
        CommandId::QDim,
        qdim_payload(request),
    )
    .unwrap();
    assert_eq!(out.objects.len(), 1);
    assert!(approx(stored(&app, out.objects[0].0).measurement(), 4.0));

    // One selected LINE cannot produce a pairwise intersection.
    select(&mut session, &[CROSS_V]);
    let mut request = qdim_request(QDimMode::Continuous);
    request.snap = QDimSnap::Intersections;
    let error = execute(
        &mut app,
        &mut session,
        CommandId::QDim,
        qdim_payload(request),
    )
    .unwrap_err();
    assert!(
        format!("{error}").contains("qdim.no_intersections"),
        "{error}"
    );
}

#[test]
fn qdim_skips_unsupported_kinds_per_item_and_refuses_empty_target_sets() {
    let (mut app, mut session) = qdim_app();
    select(&mut session, &[CIRCLE, CHAIN_A, CHAIN_B]);
    let out = execute(
        &mut app,
        &mut session,
        CommandId::QDim,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(out.objects.len(), 2);
    assert!(
        out.diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "qdim.skipped"),
        "the circle is a named per-item skip"
    );

    select(&mut session, &[CIRCLE]);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::QDim,
        CommandPayload::None,
    )
    .unwrap_err();
    assert!(format!("{error}").contains("qdim.no_targets"), "{error}");
    select(&mut session, &[]);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::QDim,
        CommandPayload::None,
    )
    .unwrap_err();
    assert!(format!("{error}").contains("qdim.selection"), "{error}");
}

#[test]
fn an_existing_dimension_is_a_qdim_input() {
    let (mut app, mut session) = qdim_app();
    select(&mut session, &[CHAIN_A, DIM_LINEAR]);
    let out = execute(
        &mut app,
        &mut session,
        CommandId::QDim,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(out.objects.len(), 3, "four origins make three rows");
    let created = stored(&app, out.objects[0].0);
    assert!(matches!(created.kind(), DimensionKind::Linear { .. }));
    // The chain direction comes from the line, so every row is a linear
    // dimension between the ordered origins (including the dimension's own).
    assert!(created.measurement() > 0.0);
}
