//! The display mesher of the pinned kernel (`solidcore-tess`) behind the
//! kernel-independent [`SolidTessellator`] seam (ADR-0036 P2.1c part 1).
//!
//! The pinned record writer (`record::to_record`, through [`crate::ops::lift`])
//! can already express a NURBS face; the neutral tessellator cannot mesh one. This
//! module is the other half: it lifts the same neutral body to a record, asks
//! `solidcore-tess` to plan every face in its own `(u, v)` and evaluates that plan
//! on the CPU, then maps the result onto the seam's [`TessellationMesh`].
//!
//! # What it meshes, and what it refuses
//!
//! * **Only** a neutral [`SolidExchange::Brep`] payload is evaluated. Raw SAT/SAB
//!   bytes are lifted by an importer, never here, and every other kind is
//!   [`TessellationOutcome::Unsupported`]
//!   with the exchange kind and a stable code. There is never a fabricated mesh.
//! * A body the lift refuses (more than one shell, an open sheet, an unsupported
//!   boundary family, ...) is `Unsupported` carrying **the refusal's own code and
//!   message**: the seam does not re-word what the kernel said.
//! * A body the mesher cannot plan at all is `Unsupported` with
//!   `kernel.unsupported_surface`; a caught mesher panic is `Failed` with
//!   `kernel.failed`.
//! * An evaluated mesh with no triangle is `Failed` with
//!   `kernel.empty_geometry`: there is no "success with no mesh" here either.
//!
//! # Tolerance
//!
//! The tolerance is **the request's**, never the mesher's own default:
//!
//! | request | `solidcore_tess::MeshTolerance` |
//! |---|---|
//! | `linear_deflection` (mm, max chordal deviation) | `chord` |
//! | `angular_deflection` (rad, max facet turn) | `angle: Some(..)` |
//!
//! The planner does clamp internally: it never goes finer than `1e-6 x size`
//! (size = the body's bounding-box diagonal, `1.0` when it has no extent, the same
//! fallback [`cad_kernel_adapter::shape_relative_tolerance`] uses) and it floors
//! the facet angle at one degree. The chord is never clamped coarser than that
//! floor, so `Precision::Approximate { error_bound }` reports the chord **actually
//! used** — `max(linear_deflection, 1e-6 x size)` — and never
//! [`Precision::Analytic`]: every face of a NURBS-bearing body is drawn as facets.
//! A request below the floor therefore produces a mesh at the floor and says so
//! with `kernel.tolerance_not_met` rather than claiming the requested accuracy.
//!
//! # Honesty of a partial display
//!
//! The upstream plan reports, per face and per edge, what it could not do. Every
//! one of those reports is carried, never dropped:
//!
//! * a face the planner left out of the plan, or the evaluator refused, becomes a
//!   [`FaceRef`] in [`TessellationDegradation::missing_faces`] (deduplicated by
//!   face, with the upstream reason text verbatim);
//! * a face the planner filled **leniently** (drawn, but not meshed to its trim),
//!   a face whose refinement stopped above the chord tolerance, an edge whose
//!   sampling stopped at a bound above it, and an edge that could not be sampled
//!   at all become diagnostics with the stable [`codes::KERNEL_TOLERANCE_NOT_MET`].
//!
//! Any of those makes the outcome [`TessellationOutcome::Partial`]; only a run
//! with nothing lost is [`TessellationOutcome::Success`]. A fidelity shortfall
//! that has no missing face is still `Partial`, with an empty degradation and the
//! diagnostics as its only record: the neutral degradation has no slot for "drawn
//! less precisely than asked", and inventing a missing face for one would be a
//! lie.
//!
//! There is deliberately **no positional open-edge scan** here (the neutral
//! path's `detect_open_edges`): the plan samples every edge once and hands the
//! neighbouring faces bit-identical boundary points, and the mesher's own
//! refusals are already carried above. A second, positional authority over the
//! same mesh would only be able to disagree with them.
//!
//! # Cancellation
//!
//! The seam's `cancelled` callback is checked **before** validation and again
//! **after** evaluation; the plan itself is not polled (it takes no cancel
//! token), so this is cooperative cancellation at the call's boundaries, with the
//! mesh dropped when the second check fires. It is not the modeler's poll-based
//! watcher, and no claim of mid-plan cancellation is made.

