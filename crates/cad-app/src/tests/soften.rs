//! `SOFTENEDGES` command contracts.
//!
//! The command writes one display attribute (`render_attributes.soften`) on each
//! selected mesh/body entity: one revision, one undo step, geometry untouched.
//! These tests pin the value, the no-op, every named refusal and undo/redo.

use super::*;
use cad_db::{DbEntity, DbObject, DrawingDatabaseBuilder, Layer};
use cad_domain::{Body3d, BodyDisplay, BodyExact, BodyKind, Completeness, Mesh, Precision};

fn body(id: u128) -> DbEntity {
    let corners = [
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        Point3 {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        },
        Point3 {
            x: 1.0,
            y: 1.0,
            z: 0.0,
        },
        Point3 {
            x: 0.0,
            y: 1.0,
            z: 0.0,
        },
    ];
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDb3dSolid".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Body3d(Box::new(Body3d {
            // A display-only body: the soften command is display state and does
            // not need an exact B-rep.
            kind: BodyKind::Unknown,
            exact: BodyExact::Unavailable {
                reason: "display-only soften fixture".into(),
            },
            display: BodyDisplay {
                mesh: Mesh {
                    vertices: corners.to_vec(),
                    triangles: vec![[0, 1, 2], [0, 2, 3]],
                    normals: Vec::new(),
                    face_sources: Vec::new(),
                    colors: Vec::new(),
                    uvs: Vec::new(),
                    texture: None,
                    pbr: None,
                },
                edges: vec![
                    vec![corners[0], corners[1]],
                    vec![corners[1], corners[2]],
                    vec![corners[2], corners[3]],
                    vec![corners[3], corners[0]],
                ],
                tolerance: None,
                precision: Precision::Analytic,
                completeness: Completeness::Complete,
            },
        })),
        draw_order: id as i64,
    }
}

fn app_with(entities: Vec<DbEntity>) -> (Application, SessionState) {
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
    let (app, session, _) = crate::tests::inquiry::app_from_builder(builder);
    (app, session)
}

fn select(app: &mut Application, session: &mut SessionState, entities: &[u128]) {
    let refs: Vec<SelectionRef> = entities
        .iter()
        .map(|id| SelectionRef {
            document: DocumentId(1),
            entity: EntityId(*id),
            instance: InstancePath::default(),
            sub_element: None,
        })
        .collect();
    app.execute(
        session,
        command(CommandId::Select, CommandPayload::Selection(refs)),
    )
    .expect("selection");
}

fn soften(
    app: &mut Application,
    session: &mut SessionState,
    payload: CommandPayload,
) -> CadResult<CommandOutcome> {
    app.execute(session, command(CommandId::SoftenEdges, payload))
}

fn revision(app: &Application) -> Revision {
    app.workspace.documents[&DocumentId(1)].drawing.revision()
}

fn soften_of(app: &Application, id: u128) -> Option<f64> {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity_render_attributes(EntityId(id))
        .soften
}

fn refusal_text(error: CadError) -> String {
    match error {
        CadError::InvalidInput(text) | CadError::Unsupported(text) => text,
        other => panic!("expected a named refusal, got {other:?}"),
    }
}

#[test]
fn a_bare_soften_applies_the_default_angle_and_undo_restores_it() {
    let (mut app, mut session) = app_with(vec![body(7)]);
    select(&mut app, &mut session, &[7]);
    let before = revision(&app);
    let outcome = soften(&mut app, &mut session, CommandPayload::None).expect("soften");
    assert_eq!(revision(&app), Revision(before.0 + 1));
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 1);
    assert_eq!(
        soften_of(&app, 7),
        Some(20.0),
        "a bare SOFTENEDGES uses 20°"
    );
    let applied = outcome
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "soften.applied")
        .expect("the success is named");
    assert!(applied.message.contains("20"), "{}", applied.message);

    app.execute(&mut session, command(CommandId::Undo, CommandPayload::None))
        .expect("undo");
    assert_eq!(soften_of(&app, 7), None, "undo restores no soften");
    app.execute(&mut session, command(CommandId::Redo, CommandPayload::None))
        .expect("redo");
    assert_eq!(soften_of(&app, 7), Some(20.0));
}

