//! The real, acadrust-free tessellator for a neutral B-rep payload, including
//! assembly, budget enforcement and open-edge detection.

use super::*;

/// The real, acadrust-free tessellator for a neutral [`BrepData`] payload.
///
/// It evaluates the documented subset (planar faces with holes, full spheres,
/// full tori and full cylindrical strips) and reports everything else as a
/// missing face. It deliberately does **not** parse SAT/SAB bytes: only the
/// importer may do that. A raw `Sat`/`Sab` payload therefore stays
/// `Unsupported(kernel.no_acis_kernel)` even here.
#[derive(Debug, Default, Clone, Copy)]
pub struct BrepTessellator;

impl BrepTessellator {
    fn failed(code: &str, message: String) -> TessellationOutcome {
        TessellationOutcome::Failed {
            diagnostics: vec![Diagnostic {
                code: code.to_string(),
                object: None,
                message,
            }],
        }
    }

    fn unsupported(exchange: ExchangeKind, code: &str, detail: String) -> TessellationOutcome {
        TessellationOutcome::Unsupported {
            reason: UnsupportedReason {
                code: code.to_string(),
                exchange,
                detail,
            },
        }
    }
}

/// Tessellate a raw exchange payload outside the database object lifecycle.
///
/// Import-time callers have no [`ObjectId`] or task stamp yet: the importer
/// lifts SAT/SAB into a neutral [`BrepData`] and needs the facet set before it
/// inserts the entity. This is the same evaluation path as
/// [`BrepTessellator::tessellate`] (tolerance/budget validation, empty-payload
/// refusal, budget enforcement), without inventing a handle or a stamp.
pub fn tessellate_exchange(
    exchange: &SolidExchange,
    tolerance: TessellationTolerance,
    budget: TessellationBudget,
) -> TessellationOutcome {
    tessellate_exchange_with(exchange, tolerance, budget, None, &|| false)
}

/// The neutral evaluation path of [`tessellate_exchange`], unchanged: what the
/// seam falls back to when it does not delegate.
pub(crate) fn tessellate_exchange_neutral(
    exchange: &SolidExchange,
    tolerance: TessellationTolerance,
    budget: TessellationBudget,
) -> TessellationOutcome {
    if let Err(error) = tolerance.validate().and_then(|_| budget.validate()) {
        return BrepTessellator::failed(error.code, error.message);
    }
    if exchange.is_empty() {
        return BrepTessellator::failed(
            codes::KERNEL_EMPTY_GEOMETRY,
            format!("{} payload carried no geometry", exchange.type_key()),
        );
    }
    match exchange {
        SolidExchange::Brep(brep) => tessellate_brep(tolerance, budget, brep),
        SolidExchange::Sat(_) | SolidExchange::Sab(_) => BrepTessellator::unsupported(
            exchange.kind(),
            codes::KERNEL_NO_ACIS_KERNEL,
            "raw SAT/SAB bytes are lifted by the importer, not by this tessellator".into(),
        ),
        SolidExchange::Unsupported { .. } => BrepTessellator::unsupported(
            ExchangeKind::Unsupported,
            codes::KERNEL_UNSUPPORTED_EXCHANGE,
            format!(
                "no decoder registered for exchange type '{}'",
                exchange.type_key()
            ),
        ),
    }
}

