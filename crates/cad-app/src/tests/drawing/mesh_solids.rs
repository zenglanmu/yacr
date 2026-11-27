//! P-MESH-1a/1b command contracts: operand classification and job start.
//!
//! A fake [`MeshModeler`] stands in for the kernel (`cad-app` links no mesh
//! kernel), exactly like `tests::drawing::solids_bool` does for exact booleans.
//! The worker is real (thread + cancellation token); the commit path is covered
//! by `host::tests::mesh_jobs`.
//!
//! Every refusal here happens **before** the job starts, so the drawing, its
//! revision, the id allocator and history stay untouched.

use super::*;
use crate::tasks::MeshJobTake;
use cad_kernel_adapter::{
    codes, inspect_topology_welded, MeshBooleanOp, MeshData, MeshModeler, MeshModelerCapabilities,
    MeshOperation, MeshOutcome, MeshRepair, MeshRequest, MeshResult, MeshTopology,
};
use std::sync::{Arc, Mutex};

struct FakeMeshModeler {
    ops: Arc<Mutex<Vec<String>>>,
    /// Result to answer with; `None` means a successful closed cube.
    refusal: Option<&'static str>,
}

impl MeshModeler for FakeMeshModeler {
    fn registration(&self) -> cad_domain::Registration {
        cad_domain::Registration {
            type_key: "test.fake-mesh".into(),
            version: 1,
            priority: 1,
            entity_types: vec!["AcDbSubDMesh".into()],
            capabilities: vec!["booleans".into(), "repair".into()],
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
        let label = match request.operation {
            MeshOperation::Boolean { op, .. } => format!("boolean:{}", op.as_str()),
            MeshOperation::Repair { mode, .. } => format!("repair:{}", mode.as_str()),
        };
        self.ops.lock().expect("op log").push(label);
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
        let data = cube_mesh();
        let output = inspect_topology_welded(&data);
        let outcome = match request.operation {
            MeshOperation::Boolean { .. } => MeshOutcome::Success {
                mesh: data,
                topology: output,
                diagnostics: Vec::new(),
            },
            MeshOperation::Repair { mode, rule, .. } => MeshOutcome::Partial {
                mesh: data,
                repair: MeshRepair {
                    mode,
                    rule,
                    input: output,
                    output,
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

/// A closed outward-wound unit cube as neutral triangle data.
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

fn cube_mesh() -> MeshData {
    cube_data()
}

fn domain_mesh() -> cad_domain::Mesh {
    let data = cube_data();
    cad_domain::Mesh {
        vertices: data.vertices,
        triangles: data.triangles,
        ..Default::default()
    }
}

fn mesh_entity(id: u128, order: i64) -> DbEntity {
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
        geometry: SemanticGeometry::Mesh(domain_mesh()),
        draw_order: order,
    }
}

fn exact_entity(id: u128, order: i64) -> DbEntity {
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
        geometry: SemanticGeometry::Body3d(Box::new(cad_domain::Body3d {
            kind: BodyKind::Solid,
            exact: BodyExact::Exact(Box::new(super::solids::unit_box())),
            display: cad_domain::BodyDisplay {
                mesh: domain_mesh(),
                edges: Vec::new(),
                tolerance: None,
                precision: Precision::Analytic,
                completeness: Completeness::Complete,
            },
        })),
        draw_order: order,
    }
}

fn app_with_entities(entities: Vec<DbEntity>) -> (Application, SessionState, Vec<EntityId>) {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    let ids = entities.iter().map(|entity| entity.id).collect();
    for entity in entities {
        b.insert_entity(entity).unwrap();
    }
    let drawing = b.finish().unwrap();
    let (mut app, session) = application_with_document();
    app.workspace.documents.insert(
        DocumentId(1),
        document_with(drawing, DocumentIdentity::Sha256([0u8; 32])),
    );
    (app, session, ids)
}

fn app_with_meshes(count: usize) -> (Application, SessionState, Vec<EntityId>) {
    let entities = (0..count)
        .map(|index| mesh_entity(index as u128 + 10, index as i64))
        .collect();
    app_with_entities(entities)
}

fn attach_modeler(app: &mut Application, refusal: Option<&'static str>) -> Arc<Mutex<Vec<String>>> {
    let ops = Arc::new(Mutex::new(Vec::new()));
    app.set_mesh_modeler(Some(Arc::new(FakeMeshModeler {
        ops: ops.clone(),
        refusal,
    })));
    ops
}

fn select(session: &mut SessionState, ids: &[EntityId]) {
    session.selection = SelectionSet::from_refs(ids.iter().map(|id| SelectionRef {
        document: DocumentId(1),
        entity: *id,
        instance: InstancePath::default(),
        sub_element: None,
    }));
}

fn invalid_text(error: CadError) -> String {
    match error {
        CadError::InvalidInput(text) => text,
        other => panic!("expected InvalidInput, got {other:?}"),
    }
}

fn unsupported_text(error: CadError) -> String {
    match error {
        CadError::Unsupported(text) => text,
        other => panic!("expected Unsupported, got {other:?}"),
    }
}

fn entity_count(app: &Application) -> usize {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .entities()
        .count()
}

#[test]
fn union_of_two_meshes_starts_a_mesh_boolean_job() {
    let (mut app, mut session, ids) = app_with_meshes(2);
    let ops = attach_modeler(&mut app, None);
    select(&mut session, &ids);

    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::Union),
        CommandPayload::None,
    )
    .expect("a mesh UNION must start a job");

    assert!(
        outcome
            .diagnostics
            .iter()
            .any(|row| row.code == "mesh.boolean_started"),
        "the outcome must name the mesh job: {:?}",
        outcome.diagnostics
    );
    assert_eq!(entity_count(&app), 2, "the command writes nothing itself");
    assert_eq!(revision(&app), Revision(0));
    assert_eq!(
        app.history
            .get(&DocumentId(1))
            .map_or(0, |history| history.undo_depth()),
        0
    );

    // Poll the real worker: the fake modeler answers a closed cube.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        match app.poll_mesh_job_take() {
            MeshJobTake::Running => {
                assert!(
                    std::time::Instant::now() < deadline,
                    "the mesh job did not settle"
                );
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            MeshJobTake::Done { topology, .. } => {
                assert_eq!(topology.topology, MeshTopology::ClosedManifold);
                break;
            }
            other => panic!("the fake mesh job must succeed, got {other:?}",),
        }
    }
    assert_eq!(
        ops.lock().expect("op log").as_slice(),
        &["boolean:union".to_string()]
    );
}

#[test]
fn mixed_exact_and_mesh_operands_refuse_by_class() {
    let (mut app, mut session, _) =
        app_with_entities(vec![exact_entity(20, 0), mesh_entity(21, 1)]);
    attach_modeler(&mut app, None);
    select(&mut session, &[EntityId(20), EntityId(21)]);

    let error = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::Subtract),
        CommandPayload::None,
    )
    .expect_err("mixed operand classes must refuse");
    let text = invalid_text(error);
    assert!(
        text.contains("boolean.operand_class_mismatch"),
        "unexpected refusal: {text}"
    );
    assert_eq!(entity_count(&app), 2);
    assert_eq!(revision(&app), Revision(0));
}

