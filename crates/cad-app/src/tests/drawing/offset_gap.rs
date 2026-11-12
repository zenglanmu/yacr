//! `OFFSETGAPTYPE` gap-type contract tests (sharp / fillet / chamfer).
//!
//! The exact right-angle case is pinned numerically: the source open L, the
//! three documented gap types, the resulting vertex/bulge counts and every
//! coordinate. The arc-aware arm is pinned by counts (the source arc bulge
//! survives and one fillet arc is added per corner), and the pre-existing
//! collapse refusal still fires.

use super::*;

use crate::command_payload::{OffsetGapType, OffsetLayer, OffsetMode, OffsetOptions};

fn build(entities: Vec<DbEntity>) -> Application {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    for entity in entities {
        b.insert_entity(entity).unwrap();
    }
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
    app
}

fn polyline(id: u128, points: Vec<Point3>, bulges: Vec<f64>, closed: bool) -> DbEntity {
    ent(
        id,
        SemanticGeometry::Polyline {
            points,
            bulges,
            closed,
            width: 0.0,
        },
        0,
    )
}

fn offset_options(gap: OffsetGapType, side: Point3) -> OffsetOptions {
    OffsetOptions {
        refs: vec![ref_of(1)],
        side_points: vec![side],
        mode: OffsetMode::Distance(2.0),
        erase_source: false,
        layer: OffsetLayer::Source,
        gap,
    }
}

fn command_for(side: Point3) -> Command {
    command(
        CommandId::OffsetEntity,
        CommandPayload::Offset {
            refs: vec![ref_of(1)],
            side_point: side,
        },
    )
}

fn offset_polyline_geometry(app: &Application, outcome: &CommandOutcome) -> SemanticGeometry {
    assert_eq!(outcome.objects.len(), 1);
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(outcome.objects[0].0))
        .expect("offset entity exists")
        .geometry
        .clone()
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

fn assert_points(actual: &[Point3], expected: &[(f64, f64)]) {
    assert_eq!(actual.len(), expected.len(), "{actual:?}");
    for (point, (x, y)) in actual.iter().zip(expected) {
        assert!(
            (point.x - x).abs() < 1e-9 && (point.y - y).abs() < 1e-9,
            "expected ({x}, {y}), got {point:?} in {actual:?}"
        );
    }
}

/// The source open L: (0,0) → (10,0) → (10,10), offset 2 to the **right** of the
/// path (the outside of its left turn), which leaves a real gap at (10,0).
fn right_angle_app() -> Application {
    build(vec![polyline(
        1,
        vec![point(0.0, 0.0), point(10.0, 0.0), point(10.0, 10.0)],
        vec![0.0, 0.0, 0.0],
        false,
    )])
}

#[test]
fn offset_right_angle_sharp_keeps_the_single_miter_vertex() {
    let mut app = right_angle_app();
    let outcome = app
        .offset_entities_with_options(
            &command_for(point(5.0, -3.0)),
            &offset_options(OffsetGapType::Sharp, point(5.0, -3.0)),
        )
        .unwrap();
    let SemanticGeometry::Polyline {
        points,
        bulges,
        closed,
        ..
    } = offset_polyline_geometry(&app, &outcome)
    else {
        panic!("expected a polyline");
    };
    assert!(!closed);
    assert_eq!(points.len(), 3, "one miter vertex per interior corner");
    assert_eq!(bulges.len(), points.len());
    assert_points(&points, &[(0.0, -2.0), (12.0, -2.0), (12.0, 10.0)]);
    assert!(bulges.iter().all(|bulge| *bulge == 0.0), "{bulges:?}");
    assert_eq!(undo_depth(&app), 1, "one transaction / one undo");
}

#[test]
fn offset_right_angle_chamfer_adds_the_gap_line() {
    let mut app = right_angle_app();
    let outcome = app
        .offset_entities_with_options(
            &command_for(point(5.0, -3.0)),
            &offset_options(OffsetGapType::Chamfer, point(5.0, -3.0)),
        )
        .unwrap();
    let SemanticGeometry::Polyline {
        points,
        bulges,
        closed,
        ..
    } = offset_polyline_geometry(&app, &outcome)
    else {
        panic!("expected a polyline");
    };
    assert!(!closed);
    // The miter vertex (12, -2) is replaced by the two untrimmed offset ends.
    assert_eq!(points.len(), 4, "one extra vertex per filled gap");
    assert_eq!(bulges.len(), points.len());
    assert_points(
        &points,
        &[(0.0, -2.0), (10.0, -2.0), (12.0, 0.0), (12.0, 10.0)],
    );
    assert!(bulges.iter().all(|bulge| *bulge == 0.0), "{bulges:?}");
    assert_eq!(undo_depth(&app), 1);
}

