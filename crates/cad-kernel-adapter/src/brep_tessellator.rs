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
    fn failed(stamp: &TaskStamp, code: &str, message: String) -> TessellationResult {
        TessellationResult {
            stamp: stamp.clone(),
            outcome: TessellationOutcome::Failed {
                diagnostics: vec![Diagnostic {
                    code: code.to_string(),
                    object: None,
                    message,
                }],
            },
        }
    }

    fn unsupported(
        stamp: &TaskStamp,
        exchange: ExchangeKind,
        code: &str,
        detail: String,
    ) -> TessellationResult {
        TessellationResult {
            stamp: stamp.clone(),
            outcome: TessellationOutcome::Unsupported {
                reason: UnsupportedReason {
                    code: code.to_string(),
                    exchange,
                    detail,
                },
            },
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
            capabilities: vec![
                "planar-faces".into(),
                "planar-faces-with-holes".into(),
                "spherical-faces".into(),
                "toroidal-faces".into(),
                "cylindrical-faces".into(),
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
            return Ok(Self::failed(
                &request.stamp,
                codes::KERNEL_MISSING_HANDLE,
                format!("geometry handle '{key}' could not be resolved"),
            ));
        }
        if request.exchange.is_empty() {
            return Ok(Self::failed(
                &request.stamp,
                codes::KERNEL_EMPTY_GEOMETRY,
                format!(
                    "{} payload carried no geometry",
                    request.exchange.type_key()
                ),
            ));
        }

        match &request.exchange {
            SolidExchange::Brep(brep) => Ok(tessellate_brep(request, brep)),
            SolidExchange::Sat(_) | SolidExchange::Sab(_) => Ok(Self::unsupported(
                &request.stamp,
                request.exchange.kind(),
                codes::KERNEL_NO_ACIS_KERNEL,
                "raw SAT/SAB bytes are lifted by the importer, not by this tessellator".into(),
            )),
            SolidExchange::Unsupported { .. } => Ok(Self::unsupported(
                &request.stamp,
                ExchangeKind::Unsupported,
                codes::KERNEL_UNSUPPORTED_EXCHANGE,
                format!(
                    "no decoder registered for exchange type '{}'",
                    request.exchange.type_key()
                ),
            )),
        }
    }
}

/// Tessellate a neutral B-rep, enforcing the request budget and reporting
/// degradation exactly (missing faces, open edges, dropped shells).
fn tessellate_brep(request: &TessellationRequest, brep: &BrepData) -> TessellationResult {
    let tol = request.tolerance;
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
                    for t in &fm.triangles {
                        triangles.push([base + t[0], base + t[1], base + t[2]]);
                        face_sources.push(None);
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
                &request.stamp,
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
                &request.stamp,
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
        return TessellationResult {
            stamp: request.stamp.clone(),
            outcome: TessellationOutcome::Failed { diagnostics },
        };
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
        },
        edges,
        precision,
    };

    if let Err(diagnostic) = request.budget.check(&mesh) {
        return TessellationResult {
            stamp: request.stamp.clone(),
            outcome: TessellationOutcome::Failed {
                diagnostics: vec![diagnostic],
            },
        };
    }

    let degradation = TessellationDegradation {
        missing_faces,
        open_edges,
        dropped_shells,
    };
    if degradation.is_empty() {
        TessellationResult {
            stamp: request.stamp.clone(),
            outcome: TessellationOutcome::Success {
                geometry: mesh,
                diagnostics: Vec::new(),
            },
        }
    } else {
        let diagnostics = degradation.diagnostics();
        TessellationResult {
            stamp: request.stamp.clone(),
            outcome: TessellationOutcome::Partial {
                geometry: mesh,
                degradation,
                diagnostics,
            },
        }
    }
}

/// Detect positional edges not shared by exactly two facets (open or
/// non-manifold) after welding coincident vertices within a relative epsilon.
fn detect_open_edges(vertices: &[Point3], triangles: &[[u32; 3]]) -> Vec<EdgeRef> {
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
