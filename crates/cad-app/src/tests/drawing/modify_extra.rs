//! `REVERSE` (AutoCAD 2026) contract tests.
//!
//! Pins the selection-based reversal of LINE / LWPOLYLINE / SPLINE: the exact
//! geometry (including the arc midpoint of every bulged segment and the
//! `new(t) == old(1 − t)` spline invariant), one transaction / one undo step,
//! and every refusal (empty selection, payload, non-addressable, unsupported
//! geometry, Viewer mode) changing no revision and recording no undo entry.

use super::*;
use cad_db::{DbEntity, DbObject, DrawingDatabaseBuilder, Layer};
use cad_geometry::{arbitrary_axis, bulge_arc_geometry, NurbsCurve};

fn p3(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

fn ent(id: u128, geometry: SemanticGeometry, order: i64) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbEntity".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry,
        draw_order: order,
    }
}

fn build(entities: Vec<DbEntity>, mode: AppMode) -> (Application, SessionState) {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    for entity in entities {
        builder.insert_entity(entity).unwrap();
    }
    let drawing = builder.finish().unwrap();
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
    (app, SessionState::new(DocumentId(1), mode))
}

fn ref_of(entity: u128) -> SelectionRef {
    SelectionRef {
        document: DocumentId(1),
        entity: EntityId(entity),
        instance: InstancePath::default(),
        sub_element: None,
    }
}

fn revision(app: &Application) -> Revision {
    app.workspace.documents[&DocumentId(1)].drawing.revision()
}

fn undo_depth(app: &Application) -> usize {
    app.history
        .get(&DocumentId(1))
        .map(|history| history.undo_depth())
        .unwrap_or(0)
}

fn geometry(app: &Application, id: u128) -> SemanticGeometry {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(id))
        .expect("entity must exist")
        .geometry
        .clone()
}

fn reverse(app: &mut Application, session: &mut SessionState) -> CadResult<CommandOutcome> {
    app.execute(session, command(CommandId::Reverse, CommandPayload::None))
}

fn open_bulged(bulges_per_vertex: bool) -> SemanticGeometry {
    // Three vertices, two bulged segments; one storage shape per vertex, one per
    // segment.
    let points = vec![p3(0.0, 0.0, 0.0), p3(4.0, 0.0, 0.0), p3(4.0, 4.0, 0.0)];
    let bulges = if bulges_per_vertex {
        vec![0.75, -0.5, 0.0]
    } else {
        vec![0.75, -0.5]
    };
    SemanticGeometry::Polyline {
        points,
        bulges,
        closed: false,
    }
}

fn closed_bulged() -> SemanticGeometry {
    SemanticGeometry::Polyline {
        points: vec![
            p3(0.0, 0.0, 0.0),
            p3(4.0, 0.0, 0.0),
            p3(4.0, 4.0, 0.0),
            p3(0.0, 4.0, 0.0),
        ],
        bulges: vec![0.5, -0.25, 0.75, -0.125],
        closed: true,
    }
}

fn spline() -> SemanticGeometry {
    // Degree 3, clamped uniform knots, four control points.
    SemanticGeometry::Spline {
        degree: 3,
        knots: vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
        control_points: vec![
            p3(0.0, 0.0, 0.0),
            p3(2.0, 5.0, 0.0),
            p3(6.0, -3.0, 0.0),
            p3(9.0, 2.0, 0.0),
        ],
        weights: vec![1.0, 0.75, 1.25, 1.0],
    }
}

