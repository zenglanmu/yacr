//! P-MESH-2/3 command contracts: mesh face move, press/pull, refine, smooth and
//! hole cap.
//!
//! These drive the real `cad-mesh-edit` layer through `Application::execute`; the
//! kernel is not involved. The drawing is a 2×2×2 cube whose two top triangles
//! carry the source face key `top`.

use super::*;

fn p3(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

fn cube_mesh(with_top: bool) -> cad_domain::Mesh {
    let vertices = vec![
        p3(0.0, 0.0, 0.0),
        p3(2.0, 0.0, 0.0),
        p3(2.0, 2.0, 0.0),
        p3(0.0, 2.0, 0.0),
        p3(0.0, 0.0, 2.0),
        p3(2.0, 0.0, 2.0),
        p3(2.0, 2.0, 2.0),
        p3(0.0, 2.0, 2.0),
    ];
    let mut triangles = vec![
        [0, 2, 1],
        [0, 3, 2],
        [4, 5, 6],
        [4, 6, 7],
        [0, 1, 5],
        [0, 5, 4],
        [1, 2, 6],
        [1, 6, 5],
        [2, 3, 7],
        [2, 7, 6],
        [3, 0, 4],
        [3, 4, 7],
    ];
    let top = SubElementId {
        source_key: "top".into(),
        topology_revision: Revision(0),
    };
    let mut face_sources = vec![None; triangles.len()];
    face_sources[2] = Some(top.clone());
    face_sources[3] = Some(top);
    if !with_top {
        triangles.remove(3);
        triangles.remove(2);
        face_sources.remove(3);
        face_sources.remove(2);
    }
    cad_domain::Mesh {
        vertices,
        triangles,
        normals: Vec::new(),
        face_sources,
        colors: Vec::new(),
        uvs: Vec::new(),
        texture: None,
        pbr: None,
    }
}

fn app_with_mesh(mesh: cad_domain::Mesh) -> (Application, SessionState, EntityId) {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    let id = EntityId(70);
    builder
        .insert_entity(DbEntity {
            object: DbObject {
                id: ObjectId(70),
                type_key: "AcDbSubDMesh".into(),
                revision: Revision(0),
                source_handle: None,
            },
            id,
            layer: LayerId(0),
            space: SpaceId::Model,
            geometry: SemanticGeometry::Mesh(mesh),
            draw_order: 0,
        })
        .unwrap();
    let drawing = builder.finish().unwrap();
    let (mut app, session) = application_with_document();
    app.workspace.documents.insert(
        DocumentId(1),
        document_with(drawing, DocumentIdentity::Sha256([0u8; 32])),
    );
    (app, session, id)
}

fn face_ref(id: EntityId) -> SelectionRef {
    SelectionRef {
        document: DocumentId(1),
        entity: id,
        instance: InstancePath::default(),
        sub_element: Some(SubElementId {
            source_key: "top".into(),
            topology_revision: Revision(0),
        }),
    }
}

fn whole_ref(id: EntityId) -> SelectionRef {
    SelectionRef {
        document: DocumentId(1),
        entity: id,
        instance: InstancePath::default(),
        sub_element: None,
    }
}

fn select(session: &mut SessionState, refs: Vec<SelectionRef>) {
    session.selection = SelectionSet::from_refs(refs);
}

fn stored_mesh(app: &Application) -> cad_domain::Mesh {
    let drawing = app.workspace.documents[&DocumentId(1)].drawing.as_ref();
    let entity = drawing.entities().next().expect("one mesh");
    let SemanticGeometry::Mesh(mesh) = &entity.geometry else {
        panic!("a mesh edit must store SemanticGeometry::Mesh");
    };
    mesh.clone()
}

fn signed_volume(mesh: &cad_domain::Mesh) -> f64 {
    let mut six = 0.0;
    for triangle in &mesh.triangles {
        let a = mesh.vertices[triangle[0] as usize];
        let b = mesh.vertices[triangle[1] as usize];
        let c = mesh.vertices[triangle[2] as usize];
        six += a.x * (b.y * c.z - b.z * c.y) - a.y * (b.x * c.z - b.z * c.x)
            + a.z * (b.x * c.y - b.y * c.x);
    }
    six / 6.0
}

fn refusal(error: CadError) -> String {
    match error {
        CadError::InvalidInput(text) | CadError::Unsupported(text) => text,
        other => panic!("expected a named refusal, got {other:?}"),
    }
}

#[test]
fn move_on_a_mesh_face_moves_the_region_and_stays_closed() {
    let (mut app, mut session, id) = app_with_mesh(cube_mesh(true));
    select(&mut session, vec![face_ref(id)]);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::MoveEntities,
        CommandPayload::Move {
            refs: vec![face_ref(id)],
            delta: p3(0.0, 0.0, 1.0),
        },
    )
    .expect("MOVE on a mesh face");
    assert!(outcome
        .diagnostics
        .iter()
        .any(|row| row.code == "mesh.move_faces"));
    let mesh = stored_mesh(&app);
    assert!((signed_volume(&mesh) - 12.0).abs() < 1e-9);
    assert!(mesh
        .face_sources
        .iter()
        .any(|source| source.as_ref().is_some_and(|id| id.source_key == "top")));
    assert!(app.can_undo(&DocumentId(1)));
}

