//! HATCHPOINT (internal-point boundary discovery) contract tests.
//!
//! Pins the declared v1 behaviour: one picked internal point selects the
//! **smallest visible whole-entity closed boundary** containing it, nested closed
//! boundaries become islands, hidden / off-plane / different-elevation / other
//! kinds are skipped (never refused), and every ambiguous or empty pick is an
//! explicit refusal with no transaction. `LINE`/`ARC` chains and the current
//! selection are deliberately not used.

use super::*;

use super::hatch::{circle, entity_count, filled_area, square, the_hatch, undo_depth};
use cad_import_acadrust::export::{export, ExportFormat, ExportLimits, ExportRequest, ExportSpace};

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// A drawing with layers 0 and 1 and the given `(id, layer, geometry)` entities.
fn point_app(entities: Vec<(u128, LayerId, SemanticGeometry)>) -> (Application, SessionState) {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    for id in [LayerId(0), LayerId(1)] {
        b.insert_layer(Layer {
            id,
            name: format!("layer-{}", id.0),
            visible: true,
        })
        .unwrap();
    }
    for (index, (id, layer, geometry)) in entities.into_iter().enumerate() {
        let mut entity = ent(id, geometry, index as i64);
        entity.layer = layer;
        b.insert_entity(entity).unwrap();
    }
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

fn full_ellipse(id: u128, major: f64, ratio: f64) -> (u128, SemanticGeometry) {
    (
        id,
        SemanticGeometry::Ellipse {
            center: point(0.0, 0.0),
            normal: p3(0.0, 0.0, 1.0),
            major_axis: p3(major, 0.0, 0.0),
            ratio,
            start: 0.0,
            sweep: std::f64::consts::TAU,
        },
    )
}

fn run_point(
    app: &mut Application,
    session: &mut SessionState,
    seed: Point3,
) -> CadResult<CommandOutcome> {
    execute(
        app,
        session,
        CommandId::HatchPoint,
        CommandPayload::HatchPoint { seed },
    )
}

fn assert_point_refused(
    app: &mut Application,
    session: &mut SessionState,
    seed: Point3,
    needle: &str,
) -> CadError {
    let before = revision(app);
    let depth = undo_depth(app);
    let count = entity_count(app);
    let error = run_point(app, session, seed).expect_err("the command must refuse");
    let text = format!("{error:?}");
    assert!(text.contains(needle), "expected {needle:?} in {text}");
    assert_eq!(revision(app), before, "a refusal must not write");
    assert_eq!(
        undo_depth(app),
        depth,
        "a refusal must not open a transaction"
    );
    assert_eq!(entity_count(app), count, "a refusal must not insert");
    error
}

// ---------------------------------------------------------------------------
// Nested closed polylines: smallest containing boundary, islands, refusals
// ---------------------------------------------------------------------------

#[test]
fn nested_closed_polylines_choose_the_smallest_boundary_and_keep_islands() {
    let (mut app, mut session) = point_app(vec![
        (1, LayerId(0), square(0, 0.0, 10.0, 0.0).1),
        (2, LayerId(0), square(1, 3.0, 7.0, 0.0).1),
    ]);

    // Between the outer and the inner square: the outer is the smallest
    // containing boundary and the inner is its island (two loops).
    run_point(&mut app, &mut session, p3(1.0, 1.0, 0.0)).expect("HATCHPOINT succeeds");
    let hatch = the_hatch(&app);
    assert_eq!(hatch.loops().len(), 2, "outer boundary plus one island");
    assert!(matches!(hatch.fill(), HatchFill::Solid));
    let area = filled_area(hatch);
    assert!(
        (area - (100.0 - 16.0)).abs() < 1e-6,
        "annulus area {area} vs 84"
    );
    assert_eq!(
        entity_count(&app),
        3,
        "hatch plus the two retained boundaries"
    );

    // Inside the inner square: it is the smallest containing boundary and has no
    // island of its own.
    let (mut app, mut session) = point_app(vec![
        (1, LayerId(0), square(0, 0.0, 10.0, 0.0).1),
        (2, LayerId(0), square(1, 3.0, 7.0, 0.0).1),
    ]);
    run_point(&mut app, &mut session, p3(5.0, 5.0, 0.0)).expect("HATCHPOINT succeeds");
    let hatch = the_hatch(&app);
    assert_eq!(hatch.loops().len(), 1, "the inner square alone");
    let area = filled_area(hatch);
    assert!((area - 16.0).abs() < 1e-6, "inner area {area} vs 16");

    // Outside everything: no visible closed boundary contains the point.
    let (mut app, mut session) = point_app(vec![
        (1, LayerId(0), square(0, 0.0, 10.0, 0.0).1),
        (2, LayerId(0), square(1, 3.0, 7.0, 0.0).1),
    ]);
    assert_point_refused(
        &mut app,
        &mut session,
        p3(20.0, 20.0, 0.0),
        "hatch.boundary_not_found",
    );
}

// ---------------------------------------------------------------------------
// Whole circles and full ellipses
// ---------------------------------------------------------------------------

#[test]
fn a_full_circle_containing_the_seed_becomes_four_quarter_arcs() {
    let (mut app, mut session) = point_app(vec![(1, LayerId(0), circle(1, 0.0, 0.0, 0.0, 5.0).1)]);
    run_point(&mut app, &mut session, p3(1.0, 1.0, 0.0)).expect("HATCHPOINT succeeds");
    let hatch = the_hatch(&app);
    assert_eq!(hatch.loops().len(), 1);
    let loop_ = &hatch.loops()[0];
    assert_eq!(loop_.points.len(), 4, "four quarter-arc vertices");
    let bulge = std::f64::consts::FRAC_PI_8.tan();
    assert_eq!(loop_.bulges, vec![bulge; 4]);
    let area = filled_area(hatch);
    let expected = std::f64::consts::PI * 25.0;
    assert!(
        (area - expected).abs() < 0.5,
        "circle area {area} vs {expected}"
    );
}

#[test]
fn a_full_ellipse_containing_the_seed_is_one_sampled_loop() {
    let (mut app, mut session) = point_app(vec![(1, LayerId(0), full_ellipse(1, 5.0, 0.5).1)]);
    run_point(&mut app, &mut session, p3(1.0, 1.0, 0.0)).expect("HATCHPOINT succeeds");
    let hatch = the_hatch(&app);
    assert_eq!(hatch.loops().len(), 1);
    assert_eq!(hatch.loops()[0].points.len(), 64, "chord-sampled ellipse");
    assert!(hatch.loops()[0].bulges.iter().all(|bulge| *bulge == 0.0));
    let area = filled_area(hatch);
    let expected = std::f64::consts::PI * 5.0 * 2.5;
    assert!(
        (area - expected).abs() < 0.3,
        "ellipse area {area} vs {expected}"
    );
}

// ---------------------------------------------------------------------------
// Discovery skips (hidden, off-plane, different elevation, other kinds)
// ---------------------------------------------------------------------------

#[test]
fn a_hidden_layer_candidate_is_ignored() {
    let (mut app, mut session) = point_app(vec![
        (1, LayerId(0), square(0, 0.0, 10.0, 0.0).1),
        (2, LayerId(1), square(1, 3.0, 7.0, 0.0).1),
    ]);
    session.layer_overrides.set(LayerId(1), false);
    // The seed is inside the hidden inner square too; only the visible outer one
    // can be discovered.
    run_point(&mut app, &mut session, p3(5.0, 5.0, 0.0)).expect("HATCHPOINT succeeds");
    let hatch = the_hatch(&app);
    assert_eq!(hatch.loops().len(), 1, "the hidden island is ignored");
    let area = filled_area(hatch);
    assert!((area - 100.0).abs() < 1e-6, "outer area {area} vs 100");
}

#[test]
fn a_candidate_on_a_non_upward_plane_is_ignored() {
    // A YZ-plane circle (normal +X) is never a boundary for a +Z hatch pick.
    let yz_circle = (
        2,
        SemanticGeometry::Circle {
            center: p3(5.0, 5.0, 0.0),
            normal: p3(1.0, 0.0, 0.0),
            radius: 50.0,
        },
    );
    let (mut app, mut session) = point_app(vec![
        (1, LayerId(0), square(0, 0.0, 10.0, 0.0).1),
        (yz_circle.0, LayerId(0), yz_circle.1),
    ]);
    run_point(&mut app, &mut session, p3(5.0, 5.0, 0.0)).expect("HATCHPOINT succeeds");
    assert_eq!(the_hatch(&app).loops().len(), 1, "only the +Z square");
}

#[test]
fn a_candidate_at_a_different_elevation_is_ignored() {
    let (mut app, mut session) = point_app(vec![
        (1, LayerId(0), square(0, 0.0, 10.0, 0.0).1),
        (2, LayerId(0), square(1, 3.0, 7.0, 5.0).1),
    ]);
    run_point(&mut app, &mut session, p3(5.0, 5.0, 0.0)).expect("HATCHPOINT succeeds");
    assert_eq!(
        the_hatch(&app).loops().len(),
        1,
        "the z=5 candidate is not on the seed plane"
    );
}

#[test]
fn open_polylines_and_ellipse_arcs_are_ignored_not_refused() {
    let open = SemanticGeometry::Polyline {
        points: vec![point(1.0, 1.0), point(9.0, 1.0), point(9.0, 9.0)],
        bulges: vec![0.0; 3],
        closed: false,
    };
    let ellipse_arc = SemanticGeometry::Ellipse {
        center: point(5.0, 5.0),
        normal: p3(0.0, 0.0, 1.0),
        major_axis: p3(4.0, 0.0, 0.0),
        ratio: 0.5,
        start: 0.0,
        sweep: std::f64::consts::PI,
    };
    let (mut app, mut session) = point_app(vec![
        (1, LayerId(0), square(0, 0.0, 10.0, 0.0).1),
        (2, LayerId(0), open),
        (3, LayerId(0), ellipse_arc),
    ]);
    run_point(&mut app, &mut session, p3(5.0, 5.0, 0.0)).expect("HATCHPOINT succeeds");
    assert_eq!(
        the_hatch(&app).loops().len(),
        1,
        "only the closed square is a candidate"
    );

    // With only ignored kinds, discovery finds nothing: an explicit
    // `boundary_not_found`, not a refusal of the individual open entities.
    let open_only = SemanticGeometry::Polyline {
        points: vec![point(1.0, 1.0), point(9.0, 1.0), point(9.0, 9.0)],
        bulges: vec![0.0; 3],
        closed: false,
    };
    let (mut app, mut session) = point_app(vec![(1, LayerId(0), open_only)]);
    assert_point_refused(
        &mut app,
        &mut session,
        p3(5.0, 5.0, 0.0),
        "hatch.boundary_not_found",
    );
}

// ---------------------------------------------------------------------------
// Explicit pick refusals
// ---------------------------------------------------------------------------

#[test]
fn a_seed_on_a_boundary_edge_is_refused() {
    let (mut app, mut session) = point_app(vec![(1, LayerId(0), square(0, 0.0, 10.0, 0.0).1)]);
    assert_point_refused(
        &mut app,
        &mut session,
        p3(0.0, 5.0, 0.0),
        "hatch.boundary_on_edge",
    );
}

#[test]
fn two_equal_area_containing_boundaries_are_refused_as_ambiguous() {
    let (mut app, mut session) = point_app(vec![
        (1, LayerId(0), circle(1, 0.0, 0.0, 0.0, 5.0).1),
        (2, LayerId(0), circle(2, 0.5, 0.0, 0.0, 5.0).1),
    ]);
    // Inside both equal-radius circles, and clear of both boundaries.
    assert_point_refused(
        &mut app,
        &mut session,
        p3(0.25, 0.25, 0.0),
        "hatch.boundary_ambiguous",
    );
}

#[test]
fn a_non_finite_seed_is_refused_before_any_lookup() {
    let (mut app, mut session) = point_app(vec![(1, LayerId(0), square(0, 0.0, 10.0, 0.0).1)]);
    assert_point_refused(
        &mut app,
        &mut session,
        p3(f64::NAN, 0.0, 0.0),
        "hatch.boundary_invalid",
    );
}

// ---------------------------------------------------------------------------
// Transaction shape, seeds and export
// ---------------------------------------------------------------------------

#[test]
fn hatchpoint_is_one_transaction_and_one_undo_step() {
    let (mut app, mut session) = point_app(vec![(1, LayerId(0), circle(1, 0.0, 0.0, 0.0, 5.0).1)]);
    let before = revision(&app);
    let out = run_point(&mut app, &mut session, p3(1.0, 1.0, 0.0)).expect("HATCHPOINT");
    assert_eq!(out.objects.len(), 1, "one inserted hatch");
    let changes = out.changes.expect("change set");
    assert_eq!(changes.before, before);
    assert_eq!(changes.after.0, before.0 + 1, "one revision bump");
    assert_eq!(undo_depth(&app), 1, "one undo step");
    assert_eq!(entity_count(&app), 2, "boundary retained plus the hatch");

    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(entity_count(&app), 1, "the boundary entity remains");
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn the_hatch_records_the_projected_seed_and_exports_group_98() {
    let (mut app, mut session) = point_app(vec![(1, LayerId(0), square(0, 0.0, 10.0, 0.0).1)]);
    run_point(&mut app, &mut session, p3(1.0, 1.0, 0.0)).expect("HATCHPOINT succeeds");
    assert_eq!(the_hatch(&app).seeds(), &[[1.0, 1.0]]);

    let (bytes, _report) = export(&ExportRequest {
        database: &app.workspace.documents[&DocumentId(1)].drawing,
        document_id: DocumentId(1),
        space: ExportSpace::Model,
        format: ExportFormat::DxfText,
        limits: ExportLimits::default(),
        units: None,
    })
    .expect("the drawing exports");
    let text = std::str::from_utf8(&bytes).expect("ASCII DXF");
    let pairs: Vec<(&str, &str)> = text
        .lines()
        .map(str::trim)
        .collect::<Vec<_>>()
        .chunks_exact(2)
        .map(|pair| (pair[0], pair[1]))
        .collect();
    let seed_count = pairs
        .iter()
        .position(|(group, value)| *group == "98" && *value == "1")
        .expect("the hatch writes a one-seed group 98");
    assert_eq!(
        &pairs[seed_count + 1..seed_count + 3],
        &[("10", "1.0"), ("20", "1.0")],
        "the projected seed follows the group 98 count"
    );
}

// ---------------------------------------------------------------------------
// Mode and capture contract
// ---------------------------------------------------------------------------

#[test]
fn hatchpoint_is_work_only_and_refused_in_viewer_mode() {
    assert!(CommandId::HatchPoint.requires_work_mode());
    let (mut app, mut session) = point_app(vec![(1, LayerId(0), square(0, 0.0, 10.0, 0.0).1)]);
    session.switch_mode(AppMode::Viewer).unwrap();
    let before = revision(&app);
    let error = run_point(&mut app, &mut session, p3(5.0, 5.0, 0.0)).expect_err("Viewer refuses");
    assert!(matches!(error, CadError::PermissionDenied), "{error:?}");
    assert_eq!(revision(&app), before);
}

#[test]
fn hatchpoint_captures_one_point_and_does_not_require_a_selection() {
    let mut tool = DrawTool::new(DrawToolKind::HatchPoint, 0);
    assert_eq!(tool.kind().key(), "hatchpoint");
    assert_eq!(tool.kind().label(), "HATCHPOINT");
    assert_eq!(tool.remaining(), Some(1));
    assert!(!tool.is_ready(), "no seed captured yet");
    tool.push_point(p3(2.0, 3.0, 0.0)).unwrap();
    assert!(tool.is_ready(), "one point and no selection requirement");
    assert_eq!(
        tool.intent().unwrap(),
        DrawIntent::HatchPoint {
            seed: p3(2.0, 3.0, 0.0)
        }
    );
    assert!(matches!(
        tool.push_point(p3(9.0, 9.0, 0.0)),
        Err(CadError::InvalidInput(_))
    ));
}
