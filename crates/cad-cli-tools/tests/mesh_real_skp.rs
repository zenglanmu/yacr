//! Real-SketchUp mesh contract: import an actual `.skp` through the real
//! adapter and run the real `manifold-rust` modeler on the resulting meshes.
//!
//! ```text
//! YACR_TEST_SKP=/path/to/model.skp cargo test -p cad-cli-tools \
//!   --test mesh_real_skp -- --nocapture
//! ```
//!
//! An unset variable returns early; that skip is **not** evidence. The sample is
//! not committed (SketchUp has no redistributable geometry fixture for this
//! repository), so the test records the file size/hash only on the local run.

use std::sync::Arc;

use cad_connector_core::{ImportLimits, ImportRequest, Importer};
use cad_connector_skp::SkpImporter;
use cad_domain::{DatabaseId, DocumentId, Revision, SemanticGeometry, TaskStamp};
use cad_kernel_adapter::{
    codes, inspect_topology_welded, MeshBooleanOp, MeshData, MeshModeler, MeshOperation,
    MeshOutcome, MeshRepairMode, MeshRequest, MeshTopology, MeshWindingRule,
};
use cad_kernel_manifold::ManifoldMeshModeler;

fn stamp() -> TaskStamp {
    TaskStamp {
        document: DocumentId(1),
        generation: 1,
        object_revision: Revision(0),
        dependency_version: 0,
        configuration_version: 0,
    }
}

fn mesh_data(mesh: &cad_domain::Mesh) -> MeshData {
    MeshData::from_parts(mesh.vertices.clone(), mesh.triangles.clone())
}

fn diagnostics(outcome: &MeshOutcome) -> String {
    outcome
        .diagnostics()
        .iter()
        .map(|row| format!("{}: {}", row.code, row.message))
        .collect::<Vec<_>>()
        .join("; ")
}

#[test]
fn a_real_sketchup_model_is_classified_and_repaired_when_supplied() {
    let Ok(path) = std::env::var("YACR_TEST_SKP") else {
        return;
    };
    let bytes = std::fs::read(&path).expect("YACR_TEST_SKP points at a readable file");
    let request = ImportRequest {
        document: DocumentId(1),
        database: DatabaseId(1),
        bytes: Arc::from(bytes.into_boxed_slice()),
        limits: ImportLimits::default(),
        generation: 0,
        xref: None,
    };
    let drawing = SkpImporter::new()
        .import(&request, &|| false)
        .expect("a real model with visible geometry must import");

    let modeler = ManifoldMeshModeler::new();
    let mut meshes = 0usize;
    let mut triangles = 0usize;
    let mut closed = 0usize;
    let mut open = 0usize;
    let mut non_manifold = 0usize;
    let mut boundary_edges = 0usize;
    let mut repaired = 0usize;
    let mut repaired_non_manifold = 0usize;
    let mut refused_open = 0usize;
    let mut refused_non_manifold_closed = 0usize;
    let mut refused_non_manifold_manifold = 0usize;
    let mut closed_for_boolean: Vec<MeshData> = Vec::new();

    for entity in drawing.database.entities() {
        let mesh = match &entity.geometry {
            SemanticGeometry::Mesh(mesh) => mesh,
            SemanticGeometry::Body3d(body) => &body.display.mesh,
            _ => continue,
        };
        if mesh.triangles.is_empty() {
            continue;
        }
        meshes += 1;
        triangles += mesh.triangles.len();
        assert_eq!(
            mesh.face_sources.len(),
            mesh.triangles.len(),
            "entity {}: every triangle must name one source face",
            entity.id.0
        );
        let data = mesh_data(mesh);
        let report = inspect_topology_welded(&data);
        boundary_edges += report.boundary_edges;
        match report.topology {
            MeshTopology::ClosedManifold => {
                closed += 1;
                closed_for_boolean.push(data.clone());
            }
            MeshTopology::OpenShell => open += 1,
            MeshTopology::NonManifold => non_manifold += 1,
        }

        let request = MeshRequest {
            stamp: stamp(),
            operation: MeshOperation::Repair {
                mesh: &data,
                mode: MeshRepairMode::Orientation,
                rule: MeshWindingRule::Positive,
            },
        };
        let outcome = modeler
            .execute(&request, &|| false)
            .expect("a valid stored mesh must not be rejected as invalid input")
            .outcome;
        match report.topology {
            MeshTopology::OpenShell => {
                let reason = outcome.unsupported().unwrap_or_else(|| {
                    panic!("entity {}: open shell must be refused", entity.id.0)
                });
                assert_eq!(
                    reason.code,
                    codes::MESH_NOT_CLOSED,
                    "entity {}: open shell must name mesh.not_closed",
                    entity.id.0
                );
                refused_open += 1;
            }
            MeshTopology::ClosedManifold => {
                if outcome.is_success() {
                    repaired += 1;
                } else {
                    panic!(
                        "entity {}: a welded-closed mesh failed repair: {}",
                        entity.id.0,
                        diagnostics(&outcome)
                    );
                }
            }
            MeshTopology::NonManifold => {
                if outcome.is_success() {
                    repaired_non_manifold += 1;
                } else if let Some(reason) = outcome.unsupported() {
                    match reason.code {
                        codes::MESH_NOT_CLOSED => refused_non_manifold_closed += 1,
                        codes::MESH_NOT_MANIFOLD => refused_non_manifold_manifold += 1,
                        other => panic!(
                            "entity {}: non-manifold repair refused with an unexpected code: {other}",
                            entity.id.0
                        ),
                    }
                } else {
                    panic!(
                        "entity {}: repair produced no mesh: {}",
                        entity.id.0,
                        diagnostics(&outcome)
                    );
                }
            }
        }
    }

    let boolean_note = if closed_for_boolean.len() >= 2 {
        let left = &closed_for_boolean[0];
        let right = &closed_for_boolean[1];
        let request = MeshRequest {
            stamp: stamp(),
            operation: MeshOperation::Boolean {
                op: MeshBooleanOp::Union,
                left,
                right,
            },
        };
        let outcome = modeler
            .execute(&request, &|| false)
            .expect("valid closed meshes must not be invalid input")
            .outcome;
        assert!(
            outcome.is_success() || outcome.unsupported().is_some(),
            "a real closed union failed without a structured refusal: {}",
            diagnostics(&outcome)
        );
        format!(
            "union={}",
            if outcome.is_success() {
                "mesh".to_string()
            } else {
                outcome
                    .unsupported()
                    .map(|reason| reason.code.to_string())
                    .unwrap_or_default()
            }
        )
    } else {
        "union=skipped (fewer than two closed meshes)".to_string()
    };

    eprintln!(
        "real SKP mesh: file={path} bytes={} meshes={meshes} triangles={triangles} \
         closed={closed} open={open} non_manifold={non_manifold} boundary_edges={boundary_edges} \
         repaired={repaired} repaired_non_manifold={repaired_non_manifold} \
         refused_open={refused_open} refused_non_manifold_closed={refused_non_manifold_closed} \
         refused_non_manifold_manifold={refused_non_manifold_manifold} {boolean_note}",
        std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0)
    );

    assert!(meshes > 0, "the real sample must yield preview meshes");
    assert_eq!(
        closed + open + non_manifold,
        meshes,
        "every preview mesh has exactly one topology class"
    );
    assert_eq!(refused_open, open, "every open mesh is refused by name");
    assert_eq!(
        repaired_non_manifold + refused_non_manifold_closed + refused_non_manifold_manifold,
        non_manifold,
        "every non-manifold mesh has exactly one structured outcome"
    );
}