#[test]
fn move_on_a_mesh_edge_translates_both_endpoints() {
    let (mut app, mut session, id) = app_with_mesh(cube_mesh(true));
    let key = cad_domain::mesh_edge_key(p3(0.0, 0.0, 2.0), p3(2.0, 0.0, 2.0));
    let edge_ref = SelectionRef {
        document: DocumentId(1),
        entity: id,
        instance: InstancePath::default(),
        sub_element: Some(SubElementId {
            source_key: key.clone(),
            topology_revision: Revision(0),
        }),
    };
    select(&mut session, vec![edge_ref.clone()]);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::MoveEntities,
        CommandPayload::Move {
            refs: vec![edge_ref],
            delta: p3(0.0, 0.0, 1.0),
        },
    )
    .expect("MOVE on a mesh edge");
    assert!(outcome
        .diagnostics
        .iter()
        .any(|row| row.code == "mesh.move_edges"));
    let mesh = stored_mesh(&app);
    assert_eq!(
        cad_mesh_edit::classify(&cad_mesh_edit::weld(&mesh).unwrap()).class,
        cad_mesh_edit::MeshClass::ClosedManifold
    );
    assert!(signed_volume(&mesh) > 8.0);
    // The moved endpoints produce a new geometry-derived key.
    assert!(cad_mesh_edit::edge_for_key(&cad_mesh_edit::weld(&mesh).unwrap(), &key).is_err());
}

#[test]
fn move_on_a_mesh_without_source_ids_refuses_by_name() {
    let mut mesh = cube_mesh(true);
    mesh.face_sources.clear();
    let (mut app, mut session, id) = app_with_mesh(mesh);
    select(&mut session, vec![face_ref(id)]);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::MoveEntities,
        CommandPayload::Move {
            refs: vec![face_ref(id)],
            delta: p3(0.0, 0.0, 1.0),
        },
    )
    .expect_err("no face identity");
    assert!(refusal(error).contains("mesh.face_identity_unavailable"));
    assert_eq!(revision(&app), Revision(0));
}

#[test]
fn presspull_on_a_mesh_face_uses_the_region_normal() {
    let (mut app, mut session, id) = app_with_mesh(cube_mesh(true));
    session
        .set_tool_parameter(ToolParameter::PressPullDistance, ToolValue::Number(1.0))
        .expect("distance");
    let error = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::PressPull),
        CommandPayload::PressPullFace {
            body: face_ref(id),
            seed: p3(1.0, 1.0, 2.0),
        },
    )
    .expect("PRESSPULL on a mesh face");
    assert!(error
        .diagnostics
        .iter()
        .any(|row| row.code == "mesh.presspull"));
    assert!((signed_volume(&stored_mesh(&app)) - 12.0).abs() < 1e-9);
}

#[test]
fn meshrefine_subdivides_the_selected_mesh_in_one_undo() {
    let (mut app, mut session, id) = app_with_mesh(cube_mesh(true));
    select(&mut session, vec![whole_ref(id)]);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::MeshRefine),
        CommandPayload::None,
    )
    .expect("MESHREFINE");
    assert!(outcome
        .diagnostics
        .iter()
        .any(|row| row.code == "mesh.refine"));
    assert_eq!(stored_mesh(&app).triangles.len(), 48);
    assert_eq!(
        app.history
            .get(&DocumentId(1))
            .map_or(0, |history| history.undo_depth()),
        1
    );
}

#[test]
fn meshsmooth_keeps_a_closed_mesh_closed() {
    let (mut app, mut session, id) = app_with_mesh(cube_mesh(true));
    select(&mut session, vec![whole_ref(id)]);
    execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::MeshSmooth),
        CommandPayload::None,
    )
    .expect("MESHSMOOTH");
    let mesh = stored_mesh(&app);
    assert!(signed_volume(&mesh) > 0.0);
    assert_eq!(
        cad_mesh_edit::classify(&cad_mesh_edit::weld(&mesh).unwrap()).class,
        cad_mesh_edit::MeshClass::ClosedManifold
    );
}

#[test]
fn meshcap_fills_the_removed_top_hole_and_reports_it() {
    let (mut app, mut session, id) = app_with_mesh(cube_mesh(false));
    select(&mut session, vec![whole_ref(id)]);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::MeshCap),
        CommandPayload::None,
    )
    .expect("MESHCAP");
    assert!(outcome
        .diagnostics
        .iter()
        .any(|row| row.code == "mesh.hole_filled"));
    assert!(outcome
        .diagnostics
        .iter()
        .any(|row| row.code == "mesh.cap_closed"));
    let mesh = stored_mesh(&app);
    assert_eq!(
        cad_mesh_edit::classify(&cad_mesh_edit::weld(&mesh).unwrap()).class,
        cad_mesh_edit::MeshClass::ClosedManifold
    );
    assert!((signed_volume(&mesh) - 8.0).abs() < 1e-9);
}

#[test]
fn meshcap_on_a_closed_mesh_refuses_without_writing() {
    let (mut app, mut session, id) = app_with_mesh(cube_mesh(true));
    select(&mut session, vec![whole_ref(id)]);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::MeshCap),
        CommandPayload::None,
    )
    .expect_err("no holes");
    assert!(refusal(error).contains("mesh.no_holes"));
    assert_eq!(revision(&app), Revision(0));
    assert!(!app.can_undo(&DocumentId(1)));
}
