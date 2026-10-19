//! DIMANGULAR / DIMORDINATE create-command contract tests (ADR 0017 decision C,
//! P5e).
//!
//! The numeric contracts pinned here:
//!
//! * the two-line angular form measures the angle whose sector contains the
//!   dimension-arc location (two different locations on the same two lines select
//!   45 vs 135 degrees), and its definition points are groups 13/14 (line 1),
//!   15/10 (line 2) and group 16 (arc location);
//! * the three-point angular form (command payload) measures the sector the
//!   location selects, including the reflex angle, with the vertex/side points in
//!   groups 15/13/14;
//! * the ordinate measurement is the feature's world-origin-relative X or Y
//!   **magnitude**, the auto axis follows "a greater Y difference measures X",
//!   the unstated equal case is Y (our declared choice), and a forced axis wins;
//! * each refusal by name (non-line pick, parallel/collinear lines, coincident
//!   points, off-plane input, zero-length leader, missing layer, Viewer mode) and
//!   one transaction / one undo step;
//! * a created angular (both forms) and ordinate dimension survives an export →
//!   re-import with the same kind and definition points.

use super::*;

use cad_import_acadrust::export::{export, ExportFormat, ExportLimits, ExportRequest, ExportSpace};
use cad_import_acadrust::{AcadrustImporter, ImportLimits, ImportRequest, Importer};

const LINE1_ID: u128 = 1;
const LINE2_ID: u128 = 2;
const PARALLEL_ID: u128 = 3;
const ARC_ID: u128 = 4;
const OFF_PLANE_ID: u128 = 5;

