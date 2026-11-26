//! The [`MeshModeler`] implementation over the pinned `manifold-rust` kernel.
//!
//! `manifold-rust` is a stateless library: a `Manifold` value is immutable
//! content, not a document handle. This modeler is therefore zero-sized and
//! `Send + Sync`; two calls share nothing and no lock of ours is needed.

use cad_domain::{CadError, CadResult, Diagnostic, Point3, Registration};
use cad_kernel_adapter::{
    codes, inspect_topology_welded, MeshBooleanOp, MeshData, MeshModeler, MeshModelerCapabilities,
    MeshOperation, MeshOperationFamily, MeshOutcome, MeshRepair, MeshRepairMode, MeshRequest,
    MeshResult, MeshTopology, MeshUnsupportedReason, MeshWindingRule,
};
use manifold_rust::manifold::Manifold;
use manifold_rust::types::{BooleanEngine, Error as KernelError, MeshGL64, OpType, WindingRule};

/// The seam's registration type key: this is the `manifold-rust` modeler.
pub const TYPE_KEY: &str = "yacr.kernel.manifold-rust";

/// The pinned upstream version this build links.
pub const UPSTREAM_VERSION: &str = "0.19.6";

/// The mesh modeler backed by the pinned `manifold-rust` kernel.
#[derive(Debug, Default, Clone, Copy)]
pub struct ManifoldMeshModeler;

impl ManifoldMeshModeler {
    /// A new (zero-sized) modeler.
    pub fn new() -> Self {
        ManifoldMeshModeler
    }
}

/// The capability declaration of this build.
///
/// `robust_boolean` is the exact-rational engine; it is what accepts a closed
/// non-manifold triangle soup. `repair_orientation` and `rebuild_solid` are the
/// two repair modes ADR-0061 selected. Hole filling (open shells) is **not**
/// declared: it is P-MESH-3.
pub fn declared_capabilities() -> MeshModelerCapabilities {
    MeshModelerCapabilities {
        boolean: true,
        robust_boolean: true,
        repair_orientation: true,
        rebuild_solid: true,
    }
}

impl MeshModeler for ManifoldMeshModeler {
    fn registration(&self) -> Registration {
        Registration {
            type_key: TYPE_KEY.into(),
            version: 1,
            priority: 10,
            // The modeler creates no entity class: callers keep `Mesh` /
            // `Body3d` ownership and only borrow the result.
            entity_types: Vec::new(),
            capabilities: vec![
                "mesh.boolean.union".into(),
                "mesh.boolean.subtract".into(),
                "mesh.boolean.intersect".into(),
                "mesh.boolean.robust".into(),
                "mesh.repair.orientation".into(),
                "mesh.repair.rebuild".into(),
                format!("manifold-rust.version={UPSTREAM_VERSION}"),
            ],
        }
    }

    fn capabilities(&self) -> MeshModelerCapabilities {
        declared_capabilities()
    }

    fn execute(
        &self,
        request: &MeshRequest<'_>,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> CadResult<MeshResult> {
        // Cancellation before any work, and before input validation: a request
        // nobody waits for is never worth a diagnostic.
        if cancelled() {
            return Err(CadError::Cancelled);
        }
        request.validate()?;

        if !self.capabilities().supports(&request.operation) {
            let outcome = unsupported(
                request.operation.family(),
                codes::MESH_UNSUPPORTED,
                "the manifold-rust modeler does not declare this operation",
            );
            return Ok(MeshResult {
                stamp: request.stamp.clone(),
                outcome,
            });
        }

        let outcome = match request.operation {
            MeshOperation::Boolean { op, left, right } => boolean(op, left, right, cancelled),
            MeshOperation::Repair { mesh, mode, rule } => repair(mesh, mode, rule, cancelled),
        }?;

        // A post-completion cancellation is an error, not a product: the result
        // must never reach the database.
        if cancelled() {
            return Err(CadError::Cancelled);
        }
        Ok(MeshResult {
            stamp: request.stamp.clone(),
            outcome,
        })
    }
}

