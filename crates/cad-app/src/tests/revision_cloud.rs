//! `REVCLOUD` contracts: Object, Rectangular, Polygonal, Freehand, Arc Length,
//! Style and Modify.
//!
//! Pinned here: the declared constant-chord arc rule, outward arc sign, the
//! deterministic 5 %-of-bbox-diagonal default, one transaction / one undo step,
//! and every named refusal writing nothing.

use super::*;
use crate::command_payload::{
    RevCloudStyle, RevisionCloudMode, RevisionCloudOptions, UtilityPayload,
};
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

fn cloud_payload(options: RevisionCloudOptions) -> CommandPayload {
    CommandPayload::Annotation(Box::new(AnnotationBatchPayload::Utility(Box::new(
        UtilityPayload::RevisionCloud(options),
    ))))
}

fn object_payload() -> CommandPayload {
    cloud_payload(RevisionCloudOptions {
        mode: RevisionCloudMode::Object,
        arc_length: None,
    })
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

fn state(app: &Application) -> (Revision, usize) {
    (revision(app), undo_depth(app))
}

fn assert_untouched(app: &Application, before: (Revision, usize)) {
    assert_eq!(revision(app), before.0, "a refusal bumps no revision");
    assert_eq!(undo_depth(app), before.1, "a refusal records no undo entry");
}

fn assert_chord(points: &[Point3], closed: bool, chord: f64) {
    let segments = if closed {
        points.len()
    } else {
        points.len() - 1
    };
    for index in 0..segments {
        let a = points[index];
        let b = points[(index + 1) % points.len()];
        let length = (a.x - b.x).hypot(a.y - b.y).hypot(a.z - b.z);
        assert!(
            (length - chord).abs() <= chord * 1e-6 + 1e-9,
            "segment {index} has chord {length}, expected {chord}"
        );
    }
}

#[test]
fn revcloud_object_uses_the_explicit_chord_and_undo_restores_the_source() {
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
        cloud_payload(RevisionCloudOptions {
            mode: RevisionCloudMode::Object,
            arc_length: Some(1.0),
        }),
    )
    .expect("REVCLOUD converts the closed rectangle");

    assert_eq!(outcome.objects, vec![ObjectId(entity)]);
    assert_eq!(outcome.changes.as_ref().unwrap().changes.len(), 1);
    assert_eq!(revision(&app).0, before.0 .0 + 1);
    assert_eq!(undo_depth(&app), before.1 + 1);
    assert_eq!(outcome.diagnostics[0].code, "revcloud.applied");
    assert!(outcome.diagnostics[0].message.is_ascii());

    let (points, bulges, closed) = stored_polyline(&app, entity);
    assert!(closed, "the rectangle stays closed");
    assert_eq!(points.len(), 14, "perimeter 14 / chord 1 = 14 arcs");
    assert_chord(&points, true, 1.0);
    assert_eq!(bulges.len(), points.len());
    for bulge in &bulges {
        assert!((bulge.abs() - 0.5).abs() < 1e-12);
        assert!(*bulge > 0.0, "a CCW ring bulges outward: {bulge}");
    }

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
fn revcloud_default_arc_length_is_five_percent_of_the_path_diagonal() {
    let (mut app, mut session) = application_with_document();
    let created = execute(
        &mut app,
        &mut session,
        CommandId::CreateRectangle,
        CommandPayload::Points(vec![point(0.0, 0.0), point(4.0, 3.0)]),
    )
    .unwrap();
    select(&mut app, &mut session, &[created.objects[0].0]);
    execute(
        &mut app,
        &mut session,
        CommandId::RevisionCloud,
        object_payload(),
    )
    .expect("the bare Object form uses the default chord");
    let (points, _, closed) = stored_polyline(&app, created.objects[0].0);
    let diagonal: f64 = 5.0;
    let chord: f64 = diagonal * 0.05;
    assert!(closed);
    assert_eq!(points.len(), (14.0 / chord).ceil() as usize);
    assert_chord(&points, true, chord);
}

#[test]
fn revcloud_object_converts_a_circle_and_accepts_an_open_polyline() {
    let circle = ent(
        1,
        SemanticGeometry::Circle {
            center: point(0.0, 0.0),
            normal: Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
            radius: 2.0,
        },
    );
    let open = ent(
        2,
        SemanticGeometry::Polyline {
            points: vec![point(10.0, 0.0), point(13.0, 0.0), point(13.0, 3.0)],
            bulges: vec![0.0; 3],
            closed: false,
        },
    );
    let (mut app, mut session) = app_with_entities(vec![circle, open]);

    select(&mut app, &mut session, &[1]);
    execute(
        &mut app,
        &mut session,
        CommandId::RevisionCloud,
        cloud_payload(RevisionCloudOptions {
            mode: RevisionCloudMode::Object,
            arc_length: Some(1.0),
        }),
    )
    .expect("a circle is converted");
    let (points, bulges, closed) = stored_polyline(&app, 1);
    assert!(closed);
    assert_eq!(
        points.len(),
        (std::f64::consts::TAU * 2.0 / 1.0).ceil() as usize
    );
    assert!(bulges.iter().all(|bulge| *bulge > 0.0));

    select(&mut app, &mut session, &[2]);
    execute(
        &mut app,
        &mut session,
        CommandId::RevisionCloud,
        cloud_payload(RevisionCloudOptions {
            mode: RevisionCloudMode::Object,
            arc_length: Some(1.0),
        }),
    )
    .expect("an open polyline becomes an open cloud");
    let (_, _, closed) = stored_polyline(&app, 2);
    assert!(!closed, "an open source path stays open");
}

#[test]
fn revcloud_rectangular_polygonal_and_freehand_build_the_documented_rings() {
    let (mut app, mut session) = app_with_entities(Vec::new());

    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::RevisionCloud,
        cloud_payload(RevisionCloudOptions {
            mode: RevisionCloudMode::Rectangular {
                first: point(0.0, 0.0),
                second: point(4.0, 4.0),
                reverse: false,
            },
            arc_length: Some(1.0),
        }),
    )
    .expect("rectangular cloud");
    let rect = outcome.objects[0].0;
    let (points, bulges, closed) = stored_polyline(&app, rect);
    assert!(closed && points.len() == 16);
    assert!(bulges.iter().all(|bulge| *bulge > 0.0));

    execute(
        &mut app,
        &mut session,
        CommandId::RevisionCloud,
        cloud_payload(RevisionCloudOptions {
            mode: RevisionCloudMode::Polygonal {
                points: vec![
                    point(10.0, 0.0),
                    point(14.0, 0.0),
                    point(14.0, 4.0),
                    point(10.0, 4.0),
                ],
                closed: true,
                reverse: true,
            },
            arc_length: Some(1.0),
        }),
    )
    .expect("polygonal cloud");
    let polygonal = find_last_cloud(&app, rect);
    let (_, polygonal_bulges, polygonal_closed) = stored_polyline(&app, polygonal);
    assert!(polygonal_closed);
    assert!(
        polygonal_bulges.iter().all(|bulge| *bulge < 0.0),
        "reverse flips the arc side"
    );

    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::RevisionCloud,
        cloud_payload(RevisionCloudOptions {
            mode: RevisionCloudMode::Freehand {
                points: vec![point(20.0, 0.0), point(24.0, 0.0), point(24.0, 4.0)],
                closed: false,
                reverse: false,
            },
            arc_length: Some(1.0),
        }),
    )
    .expect("freehand open cloud");
    let (points, _, closed) = stored_polyline(&app, outcome.objects[0].0);
    assert!(!closed);
    assert!(points.len() > 3);
}