#[test]
fn offset_right_angle_fillet_adds_the_tangent_arc() {
    let mut app = right_angle_app();
    let outcome = app
        .offset_entities_with_options(
            &command_for(point(5.0, -3.0)),
            &offset_options(OffsetGapType::Fillet, point(5.0, -3.0)),
        )
        .unwrap();
    let SemanticGeometry::Polyline {
        points,
        bulges,
        closed,
        ..
    } = offset_polyline_geometry(&app, &outcome)
    else {
        panic!("expected a polyline");
    };
    assert!(!closed);
    assert_eq!(points.len(), 4);
    assert_eq!(bulges.len(), points.len());
    assert_points(
        &points,
        &[(0.0, -2.0), (10.0, -2.0), (12.0, 0.0), (12.0, 10.0)],
    );
    // Arc of radius 2 centred on the source corner (10, 0): a quarter turn, so
    // bulge = tan(π/8).
    let expected = (std::f64::consts::FRAC_PI_8).tan();
    assert!((bulges[0]).abs() < 1e-12, "{bulges:?}");
    assert!(
        (bulges[1] - expected).abs() < 1e-9,
        "expected {} got {}",
        expected,
        bulges[1]
    );
    assert!((bulges[2]).abs() < 1e-12, "{bulges:?}");
    // The arc is tangent to both offset lines at their endpoints: the recovered
    // circle is centred on the source vertex with radius = the offset distance.
    match cad_geometry::bulge_arc_geometry(
        points[1],
        points[2],
        bulges[1],
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 1.0,
        },
    ) {
        Some(SemanticGeometry::Arc { center, radius, .. }) => {
            assert!(near(center, point(10.0, 0.0)), "{center:?}");
            assert!((radius - 2.0).abs() < 1e-9, "{radius}");
        }
        other => panic!("expected an arc, got {other:?}"),
    }
    assert_eq!(undo_depth(&app), 1);
}

#[test]
fn offset_concave_corner_keeps_the_miter_under_every_gap_type() {
    // Offset the same L to the **left** (the inside of the turn): the offset
    // segments cross, so there is no gap and every gap type must keep the miter
    // trim (the sharp result) instead of inventing an arc/line.
    for gap in [
        OffsetGapType::Sharp,
        OffsetGapType::Fillet,
        OffsetGapType::Chamfer,
    ] {
        let mut app = right_angle_app();
        let outcome = app
            .offset_entities_with_options(
                &command_for(point(5.0, 3.0)),
                &offset_options(gap, point(5.0, 3.0)),
            )
            .unwrap();
        let SemanticGeometry::Polyline { points, .. } = offset_polyline_geometry(&app, &outcome)
        else {
            panic!("expected a polyline");
        };
        assert_eq!(points.len(), 3, "{gap:?} must trim, not fill");
        assert_points(&points, &[(0.0, 2.0), (8.0, 2.0), (8.0, 10.0)]);
    }
}

#[test]
fn offset_fillet_keeps_the_source_arc_and_adds_one_fillet_at_the_joint() {
    // Open polyline: a straight bottom edge followed by a 0.5-bulge arc. The
    // joint at (10, 0) is a real corner (the turn is +36.87°), so offsetting to
    // the outside leaves a gap there. The arc segment stays a concentric arc
    // (same centre, radius + 2, same 0.5 bulge) and the gap gets one fillet arc
    // of radius 2 centred on the source vertex.
    let mut app = build(vec![polyline(
        1,
        vec![point(0.0, 0.0), point(10.0, 0.0), point(10.0, 10.0)],
        vec![0.0, 0.5, 0.0],
        false,
    )]);
    let outcome = app
        .offset_entities_with_options(
            &command_for(point(20.0, 5.0)),
            &offset_options(OffsetGapType::Fillet, point(20.0, 5.0)),
        )
        .unwrap();
    let SemanticGeometry::Polyline {
        points,
        bulges,
        closed,
        ..
    } = offset_polyline_geometry(&app, &outcome)
    else {
        panic!("expected a polyline");
    };
    assert!(!closed);
    assert_eq!(points.len(), 4, "open start + one filled joint + two ends");
    assert_eq!(bulges.len(), points.len());
    let near6 = |a: Point3, b: Point3| {
        (a.x - b.x).abs() < 1e-6 && (a.y - b.y).abs() < 1e-6 && (a.z - b.z).abs() < 1e-6
    };
    assert!(near6(points[0], point(0.0, -2.0)), "{}", points[0].x);
    assert!(near6(points[1], point(10.0, -2.0)), "{:?}", points[1]);
    assert!(near6(points[2], point(11.2, -1.6)), "{:?}", points[2]);
    assert!(near6(points[3], point(11.2, 11.6)), "{:?}", points[3]);
    // The source arc survives as an arc with its stored 0.5 bulge.
    assert!(
        (bulges[2] - 0.5).abs() < 1e-9,
        "source arc bulge: {}",
        bulges[2]
    );
    // The fillet is the radius-2 arc centred on the source corner.
    match cad_geometry::bulge_arc_geometry(
        points[1],
        points[2],
        bulges[1],
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 1.0,
        },
    ) {
        Some(SemanticGeometry::Arc { center, radius, .. }) => {
            assert!(near6(center, point(10.0, 0.0)), "{center:?}");
            assert!((radius - 2.0).abs() < 1e-9, "{radius}");
        }
        other => panic!("expected a fillet arc, got {other:?}"),
    }
    assert_eq!(undo_depth(&app), 1);
}

