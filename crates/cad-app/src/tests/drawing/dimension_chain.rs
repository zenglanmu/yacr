//! DIMBASELINE / DIMCONTINUE create-command contract tests (ADR 0017 decision C,
//! P5f — the 2026-10-21 un-gating).
//!
//! The numeric contracts pinned here:
//!
//! * **DIMBASELINE** row `i` (`i = 1..=N`, pick order) is
//!   `first = base.first`, `second = P_i`, `rotation = base.rotation`, and its
//!   dimension line sits at `base.location + n̂ · (i · DIMDLI)` where `n̂` is the
//!   base normal pointing **away from that row's measured geometry**;
//! * the stacking normal **flips** when the base dimension line is on the other
//!   side of the geometry;
//! * **DIMCONTINUE** chains `base.second → P_1 → … → P_N` on the base's own
//!   dimension line (`base.location`, no stacking), with the per-segment
//!   measurements equal to the projections along the base direction;
//! * `DIMDLI` (`DimensionStyleValues::dim_line_increment`) is the row spacing and
//!   is honoured from the style; it **never** changes a dimension's display;
//! * all N rows land in **one transaction / one undo step** on the active layer,
//!   and one undo removes every row;
//! * every refusal is named, opens no transaction and writes nothing;
//! * the created rows survive an export → re-import as real `DIMENSION` entities.

use super::*;

use cad_import_acadrust::export::{export, ExportFormat, ExportLimits, ExportRequest, ExportSpace};
use cad_import_acadrust::{AcadrustImporter, ImportLimits, ImportRequest, Importer};

const BASE_ID: u128 = 601;
const LINE_ID: u128 = 602;
const ALIGNED_ID: u128 = 603;
const RADIAL_ID: u128 = 604;
const ORDINATE_ID: u128 = 605;

/// The pinned `Standard` style resolved for a fixture dimension.
fn standard_style() -> DimensionStyleValues {
    cad_db::DimStyle::standard_metric(DimStyleId(0)).resolved(
        "Standard".to_string(),
        StyleId(0),
        None,
    )
}

/// A synthesized dimension entity, built through the **same** constructor the
/// create commands use (so the fixture is a legal base, not a hand-rolled struct).
fn synthesized(kind: DimensionKind, points: DimensionPoints) -> DimensionGeometry {
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
    .expect("the fixture dimension synthesizes")
}

fn linear_entity(first: Point3, second: Point3, location: Point3) -> SemanticGeometry {
    SemanticGeometry::Dimension(Box::new(synthesized(
        DimensionKind::Linear { rotation: 0.0 },
        DimensionPoints::Linear {
            first,
            second,
            dimension_line: location,
        },
    )))
}

/// A drawing with one **horizontal linear base** dimension (its dimension line
/// 10 units below the measured segment), one plain line and one base of every
/// other dimension kind, plus a `Standard` DIMSTYLE carrying `dimdli`.
fn chain_app(dimdli: f64, base_location: Point3) -> (Application, SessionState) {
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
        BASE_ID,
        linear_entity(point(0.0, 0.0), point(10.0, 0.0), base_location),
        0,
    ))
    .unwrap();
    b.insert_entity(ent(
        LINE_ID,
        SemanticGeometry::Line {
            start: point(0.0, 50.0),
            end: point(10.0, 50.0),
        },
        1,
    ))
    .unwrap();
    b.insert_entity(ent(
        ALIGNED_ID,
        SemanticGeometry::Dimension(Box::new(synthesized(
            DimensionKind::Aligned,
            DimensionPoints::Linear {
                first: point(0.0, 60.0),
                second: point(10.0, 70.0),
                dimension_line: point(0.0, 70.0),
            },
        ))),
        2,
    ))
    .unwrap();
    b.insert_entity(ent(
        RADIAL_ID,
        SemanticGeometry::Dimension(Box::new(synthesized(
            DimensionKind::Radius,
            DimensionPoints::Radial {
                center: point(40.0, 0.0),
                point_on_arc: point(45.0, 0.0),
                leader_length: 0.0,
            },
        ))),
        3,
    ))
    .unwrap();
    b.insert_entity(ent(
        ORDINATE_ID,
        SemanticGeometry::Dimension(Box::new(synthesized(
            DimensionKind::Ordinate { x_ordinate: true },
            DimensionPoints::Ordinate {
                feature: point(3.0, 4.0),
                leader_end: point(3.0, 9.0),
                base: point(0.0, 0.0),
            },
        ))),
        4,
    ))
    .unwrap();
    let mut style = cad_db::DimStyle::standard_metric(DimStyleId(0));
    style.dim_line_increment = dimdli;
    b.insert_dim_style(style).unwrap();
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

