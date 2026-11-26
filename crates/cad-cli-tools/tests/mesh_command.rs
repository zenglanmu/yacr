//! End-to-end P-MESH-1a/1b contract with the **real** `manifold-rust` kernel.
//!
//! The command layer (`cad-app`), the background worker, the host commit and the
//! carrier seam are all real; only the input geometry is synthetic (closed unit
//! cubes). This is not a SketchUp/GPU/performance claim: it proves that a mesh
//! `UNION`/`SUBTRACT`/`INTERSECT` and a mesh repair reach the pinned kernel and
//! publish one `AcDbSubDMesh` row with one undo step.

use std::sync::Arc;

use cad_app::host::{HostController, MeshJobPoll};
use cad_app::{AppMode, Command, CommandId, CommandPayload, ExtendedCommandId, SessionState};
use cad_db::{DbEntity, DbObject, DrawingDatabase, DrawingDatabaseBuilder, Layer};
use cad_domain::{
    Body3d, BodyDisplay, BodyExact, BodyKind, Completeness, DatabaseId, DocumentId, EntityId,
    InstancePath, LayerId, ObjectId, Point3, Precision, Revision, SelectionRef, SemanticGeometry,
    SpaceId,
};
use cad_kernel_adapter::{inspect_topology_welded, MeshData, MeshTopology};
use cad_kernel_manifold::ManifoldMeshModeler;

fn p(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

/// A closed outward-wound 2×2×2 cube as neutral triangle data (unwelded like an
/// STL). The size keeps the three boolean volumes at 12/4/4 when the second cube
/// is offset by 1 on x, matching the kernel-level fixtures.
fn cube_data(origin: [f64; 3]) -> MeshData {
    let [ox, oy, oz] = origin;
    let s = 2.0;
    let v = vec![
        p(ox, oy, oz),
        p(ox + s, oy, oz),
        p(ox + s, oy + s, oz),
        p(ox, oy + s, oz),
        p(ox, oy, oz + s),
        p(ox + s, oy, oz + s),
        p(ox + s, oy + s, oz + s),
        p(ox, oy + s, oz + s),
    ];
    let t = vec![
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
    MeshData::from_parts(v, t)
}

fn entity(id: u128, order: i64, data: &MeshData) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbSubDMesh".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Mesh(cad_domain::Mesh {
            vertices: data.vertices.clone(),
            triangles: data.triangles.clone(),
            ..Default::default()
        }),
        draw_order: order,
    }
}

fn drawing_with(meshes: &[(u128, MeshData)]) -> DrawingDatabase {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    for (index, (id, data)) in meshes.iter().enumerate() {
        builder
            .insert_entity(entity(*id, index as i64, data))
            .unwrap();
    }
    builder.finish().unwrap()
}

fn controller_with(meshes: &[(u128, MeshData)]) -> HostController {
    let mut controller = HostController::with_blank_document([800.0, 600.0]).expect("blank");
    let document = controller
        .application
        .workspace
        .documents
        .get_mut(&controller.document_id)
        .expect("document");
    document.drawing = Arc::new(drawing_with(meshes));
    controller
        .application
        .set_mesh_modeler(Some(Arc::new(ManifoldMeshModeler::new())));
    controller
}

fn select(controller: &mut HostController, ids: &[u128]) {
    let refs = ids
        .iter()
        .map(|id| SelectionRef {
            document: controller.document_id,
            entity: EntityId(*id),
            instance: InstancePath::default(),
            sub_element: None,
        })
        .collect();
    controller
        .execute(Command {
            schema_version: 1,
            id: CommandId::Select,
            document: controller.document_id,
            viewport: controller.viewport_id,
            payload: CommandPayload::Selection(refs),
        })
        .expect("selection");
}

fn execute(controller: &mut HostController, id: CommandId) -> CommandPayloadOutcome {
    let outcome = controller
        .execute(Command {
            schema_version: 1,
            id,
            document: controller.document_id,
            viewport: controller.viewport_id,
            payload: CommandPayload::None,
        })
        .expect("the command starts");
    let codes: Vec<String> = outcome
        .diagnostics
        .into_iter()
        .map(|row| row.code)
        .collect();
    CommandPayloadOutcome { codes }
}

struct CommandPayloadOutcome {
    codes: Vec<String>,
}

