//! DIMLINEAR / DIMALIGNED create-command contract tests (ADR 0017 decision C,
//! P5c).
//!
//! The three picks (two extension-line origins plus the dimension-line location)
//! become **one** `SemanticGeometry::Dimension` in one transaction / one undo
//! step on the active layer. The numeric contracts pinned here:
//!
//! * DIMLINEAR horizontal vs vertical is decided by the **location pick** (the
//!   third point), asserted by kind rotation and measurement for both;
//! * DIMALIGNED is parallel to the origin line and measures the true
//!   origin-to-origin distance (a 45° pair, so "aligned" ≠ "linear");
//! * the measurement equals the geometric distance and the drawn text agrees;
//! * the entity is on the active layer, in one transaction / one undo step;
//! * every refusal (off-plane, coincident origins, ambiguous direction, zero
//!   measurement, mismatched payload, non-finite point, missing layer, Viewer
//!   mode) is explicit and changes nothing;
//! * a **created** dimension survives an export → re-import as a `Dimension`
//!   with the same kind and definition points (the P5a export shape exercised
//!   with a created dimension for the first time).

use super::*;

use cad_import_acadrust::export::{export, ExportFormat, ExportLimits, ExportRequest, ExportSpace};
use cad_import_acadrust::{AcadrustImporter, ImportLimits, ImportRequest, Importer};

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