#[test]
fn a_carrier_without_a_mesh_modeler_refuses_by_name() {
    let (mut app, mut session, ids) = app_with_meshes(2);
    select(&mut session, &ids);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::Union),
        CommandPayload::None,
    )
    .expect_err("no mesh modeler means a named refusal");
    assert_eq!(unsupported_text(error), "unsupported-runtime: mesh-modeler");
    assert_eq!(revision(&app), Revision(0));
}

#[test]
fn an_open_mesh_refusal_is_reported_by_the_worker() {
    let (mut app, mut session, ids) = app_with_meshes(2);
    attach_modeler(&mut app, Some(codes::MESH_NOT_CLOSED));
    select(&mut session, &ids);
    execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::Union),
        CommandPayload::None,
    )
    .expect("the command starts the job; the kernel decides closure");

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        match app.poll_mesh_job_take() {
            MeshJobTake::Running => {
                assert!(std::time::Instant::now() < deadline);
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            MeshJobTake::Failed(CadError::Unsupported(text)) => {
                assert!(
                    text.contains(codes::MESH_NOT_CLOSED),
                    "the stable code must survive: {text}"
                );
                break;
            }
            other => panic!("expected a named failure, got {other:?}"),
        }
    }
    assert_eq!(entity_count(&app), 2, "a refusal writes nothing");
    assert_eq!(revision(&app), Revision(0));
}