fn find_last_cloud(app: &Application, except: u128) -> u128 {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .entities()
        .filter(|entity| entity.id.0 != except)
        .filter(|entity| matches!(entity.geometry, SemanticGeometry::Polyline { .. }))
        .map(|entity| entity.id.0)
        .max()
        .expect("the polygonal cloud")
}

#[test]
fn revcloud_modify_adds_points_and_opens_the_picked_side() {
    let (mut app, mut session) = app_with_entities(Vec::new());
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::RevisionCloud,
        cloud_payload(RevisionCloudOptions {
            mode: RevisionCloudMode::Rectangular {
                first: point(0.0, 0.0),
                second: point(4.0, 4.0),
                reverse: false,
            },
            arc_length: Some(1.0),
        }),
    )
    .unwrap();
    let target = outcome.objects[0].0;
    let before = state(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::RevisionCloud,
        cloud_payload(RevisionCloudOptions {
            mode: RevisionCloudMode::Modify {
                target: ref_of(target),
                add_points: vec![point(2.0, 2.0)],
                erase_side: point(0.0, 0.0),
            },
            arc_length: Some(1.0),
        }),
    )
    .expect("Modify rewrites the cloud");
    assert_eq!(outcome.objects, vec![ObjectId(target)]);
    let (_, _, closed) = stored_polyline(&app, target);
    assert!(!closed, "erasing a side opens the cloud");
    assert_eq!(revision(&app).0, before.0 .0 + 1);
    assert_eq!(undo_depth(&app), before.1 + 1);
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    let (_, _, closed) = stored_polyline(&app, target);
    assert!(closed, "undo restores the closed cloud");
}