/// The arc midpoint of every real segment of a bulged polyline (the straight
/// case is not used by these fixtures).
fn segment_midpoints(geometry: &SemanticGeometry) -> Vec<Point3> {
    let SemanticGeometry::Polyline {
        points,
        bulges,
        closed,
    } = geometry
    else {
        panic!("expected a polyline");
    };
    let n = points.len();
    let segments = if *closed { n } else { n - 1 };
    (0..segments)
        .map(|i| {
            let a = points[i];
            let b = points[(i + 1) % n];
            let arc = bulge_arc_geometry(a, b, bulges[i], p3(0.0, 0.0, 1.0))
                .expect("bulged segment must produce an arc");
            let SemanticGeometry::Arc {
                center,
                normal,
                radius,
                start,
                sweep,
            } = arc
            else {
                panic!("expected an arc");
            };
            let (ax, ay, _) = arbitrary_axis(normal);
            let angle = start + sweep * 0.5;
            Point3 {
                x: center.x + ax.x * radius * angle.cos() + ay.x * radius * angle.sin(),
                y: center.y + ax.y * radius * angle.cos() + ay.y * radius * angle.sin(),
                z: center.z + ax.z * radius * angle.cos() + ay.z * radius * angle.sin(),
            }
        })
        .collect()
}

fn assert_point_close(actual: Point3, expected: Point3, context: &str) {
    let drift = ((actual.x - expected.x).powi(2)
        + (actual.y - expected.y).powi(2)
        + (actual.z - expected.z).powi(2))
    .sqrt();
    assert!(drift <= 1e-9, "{context}: {actual:?} vs {expected:?}");
}

#[test]
fn reverse_swaps_a_line_and_undo_restores_it() {
    let line = SemanticGeometry::Line {
        start: p3(0.0, 0.0, 0.0),
        end: p3(10.0, 0.0, 0.0),
    };
    let (mut app, mut session) = build(vec![ent(1, line.clone(), 0)], AppMode::Work);
    session.selection = SelectionSet::from_refs(vec![ref_of(1)]);
    let before = revision(&app);
    let outcome = reverse(&mut app, &mut session).expect("REVERSE must succeed");
    assert_eq!(outcome.diagnostics[0].code, "reverse.applied");
    let SemanticGeometry::Line { start, end } = geometry(&app, 1) else {
        panic!("the entity must stay a line");
    };
    assert_eq!((start, end), (p3(10.0, 0.0, 0.0), p3(0.0, 0.0, 0.0)));
    assert_ne!(revision(&app), before);
    assert_eq!(undo_depth(&app), 1);

    app.execute(&mut session, command(CommandId::Undo, CommandPayload::None))
        .unwrap();
    assert_eq!(geometry(&app, 1), line);
}

#[test]
fn reverse_keeps_every_bulged_segment_midpoint_open_and_closed() {
    for (id, fixture) in [
        (1, open_bulged(true)),
        (2, open_bulged(false)),
        (3, closed_bulged()),
    ] {
        let (mut app, mut session) = build(vec![ent(id, fixture.clone(), 0)], AppMode::Work);
        let before_midpoints = segment_midpoints(&fixture);
        session.selection = SelectionSet::from_refs(vec![ref_of(id)]);
        reverse(&mut app, &mut session).expect("REVERSE must succeed");
        let reversed = geometry(&app, id);
        let after_midpoints = segment_midpoints(&reversed);

        let segments = before_midpoints.len();
        let closed = matches!(&fixture, SemanticGeometry::Polyline { closed: true, .. });
        for i in 0..segments {
            // Closed: segment i is original segment `(n − 2 − i) mod n`; open:
            // segment `segments − 1 − i`.
            let source = if closed {
                (segments + segments - 2 - i) % segments
            } else {
                segments - 1 - i
            };
            assert_point_close(
                after_midpoints[i],
                before_midpoints[source],
                &format!("fixture {id} segment {i}"),
            );
        }
    }
}