/// A drawing with two crossing lines (45-degree opening) and the refusal
/// fixtures, plus a pinned `Standard` DIMSTYLE.
fn angular_app() -> (Application, SessionState) {
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
        LINE1_ID,
        SemanticGeometry::Line {
            start: point(0.0, 0.0),
            end: point(10.0, 0.0),
        },
        0,
    ))
    .unwrap();
    b.insert_entity(ent(
        LINE2_ID,
        SemanticGeometry::Line {
            start: point(0.0, 0.0),
            end: point(5.0, 5.0),
        },
        1,
    ))
    .unwrap();
    b.insert_entity(ent(
        PARALLEL_ID,
        SemanticGeometry::Line {
            start: point(0.0, 2.0),
            end: point(10.0, 2.0),
        },
        2,
    ))
    .unwrap();
    b.insert_entity(ent(
        ARC_ID,
        SemanticGeometry::Arc {
            center: point(20.0, 0.0),
            normal: p3(0.0, 0.0, 1.0),
            radius: 3.0,
            start: 0.0,
            sweep: 1.0,
        },
        3,
    ))
    .unwrap();
    // A line whose far endpoint leaves the +Z plane.
    b.insert_entity(ent(
        OFF_PLANE_ID,
        SemanticGeometry::Line {
            start: point(0.0, 5.0),
            end: p3(10.0, 5.0, 3.0),
        },
        4,
    ))
    .unwrap();
    b.insert_dim_style(cad_db::DimStyle::standard_metric(DimStyleId(0)))
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
fn stored_dimension(app: &Application, object: ObjectId) -> DimensionGeometry {
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

fn two_line_payload(first: u128, second: u128, location: Point3) -> CommandPayload {
    CommandPayload::DimAngular(Box::new(crate::AngularDimensionInput::TwoLine {
        first: ref_of(first),
        second: ref_of(second),
        location,
    }))
}

fn three_point_payload(
    vertex: Point3,
    first: Point3,
    second: Point3,
    location: Point3,
) -> CommandPayload {
    CommandPayload::DimAngular(Box::new(crate::AngularDimensionInput::ThreePoint {
        vertex,
        first,
        second,
        location,
    }))
}

fn ordinate_payload(
    feature: Point3,
    leader: Point3,
    axis: Option<crate::OrdinateAxis>,
) -> CommandPayload {
    CommandPayload::DimOrdinate(Box::new(crate::OrdinateDimensionCommand {
        feature,
        leader,
        axis,
    }))
}

fn approx(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

/// The stored angular definition points `(13, 14, 15, 10, 16)`.
fn angular_line_points(dimension: &DimensionGeometry) -> (Point3, Point3, Point3, Point3, Point3) {
    match &dimension.definition().points {
        DimensionPoints::Angular2Ln {
            line1_start,
            line1_end,
            line2_start,
            line2_end,
            arc_point,
        } => (
            *line1_start,
            *line1_end,
            *line2_start,
            *line2_end,
            *arc_point,
        ),
        other => panic!("expected Angular2Ln points, got {other:?}"),
    }
}

fn angular_three_point_points(dimension: &DimensionGeometry) -> (Point3, Point3, Point3, Point3) {
    match &dimension.definition().points {
        DimensionPoints::Angular3Pt {
            vertex,
            first,
            second,
            arc_point,
        } => (*vertex, *first, *second, *arc_point),
        other => panic!("expected Angular3Pt points, got {other:?}"),
    }
}

fn ordinate_points(dimension: &DimensionGeometry) -> (Point3, Point3, Point3) {
    match &dimension.definition().points {
        DimensionPoints::Ordinate {
            feature,
            leader_end,
            base,
        } => (*feature, *leader_end, *base),
        other => panic!("expected Ordinate points, got {other:?}"),
    }
}

#[test]
fn dimangular_two_line_measures_the_sector_containing_the_location_pick() {
    let (mut app, mut session) = angular_app();
    // Lines +X and +45 degrees from the origin. A location in the 0..45 sector
    // measures 45 degrees; one in the 45..180 sector measures 135 degrees.
    let out = execute(
        &mut app,
        &mut session,
        CommandId::DimAngular,
        two_line_payload(LINE1_ID, LINE2_ID, point(2.0, 0.5)),
    )
    .unwrap();
    let dimension = stored_dimension(&app, out.objects[0]);
    assert_eq!(dimension.kind(), DimensionKind::Angular2Ln);
    assert!(
        approx(dimension.measurement(), 45.0),
        "the 0..45 sector contains the pick: {}",
        dimension.measurement()
    );
    assert_eq!(
        angular_line_points(&dimension),
        (
            point(0.0, 0.0),
            point(10.0, 0.0),
            point(0.0, 0.0),
            point(5.0, 5.0),
            point(2.0, 0.5),
        ),
        "groups 13/14, 15/10 and 16 are the picked endpoints and the arc location"
    );
    let arc = dimension
        .display()
        .iter()
        .find_map(|child| match child {
            SemanticGeometry::Arc {
                center,
                start,
                sweep,
                ..
            } => Some((*center, *start, *sweep)),
            _ => None,
        })
        .expect("an angular arc child");
    assert_eq!(arc.0, point(0.0, 0.0), "the arc is centred on the vertex");
    assert!(approx(arc.1, 0.0) && approx(arc.2, std::f64::consts::FRAC_PI_4));
    assert_eq!(drawn_text(&dimension), "45\u{00B0}");

    let out = execute(
        &mut app,
        &mut session,
        CommandId::DimAngular,
        two_line_payload(LINE1_ID, LINE2_ID, point(-1.0, 3.0)),
    )
    .unwrap();
    let dimension = stored_dimension(&app, out.objects[0]);
    assert!(
        approx(dimension.measurement(), 135.0),
        "the 45..180 sector contains the pick: {}",
        dimension.measurement()
    );
    assert_eq!(drawn_text(&dimension), "135\u{00B0}");
}

#[test]
fn dimangular_three_point_form_measures_the_selected_sector_including_reflex() {
    let (mut app, mut session) = angular_app();
    let out = execute(
        &mut app,
        &mut session,
        CommandId::DimAngular,
        three_point_payload(
            point(0.0, 0.0),
            point(10.0, 0.0),
            point(0.0, 10.0),
            point(1.0, 1.0),
        ),
    )
    .unwrap();
    let dimension = stored_dimension(&app, out.objects[0]);
    assert_eq!(dimension.kind(), DimensionKind::Angular3Pt);
    assert!(approx(dimension.measurement(), 90.0));
    assert_eq!(
        angular_three_point_points(&dimension),
        (
            point(0.0, 0.0),
            point(10.0, 0.0),
            point(0.0, 10.0),
            point(1.0, 1.0),
        )
    );
    let out = execute(
        &mut app,
        &mut session,
        CommandId::DimAngular,
        three_point_payload(
            point(0.0, 0.0),
            point(10.0, 0.0),
            point(0.0, 10.0),
            point(-1.0, -1.0),
        ),
    )
    .unwrap();
    let dimension = stored_dimension(&app, out.objects[0]);
    assert!(
        approx(dimension.measurement(), 270.0),
        "a pick in the reflex sector measures 270 degrees: {}",
        dimension.measurement()
    );
    assert_eq!(drawn_text(&dimension), "270\u{00B0}");
}

#[test]
fn dimordinate_measures_the_datum_relative_magnitude_and_the_auto_axis() {
    let (mut app, mut session) = angular_app();
    // A vertical leader (greater Y difference) measures the X ordinate.
    let out = execute(
        &mut app,
        &mut session,
        CommandId::DimOrdinate,
        ordinate_payload(point(3.0, 4.0), point(3.0, 9.0), None),
    )
    .unwrap();
    let dimension = stored_dimension(&app, out.objects[0]);
    assert_eq!(
        dimension.kind(),
        DimensionKind::Ordinate { x_ordinate: true }
    );
    assert!(approx(dimension.measurement(), 3.0), "|x| = 3");
    assert_eq!(
        ordinate_points(&dimension),
        (point(3.0, 4.0), point(3.0, 9.0), point(0.0, 0.0)),
        "group 10 is the world-origin datum"
    );
    assert_eq!(drawn_text(&dimension), "3");

    // A horizontal leader measures the Y ordinate.
    let out = execute(
        &mut app,
        &mut session,
        CommandId::DimOrdinate,
        ordinate_payload(point(3.0, 4.0), point(9.0, 4.0), None),
    )
    .unwrap();
    let dimension = stored_dimension(&app, out.objects[0]);
    assert_eq!(
        dimension.kind(),
        DimensionKind::Ordinate { x_ordinate: false }
    );
    assert!(approx(dimension.measurement(), 4.0), "|y| = 4");

    // Equal differences are unstated by Autodesk; our declared rule is Y.
    let out = execute(
        &mut app,
        &mut session,
        CommandId::DimOrdinate,
        ordinate_payload(point(2.0, 1.0), point(6.0, 5.0), None),
    )
    .unwrap();
    let dimension = stored_dimension(&app, out.objects[0]);
    assert_eq!(
        dimension.kind(),
        DimensionKind::Ordinate { x_ordinate: false },
        "the equal case is our declared Y choice"
    );
    assert!(approx(dimension.measurement(), 1.0), "|y| = 1");

    // An explicit axis wins over the auto rule.
    let out = execute(
        &mut app,
        &mut session,
        CommandId::DimOrdinate,
        ordinate_payload(
            point(3.0, 4.0),
            point(9.0, 4.0),
            Some(crate::OrdinateAxis::X),
        ),
    )
    .unwrap();
    let dimension = stored_dimension(&app, out.objects[0]);
    assert_eq!(
        dimension.kind(),
        DimensionKind::Ordinate { x_ordinate: true },
        "the payload's forced axis is honoured"
    );
    assert!(approx(dimension.measurement(), 3.0), "|x| = 3");
}

#[test]
fn angular_and_ordinate_refusals_are_explicit_and_change_nothing() {
    let (mut app, mut session) = angular_app();
    let before = revision(&app);
    let cases: Vec<(CommandId, CommandPayload, &str)> = vec![
        // A pick that is not a straight line.
        (
            CommandId::DimAngular,
            two_line_payload(LINE1_ID, ARC_ID, point(1.0, 1.0)),
            "straight-line",
        ),
        // Parallel/collinear lines have no vertex.
        (
            CommandId::DimAngular,
            two_line_payload(LINE1_ID, PARALLEL_ID, point(1.0, 1.0)),
            "non-parallel, non-collinear",
        ),
        // An off-plane line endpoint.
        (
            CommandId::DimAngular,
            two_line_payload(LINE1_ID, OFF_PLANE_ID, point(1.0, 1.0)),
            "+Z plane",
        ),
        // A location at the intersection cannot select a sector.
        (
            CommandId::DimAngular,
            two_line_payload(LINE1_ID, LINE2_ID, point(0.0, 0.0)),
            "intersection",
        ),
        // A non-finite location.
        (
            CommandId::DimAngular,
            two_line_payload(LINE1_ID, LINE2_ID, point(f64::NAN, 1.0)),
            "finite",
        ),
        // The three-point form refuses coincident side rays and a location at the
        // vertex.
        (
            CommandId::DimAngular,
            three_point_payload(
                point(0.0, 0.0),
                point(1.0, 0.0),
                point(2.0, 0.0),
                point(0.5, 1.0),
            ),
            "same ray",
        ),
        (
            CommandId::DimAngular,
            three_point_payload(
                point(0.0, 0.0),
                point(1.0, 0.0),
                point(0.0, 1.0),
                point(0.0, 0.0),
            ),
            "vertex",
        ),
        // A mismatched payload.
        (
            CommandId::DimAngular,
            CommandPayload::Points(vec![point(0.0, 0.0)]),
            "DimAngular",
        ),
        // DIMORDINATE: a zero-length leader, an off-plane pair, a non-finite
        // point and a mismatched payload.
        (
            CommandId::DimOrdinate,
            ordinate_payload(point(1.0, 1.0), point(1.0, 1.0), None),
            "distinct",
        ),
        (
            CommandId::DimOrdinate,
            ordinate_payload(point(1.0, 1.0), p3(1.0, 5.0, 3.0), None),
            "+Z plane",
        ),
        (
            CommandId::DimOrdinate,
            ordinate_payload(point(f64::NAN, 1.0), point(1.0, 5.0), None),
            "finite",
        ),
        (
            CommandId::DimOrdinate,
            CommandPayload::Points(vec![point(0.0, 0.0)]),
            "DimOrdinate",
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
fn angular_and_ordinate_are_work_only_and_refused_in_viewer_mode() {
    assert!(CommandId::DimAngular.requires_work_mode());
    assert!(CommandId::DimOrdinate.requires_work_mode());
    let (mut app, mut session) = angular_app();
    session.switch_mode(AppMode::Viewer).unwrap();
    let before = revision(&app);
    for (id, payload) in [
        (
            CommandId::DimAngular,
            two_line_payload(LINE1_ID, LINE2_ID, point(1.0, 1.0)),
        ),
        (
            CommandId::DimOrdinate,
            ordinate_payload(point(1.0, 1.0), point(1.0, 5.0), None),
        ),
    ] {
        let error = execute(&mut app, &mut session, id, payload).unwrap_err();
        assert!(matches!(error, CadError::PermissionDenied), "{error:?}");
    }
    assert_eq!(revision(&app), before);
}

#[test]
fn angular_and_ordinate_land_on_the_active_layer_in_one_transaction_and_one_undo() {
    for (id, payload) in [
        (
            CommandId::DimAngular,
            two_line_payload(LINE1_ID, LINE2_ID, point(2.0, 0.5)),
        ),
        (
            CommandId::DimOrdinate,
            ordinate_payload(point(3.0, 4.0), point(3.0, 9.0), None),
        ),
    ] {
        let (mut app, mut session) = angular_app();
        execute(
            &mut app,
            &mut session,
            CommandId::SetActiveLayer,
            CommandPayload::ActiveLayer(LayerId(3)),
        )
        .unwrap();
        let before = revision(&app);
        let out = execute(&mut app, &mut session, id, payload).unwrap();
        assert_eq!(out.objects.len(), 1, "{id:?}: one entity");
        let changes = out.changes.expect("change set");
        assert_eq!(changes.before, before);
        assert_eq!(changes.after.0, before.0 + 1, "{id:?}: one revision bump");
        assert_eq!(changes.changes.len(), 1, "{id:?}: one inserted entity");
        let entity = app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity(EntityId(out.objects[0].0))
            .expect("the dimension entity");
        assert_eq!(entity.layer, LayerId(3), "{id:?}: active layer");
        assert_eq!(entity.object.type_key, "AcDbDimension");
        assert_eq!(
            app.history[&DocumentId(1)].undo_depth(),
            1,
            "{id:?}: one undo step"
        );
    }
}

#[test]
fn angular_and_ordinate_on_a_missing_layer_are_refused_by_the_shared_funnel() {
    let (mut app, mut session) = angular_app();
    execute(
        &mut app,
        &mut session,
        CommandId::SetActiveLayer,
        CommandPayload::ActiveLayer(LayerId(99)),
    )
    .unwrap();
    let before = revision(&app);
    for (id, payload) in [
        (
            CommandId::DimAngular,
            two_line_payload(LINE1_ID, LINE2_ID, point(2.0, 0.5)),
        ),
        (
            CommandId::DimOrdinate,
            ordinate_payload(point(3.0, 4.0), point(3.0, 9.0), None),
        ),
    ] {
        let error = execute(&mut app, &mut session, id, payload).unwrap_err();
        assert!(matches!(error, CadError::InvalidInput(_)), "{error:?}");
    }
    assert_eq!(revision(&app), before);
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn angular_line_pick_from_another_document_is_refused() {
    let (mut app, mut session) = angular_app();
    let before = revision(&app);
    let mut second = ref_of(LINE2_ID);
    second.document = DocumentId(99);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::DimAngular,
        CommandPayload::DimAngular(Box::new(crate::AngularDimensionInput::TwoLine {
            first: ref_of(LINE1_ID),
            second,
            location: point(2.0, 0.5),
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
fn created_angular_and_ordinate_dimensions_round_trip_through_the_dxf_export() {
    let cases: Vec<(&str, CommandId, CommandPayload)> = vec![
        (
            "two-line",
            CommandId::DimAngular,
            two_line_payload(LINE1_ID, LINE2_ID, point(2.0, 0.5)),
        ),
        (
            "three-point",
            CommandId::DimAngular,
            three_point_payload(
                point(0.0, 0.0),
                point(10.0, 0.0),
                point(0.0, 10.0),
                point(1.0, 1.0),
            ),
        ),
        (
            "ordinate",
            CommandId::DimOrdinate,
            ordinate_payload(point(3.0, 4.0), point(3.0, 9.0), None),
        ),
    ];
    for (label, id, payload) in cases {
        let (mut app, mut session) = angular_app();
        execute(&mut app, &mut session, id, payload).unwrap();
        let stored = app.workspace.documents[&DocumentId(1)].drawing.clone();
        let before = stored
            .entities()
            .filter_map(|entity| match &entity.geometry {
                SemanticGeometry::Dimension(dimension) => Some((**dimension).clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(before.len(), 1, "{label}: one created dimension");

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
            "{label}: recorded as converted"
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
        assert_eq!(after.len(), 1, "{label}: survives as a Dimension");
        assert_eq!(after[0].kind(), before[0].kind(), "{label}: same kind");
        assert_eq!(
            after[0].definition().points,
            before[0].definition().points,
            "{label}: the definition points survive the round trip"
        );
        assert!(
            approx(after[0].measurement(), before[0].measurement()),
            "{label}: the measurement survives: {} vs {}",
            after[0].measurement(),
            before[0].measurement()
        );
    }
}