#[test]
fn meshrepair_needs_exactly_one_mesh() {
    let (mut app, mut session, ids) = app_with_meshes(2);
    attach_modeler(&mut app, None);
    select(&mut session, &ids);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::MeshRepair),
        CommandPayload::None,
    )
    .expect_err("two meshes are not one repair operand");
    assert!(invalid_text(error).contains("meshrepair.needs_one"));

    select(&mut session, &ids[..1]);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::MeshRepair),
        CommandPayload::None,
    )
    .expect("one mesh starts the repair job");
    assert!(outcome
        .diagnostics
        .iter()
        .any(|row| row.code == "mesh.repair_started"));
}

#[test]
fn meshrepair_refuses_a_non_mesh_row() {
    let (mut app, mut session, _) = app_with_entities(vec![exact_entity(30, 0)]);
    attach_modeler(&mut app, None);
    select(&mut session, &[EntityId(30)]);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::MeshRebuild),
        CommandPayload::None,
    )
    .expect_err("an exact body is not a mesh");
    assert!(invalid_text(error).contains("meshrepair.not_mesh"));
    assert_eq!(revision(&app), Revision(0));
}

#[test]
fn mesh_jobs_and_exact_booleans_mutually_exclude() {
    let (mut app, _session, _ids) = app_with_meshes(0);
    let solid_modeler: Arc<dyn cad_kernel_adapter::SolidModeler> = Arc::new(StubSolidModeler);
    app.start_solid_boolean(
        crate::tasks::SolidBooleanPlan {
            document: DocumentId(1),
            op: cad_kernel_adapter::BooleanOp::Union,
            layer: LayerId(0),
            space: SpaceId::Model,
            draw_order: 0,
            operands: Vec::new(),
        },
        solid_modeler,
    )
    .expect("the exact job slot starts");

    let mesh_modeler: Arc<dyn MeshModeler> = Arc::new(FakeMeshModeler {
        ops: Arc::new(Mutex::new(Vec::new())),
        refusal: None,
    });
    let error = app
        .start_mesh_job(
            crate::tasks::MeshJobPlan {
                document: DocumentId(1),
                kind: crate::tasks::MeshJobKind::Boolean(MeshBooleanOp::Union),
                layer: LayerId(0),
                space: SpaceId::Model,
                draw_order: 0,
                operands: Vec::new(),
            },
            mesh_modeler,
        )
        .expect_err("a mesh job cannot start while an exact boolean runs");
    let CadError::Unsupported(text) = error else {
        panic!("expected a named refusal, got {error:?}");
    };
    assert!(
        text.contains("mesh.kernel_busy"),
        "unexpected refusal: {text}"
    );
}

/// A modeler that never runs a real boolean; only the job-slot guard is probed.
struct StubSolidModeler;

impl cad_kernel_adapter::SolidModeler for StubSolidModeler {
    fn registration(&self) -> cad_domain::Registration {
        cad_domain::Registration {
            type_key: "test.stub-solid".into(),
            version: 1,
            priority: 1,
            entity_types: Vec::new(),
            capabilities: Vec::new(),
        }
    }

    fn capabilities(&self) -> cad_kernel_adapter::ModelerCapabilities {
        cad_kernel_adapter::ModelerCapabilities {
            primitives: cad_kernel_adapter::ModelerPrimitives::default(),
            booleans: true,
            mass_properties: false,
            chamfer: false,
            offset_face: false,
            extrude_profile: false,
            fillet: false,
            shell: false,
            nurbs: false,
        }
    }

    fn execute(
        &self,
        request: &cad_kernel_adapter::SolidRequest<'_>,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> CadResult<cad_kernel_adapter::SolidResult> {
        Ok(cad_kernel_adapter::SolidResult {
            stamp: request.stamp.clone(),
            outcome: cad_kernel_adapter::SolidOutcome::Failed {
                diagnostics: Vec::new(),
            },
        })
    }
}

fn open_mesh_entity(id: u128, order: i64) -> DbEntity {
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
            vertices: vec![
                cad_domain::Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                cad_domain::Point3 {
                    x: 1.0,
                    y: 0.0,
                    z: 0.0,
                },
                cad_domain::Point3 {
                    x: 0.0,
                    y: 1.0,
                    z: 0.0,
                },
            ],
            triangles: vec![[0, 1, 2]],
            ..cad_domain::Mesh::default()
        }),
        draw_order: order,
    }
}