fn wait(controller: &mut HostController) -> MeshJobPoll {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        match controller.poll_mesh_job() {
            MeshJobPoll::Running => {
                assert!(
                    std::time::Instant::now() < deadline,
                    "the real mesh job did not settle"
                );
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            poll => return poll,
        }
    }
}

fn signed_volume(mesh: &MeshData) -> f64 {
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

fn committed_mesh(controller: &HostController) -> MeshData {
    let drawing = controller.application.workspace.documents[&controller.document_id]
        .drawing
        .as_ref();
    let entity = drawing.entities().next().expect("one committed mesh");
    let SemanticGeometry::Mesh(mesh) = &entity.geometry else {
        panic!("a mesh boolean must store SemanticGeometry::Mesh");
    };
    MeshData::from_parts(mesh.vertices.clone(), mesh.triangles.clone())
}

#[test]
fn real_kernel_mesh_union_subtract_and_intersect_commit_closed_meshes() {
    for (command, expected_volume) in [
        (ExtendedCommandId::Union, 12.0),
        (ExtendedCommandId::Subtract, 4.0),
        (ExtendedCommandId::Intersect, 4.0),
    ] {
        let mut controller = controller_with(&[
            (10, cube_data([0.0, 0.0, 0.0])),
            (11, cube_data([1.0, 0.0, 0.0])),
        ]);
        select(&mut controller, &[10, 11]);
        let started = execute(&mut controller, CommandId::Extended(command));
        assert!(
            started
                .codes
                .iter()
                .any(|code| code == "mesh.boolean_started"),
            "the command must start a mesh job: {:?}",
            started.codes
        );

        let MeshJobPoll::Done { repair, .. } = wait(&mut controller) else {
            panic!("the real kernel mesh boolean must commit");
        };
        assert!(repair.is_none());
        let data = committed_mesh(&controller);
        let topology = inspect_topology_welded(&data);
        assert_eq!(topology.topology, MeshTopology::ClosedManifold);
        let volume = signed_volume(&data);
        assert!(
            (volume - expected_volume).abs() < 1e-9,
            "expected volume {expected_volume}, got {volume}"
        );
        let drawing = controller.application.workspace.documents[&controller.document_id]
            .drawing
            .as_ref();
        assert_eq!(drawing.entities().count(), 1);
        assert_eq!(drawing.revision(), Revision(1));
    }
}

#[test]
fn real_kernel_repair_returns_a_closed_mesh() {
    let mut cube = cube_data([0.0, 0.0, 0.0]);
    // Flip every triangle: the robust kernel still reads a closed shell, but the
    // winding is inside-out, so Orientation repair has real work to do.
    for triangle in &mut cube.triangles {
        triangle.swap(0, 1);
    }
    let mut controller = controller_with(&[(20, cube)]);
    select(&mut controller, &[20]);
    let started = execute(
        &mut controller,
        CommandId::Extended(ExtendedCommandId::MeshRepair),
    );
    assert!(started
        .codes
        .iter()
        .any(|code| code == "mesh.repair_started"));

    let MeshJobPoll::Done { repair, .. } = wait(&mut controller) else {
        panic!("the real kernel repair must commit");
    };
    let report = repair.expect("a repair carries the Partial report");
    assert!(report.is_closed());
    let data = committed_mesh(&controller);
    assert_eq!(
        inspect_topology_welded(&data).topology,
        MeshTopology::ClosedManifold
    );
    assert!(
        signed_volume(&data) > 0.0,
        "the repaired body must be outward-wound"
    );
}

#[test]
fn real_kernel_refuses_an_open_mesh_without_writing() {
    let mut open = cube_data([0.0, 0.0, 0.0]);
    open.triangles.pop();
    let mut controller = controller_with(&[(30, open), (31, cube_data([1.0, 0.0, 0.0]))]);
    select(&mut controller, &[30, 31]);
    execute(
        &mut controller,
        CommandId::Extended(ExtendedCommandId::Union),
    );

    let MeshJobPoll::Failed { error } = wait(&mut controller) else {
        panic!("an open shell must fail by name");
    };
    let text = error.to_string();
    assert!(
        text.contains("mesh.not_closed"),
        "expected mesh.not_closed, got {text}"
    );
    let drawing = controller.application.workspace.documents[&controller.document_id]
        .drawing
        .as_ref();
    assert_eq!(drawing.entities().count(), 2);
    assert_eq!(drawing.revision(), Revision(0));
}

#[test]
fn real_kernel_rebuild_of_a_non_manifold_closed_soup_commits_partial() {
    // Two face-sharing cubes with both copies of the shared wall: closed but
    // non-manifold. Rebuild must drop the internal wall and publish one mesh.
    let left = cube_data([0.0, 0.0, 0.0]);
    let right = cube_data([2.0, 0.0, 0.0]);
    let mut vertices = left.vertices.clone();
    let offset = vertices.len() as u32;
    vertices.extend(right.vertices.iter().copied());
    let mut triangles = left.triangles.clone();
    triangles.extend(right.triangles.iter().map(|triangle| {
        [
            triangle[0] + offset,
            triangle[1] + offset,
            triangle[2] + offset,
        ]
    }));
    let soup = MeshData::from_parts(vertices, triangles);
    assert_eq!(
        inspect_topology_welded(&soup).topology,
        MeshTopology::NonManifold
    );

    let mut controller = controller_with(&[(40, soup)]);
    select(&mut controller, &[40]);
    execute(
        &mut controller,
        CommandId::Extended(ExtendedCommandId::MeshRebuild),
    );

    let MeshJobPoll::Done { repair, .. } = wait(&mut controller) else {
        panic!("a closed non-manifold soup must rebuild to a Partial product");
    };
    let report = repair.expect("rebuild carries the Partial report");
    assert_eq!(report.input.topology, MeshTopology::NonManifold);
    assert!(report.is_closed());
    let data = committed_mesh(&controller);
    assert_eq!(
        inspect_topology_welded(&data).topology,
        MeshTopology::ClosedManifold
    );
    assert!((signed_volume(&data) - 16.0).abs() < 1e-9);
}

#[test]
fn body3d_unknown_operands_are_mesh_operands() {
    let data = cube_data([0.0, 0.0, 0.0]);
    let mut controller = controller_with(&[]);
    // Replace the document with two imported SKP-style preview bodies: the mesh
    // lives in `BodyDisplay`, the body has no exact geometry and kind Unknown.
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    for (index, x) in [0.0, 1.0].into_iter().enumerate() {
        builder
            .insert_entity(DbEntity {
                object: DbObject {
                    id: ObjectId(50 + index as u128),
                    type_key: "SkpPreviewMesh".into(),
                    revision: Revision(0),
                    source_handle: None,
                },
                id: EntityId(50 + index as u128),
                layer: LayerId(0),
                space: SpaceId::Model,
                geometry: SemanticGeometry::Body3d(Box::new(Body3d {
                    kind: BodyKind::Unknown,
                    exact: BodyExact::Unavailable {
                        reason: "skp.preview_geometry".into(),
                    },
                    display: BodyDisplay {
                        mesh: cad_domain::Mesh {
                            vertices: data
                                .vertices
                                .iter()
                                .map(|point| p(point.x + x, point.y, point.z))
                                .collect(),
                            triangles: data.triangles.clone(),
                            ..Default::default()
                        },
                        edges: Vec::new(),
                        tolerance: None,
                        precision: Precision::Unknown,
                        completeness: Completeness::Partial(vec!["skp.preview_geometry".into()]),
                    },
                })),
                draw_order: index as i64,
            })
            .unwrap();
    }
    let document = controller
        .application
        .workspace
        .documents
        .get_mut(&controller.document_id)
        .expect("document");
    document.drawing = Arc::new(builder.finish().unwrap());

    select(&mut controller, &[50, 51]);
    execute(
        &mut controller,
        CommandId::Extended(ExtendedCommandId::Union),
    );
    let MeshJobPoll::Done { .. } = wait(&mut controller) else {
        panic!("Body3d{{Unknown}} display meshes must boolean");
    };
    let data = committed_mesh(&controller);
    assert_eq!(
        inspect_topology_welded(&data).topology,
        MeshTopology::ClosedManifold
    );
    assert!((signed_volume(&data) - 12.0).abs() < 1e-9);
}

/// Keep the enum import honest: the test file drives the same session shape a
/// carrier uses, so a change to the command surface fails to compile here.
#[allow(dead_code)]
fn session_shape(document: DocumentId) -> SessionState {
    SessionState::new(document, AppMode::Work)
}