#[test]
fn a_typed_angle_is_stored_and_the_same_angle_writes_nothing() {
    let (mut app, mut session) = app_with(vec![body(7)]);
    select(&mut app, &mut session, &[7]);
    soften(&mut app, &mut session, CommandPayload::SoftenEdges(45.0)).expect("soften 45");
    assert_eq!(soften_of(&app, 7), Some(45.0));
    let before = revision(&app);
    let depth = app.history[&DocumentId(1)].undo_depth();
    let outcome = soften(&mut app, &mut session, CommandPayload::SoftenEdges(45.0)).expect("no-op");
    assert_eq!(revision(&app), before, "no revision");
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), depth);
    assert!(outcome
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "soften.unchanged"));
}

#[test]
fn every_refusal_is_named_and_writes_nothing() {
    let (mut app, mut session) = app_with(vec![body(7)]);
    select(&mut app, &mut session, &[7]);
    let before = revision(&app);
    for angle in [181.0, -1.0, f64::NAN] {
        let text = refusal_text(
            soften(&mut app, &mut session, CommandPayload::SoftenEdges(angle)).unwrap_err(),
        );
        assert!(text.starts_with("soften.angle_invalid:"), "{angle}: {text}");
    }
    assert_eq!(revision(&app), before);

    let (mut app, mut session) = app_with(vec![body(7)]);
    let text = refusal_text(
        soften(&mut app, &mut session, CommandPayload::SoftenEdges(20.0)).unwrap_err(),
    );
    assert!(text.starts_with("soften.selection_required:"), "{text}");
}

#[test]
fn a_non_body_row_is_refused_by_name() {
    let mut line = body(7);
    line.geometry = SemanticGeometry::Line {
        start: Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        end: Point3 {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        },
    };
    let (mut app, mut session) = app_with(vec![line]);
    select(&mut app, &mut session, &[7]);
    let text = refusal_text(
        soften(&mut app, &mut session, CommandPayload::SoftenEdges(20.0)).unwrap_err(),
    );
    assert!(text.starts_with("soften.not_a_body:"), "{text}");
}

#[test]
fn a_sub_element_ref_is_refused_by_name() {
    let (mut app, mut session) = app_with(vec![body(7)]);
    session.selection.replace([SelectionRef {
        document: DocumentId(1),
        entity: EntityId(7),
        instance: InstancePath::default(),
        sub_element: Some(SubElementId {
            source_key: "face-0".into(),
            topology_revision: Revision(0),
        }),
    }]);
    let text = refusal_text(
        soften(&mut app, &mut session, CommandPayload::SoftenEdges(20.0)).unwrap_err(),
    );
    assert!(text.starts_with("soften.not_addressable:"), "{text}");
}

#[test]
fn viewer_mode_refuses_the_write() {
    let (mut app, mut session) = app_with(vec![body(7)]);
    select(&mut app, &mut session, &[7]);
    session
        .switch_mode(AppMode::Viewer)
        .expect("Viewer is legal");
    assert!(matches!(
        soften(&mut app, &mut session, CommandPayload::SoftenEdges(20.0)).unwrap_err(),
        CadError::PermissionDenied
    ));
    assert_eq!(soften_of(&app, 7), None);
}

#[test]
fn the_soften_angle_survives_a_document_store_round_trip() {
    use cad_domain::DocumentIdentity;
    let (mut app, mut session) = app_with(vec![body(7)]);
    select(&mut app, &mut session, &[7]);
    soften(&mut app, &mut session, CommandPayload::SoftenEdges(12.0)).expect("soften");
    let document = &app.workspace.documents[&DocumentId(1)];
    let snapshot = document
        .drawing
        .snapshot(
            document.units.clone(),
            DocumentIdentity::Temporary(1),
            Vec::new(),
            document.drawing.revision(),
        )
        .expect("snapshot");
    assert!(snapshot.has_soften());
    let loaded = cad_db::DrawingDatabase::from_snapshot(&snapshot).expect("reopen");
    assert_eq!(
        loaded.entity_render_attributes(EntityId(7)).soften,
        Some(12.0)
    );
}