#[test]
fn outer_shell_starts_one_union_job_over_two_closed_meshes() {
    let (mut app, mut session, ids) = app_with_meshes(2);
    let ops = attach_modeler(&mut app, None);
    select(&mut session, &ids);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::OuterShell,
        CommandPayload::None,
    )
    .expect("a valid outer shell must start a union job");
    let diagnostic = outcome
        .diagnostics
        .iter()
        .find(|row| row.code == "mesh.boolean_started")
        .expect("the outcome names the mesh job");
    assert!(diagnostic.message.contains("OUTERSHELL"), "{diagnostic:?}");
    assert!(
        ops.lock().expect("op log").is_empty(),
        "the worker runs later"
    );
    assert_eq!(entity_count(&app), 2, "the command writes nothing itself");

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        match app.poll_mesh_job_take() {
            MeshJobTake::Running => {
                assert!(
                    std::time::Instant::now() < deadline,
                    "the mesh job did not settle"
                );
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            MeshJobTake::Done { topology, .. } => {
                assert_eq!(topology.topology, MeshTopology::ClosedManifold);
                break;
            }
            other => panic!("the fake mesh job must succeed, got {other:?}"),
        }
    }
    assert_eq!(
        ops.lock().expect("op log").as_slice(),
        &["boolean:union".to_string()],
        "outer shell is a mesh union at the kernel"
    );
}

#[test]
fn outer_shell_needs_two_bodies_and_writes_nothing_without_one() {
    let (mut app, mut session, ids) = app_with_meshes(2);
    attach_modeler(&mut app, None);
    select(&mut session, &ids[..1]);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::OuterShell,
        CommandPayload::None,
    )
    .expect_err("one body is not a shell");
    assert!(invalid_text(error).contains("outershell.needs_two"));
    assert_eq!(entity_count(&app), 2);
    assert_eq!(revision(&app), Revision(0));
}

#[test]
fn outer_shell_refuses_an_open_shell_before_the_kernel() {
    let (mut app, mut session, ids) =
        app_with_entities(vec![mesh_entity(10, 0), open_mesh_entity(11, 1)]);
    let ops = attach_modeler(&mut app, None);
    select(&mut session, &ids);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::OuterShell,
        CommandPayload::None,
    )
    .expect_err("an open shell cannot be unioned");
    let text = invalid_text(error);
    assert!(text.contains("outershell.open_shell"), "{text}");
    assert!(text.contains("boundary edge"), "{text}");
    assert!(ops.lock().expect("op log").is_empty(), "no worker started");
    assert_eq!(entity_count(&app), 2);
    assert_eq!(revision(&app), Revision(0));
}

#[test]
fn outer_shell_refuses_an_exact_solid_operand_by_name() {
    let (mut app, mut session, ids) =
        app_with_entities(vec![mesh_entity(10, 0), exact_entity(11, 1)]);
    attach_modeler(&mut app, None);
    select(&mut session, &ids);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::OuterShell,
        CommandPayload::None,
    )
    .expect_err("outer shell is mesh-only");
    let text = invalid_text(error);
    assert!(text.contains("outershell.not_mesh"), "{text}");
    assert!(text.contains("exact solid"), "{text}");
    assert_eq!(entity_count(&app), 2);
}

#[test]
fn outer_shell_refuses_without_a_mesh_modeler_and_over_budget() {
    // No modeler installed: a valid two-mesh selection reports the carrier's
    // stable refusal.
    let (mut app, mut session, ids) = app_with_meshes(2);
    select(&mut session, &ids);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::OuterShell,
        CommandPayload::None,
    )
    .expect_err("no mesh modeler");
    assert!(unsupported_text(error).contains("unsupported-runtime: mesh-modeler"));

    // Budget: one operand over the input cap is refused before any topology walk.
    let mut huge = domain_mesh();
    huge.triangles =
        vec![[0, 1, 2]; crate::app_drawing::mesh_solids::OUTERSHELL_MAX_INPUT_TRIANGLES + 1];
    let huge_entity = DbEntity {
        geometry: SemanticGeometry::Mesh(huge),
        ..mesh_entity(12, 0)
    };
    let (mut app, mut session, ids) = app_with_entities(vec![mesh_entity(10, 0), huge_entity]);
    attach_modeler(&mut app, None);
    select(&mut session, &ids);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::OuterShell,
        CommandPayload::None,
    )
    .expect_err("over the shell budget");
    assert!(invalid_text(error).contains("outershell.budget"));
}