/// Tessellate an exchange payload through an **optional** kernel tessellator,
/// falling back to the neutral B-rep mesher.
///
/// The seam is only consulted for a payload the neutral mesher provably cannot
/// draw completely: a `SolidExchange::Brep` with **at least one**
/// [`BrepSurface::Nurbs`] face. Every other body keeps exactly the neutral path
/// (and its `Analytic` precision), so analytic-only display does not churn when a
/// carrier injects a mesher.
///
/// The request handed to the seam carries a deliberately synthetic identity —
/// [`GeometryHandle::Resolved`]`(ObjectId(0))` and
/// `TaskStamp::new(DocumentId(0), 0)`. This entry point is used outside the
/// database object lifecycle (import-time and command-time callers that have no
/// object id yet), and the stamp is only the seam's **result tag**: the caller of
/// this function discards it. Nothing may treat it as an object's identity.
///
/// When the seam returns usable facets (`Success` or `Partial`, non-empty) its
/// outcome is returned. When it cannot produce any (`Unsupported`, `Failed`,
/// empty, or an `Err`), the neutral path runs and **the seam's refusal stays
/// visible**: its diagnostics are prepended to the neutral outcome, or its detail
/// is folded into the neutral `Unsupported` reason. A neutral `Partial` stays
/// `Partial`.
pub fn tessellate_exchange_with(
    exchange: &SolidExchange,
    tolerance: TessellationTolerance,
    budget: TessellationBudget,
    tessellator: Option<&dyn SolidTessellator>,
    cancelled: &dyn Fn() -> bool,
) -> TessellationOutcome {
    let neutral = || tessellate_exchange_neutral(exchange, tolerance, budget);
    let Some(tessellator) = tessellator else {
        return neutral();
    };
    // Delegation rule (deliberate): only a neutral B-rep that carries a NURBS
    // face. A body of analytic faces is drawn by the neutral mesher, whose curved
    // arms are exact enough that a change of mesh would be churn, not an
    // improvement.
    let SolidExchange::Brep(brep) = exchange else {
        return neutral();
    };
    if !brep
        .shells
        .iter()
        .flat_map(|shell| &shell.faces)
        .any(|face| face_is_nurbs(&face.surface))
    {
        return neutral();
    }

    let request = TessellationRequest {
        geometry: GeometryHandle::Resolved(ObjectId(0)),
        exchange: exchange.clone(),
        tolerance,
        budget,
        stamp: TaskStamp::new(DocumentId(0), 0),
    };
    let refusal = match tessellator.tessellate(&request, cancelled) {
        Ok(result) => result.outcome,
        // The seam has no outcome to keep, so its error becomes one. A cancel is
        // its own code; the neutral path below still runs (it is not
        // cancellable), so the caller keeps a display and sees the refusal.
        Err(CadError::Cancelled) => TessellationOutcome::Failed {
            diagnostics: vec![Diagnostic {
                code: codes::KERNEL_CANCELLED.to_string(),
                object: None,
                message: "the solid tessellator reported the request as cancelled".into(),
            }],
        },
        Err(error) => TessellationOutcome::Failed {
            diagnostics: vec![Diagnostic {
                code: codes::SOLID_FAILED.to_string(),
                object: None,
                message: error.to_string(),
            }],
        },
    };
    if refusal.mesh().is_some_and(|mesh| !mesh.is_empty()) {
        return refusal;
    }
    with_refusal(neutral(), &refusal)
}

/// `BrepSurface::Nurbs` by name, with every other family listed explicitly: a new
/// variant has to be classified here rather than silently treated as analytic
/// (and therefore as "do not delegate").
fn face_is_nurbs(surface: &BrepSurface) -> bool {
    match surface {
        BrepSurface::Nurbs { .. } => true,
        BrepSurface::Plane { .. }
        | BrepSurface::Sphere { .. }
        | BrepSurface::Torus { .. }
        | BrepSurface::Cylinder { .. }
        | BrepSurface::Cone { .. }
        | BrepSurface::Unsupported { .. } => false,
    }
}

