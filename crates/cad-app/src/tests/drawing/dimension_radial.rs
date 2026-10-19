//! DIMRADIUS / DIMDIAMETER / DIMCENTER create-command contract tests (ADR 0017
//! decision C, P5d).
//!
//! The radial pair starts with an **entity pick** of an arc/circle (their only
//! documented entry point) and then takes the dimension-line location; the
//! definition's group-10 slot is the centre for a radius and the **opposite chord
//! point** for a diameter. DIMCENTER produces **plain LINE geometry** — the
//! non-associative centre mark/centrelines — **not** a `DIMENSION` entity.
//!
//! The numeric contracts pinned here:
//!
//! * the definition points (group 15 on the circle, group 10 centre vs opposite
//!   chord point), the measurement (radius vs diameter) and the synthesized
//!   display children (the leader line + arrowhead(s) + text);
//! * the entity pick really selects the arc/circle, and a **fillet arc** (an
//!   `Arc`) counts;
//! * `DIMCEN > 0` a crossed centre mark, `< 0` centrelines past the circle, `= 0`
//!   **nothing** with a diagnostic and no undo entry;
//! * the produced DIMCENTER entities are ordinary geometry, **not** a `Dimension`;
//! * every refusal (non-arc/circle pick, off-plane entity, off-plane location, a
//!   location at the centre, non-finite location, mismatched payload, another
//!   document, a missing layer, Viewer mode) is explicit and changes nothing;
//! * one transaction / one undo step per command;
//! * a created radius and diameter dimension survive an export → re-import with
//!   the same kind and definition points.

use super::*;

use cad_import_acadrust::export::{export, ExportFormat, ExportLimits, ExportRequest, ExportSpace};
use cad_import_acadrust::{AcadrustImporter, ImportLimits, ImportRequest, Importer};

const CIRCLE_ID: u128 = 1;
const ARC_ID: u128 = 2;
const LINE_ID: u128 = 3;
const OFF_PLANE_ID: u128 = 4;

/// A drawing with a circle (r 5), an arc (r 4, a fillet-like entity), a plain
/// line, an off-plane circle, and a `Standard` DIMSTYLE with `center_mark =
/// dimcen`. Layers 0 and 3 exist so the active-layer path can be exercised.
fn radial_app(dimcen: f64) -> (Application, SessionState) {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    for (id, name) in [(LayerId(0), "0"), (LayerId(3), "WALLS")] {
        b.insert_layer(Layer {
            id,
            name: name.into(),
            visible: true,
        })
        .unwrap();
    }
    b.insert_entity(ent(
        CIRCLE_ID,
        SemanticGeometry::Circle {
            center: point(0.0, 0.0),
            normal: p3(0.0, 0.0, 1.0),
            radius: 5.0,
        },
        0,
    ))
    .unwrap();
    b.insert_entity(ent(
        ARC_ID,
        SemanticGeometry::Arc {
            center: point(10.0, 0.0),
            normal: p3(0.0, 0.0, 1.0),
            radius: 4.0,
            start: 0.0,
            sweep: 1.5,
        },
        1,
    ))
    .unwrap();
    b.insert_entity(ent(
        LINE_ID,
        SemanticGeometry::Line {
            start: point(0.0, 0.0),
            end: point(2.0, 0.0),
        },
        2,
    ))
    .unwrap();
    // An XY-parallel-plane violation: the circle's normal has an X component.
    b.insert_entity(ent(
        OFF_PLANE_ID,
        SemanticGeometry::Circle {
            center: point(0.0, 20.0),
            normal: p3(1.0, 0.0, 0.0),
            radius: 3.0,
        },
        3,
    ))
    .unwrap();
    b.insert_dim_style(cad_db::DimStyle {
        center_mark: dimcen,
        ..cad_db::DimStyle::standard_metric(DimStyleId(0))
    })
    .unwrap();
    let drawing = b.finish().unwrap();

    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([9u8; 32]),
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

/// The stored dimension for a created object, asserting the type key.
fn dimension(app: &Application, object: ObjectId) -> DimensionGeometry {
    let entity = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(object.0))
        .expect("the dimension entity");
    assert_eq!(entity.object.type_key, "AcDbDimension");
    match &entity.geometry {
        SemanticGeometry::Dimension(dimension) => (**dimension).clone(),
        other => panic!("expected a dimension, got {other:?}"),
    }
}

