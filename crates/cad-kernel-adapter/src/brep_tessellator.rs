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