/// The neutral outcome, with the seam's refusal kept visible on it.
fn with_refusal(
    neutral: TessellationOutcome,
    refusal: &TessellationOutcome,
) -> TessellationOutcome {
    let mut diagnostics = refusal_diagnostics(refusal);
    match neutral {
        TessellationOutcome::Success {
            geometry,
            diagnostics: mut neutral_diagnostics,
        } => {
            diagnostics.append(&mut neutral_diagnostics);
            TessellationOutcome::Success {
                geometry,
                diagnostics,
            }
        }
        TessellationOutcome::Partial {
            geometry,
            degradation,
            diagnostics: mut neutral_diagnostics,
        } => {
            diagnostics.append(&mut neutral_diagnostics);
            TessellationOutcome::Partial {
                geometry,
                degradation,
                diagnostics,
            }
        }
        TessellationOutcome::Failed {
            diagnostics: mut neutral_diagnostics,
        } => {
            diagnostics.append(&mut neutral_diagnostics);
            TessellationOutcome::Failed { diagnostics }
        }
        // `Unsupported` carries no diagnostics list, so the seam's detail is
        // folded into its reason instead of dropped.
        TessellationOutcome::Unsupported { reason } => TessellationOutcome::Unsupported {
            reason: UnsupportedReason {
                code: reason.code,
                exchange: reason.exchange,
                detail: format!("{}; {}", refusal_detail(refusal), reason.detail),
            },
        },
    }
}

/// The diagnostics a refused seam outcome stands for.
fn refusal_diagnostics(refusal: &TessellationOutcome) -> Vec<Diagnostic> {
    match refusal {
        TessellationOutcome::Failed { diagnostics } => diagnostics.clone(),
        TessellationOutcome::Unsupported { reason } => vec![Diagnostic {
            code: reason.code.clone(),
            object: None,
            message: format!(
                "the solid tessellator cannot evaluate this body: {}",
                reason.detail
            ),
        }],
        // A non-empty success/partial is returned as it is, so these cannot reach
        // here; an empty one has nothing to say.
        TessellationOutcome::Success { .. } | TessellationOutcome::Partial { .. } => Vec::new(),
    }
}

/// One line naming what the seam refused, for an `Unsupported` reason's detail.
fn refusal_detail(refusal: &TessellationOutcome) -> String {
    match refusal {
        TessellationOutcome::Unsupported { reason } => {
            format!("{}: {}", reason.code, reason.detail)
        }
        TessellationOutcome::Failed { diagnostics } => {
            match (diagnostics.first(), diagnostics.len()) {
                (Some(first), count) => {
                    format!("{}: {} (+{} more)", first.code, first.message, count - 1)
                }
                (None, _) => "the solid tessellator produced no usable facet set".into(),
            }
        }
        TessellationOutcome::Success { .. } | TessellationOutcome::Partial { .. } => {
            "the solid tessellator produced no usable facet set".into()
        }
    }
}

impl SolidTessellator for BrepTessellator {
    fn registration(&self) -> Registration {
        Registration {
            type_key: "yacr.kernel.brep".into(),
            version: 1,
            priority: 1,
            entity_types: vec![
                "3DSOLID".into(),
                "BODY".into(),
                "REGION".into(),
                "SURFACE".into(),
            ],
            // Deliberately no `nurbs-*` entry: a NURBS face is carried by the
            // neutral model (P2.1b part 1) but this tessellator has no mesh for
            // one (`solidcore-tess`, phase P2.1c), so nothing here may claim it.
            // This list is what is supported, not what may arrive.
            capabilities: vec![
                "planar-faces".into(),
                "planar-faces-with-holes".into(),
                "spherical-faces".into(),
                "toroidal-faces".into(),
                "cylindrical-faces".into(),
                "conical-faces".into(),
            ],
        }
    }

    fn tessellate(
        &self,
        request: &TessellationRequest,
        cancelled: &dyn Fn() -> bool,
    ) -> CadResult<TessellationResult> {
        if cancelled() {
            return Err(CadError::Cancelled);
        }
        request
            .validate()
            .map_err(|e| CadError::InvalidInput(e.to_string()))?;

        if let GeometryHandle::Missing { key } = &request.geometry {
            return Ok(TessellationResult {
                stamp: request.stamp.clone(),
                outcome: Self::failed(
                    codes::KERNEL_MISSING_HANDLE,
                    format!("geometry handle '{key}' could not be resolved"),
                ),
            });
        }

        Ok(TessellationResult {
            stamp: request.stamp.clone(),
            outcome: tessellate_exchange(&request.exchange, request.tolerance, request.budget),
        })
    }
}