/// The `Radial` definition points `(group 10, group 15, group 40)`.
fn radial_points(dimension: &DimensionGeometry) -> (Point3, Point3, f64) {
    match &dimension.definition().points {
        DimensionPoints::Radial {
            center,
            point_on_arc,
            leader_length,
        } => (*center, *point_on_arc, *leader_length),
        other => panic!("expected Radial points, got {other:?}"),
    }
}

/// The drawn measurement text (the synthesis appends it last).
fn drawn_text(dimension: &DimensionGeometry) -> String {
    dimension
        .display()
        .iter()
        .rev()
        .find_map(|child| match child {
            SemanticGeometry::Text { text, .. } => Some(text.clone()),
            _ => None,
        })
        .expect("a measurement Text child")
}

/// The lines of a dimension's display as `(start, end)` pairs.
fn display_lines(dimension: &DimensionGeometry) -> Vec<(Point3, Point3)> {
    dimension
        .display()
        .iter()
        .filter_map(|child| match child {
            SemanticGeometry::Line { start, end } => Some((*start, *end)),
            _ => None,
        })
        .collect()
}

fn radial_payload(target: u128, location: Point3) -> CommandPayload {
    CommandPayload::DimRadial(Box::new(crate::RadialDimensionCommand {
        target: ref_of(target),
        location,
    }))
}

fn approx(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

/// Euclidean distance (the crate-internal `distance` helpers are not reachable
/// from this test module).
fn distance(a: Point3, b: Point3) -> f64 {
    (a.x - b.x).hypot(a.y - b.y).hypot(a.z - b.z)
}

#[test]
fn dimradius_definition_points_measurement_and_display() {
    let (mut app, mut session) = radial_app(0.0);
    // The location pick points along +Y, so the leader meets the circle at (0,5).
    let out = execute(
        &mut app,
        &mut session,
        CommandId::DimRadius,
        radial_payload(CIRCLE_ID, point(0.0, 6.0)),
    )
    .unwrap();
    let dimension = dimension(&app, out.objects[0]);
    assert_eq!(dimension.kind(), DimensionKind::Radius);
    let (group10, group15, leader) = radial_points(&dimension);
    assert_eq!(group10, point(0.0, 0.0), "group 10 is the circle centre");
    assert_eq!(
        group15,
        point(0.0, 5.0),
        "group 15 is the point on the circle"
    );
    assert!(
        approx(leader, 0.0),
        "group 40 leader length is the neutral 0"
    );
    assert!(
        approx(dimension.measurement(), 5.0),
        "a radius dimension measures the radius: {}",
        dimension.measurement()
    );
    // The display: a leader line from the centre to the point on the circle, one
    // arrowhead, and the measurement text.
    assert_eq!(
        display_lines(&dimension),
        vec![(point(0.0, 0.0), point(0.0, 5.0))],
        "one leader line from the centre to the point on the circle"
    );
    assert_eq!(
        dimension
            .display()
            .iter()
            .filter(|child| matches!(child, SemanticGeometry::Mesh(_)))
            .count(),
        1,
        "a radius dimension has one arrowhead"
    );
    assert_eq!(drawn_text(&dimension), "R5");
}

#[test]
fn dimdiameter_uses_the_opposite_chord_point_and_measures_the_diameter() {
    let (mut app, mut session) = radial_app(0.0);
    let out = execute(
        &mut app,
        &mut session,
        CommandId::DimDiameter,
        radial_payload(CIRCLE_ID, point(6.0, 0.0)),
    )
    .unwrap();
    let dimension = dimension(&app, out.objects[0]);
    assert_eq!(dimension.kind(), DimensionKind::Diameter);
    let (group10, group15, _) = radial_points(&dimension);
    assert_eq!(
        group10,
        point(-5.0, 0.0),
        "group 10 is the opposite chord point, not the centre"
    );
    assert_eq!(group15, point(5.0, 0.0), "group 15 is the near chord point");
    assert!(
        approx(dimension.measurement(), 10.0),
        "a diameter dimension measures the diameter: {}",
        dimension.measurement()
    );
    assert_eq!(
        display_lines(&dimension),
        vec![(point(-5.0, 0.0), point(5.0, 0.0))],
        "the leader spans the full diameter"
    );
    assert_eq!(
        dimension
            .display()
            .iter()
            .filter(|child| matches!(child, SemanticGeometry::Mesh(_)))
            .count(),
        2,
        "a diameter dimension has two arrowheads"
    );
    assert_eq!(drawn_text(&dimension), "\u{2300}10");
}

#[test]
fn radial_entity_pick_selects_an_arc_and_a_fillet_arc_counts() {
    // The arc entity (id 2) is a fillet-like Arc; picking it must work, and the
    // leader must meet the arc's circle at the picked direction (centre (10,0)).
    let (mut app, mut session) = radial_app(0.0);
    let out = execute(
        &mut app,
        &mut session,
        CommandId::DimRadius,
        radial_payload(ARC_ID, point(10.0, 6.0)),
    )
    .unwrap();
    let dimension = dimension(&app, out.objects[0]);
    let (group10, group15, _) = radial_points(&dimension);
    assert_eq!(group10, point(10.0, 0.0), "the arc's centre is group 10");
    assert_eq!(
        group15,
        point(10.0, 4.0),
        "the point lies on the arc's circle"
    );
    assert!(approx(dimension.measurement(), 4.0));
}

#[test]
fn dimcenter_positive_dimcen_draws_the_crossed_marks() {
    let (mut app, mut session) = radial_app(2.5);
    let before = revision(&app);
    let out = execute(
        &mut app,
        &mut session,
        CommandId::DimCenter,
        CommandPayload::DimCenter(ref_of(CIRCLE_ID)),
    )
    .unwrap();
    assert_eq!(
        out.objects.len(),
        2,
        "a centre mark is a crossed pair of lines"
    );
    let changes = out.changes.expect("change set");
    assert_eq!(changes.before, before);
    assert_eq!(changes.after.0, before.0 + 1, "exactly one revision bump");
    assert_eq!(changes.changes.len(), 2, "two inserted entities");
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 1, "one undo step");

    // Both entities are ordinary LINEs crossing the centre, arms of length 2.5.
    let mut axes = Vec::new();
    for object in &out.objects {
        let entity = app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity(EntityId(object.0))
            .expect("a centre-mark entity");
        assert_eq!(entity.object.type_key, "AcDbLine", "plain geometry");
        match &entity.geometry {
            SemanticGeometry::Line { start, end } => {
                assert!(
                    approx(distance(*start, *end), 5.0),
                    "an arm spans 2 * DIMCEN"
                );
                axes.push((*start, *end));
            }
            other => panic!("expected a Line, got {other:?}"),
        }
    }
    assert!(
        axes.iter()
            .any(|(s, e)| approx(s.x, -2.5) && approx(e.x, 2.5)),
        "the X axis mark spans [-2.5, 2.5]"
    );
    assert!(
        axes.iter()
            .any(|(s, e)| approx(s.y, -2.5) && approx(e.y, 2.5)),
        "the Y axis mark spans [-2.5, 2.5]"
    );
}

