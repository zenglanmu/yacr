//! `REVCLOUD` v1 contracts: the selection-based **Object mode** that turns one
//! closed straight-segment LWPOLYLINE into a revision cloud.
//!
//! Pinned here: the exact rewrite (same vertices, one `|bulge| = 0.5` per edge,
//! bulging outward from the interior), the single transaction / single undo step,
//! the undo restoring the straight closed rectangle, and every required refusal
//! (`revcloud.payload`, `revcloud.selection`, `revcloud.open_polyline`,
//! `revcloud.bulged_segment`, `revcloud.not_polyline`) changing nothing.

use super::*;
use cad_db::{DbEntity, DbObject, DrawingDatabaseBuilder, Layer};
use cad_geometry::bulge_arc_geometry;

fn ent(id: u128, geometry: SemanticGeometry) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbPolyline".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry,
        draw_order: id as i64,
    }
}

/// A fresh application whose document holds exactly `entities` (layer 0).
fn app_with_entities(entities: Vec<DbEntity>) -> (Application, SessionState) {
    let (mut app, session) = application_with_document();
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
    app.workspace
        .documents
        .get_mut(&DocumentId(1))
        .unwrap()
        .drawing = Arc::new(drawing);
    (app, session)
}

fn ref_of(entity: u128) -> SelectionRef {
    SelectionRef {
        document: DocumentId(1),
        entity: EntityId(entity),
        instance: InstancePath::default(),
        sub_element: None,
    }
}

fn execute(
    app: &mut Application,
    session: &mut SessionState,
    id: CommandId,
    payload: CommandPayload,
) -> CadResult<CommandOutcome> {
    app.execute(session, command(id, payload))
}

fn select(app: &mut Application, session: &mut SessionState, entities: &[u128]) {
    execute(
        app,
        session,
        CommandId::Select,
        CommandPayload::Selection(entities.iter().copied().map(ref_of).collect()),
    )
    .unwrap();
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

fn stored_polyline(app: &Application, entity: u128) -> (Vec<Point3>, Vec<f64>, bool) {
    match &app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(entity))
        .unwrap_or_else(|| panic!("entity {entity} must exist"))
        .geometry
    {
        SemanticGeometry::Polyline {
            points,
            bulges,
            closed,
        } => (points.clone(), bulges.clone(), *closed),
        other => panic!("expected an LWPOLYLINE, got {other:?}"),
    }
}

/// The message of a named refusal (`InvalidInput` or `Unsupported`).
fn refusal(error: CadError) -> String {
    match error {
        CadError::InvalidInput(message) | CadError::Unsupported(message) => message,
        other => panic!("expected a named refusal, got {other:?}"),
    }
}

/// Assert that a refused invocation changed nothing at all.
fn state(app: &Application) -> (Revision, usize) {
    (revision(app), undo_depth(app))
}

fn assert_untouched(app: &Application, before: (Revision, usize)) {
    assert_eq!(revision(app), before.0, "a refusal bumps no revision");
    assert_eq!(undo_depth(app), before.1, "a refusal records no undo entry");
}