#[test]
fn revcloud_refuses_calligraphy_and_bad_arc_lengths_without_writing() {
    let (mut app, mut session) = application_with_document();
    let before = state(&app);
    let message = refusal(
        execute(
            &mut app,
            &mut session,
            CommandId::RevisionCloud,
            cloud_payload(RevisionCloudOptions {
                mode: RevisionCloudMode::Style(RevCloudStyle::Calligraphy),
                arc_length: None,
            }),
        )
        .unwrap_err(),
    );
    assert!(message.contains("revcloud.style_unsupported"), "{message}");
    assert_untouched(&app, before);

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
            cloud_payload(RevisionCloudOptions {
                mode: RevisionCloudMode::Object,
                arc_length: Some(0.0),
            }),
        )
        .unwrap_err(),
    );
    assert!(message.contains("revcloud.invalid_arc_length"), "{message}");
    assert_untouched(&app, before);
}

#[test]
fn revcloud_refuses_a_line_and_a_multi_selection_without_writing() {
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
            object_payload(),
        )
        .unwrap_err(),
    );
    assert!(message.contains("revcloud.not_convertible"), "{message}");
    assert_untouched(&app, before);

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
    select(&mut app, &mut session, &[first, second]);
    let before = state(&app);
    let message = refusal(
        execute(
            &mut app,
            &mut session,
            CommandId::RevisionCloud,
            object_payload(),
        )
        .unwrap_err(),
    );
    assert!(message.contains("revcloud.selection"), "{message}");
    assert_untouched(&app, before);
}

#[test]
fn revcloud_rejects_a_wrong_payload_without_writing() {
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
    assert!(message.contains("revcloud.payload"), "{message}");
    assert_untouched(&app, before);
}

#[test]
fn revcloud_bulge_arcs_really_bulge_outward() {
    let (mut app, mut session) = application_with_document();
    let created = execute(
        &mut app,
        &mut session,
        CommandId::CreateRectangle,
        CommandPayload::Points(vec![point(0.0, 0.0), point(4.0, 3.0)]),
    )
    .unwrap();
    let entity = created.objects[0].0;
    select(&mut app, &mut session, &[entity]);
    execute(
        &mut app,
        &mut session,
        CommandId::RevisionCloud,
        cloud_payload(RevisionCloudOptions {
            mode: RevisionCloudMode::Object,
            arc_length: Some(1.0),
        }),
    )
    .unwrap();
    let (points, bulges, _) = stored_polyline(&app, entity);
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
            arc_distance >= chord_distance,
            "edge {index} must not bulge inward"
        );
    }
}