/// The `Linear` definition points `(first, second, dimension_line)`.
fn linear_points(dimension: &DimensionGeometry) -> (Point3, Point3, Point3) {
    match &dimension.definition().points {
        DimensionPoints::Linear {
            first,
            second,
            dimension_line,
        } => (*first, *second, *dimension_line),
        other => panic!("expected Linear points, got {other:?}"),
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

/// The group-50 rotation of a `Linear` kind.
fn linear_rotation(dimension: &DimensionGeometry) -> f64 {
    match dimension.kind() {
        DimensionKind::Linear { rotation } => rotation,
        other => panic!("expected a Linear kind, got {other:?}"),
    }
}

#[test]
fn dimlinear_horizontal_and_vertical_are_decided_by_the_location_pick() {
    let (mut app, mut session) = drawing_app();
    let first = point(0.0, 0.0);
    let second = point(10.0, 5.0);

    // A location offset more vertically than horizontally => horizontal (X),
    // rotation 0, measuring the X extent.
    let horizontal_location = point(5.0, 12.0);
    let out = execute(
        &mut app,
        &mut session,
        CommandId::DimLinear,
        CommandPayload::Points(vec![first, second, horizontal_location]),
    )
    .unwrap();
    let horizontal = dimension(&app, out.objects[0]);
    assert!(
        linear_rotation(&horizontal).abs() < 1e-12,
        "a vertical offset gives a horizontal dimension (rotation 0)"
    );
    assert_eq!(
        linear_points(&horizontal),
        (first, second, horizontal_location),
        "the definition points are the three clicks in order"
    );
    assert!(
        (horizontal.measurement() - 10.0).abs() < 1e-9,
        "a horizontal dimension measures the X extent: {}",
        horizontal.measurement()
    );

    // A location offset more horizontally than vertically => vertical (Y),
    // rotation π/2, measuring the Y extent.
    let vertical_location = point(20.0, 2.5);
    let out = execute(
        &mut app,
        &mut session,
        CommandId::DimLinear,
        CommandPayload::Points(vec![first, second, vertical_location]),
    )
    .unwrap();
    let vertical = dimension(&app, out.objects[0]);
    assert!(
        (linear_rotation(&vertical) - std::f64::consts::FRAC_PI_2).abs() < 1e-12,
        "a horizontal offset gives a vertical dimension (rotation π/2)"
    );
    assert_eq!(
        linear_points(&vertical),
        (first, second, vertical_location),
        "the definition points are the three clicks in order"
    );
    assert!(
        (vertical.measurement() - 5.0).abs() < 1e-9,
        "a vertical dimension measures the Y extent: {}",
        vertical.measurement()
    );
}

#[test]
fn dimlinear_third_pick_is_what_determines_horizontal_vs_vertical() {
    // The origins differ only in Y, so a dimension that always produced a
    // horizontal one would measure zero (and be refused). The left/right pick
    // must therefore select a **vertical** dimension and measure the 7-unit Y
    // extent.
    let (mut app, mut session) = drawing_app();
    let first = point(1.0, 2.0);
    let second = point(1.0, 9.0);
    let location = point(10.0, 5.5);
    let out = execute(
        &mut app,
        &mut session,
        CommandId::DimLinear,
        CommandPayload::Points(vec![first, second, location]),
    )
    .unwrap();
    let dimension = dimension(&app, out.objects[0]);
    assert!(
        (linear_rotation(&dimension) - std::f64::consts::FRAC_PI_2).abs() < 1e-12,
        "a left/right location pick must give a vertical dimension, got {:?}",
        dimension.kind()
    );
    assert!(
        (dimension.measurement() - 7.0).abs() < 1e-9,
        "a vertical dimension measures the Y extent: {}",
        dimension.measurement()
    );
}

#[test]
fn dimaligned_is_parallel_to_the_origin_line() {
    // A 45° origin pair, so "aligned" (the true distance) is genuinely distinct
    // from a horizontal/vertical linear dimension (which would measure an axis
    // extent).
    let (mut app, mut session) = drawing_app();
    let first = point(0.0, 0.0);
    let second = point(3.0, 3.0);
    let location = point(0.0, 3.0);
    let out = execute(
        &mut app,
        &mut session,
        CommandId::DimAligned,
        CommandPayload::Points(vec![first, second, location]),
    )
    .unwrap();
    let dimension = dimension(&app, out.objects[0]);
    assert_eq!(dimension.kind(), DimensionKind::Aligned);
    assert_eq!(
        linear_points(&dimension),
        (first, second, location),
        "the definition points are the three clicks in order"
    );
    let expected = (3.0_f64 * 3.0 + 3.0 * 3.0).sqrt();
    assert!(
        (dimension.measurement() - expected).abs() < 1e-9,
        "an aligned dimension measures the true origin-to-origin distance: {}",
        dimension.measurement()
    );
    assert!(
        (dimension.measurement() - 3.0).abs() > 1.0,
        "the aligned measurement must not collapse to an axis extent"
    );
}

#[test]
fn dimension_measurement_equals_the_geometric_distance_and_the_text_agrees() {
    let (mut app, mut session) = drawing_app();
    // A 3-4-5 leg: the origin-to-origin distance is exactly 5.
    let out = execute(
        &mut app,
        &mut session,
        CommandId::DimAligned,
        CommandPayload::Points(vec![point(0.0, 0.0), point(3.0, 4.0), point(-2.0, 2.0)]),
    )
    .unwrap();
    let dimension = dimension(&app, out.objects[0]);
    assert!((dimension.measurement() - 5.0).abs() < 1e-9);
    // Standard is DIMDEC=2 with DIMZIN bit 3 (trailing zeros suppressed), so the
    // shared formatting path draws "5".
    assert_eq!(drawn_text(&dimension), "5");
}

#[test]
fn created_dimension_is_on_the_active_layer_in_one_transaction_and_one_undo_step() {
    let (mut app, mut session) = application_with_document();
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
        CommandId::DimAligned,
        CommandPayload::Points(vec![point(0.0, 0.0), point(4.0, 0.0), point(0.0, 3.0)]),
    )
    .unwrap();
    assert_eq!(out.objects.len(), 1, "one entity for one dimension");
    let changes = out.changes.expect("change set");
    assert_eq!(changes.before, before);
    assert_eq!(changes.after.0, before.0 + 1, "exactly one revision bump");
    assert_eq!(changes.changes.len(), 1, "one transaction");

    let entity = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(out.objects[0].0))
        .expect("the dimension entity");
    assert_eq!(entity.layer, LayerId(3), "on the active layer");
    assert_eq!(entity.space, SpaceId::Model);
    assert_eq!(entity.object.type_key, "AcDbDimension");
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 1, "one undo step");
}