use cad_domain::{
    CadError, CadResult, Diagnostic, Mesh, Point3, Precision, Registration, Revision, SubElementId,
};
use cad_kernel_adapter::{
    brep_diagonal, codes, ExchangeKind, FaceRef, GeometryHandle, SolidExchange, SolidTessellator,
    TessellationDegradation, TessellationMesh, TessellationOutcome, TessellationRequest,
    TessellationResult, UnsupportedReason,
};

/// The seam's registration key: the pinned kernel's display mesher, by name.
pub const TYPE_KEY: &str = "yacr.kernel.solidcore.tess";

/// The fraction of the body's size below which the planner will not refine
/// (`solidcore-tess`'s own floor). A request finer than this is served at the
/// floor, and [`SolidCoreTessellator`] says so.
const FINEST_CHORD_FRACTION: f64 = 1e-6;

/// The `solidcore-tess` display mesher over the pinned kernel revision.
///
/// Zero-sized and stateless, like [`crate::SolidCoreModeler`]: content in,
/// content out. One instance per process is enough.
#[derive(Debug, Default, Clone, Copy)]
pub struct SolidCoreTessellator;

impl SolidCoreTessellator {
    /// A new (zero-sized) tessellator.
    pub fn new() -> Self {
        SolidCoreTessellator
    }