#[test]
fn dimcenter_negative_dimcen_draws_centrelines_past_the_circle() {
    let (mut app, mut session) = radial_app(-2.5);
    let out = execute(
        &mut app,
        &mut session,
        CommandId::DimCenter,
        CommandPayload::DimCenter(ref_of(CIRCLE_ID)),
    )
    .unwrap();
    assert_eq!(
        out.objects.len(),
        2,
        "centrelines are a crossed pair of lines"
    );
    // The reach is radius + |DIMCEN| = 5 + 2.5 = 7.5, i.e. past the circle. The
    // AutoCAD "gapped centrelines" variant is NOT modelled: we draw a plain cross
    // (the same declared approximation as the radial synthesis).
    for object in &out.objects {
        let entity = app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity(EntityId(object.0))
            .expect("a centreline entity");
        match &entity.geometry {
            SemanticGeometry::Line { start, end } => {
                assert!(
                    approx(distance(*start, *end), 15.0),
                    "a centreline spans 2 * (radius + |DIMCEN|)"
                );
                let reach = start.x.abs().max(start.y.abs());
                assert!(
                    approx(reach, 7.5),
                    "reaches 7.5 from the centre, got {reach}"
                );
            }
            other => panic!("expected a Line, got {other:?}"),
        }
    }
}

#[test]
fn dimcenter_zero_dimcen_produces_nothing_with_a_diagnostic_and_no_undo() {
    let (mut app, mut session) = radial_app(0.0);
    let before = revision(&app);
    let out = execute(
        &mut app,
        &mut session,
        CommandId::DimCenter,
        CommandPayload::DimCenter(ref_of(CIRCLE_ID)),
    )
    .unwrap();
    assert!(out.objects.is_empty(), "DIMCEN=0 creates no entity");
    assert!(out.changes.is_none(), "DIMCEN=0 opens no transaction");
    assert!(
        out.diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("DIMCEN is 0")),
        "DIMCEN=0 must be reported explicitly, not silently: {:?}",
        out.diagnostics
    );
    assert_eq!(revision(&app), before, "nothing is written");
    assert!(
        !app.can_undo(&DocumentId(1)),
        "DIMCEN=0 must not record an undo entry"
    );
}