/// Tessellate a neutral B-rep, enforcing the request budget and reporting
/// degradation exactly (missing faces, open edges, dropped shells).
///
/// Every produced triangle is stamped with a stable
/// [`SubElementId`] of the form `brep:face:<shell id>/<face id>`, one entry per
/// triangle in [`Mesh::face_sources`], so a pick on a display triangle can be
/// mapped back to the B-rep face that produced it. The `topology_revision` is
/// `Revision(0)`: this tessellator has no entity revision at facet time (the
/// same documented convention `cad-measure` uses when it synthesises a
/// sub-element from a pick key — see its `snap/resolve.rs`). It is therefore a
/// placeholder, not a claim that the face set is at database revision 0. **No
/// invalidation is implemented today**: these ids do not change when a body's
/// exact topology changes, and nothing re-stamps `face_sources` on a revision
/// bump or drops a stale one. A face-level **edit** (or any consumer that binds
/// persistent data to a face id) must therefore not ship until the producer
/// stamps the real revision and invalidates the source list when it changes; a
/// display-only pick can use the placeholder as it does now.
fn tessellate_brep(
    tolerance: TessellationTolerance,
    budget: TessellationBudget,
    brep: &BrepData,
) -> TessellationOutcome {
    let tol = tolerance;
    let placement = brep.placement.unwrap_or_else(BrepPlacement::identity);

    let mut vertices: Vec<Point3> = Vec::new();
    let mut normals: Vec<Point3> = Vec::new();
    let mut triangles: Vec<[u32; 3]> = Vec::new();
    let mut face_sources: Vec<Option<SubElementId>> = Vec::new();
    let mut edges: Vec<Vec<Point3>> = Vec::new();
    let mut missing_faces: Vec<FaceRef> = Vec::new();
    let mut dropped_shells: Vec<ShellRef> = Vec::new();
    let mut all_unsupported = true;
    let mut all_planar = true;
    let mut error_bound = 0.0f64;

    for shell in &brep.shells {
        let mut shell_triangles = 0usize;
        for face in &shell.faces {
            match brep::tessellate_face(face, tol.linear_deflection, tol.angular_deflection) {
                Ok(fm) => {
                    if fm.curved {
                        all_planar = false;
                        error_bound = error_bound.max(fm.error_bound.unwrap_or(0.0));
                    }
                    let base = vertices.len() as u32;
                    let produced = fm.triangles.len();
                    // One stable source id per produced triangle. `Revision(0)`
                    // is the documented placeholder for a producer with no
                    // entity revision (see the function doc).
                    let source = SubElementId {
                        source_key: format!("brep:face:{}/{}", shell.id, face.id),
                        topology_revision: Revision(0),
                    };
                    for t in &fm.triangles {
                        triangles.push([base + t[0], base + t[1], base + t[2]]);
                        face_sources.push(Some(source.clone()));
                    }
                    vertices.extend(fm.vertices.iter().map(|p| placement.apply(*p)));
                    normals.extend(fm.normals.iter().map(|n| placement.apply_direction(*n)));
                    for edge in fm.edges {
                        edges.push(edge.iter().map(|p| placement.apply(*p)).collect());
                    }
                    shell_triangles += produced;
                }
                Err(failure) => {
                    if !failure.is_unsupported() {
                        all_unsupported = false;
                    }
                    missing_faces.push(FaceRef {
                        id: face.id,
                        reason: failure.reason(),
                    });
                }
            }
        }
        if shell_triangles == 0 {
            dropped_shells.push(ShellRef {
                id: shell.id,
                reason: "no face in the shell could be tessellated".into(),
            });
        }
    }

    // No usable facet set is never a success.
    if triangles.is_empty() {
        if missing_faces.is_empty() {
            return BrepTessellator::failed(
                codes::KERNEL_EMPTY_GEOMETRY,
                "neutral B-rep carried no tessellatable face".into(),
            );
        }
        if all_unsupported {
            let detail = missing_faces
                .iter()
                .map(|f| f.reason.as_str())
                .collect::<Vec<_>>()
                .join("; ");
            return BrepTessellator::unsupported(
                ExchangeKind::Brep,
                codes::KERNEL_UNSUPPORTED_SURFACE,
                detail,
            );
        }
        let diagnostics = TessellationDegradation {
            missing_faces,
            ..TessellationDegradation::default()
        }
        .diagnostics();
        return TessellationOutcome::Failed { diagnostics };
    }

    let open_edges = detect_open_edges(&vertices, &triangles);
    let precision = if all_planar {
        Precision::Analytic
    } else {
        Precision::Approximate {
            error_bound: Some(error_bound),
        }
    };
    let mesh = TessellationMesh {
        mesh: Mesh {
            vertices,
            triangles,
            normals,
            face_sources,
            colors: Vec::new(),
            uvs: Vec::new(),
            texture: None,
        },
        edges,
        precision,
    };

    if let Err(diagnostic) = budget.check(&mesh) {
        return TessellationOutcome::Failed {
            diagnostics: vec![diagnostic],
        };
    }

    let degradation = TessellationDegradation {
        missing_faces,
        open_edges,
        dropped_shells,
    };
    if degradation.is_empty() {
        TessellationOutcome::Success {
            geometry: mesh,
            diagnostics: Vec::new(),
        }
    } else {
        let diagnostics = degradation.diagnostics();
        TessellationOutcome::Partial {
            geometry: mesh,
            degradation,
            diagnostics,
        }
    }
}

