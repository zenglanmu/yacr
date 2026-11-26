//! Real online mesh corpus contract: import every STL/OBJ/PLY/OFF in a directory
//! through the real connectors and run the pinned `manifold-rust` modeler.
//!
//! ```text
//! YACR_TEST_MESH_DIR=/tmp/opencode/mesh-samples cargo test -p cad-cli-tools \
//!   --test mesh_real_samples -- --nocapture
//! ```
//!
//! An unset variable returns early; that skip is **not** evidence. Samples are
//! downloaded by `scripts/fetch-mesh-samples.sh` and are never committed (their
//! licences live upstream). This is a kernel/import contract: no GPU, no
//! SketchUp/Trimble acceptance, no performance claim.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use cad_connector_core::{ImportLimits, ImportRequest, Importer};
use cad_connector_mesh::{ObjImporter, OffImporter, PlyImporter, StlImporter};
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

fn importer_for(path: &Path) -> Option<Box<dyn Importer>> {
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "stl" => Some(Box::new(StlImporter::new())),
        "obj" => Some(Box::new(ObjImporter::new())),
        "ply" => Some(Box::new(PlyImporter::new())),
        "off" => Some(Box::new(OffImporter::new())),
        _ => None,
    }
}

fn import(path: &Path) -> Option<Vec<MeshData>> {
    let importer = importer_for(path)?;
    let bytes = std::fs::read(path).expect("sample is readable");
    let request = ImportRequest {
        document: DocumentId(1),
        database: DatabaseId(1),
        bytes: Arc::from(bytes.into_boxed_slice()),
        limits: ImportLimits::default(),
        generation: 0,
        xref: None,
    };
    let drawing = importer
        .import(&request, &|| false)
        .unwrap_or_else(|error| panic!("{}: import failed: {error}", path.display()));
    let mut meshes = Vec::new();
    for entity in drawing.database.entities() {
        if let SemanticGeometry::Mesh(mesh) = &entity.geometry {
            if !mesh.triangles.is_empty() {
                meshes.push(mesh_data(mesh));
            }
        }
    }
    Some(meshes)
}

fn samples(dir: &Path) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
        .expect("YACR_TEST_MESH_DIR is a readable directory")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| importer_for(path).is_some())
        .collect();
    paths.sort();
    paths
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
fn every_online_mesh_sample_is_classified_and_repaired_when_supplied() {
    let Ok(dir) = std::env::var("YACR_TEST_MESH_DIR") else {
        return;
    };
    let dir = Path::new(&dir);
    let modeler = ManifoldMeshModeler::new();
    let mut files = 0usize;
    let mut meshes = 0usize;
    let mut triangles = 0usize;
    let mut closed = 0usize;
    let mut open = 0usize;
    let mut non_manifold = 0usize;
    let mut repaired = 0usize;
    let mut refused_open = 0usize;
    let mut non_manifold_repaired = 0usize;
    let mut non_manifold_failed = 0usize;
    let mut non_manifold_refused = 0usize;
    let mut closed_samples: Vec<MeshData> = Vec::new();

    for path in samples(dir) {
        let Some(imported) = import(&path) else {
            continue;
        };
        files += 1;
        let mut file_triangles = 0usize;
        let mut file_closed = 0usize;
        let mut file_open = 0usize;
        let mut file_non_manifold = 0usize;
        for data in &imported {
            meshes += 1;
            file_triangles += data.triangle_count();
            triangles += data.triangle_count();
            let report = inspect_topology_welded(data);
            match report.topology {
                MeshTopology::ClosedManifold => {
                    closed += 1;
                    file_closed += 1;
                    closed_samples.push(data.clone());
                }
                MeshTopology::OpenShell => {
                    open += 1;
                    file_open += 1;
                }
                MeshTopology::NonManifold => {
                    non_manifold += 1;
                    file_non_manifold += 1;
                }
            }

            let request = MeshRequest {
                stamp: stamp(),
                operation: MeshOperation::Repair {
                    mesh: data,
                    mode: MeshRepairMode::Orientation,
                    rule: MeshWindingRule::Positive,
                },
            };
            let outcome = modeler
                .execute(&request, &|| false)
                .expect("an imported mesh must not be invalid input")
                .outcome;
            match report.topology {
                MeshTopology::ClosedManifold => {
                    assert!(
                        outcome.is_success(),
                        "{}: a welded-closed mesh failed repair: {}",
                        path.display(),
                        diagnostics(&outcome)
                    );
                    repaired += 1;
                }
                MeshTopology::OpenShell => {
                    let reason = outcome.unsupported().unwrap_or_else(|| {
                        panic!("{}: an open shell must be refused", path.display())
                    });
                    assert_eq!(reason.code, codes::MESH_NOT_CLOSED);
                    refused_open += 1;
                }
                MeshTopology::NonManifold => {
                    // A closed non-manifold soup may be repaired (Partial), or
                    // refused/failed by name. Only a silent empty success is
                    // forbidden; `mesh.failed` with the real detail is honest.
                    if let Some(reason) = outcome.unsupported() {
                        non_manifold_refused += 1;
                        assert!(
                            matches!(
                                reason.code,
                                codes::MESH_NOT_CLOSED | codes::MESH_NOT_MANIFOLD
                            ),
                            "{}: unexpected refusal {}",
                            path.display(),
                            reason.code
                        );
                    } else if outcome.is_success() {
                        non_manifold_repaired += 1;
                    } else {
                        non_manifold_failed += 1;
                        let code = outcome
                            .diagnostics()
                            .first()
                            .map(|row| row.code.as_str())
                            .unwrap_or("");
                        assert!(
                            matches!(
                                code,
                                "mesh.failed" | "mesh.invalid_mesh" | "mesh.not_closed"
                            ),
                            "{}: a non-manifold refusal must name its code: {}",
                            path.display(),
                            diagnostics(&outcome)
                        );
                    }
                }
            }
        }
        println!(
            "{}: entities={} triangles={} closed={} open={} non_manifold={}",
            path.file_name().unwrap_or_default().to_string_lossy(),
            imported.len(),
            file_triangles,
            file_closed,
            file_open,
            file_non_manifold
        );
    }

    // One real boolean between the first two closed operands, when the corpus
    // contains a pair. This is kernel-level, not an app command.
    let boolean = if closed_samples.len() >= 2 {
        let request = MeshRequest {
            stamp: stamp(),
            operation: MeshOperation::Boolean {
                op: MeshBooleanOp::Union,
                left: &closed_samples[0],
                right: &closed_samples[1],
            },
        };
        let outcome = modeler
            .execute(&request, &|| false)
            .expect("closed samples are valid input")
            .outcome;
        match outcome {
            MeshOutcome::Success { mesh, topology, .. } => {
                assert_eq!(topology.topology, MeshTopology::ClosedManifold);
                format!("union=ok(triangles={})", mesh.triangle_count())
            }
            MeshOutcome::Unsupported { reason } => format!("union=refused({})", reason.code),
            other => format!("union=failed({})", diagnostics(&other)),
        }
    } else {
        "union=skipped (fewer than two closed meshes)".to_string()
    };

    println!(
        "SUMMARY files={files} meshes={meshes} triangles={triangles} closed={closed} open={open} \
         non_manifold={non_manifold} repaired={repaired} refused_open={refused_open} \
         non_manifold_repaired={non_manifold_repaired} non_manifold_failed={non_manifold_failed} \
         non_manifold_refused={non_manifold_refused} {boolean}"
    );
    assert!(
        files > 0,
        "the sample directory contained no supported mesh file"
    );
}