#[test]
fn offset_collapse_refusal_still_fires_under_a_gap_type() {
    // A doubling-back vertex has no offset join at all: the miter denominator
    // vanishes, which stays the named `offset.collapsed` refusal for every gap
    // type (nothing is written, no history).
    for gap in [OffsetGapType::Fillet, OffsetGapType::Chamfer] {
        let mut app = build(vec![polyline(
            1,
            vec![point(0.0, 0.0), point(10.0, 0.0), point(0.0, 0.0)],
            vec![0.0, 0.0, 0.0],
            false,
        )]);
        let rev = revision(&app);
        let error = app
            .offset_entities_with_options(
                &command_for(point(5.0, -3.0)),
                &offset_options(gap, point(5.0, -3.0)),
            )
            .unwrap_err();
        match error {
            CadError::Unsupported(reason) => {
                assert!(reason.contains("offset.collapsed"), "{reason}");
            }
            other => panic!("expected Unsupported, got {other:?}"),
        }
        assert_eq!(revision(&app), rev, "nothing written");
        assert_eq!(undo_depth(&app), 0, "no history");
    }
}

#[test]
fn offset_closed_square_gap_types_fill_all_four_wrap_corners() {
    // A closed 8 × 8 square offset 2 outward. Every convex corner is a real gap:
    // the offset edges do not meet, so both gap types insert one segment per
    // corner (8 vertices, closed) in one transaction / one undo step. This is the
    // closed-ring branch of the gap-type path (the open right-angle cases above
    // only exercise the non-wrapping branch).
    for gap in [OffsetGapType::Fillet, OffsetGapType::Chamfer] {
        let mut app = build(vec![polyline(
            1,
            vec![
                point(0.0, 0.0),
                point(8.0, 0.0),
                point(8.0, 8.0),
                point(0.0, 8.0),
            ],
            vec![0.0; 4],
            true,
        )]);
        let before = revision(&app);
        let outcome = app
            .offset_entities_with_options(
                &command_for(point(-2.0, -2.0)),
                &offset_options(gap, point(-2.0, -2.0)),
            )
            .unwrap();
        let SemanticGeometry::Polyline {
            points,
            bulges,
            closed,
            ..
        } = offset_polyline_geometry(&app, &outcome)
        else {
            panic!("expected a polyline");
        };
        assert!(closed, "{gap:?} keeps the ring closed");
        assert_eq!(points.len(), 8, "{gap:?}: two vertices per wrap corner");
        assert_eq!(bulges.len(), 8);
        assert_eq!(revision(&app).0, before.0 + 1, "{gap:?}: one revision bump");
        assert_eq!(undo_depth(&app), 1, "{gap:?}: one undo step");
        assert_points(
            &points,
            &[
                (-2.0, 0.0),
                (0.0, -2.0),
                (8.0, -2.0),
                (10.0, 0.0),
                (10.0, 8.0),
                (8.0, 10.0),
                (0.0, 10.0),
                (-2.0, 8.0),
            ],
        );
        let vertices = [
            point(0.0, 0.0),
            point(8.0, 0.0),
            point(8.0, 8.0),
            point(0.0, 8.0),
        ];
        for (joint, vertex) in vertices.iter().enumerate() {
            let edge = joint * 2;
            match gap {
                OffsetGapType::Fillet => {
                    assert!(bulges[edge].abs() > 1e-9, "{gap:?} fills edge {edge}");
                    match cad_geometry::bulge_arc_geometry(
                        points[edge],
                        points[edge + 1],
                        bulges[edge],
                        Point3 {
                            x: 0.0,
                            y: 0.0,
                            z: 1.0,
                        },
                    ) {
                        Some(SemanticGeometry::Arc { center, radius, .. }) => {
                            assert!(near(center, *vertex), "{center:?} vs {vertex:?}");
                            assert!((radius - 2.0).abs() < 1e-9, "radius {radius}");
                        }
                        other => panic!("expected the gap fillet arc, got {other:?}"),
                    }
                }
                OffsetGapType::Chamfer => {
                    assert_eq!(bulges[edge], 0.0, "{gap:?} fills edge {edge} straight");
                }
                OffsetGapType::Sharp => unreachable!(),
            }
        }
    }
}