/// Boolean union/subtract/intersect.
///
/// The local topology census refuses an open operand before any kernel call
/// (`mesh.not_closed`) so the reason is stable and does not depend on the
/// kernel's own import classification. A closed non-manifold soup is passed to
/// the robust engine (`Auto`); it is not silently treated as a manifold.
fn boolean(
    op: MeshBooleanOp,
    left: &MeshData,
    right: &MeshData,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> CadResult<MeshOutcome> {
    let family = MeshOperationFamily::Boolean;
    for (name, mesh) in [("left", left), ("right", right)] {
        let report = inspect_topology_welded(mesh);
        if report.boundary_edges > 0 {
            return Ok(unsupported(
                family,
                codes::MESH_NOT_CLOSED,
                format!(
                    "the {name} operand has {} boundary edge(s); a mesh boolean needs a \
                     geometrically closed, orientable shell (hole filling is not attempted)",
                    report.boundary_edges
                ),
            ));
        }
    }

    let a = match import_mesh(left, family) {
        Ok(manifold) => manifold,
        Err(KernelRun::Cancelled) => return Err(CadError::Cancelled),
        Err(KernelRun::Refused(outcome)) => return Ok(*outcome),
    };
    let b = match import_mesh(right, family) {
        Ok(manifold) => manifold,
        Err(KernelRun::Cancelled) => return Err(CadError::Cancelled),
        Err(KernelRun::Refused(outcome)) => return Ok(*outcome),
    };

    let result = boolean_with_cancel(&a, &b, op, cancelled);
    if result.status() != KernelError::NoError {
        return match status_outcome(result.status(), family) {
            KernelRun::Cancelled => Err(CadError::Cancelled),
            KernelRun::Refused(outcome) => Ok(*outcome),
        };
    }

    match mesh_from_kernel(&result, family) {
        Ok(mesh) => Ok(MeshOutcome::Success {
            topology: inspect_topology_welded(&mesh),
            mesh,
            diagnostics: Vec::new(),
        }),
        Err(KernelRun::Cancelled) => Err(CadError::Cancelled),
        Err(KernelRun::Refused(outcome)) => Ok(*outcome),
    }
}

/// Orientation repair or winding-number rebuild.
///
/// Both modes require the input to be closed; an open shell is refused before
/// the kernel call. The result is always `Partial`: a repaired mesh is not exact
/// B-rep geometry, and the report names what was requested and what came out.
fn repair(
    mesh: &MeshData,
    mode: MeshRepairMode,
    rule: MeshWindingRule,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> CadResult<MeshOutcome> {
    let family = MeshOperationFamily::Repair;
    let input = inspect_topology_welded(mesh);
    if input.boundary_edges > 0 {
        return Ok(unsupported(
            family,
            codes::MESH_NOT_CLOSED,
            format!(
                "repair needs a geometrically closed, orientable shell; the input has {} \
                 boundary edge(s) (hole filling is P-MESH-3)",
                input.boundary_edges
            ),
        ));
    }

    let source = match import_mesh(mesh, family) {
        Ok(manifold) => manifold,
        Err(KernelRun::Cancelled) => return Err(CadError::Cancelled),
        Err(KernelRun::Refused(outcome)) => return Ok(*outcome),
    };
    let repaired = repair_with_cancel(&source, mode, rule, cancelled);
    if repaired.status() != KernelError::NoError {
        return match status_outcome(repaired.status(), family) {
            KernelRun::Cancelled => Err(CadError::Cancelled),
            KernelRun::Refused(outcome) => Ok(*outcome),
        };
    }

    let output_mesh = match mesh_from_kernel(&repaired, family) {
        Ok(mesh) => mesh,
        Err(KernelRun::Cancelled) => return Err(CadError::Cancelled),
        Err(KernelRun::Refused(outcome)) => return Ok(*outcome),
    };
    let output = inspect_topology_welded(&output_mesh);
    if output.topology != MeshTopology::ClosedManifold {
        return Ok(failed(
            codes::MESH_FAILED,
            format!(
                "repair produced a {} mesh ({} boundary, {} non-manifold edge(s)); a repair \
                 product must be a closed 2-manifold",
                output.topology.as_str(),
                output.boundary_edges,
                output.non_manifold_edges
            ),
        ));
    }

    Ok(MeshOutcome::Partial {
        mesh: output_mesh,
        repair: MeshRepair {
            mode,
            rule,
            input,
            output,
            welded_vertices: input.welded_vertices,
        },
        diagnostics: Vec::new(),
    })
}

/// Convert a neutral mesh into the kernel's `MeshGL64` (f64 positions, u64
/// indices, three position properties per vertex).
pub(crate) fn to_kernel_mesh(data: &MeshData) -> MeshGL64 {
    let vert_properties = data
        .vertices
        .iter()
        .flat_map(|point| [point.x, point.y, point.z])
        .collect();
    let tri_verts = data
        .triangles
        .iter()
        .flat_map(|triangle| triangle.iter().map(|index| u64::from(*index)))
        .collect();
    MeshGL64 {
        num_prop: 3,
        vert_properties,
        tri_verts,
        ..Default::default()
    }
}

/// Import a neutral mesh through the kernel's **robust** path (non-manifold
/// closed soup is kept; open/unorientable input is refused by name).
pub(crate) fn import_mesh(
    data: &MeshData,
    family: MeshOperationFamily,
) -> Result<Manifold, KernelRun> {
    let kernel = Manifold::from_mesh_gl64_robust(&to_kernel_mesh(data));
    match kernel.status() {
        KernelError::NoError => Ok(kernel),
        status => Err(status_outcome(status, family)),
    }
}

/// Export one kernel mesh back to neutral data.
pub(crate) fn mesh_from_kernel(
    manifold: &Manifold,
    family: MeshOperationFamily,
) -> Result<MeshData, KernelRun> {
    match manifold.status() {
        KernelError::NoError => {}
        status => return Err(status_outcome(status, family)),
    }
    let mesh = manifold.get_mesh_gl64(-1);
    if mesh.num_vert() == 0 || mesh.num_tri() == 0 {
        return Err(KernelRun::Refused(Box::new(failed(
            codes::MESH_FAILED,
            "the kernel produced an empty mesh (no usable product)",
        ))));
    }
    let num_prop = mesh.num_prop as usize;
    if num_prop < 3 || !mesh.vert_properties.len().is_multiple_of(num_prop) {
        return Err(KernelRun::Refused(Box::new(failed(
            codes::MESH_INVALID,
            format!(
                "the kernel returned {} vertex properties per vertex, which cannot carry xyz",
                mesh.num_prop
            ),
        ))));
    }
    let vertices: Vec<Point3> = mesh
        .vert_properties
        .chunks_exact(num_prop)
        .map(|chunk| Point3 {
            x: chunk[0],
            y: chunk[1],
            z: chunk[2],
        })
        .collect();
    let (triangles, rest) = mesh.tri_verts.as_chunks::<3>();
    if !rest.is_empty() {
        return Err(KernelRun::Refused(Box::new(failed(
            codes::MESH_INVALID,
            "the kernel returned an index buffer whose length is not a multiple of three",
        ))));
    }
    let mut out_triangles = Vec::with_capacity(triangles.len());
    for triangle in triangles {
        let mut out = [0u32; 3];
        for (slot, index) in out.iter_mut().zip(triangle) {
            *slot = u32::try_from(*index).map_err(|_| {
                KernelRun::Refused(Box::new(failed(
                    codes::MESH_RESULT_TOO_LARGE,
                    format!("the kernel returned vertex index {index}, wider than u32"),
                )))
            })?;
        }
        out_triangles.push(out);
    }
    let data = MeshData::from_parts(vertices, out_triangles);
    data.validate().map_err(|error| {
        KernelRun::Refused(Box::new(failed(
            codes::MESH_INVALID,
            format!("the kernel produced an invalid mesh: {error}"),
        )))
    })?;
    Ok(data)
}

/// The signed volume of a neutral mesh, re-imported through the robust kernel.
/// Used by the contract tests; a `None` means the kernel could not import it.
#[cfg(test)]
pub(crate) fn volume_of(mesh: &MeshData) -> Option<f64> {
    let kernel = Manifold::from_mesh_gl64_robust(&to_kernel_mesh(mesh));
    (kernel.status() == KernelError::NoError).then(|| kernel.volume())
}

fn op_type(op: MeshBooleanOp) -> OpType {
    match op {
        MeshBooleanOp::Union => OpType::Add,
        MeshBooleanOp::Subtract => OpType::Subtract,
        MeshBooleanOp::Intersect => OpType::Intersect,
    }
}

fn winding_rule(rule: MeshWindingRule) -> WindingRule {
    match rule {
        MeshWindingRule::Positive => WindingRule::Positive,
        MeshWindingRule::Nonzero => WindingRule::Nonzero,
    }
}

/// A kernel result that is either a structured product/refusal or a real
/// cancellation. Cancellation is **not** a `MeshOutcome`: it is an error and must
/// never be committed (ADR-0061 D7 / seam contract).
#[derive(Debug)]
pub(crate) enum KernelRun {
    Refused(Box<MeshOutcome>),
    Cancelled,
}

fn status_outcome(status: KernelError, family: MeshOperationFamily) -> KernelRun {
    match status {
        KernelError::NotClosed => KernelRun::Refused(Box::new(unsupported(
            family,
            codes::MESH_NOT_CLOSED,
            "the kernel requires a closed mesh",
        ))),
        KernelError::NotManifold => KernelRun::Refused(Box::new(unsupported(
            family,
            codes::MESH_NOT_MANIFOLD,
            "the kernel cannot pair the mesh's halfedges",
        ))),
        KernelError::ResultTooLarge => KernelRun::Refused(Box::new(failed(
            codes::MESH_RESULT_TOO_LARGE,
            "the result exceeds the kernel's index/result limits",
        ))),
        KernelError::Cancelled => KernelRun::Cancelled,
        other => KernelRun::Refused(Box::new(failed(
            codes::MESH_FAILED,
            format!("the kernel failed with status {other:?}"),
        ))),
    }
}

/// Run a boolean with a real kernel cancellation token on native targets.
///
/// The host's `cancelled` closure is not a kernel token, so a small watchdog
/// thread bridges it: when the closure turns true, it calls
/// `CancelToken::cancel()` and the kernel returns `Cancelled` at its next check.
/// The thread is scoped, so a panic in the kernel cannot leave it running.
#[cfg(not(target_arch = "wasm32"))]
fn boolean_with_cancel(
    left: &Manifold,
    right: &Manifold,
    op: MeshBooleanOp,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Manifold {
    use manifold_rust::cancel::CancelToken;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    let token = CancelToken::new();
    let done = AtomicBool::new(false);
    std::thread::scope(|scope| {
        let done = &done;
        let token = &token;
        scope.spawn(move || {
            while !done.load(Ordering::Relaxed) {
                if cancelled() {
                    token.cancel();
                    break;
                }
                std::thread::sleep(Duration::from_millis(2));
            }
        });
        let _finished = DoneGuard(done);
        left.boolean_with_engine_and_token(right, op_type(op), BooleanEngine::Auto, Some(token))
    })
}

/// wasm has no threads: only the pre/post checks in `execute` apply.
#[cfg(target_arch = "wasm32")]
fn boolean_with_cancel(
    left: &Manifold,
    right: &Manifold,
    op: MeshBooleanOp,
    _cancelled: &(dyn Fn() -> bool + Sync),
) -> Manifold {
    left.boolean_with_engine(right, op_type(op), BooleanEngine::Auto)
}

#[cfg(not(target_arch = "wasm32"))]
fn repair_with_cancel(
    source: &Manifold,
    mode: MeshRepairMode,
    rule: MeshWindingRule,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Manifold {
    use manifold_rust::cancel::CancelToken;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    let token = CancelToken::new();
    let done = AtomicBool::new(false);
    std::thread::scope(|scope| {
        let done = &done;
        let token = &token;
        scope.spawn(move || {
            while !done.load(Ordering::Relaxed) {
                if cancelled() {
                    token.cancel();
                    break;
                }
                std::thread::sleep(Duration::from_millis(2));
            }
        });
        let _finished = DoneGuard(done);
        match mode {
            MeshRepairMode::Orientation => source.repair_orientation_with_token(Some(token)),
            MeshRepairMode::Rebuild => {
                source.rebuild_solid_with_token(winding_rule(rule), Some(token))
            }
        }
    })
}

#[cfg(target_arch = "wasm32")]
fn repair_with_cancel(
    source: &Manifold,
    mode: MeshRepairMode,
    rule: MeshWindingRule,
    _cancelled: &(dyn Fn() -> bool + Sync),
) -> Manifold {
    match mode {
        MeshRepairMode::Orientation => source.repair_orientation(),
        MeshRepairMode::Rebuild => source.rebuild_solid(winding_rule(rule)),
    }
}

/// Sets the watcher's stop flag even when the kernel panics.
#[cfg(not(target_arch = "wasm32"))]
struct DoneGuard<'a>(&'a std::sync::atomic::AtomicBool);

#[cfg(not(target_arch = "wasm32"))]
impl Drop for DoneGuard<'_> {
    fn drop(&mut self) {
        self.0.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

fn unsupported(
    operation: MeshOperationFamily,
    code: &'static str,
    detail: impl Into<String>,
) -> MeshOutcome {
    MeshOutcome::Unsupported {
        reason: MeshUnsupportedReason {
            code,
            operation,
            detail: detail.into(),
        },
    }
}

fn failed(code: &'static str, message: impl Into<String>) -> MeshOutcome {
    MeshOutcome::Failed {
        diagnostics: vec![Diagnostic {
            object: None,
            code: code.into(),
            message: message.into(),
        }],
    }
}