/// The default fixture: the base's dimension line 10 units **below** the measured
/// segment (`y = -10`), so the stacking normal points to `-Y`.
fn default_app() -> (Application, SessionState) {
    chain_app(3.75, point(5.0, -10.0))
}

/// A chain payload for `base`.
fn payload(base: SelectionRef, points: Vec<Point3>) -> CommandPayload {
    CommandPayload::DimChain(Box::new(crate::DimensionChainCommand { base, points }))
}

/// The stored dimension behind a created object, asserting the DXF type key.
fn stored(app: &Application, object: ObjectId) -> DimensionGeometry {
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

fn rotation_of(dimension: &DimensionGeometry) -> f64 {
    match dimension.kind() {
        DimensionKind::Linear { rotation } => rotation,
        other => panic!("expected a Linear kind, got {other:?}"),
    }
}

fn approx(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

fn undo_depth(app: &Application) -> usize {
    app.history
        .get(&DocumentId(1))
        .map(|history| history.undo_depth())
        .unwrap_or(0)
}

fn entity_count(app: &Application) -> usize {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity_count()
}

#[test]
fn dimbaseline_stacks_one_dimension_per_point_away_from_the_geometry() {
    let (mut app, mut session) = default_app();
    let before = revision(&app);
    let points = vec![point(20.0, 0.0), point(30.0, 0.0), point(45.0, 0.0)];
    let out = execute(
        &mut app,
        &mut session,
        CommandId::DimBaseline,
        payload(ref_of(BASE_ID), points.clone()),
    )
    .unwrap();
    assert_eq!(out.objects.len(), 3, "one dimension per picked point");
    let changes = out.changes.expect("change set");
    assert_eq!(changes.before, before);
    assert_eq!(changes.after.0, before.0 + 1, "one revision bump");

    for (index, object) in out.objects.iter().enumerate() {
        let row = index + 1;
        let dimension = stored(&app, *object);
        // Row `i`: the base's own first origin, the picked point, the base
        // rotation, and the base dimension line offset by `i · DIMDLI` along the
        // normal away from the measured geometry (-Y, because the base line sits
        // below it).
        let expected_line = point(5.0, -10.0 - 3.75 * row as f64);
        assert_eq!(
            linear_points(&dimension),
            (point(0.0, 0.0), points[index], expected_line),
            "row {row} definition"
        );
        assert!(
            rotation_of(&dimension).abs() < 1e-12,
            "row {row} keeps the base rotation"
        );
        assert!(
            approx(dimension.measurement(), points[index].x),
            "row {row} measures the projection along the base direction: {}",
            dimension.measurement()
        );
        // The row's extension lines reach from the measured points down to its
        // own stacked dimension line, where they run `DIMEXE` (1.25 in the pinned
        // metric `Standard`) past it — so the spacing is visible in the display.
        let extension_end_y = dimension
            .display()
            .iter()
            .filter_map(|child| match child {
                SemanticGeometry::Line { start, end } => Some(start.y.min(end.y)),
                _ => None,
            })
            .fold(f64::INFINITY, f64::min);
        assert!(
            approx(extension_end_y, expected_line.y - 1.25),
            "row {row} extension lines stop past its stacked line: {extension_end_y}"
        );
    }
}

#[test]
fn a_base_on_the_other_side_flips_the_stacking_normal() {
    // The same geometry, but the base's dimension line is now 10 units **above**
    // the measured segment: `n̂` must flip, so the rows stack to +Y.
    let (mut app, mut session) = chain_app(3.75, point(5.0, 10.0));
    let out = execute(
        &mut app,
        &mut session,
        CommandId::DimBaseline,
        payload(ref_of(BASE_ID), vec![point(20.0, 0.0)]),
    )
    .unwrap();
    let dimension = stored(&app, out.objects[0]);
    assert_eq!(
        linear_points(&dimension).2,
        point(5.0, 10.0 + 3.75),
        "the stacking normal follows the base's side"
    );
}

#[test]
fn dimcontinue_chains_segments_on_the_base_dimension_line() {
    let (mut app, mut session) = default_app();
    let points = vec![point(20.0, 0.0), point(35.0, 0.0), point(40.0, 0.0)];
    let out = execute(
        &mut app,
        &mut session,
        CommandId::DimContinue,
        payload(ref_of(BASE_ID), points.clone()),
    )
    .unwrap();
    assert_eq!(out.objects.len(), 3, "one dimension per segment");
    // Segment 1 starts at the base's **second** origin, then the chain follows
    // the pick order; every segment keeps the base dimension line (no stacking).
    let expected = [
        (point(10.0, 0.0), point(20.0, 0.0), 10.0),
        (point(20.0, 0.0), point(35.0, 0.0), 15.0),
        (point(35.0, 0.0), point(40.0, 0.0), 5.0),
    ];
    for (index, (first, second, measurement)) in expected.into_iter().enumerate() {
        let dimension = stored(&app, out.objects[index]);
        assert_eq!(
            linear_points(&dimension),
            (first, second, point(5.0, -10.0)),
            "segment {} definition",
            index + 1
        );
        assert!(
            approx(dimension.measurement(), measurement),
            "segment {} measures its projection: {}",
            index + 1,
            dimension.measurement()
        );
        assert!(rotation_of(&dimension).abs() < 1e-12);
    }
}

#[test]
fn the_style_dline_increment_sets_the_row_spacing() {
    // A non-default DIMDLI is honoured verbatim (DIMSCALE = 1 here).
    let (mut app, mut session) = chain_app(7.5, point(5.0, -10.0));
    let out = execute(
        &mut app,
        &mut session,
        CommandId::DimBaseline,
        payload(ref_of(BASE_ID), vec![point(20.0, 0.0), point(30.0, 0.0)]),
    )
    .unwrap();
    assert_eq!(
        linear_points(&stored(&app, out.objects[0])).2,
        point(5.0, -17.5),
        "row 1 is one DIMDLI below the base line"
    );
    assert_eq!(
        linear_points(&stored(&app, out.objects[1])).2,
        point(5.0, -25.0),
        "row 2 is two DIMDLI below the base line"
    );
    // The resolved snapshot carries the value, so the spacing is never a
    // hard-coded constant.
    assert!(approx(standard_style().dim_line_increment, 3.75));
    assert!(approx(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .resolve_dim_style("Standard")
            .expect("Standard")
            .dim_line_increment,
        7.5
    ));
}

#[test]
fn the_row_spacing_never_changes_a_dimensions_display() {
    // The ADR's declared one-off deviation: `dim_line_increment` is a **creation**
    // parameter. Two dimensions with the same definition and otherwise identical
    // styles therefore have byte-identical display children and measurement.
    let definition = DimensionDefinition {
        kind: DimensionKind::Linear { rotation: 0.0 },
        points: DimensionPoints::Linear {
            first: point(0.0, 0.0),
            second: point(20.0, 0.0),
            dimension_line: point(5.0, -13.75),
        },
        text_middle: point(0.0, 0.0),
        text_rotation: 0.0,
        normal: p3(0.0, 0.0, 1.0),
    };
    let narrow = standard_style();
    let mut wide = narrow.clone();
    wide.dim_line_increment = 250.0;
    let a =
        DimensionGeometry::synthesized(definition.clone(), "Standard".to_string(), narrow, None)
            .unwrap();
    let b = DimensionGeometry::synthesized(definition, "Standard".to_string(), wide, None).unwrap();
    assert_eq!(a.display(), b.display(), "display ignores DIMDLI");
    assert_eq!(a.measurement(), b.measurement());
}

#[test]
fn the_whole_batch_is_one_transaction_and_one_undo_step() {
    let (mut app, mut session) = default_app();
    let rows = entity_count(&app);
    let out = execute(
        &mut app,
        &mut session,
        CommandId::DimBaseline,
        payload(
            ref_of(BASE_ID),
            vec![point(20.0, 0.0), point(30.0, 0.0), point(45.0, 0.0)],
        ),
    )
    .unwrap();
    assert_eq!(out.objects.len(), 3);
    assert_eq!(entity_count(&app), rows + 3);
    assert_eq!(undo_depth(&app), 1, "one undo step for all three rows");
    // Every row is on the **active** layer.
    execute(
        &mut app,
        &mut session,
        CommandId::SetActiveLayer,
        CommandPayload::ActiveLayer(LayerId(3)),
    )
    .unwrap();
    let out = execute(
        &mut app,
        &mut session,
        CommandId::DimContinue,
        payload(ref_of(BASE_ID), vec![point(20.0, 0.0), point(30.0, 0.0)]),
    )
    .unwrap();
    for object in &out.objects {
        let entity = app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity(EntityId(object.0))
            .expect("the chained dimension");
        assert_eq!(entity.layer, LayerId(3));
    }
    assert_eq!(undo_depth(&app), 2);

    // One undo removes every row of the first batch, not just the last.
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(entity_count(&app), rows, "both batches are undone");
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn every_refusal_is_named_and_writes_nothing() {
    // A ref that carries an instance path (a block-instance member) and one that
    // carries a sub-element: neither is a directly addressable dimension.
    let mut instance_ref = ref_of(BASE_ID);
    instance_ref.instance = InstancePath(vec![EntityId(999)]);
    let mut sub_element_ref = ref_of(BASE_ID);
    sub_element_ref.sub_element = Some(SubElementId {
        source_key: "face".into(),
        topology_revision: Revision(0),
    });
    // A base from another document.
    let mut other_document = ref_of(BASE_ID);
    other_document.document = DocumentId(99);
    let mut off_plane = point(20.0, 0.0);
    off_plane.z = 3.0;

    let cases: Vec<(&str, CommandId, CommandPayload, &str)> = vec![
        (
            "non-dimension base",
            CommandId::DimBaseline,
            payload(ref_of(LINE_ID), vec![point(20.0, 0.0)]),
            "must be a linear dimension",
        ),
        (
            "aligned base",
            CommandId::DimBaseline,
            payload(ref_of(ALIGNED_ID), vec![point(20.0, 0.0)]),
            "an aligned dimension",
        ),
        (
            "radius base",
            CommandId::DimContinue,
            payload(ref_of(RADIAL_ID), vec![point(20.0, 0.0)]),
            "a radius dimension",
        ),
        (
            "ordinate base",
            CommandId::DimContinue,
            payload(ref_of(ORDINATE_ID), vec![point(20.0, 0.0)]),
            "an ordinate dimension",
        ),
        (
            "missing base",
            CommandId::DimBaseline,
            payload(ref_of(999), vec![point(20.0, 0.0)]),
            "does not exist",
        ),
        (
            "instance-path base",
            CommandId::DimBaseline,
            payload(instance_ref, vec![point(20.0, 0.0)]),
            "block-instance member",
        ),
        (
            "sub-element base",
            CommandId::DimContinue,
            payload(sub_element_ref, vec![point(20.0, 0.0)]),
            "sub-element",
        ),
        (
            "cross-document base",
            CommandId::DimBaseline,
            payload(other_document, vec![point(20.0, 0.0)]),
            "another document",
        ),
        (
            "zero points",
            CommandId::DimBaseline,
            payload(ref_of(BASE_ID), Vec::new()),
            "at least one picked point",
        ),
        (
            "non-finite point",
            CommandId::DimContinue,
            payload(ref_of(BASE_ID), vec![point(f64::NAN, 0.0)]),
            "must all be finite",
        ),
        (
            "off-plane point",
            CommandId::DimContinue,
            payload(ref_of(BASE_ID), vec![off_plane]),
            "+Z plane",
        ),
        (
            "degenerate baseline row",
            CommandId::DimBaseline,
            payload(ref_of(BASE_ID), vec![point(0.0, 0.0)]),
            "degenerate",
        ),
        (
            "non-advancing continue segment",
            CommandId::DimContinue,
            payload(ref_of(BASE_ID), vec![point(5.0, 0.0)]),
            "advance",
        ),
        (
            "zero-length continue segment",
            CommandId::DimContinue,
            payload(ref_of(BASE_ID), vec![point(10.0, 0.0)]),
            "advance",
        ),
    ];
    for (label, id, payload, reason) in cases {
        let (mut app, mut session) = default_app();
        let before = revision(&app);
        let error = execute(&mut app, &mut session, id, payload)
            .expect_err(&format!("{label} must be refused"));
        assert!(
            format!("{error:?}").contains(reason),
            "{label}: expected '{reason}' in {error:?}"
        );
        assert_eq!(revision(&app), before, "{label}: nothing written");
        assert_eq!(undo_depth(&app), 0, "{label}: no undo entry");
    }
}

#[test]
fn an_undecidable_stacking_side_is_refused_instead_of_guessed() {
    // The base's dimension line lies exactly on the measured segment, so the
    // dot product the ADR uses to pick the normal's sign is zero.
    let (mut app, mut session) = chain_app(3.75, point(5.0, 0.0));
    let before = revision(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::DimBaseline,
        payload(ref_of(BASE_ID), vec![point(20.0, 0.0)]),
    )
    .unwrap_err();
    assert!(format!("{error:?}").contains("cannot decide"), "{error:?}");
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn a_missing_active_layer_and_viewer_mode_are_refused_by_name() {
    // Missing layer: the shared insert funnel refuses after every input was
    // resolved, and opens no transaction.
    let (mut app, mut session) = default_app();
    execute(
        &mut app,
        &mut session,
        CommandId::SetActiveLayer,
        CommandPayload::ActiveLayer(LayerId(99)),
    )
    .unwrap();
    let before = revision(&app);
    for id in [CommandId::DimBaseline, CommandId::DimContinue] {
        let error = execute(
            &mut app,
            &mut session,
            id,
            payload(ref_of(BASE_ID), vec![point(20.0, 0.0)]),
        )
        .unwrap_err();
        assert!(matches!(error, CadError::InvalidInput(_)), "{error:?}");
        assert_eq!(revision(&app), before);
    }
    assert_eq!(undo_depth(&app), 0);

    // Viewer mode: Work-only, refused before anything is resolved.
    let (mut app, mut session) = default_app();
    session.switch_mode(AppMode::Viewer).unwrap();
    let before = revision(&app);
    for id in [CommandId::DimBaseline, CommandId::DimContinue] {
        let error = execute(
            &mut app,
            &mut session,
            id,
            payload(ref_of(BASE_ID), vec![point(20.0, 0.0)]),
        )
        .unwrap_err();
        assert!(matches!(error, CadError::PermissionDenied), "{error:?}");
        assert_eq!(revision(&app), before);
    }
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn a_base_deleted_after_the_pick_is_refused_at_apply_time() {
    let (mut app, mut session) = default_app();
    // Erase the base between the pick and the Confirm: the command re-reads it,
    // so a stale pick can never retarget or silently create a thin-air base.
    session.selection = SelectionSet::from_refs(vec![ref_of(BASE_ID)]);
    execute(
        &mut app,
        &mut session,
        CommandId::EraseEntities,
        CommandPayload::None,
    )
    .unwrap();
    let before = revision(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::DimBaseline,
        payload(ref_of(BASE_ID), vec![point(20.0, 0.0)]),
    )
    .unwrap_err();
    assert!(format!("{error:?}").contains("does not exist"), "{error:?}");
    assert_eq!(revision(&app), before);
}

#[test]
fn created_rows_round_trip_through_the_dxf_export() {
    let (mut app, mut session) = default_app();
    execute(
        &mut app,
        &mut session,
        CommandId::DimBaseline,
        payload(
            ref_of(BASE_ID),
            vec![point(20.0, 0.0), point(30.0, 0.0), point(45.0, 0.0)],
        ),
    )
    .unwrap();
    let stored_drawing = app.workspace.documents[&DocumentId(1)].drawing.clone();
    let before: Vec<DimensionGeometry> = stored_drawing
        .entities()
        .filter_map(|entity| match &entity.geometry {
            SemanticGeometry::Dimension(dimension) => Some((**dimension).clone()),
            _ => None,
        })
        .collect();
    // The fixture carries four dimensions and the command added three rows.
    assert_eq!(before.len(), 7);

    let (bytes, report) = export(&ExportRequest {
        database: &stored_drawing,
        document_id: DocumentId(1),
        space: ExportSpace::Model,
        format: ExportFormat::DxfText,
        limits: ExportLimits::default(),
        units: Some(UnitContext::drawing_units()),
    })
    .expect("export succeeds");
    assert!(report.counts.converted >= 7, "every dimension is converted");

    let imported = AcadrustImporter::new()
        .import(
            &ImportRequest {
                document: DocumentId(1),
                database: DatabaseId(1),
                bytes: Arc::from(bytes.into_boxed_slice()),
                limits: ImportLimits::default(),
                generation: 0,
            },
            &|| false,
        )
        .expect("re-import succeeds");
    let after: Vec<DimensionGeometry> = imported
        .database
        .entities()
        .filter_map(|entity| match &entity.geometry {
            SemanticGeometry::Dimension(dimension) => Some((**dimension).clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        after.len(),
        before.len(),
        "every dimension comes back as a real DIMENSION entity"
    );
    // The three created rows keep their kind, definition points and measurement.
    for row in before.iter().filter(|dimension| {
        matches!(dimension.kind(), DimensionKind::Linear { .. })
            && linear_points(dimension).2.y < -10.0
    }) {
        let found = after
            .iter()
            .find(|candidate| {
                candidate.kind() == row.kind()
                    && candidate.definition().points == row.definition().points
            })
            .unwrap_or_else(|| panic!("row {:?} did not survive", linear_points(row)));
        assert!(approx(found.measurement(), row.measurement()));
    }
    assert_eq!(
        before
            .iter()
            .filter(|dimension| linear_points_filtered(dimension))
            .count(),
        3,
        "the three created rows are stacked below the base"
    );
}

/// Whether a dimension's dimension line is one of the stacked rows (below the
/// fixture base line at `y = -10`).
fn linear_points_filtered(dimension: &DimensionGeometry) -> bool {
    match dimension.kind() {
        DimensionKind::Linear { .. } => {
            let (_, _, line) = linear_points(dimension);
            line.y < -10.0
        }
        _ => false,
    }
}