/// Detect positional edges not shared by exactly two facets (open or
/// non-manifold) after welding coincident vertices within a relative epsilon.
pub(crate) fn detect_open_edges(vertices: &[Point3], triangles: &[[u32; 3]]) -> Vec<EdgeRef> {
    use std::collections::HashMap;
    if vertices.is_empty() {
        return Vec::new();
    }
    let (mut min, mut max) = (vertices[0], vertices[0]);
    for p in vertices {
        min = Point3 {
            x: min.x.min(p.x),
            y: min.y.min(p.y),
            z: min.z.min(p.z),
        };
        max = Point3 {
            x: max.x.max(p.x),
            y: max.y.max(p.y),
            z: max.z.max(p.z),
        };
    }
    let diag = {
        let d = brep::sub(max, min);
        brep::length(d)
    };
    let eps = (1e-9 * diag.max(1.0)).max(1e-12);
    let key = |p: Point3| -> (i64, i64, i64) {
        (
            (p.x / eps).round() as i64,
            (p.y / eps).round() as i64,
            (p.z / eps).round() as i64,
        )
    };
    let mut weld: HashMap<(i64, i64, i64), u32> = HashMap::new();
    let mut welded = Vec::with_capacity(vertices.len());
    for p in vertices {
        let k = key(*p);
        let id = *weld.entry(k).or_insert_with(|| welded.len() as u32);
        welded.push(id);
    }
    let mut counts: HashMap<(u32, u32), u32> = HashMap::new();
    for t in triangles {
        let w = [
            welded[t[0] as usize],
            welded[t[1] as usize],
            welded[t[2] as usize],
        ];
        for e in [(w[0], w[1]), (w[1], w[2]), (w[2], w[0])] {
            let k = if e.0 <= e.1 { (e.0, e.1) } else { (e.1, e.0) };
            *counts.entry(k).or_insert(0) += 1;
        }
    }
    let mut open: Vec<EdgeRef> = counts
        .into_iter()
        .filter(|(_, c)| *c != 2)
        .map(|(_, c)| EdgeRef {
            id: 0,
            reason: if c == 1 {
                "boundary edge used by a single facet".to_string()
            } else {
                format!("non-manifold edge shared by {c} facets")
            },
        })
        .collect();
    open.sort_by(|a, b| a.reason.cmp(&b.reason));
    for (i, e) in open.iter_mut().enumerate() {
        e.id = i as u32;
    }
    open
}
