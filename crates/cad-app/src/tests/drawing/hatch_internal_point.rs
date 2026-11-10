//! HATCHPOINT (internal-point boundary discovery) contract tests.
//!
//! Pins the declared behaviour: one picked internal point selects the **smallest
//! visible closed boundary** containing it — a whole closed entity or a stitched
//! `LINE`/`ARC` edge face — nested boundaries become islands, hidden / off-plane /
//! different-elevation / other kinds are skipped (never refused), and every
//! ambiguous or empty pick is an explicit refusal with no transaction. The current
//! selection is deliberately not used.

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
        .as_chunks::<2>()
        .0
        .iter()
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

// ---------------------------------------------------------------------------
// LINE / ARC edge faces (the v2 discovery path)
// ---------------------------------------------------------------------------

fn line_geometry(start: (f64, f64), end: (f64, f64)) -> SemanticGeometry {
    SemanticGeometry::Line {
        start: p3(start.0, start.1, 0.0),
        end: p3(end.0, end.1, 0.0),
    }
}

fn arc_geometry(center: (f64, f64), radius: f64, start: f64, sweep: f64) -> SemanticGeometry {
    SemanticGeometry::Arc {
        center: p3(center.0, center.1, 0.0),
        normal: p3(0.0, 0.0, 1.0),
        radius,
        start,
        sweep,
    }
}

/// The four `LINE` entities of an axis-aligned square outline, on layer 0.
fn line_rectangle(base: u128, min: f64, max: f64) -> Vec<(u128, LayerId, SemanticGeometry)> {
    vec![
        (base, LayerId(0), line_geometry((min, min), (max, min))),
        (base + 1, LayerId(0), line_geometry((max, min), (max, max))),
        (base + 2, LayerId(0), line_geometry((max, max), (min, max))),
        (base + 3, LayerId(0), line_geometry((min, max), (min, min))),
    ]
}

/// A rectangle of `LINE`s whose left half is separated by an internal wall from
/// `(5, 0)` to `(5, 10)` — a two-face subdivision with two degree-3 T-junctions.
fn t_junction_rectangle() -> Vec<(u128, LayerId, SemanticGeometry)> {
    vec![
        (1, LayerId(0), line_geometry((0.0, 0.0), (5.0, 0.0))),
        (2, LayerId(0), line_geometry((5.0, 0.0), (10.0, 0.0))),
        (3, LayerId(0), line_geometry((10.0, 0.0), (10.0, 10.0))),
        (4, LayerId(0), line_geometry((10.0, 10.0), (5.0, 10.0))),
        (5, LayerId(0), line_geometry((5.0, 10.0), (0.0, 10.0))),
        (6, LayerId(0), line_geometry((0.0, 10.0), (0.0, 0.0))),
        (7, LayerId(0), line_geometry((5.0, 0.0), (5.0, 10.0))),
    ]
}

#[test]
fn four_lines_around_the_seed_become_one_exact_hatch_loop() {
    let (mut app, mut session) = point_app(line_rectangle(1, 0.0, 10.0));
    run_point(&mut app, &mut session, p3(2.0, 3.0, 0.0))
        .expect("a closed rectangle of LINEs hatches");
    let hatch = the_hatch(&app);
    assert_eq!(hatch.loops().len(), 1, "one face, one loop");
    let loop_ = &hatch.loops()[0];
    assert_eq!(
        loop_.points,
        vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]],
        "one vertex per LINE edge, in the counter-clockwise face walk"
    );
    assert_eq!(
        loop_.bulges,
        vec![0.0; 4],
        "LINE edges are bulge-0 segments"
    );
    let area = filled_area(hatch);
    assert!((area - 100.0).abs() < 1e-6, "rectangle area {area}");
    assert_eq!(hatch.seeds(), &[[2.0, 3.0]], "the seed stays group 98");
    assert_eq!(entity_count(&app), 5, "four retained edges plus the hatch");
}

#[test]
fn nested_line_rectangles_keep_the_outer_and_its_island() {
    let mut entities = line_rectangle(1, 0.0, 10.0);
    entities.extend(line_rectangle(5, 3.0, 7.0));

    // Between the two rectangles: the outer one is the containing face and the
    // inner one is its island.
    let (mut app, mut session) = point_app(entities.clone());
    run_point(&mut app, &mut session, p3(1.0, 1.0, 0.0)).expect("the annulus hatches");
    let hatch = the_hatch(&app);
    assert_eq!(hatch.loops().len(), 2, "outer rectangle plus one island");
    let area = filled_area(hatch);
    assert!((area - 84.0).abs() < 1e-6, "annulus area {area} vs 84");

    // Inside the inner rectangle: it is the smallest containing face and has no
    // island of its own.
    let (mut app, mut session) = point_app(entities);
    run_point(&mut app, &mut session, p3(5.0, 5.0, 0.0)).expect("the inner square hatches");
    let hatch = the_hatch(&app);
    assert_eq!(hatch.loops().len(), 1, "the inner rectangle alone");
    let area = filled_area(hatch);
    assert!((area - 16.0).abs() < 1e-6, "inner area {area} vs 16");
}

