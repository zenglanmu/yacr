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

// --- OOPS --------------------------------------------------------------------

fn closed_rectangle() -> SemanticGeometry {
    SemanticGeometry::Polyline {
        points: vec![
            p3(0.0, 0.0, 0.0),
            p3(6.0, 0.0, 0.0),
            p3(6.0, 3.0, 0.0),
            p3(0.0, 3.0, 0.0),
        ],
        bulges: vec![0.0; 4],
        closed: true,
    }
}

fn build_rectangle_with_color() -> (Application, SessionState) {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder
        .insert_entity(ent(1, closed_rectangle(), 0))
        .unwrap();
    builder
        .set_entity_render_attributes(
            EntityId(1),
            cad_db::EntityRenderAttributes {
                color: cad_db::EntityColor::Explicit([12, 34, 56]),
                ..Default::default()
            },
        )
        .unwrap();
    let drawing = builder.finish().unwrap();
    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([8u8; 32]),
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

#[test]
fn oops_restores_the_last_erased_rectangle_with_a_new_id_and_one_undo_step() {
    let (mut app, mut session) = build_rectangle_with_color();
    let erased_color = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity_display_state(EntityId(1))
        .attributes
        .color
        .clone();
    session.selection = SelectionSet::from_refs(vec![ref_of(1)]);
    app.execute(
        &mut session,
        command(CommandId::EraseEntities, CommandPayload::None),
    )
    .unwrap();
    assert!(app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(1))
        .is_none());
    assert_eq!(undo_depth(&app), 1);

    let outcome = app
        .execute(&mut session, command(CommandId::Oops, CommandPayload::None))
        .expect("OOPS must restore");
    assert_eq!(outcome.diagnostics[0].code, "oops.applied");
    assert_eq!(undo_depth(&app), 2);

    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    let restored: Vec<_> = drawing.entities().collect();
    assert_eq!(restored.len(), 1);
    let restored = restored[0];
    assert_ne!(restored.id, EntityId(1), "a restore gets a fresh id");
    assert_eq!(restored.geometry, closed_rectangle());
    assert_eq!(restored.layer, LayerId(0));
    assert_eq!(restored.space, SpaceId::Model);
    assert_eq!(
        drawing.entity_display_state(restored.id).attributes.color,
        erased_color,
        "the render attributes are restored"
    );

    // The snapshot is consumed: a second OOPS is a no-op that writes nothing.
    let rev = revision(&app);
    let outcome = app
        .execute(&mut session, command(CommandId::Oops, CommandPayload::None))
        .unwrap();
    assert_eq!(outcome.diagnostics[0].code, "oops.nothing");
    assert!(outcome.changes.is_none());
    assert_eq!(revision(&app), rev);
    assert_eq!(undo_depth(&app), 2);

    // Undo of OOPS removes the restored copy but does not recreate the snapshot.
    app.execute(&mut session, command(CommandId::Undo, CommandPayload::None))
        .unwrap();
    assert_eq!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity_count(),
        0
    );
    let outcome = app
        .execute(&mut session, command(CommandId::Oops, CommandPayload::None))
        .unwrap();
    assert_eq!(outcome.diagnostics[0].code, "oops.nothing");
}