#[test]
fn trim_solid_starts_one_subtract_job_and_keeps_the_cutters() {
    let (mut app, mut session, ids) = app_with_meshes(3);
    let ops = attach_modeler(&mut app, None);
    select(&mut session, &ids);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::TrimSolid,
        CommandPayload::None,
    )
    .expect("a valid trim must start a job");
    let diagnostic = outcome
        .diagnostics
        .iter()
        .find(|row| row.code == "mesh.trim_started")
        .expect("the outcome names the trim job");
    assert!(
        diagnostic.message.contains("cutters are kept"),
        "{diagnostic:?}"
    );
    assert!(
        ops.lock().expect("op log").is_empty(),
        "the worker runs later"
    );
    assert_eq!(entity_count(&app), 3, "the command writes nothing itself");

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        match app.poll_mesh_job_take() {
            MeshJobTake::Running => {
                assert!(
                    std::time::Instant::now() < deadline,
                    "the trim job did not settle"
                );
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            MeshJobTake::Done { plan, topology, .. } => {
                assert_eq!(topology.topology, MeshTopology::ClosedManifold);
                assert_eq!(plan.kind, crate::tasks::MeshJobKind::Trim);
                break;
            }
            other => panic!("the fake trim job must succeed, got {other:?}"),
        }
    }
    assert_eq!(
        ops.lock().expect("op log").as_slice(),
        &[
            "boolean:subtract".to_string(),
            "boolean:subtract".to_string()
        ],
        "trim folds subtract over the two cutters, left to right"
    );
}

#[test]
fn trim_solid_refuses_invalid_operands_before_the_kernel() {
    // Fewer than two bodies.
    let (mut app, mut session, ids) = app_with_meshes(2);
    attach_modeler(&mut app, None);
    select(&mut session, &ids[..1]);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::TrimSolid,
        CommandPayload::None,
    )
    .expect_err("one body cannot be trimmed");
    assert!(invalid_text(error).contains("trimsolid.needs_two"));

    // An open shell.
    let (mut app, mut session, ids) =
        app_with_entities(vec![mesh_entity(10, 0), open_mesh_entity(11, 1)]);
    let ops = attach_modeler(&mut app, None);
    select(&mut session, &ids);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::TrimSolid,
        CommandPayload::None,
    )
    .expect_err("an open shell cannot be trimmed");
    assert!(invalid_text(error).contains("trimsolid.open_shell"));
    assert!(ops.lock().expect("op log").is_empty(), "no worker started");

    // An exact solid.
    let (mut app, mut session, ids) =
        app_with_entities(vec![mesh_entity(10, 0), exact_entity(11, 1)]);
    attach_modeler(&mut app, None);
    select(&mut session, &ids);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::TrimSolid,
        CommandPayload::None,
    )
    .expect_err("trim is mesh-only");
    assert!(invalid_text(error).contains("trimsolid.not_mesh"));

    // Viewer.
    let (mut app, mut session, ids) = app_with_meshes(2);
    attach_modeler(&mut app, None);
    select(&mut session, &ids);
    session.switch_mode(AppMode::Viewer).expect("Viewer");
    let error = execute(
        &mut app,
        &mut session,
        CommandId::TrimSolid,
        CommandPayload::None,
    )
    .expect_err("a document write is work-only");
    assert!(matches!(error, CadError::PermissionDenied), "{error:?}");
}

#[test]
fn trim_solid_refuses_without_a_modeler_and_over_budget() {
    let (mut app, mut session, ids) = app_with_meshes(2);
    select(&mut session, &ids);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::TrimSolid,
        CommandPayload::None,
    )
    .expect_err("no mesh modeler");
    assert!(unsupported_text(error).contains("unsupported-runtime: mesh-modeler"));

    let mut huge = domain_mesh();
    huge.triangles =
        vec![[0, 1, 2]; crate::app_drawing::mesh_solids::TRIMSOLID_MAX_INPUT_TRIANGLES + 1];
    let huge_entity = DbEntity {
        geometry: SemanticGeometry::Mesh(huge),
        ..mesh_entity(12, 0)
    };
    let (mut app, mut session, ids) = app_with_entities(vec![mesh_entity(10, 0), huge_entity]);
    attach_modeler(&mut app, None);
    select(&mut session, &ids);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::TrimSolid,
        CommandPayload::None,
    )
    .expect_err("over the trim budget");
    assert!(invalid_text(error).contains("trimsolid.budget"));
}