#[test]
fn dimcenter_entities_are_ordinary_geometry_not_a_dimension() {
    let (mut app, mut session) = radial_app(2.5);
    let out = execute(
        &mut app,
        &mut session,
        CommandId::DimCenter,
        CommandPayload::DimCenter(ref_of(CIRCLE_ID)),
    )
    .unwrap();
    for object in &out.objects {
        let entity = app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity(EntityId(object.0))
            .expect("a centre-mark entity");
        assert_eq!(entity.object.type_key, "AcDbLine");
        assert!(
            !matches!(entity.geometry, SemanticGeometry::Dimension(_)),
            "DIMCENTER must not produce a Dimension entity"
        );
    }
}

#[test]
fn radial_and_center_refusals_are_explicit_and_change_nothing() {
    let (mut app, mut session) = radial_app(2.5);
    let before = revision(&app);
    let cases: Vec<(CommandId, CommandPayload, &str)> = vec![
        // A pick that is not an arc/circle.
        (
            CommandId::DimRadius,
            radial_payload(LINE_ID, point(1.0, 1.0)),
            "arc or circle",
        ),
        (
            CommandId::DimDiameter,
            radial_payload(LINE_ID, point(1.0, 1.0)),
            "arc or circle",
        ),
        (
            CommandId::DimCenter,
            CommandPayload::DimCenter(ref_of(LINE_ID)),
            "arc or circle",
        ),
        // An off-plane entity (normal has an X component).
        (
            CommandId::DimRadius,
            radial_payload(OFF_PLANE_ID, point(0.0, 26.0)),
            "XY plane",
        ),
        (
            CommandId::DimCenter,
            CommandPayload::DimCenter(ref_of(OFF_PLANE_ID)),
            "XY plane",
        ),
        // A location at the centre (no direction).
        (
            CommandId::DimRadius,
            radial_payload(CIRCLE_ID, point(0.0, 0.0)),
            "differ from the arc/circle centre",
        ),
        // An off-plane location.
        (
            CommandId::DimRadius,
            radial_payload(CIRCLE_ID, p3(0.0, 6.0, 5.0)),
            "plane",
        ),
        // A non-finite location.
        (
            CommandId::DimRadius,
            radial_payload(CIRCLE_ID, point(f64::NAN, 6.0)),
            "finite",
        ),
        // A mismatched payload.
        (
            CommandId::DimRadius,
            CommandPayload::Points(vec![point(0.0, 0.0)]),
            "DimRadial",
        ),
        (
            CommandId::DimCenter,
            CommandPayload::Points(vec![point(0.0, 0.0)]),
            "DimCenter",
        ),
    ];
    for (id, payload, needle) in cases {
        let error = execute(&mut app, &mut session, id, payload).unwrap_err();
        let text = format!("{error:?}");
        assert!(
            text.contains(needle),
            "{id:?}: expected {needle:?} in {text}"
        );
    }
    assert_eq!(revision(&app), before, "a refused command must not write");
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn radial_target_from_another_document_is_refused() {
    let (mut app, mut session) = radial_app(0.0);
    let before = revision(&app);
    let mut target = ref_of(CIRCLE_ID);
    target.document = DocumentId(99);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::DimRadius,
        CommandPayload::DimRadial(Box::new(crate::RadialDimensionCommand {
            target,
            location: point(0.0, 6.0),
        })),
    )
    .unwrap_err();
    assert!(
        format!("{error:?}").contains("another document"),
        "{error:?}"
    );
    assert_eq!(revision(&app), before);
}