#[test]
fn a_t_junction_branch_walks_the_face_that_holds_the_seed() {
    // Two halves of equal area meet at two degree-3 nodes. The face walk (not a
    // refusal) must pick the half containing the seed.
    let (mut app, mut session) = point_app(t_junction_rectangle());
    run_point(&mut app, &mut session, p3(2.0, 5.0, 0.0)).expect("the left face hatches");
    let hatch = the_hatch(&app);
    assert_eq!(hatch.loops().len(), 1, "one face around the seed");
    let loop_ = &hatch.loops()[0];
    assert_eq!(
        loop_.points,
        vec![[0.0, 0.0], [5.0, 0.0], [5.0, 10.0], [0.0, 10.0]],
        "the left half, bounded by the internal wall"
    );
    assert_eq!(loop_.bulges, vec![0.0; 4]);
    let area = filled_area(hatch);
    assert!((area - 50.0).abs() < 1e-6, "left half area {area} vs 50");

    // The right half is the other face of the same graph.
    let (mut app, mut session) = point_app(t_junction_rectangle());
    run_point(&mut app, &mut session, p3(8.0, 5.0, 0.0)).expect("the right face hatches");
    let hatch = the_hatch(&app);
    let loop_ = &hatch.loops()[0];
    assert_eq!(
        loop_.points,
        vec![[5.0, 0.0], [10.0, 0.0], [10.0, 10.0], [5.0, 10.0]],
    );
    let area = filled_area(hatch);
    assert!((area - 50.0).abs() < 1e-6, "right half area {area} vs 50");
}

#[test]
fn a_dangling_line_on_the_boundary_is_ignored() {
    // An inward spur attached to the right edge ends inside the rectangle. A
    // degree-1 chain bounds no face, so it is pruned and never fails the pick.
    let mut entities = line_rectangle(1, 0.0, 10.0);
    entities.push((9, LayerId(0), line_geometry((10.0, 5.0), (6.0, 5.0))));
    let (mut app, mut session) = point_app(entities);
    run_point(&mut app, &mut session, p3(2.0, 2.0, 0.0))
        .expect("the dangling spur does not fail the pick");
    let hatch = the_hatch(&app);
    assert_eq!(hatch.loops().len(), 1);
    assert_eq!(hatch.loops()[0].points.len(), 4, "the spur adds no vertex");
    let area = filled_area(hatch);
    assert!((area - 100.0).abs() < 1e-6, "rectangle area {area}");
    assert_eq!(entity_count(&app), 6, "the spur is retained, not deleted");
}

#[test]
fn a_semicircle_closed_by_its_chord_is_an_exact_lens_not_sampled_chords() {
    // A half disc: one exact semicircle plus the diameter LINE. The two-edge face
    // is split exactly into two quarter arcs so the creation-path validator sees
    // three vertices — the bulge stays `tan(sweep / 4)` per sub-arc, never a
    // sampled chord.
    let entities = vec![
        (
            1,
            LayerId(0),
            arc_geometry((5.0, 0.0), 5.0, 0.0, std::f64::consts::PI),
        ),
        (2, LayerId(0), line_geometry((0.0, 0.0), (10.0, 0.0))),
    ];
    let (mut app, mut session) = point_app(entities);
    run_point(&mut app, &mut session, p3(5.0, 2.0, 0.0)).expect("the half disc hatches");
    let hatch = the_hatch(&app);
    assert_eq!(hatch.loops().len(), 1);
    let loop_ = &hatch.loops()[0];
    assert_eq!(
        loop_.points.len(),
        3,
        "the arc is split into two exact quarters"
    );
    let quarters = std::f64::consts::FRAC_PI_8.tan();
    let mut arc_bulges: Vec<f64> = loop_
        .bulges
        .iter()
        .copied()
        .filter(|bulge| *bulge != 0.0)
        .collect();
    arc_bulges.sort_by(f64::total_cmp);
    assert_eq!(
        arc_bulges,
        vec![quarters, quarters],
        "exact quarter-arc bulges"
    );
    // Every vertex of the loop stays on the circle: no sampled chord vertex.
    for point in &loop_.points {
        let radius = (point[0] - 5.0).hypot(point[1]);
        assert!((radius - 5.0).abs() < 1e-12, "{point:?} is off the circle");
    }
    // The exact half-disc area, not a chord-polygon approximation.
    let area = filled_area(hatch);
    let expected = std::f64::consts::PI * 25.0 / 2.0;
    assert!(
        (area - expected).abs() < 0.2,
        "half-disc area {area} vs {expected}"
    );
}