    /// Evaluate one validated request. Cancellation is the caller's boundary
    /// check (see the module docs).
    fn evaluate(request: &TessellationRequest) -> TessellationOutcome {
        if let GeometryHandle::Missing { key } = &request.geometry {
            return failed(
                codes::KERNEL_MISSING_HANDLE,
                format!("geometry handle '{key}' could not be resolved"),
            );
        }
        if request.exchange.is_empty() {
            return failed(
                codes::KERNEL_EMPTY_GEOMETRY,
                format!(
                    "{} payload carried no geometry",
                    request.exchange.type_key()
                ),
            );
        }
        let brep = match &request.exchange {
            SolidExchange::Brep(brep) => brep,
            // Raw ACIS bytes are the importer's to lift; a mesher never parses
            // them. Same category (and code) the neutral path reports.
            SolidExchange::Sat(_) | SolidExchange::Sab(_) => {
                return unsupported(
                    request.exchange.kind(),
                    codes::KERNEL_NO_ACIS_KERNEL,
                    "raw SAT/SAB bytes are lifted by the importer, not by this tessellator"
                        .to_string(),
                );
            }
            SolidExchange::Unsupported { .. } => {
                return unsupported(
                    ExchangeKind::Unsupported,
                    codes::KERNEL_UNSUPPORTED_EXCHANGE,
                    format!(
                        "no decoder registered for exchange type '{}'",
                        request.exchange.type_key()
                    ),
                );
            }
        };

        // The lift is the same gate every modelling operation uses, so a body
        // this mesher draws is one the kernel accepted structurally; a refusal
        // keeps its own stable code and its own text.
        let record = match crate::ops::lift(brep) {
            Ok(record) => record,
            Err(refusal) => {
                return unsupported(ExchangeKind::Brep, refusal.code, refusal.detail);
            }
        };

        let size = brep_diagonal(brep).unwrap_or(1.0);
        let requested = request.tolerance.linear_deflection;
        // The planner's chord, and the chord it will actually use (its own floor
        // is `size x 1e-6`; the facet angle it floors at one degree is a direction
        // bound, not a length, so it does not enter `error_bound`).
        let tolerance = solidcore_tess::MeshTolerance {
            chord: requested,
            angle: Some(request.tolerance.angular_deflection),
        };
        let chord_used = requested.max(size * FINEST_CHORD_FRACTION);
        let (plan, stats) = match solidcore_tess::plan(&record, &tolerance, size) {
            Ok(planned) => planned,
            Err(error) => return plan_failure(&error),
        };
        let (mesh, refused) = plan.evaluate();

        // The record's face order is the neutral body's face order (the lift walks
        // `shell.faces` in order), so a `tri_face` index names the same face the
        // neutral path would stamp. The shell id is the lift's single shell.
        let record_faces: Vec<u32> = brep
            .shells
            .iter()
            .flat_map(|shell| &shell.faces)
            .map(|face| face.id)
            .collect();
        let shell_id = brep.shells.first().map_or(0, |shell| shell.id);

        if mesh.normals.len() != mesh.positions.len() {
            // One normal per position is the neutral mesh's invariant. A mesher
            // answer that breaks it is not displayed half-filled.
            return failed(
                crate::KERNEL_FAILED,
                format!(
                    "solidcore-tess returned {} normals for {} vertices",
                    mesh.normals.len(),
                    mesh.positions.len()
                ),
            );
        }
        if mesh.tri_face.len() != mesh.triangles.len() {
            return failed(
                crate::KERNEL_FAILED,
                format!(
                    "solidcore-tess returned {} face ids for {} triangles",
                    mesh.tri_face.len(),
                    mesh.triangles.len()
                ),
            );
        }
        if mesh.triangles.is_empty() {
            // No usable facet set is never a success, whatever the stats say.
            return failed(
                codes::KERNEL_EMPTY_GEOMETRY,
                "solidcore-tess evaluated the body to no triangle".to_string(),
            );
        }
        let vertices: Vec<Point3> = mesh.positions.iter().map(|v| point(*v)).collect();
        if mesh
            .triangles
            .iter()
            .flatten()
            .any(|&index| index as usize >= vertices.len())
        {
            return failed(
                crate::KERNEL_FAILED,
                "solidcore-tess returned a triangle index past its own vertex list".to_string(),
            );
        }

        let face_sources: Vec<Option<SubElementId>> = mesh
            .tri_face
            .iter()
            .map(|&index| {
                // `brep:face:<shell>/<face>`, the same convention the neutral path
                // stamps. `Revision(0)` is its documented placeholder (see
                // `cad_kernel_adapter::brep_tessellator`): nothing invalidates
                // these ids on a topology change yet. An index outside the lifted
                // face list cannot occur for a record lifted from this body; it
                // claims no source rather than a fabricated id.
                record_faces
                    .get(index as usize)
                    .copied()
                    .map(|id| SubElementId {
                        source_key: format!("brep:face:{shell_id}/{id}"),
                        topology_revision: Revision(0),
                    })
            })
            .collect();
        let normals: Vec<Point3> = mesh.normals.iter().map(|n| point(*n)).collect();
        let edges: Vec<Vec<Point3>> = mesh
            .edges
            .iter()
            .map(|polyline| polyline.iter().map(|v| point(*v)).collect())
            .collect();

        let geometry = TessellationMesh {
            mesh: Mesh {
                vertices,
                triangles: mesh.triangles.clone(),
                normals,
                face_sources,
                // The neutral assembly carries no vertex colours either.
                colors: Vec::new(),
                uvs: Vec::new(),
                texture: None,
                pbr: None,
            },
            edges,
            precision: Precision::Approximate {
                error_bound: Some(chord_used),
            },
        };

        // Refused/unmeshed faces: the plan's own list plus the evaluator's (a face
        // whose nodes the surface refused), deduplicated by face so a face is
        // named once with the first reason that reached us.
        let mut missing_faces: Vec<FaceRef> = Vec::new();
        // The neutral face id a record face index names. An index outside this
        // lift's face list cannot occur for a record lifted from this very body;
        // should it ever, its own index is reported rather than a guessed id.
        let face_id = |index: usize| -> u32 {
            record_faces
                .get(index)
                .copied()
                .unwrap_or_else(|| u32::try_from(index).unwrap_or(u32::MAX))
        };
        for face in stats.unmeshed.iter().chain(refused.iter()) {
            let id = face_id(face.face);
            if missing_faces.iter().any(|known| known.id == id) {
                continue;
            }
            missing_faces.push(FaceRef {
                id,
                reason: face.reason.clone(),
            });
        }

        let mut diagnostics: Vec<Diagnostic> = Vec::new();
        if chord_used > requested {
            diagnostics.push(tolerance_not_met(format!(
                "the mesher will not refine below {chord_used:.6e} for a body of size {size:.6e}; \
                 the requested chord {requested:.6e} was served coarser"
            )));
        }
        for face in &stats.lenient {
            diagnostics.push(tolerance_not_met(format!(
                "face {} could not be meshed to its trim and was filled leniently: {}",
                face_id(face.face),
                face.reason
            )));
        }
        for face in &stats.over_tolerance {
            if face.within {
                continue;
            }
            diagnostics.push(tolerance_not_met(format!(
                "face {} reaches a sag of {:.6e} against a chord tolerance of {:.6e} (its \
                 refinement stopped at a memory bound)",
                face_id(face.face),
                face.sag,
                face.tol
            )));
        }
        for edge in &stats.edges_capped {
            if edge.within {
                continue;
            }
            diagnostics.push(tolerance_not_met(format!(
                "edge {} lies {:.6e} off its curve against a chord tolerance of {:.6e} (its \
                 sampling stopped at a bound)",
                edge.edge, edge.sag, edge.tol
            )));
        }
        for edge in &stats.unsampled_edges {
            diagnostics.push(tolerance_not_met(format!(
                "edge {} was left without a display polyline: {}",
                edge.edge, edge.reason
            )));
        }

        if let Err(diagnostic) = request.budget.check(&geometry) {
            return TessellationOutcome::Failed {
                diagnostics: vec![diagnostic],
            };
        }

        let degradation = TessellationDegradation {
            missing_faces,
            // Deliberately not a positional scan: see the module docs.
            open_edges: Vec::new(),
            dropped_shells: Vec::new(),
        };
        if degradation.is_empty() && diagnostics.is_empty() {
            TessellationOutcome::Success {
                geometry,
                diagnostics: Vec::new(),
            }
        } else {
            TessellationOutcome::Partial {
                geometry,
                degradation,
                diagnostics,
            }
        }
    }
}