#[test]
fn split_solid_starts_one_job_and_produces_two_products() {
    let (mut app, mut session, ids) = app_with_meshes(2);
    let ops = attach_modeler(&mut app, None);
    select(&mut session, &ids);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::SplitSolid,
        CommandPayload::None,
    )
    .expect("a valid split must start a job");
    let diagnostic = outcome
        .diagnostics
        .iter()
        .find(|row| row.code == "mesh.split_started")
        .expect("the outcome names the split job");
    assert!(
        diagnostic.message.contains("outside and inside"),
        "{diagnostic:?}"
    );
    assert!(
        ops.lock().expect("op log").is_empty(),
        "the worker runs later"
    );

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        match app.poll_mesh_job_take() {
            MeshJobTake::Running => {
                assert!(
                    std::time::Instant::now() < deadline,
                    "the split job did not settle"
                );
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            MeshJobTake::Done {
                plan,
                second,
                topology,
                ..
            } => {
                assert_eq!(topology.topology, MeshTopology::ClosedManifold);
                assert_eq!(plan.kind, crate::tasks::MeshJobKind::Split);
                assert!(second.is_some(), "a split produces the inside piece too");
                break;
            }
            other => panic!("the fake split job must succeed, got {other:?}"),
        }
    }
    assert_eq!(
        ops.lock().expect("op log").as_slice(),
        &[
            "boolean:subtract".to_string(),
            "boolean:intersect".to_string()
        ],
        "one cutter needs no union: subtract for outside, intersect for inside"
    );
}

#[test]
fn split_solid_refuses_invalid_operands_before_the_kernel() {
    let (mut app, mut session, ids) = app_with_meshes(2);
    attach_modeler(&mut app, None);
    select(&mut session, &ids[..1]);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::SplitSolid,
        CommandPayload::None,
    )
    .expect_err("one body cannot be split");
    assert!(invalid_text(error).contains("splitsolid.needs_two"));

    let (mut app, mut session, ids) =
        app_with_entities(vec![mesh_entity(10, 0), open_mesh_entity(11, 1)]);
    attach_modeler(&mut app, None);
    select(&mut session, &ids);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::SplitSolid,
        CommandPayload::None,
    )
    .expect_err("an open shell cannot be split");
    assert!(invalid_text(error).contains("splitsolid.open_shell"));

    let (mut app, mut session, ids) =
        app_with_entities(vec![mesh_entity(10, 0), exact_entity(11, 1)]);
    attach_modeler(&mut app, None);
    select(&mut session, &ids);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::SplitSolid,
        CommandPayload::None,
    )
    .expect_err("split is mesh-only");
    assert!(invalid_text(error).contains("splitsolid.not_mesh"));

    let (mut app, mut session, ids) = app_with_meshes(2);
    attach_modeler(&mut app, None);
    select(&mut session, &ids);
    session.switch_mode(AppMode::Viewer).expect("Viewer");
    let error = execute(
        &mut app,
        &mut session,
        CommandId::SplitSolid,
        CommandPayload::None,
    )
    .expect_err("a document write is work-only");
    assert!(matches!(error, CadError::PermissionDenied), "{error:?}");
}

#[test]
fn split_solid_refuses_without_a_modeler_and_over_budget() {
    let (mut app, mut session, ids) = app_with_meshes(2);
    select(&mut session, &ids);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::SplitSolid,
        CommandPayload::None,
    )
    .expect_err("no mesh modeler");
    assert!(unsupported_text(error).contains("unsupported-runtime: mesh-modeler"));

    let mut huge = domain_mesh();
    huge.triangles =
        vec![[0, 1, 2]; crate::app_drawing::mesh_solids::SPLITSOLID_MAX_INPUT_TRIANGLES + 1];
    let huge_entity = DbEntity {
        geometry: SemanticGeometry::Mesh(huge),
        ..mesh_entity(12, 0)
    };
    let (mut app, mut session, ids) = app_with_entities(vec![mesh_entity(10, 0), huge_entity]);
    attach_modeler(&mut app, None);
    select(&mut session, &ids);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::SplitSolid,
        CommandPayload::None,
    )
    .expect_err("over the split budget");
    assert!(invalid_text(error).contains("splitsolid.budget"));
}