#[test]
fn oops_without_an_erase_is_a_named_no_op() {
    let (mut app, mut session) = build(vec![ent(1, closed_rectangle(), 0)], AppMode::Work);
    let rev = revision(&app);
    let outcome = app
        .execute(&mut session, command(CommandId::Oops, CommandPayload::None))
        .unwrap();
    assert_eq!(outcome.diagnostics[0].code, "oops.nothing");
    assert!(outcome.changes.is_none());
    assert_eq!(revision(&app), rev);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn oops_refuses_a_payload_and_never_stores_a_foreign_document_snapshot() {
    let (mut app, mut session) = build(vec![ent(1, closed_rectangle(), 0)], AppMode::Work);
    let err = app
        .execute(
            &mut session,
            command(
                CommandId::Oops,
                CommandPayload::Points(vec![p3(0.0, 0.0, 0.0)]),
            ),
        )
        .unwrap_err();
    assert!(
        matches!(err, CadError::InvalidInput(ref reason) if reason.starts_with("oops.payload"))
    );

    // A snapshot for another document is dropped by the session setter, so the
    // foreign-document restore path is unreachable through the public API.
    session.set_last_erase(Some(crate::session_state::EraseSnapshot {
        document: DocumentId(2),
        entries: Vec::new(),
    }));
    assert!(session.last_erase().is_none());
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn oops_is_refused_in_viewer_mode() {
    let (mut app, mut session) = build(vec![ent(1, closed_rectangle(), 0)], AppMode::Viewer);
    let rev = revision(&app);
    let err = app
        .execute(&mut session, command(CommandId::Oops, CommandPayload::None))
        .unwrap_err();
    assert_eq!(err, CadError::PermissionDenied);
    assert_eq!(revision(&app), rev);
    assert_eq!(undo_depth(&app), 0);
}

// --- OVERKILL ----------------------------------------------------------------

fn ent_on(id: u128, layer: LayerId, geometry: SemanticGeometry, order: i64) -> DbEntity {
    let mut entity = ent(id, geometry, order);
    entity.layer = layer;
    entity
}

fn build_layered(entities: Vec<DbEntity>) -> (Application, SessionState) {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    for (id, name) in [(LayerId(0), "0"), (LayerId(7), "WALLS")] {
        builder
            .insert_layer(Layer {
                id,
                name: name.into(),
                visible: true,
            })
            .unwrap();
    }
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

fn line(x0: f64, y0: f64, x1: f64, y1: f64) -> SemanticGeometry {
    SemanticGeometry::Line {
        start: p3(x0, y0, 0.0),
        end: p3(x1, y1, 0.0),
    }
}

fn entity_count(app: &Application) -> usize {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity_count()
}

#[test]
fn overkill_deletes_exact_duplicates_and_keeps_the_lowest_draw_order() {
    let a = line(0.0, 0.0, 10.0, 0.0);
    let mut reversed = line(0.0, 0.0, 10.0, 0.0);
    if let SemanticGeometry::Line { start, end } = &mut reversed {
        std::mem::swap(start, end);
    }
    let (mut app, mut session) = build_layered(vec![
        ent_on(1, LayerId(0), a.clone(), 5),
        ent_on(2, LayerId(0), a.clone(), 1),
        // Same unordered endpoint pair: a duplicate under the v1 rule.
        ent_on(3, LayerId(0), reversed, 9),
        // Same geometry on another layer: never merged or deleted.
        ent_on(5, LayerId(7), a, 0),
        // A different line stays.
        ent_on(4, LayerId(0), line(0.0, 0.0, 10.0, 5.0), 2),
    ]);
    session.selection = SelectionSet::from_refs(
        [1u128, 2, 3, 4, 5]
            .into_iter()
            .map(ref_of)
            .collect::<Vec<_>>(),
    );
    let rev = revision(&app);
    let outcome = app
        .execute(&mut session, command(CommandId::Overkill, CommandPayload::None))
        .expect("OVERKILL must succeed");
    assert!(outcome
        .diagnostics
        .iter()
        .any(|d| d.code == "overkill.applied"));
    assert!(outcome
        .diagnostics
        .iter()
        .any(|d| d.code == "overkill.v1_exact_duplicates_only"));
    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    assert!(drawing.entity(EntityId(1)).is_none(), "higher draw_order deleted");
    assert!(drawing.entity(EntityId(2)).is_some(), "keeper stays");
    assert!(drawing.entity(EntityId(3)).is_none(), "reversed duplicate deleted");
    assert!(drawing.entity(EntityId(4)).is_some(), "different line stays");
    assert!(
        drawing.entity(EntityId(5)).is_some(),
        "same geometry on another layer is not a duplicate"
    );
    assert_ne!(revision(&app), rev);
    assert_eq!(undo_depth(&app), 1);

    // One undo restores both deleted rows.
    app.execute(&mut session, command(CommandId::Undo, CommandPayload::None))
        .unwrap();
    assert_eq!(entity_count(&app), 5);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn overkill_without_duplicates_records_no_transaction() {
    let (mut app, mut session) = build_layered(vec![
        ent_on(1, LayerId(0), line(0.0, 0.0, 10.0, 0.0), 0),
        // Collinear but partially overlapping: v1 never merges it.
        ent_on(2, LayerId(0), line(5.0, 0.0, 20.0, 0.0), 1),
    ]);
    session.selection = SelectionSet::from_refs(vec![ref_of(1), ref_of(2)]);
    let rev = revision(&app);
    let outcome = app
        .execute(&mut session, command(CommandId::Overkill, CommandPayload::None))
        .unwrap();
    assert!(outcome.changes.is_none());
    assert!(outcome
        .diagnostics
        .iter()
        .any(|d| d.code == "overkill.unchanged"));
    assert!(outcome
        .diagnostics
        .iter()
        .any(|d| d.code == "overkill.v1_exact_duplicates_only"));
    assert_eq!(revision(&app), rev);
    assert_eq!(undo_depth(&app), 0);
    assert_eq!(entity_count(&app), 2);
}

#[test]
fn overkill_refuses_non_addressable_and_empty_selections() {
    let (mut app, mut session) = build_layered(vec![ent_on(
        1,
        LayerId(0),
        line(0.0, 0.0, 10.0, 0.0),
        0,
    )]);
    let rev = revision(&app);
    let err = app
        .execute(&mut session, command(CommandId::Overkill, CommandPayload::None))
        .unwrap_err();
    assert!(matches!(err, CadError::InvalidInput(ref reason) if reason.starts_with("overkill.selection")));

    let mut reference = ref_of(1);
    reference.instance = InstancePath(vec![EntityId(9)]);
    session.selection = SelectionSet::from_refs(vec![reference]);
    let err = app
        .execute(&mut session, command(CommandId::Overkill, CommandPayload::None))
        .unwrap_err();
    assert!(matches!(err, CadError::InvalidInput(ref reason) if reason.starts_with("overkill.not_addressable")));
    assert_eq!(revision(&app), rev);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn overkill_is_refused_in_viewer_mode() {
    let (mut app, mut session) = build_layered(vec![
        ent_on(1, LayerId(0), line(0.0, 0.0, 10.0, 0.0), 0),
        ent_on(2, LayerId(0), line(0.0, 0.0, 10.0, 0.0), 1),
    ]);
    let mut session_viewer = SessionState::new(DocumentId(1), AppMode::Viewer);
    session_viewer.selection = SelectionSet::from_refs(vec![ref_of(1), ref_of(2)]);
    let rev = revision(&app);
    let err = app
        .execute(
            &mut session_viewer,
            command(CommandId::Overkill, CommandPayload::None),
        )
        .unwrap_err();
    assert_eq!(err, CadError::PermissionDenied);
    assert_eq!(revision(&app), rev);
    assert_eq!(undo_depth(&app), 0);
    let _ = &mut session;
}