#[test]
fn reverse_spline_satisfies_new_t_equals_old_one_minus_t() {
    let original = spline();
    let SemanticGeometry::Spline {
        degree,
        knots,
        control_points,
        weights,
    } = original.clone()
    else {
        panic!();
    };
    let old = NurbsCurve::new(degree as usize, control_points, knots, weights).unwrap();
    let (mut app, mut session) = build(vec![ent(1, original, 0)], AppMode::Work);
    session.selection = SelectionSet::from_refs(vec![ref_of(1)]);
    reverse(&mut app, &mut session).expect("REVERSE must succeed");

    let SemanticGeometry::Spline {
        degree,
        knots,
        control_points,
        weights,
    } = geometry(&app, 1)
    else {
        panic!("the spline must stay a spline");
    };
    let new = NurbsCurve::new(degree as usize, control_points, knots, weights).unwrap();
    let (old_min, old_max) = old.domain();
    let (new_min, new_max) = new.domain();
    for step in 0..=10 {
        let t = step as f64 / 10.0;
        let old_point = old.evaluate(old_min + (1.0 - t) * (old_max - old_min));
        let new_point = new.evaluate(new_min + t * (new_max - new_min));
        assert_point_close(new_point, old_point, &format!("spline t={t}"));
    }
}

#[test]
fn reverse_refuses_an_unsupported_geometry_and_writes_nothing() {
    let arc = SemanticGeometry::Arc {
        center: p3(0.0, 0.0, 0.0),
        normal: p3(0.0, 0.0, 1.0),
        radius: 2.0,
        start: 0.0,
        sweep: 1.0,
    };
    let (mut app, mut session) = build(vec![ent(1, arc, 0)], AppMode::Work);
    session.selection = SelectionSet::from_refs(vec![ref_of(1)]);
    let before = revision(&app);
    let err = reverse(&mut app, &mut session).unwrap_err();
    assert!(
        matches!(err, CadError::Unsupported(ref reason) if reason.starts_with("reverse.unsupported"))
    );
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn reverse_refuses_empty_selection_and_payloads_without_writing() {
    let (mut app, mut session) = build(vec![ent(1, open_bulged(true), 0)], AppMode::Work);
    let before = revision(&app);
    let err = reverse(&mut app, &mut session).unwrap_err();
    assert!(
        matches!(err, CadError::InvalidInput(ref reason) if reason.starts_with("reverse.selection"))
    );

    session.selection = SelectionSet::from_refs(vec![ref_of(1)]);
    let err = app
        .execute(
            &mut session,
            command(
                CommandId::Reverse,
                CommandPayload::Points(vec![p3(0.0, 0.0, 0.0)]),
            ),
        )
        .unwrap_err();
    assert!(
        matches!(err, CadError::InvalidInput(ref reason) if reason.starts_with("reverse.payload"))
    );
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn reverse_refuses_non_addressable_refs_without_writing() {
    let (mut app, mut session) = build(vec![ent(1, open_bulged(true), 0)], AppMode::Work);
    let before = revision(&app);
    let mut reference = ref_of(1);
    reference.instance = InstancePath(vec![EntityId(9)]);
    session.selection = SelectionSet::from_refs(vec![reference]);
    let err = reverse(&mut app, &mut session).unwrap_err();
    assert!(
        matches!(err, CadError::InvalidInput(ref reason) if reason.starts_with("reverse.not_addressable"))
    );
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 0);

    let mut missing = ref_of(99);
    missing.sub_element = Some(SubElementId {
        source_key: "edge".into(),
        topology_revision: Revision(0),
    });
    session.selection = SelectionSet::from_refs(vec![missing]);
    let err = reverse(&mut app, &mut session).unwrap_err();
    assert!(
        matches!(err, CadError::InvalidInput(ref reason) if reason.starts_with("reverse.not_addressable"))
    );
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn reverse_is_refused_in_viewer_mode() {
    let (mut app, mut session) = build(vec![ent(1, open_bulged(true), 0)], AppMode::Viewer);
    session.selection = SelectionSet::from_refs(vec![ref_of(1)]);
    let before = revision(&app);
    let err = reverse(&mut app, &mut session).unwrap_err();
    assert_eq!(err, CadError::PermissionDenied);
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 0);
}