#[test]
fn revcloud_turns_a_closed_rectangle_into_one_outward_arc_per_edge() {
    let (mut app, mut session) = application_with_document();
    let created = execute(
        &mut app,
        &mut session,
        CommandId::CreateRectangle,
        CommandPayload::Points(vec![point(0.0, 0.0), point(4.0, 3.0)]),
    )
    .expect("RECTANG creates the fixture polyline");
    let entity = created.objects[0].0;
    select(&mut app, &mut session, &[entity]);
    let before = state(&app);

    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::RevisionCloud,
        CommandPayload::None,
    )
    .expect("REVCLOUD converts the closed straight rectangle");

    // One entity rewritten in place: one change, one transaction, one undo step.
    assert_eq!(outcome.objects, vec![ObjectId(entity)]);
    assert_eq!(
        outcome.changes.as_ref().unwrap().changes.len(),
        1,
        "the polyline is rewritten in place (no insert, no delete)"
    );
    assert_eq!(
        revision(&app).0,
        before.0 .0 + 1,
        "exactly one revision bump"
    );
    assert_eq!(undo_depth(&app), before.1 + 1, "exactly one undo step");
    assert_eq!(outcome.diagnostics.len(), 1);
    assert_eq!(outcome.diagnostics[0].code, "revcloud.applied");
    assert!(
        outcome.diagnostics[0].message.contains('4'),
        "the diagnostic names the edge count: {}",
        outcome.diagnostics[0].message
    );
    assert!(
        outcome.diagnostics[0].message.is_ascii(),
        "the diagnostic is ASCII: {}",
        outcome.diagnostics[0].message
    );

    let (points, bulges, closed) = stored_polyline(&app, entity);
    assert!(closed, "the revision cloud stays closed");
    assert_eq!(points.len(), 4, "the vertices themselves do not move");
    assert_eq!(points[0], point(0.0, 0.0));
    assert_eq!(points[1], point(4.0, 0.0));
    assert_eq!(points[2], point(4.0, 3.0));
    assert_eq!(points[3], point(0.0, 3.0));
    assert_eq!(bulges.len(), points.len(), "one bulge per vertex");
    for bulge in &bulges {
        assert!(
            (bulge.abs() - 0.5).abs() < 1e-12,
            "every edge has bulge magnitude 0.5: {bulge}"
        );
        // `rectangle_geometry` canonicalises a counter-clockwise ring, so outward
        // is the positive bulge in this build's bulge convention.
        assert!(*bulge > 0.0, "a CCW ring bulges outward with +0.5: {bulge}");
    }

    // The arcs really are outside the rectangle: each arc's midpoint is farther
    // from the interior centre than its chord's midpoint.
    let centre = point(2.0, 1.5);
    let normal = Point3 {
        x: 0.0,
        y: 0.0,
        z: 1.0,
    };
    for index in 0..points.len() {
        let start = points[index];
        let end = points[(index + 1) % points.len()];
        let chord_mid = Point3 {
            x: (start.x + end.x) * 0.5,
            y: (start.y + end.y) * 0.5,
            z: (start.z + end.z) * 0.5,
        };
        let Some(SemanticGeometry::Arc {
            center,
            radius,
            start: arc_start,
            sweep,
            ..
        }) = bulge_arc_geometry(start, end, bulges[index], normal)
        else {
            panic!("a 0.5 bulge is an arc");
        };
        let middle = arc_start + sweep * 0.5;
        let arc_mid = Point3 {
            x: center.x + radius * middle.cos(),
            y: center.y + radius * middle.sin(),
            z: centre.z,
        };
        let chord_distance = (chord_mid.x - centre.x).hypot(chord_mid.y - centre.y);
        let arc_distance = (arc_mid.x - centre.x).hypot(arc_mid.y - centre.y);
        assert!(
            arc_distance > chord_distance,
            "edge {index}: the arc bulges outward ({arc_distance} > {chord_distance})"
        );
    }

    // One undo restores the straight closed rectangle exactly.
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    let (restored, restored_bulges, restored_closed) = stored_polyline(&app, entity);
    assert!(restored_closed);
    assert_eq!(
        restored,
        vec![
            point(0.0, 0.0),
            point(4.0, 0.0),
            point(4.0, 3.0),
            point(0.0, 3.0),
        ]
    );
    assert_eq!(restored_bulges, vec![0.0; 4]);
}

#[test]
fn open_polyline_is_refused_by_name_and_changes_nothing() {
    let (mut app, mut session) = app_with_entities(vec![ent(
        1,
        SemanticGeometry::Polyline {
            points: vec![point(0.0, 0.0), point(4.0, 0.0), point(4.0, 3.0)],
            bulges: vec![0.0; 3],
            closed: false,
        },
    )]);
    select(&mut app, &mut session, &[1]);
    let before = state(&app);
    let message = refusal(
        execute(
            &mut app,
            &mut session,
            CommandId::RevisionCloud,
            CommandPayload::None,
        )
        .unwrap_err(),
    );
    assert!(
        message.contains("revcloud.open_polyline"),
        "the refusal names its code: {message}"
    );
    assert_untouched(&app, before);
}