#[test]
fn dimension_refusals_are_explicit_and_change_nothing() {
    let (mut app, mut session) = drawing_app();
    let before = revision(&app);
    let cases: Vec<(CommandId, CommandPayload, &str)> = vec![
        (
            CommandId::DimLinear,
            CommandPayload::Points(vec![point(0.0, 0.0), point(1.0, 0.0)]),
            "three points",
        ),
        (
            CommandId::DimLinear,
            CommandPayload::Points(vec![point(0.0, 0.0), point(f64::NAN, 0.0), point(0.0, 1.0)]),
            "finite",
        ),
        // Off one `+Z` plane (ADR 0017 D.6).
        (
            CommandId::DimLinear,
            CommandPayload::Points(vec![point(0.0, 0.0), point(1.0, 0.0), p3(0.0, 1.0, 5.0)]),
            "plane",
        ),
        // Coincident origins.
        (
            CommandId::DimLinear,
            CommandPayload::Points(vec![point(1.0, 1.0), point(1.0, 1.0), point(2.0, 5.0)]),
            "distinct",
        ),
        // Ambiguous direction: the location is exactly the origins' midpoint.
        (
            CommandId::DimLinear,
            CommandPayload::Points(vec![point(0.0, 0.0), point(10.0, 0.0), point(5.0, 0.0)]),
            "direction",
        ),
        // Zero measurement: origins differ only in Y and the pick is more
        // vertical, so the chosen horizontal direction measures zero.
        (
            CommandId::DimLinear,
            CommandPayload::Points(vec![point(0.0, 0.0), point(0.0, 8.0), point(0.0, 20.0)]),
            "measures zero",
        ),
        // DIMALIGNED with coincident origins (no direction).
        (
            CommandId::DimAligned,
            CommandPayload::Points(vec![point(2.0, 2.0), point(2.0, 2.0), point(5.0, 5.0)]),
            "distinct",
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
    assert_eq!(revision(&app), before, "a refused dimension must not write");
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn dimension_on_a_missing_layer_is_refused_by_the_shared_funnel() {
    let (mut app, mut session) = drawing_app();
    let before = revision(&app);
    execute(
        &mut app,
        &mut session,
        CommandId::SetActiveLayer,
        CommandPayload::ActiveLayer(LayerId(99)),
    )
    .unwrap();
    let error = execute(
        &mut app,
        &mut session,
        CommandId::DimLinear,
        CommandPayload::Points(vec![point(0.0, 0.0), point(4.0, 0.0), point(0.0, 3.0)]),
    )
    .unwrap_err();
    assert!(matches!(error, CadError::InvalidInput(_)), "{error:?}");
    assert_eq!(revision(&app), before);
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn dimension_commands_are_work_only_and_refused_in_viewer_mode() {
    assert!(CommandId::DimLinear.requires_work_mode());
    assert!(CommandId::DimAligned.requires_work_mode());
    let (mut app, mut session) = drawing_app();
    session.switch_mode(AppMode::Viewer).unwrap();
    let before = revision(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::DimLinear,
        CommandPayload::Points(vec![point(0.0, 0.0), point(1.0, 0.0), point(0.0, 1.0)]),
    )
    .unwrap_err();
    assert!(matches!(error, CadError::PermissionDenied), "{error:?}");
    assert_eq!(revision(&app), before);
}

#[test]
fn created_dimension_round_trips_through_the_dxf_export() {
    let (mut app, mut session) = drawing_app();
    let first = point(0.0, 0.0);
    let second = point(3.0, 3.0);
    let location = point(0.0, 3.0);
    execute(
        &mut app,
        &mut session,
        CommandId::DimAligned,
        CommandPayload::Points(vec![first, second, location]),
    )
    .unwrap();

    let drawing = app.workspace.documents[&DocumentId(1)].drawing.clone();
    let (bytes, report) = export(&ExportRequest {
        database: &drawing,
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
    let dimensions: Vec<DimensionGeometry> = imported
        .database
        .entities()
        .filter_map(|entity| match &entity.geometry {
            SemanticGeometry::Dimension(dimension) => Some((**dimension).clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        dimensions.len(),
        1,
        "the created dimension survives as exactly one Dimension"
    );
    assert_eq!(dimensions[0].kind(), DimensionKind::Aligned);
    assert_eq!(
        linear_points(&dimensions[0]),
        (first, second, location),
        "the definition points survive the round trip"
    );
}