impl SolidTessellator for SolidCoreTessellator {
    fn registration(&self) -> Registration {
        Registration {
            type_key: TYPE_KEY.into(),
            // The seam's contract version; the upstream `1.0.1+<rev>` string is
            // advertised by the modeler's registration (one pin, one place).
            version: 1,
            priority: 10,
            // The two classes this mesher draws. `REGION`/`SURFACE` are sheets,
            // which the lift refuses by name (one use per edge), so they are not
            // advertised.
            entity_types: vec!["3DSOLID".into(), "BODY".into()],
            // Both face families go through the same exact surface evaluator: the
            // neutral model's analytic faces are evaluated as their exact form, and
            // a form face is meshed in its own parameters.
            capabilities: vec!["nurbs-faces".into(), "analytic-faces".into()],
        }
    }

    fn tessellate(
        &self,
        request: &TessellationRequest,
        cancelled: &dyn Fn() -> bool,
    ) -> CadResult<TessellationResult> {
        // Cancellation before any work, and before input validation: a request
        // nobody waits for is never worth a diagnostic.
        if cancelled() {
            return Err(CadError::Cancelled);
        }
        request
            .validate()
            .map_err(|error| CadError::InvalidInput(error.to_string()))?;
        let outcome = Self::evaluate(request);
        // A mesh that arrives after cancellation is dropped, never returned.
        if cancelled() {
            return Err(CadError::Cancelled);
        }
        Ok(TessellationResult {
            stamp: request.stamp.clone(),
            outcome,
        })
    }
}

/// An upstream position/normal/direction in the neutral model.
fn point(v: solidcore_record::Vec3) -> Point3 {
    Point3 {
        x: v.x,
        y: v.y,
        z: v.z,
    }
}

/// A `Failed` outcome carrying one diagnostic with a stable code.
fn failed(code: &str, message: String) -> TessellationOutcome {
    TessellationOutcome::Failed {
        diagnostics: vec![Diagnostic {
            object: None,
            code: code.to_string(),
            message,
        }],
    }
}

/// An `Unsupported` outcome carrying a stable code and the exchange kind.
fn unsupported(exchange: ExchangeKind, code: &str, detail: String) -> TessellationOutcome {
    TessellationOutcome::Unsupported {
        reason: UnsupportedReason {
            code: code.to_string(),
            exchange,
            detail,
        },
    }
}

/// The planner could not produce a plan for the body at all.
///
/// A caught panic (`TessError::Internal`) is a `Failed` with `kernel.failed` and a
/// **generic** message: a panic payload can quote kernel internals and never
/// reaches a diagnostic. Every other planner error means the mesher cannot
/// evaluate this body (malformed net, a kind it does not plan), which is a
/// capability refusal, not a kernel crash.
fn plan_failure(error: &solidcore_tess::TessError) -> TessellationOutcome {
    match error {
        solidcore_tess::TessError::Internal(_) => failed(
            crate::KERNEL_FAILED,
            "the mesher panicked during this call; nothing was returned".to_string(),
        ),
        solidcore_tess::TessError::Invalid(_)
        | solidcore_tess::TessError::Failed(_)
        | solidcore_tess::TessError::TooManyVertices { .. } => unsupported(
            ExchangeKind::Brep,
            codes::KERNEL_UNSUPPORTED_SURFACE,
            format!("solidcore-tess could not plan the body: {error}"),
        ),
    }
}

/// A fidelity shortfall of the produced mesh (not a missing entity).
fn tolerance_not_met(message: String) -> Diagnostic {
    Diagnostic {
        object: None,
        code: codes::KERNEL_TOLERANCE_NOT_MET.to_string(),
        message,
    }
}