#[test]
fn bulged_closed_polyline_is_refused_instead_of_being_overwritten() {
    let (mut app, mut session) = app_with_entities(vec![ent(
        1,
        SemanticGeometry::Polyline {
            points: vec![
                point(0.0, 0.0),
                point(4.0, 0.0),
                point(4.0, 3.0),
                point(0.0, 3.0),
            ],
            bulges: vec![0.5, 0.0, 0.0, 0.0],
            closed: true,
        },
    )]);
    select(&mut app, &mut session, &[1]);
    let before = state(&app);
    let message = refusal(
        execute(
            &mut app,
            &mut session,
            CommandId::RevisionCloud,
            CommandPayload::None,
        )
        .unwrap_err(),
    );
    assert!(
        message.contains("revcloud.bulged_segment"),
        "the refusal names its code: {message}"
    );
    assert_untouched(&app, before);
    // The stored arc segment is untouched.
    let (_, bulges, _) = stored_polyline(&app, 1);
    assert_eq!(bulges, vec![0.5, 0.0, 0.0, 0.0]);
}

#[test]
fn non_polyline_is_refused_by_name_and_changes_nothing() {
    let (mut app, mut session) = app_with_entities(vec![ent(
        1,
        SemanticGeometry::Line {
            start: point(0.0, 0.0),
            end: point(4.0, 0.0),
        },
    )]);
    select(&mut app, &mut session, &[1]);
    let before = state(&app);
    let message = refusal(
        execute(
            &mut app,
            &mut session,
            CommandId::RevisionCloud,
            CommandPayload::None,
        )
        .unwrap_err(),
    );
    assert!(
        message.contains("revcloud.not_polyline"),
        "the refusal names its code: {message}"
    );
    assert_untouched(&app, before);
}

#[test]
fn payload_is_refused_by_name_and_changes_nothing() {
    let (mut app, mut session) = application_with_document();
    let created = execute(
        &mut app,
        &mut session,
        CommandId::CreateRectangle,
        CommandPayload::Points(vec![point(0.0, 0.0), point(4.0, 3.0)]),
    )
    .unwrap();
    select(&mut app, &mut session, &[created.objects[0].0]);
    let before = state(&app);
    let message = refusal(
        execute(
            &mut app,
            &mut session,
            CommandId::RevisionCloud,
            CommandPayload::Points(vec![point(0.0, 0.0)]),
        )
        .unwrap_err(),
    );
    assert!(
        message.contains("revcloud.payload"),
        "the refusal names its code: {message}"
    );
    assert_untouched(&app, before);
    let (_, bulges, _) = stored_polyline(&app, created.objects[0].0);
    assert_eq!(bulges, vec![0.0; 4], "the straight rectangle is untouched");
}

#[test]
fn zero_or_multiple_selection_is_refused_by_name_and_changes_nothing() {
    let (mut app, mut session) = application_with_document();
    let first = execute(
        &mut app,
        &mut session,
        CommandId::CreateRectangle,
        CommandPayload::Points(vec![point(0.0, 0.0), point(4.0, 3.0)]),
    )
    .unwrap()
    .objects[0]
        .0;
    let second = execute(
        &mut app,
        &mut session,
        CommandId::CreateRectangle,
        CommandPayload::Points(vec![point(10.0, 10.0), point(14.0, 13.0)]),
    )
    .unwrap()
    .objects[0]
        .0;
    let before = state(&app);

    for selection in [Vec::new(), vec![first, second]] {
        select(&mut app, &mut session, &selection);
        let message = refusal(
            execute(
                &mut app,
                &mut session,
                CommandId::RevisionCloud,
                CommandPayload::None,
            )
            .unwrap_err(),
        );
        assert!(
            message.contains("revcloud.selection"),
            "the refusal names its code: {message}"
        );
        assert_eq!(revision(&app), before.0, "a refusal bumps no revision");
    }
    assert_eq!(
        undo_depth(&app),
        before.1,
        "a refusal records no undo entry"
    );
    for entity in [first, second] {
        let (_, bulges, _) = stored_polyline(&app, entity);
        assert_eq!(bulges, vec![0.0; 4], "both rectangles stay straight");
    }
}
