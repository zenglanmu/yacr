//! P-MESH-1a/1b host commit: the background mesh job publishes once.
//!
//! These contracts drive the real worker (thread + cancellation token) with a
//! fake mesh modeler, then commit through the real `HostController` poll path:
//! one transaction, one undo step, operands gone, the product stored as a mesh
//! (never `Body3d { Solid, Exact }`). The real kernel is exercised by
//! `cad-kernel-manifold` and `cad-cli-tools/tests/mesh_real_samples.rs`.

use super::*;
use crate::host::MeshJobPoll;
use crate::tasks::{MeshJobKind, MeshJobOperand, MeshJobPlan};
use crate::{CommandId, CommandPayload};
use cad_kernel_adapter::{
    codes, inspect_topology_welded, MeshBooleanOp, MeshData, MeshModeler, MeshModelerCapabilities,
    MeshOperation, MeshOutcome, MeshRepair, MeshRepairMode, MeshRequest, MeshResult,
    MeshWindingRule,
};
use std::sync::Arc;

struct FakeMeshModeler {
    refusal: Option<&'static str>,
}

impl MeshModeler for FakeMeshModeler {
    fn registration(&self) -> cad_domain::Registration {
        cad_domain::Registration {
            type_key: "test.fake-mesh".into(),
            version: 1,
            priority: 1,
            entity_types: Vec::new(),
            capabilities: Vec::new(),
        }
    }

    fn capabilities(&self) -> MeshModelerCapabilities {
        MeshModelerCapabilities::all()
    }

    fn execute(
        &self,
        request: &MeshRequest<'_>,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> CadResult<MeshResult> {
        if let Some(code) = self.refusal {
            return Ok(MeshResult {
                stamp: request.stamp.clone(),
                outcome: MeshOutcome::Unsupported {
                    reason: cad_kernel_adapter::MeshUnsupportedReason {
                        code,
                        operation: request.operation.family(),
                        detail: "the fake modeler refuses this input".into(),
                    },
                },
            });
        }
        let data = cube_data();
        let topology = inspect_topology_welded(&data);
        let outcome = match request.operation {
            MeshOperation::Boolean { .. } => MeshOutcome::Success {
                mesh: data,
                topology,
                diagnostics: vec![Diagnostic {
                    object: None,
                    code: "mesh.test_note".into(),
                    message: "the fake kernel folded the meshes".into(),
                }],
            },
            MeshOperation::Repair { mode, rule, .. } => MeshOutcome::Partial {
                mesh: data,
                repair: MeshRepair {
                    mode,
                    rule,
                    input: topology,
                    output: topology,
                    welded_vertices: 0,
                },
                diagnostics: Vec::new(),
            },
        };
        Ok(MeshResult {
            stamp: request.stamp.clone(),
            outcome,
        })
    }
}