#[test]
fn a_semicircle_edge_keeps_the_exact_tan_pi_over_four_bulge() {
    // One semicircle bulging over the top plus a three-line skirt: a four-edge face
    // whose chord polygon is non-degenerate, so the semicircle stays **one** bulge
    // segment with exactly `tan(pi / 4)`.
    let entities = vec![
        (
            1,
            LayerId(0),
            arc_geometry((5.0, 0.0), 5.0, 0.0, std::f64::consts::PI),
        ),
        (2, LayerId(0), line_geometry((0.0, 0.0), (0.0, -4.0))),
        (3, LayerId(0), line_geometry((0.0, -4.0), (10.0, -4.0))),
        (4, LayerId(0), line_geometry((10.0, -4.0), (10.0, 0.0))),
    ];
    let (mut app, mut session) = point_app(entities);
    run_point(&mut app, &mut session, p3(5.0, 1.0, 0.0)).expect("the arc-and-lines face hatches");
    let hatch = the_hatch(&app);
    assert_eq!(hatch.loops().len(), 1);
    let loop_ = &hatch.loops()[0];
    let expected_points = [[10.0, 0.0], [0.0, 0.0], [0.0, -4.0], [10.0, -4.0]];
    assert_eq!(
        loop_.points.len(),
        expected_points.len(),
        "one vertex per edge"
    );
    for (actual, expected) in loop_.points.iter().zip(expected_points) {
        assert!(
            (actual[0] - expected[0]).abs() < 1e-12 && (actual[1] - expected[1]).abs() < 1e-12,
            "vertex {actual:?} vs {expected:?}"
        );
    }
    assert_eq!(
        loop_.bulges,
        vec![std::f64::consts::FRAC_PI_4.tan(), 0.0, 0.0, 0.0],
        "the semicircle is exactly tan(sweep / 4), never sampled chords"
    );
    let area = filled_area(hatch);
    let expected = std::f64::consts::PI * 25.0 / 2.0 + 40.0;
    assert!((area - expected).abs() < 0.2, "area {area} vs {expected}");
}

#[test]
fn edge_face_pick_refusals_and_ignores_match_the_entity_rules() {
    // A seed outside every LINE face: no boundary found, no transaction.
    let (mut app, mut session) = point_app(line_rectangle(1, 0.0, 10.0));
    assert_point_refused(
        &mut app,
        &mut session,
        p3(20.0, 20.0, 0.0),
        "hatch.boundary_not_found",
    );

    // A seed on a LINE edge: the analytic on-edge refusal, no transaction.
    let (mut app, mut session) = point_app(line_rectangle(1, 0.0, 10.0));
    assert_point_refused(
        &mut app,
        &mut session,
        p3(0.0, 5.0, 0.0),
        "hatch.boundary_on_edge",
    );

    // A hidden inner LINE square is ignored: only the visible outer face remains.
    let mut entities = line_rectangle(1, 0.0, 10.0);
    for (id, _, geometry) in line_rectangle(5, 3.0, 7.0) {
        entities.push((id, LayerId(1), geometry));
    }
    let (mut app, mut session) = point_app(entities);
    session.layer_overrides.set(LayerId(1), false);
    run_point(&mut app, &mut session, p3(5.0, 5.0, 0.0)).expect("the visible outer face hatches");
    let hatch = the_hatch(&app);
    assert_eq!(hatch.loops().len(), 1, "the hidden face is ignored");
    assert!((filled_area(hatch) - 100.0).abs() < 1e-6);

    // LINE edges at another elevation are ignored (same z-filter as entities).
    let mut entities = line_rectangle(1, 0.0, 10.0);
    for (id, layer, geometry) in line_rectangle(5, 3.0, 7.0) {
        let mut geometry = geometry;
        if let SemanticGeometry::Line { start, end } = &mut geometry {
            start.z = 5.0;
            end.z = 5.0;
        }
        entities.push((id, layer, geometry));
    }
    let (mut app, mut session) = point_app(entities);
    run_point(&mut app, &mut session, p3(5.0, 5.0, 0.0)).expect("only the seed-plane face is used");
    assert_eq!(
        the_hatch(&app).loops().len(),
        1,
        "the z=5 edges are skipped"
    );

    // An ARC about a non-upward normal never joins the edge graph.
    let mut entities = line_rectangle(1, 0.0, 10.0);
    entities.push((
        9,
        LayerId(0),
        SemanticGeometry::Arc {
            center: p3(5.0, 5.0, 0.0),
            normal: p3(1.0, 0.0, 0.0),
            radius: 50.0,
            start: 0.0,
            sweep: std::f64::consts::PI,
        },
    ));
    let (mut app, mut session) = point_app(entities);
    run_point(&mut app, &mut session, p3(2.0, 2.0, 0.0)).expect("the +Z rectangle hatches");
    assert_eq!(the_hatch(&app).loops().len(), 1, "the YZ arc is ignored");

    // One transaction / one undo step: undo restores the pre-command count.
    let (mut app, mut session) = point_app(line_rectangle(1, 0.0, 10.0));
    let before = entity_count(&app);
    run_point(&mut app, &mut session, p3(2.0, 2.0, 0.0)).expect("the rectangle hatches");
    assert_eq!(entity_count(&app), before + 1);
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(entity_count(&app), before, "undo removes only the hatch");
    assert_eq!(undo_depth(&app), 0);
}