#[test]
fn radial_and_center_on_a_missing_layer_are_refused_by_the_shared_funnel() {
    let (mut app, mut session) = radial_app(2.5);
    let before = revision(&app);
    execute(
        &mut app,
        &mut session,
        CommandId::SetActiveLayer,
        CommandPayload::ActiveLayer(LayerId(99)),
    )
    .unwrap();
    let radial = execute(
        &mut app,
        &mut session,
        CommandId::DimRadius,
        radial_payload(CIRCLE_ID, point(0.0, 6.0)),
    )
    .unwrap_err();
    assert!(matches!(radial, CadError::InvalidInput(_)), "{radial:?}");
    let center = execute(
        &mut app,
        &mut session,
        CommandId::DimCenter,
        CommandPayload::DimCenter(ref_of(CIRCLE_ID)),
    )
    .unwrap_err();
    assert!(matches!(center, CadError::InvalidInput(_)), "{center:?}");
    assert_eq!(revision(&app), before);
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn radial_and_center_are_work_only_and_refused_in_viewer_mode() {
    assert!(CommandId::DimRadius.requires_work_mode());
    assert!(CommandId::DimDiameter.requires_work_mode());
    assert!(CommandId::DimCenter.requires_work_mode());
    let (mut app, mut session) = radial_app(2.5);
    session.switch_mode(AppMode::Viewer).unwrap();
    let before = revision(&app);
    for (id, payload) in [
        (
            CommandId::DimRadius,
            radial_payload(CIRCLE_ID, point(0.0, 6.0)),
        ),
        (
            CommandId::DimCenter,
            CommandPayload::DimCenter(ref_of(CIRCLE_ID)),
        ),
    ] {
        let error = execute(&mut app, &mut session, id, payload).unwrap_err();
        assert!(matches!(error, CadError::PermissionDenied), "{error:?}");
    }
    assert_eq!(revision(&app), before);
}

#[test]
fn radial_dimension_is_on_the_active_layer_in_one_transaction_and_one_undo_step() {
    let (mut app, mut session) = radial_app(0.0);
    execute(
        &mut app,
        &mut session,
        CommandId::SetActiveLayer,
        CommandPayload::ActiveLayer(LayerId(3)),
    )
    .unwrap();
    let before = revision(&app);
    let out = execute(
        &mut app,
        &mut session,
        CommandId::DimDiameter,
        radial_payload(CIRCLE_ID, point(0.0, 6.0)),
    )
    .unwrap();
    assert_eq!(out.objects.len(), 1, "one entity for one dimension");
    let changes = out.changes.expect("change set");
    assert_eq!(changes.before, before);
    assert_eq!(changes.after.0, before.0 + 1, "exactly one revision bump");
    assert_eq!(changes.changes.len(), 1, "one entity");
    let entity = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(out.objects[0].0))
        .expect("the dimension entity");
    assert_eq!(entity.layer, LayerId(3), "on the active layer");
    assert_eq!(entity.object.type_key, "AcDbDimension");
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 1, "one undo step");
}

#[test]
fn created_radial_dimensions_round_trip_through_the_dxf_export() {
    for (id, location, expected_kind, expected_measurement) in [
        (
            CommandId::DimRadius,
            point(0.0, 6.0),
            DimensionKind::Radius,
            5.0_f64,
        ),
        (
            CommandId::DimDiameter,
            point(6.0, 0.0),
            DimensionKind::Diameter,
            10.0,
        ),
    ] {
        let (mut app, mut session) = radial_app(0.0);
        execute(
            &mut app,
            &mut session,
            id,
            radial_payload(CIRCLE_ID, location),
        )
        .unwrap();
        let stored = app.workspace.documents[&DocumentId(1)].drawing.clone();
        let before = stored
            .entities()
            .filter_map(|entity| match &entity.geometry {
                SemanticGeometry::Dimension(dimension) => Some((**dimension).clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(before.len(), 1, "one created dimension");

        let (bytes, report) = export(&ExportRequest {
            database: &stored,
            document_id: DocumentId(1),
            space: ExportSpace::Model,
            format: ExportFormat::DxfText,
            limits: ExportLimits::default(),
            units: Some(UnitContext::drawing_units()),
        })
        .expect("export succeeds");
        assert!(
            report.counts.converted >= 1,
            "the created dimension is recorded as converted"
        );

        let request = ImportRequest {
            document: DocumentId(1),
            database: DatabaseId(1),
            bytes: Arc::from(bytes.into_boxed_slice()),
            limits: ImportLimits::default(),
            generation: 0,
        };
        let imported = AcadrustImporter::new()
            .import(&request, &|| false)
            .expect("re-import succeeds");
        let after = imported
            .database
            .entities()
            .filter_map(|entity| match &entity.geometry {
                SemanticGeometry::Dimension(dimension) => Some((**dimension).clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            after.len(),
            1,
            "the created dimension survives as a Dimension"
        );
        assert_eq!(after[0].kind(), expected_kind);
        assert!(
            approx(after[0].measurement(), expected_measurement),
            "the measurement survives: {}",
            after[0].measurement()
        );
        assert_eq!(
            radial_points(&after[0]),
            radial_points(&before[0]),
            "the definition points survive the round trip"
        );
    }
}