fn p(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

fn cube_data() -> MeshData {
    let v = vec![
        p(0.0, 0.0, 0.0),
        p(1.0, 0.0, 0.0),
        p(1.0, 1.0, 0.0),
        p(0.0, 1.0, 0.0),
        p(0.0, 0.0, 1.0),
        p(1.0, 0.0, 1.0),
        p(1.0, 1.0, 1.0),
        p(0.0, 1.0, 1.0),
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

/// A controller whose document holds the given meshes.
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
}

fn plan(controller: &HostController, kind: MeshJobKind, ids: &[u128]) -> MeshJobPlan {
    let drawing = controller.application.workspace.documents[&controller.document_id]
        .drawing
        .as_ref();
    let operands = ids
        .iter()
        .map(|id| {
            let entity = drawing.entity(EntityId(*id)).expect("operand");
            MeshJobOperand::new(entity.clone())
        })
        .collect();
    MeshJobPlan {
        document: controller.document_id,
        kind,
        layer: LayerId(0),
        space: SpaceId::Model,
        draw_order: 0,
        operands,
    }
}

fn wait(controller: &mut HostController) -> MeshJobPoll {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        match controller.poll_mesh_job() {
            MeshJobPoll::Running => {
                assert!(
                    std::time::Instant::now() < deadline,
                    "the mesh job did not settle"
                );
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            poll => return poll,
        }
    }
}

fn entity_count(controller: &HostController) -> usize {
    controller.application.workspace.documents[&controller.document_id]
        .drawing
        .entities()
        .count()
}

fn revision(controller: &HostController) -> Revision {
    controller.application.workspace.documents[&controller.document_id]
        .drawing
        .revision()
}

fn undo_depth(controller: &HostController) -> usize {
    controller
        .application
        .history
        .get(&controller.document_id)
        .map_or(0, |history| history.undo_depth())
}

#[test]
fn a_mesh_boolean_commits_one_mesh_row_and_one_undo_step() {
    let mut controller = controller_with(&[(10, cube_data()), (11, cube_data())]);
    let plan = plan(
        &controller,
        MeshJobKind::Boolean(MeshBooleanOp::Union),
        &[10, 11],
    );
    controller
        .application
        .start_mesh_job(plan, Arc::new(FakeMeshModeler { refusal: None }))
        .expect("the mesh job starts");

    let MeshJobPoll::Done {
        object,
        second_object,
        repair,
        diagnostics,
    } = wait(&mut controller)
    else {
        panic!("the fake mesh boolean must commit");
    };
    assert!(repair.is_none(), "a boolean carries no repair report");
    assert!(second_object.is_none(), "a boolean has one product");
    assert_eq!(diagnostics[0].code, "mesh.test_note");

    assert_eq!(entity_count(&controller), 1, "both operands are replaced");
    let committed = controller.application.workspace.documents[&controller.document_id]
        .drawing
        .entity(EntityId(object.0))
        .expect("the committed row is addressable");
    assert_eq!(committed.object.type_key, "AcDbSubDMesh");
    let SemanticGeometry::Mesh(mesh) = &committed.geometry else {
        panic!("a mesh boolean must store SemanticGeometry::Mesh");
    };
    assert!(!mesh.triangles.is_empty());
    assert!(mesh.face_sources.is_empty(), "no source face identity");
    assert!(mesh.normals.is_empty());
    assert!(mesh.uvs.is_empty());
    assert!(mesh.texture.is_none());
    assert!(mesh.pbr.is_none());
    assert!(revision(&controller) > Revision(0));
    assert_eq!(undo_depth(&controller), 1);

    // One undo restores both operands; the mesh row is gone.
    controller
        .execute(Command {
            schema_version: 1,
            id: CommandId::Undo,
            document: controller.document_id,
            viewport: controller.viewport_id,
            payload: CommandPayload::None,
        })
        .expect("the mesh commit is undoable");
    assert_eq!(entity_count(&controller), 2);
}

#[test]
fn a_mesh_repair_commits_a_closed_mesh_and_reports_partial() {
    let mut controller = controller_with(&[(20, cube_data())]);
    let plan = plan(
        &controller,
        MeshJobKind::Repair {
            mode: MeshRepairMode::Orientation,
            rule: MeshWindingRule::Positive,
        },
        &[20],
    );
    controller
        .application
        .start_mesh_job(plan, Arc::new(FakeMeshModeler { refusal: None }))
        .expect("the repair job starts");

    let MeshJobPoll::Done { repair, .. } = wait(&mut controller) else {
        panic!("the fake repair must commit");
    };
    let repair = repair.expect("a repair carries the Partial report");
    assert_eq!(repair.mode, MeshRepairMode::Orientation);
    assert!(repair.is_closed());
    assert_eq!(entity_count(&controller), 1);
    assert_eq!(undo_depth(&controller), 1);
}

#[test]
fn an_open_mesh_refusal_publishes_nothing() {
    let mut controller = controller_with(&[(30, cube_data()), (31, cube_data())]);
    let plan = plan(
        &controller,
        MeshJobKind::Boolean(MeshBooleanOp::Union),
        &[30, 31],
    );
    controller
        .application
        .start_mesh_job(
            plan,
            Arc::new(FakeMeshModeler {
                refusal: Some(codes::MESH_NOT_CLOSED),
            }),
        )
        .expect("the job starts; the modeler answers");

    let MeshJobPoll::Failed {
        error: CadError::Unsupported(text),
    } = wait(&mut controller)
    else {
        panic!("an open mesh must fail by name");
    };
    assert!(text.contains(codes::MESH_NOT_CLOSED), "{text}");
    assert_eq!(entity_count(&controller), 2);
    assert_eq!(revision(&controller), Revision(0));
    assert_eq!(undo_depth(&controller), 0);
}

#[test]
fn a_cancelled_mesh_job_never_publishes() {
    let mut controller = controller_with(&[(40, cube_data()), (41, cube_data())]);
    let plan = plan(
        &controller,
        MeshJobKind::Boolean(MeshBooleanOp::Subtract),
        &[40, 41],
    );
    controller
        .application
        .start_mesh_job(plan, Arc::new(FakeMeshModeler { refusal: None }))
        .expect("the job starts");
    controller.cancel_mesh_job();

    assert!(matches!(wait(&mut controller), MeshJobPoll::Cancelled));
    assert_eq!(entity_count(&controller), 2);
    assert_eq!(revision(&controller), Revision(0));
    assert_eq!(undo_depth(&controller), 0);
}

#[test]
fn an_edited_operand_discards_the_mesh_result_as_stale() {
    let mut controller = controller_with(&[(50, cube_data()), (51, cube_data())]);
    let plan = plan(
        &controller,
        MeshJobKind::Boolean(MeshBooleanOp::Union),
        &[50, 51],
    );
    controller
        .application
        .start_mesh_job(plan, Arc::new(FakeMeshModeler { refusal: None }))
        .expect("the job starts");

    // Replace the drawing with the same ids but a moved mesh: the full-row
    // snapshot no longer matches, so the result must be discarded.
    let moved = cube_data()
        .vertices
        .iter()
        .map(|point| p(point.x + 1.0, point.y, point.z))
        .collect();
    let moved = MeshData::from_parts(moved, cube_data().triangles);
    {
        let document = controller
            .application
            .workspace
            .documents
            .get_mut(&controller.document_id)
            .expect("document");
        document.drawing = Arc::new(drawing_with(&[(50, moved.clone()), (51, cube_data())]));
    }

    assert!(matches!(wait(&mut controller), MeshJobPoll::Stale));
    assert_eq!(entity_count(&controller), 2);
    assert_eq!(revision(&controller), Revision(0));
    assert_eq!(undo_depth(&controller), 0);
}

#[test]
fn outer_shell_commits_one_mesh_row_and_one_undo_step_through_the_command() {
    let mut controller = controller_with(&[(40, cube_data()), (41, cube_data())]);
    controller
        .application
        .set_mesh_modeler(Some(Arc::new(FakeMeshModeler { refusal: None })));
    let references: Vec<cad_domain::SelectionRef> = [40_u128, 41]
        .into_iter()
        .map(|id| cad_domain::SelectionRef {
            document: controller.document_id,
            entity: EntityId(id),
            instance: cad_domain::InstancePath::default(),
            sub_element: None,
        })
        .collect();
    controller.session.selection.replace(references);
    controller
        .execute(Command {
            schema_version: 1,
            id: CommandId::OuterShell,
            document: controller.document_id,
            viewport: controller.viewport_id,
            payload: CommandPayload::None,
        })
        .expect("OUTERSHELL starts the union job");

    let MeshJobPoll::Done { object, .. } = wait(&mut controller) else {
        panic!("the fake outer shell must commit");
    };
    assert_eq!(
        entity_count(&controller),
        1,
        "the shell replaces both operands"
    );
    let committed = controller.application.workspace.documents[&controller.document_id]
        .drawing
        .entity(EntityId(object.0))
        .expect("the committed row");
    assert_eq!(committed.object.type_key, "AcDbSubDMesh");
    assert_eq!(undo_depth(&controller), 1);
    controller
        .execute(Command {
            schema_version: 1,
            id: CommandId::Undo,
            document: controller.document_id,
            viewport: controller.viewport_id,
            payload: CommandPayload::None,
        })
        .expect("the shell is undoable");
    assert_eq!(entity_count(&controller), 2);
}

#[test]
fn trim_solid_keeps_the_cutter_and_replaces_only_the_target() {
    let mut controller = controller_with(&[(40, cube_data()), (41, cube_data())]);
    controller
        .application
        .set_mesh_modeler(Some(Arc::new(FakeMeshModeler { refusal: None })));
    let cutter_before = controller.application.workspace.documents[&controller.document_id]
        .drawing
        .entity(EntityId(41))
        .expect("the cutter row")
        .clone();
    let references: Vec<cad_domain::SelectionRef> = [40_u128, 41]
        .into_iter()
        .map(|id| cad_domain::SelectionRef {
            document: controller.document_id,
            entity: EntityId(id),
            instance: cad_domain::InstancePath::default(),
            sub_element: None,
        })
        .collect();
    controller.session.selection.replace(references);
    controller
        .execute(Command {
            schema_version: 1,
            id: CommandId::TrimSolid,
            document: controller.document_id,
            viewport: controller.viewport_id,
            payload: CommandPayload::None,
        })
        .expect("TRIMSOLID starts the subtract job");

    let MeshJobPoll::Done { object, .. } = wait(&mut controller) else {
        panic!("the fake trim must commit");
    };
    assert_eq!(
        entity_count(&controller),
        2,
        "the target is replaced and the cutter is kept"
    );
    let document = &controller.application.workspace.documents[&controller.document_id];
    let cutter_after = document
        .drawing
        .entity(EntityId(41))
        .expect("the cutter survives");
    assert_eq!(cutter_after.object.revision, cutter_before.object.revision);
    assert_eq!(cutter_after.geometry, cutter_before.geometry);
    let result = document
        .drawing
        .entity(EntityId(object.0))
        .expect("the trimmed row");
    assert_eq!(result.object.type_key, "AcDbSubDMesh");
    assert_ne!(result.id, EntityId(41));
    assert_eq!(undo_depth(&controller), 1);

    controller
        .execute(Command {
            schema_version: 1,
            id: CommandId::Undo,
            document: controller.document_id,
            viewport: controller.viewport_id,
            payload: CommandPayload::None,
        })
        .expect("the trim commit is undoable");
    assert_eq!(entity_count(&controller), 2, "the original pair is back");
    let document = &controller.application.workspace.documents[&controller.document_id];
    assert!(document.drawing.entity(EntityId(40)).is_some());
    assert!(document.drawing.entity(EntityId(object.0)).is_none());
}

#[test]
fn split_solid_commits_two_pieces_and_keeps_the_cutter() {
    let mut controller = controller_with(&[(60, cube_data()), (61, cube_data())]);
    controller
        .application
        .set_mesh_modeler(Some(Arc::new(FakeMeshModeler { refusal: None })));
    let cutter_before = controller.application.workspace.documents[&controller.document_id]
        .drawing
        .entity(EntityId(61))
        .expect("the cutter row")
        .clone();
    let references: Vec<cad_domain::SelectionRef> = [60_u128, 61]
        .into_iter()
        .map(|id| cad_domain::SelectionRef {
            document: controller.document_id,
            entity: EntityId(id),
            instance: cad_domain::InstancePath::default(),
            sub_element: None,
        })
        .collect();
    controller.session.selection.replace(references);
    controller
        .execute(Command {
            schema_version: 1,
            id: CommandId::SplitSolid,
            document: controller.document_id,
            viewport: controller.viewport_id,
            payload: CommandPayload::None,
        })
        .expect("SPLITSOLID starts the split job");

    let MeshJobPoll::Done {
        object,
        second_object,
        ..
    } = wait(&mut controller)
    else {
        panic!("the fake split must commit");
    };
    let second_object = second_object.expect("a split produces two rows");
    assert_ne!(object, second_object);
    assert_eq!(
        entity_count(&controller),
        3,
        "two split pieces plus the kept cutter"
    );
    let document = &controller.application.workspace.documents[&controller.document_id];
    let cutter_after = document
        .drawing
        .entity(EntityId(61))
        .expect("the cutter survives");
    assert_eq!(cutter_after.object.revision, cutter_before.object.revision);
    assert_eq!(cutter_after.geometry, cutter_before.geometry);
    for id in [object, second_object] {
        let row = document
            .drawing
            .entity(EntityId(id.0))
            .expect("the split piece");
        assert_eq!(row.object.type_key, "AcDbSubDMesh");
    }
    assert!(document.drawing.entity(EntityId(60)).is_none());
    assert_eq!(undo_depth(&controller), 1);

    controller
        .execute(Command {
            schema_version: 1,
            id: CommandId::Undo,
            document: controller.document_id,
            viewport: controller.viewport_id,
            payload: CommandPayload::None,
        })
        .expect("the split commit is undoable");
    assert_eq!(entity_count(&controller), 2, "the original pair is back");
}
